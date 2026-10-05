// Ported from tmux server-client.c, tty.c @ 8f25579c
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

//! Server-side client life cycle (`server-client.c:158-543`): create, open,
//! set session, lost, reference leases and deferred free, suspend, detach,
//! exec and the ready message, plus the tty window offset
//! (`tty.c:972-1073`).

use std::os::fd::{AsFd, BorrowedFd};
use std::time::Duration;

use crate::client::{Client, ClientCreateError, ClientExitType, ClientFlags};
use crate::cmd::find::{self, CmdFindFlags};
use crate::ids::{ArenaError, ClientId, SessionId, WindowId};
use crate::model::WinlinkFlags;
use crate::model::alerts::alerts_check_session;
use crate::model::session::{session_theme_changed, session_update_activity};
use crate::options::OptionsArrayKey;
use crate::server::Server;
use crate::server::event_loop::LoopAction;
use crate::server::events::{self, EventPayload};
use crate::server::operations::{server_check_unattached, server_redraw_client};
use crate::server::proc::{proc_remove, proc_send};
use crate::server::protocol::{ProtocolError, ProtocolMessage, ProtocolMessageKind, encode_string};
use crate::server::run::{server_add_accept, server_update_socket};
use rmux_emu::screen::ScreenMode;
use rmux_tty::keys::tables::CODE_KEYS;
use rmux_tty::term::terminfo::CapList;
use rmux_tty::tty::{TtyFlags, TtyOptions};
use rmux_util::bytes::ByteString;
use rmux_util::log_debug;
use rmux_util::time::Timestamp;

/// `gettimeofday` as the `(sec, usec)` pair the model stores.
pub fn now() -> (i64, i64) {
    let t = Timestamp::now();
    (t.sec, i64::from(t.usec))
}

/// The tty options G07 reads from the global option tree (`TtyOptions`
/// replaces `options_get_*(global_options, ...)` inside `tty.c`).
pub fn tty_options(server: &Server) -> TtyOptions {
    let global = server.options.global;
    let array = |name: &[u8]| -> Vec<ByteString> {
        server
            .options
            .get(global, name)
            .map_or_else(Vec::new, |(_, entry)| {
                entry
                    .array_items()
                    .map(|(_, item)| ByteString::from(item.value().as_string()))
                    .collect()
            })
    };
    TtyOptions {
        clear_on_attach: server.options.get_number(global, b"clear-on-attach") != 0,
        extended_keys: server.options.get_number(global, b"extended-keys") != 0,
        focus_events: server.options.get_number(global, b"focus-events") != 0,
        default_terminal: ByteString::from(server.options.get_string(global, b"default-terminal")),
        terminal_overrides: array(b"terminal-overrides"),
        terminal_features: array(b"terminal-features"),
    }
}

/// `server_client_create` (`server-client.c:158-206`). The arena slot
/// starts with the connection lease; the peer is registered with the event
/// loop and the `root` key table is referenced. Partial setup is undone on
/// failure.
pub fn create(server: &mut Server, fd: rmux_sys::OwnedFd) -> Result<ClientId, ClientCreateError> {
    let peer = server.process.add_peer(fd)?;
    if let Err(error) = server.process.update_event(peer, &mut server.event_loop) {
        proc_remove(server, peer);
        return Err(error.into());
    }
    let id = match server.clients.insert(Client::new(Some(peer), now())) {
        Ok(id) => id,
        Err(error) => {
            proc_remove(server, peer);
            return Err(error.into());
        }
    };
    server.clients.retain(id).expect("fresh client slot");
    let table = match server.key_bindings.get_table(b"root", true) {
        Ok(Some(table)) => table,
        Ok(None) => unreachable!("get_table(create = true) always yields a table"),
        Err(error) => {
            let _ = server.clients.request_remove(id);
            proc_remove(server, peer);
            return Err(error.into());
        }
    };
    if let Err(error) = server.key_bindings.retain_table(table) {
        let _ = server.clients.request_remove(id);
        proc_remove(server, peer);
        return Err(error.into());
    }
    server.clients.get_mut(id).expect("fresh client").keytable = Some(table);
    server.client_order.push_back(id);
    log_debug!("new client {id:?}");
    Ok(id)
}

