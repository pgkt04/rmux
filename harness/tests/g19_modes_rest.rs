//! G19 exact screen differentials against tmux 8f25579c.
//! An oracle outer terminal hosts each binary's attached inner client on the
//! same PTY. Captures compare all displayed cells and ANSI styles byte for byte;
//! a bounded redraw marker excludes stale/base-screen captures.
//!
//! Only missing binaries skip. `RMUX_G19_MUTATE=1` corrupts one compared capture.
//! Fixed 12:05/23:59:59 clock differentials require an external fixed-time fixture;
//! live clock checks reject rollover samples rather than normalizing digits.

use rmux_harness::differential::{CommandOutput, Difference};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn oracle() -> Option<PathBuf> {
    let path = std::env::var_os("RMUX_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/bin/tmux"));
    path.exists().then_some(path)
}

fn rmux() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RMUX_BINARY") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(dir).join("debug/rmux"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/rmux"));
    candidates.into_iter().find(|p| p.exists())
}

type Script = Vec<Vec<String>>;

struct FixtureServer {
    binary: PathBuf,
    directory: tempfile::TempDir,
}

impl FixtureServer {
    fn new(binary: &Path) -> Self {
        Self {
            binary: std::fs::canonicalize(binary).expect("fixture binary"),
            directory: tempfile::Builder::new()
                .prefix("g19-")
                .tempdir_in("/tmp")
                .expect("private fixture directory"),
        }
    }

    fn socket(&self) -> PathBuf {
        self.directory.path().join("socket")
    }

    fn command(&self, args: &[&str]) -> CommandOutput {
        let mut stdout = tempfile::tempfile().expect("fixture stdout");
        let mut stderr = tempfile::tempfile().expect("fixture stderr");
        let mut child = Command::new(&self.binary)
            .args(["-S"])
            .arg(self.socket())
            .args(["-f", "/dev/null"])
            .args(args)
            .current_dir("/tmp")
            .env_remove("TMUX")
            .env_remove("RMUX")
            .env_remove("TMUX_PANE")
            .env_remove("RMUX_PANE")
            .env("TERM", "screen-256color")
            .env("TZ", "UTC")
            .env("LC_ALL", "C")
            .stdout(Stdio::from(stdout.try_clone().expect("clone stdout")))
            .stderr(Stdio::from(stderr.try_clone().expect("clone stderr")))
            .spawn()
            .expect("fixture command spawn");
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("fixture command wait") {
                break status;
            }
            if start.elapsed() >= Duration::from_secs(10) {
                child.kill().expect("kill timed out fixture command");
                child.wait().expect("reap timed out fixture command");
                panic!("fixture command timed out: {args:?}");
            }
            thread::sleep(Duration::from_millis(10));
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        stdout.rewind().expect("rewind stdout");
        stderr.rewind().expect("rewind stderr");
        stdout.read_to_end(&mut out).expect("read stdout");
        stderr.read_to_end(&mut err).expect("read stderr");
        CommandOutput {
            status: status.code(),
            stdout: out,
            stderr: err,
        }
    }

    fn require(&self, args: &[&str]) -> CommandOutput {
        let output = self.command(args);
        assert_eq!(output.status, Some(0), "{args:?}: {output:?}");
        output
    }

