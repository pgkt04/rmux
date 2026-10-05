// Ported from tmux input-keys.c @ 8f25579c
//! Pane key encoding: the built key table, `input_key` cases taken from
//! `regress/input-keys.sh:59-314`, the `backspace` option, and
//! `input_key_get_mouse`.

use rmux_emu::input::ExtendedKeysFormat;
use rmux_emu::input::keys::{
    KEY_TABLE_LEN, KeyEncodeError, KeyPolicy, encode_key, encode_mouse, table_entries,
};
use rmux_emu::screen::ScreenMode;
use rmux_util::key::{KeyCode, KeyFlags, KeyModifiers, SpecialKey};
use rmux_util::utf8::{Utf8Data, from_data};

const SHIFT: u64 = KeyModifiers::SHIFT.0;
const META: u64 = KeyModifiers::META.0;
const CTRL: u64 = KeyModifiers::CTRL.0;
const CURSOR: u64 = KeyFlags::CURSOR.0;
const KEYPAD: u64 = KeyFlags::KEYPAD.0;
const IMPLIED_META: u64 = KeyFlags::IMPLIED_META.0;
const LITERAL: u64 = KeyFlags::LITERAL.0;

fn k(key: u64) -> KeyCode {
    KeyCode(key)
}

fn encode(mode: ScreenMode, key: u64) -> Result<Vec<u8>, (KeyEncodeError, Vec<u8>)> {
    encode_with(mode, key, &KeyPolicy::default())
}

fn encode_with(
    mode: ScreenMode,
    key: u64,
    policy: &KeyPolicy,
) -> Result<Vec<u8>, (KeyEncodeError, Vec<u8>)> {
    let mut out = Vec::new();
    match encode_key(mode, k(key), policy, &mut out) {
        Ok(()) => Ok(out),
        Err(e) => Err((e, out)),
    }
}

/// `cat -tv` notation as used by `regress/input-keys.sh`: `^X` for
/// controls, `^?` for DEL, `M-x` for bytes with the high bit.
fn cat_tv(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        match b {
            0x7f => s.push_str("^?"),
            0..=0x1f => {
                if b == b'\n' {
                    continue;
                }
                s.push('^');
                s.push(char::from(b + 0x40));
            }
            0x80..=0xff => {
                s.push_str("M-");
                let low = b & 0x7f;
                if low == 0x7f {
                    s.push_str("^?");
                } else if low < 0x20 {
                    s.push('^');
                    s.push(char::from(low + 0x40));
                } else {
                    s.push(char::from(low));
                }
            }
            _ => s.push(char::from(b)),
        }
    }
    s
}

fn vis(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        match b {
            0x1b => s.push_str("\\033"),
            b'\n' => s.push_str("\\n"),
            0x20..=0x7e => s.push(char::from(b)),
            _ => s.push_str(&format!("\\{b:03o}")),
        }
    }
    s
}

/// `input_key_build` log line (`input-keys.c:390-393`) without the
/// `key_string_lookup_key` name, which has no port yet.
fn dump_line(key: KeyCode, bytes: &[u8]) -> String {
    format!("input_key_build: 0x{:x} is {}", key.0, vis(bytes))
}

fn utf8_key(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut ud = Utf8Data {
        size: b.len() as u8,
        have: b.len() as u8,
        width: 1,
        ..Utf8Data::default()
    };
    ud.data[..b.len()].copy_from_slice(b);
    let (uc, _) = from_data(&ud);
    u64::from(uc.0)
}

#[test]
fn table_has_217_sorted_unique_entries() {
    assert_eq!(KEY_TABLE_LEN, 217);
    let entries = table_entries();
    assert_eq!(entries.len(), 217);
    for pair in entries.windows(2) {
        assert!(
            pair[0].0.0 < pair[1].0.0,
            "{:x} !< {:x}",
            pair[0].0.0,
            pair[1].0.0
        );
    }
    for (_, bytes) in entries {
        assert!(!bytes.is_empty() && bytes.len() <= 7);
        assert!(!bytes.contains(&b'_'));
    }
}

