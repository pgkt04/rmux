// Ported from tmux cmd-attach-session.c @ 8f25579c
use crate::{
    client::{self, ClientFlags},
    cmd::{
        Command,
        find::{self, CmdFindFlags, CmdFindType},
        queue::{self, CmdReturn, QueueStateFlags},
    },
    format,
    ids::QueueItemId,
    server::{Server, operations},
};

#[allow(clippy::too_many_arguments)]
pub fn attach(
    server: &mut Server,
    item: QueueItemId,
    name: Option<&[u8]>,
    detach: bool,
    kill: bool,
    readonly: bool,
    cwd: Option<&[u8]>,
    no_environment: bool,
    flags: Option<&[u8]>,
) -> CmdReturn {
    if server.session_names.is_empty() {
        queue::error(server, item, b"no sessions");
        return CmdReturn::Error;
    }
    let Some(client) = server.queue.items.get(item).and_then(|i| i.client) else {
        return CmdReturn::Normal;
    };
    if client::registry::check_nested(server, client) {
        queue::error(
            server,
            item,
            b"sessions should be nested with care, unset $TMUX to force",
        );
        return CmdReturn::Error;
    }
    let pane_target = name.is_some_and(|name| name.contains(&b':') || name.contains(&b'.'));
    let context = find::FindContext {
        client: Some(client),
        current: server
            .queue
            .items
            .get(item)
            .and_then(|i| server.queue.states.get(i.state))
            .map(|s| s.current)
            .unwrap_or_default(),
        ..find::FindContext::default()
    };
    let target = match find::target(
        server,
        &context,
        name,
        if pane_target {
            CmdFindType::Pane
        } else {
            CmdFindType::Session
        },
        CmdFindFlags::PREFER_UNATTACHED,
    ) {
        Ok(target) => target,
        Err(cause) => {
            if let Some(message) = cause.message {
                queue::error(server, item, &message);
            }
            return CmdReturn::Error;
        }
    };
    let Some(session) = target.s else {
        return CmdReturn::Error;
    };
    if let (Some(window), Some(pane)) = (target.w, target.wp) {
        if crate::model::window::window_redraw_active_switch(server, window, Some(pane)).is_err() {
            return CmdReturn::Error;
        }
        let _ = crate::model::window::window_set_active_pane(server, window, pane, true);
    }
    if let Some(link) = target.wl {
        crate::model::session::session_set_current(server, session, Some(link));
    }
    if let Some(queued) = server.queue.items.get(item) {
        if let Some(state) = server.queue.states.get_mut(queued.state) {
            state.current = target;
        }
    }
    if let Some(cwd) = cwd {
        let expanded = format::single(
            server,
            Some(item),
            crate::format::FormatContext {
                session: target.s,
                winlink: target.wl,
                window: target.w,
                pane: target.wp,
                evaluated_client: Some(client),
                ..Default::default()
            },
            cwd,
        );
        if let Some(s) = server.sessions.get_mut(session) {
            s.cwd = expanded.to_vec();
        }
    }
    if let Some(flags) = flags {
        client::flags::set_flags(server, client, flags);
    }
    if readonly {
        let denied = server.clients.get(client).is_some_and(|c| {
            c.flags.contains(ClientFlags::READONLY)
                && c.peer
                    .and_then(|p| server.process.peer_uid(p))
                    .is_some_and(|uid| uid != rmux_sys::proc::getuid())
        });
        if denied {
            queue::error(server, item, b"client is read-only");
            return CmdReturn::Error;
        }
        if let Some(c) = server.clients.get_mut(client) {
            c.flags
                .insert(ClientFlags::READONLY | ClientFlags::IGNORESIZE);
        }
    }
    let attached = server
        .clients
        .get(client)
        .is_some_and(|c| c.session.is_some());
    if !attached {
        if let Err(cause) = client::lifecycle::open(server, client) {
            let mut message = b"open terminal failed: ".to_vec();
            message.extend_from_slice(&cause);
            queue::error(server, item, &message);
            return CmdReturn::Error;
        }
    }
    if detach || kill {
        let others: Vec<_> = server
            .client_order
            .iter()
            .copied()
            .filter(|c| {
                *c != client
                    && server
                        .clients
                        .get(*c)
                        .is_some_and(|c| c.session == Some(session))
            })
            .collect();
        for other in others {
            client::lifecycle::detach(server, other, kill);
        }
    }
    if !no_environment {
        operations::update_session_environment(server, session, client);
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
    if !attached || !repeat {
        client::keys::set_key_table(server, client, None);
    }
    if !attached {
        if server
            .clients
            .get(client)
            .is_some_and(|c| !c.flags.contains(ClientFlags::CONTROL))
        {
            let _ = client::lifecycle::ready(server, client);
        }
        crate::server::events::fire_client(server, b"client-attached", client);
        if let Some(c) = server.clients.get_mut(client) {
            c.flags.insert(ClientFlags::ATTACHED);
        }
    }
    if let Some(queued) = server.queue.items.get(item) {
        if let Some(state) = server.queue.states.get_mut(queued.state) {
            state.current = target;
        }
    }
    CmdReturn::Normal
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    attach(
        server,
        item,
        args.get(b't'),
        args.has(b'd') != 0,
        args.has(b'x') != 0,
        args.has(b'r') != 0,
        args.get(b'c'),
        args.has(b'E') != 0,
        args.get(b'f'),
    )
}

#[cfg(test)]
mod tests {
    use super::super::break_pane::tests::{
        command, create_session, create_window, item, source, split,
    };
    use super::*;
    use crate::cmd::metadata;
    use crate::model::{ModelEffect, PaneFlags, window};
    use rmux_emu::colour::Colour;

    #[test]
    fn attached_pane_selection_invalidates_changed_styles_only() {
        for different_styles in [true, false] {
            let mut server = Server::default();
            let session = create_session(&mut server, b"redraw");
            let (window, link, target) = create_window(&mut server, session, 0);
            let previous = split(&mut server, window, target);
            let untouched = split(&mut server, window, previous);
            window::window_set_active_pane(&mut server, window, previous, false).unwrap();
            let current = source(&server, link, previous);
            let item = item(&mut server, current, current);
            let mut client = client::Client::new(None, (0, 0));
            client.session = Some(session);
            let client = server.clients.insert(client).unwrap();
            server.queue.items.get_mut(item).unwrap().client = Some(client);
            for pane in [target, previous, untouched] {
                let pane = server.panes.get_mut(pane).unwrap();
                pane.cached_gc.bg = Colour::from_raw(1);
                pane.cached_active_gc = pane.cached_gc;
                if different_styles {
                    pane.cached_active_gc.bg = Colour::from_raw(4);
                }
                pane.flags.remove(PaneFlags::REDRAW);
            }
            server.effects.clear();
            let command = command(
                &metadata::CMD_ATTACH_SESSION,
                &[(b't', Some(b"redraw:0.0"))],
            );

            assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
            assert_eq!(server.windows.get(window).unwrap().active, Some(target));
            for pane in [target, previous] {
                assert_eq!(
                    server
                        .panes
                        .get(pane)
                        .unwrap()
                        .flags
                        .contains(PaneFlags::REDRAW),
                    different_styles,
                    "style invalidation for {pane:?}, different styles: {different_styles}"
                );
            }
            assert!(
                !server
                    .panes
                    .get(untouched)
                    .unwrap()
                    .flags
                    .contains(PaneFlags::REDRAW)
            );
            assert!(server.effects.iter().any(|effect| matches!(
                effect,
                ModelEffect::Window(window::WindowEffect::PaneChanged { window: changed_window, old, new })
                    if *changed_window == window && *old == Some(previous) && *new == target
            )));
            let queued = server.queue.items.get(item).unwrap();
            assert_eq!(
                server.queue.states.get(queued.state).unwrap().current.wp,
                Some(target)
            );
        }
    }
}
