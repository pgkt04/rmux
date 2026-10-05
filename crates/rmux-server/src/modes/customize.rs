// Ported from tmux window-customize.c @ 8f25579c
/*
 * Copyright (c) 2020 Nicholas Marriott <nicholas.marriott@gmail.com>
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

use std::rc::Rc;

use crate::client::ResolvedMouseEvent;
use crate::cmd::CommandListPrintFlags;
use crate::cmd::arguments::Args;
use crate::cmd::find::{CmdFindFlags, CmdFindState, from_pane};
use crate::cmd::hooks;
use crate::format::{self, FormatTree};
use crate::ids::{
    ClientId, EditorId, HooksMonitorId, KeyTableId, ModeId, OptionsId, PaneId, SessionId,
};
use crate::model::pane::{PaneMode, PaneModeDriver, pane_index, pane_reset_mode};
use crate::model::spawn::{SpawnContext, spawn_cancel_editor, spawn_editor};
use crate::model::{ModelError, PaneFlags};
use crate::modes::buffer::draw_waiting;
use crate::modes::tree::{
    ModeAction, ModeTreeCallbacks, ModeTreeData, ModeTreeItemId, ModeTreeMenuItem, ModeTreeTag,
    TreeModeState,
};
use crate::options::environment::{Environment, EnvironmentFlags};
use crate::options::push::push_changes;
use crate::options::{
    self, CommandParser, MonitorSink, OPTIONS_TABLE, OptionName, OptionsArrayKey, OptionsEntry,
    OptionsParseCtx, OptionsScope, OptionsStore, OptionsTableEntry, OptionsTableFlags,
    OptionsTableType, match_name,
};
use crate::server::Server;
use crate::ui::prompt::{PromptCreateData, PromptFlags, PromptType};
use crate::ui::status::status_message_set;
use crate::ui::styles::style_apply;
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::{Colour, ColourFlags, ColourTheme};
use rmux_emu::screen::write::ScreenWriteCtx;
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_tty::key_string::{key_name, parse_key_name};
use rmux_util::bytes::ByteString;
use rmux_util::key::{C0, KeyCode, SpecialKey};
use rmux_util::time::Timestamp;

pub const NAME: &[u8] = b"options-mode";
pub const DEFAULT_FORMAT: &[u8] = b"#{?is_option,\
#{?option_is_global,,#[reverse](#{option_scope})#[default] }\
#[fg=themelightgrey]#[ignore]#{option_value}\
#{?option_unit, #{option_unit},}\
,\
#{?is_environment,\
#[fg=themelightgrey]#[ignore]#{environment_value}\
,\
#{key}\
}\
}";

const NONE: u64 = SpecialKey::NONE;
pub const MENU_ITEMS: &[ModeTreeMenuItem] = &[
    ModeTreeMenuItem {
        name: "Select",
        key: b'\r' as u64,
    },
    ModeTreeMenuItem {
        name: "Edit",
        key: b'e' as u64,
    },
    ModeTreeMenuItem {
        name: "Expand",
        key: SpecialKey::RIGHT,
    },
    ModeTreeMenuItem {
        name: "",
        key: NONE,
    },
    ModeTreeMenuItem {
        name: "Tag",
        key: b't' as u64,
    },
    ModeTreeMenuItem {
        name: "Tag All",
        key: 0o24,
    },
    ModeTreeMenuItem {
        name: "Tag None",
        key: b'T' as u64,
    },
    ModeTreeMenuItem {
        name: "",
        key: NONE,
    },
    ModeTreeMenuItem {
        name: "Changed Only",
        key: b'C' as u64,
    },
    ModeTreeMenuItem {
        name: "",
        key: NONE,
    },
    ModeTreeMenuItem {
        name: "Cancel",
        key: b'q' as u64,
    },
];

pub const HELP_LINES: &[&str] = &[
    "#[fg=themelightgrey]   Enter, s #[#{E:tree-mode-border-style},acs]x#[default] Set %1 value",
    "#[fg=themelightgrey]          S #[#{E:tree-mode-border-style},acs]x#[default] Set global %1 value",
    "#[fg=themelightgrey]          w #[#{E:tree-mode-border-style},acs]x#[default] Set window %1 value",
    "#[fg=themelightgrey]          d #[#{E:tree-mode-border-style},acs]x#[default] Set to default value",
    "#[fg=themelightgrey]          D #[#{E:tree-mode-border-style},acs]x#[default] Set tagged %1s to default value",
    "#[fg=themelightgrey]          u #[#{E:tree-mode-border-style},acs]x#[default] Unset an %1",
    "#[fg=themelightgrey]          U #[#{E:tree-mode-border-style},acs]x#[default] Unset tagged %1s",
    "#[fg=themelightgrey]          a #[#{E:tree-mode-border-style},acs]x#[default] Change array key",
    "#[fg=themelightgrey]          e #[#{E:tree-mode-border-style},acs]x#[default] Open %1 value in editor",
    "#[fg=themelightgrey]          f #[#{E:tree-mode-border-style},acs]x#[default] Enter a filter",
    "#[fg=themelightgrey]          C #[#{E:tree-mode-border-style},acs]x#[default] Toggle only changed items",
    "#[fg=themelightgrey]          v #[#{E:tree-mode-border-style},acs]x#[default] Toggle information",
];

/// `(3ULL << 62)|...` section tags: kind, then the options table scope bit
/// or the environment discriminator (1 global, 3 session).
pub const SECTION_OPTIONS: u8 = 0;
pub const SECTION_HOOKS: u8 = 1;
pub const SECTION_ENVIRONMENT: u8 = 2;

/// Object kinds inside `ModeTreeTag::Serial`.
pub const TAG_OPTION: u8 = 1;
pub const TAG_ARRAY_ITEM: u8 = 2;
pub const TAG_ENVIRONMENT: u8 = 3;
pub const TAG_KEY_TABLE: u8 = 4;
pub const TAG_KEY_BINDING: u8 = 5;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum CustomizeScope {
    #[default]
    None,
    Key,
    Server,
    GlobalSession,
    Session,
    GlobalWindow,
    Window,
    Pane,
    GlobalEnvironment,
    SessionEnvironment,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CustomizeChange {
    #[default]
    Unset,
    Reset,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CustomizeOptionType {
    #[default]
    Options,
    Hooks,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CustomizeItemType {
    #[default]
    Option,
    Key,
    Environment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CustomizeEditType {
    Option,
    KeyCommand,
    KeyNote,
    Environment,
}

/// `item->environ`: the global environment or a session's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentRef {
    Global,
    Session(SessionId),
}

/// `struct window_customize_itemdata`.
#[derive(Clone, Debug)]
pub struct CustomizeItem {
    pub kind: CustomizeItemType,
    pub option_type: CustomizeOptionType,
    pub scope: CustomizeScope,
    pub table: Option<ByteString>,
    pub key: KeyCode,
    pub oo: Option<OptionsId>,
    pub environment: Option<EnvironmentRef>,
    pub environment_flags: EnvironmentFlags,
    pub name: ByteString,
    pub array_key: Option<ByteString>,
}

impl CustomizeItem {
    fn new(kind: CustomizeItemType) -> Self {
        Self {
            kind,
            option_type: CustomizeOptionType::Options,
            scope: CustomizeScope::None,
            table: None,
            key: KeyCode(NONE),
            oo: None,
            environment: None,
            environment_flags: EnvironmentFlags(0),
            name: ByteString::new(),
            array_key: None,
        }
    }
}

/// One step of the preview text, computed in `prepare_draw` and replayed in
/// `draw` with the `screen_write_text` flow control of the C code.
#[derive(Clone)]
enum DrawOp {
    Text {
        more: bool,
        gc: GridCell,
        text: ByteString,
    },
    Value {
        label: ByteString,
        value: ByteString,
    },
    Skip {
        strict: bool,
    },
}

pub struct CustomizeBackend {
    pub wp: PaneId,
    pub fs: CmdFindState,
    pub format: Vec<u8>,
    pub hide_global: bool,
    pub hide_default: bool,
    pub prompt_flags: PromptFlags,
    pub editor: Option<EditorId>,
    pub change: CustomizeChange,
    pub items: Vec<CustomizeItem>,
    cache: Vec<DrawOp>,
}

pub type CustomizeState = TreeModeState<CustomizeBackend>;

pub struct CustomizeMode {
    args: Args,
    fs: CmdFindState,
}

impl CustomizeMode {
    pub fn new(args: Args, fs: CmdFindState) -> Self {
        Self { args, fs }
    }
}

fn concat(parts: &[&[u8]]) -> ByteString {
    let mut out = ByteString::with_capacity(parts.iter().map(|p| p.len()).sum());
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

fn flag(value: bool) -> ByteString {
    ByteString::from(if value { "1" } else { "0" })
}

fn grey(text: &[u8]) -> ByteString {
    concat(&[b"#[fg=themelightgrey]#[ignore]", text])
}

fn light_grey() -> Colour {
    Colour(ColourTheme::LightGrey as i32 | ColourFlags::THEME.bits() as i32)
}

fn text_op(text: ByteString) -> DrawOp {
    DrawOp::Text {
        more: false,
        gc: DEFAULT_CELL,
        text,
    }
}

fn value_op(label: &[u8], value: ByteString) -> DrawOp {
    DrawOp::Value {
        label: label.into(),
        value,
    }
}

pub fn section_tag(kind: u8, scope: u8) -> ModeTreeTag {
    ModeTreeTag::Section { kind, scope }
}

pub fn binding_tag(serial: u64, subitem: u8) -> ModeTreeTag {
    ModeTreeTag::Serial {
        kind: TAG_KEY_BINDING,
        value: serial,
        subitem,
    }
}

/// `window_customize_get_tag` for an option entry (table index when the
/// option is in the table, else the entry serial).
pub fn option_tag(serial: u64, oe: Option<&'static OptionsTableEntry>) -> ModeTreeTag {
    match oe.and_then(|oe| OPTIONS_TABLE.iter().position(|e| std::ptr::eq(e, oe))) {
        Some(index) => ModeTreeTag::OptionTable(index),
        None => ModeTreeTag::Serial {
            kind: TAG_OPTION,
            value: serial,
            subitem: 0,
        },
    }
}

/// `data->fs` when valid, else the state of the mode pane.
fn resolve_fs(s: &Server, fs: &CmdFindState, wp: PaneId) -> Option<CmdFindState> {
    if fs.is_valid(s) {
        Some(*fs)
    } else {
        from_pane(s, wp, CmdFindFlags::default())
    }
}

/// `window_customize_get_tree`.
fn get_tree(s: &Server, scope: CustomizeScope, fs: &CmdFindState) -> Option<OptionsId> {
    match scope {
        CustomizeScope::Server => Some(s.options.global),
        CustomizeScope::GlobalSession => Some(s.options.global_s),
        CustomizeScope::Session => s.sessions.get(fs.s?).map(|x| x.options),
        CustomizeScope::GlobalWindow => Some(s.options.global_w),
        CustomizeScope::Window => s.windows.get(fs.w?).map(|w| w.options),
        CustomizeScope::Pane => s.panes.get(fs.wp?).map(|p| p.options),
        _ => None,
    }
}

/// `window_customize_get_environment`.
fn get_environment(scope: CustomizeScope, fs: &CmdFindState) -> Option<EnvironmentRef> {
    match scope {
        CustomizeScope::GlobalEnvironment => Some(EnvironmentRef::Global),
        CustomizeScope::SessionEnvironment => fs.s.map(EnvironmentRef::Session),
        _ => None,
    }
}

fn environment(s: &Server, env: EnvironmentRef) -> Option<&Environment> {
    match env {
        EnvironmentRef::Global => Some(&s.global_environment),
        EnvironmentRef::Session(id) => s.sessions.get(id).map(|x| &x.environment),
    }
}

fn environment_mut(s: &mut Server, env: EnvironmentRef) -> Option<&mut Environment> {
    match env {
        EnvironmentRef::Global => Some(&mut s.global_environment),
        EnvironmentRef::Session(id) => s.sessions.get_mut(id).map(|x| &mut x.environment),
    }
}

/// `window_customize_check_item`: the find state when the item still names
/// the tree or environment it was built from.
fn check_item(
    s: &Server,
    fs: &CmdFindState,
    wp: PaneId,
    item: &CustomizeItem,
) -> Option<CmdFindState> {
    let fs = resolve_fs(s, fs, wp)?;
    let ok = if item.kind == CustomizeItemType::Environment {
        item.environment == get_environment(item.scope, &fs)
    } else {
        item.oo == get_tree(s, item.scope, &fs)
    };
    ok.then_some(fs)
}

/// `window_customize_get_key`: the table and key of a bound key item.
fn get_key(s: &Server, item: &CustomizeItem) -> Option<(KeyTableId, KeyCode)> {
    let table = item.table.as_ref()?;
    let id = s.key_bindings.find_table(table)?;
    s.key_bindings.get(id, item.key)?;
    Some((id, item.key))
}

fn table_name(s: &Server, id: KeyTableId) -> Option<ByteString> {
    s.key_bindings.tables.get(id).map(|t| t.name.clone())
}

/// `window_customize_scope_text`.
fn scope_text(s: &Server, scope: CustomizeScope, fs: &CmdFindState) -> ByteString {
    match scope {
        CustomizeScope::Pane => {
            let idx = fs.wp.and_then(|wp| pane_index(s, wp)).unwrap_or(0);
            ByteString::from(format!("pane {idx}"))
        }
        CustomizeScope::Session | CustomizeScope::SessionEnvironment => {
            let name =
                fs.s.and_then(|id| s.sessions.get(id))
                    .map(|x| x.name.as_slice())
                    .unwrap_or(b"");
            concat(&[b"session ", name])
        }
        CustomizeScope::Window => {
            let idx = fs
                .wl
                .and_then(|wl| s.winlinks.get(wl))
                .map(|wl| wl.index)
                .unwrap_or(0);
            ByteString::from(format!("window {idx}"))
        }
        _ => ByteString::new(),
    }
}

/// `options_to_string(o, item->array_key, 0)`.
fn value_string(o: &OptionsEntry, array_key: Option<&ByteString>) -> ByteString {
    match array_key {
        Some(key) => match OptionsArrayKey::parse(key) {
            Ok(key) => o.to_string(Some(&key), false),
            Err(_) => ByteString::new(),
        },
        None => o.to_string(None, false),
    }
}

/// The first unused numeric key below `INT_MAX` (`window-customize.c:454-463`).
fn first_unused_index(o: &OptionsEntry) -> u32 {
    let limit = i32::MAX as u32;
    (0..limit)
        .find(|i| o.array_get(&OptionsArrayKey::Index(*i)).is_none())
        .unwrap_or(limit)
}

/// Monitors detached by an options removal, freed once the store borrow ends
/// (same convention as `cmd/commands/set_option.rs`).
#[derive(Default)]
struct FreedMonitors(Vec<HooksMonitorId>);

impl MonitorSink for FreedMonitors {
    fn monitor_free(&mut self, monitor: HooksMonitorId) {
        self.0.push(monitor);
    }
}

fn with_store<R>(
    server: &mut Server,
    f: impl FnOnce(&mut OptionsStore, &mut OptionsParseCtx<'_>, &mut FreedMonitors) -> R,
) -> R {
    let mut store = std::mem::take(&mut server.options);
    let mut links = std::mem::take(&mut server.hyperlinks);
    let mut freed = FreedMonitors::default();
    let result = {
        let mut ctx = OptionsParseCtx {
            parser: server,
            links: &mut links,
            program: b"rmux",
        };
        f(&mut store, &mut ctx, &mut freed)
    };
    server.hyperlinks = links;
    server.options = store;
    for monitor in freed.0 {
        hooks::monitor_free(server, monitor);
    }
    result
}

/// `options_push_changes` plus the server-side application.
fn push(server: &mut Server, name: &[u8]) {
    let changes = push_changes(name);
    crate::server::run::apply_option_changes(server, changes);
}

/// `window_customize_option_is_changed`.
pub fn option_is_changed(
    s: &mut Server,
    owner: OptionsId,
    name: &[u8],
    array_key: Option<&OptionsArrayKey>,
) -> bool {
    if name.first() == Some(&b'@') && hooks::is_event(&s.hooks, name) {
        return true;
    }
    with_store(s, |store, ctx, _| {
        let Some(o) = store.get_only(owner, name) else {
            return true;
        };
        let Some(oe) = o.table_entry() else {
            return true;
        };
        if o.monitor().is_some() {
            return true;
        }
        if oe.is_array() {
            let has = array_key.map(|key| o.array_get(key).is_some());
            let value = o.to_string(array_key, false);
            let tmp = store.create(None);
            let defaults = store.default(tmp, oe, &mut *ctx.parser);
            let changed = match (array_key, has) {
                (Some(key), Some(has)) => {
                    let default_has = defaults.array_get(key).is_some();
                    if !has || !default_has {
                        has != default_has
                    } else {
                        value != defaults.to_string(Some(key), false)
                    }
                }
                _ => value != defaults.to_string(None, false),
            };
            store.free(tmp);
            return changed;
        }
        o.to_string(None, false) != options::default_to_string(oe)
    })
}

/// `window_customize_key_is_changed`.
pub fn key_is_changed(
    kt: &crate::cmd::key_bindings::KeyTable,
    bd: &crate::cmd::key_bindings::KeyBinding,
) -> bool {
    let Some(default) = kt.get_default(bd.key) else {
        return true;
    };
    if bd.flags != default.flags {
        return true;
    }
    if bd.note.is_some() != default.note.is_some() {
        return true;
    }
    if bd.note.is_some() && bd.note != default.note {
        return true;
    }
    bd.list.print(CommandListPrintFlags(0)) != default.list.print(CommandListPrintFlags(0))
}

fn no_tag(tree: &mut ModeTreeData, id: ModeTreeItemId) {
    if let Some(item) = tree.items.get_mut(id) {
        item.no_tag = true;
    }
}

fn child_flags(tree: &mut ModeTreeData, id: ModeTreeItemId) {
    if let Some(item) = tree.items.get_mut(id) {
        item.draw_as_parent = true;
        item.no_tag = true;
    }
}

/// `mode_tree_remove` for a root: the engine has no removal entry yet, so
/// the root and its subtree leave the arena here.
fn remove_root(tree: &mut ModeTreeData, id: ModeTreeItemId) {
    tree.children.retain(|c| *c != id);
    let mut stack = vec![id];
    while let Some(id) = stack.pop() {
        if let Ok(Some(item)) = tree.items.request_remove(id) {
            stack.extend(item.children);
        }
    }
}

/// What `build_keys` copies out of a binding before the format tree needs
/// the server.
struct BindingSnapshot {
    key: KeyCode,
    serial: u64,
    note: Option<ByteString>,
    repeat: bool,
    cmd: ByteString,
    changed: bool,
}

impl CustomizeBackend {
    fn push_item(&mut self, item: CustomizeItem) -> u32 {
        self.items.push(item);
        (self.items.len() - 1) as u32
    }

    /// `window_customize_build_array`.
    #[allow(clippy::too_many_arguments)]
    fn build_array(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        top: ModeTreeItemId,
        scope: CustomizeScope,
        owner: OptionsId,
        name: &[u8],
        ft: &mut FormatTree,
    ) -> u32 {
        let Some(o) = s.options.get_only(owner, name) else {
            return 0;
        };
        let is_hook = o.table_entry().is_some_and(|oe| oe.is_hook());
        let elements: Vec<(OptionsArrayKey, u64, ByteString)> = o
            .array_items()
            .map(|(key, ai)| (key.clone(), ai.serial(), o.to_string(Some(key), false)))
            .collect();
        let mut count = 0;
        for (key, serial, value) in elements {
            if self.hide_default && !option_is_changed(s, owner, name, Some(&key)) {
                continue;
            }
            let key_text = key.to_bytes();
            let display = concat(&[name, b"[", &key_text, b"]"]);
            ft.add(b"option_name", display.clone());
            ft.add(b"option_value", value);
            let mut item = CustomizeItem::new(CustomizeItemType::Option);
            if is_hook {
                item.option_type = CustomizeOptionType::Hooks;
            }
            item.scope = scope;
            item.oo = Some(owner);
            item.name = name.into();
            item.array_key = Some(key_text);
            let idx = self.push_item(item);
            let text = ft.expand(s, &self.format);
            let tag = ModeTreeTag::Serial {
                kind: TAG_ARRAY_ITEM,
                value: serial,
                subitem: 0,
            };
            tree.add(Some(top), Some(idx), tag, &display, Some(&text), -1);
            count += 1;
        }
        count
    }

    /// `window_customize_build_option`.
    #[allow(clippy::too_many_arguments)]
    fn build_option(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        top: ModeTreeItemId,
        scope: CustomizeScope,
        owner: OptionsId,
        name: &[u8],
        ft: &mut FormatTree,
        filter: Option<&[u8]>,
        fs: &CmdFindState,
        kind: CustomizeOptionType,
    ) -> u32 {
        let Some(o) = s.options.get_only(owner, name) else {
            return 0;
        };
        let oe = o.table_entry();
        let serial = o.serial();
        let monitor = o.monitor();
        let is_hook = oe.is_some_and(|oe| oe.is_hook());
        let is_monitor = monitor.is_some();
        let is_user_hook = name.first() == Some(&b'@') && hooks::is_event(&s.hooks, name);
        let is_any_hook = is_hook || is_monitor || is_user_hook;
        match kind {
            CustomizeOptionType::Options if is_any_hook => return 0,
            CustomizeOptionType::Hooks if !is_any_hook => return 0,
            _ => {}
        }
        let array = oe.is_some_and(|oe| oe.is_array());
        let global = matches!(
            scope,
            CustomizeScope::Server | CustomizeScope::GlobalSession | CustomizeScope::GlobalWindow
        );
        if self.hide_global && global {
            return 0;
        }
        let value = (!array).then(|| o.to_string(None, false));
        let monitor_text = monitor
            .and_then(|id| s.hooks.monitors.get(id).and_then(Option::as_ref))
            .map(|m| hooks::monitor_to_string(name, m))
            .unwrap_or_default();
        if self.hide_default && !option_is_changed(s, owner, name, None) {
            return 0;
        }

        ft.add(b"option_name", name.into());
        ft.add(b"option_is_global", flag(global));
        ft.add(b"option_is_array", flag(array));
        ft.add(b"option_is_hook", flag(is_hook));
        ft.add(b"option_is_monitor", flag(is_monitor));
        ft.add(b"option_scope", scope_text(s, scope, fs));
        ft.add(
            b"option_unit",
            oe.and_then(|oe| oe.unit).unwrap_or(b"").into(),
        );
        ft.add(b"option_monitor", monitor_text);
        if let Some(value) = value {
            ft.add(b"option_value", value);
        }
        if let Some(filter) = filter {
            let expanded = ft.expand(s, filter);
            if !format::true_value(Some(&expanded)) {
                return 0;
            }
        }
        let mut item = CustomizeItem::new(CustomizeItemType::Option);
        item.option_type = kind;
        item.oo = Some(owner);
        item.scope = scope;
        item.name = name.into();
        let idx = self.push_item(item);

        let text = (!array).then(|| ft.expand(s, &self.format));
        let tag = option_tag(serial, oe);
        let top = tree.add(
            Some(top),
            Some(idx),
            tag,
            name,
            text.as_ref().map(ByteString::as_bytes),
            0,
        );
        if !array {
            return 1;
        }
        1 + self.build_array(s, tree, top, scope, owner, name, ft)
    }

    /// `window_customize_build_options`: `trees` is `(scope0, oo0)`,
    /// `(scope1, oo1)`, `(scope2, oo2)`.
    #[allow(clippy::too_many_arguments)]
    fn build_options(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        title: &[u8],
        tag: ModeTreeTag,
        trees: [(CustomizeScope, Option<OptionsId>); 3],
        ft: &mut FormatTree,
        filter: Option<&[u8]>,
        fs: &CmdFindState,
        kind: CustomizeOptionType,
    ) {
        let top = tree.add(None, None, tag, title, None, 0);
        no_tag(tree, top);
        let [(scope0, oo0), (scope1, oo1), (scope2, oo2)] = trees;
        let Some(oo0) = oo0 else {
            return;
        };
        let scope_for = |owner: OptionsId| {
            if Some(owner) == oo2 {
                scope2
            } else if Some(owner) == oo1 {
                scope1
            } else {
                scope0
            }
        };

        // Options come from the first tree but are built from the most
        // specific tree; user options may live in any of the three.
        let mut list: Vec<ByteString> = Vec::new();
        for oo in [Some(oo0), oo1, oo2].into_iter().flatten() {
            for o in s.options.entries(oo) {
                let name = o.name();
                if name.first() != Some(&b'@') || list.iter().any(|n| n.as_bytes() == name) {
                    continue;
                }
                list.push(name.into());
            }
        }
        let mut plan: Vec<(CustomizeScope, OptionsId, ByteString)> = Vec::new();
        for name in &list {
            let mut found = oo2.and_then(|oo| s.options.get(oo, name));
            if found.is_none() {
                found = oo1.and_then(|oo| s.options.get(oo, name));
            }
            if found.is_none() {
                found = s.options.get(oo0, name);
            }
            if let Some((owner, _)) = found {
                plan.push((scope_for(owner), owner, name.clone()));
            }
        }
        for o in s.options.entries(oo0) {
            let name = o.name();
            if name.first() == Some(&b'@') {
                continue;
            }
            let found = if let Some(oo) = oo2 {
                s.options.get(oo, name)
            } else if let Some(oo) = oo1 {
                s.options.get(oo, name)
            } else {
                Some((oo0, o))
            };
            if let Some((owner, _)) = found {
                plan.push((scope_for(owner), owner, name.into()));
            }
        }

        let mut count = 0;
        for (scope, owner, name) in plan {
            count += self.build_option(s, tree, top, scope, owner, &name, ft, filter, fs, kind);
        }
        if self.hide_default && count == 0 {
            remove_root(tree, top);
        }
    }

    /// `window_customize_build_keys`.
    fn build_keys(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        id: KeyTableId,
        filter: Option<&[u8]>,
        fs: &CmdFindState,
    ) {
        let Some(kt) = s.key_bindings.tables.get(id) else {
            return;
        };
        let table = kt.name.clone();
        let table_serial = kt.serial();
        let bindings: Vec<BindingSnapshot> = kt
            .bindings()
            .map(|bd| BindingSnapshot {
                key: bd.key,
                serial: bd.serial(),
                note: bd.note.clone(),
                repeat: bd
                    .flags
                    .contains(crate::cmd::key_bindings::KeyBindingFlags::REPEAT),
                cmd: bd.list.print(CommandListPrintFlags(0)),
                changed: key_is_changed(kt, bd),
            })
            .collect();

        let title = concat(&[b"Key Table - ", &table]);
        let top = tree.add(
            None,
            None,
            ModeTreeTag::Serial {
                kind: TAG_KEY_TABLE,
                value: table_serial,
                subitem: 0,
            },
            &title,
            None,
            0,
        );
        no_tag(tree, top);

        let mut ft = format::create_from_state(s, None, None, fs);
        ft.add(b"is_option", flag(false));
        ft.add(b"is_key", flag(true));
        ft.add(b"is_environment", flag(false));

        let mut count = 0;
        for BindingSnapshot {
            key,
            serial,
            note,
            repeat,
            cmd,
            changed,
        } in bindings
        {
            if self.hide_default && !changed {
                continue;
            }
            let name = ByteString::from(key_name(key, false));
            ft.add(b"key", name.clone());
            if let Some(note) = &note {
                ft.add(b"key_note", note.clone());
            }
            if let Some(filter) = filter {
                let expanded = ft.expand(s, filter);
                if !format::true_value(Some(&expanded)) {
                    continue;
                }
            }
            let mut item = CustomizeItem::new(CustomizeItemType::Key);
            item.scope = CustomizeScope::Key;
            item.table = Some(table.clone());
            item.key = key;
            item.name = name;
            let idx = self.push_item(item);

            let expanded = ft.expand(s, &self.format);
            let child = tree.add(
                Some(top),
                Some(idx),
                binding_tag(serial, 0),
                &expanded,
                None,
                0,
            );

            let text = grey(&cmd);
            let mti = tree.add(
                Some(child),
                Some(idx),
                binding_tag(serial, 1),
                b"Command",
                Some(&text),
                -1,
            );
            child_flags(tree, mti);

            let text = note.as_ref().map(|n| grey(n)).unwrap_or_default();
            let mti = tree.add(
                Some(child),
                Some(idx),
                binding_tag(serial, 2),
                b"Note",
                Some(&text),
                -1,
            );
            child_flags(tree, mti);

            let text = grey(if repeat { b"on".as_slice() } else { b"off" });
            let mti = tree.add(
                Some(child),
                Some(idx),
                binding_tag(serial, 3),
                b"Repeat",
                Some(&text),
                -1,
            );
            child_flags(tree, mti);

            count += 1;
        }
        ft.release(s);
        if self.hide_default && count == 0 {
            remove_root(tree, top);
        }
    }

    /// `window_customize_build_environment`.
    #[allow(clippy::too_many_arguments)]
    fn build_environment(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        title: &[u8],
        tag: ModeTreeTag,
        scope: CustomizeScope,
        env: EnvironmentRef,
        ft: &mut FormatTree,
        filter: Option<&[u8]>,
        fs: &CmdFindState,
    ) {
        if self.hide_default {
            return;
        }
        let top = tree.add(None, None, tag, title, None, 0);
        no_tag(tree, top);

        let global = scope == CustomizeScope::GlobalEnvironment;
        ft.add(b"is_option", flag(false));
        ft.add(b"is_key", flag(false));
        ft.add(b"is_environment", flag(true));
        ft.add(b"environment_is_global", flag(global));
        ft.add(b"environment_scope", scope_text(s, scope, fs));

        let Some(e) = environment(s, env) else {
            return;
        };
        let entries: Vec<(ByteString, Option<ByteString>, EnvironmentFlags, u64)> = e
            .iter()
            .map(|(name, entry)| {
                (
                    name.into(),
                    entry.value.clone(),
                    entry.flags,
                    entry.serial(),
                )
            })
            .collect();
        for (name, value, flags, serial) in entries {
            ft.add(b"environment_name", name.clone());
            ft.add(
                b"environment_hidden",
                flag(flags.contains(EnvironmentFlags::HIDDEN)),
            );
            ft.add(b"environment_removed", flag(value.is_none()));
            ft.add(b"environment_value", value.clone().unwrap_or_default());
            if let Some(filter) = filter {
                let expanded = ft.expand(s, filter);
                if !format::true_value(Some(&expanded)) {
                    continue;
                }
            }
            let mut item = CustomizeItem::new(CustomizeItemType::Environment);
            item.scope = scope;
            item.environment = Some(env);
            item.environment_flags = flags;
            item.name = name.clone();
            let idx = self.push_item(item);

            let (display, text) = match value {
                None => (concat(&[b"-", &name]), None),
                Some(_) => (name, Some(ft.expand(s, &self.format))),
            };
            let tag = ModeTreeTag::Serial {
                kind: TAG_ENVIRONMENT,
                value: serial,
                subitem: 0,
            };
            tree.add(
                Some(top),
                Some(idx),
                tag,
                &display,
                text.as_ref().map(ByteString::as_bytes),
                0,
            );
        }
    }

    /// `window_customize_draw_key` as draw ops.
    fn prepare_key(&mut self, s: &Server, item: &CustomizeItem) {
        let Some((id, key)) = get_key(s, item) else {
            return;
        };
        let Some(kt) = s.key_bindings.tables.get(id) else {
            return;
        };
        let Some(bd) = kt.get(key) else {
            return;
        };
        let note: &[u8] = bd
            .note
            .as_ref()
            .map_or(b"There is no note for this key.".as_slice(), |n| {
                n.as_bytes()
            });
        let period: &[u8] = if !note.is_empty() && note.last() != Some(&b'.') {
            b"."
        } else {
            b""
        };
        self.cache.push(text_op(concat(&[note, period])));
        self.cache.push(DrawOp::Skip { strict: false });
        self.cache.push(text_op(concat(&[
            b"This key is in the ",
            &kt.name,
            b" table.",
        ])));
        let repeat = bd
            .flags
            .contains(crate::cmd::key_bindings::KeyBindingFlags::REPEAT);
        self.cache.push(value_op(
            b"Repeat: ",
            ByteString::from(if repeat { "on" } else { "off" }),
        ));
        self.cache.push(DrawOp::Skip { strict: false });
        let cmd = bd.list.print(CommandListPrintFlags(0));
        self.cache.push(value_op(b"Command: ", cmd.clone()));
        if let Some(default) = kt.get_default(key) {
            let default_cmd = default.list.print(CommandListPrintFlags(0));
            if default_cmd != cmd {
                self.cache.push(value_op(b"The default is: ", default_cmd));
            }
        }
    }

    /// `window_customize_draw_environment` as draw ops.
    fn prepare_environment(&mut self, s: &Server, item: &CustomizeItem) {
        if check_item(s, &self.fs, self.wp, item).is_none() {
            return;
        }
        let Some(entry) = item
            .environment
            .and_then(|env| environment(s, env))
            .and_then(|env| env.find(&item.name))
        else {
            return;
        };
        let text: &[u8] = if item.scope == CustomizeScope::GlobalEnvironment {
            b"global"
        } else {
            b"session"
        };
        self.cache.push(text_op(concat(&[
            b"This is a ",
            text,
            b" environment variable.",
        ])));
        if entry.flags.contains(EnvironmentFlags::HIDDEN) {
            self.cache
                .push(text_op(ByteString::from("This variable is hidden.")));
        }
        self.cache.push(DrawOp::Skip { strict: false });
        match &entry.value {
            None => self
                .cache
                .push(text_op(ByteString::from("Variable is removed."))),
            Some(value) => self
                .cache
                .push(value_op(b"Variable value: ", value.clone())),
        }
        if item.scope != CustomizeScope::SessionEnvironment {
            return;
        }
        let Some(parent) = s.global_environment.find(&item.name) else {
            return;
        };
        match &parent.value {
            None => self
                .cache
                .push(text_op(ByteString::from("Global variable is removed."))),
            Some(value) => self.cache.push(value_op(b"Global value: ", value.clone())),
        }
    }

    /// `window_customize_draw_option` as draw ops.
    fn prepare_option(&mut self, s: &mut Server, item: &CustomizeItem) {
        let Some(fs) = check_item(s, &self.fs, self.wp, item) else {
            return;
        };
        let Some(oo) = item.oo else {
            return;
        };
        let name = item.name.clone();
        let Some((owner, o)) = s.options.get(oo, &name) else {
            return;
        };
        let oe = o.table_entry();
        let monitor = o.monitor();
        let is_hook = oe.is_some_and(|oe| oe.is_hook());
        let is_monitor = monitor.is_some();
        let is_user_hook = name.first() == Some(&b'@') && hooks::is_event(&s.hooks, &name);
        let is_any_hook = is_hook || is_monitor || is_user_hook;
        let (space, unit): (&[u8], &[u8]) = match oe.and_then(|oe| oe.unit) {
            Some(unit) => (b" ".as_slice(), unit),
            None => (b"".as_slice(), b"".as_slice()),
        };
        let value = value_string(o, item.array_key.as_ref());
        let fire = (
            o.fire_count(),
            o.fire_time().sec,
            monitor
                .and_then(|id| s.hooks.monitors.get(id).and_then(Option::as_ref))
                .map(|m| hooks::monitor_to_string(&name, m)),
        );
        let (fire_count, fire_time) = match monitor {
            Some(id) => (
                hooks::monitor_get_fire_count(&*s, id, &name),
                hooks::monitor_get_fire_time(&*s, id, &name),
            ),
            None => (fire.0, fire.1),
        };
        let monitor_text = fire.2;
        let mut ft = format::create_from_state(s, None, None, &fs);

        let hook_fire = |count: u32, time: i64| {
            if time != 0 {
                let pretty = format::pretty_time(Timestamp::new(time, 0), false);
                text_op(ByteString::from(format!(
                    "This hook has been fired {count} times, last {}.",
                    String::from_utf8_lossy(&pretty)
                )))
            } else {
                text_op(ByteString::from(format!(
                    "This hook has been fired {count} times."
                )))
            }
        };

        let description: &[u8] = match oe {
            Some(oe) if !oe.text.is_empty() => oe.text,
            _ => {
                if is_monitor {
                    b"This hook runs when a monitor changes."
                } else if is_user_hook {
                    b"This hook doesn't have a description."
                } else {
                    b"This option doesn't have a description."
                }
            }
        };
        self.cache.push(text_op(description.into()));
        self.cache.push(DrawOp::Skip { strict: false });

        if is_monitor {
            self.cache
                .push(text_op(ByteString::from("This is a monitor hook.")));
        } else {
            let text: &[u8] = match oe {
                None => b"user",
                Some(oe) if oe.scope.contains(OptionsScope::WINDOW | OptionsScope::PANE) => {
                    b"window and pane"
                }
                Some(oe) if oe.scope.contains(OptionsScope::WINDOW) => b"window",
                Some(oe) if oe.scope.contains(OptionsScope::SESSION) => b"session",
                Some(_) => b"server",
            };
            if is_user_hook {
                self.cache
                    .push(text_op(ByteString::from("This is a user hook.")));
            } else if is_hook {
                self.cache
                    .push(text_op(concat(&[b"This is a ", text, b" hook."])));
            } else {
                self.cache
                    .push(text_op(concat(&[b"This is a ", text, b" option."])));
            }
        }

        if let Some(monitor_text) = monitor_text {
            self.cache.push(value_op(b"Monitor: ", monitor_text));
        }
        let array_key = item.array_key.as_ref();
        if oe.is_some_and(|oe| oe.is_array()) {
            if is_hook {
                if array_key.is_none() {
                    self.cache
                        .push(text_op(ByteString::from("This is an array hook.")));
                    self.cache.push(hook_fire(fire_count, fire_time));
                    ft.release(s);
                    return;
                }
            } else if let Some(key) = array_key {
                self.cache.push(text_op(concat(&[
                    b"This is an array option, key ",
                    key,
                    b".",
                ])));
            } else {
                self.cache
                    .push(text_op(ByteString::from("This is an array option.")));
            }
            if array_key.is_none() {
                ft.release(s);
                return;
            }
        }
        self.cache.push(DrawOp::Skip { strict: false });

        let default_value = match (oe, array_key) {
            (Some(oe), None) => Some(options::default_to_string(oe)).filter(|d| *d != value),
            _ => None,
        };
        if is_any_hook {
            self.cache
                .push(value_op(b"Hook command: ", concat(&[&value, space, unit])));
            self.cache.push(hook_fire(fire_count, fire_time));
        } else {
            self.cache
                .push(value_op(b"Option value: ", concat(&[&value, space, unit])));
        }
        if oe.is_none_or(|oe| oe.kind == OptionsTableType::String) {
            let expanded = ft.expand(s, &value);
            if expanded != value {
                self.cache.push(value_op(b"This expands to: ", expanded));
            }
        }
        if let Some(oe) = oe.filter(|oe| oe.kind == OptionsTableType::Choice) {
            // strlcat into a 256 byte buffer, then drop the trailing ", ".
            let mut choices = Vec::new();
            for choice in oe.choices.unwrap_or(&[]) {
                choices.extend_from_slice(choice);
                choices.extend_from_slice(b", ");
            }
            choices.truncate(255);
            let len = choices.len().saturating_sub(2);
            choices.truncate(len);
            self.cache.push(value_op(
                b"Available values are: ",
                ByteString::from(choices),
            ));
        }
        if oe.is_some_and(|oe| oe.kind == OptionsTableType::Colour) {
            self.cache.push(DrawOp::Text {
                more: true,
                gc: DEFAULT_CELL,
                text: ByteString::from("This is a colour option: "),
            });
            let mut gc = DEFAULT_CELL;
            gc.fg = Colour(s.options.get_number(oo, &name) as i32);
            self.cache.push(DrawOp::Text {
                more: false,
                gc,
                text: ByteString::from("EXAMPLE"),
            });
        }
        if oe.is_some_and(|oe| oe.flags.contains(OptionsTableFlags::COLOUR)) {
            self.cache.push(DrawOp::Text {
                more: true,
                gc: DEFAULT_CELL,
                text: ByteString::from("This is a colour option: "),
            });
            let mut gc = DEFAULT_CELL;
            style_apply(s, &mut gc, oo, &name, Some(&mut ft));
            self.cache.push(DrawOp::Text {
                more: false,
                gc,
                text: ByteString::from("EXAMPLE"),
            });
        }
        if oe.is_some_and(|oe| oe.flags.contains(OptionsTableFlags::STYLE)) {
            self.cache.push(DrawOp::Text {
                more: true,
                gc: DEFAULT_CELL,
                text: ByteString::from("This is a style option: "),
            });
            let mut gc = DEFAULT_CELL;
            style_apply(s, &mut gc, oo, &name, Some(&mut ft));
            self.cache.push(DrawOp::Text {
                more: false,
                gc,
                text: ByteString::from("EXAMPLE"),
            });
        }
        if let Some(default_value) = default_value {
            self.cache.push(value_op(
                b"The default is: ",
                concat(&[&default_value, space, unit]),
            ));
        }

        self.cache.push(DrawOp::Skip { strict: true });
        let (wo, go) = if oe.is_some_and(|oe| oe.is_array()) {
            (None, None)
        } else {
            match item.scope {
                CustomizeScope::Pane => {
                    let wo = s.options.parent(oo);
                    (wo, wo.and_then(|wo| s.options.parent(wo)))
                }
                CustomizeScope::Window | CustomizeScope::Session => (None, s.options.parent(oo)),
                _ => (None, None),
            }
        };
        let parent_value = |s: &Server, oo: Option<OptionsId>| {
            oo.filter(|oo| owner != *oo)
                .and_then(|oo| s.options.get_only(oo, &name))
                .map(|parent| concat(&[&parent.to_string(None, false), space, unit]))
        };
        if let Some(value) = parent_value(s, wo) {
            let idx = fs
                .wl
                .and_then(|wl| s.winlinks.get(wl))
                .map(|wl| wl.index)
                .unwrap_or(0);
            let label = format!("Window value (from window {idx}): ");
            self.cache.push(value_op(label.as_bytes(), value));
        }
        if let Some(value) = parent_value(s, go) {
            self.cache.push(value_op(b"Global value: ", value));
        }
        ft.release(s);
    }
}

/// `window_customize_write_value`.
fn write_value(
    ctx: &mut ScreenWriteCtx<'_>,
    cx: u32,
    sx: u32,
    sy: u32,
    more: bool,
    label: &[u8],
    value: &[u8],
) -> bool {
    let cy = ctx.screen.cy;
    if sy == 0 {
        return false;
    }
    if !ctx.text(cx, sx, sy, true, &DEFAULT_CELL, label) {
        return false;
    }
    let used = ctx.screen.cy.saturating_sub(cy);
    if used >= sy {
        return false;
    }
    let mut gc = DEFAULT_CELL;
    gc.fg = light_grey();
    ctx.text(cx, sx, sy - used, more, &gc, value)
}

impl ModeTreeCallbacks for CustomizeBackend {
    type Item = CustomizeItem;

    fn build(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        _sort: &crate::format::sort::SortCriteria,
        _tag: &mut ModeTreeTag,
        filter: Option<&[u8]>,
    ) {
        self.items.clear();
        let Some(fs) = resolve_fs(s, &self.fs, self.wp) else {
            return;
        };
        let (Some(sid), Some(wid), Some(pid)) = (fs.s, fs.w, fs.wp) else {
            return;
        };
        let Some(so) = s.sessions.get(sid).map(|x| x.options) else {
            return;
        };
        let Some(wo) = s.windows.get(wid).map(|w| w.options) else {
            return;
        };
        let Some(po) = s.panes.get(pid).map(|p| p.options) else {
            return;
        };
        let (g, gs, gw) = (s.options.global, s.options.global_s, s.options.global_w);

        let mut ft = format::create_from_state(s, None, None, &fs);
        ft.add(b"is_option", flag(true));
        ft.add(b"is_key", flag(false));
        ft.add(b"is_environment", flag(false));

        use CustomizeOptionType::{Hooks, Options};
        use CustomizeScope as Sc;
        let server_trees = [(Sc::Server, Some(g)), (Sc::None, None), (Sc::None, None)];
        let session_trees = [
            (Sc::GlobalSession, Some(gs)),
            (Sc::Session, Some(so)),
            (Sc::None, None),
        ];
        let window_trees = [
            (Sc::GlobalWindow, Some(gw)),
            (Sc::Window, Some(wo)),
            (Sc::Pane, Some(po)),
        ];
        let session_scope = OptionsScope::SESSION.bits() as u8;
        let window_scope = OptionsScope::WINDOW.bits() as u8;
        let server_scope = OptionsScope::SERVER.bits() as u8;
        self.build_options(
            s,
            tree,
            b"Server Options",
            section_tag(SECTION_OPTIONS, server_scope),
            server_trees,
            &mut ft,
            filter,
            &fs,
            Options,
        );
        self.build_options(
            s,
            tree,
            b"Session Options",
            section_tag(SECTION_OPTIONS, session_scope),
            session_trees,
            &mut ft,
            filter,
            &fs,
            Options,
        );
        self.build_options(
            s,
            tree,
            b"Window & Pane Options",
            section_tag(SECTION_OPTIONS, window_scope),
            window_trees,
            &mut ft,
            filter,
            &fs,
            Options,
        );
        self.build_options(
            s,
            tree,
            b"Session Hooks",
            section_tag(SECTION_HOOKS, session_scope),
            session_trees,
            &mut ft,
            filter,
            &fs,
            Hooks,
        );
        self.build_options(
            s,
            tree,
            b"Window & Pane Hooks",
            section_tag(SECTION_HOOKS, window_scope),
            window_trees,
            &mut ft,
            filter,
            &fs,
            Hooks,
        );
        self.build_environment(
            s,
            tree,
            b"Global Environment",
            section_tag(SECTION_ENVIRONMENT, 1),
            Sc::GlobalEnvironment,
            EnvironmentRef::Global,
            &mut ft,
            filter,
            &fs,
        );
        self.build_environment(
            s,
            tree,
            b"Session Environment",
            section_tag(SECTION_ENVIRONMENT, 3),
            Sc::SessionEnvironment,
            EnvironmentRef::Session(sid),
            &mut ft,
            filter,
            &fs,
        );
        ft.release(s);

        let tables: Vec<KeyTableId> = s
            .key_bindings
            .tables()
            .filter(|id| {
                s.key_bindings
                    .tables
                    .get(*id)
                    .is_some_and(|t| !t.bindings.is_empty())
            })
            .collect();
        for id in tables {
            self.build_keys(s, tree, id, filter, &fs);
        }
    }

    fn prepare_draw(&mut self, s: &mut Server, item: Option<u32>, _sx: u32, _sy: u32) {
        self.cache.clear();
        let Some(item) = item.and_then(|n| self.items.get(n as usize)).cloned() else {
            return;
        };
        match item.kind {
            CustomizeItemType::Key => self.prepare_key(s, &item),
            CustomizeItemType::Environment => self.prepare_environment(s, &item),
            CustomizeItemType::Option => self.prepare_option(s, &item),
        }
    }

    fn draw(
        &self,
        _s: &Server,
        item: Option<&CustomizeItem>,
        ctx: &mut ScreenWriteCtx<'_>,
        sx: u32,
        sy: u32,
    ) {
        if item.is_none() {
            return;
        }
        let cx = ctx.screen.cx;
        let cy = ctx.screen.cy;
        for op in &self.cache {
            let used = ctx.screen.cy.saturating_sub(cy);
            match op {
                DrawOp::Text { more, gc, text } => {
                    if !ctx.text(cx, sx, sy.saturating_sub(used), *more, gc, text) {
                        return;
                    }
                }
                DrawOp::Value { label, value } => {
                    if !write_value(ctx, cx, sx, sy.saturating_sub(used), false, label, value) {
                        return;
                    }
                }
                DrawOp::Skip { strict } => {
                    let next = ctx.screen.cy + 1;
                    ctx.cursormove(cx as i32, next as i32, false);
                    let limit = (cy + sy).saturating_sub(1);
                    let now = ctx.screen.cy;
                    if (*strict && now > limit) || (!*strict && now >= limit) {
                        return;
                    }
                }
            }
        }
    }

    fn has_draw(&self) -> bool {
        true
    }

    fn search(
        &self,
        _s: &Server,
        _item: Option<&CustomizeItem>,
        _needle: &[u8],
        _icase: bool,
    ) -> Option<bool> {
        None
    }

    fn menu(&mut self, mode: ModeId, client: Option<ClientId>, key: KeyCode) -> ModeAction {
        Box::new(move |server: &mut Server| {
            let first = server
                .panes
                .get(mode.owner)
                .and_then(|p| p.modes.first())
                .map(|m| m.id);
            if first == Some(mode) {
                key_impl(server, mode, client, key, None);
            }
        })
    }

    fn height(&self, _screen_sy: u32) -> Option<u32> {
        Some(12)
    }

    fn key(&mut self, _s: &mut Server, _item: Option<u32>, _line: u32) -> Option<KeyCode> {
        None
    }

    fn swap(
        &self,
        _cur: &CustomizeItem,
        _other: &CustomizeItem,
        _sort: &crate::format::sort::SortCriteria,
    ) -> ModeAction {
        Box::new(|_| {})
    }

    fn can_swap(
        &self,
        _s: &Server,
        _cur: &CustomizeItem,
        _other: &CustomizeItem,
        _sort: &crate::format::sort::SortCriteria,
    ) -> bool {
        false
    }

    fn sort(&self, _sort: &mut crate::format::sort::SortCriteria) -> bool {
        false
    }

    fn help(&self) -> Option<(u32, &'static str, &'static [&'static str])> {
        Some((52, "item", HELP_LINES))
    }

    fn items(&self) -> &[CustomizeItem] {
        &self.items
    }
}

fn mode_mut(server: &mut Server, id: ModeId) -> Option<&mut PaneMode> {
    server
        .panes
        .get_mut(id.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == id)
}

fn take_state(server: &mut Server, id: ModeId) -> Option<Box<CustomizeState>> {
    let mode = mode_mut(server, id)?;
    let data = mode.data.take()?;
    match data.downcast::<CustomizeState>() {
        Ok(state) => Some(state),
        Err(other) => {
            mode.data = Some(other);
            None
        }
    }
}

fn restore_state(server: &mut Server, id: ModeId, state: Box<CustomizeState>) {
    match mode_mut(server, id) {
        Some(mode) => mode.data = Some(state),
        None => state.free(server),
    }
}

/// Run `f` with the state and the mode screen taken out of the pane.
fn with_state(
    server: &mut Server,
    id: ModeId,
    f: impl FnOnce(&mut Server, &mut CustomizeState, &mut Screen),
) {
    let Some(mut state) = take_state(server, id) else {
        return;
    };
    let Some(mut screen) = mode_mut(server, id).and_then(|m| m.screen.take()) else {
        restore_state(server, id, state);
        return;
    };
    f(server, &mut state, &mut screen);
    match mode_mut(server, id) {
        Some(mode) => {
            mode.screen = Some(screen);
            mode.data = Some(state);
        }
        None => {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
            state.free(server);
        }
    }
}

/// The first mode of `wp` when it is an options mode.
fn first_mode(server: &Server, wp: PaneId) -> Option<ModeId> {
    let first = server.panes.get(wp)?.modes.first()?;
    (first.name == NAME).then_some(first.id)
}

fn redraw(server: &mut Server, state: &mut CustomizeState, screen: &mut Screen) {
    state.draw(server, screen);
    draw_waiting(server, state.backend.editor, screen);
    if let Some(p) = server.panes.get_mut(state.backend.wp) {
        p.flags.insert(PaneFlags::REDRAW);
    }
}

/// `mode_tree_build; mode_tree_draw; wp->flags |= PANE_REDRAW`.
fn rebuild(server: &mut Server, state: &mut CustomizeState, screen: &mut Screen) {
    state.build(server);
    state.draw(server, screen);
    if let Some(p) = server.panes.get_mut(state.backend.wp) {
        p.flags.insert(PaneFlags::REDRAW);
    }
}

fn status_text(server: &mut Server, c: Option<ClientId>, text: &[u8]) {
    status_message_set(server, c, -1, true, false, false, text);
}

/// A prompt the mode wants opened once its state is back in the pane
/// (`mode_tree_set_prompt`).
struct PromptRequest {
    msg: ByteString,
    input: ByteString,
    flags: PromptFlags,
    kind: PromptKind,
}

enum PromptKind {
    SetOption(CustomizeItem),
    SetEnvironment(CustomizeItem),
    AddOption(CustomizeItem),
    AddEnvironment(CustomizeItem),
    SetArrayKey(CustomizeItem),
    SetCommand(CustomizeItem),
    SetNote(CustomizeItem),
    AddKey(CustomizeItem),
    ChangeCurrent,
    ChangeTagged,
}

/// Work left over from a step run with the mode state taken out: option
/// change pushes and user hook events run with the state restored, then the
/// rebuild, the status message and the prompt.
#[derive(Default)]
struct Followup {
    events: Vec<ByteString>,
    pushes: Vec<ByteString>,
    rebuild: bool,
    status: Option<ByteString>,
    prompt: Option<PromptRequest>,
}

fn apply_followup(server: &mut Server, mode: ModeId, c: Option<ClientId>, followup: Followup) {
    for name in followup.events {
        hooks::add_event(server, &name);
    }
    for name in followup.pushes {
        push(server, &name);
    }
    if let Some(text) = followup.status {
        status_text(server, c, &text);
    }
    if followup.rebuild {
        with_state(server, mode, |server, state, screen| {
            rebuild(server, state, screen)
        });
    }
    if let Some(request) = followup.prompt {
        open_prompt(server, mode, c, request);
    }
}

/// `mode_tree_set_prompt`: the tree owns the prompt and draws it in the mode.
fn open_prompt(server: &mut Server, mode: ModeId, c: Option<ClientId>, request: PromptRequest) {
    let Some(c) = c else {
        return;
    };
    let mut kind = Some(request.kind);
    with_state(server, mode, |server, state, screen| {
        state.set_prompt(
            server,
            mode,
            Some(c),
            PromptCreateData {
                prompt: request.msg,
                input: Some(request.input),
                flags: request.flags,
                ty: PromptType::Command,
                ..Default::default()
            },
            move |text, _| {
                let text = rmux_util::bytes::cstr(text?).to_vec();
                let kind = kind.take()?;
                Some(Box::new(move |server| {
                    if first_mode(server, mode.owner) == Some(mode) {
                        prompt_fire(server, mode, Some(c), &kind, &text);
                    }
                }))
            },
        );
        redraw(server, state, screen);
    });
}

/// The prompt input callbacks (`window_customize_*_callback`).
fn prompt_fire(
    server: &mut Server,
    mode: ModeId,
    c: Option<ClientId>,
    kind: &PromptKind,
    text: &[u8],
) {
    if text.is_empty() && !matches!(kind, PromptKind::SetEnvironment(_)) {
        return;
    }
    let mut followup = Followup::default();
    with_state(server, mode, |server, state, _screen| {
        let fs = state.backend.fs;
        let wp = state.backend.wp;
        match kind {
            PromptKind::SetOption(item) => {
                if check_item(server, &fs, wp, item).is_none() {
                    return;
                }
                match set_option_value(server, item, text, &mut followup) {
                    Ok(()) => followup.rebuild = true,
                    Err(Some(cause)) => followup.status = Some(upper_first(cause)),
                    Err(None) => {}
                }
            }
            PromptKind::SetEnvironment(item) => {
                if check_item(server, &fs, wp, item).is_none() {
                    return;
                }
                set_environment_value(server, item, text);
                followup.rebuild = true;
            }
            PromptKind::AddOption(item) => {
                add_option_fire(server, item, text, &fs, wp, &mut followup)
            }
            PromptKind::AddEnvironment(item) => {
                add_environment_fire(server, item, text, &fs, wp, &mut followup)
            }
            PromptKind::SetArrayKey(item) => {
                set_array_key_fire(server, item, text, &fs, wp, &mut followup)
            }
            PromptKind::SetCommand(item) => match set_command_value(server, item, text) {
                Ok(()) => followup.rebuild = true,
                Err(Some(cause)) => followup.status = Some(upper_first(cause)),
                Err(None) => {}
            },
            PromptKind::SetNote(item) => {
                let Some((id, key)) = get_key(server, item) else {
                    return;
                };
                if let Some(bd) = binding_mut(server, id, key) {
                    bd.note = Some(text.into());
                    followup.rebuild = true;
                }
            }
            PromptKind::AddKey(item) => add_key_fire(server, item, text, &mut followup),
            PromptKind::ChangeCurrent => {
                if !confirmed(text) {
                    return;
                }
                let Some(idx) = state.tree.get_current().and_then(|i| i.item) else {
                    return;
                };
                change_item(server, state, idx, &mut followup);
                followup.rebuild = true;
            }
            PromptKind::ChangeTagged => {
                if !confirmed(text) {
                    return;
                }
                for idx in state.tree.tagged_items(false) {
                    change_item(server, state, idx, &mut followup);
                }
                followup.rebuild = true;
            }
        }
    });
    apply_followup(server, mode, c, followup);
}

/// `tolower(s[0]) == 'y' && s[1] == '\0'`.
fn confirmed(text: &[u8]) -> bool {
    text.len() == 1 && crate::modes::tree::tolower(text[0]) == b'y'
}

fn upper_first(mut cause: ByteString) -> ByteString {
    if let Some(first) = cause.first_mut() {
        first.make_ascii_uppercase();
    }
    cause
}

fn binding_mut(
    server: &mut Server,
    id: KeyTableId,
    key: KeyCode,
) -> Option<&mut crate::cmd::key_bindings::KeyBinding> {
    server
        .key_bindings
        .tables
        .get_mut(id)?
        .bindings
        .get_mut(&key)
}

/// `window_customize_set_option_value`; `Err(None)` is the silent `-1`.
/// The hook event and the push are queued on `followup`.
fn set_option_value(
    server: &mut Server,
    item: &CustomizeItem,
    value: &[u8],
    followup: &mut Followup,
) -> Result<(), Option<ByteString>> {
    let oo = item.oo.ok_or(None)?;
    let name = item.name.clone();
    with_store(server, |store, ctx, _| -> Result<(), Option<ByteString>> {
        let (owner, o) = store.get(oo, &name).ok_or(None)?;
        let oe = o.table_entry();
        if oe.is_some_and(|oe| oe.is_array()) {
            let key = match &item.array_key {
                Some(key) => OptionsArrayKey::parse(key).map_err(|e| Some(e.0))?,
                None => OptionsArrayKey::Index(first_unused_index(o)),
            };
            let o = store.get_mut_only(owner, &name).ok_or(None)?;
            o.array_set(&key, Some(value), false, &mut *ctx.parser)
                .map_err(|e| Some(e.0))
        } else {
            store
                .from_string(oo, oe, &name, Some(value), false, ctx)
                .map_err(|e| Some(e.0))
        }
    })?;
    if item.option_type == CustomizeOptionType::Hooks && name.first() == Some(&b'@') {
        followup.events.push(name.clone());
    }
    followup.pushes.push(name);
    Ok(())
}

/// `window_customize_option_editable`.
fn option_editable(server: &Server, fs: &CmdFindState, wp: PaneId, item: &CustomizeItem) -> bool {
    if item.kind != CustomizeItemType::Option || check_item(server, fs, wp, item).is_none() {
        return false;
    }
    let Some((_, o)) = item.oo.and_then(|oo| server.options.get(oo, &item.name)) else {
        return false;
    };
    match o.table_entry() {
        None => true,
        Some(oe) => !matches!(oe.kind, OptionsTableType::Flag | OptionsTableType::Choice),
    }
}

/// `window_customize_set_command_value`.
fn set_command_value(
    server: &mut Server,
    item: &CustomizeItem,
    s: &[u8],
) -> Result<(), Option<ByteString>> {
    let (id, key) = get_key(server, item).ok_or(None)?;
    let list = CommandParser::parse_from_string(server, s)
        .map_err(|e| Some(ByteString::from(e.message())))?;
    let bd = binding_mut(server, id, key).ok_or(None)?;
    bd.list = list;
    Ok(())
}

/// `window_customize_set_note_value`.
fn set_note_value(server: &mut Server, item: &CustomizeItem, s: &[u8]) -> Result<(), ()> {
    let (id, key) = get_key(server, item).ok_or(())?;
    let bd = binding_mut(server, id, key).ok_or(())?;
    bd.note = (!s.is_empty()).then(|| s.into());
    Ok(())
}

/// `window_customize_set_environment_value`.
fn set_environment_value(server: &mut Server, item: &CustomizeItem, s: &[u8]) {
    let Some(env) = item
        .environment
        .and_then(|env| environment_mut(server, env))
    else {
        return;
    };
    let flags = env
        .find(&item.name)
        .map_or(item.environment_flags, |e| e.flags);
    env.set(&item.name, flags, s);
}

/// `window_customize_add_option_callback`.
fn add_option_fire(
    server: &mut Server,
    item: &CustomizeItem,
    s: &[u8],
    fs: &CmdFindState,
    wp: PaneId,
    followup: &mut Followup,
) {
    if check_item(server, fs, wp, item).is_none() {
        return;
    }
    let namelen = s
        .iter()
        .position(|b| *b == b' ' || *b == b'\t')
        .unwrap_or(s.len());
    if namelen == 0 || namelen == s.len() {
        followup.status = Some(ByteString::from("User option must be @name value"));
        return;
    }
    let value = &s[namelen
        + s[namelen..]
            .iter()
            .take_while(|b| **b == b' ' || **b == b'\t')
            .count()..];
    if value.is_empty() {
        followup.status = Some(ByteString::from("User option must be @name value"));
        return;
    }
    let name = match match_name(&s[..namelen]) {
        Ok(Some((OptionName::User(name), None))) => name,
        _ => {
            let what: &[u8] = if item.option_type == CustomizeOptionType::Hooks {
                b"hook"
            } else {
                b"option"
            };
            followup.status = Some(concat(&[b"User ", what, b" name must start with @"]));
            return;
        }
    };
    let Some(oo) = item.oo else {
        return;
    };
    with_store(server, |store, ctx, _| {
        store.set_string(oo, &name, false, value, &mut *ctx.parser);
    });
    if item.option_type == CustomizeOptionType::Hooks {
        followup.events.push(name.clone());
    }
    followup.pushes.push(name);
    followup.rebuild = true;
}

/// `window_customize_add_environment_callback`.
fn add_environment_fire(
    server: &mut Server,
    item: &CustomizeItem,
    s: &[u8],
    fs: &CmdFindState,
    wp: PaneId,
    followup: &mut Followup,
) {
    if check_item(server, fs, wp, item).is_none() {
        return;
    }
    let Some(env) = item
        .environment
        .and_then(|env| environment_mut(server, env))
    else {
        return;
    };
    if s[0] == b'-' {
        if s.len() == 1 || s[1..].contains(&b'=') {
            followup.status = Some(concat(&[b"Bad environment variable: ", s]));
            return;
        }
        env.clear(&s[1..]);
    } else {
        let Some(eq) = s.iter().position(|b| *b == b'=').filter(|eq| *eq != 0) else {
            followup.status = Some(ByteString::from("Environment variable must be NAME=value"));
            return;
        };
        env.set(&s[..eq], EnvironmentFlags(0), &s[eq + 1..]);
    }
    followup.rebuild = true;
}

/// `window_customize_set_array_key_callback`: write the new key before the
/// old one is removed.
fn set_array_key_fire(
    server: &mut Server,
    item: &CustomizeItem,
    s: &[u8],
    fs: &CmdFindState,
    wp: PaneId,
    followup: &mut Followup,
) {
    let Some(array_key) = item.array_key.clone() else {
        return;
    };
    if check_item(server, fs, wp, item).is_none() {
        return;
    }
    let Some(oo) = item.oo else {
        return;
    };
    let name = item.name.clone();
    let result = with_store(server, |store, ctx, _| -> Result<bool, ByteString> {
        let Some((owner, o)) = store.get(oo, &name) else {
            return Ok(false);
        };
        let new_key = OptionsArrayKey::parse(s);
        if new_key.as_ref().is_ok_and(|key| o.array_get(key).is_some()) {
            return Ok(false);
        }
        let Ok(old_key) = OptionsArrayKey::parse(&array_key) else {
            return Ok(false);
        };
        let value = o.to_string(Some(&old_key), false);
        let new_key = new_key.map_err(|e| e.0)?;
        let Some(o) = store.get_mut_only(owner, &name) else {
            return Ok(false);
        };
        o.array_set(&new_key, Some(&value), false, &mut *ctx.parser)
            .map_err(|e| e.0)?;
        let _ = o.array_set(&old_key, None, false, &mut *ctx.parser);
        Ok(true)
    });
    match result {
        Ok(true) => {
            followup.pushes.push(name);
            followup.rebuild = true;
        }
        Ok(false) => {}
        Err(cause) => followup.status = Some(upper_first(cause)),
    }
}

/// `window_customize_add_key_callback`.
fn add_key_fire(server: &mut Server, item: &CustomizeItem, s: &[u8], followup: &mut Followup) {
    let keylen = s
        .iter()
        .position(|b| *b == b' ' || *b == b'\t')
        .unwrap_or(s.len());
    if keylen == 0 || keylen == s.len() {
        followup.status = Some(ByteString::from("Key binding must be key command"));
        return;
    }
    let command = &s[keylen
        + s[keylen..]
            .iter()
            .take_while(|b| **b == b' ' || **b == b'\t')
            .count()..];
    if command.is_empty() {
        followup.status = Some(ByteString::from("Key binding must be key command"));
        return;
    }
    let keystr = &s[..keylen];
    let key = parse_key_name(keystr);
    if key.0 == SpecialKey::NONE || key.0 == SpecialKey::UNKNOWN {
        followup.status = Some(concat(&[b"Unknown key: ", keystr]));
        return;
    }
    let list = match CommandParser::parse_from_string(server, command) {
        Ok(list) => list,
        Err(e) => {
            followup.status = Some(upper_first(ByteString::from(e.message())));
            return;
        }
    };
    let Some(table) = &item.table else {
        return;
    };
    let _ = server.key_bindings.add(table, key, None, false, Some(list));
    followup.rebuild = true;
}

fn is_current(state: &CustomizeState, idx: u32) -> bool {
    state.tree.get_current().and_then(|i| i.item) == Some(idx)
}

/// `window_customize_unset_environment`.
fn unset_environment(
    server: &mut Server,
    state: &mut CustomizeState,
    idx: u32,
    item: &CustomizeItem,
) {
    let backend = &state.backend;
    if check_item(server, &backend.fs, backend.wp, item).is_none() {
        return;
    }
    let Some(env) = item.environment else {
        return;
    };
    if environment(server, env)
        .and_then(|e| e.find(&item.name))
        .is_none()
    {
        return;
    }
    if is_current(state, idx) {
        state.tree.up(false);
    }
    if let Some(env) = environment_mut(server, env) {
        env.unset(&item.name);
    }
}

/// `window_customize_unset_option`.
fn unset_option(server: &mut Server, state: &mut CustomizeState, idx: u32, item: &CustomizeItem) {
    let backend = &state.backend;
    if check_item(server, &backend.fs, backend.wp, item).is_none() {
        return;
    }
    let Some(oo) = item.oo else {
        return;
    };
    let Some((owner, _)) = server.options.get(oo, &item.name) else {
        return;
    };
    if item.array_key.is_some() && is_current(state, idx) {
        state.tree.up(false);
    }
    let key = item
        .array_key
        .as_ref()
        .and_then(|k| OptionsArrayKey::parse(k).ok());
    let name = item.name.clone();
    with_store(server, |store, ctx, freed| {
        let _ = store.remove_or_default(owner, &name, key.as_ref(), &mut *ctx.parser, freed);
    });
}

/// `window_customize_reset_option`: remove from the owner and every parent.
fn reset_option(server: &mut Server, state: &CustomizeState, item: &CustomizeItem) {
    let backend = &state.backend;
    if check_item(server, &backend.fs, backend.wp, item).is_none() {
        return;
    }
    if item.array_key.is_some() {
        return;
    }
    let mut oo = item.oo;
    let name = item.name.clone();
    with_store(server, |store, ctx, freed| {
        while let Some(id) = oo {
            if store.get_only(id, &name).is_some() {
                let _ = store.remove_or_default(id, &name, None, &mut *ctx.parser, freed);
            }
            oo = store.parent(id);
        }
    });
}

/// `window_customize_unset_key`.
fn unset_key(server: &mut Server, state: &mut CustomizeState, idx: u32, item: &CustomizeItem) {
    let Some((id, key)) = get_key(server, item) else {
        return;
    };
    let Some(table) = table_name(server, id) else {
        return;
    };
    if is_current(state, idx) {
        state.tree.up(false);
    }
    let _ = server.key_bindings.remove(&table, key);
}

/// `window_customize_reset_key`: nothing when the binding already has the
/// default command list.
fn reset_key(server: &mut Server, state: &mut CustomizeState, idx: u32, item: &CustomizeItem) {
    let Some((id, key)) = get_key(server, item) else {
        return;
    };
    let Some(kt) = server.key_bindings.tables.get(id) else {
        return;
    };
    let Some(bd) = kt.get(key) else {
        return;
    };
    let default = kt.get_default(key);
    if default.is_some_and(|dd| Rc::ptr_eq(&bd.list, &dd.list)) {
        return;
    }
    let table = kt.name.clone();
    if default.is_none() && is_current(state, idx) {
        state.tree.up(false);
    }
    let _ = server.key_bindings.reset(&table, key);
}

/// `window_customize_change_each` and the body of
/// `window_customize_change_current_callback`.
fn change_item(server: &mut Server, state: &mut CustomizeState, idx: u32, followup: &mut Followup) {
    let Some(item) = state.backend.items.get(idx as usize).cloned() else {
        return;
    };
    match (state.backend.change, item.kind) {
        (CustomizeChange::Unset, CustomizeItemType::Key) => unset_key(server, state, idx, &item),
        (CustomizeChange::Unset, CustomizeItemType::Environment) => {
            unset_environment(server, state, idx, &item)
        }
        (CustomizeChange::Unset, CustomizeItemType::Option) => {
            unset_option(server, state, idx, &item)
        }
        (CustomizeChange::Reset, CustomizeItemType::Key) => reset_key(server, state, idx, &item),
        (CustomizeChange::Reset, CustomizeItemType::Option) => reset_option(server, state, &item),
        (CustomizeChange::Reset, CustomizeItemType::Environment) => {}
    }
    if item.kind == CustomizeItemType::Option {
        followup.pushes.push(item.name);
    }
}

/// The scope a non-array set selects (`window-customize.c:2148-2192`).
fn select_scope(scope: CustomizeScope, global: bool, pane: bool) -> CustomizeScope {
    use CustomizeScope as Sc;
    if global {
        match scope {
            Sc::Session => Sc::GlobalSession,
            Sc::Window | Sc::Pane => Sc::GlobalWindow,
            other => other,
        }
    } else {
        match scope {
            Sc::Window | Sc::Pane | Sc::GlobalWindow => {
                if pane {
                    Sc::Pane
                } else {
                    Sc::Window
                }
            }
            Sc::GlobalSession => Sc::Session,
            other => other,
        }
    }
}

/// `window_customize_set_option`: flags toggle and choices cycle at once;
/// other types return a prompt.
fn set_option(
    server: &mut Server,
    state: &CustomizeState,
    idx: u32,
    global: bool,
    pane: bool,
) -> Option<PromptRequest> {
    let item = state.backend.items.get(idx as usize)?.clone();
    let fs = check_item(server, &state.backend.fs, state.backend.wp, &item)?;
    let item_oo = item.oo?;
    let name = item.name.clone();
    let (_, o) = server.options.get(item_oo, &name)?;
    let oe = o.table_entry();
    let value = value_string(o, item.array_key.as_ref());
    let pane = pane && oe.is_none_or(|oe| oe.scope.contains(OptionsScope::PANE));
    let is_array = oe.is_some_and(|oe| oe.is_array());
    let (scope, oo) = if is_array {
        (item.scope, item_oo)
    } else {
        let scope = select_scope(item.scope, global, pane);
        let oo = if scope == item.scope {
            item_oo
        } else {
            get_tree(server, scope, &fs)?
        };
        (scope, oo)
    };

    match oe.map(|oe| oe.kind) {
        Some(OptionsTableType::Flag) => {
            let flag = server.options.get_number(oo, &name);
            server
                .options
                .set_number_value(oo, &name, i64::from(flag == 0));
            None
        }
        Some(OptionsTableType::Choice) => {
            let choices = oe.and_then(|oe| oe.choices).unwrap_or(&[]);
            let choice = server.options.get_number(oo, &name).max(0) as usize;
            let next = if choice + 1 >= choices.len() {
                0
            } else {
                choice + 1
            };
            server.options.set_number_value(oo, &name, next as i64);
            None
        }
        _ => {
            let text = scope_text(server, scope, &fs);
            let space: &[u8] = if !text.is_empty() {
                b", for "
            } else if scope != CustomizeScope::Server {
                b", global"
            } else {
                b""
            };
            let msg = if is_array {
                match &item.array_key {
                    None => concat(&[b"(", &name, b"[+]", space, &text, b") "]),
                    Some(key) => concat(&[b"(", &name, b"[", key, b"]", space, &text, b") "]),
                }
            } else {
                concat(&[b"(", &name, space, &text, b") "])
            };
            let mut new_item = CustomizeItem::new(CustomizeItemType::Option);
            new_item.option_type = item.option_type;
            new_item.scope = scope;
            new_item.oo = Some(oo);
            new_item.name = name;
            new_item.array_key = item.array_key.clone();
            Some(PromptRequest {
                msg,
                input: value,
                flags: PromptFlags::NOFORMAT,
                kind: PromptKind::SetOption(new_item),
            })
        }
    }
}

/// `window_customize_set_environment`.
fn set_environment(
    server: &mut Server,
    state: &CustomizeState,
    idx: u32,
    global: bool,
) -> Option<PromptRequest> {
    let item = state.backend.items.get(idx as usize)?.clone();
    let fs = check_item(server, &state.backend.fs, state.backend.wp, &item)?;
    let entry = environment(server, item.environment?)?.find(&item.name)?;
    let (value, flags) = (entry.value.clone().unwrap_or_default(), entry.flags);
    let (scope, env) = if global {
        (CustomizeScope::GlobalEnvironment, EnvironmentRef::Global)
    } else {
        (item.scope, item.environment?)
    };
    let text = scope_text(server, scope, &fs);
    let space: &[u8] = if !text.is_empty() {
        b", for "
    } else if scope == CustomizeScope::GlobalEnvironment {
        b", global"
    } else {
        b""
    };
    let msg = concat(&[b"(", &item.name, space, &text, b") "]);
    let mut new_item = CustomizeItem::new(CustomizeItemType::Environment);
    new_item.scope = scope;
    new_item.environment = Some(env);
    new_item.environment_flags = flags;
    new_item.name = item.name.clone();
    Some(PromptRequest {
        msg,
        input: value,
        flags: PromptFlags::NOFORMAT,
        kind: PromptKind::SetEnvironment(new_item),
    })
}

/// `window_customize_add_option`.
fn add_option(scope: CustomizeScope, oo: OptionsId, kind: CustomizeOptionType) -> PromptRequest {
    let what: &[u8] = if kind == CustomizeOptionType::Hooks {
        b"hook"
    } else {
        b"option"
    };
    let mut new_item = CustomizeItem::new(CustomizeItemType::Option);
    new_item.option_type = kind;
    new_item.scope = scope;
    new_item.oo = Some(oo);
    PromptRequest {
        msg: concat(&[b"New user ", what, b": "]),
        input: ByteString::from("@"),
        flags: PromptFlags::NOFORMAT,
        kind: PromptKind::AddOption(new_item),
    }
}

/// `window_customize_add_environment`.
fn add_environment(scope: CustomizeScope, env: EnvironmentRef) -> PromptRequest {
    let mut new_item = CustomizeItem::new(CustomizeItemType::Environment);
    new_item.scope = scope;
    new_item.environment = Some(env);
    PromptRequest {
        msg: ByteString::from("New environment: "),
        input: ByteString::new(),
        flags: PromptFlags::NOFORMAT,
        kind: PromptKind::AddEnvironment(new_item),
    }
}

/// `window_customize_add_key`.
fn add_key(table: &[u8]) -> PromptRequest {
    let mut new_item = CustomizeItem::new(CustomizeItemType::Key);
    new_item.scope = CustomizeScope::Key;
    new_item.table = Some(table.into());
    PromptRequest {
        msg: concat(&[b"New key in ", table, b": "]),
        input: ByteString::new(),
        flags: PromptFlags::NOFORMAT,
        kind: PromptKind::AddKey(new_item),
    }
}

/// `window_customize_set_array_key`.
fn set_array_key(server: &Server, state: &CustomizeState, idx: u32) -> Option<PromptRequest> {
    let item = state.backend.items.get(idx as usize)?;
    let array_key = item.array_key.clone()?;
    check_item(server, &state.backend.fs, state.backend.wp, item)?;
    let mut new_item = CustomizeItem::new(CustomizeItemType::Option);
    new_item.option_type = item.option_type;
    new_item.scope = item.scope;
    new_item.oo = item.oo;
    new_item.name = item.name.clone();
    new_item.array_key = Some(array_key.clone());
    Some(PromptRequest {
        msg: concat(&[b"(", &item.name, b"[", &array_key, b"]) "]),
        input: array_key,
        flags: PromptFlags::NOFORMAT,
        kind: PromptKind::SetArrayKey(new_item),
    })
}

/// `window_customize_set_key`: `Repeat` toggles at once; `Command` and
/// `Note` prompt.
fn set_key(server: &mut Server, state: &CustomizeState, idx: u32) -> Option<PromptRequest> {
    let item = state.backend.items.get(idx as usize)?.clone();
    let (id, key) = get_key(server, &item)?;
    let current = state.tree.get_current_name()?.to_vec();
    let mut new_item = CustomizeItem::new(CustomizeItemType::Key);
    new_item.scope = item.scope;
    new_item.table = item.table.clone();
    new_item.key = key;
    let msg = concat(&[b"(", &key_name(key, false), b") "]);
    match current.as_slice() {
        b"Repeat" => {
            if let Some(bd) = binding_mut(server, id, key) {
                bd.flags.0 ^= crate::cmd::key_bindings::KeyBindingFlags::REPEAT.0;
            }
            None
        }
        b"Command" => {
            let bd = server.key_bindings.get(id, key)?;
            let value = bd.list.print(CommandListPrintFlags(0));
            Some(PromptRequest {
                msg,
                input: value,
                flags: PromptFlags::NOFORMAT,
                kind: PromptKind::SetCommand(new_item),
            })
        }
        b"Note" => {
            let bd = server.key_bindings.get(id, key)?;
            let value = bd.note.clone().unwrap_or_default();
            Some(PromptRequest {
                msg,
                input: value,
                flags: PromptFlags::NOFORMAT,
                kind: PromptKind::SetNote(new_item),
            })
        }
        _ => None,
    }
}

/// `window_customize_add_current`: a section title adds a new entry.
fn add_current(server: &Server, state: &CustomizeState) -> Option<PromptRequest> {
    let name = state.tree.get_current_name()?.to_vec();
    let fs = resolve_fs(server, &state.backend.fs, state.backend.wp)?;
    let session_options = || fs.s.and_then(|s| server.sessions.get(s)).map(|s| s.options);
    let pane_options = || fs.wp.and_then(|p| server.panes.get(p)).map(|p| p.options);
    use CustomizeOptionType::{Hooks, Options};
    use CustomizeScope as Sc;
    match name.as_slice() {
        b"Server Options" => Some(add_option(Sc::Server, server.options.global, Options)),
        b"Session Options" => Some(add_option(Sc::Session, session_options()?, Options)),
        b"Window & Pane Options" => Some(add_option(Sc::Pane, pane_options()?, Options)),
        b"Session Hooks" => Some(add_option(Sc::Session, session_options()?, Hooks)),
        b"Window & Pane Hooks" => Some(add_option(Sc::Pane, pane_options()?, Hooks)),
        b"Global Environment" => Some(add_environment(
            Sc::GlobalEnvironment,
            EnvironmentRef::Global,
        )),
        b"Session Environment" => Some(add_environment(
            Sc::SessionEnvironment,
            EnvironmentRef::Session(fs.s?),
        )),
        other => other.strip_prefix(b"Key Table - ").map(add_key),
    }
}

/// `window_customize_edit_close_cb`.
fn edit_close(
    server: &mut Server,
    wp: PaneId,
    edit_type: CustomizeEditType,
    item: CustomizeItem,
    editor: EditorId,
    buf: Option<Vec<u8>>,
) {
    let Some(mode) = first_mode(server, wp) else {
        return;
    };
    let mut fs = None;
    with_state(server, mode, |_, state, _| {
        if state.backend.editor == Some(editor) {
            state.backend.editor = None;
        }
        fs = Some((state.backend.fs, state.backend.wp));
    });
    let Some((fs, wp)) = fs else {
        return;
    };
    let Some(mut buf) = buf.filter(|b| !b.is_empty()) else {
        return;
    };
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    let mut followup = Followup::default();
    let ok = match edit_type {
        CustomizeEditType::Option => {
            !option_editable(server, &fs, wp, &item)
                || set_option_value(server, &item, &buf, &mut followup).is_ok()
        }
        CustomizeEditType::KeyCommand => set_command_value(server, &item, &buf).is_ok(),
        CustomizeEditType::KeyNote => set_note_value(server, &item, &buf).is_ok(),
        CustomizeEditType::Environment => {
            if check_item(server, &fs, wp, &item).is_none() {
                false
            } else {
                set_environment_value(server, &item, &buf);
                true
            }
        }
    };
    followup.rebuild = ok;
    apply_followup(server, mode, None, followup);
}

/// `window_customize_start_edit`.
fn start_edit(server: &mut Server, mode: ModeId, idx: u32, c: Option<ClientId>) {
    let Some(state) = server
        .panes
        .get(mode.owner)
        .and_then(|pane| pane.modes.iter().find(|entry| entry.id == mode))
        .and_then(|entry| entry.data.as_ref())
        .and_then(|data| data.downcast_ref::<CustomizeState>())
    else {
        return;
    };
    if state.backend.editor.is_some() {
        return;
    }
    let Some(item) = state.backend.items.get(idx as usize).cloned() else {
        return;
    };
    let backend = &state.backend;
    let (value, edit_type) = match item.kind {
        CustomizeItemType::Option => {
            if !option_editable(server, &backend.fs, backend.wp, &item) {
                return;
            }
            let Some((_, o)) = item.oo.and_then(|oo| server.options.get(oo, &item.name)) else {
                return;
            };
            (
                value_string(o, item.array_key.as_ref()),
                CustomizeEditType::Option,
            )
        }
        CustomizeItemType::Key => {
            let Some(name) = state.tree.get_current_name() else {
                return;
            };
            let Some((id, key)) = get_key(server, &item) else {
                return;
            };
            let Some(bd) = server.key_bindings.get(id, key) else {
                return;
            };
            match name {
                b"Command" => (
                    bd.list.print(CommandListPrintFlags(0)),
                    CustomizeEditType::KeyCommand,
                ),
                b"Note" => (
                    bd.note.clone().unwrap_or_default(),
                    CustomizeEditType::KeyNote,
                ),
                _ => return,
            }
        }
        CustomizeItemType::Environment => {
            if check_item(server, &backend.fs, backend.wp, &item).is_none() {
                return;
            }
            let Some(value) = item
                .environment
                .and_then(|env| environment(server, env))
                .and_then(|env| env.find(&item.name))
                .and_then(|e| e.value.clone())
            else {
                return;
            };
            (value, CustomizeEditType::Environment)
        }
    };

    let Some(client) = c.and_then(|c| server.clients.get(c)) else {
        return;
    };
    let Some(session) = client.session else {
        return;
    };
    let mut context = SpawnContext::new(session);
    context.client = c;
    context.client_environment = Some(client.environ.clone());
    context.client_cwd = client.cwd.clone();
    context.client_attached = true;
    let wp = state.backend.wp;
    let buf: &[u8] = if value.is_empty() {
        b"\n"
    } else {
        value.as_bytes()
    };
    let callback = Box::new(
        move |server: &mut Server, editor: EditorId, buf: Option<Vec<u8>>| {
            edit_close(server, wp, edit_type, item, editor, buf);
        },
    );
    if let Ok(editor) = spawn_editor(server, &context, buf, callback) {
        with_state(server, mode, |_, state, _| {
            state.backend.editor = Some(editor);
        });
    }
}

/// `window_customize_key` for a mode entry; `c` may be absent from a menu.
fn key_impl(
    server: &mut Server,
    id: ModeId,
    c: Option<ClientId>,
    key: KeyCode,
    m: Option<&ResolvedMouseEvent>,
) {
    let wp = id.owner;
    // Mode prompts are dispatched by the tree engine before backend actions.
    let mut finished = false;
    let mut followup = Followup::default();
    let mut edit = None;
    let mut action = None;
    with_state(server, id, |server, state, screen| {
        if state.backend.editor.is_some() {
            let key = if key.0 & rmux_util::key::KeyModifiers::CTRL.bits() != 0
                && key.0 & !rmux_util::key::KeyModifiers::CTRL.bits() < 128
            {
                key.0 & 31
            } else {
                key.0
            };
            finished = key == u64::from(b'q') || key == u64::from(C0::ESC) || key == 3;
            return;
        }
        let result = state.key(server, id, c, key, m, screen);
        action = state.tree.pending_action.take();
        finished = result.finished;
        let key = result.key.0;
        let current = state.tree.get_current().and_then(|i| i.item);
        let item = current
            .and_then(|n| state.backend.items.get(n as usize))
            .cloned();
        let kind = item.as_ref().map(|i| i.kind);
        match key {
            k if k == u64::from(b'e') => {
                if let Some(idx) = current {
                    edit = Some(idx);
                }
            }
            k if k == u64::from(b'a') => {
                if let (Some(idx), Some(CustomizeItemType::Option)) = (current, kind) {
                    followup.prompt = set_array_key(server, state, idx);
                }
            }
            k if k == u64::from(b'\r') || k == u64::from(b's') => {
                let Some(idx) = current else {
                    followup.prompt = add_current(server, state);
                    followup.rebuild = followup.prompt.is_some();
                    return;
                };
                match kind {
                    Some(CustomizeItemType::Key) => followup.prompt = set_key(server, state, idx),
                    Some(CustomizeItemType::Environment) => {
                        followup.prompt = set_environment(server, state, idx, false)
                    }
                    _ => {
                        followup.prompt = set_option(server, state, idx, false, true);
                        followup
                            .pushes
                            .push(item.map(|i| i.name).unwrap_or_default());
                    }
                }
                followup.rebuild = true;
            }
            k if k == u64::from(b'w') => {
                if let (Some(idx), Some(CustomizeItemType::Option)) = (current, kind) {
                    followup.prompt = set_option(server, state, idx, false, false);
                    followup
                        .pushes
                        .push(item.map(|i| i.name).unwrap_or_default());
                    followup.rebuild = true;
                }
            }
            k if k == u64::from(b'S') || k == u64::from(b'W') => {
                let Some(idx) = current else {
                    return;
                };
                match kind {
                    Some(CustomizeItemType::Key) => return,
                    Some(CustomizeItemType::Environment) => {
                        followup.prompt = set_environment(server, state, idx, true)
                    }
                    _ => {
                        followup.prompt = set_option(server, state, idx, true, false);
                        followup
                            .pushes
                            .push(item.map(|i| i.name).unwrap_or_default());
                    }
                }
                followup.rebuild = true;
            }
            k if k == u64::from(b'd') => {
                let Some(item) = item else {
                    return;
                };
                if (item.kind == CustomizeItemType::Option && item.array_key.is_some())
                    || item.kind == CustomizeItemType::Environment
                {
                    return;
                }
                state.backend.change = CustomizeChange::Reset;
                followup.prompt = Some(PromptRequest {
                    msg: concat(&[b"Reset ", &item.name, b" to default? "]),
                    input: ByteString::new(),
                    flags: PromptFlags::SINGLE | PromptFlags::NOFORMAT | state.backend.prompt_flags,
                    kind: PromptKind::ChangeCurrent,
                });
            }
            k if k == u64::from(b'D') => {
                let tagged = state.tree.count_tagged();
                if tagged == 0 {
                    return;
                }
                state.backend.change = CustomizeChange::Reset;
                followup.prompt = Some(PromptRequest {
                    msg: ByteString::from(format!("Reset {tagged} tagged to default? ")),
                    input: ByteString::new(),
                    flags: PromptFlags::SINGLE | PromptFlags::NOFORMAT | state.backend.prompt_flags,
                    kind: PromptKind::ChangeTagged,
                });
            }
            k if k == u64::from(b'u') => {
                let Some(item) = item else {
                    return;
                };
                let msg = match &item.array_key {
                    Some(key) => concat(&[b"Unset ", &item.name, b"[", key, b"]? "]),
                    None => concat(&[b"Unset ", &item.name, b"? "]),
                };
                state.backend.change = CustomizeChange::Unset;
                followup.prompt = Some(PromptRequest {
                    msg,
                    input: ByteString::new(),
                    flags: PromptFlags::SINGLE | PromptFlags::NOFORMAT | state.backend.prompt_flags,
                    kind: PromptKind::ChangeCurrent,
                });
            }
            k if k == u64::from(b'U') => {
                let tagged = state.tree.count_tagged();
                if tagged == 0 {
                    return;
                }
                state.backend.change = CustomizeChange::Unset;
                followup.prompt = Some(PromptRequest {
                    msg: ByteString::from(format!("Unset {tagged} tagged? ")),
                    input: ByteString::new(),
                    flags: PromptFlags::SINGLE | PromptFlags::NOFORMAT | state.backend.prompt_flags,
                    kind: PromptKind::ChangeTagged,
                });
            }
            k if k == u64::from(b'H') => {
                state.backend.hide_global = !state.backend.hide_global;
                followup.rebuild = true;
            }
            k if k == u64::from(b'C') => {
                state.backend.hide_default = !state.backend.hide_default;
                followup.rebuild = true;
            }
            _ => {}
        }
    });
    if let Some(action) = action {
        action(server);
    }
    if finished {
        let _ = pane_reset_mode(server, wp);
        return;
    }
    if let Some(idx) = edit {
        start_edit(server, id, idx, c);
    }
    for name in std::mem::take(&mut followup.events) {
        hooks::add_event(server, &name);
    }
    for name in std::mem::take(&mut followup.pushes) {
        push(server, &name);
    }
    let do_rebuild = std::mem::take(&mut followup.rebuild);
    with_state(server, id, |server, state, screen| {
        if do_rebuild {
            state.build(server);
        }
        redraw(server, state, screen);
    });
    apply_followup(server, id, c, followup);
}

impl PaneModeDriver for CustomizeMode {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
        let wp = id.owner;
        let (sx, sy) = {
            let p = server.panes.get(wp)?;
            (p.base.grid.sx(), p.base.grid.sy())
        };
        let args = &self.args;
        let backend = CustomizeBackend {
            wp,
            fs: self.fs,
            format: args.get(b'F').unwrap_or(DEFAULT_FORMAT).to_vec(),
            hide_global: false,
            hide_default: false,
            prompt_flags: if args.has(b'y') != 0 {
                PromptFlags::ACCEPT
            } else {
                PromptFlags::default()
            },
            editor: None,
            change: CustomizeChange::Unset,
            items: Vec::new(),
            cache: Vec::new(),
        };
        let preview = ModeTreeData::preview_from_args(Some(args), backend.has_draw());
        let mut tree = ModeTreeData::start(sx, sy, preview);
        tree.filter = args.get(b'f').map(ByteString::from);
        tree.menu_items = MENU_ITEMS;
        let state = TreeModeState { tree, backend };
        let mut screen = Screen::new(
            sx,
            sy,
            0,
            ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .ok()?;
        screen.mode.remove(ScreenMode::CURSOR);
        let (mut state, mut screen) =
            super::tree::init_zoom(server, id, state, screen, Some(args))?;
        state.build(server);
        state.draw(server, &mut screen);
        match mode_mut(server, id) {
            Some(mode) => mode.data = Some(Box::new(state)),
            None => {
                state.free(server);
                let _ = screen.release(
                    &mut server.hyperlinks,
                    #[cfg(feature = "sixel")]
                    None,
                );
                return None;
            }
        }
        Some(screen)
    }

    fn free(&self, server: &mut Server, mut mode: PaneMode) {
        if let Some(state) = mode
            .data
            .take()
            .and_then(|d| d.downcast::<CustomizeState>().ok())
        {
            if let Some(editor) = state.backend.editor {
                spawn_cancel_editor(server, editor);
            }
            state.free(server);
        }
        if let Some(mut screen) = mode.screen.take() {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
        }
    }

    fn resize(&self, server: &mut Server, id: ModeId, sx: u32, sy: u32) {
        with_state(server, id, |server, state, screen| {
            state.resize(server, screen, sx, sy)
        });
    }

    fn update(&self, server: &mut Server, id: ModeId) {
        with_state(server, id, |server, state, screen| {
            draw_waiting(server, state.backend.editor, screen)
        });
    }

    fn key(
        &self,
        server: &mut Server,
        id: ModeId,
        client: ClientId,
        key: KeyCode,
        mouse: Option<&ResolvedMouseEvent>,
    ) {
        key_impl(server, id, Some(client), key, mouse);
    }

    fn append_output(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _bytes: &[u8],
    ) -> Result<(), ModelError> {
        Err(ModelError::Message(b"options-mode has no output".to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::commands::break_pane::tests::{create_session, create_window};
    use crate::model::pane::pane_set_mode;
    use crate::modes::WindowModeFlags;

    #[test]
    fn literals() {
        assert_eq!(NAME, b"options-mode");
        assert_eq!(HELP_LINES.len(), 12);
        assert!(HELP_LINES[0].ends_with("Set %1 value"));
        assert!(HELP_LINES[11].ends_with("Toggle information"));
        assert_eq!(MENU_ITEMS.len(), 11);
        assert_eq!(MENU_ITEMS[2].key, SpecialKey::RIGHT);
        assert_eq!(MENU_ITEMS[5].key, 0o24);
        assert!(DEFAULT_FORMAT.starts_with(
            b"#{?is_option,#{?option_is_global,,#[reverse](#{option_scope})#[default] }"
        ));
        assert!(DEFAULT_FORMAT.ends_with(b"#{key}}}"));
    }

    #[test]
    fn binding_child_tags_differ() {
        let tags = [
            binding_tag(7, 0),
            binding_tag(7, 1),
            binding_tag(7, 2),
            binding_tag(7, 3),
        ];
        for (i, a) in tags.iter().enumerate() {
            for (j, b) in tags.iter().enumerate() {
                assert_eq!(a == b, i == j);
            }
        }
        assert_ne!(binding_tag(7, 1), binding_tag(8, 1));
        assert_ne!(
            ModeTreeTag::Serial {
                kind: TAG_KEY_TABLE,
                value: 7,
                subitem: 0
            },
            binding_tag(7, 0)
        );
        assert_ne!(
            option_tag(7, None),
            ModeTreeTag::Serial {
                kind: TAG_ARRAY_ITEM,
                value: 7,
                subitem: 0
            }
        );
        let oe = options::search(b"escape-time").unwrap();
        let index = OPTIONS_TABLE
            .iter()
            .position(|e| std::ptr::eq(e, oe))
            .unwrap();
        assert_eq!(option_tag(1, Some(oe)), ModeTreeTag::OptionTable(index));
        assert_ne!(
            section_tag(SECTION_OPTIONS, 2),
            section_tag(SECTION_HOOKS, 2)
        );
    }

    #[test]
    fn scope_selection_rules() {
        use CustomizeScope as Sc;
        assert_eq!(select_scope(Sc::Session, true, false), Sc::GlobalSession);
        assert_eq!(select_scope(Sc::Pane, true, false), Sc::GlobalWindow);
        assert_eq!(select_scope(Sc::Server, true, false), Sc::Server);
        assert_eq!(select_scope(Sc::GlobalSession, false, true), Sc::Session);
        assert_eq!(select_scope(Sc::GlobalWindow, false, true), Sc::Pane);
        assert_eq!(select_scope(Sc::GlobalWindow, false, false), Sc::Window);
        assert_eq!(select_scope(Sc::Window, false, true), Sc::Pane);
        assert!(confirmed(b"y") && confirmed(b"Y") && !confirmed(b"yes") && !confirmed(b""));
    }

    #[test]
    fn changed_option_detection() {
        let mut s = Server::default();
        let global = s.options.global;
        let oe = options::search(b"escape-time").unwrap();
        with_store(&mut s, |store, ctx, _| {
            store.default(global, oe, &mut *ctx.parser);
        });
        assert!(!option_is_changed(&mut s, global, b"escape-time", None));
        s.options
            .set_number_value(global, b"escape-time", oe.default_num + 1);
        assert!(option_is_changed(&mut s, global, b"escape-time", None));

        let gs = s.options.global_s;
        let arr = options::search(b"update-environment").unwrap();
        with_store(&mut s, |store, ctx, _| {
            store.default(gs, arr, &mut *ctx.parser);
        });
        assert!(!option_is_changed(&mut s, gs, b"update-environment", None));
        assert!(!option_is_changed(
            &mut s,
            gs,
            b"update-environment",
            Some(&OptionsArrayKey::Index(0))
        ));
        with_store(&mut s, |store, ctx, _| {
            let o = store.get_mut_only(gs, b"update-environment").unwrap();
            o.array_set(
                &OptionsArrayKey::Index(99),
                Some(b"FOO"),
                false,
                &mut *ctx.parser,
            )
            .unwrap();
        });
        assert!(option_is_changed(&mut s, gs, b"update-environment", None));
        assert!(option_is_changed(
            &mut s,
            gs,
            b"update-environment",
            Some(&OptionsArrayKey::Index(99))
        ));
        assert!(!option_is_changed(
            &mut s,
            gs,
            b"update-environment",
            Some(&OptionsArrayKey::Index(0))
        ));

        with_store(&mut s, |store, ctx, _| {
            store.set_string(gs, b"@user", false, b"x", &mut *ctx.parser);
        });
        assert!(option_is_changed(&mut s, gs, b"@user", None));
    }

    fn mode_fixture(s: &mut Server, format: Option<&[u8]>) -> (PaneId, ModeId, CmdFindState) {
        let mut args = Args::create();
        if let Some(format) = format {
            args.set(
                b'F',
                Some(crate::cmd::arguments::ArgsValue::string(format.into())),
                Default::default(),
            );
        }
        mode_fixture_with_args(s, args)
    }

    fn mode_fixture_with_args(s: &mut Server, args: Args) -> (PaneId, ModeId, CmdFindState) {
        let session = create_session(s, b"main");
        let (_, _, wp) = create_window(s, session, 0);
        let fs = from_pane(&*s, wp, CmdFindFlags::default()).unwrap();
        let driver: Rc<dyn PaneModeDriver> = Rc::new(CustomizeMode::new(args, fs));
        let mode = pane_set_mode(s, wp, NAME, WindowModeFlags::default(), driver, false)
            .unwrap()
            .unwrap();
        (wp, mode, fs)
    }

    fn filtered_fixture(
        s: &mut Server,
        filter: &[u8],
        accept: bool,
    ) -> (PaneId, ModeId, CmdFindState) {
        let mut args = Args::create();
        args.set(
            b'f',
            Some(crate::cmd::arguments::ArgsValue::string(filter.into())),
            Default::default(),
        );
        if accept {
            args.set(b'y', None, Default::default());
        }
        mode_fixture_with_args(s, args)
    }

    fn attached_client(s: &mut Server, fs: &CmdFindState) -> ClientId {
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = fs.s;
        s.clients.insert(client).unwrap()
    }

    fn select_filtered_window_option(s: &mut Server, mode: ModeId, client: ClientId) {
        for key in [b'j' as u64, b'j' as u64, SpecialKey::RIGHT, b'j' as u64] {
            key_impl(s, mode, Some(client), KeyCode(key), None);
        }
        with_state(s, mode, |_, state, _| {
            assert_eq!(
                state.tree.get_current_name(),
                Some(b"window-style".as_slice())
            );
            let idx = state.tree.get_current().unwrap().item.unwrap();
            assert_eq!(
                state.backend.items[idx as usize].scope,
                CustomizeScope::Pane
            );
        });
    }

    #[test]
    fn filtered_reset_clears_pane_window_and_global_with_mode_prompt() {
        for top in [false, true] {
            let mut s = Server::default();
            let (wp, mode, fs) =
                filtered_fixture(&mut s, b"#{==:#{option_name},window-style}", false);
            let client = attached_client(&mut s, &fs);
            let so = s.sessions.get(fs.s.unwrap()).unwrap().options;
            s.options
                .set_number_value(so, b"status-position", i64::from(!top));
            let gw = s.options.global_w;
            let wo = s.windows.get(fs.w.unwrap()).unwrap().options;
            let po = s.panes.get(wp).unwrap().options;
            with_store(&mut s, |store, ctx, _| {
                for (oo, value) in [
                    (gw, b"fg=red".as_slice()),
                    (wo, b"fg=green".as_slice()),
                    (po, b"fg=blue".as_slice()),
                ] {
                    store.set_string(oo, b"window-style", false, value, &mut *ctx.parser);
                }
            });
            with_state(&mut s, mode, rebuild);
            select_filtered_window_option(&mut s, mode, client);
            key_impl(&mut s, mode, Some(client), KeyCode(b'd' as u64), None);
            assert!(s.panes.get(wp).unwrap().prompt.is_none());
            with_state(&mut s, mode, |_, state, screen| {
                assert!(state.tree.prompt.is_some());
                let row = if top { 0 } else { screen.grid.sy() - 1 };
                let message: Vec<u8> = (0..screen.grid.sx())
                    .map(|x| screen.grid.view_get_cell(x, row).data.data[0])
                    .collect();
                assert!(message.starts_with(b"Reset window-style to default? "));
                assert_eq!(screen.cy, row);
                assert!(screen.mode.contains(ScreenMode::CURSOR));
            });
            key_impl(&mut s, mode, Some(client), KeyCode(b'Y' as u64), None);
            with_state(&mut s, mode, |_, state, _| {
                assert!(state.tree.prompt.is_none());
            });
            assert_eq!(s.options.get_string(gw, b"window-style"), b"default");
            assert!(s.options.get_only(wo, b"window-style").is_none());
            assert!(s.options.get_only(po, b"window-style").is_none());
            with_state(&mut s, mode, |_, state, screen| {
                let idx = state.tree.get_current().unwrap().item.unwrap();
                assert_eq!(
                    state.backend.items[idx as usize].scope,
                    CustomizeScope::GlobalWindow
                );
                assert!(!screen.mode.contains(ScreenMode::CURSOR));
            });
        }
    }

    #[test]
    fn filtered_reset_accept_and_cancel_preserve_confirmation_rules() {
        for (accept, reply, reset) in [
            (true, None, true),
            (false, Some(b'n' as u64), false),
            (false, Some(C0::ESC as u64), false),
        ] {
            let mut s = Server::default();
            let (wp, mode, fs) =
                filtered_fixture(&mut s, b"#{==:#{option_name},window-style}", accept);
            let client = attached_client(&mut s, &fs);
            let po = s.panes.get(wp).unwrap().options;
            with_store(&mut s, |store, ctx, _| {
                store.set_string(po, b"window-style", false, b"fg=blue", &mut *ctx.parser);
            });
            with_state(&mut s, mode, rebuild);
            select_filtered_window_option(&mut s, mode, client);
            key_impl(&mut s, mode, Some(client), KeyCode(b'd' as u64), None);
            if let Some(reply) = reply {
                key_impl(&mut s, mode, Some(client), KeyCode(reply), None);
            }
            crate::cmd::queue::next(&mut s, Some(client));
            assert_eq!(s.options.get_only(po, b"window-style").is_none(), reset);
            assert_eq!(first_mode(&s, wp), Some(mode));
            with_state(&mut s, mode, |_, state, _| {
                assert!(state.tree.prompt.is_none());
            });
        }
    }

    #[test]
    fn initial_filter_selects_only_requested_item_types() {
        for (filter, kind) in [
            (b"#{is_key}".as_slice(), CustomizeItemType::Key),
            (
                b"#{is_environment}".as_slice(),
                CustomizeItemType::Environment,
            ),
        ] {
            let mut s = Server::default();
            s.global_environment
                .set(b"CM_ENV", EnvironmentFlags(0), b"value");
            let list = CommandParser::parse_from_string(&mut s, b"display-message value").unwrap();
            s.key_bindings
                .add(b"prefix", KeyCode(b'x' as u64), None, false, Some(list))
                .unwrap();
            let (_, mode, _) = filtered_fixture(&mut s, filter, false);
            with_state(&mut s, mode, |_, state, _| {
                assert_eq!(state.tree.filter.as_ref().unwrap().as_bytes(), filter);
                assert!(!state.backend.items.is_empty());
                assert!(state.backend.items.iter().all(|item| item.kind == kind));
                assert!(!state.tree.no_matches);
            });
        }
    }

    #[test]
    fn stale_option_prompt_does_not_recreate_removed_user_option() {
        let mut s = Server::default();
        let (wp, mode, fs) = mode_fixture(&mut s, None);
        let client = attached_client(&mut s, &fs);
        let global = s.options.global;
        with_store(&mut s, |store, ctx, _| {
            store.set_string(global, b"@stale", false, b"old", &mut *ctx.parser);
        });
        with_state(&mut s, mode, |server, state, screen| {
            state.tree.filter = Some(b"#{==:#{option_name},@stale}".as_slice().into());
            rebuild(server, state, screen);
        });
        for key in [SpecialKey::RIGHT, b'j' as u64, b'\r' as u64] {
            key_impl(&mut s, mode, Some(client), KeyCode(key), None);
        }
        with_state(&mut s, mode, |_, state, _| {
            assert!(state.tree.prompt.is_some());
        });
        with_store(&mut s, |store, ctx, freed| {
            store
                .remove_or_default(global, b"@stale", None, &mut *ctx.parser, freed)
                .unwrap();
        });
        for key in [KeyCode(21), KeyCode(b'n' as u64), KeyCode(b'\r' as u64)] {
            key_impl(&mut s, mode, Some(client), key, None);
        }
        assert!(s.options.get_only(global, b"@stale").is_none());
        assert_eq!(first_mode(&s, wp), Some(mode));
    }

    #[test]
    fn array_rename_preserves_collision_and_failed_parse_then_moves_value() {
        let mut s = Server::default();
        let (_, mode, fs) = mode_fixture(&mut s, None);
        let gs = s.options.global_s;
        let oe = options::search(b"after-new-session").unwrap();
        with_store(&mut s, |store, ctx, _| {
            let o = store.default(gs, oe, &mut *ctx.parser);
            o.array_set(
                &OptionsArrayKey::Index(0),
                Some(b"display-message old"),
                false,
                &mut *ctx.parser,
            )
            .unwrap();
            o.array_set(
                &OptionsArrayKey::Index(1),
                Some(b"display-message taken"),
                false,
                &mut *ctx.parser,
            )
            .unwrap();
        });
        let mut item = CustomizeItem::new(CustomizeItemType::Option);
        item.scope = CustomizeScope::GlobalSession;
        item.oo = Some(gs);
        item.name = b"after-new-session".as_slice().into();
        item.array_key = Some(b"0".as_slice().into());
        for key in [b"1".as_slice(), b"4294967296".as_slice()] {
            let mut followup = Followup::default();
            set_array_key_fire(&mut s, &item, key, &fs, mode.owner, &mut followup);
            assert!(!followup.rebuild);
            assert!(followup.pushes.is_empty());
            let o = s.options.get_only(gs, b"after-new-session").unwrap();
            assert_eq!(
                o.to_string(Some(&OptionsArrayKey::Index(0)), false)
                    .as_bytes(),
                b"display-message old"
            );
            assert_eq!(
                o.to_string(Some(&OptionsArrayKey::Index(1)), false)
                    .as_bytes(),
                b"display-message taken"
            );
            assert_eq!(followup.status.is_some(), key == b"4294967296");
        }
        let mut followup = Followup::default();
        set_array_key_fire(&mut s, &item, b"2", &fs, mode.owner, &mut followup);
        assert!(followup.rebuild);
        assert_eq!(followup.pushes, [ByteString::from("after-new-session")]);
        let o = s.options.get_only(gs, b"after-new-session").unwrap();
        assert!(o.array_get(&OptionsArrayKey::Index(0)).is_none());
        assert_eq!(
            o.to_string(Some(&OptionsArrayKey::Index(2)), false)
                .as_bytes(),
            b"display-message old"
        );
    }

    #[test]
    fn tagged_reset_skips_environment_and_array_elements() {
        let mut s = Server::default();
        let (_, mode, _) = mode_fixture(&mut s, None);
        let gs = s.options.global_s;
        with_store(&mut s, |store, ctx, _| {
            store.set_string(gs, b"@reset", false, b"old", &mut *ctx.parser);
            let o = store.get_mut_only(gs, b"update-environment").unwrap();
            o.array_set(
                &OptionsArrayKey::Index(99),
                Some(b"CM_ARRAY"),
                false,
                &mut *ctx.parser,
            )
            .unwrap();
        });
        s.global_environment
            .set(b"CM_ENV", EnvironmentFlags(0), b"old");
        with_state(&mut s, mode, |server, state, screen| {
            rebuild(server, state, screen);
            let mut ids = state.tree.children.clone();
            for id in &state.tree.children {
                state.tree.items.get_mut(*id).unwrap().expanded = true;
            }
            while let Some(id) = ids.pop() {
                let item = state.tree.items.get_mut(id).unwrap();
                ids.extend_from_slice(&item.children);
                if item.name.as_bytes() == b"@reset"
                    || item.name.as_bytes() == b"CM_ENV"
                    || item.name.as_bytes() == b"update-environment"
                {
                    item.expanded = true;
                }
                if item.name.as_bytes() == b"@reset"
                    || item.name.as_bytes() == b"CM_ENV"
                    || item.name.as_bytes() == b"update-environment[99]"
                {
                    item.tagged = true;
                }
            }
            state.tree.build_lines();
            state.backend.change = CustomizeChange::Reset;
            assert_eq!(state.tree.count_tagged(), 3);
        });
        prompt_fire(&mut s, mode, None, &PromptKind::ChangeTagged, b"y");
        assert!(s.options.get_only(gs, b"@reset").is_none());
        assert_eq!(
            s.global_environment
                .find(b"CM_ENV")
                .unwrap()
                .value
                .as_ref()
                .unwrap()
                .as_bytes(),
            b"old"
        );
        assert_eq!(
            s.options
                .get_only(gs, b"update-environment")
                .unwrap()
                .to_string(Some(&OptionsArrayKey::Index(99)), false)
                .as_bytes(),
            b"CM_ARRAY"
        );
    }

    #[test]
    fn customize_without_swap_callback_does_not_change_selection() {
        let mut s = Server::default();
        let (_, mode, fs) = mode_fixture(&mut s, None);
        let client = attached_client(&mut s, &fs);
        let global = s.options.global;
        with_store(&mut s, |store, ctx, _| {
            store.set_string(global, b"@a", false, b"a", &mut *ctx.parser);
            store.set_string(global, b"@b", false, b"b", &mut *ctx.parser);
        });
        with_state(&mut s, mode, |server, state, screen| {
            state.tree.filter = Some(b"#{m/r:^@[ab]$,#{option_name}}".as_slice().into());
            rebuild(server, state, screen);
        });
        for key in [SpecialKey::RIGHT, b'j' as u64, b'J' as u64] {
            key_impl(&mut s, mode, Some(client), KeyCode(key), None);
        }
        assert_eq!(crate::cmd::queue::next(&mut s, Some(client)), 0);
        with_state(&mut s, mode, |_, state, _| {
            assert_eq!(state.tree.get_current_name(), Some(b"@a".as_slice()));
        });
    }

    #[test]
    fn accepted_prompt_cannot_mutate_a_replacement_mode() {
        let mut s = Server::default();
        let (wp, mode, fs) = filtered_fixture(&mut s, b"#{==:#{option_name},window-style}", true);
        let client = attached_client(&mut s, &fs);
        let po = s.panes.get(wp).unwrap().options;
        with_store(&mut s, |store, ctx, _| {
            store.set_string(po, b"window-style", false, b"fg=blue", &mut *ctx.parser);
        });
        with_state(&mut s, mode, rebuild);
        select_filtered_window_option(&mut s, mode, client);
        key_impl(&mut s, mode, Some(client), KeyCode(b'd' as u64), None);
        pane_reset_mode(&mut s, wp).unwrap();
        let driver: Rc<dyn PaneModeDriver> = Rc::new(CustomizeMode::new(Args::create(), fs));
        let replacement =
            pane_set_mode(&mut s, wp, NAME, WindowModeFlags::default(), driver, false)
                .unwrap()
                .unwrap();
        crate::cmd::queue::next(&mut s, Some(client));
        assert_ne!(replacement, mode);
        assert_eq!(s.options.get_string(po, b"window-style"), b"fg=blue");
        assert_eq!(first_mode(&s, wp), Some(replacement));
    }

    fn root_names(s: &mut Server, mode: ModeId) -> Vec<Vec<u8>> {
        let mut names = Vec::new();
        with_state(s, mode, |_, state, _| {
            names = state
                .tree
                .children
                .iter()
                .map(|id| state.tree.items.get(*id).unwrap().name.to_vec())
                .collect();
        });
        names
    }

    #[test]
    fn section_order_and_format_variables() {
        let mut s = Server::default();
        let session = create_session(&mut s, b"main");
        let so = s.sessions.get(session).unwrap().options;
        with_store(&mut s, |store, ctx, _| {
            store.set_string(so, b"@foo", false, b"bar", &mut *ctx.parser);
        });
        s.global_environment
            .set(b"PATH", EnvironmentFlags(0), b"/bin");
        s.global_environment.clear(b"GONE");
        let (_, _, wp) = create_window(&mut s, session, 0);
        let fs = from_pane(&s, wp, CmdFindFlags::default()).unwrap();
        let mut args = Args::create();
        let format: &[u8] = b"#{is_option}#{is_key}#{is_environment}:#{option_name}=#{option_value}/#{option_is_global}/#{option_scope}:#{environment_name}=#{environment_value}/#{environment_is_global}/#{environment_removed}";
        args.set(
            b'F',
            Some(crate::cmd::arguments::ArgsValue::string(format.into())),
            Default::default(),
        );
        let driver: Rc<dyn PaneModeDriver> = Rc::new(CustomizeMode::new(args, fs));
        let mode = pane_set_mode(&mut s, wp, NAME, WindowModeFlags::default(), driver, false)
            .unwrap()
            .unwrap();

        assert_eq!(
            root_names(&mut s, mode),
            [
                b"Server Options".to_vec(),
                b"Session Options".to_vec(),
                b"Window & Pane Options".to_vec(),
                b"Session Hooks".to_vec(),
                b"Window & Pane Hooks".to_vec(),
                b"Global Environment".to_vec(),
                b"Session Environment".to_vec(),
            ]
        );
        with_state(&mut s, mode, |_, state, _| {
            let tree = &state.tree;
            let session_options = tree.items.get(tree.children[1]).unwrap();
            let foo = tree.items.get(session_options.children[0]).unwrap();
            assert_eq!(foo.name.as_bytes(), b"@foo");
            assert_eq!(
                foo.text.as_ref().unwrap().as_bytes(),
                b"100:@foo=bar/0/session main:=//".as_slice()
            );
            let item = &state.backend.items[foo.item.unwrap() as usize];
            assert_eq!(item.scope, CustomizeScope::Session);
            assert_eq!(item.oo, Some(so));

            let global_env = tree.items.get(tree.children[5]).unwrap();
            let names: Vec<&[u8]> = global_env
                .children
                .iter()
                .map(|id| tree.items.get(*id).unwrap().name.as_bytes())
                .collect();
            assert_eq!(names, [b"-GONE".as_slice(), b"PATH".as_slice()]);
            let path = tree.items.get(global_env.children[1]).unwrap();
            assert!(
                path.text
                    .as_ref()
                    .unwrap()
                    .as_bytes()
                    .ends_with(b":PATH=/bin/1/0")
            );
            assert!(
                tree.items
                    .get(global_env.children[0])
                    .unwrap()
                    .text
                    .is_none()
            );
            assert!(session_options.no_tag && !foo.no_tag);
        });
    }

    #[test]
    fn editor_completion_requires_first_options_mode() {
        let mut s = Server::default();
        let (wp, mode, fs) = mode_fixture(&mut s, None);
        let so = s.sessions.get(fs.s.unwrap()).unwrap().options;
        with_store(&mut s, |store, ctx, _| {
            store.set_string(so, b"@foo", false, b"old", &mut *ctx.parser);
        });
        let mut item = CustomizeItem::new(CustomizeItemType::Option);
        item.scope = CustomizeScope::Session;
        item.oo = Some(so);
        item.name = ByteString::from("@foo");
        let editor = {
            use crate::ids::ArenaId;
            EditorId::from_parts(3, 1)
        };
        with_state(&mut s, mode, |_, state, _| {
            state.backend.editor = Some(editor)
        });

        struct Other;
        impl PaneModeDriver for Other {
            fn init(&self, server: &mut Server, _id: ModeId) -> Option<Screen> {
                Screen::new(
                    1,
                    1,
                    0,
                    ScreenResetPolicy::default(),
                    &mut server.hyperlinks,
                )
                .ok()
            }
            fn free(&self, server: &mut Server, mut mode: PaneMode) {
                if let Some(mut screen) = mode.screen.take() {
                    let _ = screen.release(
                        &mut server.hyperlinks,
                        #[cfg(feature = "sixel")]
                        None,
                    );
                }
            }
            fn resize(&self, _: &mut Server, _: ModeId, _: u32, _: u32) {}
            fn key(
                &self,
                _: &mut Server,
                _: ModeId,
                _: ClientId,
                _: KeyCode,
                _: Option<&ResolvedMouseEvent>,
            ) {
            }
            fn append_output(&self, _: &mut Server, _: ModeId, _: &[u8]) -> Result<(), ModelError> {
                Ok(())
            }
        }
        pane_set_mode(
            &mut s,
            wp,
            b"other",
            WindowModeFlags::default(),
            Rc::new(Other),
            false,
        )
        .unwrap()
        .unwrap();

        // Another mode on top: no mutation and the editor marker stays.
        edit_close(
            &mut s,
            wp,
            CustomizeEditType::Option,
            item.clone(),
            editor,
            Some(b"new\n".to_vec()),
        );
        assert_eq!(
            s.options
                .get_only(so, b"@foo")
                .unwrap()
                .to_string(None, false)
                .as_bytes(),
            b"old"
        );
        let mut marker = None;
        with_state(&mut s, mode, |_, state, _| {
            marker = Some(state.backend.editor)
        });
        assert_eq!(marker, Some(Some(editor)));

        pane_reset_mode(&mut s, wp).unwrap();
        assert_eq!(first_mode(&s, wp), Some(mode));

        // Empty output: marker cleared, no mutation.
        edit_close(
            &mut s,
            wp,
            CustomizeEditType::Option,
            item.clone(),
            editor,
            Some(Vec::new()),
        );
        assert_eq!(
            s.options
                .get_only(so, b"@foo")
                .unwrap()
                .to_string(None, false)
                .as_bytes(),
            b"old"
        );
        with_state(&mut s, mode, |_, state, _| {
            marker = Some(state.backend.editor)
        });
        assert_eq!(marker, Some(None));

        // One trailing newline is dropped, even when only it remains.
        edit_close(
            &mut s,
            wp,
            CustomizeEditType::Option,
            item.clone(),
            editor,
            Some(b"new\n".to_vec()),
        );
        assert_eq!(
            s.options
                .get_only(so, b"@foo")
                .unwrap()
                .to_string(None, false)
                .as_bytes(),
            b"new"
        );
        edit_close(
            &mut s,
            wp,
            CustomizeEditType::Option,
            item.clone(),
            editor,
            Some(b"\n".to_vec()),
        );
        assert_eq!(
            s.options
                .get_only(so, b"@foo")
                .unwrap()
                .to_string(None, false)
                .as_bytes(),
            b""
        );

        // A stale scope is rejected.
        let mut stale = item.clone();
        stale.scope = CustomizeScope::GlobalSession;
        edit_close(
            &mut s,
            wp,
            CustomizeEditType::Option,
            stale,
            editor,
            Some(b"again".to_vec()),
        );
        assert_eq!(
            s.options
                .get_only(so, b"@foo")
                .unwrap()
                .to_string(None, false)
                .as_bytes(),
            b""
        );
    }
}
