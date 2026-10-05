// Ported from tmux input-keys.c @ 8f25579c
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

//! Key code to pane byte sequence translation (`input-keys.c`): the
//! opposite direction of the parser.

use crate::input::effect::ExtendedKeysFormat;
use crate::screen::ScreenMode;
use rmux_util::key::{
    C0, KeyCode, KeyFlags, KeyMasks, KeyModifiers, MOUSE_PARAM_BTN_OFF, MOUSE_PARAM_MAX,
    MOUSE_PARAM_POS_OFF, MOUSE_PARAM_UTF8_MAX, MouseButtonBits, MouseEvent, SpecialKey,
};
use rmux_util::utf8::{Utf8Char, to_data};
use std::io::Write;
use std::ops::Deref;

/// The `backspace` and `extended-keys-format` global options that
/// `input_key` reads (`input-keys.c:467,593`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyPolicy {
    pub backspace: KeyCode,
    pub format: ExtendedKeysFormat,
}

impl Default for KeyPolicy {
    /// `backspace` is `C-?` (`options-table.c:292-295`) and
    /// `extended-keys-format` is `xterm` (`options-table.c:422-428`).
    fn default() -> Self {
        Self {
            backspace: KeyCode(0x7f),
            format: ExtendedKeysFormat::Xterm,
        }
    }
}

/// The `-1` returns of `input_key` and its helpers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEncodeError {
    /// Extended form without a modifier (`input-keys.c:454-455`).
    NoModifier,
    /// Extended form of a Unicode key `utf8_towc` rejects
    /// (`input-keys.c:462-463`).
    InvalidUnicode,
    /// vt10x Ctrl key with no C0 mapping (`input-keys.c:532-533`).
    Unmappable,
}

/// One table entry's output bytes; the longest C sequence has 7 bytes
/// (`input-keys.c:262-283`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyBytes {
    bytes: [u8; 8],
    len: u8,
}

impl KeyBytes {
    const fn from_slice(data: &[u8]) -> Self {
        let mut bytes = [0u8; 8];
        let mut i = 0;
        while i < data.len() {
            bytes[i] = data[i];
            i += 1;
        }
        Self {
            bytes,
            len: data.len() as u8,
        }
    }

    /// `input_key_build` (`input-keys.c:381`): the first `_` becomes `'0' + j`.
    const fn with_modifier_digit(self, j: u8) -> Self {
        let mut out = self;
        let mut i = 0;
        while i < out.len as usize {
            if out.bytes[i] == b'_' {
                out.bytes[i] = b'0' + j;
                break;
            }
            i += 1;
        }
        out
    }
}

impl Deref for KeyBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

impl AsRef<[u8]> for KeyBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

const SHIFT: u64 = KeyModifiers::SHIFT.0;
const META: u64 = KeyModifiers::META.0;
const CTRL: u64 = KeyModifiers::CTRL.0;
const LITERAL: u64 = KeyFlags::LITERAL.0;
const KEYPAD: u64 = KeyFlags::KEYPAD.0;
const CURSOR: u64 = KeyFlags::CURSOR.0;
const IMPLIED_META: u64 = KeyFlags::IMPLIED_META.0;
const BUILD_MODIFIERS: u64 = KeyFlags::BUILD_MODIFIERS.0;

