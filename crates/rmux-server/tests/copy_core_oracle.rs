// Ported from tmux window-copy.c and regress/copy-mode-*.sh @ 8f25579c
//! G18 differential cases 2--12. Every observation comes from a private real
//! server or an attached PTY; capture -M is the backing, not the visible screen.
//! Run with RMUX_COPY_BINARY and RMUX_ORACLE, or the default debug/oracle paths.
//! RMUX_COPY_CORE_ORACLE_MUTATE=1 changes one observed byte (negative control).
//! Wall-clock top_line_time is normalized, preserving absent/zero/nonzero. PTY
//! state and OSC52 payloads are exact; redraw packetization is not canonical.

use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::input::InputCtx;
use rmux_emu::input::effect::{InputPolicy, NullSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

const PANE: &str = "core:0.0";
const FORMATS: [&str; 24] = [
    "top_line_time",
    "scroll_position",
    "copy_position",
    "copy_position_limit",
    "copy_line_numbers",
    "refresh_active",
    "rectangle_toggle",
    "copy_cursor_x",
    "copy_cursor_y",
    "selection_start_x",
    "selection_start_y",
    "selection_end_x",
    "selection_end_y",
    "selection_active",
    "selection_present",
    "selection_mode",
    "search_present",
    "search_timed_out",
    "search_count",
    "search_count_partial",
    "search_match",
    "copy_cursor_word",
    "copy_cursor_line",
    "copy_cursor_hyperlink",
];
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);
static NEXT_COMMAND: AtomicU64 = AtomicU64::new(0);
static CORE_SLOTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
struct CoreSlot;
impl CoreSlot {
    fn acquire() -> Self {
        let (slots, ready) = &CORE_SLOTS;
        let mut active = slots.lock().unwrap_or_else(|error| error.into_inner());
        while *active == 2 {
            active = ready
                .wait(active)
                .unwrap_or_else(|error| error.into_inner());
        }
        *active += 1;
        Self
    }
}
impl Drop for CoreSlot {
    fn drop(&mut self) {
        let (slots, ready) = &CORE_SLOTS;
        *slots.lock().unwrap_or_else(|error| error.into_inner()) -= 1;
        ready.notify_one();
    }
}