/// The tty names of the server's own standard streams (`server-client.c:218-226`).
fn own_tty_names() -> Vec<Vec<u8>> {
    let (stdin, stdout, stderr) = (std::io::stdin(), std::io::stdout(), std::io::stderr());
    let fds: [BorrowedFd<'_>; 3] = [stdin.as_fd(), stdout.as_fd(), stderr.as_fd()];
    fds.iter()
        .filter(|fd| rmux_sys::fd::isatty(**fd))
        .filter_map(|fd| rmux_sys::client::ttyname(*fd))
        .collect()
}

/// `server_client_open` (`server-client.c:209-241`): a control client needs
/// nothing; the tty must not be `/dev/tty` or one of the server's own ttys
/// (`can't use %s`), the client must be a terminal (`not a terminal`), then
/// the tty is opened and the theme colours computed.
pub fn open(server: &mut Server, id: ClientId) -> Result<(), Vec<u8>> {
    let Some(c) = server.clients.get(id) else {
        return Err(b"not a terminal".to_vec());
    };
    if c.flags.intersects(ClientFlags::CONTROL) {
        return Ok(());
    }
    let ttyname = c.ttyname.clone().unwrap_or_default();
    if ttyname == b"/dev/tty" || own_tty_names().contains(&ttyname) {
        let mut cause = b"can't use ".to_vec();
        cause.extend_from_slice(&ttyname);
        return Err(cause);
    }
    if !c.flags.intersects(ClientFlags::TERMINAL) || c.tty.is_none() {
        return Err(b"not a terminal".to_vec());
    }

    let opts = tty_options(server);
    let Server { clients, tparm, .. } = server;
    let c = clients.get_mut(id).expect("client checked above");
    let name = c.term_name.clone().unwrap_or_else(|| b"unknown".to_vec());
    let caps: CapList = c
        .term_caps
        .iter()
        .map(|cap| ByteString::from(cap.as_slice()))
        .collect();
    let colorterm = c
        .environ
        .find(b"COLORTERM")
        .and_then(|entry| entry.value.as_ref())
        .map(|value| value.as_bytes().to_vec());
    let tty = c.tty.as_mut().expect("terminal client has a tty");
    tty.open(tparm, &name, &caps, &opts, colorterm.as_deref())
        .map_err(ByteString::into_vec)?;

    crate::client::theme::update_theme_colours(server, id);
    crate::client::tty_io::sync(server, id);
    Ok(())
}

/// `server_client_attached_lost` (`server-client.c:244-274`): hand `latest`
/// of every window this client owned to the most recently active other
/// client showing that window.
fn attached_lost(server: &mut Server, id: ClientId) {
    log_debug!("lost attached client {id:?}");
    let windows: Vec<WindowId> = server
        .window_ids
        .values()
        .copied()
        .filter(|w| server.windows.get(*w).is_some_and(|w| w.latest == Some(id)))
        .collect();
    for w in windows {
        let mut found: Option<(ClientId, (i64, i64))> = None;
        for loop_id in server.client_order.clone() {
            if loop_id == id {
                continue;
            }
            let Some(c) = server.clients.get(loop_id) else {
                continue;
            };
            let current = c
                .session
                .and_then(|s| server.sessions.get(s))
                .and_then(|s| s.current)
                .and_then(|wl| server.winlinks.get(wl))
                .map(|wl| wl.window);
            if current != Some(w) {
                continue;
            }
            if found.is_none_or(|(_, activity)| c.activity_time > activity) {
                found = Some((loop_id, c.activity_time));
            }
        }
        if let Some((found, _)) = found {
            crate::client::keys::update_latest(server, found);
        }
    }
}

/// The target part shared by the session-changed and resized payloads
/// (`server-client.c:284-300,312-324`).
fn target_payload(server: &mut Server, id: ClientId) -> (EventPayload, find::CmdFindState) {
    let fs = find::from_client(server, Some(id), CmdFindFlags::default()).unwrap_or_default();
    let mut ep = EventPayload::new();
    ep.set_target(server, &fs);
    ep.set_client(server, b"client", id);
    (ep, fs)
}

fn add_window_fields(server: &mut Server, ep: &mut EventPayload, fs: &find::CmdFindState) {
    if let Some(w) = fs.w {
        ep.set_window(server, b"window", w);
    }
    if let Some(wl) = fs.wl.and_then(|id| server.winlinks.get(id)) {
        let index = wl.index;
        ep.set_int(server, b"window_index", index);
    } else if fs.idx != -1 {
        ep.set_int(server, b"window_index", fs.idx);
    }
    if let Some(wp) = fs.wp {
        ep.set_pane(server, b"pane", wp);
    }
}

/// `server_client_fire_session_changed` (`server-client.c:277-302`).
pub(crate) fn fire_session_changed(server: &mut Server, id: ClientId, old: Option<SessionId>) {
    let (mut ep, fs) = target_payload(server, id);
    if let Some(s) = fs.s {
        ep.set_session(server, b"session", s);
        ep.set_session(server, b"new_session", s);
    }
    if let Some(old) = old
        && server.sessions.get(old).is_some()
    {
        ep.set_session(server, b"old_session", old);
    }
    add_window_fields(server, &mut ep, &fs);
    events::fire(server, b"client-session-changed", ep);
}

/// `server_client_fire_resized` (`server-client.c:305-330`).
pub(crate) fn fire_resized(server: &mut Server, id: ClientId, old_sx: u32, old_sy: u32) {
    let (mut ep, fs) = target_payload(server, id);
    if let Some(s) = fs.s {
        ep.set_session(server, b"session", s);
    }
    add_window_fields(server, &mut ep, &fs);
    let (sx, sy) = server
        .clients
        .get(id)
        .map_or((old_sx, old_sy), Client::tty_size);
    ep.set_uint(server, b"width", sx);
    ep.set_uint(server, b"height", sy);
    ep.set_uint(server, b"old_width", old_sx);
    ep.set_uint(server, b"old_height", old_sy);
    events::fire(server, b"client-resized", ep);
}

/// `window_pane_update_focus` client loop (`window.c:718-733`): a window is
/// focused when a client with an attached session, `CLIENT_FOCUSED` and this
/// window current exists. Stores `Window.focused`, then runs the model's
/// `window_update_focus` which applies it to the active pane.
pub fn update_window_focus(server: &mut Server, window: WindowId) {
    let focused = server.client_order.iter().any(|id| {
        server.clients.get(*id).is_some_and(|c| {
            c.flags.intersects(ClientFlags::FOCUSED)
                && c.session
                    .and_then(|s| server.sessions.get(s))
                    .filter(|s| s.attached != 0)
                    .and_then(|s| s.current)
                    .and_then(|wl| server.winlinks.get(wl))
                    .is_some_and(|wl| wl.window == window)
        })
    });
    if let Some(w) = server.windows.get_mut(window) {
        w.focused = focused;
    }
    crate::model::window::window_update_focus(server, window);
}

/// `server_client_set_session` (`server-client.c:333-370`).
pub fn set_session(server: &mut Server, id: ClientId, s: Option<SessionId>) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let old = c.session;
    if s.is_some() && c.session.is_some() && c.session != s {
        c.last_session = c.session;
    } else if s.is_none() {
        c.last_session = None;
    }
    c.session = s;
    c.flags.insert(ClientFlags::FOCUSED);

    if let Some(old_window) = old
        .and_then(|old| server.sessions.get(old))
        .and_then(|old| old.current)
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.window)
    {
        update_window_focus(server, old_window);
    }
    if let Some(s) = s {
        let Some((curw, window)) = server
            .sessions
            .get(s)
            .and_then(|s| s.current)
            .and_then(|wl| server.winlinks.get(wl).map(|l| (wl, l.window)))
        else {
            server_check_unattached(server);
            server_update_socket(server);
            return;
        };
        if let Some(w) = server.windows.get_mut(window) {
            w.latest = Some(id);
        }
        crate::server::run::recalculate_sizes(server);
        update_window_focus(server, window);
        session_update_activity(server, s, None);
        session_theme_changed(server, Some(s));
        if let Some(session) = server.sessions.get_mut(s) {
            session.last_attached = now();
        }
        if let Some(wl) = server.winlinks.get_mut(curw) {
            wl.flags.remove(WinlinkFlags::ALERTFLAGS);
        }
        alerts_check_session(server, s);
        update_offset(server, id);
        crate::ui::status::status_timer_start(server, id);
        fire_session_changed(server, id, old);

        // Redraw if the session or displayed window changed; the cached scene
        // tells whether the client already shows the current window.
        if old != Some(s) || !crate::ui::redraw::redraw_client_has_window(server, id, window) {
            server_redraw_client(server, id);
        }
    }

    server_check_unattached(server);
    server_update_socket(server);
}

