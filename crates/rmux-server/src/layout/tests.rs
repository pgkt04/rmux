// Ported from tmux layout.c, layout-custom.c, layout-set.c @ 8f25579c (tests)
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

use rmux_emu::screen::PaneLines;
use rmux_util::bytes::ByteString;

use super::fixture::FakeServer;
use super::*;
use crate::cmd::arguments::{Args, ArgsEntryFlags, ArgsValue};
use crate::ids::{ArenaId, LayoutCellId, PaneId, QueueItemId, WindowId};
use crate::model::spawn::SpawnFlags;
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use crate::ui::status::PaneStatusPosition;

const LR: LayoutType = LayoutType::Leftright;
const TB: LayoutType = LayoutType::Topbottom;

fn item() -> QueueItemId {
    QueueItemId::from_parts(0, 0)
}

fn args(pairs: &[(u8, &str)]) -> Args {
    let mut a = Args::create();
    for &(flag, value) in pairs {
        let value = if value.is_empty() {
            None
        } else {
            Some(ArgsValue::string(ByteString::from(value)))
        };
        a.set(flag, value, ArgsEntryFlags::default());
    }
    a
}

/// `split-window` on `wp`: split, add a pane and assign it.
fn split(
    srv: &mut FakeServer,
    wp: PaneId,
    kind: LayoutType,
    size: i32,
    flags: SpawnFlags,
) -> PaneId {
    let w = srv.pane(wp).window;
    let lc = split_pane(srv, wp, kind, size, flags).expect("no space for a new pane");
    let new = srv.add_pane(w);
    assign_pane(srv, lc, new, false);
    new
}

fn split_cmd(
    srv: &mut FakeServer,
    wp: PaneId,
    pairs: &[(u8, &str)],
    flags: SpawnFlags,
) -> Result<PaneId, LayoutError> {
    let w = srv.pane(wp).window;
    let lc = get_tiled_cell(srv, item(), &args(pairs), w, wp, flags)?;
    let new = srv.add_pane(w);
    assign_pane(srv, lc, new, false);
    Ok(new)
}

/// `new-pane` with `-x -y -X -Y`.
fn new_float(
    srv: &mut FakeServer,
    wp: PaneId,
    pairs: &[(u8, &str)],
    lines: PaneLines,
) -> Result<PaneId, LayoutError> {
    let w = srv.pane(wp).window;
    let lc = get_floating_cell(
        srv,
        item(),
        &args(pairs),
        lines,
        w,
        wp,
        SpawnFlags::default(),
    )?;
    let new = srv.add_pane(w);
    srv.pane_mut(new).lines = lines;
    assign_pane(srv, lc, new, false);
    Ok(new)
}

fn kill(srv: &mut FakeServer, wp: PaneId) {
    close_pane(srv, wp);
    srv.remove_pane(wp);
}

fn g(srv: &FakeServer, wp: PaneId) -> (i32, i32, u32, u32) {
    let lc = srv.pane(wp).layout_cell.unwrap();
    let g = srv.cells.get(lc).unwrap().g;
    (g.xoff, g.yoff, g.sx, g.sy)
}

fn pane_geom(srv: &FakeServer, wp: PaneId) -> (i32, i32, u32, u32) {
    let p = srv.pane(wp);
    (p.xoff, p.yoff, p.sx, p.sy)
}

fn v2(srv: &FakeServer, w: WindowId) -> String {
    let root = srv.win(w).layout_root;
    String::from_utf8(
        dump(srv, w, root, LayoutDumpFlags::default())
            .unwrap()
            .into_vec(),
    )
    .unwrap()
}

fn v1(srv: &FakeServer, w: WindowId) -> String {
    let root = srv.win(w).layout_root;
    String::from_utf8(
        dump(srv, w, root, LayoutDumpFlags::OLD_FORMAT)
            .unwrap()
            .into_vec(),
    )
    .unwrap()
}

fn with_checksum(body: &str) -> String {
    format!("{:04x},{}", custom::checksum(body.as_bytes()), body)
}

fn cause(r: Result<(), LayoutError>) -> String {
    String::from_utf8(r.unwrap_err().cause.into_vec()).unwrap()
}

fn tree_ok(srv: &FakeServer, w: WindowId) {
    // Every assigned leaf links back; every child names its parent.
    fn walk(srv: &FakeServer, lc: LayoutCellId, parent: Option<LayoutCellId>) {
        let c = srv.cells.get(lc).unwrap();
        assert_eq!(c.parent, parent, "parent link of {lc:?}");
        if let Some(wp) = c.pane {
            assert_eq!(srv.pane(wp).layout_cell, Some(lc));
        }
        for &child in &c.children {
            walk(srv, child, Some(lc));
        }
    }
    walk(srv, srv.win(w).layout_root.unwrap(), None);
}

// --- work item 1: build and free -------------------------------------------

#[test]
fn free_clears_current_links_but_not_saved() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, TB, -1, SpawnFlags::default());
    let root = srv.win(w).layout_root.unwrap();
    let c1 = srv.pane(p1).layout_cell.unwrap();
    srv.pane_mut(p1).saved_layout_cell = Some(c1);
    free(&mut srv, w, false);
    assert_eq!(srv.pane(p0).layout_cell, None);
    assert_eq!(srv.pane(p1).layout_cell, None);
    assert_eq!(srv.pane(p1).saved_layout_cell, Some(c1));
    assert!(srv.cells.get(root).is_none());
    assert!(srv.cells.is_empty());
}

#[test]
fn free_only_nodes_keeps_leaves() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, TB, -1, SpawnFlags::default());
    free(&mut srv, w, true);
    let c0 = srv.pane(p0).layout_cell.unwrap();
    let c1 = srv.pane(p1).layout_cell.unwrap();
    assert_eq!(srv.cells.get(c0).unwrap().parent, None);
    assert_eq!(srv.cells.get(c1).unwrap().pane, Some(p1));
    assert_eq!(srv.cells.len(), 2);
}

#[test]
fn detached_tree_frees_without_panes() {
    let mut cells = Cells::new();
    let root = create_cell(&mut cells, None);
    make_node(&mut cells, root, LR);
    let a = create_cell(&mut cells, Some(root));
    let b = create_cell(&mut cells, Some(root));
    cells.get_mut(root).unwrap().children = vec![a, b];
    free_cell(&mut cells, Some(root), false);
    assert!(cells.is_empty());
}

// --- work item 2: queries ----------------------------------------------------

#[test]
fn queries_with_floating_sibling() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, LR, -1, SpawnFlags::default());
    let p2 = split(&mut srv, p1, LR, -1, SpawnFlags::default());
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    let root = srv.win(w).layout_root.unwrap();
    let (c0, c1, c2, cf) = (
        srv.pane(p0).layout_cell.unwrap(),
        srv.pane(p1).layout_cell.unwrap(),
        srv.pane(p2).layout_cell.unwrap(),
        srv.pane(f).layout_cell.unwrap(),
    );
    assert!(cell_is_tiled(&srv.cells, c0));
    assert!(!cell_is_tiled(&srv.cells, cf));
    assert!(!cell_is_tiled(&srv.cells, root));
    assert!(cell_has_tiled_child(&srv.cells, root));
    assert!(!cell_has_tiled_child(&srv.cells, cf));
    assert_eq!(count_cells(&srv.cells, root, false), 3);
    assert_eq!(count_cells(&srv.cells, root, true), 4);
    // Float is inserted after p0: children are c0, cf, c1, c2.
    assert_eq!(srv.cells.get(root).unwrap().children, vec![c0, cf, c1, c2]);
    assert_eq!(
        cell_get_neighbour(&srv.cells, c0),
        Some(c1),
        "skips the float"
    );
    assert_eq!(
        cell_get_neighbour(&srv.cells, c2),
        Some(c1),
        "last prefers previous"
    );
    assert_eq!(cell_get_neighbour(&srv.cells, root), None);
    assert!(srv.pane_is_floating(f));
    assert!(srv.window_has_floating_panes(w));
    assert_eq!(srv.window_count_panes(w, false), 3);
    tree_ok(&srv, w);
}

#[test]
fn search_by_border_walks_children_in_order() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, LR, -1, SpawnFlags::default());
    let _p2 = split(&mut srv, p1, TB, -1, SpawnFlags::default());
    let root = srv.win(w).layout_root.unwrap();
    let c0 = srv.pane(p0).layout_cell.unwrap();
    let c1 = srv.pane(p1).layout_cell.unwrap();
    // p0 is 0..40, border at 40, right node 41..80 split at row 12.
    assert_eq!(search_by_border(&srv.cells, root, 40, 5), Some(c0));
    assert_eq!(search_by_border(&srv.cells, root, 50, 12), Some(c1));
    assert_eq!(
        search_by_border(&srv.cells, root, 5, 5),
        None,
        "inside a leaf"
    );
    assert_eq!(search_by_border(&srv.cells, root, 200, 200), None);
    // A float overlapping the border: the float child is inside, so recursion
    // into the (leaf) float returns None.
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "10"), (b'y', "6"), (b'X', "38"), (b'Y', "2")],
        PaneLines::None,
    )
    .unwrap();
    assert_eq!(g(&srv, f), (38, 2, 10, 6));
    // The float is inside, so the search recurses into it and finds nothing.
    assert_eq!(search_by_border(&srv.cells, root, 40, 5), None);
}

// --- work item 3: fix_offsets and fix_panes ----------------------------------

#[test]
fn fix_panes_status_borders() {
    for (status, top, bottom) in [
        (PaneStatusPosition::Off, (0, 0, 80, 11), (0, 12, 80, 12)),
        (PaneStatusPosition::Top, (0, 1, 80, 10), (0, 12, 80, 12)),
        (PaneStatusPosition::Bottom, (0, 0, 80, 11), (0, 12, 80, 11)),
    ] {
        let mut srv = FakeServer::new();
        let (w, p0) = srv.window(80, 24);
        srv.win_mut(w).pane_status = status;
        let p1 = split(&mut srv, p0, TB, 12, SpawnFlags::default());
        assert_eq!(pane_geom(&srv, p0), top, "{status:?}");
        assert_eq!(pane_geom(&srv, p1), bottom, "{status:?}");
    }
}

