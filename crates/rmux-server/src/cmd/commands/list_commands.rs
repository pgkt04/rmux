// Ported from tmux cmd-list-commands.c @ 8f25579c
use crate::cmd::{
    self, Command,
    queue::{self, CmdReturn},
};
use crate::format::{self, FormatContext};
use crate::ids::QueueItemId;
use crate::server::Server;

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let entries = if let Some(name) = command.args.string(0) {
        match cmd::find_entry(name) {
            Ok(entry) => vec![entry],
            Err(cause) => {
                queue::error(server, item, &cause);
                return CmdReturn::Error;
            }
        }
    } else {
        cmd::COMMAND_TABLE.to_vec()
    };
    let template = command.args.get(b'F').unwrap_or(b"#{command_list_name}#{?command_list_alias, (#{command_list_alias}),} #{command_list_usage}");
    let mut tree = format::create_defaults(server, Some(item), FormatContext::default());
    for entry in entries {
        tree.add(b"command_list_name", entry.name.into());
        tree.add(b"command_list_alias", entry.alias.unwrap_or(b"").into());
        tree.add(b"command_list_usage", entry.usage.into());
        let output = tree.expand(server, template);
        if !output.is_empty() {
            queue::print(server, item, &output);
        }
    }
    tree.release(server);
    CmdReturn::Normal
}
