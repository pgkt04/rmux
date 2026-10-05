// Ported from tmux window-copy.c @ 8f25579c
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
pub mod commands;
pub mod keys;
pub mod motion;
pub mod mouse;
pub mod render;
pub mod search;
pub mod select;
pub mod state;
pub mod view;
use crate::cmd::arguments::Args;
use crate::ids::{ClientId, ModeId, PaneId, SessionId, WinlinkId};
use crate::model::pane::{PaneMode, PaneModeDriver};
use crate::model::{ModelError, Server};
pub use render::{
    current_offset, formats, get_hyperlink, get_line, get_word, set_line_numbers, style_changed,
};
use rmux_emu::grid::{Grid, GridFlags};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_util::key::KeyCode;
pub use state::*;
use std::time::Duration;
#[cfg(test)]
#[path = "../../../../rmux-util/tests/common/mod.rs"]
mod test_common;

#[derive(Clone)]
pub enum CopyModeKind {
    Copy { source: Option<PaneId>, args: Args },
    View,
}
pub struct CopyModeDriver {
    pub kind: CopyModeKind,
}
impl PaneModeDriver for CopyModeDriver {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
        match &self.kind {
            CopyModeKind::Copy { source, args } => {
                init_copy_from(server, id, source.unwrap_or(id.owner), args)
            }
            CopyModeKind::View => view::init_view(server, id),
        }
    }
    fn free(&self, server: &mut Server, mode: PaneMode) {
        free_entry(server, mode);
    }
    fn resize(&self, server: &mut Server, id: ModeId, sx: u32, sy: u32) {
        resize(server, id, sx, sy);
    }
    fn key(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _client: ClientId,
        _key: KeyCode,
        _mouse: Option<&crate::client::ResolvedMouseEvent>,
    ) {
        // Copy mode receives bound commands through its key table, not raw keys.
    }
    fn append_output(
        &self,
        server: &mut Server,
        id: ModeId,
        bytes: &[u8],
    ) -> Result<(), ModelError> {
        view::append_output(server, id, bytes)
    }
    fn current_offset(&self, server: &Server, id: ModeId) -> Option<(u32, u32)> {
        let d = data(server, id)?;
        let h = d.backing.screen().grid.hsize();
        Some((h.saturating_sub(d.oy), h))
    }
    fn key_table(&self, server: &Server, id: ModeId) -> Option<Vec<u8>> {
        Some(keys::key_table(server, id).to_vec())
    }
    fn has_command(&self) -> bool {
        true
    }
    #[allow(clippy::too_many_arguments)]
    fn command(
        &self,
        server: &mut Server,
        id: ModeId,
        client: Option<ClientId>,
        session: Option<SessionId>,
        winlink: Option<WinlinkId>,
        args: &Args,
        event: Option<&crate::cmd::queue::QueueEvent>,
    ) {
        keys::command(server, id, client, session, winlink, args, event);
    }
    fn style_changed(&self, server: &mut Server, id: ModeId) {
        render::style_changed(server, id);
    }
    fn drag_update(
        &self,
        server: &mut Server,
        id: ModeId,
        client: ClientId,
        event: &crate::client::ResolvedMouseEvent,
    ) {
        mouse::drag_update(server, id, client, event);
    }
    fn drag_release(
        &self,
        server: &mut Server,
        id: ModeId,
        client: ClientId,
        event: &crate::client::ResolvedMouseEvent,
    ) {
        mouse::drag_release(server, id, client, event);
    }
}

pub fn backing_screen(server: &Server, mode: ModeId) -> Option<&Screen> {
    Some(data(server, mode)?.backing.screen())
}

pub fn clone_screen(
    source: &Screen,
    width: u32,
    height: u32,
    trim: bool,
    registry: &mut rmux_emu::hyperlinks::HyperlinkRegistry,
) -> (Screen, u32, u32) {
    let sg = &source.grid;
    let mut total = sg
        .hsize()
        .checked_add(sg.sy())
        .expect("copy grid dimensions overflow");
    if trim {
        while total > sg.hsize() + 1 && sg.get_line(total - 1).cellused() == 0 {
            total -= 1;
        }
    }
    let mut dst = Screen::new(
        sg.sx(),
        total,
        sg.hlimit(),
        ScreenResetPolicy::default(),
        registry,
    )
    .expect("copy screen hyperlink lease");
    dst.grid.flags.insert(GridFlags::HISTORY);
    dst.grid.duplicate_lines(0, sg, 0, total);
    dst.grid.set_sy_unchecked(total - sg.hsize());
    dst.grid.set_hsize_unchecked(sg.hsize());
    dst.grid.hscrolled = sg.hscrolled;
    if source.cy > dst.grid.sy() - 1 {
        dst.cx = 0;
        dst.cy = dst.grid.sy() - 1;
    } else {
        dst.cx = source.cx;
        dst.cy = source.cy;
    }
    let (mut cx, mut cy) = (dst.cx, dst.grid.hsize() + dst.cy);
    let wrapped = (sg.sx() != width).then(|| dst.grid.wrap_position(cx, cy));
    dst.resize_cursor(
        width,
        height,
        true,
        false,
        false,
        #[cfg(feature = "sixel")]
        None,
    );
    if let Some((wx, wy)) = wrapped {
        (cx, cy) = dst.grid.unwrap_position(wx, wy);
    }
    (dst, cx, cy)
}

