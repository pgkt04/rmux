// Ported from tmux input.c, window.c, screen-write.c @ 8f25579c
use super::PaneFlags;
use super::pane::{PaneOffset, pane_clear_resizes, pane_send_resize};
use super::state::{ModelEffect, ModelError, Server};
use super::window::{self, WindowEffect, option_number};
use crate::ids::{ArenaId, ClientId, PaneId, RequestId};
use rmux_emu::colour::{ClientTheme, Colour};
use rmux_emu::input::{
    self, ColourQueryKind, ExtendedKeys, GetClipboard, InputEffect, InputEnd, InputPolicy,
    InputReply, InputRequestKind, InputStep, Osc133Event, Passthrough,
};
use rmux_emu::screen::ScreenMode;
use rmux_emu::screen::write::{ScreenWriteCtx, ScreenWritePolicy, TtySink};
use std::collections::VecDeque;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnedInputEffect {
    Reply(Vec<u8>),
    Request {
        kind: InputRequestKind,
        end: InputEnd,
    },
    ClipboardQuery {
        clip: u8,
        end: InputEnd,
    },
    ClipboardReceived {
        clip: Vec<u8>,
        data: Vec<u8>,
    },
    ColourQuery {
        which: ColourQueryKind,
        end: InputEnd,
    },
    ThemeReport,
    ThemeUpdatesEnabled,
    ThemeUpdatesDisabled,
    Bell,
    TitleChanged(Vec<u8>),
    TitlePopped(Vec<u8>),
    PathChanged,
    Rename(Option<Vec<u8>>),
    ProgressChanged,
    StyleChanged {
        theme: bool,
    },
    SyncStart,
    SyncEnd,
    Osc133(Osc133Event),
    GroundTimer(bool),
    AlternateChanged {
        entering: bool,
    },
}
impl From<InputEffect<'_>> for OwnedInputEffect {
    fn from(value: InputEffect<'_>) -> Self {
        match value {
            InputEffect::Reply(bytes) => Self::Reply(bytes.to_vec()),
            InputEffect::Request { kind, end } => Self::Request { kind, end },
            InputEffect::ClipboardQuery { clip, end } => Self::ClipboardQuery { clip, end },
            InputEffect::ClipboardReceived { clip, data } => Self::ClipboardReceived {
                clip: clip.to_vec(),
                data: data.to_vec(),
            },
            InputEffect::ColourQuery { which, end } => Self::ColourQuery { which, end },
            InputEffect::ThemeReport => Self::ThemeReport,
            InputEffect::ThemeUpdatesEnabled => Self::ThemeUpdatesEnabled,
            InputEffect::ThemeUpdatesDisabled => Self::ThemeUpdatesDisabled,
            InputEffect::Bell => Self::Bell,
            InputEffect::TitleChanged(bytes) => Self::TitleChanged(bytes.to_vec()),
            InputEffect::TitlePopped(bytes) => Self::TitlePopped(bytes.to_vec()),
            InputEffect::PathChanged => Self::PathChanged,
            InputEffect::Rename(name) => Self::Rename(name.map(<[u8]>::to_vec)),
            InputEffect::ProgressChanged => Self::ProgressChanged,
            InputEffect::StyleChanged { theme } => Self::StyleChanged { theme },
            InputEffect::SyncStart => Self::SyncStart,
            InputEffect::SyncEnd => Self::SyncEnd,
            InputEffect::Osc133(event) => Self::Osc133(event),
            InputEffect::GroundTimer(arm) => Self::GroundTimer(arm),
            InputEffect::AlternateChanged { entering } => Self::AlternateChanged { entering },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputTimer {
    Ground,
    Requests,
    Sync,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputAction {
    Effect {
        pane: PaneId,
        effect: OwnedInputEffect,
    },
    Timer {
        pane: PaneId,
        timer: InputTimer,
        after: Option<Duration>,
    },
    CancelRequest {
        pane: PaneId,
        request: RequestId,
        client: ClientId,
    },
    ResetConsumers(PaneId),
    StdinStart {
        pane: PaneId,
        client: ClientId,
        item: crate::ids::QueueItemId,
    },
    StdinCancel {
        pane: PaneId,
        client: ClientId,
        item: crate::ids::QueueItemId,
        exit_status: Option<i32>,
    },
    StdinFinished {
        client: ClientId,
        item: crate::ids::QueueItemId,
    },
}
#[derive(Debug)]
pub enum RequestKind {
    Palette { idx: u8 },
    Clipboard { clip: u8 },
    Queue(Vec<u8>),
}
#[derive(Debug)]
pub struct InputRequest {
    pub id: RequestId,
    pub sequence: u64,
    pub client: Option<ClientId>,
    pub kind: RequestKind,
    pub end: InputEnd,
    pub created_ms: u64,
}
#[derive(Debug, Default)]
pub struct PaneInputState {
    pub requests: VecDeque<InputRequest>,
    pub ground_timer: bool,
    pub request_timer: bool,
    pub sync_timer: bool,
    pub last_theme: Option<ClientTheme>,
    pub theme_updates: bool,
    pub buffer_generation: u64,
}
impl PaneInputState {
    pub fn clear(&mut self) {
        self.requests.clear();
        self.ground_timer = false;
        self.request_timer = false;
        self.sync_timer = false;
        self.buffer_generation = self.buffer_generation.wrapping_add(1);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InputClient {
    pub id: ClientId,
    pub attached: bool,
    pub tty_started: bool,
    pub activity: (i64, i64),
    pub has_window: bool,
}
pub fn select_request_client(clients: impl IntoIterator<Item = InputClient>) -> Option<ClientId> {
    let mut best: Option<InputClient> = None;
    for client in clients {
        if !client.attached || !client.tty_started || !client.has_window {
            continue;
        }
        if best.is_none_or(|best| client.activity > best.activity) {
            best = Some(client);
        }
    }
    best.map(|client| client.id)
}

/// Synchronous runtime boundary: callbacks may mutate/destroy model objects.
/// Pipe and control delivery complete before parsing; alternate/sync repairs
/// complete before the next parser handler runs.
pub trait PaneInputHost {
    fn begin_draw(&mut self, _server: &mut Server, _pane: PaneId) {}
    fn end_draw(&mut self, _server: &mut Server, _pane: PaneId) {}
    fn tty_sink(&mut self) -> &mut dyn TtySink;
    fn write_policy(&self, server: &Server, pane: PaneId) -> ScreenWritePolicy;
    fn now_ms(&self) -> u64;
    fn request_client(&self, server: &Server, pane: PaneId) -> Option<ClientId>;
    fn send_request(
        &mut self,
        server: &mut Server,
        pane: PaneId,
        request: RequestId,
        client: ClientId,
        kind: InputRequestKind,
    );
    fn colour(&self, server: &Server, pane: PaneId, which: ColourQueryKind) -> Colour;
    fn theme(&self, server: &Server, pane: PaneId) -> ClientTheme;
    fn effect(&mut self, server: &mut Server, pane: PaneId, effect: &OwnedInputEffect);
    fn pipe_output(&mut self, server: &mut Server, pane: PaneId);
    fn control_output(&mut self, server: &mut Server, pane: PaneId);
    fn disable_reads(&mut self, server: &mut Server, pane: PaneId);
    fn stop_sync(&mut self, server: &mut Server, pane: PaneId);
}

fn action(server: &mut Server, value: InputAction) {
    server.effects.push_back(ModelEffect::Input(value));
}
fn timer(server: &mut Server, pane: PaneId, kind: InputTimer, after: Option<Duration>) {
    action(
        server,
        InputAction::Timer {
            pane,
            timer: kind,
            after,
        },
    );
}

pub fn input_policy(server: &Server, id: PaneId) -> Result<InputPolicy, ModelError> {
    let pane = server.panes.get(id).ok_or(ModelError::StaleId)?;
    let w = server.windows.get(pane.window).ok_or(ModelError::StaleId)?;
    let global = server.options.global;
    Ok(InputPolicy {
        allow_passthrough: match option_number(server, pane.options, b"allow-passthrough", 0) {
            1 => Passthrough::On,
            2 => Passthrough::All,
            _ => Passthrough::Off,
        },
        allow_set_title: option_number(server, pane.options, b"allow-set-title", 1) != 0,
        allow_rename: option_number(server, pane.options, b"allow-rename", 0) != 0,
        extended_keys: match option_number(server, global, b"extended-keys", 0) {
            1 => ExtendedKeys::On,
            2 => ExtendedKeys::Always,
            _ => ExtendedKeys::Off,
        },
        cursor_style: option_number(server, pane.options, b"cursor-style", 0) as i32,
        set_clipboard_on: option_number(server, global, b"set-clipboard", 1) == 2,
        get_clipboard: clipboard_policy(server),
        buffer_limit: option_number(
            server,
            global,
            b"input-buffer-size",
            input::INPUT_BUF_DEFAULT_SIZE as i64,
        ) as usize,
        sixel: false,
        pixels: Some((w.xpixel, w.ypixel)),
        has_pane: true,
        writer_has_pane: pane.modes.is_empty(),
        reset_extended_keys: option_number(server, global, b"extended-keys", 0) == 2,
    })
}
fn clipboard_policy(server: &Server) -> GetClipboard {
    match option_number(server, server.options.global, b"get-clipboard", 1) {
        0 => GetClipboard::Off,
        2 => GetClipboard::Request,
        3 => GetClipboard::Both,
        _ => GetClipboard::Buffer,
    }
}

pub fn pane_get_new_data<'a>(
    server: &'a Server,
    id: PaneId,
    offset: &PaneOffset,
) -> Result<&'a [u8], ModelError> {
    let pane = server.panes.get(id).ok_or(ModelError::StaleId)?;
    let used = offset
        .used
        .checked_sub(pane.base_offset)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| ModelError::message(b"pane offset outside retained buffer"))?;
    pane.input
        .get(used..)
        .ok_or_else(|| ModelError::message(b"pane offset outside retained buffer"))
}
pub fn pane_update_used_data(
    server: &Server,
    id: PaneId,
    offset: &mut PaneOffset,
    size: usize,
) -> Result<(), ModelError> {
    let available = pane_get_new_data(server, id, offset)?.len();
    offset.used += size.min(available) as u64;
    Ok(())
}
pub fn pane_drain_input(
    server: &mut Server,
    id: PaneId,
    minimum_used: u64,
) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let used = minimum_used
        .min(p.parser_offset)
        .checked_sub(p.base_offset)
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v <= p.input.len())
        .ok_or_else(|| ModelError::message(b"pane offset outside retained buffer"))?;
    p.input.drain(..used);
    p.base_offset += used as u64;
    p.input_state.buffer_generation = p.input_state.buffer_generation.wrapping_add(1);
    Ok(())
}

