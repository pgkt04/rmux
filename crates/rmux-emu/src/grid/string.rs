// Ported from tmux grid.c @ 8f25579c
/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
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
//! `grid_string_cells` and the SGR delta (`grid.c:857-1271`). The code
//! scratch is one fixed 8192-byte buffer filled with `strlcat` semantics so
//! that truncation and the repeated final-cell code before the hyperlink
//! close match tmux byte for byte.

use super::{Grid, GridStringFlags};
use crate::cell::{GridAttributes, GridCell, GridCellFlags};
use crate::colour::{self, Colour, ColourFlags};
use crate::hyperlinks::{HyperlinkRegistry, Hyperlinks};

/// Arguments of `grid_string_cells` other than the position
/// (`grid.c:1179-1180`). `last` is the caller-owned previous cell; `None`
/// emits no sequences. `hyperlinks` stands for `s->hyperlinks`.
pub struct StringCellsCtx<'a> {
    pub last: Option<&'a mut GridCell>,
    pub flags: GridStringFlags,
    pub hyperlinks: Option<(&'a HyperlinkRegistry, &'a Hyperlinks)>,
}

const CODE_LEN: usize = 8192;

/// `char code[8192]` with `strlcat` appends: at most 8191 content bytes.
struct CodeBuf {
    buf: [u8; CODE_LEN],
    len: usize,
}

