// Ported from tmux screen-write.c and tmux.h @ 8f25579c
use super::{Screen, ScreenMode};
use crate::cell::{DEFAULT_CELL, GridCell, GridCellFlags};
use crate::colour::Colour;
use crate::hyperlinks::HyperlinkRegistry;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScreenWriteFlags(pub u32);
impl ScreenWriteFlags {
    pub const SYNC: Self = Self(1);
    pub const OBSCURED: Self = Self(2);
    pub const CHECKED_IF_OBSCURED: Self = Self(4);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for ScreenWriteFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for ScreenWriteFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for ScreenWriteFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ScreenWritePolicy {
    pub pane_backed: bool,
    pub alternate_screen: bool,
    pub scroll_on_clear: bool,
    pub variation_selector_always_wide: bool,
    pub extended_keys: bool,
}
/// Defaults follow options-table.c @ 8f25579c (alternate-screen 1,
/// scroll-on-clear 1, variation-selector-always-wide 1, extended-keys off)
/// for a screen with no pane.
impl Default for ScreenWritePolicy {
    fn default() -> Self {
        Self {
            pane_backed: false,
            alternate_screen: true,
            scroll_on_clear: true,
            variation_selector_always_wide: true,
            extended_keys: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawSnapshot {
    pub width: u32,
    pub height: u32,
    pub old_cx: u32,
    pub old_cy: u32,
    pub rupper: u32,
    pub rlower: u32,
    pub wrapped: bool,
    pub invalidate_cursor: bool,
    pub sync: bool,
}
#[derive(Debug)]
pub enum DrawCommand<'a> {
    SyncStart,
    Cell(&'a GridCell),
    Cells {
        cell: &'a GridCell,
        data: &'a [u8],
    },
    RedrawLine {
        start: u32,
        row: u32,
        count: u32,
    },
    AlignmentTest,
    InsertCharacter {
        count: u32,
        bg: Colour,
    },
    DeleteCharacter {
        count: u32,
        bg: Colour,
    },
    ClearCharacter {
        count: u32,
        bg: Colour,
    },
    InsertLine {
        count: u32,
        bg: Colour,
    },
    DeleteLine {
        count: u32,
        bg: Colour,
    },
    ClearEndOfScreen {
        bg: Colour,
    },
    ClearStartOfScreen {
        bg: Colour,
    },
    ClearScreen {
        bg: Colour,
    },
    ScrollUp {
        count: u32,
        bg: Colour,
    },
    ScrollDown {
        count: u32,
        bg: Colour,
    },
    ReverseIndex {
        bg: Colour,
    },
    SetSelection {
        selector: &'a [u8],
        data: &'a [u8],
    },
    RawString {
        data: &'a [u8],
        allow_invisible: bool,
    },
}
pub struct DrawOp<'a> {
    pub command: DrawCommand<'a>,
    pub screen: &'a Screen,
    pub hyperlinks: &'a HyperlinkRegistry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenRenderEffects {
    CursorMoved {
        x: u32,
        y: u32,
    },
    DamageRows {
        start: u32,
        count: u32,
    },
    DirtyRows {
        start: u32,
        count: u32,
    },
    DeferredScroll {
        count: u32,
        rupper: u32,
        rlower: u32,
        bg: Colour,
    },
    ScrollbarChanged,
    RequirePaneRedraw,
    AlternateChanged {
        entering: bool,
        successful: bool,
    },
    StartSyncTimer,
    StopSync,
}
pub trait TtySink {
    fn draw(&mut self, op: DrawOp<'_>, snapshot: &DrawSnapshot);
    fn visible_columns(&mut self, x: u32, y: u32, n: u32, out: &mut Vec<Range<u32>>);
    fn obscured(&mut self) -> bool;
    /// window_pane_scrollbar_overlay_visible: a scroll flush must fall back
    /// to a pane redraw instead of a terminal scroll (screen-write.c:2443).
    fn scrollbar_overlay_visible(&mut self) -> bool {
        false
    }
    fn redraw_pending(&self) -> bool;
    fn effect(&mut self, effect: ScreenRenderEffects, screen: &Screen);
    fn begin_write(&mut self);
}
#[derive(Default)]
pub struct ScreenOnlySink;
impl TtySink for ScreenOnlySink {
    fn draw(&mut self, _: DrawOp<'_>, _: &DrawSnapshot) {}
    fn visible_columns(&mut self, x: u32, _: u32, n: u32, out: &mut Vec<Range<u32>>) {
        out.clear();
        if n != 0 {
            out.push(x..x + n);
        }
    }
    fn obscured(&mut self) -> bool {
        false
    }
    fn redraw_pending(&self) -> bool {
        false
    }
    fn effect(&mut self, _: ScreenRenderEffects, _: &Screen) {}
    fn begin_write(&mut self) {}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenWriteItemKind {
    Text,
    Clear,
}
#[derive(Clone, Debug)]
pub struct ScreenWriteItem {
    pub x: u32,
    pub used: u32,
    pub wrapped: bool,
    pub kind: ScreenWriteItemKind,
    pub bg: Colour,
    pub gc: GridCell,
}
impl Default for ScreenWriteItem {
    fn default() -> Self {
        Self {
            x: 0,
            used: 0,
            wrapped: false,
            kind: ScreenWriteItemKind::Text,
            bg: Colour::DEFAULT,
            gc: DEFAULT_CELL,
        }
    }
}
#[derive(Debug, Default)]
pub struct ScreenWriteLine {
    pub data: Vec<u8>,
    pub items: Vec<ScreenWriteItem>,
}
pub struct ScreenWriteCtx<'a> {
    pub screen: &'a mut Screen,
    pub(crate) sink: &'a mut dyn TtySink,
    pub policy: ScreenWritePolicy,
    pub registry: &'a mut HyperlinkRegistry,
    pub flags: ScreenWriteFlags,
    pub(crate) spans: Vec<Range<u32>>,
    pub(crate) scrolled: u32,
    pub(crate) bg: Colour,
    pub(crate) item: ScreenWriteItem,
}
impl<'a> ScreenWriteCtx<'a> {
    pub fn start(
        screen: &'a mut Screen,
        sink: &'a mut dyn TtySink,
        policy: ScreenWritePolicy,
        registry: &'a mut HyperlinkRegistry,
    ) -> Self {
        sink.begin_write();
        screen
            .write_list
            .resize_with(screen.grid.sy() as usize, ScreenWriteLine::default);
        Self {
            screen,
            sink,
            policy,
            registry,
            flags: ScreenWriteFlags::default(),
            spans: Vec::new(),
            scrolled: 0,
            bg: Colour::DEFAULT,
            item: ScreenWriteItem::default(),
        }
    }
    pub fn finish(mut self) {
        self.collect_end();
        self.flush(false);
    }
    pub(crate) fn set_cursor(&mut self, x: Option<u32>, y: Option<u32>) {
        if x == Some(self.screen.cx) && y == Some(self.screen.cy) {
            return;
        }
        if let Some(x) = x {
            self.screen.cx = if x > self.screen.grid.sx() {
                self.screen.grid.sx() - 1
            } else {
                x
            };
        }
        if let Some(y) = y {
            self.screen.cy = y.min(self.screen.grid.sy() - 1);
        }
        if self.policy.pane_backed {
            self.sink.effect(
                ScreenRenderEffects::CursorMoved {
                    x: self.screen.cx,
                    y: self.screen.cy,
                },
                self.screen,
            );
        }
    }
    pub(crate) fn obscured(&mut self) -> bool {
        if !self.policy.pane_backed {
            return false;
        }
        if !self.flags.contains(ScreenWriteFlags::CHECKED_IF_OBSCURED) {
            self.flags.insert(ScreenWriteFlags::CHECKED_IF_OBSCURED);
            if self.sink.obscured() {
                self.flags.insert(ScreenWriteFlags::OBSCURED);
            }
        }
        self.flags.contains(ScreenWriteFlags::OBSCURED)
    }
    pub(crate) fn snapshot(&mut self, sync: bool) -> DrawSnapshot {
        // TTY_CTX_SYNC only travels with the first context of a transaction,
        // the one that carries tty_cmd_syncstart (screen-write.c:332-346).
        let first = !self.flags.contains(ScreenWriteFlags::SYNC);
        let s = &self.screen;
        let snap = DrawSnapshot {
            width: s.grid.sx(),
            height: s.grid.sy(),
            old_cx: s.cx,
            old_cy: s.cy,
            rupper: s.rupper,
            rlower: s.rlower,
            sync: first && sync,
            ..DrawSnapshot::default()
        };
        if first {
            self.flags.insert(ScreenWriteFlags::SYNC);
            self.emit(DrawCommand::SyncStart, snap);
        }
        snap
    }
    pub(crate) fn emit(&mut self, command: DrawCommand<'_>, snapshot: DrawSnapshot) {
        self.sink.draw(
            DrawOp {
                command,
                screen: self.screen,
                hyperlinks: self.registry,
            },
            &snapshot,
        );
    }
    pub(crate) fn should_draw(&mut self, y: u32, count: u32) -> bool {
        if self.sink.redraw_pending() {
            return false;
        }
        if self.screen.mode.contains(ScreenMode::SYNC) {
            if self.policy.pane_backed && y < self.screen.grid.sy() && count != 0 {
                self.sink.effect(
                    ScreenRenderEffects::DirtyRows {
                        start: y,
                        count: count.min(self.screen.grid.sy() - y),
                    },
                    self.screen,
                );
            }
            return false;
        }
        true
    }
    pub(crate) fn fully_visible(&mut self, x: u32, y: u32, n: u32) -> bool {
        self.spans.clear();
        self.sink.visible_columns(x, y, n, &mut self.spans);
        self.spans.iter().map(|r| r.end - r.start).sum::<u32>() == n
    }
    pub(crate) fn redraw_rows_snapshot(&mut self, y: u32, count: u32, snapshot: DrawSnapshot) {
        let sx = self.screen.grid.sx();
        for row in y..y + count {
            self.spans.clear();
            self.sink.visible_columns(0, row, sx, &mut self.spans);
            for i in 0..self.spans.len() {
                let r = self.spans[i].clone();
                if r.start >= sx || r.is_empty() {
                    continue;
                }
                let n = r.end.min(sx) - r.start;
                if n == 1 {
                    let cell = self.screen.grid.view_get_cell(r.start, row);
                    if cell_is_single(&cell) {
                        let display = if cell.flags.contains(GridCellFlags::SELECTED) {
                            self.screen.select_cell(&cell)
                        } else {
                            cell
                        };
                        let mut positioned = snapshot;
                        positioned.old_cx = r.start;
                        positioned.old_cy = row;
                        self.emit(DrawCommand::Cell(&display), positioned);
                        continue;
                    }
                }
                self.emit(
                    DrawCommand::RedrawLine {
                        start: r.start,
                        row,
                        count: n,
                    },
                    snapshot,
                );
            }
        }
    }
    pub fn mode_set(&mut self, mode: ScreenMode) {
        self.screen.mode.insert(mode);
    }
    pub fn mode_clear(&mut self, mode: ScreenMode) {
        self.screen.mode.remove(mode);
    }
    pub fn start_sync(&mut self) {
        if self.policy.pane_backed {
            self.screen.mode.insert(ScreenMode::SYNC);
            self.sink
                .effect(ScreenRenderEffects::StartSyncTimer, self.screen);
        }
    }
    pub fn end_sync(&mut self) {
        if !self.policy.pane_backed {
            return;
        }
        if self.screen.mode.contains(ScreenMode::SYNC) {
            self.flush(false);
            self.screen.mode.remove(ScreenMode::SYNC);
            self.sink.effect(ScreenRenderEffects::StopSync, self.screen);
        }
    }
    pub fn setselection(&mut self, selector: &[u8], data: &[u8]) {
        let snap = self.snapshot(false);
        self.emit(DrawCommand::SetSelection { selector, data }, snap);
    }
    pub fn rawstring(&mut self, data: &[u8], allow_invisible: bool) {
        let snap = self.snapshot(false);
        self.emit(
            DrawCommand::RawString {
                data,
                allow_invisible,
            },
            snap,
        );
    }
    pub fn alternateon(&mut self, cell: &GridCell, save_cursor: bool) -> bool {
        if !self.policy.alternate_screen {
            return false;
        }
        self.flush(false);
        let changed = self.screen.alternate_on(cell, save_cursor);
        if changed {
            let _ = self.snapshot(true);
            self.sink.effect(
                ScreenRenderEffects::AlternateChanged {
                    entering: true,
                    successful: true,
                },
                self.screen,
            );
        }
        changed
    }
    pub fn alternateoff(&mut self, cell: Option<&mut GridCell>, restore_cursor: bool) -> bool {
        if !self.policy.alternate_screen {
            return false;
        }
        self.flush(false);
        let changed = self.screen.alternate_off(cell, restore_cursor);
        if changed {
            let _ = self.snapshot(true);
            self.sink.effect(
                ScreenRenderEffects::AlternateChanged {
                    entering: false,
                    successful: true,
                },
                self.screen,
            );
        }
        changed
    }
}

/// screen_write_cell_is_single: a plain single-width ASCII cell.
pub(crate) fn cell_is_single(cell: &GridCell) -> bool {
    cell.data.width == 1
        && cell.data.size == 1
        && cell.data.data[0] >= 0x20
        && cell.data.data[0] != 0x7f
        && !cell
            .flags
            .intersects(GridCellFlags::CLEARED | GridCellFlags::PADDING | GridCellFlags::TAB)
}
