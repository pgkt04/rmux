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

//! SGR numeric and colon forms (`input.c:2221-2526`).

use super::InputCtx;
use super::params::InputParam;
use crate::cell::{DEFAULT_CELL, GridAttributes, GridCell};
use crate::colour::{Colour, ColourFlags};
use rmux_util::strtonum::strtonum;

/// `input_csi_dispatch_sgr_256_do` (`input.c:2222-2240`).
fn sgr_256(gc: &mut GridCell, fgbg: i32, c: i32) {
    if c == -1 || c > 255 {
        match fgbg {
            38 => gc.fg = Colour::DEFAULT,
            48 => gc.bg = Colour::DEFAULT,
            _ => {}
        }
    } else {
        let c = Colour(c | ColourFlags::_256.bits() as i32);
        match fgbg {
            38 => gc.fg = c,
            48 => gc.bg = c,
            58 => gc.us = c,
            _ => {}
        }
    }
}

/// `input_csi_dispatch_sgr_rgb_do` (`input.c:2255-2274`).
fn sgr_rgb(gc: &mut GridCell, fgbg: i32, r: i32, g: i32, b: i32) -> bool {
    if !(0..=255).contains(&r) || !(0..=255).contains(&g) || !(0..=255).contains(&b) {
        return false;
    }
    let c = Colour::rgb(r as u8, g as u8, b as u8);
    match fgbg {
        38 => gc.fg = c,
        48 => gc.bg = c,
        58 => gc.us = c,
        _ => {}
    }
    true
}

fn set_underscore(gc: &mut GridCell, bit: GridAttributes) {
    gc.attr.remove(GridAttributes::ALL_UNDERSCORE);
    gc.attr.insert(bit);
}

impl InputCtx {
    /// `input_csi_dispatch_sgr_colon` (`input.c:2291-2375`).
    fn sgr_colon(&mut self, s: &[u8]) {
        let gc = &mut self.cell.cell;
        let mut p = [-1i32; 8];
        let mut n = 0usize;
        for out in s.split(|&b| b == b':') {
            if !out.is_empty() {
                match strtonum(out, 0, i64::from(i32::MAX)) {
                    Ok(v) => p[n] = v as i32,
                    Err(_) => return,
                }
                n += 1;
                if n == p.len() {
                    return;
                }
            } else {
                n += 1;
                if n == p.len() {
                    return;
                }
            }
        }
        if n == 0 {
            return;
        }
        if p[0] == 4 {
            if n != 2 {
                return;
            }
            match p[1] {
                0 => gc.attr.remove(GridAttributes::ALL_UNDERSCORE),
                1 => set_underscore(gc, GridAttributes::UNDERSCORE),
                2 => set_underscore(gc, GridAttributes::UNDERSCORE_2),
                3 => set_underscore(gc, GridAttributes::UNDERSCORE_3),
                4 => set_underscore(gc, GridAttributes::UNDERSCORE_4),
                5 => set_underscore(gc, GridAttributes::UNDERSCORE_5),
                _ => {}
            }
            return;
        }
        if n < 2 || (p[0] != 38 && p[0] != 48 && p[0] != 58) {
            return;
        }
        match p[1] {
            2 => {
                if n < 3 {
                    return;
                }
                let i = if n == 5 { 2 } else { 3 };
                if n < i + 3 {
                    return;
                }
                sgr_rgb(gc, p[0], p[i], p[i + 1], p[i + 2]);
            }
            5 => {
                if n < 3 {
                    return;
                }
                sgr_256(gc, p[0], p[2]);
            }
            _ => {}
        }
    }

    /// `input_csi_dispatch_sgr` (`input.c:2379-2526`).
    pub(super) fn sgr(&mut self) {
        if self.params.len() == 0 {
            self.cell.cell = DEFAULT_CELL;
            return;
        }
        let mut i = 0;
        while i < self.params.len() {
            if let Some(InputParam::Colon { start, len }) = self.params.raw(i) {
                let (start, len) = (usize::from(start), usize::from(len));
                let mut field = [0u8; 64];
                field[..len].copy_from_slice(&self.param_buf.as_slice()[start..start + len]);
                self.sgr_colon(&field[..len]);
                i += 1;
                continue;
            }
            let n = self.params.get(i, 0, 0);
            if n == -1 {
                i += 1;
                continue;
            }
            if n == 38 || n == 48 || n == 58 {
                i += 1;
                match self.params.get(i, 0, -1) {
                    2 => {
                        let r = self.params.get(i + 1, 0, -1);
                        let g = self.params.get(i + 2, 0, -1);
                        let b = self.params.get(i + 3, 0, -1);
                        if sgr_rgb(&mut self.cell.cell, n, r, g, b) {
                            i += 3;
                        }
                    }
                    5 => {
                        let c = self.params.get(i + 1, 0, -1);
                        sgr_256(&mut self.cell.cell, n, c);
                        i += 1;
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }
            let gc = &mut self.cell.cell;
            match n {
                0 => {
                    let link = gc.link;
                    *gc = DEFAULT_CELL;
                    gc.link = link;
                }
                1 => gc.attr.insert(GridAttributes::BRIGHT),
                2 => gc.attr.insert(GridAttributes::DIM),
                3 => gc.attr.insert(GridAttributes::ITALICS),
                4 => set_underscore(gc, GridAttributes::UNDERSCORE),
                5 | 6 => gc.attr.insert(GridAttributes::BLINK),
                7 => gc.attr.insert(GridAttributes::REVERSE),
                8 => gc.attr.insert(GridAttributes::HIDDEN),
                9 => gc.attr.insert(GridAttributes::STRIKETHROUGH),
                21 => set_underscore(gc, GridAttributes::UNDERSCORE_2),
                22 => gc.attr.remove(GridAttributes::BRIGHT | GridAttributes::DIM),
                23 => gc.attr.remove(GridAttributes::ITALICS),
                24 => gc.attr.remove(GridAttributes::ALL_UNDERSCORE),
                25 => gc.attr.remove(GridAttributes::BLINK),
                27 => gc.attr.remove(GridAttributes::REVERSE),
                28 => gc.attr.remove(GridAttributes::HIDDEN),
                29 => gc.attr.remove(GridAttributes::STRIKETHROUGH),
                30..=37 => gc.fg = Colour(n - 30),
                39 => gc.fg = Colour::DEFAULT,
                40..=47 => gc.bg = Colour(n - 40),
                49 => gc.bg = Colour::DEFAULT,
                53 => gc.attr.insert(GridAttributes::OVERLINE),
                55 => gc.attr.remove(GridAttributes::OVERLINE),
                59 => gc.us = Colour::DEFAULT,
                90..=97 => gc.fg = Colour(n),
                100..=107 => gc.bg = Colour(n - 10),
                _ => {}
            }
            i += 1;
        }
    }
}
