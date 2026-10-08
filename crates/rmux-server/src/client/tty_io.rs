// Ported from tmux tty.c, tty-keys.c @ 8f25579c
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
//! The client tty event glue: `tty->event_in`/`event_out` and tty
//! timers (`tty.c:tty_read_callback`, `tty_write_callback`,
//! `tty_timer_callback`, `tty_start_tty`), and the `tty_keys_next` loop that
//! turns decoded input into key events and terminal replies
//! (`tty-keys.c:745-1081`).

use crate::client::{ClientFlags, KeyEvent};
use crate::ids::{ClientId, TimerId};
use crate::model::session::session_theme_changed;
use crate::server::Server;
use crate::server::event_loop::LoopAction;
use rmux_emu::input::effect::{InputReply, InputRequestClipboardData};
use rmux_tty::features::{TtyFeatures, default_features, parse_features};
use rmux_tty::keys::{DecodeStep, Discovery, KeyDecodeContext, TtyInput};
use rmux_tty::tty::{TimerRequest, TtyEffect, TtyFlags, TtyTimer};
use rmux_util::key::{KeyCode, SpecialKey};
use rmux_util::log_debug;

/// One `TimerId` slot per `TtyTimer` (`tty.c` has one `struct event` each).
#[derive(Clone, Copy, Debug, Default)]
pub struct TtyTimers {
    start: Option<TimerId>,
    clipboard: Option<TimerId>,
    block: Option<TimerId>,
    key: Option<TimerId>,
    protocol: Option<TimerId>,
    stop: Option<TimerId>,
}
impl TtyTimers {
    fn slot(&mut self, timer: TtyTimer) -> &mut Option<TimerId> {
        match timer {
            TtyTimer::Start => &mut self.start,
            TtyTimer::Clipboard => &mut self.clipboard,
            TtyTimer::Block => &mut self.block,
            TtyTimer::Key => &mut self.key,
            TtyTimer::Protocol => &mut self.protocol,
            TtyTimer::Stop => &mut self.stop,
        }
    }
    /// Cancel every timer (`tty_stop_tty`/`tty_free` `evtimer_del` calls).
    pub fn cancel_all(&mut self, event_loop: &mut crate::server::event_loop::EventLoop) {
        for timer in [
            TtyTimer::Start,
            TtyTimer::Clipboard,
            TtyTimer::Block,
            TtyTimer::Key,
            TtyTimer::Protocol,
            TtyTimer::Stop,
        ] {
            if let Some(id) = self.slot(timer).take() {
                event_loop.cancel(id);
            }
        }
    }
}

fn apply_timer(server: &mut Server, id: ClientId, request: TimerRequest) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let slot = c.tty_timers.slot(request.timer);
    if let Some(old) = slot.take() {
        server.event_loop.cancel(old);
    }
    if let Some(after) = request.after {
        let timer = server
            .event_loop
            .schedule(after, LoopAction::ClientTtyTimer(id, request.timer));
        if let Some(c) = server.clients.get_mut(id) {
            *c.tty_timers.slot(request.timer) = Some(timer);
        }
    }
}