#[test]
fn table_dump_matches_fixture() {
    let dump: Vec<String> = table_entries()
        .iter()
        .map(|(key, bytes)| dump_line(*key, bytes))
        .collect();
    let expected = [
        "input_key_build: 0x200000008 is \\033OP",
        "input_key_build: 0x20000001b is \\033[A",
        "input_key_build: 0x400020000001b is \\033OA",
        "input_key_build: 0x20000002c is \\n",
        "input_key_build: 0x200020000002c is \\033OM",
        "input_key_build: 0x20020000001b is \\033[1;5A",
        "input_key_build: 0x810020000001b is \\033[1;3A",
        "input_key_build: 0x870020000000c is \\033[15;8~",
        "input_key_build: 0x40020000000d is \\033[17;2~",
        "input_key_build: 0x1000020000000a is \\033[1;_R",
    ];
    for line in &expected[..9] {
        assert!(dump.contains(&line.to_string()), "missing {line}");
    }
    assert!(!dump.contains(&expected[9].to_string()));
    assert_eq!(dump[0], "input_key_build: 0x200000005 is \\033[200~");
    assert_eq!(dump[1], "input_key_build: 0x200000006 is \\033[201~");
    assert_eq!(
        dump.last().unwrap(),
        "input_key_build: 0x870020000001e is \\033[1;8C"
    );
}

#[test]
fn regress_vt10x_c0_and_meta() {
    let m = ScreenMode::default();
    let cases: &[(u64, &str)] = &[
        (u64::from(b' ') | CTRL, "^@"),
        (u64::from(b'a') | CTRL, "^A"),
        (u64::from(b'a') | CTRL | META, "^[^A"),
        (u64::from(b'h') | CTRL, "^H"),
        (u64::from(b'i') | CTRL, "^I"),
        (u64::from(b'j') | CTRL, ""),
        (u64::from(b'j') | CTRL | META, "^["),
        (u64::from(b'm') | CTRL, "^M"),
        (u64::from(b'z') | CTRL | META, "^[^Z"),
        (0x1b, "^["),
        (0x1b | META, "^[^["),
        (u64::from(b'\\') | CTRL, "^\\"),
        (u64::from(b'\\') | CTRL | META, "^[^\\"),
        (u64::from(b']') | CTRL, "^]"),
        (u64::from(b'^') | CTRL, "^^"),
        (u64::from(b'_') | CTRL | META, "^[^_"),
        (u64::from(b' '), " "),
        (u64::from(b' ') | META, "^[ "),
        (u64::from(b'!'), "!"),
        (u64::from(b'!') | META, "^[!"),
        (u64::from(b';'), ";"),
        (u64::from(b'@') | META, "^[@"),
        (u64::from(b'A'), "A"),
        (u64::from(b'Z') | META, "^[Z"),
        (u64::from(b'`') | META, "^[`"),
        (u64::from(b'a'), "a"),
        (u64::from(b'a') | META, "^[a"),
        (u64::from(b'~') | META, "^[~"),
        (0x09, "^I"),
        (0x09 | META, "^[^I"),
        (0x0d, "^M"),
        (SpecialKey::BSPACE, "^?"),
        (SpecialKey::BSPACE | META, "^[^?"),
        (u64::from(b'2') | CTRL, "^@"),
        (u64::from(b'3') | CTRL, "^["),
        (u64::from(b'7') | CTRL, "^_"),
        (u64::from(b'8') | CTRL, "^?"),
        (u64::from(b'/') | CTRL, "^_"),
        (u64::from(b'1') | CTRL, "1"),
        (u64::from(b'=') | CTRL, "="),
    ];
    for &(key, expected) in cases {
        let out = encode(m, key).unwrap_or_else(|e| panic!("{key:x}: {e:?}"));
        assert_eq!(cat_tv(&out), expected, "key {key:x}");
    }
}

