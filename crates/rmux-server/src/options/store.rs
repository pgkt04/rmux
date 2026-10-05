// Ported from tmux options.c @ 8f25579c
//! The option store: one `Options` tree per scope object with a parent link,
//! entries, values, arrays, defaults, staged removal, and string conversion.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use rmux_emu::cell::DEFAULT_CELL;
use rmux_emu::colour::{Colour, parse_colour, write_colour};
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::style::Style;
use rmux_sys::fnmatch::{FnmatchFlags, fnmatch};
use rmux_tty::key_string::{key_name, parse_key_name};
use rmux_util::bytes::{ByteString, cstr, strsep};
use rmux_util::key::{KeyCode, SpecialKey};
use rmux_util::log_debug;
use rmux_util::shell::check_shell;
use rmux_util::strtonum::{StrtonumError, strtonum};
use rmux_util::time::Timestamp;

pub use crate::cmd::CommandList;
use crate::cmd::CommandListPrintFlags;
pub use crate::cmd::hooks::HooksMonitorId;
pub use crate::cmd::parse::{CmdParseError, CommandParser};
use crate::ids::{Arena, OptionsId};

use super::parse::{find_choice, map_name, match_name, parse_name};
use super::{
    OptionName, OptionsArrayKey, OptionsError, OptionsTableEntry, OptionsTableFlags,
    OptionsTableType,
};

/// `format_expand(ft, s)` for dynamic styles (G10); tests supply a double.
pub trait FormatExpander {
    fn expand(&mut self, s: &[u8]) -> ByteString;
}

/// `hooks_monitor_free` (G11) called between value destruction and unlink.
pub trait MonitorSink {
    fn monitor_free(&mut self, monitor: HooksMonitorId);
}

/// Everything `options_from_string` reaches outside the store.
pub struct OptionsParseCtx<'a> {
    pub parser: &'a mut dyn CommandParser,
    pub links: &'a mut HyperlinkRegistry,
    /// `getprogname()` for `checkshell`.
    pub program: &'a [u8],
}

static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

/// Process-wide creation identity for entries and items (G19 tags).
fn next_serial() -> u64 {
    let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
    assert!(serial != 0, "options serial counter exhausted");
    serial
}

/// `union options_value` (`tmux.h:2439-2445`), discriminated by the table type.
#[derive(Clone, Debug)]
pub enum OptionsValue {
    String(ByteString),
    Number(i64),
    /// `None` when the default failed to parse (`options.c:343-344`).
    Command(Option<Rc<CommandList>>),
    Array(OptionsArray),
}

