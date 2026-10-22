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
    collections::{BTreeSet, VecDeque},
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
                    // A `b` body is `id=…;mime=…;base64`, not JSON.
                    let body = if bytes[6] == b'b' {
                        Value::String(String::from_utf8(bytes[8..end].to_vec()).unwrap())
                    } else {
                        serde_json::from_slice(&bytes[8..end]).expect("complete TSP APC")
                    };
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
        "settle" => {}
        // The surface stays, hidden; the root carries the flag as a host would.
        "suspend" => {
            tree.as_object_mut()
                .unwrap()
                .insert("suspended".into(), json!(true));
        }
        "resume" => {
            tree.as_object_mut().unwrap().remove("suspended");
        }
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
    tree_inline: bool,
    focus: Option<String>,
    /// An inline surface under the screen `tree`, with its focus, as Tern
    /// keeps it.
    covered: Option<(Value, Option<String>)>,
    /// Inline surfaces rsttyd parked under a later `o`, oldest first, each
    /// with its focus. Only a `["resume"]` frame or an `x` reaches one.
    parked: Vec<(Value, Option<String>)>,
    /// How many parked surfaces the host keeps: its hello's `suspend`. None
    /// for rsttyd up to 1.10, whose `#replaceInline` ends the suspended
    /// surface without a word, and for every other host.
    parks: usize,
    frames: VecDeque<Value>,
    grid: Server,
    grid_pane: PaneId,
    opens: usize,
    /// The blob ids the host holds, from every `b` it received; its `q blobs`
    /// answer when `answers_blobs`. Otherwise the query and the DA1 fence
    /// right after it stay unanswered.
    blobs: BTreeSet<String>,
    answers_blobs: bool,
    held_fence: bool,
    /// The hello replies and DA1s of the probes the host has not answered
    /// yet: a page that was slow to see them. They go out in order, before
    /// any later answer, from `answer_probe` or the next probe it answers.
    held_probe: Option<Vec<u8>>,
    /// Whether the host answers `q hello` at once; else it holds the answer.
    answers_probe: bool,
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
        Self::attach_answering(server, session, native, hello, true)
    }

    /// A TSP host that holds its answers to the probes until `answer_probe`
    /// or `answers_probe`, as a page behind a slow server does.
    fn attach_holding_probe(server: &mut Server, session: Option<SessionId>) -> Self {
        Self::attach_answering(server, session, true, terminal_hello(), false)
    }

    fn attach_answering(
        server: &mut Server,
        session: Option<SessionId>,
        native: bool,
        hello: Value,
        answers_probe: bool,
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
        // A later attach is the newer client, as it is with a real clock.
        let newest = server
            .client_order
            .iter()
            .filter_map(|id| server.clients.get(*id))
            .map(|c| c.activity_time)
            .max()
            .unwrap_or((0, 0));
        let mut client = Client::new(None, (newest.0 + 1, 0));
        client.session = session;
        client.flags.insert(ClientFlags::ATTACHED);
        client.tty = Some(tty);
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        let (grid, grid_pane, _) = model();
        let parks = hello["suspend"].as_u64().map_or(0, |n| n as usize);
        let mut terminal = Self {
            client: id,
            master,
            native,
            hello,
            framing: Framing::default(),
            packets: vec![],
            bytes: vec![],
            tree: None,
            tree_inline: false,
            focus: None,
            covered: None,
            parked: Vec::new(),
            parks,
            frames: VecDeque::new(),
            grid,
            grid_pane,
            opens: 0,
            blobs: BTreeSet::new(),
            answers_blobs: true,
            held_fence: false,
            held_probe: None,
            answers_probe,
        };
        broker::probe_client(server, id);
        terminal.pump(server);
        terminal
    }
    /// The host catches up: it answers the probes it held, in order, and
    /// every later one at once.
    fn answer_probe(&mut self, server: &mut Server) {
        self.answers_probe = true;
        let held = self.held_probe.take().expect("a held probe answer");
        self.send(server, &held);
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
            Packet::Sentinel => {
                if self.held_fence {
                    self.held_fence = false;
                    if let Some(held) = self.held_probe.as_mut() {
                        held.extend_from_slice(b"\x1b[?1;2c");
                    }
                } else {
                    response.extend_from_slice(b"\x1b[?1;2c")
                }
            }
            Packet::ReplySentinel => {}
            Packet::Cells(bytes) => {
                pane_runtime::pane_parse_buffer(&mut self.grid, self.grid_pane, &bytes).unwrap();
                // Only rsttyd gives the whole pane to the grid on suspend; Tern
                // keeps a live inline surface's dock strip over the last row.
                assert!(
                    !(self.alternate() && self.tree_inline && self.tree.is_some())
                        || (self.hello["term"] == "rsttyd" && self.suspending()),
                    "an inline surface stays over the alternate screen"
                );
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
                        let reply = apc(b'r', &self.hello);
                        if self.answers_probe {
                            response.extend(self.held_probe.take().into_iter().flatten());
                            response.extend(reply)
                        } else {
                            self.held_probe.get_or_insert_default().extend(reply);
                            self.held_fence = true;
                        }
                    }
                    b'q' if body["q"] == "blobs" && self.native => {
                        if self.answers_blobs {
                            let have: Vec<&Value> = body["ids"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .filter(|id| self.blobs.contains(id.as_str().unwrap()))
                                .collect();
                            response.extend(apc(b'r', &json!({"r":"blobs","have":have})));
                        } else {
                            self.held_fence = true;
                        }
                    }
                    b'b' => {
                        self.blobs.insert(blob_id(&body));
                    }
                    b'o' => {
                        let over_inline = body["mode"] == "screen"
                            && self.tree.is_some()
                            && self.tree_inline
                            && self.covered.is_none();
                        let over_parked = body["mode"] == "inline"
                            && self.tree_inline
                            && self.tree.is_some()
                            && self.suspending();
                        assert!(
                            self.tree.is_none() || over_inline || over_parked,
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
                        if over_inline {
                            self.covered = self.tree.take().map(|tree| (tree, self.focus.take()));
                        } else if over_parked {
                            let left = (self.tree.take().unwrap(), self.focus.take());
                            if self.parks > 0 {
                                self.parked.push(left);
                            } else {
                                self.frames.retain(|frame| frame["sf"] != left.0["id"]);
                            }
                            assert!(
                                self.parked.len() <= self.parks,
                                "rmux parked more surfaces than the host keeps"
                            );
                        }
                        self.tree_inline = body["mode"] == "inline";
                        self.tree = Some(json!({"id":body["id"],"k":"surface","c":[]}));
                        self.opens += 1;
                    }
                    b'x' => {
                        if self
                            .covered
                            .as_ref()
                            .is_some_and(|(tree, _)| tree["id"] == body["id"])
                        {
                            self.covered = None;
                        } else if let Some(index) = self
                            .parked
                            .iter()
                            .position(|(tree, _)| tree["id"] == body["id"])
                        {
                            self.parked.remove(index);
                        } else if self
                            .tree
                            .as_ref()
                            .is_some_and(|tree| tree["id"] == body["id"])
                        {
                            if let Some((tree, focus)) = self.covered.take() {
                                self.tree = Some(tree);
                                self.tree_inline = true;
                                self.focus = focus;
                            } else {
                                self.tree = None;
                                self.focus = None;
                            }
                        }
                        // Else a surface the host ended already: nothing to close.
                        self.frames.retain(|frame| frame["sf"] != body["id"]);
                    }
                    b'f' => {
                        if !self.knows(&body["sf"]) {
                            // rsttyd answers a frame for a surface it ended.
                            response.extend(apc(
                                b'e',
                                &json!({"ev":"error","sf":body["sf"],"s":body["s"],"msg":"unknown surface"}),
                            ));
                            return;
                        }
                        let resumes = body["ops"]
                            .as_array()
                            .is_some_and(|ops| ops.iter().any(|op| op == &json!(["resume"])));
                        assert!(
                            !(resumes && self.tree_inline && self.alternate()),
                            "an inline surface resumed on the alternate screen"
                        );
                        self.frames.push_back(body)
                    }
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
        let mut drawn: Vec<(Value, u64)> = Vec::new();
        let mut last = None;
        while let Some(frame) = self.frames.pop_front() {
            let resumes = frame["ops"]
                .as_array()
                .is_some_and(|ops| ops.iter().any(|op| op == &json!(["resume"])));
            if resumes
                && let Some(index) = self
                    .parked
                    .iter()
                    .position(|(tree, _)| tree["id"] == frame["sf"])
            {
                // rsttyd `#replaceInline`: the parked surface comes back over
                // the live one, which it parks in turn; or over nothing, after
                // an `x` for what was shown.
                let (tree, focus) = self.parked.remove(index);
                if let Some(live) = self.tree.replace(tree) {
                    assert!(
                        live["suspended"] == true,
                        "a parked surface resumed over a live one"
                    );
                    self.parked.push((live, self.focus.take()));
                }
                self.focus = focus;
                self.tree_inline = true;
            }
            let (tree, focus) = match self.covered.as_mut() {
                Some((tree, focus)) if tree["id"] == frame["sf"] => (tree, focus),
                _ => match self
                    .parked
                    .iter_mut()
                    .find(|(tree, _)| tree["id"] == frame["sf"])
                {
                    Some((tree, focus)) => (tree, focus),
                    None => (
                        self.tree.as_mut().expect("frame before open"),
                        &mut self.focus,
                    ),
                },
            };
            assert_eq!(frame["sf"], tree["id"]);
            for op in frame["ops"].as_array().unwrap() {
                apply_independent(tree, op, focus);
            }
            let sequence = frame["s"].as_u64().unwrap();
            match drawn.iter_mut().find(|(sf, _)| *sf == frame["sf"]) {
                Some(entry) => entry.1 = sequence,
                None => drawn.push((frame["sf"].clone(), sequence)),
            }
            last = Some(sequence);
        }
        for (sf, sequence) in drawn {
            self.send(
                server,
                &apc(b'e', &json!({"ev":"ack","sf":sf,"s":sequence})),
            );
        }
        last
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

    /// The shown surface is suspended: kept, with the grid over it.
    fn suspended(&self) -> bool {
        self.tree
            .as_ref()
            .is_some_and(|tree| tree["suspended"] == true)
    }

    /// The host has the surface `sf`: shown, under a page, or parked.
    fn knows(&self, sf: &Value) -> bool {
        self.tree.as_ref().is_some_and(|tree| &tree["id"] == sf)
            || self
                .covered
                .as_ref()
                .is_some_and(|(tree, _)| &tree["id"] == sf)
            || self.parked.iter().any(|(tree, _)| &tree["id"] == sf)
    }

    /// `suspended`, or a suspend frame for the shown surface received and not
    /// yet drawn.
    fn suspending(&self) -> bool {
        self.suspended()
            || self.frames.iter().any(|frame| {
                self.tree
                    .as_ref()
                    .is_some_and(|tree| tree["id"] == frame["sf"])
                    && frame["ops"]
                        .as_array()
                        .is_some_and(|ops| ops.iter().any(|op| op == &json!(["suspend"])))
            })
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
    native_status_terminal_with(None)
}

/// `term`: the host's name in its hello reply, none for an anonymous host.
fn native_status_terminal_with(term: Option<&str>) -> (Server, PaneId, FakeTerminal) {
    let mut hello = json!({"r":"hello","v":1,"kinds":["col","text","editor","input","status","seg"],
        "features":["edit","undo","send","settle","dock"],"credits":2});
    if let Some(term) = term {
        hello["term"] = term.into();
    }
    stock_native_with_program(hello, json!(["edit", "undo", "send"]))
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

/// The native status terminal with a second window `1:logs` beside the
/// current `0`, `mouse` and the host's `term` as given: the server, the
/// terminal, the session and the two winlinks.
fn native_two_window_bar(
    mouse: bool,
    term: Option<&str>,
) -> (
    Server,
    FakeTerminal,
    SessionId,
    crate::ids::WinlinkId,
    crate::ids::WinlinkId,
) {
    let (mut server, _, mut terminal) = native_status_terminal_with(term);
    let session = server
        .clients
        .get(terminal.client)
        .unwrap()
        .session
        .unwrap();
    let first = server.sessions.get(session).unwrap().current.unwrap();
    let window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
    server.windows.get_mut(window).unwrap().name = b"logs".to_vec();
    let pane =
        window::window_add_pane(&mut server, window, None, 0, SpawnFlags::default()).unwrap();
    server.windows.get_mut(window).unwrap().active = Some(pane);
    crate::layout::init(&mut server, window, pane);
    let second = session::session_attach(&mut server, session, window, 1).unwrap();
    let options = server.sessions.get(session).unwrap().options;
    server
        .options
        .set_number_value(options, b"mouse", i64::from(mouse));
    server
        .clients
        .get_mut(terminal.client)
        .unwrap()
        .flags
        .insert(ClientFlags::REDRAWSTATUS);
    native_keys(&mut server, &mut terminal, b"");
    (server, terminal, session, first, second)
}

fn native_bar_seg(terminal: &mut FakeTerminal, prefix: &str) -> Value {
    node_mut(terminal.tree.as_mut().unwrap(), "rmux:bar:0").unwrap()["c"]
        .as_array()
        .unwrap()
        .iter()
        .find(|seg| {
            seg["p"]["text"]
                .as_str()
                .is_some_and(|text| text.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("no bar segment {prefix}"))
        .clone()
}

/// Tern's event for a click on the node `id`, run to the end of the
/// client's queue. Not drawn after: the click may take the client off the
/// projected pane.
fn native_action(server: &mut Server, terminal: &FakeTerminal, id: &Value, act: &str) {
    let sf = terminal.tree.as_ref().unwrap()["id"].clone();
    terminal.send(
        server,
        &apc(b'e', &json!({"ev":"action","sf":sf,"id":id,"act":act})),
    );
    crate::cmd::queue::next(server, Some(terminal.client));
}

#[test]
fn native_bar_click_runs_the_status_mouse_binding() {
    let (mut server, mut terminal, session, _, second) = native_two_window_bar(true, None);
    let seg = native_bar_seg(&mut terminal, "1:logs");
    assert_eq!(seg["p"]["actions"], json!({"click": "status"}));
    // `MouseDown1Status { switch-client -t= }`, `=` the clicked window.
    native_action(&mut server, &terminal, &seg["id"], "status");
    assert_eq!(server.sessions.get(session).unwrap().current, Some(second));
}

#[test]
fn native_bar_click_follows_a_rebound_status_binding() {
    use crate::cmd::key_bindings::KeyInitRuntime;
    let (mut server, mut terminal, session, first, second) = native_two_window_bar(true, None);
    let list = crate::cmd::parse::from_string(
        &mut server,
        b"bind -n MouseDown1Status rename-window -t= clicked",
        &mut crate::cmd::parse::CmdParseInput::default(),
    )
    .unwrap();
    server.append_default_commands(list);
    crate::cmd::queue::next(&mut server, None);
    let seg = native_bar_seg(&mut terminal, "1:logs");
    native_action(&mut server, &terminal, &seg["id"], "status");
    let window = server.winlinks.get(second).unwrap().window;
    assert_eq!(server.windows.get(window).unwrap().name, b"clicked");
    assert_eq!(server.sessions.get(session).unwrap().current, Some(first));
}

#[test]
fn native_bar_ignores_unknown_segments_other_actions_and_mouse_off() {
    let (mut server, mut terminal, session, first, _) = native_two_window_bar(true, None);
    let seg = native_bar_seg(&mut terminal, "1:logs");
    for (id, act) in [
        (json!("rmux:bar:0:99"), "status"),
        (json!("rmux:bar:0"), "status"),
        (seg["id"].clone(), "other"),
    ] {
        native_action(&mut server, &terminal, &id, act);
        assert_eq!(server.sessions.get(session).unwrap().current, Some(first));
    }

    let (mut server, mut terminal, session, first, _) = native_two_window_bar(false, None);
    let seg = native_bar_seg(&mut terminal, "1:logs");
    assert!(seg["p"].get("actions").is_none());
    native_action(&mut server, &terminal, &seg["id"], "status");
    assert_eq!(server.sessions.get(session).unwrap().current, Some(first));
}

/// A window switch by `keys`, run through the server loop's order: the
/// command, its effects, the client tick, then a second tick for the grid
/// redraw that waited for the protocol bytes to drain.
fn switch_window(server: &mut Server, terminal: &mut FakeTerminal, keys: &[u8]) {
    terminal.send(server, keys);
    crate::cmd::queue::next(server, Some(terminal.client));
    crate::server::run::drain_effects(server).unwrap();
    crate::client::tick::tick(server);
    crate::server::run::drain_effects(server).unwrap();
    terminal.pump(server);
    terminal.draw(server);
    crate::client::tick::tick(server);
    terminal.pump(server);
    terminal.draw(server);
}

fn verbs_since(terminal: &FakeTerminal, sent: usize) -> Vec<char> {
    terminal.packets[sent..]
        .iter()
        .map(|(verb, _)| char::from(*verb))
        .collect()
}

/// The first op of each `f` since `sent`.
fn frame_heads(terminal: &FakeTerminal, sent: usize) -> Vec<Value> {
    terminal.packets[sent..]
        .iter()
        .filter(|(verb, _)| *verb == b'f')
        .map(|(_, body)| body["ops"][0].clone())
        .collect()
}

/// rsttyd gives the whole pane to the grid on `["suspend"]`: the projection
/// hides instead of closing.
#[test]
fn switching_to_a_grid_window_suspends_the_native_projection_on_rsttyd() {
    let (mut server, mut terminal, _, _, second) = native_two_window_bar(false, Some("rsttyd"));
    let logs = server.winlinks.get(second).unwrap().window;
    let logs_pane = server.windows.get(logs).unwrap().active.unwrap();
    pane_runtime::pane_parse_buffer(&mut server, logs_pane, b"logs pane").unwrap();
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x021");
    assert_eq!(frame_heads(&terminal, sent), vec![json!(["suspend"])]);
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(
        verbs,
        vec!['f'],
        "one suspend frame, nothing else: {verbs:?}"
    );
    assert_eq!(terminal.opens, 1);
    assert!(
        terminal.suspended(),
        "the host keeps the surface, suspended"
    );
    assert!(
        terminal.alternate(),
        "the grid is back on the alternate screen"
    );
    assert_eq!(terminal.row(0), "logs pane");
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.projection.is_none() && c.tsp.hidden.len() == 1);
}

#[test]
fn switching_back_resumes_the_native_projection_without_a_replay_on_rsttyd() {
    let (mut server, mut terminal, _, _, _) = native_two_window_bar(false, Some("rsttyd"));
    let before = terminal.tree.clone().unwrap();
    let generation = projection_generation(&server, &terminal);
    assert!(generation.is_some());
    switch_window(&mut server, &mut terminal, b"\x021");
    assert!(terminal.suspended());
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(frame_heads(&terminal, sent), vec![json!(["resume"])]);
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(
        verbs,
        vec!['f'],
        "one resume frame, nothing else: {verbs:?}"
    );
    assert_eq!(terminal.opens, 1);
    assert_eq!(projection_generation(&server, &terminal), generation);
    assert!(!terminal.suspended());
    assert!(
        !terminal.alternate(),
        "the inline surface is on the main screen"
    );
    // The resume frame carries the bar refresh: window 1 is now the last one.
    assert_eq!(
        native_bar_seg(&mut terminal, "1:logs")["p"]["text"],
        "1:logs-"
    );
    let without_bar = |mut tree: Value| {
        take_node(&mut tree, "rmux:bar:0").unwrap();
        tree
    };
    assert_eq!(
        without_bar(terminal.tree.clone().unwrap()),
        without_bar(before),
        "every node survived"
    );
    assert_eq!(terminal.text("transcript"), "retained transcript");
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.hidden.is_empty());
}

/// One `b` APC as a program writes it: `id=…;mime=…;<base64>`.
fn blob_apc(id: &str, bytes: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let mut apc = format!("\x1b_tsp;b;id={id};mime=image/png;").into_bytes();
    apc.extend(
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into_bytes(),
    );
    apc.extend(b"\x1b\\");
    apc
}

#[test]
fn blob_that_arrives_after_its_frame_is_pushed_at_once() {
    use sha2::Digest;
    let (mut server, pane, mut terminal) = stock_native_with_hello(json!({
        "r":"hello","v":1,"term":"fixture","version":"1","kinds":["col","text","editor","image"],
        "features":["edit","undo","send","settle","blobs"],"apc":65536,"credits":2,
        "cell":{"w":8,"h":16},"dark":true,"reduceMotion":false}));
    let bytes = b"\x89PNG late blob";
    let id = format!("{:x}", sha2::Sha256::digest(bytes));
    // omp's first blob: the frame goes out at once, the bytes follow the
    // `blobs` query round trip.
    let mut program = apc(b'q', &json!({"q":"blobs","ids":[id]}));
    program.extend(apc(
        b'f',
        &json!({"sf":"program","s":2,"ops":[
        ["add","shot","program",null,{"id":"shot","k":"image","p":{"blob":id,"w":4,"h":4}}]]}),
    ));
    let sent = terminal.packets.len();
    pane_runtime::pane_parse_buffer(&mut server, pane, &program).unwrap();
    let reply = apc(b'r', &json!({"r":"blobs","have":[]}));
    let output = &server.panes.get(pane).unwrap().output;
    assert!(
        output.windows(reply.len()).any(|window| window == reply),
        "rmux answers the query itself: {}",
        String::from_utf8_lossy(output)
    );
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(
        verbs_since(&terminal, sent),
        vec!['f'],
        "the frame went without the blob"
    );
    assert_eq!(
        node_mut(terminal.tree.as_mut().unwrap(), "shot").unwrap()["p"]["blob"],
        id
    );

    let sent = terminal.packets.len();
    pane_runtime::pane_parse_buffer(&mut server, pane, &blob_apc(&id, bytes)).unwrap();
    terminal.pump(&mut server);
    assert_eq!(
        verbs_since(&terminal, sent),
        vec!['b'],
        "the late blob reaches the host on its own, with no new frame"
    );
    let body = terminal.packets[sent].1.as_str().unwrap();
    assert!(
        body.starts_with(&format!("id={id};mime=image/png;")),
        "{body}"
    );

    // The host keeps it: the next frame does not send it again, and a repeat
    // of the same `b` is not pushed twice.
    let sent = terminal.packets.len();
    let mut again = blob_apc(&id, bytes);
    again.extend(apc(
        b'f',
        &json!({"sf":"program","s":3,"ops":[["set","transcript",{"text":"two"}]]}),
    ));
    pane_runtime::pane_parse_buffer(&mut server, pane, &again).unwrap();
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(verbs_since(&terminal, sent), vec!['f']);
    assert_eq!(terminal.text("transcript"), "two");
}

/// Tern keeps a live suspended surface's dock strip over the last grid row,
/// and an unknown host may do the same: the switch closes the projection
/// (`x`, `keep:false`) and the return reopens it with a replay.
#[test]
fn switching_closes_and_reopens_the_native_projection_on_other_hosts() {
    for term in [Some("tern"), None] {
        let (mut server, mut terminal, _, _, second) = native_two_window_bar(false, term);
        let logs = server.winlinks.get(second).unwrap().window;
        let logs_pane = server.windows.get(logs).unwrap().active.unwrap();
        pane_runtime::pane_parse_buffer(&mut server, logs_pane, b"logs pane").unwrap();
        let generation = projection_generation(&server, &terminal);
        let sent = terminal.packets.len();
        switch_window(&mut server, &mut terminal, b"\x021");
        let verbs = verbs_since(&terminal, sent);
        assert_eq!(verbs, vec!['x'], "{term:?}: close only: {verbs:?}");
        assert_eq!(terminal.packets[sent].1["keep"], false, "{term:?}");
        assert!(
            terminal.tree.is_none(),
            "{term:?}: the host dropped the surface"
        );
        assert!(terminal.alternate(), "{term:?}");
        assert_eq!(terminal.row(0), "logs pane", "{term:?}");
        let c = server.clients.get(terminal.client).unwrap();
        assert!(
            c.tsp.projection.is_none() && c.tsp.hidden.is_empty(),
            "{term:?}"
        );

        let sent = terminal.packets.len();
        switch_window(&mut server, &mut terminal, b"\x020");
        let verbs = verbs_since(&terminal, sent);
        assert!(verbs.contains(&'o'), "{term:?}: reopened: {verbs:?}");
        assert!(
            frame_heads(&terminal, sent)
                .iter()
                .all(|head| head != &json!(["resume"])),
            "{term:?}: nothing to resume"
        );
        assert_eq!(terminal.opens, 2, "{term:?}");
        assert_ne!(
            projection_generation(&server, &terminal),
            generation,
            "{term:?}"
        );
        assert!(!terminal.alternate(), "{term:?}");
        assert_eq!(
            terminal.text("transcript"),
            "retained transcript",
            "{term:?}"
        );
    }
}

/// The `id` of a `b` body `id=…;mime=…;<base64>`.
fn blob_id(body: &Value) -> String {
    body.as_str()
        .unwrap()
        .strip_prefix("id=")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// The ids of the `b` APCs since `sent`, in order.
fn blobs_since(terminal: &FakeTerminal, sent: usize) -> Vec<String> {
    terminal.packets[sent..]
        .iter()
        .filter(|(verb, _)| *verb == b'b')
        .map(|(_, body)| blob_id(body))
        .collect()
}

/// The `ids` of each `q blobs` since `sent`.
fn blobs_queries_since(terminal: &FakeTerminal, sent: usize) -> Vec<Value> {
    terminal.packets[sent..]
        .iter()
        .filter(|(verb, body)| *verb == b'q' && body["q"] == "blobs")
        .map(|(_, body)| body["ids"].clone())
        .collect()
}

/// The verbs since `sent` from the new projection's `o` on.
fn verbs_from_open(terminal: &FakeTerminal, sent: usize) -> Vec<char> {
    let verbs = verbs_since(terminal, sent);
    let open = verbs
        .iter()
        .position(|verb| *verb == 'o')
        .unwrap_or_else(|| panic!("no new projection: {verbs:?}"));
    verbs[open..].to_vec()
}

/// Each `q` APC in `bytes`: its body and the offset right after its terminator.
fn queries(bytes: &[u8]) -> Vec<(Value, usize)> {
    let query = b"\x1b_tsp;q;";
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = bytes[at..]
        .windows(query.len())
        .position(|window| window == query)
    {
        let start = at + start + query.len();
        let Some(end) = bytes[start..]
            .windows(2)
            .position(|window| window == b"\x1b\\")
        else {
            break;
        };
        let end = start + end;
        out.push((serde_json::from_slice(&bytes[start..end]).unwrap(), end + 2));
        at = end + 2;
    }
    out
}

/// The offset of each DA1 query `ESC [ c` in `bytes`.
fn fences(bytes: &[u8]) -> Vec<usize> {
    bytes
        .windows(3)
        .enumerate()
        .filter(|(_, window)| *window == b"\x1b[c")
        .map(|(at, _)| at)
        .collect()
}

/// The DA1 fence right after the `q blobs` APC in `bytes`: the query and the
/// fence are one protocol transaction.
fn fence_follows_blobs_query(bytes: &[u8]) -> bool {
    queries(bytes)
        .iter()
        .find(|(body, _)| body["q"] == "blobs")
        .is_some_and(|(_, end)| bytes[*end..].starts_with(b"\x1b[c"))
}

/// What `close_bytes` adds after an inline surface's `x`.
const BACK_TO_GRID: &[u8] = b"\x1b[r\x1b8\x1b[A\x1b[M";

fn bytes_contain(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// Window 0's two images in `two_omp_windows`.
const SHOTS: [&[u8]; 2] = [b"\x89PNG shot a", b"\x89PNG shot b"];

/// rsttyd with a whole-terminal blob store: `blobstore` beside its 1.4.0
/// features.
fn blobstore_hello() -> Value {
    json!({"r":"hello","v":1,"term":"rsttyd","version":"1.5.0","kinds":["col","text","editor","image"],
        "features":["dock","adopt","program-palette","scroll","settle","edit","undo","blobs","styles","blobstore"],
        "apc":65536,"credits":2,"cell":{"w":8,"h":16},"dark":true,"reduceMotion":false})
}

fn rsttyd_1_4_hello() -> Value {
    json!({"r":"hello","v":1,"term":"rsttyd","version":"1.4.0","kinds":["col","text","editor","image"],
        "features":["dock","adopt","program-palette","scroll","settle","edit","undo","blobs","styles"],
        "apc":65536,"credits":2,"cell":{"w":8,"h":16},"dark":true,"reduceMotion":false})
}

fn tern_hello() -> Value {
    json!({"r":"hello","v":1,"term":"tern","version":"0.8.4","kinds":["col","text","editor","image"],
        "features":["blobs","settle","adopt","dock","program-palette","reduce-motion","aside","scroll","styles","flow"],
        "apc":65536,"credits":2,"cell":{"w":8,"h":16},"dark":true,"reduceMotion":false})
}

/// The stock omp window of `stock_native_with_hello`, its program showing two
/// images: the server, the terminal, the pane and the blob ids, sorted as the
/// query lists them.
fn shots_window(hello: Value) -> (Server, FakeTerminal, PaneId, Vec<String>) {
    let (mut server, pane, mut terminal) = stock_native_with_hello(hello);
    let shots = SHOTS;
    let mut ids: Vec<String> = shots.iter().map(|bytes| sha256_hex(bytes)).collect();
    ids.sort();
    let mut program = blob_apc(&sha256_hex(shots[0]), shots[0]);
    program.extend(blob_apc(&sha256_hex(shots[1]), shots[1]));
    program.extend(apc(
        b'f',
        &json!({"sf":"program","s":2,"ops":[
            ["add","shot-a","program",null,{"id":"shot-a","k":"image","p":{"blob":sha256_hex(shots[0]),"w":4,"h":4}}],
            ["add","shot-b","program",null,{"id":"shot-b","k":"image","p":{"blob":sha256_hex(shots[1]),"w":4,"h":4}}]]}),
    ));
    let sent = terminal.packets.len();
    pane_runtime::pane_parse_buffer(&mut server, pane, &program).unwrap();
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    let mut first = blobs_since(&terminal, sent);
    first.sort();
    assert_eq!(first, ids, "the first frame carries both blobs");
    assert_eq!(terminal.blobs.iter().cloned().collect::<Vec<_>>(), ids);
    (server, terminal, pane, ids)
}

/// A new window with one empty pane at `index` of the terminal's session,
/// made current by `C-b index`; the pane.
fn new_window(server: &mut Server, terminal: &mut FakeTerminal, index: i32) -> PaneId {
    let session = server
        .clients
        .get(terminal.client)
        .unwrap()
        .session
        .unwrap();
    let window = window::window_create(server, 80, 24, 0, 0).unwrap();
    let pane = window::window_add_pane(server, window, None, 0, SpawnFlags::default()).unwrap();
    server.windows.get_mut(window).unwrap().active = Some(pane);
    crate::layout::init(server, window, pane);
    session::session_attach(server, session, window, index).unwrap();
    switch_window(server, terminal, format!("\x02{index}").as_bytes());
    pane
}

/// An omp with program `features` starts in `pane`, the current window, and
/// shows `transcript`.
fn start_omp(
    server: &mut Server,
    terminal: &mut FakeTerminal,
    pane: PaneId,
    transcript: &str,
    features: Value,
) {
    let mut program = apc(
        b'q',
        &json!({"q":"hello","v":[1],"app":"omp","features":features}),
    );
    program.extend(apc(
        b'o',
        &json!({"id":"program","mode":"inline","title":"omp"}),
    ));
    program.extend(apc(
        b'f',
        &json!({"sf":"program","s":1,"ops":[["add","transcript","program",null,{"id":"transcript","k":"text","p":{"text":transcript}}]]}),
    ));
    pane_runtime::pane_parse_buffer(server, pane, &program).unwrap();
    terminal.pump(server);
    terminal.draw(server);
    assert_eq!(terminal.text("transcript"), transcript);
}

/// Two omp windows on one terminal: window 0's program shows two images,
/// window 1's a transcript with program `other_features`. Window 1 is current,
/// so `C-b 0` opens a new projection of window 0 (rsttyd keeps one suspended
/// surface, which the other window's open closed; every other host closes on
/// the switch). The server, the terminal, the two panes and window 0's blob
/// ids, sorted as the query lists them.
fn two_omp_windows(
    hello: Value,
    other_features: Value,
) -> (Server, FakeTerminal, PaneId, PaneId, Vec<String>) {
    let (mut server, mut terminal, pane, ids) = shots_window(hello);
    let other = new_window(&mut server, &mut terminal, 1);
    start_omp(
        &mut server,
        &mut terminal,
        other,
        "other transcript",
        other_features,
    );
    assert_eq!(terminal.opens, 2);
    let c = server.clients.get(terminal.client).unwrap();
    assert!(
        c.tsp.hidden.is_empty(),
        "the other window's open closed the suspended surface"
    );
    (server, terminal, pane, other, ids)
}

/// Omp windows `1..count` beside the shots window 0, each showing
/// `transcript <index>`; the last one is current. The server, the terminal,
/// the panes by window and window 0's blob ids.
fn omp_windows(hello: Value, count: i32) -> (Server, FakeTerminal, Vec<PaneId>, Vec<String>) {
    let (mut server, mut terminal, pane, ids) = shots_window(hello);
    let mut panes = vec![pane];
    for index in 1..count {
        let pane = new_window(&mut server, &mut terminal, index);
        start_omp(
            &mut server,
            &mut terminal,
            pane,
            &format!("transcript {index}"),
            json!([]),
        );
        panes.push(pane);
    }
    (server, terminal, panes, ids)
}

/// The replay the client is holding, after the terminal answered the pump.
fn settle_replay(server: &mut Server, terminal: &mut FakeTerminal) {
    terminal.pump(server);
    terminal.draw(server);
    crate::client::tick::tick(server);
    terminal.pump(server);
    terminal.draw(server);
}

/// The host's `r blobs` and the DA1 reply that closes the request.
fn answer_blobs(server: &mut Server, terminal: &mut FakeTerminal, have: &[String]) {
    let mut reply = apc(b'r', &json!({"r":"blobs","have":have}));
    reply.extend(b"\x1b[?1;2c");
    terminal.send(server, &reply);
    settle_replay(server, terminal);
}

fn awaiting_blobs(server: &Server, terminal: &FakeTerminal) -> bool {
    server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .projection
        .as_ref()
        .is_some_and(|p| p.awaiting_blobs().is_some())
}

fn confirmed_blobs(server: &Server, terminal: &FakeTerminal) -> Vec<String> {
    server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .confirmed_blobs
        .iter()
        .cloned()
        .collect()
}

fn assert_both_shots(terminal: &mut FakeTerminal, ids: &[String]) {
    let mut shown: Vec<String> = ["shot-a", "shot-b"]
        .iter()
        .map(|id| {
            node_mut(terminal.tree.as_mut().unwrap(), id).unwrap()["p"]["blob"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    shown.sort();
    assert_eq!(shown, ids);
}

#[test]
fn blobstore_host_is_asked_before_a_new_projection_and_gets_no_blob_it_has() {
    let (mut server, mut terminal, _, _, ids) = two_omp_windows(blobstore_hello(), json!([]));
    terminal.answers_blobs = false;
    let sent = terminal.packets.len();
    let bytes = terminal.bytes.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert!(
        fence_follows_blobs_query(&terminal.bytes[bytes..]),
        "the DA1 fence closes the query"
    );
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(verbs.last(), Some(&'q'), "{verbs:?}");
    assert!(
        !verbs
            .iter()
            .any(|verb| matches!(verb, 'o' | 't' | 's' | 'b')),
        "nothing of the new projection before the answer: {verbs:?}"
    );
    assert!(
        frame_heads(&terminal, sent)
            .iter()
            .all(|head| head == &json!(["suspend"])),
        "no frame but the left window's suspend: {verbs:?}"
    );
    assert_eq!(terminal.opens, 2);
    assert!(awaiting_blobs(&server, &terminal));
    // The waiting client is a native one: no grid or status-line repaint of
    // the pane follows the fence.
    assert!(
        terminal.bytes[bytes..].ends_with(b"\x1b[c"),
        "bytes after the fence while waiting: {:?}",
        String::from_utf8_lossy(&terminal.bytes[bytes..])
    );

    let sent = terminal.packets.len();
    answer_blobs(&mut server, &mut terminal, &ids);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert_eq!(terminal.opens, 3);
    assert!(!awaiting_blobs(&server, &terminal));
    assert_eq!(confirmed_blobs(&server, &terminal), ids);
    assert_both_shots(&mut terminal, &ids);
}

#[test]
fn blobstore_host_that_lost_its_store_gets_every_blob_before_the_frame() {
    let (mut server, mut terminal, _, _, ids) = two_omp_windows(blobstore_hello(), json!([]));
    terminal.blobs.clear();
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'b', 'b', 'f']);
    let mut sent_blobs = blobs_since(&terminal, sent);
    sent_blobs.sort();
    assert_eq!(sent_blobs, ids);
    assert_eq!(terminal.opens, 3);
    assert!(confirmed_blobs(&server, &terminal).is_empty());
    assert_both_shots(&mut terminal, &ids);
}

#[test]
fn blobstore_host_gets_only_the_blob_it_lacks() {
    let (mut server, mut terminal, _, _, ids) = two_omp_windows(blobstore_hello(), json!([]));
    terminal.blobs.remove(&ids[1]);
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'b', 'f']);
    assert_eq!(blobs_since(&terminal, sent), vec![ids[1].clone()]);
    assert_eq!(confirmed_blobs(&server, &terminal), vec![ids[0].clone()]);
    assert_both_shots(&mut terminal, &ids);
}

/// The deferred 1 s timer of the shown projection's blob wait.
fn fire_blob_timeout(server: &mut Server, terminal: &FakeTerminal) {
    let timer = server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .projection
        .as_ref()
        .unwrap()
        .awaiting_blobs()
        .expect("waiting for the host's answer");
    let crate::server::event_loop::LoopAction::Deferred(id) =
        *server.event_loop.timer_action(timer).unwrap()
    else {
        panic!("typed timeout")
    };
    server.deferred.remove(&id).unwrap()(server);
}

#[test]
fn blobstore_host_that_never_answers_gets_every_blob_after_a_second() {
    let (mut server, mut terminal, _, _, ids) = two_omp_windows(blobstore_hello(), json!([]));
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(confirmed_blobs(&server, &terminal), ids);
    switch_window(&mut server, &mut terminal, b"\x021");
    assert_eq!(terminal.text("transcript"), "other transcript");

    terminal.answers_blobs = false;
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(verbs.last(), Some(&'q'), "{verbs:?}");
    assert_eq!(terminal.opens, 4);

    fire_blob_timeout(&mut server, &terminal);
    assert!(!awaiting_blobs(&server, &terminal));
    let sent = terminal.packets.len();
    settle_replay(&mut server, &mut terminal);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'b', 'b', 'f']);
    let mut sent_blobs = blobs_since(&terminal, sent);
    sent_blobs.sort();
    assert_eq!(sent_blobs, ids);
    assert_eq!(terminal.opens, 5);
    assert!(
        confirmed_blobs(&server, &terminal).is_empty(),
        "the ids asked about are forgotten on the timeout"
    );
    assert_both_shots(&mut terminal, &ids);

    // The host's late answer changes nothing.
    let sent = terminal.packets.len();
    answer_blobs(&mut server, &mut terminal, &ids);
    assert!(confirmed_blobs(&server, &terminal).is_empty());
    let verbs = verbs_since(&terminal, sent);
    assert!(verbs.is_empty(), "{verbs:?}");
}

#[test]
fn blobstore_host_answer_replaces_what_rmux_knows_it_has() {
    let (mut server, mut terminal, _, _, ids) = two_omp_windows(blobstore_hello(), json!([]));
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert_eq!(confirmed_blobs(&server, &terminal), ids);

    // The page reloaded: its store is empty.
    terminal.blobs.clear();
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x021");
    assert!(
        blobs_queries_since(&terminal, sent).is_empty(),
        "a document without blobs asks nothing"
    );
    assert_eq!(terminal.text("transcript"), "other transcript");
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'b', 'b', 'f']);
    let mut sent_blobs = blobs_since(&terminal, sent);
    sent_blobs.sort();
    assert_eq!(sent_blobs, ids);
    assert!(confirmed_blobs(&server, &terminal).is_empty());
    assert_both_shots(&mut terminal, &ids);
}

/// Tern and rsttyd 1.4.0 announce `blobs`, not `blobstore`: no query, no
/// wait, every blob in the replay as before.
#[test]
fn hosts_without_a_blob_store_get_every_blob_with_no_query() {
    for hello in [tern_hello(), rsttyd_1_4_hello()] {
        let term = hello["term"].clone();
        let (mut server, mut terminal, _, _, ids) = two_omp_windows(hello, json!([]));
        terminal.answers_blobs = false;
        let sent = terminal.packets.len();
        let bytes = terminal.bytes.len();
        switch_window(&mut server, &mut terminal, b"\x020");
        assert!(
            blobs_queries_since(&terminal, 0).is_empty(),
            "{term}: never asked"
        );
        // The program hello is unchanged: the switch writes no query and no
        // DA1 at all, as before.
        let written = &terminal.bytes[bytes..];
        assert!(queries(written).is_empty(), "{term}: a query on the wire");
        assert!(fences(written).is_empty(), "{term}: a DA1 on the wire");
        assert!(
            server
                .clients
                .get(terminal.client)
                .unwrap()
                .tsp
                .requests
                .is_empty(),
            "{term}: a request left open"
        );
        assert_eq!(
            verbs_from_open(&terminal, sent),
            vec!['o', 'b', 'b', 'f'],
            "{term}"
        );
        let mut sent_blobs = blobs_since(&terminal, sent);
        sent_blobs.sort();
        assert_eq!(sent_blobs, ids, "{term}");
        assert_eq!(terminal.opens, 3, "{term}");
        assert!(!awaiting_blobs(&server, &terminal), "{term}");
        assert_both_shots(&mut terminal, &ids);
    }
}

/// omp's first blob follows its `blobs` round trip, so a frame may reference
/// a blob rmux does not hold yet. It arrives while the new projection waits
/// for the host's answer: it goes with the replay, once.
#[test]
fn late_blob_reaches_a_blobstore_host_that_is_still_being_asked() {
    let (mut server, mut terminal, pane, _, ids) = two_omp_windows(blobstore_hello(), json!([]));
    let late = b"\x89PNG shot c";
    let late_id = sha256_hex(late);
    let mut program = apc(b'q', &json!({"q":"blobs","ids":[late_id]}));
    program.extend(apc(
        b'f',
        &json!({"sf":"program","s":3,"ops":[
            ["add","shot-c","program",null,{"id":"shot-c","k":"image","p":{"blob":late_id,"w":4,"h":4}}]]}),
    ));
    pane_runtime::pane_parse_buffer(&mut server, pane, &program).unwrap();

    terminal.answers_blobs = false;
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(
        blobs_queries_since(&terminal, sent),
        vec![json!(ids)],
        "only the blobs rmux holds are asked about"
    );
    assert!(awaiting_blobs(&server, &terminal));

    let sent = terminal.packets.len();
    pane_runtime::pane_parse_buffer(&mut server, pane, &blob_apc(&late_id, late)).unwrap();
    terminal.pump(&mut server);
    assert!(
        verbs_since(&terminal, sent).is_empty(),
        "the late blob waits with the replay"
    );

    answer_blobs(&mut server, &mut terminal, &ids);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'b', 'f']);
    assert_eq!(blobs_since(&terminal, sent), vec![late_id.clone()]);
    assert_eq!(terminal.opens, 3);
    assert_eq!(
        node_mut(terminal.tree.as_mut().unwrap(), "shot-c").unwrap()["p"]["blob"],
        late_id
    );
    assert_both_shots(&mut terminal, &ids);
}

