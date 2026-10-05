// Ported from tmux tty.c, compat.h, compat/fdforkpty.c @ 8f25579c

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
