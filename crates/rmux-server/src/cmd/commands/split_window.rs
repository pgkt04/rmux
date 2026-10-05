// Ported from tmux cmd-split-window.c @ 8f25579c
use super::support::{
    concat, fail, item_client, item_event, item_target, item_target_client, set_item_current,
};
use crate::client::ClientFlags;
use crate::client::mouse::{MouseDragAction, MouseTarget, ResolvedMouseEvent};
use crate::cmd::find::{self, CmdFindFlags, MouseInput};
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::format::{self, FormatContext};
use crate::ids::{ClientId, OptionsId, PaneId, QueueItemId, WindowId};
use crate::layout::{self, LayoutGeometry, PANE_MINIMUM};
use crate::model::spawn::{self, SpawnContext, SpawnFlags};
use crate::model::{ModelError, PaneFlags, WindowFlags, pane, window};
use crate::options::environment::EnvironmentFlags;
use crate::server::events::{self, EventPayload};
use crate::server::{Server, operations};
use rmux_emu::screen::PaneLines;
use rmux_util::key::MouseEvent;

const SPLIT_WINDOW_TEMPLATE: &[u8] = b"#{session_name}:#{window_index}.#{pane_index}";
/// `PANE_LINES_NONE` (`tmux.h`).
const PANE_LINES_NONE: i64 = 6;

/// The `char *cause` bytes of a failed spawn.
pub(super) fn spawn_cause(error: &ModelError) -> Vec<u8> {
    match error {
        ModelError::Message(bytes) => bytes.clone(),
        other => other.to_string().into_bytes(),
    }
}

/// `options_set_string(new_wp->options, name, 0, "%s", value)` (`cmd-split-window.c:227-250`);
/// false when no table entry resolves for `name`, the case where the C returns NULL.
fn set_pane_string(server: &mut Server, options: OptionsId, name: &[u8], value: &[u8]) -> bool {
    if server.options.get(options, name).is_none() {
        return false;
    }
    let mut store = std::mem::take(&mut server.options);
    store.set_string(options, name, false, value, server);
    server.options = store;
    true
}

fn set_pane_number(server: &mut Server, options: OptionsId, name: &[u8], value: i64) {
    let mut store = std::mem::take(&mut server.options);
    store.set_number(options, name, value, server);
    server.options = store;
}

/// `cmd-split-window.c:334-352`: tear down a pane spawned before a later failure.
fn fail_cleanup(
    server: &mut Server,
    new_wp: Option<PaneId>,
    w: WindowId,
    is_floating: bool,
    restore_zoom: bool,
    flags: SpawnFlags,
) -> CmdReturn {
    if let Some(p) = new_wp {
        crate::client::mouse::remove_pane(server, p);
        if !is_floating {
            layout::close_pane(server, p);
        }
        let _ = window::window_remove_pane(server, w, p);
    }
    if restore_zoom || !flags.contains(SpawnFlags::FLOATING) {
        let _ = window::window_pop_zoom(server, w, true);
    }
    CmdReturn::Error
}

