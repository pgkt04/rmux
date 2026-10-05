// Ported from tmux tty-keys.c and key-string.c @ 8f25579c
//! Differential test: the same command script drives the pinned C
//! `tty_keys_next`/`key_string_lookup_*` (tests/keys_reference.c) and the
//! Rust decoder; every step line, tree dump and key name must match.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use rmux_tty::key_string::{key_name, parse_key_name, table_entries};
use rmux_tty::keys::tables::{CODE_KEYS, RAW_KEYS, XTERM_KEYS};
use rmux_tty::keys::{
    ColourTarget, DecodeStep, Discovery, KeyDecodeContext, TimerPhase, TtyInput, TtyKey,
    TtyKeyDecoder,
};
use rmux_tty::term::{TtyCodeCode, terminfo};
use rmux_tty::tty::TtyFlags;
use rmux_util::key::{KeyCode, KeyFlags, KeyModifiers, SpecialKey as K};

fn reference() -> Option<std::path::PathBuf> {
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/keys_reference.c");
    common::build_c(
        "keys",
        &[
            &driver,
            Path::new("utf8.c"),
            Path::new("utf8-combined.c"),
            Path::new("colour.c"),
            Path::new("xmalloc.c"),
            Path::new("compat/utf8proc.c"),
            Path::new("compat/vis.c"),
            Path::new("compat/strtonum.c"),
            Path::new("compat/reallocarray.c"),
            Path::new("compat/recallocarray.c"),
            Path::new("compat/explicit_bzero.c"),
        ],
        // b64_pton comes from libresolv on both platforms.
        &["-lresolv"],
        cfg!(target_os = "macos"),
    )
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::new();
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

#[derive(Clone, Debug)]
enum Cmd {
    Cap(TtyCodeCode, Vec<u8>),
    ClearCaps,
    User(u32, Vec<u8>),
    Build,
    Tree,
    Init,
    Flags(TtyFlags),
    Session(bool),
    Verase(u8),
    Size(u32, u32, u32, u32),
    Colours(i32, i32),
    Requests(bool),
    Escape(u32),
    Feed(Vec<u8>),
    Expire,
    Step,
    Name(Vec<u8>),
    Key(u64, bool),
}

#[derive(Default)]
struct Script {
    cmds: Vec<Cmd>,
}

impl Script {
    fn push(&mut self, c: Cmd) {
        self.cmds.push(c);
    }
    fn feed(&mut self, bytes: &[u8]) {
        self.push(Cmd::Feed(bytes.to_vec()));
    }
    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.push(Cmd::Step);
        }
    }
    fn pre(&mut self, pre: &[Cmd]) {
        self.push(Cmd::Init);
        for c in pre {
            self.push(c.clone());
        }
    }
    /// Whole sequence, then every split point with and without expiry.
    fn case(&mut self, pre: &[Cmd], bytes: &[u8], split: bool) {
        let n = bytes.len() + 1;
        self.pre(pre);
        self.feed(bytes);
        self.steps(n);
        if !split {
            return;
        }
        for k in 1..bytes.len() {
            for expire in [false, true] {
                self.pre(pre);
                self.feed(&bytes[..k]);
                self.steps(2);
                if expire {
                    self.push(Cmd::Expire);
                    self.steps(2);
                }
                self.feed(&bytes[k..]);
                self.steps(n);
            }
        }
    }
    fn whole(&mut self, bytes: &[u8]) {
        self.case(&[], bytes, false);
    }
    fn split(&mut self, bytes: &[u8]) {
        self.case(&[], bytes, true);
    }

    fn text(&self) -> String {
        let mut s = String::new();
        for c in &self.cmds {
            match c {
                Cmd::Cap(code, v) => writeln!(s, "cap {} {}", *code as i32, hex(v)),
                Cmd::ClearCaps => writeln!(s, "clearcaps"),
                Cmd::User(i, v) => writeln!(s, "user {i} {}", hex(v)),
                Cmd::Build => writeln!(s, "build"),
                Cmd::Tree => writeln!(s, "tree"),
                Cmd::Init => writeln!(s, "init"),
                Cmd::Flags(f) => writeln!(s, "flags {:x}", f.bits()),
                Cmd::Session(on) => writeln!(s, "session {}", u8::from(*on)),
                Cmd::Verase(v) => writeln!(s, "verase {v}"),
                Cmd::Size(a, b, c, d) => writeln!(s, "size {a} {b} {c} {d}"),
                Cmd::Colours(fg, bg) => writeln!(s, "colours {fg} {bg}"),
                Cmd::Requests(on) => writeln!(s, "requests {}", u8::from(*on)),
                Cmd::Escape(ms) => writeln!(s, "escape {ms}"),
                Cmd::Feed(v) => writeln!(s, "feed {}", hex(v)),
                Cmd::Expire => writeln!(s, "expire"),
                Cmd::Step => writeln!(s, "step"),
                Cmd::Name(v) => writeln!(s, "name {}", hex(v)),
                Cmd::Key(k, flags) => writeln!(s, "key {k:x} {}", u8::from(*flags)),
            }
            .unwrap();
        }
        s
    }
}

/// The Rust side of the script: a recording host applying each step the way
/// the server adapter must.
struct Sim {
    caps: BTreeMap<i32, Vec<u8>>,
    users: BTreeMap<u32, Vec<u8>>,
    built: (BTreeMap<i32, Vec<u8>>, BTreeMap<u32, Vec<u8>>),
    dec: TtyKeyDecoder,
    buf: Vec<u8>,
    flags: TtyFlags,
    session: bool,
    verase: Option<u8>,
    size: (u32, u32, u32, u32),
    fg: i32,
    bg: i32,
    requests: bool,
    escape: u32,
    term_type: Vec<u8>,
    out: Vec<String>,
}

impl Sim {
    fn new() -> Sim {
        let mut sim = Sim {
            caps: BTreeMap::new(),
            users: BTreeMap::new(),
            built: Default::default(),
            dec: TtyKeyDecoder::new(),
            buf: Vec::new(),
            flags: TtyFlags(0),
            session: true,
            verase: Some(0x7f),
            size: (80, 24, 0, 0),
            fg: -1,
            bg: -1,
            requests: false,
            escape: 10,
            term_type: Vec::new(),
            out: Vec::new(),
        };
        sim.init();
        sim
    }

    fn init(&mut self) {
        self.buf.clear();
        self.flags = TtyFlags(0);
        self.size = (80, 24, 0, 0);
        self.fg = -1;
        self.bg = -1;
        self.verase = Some(0x7f);
        self.escape = 10;
        self.requests = false;
        self.session = true;
        self.term_type.clear();
        // C keeps the tree and resets the rest; a fresh decoder with the
        // same tree resets paste, mouse and timer state the same way.
        self.dec = TtyKeyDecoder::new();
        let (caps, users) = self.built.clone();
        self.dec.rebuild_with(
            |code| caps.get(&(code as i32)).map_or(&[][..], |v| v),
            users.iter().map(|(i, v)| (*i, &v[..])),
        );
    }

