// Ported from tmux key-string.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

//! Key names: `key_string_lookup_string` (`parse_key_name`) and
//! `key_string_lookup_key` (`write_key_name`).

use rmux_util::bytes::cstr;
use rmux_util::key::{
    C0, KeyCode, KeyCodeType, KeyFlags, KeyMasks, KeyModifiers, MouseLocation, SpecialKey as K,
};
use rmux_util::utf8::{self, Utf8Data, Utf8State};

const META: u64 = KeyModifiers::META.0;
const CTRL: u64 = KeyModifiers::CTRL.0;
const SHIFT: u64 = KeyModifiers::SHIFT.0;
const KEYPAD: u64 = KeyFlags::KEYPAD.0;
const CURSOR: u64 = KeyFlags::CURSOR.0;
const IMPLIED_META: u64 = KeyFlags::IMPLIED_META.0;

/// Size of the C output buffer (`key-string.c:330`).
pub const NAME_SIZE: usize = 64;

/// The named part of `key_string_table` (`key-string.c:34-124`).
const NAMES: &[(&str, u64)] = &[
    ("F1", K::F1 | IMPLIED_META),
    ("F2", K::F2 | IMPLIED_META),
    ("F3", K::F3 | IMPLIED_META),
    ("F4", K::F4 | IMPLIED_META),
    ("F5", K::F5 | IMPLIED_META),
    ("F6", K::F6 | IMPLIED_META),
    ("F7", K::F7 | IMPLIED_META),
    ("F8", K::F8 | IMPLIED_META),
    ("F9", K::F9 | IMPLIED_META),
    ("F10", K::F10 | IMPLIED_META),
    ("F11", K::F11 | IMPLIED_META),
    ("F12", K::F12 | IMPLIED_META),
    ("IC", K::IC | IMPLIED_META),
    ("Insert", K::IC | IMPLIED_META),
    ("DC", K::DC | IMPLIED_META),
    ("Delete", K::DC | IMPLIED_META),
    ("Home", K::HOME | IMPLIED_META),
    ("End", K::END | IMPLIED_META),
    ("NPage", K::NPAGE | IMPLIED_META),
    ("PageDown", K::NPAGE | IMPLIED_META),
    ("PgDn", K::NPAGE | IMPLIED_META),
    ("PPage", K::PPAGE | IMPLIED_META),
    ("PageUp", K::PPAGE | IMPLIED_META),
    ("PgUp", K::PPAGE | IMPLIED_META),
    ("BTab", K::BTAB),
    ("Space", b' ' as u64),
    ("BSpace", K::BSPACE),
    ("[NUL]", C0::NUL as u64),
    ("[SOH]", C0::SOH as u64),
    ("[STX]", C0::STX as u64),
    ("[ETX]", C0::ETX as u64),
    ("[EOT]", C0::EOT as u64),
    ("[ENQ]", C0::ENQ as u64),
    ("[ASC]", C0::ASC as u64),
    ("[BEL]", C0::BEL as u64),
    ("[BS]", C0::BS as u64),
    ("Tab", C0::HT as u64),
    ("[LF]", C0::LF as u64),
    ("[VT]", C0::VT as u64),
    ("[FF]", C0::FF as u64),
    ("Enter", C0::CR as u64),
    ("[SO]", C0::SO as u64),
    ("[SI]", C0::SI as u64),
    ("[DLE]", C0::DLE as u64),
    ("[DC1]", C0::DC1 as u64),
    ("[DC2]", C0::DC2 as u64),
    ("[DC3]", C0::DC3 as u64),
    ("[DC4]", C0::DC4 as u64),
    ("[NAK]", C0::NAK as u64),
    ("[SYN]", C0::SYN as u64),
    ("[ETB]", C0::ETB as u64),
    ("[CAN]", C0::CAN as u64),
    ("[EM]", C0::EM as u64),
    ("[SUB]", C0::SUB as u64),
    ("Escape", C0::ESC as u64),
    ("[FS]", C0::FS as u64),
    ("[GS]", C0::GS as u64),
    ("[RS]", C0::RS as u64),
    ("[US]", C0::US as u64),
    ("Up", K::UP | CURSOR | IMPLIED_META),
    ("Down", K::DOWN | CURSOR | IMPLIED_META),
    ("Left", K::LEFT | CURSOR | IMPLIED_META),
    ("Right", K::RIGHT | CURSOR | IMPLIED_META),
    ("KP/", K::KP_SLASH | KEYPAD),
    ("KP*", K::KP_STAR | KEYPAD),
    ("KP-", K::KP_MINUS | KEYPAD),
    ("KP7", K::KP_SEVEN | KEYPAD),
    ("KP8", K::KP_EIGHT | KEYPAD),
    ("KP9", K::KP_NINE | KEYPAD),
    ("KP+", K::KP_PLUS | KEYPAD),
    ("KP4", K::KP_FOUR | KEYPAD),
    ("KP5", K::KP_FIVE | KEYPAD),
    ("KP6", K::KP_SIX | KEYPAD),
    ("KP1", K::KP_ONE | KEYPAD),
    ("KP2", K::KP_TWO | KEYPAD),
    ("KP3", K::KP_THREE | KEYPAD),
    ("KPEnter", K::KP_ENTER | KEYPAD),
    ("KP0", K::KP_ZERO | KEYPAD),
    ("KP.", K::KP_PERIOD | KEYPAD),
];