pub fn pane_reset_io(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    cancel_pane_requests(server, id)?;
    crate::server::run::close_pane_io(server, id);
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    p.fd = None;
    p.input.clear();
    p.output.clear();
    p.parser = input::InputCtx::new();
    p.base_offset = 0;
    p.parser_offset = 0;
    p.input_state.clear();
    timer(server, id, InputTimer::Ground, None);
    timer(server, id, InputTimer::Requests, None);
    timer(server, id, InputTimer::Sync, None);
    action(server, InputAction::ResetConsumers(id));
    Ok(())
}
pub fn pane_stop_sync(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    if p.base.mode.contains(ScreenMode::SYNC) {
        p.base.mode.remove(ScreenMode::SYNC);
        p.input_state.sync_timer = false;
        server
            .effects
            .push_back(ModelEffect::Pane(super::pane::PaneEffect::StopSync(id)));
        timer(server, id, InputTimer::Sync, None);
    }
    Ok(())
}

pub fn pane_read(
    server: &mut Server,
    host: &mut dyn PaneInputHost,
    id: PaneId,
    bytes: &[u8],
) -> Result<(), ModelError> {
    server
        .panes
        .get_mut(id)
        .ok_or(ModelError::StaleId)?
        .input
        .extend_from_slice(bytes);
    host.pipe_output(server, id);
    if server.panes.get(id).is_none() {
        return Ok(());
    }
    host.control_output(server, id);
    if server.panes.get(id).is_none() {
        return Ok(());
    }
    pane_parse_pending(server, host, id)?;
    if server.panes.get(id).is_some() {
        host.disable_reads(server, id);
    }
    Ok(())
}

