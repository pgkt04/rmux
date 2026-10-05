// Ported from tmux input.c, tty.c, server-client.c @ 8f25579c
// Oracle-free broker acceptance: real PTYs, controlled draws, independent trees.
use super::broker;
use crate::{
    client::{Client, ClientFlags},
    ids::{ClientId, PaneId, SessionId},
    model::{Server, session, spawn::SpawnFlags, window},
    server::pane_runtime,
};
use mio::{Events, Interest, Poll, Token, unix::SourceFd};
use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
use rmux_tty::tty::{Tty, TtyHostInfo, TtyOptions};
use rmux_util::bytes::ByteString;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    os::unix::ffi::OsStrExt,
    time::Duration,
};

fn apc(verb: u8, body: &Value) -> Vec<u8> {
    let mut bytes = b"\x1b_tsp;".to_vec();
    bytes.extend([verb, b';']);
    bytes.extend(serde_json::to_vec(body).unwrap());
    bytes.extend(b"\x1b\\");
    bytes
}

fn wait_readable(fd: BorrowedFd<'_>) {
    let mut poll = Poll::new().unwrap();
    let raw = fd.as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&raw), Token(0), Interest::READABLE)
        .unwrap();
    let mut events = Events::with_capacity(4);
    poll.poll(&mut events, Some(Duration::from_secs(5)))
        .unwrap();
    assert!(!events.is_empty(), "fixture PTY readiness deadline");
}

fn write_all_fd(fd: BorrowedFd<'_>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        match rmux_sys::fd::write(fd, bytes) {
            Ok(0) => panic!("fixture PTY zero write"),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => panic!("fixture PTY write: {e}"),
        }
    }
}

#[derive(Default)]
struct Framing {
    pending: Vec<u8>,
}
#[derive(Debug)]
enum Packet {
    Message(u8, Value),
    Sentinel,
    Cells(Vec<u8>),
    ReplySentinel,
}
impl Framing {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Packet> {
        self.pending.extend_from_slice(bytes);
        let mut packets = Vec::new();
        let mut at = 0;
        while at < self.pending.len() {
            let bytes = &self.pending[at..];
            if bytes[0] != 0x1b {
                packets.push(Packet::Cells(vec![bytes[0]]));
                at += 1;
                continue;
            }
            if bytes.len() == 1 {
                break;
            }
            match bytes[1] {
                b'[' => {
                    let Some(end) = bytes[2..]
                        .iter()
                        .position(|byte| (0x40..=0x7e).contains(byte))
                    else {
                        break;
                    };
                    let sequence = &bytes[..end + 3];
                    packets.push(if sequence == b"\x1b[c" {
                        Packet::Sentinel
                    } else if sequence == b"\x1b[?1;2c" {
                        Packet::ReplySentinel
                    } else {
                        Packet::Cells(sequence.to_vec())
                    });
                    at += sequence.len();
                }
                b']' | b'P' | b'X' | b'^' => {
                    let Some(end) = bytes
                        .windows(2)
                        .position(|pair| pair == b"\x1b\\" || pair[0] == 0x07)
                    else {
                        break;
                    };
                    packets.push(Packet::Cells(
                        bytes[..end + if bytes[end] == 0x07 { 1 } else { 2 }].to_vec(),
                    ));
                    at += end + if bytes[end] == 0x07 { 1 } else { 2 };
                }
                b'_' if bytes.get(2..6) == Some(b"tsp;")
                    || bytes.len() < 6 && b"tsp;".starts_with(&bytes[2..]) =>
                {
                    if bytes.len() < 8 {
                        break;
                    }
                    let Some(end) = bytes.windows(2).position(|pair| pair == b"\x1b\\") else {
                        break;
                    };
                    assert!(bytes[6] == b';' || end > 7, "TSP APC missing separator");
                    let body = serde_json::from_slice(&bytes[8..end]).expect("complete TSP APC");
                    packets.push(Packet::Message(bytes[6], body));
                    at += end + 2;
                }
                b'_' => {
                    let Some(end) = bytes.windows(2).position(|pair| pair == b"\x1b\\") else {
                        break;
                    };
                    packets.push(Packet::Cells(bytes[..end + 2].to_vec()));
                    at += end + 2;
                }
                _ => {
                    packets.push(Packet::Cells(bytes[..2].to_vec()));
                    at += 2;
                }
            }
        }
        self.pending.drain(..at);
        packets
    }
}

