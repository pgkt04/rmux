// Ported from tmux cmd-parse.y, cmd-queue.c, arguments.c, key-bindings.c, cfg.c, hooks.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2019 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2012 George Nachman <tmux@georgester.com>
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

use std::any::Any;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::client::{self, ClientFlags};
use crate::cmd::arguments::{ArgumentFormatRuntime, ArgumentsRuntime};
use crate::cmd::cfg::{CfgCallback, CfgRuntime, CfgState, CfgViewError};
use crate::cmd::find::{self, CmdFindFlags, CmdFindState, ModelView};
use crate::cmd::hooks::{self, HookPayload, HooksRuntime, HooksStore, MonitorChange};
use crate::cmd::key_bindings::KeyInitRuntime;
use crate::cmd::parse::{CmdParseInput, ParseContext, ParseQueueContext};
use crate::cmd::queue::{
    self, CmdReturn, ControlGuard, QueueBatch, QueueClientView, QueueEvent, QueueHookPayload,
    QueueRuntime, QueueStateFlags, QueueStore,
};
use crate::cmd::{Command, CommandList};
use crate::format::{FormatContext, FormatFlags, FormatTree};
use crate::ids::{
    ArenaError, ClientId, EventSinkId, HooksMonitorId, MonitorSetId, OptionsId, PaneId,
    QueueItemId, QueueStateId, SessionId,
};
use crate::model::Server;
use crate::model::monitor::{self, MonitorFlags, MonitorType};
use crate::model::pane::CfgModelRuntime;
use crate::model::store_runtime::HooksModelRuntime;
use crate::options::{OptionsStore, environment::EnvironmentFlags};
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::time::Timestamp;

fn new_state(
    server: &mut Server,
    target: Option<&CmdFindState>,
    event: Option<&QueueEvent>,
    flags: QueueStateFlags,
) -> QueueStateId {
    let mut store = std::mem::take(&mut server.queue);
    let state = store.new_state(server, target, event, flags);
    server.queue = store;
    state.expect("queue state arena")
}

fn copy_state(
    server: &mut Server,
    state: QueueStateId,
    current: Option<&CmdFindState>,
) -> QueueStateId {
    let mut store = std::mem::take(&mut server.queue);
    let copied = store.copy_state(server, state, current);
    server.queue = store;
    copied.expect("live queue state")
}

fn state_formats(server: &Server, item: QueueItemId) -> BTreeMap<ByteString, ByteString> {
    let mut formats = BTreeMap::new();
    server
        .queue
        .merge_formats(item, &mut formats)
        .expect("live queue item");
    formats
}

fn format_tree(
    server: &mut Server,
    item: Option<QueueItemId>,
    client: Option<ClientId>,
    target: &CmdFindState,
    flags: FormatFlags,
) -> FormatTree {
    let mut tree = FormatTree::create(client, item, 0, flags, server);
    tree.defaults(
        server,
        FormatContext {
            evaluated_client: client,
            session: target.s,
            winlink: target.wl,
            window: target.w,
            pane: target.wp,
            ..FormatContext::default()
        },
    );
    tree
}

impl ParseContext for Server {
    fn environment(&self, name: &[u8]) -> Option<&[u8]> {
        self.global_environment
            .find(cstr(name))?
            .value
            .as_ref()
            .map(ByteString::as_bytes)
    }
    fn put_environment(&mut self, assignment: &[u8], hidden: bool) {
        self.global_environment.put(
            cstr(assignment),
            if hidden {
                EnvironmentFlags::HIDDEN
            } else {
                EnvironmentFlags::default()
            },
        );
    }
    fn alias(&self, name: &[u8]) -> Option<ByteString> {
        crate::cmd::get_alias(&self.options, cstr(name))
    }
    fn condition(&mut self, value: &[u8], input: &CmdParseInput) -> bool {
        let target = if input.target.is_valid(self) {
            input.target
        } else {
            find::from_client(self, input.client, CmdFindFlags::default()).unwrap_or_default()
        };
        let mut tree = format_tree(self, input.item, input.client, &target, FormatFlags::NOJOBS);
        let expanded = tree.expand(self, value);
        tree.release(self);
        crate::format::true_value(Some(&expanded))
    }
    fn home(&mut self, user: Option<&[u8]>) -> Option<ByteString> {
        if user.is_none()
            && let Some(home) =
                ParseContext::environment(self, b"HOME").filter(|home| !home.is_empty())
        {
            return Some(home.into());
        }
        match user {
            Some(user) => rmux_sys::proc::home_directory(Some(user)).map(ByteString::from),
            None => rmux_sys::server::user_home_directory(rmux_sys::proc::getuid())
                .map(ByteString::from),
        }
    }
    fn next_group(&mut self) -> u32 {
        let group = self.next_command_group;
        self.next_command_group = group.wrapping_add(1);
        group
    }
    fn print(&mut self, message: &[u8], input: &CmdParseInput) {
        if let Some(item) = input.item {
            queue::print(self, item, message);
        }
    }
}

impl ParseQueueContext for Server {
    fn insert_commands(&mut self, list: Rc<CommandList>, after: QueueItemId, state: QueueStateId) {
        let batch = self
            .queue
            .get_command(list, Some(state))
            .expect("queue command arena");
        queue::insert_after(self, after, batch).expect("live queue insertion anchor");
    }
    fn append_commands(
        &mut self,
        list: Rc<CommandList>,
        client: Option<ClientId>,
        state: QueueStateId,
    ) {
        let batch = self
            .queue
            .get_command(list, Some(state))
            .expect("queue command arena");
        queue::append(self, client, batch).expect("live queue client");
    }
}

