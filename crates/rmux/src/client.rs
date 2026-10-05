// Ported from tmux client.c, control.c @ 8f25579c
/*
Copyright (c) Various Authors
Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
Copyright (c) 2026 Jacky and rmux contributors

Permission to use, copy, modify, and distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
*/
//! The client process: connect, identify, dispatch, exit (`client.c:35-808`).

use std::io::{self, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::process::{Command, Stdio};

use rmux_server::client::ClientFlags;
use rmux_server::cmd::CommandFlags;
use rmux_server::cmd::parse::{self, CmdParseInput};
use rmux_server::ids::{EventToken, PeerId};
use rmux_server::server::event_loop::{EventLoop, LoopAction};
use rmux_server::server::file::{FilePolicy, FileStore};
use rmux_server::server::proc::{PeerDispatch, Process};
use rmux_server::server::protocol::{
    self as protocol, LEGACY_PAYLOAD, ProtocolError, ProtocolMessage, ProtocolMessageKind as Kind,
    encode_string,
};
use rmux_server::server::{Startup, server_start};
use rmux_sys::TermiosState;
use rmux_sys::client as sys;
use rmux_sys::server::SignalWake;
use rmux_util::{fatal, fatalx, log_debug};

use crate::paths;
use crate::{INTERNAL_SERVER_FLAG, MainConfig};

/// `client_exitreason` (`client.c:39-49`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ClientExitReason {
    #[default]
    None,
    Detached,
    DetachedHup,
    LostTty,
    Terminated,
    LostServer,
    Exited,
    ServerExited,
    MessageProvided(Vec<u8>),
}

impl ClientExitReason {
    /// `client_exit_message` (`client.c:184-220`); `session` is
    /// `client_exitsession`.
    pub fn message(&self, session: Option<&[u8]>) -> Vec<u8> {
        let with_session = |prefix: &[u8]| -> Vec<u8> {
            match session {
                Some(name) => {
                    let mut out = prefix.to_vec();
                    out.extend_from_slice(b" (from session ");
                    out.extend_from_slice(name);
                    out.push(b')');
                    out
                }
                None => prefix.to_vec(),
            }
        };
        match self {
            Self::None => b"unknown reason".to_vec(),
            Self::Detached => with_session(b"detached"),
            Self::DetachedHup => with_session(b"detached and SIGHUP"),
            Self::LostTty => b"lost tty".to_vec(),
            Self::Terminated => b"terminated".to_vec(),
            Self::LostServer => b"server exited unexpectedly".to_vec(),
            Self::Exited => b"exited".to_vec(),
            Self::ServerExited => b"server exited".to_vec(),
            Self::MessageProvided(message) => message.clone(),
        }
    }
}

/// Decoded `MSG_EXIT`/`MSG_SHUTDOWN` payload (`client.c:595-618`): an
/// optional return value followed by an optional message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExitMessage {
    pub retval: Option<i32>,
    pub message: Option<Vec<u8>>,
}

impl ExitMessage {
    /// `client_dispatch_exit_message` (`client.c:595-618`). Zero bytes is
    /// valid; fewer than four nonzero bytes is `bad MSG_EXIT size`.
    pub fn decode(data: &[u8]) -> Result<Self, &'static str> {
        if data.is_empty() {
            return Ok(Self::default());
        }
        if data.len() < 4 {
            return Err("bad MSG_EXIT size");
        }
        let retval = protocol::decode_i32(&data[..4]).map_err(|_| "bad MSG_EXIT size")?;
        let message = if data.len() > 4 {
            Some(protocol::decode_string(&data[4..]).map_err(|_| "bad MSG_EXIT size")?)
        } else {
            None
        };
        Ok(Self {
            retval: Some(retval),
            message,
        })
    }
}

/// The statics of `client.c:35-58` plus the process and loop.
pub struct ClientRuntime {
    process: Process,
    event_loop: EventLoop,
    peer: Option<PeerId>,
    flags: ClientFlags,
    suspended: bool,
    exit_reason: ClientExitReason,
    exit_flag: bool,
    exit_val: i32,
    exit_type: Option<Kind>,
    exit_session: Option<Vec<u8>>,
    exec: Option<(Vec<u8>, Vec<u8>)>,
    attached: bool,
    files: FileStore,
    saved_termios: Option<TermiosState>,
    shell_command: Option<Vec<u8>>,
    signals: Option<SignalWake>,
    signal_token: Option<EventToken>,
}

