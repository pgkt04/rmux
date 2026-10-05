// Ported from tmux hooks.c @ 8f25579c
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

use super::{
    CommandList,
    find::{self, CmdFindFlags, CmdFindState, ModelView},
    parse::CommandParser,
    queue::{QueueEvent, QueueStateFlags},
};
pub use crate::ids::HooksMonitorId;
use crate::ids::{
    Arena, ArenaError, ClientId, EventSinkId, MonitorSetId, OptionsId, PaneId, QueueItemId,
    SessionId, WinlinkId,
};
use crate::model::monitor::{MonitorFlags, MonitorType};
use crate::options::{OptionsStore, OptionsValue};
use rmux_util::{bytes::ByteString, time::Timestamp};
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct HooksEvent {
    pub name: ByteString,
    pub sink: EventSinkId,
}
#[derive(Clone, Debug)]
pub struct HooksMonitor {
    pub options: OptionsId,
    pub set: MonitorSetId,
    pub sink: EventSinkId,
    pub target: CmdFindState,
    pub kind: MonitorType,
    pub public_id: i32,
    pub format: ByteString,
}
#[derive(Default)]
pub struct HooksStore {
    pub monitors: Arena<Option<HooksMonitor>, HooksMonitorId>,
    pub events: Vec<HooksEvent>,
}
#[derive(Clone, Debug, Default)]
pub struct HookPayload {
    pub target: Option<CmdFindState>,
    pub item: Option<QueueItemId>,
    pub monitor: Option<HooksMonitorId>,
    pub client: Option<ClientId>,
    pub formats: BTreeMap<ByteString, ByteString>,
}
pub struct MonitorChange<'a> {
    pub name: &'a [u8],
    pub winlink: Option<WinlinkId>,
    pub pane: Option<PaneId>,
    pub session: Option<SessionId>,
    pub client: Option<ClientId>,
    pub value: Option<&'a [u8]>,
    pub last: Option<&'a [u8]>,
}

