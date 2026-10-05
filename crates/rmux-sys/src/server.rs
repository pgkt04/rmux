// Ported from tmux server.c, proc.c, job.c, file.c, cmd-pipe-pane.c,
// cmd-source-file.c, cmd-server-access.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
use crate::{GroupId, ProcessId, UserId};
use signal_hook::iterator::{backend::SignalDelivery, exfiltrator::SignalOnly};
use std::ffi::{CStr, CString};
use std::io::{self, IoSlice};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};

/// A thread-affine mask guard; every parent return restores the previous mask.
pub struct SignalMask {
    old: libc::sigset_t,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl SignalMask {
    pub fn block() -> io::Result<Self> {
        // SAFETY: signal sets are plain C storage initialized by these calls.
        let (mut set, mut old) = unsafe { std::mem::zeroed::<(libc::sigset_t, libc::sigset_t)>() };
        // SAFETY: both pointers designate live signal sets.
        unsafe {
            libc::sigfillset(&mut set);
            if libc::sigprocmask(libc::SIG_BLOCK, &set, &mut old) == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self {
            old,
            _thread: std::marker::PhantomData,
        })
    }
}
impl Drop for SignalMask {
    fn drop(&mut self) {
        // SAFETY: this guard stays on its creating thread and owns the saved mask.
        unsafe {
            libc::sigprocmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut());
        }
    }
}

const RUNTIME_SIGNALS: [i32; 8] = [
    libc::SIGINT,
    libc::SIGHUP,
    libc::SIGCHLD,
    libc::SIGCONT,
    libc::SIGTERM,
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGWINCH,
];
const IGNORED_SIGNALS: [i32; 5] = [
    libc::SIGPIPE,
    libc::SIGTSTP,
    libc::SIGTTIN,
    libc::SIGTTOU,
    libc::SIGQUIT,
];

/// Self-pipe readiness plus coalesced pending signal numbers, without a worker thread.
pub struct SignalWake {
    delivery: Option<SignalDelivery<UnixStream, SignalOnly>>,
    previous: Vec<(i32, libc::sigaction)>,
}
impl SignalWake {
    pub fn new() -> io::Result<Self> {
        let _mask = SignalMask::block()?;
        let (read, write) = UnixStream::pair()?;
        read.set_nonblocking(true)?;
        write.set_nonblocking(true)?;
        let mut wake = Self {
            delivery: None,
            previous: Vec::with_capacity(13),
        };
        // SAFETY: sigaction is initialized C storage and all signal numbers are valid.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = libc::SIG_IGN;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: action's signal mask is live writable storage.
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        for signal in IGNORED_SIGNALS {
            // SAFETY: old is writable storage for the previous disposition.
            let mut old = unsafe { std::mem::zeroed() };
            // SAFETY: action and old stay alive for the sigaction call.
            if unsafe { libc::sigaction(signal, &action, &mut old) } == -1 {
                return Err(io::Error::last_os_error());
            }
            wake.previous.push((signal, old));
        }
        for signal in RUNTIME_SIGNALS {
            // SAFETY: old is writable signal storage; a null action queries only.
            let mut old = unsafe { std::mem::zeroed() };
            // SAFETY: the query does not change process disposition.
            if unsafe { libc::sigaction(signal, std::ptr::null(), &mut old) } == -1 {
                return Err(io::Error::last_os_error());
            }
            wake.previous.push((signal, old));
        }
        wake.delivery = Some(SignalDelivery::with_pipe(
            read,
            write,
            SignalOnly,
            RUNTIME_SIGNALS,
        )?);
        Ok(wake)
    }
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.delivery
            .as_ref()
            .expect("live signal delivery")
            .get_read()
            .as_fd()
    }
    pub fn drain(&mut self) -> io::Result<Vec<i32>> {
        Ok(self
            .delivery
            .as_mut()
            .expect("live signal delivery")
            .pending()
            .collect())
    }
}
impl Drop for SignalWake {
    fn drop(&mut self) {
        self.delivery.take();
        for (signal, previous) in self.previous.drain(..).rev() {
            // SAFETY: previous was returned by sigaction for this signal.
            unsafe {
                libc::sigaction(signal, &previous, std::ptr::null_mut());
            }
        }
    }
}