/// The mouse part of `key_string_table` (`key-string.c:126-191`): each
/// `KEYC_MOUSE_STRING(name, s)` row expands to one entry per location.
const MOUSE_NAMES: &[(&str, KeyCodeType, u32)] = &[
    ("MouseDown1", KeyCodeType::Mousedown, 1),
    ("MouseDown2", KeyCodeType::Mousedown, 2),
    ("MouseDown3", KeyCodeType::Mousedown, 3),
    ("MouseDown6", KeyCodeType::Mousedown, 6),
    ("MouseDown7", KeyCodeType::Mousedown, 7),
    ("MouseDown8", KeyCodeType::Mousedown, 8),
    ("MouseDown9", KeyCodeType::Mousedown, 9),
    ("MouseDown10", KeyCodeType::Mousedown, 10),
    ("MouseDown11", KeyCodeType::Mousedown, 11),
    ("MouseUp1", KeyCodeType::Mouseup, 1),
    ("MouseUp2", KeyCodeType::Mouseup, 2),
    ("MouseUp3", KeyCodeType::Mouseup, 3),
    ("MouseUp6", KeyCodeType::Mouseup, 6),
    ("MouseUp7", KeyCodeType::Mouseup, 7),
    ("MouseUp8", KeyCodeType::Mouseup, 8),
    ("MouseUp9", KeyCodeType::Mouseup, 9),
    ("MouseUp10", KeyCodeType::Mouseup, 10),
    ("MouseUp11", KeyCodeType::Mouseup, 11),
    ("MouseDrag1", KeyCodeType::Mousedrag, 1),
    ("MouseDrag2", KeyCodeType::Mousedrag, 2),
    ("MouseDrag3", KeyCodeType::Mousedrag, 3),
    ("MouseDrag6", KeyCodeType::Mousedrag, 6),
    ("MouseDrag7", KeyCodeType::Mousedrag, 7),
    ("MouseDrag8", KeyCodeType::Mousedrag, 8),
    ("MouseDrag9", KeyCodeType::Mousedrag, 9),
    ("MouseDrag10", KeyCodeType::Mousedrag, 10),
    ("MouseDrag11", KeyCodeType::Mousedrag, 11),
    ("MouseDragEnd1", KeyCodeType::Mousedragend, 1),
    ("MouseDragEnd2", KeyCodeType::Mousedragend, 2),
    ("MouseDragEnd3", KeyCodeType::Mousedragend, 3),
    ("MouseDragEnd6", KeyCodeType::Mousedragend, 6),
    ("MouseDragEnd7", KeyCodeType::Mousedragend, 7),
    ("MouseDragEnd8", KeyCodeType::Mousedragend, 8),
    ("MouseDragEnd9", KeyCodeType::Mousedragend, 9),
    ("MouseDragEnd10", KeyCodeType::Mousedragend, 10),
    ("MouseDragEnd11", KeyCodeType::Mousedragend, 11),
    ("WheelUp", KeyCodeType::Wheelup, 0),
    ("WheelDown", KeyCodeType::Wheeldown, 0),
    ("SecondClick1", KeyCodeType::Secondclick, 1),
    ("SecondClick2", KeyCodeType::Secondclick, 2),
    ("SecondClick3", KeyCodeType::Secondclick, 3),
    ("SecondClick6", KeyCodeType::Secondclick, 6),
    ("SecondClick7", KeyCodeType::Secondclick, 7),
    ("SecondClick8", KeyCodeType::Secondclick, 8),
    ("SecondClick9", KeyCodeType::Secondclick, 9),
    ("SecondClick10", KeyCodeType::Secondclick, 10),
    ("SecondClick11", KeyCodeType::Secondclick, 11),
    ("DoubleClick1", KeyCodeType::Doubleclick, 1),
    ("DoubleClick2", KeyCodeType::Doubleclick, 2),
    ("DoubleClick3", KeyCodeType::Doubleclick, 3),
    ("DoubleClick6", KeyCodeType::Doubleclick, 6),
    ("DoubleClick7", KeyCodeType::Doubleclick, 7),
    ("DoubleClick8", KeyCodeType::Doubleclick, 8),
    ("DoubleClick9", KeyCodeType::Doubleclick, 9),
    ("DoubleClick10", KeyCodeType::Doubleclick, 10),
    ("DoubleClick11", KeyCodeType::Doubleclick, 11),
    ("TripleClick1", KeyCodeType::Tripleclick, 1),
    ("TripleClick2", KeyCodeType::Tripleclick, 2),
    ("TripleClick3", KeyCodeType::Tripleclick, 3),
    ("TripleClick6", KeyCodeType::Tripleclick, 6),
    ("TripleClick7", KeyCodeType::Tripleclick, 7),
    ("TripleClick8", KeyCodeType::Tripleclick, 8),
    ("TripleClick9", KeyCodeType::Tripleclick, 9),
    ("TripleClick10", KeyCodeType::Tripleclick, 10),
    ("TripleClick11", KeyCodeType::Tripleclick, 11),
];