pub fn pane_parse_pending(
    server: &mut Server,
    host: &mut dyn PaneInputHost,
    id: PaneId,
) -> Result<(), ModelError> {
    parse_buffer(server, host, id, None).map(|_| ())
}

pub fn pane_parse_buffer(
    server: &mut Server,
    host: &mut dyn PaneInputHost,
    id: PaneId,
    bytes: &[u8],
) -> Result<usize, ModelError> {
    parse_buffer(server, host, id, Some(bytes))
}
pub struct PaneStdinInput {
    pub pane: PaneId,
    pub client: ClientId,
    pub item: crate::ids::QueueItemId,
    cancelled: bool,
    finished: bool,
}
impl PaneStdinInput {
    pub fn new(pane: PaneId, client: ClientId, item: crate::ids::QueueItemId) -> Self {
        Self {
            pane,
            client,
            item,
            cancelled: false,
            finished: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StdinReadState {
    pub client_dead: bool,
    pub file_present: bool,
    pub closed: bool,
    pub error: bool,
}
pub fn pane_stdin_input(
    server: &mut Server,
    host: &mut dyn PaneInputHost,
    input: &mut PaneStdinInput,
    buffer: &mut Vec<u8>,
    state: StdinReadState,
) -> Result<(), ModelError> {
    let StdinReadState {
        client_dead,
        file_present,
        closed,
        error,
    } = state;
    if input.finished {
        buffer.clear();
        return Ok(());
    }
    let alive = server
        .panes
        .get(input.pane)
        .is_some_and(|p| !p.flags.contains(PaneFlags::DESTROYED));
    let result = if file_present && (!alive || client_dead) {
        if !input.cancelled {
            input.cancelled = true;
            action(
                server,
                InputAction::StdinCancel {
                    pane: input.pane,
                    client: input.client,
                    item: input.item,
                    exit_status: (!alive).then_some(1),
                },
            );
        }
        Ok(())
    } else if !file_present || closed || error {
        input.finished = true;
        action(
            server,
            InputAction::StdinFinished {
                client: input.client,
                item: input.item,
            },
        );
        Ok(())
    } else {
        pane_parse_buffer(server, host, input.pane, buffer).map(|_| ())
    };
    buffer.clear();
    result
}

fn parse_buffer(
    server: &mut Server,
    host: &mut dyn PaneInputHost,
    id: PaneId,
    external: Option<&[u8]>,
) -> Result<usize, ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let start = if external.is_some() {
        0
    } else {
        p.parser_offset
            .checked_sub(p.base_offset)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= p.input.len())
            .ok_or_else(|| ModelError::message(b"pane offset outside retained buffer"))?
    };
    let length = external.map_or(p.input.len() - start, <[u8]>::len);
    if length == 0 {
        return Ok(0);
    }
    let generation = p.input_state.buffer_generation;
    p.output_generation = p.output_generation.wrapping_add(1);
    p.last_output_time = server.current_time.0;
    let window = p.window;
    let activity = !p.flags.contains(PaneFlags::ACTIVITY);
    p.flags.insert(PaneFlags::ACTIVITY | PaneFlags::CHANGED);
    if !p.modes.is_empty() {
        p.flags.insert(PaneFlags::UNSEENCHANGES);
    }
    window::window_update_activity(server, window);
    if activity {
        server.emit(b"pane-activity", None, Some(window), Some(id));
    }
    let mut consumed_total = 0;
    let mut state = None;
    loop {
        if !server.panes.get(id).is_some_and(|p| {
            !p.flags.contains(PaneFlags::DESTROYED) && p.input_state.buffer_generation == generation
        }) {
            return Ok(consumed_total);
        }
        let policy = input_policy(server, id)?;
        let write_policy = host.write_policy(server, id);
        host.begin_draw(server, id);
        let (consumed, effect) = {
            let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
            let bytes = external.map_or_else(
                || &p.input[start + consumed_total..start + length],
                |bytes| &bytes[consumed_total..],
            );
            let mut sw = if let Some(state) = state.take() {
                ScreenWriteCtx::resume(
                    &mut p.base,
                    host.tty_sink(),
                    write_policy,
                    &mut server.hyperlinks,
                    state,
                )
            } else {
                ScreenWriteCtx::start(
                    &mut p.base,
                    host.tty_sink(),
                    write_policy,
                    &mut server.hyperlinks,
                )
            };
            match p
                .parser
                .parse_step(&mut sw, Some(&mut p.palette), &policy, bytes)
            {
                InputStep::Complete { consumed } => {
                    sw.finish();
                    (consumed, None)
                }
                InputStep::Effect { consumed, effect } => {
                    let effect = OwnedInputEffect::from(effect);
                    state = Some(sw.suspend());
                    (consumed, Some(effect))
                }
            }
        };
        host.end_draw(server, id);
        consumed_total += consumed;
        let Some(effect) = effect else {
            if external.is_none() {
                server
                    .panes
                    .get_mut(id)
                    .ok_or(ModelError::StaleId)?
                    .parser_offset += consumed_total as u64;
            }
            return Ok(consumed_total);
        };
        apply_effect(server, host, id, effect)?;
    }
}

fn make_request(
    server: &mut Server,
    id: PaneId,
    kind: RequestKind,
    client: Option<ClientId>,
    end: InputEnd,
    now_ms: u64,
) -> Result<RequestId, ModelError> {
    let sequence = server.next_input_request;
    server.next_input_request = sequence
        .checked_add(1)
        .ok_or(crate::ids::ArenaError::CapacityExhausted)?;
    let request_id = RequestId::from_parts(sequence as u32, (sequence >> 32) as u32);
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let first = p.input_state.requests.is_empty();
    p.input_state.requests.push_back(InputRequest {
        id: request_id,
        sequence,
        client,
        kind,
        end,
        created_ms: now_ms,
    });
    if first {
        p.input_state.request_timer = true;
        timer(
            server,
            id,
            InputTimer::Requests,
            Some(Duration::from_millis(100)),
        );
    }
    Ok(request_id)
}
fn queue_reply(
    server: &mut Server,
    id: PaneId,
    bytes: Vec<u8>,
    add: bool,
    now_ms: u64,
) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    if add && !p.input_state.requests.is_empty() {
        make_request(
            server,
            id,
            RequestKind::Queue(bytes),
            None,
            InputEnd::St,
            now_ms,
        )?;
    } else {
        p.output.extend_from_slice(&bytes);
    }
    Ok(())
}
fn add_request(
    server: &mut Server,
    host: &mut dyn PaneInputHost,
    id: PaneId,
    kind: InputRequestKind,
    end: InputEnd,
) -> Result<bool, ModelError> {
    let Some(client) = host.request_client(server, id) else {
        return Ok(false);
    };
    let request_kind = match kind {
        InputRequestKind::Palette { idx } => RequestKind::Palette { idx },
        InputRequestKind::Clipboard { clip } => RequestKind::Clipboard { clip },
    };
    let request = make_request(server, id, request_kind, Some(client), end, host.now_ms())?;
    host.send_request(server, id, request, client, kind);
    Ok(true)
}