fn node_mut<'a>(tree: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if tree["id"] == id {
        return Some(tree);
    }
    for child in tree
        .get_mut("c")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(node) = node_mut(child, id) {
            return Some(node);
        }
    }
    None
}

fn apply_independent(tree: &mut Value, op: &Value, focus: &mut Option<String>) {
    let args = op.as_array().unwrap();
    match args[0].as_str().unwrap() {
        "add" => {
            let parent = node_mut(tree, args[2].as_str().unwrap()).unwrap();
            assert!(args[3].is_null(), "fixture only appends children");
            parent
                .as_object_mut()
                .unwrap()
                .entry("c")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(args[4].clone());
        }
        "set" => {
            let node = node_mut(tree, args[1].as_str().unwrap()).unwrap();
            let props = node
                .as_object_mut()
                .unwrap()
                .entry("p")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap();
            for (key, value) in args[2].as_object().unwrap() {
                if value.is_null() {
                    props.remove(key);
                } else {
                    props.insert(key.clone(), value.clone());
                }
            }
        }
        "focus" => *focus = args[1].as_str().map(str::to_owned),
        "settle" | "suspend" | "resume" => {}
        verb => panic!("unimplemented fixture operation {verb}"),
    }
}

struct FakeTerminal {
    client: ClientId,
    master: OwnedFd,
    native: bool,
    framing: Framing,
    packets: Vec<(u8, Value)>,
    bytes: Vec<u8>,
    tree: Option<Value>,
    focus: Option<String>,
    frames: VecDeque<Value>,
    grid: Server,
    grid_pane: PaneId,
    opens: usize,
}

fn model() -> (Server, PaneId, SessionId) {
    let mut server = Server::new();
    let options = server.options.create(Some(server.options.global_s));
    let session = session::session_create(
        &mut server,
        session::SessionCreate {
            prefix: None,
            name: Some(b"broker-matrix".to_vec()),
            cwd: b"/".to_vec(),
            environment: crate::options::environment::Environment::default(),
            options,
            termios: None,
        },
    );
    let window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
    let pane =
        window::window_add_pane(&mut server, window, None, 10, SpawnFlags::default()).unwrap();
    server.windows.get_mut(window).unwrap().active = Some(pane);
    crate::layout::init(&mut server, window, pane);
    let link = session::session_attach(&mut server, session, window, 0).unwrap();
    session::session_set_current(&mut server, session, Some(link));
    (server, pane, session)
}

impl FakeTerminal {
    fn attach(server: &mut Server, session: Option<SessionId>, native: bool) -> Self {
        let (master, slave, _) = rmux_sys::pty::openpty().unwrap();
        rmux_sys::fd::set_blocking(master.as_fd(), false);
        let tio = rmux_sys::TermiosState::get(slave.as_fd()).unwrap();
        let mut tty = Tty::new(slave, tio, TtyHostInfo::default());
        tty.set_size(80, 25, 0, 0);
        let caps: Vec<ByteString> = [
            "am=1",
            "clear=\x1b[2J",
            "cup=\x1b[%i%p1%d;%p2%dH",
            "csr=\x1b[%i%p1%d;%p2%dr",
            "smcup=\x1b[?1049h",
            "rmcup=\x1b[?1049l",
            "cnorm=\x1b[?25h",
            "civis=\x1b[?25l",
            "sgr0=\x1b[0m",
            "colors=8",
            "AX=1",
        ]
        .into_iter()
        .map(ByteString::from)
        .collect();
        tty.open(
            &mut server.tparm,
            b"matrix",
            &caps,
            &TtyOptions::default(),
            None,
        )
        .unwrap();
        let mut client = Client::new(None, (0, 0));
        client.session = session;
        client.flags.insert(ClientFlags::ATTACHED);
        client.tty = Some(tty);
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        let (grid, grid_pane, _) = model();
        let mut terminal = Self {
            client: id,
            master,
            native,
            framing: Framing::default(),
            packets: vec![],
            bytes: vec![],
            tree: None,
            focus: None,
            frames: VecDeque::new(),
            grid,
            grid_pane,
            opens: 0,
        };
        broker::probe_client(server, id);
        terminal.pump(server);
        terminal
    }
    fn send(&self, server: &mut Server, bytes: &[u8]) {
        write_all_fd(self.master.as_fd(), bytes);
        let tty = server
            .clients
            .get_mut(self.client)
            .unwrap()
            .tty
            .as_mut()
            .unwrap();
        wait_readable(tty.fd());
        tty.on_readable();
        crate::client::tty_io::drain_input(server, self.client);
    }