/// Drain the tty's queued effects and timer requests, then (re)register the
/// fd for the readiness the tty wants. Call after every tty call that may
/// queue output or change the started state (`tty_start_tty`/`tty_stop_tty`
/// `event_add`/`event_del`, `tty_write` `event_add(&tty->event_out)`).
pub fn sync(server: &mut Server, id: ClientId) {
    crate::tsp::broker::client_sync(server, id);
    crate::client::dispatch::finish_exiting(server, id);
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let Some(tty) = c.tty.as_mut() else {
        if let Some(token) = c.tty_token.take() {
            server.event_loop.deregister(token);
        }
        c.tty_timers.cancel_all(&mut server.event_loop);
        return;
    };
    c.term_features = tty.host_mut().features;
    if tty.host_mut().utf8 {
        c.flags.insert(ClientFlags::UTF8);
    }
    let effects: Vec<TtyEffect> = tty.drain_effects().collect();
    let timers: Vec<TimerRequest> = tty.pending_timers().collect();
    let mut lost = false;
    let mut protocol_invalidated = false;
    for effect in effects {
        match effect {
            TtyEffect::ReadClosed => lost = true,
            TtyEffect::RedrawClient => c.flags.insert(ClientFlags::ALLREDRAWFLAGS),
            TtyEffect::AllRedrawFlags => c.flags.insert(ClientFlags::ALLREDRAWFLAGS),
            TtyEffect::Discarded(n) => c.discarded = c.discarded.wrapping_add(n),
            TtyEffect::Written(n) => c.written = c.written.wrapping_add(n),
            TtyEffect::ProtocolInvalidated { generation: _ } => protocol_invalidated = true,
        }
    }
    if protocol_invalidated {
        crate::tsp::broker::client_sync(server, id);
    }
    for request in timers {
        apply_timer(server, id, request);
    }
    if lost {
        // tty_read_callback: event_del then server_client_lost.
        crate::client::lifecycle::lost(server, id);
        return;
    }
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let Some(tty) = c.tty.as_ref() else {
        return;
    };
    let started = tty.flags().contains(TtyFlags::STARTED);
    let read = tty.wants_read();
    let write = tty.wants_write();
    let active = started || read || write;
    match c.tty_token {
        Some(token) if active => {
            if let Err(e) = server.event_loop.reregister(token, read, write) {
                log_debug!("tty reregister failed: {e}");
            }
        }
        Some(token) => {
            c.tty_token = None;
            server.event_loop.deregister(token);
        }
        None if active => {
            match server
                .event_loop
                .register(tty.fd(), read, write, LoopAction::ClientTty(id))
            {
                Ok(token) => c.tty_token = Some(token),
                Err(e) => log_debug!("tty register failed: {e}"),
            }
        }
        None => {}
    }
}

/// `tty_read_callback` and `tty_write_callback` (`tty.c`).
pub fn on_ready(server: &mut Server, id: ClientId, readable: bool, writable: bool) {
    sync(server, id);
    if writable {
        let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) else {
            return;
        };
        let size = tty.out_len();
        match tty.on_writable() {
            Ok(n) => {
                let redraw_left = {
                    let c = server.clients.get_mut(id).expect("client checked above");
                    if c.redraw > 0 {
                        c.redraw = c.redraw.saturating_sub(n);
                    }
                    c.redraw
                };
                log_debug!("tty wrote {n} bytes (of {size}), redraw {redraw_left} left");
            }
            Err(e) => log_debug!("tty write error: {e}"),
        }
    }
    if readable {
        crate::tsp::broker::client_read_bound(server, id);
        let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) else {
            return;
        };
        match tty.on_readable() {
            rmux_tty::tty::ReadOutcome::Closed => {
                sync(server, id); // ReadClosed effect -> lost
                return;
            }
            rmux_tty::tty::ReadOutcome::Bytes(n) => log_debug!("tty read {n} bytes"),
        }
        while decode_step(server, id) {}
    }
    sync(server, id);
}

/// One of the tty timers fired (`tty.c:tty_timer_callback`,
/// `tty_start_timer_callback`, `tty_clipboard_query_callback`;
/// `tty-keys.c:tty_keys_callback`).
pub fn on_timer(server: &mut Server, id: ClientId, timer: TtyTimer) {
    crate::tsp::broker::client_sync(server, id);
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    *c.tty_timers.slot(timer) = None;
    let Server { clients, tparm, .. } = server;
    let Some(tty) = clients.get_mut(id).and_then(|c| c.tty.as_mut()) else {
        return;
    };
    tty.on_timer(tparm, timer);
    if matches!(timer, TtyTimer::Key | TtyTimer::Protocol) {
        while decode_step(server, id) {}
    }
    sync(server, id);
}

fn decode_context(server: &Server, id: ClientId) -> Option<KeyDecodeContext> {
    let c = server.clients.get(id)?;
    let tty = c.tty.as_ref()?;
    let (sx, sy) = tty.size();
    let (xpixel, ypixel) = tty.pixel_size();
    Some(KeyDecodeContext {
        flags: tty.flags(),
        has_session: c.session.is_some(),
        escape_time_ms: server
            .options
            .get_number(server.options.global, b"escape-time")
            .clamp(0, i64::from(u32::MAX)) as u32,
        verase: tty.verase(),
        sx,
        sy,
        xpixel,
        ypixel,
        has_input_requests: !c.input_requests.is_empty(),
        tsp_input: tty.stopping()
            || matches!(
                c.tsp.capability,
                crate::tsp::client::Capability::Probing | crate::tsp::client::Capability::V1(_)
            ),
    })
}

