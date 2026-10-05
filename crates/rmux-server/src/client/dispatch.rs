// Ported from tmux server-client.c @ 8f25579c
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

//! Semantic dispatch of client messages (`server-client.c:2543-2942`):
//! identify, command, resize, exiting, wakeup/unlock, shell, and the file
//! replies forwarded to G14. Wire shape errors kill the peer.

use std::os::fd::AsFd;
use std::time::SystemTime;

use crate::client::lifecycle::{self, tty_options};
use crate::client::{Client, ClientFlags, MouseDragState, PASTE_TIME_LIMIT};
use crate::cmd::parse::{self, CmdParseInput};
use crate::cmd::queue::{self, CmdReturn, QueueBatch, callback_for};
use crate::cmd::{CommandFlags, arguments};
use crate::ids::{ClientId, QueueItemId};
use crate::model::session::session_update_activity;
use crate::server::Server;
use crate::server::events;
use crate::server::operations::server_redraw_client;
use crate::server::proc::proc_send;
use crate::server::protocol::{
    ProtocolError, ProtocolMessage, ProtocolMessageKind as Kind, decode_i32, decode_string,
    encode_string, unpack_argv,
};
use rmux_sys::TermiosState;
use rmux_tty::tty::{Tty, TtyHostInfo};
use rmux_util::bytes::ByteString;
use rmux_util::log_debug;

/// A payload that does not fit its message (`goto bad`).
#[derive(Debug)]
struct Bad;

impl From<ProtocolError> for Bad {
    fn from(_: ProtocolError) -> Self {
        Bad
    }
}

/// `server_client_dispatch` with `imsg == NULL` (`server-client.c:2554-2557`):
/// the transport closed.
pub fn on_closed(server: &mut Server, id: ClientId) {
    if server.clients.get(id).is_none_or(Client::is_dead) {
        return;
    }
    lifecycle::lost(server, id);
}

/// `server_client_dispatch` (`server-client.c:2543-2657`).
pub fn on_message(server: &mut Server, id: ClientId, message: ProtocolMessage) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.is_dead() {
        return;
    }
    let kind = message.kind;
    let result = match kind {
        Kind::IdentifyClientpid
        | Kind::IdentifyCwd
        | Kind::IdentifyEnviron
        | Kind::IdentifyFeatures
        | Kind::IdentifyFlags
        | Kind::IdentifyLongflags
        | Kind::IdentifyStdin
        | Kind::IdentifyStdout
        | Kind::IdentifyTerm
        | Kind::IdentifyTerminfo
        | Kind::IdentifyTtyname
        | Kind::IdentifyDone => dispatch_identify(server, id, message),
        Kind::Command => dispatch_command(server, id, &message.data),
        Kind::Resize => {
            if !message.data.is_empty() {
                Err(Bad)
            } else {
                resize(server, id);
                Ok(())
            }
        }
        Kind::Exiting => {
            if !message.data.is_empty() {
                Err(Bad)
            } else {
                exiting(server, id);
                Ok(())
            }
        }
        Kind::Wakeup | Kind::Unlock => {
            if !message.data.is_empty() {
                Err(Bad)
            } else {
                wakeup(server, id);
                Ok(())
            }
        }
        Kind::Shell => {
            if !message.data.is_empty() {
                Err(Bad)
            } else {
                dispatch_shell(server, id);
                Ok(())
            }
        }
        Kind::WriteReady | Kind::WriteDone | Kind::ReadData | Kind::ReadDone => {
            crate::server::file::handle_server(server, id, message).map_err(|_| Bad)
        }
        _ => Ok(()),
    };
    if result.is_err() {
        log_debug!("client {id:?} invalid message type {kind:?}");
        if let Some(peer) = server.clients.get(id).and_then(|c| c.peer) {
            server.process.kill_peer(peer);
        }
    }
}