    fn pump(&mut self, server: &mut Server) {
        loop {
            let count = {
                let tty = server
                    .clients
                    .get_mut(self.client)
                    .unwrap()
                    .tty
                    .as_mut()
                    .unwrap();
                if tty.out_len() == 0 {
                    break;
                }
                tty.on_writable().unwrap()
            };
            if count == 0 {
                break;
            }
            wait_readable(self.master.as_fd());
            let mut response = Vec::new();
            loop {
                let mut bytes = [0; 8192];
                match rmux_sys::fd::read(self.master.as_fd(), &mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        self.bytes.extend_from_slice(&bytes[..n]);
                        for packet in self.framing.feed(&bytes[..n]) {
                            self.receive(packet, &mut response);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("terminal read: {error}"),
                }
            }
            if !response.is_empty() {
                self.send(server, &response);
            }
        }
        assert!(
            self.framing
                .pending
                .windows(6)
                .all(|window| window != b"\x1b_tsp;"),
            "complete TSP APC left undecoded"
        );
    }

    fn receive(&mut self, packet: Packet, response: &mut Vec<u8>) {
        match packet {
            Packet::Sentinel => response.extend_from_slice(b"\x1b[?1;2c"),
            Packet::ReplySentinel => {}
            Packet::Cells(bytes) => {
                pane_runtime::pane_parse_buffer(&mut self.grid, self.grid_pane, &bytes).unwrap();
            }
            Packet::Message(verb, body) => {
                self.packets.push((verb, body.clone()));
                if !self.native {
                    assert!(
                        !matches!(verb, b'o' | b'f'),
                        "plain terminal received a native surface"
                    );
                }
                match verb {
                    b'q' if body["q"] == "hello" && self.native => {
                        response.extend(apc(b'r', &terminal_hello()))
                    }
                    b'o' => {
                        assert!(
                            self.tree.is_none(),
                            "two simultaneous outer screen surfaces"
                        );
                        assert_eq!(body["mode"], "screen");
                        self.tree = Some(json!({"id":body["id"],"k":"surface","c":[]}));
                        self.opens += 1;
                    }
                    b'x' => {
                        self.tree = None;
                        self.focus = None;
                        self.frames.clear();
                    }
                    b'f' => self.frames.push_back(body),
                    _ => {}
                }
            }
        }
    }
}

fn terminal_hello() -> Value {
    json!({"r":"hello","v":1,"term":"fixture","version":"1","kinds":["col","text","editor"],"features":["edit","undo","send","settle"],"apc":65536,"credits":2,"cell":{"w":8,"h":16},"dark":true,"reduceMotion":false})
}

impl FakeTerminal {
    fn draw(&mut self, server: &mut Server) -> Option<u64> {
        let mut last = None;
        while let Some(frame) = self.frames.pop_front() {
            let tree = self.tree.as_mut().expect("frame before open");
            assert_eq!(frame["sf"], tree["id"]);
            for op in frame["ops"].as_array().unwrap() {
                apply_independent(tree, op, &mut self.focus);
            }
            last = Some(frame);
        }
        let frame = last?;
        let sequence = frame["s"].as_u64().unwrap();
        self.send(
            server,
            &apc(b'e', &json!({"ev":"ack","sf":frame["sf"],"s":sequence})),
        );
        Some(sequence)
    }

