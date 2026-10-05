// Ported from tmux screen-write.c @ 8f25579c
use super::ScreenMode;
use super::write::{
    DrawCommand, ScreenRenderEffects, ScreenWriteCtx, ScreenWriteItem, ScreenWriteItemKind,
};
use crate::cell::{GridAttributes, GridCell, GridCellFlags};
use crate::colour::Colour;
impl ScreenWriteCtx<'_> {
    pub(crate) fn insert_item(&mut self, y: u32, mut item: ScreenWriteItem) {
        if item.used == 0 {
            return;
        }
        let start = item.x;
        let end = start + item.used;
        let items = &mut self.screen.write_list[y as usize].items;
        let mut i = 0;
        while i < items.len() {
            let a = items[i].x;
            let b = a + items[i].used;
            if b <= start {
                i += 1;
                continue;
            }
            if a >= end {
                break;
            }
            if a >= start && b <= end {
                if a == 0 && items[i].wrapped {
                    item.wrapped = true;
                }
                items.remove(i);
                continue;
            }
            if a < start && b <= end {
                items[i].used = start - a;
                i += 1;
                continue;
            }
            if a >= start && b > end {
                items[i].x = end;
                items[i].used = b - end;
                break;
            }
            let mut right = items[i].clone();
            right.x = end;
            right.used = b - end;
            right.wrapped = false;
            items[i].used = start - a;
            items.insert(i + 1, right);
            i += 1;
            break;
        }
        items.insert(i, item);
    }
    pub(crate) fn insert_clear(&mut self, x: u32, y: u32, count: u32, bg: Colour) {
        self.insert_item(
            y,
            ScreenWriteItem {
                x,
                used: count,
                kind: ScreenWriteItemKind::Clear,
                bg,
                ..ScreenWriteItem::default()
            },
        );
    }
    pub(crate) fn discard_rows(&mut self, y: u32, count: u32) {
        for row in &mut self.screen.write_list[y as usize..(y + count) as usize] {
            row.items.clear();
        }
    }
    pub(crate) fn scroll_collection(&mut self, bg: Colour) {
        let top = self.screen.rupper as usize;
        let bottom = self.screen.rlower as usize;
        self.screen.write_list[top].items.clear();
        self.screen.write_list[top..=bottom].rotate_left(1);
        self.insert_clear(0, bottom as u32, self.screen.grid.sx(), bg);
    }
    pub(crate) fn insert_clears(&mut self, x: u32, n: u32) {
        let mut start = x;
        let mut bg = Colour::DEFAULT;
        for xx in x..x + n {
            let cell = self.screen.grid.view_get_cell(xx, self.screen.cy);
            if xx == start {
                bg = cell.bg;
            } else if cell.bg != bg {
                self.insert_clear(start, self.screen.cy, xx - start, bg);
                start = xx;
                bg = cell.bg;
            }
        }
        self.insert_clear(start, self.screen.cy, x + n - start, bg);
    }
    pub fn collect_end(&mut self) {
        if self.item.used == 0 {
            return;
        }
        let mut item = std::mem::take(&mut self.item);
        let x = self.screen.cx;
        let y = self.screen.cy;
        item.x = x;
        let used = item.used;
        let gc = item.gc;
        self.insert_item(y, item);
        let mut bx = x;
        if x != 0 {
            while bx > 0 {
                let c = self.screen.grid.view_get_cell(bx, y);
                if !c.flags.contains(GridCellFlags::PADDING) {
                    break;
                }
                self.clear_cell(bx, y);
                bx -= 1;
            }
            if bx != x {
                let c = self.screen.grid.view_get_cell(bx, y);
                if c.data.width > 1 || c.flags.contains(GridCellFlags::PADDING) {
                    self.clear_cell(bx, y);
                }
            }
        }
        let mut row = std::mem::take(&mut self.screen.write_list[y as usize].data);
        self.screen
            .grid
            .view_set_cells(x, y, &gc, &row[x as usize..(x + used) as usize]);
        self.screen.write_list[y as usize].data = std::mem::take(&mut row);
        if bx != x {
            self.insert_clears(bx, x - bx);
        }
        self.set_cursor(Some(x + used), None);
        let mut xx = self.screen.cx;
        while xx < self.screen.grid.sx() {
            let c = self.screen.grid.view_get_cell(xx, y);
            if !c.flags.contains(GridCellFlags::PADDING) {
                break;
            }
            self.clear_cell(xx, y);
            xx += 1;
        }
        if xx != self.screen.cx {
            self.insert_clears(self.screen.cx, xx - self.screen.cx);
        }
    }
    pub fn collect_add(&mut self, gc: &GridCell) {
        let width = self.screen.grid.sx();
        let collect = gc.data.width == 1
            && gc.data.size == 1
            && gc.data.data[0] < 0x7f
            && !gc.flags.contains(GridCellFlags::TAB)
            && !gc.attr.contains(GridAttributes::CHARSET)
            && !self.screen.mode.contains(ScreenMode::INSERT)
            && self.screen.selection.is_none()
            && (self.screen.mode.contains(ScreenMode::WRAP)
                || self.screen.cx + self.item.used < width - 1);
        if !collect {
            self.collect_end();
            self.flush(false);
            self.cell(gc);
            return;
        }
        if self.screen.cx >= width || self.item.used > width - 1 - self.screen.cx {
            self.collect_end();
        }
        if self.screen.cx >= width {
            self.item.wrapped = true;
            self.linefeed(true, Colour::DEFAULT);
            self.set_cursor(Some(0), None);
        }
        if self.item.used == 0 {
            self.item.gc = *gc;
        }
        let row = &mut self.screen.write_list[self.screen.cy as usize];
        if row.data.is_empty() {
            row.data.resize(width as usize, 0);
        }
        row.data[(self.screen.cx + self.item.used) as usize] = gc.data.data[0];
        self.item.used += 1;
    }
    pub(crate) fn flush(&mut self, scroll_only: bool) {
        let height = self.screen.grid.sy();
        if self.sink.redraw_pending() {
            self.discard_rows(0, height);
            self.scrolled = 0;
            self.bg = Colour::DEFAULT;
            return;
        }
        if self.screen.mode.contains(ScreenMode::SYNC) {
            if self.policy.pane_backed && self.scrolled != 0 {
                self.sink.effect(
                    ScreenRenderEffects::DeferredScroll {
                        count: self.scrolled,
                        rupper: self.screen.rupper,
                        rlower: self.screen.rlower,
                        bg: self.bg,
                    },
                    self.screen,
                );
            }
            for y in 0..height {
                if !self.screen.write_list[y as usize].items.is_empty() {
                    self.should_draw(y, 1);
                }
            }
            self.discard_rows(0, height);
            self.scrolled = 0;
            self.bg = Colour::DEFAULT;
            return;
        }
        if self.scrolled != 0 {
            let snap = self.snapshot(true);
            if self.obscured() {
                self.redraw_rows_snapshot(0, height, snap);
                self.discard_rows(0, height);
                self.scrolled = 0;
                self.bg = Colour::DEFAULT;
                return;
            }
            if self.policy.pane_backed && self.sink.scrollbar_overlay_visible() {
                self.sink
                    .effect(ScreenRenderEffects::RequirePaneRedraw, self.screen);
                self.discard_rows(0, height);
                self.scrolled = 0;
                self.bg = Colour::DEFAULT;
                return;
            }
            self.emit(
                DrawCommand::ScrollUp {
                    count: self
                        .scrolled
                        .min(self.screen.rlower - self.screen.rupper + 1),
                    bg: self.bg,
                },
                snap,
            );
            if self.policy.pane_backed {
                self.sink
                    .effect(ScreenRenderEffects::ScrollbarChanged, self.screen);
            }
            self.scrolled = 0;
        }
        self.bg = Colour::DEFAULT;
        if scroll_only {
            return;
        }
        let (cx, cy) = (self.screen.cx, self.screen.cy);
        for y in 0..height {
            let mut items = std::mem::take(&mut self.screen.write_list[y as usize].items);
            let data = std::mem::take(&mut self.screen.write_list[y as usize].data);
            let mut index = 0;
            while index < items.len() {
                let item = &items[index];
                self.spans.clear();
                self.sink
                    .visible_columns(item.x, y, item.used, &mut self.spans);
                let mut written = false;
                for span_index in 0..self.spans.len() {
                    let range = self.spans[span_index].clone();
                    if range.is_empty() {
                        continue;
                    }
                    self.set_cursor(Some(range.start), Some(y));
                    let mut snap = self.snapshot(item.kind == ScreenWriteItemKind::Clear);
                    match item.kind {
                        ScreenWriteItemKind::Clear => self.emit(
                            DrawCommand::ClearCharacter {
                                count: range.end - range.start,
                                bg: item.bg,
                            },
                            snap,
                        ),
                        ScreenWriteItemKind::Text => {
                            if self.obscured() {
                                self.emit(
                                    DrawCommand::RedrawLine {
                                        start: range.start,
                                        row: y,
                                        count: range.end - range.start,
                                    },
                                    snap,
                                );
                            } else {
                                snap.wrapped = item.wrapped;
                                self.emit(
                                    DrawCommand::Cells {
                                        cell: &item.gc,
                                        data: &data[range.start as usize..range.end as usize],
                                    },
                                    snap,
                                );
                            }
                        }
                    }
                    written = true;
                }
                if written {
                    items.remove(index);
                } else {
                    index += 1;
                }
            }
            self.screen.write_list[y as usize].items = items;
            self.screen.write_list[y as usize].data = data;
        }
        self.screen.cx = cx;
        self.screen.cy = cy;
    }
}

