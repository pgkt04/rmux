// Ported from tmux cmd-resize-pane.c @ 8f25579c
use super::support::{concat, fail, item_client, item_event, item_target};
use crate::client::mouse::{MouseDragAction, ResolvedMouseEvent};
use crate::cmd::find::{self, MouseInput};
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::{ClientId, LayoutCellId, PaneId, QueueItemId, WindowId};
use crate::layout::{self, LayoutGeometry, LayoutType, PANE_MAXIMUM, PANE_MINIMUM};
use crate::model::pane::{pane_is_floating, pane_scrollbar_reserve};
use crate::model::window::{
    window_get_pane_status, window_redraw_active_switch, window_set_active_pane, window_unzoom,
    window_zoom,
};
use crate::model::{PaneFlags, WindowFlags};
use crate::server::Server;
use crate::server::events;
use crate::server::operations::{
    server_redraw_window, server_redraw_window_borders, server_unzoom_window,
};
use crate::ui::scrollbar::PaneScrollbarPosition;

/// `PANE_STATUS_TOP` / `PANE_STATUS_BOTTOM` as `window_get_pane_status` returns them.
const PANE_STATUS_TOP: i64 = 1;
const PANE_STATUS_BOTTOM: i64 = 2;

/// One `U D L R` step of `cmd-resize-pane.c:163-181`: the layout axis, the
/// signed change handed to the layout, and (floating only) the sticky
/// `opposite` flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResizeStep {
    pub kind: LayoutType,
    pub adjust: i32,
    pub opposite: bool,
}

/// cmd-resize-pane.c:163-181. `opposite` is the loop-level variable of line
/// 67: once `L` or `U` sets it, later `D` and `R` keep it.
pub fn resize_step(flag: u8, adjust: i32, floating: bool, opposite: &mut bool) -> ResizeStep {
    let kind = if flag == b'L' || flag == b'R' {
        LayoutType::Leftright
    } else {
        LayoutType::Topbottom
    };
    let backwards = flag == b'L' || flag == b'U';
    if floating {
        if backwards {
            *opposite = true;
        }
        ResizeStep {
            kind,
            adjust,
            opposite: *opposite,
        }
    } else {
        ResizeStep {
            kind,
            adjust: if backwards {
                adjust.wrapping_neg()
            } else {
                adjust
            },
            opposite: true,
        }
    }
}