pub trait HooksRuntime: CommandParser {
    fn hooks(&self) -> &HooksStore;
    fn hooks_mut(&mut self) -> &mut HooksStore;
    fn options(&self) -> &OptionsStore;
    fn options_mut(&mut self) -> &mut OptionsStore;
    fn model(&self) -> &dyn ModelView;
    fn session_options(&self, session: SessionId) -> OptionsId;
    fn pane_options(&self, pane: PaneId) -> OptionsId;
    fn window_options(&self, state: &CmdFindState) -> Option<OptionsId>;
    fn now(&self) -> Timestamp;
    fn item_flags(&self, item: QueueItemId) -> QueueStateFlags;
    fn item_event(&self, item: QueueItemId) -> QueueEvent;
    /// `command` plus the item state formats, as `cmdq_merge_formats` adds them (cmd-queue.c:292-304).
    fn item_formats(&self, item: QueueItemId) -> BTreeMap<ByteString, ByteString>;
    fn item_target(&self, item: QueueItemId) -> CmdFindState;
    fn item_client(&self, item: QueueItemId) -> Option<ClientId>;
    fn global_running(&self) -> Option<QueueItemId>;
    fn expand_hook(
        &mut self,
        value: &[u8],
        target: &CmdFindState,
        client: Option<ClientId>,
        formats: &BTreeMap<ByteString, ByteString>,
    ) -> ByteString;
    fn new_hook_state(
        &mut self,
        target: &CmdFindState,
        event: Option<&QueueEvent>,
        flags: QueueStateFlags,
        formats: &BTreeMap<ByteString, ByteString>,
    ) -> crate::ids::QueueStateId;
    fn free_hook_state(&mut self, state: crate::ids::QueueStateId);
    fn insert_hook_commands(
        &mut self,
        after: Option<QueueItemId>,
        list: Rc<CommandList>,
        state: crate::ids::QueueStateId,
    ) -> QueueItemId;
    fn hook_parse_error(&mut self, item: Option<QueueItemId>, message: &[u8], debug_only: bool);
    fn add_hook_sink(&mut self, name: &[u8], monitor: Option<HooksMonitorId>) -> EventSinkId;
    fn remove_hook_sink(&mut self, sink: EventSinkId);
    fn create_monitor_set(
        &mut self,
        session: Option<SessionId>,
        monitor: HooksMonitorId,
    ) -> MonitorSetId;
    fn destroy_monitor_set(&mut self, set: MonitorSetId);
    fn add_model_monitor(
        &mut self,
        set: MonitorSetId,
        name: &[u8],
        kind: MonitorType,
        public_id: i32,
        format: &[u8],
        flags: MonitorFlags,
    );
    fn model_monitor_stats(&self, set: MonitorSetId, name: &[u8]) -> (u32, i64);
    fn ensure_monitor_option(&mut self, options: OptionsId, name: &[u8]);
    fn fire_hook_event(&mut self, name: &[u8], payload: HookPayload);
    fn monitor_formats(&self, change: &MonitorChange<'_>) -> BTreeMap<ByteString, ByteString>;
}

fn option_lookup(
    runtime: &dyn HooksRuntime,
    target: &CmdFindState,
    name: &[u8],
    explicit: Option<OptionsId>,
) -> Option<OptionsId> {
    if let Some(options) = explicit {
        return runtime.options().get_only(options, name).map(|_| options);
    }
    let session = target
        .s
        .map(|s| runtime.session_options(s))
        .unwrap_or(runtime.options().global_s);
    runtime
        .options()
        .get(session, name)
        .map(|(id, _)| id)
        .or_else(|| {
            target.wp.and_then(|pane| {
                runtime
                    .options()
                    .get(runtime.pane_options(pane), name)
                    .map(|(id, _)| id)
            })
        })
        .or_else(|| {
            runtime
                .window_options(target)
                .and_then(|options| runtime.options().get(options, name).map(|(id, _)| id))
        })
}

pub fn insert(
    runtime: &mut dyn HooksRuntime,
    after: Option<QueueItemId>,
    name: &[u8],
    payload: &HookPayload,
    explicit: Option<OptionsId>,
    expand: bool,
) {
    let target = payload
        .target
        .as_ref()
        .filter(|state| state.is_valid(runtime.model()))
        .cloned()
        .or_else(|| find::from_nothing(runtime.model(), CmdFindFlags::default()));
    let target = target.unwrap_or_default();
    let Some(options) = option_lookup(runtime, &target, name, explicit) else {
        return;
    };
    let now = runtime.now();
    runtime
        .options_mut()
        .get_mut_only(options, name)
        .expect("hook option vanished")
        .hook_fired(now);
    let entry = runtime
        .options()
        .get_only(options, name)
        .expect("hook option vanished");
    enum Value {
        Text(ByteString),
        Commands(Rc<CommandList>),
    }
    let mut values = Vec::new();
    let user = name.first() == Some(&b'@');
    if user {
        values.push(Value::Text(ByteString::from(
            runtime.options().get_string(options, name),
        )));
    } else {
        for (_, item) in entry.array_items() {
            match item.value() {
                OptionsValue::Command(Some(list)) if !expand => {
                    values.push(Value::Commands(Rc::clone(list)))
                }
                OptionsValue::String(value) if expand => values.push(Value::Text(value.clone())),
                _ => {}
            }
        }
    }
    let event = after.map(|item| runtime.item_event(item));
    let state = runtime.new_hook_state(
        &target,
        event.as_ref(),
        QueueStateFlags::NOHOOKS,
        &payload.formats,
    );
    let mut anchor = after;
    for value in values {
        let list = match value {
            Value::Commands(list) => list,
            Value::Text(value) => {
                let value = if expand {
                    runtime.expand_hook(&value, &target, payload.client, &payload.formats)
                } else {
                    value
                };
                match runtime.parse_from_string(&value) {
                    Ok(list) => list,
                    Err(error) => {
                        runtime.hook_parse_error(anchor, error.message(), user);
                        continue;
                    }
                }
            }
        };
        anchor = Some(runtime.insert_hook_commands(anchor, list, state));
    }
    runtime.free_hook_state(state);
}

pub fn insert_event(
    runtime: &mut dyn HooksRuntime,
    after: Option<QueueItemId>,
    name: &[u8],
    payload: &HookPayload,
    explicit: Option<OptionsId>,
    expand: bool,
) {
    if after.is_some_and(|item| runtime.item_flags(item).contains(QueueStateFlags::NOHOOKS)) {
        return;
    }
    let mut payload = payload.clone();
    let mut formats = after.map_or_else(BTreeMap::new, |item| runtime.item_formats(item));
    for (key, value) in std::mem::take(&mut payload.formats) {
        let mut prefixed = b"hook_".to_vec();
        prefixed.extend_from_slice(&key);
        formats.insert(prefixed.into(), value);
    }
    formats.insert("hook".into(), name.into());
    payload.formats = formats;
    insert(runtime, after, name, &payload, explicit, expand);
}
pub fn event(runtime: &mut dyn HooksRuntime, name: &[u8], payload: &HookPayload) {
    if payload.monitor.is_some() {
        return;
    }
    if let Some(item) = payload.item {
        insert_event(runtime, Some(item), name, payload, None, false);
    } else if !runtime
        .global_running()
        .is_some_and(|item| runtime.item_flags(item).contains(QueueStateFlags::NOHOOKS))
    {
        insert_event(runtime, None, name, payload, None, false);
    }
}
pub fn run(runtime: &mut dyn HooksRuntime, item: QueueItemId, name: &[u8]) {
    let mut payload = HookPayload {
        target: Some(runtime.item_target(item)),
        client: runtime.item_client(item),
        ..Default::default()
    };
    payload.formats.insert("hook".into(), name.into());
    insert(runtime, Some(item), name, &payload, None, false);
}
pub fn add_event(runtime: &mut dyn HooksRuntime, name: &[u8]) {
    if is_event(runtime.hooks(), name) {
        return;
    }
    let sink = runtime.add_hook_sink(name, None);
    runtime.hooks_mut().events.push(HooksEvent {
        name: name.into(),
        sink,
    });
}
pub fn is_event(store: &HooksStore, name: &[u8]) -> bool {
    store.events.iter().any(|event| event.name.as_ref() == name)
}
pub fn valid_event_name(name: &[u8]) -> bool {
    name.first() == Some(&b'@') || crate::options::search(name).is_some_and(|entry| entry.is_hook())
}
pub fn build_events(runtime: &mut dyn HooksRuntime) {
    for entry in crate::options::table::OPTIONS_TABLE {
        if entry.is_hook() {
            add_event(runtime, entry.name);
        }
    }
}
pub fn monitor_free(runtime: &mut dyn HooksRuntime, id: HooksMonitorId) {
    let Some(monitor) = runtime
        .hooks()
        .monitors
        .get(id)
        .and_then(Option::as_ref)
        .cloned()
    else {
        return;
    };
    runtime.remove_hook_sink(monitor.sink);
    runtime.destroy_monitor_set(monitor.set);
    runtime
        .hooks_mut()
        .monitors
        .request_remove(id)
        .expect("live hook monitor");
}
pub fn monitor_remove(runtime: &mut dyn HooksRuntime, options: OptionsId, name: &[u8]) {
    let monitor = runtime
        .options()
        .get_only(options, name)
        .and_then(|entry| entry.monitor());
    if let Some(monitor) = monitor {
        runtime
            .options_mut()
            .get_mut_only(options, name)
            .expect("monitor option")
            .set_monitor(None);
        monitor_free(runtime, monitor);
    }
}
pub fn monitor_event(
    runtime: &mut dyn HooksRuntime,
    id: HooksMonitorId,
    name: &[u8],
    payload: &HookPayload,
) {
    if payload.monitor != Some(id) {
        return;
    }
    let Some(options) = runtime
        .hooks()
        .monitors
        .get(id)
        .and_then(Option::as_ref)
        .map(|monitor| monitor.options)
    else {
        return;
    };
    let running = runtime.global_running();
    insert_event(runtime, running, name, payload, Some(options), true);
}
pub fn monitor_change(
    runtime: &mut dyn HooksRuntime,
    id: HooksMonitorId,
    change: &MonitorChange<'_>,
) {
    let Some(saved) = runtime
        .hooks()
        .monitors
        .get(id)
        .and_then(Option::as_ref)
        .map(|monitor| monitor.target)
    else {
        return;
    };
    let mut formats = runtime.monitor_formats(change);
    formats.insert("value".into(), change.value.unwrap_or_default().into());
    formats.insert("last".into(), change.last.unwrap_or_default().into());
    let target = monitor_target(runtime.model(), change, &saved);
    let payload = HookPayload {
        target: Some(target),
        monitor: Some(id),
        client: change.client,
        formats,
        ..Default::default()
    };
    runtime.fire_hook_event(change.name, payload);
}
pub struct MonitorSpec<'a> {
    pub options: OptionsId,
    pub name: &'a [u8],
    pub kind: MonitorType,
    pub public_id: i32,
    pub format: &'a [u8],
    pub flags: MonitorFlags,
    pub target: &'a CmdFindState,
    pub session: Option<SessionId>,
}
pub fn monitor_add(
    runtime: &mut dyn HooksRuntime,
    spec: MonitorSpec<'_>,
) -> Result<HooksMonitorId, ArenaError> {
    monitor_remove(runtime, spec.options, spec.name);
    runtime.ensure_monitor_option(spec.options, spec.name);
    // Reserve the id before the model can synchronously emit its initial change.
    let id = runtime.hooks_mut().monitors.insert(None)?;
    let set = runtime.create_monitor_set(spec.session, id);
    let sink = runtime.add_hook_sink(spec.name, Some(id));
    let mut target = CmdFindState::default();
    target.copy_target_from(spec.target);
    *runtime
        .hooks_mut()
        .monitors
        .get_mut(id)
        .expect("reserved hook monitor") = Some(HooksMonitor {
        options: spec.options,
        set,
        sink,
        target,
        kind: spec.kind,
        public_id: spec.public_id,
        format: spec.format.into(),
    });
    runtime
        .options_mut()
        .get_mut_only(spec.options, spec.name)
        .expect("monitor option")
        .set_monitor(Some(id));
    runtime.add_model_monitor(
        set,
        spec.name,
        spec.kind,
        spec.public_id,
        spec.format,
        spec.flags,
    );
    Ok(id)
}
pub fn monitor_to_string(name: &[u8], monitor: &HooksMonitor) -> ByteString {
    let target = match monitor.kind {
        MonitorType::Session => String::new(),
        MonitorType::Pane => format!("%{}", monitor.public_id),
        MonitorType::AllPanes => "%*".into(),
        MonitorType::Window => format!("@{}", monitor.public_id),
        MonitorType::AllWindows => "@*".into(),
    };
    let mut out = name.to_vec();
    out.push(b':');
    out.extend_from_slice(target.as_bytes());
    out.push(b':');
    out.extend_from_slice(&monitor.format);
    out.into()
}
pub fn monitor_get_fire_count(runtime: &dyn HooksRuntime, id: HooksMonitorId, name: &[u8]) -> u32 {
    runtime
        .hooks()
        .monitors
        .get(id)
        .and_then(Option::as_ref)
        .map_or(0, |monitor| {
            runtime.model_monitor_stats(monitor.set, name).0
        })
}
pub fn monitor_get_fire_time(runtime: &dyn HooksRuntime, id: HooksMonitorId, name: &[u8]) -> i64 {
    runtime
        .hooks()
        .monitors
        .get(id)
        .and_then(Option::as_ref)
        .map_or(0, |monitor| {
            runtime.model_monitor_stats(monitor.set, name).1
        })
}

