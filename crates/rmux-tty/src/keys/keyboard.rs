// Ported from tmux tty-keys.c @ 8f25579c
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

//! Keyboard input: the extended-key parser (`tty_keys_extended_key`,
//! `tty-keys.c:1083-1202`) and the byte-fallback normalization at the end
//! of `tty_keys_next` (`tty-keys.c:928-975`).

use super::scan::scan_u_pair;
use super::{KeyDecodeContext, Recognition};
use rmux_util::key::{C0, KeyCode, KeyFlags, KeyMasks, KeyModifiers, SpecialKey};
use rmux_util::utf8::{self, Utf8Data, Utf8State};

use Recognition::{Complete, NoMatch, Partial};

const META: u64 = KeyModifiers::META.0;
const CTRL: u64 = KeyModifiers::CTRL.0;
const SHIFT: u64 = KeyModifiers::SHIFT.0;
const IMPLIED_META: u64 = KeyFlags::IMPLIED_META.0;

/// `tty_keys_extended_key`: `ESC [ 27 ; m ; k ~` or `ESC [ k ; m u`.
pub fn extended_key(buf: &[u8], ctx: &KeyDecodeContext) -> Recognition<KeyCode> {
    let len = buf.len();
    if buf[0] != 0x1b {
        return NoMatch;
    }
    if len == 1 {
        return Partial;
    }
    if buf[1] != b'[' {
        return NoMatch;
    }
    if len == 2 {
        return Partial;
    }

    // Stop at '~' or anything that is not a digit or ';' (tmp is 64 bytes).
    let mut end = 2;
    while end < len && end != 64 {
        if buf[end] == b'~' {
            break;
        }
        if !buf[end].is_ascii_digit() && buf[end] != b';' {
            break;
        }
        end += 1;
    }
    if end == len {
        return Partial;
    }
    if end == 64 || (buf[end] != b'~' && buf[end] != b'u') {
        return NoMatch;
    }
    let tmp = &buf[2..end];

    let (number, modifiers) = if buf[end] == b'~' {
        match scan_u_pair(tmp, b"27;") {
            Some((modifiers, number)) => (number, modifiers),
            None => return NoMatch,
        }
    } else {
        match scan_u_pair(tmp, b"") {
            Some(pair) => pair,
            None => return NoMatch,
        }
    };
    let size = end + 1;

    let mut nkey = match ctx.verase {
        Some(bspace) if number == u32::from(bspace) => SpecialKey::BSPACE,
        _ => u64::from(number),
    };

    // Convert a UTF-32 code point into the packed internal representation.
    if nkey != SpecialKey::BSPACE && nkey & !0x7f != 0 {
        let packed = Utf8Data::from_wc(number).and_then(|ud| match utf8::from_data(&ud) {
            (uc, Utf8State::Done) => Some(uc),
            _ => None,
        });
        match packed {
            Some(uc) => nkey = u64::from(uc.0),
            None => return NoMatch,
        }
    }

    if modifiers > 0 {
        let modifiers = modifiers - 1;
        if modifiers & 1 != 0 {
            nkey |= SHIFT;
        }
        if modifiers & 2 != 0 {
            nkey |= META | IMPLIED_META;
        }
        if modifiers & 4 != 0 {
            nkey |= CTRL;
        }
        if modifiers & 8 != 0 {
            nkey |= META | IMPLIED_META;
        }
    }

    // S-Tab is Backtab.
    if nkey & KeyMasks::KEY == u64::from(C0::HT) && nkey & SHIFT != 0 {
        nkey = SpecialKey::BTAB | (nkey & !KeyMasks::KEY & !SHIFT);
    }

    // A lone Shift on a printable or Unicode key is dropped: terminals
    // disagree on whether they report S-a or A (`tty-keys.c:1175-1193`).
    let onlykey = nkey & KeyMasks::KEY;
    if ((onlykey > 0x20 && onlykey < 0x7f) || KeyCode(nkey).is_unicode())
        && nkey & KeyMasks::MODIFIERS == SHIFT
    {
        nkey &= !SHIFT;
    }

    Complete(size, KeyCode(nkey))
}

/// The byte fallback (`tty-keys.c:932-975`): one byte, or Escape plus the
/// next byte as Meta, then NUL, VERASE and C0 normalization.
pub fn fallback(buf: &[u8], ctx: &KeyDecodeContext) -> (KeyCode, usize) {
    let (mut key, size) = if buf[0] == 0x1b && buf.len() >= 2 {
        (u64::from(buf[1]) | META, 2)
    } else {
        (u64::from(buf[0]), 1)
    };

    // C-Space is special.
    if key & KeyMasks::KEY == u64::from(C0::NUL) {
        key = u64::from(b' ') | CTRL | (key & META);
    }

    // Backspace comes from termios VERASE, not terminfo kbs.
    if let Some(bspace) = ctx.verase {
        let bspace = u64::from(bspace);
        if key == bspace {
            key = SpecialKey::BSPACE;
        }
        if key == (bspace | META) {
            key = SpecialKey::BSPACE | META;
        }
    }

    // Remaining C0 codes become Ctrl keys, A-Z lowercased.
    let mut onlykey = key & KeyMasks::KEY;
    if onlykey < 0x20
        && onlykey != u64::from(C0::HT)
        && onlykey != u64::from(C0::CR)
        && onlykey != u64::from(C0::ESC)
    {
        onlykey |= 0x40;
        if (u64::from(b'A')..=u64::from(b'Z')).contains(&onlykey) {
            onlykey |= 0x20;
        }
        key = onlykey | CTRL | (key & META);
    }

    (KeyCode(key), size)
}