/// Run a `CfgState` method that needs `&mut Server`: take the state out,
/// call, then merge back anything the call added through the runtime.
fn with_cfg(server: &mut Server, f: impl FnOnce(&mut crate::cmd::cfg::CfgState, &mut Server)) {
    let mut cfg = std::mem::take(&mut server.cfg);
    f(&mut cfg, server);
    let during = std::mem::replace(&mut server.cfg, cfg);
    server.cfg.causes.extend(during.causes);
}

/// `server_client_lost` (`server-client.c:373-454`), in the C order. The
/// arena slot stays addressable (flagged `CLIENT_DEAD`) until the deferred
/// free and the last lease drop.
pub fn lost(server: &mut Server, id: ClientId) {
    with_cfg(server, |cfg, server| cfg.client_lost(server, id));
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    c.flags.insert(ClientFlags::DEAD);
    crate::tsp::broker::client_sync(server, id);

    crate::ui::status::status_prompt_clear(server, id);
    crate::ui::status::status_message_clear(server, id);

    // Fail every file with EINTR and fire done.
    crate::server::file::lost_client(server, id);

    server.client_order.retain(|other| *other != id);
    log_debug!("lost client {}", client_label(server, id));

    crate::cmd::commands::wait_for::client_lost(server, id);
    crate::cmd::queue::next(server, Some(id));

    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.flags.intersects(ClientFlags::ATTACHED) {
        attached_lost(server, id);
        events::fire_client(server, b"client-detached", id);
    }
    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.name.is_some()
        && c.flags
            .intersects(ClientFlags::CONTROL | ClientFlags::TERMINAL)
    {
        events::fire_client(server, b"client-closed", id);
    }

    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.flags.intersects(ClientFlags::CONTROL) {
        crate::control::stop(server, id);
    }
    {
        let Server {
            clients,
            tparm,
            event_loop,
            ..
        } = server;
        let Some(c) = clients.get_mut(id) else {
            return;
        };
        // tty_free: event_del on the fd and the four timers before close.
        if let Some(token) = c.tty_token.take() {
            event_loop.deregister(token);
        }
        c.tty_timers.cancel_all(event_loop);
        if c.flags.intersects(ClientFlags::TERMINAL)
            && let Some(mut tty) = c.tty.take()
        {
            tty.close(tparm);
            drop(tty);
        }
        c.tty = None;
        c.ttyname = None;
        c.clipboard_panes = Vec::new();
        c.term_name = None;
        c.term_type = None;
        c.term_caps = Vec::new();
        c.status = Default::default();
    }
    crate::tsp::broker::client_sync(server, id);
    crate::client::input_requests::cancel_all(server, id);

    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    c.title = None;
    c.path = None;
    c.cwd = None;
    c.exit_session = None;
    c.exit_message = None;

    let timers = [
        c.repeat_timer.take(),
        c.click.timer.take(),
        c.exit_timer.take(),
        c.cycle_timer.take(),
    ];
    let keytable = c.keytable.take();
    for timer in timers.into_iter().flatten() {
        server.event_loop.cancel(timer);
    }
    if let Some(table) = keytable {
        let _ = server.key_bindings.unref_table(table);
    }

    if let Some(c) = server.clients.get_mut(id) {
        c.message = Default::default();
        c.prompt = Default::default();
    }

    crate::server::run::format_lost_client(server, id);
    if let Some(c) = server.clients.get_mut(id) {
        c.environ = Default::default();
    }

    let peer = server.clients.get_mut(id).and_then(|c| c.peer.take());
    if let Some(peer) = peer {
        proc_remove(server, peer);
    }

    if let Some(c) = server.clients.get_mut(id) {
        c.out_fd = None;
        c.fd = None;
    }
    // server_client_unref: drop the connection lease and schedule the
    // deferred free (`server-client.c:447,457-465`).
    let _ = release(server, id);
    if let Some(c) = server.clients.get_mut(id)
        && c.free_scheduled.is_none()
    {
        c.free_scheduled = Some(
            server
                .event_loop
                .schedule(Duration::ZERO, LoopAction::ClientFree(id)),
        );
    }

    server_add_accept(server, 0); // may be more file descriptors now

    crate::server::run::recalculate_sizes(server);
    server_check_unattached(server);
    server_update_socket(server);
}