    fn build(&mut self) {
        self.built = (self.caps.clone(), self.users.clone());
        let (caps, users) = self.built.clone();
        self.dec.rebuild_with(
            |code| caps.get(&(code as i32)).map_or(&[][..], |v| v),
            users.iter().map(|(i, v)| (*i, &v[..])),
        );
    }

    fn dump(nodes: &[TtyKey], index: Option<u32>, s: &mut String) {
        let Some(index) = index else {
            s.push('-');
            return;
        };
        let n = &nodes[index as usize];
        write!(s, "({:02x} {:x} ", n.ch, n.key.0).unwrap();
        Self::dump(nodes, n.left, s);
        s.push(' ');
        Self::dump(nodes, n.right, s);
        s.push(' ');
        Self::dump(nodes, n.next, s);
        s.push(')');
    }

    fn run(&mut self, cmds: &[Cmd]) {
        for c in cmds {
            match c {
                Cmd::Cap(code, v) => {
                    self.caps.insert(*code as i32, v.clone());
                }
                Cmd::ClearCaps => {
                    self.caps.clear();
                    self.users.clear();
                }
                Cmd::User(i, v) => {
                    if *i <= KeyCode::NUSER {
                        self.users.insert(*i, v.clone());
                    }
                }
                Cmd::Build => self.build(),
                Cmd::Tree => {
                    let mut s = String::new();
                    let nodes = self.dec.nodes();
                    Self::dump(nodes, (!nodes.is_empty()).then_some(0), &mut s);
                    self.out.push(s);
                }
                Cmd::Init => self.init(),
                Cmd::Flags(f) => self.flags = *f,
                Cmd::Session(on) => self.session = *on,
                Cmd::Verase(v) => self.verase = (*v != 0xff).then_some(*v),
                Cmd::Size(a, b, c, d) => self.size = (*a, *b, *c, *d),
                Cmd::Colours(fg, bg) => {
                    self.fg = *fg;
                    self.bg = *bg;
                }
                Cmd::Requests(on) => self.requests = *on,
                Cmd::Escape(ms) => self.escape = *ms,
                Cmd::Feed(v) => self.buf.extend_from_slice(v),
                Cmd::Expire => self.dec.timer_fired(),
                Cmd::Step => self.step(),
                Cmd::Name(v) => self.out.push(format!("{:x}", parse_key_name(v).0)),
                Cmd::Key(k, flags) => self.out.push(hex(&key_name(KeyCode(*k), *flags))),
            }
        }
    }

    fn step(&mut self) {
        let ctx = KeyDecodeContext {
            flags: self.flags,
            has_session: self.session,
            escape_time_ms: self.escape,
            verase: self.verase,
            sx: self.size.0,
            sy: self.size.1,
            xpixel: self.size.2,
            ypixel: self.size.3,
            has_input_requests: self.requests,
        };
        let buf = std::mem::take(&mut self.buf);
        let step = self.dec.next(&buf, &ctx);
        let mut events = String::new();
        let mut delay: i64 = -1;
        let (ret, consumed) = match step {
            DecodeStep::Empty => (0, 0),
            DecodeStep::Partial {
                timer,
                theme_changed,
            } => {
                if theme_changed {
                    events.push_str(" theme-changed");
                }
                if let Some(req) = timer {
                    delay = req.after.unwrap().as_millis() as i64;
                }
                (0, 0)
            }
            DecodeStep::Discard { consumed, .. } => (1, consumed),
            DecodeStep::Complete {
                consumed,
                input,
                theme_changed,
                ..
            } => {
                if theme_changed {
                    events.push_str(" theme-changed");
                }
                self.apply(input, &mut events);
                (1, consumed)
            }
        };
        self.buf = buf;
        self.buf.drain(..consumed);
        let timer = match self.dec.timer_phase() {
            TimerPhase::Idle => "idle",
            TimerPhase::Waiting => "waiting",
            TimerPhase::Fired => "fired",
        };
        let last = self.dec.mouse_last();
        self.out.push(format!(
            "ret={ret} consumed={consumed} flags={:x} timer={timer} delay={delay} paste={} last={},{},{} fg={} bg={} size={},{},{},{} term={} |{events}",
            self.flags.bits(),
            u8::from(self.dec.bracket_paste()),
            last.x,
            last.y,
            last.b,
            self.fg,
            self.bg,
            self.size.0,
            self.size.1,
            self.size.2,
            self.size.3,
            hex(&self.term_type),
        ));
    }

    fn apply(&mut self, input: TtyInput<'_>, ev: &mut String) {
        match input {
            TtyInput::Key(k) => {
                if k.key.0 == K::FOCUS_OUT {
                    ev.push_str(" winfocus fire:client-focus-out");
                } else if k.key.0 == K::FOCUS_IN {
                    ev.push_str(" fire:client-focus-in winfocus");
                }
                write!(ev, " key:{:x}:{}", k.key.0, hex(k.raw)).unwrap();
                if let Some(m) = k.mouse {
                    write!(
                        ev,
                        " mouse:{},{},{},{},{},{},{},{}",
                        m.x, m.y, m.b, m.lx, m.ly, m.lb, m.sgr_type, m.sgr_b
                    )
                    .unwrap();
                }
            }
            TtyInput::Clipboard(r) => {
                if let Some(data) = &r.data {
                    write!(ev, " reply-clip:{}:{}", r.clip, hex(data.as_bytes())).unwrap();
                    if r.query {
                        write!(ev, " paste:{}", hex(data.as_bytes())).unwrap();
                        self.flags.remove(TtyFlags::OSC52QUERY);
                    }
                }
            }
            TtyInput::Palette(r) => {
                if let Some(p) = r.reply {
                    write!(ev, " reply-pal:{}:{}", p.idx, p.c.0).unwrap();
                }
            }
            TtyInput::Colour(r) => {
                let bg = self.bg;
                if let Some(c) = r.colour {
                    match r.target {
                        ColourTarget::Foreground => self.fg = c.0,
                        ColourTarget::Background => self.bg = c.0,
                    }
                    self.flags.remove(r.target.wait_flag());
                }
                if self.bg != bg {
                    ev.push_str(" theme-colours");
                }
                ev.push_str(" theme-changed");
            }
            TtyInput::Discovery(d) => {
                match &d {
                    Discovery::PrimaryDa { features } => {
                        for name in features.names() {
                            write!(ev, " feat:{name}").unwrap();
                        }
                    }
                    Discovery::SecondaryDa { defaults }
                    | Discovery::ExtendedDa { defaults, .. } => {
                        if let Some(name) = defaults {
                            write!(ev, " deffeat:{name}").unwrap();
                        }
                    }
                    Discovery::Sync { sync } => {
                        if *sync {
                            ev.push_str(" feat:sync");
                        }
                    }
                }
                if let Discovery::ExtendedDa {
                    term_type: Some(t), ..
                } = &d
                {
                    self.term_type = t.as_bytes().to_vec();
                }
                if d.updates_features() {
                    ev.push_str(" update");
                }
                self.flags.insert(d.have_flag());
            }
            TtyInput::Size(r) => {
                self.size = (r.sx, r.sy, r.xpixel, r.ypixel);
                write!(ev, " setsize:{},{},{},{}", r.sx, r.sy, r.xpixel, r.ypixel).unwrap();
                if r.invalidate {
                    ev.push_str(" invalidate");
                }
                if r.clear_query {
                    self.flags.remove(TtyFlags::WINSIZEQUERY);
                }
            }
        }
    }
}