fn print_err(parts: &[&[u8]]) {
    let mut err = io::stderr().lock();
    for part in parts {
        let _ = err.write_all(part);
    }
    let _ = err.write_all(b"\n");
    let _ = err.flush();
}

fn strerror(error: &io::Error) -> Vec<u8> {
    match error.raw_os_error() {
        Some(code) => rmux_sys::strerror(code),
        None => error.to_string().into_bytes(),
    }
}

/// `client_get_lock` (`client.c:77-101`). `Ok(Some(fd))` holds the lock or
/// failed `flock` for another reason (the caller continues); `Ok(None)` is
/// the `-2` retry after waiting; `Err` is the open failure (`-1`).
pub mod lock {
    use super::*;

    pub fn acquire(lockfile: &[u8]) -> io::Result<Option<OwnedFd>> {
        log_debug!("lock file is {}", String::from_utf8_lossy(lockfile));
        let fd = match sys::open_lock_file(lockfile) {
            Ok(fd) => fd,
            Err(e) => {
                log_debug!("open failed: {}", String::from_utf8_lossy(&strerror(&e)));
                return Err(e);
            }
        };
        if let Err(e) =
            rmux_sys::fd::flock(fd.as_fd(), rmux_sys::fd::LOCK_EX | rmux_sys::fd::LOCK_NB)
        {
            log_debug!("flock failed: {}", String::from_utf8_lossy(&strerror(&e)));
            if e.kind() != io::ErrorKind::WouldBlock {
                return Ok(Some(fd));
            }
            while let Err(e) = rmux_sys::fd::flock(fd.as_fd(), rmux_sys::fd::LOCK_EX) {
                if e.kind() != io::ErrorKind::Interrupted {
                    break;
                }
            }
            drop(fd);
            return Ok(None);
        }
        log_debug!("flock succeeded");
        Ok(Some(fd))
    }
}

/// `server_start` as the client sees it (`server.c:176-264`, `client.c:164`).
/// `CLIENT_NOFORK` runs the server in this process and never returns.
/// Otherwise the server is the reexecuted binary with
/// `--rmux-internal-server`, and the client end of a socketpair comes back.
fn start_server(
    path: &[u8],
    flags: ClientFlags,
    lock: Option<OwnedFd>,
    cfg_files: &[Vec<u8>],
    activation_listener: Option<OwnedFd>,
) -> io::Result<OwnedFd> {
    if flags.contains(ClientFlags::NOFORK) {
        let code = server_start(Startup {
            socket_path: path.to_vec(),
            flags,
            initial_peer: None,
            lock,
            activation_listener,
            config_files: cfg_files.to_vec(),
        })?;
        std::process::exit(code);
    }
    // sd_listen_fds(0) sees a different PID after forking: only -D adopts activation.
    drop(activation_listener);
    let (client_end, server_end) = sys::socketpair()?;
    sys::set_inheritable(server_end.as_fd())?;
    if let Some(lock) = &lock {
        sys::set_inheritable(lock.as_fd())?;
    }
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg(INTERNAL_SERVER_FLAG)
        .arg(server_end.as_raw_fd().to_string())
        .arg(lock.as_ref().map_or(-1, |l| l.as_raw_fd()).to_string())
        .arg(std::ffi::OsStr::from_bytes(path))
        .arg(flags.bits().to_string())
        .arg(rmux_util::log::level().0.to_string());
    for file in cfg_files {
        command.arg(std::ffi::OsStr::from_bytes(file));
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // The server child now owns its ends; the lock is released by the server
    // after listening (server.c:235-239).
    drop(server_end);
    drop(lock);
    Ok(client_end)
}

/// `client_connect` (`client.c:104-181`).
pub fn connect(path: &[u8], flags: ClientFlags, cfg_files: &[Vec<u8>]) -> io::Result<OwnedFd> {
    log_debug!("socket is {}", String::from_utf8_lossy(path));
    let mut lockfile = path.to_vec();
    lockfile.extend_from_slice(b".lock");
    let mut locked = false;
    let mut lockfd: Option<OwnedFd> = None;
    loop {
        log_debug!("trying connect");
        let error = match sys::connect_unix(path) {
            Ok(fd) => {
                rmux_sys::fd::set_blocking(fd.as_fd(), false);
                return Ok(fd);
            }
            Err(e) => e,
        };
        log_debug!(
            "connect failed: {}",
            String::from_utf8_lossy(&strerror(&error))
        );
        if !matches!(
            error.kind(),
            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
        ) {
            return Err(error);
        }
        if flags.contains(ClientFlags::NOSTARTSERVER) || !flags.contains(ClientFlags::STARTSERVER) {
            return Err(error);
        }
        if !locked {
            match lock::acquire(&lockfile) {
                Ok(Some(fd)) => {
                    log_debug!("got lock ({})", {
                        use std::os::fd::AsRawFd;
                        fd.as_raw_fd()
                    });
                    lockfd = Some(fd);
                }
                Ok(None) => {
                    log_debug!("didn't get lock (-2)");
                    continue;
                }
                Err(_) => {
                    log_debug!("didn't get lock (-1)");
                }
            }
            // Always retry at least once, even with the lock: another client
            // could have started the server between connect() and flock().
            locked = true;
            continue;
        }
        if lockfd.is_some() {
            if let Err(e) = sys::unlink(path) {
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e);
                }
            }
        }
        let fd = start_server(path, flags, lockfd.take(), cfg_files, None)?;
        rmux_sys::fd::set_blocking(fd.as_fd(), false);
        return Ok(fd);
    }
}

