pub mod client;
pub mod errno;
pub mod fd;
pub mod ids;
pub mod locale;
pub mod osdep;
pub mod proc;
pub mod pty;
pub mod server;
#[cfg(all(feature = "systemd", target_os = "linux"))]
pub mod systemd;
pub mod termios;

pub use errno::{access_executable, errno, strerror};
pub use ids::{GroupId, PrincipalId, ProcessId, UserId};
pub use std::os::fd::OwnedFd;
pub use termios::TermiosState;

/// `compat.h:268-270`.
pub const TTY_NAME_MAX: usize = 32;
/// `compat.h:272-274`; macOS lacks the constant and uses 255.
pub const HOST_NAME_MAX: usize = 255;

use std::io;

mod cstring;
pub mod fnmatch;
pub mod number;
pub mod path;
pub mod regex;
pub mod time;

pub use fnmatch::{FnmatchFlags, fnmatch};

pub fn terminate_process_group(child: &mut std::process::Child) -> io::Result<bool> {
    if child.try_wait()?.is_some() {
        return Ok(true);
    }
    let pid = i32::try_from(child.id()).map_err(|_| io::Error::other("process id out of range"))?;
    // The harness creates this child as its own process-group leader.
    if unsafe { libc::kill(-pid, libc::SIGTERM) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EPERM) {
            return Ok(false);
        }
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    // macOS may return EPERM rather than ESRCH for an exited, unreaped group.
    if unsafe { libc::kill(-pid, libc::SIGKILL) } == -1 {
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::ESRCH | libc::EPERM)) {
            return Err(error);
        }
    }
    Ok(true)
}