impl OptionsValue {
    pub fn as_string(&self) -> &[u8] {
        match self {
            OptionsValue::String(s) => s,
            _ => b"",
        }
    }
    pub fn as_number(&self) -> Option<i64> {
        match self {
            OptionsValue::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_command(&self) -> Option<&Rc<CommandList>> {
        match self {
            OptionsValue::Command(c) => c.as_ref(),
            _ => None,
        }
    }
}

/// The array of an `IS_ARRAY` entry, in `options_array_cmp` order.
#[derive(Clone, Debug, Default)]
pub struct OptionsArray(BTreeMap<OptionsArrayKey, OptionsArrayItem>);

/// `struct options_array_item`: an owned value with a creation serial.
#[derive(Clone, Debug)]
pub struct OptionsArrayItem {
    value: OptionsValue,
    serial: u64,
}

impl OptionsArrayItem {
    fn new(value: OptionsValue) -> OptionsArrayItem {
        OptionsArrayItem {
            value,
            serial: next_serial(),
        }
    }
    pub fn value(&self) -> &OptionsValue {
        &self.value
    }
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

impl OptionsArray {
    /// First numeric key not in use, counting from 0 and stopping at
    /// `limit` (`UINT_MAX` in C, `options.c:651-654,665-668`).
    fn first_free_index(&self, limit: u32) -> u32 {
        let mut expected = 0u32;
        for key in self.0.keys() {
            match key {
                OptionsArrayKey::Index(i) if *i == expected => {
                    if expected == limit {
                        return limit;
                    }
                    expected += 1;
                }
                _ => break,
            }
        }
        expected.min(limit)
    }
}

/// `struct options_entry` (`options.c:101-116`).
#[derive(Clone, Debug)]
pub struct OptionsEntry {
    name: ByteString,
    serial: u64,
    table: Option<&'static OptionsTableEntry>,
    value: OptionsValue,
    /// `cached` plus `style`; stored even after a failed cached parse.
    style_cache: Option<Style>,
    monitor: Option<HooksMonitorId>,
    fire_count: u32,
    fire_time: Timestamp,
}

fn empty_value(oe: Option<&'static OptionsTableEntry>) -> OptionsValue {
    match oe {
        None => OptionsValue::String(ByteString::new()),
        Some(oe) if oe.is_array() => OptionsValue::Array(OptionsArray::default()),
        Some(oe) => match oe.kind {
            OptionsTableType::String => OptionsValue::String(ByteString::new()),
            OptionsTableType::Command => OptionsValue::Command(None),
            _ => OptionsValue::Number(0),
        },
    }
}

/// `colour_fromstring` as a number, `None` for `-1`.
fn colour_from_string(value: &[u8]) -> Option<i64> {
    parse_colour(value).ok().map(|c| i64::from(c.0))
}

fn colour_to_string(number: i64, out: &mut Vec<u8>) {
    write_colour(Colour(number as i32), out);
}

impl OptionsEntry {
    fn new(name: &[u8], table: Option<&'static OptionsTableEntry>) -> OptionsEntry {
        OptionsEntry {
            name: ByteString::from(name),
            serial: next_serial(),
            table,
            value: empty_value(table),
            style_cache: None,
            monitor: None,
            fire_count: 0,
            fire_time: Timestamp::ZERO,
        }
    }

    /// `options_empty` without a tree: an entry with no value.
    pub fn new_empty(oe: &'static OptionsTableEntry) -> OptionsEntry {
        OptionsEntry::new(oe.name, Some(oe))
    }

    /// `options_default` without a tree (`options.c:312-356`).
    pub fn from_default(
        oe: &'static OptionsTableEntry,
        parser: &mut dyn CommandParser,
    ) -> OptionsEntry {
        let mut o = OptionsEntry::new_empty(oe);
        if oe.is_array() {
            match oe.default_arr {
                None => {
                    // The cause is dropped, as with a NULL cause in C.
                    let _ = o.array_assign(oe.default_str.unwrap_or(b""), parser);
                }
                Some(items) => {
                    for (i, item) in items.iter().enumerate() {
                        let _ = o.array_set(
                            &OptionsArrayKey::Index(i as u32),
                            Some(item),
                            false,
                            parser,
                        );
                    }
                }
            }
            return o;
        }
        o.value = match oe.kind {
            OptionsTableType::String => {
                OptionsValue::String(ByteString::from(oe.default_str.unwrap_or(b"")))
            }
            OptionsTableType::Command => {
                OptionsValue::Command(parser.parse_from_string(oe.default_str.unwrap_or(b"")).ok())
            }
            _ => OptionsValue::Number(oe.default_num),
        };
        o
    }

    pub fn name(&self) -> &[u8] {
        &self.name
    }
    pub fn serial(&self) -> u64 {
        self.serial
    }
    pub fn table_entry(&self) -> Option<&'static OptionsTableEntry> {
        self.table
    }
    pub fn value(&self) -> &OptionsValue {
        &self.value
    }

    /// `OPTIONS_IS_STRING`
    pub fn is_string(&self) -> bool {
        self.table
            .is_none_or(|oe| oe.kind == OptionsTableType::String)
    }
    /// `OPTIONS_IS_NUMBER`
    pub fn is_number(&self) -> bool {
        self.table.is_some_and(|oe| {
            matches!(
                oe.kind,
                OptionsTableType::Number
                    | OptionsTableType::Key
                    | OptionsTableType::Colour
                    | OptionsTableType::Flag
                    | OptionsTableType::Choice
            )
        })
    }
    /// `OPTIONS_IS_COMMAND`
    pub fn is_command(&self) -> bool {
        self.table
            .is_some_and(|oe| oe.kind == OptionsTableType::Command)
    }
    /// `OPTIONS_IS_ARRAY`
    pub fn is_array(&self) -> bool {
        self.table.is_some_and(OptionsTableEntry::is_array)
    }

    pub fn monitor(&self) -> Option<HooksMonitorId> {
        self.monitor
    }
    pub fn set_monitor(&mut self, monitor: Option<HooksMonitorId>) {
        self.monitor = monitor;
    }
    /// `options_hook_fired` (`options.c:446-451`).
    pub fn hook_fired(&mut self, now: Timestamp) {
        self.fire_count = self.fire_count.wrapping_add(1);
        self.fire_time = now;
    }
    pub fn fire_count(&self) -> u32 {
        self.fire_count
    }
    #[cfg(test)]
    pub(super) fn set_fire_count_for_test(&mut self, count: u32) {
        self.fire_count = count;
    }
    pub fn fire_time(&self) -> Timestamp {
        self.fire_time
    }
    /// The cached style, if `options_string_to_style` has cached one.
    pub fn cached_style(&self) -> Option<Style> {
        self.style_cache
    }

    /// `options_value_to_string` (`options.c:187-223`).
    fn value_to_string(&self, ov: &OptionsValue, numeric: bool, out: &mut Vec<u8>) {
        use std::io::Write;
        if self.is_command() {
            if let OptionsValue::Command(Some(list)) = ov {
                out.extend_from_slice(&list.print(CommandListPrintFlags(0)));
            }
            return;
        }
        if self.is_number() {
            let number = ov.as_number().unwrap_or(0);
            match self.table.unwrap().kind {
                OptionsTableType::Number => write!(out, "{number}").unwrap(),
                OptionsTableType::Key => {
                    out.extend_from_slice(&key_name(KeyCode(number as u64), false));
                }
                OptionsTableType::Colour => colour_to_string(number, out),
                OptionsTableType::Flag => {
                    if numeric {
                        write!(out, "{number}").unwrap();
                    } else {
                        out.extend_from_slice(if number != 0 { b"on" } else { b"off" });
                    }
                }
                OptionsTableType::Choice => {
                    let choices = self.table.unwrap().choices.unwrap_or(&[]);
                    out.extend_from_slice(choices[number as usize]);
                }
                _ => unreachable!("not a number option type"),
            }
            return;
        }
        if self.is_string() {
            out.extend_from_slice(ov.as_string());
        }
    }

    /// `options_to_string` (`options.c:719-756`): an array without a key
    /// joins the items with single spaces; a missing item is empty.
    pub fn to_string(&self, key: Option<&OptionsArrayKey>, numeric: bool) -> ByteString {
        let mut out = Vec::new();
        if let OptionsValue::Array(array) = &self.value {
            match key {
                None => {
                    for (n, item) in array.0.values().enumerate() {
                        if n > 0 {
                            out.push(b' ');
                        }
                        self.value_to_string(&item.value, numeric, &mut out);
                    }
                }
                Some(key) => {
                    if let Some(item) = array.0.get(key) {
                        self.value_to_string(&item.value, numeric, &mut out);
                    }
                }
            }
            return ByteString(out);
        }
        self.value_to_string(&self.value, numeric, &mut out);
        ByteString(out)
    }

    fn array(&self) -> Option<&OptionsArray> {
        match &self.value {
            OptionsValue::Array(a) if self.is_array() => Some(a),
            _ => None,
        }
    }
    fn array_mut(&mut self) -> Option<&mut OptionsArray> {
        if !self.is_array() {
            return None;
        }
        match &mut self.value {
            OptionsValue::Array(a) => Some(a),
            _ => None,
        }
    }

    /// `options_array_get` (`options.c:512-528`).
    pub fn array_get(&self, key: &OptionsArrayKey) -> Option<&OptionsValue> {
        self.array()?.0.get(key).map(|item| &item.value)
    }
    /// `options_array_first/next` with `options_array_item_key/value`.
    pub fn array_items(&self) -> impl Iterator<Item = (&OptionsArrayKey, &OptionsArrayItem)> {
        self.array().into_iter().flat_map(|a| a.0.iter())
    }
    /// `options_array_clear` (`options.c:500-510`).
    pub fn array_clear(&mut self) {
        if let Some(array) = self.array_mut() {
            array.0.clear();
        }
    }

    /// `options_array_set` (`options.c:546-635`). `None` deletes the item.
    pub fn array_set(
        &mut self,
        key: &OptionsArrayKey,
        value: Option<&[u8]>,
        append: bool,
        parser: &mut dyn CommandParser,
    ) -> Result<(), OptionsError> {
        if !self.is_array() {
            return Err(OptionsError::text(b"not an array"));
        }
        let Some(value) = value else {
            self.array_mut().unwrap().0.remove(key);
            return Ok(());
        };
        if self.is_command() {
            let list = parser
                .parse_from_string(value)
                .map_err(|e| OptionsError::text(e.message()))?;
            Self::array_put(
                self.array_mut().unwrap(),
                key,
                OptionsValue::Command(Some(list)),
            );
            return Ok(());
        }
        if self.is_string() {
            let array = self.array_mut().unwrap();
            match array.0.get_mut(key) {
                Some(item) if append => {
                    if let OptionsValue::String(s) = &mut item.value {
                        s.extend_from_slice(value);
                    }
                }
                _ => Self::array_put(array, key, OptionsValue::String(ByteString::from(value))),
            }
            return Ok(());
        }
        if self.table.unwrap().kind == OptionsTableType::Colour {
            let number = colour_from_string(value)
                .ok_or_else(|| OptionsError::new(b"bad colour: ", value))?;
            Self::array_put(self.array_mut().unwrap(), key, OptionsValue::Number(number));
            return Ok(());
        }
        Err(OptionsError::text(b"wrong array type"))
    }

    /// Replace the value of an existing item (keeping its identity) or
    /// create a new item (`options.c:590-595,606-610,621-626`).
    fn array_put(array: &mut OptionsArray, key: &OptionsArrayKey, value: OptionsValue) {
        match array.0.get_mut(key) {
            Some(item) => item.value = value,
            None => {
                array.0.insert(key.clone(), OptionsArrayItem::new(value));
            }
        }
    }

    /// `options_array_assign` (`options.c:637-679`) with the real
    /// `UINT_MAX` key limit.
    pub fn array_assign(
        &mut self,
        s: &[u8],
        parser: &mut dyn CommandParser,
    ) -> Result<(), OptionsError> {
        self.array_assign_limited(s, parser, u32::MAX)
    }

    /// `array_assign` with a smaller key limit so tests can reach the two
    /// exhaustion branches.
    pub fn array_assign_limited(
        &mut self,
        s: &[u8],
        parser: &mut dyn CommandParser,
        limit: u32,
    ) -> Result<(), OptionsError> {
        let separator = self.table.and_then(|oe| oe.separator).unwrap_or(b" ,");
        if separator.is_empty() {
            if s.is_empty() {
                return Ok(());
            }
            let i = self.array().map_or(0, |a| a.first_free_index(limit));
            return self.array_set(&OptionsArrayKey::Index(i), Some(s), false, parser);
        }
        if s.is_empty() {
            return Ok(());
        }
        let mut rest = Some(s);
        while let Some(string) = rest {
            let (next, tail) = strsep(string, separator);
            rest = tail;
            if next.is_empty() {
                continue;
            }
            let i = self.array().map_or(0, |a| a.first_free_index(limit));
            if i == limit {
                break;
            }
            self.array_set(&OptionsArrayKey::Index(i), Some(next), false, parser)?;
        }
        Ok(())
    }
}

/// `options_default_to_string` (`options.c:358-387`).
pub fn default_to_string(oe: &OptionsTableEntry) -> ByteString {
    let mut out = Vec::new();
    match oe.kind {
        OptionsTableType::String | OptionsTableType::Command => {
            out.extend_from_slice(oe.default_str.unwrap_or(b""));
        }
        OptionsTableType::Number => out.extend_from_slice(oe.default_num.to_string().as_bytes()),
        OptionsTableType::Key => {
            out.extend_from_slice(&key_name(KeyCode(oe.default_num as u64), false));
        }
        OptionsTableType::Colour => colour_to_string(oe.default_num, &mut out),
        OptionsTableType::Flag => {
            out.extend_from_slice(if oe.default_num != 0 { b"on" } else { b"off" });
        }
        OptionsTableType::Choice => {
            out.extend_from_slice(oe.choices.unwrap_or(&[])[oe.default_num as usize]);
        }
    }
    ByteString(out)
}

/// `options_parse_get`/`options_match_get` result: the owning tree, the
/// entry, and the parsed array key.
pub type OptionsLookup<'a> = (OptionsId, &'a OptionsEntry, Option<OptionsArrayKey>);

/// `struct options` (`options.c:118-121`): one tree in `strcmp` order.
#[derive(Debug, Default)]
pub struct Options {
    parent: Option<OptionsId>,
    entries: BTreeMap<ByteString, OptionsEntry>,
}

impl Options {
    fn get_only(&self, name: &[u8]) -> Option<&OptionsEntry> {
        self.entries
            .get(name)
            .or_else(|| self.entries.get(map_name(name)))
    }
    fn get_mut_only(&mut self, name: &[u8]) -> Option<&mut OptionsEntry> {
        if self.entries.contains_key(name) {
            return self.entries.get_mut(name);
        }
        self.entries.get_mut(map_name(name))
    }
}

/// A single-entry removal in progress: the value is destroyed, the entry is
/// still linked, and the monitor awaits G11 cleanup (`options.c:406-420`).
#[derive(Debug)]
#[must_use = "finish_removal unlinks the entry"]
pub struct OptionsRemoved {
    owner: OptionsId,
    name: ByteString,
    serial: u64,
    pub monitor: Option<HooksMonitorId>,
}

impl OptionsRemoved {
    pub fn owner(&self) -> OptionsId {
        self.owner
    }
    pub fn name(&self) -> &[u8] {
        &self.name
    }
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

/// All option trees plus the three global ones (`tmux.c:566-568`).
pub struct OptionsStore {
    arena: Arena<Options, OptionsId>,
    pub global: OptionsId,
    pub global_s: OptionsId,
    pub global_w: OptionsId,
}

impl Default for OptionsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OptionsStore {
    /// Three empty global trees without parents.
    pub fn new() -> OptionsStore {
        let mut arena = Arena::new();
        let global = arena.insert(Options::default()).expect("options arena");
        let global_s = arena.insert(Options::default()).expect("options arena");
        let global_w = arena.insert(Options::default()).expect("options arena");
        OptionsStore {
            arena,
            global,
            global_s,
            global_w,
        }
    }