#[test]
fn regress_function_and_edit_keys() {
    let m = ScreenMode::default();
    let cases: &[(u64, &str)] = &[
        (SpecialKey::F1, "^[OP"),
        (SpecialKey::F2, "^[OQ"),
        (SpecialKey::F3, "^[OR"),
        (SpecialKey::F4, "^[OS"),
        (SpecialKey::F5, "^[[15~"),
        (SpecialKey::F6, "^[[17~"),
        (SpecialKey::F8, "^[[19~"),
        (SpecialKey::F9, "^[[20~"),
        (SpecialKey::F10, "^[[21~"),
        (SpecialKey::F11, "^[[23~"),
        (SpecialKey::F12, "^[[24~"),
        (SpecialKey::IC, "^[[2~"),
        (SpecialKey::DC, "^[[3~"),
        (SpecialKey::HOME, "^[[1~"),
        (SpecialKey::END, "^[[4~"),
        (SpecialKey::NPAGE, "^[[6~"),
        (SpecialKey::PPAGE, "^[[5~"),
        (SpecialKey::BTAB, "^[[Z"),
        (SpecialKey::BTAB | CTRL, "^[[Z"),
        // `C-S-Tab` (input-keys.sh:223) is Tab with Ctrl and Shift; vt10x
        // clears Ctrl for Tab.
        (0x09 | CTRL | SHIFT, "^I"),
        (SpecialKey::UP, "^[[A"),
        (SpecialKey::DOWN, "^[[B"),
        (SpecialKey::RIGHT, "^[[C"),
        (SpecialKey::LEFT, "^[[D"),
        (SpecialKey::KP_STAR, "*"),
        (SpecialKey::KP_STAR | META, "^[*"),
        (SpecialKey::KP_PLUS | META, "^[+"),
        (SpecialKey::KP_PERIOD, "."),
        (SpecialKey::KP_SLASH | META, "^[/"),
        (SpecialKey::KP_ZERO, "0"),
        (SpecialKey::KP_NINE | META, "^[9"),
        (SpecialKey::F1 | META, "^[^[OP"),
    ];
    for &(key, expected) in cases {
        let out = encode(m, key).unwrap_or_else(|e| panic!("{key:x}: {e:?}"));
        assert_eq!(cat_tv(&out), expected, "key {key:x}");
    }
}

/// `C-S-Tab` in the regress file is `BTab|Ctrl` after `tty-keys`
/// (`regress/input-keys.sh:223`); without extended mode 2 the modifiers
/// drop and the plain table entry is used.
#[test]
fn btab_extended_2_vs_not() {
    let base = ScreenMode::default();
    assert_eq!(
        encode(base, SpecialKey::BTAB | CTRL | META).unwrap(),
        b"\x1b[Z"
    );
    let m2 = ScreenMode::KEYS_EXTENDED_2;
    assert_eq!(encode(m2, SpecialKey::BTAB).unwrap(), b"\x1b[27;2;9~");
    assert_eq!(
        encode(m2, SpecialKey::BTAB | CTRL).unwrap(),
        b"\x1b[27;6;9~"
    );
    let csiu = KeyPolicy {
        format: ExtendedKeysFormat::CsiU,
        ..KeyPolicy::default()
    };
    assert_eq!(
        encode_with(m2, SpecialKey::BTAB, &csiu).unwrap(),
        b"\x1b[9;2u"
    );
}

