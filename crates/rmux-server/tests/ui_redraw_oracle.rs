//! Differential test for G17 redraw: drive the same session in the oracle
//! tmux and in rmux, attach a client in a pty of fixed size, capture the
//! client terminal output, parse both captures with rmux-emu and compare the
//! resulting screens line by line.
//!
//! Skips (with a message) when the oracle (`oracle/bin/tmux`) or the rmux
//! binary (`$RMUX_BIN`, `$CARGO_TARGET_DIR/debug/rmux`, or
//! `target/debug/rmux`) is missing. `RMUX_UI_ORACLE_MUTATE=1` corrupts the
//! rmux rendering so the comparison can be seen to fail.

use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::input::InputCtx;
use rmux_emu::input::effect::{InputPolicy, NullSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const SX: u16 = 60;
const SY: u16 = 16;

fn oracle_path() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
    path.is_file().then_some(path)
}

fn rmux_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RMUX_BIN") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let mut candidates = Vec::new();
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(dir).join("debug/rmux"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/rmux"));
    candidates.into_iter().find(|p| p.is_file())
}

struct Mux {
    bin: PathBuf,
    socket: PathBuf,
}

impl Mux {
    fn new(bin: PathBuf, tag: &str) -> Mux {
        let dir =
            std::env::temp_dir().join(format!("rmux-ui-oracle-{}-{}", std::process::id(), tag));
        let _ = std::fs::create_dir_all(&dir);
        Mux {
            bin,
            socket: dir.join("sock"),
        }
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let out = Command::new(&self.bin)
            .arg("-S")
            .arg(&self.socket)
            .arg("-f")
            .arg("/dev/null")
            .args(args)
            .env("TERM", "screen")
            .env_remove("TMUX")
            .env_remove("RMUX")
            .output()
            .map_err(|e| format!("{}: {e}", self.bin.display()))?;
        if !out.status.success() {
            return Err(format!(
                "{} {:?} failed: {}",
                self.bin.display(),
                args,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(out.stdout)
    }
}

impl Drop for Mux {
    fn drop(&mut self) {
        let _ = self.run(&["kill-server"]);
        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

struct AttachedClient(rmux_sys::pty::LaunchedProcess);

impl Drop for AttachedClient {
    fn drop(&mut self) {
        if matches!(rmux_sys::proc::wait_process(self.0.pid, true), Ok(None)) {
            let _ = rmux_sys::proc::terminate_process(self.0.pid);
            let _ = rmux_sys::proc::wait_process(self.0.pid, false);
        }
    }
}

/// Read from the pty master until no bytes arrive for `quiet`.
fn drain(master: std::os::fd::BorrowedFd<'_>, out: &mut Vec<u8>, quiet: Duration, max: Duration) {
    let start = Instant::now();
    let mut last = Instant::now();
    let mut buf = [0u8; 8192];
    loop {
        match rmux_sys::fd::read(master, &mut buf) {
            Ok(0) => return,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                last = Instant::now();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if last.elapsed() > quiet || start.elapsed() > max {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return,
        }
    }
}

/// Attach a client in a pty, run every step through the control socket and
/// return the terminal bytes the client wrote.
fn attach_and_capture(mux: &Mux, steps: &[&[&str]]) -> Result<Vec<u8>, String> {
    let launch = PreparedLaunch::new(LaunchOptions {
        shell: b"/bin/sh".to_vec(),
        argv: vec![
            mux.bin.as_os_str().as_encoded_bytes().to_vec(),
            b"-S".to_vec(),
            mux.socket.as_os_str().as_encoded_bytes().to_vec(),
            b"-f".to_vec(),
            b"/dev/null".to_vec(),
            b"attach-session".to_vec(),
        ],
        environment: vec![
            b"TERM=screen".to_vec(),
            b"LC_ALL=C.UTF-8".to_vec(),
            b"PATH=/bin:/usr/bin".to_vec(),
            b"HOME=/tmp".to_vec(),
        ],
        cwd: b"/".to_vec(),
        home: Some(b"/tmp".to_vec()),
        termios: None,
        backspace: 0x7f,
        size: Winsize {
            rows: SY,
            cols: SX,
            xpixel: 0,
            ypixel: 0,
        },
    })
    .map_err(|e| e.to_string())?;
    let child = AttachedClient(launch.launch().map_err(|e| e.to_string())?);
    let mut out = Vec::new();
    drain(
        child.0.master.as_fd(),
        &mut out,
        Duration::from_millis(500),
        Duration::from_secs(5),
    );
    for step in steps {
        mux.run(step)?;
        drain(
            child.0.master.as_fd(),
            &mut out,
            Duration::from_millis(400),
            Duration::from_secs(5),
        );
    }
    mux.run(&["detach-client"])?;
    let mut teardown = Vec::new();
    drain(
        child.0.master.as_fd(),
        &mut teardown,
        Duration::from_millis(400),
        Duration::from_secs(5),
    );
    Ok(out)
}

/// Parse terminal bytes into a screen and return its visible lines.
fn render(bytes: &[u8]) -> Vec<String> {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(
        u32::from(SX),
        u32::from(SY),
        0,
        ScreenResetPolicy::default(),
        &mut registry,
    )
    .expect("screen");
    let mut palette = rmux_emu::colour::ColourPalette::new();
    let mut ictx = InputCtx::new();
    {
        let mut sink = ScreenOnlySink;
        let mut sw = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy {
                pane_backed: false,
                ..ScreenWritePolicy::default()
            },
            &mut registry,
        );
        let policy = InputPolicy {
            has_pane: false,
            writer_has_pane: false,
            ..InputPolicy::default()
        };
        ictx.parse(&mut sw, Some(&mut palette), &policy, &mut NullSink, bytes);
        sw.finish();
    }
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    for y in 0..u32::from(SY) {
        buf.clear();
        let text = screen.print(Some(y), &mut buf, rmux_emu::screen::borders::acs);
        lines.push(String::from_utf8_lossy(text).trim_end().to_owned());
    }
    lines
}

struct Scenario {
    name: &'static str,
    setup: Vec<Vec<&'static str>>,
    steps: Vec<Vec<&'static str>>,
}

const C: &str = "sh -c 'exec sleep 1000'";

fn scenarios() -> Vec<Scenario> {
    let base = |extra: &[&'static str]| -> Vec<Vec<&'static str>> {
        let mut v = vec![
            vec!["new-session", "-d", "-x", "60", "-y", "16", "-n", "main", C],
            vec!["set", "-g", "window-size", "manual"],
            vec!["set", "-g", "status-left", "[left]"],
            vec!["set", "-g", "status-right", "[right]"],
            vec!["set", "-g", "pane-border-format", " #{pane_index} "],
        ];
        for e in extra {
            v.push(vec!["set", "-g", "status-position", e]);
        }
        v
    };
    let cross = |lines: &'static str| -> Vec<Vec<&'static str>> {
        vec![
            vec!["set", "-g", "pane-border-lines", lines],
            vec!["split-window", "-h", C],
            vec!["split-window", "-v", C],
            vec!["select-pane", "-t", "0"],
            vec!["split-window", "-v", C],
            vec!["select-layout", "tiled"],
        ]
    };
    let mut v = Vec::new();
    for lines in [
        "single", "double", "heavy", "simple", "number", "spaces", "none",
    ] {
        v.push(Scenario {
            name: "cross",
            setup: base(&[]),
            steps: cross(lines),
        });
    }
    for status in ["top", "bottom"] {
        for n in ["off", "on", "2", "3"] {
            let mut steps = vec![vec!["set", "-g", "status", n]];
            steps.push(vec!["split-window", "-v", C]);
            v.push(Scenario {
                name: "status",
                setup: base(&[status]),
                steps,
            });
        }
    }
    for pos in ["top", "bottom"] {
        let mut steps = vec![vec!["set", "-g", "pane-border-status", pos]];
        steps.extend(cross("single"));
        steps.push(vec!["set", "-g", "pane-border-indicators", "both"]);
        steps.push(vec!["select-pane", "-t", "2"]);
        v.push(Scenario {
            name: "pane-status",
            setup: base(&[]),
            steps,
        });
    }
    v.push(Scenario {
        name: "message",
        setup: base(&[]),
        steps: vec![
            vec![
                "set",
                "-g",
                "message-style",
                "bg=blue,width=50%,align=right",
            ],
            vec!["display-message", "-d", "0", "hello ## message"],
        ],
    });
    v.push(Scenario {
        name: "prompt",
        setup: base(&[]),
        steps: vec![
            vec!["command-prompt", "-b", "-I", "abc", "-p", "name:"],
            vec!["send-keys", "-K", "d"],
        ],
    });
    v.push(Scenario {
        name: "menu",
        setup: base(&[]),
        steps: vec![
            vec!["set", "-g", "menu-border-lines", "rounded"],
            vec![
                "display-menu",
                "-x",
                "5",
                "-y",
                "3",
                "-T",
                "Title",
                "One",
                "o",
                "",
                "",
                "",
                "-Two",
                "t",
                "",
                "Three",
                "",
                "",
            ],
            vec!["send-keys", "-K", "Down"],
        ],
    });
    v
}

fn capture(bin: PathBuf, tag: &str, sc: &Scenario) -> Result<Vec<String>, String> {
    let mux = Mux::new(bin, tag);
    let _ = mux.run(&["kill-server"]);
    for step in &sc.setup {
        mux.run(step)?;
    }
    let steps: Vec<&[&str]> = sc.steps.iter().map(|s| s.as_slice()).collect();
    let bytes = attach_and_capture(&mux, &steps);
    Ok(render(&bytes?))
}

#[test]
fn client_screen_matches_oracle() {
    let Some(oracle) = oracle_path() else {
        eprintln!("skipping: oracle tmux not found at oracle/bin/tmux");
        return;
    };
    let Some(rmux) = rmux_path() else {
        eprintln!("skipping: rmux binary not found (set RMUX_BIN)");
        return;
    };
    let mutate = std::env::var_os("RMUX_UI_ORACLE_MUTATE").is_some();
    let mut failures = Vec::new();
    for (i, sc) in scenarios().iter().enumerate() {
        let tag = format!("{}-{i}", sc.name);
        let expected = capture(oracle.clone(), &format!("o-{tag}"), sc).expect("oracle run");
        let mut actual = capture(rmux.clone(), &format!("r-{tag}"), sc).expect("rmux run");
        if mutate {
            actual[0].push('X');
        }
        if expected != actual {
            failures.push(format!(
                "scenario {tag}:\n--- oracle\n{}\n--- rmux\n{}\n",
                expected.join("\n"),
                actual.join("\n")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
