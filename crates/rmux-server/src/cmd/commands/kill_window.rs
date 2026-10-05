// Ported from tmux cmd-kill-window.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    format::{self, FormatContext},
    ids::{QueueItemId, SessionId, WinlinkId},
    model::{self, ModelEffect},
    server::{Server, operations},
};

fn matches(
    server: &mut Server,
    item: QueueItemId,
    session: SessionId,
    link: WinlinkId,
    filter: Option<&[u8]>,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let window = server.winlinks.get(link).map(|l| l.window);
    let value = format::single(
        server,
        Some(item),
        FormatContext {
            session: Some(session),
            winlink: Some(link),
            window,
            ..FormatContext::default()
        },
        filter,
    );
    format::true_value(Some(&value))
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(target) = server.queue.items.get(item).map(|i| i.target) else {
        return CmdReturn::Error;
    };
    let (Some(session), Some(link), Some(window)) = (target.s, target.wl, target.w) else {
        return CmdReturn::Error;
    };
    if args.get(b'f').is_some() && args.has(b'a') == 0 {
        queue::error(server, item, b"-f only valid with -a");
        return CmdReturn::Error;
    }
    if command.entry.name == b"unlink-window" {
        if args.has(b'k') == 0 && !model::session::session_is_linked(server, session, window) {
            queue::error(server, item, b"window only linked to one session");
            return CmdReturn::Error;
        }
        let _ = operations::server_unlink_window(server, session, link);
        server.effects.push_back(ModelEffect::RecalculateSizes);
        return CmdReturn::Normal;
    }
    if args.has(b'a') == 0 {
        let _ = operations::server_kill_window(server, window, true);
        return CmdReturn::Normal;
    }
    if server
        .sessions
        .get(session)
        .is_none_or(|s| s.windows.len() <= 1)
    {
        return CmdReturn::Normal;
    }
    loop {
        let links: Vec<_> = server
            .sessions
            .get(session)
            .map(|s| s.windows.values().copied().collect())
            .unwrap_or_default();
        let found = links.into_iter().find(|link| {
            server
                .winlinks
                .get(*link)
                .is_some_and(|l| l.window != window)
                && matches(server, item, session, *link, args.get(b'f'))
        });
        let Some(other) = found.and_then(|link| server.winlinks.get(link).map(|l| l.window)) else {
            break;
        };
        let _ = operations::server_kill_window(server, other, false);
    }
    let links: Vec<_> = server
        .sessions
        .get(session)
        .map(|s| {
            s.windows
                .values()
                .copied()
                .filter(|link| {
                    server
                        .winlinks
                        .get(*link)
                        .is_some_and(|l| l.window == window)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut kill = false;
    for link in &links {
        if matches(server, item, session, *link, args.get(b'f')) {
            kill = true;
        }
    }
    if links.len() > 1 && kill {
        let _ = operations::server_kill_window(server, window, false);
    }
    operations::server_renumber_all(server);
    CmdReturn::Normal
}