impl CodeBuf {
    fn new() -> Self {
        CodeBuf {
            buf: [0; CODE_LEN],
            len: 0,
        }
    }
    fn clear(&mut self) {
        self.len = 0;
    }
    fn cat(&mut self, s: &[u8]) {
        let room = (CODE_LEN - 1).saturating_sub(self.len);
        let n = s.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&s[..n]);
        self.len += n;
    }
    fn cat_int(&mut self, v: i32) {
        let mut tmp = [0u8; 12];
        let mut i = tmp.len();
        let neg = v < 0;
        let mut n = v.unsigned_abs();
        loop {
            i -= 1;
            tmp[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        if neg {
            i -= 1;
            tmp[i] = b'-';
        }
        self.cat(&tmp[i..]);
    }
    fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

fn has(c: Colour, f: ColourFlags) -> bool {
    c.0 & f.bits() as i32 != 0
}

/// `grid_string_cells_fg` (`grid.c:858-909`).
fn fg_values(gc: &GridCell, values: &mut [i32; 8]) -> usize {
    let fg = gc.fg;
    let mut n = 0;
    if has(fg, ColourFlags::THEME) {
        let c = colour::theme_terminal_colour((fg.0 & 0xff) as u32).0;
        values[0] = if c == 8 { 39 } else { c + 30 };
        n = 1;
    } else if has(fg, ColourFlags::_256) {
        values[..3].copy_from_slice(&[38, 5, fg.0 & 0xff]);
        n = 3;
    } else if has(fg, ColourFlags::RGB) {
        let (r, g, b) = fg.split_rgb();
        values[..5].copy_from_slice(&[38, 2, i32::from(r), i32::from(g), i32::from(b)]);
        n = 5;
    } else {
        match fg.0 {
            0..=7 => {
                values[0] = fg.0 + 30;
                n = 1;
            }
            8 => {
                values[0] = 39;
                n = 1;
            }
            90..=97 => {
                values[0] = fg.0;
                n = 1;
            }
            _ => {}
        }
    }
    n
}

/// `grid_string_cells_bg` (`grid.c:912-963`).
fn bg_values(gc: &GridCell, values: &mut [i32; 8]) -> usize {
    let bg = gc.bg;
    let mut n = 0;
    if has(bg, ColourFlags::THEME) {
        let c = colour::theme_terminal_colour((bg.0 & 0xff) as u32).0;
        values[0] = if c == 8 { 49 } else { c + 40 };
        n = 1;
    } else if has(bg, ColourFlags::_256) {
        values[..3].copy_from_slice(&[48, 5, bg.0 & 0xff]);
        n = 3;
    } else if has(bg, ColourFlags::RGB) {
        let (r, g, b) = bg.split_rgb();
        values[..5].copy_from_slice(&[48, 2, i32::from(r), i32::from(g), i32::from(b)]);
        n = 5;
    } else {
        match bg.0 {
            0..=7 => {
                values[0] = bg.0 + 40;
                n = 1;
            }
            8 => {
                values[0] = 49;
                n = 1;
            }
            90..=97 => {
                values[0] = bg.0 + 10;
                n = 1;
            }
            _ => {}
        }
    }
    n
}

/// `grid_string_cells_us` (`grid.c:966-1000`).
fn us_values(gc: &GridCell, values: &mut [i32; 8]) -> usize {
    let us = gc.us;
    if has(us, ColourFlags::THEME) {
        let c = colour::theme_terminal_colour((us.0 & 0xff) as u32).0;
        if c == 8 {
            values[0] = 59;
            1
        } else {
            values[..3].copy_from_slice(&[58, 5, c]);
            3
        }
    } else if has(us, ColourFlags::_256) {
        values[..3].copy_from_slice(&[58, 5, us.0 & 0xff]);
        3
    } else if has(us, ColourFlags::RGB) {
        let (r, g, b) = us.split_rgb();
        values[..5].copy_from_slice(&[58, 2, i32::from(r), i32::from(g), i32::from(b)]);
        5
    } else {
        0
    }
}

fn csi(buf: &mut CodeBuf, flags: GridStringFlags) {
    if flags.intersects(GridStringFlags::ESCAPE_SEQUENCES) {
        buf.cat(b"\\033[");
    } else {
        buf.cat(b"\x1b[");
    }
}

/// `grid_string_cells_add_code` (`grid.c:1004-1032`).
fn add_code(buf: &mut CodeBuf, reset: bool, newc: &[i32], oldc: &[i32], flags: GridStringFlags) {
    if newc.is_empty() {
        return;
    }
    if !reset && newc == oldc {
        return;
    }
    if reset && (newc[0] == 49 || newc[0] == 39) {
        return;
    }
    csi(buf, flags);
    for (i, v) in newc.iter().enumerate() {
        buf.cat_int(*v);
        if i + 1 < newc.len() {
            buf.cat(b";");
        }
    }
    buf.cat(b"m");
}

/// `grid_string_cells_add_hyperlink` (`grid.c:1034-1059`). Returns false
/// without changing the buffer when the link is too long for it.
fn add_hyperlink(buf: &mut CodeBuf, id: &[u8], uri: &[u8], flags: GridStringFlags) -> bool {
    if uri.len() + id.len() + 17 >= CODE_LEN {
        return false;
    }
    let escape = flags.intersects(GridStringFlags::ESCAPE_SEQUENCES);
    buf.cat(if escape { b"\\033]8;" } else { b"\x1b]8;" });
    if !id.is_empty() {
        buf.cat(b"id=");
        buf.cat(id);
        buf.cat(b";");
    } else {
        buf.cat(b";");
    }
    buf.cat(uri);
    buf.cat(if escape { b"\\033\\\\" } else { b"\x1b\\" });
    true
}

const ATTRS: [(GridAttributes, i32); 13] = [
    (GridAttributes::BRIGHT, 1),
    (GridAttributes::DIM, 2),
    (GridAttributes::ITALICS, 3),
    (GridAttributes::UNDERSCORE, 4),
    (GridAttributes::BLINK, 5),
    (GridAttributes::REVERSE, 7),
    (GridAttributes::HIDDEN, 8),
    (GridAttributes::STRIKETHROUGH, 9),
    (GridAttributes::UNDERSCORE_2, 42),
    (GridAttributes::UNDERSCORE_3, 43),
    (GridAttributes::UNDERSCORE_4, 44),
    (GridAttributes::UNDERSCORE_5, 45),
    (GridAttributes::OVERLINE, 53),
];

/// `grid_string_cells_code` (`grid.c:1065-1176`).
fn cells_code(
    lastgc: &GridCell,
    gc: &GridCell,
    buf: &mut CodeBuf,
    flags: GridStringFlags,
    hyperlinks: Option<(&HyperlinkRegistry, &Hyperlinks)>,
    has_link: &mut bool,
) {
    let attr = gc.attr;
    let mut lastattr = lastgc.attr;
    let mut s = [0i32; 14];
    let mut n = 0;

    for (mask, _) in ATTRS {
        if (!attr.intersects(mask) && lastattr.intersects(mask))
            || (lastgc.us != Colour::DEFAULT && gc.us == Colour::DEFAULT)
        {
            s[n] = 0;
            n += 1;
            lastattr = lastattr & GridAttributes::CHARSET;
            break;
        }
    }
    for (mask, code) in ATTRS {
        if attr.intersects(mask) && !lastattr.intersects(mask) {
            s[n] = code;
            n += 1;
        }
    }

    buf.clear();
    if n > 0 {
        csi(buf, flags);
        for (i, v) in s[..n].iter().enumerate() {
            if *v < 10 {
                buf.cat_int(*v);
            } else {
                buf.cat_int(*v / 10);
                buf.cat(b":");
                buf.cat_int(*v % 10);
            }
            if i + 1 < n {
                buf.cat(b";");
            }
        }
        buf.cat(b"m");
    }
    let reset = n != 0 && s[0] == 0;

    let mut newc = [0i32; 8];
    let mut oldc = [0i32; 8];
    let nn = fg_values(gc, &mut newc);
    let no = fg_values(lastgc, &mut oldc);
    add_code(buf, reset, &newc[..nn], &oldc[..no], flags);
    let nn = bg_values(gc, &mut newc);
    let no = bg_values(lastgc, &mut oldc);
    add_code(buf, reset, &newc[..nn], &oldc[..no], flags);
    let nn = us_values(gc, &mut newc);
    let no = us_values(lastgc, &mut oldc);
    add_code(buf, reset, &newc[..nn], &oldc[..no], flags);

    let escape = flags.intersects(GridStringFlags::ESCAPE_SEQUENCES);
    if attr.intersects(GridAttributes::CHARSET) && !lastattr.intersects(GridAttributes::CHARSET) {
        buf.cat(if escape { b"\\016" } else { b"\x0e" });
    }
    if !attr.intersects(GridAttributes::CHARSET) && lastattr.intersects(GridAttributes::CHARSET) {
        buf.cat(if escape { b"\\017" } else { b"\x0f" });
    }

    if let Some((registry, store)) = hyperlinks
        && lastgc.link != gc.link
    {
        if let Some(link) = registry.get(store, gc.link) {
            *has_link = add_hyperlink(buf, link.internal_id(), link.uri(), flags);
        } else if *has_link {
            add_hyperlink(buf, b"", b"", flags);
            *has_link = false;
        }
    }
}

impl Grid {
    /// `grid_string_cells` (`grid.c:1179-1271`). The result can contain raw
    /// cell bytes; a C caller stops at the first NUL.
    pub fn string_cells(&self, px: u32, py: u32, nx: u32, ctx: &mut StringCellsCtx) -> Vec<u8> {
        let flags = ctx.flags;
        let mut out = Vec::new();
        let mut code = CodeBuf::new();
        let mut has_link = false;

        let Some(gl) = self.peek_line(py) else {
            return out;
        };
        let end = if flags.intersects(GridStringFlags::EMPTY_CELLS) {
            gl.cellsize()
        } else {
            gl.cellused()
        };
        let mut last = ctx.last.as_deref_mut();
        let with_sequences = flags.intersects(GridStringFlags::WITH_SEQUENCES);
        for xx in px..px.wrapping_add(nx) {
            if xx >= end {
                break;
            }
            let gc = gl.get_cell(xx);
            if gc.flags.contains(GridCellFlags::PADDING) {
                continue;
            }
            if with_sequences && let Some(lastgc) = last.as_deref_mut() {
                cells_code(lastgc, &gc, &mut code, flags, ctx.hyperlinks, &mut has_link);
                out.extend_from_slice(code.bytes());
                *lastgc = gc;
            }
            if gc.flags.contains(GridCellFlags::TAB) {
                out.push(b'\t');
            } else {
                let data = gc.data.bytes();
                if flags.intersects(GridStringFlags::ESCAPE_SEQUENCES) && data == b"\\" {
                    out.extend_from_slice(b"\\\\");
                } else {
                    out.extend_from_slice(data);
                }
            }
        }

        if has_link {
            add_hyperlink(&mut code, b"", b"", flags);
            out.extend_from_slice(code.bytes());
        }

        if flags.intersects(GridStringFlags::TRIM_SPACES) {
            while out.last() == Some(&b' ') {
                out.pop();
            }
        }
        out
    }
}