#[test]
fn fix_panes_scrollbars() {
    for (pos, width, pad, left, right) in [
        (
            PaneScrollbarPosition::Right,
            1,
            0,
            (0, 0, 39, 24),
            (41, 0, 38, 24),
        ),
        (
            PaneScrollbarPosition::Left,
            1,
            0,
            (1, 0, 39, 24),
            (42, 0, 38, 24),
        ),
        (
            PaneScrollbarPosition::Left,
            2,
            1,
            (3, 0, 37, 24),
            (44, 0, 36, 24),
        ),
        // Too narrow for the bar: PANE_MINIMUM at the right edge of the cell.
        (
            PaneScrollbarPosition::Left,
            50,
            0,
            (39, 0, 1, 24),
            (79, 0, 1, 24),
        ),
        (
            PaneScrollbarPosition::Right,
            50,
            0,
            (0, 0, 1, 24),
            (41, 0, 1, 24),
        ),
    ] {
        let mut srv = FakeServer::new();
        let (w, p0) = srv.window(80, 24);
        let p1 = split(&mut srv, p0, LR, -1, SpawnFlags::default());
        {
            let win = srv.win_mut(w);
            win.sb = PaneScrollbarPolicy::Always;
            win.sb_pos = pos;
        }
        for wp in [p0, p1] {
            srv.pane_mut(wp).scrollbar_width = width;
            srv.pane_mut(wp).scrollbar_pad = pad;
            srv.pane_mut(wp).flags.remove(PaneFlags::REDRAWSCROLLBAR);
        }
        srv.invalidations.clear();
        fix_panes(&mut srv, w, None);
        assert_eq!(pane_geom(&srv, p0), left, "{pos:?} {width} {pad}");
        assert_eq!(pane_geom(&srv, p1), right, "{pos:?} {width} {pad}");
        assert!(srv.pane(p0).flags.contains(PaneFlags::REDRAWSCROLLBAR));
        assert_eq!(srv.invalidations.len(), 1);
        // Unchanged geometry: no redraw.
        srv.invalidations.clear();
        fix_panes(&mut srv, w, None);
        assert!(srv.invalidations.is_empty());
    }
}

#[test]
fn fix_panes_skips_pane_and_cell_less_panes() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, TB, -1, SpawnFlags::default());
    let p2 = srv.add_pane(w); // no cell
    srv.pane_mut(p1).sx = 7;
    fix_panes(&mut srv, w, Some(p1));
    assert_eq!(srv.pane(p1).sx, 7);
    assert_eq!(pane_geom(&srv, p2), (0, 0, 0, 0));
    fix_panes(&mut srv, w, None);
    assert_eq!(srv.pane(p1).sx, 80);
}

// --- work item 4: resize check / adjust / window resize ------------------------

#[test]
fn resize_check_and_adjust_with_floating_sibling() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, LR, -1, SpawnFlags::default());
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    let root = srv.win(w).layout_root.unwrap();
    let env = srv.layout_env(w);
    let cf = srv.pane(f).layout_cell.unwrap();
    assert_eq!(resize_check(&srv.cells, &env, cf, LR), 0);
    assert_eq!(resize_check(&srv.cells, &env, root, LR), 39 + 38);
    assert_eq!(
        resize_check(&srv.cells, &env, root, TB),
        23,
        "minimum over children"
    );
    resize_adjust(&mut srv.cells, Some(&env), root, LR, -10);
    assert_eq!(g(&srv, p0).2 + g(&srv, p1).2 + 1, 70);
    assert_eq!(g(&srv, f), (4, 2, 18, 4), "float untouched");
    resize_adjust(&mut srv.cells, Some(&env), root, TB, -4);
    assert_eq!(g(&srv, p0).3, 20);
    assert_eq!(g(&srv, p1).3, 20);
    assert_eq!(g(&srv, f).3, 4);
}

#[test]
fn resize_check_status_and_scrollbars() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, TB, -1, SpawnFlags::default());
    srv.win_mut(w).pane_status = PaneStatusPosition::Top;
    let env = srv.layout_env(w);
    let (c0, c1) = (
        srv.pane(p0).layout_cell.unwrap(),
        srv.pane(p1).layout_cell.unwrap(),
    );
    assert_eq!(
        resize_check(&srv.cells, &env, c0, TB),
        12 - 2,
        "top cell keeps the status row"
    );
    assert_eq!(resize_check(&srv.cells, &env, c1, TB), 11 - 1);
    srv.win_mut(w).pane_status = PaneStatusPosition::Bottom;
    let env = srv.layout_env(w);
    assert_eq!(resize_check(&srv.cells, &env, c0, TB), 11);
    assert_eq!(resize_check(&srv.cells, &env, c1, TB), 9);
    srv.win_mut(w).sb = PaneScrollbarPolicy::Always;
    srv.pane_mut(p0).scrollbar_width = 3;
    srv.pane_mut(p0).scrollbar_pad = 1;
    let env = srv.layout_env(w);
    assert_eq!(
        resize_check(&srv.cells, &env, c1, LR),
        80 - 5,
        "active pane style"
    );
}

#[test]
fn window_resize_keeps_minimum_and_grows_back() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let mut panes = vec![p0];
    for _ in 0..2 {
        let last = *panes.last().unwrap();
        panes.push(split(&mut srv, last, LR, -1, SpawnFlags::default()));
    }
    resize(&mut srv, w, 3, 2);
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(
        srv.cells.get(root).unwrap().g.sx,
        5,
        "3 panes need 5 columns"
    );
    assert_eq!(srv.cells.get(root).unwrap().g.sy, 2);
    resize(&mut srv, w, 200, 50);
    let rg = srv.cells.get(root).unwrap().g;
    assert_eq!((rg.sx, rg.sy), (200, 50));
    let widths: Vec<u32> = panes.iter().map(|&p| g(&srv, p).2).collect();
    assert_eq!(widths.iter().sum::<u32>() + 2, 200);
    tree_ok(&srv, w);
}

#[test]
fn clamp_floating_panes_unsigned_rules() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "40"), (b'y', "10"), (b'X', "30"), (b'Y', "10")],
        PaneLines::Single,
    )
    .unwrap();
    assert_eq!(g(&srv, f), (31, 11, 38, 8));
    // Far-edge overflow moves the offset to size - cell - pad.
    resize(&mut srv, w, 60, 15);
    assert_eq!(g(&srv, f), (60 - 38 - 1, 15 - 8 - 1, 38, 8));
    // Smaller than the cell plus pads: shrink to avail and offset becomes pad.
    resize(&mut srv, w, 20, 6);
    assert_eq!(g(&srv, f), (1, 1, 18, 4));
    // Window smaller than the pads: PANE_MINIMUM and offset pad.
    resize(&mut srv, w, 2, 2);
    assert_eq!(g(&srv, f), (1, 1, 1, 1));
    // A negative offset does not trip the far-edge test unless it wraps past
    // the window size; -5 + 1 + 1 wraps so the cell is moved to the pad.
    srv.win_mut(w).sx = 80;
    srv.win_mut(w).sy = 24;
    let cf = srv.pane(f).layout_cell.unwrap();
    set_size(&mut srv.cells, cf, 10, 5, -20, -8);
    resize(&mut srv, w, 80, 24);
    assert_eq!(g(&srv, f), (69, 18, 10, 5));
    // -5 + 10 + 1 does not wrap past the window: untouched (lower bound is
    // not enforced).
    set_size(&mut srv.cells, cf, 10, 5, -5, -2);
    resize(&mut srv, w, 80, 24);
    assert_eq!(g(&srv, f), (-5, -2, 10, 5));
    // In range: untouched (lower bound is not enforced).
    set_size(&mut srv.cells, cf, 10, 5, -5, -2);
    srv.pane_mut(f).lines = PaneLines::None;
    set_size(&mut srv.cells, cf, 10, 5, 0, 0);
    resize(&mut srv, w, 80, 24);
    assert_eq!(g(&srv, f), (0, 0, 10, 5));
}

// --- work item 5: resize_pane --------------------------------------------------

#[test]
fn resize_pane_last_cell_steps_back_and_opposite() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, LR, -1, SpawnFlags::default());
    let p2 = split(&mut srv, p1, LR, -1, SpawnFlags::default());
    // 80 = 40 + 1 + 19 + 1 + 19 (p1 split 39 -> 19/19).
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (40, 19, 19));
    // Growing the last pane steps back to p1, which takes from p2 (the
    // shared border moves left).
    srv.events.clear();
    resize_pane(&mut srv, p2, LR, 5, false);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (40, 24, 14));
    assert_eq!(srv.events_named("window-layout-changed"), 1);
    // Growing p0 takes from p1 (tail-ward), then p2 when p1 is exhausted.
    resize_pane(&mut srv, p0, LR, 20, false);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (60, 4, 14));
    // Beyond the limit: stops when nothing can give.
    resize_pane(&mut srv, p0, LR, 100, false);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (76, 1, 1));
    // Shrink p0 gives to p1.
    resize_pane(&mut srv, p0, LR, -10, false);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (66, 11, 1));
    // Growing p1 with opposite takes from the head when the tail is exhausted.
    resize_pane(&mut srv, p1, LR, 5, false);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (66, 11, 1));
    resize_pane(&mut srv, p1, LR, 5, true);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (61, 16, 1));
    // resize_pane_to on the last cell computes the inverse change.
    resize_pane_to(&mut srv, p2, LR, 10);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (61, 7, 10));
    resize_pane_to(&mut srv, p0, LR, 20);
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (20, 48, 10));
    // No ancestor of that type: nothing happens.
    resize_pane(&mut srv, p0, TB, 5, false);
    assert_eq!(g(&srv, p0).3, 24);
    tree_ok(&srv, w);
}

#[test]
fn floating_resize_causes() {
    let mut srv = FakeServer::new();
    let (_w, p0) = srv.window(80, 24);
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    assert_eq!(
        cause(resize_floating_pane_to(&mut srv, p0, LR, 5)),
        "pane is not floating"
    );
    assert_eq!(
        cause(resize_floating_pane(&mut srv, p0, LR, 5, false)),
        "pane is not floating"
    );
    assert_eq!(
        cause(resize_floating_pane_to(&mut srv, f, LR, 10003)),
        "size is too big or too small"
    );
    assert_eq!(
        cause(resize_floating_pane_to(&mut srv, f, LR, 0)),
        "size is too big or too small"
    );
    // Pane lines take two off the requested size.
    resize_floating_pane_to(&mut srv, f, LR, 12).unwrap();
    assert_eq!(g(&srv, f).2, 10);
    // Size 2 with lines stays 2 (not >= PANE_MINIMUM + 2).
    resize_floating_pane_to(&mut srv, f, TB, 2).unwrap();
    assert_eq!(g(&srv, f).3, 2);
    srv.invalidations.clear();
    resize_floating_pane_to(&mut srv, f, TB, 4).unwrap();
    assert!(srv.invalidations.is_empty(), "equal size is a no-op");
    assert_eq!(
        cause(resize_floating_pane(&mut srv, f, TB, -5, false)),
        "change is too big or too small"
    );
    resize_floating_pane(&mut srv, f, TB, 0, true).unwrap();
    resize_floating_pane(&mut srv, f, TB, 3, true).unwrap();
    assert_eq!(g(&srv, f), (4, -1, 10, 5));
    assert_eq!(srv.invalidations.len(), 1);
}

