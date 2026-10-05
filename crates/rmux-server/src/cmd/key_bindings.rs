// Ported from tmux tmux.h, key-bindings.c @ 8f25579c
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeyBindingFlags(pub u32);
impl KeyBindingFlags {
    pub const REPEAT: Self = Self(1);
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
impl std::ops::BitOr for KeyBindingFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for KeyBindingFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for KeyBindingFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use std::collections::BTreeMap;
use std::rc::Rc;

use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, KeyMasks};

use super::find::CmdFindState;
use super::parse::{self, CmdParseError, CmdParseInput, ParseContext};
use super::queue::{QueueBatch, QueueEvent, QueueStateFlags};
use super::{CommandFlags, CommandList};
use crate::ids::{Arena, ArenaError, ClientId, KeyTableId, QueueItemId};

pub mod defaults;

pub type KeyTableIndex = BTreeMap<ByteString, KeyTableId>;

pub struct KeyBinding {
    pub key: KeyCode,
    pub list: Rc<CommandList>,
    pub note: Option<ByteString>,
    pub table: KeyTableId,
    pub flags: KeyBindingFlags,
    serial: u64,
}

impl KeyBinding {
    pub fn dispatch_snapshot(&self) -> KeyBindingDispatch {
        KeyBindingDispatch {
            list: Rc::clone(&self.list),
            flags: self.flags,
        }
    }

    pub fn serial(&self) -> u64 {
        self.serial
    }
}

pub struct KeyBindingDispatch {
    pub list: Rc<CommandList>,
    pub flags: KeyBindingFlags,
}

pub struct KeyTable {
    pub name: ByteString,
    pub bindings: BTreeMap<KeyCode, KeyBinding>,
    pub defaults: BTreeMap<KeyCode, KeyBinding>,
    pub activity_time: (i64, i64),
    serial: u64,
}

impl KeyTable {
    pub fn serial(&self) -> u64 {
        self.serial
    }
    pub fn get(&self, key: KeyCode) -> Option<&KeyBinding> {
        self.bindings.get(&key)
    }
    pub fn get_default(&self, key: KeyCode) -> Option<&KeyBinding> {
        self.defaults.get(&key)
    }
    pub fn bindings(&self) -> impl Iterator<Item = &KeyBinding> {
        self.bindings.values()
    }
}

#[derive(Default)]
pub struct KeyBindings {
    pub tables: Arena<KeyTable, KeyTableId>,
    pub index: KeyTableIndex,
    next_serial: u64,
}

impl KeyBindings {
    pub fn new() -> Self {
        Self::default()
    }

    fn serial(&mut self) -> u64 {
        self.next_serial = self.next_serial.wrapping_add(1);
        self.next_serial
    }

    pub fn get_table(
        &mut self,
        name: &[u8],
        create: bool,
    ) -> Result<Option<KeyTableId>, ArenaError> {
        if let Some(id) = self.index.get(name) {
            return Ok(Some(*id));
        }
        if !create {
            return Ok(None);
        }
        let serial = self.serial();
        let id = self.tables.insert(KeyTable {
            name: name.into(),
            bindings: BTreeMap::new(),
            defaults: BTreeMap::new(),
            activity_time: (0, 0),
            serial,
        })?;
        self.tables.retain(id)?;
        self.index.insert(name.into(), id);
        Ok(Some(id))
    }

