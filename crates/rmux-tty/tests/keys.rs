// Ported from tmux tty-keys.c and key-string.c @ 8f25579c
//! Decoder and key-name checks that need no C reference: tree insertion and
//! lookup rules, decoder-owned state, the active `regress/tty-keys.sh`
//! assertions, and key-name round trips against the oracle `list-keys`.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

use std::process::Command;

use rmux_tty::key_string::{NAME_SIZE, key_name, parse_key_name, table_entries, write_key_name};
use rmux_tty::keys::tables::CODE_KEYS;
use rmux_tty::keys::{
    DecodeStep, KeyDecodeContext, Recognition, TimerPhase, TtyInput, TtyKeyDecoder, UNKNOWN,
    cancel_key_timer, parse_colour_response,
};
use rmux_tty::term::{TtyCodeCode, terminfo};
use rmux_tty::tty::{TtyFlags, TtyTimer};
use rmux_util::key::{KeyCode, KeyFlags, KeyModifiers, SpecialKey as K};

fn ctx() -> KeyDecodeContext {
    KeyDecodeContext {
        flags: TtyFlags::ALL_REQUEST_FLAGS,
        has_session: true,
        escape_time_ms: 10,
        verase: Some(0x7f),
        sx: 80,
        sy: 24,
        xpixel: 0,
        ypixel: 0,
        has_input_requests: false,
    }
}

fn user(i: u32) -> KeyCode {
    KeyCode(K::USER + u64::from(i))
}

fn build(users: &[(u32, &[u8])]) -> TtyKeyDecoder {
    let mut dec = TtyKeyDecoder::new();
    dec.rebuild_with(|_| b"", users.iter().copied());
    dec
}

/// Decode `bytes` completely (expiring the timer when a step waits) and
/// return the keys.
fn decode_all(dec: &mut TtyKeyDecoder, bytes: &[u8], ctx: &KeyDecodeContext) -> Vec<KeyCode> {
    let mut keys = Vec::new();
    let mut buf = bytes.to_vec();
    loop {
        let step = dec.next(&buf, ctx);
        let consumed = match step {
            DecodeStep::Empty => break,
            DecodeStep::Partial { .. } => {
                dec.timer_fired();
                continue;
            }
            DecodeStep::Complete {
                consumed, input, ..
            } => {
                if let TtyInput::Key(k) = input {
                    assert_eq!(k.raw, &buf[..consumed]);
                    keys.push(k.key);
                }
                consumed
            }
            DecodeStep::Discard { consumed, .. } => consumed,
        };
        buf.drain(..consumed);
    }
    keys
}

