// Ported from tmux window.c @ 8f25579c (test double for the G12 model)
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

//! A minimal window and pane model for layout tests: the `LayoutHost`
//! methods over plain structs, with the `window.c` helper semantics the
//! layout code depends on.

use std::collections::BTreeMap;

use rmux_emu::screen::PaneLines;
use rmux_util::bytes::ByteString;

use super::{Cells, LayoutCells, LayoutHost, LayoutSetIndex};
use crate::cmd::arguments::ArgumentFormatRuntime;
use crate::ids::{Arena, LayoutCellId, PaneId, QueueItemId, WindowId};
use crate::model::PaneFlags;
use crate::model::spawn::SpawnFlags;
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use crate::ui::status::PaneStatusPosition;

#[derive(Debug)]
pub struct FakeWindow {
    pub public_id: u32,
    pub sx: u32,
    pub sy: u32,
    pub panes: Vec<PaneId>,
    pub z_index: Vec<PaneId>,
    pub last_panes: Vec<PaneId>,
    pub active: Option<PaneId>,
    pub layout_root: Option<LayoutCellId>,
    pub lastlayout: Option<LayoutSetIndex>,
    pub last_new_pane_x: i32,
    pub last_new_pane_y: i32,
    pub sb: PaneScrollbarPolicy,
    pub sb_pos: PaneScrollbarPosition,
    pub pane_status: PaneStatusPosition,
    pub zoomed: bool,
    pub strings: BTreeMap<Vec<u8>, ByteString>,
    pub numbers: BTreeMap<Vec<u8>, i64>,
}

#[derive(Debug)]
pub struct FakePane {
    pub public_id: u32,
    pub window: WindowId,
    pub xoff: i32,
    pub yoff: i32,
    pub sx: u32,
    pub sy: u32,
    pub flags: PaneFlags,
    pub layout_cell: Option<LayoutCellId>,
    pub saved_layout_cell: Option<LayoutCellId>,
    pub scrollbar_width: i32,
    pub scrollbar_pad: i32,
    pub lines: PaneLines,
    pub show_scrollbar: bool,
    pub floating_status: PaneStatusPosition,
}

#[derive(Default)]
pub struct FakeServer {
    pub cells: Cells,
    pub windows: Arena<FakeWindow, WindowId>,
    pub panes: Arena<FakePane, PaneId>,
    pub next_pane_id: u32,
    pub events: Vec<(WindowId, String)>,
    pub redraws: Vec<WindowId>,
    pub invalidations: Vec<WindowId>,
    pub zoom_pushes: Vec<(WindowId, bool, bool)>,
    pub recalculated: u32,
    pub active_changes: Vec<(WindowId, PaneId, bool)>,
}

