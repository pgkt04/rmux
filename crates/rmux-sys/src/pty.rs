// Ported from tmux spawn.c, proc.c, tty.c, compat/fdforkpty.c @ 8f25579c

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};

use crate::ProcessId;

/// `openpty(3)`: master, slave, and the raw tty name. Termios and winsize are
/// applied by the launch code (`spawn.c`), not here.
pub fn openpty() -> io::Result<(OwnedFd, OwnedFd, Vec<u8>)> {
    let (mut master, mut slave) = (-1, -1);
    // openpty(3) takes no name length; PATH_MAX bounds every pty device name.
    let mut name = [0u8; libc::PATH_MAX as usize];
    // SAFETY: the out pointers are valid for the call; name is a writable
    // buffer larger than any tty device path; the NULL termios/winsize are
    // permitted and mean "no change".
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            name.as_mut_ptr().cast(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty succeeded and returned two fresh descriptors that nothing else owns.
    let pair = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    Ok((pair.0, pair.1, name[..end].to_vec()))
}

/// `tcgetpgrp(fd)`; `None` when it returns -1 (`osdep-*.c`).
pub fn tcgetpgrp(fd: BorrowedFd<'_>) -> Option<ProcessId> {
    // SAFETY: fd is a valid open descriptor for the lifetime of the borrow.
    let pgrp = unsafe { libc::tcgetpgrp(fd.as_raw_fd()) };
    (pgrp != -1).then_some(ProcessId(pgrp))
}

/// `login_tty(3)`: new session, controlling tty, fd on 0/1/2. The descriptor
/// is consumed because login_tty closes it.
///
/// # Safety
/// The caller must have no live Rust owners or borrows of descriptors 0, 1,
/// and 2 other than `fd`; this operation replaces those descriptors.
pub unsafe fn login_tty(fd: OwnedFd) -> io::Result<()> {
    let raw = fd.into_raw_fd();
    // SAFETY: raw is an open descriptor we own; login_tty closes it (when > 2) on success.
    if unsafe { libc::login_tty(raw) } == -1 {
        let error = io::Error::last_os_error();
        // SAFETY: on failure login_tty leaves the descriptor open and we still own it.
        drop(unsafe { OwnedFd::from_raw_fd(raw) });
        return Err(error);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Winsize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

pub fn get_winsize(fd: BorrowedFd<'_>) -> io::Result<Winsize> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: fd is open for the borrow; ws is a writable winsize for TIOCGWINSZ.
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(Winsize {
        rows: ws.ws_row,
        cols: ws.ws_col,
        xpixel: ws.ws_xpixel,
        ypixel: ws.ws_ypixel,
    })
}

pub fn set_winsize(fd: BorrowedFd<'_>, size: Winsize) -> io::Result<()> {
    let ws = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: size.xpixel,
        ws_ypixel: size.ypixel,
    };
    // SAFETY: fd is open for the borrow; ws is a valid winsize for TIOCSWINSZ.
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSWINSZ, &ws) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// All byte strings and terminal policy are owned before the fork boundary.
pub struct LaunchOptions {
    pub shell: Vec<u8>,
    pub argv: Vec<Vec<u8>>,
    pub environment: Vec<Vec<u8>>,
    pub cwd: Vec<u8>,
    pub home: Option<Vec<u8>>,
    pub termios: Option<crate::TermiosState>,
    pub backspace: u64,
    pub size: Winsize,
}

pub struct PreparedLaunch {
    arguments: Vec<std::ffi::CString>,
    executables: Vec<std::ffi::CString>,
    fallback_arguments: Vec<std::ffi::CString>,
    directories: Vec<std::ffi::CString>,
    environment: Vec<std::ffi::CString>,
    working_directories: Vec<std::ffi::CString>,
    terminal: Option<crate::TermiosState>,
    erase: libc::cc_t,
    size: Winsize,
    max_fd: i32,
    direct: bool,
}

pub struct LaunchedProcess {
    pub pid: ProcessId,
    pub master: OwnedFd,
    pub tty: Vec<u8>,
}

fn launch_cstring(bytes: Vec<u8>) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "launch string contains NUL"))
}

