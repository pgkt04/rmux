// Ported from tmux cmd-select-pane.c @ 8f25579c

use super::support::{fail, item_target, set_item_current};
use crate::client::ClientFlags;
use crate::cmd::find::{self, CmdFindFlags, PaneDirection};
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::format;
use crate::ids::{ClientId, PaneId, QueueItemId, WindowId};
use crate::model::{PaneFlags, pane, session, window};
use crate::server::Server;
use crate::server::events::{self, EventPayload};
use crate::server::operations::{
    server_check_marked, server_clear_marked, server_is_marked, server_redraw_client,
    server_redraw_window, server_redraw_window_borders, server_set_marked, server_status_window,
};

/// cmd-select-pane.c:58-81
fn redraw(server: &mut Server, w: WindowId) {
    let clients: Vec<ClientId> = server.client_order.iter().copied().collect();
    for c in clients {
        let Some(client) = server.clients.get(c) else {
            continue;
        };
        let Some(s) = client.session else { continue };
        if client.flags.contains(ClientFlags::CONTROL) {
            continue;
        }
        let shows = server
            .sessions
            .get(s)
            .and_then(|s| s.current)
            .and_then(|wl| server.winlinks.get(wl))
            .is_some_and(|wl| wl.window == w);
        if shows && crate::server::run::tty_window_bigger(server, c) {
            server_redraw_client(server, c);
        } else {
            let has = session::session_has(server, s, w);
            let Some(client) = server.clients.get_mut(c) else {
                continue;
            };
            if shows {
                client.flags.insert(ClientFlags::REDRAWBORDERS);
            }
            if has {
                client.flags.insert(ClientFlags::REDRAWSTATUS);
            }
        }
    }
}

fn pane_window(server: &Server, wp: PaneId) -> Option<WindowId> {
    server.panes.get(wp).map(|p| p.window)
}

/// `wp->flags |= ...; server_redraw_window_borders(wp->window); server_status_window(wp->window)`.
fn flag_pane_and_redraw(server: &mut Server, wp: PaneId, insert: PaneFlags, remove: PaneFlags) {
    let Some(p) = server.panes.get_mut(wp) else {
        return;
    };
    p.flags.insert(insert);
    p.flags.remove(remove);
    let w = p.window;
    server_redraw_window_borders(server, w);
    server_status_window(server, w);
}

/// `w->modal != NULL && wp != w->modal` counts as visible (cmd-select-pane.c:187-190, 276-279).
fn pane_visible_for_select(server: &Server, w: WindowId, wp: PaneId) -> bool {
    if server
        .windows
        .get(w)
        .and_then(|w| w.modal)
        .is_some_and(|m| m != wp)
    {
        true
    } else {
        pane::pane_is_visible(server, wp)
    }
}

/// cmd-select-pane.c:83-144
fn marked_pane(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let (Some(s), Some(wl), Some(wp)) = (target.s, target.wl, target.wp) else {
        return fail(server, item, b"no current pane");
    };
    if args.has(b'm') != 0 && !pane::pane_is_visible(server, wp) {
        return CmdReturn::Normal;
    }
    let lwp = if server_check_marked(server) {
        server.marked_pane
    } else {
        None
    };
    if args.has(b'M') != 0 || server_is_marked(server, Some(s), Some(wl), Some(wp)) {
        server_clear_marked(server);
    } else {
        server_set_marked(server, Some(s), Some(wl), Some(wp));
    }
    let mwp = server.marked_pane;

    let mut ep = EventPayload::new();
    let fs = find::from_pane(server, mwp.or(lwp).unwrap_or(wp), CmdFindFlags::default())
        .unwrap_or_default();
    ep.set_target(server, &fs);
    if let Some(mwp) = mwp {
        ep.set_pane(server, b"pane", mwp);
        ep.set_pane(server, b"new_pane", mwp);
        if let Some(w) = pane_window(server, mwp) {
            ep.set_window(server, b"window", w);
        }
    } else if let Some(lwp) = lwp {
        ep.set_pane(server, b"pane", lwp);
        if let Some(w) = pane_window(server, lwp) {
            ep.set_window(server, b"window", w);
        }
    } else {
        ep.set_pane(server, b"pane", wp);
        if let Some(w) = pane_window(server, wp) {
            ep.set_window(server, b"window", w);
        }
    }
    if let Some(lwp) = lwp {
        ep.set_pane(server, b"old_pane", lwp);
    }
    ep.set_int(server, b"marked", i32::from(mwp.is_some()));
    events::fire(server, b"marked-pane-changed", ep);

    let changed = PaneFlags::REDRAW | PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED;
    if let Some(lwp) = lwp {
        flag_pane_and_redraw(server, lwp, changed, PaneFlags(0));
    }
    if let Some(mwp) = mwp {
        flag_pane_and_redraw(server, mwp, changed, PaneFlags(0));
    }
    if pane::pane_is_floating(server, wp)
        && let Some(w) = pane_window(server, wp)
    {
        let _ = window::window_redraw_active_switch(server, w, Some(wp));
        let _ = window::window_set_active_pane(server, w, wp, true);
    }
    CmdReturn::Normal
}