/// Owned copy of one complete decode result so the input buffer can be
/// drained before the (reentrant) server calls.
enum Owned {
    Key {
        key: KeyCode,
        mouse: Option<rmux_util::key::MouseEvent>,
        raw: Vec<u8>,
    },
    Clipboard(rmux_tty::keys::ClipboardReply),
    Palette(rmux_tty::keys::PaletteReply),
    Colour(rmux_tty::keys::ColourReply),
    Discovery(Discovery),
    Size(rmux_tty::keys::SizeReply),
    Tsp {
        verb: u8,
        body: Vec<u8>,
    },
    Da1Sentinel(Vec<u8>),
    ProtocolFault(rmux_tty::keys::ProtocolFault),
}

pub(crate) fn drain_input(server: &mut Server, id: ClientId) {
    crate::tsp::broker::client_read_bound(server, id);
    while decode_step(server, id) {}
    sync(server, id);
}

/// One `tty_keys_next` round (`tty-keys.c:745-1065`): returns true when a
/// key was consumed and another round should run.
fn decode_step(server: &mut Server, id: ClientId) -> bool {
    if !crate::tsp::input::client_can_decode(server, id) {
        return false;
    }
    let Some(ctx) = decode_context(server, id) else {
        return false;
    };
    let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) else {
        return false;
    };
    let (consumed, owned, cancel_timer, theme_changed, timer) = match tty.decode_next(&ctx) {
        DecodeStep::Empty => return false,
        DecodeStep::Partial {
            timer,
            theme_changed,
        } => (0, None, false, theme_changed, timer),
        DecodeStep::Discard {
            consumed,
            cancel_timer,
        } => (consumed, None, cancel_timer, false, None),
        DecodeStep::Complete {
            consumed,
            input,
            cancel_timer,
            theme_changed,
        } => {
            let owned = match input {
                TtyInput::Key(k) => Owned::Key {
                    key: k.key,
                    mouse: k.mouse,
                    raw: k.raw.to_vec(),
                },
                TtyInput::Clipboard(r) => Owned::Clipboard(r),
                TtyInput::Palette(r) => Owned::Palette(r),
                TtyInput::Colour(r) => Owned::Colour(r),
                TtyInput::Discovery(d) => Owned::Discovery(d),
                TtyInput::Size(r) => Owned::Size(r),
                TtyInput::Tsp { verb, body } => Owned::Tsp {
                    verb,
                    body: body.to_vec(),
                },
                TtyInput::Da1Sentinel { raw } => Owned::Da1Sentinel(raw.to_vec()),
                TtyInput::ProtocolFault(fault) => Owned::ProtocolFault(fault),
            };
            (consumed, Some(owned), cancel_timer, theme_changed, None)
        }
    };
    let reservation = match owned.as_ref() {
        Some(Owned::Key { key, mouse, raw }) if key.0 != SpecialKey::UNKNOWN => {
            match crate::client::keys::reserve_key_input(
                server,
                id,
                *key,
                mouse.as_ref(),
                raw.len(),
                consumed,
            ) {
                Ok(reservation) => reservation,
                Err(()) => return false,
            }
        }
        _ => None,
    };
    if consumed != 0 {
        if let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) {
            tty.consume_input(consumed);
        }
    }
    if cancel_timer {
        apply_timer(server, id, rmux_tty::keys::cancel_key_timer());
    }
    if let Some(request) = timer {
        apply_timer(server, id, request);
    }
    if theme_changed {
        // tty-keys.c:830-843: a changed background updates the colours.
        crate::client::theme::update_theme_colours(server, id);
        if let Some(session) = server.clients.get(id).and_then(|c| c.session) {
            session_theme_changed(server, Some(session));
        }
    }
    let Some(owned) = owned else {
        crate::tsp::broker::client_read_bound(server, id);
        return consumed != 0;
    };
    apply_input(server, id, owned, reservation);
    crate::tsp::broker::client_read_bound(server, id);
    true
}