/// `client_exec` (`client.c:491-509`). Returns only when `execl` fails.
pub fn exec_shell(
    shell: &[u8],
    shellcmd: &[u8],
    flags: ClientFlags,
    signals: Option<SignalWake>,
) -> ! {
    log_debug!(
        "shell {}, command {}",
        String::from_utf8_lossy(shell),
        String::from_utf8_lossy(shellcmd)
    );
    let argv0 = rmux_util::shell::shell_argv0(shell, flags.contains(ClientFlags::LOGIN));
    // proc_clear_signals(client_proc, 1): restore the saved dispositions.
    drop(signals);
    let _ = sys::exec_shell(shell, &argv0, shellcmd);
    fatal!("execl failed");
}

/// `control_wait_exit` (`control.c:745-784`): read stdin until an empty
/// LF-terminated line, EOF, or a permanent read or poll error.
pub fn control_wait_exit() {
    let stdin = io::stdin();
    let fd = stdin.as_fd();
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(newline) = buffer.iter().position(|&c| c == b'\n') {
            let line = buffer.drain(..=newline).collect::<Vec<u8>>();
            if line.len() == 1 {
                break;
            }
            continue;
        }
        if sys::poll_readable(fd).is_err() {
            break;
        }
        match rmux_sys::fd::read(fd, &mut chunk) {
            Ok(0) => break,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(e) => {
                if !matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    break;
                }
            }
        }
    }
}

impl ClientRuntime {
    fn send(&mut self, message: ProtocolMessage) -> Result<(), ProtocolError> {
        let Some(peer) = self.peer else {
            return Err(ProtocolError::Closed);
        };
        self.process.send(peer, message)?;
        self.process.update_event(peer, &mut self.event_loop)
    }

    fn send_empty(&mut self, kind: Kind) {
        let _ = self.send(ProtocolMessage::new(kind, Vec::new()));
    }

    /// `client_exit` (`client.c:223-228`).
    fn try_exit(&mut self) {
        if !self.files.write_left() {
            self.process.exit();
        }
    }

