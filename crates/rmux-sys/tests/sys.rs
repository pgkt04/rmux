use std::fs;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rmux_sys::{fd, locale, osdep, proc, pty, termios::TermiosState};

/// setlocale is not thread-safe; the test threads share one call.
fn utf8_locale() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| locale::setup_ctype().expect("UTF-8 locale"));
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rmux-sys-tests-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Spawn `sh -c <script>` as the session leader on the slave of a fresh pty.
fn spawn_on_pty(script: &str) -> (OwnedFd, std::process::Child) {
    let (master, slave, name) = pty::openpty().expect("openpty");
    assert!(
        name.starts_with(b"/dev/"),
        "tty name {:?}",
        String::from_utf8_lossy(&name)
    );
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut slave = Some(slave);
    // SAFETY: the hook only calls login_tty (setsid, TIOCSCTTY, dup2, close),
    // which is async-signal-safe and allocates nothing.
    unsafe {
        command.pre_exec(move || match slave.take() {
            Some(fd) => pty::login_tty(fd),
            None => Ok(()),
        });
    }
    let child = command.spawn().expect("spawn sh on pty");
    drop(command);
    (master, child)
}

#[test]
fn osdep_name_and_cwd_follow_the_foreground_process() {
    let (master, mut child) = spawn_on_pty("cd /tmp && exec sleep 30");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut name = None;
    while Instant::now() < deadline {
        name = osdep::get_name(master.as_fd());
        match name.as_deref() {
            Some(b"sleep") => break,
            Some(b"sh") | None => std::thread::sleep(Duration::from_millis(20)),
            Some(other) => panic!(
                "unexpected process name {:?}",
                String::from_utf8_lossy(other)
            ),
        }
    }
    let cwd = osdep::get_cwd(master.as_fd());
    child.kill().expect("kill sleep");
    child.wait().expect("reap sleep");
    assert_eq!(name.as_deref(), Some(&b"sleep"[..]));
    let cwd = cwd.expect("cwd of the foreground process");
    assert!(
        cwd == b"/tmp" || cwd == b"/private/tmp",
        "unexpected cwd {:?}",
        String::from_utf8_lossy(&cwd)
    );
}

#[test]
fn tcgetpgrp_is_none_without_a_controlling_process() {
    let (_master, slave, _) = pty::openpty().expect("openpty");
    let (a, _b) = UnixStream::pair().expect("socketpair");
    assert!(pty::tcgetpgrp(a.as_fd()).is_none());
    assert!(osdep::get_name(a.as_fd()).is_none());
    assert!(osdep::get_cwd(a.as_fd()).is_none());
    // A pty with no session attached has no foreground process group.
    assert!(pty::tcgetpgrp(slave.as_fd()).is_none());
}

/// Build a C program that prints the oracle's width for the hex code point argument.
fn c_wcwidth_program(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let source = dir.join("wcwidth.c");
    let binary = dir.join("wcwidth");
    let mut cc = Command::new("cc");
    if cfg!(target_os = "macos") {
        let flags = Command::new("pkg-config")
            .args(["--cflags", "--libs", "libutf8proc"])
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        fs::write(
            &source,
            "#include <stdio.h>\n#include <stdlib.h>\n#include <utf8proc.h>\n\
             int main(int argc, char **argv) {\n\
             \tint wc = (int)strtol(argv[1], NULL, 16);\n\
             \tif (utf8proc_category(wc) == UTF8PROC_CATEGORY_CO) printf(\"1\\n\");\n\
             \telse printf(\"%d\\n\", utf8proc_charwidth(wc));\n\
             \treturn 0;\n}\n",
        )
        .ok()?;
        cc.arg("-o").arg(&binary).arg(&source);
        cc.args(String::from_utf8_lossy(&flags.stdout).split_whitespace());
    } else {
        fs::write(
            &source,
            "#include <locale.h>\n#include <stdio.h>\n#include <stdlib.h>\n#include <wchar.h>\n\
             int main(int argc, char **argv) {\n\
             \tif (setlocale(LC_CTYPE, \"en_US.UTF-8\") == NULL) setlocale(LC_CTYPE, \"C.UTF-8\");\n\
             \tprintf(\"%d\\n\", wcwidth((wchar_t)strtol(argv[1], NULL, 16)));\n\
             \treturn 0;\n}\n",
        )
        .ok()?;
        cc.arg("-o").arg(&binary).arg(&source);
    }
    let status = cc.status().ok()?;
    status.success().then_some(binary)
}