#[test]
fn regress_modified_function_keys_in_table_without_extended_mode() {
    // input_key_build expands the templates, so these work in every mode
    // ("Many of these pass without extended keys enabled", input-keys.sh:276).
    let m = ScreenMode::default();
    let mods: [(u64, char); 7] = [
        (SHIFT, '2'),
        (META, '3'),
        (SHIFT | META, '4'),
        (CTRL, '5'),
        (SHIFT | CTRL, '6'),
        (CTRL | META, '7'),
        (SHIFT | CTRL | META, '8'),
    ];
    let keys: &[(u64, &str)] = &[
        (SpecialKey::F1, "^[[1;_P"),
        (SpecialKey::F4, "^[[1;_S"),
        (SpecialKey::F5, "^[[15;_~"),
        (SpecialKey::F12, "^[[24;_~"),
        (SpecialKey::UP, "^[[1;_A"),
        (SpecialKey::LEFT, "^[[1;_D"),
        (SpecialKey::HOME, "^[[1;_H"),
        (SpecialKey::END, "^[[1;_F"),
        (SpecialKey::PPAGE, "^[[5;_~"),
        (SpecialKey::NPAGE, "^[[6;_~"),
        (SpecialKey::IC, "^[[2;_~"),
        (SpecialKey::DC, "^[[3;_~"),
    ];
    for &(key, pattern) in keys {
        for &(modifier, digit) in &mods {
            let expected = pattern.replace('_', &digit.to_string());
            // key-string.c:36-59,318-319 gives these names KEYC_IMPLIED_META
            // only together with Meta, so the regress Meta keys hit the
            // template entry directly.
            let implied = if modifier & META != 0 {
                IMPLIED_META
            } else {
                0
            };
            let out = encode(m, key | modifier | implied).unwrap();
            assert_eq!(cat_tv(&out), expected, "key {key:x} mod {modifier:x}");
            if modifier & META != 0 {
                // Meta without implied Meta: ESC plus the entry without Meta
                // (input-keys.c:658-659,669-670).
                let out = encode(m, key | modifier).unwrap();
                let rest = encode(m, key | (modifier & !META)).unwrap();
                assert_eq!(
                    out,
                    [&[0x1b][..], &rest].concat(),
                    "key {key:x} mod {modifier:x}"
                );
            }
        }
    }
    // Table keys that lack a modifier template (F1 has one; KP_ENTER does
    // not) fall through to the lookup without Meta plus an ESC prefix.
    assert_eq!(encode(m, SpecialKey::KP_ENTER | META).unwrap(), b"\x1b\n");
    assert_eq!(
        encode(m, SpecialKey::F7 | CTRL | SHIFT | META).unwrap(),
        b"\x1b\x1b[18;6~"
    );
    assert_eq!(encode(m, SpecialKey::F7 | META).unwrap(), b"\x1b\x1b[18~");
    // Special keys with a modifier and no entry are ignored.
    assert_eq!(encode(m, SpecialKey::BTAB | SHIFT).unwrap(), b"\x1b[Z");
    assert_eq!(encode(m, SpecialKey::KP_ENTER | CTRL).unwrap(), b"");
    assert_eq!(encode(m, SpecialKey::FOCUS_IN).unwrap(), b"");
    assert_eq!(encode(m, KeyCode::user(3).unwrap().0).unwrap(), b"");
}

#[test]
fn regress_extended_tab_mode_2_and_mode_1() {
    let m2 = ScreenMode::KEYS_EXTENDED_2;
    assert_eq!(encode(m2, 0x09 | CTRL).unwrap(), b"\x1b[27;5;9~");
    assert_eq!(encode(m2, 0x09 | CTRL | SHIFT).unwrap(), b"\x1b[27;6;9~");
    let csiu = KeyPolicy {
        format: ExtendedKeysFormat::CsiU,
        ..KeyPolicy::default()
    };
    assert_eq!(encode_with(m2, 0x09 | CTRL, &csiu).unwrap(), b"\x1b[9;5u");
    assert_eq!(
        encode_with(m2, 0x09 | CTRL | SHIFT, &csiu).unwrap(),
        b"\x1b[9;6u"
    );

    // Mode 2: every modified key takes the extended form.
    assert_eq!(
        encode(m2, u64::from(b'a') | CTRL).unwrap(),
        b"\x1b[27;5;97~"
    );
    assert_eq!(
        encode(m2, u64::from(b'a') | META).unwrap(),
        b"\x1b[27;3;97~"
    );
    assert_eq!(
        encode(m2, u64::from(b'A') | SHIFT | META).unwrap(),
        b"\x1b[27;4;65~"
    );
    assert_eq!(
        encode(m2, u64::from(b'1') | CTRL).unwrap(),
        b"\x1b[27;5;49~"
    );
    assert_eq!(
        encode_with(m2, u64::from(b'a') | CTRL, &csiu).unwrap(),
        b"\x1b[97;5u"
    );
    assert_eq!(
        encode_with(m2, u64::from(b'a') | META, &csiu).unwrap(),
        b"\x1b[97;3u"
    );
    // Unmodified keys stay plain.
    assert_eq!(encode(m2, u64::from(b'a')).unwrap(), b"a");
    assert_eq!(encode(m2, 0x0d).unwrap(), b"\r");
    // A plain C0 byte with no modifiers reaches the extended encoder and
    // fails with no modifier (input-keys.c:454-455).
    assert_eq!(
        encode(m2, 0x01),
        Err((KeyEncodeError::NoModifier, Vec::new()))
    );

    // Mode 1: Meta-only and the xterm Ctrl set stay vt10x.
    let m1 = ScreenMode::KEYS_EXTENDED;
    assert_eq!(encode(m1, u64::from(b'a') | META).unwrap(), b"\x1ba");
    assert_eq!(
        encode(m1, u64::from(b'A') | SHIFT | META).unwrap(),
        b"\x1bA"
    );
    assert_eq!(encode(m1, u64::from(b'a') | CTRL).unwrap(), b"\x01");
    assert_eq!(encode(m1, u64::from(b' ') | CTRL).unwrap(), b"\x00");
    assert_eq!(encode(m1, u64::from(b'/') | CTRL).unwrap(), b"\x1f");
    assert_eq!(encode(m1, u64::from(b'@') | CTRL).unwrap(), b"\x00");
    assert_eq!(encode(m1, u64::from(b'^') | CTRL).unwrap(), b"\x1e");
    assert_eq!(encode(m1, u64::from(b'2') | CTRL).unwrap(), b"\x00");
    assert_eq!(encode(m1, u64::from(b'8') | CTRL).unwrap(), b"\x7f");
    assert_eq!(
        encode(m1, u64::from(b'a') | CTRL | META).unwrap(),
        b"\x1b\x01"
    );
    // Outside that set the extended form is used.
    assert_eq!(
        encode(m1, u64::from(b'1') | CTRL).unwrap(),
        b"\x1b[27;5;49~"
    );
    assert_eq!(encode(m1, u64::from(b'A') | SHIFT | CTRL).unwrap(), b"\x01");
    assert_eq!(
        encode(m1, u64::from(b'-') | CTRL).unwrap(),
        b"\x1b[27;5;45~"
    );
    assert_eq!(encode(m1, 0x09 | CTRL).unwrap(), b"\x1b[27;5;9~");
    assert_eq!(
        encode(m1, u64::from(b'a') | SHIFT).unwrap(),
        b"\x1b[27;2;97~"
    );
}