#[test]
fn tree_replacement_and_prefix_rules() {
    // Exact replacement: a user key overrides a raw entry.
    let dec = build(&[(0, b"\x1b[A")]);
    assert_eq!(dec.find(b"\x1b[A").0.unwrap().key, user(0));

    // Short then long: the lookup for `abc` stops at the known leaf `ab`
    // with input remaining, so the long sequence replaces the leaf's key
    // instead of extending the tree.
    let dec = build(&[(1, b"ab"), (2, b"abc")]);
    let (node, size) = dec.find(b"abc");
    assert_eq!((node.unwrap().key, size), (user(2), 2));
    assert_eq!(dec.find(b"ab").0.unwrap().key, user(2));
    assert_eq!(dec.find(b"abd").0.unwrap().key, user(2));

    // Long then short: lookup of the long prefix returns the interior node,
    // and insertion of the short key replaces that interior node's key.
    let dec = build(&[(3, b"xyz"), (4, b"xy")]);
    let (node, size) = dec.find(b"xy");
    assert_eq!((node.unwrap().key, size), (user(4), 2));
    assert_eq!(dec.find(b"xyz").0.unwrap().key, user(3));
    // One more unrelated byte finds nothing rather than the interior key.
    assert_eq!(dec.find(b"xyq").0, None);
    assert_eq!(dec.find(b"x").0.unwrap().key, UNKNOWN);

    // A longer sequence after a known leaf replaces the leaf's key: the
    // lookup for `abc` stops at the `ab` leaf.
    let dec = build(&[(5, b"ab"), (6, b"abc")]);
    assert_eq!(dec.find(b"ab").0.unwrap().key, user(6));
    assert_eq!(dec.find(b"abc").0.unwrap().key, user(6));
    assert_eq!(dec.find(b"abc").1, 2);

    // Duplicate bytes: last insertion wins.
    let dec = build(&[(7, b"q"), (8, b"q")]);
    assert_eq!(dec.find(b"q").0.unwrap().key, user(8));

    // Indices 0, 999 and 1000 are inserted; above 1000 never.
    let dec = build(&[(999, b"n"), (1000, b"t"), (1001, b"o")]);
    assert_eq!(dec.find(b"n").0.unwrap().key, user(999));
    assert_eq!(dec.find(b"t").0.unwrap().key, user(1000));
    assert_eq!(dec.find(b"o").0, None);

    // High bytes order before ASCII (signed char) but resolve the same.
    let dec = build(&[
        (9, b"\x80\x81"),
        (10, b"\x80"),
        (11, b"\xff"),
        (12, b"\x7f"),
    ]);
    assert_eq!(dec.find(b"\x80\x81").0.unwrap().key, user(9));
    assert_eq!(dec.find(b"\x80").0.unwrap().key, user(10));
    assert_eq!(dec.find(b"\xff").0.unwrap().key, user(11));
    assert_eq!(dec.find(b"\x7f").0.unwrap().key, user(12));
    assert_eq!(dec.find(b"\x80\x82").0, None);

    // An empty user entry is inert.
    let empty = build(&[]);
    let dec = build(&[(13, b""), (14, b"\x00ignored")]);
    assert_eq!(dec.nodes(), empty.nodes());
    assert_eq!(dec.find(b"").0, None);

    let dec = build(&[(15, b"z\x00ignored")]);
    assert_eq!(dec.find(b"z").0.unwrap().key, user(15));
    assert_eq!(dec.find(b"z").1, 1);
}

