// Ported from tmux cmd-new-window.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        find::CmdFindState,
        queue::{self, CmdReturn},
    },
    format::{self, FormatContext},
    ids::QueueItemId,
    model::{
        self,
        spawn::{SpawnContext, SpawnFlags},
        state::clean_name,
    },
    server::{Server, operations},
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let target = queued.target;
    let client = queued.target_client;
    let Some(session) = target.s else {
        return CmdReturn::Error;
    };
    let argv: Vec<_> = args
        .values()
        .iter()
        .map(|value| value.as_string().to_vec())
        .collect();
    let empty = argv.len() == 1 && argv[0].is_empty();
    if args.has(b'E') != 0 && !argv.is_empty() && !empty {
        queue::error(server, item, b"command cannot be given for empty pane");
        return CmdReturn::Error;
    }
    let name = if let Some(name) = args.get(b'n') {
        let expanded = format::single_from_target(server, item, name);
        match clean_name(&expanded, false) {
            Some(name) => Some(name),
            None => {
                let mut cause = b"invalid window name: ".to_vec();
                cause.extend_from_slice(&expanded);
                queue::error(server, item, &cause);
                return CmdReturn::Error;
            }
        }
    } else {
        None
    };
    if args.has(b'S') != 0 {
        let existing = if target.idx != -1 {
            server
                .sessions
                .get(session)
                .and_then(|s| s.windows.get(&target.idx))
                .copied()
        } else if let Some(name) = &name {
            let expanded = format::single_from_target(server, item, name);
            let links: Vec<_> = server
                .sessions
                .get(session)
                .map(|s| {
                    s.windows
                        .values()
                        .copied()
                        .filter(|id| {
                            server
                                .winlinks
                                .get(*id)
                                .and_then(|l| server.windows.get(l.window))
                                .is_some_and(|w| w.name == expanded.as_ref())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if links.len() > 1 {
                let mut cause = b"multiple windows named ".to_vec();
                cause.extend_from_slice(name);
                queue::error(server, item, &cause);
                return CmdReturn::Error;
            }
            links.first().copied()
        } else {
            None
        };
        if let Some(link) = existing {
            if args.has(b'd') != 0 {
                return CmdReturn::Normal;
            }
            let changed = server
                .sessions
                .get(session)
                .is_some_and(|s| s.current != Some(link));
            model::session::session_set_current(server, session, Some(link));
            if let Some(window) = server
                .winlinks
                .get(link)
                .map(|link| link.window)
                .and_then(|window| server.windows.get_mut(window))
            {
                window.latest = client;
            }
            if changed {
                operations::server_redraw_session_group(server, session);
            }
            server
                .effects
                .push_back(model::ModelEffect::RecalculateSizes);
            return CmdReturn::Normal;
        }
    }
    let mut context = SpawnContext::new(session);
    context.item = Some(item);
    context.client = client;
    if let Some(client) = client.and_then(|client| server.clients.get(client)) {
        context.client_environment = Some(client.environ.clone());
        context.client_cwd = client.cwd.clone();
        context.client_attached = client.session.is_some();
    }
    context.name = name;
    context.argv = argv;
    context.cwd = args.get(b'c').map(<[u8]>::to_vec);
    context.index = target.idx;
    for value in args.values_of(b'e') {
        context.environment.put(
            value.as_string(),
            crate::options::environment::EnvironmentFlags::default(),
        );
    }
    if args.has(b'a') != 0 || args.has(b'b') != 0 {
        if let Some(index) =
            model::winlink::winlink_shuffle_up(server, session, target.wl, args.has(b'b') != 0)
        {
            context.index = index;
        }
    }
    if args.has(b'd') != 0 {
        context.flags.insert(SpawnFlags::DETACHED);
    }
    if args.has(b'k') != 0 {
        context.flags.insert(SpawnFlags::KILL);
    }
    if args.has(b'E') != 0 || empty {
        context.flags.insert(SpawnFlags::EMPTY);
    }
    let link = match model::spawn::spawn_window(server, &mut context) {
        Ok(link) => link,
        Err(cause) => {
            let message = format!("create window failed: {cause}");
            queue::error(server, item, message.as_bytes());
            return CmdReturn::Error;
        }
    };
    let Some(wl) = server.winlinks.get(link) else {
        return CmdReturn::Error;
    };
    let pane = server.windows.get(wl.window).and_then(|w| w.active);
    let current = CmdFindState {
        s: Some(session),
        wl: Some(link),
        w: Some(wl.window),
        wp: pane,
        idx: wl.index,
        ..CmdFindState::default()
    };
    if args.has(b'd') == 0 {
        if let Some(queued) = server.queue.items.get(item) {
            if let Some(state) = server.queue.states.get_mut(queued.state) {
                state.current = current;
            }
        }
    }
    let is_current = server
        .sessions
        .get(session)
        .is_some_and(|s| s.current == Some(link));
    if args.has(b'd') == 0 || is_current {
        operations::server_redraw_session_group(server, session);
    } else {
        operations::server_status_session_group(server, session);
    }
    if args.has(b'P') != 0 {
        let output = format::single(
            server,
            Some(item),
            FormatContext {
                session: current.s,
                winlink: current.wl,
                window: current.w,
                pane: current.wp,
                ..FormatContext::default()
            },
            args.get(b'F')
                .unwrap_or(b"#{session_name}:#{window_index}.#{pane_index}"),
        );
        queue::print(server, item, &output);
    }
    queue::insert_hook(server, item, Some(&current), b"after-new-window");
    CmdReturn::Normal
}