/// ECHILD and no ready child both end the SIGCHLD drain.
pub fn wait_any() -> io::Result<Option<(ProcessId, i32)>> {
    let mut status = 0;
    // SAFETY: status is a valid out pointer; -1 selects any child.
    let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::WUNTRACED) };
    match pid {
        0 => Ok(None),
        -1 => {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                Ok(None)
            } else {
                Err(error)
            }
        }
        _ => Ok(Some((ProcessId(pid), status))),
    }
}
pub fn stop_signal(status: i32) -> Option<i32> {
    libc::WIFSTOPPED(status).then(|| libc::WSTOPSIG(status))
}
pub fn status_exited(status: i32) -> bool {
    libc::WIFEXITED(status)
}
pub fn status_signaled(status: i32) -> bool {
    libc::WIFSIGNALED(status)
}
pub fn status_signal(status: i32) -> Option<i32> {
    libc::WIFSIGNALED(status).then(|| libc::WTERMSIG(status))
}
pub fn terminal_stop(status: i32) -> bool {
    matches!(stop_signal(status), Some(libc::SIGTTIN | libc::SIGTTOU))
}
pub fn continue_process_group(pid: ProcessId) -> io::Result<()> {
    if pid.0 <= 0 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    // SAFETY: killpg changes process state only; the positive group id is checked.
    if unsafe { libc::killpg(pid.0, libc::SIGCONT) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn continue_process(pid: ProcessId) -> io::Result<()> {
    if pid.0 <= 0 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    // SAFETY: kill changes process state only; this cannot target an implicit group.
    if unsafe { libc::kill(pid.0, libc::SIGCONT) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn shutdown_write(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fd is open throughout the syscall.
    if unsafe { libc::shutdown(fd.as_raw_fd(), libc::SHUT_WR) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn close(fd: OwnedFd) -> io::Result<()> {
    // SAFETY: consuming the sole owner prevents subsequent close or use of this descriptor.
    if unsafe { libc::close(fd.into_raw_fd()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn duplicate_standard(fd: i32) -> io::Result<OwnedFd> {
    if !(0..=2).contains(&fd) {
        return Err(io::Error::from_raw_os_error(libc::EBADF));
    }
    // SAFETY: fcntl duplicates the descriptor without changing existing ownership.
    let raw = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl returned a new descriptor owned only here.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}
pub fn write_vectored(fd: BorrowedFd<'_>, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
    let mut vectors = [libc::iovec {
        iov_base: std::ptr::null_mut(),
        iov_len: 0,
    }; 16];
    let count = buffers.len().min(vectors.len());
    for (vector, buffer) in vectors.iter_mut().zip(buffers) {
        vector.iov_base = buffer.as_ptr().cast_mut().cast();
        vector.iov_len = buffer.len();
    }
    // SAFETY: vectors borrow live immutable slices; writev does not modify them.
    let written = unsafe { libc::writev(fd.as_raw_fd(), vectors.as_ptr(), count as i32) };
    if written == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(written as usize)
}

static BIND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub fn unix_path_limit() -> usize {
    // SAFETY: a zeroed sockaddr_un is valid storage used only for its array length.
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_path.len()
}
pub fn bind_listener(path: &[u8], default_socket: bool) -> io::Result<UnixListener> {
    // SAFETY: sockaddr_un is plain C storage.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.len() >= address.sun_path.len() {
        return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
    }
    let path = CString::new(path).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    address.sun_family = libc::AF_UNIX as _;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = size_of::<libc::sockaddr_un>() as _;
    }
    for (out, byte) in address.sun_path.iter_mut().zip(path.as_bytes()) {
        *out = *byte as _;
    }
    // SAFETY: path is terminated; unlink errors are intentionally ignored as in C.
    unsafe {
        libc::unlink(path.as_ptr());
    }
    // SAFETY: socket has no pointer arguments and returns a new descriptor.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw is a fresh descriptor with no other owner.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let fd = above_standard_fds(fd)?;
    let _lock = BIND_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mode = if default_socket { 0o117 } else { 0o177 };
    // SAFETY: umask changes process policy; serialized for all listener binds.
    let previous = unsafe { libc::umask(mode) };
    // SAFETY: address is a complete initialized sockaddr_un.
    let result = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&raw const address).cast(),
            size_of::<libc::sockaddr_un>() as _,
        )
    };
    let error = (result == -1).then(io::Error::last_os_error);
    // SAFETY: restore even after bind failure before reporting the captured errno.
    unsafe {
        libc::umask(previous);
    }
    drop(_lock);
    if let Some(error) = error {
        return Err(error);
    }
    // SAFETY: fd is the bound socket; backlog is pinned to C.
    if unsafe { libc::listen(fd.as_raw_fd(), 128) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let listener = UnixListener::from(fd);
    listener.set_nonblocking(true)?;
    Ok(listener)
}

pub fn user_name(uid: UserId) -> Option<Vec<u8>> {
    account_name(Some(uid), None)
}
pub fn group_name(gid: GroupId) -> Option<Vec<u8>> {
    account_name(None, Some(gid))
}
fn account_name(uid: Option<UserId>, gid: Option<GroupId>) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 1024];
    loop {
        // SAFETY: records are plain C storage; _r calls populate the selected one.
        let (mut user, mut group): (libc::passwd, libc::group) = unsafe { std::mem::zeroed() };
        let (mut user_result, mut group_result) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: record, result and backing buffer pointers are live and disjoint.
        let error = unsafe {
            if let Some(uid) = uid {
                libc::getpwuid_r(
                    uid.0,
                    &mut user,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut user_result,
                )
            } else {
                libc::getgrgid_r(
                    gid?.0,
                    &mut group,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut group_result,
                )
            }
        };
        if error == libc::ERANGE {
            buffer.resize(buffer.len().checked_mul(2)?, 0);
            continue;
        }
        if error != 0 {
            return None;
        }
        let name = if uid.is_some() {
            if user_result.is_null() {
                return None;
            }
            user.pw_name
        } else {
            if group_result.is_null() {
                return None;
            }
            group.gr_name
        };
        if name.is_null() {
            return None;
        }
        // SAFETY: the successful lookup returned a terminated name inside buffer.
        return Some(unsafe { CStr::from_ptr(name) }.to_bytes().to_vec());
    }
}

pub enum ExecCommand {
    Shell { shell: Vec<u8>, command: Vec<u8> },
    Argv(Vec<Vec<u8>>),
}
pub struct JobLaunchOptions {
    pub command: ExecCommand,
    pub environment: Vec<Vec<u8>>,
    pub cwd: Option<Vec<u8>>,
    pub home: Option<Vec<u8>>,
    pub pty: bool,
    pub show_stderr: bool,
    pub size: crate::pty::Winsize,
}
pub struct JobProcess {
    pub pid: ProcessId,
    pub fd: OwnedFd,
    pub tty: Vec<u8>,
}
pub struct PreparedJob {
    arguments: Vec<CString>,
    executables: Vec<CString>,
    fallback: Vec<CString>,
    directories: Vec<CString>,
    environment: Vec<CString>,
    working_directories: Vec<CString>,
    pty: bool,
    show_stderr: bool,
    direct: bool,
    size: crate::pty::Winsize,
    max_fd: i32,
}
fn cstring(value: Vec<u8>) -> io::Result<CString> {
    CString::new(value)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "job string contains NUL"))
}
impl PreparedJob {
    pub fn new(options: JobLaunchOptions) -> io::Result<Self> {
        let (program, arguments, direct) = match options.command {
            ExecCommand::Shell { shell, command } => {
                let name = shell
                    .rsplit(|b| *b == b'/')
                    .next()
                    .unwrap_or(&shell)
                    .to_vec();
                (shell, vec![name, b"-c".to_vec(), command], false)
            }
            ExecCommand::Argv(argv) => {
                let program = argv
                    .first()
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?
                    .clone();
                (program, argv, true)
            }
        };
        let executables = if direct && !program.contains(&b'/') {
            options
                .environment
                .iter()
                .find_map(|v| v.strip_prefix(b"PATH="))
                .unwrap_or(crate::pty::default_search_path())
                .split(|b| *b == b':')
                .map(|directory| {
                    if directory.is_empty() {
                        return cstring(program.clone());
                    }
                    let mut path = Vec::with_capacity(directory.len() + program.len() + 1);
                    path.extend_from_slice(directory);
                    path.push(b'/');
                    path.extend_from_slice(&program);
                    cstring(path)
                })
                .collect::<io::Result<Vec<_>>>()?
        } else {
            vec![cstring(program)?]
        };
        let arguments = arguments
            .into_iter()
            .map(cstring)
            .collect::<io::Result<Vec<_>>>()?;
        let mut fallback = vec![cstring(b"sh".to_vec())?];
        if direct {
            fallback.extend(arguments.iter().skip(1).cloned());
        }
        let mut directories = Vec::new();
        if let Some(cwd) = options.cwd {
            directories.push(cstring(cwd)?);
            if let Some(home) = options.home {
                directories.push(cstring(home)?);
            }
            directories.push(cstring(b"/".to_vec())?);
        }
        let environment = options
            .environment
            .into_iter()
            .filter(|v| directories.is_empty() || !v.starts_with(b"PWD="))
            .map(cstring)
            .collect::<io::Result<Vec<_>>>()?;
        let working_directories = directories
            .iter()
            .map(|directory| {
                let mut pwd = b"PWD=".to_vec();
                pwd.extend_from_slice(directory.as_bytes());
                cstring(pwd)
            })
            .collect::<io::Result<Vec<_>>>()?;
        // SAFETY: sysconf has no pointer arguments and runs before fork.
        let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
        Ok(Self {
            arguments,
            executables,
            fallback,
            directories,
            environment,
            working_directories,
            pty: options.pty,
            show_stderr: options.show_stderr,
            direct,
            size: options.size,
            max_fd: if max < 0 {
                256
            } else {
                max.min(i32::MAX as _) as i32
            },
        })
    }
    /// No allocation, callbacks, Rust destructor, or unwinding in the fork child.
    pub fn launch(self) -> io::Result<JobProcess> {
        let mut arguments = self
            .arguments
            .iter()
            .map(|v| v.as_ptr())
            .collect::<Vec<_>>();
        arguments.push(std::ptr::null());
        let environment = |pwd: Option<&CString>| {
            let mut pointers = Vec::with_capacity(self.environment.len() + 2);
            pointers.extend(self.environment.iter().map(|v| v.as_ptr()));
            if let Some(pwd) = pwd {
                pointers.push(pwd.as_ptr());
            }
            pointers.push(std::ptr::null());
            pointers
        };
        let environments = if self.working_directories.is_empty() {
            vec![environment(None)]
        } else {
            self.working_directories
                .iter()
                .map(|pwd| environment(Some(pwd)))
                .collect()
        };
        let mut fallback = vec![self.fallback[0].as_ptr(), std::ptr::null()];
        fallback.extend(self.fallback.iter().skip(1).map(|v| v.as_ptr()));
        fallback.push(std::ptr::null());
        let mask = SignalMask::block()?;
        let (parent, child, tty) = if self.pty {
            let (parent, child, tty) = crate::pty::openpty()?;
            crate::pty::set_winsize(child.as_fd(), self.size)?;
            (above_standard_fds(parent)?, above_standard_fds(child)?, tty)
        } else {
            let (parent, child) = UnixStream::pair()?;
            (
                above_standard_fds(parent.into())?,
                above_standard_fds(child.into())?,
                Vec::new(),
            )
        };
        let parent_fd = parent.as_raw_fd();
        let child_fd = child.as_raw_fd();
        // SAFETY: sigaction is plain initialized storage prepared before fork.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = libc::SIG_DFL;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: action mask is valid writable storage.
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        // SAFETY: the child uses only prepared buffers and async-signal-safe libc;
        // every path terminates in exec or _exit without Rust cleanup.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // SAFETY: child-private process state; all pointers and descriptors were
            // prepared before fork and are never exposed to a Rust callback.
            unsafe {
                libc::close(parent_fd);
                if self.pty
                    && (libc::setsid() == -1
                        || libc::ioctl(child_fd, libc::TIOCSCTTY as _, 0) == -1)
                {
                    libc::_exit(1);
                }
                for fd in 0..=1 {
                    if libc::dup2(child_fd, fd) == -1 {
                        libc::_exit(1);
                    }
                }
                let stderr = if self.pty || self.show_stderr {
                    child_fd
                } else {
                    libc::open(c"/dev/null".as_ptr(), libc::O_RDWR)
                };
                if stderr == -1 || libc::dup2(stderr, 2) == -1 {
                    libc::_exit(1);
                }
                for signal in
                    RUNTIME_SIGNALS
                        .into_iter()
                        .chain([libc::SIGPIPE, libc::SIGTSTP, libc::SIGQUIT])
                {
                    libc::sigaction(signal, &action, std::ptr::null_mut());
                }
                libc::sigprocmask(libc::SIG_SETMASK, &mask.old, std::ptr::null_mut());
                let mut actual = 0;
                if !self.directories.is_empty() {
                    let mut found = false;
                    for (index, directory) in self.directories.iter().enumerate() {
                        if libc::chdir(directory.as_ptr()) == 0 {
                            actual = index;
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        libc::_exit(1);
                    }
                }
                close_child_fds(self.max_fd);
                for executable in &self.executables {
                    libc::execve(
                        executable.as_ptr(),
                        arguments.as_ptr(),
                        environments[actual].as_ptr(),
                    );
                    #[cfg(target_os = "macos")]
                    let error = *libc::__error();
                    #[cfg(target_os = "linux")]
                    let error = *libc::__errno_location();
                    if self.direct && error == libc::ENOEXEC {
                        fallback[1] = executable.as_ptr();
                        libc::execve(
                            c"/bin/sh".as_ptr(),
                            fallback.as_ptr(),
                            environments[actual].as_ptr(),
                        );
                        libc::_exit(1);
                    }
                    if !self.direct
                        || !matches!(
                            error,
                            libc::EACCES
                                | libc::ENOENT
                                | libc::ENOTDIR
                                | libc::ESTALE
                                | libc::ENODEV
                                | libc::ETIMEDOUT
                        )
                    {
                        libc::_exit(1);
                    }
                }
                libc::_exit(1);
            }
        }
        let error = (pid == -1).then(io::Error::last_os_error);
        drop(mask);
        if let Some(error) = error {
            return Err(error);
        }
        drop(child);
        crate::fd::set_blocking(parent.as_fd(), false);
        Ok(JobProcess {
            pid: ProcessId(pid),
            fd: parent,
            tty,
        })
    }
}
fn above_standard_fds(fd: OwnedFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() > 2 {
        // SAFETY: fcntl changes flags of a descriptor whose ownership is unchanged.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
        return Ok(fd);
    }
    // SAFETY: duplicating keeps 0/1/2 free of child-source aliasing.
    let raw = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl created a new descriptor owned only here.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}
unsafe fn close_child_fds(max_fd: i32) {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: invoked only in the fork child, which will not run fd destructors.
        if unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) } == 0 {
            return;
        }
    }
    for fd in 3..max_fd {
        // SAFETY: close accepts arbitrary descriptor numbers in this child.
        unsafe {
            libc::close(fd);
        }
    }
}

