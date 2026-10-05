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

//! `tty_keys_mouse` (`tty-keys.c:1204-1347`): legacy `ESC [ M` reports and
//! SGR `ESC [ <` reports.

use rmux_util::key::{MOUSE_PARAM_BTN_OFF, MOUSE_PARAM_POS_OFF, MouseButtonBits, MouseEvent};

/// The last terminal mouse position and button (`tty->mouse_last_*`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseLast {
    pub x: u32,
    pub y: u32,
    pub b: u32,
}

pub enum MouseResult {
    /// A valid report: `size` bytes and the event.
    Complete(usize, MouseEvent),
    /// Probably a mouse report but more bytes are needed.
    Partial,
    /// Not a mouse report.
    NoMatch,
    /// A recognized report tmux drops: consume `size` bytes, no event.
    Discard(usize),
}

enum Field {
    End(u8),
    Partial,
    Bad,
}

/// One SGR decimal field (`tty-keys.c:1275-1304`): digits accumulate with
/// u_int wrapping until a terminator; anything else fails the candidate.
fn field(buf: &[u8], acc: &mut u32, terminators: &[u8], size: &mut usize) -> Field {
    loop {
        if buf.len() <= *size {
            return Field::Partial;
        }
        let ch = buf[*size];
        *size += 1;
        if terminators.contains(&ch) {
            return Field::End(ch);
        }
        if !ch.is_ascii_digit() {
            return Field::Bad;
        }
        *acc = acc.wrapping_mul(10).wrapping_add(u32::from(ch - b'0'));
    }
}

/// `tty_keys_mouse`. A valid event copies the previous terminal state into
/// `lx`/`ly`/`lb` and then updates `last`; discards leave it unchanged.
pub fn mouse(buf: &[u8], last: &mut MouseLast) -> MouseResult {
    let len = buf.len();
    let (mut x, mut y, mut b, mut sgr_b) = (0u32, 0u32, 0u32, 0u32);
    let mut sgr_type = b' ';

    if buf[0] != 0x1b {
        return MouseResult::NoMatch;
    }
    if len == 1 {
        return MouseResult::Partial;
    }
    if buf[1] != b'[' {
        return MouseResult::NoMatch;
    }
    if len == 2 {
        return MouseResult::Partial;
    }

    let mut size = 3;
    if buf[2] == b'M' {
        for i in 0..3 {
            if len <= size {
                return MouseResult::Partial;
            }
            let ch = u32::from(buf[size]);
            size += 1;
            match i {
                0 => b = ch,
                1 => x = ch,
                _ => y = ch,
            }
        }
        if b < MOUSE_PARAM_BTN_OFF || x < MOUSE_PARAM_POS_OFF || y < MOUSE_PARAM_POS_OFF {
            return MouseResult::Discard(size);
        }
        b -= MOUSE_PARAM_BTN_OFF;
        x -= MOUSE_PARAM_POS_OFF;
        y -= MOUSE_PARAM_POS_OFF;
    } else if buf[2] == b'<' {
        // Three decimal fields; the accumulators wrap like C u_int.
        let mut ch = 0;
        for (acc, terminators) in [
            (&mut sgr_b, b";".as_slice()),
            (&mut x, b";"),
            (&mut y, b"Mm"),
        ] {
            match field(buf, acc, terminators, &mut size) {
                Field::Bad => return MouseResult::NoMatch,
                Field::Partial => return MouseResult::Partial,
                Field::End(c) => ch = c,
            }
        }
        if x < 1 || y < 1 {
            return MouseResult::Discard(size);
        }
        x -= 1;
        y -= 1;
        b = sgr_b;
        sgr_type = ch;
        if sgr_type == b'm' {
            b = 3;
        }
        // PuTTY 0.63 sends releases for wheel presses; drop them.
        if sgr_type == b'm' && MouseButtonBits(sgr_b).is_wheel() {
            return MouseResult::Discard(size);
        }
    } else {
        return MouseResult::NoMatch;
    }

    let m = MouseEvent {
        lx: last.x,
        x,
        ly: last.y,
        y,
        lb: last.b,
        b,
        sgr_type,
        sgr_b,
    };
    last.x = x;
    last.y = y;
    last.b = b;
    MouseResult::Complete(size, m)
}