#[test]
fn rebuild_keeps_decoder_state() {
    let mut dec = build(&[]);
    let c = ctx();
    assert!(matches!(
        dec.next(b"\x1b[200~", &c),
        DecodeStep::Complete { .. }
    ));
    assert!(dec.bracket_paste());
    assert!(matches!(
        dec.next(b"\x1b[<0;5;7M", &c),
        DecodeStep::Complete { .. }
    ));
    assert_eq!(
        (dec.mouse_last().x, dec.mouse_last().y, dec.mouse_last().b),
        (4, 6, 0)
    );
    let step = dec.next(b"\x1b", &c);
    assert!(matches!(step, DecodeStep::Partial { timer: Some(_), .. }));
    assert_eq!(dec.timer_phase(), TimerPhase::Waiting);

    dec.rebuild_with(|_| b"", std::iter::empty());
    assert!(dec.bracket_paste());
    assert_eq!(dec.mouse_last().x, 4);
    assert_eq!(dec.timer_phase(), TimerPhase::Waiting);

    // More bytes do not restart the timer; firing then completes.
    assert!(matches!(
        dec.next(b"\x1b[", &c),
        DecodeStep::Partial { timer: None, .. }
    ));
    dec.timer_fired();
    assert_eq!(dec.timer_phase(), TimerPhase::Fired);
    match dec.next(b"\x1b[", &c) {
        DecodeStep::Complete {
            consumed,
            input: TtyInput::Key(k),
            cancel_timer,
            ..
        } => {
            assert_eq!(consumed, 2);
            assert_eq!(k.key, KeyCode(u64::from(b'[') | KeyModifiers::META.0));
            assert!(cancel_timer);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(dec.timer_phase(), TimerPhase::Idle);

    // Close: clear releases the tree and the timer state.
    dec.next(b"\x1b", &c);
    dec.clear();
    assert!(dec.nodes().is_empty());
    assert_eq!(dec.timer_phase(), TimerPhase::Idle);
    // Firing a timer that is not waiting is inert.
    dec.timer_fired();
    assert_eq!(dec.timer_phase(), TimerPhase::Idle);
    dec.reset_timer();
    assert_eq!(cancel_key_timer().timer, TtyTimer::Key);
    assert_eq!(cancel_key_timer().after, None);
}

#[test]
fn discards_and_timer_cancellation() {
    let mut dec = build(&[]);
    let c = ctx();
    // No session drains everything and cancels an armed timer.
    dec.next(b"\x1b", &c);
    let no_session = KeyDecodeContext {
        has_session: false,
        ..c
    };
    assert_eq!(
        dec.next(b"\x1babc", &no_session),
        DecodeStep::Discard {
            consumed: 4,
            cancel_timer: true
        }
    );
    assert_eq!(
        dec.next(b"xyz", &no_session),
        DecodeStep::Discard {
            consumed: 3,
            cancel_timer: false
        }
    );
    assert_eq!(dec.next(b"", &no_session), DecodeStep::Empty);

    // A wheel release is consumed without an event and keeps the timer.
    dec.next(b"\x1b", &c);
    assert_eq!(
        dec.next(b"\x1b[<64;1;1m", &c),
        DecodeStep::Discard {
            consumed: 10,
            cancel_timer: false
        }
    );
    assert_eq!(dec.timer_phase(), TimerPhase::Waiting);
    assert_eq!(dec.mouse_last().b, 0);

    // Partial colour replies notify the theme without consuming bytes.
    let step = dec.next(b"\x1b]10;", &c);
    assert!(matches!(
        step,
        DecodeStep::Partial {
            theme_changed: true,
            ..
        }
    ));
    assert!(matches!(
        parse_colour_response(b"\x1b]11;red\x07"),
        Recognition::Complete(9, _)
    ));
    assert_eq!(parse_colour_response(b""), Recognition::NoMatch);
}

#[test]
fn escape_time_delays() {
    let delay =
        |escape_time_ms: u32, flags: TtyFlags, requests: bool, bytes: &[u8], paste: bool| {
            let mut dec = build(&[]);
            let c = KeyDecodeContext {
                flags,
                escape_time_ms,
                has_input_requests: requests,
                ..ctx()
            };
            if paste {
                dec.next(b"\x1b[200~", &c);
            }
            match dec.next(bytes, &c) {
                DecodeStep::Partial {
                    timer: Some(req), ..
                } => req.after.unwrap().as_millis(),
                other => panic!("{other:?}"),
            }
        };
    let all = TtyFlags::ALL_REQUEST_FLAGS;
    assert_eq!(delay(0, all, false, b"\x1b", false), 1);
    assert_eq!(delay(10, all, false, b"\x1b", false), 10);
    assert_eq!(delay(499, all, false, b"\x1b", false), 499);
    assert_eq!(delay(1000, all, false, b"\x1b", false), 1000);
    assert_eq!(delay(10, TtyFlags(0), false, b"\x1b", false), 500);
    assert_eq!(
        delay(10, all | TtyFlags::WAITFG, false, b"\x1b", false),
        500
    );
    assert_eq!(
        delay(10, all | TtyFlags::OSC52QUERY, false, b"\x1b", false),
        500
    );
    assert_eq!(
        delay(10, all | TtyFlags::WINSIZEQUERY, false, b"\x1b", false),
        500
    );
    assert_eq!(delay(10, all, true, b"\x1b", false), 500);
    assert_eq!(delay(1000, TtyFlags(0), true, b"\x1b", false), 1000);
    assert_eq!(delay(10, all, false, b"\x1b[20", true), 500);
    assert_eq!(delay(10, all, false, b"\x1b[20", false), 10);
    assert_eq!(delay(10, all, false, b"\x1b[201", true), 500);
    assert_eq!(delay(10, all, false, b"\x1b[201", false), 10);
    assert_eq!(delay(10, all, false, b"\x1b[2", false), 10);
}

/// The terminfo capability name of a key code (`tty_term_codes[]`).
fn cap_name(code: TtyCodeCode) -> String {
    let lower = format!("{code:?}").to_ascii_lowercase();
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

fn terminfo_decoder(term: &str) -> Option<TtyKeyDecoder> {
    let list = terminfo::read_list(term.as_bytes()).ok()?;
    let mut caps: Vec<(TtyCodeCode, Vec<u8>)> = Vec::new();
    for &(code, _) in CODE_KEYS {
        let prefix = format!("{}=", cap_name(code));
        if let Some(v) = list
            .iter()
            .find_map(|c| c.as_bytes().strip_prefix(prefix.as_bytes()))
        {
            caps.push((code, v.to_vec()));
        }
    }
    let mut dec = TtyKeyDecoder::new();
    dec.rebuild_with(
        |code| {
            caps.iter()
                .find(|(c, _)| *c == code)
                .map_or(&[][..], |(_, v)| v)
        },
        std::iter::empty(),
    );
    Some(dec)
}

/// `regress/tty-keys.sh`: the inner tmux runs with `TERM=screen`; each
/// sequence names one key through `command-prompt -k`.
#[test]
fn regress_tty_keys_corpus() {
    let mut dec = match terminfo_decoder("screen") {
        Some(dec) => dec,
        None => {
            eprintln!("screen terminfo not found: using the built-in tables only");
            build(&[])
        }
    };
    let c = ctx();
    let mut cases: Vec<(Vec<u8>, &str)> = Vec::new();
    let c0_names = [
        "C-Space", "C-a", "C-b", "C-c", "C-d", "C-e", "C-f", "C-g", "C-h", "Tab", "C-j", "C-k",
        "C-l", "Enter", "C-n", "C-o", "C-p", "C-q", "C-r", "C-s", "C-t", "C-u", "C-v", "C-w",
        "C-x", "C-y", "C-z", "Escape", "C-\\", "C-]", "C-^", "C-_",
    ];
    for (i, name) in c0_names.iter().enumerate() {
        cases.push((vec![i as u8], name));
    }
    let meta_c0 = [
        "C-M-a", "C-M-b", "C-M-c", "C-M-d", "C-M-e", "C-M-f", "C-M-g", "C-M-h", "M-Tab", "C-M-j",
        "C-M-k", "C-M-l", "M-Enter", "C-M-n", "C-M-o", "C-M-p", "C-M-q", "C-M-r", "C-M-s", "C-M-t",
        "C-M-u", "C-M-v", "C-M-w", "C-M-x", "C-M-y", "C-M-z", "M-Escape", "C-M-\\", "C-M-]",
        "C-M-^", "C-M-_",
    ];
    for (i, name) in meta_c0.iter().enumerate() {
        cases.push((vec![0x1b, i as u8 + 1], name));
    }
    cases.push((vec![0x20], "Space"));
    cases.push((vec![0x1b, 0x20], "M-Space"));
    let printable: Vec<String> = (0x21u8..=0x7e)
        .map(|b| String::from_utf8(vec![b]).unwrap())
        .collect();
    for (b, name) in (0x21u8..=0x7e).zip(printable.iter()) {
        cases.push((vec![b], name));
    }
    let meta_printable: Vec<String> = (0x21u8..=0x7e)
        .map(|b| format!("M-{}", char::from(b)))
        .collect();
    for (b, name) in (0x21u8..=0x7e).zip(meta_printable.iter()) {
        cases.push((vec![0x1b, b], name));
    }
    cases.push((vec![0x7f], "BSpace"));
    cases.push((vec![0x1b, 0x7f], "M-BSpace"));
    let keypad = [
        ("M", "KPEnter"),
        ("j", "KP*"),
        ("k", "KP+"),
        ("m", "KP-"),
        ("n", "KP."),
        ("o", "KP/"),
        ("p", "KP0"),
        ("q", "KP1"),
        ("r", "KP2"),
        ("s", "KP3"),
        ("t", "KP4"),
        ("u", "KP5"),
        ("v", "KP6"),
        ("w", "KP7"),
        ("x", "KP8"),
        ("y", "KP9"),
        ("A", "Up"),
        ("B", "Down"),
        ("C", "Right"),
        ("D", "Left"),
        ("H", "Home"),
        ("F", "End"),
    ];
    let mut owned: Vec<(Vec<u8>, String)> = Vec::new();
    for (ch, name) in keypad {
        owned.push((format!("\x1bO{ch}").into_bytes(), name.to_string()));
        owned.push((format!("\x1b\x1bO{ch}").into_bytes(), format!("M-{name}")));
    }
    for (ch, name) in [
        ("A", "Up"),
        ("B", "Down"),
        ("C", "Right"),
        ("D", "Left"),
        ("H", "Home"),
        ("F", "End"),
    ] {
        owned.push((format!("\x1b[{ch}").into_bytes(), name.to_string()));
        owned.push((format!("\x1b\x1b[{ch}").into_bytes(), format!("M-{name}")));
    }
    for (seq, name) in [
        ("\x1bOa", "C-Up"),
        ("\x1bOb", "C-Down"),
        ("\x1bOc", "C-Right"),
        ("\x1bOd", "C-Left"),
        ("\x1b[a", "S-Up"),
        ("\x1b[b", "S-Down"),
        ("\x1b[c", "S-Right"),
        ("\x1b[d", "S-Left"),
        ("\x1b[11~", "F1"),
        ("\x1b[12~", "F2"),
        ("\x1b[13~", "F3"),
        ("\x1b[14~", "F4"),
        ("\x1b[15~", "F5"),
        ("\x1b[17~", "F6"),
        ("\x1b[18~", "F7"),
        ("\x1b[19~", "F8"),
        ("\x1b[20~", "F9"),
        ("\x1b[21~", "F10"),
        ("\x1b[23~", "F11"),
        ("\x1b[24~", "F12"),
        ("\x1b[25~", "S-F3"),
        ("\x1b[26~", "S-F4"),
        ("\x1b[28~", "S-F5"),
        ("\x1b[29~", "S-F6"),
        ("\x1b[31~", "S-F7"),
        ("\x1b[32~", "S-F8"),
        ("\x1b[33~", "S-F9"),
        ("\x1b[34~", "S-F10"),
        ("\x1b[23$", "S-F11"),
        ("\x1b[24$", "S-F12"),
        ("\x1b[11^", "C-F1"),
        ("\x1b[12^", "C-F2"),
        ("\x1b[13^", "C-F3"),
        ("\x1b[14^", "C-F4"),
        ("\x1b[15^", "C-F5"),
        ("\x1b[17^", "C-F6"),
        ("\x1b[18^", "C-F7"),
        ("\x1b[19^", "C-F8"),
        ("\x1b[20^", "C-F9"),
        ("\x1b[21^", "C-F10"),
        ("\x1b[23^", "C-F11"),
        ("\x1b[24^", "C-F12"),
        ("\x1b[11@", "C-S-F1"),
        ("\x1b[12@", "C-S-F2"),
        ("\x1b[13@", "C-S-F3"),
        ("\x1b[14@", "C-S-F4"),
        ("\x1b[15@", "C-S-F5"),
        ("\x1b[17@", "C-S-F6"),
        ("\x1b[18@", "C-S-F7"),
        ("\x1b[19@", "C-S-F8"),
        ("\x1b[20@", "C-S-F9"),
        ("\x1b[21@", "C-S-F10"),
        ("\x1b[23@", "C-S-F11"),
        ("\x1b[24@", "C-S-F12"),
        ("\x1b[I", "FocusIn"),
        ("\x1b[O", "FocusOut"),
        ("\x1b[200~", "PasteStart"),
        ("\x1b[201~", "PasteEnd"),
        ("\x1b[Z", "BTab"),
        ("\x1b[123;5u", "C-{"),
        ("\x1b[32;2u", "S-Space"),
        ("\x1b[9;5u", "C-Tab"),
        ("\x1b[1;5Z", "C-S-Tab"),
    ] {
        owned.push((seq.as_bytes().to_vec(), name.to_string()));
    }
    for (seq, name) in &owned {
        cases.push((seq.clone(), name));
    }
    for (bytes, expected) in cases {
        let keys = decode_all(&mut dec, &bytes, &c);
        assert_eq!(keys.len(), 1, "{bytes:?} -> {expected}: {keys:x?}");
        let name = String::from_utf8(key_name(keys[0], false)).unwrap();
        assert_eq!(name, expected, "{bytes:?}");
    }
}

#[test]
fn key_name_round_trips_and_buffers() {
    for (name, key) in table_entries() {
        let parsed = parse_key_name(name.as_bytes());
        assert_eq!(parsed.0, key.0 & !KeyFlags::IMPLIED_META.0, "{name}");
        let printed = key_name(key, false);
        // Aliases print as the first table entry with that key.
        assert_eq!(parse_key_name(&printed), parsed, "{name}");
        assert_eq!(parse_key_name(&key_name(parsed, false)), parsed, "{name}");
    }
    assert_eq!(key_name(parse_key_name(b"Insert"), false), b"IC");
    assert_eq!(key_name(parse_key_name(b"PgDn"), false), b"NPage");
    assert_eq!(key_name(parse_key_name(b"M-Up"), true), b"M-Up[CI]");
    assert_eq!(key_name(parse_key_name(b"Up"), true), b"Up[C]");
    assert_eq!(key_name(parse_key_name(b"KP5"), true), b"KP5[K]");
    assert_eq!(
        key_name(KeyCode(u64::from(b'a') | KeyFlags::VI.0), true),
        b"a[]"
    );
    assert_eq!(key_name(KeyCode(KeyFlags::LITERAL.0), true), b"[L]");
    assert_eq!(key_name(KeyCode(K::USER + 1000), false), b"User100");
    assert_eq!(
        key_name(KeyCode(K::MOUSEMOVE_EMPTY), false),
        b"Invalid#300000009"
    );
    assert_eq!(parse_key_name(b"FocusIn").0, K::UNKNOWN);
    assert_eq!(parse_key_name(b"MouseMovePane").0, K::UNKNOWN);
    assert_eq!(
        parse_key_name(b"^a").0,
        u64::from(b'a') | KeyModifiers::CTRL.0
    );
    assert_eq!(
        parse_key_name(b"^A").0,
        u64::from(b'a') | KeyModifiers::CTRL.0
    );
    assert_eq!(
        parse_key_name(b"C-A").0,
        u64::from(b'A') | KeyModifiers::CTRL.0
    );
    assert_eq!(parse_key_name(b"0x1f").0, 0x1f);
    assert_eq!(parse_key_name(b"0X41").0, K::UNKNOWN);
    assert_eq!(parse_key_name(b"User1001").0, K::UNKNOWN);
    assert_eq!(parse_key_name(b"user5").0, K::UNKNOWN);

    // Two successive writes to separate buffers leave the first unchanged.
    let mut a = [0xa5; NAME_SIZE];
    let mut b = [0xa5; NAME_SIZE];
    let la = write_key_name(KeyCode(K::F1 | KeyModifiers::CTRL.0), false, &mut a);
    let lb = write_key_name(KeyCode(K::DOWN), true, &mut b);
    assert_eq!(&a[..la], b"C-F1");
    assert_eq!(&b[..lb], b"Down");
    assert_eq!(a[la], 0);
    assert_eq!(b[lb], 0);
    for key in [
        KeyCode(K::USER + 1000),
        KeyCode(K::MOUSEMOVE_EMPTY),
        parse_key_name("😀".as_bytes()),
        KeyCode(KeyFlags::LITERAL.0),
        KeyCode(u64::from(b'a') | KeyFlags::VI.0),
    ] {
        for flags in [false, true] {
            a.fill(0xa5);
            let len = write_key_name(key, flags, &mut a);
            assert_eq!(a[len], 0, "{key:?}, flags={flags}");
            assert_eq!(&a[..len], key_name(key, flags));
        }
    }
    let len = write_key_name(KeyCode(u64::from(b'x')), false, &mut a);
    assert_eq!(&a[..=len], b"x\0");
}

/// Key names round-trip like the oracle: bind each name in a private table
/// and compare `list-keys -F '#{key_string}'` with the Rust printer.
#[test]
fn key_names_match_oracle_list_keys() {
    let Some(tmux) = common::oracle() else {
        eprintln!("key name oracle test skipped: oracle tmux missing");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rmux-g08-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("sock");
    let run = |args: &[&str]| -> Option<String> {
        let out = Command::new(&tmux)
            .arg("-S")
            .arg(&socket)
            .args(["-f", "/dev/null"])
            .args(args)
            .output()
            .expect("run oracle");
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    assert!(run(&["new-session", "-d", "-x", "80", "-y", "24"]).is_some());

    // Every non-mouse name plus a sample of the 1300 mouse names; each
    // bind-key is one oracle client process.
    let mut names: Vec<String> = table_entries()
        .enumerate()
        .filter(|(i, (n, _))| *i < 90 || i % 23 == 0 || n.ends_with("Control9"))
        .map(|(_, (n, _))| n)
        .collect();
    names.extend(
        [
            "C-a",
            "M-a",
            "S-a",
            "C-M-S-a",
            "c-m-s-b",
            "^c",
            "^D",
            "C-F1",
            "M-F12",
            "S-Up",
            "C-M-Left",
            "M-Home",
            "C-S-Tab",
            "S-Space",
            "M-Space",
            "C-Space",
            "0x41",
            "0x7e",
            "0xe9",
            "0x20ac",
            "0x1f600",
            "User0",
            "User999",
            "User1000",
            "User 7",
            "é",
            "€",
            "M-é",
            "insert",
            "DELETE",
            "pgup",
            "KP/",
            "KP.",
            "WheelUpStatus",
            "MouseDown1Border",
            "TripleClick11Control9",
            "C-?",
            "\\",
            "~",
            "{",
            "'",
            "C-C-a",
            "M-M-b",
            "Tab",
            "Enter",
            "Escape",
            "BSpace",
            "BTab",
            "None",
            "Any",
            "X-a",
            "^",
            "User1001",
            "FocusIn",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let mut expected: Vec<String> = Vec::new();
    for name in &names {
        let bound = run(&["bind-key", "-T", "g08", name, "send-keys", "x"]).is_some();
        let key = parse_key_name(name.as_bytes());
        let valid = key.0 != K::UNKNOWN && key.0 != K::NONE;
        assert_eq!(bound, valid, "{name}");
        if valid {
            expected.push(String::from_utf8(key_name(key, false)).unwrap());
        }
    }
    let listed = run(&["list-keys", "-T", "g08", "-F", "#{key_string}"]).unwrap();
    run(&["kill-server"]);
    let _ = std::fs::remove_dir_all(&dir);
    let mut listed: Vec<&str> = listed.lines().collect();
    listed.sort_unstable();
    listed.dedup();
    let mut expected: Vec<&str> = expected.iter().map(String::as_str).collect();
    expected.sort_unstable();
    expected.dedup();
    assert_eq!(listed, expected);
}

/// Fuzz smoke: arbitrary bytes, contexts and timer steps never panic, and
/// every non-partial step makes progress.
#[test]
fn random_input_never_panics() {
    let mut rng = common::Rng::new(0x0808);
    let mut dec = build(&[(0, b"ab"), (1, b"\x80\x81"), (2, b"\x1b[M")]);
    for _ in 0..20000 {
        let len = 1 + rng.below(12) as usize;
        let mut buf: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        if rng.below(2) == 0 {
            buf[0] = 0x1b;
        }
        let c = KeyDecodeContext {
            flags: TtyFlags(rng.next_u64() as u32 & 0x1ffff),
            has_session: rng.below(16) != 0,
            escape_time_ms: rng.below(1200) as u32,
            verase: (rng.below(3) != 0).then(|| rng.next_u64() as u8),
            sx: rng.below(300) as u32,
            sy: rng.below(100) as u32,
            xpixel: rng.below(20) as u32,
            ypixel: rng.below(20) as u32,
            has_input_requests: rng.below(2) == 0,
        };
        let mut steps = 0;
        while !buf.is_empty() {
            steps += 1;
            assert!(steps < 1000, "no progress on {buf:x?}");
            match dec.next(&buf, &c) {
                DecodeStep::Empty => unreachable!(),
                DecodeStep::Partial { .. } => {
                    if rng.below(2) == 0 {
                        dec.timer_fired();
                    } else {
                        buf.push(rng.next_u64() as u8);
                    }
                }
                DecodeStep::Complete { consumed, .. } | DecodeStep::Discard { consumed, .. } => {
                    assert!(consumed > 0 && consumed <= buf.len());
                    buf.drain(..consumed);
                }
            }
        }
        dec.reset_timer();
    }
    for _ in 0..20000 {
        let len = rng.below(8) as usize;
        let name: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        let key = parse_key_name(&name);
        let printed = key_name(key, rng.below(2) == 0);
        assert!(printed.len() < NAME_SIZE);
        let bits = KeyCode(
            rng.next_u64()
                & if rng.below(2) == 0 {
                    0xff_ffff_ffff_ffff
                } else {
                    0xffff_ffff
                },
        );
        assert!(key_name(bits, true).len() < NAME_SIZE);
    }
}
