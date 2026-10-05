// Ported from tmux cmd-command-prompt.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        arguments::{self, ArgsCommandState},
        queue::{self, CmdReturn},
    },
    ids::{ClientId, PaneId, QueueItemId},
    server::Server,
    ui::{
        prompt::{self, PanePromptInput, PromptFlags, PromptKeyResult, PromptResult, PromptType},
        status::{self, StatusPromptInput},
    },
};
use rmux_util::bytes::ByteString;

struct Prompt {
    item: Option<QueueItemId>,
    state: Option<ArgsCommandState>,
    flags: PromptFlags,
    prompts: Vec<(Vec<u8>, Vec<u8>)>,
    current: usize,
    answers: Vec<ByteString>,
    update: Option<(ByteString, ByteString)>,
}
impl Prompt {
    fn close(&mut self, server: &mut Server) {
        if let Some(item) = self.item.take() {
            queue::continue_item(&mut server.queue, item);
        }
    }
    fn free(&mut self, server: &mut Server) {
        self.close(server);
        if let Some(state) = self.state.take() {
            arguments::make_commands_free(server, state);
        }
    }
    fn answer(
        &mut self,
        server: &mut Server,
        client: Option<ClientId>,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        let Some(text) = text.filter(|_| key != PromptKeyResult::Move) else {
            self.close(server);
            return PromptResult::Close;
        };
        if key == PromptKeyResult::Close {
            if self.flags.contains(PromptFlags::INCREMENTAL) {
                self.close(server);
                return PromptResult::Close;
            }
            self.answers.push(text.into());
            self.current += 1;
            if let Some((message, input)) = self.prompts.get(self.current) {
                self.update = Some((message.as_slice().into(), input.as_slice().into()));
                return PromptResult::Continue;
            }
        }
        let mut answers = self.answers.clone();
        if key != PromptKeyResult::Close {
            answers.push(text.into());
        }
        if let Some(state) = self.state.as_ref() {
            match arguments::make_commands(server, state, &answers) {
                Ok(list) => {
                    let state = self
                        .item
                        .and_then(|item| server.queue.items.get(item).map(|i| i.state));
                    if let Ok(batch) = server.queue.get_command(list, state) {
                        if let Some(item) = self.item {
                            queue::insert_after(server, item, batch)
                                .expect("live waiting prompt queue item");
                        } else {
                            queue::append(server, client, batch).expect("live prompt client queue");
                        }
                    }
                }
                Err(cause) => {
                    if let Ok(batch) = server.queue.get_error(cause.message()) {
                        queue::append(server, client, batch).expect("live prompt error queue");
                    }
                }
            }
        }
        if self.flags.contains(PromptFlags::INCREMENTAL) {
            PromptResult::Continue
        } else {
            self.close(server);
            PromptResult::Close
        }
    }
}
impl StatusPromptInput for Prompt {
    fn fire(
        &mut self,
        server: &mut Server,
        client: ClientId,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        self.answer(server, Some(client), text, key)
    }
    fn take_update(&mut self) -> Option<(ByteString, ByteString)> {
        self.update.take()
    }
    fn free(&mut self, server: &mut Server) {
        Prompt::free(self, server);
    }
}
impl PanePromptInput for Prompt {
    fn fire(
        &mut self,
        server: &mut Server,
        _: PaneId,
        client: Option<ClientId>,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        self.answer(server, client, text, key)
    }
    fn take_update(&mut self) -> Option<(ByteString, ByteString)> {
        self.update.take()
    }
    fn free(&mut self, server: &mut Server) {
        Prompt::free(self, server);
    }
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let Some(client) = queued.target_client else {
        return CmdReturn::Normal;
    };
    let target = queued.target;
    let pane = if args.has(b'P') != 0 { target.wp } else { None };
    if args.has(b'P') != 0 {
        if pane.is_none()
            || pane
                .and_then(|pane| server.panes.get(pane))
                .is_some_and(|p| p.prompt.is_some())
        {
            return CmdReturn::Normal;
        }
    } else if server
        .clients
        .get(client)
        .is_some_and(|c| c.prompt.is_some())
    {
        return CmdReturn::Normal;
    }
    let wait = args.has(b'b') == 0 && args.has(b'i') == 0;
    let state = arguments::make_commands_prepare(
        server,
        command,
        item,
        0,
        Some(b"%1"),
        wait,
        args.has(b'F') != 0,
    );
    let mut space = true;
    let messages = args.get(b'p').map(<[u8]>::to_vec).unwrap_or_else(|| {
        if args.count() != 0 {
            let mut message = b"(".to_vec();
            message.extend_from_slice(&arguments::make_commands_get_command(&state));
            message.push(b')');
            message
        } else {
            space = false;
            b":".to_vec()
        }
    });
    let inputs = args.get(b'I').unwrap_or_default();
    let prompts = if args.has(b'l') != 0 {
        vec![(messages, inputs.to_vec())]
    } else {
        let mut inputs = inputs.split(|b| *b == b',');
        messages
            .split(|b| *b == b',')
            .map(|message| {
                let mut message = message.to_vec();
                if space {
                    message.push(b' ');
                }
                (message, inputs.next().unwrap_or_default().to_vec())
            })
            .collect()
    };
    let ty = args
        .get(b'T')
        .map(prompt::prompt_type)
        .unwrap_or(PromptType::Command);
    if ty == PromptType::Invalid {
        let mut cause = b"unknown type: ".to_vec();
        cause.extend_from_slice(args.get(b'T').unwrap_or_default());
        queue::error(server, item, &cause);
        arguments::make_commands_free(server, state);
        return CmdReturn::Error;
    }
    let mut flags = if args.has(b'1') != 0 {
        PromptFlags::SINGLE
    } else if args.has(b'N') != 0 {
        PromptFlags::NUMERIC
    } else if args.has(b'i') != 0 {
        PromptFlags::INCREMENTAL
    } else if args.has(b'k') != 0 {
        PromptFlags::KEY
    } else if args.has(b'e') != 0 {
        PromptFlags::BSPACE_EXIT
    } else {
        PromptFlags::default()
    };
    if args.has(b'C') != 0 {
        flags.insert(PromptFlags::NOFREEZE);
    }
    if pane.is_some() {
        flags.insert(PromptFlags::ISPANE);
    }
    let continuation = Prompt {
        item: wait.then_some(item),
        state: Some(state),
        flags,
        prompts,
        current: 0,
        answers: Vec::new(),
        update: None,
    };
    let (message, input) = continuation.prompts[0].clone();
    if let Some(pane) = pane {
        prompt::pane_prompt_set(
            server,
            pane,
            client,
            Some(&target),
            &message,
            Some(&input),
            Some(Box::new(continuation)),
            flags,
            ty,
        );
    } else {
        status::status_prompt_set(
            server,
            client,
            Some(&target),
            &message,
            Some(&input),
            Box::new(continuation),
            flags,
            ty,
        );
    }
    if wait {
        CmdReturn::Wait
    } else {
        CmdReturn::Normal
    }
}
