// Ported from tmux cmd-join-pane.c @ 8f25579c
/*
 * Copyright (c) 2011 George Nachman <tmux@georgester.com>
 * Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
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
use super::support::{
    concat, fail, item_client, item_event, item_source, item_target, set_item_current,
};
use crate::client::mouse::{MouseDragAction, MouseTarget, ResolvedMouseEvent};
use crate::cmd::{
    Command,
    arguments::Args,
    find::{self, CmdFindFlags, MouseInput},
    queue::CmdReturn,
};
use crate::ids::{ClientId, PaneId, QueueItemId, WindowId};
use crate::layout::{self, LayoutCellFlags, LayoutGeometry, LayoutHost};
use crate::model::{ModelEffect, PaneFlags, WindowFlags, pane, session, spawn::SpawnFlags, window};
use crate::server::{Server, events, operations};
use rmux_util::{
    key::MouseEvent,
    strtonum::{StrtonumError, strtonum},
};

fn changed(server: &mut Server, w: WindowId, invalidate: bool) {
    if invalidate {
        LayoutHost::invalidate_scene(server, w);
    }
    events::fire_window(server, b"window-layout-changed", w);
    operations::server_redraw_window(server, w);
}

fn position(
    position: &[u8],
    wx: i32,
    wy: i32,
    g: LayoutGeometry,
    border: i32,
) -> Option<(i32, i32)> {
    let (px, py) = (g.sx as i32, g.sy as i32);
    Some(match position {
        b"top-left" => (border, border),
        b"top-centre" | b"top-center" => ((wx - px) / 2, border),
        b"top-right" => (wx - px - border, border),
        b"centre-left" | b"center-left" => (border, (wy - py) / 2),
        b"centre" | b"center" => ((wx - px) / 2, (wy - py) / 2),
        b"centre-right" | b"center-right" => (wx - px - border, (wy - py) / 2),
        b"bottom-left" => (border, wy - py - border),
        b"bottom-centre" | b"bottom-center" => ((wx - px) / 2, wy - py - border),
        b"bottom-right" => (wx - px - border, wy - py - border),
        b"top-left-centre" | b"top-left-center" => (wx / 4 - px / 2, wy / 4 - py / 2),
        b"top-right-centre" | b"top-right-center" => (3 * wx / 4 - px / 2, wy / 4 - py / 2),
        b"bottom-left-centre" | b"bottom-left-center" => (wx / 4 - px / 2, 3 * wy / 4 - py / 2),
        b"bottom-right-centre" | b"bottom-right-center" => {
            (3 * wx / 4 - px / 2, 3 * wy / 4 - py / 2)
        }
        _ => return None,
    })
}

fn place(
    server: &mut Server,
    item: QueueItemId,
    w: WindowId,
    wp: PaneId,
    value: &[u8],
) -> CmdReturn {
    let lc = server
        .panes
        .get(wp)
        .and_then(|p| p.layout_cell)
        .expect("floating layout cell");
    let g = server.layout_cells.get(lc).expect("floating layout cell").g;
    let window = server.windows.get(w).expect("target window");
    let border = i32::from(pane::pane_get_pane_lines(server, wp) != 6);
    if let Some((xoff, yoff)) = position(value, window.sx as i32, window.sy as i32, g, border) {
        if xoff != g.xoff || yoff != g.yoff {
            let g = &mut server
                .layout_cells
                .get_mut(lc)
                .expect("floating layout cell")
                .g;
            g.xoff = xoff;
            g.yoff = yoff;
            layout::fix_panes(server, w, None);
        }
    } else {
        let current = window
            .z_order
            .iter()
            .position(|p| *p == wp)
            .expect("pane z-order");
        let previous = if matches!(value, b"forward" | b"forward-loop") {
            window.z_order[..current].iter().rev().copied().find(|p| {
                server
                    .panes
                    .get(*p)
                    .is_some_and(|p| p.layout_cell.is_some())
            })
        } else {
            None
        };
        let next = if matches!(value, b"backward" | b"backward-loop") {
            window.z_order[current + 1..].iter().copied().find(|p| {
                server
                    .panes
                    .get(*p)
                    .is_some_and(|p| p.layout_cell.is_some())
            })
        } else {
            None
        };
        let back = |server: &Server| {
            server
                .windows
                .get(w)
                .expect("target window")
                .z_order
                .iter()
                .copied()
                .find(|p| *p != wp && !pane::pane_is_floating_with_hidden(server, *p))
        };
        let (anchor, after) = match value {
            b"front" => (None, false),
            b"back" => (back(server), false),
            b"forward" if previous.is_none() => {
                changed(server, w, true);
                return CmdReturn::Normal;
            }
            b"forward" => (previous, false),
            b"backward" if next.is_none_or(|p| !pane::pane_is_floating(server, p)) => {
                changed(server, w, true);
                return CmdReturn::Normal;
            }
            b"backward" => (next, true),
            b"forward-loop" => (previous.or_else(|| back(server)), false),
            b"backward-loop" => (next.filter(|p| pane::pane_is_floating(server, *p)), true),
            _ => return fail(server, item, concat(&[b"unknown position: ", value])),
        };
        let front = value == b"front" || (value == b"backward-loop" && anchor.is_none());
        let z = &mut server.windows.get_mut(w).expect("target window").z_order;
        z.retain(|p| *p != wp);
        let index = anchor
            .and_then(|p| z.iter().position(|other| *other == p))
            .map(|index| index + usize::from(after))
            .unwrap_or(if front { 0 } else { z.len() });
        z.insert(index, wp);
    }
    changed(server, w, true);
    CmdReturn::Normal
}

fn number_error(error: StrtonumError) -> &'static [u8] {
    match error {
        StrtonumError::Invalid => b"invalid",
        StrtonumError::TooSmall => b"too small",
        StrtonumError::TooLarge => b"too large",
    }
}

fn offsets(
    server: &mut Server,
    item: QueueItemId,
    args: &Args,
    w: WindowId,
    wp: PaneId,
) -> CmdReturn {
    let lc = server
        .panes
        .get(wp)
        .and_then(|p| p.layout_cell)
        .expect("floating layout cell");
    let geometry = server.layout_cells.get(lc).expect("floating layout cell").g;
    let window = server.windows.get(w).expect("target window");
    let (wx, wy) = (i64::from(window.sx), i64::from(window.sy));
    let border = i32::from(pane::pane_get_pane_lines(server, wp) != 6);
    let (mut xoff, mut yoff) = (geometry.xoff, geometry.yoff);
    for (flag, dimension, offset) in [(b'X', wx, &mut xoff), (b'Y', wy, &mut yoff)] {
        if args.has(flag) != 0 {
            match args.percentage_and_expand(server, flag, -dimension, dimension, dimension, item) {
                Ok(value) => *offset = (value as i32).wrapping_add(border),
                Err(error) => return fail(server, item, concat(&[b"position ", &error])),
            }
        }
    }
    for &flag in b"UDLR" {
        if args.has(flag) == 0 {
            continue;
        }
        let value = match strtonum(
            args.get(flag).unwrap_or(b"1"),
            i64::from(i32::MIN),
            i64::from(i32::MAX),
        ) {
            Ok(value) => value as i32,
            Err(error) => return fail(server, item, concat(&[b"offset ", number_error(error)])),
        };
        match flag {
            b'U' => yoff = yoff.wrapping_sub(value),
            b'D' => yoff = yoff.wrapping_add(value),
            b'L' => xoff = xoff.wrapping_sub(value),
            _ => xoff = xoff.wrapping_add(value),
        }
    }
    if xoff != geometry.xoff || yoff != geometry.yoff {
        let g = &mut server
            .layout_cells
            .get_mut(lc)
            .expect("floating layout cell")
            .g;
        g.xoff = xoff;
        g.yoff = yoff;
        layout::fix_panes(server, w, None);
        changed(server, w, false);
    }
    CmdReturn::Normal
}

fn zindex(
    server: &mut Server,
    item: QueueItemId,
    w: WindowId,
    wp: PaneId,
    value: &[u8],
) -> CmdReturn {
    let z = match strtonum(value, 0, i64::from(u32::MAX)) {
        Ok(z) => z as u32,
        Err(error) => return fail(server, item, concat(&[b"z-index ", number_error(error)])),
    };
    server
        .windows
        .get_mut(w)
        .expect("target window")
        .z_order
        .retain(|p| *p != wp);
    let order = &server.windows.get(w).expect("target window").z_order;
    let mut count = 0;
    let index = order
        .iter()
        .position(|p| {
            if !pane::pane_is_floating_with_hidden(server, *p) {
                return true;
            }
            if server.panes.get(*p).is_none_or(|p| p.layout_cell.is_none()) {
                return false;
            }
            if count >= z {
                return true;
            }
            count += 1;
            false
        })
        .unwrap_or(order.len());
    server
        .windows
        .get_mut(w)
        .expect("target window")
        .z_order
        .insert(index, wp);
    changed(server, w, true);
    CmdReturn::Normal
}

fn tile(
    server: &mut Server,
    command: &Command,
    item: QueueItemId,
    w: WindowId,
    wp: PaneId,
) -> CmdReturn {
    if !pane::pane_is_floating(server, wp) {
        return fail(server, item, b"pane is not floating");
    }
    if server
        .windows
        .get(w)
        .is_some_and(|w| w.flags.contains(WindowFlags::ZOOMED))
    {
        return fail(server, item, b"can't tile a pane while window is zoomed");
    }
    let lc = server
        .panes
        .get(wp)
        .and_then(|p| p.layout_cell)
        .expect("floating layout cell");
    let cell = server
        .layout_cells
        .get_mut(lc)
        .expect("floating layout cell");
    cell.fg = cell.g;
    if !layout::insert_tile(server, w, lc) {
        return fail(server, item, b"no space for a new pane");
    }
    server
        .layout_cells
        .get_mut(lc)
        .expect("floating layout cell")
        .flags
        .remove(LayoutCellFlags::FLOATING);
    let z = &mut server.windows.get_mut(w).expect("target window").z_order;
    z.retain(|p| *p != wp);
    z.push(wp);
    if command.args.has(b'd') == 0 {
        let _ = window::window_set_active_pane(server, w, wp, true);
    }
    layout::fix_offsets(server, w);
    layout::fix_panes(server, w, None);
    changed(server, w, true);
    CmdReturn::Normal
}

fn mouse_input(ev: &ResolvedMouseEvent) -> MouseInput {
    MouseInput {
        valid: ev.target.valid,
        session: ev.target.session,
        window: ev.target.window,
        pane: ev.target.pane,
        x: ev.event.x,
        y: ev.event.y,
        last_x: ev.event.lx,
        last_y: ev.event.ly,
        offset_x: ev.target.ox,
        offset_y: ev.target.oy,
        status_at: ev.target.status_at,
        status_lines: ev.target.status_lines,
    }
}

fn mouse_start(server: &mut Server, item: QueueItemId) -> CmdReturn {
    let m = item_event(server, item).mouse;
    let Some((s, wl, wp)) = find::mouse_pane(server, &m) else {
        return CmdReturn::Normal;
    };
    let Some(client) = item_client(server, item) else {
        return CmdReturn::Normal;
    };
    if server
        .clients
        .get(client)
        .is_none_or(|c| c.session != Some(s))
        || !pane::pane_is_floating(server, wp)
    {
        return CmdReturn::Normal;
    }
    let w = server.winlinks.get(wl).expect("mouse window link").window;
    if window::window_redraw_active_switch(server, w, Some(wp)).is_err() {
        return CmdReturn::Error;
    }
    let _ = window::window_set_active_pane(server, w, wp, true);
    server
        .clients
        .get_mut(client)
        .expect("command client")
        .drag
        .update = Some(MouseDragAction::MoveFloating);
    let ev = ResolvedMouseEvent {
        event: MouseEvent {
            x: m.x,
            y: m.y,
            lx: m.last_x,
            ly: m.last_y,
            ..MouseEvent::default()
        },
        target: MouseTarget {
            valid: m.valid,
            session: m.session,
            window: m.window,
            pane: m.pane,
            status_at: m.status_at,
            status_lines: m.status_lines,
            ox: m.offset_x,
            oy: m.offset_y,
            ..MouseTarget::default()
        },
    };
    drag_update(server, client, &ev);
    CmdReturn::Normal
}

fn mouse_position(x: u32, y: u32, m: &MouseInput) -> (i32, i32) {
    let x = x.wrapping_add(m.offset_x) as i32;
    let mut y = y.wrapping_add(m.offset_y) as i32;
    if m.status_at == 0 && y >= m.status_lines as i32 {
        y = y.wrapping_sub(m.status_lines as i32);
    } else if m.status_at > 0 && y >= m.status_at {
        y = m.status_at - 1;
    }
    (x, y)
}

pub fn drag_update(server: &mut Server, client: ClientId, ev: &ResolvedMouseEvent) {
    let m = mouse_input(ev);
    let Some((_, wl, wp)) = find::mouse_pane(server, &m) else {
        if let Some(c) = server.clients.get_mut(client) {
            c.drag.update = None;
        }
        return;
    };
    let w = server.winlinks.get(wl).expect("mouse window link").window;
    let p = server.panes.get(wp).expect("mouse pane");
    let Some(lc) = p.layout_cell else {
        return;
    };
    let old = (p.xoff, p.yoff, p.sx, p.sy);
    let (x, y) = mouse_position(m.x, m.y, &m);
    let (lx, ly) = mouse_position(m.last_x, m.last_y, &m);
    if x != lx || y != ly {
        let g = &mut server
            .layout_cells
            .get_mut(lc)
            .expect("mouse pane layout")
            .g;
        g.xoff = g.xoff.wrapping_add(x.wrapping_sub(lx));
        g.yoff = g.yoff.wrapping_add(y.wrapping_sub(ly));
        layout::fix_panes(server, w, None);
        window::window_redraw_floating_pane(server, wp, old.0, old.1, old.2, old.3);
        operations::server_redraw_window_borders(server, w);
    }
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let (Some(dst_s), Some(dst_wl), Some(dst_wp)) = (target.s, target.wl, target.wp) else {
        return CmdReturn::Error;
    };
    let Some((dst_w, dst_idx)) = server.winlinks.get(dst_wl).map(|wl| (wl.window, wl.index)) else {
        return CmdReturn::Error;
    };
    if std::ptr::eq(command.entry, &crate::cmd::metadata::CMD_MOVE_PANE) {
        if args.has(b'M') != 0 {
            return mouse_start(server, item);
        }
        if b"PzXYUDLR".iter().any(|flag| args.has(*flag) != 0) {
            if !pane::pane_is_floating(server, dst_wp) {
                return fail(server, item, b"pane is not floating");
            }
            if let Some(value) = args.get(b'P') {
                return place(server, item, dst_w, dst_wp, value);
            }
            if let Some(value) = args.get(b'z') {
                return zindex(server, item, dst_w, dst_wp, value);
            }
            return offsets(server, item, args, dst_w, dst_wp);
        }
    }
    let source = item_source(server, item);
    let (Some(src_wl), Some(src_wp)) = (source.wl, source.wp) else {
        return CmdReturn::Error;
    };
    let Some((src_w, src_idx)) = server.winlinks.get(src_wl).map(|wl| (wl.window, wl.index)) else {
        return CmdReturn::Error;
    };
    if server
        .windows
        .get(src_w)
        .is_some_and(|w| w.modal == Some(src_wp))
        || server
            .windows
            .get(dst_w)
            .is_some_and(|w| w.modal == Some(dst_wp))
    {
        return fail(server, item, b"pane is modal");
    }
    let _ = operations::server_unzoom_window(server, dst_w);
    let _ = operations::server_unzoom_window(server, src_w);
    if src_wp == dst_wp {
        if pane::pane_is_floating(server, src_wp) {
            return tile(server, command, item, src_w, src_wp);
        }
        return fail(server, item, b"source and target panes must be different");
    }
    let mut flags = SpawnFlags::default();
    if args.has(b'h') != 0 {
        flags.insert(SpawnFlags::HORIZONTAL);
    }
    if args.has(b'b') != 0 {
        flags.insert(SpawnFlags::BEFORE);
    }
    if args.has(b'f') != 0 {
        flags.insert(SpawnFlags::FULLSIZE);
    }
    let lc = match layout::get_tiled_cell(server, item, args, dst_w, dst_wp, flags) {
        Ok(lc) => lc,
        Err(error) => return fail(server, item, concat(&[b"size or position ", &error.cause])),
    };
    layout::close_pane(server, src_wp);
    crate::client::mouse::remove_pane(server, src_wp);
    let _ = window::window_lost_pane(server, src_w, src_wp);
    let source = server.windows.get_mut(src_w).expect("source window");
    source.panes.retain(|p| *p != src_wp);
    source.z_order.retain(|p| *p != src_wp);
    let options = server
        .windows
        .get(dst_w)
        .expect("destination window")
        .options;
    let p = server.panes.get_mut(src_wp).expect("source pane");
    p.window = dst_w;
    p.flags
        .insert(PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
    server.options.set_parent(p.options, Some(options));
    let destination = server.windows.get_mut(dst_w).expect("destination window");
    for order in [&mut destination.panes, &mut destination.z_order] {
        let index = order
            .iter()
            .position(|p| *p == dst_wp)
            .expect("destination pane membership")
            + usize::from(!flags.contains(SpawnFlags::BEFORE));
        order.insert(index, src_wp);
    }
    layout::assign_pane(server, lc, src_wp, false);
    super::swap_pane::palette_from_option(server, src_wp);
    server.effects.push_back(ModelEffect::RecalculateSizes);
    operations::server_redraw_window(server, src_w);
    operations::server_redraw_window(server, dst_w);
    if args.has(b'd') == 0 {
        let _ = window::window_set_active_pane(server, dst_w, src_wp, true);
        session::session_select(server, dst_s, dst_idx);
        let current = find::from_session(server, dst_s, CmdFindFlags::default());
        set_item_current(server, item, &current);
        operations::server_redraw_session(server, dst_s);
    } else {
        operations::server_status_session(server, dst_s);
    }
    window::window_fire_pane_moved(server, src_wp, src_w, src_idx, dst_w, dst_idx);
    if window::window_count_panes(server, src_w, true) == 0 {
        let _ = operations::server_kill_window(server, src_w, true);
    } else {
        events::fire_window(server, b"window-layout-changed", src_w);
    }
    events::fire_window(server, b"window-layout-changed", dst_w);
    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::super::break_pane::tests::{
        command, create_session, create_window, item, source, split,
    };
    use super::*;
    use crate::cmd::metadata;
    use crate::ids::WinlinkId;
    use crate::server::events::EventPayload;

    fn fixture() -> (Server, WindowId, WinlinkId, PaneId, PaneId, PaneId) {
        let mut server = Server::default();
        let s = create_session(&mut server, b"floating");
        let (w, wl, tiled) = create_window(&mut server, s, 0);
        let add = |server: &mut Server, x| {
            let g = LayoutGeometry {
                sx: 20,
                sy: 6,
                xoff: x,
                yoff: 4,
            };
            let lc = layout::floating_pane(server, w, Some(tiled), &g);
            let p =
                window::window_add_pane(server, w, Some(tiled), 10, SpawnFlags::FLOATING).unwrap();
            layout::assign_pane(server, lc, p, false);
            p
        };
        let a = add(&mut server, 4);
        let b = add(&mut server, 8);
        (server, w, wl, tiled, a, b)
    }

    fn geometry(server: &Server, p: PaneId) -> LayoutGeometry {
        let lc = server.panes.get(p).unwrap().layout_cell.unwrap();
        server.layout_cells.get(lc).unwrap().g
    }

    fn record_layout(server: &mut Server, payload: &mut EventPayload) {
        server.emit(b"test-layout", None, payload.get_window(b"window"), None);
    }

    fn layout_events(server: &Server) -> usize {
        server.effects.iter().filter(|effect| matches!(effect, ModelEffect::Event { name, .. } if name == b"test-layout")).count()
    }

    #[test]
    fn named_positions_match_pinned_signed_integer_centres() {
        let g = LayoutGeometry {
            sx: 20,
            sy: 6,
            xoff: 0,
            yoff: 0,
        };
        let cases: &[(&[u8], (i32, i32))] = &[
            (b"top-left", (1, 1)),
            (b"top-center", (30, 1)),
            (b"top-right", (59, 1)),
            (b"center-left", (1, 9)),
            (b"centre", (30, 9)),
            (b"center-right", (59, 9)),
            (b"bottom-left", (1, 17)),
            (b"bottom-center", (30, 17)),
            (b"bottom-right", (59, 17)),
            (b"top-left-center", (10, 3)),
            (b"top-right-centre", (50, 3)),
            (b"bottom-left-centre", (10, 15)),
            (b"bottom-right-center", (50, 15)),
        ];
        for (name, expected) in cases {
            assert_eq!(position(name, 80, 24, g, 1), Some(*expected));
        }
        assert_eq!(position(b"top-left", 80, 24, g, 0), Some((0, 0)));
        assert_eq!(position(b"center", 17, 3, g, 1), Some((-1, -1)));
        assert_eq!(position(b"unknown", 80, 24, g, 1), None);
    }

    #[test]
    fn placement_and_numeric_z_order_stop_before_tiled_panes() {
        let (mut server, w, wl, tiled, a, b) = fixture();
        let current = source(&server, wl, a);
        let item = item(&mut server, current, current);
        assert_eq!(server.windows.get(w).unwrap().z_order, [b, a, tiled]);
        for (value, expected) in [
            (b"front".as_slice(), [a, b, tiled]),
            (b"forward-loop".as_slice(), [b, a, tiled]),
            (b"backward-loop".as_slice(), [a, b, tiled]),
            (b"back".as_slice(), [b, a, tiled]),
            (b"forward".as_slice(), [a, b, tiled]),
            (b"backward".as_slice(), [b, a, tiled]),
        ] {
            assert_eq!(place(&mut server, item, w, a, value), CmdReturn::Normal);
            assert_eq!(server.windows.get(w).unwrap().z_order, expected);
        }
        assert_eq!(zindex(&mut server, item, w, a, b"0"), CmdReturn::Normal);
        assert_eq!(server.windows.get(w).unwrap().z_order, [a, b, tiled]);
        assert_eq!(
            zindex(&mut server, item, w, a, b"4294967295"),
            CmdReturn::Normal
        );
        assert_eq!(server.windows.get(w).unwrap().z_order, [b, a, tiled]);
        let before = server.windows.get(w).unwrap().z_order.clone();
        assert_eq!(
            zindex(&mut server, item, w, a, b"4294967296"),
            CmdReturn::Error
        );
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"z-index too large"
        );
        assert_eq!(server.windows.get(w).unwrap().z_order, before);
    }

    #[test]
    fn hidden_floating_entries_are_skipped_but_retained_in_z_order() {
        let (mut server, w, wl, tiled, a, b) = fixture();
        let lc = server.panes.get(b).unwrap().layout_cell.unwrap();
        let p = server.panes.get_mut(b).unwrap();
        p.saved_layout_cell = Some(lc);
        p.layout_cell = None;
        server.windows.get_mut(w).unwrap().z_order = vec![a, b, tiled];
        let current = source(&server, wl, a);
        let item = item(&mut server, current, current);
        assert_eq!(place(&mut server, item, w, a, b"back"), CmdReturn::Normal);
        assert_eq!(server.windows.get(w).unwrap().z_order, [b, a, tiled]);
        assert_eq!(
            place(&mut server, item, w, a, b"forward"),
            CmdReturn::Normal
        );
        assert_eq!(server.windows.get(w).unwrap().z_order, [b, a, tiled]);
        assert_eq!(zindex(&mut server, item, w, a, b"0"), CmdReturn::Normal);
        assert_eq!(server.windows.get(w).unwrap().z_order, [b, a, tiled]);
    }

    #[test]
    fn offsets_are_atomic_and_only_changes_fire_layout_events() {
        let (mut server, w, wl, _, a, _) = fixture();
        server
            .events
            .add_sink(b"window-layout-changed", record_layout);
        let current = source(&server, wl, a);
        let item = item(&mut server, current, current);
        let old = geometry(&server, a);
        let invalid = command(
            &metadata::CMD_MOVE_PANE,
            &[
                (b'X', Some(b"50%")),
                (b'U', Some(b"2")),
                (b'R', Some(b"bad")),
            ],
        );
        assert_eq!(execute(&mut server, &invalid, item), CmdReturn::Error);
        assert_eq!(geometry(&server, a), old);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"offset invalid"
        );
        assert_eq!(layout_events(&server), 0);
        let valid = command(
            &metadata::CMD_MOVE_PANE,
            &[
                (b'X', Some(b"50%")),
                (b'Y', Some(b"25%")),
                (b'U', Some(b"-2")),
                (b'D', None),
                (b'L', Some(b"-3")),
                (b'R', None),
            ],
        );
        assert_eq!(execute(&mut server, &valid, item), CmdReturn::Normal);
        let g = geometry(&server, a);
        assert_eq!((g.xoff, g.yoff), (45, 10));
        assert_eq!(layout_events(&server), 1);
        let unchanged = command(&metadata::CMD_MOVE_PANE, &[(b'R', Some(b"0"))]);
        assert_eq!(execute(&mut server, &unchanged, item), CmdReturn::Normal);
        assert_eq!(layout_events(&server), 1);
        assert_eq!(place(&mut server, item, w, a, b"back"), CmdReturn::Normal);
        assert_eq!(layout_events(&server), 2);
    }

    #[test]
    fn placement_addresses_target_and_wins_over_other_flags() {
        let (mut server, w, wl, tiled, a, _) = fixture();
        let src = source(&server, wl, tiled);
        let dst = source(&server, wl, a);
        let item = item(&mut server, src, dst);
        let command = command(
            &metadata::CMD_MOVE_PANE,
            &[
                (b'P', Some(b"top-left")),
                (b'z', Some(b"bad")),
                (b'R', Some(b"bad")),
            ],
        );
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert_eq!(
            (geometry(&server, a).xoff, geometry(&server, a).yoff),
            (1, 1)
        );
        assert_eq!(server.panes.get(tiled).unwrap().window, w);
        let queued = server.queue.items.get_mut(item).unwrap();
        queued.target = src;
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"pane is not floating"
        );
    }

    #[test]
    fn same_floating_pane_tiles_preserving_geometry_and_moves_to_back() {
        let (mut server, w, wl, _, a, _) = fixture();
        let old = geometry(&server, a);
        let current = source(&server, wl, a);
        let item = item(&mut server, current, current);
        let command = command(&metadata::CMD_JOIN_PANE, &[]);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert!(!pane::pane_is_floating(&server, a));
        let lc = server.panes.get(a).unwrap().layout_cell.unwrap();
        assert_eq!(server.layout_cells.get(lc).unwrap().fg, old);
        assert_eq!(server.windows.get(w).unwrap().z_order.last(), Some(&a));
        assert_eq!(server.windows.get(w).unwrap().active, Some(a));
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"source and target panes must be different"
        );
    }

    #[test]
    fn cross_window_transfer_reuses_pane_and_destroys_empty_source() {
        let mut server = Server::default();
        let s = create_session(&mut server, b"join");
        let (src_w, src_wl, src_p) = create_window(&mut server, s, 0);
        let (dst_w, dst_wl, dst_p) = create_window(&mut server, s, 1);
        let src = source(&server, src_wl, src_p);
        let dst = source(&server, dst_wl, dst_p);
        let item = item(&mut server, src, dst);
        let command = command(
            &metadata::CMD_JOIN_PANE,
            &[(b'h', None), (b'b', None), (b'l', Some(b"20"))],
        );
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert_eq!(server.panes.get(src_p).unwrap().window, dst_w);
        assert_eq!(server.windows.get(dst_w).unwrap().panes, [src_p, dst_p]);
        assert_eq!(server.windows.get(dst_w).unwrap().active, Some(src_p));
        assert_eq!(server.pane_ids.len(), 2);
        assert!(server.windows.get(src_w).is_none());
        assert!(server.winlinks.get(src_wl).is_none());
        let queued = server.queue.items.get(item).unwrap();
        assert_eq!(
            server.queue.states.get(queued.state).unwrap().current.wp,
            Some(src_p)
        );
        assert_eq!(geometry(&server, src_p).sx, 20);
    }

    #[test]
    fn same_window_transfer_emits_both_final_layout_events() {
        let mut server = Server::default();
        let s = create_session(&mut server, b"same");
        let (w, wl, a) = create_window(&mut server, s, 0);
        let b = split(&mut server, w, a);
        server
            .events
            .add_sink(b"window-layout-changed", record_layout);
        let src = source(&server, wl, a);
        let dst = source(&server, wl, b);
        let item = item(&mut server, src, dst);
        let command = command(&metadata::CMD_MOVE_PANE, &[(b'd', None)]);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert_eq!(layout_events(&server), 2);
        assert_eq!(server.windows.get(w).unwrap().panes, [b, a]);
        let queued = server.queue.items.get(item).unwrap();
        assert_eq!(server.queue.states.get(queued.state).unwrap().current, src);
    }

    #[test]
    fn invalid_size_leaves_panes_but_keeps_prior_unzoom() {
        let mut server = Server::default();
        let s = create_session(&mut server, b"unzoom");
        let (w, wl, a) = create_window(&mut server, s, 0);
        let b = split(&mut server, w, a);
        window::window_zoom(&mut server, w, a).unwrap();
        let src = source(&server, wl, a);
        let dst = source(&server, wl, b);
        let item = item(&mut server, src, dst);
        let command = command(&metadata::CMD_JOIN_PANE, &[(b'l', Some(b"bad"))]);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"size or position invalid tiled geometry invalid"
        );
        assert!(
            !server
                .windows
                .get(w)
                .unwrap()
                .flags
                .contains(WindowFlags::ZOOMED)
        );
        assert_eq!(server.windows.get(w).unwrap().panes, [a, b]);
    }

    #[test]
    fn mouse_drag_uses_viewport_status_offsets_and_clears_missing_target() {
        let (mut server, w, wl, _, a, _) = fixture();
        let current = source(&server, wl, a);
        let item = item(&mut server, current, current);
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = current.s;
        let client = server.clients.insert(client).unwrap();
        let queued = server.queue.items.get_mut(item).unwrap();
        queued.client = Some(client);
        let state = queued.state;
        server.queue.states.get_mut(state).unwrap().event.mouse = MouseInput {
            valid: true,
            session: current.s,
            window: Some(w),
            pane: Some(a),
            x: 8,
            y: 7,
            last_x: 5,
            last_y: 5,
            offset_x: 10,
            offset_y: 3,
            status_at: 0,
            status_lines: 2,
        };
        let command = command(
            &metadata::CMD_MOVE_PANE,
            &[(b'M', None), (b'P', Some(b"invalid"))],
        );
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert_eq!(
            server.clients.get(client).unwrap().drag.update,
            Some(MouseDragAction::MoveFloating)
        );
        assert_eq!(
            (geometry(&server, a).xoff, geometry(&server, a).yoff),
            (7, 6)
        );
        assert_eq!(server.windows.get(w).unwrap().active, Some(a));
        let m = MouseInput {
            offset_x: 4,
            offset_y: 2,
            status_at: 10,
            ..Default::default()
        };
        assert_eq!(mouse_position(3, 20, &m), (7, 9));
        drag_update(&mut server, client, &ResolvedMouseEvent::default());
        assert_eq!(server.clients.get(client).unwrap().drag.update, None);
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::super::break_pane::tests::{
        command, create_session, create_window, item, source, split,
    };
    use super::*;
    use crate::cmd::metadata;

    #[test]
    fn modal_rejection_does_not_unzoom_or_remove_panes() {
        let mut server = Server::default();
        let s = create_session(&mut server, b"modal");
        let (w, wl, a) = create_window(&mut server, s, 0);
        let b = split(&mut server, w, a);
        window::window_zoom(&mut server, w, a).unwrap();
        server.windows.get_mut(w).unwrap().modal = Some(a);
        let src = source(&server, wl, a);
        let dst = source(&server, wl, b);
        let item = item(&mut server, src, dst);
        let command = command(&metadata::CMD_JOIN_PANE, &[]);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"pane is modal"
        );
        assert!(
            server
                .windows
                .get(w)
                .unwrap()
                .flags
                .contains(WindowFlags::ZOOMED)
        );
        assert_eq!(server.windows.get(w).unwrap().panes, [a, b]);
    }

    #[test]
    fn invalid_named_position_keeps_geometry_and_order() {
        let mut server = Server::default();
        let s = create_session(&mut server, b"position");
        let (w, wl, p) = create_window(&mut server, s, 0);
        let current = source(&server, wl, p);
        let item = item(&mut server, current, current);
        let float = command(&metadata::CMD_BREAK_PANE, &[(b'W', None)]);
        assert_eq!(
            super::super::break_pane::execute(&mut server, &float, item),
            CmdReturn::Normal
        );
        let lc = server.panes.get(p).unwrap().layout_cell.unwrap();
        let old = server.layout_cells.get(lc).unwrap().g;
        let invalid = command(&metadata::CMD_MOVE_PANE, &[(b'P', Some(b"invalid"))]);
        assert_eq!(execute(&mut server, &invalid, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"unknown position: invalid"
        );
        assert_eq!(server.layout_cells.get(lc).unwrap().g, old);
        assert_eq!(server.windows.get(w).unwrap().z_order, [p]);
        let invalid = command(&metadata::CMD_MOVE_PANE, &[(b'X', Some(b"81"))]);
        assert_eq!(execute(&mut server, &invalid, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"position too large"
        );
        assert_eq!(server.layout_cells.get(lc).unwrap().g, old);
    }
}
