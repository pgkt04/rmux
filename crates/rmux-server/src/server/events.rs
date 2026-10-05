// Ported from tmux events.c, events-payload.c, tmux.h @ 8f25579c
/*
 * Copyright (c) 2026 Nicholas Marriott <nicholas.marriott@gmail.com>
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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EventPayloadType {
    Pointer = 8,
    String = 0,
    Time = 1,
    Int = 2,
    Uint = 3,
    Client = 4,
    Session = 5,
    Window = 6,
    Pane = 7,
}
impl TryFrom<i32> for EventPayloadType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            8 => Ok(Self::Pointer),
            0 => Ok(Self::String),
            1 => Ok(Self::Time),
            2 => Ok(Self::Int),
            3 => Ok(Self::Uint),
            4 => Ok(Self::Client),
            5 => Ok(Self::Session),
            6 => Ok(Self::Window),
            7 => Ok(Self::Pane),
            _ => Err(value),
        }
    }
}

use crate::cmd::find::{self, CmdFindFlags, CmdFindState, ModelView};
use crate::cmd::hooks::HooksMonitorId;
use crate::format::FormatTree;
use crate::ids::{
    Arena, ClientId, EventSinkId, PaneId, QueueItemId, SessionId, WindowId, WinlinkId,
};
use crate::model::{Server, pane, session, window};
use rmux_util::bytes::{ByteString, cstr};
use std::collections::BTreeMap;
use std::io::Write;

pub type EventCallback = fn(&mut Server, &mut EventPayload);
pub type EventMonitorCallback = fn(&mut Server, &mut EventPayload, HooksMonitorId);
pub type EventSinkIdCallback = fn(&mut Server, &mut EventPayload, EventSinkId);

#[derive(Clone, Copy)]
pub enum EventSinkCallback {
    Event(EventCallback),
    Monitor(HooksMonitorId, EventMonitorCallback),
    Sink(EventSinkIdCallback),
}

pub struct EventSink {
    pub name: ByteString,
    pub callback: EventSinkCallback,
    pub dead: bool,
    pub generation: u64,
    next: Option<EventSinkId>,
}

#[derive(Default)]
pub struct Events {
    pub sinks: Arena<EventSink, EventSinkId>,
    head: Option<EventSinkId>,
    tail: Option<EventSinkId>,
    generation: u64,
    dispatching: u32,
}

impl Events {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_sink(&mut self, name: &[u8], callback: EventCallback) -> EventSinkId {
        self.add_callback(name, EventSinkCallback::Event(callback))
    }

    pub fn add_sink_with_id(&mut self, name: &[u8], callback: EventSinkIdCallback) -> EventSinkId {
        self.add_callback(name, EventSinkCallback::Sink(callback))
    }

    pub fn add_monitor_sink(
        &mut self,
        name: &[u8],
        monitor: HooksMonitorId,
        callback: EventMonitorCallback,
    ) -> EventSinkId {
        self.add_callback(name, EventSinkCallback::Monitor(monitor, callback))
    }

    fn add_callback(&mut self, name: &[u8], callback: EventSinkCallback) -> EventSinkId {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("event generation overflow");
        let id = self
            .sinks
            .insert(EventSink {
                name: cstr(name).into(),
                callback,
                dead: false,
                generation: self.generation,
                next: None,
            })
            .expect("event sink arena");
        if let Some(tail) = self.tail {
            self.sinks.get_mut(tail).expect("event tail").next = Some(id);
        } else {
            self.head = Some(id);
        }
        self.tail = Some(id);
        id
    }

    pub fn remove_sink(&mut self, id: EventSinkId) {
        if let Some(sink) = self.sinks.get_mut(id) {
            sink.dead = true;
            if self.dispatching == 0 {
                self.free_dead();
            }
        }
    }

    fn free_dead(&mut self) {
        let mut previous = None;
        let mut cursor = self.head;
        while let Some(id) = cursor {
            let sink = self.sinks.get(id).expect("event sink list");
            cursor = sink.next;
            if sink.dead {
                if let Some(previous) = previous {
                    self.sinks
                        .get_mut(previous)
                        .expect("event predecessor")
                        .next = cursor;
                } else {
                    self.head = cursor;
                }
                if self.tail == Some(id) {
                    self.tail = previous;
                }
                self.sinks.request_remove(id).expect("event sink removal");
            } else {
                previous = Some(id);
            }
        }
    }
}

pub fn add_sink(server: &mut Server, name: &[u8], callback: EventCallback) -> EventSinkId {
    server.events.add_sink(name, callback)
}

pub fn add_sink_with_id(
    server: &mut Server,
    name: &[u8],
    callback: EventSinkIdCallback,
) -> EventSinkId {
    server.events.add_sink_with_id(name, callback)
}

pub fn add_monitor_sink(
    server: &mut Server,
    name: &[u8],
    monitor: HooksMonitorId,
    callback: EventMonitorCallback,
) -> EventSinkId {
    server.events.add_monitor_sink(name, monitor, callback)
}

pub fn remove_sink(server: &mut Server, id: EventSinkId) {
    server.events.remove_sink(id);
}

pub fn fire(server: &mut Server, name: &[u8], mut payload: EventPayload) {
    let generation = server.events.generation;
    payload.set_string(server, b"event", name);
    if rmux_util::log::level().0 != 0 {
        payload.log(
            server,
            &format!("events_fire: {}: ", String::from_utf8_lossy(cstr(name))),
        );
    }
    server.events.dispatching = server
        .events
        .dispatching
        .checked_add(1)
        .expect("event depth overflow");
    let mut cursor = server.events.head;
    while let Some(id) = cursor {
        let sink = server.events.sinks.get(id).expect("dispatch sink");
        let callback =
            (!sink.dead && sink.generation <= generation && sink.name.as_bytes() == cstr(name))
                .then_some(sink.callback);
        if let Some(callback) = callback {
            match callback {
                EventSinkCallback::Event(callback) => callback(server, &mut payload),
                EventSinkCallback::Monitor(monitor, callback) => {
                    callback(server, &mut payload, monitor)
                }
                EventSinkCallback::Sink(callback) => callback(server, &mut payload, id),
            }
        }
        cursor = server
            .events
            .sinks
            .get(id)
            .expect("retained dispatch sink")
            .next;
    }
    server.events.dispatching -= 1;
    if server.events.dispatching == 0 {
        server.events.free_dead();
    }
    payload.free(server);
}

pub trait EventPrintable {
    fn print(&self, output: &mut Vec<u8>);
}

impl EventPrintable for ByteString {
    fn print(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(self.as_bytes());
    }
}

pub enum EventExtension {
    QueueItem(QueueItemId),
    HookMonitor(HooksMonitorId),
    Printable(Box<dyn EventPrintable>),
}

enum EventValue {
    String(ByteString),
    Time(i64),
    Int(i32),
    Uint(u32),
    Client(ClientId),
    Session(SessionId),
    Window(WindowId),
    Pane(PaneId),
    Extension(EventExtension),
}

pub struct EventPayloadItem {
    name: ByteString,
    value: EventValue,
}

impl EventPayloadItem {
    pub fn name(&self) -> &[u8] {
        self.name.as_bytes()
    }

    pub fn item_type(&self) -> EventPayloadType {
        match self.value {
            EventValue::String(_) => EventPayloadType::String,
            EventValue::Time(_) => EventPayloadType::Time,
            EventValue::Int(_) => EventPayloadType::Int,
            EventValue::Uint(_) => EventPayloadType::Uint,
            EventValue::Client(_) => EventPayloadType::Client,
            EventValue::Session(_) => EventPayloadType::Session,
            EventValue::Window(_) => EventPayloadType::Window,
            EventValue::Pane(_) => EventPayloadType::Pane,
            EventValue::Extension(_) => EventPayloadType::Pointer,
        }
    }

    fn add_printed(&self, server: &Server, output: &mut Vec<u8>) {
        match &self.value {
            EventValue::String(value) => output.extend_from_slice(value.as_bytes()),
            EventValue::Time(value) => write!(output, "{value}").expect("payload buffer"),
            EventValue::Int(value) => write!(output, "{value}").expect("payload buffer"),
            EventValue::Uint(value) => write!(output, "{value}").expect("payload buffer"),
            EventValue::Client(id) => {
                if let Some(name) = server.clients.get(*id).and_then(|c| c.name.as_deref()) {
                    output.extend_from_slice(cstr(name));
                }
            }
            EventValue::Session(id) => {
                let value = server
                    .sessions
                    .get(*id)
                    .expect("payload session lease")
                    .public_id;
                write!(output, "${value}").expect("payload buffer");
            }
            EventValue::Window(id) => {
                let value = server
                    .windows
                    .get(*id)
                    .expect("payload window lease")
                    .public_id;
                write!(output, "@{value}").expect("payload buffer");
            }
            EventValue::Pane(id) => {
                let value = server.panes.get(*id).expect("payload pane lease").public_id;
                write!(output, "%{value}").expect("payload buffer");
            }
            EventValue::Extension(EventExtension::Printable(value)) => value.print(output),
            EventValue::Extension(EventExtension::QueueItem(id)) => {
                write!(output, "queue:{id:?}").expect("payload buffer");
            }
            EventValue::Extension(EventExtension::HookMonitor(id)) => {
                write!(output, "monitor:{id:?}").expect("payload buffer");
            }
        }
    }

    pub fn print(&self, server: &Server) -> ByteString {
        let mut output = Vec::new();
        self.add_printed(server, &mut output);
        output.into()
    }
}

fn release_value(server: &mut Server, value: EventValue) {
    match value {
        EventValue::Client(id) => {
            crate::client::lifecycle::release(server, id).expect("payload client lease")
        }
        EventValue::Session(id) => {
            assert!(session::session_release(server, id));
        }
        EventValue::Window(id) => window::window_release(server, id).expect("payload window lease"),
        EventValue::Pane(id) => pane::pane_release(server, id).expect("payload pane lease"),
        _ => {}
    }
}

#[must_use = "payload object leases must be released with free or consumed by fire"]
#[derive(Default)]
pub struct EventPayload {
    items: BTreeMap<ByteString, EventPayloadItem>,
    target: CmdFindState,
}

impl EventPayload {
    pub fn new() -> Self {
        Self::default()
    }

    fn set_item(&mut self, server: &mut Server, name: &[u8], value: EventValue) {
        let name = cstr(name);
        if let Some(old) = self.items.remove(name) {
            release_value(server, old.value);
            self.items.insert(
                old.name.clone(),
                EventPayloadItem {
                    name: old.name,
                    value,
                },
            );
        } else {
            self.items.insert(
                name.into(),
                EventPayloadItem {
                    name: name.into(),
                    value,
                },
            );
        }
    }

    fn free_target(&mut self, server: &mut Server) {
        let target = std::mem::take(&mut self.target);
        if let Some(id) = target.s {
            assert!(session::session_release(server, id));
        }
        if let Some(id) = target.w {
            window::window_release(server, id).expect("payload target window lease");
        }
        if let Some(id) = target.wp {
            pane::pane_release(server, id).expect("payload target pane lease");
        }
    }

    pub fn free(mut self, server: &mut Server) {
        for (_, item) in std::mem::take(&mut self.items) {
            release_value(server, item.value);
        }
        self.free_target(server);
    }

    pub fn set_target(&mut self, server: &mut Server, state: &CmdFindState) {
        self.free_target(server);
        let link = state.wl.and_then(|id| server.winlinks.get(id));
        let s = state.s.or_else(|| link.map(|l| l.session));
        let w = state.w.or_else(|| link.map(|l| l.window));
        let idx = link.map_or(-1, |l| l.index);
        if let Some(id) = s {
            assert!(
                session::session_retain(server, id),
                "payload target session"
            );
            self.target.s = Some(id);
        }
        if let Some(id) = w {
            window::window_retain(server, id).expect("payload target window");
            self.target.w = Some(id);
        }
        if let Some(id) = state.wp {
            pane::pane_retain(server, id).expect("payload target pane");
            self.target.wp = Some(id);
        }
        self.target.idx = idx;
    }

    pub fn get_target(&self, server: &mut Server, flags: CmdFindFlags) -> CmdFindState {
        recover_target(server, &self.target, flags)
    }

    pub fn set_string(&mut self, server: &mut Server, name: &[u8], value: &[u8]) {
        self.set_item(server, name, EventValue::String(cstr(value).into()));
    }

    pub fn set_time(&mut self, server: &mut Server, name: &[u8], value: i64) {
        self.set_item(server, name, EventValue::Time(value));
    }

    pub fn set_int(&mut self, server: &mut Server, name: &[u8], value: i32) {
        self.set_item(server, name, EventValue::Int(value));
    }

    pub fn set_uint(&mut self, server: &mut Server, name: &[u8], value: u32) {
        self.set_item(server, name, EventValue::Uint(value));
    }

    pub fn set_client(&mut self, server: &mut Server, name: &[u8], value: ClientId) {
        crate::client::lifecycle::retain(server, value).expect("payload client");
        self.set_item(server, name, EventValue::Client(value));
    }

    pub fn set_session(&mut self, server: &mut Server, name: &[u8], value: SessionId) {
        assert!(session::session_retain(server, value), "payload session");
        self.set_item(server, name, EventValue::Session(value));
    }

    pub fn set_window(&mut self, server: &mut Server, name: &[u8], value: WindowId) {
        window::window_retain(server, value).expect("payload window");
        self.set_item(server, name, EventValue::Window(value));
    }

    pub fn set_pane(&mut self, server: &mut Server, name: &[u8], value: PaneId) {
        pane::pane_retain(server, value).expect("payload pane");
        self.set_item(server, name, EventValue::Pane(value));
    }

    pub fn set_extension(&mut self, server: &mut Server, name: &[u8], value: EventExtension) {
        self.set_item(server, name, EventValue::Extension(value));
    }

    pub fn set_queue_item(&mut self, server: &mut Server, value: QueueItemId) {
        self.set_extension(server, b"_cmdq_item", EventExtension::QueueItem(value));
    }

    pub fn set_hooks_monitor(&mut self, server: &mut Server, value: HooksMonitorId) {
        self.set_extension(
            server,
            b"_hooks_monitor",
            EventExtension::HookMonitor(value),
        );
    }

    pub fn get_string(&self, name: &[u8]) -> Option<&[u8]> {
        match &self.items.get(cstr(name))?.value {
            EventValue::String(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    pub fn get_time(&self, name: &[u8]) -> i64 {
        match self.items.get(cstr(name)).map(|i| &i.value) {
            Some(EventValue::Time(value)) => *value,
            _ => 0,
        }
    }

    pub fn get_int(&self, name: &[u8]) -> Option<i32> {
        match self.items.get(cstr(name))?.value {
            EventValue::Int(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_uint(&self, name: &[u8]) -> Option<u32> {
        match self.items.get(cstr(name))?.value {
            EventValue::Uint(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_client(&self, name: &[u8]) -> Option<ClientId> {
        match self.items.get(cstr(name))?.value {
            EventValue::Client(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_session(&self, name: &[u8]) -> Option<SessionId> {
        match self.items.get(cstr(name))?.value {
            EventValue::Session(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_window(&self, name: &[u8]) -> Option<WindowId> {
        match self.items.get(cstr(name))?.value {
            EventValue::Window(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_pane(&self, name: &[u8]) -> Option<PaneId> {
        match self.items.get(cstr(name))?.value {
            EventValue::Pane(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_extension(&self, name: &[u8]) -> Option<&EventExtension> {
        match &self.items.get(cstr(name))?.value {
            EventValue::Extension(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_queue_item(&self) -> Option<QueueItemId> {
        match self.get_extension(b"_cmdq_item")? {
            EventExtension::QueueItem(value) => Some(*value),
            _ => None,
        }
    }

    pub fn get_hooks_monitor(&self) -> Option<HooksMonitorId> {
        match self.get_extension(b"_hooks_monitor")? {
            EventExtension::HookMonitor(value) => Some(*value),
            _ => None,
        }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &EventPayloadItem> {
        self.items.values()
    }

    pub fn print(&self, server: &Server, name: &[u8]) -> Option<ByteString> {
        Some(self.items.get(cstr(name))?.print(server))
    }

    pub fn export_formats(
        &self,
        server: &Server,
        prefix: &[u8],
        mut add: impl FnMut(&[u8], ByteString),
    ) {
        let prefix = cstr(prefix);
        let mut key = Vec::new();
        for item in self.iter() {
            if item.name.first() == Some(&b'_') {
                continue;
            }
            key.clear();
            key.extend_from_slice(prefix);
            key.extend_from_slice(item.name());
            add(&key, item.print(server));
            let name = match item.value {
                EventValue::Session(id) => {
                    Some(&server.sessions.get(id).expect("payload session lease").name)
                }
                EventValue::Window(id) => {
                    Some(&server.windows.get(id).expect("payload window lease").name)
                }
                _ => None,
            };
            if let Some(name) = name {
                key.extend_from_slice(b"_name");
                add(&key, cstr(name).into());
            }
        }
    }

    pub fn add_formats(&self, server: &Server, tree: &mut FormatTree, prefix: &[u8]) {
        self.export_formats(server, prefix, |key, value| tree.add(key, value));
    }

    pub fn log_text(&self, server: &Server) -> ByteString {
        let mut output = Vec::new();
        for (index, item) in self.iter().enumerate() {
            if index != 0 {
                output.extend_from_slice(b", ");
            }
            output.extend_from_slice(item.name());
            output.push(b'=');
            item.add_printed(server, &mut output);
        }
        output.into()
    }

    pub fn log(&self, server: &Server, prefix: &str) {
        let text = self.log_text(server);
        rmux_util::log::write_bytes(prefix, text.as_bytes());
    }
}

fn recover_target(
    model: &dyn ModelView,
    saved: &CmdFindState,
    flags: CmdFindFlags,
) -> CmdFindState {
    let alive = saved
        .s
        .is_some_and(|s| model.session(s).is_some_and(|v| v.alive));
    let link = if saved.idx != -1 && alive {
        saved.s.and_then(|s| model.session(s)).and_then(|s| {
            s.winlinks.iter().copied().find(|id| {
                model
                    .winlink(*id)
                    .is_some_and(|l| l.index == saved.idx && Some(l.window) == saved.w)
            })
        })
    } else {
        None
    };
    let assembled = CmdFindState {
        flags,
        s: saved.s,
        w: saved.w,
        wp: saved.wp,
        wl: link,
        idx: link
            .and_then(|id| model.winlink(id))
            .map_or(-1, |l| l.index),
    };
    if assembled.is_valid(model) {
        return assembled;
    }
    if let (Some(link), Some(pane)) = (link, saved.wp)
        && model
            .winlink(link)
            .and_then(|l| model.window(l.window))
            .is_some_and(|w| w.panes.contains(&pane))
    {
        let state = find::from_winlink_pane(model, link, pane, flags);
        if state.is_valid(model) {
            return state;
        }
    }
    if let Some(state) = saved.wp.and_then(|p| find::from_pane(model, p, flags))
        && state.is_valid(model)
    {
        return state;
    }
    if let Some(link) = link {
        let state = find::from_winlink(model, link, flags);
        if state.is_valid(model) {
            return state;
        }
    }
    if alive {
        if let (Some(s), Some(w)) = (saved.s, saved.w)
            && let Some(state) = find::from_session_window(model, s, w, flags)
            && state.is_valid(model)
        {
            return state;
        }
        if let Some(s) = saved.s {
            let state = find::from_session(model, s, flags);
            if state.is_valid(model) {
                return state;
            }
        }
    }
    find::from_nothing(model, flags).unwrap_or_else(|| CmdFindState::clear(flags))
}

pub fn fire_client(server: &mut Server, name: &[u8], client: ClientId) {
    let state =
        find::from_client(server, Some(client), CmdFindFlags::default()).unwrap_or_default();
    let mut payload = EventPayload::new();
    payload.set_target(server, &state);
    payload.set_client(server, b"client", client);
    if let Some(id) = state.s {
        payload.set_session(server, b"session", id);
    }
    if let Some(id) = state.w {
        payload.set_window(server, b"window", id);
    }
    let index = state
        .wl
        .and_then(|id| server.winlinks.get(id))
        .map_or(state.idx, |l| l.index);
    if index != -1 {
        payload.set_int(server, b"window_index", index);
    }
    if let Some(id) = state.wp {
        payload.set_pane(server, b"pane", id);
    }
    fire(server, name, payload);
}

pub fn fire_session(server: &mut Server, name: &[u8], id: SessionId) {
    let mut payload = EventPayload::new();
    if session::session_alive(server, id) {
        let state = find::from_session(server, id, CmdFindFlags::default());
        payload.set_target(server, &state);
    }
    payload.set_session(server, b"session", id);
    fire(server, name, payload);
}

pub fn fire_window(server: &mut Server, name: &[u8], id: WindowId) {
    let state = find::from_window(server, id, CmdFindFlags::default()).unwrap_or_default();
    let mut payload = EventPayload::new();
    payload.set_target(server, &state);
    payload.set_window(server, b"window", id);
    fire(server, name, payload);
}

pub fn fire_pane(server: &mut Server, name: &[u8], id: PaneId) {
    let state = find::from_pane(server, id, CmdFindFlags::default()).unwrap_or_default();
    let window = server.panes.get(id).expect("event pane").window;
    let mut payload = EventPayload::new();
    payload.set_target(server, &state);
    payload.set_pane(server, b"pane", id);
    payload.set_window(server, b"window", window);
    fire(server, name, payload);
}

pub fn fire_winlink(server: &mut Server, name: &[u8], id: WinlinkId) {
    let state = find::from_winlink(server, id, CmdFindFlags::default());
    let link = server.winlinks.get(id).expect("event winlink");
    let (session, window, index) = (link.session, link.window, link.index);
    let mut payload = EventPayload::new();
    payload.set_target(server, &state);
    payload.set_session(server, b"session", session);
    payload.set_window(server, b"window", window);
    payload.set_int(server, b"window_index", index);
    fire(server, name, payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;
    use crate::model::{spawn::SpawnFlags, winlink};
    use crate::options::environment::Environment;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn graph(server: &mut Server, name: &[u8], index: i32) -> CmdFindState {
        let options = server.options.create(Some(server.options.global_s));
        let s = session::session_create(
            server,
            session::SessionCreate {
                prefix: None,
                name: Some(name.to_vec()),
                cwd: b"/".to_vec(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        );
        let w = window::window_create(server, 80, 24, 0, 0).unwrap();
        let wp = window::window_add_pane(server, w, None, 0, SpawnFlags::default()).unwrap();
        window::window_set_active_pane(server, w, wp, false).unwrap();
        let wl = session::session_attach(server, s, w, index).unwrap();
        session::session_set_current(server, s, Some(wl));
        find::from_winlink_pane(server, wl, wp, CmdFindFlags::default())
    }

    #[test]
    fn discriminants_are_pinned() {
        for (number, kind) in [
            EventPayloadType::String,
            EventPayloadType::Time,
            EventPayloadType::Int,
            EventPayloadType::Uint,
            EventPayloadType::Client,
            EventPayloadType::Session,
            EventPayloadType::Window,
            EventPayloadType::Pane,
            EventPayloadType::Pointer,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(kind as i32, number as i32);
            assert_eq!(EventPayloadType::try_from(number as i32), Ok(kind));
        }
        assert_eq!(EventPayloadType::try_from(9), Err(9));
    }

    #[test]
    fn typed_getters_replacement_sorted_iteration_and_decimal_print() {
        let mut server = Server::new();
        let mut payload = EventPayload::new();
        payload.set_uint(&mut server, b"z", u32::MAX);
        payload.set_time(&mut server, b"time", -123456789);
        payload.set_int(&mut server, b"int", i32::MIN);
        payload.set_string(&mut server, b"a\0ignored", b"bytes\xff\0ignored");
        assert_eq!(payload.get_string(b"a"), Some(b"bytes\xff".as_slice()));
        assert_eq!(payload.get_int(b"int"), Some(i32::MIN));
        assert_eq!(payload.get_uint(b"z"), Some(u32::MAX));
        assert_eq!(payload.get_uint(b"int"), None);
        assert_eq!(payload.get_int(b"z"), None);
        assert_eq!(payload.get_time(b"missing"), 0);
        assert_eq!(payload.get_time(b"int"), 0);
        assert_eq!(payload.get_time(b"time"), -123456789);
        assert_eq!(payload.print(&server, b"int").unwrap(), b"-2147483648");
        assert_eq!(payload.print(&server, b"z").unwrap(), b"4294967295");
        assert_eq!(payload.print(&server, b"time").unwrap(), b"-123456789");
        assert_eq!(payload.print(&server, b"absent"), None);
        payload.set_int(&mut server, b"a", 7);
        assert_eq!(payload.get_string(b"a"), None);
        assert_eq!(payload.get_client(b"a"), None);
        assert_eq!(payload.get_session(b"a"), None);
        assert_eq!(payload.get_window(b"a"), None);
        assert_eq!(payload.get_pane(b"a"), None);
        assert_eq!(
            payload.iter().map(|i| i.name()).collect::<Vec<_>>(),
            [b"a".as_slice(), b"int", b"time", b"z"]
        );
        assert_eq!(
            payload.log_text(&server),
            b"a=7, int=-2147483648, time=-123456789, z=4294967295"
        );
        payload.free(&mut server);
    }

    struct Printable(Rc<Cell<u32>>);
    impl EventPrintable for Printable {
        fn print(&self, output: &mut Vec<u8>) {
            output.extend_from_slice(b"owned");
        }
    }
    impl Drop for Printable {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn extensions_drop_on_replacement_and_free_private_fields_not_exported() {
        let mut server = Server::new();
        let drops = Rc::new(Cell::new(0));
        let mut payload = EventPayload::new();
        payload.set_extension(
            &mut server,
            b"owned",
            EventExtension::Printable(Box::new(Printable(drops.clone()))),
        );
        assert_eq!(payload.print(&server, b"owned").unwrap(), b"owned");
        payload.set_int(&mut server, b"owned", 1);
        assert_eq!(drops.get(), 1);
        payload.set_extension(
            &mut server,
            b"_owned",
            EventExtension::Printable(Box::new(Printable(drops.clone()))),
        );
        let queue = QueueItemId::from_parts(3, 5);
        let monitor = HooksMonitorId::from_parts(4, 6);
        payload.set_queue_item(&mut server, queue);
        payload.set_hooks_monitor(&mut server, monitor);
        assert_eq!(payload.get_queue_item(), Some(queue));
        assert_eq!(payload.get_hooks_monitor(), Some(monitor));
        assert!(payload.get_extension(b"owned").is_none());
        let mut formats = BTreeMap::new();
        payload.export_formats(&server, b"p_", |key, value| {
            formats.insert(key.to_vec(), value);
        });
        assert_eq!(
            formats,
            BTreeMap::from([(b"p_owned".to_vec(), ByteString::from(b"1".as_slice()))])
        );
        assert!(
            payload
                .log_text(&server)
                .as_bytes()
                .starts_with(b"_cmdq_item=")
        );
        payload.set_string(&mut server, b"_cmdq_item", b"wrong");
        assert_eq!(payload.get_queue_item(), None);
        payload.free(&mut server);
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn independent_target_and_object_leases_release_immediately_on_replacement() {
        let mut server = Server::new();
        let state = graph(&mut server, b"leases", 4);
        let (s, w, p) = (state.s.unwrap(), state.w.unwrap(), state.wp.unwrap());
        let refs = (
            server.sessions.get(s).unwrap().references,
            server.windows.get(w).unwrap().references,
            server.panes.get(p).unwrap().references,
        );
        let mut payload = EventPayload::new();
        payload.set_target(&mut server, &state);
        payload.set_session(&mut server, b"session", s);
        payload.set_window(&mut server, b"window", w);
        payload.set_pane(&mut server, b"pane", p);
        assert_eq!(server.sessions.get(s).unwrap().references, refs.0 + 2);
        assert_eq!(server.windows.get(w).unwrap().references, refs.1 + 2);
        assert_eq!(server.panes.get(p).unwrap().references, refs.2 + 2);
        payload.set_uint(&mut server, b"session", 2);
        payload.set_string(&mut server, b"window", b"replaced");
        payload.set_time(&mut server, b"pane", 3);
        assert_eq!(server.sessions.get(s).unwrap().references, refs.0 + 1);
        assert_eq!(server.windows.get(w).unwrap().references, refs.1 + 1);
        assert_eq!(server.panes.get(p).unwrap().references, refs.2 + 1);
        payload.free(&mut server);
        assert_eq!(server.sessions.get(s).unwrap().references, refs.0);
        assert_eq!(server.windows.get(w).unwrap().references, refs.1);
        assert_eq!(server.panes.get(p).unwrap().references, refs.2);
    }

    #[test]
    fn target_derives_session_window_and_copies_index_not_link_or_arbitrary_idx() {
        let mut server = Server::new();
        let state = graph(&mut server, b"derive", 7);
        let mut payload = EventPayload::new();
        let partial = CmdFindState {
            s: None,
            w: None,
            idx: 999,
            ..state
        };
        payload.set_target(&mut server, &partial);
        assert_eq!(payload.target.s, state.s);
        assert_eq!(payload.target.w, state.w);
        assert_eq!(payload.target.idx, 7);
        assert_eq!(payload.target.wl, None);
        let flags = CmdFindFlags::QUIET | CmdFindFlags::PREFER_UNATTACHED;
        assert_eq!(
            payload.get_target(&mut server, flags),
            CmdFindState { flags, ..state }
        );
        payload.set_target(
            &mut server,
            &CmdFindState {
                wl: None,
                idx: 999,
                ..state
            },
        );
        assert_eq!(payload.target.idx, -1);
        assert_eq!(payload.get_target(&mut server, flags).wl, state.wl);
        payload.free(&mut server);
    }

    #[test]
    fn index_reuse_never_selects_the_replacement_window() {
        let mut server = Server::new();
        let state = graph(&mut server, b"reuse", 2);
        let mut payload = EventPayload::new();
        payload.set_target(&mut server, &state);
        let other = graph(&mut server, b"other", 0);
        winlink::winlink_remove(&mut server, state.wl.unwrap());
        let replacement =
            session::session_attach(&mut server, state.s.unwrap(), other.w.unwrap(), 2).unwrap();
        session::session_set_current(&mut server, state.s.unwrap(), Some(replacement));
        let retained =
            session::session_attach(&mut server, other.s.unwrap(), state.w.unwrap(), 3).unwrap();
        let recovered = payload.get_target(&mut server, CmdFindFlags::QUIET);
        assert_eq!(recovered.w, state.w);
        assert_eq!(recovered.wp, state.wp);
        assert_eq!(recovered.wl, Some(retained));
        assert_ne!(recovered.wl, Some(replacement));
        payload.free(&mut server);
    }

    #[test]
    fn target_recovers_link_after_pane_removal_and_window_after_renumber() {
        let mut server = Server::new();
        let state = graph(&mut server, b"changes", 2);
        let mut payload = EventPayload::new();
        payload.set_target(&mut server, &state);
        let new_pane = window::window_add_pane(
            &mut server,
            state.w.unwrap(),
            None,
            0,
            SpawnFlags::default(),
        )
        .unwrap();
        window::window_remove_pane(&mut server, state.w.unwrap(), state.wp.unwrap()).unwrap();
        let recovered = payload.get_target(&mut server, CmdFindFlags::QUIET);
        assert_eq!(recovered.wl, state.wl);
        assert_eq!(recovered.wp, Some(new_pane));
        winlink::winlink_remove(&mut server, state.wl.unwrap());
        let link =
            session::session_attach(&mut server, state.s.unwrap(), state.w.unwrap(), 9).unwrap();
        session::session_set_current(&mut server, state.s.unwrap(), Some(link));
        assert_eq!(
            payload.get_target(&mut server, CmdFindFlags::QUIET).wl,
            Some(link)
        );
        payload.free(&mut server);
    }

    #[test]
    fn destroyed_objects_remain_printable_but_targets_fall_back_to_live_session() {
        let mut server = Server::new();
        let old = graph(&mut server, b"old", 0);
        let live = graph(&mut server, b"live", 0);
        let mut payload = EventPayload::new();
        payload.set_target(&mut server, &old);
        payload.set_session(&mut server, b"session", old.s.unwrap());
        payload.set_window(&mut server, b"window", old.w.unwrap());
        payload.set_pane(&mut server, b"pane", old.wp.unwrap());
        session::session_destroy(&mut server, old.s.unwrap(), false);
        assert!(!session::session_alive(&server, old.s.unwrap()));
        assert_eq!(payload.print(&server, b"session").unwrap(), b"$0");
        assert_eq!(payload.print(&server, b"window").unwrap(), b"@0");
        assert_eq!(payload.print(&server, b"pane").unwrap(), b"%0");
        let flags = CmdFindFlags::CANFAIL | CmdFindFlags::QUIET;
        let recovered = payload.get_target(&mut server, flags);
        assert_eq!(recovered.s, live.s);
        assert_eq!(recovered.flags, flags);
        let mut formats = BTreeMap::new();
        payload.export_formats(&server, b"hook_", |key, value| {
            formats.insert(key.to_vec(), value);
        });
        assert_eq!(
            formats.get(b"hook_session_name".as_slice()).unwrap(),
            b"old"
        );
        payload.free(&mut server);
    }

    #[test]
    fn empty_target_fallback_and_failure_preserve_flags() {
        let mut server = Server::new();
        let payload = EventPayload::new();
        let flags = CmdFindFlags::CANFAIL | CmdFindFlags::QUIET;
        assert_eq!(
            payload.get_target(&mut server, flags),
            CmdFindState::clear(flags)
        );
        let state = graph(&mut server, b"default", 5);
        assert_eq!(payload.get_target(&mut server, flags).s, state.s);
        payload.free(&mut server);
    }

    thread_local! {
        static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static REMOVED: Cell<Option<EventSinkId>> = const { Cell::new(None) };
        static SELF: Cell<Option<EventSinkId>> = const { Cell::new(None) };
    }
    fn trace(value: &str) {
        TRACE.with(|v| v.borrow_mut().push(value.to_owned()));
    }
    fn trace_take() -> Vec<String> {
        TRACE.with(|v| std::mem::take(&mut *v.borrow_mut()))
    }
    fn added(_: &mut Server, payload: &mut EventPayload) {
        trace(if payload.get_int(b"depth") == Some(1) {
            "C1"
        } else {
            "C0"
        });
    }
    fn first(server: &mut Server, payload: &mut EventPayload) {
        if payload.get_int(b"depth") == Some(1) {
            trace("A1");
            return;
        }
        trace("A0");
        let removed = REMOVED.with(Cell::get).unwrap();
        remove_sink(server, removed);
        assert!(server.events.sinks.get(removed).unwrap().dead);
        add_sink(server, b"event", added);
        payload.set_int(server, b"value", 42);
        let mut nested = EventPayload::new();
        nested.set_int(server, b"depth", 1);
        fire(server, b"event", nested);
        assert!(server.events.sinks.get(removed).is_some());
    }
    fn should_not_run(_: &mut Server, _: &mut EventPayload) {
        panic!("dead sink called");
    }
    fn last(_: &mut Server, payload: &mut EventPayload) {
        trace(if payload.get_int(b"depth") == Some(1) {
            "D1"
        } else {
            "D0"
        });
        if payload.get_int(b"depth") != Some(1) {
            assert_eq!(payload.get_int(b"value"), Some(42));
        }
        assert_eq!(payload.get_string(b"event"), Some(b"event".as_slice()));
    }

    #[test]
    fn nested_generation_mutation_and_dead_removal_match_source() {
        trace_take();
        let mut server = Server::new();
        add_sink(&mut server, b"event", first);
        let removed = add_sink(&mut server, b"event", should_not_run);
        REMOVED.with(|v| v.set(Some(removed)));
        add_sink(&mut server, b"event", last);
        add_sink(&mut server, b"event-extra", should_not_run);
        let mut payload = EventPayload::new();
        payload.set_string(&mut server, b"event", b"overwritten");
        fire(&mut server, b"event", payload);
        assert_eq!(trace_take(), ["A0", "A1", "D1", "C1", "D0"]);
        assert!(server.events.sinks.get(removed).is_none());
        remove_sink(&mut server, removed);
        assert_eq!(server.events.dispatching, 0);
    }

    fn self_remove(server: &mut Server, _: &mut EventPayload) {
        trace("self");
        let id = SELF.with(Cell::get).unwrap();
        remove_sink(server, id);
        remove_sink(server, id);
        fire(server, b"self", EventPayload::new());
        assert!(server.events.sinks.get(id).is_some());
    }
    fn following(_: &mut Server, _: &mut EventPayload) {
        trace("following");
    }

    #[test]
    fn self_removal_skips_recursion_without_freeing_outer_cursor() {
        trace_take();
        let mut server = Server::new();
        let id = add_sink(&mut server, b"self", self_remove);
        SELF.with(|v| v.set(Some(id)));
        let tail = add_sink(&mut server, b"self", following);
        fire(&mut server, b"self", EventPayload::new());
        assert_eq!(trace_take(), ["self", "following", "following"]);
        assert!(server.events.sinks.get(id).is_none());
        remove_sink(&mut server, tail);
        assert!(server.events.head.is_none());
        assert!(server.events.tail.is_none());
        assert!(server.events.sinks.is_empty());
    }

    fn inspect_schema(server: &mut Server, payload: &mut EventPayload) {
        let state = payload.get_target(server, CmdFindFlags::QUIET);
        assert!(state.is_valid(server));
        match payload.get_string(b"event").unwrap() {
            b"session" => assert_eq!(payload.iter().count(), 2),
            b"window" => assert_eq!(payload.iter().count(), 2),
            b"pane" => {
                assert_eq!(payload.iter().count(), 3);
                assert_eq!(payload.get_pane(b"pane"), state.wp);
            }
            b"winlink" => {
                assert_eq!(payload.iter().count(), 4);
                assert_eq!(payload.get_int(b"window_index"), Some(11));
            }
            _ => panic!("unexpected event"),
        }
    }

    #[test]
    fn convenience_events_have_exact_fields_and_targets() {
        let mut server = Server::new();
        let state = graph(&mut server, b"schema", 11);
        for name in [b"session".as_slice(), b"window", b"pane", b"winlink"] {
            add_sink(&mut server, name, inspect_schema);
        }
        fire_session(&mut server, b"session", state.s.unwrap());
        fire_window(&mut server, b"window", state.w.unwrap());
        fire_pane(&mut server, b"pane", state.wp.unwrap());
        fire_winlink(&mut server, b"winlink", state.wl.unwrap());
    }

    fn inspect_client(server: &mut Server, payload: &mut EventPayload) {
        assert!(payload.get_client(b"client").is_some());
        assert_eq!(payload.iter().count(), 6);
        assert_eq!(payload.get_int(b"window_index"), Some(11));
        assert!(
            payload
                .get_target(server, CmdFindFlags::QUIET)
                .is_valid(server)
        );
    }

    #[test]
    fn client_events_retain_client_and_recover_attached_target() {
        let mut server = Server::new();
        let state = graph(&mut server, b"client", 11);
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = state.s;
        client.name = Some(b"client\xff".to_vec());
        let id = server.clients.insert(client).unwrap();
        server.clients.retain(id).unwrap();
        server.client_order.push_back(id);
        let mut payload = EventPayload::new();
        payload.set_client(&mut server, b"client", id);
        assert_eq!(payload.print(&server, b"client").unwrap(), b"client\xff");
        assert_eq!(payload.get_client(b"client"), Some(id));
        payload.set_int(&mut server, b"client", 1);
        assert_eq!(payload.get_client(b"client"), None);
        payload.free(&mut server);
        add_sink(&mut server, b"attached", inspect_client);
        fire_client(&mut server, b"attached", id);
    }

    fn monitor_sink(_: &mut Server, payload: &mut EventPayload, monitor: HooksMonitorId) {
        if payload.get_hooks_monitor() == Some(monitor) {
            trace("monitor");
        }
    }

    #[test]
    fn monitor_sinks_receive_distinct_typed_context() {
        trace_take();
        let mut server = Server::new();
        let first = HooksMonitorId::from_parts(1, 2);
        let second = HooksMonitorId::from_parts(2, 3);
        add_monitor_sink(&mut server, b"hook", first, monitor_sink);
        add_monitor_sink(&mut server, b"hook", second, monitor_sink);
        let mut payload = EventPayload::new();
        payload.set_hooks_monitor(&mut server, second);
        fire(&mut server, b"hook", payload);
        assert_eq!(trace_take(), ["monitor"]);
    }

    #[test]
    fn mismatched_saved_window_uses_matching_link_then_session_fallback() {
        let mut server = Server::new();
        let state = graph(&mut server, b"saved", 1);
        let other = graph(&mut server, b"other", 3);
        let mut payload = EventPayload::new();
        payload.set_target(
            &mut server,
            &CmdFindState {
                wp: other.wp,
                ..state
            },
        );
        let recovered = payload.get_target(&mut server, CmdFindFlags::QUIET);
        assert_eq!(recovered.w, other.w);
        assert_eq!(recovered.wp, other.wp);
        payload.set_target(&mut server, &CmdFindState { wp: None, ..state });
        assert_eq!(
            payload.get_target(&mut server, CmdFindFlags::QUIET).wp,
            state.wp
        );
        payload.set_target(
            &mut server,
            &CmdFindState {
                wl: None,
                w: other.w,
                wp: None,
                ..state
            },
        );
        assert_eq!(
            payload.get_target(&mut server, CmdFindFlags::QUIET).w,
            state.w
        );
        payload.free(&mut server);
    }

    struct ReferenceDirectory(std::path::PathBuf);
    impl Drop for ReferenceDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nested_dispatch_trace_matches_pinned_c_reference() {
        let source = std::process::Command::new("git")
            .args(["-C", "/Users/j/fun/tmux", "show", "8f25579c:events.c"])
            .output();
        let Ok(source) = source else {
            eprintln!("skipping event C reference: git is unavailable");
            return;
        };
        if !source.status.success() {
            eprintln!("skipping event C reference: pinned source is unavailable");
            return;
        }
        let source = String::from_utf8(source.stdout).unwrap();
        let source = source.split("/* Fire a client event. */").next().unwrap();
        let directory = ReferenceDirectory(
            std::env::temp_dir().join(format!("rmux-events-reference-{}", std::process::id())),
        );
        std::fs::create_dir(&directory.0).unwrap();
        std::fs::write(directory.0.join("events.c"), source).unwrap();
        std::fs::write(directory.0.join("tmux.h"), r#"
#include <sys/queue.h>
#include <stdio.h>
#ifndef TAILQ_FOREACH_SAFE
#define TAILQ_FOREACH_SAFE(v,h,f,t) for ((v)=TAILQ_FIRST(h);(v)&&((t)=TAILQ_NEXT(v,f),1);(v)=(t))
#endif
struct event_payload { int depth; int value; };
typedef void (*events_cb)(const char *, struct event_payload *, void *);
static void *xcalloc(size_t n, size_t s) { void *p=calloc(n,s); if (!p) abort(); return p; }
static char *xstrdup(const char *s) { char *p=strdup(s); if (!p) abort(); return p; }
static void event_payload_set_string(struct event_payload *p, const char *key, const char *fmt, ...) { (void)p; (void)key; (void)fmt; }
static int log_get_level(void) { return 0; }
static void event_payload_log(struct event_payload *p, const char *fmt, ...) { (void)p; (void)fmt; }
static void event_payload_free(struct event_payload *p) { free(p); }
"#).unwrap();
        std::fs::write(directory.0.join("reference.c"), r#"
#include "events.c"
static struct events_sink *removed;
static void added(const char *n, struct event_payload *p, void *d) { (void)n;(void)d;printf("C%d\n",p->depth); }
static void first(const char *n, struct event_payload *p, void *d) {
    (void)d;
    printf("A%d\n",p->depth);
    if (p->depth) return;
    events_remove_sink(removed);
    events_add_sink(n,added,NULL);
    p->value=42;
    struct event_payload *nested=xcalloc(1,sizeof *nested);
    nested->depth=1;
    events_fire(n,nested);
    if (!removed->dead) abort();
}
static void dead(const char *n, struct event_payload *p, void *d) { (void)n;(void)p;(void)d;abort(); }
static void last(const char *n, struct event_payload *p, void *d) {
    (void)n;(void)d;
    if (!p->depth && p->value != 42) abort();
    printf("D%d\n",p->depth);
}
int main(void) {
    events_add_sink("event",first,NULL);
    removed=events_add_sink("event",dead,NULL);
    events_add_sink("event",last,NULL);
    events_add_sink("event-extra",dead,NULL);
    events_fire("event",xcalloc(1,sizeof(struct event_payload)));
    return 0;
}
"#).unwrap();
        let binary = directory.0.join("reference");
        let compile = std::process::Command::new("cc")
            .arg(directory.0.join("reference.c"))
            .arg("-o")
            .arg(&binary)
            .output();
        let Ok(compile) = compile else {
            eprintln!("skipping event C reference: cc is unavailable");
            return;
        };
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let expected = std::process::Command::new(binary).output().unwrap();
        assert!(expected.status.success());
        trace_take();
        let mut server = Server::new();
        add_sink(&mut server, b"event", first);
        REMOVED.with(|v| v.set(Some(add_sink(&mut server, b"event", should_not_run))));
        add_sink(&mut server, b"event", last);
        add_sink(&mut server, b"event-extra", should_not_run);
        fire(&mut server, b"event", EventPayload::new());
        let mut actual = trace_take().join("\n");
        actual.push('\n');
        assert_eq!(actual.as_bytes(), expected.stdout);
    }

    fn dead_session_event(server: &mut Server, payload: &mut EventPayload) {
        assert!(payload.get_session(b"session").is_some());
        assert!(payload.target.is_empty());
        assert_eq!(payload.iter().count(), 2);
        assert_eq!(payload.print(server, b"session").unwrap(), b"$0");
    }

    #[test]
    fn destroyed_session_convenience_event_retains_object_without_target() {
        let mut server = Server::new();
        let state = graph(&mut server, b"destroyed", 0);
        let id = state.s.unwrap();
        assert!(session::session_retain(&mut server, id));
        session::session_destroy(&mut server, id, false);
        add_sink(&mut server, b"destroyed", dead_session_event);
        fire_session(&mut server, b"destroyed", id);
        assert_eq!(server.sessions.get(id).unwrap().references, 1);
        assert!(session::session_release(&mut server, id));
    }

    #[test]
    fn object_export_resolves_current_names_instead_of_caching_at_set_time() {
        let mut server = Server::new();
        let state = graph(&mut server, b"before", 0);
        let mut payload = EventPayload::new();
        payload.set_session(&mut server, b"session", state.s.unwrap());
        payload.set_window(&mut server, b"window", state.w.unwrap());
        server.sessions.get_mut(state.s.unwrap()).unwrap().name = b"after-session".to_vec();
        server.windows.get_mut(state.w.unwrap()).unwrap().name = b"after-window".to_vec();
        let mut formats = BTreeMap::new();
        payload.export_formats(&server, b"", |key, value| {
            formats.insert(key.to_vec(), value);
        });
        assert_eq!(
            formats.get(b"session_name".as_slice()).unwrap(),
            b"after-session"
        );
        assert_eq!(
            formats.get(b"window_name".as_slice()).unwrap(),
            b"after-window"
        );
        assert_eq!(formats.get(b"session".as_slice()).unwrap(), b"$0");
        assert_eq!(formats.get(b"window".as_slice()).unwrap(), b"@0");
        payload.free(&mut server);
    }

    fn add_at_tail(server: &mut Server, _: &mut EventPayload) {
        trace("tail");
        add_sink(server, b"tail", following);
    }

    #[test]
    fn tail_addition_waits_until_next_fire_and_stale_removal_cannot_remove_new_occupant() {
        trace_take();
        let mut server = Server::new();
        let old = add_sink(&mut server, b"unrelated", should_not_run);
        remove_sink(&mut server, old);
        let new = add_sink(&mut server, b"tail", add_at_tail);
        assert_ne!(old, new);
        remove_sink(&mut server, old);
        fire(&mut server, b"tail", EventPayload::new());
        assert_eq!(trace_take(), ["tail"]);
        fire(&mut server, b"tail", EventPayload::new());
        assert_eq!(trace_take(), ["tail", "following"]);
    }

    fn identified(server: &mut Server, _: &mut EventPayload, id: EventSinkId) {
        assert_eq!(server.events.sinks.get(id).unwrap().name, b"identified");
        remove_sink(server, id);
        fire(server, b"identified", EventPayload::new());
        trace("identified");
    }

    #[test]
    fn sink_id_callback_can_remove_itself_and_is_not_reentered() {
        trace_take();
        let mut server = Server::new();
        let id = add_sink_with_id(&mut server, b"identified", identified);
        fire(&mut server, b"identified", EventPayload::new());
        assert_eq!(trace_take(), ["identified"]);
        assert!(server.events.sinks.get(id).is_none());
    }
}
