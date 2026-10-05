// Ported from tmux cmd-load-buffer.c @ 8f25579c
use crate::{
    client::ClientFlags,
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    format,
    ids::QueueItemId,
    model::paste::paste_set,
    server::{
        Server,
        file::{self, FileNotice},
    },
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let client = queued.client;
    let clipboard = if command.args.has(b'w') != 0 {
        queued.target_client
    } else {
        None
    };
    if let Some(client) = clipboard {
        let _ = server.clients.retain(client);
    }
    let name = command.args.get(b'b').map(<[u8]>::to_vec);
    let path = format::single_from_target(server, item, command.args.string(0).unwrap_or_default());
    let callback_path = path.clone();
    file::read(
        server,
        client,
        &path,
        Box::new(move |server, _, notice, data, error| {
            if notice != FileNotice::Done {
                return;
            }
            if error != 0 {
                let mut cause = rmux_sys::strerror(error);
                cause.extend_from_slice(b": ");
                cause.extend_from_slice(&callback_path);
                queue::error(server, item, &cause);
            } else if !data.is_empty() {
                let limit = server
                    .options
                    .get_number(server.options.global, b"buffer-limit")
                    as u32;
                match paste_set(server, data.to_vec(), name.as_deref(), limit) {
                    Ok(buffer) => {
                        if let Some(client) = clipboard {
                            if let Some(c) = server.clients.get_mut(client).filter(|c| {
                                c.session.is_some() && !c.flags.contains(ClientFlags::DEAD)
                            }) {
                                if let (Some(tty), Some(buffer)) =
                                    (c.tty.as_mut(), buffer.and_then(|id| server.paste.get(id)))
                                {
                                    tty.set_selection(&mut server.tparm, "", &buffer.data);
                                }
                            }
                        }
                    }
                    Err(cause) => queue::error(server, item, cause.error.to_string().as_bytes()),
                }
            }
            if let Some(client) = clipboard {
                let _ = server.clients.release(client);
            }
            queue::continue_item(&mut server.queue, item);
        }),
    );
    CmdReturn::Wait
}