    pub fn find_table(&self, name: &[u8]) -> Option<KeyTableId> {
        self.index.get(name).copied()
    }
    pub fn tables(&self) -> impl Iterator<Item = KeyTableId> + '_ {
        self.index.values().copied()
    }
    pub fn retain_table(&mut self, id: KeyTableId) -> Result<(), ArenaError> {
        self.tables.retain(id)
    }
    pub fn unref_table(&mut self, id: KeyTableId) -> Result<(), ArenaError> {
        self.tables.release(id).map(|_| ())
    }
    pub fn get(&self, id: KeyTableId, key: KeyCode) -> Option<&KeyBinding> {
        self.tables.get(id)?.get(key)
    }
    pub fn get_default(&self, id: KeyTableId, key: KeyCode) -> Option<&KeyBinding> {
        self.tables.get(id)?.get_default(key)
    }

    pub fn add(
        &mut self,
        name: &[u8],
        key: KeyCode,
        note: Option<&[u8]>,
        repeat: bool,
        list: Option<Rc<CommandList>>,
    ) -> Result<(), ArenaError> {
        let id = self.get_table(name, true)?.ok_or(ArenaError::StaleId)?;
        let key = KeyCode(key.0 & !KeyMasks::FLAGS);
        let Some(list) = list else {
            if let Some(binding) = self
                .tables
                .get_mut(id)
                .and_then(|t| t.bindings.get_mut(&key))
            {
                if let Some(note) = note {
                    binding.note = Some(note.into());
                }
                if repeat {
                    binding.flags.insert(KeyBindingFlags::REPEAT);
                }
            }
            return Ok(());
        };
        let serial = self.serial();
        let binding = KeyBinding {
            key,
            list,
            note: note.map(Into::into),
            table: id,
            flags: if repeat {
                KeyBindingFlags::REPEAT
            } else {
                KeyBindingFlags::default()
            },
            serial,
        };
        self.tables
            .get_mut(id)
            .ok_or(ArenaError::StaleId)?
            .bindings
            .insert(key, binding);
        Ok(())
    }

    fn unlink_table(&mut self, id: KeyTableId) -> Result<(), ArenaError> {
        self.tables.request_remove(id)?;
        self.unref_table(id)
    }

    pub fn remove(&mut self, name: &[u8], key: KeyCode) -> Result<(), ArenaError> {
        let Some(id) = self.find_table(name) else {
            return Ok(());
        };
        let table = self.tables.get_mut(id).ok_or(ArenaError::StaleId)?;
        if table
            .bindings
            .remove(&KeyCode(key.0 & !KeyMasks::FLAGS))
            .is_none()
        {
            return Ok(());
        }
        if table.bindings.is_empty() && table.defaults.is_empty() {
            self.index.remove(name);
            self.unlink_table(id)?;
        }
        Ok(())
    }

    pub fn reset(&mut self, name: &[u8], key: KeyCode) -> Result<(), ArenaError> {
        let Some(id) = self.find_table(name) else {
            return Ok(());
        };
        let key = KeyCode(key.0 & !KeyMasks::FLAGS);
        let table = self.tables.get_mut(id).ok_or(ArenaError::StaleId)?;
        let Some(binding) = table.bindings.get_mut(&key) else {
            return Ok(());
        };
        if let Some(default) = table.defaults.get(&key) {
            binding.list = Rc::clone(&default.list);
            binding.note = default.note.clone();
            binding.flags = default.flags;
            Ok(())
        } else {
            self.remove(name, key)
        }
    }

    pub fn remove_table(
        &mut self,
        runtime: &mut impl KeyTableRuntime,
        name: &[u8],
    ) -> Result<(), ArenaError> {
        let Some(id) = self.index.remove(name) else {
            return Ok(());
        };
        runtime.reset_clients_using_table(self, id)?;
        self.unlink_table(id)
    }

    pub fn reset_table(
        &mut self,
        runtime: &mut impl KeyTableRuntime,
        name: &[u8],
    ) -> Result<(), ArenaError> {
        let Some(id) = self.find_table(name) else {
            return Ok(());
        };
        let table = self.tables.get(id).ok_or(ArenaError::StaleId)?;
        if table.defaults.is_empty() {
            return self.remove_table(runtime, name);
        }
        let mut previous = None;
        loop {
            let table = self.tables.get(id).ok_or(ArenaError::StaleId)?;
            let next = match previous {
                None => table.bindings.keys().next().copied(),
                Some(key) => table
                    .bindings
                    .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
                    .next()
                    .map(|(k, _)| *k),
            };
            let Some(key) = next else {
                break;
            };
            self.reset(name, key)?;
            previous = Some(key);
        }
        Ok(())
    }

    pub fn init_done(&mut self) {
        let mut serial = self.next_serial;
        for id in self.index.values() {
            let table = self.tables.get_mut(*id).expect("indexed key table");
            for binding in table.bindings.values() {
                serial = serial.wrapping_add(1);
                table.defaults.insert(
                    binding.key,
                    KeyBinding {
                        key: binding.key,
                        list: Rc::clone(&binding.list),
                        note: binding.note.clone(),
                        table: *id,
                        flags: binding.flags,
                        serial,
                    },
                );
            }
        }
        self.next_serial = serial;
    }
}

