// Ported from tmux cmd-kill-pane.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    format::{self, FormatContext},
    ids::QueueItemId,
    layout, model,
    server::{Server, operations},
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(target) = server.queue.items.get(item).map(|i| i.target) else {
        return CmdReturn::Error;
    };
    if args.get(b'f').is_some() && args.has(b'a') == 0 {
        queue::error(server, item, b"-f only valid with -a");
        return CmdReturn::Error;
    }
    if args.has(b'a') == 0 {
        let Some(pane) = target.wp else {
            queue::error(server, item, b"no active pane to kill");
            return CmdReturn::Error;
        };
        let _ = operations::server_kill_pane(server, pane);
        return CmdReturn::Normal;
    }
    let Some(window) = target.w else {
        return CmdReturn::Error;
    };
    let _ = operations::server_unzoom_window(server, window);
    let panes = server
        .windows
        .get(window)
        .map(|w| w.panes.clone())
        .unwrap_or_default();
    for pane in panes {
        if Some(pane) == target.wp {
            continue;
        }
        if let Some(filter) = args.get(b'f') {
            let value = format::single(
                server,
                Some(item),
                FormatContext {
                    session: target.s,
                    winlink: target.wl,
                    window: Some(window),
                    pane: Some(pane),
                    ..FormatContext::default()
                },
                filter,
            );
            if !format::true_value(Some(&value)) {
                continue;
            }
        }
        crate::client::mouse::remove_pane(server, pane);
        layout::close_pane(server, pane);
        let _ = model::window::window_remove_pane(server, window, pane);
    }
    operations::server_redraw_window(server, window);
    CmdReturn::Normal
}
