// Ported from tmux tty-term.c @ 8f25579c
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use rmux_tty::term::terminfo::{TerminfoEntry, TerminfoError, read_list};
use rmux_tty::term::tparm::{TparmArg, TparmState, expand};
use rmux_tty::term::{CODES, CodeKind};

struct Reference {
    root: PathBuf,
    binary: PathBuf,
    prefix: Option<PathBuf>,
}

impl Drop for Reference {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn reference() -> Option<Reference> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let prefix = ["/opt/homebrew/opt/ncurses", "/usr/local/opt/ncurses"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.join("include/ncursesw/term.h").is_file());
    if cfg!(target_os = "macos") && prefix.is_none() {
        eprintln!("SKIP ncurses 6.6 differential: Homebrew ncurses is absent");
        return None;
    }
    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("SKIP ncurses differential: C compiler is absent");
        return None;
    }
    let root = std::env::temp_dir().join(format!(
        "rmux-terminfo-reference-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let binary = root.join("reference");
    let mut cc = Command::new("cc");
    cc.arg("-std=c99")
        .arg("-D_DEFAULT_SOURCE")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/terminfo_reference.c"))
        .arg("-o")
        .arg(&binary);
    if let Some(prefix) = &prefix {
        cc.arg(format!("-I{}", prefix.join("include").display()))
            .arg(format!("-L{}", prefix.join("lib").display()))
            .arg(format!("-Wl,-rpath,{}", prefix.join("lib").display()));
    }
    cc.arg("-lncursesw");
    let output = cc.output().unwrap();
    assert!(
        output.status.success(),
        "ncurses reference compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(Reference {
        root,
        binary,
        prefix,
    })
}

