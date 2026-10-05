// Ported from tmux format.c, regress/format-strings.sh, regress/format-modifiers.sh @ 8f25579c
use super::*;
use std::{cell::Cell, collections::BTreeMap, process::Command, rc::Rc};
#[derive(Default)]
struct Fixture {
    values: BTreeMap<ByteString, ByteString>,
    options: BTreeMap<(u8, ByteString), ByteString>,
    env: BTreeMap<ByteString, ByteString>,
    calls: usize,
    jobs: Vec<ByteString>,
    leases: i32,
    milliseconds: Cell<u64>,
}
impl FormatRuntime for Fixture {
    fn builtin(&mut self, _: &FormatContext, key: &[u8]) -> Option<FormatValue> {
        self.calls += 1;
        self.values.get(key).cloned().map(FormatValue::Bytes)
    }
    fn option(&mut self, _: &FormatContext, scope: OptionScope, key: &[u8]) -> Option<ByteString> {
        self.options.get(&(scope as u8, key.into())).cloned()
    }
    fn environment(&mut self, _: &FormatContext, _: bool, key: &[u8]) -> Option<ByteString> {
        self.env.get(key).cloned()
    }
    fn job(
        &mut self,
        _: Option<crate::ids::ClientId>,
        _: u32,
        _: FormatFlags,
        raw: &[u8],
        _: &[u8],
        _: i64,
    ) -> ByteString {
        self.jobs.push(raw.into());
        b"job".as_slice().into()
    }
    fn monotonic_ms(&self) -> u64 {
        self.milliseconds.get()
    }
    fn now(&self) -> Timestamp {
        Timestamp::new(1700000000, 0)
    }
    fn retain_client(&mut self, _: crate::ids::ClientId) {
        self.leases += 1;
    }
    fn release_client(&mut self, _: crate::ids::ClientId) {
        self.leases -= 1;
    }
}
fn fixture() -> Fixture {
    let mut f = Fixture::default();
    for (k, v) in [
        (b"session_name".as_slice(), b"fixed".as_slice()),
        (b"window_name", b"window"),
        (b"pane_title", b"title"),
        (b"window_index", b"0"),
        (b"pane_index", b"0"),
        (b"pane_id", b"%0"),
    ] {
        f.values.insert(k.into(), v.into());
    }
    f
}
fn run(f: &mut Fixture, input: &[u8]) -> ByteString {
    let mut t = FormatTree::create(None, None, 0, FormatFlags::NONE, f);
    t.expand(f, input)
}
#[test]
fn scanner_escapes_hashes_styles_prefix_and_nul() {
    let mut f = fixture();
    for (input, expected) in [
        (b"#S #W #T".as_slice(), b"fixed window title".as_slice()),
        (b"## ###S ####", b"# #fixed ##"),
        (b"#, #} #: #x#", b", } #: #x"),
        (b"#[fg=#S]#S", b"#[fg=#S]fixed"),
        (b"a#{missing}b#{bad", b"ab"),
        (b"a#(unterminated", b"a"),
        (b"a\0#S", b"a"),
    ] {
        assert_eq!(run(&mut f, input), expected, "{input:?}");
    }
}
#[test]
fn truth_and_delimiter_source_quirks() {
    for v in [None, Some(b"".as_slice()), Some(b"0")] {
        assert!(!true_value(v));
    }
    for v in [b"00".as_slice(), b"-0", b"0.0"] {
        assert!(true_value(Some(v)));
    }
    assert_eq!(skip(b"a},b,c", b","), None);
    assert_eq!(skip(b"a(b,c)", b","), Some(3));
    assert_eq!(skip(b"#{x},y", b","), Some(4));
    assert_eq!(skip(b"a#,b,c", b","), Some(4));
}
#[test]
fn literal_lazy_dispatch_and_fixed_postprocessing() {
    let mut f = fixture();
    for (input, expected) in [
        (b"#{l:#{missing}#,x}".as_slice(), b"#{missing},x".as_slice()),
        (b"#{?missing,#(bad),ok}", b"ok"),
        (b"#{||:1,#(bad)}", b"1"),
        (b"#{&&:0,#(bad)}", b"0"),
        (b"#{==:10,2}", b"0"),
        (b"#{<:10,2}", b"1"),
        (b"#{!;!!:0}", b"1"),
        (b"#{n;w;l:\xe7\x95\x8c}", b"1"),
        (b"#{q;l:a b}", b"a b"),
        (b"#{=/3/../;p6;l:abcdef}", b"abc.. "),
        (b"#{=/-3/../;l:abcdef}", b"..def"),
        (b"#{p-5;l:x}", b"    x"),
        (b"#{R:x,3}", b"xxx"),
        (b"#{R:x,0}", b""),
        (b"#{a:65}", b"A"),
        (b"#{e/+/f/2:1.25,2.5}", b"3.75"),
        (b"#{m/r:^(a|b)$,a}", b"1"),
        (b"#{m/r:[,x}", b"0"),
    ] {
        assert_eq!(run(&mut f, input), expected, "{input:?}");
    }
    assert!(f.jobs.is_empty());
}
#[test]
fn lookup_precedence_lazy_materialization_and_merge() {
    let mut f = fixture();
    let mut tree = FormatTree::create(None, None, 0, FormatFlags::NONE, &mut f);
    tree.add(b"session_name", b"custom".as_slice().into());
    f.env
        .insert(b"session_name".as_slice().into(), b"env".as_slice().into());
    assert_eq!(tree.expand(&mut f, b"#{session_name}"), b"fixed");
    f.values.remove(b"session_name".as_slice());
    assert_eq!(tree.expand(&mut f, b"#{session_name}"), b"");
    f.options.insert(
        (OptionScope::Server as u8, b"session_name".as_slice().into()),
        b"option".as_slice().into(),
    );
    assert_eq!(tree.expand(&mut f, b"#{session_name}"), b"option");
    let count = Rc::new(Cell::new(0));
    let captured = count.clone();
    tree.add_callback(
        b"lazy",
        Box::new(move |_, _| {
            captured.set(captured.get() + 1);
            None
        }),
    );
    tree.add_time(b"time", Timestamp::new(10, 0));
    let mut copy = FormatTree::create(None, None, 0, FormatFlags::NONE, &mut f);
    copy.merge(&tree);
    assert_eq!(copy.expand(&mut f, b"#{lazy}"), b"");
    assert_eq!(count.get(), 0);
    assert_eq!(tree.expand(&mut f, b"#{lazy}#{lazy}"), b"");
    assert_eq!(count.get(), 1);
    copy.merge(&tree);
    assert_eq!(copy.expand(&mut f, b"#{time}"), b"");
}
#[test]
fn recursion_deadline_and_repeat_boundary() {
    let mut f = fixture();
    let mut t = FormatTree::create(None, None, 0, FormatFlags::NONE, &mut f);
    t.add(b"recurse", b"#{E:recurse}".as_slice().into());
    assert_eq!(t.expand(&mut f, b"#{E:recurse}"), b"");
    assert_eq!(run(&mut f, b"#{R:abcdefgh,8192}").len(), 65536);
    assert_eq!(run(&mut f, b"#{R:abcdefgh,8193}"), b"");
    assert_eq!(run(&mut f, b"#{R:,10000}"), b"");
}

