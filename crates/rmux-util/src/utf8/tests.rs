use std::process::Command;

use super::combined::{HangulJamoState, hanguljamo_check_state, should_combine};
use super::table::{MAX_INDEX, with_table};
use super::tables::{DEFAULT_WIDTHS, SKIN_TONE_BASES, WHITESPACE};
use super::*;

const FATAL_ENV: &str = "RMUX_UTF8_TEST_FATAL";

fn setup() {
    rmux_sys::locale::setup_ctype().expect("UTF-8 locale");
    with_width_cache(|cache| cache.rebuild(std::iter::empty()));
}

fn decode(bytes: &[u8]) -> (Utf8Data, Utf8State) {
    let mut ud = Utf8Data::open(bytes[0]).expect("lead byte");
    let mut state = Utf8State::More;
    for &b in &bytes[1..] {
        state = ud.append(b);
    }
    (ud, state)
}

fn data(bytes: &[u8], width: u8) -> Utf8Data {
    let mut ud = Utf8Data::default();
    ud.data[..bytes.len()].copy_from_slice(bytes);
    ud.size = bytes.len() as u8;
    ud.have = ud.size;
    ud.width = width;
    ud
}

#[test]
fn open_classifies_lead_bytes() {
    for ch in 0u8..=0xff {
        let expected = match ch {
            0xc2..=0xdf => Some(2),
            0xe0..=0xef => Some(3),
            0xf0..=0xf4 => Some(4),
            _ => None,
        };
        let got = Utf8Data::open(ch).ok();
        assert_eq!(got.map(|ud| ud.size), expected, "lead {ch:02x}");
        if let Some(ud) = got {
            assert_eq!(ud.have, 1);
            assert_eq!(ud.data[0], ch);
            assert_eq!(ud.width, 0);
        }
    }
}

#[test]
fn decoder_states() {
    setup();
    let (ud, state) = decode(b"\xc3\xa9");
    assert_eq!(state, Utf8State::Done);
    assert_eq!((ud.size, ud.have, ud.width), (2, 2, 1));
    let (ud, state) = decode(b"\xe4\xb8\xad");
    assert_eq!(state, Utf8State::Done);
    assert_eq!(ud.width, 2);
    let (ud, state) = decode(b"\xf0\x9f\x98\x80");
    assert_eq!(state, Utf8State::Done);
    assert_eq!(ud.width, 2);

    let (ud, state) = decode(b"\xe2\x82");
    assert_eq!(state, Utf8State::More);
    assert_eq!((ud.have, ud.size), (2, 3));

    let (ud, state) = decode(b"\xc3\x41");
    assert_eq!(state, Utf8State::Error);
    assert_eq!(ud.width, INVALID_WIDTH);
    assert_eq!(ud.bytes(), b"\xc3\x41");

    // A bad continuation does not stop the sequence.
    let mut ud = Utf8Data::open(0xe2).unwrap();
    assert_eq!(ud.append(b'x'), Utf8State::More);
    assert_eq!(ud.width, INVALID_WIDTH);
    assert_eq!(ud.append(0x82), Utf8State::Error);

    // Surrogates and C0/C1 leads are rejected.
    assert_eq!(decode(b"\xed\xa0\x80").1, Utf8State::Error);
    assert!(Utf8Data::open(0xc0).is_err());
    assert!(Utf8Data::open(0xc1).is_err());
    assert!(Utf8Data::open(0xf5).is_err());
}

#[test]
fn no_width_mode_skips_width_step() {
    let mut ud = Utf8Data::open(0xc3).unwrap();
    assert_eq!(ud.append_no_width(0x41), Utf8State::Done);
    assert_eq!(ud.width, INVALID_WIDTH);
    let mut ud = Utf8Data::open(0xc3).unwrap();
    assert_eq!(ud.append_no_width(0xa9), Utf8State::Done);
    assert_eq!(ud.width, 0);
}