impl PreparedLaunch {
    pub fn new(options: LaunchOptions) -> io::Result<Self> {
        let LaunchOptions {
            shell,
            argv,
            environment,
            cwd,
            home,
            termios,
            backspace,
            size,
        } = options;
        let direct = argv.len() > 1;
        let program = if direct {
            argv[0].clone()
        } else {
            shell.clone()
        };
        let arguments = if direct {
            argv
        } else {
            let basename = shell
                .rsplit(|&b| b == b'/')
                .next()
                .filter(|b| !b.is_empty())
                .unwrap_or(&shell);
            if let Some(command) = argv.into_iter().next() {
                vec![basename.to_vec(), b"-c".to_vec(), command]
            } else {
                let mut login = Vec::with_capacity(basename.len() + 1);
                login.push(b'-');
                login.extend_from_slice(basename);
                vec![login]
            }
        };
        let executables = if direct && !program.contains(&b'/') {
            let path = environment
                .iter()
                .find_map(|v| v.strip_prefix(b"PATH="))
                .unwrap_or(default_search_path());
            path.split(|&b| b == b':')
                .map(|part| {
                    if part.is_empty() {
                        return program.clone();
                    }
                    let mut candidate = Vec::with_capacity(part.len() + program.len() + 1);
                    candidate.extend_from_slice(part);
                    candidate.push(b'/');
                    candidate.extend_from_slice(&program);
                    candidate
                })
                .collect::<Vec<_>>()
        } else {
            vec![program]
        }
        .into_iter()
        .map(launch_cstring)
        .collect::<io::Result<Vec<_>>>()?;
        let arguments = arguments
            .into_iter()
            .map(launch_cstring)
            .collect::<io::Result<Vec<_>>>()?;
        // execvp's ENOEXEC fallback has a distinct argv0 and skips the original argv0.
        let mut fallback_arguments = vec![launch_cstring(b"sh".to_vec())?];
        if direct {
            fallback_arguments.extend(arguments.iter().skip(1).cloned());
        }
        let mut directories = vec![cwd];
        if let Some(home) = home {
            directories.push(home);
        }
        directories.push(b"/".to_vec());
        let directories = directories
            .into_iter()
            .map(launch_cstring)
            .collect::<io::Result<Vec<_>>>()?;
        let environment = environment
            .into_iter()
            .filter(|v| !v.starts_with(b"PWD="))
            .map(launch_cstring)
            .collect::<io::Result<Vec<_>>>()?;
        let working_directories = directories
            .iter()
            .map(|directory| {
                let mut pwd = b"PWD=".to_vec();
                pwd.extend_from_slice(directory.as_bytes());
                launch_cstring(pwd)
            })
            .collect::<io::Result<Vec<_>>>()?;
        // SAFETY: sysconf takes no pointers and runs before fork.
        let max_fd = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
        Ok(Self {
            arguments,
            executables,
            fallback_arguments,
            directories,
            environment,
            working_directories,
            direct,
            terminal: termios,
            erase: if backspace >= 0x7f {
                0x7f
            } else {
                backspace as libc::cc_t
            },
            size,
            max_fd: if max_fd < 0 {
                256
            } else {
                max_fd.min(i64::from(i32::MAX) as _) as i32
            },
        })
    }