// --- work item 6: destroy / close / tile ---------------------------------------

#[test]
fn destroy_cell_collapses_and_prefers_next_neighbour() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, LR, -1, SpawnFlags::default());
    let p2 = split(&mut srv, p1, LR, -1, SpawnFlags::default());
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2, g(&srv, p2).2), (40, 19, 19));
    kill(&mut srv, p1);
    assert_eq!(
        (g(&srv, p0).2, g(&srv, p2).2),
        (40, 39),
        "next sibling takes the space"
    );
    assert_eq!(g(&srv, p2).0, 41);
    kill(&mut srv, p2);
    assert_eq!(
        g(&srv, p0),
        (0, 0, 80, 24),
        "last sibling gives to previous; root collapses"
    );
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(root, srv.pane(p0).layout_cell.unwrap());
    assert_eq!(srv.cells.len(), 1);
    kill(&mut srv, p0);
    assert_eq!(srv.win(w).layout_root, None);
    assert!(srv.cells.is_empty());
    assert_eq!(srv.events_named("window-layout-changed"), 3);
}

#[test]
fn destroy_floating_leaves_space_untouched() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, TB, 12, SpawnFlags::default());
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    kill(&mut srv, f);
    assert_eq!(g(&srv, p0), (0, 0, 80, 11));
    assert_eq!(g(&srv, p1), (0, 12, 80, 12));
    // Only a float left: the node collapses and the float becomes the root.
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    kill(&mut srv, p0);
    kill(&mut srv, p1);
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(root, srv.pane(f).layout_cell.unwrap());
    assert_eq!(
        g(&srv, f),
        (4, 2, 18, 4),
        "floating root is not moved to 0,0"
    );
    assert_eq!(v1(&srv, w), "0000,");
}

#[test]
fn remove_and_insert_tile_round_trip() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(81, 24);
    let p1 = split(&mut srv, p0, LR, -1, SpawnFlags::default());
    assert_eq!((g(&srv, p0).2, g(&srv, p1).2), (40, 40));
    let c1 = srv.pane(p1).layout_cell.unwrap();
    assert!(remove_tile(&mut srv, w, c1));
    assert_eq!(g(&srv, p0).2, 81);
    srv.cells
        .get_mut(c1)
        .unwrap()
        .flags
        .insert(LayoutCellFlags::FLOATING);
    assert!(!remove_tile(&mut srv, w, c1), "a floating cell returns -1");
    assert_eq!(g(&srv, p1), (0, 0, 0, 0));
    fix_offsets(&mut srv, w);
    fix_panes(&mut srv, w, None);
    assert_eq!(pane_geom(&srv, p0), (0, 0, 81, 24));
    // Back in: half of the neighbour (81 -> 40/40) and the parent's height.
    assert!(insert_tile(&mut srv, w, c1));
    srv.cells
        .get_mut(c1)
        .unwrap()
        .flags
        .remove(LayoutCellFlags::FLOATING);
    fix_offsets(&mut srv, w);
    assert_eq!(g(&srv, p0), (0, 0, 40, 24));
    assert_eq!(g(&srv, p1), (41, 0, 40, 24));
    assert!(!insert_tile(&mut srv, w, c1), "a tiled cell returns -1");
    // No space: a 3-column window cannot be split again.
    let (w2, q0) = srv.window(3, 5);
    let q1 = split(&mut srv, q0, LR, -1, SpawnFlags::default());
    let d1 = srv.pane(q1).layout_cell.unwrap();
    assert!(remove_tile(&mut srv, w2, d1));
    srv.cells
        .get_mut(d1)
        .unwrap()
        .flags
        .insert(LayoutCellFlags::FLOATING);
    assert_eq!(g(&srv, q0).2, 3);
    let d0 = srv.pane(q0).layout_cell.unwrap();
    set_size(&mut srv.cells, d0, 2, 5, 0, 0);
    assert!(!insert_tile(&mut srv, w2, d1));
}

#[test]
fn insert_tile_root_takes_window_size() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let c0 = srv.pane(p0).layout_cell.unwrap();
    srv.cells
        .get_mut(c0)
        .unwrap()
        .flags
        .insert(LayoutCellFlags::FLOATING);
    set_size(&mut srv.cells, c0, 5, 5, 3, 3);
    assert!(insert_tile(&mut srv, w, c0));
    assert_eq!(g(&srv, p0), (0, 0, 80, 24));
}

// --- work item 7: split --------------------------------------------------------

#[test]
fn split_sizes_table() {
    assert_eq!(
        split_sizes_of(24, -1, false),
        SplitSizes {
            size1: 12,
            size2: 11,
            saved: 24
        }
    );
    assert_eq!(
        split_sizes_of(24, 12, true),
        SplitSizes {
            size1: 12,
            size2: 11,
            saved: 24
        }
    );
    assert_eq!(
        split_sizes_of(24, 30, false),
        SplitSizes {
            size1: 1,
            size2: 22,
            saved: 24
        }
    );
    assert_eq!(
        split_sizes_of(24, 12, false),
        SplitSizes {
            size1: 11,
            size2: 12,
            saved: 24
        }
    );
    assert_eq!(
        split_sizes_of(24, 0, false),
        SplitSizes {
            size1: 22,
            size2: 1,
            saved: 24
        }
    );
    assert_eq!(
        split_sizes_of(80, -1, false),
        SplitSizes {
            size1: 40,
            size2: 39,
            saved: 80
        }
    );
}

#[test]
fn split_three_insertion_cases() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    // (c) replace the leaf root with a node.
    let p1 = split(&mut srv, p0, TB, 12, SpawnFlags::default());
    assert_eq!(
        v2(&srv, w),
        "{\"V\":2,\"L\":{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{\"t\":\"p\",\"w\":80,\"h\":11,\"x\":0,\"y\":0,\"a\":true,\"i\":0,\"I\":\"%0\"},{\"t\":\"p\",\"w\":80,\"h\":12,\"x\":0,\"y\":12,\"i\":1,\"I\":\"%1\"}]}}"
    );
    // (a) same-type parent: insert after p1, before p0.
    let p2 = split(&mut srv, p1, TB, -1, SpawnFlags::default());
    assert_eq!((g(&srv, p1), g(&srv, p2)), ((0, 12, 80, 6), (0, 19, 80, 5)));
    let p3 = split(&mut srv, p0, TB, 3, SpawnFlags::BEFORE);
    assert_eq!((g(&srv, p3), g(&srv, p0)), ((0, 0, 80, 3), (0, 4, 80, 7)));
    let root = srv.win(w).layout_root.unwrap();
    let order: Vec<PaneId> = srv
        .cells
        .get(root)
        .unwrap()
        .children
        .iter()
        .map(|&c| srv.cells.get(c).unwrap().pane.unwrap())
        .collect();
    assert_eq!(order, vec![p3, p0, p1, p2]);
    // (b) full size under a same-type root: children shrink proportionally.
    let p4 = split(&mut srv, p1, TB, 5, SpawnFlags::FULLSIZE);
    assert_eq!(g(&srv, p4), (0, 19, 80, 5));
    let heights: Vec<u32> = [p3, p0, p1, p2].iter().map(|&p| g(&srv, p).3).collect();
    assert_eq!(heights.iter().sum::<u32>() + 3, 18);
    assert_eq!(heights, vec![2, 5, 4, 4]);
    // Full size before with a different type: new root of that type.
    let p5 = split(
        &mut srv,
        p1,
        LR,
        20,
        SpawnFlags::FULLSIZE | SpawnFlags::BEFORE,
    );
    assert_eq!(g(&srv, p5), (0, 0, 20, 24));
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(srv.cells.get(root).unwrap().kind, LR);
    assert_eq!(g(&srv, p4), (21, 19, 59, 5));
    tree_ok(&srv, w);
    assert!(srv.events.is_empty(), "split fires no event");
}

#[test]
fn split_no_space_and_floating_cause() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(3, 3);
    assert!(split_pane(&mut srv, p0, LR, -1, SpawnFlags::default()).is_some());
    assert_eq!(
        cause(split_cmd(&mut srv, p0, &[], SpawnFlags::HORIZONTAL).map(drop)),
        "no space for a new pane"
    );
    srv.win_mut(w).pane_status = PaneStatusPosition::Top;
    let (_, q0) = srv.window(3, 3);
    let (_, r0) = srv.window(3, 4);
    srv.win_mut(srv.pane(q0).window).pane_status = PaneStatusPosition::Top;
    srv.win_mut(srv.pane(r0).window).pane_status = PaneStatusPosition::Top;
    assert!(
        split_pane(&mut srv, q0, TB, -1, SpawnFlags::default()).is_none(),
        "status needs one more row"
    );
    assert!(split_pane(&mut srv, r0, TB, -1, SpawnFlags::default()).is_some());
    let (_, s0) = srv.window(80, 24);
    let f = new_float(
        &mut srv,
        s0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    assert_eq!(
        cause(split_cmd(&mut srv, f, &[], SpawnFlags::default()).map(drop)),
        "can't split a floating pane"
    );
}

#[test]
fn split_command_sizes_and_errors() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split_cmd(&mut srv, p0, &[(b'l', "25%")], SpawnFlags::HORIZONTAL).unwrap();
    assert_eq!(g(&srv, p1).2, 20);
    let p2 = split_cmd(&mut srv, p0, &[(b'p', "50")], SpawnFlags::default()).unwrap();
    assert_eq!(g(&srv, p2).3, 12);
    assert_eq!(
        cause(split_cmd(&mut srv, p0, &[(b'l', "x")], SpawnFlags::default()).map(drop)),
        "invalid tiled geometry invalid"
    );
    assert_eq!(
        cause(split_cmd(&mut srv, p0, &[(b'p', "101")], SpawnFlags::default()).map(drop)),
        "invalid tiled geometry too large"
    );
    // Full size -l uses the window size.
    let p3 = split_cmd(&mut srv, p0, &[(b'l', "50%")], SpawnFlags::FULLSIZE).unwrap();
    assert_eq!(g(&srv, p3), (0, 12, 80, 12));
    assert_eq!(srv.zoom_pushes.len(), 3, "argument errors do not push zoom");
    assert_eq!(srv.zoom_pushes[0], (w, true, false));
    tree_ok(&srv, w);
}

// --- work item 8: floating ----------------------------------------------------