/// The queue item's `struct mouse_event` as the drag handlers receive it.
fn resolved_event(m: &MouseInput) -> ResolvedMouseEvent {
    ResolvedMouseEvent {
        event: MouseEvent {
            x: m.x,
            y: m.y,
            lx: m.last_x,
            ly: m.last_y,
            ..MouseEvent::default()
        },
        target: MouseTarget {
            session: m.session,
            window: m.window,
            pane: m.pane,
            status_at: m.status_at,
            status_lines: m.status_lines,
            ox: m.offset_x,
            oy: m.offset_y,
            valid: m.valid,
            ..MouseTarget::default()
        },
    }
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let tc = item_target_client(server, item);
    let (Some(s), Some(wl), Some(wp)) = (target.s, target.wl, target.wp) else {
        return fail(server, item, b"no current window");
    };
    let Some(w) = server.winlinks.get(wl).map(|link| link.window) else {
        return fail(server, item, b"no current window");
    };
    let event = item_event(server, item);
    let count = args.count();
    let mut flags = SpawnFlags::default();

    let mut restore_zoom = window::window_active_pane_is_over_zoom(server, w);
    let is_floating = if std::ptr::eq(command.entry, &crate::cmd::metadata::CMD_NEW_PANE) {
        args.has(b'L') == 0
    } else {
        if !pane::pane_is_visible(server, wp) {
            restore_zoom = false;
        }
        if !restore_zoom {
            let _ = window::window_unzoom(server, w, true);
        }
        flags.insert(SpawnFlags::SPLIT);
        pane::pane_is_floating(server, wp)
    };

    let modal = args.has(b'O') != 0;
    if modal {
        if !is_floating {
            return fail(server, item, b"modal pane must be floating");
        }
        if server.windows.get(w).is_some_and(|w| w.modal.is_some()) {
            return fail(server, item, b"window already has a modal pane");
        }
    }

    let mouse_resize = args.has(b'M') != 0 && is_floating;
    if mouse_resize && (!event.mouse.valid || tc.is_none()) {
        return CmdReturn::Normal;
    }

    if is_floating {
        flags.insert(SpawnFlags::FLOATING);
    }
    if args.has(b'h') != 0 {
        flags.insert(SpawnFlags::HORIZONTAL);
    }
    if args.has(b'b') != 0 {
        flags.insert(SpawnFlags::BEFORE);
    }
    if args.has(b'f') != 0 {
        flags.insert(SpawnFlags::FULLSIZE);
    }
    if args.has(b'd') != 0 {
        flags.insert(SpawnFlags::DETACHED);
    }
    if args.has(b'Z') != 0 {
        flags.insert(SpawnFlags::ZOOM);
    }
    if modal {
        flags.insert(SpawnFlags::MODAL | SpawnFlags::FLOATOVERZOOM);
    }
    if is_floating && args.has(b'A') != 0 {
        flags.insert(SpawnFlags::FLOATOVERZOOM);
    }
    if server
        .windows
        .get(w)
        .is_some_and(|w| w.flags.contains(WindowFlags::ZOOMED))
        && flags.contains(SpawnFlags::FLOATOVERZOOM)
    {
        restore_zoom = true;
    }

    let mut input = args.has(b'I') != 0;
    let only_empty_arg = count == 1 && args.string(0).is_some_and(<[u8]>::is_empty);
    let empty = input || only_empty_arg || args.has(b'E') != 0;
    if empty && count != 0 && !only_empty_arg {
        return fail(server, item, b"command cannot be given for empty pane");
    }
    if empty {
        flags.insert(SpawnFlags::EMPTY);
    }

    let lines = match args.get(b'B') {
        None => window_pane_lines(server, w),
        Some(value) => {
            let oe = crate::options::search(b"pane-border-lines")
                .expect("pane-border-lines is a table option");
            match crate::options::find_choice(oe, value) {
                Ok(lines) => lines,
                Err(cause) => {
                    return fail(
                        server,
                        item,
                        concat(&[b"pane-border-lines ", cause.as_bytes()]),
                    );
                }
            }
        }
    };

    let cell = if flags.contains(SpawnFlags::FLOATING) {
        let lines = PaneLines::try_from(lines as i32).unwrap_or(PaneLines::Single);
        layout::get_floating_cell(server, item, args, lines, w, wp, flags)
    } else {
        layout::get_tiled_cell(server, item, args, w, wp, flags)
    };
    let lc = match cell {
        Ok(lc) => lc,
        Err(error) => {
            queue::error(server, item, &error.cause);
            if restore_zoom {
                let _ = window::window_pop_zoom(server, w, true);
            }
            return CmdReturn::Error;
        }
    };

    let mut sc = SpawnContext::new(s);
    sc.item = Some(item);
    sc.winlink = Some(wl);
    sc.pane = Some(wp);
    sc.layout_cell = Some(lc);
    sc.argv = args
        .values()
        .iter()
        .map(|value| value.as_string().to_vec())
        .collect();
    for value in args.values_of(b'e') {
        sc.environment
            .put(value.as_string(), EnvironmentFlags::default());
    }
    sc.index = -1;
    sc.cwd = args.get(b'c').map(<[u8]>::to_vec);
    sc.flags = flags;

    let new_wp = match spawn::spawn_pane(server, &mut sc) {
        Ok(p) => p,
        Err(cause) => {
            queue::error(
                server,
                item,
                &concat(&[b"create pane failed: ", &spawn_cause(&cause)]),
            );
            return fail_cleanup(server, None, w, is_floating, restore_zoom, flags);
        }
    };
    let Some(new_options) = server.panes.get_mut(new_wp).map(|p| {
        if args.has(b'K') != 0 && modal {
            p.flags.insert(PaneFlags::CAPTUREALLKEYS);
        }
        if args.has(b'C') != 0 && modal {
            p.flags.insert(PaneFlags::CLOSEONCLICK);
        }
        if args.has(b'D') != 0 && modal {
            p.flags.insert(PaneFlags::CLOSEONCANCEL);
        }
        p.options
    }) else {
        return fail_cleanup(server, None, w, is_floating, restore_zoom, flags);
    };

    if let Some(style) = args.get(b's') {
        if !set_pane_string(server, new_options, b"window-style", style) {
            queue::error(server, item, &concat(&[b"bad style: ", style]));
            return fail_cleanup(server, Some(new_wp), w, is_floating, restore_zoom, flags);
        }
        set_pane_string(server, new_options, b"window-active-style", style);
        if let Some(p) = server.panes.get_mut(new_wp) {
            p.flags
                .insert(PaneFlags::REDRAW | PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
        }
    }
    if let Some(style) = args.get(b'S') {
        if !set_pane_string(server, new_options, b"pane-active-border-style", style) {
            queue::error(
                server,
                item,
                &concat(&[b"bad active border style: ", style]),
            );
            return fail_cleanup(server, Some(new_wp), w, is_floating, restore_zoom, flags);
        }
    }
    if let Some(style) = args.get(b'R') {
        if !set_pane_string(server, new_options, b"pane-border-style", style) {
            queue::error(
                server,
                item,
                &concat(&[b"bad inactive border style: ", style]),
            );
            return fail_cleanup(server, Some(new_wp), w, is_floating, restore_zoom, flags);
        }
    }
    if args.has(b'B') != 0 {
        set_pane_number(server, new_options, b"pane-border-lines", lines);
    }
    if args.has(b'k') != 0 || args.has(b'm') != 0 {
        set_pane_number(server, new_options, b"remain-on-exit", 3);
        if let Some(message) = args.get(b'm') {
            set_pane_string(server, new_options, b"remain-on-exit-format", message);
        }
    }
    if let Some(template) = args.get(b'T') {
        let title = format::single_from_target(server, item, template);
        if let Some(p) = server.panes.get_mut(new_wp) {
            p.base.set_title(&title, false);
        }
        let fs = find::from_pane(server, new_wp, CmdFindFlags::default()).unwrap_or_default();
        let mut ep = EventPayload::new();
        ep.set_target(server, &fs);
        ep.set_pane(server, b"pane", new_wp);
        ep.set_window(server, b"window", w);
        ep.set_string(server, b"new_title", &title);
        events::fire(server, b"pane-title-changed", ep);
    }

    if input {
        // window_pane_start_input: -1 is an error, 1 means nothing to read.
        match item_client(server, item) {
            None => input = false,
            Some(c) => {
                let (attached, dead) = server
                    .clients
                    .get(c)
                    .map(|c| {
                        (
                            c.session.is_some(),
                            c.flags.intersects(ClientFlags::DEAD | ClientFlags::EXITED),
                        )
                    })
                    .unwrap_or((false, true));
                match pane::pane_start_input(server, new_wp, c, item, attached, dead) {
                    Err(cause) => {
                        queue::error(server, item, &spawn_cause(&cause));
                        return fail_cleanup(
                            server,
                            Some(new_wp),
                            w,
                            is_floating,
                            restore_zoom,
                            flags,
                        );
                    }
                    Ok(None) => input = false,
                    Ok(Some(_)) => {}
                }
            }
        }
    }
    if !flags.contains(SpawnFlags::DETACHED) {
        let current = find::from_winlink_pane(server, wl, new_wp, CmdFindFlags::default());
        set_item_current(server, item, &current);
    }

    if restore_zoom || (!flags.contains(SpawnFlags::FLOATING) && !modal) {
        let _ = window::window_pop_zoom(server, w, true);
        operations::server_redraw_window(server, w);
    }
    operations::server_redraw_session(server, s);

    if mouse_resize {
        if let Some(tc) = tc {
            if let Some(c) = server.clients.get_mut(tc) {
                c.drag.last_pane = Some(new_wp);
                c.drag.update = Some(MouseDragAction::SplitWindowResize);
            }
            drag_update(server, tc, &resolved_event(&event.mouse));
        }
    }

    if args.has(b'P') != 0 {
        let template = args.get(b'F').unwrap_or(SPLIT_WINDOW_TEMPLATE);
        let output = format::single(
            server,
            Some(item),
            FormatContext {
                evaluated_client: tc,
                session: Some(s),
                winlink: Some(wl),
                window: Some(w),
                pane: Some(new_wp),
                ..FormatContext::default()
            },
            template,
        );
        queue::print(server, item, &output);
    }

    let fs = find::from_winlink_pane(server, wl, new_wp, CmdFindFlags::default());
    queue::insert_hook(server, item, Some(&fs), b"after-split-window");

    if input {
        return CmdReturn::Wait;
    }
    if args.has(b'W') != 0 {
        // Blocked until the pane's command exits; window_pane_wait_finish continues it.
        if let Some(p) = server.panes.get_mut(new_wp) {
            p.wait_item = Some(item);
        }
        return CmdReturn::Wait;
    }
    CmdReturn::Normal
}

/// `window_get_pane_lines` (`window.c`).
fn window_pane_lines(server: &Server, w: WindowId) -> i64 {
    server.windows.get(w).map_or(0, |w| {
        server.options.get_number(w.options, b"pane-border-lines")
    })
}

/// Geometry arithmetic of `cmd_split_window_mouse_resize` (`cmd-split-window.c:387-419`):
/// the drag anchor is one corner, the pointer the opposite one.
fn resize_geometry(x: i32, y: i32, drag_x: u32, drag_y: u32, border: bool) -> LayoutGeometry {
    let (x, y) = (i64::from(x), i64::from(y));
    let (drag_x, drag_y) = (i64::from(drag_x), i64::from(drag_y));
    let b = i64::from(border);
    let (mut sx, xoff) = if x >= drag_x {
        (x - drag_x + 1, drag_x + b)
    } else {
        let sx = drag_x - x + 1;
        (sx, drag_x - sx + 1 + b)
    };
    let (mut sy, yoff) = if y >= drag_y {
        (y - drag_y + 1, drag_y + b)
    } else {
        let sy = drag_y - y + 1;
        (sy, drag_y - sy + 1 + b)
    };
    let minimum = i64::from(PANE_MINIMUM);
    if border {
        sx = if sx <= 2 { minimum } else { sx - 2 };
        sy = if sy <= 2 { minimum } else { sy - 2 };
    }
    LayoutGeometry::new(
        sx.max(minimum) as u32,
        sy.max(minimum) as u32,
        xoff as i32,
        yoff as i32,
    )
}

/// `cmd_split_window_mouse_resize` (`cmd-split-window.c:356-431`).
pub fn drag_update(server: &mut Server, client: ClientId, ev: &ResolvedMouseEvent) {
    let Some(c) = server.clients.get(client) else {
        return;
    };
    let Some(wp) = c.drag.last_pane else {
        return;
    };
    let (drag_x, drag_y) = (c.drag.x, c.drag.y);
    if server.panes.get(wp).is_none() || !pane::pane_is_floating(server, wp) {
        if let Some(c) = server.clients.get_mut(client) {
            c.drag.update = None;
        }
        return;
    }
    let Some((w, lc, oxoff, oyoff, osx, osy)) = server.panes.get(wp).and_then(|p| {
        p.layout_cell
            .map(|lc| (p.window, lc, p.xoff, p.yoff, p.sx, p.sy))
    }) else {
        return;
    };

    let m = &ev.event;
    let t = &ev.target;
    let x = m.x.wrapping_add(t.ox) as i32;
    let mut y = m.y.wrapping_add(t.oy) as i32;
    if t.status_at == 0 && y >= t.status_lines as i32 {
        y -= t.status_lines as i32;
    } else if t.status_at > 0 && y >= t.status_at {
        y = t.status_at - 1;
    }

    let border = pane::pane_get_pane_lines(server, wp) != PANE_LINES_NONE;
    let g = resize_geometry(x, y, drag_x, drag_y, border);

    layout::set_size(&mut server.layout_cells, lc, g.sx, g.sy, g.xoff, g.yoff);
    layout::fix_panes(server, w, None);

    window::window_redraw_floating_pane(server, wp, oxoff, oyoff, osx, osy);
    operations::server_redraw_window_borders(server, w);
}

/// split-window installs only `mouse_drag_update` (`cmd-split-window.c:302`); the
/// release action has no C counterpart and nothing to undo.
pub fn drag_release(_server: &mut Server, _client: ClientId, _ev: &ResolvedMouseEvent) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_geometry_pointer_below_right_of_anchor() {
        let g = resize_geometry(14, 9, 10, 5, false);
        assert_eq!((g.sx, g.sy, g.xoff, g.yoff), (5, 5, 10, 5));
        let g = resize_geometry(14, 9, 10, 5, true);
        assert_eq!((g.sx, g.sy, g.xoff, g.yoff), (3, 3, 11, 6));
    }

    #[test]
    fn resize_geometry_pointer_above_left_of_anchor() {
        let g = resize_geometry(6, 2, 10, 5, false);
        assert_eq!((g.sx, g.sy, g.xoff, g.yoff), (5, 4, 6, 2));
        let g = resize_geometry(6, 2, 10, 5, true);
        assert_eq!((g.sx, g.sy, g.xoff, g.yoff), (3, 2, 7, 3));
    }

    #[test]
    fn resize_geometry_clamps_to_pane_minimum() {
        let g = resize_geometry(10, 5, 10, 5, true);
        assert_eq!(
            (g.sx, g.sy, g.xoff, g.yoff),
            (PANE_MINIMUM, PANE_MINIMUM, 11, 6)
        );
        let g = resize_geometry(11, 6, 10, 5, true);
        assert_eq!((g.sx, g.sy), (PANE_MINIMUM, PANE_MINIMUM));
        let g = resize_geometry(12, 7, 10, 5, true);
        assert_eq!((g.sx, g.sy), (PANE_MINIMUM, PANE_MINIMUM));
        let g = resize_geometry(13, 8, 10, 5, true);
        assert_eq!((g.sx, g.sy), (2, 2));
    }
}
