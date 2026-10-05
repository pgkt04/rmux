// Ported from tmux options.c, options-table.c @ 8f25579c
//! Unit, C-reference and oracle differential tests for the option store.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_util::bytes::ByteString;
use rmux_util::time::Timestamp;

use super::store::{
    CommandParser, FormatExpander, HooksMonitorId, MonitorSink, OptionsParseCtx, OptionsStore,
    OptionsValue,
};
use super::table::{OPTIONS_OTHER_NAMES, OPTIONS_TABLE, STATUS_FORMAT_DEFAULT};
use super::{
    OptionsArrayKey, OptionsError, OptionsScope, OptionsTableFlags, OptionsTableType, search,
};
use crate::cmd::CommandList;
use crate::cmd::parse::{CmdParseError, CmdParseResult};
use crate::ids::{ArenaId, OptionsId};

const ORACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../oracle/bin/tmux");
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/options-table-8f25579c.txt"
);
const DUMP_SOURCE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/options-table-dump.c"
);

/// A parser double: records inputs, fails on `if -x {`.
#[derive(Default)]
struct StubParser {
    inputs: Vec<ByteString>,
}

impl CommandParser for StubParser {
    fn parse_from_string(&mut self, s: &[u8]) -> CmdParseResult {
        self.inputs.push(ByteString::from(s));
        if s.starts_with(b"if -x {") {
            return Err(CmdParseError::new(ByteString::from("syntax error")));
        }
        Ok(Rc::new(CommandList::default()))
    }
}

/// The real G11 parser with a minimal context, for command options whose
/// printed form is compared with the oracle.
#[derive(Default)]
struct RealParser {
    group: u32,
}

impl crate::cmd::parse::ParseContext for RealParser {
    fn environment(&self, _name: &[u8]) -> Option<&[u8]> {
        None
    }
    fn put_environment(&mut self, _assignment: &[u8], _hidden: bool) {}
    fn alias(&self, _name: &[u8]) -> Option<ByteString> {
        None
    }
    fn condition(&mut self, _format: &[u8], _input: &crate::cmd::parse::CmdParseInput) -> bool {
        false
    }
    fn home(&mut self, _user: Option<&[u8]>) -> Option<ByteString> {
        None
    }
    fn next_group(&mut self) -> u32 {
        self.group = self.group.wrapping_add(1);
        self.group
    }
    fn print(&mut self, _message: &[u8], _input: &crate::cmd::parse::CmdParseInput) {}
}

impl CommandParser for RealParser {
    fn parse_from_string(&mut self, s: &[u8]) -> CmdParseResult {
        let mut input = crate::cmd::parse::CmdParseInput::default();
        crate::cmd::parse::from_string(self, s, &mut input)
    }
}

#[test]
fn numeric_setter_without_parser_matches_materialized_numbers() {
    let mut store = OptionsStore::new();
    let mut parser = StubParser::default();
    store.load_defaults(&mut parser);
    let left = store.create(Some(store.global_w));
    let right = store.create(Some(store.global_w));
    for (name, values) in [
        (b"automatic-rename".as_slice(), [0, 1]),
        (b"remain-on-exit".as_slice(), [2, 0]),
    ] {
        for value in values {
            store.set_number(left, name, value, &mut parser);
            store.set_number_value(right, name, value);
            assert_eq!(store.get_number(left, name), store.get_number(right, name));
            assert_eq!(
                store.get_only(left, name).unwrap().is_number(),
                store.get_only(right, name).unwrap().is_number()
            );
        }
    }
}

#[derive(Default)]
struct RecordingSink {
    freed: Vec<HooksMonitorId>,
}

impl MonitorSink for RecordingSink {
    fn monitor_free(&mut self, monitor: HooksMonitorId) {
        self.freed.push(monitor);
    }
}

struct Expander(RefCell<Vec<ByteString>>);
impl FormatExpander for Expander {
    fn expand(&mut self, s: &[u8]) -> ByteString {
        self.0.borrow_mut().push(ByteString::from(s));
        // "#{x}" expands to "red".
        let mut out = ByteString::new();
        let mut i = 0;
        while i < s.len() {
            if s[i..].starts_with(b"#{x}") {
                out.extend_from_slice(b"red");
                i += 4;
            } else {
                out.push(s[i]);
                i += 1;
            }
        }
        out
    }
}

fn monitor(n: u32) -> HooksMonitorId {
    HooksMonitorId::from_parts(n, 0)
}

fn startup() -> (OptionsStore, StubParser) {
    let mut parser = StubParser::default();
    let mut store = OptionsStore::new();
    store.load_defaults(&mut parser);
    (store, parser)
}

fn set(
    store: &mut OptionsStore,
    parser: &mut dyn CommandParser,
    id: OptionsId,
    name: &str,
    value: Option<&str>,
    append: bool,
) -> Result<(), String> {
    let mut links = HyperlinkRegistry::new();
    let mut ctx = OptionsParseCtx {
        parser,
        links: &mut links,
        program: b"rmux",
    };
    let (base, key) = super::parse_name(name.as_bytes()).expect("name");
    let oe = search(base);
    if let Some(key) = key {
        let o = store
            .get_mut_only(id, base)
            .ok_or_else(|| "missing".to_string())?;
        return o
            .array_set(&key, value.map(str::as_bytes), append, ctx.parser)
            .map_err(|e| e.to_string());
    }
    store
        .from_string(id, oe, base, value.map(str::as_bytes), append, &mut ctx)
        .map_err(|e| e.to_string())
}

fn show(store: &OptionsStore, id: OptionsId, name: &str) -> String {
    let (_, o, key) = store.parse_get(id, name.as_bytes(), false).expect(name);
    String::from_utf8(o.to_string(key.as_ref(), false).into_vec()).unwrap()
}

fn show_only(store: &OptionsStore, id: OptionsId, name: &str) -> Option<String> {
    let (_, o, key) = store.parse_get(id, name.as_bytes(), true)?;
    Some(String::from_utf8(o.to_string(key.as_ref(), false).into_vec()).unwrap())
}

// ---- table ----------------------------------------------------------------

fn dump_str(out: &mut String, key: &str, s: Option<&[u8]>) {
    out.push_str(key);
    out.push('=');
    match s {
        None => out.push_str("<NULL>"),
        Some(s) => {
            for &c in s {
                if c == b'\\' {
                    out.push_str("\\\\");
                } else if !(0x20..0x7f).contains(&c) {
                    out.push_str(&format!("\\x{c:02x}"));
                } else {
                    out.push(c as char);
                }
            }
        }
    }
    out.push('\n');
}

fn dump_list(out: &mut String, key: &str, list: Option<&[&[u8]]>) {
    match list {
        None => out.push_str(&format!("{key}=<NULL>\n")),
        Some(list) => {
            for (i, item) in list.iter().enumerate() {
                dump_str(out, &format!("{key}[{i}]"), Some(item));
            }
            out.push_str(&format!("{key}.len={}\n", list.len()));
        }
    }
}

/// The Rust table in the format of `scripts/options-table-dump.c`.
fn dump_table() -> String {
    let mut out = String::new();
    for map in OPTIONS_OTHER_NAMES {
        out.push_str(&format!(
            "alias {} -> {}\n",
            ByteString::from(map.from),
            ByteString::from(map.to)
        ));
    }
    for oe in OPTIONS_TABLE {
        dump_str(&mut out, "name", Some(oe.name));
        out.push_str(&format!("type={}\n", oe.kind as i32));
        out.push_str(&format!("scope={}\n", oe.scope.bits()));
        out.push_str(&format!("flags={}\n", oe.flags.bits()));
        out.push_str(&format!("minimum={}\n", oe.minimum));
        out.push_str(&format!("maximum={}\n", oe.maximum));
        dump_list(&mut out, "choices", oe.choices);
        dump_str(&mut out, "default_str", oe.default_str);
        out.push_str(&format!("default_num={}\n", oe.default_num));
        dump_list(&mut out, "default_arr", oe.default_arr);
        dump_str(&mut out, "separator", oe.separator);
        dump_str(&mut out, "pattern", oe.pattern);
        dump_str(&mut out, "text", Some(oe.text));
        dump_str(&mut out, "unit", oe.unit);
        out.push('\n');
    }
    out
}