#[test]
fn floating_args_cascade_and_bounds() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let f1 = new_float(&mut srv, p0, &[], PaneLines::Single).unwrap();
    assert_eq!(g(&srv, f1), (4, 2, 40, 6));
    let f2 = new_float(&mut srv, p0, &[], PaneLines::Single).unwrap();
    assert_eq!(g(&srv, f2), (8, 4, 40, 6));
    // 8 + 4 + 40 + 1 > 80 wraps x back to 4; y continues.
    let f3 = new_float(&mut srv, p0, &[(b'x', "70")], PaneLines::Single).unwrap();
    assert_eq!(g(&srv, f3), (4, 6, 68, 6));
    assert_eq!(srv.win(w).last_new_pane_x, 4);
    // Given -X/-Y move by one with pane lines; not without.
    let f4 = new_float(
        &mut srv,
        p0,
        &[(b'X', "10"), (b'Y', "3")],
        PaneLines::Single,
    )
    .unwrap();
    assert_eq!(g(&srv, f4), (11, 4, 40, 6));
    let f5 = new_float(
        &mut srv,
        p0,
        &[(b'x', "10"), (b'y', "4"), (b'X', "10"), (b'Y', "3")],
        PaneLines::None,
    )
    .unwrap();
    assert_eq!(g(&srv, f5), (10, 3, 10, 4));
    // Percentages are of the window size and must be nonnegative; -X ranges
    // down to -sx.
    let f6 = new_float(
        &mut srv,
        p0,
        &[(b'x', "50%"), (b'X', "-10"), (b'Y', "-6")],
        PaneLines::None,
    )
    .unwrap();
    assert_eq!(g(&srv, f6), (-10, -6, 40, 6));
    assert_eq!(
        cause(new_float(&mut srv, p0, &[(b'Y', "-25%")], PaneLines::None).map(drop)),
        "position too small"
    );
    assert_eq!(
        cause(new_float(&mut srv, p0, &[(b'X', "-41")], PaneLines::None).map(drop)),
        "position too small"
    );
    assert_eq!(
        cause(new_float(&mut srv, p0, &[(b'X', "81")], PaneLines::None).map(drop)),
        "position too large"
    );
    assert_eq!(
        cause(new_float(&mut srv, p0, &[(b'x', "a")], PaneLines::None).map(drop)),
        "position invalid"
    );
    assert_eq!(
        cause(new_float(&mut srv, p0, &[(b'x', "0")], PaneLines::None).map(drop)),
        "invalid width"
    );
    assert_eq!(
        cause(new_float(&mut srv, p0, &[(b'x', "2")], PaneLines::Single).map(drop)),
        "invalid width"
    );
    assert_eq!(
        cause(new_float(&mut srv, p0, &[(b'y', "1")], PaneLines::Single).map(drop)),
        "invalid height"
    );
    // A failed check has already moved the cascade (layout.c:1782-1826).
    let (w2, q0) = srv.window(80, 24);
    assert_eq!(
        cause(new_float(&mut srv, q0, &[(b'y', "0")], PaneLines::None).map(drop)),
        "invalid height"
    );
    assert_eq!(
        (srv.win(w2).last_new_pane_x, srv.win(w2).last_new_pane_y),
        (4, 2)
    );
    // No floats: counters reset to 0 before the cascade.
    let f = new_float(&mut srv, q0, &[], PaneLines::None).unwrap();
    assert_eq!(g(&srv, f), (4, 2, 40, 6));
    kill(&mut srv, f);
    let f = new_float(&mut srv, q0, &[], PaneLines::None).unwrap();
    assert_eq!(g(&srv, f), (4, 2, 40, 6));
    tree_ok(&srv, w);
}

#[test]
fn floating_pane_wraps_root_and_inserts_after_anchor() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let lg = LayoutGeometry::new(10, 5, 2, 3);
    let lc = floating_pane(&mut srv, w, None, &lg);
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(srv.cells.get(root).unwrap().kind, TB);
    assert_eq!(
        srv.cells.get(root).unwrap().children,
        vec![srv.pane(p0).layout_cell.unwrap(), lc]
    );
    assert!(srv.cells.get(lc).unwrap().is_floating());
    assert_eq!(srv.cells.get(lc).unwrap().g, lg);
    let f = srv.add_pane(w);
    assign_pane(&mut srv, lc, f, true);
    assert_eq!(
        pane_geom(&srv, f),
        (0, 0, 0, 0),
        "do_not_resize skips the new pane"
    );
    fix_panes(&mut srv, w, None);
    assert_eq!(pane_geom(&srv, f), (2, 3, 10, 5));
    assert_eq!(
        v2(&srv, w),
        "{\"V\":2,\"L\":{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"a\":true,\"i\":0,\"I\":\"%0\"},{\"t\":\"p\",\"w\":10,\"h\":5,\"x\":2,\"y\":3,\"i\":1,\"z\":0,\"I\":\"%1\"}]}}"
    );
}

#[test]
fn split_floating_cell_commit_order() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6"), (b'X', "8"), (b'Y', "3")],
        PaneLines::Single,
    )
    .unwrap();
    assert_eq!(g(&srv, f), (9, 4, 18, 4));
    let cf = srv.pane(f).layout_cell.unwrap();
    // Vertical split below: fits, target unchanged, new cell under it.
    let new =
        split_floating_cell(&mut srv, cf, w, PaneLines::Single, SpawnFlags::default()).unwrap();
    assert_eq!(g(&srv, f), (9, 4, 18, 4));
    assert_eq!(new, LayoutGeometry::new(18, 4, 9, 10));
    // Horizontal before: out of bounds on the left, space is split.
    let new = split_floating_cell(
        &mut srv,
        cf,
        w,
        PaneLines::Single,
        SpawnFlags::HORIZONTAL | SpawnFlags::BEFORE,
    )
    .unwrap();
    assert_eq!(new, LayoutGeometry::new(11, 4, 4, 4));
    assert_eq!(g(&srv, f), (17, 4, 10, 4), "target committed");
    // Full size horizontal after: stretches down the window.
    let new = split_floating_cell(
        &mut srv,
        cf,
        w,
        PaneLines::Single,
        SpawnFlags::HORIZONTAL | SpawnFlags::FULLSIZE,
    )
    .unwrap();
    assert_eq!(new, LayoutGeometry::new(80 - 3 - 29 - 1, 23 - 1 - 2, 29, 2));
    // No space: a one-column float at the right edge leaves no room.
    set_size(&mut srv.cells, cf, 1, 4, 75, 4);
    let before = g(&srv, f);
    assert_eq!(
        cause(
            split_floating_cell(&mut srv, cf, w, PaneLines::Single, SpawnFlags::HORIZONTAL)
                .map(drop)
        ),
        "no space for a new pane"
    );
    assert_eq!(g(&srv, f), before);
    // Through get_floating_cell with SPLIT: zoom push comes after geometry.
    srv.zoom_pushes.clear();
    set_size(&mut srv.cells, cf, 18, 4, 9, 4);
    let lc = get_floating_cell(
        &mut srv,
        item(),
        &args(&[]),
        PaneLines::Single,
        w,
        f,
        SpawnFlags::SPLIT,
    )
    .unwrap();
    assert_eq!(
        srv.cells.get(lc).unwrap().g,
        LayoutGeometry::new(18, 4, 9, 10)
    );
    assert_eq!(srv.zoom_pushes, vec![(w, true, false)]);
    srv.zoom_pushes.clear();
    set_size(&mut srv.cells, cf, 1, 4, 75, 4);
    assert!(
        get_floating_cell(
            &mut srv,
            item(),
            &args(&[]),
            PaneLines::Single,
            w,
            f,
            SpawnFlags::SPLIT | SpawnFlags::HORIZONTAL
        )
        .is_err()
    );
    assert!(
        srv.zoom_pushes.is_empty(),
        "failed geometry does not push zoom"
    );
}

/// Deliberate deviation: `layout_split_floating_cell` computes negative sizes
/// in a window too small for the borders and stores them as wrapped `u_int`
/// values, which pass its minimum check and kill the oracle server (probed:
/// 3x3 window, `split-window -h` on a float). rmux reports `no space for a
/// new pane` instead.
#[test]
fn split_floating_cell_negative_space_is_an_error() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    resize(&mut srv, w, 3, 3);
    srv.window_resize(w, 3, 3);
    assert_eq!(g(&srv, f), (1, 1, 1, 1));
    let cf = srv.pane(f).layout_cell.unwrap();
    for flags in [
        SpawnFlags::HORIZONTAL,
        SpawnFlags::HORIZONTAL | SpawnFlags::BEFORE,
        SpawnFlags::default(),
        SpawnFlags::BEFORE,
        SpawnFlags::FULLSIZE,
    ] {
        assert_eq!(
            cause(split_floating_cell(&mut srv, cf, w, PaneLines::Single, flags).map(drop)),
            "no space for a new pane",
            "{flags:?}"
        );
        assert_eq!(g(&srv, f), (1, 1, 1, 1), "target unchanged");
    }
}

// --- work item 9: spread -------------------------------------------------------

