// Ported from tmux window.c, server-client.c, input.c, screen-write.c @ 8f25579c
// Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
// Permission to use, copy, modify, and distribute this software for any purpose
// with or without fee is hereby granted, provided that the above copyright
// notice and this permission notice appear in all copies.
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

use super::event_loop::LoopAction;
use crate::{
    client::{self, ClientFlags},
    cmd::commands::pipe_pane,
    ids::{ClientId, PaneId, RequestId},
    model::{
        self, ModelError, PaneFlags, Server,
        pane_input::{self, OwnedInputEffect, PaneInputHost, PaneStdinInput, StdinReadState},
    },
    ui::fanout::{self, PaneDrawSnapshot, PaneSink},
};
use rmux_emu::{
    cell::GridCell,
    colour::{ClientTheme, Colour},
    input::{ColourQueryKind, InputRequestKind},
    screen::write::{ScreenWritePolicy, TtySink},
};
use std::{io, os::fd::AsFd};

/// Client ownership is moved only while the emulator holds the pane screen.
/// Every synchronous model callback runs after end_draw restores the root.
#[derive(Default)]
pub struct PaneHost {
    sink: Option<PaneSink>,
    colours: GridCell,
    error: Option<io::Error>,
}
impl PaneHost {
    fn record(&mut self, result: io::Result<()>) {
        if let Err(error) = result {
            if self.error.is_none() {
                self.error = Some(error);
            }
        }
    }
    fn finish(self) -> Result<(), ModelError> {
        assert!(self.sink.is_none(), "unfinished pane draw lease");
        self.error.map_or(Ok(()), |error| {
            Err(ModelError::message(error.to_string().as_bytes()))
        })
    }
}
impl PaneInputHost for PaneHost {
    fn begin_draw(&mut self, server: &mut Server, pane: PaneId) {
        assert!(self.sink.is_none(), "nested pane draw lease");
        let time = if server.current_time.0 == 0 {
            0
        } else {
            server.current_time.0.wrapping_sub(server.start_time.0).wrapping_add(1) as u32
        };
        server.panes.get_mut(pane).expect("live pane line clock")
            .base.grid.set_line_clock(rmux_emu::grid::LineTime(time));
        let snapshot = PaneDrawSnapshot::capture(server, pane).expect("live pane draw snapshot");
        self.colours = snapshot.defaults;
        self.sink = Some(PaneSink::new(
            snapshot,
            std::mem::take(&mut server.clients),
            std::mem::take(&mut server.tparm),
        ));
    }
    fn end_draw(&mut self, server: &mut Server, pane: PaneId) {
        let (clients, tparm, effects) = self.sink.take().expect("pane draw lease").into_parts();
        server.clients = clients;
        server.tparm = tparm;
        fanout::apply_effects(server, pane, effects);
    }
    fn tty_sink(&mut self) -> &mut dyn TtySink {
        self.sink.as_mut().expect("pane sink outside parse step")
    }
    fn write_policy(&self, server: &Server, pane: PaneId) -> ScreenWritePolicy {
        let p = server.panes.get(pane).expect("live pane write policy");
        ScreenWritePolicy {
            pane_backed: p.modes.is_empty(),
            alternate_screen: server.options.get_number(p.options, b"alternate-screen") != 0,
            scroll_on_clear: server.options.get_number(p.options, b"scroll-on-clear") != 0,
            variation_selector_always_wide: server
                .options
                .get_number(server.options.global, b"variation-selector-always-wide")
                != 0,
            extended_keys: server
                .options
                .get_number(server.options.global, b"extended-keys")
                != 0,
        }
    }
    fn now_ms(&self) -> u64 {
        let now = rmux_util::time::Timestamp::now();
        (now.sec as u64)
            .wrapping_mul(1000)
            .wrapping_add(now.usec as u64 / 1000)
    }
    fn request_client(&self, server: &Server, pane: PaneId) -> Option<ClientId> {
        pane_input::select_request_client(client::registry::input_clients(server, pane))
    }
    fn send_request(
        &mut self,
        server: &mut Server,
        _pane: PaneId,
        request: RequestId,
        client: ClientId,
        kind: InputRequestKind,
    ) {
        let kind = match kind {
            InputRequestKind::Palette { idx } => pane_input::RequestKind::Palette { idx },
            InputRequestKind::Clipboard { clip } => pane_input::RequestKind::Clipboard { clip },
        };
        client::input_requests::send(server, client, request, &kind);
    }
    fn colour(&self, _server: &Server, _pane: PaneId, which: ColourQueryKind) -> Colour {
        match which {
            ColourQueryKind::Foreground => self.colours.fg,
            ColourQueryKind::Background => self.colours.bg,
        }
    }
    fn theme(&self, server: &Server, pane: PaneId) -> ClientTheme {
        client::theme::pane_theme(server, pane)
    }
    fn effect(&mut self, server: &mut Server, pane: PaneId, effect: &OwnedInputEffect) {
        super::effects::input_effect(server, pane, effect);
    }
    fn pipe_output(&mut self, server: &mut Server, pane: PaneId) {
        pipe_pane::output(server, pane);
    }
    fn control_output(&mut self, server: &mut Server, pane: PaneId) {
        let clients: Vec<_> = server
            .client_order
            .iter()
            .copied()
            .filter(|id| {
                server
                    .clients
                    .get(*id)
                    .is_some_and(|c| c.session.is_some() && c.flags.contains(ClientFlags::CONTROL))
            })
            .collect();
        for client in clients {
            crate::control::write_output(server, client, pane);
        }
    }
    fn disable_reads(&mut self, server: &mut Server, pane: PaneId) {
        server.pane_read_disabled.insert(pane);
        if let Some(token) = server.pane_tokens.get(&pane).copied() {
            let write = !server.pane_io_failed.contains(&pane)
                && server.panes.get(pane).is_some_and(|p| !p.output.is_empty());
            self.record(server.event_loop.reregister(token, false, write));
        }
    }
    fn stop_sync(&mut self, server: &mut Server, pane: PaneId) {
        stop_sync(server, pane);
    }
}