fn configured_fixture() -> String {
    std::fs::read_to_string(FIXTURE)
        .expect("fixture")
        .replace(
            "default_str=tmux-256color\n",
            &format!("default_str={}\n", env!("RMUX_DEFAULT_TERM")),
        )
        .replace(
            "default_str=lock -np\n",
            &format!("default_str={}\n", env!("RMUX_LOCK_COMMAND")),
        )
}

#[test]
fn table_matches_saved_c_dump() {
    let fixture = configured_fixture();
    let dump = dump_table();
    if fixture != dump {
        let f: Vec<&str> = fixture.lines().collect();
        let d: Vec<&str> = dump.lines().collect();
        for (i, (a, b)) in f.iter().zip(d.iter()).enumerate() {
            assert_eq!(a, b, "first difference at line {}", i + 1);
        }
        assert_eq!(f.len(), d.len(), "line counts differ");
    }
}

/// Pinned tmux checkout, if available: `RMUX_TMUX_SRC` or the sibling tree.
fn tmux_tree() -> Option<PathBuf> {
    let candidate = std::env::var_os("RMUX_TMUX_SRC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tmux")
                .to_path_buf()
        });
    candidate.join(".git").exists().then_some(candidate)
}

#[test]
fn table_matches_fresh_c_dump() {
    let Some(tree) = tmux_tree() else {
        eprintln!("skipping: pinned tmux checkout not found (set RMUX_TMUX_SRC)");
        return;
    };
    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rmux-options-table-{}", std::process::id()));
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let archive = Command::new("git")
        .args(["-C", tree.to_str().unwrap(), "archive", "8f25579c"])
        .output()
        .expect("git archive");
    assert!(archive.status.success(), "git archive failed");
    let mut tar = Command::new("tar")
        .args(["-x", "-C", src.to_str().unwrap()])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(tar.stdin.as_mut().unwrap(), &archive.stdout).unwrap();
    assert!(tar.wait().unwrap().success());
    let exe = dir.join("dump");
    let mut cc = Command::new("cc");
    cc.args([
        "-DHAVE_CLOCK_GETTIME",
        "-DHAVE_EVENT2_EVENT_H",
        "-DHAVE_SYS_QUEUE_H",
        "-DHAVE_SYS_TREE_H",
        "-DHAVE_BITSTRING_H",
        "-DHAVE_U_INT",
        "-DHAVE_U_CHAR",
        "-DHAVE_STRLCPY",
        "-DHAVE_STRLCAT",
        "-DHAVE_STRNLEN",
        "-DHAVE_STRNDUP",
        "-DHAVE_SETPROCTITLE",
        "-D_FORTIFY_SOURCE=0",
        // Makefile.am:17 builds with the mouse on; tmux.h:107 is the
        // unconfigured fallback.
        "-DTMUX_MOUSE=1",
    ])
    .arg(format!("-I{}", src.display()));
    cc.arg(format!("-DTMUX_TERM=\"{}\"", env!("RMUX_DEFAULT_TERM")));
    cc.arg(format!("-DTMUX_LOCK_CMD=\"{}\"", env!("RMUX_LOCK_COMMAND")));
    for include in ["/opt/homebrew/opt/libevent/include", "/usr/local/include"] {
        if Path::new(include).exists() {
            cc.arg(format!("-I{include}"));
        }
    }
    let status = cc
        .arg(DUMP_SOURCE)
        .arg(src.join("options-table.c"))
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("cc");
    assert!(status.success(), "compiling the C dump failed");
    let out = Command::new(&exe).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let fresh = String::from_utf8(out.stdout).unwrap();
    assert_eq!(fresh, configured_fixture(), "fixture is stale");
    assert_eq!(fresh, dump_table());
}

#[test]
fn table_invariants() {
    assert_eq!(OPTIONS_TABLE.len(), 269);
    let hooks = OPTIONS_TABLE.iter().filter(|oe| oe.is_hook()).count();
    let after = OPTIONS_TABLE
        .iter()
        .filter(|oe| oe.is_hook() && oe.name.starts_with(b"after-"))
        .count();
    assert_eq!(after, 38);
    assert_eq!(hooks - after, 51);
    assert_eq!(OPTIONS_TABLE.len() - hooks, 180);
    let mut names: Vec<&[u8]> = OPTIONS_TABLE.iter().map(|oe| oe.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), OPTIONS_TABLE.len(), "duplicate names");
    for oe in OPTIONS_TABLE {
        let name = oe.name_str();
        if oe.kind == OptionsTableType::Choice {
            let choices = oe.choices.unwrap_or_else(|| panic!("{name}: no choices"));
            assert!((oe.default_num as usize) < choices.len(), "{name}");
        } else {
            assert!(oe.choices.is_none(), "{name}");
        }
        if oe.is_array() {
            assert!(
                matches!(
                    oe.kind,
                    OptionsTableType::String | OptionsTableType::Command | OptionsTableType::Colour
                ),
                "{name}"
            );
        }
        if oe.is_hook() {
            assert_eq!(oe.kind, OptionsTableType::Command, "{name}");
            assert!(oe.is_array(), "{name}");
            assert_eq!(oe.default_str, Some(&b""[..]), "{name}");
            assert_eq!(oe.separator, Some(&b""[..]), "{name}");
        }
        assert!(
            oe.scope == OptionsScope::SERVER
                || oe.scope == OptionsScope::SESSION
                || oe.scope == OptionsScope::WINDOW
                || oe.scope == (OptionsScope::WINDOW | OptionsScope::PANE),
            "{name}: scope {}",
            oe.scope.bits()
        );
    }
    let sf = search(b"status-format").unwrap();
    assert_eq!(sf.default_arr, Some(STATUS_FORMAT_DEFAULT));
    assert_eq!(STATUS_FORMAT_DEFAULT.len(), 3);
    assert_eq!(
        search(b"default-terminal").unwrap().default_str,
        Some(env!("RMUX_DEFAULT_TERM").as_bytes())
    );
    assert_eq!(
        search(b"lock-command").unwrap().default_str,
        Some(env!("RMUX_LOCK_COMMAND").as_bytes())
    );
    assert_eq!(search(b"mouse").unwrap().default_num, 1);
    assert_eq!(
        search(b"default-shell").unwrap().default_str,
        Some(&b"/bin/sh"[..])
    );
    assert_eq!(search(b"input-buffer-size").unwrap().default_num, 1_048_576);
    assert_eq!(
        search(b"default-size").unwrap().pattern,
        Some(&b"[0-9]*x[0-9]*"[..])
    );
}

#[test]
fn startup_fills_every_global_tree() {
    let (store, _) = startup();
    for oe in OPTIONS_TABLE {
        let name = oe.name_str();
        if oe.scope.contains(OptionsScope::SERVER) {
            assert!(store.get_only(store.global, oe.name).is_some(), "{name}");
        }
        if oe.scope.contains(OptionsScope::SESSION) {
            assert!(store.get_only(store.global_s, oe.name).is_some(), "{name}");
        }
        if oe.scope.contains(OptionsScope::WINDOW) {
            assert!(store.get_only(store.global_w, oe.name).is_some(), "{name}");
        }
    }
    assert_eq!(store.entries(store.global).count(), 47);
}

// ---- oracle differential -----------------------------------------------------

struct Oracle {
    socket: PathBuf,
}

