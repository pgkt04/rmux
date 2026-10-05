// Ported from tmux json.c and regsub.c @ 8f25579c
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

use rmux_server::format::{json, regsub};
use rmux_sys::regex::RegexFlags;
use rmux_util::bytes::cstr;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static REFERENCE: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    if let Some(driver) = std::env::var_os("RMUX_JSON_REGSUB_DRIVER") {
        let driver = PathBuf::from(driver);
        assert!(
            driver.is_file(),
            "RMUX_JSON_REGSUB_DRIVER is missing: {}",
            driver.display()
        );
        return Some(driver);
    }
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/format_json_regsub.c");
    let mut flags = vec!["-D_GNU_SOURCE", "-ffunction-sections"];
    if cfg!(target_os = "macos") {
        flags.extend(["-Wl,-dead_strip", "-L/opt/homebrew/opt/libevent/lib"]);
    } else {
        flags.extend([
            "-UHAVE_BITSTRING_H",
            "-Wl,--gc-sections",
            "-Wl,--no-as-needed",
        ]);
    }
    flags.push("-levent");
    common::build_c(
        "format-json-regsub",
        &[
            &driver,
            Path::new("json.c"),
            Path::new("regsub.c"),
            Path::new("xmalloc.c"),
            Path::new("compat/reallocarray.c"),
            Path::new("compat/recallocarray.c"),
            Path::new("compat/explicit_bzero.c"),
        ],
        &flags,
        false,
    )
});

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

fn ok(bytes: &[u8]) -> String {
    format!("OK {}", hex(bytes))
}

fn error(cause: &json::JsonError) -> String {
    format!("ERR {}", hex(cause.cause()))
}

fn compare(binary: &Path, cases: &[(String, String)]) {
    let mut input = String::new();
    for (request, _) in cases {
        writeln!(input, "{request}").unwrap();
    }
    let output = common::run(binary, &[], input.as_bytes());
    let output = String::from_utf8(output).expect("reference returns ASCII hex");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(
        lines.len(),
        cases.len(),
        "reference omitted comparison cases"
    );
    for ((request, rust), reference) in cases.iter().zip(lines) {
        assert_eq!(rust, reference, "pinned C differs for {request}");
    }
}

fn nesting(depth: usize, arrays: bool) -> Vec<u8> {
    let mut text = Vec::new();
    for _ in 1..depth {
        text.extend_from_slice(if arrays { b"{\"a\":[" } else { b"{\"a\":" });
    }
    text.extend_from_slice(b"{}");
    for _ in 1..depth {
        text.extend_from_slice(if arrays { b"]}" } else { b"}" });
    }
    text
}

