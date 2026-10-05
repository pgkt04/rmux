// Ported from tmux window-copy.c, cmd-send-keys.c @ 8f25579c
//! Every command row is exercised through the real CLI in both key modes.
//! Prefixes 1, 2 and 5 share dispatch probes; no build is performed here.
use rmux_emu::colour::ColourPalette;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::input::{InputCtx, InputPolicy, NullSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_server::modes::copy::commands::{COMMAND_TABLE, CopyCommandSpec};
use rmux_sys::pty::{LaunchOptions, LaunchedProcess, PreparedLaunch, Winsize};
use std::fs;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WIDTH: u16 = 40;
const HEIGHT: u16 = 10;
const TARGET: &str = "copy:1.0";
const FORMAT_KEYS: &[&str] = &[
    "pane_in_mode",
    "pane_mode",
    "pane_width",
    "pane_height",
    "history_size",
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct Reply {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl Reply {
    fn success(&self) -> bool {
        self.code == Some(0)
    }
}

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Mux {
    binary: PathBuf,
    directory: PathBuf,
    caller: Option<String>,
}
impl Mux {
    fn socket(&self) -> PathBuf {
        self.directory.join("socket")
    }
    fn try_run(&self, args: &[&str]) -> Result<Reply, String> {
        let stdout_path = self.directory.join("command.stdout");
        let stderr_path = self.directory.join("command.stderr");
        let stdout = fs::File::create(&stdout_path).map_err(|error| error.to_string())?;
        let stderr = fs::File::create(&stderr_path).map_err(|error| error.to_string())?;
        // Regular files avoid inherited pipe descriptors holding output() open.
        let mut child = Command::new(&self.binary)
            .args(["-u", "-f", "/dev/null", "-S"])
            .arg(self.socket())
            .args(args)
            .env("TERM", "screen")
            .env("LC_ALL", "C.UTF-8")
            .env_remove("TMUX")
            .env_remove("RMUX")
            .env_remove("TMUX_PANE")
            .env_remove("RMUX_PANE")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stdout = fs::read(&stdout_path).unwrap_or_default();
                    let stderr = fs::read(&stderr_path).unwrap_or_default();
                    return Err(format!(
                        "CLI deadline 5s or wait error: {result:?}; stdout={stdout:?}; stderr={stderr:?}"
                    ));
                }
            }
        };
        Ok(Reply {
            code: status.code(),
            stdout: fs::read(&stdout_path).map_err(|error| error.to_string())?,
            stderr: fs::read(&stderr_path).map_err(|error| error.to_string())?,
        })
    }
    fn run(&self, args: &[&str]) -> Reply {
        self.try_run(args).unwrap_or_else(|error| {
            panic!(
                "{} socket={} {args:?}: {error}",
                self.binary.display(),
                self.socket().display()
            )
        })
    }
    fn required(&self, args: &[&str]) -> Reply {
        let reply = self.run(args);
        assert!(
            reply.success(),
            "fixture {} {args:?}: {reply:?}",
            self.binary.display()
        );
        reply
    }
    fn mode(&self, command: &[&str]) {
        let mut args = vec!["send-keys", "-t", TARGET, "-X"];
        if let Some(caller) = &self.caller {
            args.extend(["-c", caller.as_str()]);
        }
        args.extend_from_slice(command);
        self.required(&args);
    }
    fn modes(&self, commands: &[&[&str]]) {
        let mut args = Vec::new();
        for command in commands {
            if !args.is_empty() {
                args.push(";");
            }
            args.extend(["send-keys", "-t", TARGET, "-X"]);
            if let Some(caller) = &self.caller {
                args.extend(["-c", caller.as_str()]);
            }
            args.extend_from_slice(command);
        }
        self.required(&args);
    }
    fn batch(&self, commands: &[Vec<&str>]) -> Vec<Reply> {
        const BOUNDARY: &str = "\x1eCOPY-OBSERVATION\x1e";
        let mut args = Vec::new();
        for command in commands {
            if !args.is_empty() {
                args.push(";");
            }
            args.extend_from_slice(command);
            args.extend([";", "display-message", "-p", BOUNDARY]);
        }
        let reply = self.required(&args);
        assert!(
            reply.stderr.is_empty(),
            "batched observation stderr: {:?}",
            reply.stderr
        );
        let separator = format!("{BOUNDARY}\n");
        let mut rest = reply.stdout.as_slice();
        let mut replies = Vec::new();
        for _ in commands {
            let end = rest
                .windows(separator.len())
                .position(|bytes| bytes == separator.as_bytes())
                .expect("observation boundary");
            replies.push(Reply {
                code: Some(0),
                stdout: rest[..end].to_vec(),
                stderr: Vec::new(),
            });
            rest = &rest[end + separator.len()..];
        }
        assert!(rest.is_empty(), "unexpected trailing observation bytes");
        replies
    }
}
impl Drop for Mux {
    fn drop(&mut self) {
        if let Err(error) = self.try_run(&["kill-server"]) {
            eprintln!(
                "private socket cleanup {}: {error}",
                self.socket().display()
            );
        }
    }
}