pub fn monitor_target(
    model: &dyn ModelView,
    change: &MonitorChange<'_>,
    saved: &CmdFindState,
) -> CmdFindState {
    if let Some(winlink) = change.winlink {
        if let Some(pane) = change.pane.filter(|&pane| {
            model
                .pane(pane)
                .zip(model.winlink(winlink))
                .is_some_and(|(pane, link)| pane.window == link.window)
        }) {
            return find::from_winlink_pane(model, winlink, pane, CmdFindFlags::default());
        }
        return find::from_winlink(model, winlink, CmdFindFlags::default());
    }
    if let Some(target) = change
        .pane
        .and_then(|pane| find::from_pane(model, pane, CmdFindFlags::default()))
    {
        return target;
    }
    if let Some(session) = change.session {
        return find::from_session(model, session, CmdFindFlags::default());
    }
    let mut target = CmdFindState::default();
    target.copy_target_from(saved);
    target
}

pub fn remove_option(runtime: &mut dyn HooksRuntime, options: OptionsId, name: &[u8]) {
    let Some(token) = runtime.options_mut().prepare_removal(options, name) else {
        return;
    };
    if let Some(monitor) = token.monitor {
        monitor_free(runtime, monitor);
    }
    runtime.options_mut().finish_removal(token);
}
pub fn monitor_get(store: &HooksStore, id: HooksMonitorId) -> Option<(MonitorType, i32, &[u8])> {
    let monitor = store.monitors.get(id)?.as_ref()?;
    Some((monitor.kind, monitor.public_id, monitor.format.as_ref()))
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