impl ArgumentFormatRuntime for Server {
    fn expand_from_target(&mut self, item: QueueItemId, value: &[u8]) -> ByteString {
        let target = ArgumentsRuntime::item_target(self, item);
        let client = ArgumentsRuntime::item_target_client(self, item);
        let mut tree = format_tree(self, Some(item), client, &target, FormatFlags::NONE);
        let expanded = tree.expand(self, value);
        tree.release(self);
        expanded
    }
}

impl ArgumentsRuntime for Server {
    fn item_target(&self, item: QueueItemId) -> CmdFindState {
        self.queue.items.get(item).expect("live queue item").target
    }
    fn item_target_client(&self, item: QueueItemId) -> Option<ClientId> {
        self.queue
            .items
            .get(item)
            .expect("live queue item")
            .target_client
    }
    fn retain_client(&mut self, client: ClientId) {
        client::lifecycle::retain(self, client).expect("live argument client");
    }
    fn release_client(&mut self, client: ClientId) {
        client::lifecycle::release(self, client).expect("retained argument client");
    }
    fn command_group_counter(&mut self) -> &mut u32 {
        &mut self.next_command_group
    }
    fn queue_error(&mut self, item: QueueItemId, message: &[u8]) {
        queue::error(self, item, message);
    }
}

impl QueueRuntime for Server {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn store(&self) -> &QueueStore {
        &self.queue
    }
    fn store_mut(&mut self) -> &mut QueueStore {
        &mut self.queue
    }
    fn model(&self) -> &dyn ModelView {
        self
    }
    fn client_view(&self, client: ClientId) -> Option<QueueClientView<'_>> {
        let client = self.clients.get(client)?;
        Some(QueueClientView {
            name: client.name.as_deref().unwrap_or_default(),
            flags: client.flags,
            peer_uid: client
                .peer
                .and_then(|peer| self.process.peer_uid(peer))
                .map(|uid| uid.0),
        })
    }
    fn retain_client(&mut self, client: ClientId) -> Result<(), ArenaError> {
        client::lifecycle::retain(self, client)
    }
    fn release_client(&mut self, client: ClientId) {
        client::lifecycle::release(self, client).expect("retained queue client");
    }
    fn config_finished(&self) -> bool {
        self.cfg.finished
    }
    fn now(&self) -> i64 {
        self.current_time.0
    }
    fn server_uid(&self) -> u32 {
        rmux_sys::proc::getuid().0
    }
    fn user_name(&mut self, uid: u32) -> Option<ByteString> {
        rmux_sys::server::user_name(rmux_sys::UserId(uid)).map(ByteString::from)
    }
    fn uppercase(&self, byte: u8) -> u8 {
        rmux_sys::server::uppercase(byte)
    }
    fn execute(&mut self, command: &Command, item: QueueItemId) -> CmdReturn {
        crate::cmd::commands::execute_a(self, command, item)
            .or_else(|| crate::cmd::commands::execute_b(self, command, item))
            .unwrap_or_else(|| {
                let mut cause = b"unknown command: ".to_vec();
                cause.extend_from_slice(command.entry.name);
                queue::error(self, item, &cause);
                CmdReturn::Error
            })
    }
    fn fire_event(&mut self, name: &[u8], payload: QueueHookPayload) {
        let mut event = super::events::EventPayload::new();
        event.set_queue_item(self, payload.item);
        if let Some(target) = payload.current {
            event.set_target(self, &target);
        }
        for (key, value) in payload.formats {
            event.set_string(self, &key, &value);
        }
        super::events::fire(self, name, event);
    }
    fn message(&mut self, message: &[u8]) {
        super::run::add_message(self, message);
    }
    fn config_cause(&mut self, message: &[u8]) {
        self.cfg.add_cause(message.into());
    }
    fn client_print(&mut self, client: Option<ClientId>, parse: bool, data: &[u8]) {
        client::print::print(self, client, parse, data);
    }
    fn control_guard(
        &mut self,
        client: ClientId,
        guard: ControlGuard,
        time: i64,
        number: u32,
        flags: u32,
    ) {
        let guard = match guard {
            ControlGuard::Begin => crate::control::ControlGuard::Begin,
            ControlGuard::End => crate::control::ControlGuard::End,
            ControlGuard::Error => crate::control::ControlGuard::Error,
        };
        crate::control::write_guard(self, client, guard, time, number, flags as i32);
    }
    fn control_write(&mut self, client: ClientId, message: &[u8]) {
        crate::control::write(self, client, message);
    }
    fn file_error(&mut self, client: ClientId, message: &[u8]) {
        super::file::error(self, client, message);
    }
    fn status_message(&mut self, client: ClientId, message: &[u8]) {
        crate::ui::status::status_message_set(self, Some(client), -1, true, false, false, message);
    }
    fn set_exit_status(&mut self, client: ClientId, status: i32) {
        if let Some(client) = self.clients.get_mut(client) {
            client.retval = status;
        }
    }
}