/// cmd-select-pane.c:165-204
fn last_pane(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let Some(wl) = target.wl else {
        return fail(server, item, b"no current window");
    };
    let Some(w) = target
        .w
        .or_else(|| server.winlinks.get(wl).map(|wl| wl.window))
    else {
        return fail(server, item, b"no current window");
    };
    let zflag = args.has(b'Z') != 0;

    let mut lastwp = server.windows.get(w).and_then(|w| w.last.first().copied());
    if lastwp.is_none()
        && window::window_count_panes(server, w, true) == 2
        && let Some(win) = server.windows.get(w)
        && let Some(active) = win.active
        && let Some(pos) = win.panes.iter().position(|&p| p == active)
    {
        lastwp = if pos > 0 {
            Some(win.panes[pos - 1])
        } else {
            win.panes.get(pos + 1).copied()
        };
    }
    let Some(lastwp) = lastwp else {
        return fail(server, item, b"no last pane");
    };
    if args.has(b'e') != 0 {
        flag_pane_and_redraw(server, lastwp, PaneFlags(0), PaneFlags::INPUTOFF);
    } else if args.has(b'd') != 0 {
        flag_pane_and_redraw(server, lastwp, PaneFlags::INPUTOFF, PaneFlags(0));
    } else {
        let visible = pane_visible_for_select(server, w, lastwp);
        if !visible && window::window_push_zoom(server, w, false, zflag).is_ok_and(|z| z) {
            server_redraw_window(server, w);
        }
        let _ = window::window_redraw_active_switch(server, w, Some(lastwp));
        if window::window_set_active_pane(server, w, lastwp, true).is_ok_and(|c| c) {
            let current = find::from_winlink(server, wl, CmdFindFlags::default());
            set_item_current(server, item, &current);
            redraw(server, w);
        }
        if !visible && window::window_pop_zoom(server, w, true).is_ok_and(|z| z) {
            server_redraw_window(server, w);
        }
    }
    CmdReturn::Normal
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    if std::ptr::eq(command.entry, &crate::cmd::metadata::CMD_LAST_PANE) || args.has(b'l') != 0 {
        return last_pane(server, command, item);
    }
    if args.has(b'm') != 0 || args.has(b'M') != 0 {
        return marked_pane(server, command, item);
    }

    let target = item_target(server, item);
    let (Some(_), Some(wl), Some(mut wp)) = (target.s, target.wl, target.wp) else {
        return fail(server, item, b"no current pane");
    };
    let Some(w) = target
        .w
        .or_else(|| server.winlinks.get(wl).map(|wl| wl.window))
    else {
        return fail(server, item, b"no current window");
    };
    let Some(oo) = server.panes.get(wp).map(|p| p.options) else {
        return fail(server, item, b"no current pane");
    };
    let zflag = args.has(b'Z') != 0;

    if let Some(style) = args.get(b'P') {
        // cmd-select-pane.c:209-218: options_set_string never fails, so the
        // `bad style` branch is dead; the oracle accepts any text here.
        let mut options = std::mem::take(&mut server.options);
        options.set_string(oo, b"window-style", false, style, server);
        options.set_string(oo, b"window-active-style", false, style, server);
        server.options = options;
        if let Some(p) = server.panes.get_mut(wp) {
            p.flags
                .insert(PaneFlags::REDRAW | PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
        }
    }
    if args.has(b'g') != 0 {
        let style = server.options.get_string(oo, b"window-style").to_vec();
        queue::print(server, item, &style);
        return CmdReturn::Normal;
    }

    let direction = if args.has(b'L') != 0 {
        Some(PaneDirection::Left)
    } else if args.has(b'R') != 0 {
        Some(PaneDirection::Right)
    } else if args.has(b'U') != 0 {
        Some(PaneDirection::Up)
    } else if args.has(b'D') != 0 {
        Some(PaneDirection::Down)
    } else {
        None
    };
    if let Some(direction) = direction {
        let _ = window::window_push_zoom(server, w, false, true);
        let found = window::pane_direction(server, wp, direction);
        let _ = window::window_pop_zoom(server, w, true);
        match found {
            Some(found) => wp = found,
            None => return CmdReturn::Normal,
        }
    }

    if args.has(b'e') != 0 {
        flag_pane_and_redraw(server, wp, PaneFlags(0), PaneFlags::INPUTOFF);
        return CmdReturn::Normal;
    }
    if args.has(b'd') != 0 {
        flag_pane_and_redraw(server, wp, PaneFlags::INPUTOFF, PaneFlags(0));
        return CmdReturn::Normal;
    }

    if let Some(template) = args.get(b'T') {
        let title = format::single_from_target(server, item, template);
        let changed = server
            .panes
            .get_mut(wp)
            .is_some_and(|p| p.base.set_title(&title, false));
        if changed && let Some(pw) = pane_window(server, wp) {
            let mut ep = EventPayload::new();
            let fs = find::from_pane(server, wp, CmdFindFlags::default()).unwrap_or_default();
            ep.set_target(server, &fs);
            ep.set_pane(server, b"pane", wp);
            ep.set_window(server, b"window", pw);
            ep.set_string(server, b"new_title", &title);
            events::fire(server, b"pane-title-changed", ep);
            server_redraw_window_borders(server, pw);
            server_status_window(server, pw);
        }
        return CmdReturn::Normal;
    }

    if server
        .windows
        .get(w)
        .is_some_and(|win| win.active == Some(wp))
    {
        return CmdReturn::Normal;
    }
    let visible = pane_visible_for_select(server, w, wp);
    if !visible && window::window_push_zoom(server, w, false, zflag).is_ok_and(|z| z) {
        server_redraw_window(server, w);
    }
    let _ = window::window_redraw_active_switch(server, w, Some(wp));
    if window::window_set_active_pane(server, w, wp, true).is_ok_and(|c| c) {
        let current = find::from_winlink_pane(server, wl, wp, CmdFindFlags::default());
        set_item_current(server, item, &current);
    }
    let current = super::support::item_current(server, item);
    queue::insert_hook(server, item, Some(&current), b"after-select-pane");
    redraw(server, w);
    if !visible && window::window_pop_zoom(server, w, true).is_ok_and(|z| z) {
        server_redraw_window(server, w);
    }
    CmdReturn::Normal
}