    /// The server calls this on its event-loop thread. The child executes only
    /// async-signal-safe libc operations: no allocation, callbacks or unwinding.
    /// Unlike the C parent-chdir sequence, the parent's cwd is never changed.
    pub fn launch(self) -> io::Result<LaunchedProcess> {
        use std::os::fd::AsFd;
        let mut arguments = self
            .arguments
            .iter()
            .map(|s| s.as_ptr())
            .collect::<Vec<_>>();
        arguments.push(std::ptr::null());
        let environments = self
            .working_directories
            .iter()
            .map(|pwd| {
                let mut pointers = Vec::with_capacity(self.environment.len() + 2);
                pointers.extend(self.environment.iter().map(|s| s.as_ptr()));
                pointers.push(pwd.as_ptr());
                pointers.push(std::ptr::null());
                pointers
            })
            .collect::<Vec<_>>();
        let mut fallback = Vec::with_capacity(self.fallback_arguments.len() + 2);
        fallback.push(self.fallback_arguments[0].as_ptr());
        fallback.push(std::ptr::null());
        fallback.extend(self.fallback_arguments.iter().skip(1).map(|s| s.as_ptr()));
        fallback.push(std::ptr::null());
        let mask = LaunchSignalMask::block()?;
        let (master, slave, tty) = openpty()?;
        let master = above_standard_fds(master)?;
        let slave = above_standard_fds(slave)?;
        set_winsize(slave.as_fd(), self.size)?;
        crate::fd::set_blocking(master.as_fd(), false);
        let master_fd = master.as_raw_fd();
        let slave_fd = slave.as_raw_fd();
        // SAFETY: these are plain C structures initialized entirely before fork.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: all pointers are valid local signal structures.
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
            action.sa_sigaction = libc::SIG_DFL;
            action.sa_flags = libc::SA_RESTART;
        }
        // SAFETY: the child branch uses only prepared storage and libc calls,
        // and always execs or _exits without running any Rust destructors.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // SAFETY: this branch has private descriptor/process state; every
            // buffer is prepared above and all failures terminate via _exit.
            unsafe {
                libc::close(master_fd);
                if libc::setsid() == -1 || libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) == -1 {
                    libc::_exit(1);
                }
                for fd in 0..=2 {
                    if libc::dup2(slave_fd, fd) == -1 {
                        libc::_exit(1);
                    }
                }
                let mut actual = self.directories.len() - 1;
                for (index, directory) in self.directories.iter().enumerate() {
                    if libc::chdir(directory.as_ptr()) == 0 {
                        actual = index;
                        break;
                    }
                }
                let mut terminal: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut terminal) == -1 {
                    libc::_exit(1);
                }
                if let Some(settings) = &self.terminal {
                    let cc = settings.cc();
                    std::ptr::copy_nonoverlapping(
                        cc.as_ptr(),
                        terminal.c_cc.as_mut_ptr(),
                        cc.len(),
                    );
                }
                terminal.c_cc[libc::VERASE] = self.erase;
                terminal.c_iflag |= libc::IUTF8;
                if libc::tcsetattr(0, libc::TCSANOW, &terminal) == -1 {
                    libc::_exit(1);
                }
                for signal in [
                    libc::SIGPIPE,
                    libc::SIGTSTP,
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGHUP,
                    libc::SIGCHLD,
                    libc::SIGCONT,
                    libc::SIGTERM,
                    libc::SIGUSR1,
                    libc::SIGUSR2,
                    libc::SIGWINCH,
                ] {
                    libc::sigaction(signal, &action, std::ptr::null_mut());
                }
                crate::proc::close_child_fds(self.max_fd);
                libc::sigprocmask(libc::SIG_SETMASK, &mask.old, std::ptr::null_mut());
                let env = environments[actual].as_ptr();
                for executable in &self.executables {
                    libc::execve(executable.as_ptr(), arguments.as_ptr(), env);
                    #[cfg(target_os = "macos")]
                    let error = *libc::__error();
                    #[cfg(target_os = "linux")]
                    let error = *libc::__errno_location();
                    if self.direct && error == libc::ENOEXEC {
                        fallback[1] = executable.as_ptr();
                        libc::execve(c"/bin/sh".as_ptr(), fallback.as_ptr(), env);
                        libc::_exit(1);
                    }
                    if !matches!(
                        error,
                        libc::EACCES
                            | libc::ENOENT
                            | libc::ENOTDIR
                            | libc::ESTALE
                            | libc::ENODEV
                            | libc::ETIMEDOUT
                    ) {
                        libc::_exit(1);
                    }
                }
                libc::_exit(1);
            }
        }
        let error = if pid == -1 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        drop(mask);
        if let Some(error) = error {
            return Err(error);
        }
        drop(slave);
        Ok(LaunchedProcess {
            pid: ProcessId(pid),
            master,
            tty,
        })
    }
}

struct LaunchSignalMask {
    old: libc::sigset_t,
}
impl LaunchSignalMask {
    fn block() -> io::Result<Self> {
        // SAFETY: signal sets are plain C storage; sigprocmask initializes old.
        let (mut set, mut old): (libc::sigset_t, libc::sigset_t) = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers reference initialized, live local signal sets.
        unsafe {
            libc::sigfillset(&mut set);
            if libc::sigprocmask(libc::SIG_BLOCK, &set, &mut old) == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self { old })
    }
}
impl Drop for LaunchSignalMask {
    fn drop(&mut self) {
        // SAFETY: the stored mask belongs to this launching thread; restore it.
        unsafe {
            libc::sigprocmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut());
        }
    }
}

