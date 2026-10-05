// Ported from tmux cmd-list-windows.c @ 8f25579c
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
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let client = queued.client;
    let session = queued.target.s;
    let all = args.has(b'a') != 0;
    let mut links = Vec::new();
    if all {
        sort::get_winlinks(server, &criteria, &mut links);
    } else if let Some(session) = session {
        sort::get_winlinks_session(server, session, &criteria, &mut links);
    }
    let template = args.get(b'F').unwrap_or(if all { b"#{session_name}:#{window_index}: #{window_name}#{window_raw_flags} (#{window_panes} panes) [#{window_width}x#{window_height}] " } else { b"#{window_index}: #{window_name}#{window_raw_flags} (#{window_panes} panes) [#{window_width}x#{window_height}] [layout #{window_layout}] #{window_id}#{?window_active, (active),}" });
    let count = links.len();
    for link in links {
        let Some(wl) = server.winlinks.get(link) else {
            continue;
        };
        let context = FormatContext {
            evaluated_client: client,
            session: Some(wl.session),
            winlink: Some(link),
            window: Some(wl.window),
            ..FormatContext::default()
        };
        let mut tree = format::create_defaults(server, Some(item), context);
        tree.add(b"line", count.to_string().into());
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