    /// Fill the global trees from the table by scope bit (`tmux.c:569-576`):
    /// `SERVER` to `global`, `SESSION` to `global_s`, anything with `WINDOW`
    /// (including `WINDOW|PANE`) to `global_w`.
    pub fn load_defaults(&mut self, parser: &mut dyn CommandParser) {
        for oe in super::table::OPTIONS_TABLE {
            if oe.scope.contains(super::OptionsScope::SERVER) {
                self.default(self.global, oe, parser);
            }
            if oe.scope.contains(super::OptionsScope::SESSION) {
                self.default(self.global_s, oe, parser);
            }
            if oe.scope.contains(super::OptionsScope::WINDOW) {
                self.default(self.global_w, oe, parser);
            }
        }
    }

    fn tree(&self, id: OptionsId) -> &Options {
        self.arena.get(id).expect("stale options id")
    }
    fn tree_mut(&mut self, id: OptionsId) -> &mut Options {
        self.arena.get_mut(id).expect("stale options id")
    }

    /// `options_create`
    pub fn create(&mut self, parent: Option<OptionsId>) -> OptionsId {
        if let Some(parent) = parent {
            assert!(self.arena.get(parent).is_some(), "stale parent options id");
        }
        self.arena
            .insert(Options {
                parent,
                entries: BTreeMap::new(),
            })
            .expect("options arena")
    }

