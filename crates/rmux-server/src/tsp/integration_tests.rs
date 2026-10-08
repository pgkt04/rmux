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

fn take_node(tree: &mut Value, id: &str) -> Option<Value> {
    let children = tree.get_mut("c").and_then(Value::as_array_mut)?;
    if let Some(index) = children.iter().position(|child| child["id"] == id) {
        return Some(children.remove(index));
    }
    children.iter_mut().find_map(|child| take_node(child, id))
}

fn insert_node(tree: &mut Value, parent: &str, before: &Value, node: Value) {
    let children = node_mut(tree, parent)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .entry("c")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .unwrap();
    let index = before.as_str().map_or(children.len(), |id| {
        children.iter().position(|child| child["id"] == id).unwrap()
    });
    children.insert(index, node);
}

fn apply_independent(tree: &mut Value, op: &Value, focus: &mut Option<String>) {
    let args = op.as_array().unwrap();
    match args[0].as_str().unwrap() {
        "add" => insert_node(tree, args[2].as_str().unwrap(), &args[3], args[4].clone()),
        "move" => {
            let node = take_node(tree, args[1].as_str().unwrap()).unwrap();
            insert_node(tree, args[2].as_str().unwrap(), &args[3], node);
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
        "del" => {
            assert!(take_node(tree, args[1].as_str().unwrap()).is_some());
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
    hello: Value,
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
        Self::attach_with_hello(server, session, native, terminal_hello())
    }

    fn attach_with_hello(
        server: &mut Server,
        session: Option<SessionId>,
        native: bool,
        hello: Value,
    ) -> Self {
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
            "kmous=\x1b[M",
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
            hello,
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
                        response.extend(apc(b'r', &self.hello))
                    }
                    b'o' => {
                        assert!(
                            self.tree.is_none(),
                            "two simultaneous outer screen surfaces"
                        );
                        match body["mode"].as_str() {
                            Some("inline") => assert!(
                                !self.alternate(),
                                "an inline surface opened on the alternate screen"
                            ),
                            Some("screen") => {}
                            mode => panic!("outer surface mode {mode:?}"),
                        }
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

    fn alternate(&self) -> bool {
        self.grid
            .panes
            .get(self.grid_pane)
            .unwrap()
            .base
            .is_alternate()
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
                            .write_all(&apc(b'x', &json!({"id":"program","keep":false})))
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

fn stock_native() -> (Server, PaneId, FakeTerminal) {
    stock_native_with_hello(terminal_hello())
}

fn stock_native_with_hello(hello: Value) -> (Server, PaneId, FakeTerminal) {
    stock_native_with_program(hello, json!([]))
}

fn stock_native_with_program(hello: Value, features: Value) -> (Server, PaneId, FakeTerminal) {
    let (mut server, pane, session) = model();
    let mut terminal = FakeTerminal::attach_with_hello(&mut server, Some(session), true, hello);
    crate::client::tick::reset_state(&mut server, terminal.client);
    terminal.pump(&mut server);
    assert!(
        terminal
            .grid
            .panes
            .get(terminal.grid_pane)
            .unwrap()
            .base
            .mode
            .intersects(rmux_emu::screen::ScreenMode::ALL_MOUSE_MODES)
    );
    let mut bytes = apc(
        b'q',
        &json!({"q":"hello","v":[1],"app":"omp","features":features}),
    );
    bytes.extend(apc(
        b'o',
        &json!({"id":"program","mode":"inline","title":"omp"}),
    ));
    bytes.extend(apc(
        b'f',
        &json!({"sf":"program","s":1,"ops":[["add","transcript","program",null,{"id":"transcript","k":"text","p":{"text":"retained transcript"}}]]}),
    ));
    pane_runtime::pane_parse_buffer(&mut server, pane, &bytes).unwrap();
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(terminal.text("transcript"), "retained transcript");
    crate::cmd::key_bindings::init(&mut server).unwrap();
    crate::cmd::queue::next(&mut server, None);
    crate::client::lifecycle::update_offset(&mut server, terminal.client);
    (server, pane, terminal)
}

fn native_status_terminal() -> (Server, PaneId, FakeTerminal) {
    stock_native_with_program(
        json!({"r":"hello","v":1,"kinds":["col","text","editor","input","status","seg"],
        "features":["edit","undo","send","settle","dock"],"credits":2}),
        json!(["edit", "undo", "send"]),
    )
}

fn native_keys(server: &mut Server, terminal: &mut FakeTerminal, keys: &[u8]) {
    if !keys.is_empty() {
        terminal.send(server, keys);
    }
    crate::cmd::queue::next(server, Some(terminal.client));
    broker::recompute(server);
    crate::tsp::status_bar::redraw(server, terminal.client);
    terminal.pump(server);
    terminal.draw(server);
}

fn native_event(server: &mut Server, terminal: &mut FakeTerminal, mut event: Value) {
    event["sf"] = terminal.tree.as_ref().unwrap()["id"].clone();
    native_keys(server, terminal, &apc(b'e', &event));
}

fn native_prompt_props(terminal: &mut FakeTerminal) -> Value {
    let id = terminal.focus.as_ref().unwrap().clone();
    assert!(id.starts_with("rmux:prompt:"));
    node_mut(terminal.tree.as_mut().unwrap(), &id).unwrap()["p"].clone()
}

fn native_app_draft(server: &mut Server, pane: PaneId, terminal: &mut FakeTerminal) {
    pane_runtime::pane_parse_buffer(server, pane, &apc(b'f', &json!({
        "sf":"program", "s":2, "ops":[
            ["add","draft","program",null,{"id":"draft","k":"editor","p":{"text":"app draft"}}],
            ["focus","draft"]
        ]
    }))).unwrap();
    terminal.pump(server);
    terminal.draw(server);
    assert_eq!(terminal.focus.as_deref(), Some("draft"));
}

#[test]
fn native_prompt_apc_edit_undo_send_reject_stale_and_restore_app_focus() {
    let (mut server, pane, mut terminal) = native_status_terminal();
    native_app_draft(&mut server, pane, &mut terminal);
    let window = server.panes.get(pane).unwrap().window;
    native_keys(&mut server, &mut terminal, b"\x02,");
    let stale = terminal.focus.clone().unwrap();
    native_keys(&mut server, &mut terminal, b"\x03");
    assert_eq!(terminal.focus.as_deref(), Some("draft"));
    native_keys(&mut server, &mut terminal, b"\x02,");
    native_keys(&mut server, &mut terminal, b"\x15");
    let id = terminal.focus.clone().unwrap();
    native_event(&mut server, &mut terminal, json!({"ev":"undo","id":stale}));
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":stale,"from":0,"to":0,"text":"stale","cursor":5,"len":0}),
    );
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"send","id":stale,"text":"stale"}),
    );
    assert_eq!(native_prompt_props(&mut terminal)["text"], "");
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":"draft","from":0,"to":9,"text":"wrong","cursor":5,"len":9}),
    );
    assert_eq!(terminal.text("draft"), "app draft");
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":0,"to":0,"text":"foo\n","cursor":4,"len":0}),
    );
    let props = native_prompt_props(&mut terminal);
    assert_eq!(props["text"], "foo");
    assert_eq!(props["cursor"], 3);
    assert!(props.get("anchor").is_none_or(Value::is_null));
    native_event(&mut server, &mut terminal, json!({"ev":"undo","id":id}));
    assert_eq!(native_prompt_props(&mut terminal)["text"], "");
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":0,"to":0,"text":"a😀b","cursor":4,"len":0}),
    );
    node_mut(terminal.tree.as_mut().unwrap(), &id).unwrap()["p"]["text"] = "host-diverged".into();
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":2,"to":3,"text":"split","cursor":6,"len":4}),
    );
    assert_eq!(native_prompt_props(&mut terminal)["text"], "a😀b");
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":1,"to":3,"text":"bye\nnow","cursor":8,"len":4}),
    );
    let props = native_prompt_props(&mut terminal);
    assert_eq!(props["text"], "abyenowb");
    assert_eq!(props["cursor"], 7);
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":0,"to":0,"text":"stale","cursor":5,"len":4}),
    );
    assert_eq!(native_prompt_props(&mut terminal)["text"], "abyenowb");
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"send","id":id,"text":"host\n-name"}),
    );
    assert_eq!(server.windows.get(window).unwrap().name, b"host-name");
    assert!(
        !server
            .clients
            .get(terminal.client)
            .unwrap()
            .prompt
            .is_some()
    );
    assert_eq!(terminal.focus.as_deref(), Some("draft"));
    assert_eq!(terminal.text("draft"), "app draft");
    assert_eq!(terminal.text("transcript"), "retained transcript");
    native_keys(&mut server, &mut terminal, b"\x02,");
    native_keys(&mut server, &mut terminal, b"\x15");
    let id = terminal.focus.clone().unwrap();
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":0,"to":0,"text":"foo\n","cursor":4,"len":0}),
    );
    native_keys(&mut server, &mut terminal, b"\r");
    assert_eq!(server.windows.get(window).unwrap().name, b"foo");
    assert_eq!(terminal.focus.as_deref(), Some("draft"));
}