/// `MSG_RESIZE` (`server-client.c:2581-2596`): ignored for control clients;
/// otherwise update latest, resize the tty, repeat requests, recalculate,
/// redraw, and fire `client-resized` when attached to a session.
fn resize(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.flags.intersects(ClientFlags::CONTROL) {
        return;
    }
    crate::client::keys::update_latest(server, id);
    let Server { clients, tparm, .. } = server;
    let Some(c) = clients.get_mut(id) else {
        return;
    };
    let (old_sx, old_sy) = c.tty_size();
    if let Some(tty) = c.tty.as_mut() {
        tty.resize(tparm);
        tty.repeat_requests(false, SystemTime::now());
    }
    crate::server::run::recalculate_sizes(server);
    server_redraw_client(server, id);
    if server.clients.get(id).is_some_and(|c| c.session.is_some()) {
        lifecycle::fire_resized(server, id, old_sx, old_sy);
    }
}

/// `MSG_EXITING` (`server-client.c:2597-2604`): clear the session,
/// recalculate, close the tty and answer `MSG_EXITED`.
fn exiting(server: &mut Server, id: ClientId) {
    lifecycle::set_session(server, id, None);
    crate::server::run::recalculate_sizes(server);
    let Server { clients, tparm, .. } = server;
    let Some(c) = clients.get_mut(id) else {
        return;
    };
    if let Some(tty) = c.tty.as_mut() {
        tty.close(tparm);
    }
    crate::client::tty_io::sync(server, id);
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    if let Some(peer) = c.peer {
        let _ = proc_send(server, peer, ProtocolMessage::new(Kind::Exited, Vec::new()));
    }
}

/// `MSG_WAKEUP` and `MSG_UNLOCK` (`server-client.c:2605-2627`): only a
/// suspended client resumes; without a tty or session it just clears the
/// flag.
fn wakeup(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    if !c.flags.intersects(ClientFlags::SUSPENDED) {
        return;
    }
    c.flags.remove(ClientFlags::SUSPENDED);
    if c.tty.is_none() || c.session.is_none() {
        return; // exited already
    }
    let session = c.session;
    let activity = lifecycle::now();
    c.activity_time = activity;

    let opts = tty_options(server);
    let Server { clients, tparm, .. } = server;
    let c = clients.get_mut(id).expect("client checked above");
    if let Some(tty) = c.tty.as_mut() {
        tty.start(tparm, &opts);
    }
    // tty_start_tty clears the drag flag and the two drag actions (tty.c:399-401).
    c.drag.flag = 0;
    c.drag.update = None;
    c.drag.release = None;
    crate::client::tty_io::sync(server, id);
    server_redraw_client(server, id);
    crate::server::run::recalculate_sizes(server);
    if let Some(s) = session {
        session_update_activity(server, s, Some(activity));
    }
}

/// `server_client_read_only` (`server-client.c:2660-2665`).
pub fn read_only(server: &mut Server, item: QueueItemId) -> CmdReturn {
    queue::error(server, item, b"client is read-only");
    CmdReturn::Error
}

fn read_only_batch(server: &mut Server) -> Result<QueueBatch, Bad> {
    server
        .queue
        .get_callback("server_client_read_only", callback_for::<Server>(read_only))
        .map_err(|_| Bad)
}

/// `server_client_default_command` (`server-client.c:2668-2683`): run the
/// `default-client-command` option, or the read-only error for a read-only
/// client with a command that is not `CMD_READONLY`.
pub fn default_command(server: &mut Server, item: QueueItemId) -> CmdReturn {
    let Some(client) = server.queue.items.get(item).and_then(|i| i.client) else {
        return CmdReturn::Normal;
    };
    let read_only_client = server
        .clients
        .get(client)
        .is_some_and(|c| c.flags.intersects(ClientFlags::READONLY));
    // options_get_command never yields NULL in C: the default always parses.
    let list = server
        .options
        .get_command(server.options.global, b"default-client-command")
        .cloned()
        .expect("default-client-command default parses");
    let batch = if read_only_client && !list.all_have(CommandFlags::READONLY) {
        read_only_batch(server)
    } else {
        server.queue.get_command(list, None).map_err(|_| Bad)
    };
    if let Ok(batch) = batch {
        let _ = queue::insert_after(server, item, batch);
    }
    CmdReturn::Normal
}