fn run_fatal_child(case: &str, test: &str) {
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args([test, "--exact"])
        .env(FATAL_ENV, case)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{case}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "{case}: stderr {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn fatal_child() {
    let Ok(case) = std::env::var(FATAL_ENV) else {
        return;
    };
    match case.as_str() {
        "overflow" => {
            let mut ud = Utf8Data::set(b'a');
            ud.append(b'b');
        }
        "size" => {
            let mut ud = Utf8Data {
                size: 33,
                ..Utf8Data::default()
            };
            ud.append(0xf0);
        }
        "width" => {
            from_data(&data(b"a", 3));
        }
        other => panic!("unknown case {other}"),
    }
    panic!("fatalx returned");
}

#[test]
fn append_overflow_is_fatal() {
    run_fatal_child("overflow", "utf8::tests::fatal_child");
}

#[test]
fn append_size_too_large_is_fatal() {
    run_fatal_child("size", "utf8::tests::fatal_child");
}

#[test]
fn from_data_width_three_is_fatal() {
    run_fatal_child("width", "utf8::tests::fatal_child");
}

#[test]
fn copy_from_clears_unused_tail() {
    let mut from = data(b"\xc3\xa9", 1);
    from.data[2] = 0x55;
    from.data[31] = 0x66;
    let mut to = Utf8Data::default();
    to.copy_from(&from);
    assert_eq!(to.bytes(), b"\xc3\xa9");
    assert_eq!(&to.data[2..], &[0u8; 30]);
    assert_eq!((to.have, to.size, to.width), (2, 2, 1));
}

#[test]
fn set_builds_single_byte() {
    let ud = Utf8Data::set(b'z');
    assert_eq!(ud, data(b"z", 1));
}

#[test]
fn build_one_layout() {
    assert_eq!(build_one(b'a'), Utf8Char(0x4100_0061));
    let uc = build_one(b'a');
    assert_eq!((uc.size(), uc.width(), uc.payload()), (1, 1, 0x61));
}

#[test]
fn inline_three_byte_data() {
    let (uc, state) = from_data(&data(b"\xe2\x82\xac", 1));
    assert_eq!(state, Utf8State::Done);
    assert_eq!(uc, Utf8Char(0x43ac_82e2));
    let back = to_data(uc);
    assert_eq!(back.bytes(), b"\xe2\x82\xac");
    assert_eq!((back.have, back.width), (3, 1));
}

#[test]
fn inline_packing_reads_all_three_bytes() {
    let mut ud = data(b"a", 1);
    ud.data[1] = 0xbb;
    ud.data[2] = 0xcc;
    let (uc, state) = from_data(&ud);
    assert_eq!(state, Utf8State::Done);
    assert_eq!(uc, Utf8Char(0x41cc_bb61));
    assert_eq!(to_data(uc).data[..3], [0x61, 0xbb, 0xcc]);
    let mut copy = Utf8Data::default();
    copy.copy_from(&ud);
    assert_eq!(from_data(&copy).0, Utf8Char(0x4100_0061));
}

#[test]
fn table_indexes_in_fresh_thread() {
    std::thread::spawn(|| {
        let first = data(b"\xf0\x9f\x98\x80", 2);
        let (uc, state) = from_data(&first);
        assert_eq!(state, Utf8State::Done);
        assert_eq!(uc, Utf8Char(0x6400_0000));
        assert_eq!(uc.payload(), 0);

        let mut tail = first;
        tail.data[4] = 0xff;
        tail.width = 1;
        tail.have = 0;
        let (uc2, _) = from_data(&tail);
        assert_eq!(
            uc2.payload(),
            0,
            "unused tail and width do not change identity"
        );
        assert_eq!(uc2.width(), 1);

        let (uc3, _) = from_data(&data(b"\xf0\x9f\x98\x81", 2));
        assert_eq!(uc3.payload(), 1);
        assert_eq!(uc3, Utf8Char(0x6400_0001));

        let back = to_data(uc3);
        assert_eq!(back.bytes(), b"\xf0\x9f\x98\x81");
        assert_eq!((back.have, back.size, back.width), (4, 4, 2));

        let unknown = to_data(Utf8Char(0x6500_0042));
        assert_eq!(unknown.bytes(), b"     ");
        assert_eq!((unknown.have, unknown.width), (5, 2));

        assert_eq!(with_table(|table| table.len()), 2);
    })
    .join()
    .unwrap();
}

