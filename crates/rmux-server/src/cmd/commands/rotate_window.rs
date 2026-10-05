// Ported from tmux cmd-rotate-window.c @ 8f25579c
use super::support::{fail, item_target, set_item_current};
use crate::cmd::find::{self, CmdFindFlags};
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::{LayoutCellId, PaneId, QueueItemId};
use crate::layout::LayoutHost;
use crate::model::pane::pane_resize;
use crate::model::window::{window_pop_zoom, window_push_zoom, window_set_active_pane};
use crate::server::Server;
use crate::server::operations::server_redraw_window;

/// `wp->layout_cell`, `xoff`, `yoff`, `sx`, `sy` as moved by
/// `cmd-rotate-window.c:62-78, 87-103`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Placement {
    cell: Option<LayoutCellId>,
    xoff: i32,
    yoff: i32,
    sx: u32,
    sy: u32,
}

/// cmd-rotate-window.c:57-107 on the pane list: rotate `panes` (last to head
/// for `-D`, first to tail otherwise) and return, for each position of the
/// rotated list, the position whose placement it takes. `-D`: every pane takes
/// the next pane's placement and the last takes the moved pane's; `-U` is the
/// mirror.
pub fn rotate(panes: &mut [PaneId], down: bool) -> Vec<usize> {
    let n = panes.len();
    if n == 0 {
        return Vec::new();
    }
    if down {
        panes.rotate_right(1);
        (0..n).map(|i| if i + 1 < n { i + 1 } else { 0 }).collect()
    } else {
        panes.rotate_left(1);
        (0..n).map(|i| if i > 0 { i - 1 } else { n - 1 }).collect()
    }
}

/// cmd-rotate-window.c:80-81, 105-106: the pane before (`-D`) or after the
/// active pane in the rotated list, wrapping.
pub fn next_active(panes: &[PaneId], active: Option<PaneId>, down: bool) -> Option<PaneId> {
    let n = panes.len();
    if n == 0 {
        return None;
    }
    let at = active.and_then(|a| panes.iter().position(|p| *p == a));
    Some(if down {
        match at {
            Some(i) if i > 0 => panes[i - 1],
            _ => panes[n - 1],
        }
    } else {
        match at {
            Some(i) if i + 1 < n => panes[i + 1],
            _ => panes[0],
        }
    })
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let Some(wl) = target.wl else {
        return fail(server, item, b"no current window");
    };
    let Some(w) = server.winlinks.get(wl).map(|wl| wl.window) else {
        return fail(server, item, b"no current window");
    };
    let Some(mut panes) = server.windows.get(w).map(|w| w.panes.clone()) else {
        return fail(server, item, b"no current window");
    };
    if panes.is_empty() {
        return fail(server, item, b"no current pane");
    }

    let _ = window_push_zoom(server, w, false, args.has(b'Z') != 0);

    let down = args.has(b'D') != 0;
    let sources = rotate(&mut panes, down);
    let placements: Vec<Placement> = panes
        .iter()
        .map(|wp| {
            let p = server.panes.get(*wp).expect("window pane");
            Placement {
                cell: p.layout_cell,
                xoff: p.xoff,
                yoff: p.yoff,
                sx: p.sx,
                sy: p.sy,
            }
        })
        .collect();
    if let Some(window) = server.windows.get_mut(w) {
        window.panes.clone_from(&panes);
    }
    for (i, wp) in panes.iter().copied().enumerate() {
        let from = placements[sources[i]];
        if let Some(p) = server.panes.get_mut(wp) {
            p.layout_cell = from.cell;
            p.xoff = from.xoff;
            p.yoff = from.yoff;
        }
        if let Some(lc) = from.cell.and_then(|lc| server.layout_cells.get_mut(lc)) {
            lc.pane = Some(wp);
        }
        let _ = pane_resize(server, wp, from.sx, from.sy);
    }

    let active = server.windows.get(w).and_then(|w| w.active);
    let Some(wp) = next_active(&panes, active, down) else {
        return fail(server, item, b"no current pane");
    };

    // cmd-rotate-window.c:109-113
    let _ = window_set_active_pane(server, w, wp, true);
    let current = find::from_winlink_pane(server, wl, wp, CmdFindFlags::default());
    set_item_current(server, item, &current);
    let _ = window_pop_zoom(server, w, true);
    LayoutHost::invalidate_scene(server, w);
    server_redraw_window(server, w);

    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;

    fn id(n: u32) -> PaneId {
        PaneId::from_parts(n, 0)
    }

    #[test]
    fn down_moves_last_to_head_and_shifts_placements_forward() {
        let mut panes = vec![id(0), id(1), id(2)];
        assert_eq!(rotate(&mut panes, true), vec![1, 2, 0]);
        assert_eq!(panes, vec![id(2), id(0), id(1)]);
        // Pane 2 (now first) takes pane 0's slot, 0 takes 1's, 1 takes the
        // moved pane's original placement.
        assert_eq!(next_active(&panes, Some(id(0)), true), Some(id(2)));
        assert_eq!(next_active(&panes, Some(id(2)), true), Some(id(1)));
    }

    #[test]
    fn up_moves_first_to_tail_and_shifts_placements_backward() {
        let mut panes = vec![id(0), id(1), id(2)];
        assert_eq!(rotate(&mut panes, false), vec![2, 0, 1]);
        assert_eq!(panes, vec![id(1), id(2), id(0)]);
        assert_eq!(next_active(&panes, Some(id(0)), false), Some(id(1)));
        assert_eq!(next_active(&panes, Some(id(2)), false), Some(id(0)));
    }

    #[test]
    fn single_pane_is_a_fixed_point() {
        let mut panes = vec![id(7)];
        assert_eq!(rotate(&mut panes, true), vec![0]);
        assert_eq!(rotate(&mut panes, false), vec![0]);
        assert_eq!(next_active(&panes, Some(id(7)), true), Some(id(7)));
        assert_eq!(next_active(&[], None, true), None);
    }
}
