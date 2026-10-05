// Ported from tmux server-fn.c, server.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

use super::events::{self, EventPayload};
use super::protocol::{ProtocolError, ProtocolMessage, ProtocolMessageKind};
use crate::client::ClientFlags;
use crate::cmd::find::{self, CmdFindFlags, CmdFindState};
use crate::ids::{ClientId, PaneId, SessionId, WindowId, WinlinkId};
use crate::model::{ModelError, PaneFlags, Server, WinlinkFlags, session, window};

pub fn update_session_environment(server: &mut Server, session: SessionId, client: ClientId) {
    let Some(session) = server.sessions.get_mut(session) else {
        return;
    };
    let Some(client) = server.clients.get(client) else {
        return;
    };
    crate::options::environment::environ_update(
        &server.options,
        session.options,
        &client.environ,
        &mut session.environment,
    );
}

pub fn server_redraw_client(server: &mut Server, id: ClientId) {
    if let Some(client) = server.clients.get_mut(id) {
        client.flags.insert(ClientFlags::ALLREDRAWFLAGS);
    }
}

pub fn server_status_client(server: &mut Server, id: ClientId) {
    if let Some(client) = server.clients.get_mut(id) {
        client.flags.insert(ClientFlags::REDRAWSTATUS);
    }
}

fn session_flags(server: &mut Server, id: SessionId, flags: ClientFlags) {
    for client in &server.client_order {
        if let Some(client) = server.clients.get_mut(*client)
            && client.session == Some(id)
        {
            client.flags.insert(flags);
        }
    }
}

pub fn server_redraw_session(server: &mut Server, id: SessionId) {
    session_flags(server, id, ClientFlags::ALLREDRAWFLAGS);
}

pub fn server_status_session(server: &mut Server, id: SessionId) {
    session_flags(server, id, ClientFlags::REDRAWSTATUS);
}

fn group_members(server: &Server, id: SessionId) -> Vec<SessionId> {
    session::session_group_contains(server, id)
        .and_then(|group| server.groups.get(group))
        .map_or_else(|| vec![id], |group| group.sessions.clone())
}

pub fn server_redraw_session_group(server: &mut Server, id: SessionId) {
    for member in group_members(server, id) {
        server_redraw_session(server, member);
    }
}

pub fn server_status_session_group(server: &mut Server, id: SessionId) {
    for member in group_members(server, id) {
        server_status_session(server, member);
    }
}

fn window_flags(server: &mut Server, id: WindowId, flags: ClientFlags) {
    for client_id in &server.client_order {
        let matches = server
            .clients
            .get(*client_id)
            .and_then(|client| client.session)
            .and_then(|session| server.sessions.get(session))
            .and_then(|session| session.current)
            .and_then(|link| server.winlinks.get(link))
            .is_some_and(|link| link.window == id);
        if matches && let Some(client) = server.clients.get_mut(*client_id) {
            client.flags.insert(flags);
        }
    }
}

pub fn server_redraw_window(server: &mut Server, id: WindowId) {
    window_flags(server, id, ClientFlags::ALLREDRAWFLAGS);
}

pub fn server_redraw_window_menu(server: &mut Server, id: WindowId) {
    window_flags(server, id, ClientFlags::REDRAWMENU);
}

pub fn server_redraw_window_borders(server: &mut Server, id: WindowId) {
    window_flags(server, id, ClientFlags::REDRAWBORDERS);
}

pub fn server_status_window(server: &mut Server, id: WindowId) {
    for client_id in &server.client_order {
        if server
            .clients
            .get(*client_id)
            .and_then(|client| client.session)
            .is_some_and(|s| session::session_has(server, s, id))
            && let Some(client) = server.clients.get_mut(*client_id)
        {
            client.flags.insert(ClientFlags::REDRAWSTATUS);
        }
    }
}

pub fn lock(server: &mut Server) -> Result<(), ProtocolError> {
    let clients: Vec<_> = server.client_order.iter().copied().collect();
    for id in clients {
        lock_client(server, id)?;
    }
    Ok(())
}

pub fn lock_session(server: &mut Server, session: SessionId) -> Result<(), ProtocolError> {
    let clients: Vec<_> = server
        .client_order
        .iter()
        .copied()
        .filter(|id| {
            server
                .clients
                .get(*id)
                .is_some_and(|c| c.session == Some(session))
        })
        .collect();
    for id in clients {
        lock_client(server, id)?;
    }
    Ok(())
}