pub fn sync_snapshot(data: &mut CopyModeData, source: &Grid) {
    data.sync_added = source.scroll_added;
    data.sync_collected = source.scroll_collected;
    data.sync_generation = source.scroll_generation;
}
pub fn sync_backing(data: &mut CopyModeData, source: &Screen, own_source: bool) -> bool {
    if data.backing.is_view() || !own_source {
        return false;
    }
    let sg = &source.grid;
    let added = sg.scroll_added.wrapping_sub(data.sync_added);
    let collected = sg.scroll_collected.wrapping_sub(data.sync_collected);
    let dg = &mut data.backing.screen_mut().grid;
    if sg.sx() != dg.sx() || sg.sy() != dg.sy() || sg.scroll_generation != data.sync_generation {
        return false;
    }
    let old = dg.hsize();
    let new = sg.hsize();
    let sy = sg.sy();
    if added > i32::MAX as u32
        || collected > i32::MAX as u32
        || collected > old
        || old
            .checked_add(added)
            .and_then(|n| n.checked_sub(collected))
            != Some(new)
    {
        return false;
    }
    let Some(total) = new.checked_add(sy) else {
        return false;
    };
    let kept = old - collected;
    if added == 0 && collected == 0 {
        dg.duplicate_lines(new, sg, new, sy);
    } else {
        if collected > 0 {
            dg.free_lines(0, collected);
            for row in 0..old + sy - collected {
                let line = std::mem::take(dg.get_line_mut(row + collected));
                *dg.get_line_mut(row) = line;
            }
        }
        dg.adjust_lines(total);
        dg.set_hsize_unchecked(new);
        if added > 0 {
            dg.duplicate_lines(kept, sg, kept, added);
        }
        dg.duplicate_lines(new, sg, new, sy);
    }
    dg.hscrolled = sg.hscrolled;
    let dst = data.backing.screen_mut();
    if source.cy > sy - 1 {
        dst.cx = 0;
        dst.cy = sy - 1;
    } else {
        dst.cx = source.cx;
        dst.cy = source.cy;
    }
    true
}

pub fn init_common(
    server: &mut Server,
    mode: ModeId,
    backing: CopyBacking,
    source: PaneId,
) -> Option<Screen> {
    let pane = server.panes.get(mode.owner)?;
    let (sx, sy, window) = (pane.base.grid.sx(), pane.base.grid.sy(), pane.window);
    let options = server.windows.get(window)?.options;
    let modekeys = if server.options.get_number(options, b"mode-keys") == 1 {
        ModeKeys::Vi
    } else {
        ModeKeys::Emacs
    };
    let mut d = CopyModeData::new(backing, source, modekeys);
    let pane = server.panes.get(mode.owner)?;
    d.search.term = pane.searchstr.clone();
    d.search.regex = pane.searchregex;
    d.search.searchtype = if d.search.term.is_some() {
        SearchDirection::Up
    } else {
        SearchDirection::Off
    };
    let mut visible = Screen::new(
        sx,
        sy,
        0,
        ScreenResetPolicy::default(),
        &mut server.hyperlinks,
    )
    .expect("copy visible screen hyperlink lease");
    let global = server.options.global_w;
    let mut cursor = rmux_emu::cell::GridCell::default();
    if let Some(style) =
        server
            .options
            .string_to_style(global, b"cursor-colour", None, &mut server.hyperlinks)
    {
        style.overlay_cell(&mut cursor);
    }
    visible.set_default_cursor(
        cursor.fg,
        server.options.get_number(global, b"cursor-style") as u32,
    );
    let pane = server.panes.get_mut(mode.owner)?;
    let entry = pane.modes.iter_mut().find(|m| m.id == mode)?;
    entry.data = Some(Box::new(d));
    Some(visible)
}
pub fn init_copy(server: &mut Server, mode: ModeId, args: &Args) -> Option<Screen> {
    init_copy_from(server, mode, mode.owner, args)
}
pub fn init_copy_from(
    server: &mut Server,
    mode: ModeId,
    source: PaneId,
    args: &Args,
) -> Option<Screen> {
    let pane = server.panes.get(mode.owner)?;
    let (sx, sy) = (pane.base.grid.sx(), pane.base.grid.sy());
    let src = &server.panes.get(source)?.base;
    let (backing, cx, ay) = clone_screen(src, sx, sy, source != mode.owner, &mut server.hyperlinks);
    let counters = (
        src.grid.scroll_added,
        src.grid.scroll_collected,
        src.grid.scroll_generation,
    );
    let links = src.hyperlinks.as_ref().map(|l| {
        server
            .hyperlinks
            .share(l)
            .expect("copy source hyperlink lease")
    });
    let mut visible = init_common(server, mode, CopyBacking::Snapshot(backing), source)?;
    if let Some(links) = links {
        if let Some(old) = visible.hyperlinks.replace(links) {
            server
                .hyperlinks
                .release(old)
                .expect("copy visible hyperlink release");
        }
    }
    let d = data_mut(server, mode)?;
    let h = d.backing.screen().grid.hsize();
    d.cx = cx;
    if ay < h {
        d.cy = 0;
        d.oy = h - ay;
    } else {
        d.cy = ay - h;
        d.oy = 0;
    }
    d.scroll_exit = args.has(b'e') != 0;
    d.hide_position = args.has(b'H') != 0;
    d.mx = cx;
    d.my = ay;
    d.sync_added = counters.0;
    d.sync_collected = counters.1;
    d.sync_generation = counters.2;
    render::draw_initial(server, mode, &mut visible);
    Some(visible)
}

