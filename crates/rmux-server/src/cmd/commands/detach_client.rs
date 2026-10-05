// Ported from tmux cmd-detach-client.c @ 8f25579c
use crate::{
    client::{self, ClientFlags},
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    ids::{ClientId, QueueItemId},
    server::Server,
};

fn detach(server: &mut Server, id: ClientId, command: &Command) {
    if let Some(shell) = command.args.get(b'E') {
        client::lifecycle::exec(server, id, shell);
    } else {
        client::lifecycle::detach(server, id, command.args.has(b'P') != 0);
    }
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let caller = queued.client;
    let target = queued.target_client;
    let source = queued.source.s;
    if command.entry.name == b"suspend-client" {
        if let Some(target) = target {
            client::lifecycle::suspend(server, target);
        }
        return CmdReturn::Normal;
    }
    if caller
        .and_then(|c| server.clients.get(c))
        .is_some_and(|c| c.flags.contains(ClientFlags::READONLY))
        && (command.args.has(b's') != 0 || command.args.has(b'a') != 0 || caller != target)
    {
        queue::error(server, item, b"client is read-only");
        return CmdReturn::Error;
    }
    if command.args.has(b's') != 0 {
        let Some(source) = source else {
            return CmdReturn::Normal;
        };
        let clients: Vec<_> = server
            .client_order
            .iter()
            .copied()
            .filter(|id| {
                server
                    .clients
                    .get(*id)
                    .is_some_and(|c| c.session == Some(source))
            })
            .collect();
        for id in clients {
            detach(server, id, command);
        }
        return CmdReturn::Stop;
    }
    if command.args.has(b'a') != 0 {
        let clients: Vec<_> = server
            .client_order
            .iter()
            .copied()
            .filter(|id| {
                Some(*id) != target && server.clients.get(*id).is_some_and(|c| c.session.is_some())
            })
            .collect();
        for id in clients {
            detach(server, id, command);
        }
        return CmdReturn::Normal;
    }
    if let Some(target) = target {
        detach(server, target, command);
    }
    CmdReturn::Stop
}
