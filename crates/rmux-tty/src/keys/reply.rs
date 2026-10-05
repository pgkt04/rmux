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

//! Terminal reply parsers: OSC 52 clipboard, DECRPM synchronized update,
//! primary/secondary/extended device attributes, OSC 10/11 colours, OSC 4
//! palette and window-size reports (`tty-keys.c:675-740,1349-1893`).

use super::scan::{scan_u_pair, strtol, strtoul};
use super::{
    ClipboardReply, ColourReply, ColourTarget, DaFeatures, Discovery, KeyDecodeContext,
    PaletteReply, Recognition, SizeReply,
};
use crate::tty::TtyFlags;
use rmux_emu::colour::{Colour, parse_x11_colour};
use rmux_emu::input::InputRequestPaletteData;
use rmux_util::base64;
use rmux_util::bytes::{ByteString, cstr};

use Recognition::{Complete, NoMatch, Partial};

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// Match a literal prefix byte by byte: `Partial` when input ends inside it.
fn prefix(buf: &[u8], want: &[u8]) -> Recognition<()> {
    for (i, &w) in want.iter().enumerate() {
        if i == buf.len() {
            return Partial;
        }
        if buf[i] != w {
            return NoMatch;
        }
    }
    Complete(want.len(), ())
}

/// `colour_parseX11(tmp)` on the C string in `tmp`.
fn x11(tmp: &[u8]) -> Option<Colour> {
    parse_x11_colour(cstr(tmp)).ok()
}

/// `tty_keys_clipboard` (`tty-keys.c:1353-1461`).
pub fn clipboard(buf: &[u8], ctx: &KeyDecodeContext) -> Recognition<ClipboardReply> {
    match prefix(buf, b"\x1b]52;") {
        Complete(..) => {}
        Partial => return Partial,
        NoMatch => return NoMatch,
    }
    let len = buf.len();
    if len == 5 {
        return Partial;
    }
    let mut terminator = 0;
    let mut end = 5;
    while end < len {
        if buf[end] == BEL {
            terminator = 1;
            break;
        }
        if end > 5 && buf[end - 1] == ESC && buf[end] == b'\\' {
            terminator = 2;
            break;
        }
        end += 1;
    }
    if end == len {
        return Partial;
    }
    let size = end + 1;
    let query = ctx.flags.contains(TtyFlags::OSC52QUERY);
    let consumed = |data| {
        Complete(
            size,
            ClipboardReply {
                clip: 0,
                data,
                query,
            },
        )
    };

    let mut p = &buf[5..end - (terminator - 1)];
    let mut clip = 0u8;
    if p.len() >= 2 && p[0] != b';' && p[1] == b';' {
        clip = p[0];
    }
    while !p.is_empty() && p[0] != b';' {
        p = &p[1..];
    }
    if p.len() <= 1 {
        return consumed(None);
    }
    p = &p[1..];
    let Some(out) = base64::pton(p) else {
        return consumed(None);
    };
    Complete(
        size,
        ClipboardReply {
            clip,
            data: Some(ByteString(out)),
            query,
        },
    )
}

/// The shared DA/DA2 body (`tty-keys.c:1493-1516,1624-1647`): copy up to the
/// first lowercase letter (at most 128 bytes), require `c`, then split on
/// `;` and store each `strtoul` result in a C `char`.
fn da_parameters(buf: &[u8]) -> Recognition<([i8; 32], usize)> {
    let len = buf.len();
    let mut i = 0;
    while i < 128 {
        if 3 + i == len {
            return Partial;
        }
        if buf[3 + i].is_ascii_lowercase() {
            break;
        }
        i += 1;
    }
    if i == 128 || buf[3 + i] != b'c' {
        return NoMatch;
    }
    // strsep walks tmp as a C string: an embedded NUL ends the parameters.
    let tmp = cstr(&buf[3..3 + i]);
    let mut p = [0i8; 32];
    let mut n = 0;
    for field in tmp.split(|&c| c == b';') {
        let (value, end) = strtoul(field);
        p[n] = if end != field.len() {
            0
        } else {
            value as u8 as i8
        };
        n += 1;
        if n == p.len() {
            break;
        }
    }
    Complete(4 + i, (p, n))
}