/// Location suffixes in `KEYC_MOUSE_STRING` order (`tmux.h:274-294`).
const MOUSE_LOCATIONS: &[(&str, MouseLocation)] = &[
    ("Pane", MouseLocation::Pane),
    ("Status", MouseLocation::Status),
    ("StatusLeft", MouseLocation::StatusLeft),
    ("StatusRight", MouseLocation::StatusRight),
    ("StatusDefault", MouseLocation::StatusDefault),
    ("ScrollbarUp", MouseLocation::ScrollbarUp),
    ("ScrollbarSlider", MouseLocation::ScrollbarSlider),
    ("ScrollbarDown", MouseLocation::ScrollbarDown),
    ("Empty", MouseLocation::Empty),
    ("Border", MouseLocation::Border),
    ("Control0", MouseLocation::Control0),
    ("Control1", MouseLocation::Control1),
    ("Control2", MouseLocation::Control2),
    ("Control3", MouseLocation::Control3),
    ("Control4", MouseLocation::Control4),
    ("Control5", MouseLocation::Control5),
    ("Control6", MouseLocation::Control6),
    ("Control7", MouseLocation::Control7),
    ("Control8", MouseLocation::Control8),
    ("Control9", MouseLocation::Control9),
];

/// Every table entry in C order, for callers that enumerate names.
pub fn table_entries() -> impl Iterator<Item = (String, KeyCode)> {
    NAMES
        .iter()
        .map(|&(name, key)| (name.to_owned(), KeyCode(key)))
        .chain(MOUSE_NAMES.iter().flat_map(|&(prefix, kind, button)| {
            MOUSE_LOCATIONS.iter().map(move |&(suffix, location)| {
                (
                    format!("{prefix}{suffix}"),
                    KeyCode::mouse(kind, button, location),
                )
            })
        }))
}

/// `key_string_search_table` (`key-string.c:196-209`).
fn search_table(s: &[u8]) -> KeyCode {
    for &(name, key) in NAMES {
        if s.eq_ignore_ascii_case(name.as_bytes()) {
            return KeyCode(key);
        }
    }
    for &(prefix, kind, button) in MOUSE_NAMES {
        if s.len() <= prefix.len() || !s[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes()) {
            continue;
        }
        let rest = &s[prefix.len()..];
        for &(suffix, location) in MOUSE_LOCATIONS {
            if rest.eq_ignore_ascii_case(suffix.as_bytes()) {
                return KeyCode::mouse(kind, button, location);
            }
        }
    }
    // sscanf(string, "User%u", &user) == 1 && user <= KEYC_NUSER
    if let Some(rest) = s.strip_prefix(b"User") {
        if let Some((user, _)) = crate::keys::scan::scan_u(rest) {
            if user <= KeyCode::NUSER {
                return KeyCode(K::USER + u64::from(user));
            }
        }
    }
    KeyCode(K::UNKNOWN)
}