pub fn stop_sync(server: &mut Server, pane: PaneId) {
    fanout::screen_write_stop_sync(server, pane);
}
pub fn pane_read(server: &mut Server, pane: PaneId, bytes: &[u8]) -> Result<(), ModelError> {
    let mut host = PaneHost::default();
    pane_input::pane_read(server, &mut host, pane, bytes)?;
    host.finish()
}
pub fn pane_parse_buffer(
    server: &mut Server,
    pane: PaneId,
    bytes: &[u8],
) -> Result<usize, ModelError> {
    let mut host = PaneHost::default();
    let consumed = pane_input::pane_parse_buffer(server, &mut host, pane, bytes)?;
    host.finish()?;
    Ok(consumed)
}
pub fn apply_input_effect(
    server: &mut Server,
    pane: PaneId,
    effect: OwnedInputEffect,
) -> Result<(), ModelError> {
    let mut host = PaneHost::default();
    if server.panes.get(pane).is_some() {
        host.colours = fanout::tty_default_colours(server, pane).0;
    }
    pane_input::apply_effect(server, &mut host, pane, effect)?;
    host.finish()
}
pub fn input_sync_timer(server: &mut Server, pane: PaneId) {
    pane_input::input_sync_timer(server, &mut PaneHost::default(), pane);
}
pub fn stdin_input_chunk(
    server: &mut Server,
    input: &mut PaneStdinInput,
    bytes: &mut Vec<u8>,
    state: StdinReadState,
) -> Result<(), ModelError> {
    let mut host = PaneHost::default();
    pane_input::pane_stdin_input(server, &mut host, input, bytes, state)?;
    host.finish()
}

pub fn sync_panes(server: &mut Server) -> io::Result<()> {
    let stale: Vec<_> = server
        .pane_tokens
        .keys()
        .copied()
        .filter(|id| server.panes.get(*id).is_none_or(|p| p.fd.is_none()))
        .collect();
    for pane in stale {
        if let Some(token) = server.pane_tokens.remove(&pane) {
            server.event_loop.deregister(token);
        }
        server.pane_read_disabled.remove(&pane);
        server.pane_io_failed.remove(&pane);
    }
    for pane in server.pane_ids.values().copied() {
        let Some(p) = server.panes.get(pane) else {
            continue;
        };
        let Some(fd) = p.fd.as_ref() else {
            continue;
        };
        let failed = server.pane_io_failed.contains(&pane);
        let read = !failed && !server.pane_read_disabled.contains(&pane);
        let write = !failed && !p.output.is_empty();
        if let Some(token) = server.pane_tokens.get(&pane).copied() {
            server.event_loop.reregister(token, read, write)?;
        } else {
            rmux_sys::fd::set_blocking(fd.as_fd(), false);
            let token =
                server
                    .event_loop
                    .register(fd.as_fd(), read, write, LoopAction::Pane(pane))?;
            server.pane_tokens.insert(pane, token);
        }
    }
    Ok(())
}

