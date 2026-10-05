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

use super::ScreenMode;
use super::write::{DrawCommand, ScreenWriteCtx};
use crate::cell::{DEFAULT_CELL, GridCell, GridCellFlags};
use crate::colour::Colour;
use crate::grid::GridLineFlags;
use rmux_util::utf8::Utf8Data;
use rmux_util::utf8::combined::{
    HangulJamoState, hanguljamo_check_state, has_zwj, is_hangul_filler, is_vs, is_zwj,
    should_combine,
};

impl ScreenWriteCtx<'_> {
    pub fn cell(&mut self, cell: &GridCell) {
        if cell.flags.contains(GridCellFlags::PADDING) || self.combine(cell) {
            return;
        }
        self.flush(true);
        let width = u32::from(cell.data.width);
        let sx = self.screen.grid.sx();
        let sy = self.screen.grid.sy();
        let wrap = self.screen.mode.contains(ScreenMode::WRAP);
        if !wrap
            && width > 1
            && (width > sx || (self.screen.cx != sx && self.screen.cx > sx - width))
        {
            return;
        }

        let insert = self.screen.mode.contains(ScreenMode::INSERT);
        let mut skip = !insert;
        if insert {
            self.screen.grid.view_insert_cells(
                self.screen.cx,
                self.screen.cy,
                width,
                Colour::DEFAULT,
            );
        }
        if wrap && self.screen.cx > sx.wrapping_sub(width) {
            self.linefeed(true, Colour::DEFAULT);
            self.set_cursor(Some(0), None);
            self.flush(false);
        }
        if self.screen.cx > sx.wrapping_sub(width) || self.screen.cy > sy - 1 {
            return;
        }
        let snapshot = self.snapshot(false);
        let (x, y) = (self.screen.cx, self.screen.cy);
        let mut redraw = false;
        if self
            .screen
            .grid
            .get_line(self.screen.grid.hsize() + y)
            .flags
            .contains(GridLineFlags::EXTENDED)
        {
            let mut old = self.screen.grid.view_get_cell(x, y);
            redraw = self.overwrite(&mut old, width);
            if redraw {
                skip = false;
            }
        }
        for xx in x + 1..x + width {
            self.screen.grid.view_set_padding(xx, y, cell.bg);
            skip = false;
        }
        if skip {
            let line = self.screen.grid.get_line(self.screen.grid.hsize() + y);
            skip = if x >= line.cellsize() {
                cell.cells_equal(&DEFAULT_CELL)
            } else {
                let entry = &line.entries()[x as usize];
                let compact = entry.compact();
                !entry.is_extended()
                    && cell.flags == entry.flags()
                    && cell.attr.bits() == u16::from(compact.attr)
                    && cell.fg.0 == i32::from(compact.fg)
                    && cell.bg.0 == i32::from(compact.bg)
                    && cell.data.width == 1
                    && cell.data.size == 1
                    && compact.data == cell.data.data[0]
            };
        }
        let selected = self.screen.check_selection(x, y);
        if selected && !cell.flags.contains(GridCellFlags::SELECTED) {
            let mut stored = *cell;
            stored.flags.insert(GridCellFlags::SELECTED);
            self.screen.grid.view_set_cell(x, y, &stored);
        } else if !selected && cell.flags.contains(GridCellFlags::SELECTED) {
            let mut stored = *cell;
            stored.flags.remove(GridCellFlags::SELECTED);
            self.screen.grid.view_set_cell(x, y, &stored);
        } else if !skip {
            self.screen.grid.view_set_cell(x, y, cell);
        }
        if selected {
            skip = false;
        }
        let visible = self.fully_visible(x, y, width);
        let not_wrap = u32::from(!wrap);
        let next = if x <= sx.wrapping_sub(not_wrap).wrapping_sub(width) {
            x + width
        } else {
            sx - not_wrap
        };
        self.set_cursor(Some(next), None);

        if insert {
            self.flush(false);
            if self.policy.pane_backed && self.obscured() {
                if self.should_draw(y, 1) {
                    self.redraw_rows_snapshot(y, 1, snapshot);
                }
                return;
            }
            if self.should_draw(y, 1) {
                // C memsets the tty_ctx, so this command carries bg 0, not 8.
                self.emit(
                    DrawCommand::InsertCharacter {
                        count: width,
                        bg: Colour(0),
                    },
                    snapshot,
                );
            }
        }
        if skip || !self.should_draw(y, 1) {
            return;
        }
        if redraw && self.policy.pane_backed {
            self.redraw_rows_snapshot(y, 1, snapshot);
            return;
        }
        let mut display = if selected {
            self.screen.select_cell(cell)
        } else {
            *cell
        };
        if visible {
            if self.should_draw(y, 1) {
                self.emit(DrawCommand::Cell(&display), snapshot);
            }
            return;
        }
        display.data = Utf8Data::set(b' ');
        if !self.should_draw(y, 1) {
            return;
        }
        self.fully_visible(x, y, width);
        for i in 0..self.spans.len() {
            let (start, end) = (self.spans[i].start, self.spans[i].end);
            for xx in start..end {
                let mut positioned = snapshot;
                positioned.old_cx = xx;
                self.emit(DrawCommand::Cell(&display), positioned);
            }
        }
    }

    fn combine(&mut self, cell: &GridCell) -> bool {
        let data = &cell.data;
        if is_hangul_filler(data) {
            return true;
        }
        let mut force_wide = is_vs(data) && self.policy.variation_selector_always_wide;
        let zero_width = is_zwj(data) || is_vs(data) || data.width == 0;
        let (mut cx, cy) = (self.screen.cx, self.screen.cy);
        if data.size < 2 || cx == 0 {
            return zero_width;
        }
        let mut n = 1;
        let mut last = self.screen.grid.view_get_cell(cx - n, cy);
        if cx != 1 && last.flags.contains(GridCellFlags::PADDING) {
            n = 2;
            last = self.screen.grid.view_get_cell(cx - n, cy);
        }
        if n != u32::from(last.data.width) || last.flags.contains(GridCellFlags::PADDING) {
            return zero_width;
        }
        if !zero_width {
            match hanguljamo_check_state(&last.data, data) {
                HangulJamoState::NotComposable => return true,
                HangulJamoState::Choseong => return false,
                HangulJamoState::Composable => {}
                HangulJamoState::NotHangulJamo => {
                    if should_combine(&last.data, data) || should_combine(data, &last.data) {
                        force_wide = true;
                    } else if !has_zwj(&last.data) {
                        return false;
                    }
                }
            }
        }
        let end = usize::from(last.data.size) + usize::from(data.size);
        if end > last.data.data.len() {
            return zero_width;
        }
        self.flush(false);
        let start = usize::from(last.data.size);
        last.data.data[start..end].copy_from_slice(data.bytes());
        last.data.size = end as u8;
        if last.data.width == 1 && force_wide {
            last.data.width = 2;
            n = 2;
            cx += 1;
        } else {
            force_wide = false;
        }
        self.screen.grid.view_set_cell(cx - n, cy, &last);
        if force_wide {
            self.screen.grid.view_set_padding(cx - 1, cy, last.bg);
        }
        if !self.fully_visible(cx - n, cy, n) {
            return true;
        }
        self.set_cursor(Some(cx - n), Some(cy));
        let mut snapshot = self.snapshot(false);
        snapshot.invalidate_cursor = force_wide;
        if self.should_draw(cy, 1) {
            self.emit(DrawCommand::Cell(&last), snapshot);
        }
        self.set_cursor(Some(cx), Some(cy));
        true
    }

    pub(crate) fn clear_cell(&mut self, x: u32, y: u32) {
        let cell = GridCell {
            bg: self.screen.grid.view_get_cell(x, y).bg,
            ..DEFAULT_CELL
        };
        self.screen.grid.view_set_cell(x, y, &cell);
    }

    pub(crate) fn overwrite(&mut self, cell: &mut GridCell, width: u32) -> bool {
        let (x, y) = (self.screen.cx, self.screen.cy);
        let mut done = false;
        if cell.flags.contains(GridCellFlags::PADDING) {
            let mut xx = x;
            while xx > 0 {
                if !self
                    .screen
                    .grid
                    .view_get_cell(xx, y)
                    .flags
                    .contains(GridCellFlags::PADDING)
                {
                    break;
                }
                self.clear_cell(xx, y);
                xx -= 1;
            }
            self.clear_cell(xx, y);
            done = true;
        }
        if width != 1 || cell.data.width != 1 || cell.flags.contains(GridCellFlags::PADDING) {
            let mut xx = x.wrapping_add(width);
            while xx < self.screen.grid.sx() {
                if !self
                    .screen
                    .grid
                    .view_get_cell(xx, y)
                    .flags
                    .contains(GridCellFlags::PADDING)
                {
                    break;
                }
                self.clear_cell(xx, y);
                done = true;
                xx += 1;
            }
        }
        done
    }
}