/// `c->references++`: one more lease on the arena slot.
pub fn retain(server: &mut Server, id: ClientId) -> Result<(), ArenaError> {
    server.clients.retain(id)
}

/// `server_client_unref` (`server-client.c:457-465`) for a lease other than
/// the connection: drop it. When the slot is already pending removal (the
/// deferred free ran) and this was the last lease, the client is dropped
/// here, like the final `free(c)` at `server-client.c:481`.
pub fn release(server: &mut Server, id: ClientId) -> Result<(), ArenaError> {
    if let Some(client) = server.clients.release(id)? {
        log_debug!("free client {} (0 references)", client_label_from(&client));
        drop(client);
    }
    Ok(())
}

/// `%p` of the C logs is the client name in rmux logs (the harness greps
/// `lost client <name>`), with the id for unnamed clients.
fn client_label(server: &Server, id: ClientId) -> String {
    server
        .clients
        .get(id)
        .map_or_else(|| format!("{id:?}"), client_label_from)
}

fn client_label_from(client: &Client) -> String {
    if let Some(name) = client.name.as_deref() {
        String::from_utf8_lossy(name).into_owned()
    } else if let Some(pid) = client.pid {
        format!("client-{}", pid.0)
    } else {
        String::new()
    }
}

/// `server_client_free` (`server-client.c:468-483`), run from
/// `LoopAction::ClientFree`: drop the redraw scene and the command queue
/// (which may release queue leases), then request removal of the slot. The
/// slot disappears now when no lease remains, else at the last `release`.
pub fn free(server: &mut Server, id: ClientId) {
    log_debug!(
        "free client {} ({} references)",
        client_label(server, id),
        server.clients.leases(id).unwrap_or(0)
    );
    if let Some(c) = server.clients.get_mut(id) {
        c.free_scheduled = None;
        c.redraw_scene = None;
    }
    // cmdq_free (cmd-queue.c) only drops an already drained list.
    server.queue.clients.remove(&id);
    if let Ok(Some(client)) = server.clients.request_remove(id) {
        drop(client);
    }
}

