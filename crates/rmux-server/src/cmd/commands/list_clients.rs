// Ported from tmux cmd-list-clients.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    format::{
        self, FormatContext,
        sort::{self, SortCriteria, SortOrder},
    },
    ids::QueueItemId,
    server::Server,
};

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
    let session = if args.has(b't') != 0 {
        server.queue.items.get(item).and_then(|i| i.target.s)
    } else {
        None
    };
    let mut clients = Vec::new();
    sort::get_clients(server, &criteria, &mut clients);
    let template = args.get(b'F').unwrap_or(b"#{client_name}: #{session_name} [#{client_width}x#{client_height} #{client_termname}] #{?#{!=:#{client_uid},#{uid}},[user #{?client_user,#{client_user},#{client_uid},}] ,}#{?client_flags,(,}#{client_flags}#{?client_flags,),}");
    for (line, client) in clients.into_iter().enumerate() {
        if server
            .clients
            .get(client)
            .is_none_or(|c| c.session.is_none() || session.is_some_and(|s| c.session != Some(s)))
        {
            continue;
        }
        let mut tree = format::create_defaults(
            server,
            Some(item),
            FormatContext {
                evaluated_client: Some(client),
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