fn current_window(server: &Server, id: ClientId) -> Option<crate::ids::WindowId> {
    let session = server.clients.get(id)?.session?;
    let current = server.sessions.get(session)?.current?;
    Some(server.winlinks.get(current)?.window)
}

fn apply_input(
    server: &mut Server,
    id: ClientId,
    input: Owned,
    reservation: Option<crate::client::keys::InputReservation>,
) {
    match input {
        Owned::Tsp { verb, body } => crate::tsp::broker::client_message(server, id, verb, &body),
        Owned::Da1Sentinel(raw) => crate::tsp::broker::client_sentinel(server, id, &raw),
        Owned::ProtocolFault(fault) => {
            log_debug!("tty TSP protocol fault: {fault:?}");
            crate::tsp::broker::client_protocol_fault(server, id);
        }
        Owned::Key { key, mouse, raw } => {
            // tty-keys.c:1031-1039: focus events.
            if key == KeyCode(SpecialKey::FOCUS_OUT) {
                if let Some(c) = server.clients.get_mut(id) {
                    c.flags.remove(ClientFlags::FOCUSED);
                }
                if let Some(w) = current_window(server, id) {
                    crate::client::lifecycle::update_window_focus(server, w);
                }
                crate::server::events::fire_client(server, b"client-focus-out", id);
            } else if key == KeyCode(SpecialKey::FOCUS_IN) {
                if let Some(c) = server.clients.get_mut(id) {
                    c.flags.insert(ClientFlags::FOCUSED);
                }
                crate::server::events::fire_client(server, b"client-focus-in", id);
                if let Some(w) = current_window(server, id) {
                    crate::client::lifecycle::update_window_focus(server, w);
                }
            }
            if key != KeyCode(SpecialKey::UNKNOWN) {
                let mut event = KeyEvent::new(key);
                if let Some(m) = mouse {
                    event.mouse = m;
                }
                event.paste = Some(raw);
                crate::client::keys::handle_reserved_key(server, id, event, reservation);
            }
        }
        Owned::Clipboard(reply) => {
            let Some(data) = reply.data else {
                return;
            };
            // tty-keys.c:1445-1457: a pending pane request first, then an
            // OSC 52 query becomes a paste buffer.
            if server
                .clients
                .get(id)
                .is_some_and(|c| !c.input_requests.is_empty())
            {
                let input = InputReply::Clipboard(InputRequestClipboardData {
                    buf: data.clone(),
                    clip: reply.clip,
                });
                crate::client::input_requests::reply(server, id, &input);
            }
            if reply.query {
                let limit = server
                    .options
                    .get_number(server.options.global, b"buffer-limit")
                    .clamp(0, i64::from(u32::MAX)) as u32;
                let _ = crate::model::paste::paste_add(server, None, data.into_vec(), limit);
                apply_timer(
                    server,
                    id,
                    TimerRequest {
                        timer: TtyTimer::Clipboard,
                        after: None,
                    },
                );
                if let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) {
                    tty.flags_mut().remove(TtyFlags::OSC52QUERY);
                }
            }
        }
        Owned::Palette(reply) => {
            if let Some(data) = reply.reply {
                crate::client::input_requests::reply(server, id, &InputReply::Palette(data));
            }
        }
        Owned::Colour(reply) => {
            let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) else {
                return;
            };
            let old_bg = tty.reported_colours().1;
            if let Some(colour) = reply.colour {
                let foreground = reply.target == rmux_tty::keys::ColourTarget::Foreground;
                tty.set_reported_colour(foreground, colour.0);
            }
            tty.flags_mut().remove(reply.target.wait_flag());
            let bg_changed = tty.reported_colours().1 != old_bg;
            // tty-keys.c:830-836
            if bg_changed {
                crate::client::theme::update_theme_colours(server, id);
            }
            if let Some(session) = server.clients.get(id).and_then(|c| c.session) {
                session_theme_changed(server, Some(session));
            }
        }
        Owned::Discovery(discovery) => apply_discovery(server, id, discovery),
        Owned::Size(reply) => {
            let Server { clients, tparm, .. } = server;
            let Some(tty) = clients.get_mut(id).and_then(|c| c.tty.as_mut()) else {
                return;
            };
            tty.set_size(reply.sx, reply.sy, reply.xpixel, reply.ypixel);
            if reply.invalidate {
                tty.invalidate(tparm);
            }
            if reply.clear_query {
                tty.flags_mut().remove(TtyFlags::WINSIZEQUERY);
            }
            crate::server::run::recalculate_sizes(server);
            crate::server::operations::server_redraw_client(server, id);
        }
    }
}