/// A program hello the host has not seen and the blob query share one
/// transaction: `q hello`, `q blobs`, one DA1 fence.
#[test]
fn program_hello_and_blob_query_go_out_under_one_fence() {
    let (mut server, mut terminal, _, _, ids) = two_omp_windows(blobstore_hello(), json!(["edit"]));
    terminal.answers_blobs = false;
    let sent = terminal.packets.len();
    let bytes = terminal.bytes.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    let written = &terminal.bytes[bytes..];
    let names: Vec<Value> = queries(written)
        .into_iter()
        .map(|(body, _)| body["q"].clone())
        .collect();
    assert_eq!(names, vec![json!("hello"), json!("blobs")]);
    assert_eq!(fences(written).len(), 1, "one DA1 closes both queries");
    assert!(fence_follows_blobs_query(written));
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(verbs.last(), Some(&'q'), "{verbs:?}");
    assert!(!verbs.contains(&'o'), "{verbs:?}");
    assert!(awaiting_blobs(&server, &terminal));

    // The terminal answered the program hello at once; its blobs answer and
    // the DA1 reply release the replay.
    let sent = terminal.packets.len();
    answer_blobs(&mut server, &mut terminal, &ids);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert_eq!(confirmed_blobs(&server, &terminal), ids);
    assert_eq!(terminal.opens, 3);
    assert_both_shots(&mut terminal, &ids);
}