fn target_pane(server: &mut Server, item: QueueItemId) -> Result<(PaneId, WindowId), CmdReturn> {
    let target = item_target(server, item);
    let Some(w) = target
        .wl
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.window)
    else {
        return Err(fail(server, item, b"no current window"));
    };
    let Some(wp) = target.wp else {
        return Err(fail(server, item, b"no current pane"));
    };
    Ok((wp, w))
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let (wp, w) = match target_pane(server, item) {
        Ok(found) => found,
        Err(ret) => return ret,
    };

    if args.has(b'T') != 0 {
        // cmd-resize-pane.c:71-81
        let Some(p) = server.panes.get_mut(wp) else {
            return fail(server, item, b"no current pane");
        };
        if !p.modes.is_empty() {
            return CmdReturn::Normal;
        }
        let mut adjust = i64::from(p.base.grid.sy()) - 1 - i64::from(p.base.cy);
        if adjust > i64::from(p.base.grid.hsize()) {
            adjust = i64::from(p.base.grid.hsize());
        }
        p.base.grid.remove_history(adjust as u32);
        p.base.cy = p.base.cy.wrapping_add(adjust as u32);
        p.flags.insert(PaneFlags::REDRAW);
        return CmdReturn::Normal;
    }

    if args.has(b'M') != 0 {
        return mouse_update(server, item);
    }

    if args.has(b'Z') != 0 {
        // cmd-resize-pane.c:86-93
        let zoomed = server
            .windows
            .get(w)
            .is_some_and(|w| w.flags.contains(WindowFlags::ZOOMED));
        if zoomed {
            let _ = window_unzoom(server, w, true);
        } else {
            let _ = window_zoom(server, w, wp);
        }
        server_redraw_window(server, w);
        return CmdReturn::Normal;
    }
    // cmd-resize-pane.c:94-96: the unzoom can give a zoomed float its floating
    // cell back, so `window_pane_is_floating` is re-evaluated afterwards.
    if !pane_is_floating(server, wp) {
        let _ = server_unzoom_window(server, w);
    }
    let floating = pane_is_floating(server, wp);

    let (wsx, wsy) = server
        .windows
        .get(w)
        .map(|w| (w.sx, w.sy))
        .unwrap_or_default();
    if args.has(b'x') != 0 {
        // cmd-resize-pane.c:98-114
        let x = match args.percentage(b'x', 0, i64::from(PANE_MAXIMUM), i64::from(wsx)) {
            Ok(x) => x,
            Err(cause) => return fail(server, item, concat(&[b"width ", &cause])),
        };
        if floating {
            if let Err(e) =
                layout::resize_floating_pane_to(server, wp, LayoutType::Leftright, x as u32)
            {
                return fail(server, item, concat(&[b"size ", &e.cause]));
            }
        } else {
            layout::resize_pane_to(server, wp, LayoutType::Leftright, x as u32);
        }
    }
    if args.has(b'y') != 0 {
        // cmd-resize-pane.c:115-142
        let mut y = match args.percentage(b'y', 0, i64::from(PANE_MAXIMUM), i64::from(wsy)) {
            Ok(y) => y,
            Err(cause) => return fail(server, item, concat(&[b"height ", &cause])),
        };
        let (yoff, psy) = server
            .panes
            .get(wp)
            .map(|p| (p.yoff, p.sy))
            .unwrap_or_default();
        let int_max = i64::from(i32::MAX);
        let status = window_get_pane_status(server, w);
        let on_border = (status == PANE_STATUS_TOP && yoff == 1)
            || (status == PANE_STATUS_BOTTOM
                && i64::from(yoff) + i64::from(psy) == i64::from(wsy) - 1);
        if y != int_max && on_border {
            y += 1;
        }
        if floating {
            if let Err(e) =
                layout::resize_floating_pane_to(server, wp, LayoutType::Topbottom, y as u32)
            {
                return fail(server, item, concat(&[b"size ", &e.cause]));
            }
        } else {
            layout::resize_pane_to(server, wp, LayoutType::Topbottom, y as u32);
        }
    }

    // cmd-resize-pane.c:144-182
    let mut opposite = false;
    for flag in *b"UDLR" {
        if args.has(flag) == 0 {
            continue;
        }
        let argval: &[u8] = match args.get(flag) {
            Some(value) => value,
            None if args.count() == 0 => b"1",
            None => args.string(0).unwrap_or(b""),
        };
        let adjust =
            match rmux_util::strtonum::strtonum(argval, i64::from(i32::MIN), i64::from(i32::MAX)) {
                Ok(n) => n as i32,
                Err(e) => {
                    return fail(
                        server,
                        item,
                        concat(&[b"adjustment ", e.to_string().as_bytes()]),
                    );
                }
            };
        let step = resize_step(flag, adjust, floating, &mut opposite);
        if floating {
            if let Err(e) =
                layout::resize_floating_pane(server, wp, step.kind, step.adjust, step.opposite)
            {
                return fail(server, item, concat(&[b"adjustment ", &e.cause]));
            }
        } else {
            layout::resize_pane(server, wp, step.kind, step.adjust, step.opposite);
        }
    }

    // cmd-resize-pane.c:184-188
    let has_parent = server
        .panes
        .get(wp)
        .and_then(|p| p.layout_cell)
        .and_then(|lc| server.layout_cells.get(lc))
        .is_some_and(|lc| lc.parent.is_some());
    if has_parent {
        layout::fix_offsets(server, w);
    }
    layout::fix_panes(server, w, None);
    events::fire_window(server, b"window-layout-changed", w);
    server_redraw_window(server, w);
    CmdReturn::Normal
}

/// cmd-resize-pane.c:193-222
fn mouse_update(server: &mut Server, item: QueueItemId) -> CmdReturn {
    let target = item_target(server, item);
    let m = item_event(server, item).mouse;
    let Some(w) = target
        .wl
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.window)
    else {
        return fail(server, item, b"no current window");
    };
    if !m.valid {
        return CmdReturn::Normal;
    }
    let Some((s, _, wp)) = find::mouse_pane(server, &m) else {
        return CmdReturn::Normal;
    };
    let Some(c) = item_client(server, item) else {
        return CmdReturn::Normal;
    };
    if server.clients.get(c).and_then(|c| c.session) != Some(s) {
        return CmdReturn::Normal;
    }

    if !pane_is_floating(server, wp) {
        set_drag(server, c, Some(MouseDragAction::ResizeTiled));
        resize_tiled(server, c, &m);
        return CmdReturn::Normal;
    }

    let _ = window_redraw_active_switch(server, w, Some(wp));
    let _ = window_set_active_pane(server, w, wp, true);

    set_drag(server, c, Some(MouseDragAction::ResizeMoveFloating));
    resize_move_floating(server, c, &m);
    CmdReturn::Normal
}