/// `server_client_command_done` (`server-client.c:2686-2699`): an unattached
/// client exits; an attached one that is not exiting gets `control_ready`
/// (control) and `tty_send_requests`.
pub fn command_done(server: &mut Server, item: QueueItemId) -> CmdReturn {
    let Some(client) = server.queue.items.get(item).and_then(|i| i.client) else {
        return CmdReturn::Normal;
    };
    let Some(c) = server.clients.get_mut(client) else {
        return CmdReturn::Normal;
    };
    if !c.flags.intersects(ClientFlags::ATTACHED) {
        c.flags.insert(ClientFlags::EXIT);
    } else if !c.flags.intersects(ClientFlags::EXIT) {
        let control = c.flags.intersects(ClientFlags::CONTROL);
        if control {
            crate::control::ready(server, client);
        }
        if let Some(tty) = server.clients.get_mut(client).and_then(|c| c.tty.as_mut()) {
            tty.send_requests(SystemTime::now());
        }
    }
    CmdReturn::Normal
}

/// `server_client_dispatch_command` (`server-client.c:2702-2769`).
fn dispatch_command(server: &mut Server, id: ClientId, data: &[u8]) -> Result<(), Bad> {
    let Some(c) = server.clients.get(id) else {
        return Ok(());
    };
    if c.flags.intersects(ClientFlags::EXIT) {
        return Ok(());
    }
    let read_only_client = c.flags.intersects(ClientFlags::READONLY);

    let argv: Vec<ByteString> = match unpack_argv(data) {
        Ok(argv) => argv.into_iter().map(ByteString::from).collect(),
        Err(ProtocolError::Shape("command too long")) => {
            return command_error(server, id, b"command too long");
        }
        Err(_) => return Err(Bad),
    };

    let batch = if argv.is_empty() {
        server
            .queue
            .get_callback(
                "server_client_default_command",
                callback_for::<Server>(default_command),
            )
            .map_err(|_| Bad)?
    } else {
        let values = arguments::from_vector(&argv);
        let mut input = CmdParseInput {
            client: Some(id),
            ..CmdParseInput::default()
        };
        let list = match parse::from_arguments(server, &values, &mut input) {
            Ok(list) => list,
            Err(error) => return command_error(server, id, error.message()),
        };
        if read_only_client && !list.all_have(CommandFlags::READONLY) {
            read_only_batch(server)?
        } else {
            server.queue.get_command(list, None).map_err(|_| Bad)?
        }
    };
    queue::append(server, Some(id), batch).map_err(|_| Bad)?;
    let done = server
        .queue
        .get_callback(
            "server_client_command_done",
            callback_for::<Server>(command_done),
        )
        .map_err(|_| Bad)?;
    queue::append(server, Some(id), done).map_err(|_| Bad)?;
    Ok(())
}

/// The `error:` tail of `server_client_dispatch_command`
/// (`server-client.c:2761-2768`): queue the error and mark the client to exit.
fn command_error(server: &mut Server, id: ClientId, cause: &[u8]) -> Result<(), Bad> {
    let batch = server.queue.get_error(cause).map_err(|_| Bad)?;
    queue::append(server, Some(id), batch).map_err(|_| Bad)?;
    if let Some(c) = server.clients.get_mut(id) {
        c.flags.insert(ClientFlags::EXIT);
    }
    Ok(())
}

/// Decode the little-endian 64-bit flags of `MSG_IDENTIFY_FLAGS` and
/// `MSG_IDENTIFY_LONGFLAGS` (both carry 8 bytes on the rmux wire).
fn decode_flags(data: &[u8]) -> Result<ClientFlags, Bad> {
    let bytes: [u8; 8] = data.try_into().map_err(|_| Bad)?;
    Ok(ClientFlags::from_bits_retain(u64::from_le_bytes(bytes)))
}

/// `MSG_IDENTIFY_CWD` (`server-client.c:2831-2841`): keep an executable
/// directory, else home, else `/`.
fn identify_cwd(data: &[u8]) -> Vec<u8> {
    if rmux_sys::access_executable(data) {
        data.to_vec()
    } else if let Some(home) = rmux_sys::proc::home_directory(None) {
        home
    } else {
        b"/".to_vec()
    }
}

