// Ported from tmux client.c, tmux.c, control.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
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
//! System calls used only by the `rmux` client binary.

use crate::ProcessId;
use crate::cstring::copy_cstr;
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStringExt;

fn cstring(bytes: &[u8]) -> io::Result<CString> {
    CString::new(crate::cstring::cstr(bytes).to_vec())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "string contains NUL"))
}

/// `client_connect` socket setup (`client.c:112-126`): a path at or over
/// `sizeof sun_path` gives `ENAMETOOLONG`.
pub fn connect_unix(path: &[u8]) -> io::Result<OwnedFd> {
    let path = crate::cstring::cstr(path);
    // SAFETY: sockaddr_un is plain data; zeroed is a valid value.
    let mut sa: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    sa.sun_family = libc::AF_UNIX as libc::sa_family_t;
    if path.len() >= sa.sun_path.len() {
        return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
    }
    for (dst, src) in sa.sun_path.iter_mut().zip(path) {
        *dst = *src as libc::c_char;
    }
    // SAFETY: socket takes no pointers.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw was returned by socket and is owned only here.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: sa is a fully initialized sockaddr_un and the length matches it.
    let rc = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&sa as *const libc::sockaddr_un).cast(),
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// `open(lockfile, O_WRONLY|O_CREAT, 0600)` (`client.c:84`).
pub fn open_lock_file(path: &[u8]) -> io::Result<OwnedFd> {
    let path = cstring(path)?;
    // SAFETY: path is a valid NUL-terminated string; mode is passed as the vararg.
    let raw = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_CLOEXEC,
            0o600 as libc::c_uint,
        )
    };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw was returned by open and is owned only here.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// `unlink(path)` (`client.c:159`).