/// `tty_keys_device_attributes` (`tty-keys.c:1467-1544`).
pub fn device_attributes(buf: &[u8], ctx: &KeyDecodeContext) -> Recognition<Discovery> {
    if ctx.flags.contains(TtyFlags::HAVEDA) {
        return NoMatch;
    }
    match prefix(buf, b"\x1b[?") {
        Complete(..) => {}
        Partial => return Partial,
        NoMatch => return NoMatch,
    }
    if buf.len() == 3 {
        return Partial;
    }
    let (size, (p, n)) = match da_parameters(buf) {
        Complete(size, v) => (size, v),
        Partial => return Partial,
        NoMatch => return NoMatch,
    };
    let mut features = DaFeatures::default();
    if (61..=65).contains(&p[0]) {
        for &v in &p[1..n] {
            match v {
                4 => features.sixel = true,
                21 => features.margins = true,
                28 => features.rectfill = true,
                52 => features.clipboard = true,
                _ => {}
            }
        }
    }
    Complete(size, Discovery::PrimaryDa { features })
}

/// `tty_keys_device_attributes2` (`tty-keys.c:1598-1671`).
pub fn device_attributes2(buf: &[u8], ctx: &KeyDecodeContext) -> Recognition<Discovery> {
    if ctx.flags.contains(TtyFlags::HAVEDA2) {
        return NoMatch;
    }
    match prefix(buf, b"\x1b[>") {
        Complete(..) => {}
        Partial => return Partial,
        NoMatch => return NoMatch,
    }
    if buf.len() == 3 {
        return Partial;
    }
    let (size, (p, _)) = match da_parameters(buf) {
        Complete(size, v) => (size, v),
        Partial => return Partial,
        NoMatch => return NoMatch,
    };
    let defaults = match p[0] as u8 {
        b'M' => Some("mintty"),
        b'T' => Some("tmux"),
        b'U' => Some("rxvt-unicode"),
        _ => None,
    };
    Complete(size, Discovery::SecondaryDa { defaults })
}

/// `tty_keys_sync` (`tty-keys.c:1550-1592`).
pub fn sync(buf: &[u8], ctx: &KeyDecodeContext) -> Recognition<Discovery> {
    if ctx.flags.contains(TtyFlags::HAVESYNC) {
        return NoMatch;
    }
    let mut i = match prefix(buf, b"\x1b[?2026;") {
        Complete(n, ()) => n,
        Partial => return Partial,
        NoMatch => return NoMatch,
    };
    let len = buf.len();
    if i == len {
        return Partial;
    }
    if !(b'0'..=b'4').contains(&buf[i]) {
        return NoMatch;
    }
    let status = buf[i] - b'0';
    i += 1;
    if i == len {
        return Partial;
    }
    if buf[i] != b'$' {
        return NoMatch;
    }
    i += 1;
    if i == len {
        return Partial;
    }
    if buf[i] != b'y' {
        return NoMatch;
    }
    i += 1;
    Complete(
        i,
        Discovery::Sync {
            sync: matches!(status, 1..=3),
        },
    )
}

/// `tty_keys_extended_device_attributes` (`tty-keys.c:1677-1748`).
pub fn extended_device_attributes(buf: &[u8], ctx: &KeyDecodeContext) -> Recognition<Discovery> {
    if ctx.flags.contains(TtyFlags::HAVEXDA) {
        return NoMatch;
    }
    match prefix(buf, b"\x1bP>|") {
        Complete(..) => {}
        Partial => return Partial,
        NoMatch => return NoMatch,
    }
    let len = buf.len();
    if len == 4 {
        return Partial;
    }
    let mut i = 0;
    while i < 127 {
        if 4 + i == len {
            return Partial;
        }
        if buf[4 + i - 1] == ESC && buf[4 + i] == b'\\' {
            break;
        }
        i += 1;
    }
    if i == 127 {
        return NoMatch;
    }
    let size = 5 + i;
    if i == 0 {
        return Complete(
            size,
            Discovery::ExtendedDa {
                defaults: None,
                term_type: None,
            },
        );
    }
    // tmp[i - 1] = '\0' drops the ESC of the ST; xstrdup then stops at the
    // first NUL inside the text.
    let text = cstr(&buf[4..4 + i - 1]);
    let defaults = [
        ("iTerm2 ", "iTerm2"),
        ("tmux ", "tmux"),
        ("XTerm(", "XTerm"),
        ("mintty ", "mintty"),
        ("foot(", "foot"),
        ("WezTerm ", "WezTerm"),
        ("ghostty ", "ghostty"),
        ("Rio ", "Rio"),
    ]
    .iter()
    .find(|(p, _)| text.starts_with(p.as_bytes()))
    .map(|(_, name)| *name);
    Complete(
        size,
        Discovery::ExtendedDa {
            defaults,
            term_type: Some(ByteString(text.to_vec())),
        },
    )
}