/// `server_client_suspend` (`server-client.c:486-497`).
pub fn suspend(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.session.is_none() || c.flags.intersects(ClientFlags::UNATTACHEDFLAGS) {
        return;
    }
    let opts = tty_options(server);
    let Server { clients, tparm, .. } = server;
    let c = clients.get_mut(id).expect("client checked above");
    if let Some(tty) = c.tty.as_mut() {
        tty.stop(tparm, &opts);
    }
    c.flags.insert(ClientFlags::SUSPENDED);
    if let Some(peer) = c.peer {
        let _ = proc_send(
            server,
            peer,
            ProtocolMessage::new(ProtocolMessageKind::Suspend, Vec::new()),
        );
    }
    crate::client::tty_io::sync(server, id);
}

/// `server_client_detach` (`server-client.c:500-513`): `kill_parent` picks
/// `MSG_DETACHKILL` over `MSG_DETACH`.
pub fn detach(server: &mut Server, id: ClientId, kill_parent: bool) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    let Some(session) = c.session.and_then(|s| server.sessions.get(s)) else {
        return;
    };
    if c.flags.intersects(ClientFlags::NODETACHFLAGS) {
        return;
    }
    let name = session.name.clone();
    let c = server.clients.get_mut(id).expect("client checked above");
    c.flags.insert(ClientFlags::EXIT);
    c.exit_type = ClientExitType::Detach;
    c.exit_msgtype = if kill_parent {
        ProtocolMessageKind::DetachKill
    } else {
        ProtocolMessageKind::Detach
    };
    c.exit_session = Some(name);
}