    /// `options_free` when no entry owns a monitor; otherwise use
    /// `free_with`.
    pub fn free(&mut self, id: OptionsId) {
        let tree = self.arena.request_remove(id).expect("stale options id");
        if let Some(tree) = tree {
            assert!(
                tree.entries.values().all(|o| o.monitor.is_none()),
                "options_free with live hooks monitors"
            );
        }
    }

    /// `options_free` (`options.c:236-244`): remove each entry in byte
    /// order, completing monitor cleanup per entry, then drop the tree.
    pub fn free_with(&mut self, id: OptionsId, sink: &mut dyn MonitorSink) {
        while let Some(name) = self.tree(id).entries.keys().next().cloned() {
            self.remove(id, &name, sink);
        }
        let _ = self.arena.request_remove(id).expect("stale options id");
    }

    /// `options_get_parent`
    pub fn parent(&self, id: OptionsId) -> Option<OptionsId> {
        self.tree(id).parent
    }
    /// `options_set_parent`
    pub fn set_parent(&mut self, id: OptionsId, parent: Option<OptionsId>) {
        if let Some(parent) = parent {
            assert!(self.arena.get(parent).is_some(), "stale parent options id");
        }
        self.tree_mut(id).parent = parent;
    }

    /// `options_first/next`: entries in byte order.
    pub fn entries(&self, id: OptionsId) -> impl Iterator<Item = &OptionsEntry> {
        self.tree(id).entries.values()
    }