/// `c->tty.mouse_drag_update = ...` (`cmd-resize-pane.c:211, 219, 245, 373`).
fn set_drag(server: &mut Server, client: ClientId, action: Option<MouseDragAction>) {
    if let Some(c) = server.clients.get_mut(client) {
        c.drag.update = action;
    }
}

/// The `struct mouse_event` fields the drag handlers read, from the G15 event.
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
        b: ev.event.b,
        lb: ev.event.lb,
        sgr_type: ev.event.sgr_type,
        sgr_b: ev.event.sgr_b,
        offset_x: ev.target.ox,
        offset_y: ev.target.oy,
        status_at: ev.target.status_at,
        status_lines: ev.target.status_lines,
    }
}

/// `cmd_resize_pane_mouse_resize_tiled` for `MouseDragAction::ResizeTiled`.
pub fn drag_update_tiled(server: &mut Server, client: ClientId, ev: &ResolvedMouseEvent) {
    resize_tiled(server, client, &mouse_input(ev));
}

/// `cmd_resize_pane_mouse_resize_move_floating` for `MouseDragAction::ResizeMoveFloating`.
pub fn drag_update_floating(server: &mut Server, client: ClientId, ev: &ResolvedMouseEvent) {
    resize_move_floating(server, client, &mouse_input(ev));
}

/// cmd-resize-pane.c sets no `mouse_drag_release`; the drag ends by clearing
/// both actions (`server-client.c` drag end).
pub fn drag_release_tiled(server: &mut Server, client: ClientId, _ev: &ResolvedMouseEvent) {
    clear_drag(server, client);
}

/// See `drag_release_tiled`.
pub fn drag_release_floating(server: &mut Server, client: ClientId, _ev: &ResolvedMouseEvent) {
    clear_drag(server, client);
}

fn clear_drag(server: &mut Server, client: ClientId) {
    if let Some(c) = server.clients.get_mut(client) {
        c.drag.update = None;
        c.drag.release = None;
    }
}

/// Status-line correction of `cmd-resize-pane.c:265-273, 379-387`.
pub fn status_adjust(y: i32, status_at: i32, status_lines: u32) -> i32 {
    if status_at == 0 && y >= status_lines as i32 {
        y.wrapping_sub(status_lines as i32)
    } else if status_at > 0 && y >= status_at {
        status_at - 1
    } else {
        y
    }
}

/// `m->y + m->oy` etc. (`cmd-resize-pane.c:264, 269, 378, 383`) with the
/// status correction applied: `(x, y, lx, ly)`.
fn positions(m: &MouseInput) -> (i32, i32, i32, i32) {
    let x = m.x.wrapping_add(m.offset_x) as i32;
    let y = status_adjust(
        m.y.wrapping_add(m.offset_y) as i32,
        m.status_at,
        m.status_lines,
    );
    let lx = m.last_x.wrapping_add(m.offset_x) as i32;
    let ly = status_adjust(
        m.last_y.wrapping_add(m.offset_y) as i32,
        m.status_at,
        m.status_lines,
    );
    (x, y, lx, ly)
}

/// `wp->xoff`, `wp->yoff`, `wp->sx`, `wp->sy` read at `cmd-resize-pane.c:250-253`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneBox {
    pub xoff: i32,
    pub yoff: i32,
    pub sx: i32,
    pub sy: i32,
}

/// Outcome of one floating drag step (`cmd-resize-pane.c:275-350`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatingDrag {
    /// `layout_set_size(lc, sx, sy, xoff, yoff)` with these values.
    Resize(LayoutGeometry),
    /// A border or corner matched but the new size is below `PANE_MINIMUM`.
    Reject,
    /// The last position is on no border of the pane.
    NoBorder,
}

