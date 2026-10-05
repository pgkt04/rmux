use mio::{Events, Interest, Poll, Token, unix::SourceFd};
use std::{
    fs::OpenOptions,
    io::{self, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::net::UnixStream,
    time::Duration,
};

fn pty() -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave) = (-1, -1);
    // openpty initializes two fresh owned descriptors; no borrowed pointers retained.
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    let pair = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let mut term = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(slave, term.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let mut term = unsafe { term.assume_init() };
    unsafe { libc::cfmakeraw(&mut term) };
    if unsafe { libc::tcsetattr(slave, libc::TCSANOW, &term) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(pair)
}
fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    // Successful pipe returns two newly owned descriptors.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}
fn write_byte(fd: i32) -> io::Result<()> {
    // The supplied descriptor stays owned by the probe throughout this write.
    if unsafe { libc::write(fd, b"x".as_ptr().cast(), 1) } != 1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn check(kind: &str, fd: i32, interest: Interest) -> io::Result<bool> {
    let direction = if interest.is_readable() {
        "read"
    } else {
        "write"
    };
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(8);
    let mut source = SourceFd(&fd);
    let mio_result = match poll.registry().register(&mut source, Token(0), interest) {
        Err(error) => format!("registration error: {error}"),
        Ok(()) => {
            let result = match poll.poll(&mut events, Some(Duration::from_millis(250))) {
                Err(error) => format!("wait error: {error}"),
                Ok(())
                    if events.iter().any(|event| {
                        if interest.is_readable() {
                            event.is_readable()
                        } else {
                            event.is_writable()
                        }
                    }) =>
                {
                    "ready".into()
                }
                Ok(()) => "not ready".into(),
            };
            poll.registry().deregister(&mut source)?;
            result
        }
    };
    let mut pfd = libc::pollfd {
        fd,
        events: if interest.is_readable() {
            libc::POLLIN
        } else {
            libc::POLLOUT
        },
        revents: 0,
    };
    // poll borrows one live descriptor and this stack pollfd for the call only.
    let result = unsafe { libc::poll(&mut pfd, 1, 250) };
    let poll_result = if result == -1 {
        format!("error: {}", io::Error::last_os_error())
    } else if result > 0 && pfd.revents & pfd.events != 0 {
        "ready".into()
    } else {
        format!("not ready (revents={})", pfd.revents)
    };
    println!("{kind} {direction}: mio={mio_result}; poll={poll_result}");
    Ok(mio_result == "ready")
}
fn main() -> io::Result<()> {
    let mut supported = true;
    for (kind, slave_side) in [("pty-master", false), ("client-tty", true)] {
        let (master, slave) = pty()?;
        let (fd, peer) = if slave_side {
            (slave.as_raw_fd(), master.as_raw_fd())
        } else {
            (master.as_raw_fd(), slave.as_raw_fd())
        };
        write_byte(peer)?;
        supported &= check(kind, fd, Interest::READABLE)?;
        supported &= check(kind, fd, Interest::WRITABLE)?;
    }
    let null = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    supported &= check("/dev/null", null.as_raw_fd(), Interest::READABLE)?;
    supported &= check("/dev/null", null.as_raw_fd(), Interest::WRITABLE)?;
    let (reader, writer) = pipe()?;
    write_byte(writer.as_raw_fd())?;
    supported &= check("pipe", reader.as_raw_fd(), Interest::READABLE)?;
    supported &= check("pipe", writer.as_raw_fd(), Interest::WRITABLE)?;
    let (socket, mut peer) = UnixStream::pair()?;
    peer.write_all(b"x")?;
    supported &= check("unix-socket", socket.as_raw_fd(), Interest::READABLE)?;
    supported &= check("unix-socket", socket.as_raw_fd(), Interest::WRITABLE)?;
    println!("all fd kinds supported by mio: {supported}");
    #[cfg(target_os = "macos")]
    println!("default backend: mio; null endpoints bypass registration (P0 decision)");
    #[cfg(not(target_os = "macos"))]
    println!(
        "backend candidate: {}; inspect null endpoint results",
        if supported { "mio" } else { "poll(2)" }
    );
    Ok(())
}