/// The shared OSC body scan (`tty-keys.c:1788-1805,1859-1876`): copy up to
/// ST or BEL within a 128-byte scratch array, then cut the ST's ESC. Returns
/// the consumed size and the C string payload; `None` when the terminator
/// came first (`i == 0`), which the callers accept without parsing.
fn osc_payload(buf: &[u8], start: usize) -> Recognition<Option<&[u8]>> {
    let len = buf.len();
    let mut i = 0;
    while i < 127 {
        if start + i == len {
            return Partial;
        }
        if buf[start + i - 1] == ESC && buf[start + i] == b'\\' {
            break;
        }
        if buf[start + i] == BEL {
            break;
        }
        i += 1;
    }
    if i == 127 {
        return NoMatch;
    }
    let size = start + 1 + i;
    if i == 0 {
        return Complete(size, None);
    }
    let mut tmp = &buf[start..start + i];
    if tmp[i - 1] == ESC {
        tmp = &tmp[..i - 1];
    }
    Complete(size, Some(cstr(tmp)))
}

/// `tty_keys_colours` (`tty-keys.c:1754-1826`): OSC 10 and OSC 11 replies.
/// Also the public `parse_colour_response` for refresh-client reports.
pub fn colours(buf: &[u8]) -> Recognition<ColourReply> {
    match prefix(buf, b"\x1b]1") {
        Complete(..) => {}
        Partial => return Partial,
        NoMatch => return NoMatch,
    }
    let len = buf.len();
    if len == 3 {
        return Partial;
    }
    let target = match buf[3] {
        b'0' => ColourTarget::Foreground,
        b'1' => ColourTarget::Background,
        _ => return NoMatch,
    };
    if len == 4 {
        return Partial;
    }
    if buf[4] != b';' {
        return NoMatch;
    }
    if len == 5 {
        return Partial;
    }
    let (size, tmp) = match osc_payload(buf, 5) {
        Complete(size, tmp) => (size, tmp),
        Partial => return Partial,
        NoMatch => return NoMatch,
    };
    let colour = tmp.and_then(x11);
    Complete(size, ColourReply { target, colour })
}

/// `tty_keys_palette` (`tty-keys.c:1829-1893`).
pub fn palette(buf: &[u8]) -> Recognition<PaletteReply> {
    match prefix(buf, b"\x1b]4;") {
        Complete(..) => {}
        Partial => return Partial,
        NoMatch => return NoMatch,
    }
    if buf.len() == 4 {
        return Partial;
    }
    let (size, tmp) = match osc_payload(buf, 4) {
        Complete(size, tmp) => (size, tmp),
        Partial => return Partial,
        NoMatch => return NoMatch,
    };
    let Some(tmp) = tmp else {
        return Complete(size, PaletteReply { reply: None });
    };
    let (idx, end) = strtol(tmp);
    if tmp.get(end) != Some(&b';') {
        return NoMatch;
    }
    let idx = idx as i32;
    if !(0..=255).contains(&idx) {
        return NoMatch;
    }
    let reply = x11(&tmp[end + 1..]).map(|c| InputRequestPaletteData { idx, c });
    Complete(size, PaletteReply { reply })
}

/// `tty_keys_winsz` (`tty-keys.c:676-740`).
pub fn winsz(buf: &[u8], ctx: &KeyDecodeContext) -> Recognition<SizeReply> {
    if !ctx.flags.contains(TtyFlags::WINSIZEQUERY) {
        return NoMatch;
    }
    match prefix(buf, b"\x1b[") {
        Complete(..) => {}
        Partial => return Partial,
        NoMatch => return NoMatch,
    }
    let len = buf.len();
    if len == 2 {
        return Partial;
    }
    let mut end = 2;
    while end < len && end != 64 {
        if buf[end] == b't' {
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
    if end == 64 || buf[end] != b't' {
        return NoMatch;
    }
    let tmp = &buf[2..end];
    if let Some((sy, sx)) = scan_u_pair(tmp, b"8;") {
        return Complete(
            end + 1,
            SizeReply {
                sx,
                sy,
                xpixel: ctx.xpixel,
                ypixel: ctx.ypixel,
                invalidate: false,
                clear_query: false,
            },
        );
    }
    if let Some((ypixel, xpixel)) = scan_u_pair(tmp, b"4;") {
        let char_x = if xpixel != 0 && ctx.sx != 0 {
            xpixel / ctx.sx
        } else {
            0
        };
        let char_y = if ypixel != 0 && ctx.sy != 0 {
            ypixel / ctx.sy
        } else {
            0
        };
        return Complete(
            end + 1,
            SizeReply {
                sx: ctx.sx,
                sy: ctx.sy,
                xpixel: char_x,
                ypixel: char_y,
                invalidate: true,
                clear_query: true,
            },
        );
    }
    NoMatch
}
