// Ported from tmux cmd-move-window.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        find::{self, CmdFindFlags, CmdFindType, FindContext},
        queue::{self, CmdReturn},
    },
    ids::QueueItemId,
    model,
    server::{Server, operations, run},
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let source = queued.source;
    let context = FindContext {
        client: queued.client,
        current: server
            .queue
            .states
            .get(queued.state)
            .map(|s| s.current)
            .unwrap_or_default(),
        ..FindContext::default()
    };
    let renumber = args.has(b'r') != 0;
    let target = match find::target(
        server,
        &context,
        args.get(b't'),
        if renumber {
            CmdFindType::Session
        } else {
            CmdFindType::Window
        },
        if renumber {
            CmdFindFlags::QUIET
        } else {
            CmdFindFlags::WINDOW_INDEX
        },
    ) {
        Ok(target) => target,
        Err(cause) => {
            if let Some(cause) = cause.message {
                queue::error(server, item, &cause);
            }
            return CmdReturn::Error;
        }
    };
    let Some(dst) = target.s else {
        return CmdReturn::Error;
    };
    if renumber {
        model::session::session_renumber_windows(server, dst);
        run::recalculate_sizes(server);
        operations::server_status_session(server, dst);
        return CmdReturn::Normal;
    }
    let (Some(src), Some(link)) = (source.s, source.wl) else {
        return CmdReturn::Error;
    };
    let mut index = target.idx;
    if args.has(b'a') != 0 || args.has(b'b') != 0 {
        let around = target
            .wl
            .or_else(|| server.sessions.get(dst).and_then(|s| s.current));
        let Some(shuffled) =
            model::winlink::winlink_shuffle_up(server, dst, around, args.has(b'b') != 0)
        else {
            return CmdReturn::Error;
        };
        index = shuffled;
    }
    if let Err(cause) = operations::server_link_window(
        server,
        src,
        link,
        dst,
        index,
        args.has(b'k') != 0,
        args.has(b'd') == 0,
    ) {
        queue::error(server, item, &cause);
        return CmdReturn::Error;
    }
    if command.entry.name == b"move-window" {
        let _ = operations::server_unlink_window(server, src, link);
    }
    if args.has(b's') == 0
        && server
            .sessions
            .get(src)
            .is_some_and(|s| server.options.get_number(s.options, b"renumber-windows") != 0)
    {
        model::session::session_renumber_windows(server, src);
    }
    run::recalculate_sizes(server);
    CmdReturn::Normal
}
