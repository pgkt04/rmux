// Ported from tmux input.c @ 8f25579c
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

//! Reply formatting shared by the parser and the server request code
//! (`input.c:2896-2922,3476-3501,3715-3737`).

use super::InputEnd;
use crate::colour::{ClientTheme, Colour};
use std::io::Write;

/// `input_osc_colour_reply` (`input.c:2896-2922`): `false` when the colour
/// cannot be forced to RGB and nothing is written.
pub fn colour(n: u32, idx: Option<u8>, c: Colour, end: InputEnd, out: &mut Vec<u8>) -> bool {
    if c == Colour::NONE {
        return false;
    }
    let Some(c) = c.force_rgb() else {
        return false;
    };
    let (r, g, b) = c.split_rgb();
    match idx {
        Some(idx) => write!(
            out,
            "\x1b]{n};{idx};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}"
        ),
        None => write!(
            out,
            "\x1b]{n};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}"
        ),
    }
    .expect("reply fits");
    out.extend_from_slice(end.bytes());
    true
}

/// `input_reply_clipboard` (`input.c:3476-3501`): `clip == 0` omits the
/// selector; a buffer of `INT_MAX * 3 / 4 - 1` bytes or more writes nothing.
pub fn clipboard(buf: &[u8], clip: u8, end: InputEnd, out: &mut Vec<u8>) {
    if !buf.is_empty() && buf.len() >= (i32::MAX as usize) * 3 / 4 - 1 {
        return;
    }
    out.extend_from_slice(b"\x1b]52;");
    if clip != 0 {
        out.push(clip);
    }
    out.push(b';');
    if !buf.is_empty() {
        out.extend_from_slice(&rmux_util::base64::ntop(buf).0);
    }
    out.extend_from_slice(end.bytes());
}

/// `input_report_current_theme` reply strings (`input.c:3726,3730`).
pub fn theme(theme: ClientTheme) -> Option<&'static [u8]> {
    match theme {
        ClientTheme::Dark => Some(b"\x1b[?997;1n"),
        ClientTheme::Light => Some(b"\x1b[?997;2n"),
        ClientTheme::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_formats() {
        let mut out = Vec::new();
        assert!(colour(
            4,
            Some(7),
            Colour::rgb(1, 2, 3),
            InputEnd::St,
            &mut out
        ));
        assert_eq!(out, b"\x1b]4;7;rgb:0101/0202/0303\x1b\\");
        out.clear();
        assert!(colour(11, None, Colour(1), InputEnd::Bel, &mut out));
        assert_eq!(out, b"\x1b]11;rgb:8080/0000/0000\x07");
        out.clear();
        assert!(!colour(10, None, Colour::NONE, InputEnd::Bel, &mut out));
        assert!(out.is_empty());
    }

    #[test]
    fn clipboard_formats() {
        let mut out = Vec::new();
        clipboard(b"hi", b'c', InputEnd::St, &mut out);
        assert_eq!(out, b"\x1b]52;c;aGk=\x1b\\");
        out.clear();
        clipboard(b"", 0, InputEnd::Bel, &mut out);
        assert_eq!(out, b"\x1b]52;;\x07");
    }
}