    /// `options_get_only` (`options.c:270-281`): one tree, alias retry.
    pub fn get_only(&self, id: OptionsId, name: &[u8]) -> Option<&OptionsEntry> {
        self.tree(id).get_only(name)
    }
    pub fn get_mut_only(&mut self, id: OptionsId, name: &[u8]) -> Option<&mut OptionsEntry> {
        self.tree_mut(id).get_mut_only(name)
    }

    /// `options_get` (`options.c:283-296`): walk the parent chain.
    pub fn get(&self, id: OptionsId, name: &[u8]) -> Option<(OptionsId, &OptionsEntry)> {
        let mut id = id;
        loop {
            let tree = self.tree(id);
            if let Some(o) = tree.get_only(name) {
                return Some((id, o));
            }
            id = tree.parent?;
        }
    }

    /// `options_add` (`options.c:389-404`) for entries without monitors.
    fn add(&mut self, id: OptionsId, name: &[u8], table: Option<&'static OptionsTableEntry>) {
        if let Some(old) = self.tree(id).get_only(name) {
            assert!(
                old.monitor.is_none(),
                "replacing option {} with a live hooks monitor; use the sink variant",
                ByteString::from(name)
            );
            let old_name = old.name.clone();
            self.tree_mut(id).entries.remove(&old_name);
        }
        self.insert_entry(id, OptionsEntry::new(name, table));
    }

    fn insert_entry(&mut self, id: OptionsId, entry: OptionsEntry) -> &mut OptionsEntry {
        let tree = self.tree_mut(id);
        let name = entry.name.clone();
        tree.entries.insert(name.clone(), entry);
        tree.entries.get_mut(&name).unwrap()
    }

    /// `options_empty` (`options.c:298-310`) when no monitor is attached.
    pub fn empty(&mut self, id: OptionsId, oe: &'static OptionsTableEntry) -> &mut OptionsEntry {
        self.add(id, oe.name, Some(oe));
        self.tree_mut(id).entries.get_mut(oe.name).unwrap()
    }

    /// `options_default` (`options.c:312-356`) when no monitor is attached.
    pub fn default(
        &mut self,
        id: OptionsId,
        oe: &'static OptionsTableEntry,
        parser: &mut dyn CommandParser,
    ) -> &mut OptionsEntry {
        if let Some(old) = self.tree(id).get_only(oe.name) {
            assert!(
                old.monitor.is_none(),
                "replacing option {} with a live hooks monitor; use the sink variant",
                ByteString::from(oe.name)
            );
        }
        self.tree_mut(id).entries.remove(oe.name);
        let entry = OptionsEntry::from_default(oe, parser);
        self.insert_entry(id, entry)
    }

    /// `options_empty` with monitor cleanup of any replaced entry.
    pub fn empty_with(
        &mut self,
        id: OptionsId,
        oe: &'static OptionsTableEntry,
        sink: &mut dyn MonitorSink,
    ) -> &mut OptionsEntry {
        self.remove(id, oe.name, sink);
        self.insert_entry(id, OptionsEntry::new(oe.name, Some(oe)))
    }

    /// `options_default` with monitor cleanup of any replaced entry.
    pub fn default_with(
        &mut self,
        id: OptionsId,
        oe: &'static OptionsTableEntry,
        parser: &mut dyn CommandParser,
        sink: &mut dyn MonitorSink,
    ) -> &mut OptionsEntry {
        self.remove(id, oe.name, sink);
        let entry = OptionsEntry::from_default(oe, parser);
        self.insert_entry(id, entry)
    }

    /// First half of `options_remove` (`options.c:411-416`): destroy the
    /// value and detach the monitor while the entry stays linked.
    pub fn prepare_removal(&mut self, id: OptionsId, name: &[u8]) -> Option<OptionsRemoved> {
        let o = self.tree_mut(id).get_mut_only(name)?;
        o.value = empty_value(o.table);
        o.style_cache = None;
        Some(OptionsRemoved {
            owner: id,
            name: o.name.clone(),
            serial: o.serial,
            monitor: o.monitor.take(),
        })
    }

