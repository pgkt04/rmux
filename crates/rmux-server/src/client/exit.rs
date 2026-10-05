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

//! Exit handshake: `server_client_start_exit_timer`, `_exit_timer` and
//! `_check_exit` (`server-client.c:2219-2305`).

use crate::client::{ClientExitType, ClientFlags};
use crate::ids::*;
use crate::server::Server;
use crate::server::event_loop::LoopAction;
use crate::server::proc::proc_send;
use crate::server::protocol::{ProtocolMessage, ProtocolMessageKind, encode_i32s, encode_string};
use rmux_util::log_debug;
use std::time::Duration;

/// Exit timer period (`struct timeval tv = { .tv_sec = 10 }`,
/// `server-client.c:2222`).
pub const EXIT_TIMER: Duration = Duration::from_secs(10);

/// `server_client_start_exit_timer` (`server-client.c:2219-2226`): arm the
/// timer only when it is not already pending.
pub fn start_exit_timer(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.exit_timer.is_some() {
        return;
    }
    let timer = server
        .event_loop
        .schedule(EXIT_TIMER, LoopAction::ClientExitTimer(id));
    if let Some(c) = server.clients.get_mut(id) {
        c.exit_timer = Some(timer);
    }
}

/// `server_client_exit_timer` (`server-client.c:2229-2244`): the exit timer
/// has expired, stop waiting for the client.
pub fn exit_timer(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    // The timer has fired, so it is no longer pending.
    c.exit_timer = None;
    if c.flags
        .intersects(ClientFlags::DEAD | ClientFlags::SUSPENDED)
    {
        return;
    }

    if c.flags.intersects(ClientFlags::EXITED) {
        log_debug!(
            "exit_timer: {} took too long to exit",
            String::from_utf8_lossy(c.name_bytes())
        );
        crate::client::lifecycle::lost(server, id);
    } else if c.flags.intersects(ClientFlags::EXIT) {
        log_debug!(
            "exit_timer: {} took too long to flush",
            String::from_utf8_lossy(c.name_bytes())
        );
        check_exit(server, id, true);
    }
}

/// `server_client_check_exit` (`server-client.c:2247-2305`): check if the
/// client should be exited, abandoning buffered output if forced.
pub fn check_exit(server: &mut Server, id: ClientId, force: bool) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.flags.intersects(ClientFlags::DEAD | ClientFlags::EXITED) {
        return;
    }
    if !c.flags.intersects(ClientFlags::EXIT) {
        return;
    }

    if c.flags.intersects(ClientFlags::CONTROL) {
        if force {
            crate::control::discard_all(server, id);
        } else {
            crate::control::discard(server, id);
            if !crate::control::all_done(server, id) {
                start_exit_timer(server, id);
                return;
            }
        }
    }
    if !force {
        // RB_FOREACH(cf, client_files, &c->files): any buffered bytes
        // (file.c, server-client.c:2272-2277).
        if server.files.client_has_buffered(id) {
            start_exit_timer(server, id);
            return;
        }
    }
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    c.flags.insert(ClientFlags::EXITED);

    // evtimer_del then restart (server-client.c:2281-2282).
    if let Some(timer) = c.exit_timer.take() {
        server.event_loop.cancel(timer);
    }
    start_exit_timer(server, id);

    let Some(c) = server.clients.get(id) else {
        return;
    };
    let Some(peer) = c.peer else {
        return;
    };
    let message = match c.exit_type {
        ClientExitType::Return => {
            // retval then the optional message (server-client.c:2285-2296).
            let mut data = encode_i32s(&[c.retval]);
            if let Some(msg) = c.exit_message.as_deref() {
                data.extend_from_slice(&encode_string(msg));
            }
            ProtocolMessage::new(ProtocolMessageKind::Exit, data)
        }
        ClientExitType::Shutdown => ProtocolMessage::new(ProtocolMessageKind::Shutdown, Vec::new()),
        ClientExitType::Detach => {
            let name = c.exit_session.as_deref().unwrap_or(b"");
            ProtocolMessage::new(c.exit_msgtype, encode_string(name))
        }
    };
    // A closed peer is handled by the transport's lost callback.
    let _ = proc_send(server, peer, message);
}