#[test]
fn extended_unicode_key() {
    let e_acute = utf8_key("é");
    let m2 = ScreenMode::KEYS_EXTENDED_2;
    assert_eq!(encode(m2, e_acute).unwrap(), "é".as_bytes());
    assert_eq!(encode(m2, e_acute | CTRL).unwrap(), b"\x1b[27;5;233~");
    let csiu = KeyPolicy {
        format: ExtendedKeysFormat::CsiU,
        ..KeyPolicy::default()
    };
    assert_eq!(
        encode_with(m2, e_acute | META, &csiu).unwrap(),
        b"\x1b[233;3u"
    );
    // vt10x drops Unicode modifiers after the Meta prefix.
    let base = ScreenMode::default();
    assert_eq!(encode(base, e_acute | META).unwrap(), b"\x1b\xc3\xa9");
    assert_eq!(encode(base, e_acute | CTRL).unwrap(), b"\xc3\xa9");
}

#[test]
fn vt10x_unmappable_ctrl_retains_meta_prefix() {
    let m = ScreenMode::default();
    assert_eq!(
        encode(m, u64::from(b'#') | CTRL | META),
        Err((KeyEncodeError::Unmappable, vec![0x1b]))
    );
    assert_eq!(
        encode(m, u64::from(b'#') | CTRL),
        Err((KeyEncodeError::Unmappable, Vec::new()))
    );
    assert_eq!(encode(m, u64::from(b'9') | CTRL).unwrap(), b"9");
    assert_eq!(encode(m, u64::from(b'(') | CTRL).unwrap(), b"9");
    assert_eq!(encode(m, u64::from(b'\'') | CTRL).unwrap(), b"'");
    assert_eq!(encode(m, u64::from(b'"') | CTRL).unwrap(), b"'");
    assert_eq!(encode(m, u64::from(b'-') | CTRL).unwrap(), b"\x1f");
    assert_eq!(encode(m, u64::from(b'?') | CTRL).unwrap(), b"\x7f");
    assert_eq!(encode(m, CTRL).unwrap(), b"\x00");
}

#[test]
fn literal_keys_pass_the_low_byte() {
    let m = ScreenMode::default();
    assert_eq!(encode(m, 0x01 | LITERAL).unwrap(), b"\x01");
    assert_eq!(encode(m, 0x1ff | LITERAL | CTRL).unwrap(), b"\xff");
    assert_eq!(encode(m, 0x01).unwrap(), b"\x01");
}

