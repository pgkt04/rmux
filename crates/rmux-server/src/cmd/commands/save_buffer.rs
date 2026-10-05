// Ported from tmux cmd-save-buffer.c @ 8f25579c
use super::support::{concat, fail, item_client};
use crate::client::ClientFlags;
use crate::cmd::metadata::CMD_SHOW_BUFFER;
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::format;
use crate::ids::QueueItemId;
use crate::model::paste::{paste_get_name, paste_get_top};
use crate::server::Server;
use crate::server::file::{self, ClientFileCallback, FileNotice};
use rmux_util::buffer::ByteBuffer;

/// cmd-save-buffer.c:58-69 (`cmd_save_buffer_done`): wait for the terminal
/// notice, report `<strerror>: <path>`, continue the item exactly once.
fn done_callback(item: QueueItemId, path: Vec<u8>) -> ClientFileCallback {
    let mut finished = false;
    Box::new(move |server: &mut Server, _file, notice, _data, error| {
        if notice != FileNotice::Done || finished {
            return;
        }
        finished = true;
        if error != 0 {
            queue::error(
                server,
                item,
                &concat(&[&rmux_sys::strerror(error), b": ", &path]),
            );
        }
        queue::continue_item(&mut server.queue, item);
    })
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let c = item_client(server, item);

    let pb = match args.get(b'b') {
        None => match paste_get_top(server) {
            Some(pb) => pb,
            None => return fail(server, item, b"no buffers"),
        },
        Some(bufname) => match paste_get_name(server, bufname) {
            Some(pb) => pb,
            None => return fail(server, item, concat(&[b"no buffer ", bufname])),
        },
    };
    let bufdata = server
        .paste
        .get(pb)
        .map(|b| b.data.clone())
        .unwrap_or_default();

    let path: Vec<u8> = if std::ptr::eq(command.entry, &CMD_SHOW_BUFFER) {
        let direct = c
            .and_then(|c| server.clients.get(c))
            .is_some_and(|c| c.session.is_some() || c.flags.contains(ClientFlags::CONTROL));
        if direct {
            let mut evb = ByteBuffer::new();
            evb.add(&bufdata);
            queue::print_data(server, item, &evb);
            return CmdReturn::Normal;
        }
        b"-".to_vec()
    } else {
        format::single_from_target(server, item, args.string(0).unwrap_or_default()).into_vec()
    };

    let flags = if args.has(b'a') != 0 {
        libc::O_APPEND
    } else {
        libc::O_TRUNC
    };
    let callback = done_callback(item, path.clone());
    file::write(
        server,
        c,
        &path,
        flags,
        bufdata.as_slice().to_vec(),
        callback,
    );
    CmdReturn::Wait
}
