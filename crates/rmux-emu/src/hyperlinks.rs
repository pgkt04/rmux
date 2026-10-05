// Ported from tmux hyperlinks.c and style.c @ 8f25579c
/* $OpenBSD: hyperlinks.c,v 1.6 2026/09/28 10:42:01 nicm Exp $ */
/*
 * Copyright (c) 2021 Will <author@will.party>
 * Copyright (c) 2022 Jeff Chiang <pobomp@gmail.com>
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
use crate::style::Style;
use rmux_util::{bytes::cstr, utf8, vis::VisFlags};
use std::{collections::BTreeMap, rc::Rc};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HyperlinkId(pub u32);
impl HyperlinkId {
    pub const NONE: Self = Self(0);
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HyperlinkStoreId {
    slot: usize,
    generation: u64,
}
#[derive(Debug)]
pub struct Hyperlinks {
    id: HyperlinkStoreId,
    registry: Rc<()>,
}
#[derive(Debug)]
pub struct HyperlinkUri {
    uri: Rc<[u8]>,
    internal_id: Rc<[u8]>,
    external_id: Box<[u8]>,
    sequence: u64,
}
impl HyperlinkUri {
    pub fn uri(&self) -> &[u8] {
        &self.uri
    }
    pub fn internal_id(&self) -> &[u8] {
        &self.internal_id
    }
    pub fn external_id(&self) -> &[u8] {
        &self.external_id
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HyperlinkError {
    InvalidStore,
    CounterExhausted,
}
impl std::fmt::Display for HyperlinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidStore => "invalid hyperlink store",
            Self::CounterExhausted => "hyperlink counter exhausted",
        })
    }
}
impl std::error::Error for HyperlinkError {}
type NamedKey = (Rc<[u8]>, Rc<[u8]>);
#[derive(Debug)]
struct Store {
    next_inner: u32,
    references: u32,
    records: BTreeMap<HyperlinkId, HyperlinkUri>,
    named: BTreeMap<NamedKey, HyperlinkId>,
}
#[derive(Debug)]
struct Slot {
    generation: u64,
    store: Option<Store>,
}
#[derive(Debug)]
pub struct HyperlinkRegistry {
    identity: Rc<()>,
    slots: Vec<Slot>,
    free: Vec<usize>,
    fifo: BTreeMap<u64, (HyperlinkStoreId, HyperlinkId)>,
    next_external: u64,
    style_store: Option<Hyperlinks>,
}
impl Default for HyperlinkRegistry {
    fn default() -> Self {
        Self::new()
    }
}
impl HyperlinkRegistry {
    pub fn new() -> Self {
        Self {
            identity: Rc::new(()),
            slots: Vec::new(),
            free: Vec::new(),
            fifo: BTreeMap::new(),
            next_external: 1,
            style_store: None,
        }
    }
    pub fn create(&mut self) -> Result<Hyperlinks, HyperlinkError> {
        let store = Store {
            next_inner: 1,
            references: 1,
            records: BTreeMap::new(),
            named: BTreeMap::new(),
        };
        let slot = if let Some(n) = self.free.pop() {
            self.slots[n].store = Some(store);
            n
        } else {
            self.slots.push(Slot {
                generation: 1,
                store: Some(store),
            });
            self.slots.len() - 1
        };
        Ok(Hyperlinks {
            id: HyperlinkStoreId {
                slot,
                generation: self.slots[slot].generation,
            },
            registry: Rc::clone(&self.identity),
        })
    }
    fn valid(&self, store: &Hyperlinks) -> Result<HyperlinkStoreId, HyperlinkError> {
        if Rc::ptr_eq(&store.registry, &self.identity)
            && self
                .slots
                .get(store.id.slot)
                .is_some_and(|s| s.generation == store.id.generation && s.store.is_some())
        {
            Ok(store.id)
        } else {
            Err(HyperlinkError::InvalidStore)
        }
    }
    fn store(&self, id: HyperlinkStoreId) -> &Store {
        self.slots[id.slot].store.as_ref().unwrap()
    }
    fn store_mut(&mut self, id: HyperlinkStoreId) -> &mut Store {
        self.slots[id.slot].store.as_mut().unwrap()
    }
    pub fn share(&mut self, store: &Hyperlinks) -> Result<Hyperlinks, HyperlinkError> {
        let id = self.valid(store)?;
        let s = self.store_mut(id);
        s.references = s
            .references
            .checked_add(1)
            .ok_or(HyperlinkError::CounterExhausted)?;
        Ok(Hyperlinks {
            id,
            registry: Rc::clone(&self.identity),
        })
    }
    pub fn release(&mut self, store: Hyperlinks) -> Result<(), HyperlinkError> {
        let id = self.valid(&store)?;
        let s = self.store_mut(id);
        s.references -= 1;
        if s.references == 0 {
            self.reset_id(id);
            let slot = &mut self.slots[id.slot];
            slot.store = None;
            if let Some(generation) = slot.generation.checked_add(1) {
                slot.generation = generation;
                self.free.push(id.slot);
            }
        }
        Ok(())
    }
    fn remove(&mut self, store: HyperlinkStoreId, inner: HyperlinkId) {
        let s = self.store_mut(store);
        if let Some(record) = s.records.remove(&inner) {
            if !record.internal_id.is_empty() {
                s.named.remove(&(record.internal_id, record.uri));
            }
            self.fifo.remove(&record.sequence);
        }
    }
    fn reset_id(&mut self, id: HyperlinkStoreId) {
        while let Some((&inner, _)) = self.store(id).records.first_key_value() {
            self.remove(id, inner);
        }
    }
    pub fn reset(&mut self, store: &Hyperlinks) -> Result<(), HyperlinkError> {
        let id = self.valid(store)?;
        self.reset_id(id);
        Ok(())
    }
    pub fn put(
        &mut self,
        store: &Hyperlinks,
        uri: &[u8],
        internal_id: Option<&[u8]>,
    ) -> Result<HyperlinkId, HyperlinkError> {
        let id = self.valid(store)?;
        let flags = VisFlags::OCTAL | VisFlags::CSTYLE;
        let mut escaped = Vec::new();
        utf8::strvis(&mut escaped, cstr(uri), flags);
        if escaped.len() > 1024 {
            return Ok(HyperlinkId::NONE);
        }
        let uri: Rc<[u8]> = escaped.into();
        let mut escaped = Vec::new();
        utf8::strvis(&mut escaped, cstr(internal_id.unwrap_or(b"")), flags);
        let internal: Rc<[u8]> = escaped.into();
        let key = (internal, uri);
        if !key.0.is_empty()
            && let Some(inner) = self.store(id).named.get(&key)
        {
            return Ok(*inner);
        }
        let sequence = self.next_external;
        if sequence > i64::MAX as u64 {
            return Err(HyperlinkError::CounterExhausted);
        }
        let inner = self.store(id).next_inner;
        let next_inner = inner
            .checked_add(1)
            .ok_or(HyperlinkError::CounterExhausted)?;
        let record = HyperlinkUri {
            uri: Rc::clone(&key.1),
            internal_id: Rc::clone(&key.0),
            external_id: format!("tmux{sequence:X}").into_bytes().into_boxed_slice(),
            sequence,
        };
        self.next_external += 1;
        let s = self.store_mut(id);
        s.next_inner = next_inner;
        let inner = HyperlinkId(inner);
        if !key.0.is_empty() {
            s.named.insert(key, inner);
        }
        s.records.insert(inner, record);
        self.fifo.insert(sequence, (id, inner));
        if self.fifo.len() == 5000 {
            let (&_, &(store, old)) = self.fifo.first_key_value().unwrap();
            self.remove(store, old);
        }
        Ok(inner)
    }
    pub fn get(&self, store: &Hyperlinks, id: HyperlinkId) -> Option<&HyperlinkUri> {
        self.store(self.valid(store).ok()?).records.get(&id)
    }
    pub(crate) fn put_style(&mut self, uri: &[u8]) -> Result<HyperlinkId, HyperlinkError> {
        let store = match self.style_store.take() {
            Some(store) => store,
            None => self.create()?,
        };
        let result = self.put(&store, uri, Some(uri));
        self.style_store = Some(store);
        result
    }
    pub(crate) fn style_uri(&self, id: HyperlinkId) -> Option<&HyperlinkUri> {
        self.get(self.style_store.as_ref()?, id)
    }
    pub fn copy_style_link_to_store(
        &mut self,
        style: &Style,
        destination: &Hyperlinks,
    ) -> Result<HyperlinkId, HyperlinkError> {
        let Some(uri) = self.style_uri(style.link).map(|link| Rc::clone(&link.uri)) else {
            return Ok(HyperlinkId::NONE);
        };
        self.put(destination, &uri, Some(&uri))
    }
    pub fn record_count(&self) -> usize {
        self.fifo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_store_and_counter_exhaustion_are_errors() {
        let mut registry = HyperlinkRegistry::new();
        let store = registry.create().unwrap();
        let stale = Hyperlinks {
            id: store.id,
            registry: Rc::clone(&store.registry),
        };
        registry.release(store).unwrap();
        let store = registry.create().unwrap();
        assert_eq!(registry.reset(&stale), Err(HyperlinkError::InvalidStore));
        registry.store_mut(store.id).next_inner = u32::MAX;
        assert_eq!(
            registry.put(&store, b"a", None),
            Err(HyperlinkError::CounterExhausted)
        );
        assert_eq!(registry.record_count(), 0);
        registry.store_mut(store.id).next_inner = 1;
        registry.next_external = i64::MAX as u64 + 1;
        assert_eq!(
            registry.put(&store, b"a", None),
            Err(HyperlinkError::CounterExhausted)
        );
    }
    #[test]
    fn reuse_does_not_refresh_fifo_or_protect_style_records() {
        let mut registry = HyperlinkRegistry::new();
        let store = registry.create().unwrap();
        let first = registry.put(&store, b"first", Some(b"named")).unwrap();
        for _ in 0..4998 {
            registry.put(&store, b"anonymous", None).unwrap();
        }
        assert_eq!(registry.record_count(), 4999);
        assert_eq!(
            registry.put(&store, b"first", Some(b"named")).unwrap(),
            first
        );
        registry.put(&store, b"evict", None).unwrap();
        assert!(registry.get(&store, first).is_none());
        let style = Style {
            link: registry.put_style(b"style").unwrap(),
            ..Style::default()
        };
        for _ in 0..4999 {
            registry.put(&store, b"more", None).unwrap();
        }
        assert!(style.link_uri(&registry).is_none());
        registry.reset(&store).unwrap();
        assert_eq!(registry.record_count(), 0);
        assert!(registry.store(store.id).named.is_empty());
    }
}