/// `input_key_defaults` (`input-keys.c:51-316`).
const DEFAULTS: [(u64, &[u8]); 85] = [
    (SpecialKey::PASTE_START, b"\x1b[200~"),
    (SpecialKey::PASTE_START | IMPLIED_META, b"\x1b[200~"),
    (SpecialKey::PASTE_END, b"\x1b[201~"),
    (SpecialKey::PASTE_END | IMPLIED_META, b"\x1b[201~"),
    (SpecialKey::F1, b"\x1bOP"),
    (SpecialKey::F2, b"\x1bOQ"),
    (SpecialKey::F3, b"\x1bOR"),
    (SpecialKey::F4, b"\x1bOS"),
    (SpecialKey::F5, b"\x1b[15~"),
    (SpecialKey::F6, b"\x1b[17~"),
    (SpecialKey::F7, b"\x1b[18~"),
    (SpecialKey::F8, b"\x1b[19~"),
    (SpecialKey::F9, b"\x1b[20~"),
    (SpecialKey::F10, b"\x1b[21~"),
    (SpecialKey::F11, b"\x1b[23~"),
    (SpecialKey::F12, b"\x1b[24~"),
    (SpecialKey::IC, b"\x1b[2~"),
    (SpecialKey::DC, b"\x1b[3~"),
    (SpecialKey::HOME, b"\x1b[1~"),
    (SpecialKey::END, b"\x1b[4~"),
    (SpecialKey::NPAGE, b"\x1b[6~"),
    (SpecialKey::PPAGE, b"\x1b[5~"),
    (SpecialKey::BTAB, b"\x1b[Z"),
    (SpecialKey::UP | CURSOR, b"\x1bOA"),
    (SpecialKey::DOWN | CURSOR, b"\x1bOB"),
    (SpecialKey::RIGHT | CURSOR, b"\x1bOC"),
    (SpecialKey::LEFT | CURSOR, b"\x1bOD"),
    (SpecialKey::UP, b"\x1b[A"),
    (SpecialKey::DOWN, b"\x1b[B"),
    (SpecialKey::RIGHT, b"\x1b[C"),
    (SpecialKey::LEFT, b"\x1b[D"),
    (SpecialKey::KP_SLASH | KEYPAD, b"\x1bOo"),
    (SpecialKey::KP_STAR | KEYPAD, b"\x1bOj"),
    (SpecialKey::KP_MINUS | KEYPAD, b"\x1bOm"),
    (SpecialKey::KP_SEVEN | KEYPAD, b"\x1bOw"),
    (SpecialKey::KP_EIGHT | KEYPAD, b"\x1bOx"),
    (SpecialKey::KP_NINE | KEYPAD, b"\x1bOy"),
    (SpecialKey::KP_PLUS | KEYPAD, b"\x1bOk"),
    (SpecialKey::KP_FOUR | KEYPAD, b"\x1bOt"),
    (SpecialKey::KP_FIVE | KEYPAD, b"\x1bOu"),
    (SpecialKey::KP_SIX | KEYPAD, b"\x1bOv"),
    (SpecialKey::KP_ONE | KEYPAD, b"\x1bOq"),
    (SpecialKey::KP_TWO | KEYPAD, b"\x1bOr"),
    (SpecialKey::KP_THREE | KEYPAD, b"\x1bOs"),
    (SpecialKey::KP_ENTER | KEYPAD, b"\x1bOM"),
    (SpecialKey::KP_ZERO | KEYPAD, b"\x1bOp"),
    (SpecialKey::KP_PERIOD | KEYPAD, b"\x1bOn"),
    (SpecialKey::KP_SLASH, b"/"),
    (SpecialKey::KP_STAR, b"*"),
    (SpecialKey::KP_MINUS, b"-"),
    (SpecialKey::KP_SEVEN, b"7"),
    (SpecialKey::KP_EIGHT, b"8"),
    (SpecialKey::KP_NINE, b"9"),
    (SpecialKey::KP_PLUS, b"+"),
    (SpecialKey::KP_FOUR, b"4"),
    (SpecialKey::KP_FIVE, b"5"),
    (SpecialKey::KP_SIX, b"6"),
    (SpecialKey::KP_ONE, b"1"),
    (SpecialKey::KP_TWO, b"2"),
    (SpecialKey::KP_THREE, b"3"),
    (SpecialKey::KP_ENTER, b"\n"),
    (SpecialKey::KP_ZERO, b"0"),
    (SpecialKey::KP_PERIOD, b"."),
    (SpecialKey::F1 | BUILD_MODIFIERS, b"\x1b[1;_P"),
    (SpecialKey::F2 | BUILD_MODIFIERS, b"\x1b[1;_Q"),
    (SpecialKey::F3 | BUILD_MODIFIERS, b"\x1b[1;_R"),
    (SpecialKey::F4 | BUILD_MODIFIERS, b"\x1b[1;_S"),
    (SpecialKey::F5 | BUILD_MODIFIERS, b"\x1b[15;_~"),
    (SpecialKey::F6 | BUILD_MODIFIERS, b"\x1b[17;_~"),
    (SpecialKey::F7 | BUILD_MODIFIERS, b"\x1b[18;_~"),
    (SpecialKey::F8 | BUILD_MODIFIERS, b"\x1b[19;_~"),
    (SpecialKey::F9 | BUILD_MODIFIERS, b"\x1b[20;_~"),
    (SpecialKey::F10 | BUILD_MODIFIERS, b"\x1b[21;_~"),
    (SpecialKey::F11 | BUILD_MODIFIERS, b"\x1b[23;_~"),
    (SpecialKey::F12 | BUILD_MODIFIERS, b"\x1b[24;_~"),
    (SpecialKey::UP | BUILD_MODIFIERS, b"\x1b[1;_A"),
    (SpecialKey::DOWN | BUILD_MODIFIERS, b"\x1b[1;_B"),
    (SpecialKey::RIGHT | BUILD_MODIFIERS, b"\x1b[1;_C"),
    (SpecialKey::LEFT | BUILD_MODIFIERS, b"\x1b[1;_D"),
    (SpecialKey::HOME | BUILD_MODIFIERS, b"\x1b[1;_H"),
    (SpecialKey::END | BUILD_MODIFIERS, b"\x1b[1;_F"),
    (SpecialKey::PPAGE | BUILD_MODIFIERS, b"\x1b[5;_~"),
    (SpecialKey::NPAGE | BUILD_MODIFIERS, b"\x1b[6;_~"),
    (SpecialKey::IC | BUILD_MODIFIERS, b"\x1b[2;_~"),
    (SpecialKey::DC | BUILD_MODIFIERS, b"\x1b[3;_~"),
];