#[test]
fn native_message_keeps_readonly_prompt_focus_and_multiline_order() {
    let (mut server, pane, mut terminal) = native_status_terminal();
    native_app_draft(&mut server, pane, &mut terminal);
    let session = server
        .clients
        .get(terminal.client)
        .unwrap()
        .session
        .unwrap();
    let options = server.sessions.get(session).unwrap().options;
    server.options.set_number_value(options, b"status", 3);
    crate::ui::status::status_update_cache(&mut server, session);
    server.options.set_number_value(options, b"message-line", 1);
    server
        .clients
        .get_mut(terminal.client)
        .unwrap()
        .flags
        .insert(ClientFlags::REDRAWSTATUS);
    native_keys(&mut server, &mut terminal, b"");
    native_keys(&mut server, &mut terminal, b"\x02,");
    native_keys(&mut server, &mut terminal, b"\x15keep");
    let id = terminal.focus.clone().unwrap();
    let order = |terminal: &mut FakeTerminal| -> Vec<String> {
        node_mut(terminal.tree.as_mut().unwrap(), "rmux:bar").unwrap()["c"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        order(&mut terminal),
        vec!["rmux:bar:0".to_owned(), id.clone(), "rmux:bar:2".to_owned()]
    );
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":4,"to":4,"text":"!","cursor":5,"len":4}),
    );
    assert_eq!(
        order(&mut terminal),
        vec!["rmux:bar:0".to_owned(), id.clone(), "rmux:bar:2".to_owned()]
    );
    crate::ui::status::status_message_set(
        &mut server,
        Some(terminal.client),
        0,
        true,
        true,
        false,
        b"notice",
    );
    crate::tsp::status_bar::redraw(&mut server, terminal.client);
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(terminal.focus.as_deref(), Some(id.as_str()));
    assert_eq!(native_prompt_props(&mut terminal)["readonly"], true);
    assert_eq!(native_prompt_props(&mut terminal)["text"], "keep!");
    assert_eq!(
        order(&mut terminal),
        vec![
            "rmux:bar:0".to_owned(),
            id.clone(),
            "rmux:bar:1".to_owned(),
            "rmux:bar:2".to_owned()
        ]
    );
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"send","id":id,"text":"blocked"}),
    );
    assert_eq!(native_prompt_props(&mut terminal)["text"], "keep!");
    assert_eq!(terminal.text("draft"), "app draft");
    crate::ui::status::status_message_clear(&mut server, terminal.client);
    native_keys(&mut server, &mut terminal, b"");
    assert_eq!(native_prompt_props(&mut terminal)["readonly"], false);
    assert_eq!(
        order(&mut terminal),
        vec!["rmux:bar:0".to_owned(), id, "rmux:bar:2".to_owned()]
    );
    native_keys(&mut server, &mut terminal, b"\x03");
    assert_eq!(terminal.focus.as_deref(), Some("draft"));
    assert_eq!(
        server
            .clients
            .get(terminal.client)
            .unwrap()
            .status
            .references,
        0
    );
    assert!(
        server
            .clients
            .get(terminal.client)
            .unwrap()
            .status
            .active
            .is_none()
    );
}

