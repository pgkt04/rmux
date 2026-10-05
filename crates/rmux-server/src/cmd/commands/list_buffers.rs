// Ported from tmux cmd-list-buffers.c @ 8f25579c
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::format::{
    self, FormatContext,
    sort::{self, SortCriteria, SortOrder},
};
use crate::ids::QueueItemId;
use crate::server::Server;

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let order = sort::order_from_string(args.get(b'O'));
    if order == SortOrder::End && args.has(b'O') != 0 {
        queue::error(server, item, b"invalid sort order");
        return CmdReturn::Error;
    }
    let criteria = SortCriteria {
        order,
        reversed: args.has(b'r') != 0,
        ..SortCriteria::default()
    };
    let mut buffers = Vec::new();
    sort::get_buffers(server, &criteria, &mut buffers);
    let template = args
        .get(b'F')
        .unwrap_or(b"#{buffer_name}: #{buffer_size} bytes: \"#{buffer_sample}\"");
    for buffer in buffers {
        let mut tree = format::create_defaults(
            server,
            Some(item),
            FormatContext {
                buffer: Some(buffer),
                ..FormatContext::default()
            },
        );
        if args
            .get(b'f')
            .is_none_or(|filter| format::true_value(Some(&tree.expand(server, filter))))
        {
            let output = tree.expand(server, template);
            queue::print(server, item, &output);
        }
        tree.release(server);
    }
    CmdReturn::Normal
}