#[test]
fn spread_cell_remainder() {
    for (rows, expect) in [(24, vec![4, 4, 4, 4, 4]), (26, vec![5, 5, 4, 4, 4])] {
        let mut srv = FakeServer::new();
        let (w, p0) = srv.window(80, 60);
        let mut panes = vec![p0];
        for _ in 0..4 {
            let last = *panes.last().unwrap();
            panes.push(split(&mut srv, last, TB, -1, SpawnFlags::default()));
        }
        resize(&mut srv, w, 80, rows);
        let root = srv.win(w).layout_root.unwrap();
        assert_eq!(srv.cells.get(root).unwrap().g.sy, rows);
        assert!(spread_cell(&mut srv, w, root));
        assert!(!spread_cell(&mut srv, w, root), "already even");
        let heights: Vec<u32> = panes.iter().map(|&p| g(&srv, p).3).collect();
        assert_eq!(heights, expect);
        spread_out(&mut srv, p0);
        fix_offsets(&mut srv, w);
        let offs: Vec<i32> = panes.iter().map(|&p| g(&srv, p).1).collect();
        assert_eq!(
            offs,
            expect
                .iter()
                .scan(0i32, |acc, &h| {
                    let o = *acc;
                    *acc += h as i32 + 1;
                    Some(o)
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn spread_cell_status_border_and_child_nodes() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    srv.win_mut(w).pane_status = PaneStatusPosition::Top;
    let p1 = split(&mut srv, p0, TB, 5, SpawnFlags::default());
    let p2 = split(&mut srv, p1, TB, 2, SpawnFlags::default());
    let root = srv.win(w).layout_root.unwrap();
    assert!(spread_cell(&mut srv, w, root));
    // 24 - 1 (outer status) = 23 rows, 2 borders: each 7, rem 0; top gets +1.
    assert_eq!((g(&srv, p0).3, g(&srv, p1).3, g(&srv, p2).3), (8, 7, 7));
    // A child node is left alone and a single tiled leaf returns unchanged.
    let p3 = split(&mut srv, p2, LR, -1, SpawnFlags::default());
    let node = srv
        .cells
        .get(srv.pane(p3).layout_cell.unwrap())
        .unwrap()
        .parent
        .unwrap();
    let c0 = srv.pane(p0).layout_cell.unwrap();
    set_size(&mut srv.cells, c0, 80, 3, 0, 0);
    let env = srv.layout_env(w);
    resize_adjust(&mut srv.cells, Some(&env), node, TB, 5);
    let node_g = srv.cells.get(node).unwrap().g;
    assert!(spread_cell(&mut srv, w, root));
    assert_eq!(srv.cells.get(node).unwrap().g, node_g);
    assert_eq!(
        (g(&srv, p0).3, g(&srv, p1).3),
        (12, 11),
        "two tiled leaves share 23 rows"
    );
    let (_, q0) = srv.window(80, 24);
    let q1 = split(&mut srv, q0, TB, -1, SpawnFlags::default());
    let _q2 = split(&mut srv, q1, LR, -1, SpawnFlags::default());
    let qw = srv.pane(q0).window;
    let qroot = srv.win(qw).layout_root.unwrap();
    assert!(!spread_cell(&mut srv, qw, qroot), "one direct tiled leaf");
}

// --- work item 10/11: dump and parse --------------------------------------------

#[test]
fn checksum_values() {
    assert_eq!(custom::checksum(b""), 0);
    // Reference values from regress/layout-custom.sh's awk helper.
    assert_eq!(custom::checksum(b"80x24,0,0"), awk_checksum("80x24,0,0"));
    assert_eq!(
        custom::checksum(b"80x24,0,0,0"),
        awk_checksum("80x24,0,0,0")
    );
    assert_eq!(
        custom::checksum(b"80x24,0,0{40x24,0,0,0,39x24,41,0,1}"),
        awk_checksum("80x24,0,0{40x24,0,0,0,39x24,41,0,1}")
    );
}

/// The separate implementation in `regress/layout-custom.sh:165-182`.
fn awk_checksum(s: &str) -> u16 {
    let mut csum: u32 = 0;
    for b in s.bytes() {
        csum = csum / 2 + (csum % 2) * 32768;
        csum = (csum + u32::from(b)) % 65536;
    }
    csum as u16
}

#[test]
fn dump_v2_regress_literals() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    assert_eq!(
        v2(&srv, w),
        "{\"V\":2,\"L\":{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"a\":true,\"i\":0,\"I\":\"%0\"}}"
    );
    let p1 = split(&mut srv, p0, TB, 12, SpawnFlags::default());
    assert_eq!(
        v2(&srv, w),
        "{\"V\":2,\"L\":{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{\"t\":\"p\",\"w\":80,\"h\":11,\"x\":0,\"y\":0,\"a\":true,\"i\":0,\"I\":\"%0\"},{\"t\":\"p\",\"w\":80,\"h\":12,\"x\":0,\"y\":12,\"i\":1,\"I\":\"%1\"}]}}"
    );
    srv.window_set_active_pane(w, p1, true);
    assert_eq!(
        v2(&srv, w),
        "{\"V\":2,\"L\":{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{\"t\":\"p\",\"w\":80,\"h\":11,\"x\":0,\"y\":0,\"l\":0,\"i\":0,\"I\":\"%0\"},{\"t\":\"p\",\"w\":80,\"h\":12,\"x\":0,\"y\":12,\"a\":true,\"i\":1,\"I\":\"%1\"}]}}"
    );
    srv.win_mut(w)
        .numbers
        .insert(b"pane-base-index".to_vec(), 1);
    assert!(v2(&srv, w).contains("\"i\":1,\"I\":\"%0\""));
    // A broken tree dumps as None.
    let root = srv.win(w).layout_root.unwrap();
    let c0 = srv.pane(p0).layout_cell.unwrap();
    srv.cells.get_mut(c0).unwrap().pane = None;
    assert_eq!(dump(&srv, w, Some(root), LayoutDumpFlags::default()), None);
    assert_eq!(dump(&srv, w, None, LayoutDumpFlags::default()), None);
    assert_eq!(
        dump(&srv, w, None, LayoutDumpFlags::OLD_FORMAT).unwrap(),
        "0000,"
    );
}

#[test]
fn dump_v1_drops_floats_and_collapses() {
    let mut srv = FakeServer::new();
    let (w, q0) = srv.window(80, 24);
    let q1 = split(&mut srv, q0, LR, -1, SpawnFlags::default());
    assert_eq!(
        v1(&srv, w),
        with_checksum(&format!(
            "80x24,0,0{{40x24,0,0,{},39x24,41,0,{}}}",
            srv.pane(q0).public_id,
            srv.pane(q1).public_id
        ))
    );
    let (w2, f0) = srv.window(80, 24);
    new_float(
        &mut srv,
        f0,
        &[(b'x', "20"), (b'y', "6"), (b'X', "8"), (b'Y', "3")],
        PaneLines::Single,
    )
    .unwrap();
    let json_before = v2(&srv, w2);
    assert_eq!(
        v1(&srv, w2),
        with_checksum(&format!("80x24,0,0,{}", srv.pane(f0).public_id))
    );
    assert_eq!(
        v2(&srv, w2),
        json_before,
        "the window is untouched by a v1 dump"
    );
    let (w3, m0) = srv.window(80, 24);
    let m1 = split(&mut srv, m0, TB, 12, SpawnFlags::default());
    new_float(
        &mut srv,
        m0,
        &[(b'x', "20"), (b'y', "6"), (b'X', "8"), (b'Y', "3")],
        PaneLines::Single,
    )
    .unwrap();
    assert_eq!(
        v1(&srv, w3),
        with_checksum(&format!(
            "80x24,0,0[80x11,0,0,{},80x12,0,12,{}]",
            srv.pane(m0).public_id,
            srv.pane(m1).public_id
        ))
    );
    // Two floats and no tiled pane: an empty body, server alive.
    let (w4, h0) = srv.window(80, 24);
    new_float(
        &mut srv,
        h0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    new_float(
        &mut srv,
        h0,
        &[(b'x', "30"), (b'y', "8")],
        PaneLines::Single,
    )
    .unwrap();
    kill(&mut srv, h0);
    assert_eq!(srv.win(w4).panes.len(), 2);
    assert_eq!(v1(&srv, w4), "0000,");
}

#[test]
fn zindex_in_dump_follows_z_order() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let f1 = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    let f2 = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    let out = v2(&srv, w);
    assert!(out.contains(&format!(
        "\"i\":1,\"z\":0,\"I\":\"%{}\"",
        srv.pane(f1).public_id
    )));
    assert!(out.contains(&format!(
        "\"i\":2,\"z\":1,\"I\":\"%{}\"",
        srv.pane(f2).public_id
    )));
    srv.win_mut(w).z_index = vec![f2, p0, f1];
    let out = v2(&srv, w);
    assert!(out.contains(&format!(
        "\"i\":1,\"z\":1,\"I\":\"%{}\"",
        srv.pane(f1).public_id
    )));
    assert!(out.contains(&format!(
        "\"i\":2,\"z\":0,\"I\":\"%{}\"",
        srv.pane(f2).public_id
    )));
}

#[test]
fn parse_v1_header_and_scanner_quirks() {
    let mut srv = FakeServer::new();
    let (w, _p0) = srv.window(80, 24);
    let body = "80x24,0,0";
    let ok = with_checksum(body);
    parse(&mut srv, w, ok.as_bytes()).unwrap();
    assert_eq!(
        srv.events_named("window-layout-changed"),
        1,
        "v1 fires once"
    );
    // Leading whitespace is skipped; a NUL ends the input.
    parse(&mut srv, w, format!("  \t{ok}\0garbage").as_bytes()).unwrap();
    // Header: plain, signed and 0x forms with five bytes consumed. The pane
    // id is skipped on parse, so it can be chosen to make a three-digit
    // checksum for the signed forms; the rotate-and-add sum never gets below
    // 0x100 for a cell body, so the 0x form is checked on the scanner alone.
    let csum = custom::checksum(body.as_bytes());
    let short = (0..200000u32)
        .map(|id| format!("80x24,0,0,{id}"))
        .find(|b| custom::checksum(b.as_bytes()) < 0x1000)
        .unwrap();
    let small = custom::checksum(short.as_bytes());
    parse(&mut srv, w, format!("+{small:03x},{short}").as_bytes()).unwrap();
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            format!("-{small:03x},{short}").as_bytes()
        )),
        "invalid layout checksum"
    );
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            format!("0x{small:03x},{short}").as_bytes()
        )),
        "malformed layout header"
    );
    assert_eq!(custom::v1_header(b"0x01,"), Some(1));
    assert_eq!(custom::v1_header(b"0X0f,"), Some(15));
    assert_eq!(custom::v1_header(b"0x1,"), None);
    assert_eq!(custom::v1_header(b"0x,"), None, "no hex digit after 0x");
    assert_eq!(custom::v1_header(b"+001,"), Some(1));
    assert_eq!(custom::v1_header(b"-001,"), Some(0xffff));
    assert_eq!(custom::v1_header(b"ffff,"), Some(0xffff));
    assert_eq!(
        custom::v1_header(b" 001,"),
        None,
        "whitespace was already skipped"
    );
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            format!("{small:03x},{short}").as_bytes()
        )),
        "malformed layout header"
    );
    assert_eq!(
        cause(parse(&mut srv, w, format!("{csum:05x},{body}").as_bytes())),
        "malformed layout header"
    );
    assert_eq!(
        cause(parse(&mut srv, w, format!("zzzz,{body}").as_bytes())),
        "malformed layout header"
    );
    assert_eq!(cause(parse(&mut srv, w, b"")), "malformed layout header");
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            format!("{:04x},{body}", csum ^ 1).as_bytes()
        )),
        "invalid layout checksum"
    );
    assert_eq!(
        cause(parse(&mut srv, w, with_checksum("80x24,0,0x").as_bytes())),
        "invalid layout"
    );
    assert_eq!(
        cause(parse(&mut srv, w, with_checksum("80x24,0,0}").as_bytes())),
        "trailing data"
    );
    assert_eq!(
        cause(parse(&mut srv, w, with_checksum("80x24,0").as_bytes())),
        "invalid layout"
    );
    assert_eq!(
        cause(parse(&mut srv, w, with_checksum("80x24,-1,0").as_bytes())),
        "invalid layout"
    );
    assert_eq!(
        cause(parse(&mut srv, w, with_checksum("0x24,0,0").as_bytes())),
        "invalid layout"
    );
    assert_eq!(
        cause(parse(&mut srv, w, with_checksum("10001x24,0,0").as_bytes())),
        "invalid layout"
    );
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            with_checksum("80x24,0,0{40x24,0,0,39x24,41,0]").as_bytes()
        )),
        "invalid layout"
    );
    // A one-child node is kept; the root is repaired to its child plus the
    // border and the window follows (layout-custom.c:654-680, 689-690).
    parse(
        &mut srv,
        w,
        with_checksum("80x24,0,0{40x24,0,0}").as_bytes(),
    )
    .unwrap();
    assert_eq!(srv.cells.len(), 2);
    assert_eq!((srv.win(w).sx, srv.win(w).sy), (40, 24));
    assert_eq!(pane_geom(&srv, _p0), (0, 0, 40, 24));
    parse(&mut srv, w, ok.as_bytes()).unwrap();
    assert_eq!((srv.win(w).sx, srv.win(w).sy), (80, 24));
    // Six digits in a middle field fail; the last field keeps the first five,
    // so 100000 is yoff 10000 (accepted) while 10001 is rejected.
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            with_checksum("80x24,100000,0").as_bytes()
        )),
        "invalid layout"
    );
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            with_checksum("80x24,0,10001").as_bytes()
        )),
        "invalid layout"
    );
    parse(&mut srv, w, with_checksum("80x24,0,100000").as_bytes()).unwrap();
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(
        srv.cells.get(root).unwrap().g,
        LayoutGeometry::new(80, 24, 0, 0),
        "failures leave the window unchanged"
    );
    // Pane ids are skipped, including an empty one; a comma before a cell is
    // the separator.
    let p1 = split(&mut srv, _p0, TB, 12, SpawnFlags::default());
    parse(
        &mut srv,
        w,
        with_checksum("80x24,0,0[80x7,0,0,99,80x16,0,8,]").as_bytes(),
    )
    .unwrap();
    assert_eq!(g(&srv, _p0), (0, 0, 80, 7));
    assert_eq!(g(&srv, p1), (0, 8, 80, 16));
    parse(
        &mut srv,
        w,
        with_checksum("80x24,0,0[80x11,0,0,80x12,0,12]").as_bytes(),
    )
    .unwrap();
    assert_eq!(g(&srv, p1), (0, 12, 80, 12));
}