    fn text(&mut self, id: &str) -> String {
        node_mut(self.tree.as_mut().unwrap(), id).unwrap()["p"]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn row(&self, y: u32) -> String {
        String::from_utf8(
            self.grid
                .panes
                .get(self.grid_pane)
                .unwrap()
                .base
                .grid
                .view_string_cells(0, y, 80),
        )
        .unwrap()
        .trim_end()
        .to_owned()
    }

    fn detach(&self, server: &mut Server) {
        server.clients.get_mut(self.client).unwrap().session = None;
        broker::recompute(server);
    }
}

fn program_tree(counter: u64, draft: &str) -> Value {
    json!({"id":"main","k":"col","c":[
        {"id":"transcript","k":"text","p":{"text":format!("transcript:{counter}")}},
        {"id":"counter","k":"text","p":{"text":format!("counter:{counter}")}},
        {"id":"draft","k":"editor","p":{"text":draft}}
    ]})
}

#[test]
#[ignore = "spawned explicitly as the persistent PTY pane child"]
fn pane_child_fixture() {
    if std::env::var_os("RMUX_MATRIX_CHILD").is_none() {
        return;
    }
    let stdin = io::stdin();
    let mut raw = rmux_sys::TermiosState::get(stdin.as_fd()).unwrap();
    raw.make_raw();
    raw.set(stdin.as_fd()).unwrap();
    let mut framing = Framing::default();
    let mut output = io::stdout().lock();
    let mut input = io::stdin().lock();
    let mut counter = 0_u64;
    let mut draft = String::from("unsent draft");
    let mut native = false;
    let mut epoch = 0;
    let mut sequence = 11;
    let mut keys = String::new();
    let probe = |output: &mut dyn Write, epoch: Option<u64>| {
        let mut value = json!({"q":"hello","v":[1],"app":"matrix-program","features":["edit","undo","send","rmux-reprobe"]});
        if let Some(epoch) = epoch {
            value["rmuxEpoch"] = epoch.into();
        }
        output.write_all(&apc(b'q', &value)).unwrap();
        output.write_all(b"\x1b[c").unwrap();
        output.flush().unwrap();
    };
    probe(&mut output, None);
    loop {
        let mut bytes = [0; 8192];
        let n = input.read(&mut bytes).unwrap();
        if n == 0 {
            return;
        }
        for packet in framing.feed(&bytes[..n]) {
            match packet {
                Packet::Message(b'r', body) if body["r"] == "rmux-probe" => {
                    epoch = body["epoch"].as_u64().unwrap();
                    native = body["native"] == true;
                }
                Packet::Message(b'e', body) if body["ev"] == "rmux-view" => {
                    if native {
                        output
                            .write_all(&apc(b'x', &json!({"sf":"program","keep":false})))
                            .unwrap();
                    }
                    probe(&mut output, body["epoch"].as_u64());
                }
                Packet::ReplySentinel => {
                    child_paint(&mut output, native, counter, &draft, sequence);
                    sequence += 7;
                    output.write_all(&apc(b'q', &json!({"q":"rmux-ready","epoch":epoch,"renderer":if native {"native"} else {"ansi"}}))).unwrap();
                }
                Packet::Message(b'r', body)
                    if body["r"] == "rmux-ready" && body["accepted"] == true =>
                {
                    child_checkpoint(&mut output, counter, &draft, &keys);
                }
                Packet::Message(b'q', body) if body["q"] == "fixture-work" => {
                    counter += 1;
                    if native {
                        output.write_all(&apc(b'f', &json!({"sf":"program","s":sequence,"ops":[["set","transcript",{"text":format!("transcript:{counter}")}],["set","counter",{"text":format!("counter:{counter}")}],["set","draft",{"text":draft}]]}))).unwrap();
                        sequence += 7;
                    } else {
                        child_paint(&mut output, false, counter, &draft, sequence);
                    }
                    child_checkpoint(&mut output, counter, &draft, &keys);
                }
                Packet::Message(b'q', body) if body["q"] == "fixture-stop" => return,
                Packet::Message(b'q', body) if body["q"] == "fixture-check" => {
                    child_checkpoint(&mut output, counter, &draft, &keys);
                    output
                        .write_all(&apc(b'q', &json!({"q":"fixture-barrier"})))
                        .unwrap();
                }
                Packet::Message(b'q', body) if body["q"] == "fixture-emit" => {
                    output
                        .write_all(body["bytes"].as_str().unwrap().as_bytes())
                        .unwrap();
                    child_checkpoint(&mut output, counter, &draft, &keys);
                }
                Packet::Message(b'e', body) if body["ev"] == "edit" && body["id"] == "draft" => {
                    if body["len"].as_u64() == Some(draft.encode_utf16().count() as u64) {
                        draft = body["text"].as_str().unwrap().to_owned();
                    }
                    child_checkpoint(&mut output, counter, &draft, &keys);
                }
                Packet::Cells(bytes) => {
                    keys.push_str(std::str::from_utf8(&bytes).unwrap());
                    child_checkpoint(&mut output, counter, &draft, &keys);
                }
                _ => {}
            }
        }
        output.flush().unwrap();
    }
}

fn child_paint(output: &mut dyn Write, native: bool, counter: u64, draft: &str, sequence: u64) {
    if native {
        output
            .write_all(&apc(
                b'o',
                &json!({"id":"program","mode":"inline","listen":true}),
            ))
            .unwrap();
        output.write_all(&apc(b'f', &json!({"sf":"program","s":sequence,"ops":[["add","main","program",null,program_tree(counter,draft)],["focus","draft"]]}))).unwrap();
    } else {
        write!(output, "\x1b[2J\x1b[Htranscript:{counter}\r\n\x1b[2;1Hdraft:{draft}\r\n\x1b[3;1Hcounter:{counter}").unwrap();
    }
}

fn child_checkpoint(output: &mut dyn Write, counter: u64, draft: &str, keys: &str) {
    output.write_all(&apc(b'q', &json!({"q":"fixture-state","pid":std::process::id(),"generation":1,"counter":counter,"draft":draft,"keys":keys}))).unwrap();
    output.flush().unwrap();
}

struct Fixture {
    server: Server,
    pane: PaneId,
    session: SessionId,
    pid: rmux_sys::ProcessId,
    observe: Framing,
    observations: Vec<(u8, Value)>,
    checkpoints: Vec<Value>,
    replies: Vec<(u8, Value)>,
}

impl Fixture {
    fn new() -> Self {
        let (mut server, pane, session) = model();
        let binary = std::env::current_exe().expect("test binary");
        let launch = PreparedLaunch::new(LaunchOptions {
            shell: b"/bin/sh".to_vec(),
            argv: vec![
                binary.as_os_str().as_bytes().to_vec(),
                b"--exact".to_vec(),
                b"tsp::integration_tests::pane_child_fixture".to_vec(),
                b"--ignored".to_vec(),
                b"--nocapture".to_vec(),
            ],
            environment: vec![
                b"RMUX_MATRIX_CHILD=1".to_vec(),
                b"PATH=/usr/bin:/bin".to_vec(),
            ],
            cwd: b"/".to_vec(),
            home: None,
            termios: None,
            backspace: 0x7f,
            size: Winsize {
                rows: 24,
                cols: 80,
                xpixel: 0,
                ypixel: 0,
            },
        })
        .unwrap();
        let child = launch.launch().expect("fixture child");
        let pane_state = server.panes.get_mut(pane).unwrap();
        pane_state.pid = Some(child.pid);
        pane_state.fd = Some(child.master);
        let mut fixture = Self {
            server,
            pane,
            session,
            pid: child.pid,
            observe: Framing::default(),
            observations: vec![],
            checkpoints: vec![],
            replies: vec![],
        };
        fixture.drive();
        fixture
    }