/// A page reload with the pty kept: the host's store is empty and it reports
/// the shown surface gone. The reopen asks about that surface's blobs only;
/// every other blob rmux had confirmed is lost too, so a later frame that
/// references one carries it.
#[test]
fn reloaded_blobstore_host_gets_the_blobs_confirmed_before_the_reload() {
    let (mut server, mut terminal, _, other, ids) = two_omp_windows(blobstore_hello(), json!([]));
    let own = b"\x89PNG shot y";
    let own_id = sha256_hex(own);
    let mut program = blob_apc(&own_id, own);
    program.extend(apc(
        b'f',
        &json!({"sf":"program","s":2,"ops":[
            ["add","shot-y","program",null,{"id":"shot-y","k":"image","p":{"blob":own_id,"w":4,"h":4}}]]}),
    ));
    let sent = terminal.packets.len();
    pane_runtime::pane_parse_buffer(&mut server, other, &program).unwrap();
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(blobs_since(&terminal, sent), vec![own_id.clone()]);

    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(confirmed_blobs(&server, &terminal), ids);
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x021");
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!([own_id])]);
    let mut all = ids.clone();
    all.push(own_id.clone());
    all.sort();
    assert_eq!(confirmed_blobs(&server, &terminal), all);

    terminal.blobs.clear();
    let sent = terminal.packets.len();
    host_lost_surface(&mut server, &mut terminal);
    terminal.draw(&mut server);
    assert_eq!(
        blobs_queries_since(&terminal, sent),
        vec![json!([own_id])],
        "the reopen asks about its own surface only"
    );
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'b', 'f']);
    assert_eq!(blobs_since(&terminal, sent), vec![own_id.clone()]);
    assert!(
        confirmed_blobs(&server, &terminal).is_empty(),
        "the reload lost every blob, asked about or not"
    );

    // Window 1's program now shows window 0's first image: the same bytes,
    // the same id, which rmux had confirmed before the reload.
    let shared_id = sha256_hex(SHOTS[0]);
    let mut program = blob_apc(&shared_id, SHOTS[0]);
    program.extend(apc(
        b'f',
        &json!({"sf":"program","s":3,"ops":[
            ["add","shot-x","program",null,{"id":"shot-x","k":"image","p":{"blob":shared_id,"w":4,"h":4}}]]}),
    ));
    let sent = terminal.packets.len();
    pane_runtime::pane_parse_buffer(&mut server, other, &program).unwrap();
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    let verbs = verbs_since(&terminal, sent);
    assert!(
        verbs.contains(&'f') && !verbs.contains(&'o'),
        "a live frame: {verbs:?}"
    );
    assert_eq!(
        blobs_since(&terminal, sent),
        vec![shared_id.clone()],
        "the live frame carries the blob the reloaded host lost"
    );
    assert_eq!(
        node_mut(terminal.tree.as_mut().unwrap(), "shot-x").unwrap()["p"]["blob"],
        shared_id
    );
}

