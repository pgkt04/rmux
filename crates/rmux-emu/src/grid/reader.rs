// Ported from tmux grid-reader.c @ 8f25579c
/*
 * Copyright (c) 2020 Anindya Mukherjee <anindya49@hotmail.com>
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
//! A virtual cursor over a grid in absolute rows, for copy mode motions
//! (`grid-reader.c:23-438`). The cursor can rest on padding after a wide
//! character, as in tmux.

use super::{Grid, GridLineFlags};
use crate::cell::{GridCell, GridCellFlags};
use rmux_util::utf8::Utf8Data;

/// `WHITESPACE` (`tmux.h:673`).
pub const WHITESPACE: &[u8] = b"\t ";

pub struct GridReader<'a> {
    gd: &'a Grid,
    cx: u32,
    cy: u32,
}

/// `grid_reader_cell_equals_data` (`grid-reader.c:339-349`).
fn cell_equals_data(gc: &GridCell, ud: &Utf8Data) -> bool {
    if gc.flags.contains(GridCellFlags::PADDING) {
        return false;
    }
    if gc.flags.contains(GridCellFlags::TAB) && ud.size == 1 && ud.data[0] == b'\t' {
        return true;
    }
    gc.data.size == ud.size && gc.data.bytes() == ud.bytes()
}

impl<'a> GridReader<'a> {
    /// `grid_reader_start`.
    pub fn new(gd: &'a Grid, cx: u32, cy: u32) -> Self {
        GridReader { gd, cx, cy }
    }

    /// `grid_reader_get_cursor`.
    pub fn cursor(&self) -> (u32, u32) {
        (self.cx, self.cy)
    }

    fn last_row(&self) -> u32 {
        self.gd.hsize() + self.gd.sy() - 1
    }

    fn is_padding(&self, cx: u32, cy: u32) -> bool {
        self.gd
            .get_cell(cx, cy)
            .flags
            .contains(GridCellFlags::PADDING)
    }

    fn line_wrapped(&self, cy: u32) -> bool {
        self.gd
            .get_line(cy)
            .flags
            .intersects(GridLineFlags::WRAPPED)
    }

    /// `grid_reader_line_length`.
    pub fn line_length(&self) -> u32 {
        self.gd.line_length(self.cy)
    }

    /// `grid_reader_cursor_right` (`grid-reader.c:47-72`).
    pub fn cursor_right(&mut self, wrap: bool, all: bool, onemore: bool) {
        let px = if all {
            self.gd.sx()
        } else if onemore {
            self.line_length()
        } else {
            self.gd.line_limit(self.cy)
        };

        if wrap && self.cx >= px && self.cy < self.last_row() {
            self.cursor_start_of_line(false);
            self.cursor_down();
        } else if self.cx < px {
            self.cx += 1;
            while self.cx < px {
                if !self.is_padding(self.cx, self.cy) {
                    break;
                }
                self.cx += 1;
            }
        }
    }

    /// `grid_reader_cursor_left` (`grid-reader.c:75-93`).
    pub fn cursor_left(&mut self, wrap: bool) {
        while self.cx > 0 {
            if !self.is_padding(self.cx, self.cy) {
                break;
            }
            self.cx -= 1;
        }
        if self.cx == 0 && self.cy > 0 && (wrap || self.line_wrapped(self.cy - 1)) {
            self.cursor_up();
            self.cursor_end_of_line(false, false);
        } else if self.cx > 0 {
            self.cx -= 1;
        }
    }

    /// `grid_reader_cursor_down` (`grid-reader.c:96-109`).
    pub fn cursor_down(&mut self) {
        if self.cy < self.last_row() {
            self.cy += 1;
        }
        while self.cx > 0 {
            if !self.is_padding(self.cx, self.cy) {
                break;
            }
            self.cx -= 1;
        }
    }

    /// `grid_reader_cursor_up` (`grid-reader.c:112-125`).
    pub fn cursor_up(&mut self) {
        if self.cy > 0 {
            self.cy -= 1;
        }
        while self.cx > 0 {
            if !self.is_padding(self.cx, self.cy) {
                break;
            }
            self.cx -= 1;
        }
    }

    /// `grid_reader_cursor_start_of_line` (`grid-reader.c:128-138`).
    pub fn cursor_start_of_line(&mut self, wrap: bool) {
        if wrap {
            while self.cy > 0 && self.line_wrapped(self.cy - 1) {
                self.cy -= 1;
            }
        }
        self.cx = 0;
    }

    /// `grid_reader_cursor_end_of_line` (`grid-reader.c:141-156`).
    pub fn cursor_end_of_line(&mut self, wrap: bool, all: bool) {
        if wrap {
            let yy = self.last_row();
            while self.cy < yy && self.line_wrapped(self.cy) {
                self.cy += 1;
            }
        }
        if all {
            self.cx = self.gd.sx();
        } else {
            self.cx = self.line_length();
        }
    }

    /// `grid_reader_handle_wrap` (`grid-reader.c:159-179`): false when the
    /// cursor would wrap past the bottom of the grid.
    fn handle_wrap(&mut self, xx: &mut u32, yy: u32) -> bool {
        while self.cx > *xx {
            if self.cy == yy {
                return false;
            }
            self.cursor_start_of_line(false);
            self.cursor_down();
            if self.line_wrapped(self.cy) {
                *xx = self.gd.sx() - 1;
            } else {
                *xx = self.line_length();
            }
        }
        true
    }

    /// `grid_reader_in_set`: a width (`grid-reader.c:182-186`).
    pub fn in_set(&self, set: &[u8]) -> u32 {
        self.gd.in_set(self.cx, self.cy, set)
    }

    fn word_bound(&self) -> u32 {
        if self.line_wrapped(self.cy) {
            self.gd.sx() - 1
        } else {
            self.line_length()
        }
    }

    /// `grid_reader_cursor_next_word` (`grid-reader.c:189-231`).
    pub fn cursor_next_word(&mut self, separators: &[u8]) {
        let mut xx = self.word_bound();
        let yy = self.last_row();

        if !self.handle_wrap(&mut xx, yy) {
            return;
        }
        if self.in_set(WHITESPACE) == 0 {
            if self.in_set(separators) != 0 {
                loop {
                    self.cx += 1;
                    if !(self.handle_wrap(&mut xx, yy)
                        && self.in_set(separators) != 0
                        && self.in_set(WHITESPACE) == 0)
                    {
                        break;
                    }
                }
            } else {
                loop {
                    self.cx += 1;
                    if !(self.handle_wrap(&mut xx, yy)
                        && !(self.in_set(separators) != 0 || self.in_set(WHITESPACE) != 0))
                    {
                        break;
                    }
                }
            }
        }
        while self.handle_wrap(&mut xx, yy) {
            let width = self.in_set(WHITESPACE);
            if width == 0 {
                break;
            }
            self.cx += width;
        }
    }

    /// `grid_reader_cursor_next_word_end` (`grid-reader.c:234-276`).
    pub fn cursor_next_word_end(&mut self, separators: &[u8]) {
        let mut xx = self.word_bound();
        let yy = self.last_row();

        while self.handle_wrap(&mut xx, yy) {
            if self.in_set(WHITESPACE) != 0 {
                self.cx += 1;
            } else if self.in_set(separators) != 0 {
                loop {
                    self.cx += 1;
                    if !(self.handle_wrap(&mut xx, yy)
                        && self.in_set(separators) != 0
                        && self.in_set(WHITESPACE) == 0)
                    {
                        break;
                    }
                }
                return;
            } else {
                loop {
                    self.cx += 1;
                    if !(self.handle_wrap(&mut xx, yy)
                        && !(self.in_set(WHITESPACE) != 0 || self.in_set(separators) != 0))
                    {
                        break;
                    }
                }
                return;
            }
        }
    }

    /// `grid_reader_cursor_previous_word` (`grid-reader.c:279-336`).
    pub fn cursor_previous_word(&mut self, separators: &[u8], already: bool, stop_at_eol: bool) {
        let word_is_letters: u32;
        if already || self.in_set(WHITESPACE) != 0 {
            loop {
                if self.cx > 0 {
                    self.cx -= 1;
                    if self.in_set(WHITESPACE) == 0 {
                        word_is_letters = u32::from(self.in_set(separators) == 0);
                        break;
                    }
                } else {
                    if self.cy == 0 {
                        return;
                    }
                    self.cursor_up();
                    self.cursor_end_of_line(false, false);

                    if stop_at_eol && self.cx > 0 {
                        let oldx = self.cx;
                        self.cx -= 1;
                        let at_eol = self.in_set(WHITESPACE) != 0;
                        self.cx = oldx;
                        if at_eol {
                            word_is_letters = 0;
                            break;
                        }
                    }
                }
            }
        } else {
            word_is_letters = u32::from(self.in_set(separators) == 0);
        }

        let (mut oldx, mut oldy);
        loop {
            oldx = self.cx;
            oldy = self.cy;
            if self.cx == 0 {
                if self.cy == 0 || !self.line_wrapped(self.cy - 1) {
                    break;
                }
                self.cursor_up();
                self.cursor_end_of_line(false, true);
            }
            if self.cx > 0 {
                self.cx -= 1;
            }
            if !(self.in_set(WHITESPACE) == 0 && word_is_letters != self.in_set(separators)) {
                break;
            }
        }
        self.cx = oldx;
        self.cy = oldy;
    }

    /// `grid_reader_cursor_jump` (`grid-reader.c:352-380`): forward within
    /// the wrapped chain, including the current cell.
    pub fn cursor_jump(&mut self, jc: &Utf8Data) -> bool {
        let mut px = self.cx;
        let yy = self.last_row();

        let mut py = self.cy;
        while py <= yy {
            let xx = self.gd.line_length(py);
            while px < xx {
                if cell_equals_data(&self.gd.get_cell(px, py), jc) {
                    self.cx = px;
                    self.cy = py;
                    return true;
                }
                px += 1;
            }
            if py == yy || !self.line_wrapped(py) {
                return false;
            }
            px = 0;
            py += 1;
        }
        false
    }

    /// `grid_reader_cursor_jump_back` (`grid-reader.c:383-407`).
    pub fn cursor_jump_back(&mut self, jc: &Utf8Data) -> bool {
        let mut xx = self.cx + 1;

        let mut py = self.cy + 1;
        while py > 0 {
            let mut px = xx;
            while px > 0 {
                if cell_equals_data(&self.gd.get_cell(px - 1, py - 1), jc) {
                    self.cx = px - 1;
                    self.cy = py - 1;
                    return true;
                }
                px -= 1;
            }
            if py == 1 || !self.line_wrapped(py - 2) {
                return false;
            }
            xx = self.gd.line_length(py - 2);
            py -= 1;
        }
        false
    }

    /// `grid_reader_cursor_back_to_indentation` (`grid-reader.c:410-438`).
    pub fn cursor_back_to_indentation(&mut self) {
        let yy = self.last_row();
        let oldx = self.cx;
        let oldy = self.cy;
        self.cursor_start_of_line(true);

        let mut py = self.cy;
        while py <= yy {
            let xx = self.gd.line_length(py);
            for px in 0..xx {
                let gc = self.gd.get_cell(px, py);
                if (gc.data.size != 1 || gc.data.data[0] != b' ')
                    && !gc.flags.contains(GridCellFlags::TAB)
                    && !gc.flags.contains(GridCellFlags::PADDING)
                {
                    self.cx = px;
                    self.cy = py;
                    return;
                }
            }
            if !self.line_wrapped(py) {
                break;
            }
            py += 1;
        }
        self.cx = oldx;
        self.cy = oldy;
    }
}