impl KeyInitRuntime for Server {
    fn append_default_commands(&mut self, list: Rc<CommandList>) {
        let batch = self
            .queue
            .get_command(list, None)
            .expect("default key command arena");
        queue::append(self, None, batch).expect("global command queue");
    }
    fn append_default_snapshot(&mut self) {
        let batch = self
            .queue
            .get_callback(
                "key_bindings_init_done",
                queue::callback_for::<Server>(|server, _| {
                    server.key_bindings.init_done();
                    CmdReturn::Normal
                }),
            )
            .expect("key snapshot callback arena");
        queue::append(self, None, batch).expect("global command queue");
    }
}

fn cfg_done(server: &mut Server) -> CmdReturn {
    if server.cfg.finished {
        return CmdReturn::Normal;
    }
    server.cfg.finished = true;
    let mut causes = CfgState {
        causes: std::mem::take(&mut server.cfg.causes),
        ..CfgState::default()
    };
    causes.show_causes(server, None);
    server.cfg.causes.extend(causes.causes);
    if let Some(item) = server.cfg.item.take() {
        queue::continue_item(&mut server.queue, item);
    }
    CfgRuntime::load_prompt_history(server);
    CmdReturn::Normal
}

impl CfgRuntime for Server {
    fn first_client(&self) -> Option<ClientId> {
        self.client_order.front().copied()
    }
    fn client_dead(&self, client: ClientId) -> bool {
        self.clients
            .get(client)
            .is_none_or(|client| client.flags.contains(ClientFlags::DEAD))
    }
    fn client_control(&self, client: ClientId) -> bool {
        self.clients
            .get(client)
            .is_some_and(|client| client.flags.contains(ClientFlags::CONTROL))
    }
    fn client_session(&self, client: ClientId) -> Option<SessionId> {
        self.clients.get(client)?.session
    }
    fn item_client(&self, item: QueueItemId) -> Option<ClientId> {
        self.queue.items.get(item)?.client
    }
    fn first_session_by_name(&self) -> Option<SessionId> {
        self.session_names.first_key_value().map(|(_, id)| *id)
    }
    fn session_attached(&self, session: SessionId) -> bool {
        self.sessions
            .get(session)
            .is_some_and(|session| session.attached != 0)
    }
    fn session_active_pane(&self, session: SessionId) -> PaneId {
        let link = self
            .sessions
            .get(session)
            .expect("live config session")
            .current
            .expect("session current window");
        let window = self.winlinks.get(link).expect("session winlink").window;
        self.windows
            .get(window)
            .expect("session window")
            .active
            .expect("window active pane")
    }
    fn pane_top_is_view(&self, pane: PaneId) -> bool {
        CfgModelRuntime::pane_top_is_view(self, pane)
    }
    fn enter_view_mode(&mut self, _pane: PaneId) -> Result<(), CfgViewError> {
        Err(CfgViewError::Unavailable)
    }
    fn append_view_line(&mut self, pane: PaneId, line: &[u8]) -> Result<(), CfgViewError> {
        CfgModelRuntime::append_view_line(self, pane, line).map_err(CfgViewError::Model)
    }
    fn print_cfg_fallback(&mut self, client: Option<ClientId>, cause: &[u8]) {
        if let Some(client) = client {
            client::print::print(self, Some(client), true, cause);
        } else {
            use std::io::Write;
            super::run::add_message(self, cause);
            let mut output = std::io::stdout().lock();
            if let Err(error) = output
                .write_all(cause)
                .and_then(|()| output.write_all(b"\n"))
            {
                rmux_util::log::write("cfg", format_args!("configuration output: {error}"));
            }
        }
    }
    fn notify_config_error(&mut self, client: ClientId, cause: &[u8]) {
        let mut message = b"%config-error ".to_vec();
        message.extend_from_slice(cause);
        crate::control::notify_write(self, client, &message);
    }
    fn print_cfg_cause(&mut self, item: QueueItemId, cause: &[u8]) {
        queue::print(self, item, cause);
    }
    fn load_prompt_history(&mut self) {
        crate::ui::prompt::history::load(self);
    }
    fn append_cfg_callback(
        &mut self,
        client: Option<ClientId>,
        callback: CfgCallback,
    ) -> QueueItemId {
        let name = match callback {
            CfgCallback::ClientDone => "cfg_client_done",
            CfgCallback::Done => "cfg_done",
        };
        let batch = self
            .queue
            .get_callback(
                name,
                queue::callback_for::<Server>(move |server, item| match callback {
                    CfgCallback::ClientDone => {
                        let client = server
                            .queue
                            .items
                            .get(item)
                            .and_then(|item| item.client)
                            .expect("configuration client callback");
                        server.cfg.client_done(server, client)
                    }
                    CfgCallback::Done => cfg_done(server),
                }),
            )
            .expect("config callback arena");
        queue::append(self, client, batch).expect("configuration queue client")
    }
    fn continue_cfg_item(&mut self, item: QueueItemId) {
        queue::continue_item(&mut self.queue, item);
    }
    fn new_cfg_state(&mut self) -> QueueStateId {
        new_state(self, None, None, QueueStateFlags::default())
    }
    fn copy_cfg_state(
        &mut self,
        item: QueueItemId,
        current: Option<&CmdFindState>,
    ) -> QueueStateId {
        let state = self
            .queue
            .items
            .get(item)
            .expect("live configuration item")
            .state;
        copy_state(self, state, current)
    }
    fn add_cfg_format(&mut self, state: QueueStateId, name: &[u8], value: &[u8]) {
        self.queue
            .add_format(state, name, value)
            .expect("live config state");
    }
    fn cfg_commands(&mut self, list: Rc<CommandList>, state: QueueStateId) -> QueueBatch {
        self.queue
            .get_command(list, Some(state))
            .expect("config command arena")
    }
    fn free_cfg_state(&mut self, state: QueueStateId) {
        self.queue.free_state(state).expect("retained config state");
    }
    fn append_cfg_commands(&mut self, batch: QueueBatch) -> Option<QueueItemId> {
        Some(queue::append(self, None, batch).expect("global config queue"))
    }
    fn insert_cfg_commands(&mut self, item: QueueItemId, batch: QueueBatch) -> Option<QueueItemId> {
        Some(queue::insert_after(self, item, batch).expect("live config insertion anchor"))
    }
}