/// Apply one identify payload to the client (`server-client.c:2787-2869`),
/// without the `DONE` processing.
fn apply_identify(c: &mut Client, message: &mut ProtocolMessage) -> Result<(), Bad> {
    let data = &message.data;
    match message.kind {
        Kind::IdentifyFeatures => {
            let feat = decode_i32(data)?;
            c.term_features.enabled |= feat as u32;
            log_debug!(
                "client {} IDENTIFY_FEATURES {}",
                String::from_utf8_lossy(c.name_bytes()),
                rmux_tty::features::feature_names(feat as u32)
            );
        }
        Kind::IdentifyFlags | Kind::IdentifyLongflags => {
            let flags = decode_flags(data)?;
            c.flags.insert(flags);
            log_debug!("client IDENTIFY_FLAGS {:#x}", flags.bits());
        }
        Kind::IdentifyTerm => {
            c.term_name = Some(decode_string(data)?);
        }
        Kind::IdentifyTerminfo => {
            c.term_caps.push(decode_string(data)?);
        }
        Kind::IdentifyTtyname => {
            c.ttyname = Some(decode_string(data)?);
        }
        Kind::IdentifyCwd => {
            let path = decode_string(data)?;
            c.cwd = Some(identify_cwd(&path));
        }
        Kind::IdentifyStdin => {
            if !data.is_empty() {
                return Err(Bad);
            }
            c.fd = message.fd.take();
        }
        Kind::IdentifyStdout => {
            if !data.is_empty() {
                return Err(Bad);
            }
            c.out_fd = message.fd.take();
        }
        Kind::IdentifyEnviron => {
            let entry = decode_string(data)?;
            if entry.contains(&b'=') {
                c.environ.put(&entry, Default::default());
            }
        }
        Kind::IdentifyClientpid => {
            c.pid = Some(rmux_sys::ProcessId(decode_i32(data)?));
        }
        _ => {}
    }
    Ok(())
}

/// `MSG_IDENTIFY_DONE` naming (`server-client.c:2875-2884`): `term_name`
/// falls back to `unknown`; the client name is the tty name or `client-<pid>`.
fn identify_name(c: &mut Client) {
    if c.term_name
        .as_ref()
        .is_none_or(|name| name.is_empty() || name[0] == 0)
    {
        c.term_name = Some(b"unknown".to_vec());
    }
    let name = match &c.ttyname {
        Some(ttyname) if !ttyname.is_empty() && ttyname[0] != 0 => ttyname.clone(),
        _ => format!("client-{}", c.pid.map_or(0, |pid| i64::from(pid.0))).into_bytes(),
    };
    c.name = Some(name);
}

/// `tty_init` (`tty.c:104-120`) plus the `tty_resize` and `CLIENT_TERMINAL`
/// step of `IDENTIFY_DONE` (`server-client.c:2894-2905`). Moves `c->fd` into
/// the tty; a non-tty or `tcgetattr` failure closes it.
fn init_tty(server: &mut Server, id: ClientId) {
    let Server { clients, tparm, .. } = server;
    let Some(c) = clients.get_mut(id) else {
        return;
    };
    if let Some(fd) = c.fd.take() {
        let tio = if rmux_sys::fd::isatty(fd.as_fd()) {
            TermiosState::get(fd.as_fd()).ok()
        } else {
            None
        };
        match tio {
            Some(tio) => {
                let host = TtyHostInfo {
                    name: ByteString::from(c.name_bytes()),
                    utf8: c.flags.intersects(ClientFlags::UTF8),
                    theme: c.theme,
                    theme_colours: c.theme_colours,
                    features: c.term_features,
                };
                let mut tty = Tty::new(fd, tio, host);
                tty.resize(tparm);
                c.tty = Some(tty);
                // tty_init zeroes the whole tty including the mouse state (tty.c:109-115).
                c.drag = MouseDragState::default();
                c.flags.insert(ClientFlags::TERMINAL);
            }
            None => drop(fd),
        }
    }
    c.out_fd = None;
}