fn above_standard_fds(fd: OwnedFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() > 2 {
        // SAFETY: F_SETFD updates the live descriptor without changing ownership.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
        return Ok(fd);
    }
    // SAFETY: fcntl duplicates this live fd with close-on-exec; we own the result.
    let raw = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if raw == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the duplicate is a fresh descriptor with no other owner.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

pub const fn default_search_path() -> &'static [u8] {
    #[cfg(target_os = "macos")]
    {
        b"/usr/bin:/bin:/usr/sbin:/sbin"
    }
    #[cfg(target_os = "linux")]
    {
        b"/usr/bin:/bin"
    }
}

#[cfg(test)]
mod launch_signal_tests {
    use super::*;

    fn current_mask() -> libc::sigset_t {
        // SAFETY: zeroed signal storage is valid; sigprocmask initializes it.
        let mut mask = unsafe { std::mem::zeroed() };
        // SAFETY: a null new mask requests the current thread's mask only.
        assert_eq!(
            unsafe { libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut mask) },
            0
        );
        mask
    }

    #[test]
    fn prepared_launch_restores_calling_thread_signal_mask() {
        let before = current_mask();
        let child = PreparedLaunch::new(LaunchOptions {
            shell: b"/bin/sh".to_vec(),
            argv: vec![b"exit 0".to_vec()],
            environment: Vec::new(),
            cwd: b"/".to_vec(),
            home: None,
            termios: None,
            backspace: 127,
            size: Winsize {
                cols: 80,
                rows: 24,
                ..Default::default()
            },
        })
        .unwrap()
        .launch()
        .unwrap();
        let after = current_mask();
        for signal in 1..=31 {
            // SAFETY: both sets are initialized and the signal number is in range.
            assert_eq!(unsafe { libc::sigismember(&before, signal) }, unsafe {
                libc::sigismember(&after, signal)
            });
        }
        assert_eq!(
            crate::proc::exit_code(
                crate::proc::wait_process(child.pid, false)
                    .unwrap()
                    .unwrap()
            ),
            0
        );
    }

    #[test]
    fn prepared_launch_first_output_does_not_scan_open_max() {
        use std::os::fd::{AsFd, AsRawFd};
        use std::time::{Duration, Instant};
        let launch = PreparedLaunch::new(LaunchOptions {
            shell: b"/bin/sh".to_vec(),
            argv: vec![b"printf READY".to_vec()],
            environment: Vec::new(),
            cwd: b"/".to_vec(),
            home: None,
            termios: None,
            backspace: 127,
            size: Winsize {
                cols: 80,
                rows: 24,
                ..Default::default()
            },
        })
        .unwrap();
        let started = Instant::now();
        let child = launch.launch().unwrap();
        let deadline = Duration::from_millis(100);
        let mut output = Vec::new();
        let mut failure = None;
        while !output.windows(5).any(|bytes| bytes == b"READY") {
            let Some(remaining) = deadline.checked_sub(started.elapsed()) else {
                break;
            };
            let mut descriptor = libc::pollfd {
                fd: child.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: descriptor is one initialized poll record for a live fd.
            let result = unsafe {
                libc::poll(
                    &mut descriptor,
                    1,
                    remaining.as_millis().max(1) as libc::c_int,
                )
            };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                failure = Some(error.to_string());
                break;
            }
            if result == 0 {
                break;
            }
            let mut bytes = [0u8; 32];
            match crate::fd::read(child.master.as_fd(), &mut bytes) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&bytes[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }
        let elapsed = started.elapsed();
        let status = crate::proc::wait_process(child.pid, false)
            .unwrap()
            .unwrap();
        assert!(failure.is_none(), "first child output: {failure:?}");
        assert_eq!(crate::proc::exit_code(status), 0);
        assert!(
            output.windows(5).any(|bytes| bytes == b"READY"),
            "child produced no marker within {deadline:?}; elapsed={elapsed:?}, bytes={output:?}"
        );
        assert!(
            elapsed < deadline,
            "child first-output latency {elapsed:?} exceeded {deadline:?}"
        );
    }
}
