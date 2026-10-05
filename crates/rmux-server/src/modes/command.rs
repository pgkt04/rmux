// Ported from tmux cmd-choose-tree.c, cmd-copy-mode.c @ 8f25579c
use crate::{
    cmd::{
        arguments::Args,
        find::CmdFindState,
        queue::{self, CmdReturn},
    },
    ids::{ClientId, QueueItemId},
    server::Server,
};
use rmux_util::bytes::ByteString;

pub struct CommandModeRequest {
    pub command: ByteString,
    pub args: Args,
    pub target: CmdFindState,
    pub source: CmdFindState,
    pub client: Option<ClientId>,
    pub item: QueueItemId,
}

pub fn unavailable_message(command: &[u8]) -> ByteString {
    let mut message = ByteString::from(command);
    message.extend_from_slice(b": mode not available yet");
    message
}

pub fn run_mode_command(server: &mut Server, request: CommandModeRequest) -> CmdReturn {
    queue::error(server, request.item, &unavailable_message(&request.command));
    CmdReturn::Error
}

#[cfg(test)]
mod tests {
    #[test]
    fn wave_b_mode_error_is_explicit() {
        assert_eq!(
            super::unavailable_message(b"copy-mode").as_ref(),
            b"copy-mode: mode not available yet"
        );
    }
}