/// `server_client_exec` (`server-client.c:516-542`): send `MSG_EXEC` with
/// the command and the session or global `default-shell` (or `/bin/sh`).
pub fn exec(server: &mut Server, id: ClientId, cmd: &[u8]) {
    if cmd.is_empty() || cmd[0] == 0 {
        return;
    }
    let Some(c) = server.clients.get(id) else {
        return;
    };
    let options = c
        .session
        .and_then(|s| server.sessions.get(s))
        .map_or(server.options.global_s, |s| s.options);
    let mut shell = server.options.get_string(options, b"default-shell");
    if !rmux_util::shell::check_shell(shell, b"rmux") {
        shell = b"/bin/sh";
    }
    let mut data = encode_string(rmux_util::bytes::cstr(cmd));
    data.extend(encode_string(rmux_util::bytes::cstr(shell)));
    if let Some(peer) = c.peer {
        let _ = proc_send(
            server,
            peer,
            ProtocolMessage::new(ProtocolMessageKind::Exec, data),
        );
    }
}

/// Send `MSG_READY` (`cmd-attach-session.c:163`, `cmd-new-session.c:334`).
/// Only the message: G20 owns the session and `CLIENT_ATTACHED` order.
pub fn ready(server: &mut Server, id: ClientId) -> Result<(), ProtocolError> {
    let peer = server
        .clients
        .get(id)
        .and_then(|c| c.peer)
        .ok_or(ProtocolError::Closed)?;
    proc_send(
        server,
        peer,
        ProtocolMessage::new(ProtocolMessageKind::Ready, Vec::new()),
    )
}

/// `tty_window_offset` (`tty.c:960-969`): the stored offset
/// `(oflag, ox, oy, sx, sy)`.
pub fn window_offset(server: &Server, id: ClientId) -> (bool, u32, u32, u32, u32) {
    server
        .clients
        .get(id)
        .and_then(|c| c.tty.as_ref())
        .map_or((false, 0, 0, 0, 0), |tty| tty.window_offset())
}

