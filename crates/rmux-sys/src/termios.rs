// Ported from tmux tty.c, compat.h, compat/cfmakeraw.c @ 8f25579c

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

/// `compat.h:80-82`.
pub const ECHOPRT: libc::tcflag_t = libc::ECHOPRT;
/// `compat.h:137-139`.
pub const IMAXBEL: libc::tcflag_t = libc::IMAXBEL;

/// Owned copy of a `struct termios`.
#[derive(Clone, Copy)]
pub struct TermiosState(libc::termios);

impl TermiosState {
    /// `tcgetattr(fd)`.
    pub fn get(fd: BorrowedFd<'_>) -> io::Result<Self> {
        // SAFETY: termios is plain data, so the all-zero bit pattern is a valid value.
        let mut tio: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: fd is open for the borrow; tio is a valid out pointer.
        if unsafe { libc::tcgetattr(fd.as_raw_fd(), &mut tio) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(tio))
    }

    /// `tcsetattr(fd, TCSANOW, ...)`.
    pub fn set(&self, fd: BorrowedFd<'_>) -> io::Result<()> {
        // SAFETY: fd is open for the borrow; self.0 is a valid termios.
        if unsafe { libc::tcsetattr(fd.as_raw_fd(), libc::TCSANOW, &self.0) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// `cfmakeraw` (`compat/cfmakeraw.c:25-33`).
    pub fn make_raw(&mut self) {
        use libc::{
            BRKINT, CS8, CSIZE, ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, ISIG,
            ISTRIP, IXON, OPOST, PARENB, PARMRK,
        };
        let tio = &mut self.0;
        tio.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
        tio.c_oflag &= !OPOST;
        tio.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
        tio.c_cflag &= !(CSIZE | PARENB);
        tio.c_cflag |= CS8;
    }

    /// Outer terminal mode (`tty.c:345-354`), not `cfmakeraw`.
    pub fn make_tty_raw(&mut self) {
        let tio = &mut self.0;
        tio.c_iflag &= !(libc::IXON
            | libc::IXOFF
            | libc::ICRNL
            | libc::INLCR
            | libc::IGNCR
            | libc::IMAXBEL
            | libc::ISTRIP);
        tio.c_iflag |= libc::IGNBRK;
        tio.c_oflag &= !(libc::OPOST | libc::ONLCR | libc::OCRNL | libc::ONLRET);
        tio.c_lflag &= !(libc::IEXTEN
            | libc::ICANON
            | libc::ECHO
            | libc::ECHOE
            | libc::ECHONL
            | libc::ECHOCTL
            | libc::ECHOPRT
            | libc::ECHOKE
            | libc::ISIG);
        tio.c_cc[libc::VMIN] = 1;
        tio.c_cc[libc::VTIME] = 0;
    }

    pub fn flush_output(fd: BorrowedFd<'_>) -> io::Result<()> {
        // SAFETY: fd is open for the borrow; TCOFLUSH is a valid selector.
        if unsafe { libc::tcflush(fd.as_raw_fd(), libc::TCOFLUSH) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn iflag(&self) -> libc::tcflag_t {
        self.0.c_iflag
    }

    pub fn oflag(&self) -> libc::tcflag_t {
        self.0.c_oflag
    }

    pub fn cflag(&self) -> libc::tcflag_t {
        self.0.c_cflag
    }

    pub fn lflag(&self) -> libc::tcflag_t {
        self.0.c_lflag
    }

    pub fn iflag_mut(&mut self) -> &mut libc::tcflag_t {
        &mut self.0.c_iflag
    }

    pub fn oflag_mut(&mut self) -> &mut libc::tcflag_t {
        &mut self.0.c_oflag
    }

    pub fn cflag_mut(&mut self) -> &mut libc::tcflag_t {
        &mut self.0.c_cflag
    }

    pub fn lflag_mut(&mut self) -> &mut libc::tcflag_t {
        &mut self.0.c_lflag
    }

    /// The `c_cc` control characters (`VERASE`, `VINTR`, ...).
    pub fn cc(&self) -> &[libc::cc_t] {
        &self.0.c_cc
    }

    pub fn minimum_read(&self) -> u8 {
        self.0.c_cc[libc::VMIN]
    }
    pub fn read_timeout(&self) -> u8 {
        self.0.c_cc[libc::VTIME]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn outer_tty_raw_masks_preserve_control_flags() {
        let (_master, slave, _) = crate::pty::openpty().unwrap();
        let original = TermiosState::get(slave.as_fd()).unwrap();
        let mut raw = original;
        raw.make_tty_raw();
        assert_eq!(
            raw.iflag(),
            (original.iflag()
                & !(libc::IXON
                    | libc::IXOFF
                    | libc::ICRNL
                    | libc::INLCR
                    | libc::IGNCR
                    | libc::IMAXBEL
                    | libc::ISTRIP))
                | libc::IGNBRK
        );
        assert_eq!(
            raw.oflag(),
            original.oflag() & !(libc::OPOST | libc::ONLCR | libc::OCRNL | libc::ONLRET)
        );
        assert_eq!(
            raw.lflag(),
            original.lflag()
                & !(libc::IEXTEN
                    | libc::ICANON
                    | libc::ECHO
                    | libc::ECHOE
                    | libc::ECHONL
                    | libc::ECHOCTL
                    | libc::ECHOPRT
                    | libc::ECHOKE
                    | libc::ISIG)
        );
        assert_eq!(raw.cflag(), original.cflag());
        assert_eq!(raw.cc()[libc::VMIN], 1);
        assert_eq!(raw.cc()[libc::VTIME], 0);
        raw.set(slave.as_fd()).unwrap();
        TermiosState::flush_output(slave.as_fd()).unwrap();
        let actual = TermiosState::get(slave.as_fd()).unwrap();
        assert_eq!(actual.iflag(), raw.iflag());
        assert_eq!(actual.oflag(), raw.oflag());
        assert_eq!(actual.lflag(), raw.lflag());
        original.set(slave.as_fd()).unwrap();
    }
}