impl FakeServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// A window of `sx` x `sy` with one pane and a one-leaf layout.
    pub fn window(&mut self, sx: u32, sy: u32) -> (WindowId, PaneId) {
        let w = self
            .windows
            .insert(FakeWindow {
                public_id: self.windows.len() as u32,
                sx,
                sy,
                panes: Vec::new(),
                z_index: Vec::new(),
                last_panes: Vec::new(),
                active: None,
                layout_root: None,
                lastlayout: None,
                last_new_pane_x: 0,
                last_new_pane_y: 0,
                sb: PaneScrollbarPolicy::Off,
                sb_pos: PaneScrollbarPosition::Right,
                pane_status: PaneStatusPosition::Off,
                zoomed: false,
                strings: BTreeMap::from([
                    (b"main-pane-height".to_vec(), ByteString::from("24")),
                    (b"main-pane-width".to_vec(), ByteString::from("80")),
                    (b"other-pane-height".to_vec(), ByteString::from("0")),
                    (b"other-pane-width".to_vec(), ByteString::from("0")),
                ]),
                numbers: BTreeMap::from([
                    (b"pane-base-index".to_vec(), 0),
                    (b"tiled-layout-max-columns".to_vec(), 0),
                ]),
            })
            .unwrap();
        let wp = self.add_pane(w);
        self.windows.get_mut(w).unwrap().active = Some(wp);
        super::init(self, w, wp);
        (w, wp)
    }

    /// `window_add_pane` without a layout cell: appended to the pane list and
    /// the z-index tail.
    pub fn add_pane(&mut self, w: WindowId) -> PaneId {
        let wp = self.create_pane(w);
        let window = self.windows.get_mut(w).unwrap();
        window.panes.push(wp);
        window.z_index.push(wp);
        wp
    }

    /// `window_add_pane(w, other, flags)` (`window.c:1163-1197`): list
    /// position from `BEFORE`/`FULLSIZE`/`FLOATING`, floats at the z-index
    /// head.
    pub fn add_pane_after(&mut self, w: WindowId, other: PaneId, flags: SpawnFlags) -> PaneId {
        let wp = self.create_pane(w);
        let window = self.windows.get_mut(w).unwrap();
        let at = window.panes.iter().position(|&p| p == other).unwrap();
        if flags.contains(SpawnFlags::BEFORE) {
            if flags.contains(SpawnFlags::FULLSIZE) {
                window.panes.insert(0, wp);
            } else {
                window.panes.insert(at, wp);
            }
        } else if flags.intersects(SpawnFlags::FULLSIZE | SpawnFlags::FLOATING) {
            window.panes.push(wp);
        } else {
            window.panes.insert(at + 1, wp);
        }
        if flags.contains(SpawnFlags::FLOATING) {
            window.z_index.insert(0, wp);
        } else {
            window.z_index.push(wp);
        }
        wp
    }

    fn create_pane(&mut self, w: WindowId) -> PaneId {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        self.panes
            .insert(FakePane {
                public_id: id,
                window: w,
                xoff: 0,
                yoff: 0,
                sx: 0,
                sy: 0,
                flags: PaneFlags::default(),
                layout_cell: None,
                saved_layout_cell: None,
                scrollbar_width: 1,
                scrollbar_pad: 0,
                lines: PaneLines::Single,
                show_scrollbar: true,
                floating_status: PaneStatusPosition::Off,
            })
            .unwrap()
    }

    pub fn win(&self, w: WindowId) -> &FakeWindow {
        self.windows.get(w).unwrap()
    }

    pub fn win_mut(&mut self, w: WindowId) -> &mut FakeWindow {
        self.windows.get_mut(w).unwrap()
    }

    pub fn pane(&self, wp: PaneId) -> &FakePane {
        self.panes.get(wp).unwrap()
    }

    pub fn pane_mut(&mut self, wp: PaneId) -> &mut FakePane {
        self.panes.get_mut(wp).unwrap()
    }

    /// `window_remove_pane` after `layout_close_pane` (`window.c:1200-1247`):
    /// a lost active pane falls back to the last-pane stack, then the
    /// previous pane in the list, then the next.
    pub fn remove_pane(&mut self, wp: PaneId) {
        let w = self.pane(wp).window;
        self.window_last_panes_remove(w, wp);
        let window = self.win_mut(w);
        if window.active == Some(wp) {
            let at = window.panes.iter().position(|&p| p == wp).unwrap();
            window.active = window
                .last_panes
                .first()
                .copied()
                .or_else(|| at.checked_sub(1).map(|i| window.panes[i]))
                .or_else(|| window.panes.get(at + 1).copied());
            if let Some(active) = window.active {
                self.window_last_panes_remove(w, active);
            }
        }
        let window = self.win_mut(w);
        window.panes.retain(|&p| p != wp);
        window.z_index.retain(|&p| p != wp);
        self.panes.request_remove(wp).unwrap();
    }

    /// `select-pane`: `window_redraw_active_switch` raises a floating pane
    /// (`window.c:849-853`), then `window_set_active_pane`.
    pub fn select_pane(&mut self, wp: PaneId) {
        let w = self.pane(wp).window;
        if self.win(w).active != Some(wp) && self.pane_is_floating(wp) {
            let z = &mut self.win_mut(w).z_index;
            z.retain(|&p| p != wp);
            z.insert(0, wp);
        }
        self.window_set_active_pane(w, wp, true);
    }

    pub fn events_named(&self, name: &str) -> usize {
        self.events.iter().filter(|(_, n)| n == name).count()
    }
}