/// Leaving a window whose projection still waits for the host's answer: the
/// host never saw that surface, so there is no `x` and no anchor row to
/// remove.
#[test]
fn leaving_a_waiting_projection_closes_it_without_a_word_to_the_host() {
    let (mut server, mut terminal, _, _, _) = two_omp_windows(blobstore_hello(), json!([]));
    terminal.answers_blobs = false;
    switch_window(&mut server, &mut terminal, b"\x020");
    assert!(awaiting_blobs(&server, &terminal));
    let outer = server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .projection
        .as_ref()
        .unwrap()
        .outer
        .clone();

    let sent = terminal.packets.len();
    let bytes = terminal.bytes.len();
    switch_window(&mut server, &mut terminal, b"\x021");
    assert!(
        !terminal.packets[sent..]
            .iter()
            .any(|(verb, body)| *verb == b'x' && body["id"] == outer),
        "an `x` for a surface the host never opened: {:?}",
        verbs_since(&terminal, sent)
    );
    assert!(
        !bytes_contain(&terminal.bytes[bytes..], BACK_TO_GRID),
        "an anchor row removed for a surface the host never opened"
    );
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert_eq!(terminal.text("transcript"), "other transcript");
    assert_eq!(terminal.opens, 3);
    let c = server.clients.get(terminal.client).unwrap();
    assert!(
        c.tsp.projections().all(|p| p.awaiting_blobs().is_none()),
        "no projection waits any more"
    );
}

/// rsttyd that parks `suspend` inline surfaces for each pane, beside its
/// terminal-wide blob store.
fn parked_hello(suspend: u32) -> Value {
    let mut hello = blobstore_hello();
    hello["suspend"] = suspend.into();
    hello
}

/// A host rmux knows nothing about that takes blobs, as Tern does.
fn fixture_blobs_hello() -> Value {
    json!({"r":"hello","v":1,"term":"fixture","version":"1","kinds":["col","text","editor","image"],
        "features":["edit","undo","send","settle","blobs"],"apc":65536,"credits":2,
        "cell":{"w":8,"h":16},"dark":true,"reduceMotion":false})
}

/// The surface ids of the `x` APCs since `sent`, each `keep:false`.
fn closes_since(terminal: &FakeTerminal, sent: usize) -> Vec<String> {
    terminal.packets[sent..]
        .iter()
        .filter(|(verb, _)| *verb == b'x')
        .map(|(_, body)| {
            assert_eq!(body["keep"], false);
            body["id"].as_str().unwrap().to_owned()
        })
        .collect()
}

/// The `sf` of each `f` since `sent`.
fn frame_surfaces(terminal: &FakeTerminal, sent: usize) -> Vec<String> {
    terminal.packets[sent..]
        .iter()
        .filter(|(verb, _)| *verb == b'f')
        .map(|(_, body)| body["sf"].as_str().unwrap().to_owned())
        .collect()
}

fn shown_outer(server: &Server, terminal: &FakeTerminal) -> String {
    server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .projection
        .as_ref()
        .expect("a shown projection")
        .outer
        .clone()
}

/// The hidden projections' outer ids, least recently hidden first.
fn hidden_outers(server: &Server, terminal: &FakeTerminal) -> Vec<String> {
    server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .hidden
        .iter()
        .map(|p| p.outer.clone())
        .collect()
}

/// The outer ids of exactly two hidden projections, least recently hidden
/// first.
fn hidden_pair(server: &Server, terminal: &FakeTerminal) -> (String, String) {
    let hidden = hidden_outers(server, terminal);
    assert_eq!(hidden.len(), 2, "{hidden:?}");
    (hidden[0].clone(), hidden[1].clone())
}

fn hidden_panes(server: &Server, terminal: &FakeTerminal) -> Vec<PaneId> {
    server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .hidden
        .iter()
        .map(|p| p.pane)
        .collect()
}

fn hidden_cap(server: &Server, terminal: &FakeTerminal) -> usize {
    server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .hidden_cap()
}

