// Ported from tmux server.c, proc.c, window.c, resize.c @ 8f25579c
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
use super::{
    event_loop::{LoopAction, LoopReady},
    proc::{PeerDispatch, proc_remove},
    protocol::ProtocolMessage,
};
use crate::{
    client::{ClientExitType, ClientFlags},
    cmd::{hooks, queue},
    ids::*,
    model::{self, ModelEffect, PaneFlags, Server},
};
use std::{
    collections::VecDeque,
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
            net::UnixListener,
        },
    },
    path::Path,
    time::Duration,
};
pub struct Startup {
    pub socket_path: Vec<u8>,
    pub flags: ClientFlags,
    pub initial_peer: Option<OwnedFd>,
    pub lock: Option<OwnedFd>,
    pub config_files: Vec<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub struct MessageEntry {
    pub msg_time: (i64, i64),
    pub msg_num: u32,
    pub msg: Vec<u8>,
}
#[derive(Default)]
pub struct MessageLog {
    pub entries: VecDeque<MessageEntry>,
    pub next: u32,
}
impl MessageLog {
    pub fn add(&mut self, now: (i64, i64), limit: u32, bytes: &[u8]) {
        let n = self.next;
        self.next = self.next.wrapping_add(1);
        self.entries.push_back(MessageEntry {
            msg_time: now,
            msg_num: n,
            msg: bytes.to_vec(),
        });
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.msg_num.wrapping_add(limit) < self.next)
        {
            self.entries.pop_front();
        }
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &MessageEntry> {
        self.entries.iter()
    }
}
pub fn add_message(server: &mut Server, message: &[u8]) {
    let now = wall_time();
    let limit = server
        .options
        .get_number(server.options.global, b"message-limit") as u32;
    server.message_log.add(now, limit, message);
}
fn wall_time() -> (i64, i64) {
    let t = rmux_util::time::Timestamp::now();
    (t.sec, i64::from(t.usec))
}
#[derive(Default)]
pub struct ListenerState {
    pub listener: Option<UnixListener>,
    pub token: Option<EventToken>,
    pub backoff: Option<TimerId>,
    pub attached_cache: Option<bool>,
    pub flags: ClientFlags,
}
pub fn server_add_accept(server: &mut Server, seconds: u32) {
    if let Some(token) = server.listener.token.take() {
        server.event_loop.deregister(token);
    }
    if let Some(timer) = server.listener.backoff.take() {
        server.event_loop.cancel(timer);
    }
    if seconds != 0 {
        server.listener.backoff = Some(server.event_loop.schedule(
            Duration::from_secs(u64::from(seconds)),
            LoopAction::AcceptBackoff,
        ));
        return;
    }
    if let Some(listener) = &server.listener.listener {
        server.listener.token = Some(
            server
                .event_loop
                .register(listener.as_fd(), true, false, LoopAction::Accept)
                .expect("register server listener"),
        );
    }
}
pub fn server_update_socket(server: &mut Server) {
    let attached = server
        .session_names
        .values()
        .any(|id| server.sessions.get(*id).is_some_and(|s| s.attached != 0));
    if server.listener.attached_cache == Some(attached) {
        return;
    }
    server.listener.attached_cache = Some(attached);
    let path = Path::new(std::ffi::OsStr::from_bytes(&server.socket_path));
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let mut mode = meta.mode() & 0o777;
    if attached {
        for (read, exec) in [(0o400, 0o100), (0o040, 0o010), (0o004, 0o001)] {
            if mode & read != 0 {
                mode |= exec;
            }
        }
    } else {
        mode &= !0o111;
    }
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}
fn accept(server: &mut Server) -> io::Result<()> {
    server_add_accept(server, 0);
    let Some(listener) = &server.listener.listener else {
        return Ok(());
    };
    match listener.accept() {
        Ok((stream, _)) => {
            if !server.shutting_down {
                let client = crate::client::lifecycle::create(server, stream.into())
                    .map_err(io::Error::other)?;
                if !super::acl::join(server, client) {
                    if let Some(c) = server.clients.get_mut(client) {
                        c.exit_message = Some(b"access not allowed".to_vec());
                        c.retval = 1;
                        c.flags.insert(ClientFlags::EXIT);
                    }
                }
            }
        }
        Err(e)
            if matches!(
                e.raw_os_error(),
                Some(libc::EAGAIN | libc::EINTR | libc::ECONNABORTED)
            ) => {}
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENFILE | libc::EMFILE)) => {
            server_add_accept(server, 1)
        }
        Err(e) => return Err(e),
    }
    Ok(())
}
pub fn initialize_process_options(server:&mut Server) {
    let mut options=std::mem::take(&mut server.options);
    let shell=rmux_util::shell::get_shell(b"rmux");
    options.set_string(options.global_s,b"default-shell",false,&shell,server);
    if let Some(editor)=std::env::var_os("VISUAL").or_else(||std::env::var_os("EDITOR")) {
        let editor=editor.as_bytes();options.set_string(options.global,b"editor",false,editor,server);
        let base=editor.rsplit(|&byte|byte==b'/').next().unwrap_or(editor);
        let keys=if base.windows(2).any(|word|word==b"vi"){rmux_util::key::ModeKeys::Vi}else{rmux_util::key::ModeKeys::Emacs} as i64;
        options.set_number_value(options.global_s,b"status-keys",keys);
        options.set_number_value(options.global_w,b"mode-keys",keys);
    }
    server.options=options;
}
pub fn server_start(mut startup: Startup) -> io::Result<i32> {
    rmux_util::log::open("server");
    let mut server = Server::new();
    initialize_process_options(&mut server);
    server.socket_path = std::mem::take(&mut startup.socket_path);
    server.listener.flags = startup.flags;
    for (name, value) in std::env::vars_os() {
        server.global_environment.set(
            name.as_bytes(),
            crate::options::environment::EnvironmentFlags::default(),
            value.as_bytes(),
        );
    }
    server.cfg.files = startup.config_files.into_iter().map(Into::into).collect();
    server.current_time = wall_time();
    server.start_time = server.current_time;
    let mask = rmux_sys::server::SignalMask::block()?;
    let mut signals = rmux_sys::server::SignalWake::new()?;
    server
        .event_loop
        .register(signals.fd(), true, false, LoopAction::Signal)?;
    drop(mask);
    server.model_event = Some(model_event);
    server.option_monitor_removed = Some(|s, id| hooks::monitor_free(s, id));
    server.hook_monitor_dispatch = Some(|s, id, change| hooks::monitor_change(s, id, change));
    crate::cmd::key_bindings::init(&mut server)
        .map_err(|e| io::Error::other(format!("default key binding: {e:?}")))?;
    crate::control::build_events(&mut server);
    hooks::build_events(&mut server);
    let listener = rmux_sys::server::bind_listener(
        &server.socket_path,
        startup.flags.contains(ClientFlags::DEFAULTSOCKET),
    );
    let cause = listener.as_ref().err().map(|e| {
        format!(
            "error creating {} ({})",
            String::from_utf8_lossy(&server.socket_path),
            e
        )
    });
    server.listener.listener = listener.ok();
    if server.listener.listener.is_some() {
        server_update_socket(&mut server);
    }
    let initial = if startup.flags.contains(ClientFlags::NOFORK) {
        server
            .options
            .set_number_value(server.options.global, b"exit-empty", 0);
        None
    } else {
        startup
            .initial_peer
            .take()
            .map(|fd| crate::client::lifecycle::create(&mut server, fd))
            .transpose()
            .map_err(io::Error::other)?
    };
    if let Some(lock) = startup.lock.take() {
        let mut lockpath = server.socket_path.clone();
        lockpath.extend_from_slice(b".lock");
        let _ = std::fs::remove_file(Path::new(std::ffi::OsStr::from_bytes(&lockpath)));
        drop(lock);
    }
    if let Some(cause) = cause {
        if let Some(id) = initial {
            let c = server.clients.get_mut(id).expect("initial client");
            c.exit_message = Some(cause.into_bytes());
            c.retval = 1;
            c.flags.insert(ClientFlags::EXIT);
        } else {
            eprintln!("{cause}");
            return Ok(1);
        }
    }
    super::acl::init(&mut server);
    server
        .event_loop
        .schedule(Duration::from_secs(3600), LoopAction::Tidy);
    server_add_accept(&mut server, 0);
    let mut ready = Vec::new();
    loop {
        drain_effects(&mut server)?;
        sync_panes(&mut server)?;
        if server_turn(&mut server)? {
            break;
        }
        server.event_loop.poll_into(None, &mut ready)?;
        for event in ready.drain(..) {
            if event.action == LoopAction::Signal {
                for signal in signals.drain()? {
                    server_signal(&mut server, signal)?;
                }
            } else {
                dispatch(&mut server, event)?;
            }
        }
    }
    super::job::kill_all(&server);
    crate::ui::prompt::history::save(&server);
    Ok(0)
}
fn server_turn(server: &mut Server) -> io::Result<bool> {
    server.current_time = wall_time();
    loop {
        let mut progress = queue::next(server, None);
        let clients: Vec<_> = server.client_order.iter().copied().collect();
        for id in clients {
            if server
                .clients
                .get(id)
                .is_some_and(|c| c.flags.contains(ClientFlags::IDENTIFIED))
            {
                progress = progress.saturating_add(queue::next(server, Some(id)));
            }
        }
        drain_effects(server)?;
        if progress == 0 {
            break;
        }
    }
    crate::client::tick::tick(server);
    drain_effects(server)?;
    server_update_socket(server);
    if !exit_eligible(
        server
            .options
            .get_number(server.options.global, b"exit-empty")
            != 0,
        server
            .options
            .get_number(server.options.global, b"exit-unattached")
            != 0,
        server.shutting_down,
        !server.session_names.is_empty(),
        server
            .client_order
            .iter()
            .any(|id| server.clients.get(*id).is_some_and(|c| c.session.is_some())),
    ) {
        return Ok(false);
    }
    crate::cmd::commands::wait_for::flush(server);
    Ok(server.client_order.is_empty() && !super::job::still_running(server))
}
pub fn exit_eligible(
    exit_empty: bool,
    exit_unattached: bool,
    shutdown: bool,
    sessions: bool,
    attached: bool,
) -> bool {
    if !exit_empty && !shutdown {
        return false;
    }
    if !exit_unattached && sessions {
        return false;
    }
    !attached
}
pub fn shutdown(server: &mut Server) {
    server.shutting_down = true;
    crate::cmd::commands::wait_for::flush(server);
    let clients: Vec<_> = server.client_order.iter().copied().collect();
    for id in clients {
        if server
            .clients
            .get(id)
            .is_some_and(|c| c.flags.contains(ClientFlags::SUSPENDED))
        {
            crate::client::lifecycle::lost(server, id);
        } else if let Some(c) = server.clients.get_mut(id) {
            c.flags.insert(ClientFlags::EXIT);
            c.exit_type = ClientExitType::Shutdown;
            c.session = None;
        }
    }
    let sessions: Vec<_> = server.session_names.values().copied().collect();
    for id in sessions {
        model::session::session_destroy(server, id, true);
    }
}
fn server_signal(server: &mut Server, signal: i32) -> io::Result<()> {
    match signal {
        libc::SIGINT | libc::SIGTERM => shutdown(server),
        libc::SIGCHLD => child_signal(server)?,
        libc::SIGUSR1 => {
            if let Some(token) = server.listener.token.take() {
                server.event_loop.deregister(token);
            }
            if let Ok(new) = rmux_sys::server::bind_listener(
                &server.socket_path,
                server.listener.flags.contains(ClientFlags::DEFAULTSOCKET),
            ) {
                server.listener.listener = Some(new);
                server_update_socket(server);
            }
            server_add_accept(server, 0);
        }
        libc::SIGUSR2 => {
            server.process.toggle_log();
            rmux_util::log::toggle("server");
        }
        _ => {}
    }
    Ok(())
}
fn child_signal(server: &mut Server) -> io::Result<()> {
    while let Some((pid, status)) = rmux_sys::server::wait_any()? {
        if let Some(stop) = rmux_sys::server::stop_signal(status) {
            if stop == libc::SIGTTIN || stop == libc::SIGTTOU {
                continue;
            }
            let panes: Vec<_> = server
                .pane_ids
                .values()
                .copied()
                .filter(|id| server.panes.get(*id).is_some_and(|p| p.pid == Some(pid)))
                .collect();
            if !panes.is_empty() && rmux_sys::server::continue_process_group(pid).is_err() {
                let _ = rmux_sys::server::continue_process(pid);
            }
        } else {
            let panes: Vec<_> = server
                .pane_ids
                .values()
                .copied()
                .filter(|id| server.panes.get(*id).is_some_and(|p| p.pid == Some(pid)))
                .collect();
            for id in panes {
                if let Some(p) = server.panes.get_mut(id) {
                    p.status = status;
                    p.flags.insert(PaneFlags::STATUSREADY | PaneFlags::EXITED);
                }
                model::pane::pane_wait_finish(server, id);
                model::spawn::spawn_editor_finish(server, id);
                if crate::cmd::commands::pipe_pane::destroy_ready(server, id) {
                    let _ = super::operations::server_destroy_pane(server, id, true);
                }
            }
        }
        super::job::check_died(server, pid, status)?;
    }
    Ok(())
}
fn dispatch(server: &mut Server, event: LoopReady) -> io::Result<()> {
    let LoopReady {
        action,
        readable,
        writable,
    } = event;
    match action {
        LoopAction::Accept => {
            if readable {
                accept(server)?;
            }
        }
        LoopAction::AcceptBackoff => server_add_accept(server, 0),
        LoopAction::Peer(id) => {
            let messages = server.process.ready(id, readable, writable);
            for dispatch in messages {
                let client = server
                    .client_order
                    .iter()
                    .copied()
                    .find(|cid| server.clients.get(*cid).is_some_and(|c| c.peer == Some(id)));
                match dispatch {
                    PeerDispatch::Message(_, message) => {
                        if let Some(client) = client {
                            dispatch_message(server, client, message);
                        }
                    }
                    PeerDispatch::Closed(_) => {
                        if let Some(client) = client {
                            crate::client::dispatch::on_closed(server, client);
                        } else {
                            proc_remove(server, id);
                        }
                    }
                }
            }
            if server.process.peers.get(id).is_some() {
                let _ = server.process.update_event(id, &mut server.event_loop);
            }
        }
        LoopAction::Pane(id) => pane_ready(server, id, readable, writable)?,
        LoopAction::Job(id) => super::job::on_ready(server, id, readable, writable)?,
        LoopAction::PanePipe(id) => {
            crate::cmd::commands::pipe_pane::on_ready(server, id, readable, writable)
        }
        LoopAction::ClientCycleTimer(id) => super::format_live::cycle_timer(server, id),
        LoopAction::File(id) => super::file::on_ready(server, id, readable, writable),
        LoopAction::FileDone(id) => super::file::fire_done(server, id),
        LoopAction::FilePush(id) => super::file::push(server, id),
        LoopAction::ControlRead(id) => {
            if readable {
                crate::control::on_read(server, id);
            }
            if writable {
                crate::control::on_write(server, id);
            }
        }
        LoopAction::ControlWrite(id) => crate::control::on_write(server, id),
        LoopAction::ControlMonitor(id) => crate::control::monitor_timer(server, id),
        LoopAction::ClientRepeatTimer(id) => crate::client::keys::repeat_timer(server, id),
        LoopAction::ClientClickTimer(id) => crate::client::mouse::click_timer(server, id),
        LoopAction::ClientExitTimer(id) => crate::client::exit::exit_timer(server, id),
        LoopAction::ClientFree(id) => crate::client::lifecycle::free(server, id),
        LoopAction::PaneResizeTimer(id) => crate::client::tick::resize_timer(server, id),
        LoopAction::RedrawTimer => crate::client::tick::redraw_timer(server),
        LoopAction::StatusTimer(id) => crate::ui::status::status_timer_fire(server, id),
        LoopAction::MessageTimer(id) => crate::ui::status::status_message_expire(server, id),
        action @ (LoopAction::SessionFree(_)
        | LoopAction::SessionLock(_)
        | LoopAction::WindowName(_)
        | LoopAction::WindowSilence(_)
        | LoopAction::PaneInputTimer(_, _)
        | LoopAction::PaneScrollbar(_)
        | LoopAction::AlertsCheck) => super::effects::timer_ready(server, action)?,
        LoopAction::Tidy => {
            super::format_live::tidy(server);
            server
                .event_loop
                .schedule(Duration::from_secs(3600), LoopAction::Tidy);
        }
        LoopAction::Signal => {}
        LoopAction::Deferred(id) => {
            if let Some(callback) = server.deferred.remove(&id) {
                callback(server);
            }
        }
    }
    Ok(())
}
fn model_event(
    server: &mut Server,
    name: &[u8],
    session: Option<SessionId>,
    window: Option<WindowId>,
    pane: Option<PaneId>,
) {
    use crate::cmd::find::{self, CmdFindFlags};
    let mut strings: Vec<(&[u8], Vec<u8>)> = Vec::new();
    let mut ints: Vec<(&[u8], i32)> = Vec::new();
    let mut link = None;
    match server.effects.back() {
        Some(ModelEffect::Paste(event)) => strings.push((b"name", event.name.as_bytes().to_vec())),
        Some(ModelEffect::Session(
            model::session::SessionEffect::WindowLinked { winlink, index, .. }
            | model::session::SessionEffect::WindowUnlinked { winlink, index, .. },
        )) => {
            link = Some(*winlink);
            ints.push((b"window_index", *index));
        }
        Some(ModelEffect::Session(model::session::SessionEffect::WindowChanged {
            new_index,
            old,
            ..
        })) => {
            ints.push((b"window_index", *new_index));
            if let Some((_, idx)) = old {
                ints.push((b"old_window_index", *idx));
            }
        }
        Some(ModelEffect::Session(model::session::SessionEffect::GroupChanged {
            group,
            size,
            ..
        })) => {
            strings.push((b"session_group", group.clone()));
            ints.push((b"session_group_size", *size as i32));
        }
        Some(ModelEffect::Window(model::window::WindowEffect::Renamed { old, .. })) => {
            strings.push((b"old_name", old.clone()))
        }
        Some(ModelEffect::Pane(model::pane::PaneEffect::ModeChanged {
            previous, current, ..
        })) => {
            if let Some(previous) = previous {
                strings.push((b"old_mode", previous.clone()));
            }
            if let Some(current) = current {
                strings.push((b"pane_mode", current.clone()));
            }
        }
        Some(ModelEffect::Resize(model::resize::ResizeEffect::Resized {
            old_sx, old_sy, ..
        })) => {
            ints.push((b"old_width", *old_sx as i32));
            ints.push((b"old_height", *old_sy as i32));
        }
        _ => {}
    }
    let mut target = if let Some(id) = pane {
        find::from_pane(server, id, CmdFindFlags::default())
    } else if let Some(id) = link {
        Some(find::from_winlink(server, id, CmdFindFlags::default()))
    } else if let Some(id) = session {
        Some(find::from_session(server, id, CmdFindFlags::default()))
    } else if let Some(id) = window {
        find::from_window(server, id, CmdFindFlags::default())
    } else {
        None
    }
    .unwrap_or_default();
    if let Some(s) = session {
        target.s = Some(s);
    }
    if let Some(w) = window {
        target.w = Some(w);
    }
    if let Some(p) = pane {
        target.wp = Some(p);
    }
    let mut payload = super::events::EventPayload::new();
    payload.set_target(server, &target);
    if let Some(s) = session.filter(|id| server.sessions.get(*id).is_some()) {
        payload.set_session(server, b"session", s);
    }
    if let Some(w) = window.filter(|id| server.windows.get(*id).is_some()) {
        payload.set_window(server, b"window", w);
    }
    if let Some(p) = pane.filter(|id| server.panes.get(*id).is_some()) {
        payload.set_pane(server, b"pane", p);
    }
    for (key, value) in strings {
        payload.set_string(server, key, &value);
    }
    for (key, value) in ints {
        payload.set_int(server, key, value);
    }
    super::events::fire(server, name, payload);
}
pub fn tty_update_client_offset(server: &mut Server, id: ClientId) {
    crate::client::lifecycle::update_offset(server, id);
}
pub fn format_lost_client(server: &mut Server, id: ClientId) {
    super::format_live::format_lost_client(server, id);
}
fn dispatch_message(server: &mut Server, client: ClientId, message: ProtocolMessage) {
    use super::protocol::ProtocolMessageKind::*;
    if matches!(message.kind, ReadData | ReadDone | WriteReady | WriteDone) {
        if super::file::handle_server(server, client, message).is_err() {
            crate::client::dispatch::on_closed(server, client);
        }
    } else {
        crate::client::dispatch::on_message(server, client, message);
    }
}
pub fn drain_effects(server: &mut Server) -> io::Result<()> {
    super::effects::drain_effects(server)
}
pub fn sync_panes(server: &mut Server) -> io::Result<()> {
    super::pane_runtime::sync_panes(server)
}
pub fn pane_read(server: &mut Server, id: PaneId, bytes: &[u8]) -> Result<(), model::ModelError> {
    super::pane_runtime::pane_read(server, id, bytes)
}
pub fn pane_parse_buffer(
    server: &mut Server,
    id: PaneId,
    bytes: &[u8],
) -> Result<usize, model::ModelError> {
    super::pane_runtime::pane_parse_buffer(server, id, bytes)
}
pub fn pane_ready(server: &mut Server, id: PaneId, read: bool, write: bool) -> io::Result<()> {
    super::pane_runtime::pane_ready(server, id, read, write)
}
pub fn close_pane_io(server: &mut Server, id: PaneId) {
    super::pane_runtime::close_pane_io(server, id);
}
pub fn resize_clients(server: &Server) -> Vec<model::resize::ResizeClient> {
    super::effects::resize_clients(server)
}
pub fn recalculate_sizes(server: &mut Server) {
    super::effects::recalculate_sizes(server);
}
pub fn recalculate_sizes_now(server: &mut Server, now: bool) {
    super::effects::recalculate_sizes_now(server, now);
}
pub fn tty_window_bigger(server: &Server, id: ClientId) -> bool {
    crate::client::lifecycle::window_offset(server, id).0
}
pub fn apply_option_changes(
    server: &mut Server,
    changes: Vec<crate::options::push::OptionsChange>,
) {
    super::effects::apply_option_changes(server, changes);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exit_empty_precedes_exit_unattached() {
        for shutdown in [false, true] {
            for empty in [false, true] {
                for unattached in [false, true] {
                    for sessions in [false, true] {
                        for attached in [false, true] {
                            assert_eq!(
                                exit_eligible(empty, unattached, shutdown, sessions, attached),
                                (empty || shutdown) && (unattached || !sessions) && !attached
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn log_uses_unsigned_age_and_zero_limit() {
        let mut log = MessageLog::default();
        log.add((1, 2), 0, b"gone");
        assert!(log.entries.is_empty());
        log.next = u32::MAX;
        log.add((2, 3), 1, b"wrap");
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].msg_num, u32::MAX);
        log.add((3, 4), 1, b"next");
        assert_eq!(log.entries.len(), 1);
    }
}
