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

//! `c->input_requests` (`tmux.h:2366`, `server-client.c:201,414`): the FIFO
//! of pane input requests this client is answering. G05 pushes, G15 cancels
//! on loss.

use std::collections::VecDeque;

use crate::ids::{ClientId, RequestId};
use crate::model::pane_input::RequestKind;
use crate::server::Server;
use rmux_emu::input::InputReply;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputRequests(pub VecDeque<RequestId>);

impl InputRequests {
    pub fn new() -> Self {
        Self::default()
    }
    /// `TAILQ_INSERT_TAIL(&c->input_requests, ...)`.
    pub fn push(&mut self, request: RequestId) {
        self.0.push_back(request);
    }
    /// `TAILQ_REMOVE(&c->input_requests, ...)` for one answered or cancelled
    /// request; false when it was not queued.
    pub fn remove(&mut self, request: RequestId) -> bool {
        match self.0.iter().position(|r| *r == request) {
            Some(index) => {
                self.0.remove(index);
                true
            }
            None => false,
        }
    }
    /// `TAILQ_FIRST(&c->input_requests)`.
    pub fn front(&self) -> Option<RequestId> {
        self.0.front().copied()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = RequestId> + '_ {
        self.0.iter().copied()
    }
}

/// `input_cancel_requests(c)` as called from `server_client_lost`
/// (`server-client.c:414`): drop the client FIFO, then let G05 cancel every
/// pane request that still names this client.
pub fn cancel_all(server: &mut Server, id: ClientId) {
    if let Some(c) = server.clients.get_mut(id) {
        c.input_requests.0.clear();
    }
    crate::model::pane_input::input_cancel_requests(server, id);
}

/// The client half of `input_add_request` (`input.c:3607-3620`): queue the
/// request on the client FIFO and write the terminal query to its tty. A
/// palette request is `OSC 4;idx;?`; a clipboard request is the `Ms` query;
/// a queued request writes nothing.
pub fn send(server: &mut Server, client: ClientId, request: RequestId, kind: &RequestKind) {
    let Server { clients, tparm, .. } = server;
    let Some(c) = clients.get_mut(client) else {
        return;
    };
    c.input_requests.push(request);
    let Some(tty) = c.tty.as_mut() else {
        return;
    };
    match kind {
        RequestKind::Palette { idx } => {
            let query = format!("\x1b]4;{idx};?\x1b\\");
            tty.puts(query.as_bytes());
        }
        RequestKind::Clipboard { .. } => tty.clipboard_query(tparm),
        RequestKind::Queue(_) => {}
    }
}

/// Deliver a terminal reply for the oldest outstanding request of this
/// client (`input_request_reply`, `input.c:3660`): pop the FIFO head and let
/// G05 match it against the pane request list.
pub fn reply(server: &mut Server, client: ClientId, reply: &InputReply) {
    if let Some(c) = server.clients.get_mut(client) {
        c.input_requests.0.pop_front();
    }
    let _ = crate::model::pane_input::input_request_reply(server, client, reply);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;

    fn request(n: u32) -> RequestId {
        RequestId::from_parts(n, 0)
    }

    #[test]
    fn fifo_order_and_removal() {
        let mut requests = InputRequests::new();
        requests.push(request(1));
        requests.push(request(2));
        requests.push(request(3));
        assert_eq!(requests.front(), Some(request(1)));
        assert!(requests.remove(request(2)));
        assert!(!requests.remove(request(2)));
        assert_eq!(
            requests.iter().collect::<Vec<_>>(),
            vec![request(1), request(3)]
        );
        assert_eq!(requests.len(), 2);
        assert!(!requests.is_empty());
    }
}
