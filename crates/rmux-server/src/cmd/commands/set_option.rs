// Ported from tmux cmd-set-option.c @ 8f25579c
use crate::cmd::Command;
use crate::cmd::arguments::Args;
use crate::cmd::find::CmdFindState;
use crate::cmd::hooks::{self, MonitorSpec};
use crate::cmd::metadata::{CMD_SET_HOOK, CMD_SET_WINDOW_OPTION};
use crate::cmd::queue::CmdReturn;
use crate::format;
use crate::ids::{HooksMonitorId, QueueItemId, WindowId};
use crate::model::monitor::{MonitorFlags, MonitorType, monitor_parse};
use crate::options::push::push_changes;
use crate::options::scope::{OptionsScopeFlags, OptionsScopeTarget, scope_from_name};
use crate::options::{
    Ambiguous, MonitorSink, OptionName, OptionsArrayKey, OptionsParseCtx, OptionsScope,
    OptionsStore, match_name,
};
use crate::server::Server;
use crate::server::events::{self, EventPayload};
use rmux_util::bytes::ByteString;

use super::support::{concat, fail, item_client, item_target};

/// `getprogname()` for `checkshell` inside `options_from_string`.
const PROGRAM: &[u8] = b"rmux";

/// The `args_has` flags `options_scope_from_name/flags` read.
pub(super) fn scope_flags(args: &Args, window_command: bool) -> OptionsScopeFlags {
    OptionsScopeFlags {
        global: args.has(b'g') != 0,
        server: args.has(b's') != 0,
        window: args.has(b'w') != 0,
        pane: args.has(b'p') != 0,
        window_command,
    }
}

/// The option trees of a `cmd_find_state` plus the raw `-t` text
/// (`options.c:1013-1137` reads `s->options`, `wl->window->options`,
/// `wp->options`).
pub(super) fn scope_target<'a>(
    server: &Server,
    target: &CmdFindState,
    args: &'a Args,
) -> OptionsScopeTarget<'a> {
    OptionsScopeTarget {
        session: target
            .s
            .and_then(|s| server.sessions.get(s))
            .map(|s| s.options),
        window: target
            .wl
            .and_then(|wl| server.winlinks.get(wl))
            .and_then(|wl| server.windows.get(wl.window))
            .map(|w| w.options),
        pane: target
            .wp
            .and_then(|wp| server.panes.get(wp))
            .map(|wp| wp.options),
        target: args.get(b't'),
    }
}

/// Collects the monitors `options_remove` detaches so they can be freed
/// through `hooks_monitor_free` once the store borrow ends.
#[derive(Default)]
struct FreedMonitors(Vec<HooksMonitorId>);

impl MonitorSink for FreedMonitors {
    fn monitor_free(&mut self, monitor: HooksMonitorId) {
        self.0.push(monitor);
    }
}

/// Run `f` on the option store with a parse context split off `server`
/// (the store and hyperlinks are taken out while `server` acts as the
/// command parser) and free any monitors the operation detached.
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
            program: PROGRAM,
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

/// `options_push_changes(name)` plus the server-side application of the
/// resulting change list.
fn push(server: &mut Server, name: &[u8]) {
    let changes = push_changes(name);
    crate::server::run::apply_option_changes(server, changes);
}

fn fail_with(server: &mut Server, item: QueueItemId, prefix: &[u8], value: &[u8]) -> CmdReturn {
    fail(server, item, concat(&[prefix, value]))
}

/// Decide the `-U` pane sweep (`cmd-set-option.c:321-322`): `Ok(None)` skips
/// it, `Ok(Some(w))` sweeps the panes of `w`. Upstream dereferences
/// `target->w` unconditionally; a window scope without a current window
/// (`set-option -gU`) is the approved `no current window` error instead.
pub(super) fn unset_panes_window(
    unset_panes: bool,
    scope: OptionsScope,
    window: Option<WindowId>,
) -> Result<Option<WindowId>, &'static [u8]> {
    if !unset_panes || scope != OptionsScope::WINDOW {
        return Ok(None);
    }
    match window {
        Some(w) => Ok(Some(w)),
        None => Err(b"no current window"),
    }
}