/// `input_key_modifiers` (`input-keys.c:317-327`), indexed by the xterm
/// modifier digit.
const MODIFIERS: [u64; 9] = [
    0,
    0,
    SHIFT,
    META | IMPLIED_META,
    SHIFT | META | IMPLIED_META,
    CTRL,
    SHIFT | CTRL,
    META | IMPLIED_META | CTRL,
    SHIFT | META | IMPLIED_META | CTRL,
];

const fn count_entries() -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < DEFAULTS.len() {
        if DEFAULTS[i].0 & BUILD_MODIFIERS != 0 {
            n += MODIFIERS.len() - 2;
        } else {
            n += 1;
        }
        i += 1;
    }
    n
}

/// Entries in the built key table (`input_key_tree`).
pub const KEY_TABLE_LEN: usize = count_entries();

/// `input_key_build` (`input-keys.c:364-394`) at compile time: expand the
/// `KEYC_BUILD_MODIFIERS` templates, then sort by raw key value.
const fn build_table() -> [(KeyCode, KeyBytes); KEY_TABLE_LEN] {
    let mut table = [(KeyCode(0), KeyBytes::from_slice(b"")); KEY_TABLE_LEN];
    let mut n = 0;
    let mut i = 0;
    while i < DEFAULTS.len() {
        let (key, data) = DEFAULTS[i];
        if key & BUILD_MODIFIERS == 0 {
            table[n] = (KeyCode(key), KeyBytes::from_slice(data));
            n += 1;
        } else {
            let base = key & !BUILD_MODIFIERS;
            let template = KeyBytes::from_slice(data);
            let mut j = 2;
            while j < MODIFIERS.len() {
                table[n] = (
                    KeyCode(base | MODIFIERS[j]),
                    template.with_modifier_digit(j as u8),
                );
                n += 1;
                j += 1;
            }
        }
        i += 1;
    }
    let mut a = 1;
    while a < table.len() {
        let mut b = a;
        while b > 0 && table[b - 1].0.0 > table[b].0.0 {
            let tmp = table[b - 1];
            table[b - 1] = table[b];
            table[b] = tmp;
            b -= 1;
        }
        a += 1;
    }
    table
}

const KEY_TABLE: [(KeyCode, KeyBytes); KEY_TABLE_LEN] = build_table();

/// The sorted key table (`input_key_tree` in `RB_FOREACH` order).
pub fn table_entries() -> &'static [(KeyCode, KeyBytes)] {
    &KEY_TABLE
}

