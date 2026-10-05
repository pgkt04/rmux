// Ported from tmux spawn.c, server-client.c, compat/setproctitle.c,
// compat/getpeereid.c, compat/closefrom.c, compat/daemon.c, compat/daemon-darwin.c @ 8f25579c

use std::io;
use std::os::fd::BorrowedFd;

use crate::{GroupId, ProcessId, UserId};

pub fn wait_exit_status(status: i32) -> Option<i32> {
    libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status))
}

pub fn wait_signal_name(status: i32) -> Option<Vec<u8>> {
    if !libc::WIFSIGNALED(status) {
        return None;
    }
    let signal = libc::WTERMSIG(status);
    #[cfg(target_os = "macos")]
    if signal > 0 && signal < 32 {
        unsafe extern "C" {
            static sys_signame: [*const libc::c_char; 32];
        }
        // SAFETY: Darwin exports NSIG entries; checked signal indexes it,
        // and each signal name is an immutable NUL-terminated string.
        return Some(
            unsafe { std::ffi::CStr::from_ptr(sys_signame[signal as usize]) }
                .to_bytes()
                .to_vec(),
        );
    }
    Some(signal.to_string().into_bytes())
}

pub fn home_directory(user: Option<&[u8]>) -> Option<Vec<u8>> {
    if user.is_none() {
        use std::os::unix::ffi::OsStringExt;
        if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
            return Some(home.into_vec());
        }
    }
    let name = user.map(crate::cstring::nul_terminated);
    // SAFETY: name is NUL-terminated; libc owns the returned record. Copy
    // its directory before any other passwd lookup can invalidate it.
    unsafe {
        let record = if let Some(name) = &name {
            libc::getpwnam(name.as_ptr().cast())
        } else {
            libc::getpwuid(libc::getuid())
        };
        if record.is_null() || (*record).pw_dir.is_null() {
            return None;
        }
        Some(
            std::ffi::CStr::from_ptr((*record).pw_dir)
                .to_bytes()
                .to_vec(),
        )
    }
}

/// `compat/setproctitle.c:29-47`: `<program>: <title>` in 16 bytes, cut at the
/// last retained space when truncated, then `PR_SET_NAME`.
#[cfg(target_os = "linux")]
pub fn setproctitle(program: &[u8], title: &[u8]) {
    const SIZE: usize = 16;
    fn cstr(bytes: &[u8]) -> &[u8] {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        &bytes[..end]
    }
    let program = cstr(program);
    let title = cstr(title);
    let title = &title[..title.len().min(SIZE - 1)];
    let mut full = Vec::with_capacity(program.len() + 2 + title.len());
    full.extend_from_slice(program);
    full.extend_from_slice(b": ");
    full.extend_from_slice(title);
    let used = full.len();
    let mut name = [0u8; SIZE];
    let keep = used.min(SIZE - 1);
    name[..keep].copy_from_slice(&full[..keep]);
    if used >= SIZE {
        if let Some(space) = name[..keep].iter().rposition(|&b| b == b' ') {
            name[space..].fill(0);
        }
    }
    // SAFETY: name is a NUL-terminated buffer of 16 bytes, the PR_SET_NAME limit.
    unsafe {
        libc::prctl(libc::PR_SET_NAME, name.as_ptr());
    }
}

/// `compat/setproctitle.c:49-52`: no process title support on this platform.
#[cfg(not(target_os = "linux"))]
pub fn setproctitle(_program: &[u8], _title: &[u8]) {}

/// `getpeereid(3)` (`proc.c:330`).
#[cfg(target_os = "macos")]
pub fn getpeereid(fd: BorrowedFd<'_>) -> Option<(UserId, GroupId)> {
    use std::os::fd::AsRawFd;

    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    // SAFETY: fd is open for the borrow; uid and gid are valid out pointers.
    if unsafe { libc::getpeereid(fd.as_raw_fd(), &mut uid, &mut gid) } == -1 {
        return None;
    }
    Some((UserId(uid), GroupId(gid)))
}

/// `compat/getpeereid.c:32-40`: `SO_PEERCRED`.
#[cfg(target_os = "linux")]
pub fn getpeereid(fd: BorrowedFd<'_>) -> Option<(UserId, GroupId)> {
    use std::os::fd::AsRawFd;

    // SAFETY: ucred is plain data; zero is a valid value.
    let mut uc: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: fd is open for the borrow; uc and len describe a valid buffer.
    let ret = unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut uc).cast(),
            &mut len,
        )
    };
    if ret == -1 {
        return None;
    }
    Some((UserId(uc.uid), GroupId(uc.gid)))
}