#[test]
fn json_parsing_causes_and_serialization_match_pinned_c() {
    let Some(binary) = REFERENCE.as_deref() else {
        return;
    };
    let mut inputs: Vec<Vec<u8>> = [
        "",
        " ",
        "   ",
        "\t\r\n",
        "[]",
        "true",
        "{}",
        " { } ",
        "{} x",
        "{}{}",
        "{}123456789",
        "{}[123456789]",
        "{1:2}",
        "{a",
        "{",
        "{\"a\"",
        "{\"a\" 1}",
        "{\"a\" , 1}",
        "{\"a\":}",
        "{\"a\":",
        "{\"a\":1",
        "{\"a\":true",
        "{\"a\":1 ",
        "{\"a\":1,}",
        "{\"a\":1 \"b\":2}",
        "{\"a\":1,\"a\":2}",
        "{\"\":1}",
        "{\"a\":\"\"}",
        "{\"a\":\"unfinished}",
        "{\"a\":\"\\x\"}",
        "{\"a\":\"\\u12g4\"}",
        "{\"a\":\"\\u12\"}",
        "{\"a\":[]}",
        "{\"a\":[{}]}",
        "{\"a\":[{},{}]}",
        "{\"a\":[1]}",
        "{\"a\":[true]}",
        "{\"a\":[\"x\"]}",
        "{\"a\":[[]]}",
        "{\"a\":[{},]}",
        "{\"a\":[{} {}]}",
        "{\"a\":[{}",
        "{\"a\":[",
        "{\"a\":0}",
        "{\"a\":-0}",
        "{\"a\":01}",
        "{\"a\":-00}",
        "{\"a\":1.5}",
        "{\"a\":1e5}",
        "{\"a\":+1}",
        "{\"a\":-}",
        "{\"a\":9223372036854775807}",
        "{\"a\":-9223372036854775808}",
        "{\"a\":9223372036854775808}",
        "{\"a\":-9223372036854775809}",
        "{\"a\":true}",
        "{\"a\":false}",
        "{\"a\":True}",
        "{\"a\":null}",
        "{\"z\":[{\"b\":2,\"a\":\"\\n\"},{}],\"m\":-5,\"t\":true}",
        "{\"x\":1,\"\\u0078\":2}",
        "{\"a\":\"\\\"\\\\\\/\\b\\f\\n\\r\\t\\u00e9\\uD800\"}",
        "{\"a\":1\u{000b}}",
        "{\"a\":true\u{000c}}",
        "\u{000b}{}",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    inputs.extend([
        b"{}\0ignored".to_vec(),
        b"\0{}".to_vec(),
        b"{\"a\":\"x\0y\"}".to_vec(),
        b"{\"\xff\":\"\x80\",\"a\":\"\xfe\"}".to_vec(),
    ]);
    for byte in 1..0x20 {
        let mut input = b"{\"a\":\"".to_vec();
        input.push(byte);
        input.extend_from_slice(b"\"}");
        inputs.push(input);
    }
    for arrays in [false, true] {
        for depth in [1, 199, 200, 201] {
            inputs.push(nesting(depth, arrays));
        }
    }
    let mut rng = common::Rng::new(0x8f25579c);
    for index in 0..128 {
        inputs.push(format!(
            "{{\"z\":[{{\"n\":{},\"s\":\"case{index}\\t\"}},{{}}],\"a\":{},\"o\":{{\"k\":{}}}}}",
            rng.next_u64() as i64,
            if rng.below(2) == 0 { "false" } else { "true" },
            rng.next_u64() as i64,
        ).into_bytes());
    }
    let cases: Vec<_> = inputs
        .iter()
        .map(|input| {
            let result = match json::parse(input) {
                Ok(node) => {
                    let canonical = node.to_string();
                    let reparsed = json::parse(&canonical).unwrap();
                    assert_eq!(node, reparsed, "canonical JSON did not round trip");
                    assert_eq!(canonical, reparsed.to_string());
                    ok(&canonical)
                }
                Err(cause) => error(&cause),
            };
            (format!("json {}", hex(input)), result)
        })
        .collect();
    compare(binary, &cases);
}

fn getter(node: &json::JsonNode, kind: u8) -> Option<Vec<u8>> {
    match kind {
        b's' => node.as_string().map(<[u8]>::to_vec),
        b'n' => node.as_number().map(|n| n.to_string().into_bytes()),
        b'b' => node
            .as_boolean()
            .map(|b| u8::from(b).to_string().into_bytes()),
        b'o' => node.as_object().map(|_| node.to_string().into_vec()),
        b'a' => node.as_array().map(|_| node.to_string().into_vec()),
        _ => unreachable!(),
    }
}

fn typed_find(node: &json::JsonNode, key: &[u8], kind: u8) -> Result<Vec<u8>, json::JsonError> {
    match kind {
        b's' => node.find_string(key).map(<[u8]>::to_vec),
        b'n' => node.find_number(key).map(|n| n.to_string().into_bytes()),
        b'b' => node
            .find_boolean(key)
            .map(|b| u8::from(b).to_string().into_bytes()),
        b'o' => node.find_object(key).map(|o| o.to_string().into_vec()),
        b'a' => node
            .find_array(key)
            .map(|a| json::JsonNode::Array(a.to_vec()).to_string().into_vec()),
        _ => unreachable!(),
    }
}

#[test]
fn json_getters_typed_finds_and_array_order_match_pinned_c() {
    let Some(binary) = REFERENCE.as_deref() else {
        return;
    };
    let input = b"{\"s\":\"x\\n\",\"n\":-3,\"b\":false,\"o\":{\"z\":1,\"a\":true},\"a\":[{\"i\":2},{\"i\":1},{}],\"empty\":[]}";
    let node = json::parse(input).unwrap();
    let mut cases = Vec::new();
    for key in [
        b"s".as_slice(),
        b"n",
        b"b",
        b"o",
        b"a",
        b"empty",
        b"missing",
        b"s\0ignored",
        b"",
    ] {
        cases.push((
            format!("lookup {} {}", hex(input), hex(key)),
            node.find(key)
                .map_or_else(|| "NONE".into(), |value| ok(&value.to_string())),
        ));
        for kind in b"snboa" {
            let found = typed_find(&node, key, *kind);
            cases.push((
                format!("find {} {} {}", hex(input), hex(key), char::from(*kind)),
                match found {
                    Ok(bytes) => ok(&bytes),
                    Err(cause) => error(&cause),
                },
            ));
            let field = if cstr(key).is_empty() {
                Some(&node)
            } else {
                node.find(key)
            };
            let result = match field {
                None => "NONE".into(),
                Some(field) => {
                    getter(field, *kind).map_or_else(|| "ERR".into(), |bytes| ok(&bytes))
                }
            };
            cases.push((
                format!("get {} {} {}", hex(input), hex(key), char::from(*kind)),
                result,
            ));
        }
        let array = node.find(key).and_then(json::JsonNode::as_array);
        let result = array.map_or_else(
            || "NONE".into(),
            |members| {
                let mut out = String::from("OK ");
                for member in members {
                    assert!(member.find(b"absent").is_none());
                    write!(out, "{},", hex(&member.to_string())).unwrap();
                }
                out
            },
        );
        cases.push((format!("array {} {}", hex(input), hex(key)), result));
    }
    compare(binary, &cases);
}

#[test]
fn regsub_captures_empty_matches_and_flags_match_pinned_c() {
    let Some(binary) = REFERENCE.as_deref() else {
        return;
    };
    let fixtures: &[(&[u8], &[u8], &[u8])] = &[
        (b"", b"x", b""),
        (b"(", b"x", b""),
        (b"", b"x", b"abc"),
        (b"(", b"x", b"abc"),
        (b"[", b"x", b"abc"),
        (b"q", b"x", b"abc"),
        (b"a", b"X", b"aaa"),
        (b"^a", b"X", b"aaa"),
        (b"^", b"X", b"ab"),
        (b"a|^b", b"X", b"abbb"),
        (b"(^a)|b", b"X", b"abaa"),
        (b"$", b"!", b"ab"),
        (b"x*", b"-", b"ab"),
        (b"b*", b"-", b"abc"),
        (b"a*", b"-", b"aaa"),
        (b"a?", b"-", b"ba"),
        (b".*", b"X", b"ab"),
        (b"(a)(b)?", b"[\\1\\2]", b"xacac"),
        (b"(a)(b)", b"\\2\\1\\0", b"abab"),
        (b"(a*)b", b"[\\0/\\1/\\9]", b"b ab aab"),
        (b"(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)", b"\\9\\0", b"abcdefghij"),
        (b"\\(a\\)\\(b\\)", b"\\2\\1", b"abab"),
        (b"a", b"\\x\\", b"a"),
        (b"a", b"\\\\", b"aa"),
        (b"a", b"\\9\\1\\0&", b"ba"),
        (b"A", b"x", b"aA"),
        (b"a.b", b"X", b"a\nb a_b"),
        (b"^b", b"X", b"a\nb\nb"),
        (b"[^a]+", b"X", b"b\nc a d"),
        (b"[[:alpha:]]+", b"X", b"Ab12c"),
        (b"a\0(", b"X\0ignored", b"aba\0tail"),
        (b"\0(", b"X", b"abc"),
        (b"(", b"x", b"\0abc"),
        (b"x*", b"-", b"\xc3\xa9"),
        (b"z", b"", b"\xffz\x80"),
    ];
    let mut cases = Vec::new();
    for flags_text in ["", "e", "i", "ei", "n", "en", "ein"] {
        let mut flags = RegexFlags::NONE;
        if flags_text.contains('e') {
            flags |= RegexFlags::EXTENDED;
        }
        if flags_text.contains('i') {
            flags |= RegexFlags::ICASE;
        }
        if flags_text.contains('n') {
            flags |= RegexFlags::NEWLINE;
        }
        for (pattern, with, text) in fixtures {
            let result = regsub::substitute(pattern, with, text, flags)
                .map_or_else(|_| "ERR".into(), |bytes| ok(&bytes));
            cases.push((
                format!(
                    "regsub {} {} {} {flags_text}",
                    hex(pattern),
                    hex(with),
                    hex(text)
                ),
                result,
            ));
        }
    }
    let mut rng = common::Rng::new(0x579c8f25);
    for _ in 0..256 {
        let patterns: &[&[u8]] = &[
            b"a", b"a*", b"a?", b"(a)(b)?", b"[ab]+", b"a|^b", b"$", b"^a",
        ];
        let replacements: &[&[u8]] = &[b"X", b"", b"[\\0]", b"\\2/\\1/\\9", b"\\x\\"];
        let pattern = patterns[rng.below(patterns.len() as u64) as usize];
        let with = replacements[rng.below(replacements.len() as u64) as usize];
        let text: Vec<_> = (0..rng.below(20))
            .map(|_| b"abc"[rng.below(3) as usize])
            .collect();
        let result = regsub::substitute(pattern, with, &text, RegexFlags::EXTENDED)
            .map_or_else(|_| "ERR".into(), |bytes| ok(&bytes));
        cases.push((
            format!("regsub {} {} {} e", hex(pattern), hex(with), hex(&text)),
            result,
        ));
    }
    compare(binary, &cases);
}