/// Run one script through both sides and compare.
fn compare(name: &str, script: &Script) {
    let Some(bin) = reference() else {
        eprintln!("keys C reference skipped");
        return;
    };
    let expected = common::run(&bin, &[], script.text().as_bytes());
    let expected = String::from_utf8_lossy(&expected);
    let expected: Vec<&str> = expected.lines().collect();
    let mut sim = Sim::new();
    sim.run(&script.cmds);
    // Map output lines back to the commands that produced them.
    let mut producing = Vec::new();
    for (i, c) in script.cmds.iter().enumerate() {
        if matches!(c, Cmd::Step | Cmd::Tree | Cmd::Name(_) | Cmd::Key(..)) {
            producing.push(i);
        }
    }
    assert_eq!(sim.out.len(), producing.len(), "{name}: output count");
    assert_eq!(expected.len(), sim.out.len(), "{name}: C output count");
    for (n, (c, r)) in expected.iter().zip(sim.out.iter()).enumerate() {
        if c != r {
            let at = producing[n];
            let from = at.saturating_sub(12);
            let mut context = String::new();
            for (i, cmd) in script.cmds[from..=at].iter().enumerate() {
                writeln!(context, "  {:>5}: {cmd:?}", from + i).unwrap();
            }
            panic!("{name}: line {n} differs\n  C:    {c}\n  Rust: {r}\ncommands:\n{context}");
        }
    }
}

const ESC: u8 = 0x1b;

fn xterm_sequences() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for &(template, _) in XTERM_KEYS {
        for j in 2u8..=9 {
            let s: Vec<u8> = template
                .iter()
                .map(|&c| if c == b'_' { b'0' + j } else { c })
                .collect();
            v.push(s);
        }
    }
    v
}

/// The terminfo capability name of a key code (`tty_term_codes[]`).
fn cap_name(code: TtyCodeCode) -> String {
    let dbg = format!("{code:?}");
    let lower = dbg.to_ascii_lowercase();
    let body = &lower[1..];
    if let Some(digit) = body.chars().last().filter(char::is_ascii_digit) {
        let stem = &body[..body.len() - 1];
        if matches!(
            stem,
            "dc" | "dn" | "end" | "hom" | "ic" | "lft" | "nxt" | "prv" | "rit" | "up"
        ) {
            return format!("k{}{digit}", stem.to_ascii_uppercase());
        }
    }
    lower
}

fn terminfo_caps(term: &str) -> Option<Vec<(TtyCodeCode, Vec<u8>)>> {
    let list = terminfo::read_list(term.as_bytes()).ok()?;
    let mut caps = Vec::new();
    for &(code, _) in CODE_KEYS {
        let name = cap_name(code);
        for cap in &list {
            let cap = cap.as_bytes();
            if let Some(value) = cap.strip_prefix(format!("{name}=").as_bytes()) {
                caps.push((code, value.to_vec()));
            }
        }
    }
    Some(caps)
}

#[test]
fn default_tree_and_sequences() {
    let mut s = Script::default();
    s.push(Cmd::Build);
    s.push(Cmd::Tree);
    for &(seq, _) in RAW_KEYS {
        s.split(seq);
        // An extra Escape in front: ImpliedMeta keeps it separate.
        let mut with_esc = vec![ESC];
        with_esc.extend_from_slice(seq);
        s.whole(&with_esc);
    }
    for seq in xterm_sequences() {
        s.split(&seq);
        let mut with_esc = vec![ESC];
        with_esc.extend_from_slice(&seq);
        s.whole(&with_esc);
    }
    // Several sequences in one buffer and trailing unrelated bytes.
    s.whole(b"\x1b[A\x1b[Bab\x1bOP\x1b[1;5Cz");
    s.whole(b"\x1b[A\x1b");
    s.whole(b"\x1bOAx\x1b\x1b[Dq");
    compare("default tree", &s);
}

#[test]
fn synthetic_and_real_terminfo() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    // Distinct synthetic strings, plus collisions with raw and xterm entries.
    for (i, &(code, _)) in CODE_KEYS.iter().enumerate() {
        let v = format!("\x1b[{i};9z").into_bytes();
        s.push(Cmd::Cap(code, v));
    }
    s.push(Cmd::Cap(TtyCodeCode::Kf5, b"\x1b[A".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kf6, b"\x1b[1;2P".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kf7, b"\x1b[1;2".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kf8, b"\x1b[1;2Pq".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kcbt, b"\x1b[Z".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kind, b"\x1b[1;2B".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kri, b"\x1b[1;2A".to_vec()));
    s.push(Cmd::Build);
    s.push(Cmd::Tree);
    for i in 0..CODE_KEYS.len() {
        let v = format!("\x1b[{i};9z").into_bytes();
        s.whole(&v);
    }
    for seq in [
        b"\x1b[A".as_slice(),
        b"\x1b[1;2P",
        b"\x1b[1;2",
        b"\x1b[1;2Pq",
        b"\x1b[1;2Pz",
        b"\x1b[Z",
        b"\x1b[1;2B",
        b"\x1b[1;2A",
        b"\x1b[1;2Px",
    ] {
        s.split(seq);
    }
    for term in ["xterm-256color", "screen-256color", "tmux-256color"] {
        let Some(caps) = terminfo_caps(term) else {
            eprintln!("{term}: terminfo not found, skipped");
            continue;
        };
        s.push(Cmd::ClearCaps);
        for (code, v) in &caps {
            s.push(Cmd::Cap(*code, v.clone()));
        }
        s.push(Cmd::Build);
        s.push(Cmd::Tree);
        for (_, v) in &caps {
            s.split(v);
            let mut with_esc = vec![ESC];
            with_esc.extend_from_slice(v);
            s.whole(&with_esc);
        }
    }
    compare("terminfo", &s);
}