#[test]
fn native_vi_command_prompt_rejects_host_edit_and_keeps_raw_vi_keys() {
    let (mut server, _, mut terminal) = native_status_terminal();
    let session = server
        .clients
        .get(terminal.client)
        .unwrap()
        .session
        .unwrap();
    let options = server.sessions.get(session).unwrap().options;
    server.options.set_number_value(options, b"status-keys", 1);
    native_keys(&mut server, &mut terminal, b"\x02,");
    native_keys(&mut server, &mut terminal, b"\x15abc");
    terminal.send(&mut server, b"\x1b");
    crate::client::tty_io::on_timer(&mut server, terminal.client, rmux_tty::tty::TtyTimer::Key);
    native_keys(&mut server, &mut terminal, b"h");
    let props = native_prompt_props(&mut terminal);
    assert_eq!(props["mode"], "COMMAND");
    assert_eq!(props["readonly"], true);
    let id = terminal.focus.clone().unwrap();
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"edit","id":id,"from":0,"to":3,"text":"wrong","cursor":5,"len":3}),
    );
    assert_eq!(native_prompt_props(&mut terminal)["text"], "abc");
    native_keys(&mut server, &mut terminal, b"xiZ");
    let props = native_prompt_props(&mut terminal);
    assert_eq!(props["text"], "aZc");
    assert_eq!(props["readonly"], false);
    native_keys(&mut server, &mut terminal, b"\x03");
}

