// Ported from tmux layout.c @ 8f25579c
/*
 * Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2016 Stephen Kent <smkent@smkent.net>
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

//! The window layout is a tree of cells: a left-right node, a top-bottom node,
//! or a leaf that holds one pane. A leaf is *tiled* when it is drawn as part
//! of the tiled layout; a *neighbour* is a sibling that is tiled or contains a
//! tiled leaf. Cells live in the server arena and are addressed by
//! `LayoutCellId`; a pane keeps a handle to its cell and a cell to its parent.

use rmux_emu::screen::PaneLines;
use rmux_util::bytes::ByteString;
use rmux_util::log_debug;

use super::{
    Cells, LayoutCell, LayoutCellFlags, LayoutCells, LayoutEnv, LayoutError, LayoutGeometry,
    LayoutHost, LayoutType, PANE_MAXIMUM, PANE_MINIMUM, SplitSizes,
};
use crate::cmd::arguments::{Args, ArgumentFormatRuntime};
use crate::ids::{LayoutCellId, PaneId, QueueItemId, WindowId};
use crate::model::PaneFlags;
use crate::model::spawn::SpawnFlags;
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use crate::ui::status::PaneStatusPosition;

pub(super) fn cell(cells: &Cells, lc: LayoutCellId) -> &LayoutCell {
    cells.get(lc).expect("stale layout cell")
}

pub(super) fn cell_mut(cells: &mut Cells, lc: LayoutCellId) -> &mut LayoutCell {
    cells.get_mut(lc).expect("stale layout cell")
}

fn children_len(cells: &Cells, lc: LayoutCellId) -> usize {
    cell(cells, lc).children.len()
}

fn child_at(cells: &Cells, lc: LayoutCellId, i: usize) -> LayoutCellId {
    cell(cells, lc).children[i]
}

fn position(cells: &Cells, parent: LayoutCellId, lc: LayoutCellId) -> usize {
    cell(cells, parent)
        .children
        .iter()
        .position(|&c| c == lc)
        .expect("layout cell missing from its parent")
}

/// `TAILQ_INSERT_*`: link `lc` under `parent` at `index`.
pub(super) fn link_child(cells: &mut Cells, parent: LayoutCellId, index: usize, lc: LayoutCellId) {
    cell_mut(cells, parent).children.insert(index, lc);
    cell_mut(cells, lc).parent = Some(parent);
}

/// `TAILQ_INSERT_TAIL` plus the parent link.
pub(super) fn link_child_tail(cells: &mut Cells, parent: LayoutCellId, lc: LayoutCellId) {
    let n = children_len(cells, parent);
    link_child(cells, parent, n, lc);
}

/// `TAILQ_REMOVE` plus clearing the parent link.
pub(super) fn unlink_child(cells: &mut Cells, parent: LayoutCellId, lc: LayoutCellId) {
    let i = position(cells, parent, lc);
    cell_mut(cells, parent).children.remove(i);
    cell_mut(cells, lc).parent = None;
}

fn tiled_or_has_tiled(cells: &Cells, lc: LayoutCellId) -> bool {
    cell_is_tiled(cells, lc) || cell_has_tiled_child(cells, lc)
}

/// `layout_create_cell` (`layout.c:69-84`).
pub fn create_cell(cells: &mut Cells, parent: Option<LayoutCellId>) -> LayoutCellId {
    cells
        .insert(LayoutCell {
            kind: LayoutType::Windowpane,
            flags: LayoutCellFlags::default(),
            parent,
            g: LayoutGeometry::UNSET,
            fg: LayoutGeometry::UNSET,
            pane: None,
            children: Vec::new(),
        })
        .expect("layout cell arena exhausted")
}

fn drop_cell(cells: &mut Cells, lc: LayoutCellId) {
    cells.request_remove(lc).expect("layout cell freed twice");
}

/// `layout_free_cell` (`layout.c:86-118`). With `only_nodes`, leaves stay
/// alive with their pane links; their stale parent handles are cleared so the
/// caller can relink them under a new root.
pub fn free_cell<C: LayoutCells>(store: &mut C, lc: Option<LayoutCellId>, only_nodes: bool) {
    let Some(lc) = lc else {
        return;
    };
    let kind = cell(store.cells(), lc).kind;
    if only_nodes && kind == LayoutType::Windowpane {
        return;
    }
    match kind {
        LayoutType::Leftright | LayoutType::Topbottom => {
            let children = std::mem::take(&mut cell_mut(store.cells_mut(), lc).children);
            for child in children {
                if !only_nodes || cell(store.cells(), child).kind != LayoutType::Windowpane {
                    cell_mut(store.cells_mut(), child).parent = None;
                    free_cell(store, Some(child), only_nodes);
                } else {
                    cell_mut(store.cells_mut(), child).parent = None;
                }
            }
        }
        LayoutType::Windowpane => {
            if let Some(wp) = cell(store.cells(), lc).pane {
                if let Some(current) = store.pane_layout_cell(wp) {
                    cell_mut(store.cells_mut(), current).parent = None;
                    store.set_pane_layout_cell(wp, None);
                }
            }
        }
    }
    drop_cell(store.cells_mut(), lc);
}

/// `layout_print_cell` (`layout.c:120-154`); handles print as arena slots.
pub fn debug_print(cells: &Cells, lc: Option<LayoutCellId>, hdr: &str, n: u32) {
    if !rmux_util::log::enabled() {
        return;
    }
    let Some(lc) = lc else {
        return;
    };
    let c = cell(cells, lc);
    let kind = match c.kind {
        LayoutType::Leftright => "LEFTRIGHT",
        LayoutType::Topbottom => "TOPBOTTOM",
        LayoutType::Windowpane => "WINDOWPANE",
    };
    log_debug!(
        "{}:{:>width$}{:?} type {} [parent {:?}] wp={:?} [{},{} {}x{}]",
        hdr,
        " ",
        lc,
        kind,
        c.parent,
        c.pane,
        c.g.xoff,
        c.g.yoff,
        c.g.sx,
        c.g.sy,
        width = n as usize
    );
    for i in 0..c.children.len() {
        debug_print(cells, Some(c.children[i]), hdr, n + 1);
    }
}

/// `layout_search_by_border` (`layout.c:156-194`).
pub fn search_by_border(cells: &Cells, lc: LayoutCellId, x: u32, y: u32) -> Option<LayoutCellId> {
    let x = x as i32;
    let y = y as i32;
    let parent = cell(cells, lc);
    let mut last: Option<&LayoutCell> = None;
    let mut last_id = None;
    for &child_id in &parent.children {
        let child = cell(cells, child_id);
        let g = child.g;
        if x >= g.xoff
            && x < g.xoff.wrapping_add(g.sx as i32)
            && y >= g.yoff
            && y < g.yoff.wrapping_add(g.sy as i32)
        {
            // Inside the cell - recurse.
            return search_by_border(cells, child_id, x as u32, y as u32);
        }
        let Some(prev) = last else {
            last = Some(child);
            last_id = Some(child_id);
            continue;
        };
        match parent.kind {
            LayoutType::Leftright => {
                if x < g.xoff && x >= prev.g.xoff.wrapping_add(prev.g.sx as i32) {
                    return last_id;
                }
            }
            LayoutType::Topbottom => {
                if y < g.yoff && y >= prev.g.yoff.wrapping_add(prev.g.sy as i32) {
                    return last_id;
                }
            }
            LayoutType::Windowpane => {}
        }
        last = Some(child);
        last_id = Some(child_id);
    }
    None
}

/// `layout_set_size` (`layout.c:196-205`).
pub fn set_size(cells: &mut Cells, lc: LayoutCellId, sx: u32, sy: u32, xoff: i32, yoff: i32) {
    cell_mut(cells, lc).g = LayoutGeometry { sx, sy, xoff, yoff };
}

/// `layout_make_leaf` (`layout.c:207-219`).
pub fn make_leaf<H: LayoutHost>(host: &mut H, lc: LayoutCellId, wp: PaneId) {
    let c = cell_mut(host.cells_mut(), lc);
    c.kind = LayoutType::Windowpane;
    c.children.clear();
    c.pane = Some(wp);
    host.set_pane_layout_cell(wp, Some(lc));
}

/// `layout_make_node` (`layout.c:221-233`).
pub fn make_node<C: LayoutCells>(store: &mut C, lc: LayoutCellId, kind: LayoutType) {
    if kind == LayoutType::Windowpane {
        panic!("bad layout type");
    }
    let c = cell_mut(store.cells_mut(), lc);
    c.kind = kind;
    c.children.clear();
    if let Some(wp) = c.pane.take() {
        store.set_pane_layout_cell(wp, None);
    }
}

/// `layout_cell_is_tiled` (`layout.c:236-243`).
pub fn cell_is_tiled(cells: &Cells, lc: LayoutCellId) -> bool {
    let c = cell(cells, lc);
    c.is_leaf() && !c.is_floating()
}

/// `layout_cell_has_tiled_child` (`layout.c:245-259`).
pub fn cell_has_tiled_child(cells: &Cells, lc: LayoutCellId) -> bool {
    let c = cell(cells, lc);
    if c.is_leaf() {
        return false;
    }
    c.children
        .iter()
        .any(|&child| cell_is_tiled(cells, child) || cell_has_tiled_child(cells, child))
}

/// `layout_cell_is_first_tiled` (`layout.c:261-276`).
fn cell_is_first_tiled(cells: &Cells, lc: LayoutCellId) -> bool {
    let Some(parent) = cell(cells, lc).parent else {
        return cell_is_tiled(cells, lc);
    };
    let first = cell(cells, parent)
        .children
        .iter()
        .copied()
        .find(|&child| tiled_or_has_tiled(cells, child));
    first == Some(lc)
}

/// `layout_cell_get_first_tiled` (`layout.c:278-298`).
pub(super) fn cell_get_first_tiled(cells: &Cells, lc: LayoutCellId) -> Option<LayoutCellId> {
    if cell_is_tiled(cells, lc) {
        return Some(lc);
    }
    let c = cell(cells, lc);
    if c.is_leaf() {
        return None;
    }
    for &child in &c.children {
        if cell_is_tiled(cells, child) {
            return Some(child);
        }
        if !cell(cells, child).is_leaf() {
            if let Some(found) = cell_get_first_tiled(cells, child) {
                return Some(found);
            }
        }
    }
    None
}

/// `layout_fix_offsets1` (`layout.c:300-331`).
fn fix_offsets1(cells: &mut Cells, lc: LayoutCellId) {
    let parent = cell(cells, lc);
    let kind = parent.kind;
    let pg = parent.g;
    let n = parent.children.len();
    let mut xoff = pg.xoff;
    let mut yoff = pg.yoff;
    for i in 0..n {
        let child = child_at(cells, lc, i);
        if !tiled_or_has_tiled(cells, child) {
            continue;
        }
        let c = cell_mut(cells, child);
        if kind == LayoutType::Leftright {
            c.g.xoff = xoff;
            c.g.yoff = pg.yoff;
        } else {
            c.g.xoff = pg.xoff;
            c.g.yoff = yoff;
        }
        let (is_leaf, g) = (c.is_leaf(), c.g);
        if !is_leaf {
            fix_offsets1(cells, child);
        }
        if kind == LayoutType::Leftright {
            xoff = xoff.wrapping_add(g.sx as i32).wrapping_add(1);
        } else {
            yoff = yoff.wrapping_add(g.sy as i32).wrapping_add(1);
        }
    }
}

/// `layout_fix_offsets` on an explicit root (`layout.c:333-346`).
pub fn fix_offsets_root(cells: &mut Cells, root: LayoutCellId) {
    // Root consists of a single floating cell.
    if cell(cells, root).is_floating() {
        return;
    }
    let c = cell_mut(cells, root);
    c.g.xoff = 0;
    c.g.yoff = 0;
    fix_offsets1(cells, root);
}

/// `layout_fix_offsets` (`layout.c:333-346`).
pub fn fix_offsets<H: LayoutHost>(host: &mut H, w: WindowId) {
    let root = host
        .window_layout_root(w)
        .expect("layout_fix_offsets without a layout root");
    fix_offsets_root(host.cells_mut(), root);
}

/// `layout_cell_is_last_tiled` (`layout.c:348-363`).
fn cell_is_last_tiled(cells: &Cells, lc: LayoutCellId) -> bool {
    let Some(parent) = cell(cells, lc).parent else {
        return cell_is_tiled(cells, lc);
    };
    let last = cell(cells, parent)
        .children
        .iter()
        .rev()
        .copied()
        .find(|&child| tiled_or_has_tiled(cells, child));
    last == Some(lc)
}

/// `layout_cell_is_top` (`layout.c:365-381`).
fn cell_is_top(cells: &Cells, root: Option<LayoutCellId>, mut lc: LayoutCellId) -> bool {
    while Some(lc) != root {
        let Some(next) = cell(cells, lc).parent else {
            return false;
        };
        if cell(cells, next).kind == LayoutType::Topbottom && !cell_is_first_tiled(cells, lc) {
            return false;
        }
        lc = next;
    }
    true
}

/// `layout_cell_is_bottom` (`layout.c:383-399`).
fn cell_is_bottom(cells: &Cells, root: Option<LayoutCellId>, mut lc: LayoutCellId) -> bool {
    while Some(lc) != root {
        let Some(next) = cell(cells, lc).parent else {
            return false;
        };
        if cell(cells, next).kind == LayoutType::Topbottom && !cell_is_last_tiled(cells, lc) {
            return false;
        }
        lc = next;
    }
    true
}

/// `layout_add_horizontal_border` (`layout.c:401-416`): true when the pane
/// status line needs an extra row, for the topmost or bottommost cells only.
pub fn add_horizontal_border(
    cells: &Cells,
    root: Option<LayoutCellId>,
    lc: LayoutCellId,
    status: PaneStatusPosition,
) -> bool {
    match status {
        PaneStatusPosition::Top => cell_is_top(cells, root, lc),
        PaneStatusPosition::Bottom => cell_is_bottom(cells, root, lc),
        _ => false,
    }
}

/// `layout_fix_panes` (`layout.c:418-487`).
pub fn fix_panes<H: LayoutHost>(host: &mut H, w: WindowId, skip: Option<PaneId>) {
    let root = host.window_layout_root(w);
    let sb_pos = host.window_scrollbar_position(w);
    let mut changed = false;
    let n = host.window_panes(w).len();
    for i in 0..n {
        let wp = host.window_panes(w)[i];
        let Some(lc) = host.pane_layout_cell(wp) else {
            continue;
        };
        if Some(wp) == skip {
            continue;
        }
        let (old_xoff, old_yoff, old_sx, old_sy) = host.pane_geometry(wp);

        let g = cell(host.cells(), lc).g;
        let mut xoff = g.xoff;
        let mut yoff = g.yoff;
        let mut sx = g.sx;
        let mut sy = g.sy;

        let status = host.pane_status(wp);
        if !host.pane_is_floating(wp) && add_horizontal_border(host.cells(), root, lc, status) {
            if status == PaneStatusPosition::Top {
                yoff = yoff.wrapping_add(1);
            }
            if sy > 1 {
                sy -= 1;
            }
        }

        let reserve = host.pane_scrollbar_reserve(wp);
        if reserve {
            let (mut sb_w, mut sb_pad) = host.pane_scrollbar_style(wp);
            if sb_w < 1 {
                sb_w = 1;
            }
            if sb_pad < 0 {
                sb_pad = 0;
            }
            let room = (sx as i32).wrapping_sub(sb_w).wrapping_sub(sb_pad);
            if sb_pos == PaneScrollbarPosition::Left {
                if room < PANE_MINIMUM as i32 {
                    xoff = xoff
                        .wrapping_add(sx as i32)
                        .wrapping_sub(PANE_MINIMUM as i32);
                    sx = PANE_MINIMUM;
                } else {
                    sx = (sx as i32).wrapping_sub(sb_w).wrapping_sub(sb_pad) as u32;
                    xoff = xoff.wrapping_add(sb_w).wrapping_add(sb_pad);
                }
            } else if room < PANE_MINIMUM as i32 {
                sx = PANE_MINIMUM;
            } else {
                sx = (sx as i32).wrapping_sub(sb_w).wrapping_sub(sb_pad) as u32;
            }
        }

        host.set_pane_offset(wp, xoff, yoff);
        host.pane_resize(wp, sx, sy);

        let (new_xoff, new_yoff, new_sx, new_sy) = host.pane_geometry(wp);
        if new_xoff != old_xoff || new_yoff != old_yoff || new_sx != old_sx || new_sy != old_sy {
            if reserve {
                host.pane_flags_insert(wp, PaneFlags::REDRAWSCROLLBAR);
            }
            changed = true;
        }
    }
    if changed {
        host.invalidate_scene(w);
    }
}

/// `layout_count_cells` (`layout.c:489-508`).
pub fn count_cells(cells: &Cells, lc: LayoutCellId, with_floating: bool) -> u32 {
    let c = cell(cells, lc);
    match c.kind {
        LayoutType::Windowpane => {
            if c.is_floating() && !with_floating {
                0
            } else {
                1
            }
        }
        LayoutType::Leftright | LayoutType::Topbottom => c
            .children
            .iter()
            .map(|&child| count_cells(cells, child, with_floating))
            .sum(),
    }
}

fn need_env(env: Option<&LayoutEnv>) -> &LayoutEnv {
    env.expect("layout_resize_check on a detached tree")
}

/// `layout_resize_check` (`layout.c:511-567`): how much can be removed from a
/// cell in direction `kind`. Requires a window environment.
pub fn resize_check(cells: &Cells, env: &LayoutEnv, lc: LayoutCellId, kind: LayoutType) -> u32 {
    // Floating cells do not take space from the tiled layout.
    if !tiled_or_has_tiled(cells, lc) {
        return 0;
    }
    let c = cell(cells, lc);
    if c.is_leaf() {
        // Space available in this cell only.
        let (available, minimum) = if kind == LayoutType::Leftright {
            let minimum = if env.scrollbars == PaneScrollbarPolicy::Always {
                (PANE_MINIMUM as i32)
                    .wrapping_add(env.scrollbar_width)
                    .wrapping_add(env.scrollbar_pad) as u32
            } else {
                PANE_MINIMUM
            };
            (c.g.sx, minimum)
        } else {
            let minimum = if add_horizontal_border(cells, env.root, lc, env.pane_status) {
                PANE_MINIMUM + 1
            } else {
                PANE_MINIMUM
            };
            (c.g.sy, minimum)
        };
        available.saturating_sub(minimum)
    } else if c.kind == kind {
        // Same type: total of available space in all child cells.
        c.children.iter().fold(0u32, |acc, &child| {
            acc.wrapping_add(resize_check(cells, env, child, kind))
        })
    } else {
        // Different type: minimum of available space in child cells.
        let mut minimum = u32::MAX;
        for &child in &c.children {
            if !tiled_or_has_tiled(cells, child) {
                continue;
            }
            let available = resize_check(cells, env, child, kind);
            if available < minimum {
                minimum = available;
            }
        }
        minimum
    }
}

/// `layout_resize_adjust` (`layout.c:569-634`). The change must already be
/// bounded by `resize_check`; shrinking requires an environment.
pub fn resize_adjust(
    cells: &mut Cells,
    env: Option<&LayoutEnv>,
    lc: LayoutCellId,
    kind: LayoutType,
    mut change: i32,
) {
    // Adjust the cell size.
    {
        let c = cell_mut(cells, lc);
        if kind == LayoutType::Leftright {
            c.g.sx = c.g.sx.wrapping_add(change as u32);
        } else {
            c.g.sy = c.g.sy.wrapping_add(change as u32);
        }
    }

    // If this is a leaf cell, that is all that is necessary.
    if kind == LayoutType::Windowpane {
        return;
    }

    let n = children_len(cells, lc);

    // Child cell runs in a different direction.
    if cell(cells, lc).kind != kind {
        for i in 0..n {
            let child = child_at(cells, lc, i);
            if !tiled_or_has_tiled(cells, child) {
                continue;
            }
            resize_adjust(cells, env, child, kind, change);
        }
        return;
    }

    // If a node doesn't contain any tiled cells, there is nothing to do.
    if !cell_has_tiled_child(cells, lc) {
        return;
    }

    // Child cell runs in the same direction. Adjust each child equally until
    // no further change is possible.
    while change != 0 {
        let mut changed = false;
        for i in 0..n {
            if change == 0 {
                break;
            }
            let child = child_at(cells, lc, i);
            if !tiled_or_has_tiled(cells, child) {
                continue;
            }
            if change > 0 {
                resize_adjust(cells, env, child, kind, 1);
                change -= 1;
                changed = true;
                continue;
            }
            if resize_check(cells, need_env(env), child, kind) > 0 {
                resize_adjust(cells, env, child, kind, -1);
                change += 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// `layout_resize_set_size` (`layout.c:636-648`).
pub fn resize_set_size(
    cells: &mut Cells,
    env: Option<&LayoutEnv>,
    lc: LayoutCellId,
    kind: LayoutType,
    size: u32,
) {
    let g = cell(cells, lc).g;
    let change = if kind == LayoutType::Leftright {
        size.wrapping_sub(g.sx)
    } else {
        size.wrapping_sub(g.sy)
    } as i32;
    resize_adjust(cells, env, lc, kind, change);
}

/// `layout_cell_get_neighbour_dir` (`layout.c:650-667`): `forward` walks
/// toward the tail.
fn cell_get_neighbour_dir(cells: &Cells, lc: LayoutCellId, forward: bool) -> Option<LayoutCellId> {
    let parent = cell(cells, lc).parent?;
    let siblings = &cell(cells, parent).children;
    let mut i = position(cells, parent, lc);
    loop {
        let next = if forward {
            i += 1;
            siblings.get(i).copied()
        } else {
            if i == 0 {
                return None;
            }
            i -= 1;
            Some(siblings[i])
        };
        match next {
            None => return None,
            Some(lcn) if tiled_or_has_tiled(cells, lcn) => return Some(lcn),
            Some(_) => {}
        }
    }
}

/// `layout_cell_get_neighbour` (`layout.c:669-691`): prefers the next
/// sibling, the previous one for the last child.
pub fn cell_get_neighbour(cells: &Cells, lc: LayoutCellId) -> Option<LayoutCellId> {
    let parent = cell(cells, lc).parent?;
    let mut forward = true;
    if cell(cells, parent).children.last() == Some(&lc) {
        forward = !forward;
    }
    cell_get_neighbour_dir(cells, lc, forward)
        .or_else(|| cell_get_neighbour_dir(cells, lc, !forward))
}

/// `layout_destroy_cell` (`layout.c:694-752`). `env` is `None` for a detached
/// parse tree (`layout_destroy_cell(NULL, ...)`, `layout-custom.c:642`).
pub fn destroy_cell<C: LayoutCells>(
    store: &mut C,
    env: Option<&LayoutEnv>,
    lc: LayoutCellId,
    root: &mut Option<LayoutCellId>,
) {
    // If no parent, this is the last pane in a window.
    let Some(parent) = cell(store.cells(), lc).parent else {
        if *root == Some(lc) {
            *root = None;
        }
        free_cell(store, Some(lc), false);
        return;
    };

    if cell_is_tiled(store.cells(), lc) {
        let other = cell_get_neighbour(store.cells(), lc);
        if let Some(other) = other {
            let parent_kind = cell(store.cells(), parent).kind;
            let g = cell(store.cells(), lc).g;
            let change = if parent_kind == LayoutType::Leftright {
                g.sx.wrapping_add(1)
            } else {
                g.sy.wrapping_add(1)
            } as i32;
            resize_adjust(store.cells_mut(), env, other, parent_kind, change);
        } else {
            remove_tile_cells(store.cells_mut(), env, parent);
        }
    }

    // Remove this from the parent's list.
    unlink_child(store.cells_mut(), parent, lc);
    free_cell(store, Some(lc), false);

    // If the parent now has one cell, remove the parent from the tree and
    // replace it by that cell.
    let children = &cell(store.cells(), parent).children;
    if children.len() == 1 {
        let only = children[0];
        let grandparent = cell(store.cells(), parent).parent;
        unlink_child(store.cells_mut(), parent, only);
        cell_mut(store.cells_mut(), only).parent = grandparent;
        match grandparent {
            None => {
                if cell_is_tiled(store.cells(), only) {
                    let c = cell_mut(store.cells_mut(), only);
                    c.g.xoff = 0;
                    c.g.yoff = 0;
                }
                *root = Some(only);
            }
            Some(gp) => {
                let i = position(store.cells(), gp, parent);
                cell_mut(store.cells_mut(), gp).children[i] = only;
                cell_mut(store.cells_mut(), parent).parent = None;
            }
        }
        free_cell(store, Some(parent), false);
    }
}

/// `layout_init` (`layout.c:754-764`).
pub fn init<H: LayoutHost>(host: &mut H, w: WindowId, wp: PaneId) {
    let (sx, sy) = host.window_size(w);
    let lc = create_cell(host.cells_mut(), None);
    host.set_window_layout_root(w, Some(lc));
    set_size(host.cells_mut(), lc, sx, sy, 0, 0);
    make_leaf(host, lc, wp);
    fix_panes(host, w, None);
}

/// `layout_free` (`layout.c:766-771`).
pub fn free<H: LayoutHost>(host: &mut H, w: WindowId, only_nodes: bool) {
    let root = host.window_layout_root(w);
    free_cell(host, root, only_nodes);
}

/// `layout_clamp_floating_panes` (`layout.c:773-814`). Keeps the C unsigned
/// arithmetic in `offset + size + pad`.
fn clamp_floating_panes<H: LayoutHost>(host: &mut H, w: WindowId, sx: u32, sy: u32) {
    let n = host.window_z_index(w).len();
    for i in 0..n {
        let wp = host.window_z_index(w)[i];
        let Some(lc) = host.pane_layout_cell(wp) else {
            continue;
        };
        if !cell(host.cells(), lc).is_floating() {
            continue;
        }
        let pad: u32 = if host.pane_lines(wp) == PaneLines::None {
            0
        } else {
            1
        };

        let g = cell(host.cells(), lc).g;
        let mut csx = g.sx;
        let mut avail = sx.saturating_sub(2 * pad);
        if csx > avail {
            csx = if avail > PANE_MINIMUM {
                avail
            } else {
                PANE_MINIMUM
            };
        }
        let mut csy = g.sy;
        avail = sy.saturating_sub(2 * pad);
        if csy > avail {
            csy = if avail > PANE_MINIMUM {
                avail
            } else {
                PANE_MINIMUM
            };
        }
        if csx != g.sx || csy != g.sy {
            set_size(host.cells_mut(), lc, csx, csy, g.xoff, g.yoff);
        }

        let c = cell_mut(host.cells_mut(), lc);
        if (c.g.xoff as u32).wrapping_add(c.g.sx).wrapping_add(pad) > sx {
            if c.g.sx.wrapping_add(2 * pad) >= sx {
                c.g.xoff = pad as i32;
            } else {
                c.g.xoff = (sx - c.g.sx - pad) as i32;
            }
        }
        if (c.g.yoff as u32).wrapping_add(c.g.sy).wrapping_add(pad) > sy {
            if c.g.sy.wrapping_add(2 * pad) >= sy {
                c.g.yoff = pad as i32;
            } else {
                c.g.yoff = (sy - c.g.sy - pad) as i32;
            }
        }
    }
}

/// `layout_resize` (`layout.c:816-872`): the whole layout after a window
/// resize. The layout can stay larger than the window.
pub fn resize<H: LayoutHost>(host: &mut H, w: WindowId, sx: u32, sy: u32) {
    let lc = host
        .window_layout_root(w)
        .expect("layout_resize without a layout root");
    let env = host.layout_env(w);
    let root = cell(host.cells(), lc);

    if root.is_leaf() && root.is_floating() {
        clamp_floating_panes(host, w, sx, sy);
        fix_panes(host, w, None);
        return;
    }

    // Adjust horizontally. Do not attempt to reduce the layout lower than the
    // minimum (more than the amount returned by resize_check).
    let mut xchange = sx.wrapping_sub(root.g.sx) as i32;
    let xlimit = resize_check(host.cells(), &env, lc, LayoutType::Leftright) as i32;
    if xchange < 0 && xchange < xlimit.wrapping_neg() {
        xchange = xlimit.wrapping_neg();
    }
    if xlimit == 0 {
        if sx <= root.g.sx {
            xchange = 0;
        } else {
            xchange = sx.wrapping_sub(root.g.sx) as i32;
        }
    }
    if xchange != 0 {
        resize_adjust(
            host.cells_mut(),
            Some(&env),
            lc,
            LayoutType::Leftright,
            xchange,
        );
    }

    // Adjust vertically in a similar fashion.
    let root_sy = cell(host.cells(), lc).g.sy;
    let mut ychange = sy.wrapping_sub(root_sy) as i32;
    let ylimit = resize_check(host.cells(), &env, lc, LayoutType::Topbottom) as i32;
    if ychange < 0 && ychange < ylimit.wrapping_neg() {
        ychange = ylimit.wrapping_neg();
    }
    if ylimit == 0 {
        if sy <= root_sy {
            ychange = 0;
        } else {
            ychange = sy.wrapping_sub(root_sy) as i32;
        }
    }
    if ychange != 0 {
        resize_adjust(
            host.cells_mut(),
            Some(&env),
            lc,
            LayoutType::Topbottom,
            ychange,
        );
    }

    // Fix cell offsets.
    fix_offsets(host, w);
    clamp_floating_panes(host, w, sx, sy);
    fix_panes(host, w, None);
}

/// Walk up from `wp`'s cell to the first ancestor node of `kind`; returns the
/// child on that path and the ancestor (`layout.c:879-888`, `1014-1021`).
fn find_parent_of_type(
    cells: &Cells,
    mut lc: LayoutCellId,
    kind: LayoutType,
) -> Option<(LayoutCellId, LayoutCellId)> {
    let mut parent = cell(cells, lc).parent;
    while let Some(p) = parent {
        if cell(cells, p).kind == kind {
            return Some((lc, p));
        }
        lc = p;
        parent = cell(cells, lc).parent;
    }
    None
}

/// `layout_resize_pane_to` (`layout.c:874-905`).
pub fn resize_pane_to<H: LayoutHost>(host: &mut H, wp: PaneId, kind: LayoutType, new_size: u32) {
    let start = host
        .pane_layout_cell(wp)
        .expect("pane without a layout cell");
    let Some((lc, _)) = find_parent_of_type(host.cells(), start, kind) else {
        return;
    };

    // Work out the size adjustment.
    let g = cell(host.cells(), lc).g;
    let size = if kind == LayoutType::Leftright {
        g.sx
    } else {
        g.sy
    };
    let change = if cell_is_last_tiled(host.cells(), lc) {
        size.wrapping_sub(new_size)
    } else {
        new_size.wrapping_sub(size)
    } as i32;

    resize_pane(host, wp, kind, change, true);
}

fn not_floating() -> LayoutError {
    LayoutError::new("pane is not floating")
}

/// `layout_resize_floating_pane_to` (`layout.c:907-937`).
pub fn resize_floating_pane_to<H: LayoutHost>(
    host: &mut H,
    wp: PaneId,
    kind: LayoutType,
    mut size: u32,
) -> Result<(), LayoutError> {
    let lc = host
        .pane_layout_cell(wp)
        .expect("pane without a layout cell");
    if !cell(host.cells(), lc).is_floating() {
        return Err(not_floating());
    }

    if host.pane_lines(wp) != PaneLines::None && size >= PANE_MINIMUM + 2 {
        size -= 2;
    }
    if !(PANE_MINIMUM..=PANE_MAXIMUM).contains(&size) {
        return Err(LayoutError::new("size is too big or too small"));
    }

    let c = cell_mut(host.cells_mut(), lc);
    if kind == LayoutType::Topbottom {
        if c.g.sy == size {
            return Ok(());
        }
        c.g.sy = size;
    } else {
        if c.g.sx == size {
            return Ok(());
        }
        c.g.sx = size;
    }
    let w = host.pane_window(wp);
    host.invalidate_scene(w);
    Ok(())
}

/// `layout_resize_floating_pane` (`layout.c:939-976`).
pub fn resize_floating_pane<H: LayoutHost>(
    host: &mut H,
    wp: PaneId,
    kind: LayoutType,
    change: i32,
    opposite: bool,
) -> Result<(), LayoutError> {
    let lc = host
        .pane_layout_cell(wp)
        .expect("pane without a layout cell");
    if !cell(host.cells(), lc).is_floating() {
        return Err(not_floating());
    }
    if change == 0 {
        return Ok(());
    }

    let c = cell_mut(host.cells_mut(), lc);
    if kind == LayoutType::Topbottom {
        let size = c.g.sy.wrapping_add(change as u32);
        if !(PANE_MINIMUM..=PANE_MAXIMUM).contains(&size) {
            return Err(LayoutError::new("change is too big or too small"));
        }
        c.g.sy = size;
        if opposite {
            c.g.yoff = c.g.yoff.wrapping_sub(change);
        }
    } else {
        let size = c.g.sx.wrapping_add(change as u32);
        if !(PANE_MINIMUM..=PANE_MAXIMUM).contains(&size) {
            return Err(LayoutError::new("change is too big or too small"));
        }
        c.g.sx = size;
        if opposite {
            c.g.xoff = c.g.xoff.wrapping_sub(change);
        }
    }
    let w = host.pane_window(wp);
    host.invalidate_scene(w);
    Ok(())
}

/// `layout_resize_layout` (`layout.c:978-1005`).
pub fn resize_layout<H: LayoutHost>(
    host: &mut H,
    w: WindowId,
    lc: LayoutCellId,
    kind: LayoutType,
    change: i32,
    opposite: bool,
) {
    let env = host.layout_env(w);

    // Grow or shrink the cell.
    let mut needed = change;
    while needed != 0 {
        let size = if change > 0 {
            let size = resize_pane_grow(host.cells_mut(), &env, lc, kind, needed, opposite);
            needed = needed.wrapping_sub(size);
            size
        } else {
            let size = resize_pane_shrink(host.cells_mut(), &env, lc, kind, needed);
            needed = needed.wrapping_add(size);
            size
        };
        if size == 0 {
            // no more change possible
            break;
        }
    }

    // Fix cell offsets.
    fix_offsets(host, w);
    fix_panes(host, w, None);
    host.fire_window_event(w, "window-layout-changed");
}

/// `layout_resize_pane` (`layout.c:1007-1031`).
pub fn resize_pane<H: LayoutHost>(
    host: &mut H,
    wp: PaneId,
    kind: LayoutType,
    change: i32,
    opposite: bool,
) {
    let start = host
        .pane_layout_cell(wp)
        .expect("pane without a layout cell");
    let Some((mut lc, _)) = find_parent_of_type(host.cells(), start, kind) else {
        return;
    };

    // If this is the last tiled cell, move back one.
    if cell_is_last_tiled(host.cells(), lc) {
        match cell_get_neighbour_dir(host.cells(), lc, false) {
            Some(prev) => lc = prev,
            None => return,
        }
    }

    let w = host.pane_window(wp);
    resize_layout(host, w, lc, kind, change, opposite);
}

/// `layout_resize_pane_grow` (`layout.c:1033-1072`).
fn resize_pane_grow(
    cells: &mut Cells,
    env: &LayoutEnv,
    lc: LayoutCellId,
    kind: LayoutType,
    needed: i32,
    opposite: bool,
) -> i32 {
    let mut size: u32 = 0;

    // Growing. Always add to the current cell. Look towards the tail for a
    // suitable cell for reduction.
    let mut remove = cell_get_neighbour_dir(cells, lc, true);
    while let Some(r) = remove {
        size = resize_check(cells, env, r, kind);
        if size > 0 {
            break;
        }
        remove = cell_get_neighbour_dir(cells, r, true);
    }

    // If none found, look towards the head.
    if opposite && remove.is_none() {
        remove = cell_get_neighbour_dir(cells, lc, false);
        while let Some(r) = remove {
            size = resize_check(cells, env, r, kind);
            if size > 0 {
                break;
            }
            remove = cell_get_neighbour_dir(cells, r, false);
        }
    }
    let Some(remove) = remove else {
        return 0;
    };

    // Change the cells.
    if size > needed as u32 {
        size = needed as u32;
    }
    resize_adjust(cells, Some(env), lc, kind, size as i32);
    resize_adjust(cells, Some(env), remove, kind, (size as i32).wrapping_neg());
    size as i32
}

/// `layout_resize_pane_shrink` (`layout.c:1074-1104`).
fn resize_pane_shrink(
    cells: &mut Cells,
    env: &LayoutEnv,
    lc: LayoutCellId,
    kind: LayoutType,
    needed: i32,
) -> i32 {
    // Shrinking. Find cell to remove from by walking towards head.
    let mut remove = Some(lc);
    let mut size = 0;
    while let Some(r) = remove {
        size = resize_check(cells, env, r, kind);
        if size != 0 {
            break;
        }
        remove = cell_get_neighbour_dir(cells, r, false);
    }
    let Some(remove) = remove else {
        return 0;
    };

    // And add onto the next cell (from the original cell).
    let Some(add) = cell_get_neighbour_dir(cells, lc, true) else {
        return 0;
    };

    // Change the cells.
    if size > needed.wrapping_neg() as u32 {
        size = needed.wrapping_neg() as u32;
    }
    resize_adjust(cells, Some(env), add, kind, size as i32);
    resize_adjust(cells, Some(env), remove, kind, (size as i32).wrapping_neg());
    size as i32
}

/// `layout_assign_pane` (`layout.c:1106-1116`).
pub fn assign_pane<H: LayoutHost>(host: &mut H, lc: LayoutCellId, wp: PaneId, do_not_resize: bool) {
    make_leaf(host, lc, wp);
    let w = host.pane_window(wp);
    if do_not_resize {
        fix_panes(host, w, Some(wp));
    } else {
        fix_panes(host, w, None);
    }
}

/// `layout_new_pane_size` (`layout.c:1118-1152`); the C parameter list.
#[allow(clippy::too_many_arguments)]
fn new_pane_size(
    cells: &Cells,
    env: &LayoutEnv,
    previous: u32,
    lc: LayoutCellId,
    kind: LayoutType,
    size: u32,
    count_left: u32,
    size_left: u32,
) -> u32 {
    // If this is the last cell, it can take all of the remaining size.
    if count_left == 1 {
        return size_left;
    }

    // How much is available in this parent?
    let available = resize_check(cells, env, lc, kind);

    // Work out the minimum size of this cell and the new size proportionate
    // to the previous size.
    let g = cell(cells, lc).g;
    let mut min = (PANE_MINIMUM + 1).wrapping_mul(count_left.wrapping_sub(1));
    let current = if kind == LayoutType::Leftright {
        g.sx
    } else {
        g.sy
    };
    if current.wrapping_sub(available) > min {
        min = current.wrapping_sub(available);
    }
    let mut new_size = current.wrapping_mul(size) / previous;

    // Check against the maximum and minimum size.
    let max = size_left.wrapping_sub(min);
    if new_size > max {
        new_size = max;
    }
    if new_size < PANE_MINIMUM {
        new_size = PANE_MINIMUM;
    }
    new_size
}

/// `layout_set_size_check` (`layout.c:1154-1207`).
fn set_size_check(
    cells: &Cells,
    env: &LayoutEnv,
    lc: LayoutCellId,
    kind: LayoutType,
    size: i32,
) -> bool {
    let c = cell(cells, lc);

    // Cells with no children must just be bigger than minimum.
    if c.is_leaf() {
        return size >= PANE_MINIMUM as i32;
    }
    let size = size as u32;
    let mut available = size;

    // Count number of children.
    let count = c.children.len() as u32;

    // Check new size will work for each child.
    if c.kind == kind {
        if available < count.wrapping_mul(2).wrapping_sub(1) {
            return false;
        }

        let previous = if kind == LayoutType::Leftright {
            c.g.sx
        } else {
            c.g.sy
        };

        for (idx, &child) in c.children.iter().enumerate() {
            let idx = idx as u32;
            let new_size = new_pane_size(
                cells,
                env,
                previous,
                child,
                kind,
                size,
                count - idx,
                available,
            );
            if idx == count - 1 {
                if new_size > available {
                    return false;
                }
                available -= new_size;
            } else {
                if new_size.wrapping_add(1) > available {
                    return false;
                }
                available -= new_size + 1;
            }
            if !set_size_check(cells, env, child, kind, new_size as i32) {
                return false;
            }
        }
    } else {
        for &child in &c.children {
            if cell(cells, child).is_leaf() {
                continue;
            }
            if !set_size_check(cells, env, child, kind, size as i32) {
                return false;
            }
        }
    }
    true
}

/// `layout_resize_child_cells` (`layout.c:1209-1270`).
fn resize_child_cells(cells: &mut Cells, env: &LayoutEnv, lc: LayoutCellId) {
    let parent = cell(cells, lc);
    if parent.is_leaf() {
        return;
    }
    let kind = parent.kind;
    let pg = parent.g;
    let n = parent.children.len();

    // What is the current size used?
    let mut count: u32 = 0;
    let mut prev: u32 = 0;
    for &child in &parent.children {
        if !tiled_or_has_tiled(cells, child) {
            continue;
        }
        count += 1;
        let g = cell(cells, child).g;
        if kind == LayoutType::Leftright {
            prev = prev.wrapping_add(g.sx);
        } else {
            prev = prev.wrapping_add(g.sy);
        }
    }
    prev = prev.wrapping_add(count.wrapping_sub(1));

    // And how much is available?
    let mut available = if kind == LayoutType::Leftright {
        pg.sx
    } else {
        pg.sy
    };

    // Resize children into the new size.
    let mut idx: u32 = 0;
    for i in 0..n {
        let child = child_at(cells, lc, i);
        if !tiled_or_has_tiled(cells, child) {
            continue;
        }
        if kind == LayoutType::Topbottom {
            let c = cell_mut(cells, child);
            c.g.sx = pg.sx;
            c.g.xoff = pg.xoff;
        } else {
            let sx = new_pane_size(cells, env, prev, child, kind, pg.sx, count - idx, available);
            cell_mut(cells, child).g.sx = sx;
            available = available.wrapping_sub(sx.wrapping_add(1));
        }
        if kind == LayoutType::Leftright {
            let c = cell_mut(cells, child);
            c.g.sy = pg.sy;
            c.g.yoff = pg.yoff;
        } else {
            let sy = new_pane_size(cells, env, prev, child, kind, pg.sy, count - idx, available);
            cell_mut(cells, child).g.sy = sy;
            available = available.wrapping_sub(sy.wrapping_add(1));
        }
        resize_child_cells(cells, env, child);
        idx += 1;
    }
}

/// `layout_replace_with_node` (`layout.c:1272-1296`): wrap `lc` in a new node
/// of `kind` that takes its place (and the window root when `lc` was it).
pub fn replace_with_node<H: LayoutHost>(
    host: &mut H,
    w: WindowId,
    lc: LayoutCellId,
    kind: LayoutType,
) -> LayoutCellId {
    let (old_parent, g) = {
        let c = cell(host.cells(), lc);
        (c.parent, c.g)
    };
    let parent = create_cell(host.cells_mut(), old_parent);
    make_node(host, parent, kind);
    set_size(host.cells_mut(), parent, g.sx, g.sy, g.xoff, g.yoff);
    match old_parent {
        None => host.set_window_layout_root(w, Some(parent)),
        Some(gp) => {
            let i = position(host.cells(), gp, lc);
            cell_mut(host.cells_mut(), gp).children[i] = parent;
        }
    }

    // Insert the old cell.
    let c = cell_mut(host.cells_mut(), lc);
    c.parent = Some(parent);
    cell_mut(host.cells_mut(), parent).children.insert(0, lc);
    parent
}

/// `layout_split_check_space` (`layout.c:1298-1336`): enough space for two
/// panes. Reads the split pane's scrollbar style.
pub fn split_check_space<H: LayoutHost>(
    host: &H,
    wp: PaneId,
    lc: LayoutCellId,
    kind: LayoutType,
) -> bool {
    let w = host.pane_window(wp);
    let root = host.window_layout_root(w);
    let c = cell(host.cells(), lc);
    if c.is_floating() {
        panic!("floating cells cannot be split");
    }
    let status = host.window_pane_status(w);
    match kind {
        LayoutType::Leftright => {
            let minimum = if host.window_scrollbars(w) == PaneScrollbarPolicy::Always {
                let (width, pad) = host.pane_scrollbar_style(wp);
                (PANE_MINIMUM as i32 * 2)
                    .wrapping_add(width)
                    .wrapping_add(pad) as u32
            } else {
                PANE_MINIMUM * 2 + 1
            };
            c.g.sx >= minimum
        }
        LayoutType::Topbottom => {
            let minimum = if add_horizontal_border(host.cells(), root, lc, status) {
                PANE_MINIMUM * 2 + 2
            } else {
                PANE_MINIMUM * 2 + 1
            };
            c.g.sy >= minimum
        }
        LayoutType::Windowpane => panic!("bad layout type"),
    }
}

/// `layout_split_sizes` (`layout.c:1338-1365`): `size < 0` is the default
/// half split.
pub fn split_sizes(
    cells: &Cells,
    lc: LayoutCellId,
    size: i32,
    before: bool,
    kind: LayoutType,
) -> SplitSizes {
    let g = cell(cells, lc).g;
    split_sizes_of(
        if kind == LayoutType::Leftright {
            g.sx
        } else {
            g.sy
        },
        size,
        before,
    )
}

pub(super) fn split_sizes_of(ss: u32, size: i32, before: bool) -> SplitSizes {
    let mut s2 = if size < 0 {
        (ss.wrapping_add(1) / 2).wrapping_sub(1)
    } else if before {
        ss.wrapping_sub(size as u32).wrapping_sub(1)
    } else {
        size as u32
    };
    if s2 < PANE_MINIMUM {
        s2 = PANE_MINIMUM;
    } else if s2 > ss.wrapping_sub(2) {
        s2 = ss.wrapping_sub(2);
    }
    let s1 = ss.wrapping_sub(1).wrapping_sub(s2);
    SplitSizes {
        size1: s1,
        size2: s2,
        saved: ss,
    }
}

/// `layout_split_pane` (`layout.c:1367-1498`). `size` is a hint, or -1 for
/// the default half split. Must be followed by `assign_pane`.
pub fn split_pane<H: LayoutHost>(
    host: &mut H,
    wp: PaneId,
    kind: LayoutType,
    size: i32,
    flags: SpawnFlags,
) -> Option<LayoutCellId> {
    let w = host.pane_window(wp);
    let full_size = flags.contains(SpawnFlags::FULLSIZE);
    let before = flags.contains(SpawnFlags::BEFORE);

    // If full_size is specified, add a new cell at the top of the window
    // layout. Otherwise, split the cell for the current pane.
    let lc = if full_size {
        host.window_layout_root(w)
            .expect("window without a layout root")
    } else {
        host.pane_layout_cell(wp)
            .expect("pane without a layout cell")
    };

    // Copy the old cell size.
    let LayoutGeometry { sx, sy, xoff, yoff } = cell(host.cells(), lc).g;

    // Check there is enough space for the two new panes.
    if !split_check_space(host, wp, lc, kind) {
        return None;
    }

    // Calculate new cell sizes: size1 is the top/left and size2 the
    // bottom/right.
    let SplitSizes {
        size1,
        size2,
        saved,
    } = split_sizes(host.cells(), lc, size, before, kind);

    // Which size are we using?
    let new_size = if before { size2 } else { size1 };

    // Confirm there is enough space for full size pane.
    let mut env = host.layout_env(w);
    if full_size && !set_size_check(host.cells(), &env, lc, kind, new_size as i32) {
        return None;
    }

    let (parent, parent_kind, lc_kind) = {
        let c = cell(host.cells(), lc);
        (
            c.parent,
            c.parent.map(|p| cell(host.cells(), p).kind),
            c.kind,
        )
    };
    let mut resize_first = false;
    let lcnew;
    if let (Some(parent), Some(pk)) = (parent, parent_kind)
        && pk == kind
    {
        // If the parent exists and is of the same type as the split, create
        // a new cell and insert it after this one.
        lcnew = create_cell(host.cells_mut(), Some(parent));
        let i = position(host.cells(), parent, lc);
        let at = if before { i } else { i + 1 };
        cell_mut(host.cells_mut(), parent)
            .children
            .insert(at, lcnew);
    } else if full_size && parent.is_none() && lc_kind == kind {
        // If the new full size pane is the same type as the root split,
        // insert the new pane under the existing root cell instead of
        // creating a new root cell. The existing layout must be resized
        // before inserting the new cell.
        if lc_kind == LayoutType::Leftright {
            cell_mut(host.cells_mut(), lc).g.sx = new_size;
            resize_child_cells(host.cells_mut(), &env, lc);
            cell_mut(host.cells_mut(), lc).g.sx = saved;
        } else {
            cell_mut(host.cells_mut(), lc).g.sy = new_size;
            resize_child_cells(host.cells_mut(), &env, lc);
            cell_mut(host.cells_mut(), lc).g.sy = saved;
        }
        resize_first = true;

        // Create the new cell.
        lcnew = create_cell(host.cells_mut(), Some(lc));
        let rest = saved.wrapping_sub(1).wrapping_sub(new_size);
        if lc_kind == LayoutType::Leftright {
            set_size(host.cells_mut(), lcnew, rest, sy, 0, 0);
        } else {
            set_size(host.cells_mut(), lcnew, sx, rest, 0, 0);
        }
        let at = if before {
            0
        } else {
            children_len(host.cells(), lc)
        };
        cell_mut(host.cells_mut(), lc).children.insert(at, lcnew);
    } else {
        // Otherwise create a new parent and insert it.
        let parent = replace_with_node(host, w, lc, kind);
        env.root = host.window_layout_root(w);

        // Create the new child cell.
        lcnew = create_cell(host.cells_mut(), Some(parent));
        let at = if before {
            0
        } else {
            children_len(host.cells(), parent)
        };
        cell_mut(host.cells_mut(), parent)
            .children
            .insert(at, lcnew);
    }
    let (lc1, lc2) = if before { (lcnew, lc) } else { (lc, lcnew) };

    // Set new cell sizes. size1 is the size of the top/left and size2 the
    // bottom/right.
    if !resize_first && kind == LayoutType::Leftright {
        set_size(host.cells_mut(), lc1, size1, sy, xoff, yoff);
        let lc1_sx = cell(host.cells(), lc1).g.sx;
        set_size(
            host.cells_mut(),
            lc2,
            size2,
            sy,
            xoff.wrapping_add(lc1_sx as i32).wrapping_add(1),
            yoff,
        );
    } else if !resize_first && kind == LayoutType::Topbottom {
        set_size(host.cells_mut(), lc1, sx, size1, xoff, yoff);
        let lc1_sy = cell(host.cells(), lc1).g.sy;
        set_size(
            host.cells_mut(),
            lc2,
            sx,
            size2,
            xoff,
            yoff.wrapping_add(lc1_sy as i32).wrapping_add(1),
        );
    }
    if full_size {
        if !resize_first {
            resize_child_cells(host.cells_mut(), &env, lc);
        }
        fix_offsets(host, w);
    } else {
        make_leaf(host, lc, wp);
    }

    Some(lcnew)
}

/// `layout_floating_pane` (`layout.c:1500-1530`): a cell for a new floating
/// pane, inserted after the anchor. Must be followed by `assign_pane`.
pub fn floating_pane<H: LayoutHost>(
    host: &mut H,
    w: WindowId,
    wp: Option<PaneId>,
    lg: &LayoutGeometry,
) -> LayoutCellId {
    let lc = match wp {
        None => host
            .window_layout_root(w)
            .expect("window without a layout root"),
        Some(wp) => host
            .pane_layout_cell(wp)
            .expect("pane without a layout cell"),
    };
    let parent = match cell(host.cells(), lc).parent {
        Some(p) => p,
        // Adding a pane to a root that isn't a node. Must create and insert a
        // new root.
        None => replace_with_node(host, w, lc, LayoutType::Topbottom),
    };

    let lcnew = create_cell(host.cells_mut(), Some(parent));
    let i = position(host.cells(), parent, lc);
    cell_mut(host.cells_mut(), parent)
        .children
        .insert(i + 1, lcnew);
    cell_mut(host.cells_mut(), lcnew)
        .flags
        .insert(LayoutCellFlags::FLOATING);
    set_size(host.cells_mut(), lcnew, lg.sx, lg.sy, lg.xoff, lg.yoff);
    lcnew
}

/// `layout_close_pane` (`layout.c:1532-1551`).
pub fn close_pane<H: LayoutHost>(host: &mut H, wp: PaneId) {
    let w = host.pane_window(wp);
    let Some(lc) = host.pane_layout_cell(wp) else {
        return;
    };

    // Remove the cell.
    let env = host.layout_env(w);
    let mut root = host.window_layout_root(w);
    destroy_cell(host, Some(&env), lc, &mut root);
    host.set_window_layout_root(w, root);
    host.set_pane_layout_cell(wp, None);

    // Fix pane offsets and sizes.
    if root.is_some() {
        fix_offsets(host, w);
        fix_panes(host, w, None);
    }
    host.fire_window_event(w, "window-layout-changed");
}

/// `layout_spread_cell` (`layout.c:1553-1618`): even out the direct tiled
/// leaves of `parent`. Returns whether anything changed.
pub fn spread_cell<H: LayoutHost>(host: &mut H, w: WindowId, parent: LayoutCellId) -> bool {
    let env = host.layout_env(w);
    let cells = host.cells_mut();
    let p = cell(cells, parent);
    let number = p
        .children
        .iter()
        .filter(|&&lc| cell_is_tiled(cells, lc))
        .count() as u32;
    if number <= 1 {
        return false;
    }
    let status = env.pane_status;

    let size = match p.kind {
        LayoutType::Leftright => p.g.sx,
        LayoutType::Topbottom => {
            if add_horizontal_border(cells, env.root, parent, status) {
                p.g.sy.wrapping_sub(1)
            } else {
                p.g.sy
            }
        }
        LayoutType::Windowpane => return false,
    };
    if size < number - 1 {
        return false;
    }
    let each = (size - (number - 1)) / number;
    if each == 0 {
        return false;
    }

    // Remaining space after assigning that which can be evenly distributed.
    let mut remainder = size
        .wrapping_sub(number.wrapping_mul(each + 1))
        .wrapping_add(1);

    let kind = p.kind;
    let n = p.children.len();
    let mut changed = false;
    for i in 0..n {
        let lc = child_at(cells, parent, i);
        if !cell_is_tiled(cells, lc) {
            continue;
        }
        let g = cell(cells, lc).g;
        let mut change = 0i32;
        if kind == LayoutType::Leftright {
            change = each.wrapping_sub(g.sx) as i32;
            if remainder > 0 {
                change = change.wrapping_add(1);
                remainder -= 1;
            }
            resize_adjust(cells, Some(&env), lc, LayoutType::Leftright, change);
        } else if kind == LayoutType::Topbottom {
            let mut this = if add_horizontal_border(cells, env.root, lc, status) {
                each + 1
            } else {
                each
            };
            if remainder > 0 {
                this += 1;
                remainder -= 1;
            }
            change = this.wrapping_sub(g.sy) as i32;
            resize_adjust(cells, Some(&env), lc, LayoutType::Topbottom, change);
        }
        if change != 0 {
            changed = true;
        }
    }
    changed
}

/// `layout_spread_out` (`layout.c:1620-1638`).
pub fn spread_out<H: LayoutHost>(host: &mut H, wp: PaneId) {
    let w = host.pane_window(wp);
    let lc = host
        .pane_layout_cell(wp)
        .expect("pane without a layout cell");
    let mut parent = cell(host.cells(), lc).parent;
    while let Some(p) = parent {
        if spread_cell(host, w, p) {
            fix_offsets(host, w);
            fix_panes(host, w, None);
            break;
        }
        parent = cell(host.cells(), p).parent;
    }
}

/// `layout_get_tiled_cell` (`layout.c:1640-1697`): the command-facing split.
pub fn get_tiled_cell<H: LayoutHost + ArgumentFormatRuntime>(
    host: &mut H,
    item: QueueItemId,
    args: &Args,
    w: WindowId,
    wp: PaneId,
    flags: SpawnFlags,
) -> Result<LayoutCellId, LayoutError> {
    if host.pane_is_floating(wp) {
        return Err(LayoutError::new("can't split a floating pane"));
    }

    let kind = if flags.contains(SpawnFlags::HORIZONTAL) {
        LayoutType::Leftright
    } else {
        LayoutType::Topbottom
    };

    let mut size: i32 = -1;
    let mut curval: u32 = 0;
    if args.has(b'l') != 0 || args.has(b'p') != 0 {
        curval = if flags.contains(SpawnFlags::FULLSIZE) {
            let (sx, sy) = host.window_size(w);
            if kind == LayoutType::Topbottom {
                sy
            } else {
                sx
            }
        } else {
            let (_, _, sx, sy) = host.pane_geometry(wp);
            if kind == LayoutType::Topbottom {
                sy
            } else {
                sx
            }
        };
    }

    let result = if args.has(b'l') != 0 {
        args.percentage_and_expand(host, b'l', 0, i32::MAX as i64, curval as i64, item)
    } else if args.has(b'p') != 0 {
        args.strtonum_and_expand(host, b'p', 0, 100, item)
            .map(|pct| (curval.wrapping_mul(pct as u32) / 100) as i64)
    } else {
        Ok(size as i64)
    };
    match result {
        Ok(value) => size = value as i32,
        Err(error) => {
            let mut cause = ByteString::from("invalid tiled geometry ");
            cause.extend_from_slice(&error);
            return Err(LayoutError { cause });
        }
    }

    if host.window_active_pane_is_over_zoom(w) {
        host.window_push_zoom(w, false, true);
    } else {
        host.window_push_zoom(w, true, flags.contains(SpawnFlags::ZOOM));
    }
    split_pane(host, wp, kind, size, flags)
        .ok_or_else(|| LayoutError::new("no space for a new pane"))
}

/// `layout_get_floating_cell` (`layout.c:1699-1726`).
pub fn get_floating_cell<H: LayoutHost + ArgumentFormatRuntime>(
    host: &mut H,
    item: QueueItemId,
    args: &Args,
    lines: PaneLines,
    w: WindowId,
    wp: PaneId,
    flags: SpawnFlags,
) -> Result<LayoutCellId, LayoutError> {
    let lc = host
        .pane_layout_cell(wp)
        .expect("pane without a layout cell");
    let fg = if flags.contains(SpawnFlags::SPLIT) {
        split_floating_cell(host, lc, w, lines, flags)?
    } else {
        let mut fg = LayoutGeometry::UNSET;
        floating_args_parse(host, item, args, lines, w, &mut fg)?;
        fg
    };

    let pw = host.pane_window(wp);
    if flags.contains(SpawnFlags::FLOATOVERZOOM) || host.window_active_pane_is_over_zoom(w) {
        host.window_push_zoom(pw, false, true);
    } else {
        host.window_push_zoom(pw, true, flags.contains(SpawnFlags::ZOOM));
    }
    Ok(floating_pane(host, w, Some(wp), &fg))
}

fn position_error(error: &[u8]) -> LayoutError {
    let mut cause = ByteString::from("position ");
    cause.extend_from_slice(error);
    LayoutError { cause }
}

/// `layout_floating_args_parse` (`layout.c:1728-1833`): `-x -y -X -Y` into a
/// geometry, with the cascade for an unset position.
pub fn floating_args_parse<H: LayoutHost + ArgumentFormatRuntime>(
    host: &mut H,
    item: QueueItemId,
    args: &Args,
    lines: PaneLines,
    w: WindowId,
    lg: &mut LayoutGeometry,
) -> Result<(), LayoutError> {
    let (wsx, wsy) = host.window_size(w);
    let mut sx: i32 = if lg.sx == u32::MAX {
        (wsx / 2) as i32
    } else {
        lg.sx as i32
    };
    let mut sy: i32 = if lg.sy == u32::MAX {
        (wsy / 4) as i32
    } else {
        lg.sy as i32
    };
    let mut ox = lg.xoff;
    let mut oy = lg.yoff;

    if args.has(b'x') != 0 {
        sx = args
            .percentage_and_expand(host, b'x', 0, PANE_MAXIMUM as i64, wsx as i64, item)
            .map_err(|e| position_error(&e))? as i32;
        if lines != PaneLines::None {
            sx -= 2;
        }
    }
    if args.has(b'y') != 0 {
        sy = args
            .percentage_and_expand(host, b'y', 0, PANE_MAXIMUM as i64, wsy as i64, item)
            .map_err(|e| position_error(&e))? as i32;
        if lines != PaneLines::None {
            sy -= 2;
        }
    }
    if args.has(b'X') != 0 {
        ox = args
            .percentage_and_expand(host, b'X', -(sx as i64), wsx as i64, wsx as i64, item)
            .map_err(|e| position_error(&e))? as i32;
    }
    if args.has(b'Y') != 0 {
        oy = args
            .percentage_and_expand(host, b'Y', -(sy as i64), wsy as i64, wsy as i64, item)
            .map_err(|e| position_error(&e))? as i32;
    }

    if !host.window_has_floating_panes(w) {
        host.set_window_last_new_pane(w, 0, 0);
    }
    let pad = if lines != PaneLines::None { 1 } else { 0 };
    let (last_x, last_y) = host.window_last_new_pane(w);
    if ox == i32::MAX {
        if last_x == 0 {
            ox = 4;
        } else {
            ox = last_x.wrapping_add(4);
            if ox.wrapping_add(sx).wrapping_add(pad) > wsx as i32 {
                ox = 4;
            }
        }
        host.set_window_last_new_pane(w, ox, last_y);
    } else if args.has(b'X') != 0 && lines != PaneLines::None {
        ox += 1;
    }
    let (last_x, last_y) = host.window_last_new_pane(w);
    if oy == i32::MAX {
        if last_y == 0 {
            oy = 2;
        } else {
            oy = last_y.wrapping_add(2);
            if oy.wrapping_add(sy).wrapping_add(pad) > wsy as i32 {
                oy = 2;
            }
        }
        host.set_window_last_new_pane(w, last_x, oy);
    } else if args.has(b'Y') != 0 && lines != PaneLines::None {
        oy += 1;
    }

    if sx < PANE_MINIMUM as i32 || sx > PANE_MAXIMUM as i32 {
        return Err(LayoutError::new("invalid width"));
    }
    if sy < PANE_MINIMUM as i32 || sy > PANE_MAXIMUM as i32 {
        return Err(LayoutError::new("invalid height"));
    }

    lg.sx = sx as u32;
    lg.sy = sy as u32;
    lg.xoff = ox;
    lg.yoff = oy;
    Ok(())
}

/// Signed working copy of a geometry for `split_floating_cell`: C keeps
/// `u_int` sizes and stores negative results as wrapped values, which pass
/// its minimum check and crash the server later. Here a negative size is an
/// ordinary `no space for a new pane` error (deliberate deviation, G13 Risks).
#[derive(Clone, Copy)]
struct SignedGeometry {
    sx: i32,
    sy: i32,
    xoff: i32,
    yoff: i32,
}

/// `layout_split_floating_cell` (`layout.c:1835-1958`): place a new float
/// next to `lc`. On success `lc` is moved and resized and the new geometry is
/// returned; on failure `lc` is unchanged.
pub fn split_floating_cell<H: LayoutHost>(
    host: &mut H,
    lc: LayoutCellId,
    w: WindowId,
    lines: PaneLines,
    flags: SpawnFlags,
) -> Result<LayoutGeometry, LayoutError> {
    let (wsx, wsy) = host.window_size(w);
    let tborder: i32 = 1;
    let bborder: i32 = (wsy as i32).wrapping_sub(1);
    let lborder: i32 = 3;
    let rborder: i32 = (wsx as i32).wrapping_sub(3);
    let border: i32 = if lines != PaneLines::None { 1 } else { 0 };
    let horizontal = flags.contains(SpawnFlags::HORIZONTAL);
    let before = flags.contains(SpawnFlags::BEFORE);

    // First, move the target cell in-bounds.
    let g = cell(host.cells(), lc).g;
    let mut old = SignedGeometry {
        sx: g.sx as i32,
        sy: g.sy as i32,
        xoff: g.xoff,
        yoff: g.yoff,
    };
    if lborder > old.xoff - border {
        old.xoff = lborder + border;
    }
    if rborder < old.xoff + old.sx + border {
        old.xoff = rborder - old.sx - border;
    }
    if tborder > old.yoff - border {
        old.yoff = tborder + border;
    }
    if bborder < old.yoff + old.sy + border {
        old.yoff = bborder - old.sy - border;
    }

    // Move the new cell to its ideal position.
    let mut new = old;
    if horizontal {
        if before {
            new.xoff -= old.sx + 2 * border;
        } else {
            new.xoff += old.sx + 2 * border;
        }
    } else if before {
        new.yoff -= old.sy + 2 * border;
    } else {
        new.yoff += old.sy + 2 * border;
    }

    // If the new cell is out of bounds, the available space is split and
    // equally given to both cells. Only one border is checked because the
    // target cell is in bounds already.
    if lborder > new.xoff - border {
        // Offsets are where pane contents start, so pane borders are removed
        // from the space; 1 is added in case the space is odd.
        let space = old.xoff + old.sx - lborder - 3 * border + 1;
        let size = space / 2;
        new.sx = size;
        old.sx = size;
        new.xoff = lborder + border;
        old.xoff = new.xoff + new.sx + 2 * border;
        // If the original space was odd (now even), subtract 1 from the
        // rightmost cell.
        if space % 2 == 0 {
            old.sx -= 1;
        }
    } else if rborder < new.xoff + new.sx + border {
        let space = rborder - old.xoff - 3 * border + 1;
        let size = space / 2;
        new.sx = size;
        old.sx = size;
        new.xoff = old.xoff + old.sx + 2 * border;
        if space % 2 == 0 {
            new.sx -= 1;
        }
    } else if tborder > new.yoff - border {
        let space = old.sy + old.yoff - tborder - 3 * border + 1;
        let size = space / 2;
        new.sy = size;
        old.sy = size;
        new.yoff = tborder + border;
        old.yoff = new.yoff + new.sy + 2 * border;
        if space % 2 == 0 {
            old.sy -= 1;
        }
    } else if bborder < new.yoff + new.sy + border {
        let space = bborder - old.yoff - 3 * border + 1;
        let size = space / 2;
        new.sy = size;
        old.sy = size;
        new.yoff = old.yoff + old.sy + 2 * border;
        if space % 2 == 0 {
            new.sy -= 1;
        }
    }

    // Expand the cell to occupy the whole available space where it was
    // spawned.
    if flags.contains(SpawnFlags::FULLSIZE) {
        if horizontal {
            new.yoff = tborder + border;
            new.sy = bborder - tborder - 2 * border;
            if before {
                new.xoff = lborder + border;
                new.sx = old.xoff - new.xoff - 2 * border;
            } else {
                new.sx = rborder - new.xoff - border;
            }
        } else {
            new.xoff = lborder + border;
            new.sx = rborder - lborder - 2 * border;
            if before {
                new.yoff = tborder + border;
                new.sy = old.yoff - new.yoff - 2 * border;
            } else {
                new.sy = bborder - new.yoff - border;
            }
        }
    }

    let min = PANE_MINIMUM as i32;
    if new.sx < min || new.sy < min || old.sx < min || old.sy < min {
        return Err(LayoutError::new("no space for a new pane"));
    }

    set_size(
        host.cells_mut(),
        lc,
        old.sx as u32,
        old.sy as u32,
        old.xoff,
        old.yoff,
    );
    Ok(LayoutGeometry::new(
        new.sx as u32,
        new.sy as u32,
        new.xoff,
        new.yoff,
    ))
}

/// `layout_remove_tile` on the cell arena (`layout.c:1960-1998`); recursion
/// and detached callers.
fn remove_tile_cells(cells: &mut Cells, env: Option<&LayoutEnv>, lc: LayoutCellId) -> bool {
    if cell(cells, lc).is_floating() {
        return false;
    }

    let neighbour = cell_get_neighbour(cells, lc);
    match neighbour {
        None => {
            if let Some(parent) = cell(cells, lc).parent {
                remove_tile_cells(cells, env, parent);
            }
        }
        Some(neighbour) => {
            if let Some(parent) = cell(cells, neighbour).parent {
                let kind = cell(cells, parent).kind;
                // Adding the size of the layout cell plus its border to the
                // neighbour.
                let g = cell(cells, lc).g;
                let change = if kind == LayoutType::Topbottom {
                    g.sy.wrapping_add(1)
                } else {
                    g.sx.wrapping_add(1)
                } as i32;
                resize_adjust(cells, env, neighbour, kind, change);
            }
        }
    }

    // Zero the cell geometry until the cell is retiled unless this is the
    // top level node.
    if cell(cells, lc).parent.is_some() {
        set_size(cells, lc, 0, 0, 0, 0);
    }
    true
}

/// `layout_remove_tile` (`layout.c:1960-1998`): give the cell's space to its
/// neighbour. `true` for C 0, `false` for -1. Does not set `FLOATING`.
pub fn remove_tile<H: LayoutHost>(host: &mut H, w: WindowId, lc: LayoutCellId) -> bool {
    let env = host.layout_env(w);
    remove_tile_cells(host.cells_mut(), Some(&env), lc)
}

/// `layout_insert_tile` (`layout.c:2000-2062`): put a cell back into the
/// tiled layout with half of its neighbour's space. `false` for C -1. Does
/// not clear `FLOATING`.
pub fn insert_tile<H: LayoutHost>(host: &mut H, w: WindowId, lc: LayoutCellId) -> bool {
    if cell_is_tiled(host.cells(), lc) {
        return false;
    }

    let Some(parent) = cell(host.cells(), lc).parent else {
        // Only pane in the layout.
        let (sx, sy) = host.window_size(w);
        set_size(host.cells_mut(), lc, sx, sy, 0, 0);
        return true;
    };

    let mut kind = cell(host.cells(), parent).kind;
    let env = host.layout_env(w);
    match cell_get_neighbour(host.cells(), lc) {
        None => {
            // This will become the only visible cell in the parent. Tile the
            // parent, then set the child's 'split' size.
            insert_tile(host, w, parent);
            let pg = cell(host.cells(), parent).g;
            let size1 = if kind == LayoutType::Leftright {
                pg.sx
            } else {
                pg.sy
            };
            resize_set_size(host.cells_mut(), Some(&env), lc, kind, size1);
        }
        Some(neighbour) => {
            // If the neighbour is a node, a tiled child in the subtree of
            // the neighbour is needed to check for space.
            let tiled = cell_get_first_tiled(host.cells(), neighbour)
                .expect("neighbour without a tiled leaf");
            let tiled_wp = cell(host.cells(), tiled)
                .pane
                .expect("tiled leaf without a pane");
            if !split_check_space(host, tiled_wp, neighbour, kind) {
                return false;
            }
            let SplitSizes { size1, size2, .. } =
                split_sizes(host.cells(), neighbour, -1, false, kind);
            resize_set_size(host.cells_mut(), Some(&env), lc, kind, size1);
            resize_set_size(host.cells_mut(), Some(&env), neighbour, kind, size2);
        }
    }

    // Setting opposite of the 'split' size to that of the parent.
    let pg = cell(host.cells(), parent).g;
    let size1;
    if cell(host.cells(), parent).kind == LayoutType::Leftright {
        size1 = pg.sy;
        kind = LayoutType::Topbottom;
    } else {
        size1 = pg.sx;
        kind = LayoutType::Leftright;
    }
    resize_set_size(host.cells_mut(), Some(&env), lc, kind, size1);
    true
}