#[test]
fn parse_v1_one_child_nodes_and_depth() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    // One-child nodes are retained (regress/layout-nesting-limit.sh).
    let mut body = String::new();
    for _ in 0..1000 {
        body.push_str("80x24,0,0[");
    }
    body.push_str("80x24,0,0");
    body.push_str(&"]".repeat(1000));
    parse(&mut srv, w, with_checksum(&body).as_bytes()).unwrap();
    assert_eq!(srv.cells.len(), 1001);
    tree_ok(&srv, w);
    let c0 = srv.pane(p0).layout_cell.unwrap();
    assert_eq!(
        srv.cells.get(c0).unwrap().g,
        LayoutGeometry::new(80, 24, 0, 0)
    );
    let mut deeper = String::new();
    for _ in 0..1001 {
        deeper.push_str("80x24,0,0[");
    }
    deeper.push_str("80x24,0,0");
    deeper.push_str(&"]".repeat(1001));
    assert_eq!(
        cause(parse(&mut srv, w, with_checksum(&deeper).as_bytes())),
        "invalid layout"
    );
    assert_eq!(srv.cells.len(), 1001, "unchanged, and nothing leaked");
}

#[test]
fn parse_v1_keeps_floats() {
    let mut srv = FakeServer::new();
    let (w, m0) = srv.window(80, 24);
    let m1 = split(&mut srv, m0, TB, 12, SpawnFlags::default());
    let mf = new_float(
        &mut srv,
        m0,
        &[(b'x', "20"), (b'y', "6"), (b'X', "8"), (b'Y', "3")],
        PaneLines::Single,
    )
    .unwrap();
    let before = g(&srv, mf);
    parse(
        &mut srv,
        w,
        with_checksum("80x24,0,0[80x7,0,0,80x16,0,8]").as_bytes(),
    )
    .unwrap();
    assert_eq!(g(&srv, m0), (0, 0, 80, 7));
    assert_eq!(g(&srv, m1), (0, 8, 80, 16));
    assert_eq!(g(&srv, mf), before);
    assert_eq!(srv.cells.len(), 4);
    tree_ok(&srv, w);
    // A single tiled cell plus floats: the root leaf is wrapped.
    kill(&mut srv, m1);
    parse(&mut srv, w, with_checksum("80x24,0,0").as_bytes()).unwrap();
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(srv.cells.get(root).unwrap().kind, TB);
    assert_eq!(count_cells(&srv.cells, root, true), 2);
    assert_eq!(g(&srv, mf), before);
    tree_ok(&srv, w);
}

#[test]
fn parse_v2_error_causes() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let leaf =
        |extra: &str| format!("{{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"i\":0{extra}}}");
    let wrap = |cell: &str| format!("{{\"V\":2,\"L\":{cell}}}");
    let fails = |srv: &mut FakeServer, input: String| cause(parse(srv, w, input.as_bytes()));
    assert_eq!(
        fails(&mut srv, "{".into()),
        fails(&mut srv, "{".into()),
        "json error is stable"
    );
    assert_eq!(fails(&mut srv, "{\"L\":{}}".into()), "key \"V\" not found");
    assert_eq!(fails(&mut srv, "{\"V\":2}".into()), "key \"L\" not found");
    assert_eq!(
        fails(&mut srv, "{\"V\":\"2\",\"L\":{}}".into()),
        "key \"V\" expected a number"
    );
    assert_eq!(
        fails(&mut srv, "{\"V\":2,\"L\":1}".into()),
        "key \"L\" expected an object"
    );
    assert_eq!(
        fails(&mut srv, format!("{{\"V\":1,\"L\":{}}}", leaf(""))),
        "version mismatch"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"q\",\"w\":80,\"h\":24,\"x\":0,\"y\":0}")
        ),
        "unknown cell type \"q\""
    );
    assert_eq!(
        fails(&mut srv, wrap("{\"t\":\"p\"}")),
        "key \"w\" not found"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"p\",\"w\":0,\"h\":24,\"x\":0,\"y\":0}")
        ),
        "invalid width 0"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"p\",\"w\":80,\"h\":10001,\"x\":0,\"y\":0}")
        ),
        "invalid height 10001"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":-10001,\"y\":0}")
        ),
        "invalid x-offset -10001"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":10001}")
        ),
        "invalid y-offset 10001"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[]}")
        ),
        "panes cannot have children"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0}")
        ),
        "key \"i\" not found"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"i\":-1}")
        ),
        "invalid index -1"
    );
    assert_eq!(
        fails(&mut srv, wrap(&leaf(",\"a\":1"))),
        "key \"a\" expected a boolean"
    );
    assert_eq!(fails(&mut srv, wrap(&leaf(",\"l\":-2"))), "invalid last -2");
    assert_eq!(
        fails(&mut srv, wrap(&leaf(",\"z\":2147483647"))),
        "invalid floating zindex 2147483647"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap("{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0}")
        ),
        "key \"c\" not found"
    );
    assert_eq!(
        fails(
            &mut srv,
            wrap(&format!(
                "{{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{}]}}",
                leaf("")
            ))
        ),
        "nodes must have more than one child"
    );
    // A malformed cell wins over a wrong version.
    assert_eq!(
        fails(
            &mut srv,
            "{\"V\":1,\"L\":{\"t\":\"p\",\"w\":0,\"h\":24,\"x\":0,\"y\":0}}".into()
        ),
        "invalid width 0"
    );
    let two = |a: &str, b: &str| {
        wrap(&format!(
            "{{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{a},{b}]}}"
        ))
    };
    let cell = |y: i32, h: u32, extra: &str| {
        format!("{{\"t\":\"p\",\"w\":80,\"h\":{h},\"x\":0,\"y\":{y}{extra}}}")
    };
    assert_eq!(
        fails(
            &mut srv,
            two(
                &cell(0, 11, ",\"i\":0,\"a\":true"),
                &cell(12, 12, ",\"i\":1,\"a\":true")
            )
        ),
        "more than one active pane"
    );
    assert_eq!(
        fails(
            &mut srv,
            two(&cell(0, 11, ",\"i\":0"), &cell(12, 12, ",\"i\":0"))
        ),
        "duplicate pane index"
    );
    assert_eq!(
        fails(
            &mut srv,
            two(
                &cell(0, 11, ",\"i\":0,\"z\":0"),
                &cell(12, 12, ",\"i\":1,\"z\":0")
            )
        ),
        "duplicate pane z-index"
    );
    assert_eq!(
        fails(
            &mut srv,
            two(
                &cell(0, 11, ",\"i\":0,\"l\":0"),
                &cell(12, 12, ",\"i\":1,\"l\":0")
            )
        ),
        "duplicate last pane index"
    );
    // More cells than panes trims the bottom-right cell instead of failing.
    parse(
        &mut srv,
        w,
        two(&cell(0, 11, ",\"i\":0"), &cell(12, 12, ",\"i\":1")).as_bytes(),
    )
    .unwrap();
    assert_eq!(
        g(&srv, p0),
        (0, 0, 80, 24),
        "the neighbour takes the trimmed space"
    );
    assert_eq!(srv.cells.len(), 1, "the node collapsed into the leaf");
    parse(&mut srv, w, wrap(&leaf("")).as_bytes()).unwrap();
    assert_eq!((srv.win(w).sx, srv.win(w).sy), (80, 24));
    let p1 = split(&mut srv, p0, TB, 12, SpawnFlags::default());
    // A root that only disagrees with its children is repaired; a nested
    // mismatch is rejected.
    parse(
        &mut srv,
        w,
        two(&cell(0, 11, ",\"i\":0"), &cell(12, 13, ",\"i\":1")).as_bytes(),
    )
    .unwrap();
    assert_eq!(g(&srv, p1), (0, 12, 80, 13));
    parse(
        &mut srv,
        w,
        two(&cell(0, 11, ",\"i\":0"), &cell(12, 12, ",\"i\":1")).as_bytes(),
    )
    .unwrap();
    let narrow = "{\"t\":\"p\",\"w\":70,\"h\":12,\"x\":0,\"y\":12,\"i\":1}".to_string();
    assert_eq!(
        fails(&mut srv, two(&cell(0, 11, ",\"i\":0"), &narrow)),
        "size mismatch after applying layout"
    );
    let lr = wrap(
        "{\"t\":\"h\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{\"t\":\"p\",\"w\":40,\"h\":24,\"x\":0,\"y\":0,\"i\":0},{\"t\":\"p\",\"w\":39,\"h\":20,\"x\":41,\"y\":0,\"i\":1}]}",
    );
    assert_eq!(fails(&mut srv, lr), "size mismatch after applying layout");
    assert_eq!(g(&srv, p1), (0, 12, 80, 12), "window unchanged on failure");
    assert_eq!(srv.cells.len(), 3, "no leaked cells");
    // A floating cell is trimmed from the bottom right when there are more
    // cells than panes, including its context.
    let three = wrap(&format!(
        "{{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[{},{},{}]}}",
        cell(0, 11, ",\"i\":0"),
        cell(12, 12, ",\"i\":1"),
        "{\"t\":\"p\",\"w\":10,\"h\":5,\"x\":2,\"y\":2,\"i\":2,\"z\":0}"
    ));
    let recalculated = srv.recalculated;
    parse(&mut srv, w, three.as_bytes()).unwrap();
    assert_eq!(srv.cells.len(), 3);
    assert_eq!(
        count_cells(&srv.cells, srv.win(w).layout_root.unwrap(), true),
        2
    );
    assert!(srv.events.is_empty(), "v2 fires nothing itself");
    assert_eq!(srv.recalculated, recalculated + 1);
}