#[test]
fn native_empty_backspace_exit_prompt_uses_raw_keys_until_text_exists() {
    let (mut server, _, mut terminal) = native_status_terminal();
    native_keys(&mut server, &mut terminal, b"\x02:");
    native_keys(
        &mut server,
        &mut terminal,
        b"command-prompt -e 'display-message %%'\r",
    );
    assert_eq!(native_prompt_props(&mut terminal)["readonly"], true);
    native_keys(&mut server, &mut terminal, b"a");
    assert_eq!(native_prompt_props(&mut terminal)["text"], "a");
    assert_eq!(native_prompt_props(&mut terminal)["readonly"], false);
    native_keys(&mut server, &mut terminal, b"\x15");
    assert_eq!(native_prompt_props(&mut terminal)["readonly"], true);
    native_keys(&mut server, &mut terminal, b"\x7f");
    assert!(
        !server
            .clients
            .get(terminal.client)
            .unwrap()
            .prompt
            .is_some()
    );
}

#[test]
fn native_send_advances_multianswer_command_prompt_like_enter() {
    let (mut server, pane, mut terminal) = native_status_terminal();
    native_keys(&mut server, &mut terminal, b"\x02:");
    native_keys(
        &mut server,
        &mut terminal,
        b"command-prompt -p 'first,second' -I ',seed' 'select-pane -T %1-%2'\r",
    );
    let id = terminal.focus.clone().unwrap();
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"send","id":id,"text":"one"}),
    );
    assert_eq!(terminal.focus.as_deref(), Some(id.as_str()));
    assert_eq!(native_prompt_props(&mut terminal)["text"], "seed");
    native_event(
        &mut server,
        &mut terminal,
        json!({"ev":"send","id":id,"text":"two"}),
    );
    assert_eq!(server.panes.get(pane).unwrap().base.title, b"one-two");
    assert!(
        !server
            .clients
            .get(terminal.client)
            .unwrap()
            .prompt
            .is_some()
    );
}

