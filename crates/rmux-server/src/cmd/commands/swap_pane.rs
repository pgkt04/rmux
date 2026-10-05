// Ported from tmux cmd-swap-pane.c @ 8f25579c
use super::support::{fail, item_source, item_target};
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::ids::{PaneId, QueueItemId, WindowId};
use crate::layout::{self, LayoutHost};
use crate::model::PaneFlags;
use crate::model::pane::{pane_is_floating, pane_resize};
use crate::model::window::{
    window_fire_pane_moved, window_pop_zoom, window_push_zoom, window_set_active_pane,
};
use crate::server::Server;
use crate::server::events;
use crate::server::operations::server_redraw_window;
use rmux_emu::colour::Colour;

/// `TAILQ_REMOVE(dst, dst_wp)`, `TAILQ_REPLACE(src, src_wp, dst_wp)`, then
/// `TAILQ_INSERT_HEAD/AFTER(dst, tmp_wp, src_wp)` with `tmp_wp` the old
/// predecessor of `dst_wp` (`cmd-swap-pane.c:123-131`, and `:133-141` for the
/// z-order). `dst` is `None` when both panes are in the same window and `src`
/// is that window's list. Lists missing a pane are left unchanged.
pub fn swap_order(
    src: &mut Vec<PaneId>,
    mut dst: Option<&mut Vec<PaneId>>,
    src_wp: PaneId,
    dst_wp: PaneId,
) {
    fn position(list: &[PaneId], wp: PaneId) -> Option<usize> {
        list.iter().position(|p| *p == wp)
    }
    let dst_list: &Vec<PaneId> = match &dst {
        Some(d) => d,
        None => src,
    };
    let Some(j) = position(dst_list, dst_wp) else {
        return;
    };
    if !src.contains(&src_wp) {
        return;
    }
    let mut tmp = (j > 0).then(|| dst_list[j - 1]);
    match &mut dst {
        Some(d) => d.remove(j),
        None => src.remove(j),
    };
    let Some(i) = position(src, src_wp) else {
        return;
    };
    src[i] = dst_wp;
    if tmp == Some(src_wp) {
        tmp = Some(dst_wp);
    }
    let dst_list: &mut Vec<PaneId> = match &mut dst {
        Some(d) => d,
        None => src,
    };
    match tmp.and_then(|t| position(dst_list, t)) {
        None => dst_list.insert(0, src_wp),
        Some(k) => dst_list.insert(k + 1, src_wp),
    }
}

/// `cmd_swap_pane_next_tiled_pane` (`cmd-swap-pane.c:45-51`) from list
/// position `from` forward.
fn next_tiled(server: &Server, panes: &[PaneId], from: usize) -> Option<PaneId> {
    panes[from.min(panes.len())..]
        .iter()
        .copied()
        .find(|wp| is_tiled(server, *wp))
}

/// `cmd_swap_pane_prev_tiled_pane` (`cmd-swap-pane.c:53-59`) from list
/// position `from` backward (`from == None` starts at the tail).
fn prev_tiled(server: &Server, panes: &[PaneId], from: Option<usize>) -> Option<PaneId> {
    let end = from.map_or(panes.len(), |i| i.min(panes.len()));
    panes[..end]
        .iter()
        .rev()
        .copied()
        .find(|wp| is_tiled(server, *wp))
}

/// `layout_cell_is_tiled(wp->layout_cell)`; a pane without a cell is not tiled.
fn is_tiled(server: &Server, wp: PaneId) -> bool {
    server
        .panes
        .get(wp)
        .and_then(|p| p.layout_cell)
        .is_some_and(|lc| layout::cell_is_tiled(&server.layout_cells, lc))
}

/// `colour_palette_from_option(&wp->palette, wp->options)` (`colour.c:1293-1323`).
/// Shared with break-pane/join-pane (G20PaneMoves).
pub(crate) fn palette_from_option(server: &mut Server, wp: PaneId) {
    let Some(options) = server.panes.get(wp).map(|p| p.options) else {
        return;
    };
    let defaults = server
        .options
        .get(options, b"pane-colours")
        .and_then(|(_, entry)| {
            let mut defaults = [Colour::NONE; 256];
            let mut any = false;
            for (key, item) in entry.array_items() {
                any = true;
                if let crate::options::OptionsArrayKey::Index(index) = key
                    && let Some(slot) = defaults.get_mut(*index as usize)
                    && let Some(value) = item.value().as_number()
                {
                    *slot = Colour(value as i32);
                }
            }
            any.then_some(defaults)
        });
    if let Some(p) = server.panes.get_mut(wp) {
        p.palette.replace_defaults(defaults);
    }
}