#[test]
fn user_keys_and_overlap() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::User(0, b"\x1b[A".to_vec()));
    s.push(Cmd::User(1, b"ab".to_vec()));
    s.push(Cmd::User(2, b"abc".to_vec()));
    s.push(Cmd::User(3, b"xyz".to_vec()));
    s.push(Cmd::User(4, b"xy".to_vec()));
    s.push(Cmd::User(5, b"ab".to_vec()));
    s.push(Cmd::User(6, b"\x80\x81".to_vec()));
    s.push(Cmd::User(7, b"\x80".to_vec()));
    s.push(Cmd::User(8, b"\xff".to_vec()));
    s.push(Cmd::User(9, b"\x01".to_vec()));
    s.push(Cmd::User(10, b"\x1b".to_vec()));
    s.push(Cmd::User(11, b"\x1bOA".to_vec()));
    s.push(Cmd::User(12, b"\x7f".to_vec()));
    s.push(Cmd::User(13, b"z\x00ignored".to_vec()));
    s.push(Cmd::User(999, b"q".to_vec()));
    s.push(Cmd::User(1000, b"Q".to_vec()));
    s.push(Cmd::User(1001, b"W".to_vec()));
    s.push(Cmd::Build);
    s.push(Cmd::Tree);
    for seq in [
        b"\x1b[A".as_slice(),
        b"ab",
        b"abc",
        b"abd",
        b"a",
        b"xy",
        b"xyz",
        b"xyq",
        b"x",
        b"\x80\x81",
        b"\x80\x82",
        b"\x80",
        b"\xff",
        b"\x01",
        b"\x1b",
        b"\x1ba",
        b"\x1bOA",
        b"\x1b\x1bOA",
        b"\x7f",
        b"\x00",
        b"q",
        b"Q",
        b"z",
        b"z\x00ignored",
        b"W",
        b"\x1b\x01",
        b"\x1bq",
    ] {
        s.split(seq);
    }
    // The same user bytes override ordinary fallback with VERASE disabled.
    s.case(&[Cmd::Verase(255)], b"\x7f", false);
    s.case(&[Cmd::Verase(255)], b"\x1b\x7f", false);
    compare("user keys", &s);
}

#[test]
fn escape_utf8_and_fallback() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::Build);
    for b in 0..=255u8 {
        s.whole(&[b]);
        s.whole(&[ESC, b]);
    }
    s.split(b"\x1b");
    s.split(b"\x1b\x1b");
    s.split(b"\x1b\x1b\x1b");
    s.split("é".as_bytes());
    s.split("\x1bé".as_bytes());
    s.split("€".as_bytes());
    s.split("😀".as_bytes());
    s.split("\x1b😀".as_bytes());
    s.split("a😀b".as_bytes());
    // Invalid forms: overlong, surrogate, bad continuation, truncated, F5+.
    for seq in [
        b"\xc0\x80".as_slice(),
        b"\xc1\xbf",
        b"\xe0\x80\x80",
        b"\xed\xa0\x80",
        b"\xf4\x90\x80\x80",
        b"\xf5\x80\x80\x80",
        b"\xf8\x88\x80\x80\x80",
        b"\xc3\x28",
        b"\xe2\x82",
        b"\xe2\x82\x28",
        b"\xf0\x9f\x98",
        b"\xf0\x9f\x98\x28",
        b"\x80",
        b"\xbf",
        b"\xfe",
        b"\xff",
        b"\xc3",
        b"\x1b\xc3",
        b"\x1b\xc3\x28",
    ] {
        s.split(seq);
    }
    // VERASE variants.
    for verase in [255u8, 0, 8, 9, 13, 27, 127, b'a'] {
        for seq in [
            [0u8].as_slice(),
            &[8],
            &[9],
            &[13],
            &[27],
            &[127],
            b"a",
            &[ESC, 0],
            &[ESC, 8],
            &[ESC, 127],
            b"\x1ba",
        ] {
            s.case(&[Cmd::Verase(verase)], seq, false);
        }
    }
    compare("escape and fallback", &s);
}

#[test]
fn extended_keys() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::Build);
    let keys: [u32; 20] = [
        0, 9, 13, 27, 32, 59, 65, 97, 126, 127, 128, 0xe9, 0x3b1, 0x20ac, 0x1f600, 0xd800,
        0x110000, 0x7fffffff, 0x80000000, 0xffffffff,
    ];
    let mods: [u32; 24] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 32, 33, 64, 255, 256,
        0xffffffff,
    ];
    for &k in &keys {
        for &m in &mods {
            s.whole(format!("\x1b[27;{m};{k}~").as_bytes());
            s.whole(format!("\x1b[{k};{m}u").as_bytes());
        }
    }
    s.split(b"\x1b[27;5;13~");
    s.split(b"\x1b[97;5u");
    s.split(b"\x1b[1;5Z");
    for seq in [
        b"\x1b[27;5;13;7~".as_slice(),
        b"\x1b[97;5;9u",
        b"\x1b[27;;5~",
        b"\x1b[27;5;~",
        b"\x1b[27;5~",
        b"\x1b[u",
        b"\x1b[;u",
        b"\x1b[5u",
        b"\x1b[5;u",
        b"\x1b[;5u",
        b"\x1b[027;5;13~",
        b"\x1b[27;5;13u",
        b"\x1b[99999999999999999999;1u",
        b"\x1b[65;99999999999999999999u",
        b"\x1b[123;5u",
        b"\x1b[32;2u",
        b"\x1b[9;5u",
        b"\x1b[9;2u",
        b"\x1b[9;4u",
        b"\x1b[9;6u",
        b"\x1b[9;8u",
        b"\x1b[27;2;9~",
        b"\x1b[27;6;9~",
        b"\x1b[32;2u",
        b"\x1b[65;2u",
        b"\x1b[65;6u",
        b"\x1b[233;2u",
        b"\x1b[1;5Zx",
    ] {
        s.whole(seq);
    }
    // Terminator at the 64-byte scan bound.
    for digits in [60usize, 61, 62, 63] {
        let mut v = b"\x1b[".to_vec();
        v.extend(std::iter::repeat_n(b'1', digits));
        s.whole(&v);
        let mut t = v.clone();
        t.push(b'u');
        s.whole(&t);
        let mut t = v.clone();
        t.push(b'~');
        s.whole(&t);
    }
    // VERASE matches before conversion.
    for verase in [8u8, 127, 255] {
        s.case(&[Cmd::Verase(verase)], b"\x1b[127;1u", false);
        s.case(&[Cmd::Verase(verase)], b"\x1b[8;3u", false);
        s.case(&[Cmd::Verase(verase)], b"\x1b[27;5;127~", false);
    }
    compare("extended keys", &s);
}