#[test]
fn parse_v2_applies_active_last_and_z_order() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, TB, 12, SpawnFlags::default());
    let p2 = split(&mut srv, p1, LR, -1, SpawnFlags::default());
    let f = new_float(
        &mut srv,
        p0,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    // Arbitrary unique indexes assign in index order; p2 becomes the float
    // (z) and f becomes tiled; p1 active, p0 last 1, f last 0.
    let input = "{\"V\":2,\"L\":{\"t\":\"v\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"c\":[\
        {\"t\":\"p\",\"w\":80,\"h\":5,\"x\":0,\"y\":0,\"i\":7,\"l\":1},\
        {\"t\":\"p\",\"w\":80,\"h\":18,\"x\":0,\"y\":6,\"i\":9,\"a\":true},\
        {\"t\":\"p\",\"w\":12,\"h\":3,\"x\":5,\"y\":5,\"i\":10,\"z\":3,\"a\":false,\"l\":-5},\
        {\"t\":\"p\",\"w\":30,\"h\":4,\"x\":1,\"y\":1,\"i\":1000,\"z\":1,\"l\":0}]}}";
    parse(&mut srv, w, input.as_bytes()).unwrap();
    assert_eq!(g(&srv, p0), (0, 0, 80, 5));
    assert_eq!(g(&srv, p1), (0, 6, 80, 18));
    assert_eq!(g(&srv, p2), (5, 5, 12, 3));
    assert!(srv.pane_is_floating(p2));
    assert_eq!(g(&srv, f), (1, 1, 30, 4));
    assert!(srv.pane_is_floating(f));
    assert_eq!(srv.win(w).active, Some(p1));
    assert_eq!(srv.active_changes, vec![(w, p1, true)]);
    assert_eq!(srv.win(w).last_panes, vec![f, p0]);
    // Now-tiled panes keep their z order; floats go in front, lowest z first.
    assert_eq!(srv.win(w).z_index, vec![f, p2, p0, p1]);
    assert_eq!(srv.win(w).sx, 80);
    tree_ok(&srv, w);
    // Without a:true the active pane stays; a:false ignores an invalid l.
    let input2 = input.replace("\"i\":9,\"a\":true", "\"i\":9");
    srv.active_changes.clear();
    parse(&mut srv, w, input2.as_bytes()).unwrap();
    assert_eq!(srv.win(w).active, Some(p1));
    assert!(srv.active_changes.is_empty());
    assert_eq!(srv.win(w).last_panes, vec![f, p0]);
}

#[test]
fn parse_v2_root_repair_and_window_resize() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 24);
    let p1 = split(&mut srv, p0, TB, 12, SpawnFlags::default());
    // Root 100x50 with children filling 80x24: repaired, not rejected.
    let input = "{\"V\":2,\"L\":{\"t\":\"v\",\"w\":100,\"h\":50,\"x\":0,\"y\":0,\"c\":[\
        {\"t\":\"p\",\"w\":80,\"h\":11,\"x\":0,\"y\":0,\"i\":0},\
        {\"t\":\"p\",\"w\":80,\"h\":12,\"x\":0,\"y\":12,\"i\":1}]}}";
    parse(&mut srv, w, input.as_bytes()).unwrap();
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(
        srv.cells.get(root).unwrap().g,
        LayoutGeometry::new(80, 24, 0, 0)
    );
    // A bigger layout resizes the window.
    let input = "{\"V\":2,\"L\":{\"t\":\"h\",\"w\":100,\"h\":30,\"x\":0,\"y\":0,\"c\":[\
        {\"t\":\"p\",\"w\":50,\"h\":30,\"x\":0,\"y\":0,\"i\":0},\
        {\"t\":\"p\",\"w\":49,\"h\":30,\"x\":51,\"y\":0,\"i\":1}]}}";
    parse(&mut srv, w, input.as_bytes()).unwrap();
    assert_eq!((srv.win(w).sx, srv.win(w).sy), (100, 30));
    assert_eq!(pane_geom(&srv, p1), (51, 0, 49, 30));
    assert_eq!(
        cause(parse(
            &mut srv,
            w,
            b"{\"V\":2,\"L\":{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"i\":0}}"
        )),
        "have 2 panes but need 1"
    );
    let (w2, _) = srv.window(80, 24);
    srv.win_mut(w2).panes.clear();
    assert_eq!(
        cause(parse(
            &mut srv,
            w2,
            b"{\"V\":2,\"L\":{\"t\":\"p\",\"w\":80,\"h\":24,\"x\":0,\"y\":0,\"i\":0}}"
        )),
        format!("window @{} has no panes", srv.win(w2).public_id)
    );
}