    /// Second half of `options_remove` (`options.c:417-419`): unlink the
    /// entry the token names, unless it was already replaced.
    pub fn finish_removal(&mut self, token: OptionsRemoved) {
        let tree = self.tree_mut(token.owner);
        if tree
            .entries
            .get(&token.name)
            .is_some_and(|o| o.serial == token.serial)
        {
            tree.entries.remove(&token.name);
        }
    }

    /// `options_remove`: the staged sequence with G11 cleanup in between.
    /// Returns whether an entry existed.
    pub fn remove(&mut self, id: OptionsId, name: &[u8], sink: &mut dyn MonitorSink) -> bool {
        let Some(token) = self.prepare_removal(id, name) else {
            return false;
        };
        if let Some(monitor) = token.monitor {
            sink.monitor_free(monitor);
        }
        self.finish_removal(token);
        true
    }

    /// `options_remove_or_default` (`options.c:1478-1495`).
    pub fn remove_or_default(
        &mut self,
        id: OptionsId,
        name: &[u8],
        key: Option<&OptionsArrayKey>,
        parser: &mut dyn CommandParser,
        sink: &mut dyn MonitorSink,
    ) -> Result<(), OptionsError> {
        match key {
            None => {
                let table = self.tree(id).get_only(name).and_then(|o| o.table);
                let is_global = id == self.global || id == self.global_s || id == self.global_w;
                match table {
                    Some(oe) if is_global => {
                        self.default_with(id, oe, parser, sink);
                    }
                    _ => {
                        self.remove(id, name, sink);
                    }
                }
                Ok(())
            }
            Some(key) => match self.tree_mut(id).get_mut_only(name) {
                Some(o) => o.array_set(key, None, false, parser),
                None => Ok(()),
            },
        }
    }

    /// `options_get_string` (`options.c:889-900`); panics like `fatalx`.
    pub fn get_string(&self, id: OptionsId, name: &[u8]) -> &[u8] {
        let Some((_, o)) = self.get(id, name) else {
            panic!("missing option {}", ByteString::from(name));
        };
        assert!(
            o.is_string(),
            "option {} is not a string",
            ByteString::from(name)
        );
        o.value.as_string()
    }

    /// `options_get_number` (`options.c:902-913`); panics like `fatalx`.
    pub fn get_number(&self, id: OptionsId, name: &[u8]) -> i64 {
        let Some((_, o)) = self.get(id, name) else {
            panic!("missing option {}", ByteString::from(name));
        };
        assert!(
            o.is_number(),
            "option {} is not a number",
            ByteString::from(name)
        );
        o.value.as_number().unwrap_or(0)
    }

    /// `options_get_command` (`options.c:915-926`); panics like `fatalx`.
    pub fn get_command(&self, id: OptionsId, name: &[u8]) -> Option<&Rc<CommandList>> {
        let Some((_, o)) = self.get(id, name) else {
            panic!("missing option {}", ByteString::from(name));
        };
        assert!(
            o.is_command(),
            "option {} is not a command",
            ByteString::from(name)
        );
        o.value.as_command()
    }

    /// `options_parent_table_entry` (`options.c:165-176`); panics like `fatalx`.
    fn parent_table_entry(&self, id: OptionsId, name: &[u8]) -> &'static OptionsTableEntry {
        let Some(parent) = self.tree(id).parent else {
            panic!("no parent options for {}", ByteString::from(name));
        };
        let Some((_, o)) = self.get(parent, name) else {
            panic!("{} not in parent options", ByteString::from(name));
        };
        o.table.expect("parent option without table entry")
    }

    /// Materialise a missing entry from the parent's table default
    /// (`options.c:952-958,976-981,998-1003`).
    fn materialise(&mut self, id: OptionsId, name: &[u8], parser: &mut dyn CommandParser) {
        if self.tree(id).get_only(name).is_some() {
            return;
        }
        if name.first() == Some(&b'@') {
            self.add(id, name, None);
        } else {
            let oe = self.parent_table_entry(id, name);
            self.default(id, oe, parser);
        }
    }

    /// `options_set_string` (`options.c:928-966`): append joins the local
    /// value, the table separator, and `value`.
    pub fn set_string(
        &mut self,
        id: OptionsId,
        name: &[u8],
        append: bool,
        value: &[u8],
        parser: &mut dyn CommandParser,
    ) -> &mut OptionsEntry {
        let mut new = ByteString::new();
        let existing = self.tree(id).get_only(name);
        if let Some(o) = existing.filter(|o| append && o.is_string()) {
            new.extend_from_slice(o.value.as_string());
            if name.first() != Some(&b'@') {
                new.extend_from_slice(o.table.and_then(|oe| oe.separator).unwrap_or(b""));
            }
        }
        new.extend_from_slice(value);
        self.materialise(id, name, parser);
        let o = self.tree_mut(id).get_mut_only(name).unwrap();
        assert!(
            o.is_string(),
            "option {} is not a string",
            ByteString::from(name)
        );
        o.value = OptionsValue::String(new);
        o.style_cache = None;
        o
    }

    /// `options_set_number` (`options.c:968-987`).
    pub fn set_number(
        &mut self,
        id: OptionsId,
        name: &[u8],
        value: i64,
        parser: &mut dyn CommandParser,
    ) -> &mut OptionsEntry {
        assert!(
            name.first() != Some(&b'@'),
            "user option {} must be a string",
            ByteString::from(name)
        );
        self.materialise(id, name, parser);
        let o = self.tree_mut(id).get_mut_only(name).unwrap();
        assert!(
            o.is_number(),
            "option {} is not a number",
            ByteString::from(name)
        );
        o.value = OptionsValue::Number(value);
        o
    }