#[test]
fn mouse_reports() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::Build);
    for b in [
        0x1fu8, 0x20, 0x21, 0x22, 0x23, 0x24, 0x40, 0x60, 0x61, 0x62, 0x63, 0xa0, 0xa3, 0xff,
    ] {
        for x in [0x20u8, 0x21, 0x22, 0xff] {
            for y in [0x20u8, 0x21, 0x50, 0xff] {
                s.whole(&[ESC, b'[', b'M', b, x, y]);
            }
        }
    }
    s.split(b"\x1b[M !!");
    s.split(b"\x1b[M\x20\x20\x20");
    let buttons = [
        "",
        "0",
        "1",
        "2",
        "3",
        "4",
        "8",
        "16",
        "32",
        "35",
        "64",
        "65",
        "66",
        "67",
        "96",
        "97",
        "128",
        "131",
        "160",
        "4294967295",
        "4294967296",
        "8589934592",
    ];
    let coords = [
        "",
        "0",
        "1",
        "2",
        "80",
        "255",
        "4294967295",
        "4294967296",
        "4294967297",
    ];
    for b in buttons {
        for x in coords {
            for y in coords {
                for t in ["M", "m"] {
                    s.whole(format!("\x1b[<{b};{x};{y}{t}").as_bytes());
                }
            }
        }
    }
    for seq in [
        b"\x1b[<0;1;1M".as_slice(),
        b"\x1b[<64;1;1m",
        b"\x1b[<a;1;1M",
        b"\x1b[<1;a;1M",
        b"\x1b[<1;1;aM",
        b"\x1b[<1;1;1x",
        b"\x1b[<1;1M",
        b"\x1b[<1;1;1;1M",
        b"\x1b[<;;M",
        b"\x1b[<M",
        b"\x1b[<1:1:1M",
        b"\x1b[<-1;1;1M",
        b"\x1b[<0;1;1M\x1b[<0;2;2M\x1b[<3;2;2m",
        b"\x1b[<32;5;5M\x1b[<0;9;9m",
    ] {
        s.split(seq);
    }
    // Discard with an active key timer: the timer is kept.
    s.pre(&[]);
    s.feed(b"\x1b");
    s.steps(1);
    s.feed(b"[<64;1;1m");
    s.steps(1);
    s.feed(b"a");
    s.steps(2);
    s.pre(&[]);
    s.feed(b"\x1b[<0;3;3M");
    s.steps(1);
    s.feed(b"\x1b[M\x1f!!");
    s.steps(1);
    s.feed(b"\x1b[<0;0;0M");
    s.steps(1);
    s.feed(b"\x1b[<0;4;4M");
    s.steps(1);
    compare("mouse", &s);
}

fn base64(bytes: &[u8]) -> Vec<u8> {
    rmux_util::base64::ntop(bytes).into_vec()
}

#[test]
fn clipboard_and_palette_replies() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::Build);
    let query = [Cmd::Flags(TtyFlags::OSC52QUERY)];
    for term in [b"\x07".as_slice(), b"\x1b\\"] {
        for selector in [b"".as_slice(), b"c", b"cp", b"s0"] {
            for payload in [
                b"".as_slice(),
                b"aGVsbG8=",
                b"aGVsbG8",
                b"!!!!",
                b"aGVs bG8=\n",
                b"YQ==",
                b"\x00abc",
            ] {
                let mut v = b"\x1b]52;".to_vec();
                v.extend_from_slice(selector);
                v.push(b';');
                v.extend_from_slice(payload);
                v.extend_from_slice(term);
                s.case(&[], &v, false);
                s.case(&query, &v, false);
            }
        }
        // Missing separator and binary payloads.
        let mut v = b"\x1b]52;abc".to_vec();
        v.extend_from_slice(term);
        s.case(&query, &v, false);
        let mut v = b"\x1b]52;".to_vec();
        v.extend_from_slice(term);
        s.case(&query, &v, false);
        let binary: Vec<u8> = (0..=255u8).collect();
        let mut v = b"\x1b]52;c;".to_vec();
        v.extend_from_slice(&base64(&binary));
        v.extend_from_slice(term);
        s.case(&query, &v, false);
        let long: Vec<u8> = (0..300u32).map(|i| (i % 26) as u8 + b'a').collect();
        let mut v = b"\x1b]52;c;".to_vec();
        v.extend_from_slice(&base64(&long));
        v.extend_from_slice(term);
        s.case(&query, &v, false);
    }
    s.case(&query, b"\x1b]52;c;aGVsbG8=\x07", true);
    s.case(&[], b"\x1b]52;c;aG\x1b\\", true);
    s.whole(b"\x1b]52;c;aGVsbG8=\x1b");
    s.whole(b"\x1b]52;c;aGVsbG8=\x1bx");
    // Palette.
    for idx in [
        "5",
        "-1",
        "0",
        "255",
        "256",
        " 5",
        "",
        "5x",
        "+7",
        "4294967301",
        "99999999999999999999",
    ] {
        for colour in ["red", "#ff8000", "rgb:ff/80/00", "nope", "", "colour12"] {
            for term in ["\x07", "\x1b\\"] {
                s.whole(format!("\x1b]4;{idx};{colour}{term}").as_bytes());
            }
        }
    }
    s.split(b"\x1b]4;5;red\x07");
    s.whole(b"\x1b]4;5\x07");
    s.whole(b"\x1b]4;\x07");
    s.whole(b"\x1b]4;\x1b\\");
    s.whole(b"\x1b]4;5;red\x1b");
    // Scratch-buffer boundary: 128-byte scan, final slot unused.
    for n in [120usize, 123, 124, 125, 126, 127, 128, 130] {
        let mut v = b"\x1b]4;5;".to_vec();
        v.extend(std::iter::repeat_n(b'x', n - 2));
        for term in [b"\x07".as_slice(), b"\x1b\\"] {
            let mut t = v.clone();
            t.extend_from_slice(term);
            s.whole(&t);
            t.extend_from_slice(b"abc");
            s.whole(&t);
        }
    }
    compare("clipboard and palette", &s);
}