#[test]
fn paste_keys_need_bracketpaste() {
    let off = ScreenMode::default();
    let on = ScreenMode::BRACKETPASTE;
    assert_eq!(encode(off, SpecialKey::PASTE_START).unwrap(), b"");
    assert_eq!(encode(off, SpecialKey::PASTE_END).unwrap(), b"");
    assert_eq!(encode(on, SpecialKey::PASTE_START).unwrap(), b"\x1b[200~");
    assert_eq!(encode(on, SpecialKey::PASTE_END).unwrap(), b"\x1b[201~");
    assert_eq!(
        encode(on, SpecialKey::PASTE_START | IMPLIED_META).unwrap(),
        b"\x1b[200~"
    );
    assert_eq!(
        encode(on, SpecialKey::PASTE_END | META).unwrap(),
        b"\x1b\x1b[201~"
    );
    assert_eq!(encode(off, SpecialKey::PASTE_END | META).unwrap(), b"");
}

#[test]
fn cursor_and_keypad_flags_follow_screen_modes() {
    let base = ScreenMode::default();
    assert_eq!(encode(base, SpecialKey::UP | CURSOR).unwrap(), b"\x1b[A");
    assert_eq!(
        encode(ScreenMode::KCURSOR, SpecialKey::UP | CURSOR).unwrap(),
        b"\x1bOA"
    );
    assert_eq!(
        encode(ScreenMode::KCURSOR, SpecialKey::UP).unwrap(),
        b"\x1b[A"
    );
    // A cursor flag with a modifier has no entry; the lookup retries without
    // the flag and finds the template expansion.
    assert_eq!(
        encode(ScreenMode::KCURSOR, SpecialKey::UP | CURSOR | CTRL).unwrap(),
        b"\x1b[1;5A"
    );
    assert_eq!(
        encode(ScreenMode::KCURSOR, SpecialKey::UP | CURSOR | META).unwrap(),
        b"\x1b\x1bOA"
    );
    assert_eq!(
        encode(
            ScreenMode::KCURSOR,
            SpecialKey::UP | CURSOR | META | IMPLIED_META
        )
        .unwrap(),
        b"\x1b[1;3A"
    );

    assert_eq!(encode(base, SpecialKey::KP_ENTER | KEYPAD).unwrap(), b"\n");
    assert_eq!(
        encode(ScreenMode::KKEYPAD, SpecialKey::KP_ENTER | KEYPAD).unwrap(),
        b"\x1bOM"
    );
    assert_eq!(
        encode(ScreenMode::KKEYPAD, SpecialKey::KP_ENTER).unwrap(),
        b"\n"
    );
    assert_eq!(
        encode(ScreenMode::KKEYPAD, SpecialKey::KP_FIVE | KEYPAD).unwrap(),
        b"\x1bOu"
    );
    assert_eq!(
        encode(ScreenMode::KKEYPAD, SpecialKey::KP_FIVE | KEYPAD | META).unwrap(),
        b"\x1b\x1bOu"
    );
    assert_eq!(
        encode(ScreenMode::KKEYPAD, SpecialKey::UP | CURSOR).unwrap(),
        b"\x1b[A"
    );
}

