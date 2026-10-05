// Ported from tmux utf8.c @ 8f25579c (utf8_item, utf8_item_by_data, utf8_item_by_index, utf8_put_item)
//! Intern table behind [`Utf8Char`](super::Utf8Char) for characters longer
//! than three bytes. One process-wide table in tmux; one thread-local table
//! here (`utf8.c:238,251`). Entries are never freed.

use std::cell::RefCell;
use std::collections::HashMap;

use super::UTF8_SIZE;

/// Last index `utf8_put_item` hands out (`utf8.c:452`).
pub const MAX_INDEX: u32 = 0xff_ffff;

/// Lookup key: size and the active bytes with a zeroed tail, so that equality
/// examines only `size` bytes like `utf8_data_cmp` (`utf8.c:228-234`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    size: u8,
    data: [u8; UTF8_SIZE],
}

impl Key {
    fn new(bytes: &[u8]) -> Key {
        let mut data = [0u8; UTF8_SIZE];
        data[..bytes.len()].copy_from_slice(bytes);
        Key {
            size: bytes.len() as u8,
            data,
        }
    }
}

#[derive(Default)]
pub struct Utf8Table {
    by_data: HashMap<Key, u32>,
    /// Entry `i` holds the bytes of index `i`; indexes are dense first-seen.
    by_index: Vec<Key>,
}

impl Utf8Table {
    pub fn new() -> Utf8Table {
        Utf8Table::default()
    }

    /// Number of interned sequences; also the next index (`utf8_next_index`).
    pub fn len(&self) -> usize {
        self.by_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_index.is_empty()
    }

    /// `utf8_put_item`: index of `bytes`, inserting when new. `None` when the
    /// index space is exhausted; an existing sequence still succeeds then.
    pub fn put(&mut self, bytes: &[u8]) -> Option<u32> {
        let key = Key::new(bytes);
        if let Some(&index) = self.by_data.get(&key) {
            crate::log_debug!("utf8_put_item: found {} = {}", super::Raw(bytes), index);
            return Some(index);
        }
        let index = u32::try_from(self.by_index.len()).ok()?;
        if index == MAX_INDEX + 1 {
            return None;
        }
        self.by_index.push(key);
        self.by_data.insert(key, index);
        crate::log_debug!("utf8_put_item: added {} = {}", super::Raw(bytes), index);
        Some(index)
    }

    /// `utf8_item_by_index`: the interned bytes for `index`, zero-tailed to
    /// `UTF8_SIZE` like the C `data` array.
    pub fn get(&self, index: u32) -> Option<&[u8; UTF8_SIZE]> {
        self.by_index.get(index as usize).map(|key| &key.data)
    }

    /// Advance the next index without storing entries, for the limit tests.
    #[cfg(test)]
    pub(crate) fn skip_to_index(&mut self, index: u32) {
        let filler = Key::new(&[]);
        self.by_index.resize(index as usize, filler);
    }
}

thread_local! {
    static TABLE: RefCell<Utf8Table> = RefCell::new(Utf8Table::new());
}

/// Borrow the thread's intern table. Compact handles are only meaningful on
/// the thread that created them.
pub fn with_table<R>(f: impl FnOnce(&mut Utf8Table) -> R) -> R {
    TABLE.with(|table| f(&mut table.borrow_mut()))
}
