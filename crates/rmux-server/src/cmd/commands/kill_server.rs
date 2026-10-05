// Ported from tmux cmd-kill-server.c @ 8f25579c
use crate::{
    cmd::{Command, queue::CmdReturn},
    ids::QueueItemId,
    server::Server,
};

pub fn execute(_: &mut Server, command: &Command, _: QueueItemId) -> CmdReturn {
    if command.entry.name == b"kill-server" {
        // kill(getpid(), SIGTERM): handled after this queue pass, so the
        // rest of the command list still runs before the shutdown.
        let _ = rmux_sys::client::raise(rmux_sys::client::SIGTERM);
    }
    CmdReturn::Normal
}