#[cfg(test)]
mod tests {
    use super::super::write::{ScreenOnlySink, ScreenWritePolicy};
    use super::super::{Screen, ScreenResetPolicy};
    use super::*;
    use crate::hyperlinks::HyperlinkRegistry;

    #[test]
    fn every_overlap_relation_and_random_insertions_preserve_partition() {
        let mut registry = HyperlinkRegistry::new();
        let mut screen =
            Screen::new(32, 2, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
        );
        let mut expected = [None; 32];
        let mut seed = 0x72a493u32;
        for step in 0..10000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = seed % 32;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let count = 1 + seed % (32 - x);
            let kind = if step % 2 == 0 {
                ScreenWriteItemKind::Clear
            } else {
                ScreenWriteItemKind::Text
            };
            let bg = Colour(step);
            ctx.insert_item(
                0,
                ScreenWriteItem {
                    x,
                    used: count,
                    kind,
                    bg,
                    ..ScreenWriteItem::default()
                },
            );
            expected[x as usize..(x + count) as usize].fill(Some((kind, bg)));
            let mut actual = [None; 32];
            let mut end = 0;
            for item in &ctx.screen.write_list[0].items {
                assert!(item.used != 0 && item.x >= end && item.x + item.used <= 32);
                end = item.x + item.used;
                actual[item.x as usize..end as usize].fill(Some((item.kind, item.bg)));
            }
            assert_eq!(actual, expected);
        }
        ctx.screen.write_list[0].items.clear();
        ctx.insert_item(
            0,
            ScreenWriteItem {
                x: 0,
                used: 5,
                wrapped: true,
                ..ScreenWriteItem::default()
            },
        );
        ctx.insert_clear(0, 0, 5, Colour::DEFAULT);
        assert!(ctx.screen.write_list[0].items[0].wrapped);
    }
}