pub fn lock_client(server: &mut Server, id: ClientId) -> Result<(), ProtocolError> {
    let Some(client) = server.clients.get(id) else {
        return Ok(());
    };
    if client
        .flags
        .intersects(ClientFlags::CONTROL | ClientFlags::SUSPENDED)
    {
        return Ok(());
    }
    let Some(session) = client.session.and_then(|id| server.sessions.get(id)) else {
        return Ok(());
    };
    let command =
        rmux_util::bytes::cstr(server.options.get_string(session.options, b"lock-command"));
    // MAX_IMSGSIZE - IMSG_HEADER_SIZE at the pin.
    if command.is_empty() || command.len() + 1 > super::protocol::LEGACY_PAYLOAD {
        return Ok(());
    }
    let command = super::protocol::encode_string(command);
    let options = crate::client::lifecycle::tty_options(server);
    let client = server.clients.get_mut(id).expect("lock client");
    if let Some(tty) = client.tty.as_mut() {
        tty.stop(&mut server.tparm, &options);
        for code in [
            rmux_tty::term::TtyCodeCode::Smcup,
            rmux_tty::term::TtyCodeCode::Clear,
            rmux_tty::term::TtyCodeCode::E3,
        ] {
            let capability = tty.term().string(code).to_vec();
            tty.raw(&capability);
        }
    }
    client.flags.insert(ClientFlags::SUSPENDED);
    if let Some(peer) = client.peer {
        super::proc::proc_send(
            server,
            peer,
            ProtocolMessage::new(ProtocolMessageKind::Lock, command),
        )?;
    }
    Ok(())
}

pub fn server_set_marked(
    server: &mut Server,
    s: Option<SessionId>,
    wl: Option<WinlinkId>,
    wp: Option<PaneId>,
) {
    server.marked_session = s;
    server.marked_window = wl
        .and_then(|id| server.winlinks.get(id))
        .map(|link| link.window);
    server.marked_winlink = wl;
    server.marked_pane = wp;
}

pub fn server_clear_marked(server: &mut Server) {
    server_set_marked(server, None, None, None);
}

pub fn server_check_marked(server: &Server) -> bool {
    CmdFindState {
        s: server.marked_session,
        wl: server.marked_winlink,
        w: server.marked_window,
        wp: server.marked_pane,
        ..CmdFindState::default()
    }
    .is_valid(server)
}

pub fn server_is_marked(
    server: &Server,
    s: Option<SessionId>,
    wl: Option<WinlinkId>,
    wp: Option<PaneId>,
) -> bool {
    s.is_some()
        && wl.is_some()
        && wp.is_some()
        && server.marked_session == s
        && server.marked_winlink == wl
        && server.marked_pane == wp
        && server_check_marked(server)
}

fn remove_pane(server: &mut Server, pane: PaneId) -> Result<WindowId, ModelError> {
    let p = server.panes.get(pane).ok_or(ModelError::StaleId)?;
    let (window, keep) = (p.window, p.flags.contains(PaneFlags::FLOATOVERZOOM));
    super::run::close_pane_io(server, pane);
    window::window_push_zoom(server, window, false, keep)?;
    crate::client::mouse::remove_pane(server, pane);
    crate::layout::close_pane(server, pane);
    window::window_remove_pane(server, window, pane)?;
    Ok(window)
}

pub fn server_kill_pane(server: &mut Server, pane: PaneId) -> Result<(), ModelError> {
    let window = server.panes.get(pane).ok_or(ModelError::StaleId)?.window;
    if window::window_count_panes(server, window, true) == 1 {
        server_kill_window(server, window, true)?;
        super::run::recalculate_sizes(server);
    } else {
        remove_pane(server, pane)?;
        window::window_pop_zoom(server, window, false)?;
        server_redraw_window(server, window);
    }
    Ok(())
}

pub fn server_kill_window(
    server: &mut Server,
    id: WindowId,
    renumber: bool,
) -> Result<(), ModelError> {
    window::window_retain(server, id)?;
    let result = (|| {
        let sessions: Vec<_> = server.session_names.values().copied().collect();
        for session in sessions {
            if !session::session_has(server, session, id) {
                continue;
            }
            server_unzoom_window(server, id)?;
            while let Some(link) = window::winlink_find_by_window(server, session, id) {
                if session::session_detach(server, session, link)
                    == session::DetachOutcome::DestroySession
                {
                    server_destroy_session_group(server, session);
                    break;
                }
                server_redraw_session_group(server, session);
            }
            if renumber {
                server_renumber_session(server, session);
            }
        }
        super::run::recalculate_sizes(server);
        Ok(())
    })();
    let release = window::window_release(server, id);
    result.and(release)
}

pub fn server_renumber_session(server: &mut Server, id: SessionId) {
    if !server
        .sessions
        .get(id)
        .is_some_and(|s| server.options.get_number(s.options, b"renumber-windows") != 0)
    {
        return;
    }
    for member in group_members(server, id) {
        session::session_renumber_windows(server, member);
    }
}