/// `cmd_set_hook_event_exec` (`cmd-set-option.c:90-133`).
fn hook_event(server: &mut Server, args: &Args, item: QueueItemId) -> CmdReturn {
    if args.count() == 0 {
        return fail(server, item, b"missing argument");
    }
    if args.count() != 1 {
        return fail(server, item, b"too many arguments");
    }
    let argument = format::single_from_target(server, item, args.string(0).unwrap_or(b""));
    if argument.first() != Some(&b'@') {
        return fail(server, item, b"event name must start with @");
    }
    let target = item_target(server, item);
    let mut payload = EventPayload::new();
    payload.set_target(server, &target);
    if let Some(c) = item_client(server, item).filter(|c| server.clients.get(*c).is_some()) {
        payload.set_client(server, b"client", c);
    }
    if let Some(s) = target.s.filter(|s| server.sessions.get(*s).is_some()) {
        payload.set_session(server, b"session", s);
    }
    if let Some(w) = target.w.filter(|w| server.windows.get(*w).is_some()) {
        payload.set_window(server, b"window", w);
    }
    if let Some(index) = target
        .wl
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.index)
    {
        payload.set_int(server, b"window_index", index);
    } else if target.wl.is_none() && target.idx != -1 {
        payload.set_int(server, b"window_index", target.idx);
    }
    if let Some(wp) = target.wp.filter(|wp| server.panes.get(*wp).is_some()) {
        payload.set_pane(server, b"pane", wp);
    }
    events::fire(server, &argument, payload);
    CmdReturn::Normal
}

/// `cmd_set_hook_monitor_exec` (`cmd-set-option.c:135-224`).
fn hook_monitor(server: &mut Server, args: &Args, item: QueueItemId, window: bool) -> CmdReturn {
    if args.count() > 1 {
        return fail(server, item, b"too many arguments");
    }
    let raw = args.get(b'B').unwrap_or(b"");
    let unset = args.has(b'u') != 0;
    let (name, parsed): (ByteString, Option<(MonitorType, i32, ByteString)>) =
        match monitor_parse(raw) {
            Ok(spec) => {
                let id = spec.target.map_or(-1, |t| t as i32);
                (spec.name, (!unset).then_some((spec.kind, id, spec.format)))
            }
            Err(_) if unset => (ByteString::from(raw), None),
            Err(_) => return fail_with(server, item, b"invalid subscription: ", raw),
        };
    if name.first() != Some(&b'@') {
        return fail(server, item, b"monitor hook name must start with @");
    }

    let target = item_target(server, item);
    let flags = scope_flags(args, window);
    let scope_target = scope_target(server, &target, args);
    let (_, oo) = match scope_from_name(flags, &name, &scope_target, &server.options) {
        Ok(found) => found,
        Err(cause) => return fail(server, item, cause.as_bytes()),
    };
    let mut fs = CmdFindState::default();
    fs.copy_target_from(&target);

    if unset {
        hooks::monitor_remove(server, oo, &name);
        return CmdReturn::Normal;
    }

    if args.count() != 0 {
        let mut value = ByteString::from(args.string(0).unwrap_or(b""));
        if args.has(b'F') != 0 {
            value = format::single_from_target(server, item, &value);
        }
        let exists = server.options.get_only(oo, &name).is_some();
        if args.has(b'o') == 0 || !exists {
            if args.has(b'a') != 0 && exists {
                let old = server.options.get_string(oo, &name);
                value = ByteString(concat(&[old, &value]));
            }
            with_store(server, |store, ctx, _| {
                store.set_string(oo, &name, false, &value, ctx.parser);
            });
            push(server, &name);
        }
    }

    let global = oo == server.options.global
        || oo == server.options.global_s
        || oo == server.options.global_w;
    let session = if global { None } else { target.s };
    let mut monitor_flags = MonitorFlags::default();
    if args.has(b'T') != 0 {
        monitor_flags.insert(MonitorFlags::TRUE);
    }
    let (kind, public_id, monitor_format) = parsed.expect("monitor spec parsed without -u");
    let spec = MonitorSpec {
        options: oo,
        name: &name,
        kind,
        public_id,
        format: &monitor_format,
        flags: monitor_flags,
        target: &fs,
        session,
    };
    if hooks::monitor_add(server, spec).is_err() {
        return fail_with(server, item, b"can't add monitor: ", &name);
    }
    CmdReturn::Normal
}

