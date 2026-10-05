// Ported from tmux compat.h, tmux.c @ 8f25579c

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use libc::{c_int, iovec, msghdr};

/// `compat.h:84-86`.
pub const ACCESSPERMS: libc::mode_t = 0o777;

pub const LOCK_SH: c_int = libc::LOCK_SH;
pub const LOCK_EX: c_int = libc::LOCK_EX;
pub const LOCK_NB: c_int = libc::LOCK_NB;
pub const LOCK_UN: c_int = libc::LOCK_UN;

/// Linux `SCM_MAX_FD`; the largest descriptor count one message can carry.
const MAX_FDS: usize = 253;

const fn cmsg_space(fds: usize) -> usize {
    // SAFETY: CMSG_SPACE is a pure size computation.
    unsafe { libc::CMSG_SPACE((fds * size_of::<c_int>()) as u32) as usize }
}

/// Control buffer aligned for `cmsghdr` on both targets.
#[repr(C, align(8))]
struct ControlBuffer([u8; cmsg_space(MAX_FDS)]);

impl ControlBuffer {
    const fn new() -> Self {
        Self([0; cmsg_space(MAX_FDS)])
    }
}

/// `sendmsg` with `SCM_RIGHTS` (`compat/imsg-buffer.c` message send path).
pub fn send_fds(sock: BorrowedFd<'_>, data: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
    if fds.len() > MAX_FDS {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    let mut iov = iovec {
        iov_base: data.as_ptr().cast_mut().cast(),
        iov_len: data.len(),
    };
    // SAFETY: msghdr is plain data; zero is a valid state.
    let mut msg: msghdr = unsafe { std::mem::zeroed() };
    let mut control = ControlBuffer::new();
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !fds.is_empty() {
        let space = cmsg_space(fds.len());
        msg.msg_control = control.0.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        // SAFETY: msg_control points into our buffer, so CMSG_FIRSTHDR returns
        // an aligned header inside it with room for fds.len() descriptors.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN((fds.len() * size_of::<c_int>()) as u32) as _;
            let slots = libc::CMSG_DATA(cmsg).cast::<c_int>();
            for (i, fd) in fds.iter().enumerate() {
                slots.add(i).write_unaligned(fd.as_raw_fd());
            }
        }
    }
    // SAFETY: sock is open for the borrow; msg references live local buffers.
    let n = unsafe { libc::sendmsg(sock.as_raw_fd(), &msg, 0) };
    if n == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(n as usize)
}

/// `recvmsg` collecting `SCM_RIGHTS` descriptors into `fds`. Returns the byte count (0 on EOF).
pub fn recv_fds(sock: BorrowedFd<'_>, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> io::Result<usize> {
    let mut iov = iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    // SAFETY: msghdr is plain data; zero is a valid state.
    let mut msg: msghdr = unsafe { std::mem::zeroed() };
    let mut control = ControlBuffer::new();
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.0.as_mut_ptr().cast();
    msg.msg_controllen = cmsg_space(MAX_FDS) as _;
    // SAFETY: sock is open for the borrow; msg references live local buffers.
    let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut msg, 0) };
    if n == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel filled msg_control up to msg_controllen; the CMSG
    // macros walk only that region, and each received descriptor is a fresh
    // one owned by this process exactly once.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let header = libc::CMSG_LEN(0) as usize;
                let count = ((*cmsg).cmsg_len as usize).saturating_sub(header) / size_of::<c_int>();
                let data = libc::CMSG_DATA(cmsg).cast::<c_int>();
                for i in 0..count {
                    fds.push(OwnedFd::from_raw_fd(data.add(i).read_unaligned()));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    Ok(n as usize)
}

/// `setblocking` (`tmux.c:316-328`): toggles `O_NONBLOCK`; fcntl failures are ignored like C.
pub fn set_blocking(fd: BorrowedFd<'_>, blocking: bool) {
    // SAFETY: fd is open for the borrow; F_GETFL/F_SETFL take no pointer arguments.
    unsafe {
        let mode = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        if mode != -1 {
            let mode = if blocking {
                mode & !libc::O_NONBLOCK
            } else {
                mode | libc::O_NONBLOCK
            };
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, mode);
        }
    }
}

/// `flock(fd, operation)` with the `LOCK_*` constants (`compat.h:283-288`).
pub fn flock(fd: BorrowedFd<'_>, operation: c_int) -> io::Result<()> {
    // SAFETY: fd is open for the borrow.
    if unsafe { libc::flock(fd.as_raw_fd(), operation) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
