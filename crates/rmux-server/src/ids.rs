// Ported from tmux tmux.h @ 8f25579c
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

use std::marker::PhantomData;

mod sealed {
    pub trait Sealed {}
}
pub trait ArenaId: sealed::Sealed + Copy {
    fn from_parts(slot: u32, generation: u32) -> Self;
    fn parts(self) -> (u32, u32);
}
macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name { slot: u32, generation: u32 }
        impl sealed::Sealed for $name {}
        impl ArenaId for $name {
            fn from_parts(slot: u32, generation: u32) -> Self { Self { slot, generation } }
            fn parts(self) -> (u32, u32) { (self.slot, self.generation) }
        }
    )+};
}
ids!(
    SessionId,
    WindowId,
    WinlinkId,
    PaneId,
    ClientId,
    LayoutCellId,
    ModeTreeItemId,
    SessionGroupId,
    OptionsId,
    QueueItemId,
    QueueStateId,
    KeyTableId,
    JobId,
    PeerId,
    ClientFileId,
    EventSinkId,
    RequestId,
    EditorId,
    MonitorSetId,
    HooksMonitorId,
    TimerId,
    EventToken,
    PasteBufferId
);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModeId {
    pub owner: PaneId,
    slot: u32,
    generation: u32,
}
impl ModeId {
    pub const fn new(owner: PaneId, slot: u32, generation: u32) -> Self {
        Self {
            owner,
            slot,
            generation,
        }
    }
    pub const fn local_parts(self) -> (u32, u32) {
        (self.slot, self.generation)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArenaError {
    StaleId,
    CapacityExhausted,
    LeaseOverflow,
    LeaseUnderflow,
}
impl std::fmt::Display for ArenaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ArenaError {}

struct Slot<T> {
    value: Option<T>,
    generation: u32,
    leases: u32,
    pending: bool,
    next: Option<u32>,
}

/// Handles do not retain allocations. Pending removals remain addressable until the last lease.
/// ```compile_fail
/// use rmux_server::ids::{Arena, PaneId, WindowId};
/// let mut panes: Arena<(), PaneId> = Arena::new();
/// let pane = panes.insert(()).unwrap();
/// let windows: Arena<(), WindowId> = Arena::new();
/// windows.get(pane);
/// ```
pub struct Arena<T, I: ArenaId> {
    slots: Vec<Slot<T>>,
    free: Option<u32>,
    len: usize,
    marker: PhantomData<I>,
}
impl<T, I: ArenaId> Default for Arena<T, I> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T, I: ArenaId> Arena<T, I> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: None,
            len: 0,
            marker: PhantomData,
        }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn insert(&mut self, value: T) -> Result<I, ArenaError> {
        let index = if let Some(index) = self.free {
            let slot = &mut self.slots[index as usize];
            self.free = slot.next.take();
            slot.value = Some(value);
            slot.pending = false;
            index
        } else {
            let index =
                u32::try_from(self.slots.len()).map_err(|_| ArenaError::CapacityExhausted)?;
            self.slots
                .try_reserve(1)
                .map_err(|_| ArenaError::CapacityExhausted)?;
            self.slots.push(Slot {
                value: Some(value),
                generation: 0,
                leases: 0,
                pending: false,
                next: None,
            });
            index
        };
        self.len += 1;
        Ok(I::from_parts(index, self.slots[index as usize].generation))
    }
    fn slot(&self, id: I) -> Option<&Slot<T>> {
        let (index, generation) = id.parts();
        self.slots
            .get(index as usize)
            .filter(|s| s.generation == generation && s.value.is_some())
    }
    fn slot_mut(&mut self, id: I) -> Result<&mut Slot<T>, ArenaError> {
        let (index, generation) = id.parts();
        self.slots
            .get_mut(index as usize)
            .filter(|s| s.generation == generation && s.value.is_some())
            .ok_or(ArenaError::StaleId)
    }
    pub fn get(&self, id: I) -> Option<&T> {
        self.slot(id)?.value.as_ref()
    }
    pub fn get_mut(&mut self, id: I) -> Option<&mut T> {
        self.slot_mut(id).ok()?.value.as_mut()
    }
    pub fn retain(&mut self, id: I) -> Result<(), ArenaError> {
        let slot = self.slot_mut(id)?;
        slot.leases = slot
            .leases
            .checked_add(1)
            .ok_or(ArenaError::LeaseOverflow)?;
        Ok(())
    }
    /// Live lease count (`c->references` for logging), `None` for a stale id.
    pub fn leases(&self, id: I) -> Option<u32> {
        self.slot(id).map(|slot| slot.leases)
    }
    pub fn release(&mut self, id: I) -> Result<Option<T>, ArenaError> {
        let slot = self.slot_mut(id)?;
        slot.leases = slot
            .leases
            .checked_sub(1)
            .ok_or(ArenaError::LeaseUnderflow)?;
        if slot.leases == 0 && slot.pending {
            Ok(self.remove(id))
        } else {
            Ok(None)
        }
    }
    pub fn request_remove(&mut self, id: I) -> Result<Option<T>, ArenaError> {
        let slot = self.slot_mut(id)?;
        slot.pending = true;
        if slot.leases == 0 {
            Ok(self.remove(id))
        } else {
            Ok(None)
        }
    }
    fn remove(&mut self, id: I) -> Option<T> {
        let (index, _) = id.parts();
        let slot = &mut self.slots[index as usize];
        let value = slot.value.take();
        self.len -= 1;
        if let Some(generation) = slot.generation.checked_add(1) {
            slot.generation = generation;
            slot.next = self.free;
            self.free = Some(index);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lifecycle<I: ArenaId>() {
        let mut a: Arena<Vec<u8>, I> = Arena::new();
        let id = a.insert(vec![255]).unwrap();
        a.retain(id).unwrap();
        assert_eq!(a.request_remove(id), Ok(None));
        assert_eq!(a.get(id).unwrap(), &[255]);
        a.get_mut(id).unwrap().push(0);
        assert_eq!(a.release(id), Ok(Some(vec![255, 0])));
        assert!(a.get(id).is_none());
        assert_eq!(a.retain(id), Err(ArenaError::StaleId));
        let new = a.insert(vec![]).unwrap();
        assert_ne!(id.parts(), new.parts());
        assert!(a.get(id).is_none());
        assert_eq!(a.request_remove(new), Ok(Some(vec![])));
    }
    #[test]
    fn every_id_lifecycle() {
        lifecycle::<SessionId>();
        lifecycle::<WindowId>();
        lifecycle::<WinlinkId>();
        lifecycle::<PaneId>();
        lifecycle::<ClientId>();
        lifecycle::<LayoutCellId>();
        lifecycle::<SessionGroupId>();
        lifecycle::<OptionsId>();
        lifecycle::<QueueItemId>();
        lifecycle::<QueueStateId>();
        lifecycle::<KeyTableId>();
        lifecycle::<JobId>();
        lifecycle::<PeerId>();
        lifecycle::<ClientFileId>();
        lifecycle::<EventSinkId>();
        lifecycle::<RequestId>();
        lifecycle::<EditorId>();
        lifecycle::<MonitorSetId>();
        lifecycle::<PasteBufferId>();
    }
    #[test]
    fn retirement_and_lease_errors() {
        let mut a: Arena<(), PaneId> = Arena::new();
        let id = a.insert(()).unwrap();
        assert_eq!(a.release(id), Err(ArenaError::LeaseUnderflow));
        a.slots[0].leases = u32::MAX;
        assert_eq!(a.retain(id), Err(ArenaError::LeaseOverflow));
        a.slots[0].leases = 0;
        a.slots[0].generation = u32::MAX;
        let last = PaneId::from_parts(0, u32::MAX);
        a.request_remove(last).unwrap();
        let next = a.insert(()).unwrap();
        assert_eq!(next.parts().0, 1);
        assert!(a.get(last).is_none());
    }
    #[test]
    fn active_index_is_not_a_lease() {
        let mut a: Arena<(), ClientId> = Arena::new();
        let id = a.insert(()).unwrap();
        let mut index = std::collections::BTreeMap::from([(1, id)]);
        a.retain(id).unwrap();
        index.remove(&1);
        assert_eq!(a.request_remove(id), Ok(None));
        assert!(a.get(id).is_some());
        assert_eq!(a.release(id), Ok(Some(())));
        assert!(a.get(id).is_none());
    }
}