#[test]
fn table_limit_returns_replacement() {
    std::thread::spawn(|| {
        let existing = data(b"\xf0\x9f\x98\x80", 2);
        assert_eq!(from_data(&existing).0.payload(), 0);
        with_table(|table| table.skip_to_index(MAX_INDEX));
        let (uc, state) = from_data(&data(b"\xf0\x9f\x98\x81", 2));
        assert_eq!(state, Utf8State::Done);
        assert_eq!(uc.payload(), MAX_INDEX);
        let (uc, state) = from_data(&data(b"\xf0\x9f\x98\x82", 2));
        assert_eq!(state, Utf8State::Error);
        assert_eq!(uc, Utf8Char(0x4100_2020));
        let (uc, state) = from_data(&data(b"\xf0\x9f\x98\x82", 1));
        assert_eq!((uc, state), (Utf8Char(0x4100_0020), Utf8State::Error));
        let (uc, state) = from_data(&data(b"\xf0\x9f\x98\x82", 0));
        assert_eq!((uc, state), (Utf8Char(0x2000_0000), Utf8State::Error));
        // An existing sequence still succeeds at the limit.
        let (uc, state) = from_data(&existing);
        assert_eq!((uc.payload(), state), (0, Utf8State::Done));
    })
    .join()
    .unwrap();
}

#[test]
fn oversize_gives_replacement() {
    let mut ud = data(b"", 2);
    ud.size = 33;
    let (uc, state) = from_data(&ud);
    assert_eq!((uc, state), (Utf8Char(0x4100_2020), Utf8State::Error));
    let back = to_data(uc);
    assert_eq!((back.bytes(), back.width), (&b" "[..], 1));
}