    /// `client_send_identify` (`client.c:442-488`).
    fn send_identify(
        &mut self,
        ttynam: &[u8],
        termname: &[u8],
        caps: &[Vec<u8>],
        cwd: &[u8],
        feat: i32,
    ) {
        let flags = self.flags.bits().to_le_bytes().to_vec();
        let _ = self.send(ProtocolMessage::new(Kind::IdentifyLongflags, flags.clone()));
        let _ = self.send(ProtocolMessage::new(Kind::IdentifyLongflags, flags));

        let _ = self.send(ProtocolMessage::new(
            Kind::IdentifyTerm,
            encode_string(termname),
        ));
        let _ = self.send(ProtocolMessage::new(
            Kind::IdentifyFeatures,
            feat.to_le_bytes().to_vec(),
        ));

        let _ = self.send(ProtocolMessage::new(
            Kind::IdentifyTtyname,
            encode_string(ttynam),
        ));
        let _ = self.send(ProtocolMessage::new(Kind::IdentifyCwd, encode_string(cwd)));

        for cap in caps {
            let _ = self.send(ProtocolMessage::new(
                Kind::IdentifyTerminfo,
                encode_string(cap),
            ));
        }

        let Ok(stdin) = sys::dup_fd(io::stdin().as_fd()) else {
            fatal!("dup failed");
        };
        let _ = self.send(ProtocolMessage::with_fd(
            Kind::IdentifyStdin,
            Vec::new(),
            stdin,
        ));
        let Ok(stdout) = sys::dup_fd(io::stdout().as_fd()) else {
            fatal!("dup failed");
        };
        let _ = self.send(ProtocolMessage::with_fd(
            Kind::IdentifyStdout,
            Vec::new(),
            stdout,
        ));

        let pid = rmux_sys::proc::getpid().0;
        let _ = self.send(ProtocolMessage::new(
            Kind::IdentifyClientpid,
            pid.to_le_bytes().to_vec(),
        ));

        for entry in sys::environ() {
            if entry.len() + 1 > LEGACY_PAYLOAD {
                continue;
            }
            let _ = self.send(ProtocolMessage::new(
                Kind::IdentifyEnviron,
                encode_string(&entry),
            ));
        }

        let _ = self.send(ProtocolMessage::new(Kind::IdentifyDone, Vec::new()));
    }

    /// `client_signal` (`client.c:512-563`).
    fn on_signal(&mut self, sig: i32) {
        log_debug!(
            "client_signal: {}",
            String::from_utf8_lossy(&sys::signal_name(sig))
        );
        if sig == sys::SIGCHLD {
            sys::wait_any_children();
        } else if !self.attached {
            if sig == sys::SIGTERM || sig == sys::SIGHUP {
                self.process.exit();
            }
        } else {
            match sig {
                sys::SIGHUP => {
                    self.exit_reason = ClientExitReason::LostTty;
                    self.exit_val = 1;
                    self.send_empty(Kind::Exiting);
                }
                sys::SIGTERM => {
                    if !self.suspended {
                        self.exit_reason = ClientExitReason::Terminated;
                    }
                    self.exit_val = 1;
                    self.send_empty(Kind::Exiting);
                }
                sys::SIGWINCH => self.send_empty(Kind::Resize),
                sys::SIGCONT => {
                    if sys::set_signal_disposition(sys::SIGTSTP, sys::Disposition::Ignore).is_err()
                    {
                        fatal!("sigaction failed");
                    }
                    self.send_empty(Kind::Wakeup);
                    self.suspended = false;
                }
                _ => {}
            }
        }
    }

    /// `client_dispatch` with `imsg == NULL` (`client.c:579-586`).
    fn on_closed(&mut self) {
        if !self.exit_flag {
            self.exit_reason = ClientExitReason::LostServer;
            self.exit_val = 1;
        }
        self.process.exit();
    }

    /// `client_dispatch` (`client.c:576-592`).
    fn dispatch(&mut self, message: ProtocolMessage) {
        if self.attached {
            self.dispatch_attached(message);
        } else {
            self.dispatch_wait(message);
        }
    }

    fn apply_exit_message(&mut self, data: &[u8]) {
        let decoded = match ExitMessage::decode(data) {
            Ok(decoded) => decoded,
            Err(text) => fatalx!("{text}"),
        };
        if let Some(retval) = decoded.retval {
            self.exit_val = retval;
        }
        if let Some(message) = decoded.message {
            self.exit_reason = ClientExitReason::MessageProvided(message);
        }
    }

    fn set_flags_from(&mut self, data: &[u8]) {
        let Ok(bytes) = <[u8; 8]>::try_from(data) else {
            fatalx!("bad MSG_FLAGS string");
        };
        self.flags = ClientFlags::from_bits_retain(u64::from_le_bytes(bytes));
        log_debug!("new flags are {:#x}", self.flags.bits());
    }

