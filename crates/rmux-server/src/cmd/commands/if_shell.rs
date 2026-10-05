// Ported from tmux cmd-if-shell.c @ 8f25579c
use crate::{
    cmd::{
        Command, arguments,
        queue::{self, CmdReturn},
    },
    format,
    ids::QueueItemId,
    server::{
        Server,
        job::{self, JobCommand, JobLaunch},
    },
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let condition =
        format::single_from_target(server, item, command.args.string(0).unwrap_or_default());
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let state = queued.state;
    let client = queued.client;
    let tc = queued.target_client;
    let session = queued.target.s;
    if command.args.has(b'F') != 0 {
        let index = if condition.first().is_some_and(|byte| *byte != b'0') {
            1
        } else if command.args.count() == 3 {
            2
        } else {
            return CmdReturn::Normal;
        };
        let Some(list) = arguments::make_commands_now(server, command, item, index, false) else {
            return CmdReturn::Error;
        };
        if let Ok(batch) = server.queue.get_command(list, Some(state)) {
            queue::insert_after(server, item, batch).expect("executing condition queue item");
        }
        return CmdReturn::Normal;
    }
    let wait = command.args.has(b'b') == 0;
    let yes = arguments::make_commands_prepare(server, command, item, 1, None, wait, false);
    let no = if command.args.count() == 3 {
        Some(arguments::make_commands_prepare(
            server, command, item, 2, None, wait, false,
        ))
    } else {
        None
    };
    let retained = if wait { client } else { tc };
    if let Some(client) = retained {
        let _ = server.clients.retain(client);
    }
    let prepared = std::rc::Rc::new(std::cell::RefCell::new(Some((yes, no))));
    let completion = prepared.clone();
    let mut launch = JobLaunch::new(JobCommand::Shell(condition.to_vec()));
    launch.session = session;
    launch.cwd = Some(crate::client::registry::get_cwd(server, client, session));
    let cancelled = prepared.clone();
    launch.free = Some(Box::new(move |server, _| {
        if let Some((yes, no)) = cancelled.borrow_mut().take() {
            arguments::make_commands_free(server, yes);
            if let Some(no) = no {
                arguments::make_commands_free(server, no);
            }
            if let Some(client) = retained {
                let _ = server.clients.release(client);
            }
            if wait {
                queue::continue_item(&mut server.queue, item);
            }
        }
    }));
    launch.complete = Some(Box::new(move |server, job| {
        let Some((yes, no)) = completion.borrow_mut().take() else {
            return;
        };
        let success = server.jobs.get(job).is_some_and(|job| job.status == 0);
        let branch = if success { Some(&yes) } else { no.as_ref() };
        if let Some(branch) = branch {
            match arguments::make_commands(server, branch, &[]) {
                Ok(list) => {
                    if let Ok(batch) = server
                        .queue
                        .get_command(list, if wait { Some(state) } else { None })
                    {
                        if wait {
                            queue::insert_after(server, item, batch)
                                .expect("waiting condition queue item");
                        } else {
                            queue::append(server, retained, batch)
                                .expect("retained condition client queue");
                        }
                    }
                }
                Err(cause) => {
                    if wait {
                        queue::error(server, item, cause.message());
                    } else {
                        let mut message = cause.message().to_vec();
                        if let Some(first) = message.first_mut() {
                            *first = first.to_ascii_uppercase();
                        }
                        crate::ui::status::status_message_set(
                            server, retained, -1, true, false, false, &message,
                        );
                    }
                }
            }
        }
        arguments::make_commands_free(server, yes);
        if let Some(no) = no {
            arguments::make_commands_free(server, no);
        }
        if let Some(client) = retained {
            let _ = server.clients.release(client);
        }
        if wait {
            queue::continue_item(&mut server.queue, item);
        }
    }));
    match job::run(server, launch) {
        Ok(_) => {
            if wait {
                CmdReturn::Wait
            } else {
                CmdReturn::Normal
            }
        }
        Err(_) => {
            if let Some((yes, no)) = prepared.borrow_mut().take() {
                arguments::make_commands_free(server, yes);
                if let Some(no) = no {
                    arguments::make_commands_free(server, no);
                }
            }
            if let Some(client) = retained {
                let _ = server.clients.release(client);
            }
            let mut cause = b"failed to run command: ".to_vec();
            cause.extend_from_slice(&condition);
            queue::error(server, item, &cause);
            CmdReturn::Error
        }
    }
}