fn hook_payload(server: &mut Server, event: &super::events::EventPayload) -> HookPayload {
    let target = event.get_target(server, CmdFindFlags::default());
    let mut formats = BTreeMap::new();
    event.export_formats(server, b"", |key, value| {
        formats.insert(key.into(), value);
    });
    HookPayload {
        target: Some(target),
        item: event.get_queue_item(),
        monitor: event.get_hooks_monitor(),
        client: event.get_client(b"client"),
        formats,
    }
}

fn hook_event(server: &mut Server, event: &mut super::events::EventPayload) {
    let name = ByteString::from(event.get_string(b"event").expect("event name"));
    let payload = hook_payload(server, event);
    hooks::event(server, &name, &payload);
}

fn hook_monitor_event(
    server: &mut Server,
    event: &mut super::events::EventPayload,
    monitor: HooksMonitorId,
) {
    if event.get_hooks_monitor() != Some(monitor) {
        return;
    }
    let name = ByteString::from(event.get_string(b"event").expect("event name"));
    let payload = hook_payload(server, event);
    hooks::monitor_event(server, monitor, &name, &payload);
}

fn hook_monitor_change(server: &mut Server, id: HooksMonitorId, change: &MonitorChange<'_>) {
    let Some(saved) = server
        .hooks
        .monitors
        .get(id)
        .and_then(Option::as_ref)
        .map(|monitor| monitor.target)
    else {
        return;
    };
    let target = hooks::monitor_target(server, change, &saved);
    let mut event = super::events::EventPayload::new();
    event.set_hooks_monitor(server, id);
    event.set_target(server, &target);
    event.set_string(server, b"value", change.value.unwrap_or_default());
    event.set_string(server, b"last", change.last.unwrap_or_default());
    if let Some(client) = change.client {
        event.set_client(server, b"client", client);
    }
    if let Some(session) = change.session {
        event.set_session(server, b"session", session);
    }
    if let Some(link) = change.winlink.and_then(|id| server.winlinks.get(id)) {
        let (session, window, index) = (link.session, link.window, link.index);
        if change.session.is_none() {
            event.set_session(server, b"session", session);
        }
        event.set_window(server, b"window", window);
        event.set_int(server, b"window_index", index);
    }
    if let Some(pane) = change.pane {
        let window = server.panes.get(pane).expect("monitor pane").window;
        event.set_pane(server, b"pane", pane);
        if change.winlink.is_none() {
            event.set_window(server, b"window", window);
        }
    }
    super::events::fire(server, change.name, event);
}