pub fn descriptor_is_regular(fd: BorrowedFd<'_>) -> io::Result<bool> {
    // SAFETY: stat is plain C storage initialized by fstat.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: descriptor stays open and stat is a live writable out pointer.
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(stat.st_mode & libc::S_IFMT == libc::S_IFREG)
}

pub fn uppercase(byte: u8) -> u8 {
    // SAFETY: toupper accepts unsigned-byte values represented as int.
    unsafe { libc::toupper(i32::from(byte)) as u8 }
}

/// Install proc.c's SIGPIPE policy in standalone I/O owners before writing.
pub fn ignore_sigpipe() -> io::Result<()> {
    // SAFETY: action is plain C storage initialized before sigaction.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_IGN;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: valid mask storage and SIGPIPE disposition; no borrowed data escapes.
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGPIPE, &action, std::ptr::null_mut()) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Detach the freshly reexecuted server root; stdio ownership stays with Command.
pub fn detach_session() -> io::Result<()> {
    // SAFETY: setsid changes process session state without touching Rust resources.
    if unsafe { libc::setsid() } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn user_home_directory(uid: UserId) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 1024];
    loop {
        // SAFETY: passwd is plain storage populated by the reentrant lookup.
        let mut user: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        // SAFETY: record, result and backing buffer designate live disjoint storage.
        let error = unsafe {
            libc::getpwuid_r(
                uid.0,
                &mut user,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE {
            buffer.resize(buffer.len().checked_mul(2)?, 0);
            continue;
        }
        if error != 0 || result.is_null() || user.pw_dir.is_null() {
            return None;
        }
        // SAFETY: successful lookup places a terminated directory in buffer.
        return Some(unsafe { CStr::from_ptr(user.pw_dir) }.to_bytes().to_vec());
    }
}

pub fn make_fifo(path: &[u8], mode: u32) -> io::Result<()> {
    let path = CString::new(path).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: path is terminated and mkfifo retains no pointers.
    if unsafe { libc::mkfifo(path.as_ptr(), mode as libc::mode_t) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn pending_bytes(fd: BorrowedFd<'_>) -> io::Result<usize> {
    let mut count: libc::c_int = 0;
    // SAFETY: descriptor stays open and count is live writable ioctl storage.
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIONREAD, &mut count) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(count.max(0) as usize)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobError {
    NoMatch,
    NoSpace,
    Other,
}
pub fn glob(pattern: &[u8]) -> Result<Vec<Vec<u8>>, GlobError> {
    let pattern = CString::new(pattern).map_err(|_| GlobError::Other)?;
    // SAFETY: glob_t is plain zero-initialized storage for libc glob.
    let mut matches: libc::glob_t = unsafe { std::mem::zeroed() };
    // SAFETY: pattern and output are live; flags 0 requires no error callback.
    let result = unsafe { libc::glob(pattern.as_ptr(), 0, None, &mut matches) };
    let output = if result == 0 {
        let mut output = Vec::with_capacity(matches.gl_pathc);
        for index in 0..matches.gl_pathc {
            // SAFETY: successful glob supplies gl_pathc terminated strings.
            output.push(
                unsafe { CStr::from_ptr(*matches.gl_pathv.add(index)) }
                    .to_bytes()
                    .to_vec(),
            );
        }
        Ok(output)
    } else {
        Err(match result {
            libc::GLOB_NOMATCH => GlobError::NoMatch,
            libc::GLOB_NOSPACE => GlobError::NoSpace,
            _ => GlobError::Other,
        })
    };
    // SAFETY: glob initialized the record even on partial allocation failure.
    unsafe {
        libc::globfree(&mut matches);
    }
    output
}
pub fn user_by_name(name: &[u8]) -> Option<UserId> {
    let name = CString::new(name).ok()?;
    let mut buffer = vec![0u8; 1024];
    loop {
        // SAFETY: passwd is plain storage populated by the reentrant lookup.
        let mut user: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        // SAFETY: name is terminated and result/backing storage stay live.
        let error = unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                &mut user,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE {
            buffer.resize(buffer.len().checked_mul(2)?, 0);
            continue;
        }
        if error != 0 || result.is_null() {
            return None;
        }
        return Some(UserId(user.pw_uid));
    }
}
pub fn group_by_name(name: &[u8]) -> Option<GroupId> {
    let name = CString::new(name).ok()?;
    let mut buffer = vec![0u8; 1024];
    loop {
        // SAFETY: group is plain storage populated by the reentrant lookup.
        let mut group: libc::group = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        // SAFETY: name is terminated and result/backing storage stay live.
        let error = unsafe {
            libc::getgrnam_r(
                name.as_ptr(),
                &mut group,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE {
            buffer.resize(buffer.len().checked_mul(2)?, 0);
            continue;
        }
        if error != 0 || result.is_null() {
            return None;
        }
        return Some(GroupId(group.gr_gid));
    }
}
#[derive(Debug)]
pub enum PipeSpawnError {
    Socketpair(io::Error),
    Fork(io::Error),
}
impl std::fmt::Display for PipeSpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Socketpair(error) => write!(f, "socketpair error: {error}"),
            Self::Fork(error) => write!(f, "fork error: {error}"),
        }
    }
}
impl std::error::Error for PipeSpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Socketpair(error) | Self::Fork(error) => error,
        })
    }
}
pub fn spawn_pipe_child(
    command: &[u8],
    input: bool,
    output: bool,
) -> Result<(OwnedFd, ProcessId), PipeSpawnError> {
    let command = CString::new(command)
        .map_err(|_| PipeSpawnError::Fork(io::Error::from_raw_os_error(libc::EINVAL)))?;
    let (parent, child) = UnixStream::pair().map_err(PipeSpawnError::Socketpair)?;
    let parent = above_standard_fds(parent.into()).map_err(PipeSpawnError::Socketpair)?;
    let child = above_standard_fds(child.into()).map_err(PipeSpawnError::Socketpair)?;
    // SAFETY: sysconf has no pointer arguments and runs before fork.
    let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    let max_fd = if max < 0 {
        256
    } else {
        max.min(i32::MAX as _) as i32
    };
    // SAFETY: sigaction is plain initialized storage prepared before fork.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: action mask designates live writable storage.
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    let mask = SignalMask::block().map_err(PipeSpawnError::Fork)?;
    // SAFETY: child uses only prepared storage and async-signal-safe calls, then exec/_exit.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // SAFETY: child-private state; no Rust cleanup or callback is reachable.
        unsafe {
            for signal in
                RUNTIME_SIGNALS
                    .into_iter()
                    .chain([libc::SIGPIPE, libc::SIGTSTP, libc::SIGQUIT])
            {
                libc::sigaction(signal, &action, std::ptr::null_mut());
            }
            libc::sigprocmask(libc::SIG_SETMASK, &mask.old, std::ptr::null_mut());
            libc::close(parent.as_raw_fd());
            if libc::setpgid(0, 0) == -1 {
                libc::_exit(1);
            }
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            if null == -1 {
                libc::_exit(1);
            }
            let stdin = if output { child.as_raw_fd() } else { null };
            let stdout = if input { child.as_raw_fd() } else { null };
            if libc::dup2(stdin, 0) == -1
                || libc::dup2(stdout, 1) == -1
                || libc::dup2(null, 2) == -1
            {
                libc::_exit(1);
            }
            close_child_fds(max_fd);
            let arguments = [
                c"sh".as_ptr(),
                c"-c".as_ptr(),
                command.as_ptr(),
                std::ptr::null(),
            ];
            libc::execv(c"/bin/sh".as_ptr(), arguments.as_ptr());
            libc::_exit(1);
        }
    }
    let error = (pid == -1).then(io::Error::last_os_error);
    drop(mask);
    if let Some(error) = error {
        return Err(PipeSpawnError::Fork(error));
    }
    drop(child);
    crate::fd::set_blocking(parent.as_fd(), false);
    Ok((parent, ProcessId(pid)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    fn options(command: &[u8]) -> JobLaunchOptions {
        JobLaunchOptions {
            command: ExecCommand::Shell {
                shell: b"/bin/sh".to_vec(),
                command: command.to_vec(),
            },
            environment: vec![b"PATH=/usr/bin:/bin".to_vec()],
            cwd: None,
            home: None,
            pty: false,
            show_stderr: false,
            size: crate::pty::Winsize {
                cols: 80,
                rows: 24,
                ..Default::default()
            },
        }
    }
    fn capture(options: JobLaunchOptions, input: &[u8]) -> (Vec<u8>, i32) {
        let process = PreparedJob::new(options).unwrap().launch().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = Vec::new();
        let mut offset = 0;
        let mut closed = false;
        let mut status = None;
        loop {
            if offset != input.len() {
                match crate::fd::write(process.fd.as_fd(), &input[offset..]) {
                    Ok(n) => offset += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => panic!("job write: {e}"),
                }
            } else if !closed {
                let _ = shutdown_write(process.fd.as_fd());
                closed = true;
            }
            let mut buffer = [0; 8192];
            let eof = match crate::fd::read(process.fd.as_fd(), &mut buffer) {
                Ok(0) => true,
                Ok(n) => {
                    output.extend_from_slice(&buffer[..n]);
                    false
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    false
                }
                Err(e) if e.raw_os_error() == Some(libc::EIO) => true,
                Err(e) => panic!("job read: {e}"),
            };
            if status.is_none() {
                status = crate::proc::wait_process(process.pid, true).unwrap();
            }
            if eof {
                if let Some(status) = status {
                    return (output, crate::proc::exit_code(status));
                }
            }
            if Instant::now() > deadline {
                if status.is_none() {
                    let _ = crate::proc::terminate_process(process.pid);
                    let _ = crate::proc::wait_process(process.pid, false);
                }
                panic!("job capture timed out");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn isolated(name: &str) -> bool {
        if std::env::var("RMUX_SYS_SERVER_TEST").as_deref() == Ok(name) {
            return true;
        }
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("server::tests::{name}"), "--nocapture"])
            .env("RMUX_SYS_SERVER_TEST", name)
            .status()
            .unwrap();
        assert!(result.success(), "isolated syscall fixture {name}");
        false
    }
    #[test]
    fn job_duplex_stderr_and_direct_argv() {
        let (output, status) = capture(options(b"cat; printf error >&2; exit 7"), b"binary\0input");
        assert_eq!(output, b"binary\0input");
        assert_eq!(status, 7);
        let mut policy = options(b"printf output; printf error >&2");
        policy.show_stderr = true;
        assert_eq!(capture(policy, b"").0, b"outputerror");
        let mut policy = options(b"");
        policy.command = ExecCommand::Argv(vec![b"true".to_vec()]);
        assert_eq!(capture(policy, b"").1, 0);
        let mut policy = options(b"");
        policy.command = ExecCommand::Argv(vec![
            b"printf".to_vec(),
            b"<%s>".to_vec(),
            b"$SHELL ; literal".to_vec(),
        ]);
        assert_eq!(capture(policy, b"").0, b"<$SHELL ; literal>");
    }
    #[test]
    fn job_cwd_and_nonlogin_shell_semantics() {
        let before = std::env::current_dir().unwrap();
        let mut policy = options(b"printf '%s|%s|%s' \"$0\" \"$PWD\" \"$(pwd)\"");
        policy.cwd = Some(b"/rmux-nonexistent-job-cwd".to_vec());
        policy.home = Some(b"/tmp".to_vec());
        assert_eq!(capture(policy, b"").0, b"sh|/tmp|/tmp");
        let mut policy = options(b"printf '%s|%s' \"$PWD\" \"$(pwd)\"");
        policy.cwd = Some(b"/rmux-nonexistent-job-cwd".to_vec());
        policy.home = Some(b"/rmux-nonexistent-job-home".to_vec());
        assert_eq!(capture(policy, b"").0, b"/|/");
        assert_eq!(std::env::current_dir().unwrap(), before);
        assert!(PreparedJob::new(options(b"echo\0bad")).is_err());
        let mut policy = options(b"");
        policy.command = ExecCommand::Argv(Vec::new());
        assert!(PreparedJob::new(policy).is_err());
    }
    #[test]
    fn job_pty_initial_size_and_exec_failure() {
        let mut policy = options(b"stty size");
        policy.pty = true;
        policy.size.cols = 91;
        policy.size.rows = 37;
        assert_eq!(capture(policy, b"").0, b"37 91\r\n");
        let mut policy = options(b"");
        policy.command = ExecCommand::Argv(vec![b"rmux-nonexistent-job-program".to_vec()]);
        assert_eq!(capture(policy, b"").1, 1);
    }
    #[test]
    fn prepared_job_path_search_and_enoexec_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("rmux-job-exec-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("plain");
        std::fs::write(&script, b"printf 'fallback:%s' \"$1\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut policy = options(b"");
        policy.environment = vec![
            [
                b"PATH=".as_slice(),
                directory.as_os_str().as_encoded_bytes(),
            ]
            .concat(),
        ];
        policy.command = ExecCommand::Argv(vec![b"plain".to_vec(), b"argument".to_vec()]);
        let (output, status) = capture(policy, b"");
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(status, 0);
        assert_eq!(output, b"fallback:argument");
    }
    #[test]
    fn prepared_job_restores_mask_and_closes_other_descriptors() {
        // SAFETY: query writes a valid local set and does not change the mask.
        let mut before: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: a null new set queries the current thread mask.
        assert_eq!(
            unsafe { libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut before) },
            0
        );
        let sentinel = std::fs::File::open("/dev/null").unwrap();
        // SAFETY: duplicate a live fd into the high descriptor range before fork.
        let raw = unsafe { libc::fcntl(sentinel.as_raw_fd(), libc::F_DUPFD, 128) };
        assert!(raw >= 128);
        // SAFETY: fcntl returned a fresh descriptor owned here.
        let sentinel = unsafe { OwnedFd::from_raw_fd(raw) };
        let command = format!(
            "if test -e /dev/fd/{}; then exit 9; fi; exit 0",
            sentinel.as_raw_fd()
        );
        assert_eq!(capture(options(command.as_bytes()), b"").1, 0);
        // SAFETY: query writes the valid local mask storage.
        let mut after: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: null new set requests the current mask only.
        assert_eq!(
            unsafe { libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut after) },
            0
        );
        for signal in 1..32 {
            // SAFETY: both masks are initialized and signals are valid on both platforms.
            assert_eq!(unsafe { libc::sigismember(&before, signal) }, unsafe {
                libc::sigismember(&after, signal)
            });
        }
    }
    #[test]
    fn listener_umask_pathlimit_backlog_and_nonblocking() {
        if !isolated("listener_umask_pathlimit_backlog_and_nonblocking") {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rmux-listener-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: confined subprocess owns process-global umask for this fixture.
        let previous = unsafe { libc::umask(0o027) };
        for (default_socket, expected) in [(true, 0o660), (false, 0o600)] {
            let path = dir.join(if default_socket { "default" } else { "custom" });
            let listener =
                bind_listener(path.as_os_str().as_encoded_bytes(), default_socket).unwrap();
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                expected
            );
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            let mut sockets = Vec::new();
            for _ in 0..32 {
                sockets.push(UnixStream::connect(&path).unwrap());
            }
            for _ in 0..32 {
                listener.accept().unwrap();
            }
            // SAFETY: confined test observes and restores its process umask.
            let restored = unsafe { libc::umask(0o027) };
            assert_eq!(restored, 0o027);
        }
        let missing = dir.join("missing/endpoint");
        assert!(bind_listener(missing.as_os_str().as_encoded_bytes(), true).is_err());
        assert_eq!(
            bind_listener(&vec![b'x'; unix_path_limit()], true)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::ENAMETOOLONG)
        );
        // SAFETY: restore the confined process's original mask.
        assert_eq!(unsafe { libc::umask(previous) }, 0o027);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn signal_wake_and_nonblocking_stopped_child_drain() {
        if !isolated("signal_wake_and_nonblocking_stopped_child_drain") {
            return;
        }
        let mut wake = SignalWake::new().unwrap();
        // SAFETY: reset only the raised signals in this confined test subprocess.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGUSR1);
            libc::sigaddset(&mut set, libc::SIGUSR2);
            assert_eq!(
                libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()),
                0
            );
        }
        // SAFETY: valid registered signals are raised only in this confined process.
        unsafe {
            libc::raise(libc::SIGUSR1);
            libc::raise(libc::SIGUSR2);
        }
        let signals = wake.drain().unwrap();
        assert!(signals.contains(&libc::SIGUSR1));
        assert!(signals.contains(&libc::SIGUSR2));
        assert!(wake.drain().unwrap().is_empty());
        let process = PreparedJob::new(options(b"kill -STOP $$; exit 7"))
            .unwrap()
            .launch()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stopped = false;
        let mut exited = false;
        while !exited && Instant::now() < deadline {
            if let Some((pid, status)) = wait_any().unwrap() {
                assert_eq!(pid, process.pid);
                if let Some(signal) = stop_signal(status) {
                    assert_eq!(signal, libc::SIGSTOP);
                    stopped = true;
                    if continue_process_group(pid).is_err() {
                        continue_process(pid).unwrap();
                    }
                } else {
                    assert!(status_exited(status));
                    assert_eq!(crate::proc::exit_code(status), 7);
                    exited = true;
                }
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        if !exited {
            let _ = continue_process(process.pid);
            let _ = crate::proc::terminate_process(process.pid);
            let _ = crate::proc::wait_process(process.pid, false);
        }
        assert!(stopped && exited);
        assert!(wait_any().unwrap().is_none());
        assert!(terminal_stop((libc::SIGTTIN << 8) | 0x7f));
        assert!(terminal_stop((libc::SIGTTOU << 8) | 0x7f));
        assert!(!terminal_stop((libc::SIGSTOP << 8) | 0x7f));
    }
    #[test]
    fn vectored_bytes_half_close_and_descriptor_classification() {
        let (left, mut right) = UnixStream::pair().unwrap();
        assert_eq!(
            write_vectored(
                left.as_fd(),
                &[IoSlice::new(b"first"), IoSlice::new(b"second")]
            )
            .unwrap(),
            11
        );
        shutdown_write(left.as_fd()).unwrap();
        let mut output = Vec::new();
        right.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"firstsecond");
        right.write_all(b"reply").unwrap();
        assert_eq!(pending_bytes(left.as_fd()).unwrap(), 5);
        let mut reply = [0u8; 5];
        assert_eq!(crate::fd::read(left.as_fd(), &mut reply).unwrap(), 5);
        assert_eq!(&reply, b"reply");
        assert_eq!(pending_bytes(left.as_fd()).unwrap(), 0);
        assert!(!descriptor_is_regular(left.as_fd()).unwrap());
        assert!(
            descriptor_is_regular(
                std::fs::File::open(std::env::current_exe().unwrap())
                    .unwrap()
                    .as_fd()
            )
            .unwrap()
        );
        assert!(duplicate_standard(3).is_err());
    }

    #[test]
    fn command_glob_order_and_account_lookup_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rmux-glob-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.conf"), b"").unwrap();
        std::fs::write(dir.join("a.conf"), b"").unwrap();
        let pattern = dir.join("*.conf");
        let matches = glob(pattern.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(
            matches,
            vec![
                dir.join("a.conf").as_os_str().as_encoded_bytes().to_vec(),
                dir.join("b.conf").as_os_str().as_encoded_bytes().to_vec()
            ]
        );
        assert_eq!(
            glob(dir.join("*.missing").as_os_str().as_encoded_bytes()),
            Err(GlobError::NoMatch)
        );
        assert_eq!(glob(b"invalid\0pattern"), Err(GlobError::Other));
        std::fs::remove_dir_all(dir).unwrap();
        let uid = crate::proc::getuid();
        if let Some(name) = user_name(uid) {
            assert_eq!(user_by_name(&name), Some(uid));
        }
        // SAFETY: getgid has no arguments and cannot fail.
        let gid = GroupId(unsafe { libc::getgid() });
        if let Some(name) = group_name(gid) {
            assert_eq!(group_by_name(&name), Some(gid));
        }
        assert!(user_by_name(b"rmux-no-such-user-847329").is_none());
        assert!(group_by_name(b"rmux-no-such-group-847329").is_none());
        assert_eq!(uppercase(b'a'), b'A');
    }
    #[test]
    fn pane_pipe_bidirectional_descriptor_orientation_and_group() {
        let (fd, pid) = spawn_pipe_child(b"cat; printf error >&2; exit 7", true, true).unwrap();
        assert_eq!(crate::fd::write(fd.as_fd(), b"duplex\0bytes").unwrap(), 12);
        shutdown_write(fd.as_fd()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        let mut status = None;
        loop {
            let mut buffer = [0u8; 1024];
            let eof = match crate::fd::read(fd.as_fd(), &mut buffer) {
                Ok(0) => true,
                Ok(n) => {
                    output.extend_from_slice(&buffer[..n]);
                    false
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    false
                }
                Err(e) => panic!("pipe child read: {e}"),
            };
            if status.is_none() {
                status = crate::proc::wait_process(pid, true).unwrap();
            }
            if eof && status.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                if status.is_none() {
                    let _ = crate::proc::terminate_process(pid);
                    let _ = crate::proc::wait_process(pid, false);
                }
                panic!("pipe child timed out");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(output, b"duplex\0bytes");
        assert_eq!(crate::proc::exit_code(status.unwrap()), 7);
        assert!(matches!(
            spawn_pipe_child(b"invalid\0command", true, true),
            Err(PipeSpawnError::Fork(_))
        ));
        let (fd, pid) = spawn_pipe_child(b"printf child-output", true, false).unwrap();
        crate::fd::set_blocking(fd.as_fd(), true);
        let mut stream = UnixStream::from(fd);
        let mut output = Vec::new();
        stream.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"child-output");
        assert_eq!(
            crate::proc::exit_code(crate::proc::wait_process(pid, false).unwrap().unwrap()),
            0
        );
    }
}