/// cmd-resize-pane.c:275-350. `left`/`right` are the border columns after the
/// scrollbar reserve (lines 254-262); `x, y` the current and `lx, ly` the
/// last position after the status correction.
#[allow(clippy::too_many_arguments)]
pub fn floating_drag(
    pane: PaneBox,
    g: LayoutGeometry,
    left: i32,
    right: i32,
    x: i32,
    y: i32,
    lx: i32,
    ly: i32,
) -> FloatingDrag {
    let gsx = g.sx as i32;
    let gsy = g.sy as i32;
    let min = PANE_MINIMUM as i32;
    let on_left = lx == left || lx == left + 1;
    let on_right = lx == right + 1 || lx == right;
    let on_top = ly == pane.yoff - 1;
    let on_bottom = ly == pane.yoff + pane.sy;
    let resize = |sx: i32, sy: i32, xoff: i32, yoff: i32| {
        FloatingDrag::Resize(LayoutGeometry::new(sx as u32, sy as u32, xoff, yoff))
    };
    if on_left && on_top {
        // Top left corner.
        let new_sx = gsx.wrapping_add(lx - x).max(min);
        let new_sy = gsy.wrapping_add(ly - y).max(min);
        resize(new_sx, new_sy, x + 1, y + 1)
    } else if on_right && on_top {
        // Top right corner.
        let new_sx = (x - g.xoff).max(min);
        let new_sy = gsy.wrapping_add(ly - y).max(min);
        resize(new_sx, new_sy, g.xoff, y + 1)
    } else if on_left && on_bottom {
        // Bottom left corner.
        let new_sx = gsx.wrapping_add(lx - x).max(min);
        let new_sy = y - g.yoff;
        if new_sy < min {
            return FloatingDrag::Reject;
        }
        resize(new_sx, new_sy, x + 1, g.yoff)
    } else if on_right && on_bottom {
        // Bottom right corner.
        let new_sx = (x - g.xoff).max(min);
        let new_sy = (y - g.yoff).max(min);
        resize(new_sx, new_sy, g.xoff, g.yoff)
    } else if lx == right {
        // Right border.
        let new_sx = x - g.xoff;
        if new_sx < min {
            return FloatingDrag::Reject;
        }
        resize(new_sx, gsy, g.xoff, g.yoff)
    } else if lx == left {
        // Left border.
        let new_sx = gsx.wrapping_add(lx - x);
        if new_sx < min {
            return FloatingDrag::Reject;
        }
        resize(new_sx, gsy, x + 1, g.yoff)
    } else if on_bottom {
        // Bottom border.
        let new_sy = y - g.yoff;
        if new_sy < min {
            return FloatingDrag::Reject;
        }
        resize(gsx, new_sy, g.xoff, g.yoff)
    } else if on_top {
        // Top border (move instead of resize).
        resize(gsx, gsy, g.xoff.wrapping_add(x - lx), y + 1)
    } else {
        FloatingDrag::NoBorder
    }
}

/// cmd-resize-pane.c:231-356
fn resize_move_floating(server: &mut Server, client: ClientId, m: &MouseInput) {
    let Some((_, wl, wp)) = find::mouse_pane(server, m) else {
        set_drag(server, client, None);
        return;
    };
    let Some(w) = server.winlinks.get(wl).map(|wl| wl.window) else {
        return;
    };
    let Some(p) = server.panes.get(wp) else {
        return;
    };
    let Some(lc) = p.layout_cell else {
        return;
    };
    let pane = PaneBox {
        xoff: p.xoff,
        yoff: p.yoff,
        sx: p.sx as i32,
        sy: p.sy as i32,
    };
    let reserve = p.scrollbar_style.width.wrapping_add(p.scrollbar_style.pad);
    let mut left = pane.xoff - 1;
    let mut right = pane.xoff + pane.sx;
    if pane_scrollbar_reserve(server, wp) {
        match server.windows.get(w).map(|w| w.sb_pos) {
            Some(PaneScrollbarPosition::Left) => left = left.wrapping_sub(reserve),
            Some(PaneScrollbarPosition::Right) => right = right.wrapping_add(reserve),
            None => {}
        }
    }
    let (x, y, lx, ly) = positions(m);
    let Some(g) = server.layout_cells.get(lc).map(|lc| lc.g) else {
        return;
    };
    if let FloatingDrag::Resize(new) = floating_drag(pane, g, left, right, x, y, lx, ly) {
        layout::set_size(
            &mut server.layout_cells,
            lc,
            new.sx,
            new.sy,
            new.xoff,
            new.yoff,
        );
        layout::fix_panes(server, w, None);
        crate::model::window::window_redraw_floating_pane(
            server,
            wp,
            pane.xoff,
            pane.yoff,
            pane.sx as u32,
            pane.sy as u32,
        );
        server_redraw_window_borders(server, w);
    }
}