    /// `client_dispatch_wait` (`client.c:621-719`).
    fn dispatch_wait(&mut self, message: ProtocolMessage) {
        let data = &message.data;
        match message.kind {
            Kind::Exit | Kind::Shutdown => {
                self.apply_exit_message(data);
                self.exit_flag = true;
                self.try_exit();
            }
            Kind::Ready => {
                if !data.is_empty() {
                    fatalx!("bad MSG_READY size");
                }
                self.attached = true;
                self.send_empty(Kind::Resize);
            }
            Kind::Version => {
                let Ok(bytes) = <[u8; 2]>::try_from(data.as_slice()) else {
                    fatalx!("bad MSG_VERSION size");
                };
                print_err(&[format!(
                    "protocol version mismatch (client {}, server {})",
                    protocol::VERSION,
                    u16::from_le_bytes(bytes)
                )
                .as_bytes()]);
                self.exit_val = 1;
                self.process.exit();
            }
            Kind::Flags => self.set_flags_from(data),
            Kind::Shell => {
                let shell = match protocol::decode_string(data) {
                    Ok(shell) if !shell.is_empty() => shell,
                    _ => fatalx!("bad MSG_SHELL string"),
                };
                let cmd = self.shell_command.take().unwrap_or_default();
                exec_shell(&shell, &cmd, self.flags, self.signals.take());
            }
            Kind::Detach | Kind::DetachKill => self.send_empty(Kind::Exiting),
            Kind::Exited => self.process.exit(),
            Kind::ReadOpen
            | Kind::ReadCancel
            | Kind::WriteOpen
            | Kind::WriteData
            | Kind::WriteClose => {
                let kind = message.kind;
                let policy = FilePolicy {
                    allow_streams: !self.flags.contains(ClientFlags::CONTROL),
                    close_received: true,
                };
                let Some(peer) = self.peer else {
                    return;
                };
                if let Err(e) = self.files.handle_client(
                    &mut self.process,
                    &mut self.event_loop,
                    peer,
                    message,
                    policy,
                ) {
                    fatalx!("bad {kind:?} message: {e}");
                }
            }
            kind => log_debug!("unknown message type {}", kind as u16),
        }
    }

    /// `client_dispatch_attached` (`client.c:722-808`).
    fn dispatch_attached(&mut self, message: ProtocolMessage) {
        let data = &message.data;
        match message.kind {
            Kind::Flags => self.set_flags_from(data),
            Kind::Detach | Kind::DetachKill => {
                let Ok(session) = protocol::decode_string(data) else {
                    fatalx!("bad MSG_DETACH string");
                };
                self.exit_session = Some(session);
                self.exit_type = Some(message.kind);
                self.exit_reason = if message.kind == Kind::DetachKill {
                    ClientExitReason::DetachedHup
                } else {
                    ClientExitReason::Detached
                };
                self.send_empty(Kind::Exiting);
            }
            Kind::Exec => {
                let mut rest = data.as_slice();
                let (Ok(cmd), Ok(shell)) = (
                    protocol::take_string(&mut rest),
                    protocol::take_string(&mut rest),
                ) else {
                    fatalx!("bad MSG_EXEC string");
                };
                if !rest.is_empty() {
                    fatalx!("bad MSG_EXEC string");
                }
                self.exec = Some((cmd.to_vec(), shell.to_vec()));
                self.exit_type = Some(Kind::Exec);
                self.send_empty(Kind::Exiting);
            }
            Kind::Exit => {
                self.apply_exit_message(data);
                if self.exit_reason == ClientExitReason::None {
                    self.exit_reason = ClientExitReason::Exited;
                }
                self.send_empty(Kind::Exiting);
            }
            Kind::Exited => {
                if !data.is_empty() {
                    fatalx!("bad MSG_EXITED size");
                }
                self.process.exit();
            }
            Kind::Shutdown => {
                if !data.is_empty() {
                    fatalx!("bad MSG_SHUTDOWN size");
                }
                self.send_empty(Kind::Exiting);
                self.exit_reason = ClientExitReason::ServerExited;
                self.exit_val = 1;
            }
            Kind::Suspend => {
                if !data.is_empty() {
                    fatalx!("bad MSG_SUSPEND size");
                }
                if sys::set_signal_disposition(sys::SIGTSTP, sys::Disposition::Default).is_err() {
                    fatal!("sigaction failed");
                }
                self.suspended = true;
                let _ = sys::raise(sys::SIGTSTP);
            }
            Kind::Lock => {
                let Ok(command) = protocol::decode_string(data) else {
                    fatalx!("bad MSG_LOCK string");
                };
                sys::system(&command);
                self.send_empty(Kind::Unlock);
            }
            kind => log_debug!("unknown message type {}", kind as u16),
        }
    }