pub fn close_pane_io(server: &mut Server, pane: PaneId) {
    if let Some(token) = server.pane_tokens.remove(&pane) {
        server.event_loop.deregister(token);
    }
    server.pane_read_disabled.remove(&pane);
    server.pane_io_failed.remove(&pane);
    if let Some(p) = server.panes.get_mut(pane) {
        p.fd.take();
    }
    pipe_pane::close(server, pane);
}

/// Drain only bytes consumed by every stream reader; control clients can
/// independently retain output and suppress subsequent pty reads.
pub fn drain_consumers(server: &mut Server, pane: PaneId) {
    let Some(p) = server.panes.get(pane) else {
        return;
    };
    let base = p.base_offset;
    let mut minimum = p.parser_offset;
    if let Some(pipe) = &p.pipe {
        minimum = minimum.min(pipe.offset.used);
    }
    let mut off = true;
    let mut attached = false;
    for id in server.client_order.iter().copied() {
        let Some(c) = server.clients.get(id).filter(|c| c.session.is_some()) else {
            continue;
        };
        attached = true;
        if !c.flags.contains(ClientFlags::CONTROL) {
            off = false;
            continue;
        }
        let (offset, suppressed) = crate::control::pane_offset(server, id, pane);
        if !suppressed {
            off = false;
        }
        if let Some(offset) = offset {
            minimum = minimum.min(offset.used);
        }
    }
    if !attached {
        off = false;
    }
    if base > u64::MAX / 2 {
        crate::control::rebase_offsets(server, pane, base);
        let p = server.panes.get_mut(pane).expect("pane during rebase");
        p.parser_offset = p
            .parser_offset
            .checked_sub(base)
            .expect("parser offset rebase");
        if let Some(pipe) = &mut p.pipe {
            pipe.offset.used = pipe
                .offset
                .used
                .checked_sub(base)
                .expect("pipe offset rebase");
        }
        p.base_offset = 0;
        minimum = minimum.checked_sub(base).expect("minimum offset rebase");
    }
    pane_input::pane_drain_input(server, pane, minimum).expect("valid pane consumer offsets");
    if off {
        server.pane_read_disabled.insert(pane);
    } else {
        server.pane_read_disabled.remove(&pane);
    }
    if let Some(token) = server.pane_tokens.get(&pane).copied() {
        let p = server.panes.get(pane).expect("pane after drain");
        let failed = server.pane_io_failed.contains(&pane);
        server
            .event_loop
            .reregister(token, !failed && !off, !failed && !p.output.is_empty())
            .expect("reregister pane consumers");
    }
}

