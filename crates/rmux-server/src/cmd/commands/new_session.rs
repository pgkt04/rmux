// Ported from tmux cmd-new-session.c @ 8f25579c
use crate::{
    client::{self, ClientFlags},
    cmd::{
        Command,
        find::CmdFindState,
        queue::{self, CmdReturn, QueueStateFlags},
    },
    format::{self, FormatContext},
    ids::QueueItemId,
    model::{self, session::SessionCreate, spawn::SpawnContext, state::clean_name},
    options::environment::{Environment, EnvironmentFlags},
    server::{Server, operations},
};
use std::os::fd::AsFd;

fn name(
    server: &mut Server,
    item: QueueItemId,
    client: Option<crate::ids::ClientId>,
    input: Option<&[u8]>,
    kind: &[u8],
) -> Result<Option<Vec<u8>>, ()> {
    let Some(input) = input else {
        return Ok(None);
    };
    let expanded = format::single(
        server,
        Some(item),
        FormatContext {
            evaluated_client: client,
            ..FormatContext::default()
        },
        input,
    );
    if let Some(name) = clean_name(&expanded, false) {
        return Ok(Some(name));
    }
    let mut cause = b"invalid ".to_vec();
    cause.extend_from_slice(kind);
    cause.extend_from_slice(b" name: ");
    cause.extend_from_slice(&expanded);
    queue::error(server, item, &cause);
    Err(())
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    if command.entry.name == b"has-session" {
        return CmdReturn::Normal;
    }
    let args = &command.args;
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let client = queued.client;
    let target = queued.target;
    if args.has(b't') != 0 && (args.count() != 0 || args.has(b'n') != 0) {
        queue::error(server, item, b"command or window name given with target");
        return CmdReturn::Error;
    }
    let window_name = match name(server, item, client, args.get(b'n'), b"window") {
        Ok(name) => name,
        Err(()) => return CmdReturn::Error,
    };
    let session_name = match name(server, item, client, args.get(b's'), b"session") {
        Ok(name) => name,
        Err(()) => return CmdReturn::Error,
    };
    let existing = session_name
        .as_ref()
        .and_then(|name| model::session::session_find(server, name))
        .or_else(|| {
            if session_name.is_none() {
                target.s
            } else {
                None
            }
        });
    if args.has(b'A') != 0 {
        if let Some(existing) = existing {
            let name = server
                .sessions
                .get(existing)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            return super::attach_session::attach(
                server,
                item,
                Some(&name),
                args.has(b'D') != 0,
                args.has(b'X') != 0,
                false,
                args.get(b'c'),
                args.has(b'E') != 0,
                args.get(b'f'),
            );
        }
    }
    if let Some(name) = session_name
        .as_ref()
        .filter(|name| model::session::session_find(server, name).is_some())
    {
        let mut cause = b"duplicate session: ".to_vec();
        cause.extend_from_slice(name);
        queue::error(server, item, &cause);
        return CmdReturn::Error;
    }
    let groupwith = target.s;
    let mut group = None;
    let prefix = if let Some(group_name) = args.get(b't') {
        group = groupwith
            .and_then(|s| model::session::session_group_contains(server, s))
            .or_else(|| model::session::session_group_find(server, group_name));
        if let Some(group) = group {
            server.groups.get(group).map(|g| g.name.clone())
        } else if let Some(session) = groupwith {
            server.sessions.get(session).map(|s| s.name.clone())
        } else {
            match clean_name(group_name, false) {
                Some(name) => Some(name),
                None => {
                    let mut cause = b"invalid session group name: ".to_vec();
                    cause.extend_from_slice(group_name);
                    queue::error(server, item, &cause);
                    return CmdReturn::Error;
                }
            }
        }
    } else {
        None
    };
    let detached = args.has(b'd') != 0 || client.is_none();
    let control = client
        .and_then(|c| server.clients.get(c))
        .is_some_and(|c| c.flags.contains(ClientFlags::CONTROL));
    let attached = client
        .and_then(|c| server.clients.get(c))
        .is_some_and(|c| c.session.is_some());
    let cwd = if let Some(cwd) = args.get(b'c') {
        format::single(
            server,
            Some(item),
            FormatContext {
                evaluated_client: client,
                ..FormatContext::default()
            },
            cwd,
        )
        .to_vec()
    } else {
        client::registry::get_cwd(server, client, None)
    };
    let mut termios = None;
    if !detached && !attached && !control {
        if let Some(client) = client {
            if client::registry::check_nested(server, client) {
                queue::error(
                    server,
                    item,
                    b"sessions should be nested with care, unset $TMUX to force",
                );
                return CmdReturn::Error;
            }
            if let Some(fd) = server.clients.get(client).and_then(|c| c.fd.as_ref()) {
                termios = Some(rmux_sys::TermiosState::get(fd.as_fd()).expect("tcgetattr failed"));
            }
        }
    }
    if !detached && !attached {
        if let Some(client) = client {
            if let Err(cause) = client::lifecycle::open(server, client) {
                let mut message = b"open terminal failed: ".to_vec();
                message.extend_from_slice(&cause);
                queue::error(server, item, &message);
                return CmdReturn::Error;
            }
        }
    }
    let tty_size = client
        .and_then(|c| server.clients.get(c))
        .map(|c| c.tty_size())
        .unwrap_or((80, 24));
    let mut explicit = [80, 24];
    for (index, flag, label) in [
        (0, b'x', b"width ".as_slice()),
        (1, b'y', b"height ".as_slice()),
    ] {
        if args.has(flag) != 0 {
            if args.get(flag) == Some(b"-") {
                explicit[index] = if index == 0 { tty_size.0 } else { tty_size.1 };
            } else {
                match args.strtonum(flag, 1, 65535) {
                    Ok(value) => explicit[index] = value as u32,
                    Err(cause) => {
                        let mut message = label.to_vec();
                        message.extend_from_slice(&cause);
                        queue::error(server, item, &message);
                        return CmdReturn::Error;
                    }
                }
            }
        }
    }
    let mut size = if !detached && !control {
        let mut size = tty_size;
        if size.1 > 0
            && server
                .options
                .get_number(server.options.global_s, b"status")
                != 0
        {
            size.1 -= 1;
        }
        size
    } else {
        let default = server
            .options
            .get_string(server.options.global_s, b"default-size");
        let parsed = std::str::from_utf8(default)
            .ok()
            .and_then(|s| s.split_once('x'))
            .and_then(|(x, y)| Some((x.parse::<u32>().ok()?, y.parse::<u32>().ok()?)));
        let mut size = parsed.unwrap_or((explicit[0], explicit[1]));
        if args.has(b'x') != 0 {
            size.0 = explicit[0];
        }
        if args.has(b'y') != 0 {
            size.1 = explicit[1];
        }
        size
    };
    size.0 = size.0.max(1);
    size.1 = size.1.max(1);
    let options = server.options.create(Some(server.options.global_s));
    if args.has(b'x') != 0 || args.has(b'y') != 0 {
        let value = format!("{}x{}", size.0, size.1);
        let mut store = std::mem::take(&mut server.options);
        store.set_string(options, b"default-size", false, value.as_bytes(), server);
        server.options = store;
    }
    let environment = Environment::new();
    let session = model::session::session_create(
        server,
        SessionCreate {
            prefix: prefix.clone(),
            name: session_name,
            cwd,
            environment,
            options,
            termios,
        },
    );
    if args.has(b'E') == 0 {
        if let Some(client) = client {
            operations::update_session_environment(server, session, client);
        }
    }
    for value in args.values_of(b'e') {
        if let Some(session) = server.sessions.get_mut(session) {
            session
                .environment
                .put(value.as_string(), EnvironmentFlags::default());
        }
    }
    let mut spawn = SpawnContext::new(session);
    spawn.item = Some(item);
    spawn.client = if detached { None } else { client };
    if let Some(client) = client.and_then(|client| server.clients.get(client)) {
        spawn.client_environment = Some(client.environ.clone());
        spawn.client_cwd = client.cwd.clone();
        spawn.client_attached = client.session.is_some();
    }
    spawn.name = window_name;
    spawn.argv = args
        .values()
        .iter()
        .map(|value| value.as_string().to_vec())
        .collect();
    spawn.cwd = args.get(b'c').map(<[u8]>::to_vec);
    spawn.initial_size = Some(model::resize::WindowSize {
        sx: size.0,
        sy: size.1,
        xpixel: 0,
        ypixel: 0,
    });
    let link = match model::spawn::spawn_window(server, &mut spawn) {
        Ok(link) => link,
        Err(cause) => {
            model::session::session_destroy(server, session, false);
            queue::error(
                server,
                item,
                format!("create window failed: {cause}").as_bytes(),
            );
            return CmdReturn::Error;
        }
    };
    if let Some(prefix) = prefix {
        let group = group.unwrap_or_else(|| model::session::session_group_new(server, &prefix));
        if let Some(groupwith) = groupwith {
            model::session::session_group_add(server, group, groupwith);
        }
        model::session::session_group_add(server, group, session);
        model::session::session_group_synchronize_to(server, session);
    }
    let wl = server
        .sessions
        .get(session)
        .and_then(|s| s.current)
        .unwrap_or(link);
    let current = server
        .winlinks
        .get(wl)
        .map(|wl| CmdFindState {
            s: Some(session),
            wl: Some(wl_id(server, session, wl.index)),
            w: Some(wl.window),
            wp: server.windows.get(wl.window).and_then(|w| w.active),
            idx: wl.index,
            ..CmdFindState::default()
        })
        .unwrap_or_default();
    crate::server::events::fire_session(server, b"session-created", session);
    if !detached {
        if let Some(client) = client {
            if let Some(flags) = args.get(b'f') {
                client::flags::set_flags(server, client, flags);
            }
            if !attached && !control {
                let _ = client::lifecycle::ready(server, client);
            }
            if let Some(c) = server.clients.get_mut(client) {
                c.last_session = c.session;
            }
            client::lifecycle::set_session(server, client, Some(session));
            let repeat = server
                .queue
                .items
                .get(item)
                .and_then(|i| server.queue.states.get(i.state))
                .is_some_and(|s| s.flags.contains(QueueStateFlags::REPEAT));
            if !repeat {
                client::keys::set_key_table(server, client, None);
            }
        }
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
            args.get(b'F').unwrap_or(b"#{session_name}:"),
        );
        queue::print(server, item, &output);
    }
    if !detached {
        if let Some(client) = client.and_then(|c| server.clients.get_mut(c)) {
            client.flags.insert(ClientFlags::ATTACHED);
        }
    }
    if args.has(b'd') == 0 {
        if let Some(queued) = server.queue.items.get(item) {
            if let Some(state) = server.queue.states.get_mut(queued.state) {
                state.current = current;
            }
        }
    }
    queue::insert_hook(server, item, Some(&current), b"after-new-session");
    CmdReturn::Normal
}
fn wl_id(server: &Server, session: crate::ids::SessionId, index: i32) -> crate::ids::WinlinkId {
    *server
        .sessions
        .get(session)
        .expect("new session")
        .windows
        .get(&index)
        .expect("new window")
}