#[test]
fn times_modifier_chains_and_shared_limits() {
    let mut f = fixture();
    let mut t = FormatTree::create(None, None, 0, FormatFlags::NONE, &mut f);
    t.add_time(b"zero", Timestamp::new(0, 0));
    t.add_time(b"time", Timestamp::new(1700000000, 0));
    t.add(b"template", b"%Y #{session_name}".as_slice().into());
    t.add(b"future", b"1700000001".as_slice().into());
    assert_eq!(
        t.expand(&mut f, b"#{t:zero}|#{t/r:future}|#{t/d:future}"),
        b"||-1"
    );
    let tm = rmux_sys::time::localtime(1700000000).unwrap();
    let mut buffer = [0; 512];
    let n = rmux_sys::time::strftime(&mut buffer, b"%Y", &tm);
    assert_eq!(t.expand(&mut f, b"#{t/f/%Y:time}"), &buffer[..n]);
    assert_eq!(
        t.expand(&mut f, b"#{T:template}").as_bytes(),
        [&buffer[..n], b" fixed".as_slice()].concat().as_slice()
    );
    assert_eq!(t.expand(&mut f, b"#{E;T:template}"), b"%Y fixed");
    assert_eq!(
        t.expand_time(
            &mut f,
            &vec![b'x'; 8192]
                .into_iter()
                .chain(*b"%")
                .collect::<Vec<_>>()
        ),
        b""
    );
    t.add(b"quote", b"a b#".as_slice().into());
    let quoted = variables::quote_style(&variables::quote_single(&variables::quote_shell(b"a b#")));
    assert_eq!(t.expand(&mut f, b"#{q;q/s;q/h:quote}"), quoted);
    t.flags = FormatFlags::NOJOBS;
    assert_eq!(t.expand(&mut f, b"#(printf job)"), b"");
    assert!(f.jobs.is_empty());
    struct Expired(Cell<u64>);
    impl FormatRuntime for Expired {
        fn monotonic_ms(&self) -> u64 {
            let now = self.0.get();
            self.0.set(now + 100);
            now
        }
    }
    let mut expired = Expired(Cell::new(0));
    assert_eq!(t.expand(&mut expired, b"#{l:later}"), b"");
    assert!(parse::skip_checked(&vec![b'x'; 10001], b"}", || false).is_none());
}

