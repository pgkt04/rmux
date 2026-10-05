// Ported from tmux cmd-select-layout.c @ 8f25579c
use super::support::{concat, fail, item_target, item_target_client};
use crate::client::ClientFlags;
use crate::cmd::metadata as m;
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::{QueueItemId, WindowId};
use crate::layout::{self, LayoutDumpFlags};
use crate::server::Server;
use crate::server::events;
use crate::server::operations::{server_redraw_window, server_unzoom_window};

/// `changed:` (cmd-select-layout.c:143-148).
fn changed(server: &mut Server, w: WindowId) -> CmdReturn {
    crate::server::run::recalculate_sizes(server);
    server_redraw_window(server, w);
    events::fire_window(server, b"window-layout-changed", w);
    CmdReturn::Normal
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let c = item_target_client(server, item);
    let Some(w) = target.w.or_else(|| {
        target
            .wl
            .and_then(|wl| server.winlinks.get(wl))
            .map(|wl| wl.window)
    }) else {
        return fail(server, item, b"no current window");
    };
    let wp = target
        .wp
        .or_else(|| server.windows.get(w).and_then(|w| w.active));

    let _ = server_unzoom_window(server, w);

    let next = std::ptr::eq(command.entry, &m::CMD_NEXT_LAYOUT) || args.has(b'n') != 0;
    let previous = std::ptr::eq(command.entry, &m::CMD_PREVIOUS_LAYOUT) || args.has(b'p') != 0;

    let mut flags = LayoutDumpFlags::default();
    if c.and_then(|c| server.clients.get(c)).is_some_and(|c| {
        c.flags.contains(ClientFlags::CONTROL) && !c.flags.contains(ClientFlags::CONTROL_NEWLAYOUTS)
    }) {
        flags.insert(LayoutDumpFlags::OLD_FORMAT);
    }
    let root = server.windows.get(w).and_then(|w| w.layout_root);
    let dumped = layout::dump(server, w, root, flags);
    let oldlayout = match server.windows.get_mut(w) {
        Some(win) => std::mem::replace(&mut win.old_layout, dumped),
        None => None,
    };

    if next || previous {
        if next {
            layout::set_next(server, w);
        } else {
            layout::set_previous(server, w);
        }
        return changed(server, w);
    }

    if args.has(b'E') != 0 {
        if let Some(wp) = wp {
            layout::spread_out(server, wp);
        }
        return changed(server, w);
    }

    let layoutname: Option<&[u8]> = if args.count() != 0 {
        args.string(0)
    } else if args.has(b'o') != 0 {
        oldlayout.as_deref().map(|v| v.as_slice())
    } else {
        None
    };

    if args.has(b'o') == 0 {
        let layout = match layoutname {
            None => server.windows.get(w).and_then(|w| w.lastlayout),
            Some(name) => layout::set_lookup(name),
        };
        if let Some(layout) = layout {
            layout::set_select(server, w, u32::from(layout.0));
            return changed(server, w);
        }
    }

    if let Some(name) = layoutname {
        if let Err(error) = layout::parse(server, w, name) {
            let message = concat(&[&error.cause, b": ", name]);
            // error: restore the previous old_layout (cmd-select-layout.c:150-153).
            if let Some(win) = server.windows.get_mut(w) {
                win.old_layout = oldlayout;
            }
            return fail(server, item, message);
        }
        return changed(server, w);
    }

    CmdReturn::Normal
}