#[test]
fn wcwidth_fixtures() {
    utf8_locale();
    assert_eq!(locale::wcwidth(0x00E9), 1);
    assert_eq!(locale::wcwidth(0x4E00), 2);
    assert_eq!(locale::wcwidth(0x200B), 0);
    let dir = scratch_dir("wcwidth");
    let Some(program) = c_wcwidth_program(&dir) else {
        eprintln!("skipping C wcwidth comparison: cc or pkg-config libutf8proc unavailable");
        return;
    };
    for wc in [0x00E9u32, 0x4E00, 0x200B, 0x0085, 0x0041, 0xE0B0, 0x1F600] {
        let output = Command::new(&program)
            .arg(format!("{wc:X}"))
            .output()
            .expect("run C wcwidth");
        let expected: i32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("C width");
        assert_eq!(locale::wcwidth(wc), expected, "U+{wc:04X}");
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn mbtowc_and_wctomb_round_trip() {
    utf8_locale();
    let mut buf = [0u8; 32];
    assert_eq!(locale::wctomb(0x00E9, &mut buf), Some(2));
    assert_eq!(&buf[..2], b"\xC3\xA9");
    assert_eq!(locale::wctomb(0x1F600, &mut buf), Some(4));
    assert_eq!(&buf[..4], b"\xF0\x9F\x98\x80");
    assert_eq!(locale::wctomb(b'a'.into(), &mut buf), Some(1));
    assert_eq!(locale::mbtowc(b"\xC3\xA9"), Some(0x00E9));
    assert_eq!(locale::mbtowc(b"\xF0\x9F\x98\x80"), Some(0x1F600));
    assert_eq!(
        locale::mbtowc(b"\xC3\xA9\xCC\x81"),
        Some(0x00E9),
        "first code point only"
    );
    assert_eq!(locale::mbtowc(b"a"), Some(0x61));
    // utf8proc_mbtowc returns length 1 for NUL (utf8_towc gives DONE, wc 0);
    // libc mbtowc returns 0 (utf8_towc gives ERROR).
    if cfg!(target_os = "macos") {
        assert_eq!(locale::mbtowc(b"\0"), Some(0));
    } else {
        assert_eq!(locale::mbtowc(b"\0"), None);
    }
    assert_eq!(locale::mbtowc(b""), None);
    assert_eq!(locale::mbtowc(b"\xFF"), None);
    assert_eq!(locale::mbtowc(b"\xC3"), None, "truncated sequence");
    assert_eq!(locale::mbtowc(b"\x80"), None, "lone continuation");
    assert_eq!(
        locale::mbtowc(b"\xC3\xA9"),
        Some(0x00E9),
        "state reset after failure"
    );
}

#[test]
fn send_and_recv_fds_round_trip_over_a_socketpair() {
    let (sender, receiver) = UnixStream::pair().expect("socketpair");
    let (mut payload_a, payload_b) = UnixStream::pair().expect("payload pair");
    let (payload_c, mut payload_d) = UnixStream::pair().expect("payload pair");
    let sent = fd::send_fds(
        sender.as_fd(),
        b"hello",
        &[payload_b.as_fd(), payload_c.as_fd()],
    )
    .expect("send_fds");
    assert_eq!(sent, 5);
    let mut buf = [0u8; 16];
    let mut received = Vec::new();
    let n = fd::recv_fds(receiver.as_fd(), &mut buf, &mut received).expect("recv_fds");
    assert_eq!(&buf[..n], b"hello");
    assert_eq!(received.len(), 2);
    let mut got_b = UnixStream::from(received.remove(0));
    let mut got_c = UnixStream::from(received.remove(0));
    got_b.write_all(b"via b").expect("write");
    let mut back = [0u8; 5];
    payload_a.read_exact(&mut back).expect("read");
    assert_eq!(&back, b"via b");
    got_c.write_all(b"via c").expect("write");
    payload_d.read_exact(&mut back).expect("read");
    assert_eq!(&back, b"via c");

    let sent = fd::send_fds(sender.as_fd(), b"plain", &[]).expect("send without fds");
    assert_eq!(sent, 5);
    received.clear();
    let n = fd::recv_fds(receiver.as_fd(), &mut buf, &mut received).expect("recv_fds");
    assert_eq!(&buf[..n], b"plain");
    assert!(received.is_empty());

    drop(sender);
    let n = fd::recv_fds(receiver.as_fd(), &mut buf, &mut received).expect("recv_fds at EOF");
    assert_eq!(n, 0);
}

fn flags_of(fd: &impl AsRawFd) -> i32 {
    // SAFETY: fd is open; F_GETFL takes no pointer argument.
    unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) }
}

