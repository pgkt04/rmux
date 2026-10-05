// Ported from tmux window.c, resize.c @ 8f25579c (layout host over the G12 model)
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

//! `LayoutHost` for the real server: each method maps to the `struct window`
//! or `struct window_pane` field, or the `window.c` call, that `layout.c`
//! uses. Redraw, scene and size recalculation become `ModelEffect`s.

use rmux_emu::screen::PaneLines;

use super::{Cells, LayoutCells, LayoutHost, LayoutSetIndex};
use crate::ids::{LayoutCellId, PaneId, WindowId};
use crate::model::resize::PixelUpdate;
use crate::model::{ModelEffect, Pane, PaneFlags, Server, Window, pane, window};
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use crate::ui::status::PaneStatusPosition;

fn win(server: &Server, w: WindowId) -> &Window {
    server.windows.get(w).expect("stale window id in layout")
}

fn win_mut(server: &mut Server, w: WindowId) -> &mut Window {
    server
        .windows
        .get_mut(w)
        .expect("stale window id in layout")
}

fn pane_ref(server: &Server, wp: PaneId) -> &Pane {
    server.panes.get(wp).expect("stale pane id in layout")
}

fn pane_mut(server: &mut Server, wp: PaneId) -> &mut Pane {
    server.panes.get_mut(wp).expect("stale pane id in layout")
}

fn status(value: i64) -> PaneStatusPosition {
    PaneStatusPosition::try_from(value as i32).unwrap_or(PaneStatusPosition::Off)
}

impl LayoutCells for Server {
    fn cells(&self) -> &Cells {
        &self.layout_cells
    }
    fn cells_mut(&mut self) -> &mut Cells {
        &mut self.layout_cells
    }
    fn pane_layout_cell(&self, wp: PaneId) -> Option<LayoutCellId> {
        pane_ref(self, wp).layout_cell
    }
    fn set_pane_layout_cell(&mut self, wp: PaneId, lc: Option<LayoutCellId>) {
        pane_mut(self, wp).layout_cell = lc;
    }
}

impl LayoutHost for Server {
    fn window_public_id(&self, w: WindowId) -> u32 {
        win(self, w).public_id
    }
    fn window_size(&self, w: WindowId) -> (u32, u32) {
        let w = win(self, w);
        (w.sx, w.sy)
    }
    fn window_layout_root(&self, w: WindowId) -> Option<LayoutCellId> {
        win(self, w).layout_root
    }
    fn set_window_layout_root(&mut self, w: WindowId, root: Option<LayoutCellId>) {
        win_mut(self, w).layout_root = root;
    }
    fn window_panes(&self, w: WindowId) -> &[PaneId] {
        &win(self, w).panes
    }
    fn window_z_index(&self, w: WindowId) -> &[PaneId] {
        &win(self, w).z_order
    }
    fn window_z_index_mut(&mut self, w: WindowId) -> &mut Vec<PaneId> {
        &mut win_mut(self, w).z_order
    }
    fn window_last_panes(&self, w: WindowId) -> &[PaneId] {
        &win(self, w).last
    }
    fn window_active(&self, w: WindowId) -> Option<PaneId> {
        win(self, w).active
    }
    fn window_scrollbars(&self, w: WindowId) -> PaneScrollbarPolicy {
        win(self, w).sb
    }
    fn window_scrollbar_position(&self, w: WindowId) -> PaneScrollbarPosition {
        win(self, w).sb_pos
    }
    fn window_pane_status(&self, w: WindowId) -> PaneStatusPosition {
        status(window::window_get_pane_status(self, w))
    }
    fn window_lastlayout(&self, w: WindowId) -> Option<LayoutSetIndex> {
        win(self, w).lastlayout
    }
    fn set_window_lastlayout(&mut self, w: WindowId, layout: Option<LayoutSetIndex>) {
        win_mut(self, w).lastlayout = layout;
    }
    fn window_last_new_pane(&self, w: WindowId) -> (i32, i32) {
        let w = win(self, w);
        (w.last_new_pane_x, w.last_new_pane_y)
    }
    fn set_window_last_new_pane(&mut self, w: WindowId, x: i32, y: i32) {
        let w = win_mut(self, w);
        w.last_new_pane_x = x;
        w.last_new_pane_y = y;
    }
    fn window_option_string(&self, w: WindowId, name: &[u8]) -> &[u8] {
        self.options.get_string(win(self, w).options, name)
    }
    fn window_option_number(&self, w: WindowId, name: &[u8]) -> i64 {
        self.options.get_number(win(self, w).options, name)
    }

