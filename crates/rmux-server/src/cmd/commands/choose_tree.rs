// Ported from tmux cmd-choose-tree.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    format::sort::{SortOrder, order_from_string},
    ids::QueueItemId,
    modes::{self, CommandModeRequest},
    server::Server,
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    if args.has(b'O') != 0 && order_from_string(args.get(b'O')) == SortOrder::End {
        queue::error(server, item, b"invalid sort order");
        return CmdReturn::Error;
    }
    if command.entry.name == b"choose-buffer" && server.paste.walk(None).is_none() {
        return CmdReturn::Normal;
    }
    if command.entry.name == b"choose-client" && crate::client::registry::how_many(server) == 0 {
        return CmdReturn::Normal;
    }
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let request = CommandModeRequest {
        command: command.entry.name.into(),
        args: args.copy(&[], &mut 0),
        target: queued.target,
        source: queued.source,
        client: queued.client,
        item,
    };
    modes::run_mode_command(server, request)
}