impl HooksRuntime for Server {
    fn hooks(&self) -> &HooksStore {
        &self.hooks
    }
    fn hooks_mut(&mut self) -> &mut HooksStore {
        &mut self.hooks
    }
    fn options(&self) -> &OptionsStore {
        &self.options
    }
    fn options_mut(&mut self) -> &mut OptionsStore {
        &mut self.options
    }
    fn model(&self) -> &dyn ModelView {
        self
    }
    fn session_options(&self, session: SessionId) -> OptionsId {
        self.sessions.get(session).expect("hook session").options
    }
    fn pane_options(&self, pane: PaneId) -> OptionsId {
        self.panes.get(pane).expect("hook pane").options
    }
    fn window_options(&self, state: &CmdFindState) -> Option<OptionsId> {
        let window = self.winlinks.get(state.wl?)?.window;
        Some(self.windows.get(window)?.options)
    }
    fn now(&self) -> Timestamp {
        Timestamp::new(self.current_time.0, self.current_time.1 as i32)
    }
    fn item_flags(&self, item: QueueItemId) -> QueueStateFlags {
        let state = self.queue.items.get(item).expect("hook item").state;
        self.queue.states.get(state).expect("hook state").flags
    }
    fn item_event(&self, item: QueueItemId) -> QueueEvent {
        let state = self.queue.items.get(item).expect("hook item").state;
        self.queue.states.get(state).expect("hook state").event
    }
    fn item_formats(&self, item: QueueItemId) -> BTreeMap<ByteString, ByteString> {
        state_formats(self, item)
    }
    fn item_target(&self, item: QueueItemId) -> CmdFindState {
        ArgumentsRuntime::item_target(self, item)
    }
    fn item_client(&self, item: QueueItemId) -> Option<ClientId> {
        self.queue.items.get(item)?.client
    }
    fn global_running(&self) -> Option<QueueItemId> {
        self.queue.global.running.filter(|id| {
            self.queue
                .items
                .get(*id)
                .is_some_and(|item| !item.flags.contains(queue::QueueItemFlags::WAITING))
        })
    }
    fn expand_hook(
        &mut self,
        value: &[u8],
        target: &CmdFindState,
        client: Option<ClientId>,
        formats: &BTreeMap<ByteString, ByteString>,
    ) -> ByteString {
        let mut tree = format_tree(self, None, client, target, FormatFlags::NONE);
        for (key, value) in formats {
            tree.add(key, value.clone());
        }
        let expanded = tree.expand(self, value);
        tree.release(self);
        expanded
    }
    fn new_hook_state(
        &mut self,
        target: &CmdFindState,
        event: Option<&QueueEvent>,
        flags: QueueStateFlags,
        formats: &BTreeMap<ByteString, ByteString>,
    ) -> QueueStateId {
        let state = new_state(self, Some(target), event, flags);
        self.queue
            .add_formats(state, formats)
            .expect("live hook state");
        state
    }
    fn free_hook_state(&mut self, state: QueueStateId) {
        self.queue.free_state(state).expect("retained hook state");
    }
    fn insert_hook_commands(
        &mut self,
        after: Option<QueueItemId>,
        list: Rc<CommandList>,
        state: QueueStateId,
    ) -> QueueItemId {
        let batch = self
            .queue
            .get_command(list, Some(state))
            .expect("hook command arena");
        match after {
            Some(after) => queue::insert_after(self, after, batch).expect("live hook anchor"),
            None => queue::append(self, None, batch).expect("global hook queue"),
        }
    }
    fn hook_parse_error(&mut self, item: Option<QueueItemId>, message: &[u8], debug_only: bool) {
        if !debug_only && let Some(item) = item {
            queue::error(self, item, message);
        } else {
            rmux_util::log::write(
                "hooks",
                format_args!("can't parse hook: {}", String::from_utf8_lossy(message)),
            );
        }
    }
    fn add_hook_sink(&mut self, name: &[u8], monitor: Option<HooksMonitorId>) -> EventSinkId {
        match monitor {
            Some(monitor) => {
                super::events::add_monitor_sink(self, name, monitor, hook_monitor_event)
            }
            None => super::events::add_sink(self, name, hook_event),
        }
    }
    fn remove_hook_sink(&mut self, sink: EventSinkId) {
        super::events::remove_sink(self, sink);
    }
    fn create_monitor_set(
        &mut self,
        session: Option<SessionId>,
        monitor: HooksMonitorId,
    ) -> MonitorSetId {
        self.hook_monitor_dispatch = Some(hook_monitor_change);
        self.create_hook_monitor_set(session, monitor)
            .expect("hook monitor session")
    }
    fn destroy_monitor_set(&mut self, set: MonitorSetId) {
        let mut adapter = crate::control::monitor::MonitorAdapter::new(self);
        let result = monitor::monitor_destroy(self, set, &mut adapter);
        adapter.apply(self);
        result.expect("live hook monitor set");
        self.hook_monitor_targets.remove(&set);
    }
    fn add_model_monitor(
        &mut self,
        set: MonitorSetId,
        name: &[u8],
        kind: MonitorType,
        public_id: i32,
        format: &[u8],
        flags: MonitorFlags,
    ) {
        let spec = monitor::MonitorSpec {
            name: name.into(),
            kind,
            target: (public_id >= 0).then_some(public_id as u32),
            format: format.into(),
            flags,
        };
        let mut adapter = crate::control::monitor::MonitorAdapter::new(self);
        let result = monitor::monitor_add(self, set, spec, &mut adapter);
        adapter.apply(self);
        result.expect("live hook monitor set");
    }
    fn model_monitor_stats(&self, set: MonitorSetId, name: &[u8]) -> (u32, i64) {
        (
            monitor::monitor_get_fire_count(self, set, name),
            monitor::monitor_get_fire_time(self, set, name),
        )
    }
    fn ensure_monitor_option(&mut self, options: OptionsId, name: &[u8]) {
        if self.options.get_only(options, name).is_none() {
            let mut store = std::mem::take(&mut self.options);
            store.set_string(options, name, false, b"", self);
            self.options = store;
        }
    }
    fn fire_hook_event(&mut self, name: &[u8], payload: HookPayload) {
        let mut event = super::events::EventPayload::new();
        if let Some(target) = payload.target {
            event.set_target(self, &target);
        }
        if let Some(item) = payload.item {
            event.set_queue_item(self, item);
        }
        if let Some(monitor) = payload.monitor {
            event.set_hooks_monitor(self, monitor);
        }
        for (key, value) in payload.formats {
            event.set_string(self, &key, &value);
        }
        if let Some(client) = payload.client {
            event.set_client(self, b"client", client);
        }
        super::events::fire(self, name, event);
    }
    fn monitor_formats(&self, change: &MonitorChange<'_>) -> BTreeMap<ByteString, ByteString> {
        let mut formats = self.hook_monitor_formats(change, None);
        if let Some(name) = change
            .client
            .and_then(|id| self.clients.get(id))
            .and_then(|client| client.name.as_deref())
        {
            formats.insert("client".into(), name.into());
        }
        formats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::CommandListPrintFlags;
    use crate::cmd::parse::{self, CommandParser};

    #[test]
    fn parser_environment_conditions_home_and_groups_are_live() {
        let mut server = Server::new();
        ParseContext::put_environment(&mut server, b"HOME=/runtime-home", false);
        let list = CommandParser::parse_from_string(
            &mut server,
            b"X=value\n%if 1\ndisplay-message $X\n%endif\n",
        )
        .unwrap();
        assert_eq!(
            list.print(CommandListPrintFlags::default()),
            b"display-message value"
        );
        assert_eq!(
            ParseContext::environment(&server, b"X"),
            Some(b"value".as_slice())
        );
        assert_eq!(
            ParseContext::home(&mut server, None).unwrap(),
            b"/runtime-home"
        );
        ParseContext::put_environment(&mut server, b"HOME=", false);
        assert_eq!(
            ParseContext::home(&mut server, None).map(ByteString::into_vec),
            rmux_sys::server::user_home_directory(rmux_sys::proc::getuid())
        );
        let group = ParseContext::next_group(&mut server);
        assert_eq!(
            *ArgumentsRuntime::command_group_counter(&mut server),
            group.wrapping_add(1)
        );
        assert!(!ParseContext::condition(
            &mut server,
            b"0",
            &CmdParseInput::default()
        ));
        let list = CommandParser::parse_from_string(&mut server, b"split-pane").unwrap();
        assert_eq!(list.commands[0].entry.name, b"split-window");
        let mut input = CmdParseInput {
            flags: parse::CmdParseFlags::PARSEONLY,
            ..CmdParseInput::default()
        };
        parse::from_buffer(
            &mut server,
            b"X=ignored\n%hidden SECRET=ignored\n",
            &mut input,
        )
        .unwrap();
        assert_eq!(
            ParseContext::environment(&server, b"X"),
            Some(b"value".as_slice())
        );
        assert!(ParseContext::environment(&server, b"SECRET").is_none());
        ParseContext::put_environment(&mut server, b"SECRET=stored", true);
        assert!(
            server
                .global_environment
                .find(b"SECRET")
                .unwrap()
                .flags
                .contains(EnvironmentFlags::HIDDEN)
        );
    }

    #[test]
    fn queue_states_are_restored_and_config_copy_drops_extra_formats() {
        let mut server = Server::new();
        let state = CfgRuntime::new_cfg_state(&mut server);
        server
            .queue
            .add_format(state, b"current_file", b"first.conf")
            .unwrap();
        let list = CommandParser::parse_from_string(&mut server, b"display-message value").unwrap();
        ParseQueueContext::append_commands(&mut server, list, None, state);
        let item = server.queue.global.head.unwrap();
        let copied = CfgRuntime::copy_cfg_state(&mut server, item, None);
        assert!(server.queue.states.get(copied).unwrap().formats.is_empty());
        assert_eq!(
            HooksRuntime::item_formats(&server, item)[b"command".as_slice()],
            b"display-message"
        );
        assert_eq!(
            ArgumentFormatRuntime::expand_from_target(&mut server, item, b"#{current_file}"),
            b"first.conf"
        );
        CfgRuntime::free_cfg_state(&mut server, copied);
        CfgRuntime::free_cfg_state(&mut server, state);
        assert!(server.queue.items.get(item).is_some());
    }

    #[test]
    fn ordinary_hooks_export_payload_and_private_ids_without_recursion() {
        let mut server = Server::new();
        let options = server.options.global_s;
        let mut store = std::mem::take(&mut server.options);
        store.set_string(
            options,
            b"@runtime-hook",
            false,
            b"display-message hook",
            &mut server,
        );
        server.options = store;
        hooks::add_event(&mut server, b"@runtime-hook");
        let payload = HookPayload {
            formats: BTreeMap::from([("value".into(), "changed".into())]),
            ..HookPayload::default()
        };
        HooksRuntime::fire_hook_event(&mut server, b"@runtime-hook", payload);
        let item = server.queue.global.head.unwrap();
        let flags = HooksRuntime::item_flags(&server, item);
        assert!(flags.contains(QueueStateFlags::NOHOOKS));
        let formats = HooksRuntime::item_formats(&server, item);
        assert_eq!(formats[b"hook".as_slice()], b"@runtime-hook");
        assert_eq!(formats[b"hook_value".as_slice()], b"changed");
        assert!(!formats.keys().any(|key| key.starts_with(b"hook__")));
        server.queue.global.running = Some(item);
        HooksRuntime::fire_hook_event(&mut server, b"@runtime-hook", HookPayload::default());
        assert_eq!(server.queue.global.tail, Some(item));
    }

    #[test]
    fn config_view_unavailable_is_explicit_and_append_reaches_model() {
        let mut server = Server::new();
        let window = crate::model::window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = crate::model::pane::pane_create(&mut server, window, 80, 24, 10).unwrap();
        assert!(matches!(
            CfgRuntime::enter_view_mode(&mut server, pane),
            Err(CfgViewError::Unavailable)
        ));
        assert!(matches!(
            CfgRuntime::append_view_line(&mut server, pane, b"cause"),
            Err(CfgViewError::Model(_))
        ));
        assert!(!CfgRuntime::pane_top_is_view(&server, pane));
    }

    #[test]
    fn cfg_completion_restores_finished_state_before_other_callbacks() {
        let mut server = Server::new();
        let batch = server
            .queue
            .get_callback(
                "waiting",
                queue::callback_for::<Server>(|_, _| CmdReturn::Wait),
            )
            .unwrap();
        let waiting = queue::append(&mut server, None, batch).unwrap();
        server.cfg.item = Some(waiting);
        assert_eq!(queue::next(&mut server, None), 0);
        assert!(
            server
                .queue
                .items
                .get(waiting)
                .unwrap()
                .flags
                .contains(queue::QueueItemFlags::WAITING)
        );
        let done = CfgRuntime::append_cfg_callback(&mut server, None, CfgCallback::Done);
        assert!(
            server
                .queue
                .items
                .get(done)
                .unwrap()
                .name
                .starts_with(b"[cfg_done/")
        );
        assert_eq!(cfg_done(&mut server), CmdReturn::Normal);
        assert!(server.cfg.finished);
        assert!(server.cfg.item.is_none());
        assert!(
            !server
                .queue
                .items
                .get(waiting)
                .unwrap()
                .flags
                .contains(queue::QueueItemFlags::WAITING)
        );
        assert_eq!(queue::next(&mut server, None), 1);
        assert!(server.queue.global.head.is_none());
        assert_eq!(cfg_done(&mut server), CmdReturn::Normal);
    }

    #[test]
    fn identifying_client_configuration_callback_uses_upstream_name() {
        let mut server = Server::new();
        let client = server
            .clients
            .insert(crate::client::Client::new(None, (0, 0)))
            .unwrap();
        server.client_order.push_back(client);
        let item =
            CfgRuntime::append_cfg_callback(&mut server, Some(client), CfgCallback::ClientDone);
        assert!(
            server
                .queue
                .items
                .get(item)
                .unwrap()
                .name
                .starts_with(b"[cfg_client_done/")
        );
        assert_eq!(queue::next(&mut server, Some(client)), 0);
        assert!(
            server
                .queue
                .items
                .get(item)
                .unwrap()
                .flags
                .contains(queue::QueueItemFlags::WAITING)
        );
        server.cfg.finished = true;
        CfgRuntime::continue_cfg_item(&mut server, item);
        assert_eq!(queue::next(&mut server, Some(client)), 0);
        assert!(server.queue.items.get(item).is_none());
    }

    #[test]
    fn key_snapshot_callback_runs_after_real_binding_commands() {
        let mut server = Server::new();
        let list = CommandParser::parse_from_string(
            &mut server,
            b"bind-key -T runtime x display-message value",
        )
        .unwrap();
        KeyInitRuntime::append_default_commands(&mut server, list);
        KeyInitRuntime::append_default_snapshot(&mut server);
        assert_eq!(queue::next(&mut server, None), 2);
        let table = server
            .key_bindings
            .get_table(b"runtime", false)
            .unwrap()
            .unwrap();
        let key = rmux_util::key::KeyCode(u64::from(b'x'));
        let binding = server.key_bindings.get(table, key).unwrap();
        let default = server.key_bindings.get_default(table, key).unwrap();
        assert_eq!(
            binding.list.print(CommandListPrintFlags::default()),
            b"display-message value"
        );
        assert_eq!(
            default.list.print(CommandListPrintFlags::default()),
            b"display-message value"
        );
        assert!(server.queue.global.head.is_none());
    }

    #[test]
    fn monitor_timer_delivers_typed_payload_and_cleans_up_registration() {
        let mut server = Server::new();
        server.current_time = (42, 0);
        let options = server.options.create(Some(server.options.global_s));
        let session = crate::model::session::session_create(
            &mut server,
            crate::model::session::SessionCreate {
                prefix: None,
                name: Some(b"monitor-session".to_vec()),
                cwd: b"/".to_vec(),
                environment: crate::options::environment::Environment::new(),
                options,
                termios: None,
            },
        );
        let window = crate::model::window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = crate::model::window::window_add_pane(
            &mut server,
            window,
            None,
            10,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        server.windows.get_mut(window).unwrap().active = Some(pane);
        let link = crate::model::session::session_attach(&mut server, session, window, 0).unwrap();
        crate::model::session::session_set_current(&mut server, session, Some(link));
        let mut store = std::mem::take(&mut server.options);
        store.set_string(
            options,
            b"@runtime-monitor",
            false,
            b"display-message '#{hook_value}'",
            &mut server,
        );
        server.options = store;
        super::super::events::add_sink(&mut server, b"@runtime-monitor", |server, event| {
            assert!(event.get_hooks_monitor().is_some());
            assert!(event.get_session(b"session").is_some());
            assert_eq!(
                event.get_string(b"value"),
                Some(b"monitor-session".as_slice())
            );
            assert_eq!(event.get_string(b"last"), Some(b"".as_slice()));
            assert_eq!(event.print(server, b"session_name").as_deref(), None);
        });
        let monitor = hooks::monitor_add(
            &mut server,
            hooks::MonitorSpec {
                options,
                name: b"@runtime-monitor",
                kind: MonitorType::Session,
                public_id: -1,
                format: b"#{session_name}",
                flags: MonitorFlags::INITIAL,
                target: &CmdFindState::default(),
                session: Some(session),
            },
        )
        .unwrap();
        let set = server
            .hooks
            .monitors
            .get(monitor)
            .unwrap()
            .as_ref()
            .unwrap()
            .set;
        let timer_key = format!("monitor:{set:?}").into_bytes();
        assert!(server.runtime_timers.contains_key(&timer_key));
        crate::control::monitor_timer(&mut server, set);
        assert_eq!(
            HooksRuntime::model_monitor_stats(&server, set, b"@runtime-monitor"),
            (1, 42)
        );
        let item = server.queue.global.head.unwrap();
        assert_eq!(
            server
                .queue
                .items
                .get(item)
                .unwrap()
                .command()
                .unwrap()
                .args
                .string(0),
            Some(b"monitor-session".as_slice())
        );
        assert_eq!(
            HooksRuntime::item_formats(&server, item)[b"hook_session_name".as_slice()],
            b"monitor-session"
        );
        hooks::monitor_remove(&mut server, options, b"@runtime-monitor");
        assert!(server.monitors.get(set).is_none());
        assert!(server.hooks.monitors.get(monitor).is_none());
        assert!(!server.runtime_timers.contains_key(&timer_key));
        assert!(!server.hook_monitor_targets.contains_key(&set));
        assert!(
            server
                .options
                .get_only(options, b"@runtime-monitor")
                .unwrap()
                .monitor()
                .is_none()
        );
        let set = HooksRuntime::create_monitor_set(&mut server, None, monitor);
        assert!(server.monitors.get(set).unwrap().session.is_none());
        HooksRuntime::destroy_monitor_set(&mut server, set);
        assert!(server.monitors.get(set).is_none());
    }

    #[test]
    fn command_dispatch_reaches_both_command_groups() {
        let mut server = Server::new();
        let list = CommandParser::parse_from_string(
            &mut server,
            b"start-server ; set-environment -g RUNTIME_DISPATCH reached",
        )
        .unwrap();
        let batch = server.queue.get_command(Rc::clone(&list), None).unwrap();
        let item = queue::append(&mut server, None, batch).unwrap();
        assert_eq!(
            QueueRuntime::execute(&mut server, &list.commands[0], item),
            CmdReturn::Normal
        );
        assert_eq!(
            QueueRuntime::execute(&mut server, &list.commands[1], item),
            CmdReturn::Normal
        );
        assert_eq!(
            ParseContext::environment(&server, b"RUNTIME_DISPATCH"),
            Some(b"reached".as_slice())
        );
    }

    #[test]
    fn parser_insertion_preserves_queue_order_and_queue_errors_reach_config() {
        let mut server = Server::new();
        let state = CfgRuntime::new_cfg_state(&mut server);
        let list =
            CommandParser::parse_from_string(&mut server, b"start-server ; start-server").unwrap();
        ParseQueueContext::append_commands(&mut server, list, None, state);
        let first = server.queue.global.head.unwrap();
        let old_tail = server.queue.global.tail.unwrap();
        let inserted = CommandParser::parse_from_string(&mut server, b"start-server").unwrap();
        ParseQueueContext::insert_commands(&mut server, inserted, first, state);
        let middle = server.queue.items.get(first).unwrap().next.unwrap();
        assert_ne!(middle, old_tail);
        assert_eq!(server.queue.items.get(middle).unwrap().next, Some(old_tail));
        ArgumentsRuntime::queue_error(&mut server, middle, b"runtime failure");
        assert_eq!(server.cfg.causes, vec![ByteString::from("runtime failure")]);
        assert_eq!(QueueRuntime::uppercase(&server, b'f'), b'F');
        assert_eq!(
            QueueRuntime::server_uid(&server),
            rmux_sys::proc::getuid().0
        );
        CfgRuntime::free_cfg_state(&mut server, state);
        assert_eq!(queue::next(&mut server, None), 3);
        assert!(server.queue.global.head.is_none());
    }

    #[test]
    fn client_facts_and_prepared_argument_leases_use_the_live_client() {
        let mut server = Server::new();
        let mut client = crate::client::Client::new(None, (0, 0));
        client.name = Some(b"runtime-client".to_vec());
        let client = server.clients.insert(client).unwrap();
        server.client_order.push_back(client);
        assert_eq!(CfgRuntime::first_client(&server), Some(client));
        assert_eq!(
            QueueRuntime::client_view(&server, client).unwrap().name,
            b"runtime-client"
        );
        QueueRuntime::set_exit_status(&mut server, client, 7);
        assert_eq!(server.clients.get(client).unwrap().retval, 7);
        let list =
            CommandParser::parse_from_string(&mut server, b"display-message nested").unwrap();
        let state = CfgRuntime::new_cfg_state(&mut server);
        let batch = server
            .queue
            .get_command(Rc::clone(&list), Some(state))
            .unwrap();
        let item = queue::append(&mut server, Some(client), batch).unwrap();
        server.queue.items.get_mut(item).unwrap().target_client = Some(client);
        let prepared = crate::cmd::arguments::make_commands_prepare(
            &mut server,
            &list.commands[0],
            item,
            0,
            None,
            false,
            false,
        );
        assert_eq!(
            crate::cmd::arguments::make_commands_get_command(&prepared),
            b"nested"
        );
        crate::cmd::arguments::make_commands_free(&mut server, prepared);
        CfgRuntime::free_cfg_state(&mut server, state);
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .insert(ClientFlags::DEAD);
        assert!(CfgRuntime::client_dead(&server, client));
        assert_eq!(queue::next(&mut server, Some(client)), 1);
        assert!(server.clients.get(client).is_some());
        assert!(server.queue.clients.get(&client).unwrap().head.is_none());
    }
}