/// `input_key_get` (`input-keys.c:342-347`).
fn table_get(key: u64) -> Option<&'static KeyBytes> {
    KEY_TABLE
        .binary_search_by(|entry| entry.0.0.cmp(&key))
        .ok()
        .map(|index| &KEY_TABLE[index].1)
}

/// `input_key_extended` (`input-keys.c:426-474`).
fn encode_extended(
    key: KeyCode,
    format: ExtendedKeysFormat,
    out: &mut Vec<u8>,
) -> Result<(), KeyEncodeError> {
    let modifier = match key.0 & KeyMasks::MODIFIERS {
        m if m == SHIFT => b'2',
        m if m == META => b'3',
        m if m == SHIFT | META => b'4',
        m if m == CTRL => b'5',
        m if m == SHIFT | CTRL => b'6',
        m if m == META | CTRL => b'7',
        m if m == SHIFT | META | CTRL => b'8',
        _ => return Err(KeyEncodeError::NoModifier),
    };

    let code = if key.is_unicode() {
        let ud = to_data(Utf8Char((key.0 & KeyMasks::KEY) as u32));
        match ud.to_wc() {
            Some(wc) => u64::from(wc),
            None => return Err(KeyEncodeError::InvalidUnicode),
        }
    } else {
        key.0 & KeyMasks::KEY
    };

    let modifier = char::from(modifier);
    let _ = match format {
        ExtendedKeysFormat::Xterm => write!(out, "\x1b[27;{modifier};{code}~"),
        ExtendedKeysFormat::CsiU => write!(out, "\x1b[{code};{modifier}u"),
    };
    Ok(())
}

/// `standard_map` (`input-keys.c:487-490`): the Ctrl remaps of vt10x mode.
/// The C `strchr` also matches the terminating NUL, so Ctrl-NUL maps to NUL.
const STANDARD_MAP: [u8; 22] = *b"1!9(0)=+;:'\",<.>/-8? 2";
const STANDARD_OUT: [u8; 22] = *b"119900=+;;'',,..\x1f\x1f\x7f\x7f\0\0";

/// `input_key_vt10x` (`input-keys.c:482-541`).
fn encode_vt10x(key: KeyCode, out: &mut Vec<u8>) -> Result<(), KeyEncodeError> {
    let mut key = key.0;
    if key & META != 0 {
        out.push(C0::ESC);
    }

    if KeyCode(key).is_unicode() {
        let ud = to_data(Utf8Char((key & KeyMasks::KEY) as u32));
        out.extend_from_slice(ud.bytes());
        return Ok(());
    }

    let onlykey = key & KeyMasks::KEY;
    if onlykey == u64::from(C0::CR) || onlykey == u64::from(C0::LF) || onlykey == u64::from(C0::HT)
    {
        key &= !CTRL;
    }

    if key & CTRL != 0 {
        let mapped = if onlykey == 0 {
            Some(0)
        } else {
            u8::try_from(onlykey).ok().and_then(|c| {
                STANDARD_MAP
                    .iter()
                    .position(|&m| m == c)
                    .map(|index| u64::from(STANDARD_OUT[index]))
            })
        };
        key = match mapped {
            Some(mapped) => mapped,
            None if (u64::from(b'3')..=u64::from(b'7')).contains(&onlykey) => onlykey - 0o30,
            None if (u64::from(b'@')..=u64::from(b'~')).contains(&onlykey) => onlykey & 0x1f,
            None => return Err(KeyEncodeError::Unmappable),
        };
    }

    out.push((key & 0x7f) as u8);
    Ok(())
}

/// `input_key_mode1` (`input-keys.c:545-570`): `None` is the C `-1`, which
/// falls back to the extended form without having written anything.
fn encode_mode1(key: KeyCode, out: &mut Vec<u8>) -> Option<Result<(), KeyEncodeError>> {
    if key.0 & (CTRL | META) == META {
        return Some(encode_vt10x(key, out));
    }

    let onlykey = key.0 & KeyMasks::KEY;
    if key.0 & CTRL != 0
        && (onlykey == u64::from(b' ')
            || onlykey == u64::from(b'/')
            || onlykey == u64::from(b'@')
            || onlykey == u64::from(b'^')
            || (u64::from(b'2')..=u64::from(b'8')).contains(&onlykey)
            || (u64::from(b'@')..=u64::from(b'~')).contains(&onlykey))
    {
        return Some(encode_vt10x(key, out));
    }

    None
}

