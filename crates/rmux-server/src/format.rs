// Ported from tmux format.c, tmux.h @ 8f25579c
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
pub struct FormatFlags(pub u32);
impl FormatFlags {
    pub const STATUS: Self = Self(1);
    pub const FORCE: Self = Self(2);
    pub const NOJOBS: Self = Self(4);
    pub const VERBOSE: Self = Self(8);
    pub const LAST: Self = Self(16);
    pub const NONE: Self = Self(0);
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
impl std::ops::BitOr for FormatFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for FormatFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for FormatFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct FormatTagFlags(pub u32);
impl FormatTagFlags {
    pub const PANE: Self = Self(2147483648);
    pub const WINDOW: Self = Self(1073741824);
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
impl std::ops::BitOr for FormatTagFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for FormatTagFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for FormatTagFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

pub mod draw;
mod expand;
pub mod fuzzy;
pub mod grid;
pub mod jobs;
pub mod json;
pub mod parse;
pub mod regsub;
pub mod runtime;
pub mod sort;
pub mod variables;
use crate::ids::{ClientId, PaneId, PasteBufferId, QueueItemId, SessionId, WindowId, WinlinkId};
pub use grid::{hyperlink as grid_hyperlink, line as grid_line, word as grid_word};
pub use jobs::FormatJobs;
pub use parse::skip;
use rmux_util::{bytes::ByteString, time::Timestamp};
pub use runtime::{
    FormatExternal, FormatLoopEntry, FormatRuntime, OptionScope, ServerFormatRuntime, condition,
    create_from_state, expand_hook, lost_client, single_from_state, tidy_jobs,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FormatKind {
    #[default]
    Unknown,
    Session,
    Window,
    Pane,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FormatContext {
    pub evaluated_client: Option<ClientId>,
    pub session: Option<SessionId>,
    pub winlink: Option<WinlinkId>,
    pub window: Option<WindowId>,
    pub pane: Option<PaneId>,
    pub buffer: Option<PasteBufferId>,
    pub kind: FormatKind,
}
#[derive(Clone, Debug)]
pub enum FormatValue {
    Bytes(ByteString),
    Signed(i64),
    Unsigned(u64),
    Time(Timestamp),
}
impl FormatValue {
    pub fn bytes(self) -> ByteString {
        match self {
            Self::Bytes(v) => v,
            Self::Signed(v) => v.to_string().into(),
            Self::Unsigned(v) => v.to_string().into(),
            Self::Time(v) => v.sec.to_string().into(),
        }
    }
}
pub type FormatCallback =
    Box<dyn FnMut(&FormatContext, &mut dyn FormatRuntime) -> Option<FormatValue>>;
enum Entry {
    Bytes(ByteString),
    Time(Timestamp),
    Lazy(FormatCallback, Option<ByteString>),
}
pub struct FormatTree {
    pub context: FormatContext,
    pub owner: Option<ClientId>,
    pub item: Option<QueueItemId>,
    pub tag: u32,
    pub flags: FormatFlags,
    entries: BTreeMap<ByteString, Entry>,
}
impl FormatTree {
    pub fn create(
        owner: Option<ClientId>,
        item: Option<QueueItemId>,
        tag: u32,
        flags: FormatFlags,
        runtime: &mut dyn FormatRuntime,
    ) -> Self {
        if let Some(owner) = owner {
            runtime.retain_client(owner);
        }
        let mut tree = Self {
            context: FormatContext::default(),
            owner,
            item,
            tag,
            flags,
            entries: BTreeMap::new(),
        };
        if let Some(item) = item {
            for (key, value) in runtime.queue_formats(item) {
                tree.add(&key, value);
            }
        }
        tree
    }
    pub fn release(self, runtime: &mut dyn FormatRuntime) {
        if let Some(owner) = self.owner {
            runtime.release_client(owner);
        }
    }
    pub fn defaults(&mut self, runtime: &mut dyn FormatRuntime, mut context: FormatContext) {
        context.kind = if context.pane.is_some() {
            FormatKind::Pane
        } else if context.winlink.is_some() {
            FormatKind::Window
        } else if context.session.is_some() {
            FormatKind::Session
        } else {
            FormatKind::Unknown
        };
        self.context = runtime.defaults(context);
    }
    pub fn defaults_window(&mut self, window: WindowId) {
        self.context.window = Some(window);
    }
    pub fn defaults_pane(&mut self, runtime: &mut dyn FormatRuntime, pane: PaneId) {
        self.context.pane = Some(pane);
        self.context = runtime.defaults(self.context);
    }
    pub fn defaults_paste_buffer(&mut self, buffer: PasteBufferId) {
        self.context.buffer = Some(buffer);
    }
    pub fn pane(&self) -> Option<PaneId> {
        self.context.pane
    }
    pub fn add(&mut self, key: &[u8], mut value: ByteString) {
        value.0.truncate(rmux_util::bytes::cstr(&value).len());
        self.entries
            .insert(rmux_util::bytes::cstr(key).into(), Entry::Bytes(value));
    }
    pub fn add_time(&mut self, key: &[u8], value: Timestamp) {
        self.entries
            .insert(rmux_util::bytes::cstr(key).into(), Entry::Time(value));
    }
    pub fn add_callback(&mut self, key: &[u8], cb: FormatCallback) {
        self.entries
            .insert(rmux_util::bytes::cstr(key).into(), Entry::Lazy(cb, None));
    }
    pub fn merge(&mut self, from: &Self) {
        for (key, value) in &from.entries {
            let bytes = match value {
                Entry::Bytes(v) | Entry::Lazy(_, Some(v)) => Some(v),
                _ => None,
            };
            if let Some(v) = bytes {
                self.add(key, v.clone());
            }
        }
    }
    fn custom(&mut self, runtime: &mut dyn FormatRuntime, key: &[u8]) -> Option<FormatValue> {
        match self.entries.get_mut(key)? {
            Entry::Bytes(v) => Some(FormatValue::Bytes(v.clone())),
            Entry::Time(v) => Some(FormatValue::Time(*v)),
            Entry::Lazy(cb, cache) => {
                if cache.is_none() {
                    *cache = Some(
                        cb(&self.context, runtime)
                            .map(FormatValue::bytes)
                            .unwrap_or_default(),
                    );
                }
                Some(FormatValue::Bytes(cache.as_ref().unwrap().clone()))
            }
        }
    }
    pub fn each(&mut self, runtime: &mut dyn FormatRuntime, mut emit: impl FnMut(&[u8], &[u8])) {
        for key in variables::REGISTRY {
            if let Some(value) = variables::find_owned(runtime, &self.context, self.owner, key) {
                emit(key, &value.bytes());
            }
        }
        let keys: Vec<_> = self.entries.keys().cloned().collect();
        for key in keys {
            if let Some(value) = self.custom(runtime, &key) {
                emit(&key, &value.bytes());
            }
        }
    }
    pub fn expand(&mut self, runtime: &mut dyn FormatRuntime, input: &[u8]) -> ByteString {
        expand::expand(self, runtime, input, false)
    }
    pub fn expand_time(&mut self, runtime: &mut dyn FormatRuntime, input: &[u8]) -> ByteString {
        expand::expand(self, runtime, input, true)
    }
}
pub fn true_value(input: Option<&[u8]>) -> bool {
    input.is_some_and(|v| {
        let v = rmux_util::bytes::cstr(v);
        !v.is_empty() && v != b"0"
    })
}
pub fn create_defaults(
    runtime: &mut dyn FormatRuntime,
    item: Option<QueueItemId>,
    context: FormatContext,
) -> FormatTree {
    let owner = item.and_then(|item| runtime.owner_client(item));
    let mut tree = FormatTree::create(owner, item, 0, FormatFlags::NONE, runtime);
    tree.defaults(runtime, context);
    tree
}
pub fn single(
    runtime: &mut dyn FormatRuntime,
    item: Option<QueueItemId>,
    context: FormatContext,
    input: &[u8],
) -> ByteString {
    let mut tree = create_defaults(runtime, item, context);
    let value = tree.expand(runtime, input);
    tree.release(runtime);
    value
}
pub fn create_from_target(runtime: &mut dyn FormatRuntime, item: QueueItemId) -> FormatTree {
    let context = runtime.target(item);
    create_defaults(runtime, Some(item), context)
}
pub fn single_from_target(
    runtime: &mut dyn FormatRuntime,
    item: QueueItemId,
    input: &[u8],
) -> ByteString {
    let context = runtime.target(item);
    single(runtime, Some(item), context, input)
}
#[cfg(test)]
mod tests;
pub fn pretty_time(value: Timestamp, seconds: bool) -> ByteString {
    variables::pretty_time_at(value.sec, Timestamp::now().sec, seconds)
}
pub fn log_debug(tree: &FormatTree, runtime: &mut dyn FormatRuntime, depth: u32, message: &[u8]) {
    runtime.log(
        tree.item,
        depth.min(10),
        message,
        tree.flags.contains(FormatFlags::VERBOSE),
    );
}