impl Oracle {
    fn start() -> Option<Oracle> {
        if !Path::new(ORACLE).exists() {
            eprintln!("skipping: oracle binary {ORACLE} not found");
            return None;
        }
        let socket = std::env::temp_dir().join(format!(
            "rmux-g09-oracle-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ));
        let oracle = Oracle { socket };
        let status = Command::new(ORACLE)
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .env_remove("TMUX")
            .env("SHELL", "/bin/sh")
            .arg("-S")
            .arg(&oracle.socket)
            .args(["-f", "/dev/null"])
            .args(["new-session", "-d", "-x", "80", "-y", "24"])
            .status()
            .expect("start oracle");
        assert!(status.success());
        Some(oracle)
    }

    fn run(&self, args: &[&str]) -> Vec<String> {
        let out = Command::new(ORACLE)
            .args(["-S", self.socket.to_str().unwrap()])
            .args(args)
            .output()
            .expect("oracle command");
        let text = String::from_utf8(out.stdout).unwrap();
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(out.status.success(), "{args:?}: {err}");
        text.lines().map(str::to_owned).collect()
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = Command::new(ORACLE)
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server")
            .status();
    }
}

/// `cmd_show_options_all` + `cmd_show_options_print` with the default
/// template (`cmd-show-options.c:30-37,195-260,338-390`).
fn render_show_all(
    store: &OptionsStore,
    oo: OptionsId,
    scope: OptionsScope,
    all: bool,
    hooks: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    for o in store.entries(oo) {
        if o.table_entry().is_none() {
            render_entry(o, None, false, &mut lines);
        }
    }
    for oe in OPTIONS_TABLE {
        if !oe.scope.contains(scope) {
            continue;
        }
        if !hooks && oe.is_hook() {
            continue;
        }
        let (o, parent) = match store.get_only(oo, oe.name) {
            Some(o) => (o, false),
            None => {
                if !all {
                    continue;
                }
                match store.get(oo, oe.name) {
                    Some((_, o)) => (o, true),
                    None => continue,
                }
            }
        };
        render_entry(o, None, parent, &mut lines);
    }
    lines
}

fn render_entry(
    o: &super::OptionsEntry,
    key: Option<&OptionsArrayKey>,
    parent: bool,
    lines: &mut Vec<String>,
) {
    let name = String::from_utf8_lossy(o.name()).into_owned();
    let mut has_value = true;
    let value = if let Some(key) = key {
        o.to_string(Some(key), false)
    } else if o.is_array() {
        let keys: Vec<OptionsArrayKey> = o.array_items().map(|(k, _)| k.clone()).collect();
        if !keys.is_empty() {
            for k in &keys {
                render_entry(o, Some(k), parent, lines);
            }
            return;
        }
        has_value = false;
        ByteString::new()
    } else {
        o.to_string(None, false)
    };
    let mut line = name;
    if let Some(key) = key {
        line.push_str(&format!("[{key}]"));
    }
    if parent {
        line.push('*');
    }
    if has_value {
        line.push(' ');
        let shown = if o.is_string() {
            crate::cmd::arguments::escape(&value)
        } else {
            value
        };
        line.push_str(&String::from_utf8_lossy(&shown));
    }
    lines.push(line);
}

/// Startup overrides the oracle applies with `SHELL=/bin/sh` and no
/// `VISUAL`/`EDITOR` (`tmux.c:582-596`).
fn apply_startup_overrides(store: &mut OptionsStore, parser: &mut dyn CommandParser) {
    store.set_string(store.global_s, b"default-shell", false, b"/bin/sh", parser);
}

#[test]
fn show_options_defaults_match_oracle() {
    let Some(oracle) = Oracle::start() else {
        return;
    };
    let mut parser = RealParser::default();
    let mut store = OptionsStore::new();
    store.load_defaults(&mut parser);
    apply_startup_overrides(&mut store, &mut parser);
    let window = store.create(Some(store.global_w));
    let pane = store.create(Some(window));

    let cases: &[(&str, OptionsId, OptionsScope, bool, bool)] = &[
        ("-g", store.global_s, OptionsScope::SESSION, false, false),
        ("-s", store.global, OptionsScope::SERVER, false, false),
        ("-gw", store.global_w, OptionsScope::WINDOW, false, false),
        ("-gH", store.global_s, OptionsScope::SESSION, false, true),
        ("-sH", store.global, OptionsScope::SERVER, false, true),
        ("-gwH", store.global_w, OptionsScope::WINDOW, false, true),
        ("-gA", store.global_s, OptionsScope::SESSION, true, false),
        ("-gwA", store.global_w, OptionsScope::WINDOW, true, false),
        ("-gAH", store.global_s, OptionsScope::SESSION, true, true),
        ("-p", pane, OptionsScope::PANE, false, false),
        ("-pA", pane, OptionsScope::PANE, true, false),
        ("-pAH", pane, OptionsScope::PANE, true, true),
    ];
    for (flags, oo, scope, all, hooks) in cases {
        let expected = oracle.run(&["show-options", flags]);
        let actual = render_show_all(&store, *oo, *scope, *all, *hooks);
        assert_eq!(actual, expected, "show-options {flags}");
    }

    // show -v for every table name in its scope.
    for oe in OPTIONS_TABLE {
        let (flags, oo) = if oe.scope.contains(OptionsScope::SERVER) {
            ("-sv", store.global)
        } else if oe.scope.contains(OptionsScope::SESSION) {
            ("-gv", store.global_s)
        } else {
            ("-gwv", store.global_w)
        };
        let expected = oracle.run(&["show-options", flags, oe.name_str()]);
        let o = store.get_only(oo, oe.name).unwrap();
        let mut actual = Vec::new();
        if o.is_array() {
            for (k, _) in o.array_items() {
                actual.push(String::from_utf8_lossy(&o.to_string(Some(k), false)).into_owned());
            }
        } else {
            actual.push(String::from_utf8_lossy(&o.to_string(None, false)).into_owned());
        }
        assert_eq!(actual, expected, "show-options {flags} {}", oe.name_str());
    }
}

#[test]
fn prefix_matching_matches_oracle() {
    let Some(oracle) = Oracle::start() else {
        return;
    };
    // Every unique prefix of each name resolves like the oracle; ambiguous
    // prefixes report the same error.
    let mut checked = 0;
    for oe in OPTIONS_TABLE {
        let name = oe.name_str();
        for len in 1..=name.len() {
            let prefix = &name[..len];
            let out = Command::new(ORACLE)
                .args(["-S", oracle.socket.to_str().unwrap()])
                .args(["show-options", "-gsw", prefix])
                .output()
                .unwrap();
            let stderr = String::from_utf8(out.stderr).unwrap();
            match super::match_name(prefix.as_bytes()) {
                Err(_) => assert_eq!(
                    stderr.trim(),
                    format!("ambiguous option: {prefix}"),
                    "{prefix}"
                ),
                Ok(Some((resolved, _))) => {
                    assert!(stderr.is_empty(), "{prefix}: oracle said {stderr}");
                    let stdout = String::from_utf8(out.stdout).unwrap();
                    for line in stdout.lines() {
                        let printed = line.split(['[', ' ']).next().unwrap();
                        assert_eq!(printed.as_bytes(), resolved.as_bytes(), "{prefix}");
                    }
                }
                Ok(None) => assert_eq!(stderr.trim(), format!("invalid option: {prefix}")),
            }
            checked += 1;
        }
    }
    assert!(checked > 100);
}

#[test]
fn value_and_array_mutations_match_oracle() {
    let Some(oracle) = Oracle::start() else {
        return;
    };
    let mut parser = RealParser::default();
    let mut store = OptionsStore::new();
    store.load_defaults(&mut parser);
    let cases = [
        ("display-time", Some("4000")),
        ("display-time", Some("-5")),
        ("display-time", Some("99999999999")),
        ("display-time", Some("abc")),
        ("display-time", None),
        ("status-keys", Some("vi")),
        ("status-keys", None),
        ("status-keys", Some("")),
        ("status", Some("3")),
        ("status", None),
        ("focus-events", Some("YES")),
        ("focus-events", Some("")),
        ("focus-events", None),
        ("focus-events", Some("maybe")),
        ("status-bg", Some("#00ff00")),
        ("status-bg", Some("nonsense")),
        ("prefix", Some("C-a")),
        ("prefix", Some("boguskey")),
        ("status-style", Some("fg=red,bg=black")),
        ("status-style", Some("bg=xxxyyy")),
        ("default-size", Some("80x24")),
        ("default-size", Some("wide")),
        ("default-shell", Some("/bin/sh")),
        ("default-shell", Some("/not/a/shell")),
        ("cursor-colour", Some("red")),
        ("cursor-colour", Some("nonsense")),
        ("default-client-command", Some("new-window")),
        ("default-client-command", Some("if -x {")),
        ("pane-colours[007]", Some("red")),
        ("pane-colours[007]", Some("badcolour")),
        ("status-format[abc]", Some("text")),
        ("status-format[a b]", Some("two words")),
        ("status-format[4294967295]", Some("last")),
        ("alert-bell[007]", Some("display-message hello")),
        ("alert-bell[007]", Some("if -x {")),
    ];
    for (name, value) in cases {
        let (base, _) = super::parse_name(name.as_bytes()).unwrap();
        let oe = search(base).unwrap();
        let (flags, id) = if oe.scope == OptionsScope::SERVER {
            ("-s", store.global)
        } else if oe.scope == OptionsScope::SESSION {
            ("-g", store.global_s)
        } else {
            ("-gw", store.global_w)
        };
        let mut cmd = Command::new(ORACLE);
        cmd.args([
            "-S",
            oracle.socket.to_str().unwrap(),
            "set-option",
            flags,
            name,
        ]);
        if let Some(value) = value {
            cmd.arg(value);
        }
        let output = cmd.output().unwrap();
        let actual = set(&mut store, &mut parser, id, name, value, false);
        match actual {
            Ok(()) => assert!(
                output.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(cause) => assert_eq!(
                cause,
                String::from_utf8(output.stderr)
                    .unwrap()
                    .trim_end_matches('\n'),
                "{name}"
            ),
        }
        let show_flags = format!("{flags}v");
        let expected = oracle.run(&["show-options", &show_flags, name]);
        assert_eq!(vec![show(&store, id, name)], expected, "{name}");
    }
}

// ---- store unit tests ----------------------------------------------------------

#[test]
fn from_string_values_corpus() {
    let (mut store, mut parser) = startup();
    let g = store.global_s;
    let gs = store.global;
    let gw = store.global_w;
    let s = &mut store;
    let p = &mut parser;

    assert_eq!(set(s, p, g, "display-time", Some("4000"), false), Ok(()));
    assert_eq!(show(s, g, "display-time"), "4000");
    assert_eq!(
        set(s, p, g, "display-time", Some("-5"), false),
        Err("value is too small: -5".into())
    );
    assert_eq!(
        set(s, p, g, "display-time", Some("abc"), false),
        Err("value is invalid: abc".into())
    );
    assert_eq!(
        set(s, p, g, "display-time", Some("99999999999"), false),
        Err("value is too large: 99999999999".into())
    );
    assert_eq!(
        set(s, p, g, "display-time", None, false),
        Err("empty value".into())
    );

    assert_eq!(set(s, p, g, "status-keys", Some("vi"), false), Ok(()));
    assert_eq!(show(s, g, "status-keys"), "vi");
    assert_eq!(
        set(s, p, g, "status-keys", Some("bogus"), false),
        Err("unknown value: bogus".into())
    );
    assert_eq!(show(s, g, "status-keys"), "vi");
    assert_eq!(set(s, p, g, "status-keys", None, false), Ok(()));
    assert_eq!(show(s, g, "status-keys"), "emacs");
    assert_eq!(
        set(s, p, g, "status-keys", Some(""), false),
        Err("unknown value: ".into())
    );
    // A choice at index >= 2 keeps its value on toggle.
    assert_eq!(set(s, p, g, "status", Some("3"), false), Ok(()));
    assert_eq!(set(s, p, g, "status", None, false), Ok(()));
    assert_eq!(show(s, g, "status"), "3");

    assert_eq!(
        set(s, p, gw, "pane-border-lines", Some("rounded"), false),
        Ok(())
    );
    assert_eq!(show(s, gw, "pane-border-lines"), "rounded");

    assert_eq!(set(s, p, gs, "focus-events", Some("off"), false), Ok(()));
    assert_eq!(show(s, gs, "focus-events"), "off");
    assert_eq!(set(s, p, gs, "focus-events", None, false), Ok(()));
    assert_eq!(show(s, gs, "focus-events"), "on");
    assert_eq!(set(s, p, gs, "focus-events", Some(""), false), Ok(()));
    assert_eq!(show(s, gs, "focus-events"), "off");
    assert_eq!(set(s, p, gs, "focus-events", Some("yes"), false), Ok(()));
    assert_eq!(show(s, gs, "focus-events"), "on");
    assert_eq!(set(s, p, gs, "focus-events", Some("NO"), false), Ok(()));
    assert_eq!(show(s, gs, "focus-events"), "off");
    assert_eq!(set(s, p, gs, "focus-events", Some("1"), false), Ok(()));
    assert_eq!(show(s, gs, "focus-events"), "on");
    assert_eq!(
        set(s, p, gs, "focus-events", Some("maybe"), false),
        Err("bad value: maybe".into())
    );
    let (_, o, _) = s.parse_get(gs, b"focus-events", true).unwrap();
    assert_eq!(o.to_string(None, true), b"1");

    assert_eq!(set(s, p, g, "status-bg", Some("red"), false), Ok(()));
    assert_eq!(show(s, g, "status-bg"), "red");
    assert_eq!(set(s, p, g, "status-bg", Some("colour123"), false), Ok(()));
    assert_eq!(show(s, g, "status-bg"), "colour123");
    assert_eq!(set(s, p, g, "status-bg", Some("#00ff00"), false), Ok(()));
    assert_eq!(show(s, g, "status-bg"), "#00ff00");
    assert_eq!(
        set(s, p, g, "status-bg", Some("xxxyyy"), false),
        Err("bad colour: xxxyyy".into())
    );

    assert_eq!(
        set(s, p, g, "status-style", Some("fg=red,bg=black"), false),
        Ok(())
    );
    assert_eq!(show(s, g, "status-style"), "fg=red,bg=black");
    assert_eq!(
        set(s, p, g, "status-style", Some("bg=xxxyyy"), false),
        Err("invalid style: bg=xxxyyy".into())
    );
    assert_eq!(show(s, g, "status-style"), "fg=red,bg=black");
    // A dynamic style is not validated.
    assert_eq!(
        set(s, p, g, "status-style", Some("bg=#{nope}"), false),
        Ok(())
    );

    assert_eq!(set(s, p, g, "prefix", Some("C-a"), false), Ok(()));
    assert_eq!(show(s, g, "prefix"), "C-a");
    assert_eq!(
        set(s, p, g, "prefix", Some("boguskey"), false),
        Err("bad key: boguskey".into())
    );

    let old = show(s, g, "default-shell");
    assert_eq!(
        set(s, p, g, "default-shell", Some("/not/a/shell"), false),
        Err("not a suitable shell: /not/a/shell".into())
    );
    assert_eq!(show(s, g, "default-shell"), old);
    assert_eq!(
        set(s, p, g, "default-shell", Some("/bin/sh"), false),
        Ok(())
    );
    assert_eq!(
        set(s, p, g, "default-shell", Some("/bin/rmux"), false),
        Err("not a suitable shell: /bin/rmux".into())
    );

    assert_eq!(set(s, p, g, "default-size", Some("80x24"), false), Ok(()));
    assert_eq!(
        set(s, p, g, "default-size", Some("wide"), false),
        Err("value is invalid: wide".into())
    );
    assert_eq!(show(s, g, "default-size"), "80x24");

    assert_eq!(set(s, p, gw, "cursor-colour", Some("red"), false), Ok(()));
    assert_eq!(
        set(s, p, gw, "cursor-colour", Some("nonsense"), false),
        Err("invalid colour: nonsense".into())
    );
    assert_eq!(show(s, gw, "cursor-colour"), "red");
    // The alias resolves on lookup.
    assert_eq!(show(s, gw, "cursor-color"), "red");

    assert_eq!(
        set(
            s,
            p,
            gs,
            "default-client-command",
            Some("new-window"),
            false
        ),
        Ok(())
    );
    assert_eq!(
        set(s, p, gs, "default-client-command", Some("if -x {"), false),
        Err("syntax error".into())
    );
    assert_eq!(p.inputs.last().unwrap(), b"if -x {");

    // User options through the low-level path.
    assert_eq!(
        s.from_string(
            g,
            None,
            b"nope",
            Some(b"x"),
            false,
            &mut OptionsParseCtx {
                parser: p,
                links: &mut HyperlinkRegistry::new(),
                program: b"rmux",
            },
        ),
        Err(OptionsError::text(b"bad option name"))
    );
    s.set_string(g, b"@str", false, b"foo", p);
    s.set_string(g, b"@str", true, b"bar", p);
    assert_eq!(show(s, g, "@str"), "foobar");
    assert_eq!(set(s, p, g, "@str", Some("baz"), true), Ok(()));
    assert_eq!(show(s, g, "@str"), "foobarbaz");
}

#[test]
fn scalar_append_joins_with_separator_and_ignores_inherited() {
    let (mut store, mut parser) = startup();
    let g = store.global_s;
    let session = store.create(Some(g));
    // The default has separator ","? No: status-left has none; use
    // copy-mode-match-style which declares ",".
    let gw = store.global_w;
    store.set_string(gw, b"copy-mode-match-style", false, b"bg=red", &mut parser);
    store.set_string(gw, b"copy-mode-match-style", true, b"fg=blue", &mut parser);
    assert_eq!(show(&store, gw, "copy-mode-match-style"), "bg=red,fg=blue");

    // Append with no local entry sets the value only; the inherited value is
    // not joined.
    store.set_string(g, b"status-left", false, b"GLOBAL", &mut parser);
    store.set_string(session, b"status-left", true, b"LOCAL", &mut parser);
    assert_eq!(show(&store, session, "status-left"), "LOCAL");
    assert_eq!(show(&store, g, "status-left"), "GLOBAL");

    // Rollback after a failed check materialises the inherited value locally.
    store.set_string(g, b"status-style", false, b"bg=red", &mut parser);
    assert!(show_only(&store, session, "status-style").is_none());
    assert_eq!(
        set(
            &mut store,
            &mut parser,
            session,
            "status-style",
            Some("bg=zzz"),
            false
        ),
        Err("invalid style: bg=zzz".into())
    );
    assert_eq!(
        show_only(&store, session, "status-style"),
        Some("bg=red".into())
    );
}

#[test]
fn inheritance_and_remove_or_default() {
    let (mut store, mut parser) = startup();
    let mut sink = RecordingSink::default();
    let session = store.create(Some(store.global_s));
    let window = store.create(Some(store.global_w));
    let pane = store.create(Some(window));

    store.set_number(store.global_w, b"automatic-rename", 0, &mut parser);
    assert_eq!(store.get_number(window, b"automatic-rename"), 0);
    assert_eq!(store.get_number(pane, b"automatic-rename"), 0);
    assert!(store.get_only(pane, b"automatic-rename").is_none());
    let (owner, _) = store.get(pane, b"automatic-rename").unwrap();
    assert_eq!(owner, store.global_w);

    store.set_string(store.global_s, b"status-left", false, b"X", &mut parser);
    assert_eq!(store.get_string(session, b"status-left"), b"X");
    store
        .remove_or_default(store.global_s, b"status-left", None, &mut parser, &mut sink)
        .unwrap();
    assert_eq!(
        store.get_string(session, b"status-left"),
        b"[#{session_name}] "
    );

    store.set_string(session, b"status-left", false, b"S", &mut parser);
    assert_eq!(show_only(&store, session, "status-left"), Some("S".into()));
    store
        .remove_or_default(session, b"status-left", None, &mut parser, &mut sink)
        .unwrap();
    assert!(show_only(&store, session, "status-left").is_none());

    // Pane trees re-parent on move.
    let other = store.create(Some(store.global_w));
    store.set_number(other, b"automatic-rename", 1, &mut parser);
    store.set_parent(pane, Some(other));
    assert_eq!(store.get_number(pane, b"automatic-rename"), 1);
    assert_eq!(store.parent(pane), Some(other));

    // Setting in a child without an entry copies the parent default first.
    store.set_number(pane, b"alternate-screen", 0, &mut parser);
    let o = store.get_only(pane, b"alternate-screen").unwrap();
    assert_eq!(o.table_entry().unwrap().name, b"alternate-screen");
    assert_eq!(store.get_number(window, b"alternate-screen"), 1);
}

#[test]
#[should_panic(expected = "missing option nonsense")]
fn get_string_panics_like_fatalx() {
    let (store, _) = startup();
    store.get_string(store.global, b"nonsense");
}

#[test]
#[should_panic(expected = "option escape-time is not a string")]
fn get_string_type_panics_like_fatalx() {
    let (store, _) = startup();
    store.get_string(store.global, b"escape-time");
}

#[test]
#[should_panic(expected = "no parent options for status")]
fn set_number_user_option_without_parent_panics() {
    let (mut store, mut parser) = startup();
    store.set_number(store.global, b"escape-time", 1, &mut parser);
    let orphan = store.create(None);
    store.set_string(orphan, b"status", false, b"x", &mut parser);
}

#[test]
fn array_corpus() {
    let (mut store, mut parser) = startup();
    let g = store.global_s;
    let s = &mut store;
    let p = &mut parser;
    let items = |s: &OptionsStore, name: &str| -> Vec<String> {
        let o = s.get_only(g, name.as_bytes()).unwrap();
        o.array_items()
            .map(|(k, _)| {
                format!(
                    "{name}[{k}] {}",
                    String::from_utf8_lossy(&o.to_string(Some(k), false))
                )
            })
            .collect()
    };

    let o = s.get_mut_only(g, b"update-environment").unwrap();
    o.array_clear();
    o.array_assign(b"AAA BBB,CCC", p).unwrap();
    assert_eq!(
        items(s, "update-environment"),
        [
            "update-environment[0] AAA",
            "update-environment[1] BBB",
            "update-environment[2] CCC"
        ]
    );
    s.get_mut_only(g, b"update-environment")
        .unwrap()
        .array_assign(b"DDD", p)
        .unwrap();
    assert_eq!(items(s, "update-environment").len(), 4);
    assert_eq!(set(s, p, g, "update-environment[1]", None, false), Ok(()));
    assert_eq!(
        items(s, "update-environment"),
        [
            "update-environment[0] AAA",
            "update-environment[2] CCC",
            "update-environment[3] DDD"
        ]
    );
    assert_eq!(show(s, g, "update-environment[0]"), "AAA");
    assert_eq!(show(s, g, "update-environment[1]"), "");
    assert_eq!(
        set(s, p, g, "update-environment[notify]", Some("EEE"), false),
        Ok(())
    );
    s.get_mut_only(g, b"update-environment")
        .unwrap()
        .array_assign(b"FFF", p)
        .unwrap();
    assert_eq!(
        items(s, "update-environment"),
        [
            "update-environment[0] AAA",
            "update-environment[1] FFF",
            "update-environment[2] CCC",
            "update-environment[3] DDD",
            "update-environment[notify] EEE"
        ]
    );
    assert_eq!(show(s, g, "update-environment"), "AAA FFF CCC DDD EEE");

    // Keyed sets out of order, canonical numeric keys, text keys.
    let o = s.get_mut_only(g, b"status-format").unwrap();
    o.array_clear();
    o.array_assign(b"", p).unwrap();
    assert_eq!(o.array_items().count(), 0);
    assert_eq!(o.to_string(None, false), b"");
    for (k, v) in [
        ("5", "five"),
        ("0", "zero"),
        ("2", "two"),
        ("01", "one"),
        ("zoom", "zoom"),
        ("foo-bar", "foo-bar"),
        ("xterm-256color", "xterm"),
    ] {
        assert_eq!(
            set(s, p, g, &format!("status-format[{k}]"), Some(v), false),
            Ok(())
        );
    }
    assert_eq!(
        items(s, "status-format"),
        [
            "status-format[0] zero",
            "status-format[1] one",
            "status-format[2] two",
            "status-format[5] five",
            "status-format[foo-bar] foo-bar",
            "status-format[xterm-256color] xterm",
            "status-format[zoom] zoom"
        ]
    );
    assert_eq!(show(s, g, "status-format[01]"), "one");
    assert_eq!(set(s, p, g, "status-format[zoom]", None, false), Ok(()));
    assert_eq!(show(s, g, "status-format[zoom]"), "");
    // Bad keys.
    assert_eq!(
        OptionsArrayKey::parse(b"4294967296")
            .unwrap_err()
            .to_string(),
        "bad array key: 4294967296"
    );
    assert_eq!(
        OptionsArrayKey::parse(b"4294967295"),
        Ok(OptionsArrayKey::Index(u32::MAX))
    );
    assert_eq!(
        OptionsArrayKey::parse(b"a b"),
        Ok(OptionsArrayKey::Name("a b".into()))
    );

    // Separators: user-keys splits on " ,", command-alias on ",", hooks on
    // nothing.
    let gs = s.global;
    let o = s.get_mut_only(gs, b"user-keys").unwrap();
    o.array_assign(b"One,Two Three", p).unwrap();
    assert_eq!(o.to_string(None, false), b"One Two Three");
    assert_eq!(o.array_items().count(), 2);
    let o = s.get_mut_only(gs, b"command-alias").unwrap();
    o.array_clear();
    o.array_assign(b"a=b c,d=e", p).unwrap();
    let keys: Vec<String> = o
        .array_items()
        .map(|(k, i)| format!("{k}={}", String::from_utf8_lossy(i.value().as_string())))
        .collect();
    assert_eq!(keys, ["0=a=b c", "1=d=e"]);
    let o = s.get_mut_only(g, b"alert-bell").unwrap();
    o.array_assign(b"display-message hi, and more", p).unwrap();
    assert_eq!(o.array_items().count(), 1);
    assert_eq!(p.inputs.last().unwrap(), b"display-message hi, and more");

    // Command arrays parse; errors keep the old item and its serial.
    assert_eq!(
        set(s, p, g, "alert-bell[0]", Some("display-message hi"), false),
        Ok(())
    );
    let serial = s
        .get_only(g, b"alert-bell")
        .unwrap()
        .array_items()
        .next()
        .unwrap()
        .1
        .serial();
    assert_eq!(
        set(s, p, g, "alert-bell[0]", Some("if -x {"), false),
        Err("syntax error".into())
    );
    let o = s.get_only(g, b"alert-bell").unwrap();
    let (_, item) = o.array_items().next().unwrap();
    assert_eq!(item.serial(), serial);
    assert!(matches!(item.value(), OptionsValue::Command(Some(_))));
    assert_eq!(
        set(s, p, g, "alert-bell[0]", Some("display-message bye"), false),
        Ok(())
    );
    assert_eq!(
        s.get_only(g, b"alert-bell")
            .unwrap()
            .array_items()
            .next()
            .unwrap()
            .1
            .serial(),
        serial
    );

    // Colour arrays.
    let gw = s.global_w;
    assert_eq!(set(s, p, gw, "pane-colours[0]", Some("red"), false), Ok(()));
    assert_eq!(show(s, gw, "pane-colours[0]"), "red");
    assert_eq!(
        set(s, p, gw, "pane-colours[1]", Some("xxxyyy"), false),
        Err("bad colour: xxxyyy".into())
    );
    assert_eq!(show(s, gw, "pane-colours"), "red");
    let serial = s
        .get_only(gw, b"pane-colours")
        .unwrap()
        .array_items()
        .next()
        .unwrap()
        .1
        .serial();
    assert_eq!(
        set(s, p, gw, "pane-colours[0]", Some("zzz"), false),
        Err("bad colour: zzz".into())
    );
    assert_eq!(show(s, gw, "pane-colours[0]"), "red");
    assert_eq!(
        s.get_only(gw, b"pane-colours")
            .unwrap()
            .array_items()
            .next()
            .unwrap()
            .1
            .serial(),
        serial
    );
    assert_eq!(
        set(s, p, gw, "pane-colours[0]", Some("blue"), false),
        Ok(())
    );
    assert_eq!(
        s.get_only(gw, b"pane-colours")
            .unwrap()
            .array_items()
            .next()
            .unwrap()
            .1
            .serial(),
        serial
    );
    assert_eq!(set(s, p, gw, "pane-colours[0]", None, false), Ok(()));
    assert_eq!(
        set(s, p, gw, "pane-colours[0]", Some("blue"), false),
        Ok(())
    );
    assert_ne!(
        s.get_only(gw, b"pane-colours")
            .unwrap()
            .array_items()
            .next()
            .unwrap()
            .1
            .serial(),
        serial
    );

    // Not an array / wrong type.
    assert_eq!(
        set(s, p, g, "status-left[0]", Some("x"), false),
        Err("not an array".into())
    );
    let o = s.get_mut_only(g, b"status-left").unwrap();
    assert!(o.array_get(&OptionsArrayKey::Index(0)).is_none());
    assert_eq!(o.array_items().count(), 0);
    o.array_clear();

    // String item append joins without a separator; identity is stable.
    assert_eq!(set(s, p, g, "status-format[0]", Some("A"), false), Ok(()));
    let serial = s
        .get_only(g, b"status-format")
        .unwrap()
        .array_items()
        .next()
        .unwrap()
        .1
        .serial();
    assert_eq!(set(s, p, g, "status-format[0]", Some("B"), true), Ok(()));
    assert_eq!(show(s, g, "status-format[0]"), "AB");
    assert_eq!(
        s.get_only(g, b"status-format")
            .unwrap()
            .array_items()
            .next()
            .unwrap()
            .1
            .serial(),
        serial
    );
    assert_eq!(set(s, p, g, "status-format[7]", Some("B"), true), Ok(()));
    assert_eq!(show(s, g, "status-format[7]"), "B");
}

#[test]
fn array_assign_exhaustion_branches() {
    let (mut store, mut parser) = startup();
    let g = store.global_s;
    // Split path: stops successfully when the first free key reaches the limit.
    let o = store.get_mut_only(g, b"update-environment").unwrap();
    o.array_clear();
    o.array_assign_limited(b"a b c d", &mut parser, 2).unwrap();
    let keys: Vec<String> = o.array_items().map(|(k, _)| k.to_string()).collect();
    assert_eq!(keys, ["0", "1"]);
    // A failure stops at the failing piece and keeps earlier ones.
    let o = store.get_mut_only(g, b"alert-bell").unwrap();
    o.array_clear();
    assert_eq!(o.array_assign(b"ok", &mut parser), Ok(()));
    let o = store.get_mut_only(g, b"alert-activity").unwrap();
    o.array_clear();
    // Hooks have an empty separator: the whole string is one item at the
    // first free key, and at exhaustion key UINT_MAX (limit) is replaced.
    o.array_assign_limited(b"display-message one", &mut parser, 1)
        .unwrap();
    o.array_assign_limited(b"display-message two", &mut parser, 1)
        .unwrap();
    o.array_assign_limited(b"display-message three", &mut parser, 1)
        .unwrap();
    let keys: Vec<String> = o.array_items().map(|(k, _)| k.to_string()).collect();
    assert_eq!(keys, ["0", "1"]);
    assert_eq!(parser.inputs.last().unwrap(), b"display-message three");
    // First free key skips gaps from the start, not past the highest key.
    let o = store.get_mut_only(g, b"status-format").unwrap();
    o.array_clear();
    o.array_set(&OptionsArrayKey::Index(1), Some(b"one"), false, &mut parser)
        .unwrap();
    o.array_set(
        &OptionsArrayKey::Name("z".into()),
        Some(b"z"),
        false,
        &mut parser,
    )
    .unwrap();
    o.array_assign(b"zero two", &mut parser).unwrap();
    let keys: Vec<String> = o.array_items().map(|(k, _)| k.to_string()).collect();
    assert_eq!(keys, ["0", "1", "2", "z"]);
    assert_eq!(o.to_string(None, false), b"zero one two z");
}

#[test]
fn style_cache_behaviour() {
    let (mut store, mut parser) = startup();
    let mut links = HyperlinkRegistry::new();
    let g = store.global_s;
    let session = store.create(Some(g));

    // Valid static style parses once and caches.
    store.set_string(g, b"status-style", false, b"bg=red", &mut parser);
    let style = store
        .string_to_style(session, b"status-style", None, &mut links)
        .unwrap();
    assert_eq!(style.gc.bg, rmux_emu::colour::Colour(1));
    assert!(
        store
            .get_only(g, b"status-style")
            .unwrap()
            .cached_style()
            .is_some()
    );
    assert_eq!(
        store.string_to_style(session, b"status-style", None, &mut links),
        Some(style)
    );
    // set_string clears the cache.
    store.set_string(g, b"status-style", false, b"bg=blue", &mut parser);
    assert!(
        store
            .get_only(g, b"status-style")
            .unwrap()
            .cached_style()
            .is_none()
    );
    let style = store
        .string_to_style(g, b"status-style", None, &mut links)
        .unwrap();
    assert_eq!(style.gc.bg, rmux_emu::colour::Colour(4));

    // Dynamic style never caches; expands only with a tree.
    store.set_string(g, b"status-style", false, b"bg=#{x}", &mut parser);
    let mut ft = Expander(RefCell::new(Vec::new()));
    let style = store
        .string_to_style(g, b"status-style", Some(&mut ft), &mut links)
        .unwrap();
    assert_eq!(style.gc.bg, rmux_emu::colour::Colour(1));
    assert_eq!(ft.0.borrow().len(), 1);
    assert!(
        store
            .get_only(g, b"status-style")
            .unwrap()
            .cached_style()
            .is_none()
    );
    assert_eq!(
        store.string_to_style(g, b"status-style", None, &mut links),
        None
    );
    assert!(
        store
            .get_only(g, b"status-style")
            .unwrap()
            .cached_style()
            .is_none()
    );

    // Invalid user style: None once, then the stored style.
    store.set_string(g, b"@bad", false, b"bg=zzz", &mut parser);
    assert_eq!(store.string_to_style(g, b"@bad", None, &mut links), None);
    let cached = store.get_only(g, b"@bad").unwrap().cached_style();
    assert!(cached.is_some());
    assert_eq!(store.string_to_style(g, b"@bad", None, &mut links), cached);

    // Colour options use style_parse_colour.
    let gw = store.global_w;
    store.set_string(gw, b"cursor-colour", false, b"green", &mut parser);
    let style = store
        .string_to_style(gw, b"cursor-colour", None, &mut links)
        .unwrap();
    assert_eq!(style.gc.fg, rmux_emu::colour::Colour(2));
    // Non-string options give None.
    assert_eq!(
        store.string_to_style(gw, b"cursor-style", None, &mut links),
        None
    );
    assert_eq!(
        store.string_to_style(gw, b"nonsense", None, &mut links),
        None
    );
}

#[test]
fn monitor_lifetime_ordering() {
    let (mut store, mut parser) = startup();
    let g = store.global_s;
    let session = store.create(Some(g));
    store.set_string(session, b"@a", false, b"1", &mut parser);
    store.set_string(session, b"@b", false, b"2", &mut parser);
    store
        .get_mut_only(session, b"@a")
        .unwrap()
        .set_monitor(Some(monitor(1)));
    store
        .get_mut_only(session, b"@b")
        .unwrap()
        .set_monitor(Some(monitor(2)));

    // Staged removal: value destroyed, entry still linked, then unlinked.
    let token = store.prepare_removal(session, b"@a").unwrap();
    assert_eq!(token.monitor, Some(monitor(1)));
    let o = store.get_only(session, b"@a").unwrap();
    assert_eq!(o.value().as_string(), b"");
    assert_eq!(o.monitor(), None);
    assert_eq!(o.serial(), token.serial());
    // A replacement during cleanup is not destroyed by the stale token.
    let old_serial = token.serial();
    store.finish_removal(token);
    assert!(store.get_only(session, b"@a").is_none());
    store.set_string(session, b"@a", false, b"new", &mut parser);
    let new_serial = store.get_only(session, b"@a").unwrap().serial();
    assert_ne!(new_serial, old_serial);
    store.set_string(session, b"status-left", false, b"local", &mut parser);
    let token = store.prepare_removal(session, b"status-left").unwrap();
    let replaced = store
        .default(session, search(b"status-left").unwrap(), &mut parser)
        .serial();
    assert_ne!(replaced, token.serial());
    store.finish_removal(token);
    let o = store.get_only(session, b"status-left").unwrap();
    assert_eq!(o.serial(), replaced);
    assert_eq!(o.value().as_string(), b"[#{session_name}] ");

    // free_with frees each monitor in byte order, one entry at a time.
    let mut store2 = OptionsStore::new();
    let t = store2.create(None);
    store2.set_string(t, b"@a", false, b"1", &mut parser);
    store2.set_string(t, b"@b", false, b"2", &mut parser);
    store2
        .get_mut_only(t, b"@a")
        .unwrap()
        .set_monitor(Some(monitor(1)));
    store2
        .get_mut_only(t, b"@b")
        .unwrap()
        .set_monitor(Some(monitor(2)));
    let mut sink = RecordingSink::default();
    // Remove @a first: @b still linked with its monitor afterwards.
    store2.remove(t, b"@a", &mut sink);
    assert_eq!(sink.freed, [monitor(1)]);
    assert_eq!(
        store2.get_only(t, b"@b").unwrap().monitor(),
        Some(monitor(2))
    );
    store2.free_with(t, &mut sink);
    assert_eq!(sink.freed, [monitor(1), monitor(2)]);
    assert!(store2.parent(store2.global).is_none());

    // default_with and empty_with free the replaced monitor and reset
    // counters, serial and fire stats.
    let oe = search(b"status-left").unwrap();
    store
        .get_mut_only(g, b"status-left")
        .unwrap()
        .set_monitor(Some(monitor(3)));
    store
        .get_mut_only(g, b"status-left")
        .unwrap()
        .hook_fired(Timestamp::new(10, 0));
    let serial = store.get_only(g, b"status-left").unwrap().serial();
    let mut sink = RecordingSink::default();
    let o = store.default_with(g, oe, &mut parser, &mut sink);
    assert_ne!(o.serial(), serial);
    assert_eq!(o.fire_count(), 0);
    assert_eq!(o.fire_time(), Timestamp::ZERO);
    assert_eq!(o.monitor(), None);
    assert_eq!(sink.freed, [monitor(3)]);
    store
        .get_mut_only(g, b"status-left")
        .unwrap()
        .set_monitor(Some(monitor(4)));
    let o = store.empty_with(g, oe, &mut sink);
    assert_eq!(o.value().as_string(), b"");
    assert_eq!(sink.freed, [monitor(3), monitor(4)]);
    // remove_or_default on a global resets through the sink.
    store
        .get_mut_only(g, b"status-left")
        .unwrap()
        .set_monitor(Some(monitor(5)));
    store
        .remove_or_default(g, b"status-left", None, &mut parser, &mut sink)
        .unwrap();
    assert_eq!(sink.freed, [monitor(3), monitor(4), monitor(5)]);
    assert_eq!(store.get_string(g, b"status-left"), b"[#{session_name}] ");
    // Nothing to remove.
    assert!(!store.remove(g, b"@none", &mut sink));
    assert!(store.prepare_removal(g, b"@none").is_none());
}

#[test]
fn command_value_drops_before_monitor_cleanup() {
    struct ValueSink(std::rc::Weak<CommandList>, bool);
    impl MonitorSink for ValueSink {
        fn monitor_free(&mut self, _: HooksMonitorId) {
            assert!(
                self.0.upgrade().is_none(),
                "command value still alive during monitor cleanup"
            );
            self.1 = true;
        }
    }
    let (mut store, mut parser) = startup();
    let id = store.global;
    let list = Rc::new(CommandList::default());
    let mut sink = ValueSink(Rc::downgrade(&list), false);
    store
        .set_command(id, b"default-client-command", list, &mut parser)
        .set_monitor(Some(monitor(1)));
    let token = store
        .prepare_removal(id, b"default-client-command")
        .unwrap();
    assert!(store.get_only(id, b"default-client-command").is_some());
    sink.monitor_free(token.monitor.unwrap());
    assert!(sink.1);
    assert!(store.get_only(id, b"default-client-command").is_some());
    store.finish_removal(token);
    assert!(store.get_only(id, b"default-client-command").is_none());
}

#[test]
#[should_panic(expected = "live hooks monitor")]
fn pure_default_rejects_monitored_entry() {
    let (mut store, mut parser) = startup();
    let g = store.global_s;
    store
        .get_mut_only(g, b"status-left")
        .unwrap()
        .set_monitor(Some(monitor(9)));
    store.default(g, search(b"status-left").unwrap(), &mut parser);
}

#[test]
fn hook_counters_wrap_and_entry_serials() {
    let (mut store, mut parser) = startup();
    let g = store.global_s;
    let o = store.get_mut_only(g, b"alert-bell").unwrap();
    let serial = o.serial();
    for _ in 0..3 {
        o.hook_fired(Timestamp::new(5, 0));
    }
    assert_eq!(o.fire_count(), 3);
    assert_eq!(o.fire_time(), Timestamp::new(5, 0));
    // Wrap at u32::MAX.
    let o = store.get_mut_only(g, b"alert-bell").unwrap();
    o.set_fire_count_for_test(u32::MAX);
    assert_eq!(o.fire_count(), u32::MAX);
    o.hook_fired(Timestamp::new(6, 0));
    assert_eq!(o.fire_count(), 0);
    // In-place value update keeps the serial; default replacement changes it.
    store.set_string(g, b"status-left", false, b"a", &mut parser);
    let s1 = store.get_only(g, b"status-left").unwrap().serial();
    store.set_string(g, b"status-left", false, b"b", &mut parser);
    assert_eq!(store.get_only(g, b"status-left").unwrap().serial(), s1);
    store.default(g, search(b"status-left").unwrap(), &mut parser);
    assert_ne!(store.get_only(g, b"status-left").unwrap().serial(), s1);
    assert_eq!(store.get_only(g, b"alert-bell").unwrap().serial(), serial);
}

#[test]
fn default_to_string_and_from_default() {
    let mut parser = StubParser::default();
    assert_eq!(
        super::default_to_string(search(b"escape-time").unwrap()),
        b"10"
    );
    assert_eq!(
        super::default_to_string(search(b"backspace").unwrap()),
        b"C-?"
    );
    assert_eq!(super::default_to_string(search(b"prefix").unwrap()), b"C-b");
    assert_eq!(super::default_to_string(search(b"mouse").unwrap()), b"on");
    assert_eq!(
        super::default_to_string(search(b"status-keys").unwrap()),
        b"emacs"
    );
    assert_eq!(
        super::default_to_string(search(b"display-panes-colour").unwrap()),
        b"themeblue"
    );
    assert_eq!(
        super::default_to_string(search(b"default-client-command").unwrap()),
        b"new-session"
    );
    assert_eq!(
        super::default_to_string(search(b"status-left").unwrap()),
        b"[#{session_name}] "
    );

    let o = super::OptionsEntry::from_default(search(b"status-format").unwrap(), &mut parser);
    assert_eq!(o.array_items().count(), 3);
    assert_eq!(
        o.array_get(&OptionsArrayKey::Index(1)).unwrap().as_string(),
        STATUS_FORMAT_DEFAULT[1]
    );
    let o = super::OptionsEntry::from_default(search(b"command-alias").unwrap(), &mut parser);
    assert_eq!(o.array_items().count(), 6);
    assert_eq!(
        o.array_get(&OptionsArrayKey::Index(2)).unwrap().as_string(),
        b"server-info=show-messages -JT"
    );
    let o =
        super::OptionsEntry::from_default(search(b"default-client-command").unwrap(), &mut parser);
    assert!(matches!(o.value(), OptionsValue::Command(Some(_))));
    assert_eq!(parser.inputs.last().unwrap(), b"new-session");
    let o = super::OptionsEntry::new_empty(search(b"escape-time").unwrap());
    assert_eq!(o.value().as_number(), Some(0));
    assert!(o.is_number());
    assert!(!o.is_string());
    assert_eq!(o.to_string(None, false), b"0");
}

#[test]
fn match_get_and_parse_get() {
    let (mut store, mut parser) = startup();
    let session = store.create(Some(store.global_s));
    store.set_string(store.global_s, b"@u", false, b"v", &mut parser);
    let (owner, o, key) = store
        .match_get(session, b"status-inte", false)
        .unwrap()
        .unwrap();
    assert_eq!(owner, store.global_s);
    assert_eq!(o.name(), b"status-interval");
    assert_eq!(key, None);
    assert!(
        store
            .match_get(session, b"status-interval", true)
            .unwrap()
            .is_none()
    );
    assert!(store.match_get(session, b"status-l", false).is_err());
    assert!(store.match_get(session, b"@nope", false).unwrap().is_none());
    let (_, o, key) = store
        .match_get(session, b"status-format[2]", false)
        .unwrap()
        .unwrap();
    assert_eq!(o.name(), b"status-format");
    assert_eq!(key, Some(OptionsArrayKey::Index(2)));
    assert!(
        store
            .parse_get(session, b"status-format[]", false)
            .is_none()
    );
    let (_, o, _) = store.parse_get(session, b"@u", false).unwrap();
    assert_eq!(o.value().as_string(), b"v");
    assert!(o.table_entry().is_none());
    assert!(o.is_string());
    assert!(!o.is_array());
    // Alias in parse_get through get_only.
    let gw = store.global_w;
    let (_, o, _) = store.parse_get(gw, b"pane-colors", true).unwrap();
    assert_eq!(o.name(), b"pane-colours");
}

#[test]
fn snapshot_reads_the_right_trees() {
    let (mut store, mut parser) = startup();
    let window = store.create(Some(store.global_w));
    store.set_number(store.global, b"escape-time", 42, &mut parser);
    store.set_number(window, b"alternate-screen", 0, &mut parser);
    store
        .get_mut_only(store.global, b"user-keys")
        .unwrap()
        .array_assign(b"\\033[A,\\033[B", &mut parser)
        .unwrap();
    let snap = super::OptionsSnapshot::from_store(&store, window);
    assert_eq!(snap.escape_time, 42);
    assert!(!snap.alternate_screen);
    assert!(snap.scroll_on_clear);
    assert!(snap.variation_selector_always_wide);
    assert_eq!(snap.extended_keys, 0);
    assert_eq!(
        snap.user_keys,
        [ByteString::from("\\033[A"), ByteString::from("\\033[B")]
    );
}

#[test]
fn environ_update_reads_inherited_option() {
    use super::environment::{Environment, environ_update};
    let (mut store, mut parser) = startup();
    let session = store.create(Some(store.global_s));
    store
        .get_mut_only(store.global_s, b"update-environment")
        .unwrap()
        .array_clear();
    store
        .get_mut_only(store.global_s, b"update-environment")
        .unwrap()
        .array_assign(b"DISPLAY FOO_*", &mut parser)
        .unwrap();
    let mut src = Environment::new();
    src.set(b"FOO_1", super::environment::EnvironmentFlags(0), b"1");
    src.set(b"FOO_2", super::environment::EnvironmentFlags(0), b"2");
    src.set(b"BAR", super::environment::EnvironmentFlags(0), b"3");
    let mut dst = Environment::new();
    environ_update(&store, session, &src, &mut dst);
    let names: Vec<String> = dst
        .iter()
        .map(|(n, _)| String::from_utf8_lossy(n).into_owned())
        .collect();
    assert_eq!(names, ["DISPLAY", "FOO_1", "FOO_2"]);
    assert_eq!(dst.find(b"DISPLAY").unwrap().value, None);
    let orphan = store.create(None);
    environ_update(&store, orphan, &src, &mut dst);
    assert_eq!(dst.len(), 3);
}

#[test]
fn error_flags_are_not_hooks() {
    // Sanity on the G00 flag constants used by the table.
    assert_eq!(OptionsTableFlags::ARRAY.bits(), 1);
    assert_eq!(OptionsTableFlags::HOOK.bits(), 2);
    assert_eq!(OptionsTableFlags::STYLE.bits(), 4);
    assert_eq!(OptionsTableFlags::COLOUR.bits(), 8);
    assert_eq!(OptionsTableType::Command as i32, 6);
}