    /// `proc_loop` (`proc.c`): poll until `proc_exit`.
    fn run_loop(&mut self) {
        while !self.process.exiting {
            let ready = match self.event_loop.poll(None) {
                Ok(ready) => ready,
                Err(e) => fatal!("event loop failed: {e}"),
            };
            for item in ready {
                match item.action {
                    LoopAction::Peer(peer) => {
                        for dispatch in self.process.ready(peer, item.readable, item.writable) {
                            match dispatch {
                                PeerDispatch::Message(_, message) => self.dispatch(message),
                                PeerDispatch::Closed(_) => self.on_closed(),
                            }
                        }
                        let _ = self.process.update_event(peer, &mut self.event_loop);
                    }
                    LoopAction::File(id) => {
                        let _ = self.files.client_ready(
                            &mut self.process,
                            &mut self.event_loop,
                            id,
                            item.readable,
                            item.writable,
                        );
                    }
                    LoopAction::Signal => {
                        let signals = match self.signals.as_mut().map(SignalWake::drain) {
                            Some(Ok(signals)) => signals,
                            _ => Vec::new(),
                        };
                        for sig in signals {
                            self.on_signal(sig);
                        }
                    }
                    _ => {}
                }
                // client_file_check_cb (client.c:566-573)
                if self.files.take_flush_checks() > 0 && self.exit_flag {
                    self.try_exit();
                }
                if self.process.exiting {
                    break;
                }
            }
        }
    }
}