#[test]
fn backspace_option_cases() {
    let m = ScreenMode::default();
    // Default C-? (0x7f).
    assert_eq!(encode(m, SpecialKey::BSPACE).unwrap(), b"\x7f");
    // Option C-h as a plain byte.
    let ctrl_h_byte = KeyPolicy {
        backspace: k(0x08),
        ..KeyPolicy::default()
    };
    assert_eq!(
        encode_with(m, SpecialKey::BSPACE, &ctrl_h_byte).unwrap(),
        b"\x08"
    );
    // Byte 255 is the no-output sentinel.
    let none = KeyPolicy {
        backspace: k(255),
        ..KeyPolicy::default()
    };
    assert_eq!(encode_with(m, SpecialKey::BSPACE, &none).unwrap(), b"");
    // Ctrl + option key.
    for (opt, expected) in [
        (b'h', 0x08u8),
        (b'?', 0x7f),
        (b'@', 0x00),
        (b'_', 0x1f),
        (b'z', 0x1a),
        (b'A', 0x01),
    ] {
        let p = KeyPolicy {
            backspace: k(u64::from(opt) | CTRL),
            ..KeyPolicy::default()
        };
        assert_eq!(
            encode_with(m, SpecialKey::BSPACE, &p).unwrap(),
            [expected],
            "C-{}",
            opt as char
        );
    }
    // Ctrl + key outside the mapped ranges, or any other modifier: nothing.
    let ctrl_one = KeyPolicy {
        backspace: k(u64::from(b'1') | CTRL),
        ..KeyPolicy::default()
    };
    assert_eq!(encode_with(m, SpecialKey::BSPACE, &ctrl_one).unwrap(), b"");
    let meta_h = KeyPolicy {
        backspace: k(u64::from(b'h') | META),
        ..KeyPolicy::default()
    };
    assert_eq!(encode_with(m, SpecialKey::BSPACE, &meta_h).unwrap(), b"");
    // Incoming modifiers: option key OR flags/modifiers, then normal lookup.
    assert_eq!(encode(m, SpecialKey::BSPACE | META).unwrap(), b"\x1b\x7f");
    assert_eq!(
        encode_with(m, SpecialKey::BSPACE | META, &ctrl_h_byte).unwrap(),
        b"\x1b\x08"
    );
    // 0x08 with Ctrl has no vt10x mapping, so like C this fails after
    // writing nothing.
    assert_eq!(
        encode_with(m, SpecialKey::BSPACE | CTRL, &ctrl_h_byte),
        Err((KeyEncodeError::Unmappable, Vec::new()))
    );
    let ctrl_h = KeyPolicy {
        backspace: k(u64::from(b'h') | CTRL),
        ..KeyPolicy::default()
    };
    assert_eq!(
        encode_with(m, SpecialKey::BSPACE | META, &ctrl_h).unwrap(),
        b"\x1b\x08"
    );
    let m2 = ScreenMode::KEYS_EXTENDED_2;
    assert_eq!(
        encode(m2, SpecialKey::BSPACE | CTRL).unwrap(),
        b"\x1b[27;5;127~"
    );
    assert_eq!(
        encode_with(m2, SpecialKey::BSPACE | META, &ctrl_h_byte).unwrap(),
        b"\x1b[27;3;8~"
    );
}

#[test]
fn mouse_drag_and_release_filters() {
    let std = ScreenMode::MOUSE_STANDARD;
    let btn = ScreenMode::MOUSE_BUTTON;
    let all = ScreenMode::MOUSE_ALL;
    // No mouse mode: nothing.
    assert_eq!(
        encode_mouse(ScreenMode::default(), 0, 3, 0, b' ', 0, 0),
        None
    );
    // Drag needs button or all mode.
    assert_eq!(encode_mouse(std, 32, 0, 0, b' ', 0, 0), None);
    assert!(encode_mouse(btn, 32, 0, 0, b' ', 0, 0).is_some());
    assert!(encode_mouse(all, 32, 0, 0, b' ', 0, 0).is_some());
    // Legacy release after release in drag: only in all mode.
    assert_eq!(encode_mouse(btn, 32 | 3, 3, 0, b' ', 0, 0), None);
    assert!(encode_mouse(all, 32 | 3, 3, 0, b' ', 0, 0).is_some());
    assert!(encode_mouse(btn, 32 | 3, 0, 0, b' ', 0, 0).is_some());
    // SGR release in drag: the sgr_b decides.
    assert_eq!(encode_mouse(btn, 32, 0, 32 | 3, b'm', 0, 0), None);
    assert!(encode_mouse(all, 32, 0, 32 | 3, b'm', 0, 0).is_some());
    assert!(encode_mouse(btn, 32, 0, 32, b'M', 0, 0).is_some());
    // Non-drag release passes.
    assert!(encode_mouse(std, 3, 0, 0, b' ', 0, 0).is_some());
}