#[test]
fn set_blocking_toggles_o_nonblock() {
    let (a, _b) = UnixStream::pair().expect("socketpair");
    assert_eq!(flags_of(&a) & libc::O_NONBLOCK, 0);
    fd::set_blocking(a.as_fd(), false);
    assert_ne!(flags_of(&a) & libc::O_NONBLOCK, 0);
    fd::set_blocking(a.as_fd(), true);
    assert_eq!(flags_of(&a) & libc::O_NONBLOCK, 0);
}

#[test]
fn termios_make_raw_applies_to_a_pty() {
    let (_master, slave, _) = pty::openpty().expect("openpty");
    let mut state = TermiosState::get(slave.as_fd()).expect("tcgetattr");
    assert_ne!(state.lflag() & libc::ECHO, 0, "a fresh pty echoes");
    state.make_raw();
    state.set(slave.as_fd()).expect("tcsetattr");
    let applied = TermiosState::get(slave.as_fd()).expect("tcgetattr");
    assert_eq!(
        applied.lflag() & (libc::ECHO | libc::ICANON | libc::ISIG | libc::IEXTEN),
        0
    );
    assert_eq!(
        applied.iflag() & (libc::ICRNL | libc::IXON | libc::BRKINT),
        0
    );
    assert_eq!(applied.oflag() & libc::OPOST, 0);
    assert_eq!(applied.cflag() & libc::CSIZE, libc::CS8);
    assert_eq!(applied.cflag() & libc::PARENB, 0);
    assert_eq!(applied.cc().len(), libc::NCCS);
    let _ = (rmux_sys::termios::ECHOPRT, rmux_sys::termios::IMAXBEL);
}

#[test]
fn closefrom_closes_descriptors_at_and_above_lowfd() {
    if std::env::var_os("RMUX_SYS_CLOSEFROM_CHILD").is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "closefrom_closes_descriptors_at_and_above_lowfd",
                "--test-threads=1",
            ])
            .env("RMUX_SYS_CLOSEFROM_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let (keep, _b) = UnixStream::pair().expect("socketpair");
    // SAFETY: F_DUPFD returns a fresh descriptor numbered at least 200.
    let high = unsafe { libc::fcntl(keep.as_raw_fd(), libc::F_DUPFD, 200) };
    assert_eq!(high, 200, "descriptor 200 is free in the test process");
    let higher = unsafe { libc::fcntl(keep.as_raw_fd(), libc::F_DUPFD, 230) };
    assert_eq!(higher, 230);
    // SAFETY: the high descriptors are raw, uniquely owned test duplicates.
    unsafe { proc::closefrom(199) };
    // SAFETY: F_GETFD takes no pointer argument; EBADF is the expected outcome.
    assert_eq!(unsafe { libc::fcntl(200, libc::F_GETFD) }, -1);
    assert_eq!(unsafe { libc::fcntl(230, libc::F_GETFD) }, -1);
    assert_ne!(flags_of(&keep), -1, "descriptors below lowfd stay open");
}

#[test]
fn strerror_has_no_rust_suffix() {
    assert_eq!(
        rmux_sys::strerror(libc::ENOENT),
        b"No such file or directory"
    );
    assert_eq!(rmux_sys::strerror(libc::EACCES), b"Permission denied");
    assert!(!rmux_sys::strerror(999_999).is_empty());
}

#[test]
fn errno_reflects_the_last_failure() {
    // SAFETY: closing an invalid descriptor only sets errno.
    unsafe {
        libc::close(-1);
    }
    assert_eq!(rmux_sys::errno(), libc::EBADF);
}

#[test]
fn access_executable_checks_x_ok() {
    assert!(rmux_sys::access_executable(b"/bin/sh"));
    assert!(rmux_sys::access_executable(b"/bin/sh\0ignored"));
    assert!(!rmux_sys::access_executable(b"/nonexistent/rmux-shell"));
    assert!(!rmux_sys::access_executable(b"/etc/hosts"));
}