/// `server_client_dispatch_identify` (`server-client.c:2772-2927`).
fn dispatch_identify(
    server: &mut Server,
    id: ClientId,
    mut message: ProtocolMessage,
) -> Result<(), Bad> {
    let Some(c) = server.clients.get_mut(id) else {
        return Ok(());
    };
    if c.flags.intersects(ClientFlags::IDENTIFIED) {
        return Err(Bad);
    }
    apply_identify(c, &mut message)?;
    if message.kind != Kind::IdentifyDone {
        return Ok(());
    }
    c.flags.insert(ClientFlags::IDENTIFIED);
    identify_name(c);
    if let Some(pid) = c.pid {
        log_debug!(
            "client {} IDENTIFY_CLIENTPID {}",
            String::from_utf8_lossy(c.name_bytes()),
            pid.0
        );
    }
    log_debug!(
        "client {id:?} name is {}",
        String::from_utf8_lossy(c.name_bytes())
    );

    if c.flags.intersects(ClientFlags::CONTROL) {
        if let Err(error) = crate::control::start(server, id) {
            log_debug!("client {id:?} control start failed: {error}");
        }
    } else if c.fd.is_some() {
        init_tty(server, id);
    }

    let Some(c) = server.clients.get(id) else {
        return Ok(());
    };
    if c.flags
        .intersects(ClientFlags::CONTROL | ClientFlags::TERMINAL)
    {
        events::fire_client(server, b"client-created", id);
    }

    // If pasting has taken too long, turn it off.
    let current_time = server.current_time.0;
    if let Some(c) = server.clients.get_mut(id)
        && c.flags
            .intersects(ClientFlags::BRACKETPASTING | ClientFlags::ASSUMEPASTING)
        && current_time - c.paste_time > PASTE_TIME_LIMIT
    {
        log_debug!(
            "{}: paste time limit exceeded",
            String::from_utf8_lossy(c.name_bytes())
        );
        c.flags
            .remove(ClientFlags::BRACKETPASTING | ClientFlags::ASSUMEPASTING);
    }

    // The first client loads the configuration; later clients continue with
    // their command even while it is still loading.
    let first = server.client_order.front() == Some(&id);
    let exiting = server
        .clients
        .get(id)
        .is_none_or(|c| c.flags.intersects(ClientFlags::EXIT));
    if !exiting && !server.cfg.finished && first {
        let mut cfg = std::mem::take(&mut server.cfg);
        cfg.start_cfg(server);
        let during = std::mem::replace(&mut server.cfg, cfg);
        server.cfg.causes.extend(during.causes);
    }
    Ok(())
}