#[test]
fn discovery_and_colour_replies() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::Build);
    let have = [Cmd::Flags(
        TtyFlags::HAVEDA | TtyFlags::HAVEDA2 | TtyFlags::HAVEXDA | TtyFlags::HAVESYNC,
    )];
    for payload in [
        "62;4;21;28;52",
        "61;4",
        "64;1;2;3",
        "65;52",
        "60;4",
        "66;4",
        "62;4x;21",
        "62;;4",
        "62;4;;52",
        "317;4",
        "62;260",
        "",
        ";",
        "62",
        "62;",
        "62; 4",
        "62;+4",
        "62;-4",
        "62;4;4;4",
        "1;2;3;4;5;6;7;8;9;10;11;12;13;14;15;16;17;18;19;20;21;22;23;24;25;26;27;28;29;30;31;32;33;34",
        "62;4;21;28;52;1;2;3;4;5;6;7;8;9;10;11;12;13;14;15;16;17;18;19;20;21;22;23;24;25;26;27;28;29;30;31;32;33;34;35",
        "62;4\u{0};52",
        "62;A;52",
    ] {
        for term in ["c", "d", "x"] {
            let v = format!("\x1b[?{payload}{term}").into_bytes();
            s.whole(&v);
            s.case(&have, &v, false);
            let v = format!("\x1b[>{payload}{term}").into_bytes();
            s.whole(&v);
            s.case(&have, &v, false);
        }
    }
    for payload in [
        b"62;\x80;52".as_slice(),
        b"62;\xa04",
        b"62;\xa0;4",
        b"\xa061;4",
    ] {
        for prefix in [b"\x1b[?".as_slice(), b"\x1b[>"] {
            let mut v = prefix.to_vec();
            v.extend_from_slice(payload);
            v.push(b'c');
            s.whole(&v);
        }
    }
    for payload in [
        "77;1;2", "84;1", "85;0", "1;2", "333;1", "341;0", "589;1", "77x", "0;77",
    ] {
        s.whole(format!("\x1b[>{payload}c").as_bytes());
    }
    s.split(b"\x1b[?62;4;52c");
    s.split(b"\x1b[>84;1c");
    // 128-byte scratch bound.
    for n in [125usize, 126, 127, 128, 129] {
        let mut v = b"\x1b[?".to_vec();
        v.extend(std::iter::repeat_n(b'1', n));
        v.push(b'c');
        s.whole(&v);
        let mut v = b"\x1b[>".to_vec();
        v.extend(std::iter::repeat_n(b'1', n));
        v.push(b'c');
        s.whole(&v);
    }
    // Sync.
    for status in ["0", "1", "2", "3", "4", "5", "9", "", "12"] {
        for tail in ["$y", "$x", "y", "$"] {
            let v = format!("\x1b[?2026;{status}{tail}").into_bytes();
            s.whole(&v);
            s.case(&have, &v, false);
        }
    }
    s.split(b"\x1b[?2026;1$y");
    s.whole(b"\x1b[?2026;1$yz");
    // XDA.
    for text in [
        "iTerm2 3.4.0",
        "tmux 3.3a",
        "XTerm(370)",
        "mintty 3.6.1",
        "foot(1.13.1)",
        "WezTerm 20230712",
        "ghostty 1.0.0",
        "Rio 0.1",
        "unknown",
        "",
        "iTerm2",
        "iterm2 3",
        "XTerm 370",
        "tmux",
        "ab\x00cd",
        "WezTerm ",
        "Rio \x00tmux ",
    ] {
        let mut v = b"\x1bP>|".to_vec();
        v.extend_from_slice(text.as_bytes());
        for term in [b"\x1b\\".as_slice(), b"\x07", b"\x07\x1b\\"] {
            let mut t = v.clone();
            t.extend_from_slice(term);
            s.whole(&t);
            s.case(&have, &t, false);
        }
    }
    s.split(b"\x1bP>|tmux 3.3a\x1b\\");
    for n in [122usize, 123, 124, 125, 126, 127, 128] {
        let mut v = b"\x1bP>|".to_vec();
        v.extend(std::iter::repeat_n(b'x', n));
        v.extend_from_slice(b"\x1b\\");
        s.whole(&v);
        let mut w = v.clone();
        w.extend_from_slice(b"xyz");
        s.whole(&w);
    }
    s.whole(b"\x1bP>|abc\x1b");
    s.whole(b"\x1bP>|abc\x1bx\x1b\\");
    // Colours.
    let wait = [
        Cmd::Flags(TtyFlags::WAITFG | TtyFlags::WAITBG),
        Cmd::Colours(3, 4),
    ];
    for which in ["10", "11", "12", "1"] {
        for colour in [
            "rgb:ff/80/00",
            "#123456",
            "red",
            "rgb:ffff/8000/0000",
            "invalid",
            "",
            "colour5",
            "default",
            "\x1b",
        ] {
            for term in ["\x07", "\x1b\\", "\x1b\x07"] {
                let v = format!("\x1b]{which};{colour}{term}").into_bytes();
                s.whole(&v);
                s.case(&wait, &v, false);
            }
        }
    }
    s.case(&wait, b"\x1b]10;red\x07", true);
    s.case(&wait, b"\x1b]11;blue\x1b\\", true);
    s.whole(b"\x1b]10;red\x07\x1b]11;blue\x07a");
    for n in [124usize, 125, 126, 127, 128] {
        let mut v = b"\x1b]10;".to_vec();
        v.extend(std::iter::repeat_n(b'x', n));
        for term in [b"\x07".as_slice(), b"\x1b\\"] {
            let mut t = v.clone();
            t.extend_from_slice(term);
            s.whole(&t);
        }
    }
    // Window size.
    let query = [Cmd::Flags(TtyFlags::WINSIZEQUERY), Cmd::Size(80, 24, 7, 9)];
    for payload in [
        "8;24;80",
        "8;50;132",
        "4;480;640",
        "4;0;640",
        "4;480;0",
        "8;24",
        "4;480",
        "8;24;80;1",
        "4;480;640;2",
        "8;;80",
        "9;1;2",
        "8;99999999999999999999;5",
        "48;1;2",
        "",
        ";",
        "8;24;80x",
    ] {
        for term in ["t", "x"] {
            let v = format!("\x1b[{payload}{term}").into_bytes();
            s.whole(&v);
            s.case(&query, &v, false);
            s.case(
                &[Cmd::Flags(TtyFlags::WINSIZEQUERY), Cmd::Size(0, 0, 0, 0)],
                &v,
                false,
            );
        }
    }
    s.case(&query, b"\x1b[4;480;640t", true);
    s.case(&query, b"\x1b[8;24;80t\x1b[4;480;640t", false);
    for digits in [60usize, 61, 62, 63] {
        let mut v = b"\x1b[".to_vec();
        v.extend(std::iter::repeat_n(b'1', digits));
        s.case(&query, &v, false);
        let mut t = v.clone();
        t.push(b't');
        s.case(&query, &t, false);
    }
    compare("discovery and colours", &s);
}

