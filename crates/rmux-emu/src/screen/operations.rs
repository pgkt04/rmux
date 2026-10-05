// Ported from tmux screen-write.c @ 8f25579c
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

use super::write::{DrawCommand, ScreenRenderEffects, ScreenWriteCtx};
use super::{ScreenMode, ScreenResetPolicy};
use crate::cell::DEFAULT_CELL;
use crate::colour::Colour;
use crate::grid::{GridFlags, GridLineFlags};
use rmux_util::utf8::Utf8Data;

impl ScreenWriteCtx<'_> {
    pub fn reset(&mut self, policy: ScreenResetPolicy) {
        self.screen.reset_tabs();
        self.scrollregion(0, self.screen.grid.sy() - 1);
        self.screen.mode = ScreenMode::CURSOR | ScreenMode::WRAP;
        if policy.extended_keys {
            self.screen.mode.insert(ScreenMode::KEYS_EXTENDED);
        }
        self.clearscreen(Colour::DEFAULT);
        self.set_cursor(Some(0), Some(0));
    }

    pub fn cursorup(&mut self, count: u32) {
        let mut x = self.screen.cx;
        let y = self.screen.cy;
        let available = if y < self.screen.rupper {
            y
        } else {
            y - self.screen.rupper
        };
        let count = count.max(1).min(available);
        if x == self.screen.grid.sx() {
            x -= 1;
        }
        self.set_cursor(Some(x), Some(y - count));
    }

    pub fn cursordown(&mut self, count: u32) {
        let mut x = self.screen.cx;
        let y = self.screen.cy;
        let available = if y > self.screen.rlower {
            self.screen.grid.sy() - 1 - y
        } else {
            self.screen.rlower - y
        };
        let count = count.max(1).min(available);
        if x == self.screen.grid.sx() {
            x -= 1;
        } else if count == 0 {
            return;
        }
        self.set_cursor(Some(x), Some(y + count));
    }

    pub fn cursorright(&mut self, count: u32) {
        let x = self.screen.cx;
        // Pending wrap makes C's remaining-column subtraction underflow.
        let available = (self.screen.grid.sx() - 1).wrapping_sub(x);
        let count = count.max(1).min(available);
        if count != 0 {
            self.set_cursor(Some(x.wrapping_add(count)), Some(self.screen.cy));
        }
    }

    pub fn cursorleft(&mut self, count: u32) {
        let count = count.max(1).min(self.screen.cx);
        if count != 0 {
            self.set_cursor(Some(self.screen.cx - count), Some(self.screen.cy));
        }
    }

    pub fn backspace(&mut self) {
        let mut x = self.screen.cx;
        let mut y = self.screen.cy;
        if x == 0 {
            if y == 0 {
                return;
            }
            let line = self.screen.grid.get_line(self.screen.grid.hsize() + y - 1);
            if line.flags.contains(GridLineFlags::WRAPPED) {
                y -= 1;
                x = self.screen.grid.sx() - 1;
            }
        } else {
            x -= 1;
        }
        self.set_cursor(Some(x), Some(y));
    }

    pub fn cursormove(&mut self, x: i32, y: i32, origin: bool) {
        let x = (x != -1).then_some(x as u32);
        let mut y = (y != -1).then_some(y as u32);
        if origin && self.screen.mode.contains(ScreenMode::ORIGIN) {
            y = y.map(|y| {
                if y > self.screen.rlower - self.screen.rupper {
                    self.screen.rlower
                } else {
                    y + self.screen.rupper
                }
            });
        }
        self.set_cursor(
            x.map(|x| x.min(self.screen.grid.sx() - 1)),
            y.map(|y| y.min(self.screen.grid.sy() - 1)),
        );
    }

    pub fn scrollregion(&mut self, top: u32, bottom: u32) {
        let last = self.screen.grid.sy() - 1;
        let top = top.min(last);
        let bottom = bottom.min(last);
        if top >= bottom {
            return;
        }
        self.flush(false);
        self.set_cursor(Some(0), Some(0));
        self.screen.rupper = top;
        self.screen.rlower = bottom;
    }

    pub fn carriagereturn(&mut self) {
        self.set_cursor(Some(0), None);
    }

    fn prepare_scroll(&mut self, bg: Colour) {
        if bg != self.bg {
            self.flush(true);
            self.bg = bg;
        }
    }

    pub fn linefeed(&mut self, wrapped: bool, bg: Colour) {
        if wrapped {
            let y = self.screen.grid.hsize() + self.screen.cy;
            self.screen
                .grid
                .get_line_mut(y)
                .flags
                .insert(GridLineFlags::WRAPPED);
        }
        self.prepare_scroll(bg);
        if self.screen.cy != self.screen.rlower {
            if self.screen.cy < self.screen.grid.sy() - 1 {
                self.set_cursor(None, Some(self.screen.cy + 1));
            }
            return;
        }
        self.screen
            .grid
            .view_scroll_region_up(self.screen.rupper, self.screen.rlower, bg);
        self.scroll_collection(bg);
        self.scrolled = self.scrolled.wrapping_add(1);
    }

    pub fn scrollup(&mut self, count: u32, bg: Colour) {
        let count = count
            .max(1)
            .min(self.screen.rlower - self.screen.rupper + 1);
        self.prepare_scroll(bg);
        for _ in 0..count {
            self.screen
                .grid
                .view_scroll_region_up(self.screen.rupper, self.screen.rlower, bg);
            self.scroll_collection(bg);
        }
        self.scrolled = self.scrolled.wrapping_add(count);
    }

    pub fn scrolldown(&mut self, count: u32, bg: Colour) {
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        let count = count
            .max(1)
            .min(self.screen.rlower - self.screen.rupper + 1);
        for _ in 0..count {
            self.screen
                .grid
                .view_scroll_region_down(self.screen.rupper, self.screen.rlower, bg);
        }
        self.flush(false);
        let height = self.screen.rlower - self.screen.rupper + 1;
        if !self.should_draw(self.screen.rupper, height) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::ScrollDown { count, bg }, snapshot);
        } else {
            self.redraw_rows_snapshot(0, self.screen.grid.sy(), snapshot);
        }
    }

    pub fn reverseindex(&mut self, bg: Colour) {
        if self.screen.cy != self.screen.rupper {
            if self.screen.cy > 0 {
                self.set_cursor(None, Some(self.screen.cy - 1));
            }
            return;
        }
        self.screen
            .grid
            .view_scroll_region_down(self.screen.rupper, self.screen.rlower, bg);
        self.flush(false);
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        let height = self.screen.rlower - self.screen.rupper + 1;
        if !self.should_draw(self.screen.rupper, height) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::ReverseIndex { bg }, snapshot);
        } else {
            self.redraw_rows_snapshot(0, self.screen.grid.sy(), snapshot);
        }
    }

    pub fn alignmenttest(&mut self) {
        let mut cell = DEFAULT_CELL;
        cell.data = Utf8Data::set(b'E');
        let width = self.screen.grid.sx();
        let height = self.screen.grid.sy();
        for y in 0..height {
            for x in 0..width {
                self.screen.grid.view_set_cell(x, y, &cell);
            }
        }
        self.set_cursor(Some(0), Some(0));
        self.screen.rupper = 0;
        self.screen.rlower = height - 1;
        // The pinned call passes height - 1 as the count, not the last row.
        self.discard_rows(0, height - 1);
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        if !self.should_draw(0, height) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::AlignmentTest, snapshot);
        } else {
            self.redraw_rows_snapshot(0, height, snapshot);
        }
    }

    pub fn insertcharacter(&mut self, count: u32, bg: Colour) {
        let count = count.max(1).min(self.screen.grid.sx() - self.screen.cx);
        if count == 0 || self.screen.cx >= self.screen.grid.sx() {
            return;
        }
        let snapshot = self.snapshot(false);
        let obscured = self.obscured();
        self.screen
            .grid
            .view_insert_cells(self.screen.cx, self.screen.cy, count, bg);
        self.flush(false);
        if !self.should_draw(self.screen.cy, 1) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::InsertCharacter { count, bg }, snapshot);
        } else {
            self.redraw_rows_snapshot(self.screen.cy, 1, snapshot);
        }
    }

    pub fn deletecharacter(&mut self, count: u32, bg: Colour) {
        let count = count.max(1).min(self.screen.grid.sx() - self.screen.cx);
        if count == 0 || self.screen.cx >= self.screen.grid.sx() {
            return;
        }
        let snapshot = self.snapshot(false);
        let obscured = self.obscured();
        self.screen
            .grid
            .view_delete_cells(self.screen.cx, self.screen.cy, count, bg);
        self.flush(false);
        if !self.should_draw(self.screen.cy, 1) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::DeleteCharacter { count, bg }, snapshot);
        } else {
            self.redraw_rows_snapshot(self.screen.cy, 1, snapshot);
        }
    }

    pub fn clearcharacter(&mut self, count: u32, bg: Colour) {
        let count = count.max(1).min(self.screen.grid.sx() - self.screen.cx);
        if count == 0 || self.screen.cx >= self.screen.grid.sx() {
            return;
        }
        let snapshot = self.snapshot(false);
        let obscured = self.obscured();
        self.screen
            .grid
            .view_clear(self.screen.cx, self.screen.cy, count, 1, bg);
        self.flush(false);
        if !self.should_draw(self.screen.cy, 1) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::ClearCharacter { count, bg }, snapshot);
        } else {
            self.redraw_rows_snapshot(self.screen.cy, 1, snapshot);
        }
    }

    pub fn insertline(&mut self, count: u32, bg: Colour) {
        let y = self.screen.cy;
        let height = self.screen.grid.sy();
        let in_region = y >= self.screen.rupper && y <= self.screen.rlower;
        let affected = if in_region {
            self.screen.rlower + 1 - y
        } else {
            height - y
        };
        let count = count.max(1).min(affected);
        if count == 0 {
            return;
        }
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        if in_region {
            self.screen
                .grid
                .view_insert_lines_region(self.screen.rlower, y, count, bg);
        } else {
            self.screen.grid.view_insert_lines(y, count, bg);
        }
        self.flush(false);
        if !self.should_draw(y, affected) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::InsertLine { count, bg }, snapshot);
        } else {
            self.redraw_rows_snapshot(0, height, snapshot);
        }
    }

    pub fn deleteline(&mut self, count: u32, bg: Colour) {
        let y = self.screen.cy;
        let height = self.screen.grid.sy();
        let in_region = y >= self.screen.rupper && y <= self.screen.rlower;
        let affected = if in_region {
            self.screen.rlower + 1 - y
        } else {
            height - y
        };
        let count = count.max(1).min(affected);
        if count == 0 {
            return;
        }
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        if in_region {
            self.screen
                .grid
                .view_delete_lines_region(self.screen.rlower, y, count, bg);
        } else {
            self.screen.grid.view_delete_lines(y, count, bg);
        }
        self.flush(false);
        let (dirty_y, dirty_count) = if in_region {
            (y, affected)
        } else {
            // Unlike insertline, outside-region deletion dirties the region.
            (
                self.screen.rupper,
                self.screen.rlower + 1 - self.screen.rupper,
            )
        };
        if !self.should_draw(dirty_y, dirty_count) {
            return;
        }
        if !obscured || !self.policy.pane_backed {
            self.emit(DrawCommand::DeleteLine { count, bg }, snapshot);
        } else {
            self.redraw_rows_snapshot(0, height, snapshot);
        }
    }

    pub fn clearline(&mut self, bg: Colour) {
        let width = self.screen.grid.sx();
        let y = self.screen.cy;
        let absolute_y = self.screen.grid.hsize() + y;
        let line = self.screen.grid.get_line(absolute_y);
        if line.cellsize() == 0 && bg.is_default() {
            return;
        }
        let flags = line.flags & GridLineFlags::OSC133_FLAGS;
        let osc133 = line.osc133;
        self.screen.grid.view_clear(0, y, width, 1, bg);
        let line = self.screen.grid.get_line_mut(absolute_y);
        line.flags.insert(flags);
        line.osc133 = osc133;
        self.discard_rows(y, 1);
        self.insert_clear(0, y, width, bg);
    }

    pub fn clearendofline(&mut self, bg: Colour) {
        let x = self.screen.cx;
        if x == 0 {
            self.clearline(bg);
            return;
        }
        let width = self.screen.grid.sx();
        let y = self.screen.cy;
        let line = self.screen.grid.get_line(self.screen.grid.hsize() + y);
        if x >= width || (x >= line.cellsize() && bg.is_default()) {
            return;
        }
        self.screen.grid.view_clear(x, y, width - x, 1, bg);
        self.insert_clear(x, y, width - x, bg);
    }

    pub fn clearstartofline(&mut self, bg: Colour) {
        if self.screen.cx >= self.screen.grid.sx() - 1 {
            self.clearline(bg);
            return;
        }
        let count = self.screen.cx + 1;
        let y = self.screen.cy;
        self.screen.grid.view_clear(0, y, count, 1, bg);
        self.insert_clear(0, y, count, bg);
    }

    fn scroll_on_clear(&self) -> bool {
        self.screen.grid.flags.contains(GridFlags::HISTORY)
            && self.policy.pane_backed
            && self.policy.scroll_on_clear
    }

    fn collect_visible_clear(&mut self, x: u32, query_y: u32, count: u32, bg: Colour) {
        self.spans.clear();
        self.sink
            .visible_columns(x, query_y, count, &mut self.spans);
        for index in 0..self.spans.len() {
            let span = self.spans[index].clone();
            if span.start < span.end {
                self.insert_clear(span.start, self.screen.cy, span.end - span.start, bg);
            }
        }
    }

    pub fn clearendofscreen(&mut self, bg: Colour) {
        let width = self.screen.grid.sx();
        let height = self.screen.grid.sy();
        let x = self.screen.cx;
        let y = self.screen.cy;
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        if x == 0 && y == 0 && self.scroll_on_clear() {
            self.screen.grid.view_clear_history(bg);
        } else {
            if x < width {
                self.screen.grid.view_clear(x, y, width - x, 1, bg);
            }
            self.screen
                .grid
                .view_clear(0, y + 1, width, height - y - 1, bg);
        }
        self.discard_rows(y + 1, height - y - 1);
        self.flush(false);
        if !self.should_draw(y, height - y) {
            return;
        }
        if !obscured {
            self.emit(DrawCommand::ClearEndOfScreen { bg }, snapshot);
            return;
        }
        if x < width {
            self.collect_visible_clear(x, y, width - x, bg);
        }
        for row in y + 1..height {
            self.set_cursor(Some(0), Some(row));
            self.collect_visible_clear(0, row, width, bg);
        }
        self.set_cursor(Some(x), Some(y));
    }

    pub fn clearstartofscreen(&mut self, bg: Colour) {
        let width = self.screen.grid.sx();
        let x = self.screen.cx;
        let y = self.screen.cy;
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        if y > 0 {
            self.screen.grid.view_clear(0, 0, width, y, bg);
        }
        let count = if x >= width { width } else { x + 1 };
        self.screen.grid.view_clear(0, y, count, 1, bg);
        self.discard_rows(0, y);
        self.flush(false);
        if !self.should_draw(0, y + 1) {
            return;
        }
        if !obscured {
            self.emit(DrawCommand::ClearStartOfScreen { bg }, snapshot);
            return;
        }
        // The C loop compares against mutable cy, and its first setter homes cy.
        let mut row = 0;
        while row < self.screen.cy {
            self.set_cursor(Some(0), Some(row));
            self.collect_visible_clear(0, row, width, bg);
            row += 1;
        }
        self.set_cursor(Some(0), Some(self.screen.cy));
        self.collect_visible_clear(0, y, self.screen.cx + 1, bg);
        self.set_cursor(Some(x), Some(y));
    }

    pub fn clearscreen(&mut self, bg: Colour) {
        let width = self.screen.grid.sx();
        let height = self.screen.grid.sy();
        let snapshot = self.snapshot(true);
        let obscured = self.obscured();
        if self.scroll_on_clear() {
            self.screen.grid.view_clear_history(bg);
        } else {
            self.screen.grid.view_clear(0, 0, width, height, bg);
        }
        self.discard_rows(0, height);
        if !self.should_draw(0, height) {
            return;
        }
        if !obscured {
            self.emit(DrawCommand::ClearScreen { bg }, snapshot);
            return;
        }
        let x = self.screen.cx;
        let y = self.screen.cy;
        for row in 0..height {
            self.set_cursor(Some(0), Some(row));
            self.collect_visible_clear(0, row, width, bg);
        }
        self.set_cursor(Some(x), Some(y));
    }

    pub fn clearhistory(&mut self) {
        self.screen.grid.clear_history();
    }

    pub fn fullredraw(&mut self) {
        self.flush(false);
        self.snapshot(true);
        if self.policy.pane_backed {
            self.sink.effect(
                ScreenRenderEffects::DamageRows {
                    start: 0,
                    count: self.screen.grid.sy(),
                },
                self.screen,
            );
        }
    }
}