    /// Numeric materialization never reaches command parsing (`options.c:968-987`).
    pub fn set_number_value(
        &mut self,
        id: OptionsId,
        name: &[u8],
        value: i64,
    ) -> &mut OptionsEntry {
        assert!(name.first() != Some(&b'@'), "user option must be a string");
        if self.get_only(id, name).is_none() {
            let table = self.parent_table_entry(id, name);
            let entry = OptionsEntry::new(name, Some(table));
            assert!(entry.is_number(), "option is not a number");
            self.insert_entry(id, entry);
        }
        let entry = self.get_mut_only(id, name).unwrap();
        assert!(entry.is_number(), "option is not a number");
        entry.value = OptionsValue::Number(value);
        entry
    }

    /// `options_set_command` (`options.c:989-1011`).
    pub fn set_command(
        &mut self,
        id: OptionsId,
        name: &[u8],
        value: Rc<CommandList>,
        parser: &mut dyn CommandParser,
    ) -> &mut OptionsEntry {
        assert!(
            name.first() != Some(&b'@'),
            "user option {} must be a string",
            ByteString::from(name)
        );
        self.materialise(id, name, parser);
        let o = self.tree_mut(id).get_mut_only(name).unwrap();
        assert!(
            o.is_command(),
            "option {} is not a command",
            ByteString::from(name)
        );
        o.value = OptionsValue::Command(Some(value));
        o
    }

    /// `options_parse_get` (`options.c:787-806`).
    pub fn parse_get(&self, id: OptionsId, s: &[u8], only: bool) -> Option<OptionsLookup<'_>> {
        let (name, key) = parse_name(s)?;
        let (owner, o) = if only {
            (id, self.get_only(id, name)?)
        } else {
            self.get(id, name)?
        };
        Some((owner, o, key))
    }