pub fn pane_ready(
    server: &mut Server,
    pane: PaneId,
    readable: bool,
    writable: bool,
) -> io::Result<()> {
    if writable && !server.pane_io_failed.contains(&pane) {
        let mut written = 0;
        while written < super::io::IO_BUDGET {
            let Some(p) = server.panes.get_mut(pane) else {
                return Ok(());
            };
            let Some(fd) = p.fd.as_ref() else {
                break;
            };
            if p.output.is_empty() {
                break;
            }
            let size = p.output.len().min(super::io::IO_BUDGET - written);
            match rmux_sys::fd::write(fd.as_fd(), &p.output[..size]) {
                Ok(0) => {
                    p.flags.insert(PaneFlags::EXITED);
                    server.pane_io_failed.insert(pane);
                    break;
                }
                Ok(n) => {
                    p.output.drain(..n);
                    written += n;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    p.flags.insert(PaneFlags::EXITED);
                    server.pane_io_failed.insert(pane);
                    break;
                }
            }
        }
    }
    if readable
        && !server.pane_io_failed.contains(&pane)
        && !server.pane_read_disabled.contains(&pane)
    {
        let mut buffer = [0u8; 8192];
        loop {
            let Some(p) = server.panes.get(pane) else {
                return Ok(());
            };
            let Some(fd) = p.fd.as_ref() else {
                break;
            };
            match rmux_sys::fd::read(fd.as_fd(), &mut buffer) {
                Ok(0) => {
                    server
                        .panes
                        .get_mut(pane)
                        .expect("read pane")
                        .flags
                        .insert(PaneFlags::EXITED);
                    server.pane_io_failed.insert(pane);
                    break;
                }
                Ok(n) => {
                    pane_read(server, pane, &buffer[..n]).map_err(io::Error::other)?;
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    server
                        .panes
                        .get_mut(pane)
                        .expect("read pane")
                        .flags
                        .insert(PaneFlags::EXITED);
                    server.pane_io_failed.insert(pane);
                    break;
                }
            }
        }
    }
    if let Some(p) = server
        .panes
        .get(pane)
        .filter(|p| p.flags.contains(PaneFlags::EXITED))
    {
        let unread =
            p.fd.as_ref()
                .map_or(Ok(0), |fd| rmux_sys::server::pending_bytes(fd.as_fd()))?;
        let pipe_empty = p.pipe.as_ref().is_none_or(|pipe| pipe.io.output_len() == 0);
        if model::pane::pane_destroy_ready(server, pane, pipe_empty, unread) {
            super::operations::server_destroy_pane(server, pane, true).map_err(io::Error::other)?;
        }
    }
    sync_panes(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;
    use crate::{
        client::Client,
        model::{
            session::{self, SessionCreate},
            spawn::SpawnFlags,
            window,
        },
        options::environment::Environment,
    };
    use std::os::unix::net::UnixStream;

    fn fixture() -> (Server, PaneId, ClientId) {
        let mut server = Server::new();
        let window = window::window_create(&mut server, 20, 4, 0, 0).unwrap();
        let pane =
            window::window_add_pane(&mut server, window, None, 10, SpawnFlags::default()).unwrap();
        let options = server.options.global_s;
        let session = session::session_create(
            &mut server,
            SessionCreate {
                prefix: None,
                name: Some(b"pane-host".to_vec()),
                cwd: b"/".to_vec(),
                environment: Environment::new(),
                options,
                termios: None,
            },
        );
        let link = session::session_attach(&mut server, session, window, 0).unwrap();
        server.sessions.get_mut(session).unwrap().current = Some(link);
        let mut client = Client::new(None, (0, 0));
        client.session = Some(session);
        client.flags.insert(ClientFlags::CONTROL);
        client.control = Some(crate::control::ControlState::default());
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        (server, pane, id)
    }

    fn assert_restored(
        server: &mut Server,
        name: &[u8],
        _: Option<crate::ids::SessionId>,
        _: Option<crate::ids::WindowId>,
        pane: Option<PaneId>,
    ) {
        if name != b"pane-title-changed" && name != b"pane-shell-prompt" {
            return;
        }
        let pane = pane.unwrap();
        let client = *server.client_order.front().unwrap();
        let client = server
            .clients
            .get_mut(client)
            .expect("root clients restored before parser callback");
        client.retval += 1;
        if name == b"pane-title-changed" {
            assert_eq!(server.panes.get(pane).unwrap().base.title, b"leased title");
        }
    }

    #[test]
    fn actual_parser_callbacks_restore_clients_between_steps() {
        let (mut server, pane, client) = fixture();
        server.model_event = Some(assert_restored);
        let input = b"\x1b]2;leased title\x07\x1b]133;A\x07X";
        assert_eq!(
            pane_parse_buffer(&mut server, pane, input).unwrap(),
            input.len()
        );
        assert_eq!(server.clients.get(client).unwrap().retval, 2);
        assert_eq!(
            server
                .panes
                .get(pane)
                .unwrap()
                .base
                .grid
                .view_get_cell(0, 0)
                .data
                .bytes(),
            b"X"
        );
    }

    fn assert_consumers_before_parser(
        server: &mut Server,
        name: &[u8],
        _: Option<crate::ids::SessionId>,
        _: Option<crate::ids::WindowId>,
        pane: Option<PaneId>,
    ) {
        if name != b"pane-title-changed" {
            return;
        }
        let pane = pane.unwrap();
        let p = server.panes.get(pane).unwrap();
        let pipe = p.pipe.as_ref().unwrap();
        assert_eq!(pipe.offset.used, p.input.len() as u64);
        assert_eq!(pipe.io.output_len(), p.input.len());
        let id = *server.client_order.front().unwrap();
        let control = server.clients.get(id).unwrap().control.as_ref().unwrap();
        assert!(control.panes.contains_key(&p.public_id));
        assert_eq!(
            control.panes[&p.public_id].queued.used,
            p.input.len() as u64
        );
        server.clients.get_mut(id).unwrap().retval = 1;
    }

    #[test]
    fn actual_parser_observes_pipe_and_control_delivery_first() {
        let (mut server, pane, client) = fixture();
        let (pipe, _child) = UnixStream::pair().unwrap();
        server.panes.get_mut(pane).unwrap().pipe = Some(pipe_pane::PipePaneState {
            io: super::super::io::BufferedIo::new(pipe.into()),
            pid: rmux_sys::proc::getpid(),
            offset: model::pane::PaneOffset::default(),
            input_enabled: false,
            output_enabled: true,
            token: None,
        });
        server.model_event = Some(assert_consumers_before_parser);
        let input = b"\x1b]2;ordered\x07X";
        pane_read(&mut server, pane, input).unwrap();
        assert_eq!(server.clients.get(client).unwrap().retval, 1);
        assert_eq!(
            server.panes.get(pane).unwrap().parser_offset,
            input.len() as u64
        );
        assert!(server.pane_read_disabled.contains(&pane));
    }

    #[test]
    fn drain_keeps_slowest_consumer_and_attached_normal_client_enables_reads() {
        let (mut server, pane, client) = fixture();
        pane_read(&mut server, pane, b"abcdef").unwrap();
        drain_consumers(&mut server, pane);
        assert_eq!(server.panes.get(pane).unwrap().base_offset, 0);
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .remove(ClientFlags::CONTROL);
        server.clients.get_mut(client).unwrap().control = None;
        drain_consumers(&mut server, pane);
        assert_eq!(server.panes.get(pane).unwrap().base_offset, 6);
        assert!(server.panes.get(pane).unwrap().input.is_empty());
        assert!(!server.pane_read_disabled.contains(&pane));
    }

    #[test]
    fn rebase_updates_paused_control_sent_and_queued_and_pipe() {
        let (mut server, pane, client) = fixture();
        let base = u64::MAX - 64;
        let public = server.panes.get(pane).unwrap().public_id;
        let (pipe, _child) = UnixStream::pair().unwrap();
        let p = server.panes.get_mut(pane).unwrap();
        p.base_offset = base;
        p.parser_offset = base + 6;
        p.input.extend_from_slice(b"abcdef");
        p.pipe = Some(pipe_pane::PipePaneState {
            io: super::super::io::BufferedIo::new(pipe.into()),
            pid: rmux_sys::proc::getpid(),
            offset: model::pane::PaneOffset { used: base + 5 },
            input_enabled: false,
            output_enabled: true,
            token: None,
        });
        let control = server
            .clients
            .get_mut(client)
            .unwrap()
            .control
            .as_mut()
            .unwrap();
        control.add_pane(public, pane, model::pane::PaneOffset { used: base });
        let cp = control.panes.get_mut(&public).unwrap();
        cp.flags.paused = true;
        cp.sent.used = base + 2;
        cp.queued.used = base + 4;
        drain_consumers(&mut server, pane);
        let cp = &server
            .clients
            .get(client)
            .unwrap()
            .control
            .as_ref()
            .unwrap()
            .panes[&public];
        assert_eq!((cp.sent.used, cp.queued.used), (2, 4));
        assert_eq!(
            server
                .panes
                .get(pane)
                .unwrap()
                .pipe
                .as_ref()
                .unwrap()
                .offset
                .used,
            5
        );
        assert_eq!(server.panes.get(pane).unwrap().parser_offset, 6);
        assert_eq!(server.panes.get(pane).unwrap().base_offset, 5);
    }

    #[test]
    fn readiness_flushes_output_and_close_removes_registration() {
        use std::io::Read;
        let (mut server, pane, _) = fixture();
        let (master, mut child) = UnixStream::pair().unwrap();
        let p = server.panes.get_mut(pane).unwrap();
        p.fd = Some(master.into());
        p.output.extend_from_slice(b"reply");
        sync_panes(&mut server).unwrap();
        assert!(server.pane_tokens.contains_key(&pane));
        pane_ready(&mut server, pane, false, true).unwrap();
        let mut bytes = [0; 5];
        child.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"reply");
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        close_pane_io(&mut server, pane);
        assert!(!server.pane_tokens.contains_key(&pane));
        assert!(server.panes.get(pane).unwrap().fd.is_none());
    }

    #[test]
    fn child_exit_still_reads_output_but_io_eof_does_not_rearm() {
        use std::io::Write;
        let (mut server, pane, _) = fixture();
        let (master, mut child) = UnixStream::pair().unwrap();
        let p = server.panes.get_mut(pane).unwrap();
        p.fd = Some(master.into());
        p.flags.insert(PaneFlags::EXITED);
        p.wait_item = Some(crate::ids::QueueItemId::from_parts(0, 0));
        sync_panes(&mut server).unwrap();
        child.write_all(b"Z").unwrap();
        pane_ready(&mut server, pane, true, false).unwrap();
        assert_eq!(
            server
                .panes
                .get(pane)
                .unwrap()
                .base
                .grid
                .view_get_cell(0, 0)
                .data
                .bytes(),
            b"Z"
        );
        assert!(!server.pane_io_failed.contains(&pane));
        drop(child);
        drain_consumers(&mut server, pane);
        pane_ready(&mut server, pane, true, false).unwrap();
        assert!(server.pane_io_failed.contains(&pane));
        drain_consumers(&mut server, pane);
        sync_panes(&mut server).unwrap();
        assert!(
            server
                .event_loop
                .poll(Some(std::time::Duration::ZERO))
                .unwrap()
                .is_empty()
        );
        close_pane_io(&mut server, pane);
    }

    #[test]
    fn history_lines_are_stamped_at_each_parse_step() {
        let (mut server, pane, _) = fixture();
        server.start_time = (100, 0);
        server.current_time = (105, 0);
        pane_parse_buffer(&mut server, pane, b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n").unwrap();
        let grid = &server.panes.get(pane).unwrap().base.grid;
        assert!(grid.hsize() > 0);
        assert_eq!(grid.lines()[0].time.to_wall(server.start_time.0), 105);
        server.current_time = (109, 0);
        pane_parse_buffer(&mut server, pane, b"six\r\n").unwrap();
        let grid = &server.panes.get(pane).unwrap().base.grid;
        assert_eq!(grid.lines()[grid.hsize() as usize - 1].time.to_wall(server.start_time.0), 109);
    }

    #[test]
    fn small_history_limit_clear_and_ed_history_use_production_host() {
        let (mut server, pane, _) = fixture();
        model::pane::pane_resize(&mut server, pane, 6, 3).unwrap();
        server.panes.get_mut(pane).unwrap().base.grid.set_hlimit(2);
        pane_parse_buffer(&mut server, pane, b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix").unwrap();
        let grid = &server.panes.get(pane).unwrap().base.grid;
        assert_eq!((grid.sx(), grid.sy(), grid.hsize(), grid.hlimit()), (6, 3, 2, 2));
        server.panes.get_mut(pane).unwrap().base.grid.clear_history();
        assert_eq!(server.panes.get(pane).unwrap().base.grid.hsize(), 0);
        assert_eq!(server.panes.get(pane).unwrap().base.grid.view_get_cell(0, 0).data.bytes(), b"f");
        assert_eq!(server.panes.get(pane).unwrap().base.grid.view_get_cell(0, 2).data.bytes(), b"s");
        server.panes.get_mut(pane).unwrap().base.grid.set_hlimit(5);
        pane_parse_buffer(&mut server, pane, b"\x1b[H\x1b[JZ").unwrap();
        assert!(server.panes.get(pane).unwrap().base.grid.hsize() > 0);
        assert_eq!(server.panes.get(pane).unwrap().base.grid.view_get_cell(0, 0).data.bytes(), b"Z");
    }
}
