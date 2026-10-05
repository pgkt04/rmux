// Ported from tmux cmd-lock-server.c @ 8f25579c
use crate::{
    cmd::{Command, queue::CmdReturn},
    ids::QueueItemId,
    model::ModelEffect,
    server::{Server, operations},
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let session = queued.target.s;
    let client = queued.target_client;
    match command.entry.name {
        b"lock-server" => {
            let _ = operations::lock(server);
        }
        b"lock-session" => {
            if let Some(session) = session {
                let _ = operations::lock_session(server, session);
            }
        }
        _ => {
            if let Some(client) = client {
                let _ = operations::lock_client(server, client);
            }
        }
    }
    server.effects.push_back(ModelEffect::RecalculateSizes);
    CmdReturn::Normal
}