pub fn unlink(path: &[u8]) -> io::Result<()> {
    let path = cstring(path)?;
    // SAFETY: path is a valid NUL-terminated string.
    if unsafe { libc::unlink(path.as_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `ttyname(fd)` (`client.c:305`).
pub fn ttyname(fd: BorrowedFd<'_>) -> Option<Vec<u8>> {
    let mut buf = [0 as libc::c_char; 256];
    // SAFETY: buf is writable storage of the given length; fd is open for the borrow.
    let rc = unsafe { libc::ttyname_r(fd.as_raw_fd(), buf.as_mut_ptr(), buf.len()) };
    if rc != 0 {
        return None;
    }
    // SAFETY: ttyname_r wrote a NUL-terminated string into buf.
    Some(unsafe { copy_cstr(buf.as_ptr()) })
}

/// `getppid()` (`client.c:414`).
#[must_use]
pub fn getppid() -> ProcessId {
    // SAFETY: getppid takes no arguments and cannot fail.
    ProcessId(unsafe { libc::getppid() })
}

/// `kill(pid, signal)` (`client.c:416,795`).
pub fn kill(pid: ProcessId, signal: i32) -> io::Result<()> {
    // SAFETY: kill takes no pointers.
    if unsafe { libc::kill(pid.0, signal) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `kill(getpid(), signal)` (`client.c:795`).
pub fn raise(signal: i32) -> io::Result<()> {
    // SAFETY: raise takes no pointers.
    if unsafe { libc::raise(signal) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Signal disposition for [`set_signal_disposition`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    Default,
    Ignore,
}

/// `sigaction` with an empty mask and `SA_RESTART` (`client.c:552-557,788-793`).
pub fn set_signal_disposition(signal: i32, disposition: Disposition) -> io::Result<()> {
    // SAFETY: sigaction is plain C storage initialized before use.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = match disposition {
        Disposition::Default => libc::SIG_DFL,
        Disposition::Ignore => libc::SIG_IGN,
    };
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: the mask is live storage; the action stays alive for the call.
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(signal, &action, std::ptr::null_mut()) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// `system(3)` (`client.c:801`); returns the raw wait status or -1.
pub fn system(command: &[u8]) -> i32 {
    let Ok(command) = cstring(command) else {
        return -1;
    };
    // SAFETY: command is a valid NUL-terminated string.
    unsafe { libc::system(command.as_ptr()) }
}

/// `client_exec` tail (`client.c:498-508`): set `SHELL`, make the standard
/// streams blocking, close everything above stderr, then
/// `execl(shell, argv0, "-c", cmd)`. Returns only on failure.
pub fn exec_shell(shell: &[u8], argv0: &[u8], cmd: &[u8]) -> io::Error {
    let (Ok(shell_c), Ok(argv0_c), Ok(cmd_c)) = (cstring(shell), cstring(argv0), cstring(cmd))
    else {
        return io::Error::new(io::ErrorKind::InvalidInput, "string contains NUL");
    };
    let key = c"SHELL";
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe { libc::setenv(key.as_ptr(), shell_c.as_ptr(), 1) };
    for fd in 0..=2 {
        // SAFETY: standard descriptors are valid for the whole process.
        crate::fd::set_blocking(unsafe { BorrowedFd::borrow_raw(fd) }, true);
    }
    // SAFETY: nothing above stderr is used after this point; exec follows.
    unsafe { crate::proc::closefrom(3) };
    let dash_c = c"-c";
    // SAFETY: every argument is a live NUL-terminated string and the list ends with NULL.
    unsafe {
        libc::execl(
            shell_c.as_ptr(),
            argv0_c.as_ptr(),
            dash_c.as_ptr(),
            cmd_c.as_ptr(),
            std::ptr::null::<libc::c_char>(),
        )
    };
    io::Error::last_os_error()
}

/// Take ownership of standard descriptor `n` (0, 1 or 2) for the CLI.
///
/// Must be called at most once per descriptor in the process: the returned
/// `OwnedFd` closes it on drop and nothing else may hold ownership.
#[must_use]
pub fn take_standard_fd(n: u8) -> OwnedFd {
    assert!(n <= 2, "standard descriptor out of range");
    // SAFETY: the caller promises exclusive one-time ownership of this descriptor.
    unsafe { OwnedFd::from_raw_fd(RawFd::from(n)) }
}

/// Take ownership of a descriptor inherited across exec (`--rmux-internal-server`).
///
/// Must be called at most once per descriptor number in the process.
#[must_use]
pub fn take_inherited_fd(fd: i32) -> OwnedFd {
    assert!(fd >= 0, "inherited descriptor out of range");
    // SAFETY: the caller promises the number names an open inherited descriptor owned once.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

/// `dup(fd)` (`client.c:470,473`).
pub fn dup_fd(fd: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    // SAFETY: fd is open for the borrow; dup takes no pointers.
    let raw = unsafe { libc::dup(fd.as_raw_fd()) };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: dup returned a new descriptor owned only here.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Clear `FD_CLOEXEC` so a child exec inherits `fd`.
pub fn set_inheritable(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fd is open for the borrow; F_GETFD/F_SETFD take no pointers.
    unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFD);
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// `socketpair(AF_UNIX, SOCK_STREAM)` (`proc.c:379`). Both ends are
/// close-on-exec: the reexecuted server must not inherit the client end
/// (`proc.c:385` closes it in the child), so `set_inheritable` picks the
/// end that crosses.
pub fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut pair = [0 as libc::c_int; 2];
    // SAFETY: pair is writable storage for two descriptors.
    if unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_STREAM,
            libc::PF_UNSPEC,
            pair.as_mut_ptr(),
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors were returned by socketpair and are owned only here.
    let pair = unsafe { (OwnedFd::from_raw_fd(pair[0]), OwnedFd::from_raw_fd(pair[1])) };
    for fd in [&pair.0, &pair.1] {
        // SAFETY: fd is open and owned by pair; F_SETFD takes no pointers.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(pair)
}

/// `getcwd()` (`tmux.c:389`).
pub fn getcwd() -> Option<Vec<u8>> {
    std::env::current_dir()
        .ok()
        .map(|p| p.into_os_string().into_vec())
}

/// `realpath(path)` (`tmux.c:180,251,398`).
pub fn realpath(path: &[u8]) -> io::Result<Vec<u8>> {
    let path = cstring(path)?;
    // SAFETY: path is NUL-terminated; a null buffer asks realpath to allocate.
    let resolved = unsafe { libc::realpath(path.as_ptr(), std::ptr::null_mut()) };
    if resolved.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: realpath returned a malloc'd NUL-terminated string that we free once.
    let out = unsafe { copy_cstr(resolved) };
    // SAFETY: resolved was allocated by realpath and is not used afterwards.
    unsafe { libc::free(resolved.cast()) };
    Ok(out)
}

/// `mkdir(path, mode)` (`tmux.c:271`).
pub fn mkdir(path: &[u8], mode: u32) -> io::Result<()> {
    let path = cstring(path)?;
    // SAFETY: path is a valid NUL-terminated string.
    if unsafe { libc::mkdir(path.as_ptr(), mode as libc::mode_t) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The `lstat` fields `make_label` checks (`tmux.c:276-288`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LstatInfo {
    pub uid: u32,
    pub mode: u32,
    pub is_dir: bool,
}

/// `lstat(path)` (`tmux.c:276`).
pub fn lstat_info(path: &[u8]) -> io::Result<LstatInfo> {
    let path = cstring(path)?;
    // SAFETY: stat is plain data; zeroed is a valid value.
    let mut sb: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: path is NUL-terminated and sb is a valid out pointer.
    if unsafe { libc::lstat(path.as_ptr(), &mut sb) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(LstatInfo {
        uid: sb.st_uid,
        mode: sb.st_mode as u32,
        is_dir: (sb.st_mode & libc::S_IFMT) == libc::S_IFDIR,
    })
}

/// `sig2name` (`tmux.c:369-380`): `sys_signame` on macOS for `0 < signo < NSIG`,
/// else the decimal number.
#[must_use]
pub fn signal_name(signo: i32) -> Vec<u8> {
    #[cfg(target_os = "macos")]
    {
        // Darwin <signal.h> NSIG, absent from the libc crate.
        const NSIG: i32 = 32;
        if signo > 0 && signo < NSIG {
            unsafe extern "C" {
                static sys_signame: [*const libc::c_char; NSIG as usize];
            }
            // SAFETY: sys_signame has NSIG entries and the index was bounds-checked.
            let name = unsafe { sys_signame[signo as usize] };
            if !name.is_null() {
                // SAFETY: libc's table holds NUL-terminated static strings.
                return unsafe { copy_cstr(name) };
            }
        }
    }
    signo.to_string().into_bytes()
}

/// `SIGCHLD` reap loop (`client.c:520-531`): `waitpid(WAIT_ANY, WNOHANG)` until
/// no child is ready or `ECHILD`.
pub fn wait_any_children() {
    loop {
        let mut status = 0;
        // SAFETY: status is a valid out pointer; -1 selects any child.
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid == 0 {
            break;
        }
        if pid == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
            break;
        }
    }
}

/// `environ` as `NAME=value` entries (`client.c:480`, `tmux.c:461`).
#[must_use]
pub fn environ() -> Vec<Vec<u8>> {
    std::env::vars_os()
        .map(|(key, value)| {
            let mut entry = key.into_vec();
            entry.push(b'=');
            entry.extend_from_slice(&value.into_vec());
            entry
        })
        .collect()
}

/// `getenv(name)` as bytes; `None` when unset.
#[must_use]
pub fn getenv(name: &str) -> Option<Vec<u8>> {
    std::env::var_os(name).map(OsStringExt::into_vec)
}

/// The passwd shell of the current user (`tmux.c:90-92`).
#[must_use]
pub fn passwd_shell() -> Option<Vec<u8>> {
    // SAFETY: libc owns the returned record; its shell is copied before any
    // other passwd lookup can invalidate it.
    unsafe {
        let record = libc::getpwuid(libc::getuid());
        if record.is_null() || (*record).pw_shell.is_null() {
            return None;
        }
        Some(copy_cstr((*record).pw_shell))
    }
}

/// Signal numbers the client handles (`client.c:512-563,784-796`).
pub const SIGCHLD: i32 = libc::SIGCHLD;
pub const SIGHUP: i32 = libc::SIGHUP;
pub const SIGTERM: i32 = libc::SIGTERM;
pub const SIGWINCH: i32 = libc::SIGWINCH;
pub const SIGCONT: i32 = libc::SIGCONT;
pub const SIGTSTP: i32 = libc::SIGTSTP;

/// `poll(fd, POLLIN, INFTIM)` with `EINTR` retried (`control.c:767-774`).
pub fn poll_readable(fd: BorrowedFd<'_>) -> io::Result<()> {
    let mut pfd = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: pfd is a valid pollfd array of length one.
        if unsafe { libc::poll(&mut pfd, 1, -1) } == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn long_socket_path_is_enametoolong() {
        let path = vec![b'a'; 200];
        let error = connect_unix(&path).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENAMETOOLONG));
    }

    #[test]
    fn missing_socket_is_enoent() {
        let error = connect_unix(b"/nonexistent/rmux-test.sock").unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
    }

    #[test]
    fn signal_names_follow_platform() {
        assert_eq!(signal_name(0), b"0");
        assert_eq!(signal_name(-3), b"-3");
        #[cfg(target_os = "macos")]
        assert_eq!(signal_name(libc::SIGHUP), b"hup");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            signal_name(libc::SIGHUP),
            libc::SIGHUP.to_string().as_bytes()
        );
    }

    #[test]
    fn socketpair_and_inheritable() {
        // SAFETY: F_GETFD takes no pointers; fd is open for the borrow.
        let cloexec = |fd: BorrowedFd<'_>| unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) }
            & libc::FD_CLOEXEC
            != 0;
        let (a, b) = socketpair().unwrap();
        // Both ends close on exec until one is chosen to cross (proc.c:385).
        assert!(cloexec(a.as_fd()));
        assert!(cloexec(b.as_fd()));
        set_inheritable(a.as_fd()).unwrap();
        assert!(!cloexec(a.as_fd()));
        assert!(cloexec(b.as_fd()));
        let dup = dup_fd(b.as_fd()).unwrap();
        assert!(dup.as_raw_fd() > 2);
    }

    #[test]
    fn lstat_reports_directory() {
        let info = lstat_info(b"/").unwrap();
        assert!(info.is_dir);
        assert_eq!(info.uid, 0);
    }
}