/// The ids of the surfaces the host keeps parked, oldest first.
fn parked_ids(terminal: &FakeTerminal) -> Vec<String> {
    terminal
        .parked
        .iter()
        .map(|(tree, _)| tree["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The `ack` sequences the program in `pane` received since its output was
/// last taken.
fn take_pane_acks(server: &mut Server, pane: PaneId) -> Vec<u64> {
    let output = std::mem::take(&mut server.panes.get_mut(pane).unwrap().output);
    Framing::default()
        .feed(&output)
        .into_iter()
        .filter_map(|packet| match packet {
            Packet::Message(b'e', body) if body["ev"] == "ack" => body["s"].as_u64(),
            _ => None,
        })
        .collect()
}

/// A program frame `s` that sets the transcript of `pane` to `text`.
fn transcript_frame(server: &mut Server, pane: PaneId, s: u64, text: &str) {
    pane_runtime::pane_parse_buffer(
        server,
        pane,
        &apc(
            b'f',
            &json!({"sf":"program","s":s,"ops":[["set","transcript",{"text":text}]]}),
        ),
    )
    .unwrap();
}

fn renderer(server: &Server, pane: PaneId) -> broker::Renderer {
    server
        .panes
        .get(pane)
        .unwrap()
        .tsp
        .as_ref()
        .unwrap()
        .renderer
}

/// The `q hello` probes rmux sent the terminal so far.
fn probes(terminal: &FakeTerminal) -> usize {
    terminal
        .packets
        .iter()
        .filter(|(verb, body)| *verb == b'q' && body["q"] == "hello" && body["app"] == "rmux")
        .count()
}

/// Fire the client's pending probe timer: a deadline or a retry.
fn fire_probe_timer(server: &mut Server, terminal: &FakeTerminal) {
    let timer = server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .timer
        .expect("a probe timer");
    let crate::server::event_loop::LoopAction::Deferred(id) =
        *server.event_loop.timer_action(timer).unwrap()
    else {
        panic!("typed timeout")
    };
    server.deferred.remove(&id).unwrap()(server);
}

fn probe_timer_pending(server: &Server, terminal: &FakeTerminal) -> bool {
    server
        .clients
        .get(terminal.client)
        .unwrap()
        .tsp
        .timer
        .is_some()
}

fn client_tsp(server: &Server, terminal: &FakeTerminal) -> &'static str {
    server.clients.get(terminal.client).unwrap().tsp.format()
}

/// The client answered its probe: the program's next checkpoint renders it
/// native, with the transcript in the terminal's tree.
fn assert_settles_native(fixture: &mut Fixture, terminal: &mut FakeTerminal) {
    assert_eq!(client_tsp(&fixture.server, terminal), "v1");
    assert!(
        !probe_timer_pending(&fixture.server, terminal),
        "no retry is left"
    );
    fixture.settle(terminal);
    assert_eq!(
        super::contract::desired_view(&fixture.server, fixture.pane),
        "native"
    );
    assert_eq!(
        renderer(&fixture.server, fixture.pane),
        broker::Renderer::Native
    );
    assert_eq!(terminal.text("transcript"), "transcript:0");
    assert!(!terminal.alternate());
}

#[test]
fn probe_answered_after_its_deadline_still_makes_the_client_native() {
    let mut fixture = Fixture::new();
    let mut terminal =
        FakeTerminal::attach_holding_probe(&mut fixture.server, Some(fixture.session));
    assert_eq!(probes(&terminal), 1);
    assert_eq!(client_tsp(&fixture.server, &terminal), "unknown");
    assert_eq!(
        super::contract::desired_view(&fixture.server, fixture.pane),
        "pending"
    );

    fire_probe_timer(&mut fixture.server, &terminal);
    assert_eq!(client_tsp(&fixture.server, &terminal), "no");
    assert_eq!(
        super::contract::desired_view(&fixture.server, fixture.pane),
        "ansi"
    );
    assert!(
        probe_timer_pending(&fixture.server, &terminal),
        "a retry is scheduled"
    );
    fixture.settle(&mut terminal);
    assert!(
        terminal.tree.is_none(),
        "the client shows the grid meanwhile"
    );
    assert_eq!(terminal.row(0), "transcript:0");
    assert_eq!(probes(&terminal), 1, "the retry has not gone out yet");

    terminal.answer_probe(&mut fixture.server);
    assert_settles_native(&mut fixture, &mut terminal);
    assert_eq!(probes(&terminal), 1);
}

#[test]
fn retried_probe_is_answered_and_the_client_goes_native() {
    let mut fixture = Fixture::new();
    let mut terminal =
        FakeTerminal::attach_holding_probe(&mut fixture.server, Some(fixture.session));
    fire_probe_timer(&mut fixture.server, &terminal); // the deadline
    fixture.settle(&mut terminal);
    terminal.answers_probe = true;
    fire_probe_timer(&mut fixture.server, &terminal); // the retry
    assert_eq!(
        client_tsp(&fixture.server, &terminal),
        "no",
        "a retry keeps the grid the client fell back to"
    );
    assert_eq!(
        super::contract::desired_view(&fixture.server, fixture.pane),
        "ansi"
    );
    // The page catches up: the first probe's answer, now stale, then the
    // retry's, which counts.
    terminal.pump(&mut fixture.server);
    assert_eq!(probes(&terminal), 2);
    assert!(terminal.held_probe.is_none());
    assert_settles_native(&mut fixture, &mut terminal);
}

#[test]
fn input_after_an_unanswered_probe_retries_it_at_once() {
    let mut fixture = Fixture::new();
    let mut terminal =
        FakeTerminal::attach_holding_probe(&mut fixture.server, Some(fixture.session));
    fire_probe_timer(&mut fixture.server, &terminal);
    fixture.settle(&mut terminal);
    terminal.answers_probe = true;
    crate::cmd::key_bindings::init(&mut fixture.server).unwrap();
    terminal.send(&mut fixture.server, b"x");
    crate::cmd::queue::next(&mut fixture.server, Some(terminal.client));
    terminal.pump(&mut fixture.server);
    assert_eq!(probes(&terminal), 2);
    assert_settles_native(&mut fixture, &mut terminal);
}

#[test]
fn client_that_answers_da1_without_a_hello_is_not_probed_again() {
    let (mut server, pane, session) = model();
    let mut terminal = FakeTerminal::attach(&mut server, Some(session), false);
    assert_eq!(probes(&terminal), 1);
    assert_eq!(client_tsp(&server, &terminal), "no");
    assert!(!probe_timer_pending(&server, &terminal), "no retry");
    assert_eq!(super::contract::desired_view(&server, pane), "ansi");
    crate::cmd::key_bindings::init(&mut server).unwrap();
    terminal.send(&mut server, b"x");
    crate::cmd::queue::next(&mut server, Some(terminal.client));
    resize_terminal(&mut server, &mut terminal, 100, 30);
    assert_eq!(probes(&terminal), 1, "input and a resize send nothing more");
    assert_eq!(client_tsp(&server, &terminal), "no");
}

#[test]
fn hidden_cap_follows_the_host_hello() {
    for (hello, cap) in [
        (parked_hello(4), 4),
        (parked_hello(20), 8),
        (parked_hello(1), 1),
        (blobstore_hello(), 1),
        (rsttyd_1_4_hello(), 1),
        (tern_hello(), 0),
        (fixture_blobs_hello(), 0),
    ] {
        let name = format!("{} suspend {}", hello["term"], hello["suspend"]);
        let (server, _, terminal) = stock_native_with_hello(hello);
        assert_eq!(hidden_cap(&server, &terminal), cap, "{name}");
    }
}

#[test]
fn returning_to_a_parked_omp_window_resumes_it_with_one_frame_and_no_replay() {
    let (mut server, mut terminal, panes, ids) = omp_windows(parked_hello(4), 2);
    assert_eq!(terminal.opens, 2);
    assert_eq!(hidden_panes(&server, &terminal), vec![panes[0]]);
    let a_outer = hidden_outers(&server, &terminal)[0].clone();
    let b_outer = shown_outer(&server, &terminal);
    assert_eq!(parked_ids(&terminal), vec![a_outer.clone()]);
    let c = server.clients.get(terminal.client).unwrap();
    let a_generation = c.tsp.hidden[0].generation;

    let sent = terminal.packets.len();
    let bytes = terminal.bytes.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(
        verbs,
        vec!['f', 'f'],
        "suspend and resume, nothing else: {verbs:?}"
    );
    assert_eq!(
        frame_heads(&terminal, sent),
        vec![json!(["suspend"]), json!(["resume"])]
    );
    assert_eq!(
        frame_surfaces(&terminal, sent),
        vec![b_outer.clone(), a_outer.clone()]
    );
    let written = &terminal.bytes[bytes..];
    assert!(queries(written).is_empty(), "a query on the wire");
    assert!(fences(written).is_empty(), "a DA1 on the wire");
    assert_eq!(terminal.opens, 2);
    assert_eq!(
        projection_generation(&server, &terminal),
        Some(a_generation)
    );
    assert_eq!(hidden_outers(&server, &terminal), vec![b_outer.clone()]);
    assert_eq!(parked_ids(&terminal), vec![b_outer.clone()]);
    assert!(
        !terminal.alternate(),
        "the inline surface is on the main screen"
    );
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert_both_shots(&mut terminal, &ids);

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x021");
    assert_eq!(verbs_since(&terminal, sent), vec!['f', 'f']);
    assert_eq!(
        frame_heads(&terminal, sent),
        vec![json!(["suspend"]), json!(["resume"])]
    );
    assert_eq!(
        frame_surfaces(&terminal, sent),
        vec![a_outer.clone(), b_outer]
    );
    assert_eq!(terminal.text("transcript"), "transcript 1");
    assert_eq!(hidden_outers(&server, &terminal), vec![a_outer]);
    assert_eq!(terminal.opens, 2);
}

#[test]
fn over_the_cap_the_least_recently_parked_window_is_closed_and_returns_with_a_replay() {
    let (mut server, mut terminal, a, ids) = shots_window(parked_hello(2));
    assert_eq!(hidden_cap(&server, &terminal), 2);
    let a_outer = shown_outer(&server, &terminal);
    let b = new_window(&mut server, &mut terminal, 1);
    start_omp(&mut server, &mut terminal, b, "transcript 1", json!([]));
    let b_outer = shown_outer(&server, &terminal);
    let c = new_window(&mut server, &mut terminal, 2);
    start_omp(&mut server, &mut terminal, c, "transcript 2", json!([]));
    let c_outer = shown_outer(&server, &terminal);
    assert_eq!(
        hidden_outers(&server, &terminal),
        vec![a_outer.clone(), b_outer.clone()]
    );
    assert_eq!(
        parked_ids(&terminal),
        vec![a_outer.clone(), b_outer.clone()]
    );

    // Hiding C fills the cap: A, the least recently hidden, goes.
    let sent = terminal.packets.len();
    let d = new_window(&mut server, &mut terminal, 3);
    assert_eq!(closes_since(&terminal, sent), vec![a_outer.clone()]);
    assert_eq!(frame_heads(&terminal, sent), vec![json!(["suspend"])]);
    assert_eq!(
        hidden_outers(&server, &terminal),
        vec![b_outer.clone(), c_outer.clone()]
    );
    assert_eq!(parked_ids(&terminal), vec![b_outer.clone()]);
    assert!(
        terminal.suspended(),
        "C waits on the host for the next open"
    );
    assert!(terminal.alternate());
    let sent = terminal.packets.len();
    start_omp(&mut server, &mut terminal, d, "transcript 3", json!([]));
    assert!(closes_since(&terminal, sent).is_empty());
    assert_eq!(
        parked_ids(&terminal),
        vec![b_outer.clone(), c_outer.clone()]
    );
    let d_outer = shown_outer(&server, &terminal);
    assert_eq!(terminal.opens, 4);

    // Back to A: D hides and evicts B; A is gone on both sides, so it opens
    // again, asking the blob store first.
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(closes_since(&terminal, sent), vec![b_outer]);
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert!(
        frame_heads(&terminal, sent)
            .iter()
            .all(|head| head != &json!(["resume"])),
        "nothing to resume"
    );
    assert_eq!(terminal.opens, 5);
    assert_ne!(shown_outer(&server, &terminal), a_outer);
    assert_eq!(hidden_outers(&server, &terminal), vec![c_outer, d_outer]);
    assert_eq!(parked_ids(&terminal), hidden_outers(&server, &terminal));
    assert_eq!(hidden_panes(&server, &terminal), vec![c, d]);
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert_both_shots(&mut terminal, &ids);
    let client = server.clients.get(terminal.client).unwrap();
    assert_eq!(client.tsp.projection.as_ref().unwrap().pane, a);
}

/// rsttyd without `suspend` keeps one slot, and `#replaceInline` ends it
/// without a word, so another window's open closes it first, as in 95ae4e9.
#[test]
fn legacy_rsttyd_keeps_one_parked_window_and_closes_it_before_another_opens() {
    for hello in [blobstore_hello(), rsttyd_1_4_hello()] {
        let version = hello["version"].clone();
        let asks = hello["features"]
            .as_array()
            .unwrap()
            .contains(&json!("blobstore"));
        let (mut server, mut terminal, _, ids) = shots_window(hello);
        assert_eq!(hidden_cap(&server, &terminal), 1, "{version}");
        let a_outer = shown_outer(&server, &terminal);

        let sent = terminal.packets.len();
        let b = new_window(&mut server, &mut terminal, 1);
        assert_eq!(verbs_since(&terminal, sent), vec!['f'], "{version}");
        assert_eq!(frame_heads(&terminal, sent), vec![json!(["suspend"])]);
        assert_eq!(hidden_outers(&server, &terminal), vec![a_outer.clone()]);
        assert!(terminal.suspended(), "{version}");

        let sent = terminal.packets.len();
        start_omp(&mut server, &mut terminal, b, "transcript 1", json!([]));
        let verbs = verbs_since(&terminal, sent);
        assert!(
            verbs.iter().position(|v| *v == 'x') < verbs.iter().position(|v| *v == 'o'),
            "{version}: the parked window closes before the other opens: {verbs:?}"
        );
        assert_eq!(closes_since(&terminal, sent), vec![a_outer], "{version}");
        assert!(hidden_outers(&server, &terminal).is_empty(), "{version}");
        assert!(parked_ids(&terminal).is_empty(), "{version}");
        let b_outer = shown_outer(&server, &terminal);

        let sent = terminal.packets.len();
        switch_window(&mut server, &mut terminal, b"\x020");
        let verbs = verbs_since(&terminal, sent);
        // As at a419ed2: pane A's lower id puts its recompute first, so
        // `project_pending` closes B before anything would hide it, and A's
        // replay is the only frame.
        assert!(
            frame_heads(&terminal, sent)
                .iter()
                .all(|head| head != &json!(["resume"])),
            "{version}: nothing to resume"
        );
        assert!(
            verbs.iter().position(|v| *v == 'x') < verbs.iter().position(|v| *v == 'o'),
            "{version}: {verbs:?}"
        );
        assert_eq!(closes_since(&terminal, sent), vec![b_outer], "{version}");
        if asks {
            assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
            assert_eq!(
                verbs_from_open(&terminal, sent),
                vec!['o', 'f'],
                "{version}"
            );
        } else {
            assert!(blobs_queries_since(&terminal, sent).is_empty());
            assert_eq!(
                verbs_from_open(&terminal, sent),
                vec!['o', 'b', 'b', 'f'],
                "{version}"
            );
        }
        assert!(hidden_outers(&server, &terminal).is_empty(), "{version}");
        assert_eq!(terminal.opens, 3, "{version}");
        assert_eq!(terminal.text("transcript"), "retained transcript");
        assert_both_shots(&mut terminal, &ids);
    }
}

/// A host rmux does not know closes on every switch and gets every blob
/// again with no query and no DA1, as with bb69734.
#[test]
fn unknown_host_keeps_the_close_and_replay_switch_with_no_query() {
    let (mut server, mut terminal, _, ids) = shots_window(fixture_blobs_hello());
    assert_eq!(hidden_cap(&server, &terminal), 0);
    let a_outer = shown_outer(&server, &terminal);
    let bytes = terminal.bytes.len();

    let sent = terminal.packets.len();
    let b = new_window(&mut server, &mut terminal, 1);
    assert_eq!(verbs_since(&terminal, sent), vec!['x']);
    assert_eq!(closes_since(&terminal, sent), vec![a_outer]);
    assert!(terminal.tree.is_none(), "the host dropped the surface");
    assert!(terminal.alternate());
    assert!(hidden_outers(&server, &terminal).is_empty());

    let sent = terminal.packets.len();
    start_omp(&mut server, &mut terminal, b, "transcript 1", json!([]));
    // The replay of the just-opened empty surface, then the program's frame.
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(verbs.first(), Some(&'o'), "{verbs:?}");
    assert!(verbs[1..].iter().all(|verb| *verb == 'f'), "{verbs:?}");
    let b_outer = shown_outer(&server, &terminal);

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(verbs_since(&terminal, sent), vec!['x', 'o', 'b', 'b', 'f']);
    assert_eq!(closes_since(&terminal, sent), vec![b_outer]);
    let mut sent_blobs = blobs_since(&terminal, sent);
    sent_blobs.sort();
    assert_eq!(sent_blobs, ids);
    let written = &terminal.bytes[bytes..];
    assert!(queries(written).is_empty(), "a query on the wire");
    assert!(fences(written).is_empty(), "a DA1 on the wire");
    assert!(hidden_outers(&server, &terminal).is_empty());
    assert!(parked_ids(&terminal).is_empty());
    assert_eq!(terminal.opens, 3);
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert_both_shots(&mut terminal, &ids);
}

#[test]
fn host_gone_for_one_parked_window_drops_only_that_entry_with_no_close() {
    let (mut server, mut terminal, panes, ids) = omp_windows(parked_hello(4), 3);
    let (a_outer, b_outer) = hidden_pair(&server, &terminal);
    let c_outer = shown_outer(&server, &terminal);

    // The page evicted A: it is gone there already.
    terminal.parked.retain(|(tree, _)| tree["id"] != a_outer);
    let sent = terminal.packets.len();
    terminal.send(
        &mut server,
        &apc(b'e', &json!({"ev":"gone","sf":a_outer,"ids":[a_outer]})),
    );
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert!(
        verbs_since(&terminal, sent).is_empty(),
        "nothing to the host: {:?}",
        verbs_since(&terminal, sent)
    );
    assert_eq!(hidden_outers(&server, &terminal), vec![b_outer.clone()]);
    assert_eq!(shown_outer(&server, &terminal), c_outer);
    assert_eq!(parked_ids(&terminal), vec![b_outer.clone()]);
    assert_eq!(terminal.text("transcript"), "transcript 2");

    // B still resumes; A opens again, asking the blob store first.
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x021");
    assert_eq!(verbs_since(&terminal, sent), vec!['f', 'f']);
    assert_eq!(frame_surfaces(&terminal, sent)[1], b_outer);
    assert_eq!(terminal.text("transcript"), "transcript 1");
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert!(closes_since(&terminal, sent).is_empty());
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert_eq!(terminal.opens, 4);
    assert_eq!(hidden_panes(&server, &terminal), vec![panes[2], panes[1]]);
    assert_both_shots(&mut terminal, &ids);
}

#[test]
fn host_error_for_one_parked_window_closes_only_that_entry() {
    let (mut server, mut terminal, panes, _) = omp_windows(parked_hello(4), 3);
    let (a_outer, b_outer) = hidden_pair(&server, &terminal);
    let c_outer = shown_outer(&server, &terminal);
    let c_generation = projection_generation(&server, &terminal);

    let sent = terminal.packets.len();
    terminal.send(
        &mut server,
        &apc(
            b'e',
            &json!({"ev":"error","sf":b_outer,"msg":"surface rejected"}),
        ),
    );
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(verbs_since(&terminal, sent), vec!['x']);
    assert_eq!(closes_since(&terminal, sent), vec![b_outer]);
    assert_eq!(hidden_outers(&server, &terminal), vec![a_outer.clone()]);
    assert_eq!(parked_ids(&terminal), vec![a_outer.clone()]);
    assert_eq!(shown_outer(&server, &terminal), c_outer);
    assert_eq!(projection_generation(&server, &terminal), c_generation);
    assert!(
        !terminal.alternate(),
        "the shown surface stays on the main screen"
    );
    assert_eq!(terminal.text("transcript"), "transcript 2");

    // A still resumes.
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(verbs_since(&terminal, sent), vec!['f', 'f']);
    assert_eq!(frame_surfaces(&terminal, sent)[1], a_outer);
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert_eq!(hidden_panes(&server, &terminal), vec![panes[2]]);
}

/// A parked window's program keeps drawing. Nobody draws its frames, so they
/// are acked at once and the program never stalls; the return carries them.
#[test]
fn programs_of_parked_windows_keep_their_acks_and_their_frames_come_with_the_resume() {
    let (mut server, mut terminal, panes, _) = omp_windows(parked_hello(4), 3);
    let (a, b) = (panes[0], panes[1]);
    take_pane_acks(&mut server, a);
    take_pane_acks(&mut server, b);
    assert_eq!(renderer(&server, a), broker::Renderer::Detached);
    assert_eq!(renderer(&server, b), broker::Renderer::Detached);

    let sent = terminal.packets.len();
    for s in 3..=6 {
        transcript_frame(&mut server, a, s, &format!("a:{s}"));
        transcript_frame(&mut server, b, s - 1, &format!("b:{}", s - 1));
        terminal.pump(&mut server);
        terminal.draw(&mut server);
        assert_eq!(take_pane_acks(&mut server, a), vec![s], "a:{s}");
        assert_eq!(take_pane_acks(&mut server, b), vec![s - 1], "b:{}", s - 1);
    }
    assert!(
        verbs_since(&terminal, sent).is_empty(),
        "a parked surface gets nothing: {:?}",
        verbs_since(&terminal, sent)
    );
    assert_eq!(terminal.text("transcript"), "transcript 2");

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(verbs_since(&terminal, sent), vec!['f', 'f']);
    assert_eq!(frame_heads(&terminal, sent)[1], json!(["resume"]));
    assert_eq!(terminal.text("transcript"), "a:6");
    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x021");
    assert_eq!(verbs_since(&terminal, sent), vec!['f', 'f']);
    assert_eq!(terminal.text("transcript"), "b:5");
    assert_eq!(terminal.opens, 3);
}

#[test]
fn stopping_the_tty_closes_every_parked_window_and_the_resync_forgets_them() {
    let (mut server, mut terminal, _, _) = omp_windows(parked_hello(4), 4);
    let mut outers = hidden_outers(&server, &terminal);
    assert_eq!(outers.len(), 3);
    outers.push(shown_outer(&server, &terminal));
    outers.sort();

    let sent = terminal.packets.len();
    server
        .clients
        .get_mut(terminal.client)
        .unwrap()
        .tty
        .as_mut()
        .unwrap()
        .stop(&mut server.tparm, &TtyOptions::default());
    terminal.pump(&mut server);
    let mut closed = closes_since(&terminal, sent);
    closed.sort();
    assert_eq!(closed, outers, "an `x` for each surface the host has");
    assert!(terminal.tree.is_none());
    assert!(parked_ids(&terminal).is_empty());
    assert!(
        !terminal.alternate(),
        "the restore leaves the alternate screen"
    );

    broker::client_sync(&mut server, terminal.client);
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.hidden.is_empty() && c.tsp.projection.is_none());
    assert_eq!(c.tsp.format(), "unknown");
}

#[test]
fn a_parked_window_whose_program_exits_is_closed_alone() {
    let (mut server, mut terminal, panes, _) = omp_windows(parked_hello(4), 3);
    let (a_outer, b_outer) = hidden_pair(&server, &terminal);
    let c_outer = shown_outer(&server, &terminal);

    let sent = terminal.packets.len();
    broker::pane_exited(&mut server, panes[1]);
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(closes_since(&terminal, sent), vec![b_outer]);
    assert_eq!(hidden_outers(&server, &terminal), vec![a_outer.clone()]);
    assert_eq!(parked_ids(&terminal), vec![a_outer]);
    assert_eq!(shown_outer(&server, &terminal), c_outer);
    assert!(!terminal.alternate());
    assert_eq!(terminal.text("transcript"), "transcript 2");
    assert_eq!(terminal.opens, 3);
}

#[test]
fn refreshing_the_client_closes_every_parked_window_and_detects_the_host_again() {
    let (mut server, mut terminal, _, _) = omp_windows(parked_hello(4), 3);
    let mut outers = hidden_outers(&server, &terminal);
    outers.push(shown_outer(&server, &terminal));
    outers.sort();

    let sent = terminal.packets.len();
    broker::refresh_client(&mut server, terminal.client);
    settle_replay(&mut server, &mut terminal);
    let mut closed = closes_since(&terminal, sent);
    closed.sort();
    assert_eq!(closed, outers);
    assert!(
        terminal.packets[sent..]
            .iter()
            .any(|(verb, body)| *verb == b'q' && body["q"] == "hello" && body["app"] == "rmux"),
        "the host is detected again"
    );
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.hidden.is_empty());
    assert_eq!(c.tsp.format(), "v1");
    assert_eq!(hidden_cap(&server, &terminal), 4);
    assert!(parked_ids(&terminal).is_empty());
    assert_eq!(terminal.opens, 4, "the shown window opens again");
    assert!(
        frame_heads(&terminal, sent)
            .iter()
            .all(|head| head != &json!(["resume"]))
    );
    assert_eq!(terminal.text("transcript"), "transcript 2");
}