struct Terminal {
    process: LaunchedProcess,
    screen: Screen,
    registry: HyperlinkRegistry,
    palette: ColourPalette,
    parser: InputCtx,
}
impl Terminal {
    fn attach(mux: &Mux, read_only: bool) -> Self {
        let mut argv = vec![
            mux.binary.as_os_str().as_encoded_bytes().to_vec(),
            b"-u".to_vec(),
            b"-S".to_vec(),
            mux.socket().as_os_str().as_encoded_bytes().to_vec(),
            b"-f".to_vec(),
            b"/dev/null".to_vec(),
            b"attach-session".to_vec(),
            b"-t".to_vec(),
            b"copy".to_vec(),
        ];
        if read_only {
            argv.push(b"-r".to_vec());
        }
        let process = PreparedLaunch::new(LaunchOptions {
            shell: b"/bin/sh".to_vec(),
            argv,
            environment: vec![
                b"TERM=screen".to_vec(),
                b"LC_ALL=C.UTF-8".to_vec(),
                b"PATH=/usr/bin:/bin".to_vec(),
                b"HOME=/tmp".to_vec(),
            ],
            cwd: b"/".to_vec(),
            home: Some(b"/tmp".to_vec()),
            termios: None,
            backspace: 0x7f,
            size: Winsize {
                cols: WIDTH,
                rows: HEIGHT,
                xpixel: 0,
                ypixel: 0,
            },
        })
        .expect("PTY client launch preparation")
        .launch()
        .expect("PTY client launch");
        let mut registry = HyperlinkRegistry::new();
        let screen = Screen::new(
            u32::from(WIDTH),
            u32::from(HEIGHT),
            0,
            ScreenResetPolicy::default(),
            &mut registry,
        )
        .expect("client terminal model");
        let mut terminal = Self {
            process,
            screen,
            registry,
            palette: ColourPalette::new(),
            parser: InputCtx::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let listing = mux.required(&["list-clients", "-F", "#{client_name}\t#{client_flags}"]);
            let tty = terminal.name();
            if listing.stdout.split(|&byte| byte == b'\n').any(|line| {
                line.split(|&byte| byte == b'\t').next() == Some(tty.as_bytes())
                    && (!read_only || contains(line, b"read-only"))
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "attached client not registered: {listing:?}"
            );
            terminal.drain();
        }
        terminal.drain();
        terminal
    }
    fn name(&self) -> String {
        String::from_utf8(self.process.tty.clone()).expect("PTY client name")
    }
    fn drain(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut last = Instant::now();
        let mut buffer = [0; 16384];
        loop {
            match rmux_sys::fd::read(self.process.master.as_fd(), &mut buffer) {
                Ok(0) => panic!("attached client exited before snapshot"),
                Ok(size) => {
                    let mut sink = ScreenOnlySink;
                    let mut writer = ScreenWriteCtx::start(
                        &mut self.screen,
                        &mut sink,
                        ScreenWritePolicy {
                            pane_backed: false,
                            ..Default::default()
                        },
                        &mut self.registry,
                        #[cfg(feature = "sixel")]
                        None,
                    );
                    self.parser.parse(
                        &mut writer,
                        Some(&mut self.palette),
                        &InputPolicy {
                            has_pane: false,
                            writer_has_pane: false,
                            ..Default::default()
                        },
                        &mut NullSink,
                        &buffer[..size],
                    );
                    writer.finish();
                    last = Instant::now();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if last.elapsed() >= Duration::from_millis(65) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("reading attached terminal: {error}"),
            }
            assert!(Instant::now() < deadline, "terminal output did not settle");
        }
    }
    fn model(&mut self) -> Vec<u8> {
        self.drain();
        // Compare terminal state, not redraw packetization or transient title bytes.
        let mut bytes = format!(
            "{} {} {} {}\n",
            self.screen.cx,
            self.screen.cy,
            self.screen.mode.bits(),
            self.screen.cstyle as i32
        )
        .into_bytes();
        for y in 0..u32::from(HEIGHT) {
            for x in 0..u32::from(WIDTH) {
                let cell = self.screen.grid.view_get_cell(x, y);
                bytes.extend_from_slice(
                    format!(
                        "{:?}|{}|{}|{}|{}|{}|{};",
                        cell.data.bytes(),
                        cell.data.width,
                        cell.attr.0,
                        cell.flags.0,
                        cell.fg.0,
                        cell.bg.0,
                        cell.us.0
                    )
                    .as_bytes(),
                );
            }
            bytes.push(b'\n');
        }
        bytes
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        if matches!(
            rmux_sys::proc::wait_process(self.process.pid, true),
            Ok(None)
        ) {
            let _ = rmux_sys::proc::terminate_process(self.process.pid);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match rmux_sys::proc::wait_process(self.process.pid, true) {
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Ok(None) => {
                        eprintln!(
                            "owned PTY client {:?} did not reap within 5s",
                            self.process.pid
                        );
                        break;
                    }
                    _ => break,
                }
            }
        }
    }
}

struct Driver {
    // Clients are closed before their server and its directory.
    writable: Terminal,
    read_only: Option<Terminal>,
    mux: Mux,
}
impl Driver {
    fn start(binary: PathBuf, directory: PathBuf) -> Self {
        fs::create_dir(&directory).expect("private server directory");
        for name in ["output-gate", "mutation-gate"] {
            let status = Command::new("mkfifo")
                .arg(directory.join(name))
                .status()
                .expect("fixture FIFO");
            assert!(status.success(), "fixture FIFO creation");
        }
        let mut mux = Mux {
            binary,
            directory,
            caller: None,
        };
        mux.required(&[
            "new-session",
            "-d",
            "-s",
            "copy",
            "-x",
            "40",
            "-y",
            "10",
            "exec sleep 36000",
        ]);
        for args in [
            vec!["set-option", "-g", "status", "off"],
            vec!["set-option", "-g", "set-clipboard", "off"],
            vec!["set-option", "-g", "history-limit", "2000"],
            vec!["set-option", "-s", "message-limit", "200"],
            vec!["set-option", "-g", "default-shell", "/bin/sh"],
            vec!["set-option", "-g", "mouse", "on"],
            vec!["set-window-option", "-g", "window-size", "manual"],
            vec!["set-window-option", "-g", "copy-mode-line-numbers", "off"],
            vec![
                "set-window-option",
                "-g",
                "copy-mode-position-format",
                "[#{scroll_position}/#{history_size}]",
            ],
            vec!["set-window-option", "-g", "pane-scrollbars", "off"],
            vec!["set-option", "-g", "display-time", "3600000"],
        ] {
            mux.required(&args);
        }
        let writable = Terminal::attach(&mux, false);
        mux.caller = Some(writable.name());
        Self {
            mux,
            writable,
            read_only: None,
        }
    }
    fn readonly(&mut self) {
        self.read_only = Some(Terminal::attach(&self.mux, true));
    }
    fn spawn_fixture(&self, fixture: &Path, tag: usize, keys: &str) -> PathBuf {
        if tag != 0 {
            self.mux.required(&["kill-window", "-t", "copy:1"]);
        }
        let buffers = self.mux.required(&["list-buffers", "-F", "#{buffer_name}"]);
        for name in buffers
            .stdout
            .split(|&byte| byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            self.mux
                .required(&["delete-buffer", "-b", std::str::from_utf8(name).unwrap()]);
        }
        let mutation = self.mux.directory.join("next-output.bin");
        let _ = fs::remove_file(&mutation);
        let started = self.mux.directory.join(format!("started-{tag}"));
        let shell = format!(
            "stty -echo -opost; : > {}; read -r token < {}; cat {}; read -r token < {}; cat {}; exec sleep 36000",
            quote_path(&started),
            quote_path(&self.mux.directory.join("output-gate")),
            quote_path(fixture),
            quote_path(&self.mux.directory.join("mutation-gate")),
            quote_path(&mutation)
        );
        self.mux.required(&[
            "new-window",
            "-d",
            "-t",
            "copy:1",
            "-n",
            "fixture",
            &shell,
            ";",
            "set-window-option",
            "-t",
            "copy:1",
            "mode-keys",
            keys,
            ";",
            "resize-window",
            "-t",
            "copy:1",
            "-x",
            "40",
            "-y",
            "10",
            ";",
            "select-window",
            "-t",
            "copy:1",
            ";",
            "set-buffer",
            "-b",
            "seed",
            "seed-prefix:\twide=界\n",
        ]);
        started
    }
    fn enter(&self, command: &str) {
        self.mux.required(&["copy-mode", "-t", TARGET]);
        self.mux.modes(&[
            &["history-top"],
            &["cursor-down"],
            &["start-of-line"],
            &["search-forward-text", "alpha"],
            &["set-mark"],
            &["cursor-down"],
            &["start-of-line"],
            &["cursor-right"],
            &["cursor-right"],
            &["begin-selection"],
            &["cursor-right"],
            &["cursor-down"],
            &["cursor-right"],
            // Retain marks after selection setup to observe each clear policy.
            &["search-forward-text", "alpha"],
        ]);
        if command.starts_with("refresh-") {
            fs::write(
                self.mux.directory.join("next-output.bin"),
                b"BASE-UPDATED-BEHIND-COPY\r\n",
            )
            .expect("refresh pane output mutation");
            fs::write(self.mux.directory.join("mutation-gate"), b"go\n")
                .expect("refresh output gate");
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let output = self
                    .mux
                    .required(&["capture-pane", "-p", "-t", TARGET, "-S", "-"]);
                if contains(&output.stdout, b"BASE-UPDATED-BEHIND-COPY") {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "refresh output mutation was not parsed"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if command.starts_with("jump-") && command != "jump-to-mark" {
            self.mux.mode(&["clear-selection"]);
            self.mux.mode(&["start-of-line"]);
            self.mux.mode(&["jump-forward", "x"]);
            if matches!(command, "jump-forward" | "jump-to-forward" | "jump-again") {
                self.mux.mode(&["start-of-line"]);
            } else {
                self.mux.mode(&["end-of-line"]);
            }
        }
        match command {
            "begin-selection" => self.mux.mode(&["clear-selection"]),
            "line-numbers-off" => self.mux.mode(&["line-numbers-on"]),
            "rectangle-off" => self.mux.mode(&["rectangle-on"]),
            "refresh-off" => self.mux.mode(&["refresh-on"]),
            "scroll-exit-off" => self.mux.mode(&["scroll-exit-on"]),
            "next-matching-bracket" => {
                self.mux.mode(&["clear-selection"]);
                self.mux.mode(&["search-forward-text", "("]);
            }
            "previous-matching-bracket" => {
                self.mux.mode(&["clear-selection"]);
                self.mux.mode(&["search-forward-text", ")"]);
            }
            "jump-to-mark" => self.mux.mode(&["cursor-down"]),
            // Force all conditional cancel variants onto their exit path.
            "cursor-down-and-cancel"
            | "halfpage-down-and-cancel"
            | "page-down-and-cancel"
            | "scroll-down-and-cancel" => {
                self.mux.mode(&["clear-selection"]);
                self.mux.mode(&["history-bottom"]);
                self.mux.mode(&["bottom-line"]);
            }
            _ => {}
        }
    }
    fn dispatch(&self, positional: &[String], prefix: u32, read_only: bool) -> Reply {
        let caller = if read_only {
            self.read_only.as_ref().unwrap()
        } else {
            &self.writable
        };
        let name = caller.name();
        let count = prefix.to_string();
        let mut args = vec!["send-keys", "-c", &name, "-t", TARGET, "-X", "-N", &count];
        args.extend(positional.iter().map(String::as_str));
        self.mux.run(&args)
    }
    fn dispatch_mouse(
        &mut self,
        positional: &[String],
        read_only: bool,
        prefix: u32,
        keys: &str,
        case: usize,
    ) -> Reply {
        let table = if keys == "vi" {
            "copy-mode-vi"
        } else {
            "copy-mode"
        };
        let count = prefix.to_string();
        let done = self.mux.directory.join(format!("mouse-{case}.done"));
        let signal = format!(": > {}", quote_path(&done));
        let caller = if read_only {
            self.read_only.as_ref().unwrap()
        } else {
            &self.writable
        };
        let mut command = vec![
            "send-keys".to_owned(),
            "-c".to_owned(),
            caller.name(),
            "-t".to_owned(),
            TARGET.to_owned(),
            "-X".to_owned(),
            "-N".to_owned(),
            count,
        ];
        command.extend_from_slice(positional);
        let script = format!(
            "{}; run-shell {}",
            command
                .iter()
                .map(|arg| quote_argument(arg))
                .collect::<Vec<_>>()
                .join(" "),
            quote_argument(&signal)
        );
        let reply = self
            .mux
            .required(&["bind-key", "-T", table, "MouseDown1Pane", &script]);
        let event = b"\x1b[<0;15;6M";
        assert_eq!(
            rmux_sys::fd::write(self.writable.process.master.as_fd(), event)
                .expect("injecting real SGR mouse input"),
            event.len()
        );
        wait_file(&done);
        let release = b"\x1b[<0;15;6m";
        assert_eq!(
            rmux_sys::fd::write(self.writable.process.master.as_fd(), release)
                .expect("releasing real SGR mouse input"),
            release.len()
        );
        self.mux
            .required(&["unbind-key", "-T", table, "MouseDown1Pane"]);
        reply
    }
    fn probe_prefix(&self) -> Reply {
        let name = self.writable.name();
        self.mux
            .run(&["send-keys", "-c", &name, "-t", TARGET, "-X", "cursor-right"])
    }
    fn snapshot(&mut self) -> Snapshot {
        let format = FORMAT_KEYS
            .iter()
            .map(|key| format!("{key}=#{{{key}}}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut commands = vec![vec!["display-message", "-p", "-t", TARGET, &format]];
        commands.extend([
            vec![
                "capture-pane",
                "-p",
                "-t",
                TARGET,
                "-M",
                "-S",
                "-",
                "-E",
                "-",
            ],
            vec![
                "capture-pane",
                "-p",
                "-t",
                TARGET,
                "-M",
                "-e",
                "-N",
                "-T",
                "-S",
                "-",
                "-E",
                "-",
            ],
            vec!["capture-pane", "-p", "-t", TARGET, "-S", "-", "-E", "-"],
            vec![
                "capture-pane",
                "-p",
                "-t",
                TARGET,
                "-M",
                "-F",
                "-L",
                "-S",
                "-",
                "-E",
                "-",
            ],
            vec!["list-buffers", "-F", "#{buffer_name}\t#{buffer_size}"],
        ]);
        if self.read_only.is_some() {
            commands.push(vec!["show-messages"]);
        }
        let mut replies = self.mux.batch(&commands).into_iter();
        let mut formats = replies.next().unwrap();
        let timestamp = b"top_line_time=";
        if let Some(start) = formats
            .stdout
            .windows(timestamp.len())
            .position(|bytes| bytes == timestamp)
        {
            let start = start + timestamp.len();
            let end = start
                + formats.stdout[start..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .unwrap();
            if end > start && &formats.stdout[start..end] != b"0" {
                assert!(formats.stdout[start..end].iter().all(u8::is_ascii_digit));
                formats
                    .stdout
                    .splice(start..end, b"<wall-time>".iter().copied());
            }
        }
        let captures = replies.by_ref().take(4).collect();
        let listing = replies.next().unwrap();
        let mut buffers = Vec::new();
        for line in listing
            .stdout
            .split(|&byte| byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let name = line.split(|&byte| byte == b'\t').next().unwrap();
            let bytes =
                self.mux
                    .run(&["save-buffer", "-b", std::str::from_utf8(name).unwrap(), "-"]);
            buffers.push((name.to_vec(), bytes));
        }
        let readonly_messages = if self.read_only.is_some() {
            let messages = replies.next().unwrap();
            assert!(
                messages.success(),
                "read-only status message log unavailable: {messages:?}"
            );
            messages
                .stdout
                .split(|&byte| byte == b'\n')
                .filter(|line| contains(line, b"client is read-only"))
                .count()
        } else {
            0
        };
        let visible = self.writable.model();
        let readonly_visible = self.read_only.as_mut().map(Terminal::model);
        Snapshot {
            formats,
            captures,
            listing,
            buffers,
            visible,
            readonly_visible,
            readonly_messages,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Snapshot {
    formats: Reply,
    captures: Vec<Reply>,
    listing: Reply,
    buffers: Vec<(Vec<u8>, Reply)>,
    visible: Vec<u8>,
    readonly_visible: Option<Vec<u8>>,
    readonly_messages: usize,
}
impl Snapshot {
    fn mode_present(&self) -> bool {
        contains(&self.formats.stdout, b"pane_in_mode=1\n")
    }
    fn assert_available(&self, context: &str) {
        assert!(
            self.formats.success(),
            "{context}: format query unavailable: {:?}",
            self.formats
        );
        for capture in &self.captures {
            assert!(
                capture.success(),
                "{context}: capture unavailable (no fallback): {capture:?}"
            );
        }
        assert!(
            self.listing.success(),
            "{context}: buffer enumeration failed: {:?}",
            self.listing
        );
        for (_, buffer) in &self.buffers {
            assert!(
                buffer.success(),
                "{context}: raw paste buffer export failed: {buffer:?}"
            );
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|bytes| bytes == needle)
}
fn quote_argument(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', "'\\''"))
}
fn quote_path(path: &Path) -> String {
    quote_argument(path.to_str().expect("private fixture path"))
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "pane fixture did not start: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn fixture_bytes() -> Vec<u8> {
    let mut output = b"\x1b[2J\x1b[H".to_vec();
    for row in 0..84 {
        if row % 7 == 6 {
            output.extend_from_slice(b"\r\n");
            continue;
        }
        output.extend_from_slice(b"\x1b]133;A\x07");
        output.extend_from_slice(format!("    x alpha {row:02} (beta[x]gamma) x\r\n").as_bytes());
        output.extend_from_slice(b"\x1b]133;B\x07\x1b]133;C\x07");
        output.extend_from_slice(b"tab\twide=\xe7\x95\x8c combine=e\xcc\x81  x alpha x\r\n");
        output.extend_from_slice(b"\x1b[31mwrapped alpha x 012345678901234567890123456789012345678901234567890 x tail\x1b[0m\r\n");
        output.extend_from_slice(
            b"\x1b]8;;https://example.invalid/copy\x07link alpha x\x1b]8;;\x07\r\n",
        );
        output.extend_from_slice(b"\x1b]133;D;0\x07");
    }
    output.extend_from_slice(b"COPY-FIXTURE-READY\r\n");
    output
}
fn prepare_pair(
    expected: &mut Driver,
    actual: &mut Driver,
    fixture: &Path,
    case: usize,
    keys: &str,
    command: &str,
) {
    let first = expected.spawn_fixture(fixture, case, keys);
    let second = actual.spawn_fixture(fixture, case, keys);
    wait_file(&first);
    wait_file(&second);
    for driver in [&*expected, &*actual] {
        fs::write(driver.mux.directory.join("output-gate"), b"go\n")
            .expect("paired pane output gate");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ready = [false; 2];
    while !ready.iter().all(|value| *value) {
        for (index, driver) in [&mut *expected, &mut *actual].into_iter().enumerate() {
            if ready[index] {
                continue;
            }
            let capture = driver
                .mux
                .required(&["capture-pane", "-p", "-t", TARGET, "-S", "-"]);
            ready[index] = contains(&capture.stdout, b"COPY-FIXTURE-READY");
            driver.writable.drain();
            if let Some(client) = &mut driver.read_only {
                client.drain();
            }
        }
        assert!(
            Instant::now() < deadline,
            "paired fixture readiness timeout"
        );
    }
    expected.enter(command);
    actual.enter(command);
}

fn valid_arguments(spec: &CopyCommandSpec, pipe: &str, flags: &str) -> Vec<String> {
    let name = std::str::from_utf8(spec.name).unwrap();
    let mut positional = vec![name.to_owned()];
    for flag in flags.bytes() {
        assert!(
            spec.args.template.contains(&flag),
            "{name}: flag absent from command table"
        );
        positional.push(format!("-{}", char::from(flag)));
    }
    if name.contains("pipe") {
        positional.push(pipe.to_owned());
        if spec.args.upper == 2 {
            positional.push("copy-prefix".to_owned());
        }
    } else if spec.args.template == b"CP" {
        positional.push("copy-prefix".to_owned());
    } else if spec.args.upper > 0 {
        positional.push(
            match name {
                "goto-line" => "3",
                "selection-mode" => "line",
                _ if name.starts_with("jump-") => "x",
                _ if name.ends_with("incremental") => "=alpha",
                _ if name.ends_with("-text") => "alpha",
                _ if name.starts_with("search-") => "alpha|beta",
                _ => panic!("no valid argument fixture for {name}"),
            }
            .to_owned(),
        );
    }
    let count = positional.len() - 1 - flags.len();
    assert!(
        count >= spec.args.lower as usize && count <= spec.args.upper as usize,
        "{name}: fixture arguments violate table bounds"
    );
    positional
}

struct Comparison {
    differences: usize,
    examples: Vec<String>,
    mutation_pending: bool,
    mutation_exercised: bool,
}
impl Comparison {
    fn record<T: std::fmt::Debug + PartialEq>(&mut self, expected: &T, actual: &T, case: &str) {
        if expected != actual {
            self.differences += 1;
            if self.examples.is_empty() {
                eprintln!("COPY FIRST DIFFERENCE {case}: oracle={expected:?}; rmux={actual:?}");
            }
            if self.examples.len() < 12 {
                self.examples
                    .push(format!("{case}\noracle={expected:?}\nrmux={actual:?}"));
            }
        }
    }
    fn snapshot(&mut self, expected: &Snapshot, mut actual: Snapshot, case: &str, mutate: bool) {
        if mutate && self.mutation_pending {
            let original = actual.clone();
            let byte = actual
                .formats
                .stdout
                .first_mut()
                .expect("nonempty compared format byte");
            *byte ^= 1;
            assert_ne!(
                actual, original,
                "one-byte mutation was not visible to snapshot equality"
            );
            let mut corrupted_oracle = expected.clone();
            *corrupted_oracle
                .formats
                .stdout
                .first_mut()
                .expect("oracle compared byte") ^= 1;
            assert_ne!(
                expected, &corrupted_oracle,
                "oracle comparator mutation sensitivity"
            );
            self.mutation_pending = false;
            self.mutation_exercised = true;
        }
        self.record(
            &expected.formats,
            &actual.formats,
            &format!("{case} formats"),
        );
        self.record(
            &expected.captures,
            &actual.captures,
            &format!("{case} captures"),
        );
        self.record(
            &expected.listing,
            &actual.listing,
            &format!("{case} buffer listing"),
        );
        self.record(
            &expected.buffers,
            &actual.buffers,
            &format!("{case} buffers"),
        );
        self.record(
            &expected.visible,
            &actual.visible,
            &format!("{case} visible"),
        );
        self.record(
            &expected.readonly_visible,
            &actual.readonly_visible,
            &format!("{case} readonly visible"),
        );
        self.record(
            &expected.readonly_messages,
            &actual.readonly_messages,
            &format!("{case} readonly messages"),
        );
    }
}

fn pipe_command(driver: &Driver, case: usize) -> String {
    let output = driver.mux.directory.join(format!("pipe-{case}.bin"));
    let done = driver.mux.directory.join(format!("pipe-{case}.done"));
    // Completion is signalled only after cat reaches EOF and closes the output.
    format!("cat > {}; : > {}", quote_path(&output), quote_path(&done))
}
fn pipe_bytes(driver: &Driver, case: usize, launched: bool) -> Option<Vec<u8>> {
    if !launched {
        return None;
    }
    wait_file(&driver.mux.directory.join(format!("pipe-{case}.done")));
    Some(
        fs::read(driver.mux.directory.join(format!("pipe-{case}.bin")))
            .expect("completed pipe bytes"),
    )
}

struct Case<'a> {
    keys: &'a str,
    prefix: u32,
    spec: Option<&'a CopyCommandSpec>,
    kind: &'a str,
    flags: &'a str,
    read_only: bool,
}
#[allow(clippy::too_many_arguments)]
fn run_case(
    expected: &mut Driver,
    actual: &mut Driver,
    _directory: &Path,
    fixture: &Path,
    index: usize,
    case: Case<'_>,
    comparison: &mut Comparison,
) {
    let name = case.spec.map_or("unknown-copy-command", |spec| {
        std::str::from_utf8(spec.name).unwrap()
    });
    let label = format!(
        "{index}: {} prefix={} {name} {} flags={} readonly={}",
        case.keys, case.prefix, case.kind, case.flags, case.read_only
    );
    eprintln!("COPY COMMAND CASE BEGIN {label}");
    prepare_pair(expected, actual, fixture, index, case.keys, name);
    if case.read_only {
        // Keep the rejection witness local; connection logs otherwise evict it.
        for driver in [&*expected, &*actual] {
            driver
                .mux
                .required(&["set-option", "-s", "message-limit", "0"]);
            driver
                .mux
                .required(&["set-option", "-s", "message-limit", "200"]);
        }
    }
    let before_expected = expected.snapshot();
    let before_actual = actual.snapshot();
    before_expected.assert_available(&label);
    before_actual.assert_available(&label);
    assert!(
        before_expected.mode_present() && before_actual.mode_present(),
        "{label}: copy mode entry failed"
    );
    comparison.snapshot(
        &before_expected,
        before_actual,
        &format!("{label} before"),
        false,
    );
    let arguments = |driver: &Driver| match case.kind {
        "valid" => valid_arguments(case.spec.unwrap(), &pipe_command(driver, index), case.flags),
        "wrong-arity" => {
            let spec = case.spec.unwrap();
            let mut args = vec![name.to_owned()];
            // Cover both too few required arguments and too many optional arguments.
            let count = if spec.args.lower > 0 {
                spec.args.lower - 1
            } else {
                spec.args.upper + 1
            };
            args.extend((0..count).map(|_| "extra".to_owned()));
            args
        }
        "invalid-flag" => vec![name.to_owned(), "-Z".to_owned()],
        "unknown" => vec![name.to_owned()],
        "no-positional" => Vec::new(),
        "outer-repeat-error" => vec!["cursor-left".to_owned()],
        _ => panic!("unknown case kind"),
    };
    let (reply_expected, reply_actual) = if case.kind == "outer-repeat-error" {
        (
            expected
                .mux
                .run(&["send-keys", "-t", TARGET, "-X", "-N", "0", "cursor-left"]),
            actual
                .mux
                .run(&["send-keys", "-t", TARGET, "-X", "-N", "0", "cursor-left"]),
        )
    } else if name == "scroll-to-mouse" {
        let first = arguments(expected);
        let second = arguments(actual);
        (
            expected.dispatch_mouse(&first, case.read_only, case.prefix, case.keys, index),
            actual.dispatch_mouse(&second, case.read_only, case.prefix, case.keys, index),
        )
    } else {
        (
            expected.dispatch(&arguments(expected), case.prefix, case.read_only),
            actual.dispatch(&arguments(actual), case.prefix, case.read_only),
        )
    };
    comparison.record(
        &reply_expected,
        &reply_actual,
        &format!("{label} dispatch status/stdout/stderr"),
    );
    let pipe_launched = case.kind == "valid" && name.contains("pipe") && !case.read_only;
    let expected_pipe = pipe_bytes(expected, index, pipe_launched);
    let actual_pipe = pipe_bytes(actual, index, pipe_launched);
    comparison.record(
        &expected_pipe,
        &actual_pipe,
        &format!("{label} eventual pipe bytes"),
    );
    let after_expected = expected.snapshot();
    let after_actual = actual.snapshot();
    after_expected.assert_available(&label);
    after_actual.assert_available(&label);
    if case.kind == "valid"
        && !case.read_only
        && (name == "cancel" || name.ends_with("-and-cancel"))
    {
        assert!(
            !after_expected.mode_present(),
            "{label}: oracle fixture missed cancellation effect path"
        );
    }
    if case.read_only && !case.spec.unwrap().read_only {
        assert!(
            after_expected.mode_present(),
            "{label}: oracle read-only rejection removed mode"
        );
        assert!(
            after_expected.readonly_messages > before_expected.readonly_messages,
            "{label}: read-only caller did not reach the command rejection path"
        );
    }
    comparison.snapshot(
        &after_expected,
        after_actual,
        &format!("{label} after"),
        true,
    );
    // With no -N, this observes prefix reset, or retention on early returns.
    let probe_expected = expected.probe_prefix();
    let probe_actual = actual.probe_prefix();
    comparison.record(
        &probe_expected,
        &probe_actual,
        &format!("{label} prefix probe status/stdout/stderr"),
    );
    comparison.snapshot(
        &expected.snapshot(),
        actual.snapshot(),
        &format!("{label} prefix probe state"),
        false,
    );
    eprintln!(
        "COPY COMMAND CASE END {label}; differences={}",
        comparison.differences
    );
}

fn binaries() -> Option<(PathBuf, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let oracle = std::env::var_os("RMUX_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("oracle/bin/tmux"));
    if !oracle.is_file() {
        eprintln!(
            "SKIP copy command oracle: pinned tmux 8f25579c missing at {}; set RMUX_ORACLE",
            oracle.display()
        );
        return None;
    }
    let actual = if let Some(binary) = std::env::var_os("RMUX_COPY_BINARY") {
        PathBuf::from(binary)
    } else {
        let executable = std::env::current_exe().expect("integration test executable");
        let profile = executable
            .parent()
            .and_then(Path::parent)
            .expect("Cargo test profile");
        let current = profile.join("rmux");
        if current.is_file() {
            current
        } else {
            std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("target"))
                .join("debug/rmux")
        }
    };
    if !actual.is_file() {
        eprintln!(
            "SKIP copy command oracle: built rmux missing at {}; build rmux or set RMUX_COPY_BINARY",
            actual.display()
        );
        return None;
    }
    Some((
        fs::canonicalize(oracle).unwrap(),
        fs::canonicalize(actual).unwrap(),
    ))
}

fn command_lane(
    oracle: PathBuf,
    binary: PathBuf,
    keys: &'static str,
    exhaustive: bool,
) -> Comparison {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = Directory(PathBuf::from("/tmp/swarm-rmux-build").join(format!(
        "copy-command-oracle-{}-{nonce}-{keys}",
        std::process::id()
    )));
    fs::create_dir_all(&directory.0).expect("private oracle fixture directory");
    let fixture = directory.0.join("fixture.bin");
    fs::write(&fixture, fixture_bytes()).expect("deterministic terminal fixture");
    let mut expected = Driver::start(oracle, directory.0.join("oracle"));
    let mut actual = Driver::start(binary, directory.0.join("rmux"));
    let mutate = keys == "emacs"
        && std::env::var_os("RMUX_COPY_COMMAND_ORACLE_MUTATE").is_some_and(|value| value == "1");
    let mut comparison = Comparison {
        differences: 0,
        examples: Vec::new(),
        mutation_pending: mutate,
        mutation_exercised: false,
    };
    let mut cases = Vec::new();
    for (row, spec) in COMMAND_TABLE.iter().enumerate() {
        let prefixes: &[u32] = if exhaustive {
            &[1, 2, 5]
        } else {
            &[[1, 2, 5][row % 3]]
        };
        for &prefix in prefixes {
            cases.push(Case {
                keys,
                prefix,
                spec: Some(spec),
                kind: "valid",
                flags: "",
                read_only: false,
            });
        }
    }
    assert_eq!(
        cases.len(),
        if exhaustive { 297 } else { 99 },
        "every command runs in each key mode"
    );
    // Repeat values share dispatch. Exercise one iterative movement at all three.
    let repeat = COMMAND_TABLE
        .iter()
        .find(|spec| spec.name == b"cursor-right")
        .unwrap();
    for prefix in [1, 2, 5] {
        cases.push(Case {
            keys,
            prefix,
            spec: Some(repeat),
            kind: "valid",
            flags: "",
            read_only: false,
        });
    }
    {
        let prefix = 1;
        // Parsing occurs before handler dispatch (window-copy.c:3936-3944).
        // Commands sharing the same argument grammar take the same error branch.
        let mut grammars = std::collections::HashSet::new();
        for spec in &COMMAND_TABLE {
            if grammars.insert((spec.args.template, spec.args.lower, spec.args.upper)) {
                for kind in ["wrong-arity", "invalid-flag"] {
                    cases.push(Case {
                        keys,
                        prefix,
                        spec: Some(spec),
                        kind,
                        flags: "",
                        read_only: false,
                    });
                }
            }
        }
        for kind in ["unknown", "no-positional", "outer-repeat-error"] {
            cases.push(Case {
                keys,
                prefix,
                spec: None,
                kind,
                flags: "",
                read_only: false,
            });
        }
    }
    {
        let prefix = 2;
        for spec in &COMMAND_TABLE {
            if !spec.args.template.is_empty() {
                let flags: &[&str] = match spec.args.template {
                    b"CP" => &["C", "P", "CP"],
                    b"o" => &["o"],
                    _ => &["e"],
                };
                for flags in flags {
                    cases.push(Case {
                        keys,
                        prefix,
                        spec: Some(spec),
                        kind: "valid",
                        flags,
                        read_only: false,
                    });
                }
            }
        }
    }
    let writable_count = cases.len();
    for (index, case) in cases.into_iter().enumerate() {
        run_case(
            &mut expected,
            &mut actual,
            &directory.0,
            &fixture,
            index,
            case,
            &mut comparison,
        );
    }
    {
        let prefix = 5;
        expected.readonly();
        actual.readonly();
        // Permission checking precedes repeat handling (window-copy.c:3927-3933).
        let permissions = [b"cursor-left".as_slice(), b"copy-selection".as_slice()];
        for (index, name) in permissions.iter().enumerate() {
            let spec = COMMAND_TABLE
                .iter()
                .find(|spec| spec.name == *name)
                .unwrap();
            run_case(
                &mut expected,
                &mut actual,
                &directory.0,
                &fixture,
                writable_count + index,
                Case {
                    keys,
                    prefix,
                    spec: Some(spec),
                    kind: "valid",
                    flags: "",
                    read_only: true,
                },
                &mut comparison,
            );
        }
    }
    assert_eq!(
        comparison.mutation_exercised, mutate,
        "mutation hook was not exercised"
    );
    comparison
}

fn command_matrix(exhaustive: bool) {
    let Some((oracle, binary)) = binaries() else {
        return;
    };
    assert_eq!(
        COMMAND_TABLE.len(),
        99,
        "the pinned command inventory changed"
    );
    let started = Instant::now();
    let mut lanes = Vec::new();
    for keys in ["emacs", "vi"] {
        let oracle = oracle.clone();
        let binary = binary.clone();
        lanes.push(std::thread::spawn(move || {
            command_lane(oracle, binary, keys, exhaustive)
        }));
    }
    let mut differences = 0;
    let mut examples = Vec::new();
    for lane in lanes {
        let comparison = lane.join().expect("private command lane");
        differences += comparison.differences;
        examples.extend(comparison.examples);
    }
    eprintln!(
        "copy oracle exhaustive={exhaustive}, every command/mode pair in {:?}",
        started.elapsed()
    );
    assert_eq!(
        differences,
        0,
        "copy command oracle mismatches:\n{}",
        examples.join("\n\n")
    );
}

#[test]
fn all_copy_commands_modes_prefixes_match_pinned_oracle() {
    command_matrix(false);
}

#[test]
#[ignore = "exhaustive command × mode × prefix matrix; run on demand"]
fn exhaustive_copy_commands_modes_prefixes_match_pinned_oracle() {
    command_matrix(true);
}