    fn pane_window(&self, wp: PaneId) -> WindowId {
        pane_ref(self, wp).window
    }
    fn pane_public_id(&self, wp: PaneId) -> u32 {
        pane_ref(self, wp).public_id
    }
    fn pane_saved_layout_cell(&self, wp: PaneId) -> Option<LayoutCellId> {
        pane_ref(self, wp).saved_layout_cell
    }
    fn pane_geometry(&self, wp: PaneId) -> (i32, i32, u32, u32) {
        let p = pane_ref(self, wp);
        (p.xoff, p.yoff, p.sx, p.sy)
    }
    fn set_pane_offset(&mut self, wp: PaneId, xoff: i32, yoff: i32) {
        let p = pane_mut(self, wp);
        p.xoff = xoff;
        p.yoff = yoff;
    }
    fn pane_scrollbar_style(&self, wp: PaneId) -> (i32, i32) {
        let style = &pane_ref(self, wp).scrollbar_style;
        (style.width, style.pad)
    }
    fn pane_flags(&self, wp: PaneId) -> PaneFlags {
        pane_ref(self, wp).flags
    }
    fn pane_flags_insert(&mut self, wp: PaneId, flags: PaneFlags) {
        pane_mut(self, wp).flags.insert(flags);
    }
    fn pane_status(&self, wp: PaneId) -> PaneStatusPosition {
        status(pane::pane_get_pane_status(self, wp))
    }
    fn pane_lines(&self, wp: PaneId) -> PaneLines {
        PaneLines::try_from(pane::pane_get_pane_lines(self, wp) as i32).unwrap_or(PaneLines::Single)
    }
    fn pane_scrollbar_reserve(&self, wp: PaneId) -> bool {
        pane::pane_scrollbar_reserve(self, wp)
    }

    fn pane_resize(&mut self, wp: PaneId, sx: u32, sy: u32) {
        pane::pane_resize(self, wp, sx, sy).expect("stale pane id in layout");
    }
    fn window_resize(&mut self, w: WindowId, sx: u32, sy: u32) {
        window::window_resize(self, w, sx, sy, PixelUpdate::Keep, PixelUpdate::Keep)
            .expect("stale window id in layout");
    }
    fn window_set_active_pane(&mut self, w: WindowId, wp: PaneId, notify: bool) {
        window::window_set_active_pane(self, w, wp, notify).expect("stale ids in layout");
    }
    // window_pane_stack_push (window.c:2461-2469).
    fn window_last_panes_push(&mut self, w: WindowId, wp: PaneId) {
        self.window_last_panes_remove(w, wp);
        win_mut(self, w).last.insert(0, wp);
        pane_mut(self, wp).flags.insert(PaneFlags::VISITED);
    }
    // window_pane_stack_remove (window.c:2472-2479).
    fn window_last_panes_remove(&mut self, _w: WindowId, wp: PaneId) {
        window::window_pane_stack_remove(self, wp);
    }
    fn window_push_zoom(&mut self, w: WindowId, always: bool, flag: bool) -> bool {
        window::window_push_zoom(self, w, always, flag).expect("stale window id in layout")
    }
    fn window_active_pane_is_over_zoom(&self, w: WindowId) -> bool {
        window::window_active_pane_is_over_zoom(self, w)
    }
    fn recalculate_sizes(&mut self) {
        self.effects.push_back(ModelEffect::RecalculateSizes);
    }

    fn fire_window_event(&mut self, w: WindowId, name: &str) {
        self.emit(name.as_bytes(), None, Some(w), None);
    }
    fn redraw_window(&mut self, w: WindowId) {
        self.effects.push_back(ModelEffect::RedrawWindow(w));
    }
    fn invalidate_scene(&mut self, w: WindowId) {
        self.effects.push_back(ModelEffect::InvalidateScene(w));
    }
}