pub fn free_entry(server: &mut Server, mut mode: PaneMode) {
    if let Some(boxed) = mode.data.take() {
        let mut d = *boxed
            .downcast::<CopyModeData>()
            .expect("copy mode state type");
        if let Some(timer) = d.dragtimer.take() {
            server.event_loop.cancel(timer);
        }
        if let Some(timer) = d.refresh_timer.take() {
            server.event_loop.cancel(timer);
        }
        if let CopyBacking::Output { ground_timer, .. } = &mut d.backing
            && let Some(timer) = ground_timer.take()
        {
            server.event_loop.cancel(timer);
        }
        d.backing
            .screen_mut()
            .release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            )
            .expect("copy backing release");
    }
    if let Some(mut screen) = mode.screen.take() {
        screen
            .release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            )
            .expect("copy visible release");
    }
    for client_id in &server.client_order {
        if let Some(client) = server.clients.get_mut(*client_id) {
            if client.drag.update == Some(crate::client::MouseDragAction::Mode(mode.id)) {
                client.drag.update = None;
            }
            if client.drag.release == Some(crate::client::MouseDragAction::Mode(mode.id)) {
                client.drag.release = None;
            }
        }
    }
}

pub fn resize(server: &mut Server, mode: ModeId, sx: u32, sy: u32) {
    let Some(pane) = server.panes.get_mut(mode.owner) else {
        return;
    };
    let Some(entry) = pane.modes.iter_mut().find(|m| m.id == mode) else {
        return;
    };
    let (Some(boxed), Some(visible)) = (&mut entry.data, &mut entry.screen) else {
        return;
    };
    let Some(d) = boxed.downcast_mut::<CopyModeData>() else {
        return;
    };
    visible.resize(
        sx,
        sy,
        false,
        #[cfg(feature = "sixel")]
        None,
    );
    let gd = &d.backing.screen().grid;
    d.oy = d.oy.min(gd.hsize().saturating_add(d.cy));
    let (mut cx, mut ay) = (d.cx, gd.hsize() + d.cy - d.oy);
    let wrapped = (gd.sx() != sx).then(|| gd.wrap_position(cx, ay));
    d.backing.screen_mut().resize_cursor(
        sx,
        sy,
        true,
        false,
        false,
        #[cfg(feature = "sixel")]
        None,
    );
    if let Some((wx, wy)) = wrapped {
        (cx, ay) = d.backing.screen().grid.unwrap_position(wx, wy);
    }
    let h = d.backing.screen().grid.hsize();
    d.cx = cx;
    if ay < h {
        d.cy = 0;
        d.oy = h - ay;
    } else {
        d.cy = ay - h;
        d.oy = 0;
    }
    size_changed(server, mode);
    render::redraw_screen(server, mode);
}
pub fn size_changed(server: &mut Server, mode: ModeId) {
    let marks = data(server, mode).is_some_and(|d| d.search.marks.is_some());
    select::clear_selection(server, mode);
    search::clear_marks(server, mode);
    let style = render::render_style(server, mode);
    render::with_pane_write(server, mode, true, |d, ctx| {
        let height = ctx.screen.grid.sy();
        render::write_lines(d, ctx, 0, height, &style);
    });
    if marks && data(server, mode).is_some_and(|d| !d.timeout) {
        search::search_marks(server, mode, false);
    }
    if let Some(d) = data_mut(server, mode) {
        d.search.x = Some(d.cx);
        d.search.y = Some(d.cy);
        d.search.o = Some(d.oy);
    }
}
pub fn refresh_allowed(server: &Server, mode: ModeId) -> bool {
    data(server, mode).is_some_and(|d| !d.backing.is_view() && d.source == mode.owner)
}
pub fn do_refresh(server: &mut Server, mode: ModeId, follow: bool) {
    let Some(option) = server
        .panes
        .get(mode.owner)
        .and_then(|p| server.windows.get(p.window))
        .map(|w| w.options)
    else {
        return;
    };
    let vi = server.options.get_number(option, b"mode-keys") == 1;
    let Some(pane) = server.panes.get_mut(mode.owner) else {
        return;
    };
    let (source, modes) = (&pane.base, &mut pane.modes);
    let Some(entry) = modes.iter_mut().find(|m| m.id == mode) else {
        return;
    };
    let Some(d) = entry
        .data
        .as_mut()
        .and_then(|d| d.downcast_mut::<CopyModeData>())
    else {
        return;
    };
    d.oy = d.oy.min(d.backing.screen().grid.hsize());
    let top = d.backing.screen().grid.hsize() - d.oy;
    if !sync_backing(d, source, d.source == mode.owner) {
        let visible = entry.screen.as_ref().expect("copy visible screen");
        let (mut next, _, _) = clone_screen(
            source,
            visible.grid.sx(),
            visible.grid.sy(),
            d.source != mode.owner,
            &mut server.hyperlinks,
        );
        std::mem::swap(d.backing.screen_mut(), &mut next);
        next.release(
            &mut server.hyperlinks,
            #[cfg(feature = "sixel")]
            None,
        )
        .expect("copy replaced backing release");
    }
    let h = d.backing.screen().grid.hsize();
    if follow {
        d.cy = entry.screen.as_ref().expect("copy screen").grid.sy() - 1;
        d.oy = 0;
        let length = d.backing.screen().grid.line_length(h + d.cy);
        d.cx = if vi {
            d.backing.screen().grid.line_limit(h + d.cy)
        } else {
            length
        };
    } else if top <= h {
        d.oy = h - top;
    } else {
        d.cy = 0;
        d.oy = h;
    }
    sync_snapshot(d, &source.grid);
    size_changed(server, mode);
}
fn refresh_arm(server: &mut Server, mode: ModeId) {
    if !data(server, mode).is_some_and(|d| d.refresh_active) {
        return;
    }
    if let Some(old) = data_mut(server, mode).and_then(|d| d.refresh_timer.take()) {
        server.event_loop.cancel(old);
    }
    let timer = server.event_loop.schedule(
        Duration::from_millis(50),
        crate::server::event_loop::LoopAction::CopyTimer(CopyTimerAction::Refresh(mode)),
    );
    if let Some(d) = data_mut(server, mode) {
        d.refresh_timer = Some(timer);
    }
}
pub fn refresh_start(server: &mut Server, mode: ModeId) {
    if !refresh_allowed(server, mode) || data(server, mode).is_some_and(|d| d.refresh_active) {
        return;
    }
    if let Some(d) = data_mut(server, mode) {
        d.refresh_active = true;
    }
    refresh_arm(server, mode);
}
pub fn refresh_stop(server: &mut Server, mode: ModeId) {
    let old = if let Some(d) = data_mut(server, mode) {
        d.refresh_active = false;
        d.refresh_timer.take()
    } else {
        None
    };
    if let Some(old) = old {
        server.event_loop.cancel(old);
    }
}
pub fn refresh_now(server: &mut Server, mode: ModeId) -> bool {
    if !refresh_allowed(server, mode) {
        return false;
    }
    let follow = parts_mut(server, mode).is_some_and(|(d, s)| d.oy == 0 && d.cy == s.grid.sy() - 1);
    do_refresh(server, mode, follow);
    if let Some(p) = server.panes.get_mut(mode.owner) {
        p.flags.remove(crate::model::PaneFlags::UNSEENCHANGES);
    }
    true
}
pub fn refresh_timer(server: &mut Server, mode: ModeId) {
    if !server
        .panes
        .get(mode.owner)
        .is_some_and(|p| p.modes.first().is_some_and(|m| m.id == mode))
        || !data(server, mode).is_some_and(|d| d.refresh_active)
    {
        return;
    }
    if let Some(d) = data_mut(server, mode) {
        d.refresh_timer = None;
    }
    let changed = server
        .panes
        .get(mode.owner)
        .is_some_and(|p| p.flags.contains(crate::model::PaneFlags::UNSEENCHANGES));
    let can = parts_mut(server, mode)
        .is_some_and(|(d, s)| s.selection.is_none() && d.selection.cursordrag == CursorDrag::None);
    if changed && can {
        refresh_now(server, mode);
        render::redraw_screen(server, mode);
        if let Some(p) = server.panes.get_mut(mode.owner) {
            p.flags.insert(crate::model::PaneFlags::REDRAW);
        }
    }
    refresh_arm(server, mode);
}
pub fn timer(server: &mut Server, action: CopyTimerAction) {
    match action {
        CopyTimerAction::Drag(m) => mouse::scroll_timer(server, m),
        CopyTimerAction::Refresh(m) => refresh_timer(server, m),
        CopyTimerAction::ParserGround(m) => view::parser_ground_timer(server, m),
    }
}