pub fn apply_effect(
    server: &mut Server,
    host: &mut dyn PaneInputHost,
    id: PaneId,
    effect: OwnedInputEffect,
) -> Result<(), ModelError> {
    if !server
        .panes
        .get(id)
        .is_some_and(|p| !p.flags.contains(PaneFlags::DESTROYED))
    {
        return Ok(());
    }
    match &effect {
        OwnedInputEffect::Reply(bytes) => {
            queue_reply(server, id, bytes.clone(), true, host.now_ms())?
        }
        OwnedInputEffect::Request { kind, end } => {
            add_request(server, host, id, *kind, *end)?;
        }
        OwnedInputEffect::ClipboardQuery { clip, end } => {
            let state = clipboard_policy(server);
            if state == GetClipboard::Buffer {
                if let Some(buffer) = server.paste.top().and_then(|b| server.paste.get(b)) {
                    let mut reply = Vec::new();
                    input::reply::clipboard(&buffer.data, *clip, *end, &mut reply);
                    server
                        .panes
                        .get_mut(id)
                        .ok_or(ModelError::StaleId)?
                        .output
                        .extend_from_slice(&reply);
                }
            } else if matches!(state, GetClipboard::Request | GetClipboard::Both) {
                add_request(
                    server,
                    host,
                    id,
                    InputRequestKind::Clipboard { clip: *clip },
                    *end,
                )?;
            }
        }
        OwnedInputEffect::ClipboardReceived { .. } => {}
        OwnedInputEffect::ColourQuery { which, end } => {
            let colour = host.colour(server, id, *which);
            let mut reply = Vec::new();
            input::reply::colour(
                if *which == ColourQueryKind::Foreground {
                    10
                } else {
                    11
                },
                None,
                colour,
                *end,
                &mut reply,
            );
            queue_reply(server, id, reply, true, host.now_ms())?;
        }
        OwnedInputEffect::ThemeReport => {
            let theme = host.theme(server, id);
            let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
            p.input_state.last_theme = Some(theme);
            p.flags.remove(PaneFlags::THEMECHANGED);
            if let Some(bytes) = input::reply::theme(theme) {
                p.output.extend_from_slice(bytes);
            }
        }
        OwnedInputEffect::ThemeUpdatesEnabled => {
            server
                .panes
                .get_mut(id)
                .ok_or(ModelError::StaleId)?
                .input_state
                .theme_updates = true
        }
        OwnedInputEffect::ThemeUpdatesDisabled => {
            server
                .panes
                .get_mut(id)
                .ok_or(ModelError::StaleId)?
                .input_state
                .theme_updates = false
        }
        OwnedInputEffect::Rename(name) => {
            let window = server.panes.get(id).ok_or(ModelError::StaleId)?.window;
            let options = server
                .windows
                .get(window)
                .ok_or(ModelError::StaleId)?
                .options;
            if let Some(name) = name {
                server
                    .options
                    .set_number_value(options, b"automatic-rename", 0);
                window::window_set_name(server, window, name, true)?;
            } else {
                if server
                    .options
                    .get_only(options, b"automatic-rename")
                    .is_some_and(|entry| entry.monitor().is_some())
                    && server.option_monitor_removed.is_none()
                {
                    return Err(ModelError::message(
                        b"option monitor cleanup dispatcher is missing",
                    ));
                }
                if let Some(token) = server.options.prepare_removal(options, b"automatic-rename") {
                    if let Some(monitor) = token.monitor {
                        if let Some(callback) = server.option_monitor_removed {
                            callback(server, monitor);
                        }
                    }
                    server.options.finish_removal(token);
                }
                if option_number(server, options, b"automatic-rename", 1) == 0 {
                    window::window_set_name(server, window, b"", true)?;
                }
            }
            window::effect(server, WindowEffect::Borders(window));
            window::effect(server, WindowEffect::Status(window));
        }
        OwnedInputEffect::StyleChanged { theme } => {
            let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
            p.flags.insert(PaneFlags::STYLECHANGED);
            if *theme {
                p.flags.insert(PaneFlags::THEMECHANGED);
            }
        }
        OwnedInputEffect::SyncStart => {
            let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
            p.base.mode.insert(ScreenMode::SYNC);
            p.input_state.sync_timer = true;
            timer(server, id, InputTimer::Sync, Some(Duration::from_secs(1)));
        }
        OwnedInputEffect::SyncEnd => {
            host.stop_sync(server, id);
            if let Some(p) = server.panes.get_mut(id) {
                p.base.mode.remove(ScreenMode::SYNC);
                p.input_state.sync_timer = false;
            }
            timer(server, id, InputTimer::Sync, None);
        }
        OwnedInputEffect::GroundTimer(arm) => {
            server
                .panes
                .get_mut(id)
                .ok_or(ModelError::StaleId)?
                .input_state
                .ground_timer = *arm;
            timer(
                server,
                id,
                InputTimer::Ground,
                arm.then_some(Duration::from_secs(5)),
            );
        }
        OwnedInputEffect::Osc133(event) => {
            let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
            let event_name: &[u8] = match event {
                Osc133Event::Prompt => {
                    p.last_prompt_time = server.current_time.0;
                    b"pane-shell-prompt"
                }
                Osc133Event::CommandStarted => {
                    p.cmd_start_time = server.current_time.0;
                    p.cmd_end_time = 0;
                    p.cmd_status = -1;
                    p.flags.insert(PaneFlags::CMDRUNNING);
                    b"pane-command-started"
                }
                Osc133Event::CommandFinished { status } => {
                    p.cmd_end_time = server.current_time.0;
                    p.cmd_status = i32::from(*status);
                    p.flags.remove(PaneFlags::CMDRUNNING);
                    b"pane-command-finished"
                }
            };
            let window = p.window;
            server.emit(event_name, None, Some(window), Some(id));
        }
        OwnedInputEffect::AlternateChanged { entering } => {
            let window = server.panes.get(id).ok_or(ModelError::StaleId)?.window;
            if *entering {
                pane_clear_resizes(server, id, None)?;
            }
            crate::layout::fix_panes(server, window, None);
            let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
            if *entering && !p.resizes.is_empty() {
                let (sx, sy) = (p.sx, p.sy);
                pane_send_resize(server, id, sx, sy)?;
                pane_clear_resizes(server, id, None)?;
            }
            window::effect(server, WindowEffect::Borders(window));
            window::effect(server, WindowEffect::Redraw(window));
        }
        OwnedInputEffect::TitleChanged(_)
        | OwnedInputEffect::TitlePopped(_)
        | OwnedInputEffect::PathChanged
        | OwnedInputEffect::ProgressChanged => {
            let window = server.panes.get(id).ok_or(ModelError::StaleId)?.window;
            window::effect(server, WindowEffect::Borders(window));
            window::effect(server, WindowEffect::Status(window));
            if matches!(
                &effect,
                OwnedInputEffect::TitleChanged(_) | OwnedInputEffect::TitlePopped(_)
            ) {
                server.emit(b"pane-title-changed", None, Some(window), Some(id));
            }
        }
        OwnedInputEffect::Bell => {}
    }
    if server
        .panes
        .get(id)
        .is_some_and(|p| !p.flags.contains(PaneFlags::DESTROYED))
    {
        host.effect(server, id, &effect);
    }
    if let OwnedInputEffect::ClipboardReceived { data, .. } = &effect {
        let limit = option_number(server, server.options.global, b"buffer-limit", 50) as u32;
        super::paste::paste_add(server, None, data.clone(), limit)?;
    }
    Ok(())
}