/// The page reloaded into an rsttyd that keeps one slot while rmux still
/// holds the `suspend: 3` hello it detected (plan 2.4): the page ended A on
/// B's open, so A's `["resume"]` gets `error unknown surface` and A replays.
#[test]
fn resume_rejected_by_a_rolled_back_host_closes_that_entry_and_replays_it() {
    let (mut server, mut terminal, _, ids) = shots_window(parked_hello(3));
    assert_eq!(hidden_cap(&server, &terminal), 3);
    terminal.parks = 0;
    let a_outer = shown_outer(&server, &terminal);
    let b = new_window(&mut server, &mut terminal, 1);
    start_omp(&mut server, &mut terminal, b, "transcript 1", json!([]));
    assert_eq!(hidden_outers(&server, &terminal), vec![a_outer.clone()]);
    assert!(parked_ids(&terminal).is_empty(), "the old page ended A");

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(
        frame_heads(&terminal, sent)[..2],
        [json!(["suspend"]), json!(["resume"])]
    );
    assert_eq!(frame_surfaces(&terminal, sent)[1], a_outer);
    assert!(
        closes_since(&terminal, sent).contains(&a_outer),
        "the rejected surface closes: {verbs:?}"
    );
    assert_eq!(blobs_queries_since(&terminal, sent), vec![json!(ids)]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert_eq!(terminal.opens, 3);
    assert_ne!(shown_outer(&server, &terminal), a_outer);
    assert!(
        !hidden_outers(&server, &terminal).contains(&a_outer),
        "no second resume of a surface the host has not"
    );
    assert!(!terminal.alternate());
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert_both_shots(&mut terminal, &ids);
}

/// rsttyd leaves a window that another client, on a host that closes, still
/// shows: the pane stays native, rsttyd's projection parks and resumes, and
/// the other client hears nothing of it.
#[test]
fn a_window_another_client_keeps_showing_parks_and_resumes_for_rsttyd_alone() {
    let (mut server, mut rsttyd, a, ids) = shots_window(parked_hello(4));
    let window = server.panes.get(a).unwrap().window;
    let options = server.options.create(Some(server.options.global_s));
    let other = session::session_create(
        &mut server,
        session::SessionCreate {
            prefix: None,
            name: Some(b"tern".to_vec()),
            cwd: b"/".to_vec(),
            environment: crate::options::environment::Environment::default(),
            options,
            termios: None,
        },
    );
    let link = session::session_attach(&mut server, other, window, 0).unwrap();
    session::session_set_current(&mut server, other, Some(link));
    let mut tern =
        FakeTerminal::attach_with_hello(&mut server, Some(other), true, fixture_blobs_hello());
    broker::recompute(&mut server);
    settle_replay(&mut server, &mut tern);
    assert_eq!(hidden_cap(&server, &tern), 0);
    assert_eq!(tern.text("transcript"), "retained transcript");
    assert_both_shots(&mut tern, &ids);
    // The second viewer narrowed the stock contract, which reopened rsttyd's
    // projection once; the contract stays the same while rsttyd comes and goes.
    settle_replay(&mut server, &mut rsttyd);
    assert_eq!(rsttyd.text("transcript"), "retained transcript");
    let opens = rsttyd.opens;
    let tern_sent = tern.packets.len();
    let a_outer = shown_outer(&server, &rsttyd);

    let sent = rsttyd.packets.len();
    let b = new_window(&mut server, &mut rsttyd, 1);
    assert_eq!(verbs_since(&rsttyd, sent), vec!['f']);
    assert_eq!(frame_heads(&rsttyd, sent), vec![json!(["suspend"])]);
    assert_eq!(hidden_outers(&server, &rsttyd), vec![a_outer.clone()]);
    assert!(rsttyd.alternate());
    assert_eq!(renderer(&server, a), broker::Renderer::Native);
    tern.pump(&mut server);
    assert_eq!(
        tern.packets.len(),
        tern_sent,
        "the other client heard nothing"
    );

    // The program draws on for the other client; rsttyd's parked copy
    // keeps the frame for its return.
    let sent = rsttyd.packets.len();
    transcript_frame(&mut server, a, 3, "while away");
    tern.pump(&mut server);
    tern.draw(&mut server);
    rsttyd.pump(&mut server);
    assert_eq!(tern.text("transcript"), "while away");
    assert!(verbs_since(&rsttyd, sent).is_empty());
    let tern_sent = tern.packets.len();

    start_omp(&mut server, &mut rsttyd, b, "transcript 1", json!([]));
    let b_outer = shown_outer(&server, &rsttyd);
    let sent = rsttyd.packets.len();
    switch_window(&mut server, &mut rsttyd, b"\x020");
    assert_eq!(verbs_since(&rsttyd, sent), vec!['f', 'f']);
    assert_eq!(
        frame_heads(&rsttyd, sent),
        vec![json!(["suspend"]), json!(["resume"])]
    );
    assert_eq!(
        frame_surfaces(&rsttyd, sent),
        vec![b_outer.clone(), a_outer]
    );
    assert_eq!(rsttyd.text("transcript"), "while away");
    assert_both_shots(&mut rsttyd, &ids);
    assert_eq!(hidden_outers(&server, &rsttyd), vec![b_outer]);
    assert_eq!(rsttyd.opens, opens + 1, "only B's open");

    tern.pump(&mut server);
    assert_eq!(
        tern.packets.len(),
        tern_sent,
        "the other client heard nothing: {:?}",
        verbs_since(&tern, tern_sent)
    );
    assert_eq!(tern.text("transcript"), "while away");
}

/// The program in `pane` draws `count` frames from `s` on, which the host
/// receives but does not draw: its credits are used up.
fn queue_undrawn_frames(
    server: &mut Server,
    terminal: &mut FakeTerminal,
    pane: PaneId,
    s: u64,
    count: u64,
) {
    let queued = terminal.frames.len();
    for s in s..s + count {
        transcript_frame(server, pane, s, &format!("b:{s}"));
    }
    terminal.pump(server);
    assert_eq!(
        terminal.frames.len(),
        queued + count as usize,
        "the frames reached the host and wait for its draw"
    );
}

/// rsttyd `#replaceInline` parks the previous surface only when it is
/// suspended; a `["suspend"]` that waits for credit must not be overtaken by
/// the next window's `o` or `["resume"]`. Either B's suspend reaches the host
/// first or rmux closes B and replays it later; B never ends silently and its
/// program never stalls.
#[test]
fn a_window_left_without_credit_is_parked_in_order_or_closed_never_lost() {
    let (mut server, mut terminal, panes, ids) = omp_windows(parked_hello(4), 2);
    let (a, b) = (panes[0], panes[1]);
    let a_outer = hidden_outers(&server, &terminal)[0].clone();
    let b_outer = shown_outer(&server, &terminal);
    queue_undrawn_frames(&mut server, &mut terminal, b, 2, 2);
    take_pane_acks(&mut server, b);

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert!(!terminal.alternate());
    assert_eq!(terminal.tree.as_ref().unwrap()["id"], a_outer);
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert_both_shots(&mut terminal, &ids);
    assert!(!hidden_panes(&server, &terminal).contains(&a));
    let parked = parked_ids(&terminal).contains(&b_outer);
    let closed = closes_since(&terminal, sent).contains(&b_outer);
    assert!(
        parked != closed,
        "B is parked or closed, not both and not neither: parked {parked}, closed {closed}"
    );
    assert_eq!(
        hidden_outers(&server, &terminal).contains(&b_outer),
        parked,
        "rmux and the host agree about B"
    );

    // The return shows B's latest frame, resumed or replayed, and B goes on.
    switch_window(&mut server, &mut terminal, b"\x021");
    assert!(!terminal.alternate());
    assert_eq!(terminal.text("transcript"), "b:3");
    take_pane_acks(&mut server, b);
    transcript_frame(&mut server, b, 4, "b:4");
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(terminal.text("transcript"), "b:4");
    assert_eq!(
        take_pane_acks(&mut server, b),
        vec![4],
        "B's program is not stalled"
    );
}

/// A shown window that cannot park (its rewind page is open) closes when the
/// client leaves for a shell window; the parked windows stay parked.
#[test]
fn leaving_a_window_with_a_page_for_the_grid_keeps_the_other_windows_parked() {
    let (mut server, mut terminal, panes, _) = omp_windows(parked_hello(4), 2);
    let a_outer = hidden_outers(&server, &terminal)[0].clone();
    let b_outer = shown_outer(&server, &terminal);
    open_page_over_transcript(&mut server, panes[1], &mut terminal);
    let page_outer = shown_outer(&server, &terminal);
    assert_ne!(page_outer, b_outer);
    assert_eq!(terminal.text("turn"), "turn 1");

    let sent = terminal.packets.len();
    new_window(&mut server, &mut terminal, 2);
    let mut closed = closes_since(&terminal, sent);
    closed.sort();
    let mut both = vec![page_outer, b_outer];
    both.sort();
    assert_eq!(closed, both, "B's page and transcript close, A stays");
    assert!(terminal.tree.is_none());
    assert!(terminal.covered.is_none());
    assert!(terminal.alternate());
    assert_eq!(hidden_outers(&server, &terminal), vec![a_outer.clone()]);
    assert_eq!(parked_ids(&terminal), vec![a_outer.clone()]);
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.projection.is_none() && c.tsp.covered.is_none());

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(verbs_since(&terminal, sent), vec!['f']);
    assert_eq!(frame_heads(&terminal, sent), vec![json!(["resume"])]);
    assert_eq!(frame_surfaces(&terminal, sent), vec![a_outer]);
    assert_eq!(terminal.text("transcript"), "retained transcript");
}