/// `input_key` (`input-keys.c:574-709`): append the pane bytes for `key`.
/// Bytes already appended stay in `out` on error, as C already wrote them.
pub fn encode_key(
    mode: ScreenMode,
    key: KeyCode,
    policy: &KeyPolicy,
    out: &mut Vec<u8>,
) -> Result<(), KeyEncodeError> {
    if key.is_mouse() {
        return Ok(());
    }

    let mut key = key.0;
    if key & LITERAL != 0 {
        out.push(key as u8);
        return Ok(());
    }

    if key & KeyMasks::KEY == SpecialKey::BSPACE {
        let mut newkey = policy.backspace.0;
        if key & KeyMasks::MODIFIERS == 0 {
            let mut byte = 255u8;
            if newkey & KeyMasks::MODIFIERS == 0 {
                byte = newkey as u8;
            } else if newkey & KeyMasks::MODIFIERS == CTRL {
                newkey &= KeyMasks::KEY;
                if newkey == u64::from(b'?') {
                    byte = 0x7f;
                } else if (u64::from(b'@')..=u64::from(b'_')).contains(&newkey) {
                    byte = (newkey - 0x40) as u8;
                } else if (u64::from(b'a')..=u64::from(b'z')).contains(&newkey) {
                    byte = (newkey - 0x60) as u8;
                }
            }
            if byte != 255 {
                out.push(byte);
            }
            return Ok(());
        }
        key = newkey | (key & (KeyMasks::FLAGS | KeyMasks::MODIFIERS));
    }

    if key & KeyMasks::KEY == SpecialKey::BTAB {
        if mode.contains(ScreenMode::KEYS_EXTENDED_2) {
            key = u64::from(C0::HT) | (key & !KeyMasks::KEY) | SHIFT;
        } else {
            key &= !KeyMasks::MODIFIERS;
        }
    }

    if key & !KeyMasks::KEY == 0 {
        if key == u64::from(C0::HT)
            || key == u64::from(C0::CR)
            || key == u64::from(C0::ESC)
            || (0x20..=0x7f).contains(&key)
        {
            out.push(key as u8);
            return Ok(());
        }
        if KeyCode(key).is_unicode() {
            let ud = to_data(Utf8Char(key as u32));
            out.extend_from_slice(ud.bytes());
            return Ok(());
        }
    }

    if !mode.contains(ScreenMode::KKEYPAD) {
        key &= !KEYPAD;
    }
    if !mode.contains(ScreenMode::KCURSOR) {
        key &= !CURSOR;
    }
    let mut ike = table_get(key);
    if ike.is_none() && key & META != 0 && key & IMPLIED_META == 0 {
        ike = table_get(key & !META);
    }
    if ike.is_none() && key & CURSOR != 0 {
        ike = table_get(key & !CURSOR);
    }
    if ike.is_none() && key & KEYPAD != 0 {
        ike = table_get(key & !KEYPAD);
    }
    if let Some(data) = ike {
        if KeyCode(key).is_paste() && !mode.contains(ScreenMode::BRACKETPASTE) {
            return Ok(());
        }
        if key & META != 0 && key & IMPLIED_META == 0 {
            out.push(C0::ESC);
        }
        out.extend_from_slice(data);
        return Ok(());
    }

    let key = KeyCode(key);
    if key.is_user() || key.is_special() || key.is_mouse() {
        return Ok(());
    }

    match ScreenMode(mode.0 & ScreenMode::EXTENDED_KEY_MODES.0) {
        ScreenMode::KEYS_EXTENDED_2 => encode_extended(key, policy.format, out),
        ScreenMode::KEYS_EXTENDED => match encode_mode1(key, out) {
            Some(result) => result,
            None => encode_extended(key, policy.format, out),
        },
        _ => encode_vt10x(key, out),
    }
}