/// cmd-resize-pane.c:358-422
fn resize_tiled(server: &mut Server, client: ClientId, m: &MouseInput) {
    const OFFSETS: [(i32, i32); 5] = [(0, 0), (0, 1), (1, 0), (0, -1), (-1, 0)];
    let Some((_, Some(wl))) = find::mouse_window(server, m) else {
        set_drag(server, client, None);
        return;
    };
    let Some(w) = server.winlinks.get(wl).map(|wl| wl.window) else {
        return;
    };
    let Some(root) = server.windows.get(w).and_then(|w| w.layout_root) else {
        return;
    };
    let (x, y, lx, ly) = positions(m);

    let mut cells: Vec<LayoutCellId> = Vec::with_capacity(OFFSETS.len());
    for (dx, dy) in OFFSETS {
        let Some(lc) = layout::search_by_border(
            &server.layout_cells,
            root,
            (lx as u32).wrapping_add_signed(dx),
            (ly as u32).wrapping_add_signed(dy),
        ) else {
            continue;
        };
        if !cells.contains(&lc) {
            cells.push(lc);
        }
    }
    if cells.is_empty() {
        return;
    }

    let mut resizes = 0;
    for lc in cells {
        let Some(kind) = server
            .layout_cells
            .get(lc)
            .and_then(|c| c.parent)
            .and_then(|parent| server.layout_cells.get(parent))
            .map(|parent| parent.kind)
        else {
            continue;
        };
        if y != ly && kind == LayoutType::Topbottom {
            layout::resize_layout(server, w, lc, kind, y.wrapping_sub(ly), false);
            resizes += 1;
        } else if x != lx && kind == LayoutType::Leftright {
            layout::resize_layout(server, w, lc, kind, x.wrapping_sub(lx), false);
            resizes += 1;
        }
    }
    if resizes != 0 {
        server_redraw_window(server, w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20x10 floating pane at (10, 5); its cell geometry is the same.
    fn pane() -> (PaneBox, LayoutGeometry) {
        (
            PaneBox {
                xoff: 10,
                yoff: 5,
                sx: 20,
                sy: 10,
            },
            LayoutGeometry::new(20, 10, 10, 5),
        )
    }
    const LEFT: i32 = 9;
    const RIGHT: i32 = 30;

    fn drag(x: i32, y: i32, lx: i32, ly: i32) -> FloatingDrag {
        let (p, g) = pane();
        floating_drag(p, g, LEFT, RIGHT, x, y, lx, ly)
    }
    fn geometry(sx: u32, sy: u32, xoff: i32, yoff: i32) -> FloatingDrag {
        FloatingDrag::Resize(LayoutGeometry::new(sx, sy, xoff, yoff))
    }

    #[test]
    fn top_left_corner_resizes_and_moves() {
        // Drag the corner (9,4) to (7,2): grow by 2 in both axes.
        assert_eq!(drag(7, 2, LEFT, 4), geometry(22, 12, 8, 3));
        // The character next to the corner counts as the corner; the C adds
        // the literal `lx - x`, so the width grows by one more.
        assert_eq!(drag(7, 2, LEFT + 1, 4), geometry(23, 12, 8, 3));
        // Clamp (not reject) below PANE_MINIMUM.
        assert_eq!(drag(40, 30, LEFT, 4), geometry(1, 1, 41, 31));
    }

    #[test]
    fn top_right_corner_keeps_xoff() {
        assert_eq!(drag(35, 2, RIGHT, 4), geometry(25, 12, 10, 3));
        assert_eq!(drag(35, 2, RIGHT + 1, 4), geometry(25, 12, 10, 3));
        assert_eq!(drag(5, 30, RIGHT, 4), geometry(1, 1, 10, 31));
    }

    #[test]
    fn bottom_left_corner_clamps_width_but_rejects_height() {
        assert_eq!(drag(7, 18, LEFT, 15), geometry(22, 13, 8, 5));
        assert_eq!(drag(40, 18, LEFT + 1, 15), geometry(1, 13, 41, 5));
        assert_eq!(drag(7, 5, LEFT, 15), FloatingDrag::Reject);
    }

    #[test]
    fn bottom_right_corner_clamps_both() {
        assert_eq!(drag(35, 18, RIGHT, 15), geometry(25, 13, 10, 5));
        assert_eq!(drag(5, 5, RIGHT + 1, 15), geometry(1, 1, 10, 5));
    }

    #[test]
    fn edges_resize_one_axis_and_reject_below_minimum() {
        // Right border.
        assert_eq!(drag(35, 8, RIGHT, 8), geometry(25, 10, 10, 5));
        assert_eq!(drag(10, 8, RIGHT, 8), FloatingDrag::Reject);
        // Left border; the extra column (left + 1) is not a border.
        assert_eq!(drag(7, 8, LEFT, 8), geometry(22, 10, 8, 5));
        assert_eq!(drag(29, 8, LEFT, 8), FloatingDrag::Reject);
        assert_eq!(drag(7, 8, LEFT + 1, 8), FloatingDrag::NoBorder);
        // Bottom border.
        assert_eq!(drag(20, 18, 20, 15), geometry(20, 13, 10, 5));
        assert_eq!(drag(20, 5, 20, 15), FloatingDrag::Reject);
    }

    #[test]
    fn top_border_moves_without_resizing() {
        assert_eq!(drag(23, 7, 20, 4), geometry(20, 10, 13, 8));
        assert_eq!(drag(0, 0, 20, 4), geometry(20, 10, -10, 1));
    }

    #[test]
    fn no_border_matches_inside_and_outside() {
        assert_eq!(drag(20, 8, 20, 8), FloatingDrag::NoBorder);
        assert_eq!(drag(0, 0, 50, 50), FloatingDrag::NoBorder);
    }

    #[test]
    fn corner_tests_precede_edge_tests() {
        // (RIGHT, yoff + sy) is both the right edge and bottom edge: the corner
        // branch clamps instead of rejecting.
        assert_eq!(drag(5, 5, RIGHT, 15), geometry(1, 1, 10, 5));
    }

    #[test]
    fn status_line_correction() {
        assert_eq!(status_adjust(3, 0, 1), 2);
        assert_eq!(status_adjust(0, 0, 1), 0);
        assert_eq!(status_adjust(25, 23, 1), 22);
        assert_eq!(status_adjust(10, 23, 1), 10);
        assert_eq!(status_adjust(10, -1, 0), 10);
    }

    fn steps(flags: &[u8], floating: bool) -> Vec<ResizeStep> {
        let mut opposite = false;
        flags
            .iter()
            .map(|f| resize_step(*f, 3, floating, &mut opposite))
            .collect()
    }
    fn step(kind: LayoutType, adjust: i32, opposite: bool) -> ResizeStep {
        ResizeStep {
            kind,
            adjust,
            opposite,
        }
    }

    #[test]
    fn floating_opposite_is_sticky() {
        assert_eq!(
            steps(b"UD", true),
            vec![
                step(LayoutType::Topbottom, 3, true),
                step(LayoutType::Topbottom, 3, true)
            ]
        );
        assert_eq!(
            steps(b"LR", true),
            vec![
                step(LayoutType::Leftright, 3, true),
                step(LayoutType::Leftright, 3, true)
            ]
        );
        // The command loop order is U D L R: D before U is not sticky yet.
        assert_eq!(
            steps(b"DU", true),
            vec![
                step(LayoutType::Topbottom, 3, false),
                step(LayoutType::Topbottom, 3, true)
            ]
        );
        assert_eq!(
            steps(b"DR", true),
            vec![
                step(LayoutType::Topbottom, 3, false),
                step(LayoutType::Leftright, 3, false)
            ]
        );
    }

    #[test]
    fn tiled_steps_negate_up_and_left() {
        assert_eq!(
            steps(b"UDLR", false),
            vec![
                step(LayoutType::Topbottom, -3, true),
                step(LayoutType::Topbottom, 3, true),
                step(LayoutType::Leftright, -3, true),
                step(LayoutType::Leftright, 3, true),
            ]
        );
        let mut opposite = false;
        assert_eq!(
            resize_step(b'U', i32::MIN, false, &mut opposite).adjust,
            i32::MIN
        );
    }
}
