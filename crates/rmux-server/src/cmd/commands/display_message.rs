// Ported from tmux cmd-display-message.c @ 8f25579c
use crate::{
    client::ClientFlags,
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    format::{self, FormatContext, FormatFlags},
    ids::QueueItemId,
    server::Server,
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let target = queued.target;
    let owner = queued.client;
    let tc = queued.target_client;
    if args.has(b'I') != 0 && args.has(b'j') == 0 {
        let Some(pane) = target.wp else {
            return CmdReturn::Normal;
        };
        return match crate::server::file::start_pane_input(server, pane, item) {
            Ok(result) => result,
            Err(cause) => {
                queue::error(server, item, &cause);
                CmdReturn::Error
            }
        };
    }
    if args.has(b'F') != 0 && args.count() != 0 {
        queue::error(server, item, b"only one of -F or argument must be given");
        return CmdReturn::Error;
    }
    let delay = if args.has(b'd') != 0 {
        match args.strtonum(b'd', 0, u32::MAX as i64) {
            Ok(delay) => delay as i32,
            Err(cause) => {
                let mut message = b"delay ".to_vec();
                message.extend_from_slice(&cause);
                queue::error(server, item, &message);
                return CmdReturn::Error;
            }
        }
    } else {
        -1
    };
    let template = args.string(0).or_else(|| args.get(b'F')).unwrap_or(if args.has(b'j') != 0 { b"" } else { b"[#{session_name}] #{window_index}:#{window_name}, current pane #{pane_index} - (%H:%M %d-%b-%y)" });
    let evaluated = tc
        .filter(|id| {
            args.has(b'c') != 0
                || server
                    .clients
                    .get(*id)
                    .is_some_and(|c| c.session == target.s)
        })
        .or_else(|| {
            server.client_order.iter().copied().find(|id| {
                server
                    .clients
                    .get(*id)
                    .is_some_and(|c| c.session == target.s && c.session.is_some())
            })
        });
    let mut tree = format::FormatTree::create(
        owner,
        Some(item),
        0,
        if args.has(b'v') != 0 {
            FormatFlags::VERBOSE
        } else {
            FormatFlags::NONE
        },
        server,
    );
    tree.defaults(
        server,
        FormatContext {
            evaluated_client: evaluated,
            session: target.s,
            winlink: target.wl,
            window: target.w,
            pane: target.wp,
            ..FormatContext::default()
        },
    );
    if args.has(b'a') != 0 && args.has(b'j') == 0 {
        let mut lines = Vec::new();
        tree.each(server, |key, value| {
            let mut line = key.to_vec();
            line.push(b'=');
            line.extend_from_slice(value);
            lines.push(line);
        });
        for line in lines {
            queue::print(server, item, &line);
        }
        tree.release(server);
        return CmdReturn::Normal;
    }
    let mut output = if args.has(b'l') != 0 {
        template.into()
    } else {
        tree.expand_time(server, template)
    };
    if args.has(b'j') != 0 {
        output = match format::json::parse(&output) {
            Ok(node) => node.to_string(),
            Err(cause) => {
                queue::error(server, item, cause.cause());
                tree.release(server);
                return CmdReturn::Error;
            }
        };
    }
    if owner.is_none() {
        queue::error(server, item, &output);
    } else if args.has(b'p') != 0 {
        queue::print(server, item, &output);
    } else if let Some(tc) = tc {
        if server
            .clients
            .get(tc)
            .is_some_and(|c| c.flags.contains(ClientFlags::CONTROL))
        {
            let mut message = b"%message ".to_vec();
            message.extend_from_slice(&output);
            crate::client::print::print(server, Some(tc), false, &message);
        } else {
            crate::ui::status::status_message_set(
                server,
                Some(tc),
                delay,
                false,
                args.has(b'N') != 0,
                args.has(b'C') != 0,
                &output,
            );
        }
    }
    tree.release(server);
    CmdReturn::Normal
}
