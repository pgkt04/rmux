// Ported from tmux cmd-list-panes.c @ 8f25579c
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
    let target = queued.target;
    let scope = if args.has(b'a') != 0 {
        2
    } else if args.has(b's') != 0 {
        1
    } else {
        0
    };
    let sessions: Vec<_> = if scope == 2 {
        server.session_names.values().copied().collect()
    } else {
        target.s.into_iter().collect()
    };
    for session in sessions {
        let links: Vec<_> = if scope == 0 {
            target.wl.into_iter().collect()
        } else {
            server
                .sessions
                .get(session)
                .map(|s| s.windows.values().copied().collect())
                .unwrap_or_default()
        };
        for link in links {
            let Some(window) = server.winlinks.get(link).map(|l| l.window) else {
                continue;
            };
            let mut panes = Vec::new();
            sort::get_panes_window(server, window, &criteria, &mut panes);
            let count = panes.len();
            let prefix: &[u8] = match scope {
                2 => b"#{session_name}:#{window_index}.",
                1 => b"#{window_index}.",
                _ => b"",
            };
            let mut default_template = prefix.to_vec();
            default_template.extend_from_slice(b"#{pane_index}: [#{pane_width}x#{pane_height}#{?pane_floating_flag, #{pane_x}#,#{pane_y}#,#{pane_z}}] [history #{history_size}/#{history_limit}, #{history_bytes} bytes] #{pane_id}#{?pane_active, (active),}#{?pane_dead, (dead),}");
            let template = args.get(b'F').unwrap_or(&default_template);
            for pane in panes {
                let mut tree = format::create_defaults(
                    server,
                    Some(item),
                    FormatContext {
                        evaluated_client: client,
                        session: Some(session),
                        winlink: Some(link),
                        window: Some(window),
                        pane: Some(pane),
                        ..FormatContext::default()
                    },
                );
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
        }
    }
    CmdReturn::Normal
}
