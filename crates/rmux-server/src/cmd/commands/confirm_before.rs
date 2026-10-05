// Ported from tmux cmd-confirm-before.c @ 8f25579c
use crate::{
    client::ClientFlags,
    cmd::{
        Command, CommandList, arguments,
        queue::{self, CmdReturn},
    },
    ids::{ClientId, QueueItemId},
    server::Server,
    ui::{
        prompt::{PromptFlags, PromptKeyResult, PromptResult, PromptType},
        status::{self, StatusPromptInput},
    },
};
use std::rc::Rc;

struct Confirmation {
    item: Option<QueueItemId>,
    list: Rc<CommandList>,
    key: u8,
    default_yes: bool,
}
impl StatusPromptInput for Confirmation {
    fn free(&mut self, server: &mut Server) {
        if let Some(item) = self.item.take() {
            queue::continue_item(&mut server.queue, item);
        }
    }
    fn fire(
        &mut self,
        server: &mut Server,
        client: ClientId,
        text: Option<&[u8]>,
        _: PromptKeyResult,
    ) -> PromptResult {
        let yes = server
            .clients
            .get(client)
            .is_some_and(|c| !c.flags.contains(ClientFlags::DEAD))
            && text
                .and_then(|s| s.first())
                .is_some_and(|key| *key == self.key || (*key == b'\r' && self.default_yes));
        if yes {
            let state = self
                .item
                .and_then(|item| server.queue.items.get(item).map(|i| i.state));
            if let Ok(batch) = server.queue.get_command(self.list.clone(), state) {
                if let Some(item) = self.item {
                    queue::insert_after(server, item, batch)
                        .expect("live waiting confirmation item");
                } else {
                    queue::append(server, Some(client), batch)
                        .expect("live confirmation client queue");
                }
            }
        }
        if let Some(item) = self.item.take() {
            if let Some(c) = server
                .queue
                .items
                .get(item)
                .and_then(|i| i.client)
                .and_then(|c| server.clients.get_mut(c))
                .filter(|c| c.session.is_none())
            {
                c.retval = i32::from(!yes);
            }
            queue::continue_item(&mut server.queue, item);
        }
        PromptResult::Close
    }
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let Some(list) = arguments::make_commands_now(server, command, item, 0, true) else {
        return CmdReturn::Error;
    };
    let key = command.args.get(b'c').unwrap_or(b"y");
    if key.len() != 1 || key[0] <= 31 || key[0] >= 127 {
        queue::error(server, item, b"invalid confirm key");
        return CmdReturn::Error;
    }
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let Some(client) = queued.target_client else {
        return CmdReturn::Normal;
    };
    let target = queued.target;
    let wait = command.args.has(b'b') == 0;
    let mut prompt = if let Some(prompt) = command.args.get(b'p') {
        prompt.to_vec()
    } else {
        let name = list
            .commands
            .first()
            .map(|c| c.entry.name)
            .unwrap_or_default();
        let mut prompt = b"Confirm '".to_vec();
        prompt.extend_from_slice(name);
        prompt.extend_from_slice(b"'? (");
        prompt.push(key[0]);
        prompt.extend_from_slice(b"/n)");
        prompt
    };
    prompt.push(b' ');
    status::status_prompt_set(
        server,
        client,
        Some(&target),
        &prompt,
        None,
        Box::new(Confirmation {
            item: wait.then_some(item),
            list,
            key: key[0],
            default_yes: command.args.has(b'y') != 0,
        }),
        PromptFlags::SINGLE,
        PromptType::Command,
    );
    if wait {
        CmdReturn::Wait
    } else {
        CmdReturn::Normal
    }
}