#[test]
fn timers_and_session() {
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::Build);
    let all = TtyFlags::ALL_REQUEST_FLAGS;
    for ms in [0u32, 1, 10, 499, 500, 501, 1000, 65535] {
        s.case(&[Cmd::Escape(ms), Cmd::Flags(all)], b"\x1b", false);
        s.case(&[Cmd::Escape(ms)], b"\x1b", false);
        s.case(
            &[Cmd::Escape(ms), Cmd::Flags(all | TtyFlags::WAITFG)],
            b"\x1b",
            false,
        );
        s.case(
            &[Cmd::Escape(ms), Cmd::Flags(all | TtyFlags::WAITBG)],
            b"\x1b[",
            false,
        );
        s.case(
            &[Cmd::Escape(ms), Cmd::Flags(all | TtyFlags::OSC52QUERY)],
            b"\x1b]",
            false,
        );
        s.case(
            &[Cmd::Escape(ms), Cmd::Flags(all | TtyFlags::WINSIZEQUERY)],
            b"\x1b[8",
            false,
        );
        s.case(
            &[Cmd::Escape(ms), Cmd::Flags(all), Cmd::Requests(true)],
            b"\x1b",
            false,
        );
        for missing in [
            TtyFlags::HAVEDA,
            TtyFlags::HAVEDA2,
            TtyFlags::HAVEXDA,
            TtyFlags::HAVESYNC,
        ] {
            s.case(
                &[
                    Cmd::Escape(ms),
                    Cmd::Flags(TtyFlags(all.bits() & !missing.bits())),
                ],
                b"\x1b",
                false,
            );
        }
    }
    // Paste-end prefix only extends the delay inside a paste.
    for prefix in [
        b"\x1b".as_slice(),
        b"\x1b[",
        b"\x1b[2",
        b"\x1b[20",
        b"\x1b[201",
        b"\x1b[200",
    ] {
        s.pre(&[Cmd::Flags(all)]);
        s.feed(b"\x1b[200~");
        s.steps(1);
        s.feed(prefix);
        s.steps(1);
        s.feed(b"xyz");
        s.steps(4);
        s.pre(&[Cmd::Flags(all)]);
        s.feed(prefix);
        s.steps(1);
    }
    s.pre(&[Cmd::Flags(all)]);
    s.feed(b"\x1b[200~abc\x1b[201~\x1b[20");
    s.steps(8);
    // Paste state persists across completion and the extra-Escape rule.
    s.pre(&[Cmd::Flags(all)]);
    s.feed(b"\x1b\x1b[200~");
    s.steps(3);
    s.feed(b"\x1b[201~");
    s.steps(2);
    // More bytes never restart a running timer; completion cancels it.
    s.pre(&[Cmd::Flags(all)]);
    s.feed(b"\x1b");
    s.steps(1);
    s.feed(b"[");
    s.steps(1);
    s.feed(b"1");
    s.steps(1);
    s.feed(b";5A");
    s.steps(2);
    // Fired timer: incomplete replies become keyboard bytes.
    for prefix in [
        b"\x1b".as_slice(),
        b"\x1b]",
        b"\x1b]52;",
        b"\x1b]52;c;aGVs",
        b"\x1b]10;",
        b"\x1b]4;",
        b"\x1b[?",
        b"\x1b[?62;4",
        b"\x1b[>",
        b"\x1bP",
        b"\x1bP>|",
        b"\x1bP>|tmux",
        b"\x1b[?2026;",
        b"\x1b[<",
        b"\x1b[<0;1",
        b"\x1b[M",
        b"\x1b[M!",
        b"\x1b[27;5",
        b"\x1b[8;24",
        b"\x1bO",
        b"\x1b[",
        b"\x1b[1;",
        b"\xe2\x82",
        b"\x1b\xe2\x82",
    ] {
        for flags in [
            TtyFlags(0),
            all,
            TtyFlags::WINSIZEQUERY | TtyFlags::OSC52QUERY,
        ] {
            s.pre(&[Cmd::Flags(flags)]);
            s.feed(prefix);
            s.steps(1);
            s.push(Cmd::Expire);
            s.steps(prefix.len() + 1);
            s.feed(b"z");
            s.steps(2);
        }
    }
    // No session: everything drains and the timer is cancelled.
    for bytes in [
        b"abc".as_slice(),
        b"\x1b",
        b"\x1b[A\x1b[",
        b"\x1b]52;c;aGVsbG8=\x07",
    ] {
        s.pre(&[Cmd::Session(false)]);
        s.feed(bytes);
        s.steps(2);
        s.pre(&[]);
        s.feed(b"\x1b");
        s.steps(1);
        s.push(Cmd::Session(false));
        s.feed(bytes);
        s.steps(2);
        s.push(Cmd::Session(true));
        s.feed(bytes);
        s.steps(bytes.len() + 1);
    }
    // Expire with no timer and after completion is inert.
    s.pre(&[]);
    s.push(Cmd::Expire);
    s.feed(b"\x1b[A");
    s.steps(1);
    s.push(Cmd::Expire);
    s.feed(b"\x1b");
    s.steps(1);
    s.push(Cmd::Expire);
    s.push(Cmd::Expire);
    s.steps(2);
    compare("timers", &s);
}

#[test]
fn random_sequences() {
    let fragments: &[&[u8]] = &[
        b"\x1b",
        b"[",
        b"O",
        b"]",
        b"P",
        b"<",
        b"M",
        b";",
        b"~",
        b"u",
        b"t",
        b"c",
        b"m",
        b"$",
        b"y",
        b"\\",
        b"\x07",
        b"?",
        b">",
        b"|",
        b"0",
        b"1",
        b"2",
        b"5",
        b"6",
        b"9",
        b"27",
        b"52",
        b"10",
        b"11",
        b"4",
        b"2026",
        b"200",
        b"201",
        b"A",
        b"B",
        b"P",
        b"Z",
        b"a",
        b"z",
        b" ",
        b"\x00",
        b"\x01",
        b"\x08",
        b"\x09",
        b"\x0d",
        b"\x7f",
        b"\x80",
        b"\xc3",
        b"\xa9",
        b"\xe2\x82",
        b"\xac",
        b"\xf0\x9f\x98\x80",
        b"\xff",
        b"!",
        b"#",
        b"rgb:ff/00/00",
        b"red",
        b"aGVsbG8=",
        b"tmux ",
        b"XTerm(",
        b"62",
        b"84",
        b"4",
        b"21",
        b"\x1b[A",
        b"\x1bOP",
        b"\x1b[1;5C",
        b"\x1b[<0;1;1M",
        b"\x1b[M!!!",
        b"\x1b[27;5;13~",
        b"\x1b]10;red\x07",
        b"\x1b[?62;4c",
        b"\x1b[I",
        b"\x1b[O",
        b"\x1b[200~",
        b"\x1b[201~",
    ];
    let flag_choices = [
        TtyFlags(0),
        TtyFlags::ALL_REQUEST_FLAGS,
        TtyFlags::OSC52QUERY | TtyFlags::WINSIZEQUERY | TtyFlags::WAITFG | TtyFlags::WAITBG,
        TtyFlags::HAVEDA | TtyFlags::WINSIZEQUERY,
    ];
    let mut rng = common::Rng::new(0x6808);
    let mut s = Script::default();
    s.push(Cmd::ClearCaps);
    s.push(Cmd::Cap(TtyCodeCode::Kf1, b"\x1bOP".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kcuu1, b"\x1bOA".to_vec()));
    s.push(Cmd::Cap(TtyCodeCode::Kdc5, b"\x1b[3;5~".to_vec()));
    s.push(Cmd::User(0, b"\x1b[M".to_vec()));
    s.push(Cmd::User(1, b"ab".to_vec()));
    s.push(Cmd::Build);
    for _ in 0..1500 {
        let mut bytes = Vec::new();
        for _ in 0..(1 + rng.below(6)) {
            bytes.extend_from_slice(fragments[rng.below(fragments.len() as u64) as usize]);
        }
        let flags = flag_choices[rng.below(4) as usize];
        let mut pre = vec![Cmd::Flags(flags), Cmd::Size(80, 24, 0, 0)];
        if rng.below(4) == 0 {
            pre.push(Cmd::Verase(8));
        }
        if rng.below(5) == 0 {
            pre.push(Cmd::Escape(0));
        }
        s.pre(&pre);
        let mut at = 0;
        while at < bytes.len() {
            let n = 1 + rng.below((bytes.len() - at) as u64) as usize;
            s.feed(&bytes[at..at + n]);
            at += n;
            s.steps(1 + rng.below(3) as usize);
            if rng.below(3) == 0 {
                s.push(Cmd::Expire);
                s.steps(1);
            }
        }
        s.push(Cmd::Expire);
        s.steps(bytes.len() + 1);
    }
    compare("random", &s);
}