#[test]
fn flock_takes_and_detects_locks() {
    let dir = scratch_dir("flock");
    let path = dir.join("lock");
    let first = fs::File::create(&path).expect("lock file");
    let second = fs::File::open(&path).expect("lock file");
    fd::flock(first.as_fd(), fd::LOCK_EX | fd::LOCK_NB).expect("first lock");
    let error =
        fd::flock(second.as_fd(), fd::LOCK_EX | fd::LOCK_NB).expect_err("second lock fails");
    assert_eq!(error.raw_os_error(), Some(libc::EWOULDBLOCK));
    fd::flock(first.as_fd(), fd::LOCK_UN).expect("unlock");
    fd::flock(second.as_fd(), fd::LOCK_EX | fd::LOCK_NB).expect("lock after unlock");
    assert_eq!(fd::ACCESSPERMS, 0o777);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ids_match_libc() {
    assert_eq!(proc::getpid().0 as u32, std::process::id());
    // SAFETY: getuid takes no arguments.
    assert_eq!(proc::getuid().0, unsafe { libc::getuid() });
}

#[test]
fn getpeereid_reports_the_peer_credentials() {
    let (a, _b) = UnixStream::pair().expect("socketpair");
    let (uid, gid) = proc::getpeereid(a.as_fd()).expect("peer credentials");
    // SAFETY: getuid and getgid take no arguments.
    assert_eq!(uid.0, unsafe { libc::getuid() });
    assert_eq!(gid.0, unsafe { libc::getgid() });
    let file = fs::File::open("/dev/null").expect("/dev/null");
    assert!(proc::getpeereid(file.as_fd()).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn setproctitle_truncates_at_the_last_space() {
    proc::setproctitle(b"rmux", b"server (foo)");
    let comm = fs::read_to_string("/proc/thread-self/comm").expect("comm");
    assert_eq!(comm.trim_end(), "rmux: server");
    proc::setproctitle(b"rmux", b"client");
    let comm = fs::read_to_string("/proc/thread-self/comm").expect("comm");
    assert_eq!(comm.trim_end(), "rmux: client");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn setproctitle_is_a_no_op() {
    proc::setproctitle(b"rmux", b"server (foo)");
}

const DAEMON_VAR: &str = "RMUX_SYS_DAEMON_CHILD";

/// Subprocess entry for the daemon test: the surviving grandchild records its state.
#[test]
fn daemon_child() {
    let Ok(report) = std::env::var(DAEMON_VAR) else {
        return;
    };
    // SAFETY: this isolated subprocess holds no fd owners for 0/1/2 or locks.
    unsafe { proc::daemon(false, false) }.expect("daemon");
    // SAFETY: getsid and getpid take plain integer arguments.
    let (sid, pid) = unsafe { (libc::getsid(0), libc::getpid()) };
    let cwd = std::env::current_dir().expect("cwd");
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: stat is a valid out pointer for fstat on descriptor 0.
    let devnull =
        unsafe { libc::fstat(0, &mut stat) } == 0 && (stat.st_mode & libc::S_IFMT) == libc::S_IFCHR;
    fs::write(report, format!("{sid} {pid} {} {devnull}", cwd.display())).expect("report");
    std::process::exit(0);
}

#[test]
fn daemon_detaches_the_grandchild() {
    let dir = scratch_dir("daemon");
    let report = dir.join("daemon-report");
    let status = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "daemon_child", "--nocapture"])
        .env(DAEMON_VAR, &report)
        .status()
        .expect("spawn test binary");
    assert!(status.success(), "forking parent exits 0");
    let deadline = Instant::now() + Duration::from_secs(10);
    let contents = loop {
        if let Ok(contents) = fs::read_to_string(&report) {
            if !contents.is_empty() {
                break contents;
            }
        }
        assert!(
            Instant::now() < deadline,
            "daemon grandchild did not report"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let fields: Vec<&str> = contents.split(' ').collect();
    assert_eq!(fields.len(), 4, "report {contents:?}");
    assert_eq!(fields[0], fields[1], "the daemon leads its own session");
    assert_eq!(fields[2], "/");
    assert_eq!(fields[3], "true", "stdin is /dev/null");
    let _ = fs::remove_dir_all(dir);
}