/// Device-attribute replies (`tty-keys.c:1540-1744`): update the client's
/// features, then the tty, then set the `have` flag.
pub(crate) fn apply_discovery(server: &mut Server, id: ClientId, discovery: Discovery) {
    let mut features: TtyFeatures = match server.clients.get(id) {
        Some(c) => c.term_features,
        None => return,
    };
    let (name, version) = {
        let c = server.clients.get(id).expect("client checked above");
        let tty = c.tty.as_ref();
        (
            tty.map(|t| String::from_utf8_lossy(t.term().name()).into_owned())
                .unwrap_or_default(),
            0u32,
        )
    };
    match &discovery {
        Discovery::PrimaryDa { features: da } => {
            for feature in da.names() {
                parse_features(feature, ",", &mut features);
            }
        }
        Discovery::SecondaryDa { defaults } | Discovery::ExtendedDa { defaults, .. } => {
            if let Some(defaults) = defaults {
                default_features(defaults, version, &mut features);
            } else {
                default_features(&name, version, &mut features);
            }
        }
        Discovery::Sync { sync } => {
            if *sync {
                parse_features("sync", ",", &mut features);
            }
        }
    }
    if let Discovery::ExtendedDa {
        term_type: Some(term_type),
        ..
    } = &discovery
    {
        if let Some(c) = server.clients.get_mut(id) {
            c.term_type = Some(term_type.to_vec());
        }
    }
    let opts = crate::client::lifecycle::tty_options(server);
    let Server { clients, tparm, .. } = server;
    let Some(c) = clients.get_mut(id) else {
        return;
    };
    c.term_features = features;
    if let Some(tty) = c.tty.as_mut() {
        tty.host_mut().features = features;
        if discovery.updates_features() {
            tty.update_features(tparm, &opts);
        }
        tty.flags_mut().insert(discovery.have_flag());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Client;
    use rmux_tty::tty::{Tty, TtyHostInfo};
    use std::os::fd::AsFd;
    #[test]
    fn protocol_timer_has_an_independent_owned_slot() {
        use crate::ids::ArenaId;
        let mut timers = TtyTimers::default();
        let key = TimerId::from_parts(1, 0);
        let protocol = TimerId::from_parts(2, 0);
        *timers.slot(TtyTimer::Key) = Some(key);
        *timers.slot(TtyTimer::Protocol) = Some(protocol);
        assert_eq!(timers.slot(TtyTimer::Key).take(), Some(key));
        assert_eq!(timers.slot(TtyTimer::Protocol).take(), Some(protocol));
    }

    #[test]
    fn applied_terminal_features_update_client_formats_and_utf8() {
        let mut server = Server::new();
        let (_master, slave, _) = rmux_sys::pty::openpty().unwrap();
        let tio = rmux_sys::TermiosState::get(slave.as_fd()).unwrap();
        let mut host = TtyHostInfo::default();
        parse_features("utf8,RGB", ",", &mut host.features);
        host.utf8 = true;
        let features = host.features;
        let mut client = Client::new(None, (0, 0));
        client.tty = Some(Tty::new(slave, tio, host));
        let id = server.clients.insert(client).unwrap();
        sync(&mut server, id);
        let client = server.clients.get_mut(id).unwrap();
        assert!(client.flags.contains(ClientFlags::UTF8));
        assert_eq!(client.term_features, features);
        client.tty.as_mut().unwrap().host_mut().utf8 = false;
        sync(&mut server, id);
        assert!(
            server
                .clients
                .get(id)
                .unwrap()
                .flags
                .contains(ClientFlags::UTF8)
        );
    }
}