fn cancel_request(server: &mut Server, pane: PaneId, request: InputRequest) {
    if let Some(client) = request.client {
        action(
            server,
            InputAction::CancelRequest {
                pane,
                request: request.id,
                client,
            },
        );
    }
}
pub fn cancel_pane_requests(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let requests = std::mem::take(&mut p.input_state.requests);
    p.input_state.request_timer = false;
    for request in requests {
        cancel_request(server, id, request);
    }
    timer(server, id, InputTimer::Requests, None);
    Ok(())
}

fn next_client_request(server: &Server, client: ClientId) -> Option<(PaneId, RequestId)> {
    let mut best = None;
    for pane in server.pane_ids.values() {
        let p = server.panes.get(*pane)?;
        for request in &p.input_state.requests {
            if request.client == Some(client)
                && best.is_none_or(|(_, _, sequence)| request.sequence < sequence)
            {
                best = Some((*pane, request.id, request.sequence));
            }
        }
    }
    best.map(|(pane, request, _)| (pane, request))
}
fn remove_request(server: &mut Server, pane: PaneId, request: RequestId) -> Option<InputRequest> {
    let p = server.panes.get_mut(pane)?;
    let position = p
        .input_state
        .requests
        .iter()
        .position(|r| r.id == request)?;
    p.input_state.requests.remove(position)
}
pub fn input_cancel_requests(server: &mut Server, client: ClientId) {
    while let Some((pane, request)) = next_client_request(server, client) {
        if let Some(request) = remove_request(server, pane, request) {
            cancel_request(server, pane, request);
        }
    }
}
pub fn input_request_reply(
    server: &mut Server,
    client: ClientId,
    reply: &InputReply,
) -> Result<(), ModelError> {
    let found = loop {
        let Some((pane, id)) = next_client_request(server, client) else {
            return Ok(());
        };
        let p = server.panes.get(pane).ok_or(ModelError::StaleId)?;
        let request = p
            .input_state
            .requests
            .iter()
            .find(|r| r.id == id)
            .ok_or(ModelError::StaleId)?;
        let matching = match (&request.kind, reply) {
            (RequestKind::Palette { idx }, InputReply::Palette(data)) => {
                i32::from(*idx) == data.idx
            }
            (RequestKind::Clipboard { .. }, InputReply::Clipboard(_)) => true,
            _ => false,
        };
        if matching {
            break (pane, id);
        }
        if let Some(request) = remove_request(server, pane, id) {
            cancel_request(server, pane, request);
        }
    };
    let (pane, found_id) = found;
    let mut complete = false;
    loop {
        let p = server.panes.get_mut(pane).ok_or(ModelError::StaleId)?;
        if p.input_state
            .requests
            .front()
            .is_none_or(|request| complete && !matches!(&request.kind, RequestKind::Queue(_)))
        {
            break;
        }
        let request = p.input_state.requests.pop_front().expect("request front");
        match &request.kind {
            RequestKind::Queue(bytes) => p.output.extend_from_slice(rmux_util::bytes::cstr(bytes)),
            _ if request.id == found_id => {
                match reply {
                    InputReply::Palette(data) => {
                        input::reply::colour(
                            4,
                            u8::try_from(data.idx).ok(),
                            data.c,
                            request.end,
                            &mut p.output,
                        );
                    }
                    InputReply::Clipboard(data) => {
                        let policy = clipboard_policy(server);
                        if matches!(policy, GetClipboard::Request | GetClipboard::Both) {
                            if policy == GetClipboard::Both {
                                let limit = option_number(
                                    server,
                                    server.options.global,
                                    b"buffer-limit",
                                    50,
                                ) as u32;
                                super::paste::paste_add(
                                    server,
                                    None,
                                    data.buf.as_bytes().to_vec(),
                                    limit,
                                )?;
                            }
                            let end = request.end;
                            input::reply::clipboard(
                                data.buf.as_bytes(),
                                data.clip,
                                end,
                                &mut server
                                    .panes
                                    .get_mut(pane)
                                    .ok_or(ModelError::StaleId)?
                                    .output,
                            );
                        }
                    }
                }
                complete = true;
            }
            _ => {}
        }
        cancel_request(server, pane, request);
    }
    Ok(())
}

