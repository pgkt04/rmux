// Ported from tmux spawn.c, input.c @ 8f25579c
use rmux_emu::colour::ColourPalette;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::input::{InputCtx, InputPolicy, NullSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct Oracle {
    binary: PathBuf,
    socket: PathBuf,
}
impl Oracle {
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.binary)
            .args(["-f", "/dev/null", "-S"])
            .arg(&self.socket)
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = Command::new(&self.binary)
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server")
            .output();
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[test]
fn real_pty_launch_parser_capture_matches_pinned_oracle() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let binary = std::env::var_os("RMUX_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("oracle/bin/tmux"));
    if !binary.is_file() {
        eprintln!(
            "skipping spawn oracle: pinned tmux is missing at {}",
            binary.display()
        );
        return;
    }
    let oracle = Oracle {
        binary,
        socket: PathBuf::from(format!("/tmp/rmux-spawn-{}.sock", std::process::id())),
    };
    let script = r"stty -echo -opost; printf '\033[31mred\033[0m\r\nsecond\r\n'; printf '\033[2;3HX\033[?2004h'; printf 'DONE';";
    let oracle_script = format!("{script} exec sleep 30");
    let start = oracle.run(&["new", "-d", "-x", "80", "-y", "24", &oracle_script]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let expected = loop {
        let capture = oracle.run(&["capture-pane", "-p", "-e", "-N", "-T", "-S", "-"]);
        assert!(capture.status.success());
        if capture.stdout.windows(4).any(|s| s == b"DONE") {
            break capture.stdout;
        }
        assert!(Instant::now() < deadline, "oracle output timed out");
        std::thread::sleep(Duration::from_millis(10));
    };
    let process = PreparedLaunch::new(LaunchOptions {
        shell: b"/bin/sh".to_vec(),
        argv: vec![script.as_bytes().to_vec()],
        environment: vec![
            b"PATH=/usr/bin:/bin".to_vec(),
            b"TERM=tmux-256color".to_vec(),
        ],
        cwd: b"/".to_vec(),
        home: None,
        termios: None,
        backspace: 127,
        size: Winsize {
            cols: 80,
            rows: 24,
            xpixel: 640,
            ypixel: 384,
        },
    })
    .unwrap()
    .launch()
    .unwrap();
    let mut bytes = Vec::new();
    let mut status = None;
    loop {
        let mut buffer = [0; 4096];
        let eof = match rmux_sys::fd::read(process.master.as_fd(), &mut buffer) {
            Ok(0) => true,
            Ok(n) => {
                bytes.extend_from_slice(&buffer[..n]);
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(e) if e.raw_os_error() == Some(5) => true,
            Err(e) => panic!("{e}"),
        };
        if status.is_none() {
            status = rmux_sys::proc::wait_process(process.pid, true).unwrap();
        }
        if eof && status.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            if status.is_none() {
                let _ = rmux_sys::proc::terminate_process(process.pid);
                let _ = rmux_sys::proc::wait_process(process.pid, false);
            }
            panic!("pty output timed out");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(rmux_sys::proc::exit_code(status.unwrap()), 0);
    let mut registry = HyperlinkRegistry::new();
    let mut screen =
        Screen::new(80, 24, 2000, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut palette = ColourPalette::new();
    let mut parser = InputCtx::new();
    let mut sink = ScreenOnlySink;
    let mut writer = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy {
            pane_backed: true,
            ..Default::default()
        },
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    parser.parse(
        &mut writer,
        Some(&mut palette),
        &InputPolicy::default(),
        &mut NullSink,
        &bytes,
    );
    writer.finish();
    let actual = rmux_emu::input::dump::capture_pane_used(&screen, &registry);
    assert_eq!(
        actual, expected,
        "real fork/pty/parser capture differs from tmux"
    );
}