#[cfg(test)]
mod tests {
    use super::test_common as common;
    use super::*;
    use crate::ids::ArenaId;
    use rmux_emu::cell::GridCell;
    use rmux_emu::colour::Colour;
    use rmux_emu::grid::GridLineFlags;
    use rmux_emu::hyperlinks::HyperlinkRegistry;
    use rmux_util::utf8::Utf8Data;

    fn source(registry: &mut HyperlinkRegistry, width: u32, height: u32) -> Screen {
        Screen::new(width, height, 100, ScreenResetPolicy::default(), registry).unwrap()
    }
    fn row(screen: &mut Screen, y: u32, bytes: &[u8]) {
        for (x, &byte) in bytes.iter().enumerate() {
            screen.grid.set_cell(
                x as u32,
                y,
                &GridCell {
                    data: Utf8Data::set(byte),
                    ..GridCell::default()
                },
            );
        }
    }
    fn same(a: &Screen, b: &Screen) {
        assert_eq!(
            (
                a.grid.sx(),
                a.grid.sy(),
                a.grid.hsize(),
                a.grid.hscrolled,
                a.cx,
                a.cy
            ),
            (
                b.grid.sx(),
                b.grid.sy(),
                b.grid.hsize(),
                b.grid.hscrolled,
                b.cx,
                b.cy
            )
        );
        for y in 0..a.grid.hsize() + a.grid.sy() {
            let (al, bl) = (a.grid.get_line(y), b.grid.get_line(y));
            assert_eq!(
                (
                    al.cellused(),
                    al.cellsize(),
                    al.extdsize(),
                    al.flags,
                    al.time,
                    al.osc133
                ),
                (
                    bl.cellused(),
                    bl.cellsize(),
                    bl.extdsize(),
                    bl.flags,
                    bl.time,
                    bl.osc133
                )
            );
            for x in 0..al.cellsize() {
                assert_eq!(a.grid.get_cell(x, y), b.grid.get_cell(x, y));
            }
        }
    }
    #[test]
    fn snapshot_trim_keeps_one_viewport_row_and_clamps_cursor() {
        let mut registry = HyperlinkRegistry::new();
        let mut src = source(&mut registry, 10, 4);
        src.cx = 5;
        src.cy = 3;
        let (mut dst, cx, cy) = clone_screen(&src, 10, 1, true, &mut registry);
        assert_eq!(
            (dst.grid.hsize(), dst.grid.sy(), cx, cy, dst.cx, dst.cy),
            (0, 1, 0, 0, 0, 0)
        );
        dst.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
        row(&mut src, 2, b" ");
        let (mut dst, _, _) = clone_screen(&src, 10, 1, true, &mut registry);
        assert_eq!(dst.grid.hsize(), 2);
        assert_eq!(dst.grid.get_line(2).cellused(), 1);
        dst.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
        src.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
    }
    #[test]
    fn snapshot_reflow_tracks_logical_cursor_both_directions() {
        let mut registry = HyperlinkRegistry::new();
        let mut src = source(&mut registry, 8, 2);
        row(&mut src, 0, b"abcdefgh");
        row(&mut src, 1, b"ijkl");
        src.grid
            .get_line_mut(0)
            .flags
            .insert(GridLineFlags::WRAPPED);
        src.cx = 3;
        src.cy = 1;
        let (mut narrow, cx, cy) = clone_screen(&src, 4, 2, false, &mut registry);
        assert_eq!((cx, cy), (3, 2));
        narrow.cx = cx;
        narrow.cy = cy - narrow.grid.hsize();
        let (mut wide, cx, cy) = clone_screen(&narrow, 12, 2, false, &mut registry);
        assert_eq!((cx, cy), (11, 0));
        narrow
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
        wide.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
        src.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
    }
    #[test]
    fn incremental_sync_matches_clone_with_collection_and_wrapped_rows() {
        let mut registry = HyperlinkRegistry::new();
        let mut src = source(&mut registry, 8, 3);
        src.grid.set_hlimit(5);
        for y in 0..3 {
            row(&mut src, y, b"abcdefgh");
        }
        src.grid
            .get_line_mut(1)
            .flags
            .insert(GridLineFlags::WRAPPED);
        for _ in 0..4 {
            src.grid.scroll_history(Colour(8));
        }
        let (backing, _, _) = clone_screen(&src, 8, 3, false, &mut registry);
        let mut d = CopyModeData::new(
            CopyBacking::Snapshot(backing),
            PaneId::from_parts(0, 0),
            ModeKeys::Emacs,
        );
        sync_snapshot(&mut d, &src.grid);
        for step in 0..8 {
            src.grid.scroll_history(Colour(8));
            let bottom = src.grid.hsize() + 2;
            row(&mut src, bottom, &[b'0' + step]);
            if step % 2 == 0 {
                src.grid.collect_history(false);
            }
            src.cx = u32::from(step % 5);
            src.cy = 2;
            assert!(sync_backing(&mut d, &src, true));
            let (mut expected, _, _) = clone_screen(&src, 8, 3, false, &mut registry);
            same(d.backing.screen(), &expected);
            expected
                .release(
                    &mut registry,
                    #[cfg(feature = "sixel")]
                    None,
                )
                .unwrap();
            sync_snapshot(&mut d, &src.grid);
        }
        let top = src.grid.hsize();
        row(&mut src, top, b"changed");
        assert!(sync_backing(&mut d, &src, true));
        let (mut expected, _, _) = clone_screen(&src, 8, 3, false, &mut registry);
        same(d.backing.screen(), &expected);
        expected
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
        d.backing
            .screen_mut()
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
        src.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
    }
    #[test]
    fn incremental_sync_rejects_all_balance_and_identity_failures() {
        let mut registry = HyperlinkRegistry::new();
        let mut src = source(&mut registry, 8, 3);
        let (backing, _, _) = clone_screen(&src, 8, 3, false, &mut registry);
        let mut d = CopyModeData::new(
            CopyBacking::Snapshot(backing),
            PaneId::from_parts(0, 0),
            ModeKeys::Emacs,
        );
        sync_snapshot(&mut d, &src.grid);
        assert!(!sync_backing(&mut d, &src, false));
        src.grid.scroll_generation += 1;
        assert!(!sync_backing(&mut d, &src, true));
        src.grid.scroll_generation -= 1;
        src.grid.set_sx(7);
        assert!(!sync_backing(&mut d, &src, true));
        src.grid.set_sx(8);
        src.grid.set_sy_unchecked(2);
        assert!(!sync_backing(&mut d, &src, true));
        src.grid.set_sy_unchecked(3);
        src.grid.scroll_added = i32::MAX as u32 + 1;
        assert!(!sync_backing(&mut d, &src, true));
        src.grid.scroll_added = 0;
        src.grid.scroll_collected = i32::MAX as u32 + 1;
        assert!(!sync_backing(&mut d, &src, true));
        src.grid.scroll_collected = 1;
        assert!(!sync_backing(&mut d, &src, true));
        src.grid.scroll_collected = 0;
        src.grid.scroll_added = 1;
        assert!(!sync_backing(&mut d, &src, true));
        d.backing
            .screen_mut()
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
        src.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
    }
    #[test]
    fn pinned_clone_cursor_and_grid_differential() {
        use std::fmt::Write;
        use std::path::Path;
        use std::process::Command;
        let Some(pinned) = common::pinned_source() else {
            return;
        };
        let copy = std::fs::read_to_string(pinned.join("window-copy.c")).unwrap();
        let start = copy
            .find("static struct screen *\nwindow_copy_clone_screen")
            .unwrap();
        let end = start + copy[start..].find("\n/*\n * Snapshot").unwrap();
        let driver =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../rmux-emu/tests/screen_reference.c");
        let c = format!(
            "#define main screen_reference_main\n#include {:?}\n#undef main\n{}\nint main(void) {{\nstruct screen src,hint,*dst;u_int i,j,k,cx,cy;struct grid_cell gc;\nsetlocale(LC_CTYPE,\"en_US.UTF-8\");utf8_update_width_cache();\nfor(k=0;k<8;k++){{screen_init(&src,8,4,100);screen_init(&hint,(k%4==0?4:k%4==1?12:8),(k%4==2?1:3),0);\nfor(i=0;i<(k<4?2:0);i++)for(j=0;j<8;j++){{gc=grid_default_cell;utf8_set(&gc.data,'a'+i*8+j);grid_set_cell(src.grid,j,i,&gc);}}\nif(k<4)src.grid->linedata[0].flags|=GRID_LINE_WRAPPED;src.cx=3;src.cy=3;\ndst=window_copy_clone_screen(&src,&hint,&cx,&cy,k>=4);printf(\"case %u %u %u %u %u %u %u %u %u\\n\",k,cx,cy,dst->cx,dst->cy,dst->grid->sx,dst->grid->sy,dst->grid->hsize,dst->grid->hscrolled);\nfor(i=0;i<dst->grid->hsize+dst->grid->sy;i++){{printf(\"row %u %u %u %u\",i,dst->grid->linedata[i].cellsize,dst->grid->linedata[i].cellused,dst->grid->linedata[i].flags);for(j=0;j<dst->grid->sx;j++){{grid_get_cell(dst->grid,j,i,&gc);printf(\" %u/%u/\",gc.flags,gc.data.width);hex(gc.data.data,gc.data.size);}}puts(\"\");}}screen_free(dst);free(dst);screen_free(&hint);screen_free(&src);}}return 0;}}",
            driver,
            &copy[start..end]
        );
        let Some(driver) = common::write_c("copy-snapshot.c", &c) else {
            return;
        };
        let mut flags = vec!["-ffunction-sections", "-levent"];
        if cfg!(target_os = "macos") {
            flags.extend(["-Wl,-dead_strip", "-L/opt/homebrew/opt/libevent/lib"]);
        } else {
            flags.extend(["-Wl,--gc-sections", "-Wl,--no-as-needed"]);
        }
        let Some(binary) = common::build_c(
            "copy-snapshot",
            &[
                &driver,
                Path::new("utf8.c"),
                Path::new("utf8-combined.c"),
                Path::new("xmalloc.c"),
                Path::new("compat/utf8proc.c"),
                Path::new("compat/vis.c"),
                Path::new("compat/strtonum.c"),
                Path::new("compat/reallocarray.c"),
                Path::new("compat/recallocarray.c"),
                Path::new("compat/explicit_bzero.c"),
            ],
            &flags,
            cfg!(target_os = "macos"),
        ) else {
            return;
        };
        let reference = Command::new(binary).output().unwrap();
        assert!(reference.status.success());
        let mut registry = HyperlinkRegistry::new();
        let mut actual = String::new();
        for k in 0..8 {
            let mut src = source(&mut registry, 8, 4);
            if k < 4 {
                row(&mut src, 0, b"abcdefgh");
                row(&mut src, 1, b"ijklmnop");
                src.grid
                    .get_line_mut(0)
                    .flags
                    .insert(GridLineFlags::WRAPPED);
            }
            src.cx = 3;
            src.cy = 3;
            let width = match k % 4 {
                0 => 4,
                1 => 12,
                _ => 8,
            };
            let height = if k % 4 == 2 { 1 } else { 3 };
            let (mut dst, cx, cy) = clone_screen(&src, width, height, k >= 4, &mut registry);
            writeln!(
                actual,
                "case {k} {cx} {cy} {} {} {} {} {} {}",
                dst.cx,
                dst.cy,
                dst.grid.sx(),
                dst.grid.sy(),
                dst.grid.hsize(),
                dst.grid.hscrolled
            )
            .unwrap();
            for y in 0..dst.grid.hsize() + dst.grid.sy() {
                let line = dst.grid.get_line(y);
                write!(
                    actual,
                    "row {y} {} {} {}",
                    line.cellsize(),
                    line.cellused(),
                    line.flags.bits()
                )
                .unwrap();
                for x in 0..dst.grid.sx() {
                    let cell = dst.grid.get_cell(x, y);
                    write!(actual, " {}/{}/", cell.flags.bits(), cell.data.width).unwrap();
                    for b in cell.data.bytes() {
                        write!(actual, "{b:02x}").unwrap();
                    }
                }
                actual.push('\n');
            }
            dst.release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
            src.release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
        }
        if std::env::var_os("RMUX_COPY_SNAPSHOT_MUTATE").is_some() {
            actual.push('!');
        }
        assert_eq!(actual.as_bytes(), reference.stdout);
    }
    fn live_mode() -> (Server, ModeId) {
        let mut server = Server::new();
        let window = crate::model::window::window_create(&mut server, 8, 3, 0, 0).unwrap();
        let pane = crate::model::pane::pane_create(&mut server, window, 8, 3, 10).unwrap();
        server.windows.get_mut(window).unwrap().panes.push(pane);
        server.windows.get_mut(window).unwrap().active = Some(pane);
        let driver = std::rc::Rc::new(CopyModeDriver {
            kind: CopyModeKind::Copy {
                source: None,
                args: Args::default(),
            },
        });
        let mode = crate::model::pane::pane_set_mode(
            &mut server,
            pane,
            b"copy-mode",
            crate::modes::WindowModeFlags::default(),
            driver,
            false,
        )
        .unwrap()
        .unwrap();
        (server, mode)
    }
    #[test]
    fn refresh_pauses_selection_manual_refresh_clears_and_follows() {
        let (mut server, mode) = live_mode();
        let p = server.panes.get_mut(mode.owner).unwrap();
        row(&mut p.base, 2, b"output");
        data_mut(&mut server, mode).unwrap().cy = 2;
        select::start_selection(&mut server, mode);
        refresh_start(&mut server, mode);
        server
            .panes
            .get_mut(mode.owner)
            .unwrap()
            .flags
            .insert(crate::model::PaneFlags::UNSEENCHANGES);
        refresh_timer(&mut server, mode);
        assert!(data(&server, mode).unwrap().selection.active);
        assert!(
            server
                .panes
                .get(mode.owner)
                .unwrap()
                .flags
                .contains(crate::model::PaneFlags::UNSEENCHANGES)
        );
        assert!(data(&server, mode).unwrap().refresh_timer.is_some());
        assert!(refresh_now(&mut server, mode));
        let d = data(&server, mode).unwrap();
        assert!(!d.selection.active);
        assert_eq!((d.cx, d.cy, d.oy), (6, 2, 0));
        assert!(
            !server
                .panes
                .get(mode.owner)
                .unwrap()
                .flags
                .contains(crate::model::PaneFlags::UNSEENCHANGES)
        );
        refresh_stop(&mut server, mode);
        let d = data(&server, mode).unwrap();
        assert!(!d.refresh_active);
        assert!(d.refresh_timer.is_none());
        crate::model::pane::pane_reset_mode(&mut server, mode.owner).unwrap();
    }
    #[test]
    fn resize_rebuilds_marks_and_stale_timer_cannot_target_reused_mode() {
        let (mut server, mode) = live_mode();
        let d = data_mut(&mut server, mode).unwrap();
        row(d.backing.screen_mut(), 0, b"abababab");
        d.search.term = Some(b"ab".to_vec());
        search::search_marks(&mut server, mode, false);
        select::start_selection(&mut server, mode);
        resize(&mut server, mode, 4, 2);
        let d = data(&server, mode).unwrap();
        assert!(!d.selection.active);
        assert_eq!(d.search.marks.as_ref().unwrap().len(), 8);
        assert_eq!(
            (d.search.x, d.search.y, d.search.o),
            (Some(d.cx), Some(d.cy), Some(d.oy))
        );
        refresh_start(&mut server, mode);
        crate::model::pane::pane_reset_mode(&mut server, mode.owner).unwrap();
        let driver = std::rc::Rc::new(CopyModeDriver {
            kind: CopyModeKind::Copy {
                source: None,
                args: Args::default(),
            },
        });
        let next = crate::model::pane::pane_set_mode(
            &mut server,
            mode.owner,
            b"copy-mode",
            crate::modes::WindowModeFlags::default(),
            driver,
            false,
        )
        .unwrap()
        .unwrap();
        assert_ne!(next, mode);
        timer(&mut server, CopyTimerAction::Refresh(mode));
        timer(&mut server, CopyTimerAction::Drag(mode));
        timer(&mut server, CopyTimerAction::ParserGround(mode));
        assert!(!data(&server, next).unwrap().refresh_active);
        assert!(data(&server, next).unwrap().refresh_timer.is_none());
        crate::model::pane::pane_reset_mode(&mut server, mode.owner).unwrap();
    }
    #[test]
    fn buried_refresh_delivery_does_not_rearm() {
        let (mut server, mode) = live_mode();
        refresh_start(&mut server, mode);
        let driver = std::rc::Rc::new(CopyModeDriver {
            kind: CopyModeKind::View,
        });
        crate::model::pane::pane_set_mode(
            &mut server,
            mode.owner,
            b"view-mode",
            crate::modes::WindowModeFlags::default(),
            driver,
            false,
        )
        .unwrap();
        data_mut(&mut server, mode).unwrap().refresh_timer = None;
        refresh_timer(&mut server, mode);
        assert!(data(&server, mode).unwrap().refresh_timer.is_none());
        crate::model::pane::pane_reset_mode_all(&mut server, mode.owner).unwrap();
    }
    #[test]
    fn refresh_preserves_numeric_top_and_clamps_collected_history() {
        let (mut server, mode) = live_mode();
        let src = &mut server.panes.get_mut(mode.owner).unwrap().base;
        for _ in 0..4 {
            src.grid.scroll_history(Colour(8));
        }
        do_refresh(&mut server, mode, false);
        let d = data_mut(&mut server, mode).unwrap();
        d.oy = 2;
        d.cy = 1;
        let src = &mut server.panes.get_mut(mode.owner).unwrap().base;
        src.grid.scroll_history(Colour(8));
        do_refresh(&mut server, mode, false);
        let d = data(&server, mode).unwrap();
        assert_eq!((d.backing.screen().grid.hsize() - d.oy, d.cy), (2, 1));
        let src = &mut server.panes.get_mut(mode.owner).unwrap().base;
        src.grid.set_hlimit(1);
        src.grid.collect_history(true);
        do_refresh(&mut server, mode, false);
        let d = data(&server, mode).unwrap();
        assert_eq!((d.cx, d.cy, d.oy), (0, 0, 1));
        crate::model::pane::pane_reset_mode(&mut server, mode.owner).unwrap();
    }
    #[test]
    fn refresh_disallowed_for_other_source_and_view() {
        let (mut server, mode) = live_mode();
        let window = server.panes.get(mode.owner).unwrap().window;
        let other = crate::model::pane::pane_create(&mut server, window, 8, 3, 10).unwrap();
        data_mut(&mut server, mode).unwrap().source = other;
        assert!(!refresh_allowed(&server, mode));
        refresh_start(&mut server, mode);
        assert!(!refresh_now(&mut server, mode));
        assert!(!data(&server, mode).unwrap().refresh_active);
        crate::model::pane::pane_reset_mode(&mut server, mode.owner).unwrap();
        let driver = std::rc::Rc::new(CopyModeDriver {
            kind: CopyModeKind::View,
        });
        let view = crate::model::pane::pane_set_mode(
            &mut server,
            mode.owner,
            b"view-mode",
            crate::modes::WindowModeFlags::default(),
            driver,
            false,
        )
        .unwrap()
        .unwrap();
        assert!(!refresh_allowed(&server, view));
        refresh_start(&mut server, view);
        assert!(!refresh_now(&mut server, view));
        assert!(!data(&server, view).unwrap().refresh_active);
        crate::model::pane::pane_reset_mode(&mut server, view.owner).unwrap();
    }
}