pub trait KeyTableRuntime {
    /// Reset matching clients and release their table leases before the index lease is released.
    fn reset_clients_using_table(
        &mut self,
        bindings: &mut KeyBindings,
        table: KeyTableId,
    ) -> Result<(), ArenaError>;
}

pub trait KeyDispatchRuntime {
    fn client_read_only(&self, client: ClientId) -> bool;
    fn binding_commands(
        &mut self,
        list: Rc<CommandList>,
        current: &CmdFindState,
        event: Option<&QueueEvent>,
        flags: QueueStateFlags,
    ) -> QueueBatch;
    /// Callback reports `client is read-only` via queue error output and returns Error when fired.
    fn binding_read_only(&mut self) -> QueueBatch;
    fn append_binding(
        &mut self,
        client: Option<ClientId>,
        batch: QueueBatch,
    ) -> Option<QueueItemId>;
    fn insert_binding(&mut self, item: QueueItemId, batch: QueueBatch) -> Option<QueueItemId>;
}

pub fn dispatch<R: KeyDispatchRuntime>(
    runtime: &mut R,
    binding: KeyBindingDispatch,
    item: Option<QueueItemId>,
    client: Option<ClientId>,
    event: Option<&QueueEvent>,
    current: &CmdFindState,
) -> Option<QueueItemId> {
    let batch = if client.is_some_and(|c| runtime.client_read_only(c))
        && !binding.list.all_have(CommandFlags::READONLY)
    {
        runtime.binding_read_only()
    } else {
        let flags = if binding.flags.contains(KeyBindingFlags::REPEAT) {
            QueueStateFlags::REPEAT
        } else {
            QueueStateFlags::default()
        };
        runtime.binding_commands(binding.list, current, event, flags)
    };
    match item {
        Some(item) => runtime.insert_binding(item, batch),
        None => runtime.append_binding(client, batch),
    }
}

pub fn read_only(
    runtime: &mut dyn super::queue::QueueRuntime,
    item: QueueItemId,
) -> super::queue::CmdReturn {
    super::queue::error(runtime, item, b"client is read-only");
    super::queue::CmdReturn::Error
}

pub fn has_repeat<'a>(bindings: impl IntoIterator<Item = &'a KeyBinding>) -> bool {
    bindings
        .into_iter()
        .any(|binding| binding.flags.contains(KeyBindingFlags::REPEAT))
}

pub trait KeyInitRuntime: ParseContext {
    fn append_default_commands(&mut self, list: Rc<CommandList>);
    /// Append a callback which invokes KeyBindings::init_done after all default commands.
    fn append_default_snapshot(&mut self);
}

#[derive(Debug)]
pub struct DefaultKeyError {
    pub binding: &'static [u8],
    pub error: CmdParseError,
}

pub fn init(runtime: &mut impl KeyInitRuntime) -> Result<(), DefaultKeyError> {
    for binding in defaults::DEFAULT_BINDINGS {
        let list = parse::from_string(runtime, binding, &mut CmdParseInput::default())
            .map_err(|error| DefaultKeyError { binding, error })?;
        runtime.append_default_commands(list);
    }
    runtime.append_default_snapshot();
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_support;