fn pop_zoom(server: &mut Server, w: WindowId) {
    if matches!(window_pop_zoom(server, w, true), Ok(true)) {
        server_redraw_window(server, w);
    }
}

fn link_window(server: &Server, wl: Option<crate::ids::WinlinkId>) -> Option<(WindowId, i32)> {
    wl.and_then(|wl| server.winlinks.get(wl))
        .map(|wl| (wl.window, wl.index))
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let source = item_source(server, item);
    let target = item_target(server, item);

    let Some((dst_w, dst_idx)) = link_window(server, target.wl) else {
        return fail(server, item, b"no current window");
    };
    let Some(dst_wp) = target.wp else {
        return fail(server, item, b"no current pane");
    };
    let Some((mut src_w, src_idx)) = link_window(server, source.wl) else {
        return fail(server, item, b"no current window");
    };
    let Some(mut src_wp) = source.wp else {
        return fail(server, item, b"no current pane");
    };

    let modal = |server: &Server, w: WindowId| server.windows.get(w).and_then(|w| w.modal);
    if modal(server, src_w) == Some(src_wp) || modal(server, dst_w) == Some(dst_wp) {
        return fail(server, item, b"pane is modal");
    }

    let keep_zoom = args.has(b'Z') != 0;
    if matches!(window_push_zoom(server, dst_w, false, keep_zoom), Ok(true)) {
        server_redraw_window(server, dst_w);
    }

    if args.has(b'D') != 0 {
        if pane_is_floating(server, dst_wp) {
            queue::error(server, item, b"cannot swap down on floating pane");
            pop_zoom(server, dst_w);
            return CmdReturn::Error;
        }
        src_w = dst_w;
        let panes = server
            .windows
            .get(dst_w)
            .map(|w| w.panes.clone())
            .unwrap_or_default();
        let at = panes
            .iter()
            .position(|p| *p == dst_wp)
            .map_or(panes.len(), |i| i + 1);
        src_wp = next_tiled(server, &panes, at)
            .or_else(|| next_tiled(server, &panes, 0))
            .unwrap_or(dst_wp);
    } else if args.has(b'U') != 0 {
        if pane_is_floating(server, dst_wp) {
            queue::error(server, item, b"cannot swap up on floating pane");
            pop_zoom(server, dst_w);
            return CmdReturn::Error;
        }
        src_w = dst_w;
        let panes = server
            .windows
            .get(dst_w)
            .map(|w| w.panes.clone())
            .unwrap_or_default();
        let at = panes.iter().position(|p| *p == dst_wp).unwrap_or(0);
        src_wp = prev_tiled(server, &panes, Some(at))
            .or_else(|| prev_tiled(server, &panes, None))
            .unwrap_or(dst_wp);
    }

    if src_w != dst_w && matches!(window_push_zoom(server, src_w, false, keep_zoom), Ok(true)) {
        server_redraw_window(server, src_w);
    }

    if src_wp != dst_wp {
        swap(
            server,
            args.has(b'd') != 0,
            src_w,
            src_wp,
            src_idx,
            dst_w,
            dst_wp,
            dst_idx,
        );
    }

    // out: cmd-swap-pane.c:199-204
    pop_zoom(server, src_w);
    if src_w != dst_w {
        pop_zoom(server, dst_w);
    }
    CmdReturn::Normal
}