#[test]
fn size_31_and_32_packing() {
    std::thread::spawn(|| {
        let mut bytes = [0xa9u8; 32];
        bytes[0] = 0xc3;
        for width in 0..=2u8 {
            let (uc, state) = from_data(&data(&bytes[..31], width));
            assert_eq!(state, Utf8State::Done);
            assert_eq!(
                (uc.size(), uc.width()),
                (31, width),
                "size 31 width {width}"
            );
            assert_eq!(to_data(uc).bytes(), &bytes[..31]);

            let (uc, state) = from_data(&data(&bytes, width));
            assert_eq!(state, Utf8State::Done);
            assert_eq!(uc.size(), 0, "size 32 decodes as size 0");
            let expected = match width {
                0 => 0,
                _ => 2,
            };
            assert_eq!(uc.width(), expected, "size 32 width {width}");
            let back = to_data(uc);
            assert_eq!(back.size, 0);
            assert_eq!(back.width, expected);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn default_table_counts() {
    assert_eq!(DEFAULT_WIDTHS.len(), 162);
    assert_eq!(SKIN_TONE_BASES.len(), 72);
    assert_eq!(WHITESPACE.len(), 25);
    let mut cache = WidthCache::new();
    cache.rebuild(std::iter::empty());
    assert_eq!(cache.len(), 162);
    assert_eq!(cache.get(0x1F1E6), Some(1));
    assert_eq!(cache.get(0x1F3FB), Some(2));
    assert_eq!(cache.get(0x261D), Some(2));
    assert_eq!(cache.get(0x41), None);
}

#[test]
fn generated_tables_match_generator() {
    let tool = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/gen-utf8-tables.py"
    );
    let Ok(out) = Command::new("python3").args([tool, "-", "-"]).output() else {
        eprintln!("table regeneration skipped: python3 missing");
        return;
    };
    if !out.status.success() {
        eprintln!(
            "table regeneration skipped: pinned source unavailable\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        include_str!("tables.rs"),
        "tables.rs differs from the generator output"
    );
}

fn rebuilt(entries: &[&str]) -> WidthCache {
    rebuilt_bytes(&entries.iter().map(|s| s.as_bytes()).collect::<Vec<_>>())
}

fn rebuilt_bytes(entries: &[&[u8]]) -> WidthCache {
    let mut cache = WidthCache::new();
    cache.rebuild(entries.iter().copied());
    cache
}

#[test]
fn width_cache_regress_strings() {
    setup();
    let cache = rebuilt(&["U+1F600=2"]);
    assert_eq!(cache.get(0x1F600), Some(2));

    let cache = rebuilt(&["U+7FFFFFFF=1"]);
    assert_eq!(cache.get(0x7FFF_FFFF), Some(1));
    let cache = rebuilt(&["U+7FFFFFFE-U+7FFFFFFF=2"]);
    assert_eq!(cache.get(0x7FFF_FFFE), Some(2));
    assert_eq!(cache.get(0x7FFF_FFFF), Some(2));
    assert_eq!(cache.len(), 164);

    let cache = rebuilt(&["U+FFFFFFFF=1", "U+FFFFFFFE-U+FFFFFFFF=2", "U+80000000=1"]);
    assert_eq!(cache.len(), 162);

    let cache = rebuilt(&["U+03B1-U+03B3=2"]);
    assert_eq!(cache.get(0x3B1), Some(2));
    assert_eq!(cache.get(0x3B3), Some(2));
    assert_eq!(cache.get(0x3B4), None);
    assert_eq!(cache.len(), 165);
    with_width_cache(|c| c.rebuild(["U+03B1-U+03B3=2".as_bytes()].into_iter()));
    assert_eq!(from_cstr("αβγδ".as_bytes()).width(None), 7);
    with_width_cache(|c| c.rebuild(std::iter::empty()));
    assert_eq!(from_cstr("αβγδ".as_bytes()).width(None), 4);
}

#[test]
fn width_cache_large_range_is_fast() {
    let start = std::time::Instant::now();
    let cache = rebuilt(&["U+20000-U+50D3F=1"]);
    assert_eq!(cache.get(0x20000), Some(1));
    assert_eq!(cache.get(0x50D3F), Some(1));
    assert_eq!(
        cache.len(),
        162 + 200_000
            - DEFAULT_WIDTHS
                .iter()
                .filter(|(wc, _)| (0x20000..=0x50D3F).contains(wc))
                .count()
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn width_cache_literal_key() {
    setup();
    let cache = rebuilt(&["α=2"]);
    assert_eq!(cache.get(0x3B1), Some(2));
    let cache = rebuilt(&["αβ=2", "=2", "\u{3B1}"]);
    assert_eq!(cache.len(), 162);
    // A bad continuation reaches Done in no-width mode; libc then rejects it.
    let cache = rebuilt_bytes(&[b"\xc3A=1"]);
    assert_eq!(cache.len(), 162);
}

#[test]
fn width_cache_parser_rejects() {
    let cache = rebuilt(&[
        "U+41",
        "U+41=3",
        "U+41=-1",
        "U+41=1x",
        "U+=1",
        "U+0=1",
        "U+41x=1",
        "U+41-U+40=1",
        "U+41-U42=1",
        "U+41-U+42x=1",
        "U+FFFFFFFFFFFFFFFFFF=1",
        "U+41=",
    ]);
    assert_eq!(cache.len(), 162);
    assert_eq!(cache.get(0x41), None);
}

#[test]
fn width_cache_parser_accepts_strtoull_forms() {
    // strtoull skips leading whitespace, takes a sign and a 0x prefix.
    let cache = rebuilt(&["U+ 41=1", "U+0x42=2", "U++43=0", "U+44-U+ 45=1"]);
    assert_eq!(cache.get(0x41), Some(1));
    assert_eq!(cache.get(0x42), Some(2));
    assert_eq!(cache.get(0x43), Some(0));
    assert_eq!(cache.get(0x44), Some(1));
    assert_eq!(cache.get(0x45), Some(1));
    // Later entries replace earlier ones.
    let cache = rebuilt(&["U+1F1E6=2", "U+1F1E6=0"]);
    assert_eq!(cache.get(0x1F1E6), Some(0));
}

#[test]
fn width_of_applies_cache_then_wcwidth() {
    setup();
    assert_eq!(width_of(&data(b"a", 0)), Some(1));
    assert_eq!(width_of(&data("\u{1F1E6}".as_bytes(), 0)), Some(1));
    assert_eq!(width_of(&data("\u{1F3FB}".as_bytes(), 0)), Some(2));
    assert_eq!(width_of(&data(b"\xc2\x85", 0)), Some(0));
    assert_eq!(width_of(&data(b"\xc3\x41", 0)), None);
    assert_eq!(width_of(&data(b"\0", 0)), nul_width());
}

/// utf8proc (macOS oracle) decodes NUL as U+0000 with width 0; libc mbtowc
/// returns 0, which utf8_towc treats as an error.
fn nul_width() -> Option<u8> {
    if cfg!(target_os = "macos") {
        Some(0)
    } else {
        None
    }
}

#[test]
fn from_wc_round_trip() {
    setup();
    let ud = Utf8Data::from_wc(0x1F600).unwrap();
    assert_eq!(ud.bytes(), b"\xf0\x9f\x98\x80");
    assert_eq!((ud.have, ud.width), (4, 2));
    assert_eq!(ud.to_wc(), Some(0x1F600));
    assert_eq!(Utf8Data::from_wc(0).map(|ud| ud.width), nul_width());
    assert_eq!(Utf8Data::from_wc(0xD800), None);
    assert_eq!(Utf8Data::from_wc(0x11_0000), None);
}

#[test]
fn has_whitespace_points() {
    setup();
    for &wc in &WHITESPACE {
        let ud = Utf8Data::from_wc(wc).unwrap();
        assert!(ud.has_whitespace(), "U+{wc:04X}");
        let mut combined = Utf8Data::from_wc(0x61).unwrap();
        combined.data[1..=ud.bytes().len()].copy_from_slice(ud.bytes());
        combined.size = 1 + ud.size;
        combined.have = combined.size;
        assert!(combined.has_whitespace(), "a + U+{wc:04X}");
    }
    assert!(!Utf8Data::set(b'a').has_whitespace());
    assert!(!Utf8Data::from_wc(0x200B).unwrap().has_whitespace());
    assert!(!data(b"\xc3\x41", 1).has_whitespace());
    assert!(!data(b"\xe2\x80", 1).has_whitespace());
    assert!(!data(b"\xff\x20", 1).has_whitespace());
}

#[test]
fn combined_skin_tone_points() {
    setup();
    let tone = Utf8Data::from_wc(0x1F3FB).unwrap();
    for &wc in &SKIN_TONE_BASES {
        let base = Utf8Data::from_wc(wc).unwrap();
        assert!(should_combine(&tone, &base), "U+{wc:05X}");
        assert!(!should_combine(&base, &tone), "reverse U+{wc:05X}");
    }
    assert!(!should_combine(&tone, &Utf8Data::from_wc(0x1F600).unwrap()));
    assert!(!should_combine(
        &Utf8Data::from_wc(0x1F3FA).unwrap(),
        &Utf8Data::from_wc(0x1F44B).unwrap()
    ));
}

#[test]
fn combined_regional_indicators() {
    setup();
    let d = Utf8Data::from_wc(0x1F1E9).unwrap();
    let e = Utf8Data::from_wc(0x1F1EA).unwrap();
    assert!(should_combine(&d, &e));
    let mut de = d;
    de.data[4..8].copy_from_slice(e.bytes());
    de.size = 8;
    de.have = 8;
    assert!(
        !should_combine(&de, &d),
        "three regional indicators do not combine"
    );
    assert!(!should_combine(&d, &Utf8Data::set(b'a')));
}

#[test]
fn combined_byte_predicates() {
    use super::combined::{has_zwj, is_hangul_filler, is_vs, is_zwj};
    let zwj = data(b"\xe2\x80\x8d", 0);
    assert!(is_zwj(&zwj) && has_zwj(&zwj));
    assert!(is_vs(&data(b"\xef\xb8\x8f", 0)));
    assert!(is_hangul_filler(&data(b"\xe3\x85\xa4", 0)));
    let mut long = data(b"\xf0\x9f\x98\x80\xe2\x80\x8d", 2);
    assert!(has_zwj(&long) && !is_zwj(&long));
    long.size = 4;
    assert!(!has_zwj(&long));
    assert!(!has_zwj(&data(b"\x8d", 1)));
}

#[test]
fn hangul_jamo_boundaries() {
    let s = |b: &[u8]| data(b, 1);
    let cho = s(b"\xe1\x84\x80");
    let jung = s(b"\xe1\x85\xa1");
    let jong = s(b"\xe1\x86\xa8");
    let none = s(b"abc");
    let check = hanguljamo_check_state;
    assert_eq!(check(&none, &none), HangulJamoState::NotHangulJamo);
    assert_eq!(
        check(&none, &Utf8Data::set(b'a')),
        HangulJamoState::NotHangulJamo
    );

    // Choseong boundaries: E1 84 80-92, old E1 84 93-BF and E1 85 80-9E,
    // filler E1 85 9F, extended EA A5 A0-BC.
    for b in [
        b"\xe1\x84\x80",
        b"\xe1\x84\x92",
        b"\xe1\x84\x93",
        b"\xe1\x84\xbf",
        b"\xe1\x85\x80",
        b"\xe1\x85\x9e",
        b"\xe1\x85\x9f",
        b"\xea\xa5\xa0",
        b"\xea\xa5\xbc",
    ] {
        assert_eq!(check(&none, &s(b)), HangulJamoState::Choseong, "{b:02x?}");
    }
    assert_eq!(
        check(&none, &s(b"\xea\xa5\x9f")),
        HangulJamoState::NotHangulJamo
    );
    assert_eq!(
        check(&none, &s(b"\xea\xa5\xbd")),
        HangulJamoState::NotHangulJamo
    );
    assert_eq!(
        check(&none, &s(b"\xea\xa4\xa0")),
        HangulJamoState::NotHangulJamo
    );

    // Jungseong boundaries: filler E1 85 A0, E1 85 A1-B5, old E1 85 B6-BF and
    // E1 86 80-A7, extended ED 9E B0-BF and ED 9F 80-86.
    for b in [
        b"\xe1\x85\xa0",
        b"\xe1\x85\xa1",
        b"\xe1\x85\xb5",
        b"\xe1\x85\xb6",
        b"\xe1\x85\xbf",
        b"\xe1\x86\x80",
        b"\xe1\x86\xa7",
        b"\xed\x9e\xb0",
        b"\xed\x9e\xbf",
        b"\xed\x9f\x80",
        b"\xed\x9f\x86",
    ] {
        assert_eq!(check(&cho, &s(b)), HangulJamoState::Composable, "{b:02x?}");
        assert_eq!(
            check(&jung, &s(b)),
            HangulJamoState::NotComposable,
            "{b:02x?}"
        );
        assert_eq!(
            check(&none, &s(b)),
            HangulJamoState::NotComposable,
            "{b:02x?}"
        );
    }
    assert_eq!(
        check(&cho, &s(b"\xed\x9e\xaf")),
        HangulJamoState::NotHangulJamo
    );
    assert_eq!(
        check(&cho, &s(b"\xed\x9f\x87")),
        HangulJamoState::NotHangulJamo
    );

    // Jongseong boundaries: E1 86 A8-BF, E1 87 80-82, old E1 87 83-BF,
    // extended ED 9F 8B-BB.
    for b in [
        b"\xe1\x86\xa8",
        b"\xe1\x86\xbf",
        b"\xe1\x87\x80",
        b"\xe1\x87\x82",
        b"\xe1\x87\x83",
        b"\xe1\x87\xbf",
        b"\xed\x9f\x8b",
        b"\xed\x9f\xbb",
    ] {
        assert_eq!(check(&jung, &s(b)), HangulJamoState::Composable, "{b:02x?}");
        assert_eq!(
            check(&cho, &s(b)),
            HangulJamoState::NotComposable,
            "{b:02x?}"
        );
        assert_eq!(
            check(&jong, &s(b)),
            HangulJamoState::NotComposable,
            "{b:02x?}"
        );
    }
    assert_eq!(
        check(&jung, &s(b"\xed\x9f\x8a")),
        HangulJamoState::NotHangulJamo
    );
    assert_eq!(
        check(&jung, &s(b"\xed\x9f\xbc")),
        HangulJamoState::NotHangulJamo
    );
    assert_eq!(
        check(&jung, &s(b"\xe1\x88\x80")),
        HangulJamoState::NotHangulJamo
    );

    // Previous character: only its last three bytes matter; short prev fails.
    let mut cho_long = s(b"\xf0\x9f\x98\x80\xe1\x84\x80");
    assert_eq!(check(&cho_long, &jung), HangulJamoState::Composable);
    cho_long.size = 2;
    assert_eq!(check(&cho_long, &jung), HangulJamoState::NotComposable);
    // A two-byte sized current character is not a Jamo.
    let mut short = jung;
    short.size = 2;
    assert_eq!(check(&cho, &short), HangulJamoState::NotHangulJamo);
}

#[test]
fn string_helpers_basic() {
    setup();
    assert!(is_valid(b"abc \x7e"));
    assert!(is_valid("aé中😀".as_bytes()));
    assert!(!is_valid(b"a\x7f"));
    assert!(!is_valid(b"a\x1f"));
    assert!(!is_valid(b"a\xc3"));
    assert!(!is_valid(b"a\xc3\x41"));
    assert!(is_valid(b"ab\0\xff"), "stops at NUL");

    let mixed: &[u8] = b"a\xc3\xa9\xe4\xb8\xad\xe2\x80\x8b\x01\x7f\xff";
    assert_eq!(sanitize(mixed).as_bytes(), b"a______");
    assert_eq!(sanitize(b"ok \x7e\0zz").as_bytes(), b"ok ~");

    // cstr_width: valid widths plus printable ASCII; the C char comparison
    // decides whether 0x80-0xff bytes count.
    let high_counts = u32::from(std::ffi::c_char::MAX as i64 > 0x7f);
    assert_eq!(cstr_width(b"abc"), 3);
    assert_eq!(cstr_width(mixed), 1 + 1 + 2 + high_counts);
    assert_eq!(cstr_width(b"\x1f\x7f"), 0);
    assert_eq!(cstr_width(b"ab\0cd"), 2);

    assert_eq!(pad_right(b"ab", 4).as_bytes(), b"ab  ");
    assert_eq!(pad_right(b"ab", 2).as_bytes(), b"ab");
    assert_eq!(pad_right("中".as_bytes(), 3).as_bytes(), "中 ".as_bytes());
    assert_eq!(pad_left(b"ab", 4).as_bytes(), b"  ab");
    assert_eq!(pad_left(b"abc", 1).as_bytes(), b"abc");
    assert_eq!(pad_left(b"a\0b", 3).as_bytes(), b"  a");

    let s = from_cstr(mixed);
    assert_eq!(s.len(), 7);
    assert_eq!(s[0], Utf8Data::set(b'a'));
    assert_eq!((s[1].size, s[1].width), (2, 1));
    assert_eq!((s[2].size, s[2].width), (3, 2));
    assert_eq!((s[3].size, s[3].width), (3, 0));
    assert_eq!(s[4], Utf8Data::set(1));
    assert_eq!(s[6], Utf8Data::set(0xff));
    assert_eq!(s.to_bytes().as_bytes(), mixed);
    assert_eq!(s.width(None), 7);
    assert_eq!(s.width(Some(3)), 4);
    assert_eq!(s.width(Some(0)), 0);
    assert_eq!(s.width(Some(100)), 7);
    assert!(s.contains(&data(b"\xe4\xb8\xad", 7)), "width is ignored");
    assert!(s.contains(&Utf8Data::set(0xff)));
    assert!(!s.contains(&data(b"\xe4\xb8", 1)));
    assert!(from_cstr(b"").is_empty());

    // Invalid sequences split into single bytes, reprocessing the lead byte.
    let s = from_cstr(b"\xe2\x82A\xc3");
    assert_eq!(s.len(), 4);
    assert_eq!(s.to_bytes().as_bytes(), b"\xe2\x82A\xc3");
    assert!(s.iter().all(|ud| ud.size == 1 && ud.width == 1));
}

#[test]
fn strvis_preserves_utf8() {
    setup();
    let flags = VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL;
    let mut dst = Vec::new();
    strvis(&mut dst, b"a\t\xc3\xa9\n\x1b\xff$x\0", flags);
    assert_eq!(dst, b"a\\t\xc3\xa9\\n\\033\\377$x\\0");

    let mut dst = Vec::new();
    strvis(&mut dst, b"$a $_ ${ $1 $", flags | VisFlags::DQ);
    assert_eq!(dst, b"\\$a \\$_ \\${ $1 $");
    let mut dst = Vec::new();
    strvis(&mut dst, b"\xc3\x41\xe2\x82", VisFlags::OCTAL);
    assert_eq!(dst, b"\\303A\\342\\202");
}