// Regular files avoid waiting for pipe EOF inherited by a daemon or pane.
// Completion follows only this command client's owned PID and a deadline.
fn bounded_output(
    command: &mut Command,
    directory: &Path,
    timeout: Duration,
) -> io::Result<Output> {
    let id = NEXT_COMMAND.fetch_add(1, Ordering::Relaxed);
    let stdout_path = directory.join(format!("command-{id}.stdout"));
    let stderr_path = directory.join(format!("command-{id}.stderr"));
    let stdout = File::create(&stdout_path)?;
    let stderr = File::create(&stderr_path)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()?;
    let until = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(5)),
            result => {
                let error = match result {
                    Err(error) => error,
                    _ => io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("command deadline {timeout:?}: {command:?}"),
                    ),
                };
                let _ = child.kill();
                let reap_until = Instant::now() + Duration::from_secs(2);
                while matches!(child.try_wait(), Ok(None)) && Instant::now() < reap_until {
                    std::thread::sleep(Duration::from_millis(5));
                }
                break Err(error);
            }
        }
    };
    let stdout = std::fs::read(&stdout_path);
    let stderr = std::fs::read(&stderr_path);
    let _ = std::fs::remove_file(stdout_path);
    let _ = std::fs::remove_file(stderr_path);
    match status {
        Ok(status) => Ok(Output {
            status,
            stdout: stdout?,
            stderr: stderr?,
        }),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "{error}; stdout={:?}; stderr={:?}",
                stdout.unwrap_or_default(),
                stderr.unwrap_or_default()
            ),
        )),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Bytes {
    length: usize,
    bytes: Vec<u8>,
}
impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self {
            length: bytes.len(),
            bytes,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Observation {
    label: String,
    success: bool,
    stdout: Bytes,
    stderr: Bytes,
}
#[derive(Clone)]
enum Step {
    Command(Vec<String>),
    Emit(Vec<u8>),
    Check(String),
    Buffer(String),
    Input(Vec<u8>),
    MouseScrollbar,
    RequireFormat(&'static str, &'static str, &'static str),
    Wait(Duration),
}
struct Case {
    name: String,
    width: u16,
    height: u16,
    history: u32,
    mode_keys: &'static str,
    attached: bool,
    steps: Vec<Step>,
}
impl Case {
    fn new(name: impl Into<String>, width: u16, height: u16) -> Self {
        Self {
            name: name.into(),
            width,
            height,
            history: 4096,
            mode_keys: "vi",
            attached: true,
            steps: Vec::new(),
        }
    }
    fn command(&mut self, words: &[&str]) {
        self.steps.push(Step::Command(
            words.iter().map(|s| (*s).to_owned()).collect(),
        ));
    }
    fn copy(&mut self, words: &[&str]) {
        let mut args = vec!["send-keys", "-t", PANE, "-X"];
        args.extend_from_slice(words);
        self.command(&args);
    }
    fn repeat(&mut self, n: usize, words: &[&str]) {
        let count = n.to_string();
        let mut args = vec!["send-keys", "-t", PANE, "-N", &count, "-X"];
        args.extend_from_slice(words);
        self.command(&args);
    }
    fn enter(&mut self) {
        self.command(&["copy-mode", "-t", PANE]);
        self.steps
            .push(Step::RequireFormat(PANE, "#{pane_mode}", "copy-mode"));
    }
    fn emit(&mut self, bytes: impl Into<Vec<u8>>) {
        self.steps.push(Step::Emit(bytes.into()));
    }
    fn check(&mut self, label: impl Into<String>) {
        self.steps.push(Step::Check(label.into()));
    }
    fn buffer(&mut self, label: impl Into<String>) {
        self.copy(&["copy-selection-no-clear"]);
        self.steps.push(Step::Buffer(label.into()));
    }
    fn wait(&mut self) {
        self.steps.push(Step::Wait(Duration::from_millis(250)));
    }
}

struct Mux {
    binary: PathBuf,
    directory: PathBuf,
    feed: Option<File>,
    next_emit: usize,
}
impl Mux {
    fn run(&self, args: &[String]) -> Output {
        self.try_run(args, Duration::from_secs(10))
            .unwrap_or_else(|e| {
                panic!(
                    "{} socket {} {args:?}: {e}",
                    self.binary.display(),
                    self.directory.join("socket").display()
                )
            })
    }
    fn try_run(&self, args: &[String], timeout: Duration) -> io::Result<Output> {
        let mut command = Command::new(&self.binary);
        command
            .args(["-S"])
            .arg(self.directory.join("socket"))
            .args(["-f", "/dev/null"])
            .args(args)
            .env("TERM", "screen-256color")
            .env("LC_ALL", "C.UTF-8")
            .env("PATH", "/bin:/usr/bin")
            .env_remove("TMUX")
            .env_remove("RMUX")
            .env_remove("TMUX_PANE")
            .env_remove("RMUX_PANE");
        bounded_output(&mut command, &self.directory, timeout)
    }
    fn command(&self, args: &[&str]) -> Vec<u8> {
        let args = args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        self.checked(&args)
    }
    fn checked(&self, args: &[String]) -> Vec<u8> {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{} {args:?}: {}",
            self.binary.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stderr.is_empty(),
            "{} {args:?}: unexpected stderr {:?}",
            self.binary.display(),
            out.stderr
        );
        out.stdout
    }
    fn batch(&self, commands: &[Vec<&str>]) -> Vec<Output> {
        const BOUNDARY: &str = "\x1eCOPY-CORE-OBSERVATION\x1e";
        let mut args = Vec::new();
        for command in commands {
            if !args.is_empty() {
                args.push(";".to_owned());
            }
            args.extend(command.iter().map(|word| (*word).to_owned()));
            args.extend([";", "display-message", "-p", BOUNDARY].map(str::to_owned));
        }
        let reply = self.run(&args);
        assert!(
            reply.status.success() && reply.stderr.is_empty(),
            "batched observation failed: {reply:?}"
        );
        let separator = format!("{BOUNDARY}\n");
        let mut rest = reply.stdout.as_slice();
        let mut replies = Vec::new();
        for _ in commands {
            let end = rest
                .windows(separator.len())
                .position(|bytes| bytes == separator.as_bytes())
                .expect("observation boundary");
            replies.push(Output {
                status: reply.status,
                stdout: rest[..end].to_vec(),
                stderr: Vec::new(),
            });
            rest = &rest[end + separator.len()..];
        }
        assert!(rest.is_empty(), "unexpected trailing observation bytes");
        replies
    }
    fn new(binary: &Path, case: &Case, role: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "rmux-copy-core-{}-{}-{role}",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).expect("unique private socket directory");
        let mut mux = Self {
            binary: binary.to_owned(),
            directory,
            feed: None,
            next_emit: 0,
        };
        let fifo = mux.directory.join("feed");
        let mkfifo = bounded_output(
            Command::new("mkfifo").arg(&fifo),
            &mux.directory,
            Duration::from_secs(5),
        )
        .expect("mkfifo");
        assert!(mkfifo.status.success(), "mkfifo: {:?}", mkfifo.stderr);
        // The application accepts file names over its own FIFO, even while a
        // mode consumes normal send-keys. No input/output echo or shell prompt.
        let script = "stty -echo -onlcr; : > \"$1/ready\"; while IFS= read -r token; do cat \"$1/$token\"; : > \"$1/ack-$token\"; done < \"$1/feed\"; exec sleep 600";
        std::fs::write(mux.directory.join("emitter.sh"), script).expect("emitter script");
        mux.command(&[
            "start-server",
            ";",
            "set-option",
            "-g",
            "history-limit",
            &case.history.to_string(),
            ";",
            "new-session",
            "-d",
            "-s",
            "core",
            "-x",
            &case.width.to_string(),
            "-y",
            &case.height.to_string(),
            &format!(
                "/bin/sh {} {}",
                quote(&mux.directory.join("emitter.sh")),
                quote(&mux.directory)
            ),
        ]);
        for args in [
            vec!["set-option", "-g", "status", "off"],
            vec!["set-option", "-g", "window-size", "manual"],
            vec!["set-option", "-g", "mode-keys", case.mode_keys],
            vec!["set-option", "-g", "set-clipboard", "off"],
            vec!["set-option", "-g", "copy-mode-position-format", ""],
            vec![
                "set-option",
                "-g",
                "terminal-features",
                "screen*:sync:clipboard",
            ],
        ] {
            mux.command(&args);
        }
        await_file(&mux.directory.join("ready"));
        let until = Instant::now() + Duration::from_secs(10);
        mux.feed = Some(loop {
            match OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo)
            {
                Ok(feed) => break feed,
                Err(error)
                    if error.raw_os_error() == Some(libc::ENXIO) && Instant::now() < until =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("application FIFO {}: {error}", fifo.display()),
            }
        });
        mux
    }
    fn emit(&mut self, bytes: &[u8]) {
        let token = format!("event-{}", self.next_emit);
        self.next_emit += 1;
        std::fs::write(self.directory.join(&token), bytes).expect("owned arbitrary byte fixture");
        let event = format!("{token}\n");
        let until = Instant::now() + Duration::from_secs(5);
        let mut offset = 0;
        while offset < event.len() {
            match self
                .feed
                .as_mut()
                .expect("application FIFO")
                .write(&event.as_bytes()[offset..])
            {
                Ok(0) => panic!("application FIFO closed"),
                Ok(written) => offset += written,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < until =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("application FIFO event {token}: {error}"),
            }
        }
        await_file(&self.directory.join(format!("ack-{token}")));
        // The ack proves the application wrote, not that the mux consumed its
        // PTY. Wait for actual base capture/cursor/history to settle afterward.
        let until = Instant::now() + Duration::from_secs(5);
        let mut previous = Vec::new();
        let mut stable_since = Instant::now();
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(25));
            let samples = self.batch(&[
                vec!["capture-pane", "-p", "-t", PANE, "-S", "-", "-E", "-"],
                vec![
                    "display-message",
                    "-p",
                    "-t",
                    PANE,
                    "#{cursor_x}|#{cursor_y}|#{history_size}|#{synchronized_output_flag}",
                ],
            ]);
            let now = samples
                .into_iter()
                .flat_map(|sample| sample.stdout)
                .collect::<Vec<_>>();
            if now != previous {
                stable_since = Instant::now();
            } else if stable_since.elapsed() >= Duration::from_millis(100) {
                return;
            }
            previous = now;
        }
        panic!(
            "{} application output failed to settle: {:?}",
            self.binary.display(),
            String::from_utf8_lossy(&previous)
        );
    }
}
impl Drop for Mux {
    fn drop(&mut self) {
        self.feed.take();
        match self.try_run(&["kill-server".to_owned()], Duration::from_secs(5)) {
            Ok(_) => {
                let _ = std::fs::remove_dir_all(&self.directory);
            }
            Err(error) => eprintln!(
                "private socket cleanup failed: {}; retained {}: {error}",
                self.binary.display(),
                self.directory.display()
            ),
        }
    }
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
fn await_file(path: &Path) {
    let until = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < until,
            "application handshake timed out: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Client(rmux_sys::pty::LaunchedProcess);
impl Drop for Client {
    fn drop(&mut self) {
        if matches!(rmux_sys::proc::wait_process(self.0.pid, true), Ok(None)) {
            let _ = rmux_sys::proc::terminate_process(self.0.pid);
            let until = Instant::now() + Duration::from_secs(2);
            while matches!(rmux_sys::proc::wait_process(self.0.pid, true), Ok(None)) {
                if Instant::now() >= until {
                    eprintln!("owned client {:?} did not reap after SIGKILL", self.0.pid);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
impl Client {
    fn attach(mux: &Mux, width: u16, height: u16) -> Self {
        let launch = PreparedLaunch::new(LaunchOptions {
            shell: b"/bin/sh".to_vec(),
            argv: vec![
                mux.binary.as_os_str().as_encoded_bytes().to_vec(),
                b"-S".to_vec(),
                mux.directory
                    .join("socket")
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
                b"-f".to_vec(),
                b"/dev/null".to_vec(),
                b"attach-session".to_vec(),
                b"-t".to_vec(),
                b"core".to_vec(),
            ],
            environment: vec![
                b"TERM=screen-256color".to_vec(),
                b"LC_ALL=C.UTF-8".to_vec(),
                b"PATH=/bin:/usr/bin".to_vec(),
                b"HOME=/tmp".to_vec(),
            ],
            cwd: b"/".to_vec(),
            home: Some(b"/tmp".to_vec()),
            termios: None,
            backspace: 0x7f,
            size: Winsize {
                rows: height,
                cols: width,
                xpixel: 0,
                ypixel: 0,
            },
        })
        .expect("PTY launch preparation");
        let client = Self(launch.launch().expect("attached client"));
        let tty = String::from_utf8(client.0.tty.clone()).expect("client tty name");
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let names = mux.command(&["list-clients", "-F", "#{client_name}"]);
            if names
                .split(|byte| *byte == b'\n')
                .any(|name| name == tty.as_bytes())
            {
                break;
            }
            assert!(
                Instant::now() < until,
                "attached client never registered: {:?}, child {:?}",
                names,
                rmux_sys::proc::wait_process(client.0.pid, true)
            );
        }
        client
    }
    fn input(&self, bytes: &[u8]) {
        let mut offset = 0;
        let until = Instant::now() + Duration::from_secs(5);
        while offset < bytes.len() {
            match rmux_sys::fd::write(self.0.master.as_fd(), &bytes[offset..]) {
                Ok(0) => panic!("closed client PTY"),
                Ok(n) => offset += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < until, "client input timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("client input: {e}"),
            }
        }
    }
    fn drain(&self) -> Vec<u8> {
        self.drain_for(Duration::from_millis(100))
    }
    fn drain_ready(&self) -> Vec<u8> {
        self.drain_for(Duration::ZERO)
    }
    fn drain_for(&self, quiet: Duration) -> Vec<u8> {
        let start = Instant::now();
        let mut last = Instant::now();
        let mut result = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            match rmux_sys::fd::read(self.0.master.as_fd(), &mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    result.extend_from_slice(&buffer[..n]);
                    last = Instant::now();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if last.elapsed() >= quiet {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                Err(e) => panic!("client recorder: {e}"),
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "client output did not become quiet"
            );
        }
        result
    }
}

fn snapshot(bytes: &[u8], width: u16, height: u16) -> Vec<u8> {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(
        u32::from(width),
        u32::from(height),
        0,
        ScreenResetPolicy::default(),
        &mut registry,
    )
    .expect("recorder screen");
    let mut palette = rmux_emu::colour::ColourPalette::new();
    let mut parser = InputCtx::new();
    let mut sink = ScreenOnlySink;
    let mut writer = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    parser.parse(
        &mut writer,
        Some(&mut palette),
        &InputPolicy::screen_only(),
        &mut NullSink,
        bytes,
    );
    writer.finish();
    let mut result = Vec::new();
    for y in 0..u32::from(height) {
        for x in 0..u32::from(width) {
            let cell = screen.grid.get_cell(x, screen.grid.hsize() + y);
            result.extend_from_slice(&[cell.data.size, cell.data.width]);
            result.extend_from_slice(&cell.data.data[..usize::from(cell.data.size)]);
            result.extend_from_slice(&cell.attr.0.to_le_bytes());
            result.extend_from_slice(&cell.fg.0.to_le_bytes());
            result.extend_from_slice(&cell.bg.0.to_le_bytes());
            result.extend_from_slice(&cell.us.0.to_le_bytes());
        }
    }
    result.extend_from_slice(&screen.cx.to_le_bytes());
    result.extend_from_slice(&screen.cy.to_le_bytes());
    result.push(u8::from(screen.mode.contains(ScreenMode::CURSOR)));
    result
}
fn clipboard_events(wire: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut synchronized = false;
    let mut offset = 0;
    while offset < wire.len() {
        let rest = &wire[offset..];
        if rest.starts_with(b"\x1b[?2026h") {
            synchronized = true;
            offset += 8;
        } else if rest.starts_with(b"\x1b[?2026l") {
            synchronized = false;
            offset += 8;
        } else if rest.starts_with(b"\x1b]52;") {
            assert!(
                !synchronized,
                "clipboard must flush pending synchronized redraw"
            );
            let bel = rest.iter().position(|byte| *byte == 7).map(|end| end + 1);
            let st = rest
                .windows(2)
                .position(|bytes| bytes == b"\x1b\\")
                .map(|end| end + 2);
            let end = match (bel, st) {
                (Some(bel), Some(st)) => bel.min(st),
                (Some(end), None) | (None, Some(end)) => end,
                (None, None) => panic!("complete OSC52 clipboard event"),
            };
            result.extend_from_slice(&rest[..end]);
            offset += end;
        } else {
            offset += 1;
        }
    }
    result
}

#[test]
fn clipboard_wire_records_payload_after_sync_barrier_not_redraw_packets() {
    let wire = b"\x1b[?2026hredraw\x1b[?2026l\x1b]52;c;YWJj\x1b\\cursor\x1b]52;c;ZA==\x07";
    assert_eq!(
        clipboard_events(wire),
        b"\x1b]52;c;YWJj\x1b\\\x1b]52;c;ZA==\x07"
    );
    assert!(
        std::panic::catch_unwind(|| clipboard_events(b"\x1b[?2026h\x1b]52;c;YWJj\x07")).is_err()
    );
}

fn record(outputs: &mut Vec<Observation>, label: String, output: Output) {
    outputs.push(Observation {
        label,
        success: output.status.success(),
        stdout: output.stdout.into(),
        stderr: output.stderr.into(),
    });
}
fn bytes(outputs: &mut Vec<Observation>, label: String, data: Vec<u8>) {
    outputs.push(Observation {
        label,
        success: true,
        stdout: data.into(),
        stderr: Vec::new().into(),
    });
}
fn observe(mux: &Mux, outputs: &mut Vec<Observation>, label: &str) {
    let template = FORMATS
        .iter()
        .map(|key| format!("{key}=#{{{key}}}"))
        .collect::<Vec<_>>()
        .join("\x1f");
    let observations = [
        (
            "all24-formats",
            vec!["display-message", "-p", "-t", PANE, &template],
        ),
        (
            "pane-state",
            vec![
                "display-message",
                "-p",
                "-t",
                PANE,
                "#{pane_mode}|#{pane_in_mode}|#{pane_width}|#{pane_height}|#{cursor_x}|#{cursor_y}|#{history_size}|#{history_limit}|#{synchronized_output_flag}|#{pane_active}",
            ],
        ),
        (
            "backing",
            vec![
                "capture-pane",
                "-p",
                "-M",
                "-t",
                PANE,
                "-S",
                "-",
                "-E",
                "-",
                "-T",
            ],
        ),
        (
            "backing-styled",
            vec![
                "capture-pane",
                "-p",
                "-M",
                "-e",
                "-t",
                PANE,
                "-S",
                "-",
                "-E",
                "-",
                "-T",
            ],
        ),
        (
            "backing-logical",
            vec![
                "capture-pane",
                "-p",
                "-M",
                "-J",
                "-t",
                PANE,
                "-S",
                "-",
                "-E",
                "-",
            ],
        ),
    ];
    let replies = mux.batch(
        &observations
            .iter()
            .map(|(_, args)| args.clone())
            .collect::<Vec<_>>(),
    );
    let mut replies = replies.into_iter();
    let mut out = replies.next().unwrap();
    // A top-row timestamp is sampled in different real servers. Normalize only
    // that numeric field, keeping whether it is absent, zero, or nonzero.
    if out.stdout.starts_with(b"top_line_time=") {
        let end = out
            .stdout
            .iter()
            .position(|b| *b == 0x1f)
            .expect("format delimiter");
        let value = &out.stdout[b"top_line_time=".len()..end];
        if !value.is_empty() && value != b"0" {
            assert!(
                value.iter().all(u8::is_ascii_digit),
                "invalid wall timestamp {value:?}"
            );
            out.stdout
                .splice(b"top_line_time=".len()..end, b"<wall-time>".iter().copied());
        }
    }
    record(outputs, format!("{label}/all24-formats"), out);
    for ((name, _), output) in observations.iter().skip(1).zip(replies) {
        record(outputs, format!("{label}/{name}"), output);
    }
}
fn run_case(binary: &Path, case: &Case, role: &str) -> Vec<Observation> {
    eprintln!("G18 {role} case {}: {} steps", case.name, case.steps.len());
    let mut mux = Mux::new(binary, case, role);
    let client = case
        .attached
        .then(|| Client::attach(&mux, case.width, case.height));
    let mut wire = Vec::new();
    let mut outputs = Vec::new();
    if let Some(client) = &client {
        wire.extend(client.drain());
        // A nonempty initialization stream can still accompany a broken blank
        // renderer. Require an application marker in the parsed client screen.
        mux.emit(b"\x1b[2J\x1b[HR");
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            wire.extend(client.drain());
            if snapshot(&wire, case.width, case.height).starts_with(&[1, 1, b'R']) {
                break;
            }
            assert!(
                Instant::now() < until,
                "{role} {}: client never rendered application readiness marker; terminal {:?}; base {:?}",
                case.name,
                snapshot(&wire, case.width, case.height).get(..20),
                mux.command(&["capture-pane", "-p", "-t", PANE])
            );
        }
        mux.emit(b"\x1b[2J\x1b[H");
        wire.extend(client.drain());
    }
    for (index, step) in case.steps.iter().enumerate() {
        let label = format!("{}:{index}", case.name);
        if index % 50 == 0 || matches!(step, Step::Check(_)) {
            eprintln!("G18 {role} {label}/{}", case.steps.len());
        }
        match step {
            Step::Command(args) => {
                let output = mux.run(args);
                assert!(
                    output.status.success(),
                    "{role} {} {args:?}: {}",
                    case.name,
                    String::from_utf8_lossy(&output.stderr)
                );
                record(&mut outputs, format!("{label}/command"), output);
            }
            Step::Emit(data) => mux.emit(data),
            Step::Wait(duration) => std::thread::sleep(*duration),
            Step::Input(data) => client
                .as_ref()
                .expect("PTY input requires attached client")
                .input(data),
            Step::MouseScrollbar => {
                let fields = mux.command(&[
                    "display-message",
                    "-p",
                    "-t",
                    "core:0.1",
                    "#{pane_left} #{pane_top} #{pane_width}",
                ]);
                let fields = String::from_utf8(fields)
                    .expect("mouse geometry")
                    .split_whitespace()
                    .map(|n| n.parse::<u32>().expect("mouse geometry number"))
                    .collect::<Vec<_>>();
                assert_eq!(fields.len(), 3);
                let x = fields[0] + fields[2] + 1;
                let y = fields[1] + 1;
                let client = client.as_ref().expect("mouse client");
                for sequence in [
                    format!("\x1b[<0;{x};{y}M"),
                    format!("\x1b[<32;{x};{}M", y + 1),
                    format!("\x1b[<0;{x};{}m", y + 1),
                ] {
                    client.input(sequence.as_bytes());
                    wire.extend(client.drain());
                }
            }
            Step::RequireFormat(target, format, expected) => {
                let output = mux.command(&["display-message", "-p", "-t", target, format]);
                assert_eq!(
                    output,
                    format!("{expected}\n").as_bytes(),
                    "{role} {}: acceptance precondition {format}",
                    case.name
                );
                bytes(
                    &mut outputs,
                    format!("{label}/acceptance-precondition"),
                    output,
                );
            }
            Step::Buffer(name) => {
                record(
                    &mut outputs,
                    format!("{label}/{name}/buffer-size"),
                    mux.run(&[
                        "list-buffers".to_owned(),
                        "-F".to_owned(),
                        "#{buffer_size}".to_owned(),
                    ]),
                );
                let out = mux.run(&["save-buffer".to_owned(), "-".to_owned()]);
                assert!(
                    out.status.success(),
                    "{role}: copied buffer missing at {label}/{name}"
                );
                record(&mut outputs, format!("{label}/{name}/exact-buffer"), out);
            }
            Step::Check(name) => {
                observe(&mux, &mut outputs, &format!("{label}/{name}"));
                if let Some(client) = &client {
                    let chunk = client.drain();
                    wire.extend_from_slice(&chunk);
                    bytes(
                        &mut outputs,
                        format!("{label}/{name}/terminal-snapshot"),
                        snapshot(&wire, case.width, case.height),
                    );
                }
                continue;
            }
        }
        if let Some(client) = &client {
            let chunk = client.drain_ready();
            wire.extend_from_slice(&chunk);
        }
    }
    if let Some(client) = &client {
        wire.extend(client.drain());
        bytes(
            &mut outputs,
            format!("{}/clipboard-wire-events", case.name),
            clipboard_events(&wire),
        );
    }
    outputs
}
fn binaries() -> Option<(PathBuf, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let oracle = std::env::var_os("RMUX_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("oracle/bin/tmux"));
    let binary = std::env::var_os("RMUX_COPY_BINARY")
        .map(PathBuf::from)
        .or_else(|| {
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("target"));
            [target.join("debug/rmux"), root.join("target/debug/rmux")]
                .into_iter()
                .find(|p| p.is_file())
        });
    let Some(binary) = binary.filter(|p| p.is_file()) else {
        eprintln!("SKIP G18 core oracle: built rmux missing; set RMUX_COPY_BINARY");
        return None;
    };
    if !oracle.is_file() {
        eprintln!(
            "SKIP G18 core oracle: pinned oracle missing at {}; set RMUX_ORACLE",
            oracle.display()
        );
        return None;
    }
    Some((oracle, binary))
}
fn differential(cases: Vec<Case>) {
    let _slot = CoreSlot::acquire();
    let Some((oracle, binary)) = binaries() else {
        return;
    };
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        // An oracle setup failure panics as an oracle error, not an rmux parity
        // assertion, and is never converted into a fabricated expected output.
        let expected = run_case(&oracle, case, "oracle");
        let mut actual = run_case(&binary, case, "rmux");
        if i == 0 && std::env::var("RMUX_COPY_CORE_ORACLE_MUTATE").as_deref() == Ok("1") {
            let observed = actual
                .iter_mut()
                .find(|o| !o.stdout.bytes.is_empty())
                .expect("real observation");
            observed.stdout.bytes[0] ^= 1;
        }
        if actual.len() != expected.len() {
            failures.push(format!(
                "{}: observation count {} != {}",
                case.name,
                actual.len(),
                expected.len()
            ));
        }
        for (actual, expected) in actual.iter().zip(&expected) {
            if actual != expected {
                failures.push(format!(
                    "{}: oracle {expected:?}\nrmux {actual:?}",
                    case.name
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "G18 core differential failures:\n{}",
        failures.join("\n")
    );
}

fn numbered_lines(count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in 0..count {
        bytes.extend_from_slice(format!("L{row:04}-abcdefghij  \r\n").as_bytes());
    }
    bytes
}

#[test]
fn gutters_five_modes_narrow_widths_and_history_digits() {
    let mut cases = Vec::new();
    for width in [1, 2, 4, 8, 80] {
        for mode in ["off", "default", "absolute", "relative", "hybrid"] {
            let mut case = Case::new(format!("gutter-{mode}-{width}"), width, 8);
            case.emit(b"123456789ABCDEFG\r\nshort\r\n".to_vec());
            case.command(&["set-option", "-g", "copy-mode-line-numbers", mode]);
            case.enter();
            case.copy(&["history-top"]);
            case.check("initial-position");
            case.repeat(3, &["cursor-right"]);
            case.check("gutter-cursor-clipping");
            case.copy(&["line-numbers-off"]);
            case.check("forced-off");
            case.copy(&["line-numbers-on"]);
            case.check("forced-on-over-option-off");
            case.copy(&["line-numbers-toggle"]);
            case.check("toggle");
            case.copy(&["cancel"]);
            case.command(&["set-option", "-g", "mouse", "on"]);
            case.steps.push(Step::Input(b"\x1b[<64;1;2M".to_vec()));
            case.wait();
            case.check("mouse-entry-hides-gutter");
            case.command(&["copy-mode", "-q", "-t", PANE]);
            cases.push(case);
        }
    }
    // At width 80 every emitted line is one physical row. Cross the exact
    // total-row thresholds used by gutter digits, not just decimal labels.
    for mode in ["off", "default", "absolute", "relative", "hybrid"] {
        let mut case = Case::new(format!("gutter-digits-{mode}"), 80, 8);
        case.command(&["set-option", "-g", "copy-mode-line-numbers", mode]);
        case.emit(b"\x1b[8;1H".to_vec());
        case.enter();
        let mut total = 8usize;
        for target in [8usize, 9, 10, 98, 99, 100, 998, 999, 1000] {
            if target > total {
                case.emit(numbered_lines(target - total));
            }
            total = target;
            case.copy(&["refresh-now"]);
            case.copy(&["history-top"]);
            case.check(format!("total-{target}-top"));
            case.copy(&["history-bottom"]);
            case.check(format!("total-{target}-bottom"));
        }
        cases.push(case);
    }
    differential(cases);
}

#[test]
fn extraction_unicode_tabs_acs_wrap_rectangles_and_stopped_endpoints() {
    let text = b"A\tB  \r\n\x1b(0lqqk\x1b(B\r\ne\xcc\x81-\xe7\x95\x8c-Z  \r\n0123456789abcdef0123456789\r\nHARD\r\nlast  ".to_vec();
    let mut cases = Vec::new();
    for keys in ["emacs", "vi"] {
        for rectangle in [false, true] {
            for reverse in [false, true] {
                for side in [1usize, 6] {
                    let mut case = Case::new(
                        format!("extract-{keys}-rect{rectangle}-reverse{reverse}-side{side}"),
                        12,
                        8,
                    );
                    case.mode_keys = keys;
                    case.emit(text.clone());
                    case.enter();
                    case.copy(&["history-top"]);
                    if reverse {
                        case.repeat(5, &["cursor-down"]);
                    }
                    case.copy(&["start-of-line"]);
                    case.repeat(side, &["cursor-right"]);
                    if rectangle {
                        case.copy(&["rectangle-toggle"]);
                    }
                    case.copy(&["begin-selection"]);
                    case.repeat(5, &[if reverse { "cursor-up" } else { "cursor-down" }]);
                    case.copy(&["start-of-line"]);
                    case.repeat(if side == 1 { 8 } else { 1 }, &["cursor-right"]);
                    case.check("selection");
                    case.buffer("exact-linear-or-rectangle");
                    case.copy(&["stop-selection"]);
                    case.copy(&["history-bottom"]);
                    case.copy(&["history-top"]);
                    case.buffer("stopped-endpoints");
                    case.check("stopped");
                    cases.push(case);
                }
            }
        }
        for repeats in [1usize, 2, 5, 6] {
            let mut case = Case::new(format!("buried-endpoints-{keys}-exchange{repeats}"), 40, 8);
            case.mode_keys = keys;
            case.emit(numbered_lines(80));
            case.enter();
            case.copy(&["history-top"]);
            case.repeat(10, &["cursor-down"]);
            case.copy(&["start-of-line"]);
            case.copy(&["begin-selection"]);
            case.repeat(2, &["cursor-down"]);
            case.buffer("before-freeze");
            case.copy(&["stop-selection"]);
            for command in [
                "history-bottom",
                "history-top",
                "scroll-middle",
                "scroll-bottom",
                "scroll-top",
                "recentre-top-bottom",
            ] {
                case.copy(&[command]);
                case.check(command);
                case.buffer(format!("frozen-after-{command}"));
            }
            case.repeat(repeats, &["other-end"]);
            case.copy(&["cursor-down"]);
            case.buffer("exchange-then-extend");
            case.check("extended");
            cases.push(case);
        }
        for end in ["end-of-line", "history-bottom"] {
            let mut case = Case::new(format!("final-newline-{keys}-{end}"), 12, 4);
            case.mode_keys = keys;
            case.emit(b"trailing   \r\nsoftwrapped-0123456789\r\nfinal   ".to_vec());
            case.enter();
            case.copy(&["history-top"]);
            case.copy(&["begin-selection"]);
            case.copy(&[end]);
            if end == "history-bottom" {
                case.copy(&["end-of-line"]);
            }
            case.buffer("exact-final-newline-rule");
            case.check("final");
            cases.push(case);
        }
    }
    differential(cases);
}

#[test]
fn search_posix_smartcase_directions_wrap_and_unicode_cell_mapping() {
    let mut cases = Vec::new();
    let fixture = b"alpha ALPHA Alpha aLpHa\r\nA\tB\r\nwide-\xe7\x95\x8c-wide\r\ne\xcc\x81 repeated e\xcc\x81\r\nstart end\r\n[ literal ^ $\r\nalpha ALPHA\r\n".to_vec();
    for keys in ["emacs", "vi"] {
        for wrap in ["on", "off"] {
            for direction in ["forward", "backward"] {
                for regex in [false, true] {
                    let mut case = Case::new(
                        format!("search-{keys}-{wrap}-{direction}-regex{regex}"),
                        24,
                        6,
                    );
                    case.mode_keys = keys;
                    case.emit(fixture.clone());
                    case.command(&["set-option", "-g", "wrap-search", wrap]);
                    case.enter();
                    let command = format!("search-{direction}{}", if regex { "" } else { "-text" });
                    for term in [
                        "alpha",
                        "ALPHA",
                        "aLpHa",
                        "A\tB",
                        "界",
                        "e\u{301}",
                        "^",
                        "$",
                        "^start",
                        "end$",
                        "[",
                        "(unclosed",
                    ] {
                        case.copy(&[if direction == "forward" {
                            "history-top"
                        } else {
                            "history-bottom"
                        }]);
                        case.copy(&[&command, "--", term]);
                        case.check(format!("term-{term:?}"));
                        case.repeat(2, &["search-again"]);
                        case.check(format!("repeat-{term:?}"));
                        case.copy(&["search-reverse"]);
                        case.check(format!("reverse-{term:?}"));
                    }
                    // Invalid compiled expressions must not leave resize-unsafe marks.
                    case.command(&["resize-window", "-t", "core:0", "-x", "17", "-y", "7"]);
                    case.check("resize-after-invalid");
                    cases.push(case);
                }
            }
        }
    }
    differential(cases);
}

#[test]
fn search_overlap_softwrap_generation_rollover_empty_and_history_entry() {
    let mut cases = Vec::new();
    for direction in ["forward", "backward"] {
        let mut case = Case::new(format!("search-generation-{direction}"), 12, 6);
        let mut fixture = b"ABOVE-VIEWPORT\r\nababababaaaaaa\r\nwrapwrapwrapwrapwrap\r\n".to_vec();
        for _ in 0..300 {
            fixture.extend_from_slice(b"a aa aaa\r\n");
        }
        case.emit(fixture);
        case.enter();
        let command = format!("search-{direction}");
        for term in ["aba", "aa", "wrapwrap", "a", "^", "$", "a*", ".*", "^$"] {
            case.copy(&["history-top"]);
            case.copy(&[&command, "--", term]);
            case.check(format!("overlap-adjacent-empty-{term}"));
            case.repeat(5, &["search-again"]);
            case.check(format!("repeated-{term}"));
        }
        case.copy(&["history-bottom"]);
        case.copy(&["search-backward-text", "ABOVE-VIEWPORT"]);
        case.check("history-match-enters-viewport");
        // Each search rebuilds generation-marked runs; exceed the byte generation.
        for generation in 0..270 {
            case.copy(&[&command, if generation % 2 == 0 { "aa" } else { "a" }]);
            if [0, 253, 254, 255, 256, 269].contains(&generation) {
                case.check(format!("generation-{generation}"));
            }
        }
        case.copy(&["history-top"]);
        case.copy(&["search-forward-text", "wrapwrap"]);
        case.copy(&["scroll-down"]);
        case.check("match-start-above-current-view");
        cases.push(case);
    }
    differential(cases);
}

#[test]
fn incremental_search_terms_prefixes_failures_and_resize_origin() {
    let mut cases = Vec::new();
    for direction in ["forward", "backward"] {
        for wrap in ["on", "off"] {
            let mut case = Case::new(format!("incremental-{direction}-{wrap}"), 18, 6);
            case.emit(b"alpha beta alpha\r\nbeta alpha beta\r\n0123456789alpha0123456789\r\nlast beta\r\n".to_vec());
            case.command(&["set-option", "-g", "wrap-search", wrap]);
            case.enter();
            case.copy(&["history-top"]);
            case.repeat(3, &["cursor-right"]);
            case.check("origin");
            let command = format!("search-{direction}-incremental");
            for term in [
                "=alpha", "+alpha", "-alpha", "=beta", "+beta", "-beta", "=absent", "=", "+", "-",
                "=alpha",
            ] {
                case.copy(&[&command, "--", term]);
                case.check(format!("argument-{term}"));
            }
            case.command(&["resize-window", "-t", "core:0", "-x", "9", "-y", "7"]);
            case.check("resized-origin-invalidated");
            case.copy(&[&command, "=beta"]);
            case.check("new-origin-after-resize");
            case.copy(&[&command, "+alpha"]);
            case.check("changed-term-after-resize");
            cases.push(case);
        }
    }
    differential(cases);
}

#[test]
fn all_twenty_four_formats_search_selection_clear_stopped_and_view() {
    let mut cases = Vec::new();
    for keys in ["emacs", "vi"] {
        let mut case = Case::new(format!("all24-{keys}"), 32, 6);
        case.mode_keys = keys;
        case.emit(b"word \x1b]8;;https://example.test/copy\x1b\\linked\x1b]8;;\x1b\\ tail\r\nword second\r\nthird\r\n".to_vec());
        case.check("outside-mode");
        case.enter();
        case.copy(&["history-top"]);
        case.check("before-search");
        case.copy(&["search-forward-text", "word"]);
        case.check("during-search");
        case.copy(&["clear-selection"]);
        case.check("after-clear-selection-policy");
        case.copy(&["search-forward-text", "word"]);
        case.copy(&["begin-selection"]);
        case.check("after-unconditional-mark-clear");
        case.copy(&["clear-selection"]);
        case.copy(&["begin-selection"]);
        case.check("equal-selection-endpoints");
        case.repeat(4, &["cursor-right"]);
        case.check("active-selection");
        case.copy(&["stop-selection"]);
        case.check("stopped-selection");
        case.copy(&["clear-selection"]);
        case.copy(&["history-top"]);
        case.copy(&["start-of-line"]);
        case.repeat(5, &["cursor-right"]);
        case.check("hyperlink-cursor");
        case.copy(&["line-numbers-on"]);
        case.check("gutter-format-with-hyperlink");
        case.copy(&["cancel"]);
        case.command(&[
            "run-shell",
            "-t",
            PANE,
            "printf 'view word\\nsecond word\\n'",
        ]);
        case.check("output-view");
        case.copy(&["history-top"]);
        case.copy(&["search-forward-text", "word"]);
        case.check("output-view-search");
        cases.push(case);
    }
    differential(cases);
}

#[test]
fn parsed_output_view_append_anchoring_crlf_capture_and_cursor() {
    let mut case = Case::new("view-parsed-scrolled-append", 18, 6);
    case.command(&[
        "run-shell",
        "-t",
        PANE,
        r#"i=0; while [ $i -lt 30 ]; do printf '\033[31mV%02d\033[0m\r\n' "$i"; i=$((i+1)); done"#,
    ]);
    case.check("styled-view-created");
    case.copy(&["history-top"]);
    case.repeat(2, &["scroll-down"]);
    case.check("scrolled-away-from-bottom");
    case.command(&[
        "run-shell",
        "-t",
        PANE,
        r"printf '\033[32mAPPEND\033[0m\r\nCR\rreplace\nTAB\t界e\314\201\nunterminated'",
    ]);
    case.check("parsed-append-preserves-top-anchor");
    case.copy(&["history-top"]);
    case.copy(&["begin-selection"]);
    case.copy(&["history-bottom"]);
    case.copy(&["end-of-line"]);
    case.buffer("view-crlf-exact-bytes");
    case.check("view-bottom-cursor-history");
    // A command issued by the attached client routes cmdq_print into a view.
    // Plain parse=false append has no public oracle CLI caller; its real
    // Server-boundary differential belongs to the parent's view unit fixture.
    case.command(&["copy-mode", "-q", "-t", PANE]);
    case.command(&[
        "bind-key",
        "-n",
        "F12",
        "display-message",
        "-p",
        "plain word",
    ]);
    case.steps.push(Step::Input(b"\x1b[24~".to_vec()));
    case.wait();
    case.check("attached-command-output-view");
    differential(vec![case]);
}

#[test]
fn refresh_counter_paths_follow_and_anchor_geometry_history_generation() {
    let mut cases = Vec::new();
    for follow in [false, true] {
        let mut case = Case::new(format!("refresh-counters-follow{follow}"), 24, 6);
        case.history = 24;
        case.emit(numbered_lines(20));
        case.enter();
        case.copy(&[if follow {
            "history-bottom"
        } else {
            "history-top"
        }]);
        if !follow {
            case.repeat(2, &["scroll-down"]);
        }
        case.check("before-refresh");
        for (name, data) in [
            ("mutation-without-scroll", b"\x1b[2;1HMUTATION".to_vec()),
            ("scroll-addition", b"\x1b[6;1H\r\nNEW-ROW\r\n".to_vec()),
            ("history-collection", numbered_lines(80)),
            ("generation-reset", b"\x1b[3J\x1b[HGENERATION".to_vec()),
        ] {
            case.emit(data);
            case.check(format!("{name}-stale-before-manual"));
            case.copy(&["refresh-now"]);
            case.check(format!("{name}-refreshed"));
        }
        case.command(&["resize-window", "-t", "core:0", "-x", "13", "-y", "7"]);
        case.copy(&["refresh-now"]);
        case.check("geometry-change");
        case.command(&["set-option", "-g", "history-limit", "10"]);
        case.command(&["clear-history", "-t", PANE]);
        // clear-history resets every mode (cmd-capture-pane.c:424-426).
        case.enter();
        case.emit(numbered_lines(40));
        case.copy(&["refresh-now"]);
        case.check("history-limit-change-and-collection");
        case.copy(&["refresh-on"]);
        case.check("automatic-active-format");
        case.emit(b"\r\nAUTO-TIMER-DELIVERY\r\n".to_vec());
        case.wait();
        case.check("actual-automatic-delivery");
        cases.push(case);
    }
    differential(cases);
}

#[test]
fn automatic_refresh_selection_pauses_manual_clear_buried_source_and_view() {
    let mut cases = Vec::new();
    for selection in ["active", "stopped", "cleared"] {
        let mut case = Case::new(format!("refresh-selection-{selection}"), 28, 6);
        case.emit(numbered_lines(12));
        case.enter();
        case.copy(&["history-top"]);
        case.copy(&["refresh-on"]);
        case.copy(&["begin-selection"]);
        case.repeat(3, &["cursor-right"]);
        if selection != "active" {
            case.copy(&["stop-selection"]);
        }
        if selection == "cleared" {
            case.copy(&["clear-selection"]);
        }
        case.check("active-format-before-output");
        case.emit(b"\x1b[HPAUSE-PROBE\r\n".to_vec());
        case.wait();
        case.check("timer-selection-pause-or-delivery");
        case.copy(&["refresh-now"]);
        case.check("manual-refresh-clears-selection");
        case.emit(b"\x1b[HAFTER-MANUAL\r\n".to_vec());
        case.wait();
        case.check("timer-after-manual");
        case.copy(&["refresh-off"]);
        case.emit(b"\x1b[HOFF-PROBE\r\n".to_vec());
        case.wait();
        case.check("timer-disabled-no-delivery");
        cases.push(case);
    }
    let mut case = Case::new("refresh-buried-mode", 28, 6);
    case.emit(numbered_lines(20));
    case.enter();
    case.copy(&["refresh-on"]);
    case.command(&["choose-tree", "-t", PANE]);
    case.emit(b"\x1b[HBURIED-PROBE\r\n".to_vec());
    case.wait();
    case.check("copy-mode-buried-under-tree");
    case.command(&["send-keys", "-t", PANE, "q"]);
    case.check("copy-unburied-before-new-output");
    case.emit(b"\x1b[HUNBURIED-PROBE\r\n".to_vec());
    case.wait();
    case.check("delivery-after-unbury");
    cases.push(case);
    let mut case = Case::new("refresh-different-source", 28, 6);
    case.emit(numbered_lines(20));
    case.command(&["new-window", "-d", "-t", "core:1", "sleep 600"]);
    case.command(&["copy-mode", "-s", PANE, "-t", "core:1.0"]);
    case.command(&["send-keys", "-t", "core:1.0", "-X", "refresh-on"]);
    case.command(&["select-window", "-t", "core:1"]);
    case.emit(b"\x1b[HDIFFERENT-SOURCE\r\n".to_vec());
    case.wait();
    for args in [
        vec![
            "display-message",
            "-p",
            "-t",
            "core:1.0",
            "#{refresh_active}|#{copy_cursor_x}|#{copy_cursor_y}|#{scroll_position}",
        ],
        vec![
            "capture-pane",
            "-p",
            "-M",
            "-t",
            "core:1.0",
            "-S",
            "-",
            "-E",
            "-",
        ],
        vec!["send-keys", "-t", "core:1.0", "-X", "refresh-now"],
        vec![
            "capture-pane",
            "-p",
            "-M",
            "-t",
            "core:1.0",
            "-S",
            "-",
            "-E",
            "-",
        ],
    ] {
        case.command(&args);
    }
    case.check("different-source-timer-rejected");
    cases.push(case);
    let mut case = Case::new("refresh-view-rejected", 28, 6);
    case.command(&["run-shell", "-t", PANE, "printf 'VIEW-STATIC\\n'"]);
    case.copy(&["refresh-on"]);
    case.check("view-active-format-remains-off");
    case.emit(b"\x1b[HBASE-CHANGED-BEHIND-VIEW\r\n".to_vec());
    case.wait();
    case.check("view-no-timer-delivery");
    case.copy(&["refresh-now"]);
    case.check("view-manual-rejected");
    cases.push(case);
    differential(cases);
}

#[test]
fn pty_position_tail_alignment_gutter_overflow_clipboard_scrollbar_and_sync() {
    let mut cases = Vec::new();
    for align in ["left", "centre", "right"] {
        let mut case = Case::new(format!("pty-indicator-{align}"), 40, 8);
        case.emit(b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ1234\r\n".to_vec());
        case.emit(numbered_lines(90));
        case.enter();
        case.copy(&["history-top"]);
        for text in ["[23/100-LONGTAIL]", "[1/100]", ""] {
            case.command(&[
                "set-option",
                "-g",
                "copy-mode-position-format",
                &format!("#[align={align}]{text}"),
            ]);
            case.copy(&["cursor-right"]);
            case.check(format!("indicator-{text:?}"));
        }
        case.command(&["set-option", "-g", "copy-mode-line-numbers", "absolute"]);
        case.copy(&["line-numbers-on"]);
        case.repeat(39, &["cursor-right"]);
        case.check("hidden-cursor-overflow-marker");
        case.command(&["set-option", "-g", "set-clipboard", "external"]);
        case.copy(&["start-of-line"]);
        case.copy(&["begin-selection"]);
        case.repeat(4, &["cursor-right"]);
        // One queue transaction leaves a pending redraw when the copy emits
        // OSC52; capture the actual client event ordering, not a fake sink.
        case.command(&[
            "send-keys",
            "-t",
            PANE,
            "-X",
            "cursor-right",
            ";",
            "send-keys",
            "-t",
            PANE,
            "-X",
            "copy-selection-no-clear",
        ]);
        case.steps
            .push(Step::Buffer("clipboard-pending-redraw".to_owned()));
        case.check("clipboard-pending-redraw-wire");
        cases.push(case);
    }
    let mut case = Case::new("pty-scrollbar-overlay-active-switch", 40, 10);
    case.command(&["set-option", "-g", "mouse", "on"]);
    case.command(&["set-option", "-g", "pane-scrollbars", "on"]);
    case.command(&["set-option", "-g", "pane-scrollbars-position", "right"]);
    case.command(&["set-option", "-g", "pane-scrollbars-style", "width=1,pad=0"]);
    case.command(&["set-option", "-g", "window-active-style", "bg=red"]);
    case.command(&["set-option", "-g", "window-style", "bg=green"]);
    case.command(&[
        "split-window",
        "-h",
        "-d",
        "-t",
        PANE,
        "printf MARK1; exec sleep 600",
    ]);
    case.wait();
    case.command(&["copy-mode", "-t", "core:0.1"]);
    case.check("inactive-scrollbar-overlay");
    case.steps.push(Step::MouseScrollbar);
    case.command(&[
        "display-message",
        "-p",
        "-t",
        "core:0.1",
        "#{pane_active}|#{copy_cursor_x}|#{copy_cursor_y}",
    ]);
    case.steps
        .push(Step::RequireFormat("core:0.1", "#{pane_active}", "1"));
    case.check("scrollbar-drag-switches-body-style");
    cases.push(case);
    let mut case = Case::new("pty-base-sync-copy-cursor", 40, 8);
    case.emit(b"copy cursor".to_vec());
    case.enter();
    case.check("before-sync-cursor");
    // DECSET remains open while the copy cursor moves. Re-arm between steps
    // rather than depending on the base-screen one-second watchdog duration.
    case.emit(b"\x1b[?2026h".to_vec());
    case.copy(&["cursor-left"]);
    case.steps.push(Step::RequireFormat(
        PANE,
        "#{synchronized_output_flag}",
        "1",
    ));
    case.check("sync-open-first-copy-cursor-movement");
    case.emit(b"\x1b[?2026h".to_vec());
    case.copy(&["cursor-left"]);
    case.steps.push(Step::RequireFormat(
        PANE,
        "#{synchronized_output_flag}",
        "1",
    ));
    case.check("sync-open-second-copy-cursor-movement");
    case.emit(b"\x1b[?2026l".to_vec());
    case.check("sync-closed");
    cases.push(case);
    differential(cases);
}