pub fn input_request_timer(server: &mut Server, id: PaneId, now_ms: u64) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    p.input_state.request_timer = false;
    let mut at = 0;
    while at
        < server
            .panes
            .get(id)
            .ok_or(ModelError::StaleId)?
            .input_state
            .requests
            .len()
    {
        let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
        if p.input_state.requests[at].created_ms >= now_ms.wrapping_sub(500) {
            at += 1;
            continue;
        }
        let request = p.input_state.requests.remove(at).expect("request index");
        if let RequestKind::Queue(bytes) = &request.kind {
            p.output.extend_from_slice(rmux_util::bytes::cstr(bytes));
        }
        cancel_request(server, id, request);
    }
    if !server
        .panes
        .get(id)
        .ok_or(ModelError::StaleId)?
        .input_state
        .requests
        .is_empty()
    {
        server
            .panes
            .get_mut(id)
            .ok_or(ModelError::StaleId)?
            .input_state
            .request_timer = true;
        timer(
            server,
            id,
            InputTimer::Requests,
            Some(Duration::from_millis(100)),
        );
    }
    Ok(())
}

pub fn input_ground_timer(server: &mut Server, id: PaneId) {
    if let Some(p) = server
        .panes
        .get_mut(id)
        .filter(|p| p.input_state.ground_timer)
    {
        p.parser.ground_timeout();
        p.input_state.ground_timer = false;
    }
}