#[test]
fn mouse_output_formats() {
    let sgr = ScreenMode::MOUSE_STANDARD | ScreenMode::MOUSE_SGR;
    let utf8 = ScreenMode::MOUSE_STANDARD | ScreenMode::MOUSE_UTF8;
    let legacy = ScreenMode::MOUSE_STANDARD;

    // SGR: requested and sent in SGR form.
    let out = encode_mouse(sgr, 0, 0, 0, b'M', 4, 9).unwrap();
    assert_eq!(&*out, b"\x1b[<0;5;10M");
    let out = encode_mouse(sgr, 3, 0, 0, b'm', 300, 1000).unwrap();
    assert_eq!(&*out, b"\x1b[<0;301;1001m");
    let out = encode_mouse(sgr, 64, 0, 64, b'M', 0, 0).unwrap();
    assert_eq!(&*out, b"\x1b[<64;1;1M");
    // Mismatch: SGR mode without an SGR event falls back to legacy; an SGR
    // event without SGR mode falls back too.
    let out = encode_mouse(sgr, 0, 0, 0, b' ', 4, 9).unwrap();
    assert_eq!(&*out, b"\x1b[M\x20\x25\x2a");
    let out = encode_mouse(legacy, 0, 0, 0, b'M', 4, 9).unwrap();
    assert_eq!(&*out, b"\x1b[M\x20\x25\x2a");
    let out = encode_mouse(utf8, 0, 0, 0, b'M', 4, 9).unwrap();
    assert_eq!(&*out, b"\x1b[M\x20\x25\x2a");

    // UTF-8: two-byte split above 0x7f, bound 0x7ff.
    let out = encode_mouse(utf8, 0, 0, 0, b' ', 0x5e, 0x5f).unwrap();
    assert_eq!(&*out, b"\x1b[M\x20\x7f\xc2\x80");
    let out = encode_mouse(utf8, 0, 0, 0, b' ', 0x7ff - 0x21, 0).unwrap();
    assert_eq!(&*out, b"\x1b[M\x20\xdf\xbf\x21");
    assert_eq!(encode_mouse(utf8, 0, 0, 0, b' ', 0x7ff - 0x20, 0), None);
    assert_eq!(encode_mouse(utf8, 0, 0, 0, b' ', 0, 0x7ff - 0x20), None);
    assert_eq!(encode_mouse(utf8, 0x7ff - 0x1f, 0, 0, b' ', 0, 0), None);
    let out = encode_mouse(utf8, 0x7ff - 0x20, 0, 0, b' ', 0, 0).unwrap();
    assert_eq!(&*out, b"\x1b[M\xdf\xbf\x21\x21");

    // Legacy: button beyond 0xff dropped, x/y clamped to 0xff.
    let out = encode_mouse(legacy, 0xdf, 0, 0, b' ', 0xde, 0xdf).unwrap();
    assert_eq!(&*out, b"\x1b[M\xff\xff\xff");
    assert_eq!(encode_mouse(legacy, 0xe0, 0, 0, b' ', 0, 0), None);
    let out = encode_mouse(legacy, 0, 0, 0, b' ', 1000, 0).unwrap();
    assert_eq!(&*out, b"\x1b[M\x20\xff\x21");
    let out = encode_mouse(legacy, 0, 0, 0, b' ', 0, 1000).unwrap();
    assert_eq!(&*out, b"\x1b[M\x20\x21\xff");
}

#[test]
fn mouse_keys_encode_nothing() {
    let m = ScreenMode::default();
    assert_eq!(encode(m, SpecialKey::MOUSEMOVE_PANE).unwrap(), b"");
    assert_eq!(encode(m, SpecialKey::MOUSE).unwrap(), b"");
    assert_eq!(encode(m, SpecialKey::MOUSE | META).unwrap(), b"");
}

#[test]
fn mouse_offsets_wrap_as_unsigned_c() {
    let legacy = ScreenMode::MOUSE_ALL;
    assert_eq!(
        encode_mouse(legacy, u32::MAX, 0, 0, b' ', u32::MAX, u32::MAX)
            .unwrap()
            .as_ref(),
        b"\x1b[M\x1f\x20\x20"
    );
    let sgr = legacy | ScreenMode::MOUSE_SGR;
    assert_eq!(
        encode_mouse(sgr, 0, 0, u32::MAX, b'M', u32::MAX, u32::MAX)
            .unwrap()
            .as_ref(),
        b"\x1b[<4294967295;0;0M"
    );
    let both = ScreenMode::KEYS_EXTENDED | ScreenMode::KEYS_EXTENDED_2;
    assert_eq!(encode(both, u64::from(b'a') | CTRL).unwrap(), b"\x01");
}