/// `compat/closefrom.c:65-85`: close every descriptor from lowfd to the table size.
fn closefrom_fallback(lowfd: i32) {
    // SAFETY: sysconf takes no pointers.
    let mut maxfd = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    if maxfd < 0 {
        maxfd = 256;
    }
    let mut fd = i64::from(lowfd);
    while fd < maxfd as i64 {
        // SAFETY: closing an arbitrary descriptor number is what closefrom does;
        // callers invoke it only where no Rust owner of those descriptors exists.
        unsafe {
            libc::close(fd as i32);
        }
        fd += 1;
    }
}

/// # Safety
/// The caller must exclusively own every open descriptor at or above `lowfd`,
/// with no live Rust fd owners or borrows for them.
/// `compat/closefrom.c:95-122`: `PROC_PIDLISTFDS`, then the brute-force fallback.
#[cfg(target_os = "macos")]
pub unsafe fn closefrom(lowfd: i32) {
    // SAFETY: getpid takes no arguments; a NULL buffer with size 0 only queries the size.
    let sz = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDLISTFDS,
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    if sz == 0 {
        return;
    }
    if sz > 0 {
        let count = sz as usize / size_of::<libc::proc_fdinfo>();
        let mut infos: Vec<libc::proc_fdinfo> = vec![
            libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0
            };
            count
        ];
        let bytes = (count * size_of::<libc::proc_fdinfo>()) as libc::c_int;
        // SAFETY: infos has room for `bytes` bytes of proc_fdinfo records.
        let r = unsafe {
            libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDLISTFDS,
                0,
                infos.as_mut_ptr().cast(),
                bytes,
            )
        };
        if r >= 0 && r <= bytes {
            let filled = r as usize / size_of::<libc::proc_fdinfo>();
            for info in &infos[..filled] {
                if info.proc_fd >= lowfd {
                    // SAFETY: the kernel reported this descriptor as open in this process.
                    unsafe {
                        libc::close(info.proc_fd);
                    }
                }
            }
            return;
        }
    }
    closefrom_fallback(lowfd);
}

/// # Safety
/// The caller must exclusively own every open descriptor at or above `lowfd`,
/// with no live Rust fd owners or borrows for them.
/// `close_range(2)`, then the `/proc/self/fd` scan (`compat/closefrom.c:124-147`),
/// then the brute-force fallback.
#[cfg(target_os = "linux")]
pub unsafe fn closefrom(lowfd: i32) {
    let first = libc::c_uint::try_from(lowfd).unwrap_or(0);
    // SAFETY: close_range takes only integer arguments.
    if unsafe {
        libc::syscall(
            libc::SYS_close_range,
            first,
            libc::c_uint::MAX,
            0 as libc::c_uint,
        )
    } == 0
    {
        return;
    }
    if let Ok(dir) = std::fs::read_dir("/proc/self/fd") {
        let fds: Vec<i32> = dir
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
            .filter(|&fd| fd >= lowfd)
            .collect();
        for fd in fds {
            // SAFETY: the kernel listed this descriptor as open; the directory
            // handle is already closed, so a stale entry only yields EBADF.
            unsafe {
                libc::close(fd);
            }
        }
        return;
    }
    closefrom_fallback(lowfd);
}

#[cfg(target_os = "macos")]
mod darwin {
    use libc::{kern_return_t, mach_port_t};

    unsafe extern "C" {
        pub static mut bootstrap_port: mach_port_t;
        // mach_task_self() is a macro over this global in <mach/mach_init.h>.
        pub static mut mach_task_self_: mach_port_t;
        pub fn bootstrap_get_root(bp: mach_port_t, root: *mut mach_port_t) -> kern_return_t;
        pub fn bootstrap_look_up_per_user(
            bp: mach_port_t,
            service_name: *const libc::c_char,
            uid: libc::uid_t,
            sp: *mut mach_port_t,
        ) -> kern_return_t;
        // task_set_bootstrap_port() is a macro over this in <mach/task_special_ports.h>.
        pub fn task_set_special_port(
            task: mach_port_t,
            which: libc::c_int,
            port: mach_port_t,
        ) -> kern_return_t;
        pub fn mach_port_deallocate(task: mach_port_t, name: mach_port_t) -> kern_return_t;
    }