    fn drive(&mut self) {
        let fd = self
            .server
            .panes
            .get(self.pane)
            .unwrap()
            .fd
            .as_ref()
            .unwrap()
            .try_clone()
            .unwrap();
        let input = std::mem::take(&mut self.server.panes.get_mut(self.pane).unwrap().output);
        for packet in Framing::default().feed(&input) {
            if let Packet::Message(verb, body) = packet {
                self.replies.push((verb, body));
            }
        }
        if !input.is_empty() {
            write_all_fd(fd.as_fd(), &input);
        }
        let initial = self.observations.is_empty();
        if !self.observations.is_empty() {
            write_all_fd(fd.as_fd(), &apc(b'q', &json!({"q":"fixture-check"})));
        }
        wait_readable(fd.as_fd());
        let mut barrier = false;
        loop {
            let mut bytes = [0; 8192];
            match rmux_sys::fd::read(fd.as_fd(), &mut bytes) {
                Ok(0) => panic!("fixture child exited before completion"),
                Ok(n) => {
                    pane_runtime::pane_read(&mut self.server, self.pane, &bytes[..n]).unwrap();
                    for packet in self.observe.feed(&bytes[..n]) {
                        if let Packet::Message(verb, body) = packet {
                            if verb == b'q' && body["q"] == "fixture-state" {
                                self.checkpoints.push(body);
                            } else if verb == b'q' && body["q"] == "fixture-barrier" {
                                barrier = true;
                            } else {
                                self.observations.push((verb, body));
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if barrier || initial && !self.observations.is_empty() {
                        break;
                    }
                    wait_readable(fd.as_fd());
                }
                Err(error) => panic!("pane read: {error}"),
            }
        }
    }

    fn state(&self) -> &Value {
        self.checkpoints.last().expect("child checkpoint")
    }
    fn emit(&mut self, bytes: Vec<u8>) {
        let fd = self
            .server
            .panes
            .get(self.pane)
            .unwrap()
            .fd
            .as_ref()
            .unwrap();
        write_all_fd(
            fd.as_fd(),
            &apc(
                b'q',
                &json!({"q":"fixture-emit","bytes":String::from_utf8(bytes).unwrap()}),
            ),
        );
    }

    fn settle(&mut self, terminal: &mut FakeTerminal) {
        let before = self.checkpoints.len();
        for _ in 0..16 {
            terminal.pump(&mut self.server);
            self.drive();
            if self.checkpoints.len() > before
                && self
                    .server
                    .panes
                    .get(self.pane)
                    .unwrap()
                    .tsp
                    .as_ref()
                    .is_some_and(|s| s.switch.is_none())
            {
                crate::ui::redraw::redraw_screen(&mut self.server, terminal.client);
                terminal.pump(&mut self.server);
                terminal.draw(&mut self.server);
                return;
            }
        }
        panic!("child did not checkpoint: {:?}", self.observations);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(fd) = self
            .server
            .panes
            .get(self.pane)
            .and_then(|pane| pane.fd.as_ref())
        {
            let _ = rmux_sys::fd::write(fd.as_fd(), &apc(b'q', &json!({"q":"fixture-stop"})));
        }
        for _ in 0..50 {
            if rmux_sys::proc::wait_process(self.pid, true)
                .ok()
                .flatten()
                .is_some()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = rmux_sys::proc::terminate_process(self.pid);
        let _ = rmux_sys::proc::wait_process(self.pid, false);
    }
}

#[test]
fn native_child_keeps_pid_across_ansi_and_back() {
    let mut fixture = Fixture::new();
    let mut tern = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut tern);
    let pid = fixture.state()["pid"].as_u64().unwrap();
    assert_eq!(pid, fixture.pid.0 as u64);
    assert_eq!(fixture.state()["generation"], 1);
    assert!(
        tern.tree.is_some(),
        "native viewer received the program tree"
    );
    assert_eq!(tern.text("transcript"), "transcript:0");
    assert_eq!(tern.text("draft"), "unsent draft");
    let mut plain = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), false);
    fixture.settle(&mut plain);
    assert!(plain.tree.is_none(), "plain viewer stays on the ANSI grid");
    assert_eq!(plain.row(0), "transcript:0");
    assert_eq!(plain.row(1), "draft:unsent draft");
    assert_eq!(plain.row(2), "counter:0");
    assert_eq!(fixture.state()["pid"].as_u64().unwrap(), pid);
    tern.pump(&mut fixture.server);
    assert!(tern.tree.is_none(), "mixed viewers leave native rendering");
    let fd = fixture
        .server
        .panes
        .get(fixture.pane)
        .unwrap()
        .fd
        .as_ref()
        .unwrap();
    write_all_fd(fd.as_fd(), &apc(b'q', &json!({"q":"fixture-work"})));
    fixture.drive();
    crate::ui::redraw::redraw_screen(&mut fixture.server, plain.client);
    crate::ui::redraw::redraw_screen(&mut fixture.server, tern.client);
    plain.pump(&mut fixture.server);
    tern.pump(&mut fixture.server);
    assert_eq!(fixture.state()["counter"], 1);
    assert_eq!(fixture.state()["generation"], 1);
    assert_eq!(plain.row(0), "transcript:1");
    assert_eq!(tern.row(1), "draft:unsent draft");
    plain.detach(&mut fixture.server);
    fixture.settle(&mut tern);
    assert_eq!(fixture.state()["pid"].as_u64().unwrap(), pid);
    assert!(
        tern.tree.is_some(),
        "native rendering returns without restarting the child"
    );
    assert_eq!(tern.text("transcript"), "transcript:1");
    assert_eq!(tern.text("draft"), "unsent draft");
    assert_eq!(fixture.state()["generation"], 1);
}

#[test]
fn source_ack_requires_every_viewer_and_none_when_detached() {
    let mut fixture = Fixture::new();
    let mut slow = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut slow);
    fixture.drive();
    let before = fixture
        .replies
        .iter()
        .filter(|(verb, body)| *verb == b'e' && body["ev"] == "ack")
        .count();
    let mut second = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    second.pump(&mut fixture.server);
    assert!(
        second.tree.is_some() && !second.frames.is_empty(),
        "newcomer receives a snapshot"
    );
    second.draw(&mut fixture.server);
    fixture.drive();
    assert_eq!(
        fixture
            .replies
            .iter()
            .filter(|(verb, body)| *verb == b'e' && body["ev"] == "ack")
            .count(),
        before
    );
    let fd = fixture
        .server
        .panes
        .get(fixture.pane)
        .unwrap()
        .fd
        .as_ref()
        .unwrap();
    write_all_fd(fd.as_fd(), &apc(b'q', &json!({"q":"fixture-work"})));
    fixture.drive();
    broker::project_pending(&mut fixture.server, slow.client);
    broker::project_pending(&mut fixture.server, second.client);
    slow.pump(&mut fixture.server);
    second.pump(&mut fixture.server);
    second.draw(&mut fixture.server);
    fixture.drive();
    assert_eq!(
        fixture
            .replies
            .iter()
            .filter(|(verb, body)| *verb == b'e' && body["ev"] == "ack")
            .count(),
        before,
        "fast viewer cannot pay slow viewer draw debt"
    );
    slow.draw(&mut fixture.server);
    fixture.drive();
    let drawn = fixture
        .replies
        .iter()
        .filter(|(verb, body)| *verb == b'e' && body["ev"] == "ack")
        .count();
    assert_eq!(drawn, before + 1);
    slow.detach(&mut fixture.server);
    second.detach(&mut fixture.server);
    fixture.drive();
    let acks = fixture
        .replies
        .iter()
        .filter(|(verb, body)| *verb == b'e' && body["ev"] == "ack")
        .count();
    assert_eq!(
        acks, drawn,
        "no source ack is invented for an empty viewer set"
    );
}

#[test]
fn read_only_edit_is_not_forwarded() {
    let mut fixture = Fixture::new();
    let mut terminal = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut terminal);
    fixture
        .server
        .clients
        .get_mut(terminal.client)
        .unwrap()
        .flags
        .insert(ClientFlags::READONLY);
    let outer = terminal.tree.as_ref().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    terminal.send(
        &mut fixture.server,
        &apc(
            b'e',
            &json!({"ev":"edit","sf":outer,"id":"draft","len":12,"text":"changed"}),
        ),
    );
    fixture.drive();
    assert_eq!(fixture.state()["draft"], "unsent draft");
}

#[test]
fn stale_ready_and_stale_surface_are_rejected() {
    let mut fixture = Fixture::new();
    let mut terminal = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut terminal);
    fixture.emit(apc(
        b'q',
        &json!({"q":"rmux-ready","epoch":0,"renderer":"native"}),
    ));
    fixture.drive();
    fixture.drive();
    assert!(fixture.replies.iter().any(|(verb, body)| *verb == b'r'
        && body["r"] == "rmux-ready"
        && body["epoch"] == 0
        && body["accepted"] == false));
    terminal.send(
        &mut fixture.server,
        &apc(
            b'e',
            &json!({"ev":"edit","sf":"rmux:stale","id":"draft","len":12,"text":"nope"}),
        ),
    );
    fixture.drive();
    assert_eq!(fixture.state()["draft"], "unsent draft");
}