impl LayoutCells for FakeServer {
    fn cells(&self) -> &Cells {
        &self.cells
    }
    fn cells_mut(&mut self) -> &mut Cells {
        &mut self.cells
    }
    fn pane_layout_cell(&self, wp: PaneId) -> Option<LayoutCellId> {
        self.pane(wp).layout_cell
    }
    fn set_pane_layout_cell(&mut self, wp: PaneId, lc: Option<LayoutCellId>) {
        self.pane_mut(wp).layout_cell = lc;
    }
}

impl LayoutHost for FakeServer {
    fn window_public_id(&self, w: WindowId) -> u32 {
        self.win(w).public_id
    }
    fn window_size(&self, w: WindowId) -> (u32, u32) {
        let win = self.win(w);
        (win.sx, win.sy)
    }
    fn window_layout_root(&self, w: WindowId) -> Option<LayoutCellId> {
        self.win(w).layout_root
    }
    fn set_window_layout_root(&mut self, w: WindowId, root: Option<LayoutCellId>) {
        self.win_mut(w).layout_root = root;
    }
    fn window_panes(&self, w: WindowId) -> &[PaneId] {
        &self.win(w).panes
    }
    fn window_z_index(&self, w: WindowId) -> &[PaneId] {
        &self.win(w).z_index
    }
    fn window_z_index_mut(&mut self, w: WindowId) -> &mut Vec<PaneId> {
        &mut self.win_mut(w).z_index
    }
    fn window_last_panes(&self, w: WindowId) -> &[PaneId] {
        &self.win(w).last_panes
    }
    fn window_active(&self, w: WindowId) -> Option<PaneId> {
        self.win(w).active
    }
    fn window_scrollbars(&self, w: WindowId) -> PaneScrollbarPolicy {
        self.win(w).sb
    }
    fn window_scrollbar_position(&self, w: WindowId) -> PaneScrollbarPosition {
        self.win(w).sb_pos
    }
    fn window_pane_status(&self, w: WindowId) -> PaneStatusPosition {
        match self.win(w).pane_status {
            PaneStatusPosition::TopFloating | PaneStatusPosition::BottomFloating => {
                PaneStatusPosition::Off
            }
            status => status,
        }
    }
    fn window_lastlayout(&self, w: WindowId) -> Option<LayoutSetIndex> {
        self.win(w).lastlayout
    }
    fn set_window_lastlayout(&mut self, w: WindowId, layout: Option<LayoutSetIndex>) {
        self.win_mut(w).lastlayout = layout;
    }
    fn window_last_new_pane(&self, w: WindowId) -> (i32, i32) {
        let win = self.win(w);
        (win.last_new_pane_x, win.last_new_pane_y)
    }
    fn set_window_last_new_pane(&mut self, w: WindowId, x: i32, y: i32) {
        let win = self.win_mut(w);
        win.last_new_pane_x = x;
        win.last_new_pane_y = y;
    }
    fn window_option_string(&self, w: WindowId, name: &[u8]) -> &[u8] {
        self.win(w)
            .strings
            .get(name)
            .map(|s| s.as_bytes())
            .unwrap_or_else(|| panic!("missing option {}", ByteString::from(name)))
    }
    fn window_option_number(&self, w: WindowId, name: &[u8]) -> i64 {
        *self
            .win(w)
            .numbers
            .get(name)
            .unwrap_or_else(|| panic!("missing option {}", ByteString::from(name)))
    }

