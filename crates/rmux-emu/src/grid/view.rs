// Ported from tmux grid-view.c @ 8f25579c
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
//! Grid operations with coordinates relative to the visible area: each row
//! has `hsize` added (`grid-view.c:30-31`). The formulas use wrapping
//! unsigned arithmetic exactly as the C code; callers (screen-write) must
//! supply regions and counts inside the visible area.

use super::{Grid, GridStringFlags, StringCellsCtx};
use crate::cell::GridCell;
use crate::colour::Colour;

impl Grid {
    fn view_y(&self, y: u32) -> u32 {
        self.hsize().wrapping_add(y)
    }

    /// `grid_view_get_cell` (`grid-view.c:34-38`).
    pub fn view_get_cell(&self, px: u32, py: u32) -> GridCell {
        self.get_cell(px, self.view_y(py))
    }

    /// `grid_view_set_cell` (`grid-view.c:41-46`).
    pub fn view_set_cell(&mut self, px: u32, py: u32, gc: &GridCell) {
        let y = self.view_y(py);
        self.set_cell(px, y, gc);
    }

    /// `grid_view_set_padding` (`grid-view.c:49-53`).
    pub fn view_set_padding(&mut self, px: u32, py: u32, bg: Colour) {
        let y = self.view_y(py);
        self.set_padding(px, y, bg);
    }

    /// `grid_view_set_cells` (`grid-view.c:56-62`).
    pub fn view_set_cells(&mut self, px: u32, py: u32, gc: &GridCell, s: &[u8]) {
        let y = self.view_y(py);
        self.set_cells(px, y, gc, s);
    }

    /// `grid_view_clear_history` (`grid-view.c:65-91`).
    pub fn view_clear_history(&mut self, bg: Colour) {
        let mut last = 0;
        for yy in 0..self.sy() {
            if self.get_line(self.view_y(yy)).cellused() != 0 {
                last = yy + 1;
            }
        }
        if last == 0 {
            self.view_clear(0, 0, self.sx(), self.sy(), bg);
            return;
        }
        for _ in 0..last {
            self.collect_history(false);
            self.scroll_history(bg);
        }
        if last < self.sy() {
            self.view_clear(0, 0, self.sx(), self.sy() - last, bg);
        }
        self.hscrolled = 0;
    }

    /// `grid_view_clear` (`grid-view.c:94-102`).
    pub fn view_clear(&mut self, px: u32, py: u32, nx: u32, ny: u32, bg: Colour) {
        let y = self.view_y(py);
        self.clear(px, y, nx, ny, bg);
    }

    /// `grid_view_scroll_region_up` (`grid-view.c:105-123`).
    pub fn view_scroll_region_up(&mut self, rupper: u32, rlower: u32, bg: Colour) {
        if self.flags.intersects(super::GridFlags::HISTORY) {
            self.collect_history(false);
            if rupper == 0 && rlower == self.sy().wrapping_sub(1) {
                self.scroll_history(bg);
            } else {
                let (u, l) = (self.view_y(rupper), self.view_y(rlower));
                self.scroll_history_region(u, l, bg);
            }
        } else {
            let (u, l) = (self.view_y(rupper), self.view_y(rlower));
            self.move_lines(u, u.wrapping_add(1), l.wrapping_sub(u), bg);
        }
    }

    /// `grid_view_scroll_region_down` (`grid-view.c:126-134`).
    pub fn view_scroll_region_down(&mut self, rupper: u32, rlower: u32, bg: Colour) {
        let (u, l) = (self.view_y(rupper), self.view_y(rlower));
        self.move_lines(u.wrapping_add(1), u, l.wrapping_sub(u), bg);
    }

    /// `grid_view_insert_lines` (`grid-view.c:137-146`).
    pub fn view_insert_lines(&mut self, py: u32, ny: u32, bg: Colour) {
        let py = self.view_y(py);
        let sy = self.view_y(self.sy());
        self.move_lines(
            py.wrapping_add(ny),
            py,
            sy.wrapping_sub(py).wrapping_sub(ny),
            bg,
        );
    }

    /// `grid_view_insert_lines_region` (`grid-view.c:149-163`).
    pub fn view_insert_lines_region(&mut self, rlower: u32, py: u32, ny: u32, bg: Colour) {
        let rlower = self.view_y(rlower);
        let py = self.view_y(py);
        let ny2 = rlower.wrapping_add(1).wrapping_sub(py).wrapping_sub(ny);
        self.move_lines(rlower.wrapping_add(1).wrapping_sub(ny2), py, ny2, bg);
        self.clear(0, py.wrapping_add(ny2), self.sx(), ny.wrapping_sub(ny2), bg);
    }

    /// `grid_view_delete_lines` (`grid-view.c:166-177`).
    pub fn view_delete_lines(&mut self, py: u32, ny: u32, bg: Colour) {
        let py = self.view_y(py);
        let sy = self.view_y(self.sy());
        self.move_lines(
            py,
            py.wrapping_add(ny),
            sy.wrapping_sub(py).wrapping_sub(ny),
            bg,
        );
        self.clear(0, sy.wrapping_sub(ny), self.sx(), ny, bg);
    }

    /// `grid_view_delete_lines_region` (`grid-view.c:180-193`).
    pub fn view_delete_lines_region(&mut self, rlower: u32, py: u32, ny: u32, bg: Colour) {
        let rlower = self.view_y(rlower);
        let py = self.view_y(py);
        let ny2 = rlower.wrapping_add(1).wrapping_sub(py).wrapping_sub(ny);
        self.move_lines(py, py.wrapping_add(ny), ny2, bg);
        self.clear(0, py.wrapping_add(ny2), self.sx(), ny.wrapping_sub(ny2), bg);
    }

    /// `grid_view_insert_cells` (`grid-view.c:196-210`).
    pub fn view_insert_cells(&mut self, px: u32, py: u32, nx: u32, bg: Colour) {
        let py = self.view_y(py);
        let sx = self.sx();
        if px >= sx.wrapping_sub(1) {
            self.clear(px, py, 1, 1, bg);
        } else {
            self.move_cells(
                px.wrapping_add(nx),
                px,
                py,
                sx.wrapping_sub(px).wrapping_sub(nx),
                bg,
            );
        }
    }

    /// `grid_view_delete_cells` (`grid-view.c:213-225`).
    pub fn view_delete_cells(&mut self, px: u32, py: u32, nx: u32, bg: Colour) {
        let py = self.view_y(py);
        let sx = self.sx();
        self.move_cells(
            px,
            px.wrapping_add(nx),
            py,
            sx.wrapping_sub(px).wrapping_sub(nx),
            bg,
        );
        self.clear(sx.wrapping_sub(nx), py, nx, 1, bg);
    }

    /// `grid_view_string_cells` (`grid-view.c:228-235`): no sequences, no
    /// hyperlinks.
    pub fn view_string_cells(&self, px: u32, py: u32, nx: u32) -> Vec<u8> {
        let mut ctx = StringCellsCtx {
            last: None,
            flags: GridStringFlags(0),
            hyperlinks: None,
        };
        self.string_cells(px, self.view_y(py), nx, &mut ctx)
    }
}