    fn wait_output(&self, target: &str, expected: &[u8]) {
        assert!(!expected.is_empty(), "render barriers must not be empty");
        let start = Instant::now();
        loop {
            let output = self.require(&["capture-pane", "-p", "-t", target]);
            if output.stdout.windows(expected.len()).any(|s| s == expected) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "pane {target} did not render {:?}; last screen: {:?}",
                String::from_utf8_lossy(expected),
                String::from_utf8_lossy(&output.stdout),
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        let _ = self.command(&["kill-server"]);
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

struct AttachedFixture {
    outer: FixtureServer,
    generation: usize,
}

impl AttachedFixture {
    fn new(oracle: &Path) -> Self {
        let outer = FixtureServer::new(oracle);
        outer.require(&[
            "new-session", "-d", "-s", "out", "-n", "terminal", "-x", "80", "-y", "25",
            "stty -echo; printf 'READY\\n'; while IFS= read -r command; do eval \"$command\"; printf '\\033[2J\\033[HREADY\\n'; done",
        ]);
        outer.require(&["set", "-g", "status", "off"]);
        outer.wait_output("out:0.0", b"READY");
        Self {
            outer,
            generation: 0,
        }
    }

    fn resize(&self, sx: &str, sy: &str) {
        let outer_height = (sy.parse::<u32>().expect("fixture height") + 1).to_string();
        self.outer.require(&[
            "resize-window",
            "-t",
            "out:0",
            "-x",
            sx,
            "-y",
            &outer_height,
        ]);
    }

    fn attach(&mut self, inner: &FixtureServer, sx: &str, sy: &str) {
        self.resize(sx, sy);
        let command = format!(
            "(unset TMUX RMUX TMUX_PANE RMUX_PANE; export TERM=screen-256color TZ=UTC LC_ALL=C; {} -S {} -f /dev/null attach-session -t m)",
            shell_quote(&inner.binary),
            shell_quote(&inner.socket()),
        );
        // Keep one shell/PTY alive so literal client names match without masking.
        self.outer
            .require(&["send-keys", "-t", "out:0.0", "-l", &command]);
        self.outer.require(&["send-keys", "-t", "out:0.0", "Enter"]);
        let start = Instant::now();
        loop {
            let output = inner.require(&["list-clients", "-F", "#{session_name}"]);
            if output.stdout == b"m\n" {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "inner attach timed out"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn capture(&mut self, inner: &FixtureServer, sy: &str) -> CommandOutput {
        self.generation += 1;
        let marker = format!("R{:05x}", self.generation);
        inner.require(&["display-message", "-C", "-d", "0", &marker]);
        inner.require(&["refresh-client"]);
        self.outer.wait_output("out:0.0", marker.as_bytes());
        let end = (sy.parse::<u32>().expect("fixture height") - 1).to_string();
        let position = inner.require(&["show-options", "-gv", "-t", "m", "status-position"]);
        let (start_row, end_row) = if position.stdout == b"top\n" {
            (String::from("1"), sy.to_string())
        } else {
            (String::from("0"), end)
        };
        let args = [
            "capture-pane",
            "-p",
            "-e",
            "-t",
            "out:0.0",
            "-S",
            &start_row,
            "-E",
            &end_row,
        ];
        // A slow machine can pause in the middle of a redraw, so a screen
        // counts as settled only after it stays the same for three reads.
        let mut previous = self.outer.require(&args);
        let mut unchanged = 0;
        let start = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(20));
            let current = self.outer.require(&args);
            if current == previous {
                unchanged += 1;
                if unchanged == 2 {
                    return current;
                }
            } else {
                unchanged = 0;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "render did not settle"
            );
            previous = current;
        }
    }

    fn run(&mut self, binary: &Path, steps: &[Vec<String>]) -> Vec<CommandOutput> {
        self.generation = 0;
        let inner = FixtureServer::new(binary);
        let mut output = Vec::new();
        let mut sy = String::from("24");
        for step in steps {
            let args: Vec<_> = step.iter().map(String::as_str).collect();
            if let ["wait-for-pane-output", target, expected] = args.as_slice() {
                inner.wait_output(target, expected.as_bytes());
                continue;
            }
            if args[0] == "capture-pane" {
                output.push(self.capture(&inner, &sy));
                continue;
            }
            let result = inner.command(&args);
            let expected_error = args[0] == "display-panes"
                && args
                    .windows(2)
                    .any(|a| a == ["-d", "abc"] || a == ["-d", "-1"]);
            if !expected_error {
                assert_eq!(result.status, Some(0), "fixture step {args:?}: {result:?}");
            }
            output.push(result);
            if args[0] == "new-session" && args.windows(2).any(|a| a == ["-s", "m"]) {
                let sx = args.windows(2).find(|a| a[0] == "-x").unwrap()[1];
                sy = args.windows(2).find(|a| a[0] == "-y").unwrap()[1].to_string();
                self.attach(&inner, sx, &sy);
            } else if args[0] == "resize-window" && args.windows(2).any(|a| a == ["-t", "m:0"]) {
                let sx = args.windows(2).find(|a| a[0] == "-x").unwrap()[1];
                sy = args.windows(2).find(|a| a[0] == "-y").unwrap()[1].to_string();
                self.resize(sx, &sy);
            }
        }
        inner.require(&["kill-server"]);
        self.outer.wait_output("out:0.0", b"READY");
        output
    }
}

fn report(name: &str, differences: &[Difference]) {
    for d in differences {
        eprintln!(
            "{name}: step {} differs\n  oracle: status={:?}\n  stdout={:?}\n  stderr={:?}\n  rmux:   status={:?}\n  stdout={:?}\n  stderr={:?}",
            d.command_index,
            d.left.status,
            String::from_utf8_lossy(&d.left.stdout),
            String::from_utf8_lossy(&d.left.stderr),
            d.right.status,
            String::from_utf8_lossy(&d.right.stdout),
            String::from_utf8_lossy(&d.right.stderr),
        );
    }
}

fn differential(name: &str, steps: &[Vec<String>]) -> Option<Vec<Difference>> {
    let (Some(oracle), Some(rmux)) = (oracle(), rmux()) else {
        eprintln!("{name}: skipped, oracle or rmux binary missing (set RMUX_ORACLE / RMUX_BINARY)");
        return None;
    };
    let mut fixture = AttachedFixture::new(&oracle);
    let left = fixture.run(&oracle, steps);
    let mut right = fixture.run(&rmux, steps);
    if std::env::var_os("RMUX_G19_MUTATE").is_some() {
        let index = steps
            .iter()
            .filter(|s| s[0] != "wait-for-pane-output")
            .position(|s| s[0] == "capture-pane")
            .expect("differential must capture a displayed screen");
        right[index]
            .stdout
            .extend_from_slice(b"__mutated_render__\n");
    }
    let differences: Vec<_> = left
        .into_iter()
        .zip(right)
        .enumerate()
        .filter_map(|(command_index, (left, right))| {
            (left != right).then_some(Difference {
                command_index,
                left,
                right,
            })
        })
        .collect();
    report(name, &differences);
    Some(differences)
}

macro_rules! differential_test {
    ($name:ident, $steps:expr) => {
        #[test]
        fn $name() {
            if let Some(differences) = differential(stringify!($name), &$steps) {
                assert!(differences.is_empty(), "{} steps differ", differences.len());
            }
        }
    };
}

fn v(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_string()).collect()
}

fn default_session(sx: &str, sy: &str) -> Script {
    vec![
        v(&[
            "new-session",
            "-d",
            "-s",
            "m",
            "-x",
            sx,
            "-y",
            sy,
            "stty -echo; printf '__g19_base_ready__\\n'; exec cat",
        ]),
        v(&["wait-for-pane-output", "m:0", "__g19_base_ready__"]),
    ]
}

fn session(sx: &str, sy: &str) -> Script {
    let mut steps = default_session(sx, sy);
    // An explicit window name disables automatic-rename in both implementations.
    steps[0].splice(4..4, [String::from("-n"), String::from("main")]);
    steps.push(v(&["set", "-g", "display-panes-time", "60000"]));
    steps.push(v(&["set", "-g", "pane-border-lines", "single"]));
    steps
}

fn capture() -> Vec<String> {
    v(&["capture-pane", "-p", "-e", "-t", "m:0"])
}

/// `display-panes` picture for several layouts and sizes (differential 6).
fn display_panes_script(sx: &str, sy: &str, layout: &str, status: &str) -> Script {
    let mut s = session(sx, sy);
    s.push(v(&["split-window", "-t", "m:0", "cat"]));
    s.push(v(&["split-window", "-h", "-t", "m:0", "cat"]));
    s.push(v(&["split-window", "-t", "m:0", "cat"]));
    s.push(v(&["select-layout", "-t", "m:0", layout]));
    s.push(v(&["set", "-g", "pane-border-status", status]));
    s.push(v(&["set", "-g", "display-panes-format", "P#{pane_index}"]));
    s.push(v(&["display-panes", "-d", "0", "-t", "m:0.0"]));
    s.push(v(&["display-message", "-p", "-t", "m:0.0", "#{pane_mode}"]));
    s.push(v(&["capture-pane", "-p", "-e", "-t", "m:0.0"]));
    s.push(v(&["resize-window", "-t", "m:0", "-x", "40", "-y", "10"]));
    s.push(v(&["capture-pane", "-p", "-e", "-t", "m:0.0"]));
    s.push(v(&["send-keys", "-t", "m:0.0", "q"]));
    s.push(v(&[
        "display-message",
        "-p",
        "-t",
        "m:0.0",
        "#{pane_mode}#{window_zoomed_flag}",
    ]));
    s.push(v(&["display-panes", "-N", "-d", "0", "-t", "m:0.0"]));
    s.push(v(&["send-keys", "-t", "m:0.0", "1"]));
    s.push(v(&["display-message", "-p", "-t", "m:0.0", "#{pane_mode}"]));
    s.push(v(&["send-keys", "-t", "m:0.0", "Escape"]));
    s.push(v(&["display-message", "-p", "-t", "m:0.0", "#{pane_mode}"]));
    s.push(v(&[
        "display-panes",
        "-d",
        "0",
        "-t",
        "m:0.0",
        "set -g @picked %%",
    ]));
    s.push(v(&["send-keys", "-t", "m:0.0", "2"]));
    s.push(v(&["show", "-gv", "@picked"]));
    s.push(v(&["display-message", "-p", "-t", "m:0.0", "#{pane_mode}"]));
    s
}

differential_test!(
    display_panes_tiled_small,
    display_panes_script("40", "10", "tiled", "off")
);
differential_test!(
    display_panes_main_vertical_large,
    display_panes_script("200", "50", "main-vertical", "off")
);
differential_test!(
    display_panes_status_top,
    display_panes_script("80", "24", "tiled", "top")
);
differential_test!(
    display_panes_status_bottom_even,
    display_panes_script("80", "24", "even-horizontal", "bottom")
);

differential_test!(display_panes_floating, {
    let mut s = session("80", "24");
    s.push(v(&["split-window", "-t", "m:0", "cat"]));
    s.push(v(&[
        "new-pane", "-d", "-B", "none", "-x", "30", "-y", "8", "-X", "8", "-Y", "3", "-t", "m:0",
        "cat",
    ]));
    s.push(v(&["set", "-g", "display-panes-format", ""]));
    s.push(v(&["display-panes", "-Z", "-d", "0", "-t", "m:0.0"]));
    s.push(v(&["display-message", "-p", "-t", "m:0.0", "#{pane_mode}"]));
    s.push(v(&["capture-pane", "-p", "-e", "-t", "m:0.0"]));
    s.push(v(&["send-keys", "-t", "m:0.0", "q"]));
    s.push(v(&["display-panes", "-d", "0", "-t", "m:0.0"]));
    s.push(v(&["capture-pane", "-p", "-e", "-t", "m:0.0"]));
    s.push(v(&["send-keys", "-t", "m:0.0", "q"]));
    s.push(v(&[
        "display-message",
        "-p",
        "-t",
        "m:0.0",
        "#{pane_mode}#{window_zoomed_flag}",
    ]));
    s
});

differential_test!(display_panes_delay_errors, {
    let mut s = session("80", "24");
    s.push(v(&["display-panes", "-d", "abc", "-t", "m:0"]));
    s.push(v(&["display-panes", "-d", "-1", "-t", "m:0"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s.push(capture());
    s
});

fn wall_second() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

fn clock_differential(style: &str) {
    let (Some(oracle), Some(rmux)) = (oracle(), rmux()) else {
        eprintln!("clock style {style}: skipped, oracle or rmux binary missing");
        return;
    };
    eprintln!(
        "clock style {style}: live TZ=UTC comparison; fixed 12:05/23:59:59 oracle fixture unavailable"
    );
    let left = FixtureServer::new(&oracle);
    let right = FixtureServer::new(&rmux);
    let mut left_terminal = AttachedFixture::new(&oracle);
    let mut right_terminal = AttachedFixture::new(&oracle);
    for (inner, terminal) in [(&left, &mut left_terminal), (&right, &mut right_terminal)] {
        inner.require(&[
            "new-session",
            "-d",
            "-s",
            "m",
            "-n",
            "clock",
            "-x",
            "80",
            "-y",
            "24",
            "cat",
        ]);
        terminal.attach(inner, "80", "24");
        inner.require(&["set", "-w", "-t", "m:0", "clock-mode-style", style]);
        inner.require(&["set", "-w", "-t", "m:0", "clock-mode-colour", "red"]);
    }
    for (sx, sy) in [("80", "24"), ("20", "5"), ("6", "3")] {
        for (inner, terminal) in [(&left, &mut left_terminal), (&right, &mut right_terminal)] {
            inner.require(&["resize-window", "-t", "m:0", "-x", sx, "-y", sy]);
            terminal.resize(sx, sy);
        }
        // A clock without seconds changes once a minute.
        let period = if style.ends_with("-with-seconds") {
            1
        } else {
            60
        };
        let mut matched = false;
        for _ in 0..20 {
            // Start near the top of a second, so the time is less likely to
            // change during the sample. Re-entering clock mode makes both
            // renderers read the current local time.
            let subsec = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .subsec_millis();
            if subsec > 100 {
                thread::sleep(Duration::from_millis(u64::from(1000 - subsec)));
            }
            let before = wall_second() / period;
            left.require(&["clock-mode", "-t", "m:0"]);
            right.require(&["clock-mode", "-t", "m:0"]);
            let a = left_terminal.capture(&left, sy);
            let mut b = right_terminal.capture(&right, sy);
            if std::env::var_os("RMUX_G19_MUTATE").is_some() {
                b.stdout.extend_from_slice(b"__mutated_render__\n");
            }
            // rmux is captured after the oracle. If the clock ticked between
            // the two, a second oracle capture shows the time that rmux drew.
            let same = a == b || left_terminal.capture(&left, sy) == b;
            let moved = wall_second() / period != before;
            left.require(&["send-keys", "-t", "m:0", "x"]);
            right.require(&["send-keys", "-t", "m:0", "x"]);
            if same {
                matched = true;
                break;
            }
            // A difference is excused, and sampled again, only when the shown
            // time may have changed during the sample.
            if !moved {
                assert_eq!(a, b, "clock style {style}, {sx}x{sy}");
            }
        }
        assert!(
            matched,
            "clock style {style}, {sx}x{sy}: the time changed in each of 20 samples"
        );
        assert_eq!(
            left.require(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]),
            right.require(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]),
            "clock exit",
        );
    }
}

#[test]
fn clock_style_0() {
    clock_differential("12");
}

#[test]
fn clock_style_1() {
    clock_differential("24");
}

#[test]
fn clock_compact_style_2() {
    clock_differential("12-with-seconds");
}

#[test]
fn clock_style_3() {
    clock_differential("24-with-seconds");
}

// Capture lines 0..199, not merely the first and final screens. Section roots
// start collapsed in C; M-+ expands them before scrolling 50 rows at a time.
differential_test!(customize_mode_format, {
    let mut s = default_session("80", "50");
    s.push(v(&[
        "customize-mode",
        "-N",
        "-t",
        "m:0",
        "-F",
        "#{option_name}=#{option_value}",
    ]));
    s.push(v(&["send-keys", "-t", "m:0", "M-+", "g"]));
    s.push(capture());
    for _ in 0..49 {
        s.push(v(&["send-keys", "-t", "m:0", "Down"]));
    }
    for _ in 0..3 {
        s.push(v(&["send-keys", "-t", "m:0", "NPage"]));
        s.push(capture());
    }
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    s
});

differential_test!(customize_mode_key_filter_and_preview, {
    let mut s = session("80", "50");
    s.push(v(&["set", "-g", "@user", "value"]));
    s.push(v(&["customize-mode", "-t", "m:0", "-f", "#{is_key}"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    s.push(v(&["customize-mode", "-t", "m:0"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "H"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "C"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s
});

// `switch-mode` list and match highlighting (differential 5).
differential_test!(switch_mode_match_style, {
    let mut s = session("80", "24");
    s.push(v(&[
        "new-session",
        "-d",
        "-s",
        "alpha",
        "-n",
        "alpha-main",
        "cat",
    ]));
    s.push(v(&[
        "new-session",
        "-d",
        "-s",
        "beta",
        "-n",
        "beta-main",
        "cat",
    ]));
    s.push(v(&["new-window", "-d", "-t", "alpha", "-n", "work", "cat"]));
    s.push(v(&[
        "set",
        "-g",
        "switch-mode-match-style",
        "fg=red,underscore",
    ]));
    s.push(v(&["set", "-g", "mode-style", "bg=blue"]));
    s.push(v(&["switch-mode", "-t", "m:0", "-F", "#{session_name}"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "-l", "al"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "Down"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "C-u"]));
    s.push(v(&["send-keys", "-t", "m:0", "-l", "zzzz"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "Enter"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s.push(v(&["send-keys", "-t", "m:0", "Escape"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s.push(v(&[
        "switch-mode",
        "-w",
        "-t",
        "m:0",
        "-F",
        "#{window_name}",
    ]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "C-g"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s
});

// buffer-mode list, preview, delete and no-match fallback.
differential_test!(buffer_mode_list_and_delete, {
    let mut s = session("80", "24");
    s.push(v(&["set-buffer", "-b", "one", "first\tline\nsecond"]));
    s.push(v(&["set-buffer", "-b", "two", "other"]));
    s.push(v(&["set-buffer", "-b", "three", "third \x01 ctrl"]));
    s.push(v(&[
        "choose-buffer",
        "-t",
        "m:0",
        "-F",
        "#{buffer_name}:#{buffer_size}",
    ]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "Down", "d"]));
    s.push(v(&["list-buffers", "-F", "#{buffer_name}"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    s.push(v(&[
        "choose-buffer",
        "-t",
        "m:0",
        "-f",
        "#{m:nomatch*,#{buffer_name}}",
        "-F",
        "#{buffer_name}",
    ]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "C-t", "D"]));
    s.push(v(&["list-buffers", "-F", "#{buffer_name}"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s
});

fn ready_pane(s: &mut Script, target: &str, marker: &str) {
    s.push(v(&["wait-for-pane-output", target, marker]));
}

fn pane_command(marker: &str) -> String {
    format!("printf '{marker}\\n'; exec sleep 300")
}

fn tree_prefix_script(sy: &str, preview: usize) -> Script {
    let mut s = session("80", sy);
    // 25 lines in m plus 16 in n minus the hidden mode pane = exactly 40.
    for (name, windows, panes) in [("m", 6, 3), ("n", 3, 4)] {
        for window in 0..windows {
            let target = format!("{name}:{window}");
            if name == "n" && window == 0 {
                s.push(v(&[
                    "new-session",
                    "-d",
                    "-s",
                    name,
                    "-n",
                    "w0",
                    "-x",
                    "80",
                    "-y",
                    "50",
                    &pane_command("TREE_SOURCE"),
                ]));
            } else if window != 0 {
                s.push(v(&[
                    "new-window",
                    "-d",
                    "-t",
                    &target,
                    "-n",
                    &format!("w{window}"),
                    &pane_command("TREE_SOURCE"),
                ]));
            }
            for pane in 1..panes {
                s.push(v(&[
                    "split-window",
                    "-d",
                    "-h",
                    "-t",
                    &format!("{target}.{}", pane - 1),
                    &pane_command("TREE_SOURCE"),
                ]));
                s.push(v(&["select-layout", "-t", &target, "even-horizontal"]));
                ready_pane(&mut s, &format!("{target}.{pane}"), "TREE_SOURCE");
            }
            if !(name == "m" && window == 0) {
                ready_pane(&mut s, &format!("{target}.0"), "TREE_SOURCE");
            }
        }
    }
    s.push(v(&[
        "list-panes",
        "-a",
        "-F",
        "#{session_name}:#{window_index}.#{pane_index}",
    ]));
    s.push(v(&[
        "set",
        "-g",
        "tree-mode-selection-style",
        "fg=yellow,bg=blue",
    ]));
    s.push(v(&["set", "-g", "tree-mode-border-style", "fg=red"]));
    let mut choose = v(&[
        "choose-tree",
        "-Z",
        "-h",
        "-t",
        "m:0.0",
        "-O",
        "index",
        "-F",
        "row",
    ]);
    choose.extend((0..preview).map(|_| String::from("-N")));
    s.push(choose);
    s.push(v(&["send-keys", "-t", "m:0.0", "g"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0.0", "Down", "Down", "t"]));
    s.push(capture());
    for _ in 0..20 {
        s.push(v(&["send-keys", "-t", "m:0.0", "NPage"]));
        s.push(capture());
    }
    for key in ["G", "g", "Left", "Right", "M--", "M-+"] {
        s.push(v(&["send-keys", "-t", "m:0.0", key]));
        s.push(capture());
    }
    s
}

#[test]
fn tree_prefix_three_levels_forty_lines() {
    for sy in ["10", "24", "50"] {
        for (preview, name) in [(1, "off"), (0, "normal"), (2, "big")] {
            let name = format!("tree_prefix_80x{sy}_{name}");
            if let Some(differences) = differential(&name, &tree_prefix_script(sy, preview)) {
                assert!(
                    differences.is_empty(),
                    "{name}: {} steps differ",
                    differences.len()
                );
            }
        }
    }
}

differential_test!(buffer_key_column_thirty_six_and_unkeyed, {
    let mut s = session("80", "50");
    for index in 0..40 {
        s.push(v(&[
            "set-buffer",
            "-b",
            &format!("b{index:02}"),
            &format!("buffer {index:02}"),
        ]));
    }
    s.push(v(&[
        "choose-buffer",
        "-N",
        "-O",
        "name",
        "-t",
        "m:0",
        "-F",
        "#{buffer_size}",
    ]));
    s.push(v(&["send-keys", "-t", "m:0", "g"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "G", "t"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    for key in ["0", "9", "M-a", "M-z"] {
        s.push(v(&[
            "choose-buffer",
            "-N",
            "-O",
            "name",
            "-t",
            "m:0",
            "-F",
            "#{buffer_size}",
            "set -g @key_target %%",
        ]));
        s.push(v(&["send-keys", "-t", "m:0", key]));
        s.push(v(&["show", "-gv", "@key_target"]));
    }
    s
});

fn help_script(command: &str, sx: &str, sy: &str) -> Script {
    let mut s = session(sx, sy);
    s.push(v(&["set-buffer", "-b", "help", "fixed buffer"]));
    let format = if command == "choose-client" {
        "fixed client"
    } else {
        "help row"
    };
    s.push(v(&[command, "-N", "-t", "m:0", "-F", format]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "F1"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    s
}

#[test]
fn all_tree_backend_help_boxes() {
    // Width 60 actually fits all four C help widths; height 24 exercises
    // the insufficient-box case. Both cases still compare the entire screen.
    for command in [
        "choose-tree",
        "choose-buffer",
        "choose-client",
        "customize-mode",
    ] {
        for sx in ["80", "60"] {
            for sy in ["50", "24"] {
                let name = format!("{command}_help_{sx}x{sy}");
                if let Some(differences) = differential(&name, &help_script(command, sx, sy)) {
                    assert!(
                        differences.is_empty(),
                        "{name}: {} steps differ",
                        differences.len()
                    );
                }
            }
        }
    }
}

fn strip_script(sx: &str, count: usize, window: bool) -> Script {
    let mut s = session(sx, "50");
    s.push(v(&[
        "set",
        "-g",
        "tree-mode-preview-format",
        "#{window_name}/#{pane_index}",
    ]));
    s.push(v(&[
        "set",
        "-g",
        "tree-mode-preview-style",
        "fg=green,bg=black",
    ]));
    s.push(v(&["set", "-g", "tree-mode-border-style", "fg=red"]));
    if window {
        s.push(v(&[
            "new-window",
            "-d",
            "-t",
            "m:1",
            "-n",
            "strips",
            &pane_command("P00"),
        ]));
        // Source geometry is deliberately wide enough for twelve real panes.
        s.push(v(&["resize-window", "-t", "m:1", "-x", "400", "-y", "50"]));
        ready_pane(&mut s, "m:1.0", "P00");
        for pane in 1..count {
            let marker = format!("P{pane:02}");
            s.push(v(&[
                "split-window",
                "-d",
                "-h",
                "-t",
                &format!("m:1.{}", pane - 1),
                &pane_command(&marker),
            ]));
            s.push(v(&["select-layout", "-t", "m:1", "even-horizontal"]));
            ready_pane(&mut s, &format!("m:1.{pane}"), &marker);
        }
        s.push(v(&[
            "choose-tree",
            "-w",
            "-h",
            "-t",
            "m:0",
            "-O",
            "index",
            "-F",
            "strip",
        ]));
        s.push(v(&["send-keys", "-t", "m:0", "Down"]));
    } else {
        for index in 0..count {
            let marker = format!("W{index:02}");
            if index == 0 {
                s.push(v(&[
                    "new-session",
                    "-d",
                    "-s",
                    "strips",
                    "-n",
                    "w00",
                    "-x",
                    "80",
                    "-y",
                    "24",
                    &pane_command(&marker),
                ]));
            } else {
                s.push(v(&[
                    "new-window",
                    "-d",
                    "-t",
                    &format!("strips:{index}"),
                    "-n",
                    &format!("w{index:02}"),
                    &pane_command(&marker),
                ]));
            }
            ready_pane(&mut s, &format!("strips:{index}.0"), &marker);
        }
        s.push(v(&[
            "choose-tree",
            "-s",
            "-h",
            "-t",
            "m:0",
            "-O",
            "index",
            "-F",
            "strip",
        ]));
        s.push(v(&["send-keys", "-t", "m:0", "Down"]));
    }
    s.push(capture());
    for _ in 0..count + 1 {
        s.push(v(&["send-keys", "-t", "m:0", ">"]));
        s.push(capture());
    }
    for _ in 0..count + 1 {
        s.push(v(&["send-keys", "-t", "m:0", "<"]));
        s.push(capture());
    }
    s.push(v(&["send-keys", "-t", "m:0", "q"]));
    // Exercise initial centered/end strip placement, not only manual offsets.
    for current in [count / 2, count - 1] {
        if window {
            s.push(v(&["select-pane", "-t", &format!("m:1.{current}")]));
            s.push(v(&[
                "choose-tree",
                "-w",
                "-h",
                "-t",
                "m:0",
                "-O",
                "index",
                "-F",
                "strip",
            ]));
        } else {
            s.push(v(&["select-window", "-t", &format!("strips:{current}")]));
            s.push(v(&[
                "choose-tree",
                "-s",
                "-h",
                "-t",
                "m:0",
                "-O",
                "index",
                "-F",
                "strip",
            ]));
        }
        s.push(v(&["send-keys", "-t", "m:0", "Down"]));
        s.push(capture());
        s.push(v(&["send-keys", "-t", "m:0", "q"]));
    }
    s
}

#[test]
fn session_and_window_preview_strip_matrix() {
    for sx in ["40", "80", "200"] {
        for count in [1, 3, 5, 12] {
            for window in [false, true] {
                let kind = if window { "window" } else { "session" };
                let name = format!("{kind}_strips_{count}_width_{sx}");
                if let Some(differences) = differential(&name, &strip_script(sx, count, window)) {
                    assert!(
                        differences.is_empty(),
                        "{name}: {} steps differ",
                        differences.len()
                    );
                }
            }
        }
    }
}

#[test]
fn customize_option_and_environment_previews() {
    for (name, filter) in [
        ("status-style", "#{==:#{option_name},status-style}"),
        ("prefix", "#{==:#{option_name},prefix}"),
        ("@user", "#{==:#{option_name},@user}"),
        ("PATH", "#{==:#{environment_name},PATH}"),
    ] {
        let mut s = session("80", "50");
        s.push(v(&["set", "-g", "@user", "a deterministic user value"]));
        s.push(v(&["set-environment", "-g", "PATH", "/bin:/usr/bin"]));
        s.push(v(&["set-environment", "-t", "m", "PATH", "/bin:/usr/bin"]));
        s.push(v(&["customize-mode", "-t", "m:0", "-f", filter]));
        s.push(v(&["send-keys", "-t", "m:0", "M-+", "g"]));
        s.push(capture());
        // Empty filtered section roots remain in C; walk each root and each
        // matching child, covering global and session PATH ownership as well.
        for _ in 0..12 {
            s.push(v(&["send-keys", "-t", "m:0", "Down"]));
            s.push(capture());
        }
        if let Some(differences) = differential(name, &s) {
            assert!(
                differences.is_empty(),
                "{name}: {} steps differ",
                differences.len()
            );
        }
    }
}

#[test]
fn display_panes_complete_geometry_matrix() {
    for (sx, sy) in [("40", "10"), ("200", "50")] {
        for (layout, status) in [
            ("tiled", "off"),
            ("main-vertical", "off"),
            ("tiled", "top"),
            ("tiled", "bottom"),
        ] {
            let name = format!("display_panes_{layout}_{status}_{sx}x{sy}");
            if let Some(differences) =
                differential(&name, &display_panes_script(sx, sy, layout, status))
            {
                assert!(
                    differences.is_empty(),
                    "{name}: {} steps differ",
                    differences.len()
                );
            }
        }
        let mut s = session(sx, sy);
        s.push(v(&["set", "-g", "display-panes-format", "P#{pane_index}"]));
        s.push(v(&["split-window", "-t", "m:0", "cat"]));
        s.push(v(&[
            "new-pane", "-d", "-B", "single", "-x", "20", "-y", "6", "-X", "8", "-Y", "2", "-t",
            "m:0", "cat",
        ]));
        s.push(v(&["display-panes", "-d", "0", "-t", "m:0.0"]));
        s.push(capture());
        s.push(v(&["send-keys", "-t", "m:0.0", "q"]));
        let name = format!("display_panes_floating_{sx}x{sy}");
        if let Some(differences) = differential(&name, &s) {
            assert!(
                differences.is_empty(),
                "{name}: {} steps differ",
                differences.len()
            );
        }
    }
}

differential_test!(switch_mode_window_matches_and_stale_target, {
    let mut s = session("80", "24");
    for name in ["alpha", "alpine", "beta"] {
        s.push(v(&["new-window", "-d", "-t", "m:", "-n", name, "cat"]));
    }
    s.push(v(&[
        "set",
        "-g",
        "switch-mode-match-style",
        "fg=red,underscore",
    ]));
    s.push(v(&["set", "-g", "mode-style", "bg=blue"]));
    s.push(v(&[
        "switch-mode",
        "-w",
        "-t",
        "m:0",
        "-F",
        "#{window_name}",
    ]));
    s.push(v(&["send-keys", "-t", "m:0", "-l", "al"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "Down"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "Escape"]));
    s.push(v(&[
        "new-session",
        "-d",
        "-s",
        "stale",
        "-n",
        "stale-main",
        "cat",
    ]));
    s.push(v(&["switch-mode", "-t", "m:0", "-F", "#{session_name}"]));
    s.push(v(&["send-keys", "-t", "m:0", "-l", "stale"]));
    s.push(capture());
    s.push(v(&["rename-session", "-t", "stale", "renamed"]));
    s.push(v(&["send-keys", "-t", "m:0", "Enter"]));
    s.push(v(&["display-message", "-p", "-t", "m:0", "#{pane_mode}"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "Escape"]));
    s
});

differential_test!(client_mode_hidden_source_status_preview, {
    let mut s = session("80", "50");
    s.push(v(&[
        "new-window",
        "-t",
        "m:1",
        "-n",
        "source",
        &pane_command("CLIENT_SOURCE"),
    ]));
    ready_pane(&mut s, "m:1", "CLIENT_SOURCE");
    s.push(v(&["select-window", "-t", "m:0"]));
    s.push(v(&["set", "-g", "status-left", "FIXED STATUS"]));
    s.push(v(&["set", "-g", "status-right", ""]));
    for position in ["top", "bottom"] {
        s.push(v(&["set", "-g", "status-position", position]));
        s.push(v(&[
            "choose-client",
            "-h",
            "-t",
            "m:0",
            "-F",
            "#{session_name}:#{client_width}x#{client_height}",
        ]));
        s.push(capture());
        s.push(v(&["send-keys", "-t", "m:0", "q"]));
    }
    s
});

differential_test!(buffer_escaped_multiline_preview, {
    let mut s = session("80", "50");
    s.push(v(&[
        "set-buffer",
        "-b",
        "escaped",
        "first\tcolumn\nsecond \x01\x02\x7f\r\nthird\\line",
    ]));
    s.push(v(&[
        "choose-buffer",
        "-t",
        "m:0",
        "-F",
        "#{buffer_name}:#{buffer_size}",
    ]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "v"]));
    s.push(capture());
    s.push(v(&["send-keys", "-t", "m:0", "v"]));
    s.push(capture());
    s
});
