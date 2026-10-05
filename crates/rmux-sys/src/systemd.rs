// Ported from tmux compat/systemd.c, server.c, client.c @ 8f25579c
/*
 * Copyright (c) 2022 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
//! Linux socket activation without a libsystemd dependency.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixListener;

const LISTEN_FDS_START: i32 = 3;
static ACTIVATION_CLAIMED: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

fn decimal(value: &[u8]) -> Option<u32> {
    let value = value.strip_prefix(b"+").unwrap_or(value);
    if value.is_empty() {
        return None;
    }
    value.iter().try_fold(0u32, |n, &digit| {
        if !digit.is_ascii_digit() {
            return None;
        }
        n.checked_mul(10)?.checked_add(u32::from(digit - b'0'))
    })
}

fn activation_count(pid: Option<&[u8]>, fds: Option<&[u8]>, current_pid: u32) -> io::Result<i32> {
    let Some(pid) = pid else { return Ok(0) };
    let pid = decimal(pid)
        .filter(|&pid| pid > 0)
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    if pid != current_pid {
        return Ok(0);
    }
    let Some(fds) = fds else { return Ok(0) };
    let fds = decimal(fds).ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    if fds > (i32::MAX - LISTEN_FDS_START) as u32 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    Ok(fds as i32)
}

// sd_listen_fds(0): preserve the environment and mark every inherited fd CLOEXEC.
fn listen_fds() -> io::Result<i32> {
    let pid = std::env::var_os("LISTEN_PID");
    let fds = std::env::var_os("LISTEN_FDS");
    let count = activation_count(
        pid.as_deref().map(OsStrExt::as_bytes),
        fds.as_deref().map(OsStrExt::as_bytes),
        std::process::id(),
    )?;
    for fd in LISTEN_FDS_START..LISTEN_FDS_START + count {
        // SAFETY: fcntl only queries or changes descriptor flags, retaining no pointers.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd was just validated; this does not change its ownership.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(count)
}

/// Harvest the process-start activation descriptors before any other descriptor opens.
/// Uses the canonical inherited-fd boundary; subsequent calls never claim descriptors.
/// Invalid activation metadata falls back to normal startup as sd_listen_fds(0) does.
pub fn take_activation_listener() -> io::Result<Option<OwnedFd>> {
    let mut claimed = ACTIVATION_CLAIMED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *claimed {
        return Ok(None);
    }
    *claimed = true;
    let count = listen_fds().unwrap_or(0);
    for name in [c"LISTEN_PID", c"LISTEN_FDS", c"LISTEN_FDNAMES"] {
        // SAFETY: called once at process startup before any threads or environment readers.
        unsafe { libc::unsetenv(name.as_ptr()) };
    }
    let mut listener = None;
    for fd in LISTEN_FDS_START..LISTEN_FDS_START + count {
        let owned = crate::client::take_inherited_fd(fd);
        if count == 1 {
            listener = Some(owned);
        }
    }
    if count > 1 {
        return Err(io::Error::from_raw_os_error(libc::E2BIG));
    }
    Ok(listener)
}

fn socket_option(fd: i32, option: i32) -> io::Result<i32> {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of_val(&value) as libc::socklen_t;
    // SAFETY: value and len are correctly sized writable storage for getsockopt.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&raw mut value).cast(),
            &mut len,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

fn is_socket_unix(fd: i32) -> io::Result<bool> {
    // SAFETY: stat is plain C storage and fstat fills it without retaining a pointer.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: stat has the correct size and alignment for fstat.
    if unsafe { libc::fstat(fd, &mut stat) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if stat.st_mode & libc::S_IFMT != libc::S_IFSOCK
        || socket_option(fd, libc::SO_TYPE)? != libc::SOCK_STREAM
        || socket_option(fd, libc::SO_ACCEPTCONN)? != 1
    {
        return Ok(false);
    }
    // SAFETY: sockaddr_un is plain C storage and zero initialization is valid.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&address) as libc::socklen_t;
    // SAFETY: address has len bytes of writable storage; getsockname retains no pointer.
    if unsafe { libc::getsockname(fd, (&raw mut address).cast(), &mut len) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(address.sun_family == libc::AF_UNIX as _)
}

fn listener_path(fd: i32) -> io::Result<Vec<u8>> {
    // The pin tests !sd_is_socket_unix: negative errors fall through to getsockname.
    if matches!(is_socket_unix(fd), Ok(false)) {
        return Err(io::Error::from_raw_os_error(libc::EPFNOSUPPORT));
    }
    // SAFETY: sockaddr_un is plain C storage and zero initialization is valid.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&address) as libc::socklen_t;
    // SAFETY: address has len bytes of writable storage; getsockname retains no pointer.
    if unsafe { libc::getsockname(fd, (&raw mut address).cast(), &mut len) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // xstrdup(sa.sun_path) deliberately exposes an empty path for abstract sockets.
    Ok(address
        .sun_path
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect())
}

/// Validate an owned activation listener and return its actual socket pathname.
/// The caller formats errors as `systemd socket error (<strerror>)`.
pub fn create_listener(fd: OwnedFd) -> io::Result<(UnixListener, Vec<u8>)> {
    let path = listener_path(fd.as_raw_fd())?;
    let listener = UnixListener::from(fd);
    listener.set_nonblocking(true)?;
    Ok((listener, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::{UnixDatagram, UnixStream};

    #[test]
    fn activation_child() {
        let Ok(case) = std::env::var("RMUX_ACTIVATION_TEST") else {
            return;
        };
        let activation = take_activation_listener();
        assert!(take_activation_listener().unwrap().is_none());
        for name in ["LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"] {
            assert!(std::env::var_os(name).is_none());
        }
        if case == "many" {
            assert_eq!(activation.unwrap_err().raw_os_error(), Some(libc::E2BIG));
            for fd in [3, 4] {
                // SAFETY: fcntl checks that rejecting multiple descriptors closed each owner.
                assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
            }
        } else {
            let (listener, path) = create_listener(activation.unwrap().unwrap()).unwrap();
            assert_eq!(
                path,
                std::env::var_os("RMUX_ACTIVATION_PATH").unwrap().as_bytes()
            );
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            assert_eq!(listener.as_raw_fd(), LISTEN_FDS_START);
            // SAFETY: fcntl reads flags on the live inherited descriptor.
            assert_ne!(
                unsafe { libc::fcntl(LISTEN_FDS_START, libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
            drop(listener);
            // SAFETY: fcntl checks that dropping the owner closed the inherited descriptor.
            assert_eq!(unsafe { libc::fcntl(LISTEN_FDS_START, libc::F_GETFD) }, -1);
        }
    }

    #[test]
    fn activation_process_clears_environment_and_closes_rejected_fds() {
        use std::os::unix::process::CommandExt;
        let path =
            std::env::temp_dir().join(format!("rmux-systemd-process-{}", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        for case in ["one", "many"] {
            let fd = listener.as_raw_fd();
            let mut command = std::process::Command::new("/bin/sh");
            command
                .arg("-c")
                .arg("LISTEN_PID=$$; export LISTEN_PID; exec \"$@\"")
                .arg("sh")
                .arg(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("systemd::tests::activation_child")
                .arg("--nocapture")
                .env("LISTEN_FDS", if case == "many" { "2" } else { "1" })
                .env("RMUX_ACTIVATION_TEST", case)
                .env("RMUX_ACTIVATION_PATH", &path);
            // SAFETY: the child callback uses only async-signal-safe descriptor operations.
            unsafe {
                command.pre_exec(move || {
                    if libc::dup2(fd, 3) == -1 || libc::fcntl(3, libc::F_SETFD, 0) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    if case == "many" && libc::dup2(3, 4) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            assert!(
                command.status().unwrap().success(),
                "activation case {case}"
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn activation_environment_matches_pid_and_limits() {
        assert_eq!(activation_count(None, Some(b"1"), 42).unwrap(), 0);
        assert_eq!(activation_count(Some(b"41"), Some(b"bad"), 42).unwrap(), 0);
        assert_eq!(activation_count(Some(b"42"), None, 42).unwrap(), 0);
        assert_eq!(activation_count(Some(b"+42"), Some(b"2"), 42).unwrap(), 2);
        assert_eq!(activation_count(Some(b"42"), Some(b"0"), 42).unwrap(), 0);
        for value in [
            b"".as_slice(),
            b"-1",
            b"1x",
            b" 1",
            b"2147483645",
            b"4294967296",
        ] {
            assert!(activation_count(Some(b"42"), Some(value), 42).is_err());
        }
        assert!(activation_count(Some(b"0"), Some(b"1"), 42).is_err());
    }

    #[test]
    fn activated_listener_path_preserves_original_owner() {
        let path = std::env::temp_dir().join(format!("rmux-systemd-{}", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let actual = listener_path(listener.as_raw_fd()).unwrap();
        assert_eq!(actual, path.as_os_str().as_bytes());
        let stream = UnixStream::connect(&path).unwrap();
        let accepted = listener.accept().unwrap();
        drop((stream, accepted));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_non_listener_and_wrong_socket_type() {
        let (stream, _) = UnixStream::pair().unwrap();
        let datagram = UnixDatagram::unbound().unwrap();
        for fd in [stream.as_raw_fd(), datagram.as_raw_fd()] {
            assert_eq!(
                listener_path(fd).unwrap_err().raw_os_error(),
                Some(libc::EPFNOSUPPORT)
            );
        }
        assert_eq!(
            listener_path(-1).unwrap_err().raw_os_error(),
            Some(libc::EBADF)
        );
    }
}