pub fn input_sync_timer(server: &mut Server, host: &mut dyn PaneInputHost, id: PaneId) {
    if !server
        .panes
        .get(id)
        .is_some_and(|p| p.input_state.sync_timer)
    {
        return;
    }
    if let Some(p) = server.panes.get_mut(id) {
        p.input_state.sync_timer = false;
        p.base.mode.remove(ScreenMode::SYNC);
    }
    timer(server, id, InputTimer::Sync, None);
    host.stop_sync(server, id);
}

#[cfg(test)]
mod tests {
    use super::super::spawn::SpawnFlags;
    use super::*;
    fn pane(s: &mut Server) -> PaneId {
        let w = window::window_create(s, 20, 4, 0, 0).unwrap();
        window::window_add_pane(s, w, None, 10, SpawnFlags::default()).unwrap()
    }
    #[test]
    fn offsets_clamp_drain_and_reset() {
        let mut s = Server::default();
        let id = pane(&mut s);
        s.panes
            .get_mut(id)
            .unwrap()
            .input
            .extend_from_slice(b"abcdef");
        let mut offset = PaneOffset::default();
        pane_update_used_data(&s, id, &mut offset, 99).unwrap();
        assert_eq!(offset.used, 6);
        s.panes.get_mut(id).unwrap().parser_offset = 4;
        pane_drain_input(&mut s, id, 3).unwrap();
        assert_eq!(s.panes.get(id).unwrap().input, b"def");
        assert_eq!(pane_get_new_data(&s, id, &offset).unwrap(), b"");
        pane_reset_io(&mut s, id).unwrap();
        assert_eq!(s.panes.get(id).unwrap().base_offset, 0);
        assert_eq!(s.panes.get(id).unwrap().parser_offset, 0);
    }
    #[test]
    fn fifo_reply_flushes_queues_only_through_match() {
        let mut s = Server::default();
        let id = pane(&mut s);
        let client = ClientId::from_parts(1, 0);
        make_request(
            &mut s,
            id,
            RequestKind::Palette { idx: 1 },
            Some(client),
            InputEnd::Bel,
            100,
        )
        .unwrap();
        queue_reply(&mut s, id, b"first".to_vec(), true, 100).unwrap();
        make_request(
            &mut s,
            id,
            RequestKind::Palette { idx: 2 },
            Some(client),
            InputEnd::St,
            100,
        )
        .unwrap();
        queue_reply(&mut s, id, b"second".to_vec(), true, 100).unwrap();
        input_request_reply(
            &mut s,
            client,
            &InputReply::Palette(input::InputRequestPaletteData {
                idx: 1,
                c: Colour::NONE,
            }),
        )
        .unwrap();
        assert_eq!(s.panes.get(id).unwrap().output, b"first");
        assert_eq!(s.panes.get(id).unwrap().input_state.requests.len(), 2);
        input_request_reply(
            &mut s,
            client,
            &InputReply::Palette(input::InputRequestPaletteData {
                idx: 2,
                c: Colour::NONE,
            }),
        )
        .unwrap();
        assert_eq!(s.panes.get(id).unwrap().output, b"firstsecond");
        assert!(s.panes.get(id).unwrap().input_state.requests.is_empty());
    }
    #[test]
    fn timeout_boundary_and_client_cancel_leave_queued_reply() {
        let mut s = Server::default();
        let id = pane(&mut s);
        let client = ClientId::from_parts(1, 0);
        make_request(
            &mut s,
            id,
            RequestKind::Clipboard { clip: 0 },
            Some(client),
            InputEnd::St,
            100,
        )
        .unwrap();
        queue_reply(&mut s, id, b"reply".to_vec(), true, 100).unwrap();
        input_request_timer(&mut s, id, 600).unwrap();
        assert_eq!(s.panes.get(id).unwrap().input_state.requests.len(), 2);
        input_cancel_requests(&mut s, client);
        assert_eq!(s.panes.get(id).unwrap().input_state.requests.len(), 1);
        input_request_timer(&mut s, id, 601).unwrap();
        assert_eq!(s.panes.get(id).unwrap().output, b"reply");
    }
    #[test]
    fn most_active_client_first_on_tie() {
        let a = ClientId::from_parts(0, 0);
        let b = ClientId::from_parts(1, 0);
        let view = |id, activity| InputClient {
            id,
            attached: true,
            tty_started: true,
            activity,
            has_window: true,
        };
        assert_eq!(
            select_request_client([view(a, (5, 0)), view(b, (5, 0))]),
            Some(a)
        );
        assert_eq!(
            select_request_client([view(a, (5, 0)), view(b, (6, 0))]),
            Some(b)
        );
    }
    struct Host {
        sink: rmux_emu::screen::write::ScreenOnlySink,
        events: Vec<&'static str>,
        destroy_on_title: bool,
        alternate_clean: bool,
    }
    impl Default for Host {
        fn default() -> Self {
            Self {
                sink: rmux_emu::screen::write::ScreenOnlySink,
                events: Vec::new(),
                destroy_on_title: false,
                alternate_clean: false,
            }
        }
    }
    impl PaneInputHost for Host {
        fn tty_sink(&mut self) -> &mut dyn TtySink {
            &mut self.sink
        }
        fn write_policy(&self, _: &Server, _: PaneId) -> ScreenWritePolicy {
            ScreenWritePolicy {
                pane_backed: true,
                ..ScreenWritePolicy::default()
            }
        }
        fn now_ms(&self) -> u64 {
            1000
        }
        fn request_client(&self, _: &Server, _: PaneId) -> Option<ClientId> {
            None
        }
        fn send_request(
            &mut self,
            _: &mut Server,
            _: PaneId,
            _: RequestId,
            _: ClientId,
            _: InputRequestKind,
        ) {
            self.events.push("request");
        }
        fn colour(&self, _: &Server, _: PaneId, _: ColourQueryKind) -> Colour {
            Colour::NONE
        }
        fn theme(&self, _: &Server, _: PaneId) -> ClientTheme {
            ClientTheme::Unknown
        }
        fn effect(&mut self, server: &mut Server, pane: PaneId, effect: &OwnedInputEffect) {
            if matches!(
                effect,
                OwnedInputEffect::AlternateChanged { entering: true }
            ) {
                self.alternate_clean = server.panes.get(pane).unwrap().resizes.is_empty();
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
                    b" "
                );
            }
            if matches!(effect, OwnedInputEffect::TitleChanged(_)) && self.destroy_on_title {
                super::super::pane::pane_destroy(server, pane).unwrap();
            }
        }
        fn pipe_output(&mut self, _: &mut Server, _: PaneId) {
            self.events.push("pipe");
        }
        fn control_output(&mut self, _: &mut Server, _: PaneId) {
            self.events.push("control");
        }
        fn disable_reads(&mut self, _: &mut Server, _: PaneId) {
            self.events.push("disable");
        }
        fn stop_sync(&mut self, _: &mut Server, _: PaneId) {
            self.events.push("sync-stop");
        }
    }
    #[test]
    fn alternate_repairs_before_remaining_input_and_orders_consumers() {
        let mut s = Server::default();
        let id = pane(&mut s);
        let w = s.panes.get(id).unwrap().window;
        crate::layout::init(&mut s, w, id);
        super::super::pane::pane_resize(&mut s, id, 19, 4).unwrap();
        let mut host = Host::default();
        pane_read(&mut s, &mut host, id, b"\x1b[?1049hX").unwrap();
        assert!(host.alternate_clean);
        assert_eq!(host.events, ["pipe", "control", "disable"]);
        assert_eq!(
            s.panes
                .get(id)
                .unwrap()
                .base
                .grid
                .view_string_cells(0, 0, 1),
            b"X"
        );
        assert_eq!(s.panes.get(id).unwrap().parser_offset, 9);
    }
    #[test]
    fn callback_destroy_does_not_resume_parser_or_reuse_stale_timer() {
        let mut s = Server::default();
        let id = pane(&mut s);
        let mut host = Host {
            destroy_on_title: true,
            ..Host::default()
        };
        let used = pane_parse_buffer(&mut s, &mut host, id, b"\x1b]2;title\x07remaining").unwrap();
        assert_eq!(used, 10);
        assert!(s.panes.get(id).is_none());
        input_ground_timer(&mut s, id);
        let replacement = pane(&mut s);
        assert_ne!(id, replacement);
        input_ground_timer(&mut s, id);
        assert!(s.panes.get(replacement).is_some());
    }
    #[test]
    fn ground_timeout_keeps_pending_bytes_and_partial_parser_input() {
        let mut s = Server::default();
        let id = pane(&mut s);
        let mut host = Host::default();
        pane_parse_buffer(&mut s, &mut host, id, b"\x1b]2;unfinished").unwrap();
        let pending = s.panes.get(id).unwrap().parser.pending().to_vec();
        input_ground_timer(&mut s, id);
        assert_eq!(s.panes.get(id).unwrap().parser.pending(), pending);
        assert_eq!(s.panes.get(id).unwrap().parser.state_name(), "ground");
    }
    #[test]
    fn stdin_chunks_drain_and_missing_pane_finishes_once_after_cancel() {
        let mut s = Server::default();
        let id = pane(&mut s);
        let mut host = Host::default();
        let mut stdin = PaneStdinInput::new(
            id,
            ClientId::from_parts(1, 0),
            crate::ids::QueueItemId::from_parts(1, 0),
        );
        let mut buffer = b"X".to_vec();
        pane_stdin_input(
            &mut s,
            &mut host,
            &mut stdin,
            &mut buffer,
            StdinReadState {
                client_dead: false,
                file_present: true,
                closed: false,
                error: false,
            },
        )
        .unwrap();
        assert!(buffer.is_empty());
        assert_eq!(
            s.panes
                .get(id)
                .unwrap()
                .base
                .grid
                .view_string_cells(0, 0, 1),
            b"X"
        );
        super::super::pane::pane_destroy(&mut s, id).unwrap();
        pane_stdin_input(
            &mut s,
            &mut host,
            &mut stdin,
            &mut buffer,
            StdinReadState {
                client_dead: false,
                file_present: true,
                closed: false,
                error: false,
            },
        )
        .unwrap();
        pane_stdin_input(
            &mut s,
            &mut host,
            &mut stdin,
            &mut buffer,
            StdinReadState {
                client_dead: false,
                file_present: false,
                closed: true,
                error: false,
            },
        )
        .unwrap();
        pane_stdin_input(
            &mut s,
            &mut host,
            &mut stdin,
            &mut buffer,
            StdinReadState {
                client_dead: false,
                file_present: false,
                closed: true,
                error: false,
            },
        )
        .unwrap();
        assert_eq!(
            s.effects
                .iter()
                .filter(|e| matches!(
                    e,
                    ModelEffect::Input(InputAction::StdinCancel {
                        exit_status: Some(1),
                        ..
                    })
                ))
                .count(),
            1
        );
        assert_eq!(
            s.effects
                .iter()
                .filter(|e| matches!(e, ModelEffect::Input(InputAction::StdinFinished { .. })))
                .count(),
            1
        );
    }
}
