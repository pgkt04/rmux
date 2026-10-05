// Ported from tmux osdep-darwin.c, osdep-linux.c @ 8f25579c

use std::os::fd::BorrowedFd;

use crate::pty::tcgetpgrp;

fn until_nul(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// `osdep_get_name`: the raw kernel process name of the pty's foreground group.
#[cfg(target_os = "macos")]
pub fn get_name(fd: BorrowedFd<'_>) -> Option<Vec<u8>> {
    let pgrp = tcgetpgrp(fd)?;
    // SAFETY: proc_bsdshortinfo is plain data; zero is a valid value.
    let mut bsdinfo: libc::proc_bsdshortinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_bsdshortinfo>() as libc::c_int;
    // SAFETY: the buffer pointer and size describe bsdinfo exactly.
    let ret = unsafe {
        libc::proc_pidinfo(
            pgrp.0,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            (&raw mut bsdinfo).cast(),
            size,
        )
    };
    if ret != size {
        return None;
    }
    // SAFETY: c_char and u8 have the same size and alignment.
    let comm: &[u8] = unsafe {
        std::slice::from_raw_parts(bsdinfo.pbsi_comm.as_ptr().cast(), bsdinfo.pbsi_comm.len())
    };
    let comm = until_nul(comm);
    (!comm.is_empty()).then(|| comm.to_vec())
}

/// `osdep_get_cwd`: the current directory of the pty's foreground group.
#[cfg(target_os = "macos")]
pub fn get_cwd(fd: BorrowedFd<'_>) -> Option<Vec<u8>> {
    let pgrp = tcgetpgrp(fd)?;
    // SAFETY: proc_vnodepathinfo is plain data; zero is a valid value.
    let mut pathinfo: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    // SAFETY: the buffer pointer and size describe pathinfo exactly.
    let ret = unsafe {
        libc::proc_pidinfo(
            pgrp.0,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut pathinfo).cast(),
            size,
        )
    };
    if ret != size {
        return None;
    }
    let path = pathinfo.pvi_cdir.vip_path.as_flattened();
    // SAFETY: c_char and u8 have the same size and alignment.
    let path: &[u8] = unsafe { std::slice::from_raw_parts(path.as_ptr().cast(), path.len()) };
    // strlcpy into wd[PATH_MAX] keeps at most PATH_MAX - 1 bytes.
    let path = until_nul(path);
    let keep = path.len().min(libc::PATH_MAX as usize - 1);
    Some(path[..keep].to_vec())
}

/// `osdep_get_name`: `/proc/<pgrp>/cmdline` up to the first NUL; unreadable or empty gives `None`.
#[cfg(target_os = "linux")]
pub fn get_name(fd: BorrowedFd<'_>) -> Option<Vec<u8>> {
    let pgrp = tcgetpgrp(fd)?;
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", pgrp.0)).ok()?;
    let name = until_nul(&cmdline);
    (!name.is_empty()).then(|| name.to_vec())
}

#[cfg(target_os = "linux")]
fn readlink_cwd(pid: libc::pid_t, target: &mut [u8]) -> libc::ssize_t {
    let path = std::ffi::CString::new(format!("/proc/{pid}/cwd")).expect("no NUL in path");
    // SAFETY: path is NUL-terminated; target is writable for target.len() bytes.
    unsafe { libc::readlink(path.as_ptr(), target.as_mut_ptr().cast(), target.len()) }
}

/// `osdep_get_cwd`: `readlink("/proc/<pgrp>/cwd")`, retried with the
/// `TIOCGSID` session id only when readlink fails; bounded to `MAXPATHLEN`.
#[cfg(target_os = "linux")]
pub fn get_cwd(fd: BorrowedFd<'_>) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;

    const MAXPATHLEN: usize = libc::PATH_MAX as usize;
    let pgrp = tcgetpgrp(fd)?;
    let mut target = [0u8; MAXPATHLEN + 1];
    let mut n = readlink_cwd(pgrp.0, &mut target[..MAXPATHLEN]);
    if n == -1 {
        let mut sid: libc::pid_t = 0;
        // SAFETY: fd is open for the borrow; TIOCGSID writes one pid_t into sid.
        if unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCGSID, &mut sid) } != -1 {
            n = readlink_cwd(sid, &mut target[..MAXPATHLEN]);
        }
    }
    (n > 0).then(|| target[..n as usize].to_vec())
}
