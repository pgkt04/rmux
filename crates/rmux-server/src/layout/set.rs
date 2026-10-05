// Ported from tmux layout-set.c @ 8f25579c
/*
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

//! Set window layouts - predefined methods to arrange windows. These are
//! one-off and generate a layout tree.

use rmux_util::bytes::cstr;

use super::tree::{
    cell, cell_is_tiled, cell_mut, create_cell, debug_print, fix_offsets, fix_panes, free,
    link_child, link_child_tail, make_node, resize_adjust, set_size, spread_cell,
};
use super::{LayoutHost, LayoutSetIndex, LayoutType, PANE_MINIMUM};
use crate::cmd::arguments::string_percentage;
use crate::ids::{LayoutCellId, PaneId, WindowId};

type Arrange<H> = fn(&mut H, WindowId);

/// `layout_sets` (`layout-set.c:39-50`), in table order.
pub const LAYOUT_SET_NAMES: [&[u8]; 7] = [
    b"even-horizontal",
    b"even-vertical",
    b"main-horizontal",
    b"main-horizontal-mirrored",
    b"main-vertical",
    b"main-vertical-mirrored",
    b"tiled",
];

fn arrangers<H: LayoutHost>() -> [Arrange<H>; 7] {
    [
        set_even_h,
        set_even_v,
        set_main_h,
        set_main_h_mirrored,
        set_main_v,
        set_main_v_mirrored,
        set_tiled,
    ]
}

const LAST: u32 = LAYOUT_SET_NAMES.len() as u32 - 1;

/// `layout_set_lookup` (`layout-set.c:52-71`): exact match, else a unique
/// prefix.
pub fn set_lookup(name: &[u8]) -> Option<LayoutSetIndex> {
    let name = cstr(name);
    if let Some(i) = LAYOUT_SET_NAMES.iter().position(|&n| n == name) {
        return Some(LayoutSetIndex(i as u8));
    }
    let mut matched = None;
    for (i, &n) in LAYOUT_SET_NAMES.iter().enumerate() {
        if n.starts_with(name) {
            if matched.is_some() {
                // ambiguous
                return None;
            }
            matched = Some(LayoutSetIndex(i as u8));
        }
    }
    matched
}

fn arrange<H: LayoutHost>(host: &mut H, w: WindowId, layout: u32) -> LayoutSetIndex {
    arrangers::<H>()[layout as usize](host, w);
    let index = LayoutSetIndex(layout as u8);
    host.set_window_lastlayout(w, Some(index));
    index
}

/// `layout_set_select` (`layout-set.c:73-84`): clamps to the last entry.
pub fn set_select<H: LayoutHost>(host: &mut H, w: WindowId, layout: u32) -> LayoutSetIndex {
    arrange(host, w, layout.min(LAST))
}

/// `layout_set_next` (`layout-set.c:86-103`).
pub fn set_next<H: LayoutHost>(host: &mut H, w: WindowId) -> LayoutSetIndex {
    let layout = match host.window_lastlayout(w) {
        None => 0,
        Some(LayoutSetIndex(last)) => {
            let next = u32::from(last) + 1;
            if next > LAST { 0 } else { next }
        }
    };
    arrange(host, w, layout)
}

/// `layout_set_previous` (`layout-set.c:105-124`).
pub fn set_previous<H: LayoutHost>(host: &mut H, w: WindowId) -> LayoutSetIndex {
    let layout = match host.window_lastlayout(w) {
        None => LAST,
        Some(LayoutSetIndex(0)) => LAST,
        Some(LayoutSetIndex(last)) => u32::from(last) - 1,
    };
    arrange(host, w, layout)
}

/// `layout_set_first_tiled` (`layout-set.c:126-136`).
fn first_tiled<H: LayoutHost>(host: &H, w: WindowId) -> Option<PaneId> {
    host.window_panes(w).iter().copied().find(|&wp| {
        host.pane_layout_cell(wp)
            .is_some_and(|lc| cell_is_tiled(host.cells(), lc))
    })
}

fn pane_cell<H: LayoutHost>(host: &H, wp: PaneId) -> LayoutCellId {
    host.pane_layout_cell(wp)
        .expect("pane without a layout cell in a preset layout")
}

/// `layout_set_link_floating` (`layout-set.c:138-151`): every non-tiled cell
/// goes under the root.
fn link_floating<H: LayoutHost>(host: &mut H, w: WindowId, root: LayoutCellId) {
    let n = host.window_panes(w).len();
    for i in 0..n {
        let wp = host.window_panes(w)[i];
        let lc = pane_cell(host, wp);
        if !cell_is_tiled(host.cells(), lc) {
            link_child_tail(host.cells_mut(), root, lc);
        }
    }
}

/// The tail every arranger shares (`layout-set.c:193-202`).
fn finish<H: LayoutHost>(host: &mut H, w: WindowId, root: LayoutCellId, func: &str) {
    fix_offsets(host, w);
    fix_panes(host, w, None);
    debug_print(host.cells(), Some(root), func, 1);
    let g = cell(host.cells(), root).g;
    host.window_resize(w, g.sx, g.sy);
    host.fire_window_event(w, "window-layout-changed");
    host.redraw_window(w);
}

fn new_root<H: LayoutHost>(
    host: &mut H,
    w: WindowId,
    sx: u32,
    sy: u32,
    kind: LayoutType,
) -> LayoutCellId {
    free(host, w, true);
    let root = create_cell(host.cells_mut(), None);
    host.set_window_layout_root(w, Some(root));
    set_size(host.cells_mut(), root, sx, sy, 0, 0);
    make_node(host, root, kind);
    root
}

/// `layout_set_even` (`layout-set.c:153-203`).
fn set_even<H: LayoutHost>(host: &mut H, w: WindowId, kind: LayoutType) {
    debug_print(
        host.cells(),
        host.window_layout_root(w),
        "layout_set_even",
        1,
    );

    let n = host.window_count_panes(w, false);
    if n <= 1 {
        return;
    }

    let (wsx, wsy) = host.window_size(w);
    let (sx, sy) = if kind == LayoutType::Leftright {
        ((n * (PANE_MINIMUM + 1)) - 1, wsy)
    } else {
        (wsx, (n * (PANE_MINIMUM + 1)) - 1)
    };
    let sx = sx.max(wsx);
    let sy = sy.max(wsy);

    let root = new_root(host, w, sx, sy, kind);

    let count = host.window_panes(w).len();
    for i in 0..count {
        let wp = host.window_panes(w)[i];
        let lc = pane_cell(host, wp);
        link_child_tail(host.cells_mut(), root, lc);
        if cell_is_tiled(host.cells(), lc) {
            let c = cell_mut(host.cells_mut(), lc);
            c.g.sx = wsx;
            c.g.sy = wsy;
        }
    }

    spread_cell(host, w, root);
    finish(host, w, root, "layout_set_even");
}

fn set_even_h<H: LayoutHost>(host: &mut H, w: WindowId) {
    set_even(host, w, LayoutType::Leftright);
}

fn set_even_v<H: LayoutHost>(host: &mut H, w: WindowId) {
    set_even(host, w, LayoutType::Topbottom);
}

/// The main/other size rule shared by the four main layouts
/// (`layout-set.c:236-261`, `434-459`): `avail` is the window axis minus one
/// border. Returns `(main, other)`.
fn main_sizes<H: LayoutHost>(
    host: &H,
    w: WindowId,
    avail: u32,
    main_option: &[u8],
    other_option: &[u8],
    default_main: u32,
) -> (u32, u32) {
    let s = host.window_option_string(w, main_option);
    let mut main = match string_percentage(s, 0, avail as i64, avail as i64) {
        Ok(v) => v as u32,
        Err(_) => default_main,
    };

    let other;
    if main.wrapping_add(PANE_MINIMUM) >= avail {
        main = if avail <= PANE_MINIMUM + PANE_MINIMUM {
            PANE_MINIMUM
        } else {
            avail - PANE_MINIMUM
        };
        other = PANE_MINIMUM;
    } else {
        let s = host.window_option_string(w, other_option);
        match string_percentage(s, 0, avail as i64, avail as i64) {
            Err(_) | Ok(0) => other = avail - main,
            Ok(v) => {
                let v = v as u32;
                if v > avail || avail - v < main {
                    other = avail - main;
                } else {
                    other = v;
                    main = avail - other;
                }
            }
        }
    }
    (main, other)
}

/// `layout_set_main_h` and `layout_set_main_h_mirrored`
/// (`layout-set.c:217-413`); `mirrored` puts the others at the head.
fn set_main_h_impl<H: LayoutHost>(host: &mut H, w: WindowId, mirrored: bool) {
    let func = if mirrored {
        "layout_set_main_h_mirrored"
    } else {
        "layout_set_main_h"
    };
    debug_print(host.cells(), host.window_layout_root(w), func, 1);

    let mut n = host.window_count_panes(w, false);
    if n <= 1 {
        return;
    }
    n -= 1; // take off main pane

    // Find available height - take off one line for the border.
    let (wsx, wsy) = host.window_size(w);
    let sy = wsy.wrapping_sub(1);

    let (mainh, otherh) = main_sizes(host, w, sy, b"main-pane-height", b"other-pane-height", 24);

    // Work out what width is needed.
    let sx = ((n * (PANE_MINIMUM + 1)) - 1).max(wsx);

    let root = new_root(host, w, sx, mainh + otherh + 1, LayoutType::Topbottom);

    let wpmain = first_tiled(host, w).expect("main layout without a tiled pane");
    let lcmain = pane_cell(host, wpmain);
    set_size(host.cells_mut(), lcmain, sx, mainh, 0, 0);
    link_child_tail(host.cells_mut(), root, lcmain);

    if n == 1 {
        let wp = next_tiled_after(host, w, wpmain).expect("main layout: one other tiled pane");
        let lc = pane_cell(host, wp);
        if mirrored {
            link_child(host.cells_mut(), root, 0, lc);
        } else {
            link_child_tail(host.cells_mut(), root, lc);
        }
        set_size(host.cells_mut(), lc, sx, otherh, 0, 0);
        link_floating(host, w, root);
    } else {
        let lcother = create_cell(host.cells_mut(), Some(root));
        set_size(host.cells_mut(), lcother, sx, otherh, 0, 0);
        make_node(host, lcother, LayoutType::Leftright);
        if mirrored {
            link_child(host.cells_mut(), root, 0, lcother);
        } else {
            link_child_tail(host.cells_mut(), root, lcother);
        }

        let count = host.window_panes(w).len();
        for i in 0..count {
            let wp = host.window_panes(w)[i];
            if wp == wpmain {
                continue;
            }
            let lc = pane_cell(host, wp);
            link_child_tail(host.cells_mut(), lcother, lc);
            if cell_is_tiled(host.cells(), lc) {
                set_size(host.cells_mut(), lc, PANE_MINIMUM, otherh, 0, 0);
            }
        }
        spread_cell(host, w, lcother);
    }

    finish(host, w, root, func);
}

/// The first tiled pane after `wpmain` in list order (`layout-set.c:279-282`).
fn next_tiled_after<H: LayoutHost>(host: &H, w: WindowId, wpmain: PaneId) -> Option<PaneId> {
    let panes = host.window_panes(w);
    let start = panes.iter().position(|&p| p == wpmain)? + 1;
    panes[start..]
        .iter()
        .copied()
        .find(|&wp| cell_is_tiled(host.cells(), pane_cell(host, wp)))
}

fn set_main_h<H: LayoutHost>(host: &mut H, w: WindowId) {
    set_main_h_impl(host, w, false);
}

fn set_main_h_mirrored<H: LayoutHost>(host: &mut H, w: WindowId) {
    set_main_h_impl(host, w, true);
}

/// `layout_set_main_v` and `layout_set_main_v_mirrored`
/// (`layout-set.c:415-612`).
fn set_main_v_impl<H: LayoutHost>(host: &mut H, w: WindowId, mirrored: bool) {
    let func = if mirrored {
        "layout_set_main_v_mirrored"
    } else {
        "layout_set_main_v"
    };
    debug_print(host.cells(), host.window_layout_root(w), func, 1);

    let mut n = host.window_count_panes(w, false);
    if n <= 1 {
        return;
    }
    n -= 1; // take off main pane

    // Find available width - take off one column for the border.
    let (wsx, wsy) = host.window_size(w);
    let sx = wsx.wrapping_sub(1);

    let (mainw, otherw) = main_sizes(host, w, sx, b"main-pane-width", b"other-pane-width", 80);

    // Work out what height is needed.
    let sy = ((n * (PANE_MINIMUM + 1)) - 1).max(wsy);

    let root = new_root(host, w, mainw + otherw + 1, sy, LayoutType::Leftright);

    let wpmain = first_tiled(host, w).expect("main layout without a tiled pane");
    let lcmain = pane_cell(host, wpmain);
    set_size(host.cells_mut(), lcmain, mainw, sy, 0, 0);
    link_child_tail(host.cells_mut(), root, lcmain);

    if n == 1 {
        let wp = next_tiled_after(host, w, wpmain).expect("main layout: one other tiled pane");
        let lc = pane_cell(host, wp);
        if mirrored {
            link_child(host.cells_mut(), root, 0, lc);
        } else {
            link_child_tail(host.cells_mut(), root, lc);
        }
        set_size(host.cells_mut(), lc, otherw, sy, 0, 0);
        link_floating(host, w, root);
    } else {
        let lcother = create_cell(host.cells_mut(), Some(root));
        make_node(host, lcother, LayoutType::Topbottom);
        set_size(host.cells_mut(), lcother, otherw, sy, 0, 0);
        if mirrored {
            link_child(host.cells_mut(), root, 0, lcother);
        } else {
            link_child_tail(host.cells_mut(), root, lcother);
        }

        let count = host.window_panes(w).len();
        for i in 0..count {
            let wp = host.window_panes(w)[i];
            if wp == wpmain {
                continue;
            }
            let lc = pane_cell(host, wp);
            link_child_tail(host.cells_mut(), lcother, lc);
            if cell_is_tiled(host.cells(), lc) {
                set_size(host.cells_mut(), lc, otherw, PANE_MINIMUM, 0, 0);
            }
        }
        spread_cell(host, w, lcother);
    }

    finish(host, w, root, func);
}

fn set_main_v<H: LayoutHost>(host: &mut H, w: WindowId) {
    set_main_v_impl(host, w, false);
}

fn set_main_v_mirrored<H: LayoutHost>(host: &mut H, w: WindowId) {
    set_main_v_impl(host, w, true);
}

/// `layout_set_tiled` grid shape (`layout-set.c:633-640`): rows and columns
/// for `n` panes with `max_columns` (0 = unlimited).
pub fn tiled_grid(n: u32, max_columns: u32) -> (u32, u32) {
    let mut rows = 1;
    let mut columns = 1;
    while rows * columns < n {
        rows += 1;
        if rows * columns < n && (max_columns == 0 || columns < max_columns) {
            columns += 1;
        }
    }
    (rows, columns)
}

/// `layout_set_tiled` (`layout-set.c:614-735`). Keeps the C unsigned
/// arithmetic in the width and height computation.
fn set_tiled<H: LayoutHost>(host: &mut H, w: WindowId) {
    debug_print(
        host.cells(),
        host.window_layout_root(w),
        "layout_set_tiled",
        1,
    );

    // Get number of panes.
    let n = host.window_count_panes(w, false);
    if n <= 1 {
        return;
    }

    // Get maximum columns from window option.
    let max_columns = host.window_option_number(w, b"tiled-layout-max-columns") as u32;

    // How many rows and columns are wanted?
    let (rows, columns) = tiled_grid(n, max_columns);

    // What width and height should they be?
    let (wsx, wsy) = host.window_size(w);
    let mut width = wsx.wrapping_sub(columns - 1) / columns;
    if width < PANE_MINIMUM {
        width = PANE_MINIMUM;
    }
    let mut height = wsy.wrapping_sub(rows - 1) / rows;
    if height < PANE_MINIMUM {
        height = PANE_MINIMUM;
    }

    let sx = (width.wrapping_add(1).wrapping_mul(columns).wrapping_sub(1)).max(wsx);
    let sy = (height.wrapping_add(1).wrapping_mul(rows).wrapping_sub(1)).max(wsy);

    let root = new_root(host, w, sx, sy, LayoutType::Topbottom);
    let env = host.layout_env(w);

    // Create a grid of the tiled cells.
    let count = host.window_panes(w).len();
    let mut cursor = 0usize;
    let next_tiled = |host: &H, cursor: &mut usize| -> Option<PaneId> {
        while *cursor < count {
            let wp = host.window_panes(w)[*cursor];
            if cell_is_tiled(host.cells(), pane_cell(host, wp)) {
                return Some(wp);
            }
            *cursor += 1;
        }
        None
    };
    for j in 0..rows {
        // If this is the last cell, all done.
        let Some(wp) = next_tiled(host, &mut cursor) else {
            break;
        };
        let mut lcchild = pane_cell(host, wp);

        // If only one column, just use the row directly.
        if n.wrapping_sub(j * columns) == 1 || columns == 1 {
            link_child_tail(host.cells_mut(), root, lcchild);
            set_size(host.cells_mut(), lcchild, wsx, height, 0, 0);
            cursor += 1;
            continue;
        }

        // Create the new row.
        let lcrow = create_cell(host.cells_mut(), Some(root));
        make_node(host, lcrow, LayoutType::Leftright);
        set_size(host.cells_mut(), lcrow, wsx, height, 0, 0);
        link_child_tail(host.cells_mut(), root, lcrow);

        // Add in the columns.
        let mut i = 0;
        while i < columns {
            // Create and add a pane cell.
            link_child_tail(host.cells_mut(), lcrow, lcchild);
            set_size(host.cells_mut(), lcchild, width, height, 0, 0);

            // Move to the next non-floating cell.
            cursor += 1;
            let Some(wp) = next_tiled(host, &mut cursor) else {
                break;
            };
            lcchild = pane_cell(host, wp);
            i += 1;
        }

        // Adjust the row and columns to fit the full width if necessary.
        if i == columns {
            i -= 1;
        }
        let used = (i + 1).wrapping_mul(width.wrapping_add(1)).wrapping_sub(1);
        if wsx <= used {
            continue;
        }
        let last = *cell(host.cells(), lcrow)
            .children
            .last()
            .expect("tiled row without cells");
        resize_adjust(
            host.cells_mut(),
            Some(&env),
            last,
            LayoutType::Leftright,
            (wsx - used) as i32,
        );
    }

    let used = rows.wrapping_mul(height).wrapping_add(rows).wrapping_sub(1);
    if wsy > used {
        let last = *cell(host.cells(), root)
            .children
            .last()
            .expect("tiled layout without rows");
        resize_adjust(
            host.cells_mut(),
            Some(&env),
            last,
            LayoutType::Topbottom,
            (wsy - used) as i32,
        );
    }

    link_floating(host, w, root);
    finish(host, w, root, "layout_set_tiled");
}