/// Returning to a parked window from one whose rewind page is open: the page
/// and the transcript under it close first, then the parked one resumes. The
/// later return to the page window opens it afresh with no rejected surface.
#[test]
fn resuming_a_parked_window_over_a_page_closes_the_page_and_its_transcript_first() {
    let (mut server, mut terminal, panes, ids) = omp_windows(parked_hello(4), 2);
    let p_outer = hidden_outers(&server, &terminal)[0].clone();
    let x_inline = shown_outer(&server, &terminal);
    open_page_over_transcript(&mut server, panes[1], &mut terminal);
    let x_page = shown_outer(&server, &terminal);
    assert_eq!(
        server
            .clients
            .get(terminal.client)
            .unwrap()
            .tsp
            .covered
            .as_ref()
            .map(|p| p.outer.clone()),
        Some(x_inline.clone())
    );

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    let verbs = verbs_since(&terminal, sent);
    let mut closed = closes_since(&terminal, sent);
    closed.sort();
    let mut both = vec![x_page, x_inline];
    both.sort();
    assert_eq!(closed, both, "{verbs:?}");
    assert!(!verbs.contains(&'o'), "{verbs:?}");
    assert_eq!(frame_heads(&terminal, sent), vec![json!(["resume"])]);
    assert_eq!(frame_surfaces(&terminal, sent), vec![p_outer.clone()]);
    assert!(
        verbs.iter().position(|v| *v == 'x') < verbs.iter().position(|v| *v == 'f'),
        "the closes go before the resume: {verbs:?}"
    );
    assert_eq!(shown_outer(&server, &terminal), p_outer);
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.covered.is_none(), "no dead covered entry");
    assert!(terminal.covered.is_none());
    assert!(!terminal.alternate());
    assert_eq!(terminal.text("transcript"), "retained transcript");
    assert_both_shots(&mut terminal, &ids);

    let sent = terminal.packets.len();
    let opens = terminal.opens;
    switch_window(&mut server, &mut terminal, b"\x021");
    assert!(verbs_since(&terminal, sent).contains(&'o'));
    assert_eq!(terminal.opens, opens + 1, "the page opens once");
    assert_eq!(terminal.text("turn"), "turn 1");
    assert_eq!(hidden_outers(&server, &terminal), vec![p_outer]);
    let c = server.clients.get(terminal.client).unwrap();
    assert!(!c.tsp.rebuild_attempted, "no surface was rejected");
}

/// A parked window's program exits while the shown window has its rewind page
/// open: only that window's `x` goes, with no grid restore under the live
/// pair, and the page's queued frame is not lost.
#[test]
fn closing_a_parked_window_under_a_page_sends_a_bare_close_and_keeps_queued_frames() {
    let (mut server, mut terminal, panes, _) = omp_windows(parked_hello(4), 2);
    let p_outer = hidden_outers(&server, &terminal)[0].clone();
    open_page_over_transcript(&mut server, panes[1], &mut terminal);
    let x_page = shown_outer(&server, &terminal);
    pane_runtime::pane_parse_buffer(
        &mut server,
        panes[1],
        &apc(
            b'f',
            &json!({"sf":"page","s":2,"ops":[["set","turn",{"text":"turn 2"}]]}),
        ),
    )
    .unwrap();

    let sent = terminal.packets.len();
    let bytes = terminal.bytes.len();
    broker::pane_exited(&mut server, panes[0]);
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert_eq!(closes_since(&terminal, sent), vec![p_outer]);
    assert!(
        !bytes_contain(&terminal.bytes[bytes..], BACK_TO_GRID),
        "no anchor row removed under the live pair"
    );
    assert!(!terminal.alternate(), "the pair stays on the main screen");
    assert_eq!(
        terminal.text("turn"),
        "turn 2",
        "the queued page frame arrived"
    );
    assert_eq!(shown_outer(&server, &terminal), x_page);
    assert!(terminal.covered.is_some());
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.covered.is_some() && c.tsp.hidden.is_empty());
}

/// With the parked set full, a switch to one of the parked windows must not
/// evict that very window: the shown one parks and the target resumes.
#[test]
fn switching_to_a_parked_window_with_a_full_set_never_evicts_the_target() {
    let (mut server, mut terminal, _, _) = shots_window(parked_hello(2));
    for index in 1..4 {
        let pane = new_window(&mut server, &mut terminal, index);
        start_omp(
            &mut server,
            &mut terminal,
            pane,
            &format!("transcript {index}"),
            json!([]),
        );
    }
    switch_window(&mut server, &mut terminal, b"\x020");
    let a_outer = shown_outer(&server, &terminal);
    let (c_outer, d_outer) = hidden_pair(&server, &terminal);
    assert_eq!(
        parked_ids(&terminal),
        vec![c_outer.clone(), d_outer.clone()]
    );

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x022");
    let verbs = verbs_since(&terminal, sent);
    assert_eq!(verbs, vec!['f', 'f'], "suspend and resume only: {verbs:?}");
    assert_eq!(
        frame_heads(&terminal, sent),
        vec![json!(["suspend"]), json!(["resume"])]
    );
    assert_eq!(
        frame_surfaces(&terminal, sent),
        vec![a_outer.clone(), c_outer.clone()]
    );
    assert_eq!(terminal.text("transcript"), "transcript 2");
    assert_eq!(shown_outer(&server, &terminal), c_outer);
    assert_eq!(hidden_outers(&server, &terminal), vec![d_outer, a_outer]);
    assert_eq!(parked_ids(&terminal), hidden_outers(&server, &terminal));
}

/// A host error on the shown surface rebuilds that projection alone; the
/// parked windows are not closed with it.
#[test]
fn host_error_on_the_shown_window_rebuilds_it_and_keeps_the_parked_windows() {
    let (mut server, mut terminal, _, _) = omp_windows(parked_hello(4), 3);
    let (a_outer, b_outer) = hidden_pair(&server, &terminal);
    let c_outer = shown_outer(&server, &terminal);

    let sent = terminal.packets.len();
    terminal.send(
        &mut server,
        &apc(b'e', &json!({"ev":"error","sf":c_outer,"msg":"bad op"})),
    );
    settle_replay(&mut server, &mut terminal);
    assert_eq!(closes_since(&terminal, sent), vec![c_outer.clone()]);
    assert_eq!(verbs_from_open(&terminal, sent), vec!['o', 'f']);
    assert_ne!(shown_outer(&server, &terminal), c_outer);
    assert_eq!(
        hidden_outers(&server, &terminal),
        vec![a_outer.clone(), b_outer.clone()]
    );
    assert_eq!(parked_ids(&terminal), vec![a_outer.clone(), b_outer]);
    assert_eq!(terminal.opens, 4);
    assert!(!terminal.alternate());
    assert_eq!(terminal.text("transcript"), "transcript 2");

    let sent = terminal.packets.len();
    switch_window(&mut server, &mut terminal, b"\x020");
    assert_eq!(verbs_since(&terminal, sent), vec!['f', 'f']);
    assert_eq!(frame_surfaces(&terminal, sent)[1], a_outer);
    assert_eq!(terminal.text("transcript"), "retained transcript");
}

/// `MSG_RESIZE` after the terminal changed to `sx`x`sy`, run through the
/// server loop's order: the message, its effects, then the client tick.
fn resize_terminal(server: &mut Server, terminal: &mut FakeTerminal, sx: u16, sy: u16) {
    let tty = server.clients.get(terminal.client).unwrap().tty.as_ref();
    rmux_sys::pty::set_winsize(
        tty.unwrap().fd(),
        Winsize {
            rows: sy,
            cols: sx,
            xpixel: sx * 8,
            ypixel: sy * 16,
        },
    )
    .unwrap();
    crate::client::dispatch::on_message(
        server,
        terminal.client,
        crate::server::protocol::ProtocolMessage::new(
            crate::server::protocol::ProtocolMessageKind::Resize,
            Vec::new(),
        ),
    );
    crate::server::run::drain_effects(server).unwrap();
    crate::client::tick::tick(server);
    crate::server::run::drain_effects(server).unwrap();
    terminal.pump(server);
    terminal.draw(server);
}