#[test]
fn generic_program_without_edit_keeps_original_hello_and_raw_prompt_keys() {
    let hello = json!({"r":"hello","v":1,"kinds":["col","text","editor","input","status","seg"],"features":["edit","undo","send","dock"],"credits":2});
    let (mut server, pane, mut terminal) = stock_native_with_program(hello, json!([]));
    assert_eq!(
        server
            .clients
            .get(terminal.client)
            .unwrap()
            .tsp
            .projection_hello
            .as_ref()
            .unwrap()["features"],
        json!([])
    );
    native_keys(&mut server, &mut terminal, b"\x02,");
    native_keys(&mut server, &mut terminal, b"\x15raw");
    assert_eq!(native_prompt_props(&mut terminal)["readonly"], true);
    let window = server.panes.get(pane).unwrap().window;
    native_keys(&mut server, &mut terminal, b"\r");
    assert_eq!(server.windows.get(window).unwrap().name, b"raw");
}

#[test]
fn native_prefix_rename_submit_cancel_and_command_keep_transcript() {
    let (mut server, pane, mut terminal) = native_status_terminal();
    let window = server.panes.get(pane).unwrap().window;
    let session = server
        .clients
        .get(terminal.client)
        .unwrap()
        .session
        .unwrap();
    let original_surface = terminal.tree.as_ref().unwrap()["id"].clone();
    for (prefix, name) in [
        (b',', b"native-window".as_slice()),
        (b'$', b"native-session"),
        (b'T', b"native-pane"),
    ] {
        native_keys(&mut server, &mut terminal, &[2, prefix]);
        assert!(
            server
                .clients
                .get(terminal.client)
                .unwrap()
                .prompt
                .is_some()
        );
        assert_eq!(terminal.tree.as_ref().unwrap()["id"], original_surface);
        assert!(
            terminal
                .focus
                .as_ref()
                .is_some_and(|id| id.starts_with("rmux:prompt:"))
        );
        let mut keys = vec![21];
        keys.extend(name);
        keys.push(b'\r');
        native_keys(&mut server, &mut terminal, &keys);
        assert!(
            !server
                .clients
                .get(terminal.client)
                .unwrap()
                .prompt
                .is_some()
        );
        assert_eq!(terminal.text("transcript"), "retained transcript");
    }
    assert_eq!(server.windows.get(window).unwrap().name, b"native-window");
    assert_eq!(
        server.sessions.get(session).unwrap().name,
        b"native-session"
    );
    assert_eq!(server.panes.get(pane).unwrap().base.title, b"native-pane");
    native_keys(&mut server, &mut terminal, b"\x02,");
    native_keys(&mut server, &mut terminal, b"\x15discarded\x03");
    assert_eq!(server.windows.get(window).unwrap().name, b"native-window");
    native_keys(&mut server, &mut terminal, b"\x02:");
    native_keys(
        &mut server,
        &mut terminal,
        b"select-pane -T command-title\r",
    );
    assert_eq!(server.panes.get(pane).unwrap().base.title, b"command-title");
    assert_eq!(terminal.tree.as_ref().unwrap()["id"], original_surface);
    assert_eq!(terminal.text("transcript"), "retained transcript");
}

#[test]
fn native_open_releases_cell_mouse_capture_without_resize() {
    let (mut server, _, mut terminal) = stock_native();
    let mouse_captured = |terminal: &FakeTerminal| {
        terminal
            .grid
            .panes
            .get(terminal.grid_pane)
            .unwrap()
            .base
            .mode
            .intersects(rmux_emu::screen::ScreenMode::ALL_MOUSE_MODES)
    };
    assert!(!mouse_captured(&terminal));
    terminal.send(&mut server, b"\x02[");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    broker::recompute(&mut server);
    crate::client::tick::reset_state(&mut server, terminal.client);
    terminal.pump(&mut server);
    assert!(mouse_captured(&terminal));
    terminal.send(&mut server, b"q");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    broker::recompute(&mut server);
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert!(!mouse_captured(&terminal));
}