fn run(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {:?}: {}",
        command,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn decode_hex(input: &[u8]) -> Vec<u8> {
    assert_eq!(input.len() % 2, 0);
    input
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn infocmp_string(value: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < value.len() {
        let byte = value[index];
        index += 1;
        if byte == b'^' {
            let ch = value[index];
            index += 1;
            result.push(if ch == b'?' { 0x7f } else { ch & 0x1f });
        } else if byte == b'\\' {
            let ch = value[index];
            index += 1;
            if (b'0'..=b'7').contains(&ch) {
                let mut octal = ch - b'0';
                for _ in 0..2 {
                    if let Some(&next @ b'0'..=b'7') = value.get(index) {
                        octal = octal.wrapping_mul(8).wrapping_add(next - b'0');
                        index += 1;
                    } else {
                        break;
                    }
                }
                result.push(octal);
            } else {
                result.push(match ch {
                    b'E' | b'e' => 0x1b,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'b' => 8,
                    b'f' => 12,
                    b's' => b' ',
                    _ => ch,
                });
            }
        } else {
            result.push(byte);
        }
    }
    result
}

fn compare_infocmp(name: &str, text: &[u8]) {
    let actual = read_list(name.as_bytes()).unwrap();
    let mut checked = 0;
    for line in text.split(|&ch| ch == b'\n') {
        let line = line.trim_ascii();
        let Some(line) = line.strip_suffix(b",") else {
            continue;
        };
        let end = line
            .iter()
            .position(|&ch| b"=#@".contains(&ch))
            .unwrap_or(line.len());
        let Some(code) = CODES
            .iter()
            .find(|code| code.name.as_bytes() == &line[..end])
        else {
            continue;
        };
        let mut expected = code.name.as_bytes().to_vec();
        expected.push(b'=');
        match line.get(end) {
            Some(b'@') => {
                assert!(!actual.iter().any(|cap| cap.starts_with(&expected)));
                continue;
            }
            Some(b'=') => expected.extend(infocmp_string(&line[end + 1..])),
            Some(b'#') => {
                let number = std::str::from_utf8(&line[end + 1..]).unwrap();
                let n = if let Some(hex) = number.strip_prefix("0x") {
                    i32::from_str_radix(hex, 16).unwrap()
                } else if number.starts_with('0') && number.len() > 1 {
                    i32::from_str_radix(number, 8).unwrap()
                } else {
                    number.parse::<i32>().unwrap()
                };
                expected.extend(n.to_string().bytes());
            }
            None => expected.push(b'1'),
            _ => unreachable!(),
        }
        assert!(
            actual.iter().any(|cap| cap.as_bytes() == expected),
            "infocmp {name}: {expected:?}"
        );
        checked += 1;
    }
    assert!(checked > 0, "no infocmp capabilities compared for {name}");
}

fn compare_caps(name: &str, expected: &[u8]) {
    let actual = read_list(name.as_bytes());
    let mut lines = expected.split(|&ch| ch == b'\n');
    let first = lines.next().unwrap();
    if first.starts_with(b"ERR ") {
        let error = match &first[4..] {
            b"1" => TerminfoError::Hardcopy,
            b"0" => TerminfoError::Missing,
            b"-1" => TerminfoError::NoDatabase,
            _ => TerminfoError::Unknown,
        };
        assert_eq!(actual.unwrap_err(), error, "terminal {name}");
        eprintln!("ncurses rejected terminal {name}: {error}");
        return;
    }
    assert_eq!(first, b"OK");
    let actual = actual.unwrap();
    let expected: Vec<Vec<u8>> = lines
        .filter(|s| !s.is_empty())
        .map(|line| {
            let split = line.iter().position(|&ch| ch == b'=').unwrap();
            let name = &line[..split];
            let code = CODES
                .iter()
                .find(|code| code.name.as_bytes() == name)
                .unwrap();
            let mut bytes = line[..=split].to_vec();
            if matches!(code.kind, CodeKind::String) {
                bytes.extend_from_slice(&decode_hex(&line[split + 1..]));
            } else {
                bytes.extend_from_slice(&line[split + 1..]);
            }
            bytes
        })
        .collect();
    assert_eq!(
        actual.iter().map(|s| s.as_bytes()).collect::<Vec<_>>(),
        expected.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        "terminal {name}"
    );
}

#[test]
fn raw_values_match_ncurses_and_infocmp_for_eight_terminals() {
    let Some(reference) = reference() else {
        return;
    };
    for name in [
        "xterm-256color",
        "screen-256color",
        "tmux-256color",
        "screen",
        "vt100",
        "rxvt-unicode-256color",
        "alacritty",
        "foot",
    ] {
        let expected = run(Command::new(&reference.binary).args(["caps", name]));
        if matches!(name, "xterm-256color" | "screen-256color" | "tmux-256color") {
            assert!(
                expected.stdout.starts_with(b"OK\n"),
                "required terminal {name} is unavailable"
            );
        }
        compare_caps(name, &expected.stdout);
        if !expected.stdout.starts_with(b"OK\n") {
            continue;
        }
        let infocmp = reference
            .prefix
            .as_ref()
            .map_or_else(|| PathBuf::from("infocmp"), |p| p.join("bin/infocmp"));
        let output = run(Command::new(infocmp).args(["-1", "-x", name]));
        assert!(
            output
                .stdout
                .windows(name.len())
                .any(|s| s == name.as_bytes())
        );
        compare_infocmp(name, &output.stdout);
    }
}

#[test]
fn typed_parameter_corpus_matches_ncurses() {
    let Some(reference) = reference() else {
        return;
    };
    let cases = [
        ("%%:%i%i%p1%d;%p2%d", vec![3, 9]),
        ("%p1%{3}%+%{2}%*%d", vec![7]),
        ("%p1%{3}%-%p2%/%d", vec![20, 2]),
        ("%p1%p2%m%d", vec![13, 5]),
        ("%p1%p2%&%d:%p1%p2%|%d:%p1%p2%^%d", vec![13, 5]),
        ("%p1%!%d:%p1%~%d", vec![2]),
        ("%p1%p2%=%d:%p1%p2%<%d:%p1%p2%>%d", vec![2, 3]),
        ("%p1%p2%A%d:%p1%p2%O%d", vec![0, 3]),
        ("%'x'%c", vec![]),
        ("%{0}%c", vec![]),
        ("%{256}%cafter", vec![]),
        ("%p1%: #08x", vec![42]),
        ("%p1%:#08X", vec![42]),
        ("%p1%:#6.4o", vec![42]),
        ("%p1%: 07d", vec![-42]),
        ("%p1%:-7.5d", vec![42]),
        ("%p1%:.0d", vec![0]),
        ("%p1%{0}%/%d:%p1%{0}%m%d", vec![7]),
        ("%{2147483647}%{1}%+%d", vec![]),
        ("%{1}%{2}%:+d%{9}%d", vec![]),
        ("%d;%d", vec![5, 6]),
        ("%i%d;%d", vec![5, 6]),
        ("%?%p1%{1}%=%t%?%p2%tA%eB%;%e%p1%{2}%=%tC%eD%;", vec![1, 1]),
        ("%?%p1%{1}%=%tA%e%p1%{2}%=%tB%eC%;", vec![2]),
    ];
    for (cap, params) in cases {
        let values: Vec<String> = params.iter().map(ToString::to_string).collect();
        let expected = run(Command::new(&reference.binary)
            .args(["parm", cap])
            .args(&values));
        let args: Vec<TparmArg<'_>> = params.iter().map(|&n| TparmArg::Int(n)).collect();
        let mut actual = Vec::new();
        expand(
            &mut TparmState::default(),
            cap.as_bytes(),
            &args,
            &mut actual,
        )
        .unwrap();
        assert_eq!(
            actual,
            decode_hex(expected.stdout.trim_ascii()),
            "{cap} {params:?}"
        );
    }
    let mut seed = 0x0766_2026_u32;
    for _ in 0..48 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let x = i64::from((seed % 2001) as i32 - 1000);
        let y = i64::from(((seed >> 12) % 2001) as i32 - 1000);
        let op = b"+-*/m&|^=<>AO"[(seed as usize >> 24) % 13];
        let cap = format!("%p1%p2%{}%d", char::from(op));
        let expected = run(Command::new(&reference.binary).args([
            "parm",
            &cap,
            &x.to_string(),
            &y.to_string(),
        ]));
        let mut actual = Vec::new();
        expand(
            &mut TparmState::default(),
            cap.as_bytes(),
            &[TparmArg::Int(x), TparmArg::Int(y)],
            &mut actual,
        )
        .unwrap();
        assert_eq!(
            actual,
            decode_hex(expected.stdout.trim_ascii()),
            "{cap} {x} {y}"
        );
    }
    for cap in ["%p1%:-8.3s:%p2%s", "%p1%l%d:%p2%s"] {
        let expected =
            run(Command::new(&reference.binary).args(["parm", cap, "ss", "abcdef", "ghi"]));
        let mut actual = Vec::new();
        expand(
            &mut TparmState::default(),
            cap.as_bytes(),
            &[TparmArg::Str(b"abcdef"), TparmArg::Str(b"ghi")],
            &mut actual,
        )
        .unwrap();
        assert_eq!(actual, decode_hex(expected.stdout.trim_ascii()));
    }
    let expected = run(Command::new(&reference.binary).args(["checks", "unused"]));
    assert_eq!(expected.stdout, b"NULL\nNULL\nNULL\nNULL\n");
    for (cap, args) in [
        (b"constant".as_slice(), vec![TparmArg::Int(1)]),
        (b"%p2%d", vec![TparmArg::Int(1)]),
        (b"%p1%s", vec![TparmArg::Int(1)]),
        (b"%p1%d", vec![TparmArg::Str(b"x")]),
    ] {
        let mut out = Vec::new();
        assert!(expand(&mut TparmState::default(), cap, &args, &mut out).is_err());
        assert!(out.is_empty());
    }
    let expected = run(Command::new(&reference.binary).args(["vars", "unused"]));
    let mut state = TparmState::default();
    for (cap, line) in [b"%{42}%PA%{9}%Pa%gA%d:%ga%d".as_slice(), b"%gA%d:%ga%d"]
        .into_iter()
        .zip(expected.stdout.split(|&b| b == b'\n'))
    {
        let mut actual = Vec::new();
        expand(&mut state, cap, &[], &mut actual).unwrap();
        assert_eq!(actual, decode_hex(line));
    }
    let tput = reference
        .prefix
        .as_ref()
        .map_or_else(|| PathBuf::from("tput"), |p| p.join("bin/tput"));
    let cup = read_list(b"xterm-256color")
        .unwrap()
        .into_iter()
        .find(|s| s.starts_with(b"cup="))
        .unwrap();
    for (row, col) in [(0, 0), (1, 2), (24, 79), (999, 999)] {
        let expected = run(Command::new(&tput).args([
            "-T",
            "xterm-256color",
            "cup",
            &row.to_string(),
            &col.to_string(),
        ]));
        let mut actual = Vec::new();
        expand(
            &mut state,
            &cup[4..],
            &[TparmArg::Int(row), TparmArg::Int(col)],
            &mut actual,
        )
        .unwrap();
        assert_eq!(actual, expected.stdout);
    }
}

#[test]
fn reader_environment_case() {
    let Some(binary) = std::env::var_os("RMUX_TERMINFO_REFERENCE_BINARY") else {
        return;
    };
    let name = std::env::var("RMUX_TERMINFO_REFERENCE_NAME").unwrap();
    let expected = run(Command::new(binary).args(["caps", &name]));
    compare_caps(&name, &expected.stdout);
}

fn environment_case(
    reference: &Reference,
    name: &str,
    terminfo: &OsStr,
    home: &Path,
    dirs: &OsStr,
) {
    run(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "reader_environment_case", "--nocapture"])
        .env("TERMINFO", terminfo)
        .env("HOME", home)
        .env("TERMINFO_DIRS", dirs)
        .env("RMUX_TERMINFO_REFERENCE_BINARY", &reference.binary)
        .env("RMUX_TERMINFO_REFERENCE_NAME", name));
}