#[test]
fn rows_only_client_resize_keeps_the_native_projection() {
    let (mut server, pane, mut terminal) = stock_native();
    let client = terminal.client;
    let generation = |server: &Server| {
        let c = server.clients.get(client).unwrap();
        c.tsp.projection.as_ref().map(|p| p.generation)
    };
    let before = generation(&server);
    assert!(before.is_some());
    let window = server.panes.get(pane).unwrap().window;
    // A real terminal client, the program's cursor on the last row: a
    // smaller tty follows it with a window offset.
    server
        .clients
        .get_mut(client)
        .unwrap()
        .flags
        .insert(ClientFlags::TERMINAL);
    pane_runtime::pane_parse_buffer(&mut server, pane, b"\x1b[24;1H\x1b[?25h").unwrap();
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    let sent = terminal.packets.len();
    // A phone's soft keyboard opens, then closes: only the rows change.
    for (rows, window_rows) in [(19, 18), (25, 24)] {
        resize_terminal(&mut server, &mut terminal, 80, rows);
        let w = server.windows.get(window).unwrap();
        assert_eq!((w.sx, w.sy), (80, window_rows));
        assert_eq!(generation(&server), before, "{rows} rows re-projected");
        assert_eq!(
            server
                .panes
                .get(pane)
                .unwrap()
                .tsp
                .as_ref()
                .unwrap()
                .renderer,
            broker::Renderer::Native
        );
    }
    let verbs: Vec<char> = terminal.packets[sent..]
        .iter()
        .map(|(verb, _)| char::from(*verb))
        .collect();
    assert!(
        verbs.iter().all(|verb| !"xostb".contains(*verb)),
        "re-projected on a rows-only resize: {verbs:?}"
    );
    assert_eq!(terminal.opens, 1);
    assert_eq!(terminal.text("transcript"), "retained transcript");
}

#[test]
fn client_that_stays_smaller_than_the_window_gets_no_projection() {
    let (mut server, pane, mut big) = stock_native();
    let window = server.panes.get(pane).unwrap().window;
    let options = server.windows.get(window).unwrap().options;
    // `window-size largest`: the bigger client keeps the window at 80x24.
    server.options.set_number_value(options, b"window-size", 0);
    let session = server.clients.get(big.client).unwrap().session;
    let mut small = FakeTerminal::attach(&mut server, session, true);
    server.clients.get_mut(big.client).unwrap().activity_time = (i64::MAX, 0);
    broker::recompute(&mut server);
    small.pump(&mut server);
    big.pump(&mut server);
    big.draw(&mut server);
    let projection = |server: &Server, client| {
        let c: &Client = server.clients.get(client).unwrap();
        c.tsp.projection.as_ref().map(|p| p.generation)
    };
    let before = projection(&server, big.client);
    assert!(before.is_some());
    assert!(projection(&server, small.client).is_some());

    resize_terminal(&mut server, &mut small, 80, 19);
    big.pump(&mut server);
    let w = server.windows.get(window).unwrap();
    assert_eq!((w.sx, w.sy), (80, 24));
    assert_eq!(projection(&server, small.client), None);
    assert!(small.tree.is_none());
    assert_eq!(projection(&server, big.client), before);
}

/// The TSP host after a page reload that kept the pty: it reports the outer
/// surface gone. The terminal forgets its tree, as the reloaded page did.
fn host_lost_surface(server: &mut Server, terminal: &mut FakeTerminal) {
    let outer = terminal.tree.take().unwrap()["id"].clone();
    terminal.focus = None;
    terminal.frames.clear();
    terminal.send(
        server,
        &apc(b'e', &json!({"ev":"gone","sf":outer,"ids":[outer]})),
    );
    terminal.pump(server);
}

fn projection_generation(server: &Server, terminal: &FakeTerminal) -> Option<u64> {
    let c = server.clients.get(terminal.client).unwrap();
    c.tsp.projection.as_ref().map(|p| p.generation)
}

#[test]
fn host_that_lost_the_surface_gets_it_again_after_every_reload() {
    let (mut server, _, mut terminal) = stock_native();
    let mut generations = vec![projection_generation(&server, &terminal).unwrap()];
    for _ in 0..3 {
        host_lost_surface(&mut server, &mut terminal);
        assert!(
            terminal.draw(&mut server).is_some(),
            "no replay after a reload"
        );
        assert_eq!(terminal.text("transcript"), "retained transcript");
        let c = server.clients.get(terminal.client).unwrap();
        assert_eq!(c.tsp.format(), "v1");
        generations.push(projection_generation(&server, &terminal).unwrap());
    }
    generations.dedup();
    assert_eq!(generations.len(), 4, "{generations:?}");
}

#[test]
fn host_that_loses_the_reopened_surface_before_an_ack_falls_back() {
    let (mut server, _, mut terminal) = stock_native();
    host_lost_surface(&mut server, &mut terminal);
    assert!(projection_generation(&server, &terminal).is_some());
    host_lost_surface(&mut server, &mut terminal);
    let c = server.clients.get(terminal.client).unwrap();
    assert!(c.tsp.projection.is_none());
    assert_eq!(c.tsp.format(), "no");
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

fn open_page_over_transcript(server: &mut Server, pane: PaneId, terminal: &mut FakeTerminal) {
    let mut page = apc(
        b'o',
        &json!({"id":"page","mode":"screen","title":"omp","role":"omp.rewind"}),
    );
    page.extend(apc(
        b'f',
        &json!({"sf":"page","s":1,"ops":[["add","turn","page",null,{"id":"turn","k":"text","p":{"text":"turn 1"}}]]}),
    ));
    pane_runtime::pane_parse_buffer(server, pane, &page).unwrap();
    terminal.pump(server);
    terminal.draw(server);
}

#[test]
fn closing_a_page_over_the_transcript_shows_it_again_without_a_replay() {
    let (mut server, pane, mut terminal) = stock_native();
    let transcript = terminal.tree.as_ref().unwrap()["id"].clone();
    let opens = terminal.opens;
    open_page_over_transcript(&mut server, pane, &mut terminal);
    assert_eq!(terminal.text("turn"), "turn 1");
    assert_eq!(
        terminal.covered.as_ref().map(|(tree, _)| &tree["id"]),
        Some(&transcript),
        "the transcript stays open under the page"
    );
    server.panes.get_mut(pane).unwrap().output.clear();
    let mut back = apc(b'x', &json!({"id":"page","keep":false}));
    back.extend(apc(
        b'f',
        &json!({"sf":"program","s":2,"ops":[["set","transcript",{"text":"after the page"}]]}),
    ));
    pane_runtime::pane_parse_buffer(&mut server, pane, &back).unwrap();
    terminal.pump(&mut server);
    terminal.draw(&mut server);
    assert!(terminal.covered.is_none());
    assert_eq!(terminal.tree.as_ref().unwrap()["id"], transcript);
    assert_eq!(terminal.text("transcript"), "after the page");
    assert_eq!(terminal.opens, opens + 1, "only the page was opened");
    let output = std::mem::take(&mut server.panes.get_mut(pane).unwrap().output);
    assert!(
        Framing::default().feed(&output).iter().any(|packet| matches!(
            packet,
            Packet::Message(b'e', body) if body["ev"] == "ack" && body["sf"] == "program" && body["s"] == 2
        )),
        "the program's transcript frame is acked"
    );
}

#[test]
fn stopping_the_tty_under_a_page_closes_both_surfaces() {
    let (mut server, pane, mut terminal) = stock_native();
    open_page_over_transcript(&mut server, pane, &mut terminal);
    assert!(terminal.covered.is_some());
    server
        .clients
        .get_mut(terminal.client)
        .unwrap()
        .tty
        .as_mut()
        .unwrap()
        .stop(&mut server.tparm, &TtyOptions::default());
    terminal.pump(&mut server);
    assert!(terminal.tree.is_none() && terminal.covered.is_none());
    assert!(!terminal.alternate());
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
fn source_ack_follows_the_viewer_in_use_and_the_slow_viewer_catches_up() {
    let mut fixture = Fixture::new();
    let acks = |fixture: &Fixture| {
        fixture
            .replies
            .iter()
            .filter(|(verb, body)| *verb == b'e' && body["ev"] == "ack")
            .count()
    };
    let work = |fixture: &mut Fixture| {
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
    };

    let mut active = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut active);
    fixture.drive();
    let mut slow = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    slow.pump(&mut fixture.server);
    assert!(slow.tree.is_some(), "newcomer receives a snapshot");
    slow.draw(&mut fixture.server);
    fixture
        .server
        .clients
        .get_mut(active.client)
        .unwrap()
        .activity_time = (i64::MAX, 0);
    broker::recompute(&mut fixture.server);
    fixture.drive();
    let before = acks(&fixture);

    work(&mut fixture);
    broker::project_pending(&mut fixture.server, active.client);
    broker::project_pending(&mut fixture.server, slow.client);
    active.pump(&mut fixture.server);
    slow.pump(&mut fixture.server);
    active.draw(&mut fixture.server);
    fixture.drive();
    assert_eq!(
        acks(&fixture),
        before + 1,
        "the viewer in use pays the debt; the slow viewer does not hold it"
    );

    slow.draw(&mut fixture.server);
    fixture.drive();
    assert_eq!(acks(&fixture), before + 1, "no second ack for paid debt");

    let mut expected = before + 1;
    for round in 2..=5 {
        work(&mut fixture);
        broker::project_pending(&mut fixture.server, active.client);
        active.pump(&mut fixture.server);
        active.draw(&mut fixture.server);
        fixture.drive();
        expected += 1;
        assert_eq!(
            acks(&fixture),
            expected,
            "a stalled viewer never stalls the program"
        );
        assert_eq!(active.text("counter"), format!("counter:{round}"));
        assert_eq!(fixture.state()["counter"], round);
    }

    for _ in 0..8 {
        broker::project_pending(&mut fixture.server, slow.client);
        slow.pump(&mut fixture.server);
        slow.draw(&mut fixture.server);
        fixture.drive();
        if slow.frames.is_empty() && slow.text("counter") == active.text("counter") {
            break;
        }
    }
    assert!(slow.frames.is_empty(), "slow viewer caught up");
    assert_eq!(slow.text("counter"), active.text("counter"));
    assert_eq!(acks(&fixture), expected, "catch-up pays no debt");

    fixture
        .server
        .clients
        .get_mut(active.client)
        .unwrap()
        .activity_time = (1, 0);
    fixture
        .server
        .clients
        .get_mut(slow.client)
        .unwrap()
        .activity_time = (2, 0);
    broker::recompute(&mut fixture.server);
    work(&mut fixture);
    broker::project_pending(&mut fixture.server, active.client);
    broker::project_pending(&mut fixture.server, slow.client);
    active.pump(&mut fixture.server);
    slow.pump(&mut fixture.server);
    active.draw(&mut fixture.server);
    fixture.drive();
    assert_eq!(acks(&fixture), expected, "an idle viewer pays no debt");
    slow.draw(&mut fixture.server);
    fixture.drive();
    expected += 1;
    assert_eq!(
        acks(&fixture),
        expected,
        "the ack moved with the viewer in use"
    );

    active.detach(&mut fixture.server);
    slow.detach(&mut fixture.server);
    fixture.drive();
    assert_eq!(
        acks(&fixture),
        expected,
        "no source ack is invented for an empty viewer set"
    );
}

#[test]
fn smaller_idle_viewer_gets_the_note_and_the_viewer_in_use_keeps_native() {
    let mut fixture = Fixture::new();
    let mut ghost = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    fixture.settle(&mut ghost);
    let mut live = FakeTerminal::attach(&mut fixture.server, Some(fixture.session), true);
    live.pump(&mut fixture.server);
    live.draw(&mut fixture.server);
    fixture.drive();
    // A closed browser tab keeps its client, now smaller than the window.
    fixture
        .server
        .clients
        .get_mut(ghost.client)
        .unwrap()
        .tty
        .as_mut()
        .unwrap()
        .set_size(40, 12, 0, 0);
    broker::recompute(&mut fixture.server);
    ghost.pump(&mut fixture.server);
    live.pump(&mut fixture.server);
    let pane = fixture.pane;
    let renderer = |server: &Server| {
        server
            .panes
            .get(pane)
            .unwrap()
            .tsp
            .as_ref()
            .unwrap()
            .renderer
    };
    assert_eq!(renderer(&fixture.server), broker::Renderer::Native);
    assert!(
        fixture
            .server
            .clients
            .get(ghost.client)
            .unwrap()
            .tsp
            .projection
            .is_none()
    );
    assert_eq!(live.text("transcript"), "transcript:0");

    let acks = |fixture: &Fixture| {
        fixture
            .replies
            .iter()
            .filter(|(verb, body)| *verb == b'e' && body["ev"] == "ack")
            .count()
    };
    let before = acks(&fixture);
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
    live.pump(&mut fixture.server);
    live.draw(&mut fixture.server);
    fixture.drive();
    assert_eq!(live.text("transcript"), "transcript:1");
    assert_eq!(
        acks(&fixture),
        before + 1,
        "the viewer in use alone pays the draw debt"
    );

    // Typing in the small client makes it the one in use: everyone gets text.
    fixture
        .server
        .clients
        .get_mut(ghost.client)
        .unwrap()
        .activity_time = (i64::MAX, 0);
    broker::recompute(&mut fixture.server);
    fixture.settle(&mut ghost);
    live.pump(&mut fixture.server);
    assert_eq!(renderer(&fixture.server), broker::Renderer::Ansi);
    assert!(live.tree.is_none());
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