/// cmd-swap-pane.c:120-197
#[allow(clippy::too_many_arguments)]
fn swap(
    server: &mut Server,
    detached: bool,
    src_w: WindowId,
    src_wp: PaneId,
    src_idx: i32,
    dst_w: WindowId,
    dst_wp: PaneId,
    dst_idx: i32,
) {
    crate::client::mouse::remove_pane(server, src_wp);
    crate::client::mouse::remove_pane(server, dst_wp);

    if src_w == dst_w {
        if let Some(w) = server.windows.get_mut(dst_w) {
            swap_order(&mut w.panes, None, src_wp, dst_wp);
            swap_order(&mut w.z_order, None, src_wp, dst_wp);
        }
    } else {
        let mut src_panes = server
            .windows
            .get_mut(src_w)
            .map(|w| std::mem::take(&mut w.panes))
            .unwrap_or_default();
        let mut src_z = server
            .windows
            .get_mut(src_w)
            .map(|w| std::mem::take(&mut w.z_order))
            .unwrap_or_default();
        let mut dst_panes = server
            .windows
            .get_mut(dst_w)
            .map(|w| std::mem::take(&mut w.panes))
            .unwrap_or_default();
        let mut dst_z = server
            .windows
            .get_mut(dst_w)
            .map(|w| std::mem::take(&mut w.z_order))
            .unwrap_or_default();
        swap_order(&mut src_panes, Some(&mut dst_panes), src_wp, dst_wp);
        swap_order(&mut src_z, Some(&mut dst_z), src_wp, dst_wp);
        if let Some(w) = server.windows.get_mut(src_w) {
            w.panes = src_panes;
            w.z_order = src_z;
        }
        if let Some(w) = server.windows.get_mut(dst_w) {
            w.panes = dst_panes;
            w.z_order = dst_z;
        }
    }

    // cmd-swap-pane.c:143-148
    let src_lc = server.panes.get(src_wp).and_then(|p| p.layout_cell);
    let dst_lc = server.panes.get(dst_wp).and_then(|p| p.layout_cell);
    if let Some(lc) = src_lc.and_then(|lc| server.layout_cells.get_mut(lc)) {
        lc.pane = Some(dst_wp);
    }
    if let Some(lc) = dst_lc.and_then(|lc| server.layout_cells.get_mut(lc)) {
        lc.pane = Some(src_wp);
    }

    // cmd-swap-pane.c:150-162
    let src_options = server.windows.get(src_w).map(|w| w.options);
    let dst_options = server.windows.get(dst_w).map(|w| w.options);
    let (ssx, ssy, sxoff, syoff) = geometry(server, src_wp);
    let (dsx, dsy, dxoff, dyoff) = geometry(server, dst_wp);
    if let Some(p) = server.panes.get_mut(src_wp) {
        p.layout_cell = dst_lc;
        p.window = dst_w;
        p.flags
            .insert(PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
        p.xoff = dxoff;
        p.yoff = dyoff;
        let options = p.options;
        server.options.set_parent(options, dst_options);
    }
    if let Some(p) = server.panes.get_mut(dst_wp) {
        p.layout_cell = src_lc;
        p.window = src_w;
        p.flags
            .insert(PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
        p.xoff = sxoff;
        p.yoff = syoff;
        let options = p.options;
        server.options.set_parent(options, src_options);
    }
    let _ = pane_resize(server, src_wp, dsx, dsy);
    let _ = pane_resize(server, dst_wp, ssx, ssy);

    // cmd-swap-pane.c:164-177
    let active = |server: &Server, w: WindowId| server.windows.get(w).and_then(|w| w.active);
    if !detached {
        if src_w != dst_w {
            let _ = window_set_active_pane(server, src_w, dst_wp, true);
            let _ = window_set_active_pane(server, dst_w, src_wp, true);
        } else {
            let _ = window_set_active_pane(server, src_w, dst_wp, true);
        }
    } else {
        if active(server, src_w) == Some(src_wp) {
            let _ = window_set_active_pane(server, src_w, dst_wp, true);
        }
        if active(server, dst_w) == Some(dst_wp) {
            let _ = window_set_active_pane(server, dst_w, src_wp, true);
        }
    }

    // cmd-swap-pane.c:178-189
    if src_w != dst_w {
        LayoutHost::window_last_panes_remove(server, src_w, src_wp);
        LayoutHost::window_last_panes_remove(server, dst_w, dst_wp);
        palette_from_option(server, src_wp);
        palette_from_option(server, dst_wp);
        layout::fix_panes(server, src_w, None);
        LayoutHost::invalidate_scene(server, src_w);
        server_redraw_window(server, src_w);
    }
    layout::fix_panes(server, dst_w, None);
    LayoutHost::invalidate_scene(server, dst_w);
    server_redraw_window(server, dst_w);

    // cmd-swap-pane.c:191-197
    if src_w != dst_w {
        window_fire_pane_moved(server, src_wp, src_w, src_idx, dst_w, dst_idx);
        window_fire_pane_moved(server, dst_wp, dst_w, dst_idx, src_w, src_idx);
    }
    events::fire_window(server, b"window-layout-changed", src_w);
    if src_w != dst_w {
        events::fire_window(server, b"window-layout-changed", dst_w);
    }
}

/// `wp->sx, sy, xoff, yoff`.
fn geometry(server: &Server, wp: PaneId) -> (u32, u32, i32, i32) {
    server
        .panes
        .get(wp)
        .map(|p| (p.sx, p.sy, p.xoff, p.yoff))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::swap_order;
    use crate::ids::{ArenaId, PaneId};

    fn id(n: u32) -> PaneId {
        PaneId::from_parts(n, 0)
    }
    fn ids(ns: &[u32]) -> Vec<PaneId> {
        ns.iter().map(|n| id(*n)).collect()
    }

    #[test]
    fn same_window_adjacent_with_src_before_dst() {
        // tmp_wp = PREV(dst) = src, so after the replace tmp_wp becomes dst
        // and src is inserted right after it.
        let mut list = ids(&[0, 1, 2, 3]);
        swap_order(&mut list, None, id(1), id(2));
        assert_eq!(list, ids(&[0, 2, 1, 3]));
    }

    #[test]
    fn same_window_dst_at_head() {
        // tmp_wp = NULL: src goes to the head.
        let mut list = ids(&[0, 1, 2, 3]);
        swap_order(&mut list, None, id(2), id(0));
        assert_eq!(list, ids(&[2, 1, 0, 3]));
    }

    #[test]
    fn same_window_distant_panes_both_directions() {
        let mut list = ids(&[0, 1, 2, 3, 4]);
        swap_order(&mut list, None, id(1), id(3));
        assert_eq!(list, ids(&[0, 3, 2, 1, 4]));
        let mut list = ids(&[0, 1, 2, 3, 4]);
        swap_order(&mut list, None, id(3), id(1));
        assert_eq!(list, ids(&[0, 3, 2, 1, 4]));
        // src directly after dst: tmp_wp = PREV(dst) = 0.
        let mut list = ids(&[0, 1, 2, 3]);
        swap_order(&mut list, None, id(2), id(1));
        assert_eq!(list, ids(&[0, 2, 1, 3]));
    }

    #[test]
    fn cross_window_dst_at_head_inserts_src_at_head() {
        let mut src = ids(&[0, 1, 2]);
        let mut dst = ids(&[10, 11, 12]);
        swap_order(&mut src, Some(&mut dst), id(1), id(10));
        assert_eq!(src, ids(&[0, 10, 2]));
        assert_eq!(dst, ids(&[1, 11, 12]));
    }

    #[test]
    fn cross_window_dst_in_middle_and_tail() {
        let mut src = ids(&[0, 1, 2]);
        let mut dst = ids(&[10, 11, 12]);
        swap_order(&mut src, Some(&mut dst), id(0), id(11));
        assert_eq!(src, ids(&[11, 1, 2]));
        assert_eq!(dst, ids(&[10, 0, 12]));

        let mut src = ids(&[0]);
        let mut dst = ids(&[10, 11, 12]);
        swap_order(&mut src, Some(&mut dst), id(0), id(12));
        assert_eq!(src, ids(&[12]));
        assert_eq!(dst, ids(&[10, 11, 0]));
    }

    #[test]
    fn z_order_lists_differ_from_pane_lists() {
        // The z-order has its own positions; the same dance applies to it.
        let mut src_z = ids(&[2, 0, 1]);
        let mut dst_z = ids(&[12, 10, 11]);
        swap_order(&mut src_z, Some(&mut dst_z), id(1), id(12));
        assert_eq!(src_z, ids(&[2, 0, 12]));
        assert_eq!(dst_z, ids(&[1, 10, 11]));
    }

    #[test]
    fn missing_pane_leaves_lists_unchanged() {
        let mut src = ids(&[0, 1]);
        let mut dst = ids(&[10]);
        swap_order(&mut src, Some(&mut dst), id(5), id(10));
        assert_eq!(src, ids(&[0, 1]));
        assert_eq!(dst, ids(&[10]));
    }
}