/// The `static char buf[40]` of `input_key_get_mouse` (`input-keys.c:716`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseBytes {
    bytes: [u8; 40],
    len: usize,
}

impl MouseBytes {
    fn push(&mut self, byte: u8) {
        self.bytes[self.len] = byte;
        self.len += 1;
    }

    /// `input_key_split2` (`input-keys.c:351-360`).
    fn push_split2(&mut self, c: u32) {
        if c > 0x7f {
            self.push(((c >> 6) | 0xc0) as u8);
            self.push(((c & 0x3f) | 0x80) as u8);
        } else {
            self.push(c as u8);
        }
    }
}

impl Deref for MouseBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl AsRef<[u8]> for MouseBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

/// `input_key_get_mouse` (`input-keys.c:713-793`): reads `b`, `lb`,
/// `sgr_b` and `sgr_type` from the decoded event and takes the
/// pane-relative `x`, `y` the server computed. `None` is the C `0` return.
pub fn encode_mouse(mode: ScreenMode, m: &MouseEvent, x: u32, y: u32) -> Option<MouseBytes> {
    let (b, lb, sgr_b, sgr_type) = (m.b, m.lb, m.sgr_b, m.sgr_type);
    let b_bits = MouseButtonBits(b);
    if b_bits.is_drag() && !mode.intersects(ScreenMode::MOTION_MOUSE_MODES) {
        return None;
    }
    if !mode.intersects(ScreenMode::ALL_MOUSE_MODES) {
        return None;
    }

    if sgr_type != b' ' {
        let sgr_bits = MouseButtonBits(sgr_b);
        if sgr_bits.is_drag() && sgr_bits.is_release() && !mode.contains(ScreenMode::MOUSE_ALL) {
            return None;
        }
    } else if b_bits.is_drag()
        && b_bits.is_release()
        && MouseButtonBits(lb).is_release()
        && !mode.contains(ScreenMode::MOUSE_ALL)
    {
        return None;
    }

    let mut buf = MouseBytes {
        bytes: [0; 40],
        len: 0,
    };
    if sgr_type != b' ' && mode.contains(ScreenMode::MOUSE_SGR) {
        let mut cursor = &mut buf.bytes[..];
        let _ = write!(
            cursor,
            "\x1b[<{sgr_b};{};{}",
            x.wrapping_add(1),
            y.wrapping_add(1)
        );
        buf.len = 40 - cursor.len();
        buf.push(sgr_type);
    } else if mode.contains(ScreenMode::MOUSE_UTF8) {
        if b > MOUSE_PARAM_UTF8_MAX - MOUSE_PARAM_BTN_OFF
            || x > MOUSE_PARAM_UTF8_MAX - MOUSE_PARAM_POS_OFF
            || y > MOUSE_PARAM_UTF8_MAX - MOUSE_PARAM_POS_OFF
        {
            return None;
        }
        buf.bytes[..3].copy_from_slice(b"\x1b[M");
        buf.len = 3;
        buf.push_split2(b + MOUSE_PARAM_BTN_OFF);
        buf.push_split2(x + MOUSE_PARAM_POS_OFF);
        buf.push_split2(y + MOUSE_PARAM_POS_OFF);
    } else {
        if b.wrapping_add(MOUSE_PARAM_BTN_OFF) > MOUSE_PARAM_MAX {
            return None;
        }
        buf.bytes[..3].copy_from_slice(b"\x1b[M");
        buf.len = 3;
        buf.push(b.wrapping_add(MOUSE_PARAM_BTN_OFF) as u8);
        buf.push(if x.wrapping_add(MOUSE_PARAM_POS_OFF) > MOUSE_PARAM_MAX {
            MOUSE_PARAM_MAX as u8
        } else {
            x.wrapping_add(MOUSE_PARAM_POS_OFF) as u8
        });
        buf.push(if y.wrapping_add(MOUSE_PARAM_POS_OFF) > MOUSE_PARAM_MAX {
            MOUSE_PARAM_MAX as u8
        } else {
            y.wrapping_add(MOUSE_PARAM_POS_OFF) as u8
        });
    }
    Some(buf)
}
