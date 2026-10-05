// Ported from tmux cmd-queue.c, tmux.h @ 8f25579c
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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CmdReturn {
    Stop = 2,
    Error = -1,
    Normal = 0,
    Wait = 1,
}
impl TryFrom<i32> for CmdReturn {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            2 => Ok(Self::Stop),
            -1 => Ok(Self::Error),
            0 => Ok(Self::Normal),
            1 => Ok(Self::Wait),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct QueueStateFlags(pub u32);
impl QueueStateFlags {
    pub const REPEAT: Self = Self(1);
    pub const CONTROL: Self = Self(2);
    pub const NOHOOKS: Self = Self(4);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for QueueStateFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for QueueStateFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for QueueStateFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use super::find::{self, CmdFindFlags, CmdFindState, FindContext, ModelView, MouseInput};
use super::{Command, CommandEntryFlag, CommandFlags, CommandList};
use crate::client::ClientFlags;
use crate::ids::{Arena, ArenaError, ArenaId, ClientId, QueueItemId, QueueStateId};
use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, SpecialKey};
use std::any::Any;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueItemFlags(pub u32);
impl QueueItemFlags {
    pub const FIRED: Self = Self(1);
    pub const WAITING: Self = Self(2);
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueEvent {
    pub key: KeyCode,
    pub mouse: MouseInput,
}
impl Default for QueueEvent {
    fn default() -> Self {
        Self {
            key: KeyCode(SpecialKey::NONE),
            mouse: MouseInput::default(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct QueueState {
    pub flags: QueueStateFlags,
    pub event: QueueEvent,
    pub current: CmdFindState,
    pub formats: BTreeMap<ByteString, ByteString>,
}
pub type QueueCallback = Box<dyn FnOnce(&mut dyn QueueRuntime, QueueItemId) -> CmdReturn>;
pub fn callback_for<R: QueueRuntime + 'static>(
    callback: impl FnOnce(&mut R, QueueItemId) -> CmdReturn + 'static,
) -> QueueCallback {
    Box::new(move |runtime, item| {
        callback(
            runtime
                .as_any_mut()
                .downcast_mut::<R>()
                .expect("queue callback runtime type"),
            item,
        )
    })
}
pub enum QueueItemKind {
    Command {
        list: Rc<CommandList>,
        index: usize,
    },
    Callback {
        name: &'static str,
        cb: Option<QueueCallback>,
    },
}
pub struct QueueItem {
    pub name: ByteString,
    pub kind: QueueItemKind,
    pub group: u32,
    pub state: QueueStateId,
    pub client: Option<ClientId>,
    pub target_client: Option<ClientId>,
    pub flags: QueueItemFlags,
    pub time: i64,
    pub number: u32,
    pub source: CmdFindState,
    pub target: CmdFindState,
    pub owner: Option<Option<ClientId>>,
    pub prev: Option<QueueItemId>,
    pub next: Option<QueueItemId>,
}
impl QueueItem {
    pub fn command(&self) -> Option<&Command> {
        match &self.kind {
            QueueItemKind::Command { list, index } => list.commands.get(*index),
            QueueItemKind::Callback { .. } => None,
        }
    }
}
#[derive(Debug)]
pub struct QueueBatch {
    pub items: Vec<QueueItemId>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandQueue {
    pub head: Option<QueueItemId>,
    pub tail: Option<QueueItemId>,
    pub running: Option<QueueItemId>,
    dispatching: bool,
}
#[derive(Default)]
pub struct QueueStore {
    pub items: Arena<QueueItem, QueueItemId>,
    pub states: Arena<QueueState, QueueStateId>,
    pub global: CommandQueue,
    pub clients: BTreeMap<ClientId, CommandQueue>,
    pub next_number: u32,
}
impl QueueStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn queue(&self, client: Option<ClientId>) -> Option<&CommandQueue> {
        match client {
            Some(c) => self.clients.get(&c),
            None => Some(&self.global),
        }
    }
    pub fn queue_mut(&mut self, client: Option<ClientId>) -> &mut CommandQueue {
        match client {
            Some(c) => self.clients.entry(c).or_default(),
            None => &mut self.global,
        }
    }
    fn insert_state(&mut self, state: QueueState) -> Result<QueueStateId, ArenaError> {
        let id = self.states.insert(state)?;
        self.states.retain(id)?;
        self.states.request_remove(id)?;
        Ok(id)
    }
    pub fn new_state(
        &mut self,
        model: &dyn ModelView,
        current: Option<&CmdFindState>,
        event: Option<&QueueEvent>,
        flags: QueueStateFlags,
    ) -> Result<QueueStateId, ArenaError> {
        let mut target = CmdFindState::default();
        if let Some(current) = current.filter(|c| c.is_valid(model)) {
            target.copy_target_from(current);
        }
        self.insert_state(QueueState {
            flags,
            current: target,
            event: event.copied().unwrap_or_default(),
            formats: BTreeMap::new(),
        })
    }
    pub fn copy_state(
        &mut self,
        model: &dyn ModelView,
        id: QueueStateId,
        current: Option<&CmdFindState>,
    ) -> Result<QueueStateId, ArenaError> {
        let state = self.states.get(id).ok_or(ArenaError::StaleId)?;
        let (event, flags, target) = (
            state.event,
            state.flags,
            current.copied().unwrap_or(state.current),
        );
        self.new_state(model, Some(&target), Some(&event), flags)
    }
    pub fn link_state(&mut self, id: QueueStateId) -> Result<QueueStateId, ArenaError> {
        self.states.retain(id)?;
        Ok(id)
    }
    pub fn free_state(&mut self, id: QueueStateId) -> Result<(), ArenaError> {
        self.states.release(id)?;
        Ok(())
    }
    pub fn add_format(
        &mut self,
        id: QueueStateId,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), ArenaError> {
        self.states
            .get_mut(id)
            .ok_or(ArenaError::StaleId)?
            .formats
            .insert(ByteString::from(key), ByteString::from(value));
        Ok(())
    }
    pub fn add_formats(
        &mut self,
        id: QueueStateId,
        formats: &BTreeMap<ByteString, ByteString>,
    ) -> Result<(), ArenaError> {
        self.states
            .get_mut(id)
            .ok_or(ArenaError::StaleId)?
            .formats
            .extend(formats.iter().map(|(k, v)| (k.clone(), v.clone())));
        Ok(())
    }
    pub fn merge_formats(
        &self,
        id: QueueItemId,
        formats: &mut BTreeMap<ByteString, ByteString>,
    ) -> Result<(), ArenaError> {
        let item = self.items.get(id).ok_or(ArenaError::StaleId)?;
        if let Some(cmd) = item.command() {
            formats.insert(
                ByteString::from("command"),
                ByteString::from(cmd.entry.name),
            );
        }
        let state = self.states.get(item.state).ok_or(ArenaError::StaleId)?;
        formats.extend(state.formats.iter().map(|(k, v)| (k.clone(), v.clone())));
        Ok(())
    }
    fn empty_state(&mut self) -> Result<QueueStateId, ArenaError> {
        self.insert_state(QueueState {
            flags: QueueStateFlags::default(),
            event: QueueEvent::default(),
            current: CmdFindState::default(),
            formats: BTreeMap::new(),
        })
    }
    fn make_item(
        &mut self,
        kind: QueueItemKind,
        group: u32,
        state: QueueStateId,
        name: &[u8],
    ) -> Result<QueueItemId, ArenaError> {
        let id = self.items.insert(QueueItem {
            name: ByteString::new(),
            kind,
            group,
            state,
            client: None,
            target_client: None,
            flags: QueueItemFlags::default(),
            time: 0,
            number: 0,
            source: CmdFindState::default(),
            target: CmdFindState::default(),
            owner: None,
            prev: None,
            next: None,
        })?;
        let (slot, generation) = id.parts();
        let mut full_name = b"[".to_vec();
        full_name.extend_from_slice(name);
        full_name.extend_from_slice(format!("/{slot:x}.{generation:x}]").as_bytes());
        self.items.get_mut(id).expect("new item").name = ByteString(full_name);
        Ok(id)
    }
    pub fn get_command(
        &mut self,
        list: Rc<CommandList>,
        state: Option<QueueStateId>,
    ) -> Result<QueueBatch, ArenaError> {
        if list.commands.is_empty() {
            return self.get_callback("cmdq_empty_command", Box::new(|_, _| CmdReturn::Normal));
        }
        let created = state.is_none();
        let state = match state {
            Some(s) => s,
            None => self.empty_state()?,
        };
        let mut batch = QueueBatch {
            items: Vec::with_capacity(list.commands.len()),
        };
        for (index, cmd) in list.commands.iter().enumerate() {
            if let Err(error) = self.states.retain(state) {
                self.discard_batch(batch)?;
                if created {
                    self.free_state(state)?;
                }
                return Err(error);
            }
            match self.make_item(
                QueueItemKind::Command {
                    list: Rc::clone(&list),
                    index,
                },
                cmd.group,
                state,
                cmd.entry.name,
            ) {
                Ok(id) => batch.items.push(id),
                Err(error) => {
                    self.free_state(state)?;
                    self.discard_batch(batch)?;
                    if created {
                        self.free_state(state)?;
                    }
                    return Err(error);
                }
            }
        }
        if created {
            self.free_state(state)?;
        }
        Ok(batch)
    }
    pub fn get_callback(
        &mut self,
        name: &'static str,
        cb: QueueCallback,
    ) -> Result<QueueBatch, ArenaError> {
        let state = self.empty_state()?;
        match self.make_item(
            QueueItemKind::Callback { name, cb: Some(cb) },
            0,
            state,
            name.as_bytes(),
        ) {
            Ok(id) => Ok(QueueBatch { items: vec![id] }),
            Err(error) => {
                self.free_state(state)?;
                Err(error)
            }
        }
    }
    pub fn get_error(&mut self, message: &[u8]) -> Result<QueueBatch, ArenaError> {
        let message = ByteString::from(message);
        self.get_callback(
            "cmdq_error_callback",
            Box::new(move |runtime, item| {
                error(runtime, item, &message);
                CmdReturn::Normal
            }),
        )
    }
    pub fn discard_batch(&mut self, batch: QueueBatch) -> Result<(), ArenaError> {
        for id in batch.items {
            let item = self.items.get(id).ok_or(ArenaError::StaleId)?;
            assert!(item.owner.is_none(), "discarding queued batch");
            let state = item.state;
            self.items.request_remove(id)?;
            self.free_state(state)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlGuard {
    Begin,
    End,
    Error,
}
pub struct QueueClientView<'a> {
    pub name: &'a [u8],
    pub flags: ClientFlags,
    pub peer_uid: Option<u32>,
}
#[derive(Clone, Debug)]
pub struct QueueHookPayload {
    pub item: QueueItemId,
    pub current: Option<CmdFindState>,
    pub formats: BTreeMap<ByteString, ByteString>,
}
/// Host owns client leases, execution, model snapshots, locale and synchronous events/output.
pub trait QueueRuntime {
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn store(&self) -> &QueueStore;
    fn store_mut(&mut self) -> &mut QueueStore;
    fn model(&self) -> &dyn ModelView;
    fn client_view(&self, client: ClientId) -> Option<QueueClientView<'_>>;
    fn retain_client(&mut self, client: ClientId) -> Result<(), ArenaError>;
    fn release_client(&mut self, client: ClientId);
    fn config_finished(&self) -> bool;
    fn now(&self) -> i64;
    fn server_uid(&self) -> u32;
    fn user_name(&mut self, uid: u32) -> Option<ByteString>;
    fn uppercase(&self, byte: u8) -> u8;
    fn execute(&mut self, command: &Command, item: QueueItemId) -> CmdReturn;
    fn fire_event(&mut self, name: &[u8], payload: QueueHookPayload);
    fn message(&mut self, message: &[u8]);
    fn config_cause(&mut self, message: &[u8]);
    fn client_print(&mut self, client: Option<ClientId>, parse: bool, data: &[u8]);
    fn control_guard(
        &mut self,
        client: ClientId,
        guard: ControlGuard,
        time: i64,
        number: u32,
        flags: u32,
    );
    fn control_write(&mut self, client: ClientId, message: &[u8]);
    fn file_error(&mut self, client: ClientId, message: &[u8]);
    fn status_message(&mut self, client: ClientId, message: &[u8]);
    fn set_exit_status(&mut self, client: ClientId, status: i32);
}

fn enqueue(
    runtime: &mut dyn QueueRuntime,
    owner: Option<ClientId>,
    client: Option<ClientId>,
    anchor: Option<QueueItemId>,
    batch: QueueBatch,
) -> Result<QueueItemId, ArenaError> {
    let last = *batch.items.last().ok_or(ArenaError::StaleId)?;
    for id in &batch.items {
        let item = runtime.store().items.get(*id).ok_or(ArenaError::StaleId)?;
        assert!(item.owner.is_none(), "item already queued");
    }
    if let Some(client) = client {
        for (retained, _) in batch.items.iter().enumerate() {
            if let Err(error) = runtime.retain_client(client) {
                for _ in 0..retained {
                    runtime.release_client(client);
                }
                runtime.store_mut().discard_batch(batch)?;
                return Err(error);
            }
        }
    }
    let store = runtime.store_mut();
    let mut after = anchor.or_else(|| store.queue(owner).and_then(|q| q.tail));
    for id in batch.items {
        let next = after.and_then(|a| store.items.get(a)).and_then(|a| a.next);
        let item = store.items.get_mut(id).expect("batch item");
        item.owner = Some(owner);
        item.client = client;
        item.prev = after;
        item.next = next;
        if let Some(a) = after {
            store.items.get_mut(a).expect("anchor").next = Some(id);
        } else {
            store.queue_mut(owner).head = Some(id);
        }
        if let Some(n) = next {
            store.items.get_mut(n).expect("successor").prev = Some(id);
        } else {
            store.queue_mut(owner).tail = Some(id);
        }
        after = Some(id);
    }
    Ok(last)
}
pub fn append(
    runtime: &mut dyn QueueRuntime,
    client: Option<ClientId>,
    batch: QueueBatch,
) -> Result<QueueItemId, ArenaError> {
    enqueue(runtime, client, client, None, batch)
}
pub fn insert_after(
    runtime: &mut dyn QueueRuntime,
    after: QueueItemId,
    batch: QueueBatch,
) -> Result<QueueItemId, ArenaError> {
    let (owner, client) = runtime
        .store()
        .items
        .get(after)
        .and_then(|i| i.owner.map(|owner| (owner, i.client)))
        .ok_or(ArenaError::StaleId)?;
    enqueue(runtime, owner, client, Some(after), batch)
}
pub fn continue_item(store: &mut QueueStore, id: QueueItemId) {
    if let Some(item) = store.items.get_mut(id) {
        item.flags.remove(QueueItemFlags::WAITING);
    }
}
pub fn running(store: &QueueStore, client: Option<ClientId>) -> Option<QueueItemId> {
    store.queue(client)?.running.filter(|id| {
        store
            .items
            .get(*id)
            .is_some_and(|i| !i.flags.contains(QueueItemFlags::WAITING))
    })
}
fn remove(runtime: &mut dyn QueueRuntime, id: QueueItemId) {
    let store = runtime.store_mut();
    let Some(item) = store.items.get(id) else {
        return;
    };
    let (Some(owner), prev, next, client, state) =
        (item.owner, item.prev, item.next, item.client, item.state)
    else {
        return;
    };
    if let Some(prev) = prev {
        store
            .items
            .get_mut(prev)
            .expect("previous queued item")
            .next = next;
    } else {
        store.queue_mut(owner).head = next;
    }
    if let Some(next) = next {
        store.items.get_mut(next).expect("next queued item").prev = prev;
    } else {
        store.queue_mut(owner).tail = prev;
    }
    store.items.get_mut(id).expect("removed item").owner = None;
    store.items.request_remove(id).expect("live item");
    store.free_state(state).expect("item state lease");
    if let Some(client) = client {
        runtime.release_client(client);
    }
}
fn remove_group(runtime: &mut dyn QueueRuntime, id: QueueItemId) {
    let Some(item) = runtime.store().items.get(id) else {
        return;
    };
    let group = item.group;
    if group == 0 {
        return;
    }
    let mut cursor = item.next;
    while let Some(id) = cursor {
        let item = runtime.store().items.get(id).expect("queued successor");
        cursor = item.next;
        if item.group == group {
            remove(runtime, id);
        }
    }
}
pub fn next(runtime: &mut dyn QueueRuntime, client: Option<ClientId>) -> u32 {
    let Some(queue) = runtime.store().queue(client) else {
        return 0;
    };
    if queue.dispatching
        || queue.head.is_none()
        || queue.head.is_some_and(|id| {
            runtime
                .store()
                .items
                .get(id)
                .is_some_and(|i| i.flags.contains(QueueItemFlags::WAITING))
        })
    {
        return 0;
    }
    runtime.store_mut().queue_mut(client).dispatching = true;
    let mut count: u32 = 0;
    loop {
        let head = runtime.store().queue(client).and_then(|q| q.head);
        runtime.store_mut().queue_mut(client).running = head;
        let Some(id) = head else { break };
        let item = runtime.store().items.get(id).expect("queue head");
        if item.flags.contains(QueueItemFlags::WAITING) {
            break;
        }
        if !item.flags.contains(QueueItemFlags::FIRED) {
            let time = runtime.now();
            let store = runtime.store_mut();
            store.next_number = store.next_number.wrapping_add(1);
            let item = store.items.get_mut(id).expect("head");
            item.time = time;
            item.number = store.next_number;
            store.items.retain(id).expect("dispatch lease");
            let command = match &store.items.get(id).expect("head").kind {
                QueueItemKind::Command { list, index } => Some((Rc::clone(list), *index)),
                _ => None,
            };
            let retval = if let Some((list, index)) = command {
                let retval = fire_command(runtime, id, &list.commands[index]);
                if retval == CmdReturn::Error {
                    remove_group(runtime, id);
                }
                retval
            } else {
                let callback = match &mut runtime
                    .store_mut()
                    .items
                    .get_mut(id)
                    .expect("callback item")
                    .kind
                {
                    QueueItemKind::Callback { cb, .. } => cb.take().expect("callback fired once"),
                    _ => unreachable!(),
                };
                callback(runtime, id)
            };
            if let Some(item) = runtime.store_mut().items.get_mut(id) {
                item.flags.insert(QueueItemFlags::FIRED);
                if retval == CmdReturn::Wait {
                    item.flags.insert(QueueItemFlags::WAITING);
                }
            }
            runtime
                .store_mut()
                .items
                .release(id)
                .expect("dispatch lease");
            if retval == CmdReturn::Wait {
                break;
            }
            count = count.wrapping_add(1);
        }
        remove(runtime, id);
    }
    runtime.store_mut().queue_mut(client).dispatching = false;
    count
}

fn find_context(runtime: &dyn QueueRuntime, id: QueueItemId) -> FindContext {
    let item = runtime.store().items.get(id).expect("live queue item");
    let state = runtime.store().states.get(item.state).expect("item state");
    FindContext {
        client: item.client,
        current: state.current,
        mouse: state.event.mouse,
    }
}
fn find_flag(
    runtime: &mut dyn QueueRuntime,
    id: QueueItemId,
    cmd: &Command,
    flag: CommandEntryFlag,
    source: bool,
) -> bool {
    let mut diagnostic = None;
    let result = if flag.flag == 0 {
        let tc = runtime
            .store()
            .items
            .get(id)
            .expect("live item")
            .target_client;
        Ok(find::from_client(runtime.model(), tc, CmdFindFlags::default()).unwrap_or_default())
    } else {
        find::target_with_error(
            runtime.model(),
            &find_context(runtime, id),
            cmd.args.get(flag.flag),
            flag.kind,
            flag.flags,
            &mut |message| diagnostic = Some(ByteString::from(message)),
        )
    };
    if let Some(message) = diagnostic {
        error(runtime, id, &message);
    }
    match result {
        Ok(state) => {
            let item = runtime.store_mut().items.get_mut(id).expect("item");
            if source {
                item.source = state;
            } else {
                item.target = state;
            }
            true
        }
        Err(_) => {
            let item = runtime.store_mut().items.get_mut(id).expect("item");
            if source {
                item.source = CmdFindState::default();
            } else {
                item.target = CmdFindState::default();
            }
            false
        }
    }
}
fn hook_target(runtime: &dyn QueueRuntime, id: QueueItemId) -> Option<CmdFindState> {
    let item = runtime.store().items.get(id)?;
    if item.target.is_valid(runtime.model()) {
        return Some(item.target);
    }
    let current = runtime.store().states.get(item.state)?.current;
    if current.is_valid(runtime.model()) {
        return Some(current);
    }
    find::from_client(runtime.model(), item.client, CmdFindFlags::default())
}
fn add_message(runtime: &mut dyn QueueRuntime, id: QueueItemId, cmd: &Command) {
    let item = runtime.store().items.get(id).expect("item");
    let key = runtime
        .store()
        .states
        .get(item.state)
        .expect("state")
        .event
        .key;
    let c = item.client;
    let mut text = Vec::new();
    if let Some(c) = c {
        let Some(v) = runtime.client_view(c) else {
            return;
        };
        text.extend_from_slice(v.name);
        let uid = v.peer_uid.filter(|uid| *uid != runtime.server_uid());
        if let Some(uid) = uid {
            let user = runtime
                .user_name(uid)
                .unwrap_or_else(|| ByteString::from("unknown"));
            text.push(b'[');
            text.extend_from_slice(&user);
            text.push(b']');
        }
        if runtime
            .model()
            .client(c)
            .is_some_and(|c| c.session.is_some())
            && key != KeyCode(SpecialKey::NONE)
        {
            text.extend_from_slice(b" key ");
            let mut name = [0; rmux_tty::key_string::NAME_SIZE];
            let len = rmux_tty::key_string::write_key_name(key, false, &mut name);
            text.extend_from_slice(&name[..len]);
            text.extend_from_slice(b": ");
        } else {
            text.extend_from_slice(b" command: ");
        }
    } else {
        text.extend_from_slice(b"command: ");
    }
    text.extend_from_slice(&cmd.print());
    runtime.message(&text);
}
fn fire_command(runtime: &mut dyn QueueRuntime, id: QueueItemId, cmd: &Command) -> CmdReturn {
    let saved = runtime.store().items.get(id).expect("item").client;
    if saved.is_some_and(|c| {
        runtime
            .client_view(c)
            .is_some_and(|c| c.flags.contains(ClientFlags::DEAD))
    }) {
        return CmdReturn::Error;
    }
    if runtime.config_finished() {
        add_message(runtime, id, cmd);
    }
    let state = runtime.store().items.get(id).expect("item").state;
    let guard_flags = u32::from(
        runtime
            .store()
            .states
            .get(state)
            .expect("state")
            .flags
            .contains(QueueStateFlags::CONTROL),
    );
    guard(runtime, id, ControlGuard::Begin, guard_flags);
    if saved.is_none() {
        let fallback = find::client(runtime.model(), None, None, true)
            .ok()
            .flatten();
        runtime.store_mut().items.get_mut(id).expect("item").client = fallback;
    }
    let quiet = cmd.entry.flags.contains(CommandFlags::CLIENT_CANFAIL);
    let explicit = cmd
        .entry
        .flags
        .intersects(CommandFlags::CLIENT_CFLAG | CommandFlags::CLIENT_TFLAG);
    let target = if cmd.entry.flags.contains(CommandFlags::CLIENT_CFLAG) {
        cmd.args.get(b'c')
    } else if cmd.entry.flags.contains(CommandFlags::CLIENT_TFLAG) {
        cmd.args.get(b't')
    } else {
        None
    };
    let current = runtime.store().items.get(id).expect("item").client;
    let client = find::client(
        runtime.model(),
        current,
        target,
        if explicit { quiet } else { true },
    );
    let mut retval = CmdReturn::Error;
    match client {
        Err(cause) => error(runtime, id, &cause),
        Ok(tc) => {
            if !explicit || tc.is_some() || quiet {
                runtime
                    .store_mut()
                    .items
                    .get_mut(id)
                    .expect("item")
                    .target_client = tc;
                if find_flag(runtime, id, cmd, cmd.entry.source, true)
                    && find_flag(runtime, id, cmd, cmd.entry.target, false)
                {
                    retval = runtime.execute(cmd, id);
                    if retval != CmdReturn::Error
                        && cmd.entry.flags.contains(CommandFlags::AFTERHOOK)
                        && let Some(target) = hook_target(runtime, id)
                    {
                        let mut name = b"after-".to_vec();
                        name.extend_from_slice(cmd.entry.name);
                        insert_hook(runtime, id, Some(&target), &name);
                    }
                }
            }
        }
    }
    runtime
        .store_mut()
        .items
        .get_mut(id)
        .expect("item retained during dispatch")
        .client = saved;
    if retval == CmdReturn::Error {
        let target = hook_target(runtime, id);
        insert_hook(runtime, id, target.as_ref(), b"command-error");
        guard(runtime, id, ControlGuard::Error, guard_flags);
    } else {
        guard(runtime, id, ControlGuard::End, guard_flags);
    }
    retval
}
pub fn insert_hook(
    runtime: &mut dyn QueueRuntime,
    id: QueueItemId,
    current: Option<&CmdFindState>,
    name: &[u8],
) {
    let item = runtime.store().items.get(id).expect("hook item");
    let state = runtime.store().states.get(item.state).expect("hook state");
    if state.flags.contains(QueueStateFlags::NOHOOKS) {
        return;
    }
    let Some(cmd) = item.command() else { return };
    let mut formats = BTreeMap::new();
    formats.insert(ByteString::from("arguments"), cmd.args.print());
    for (i, value) in cmd.args.values.iter().enumerate() {
        formats.insert(
            ByteString::from(format!("argument_{i}")),
            ByteString::from(value.as_string()),
        );
    }
    for (&flag, entry) in &cmd.args.flags {
        formats.insert(
            ByteString::from(format!("flag_{}", char::from(flag))),
            ByteString::from(cmd.args.get(flag).unwrap_or(b"1")),
        );
        for (i, value) in entry.values.iter().enumerate() {
            formats.insert(
                ByteString::from(format!("flag_{}_{i}", char::from(flag))),
                ByteString::from(value.as_string()),
            );
        }
    }
    runtime.fire_event(
        name,
        QueueHookPayload {
            item: id,
            current: current.copied(),
            formats,
        },
    );
}
pub fn guard(runtime: &mut dyn QueueRuntime, id: QueueItemId, guard: ControlGuard, flags: u32) {
    let Some(item) = runtime.store().items.get(id) else {
        return;
    };
    let Some(c) = item.client else { return };
    let (time, number) = (item.time, item.number);
    if runtime
        .client_view(c)
        .is_some_and(|v| v.flags.contains(ClientFlags::CONTROL))
    {
        runtime.control_guard(c, guard, time, number, flags);
    }
}
pub fn print(runtime: &mut dyn QueueRuntime, id: QueueItemId, message: &[u8]) {
    let client = runtime.store().items.get(id).and_then(|item| item.client);
    runtime.client_print(client, true, message);
}
pub fn print_data(
    runtime: &mut dyn QueueRuntime,
    id: QueueItemId,
    buffer: &rmux_util::buffer::ByteBuffer,
) {
    print(runtime, id, buffer.data());
}
pub fn error(runtime: &mut dyn QueueRuntime, id: QueueItemId, message: &[u8]) {
    let Some(item) = runtime.store().items.get(id) else {
        return;
    };
    if let Some(c) = item.client {
        let Some(view) = runtime.client_view(c) else {
            return;
        };
        let flags = view.flags;
        if runtime
            .model()
            .client(c)
            .is_none_or(|v| v.session.is_none())
            || flags.contains(ClientFlags::CONTROL)
        {
            let mut log = view.name.to_vec();
            log.extend_from_slice(b" message: ");
            log.extend_from_slice(message);
            runtime.message(&log);
            let sanitized;
            let message = if flags.contains(ClientFlags::UTF8) {
                message
            } else {
                sanitized = rmux_util::utf8::sanitize(message);
                &sanitized
            };
            if flags.contains(ClientFlags::CONTROL) {
                runtime.control_write(c, message);
            } else {
                let mut text = message.to_vec();
                text.push(b'\n');
                runtime.file_error(c, &text);
            }
            runtime.set_exit_status(c, 1);
        } else {
            let mut text = message.to_vec();
            if let Some(first) = text.first_mut() {
                *first = runtime.uppercase(*first);
            }
            runtime.status_message(c, &text);
        }
    } else {
        let mut text = Vec::new();
        if let Some(cmd) = item.command()
            && let Some(file) = &cmd.file
        {
            text.extend_from_slice(file);
            text.extend_from_slice(format!(":{}: ", cmd.line).as_bytes());
        }
        text.extend_from_slice(message);
        if runtime.config_finished() {
            let mut log = b"message: ".to_vec();
            log.extend_from_slice(&text);
            runtime.message(&log);
        } else {
            runtime.config_cause(&text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::arguments::{Args, ArgsEntryFlags, ArgsParse, ArgsValue};
    use super::super::find::CmdFindType;
    use super::super::find::tests::{Fixture, cid, lid, pid, sid};
    use super::super::{CommandEntry, CommandEntryFlag};
    use super::*;
    use std::collections::VecDeque;

    const NO_TARGET: CommandEntryFlag = CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    };
    static COMMAND: CommandEntry = CommandEntry {
        name: b"work",
        alias: None,
        args: ArgsParse {
            template: b"",
            lower: 0,
            upper: -1,
            cb: None,
        },
        usage: b"",
        source: NO_TARGET,
        target: NO_TARGET,
        flags: CommandFlags(4),
    };
    static SOURCE: CommandEntry = CommandEntry {
        name: b"find",
        alias: None,
        args: ArgsParse {
            template: b"s:t:",
            lower: 0,
            upper: -1,
            cb: None,
        },
        usage: b"",
        source: CommandEntryFlag {
            flag: b's',
            kind: CmdFindType::Pane,
            flags: CmdFindFlags(0),
        },
        target: CommandEntryFlag {
            flag: b't',
            kind: CmdFindType::Window,
            flags: CmdFindFlags(64),
        },
        flags: CommandFlags(0),
    };
    static CLIENT: CommandEntry = CommandEntry {
        name: b"client",
        alias: None,
        args: ArgsParse {
            template: b"c:",
            lower: 0,
            upper: -1,
            cb: None,
        },
        usage: b"",
        source: NO_TARGET,
        target: NO_TARGET,
        flags: CommandFlags(8),
    };
    static TCLIENT: CommandEntry = CommandEntry {
        name: b"target-client",
        alias: None,
        args: ArgsParse {
            template: b"t:",
            lower: 0,
            upper: -1,
            cb: None,
        },
        usage: b"",
        source: NO_TARGET,
        target: NO_TARGET,
        flags: CommandFlags(16),
    };
    static OPTIONAL_CLIENT: CommandEntry = CommandEntry {
        name: b"optional-client",
        alias: None,
        args: ArgsParse {
            template: b"c:",
            lower: 0,
            upper: -1,
            cb: None,
        },
        usage: b"",
        source: NO_TARGET,
        target: NO_TARGET,
        flags: CommandFlags(40),
    };
    fn list(groups: &[u32]) -> Rc<CommandList> {
        Rc::new(CommandList {
            group: 0,
            commands: groups
                .iter()
                .map(|group| Command {
                    entry: &COMMAND,
                    args: Args::default(),
                    group: *group,
                    file: Some(ByteString::from("test.conf")),
                    line: 9,
                    parse_flags: Default::default(),
                })
                .collect(),
        })
    }
    struct Host {
        store: QueueStore,
        model: Fixture,
        flags: Vec<ClientFlags>,
        leases: Vec<i32>,
        finished: bool,
        returns: VecDeque<CmdReturn>,
        trace: Vec<ByteString>,
        events: Vec<(ByteString, QueueHookPayload)>,
        hook_insert: bool,
        recurse: bool,
        outcomes: Vec<(CmdFindState, CmdFindState)>,
        uid: Option<u32>,
    }
    impl Host {
        fn new() -> Self {
            Self {
                store: QueueStore::new(),
                model: Fixture::new(),
                flags: vec![ClientFlags::UTF8; 3],
                leases: vec![0; 3],
                finished: false,
                returns: VecDeque::new(),
                trace: Vec::new(),
                events: Vec::new(),
                hook_insert: false,
                recurse: false,
                outcomes: Vec::new(),
                uid: None,
            }
        }
        fn commands(&mut self, groups: &[u32], owner: Option<ClientId>) -> QueueItemId {
            let batch = self.store.get_command(list(groups), None).unwrap();
            append(self, owner, batch).unwrap()
        }
        fn record(&mut self, prefix: &str, data: &[u8]) {
            let mut text = prefix.as_bytes().to_vec();
            text.extend_from_slice(data);
            self.trace.push(ByteString(text));
        }
    }
    impl QueueRuntime for Host {
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn store(&self) -> &QueueStore {
            &self.store
        }
        fn store_mut(&mut self) -> &mut QueueStore {
            &mut self.store
        }
        fn model(&self) -> &dyn ModelView {
            &self.model
        }
        fn client_view(&self, c: ClientId) -> Option<QueueClientView<'_>> {
            Some(QueueClientView {
                name: self.model.client(c)?.name,
                flags: *self.flags.get(c.parts().0 as usize)?,
                peer_uid: self.uid,
            })
        }
        fn retain_client(&mut self, c: ClientId) -> Result<(), ArenaError> {
            self.leases[c.parts().0 as usize] += 1;
            Ok(())
        }
        fn release_client(&mut self, c: ClientId) {
            self.leases[c.parts().0 as usize] -= 1;
        }
        fn config_finished(&self) -> bool {
            self.finished
        }
        fn now(&self) -> i64 {
            42
        }
        fn server_uid(&self) -> u32 {
            100
        }
        fn user_name(&mut self, _: u32) -> Option<ByteString> {
            Some(ByteString::from("guest"))
        }
        fn uppercase(&self, byte: u8) -> u8 {
            byte.to_ascii_uppercase()
        }
        fn execute(&mut self, _: &Command, id: QueueItemId) -> CmdReturn {
            self.trace.push(ByteString::from("exec"));
            let item = self.store.items.get(id).unwrap();
            self.outcomes.push((item.source, item.target));
            assert_eq!(running(&self.store, item.owner.unwrap()), Some(id));
            assert!(!item.flags.contains(QueueItemFlags::FIRED));
            if self.recurse {
                let owner = item.owner.unwrap();
                assert_eq!(next(self, owner), 0);
            }
            self.returns.pop_front().unwrap_or(CmdReturn::Normal)
        }
        fn fire_event(&mut self, name: &[u8], payload: QueueHookPayload) {
            self.record("event:", name);
            if self.hook_insert && name.starts_with(b"after-") {
                let batch = self
                    .store
                    .get_callback(
                        "hook",
                        callback_for::<Host>(|host, _| {
                            host.trace.push(ByteString::from("hook"));
                            CmdReturn::Normal
                        }),
                    )
                    .unwrap();
                insert_after(self, payload.item, batch).unwrap();
            }
            self.events.push((ByteString::from(name), payload));
        }
        fn message(&mut self, message: &[u8]) {
            self.record("log:", message);
        }
        fn config_cause(&mut self, message: &[u8]) {
            self.record("cause:", message);
        }
        fn client_print(&mut self, _: Option<ClientId>, parse: bool, data: &[u8]) {
            assert!(parse);
            self.record("print:", data);
        }
        fn control_guard(
            &mut self,
            c: ClientId,
            guard: ControlGuard,
            time: i64,
            number: u32,
            flags: u32,
        ) {
            self.trace.push(ByteString::from(format!(
                "guard:{}:{guard:?}:{time}:{number}:{flags}",
                c.parts().0
            )));
        }
        fn control_write(&mut self, _: ClientId, message: &[u8]) {
            self.record("control:", message);
        }
        fn file_error(&mut self, _: ClientId, message: &[u8]) {
            self.record("stderr:", message);
        }
        fn status_message(&mut self, _: ClientId, message: &[u8]) {
            self.record("status:", message);
        }
        fn set_exit_status(&mut self, _: ClientId, status: i32) {
            self.trace.push(ByteString::from(format!("exit:{status}")));
        }
    }
    #[test]
    fn wait_fires_once_resume_counts_stop_and_running() {
        let mut host = Host::new();
        host.returns
            .extend([CmdReturn::Wait, CmdReturn::Stop, CmdReturn::Normal]);
        host.recurse = true;
        host.commands(&[1, 2, 3], Some(cid(0)));
        let first = host.store.queue(Some(cid(0))).unwrap().head.unwrap();
        assert_eq!(next(&mut host, Some(cid(0))), 0);
        assert_eq!(running(&host.store, Some(cid(0))), None);
        let item = host.store.items.get(first).unwrap();
        assert_eq!((item.time, item.number), (42, 1));
        assert!(item.flags.contains(QueueItemFlags::FIRED));
        assert_eq!(next(&mut host, Some(cid(0))), 0);
        continue_item(&mut host.store, first);
        assert_eq!(next(&mut host, Some(cid(0))), 2);
        assert_eq!(
            host.trace
                .iter()
                .filter(|v| v.as_bytes() == b"exec")
                .count(),
            3
        );
        assert!(host.store.items.is_empty());
        assert!(host.store.states.is_empty());
        assert_eq!(host.leases[0], 0);
    }
    #[test]
    fn hooks_precede_next_command_and_wait_stops_insertions() {
        let mut host = Host::new();
        host.hook_insert = true;
        host.returns.push_back(CmdReturn::Wait);
        host.commands(&[1, 1], None);
        assert_eq!(next(&mut host, None), 0);
        assert_eq!(host.events.len(), 1);
        assert_eq!(host.events[0].0, ByteString::from("after-work"));
        assert!(!host.trace.iter().any(|t| t.as_bytes() == b"hook"));
        let first = host.store.global.head.unwrap();
        continue_item(&mut host.store, first);
        assert_eq!(next(&mut host, None), 3);
        assert_eq!(
            host.trace
                .iter()
                .filter(|t| t.as_bytes() == b"exec" || t.as_bytes() == b"hook")
                .map(|t| t.as_bytes())
                .collect::<Vec<_>>(),
            vec![b"exec".as_slice(), b"hook", b"exec", b"hook"]
        );
    }
    #[test]
    fn inserted_items_take_the_anchor_client_not_the_queue_owner() {
        let mut host = Host::new();
        host.hook_insert = true;
        host.returns.push_back(CmdReturn::Wait);
        let fallback = find::client(&host.model, None, None, true)
            .unwrap()
            .expect("fixture has an attached client");
        let slot = fallback.parts().0 as usize;
        host.commands(&[1], None);
        assert_eq!(next(&mut host, None), 0);
        let first = host.store.global.head.unwrap();
        assert_eq!(host.store.items.get(first).unwrap().client, None);
        let hook = host.store.items.get(first).unwrap().next.unwrap();
        let item = host.store.items.get(hook).unwrap();
        assert_eq!(item.owner, Some(None));
        assert_eq!(item.client, Some(fallback));
        assert_eq!(host.leases[slot], 1);
        continue_item(&mut host.store, first);
        assert_eq!(next(&mut host, None), 1);
        assert!(host.store.items.is_empty());
        assert_eq!(host.leases[slot], 0);
    }
    #[test]
    fn error_removes_same_group_past_other_groups_and_dead_callbacks_run() {
        let mut host = Host::new();
        host.returns.push_back(CmdReturn::Error);
        host.commands(&[7, 8, 7, 9, 7], None);
        assert_eq!(next(&mut host, None), 3);
        assert_eq!(
            host.events
                .iter()
                .filter(|(n, _)| n.as_bytes() == b"command-error")
                .count(),
            1
        );
        host.flags[0].insert(ClientFlags::DEAD);
        host.commands(&[1, 1], Some(cid(0)));
        let batch = host
            .store
            .get_callback(
                "on_dead",
                callback_for::<Host>(|host, _| {
                    host.trace.push(ByteString::from("dead callback"));
                    CmdReturn::Normal
                }),
            )
            .unwrap();
        append(&mut host, Some(cid(0)), batch).unwrap();
        let events = host.events.len();
        assert_eq!(next(&mut host, Some(cid(0))), 2);
        assert_eq!(host.events.len(), events);
        assert!(host.trace.contains(&ByteString::from("dead callback")));
        assert_eq!(host.leases[0], 0);
    }
    #[test]
    fn multiple_insertion_links_and_empty_command_list() {
        let mut host = Host::new();
        let anchor = host.commands(&[1], None);
        let a = host
            .store
            .get_callback("a", Box::new(|_, _| CmdReturn::Normal))
            .unwrap();
        let a = insert_after(&mut host, anchor, a).unwrap();
        let b = host
            .store
            .get_callback("b", Box::new(|_, _| CmdReturn::Normal))
            .unwrap();
        let b = insert_after(&mut host, anchor, b).unwrap();
        assert_eq!(host.store.items.get(anchor).unwrap().next, Some(b));
        assert_eq!(host.store.items.get(b).unwrap().prev, Some(anchor));
        assert_eq!(host.store.items.get(b).unwrap().next, Some(a));
        assert_eq!(host.store.items.get(a).unwrap().prev, Some(b));
        assert_eq!(host.store.global.tail, Some(a));
        assert_eq!(next(&mut host, None), 3);
        assert!(host.store.global.head.is_none());
        assert!(host.store.global.tail.is_none());
        let batch = host
            .store
            .get_command(Rc::new(CommandList::default()), None)
            .unwrap();
        append(&mut host, None, batch).unwrap();
        assert_eq!(next(&mut host, None), 1);
    }
    #[test]
    fn state_leases_share_current_and_copy_drops_formats() {
        let mut host = Host::new();
        let current = find::from_session(&host.model, sid(0), CmdFindFlags::QUIET);
        let event = QueueEvent {
            key: KeyCode(120),
            ..QueueEvent::default()
        };
        let state = host
            .store
            .new_state(
                &host.model,
                Some(&current),
                Some(&event),
                QueueStateFlags::REPEAT,
            )
            .unwrap();
        host.store.add_format(state, b"extra", b"value").unwrap();
        let copy = host.store.copy_state(&host.model, state, None).unwrap();
        let copied = host.store.states.get(copy).unwrap();
        assert!(copied.formats.is_empty());
        assert_eq!(copied.event, event);
        assert_eq!(copied.flags, QueueStateFlags::REPEAT);
        assert_eq!(copied.current.flags, CmdFindFlags::default());
        let batch = host.store.get_command(list(&[1, 1]), Some(state)).unwrap();
        let ids = batch.items.clone();
        assert_eq!(
            host.store.items.get(ids[0]).unwrap().state,
            host.store.items.get(ids[1]).unwrap().state
        );
        let new = find::from_winlink_pane(&host.model, lid(1), pid(3), CmdFindFlags::default());
        host.store.states.get_mut(state).unwrap().current = new;
        assert_eq!(
            host.store
                .states
                .get(host.store.items.get(ids[1]).unwrap().state)
                .unwrap()
                .current,
            new
        );
        let mut formats = BTreeMap::new();
        host.store.merge_formats(ids[0], &mut formats).unwrap();
        assert_eq!(
            formats.get(b"command".as_slice()).unwrap().as_bytes(),
            b"work"
        );
        assert_eq!(
            formats.get(b"extra".as_slice()).unwrap().as_bytes(),
            b"value"
        );
        host.store.free_state(state).unwrap();
        host.store.free_state(copy).unwrap();
        append(&mut host, None, batch).unwrap();
        assert_eq!(next(&mut host, None), 2);
        assert!(host.store.states.is_empty());
    }
    #[test]
    fn hook_payload_repeated_flags_and_nohooks() {
        let mut host = Host::new();
        let mut args = Args::default();
        args.set(
            b'e',
            Some(ArgsValue::string(ByteString::from("A=1"))),
            ArgsEntryFlags::default(),
        );
        args.set(
            b'e',
            Some(ArgsValue::string(ByteString::from("B=2"))),
            ArgsEntryFlags::default(),
        );
        args.set(b'd', None, ArgsEntryFlags::default());
        args.values
            .push(ArgsValue::string(ByteString::from("hello")));
        let commands = Rc::new(CommandList {
            group: 1,
            commands: vec![Command {
                entry: &COMMAND,
                args,
                group: 1,
                file: None,
                line: 0,
                parse_flags: Default::default(),
            }],
        });
        let batch = host.store.get_command(Rc::clone(&commands), None).unwrap();
        append(&mut host, None, batch).unwrap();
        next(&mut host, None);
        let formats = &host.events[0].1.formats;
        for (name, value) in [
            (b"flag_d".as_slice(), b"1".as_slice()),
            (b"flag_e", b"B=2"),
            (b"flag_e_0", b"A=1"),
            (b"flag_e_1", b"B=2"),
            (b"argument_0", b"hello"),
        ] {
            assert_eq!(formats.get(name).unwrap().as_bytes(), value);
        }
        let state = host
            .store
            .new_state(&host.model, None, None, QueueStateFlags::NOHOOKS)
            .unwrap();
        let batch = host.store.get_command(commands, Some(state)).unwrap();
        host.store.free_state(state).unwrap();
        append(&mut host, None, batch).unwrap();
        next(&mut host, None);
        assert_eq!(host.events.len(), 1);
    }
    #[test]
    fn target_client_source_errors_and_can_fail_partial_target() {
        let mut host = Host::new();
        let mut args = Args::default();
        args.set(
            b's',
            Some(ArgsValue::string(ByteString::from("%31"))),
            ArgsEntryFlags::default(),
        );
        args.set(
            b't',
            Some(ArgsValue::string(ByteString::from("alpha:nope"))),
            ArgsEntryFlags::default(),
        );
        let commands = Rc::new(CommandList {
            group: 1,
            commands: vec![Command {
                entry: &SOURCE,
                args,
                group: 1,
                file: None,
                line: 0,
                parse_flags: Default::default(),
            }],
        });
        let batch = host.store.get_command(commands, None).unwrap();
        append(&mut host, Some(cid(0)), batch).unwrap();
        assert_eq!(next(&mut host, Some(cid(0))), 1);
        assert_eq!(host.outcomes[0].0.wp, Some(pid(1)));
        assert_eq!(host.outcomes[0].1.s, Some(sid(0)));
        assert_eq!(host.outcomes[0].1.wl, None);
        let mut args = Args::default();
        args.set(
            b'c',
            Some(ArgsValue::string(ByteString::from("missing"))),
            ArgsEntryFlags::default(),
        );
        let commands = Rc::new(CommandList {
            group: 1,
            commands: vec![Command {
                entry: &CLIENT,
                args,
                group: 1,
                file: None,
                line: 0,
                parse_flags: Default::default(),
            }],
        });
        let batch = host.store.get_command(commands, None).unwrap();
        append(&mut host, Some(cid(0)), batch).unwrap();
        assert_eq!(next(&mut host, Some(cid(0))), 1);
        assert_eq!(host.outcomes.len(), 1);
        assert!(
            host.trace
                .contains(&ByteString::from("status:Can't find client: missing"))
        );
    }
    #[test]
    fn original_client_restored_before_guard_and_messages() {
        let mut host = Host::new();
        host.flags[0].insert(ClientFlags::CONTROL);
        host.finished = true;
        host.uid = Some(101);
        host.commands(&[1], None);
        assert_eq!(next(&mut host, None), 1);
        assert!(!host.trace.iter().any(|t| t.starts_with(b"guard:")));
        assert!(host.trace.contains(&ByteString::from("log:command: work")));
        let state = host
            .store
            .new_state(
                &host.model,
                None,
                Some(&QueueEvent {
                    key: KeyCode(120),
                    ..QueueEvent::default()
                }),
                QueueStateFlags::CONTROL,
            )
            .unwrap();
        let batch = host.store.get_command(list(&[1]), Some(state)).unwrap();
        host.store.free_state(state).unwrap();
        append(&mut host, Some(cid(0)), batch).unwrap();
        next(&mut host, Some(cid(0)));
        assert!(
            host.trace
                .contains(&ByteString::from("log:first[guest] key x: work"))
        );
        assert!(
            host.trace
                .contains(&ByteString::from("guard:0:Begin:42:2:1"))
        );
        assert!(host.trace.contains(&ByteString::from("guard:0:End:42:2:1")));
    }
    #[test]
    fn every_error_output_route_print_and_read_only_callback() {
        let mut host = Host::new();
        let global = host.commands(&[1], None);
        error(&mut host, global, b"bad");
        assert!(
            host.trace
                .contains(&ByteString::from("cause:test.conf:9: bad"))
        );
        host.finished = true;
        error(&mut host, global, b"bad");
        assert!(
            host.trace
                .contains(&ByteString::from("log:message: test.conf:9: bad"))
        );
        let attached = host.commands(&[1], Some(cid(0)));
        error(&mut host, attached, b"bad");
        assert!(host.trace.contains(&ByteString::from("status:Bad")));
        host.flags[0].insert(ClientFlags::CONTROL);
        error(&mut host, attached, b"bad");
        assert!(host.trace.contains(&ByteString::from("control:bad")));
        assert!(host.trace.contains(&ByteString::from("exit:1")));
        let unattached = host.commands(&[1], Some(cid(2)));
        host.flags[2] = ClientFlags::default();
        error(&mut host, unattached, b"bad \xff");
        assert!(host.trace.contains(&ByteString::from("stderr:bad _\n")));
        print(&mut host, attached, b"out");
        assert!(host.trace.contains(&ByteString::from("print:out")));
        let batch = host
            .store
            .get_callback("readonly", Box::new(super::super::key_bindings::read_only))
            .unwrap();
        append(&mut host, Some(cid(0)), batch).unwrap();
        next(&mut host, Some(cid(0)));
        assert!(
            host.trace
                .contains(&ByteString::from("control:client is read-only"))
        );
    }
    #[test]
    fn source_failure_clears_state_and_client_flag_paths() {
        let mut host = Host::new();
        host.flags[0].insert(ClientFlags::CONTROL);
        let mut args = Args::default();
        args.set(
            b's',
            Some(ArgsValue::string(ByteString::from("%999"))),
            ArgsEntryFlags::default(),
        );
        let commands = Rc::new(CommandList {
            group: 1,
            commands: vec![Command {
                entry: &SOURCE,
                args,
                group: 1,
                file: None,
                line: 0,
                parse_flags: Default::default(),
            }],
        });
        let batch = host.store.get_command(commands, None).unwrap();
        append(&mut host, Some(cid(0)), batch).unwrap();
        assert_eq!(next(&mut host, Some(cid(0))), 1);
        assert!(host.outcomes.is_empty());
        assert!(
            host.trace
                .contains(&ByteString::from("control:can't find pane: %999"))
        );
        assert!(
            host.trace
                .contains(&ByteString::from("guard:0:Error:42:1:0"))
        );
        for (entry, flag, value) in [
            (&TCLIENT, b't', "second"),
            (&OPTIONAL_CLIENT, b'c', "missing"),
        ] {
            let mut args = Args::default();
            args.set(
                flag,
                Some(ArgsValue::string(ByteString::from(value))),
                ArgsEntryFlags::default(),
            );
            let commands = Rc::new(CommandList {
                group: 1,
                commands: vec![Command {
                    entry,
                    args,
                    group: 1,
                    file: None,
                    line: 0,
                    parse_flags: Default::default(),
                }],
            });
            let batch = host.store.get_command(commands, None).unwrap();
            append(&mut host, Some(cid(0)), batch).unwrap();
            assert_eq!(next(&mut host, Some(cid(0))), 1);
        }
        assert_eq!(host.outcomes[0].1.s, Some(sid(1)));
        assert_eq!(host.outcomes[1].1.s, Some(sid(0)));
        let stale = host.store.empty_state().unwrap();
        host.store.free_state(stale).unwrap();
        assert_eq!(host.store.link_state(stale), Err(ArenaError::StaleId));
    }
}