#[test]
fn user_databases_extended_values_inline_and_setupterm_errors() {
    let Some(reference) = reference() else {
        return;
    };
    let tic = reference
        .prefix
        .as_ref()
        .map_or_else(|| PathBuf::from("tic"), |p| p.join("bin/tic"));
    let source = reference.root.join("fixture.src");
    std::fs::write(&source, b"rmux-fixture|rmux-alias|raw terminfo fixture,\n\tam, AX, RGB@, colors#100000, U8#0, clear=\\E[H\\E[2J$<5>, cup=\\E[%i%p1%d;%p2%dH, Ms=, Cs@, setal=\\E[1m,\nrmux-hardcopy|hardcopy fixture,\n\thc,\nrmux-generic|generic fixture,\n\tgn,\nrmux-generic-addressed|generic addressed fixture,\n\tgn, clear=clear, cup=cursor,\n").unwrap();
    let database = reference.root.join("database");
    run(Command::new(tic)
        .arg("-x")
        .arg("-o")
        .arg(&database)
        .arg(&source));
    let home = reference.root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    for name in [
        "rmux-fixture",
        "rmux-alias",
        "rmux-hardcopy",
        "rmux-generic",
        "rmux-generic-addressed",
        "rmux-absent",
    ] {
        environment_case(
            &reference,
            name,
            database.as_os_str(),
            &home,
            OsStr::new(""),
        );
    }
    let binary = [
        database.join("r/rmux-fixture"),
        database.join("72/rmux-fixture"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .unwrap();
    let bytes = std::fs::read(binary).unwrap();
    let entry = TerminfoEntry::from_bytes(&bytes).unwrap();
    assert_eq!(entry.string(b"Ms"), Some(b"".as_slice()));
    assert_eq!(entry.string(b"Cs"), None);
    assert_eq!(entry.string(b"setal"), Some(b"\x1b[1m".as_slice()));
    assert_eq!(entry.flag(b"AX"), Some(true));
    let hex: String = bytes.iter().fold(String::from("hex:"), |mut s, byte| {
        use std::fmt::Write;
        write!(s, "{byte:02x}").unwrap();
        s
    });
    environment_case(
        &reference,
        "rmux-fixture",
        OsStr::new(&hex),
        &home,
        OsStr::new(""),
    );
    let infocmp = reference
        .prefix
        .as_ref()
        .map_or_else(|| PathBuf::from("infocmp"), |p| p.join("bin/infocmp"));
    let output = run(Command::new(infocmp)
        .args(["-x", "-Q", "2", "-q", "rmux-fixture"])
        .env("TERMINFO", &database));
    let text = std::str::from_utf8(&output.stdout).unwrap().trim();
    environment_case(
        &reference,
        "rmux-alias",
        OsStr::new(text),
        &home,
        OsStr::new(""),
    );
    std::fs::rename(&database, home.join(".terminfo")).unwrap();
    environment_case(
        &reference,
        "rmux-fixture",
        OsStr::new("/rmux-missing-database"),
        &home,
        OsStr::new(""),
    );
    let dirs = reference.root.join("dirs");
    std::fs::rename(home.join(".terminfo"), &dirs).unwrap();
    environment_case(
        &reference,
        "rmux-fixture",
        OsStr::new("/rmux-missing-database"),
        &home,
        dirs.as_os_str(),
    );
    let file = reference.root.join("nonempty.db");
    std::fs::write(&file, b"unsupported hashed database").unwrap();
    environment_case(
        &reference,
        "rmux-absent",
        file.as_os_str(),
        &home,
        OsStr::new(""),
    );
}