#[test]
fn overlay_and_alternate_screen_close_projection() {
    let mut fixture = Fixture::new();
    let mut terminal = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut terminal);
    fixture.emit(apc(
        b'o',
        &json!({"id":"overlay","mode":"screen","listen":true}),
    ));
    fixture.drive();
    terminal.pump(&mut fixture.server);
    broker::pane_alternate(&mut fixture.server, fixture.pane, true);
    terminal.pump(&mut fixture.server);
    assert!(
        terminal.tree.is_none()
            || fixture
                .server
                .clients
                .get(terminal.client)
                .unwrap()
                .tsp
                .projection
                .is_none()
    );
}

#[test]
fn held_input_keeps_order_and_control_is_not_paused() {
    let mut fixture = Fixture::new();
    let mut terminal = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut terminal);
    fixture
        .server
        .panes
        .get_mut(fixture.pane)
        .unwrap()
        .tsp
        .as_mut()
        .unwrap()
        .renderer = broker::Renderer::Switching;
    let prefix = b"PREFIX-".to_vec();
    let paste = vec![b'P'; 70 * 1024];
    crate::model::pane::pane_input_bytes(
        &mut fixture.server,
        fixture.pane,
        Some(terminal.client),
        &prefix,
    )
    .unwrap();
    crate::model::pane::pane_input_bytes(
        &mut fixture.server,
        fixture.pane,
        Some(terminal.client),
        &paste,
    )
    .unwrap();
    broker::client_read_bound(&mut fixture.server, terminal.client);
    assert_eq!(
        fixture
            .server
            .panes
            .get(fixture.pane)
            .unwrap()
            .tsp
            .as_ref()
            .unwrap()
            .held_bytes,
        super::input::INPUT_LIMIT
    );
    assert!(
        !fixture
            .server
            .clients
            .get(terminal.client)
            .unwrap()
            .tty
            .as_ref()
            .unwrap()
            .wants_read()
    );
    let mut control = Client::new(None, (0, 0));
    control.session = Some(fixture.session);
    control
        .flags
        .insert(ClientFlags::CONTROL | ClientFlags::ATTACHED);
    let control = fixture.server.clients.insert(control).unwrap();
    fixture.server.client_order.push_back(control);
    broker::client_read_bound(&mut fixture.server, control);
    fixture
        .server
        .panes
        .get_mut(fixture.pane)
        .unwrap()
        .tsp
        .as_mut()
        .unwrap()
        .renderer = broker::Renderer::Ansi;
    broker::release_input(&mut fixture.server, fixture.pane);
    let output = &fixture.server.panes.get(fixture.pane).unwrap().output;
    assert!(output.windows(prefix.len()).any(|window| window == prefix));
    assert!(
        output
            .windows(4)
            .position(|window| window == b"PPPP")
            .unwrap()
            > output
                .windows(prefix.len())
                .position(|window| window == prefix)
                .unwrap()
    );
    assert!(output.ends_with(&[prefix.as_slice(), paste.as_slice()].concat()));
}
