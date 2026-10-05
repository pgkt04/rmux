// Ported from tmux control.c @ 8f25579c
use super::{ControlGuard, ControlState, ControlTransport, input::InputLine, io::ControlIo};
use crate::{
    client::ClientFlags,
    ids::{ClientId, PaneId},
    model::{Server, pane::PaneOffset},
    server::{
        event_loop::{LoopAction, LoopError},
        io::BufferedIo,
    },
};
use std::{
    io,
    time::{Duration, Instant},
};
fn state(server: &mut Server, id: ClientId) -> Option<&mut ControlState> {
    server.clients.get_mut(id).and_then(|c| c.control.as_mut())
}
fn register_io(
    server: &mut Server,
    io: &BufferedIo,
    action: LoopAction,
) -> Result<Option<crate::ids::EventToken>, LoopError> {
    if rmux_sys::server::descriptor_is_regular(io.fd())? {
        return Ok(None);
    }
    server
        .event_loop
        .register(io.fd(), false, false, action)
        .map(Some)
}
pub fn start(server: &mut Server, id: ClientId) -> Result<(), LoopError> {
    let c = server
        .clients
        .get_mut(id)
        .ok_or_else(|| io::Error::other("missing control client"))?;
    let fd =
        c.fd.take()
            .ok_or_else(|| io::Error::other("missing control input fd"))?;
    let double = c.flags.contains(ClientFlags::CONTROLCONTROL);
    let output = c.out_fd.take();
    let mut s = ControlState::new(double);
    let mut read = BufferedIo::new(fd);
    read.set_watermarks(1, None, super::BUFFER_LOW);
    read.enable_read(false);
    if double {
        drop(output);
        let token = register_io(server, &read, LoopAction::ControlRead(id))?;
        read.set_null(token.is_some_and(|token| server.event_loop.is_null(token)));
        s.io = Some(ControlIo::Shared { io: read, token });
    } else {
        let mut write =
            BufferedIo::new(output.ok_or_else(|| io::Error::other("missing control output fd"))?);
        write.set_watermarks(1, None, super::BUFFER_LOW);
        let read_token = register_io(server, &read, LoopAction::ControlRead(id))?;
        let write_token = match register_io(server, &write, LoopAction::ControlWrite(id)) {
            Ok(token) => token,
            Err(error) => {
                if let Some(token) = read_token {
                    server.event_loop.deregister(token);
                }
                return Err(error);
            }
        };
        read.set_null(read_token.is_some_and(|token| server.event_loop.is_null(token)));
        write.set_null(write_token.is_some_and(|token| server.event_loop.is_null(token)));
        s.io = Some(ControlIo::Separate {
            read,
            write,
            read_token,
            write_token,
        });
    }
    let monitors =
        match crate::model::monitor::monitor_create_client(server, id, super::monitor::changed) {
            Ok(set) => set,
            Err(error) => {
                if let Some(io) = s.io.take() {
                    match io {
                        ControlIo::Shared { token, .. } => {
                            if let Some(token) = token {
                                server.event_loop.deregister(token);
                            }
                        }
                        ControlIo::Separate {
                            read_token,
                            write_token,
                            ..
                        } => {
                            if let Some(token) = read_token {
                                server.event_loop.deregister(token);
                            }
                            if let Some(token) = write_token {
                                server.event_loop.deregister(token);
                            }
                        }
                    }
                }
                return Err(io::Error::other(error));
            }
        };
    s.monitors = Some(monitors);
    server.clients.get_mut(id).unwrap().control = Some(s);
    sync(server, id);
    Ok(())
}
pub fn stop(server: &mut Server, id: ClientId) {
    let Some(mut s) = server.clients.get_mut(id).and_then(|c| c.control.take()) else {
        return;
    };
    if let Some((timer, callback)) = s.direct_drive.take() {
        server.event_loop.cancel(timer);
        server.deferred.remove(&callback);
    }
    if let Some(set) = s.monitors.take() {
        let mut r = super::monitor::MonitorAdapter::new(server);
        let _ = crate::model::monitor::monitor_destroy(server, set, &mut r);
        r.apply(server);
    }
    if let Some(io) = s.io.take() {
        match io {
            ControlIo::Shared { token, .. } => {
                if let Some(token) = token {
                    server.event_loop.deregister(token);
                }
            }
            ControlIo::Separate {
                read_token,
                write_token,
                ..
            } => {
                if let Some(token) = read_token {
                    server.event_loop.deregister(token);
                }
                if let Some(token) = write_token {
                    server.event_loop.deregister(token);
                }
            }
        }
    }
}
pub fn ready(server: &mut Server, id: ClientId) {
    if let Some(s) = state(server, id) {
        s.ready();
    }
    sync(server, id);
}
pub fn write(server: &mut Server, id: ClientId, text: &[u8]) {
    if let Some(s) = state(server, id) {
        s.write(text);
    }
    sync(server, id);
}
pub fn notify_write(server: &mut Server, id: ClientId, text: &[u8]) {
    if let Some(s) = state(server, id) {
        s.notify_write(text);
    }
    sync(server, id);
}
pub fn write_guard(
    server: &mut Server,
    id: ClientId,
    guard: ControlGuard,
    time: i64,
    number: u32,
    flags: i32,
) {
    if let Some(s) = state(server, id) {
        s.write_guard(guard, time, number, flags);
    }
    sync(server, id);
}
pub fn discard(server: &mut Server, id: ClientId) {
    if let Some(s) = state(server, id) {
        s.discard();
    }
    sync(server, id);
}
pub fn discard_all(server: &mut Server, id: ClientId) {
    if let Some(s) = state(server, id) {
        s.discard_all();
    }
    sync(server, id);
}
pub fn all_done(server: &Server, id: ClientId) -> bool {
    server
        .clients
        .get(id)
        .and_then(|c| c.control.as_ref())
        .is_none_or(|s| s.all_done() && s.io.as_ref().is_none_or(|io| io.output_len() == 0))
}
pub fn reset_offsets(server: &mut Server, id: ClientId) {
    if let Some(s) = state(server, id) {
        s.reset_offsets();
    }
    refresh_flags(server, id);
}
pub fn pane_offset(server: &Server, id: ClientId, pane: PaneId) -> (Option<PaneOffset>, bool) {
    if server
        .clients
        .get(id)
        .is_some_and(|c| c.flags.contains(ClientFlags::CONTROL_NOOUTPUT))
    {
        return (None, false);
    }
    let Some(public) = server.panes.get(pane).map(|p| p.public_id) else {
        return (None, false);
    };
    let Some(s) = server.clients.get(id).and_then(|c| c.control.as_ref()) else {
        return (None, false);
    };
    let mut status = s.pane_offset(public);
    if status.offset.is_some()
        && s.io
            .as_ref()
            .is_some_and(|io| io.output_len() >= super::BUFFER_LOW)
    {
        status.suppress_read = true;
    }
    (status.offset, status.suppress_read)
}
pub fn reset_pane(server: &mut Server, id: ClientId, pane: PaneId) {
    let Some(p) = server.panes.get(pane) else {
        return;
    };
    let (public, offset) = (
        p.public_id,
        PaneOffset {
            used: p.parser_offset,
        },
    );
    if let Some(s) = state(server, id) {
        s.reset_pane(public, offset);
    }
}
pub fn write_output(server: &mut Server, id: ClientId, pane: PaneId) {
    let Some(p) = server.panes.get(pane) else {
        return;
    };
    let (public, window, parser, end) = (
        p.public_id,
        p.window,
        PaneOffset {
            used: p.parser_offset,
        },
        PaneOffset {
            used: p.base_offset + p.input.len() as u64,
        },
    );
    if !server
        .clients
        .get(id)
        .and_then(|c| c.session)
        .and_then(|s| server.sessions.get(s))
        .is_some_and(|s| {
            s.windows
                .values()
                .any(|l| server.winlinks.get(*l).is_some_and(|l| l.window == window))
        })
    {
        return;
    }
    let now = now_ms();
    refresh_flags(server, id);
    if let Some(s) = state(server, id) {
        s.write_output(public, pane, parser, end, now);
    }
    sync(server, id);
}
fn refresh_flags(server: &mut Server, id: ClientId) {
    if let Some(c) = server.clients.get_mut(id) {
        if let Some(s) = &mut c.control {
            s.no_output = c.flags.contains(ClientFlags::CONTROL_NOOUTPUT);
            s.ignore_output = c.flags.intersects(ClientFlags::UNATTACHEDFLAGS);
            s.pause_after = c
                .flags
                .contains(ClientFlags::CONTROL_PAUSEAFTER)
                .then_some(c.pause_age as u64);
        }
    }
}
pub(super) fn sync(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let Some(s) = &mut c.control else {
        return;
    };
    if s.exiting {
        c.flags.insert(ClientFlags::EXIT);
        c.exit_message = s.exit_reason.map(|s| s.as_bytes().to_vec());
    }
    if s.discard_replies {
        c.flags.insert(ClientFlags::CONTROL_DISCARD);
    }
    let Some(io) = &mut s.io else {
        return;
    };
    if !s.output.is_empty() {
        io.writer().queue(std::mem::take(&mut s.output));
    }
    io.reader().enable_read(s.read_enabled);
    io.writer().enable_write(s.write_enabled);
    let result = match io {
        ControlIo::Shared { io, token } => {
            if let Some(token) = token {
                let result = server
                    .event_loop
                    .reregister(*token, s.read_enabled, s.write_enabled);
                io.set_null(server.event_loop.is_null(*token));
                result
            } else {
                Ok(())
            }
        }
        ControlIo::Separate {
            read,
            write,
            read_token,
            write_token,
        } => {
            let result = if let Some(token) = read_token {
                let result = server.event_loop.reregister(*token, s.read_enabled, false);
                read.set_null(server.event_loop.is_null(*token));
                result
            } else {
                Ok(())
            };
            result.and_then(|()| {
                if let Some(token) = write_token {
                    let result = server.event_loop.reregister(*token, false, s.write_enabled);
                    write.set_null(server.event_loop.is_null(*token));
                    result
                } else {
                    Ok(())
                }
            })
        }
    };
    if result.is_err() {
        c.flags.insert(ClientFlags::EXIT);
    }
    let direct = match io {
        ControlIo::Shared { io, token } => token.is_none() && (io.interests().0 || s.write_enabled),
        ControlIo::Separate {
            read,
            read_token,
            write_token,
            ..
        } => {
            (read_token.is_none() && read.interests().0)
                || (write_token.is_none() && s.write_enabled)
        }
    };
    if direct && s.direct_drive.is_none() {
        let timer = crate::server::event_loop::schedule_deferred(
            server,
            Duration::ZERO,
            Box::new(move |server| direct_drive(server, id)),
        );
        if let Some(s) = state(server, id) {
            s.direct_drive = Some(timer);
        }
    }
}
fn direct_drive(server: &mut Server, id: ClientId) {
    let Some(s) = state(server, id) else {
        return;
    };
    s.direct_drive = None;
    let Some(io) = &mut s.io else {
        return;
    };
    let (read, write) = match io {
        ControlIo::Shared { io, token } => (
            token.is_none() && io.interests().0,
            token.is_none() && s.write_enabled,
        ),
        ControlIo::Separate {
            read,
            read_token,
            write_token,
            ..
        } => (
            read_token.is_none() && read.interests().0,
            write_token.is_none() && s.write_enabled,
        ),
    };
    if read {
        on_read(server, id);
    }
    if write {
        on_write(server, id);
    }
}
fn now_ms() -> u64 {
    static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    START.elapsed().as_millis() as u64
}
struct Host<'a> {
    panes: &'a crate::ids::Arena<crate::model::state::Pane, PaneId>,
    sessions: &'a crate::ids::Arena<crate::model::state::Session, crate::ids::SessionId>,
    winlinks: &'a crate::ids::Arena<crate::model::state::Winlink, crate::ids::WinlinkId>,
    session: Option<crate::ids::SessionId>,
}
impl ControlTransport for Host<'_> {
    fn pane_bytes(&self, id: PaneId, offset: PaneOffset) -> Option<&[u8]> {
        let p = self.panes.get(id)?;
        p.fd.as_ref()?;
        let s = self.sessions.get(self.session?)?;
        if !s
            .windows
            .values()
            .any(|l| self.winlinks.get(*l).is_some_and(|l| l.window == p.window))
        {
            return None;
        }
        p.input
            .get(usize::try_from(offset.used.checked_sub(p.base_offset)?).ok()?..)
    }
    fn now_ms(&self) -> u64 {
        now_ms()
    }
}
pub fn tick(server: &mut Server, id: ClientId) {
    refresh_flags(server, id);
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let Some(s) = &mut c.control else {
        return;
    };
    if s.io
        .as_ref()
        .is_none_or(|io| io.output_len() <= super::BUFFER_LOW)
    {
        let host = Host {
            panes: &server.panes,
            sessions: &server.sessions,
            winlinks: &server.winlinks,
            session: c.session,
        };
        s.service(&host);
    }
    sync(server, id);
}
pub fn on_write(server: &mut Server, id: ClientId) {
    let result = state(server, id)
        .and_then(|s| s.io.as_mut())
        .map(|io| io.writer().write_ready());
    if result.is_some_and(|r| r.is_err()) {
        if let Some(c) = server.clients.get_mut(id) {
            c.flags.insert(ClientFlags::EXIT);
        }
    }
    tick(server, id);
}
pub fn on_read(server: &mut Server, id: ClientId) {
    let Some(s) = state(server, id) else {
        return;
    };
    let Some(io) = &mut s.io else {
        return;
    };
    let result = io.reader().read_once();
    let bytes = io.reader().take_input();
    let lines = s.input.feed(&bytes);
    if result.is_err() || result.is_ok_and(|p| p.eof) {
        s.input.eof();
        s.exiting = true;
    }
    for line in lines {
        match line {
            InputLine::Exit => {
                if let Some(s) = state(server, id) {
                    s.exiting = true;
                }
                break;
            }
            InputLine::Command(line) => append_command(server, id, &line),
        }
    }
    sync(server, id);
}
fn append_command(server: &mut Server, id: ClientId, line: &[u8]) {
    use crate::cmd::{
        parse,
        queue::{self, QueueStateFlags},
    };
    let mut input = parse::CmdParseInput {
        client: Some(id),
        ..Default::default()
    };
    let result = parse::from_string(server, line, &mut input);
    let batch = match result {
        Ok(list) => {
            let mut store = std::mem::take(&mut server.queue);
            let qstate = store
                .new_state(server, None, None, QueueStateFlags::CONTROL)
                .expect("control queue state");
            server.queue = store;
            let batch = server
                .queue
                .get_command(list, Some(qstate))
                .expect("control command batch");
            server
                .queue
                .free_state(qstate)
                .expect("control queue state release");
            batch
        }
        Err(error) => {
            let cause = error.message().to_vec();
            server
                .queue
                .get_callback(
                    "control_error",
                    Box::new(move |runtime, item| {
                        queue::guard(runtime, item, queue::ControlGuard::Begin, 1);
                        let mut text = b"parse error: ".to_vec();
                        text.extend_from_slice(&cause);
                        queue::print(runtime, item, &text);
                        queue::guard(runtime, item, queue::ControlGuard::Error, 1);
                        queue::CmdReturn::Normal
                    }),
                )
                .expect("control error callback")
        }
    };
    queue::append(server, Some(id), batch).expect("control queue append");
}
pub fn set_window_size(server: &mut Server, id: ClientId, window: u32, width: u32, height: u32) {
    if let Some(s) = state(server, id) {
        s.set_window_size(window, width, height);
    }
}
pub fn get_window_size(server: &Server, id: ClientId, window: u32) -> Option<(u32, u32)> {
    server
        .clients
        .get(id)?
        .control
        .as_ref()?
        .get_window_size(window)
}
pub fn clear_window_size(server: &mut Server, id: ClientId, window: u32) {
    if let Some(s) = state(server, id) {
        s.clear_window_size(window);
    }
}
macro_rules! pane_action {
    ($name:ident) => {
        pub fn $name(server: &mut Server, id: ClientId, pane: PaneId) {
            let Some(p) = server.panes.get(pane) else {
                return;
            };
            let (public, offset) = (
                p.public_id,
                PaneOffset {
                    used: p.parser_offset,
                },
            );
            if let Some(s) = state(server, id) {
                s.$name(public, pane, offset);
            }
            sync(server, id);
        }
    };
}
pane_action!(set_pane_off);
pane_action!(pause_pane);
pub fn set_pane_on(server: &mut Server, id: ClientId, pane: PaneId) {
    let Some(p) = server.panes.get(pane) else {
        return;
    };
    let (public, offset) = (
        p.public_id,
        PaneOffset {
            used: p.parser_offset,
        },
    );
    if let Some(s) = state(server, id) {
        s.set_pane_on(public, offset);
    }
    sync(server, id);
}
pub fn continue_pane(server: &mut Server, id: ClientId, pane: PaneId) {
    let Some(p) = server.panes.get(pane) else {
        return;
    };
    let (public, offset) = (
        p.public_id,
        PaneOffset {
            used: p.parser_offset,
        },
    );
    if let Some(s) = state(server, id) {
        s.continue_pane(public, offset);
    }
    sync(server, id);
}
pub fn rebase_offsets(server: &mut Server, pane: PaneId, amount: u64) {
    let ids: Vec<_> = server.client_order.iter().copied().collect();
    for id in ids {
        if let Some(s) = state(server, id) {
            s.rebase_offsets(pane, amount);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regular_output_drains_pre_ready_guards_and_cancelled_drive() {
        let mut server = Server::new();
        let (input, _sender) = std::os::unix::net::UnixStream::pair().unwrap();
        let path = std::env::temp_dir().join(format!("rmux-control-direct-{}", std::process::id()));
        let output = std::fs::File::create(&path).unwrap();
        let mut client = crate::client::Client::new(None, (0, 0));
        client.flags = ClientFlags::CONTROL;
        client.fd = Some(input.into());
        client.out_fd = Some(output.into());
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        start(&mut server, id).unwrap();
        write_guard(&mut server, id, ControlGuard::Begin, 1, 2, 0);
        write_guard(&mut server, id, ControlGuard::End, 1, 2, 0);
        notify_write(&mut server, id, b"%session-changed $0 attached");
        let (_, callback) = state(&mut server, id).unwrap().direct_drive.unwrap();
        assert!(!state(&mut server, id).unwrap().read_enabled);
        ready(&mut server, id);
        assert!(state(&mut server, id).unwrap().read_enabled);
        server.deferred.remove(&callback).unwrap()(&mut server);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"%begin 1 2 0\n%end 1 2 0\n%session-changed $0 attached\n"
        );
        write(&mut server, id, b"cancelled");
        let (_, callback) = state(&mut server, id).unwrap().direct_drive.unwrap();
        stop(&mut server, id);
        assert!(!server.deferred.contains_key(&callback));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn commands_arrive_before_control_eof_marks_client_exiting() {
        use std::os::fd::AsFd;

        let mut server = Server::new();
        let (input, sender) = rmux_sys::fd::pipe().unwrap();
        rmux_sys::fd::write(sender.as_fd(), b"display-message -p value\n").unwrap();
        drop(sender);
        let output = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut client = crate::client::Client::new(None, (0, 0));
        client.flags = ClientFlags::CONTROL;
        client.fd = Some(input);
        client.out_fd = Some(output.into());
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        start(&mut server, id).unwrap();
        ready(&mut server, id);
        on_read(&mut server, id);
        assert!(server.queue.clients.get(&id).unwrap().head.is_some());
        assert!(
            !server
                .clients
                .get(id)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        on_read(&mut server, id);
        assert!(
            server
                .clients
                .get(id)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        stop(&mut server, id);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn fifo_writer_close_exits_control_client_without_input() {
        use std::os::fd::AsFd;
        use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};

        let path = std::env::temp_dir().join(format!("rmux-control-fifo-{}", std::process::id()));
        rmux_sys::server::make_fifo(path.as_os_str().as_bytes(), 0o600).unwrap();
        let input = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        let sender = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(rmux_sys::server::descriptor_is_fifo(input.as_fd()).unwrap());
        let output = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut server = Server::new();
        let mut client = crate::client::Client::new(None, (0, 0));
        client.flags = ClientFlags::CONTROL;
        client.fd = Some(input.into());
        client.out_fd = Some(output.into());
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        start(&mut server, id).unwrap();
        ready(&mut server, id);
        let events = server.event_loop.poll(Some(Duration::ZERO)).unwrap();
        assert!(!events.iter().any(|event| {
            matches!(event.action, LoopAction::ControlRead(client) if client == id)
                && event.readable
        }));
        assert!(
            !server
                .clients
                .get(id)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        drop(sender);
        let events = server
            .event_loop
            .poll(Some(Duration::from_millis(100)))
            .unwrap();
        assert!(events.iter().any(|event| {
            matches!(event.action, LoopAction::ControlRead(client) if client == id)
                && event.readable
        }));
        on_read(&mut server, id);
        assert!(
            server
                .clients
                .get(id)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        assert!(state(&mut server, id).unwrap().input.exited);
        assert!(state(&mut server, id).unwrap().direct_drive.is_none());
        stop(&mut server, id);
    }
}
