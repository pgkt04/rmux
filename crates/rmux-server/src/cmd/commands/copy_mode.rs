// Ported from tmux cmd-copy-mode.c @ 8f25579c
use crate::{
    cmd::{Command, queue::CmdReturn},
    ids::QueueItemId,
    modes::{self, CommandModeRequest},
    server::Server,
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let target = queued.target;
    let source = queued.source;
    let client = queued.client;
    if command.args.has(b'q') != 0 {
        if let Some(pane) = target.wp {
            let _ = crate::model::pane::pane_reset_mode_all(server, pane);
        }
        return CmdReturn::Normal;
    }
    modes::run_mode_command(
        server,
        CommandModeRequest {
            command: command.entry.name.into(),
            args: command.args.copy(&[], &mut 0),
            target,
            source,
            client,
            item,
        },
    )
}
