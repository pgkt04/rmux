// Ported from tmux cmd-kill-session.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    format::{self, FormatContext},
    ids::QueueItemId,
    model::{self, WindowFlags, WinlinkFlags},
    server::{Server, operations},
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(session) = server.queue.items.get(item).and_then(|i| i.target.s) else {
        return CmdReturn::Error;
    };
    if args.get(b'f').is_some() && (args.has(b'a') == 0 || args.has(b'C') != 0) {
        queue::error(server, item, b"-f only valid with -a");
        return CmdReturn::Error;
    }
    if args.has(b'C') != 0 {
        let links: Vec<_> = server
            .sessions
            .get(session)
            .map(|s| s.windows.values().copied().collect())
            .unwrap_or_default();
        for link in links {
            if let Some(wl) = server.winlinks.get_mut(link) {
                wl.flags.remove(WinlinkFlags::ALERTFLAGS);
                if let Some(window) = server.windows.get_mut(wl.window) {
                    window.flags.remove(WindowFlags::ALERTFLAGS);
                }
            }
        }
        operations::server_redraw_session(server, session);
        return CmdReturn::Normal;
    }
    let sessions: Vec<_> = if args.has(b'a') != 0 {
        server
            .session_names
            .values()
            .copied()
            .filter(|s| *s != session)
            .collect()
    } else if args.has(b'g') != 0 {
        model::session::session_group_contains(server, session)
            .and_then(|g| server.groups.get(g))
            .map(|g| g.sessions.clone())
            .unwrap_or_else(|| vec![session])
    } else {
        vec![session]
    };
    for current in sessions {
        if let Some(filter) = args.get(b'f') {
            let value = format::single(
                server,
                Some(item),
                FormatContext {
                    session: Some(current),
                    ..FormatContext::default()
                },
                filter,
            );
            if !format::true_value(Some(&value)) {
                continue;
            }
        }
        operations::server_destroy_session(server, current);
        model::session::session_destroy(server, current, true);
    }
    CmdReturn::Normal
}