    /// `TASK_BOOTSTRAP_PORT` from `<mach/task_special_ports.h>`.
    const TASK_BOOTSTRAP_PORT: libc::c_int = 4;

    /// `compat/daemon-darwin.c:63-76`: move the daemon into the per-user bootstrap namespace.
    pub fn daemon_darwin() {
        let mut root: mach_port_t = libc::MACH_PORT_NULL as mach_port_t;
        let mut s: mach_port_t = libc::MACH_PORT_NULL as mach_port_t;
        // SAFETY: bootstrap_port is the process bootstrap port global; the mach
        // calls receive valid out pointers and the task's own port. This runs in
        // the freshly forked single-threaded daemon, so no other thread reads
        // the global concurrently.
        unsafe {
            let uid = libc::getuid();
            if bootstrap_get_root(bootstrap_port, &mut root) == libc::KERN_SUCCESS
                && bootstrap_look_up_per_user(root, std::ptr::null(), uid, &mut s)
                    == libc::KERN_SUCCESS
                && task_set_special_port(mach_task_self_, TASK_BOOTSTRAP_PORT, s)
                    == libc::KERN_SUCCESS
                && mach_port_deallocate(mach_task_self_, bootstrap_port) == libc::KERN_SUCCESS
            {
                bootstrap_port = s;
            }
        }
    }
}

/// `compat/daemon.c:43-75`: fork, parent exits, `setsid`, optional `chdir("/")`
/// and `/dev/null` on 0/1/2. Returns in the child only.
///
/// # Safety
/// Call only in a single-threaded process with no live Rust owners or borrows
/// of descriptors 0, 1, and 2 when `noclose` is false. No inherited lock may be held.
pub unsafe fn daemon(nochdir: bool, noclose: bool) -> io::Result<()> {
    // SAFETY: fork and the following calls are the daemon(3) sequence; the
    // child continues with copies of this process's state and the parent
    // exits without running destructors, exactly like the C version.
    unsafe {
        match libc::fork() {
            -1 => return Err(io::Error::last_os_error()),
            0 => {}
            _ => libc::_exit(0),
        }
        if libc::setsid() == -1 {
            return Err(io::Error::last_os_error());
        }
        if !nochdir {
            libc::chdir(c"/".as_ptr());
        }
        if !noclose {
            let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR, 0);
            if fd != -1 {
                libc::dup2(fd, libc::STDIN_FILENO);
                libc::dup2(fd, libc::STDOUT_FILENO);
                libc::dup2(fd, libc::STDERR_FILENO);
                if fd > 2 {
                    libc::close(fd);
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    darwin::daemon_darwin();
    Ok(())
}

pub fn getpid() -> ProcessId {
    // SAFETY: getpid takes no arguments and cannot fail.
    ProcessId(unsafe { libc::getpid() })
}

pub fn getuid() -> UserId {
    // SAFETY: getuid takes no arguments and cannot fail.
    UserId(unsafe { libc::getuid() })
}

/// Wait for this owned child only, preserving the raw wait status for the model.
pub fn wait_process(pid: ProcessId, nonblocking: bool) -> io::Result<Option<i32>> {
    let mut status = 0;
    loop {
        // SAFETY: status is writable; pid designates the child being waited for.
        let result = unsafe {
            libc::waitpid(
                pid.0,
                &mut status,
                if nonblocking { libc::WNOHANG } else { 0 },
            )
        };
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        return Ok((result != 0).then_some(status));
    }
}

pub fn exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        128 + libc::SIGHUP
    }
}

/// Terminate one owned child; never signal unrelated processes or groups.
pub fn terminate_process(pid: ProcessId) -> io::Result<()> {
    // SAFETY: kill receives an explicit child pid and a valid signal number.
    if unsafe { libc::kill(pid.0, libc::SIGKILL) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Create the editor file with mkstemp's exclusive, mode-0600 semantics.
pub fn temporary_file(template: &[u8]) -> io::Result<(std::fs::File, Vec<u8>)> {
    use std::os::fd::FromRawFd;
    let mut path = std::ffi::CString::new(template)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "temporary path contains NUL"))?
        .into_bytes_with_nul();
    // SAFETY: path is a mutable NUL-terminated template; mkstemp returns a fresh fd.
    let fd = unsafe { libc::mkstemp(path.as_mut_ptr().cast()) };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    path.pop();
    // SAFETY: successful mkstemp transfers unique ownership of its new fd.
    Ok((unsafe { std::fs::File::from_raw_fd(fd) }, path))
}