/// `server_client_dispatch_shell` (`server-client.c:2930-2942`): answer with
/// the global `default-shell` (or `/bin/sh`) and kill the peer.
fn dispatch_shell(server: &mut Server, id: ClientId) {
    let mut shell = server
        .options
        .get_string(server.options.global_s, b"default-shell");
    if !rmux_util::shell::check_shell(shell, b"rmux") {
        shell = b"/bin/sh";
    }
    let data = encode_string(rmux_util::bytes::cstr(shell));
    let Some(peer) = server.clients.get(id).and_then(|c| c.peer) else {
        return;
    };
    let _ = proc_send(server, peer, ProtocolMessage::new(Kind::Shell, data));
    server.process.kill_peer(peer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::environment::EnvironmentFlags;

    fn message(kind: Kind, data: Vec<u8>) -> ProtocolMessage {
        ProtocolMessage::new(kind, data)
    }

    #[test]
    fn identify_flags_accumulate() {
        let mut c = Client::new(None, (0, 0));
        apply_identify(
            &mut c,
            &mut message(
                Kind::IdentifyLongflags,
                ClientFlags::UTF8.bits().to_le_bytes().to_vec(),
            ),
        )
        .unwrap();
        apply_identify(
            &mut c,
            &mut message(
                Kind::IdentifyFlags,
                ClientFlags::CONTROL.bits().to_le_bytes().to_vec(),
            ),
        )
        .unwrap();
        assert!(
            c.flags
                .contains(ClientFlags::FOCUSED | ClientFlags::UTF8 | ClientFlags::CONTROL)
        );
        assert!(apply_identify(&mut c, &mut message(Kind::IdentifyFlags, vec![1, 2, 3])).is_err());
    }

    #[test]
    fn identify_features_or_together() {
        let mut c = Client::new(None, (0, 0));
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyFeatures, 1i32.to_le_bytes().to_vec()),
        )
        .unwrap();
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyFeatures, 4i32.to_le_bytes().to_vec()),
        )
        .unwrap();
        assert_eq!(c.term_features.enabled, 5);
        assert!(apply_identify(&mut c, &mut message(Kind::IdentifyFeatures, vec![1])).is_err());
    }

    #[test]
    fn identify_cwd_falls_back_when_not_executable() {
        assert_eq!(identify_cwd(b"/"), b"/");
        let fallback = identify_cwd(b"/nonexistent/rmux-cwd");
        assert_eq!(
            fallback,
            rmux_sys::proc::home_directory(None).unwrap_or_else(|| b"/".to_vec())
        );
        let mut c = Client::new(None, (0, 0));
        apply_identify(&mut c, &mut message(Kind::IdentifyCwd, encode_string(b"/"))).unwrap();
        assert_eq!(c.cwd.as_deref(), Some(&b"/"[..]));
    }

    #[test]
    fn identify_environ_keeps_only_assignments() {
        let mut c = Client::new(None, (0, 0));
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyEnviron, encode_string(b"FOO=bar")),
        )
        .unwrap();
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyEnviron, encode_string(b"NOEQUALS")),
        )
        .unwrap();
        assert_eq!(
            c.environ
                .find(b"FOO")
                .and_then(|e| e.value.as_ref())
                .map(|v| v.as_bytes()),
            Some(&b"bar"[..])
        );
        assert!(c.environ.find(b"NOEQUALS").is_none());
        assert_eq!(
            c.environ.find(b"FOO").unwrap().flags,
            EnvironmentFlags::default()
        );
    }

    #[test]
    fn identify_strings_and_pid() {
        let mut c = Client::new(None, (0, 0));
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyTerm, encode_string(b"xterm")),
        )
        .unwrap();
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyTerminfo, encode_string(b"cup=x")),
        )
        .unwrap();
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyTerminfo, encode_string(b"clear=y")),
        )
        .unwrap();
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyTtyname, encode_string(b"/dev/ttys9")),
        )
        .unwrap();
        apply_identify(
            &mut c,
            &mut message(Kind::IdentifyClientpid, 4242i32.to_le_bytes().to_vec()),
        )
        .unwrap();
        assert_eq!(c.term_name.as_deref(), Some(&b"xterm"[..]));
        assert_eq!(c.term_caps, vec![b"cup=x".to_vec(), b"clear=y".to_vec()]);
        assert_eq!(c.ttyname.as_deref(), Some(&b"/dev/ttys9"[..]));
        assert_eq!(c.pid, Some(rmux_sys::ProcessId(4242)));
        assert!(apply_identify(&mut c, &mut message(Kind::IdentifyTerm, b"raw".to_vec())).is_err());
        assert!(apply_identify(&mut c, &mut message(Kind::IdentifyStdin, vec![0])).is_err());
    }

    #[test]
    fn done_names_client_from_tty_or_pid() {
        let mut c = Client::new(None, (0, 0));
        c.pid = Some(rmux_sys::ProcessId(77));
        identify_name(&mut c);
        assert_eq!(c.name.as_deref(), Some(&b"client-77"[..]));
        assert_eq!(c.term_name.as_deref(), Some(&b"unknown"[..]));

        let mut c = Client::new(None, (0, 0));
        c.ttyname = Some(b"".to_vec());
        c.term_name = Some(b"".to_vec());
        identify_name(&mut c);
        assert_eq!(c.name.as_deref(), Some(&b"client-0"[..]));
        assert_eq!(c.term_name.as_deref(), Some(&b"unknown"[..]));

        let mut c = Client::new(None, (0, 0));
        c.ttyname = Some(b"/dev/ttys3".to_vec());
        c.term_name = Some(b"screen".to_vec());
        identify_name(&mut c);
        assert_eq!(c.name.as_deref(), Some(&b"/dev/ttys3"[..]));
        assert_eq!(c.term_name.as_deref(), Some(&b"screen"[..]));
    }
}
