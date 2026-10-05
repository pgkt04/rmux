// Ported from tmux cmd-set-buffer.c @ 8f25579c
use super::support::{concat, fail, item_target_client};
use crate::cmd::metadata::CMD_DELETE_BUFFER;
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::QueueItemId;
use crate::model::paste::{
    paste_buffer_name, paste_get_name, paste_get_top, paste_remove, paste_rename, paste_set,
};
use crate::model::state::ModelError;
use crate::server::Server;
use rmux_util::bytes::cstr;

/// The `cause` text of a paste failure, byte-exact for `cmdq_error(item, "%s", cause)`.
fn cause_bytes(error: ModelError) -> Vec<u8> {
    match error {
        ModelError::Message(bytes) => bytes,
        other => other.to_string().into_bytes(),
    }
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let tc = item_target_client(server, item);

    let mut bufname: Option<Vec<u8>> = args.get(b'b').map(<[u8]>::to_vec);
    let mut pb = bufname
        .as_deref()
        .and_then(|name| paste_get_name(server, name));

    let is_delete = std::ptr::eq(command.entry, &CMD_DELETE_BUFFER);
    if is_delete || args.has(b'n') != 0 {
        // cmd-set-buffer.c:70-78, 87-95: fall back to the top buffer and its name.
        if pb.is_none() {
            if let Some(name) = &bufname {
                return fail(server, item, concat(&[b"unknown buffer: ", name]));
            }
            pb = paste_get_top(server);
            bufname = pb
                .and_then(|id| paste_buffer_name(server, id))
                .map(<[u8]>::to_vec);
        }
        let Some(pb) = pb else {
            return fail(server, item, b"no buffer");
        };
        if is_delete {
            if let Err(e) = paste_remove(server, pb) {
                return fail(server, item, cause_bytes(e));
            }
            return CmdReturn::Normal;
        }
        let newname = args.get(b'n').unwrap_or_default();
        if let Err(cause) = paste_rename(server, bufname.as_deref(), newname) {
            return fail(server, item, cause_bytes(cause));
        }
        return CmdReturn::Normal;
    }

    if args.count() != 1 {
        return fail(server, item, b"no data specified");
    }
    let newdata = cstr(args.string(0).unwrap_or_default());
    if newdata.is_empty() {
        return CmdReturn::Normal;
    }

    let mut bufdata = Vec::with_capacity(newdata.len());
    if args.has(b'a') != 0
        && let Some(pb) = pb
        && let Some(old) = server.paste.get(pb)
    {
        bufdata.extend_from_slice(&old.data);
    }
    bufdata.extend_from_slice(newdata);

    let limit = server
        .options
        .get_number(server.options.global, b"buffer-limit") as u32;
    let set = match paste_set(server, bufdata, bufname.as_deref(), limit) {
        Ok(set) => set,
        Err(e) => return fail(server, item, cause_bytes(e.error)),
    };

    if args.has(b'w') != 0
        && let Some(tc) = tc
        && let Some(id) = set
        && let Some(buffer) = server.paste.get(id)
        && let Some(tty) = server.clients.get_mut(tc).and_then(|c| c.tty.as_mut())
    {
        tty.set_selection(&mut server.tparm, "", &buffer.data);
    }
    CmdReturn::Normal
}