/// `tty_window_offset1` (`tty.c:972-1033`): compute where the current
/// window is drawn for this client, updating the pan state.
fn window_offset1(server: &mut Server, id: ClientId) -> Option<(bool, u32, u32, u32, u32)> {
    let lines = crate::ui::status::status_line_size(server, id);
    let c = server.clients.get(id)?;
    let (tty_sx, tty_sy) = c.tty_size();
    let w_id = server
        .sessions
        .get(c.session?)?
        .current
        .and_then(|wl| server.winlinks.get(wl))?
        .window;
    let w = server.windows.get(w_id)?;
    let (w_sx, w_sy) = (w.sx, w.sy);
    let active = w.active.and_then(|wp| server.panes.get(wp));

    if tty_sx >= w_sx && tty_sy.wrapping_sub(lines) >= w_sy {
        server.clients.get_mut(id)?.pan_window = None;
        return Some((false, 0, 0, w_sx, w_sy));
    }

    let sx = tty_sx;
    let sy = tty_sy.wrapping_sub(lines);

    if c.pan_window == Some(w_id) {
        let c = server.clients.get_mut(id)?;
        if sx >= w_sx {
            c.pan_ox = 0;
        } else if c.pan_ox + sx > w_sx {
            c.pan_ox = w_sx - sx;
        }
        let ox = c.pan_ox;
        if sy >= w_sy {
            c.pan_oy = 0;
        } else if c.pan_oy + sy > w_sy {
            c.pan_oy = w_sy - sy;
        }
        let oy = c.pan_oy;
        return Some((true, ox, oy, sx, sy));
    }

    let (ox, oy) = match active {
        Some(wp) if wp.screen().mode.contains(ScreenMode::CURSOR) => {
            let screen = wp.screen();
            let cx = wp.xoff.wrapping_add(screen.cx as i32) as u32;
            let cy = wp.yoff.wrapping_add(screen.cy as i32) as u32;
            let ox = if cx < sx {
                0
            } else if cx > w_sx.wrapping_sub(sx) {
                w_sx.wrapping_sub(sx)
            } else {
                cx - sx / 2
            };
            let oy = if cy < sy {
                0
            } else if cy > w_sy.wrapping_sub(sy) {
                w_sy.wrapping_sub(sy)
            } else {
                cy - sy + 1
            };
            (ox, oy)
        }
        _ => (0, 0),
    };
    server.clients.get_mut(id)?.pan_window = None;
    Some((true, ox, oy, sx, sy))
}

/// `tty_update_client_offset` (`tty.c:1050-1073`): recompute the window
/// offset of a terminal client; a change marks the window and status for
/// redraw.
pub fn update_offset(server: &mut Server, id: ClientId) {
    if !server
        .clients
        .get(id)
        .is_some_and(|c| c.flags.intersects(ClientFlags::TERMINAL) && c.tty.is_some())
    {
        return;
    }
    let Some((oflag, ox, oy, sx, sy)) = window_offset1(server, id) else {
        return;
    };
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let name = String::from_utf8_lossy(c.name_bytes()).into_owned();
    let Some(tty) = c.tty.as_mut() else {
        return;
    };
    let (_, oox, ooy, osx, osy) = tty.window_offset();
    if ox == oox && oy == ooy && sx == osx && sy == osy {
        tty.set_window_offset(oflag, oox, ooy, osx, osy);
        return;
    }
    log_debug!(
        "tty_update_client_offset: {name} offset has changed ({oox},{ooy} {osx}x{osy} -> {ox},{oy} {sx}x{sy})"
    );
    tty.set_window_offset(oflag, ox, oy, sx, sy);
    c.flags
        .insert(ClientFlags::REDRAWWINDOW | ClientFlags::REDRAWSTATUS);
}