#[test]
fn key_names() {
    let mut s = Script::default();
    let mut names: Vec<Vec<u8>> = Vec::new();
    for (name, _) in table_entries() {
        names.push(name.as_bytes().to_vec());
        names.push(name.to_ascii_lowercase().into_bytes());
        names.push(name.to_ascii_uppercase().into_bytes());
        names.push(format!("C-{name}").into_bytes());
        names.push(format!("M-{name}").into_bytes());
        names.push(format!("S-{name}").into_bytes());
        names.push(format!("C-M-S-{name}").into_bytes());
        names.push(format!("s-m-c-{name}").into_bytes());
        names.push(format!("^{name}").into_bytes());
    }
    for n in [
        "None",
        "none",
        "NONE",
        "Any",
        "any",
        "C-None",
        "M-Any",
        "",
        "-",
        "a",
        "A",
        "C-a",
        "C-A",
        "M-a",
        "S-a",
        "C-M-a",
        "C-C-a",
        "X-a",
        "C-",
        "C-M-",
        "^a",
        "^A",
        "^",
        "^ab",
        "^C-a",
        "^-",
        "^^",
        "^ ",
        "^\x7f",
        "^é",
        "^\u{ff}",
        "^\x01",
        "\x01",
        "\x1f",
        " ",
        "\x7f",
        "0x",
        "0x1",
        "0x1f",
        "0x20",
        "0x41",
        "0x7f",
        "0x80",
        "0xe9",
        "0x3b1",
        "0x1f600",
        "0xd800",
        "0x110000",
        "0xffffffff",
        "0X41",
        "0x41zz",
        "0x 41",
        "0x0x41",
        "0x-1",
        "0x+41",
        "0xg",
        "0x7fffffff",
        "User0",
        "User1000",
        "User1001",
        "user5",
        "USER5",
        "User+5",
        "User-0",
        "User-1",
        "User 7",
        "User7x",
        "Userx",
        "User",
        "User05",
        "User99999999999999999999",
        "User\t5",
        "é",
        "éa",
        "€",
        "😀",
        "M-é",
        "C-€",
        "\u{80}",
        "Space",
        "space",
        "BSpace",
        "Tab",
        "Enter",
        "Escape",
        "KP/",
        "kp/",
        "KPEnter",
        "MouseDown1Pane",
        "mousedown1pane",
        "WheelUpStatus",
        "MouseMovePane",
        "FocusIn",
        "PasteStart",
        "Mouse",
        "DC",
        "Delete",
        "IC",
        "Insert",
        "PgUp",
        "NPage",
        "PageDown",
        "F1",
        "f12",
        "F13",
        "Up",
        "M-Up",
        "C-Up",
        "C-M-S-Up",
        "C-S-Tab",
        "S-Tab",
        "BTab",
        "[NUL]",
        "[nul]",
        "Nul",
        "C-[NUL]",
        "\u{1}a",
        "a\u{0}b",
    ] {
        names.push(n.as_bytes().to_vec());
    }
    names.push(b"^\xc3".to_vec());
    names.push(b"User\xa05".to_vec());
    names.push(b"\xc3".to_vec());
    names.push(b"\xff".to_vec());
    names.push(b"\x80\x80".to_vec());
    names.push(b"C-\xff".to_vec());
    names.push(b"\xa0a".to_vec());
    names.push(b"^\xff".to_vec());
    names.push(b"\xc3\xa9\xc3".to_vec());
    for n in &names {
        s.push(Cmd::Name(n.clone()));
    }
    // Output: every table key, special keys, users, literals, Unicode,
    // invalid values, modifiers and flags.
    let mut keys: Vec<u64> = table_entries().map(|(_, k)| k.0).collect();
    keys.extend([
        K::NONE,
        K::UNKNOWN,
        K::ANY,
        K::FOCUS_IN,
        K::FOCUS_OUT,
        K::PASTE_START,
        K::PASTE_END,
        K::REPORT_DARK_THEME,
        K::REPORT_LIGHT_THEME,
        K::MOUSE,
        K::DRAGGING,
        K::MOUSEMOVE_PANE,
        K::MOUSEMOVE_STATUS,
        K::MOUSEMOVE_STATUS_LEFT,
        K::MOUSEMOVE_STATUS_RIGHT,
        K::MOUSEMOVE_STATUS_DEFAULT,
        K::MOUSEMOVE_BORDER,
        K::MOUSEMOVE_SCROLLBAR_UP,
        K::MOUSEMOVE_SCROLLBAR_SLIDER,
        K::MOUSEMOVE_SCROLLBAR_DOWN,
        K::MOUSEMOVE_EMPTY,
        K::MOUSEMOVE_CONTROL0,
        K::MOUSEMOVE_CONTROL9,
        K::MOUSEMOVE1_PANE,
        K::USER,
        K::USER + 999,
        K::USER + 1000,
        K::USER + 1001,
        K::USER + 0xffff_ffff,
        K::F1,
        K::F12,
        K::BSPACE,
        K::UP,
        K::KP_SLASH,
        K::BTAB,
        0,
        1,
        9,
        13,
        27,
        31,
        32,
        33,
        65,
        97,
        126,
        127,
        128,
        200,
        255,
        256,
        0x1000,
        0xffff_ffff,
        0x1_0000_0000 - 1,
        0x200000000 + 200,
        0x2000000ff,
        0x41000041,
        0x4100_00e9,
        0x4200_a9c3,
        0x4300_82e2,
        0x6300_82e2,
        0x0100_0080,
        0x0000_0080,
        0x2000_00ff,
        0xe4000001,
        0xe4ffffff,
        0x200000000,
        0x300000000,
        0x3ff_0000_0000,
        0xff00_0000_0000,
        0x4100_0041 | KeyModifiers::CTRL.0,
        K::F1 | KeyModifiers::CTRL.0,
        K::F1 | KeyModifiers::META.0,
        K::F1 | KeyModifiers::SHIFT.0,
        K::F1 | KeyModifiers::CTRL.0 | KeyModifiers::META.0 | KeyModifiers::SHIFT.0,
        u64::from(b'a') | KeyModifiers::SHIFT.0 | KeyModifiers::CTRL.0,
        u64::from(b'a') | KeyFlags::LITERAL.0,
        KeyFlags::LITERAL.0,
        0x1ff | KeyFlags::LITERAL.0,
        0x100 | KeyFlags::LITERAL.0,
        u64::from(b'a') | KeyFlags::LITERAL.0 | KeyModifiers::CTRL.0,
        u64::from(b'a') | KeyFlags::LITERAL.0 | KeyFlags::KEYPAD.0,
        K::UP | KeyFlags::CURSOR.0,
        K::UP | KeyFlags::CURSOR.0 | KeyFlags::IMPLIED_META.0,
        K::KP_ONE | KeyFlags::KEYPAD.0,
        u64::from(b'a') | KeyFlags::BUILD_MODIFIERS.0,
        u64::from(b'a') | KeyFlags::VI.0,
        u64::from(b'a') | KeyFlags::SENT.0,
        u64::from(b'a') | KeyMasks_flags(),
        K::F1 | KeyMasks_flags() | KeyModifiers::META.0,
        K::IC,
        K::DC,
        K::NPAGE,
        K::PPAGE,
        K::HOME,
        K::END,
        0x8000_0000_0000_0000,
        u64::MAX,
        0x0000_0100_0000_0041,
    ]);
    for k in &keys {
        s.push(Cmd::Key(*k, false));
        s.push(Cmd::Key(*k, true));
    }
    compare("key names", &s);
}

#[allow(non_snake_case)]
fn KeyMasks_flags() -> u64 {
    rmux_util::key::KeyMasks::FLAGS
}