/// `cmd_set_option_exec` (`cmd-set-option.c:226-397`).
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let is_hook = std::ptr::eq(command.entry, &CMD_SET_HOOK);
    let window = std::ptr::eq(command.entry, &CMD_SET_WINDOW_OPTION);
    let append = args.has(b'a') != 0;
    let quiet = args.has(b'q') != 0;

    if is_hook && args.has(b'E') != 0 {
        return hook_event(server, args, item);
    }
    if is_hook && args.has(b'B') != 0 {
        return hook_monitor(server, args, item, window);
    }
    if args.count() == 0 {
        return fail(server, item, b"missing argument");
    }

    let argument = format::single_from_target(server, item, args.string(0).unwrap_or(b""));

    if is_hook && args.has(b'R') != 0 {
        hooks::run(server, item, &argument);
        return CmdReturn::Normal;
    }

    let (name, array_key): (OptionName, Option<OptionsArrayKey>) = match match_name(&argument) {
        Ok(Some(found)) => found,
        Ok(None) => {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail_with(server, item, b"invalid option: ", &argument);
        }
        Err(Ambiguous) => {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail_with(server, item, b"ambiguous option: ", &argument);
        }
    };
    let is_user = matches!(name, OptionName::User(_));
    let name = name.as_bytes();
    let mut value: Option<ByteString> = args.string(1).map(ByteString::from);
    if let Some(v) = &value
        && args.has(b'F') != 0
    {
        value = Some(format::single_from_target(server, item, v));
    }

    let target = item_target(server, item);
    let flags = scope_flags(args, window);
    let scope_target = scope_target(server, &target, args);
    let (scope, oo) = match scope_from_name(flags, name, &scope_target, &server.options) {
        Ok(found) => found,
        Err(cause) => {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail(server, item, cause.as_bytes());
        }
    };
    let exists = server.options.get_only(oo, name).is_some();
    let parent = server.options.get(oo, name).map(|(_, o)| o);
    let parent_is_array = parent.is_some_and(|o| o.is_array());
    let parent_table = parent.and_then(|o| o.table_entry());

    if array_key.is_some() && (is_user || !parent_is_array) {
        return fail_with(server, item, b"not an array: ", &argument);
    }

    let unset = args.has(b'u') != 0;
    let unset_panes = args.has(b'U') != 0;
    if !unset && args.has(b'o') != 0 {
        let already = match &array_key {
            None => exists,
            Some(key) => server
                .options
                .get_only(oo, name)
                .is_some_and(|o| o.array_get(key).is_some()),
        };
        if already {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail_with(server, item, b"already set: ", &argument);
        }
    }

    match unset_panes_window(unset_panes, scope, target.w) {
        Err(cause) => return fail(server, item, cause),
        Ok(Some(w)) => {
            let panes = server
                .windows
                .get(w)
                .map(|w| w.panes.clone())
                .unwrap_or_default();
            for pane in panes {
                let Some(po) = server.panes.get(pane).map(|p| p.options) else {
                    continue;
                };
                if server.options.get_only(po, name).is_none() {
                    continue;
                }
                let result = with_store(server, |store, ctx, sink| {
                    store.remove_or_default(po, name, array_key.as_ref(), ctx.parser, sink)
                });
                if let Err(cause) = result {
                    return fail(server, item, cause.as_bytes());
                }
            }
        }
        Ok(None) => {}
    }

    if unset || unset_panes {
        if !exists {
            return CmdReturn::Normal;
        }
        let result = with_store(server, |store, ctx, sink| {
            store.remove_or_default(oo, name, array_key.as_ref(), ctx.parser, sink)
        });
        if let Err(cause) = result {
            return fail(server, item, cause.as_bytes());
        }
    } else if is_user {
        let Some(value) = &value else {
            return fail(server, item, b"empty value");
        };
        with_store(server, |store, ctx, _| {
            store.set_string(oo, name, append, value, ctx.parser);
        });
        if is_hook {
            hooks::add_event(server, name);
        }
    } else if array_key.is_none() && !parent_is_array {
        let table_name = parent_table.map_or(name, |oe| oe.name);
        let value = value.as_ref().map(ByteString::as_bytes);
        let result = with_store(server, |store, ctx, _| {
            store.from_string(oo, parent_table, table_name, value, append, ctx)
        });
        if let Err(cause) = result {
            return fail(server, item, cause.as_bytes());
        }
    } else {
        let Some(value) = &value else {
            return fail(server, item, b"empty value");
        };
        let Some(oe) = parent_table else {
            return fail_with(server, item, b"not an array: ", &argument);
        };
        let result = with_store(server, |store, ctx, _| {
            if !exists {
                store.empty(oo, oe);
            }
            let o = store.get_mut_only(oo, name).expect("array option entry");
            match &array_key {
                None => {
                    if !append {
                        o.array_clear();
                    }
                    o.array_assign(value, ctx.parser)
                }
                Some(key) => o.array_set(key, Some(value), append, ctx.parser),
            }
        });
        if let Err(cause) = result {
            return fail(server, item, cause.as_bytes());
        }
    }

    push(server, name);
    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Arena;

    fn window_id() -> WindowId {
        let mut arena: Arena<(), WindowId> = Arena::new();
        arena.insert(()).unwrap()
    }

    /// Spec unit test 9: `set-option -gU` on a window option without a
    /// current window reports `no current window` before any mutation.
    #[test]
    fn global_unset_panes_without_window_is_an_error() {
        assert_eq!(
            unset_panes_window(true, OptionsScope::WINDOW, None),
            Err(b"no current window".as_slice())
        );
    }

    #[test]
    fn unset_panes_sweeps_only_window_scope_with_a_window() {
        let w = window_id();
        assert_eq!(
            unset_panes_window(true, OptionsScope::WINDOW, Some(w)),
            Ok(Some(w))
        );
        assert_eq!(
            unset_panes_window(true, OptionsScope::SESSION, None),
            Ok(None)
        );
        assert_eq!(
            unset_panes_window(true, OptionsScope::PANE, Some(w)),
            Ok(None)
        );
        assert_eq!(
            unset_panes_window(false, OptionsScope::WINDOW, None),
            Ok(None)
        );
    }

    #[test]
    fn scope_flags_read_every_letter() {
        let spec = crate::cmd::arguments::ArgsParse {
            template: b"aFgopqst:uUw",
            lower: 1,
            upper: 2,
            cb: None,
        };
        let args = spec
            .parse(&[
                crate::cmd::arguments::ArgsValue::string("set-option".into()),
                crate::cmd::arguments::ArgsValue::string("-gsw".into()),
                crate::cmd::arguments::ArgsValue::string("-p".into()),
                crate::cmd::arguments::ArgsValue::string("status".into()),
            ])
            .unwrap()
            .unwrap();
        assert_eq!(
            scope_flags(&args, true),
            OptionsScopeFlags {
                global: true,
                server: true,
                window: true,
                pane: true,
                window_command: true,
            }
        );
    }
}