struct Oracle {
    path: std::path::PathBuf,
    socket: std::path::PathBuf,
}
impl Oracle {
    fn start() -> Option<Self> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
        if !path.exists() {
            eprintln!("skipping format oracle: pinned oracle missing");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("rmux-format-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("s");
        let oracle = Self { path, socket };
        oracle.command(&[
            "new-session",
            "-d",
            "-s",
            "fixed",
            "-n",
            "window",
            "sleep 600",
        ]);
        oracle.command(&["select-pane", "-T", "title"]);
        Some(oracle)
    }
    fn command(&self, args: &[&str]) -> Vec<u8> {
        let output = Command::new(&self.path)
            .arg("-S")
            .arg(&self.socket)
            .args(["-f", "/dev/null"])
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "oracle {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
}
impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = Command::new(&self.path)
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server")
            .output();
    }
}
#[test]
fn pinned_oracle_format_corpus() {
    let Some(oracle) = Oracle::start() else {
        return;
    };
    let mut f = fixture();
    let corpus = [
        "#S/#W/#T/#I/#P/#D",
        "## ###S #### #####S",
        "#[fg=#S]#S",
        "#{?missing,wrong,default}",
        "#{?#{==:x,x},chosen,default}",
        "#{l:#{missing}#,x}",
        "#{||:1,#(printf bad)}",
        "#{&&:0,#(printf bad)}",
        "#{!!:00}",
        "#{<:10,2}",
        "#{q;l:a b}",
        "#{n;l:界}",
        "#{w;l:界}",
        "#{=/3/../;p6;l:abcdef}",
        "#{=/-3/../;l:abcdef}",
        "#{p-5;l:x}",
        "#{R:x,3}",
        "#{a:65}",
        "#{c:red}",
        "#{c/f:red}",
        "#{e/+/f/2:1.25,2.5}",
        "#{e/+/f/-1:1.25,2.5}",
        "#{e/+:1.9,2.9}",
        "#{e/==/f:0.000000001,0}",
        "#{m/r:^(a|b)$,a}",
        "#{m/i:A*,abc}",
        "#{m/r:[,x}",
        "#{s/a/b/;l:aaa}",
        "#{s/(a)/<\\1>/;l:aba}",
        "prefix#{==:broken}suffix",
        "prefix#{bad",
        "trailing#",
    ];
    for input in corpus {
        let mut expected = oracle.command(&["display-message", "-p", "-F", input]);
        if expected.last() == Some(&b'\n') {
            expected.pop();
        }
        let actual = run(&mut f, input.as_bytes());
        assert_eq!(actual.0, expected, "format {input:?}");
    }
}
