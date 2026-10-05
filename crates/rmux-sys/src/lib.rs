use std::io;

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
