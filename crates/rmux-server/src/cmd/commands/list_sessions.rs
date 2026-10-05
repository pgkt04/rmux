// Ported from tmux cmd-list-sessions.c @ 8f25579c
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
    let mut sessions = Vec::new();
    sort::get_sessions(server, &criteria, &mut sessions);
    let client = server.queue.items.get(item).and_then(|i| i.client);
    let template = args.get(b'F').unwrap_or(b"#{session_name}: #{session_windows} windows (created #{t:session_created})#{?session_grouped, (group ,}#{session_group}#{?session_grouped,),}#{?session_attached, (attached),}");
    for (line, session) in sessions.into_iter().enumerate() {
        let mut tree = format::create_defaults(
            server,
            Some(item),
            FormatContext {
                evaluated_client: client,
                session: Some(session),
                ..FormatContext::default()
            },
        );
        tree.add(b"line", line.to_string().into());
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