/// `key_string_get_modifiers` (`key-string.c:213-239`): `None` for an
/// unknown one-letter prefix.
fn get_modifiers(s: &mut &[u8]) -> Option<u64> {
    let mut modifiers = 0;
    while s.len() >= 2 && s[1] == b'-' {
        match s[0] {
            b'C' | b'c' => modifiers |= CTRL,
            b'M' | b'm' => modifiers |= META,
            b'S' | b's' => modifiers |= SHIFT,
            _ => return None,
        }
        *s = &s[2..];
    }
    Some(modifiers)
}

/// libc `tolower((u_char)c)` under tmux's UTF-8 locale: the macOS rune
/// table maps bytes 0x80-0xff as U+0080-U+00FF; glibc leaves them alone.
fn to_lower(c: u8) -> u8 {
    if cfg!(target_os = "macos") {
        let lower = char::from(c).to_lowercase().next().unwrap_or(char::from(c));
        u8::try_from(u32::from(lower)).unwrap_or(c)
    } else {
        c.to_ascii_lowercase()
    }
}

/// `key_string_lookup_string` (`key-string.c:242-323`): `Unknown` for an
/// invalid name. `name` is read up to its first NUL.
pub fn parse_key_name(name: &[u8]) -> KeyCode {
    let mut s = cstr(name);
    let mut modifiers = 0u64;

    if s.eq_ignore_ascii_case(b"None") {
        return KeyCode(K::NONE);
    }
    if s.eq_ignore_ascii_case(b"Any") {
        return KeyCode(K::ANY);
    }

    // Hexadecimal: sscanf("%x") on the rest, then wctomb and one UTF-8 char.
    if s.len() >= 2 && s[0] == b'0' && s[1] == b'x' {
        let Some((u, _)) = crate::keys::scan::scan_x(&s[2..]) else {
            return KeyCode(K::UNKNOWN);
        };
        if u < 32 {
            return KeyCode(u64::from(u));
        }
        let mut m = [0u8; 32];
        let Some(mlen) = rmux_sys::locale::wctomb(u, &mut m) else {
            return KeyCode(K::UNKNOWN);
        };
        let chars = utf8::from_cstr(&m[..mlen]);
        if chars.0.len() != 1 {
            return KeyCode(K::UNKNOWN);
        }
        return match utf8::from_data(&chars.0[0]) {
            (uc, Utf8State::Done) => KeyCode(u64::from(uc.0)),
            _ => KeyCode(K::UNKNOWN),
        };
    }

    // Short Ctrl key.
    if s.len() >= 2 && s[0] == b'^' {
        if s.len() == 2 {
            return KeyCode(u64::from(to_lower(s[1])) | CTRL);
        }
        modifiers |= CTRL;
        s = &s[1..];
    }

    match get_modifiers(&mut s) {
        Some(m) => modifiers |= m,
        None => return KeyCode(K::UNKNOWN),
    }
    if s.is_empty() {
        return KeyCode(K::UNKNOWN);
    }

    let key = if s.len() == 1 && s[0] <= 127 {
        if s[0] < 32 {
            return KeyCode(K::UNKNOWN);
        }
        u64::from(s[0])
    } else {
        if let Ok(mut ud) = Utf8Data::open(s[0]) {
            if s.len() != usize::from(ud.size) {
                return KeyCode(K::UNKNOWN);
            }
            let mut more = Utf8State::More;
            for &b in &s[1..] {
                more = ud.append(b);
            }
            if more != Utf8State::Done {
                return KeyCode(K::UNKNOWN);
            }
            return match utf8::from_data(&ud) {
                (uc, Utf8State::Done) => KeyCode(u64::from(uc.0) | modifiers),
                _ => KeyCode(K::UNKNOWN),
            };
        }
        let mut key = search_table(s).0;
        if key == K::UNKNOWN {
            return KeyCode(K::UNKNOWN);
        }
        if !modifiers & META != 0 {
            key &= !IMPLIED_META;
        }
        key
    };
    KeyCode(key | modifiers)
}

/// The C `out[64]` with `strlcat` semantics.
struct Out<'a> {
    buf: &'a mut [u8; NAME_SIZE],
    len: usize,
}