/// `tty_keys_build` (`tty-keys.c:505-540`) for a client whose tty is open:
/// rebuild the key decoder from the terminal strings and the global
/// `user-keys` array.
pub fn rebuild_tty_keys(server: &mut Server, id: ClientId) {
    let user_keys: Vec<(u32, Vec<u8>)> = server
        .options
        .get(server.options.global, b"user-keys")
        .map_or_else(Vec::new, |(_, entry)| {
            entry
                .array_items()
                .filter_map(|(key, item)| match key {
                    OptionsArrayKey::Index(index) => {
                        Some((*index, item.value().as_string().to_vec()))
                    }
                    OptionsArrayKey::Name(_) => None,
                })
                .collect()
        });
    let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) else {
        return;
    };
    if !tty.flags().contains(TtyFlags::OPENED) {
        return;
    }
    let caps: Vec<(rmux_tty::term::TtyCodeCode, Vec<u8>)> = {
        let term = tty.term();
        CODE_KEYS
            .iter()
            .map(|(code, _)| {
                (
                    *code,
                    if term.has(*code) {
                        term.string(*code).to_vec()
                    } else {
                        Vec::new()
                    },
                )
            })
            .collect()
    };
    tty.keys_mut().rebuild_with(
        |code| {
            caps.iter()
                .find(|(c, _)| *c == code)
                .map_or(&[][..], |(_, s)| s.as_slice())
        },
        user_keys.iter().map(|(index, s)| (*index, s.as_slice())),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::session::{self, SessionCreate};
    use crate::options::environment::Environment;

    fn session(server: &mut Server, name: &[u8]) -> SessionId {
        let options = server.options.create(Some(server.options.global_s));
        session::session_create(
            server,
            SessionCreate {
                prefix: None,
                name: Some(name.to_vec()),
                cwd: b"/".to_vec(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        )
    }

    fn client(server: &mut Server) -> ClientId {
        let id = server.clients.insert(Client::new(None, now())).unwrap();
        server.clients.retain(id).unwrap();
        server.client_order.push_back(id);
        id
    }

    #[test]
    fn detach_records_exit_state() {
        let mut server = Server::new();
        let s = session(&mut server, b"main");
        let id = client(&mut server);
        detach(&mut server, id, false);
        assert!(
            !server
                .clients
                .get(id)
                .unwrap()
                .flags
                .intersects(ClientFlags::EXIT)
        );

        server.clients.get_mut(id).unwrap().session = Some(s);
        detach(&mut server, id, true);
        let c = server.clients.get(id).unwrap();
        assert!(c.flags.intersects(ClientFlags::EXIT));
        assert_eq!(c.exit_type, ClientExitType::Detach);
        assert_eq!(c.exit_msgtype, ProtocolMessageKind::DetachKill);
        assert_eq!(c.exit_session.as_deref(), Some(&b"main"[..]));

        // NODETACHFLAGS blocks a second detach.
        let c = server.clients.get_mut(id).unwrap();
        c.flags.remove(ClientFlags::EXIT);
        c.flags.insert(ClientFlags::DEAD);
        c.exit_msgtype = ProtocolMessageKind::Exit;
        detach(&mut server, id, false);
        assert_eq!(
            server.clients.get(id).unwrap().exit_msgtype,
            ProtocolMessageKind::Exit
        );
    }

    #[test]
    fn release_drops_the_slot_only_once_pending() {
        let mut server = Server::new();
        let id = client(&mut server);
        retain(&mut server, id).unwrap();
        release(&mut server, id).unwrap();
        assert!(server.clients.get(id).is_some());
        assert!(matches!(server.clients.request_remove(id), Ok(None)));
        assert!(server.clients.get(id).is_some());
        release(&mut server, id).unwrap();
        assert!(server.clients.get(id).is_none());
        assert_eq!(release(&mut server, id), Err(ArenaError::StaleId));
    }

    #[test]
    fn tty_options_follow_global_options() {
        let mut server = Server::new();
        let opts = tty_options(&server);
        assert_eq!(
            opts.default_terminal.as_bytes(),
            server
                .options
                .get_string(server.options.global, b"default-terminal")
        );
        assert_eq!(
            opts.focus_events,
            server
                .options
                .get_number(server.options.global, b"focus-events")
                != 0
        );
        let global = server.options.global;
        server.options.set_number_value(global, b"focus-events", 1);
        server.options.set_number_value(global, b"extended-keys", 0);
        let opts = tty_options(&server);
        assert!(opts.focus_events);
        assert!(!opts.extended_keys);
    }

    #[test]
    fn window_offset_without_tty_is_unset() {
        let mut server = Server::new();
        let id = client(&mut server);
        assert_eq!(window_offset(&server, id), (false, 0, 0, 0, 0));
        update_offset(&mut server, id);
        assert_eq!(window_offset(&server, id), (false, 0, 0, 0, 0));
    }
}