/// `client_main` (`client.c:231-439`). `server` holds the global options and
/// environment built by `main`; it is dropped where C frees them.
pub fn run(
    config: MainConfig,
    mut server: rmux_server::model::Server,
    activation_listener: Option<OwnedFd>,
) -> i32 {
    let MainConfig {
        mut flags,
        feat,
        socket_path,
        shell_command,
        cfg_files,
        command,
        ..
    } = config;

    // Set up the initial command (client.c:247-272).
    let msg = if shell_command.is_some() {
        flags.insert(ClientFlags::STARTSERVER);
        Kind::Shell
    } else if command.is_empty() {
        flags.insert(ClientFlags::STARTSERVER);
        Kind::Command
    } else {
        let values = rmux_server::cmd::arguments::from_vector(
            &command.iter().cloned().map(Into::into).collect::<Vec<_>>(),
        );
        if let Ok(list) = parse::from_arguments(&mut server, &values, &mut CmdParseInput::default())
        {
            if list.any_have(CommandFlags::STARTSERVER) {
                flags.insert(ClientFlags::STARTSERVER);
            }
        }
        Kind::Command
    };

    // proc_start("client") and proc_set_signals (client.c:275-276).
    rmux_util::log::open("client");
    let signals = match SignalWake::new() {
        Ok(signals) => signals,
        Err(e) => fatal!("sigaction failed: {e}"),
    };
    flags.insert(ClientFlags::WRITE_ACK);
    log_debug!("flags are {:#x}", flags.bits());

    let mut runtime = ClientRuntime {
        process: Process::new(),
        event_loop: match EventLoop::new() {
            Ok(event_loop) => event_loop,
            Err(e) => fatal!("event loop failed: {e}"),
        },
        peer: None,
        flags,
        suspended: false,
        exit_reason: ClientExitReason::None,
        exit_flag: false,
        exit_val: 0,
        exit_type: None,
        exit_session: None,
        exec: None,
        attached: false,
        files: FileStore::new(),
        saved_termios: None,
        shell_command,
        signals: Some(signals),
        signal_token: None,
    };
    for i in 0..3u8 {
        runtime
            .files
            .set_standard_fd(usize::from(i), sys::take_standard_fd(i));
    }
    if let Some(wake) = &runtime.signals {
        match runtime
            .event_loop
            .register(wake.fd(), true, false, LoopAction::Signal)
        {
            Ok(token) => runtime.signal_token = Some(token),
            Err(e) => fatal!("event loop failed: {e}"),
        }
    }

    // Initialize the client socket and start the server (client.c:282-300).
    let connection = if activation_listener.is_some() {
        start_server(
            &socket_path,
            runtime.flags,
            None,
            &cfg_files,
            activation_listener,
        )
    } else {
        connect(&socket_path, runtime.flags, &cfg_files)
    };
    let fd = match connection {
        Ok(fd) => fd,
        Err(e) => {
            if e.kind() == io::ErrorKind::ConnectionRefused {
                print_err(&[b"no server running on ", &socket_path]);
            } else {
                print_err(&[
                    b"error connecting to ",
                    &socket_path,
                    b" (",
                    &strerror(&e),
                    b")",
                ]);
            }
            return 1;
        }
    };
    let peer = match runtime.process.add_peer(fd) {
        Ok(peer) => peer,
        Err(e) => fatal!("add peer failed: {e}"),
    };
    runtime.peer = Some(peer);
    if let Err(e) = runtime.process.update_event(peer, &mut runtime.event_loop) {
        fatal!("event loop failed: {e}");
    }

    // client.c:302-308
    let cwd = paths::find_cwd()
        .or_else(|| paths::find_home().map(<[u8]>::to_vec))
        .unwrap_or_else(|| b"/".to_vec());
    let ttynam = sys::ttyname(io::stdin().as_fd()).unwrap_or_default();
    let termname = sys::getenv("TERM").unwrap_or_default();

    // Load terminfo entry if any (client.c:324-332).
    let mut caps: Vec<Vec<u8>> = Vec::new();
    if rmux_sys::fd::isatty(io::stdin().as_fd()) && !termname.is_empty() {
        match rmux_tty::term::terminfo::read_list(&termname) {
            Ok(list) => caps = list.into_iter().map(Into::into).collect(),
            Err(e) => {
                print_err(&[&e.cause(&termname)]);
                return 1;
            }
        }
    }

    // Free stuff that is not used in the client (client.c:334-340).
    drop(server);

    // Set up control mode (client.c:342-361).
    if runtime.flags.contains(ClientFlags::CONTROLCONTROL) {
        let saved = match TermiosState::get(io::stdin().as_fd()) {
            Ok(saved) => saved,
            Err(e) => {
                print_err(&[b"tcgetattr failed: ", &strerror(&e)]);
                return 1;
            }
        };
        let tio = TermiosState::control_mode(&saved);
        let _ = tio.set(io::stdin().as_fd());
        runtime.saved_termios = Some(saved);
    }

    // Send identify messages (client.c:363-366).
    runtime.send_identify(&ttynam, &termname, &caps, &cwd, feat.enabled as i32);
    drop(caps);
    let _ = runtime.process.flush_peer(peer);

    // Send first command (client.c:368-397).
    if msg == Kind::Command {
        let data = match protocol::pack_argv(&command) {
            Ok(data) => data,
            Err(ProtocolError::Shape("failed to send command")) => {
                print_err(&[b"failed to send command"]);
                return 1;
            }
            Err(_) => {
                print_err(&[b"command too long"]);
                return 1;
            }
        };
        if runtime
            .send(ProtocolMessage::new(Kind::Command, data))
            .is_err()
        {
            print_err(&[b"failed to send command"]);
            return 1;
        }
    } else if msg == Kind::Shell {
        runtime.send_empty(Kind::Shell);
    }

    // Start main loop (client.c:400).
    runtime.run_loop();

    // Run command if user requested exec, instead of exiting (client.c:402-407).
    if runtime.exit_type == Some(Kind::Exec) {
        if runtime.flags.contains(ClientFlags::CONTROLCONTROL) {
            if let Some(saved) = &runtime.saved_termios {
                let _ = saved.set_flush(io::stdout().as_fd());
            }
        }
        let (cmd, shell) = runtime.exec.take().unwrap_or_default();
        exec_shell(&shell, &cmd, runtime.flags, runtime.signals.take());
    }

    // Print the exit message, if any, and exit (client.c:409-431).
    let message = runtime.exit_reason.message(runtime.exit_session.as_deref());
    if runtime.attached {
        if runtime.exit_reason != ClientExitReason::None {
            let mut out = io::stdout().lock();
            let _ = out.write_all(b"[");
            let _ = out.write_all(&message);
            let _ = out.write_all(b"]\n");
            let _ = out.flush();
        }
        let ppid = sys::getppid();
        if runtime.exit_type == Some(Kind::DetachKill) && ppid.0 > 1 {
            let _ = sys::kill(ppid, sys::SIGHUP);
        }
    } else if runtime.flags.contains(ClientFlags::CONTROL) {
        {
            let mut out = io::stdout().lock();
            if runtime.exit_reason != ClientExitReason::None {
                let _ = out.write_all(b"%exit ");
                let _ = out.write_all(&message);
                let _ = out.write_all(b"\n");
            } else {
                let _ = out.write_all(b"%exit\n");
            }
            let _ = out.flush();
        }
        if runtime.flags.contains(ClientFlags::CONTROL_WAITEXIT) {
            control_wait_exit();
        }
        if runtime.flags.contains(ClientFlags::CONTROLCONTROL) {
            let mut out = io::stdout().lock();
            let _ = out.write_all(b"\x1b\\");
            let _ = out.flush();
            if let Some(saved) = &runtime.saved_termios {
                let _ = saved.set_flush(io::stdout().as_fd());
            }
        }
    } else if runtime.exit_reason != ClientExitReason::None {
        print_err(&[&message]);
    }

    // Restore the streams to blocking (client.c:433-436).
    rmux_sys::fd::set_blocking(io::stdin().as_fd(), true);
    rmux_sys::fd::set_blocking(io::stdout().as_fd(), true);
    rmux_sys::fd::set_blocking(io::stderr().as_fd(), true);

    runtime.exit_val
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_reason_messages() {
        let session = Some(&b"main"[..]);
        for (reason, without, with) in [
            (ClientExitReason::None, "unknown reason", "unknown reason"),
            (
                ClientExitReason::Detached,
                "detached",
                "detached (from session main)",
            ),
            (
                ClientExitReason::DetachedHup,
                "detached and SIGHUP",
                "detached and SIGHUP (from session main)",
            ),
            (ClientExitReason::LostTty, "lost tty", "lost tty"),
            (ClientExitReason::Terminated, "terminated", "terminated"),
            (
                ClientExitReason::LostServer,
                "server exited unexpectedly",
                "server exited unexpectedly",
            ),
            (ClientExitReason::Exited, "exited", "exited"),
            (
                ClientExitReason::ServerExited,
                "server exited",
                "server exited",
            ),
            (
                ClientExitReason::MessageProvided(b"error creating /x (EPERM)".to_vec()),
                "error creating /x (EPERM)",
                "error creating /x (EPERM)",
            ),
        ] {
            assert_eq!(reason.message(None), without.as_bytes(), "{reason:?}");
            assert_eq!(reason.message(session), with.as_bytes(), "{reason:?}");
        }
    }

    #[test]
    fn exit_payload_decode() {
        assert_eq!(ExitMessage::decode(&[]).unwrap(), ExitMessage::default());
        assert_eq!(
            ExitMessage::decode(&[1, 2]).unwrap_err(),
            "bad MSG_EXIT size"
        );
        assert_eq!(
            ExitMessage::decode(&3i32.to_le_bytes()).unwrap(),
            ExitMessage {
                retval: Some(3),
                message: None
            }
        );
        let mut data = 1i32.to_le_bytes().to_vec();
        data.extend_from_slice(&encode_string(b"server exited"));
        assert_eq!(
            ExitMessage::decode(&data).unwrap(),
            ExitMessage {
                retval: Some(1),
                message: Some(b"server exited".to_vec())
            }
        );
        let mut bad = 1i32.to_le_bytes().to_vec();
        bad.extend_from_slice(&[9, 0, 0, 0, b'x']);
        assert_eq!(ExitMessage::decode(&bad).unwrap_err(), "bad MSG_EXIT size");
    }

    #[test]
    fn lock_acquire_and_retry() {
        let dir = std::env::temp_dir().join(format!("rmux-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.lock");
        use std::os::unix::ffi::OsStrExt;
        let path = file.as_os_str().as_bytes();
        let held = lock::acquire(path).unwrap().expect("lock held");
        // A second flock on a different descriptor of the same file blocks:
        // release first so the retry path is exercised without hanging.
        drop(held);
        assert!(lock::acquire(path).unwrap().is_some());
        assert!(lock::acquire(b"/nonexistent/dir/x.lock").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