impl Out<'_> {
    fn cat(&mut self, s: &[u8]) {
        for &b in s {
            if self.len >= NAME_SIZE - 1 {
                break;
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    fn number(&mut self, mut value: u64, base: u64, limit: usize) {
        let mut digits = [0u8; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b"0123456789abcdef"[(value % base) as usize];
            value /= base;
            if value == 0 {
                break;
            }
        }
        self.cat(&digits[start..][..(digits.len() - start).min(limit)]);
    }
}

/// The special names tested before the table (`key-string.c:354-420`).
fn special_name(key: u64) -> Option<&'static str> {
    Some(match key {
        K::NONE => "None",
        K::UNKNOWN => "Unknown",
        K::ANY => "Any",
        K::FOCUS_IN => "FocusIn",
        K::FOCUS_OUT => "FocusOut",
        K::PASTE_START => "PasteStart",
        K::PASTE_END => "PasteEnd",
        K::REPORT_DARK_THEME => "ReportDarkTheme",
        K::REPORT_LIGHT_THEME => "ReportLightTheme",
        K::MOUSE => "Mouse",
        K::DRAGGING => "Dragging",
        K::MOUSEMOVE_PANE => "MouseMovePane",
        K::MOUSEMOVE_STATUS => "MouseMoveStatus",
        K::MOUSEMOVE_STATUS_LEFT => "MouseMoveStatusLeft",
        K::MOUSEMOVE_STATUS_RIGHT => "MouseMoveStatusRight",
        K::MOUSEMOVE_BORDER => "MouseMoveBorder",
        _ => return None,
    })
}

/// The first table entry whose masked key matches (`key-string.c:428-434`).
fn table_name(key: u64, out: &mut Out<'_>) -> bool {
    if let Some(&(name, _)) = NAMES.iter().find(|&&(_, k)| key == k & KeyMasks::KEY) {
        out.cat(name.as_bytes());
        return true;
    }
    for &(prefix, kind, button) in MOUSE_NAMES {
        for &(suffix, location) in MOUSE_LOCATIONS {
            if key == KeyCode::mouse(kind, button, location).0 {
                out.cat(prefix.as_bytes());
                out.cat(suffix.as_bytes());
                return true;
            }
        }
    }
    false
}

/// `key_string_lookup_key` (`key-string.c:327-485`) into caller storage;
/// returns the length (the C `strlen`).
pub fn write_key_name(key: KeyCode, with_flags: bool, out: &mut [u8; NAME_SIZE]) -> usize {
    let saved = key.0;
    let mut out = Out { buf: out, len: 0 };

    // Literal keys are themselves; "%c" of NUL leaves an empty string.
    if saved & KeyFlags::LITERAL.0 != 0 {
        let c = (saved & 0xff) as u8;
        if c != 0 {
            out.cat(&[c]);
        }
    } else {
        if saved & CTRL != 0 {
            out.cat(b"C-");
        }
        if saved & META != 0 {
            out.cat(b"M-");
        }
        if saved & SHIFT != 0 {
            out.cat(b"S-");
        }
        let key = saved & KeyMasks::KEY;

        if let Some(s) = special_name(key) {
            out.cat(s.as_bytes());
        } else if KeyCode(key).is_user() {
            // snprintf(tmp, sizeof tmp = 8, "User%u", ...) truncates.
            out.cat(b"User");
            out.number(u64::from(key.wrapping_sub(K::USER) as u32), 10, 3);
        } else if table_name(key, &mut out) {
        } else if KeyCode(key).is_unicode() {
            let ud = utf8::to_data(rmux_util::utf8::Utf8Char(key as u32));
            let off = out.len;
            let size = usize::from(ud.size);
            out.buf[off..off + size].copy_from_slice(ud.bytes());
            out.len = out.buf[..off + size]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(off + size);
        } else if key > 255 {
            out.len = 0;
            out.cat(b"Invalid#");
            out.number(saved, 16, 16);
        } else if key > 32 && key <= 126 {
            out.cat(&[key as u8]);
        } else if key == 127 {
            out.cat(b"C-?");
        } else if key >= 128 {
            out.cat(b"\\");
            out.number(key, 8, 3);
        }
    }

    if with_flags && saved & KeyMasks::FLAGS != 0 {
        out.cat(b"[");
        for (flag, letter) in [
            (KeyFlags::LITERAL, b"L".as_slice()),
            (KeyFlags::KEYPAD, b"K"),
            (KeyFlags::CURSOR, b"C"),
            (KeyFlags::IMPLIED_META, b"I"),
            (KeyFlags::BUILD_MODIFIERS, b"B"),
            (KeyFlags::SENT, b"S"),
        ] {
            if saved & flag.0 != 0 {
                out.cat(letter);
            }
        }
        out.cat(b"]");
    }
    out.buf[out.len] = 0;
    out.len
}

/// `write_key_name` as an owned byte string.
pub fn key_name(key: KeyCode, with_flags: bool) -> Vec<u8> {
    let mut out = [0u8; NAME_SIZE];
    let len = write_key_name(key, with_flags, &mut out);
    out[..len].to_vec()
}