#[test]
fn stock_native_wheel_keeps_view_and_prefix_detach_works() {
    let (mut server, pane, mut terminal) = stock_native();
    terminal.send(&mut server, b"\x1b[<64;10;10M");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    terminal.pump(&mut server);
    assert!(
        terminal.tree.is_some(),
        "scrolling must not close the native view"
    );
    assert!(server.panes.get(pane).unwrap().modes.is_empty());
    terminal.send(&mut server, b"\x02d");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    assert!(
        server
            .clients
            .get(terminal.client)
            .unwrap()
            .flags
            .contains(ClientFlags::EXIT)
    );
}

#[test]
fn stock_native_copy_mode_can_cancel_and_detach() {
    let (mut server, pane, mut terminal) = stock_native();
    terminal.send(&mut server, b"\x02[");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    broker::recompute(&mut server);
    terminal.pump(&mut server);
    assert_eq!(
        server.panes.get(pane).unwrap().modes.first().unwrap().name,
        b"copy-mode"
    );
    assert!(terminal.tree.is_none());
    terminal.send(&mut server, b"q");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    broker::recompute(&mut server);
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert!(server.panes.get(pane).unwrap().modes.is_empty());
    assert_eq!(terminal.text("transcript"), "retained transcript");
    terminal.send(&mut server, b"\x02d");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    assert!(
        server
            .clients
            .get(terminal.client)
            .unwrap()
            .flags
            .contains(ClientFlags::EXIT)
    );
}

#[test]
fn scrolling_native_surface_offscreen_keeps_visibility_events_routable() {
    let (mut server, _, mut terminal) = stock_native();
    let outer = terminal.tree.as_ref().unwrap()["id"].clone();
    terminal.send(
        &mut server,
        &apc(b'e', &json!({"ev":"visible","sf":outer,"visible":false})),
    );
    terminal.pump(&mut server);
    assert!(!server.clients.get(terminal.client).unwrap().tsp.visible);
    assert!(
        terminal.tree.is_some(),
        "an offscreen surface must stay open"
    );
    terminal.send(
        &mut server,
        &apc(b'e', &json!({"ev":"visible","sf":outer,"visible":true})),
    );
    terminal.pump(&mut server);
    assert!(server.clients.get(terminal.client).unwrap().tsp.visible);
    assert_eq!(terminal.text("transcript"), "retained transcript");
}

#[test]
fn app_prompt_markers_during_text_view_preserve_native_recovery() {
    let mut fixture = Fixture::new();
    let mut native = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut native);
    let pid = fixture.state()["pid"].clone();
    let mut plain = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), false);
    fixture.settle(&mut plain);
    assert_eq!(
        fixture
            .server
            .panes
            .get(fixture.pane)
            .unwrap()
            .tsp
            .as_ref()
            .unwrap()
            .renderer,
        broker::Renderer::Ansi
    );
    fixture
        .emit(b"\x1b]133;A\x07user message\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;0\x07".to_vec());
    fixture.drive();
    assert!(
        fixture
            .server
            .panes
            .get(fixture.pane)
            .unwrap()
            .tsp
            .as_ref()
            .unwrap()
            .registered,
        "foreground application prompt zones are not a shell handoff"
    );
    plain.detach(&mut fixture.server);
    fixture.settle(&mut native);
    assert_eq!(fixture.state()["pid"], pid);
    assert_eq!(native.text("draft"), "unsent draft");
    assert_eq!(native.text("transcript"), "transcript:0");
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
    assert!(
        !tern.alternate(),
        "the inline projection sits on the main screen"
    );
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
    assert!(
        tern.alternate(),
        "the cell view is back on the alternate screen"
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
    assert!(!tern.alternate());
    assert_eq!(tern.text("draft"), "unsent draft");
    assert_eq!(fixture.state()["generation"], 1);
}