    /// `options_match_get` (`options.c:866-887`).
    pub fn match_get(
        &self,
        id: OptionsId,
        s: &[u8],
        only: bool,
    ) -> Result<Option<OptionsLookup<'_>>, super::Ambiguous> {
        let Some((name, key)) = match_name(s)? else {
            return Ok(None);
        };
        let name: &[u8] = match &name {
            OptionName::Table(n) => n,
            OptionName::User(n) => n,
        };
        let found = if only {
            self.get_only(id, name).map(|o| (id, o))
        } else {
            self.get(id, name)
        };
        Ok(found.map(|(owner, o)| (owner, o, key)))
    }

    /// `options_from_string_check` (`options.c:1183-1212`).
    fn from_string_check(
        oe: Option<&OptionsTableEntry>,
        value: &[u8],
        ctx: &mut OptionsParseCtx<'_>,
    ) -> Result<(), OptionsError> {
        let Some(oe) = oe else {
            return Ok(());
        };
        if oe.name == b"default-shell" && !check_shell(value, ctx.program) {
            return Err(OptionsError::new(b"not a suitable shell: ", value));
        }
        if let Some(pattern) = oe.pattern {
            if !fnmatch(pattern, value, FnmatchFlags::NONE) {
                return Err(OptionsError::new(b"value is invalid: ", value));
            }
        }
        let dynamic = contains_format(value);
        if oe.flags.contains(OptionsTableFlags::STYLE) && !dynamic {
            let mut sy = Style::default();
            if sy.parse(&DEFAULT_CELL, value, ctx.links).is_err() {
                return Err(OptionsError::new(b"invalid style: ", value));
            }
        }
        if oe.flags.contains(OptionsTableFlags::COLOUR) && !dynamic {
            let mut sy = Style::default();
            if sy.parse_colour(&DEFAULT_CELL, value).is_err() {
                return Err(OptionsError::new(b"invalid colour: ", value));
            }
        }
        Ok(())
    }

    /// `options_from_string_flag` (`options.c:1214-1236`).
    fn set_flag_from_string(
        &mut self,
        id: OptionsId,
        name: &[u8],
        value: Option<&[u8]>,
        parser: &mut dyn CommandParser,
    ) -> Result<(), OptionsError> {
        let flag = match value {
            None | Some(b"") => i64::from(self.get_number(id, name) == 0),
            Some(v)
                if v == b"1" || v.eq_ignore_ascii_case(b"on") || v.eq_ignore_ascii_case(b"yes") =>
            {
                1
            }
            Some(v)
                if v == b"0" || v.eq_ignore_ascii_case(b"off") || v.eq_ignore_ascii_case(b"no") =>
            {
                0
            }
            Some(v) => return Err(OptionsError::new(b"bad value: ", v)),
        };
        self.set_number(id, name, flag, parser);
        Ok(())
    }

    /// `options_from_string_choice` (`options.c:1257-1274`).
    fn set_choice_from_string(
        &mut self,
        id: OptionsId,
        oe: &OptionsTableEntry,
        name: &[u8],
        value: Option<&[u8]>,
        parser: &mut dyn CommandParser,
    ) -> Result<(), OptionsError> {
        let choice = match value {
            None => {
                let current = self.get_number(id, name);
                if current < 2 {
                    i64::from(current == 0)
                } else {
                    current
                }
            }
            Some(v) => find_choice(oe, v)?,
        };
        self.set_number(id, name, choice, parser);
        Ok(())
    }

    /// `options_from_string` (`options.c:1276-1356`).
    pub fn from_string(
        &mut self,
        id: OptionsId,
        oe: Option<&'static OptionsTableEntry>,
        name: &[u8],
        value: Option<&[u8]>,
        append: bool,
        ctx: &mut OptionsParseCtx<'_>,
    ) -> Result<(), OptionsError> {
        let kind = match oe {
            Some(oe) => {
                if value.is_none()
                    && oe.kind != OptionsTableType::Flag
                    && oe.kind != OptionsTableType::Choice
                {
                    return Err(OptionsError::text(b"empty value"));
                }
                oe.kind
            }
            None => {
                if name.first() != Some(&b'@') {
                    return Err(OptionsError::text(b"bad option name"));
                }
                OptionsTableType::String
            }
        };
        match kind {
            OptionsTableType::String => {
                let value = value.unwrap_or(b"");
                let old = ByteString::from(self.get_string(id, name));
                self.set_string(id, name, append, value, ctx.parser);
                let new = self.get_string(id, name);
                if let Err(cause) = Self::from_string_check(oe, new, ctx) {
                    self.set_string(id, name, false, &old, ctx.parser);
                    return Err(cause);
                }
                Ok(())
            }
            OptionsTableType::Number => {
                let oe = oe.unwrap();
                let value = value.unwrap();
                let number = strtonum(value, i64::from(oe.minimum), i64::from(oe.maximum))
                    .map_err(|e| {
                        let errstr: &[u8] = match e {
                            StrtonumError::Invalid => b"invalid",
                            StrtonumError::TooSmall => b"too small",
                            StrtonumError::TooLarge => b"too large",
                        };
                        let mut cause = ByteString::from("value is ");
                        cause.extend_from_slice(errstr);
                        cause.extend_from_slice(b": ");
                        cause.extend_from_slice(value);
                        OptionsError(cause)
                    })?;
                self.set_number(id, name, number, ctx.parser);
                Ok(())
            }
            OptionsTableType::Key => {
                let value = value.unwrap();
                let key = parse_key_name(value);
                if key.0 == SpecialKey::UNKNOWN {
                    return Err(OptionsError::new(b"bad key: ", value));
                }
                self.set_number(id, name, key.0 as i64, ctx.parser);
                Ok(())
            }
            OptionsTableType::Colour => {
                let value = value.unwrap();
                let number = colour_from_string(value)
                    .ok_or_else(|| OptionsError::new(b"bad colour: ", value))?;
                self.set_number(id, name, number, ctx.parser);
                Ok(())
            }
            OptionsTableType::Flag => self.set_flag_from_string(id, name, value, ctx.parser),
            OptionsTableType::Choice => {
                self.set_choice_from_string(id, oe.unwrap(), name, value, ctx.parser)
            }
            OptionsTableType::Command => {
                let list = ctx
                    .parser
                    .parse_from_string(value.unwrap())
                    .map_err(|e| OptionsError::text(e.message()))?;
                self.set_command(id, name, list, ctx.parser);
                Ok(())
            }
        }
    }

    /// `options_string_to_style` (`options.c:1139-1181`): a literal value is
    /// parsed once and cached (even when the parse fails); a `#{` value is
    /// expanded through `ft` when given and never cached.
    pub fn string_to_style(
        &mut self,
        id: OptionsId,
        name: &[u8],
        ft: Option<&mut dyn FormatExpander>,
        links: &mut HyperlinkRegistry,
    ) -> Option<Style> {
        let (owner, o) = self.get(id, name)?;
        if !o.is_string() {
            return None;
        }
        if let Some(style) = o.style_cache {
            return Some(style);
        }
        let is_colour = o
            .table
            .is_some_and(|oe| oe.flags.contains(OptionsTableFlags::COLOUR));
        let s = o.value.as_string();
        log_debug!(
            "options_string_to_style: {} is '{}'",
            ByteString::from(name),
            String::from_utf8_lossy(s)
        );
        let mut style = Style::from_cell(DEFAULT_CELL);
        let cached = !contains_format(s);
        let parse = |style: &mut Style, bytes: &[u8], links: &mut HyperlinkRegistry| {
            if is_colour {
                style.parse_colour(&DEFAULT_CELL, bytes).is_ok()
            } else {
                style.parse(&DEFAULT_CELL, bytes, links).is_ok()
            }
        };
        if let (Some(ft), false) = (ft, cached) {
            let expanded = ft.expand(s);
            return parse(&mut style, &expanded, links).then_some(style);
        }
        let ok = parse(&mut style, s, links);
        if cached {
            let o = self.tree_mut(owner).get_mut_only(name).unwrap();
            o.style_cache = Some(style);
        }
        ok.then_some(style)
    }
}

/// `strstr(s, "#{") != NULL`
fn contains_format(s: &[u8]) -> bool {
    cstr(s).windows(2).any(|w| w == b"#{")
}
