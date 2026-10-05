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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OptionsTableType {
    Command = 6,
    String = 0,
    Number = 1,
    Key = 2,
    Colour = 3,
    Flag = 4,
    Choice = 5,
}
impl TryFrom<i32> for OptionsTableType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            6 => Ok(Self::Command),
            0 => Ok(Self::String),
            1 => Ok(Self::Number),
            2 => Ok(Self::Key),
            3 => Ok(Self::Colour),
            4 => Ok(Self::Flag),
            5 => Ok(Self::Choice),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct OptionsScope(pub u32);
impl OptionsScope {
    pub const NONE: Self = Self(0);
    pub const SERVER: Self = Self(1);
    pub const SESSION: Self = Self(2);
    pub const WINDOW: Self = Self(4);
    pub const PANE: Self = Self(8);
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
impl std::ops::BitOr for OptionsScope {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for OptionsScope {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for OptionsScope {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct OptionsTableFlags(pub u32);
impl OptionsTableFlags {
    pub const ARRAY: Self = Self(1);
    pub const HOOK: Self = Self(2);
    pub const STYLE: Self = Self(4);
    pub const COLOUR: Self = Self(8);
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
impl std::ops::BitOr for OptionsTableFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for OptionsTableFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for OptionsTableFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PaneBorderIndicator {
    Off = 0,
    Colour = 1,
    Arrows = 2,
    Both = 3,
}
impl TryFrom<i32> for PaneBorderIndicator {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Colour),
            2 => Ok(Self::Arrows),
            3 => Ok(Self::Both),
            _ => Err(value),
        }
    }
}

pub mod environment;
pub mod parse;
pub mod push;
pub mod scope;
pub mod store;
pub mod table;
#[cfg(test)]
mod tests;

pub use parse::{Ambiguous, find_choice, map_name, match_name, parse_name, search};
pub use store::{
    CommandParser, FormatExpander, MonitorSink, Options, OptionsArray, OptionsArrayItem,
    OptionsEntry, OptionsLookup, OptionsParseCtx, OptionsRemoved, OptionsStore, OptionsValue,
    default_to_string,
};
pub use table::{OPTIONS_OTHER_NAMES, OPTIONS_TABLE};

use rmux_util::bytes::ByteString;

/// One row of `options_table[]` (`tmux.h:2469-2488`) without the unused
/// `alternative_name`.
#[derive(Debug)]
pub struct OptionsTableEntry {
    pub name: &'static [u8],
    pub kind: OptionsTableType,
    pub scope: OptionsScope,
    pub flags: OptionsTableFlags,
    pub minimum: u32,
    pub maximum: u32,
    pub choices: Option<&'static [&'static [u8]]>,
    pub default_str: Option<&'static [u8]>,
    pub default_num: i64,
    pub default_arr: Option<&'static [&'static [u8]]>,
    pub separator: Option<&'static [u8]>,
    pub pattern: Option<&'static [u8]>,
    pub text: &'static [u8],
    pub unit: Option<&'static [u8]>,
}

impl OptionsTableEntry {
    pub fn is_array(&self) -> bool {
        self.flags.contains(OptionsTableFlags::ARRAY)
    }
    pub fn is_hook(&self) -> bool {
        self.flags.contains(OptionsTableFlags::HOOK)
    }
    /// The name as UTF-8 for diagnostics; table names are ASCII.
    pub fn name_str(&self) -> &'static str {
        std::str::from_utf8(self.name).unwrap_or("<non-utf8>")
    }
}

/// One row of `options_other_names[]` (`tmux.h:2491-2494`).
#[derive(Debug)]
pub struct OptionsNameMap {
    pub from: &'static [u8],
    pub to: &'static [u8],
}

/// An array item key; derive order equals `options_array_cmp`
/// (`options.c:78-98`): numeric keys first by value, then text keys by bytes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OptionsArrayKey {
    Index(u32),
    Name(ByteString),
}

/// A resolved option name: a table name borrowed from the table, or an
/// owned `@` user option name.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum OptionName {
    Table(&'static [u8]),
    User(ByteString),
}

impl OptionName {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            OptionName::Table(n) => n,
            OptionName::User(n) => n,
        }
    }
}

/// The `char **cause` text of a failed option operation, byte-exact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionsError(pub ByteString);

impl OptionsError {
    pub fn new(prefix: &[u8], value: &[u8]) -> OptionsError {
        let mut s = ByteString::with_capacity(prefix.len() + value.len());
        s.extend_from_slice(prefix);
        s.extend_from_slice(value);
        OptionsError(s)
    }
    pub fn text(text: &[u8]) -> OptionsError {
        OptionsError(ByteString::from(text))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Display for OptionsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for OptionsError {}

/// Read-only option values that lower crates consume without the store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionsSnapshot {
    pub escape_time: i64,
    pub user_keys: Vec<ByteString>,
    pub alternate_screen: bool,
    pub scroll_on_clear: bool,
    pub variation_selector_always_wide: bool,
    pub extended_keys: i64,
}

impl OptionsSnapshot {
    /// `escape-time`, `user-keys`, `extended-keys`, and
    /// `variation-selector-always-wide` come from the server tree;
    /// `alternate-screen` and `scroll-on-clear` from `window` (a window or
    /// pane tree, or `global_w`).
    pub fn from_store(store: &OptionsStore, window: crate::ids::OptionsId) -> OptionsSnapshot {
        let global = store.global;
        let user_keys = store
            .get(global, b"user-keys")
            .map(|(_, o)| {
                o.array_items()
                    .map(|(_, item)| ByteString::from(item.value().as_string()))
                    .collect()
            })
            .unwrap_or_default();
        OptionsSnapshot {
            escape_time: store.get_number(global, b"escape-time"),
            user_keys,
            alternate_screen: store.get_number(window, b"alternate-screen") != 0,
            scroll_on_clear: store.get_number(window, b"scroll-on-clear") != 0,
            variation_selector_always_wide: store
                .get_number(global, b"variation-selector-always-wide")
                != 0,
            extended_keys: store.get_number(global, b"extended-keys"),
        }
    }
}