#[test]
fn detach_waits_for_terminal_reply_barrier_before_shell_handoff() {
    let (mut server, pane, terminal) = stock_native();
    let outer = terminal.tree.as_ref().unwrap()["id"].clone();
    server.panes.get_mut(pane).unwrap().output.clear();
    crate::client::lifecycle::detach(&mut server, terminal.client, false);
    crate::client::dispatch::on_message(
        &mut server,
        terminal.client,
        crate::server::protocol::ProtocolMessage::new(
            crate::server::protocol::ProtocolMessageKind::Exiting,
            Vec::new(),
        ),
    );
    let tty = server
        .clients
        .get_mut(terminal.client)
        .unwrap()
        .tty
        .as_mut()
        .unwrap();
    while tty.out_len() != 0 {
        tty.on_writable().unwrap();
    }
    let mode = rmux_sys::TermiosState::get(tty.fd()).unwrap();
    assert_eq!(mode.lflag() & (libc::ECHO | libc::ICANON), 0);
    assert!(tty.wants_read(), "delayed replies still belong to rmux");
    server.panes.get_mut(pane).unwrap().output.clear();
    let mut replies = apc(b'e', &json!({"ev":"ack","sf":outer,"s":2}));
    let tail = replies.split_off(17);
    terminal.send(&mut server, &replies);
    assert!(
        server
            .clients
            .get(terminal.client)
            .unwrap()
            .tty
            .as_ref()
            .unwrap()
            .stopping()
    );
    replies = tail;
    replies.extend_from_slice(b"\x1b[?1;2c");
    terminal.send(&mut server, &replies);
    let tty = server
        .clients
        .get(terminal.client)
        .unwrap()
        .tty
        .as_ref()
        .unwrap();
    assert!(!tty.flags().contains(rmux_tty::tty::TtyFlags::OPENED));
    assert!(server.panes.get(pane).unwrap().output.is_empty());
}

#[test]
fn stopping_the_tty_closes_an_inline_projection_first() {
    let mut fixture = Fixture::new();
    let mut tern = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut tern);
    assert!(tern.tree.is_some());
    let server = &mut fixture.server;
    server
        .clients
        .get_mut(tern.client)
        .unwrap()
        .tty
        .as_mut()
        .unwrap()
        .stop(&mut server.tparm, &TtyOptions::default());
    tern.pump(&mut fixture.server);
    assert!(tern.tree.is_none(), "Tern got the close before the restore");
    assert!(!tern.alternate(), "the restore leaves the alternate screen");
}

#[test]
fn a_program_that_leaves_without_closing_gives_the_grid_back() {
    let mut fixture = Fixture::new();
    let mut tern = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut tern);
    assert!(tern.tree.is_some());
    crate::tsp::lifetime::reap_programs(&mut fixture.server);
    tern.pump(&mut fixture.server);
    assert!(tern.tree.is_some(), "the program still owns the terminal");
    // A SIGINT killed it before its `x`, and another group took the terminal.
    let state = || {
        fixture
            .server
            .panes
            .get(fixture.pane)
            .unwrap()
            .tsp
            .as_ref()
            .unwrap()
    };
    assert!(state().program_pgrp.is_some());
    fixture
        .server
        .panes
        .get_mut(fixture.pane)
        .unwrap()
        .tsp
        .as_mut()
        .unwrap()
        .program_pgrp = Some(rmux_sys::ProcessId(1));
    crate::tsp::lifetime::reap_programs(&mut fixture.server);
    tern.pump(&mut fixture.server);
    assert!(tern.tree.is_none(), "the frozen native view closed");
    assert_eq!(
        fixture
            .server
            .panes
            .get(fixture.pane)
            .unwrap()
            .tsp
            .as_ref()
            .unwrap()
            .renderer,
        crate::tsp::broker::Renderer::Ansi
    );
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