    fn pane_window(&self, wp: PaneId) -> WindowId {
        self.pane(wp).window
    }
    fn pane_public_id(&self, wp: PaneId) -> u32 {
        self.pane(wp).public_id
    }
    fn pane_saved_layout_cell(&self, wp: PaneId) -> Option<LayoutCellId> {
        self.pane(wp).saved_layout_cell
    }
    fn pane_geometry(&self, wp: PaneId) -> (i32, i32, u32, u32) {
        let p = self.pane(wp);
        (p.xoff, p.yoff, p.sx, p.sy)
    }
    fn set_pane_offset(&mut self, wp: PaneId, xoff: i32, yoff: i32) {
        let p = self.pane_mut(wp);
        p.xoff = xoff;
        p.yoff = yoff;
    }
    fn pane_scrollbar_style(&self, wp: PaneId) -> (i32, i32) {
        let p = self.pane(wp);
        (p.scrollbar_width, p.scrollbar_pad)
    }
    fn pane_flags(&self, wp: PaneId) -> PaneFlags {
        self.pane(wp).flags
    }
    fn pane_flags_insert(&mut self, wp: PaneId, flags: PaneFlags) {
        self.pane_mut(wp).flags.insert(flags);
    }
    fn pane_status(&self, wp: PaneId) -> PaneStatusPosition {
        // window_pane_get_pane_status (window.c:2945-2968) without modes.
        if !self.pane_is_floating(wp) {
            return self.window_pane_status(self.pane(wp).window);
        }
        if self.pane_lines(wp) == PaneLines::None {
            return PaneStatusPosition::Off;
        }
        match self.pane(wp).floating_status {
            PaneStatusPosition::TopFloating => PaneStatusPosition::Top,
            PaneStatusPosition::BottomFloating => PaneStatusPosition::Bottom,
            status => status,
        }
    }
    fn pane_lines(&self, wp: PaneId) -> PaneLines {
        self.pane(wp).lines
    }
    fn pane_scrollbar_reserve(&self, wp: PaneId) -> bool {
        self.pane(wp).show_scrollbar
            && self.win(self.pane(wp).window).sb == PaneScrollbarPolicy::Always
    }

    fn pane_resize(&mut self, wp: PaneId, sx: u32, sy: u32) {
        let p = self.pane_mut(wp);
        p.sx = sx;
        p.sy = sy;
    }
    fn window_resize(&mut self, w: WindowId, sx: u32, sy: u32) {
        let win = self.win_mut(w);
        win.sx = sx;
        win.sy = sy;
    }
    fn window_set_active_pane(&mut self, w: WindowId, wp: PaneId, notify: bool) {
        // window.c:756-800 without zoom, focus and redraw.
        let old = self.win(w).active;
        if old == Some(wp) {
            return;
        }
        self.window_last_panes_remove(w, wp);
        if let Some(old) = old {
            self.window_last_panes_push(w, old);
        }
        self.win_mut(w).active = Some(wp);
        self.active_changes.push((w, wp, notify));
    }
    fn window_last_panes_push(&mut self, w: WindowId, wp: PaneId) {
        self.window_last_panes_remove(w, wp);
        self.win_mut(w).last_panes.insert(0, wp);
        self.pane_mut(wp).flags.insert(PaneFlags::VISITED);
    }
    fn window_last_panes_remove(&mut self, w: WindowId, wp: PaneId) {
        if self.pane(wp).flags.contains(PaneFlags::VISITED) {
            self.win_mut(w).last_panes.retain(|&p| p != wp);
            self.pane_mut(wp).flags.remove(PaneFlags::VISITED);
        }
    }
    fn window_push_zoom(&mut self, w: WindowId, always: bool, flag: bool) -> bool {
        self.zoom_pushes.push((w, always, flag));
        false
    }
    fn window_active_pane_is_over_zoom(&self, w: WindowId) -> bool {
        let win = self.win(w);
        win.zoomed
            && win.active.is_some_and(|wp| {
                self.pane(wp).flags.contains(PaneFlags::FLOATOVERZOOM) && self.pane_is_floating(wp)
            })
    }
    fn recalculate_sizes(&mut self) {
        self.recalculated += 1;
    }

    fn fire_window_event(&mut self, w: WindowId, name: &str) {
        self.events.push((w, name.to_string()));
    }
    fn redraw_window(&mut self, w: WindowId) {
        self.redraws.push(w);
    }
    fn invalidate_scene(&mut self, w: WindowId) {
        self.invalidations.push(w);
    }
}

impl ArgumentFormatRuntime for FakeServer {
    fn expand_from_target(&mut self, _item: QueueItemId, value: &[u8]) -> ByteString {
        ByteString::from(value)
    }
}