#[test]
fn dump_parse_round_trip() {
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(100, 40);
    let p1 = split(&mut srv, p0, LR, 30, SpawnFlags::default());
    let p2 = split(&mut srv, p1, TB, -1, SpawnFlags::BEFORE);
    let p3 = split(&mut srv, p0, TB, 7, SpawnFlags::FULLSIZE);
    let f = new_float(
        &mut srv,
        p2,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    srv.window_set_active_pane(w, p2, true);
    srv.window_set_active_pane(w, p3, true);
    let before: Vec<_> = [p0, p1, p2, p3, f].iter().map(|&p| g(&srv, p)).collect();
    let json = v2(&srv, w);
    let legacy = v1(&srv, w);
    // Scramble, then restore from v2: geometry, floats, active and last.
    set_select(&mut srv, w, 6);
    parse(&mut srv, w, json.as_bytes()).unwrap();
    let after: Vec<_> = [p0, p1, p2, p3, f].iter().map(|&p| g(&srv, p)).collect();
    assert_eq!(after, before);
    assert_eq!(v2(&srv, w), json);
    assert_eq!(srv.win(w).active, Some(p3));
    assert_eq!(srv.win(w).last_panes, vec![p2, p0]);
    // v1 restores the tiled geometry only and keeps the float as it is.
    set_select(&mut srv, w, 0);
    parse(&mut srv, w, legacy.as_bytes()).unwrap();
    // v1 names no panes: tiled cells take panes in tree order, so p1 and p2
    // (split before) swap geometry.
    let after: Vec<_> = [p0, p2, p1, p3, f].iter().map(|&p| g(&srv, p)).collect();
    assert_eq!(after, before);
    // The v1 dump now carries the swapped pane ids; the geometry is the same.
    assert_eq!(
        v1(&srv, w).split(',').filter(|s| s.contains('x')).count(),
        7
    );
    tree_ok(&srv, w);
}

// --- work item 12: presets -----------------------------------------------------

#[test]
fn set_lookup_rules() {
    assert_eq!(set_lookup(b"even-h"), Some(LayoutSetIndex(0)));
    assert_eq!(set_lookup(b"main"), None);
    assert_eq!(set_lookup(b"tiled"), Some(LayoutSetIndex(6)));
    assert_eq!(set_lookup(b"x"), None);
    assert_eq!(
        set_lookup(b"main-vertical"),
        Some(LayoutSetIndex(4)),
        "exact beats the mirrored prefix"
    );
    assert_eq!(set_lookup(b"main-vertical-"), Some(LayoutSetIndex(5)));
    assert_eq!(set_lookup(b""), None, "every name has the empty prefix");
}

#[test]
fn tiled_grid_counts() {
    let expect0 = [
        (1, 1),
        (2, 1),
        (2, 2),
        (2, 2),
        (3, 2),
        (3, 2),
        (3, 3),
        (3, 3),
        (3, 3),
        (4, 3),
        (4, 3),
        (4, 3),
    ];
    for (n, e) in (1..=12).zip(expect0) {
        assert_eq!(set::tiled_grid(n, 0), e, "n={n}");
    }
    for n in 2..=12u32 {
        let (rows, cols) = set::tiled_grid(n, 2);
        assert!(cols <= 2 && rows * cols >= n, "n={n} {rows}x{cols}");
        let (rows, cols) = set::tiled_grid(n, 3);
        assert!(cols <= 3 && rows * cols >= n, "n={n} {rows}x{cols}");
    }
    assert_eq!(set::tiled_grid(5, 2), (3, 2));
    assert_eq!(set::tiled_grid(12, 3), (4, 3));
    assert_eq!(set::tiled_grid(7, 3), (3, 3));
}

fn five(srv: &mut FakeServer, sx: u32, sy: u32) -> (WindowId, Vec<PaneId>) {
    let (w, p0) = srv.window(sx, sy);
    let mut panes = vec![p0];
    for _ in 0..4 {
        let last = *panes.last().unwrap();
        panes.push(split(srv, last, TB, -1, SpawnFlags::default()));
    }
    (w, panes)
}

#[test]
fn presets_select_next_previous() {
    let mut srv = FakeServer::new();
    let (w, panes) = five(&mut srv, 100, 40);
    assert_eq!(set_select(&mut srv, w, 99), LayoutSetIndex(6), "clamped");
    assert_eq!(srv.win(w).lastlayout, Some(LayoutSetIndex(6)));
    assert_eq!(set_next(&mut srv, w), LayoutSetIndex(0));
    assert_eq!(set_previous(&mut srv, w), LayoutSetIndex(6));
    srv.win_mut(w).lastlayout = None;
    assert_eq!(set_next(&mut srv, w), LayoutSetIndex(0));
    srv.win_mut(w).lastlayout = None;
    assert_eq!(set_previous(&mut srv, w), LayoutSetIndex(6));
    // even-horizontal at 100 wide: 5 panes, 4 borders: 96/5 = 19 rem 1.
    set_select(&mut srv, w, 0);
    let widths: Vec<u32> = panes.iter().map(|&p| g(&srv, p).2).collect();
    assert_eq!(widths, vec![20, 19, 19, 19, 19]);
    assert_eq!(srv.events_named("window-layout-changed"), 6);
    assert_eq!(srv.redraws.len(), 6);
    tree_ok(&srv, w);
    // Only the nodes of the old tree are freed: 5 leaves plus the new root.
    assert_eq!(srv.cells.len(), 6);
    // main-horizontal: main 24 rows, others 40-1-24 = 15 rows across.
    set_select(&mut srv, w, 2);
    assert_eq!(g(&srv, panes[0]), (0, 0, 100, 24));
    assert_eq!(g(&srv, panes[1]), (0, 25, 25, 15));
    assert_eq!(g(&srv, panes[4]), (26 + 25 + 25, 25, 24, 15));
    // mirrored puts the main pane at the bottom.
    set_select(&mut srv, w, 3);
    assert_eq!(g(&srv, panes[0]), (0, 16, 100, 24));
    assert_eq!(g(&srv, panes[1]), (0, 0, 25, 15));
    // main-vertical: main 80 of 99, others 19 wide stacked.
    set_select(&mut srv, w, 4);
    assert_eq!(g(&srv, panes[0]), (0, 0, 80, 40));
    assert_eq!(g(&srv, panes[1]), (81, 0, 19, 10));
    set_select(&mut srv, w, 5);
    assert_eq!(g(&srv, panes[0]), (20, 0, 80, 40));
    assert_eq!(g(&srv, panes[4]), (0, 31, 19, 9));
    // tiled: 5 panes -> 3 rows x 2 columns; the last row is one pane wide.
    set_select(&mut srv, w, 6);
    assert_eq!(g(&srv, panes[0]), (0, 0, 49, 12));
    assert_eq!(g(&srv, panes[1]), (50, 0, 50, 12));
    assert_eq!(g(&srv, panes[4]), (0, 26, 100, 14));
    tree_ok(&srv, w);
    // One pane: every arranger returns at once.
    let (w1, _) = srv.window(80, 24);
    srv.events.clear();
    for i in 0..7 {
        set_select(&mut srv, w1, i);
    }
    assert!(srv.events.is_empty());
}

#[test]
fn presets_small_window_and_options() {
    let mut srv = FakeServer::new();
    let (w, panes) = five(&mut srv, 10, 60);
    srv.win_mut(w).sy = 6;
    set_select(&mut srv, w, 2);
    // sy = 5; mainh 24 clamps to sy - 1 = 4, otherh 1; root 10 x 6.
    assert_eq!(g(&srv, panes[0]), (0, 0, 10, 4));
    assert_eq!(g(&srv, panes[1]), (0, 5, 2, 1));
    assert_eq!((srv.win(w).sx, srv.win(w).sy), (10, 6));
    set_select(&mut srv, w, 1);
    // even-vertical: 5 panes need 9 rows; the window grows.
    assert_eq!((srv.win(w).sx, srv.win(w).sy), (10, 9));
    // other-pane-height sets mainh = sy - otherh.
    let (w2, q) = five(&mut srv, 100, 40);
    srv.win_mut(w2)
        .strings
        .insert(b"other-pane-height".to_vec(), ByteString::from("10"));
    set_select(&mut srv, w2, 2);
    assert_eq!(g(&srv, q[0]).3, 29);
    assert_eq!(g(&srv, q[1]).3, 10);
    srv.win_mut(w2)
        .strings
        .insert(b"main-pane-height".to_vec(), ByteString::from("bad"));
    srv.win_mut(w2)
        .strings
        .insert(b"other-pane-height".to_vec(), ByteString::from("30%"));
    set_select(&mut srv, w2, 2);
    assert_eq!(g(&srv, q[0]).3, 39 - 11);
    srv.win_mut(w2)
        .strings
        .insert(b"other-pane-width".to_vec(), ByteString::from("50"));
    set_select(&mut srv, w2, 4);
    assert_eq!(
        (g(&srv, q[0]).2, g(&srv, q[1]).2),
        (80, 19),
        "a too-wide other width falls back"
    );
    srv.win_mut(w2)
        .numbers
        .insert(b"tiled-layout-max-columns".to_vec(), 2);
    set_select(&mut srv, w2, 6);
    assert_eq!(g(&srv, q[4]), (0, 26, 100, 14));
    srv.win_mut(w2)
        .numbers
        .insert(b"tiled-layout-max-columns".to_vec(), 1);
    set_select(&mut srv, w2, 6);
    assert_eq!(g(&srv, q[1]), (0, 8, 100, 7));
    tree_ok(&srv, w2);
}

#[test]
fn presets_keep_floats_under_root() {
    let mut srv = FakeServer::new();
    let (w, panes) = five(&mut srv, 100, 40);
    let f = new_float(
        &mut srv,
        panes[2],
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    for i in 0..7 {
        set_select(&mut srv, w, i);
        assert_eq!(g(&srv, f), (4, 2, 18, 4), "layout {i}");
        // The even and tiled layouts link floats under the root; the main
        // layouts with several others put them in the others node
        // (layout-set.c:287-296).
        let root = srv.win(w).layout_root.unwrap();
        let parent = srv
            .cells
            .get(srv.pane(f).layout_cell.unwrap())
            .unwrap()
            .parent
            .unwrap();
        if (2..=5).contains(&i) {
            assert_eq!(
                srv.cells.get(parent).unwrap().parent,
                Some(root),
                "layout {i}"
            );
            assert!(!srv.cells.get(parent).unwrap().is_leaf());
        } else {
            assert_eq!(parent, root, "layout {i}");
        }
        tree_ok(&srv, w);
    }
    // main-horizontal with one other tiled pane skips the float.
    kill(&mut srv, panes[4]);
    kill(&mut srv, panes[3]);
    kill(&mut srv, panes[2]);
    set_select(&mut srv, w, 2);
    assert_eq!(g(&srv, panes[1]), (0, 25, 100, 15));
    let root = srv.win(w).layout_root.unwrap();
    assert_eq!(srv.cells.get(root).unwrap().children.len(), 3);
}

#[test]
fn tiled_preset_separators_wider_than_window() {
    // 2 columns in a 1-wide window: `(1 - 1) / 2 = 0` -> PANE_MINIMUM; 3 rows
    // in a 1-high window: `1 - 2` wraps, so height is huge and the root grows.
    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(80, 60);
    let p1 = split(&mut srv, p0, TB, -1, SpawnFlags::default());
    let p2 = split(&mut srv, p1, TB, -1, SpawnFlags::default());
    let p3 = split(&mut srv, p2, TB, -1, SpawnFlags::default());
    let p4 = split(&mut srv, p3, TB, -1, SpawnFlags::default());
    srv.win_mut(w).sx = 1;
    srv.win_mut(w).sy = 1;
    set_select(&mut srv, w, 6);
    let root = srv.win(w).layout_root.unwrap();
    let rg = srv.cells.get(root).unwrap().g;
    let height = (1u32.wrapping_sub(2)) / 3;
    assert_eq!(rg.sx, 3);
    assert_eq!(
        rg.sy,
        height.wrapping_add(1).wrapping_mul(3).wrapping_sub(1)
    );
    assert_eq!(g(&srv, p0).2, 1);
    assert_eq!(g(&srv, p4).3, height);
    tree_ok(&srv, w);
}

// --- work item 14: parser fuzz -----------------------------------------------

/// Mutate valid v1 and v2 layout strings at random and feed them to `parse`:
/// no panic (overflow checks are on in test builds), no leaked cells, and a
/// failed parse leaves the window unchanged. `RMUX_LAYOUT_FUZZ_SECONDS`
/// extends the default two-second budget for the ten-minute acceptance run;
/// `RMUX_LAYOUT_FUZZ_SEED` picks the seed.
#[test]
fn parse_fuzz_never_panics() {
    let seconds: u64 = std::env::var("RMUX_LAYOUT_FUZZ_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let mut state: u64 = std::env::var("RMUX_LAYOUT_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x2545_f491_4f6c_dd1d);
    let mut next = move |n: usize| -> usize {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };

    let mut srv = FakeServer::new();
    let (w, p0) = srv.window(100, 40);
    let p1 = split(&mut srv, p0, LR, 30, SpawnFlags::default());
    let _p2 = split(&mut srv, p1, TB, -1, SpawnFlags::BEFORE);
    let _p3 = split(&mut srv, p0, TB, 7, SpawnFlags::FULLSIZE);
    let _f = new_float(
        &mut srv,
        p1,
        &[(b'x', "20"), (b'y', "6")],
        PaneLines::Single,
    )
    .unwrap();
    let seeds = [v2(&srv, w).into_bytes(), v1(&srv, w).into_bytes()];
    let alphabet = b"0123456789{}[],:\"xVLtwhypiaIzl-+ %\\tr";

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut rounds = 0u64;
    let mut accepted = 0u64;
    while std::time::Instant::now() < deadline {
        rounds += 1;
        let mut input = seeds[next(2)].clone();
        for _ in 0..1 + next(6) {
            if input.is_empty() {
                input.push(alphabet[next(alphabet.len())]);
                continue;
            }
            let at = next(input.len());
            match next(5) {
                0 => input[at] = alphabet[next(alphabet.len())],
                1 => input.insert(at, alphabet[next(alphabet.len())]),
                2 => {
                    input.remove(at);
                }
                3 => {
                    let digits = format!(
                        "{}",
                        [
                            0i64,
                            1,
                            9999,
                            10000,
                            10001,
                            -1,
                            -10000,
                            -10001,
                            i64::MAX,
                            i64::MIN
                        ][next(10)]
                    );
                    input.splice(at..at, digits.bytes());
                }
                _ => input.truncate(at),
            }
        }
        let before = v2(&srv, w);
        let cells = srv.cells.len();
        match parse(&mut srv, w, &input) {
            Ok(()) => {
                accepted += 1;
                tree_ok(&srv, w);
                // Restore the fixture so later rounds start from a known tree.
                parse(&mut srv, w, &seeds[0]).unwrap();
            }
            Err(_) => {
                assert_eq!(
                    v2(&srv, w),
                    before,
                    "failed parse changed the window: {input:?}"
                );
                assert_eq!(
                    srv.cells.len(),
                    cells,
                    "failed parse leaked cells: {input:?}"
                );
            }
        }
    }
    assert!(rounds > 100, "fuzz loop ran {rounds} rounds");
    eprintln!("layout parse fuzz: {rounds} rounds, {accepted} accepted");
}