pub fn server_renumber_all(server: &mut Server) {
    let sessions: Vec<_> = server.session_names.values().copied().collect();
    for session in sessions {
        server_renumber_session(server, session);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn server_link_window(
    server: &mut Server,
    src: SessionId,
    srcwl: WinlinkId,
    dst: SessionId,
    mut index: i32,
    kill: bool,
    mut select: bool,
) -> Result<WinlinkId, Vec<u8>> {
    let src_group = session::session_group_contains(server, src);
    if src != dst
        && src_group.is_some()
        && src_group == session::session_group_contains(server, dst)
    {
        return Err(b"sessions are grouped".to_vec());
    }
    let source = server
        .winlinks
        .get(srcwl)
        .filter(|link| link.session == src)
        .ok_or_else(|| b"stale target".to_vec())?
        .window;
    if index >= 0
        && let Some(old) = window::winlink_find_by_index(server, dst, index)
    {
        if server
            .winlinks
            .get(old)
            .is_some_and(|link| link.window == source)
        {
            return Err(format!("same index: {index}").into_bytes());
        }
        if kill {
            events::fire_winlink(server, b"window-unlinked", old);
            let Some(link) = server.winlinks.get_mut(old) else {
                return Err(b"stale target".to_vec());
            };
            link.flags.remove(WinlinkFlags::ALERTFLAGS);
            window::winlink_stack_remove(server, dst, old);
            let current = server
                .sessions
                .get(dst)
                .is_some_and(|s| s.current == Some(old));
            window::winlink_remove(server, old);
            if current && let Some(s) = server.sessions.get_mut(dst) {
                s.current = None;
                select = true;
            }
        }
    }
    if index == -1 {
        let options = server
            .sessions
            .get(dst)
            .ok_or_else(|| b"stale target".to_vec())?
            .options;
        index = -1 - server.options.get_number(options, b"base-index") as i32;
    }
    let new = session::session_attach(server, dst, source, index).map_err(|error| match error {
        ModelError::Message(bytes) => bytes,
        other => other.to_string().into_bytes(),
    })?;
    if server.marked_winlink == Some(srcwl) {
        server.marked_winlink = Some(new);
    }
    if select {
        let index = server
            .winlinks
            .get(new)
            .ok_or_else(|| b"stale target".to_vec())?
            .index;
        session::session_select(server, dst, index);
    }
    server_redraw_session_group(server, dst);
    Ok(new)
}

pub fn server_unlink_window(
    server: &mut Server,
    s: SessionId,
    wl: WinlinkId,
) -> Result<(), ModelError> {
    match session::session_detach(server, s, wl) {
        session::DetachOutcome::DestroySession => server_destroy_session_group(server, s),
        session::DetachOutcome::Detached => server_redraw_session_group(server, s),
        session::DetachOutcome::Missing => return Err(ModelError::StaleId),
    }
    Ok(())
}

pub fn server_unzoom_window(server: &mut Server, w: WindowId) -> Result<(), ModelError> {
    if window::window_unzoom(server, w, true)? {
        server_redraw_window(server, w);
    }
    Ok(())
}

pub fn server_fire_pane_exit(server: &mut Server, name: &[u8], id: PaneId) {
    let Some(pane) = server.panes.get(id) else {
        return;
    };
    let (window, status) = (pane.window, pane.status);
    let target = find::from_pane(server, id, CmdFindFlags::default()).unwrap_or_default();
    let mut payload = EventPayload::new();
    payload.set_target(server, &target);
    payload.set_pane(server, b"pane", id);
    payload.set_window(server, b"window", window);
    if let Some(status) = rmux_sys::proc::wait_exit_status(status) {
        payload.set_int(server, b"exit_status", status);
    } else if let Some(signal) = rmux_sys::proc::wait_signal_name(status) {
        payload.set_string(server, b"exit_signal", &signal);
    }
    payload.set_int(server, b"exit_success", i32::from(status == 0));
    events::fire(server, name, payload);
}

fn retain_exit(choice: i64, status: i32) -> bool {
    matches!(choice, 1 | 3)
        || (matches!(choice, 2 | 4) && rmux_sys::proc::wait_exit_status(status) != Some(0))
}

pub fn server_destroy_pane(
    server: &mut Server,
    id: PaneId,
    notify: bool,
) -> Result<(), ModelError> {
    super::run::close_pane_io(server, id);
    let pane = server.panes.get(id).ok_or(ModelError::StaleId)?;
    if !pane.flags.contains(PaneFlags::STATUSREADY) {
        return Ok(());
    }
    let retain = retain_exit(
        server.options.get_number(pane.options, b"remain-on-exit"),
        pane.status,
    );
    if retain {
        if pane.flags.contains(PaneFlags::STATUSDRAWN) {
            return Ok(());
        }
        let pane = server.panes.get_mut(id).expect("dead pane");
        pane.flags.insert(PaneFlags::STATUSDRAWN);
        pane.dead_time = server.current_time;
        if notify {
            server_fire_pane_exit(server, b"pane-died", id);
        }
        let Some(pane) = server
            .panes
            .get(id)
            .filter(|p| !p.flags.contains(PaneFlags::DESTROYED))
        else {
            return Ok(());
        };
        let template = server
            .options
            .get_string(pane.options, b"remain-on-exit-format")
            .to_vec();
        if !template.is_empty() {
            let state = CmdFindState {
                w: Some(pane.window),
                wp: Some(id),
                ..CmdFindState::default()
            };
            let options = pane.options;
            let expanded =
                crate::format::runtime::single_from_state(server, None, None, &state, &template);
            let policy = rmux_emu::screen::write::ScreenWritePolicy {
                pane_backed: true,
                alternate_screen: server.options.get_number(options, b"alternate-screen") != 0,
                scroll_on_clear: server.options.get_number(options, b"scroll-on-clear") != 0,
                variation_selector_always_wide: server
                    .options
                    .get_number(server.options.global, b"variation-selector-always-wide")
                    != 0,
                extended_keys: server
                    .options
                    .get_number(server.options.global, b"extended-keys")
                    != 0,
            };
            let pane = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
            let (sx, sy) = (pane.base.grid.sx(), pane.base.grid.sy());
            let mut sink = rmux_emu::screen::write::ScreenOnlySink;
            let mut writer = rmux_emu::screen::write::ScreenWriteCtx::start(
                &mut pane.base,
                &mut sink,
                policy,
                &mut server.hyperlinks,
            );
            writer.scrollregion(0, sy - 1);
            writer.cursormove(0, (sy - 1) as i32, false);
            writer.linefeed(true, rmux_emu::colour::Colour::DEFAULT);
            crate::format::draw::draw(
                &mut writer,
                &rmux_emu::cell::DEFAULT_CELL,
                sx,
                expanded.as_bytes(),
                None,
                false,
            );
            writer.finish();
        }
        if let Some(pane) = server.panes.get_mut(id) {
            pane.base.mode.remove(rmux_emu::screen::ScreenMode::CURSOR);
            pane.flags.insert(PaneFlags::REDRAW);
        }
        return Ok(());
    }
    if notify {
        server_fire_pane_exit(server, b"pane-exited", id);
    }
    if server
        .panes
        .get(id)
        .is_none_or(|p| p.flags.contains(PaneFlags::DESTROYED))
    {
        return Ok(());
    }
    let window = remove_pane(server, id)?;
    if window::window_count_panes(server, window, true) == 0 {
        server_kill_window(server, window, true)?;
    } else {
        window::window_pop_zoom(server, window, false)?;
        server_redraw_window(server, window);
    }
    Ok(())
}

pub fn server_newer_session(
    server: &Server,
    candidate: SessionId,
    current: Option<SessionId>,
) -> bool {
    let Some(candidate) = server.sessions.get(candidate) else {
        return false;
    };
    current
        .and_then(|id| server.sessions.get(id))
        .is_none_or(|current| candidate.activity > current.activity)
}

pub fn server_newer_detached_session(
    server: &Server,
    candidate: SessionId,
    current: Option<SessionId>,
) -> bool {
    server
        .sessions
        .get(candidate)
        .is_some_and(|s| s.attached == 0)
        && server_newer_session(server, candidate, current)
}

pub fn server_find_session(
    server: &Server,
    excluded: SessionId,
    predicate: fn(&Server, SessionId, Option<SessionId>) -> bool,
) -> Option<SessionId> {
    let mut selected = None;
    for id in server.session_names.values().copied() {
        if id != excluded && predicate(server, id, selected) {
            selected = Some(id);
        }
    }
    selected
}

fn replacement(server: &Server, id: SessionId) -> (Option<SessionId>, Option<SessionId>) {
    let Some(s) = server.sessions.get(id) else {
        return (None, None);
    };
    let choice = server.options.get_number(s.options, b"detach-on-destroy");
    let normal = match choice {
        0 => server_find_session(server, id, server_newer_session),
        2 => server_find_session(server, id, server_newer_detached_session),
        3 | 4 => {
            let sorted: Vec<_> = server.session_names.values().copied().collect();
            if choice == 3 {
                session::session_previous_session(server, id, &sorted)
            } else {
                session::session_next_session(server, id, &sorted)
            }
        }
        _ => None,
    }
    .filter(|selected| *selected != id);
    let fallback = if normal.is_none() && matches!(choice, 1 | 2) {
        server_find_session(server, id, server_newer_session)
    } else {
        None
    };
    (normal, fallback)
}

pub fn server_destroy_session(server: &mut Server, id: SessionId) {
    let (normal, fallback) = replacement(server, id);
    let clients: Vec<_> = server.client_order.iter().copied().collect();
    for client in clients {
        let Some(c) = server
            .clients
            .get_mut(client)
            .filter(|c| c.session == Some(id))
        else {
            continue;
        };
        let destination = normal.or_else(|| {
            c.flags
                .contains(ClientFlags::NO_DETACH_ON_DESTROY)
                .then_some(fallback)
                .flatten()
        });
        c.session = None;
        c.last_session = None;
        crate::client::lifecycle::set_session(server, client, destination);
        if destination.is_none()
            && let Some(c) = server.clients.get_mut(client)
        {
            c.flags.insert(ClientFlags::EXIT);
        }
    }
    super::run::recalculate_sizes(server);
}

pub fn server_destroy_session_group(server: &mut Server, id: SessionId) {
    for member in group_members(server, id) {
        if !session::session_alive(server, member) {
            continue;
        }
        server_destroy_session(server, member);
        session::session_destroy(server, member, true);
    }
}

pub fn server_check_unattached(server: &mut Server) {
    let sessions: Vec<_> = server.session_names.values().copied().collect();
    for id in sessions {
        let Some(s) = server.sessions.get(id) else {
            continue;
        };
        if s.attached != 0 {
            continue;
        }
        let group = s
            .group
            .map(|group| session::session_group_count(server, group));
        let destroy = match server.options.get_number(s.options, b"destroy-unattached") {
            0 => false,
            1 => true,
            2 => group.is_some_and(|count| count > 1),
            3 => group != Some(1),
            _ => true,
        };
        if destroy {
            server_destroy_session(server, id);
            session::session_destroy(server, id, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(server: &mut Server, name: &[u8]) -> SessionId {
        let options = server.options.create(Some(server.options.global_s));
        session::session_create(
            server,
            session::SessionCreate {
                prefix: None,
                name: Some(name.to_vec()),
                cwd: b"/".to_vec(),
                environment: crate::options::environment::Environment::default(),
                options,
                termios: None,
            },
        )
    }

    fn window(
        server: &mut Server,
        session: SessionId,
        index: i32,
    ) -> (WindowId, WinlinkId, PaneId) {
        let window = window::window_create(server, 40, 8, 0, 0).unwrap();
        let pane = window::window_add_pane(
            server,
            window,
            None,
            0,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        window::window_set_active_pane(server, window, pane, false).unwrap();
        let link = session::session_attach(server, session, window, index).unwrap();
        session::session_set_current(server, session, Some(link));
        (window, link, pane)
    }

    fn client(server: &mut Server, session: SessionId) -> ClientId {
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = Some(session);
        client.flags = ClientFlags::ATTACHED;
        let id = server.clients.insert(client).unwrap();
        server.clients.retain(id).unwrap();
        server.client_order.push_back(id);
        id
    }

    fn option(server: &mut Server, session: SessionId, name: &[u8], value: i64) {
        let options = server.sessions.get(session).unwrap().options;
        server.options.set_number_value(options, name, value);
    }

    #[test]
    fn redraw_window_only_current_status_all_containing_sessions() {
        let mut server = Server::new();
        let a = session(&mut server, b"a");
        let b = session(&mut server, b"b");
        let (w, _, _) = window(&mut server, a, 0);
        session::session_attach(&mut server, b, w, 0).unwrap();
        window(&mut server, b, 1);
        let ca = client(&mut server, a);
        let cb = client(&mut server, b);
        server_redraw_window_menu(&mut server, w);
        assert!(
            server
                .clients
                .get(ca)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWMENU)
        );
        assert!(
            !server
                .clients
                .get(cb)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWMENU)
        );
        server_redraw_window_borders(&mut server, w);
        server_status_window(&mut server, w);
        assert!(
            server
                .clients
                .get(ca)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWBORDERS)
        );
        assert!(
            server
                .clients
                .get(cb)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWSTATUS)
        );
        assert!(
            !server
                .clients
                .get(cb)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWWINDOW)
        );
        server_redraw_session(&mut server, b);
        assert!(
            server
                .clients
                .get(cb)
                .unwrap()
                .flags
                .contains(ClientFlags::ALLREDRAWFLAGS)
        );
    }

    #[test]
    fn group_redraw_and_renumber_reach_each_member() {
        let mut server = Server::new();
        let a = session(&mut server, b"a");
        let b = session(&mut server, b"b");
        window(&mut server, a, 7);
        let group = session::session_group_new(&mut server, b"g");
        session::session_group_add(&mut server, group, a);
        session::session_group_add(&mut server, group, b);
        session::session_group_synchronize_to(&mut server, b);
        let ca = client(&mut server, a);
        let cb = client(&mut server, b);
        server_status_session_group(&mut server, a);
        for c in [ca, cb] {
            assert!(
                server
                    .clients
                    .get(c)
                    .unwrap()
                    .flags
                    .contains(ClientFlags::REDRAWSTATUS)
            );
        }
        option(&mut server, a, b"renumber-windows", 1);
        server_renumber_session(&mut server, a);
        for s in [a, b] {
            assert_eq!(
                server
                    .sessions
                    .get(s)
                    .unwrap()
                    .windows
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![0]
            );
        }
        let link = *server
            .sessions
            .get(a)
            .unwrap()
            .windows
            .values()
            .next()
            .unwrap();
        assert_eq!(
            server_link_window(&mut server, a, link, b, -1, false, false),
            Err(b"sessions are grouped".to_vec())
        );
    }

    #[test]
    fn marked_link_moves_but_stored_session_does_not() {
        let mut server = Server::new();
        let a = session(&mut server, b"a");
        let b = session(&mut server, b"b");
        let (w, old, pane) = window(&mut server, a, 0);
        server_set_marked(&mut server, Some(a), Some(old), Some(pane));
        assert!(server_is_marked(&server, Some(a), Some(old), Some(pane)));
        let new = server_link_window(&mut server, a, old, b, 0, false, true).unwrap();
        assert_eq!(server.marked_winlink, Some(new));
        assert_eq!(server.marked_session, Some(a));
        assert!(!server_check_marked(&server));
        assert_eq!(
            server_link_window(&mut server, a, old, b, 0, true, false),
            Err(b"same index: 0".to_vec())
        );
        assert_eq!(server.windows.get(w).unwrap().links.len(), 2);
        server_clear_marked(&mut server);
        assert!(!server_is_marked(&server, None, None, None));
    }

    #[test]
    fn replacement_choices_and_equal_activity_name_order() {
        let mut server = Server::new();
        let a = session(&mut server, b"a");
        let b = session(&mut server, b"b");
        let c = session(&mut server, b"c");
        for s in [a, b, c] {
            window(&mut server, s, 0);
        }
        for s in [a, c] {
            server.sessions.get_mut(s).unwrap().activity = (10, 2);
        }
        assert_eq!(
            server_find_session(&server, b, server_newer_session),
            Some(a)
        );
        for (choice, expected) in [
            (0, Some(a)),
            (1, None),
            (2, Some(a)),
            (3, Some(a)),
            (4, Some(c)),
        ] {
            option(&mut server, b, b"detach-on-destroy", choice);
            assert_eq!(replacement(&server, b).0, expected);
        }
        server.sessions.get_mut(a).unwrap().attached = 1;
        server.sessions.get_mut(c).unwrap().attached = 1;
        option(&mut server, b, b"detach-on-destroy", 2);
        assert_eq!(replacement(&server, b), (None, Some(a)));
        let ordinary = client(&mut server, b);
        let no_detach = client(&mut server, b);
        server
            .clients
            .get_mut(no_detach)
            .unwrap()
            .flags
            .insert(ClientFlags::NO_DETACH_ON_DESTROY);
        server_destroy_session(&mut server, b);
        assert!(
            server
                .clients
                .get(ordinary)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        assert_eq!(server.clients.get(no_detach).unwrap().session, Some(a));
        assert_eq!(server.clients.get(no_detach).unwrap().last_session, None);
    }

    #[test]
    fn unattached_keep_last_and_keep_group_differ() {
        for (choice, keep) in [(0, true), (1, false), (2, true), (3, false)] {
            let mut server = Server::new();
            let s = session(&mut server, b"s");
            window(&mut server, s, 0);
            option(&mut server, s, b"destroy-unattached", choice);
            server_check_unattached(&mut server);
            assert_eq!(session::session_alive(&server, s), keep);
        }
        for choice in [2, 3] {
            let mut server = Server::new();
            let s = session(&mut server, b"s");
            window(&mut server, s, 0);
            let group = session::session_group_new(&mut server, b"g");
            session::session_group_add(&mut server, group, s);
            option(&mut server, s, b"destroy-unattached", choice);
            server_check_unattached(&mut server);
            assert!(session::session_alive(&server, s));
        }
    }

    #[test]
    fn kill_window_releases_only_its_own_lifetime_lease() {
        let mut server = Server::new();
        let s = session(&mut server, b"s");
        let (w, _, pane) = window(&mut server, s, 0);
        window::window_retain(&mut server, w).unwrap();
        server_kill_window(&mut server, w, true).unwrap();
        assert!(!session::session_alive(&server, s));
        assert!(server.windows.get(w).is_some());
        assert!(server.panes.get(pane).is_some());
        assert_eq!(server.windows.get(w).unwrap().references, 1);
        window::window_release(&mut server, w).unwrap();
        assert!(server.windows.get(w).is_none());
        assert!(server.panes.get(pane).is_none());
    }

    #[test]
    fn remain_choices_use_normal_success_not_merely_zero_exit_code() {
        for choice in 0..=4 {
            assert_eq!(retain_exit(choice, 0), matches!(choice, 1 | 3));
            assert_eq!(retain_exit(choice, 7 << 8), choice != 0);
            assert_eq!(retain_exit(choice, 15), choice != 0);
        }
    }

    fn died(server: &mut Server, payload: &mut EventPayload) {
        assert_eq!(payload.get_int(b"exit_status"), Some(7));
        assert_eq!(payload.get_int(b"exit_success"), Some(0));
        let pane = payload.get_pane(b"pane").unwrap();
        server.panes.get_mut(pane).unwrap().cmd_status += 1;
    }

    #[test]
    fn retained_dead_pane_draws_once_and_preserves_model_lifetime() {
        let mut server = Server::new();
        let s = session(&mut server, b"s");
        let (_, _, pane) = window(&mut server, s, 0);
        let options = server.panes.get(pane).unwrap().options;
        server
            .options
            .set_number_value(options, b"remain-on-exit", 1);
        server.current_time = (123, 4);
        let p = server.panes.get_mut(pane).unwrap();
        p.cmd_status = 0;
        p.status = 7 << 8;
        p.flags.insert(PaneFlags::STATUSREADY);
        events::add_sink(&mut server, b"pane-died", died);
        server_destroy_pane(&mut server, pane, true).unwrap();
        let p = server.panes.get(pane).unwrap();
        assert!(p.flags.contains(PaneFlags::STATUSDRAWN | PaneFlags::REDRAW));
        assert!(!p.base.mode.contains(rmux_emu::screen::ScreenMode::CURSOR));
        assert_eq!(p.dead_time, (123, 4));
        let mut capture = rmux_emu::grid::StringCellsCtx {
            last: None,
            flags: rmux_emu::grid::GridStringFlags::default(),
            hyperlinks: None,
        };
        let bottom = p.base.grid.string_cells(
            0,
            p.base.grid.hsize() + p.base.grid.sy() - 1,
            p.base.grid.sx(),
            &mut capture,
        );
        assert!(
            bottom
                .windows(b"Pane is dead".len())
                .any(|text| text == b"Pane is dead")
        );
        let cursor = (p.base.cx, p.base.cy);
        server.current_time = (456, 0);
        server_destroy_pane(&mut server, pane, true).unwrap();
        let p = server.panes.get(pane).unwrap();
        assert_eq!(p.dead_time, (123, 4));
        assert_eq!((p.base.cx, p.base.cy), cursor);
        assert_eq!(p.cmd_status, 1);
    }

    #[test]
    fn lock_skips_control_and_suspended_clients() {
        let mut server = Server::new();
        let s = session(&mut server, b"s");
        let c = client(&mut server, s);
        server
            .clients
            .get_mut(c)
            .unwrap()
            .flags
            .insert(ClientFlags::CONTROL);
        lock_client(&mut server, c).unwrap();
        assert!(
            !server
                .clients
                .get(c)
                .unwrap()
                .flags
                .contains(ClientFlags::SUSPENDED)
        );
        server
            .clients
            .get_mut(c)
            .unwrap()
            .flags
            .remove(ClientFlags::CONTROL);
        lock_session(&mut server, s).unwrap();
        assert!(
            server
                .clients
                .get(c)
                .unwrap()
                .flags
                .contains(ClientFlags::SUSPENDED)
        );
        lock(&mut server).unwrap();
        assert!(
            server
                .clients
                .get(c)
                .unwrap()
                .flags
                .contains(ClientFlags::SUSPENDED)
        );
    }

    fn exited(server: &mut Server, payload: &mut EventPayload) {
        let pane = payload.get_pane(b"pane").unwrap();
        let window = payload.get_window(b"window").unwrap();
        assert!(server.panes.get(pane).is_some());
        assert!(server.windows.get(window).is_some());
        assert_eq!(payload.get_int(b"exit_status"), Some(0));
        assert_eq!(payload.get_int(b"exit_success"), Some(1));
        assert!(
            payload
                .get_target(server, CmdFindFlags::default())
                .is_valid(server)
        );
        server.panes.get_mut(pane).unwrap().cmd_status += 1;
    }

    #[test]
    fn successful_failed_remain_exit_notifies_before_last_window_destruction() {
        let mut server = Server::new();
        let s = session(&mut server, b"s");
        let (w, _, pane) = window(&mut server, s, 0);
        let options = server.panes.get(pane).unwrap().options;
        server
            .options
            .set_number_value(options, b"remain-on-exit", 2);
        let p = server.panes.get_mut(pane).unwrap();
        p.cmd_status = 0;
        p.status = 0;
        p.flags.insert(PaneFlags::STATUSREADY);
        events::add_sink(&mut server, b"pane-exited", exited);
        crate::model::pane::pane_retain(&mut server, pane).unwrap();
        server_destroy_pane(&mut server, pane, true).unwrap();
        assert_eq!(server.panes.get(pane).unwrap().cmd_status, 1);
        assert!(!session::session_alive(&server, s));
        assert!(server.windows.get(w).is_none());
        crate::model::pane::pane_release(&mut server, pane).unwrap();
        assert!(server.panes.get(pane).is_none());
    }

    #[test]
    fn unlink_last_window_destroys_whole_group_after_client_migration() {
        let mut server = Server::new();
        let a = session(&mut server, b"a");
        let b = session(&mut server, b"b");
        let fallback = session(&mut server, b"c");
        let (_, link, _) = window(&mut server, a, 0);
        window(&mut server, fallback, 0);
        let group = session::session_group_new(&mut server, b"g");
        session::session_group_add(&mut server, group, a);
        session::session_group_add(&mut server, group, b);
        session::session_group_synchronize_to(&mut server, b);
        option(&mut server, a, b"detach-on-destroy", 0);
        option(&mut server, b, b"detach-on-destroy", 0);
        server.sessions.get_mut(fallback).unwrap().activity = (100, 0);
        let ca = client(&mut server, a);
        let cb = client(&mut server, b);
        server_unlink_window(&mut server, a, link).unwrap();
        for s in [a, b] {
            assert!(!session::session_alive(&server, s));
        }
        for c in [ca, cb] {
            assert_eq!(server.clients.get(c).unwrap().session, Some(fallback));
        }
        assert!(server.groups.get(group).is_none());
    }

    #[test]
    fn no_status_ready_defers_model_removal_and_single_neighbor_wraps_to_none() {
        let mut server = Server::new();
        let s = session(&mut server, b"s");
        let (_, _, pane) = window(&mut server, s, 0);
        server_destroy_pane(&mut server, pane, true).unwrap();
        assert!(server.panes.get(pane).is_some());
        for choice in [3, 4] {
            option(&mut server, s, b"detach-on-destroy", choice);
            assert_eq!(replacement(&server, s), (None, None));
        }
    }

    fn kill_during_died(server: &mut Server, payload: &mut EventPayload) {
        server_kill_pane(server, payload.get_pane(b"pane").unwrap()).unwrap();
    }

    #[test]
    fn died_sink_can_kill_its_pane_without_post_callback_model_access() {
        let mut server = Server::new();
        let s = session(&mut server, b"s");
        let (w, _, pane) = window(&mut server, s, 0);
        let options = server.panes.get(pane).unwrap().options;
        server
            .options
            .set_number_value(options, b"remain-on-exit", 1);
        server
            .panes
            .get_mut(pane)
            .unwrap()
            .flags
            .insert(PaneFlags::STATUSREADY);
        events::add_sink(&mut server, b"pane-died", kill_during_died);
        server_destroy_pane(&mut server, pane, true).unwrap();
        assert!(server.panes.get(pane).is_none());
        assert!(server.windows.get(w).is_none());
        assert!(!session::session_alive(&server, s));
    }

    fn unlinked_before_removal(server: &mut Server, payload: &mut EventPayload) {
        let state = payload.get_target(server, CmdFindFlags::default());
        assert!(state.is_valid(server));
        let session = server.sessions.get(state.s.unwrap()).unwrap();
        assert_eq!(session.current, state.wl);
        let pane = state.wp.unwrap();
        server.panes.get_mut(pane).unwrap().cmd_status += 1;
    }

    #[test]
    fn replacing_current_link_fires_before_removal_and_forces_selection() {
        let mut server = Server::new();
        let src = session(&mut server, b"src");
        let dst = session(&mut server, b"dst");
        let (source, source_link, _) = window(&mut server, src, 0);
        let (_, old, old_pane) = window(&mut server, dst, 0);
        server.panes.get_mut(old_pane).unwrap().cmd_status = 0;
        crate::model::pane::pane_retain(&mut server, old_pane).unwrap();
        events::add_sink(&mut server, b"window-unlinked", unlinked_before_removal);
        let new = server_link_window(&mut server, src, source_link, dst, 0, true, false).unwrap();
        assert_eq!(server.panes.get(old_pane).unwrap().cmd_status, 1);
        assert!(server.winlinks.get(old).is_none());
        assert_eq!(server.sessions.get(dst).unwrap().current, Some(new));
        assert_eq!(server.winlinks.get(new).unwrap().window, source);
        crate::model::pane::pane_release(&mut server, old_pane).unwrap();
    }

    #[test]
    fn destroyed_marked_pane_does_not_match_reused_arena_slot() {
        let mut server = Server::new();
        let s = session(&mut server, b"s");
        let (w, link, pane) = window(&mut server, s, 0);
        server_set_marked(&mut server, Some(s), Some(link), Some(pane));
        window::window_remove_pane(&mut server, w, pane).unwrap();
        let new = window::window_add_pane(
            &mut server,
            w,
            None,
            0,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        assert_ne!(pane, new);
        assert!(!server_check_marked(&server));
        assert!(!server_is_marked(&server, Some(s), Some(link), Some(new)));
    }
}
