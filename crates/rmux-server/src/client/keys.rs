// Ported from tmux server-client.c @ 8f25579c
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

//! Key tables, key dispatch and the repeat timer (`server-client.c:112-155`,
//! `1194-1750`, `2181-2191`).

use crate::client::{Client, ClientFlags, KeyEvent, lifecycle, registry};
use crate::cmd::CommandList;
use crate::cmd::find::{self, CmdFindFlags, CmdFindState, MouseInput};
use crate::cmd::key_bindings::{
    self, KeyBindingFlags, KeyBindings, KeyDispatchRuntime, KeyTableRuntime,
};
use crate::cmd::queue::{self, CmdReturn, QueueBatch, QueueEvent, QueueStateFlags};
use crate::ids::{Arena, ArenaError, ClientId, KeyTableId, PaneId, QueueItemId, WindowId};
use crate::model::pane::{pane_is_visible, pane_key, pane_paste, pane_prompt_key};
use crate::model::resize::recalculate_window_size;
use crate::model::session::session_update_activity;
use crate::model::{PaneFlags, WindowSizePolicy};
use crate::server::Server;
use crate::server::event_loop::LoopAction;
use crate::server::events::fire_client;
use crate::server::operations::{server_destroy_pane, server_kill_pane, server_status_client};
use crate::ui::menu::{menu_close, menu_key};
use crate::ui::prompt::PromptKeyResult;
use crate::ui::status::{
    status_at_line, status_line_size, status_message_clear, status_prompt_key,
};
use rmux_emu::colour::ClientTheme;
use rmux_emu::input::ExtendedKeysFormat;
use rmux_emu::input::keys::KeyPolicy;
use rmux_tty::term::TtyCodeCode;
use rmux_tty::tty::TtyFlags;
use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, KeyFlags, KeyMasks, KeyModifiers, SpecialKey};
use rmux_util::log_debug;
use rmux_util::time::Timestamp;
use std::rc::Rc;
use std::time::Duration;

/// `gettimeofday` as the `(sec, usec)` pair the client and key tables store.
fn now() -> (i64, i64) {
    let t = Timestamp::now();
    (t.sec, i64::from(t.usec))
}

/// `timersub(a, b)`: normalised so that `0 <= usec < 1_000_000`.
fn timersub(a: (i64, i64), b: (i64, i64)) -> (i64, i64) {
    let mut sec = a.0 - b.0;
    let mut usec = a.1 - b.1;
    if usec < 0 {
        sec -= 1;
        usec += 1_000_000;
    }
    (sec, usec)
}

/// The name of a key table, or empty when the id is stale.
fn table_name(server: &Server, table: KeyTableId) -> &[u8] {
    server
        .key_bindings
        .tables
        .get(table)
        .map_or(b"", |t| t.name.as_ref())
}

/// `server_client_set_key_table` body on an explicit `KeyBindings`
/// (`server-client.c:118-122`), so `KeyTableRuntime` can run it while the
/// store is borrowed out of the server.
fn set_key_table_in(
    clients: &mut Arena<Client, ClientId>,
    bindings: &mut KeyBindings,
    id: ClientId,
    name: &[u8],
    now: (i64, i64),
) {
    let Some(c) = clients.get_mut(id) else {
        return;
    };
    if let Some(old) = c.keytable.take() {
        let _ = bindings.unref_table(old);
    }
    let table = bindings
        .get_table(name, true)
        .expect("key table arena")
        .expect("created key table");
    bindings.retain_table(table).expect("live key table");
    if let Some(t) = bindings.tables.get_mut(table) {
        t.activity_time = now;
    }
    c.keytable = Some(table);
}

/// `server_client_set_key_table` (`server-client.c:112-123`). `None` means
/// the client's default table.
pub fn set_key_table(server: &mut Server, id: ClientId, name: Option<&[u8]>) {
    let name = match name {
        Some(name) => name.to_vec(),
        None => get_key_table(server, id),
    };
    let now = now();
    let Server {
        clients,
        key_bindings,
        ..
    } = server;
    set_key_table_in(clients, key_bindings, id, &name, now);
}

/// `server_client_key_table_activity_diff` (`server-client.c:125-132`):
/// milliseconds from the table's activity time to the client's, with the C
/// unsigned wrap for a negative difference.
pub fn key_table_activity_diff(server: &Server, id: ClientId) -> u64 {
    let Some(c) = server.clients.get(id) else {
        return 0;
    };
    let table_time = c
        .keytable
        .and_then(|t| server.key_bindings.tables.get(t))
        .map_or((0, 0), |t| t.activity_time);
    activity_diff_ms(c.activity_time, table_time)
}

/// Pure body of `server_client_key_table_activity_diff`
/// (`server-client.c:130-131`).
fn activity_diff_ms(activity: (i64, i64), table_activity: (i64, i64)) -> u64 {
    let (sec, usec) = timersub(activity, table_activity);
    (sec as u64)
        .wrapping_mul(1000)
        .wrapping_add((usec as u64) / 1000)
}

/// `server_client_get_key_table` (`server-client.c:134-148`).
pub fn get_key_table(server: &Server, id: ClientId) -> Vec<u8> {
    let Some(s) = server.clients.get(id).and_then(|c| c.session) else {
        return b"root".to_vec();
    };
    let Some(session) = server.sessions.get(s) else {
        return b"root".to_vec();
    };
    let name = server.options.get_string(session.options, b"key-table");
    if name.is_empty() {
        return b"root".to_vec();
    }
    name.to_vec()
}

/// `server_client_is_default_key_table` (`server-client.c:150-155`) for an
/// arbitrary table.
fn is_default_table(server: &Server, id: ClientId, table: KeyTableId) -> bool {
    table_name(server, table) == get_key_table(server, id).as_slice()
}

/// `server_client_is_default_key_table(c, c->keytable)`.
pub fn is_default_key_table(server: &Server, id: ClientId) -> bool {
    server
        .clients
        .get(id)
        .and_then(|c| c.keytable)
        .is_some_and(|table| is_default_table(server, id, table))
}

/// `c->keytable`, created when the client has none yet
/// (`server-client.c:196`).
fn client_table(server: &mut Server, id: ClientId) -> Option<KeyTableId> {
    if let Some(table) = server.clients.get(id)?.keytable {
        return Some(table);
    }
    set_key_table(server, id, None);
    server.clients.get(id)?.keytable
}

/// Pure body of `server_client_is_bracket_paste` (`server-client.c:1194-1211`).
fn bracket_paste_step(
    flags: &mut ClientFlags,
    paste_time: &mut i64,
    key: KeyCode,
    current_time: i64,
) -> bool {
    let key0 = key.0 & KeyMasks::KEY;
    if key0 == SpecialKey::PASTE_START {
        flags.insert(ClientFlags::BRACKETPASTING);
        *paste_time = current_time;
        return false;
    }
    if key0 == SpecialKey::PASTE_END {
        flags.remove(ClientFlags::BRACKETPASTING);
        return false;
    }
    flags.intersects(ClientFlags::BRACKETPASTING)
}

/// `server_client_is_bracket_paste` (`server-client.c:1193-1211`).
fn is_bracket_paste(server: &mut Server, id: ClientId, key: KeyCode) -> bool {
    let current_time = server.current_time.0;
    let Some(c) = server.clients.get_mut(id) else {
        return false;
    };
    let before = c.flags;
    let result = bracket_paste_step(&mut c.flags, &mut c.paste_time, key, current_time);
    if before != c.flags {
        let on = c.flags.intersects(ClientFlags::BRACKETPASTING);
        log_debug!(
            "{}: bracket paste {}",
            ByteString::from(c.name_bytes()),
            if on { "on" } else { "off" }
        );
    }
    result
}

/// Pure body of `server_client_is_assume_paste` (`server-client.c:1214-1242`).
/// `activity`/`last_activity` are the client's two activity stamps.
fn assume_paste_step(
    flags: &mut ClientFlags,
    paste_time: &mut i64,
    assume_paste_time: i64,
    has_enbp: bool,
    activity: (i64, i64),
    last_activity: (i64, i64),
    current_time: i64,
) -> bool {
    if flags.intersects(ClientFlags::BRACKETPASTING) {
        return false;
    }
    if assume_paste_time == 0 {
        return false;
    }
    if has_enbp {
        return false;
    }
    let (sec, usec) = timersub(activity, last_activity);
    if sec == 0 && usec < assume_paste_time * 1000 {
        if flags.intersects(ClientFlags::ASSUMEPASTING) {
            return true;
        }
        flags.insert(ClientFlags::ASSUMEPASTING);
        *paste_time = current_time;
        return false;
    }
    if flags.intersects(ClientFlags::ASSUMEPASTING) {
        flags.remove(ClientFlags::ASSUMEPASTING);
    }
    false
}

/// `tty_term_has(c->tty.term, TTYC_ENBP)`; a client without an open
/// terminal has no capabilities.
fn tty_has_enbp(c: &Client) -> bool {
    c.tty.as_ref().is_some_and(|tty| {
        tty.flags().contains(TtyFlags::OPENED) && tty.term().has(TtyCodeCode::Enbp)
    })
}

/// `server_client_is_assume_paste` (`server-client.c:1213-1242`).
fn is_assume_paste(server: &mut Server, id: ClientId) -> bool {
    let current_time = server.current_time.0;
    let Some(c) = server.clients.get(id) else {
        return false;
    };
    let Some(s) = c.session.and_then(|s| server.sessions.get(s)) else {
        return false;
    };
    let assume_paste_time = server.options.get_number(s.options, b"assume-paste-time");
    let has_enbp = tty_has_enbp(c);
    let Some(c) = server.clients.get_mut(id) else {
        return false;
    };
    let before = c.flags;
    let result = assume_paste_step(
        &mut c.flags,
        &mut c.paste_time,
        assume_paste_time,
        has_enbp,
        c.activity_time,
        c.last_activity_time,
        current_time,
    );
    if before != c.flags {
        let on = c.flags.intersects(ClientFlags::ASSUMEPASTING);
        log_debug!(
            "{}: assume paste {}",
            ByteString::from(c.name_bytes()),
            if on { "on" } else { "off" }
        );
    }
    result
}

/// `server_client_update_latest` (`server-client.c:1244-1262`).
pub fn update_latest(server: &mut Server, id: ClientId) {
    let Some(s) = server.clients.get(id).and_then(|c| c.session) else {
        return;
    };
    let Some(w) = server
        .sessions
        .get(s)
        .and_then(|s| s.current)
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.window)
    else {
        return;
    };
    let Some(window) = server.windows.get_mut(w) else {
        return;
    };
    if window.latest == Some(id) {
        return;
    }
    window.latest = Some(id);
    let options = window.options;
    if server.options.get_number(options, b"window-size") == WindowSizePolicy::Latest as i64 {
        let clients = registry::resize_clients(server);
        let _ = recalculate_window_size(server, &clients, w, false);
    }
    fire_client(server, b"client-active", id);
}

/// Pure body of `server_client_repeat_time` (`server-client.c:1265-1282`):
/// `repeat` and `initial` are the `repeat-time` and `initial-repeat-time`
/// option values.
fn repeat_time_for(
    client_flags: ClientFlags,
    last_key: KeyCode,
    binding_key: KeyCode,
    binding_flags: KeyBindingFlags,
    repeat: i64,
    initial: i64,
) -> u32 {
    if !binding_flags.contains(KeyBindingFlags::REPEAT) {
        return 0;
    }
    let mut repeat = repeat as u32;
    if repeat == 0 {
        return 0;
    }
    if !client_flags.intersects(ClientFlags::REPEAT) || binding_key != last_key {
        let initial = initial as u32;
        if initial != 0 {
            repeat = initial;
        }
    }
    repeat
}

/// `server_client_repeat_time` (`server-client.c:1264-1282`).
fn repeat_time(
    server: &Server,
    id: ClientId,
    binding_key: KeyCode,
    binding_flags: KeyBindingFlags,
) -> u32 {
    let Some(c) = server.clients.get(id) else {
        return 0;
    };
    let Some(s) = c.session.and_then(|s| server.sessions.get(s)) else {
        return 0;
    };
    let repeat = server.options.get_number(s.options, b"repeat-time");
    let initial = server.options.get_number(s.options, b"initial-repeat-time");
    repeat_time_for(
        c.flags,
        c.last_key,
        binding_key,
        binding_flags,
        repeat,
        initial,
    )
}

/// `server_client_handle_dead_key` (`server-client.c:1284-1301`).
fn handle_dead_key(server: &mut Server, wp: Option<PaneId>, key: KeyCode) -> bool {
    let Some(wp) = wp else {
        return false;
    };
    let Some(pane) = server.panes.get(wp) else {
        return false;
    };
    if !pane.flags.contains(PaneFlags::EXITED) || key.is_mouse() || key.is_paste() {
        return false;
    }
    let options = pane.options;
    let remain_on_exit = server.options.get_number(options, b"remain-on-exit");
    if remain_on_exit != 3 && remain_on_exit != 4 {
        return false;
    }
    server
        .options
        .set_number_value(options, b"remain-on-exit", 0);
    let _ = server_destroy_pane(server, wp, false);
    true
}

/// `KEYC_IS_MOUSE(key)` plus the resolved target, as `cmd_find_from_mouse`
/// and queue states consume it (`struct mouse_event`, `tmux.h:1699-1720`).
pub fn mouse_input(ev: &KeyEvent) -> MouseInput {
    let t = &ev.target;
    let m = &ev.mouse;
    MouseInput {
        valid: t.valid,
        session: t.session,
        window: t.window,
        pane: t.pane,
        x: m.x,
        y: m.y,
        last_x: m.lx,
        last_y: m.ly,
        offset_x: t.ox,
        offset_y: t.oy,
        status_at: t.status_at,
        status_lines: t.status_lines,
    }
}

/// The `backspace` and `extended-keys-format` server options that
/// `window_pane_key` reads through `input_key` (`input-keys.c:467,593`).
pub fn key_policy(server: &Server) -> KeyPolicy {
    let global = server.options.global;
    KeyPolicy {
        backspace: KeyCode(server.options.get_number(global, b"backspace") as u64),
        format: if server.options.get_number(global, b"extended-keys-format") == 0 {
            ExtendedKeysFormat::CsiU
        } else {
            ExtendedKeysFormat::Xterm
        },
    }
}

/// `tty_sync_start(&c->tty)`.
fn tty_sync_start(server: &mut Server, id: ClientId) {
    let Server { clients, tparm, .. } = server;
    if let Some(tty) = clients.get_mut(id).and_then(|c| c.tty.as_mut()) {
        tty.sync_start(tparm);
    }
}

/// The mode key table of the active pane's first mode, when it has one
/// (`wme->mode->key_table(wme)`, `server-client.c:1407-1409`).
fn mode_key_table(server: &mut Server, wp: Option<PaneId>) -> Option<KeyTableId> {
    let wp = wp?;
    if server.panes.get(wp)?.modes.is_empty() {
        return None;
    }
    let name = crate::model::pane::pane_mode_key_table(server, wp)?;
    server.key_bindings.get_table(&name, true).ok().flatten()
}

/// `(key & (KEYC_MASK_KEY|KEYC_MASK_MODIFIERS))`.
const fn key_and_modifiers(key: KeyCode) -> KeyCode {
    KeyCode(key.0 & (KeyMasks::KEY | KeyMasks::MODIFIERS))
}

/// The prefix test of `table_changed` (`server-client.c:1421-1423`).
fn is_prefix_key(key0: KeyCode, prefix: KeyCode, prefix2: KeyCode) -> bool {
    key0 == key_and_modifiers(prefix) || key0 == key_and_modifiers(prefix2)
}

/// The mouse-move keys that never leave the prefix table
/// (`server-client.c:1529-1534`).
const fn is_mouse_move_key(key: KeyCode) -> bool {
    matches!(
        key.0,
        SpecialKey::MOUSEMOVE_PANE
            | SpecialKey::MOUSEMOVE_STATUS
            | SpecialKey::MOUSEMOVE_STATUS_LEFT
            | SpecialKey::MOUSEMOVE_STATUS_RIGHT
            | SpecialKey::MOUSEMOVE_STATUS_DEFAULT
            | SpecialKey::MOUSEMOVE_BORDER
    )
}

/// The five `goto` labels of `server_client_key_callback`
/// (`server-client.c:1414-1581`, spec 4.4).
enum KeyStep {
    TableChanged,
    TryAgain,
    Forward,
    Paste,
    Done,
}

/// Mutable state shared by the `KeyStep` arms.
struct KeyRun {
    id: ClientId,
    item: QueueItemId,
    ev: KeyEvent,
    key: KeyCode,
    key0: KeyCode,
    /// `table` and `first`; `None` only on the paths that skip table lookup.
    table: Option<KeyTableId>,
    first: Option<KeyTableId>,
    flags: ClientFlags,
    wp: Option<PaneId>,
    fs: CmdFindState,
}

/// Reads `c->flags`; a vanished client reads as no flags.
fn client_flags(server: &Server, id: ClientId) -> ClientFlags {
    server
        .clients
        .get(id)
        .map_or(ClientFlags::default(), |c| c.flags)
}

/// `server_client_set_key_table(c, NULL)` followed by `table = c->keytable`.
fn reset_table(server: &mut Server, id: ClientId) -> KeyTableId {
    set_key_table(server, id, None);
    server
        .clients
        .get(id)
        .and_then(|c| c.keytable)
        .expect("reset key table")
}

/// `table_changed:` (`server-client.c:1414-1429`).
fn step_table_changed(server: &mut Server, run: &mut KeyRun) -> KeyStep {
    let Some(session) = server
        .clients
        .get(run.id)
        .and_then(|c| c.session)
        .and_then(|s| server.sessions.get(s))
    else {
        return KeyStep::Done;
    };
    let prefix = KeyCode(server.options.get_number(session.options, b"prefix") as u64);
    let prefix2 = KeyCode(server.options.get_number(session.options, b"prefix2") as u64);
    run.key0 = key_and_modifiers(run.key);
    let Some(table) = run.table else {
        return KeyStep::Done;
    };
    if is_prefix_key(run.key0, prefix, prefix2) && table_name(server, table) != b"prefix" {
        set_key_table(server, run.id, Some(b"prefix"));
        server_status_client(server, run.id);
        return KeyStep::Done;
    }
    run.flags = client_flags(server, run.id);
    KeyStep::TryAgain
}

/// `try_again:` (`server-client.c:1431-1562`).
fn step_try_again(server: &mut Server, run: &mut KeyRun) -> KeyStep {
    let Some(table) = run.table else {
        return KeyStep::Done;
    };
    let name = ByteString::from(table_name(server, table));
    match run.wp.and_then(|wp| server.panes.get(wp)) {
        None => log_debug!("key table {} (no pane)", name),
        Some(pane) => log_debug!("key table {} (pane %{})", name, pane.public_id),
    }
    let repeating = client_flags(server, run.id).intersects(ClientFlags::REPEAT);
    if repeating {
        log_debug!("currently repeating");
    }

    let bd = server
        .key_bindings
        .get(table, run.key0)
        .map(|bd| (bd.key, bd.flags, bd.dispatch_snapshot()));
    let bd_repeats = bd
        .as_ref()
        .is_some_and(|(_, flags, _)| flags.contains(KeyBindingFlags::REPEAT));

    // Prefix timeout (server-client.c:1446-1465).
    let prefix_delay = server
        .options
        .get_number(server.options.global, b"prefix-timeout");
    if prefix_delay > 0
        && table_name(server, table) == b"prefix"
        && key_table_activity_diff(server, run.id) > prefix_delay as u64
    {
        if bd.is_some() && repeating && bd_repeats {
            log_debug!("prefix timeout ignored, repeat is active");
        } else {
            log_debug!("prefix timeout exceeded");
            run.table = Some(reset_table(server, run.id));
            run.first = run.table;
            server_status_client(server, run.id);
            return KeyStep::TableChanged;
        }
    }

    if let Some((bd_key, bd_flags, snapshot)) = bd {
        // Repeating but a non-repeating binding (server-client.c:1474-1483).
        if repeating && !bd_repeats {
            log_debug!("found in key table {} (not repeating)", name);
            run.table = Some(reset_table(server, run.id));
            run.first = run.table;
            if let Some(c) = server.clients.get_mut(run.id) {
                c.flags.remove(ClientFlags::REPEAT);
            }
            server_status_client(server, run.id);
            return KeyStep::TableChanged;
        }
        log_debug!("found in key table {}", name);

        // Hold the table while the binding runs (server-client.c:1490).
        let _ = server.key_bindings.retain_table(table);

        let repeat = repeat_time(server, run.id, bd_key, bd_flags);
        if repeat != 0 {
            if let Some(c) = server.clients.get_mut(run.id) {
                c.flags.insert(ClientFlags::REPEAT);
                c.last_key = bd_key;
            }
            let timer = server.event_loop.schedule(
                Duration::from_millis(u64::from(repeat)),
                LoopAction::ClientRepeatTimer(run.id),
            );
            if let Some(old) = server
                .clients
                .get_mut(run.id)
                .and_then(|c| c.repeat_timer.replace(timer))
            {
                server.event_loop.cancel(old);
            }
        } else {
            if let Some(c) = server.clients.get_mut(run.id) {
                c.flags.remove(ClientFlags::REPEAT);
            }
            set_key_table(server, run.id, None);
        }
        server_status_client(server, run.id);

        // Execute the key binding (server-client.c:1512-1513).
        let event = QueueEvent {
            key: run.ev.key,
            mouse: mouse_input(&run.ev),
        };
        key_bindings::dispatch(
            server,
            snapshot,
            Some(run.item),
            Some(run.id),
            Some(&event),
            &run.fs,
        );
        let _ = server.key_bindings.unref_table(table);
        return KeyStep::Done;
    }

    // No match, try the ANY key (server-client.c:1520-1523).
    if run.key0.0 != SpecialKey::ANY {
        run.key0 = KeyCode(SpecialKey::ANY);
        return KeyStep::TryAgain;
    }

    // Movement keys never leave the prefix table (server-client.c:1529-1535).
    if is_mouse_move_key(run.key) {
        return KeyStep::Forward;
    }

    // Not in the root table, or repeating (server-client.c:1541-1552).
    log_debug!("not found in key table {}", name);
    if !is_default_table(server, run.id, table) || repeating {
        log_debug!("trying in root table");
        run.table = Some(reset_table(server, run.id));
        if repeating {
            run.first = run.table;
        }
        if let Some(c) = server.clients.get_mut(run.id) {
            c.flags.remove(ClientFlags::REPEAT);
        }
        server_status_client(server, run.id);
        return KeyStep::TableChanged;
    }

    // Not the first table tried (server-client.c:1558-1562).
    if run.first != run.table && !run.flags.intersects(ClientFlags::REPEAT) {
        set_key_table(server, run.id, None);
        server_status_client(server, run.id);
        return KeyStep::Done;
    }
    KeyStep::Forward
}

/// `forward_key:` (`server-client.c:1564-1571`).
fn step_forward(server: &mut Server, run: &mut KeyRun) -> KeyStep {
    if handle_dead_key(server, run.wp, run.key) {
        return KeyStep::Done;
    }
    if client_flags(server, run.id).intersects(ClientFlags::READONLY) {
        return KeyStep::Done;
    }
    if let Some(wp) = run.wp {
        let policy = key_policy(server);
        let _ = pane_key(
            server,
            wp,
            Some(run.id),
            run.key,
            Some(&run.ev.mouse),
            &policy,
        );
    }
    KeyStep::Done
}

/// `paste_key:` (`server-client.c:1573-1579`).
fn step_paste(server: &mut Server, run: &mut KeyRun) -> KeyStep {
    if client_flags(server, run.id).intersects(ClientFlags::READONLY) {
        return KeyStep::Done;
    }
    if let (Some(wp), Some(buf)) = (run.wp, run.ev.paste.as_deref()) {
        let _ = pane_paste(server, wp, run.key, buf);
    }
    run.key = KeyCode(SpecialKey::NONE);
    KeyStep::Done
}

/// `out:` (`server-client.c:1581-1588`). `key` is the final key value;
/// `attached` is whether the session check at entry passed.
fn key_out(server: &mut Server, id: ClientId, ec: Option<ClientId>, attached: bool, key: KeyCode) {
    if attached && key.0 != SpecialKey::FOCUS_OUT {
        update_latest(server, id);
    }
    if let Some(ec) = ec {
        let _ = lifecycle::release(server, ec);
    }
}

/// `server_client_key_callback` (`server-client.c:1303-1589`). Owns the
/// event; `item` is the queue item running the callback.
pub fn key_callback(server: &mut Server, item: QueueItemId, mut ev: KeyEvent) {
    let ec = ev.client;
    let id = match ec.or_else(|| server.queue.items.get(item).and_then(|i| i.client)) {
        Some(id) => id,
        None => {
            if let Some(ec) = ec {
                let _ = lifecycle::release(server, ec);
            }
            return;
        }
    };
    let mut key = ev.key;

    // Check the client is good to accept input (server-client.c:1332-1335).
    let Some(s) = server
        .clients
        .get(id)
        .filter(|c| !c.flags.intersects(ClientFlags::UNATTACHEDFLAGS))
        .and_then(|c| c.session)
    else {
        return key_out(server, id, ec, false, key);
    };

    // Update the activity timer (server-client.c:1337-1342).
    let now = now();
    if let Some(c) = server.clients.get_mut(id) {
        c.last_activity_time = c.activity_time;
        c.activity_time = now;
    }
    session_update_activity(server, s, Some(now));

    // Check for mouse keys (server-client.c:1344-1366).
    ev.target.valid = false;
    if key.0 == SpecialKey::MOUSE || key.0 == SpecialKey::DOUBLECLICK {
        if client_flags(server, id).intersects(ClientFlags::READONLY) {
            return key_out(server, id, ec, true, key);
        }
        key = crate::client::mouse::check_mouse(server, id, &mut ev);
        if key.0 == SpecialKey::UNKNOWN {
            return key_out(server, id, ec, true, key);
        }
        ev.target.valid = true;
        ev.target.key = key;

        // Synchronize direct drag output with the later damage redraw
        // before invoking the drag callback (server-client.c:1360-1364).
        if key.0 & KeyMasks::KEY == SpecialKey::DRAGGING {
            tty_sync_start(server, id);
            if let Some(action) = server.clients.get(id).and_then(|c| c.drag.update) {
                action.update(server, id, &ev.resolved());
            }
            return key_out(server, id, ec, true, key);
        }
        ev.key = key;
    }

    // Find affected pane (server-client.c:1368-1371).
    let mut fs = None;
    if key.is_mouse() {
        fs = find::from_mouse(server, &mouse_input(&ev), CmdFindFlags::default());
    }
    let fs = fs
        .or_else(|| find::from_client(server, Some(id), CmdFindFlags::default()))
        .unwrap_or_default();
    let wp = fs.wp;

    let mouse_enabled = server
        .sessions
        .get(s)
        .is_some_and(|s| server.options.get_number(s.options, b"mouse") != 0);

    let mut run = KeyRun {
        id,
        item,
        key,
        key0: key_and_modifiers(key),
        table: None,
        first: None,
        flags: ClientFlags::default(),
        wp,
        fs,
        ev,
    };

    let mut step = 'entry: {
        // Forward mouse keys if disabled (server-client.c:1373-1375).
        if key.is_mouse() && !mouse_enabled {
            break 'entry KeyStep::Forward;
        }
        // Forward if bracket pasting (server-client.c:1377-1379).
        if is_bracket_paste(server, id, key) {
            break 'entry KeyStep::Paste;
        }
        // Treat everything as a regular key when pasting is detected
        // (server-client.c:1381-1387).
        if !key.is_mouse()
            && key.0 != SpecialKey::FOCUS_IN
            && key.0 != SpecialKey::FOCUS_OUT
            && key.0 & KeyFlags::SENT.0 == 0
            && is_assume_paste(server, id)
        {
            break 'entry KeyStep::Paste;
        }
        // Forward keys directly if this pane is capturing all keys
        // (server-client.c:1389-1395).
        if let Some(pane) = wp.and_then(|wp| server.panes.get(wp)) {
            if pane.flags.contains(PaneFlags::CAPTUREALLKEYS)
                && !pane.flags.contains(PaneFlags::EXITED)
                && !key.is_mouse()
                && pane.modes.is_empty()
            {
                break 'entry KeyStep::Forward;
            }
        }
        // Focus events are not keys and cannot be bound
        // (server-client.c:1397-1399).
        if key.0 == SpecialKey::FOCUS_IN || key.0 == SpecialKey::FOCUS_OUT {
            break 'entry KeyStep::Forward;
        }

        // Work out the current key table (server-client.c:1401-1412).
        let Some(client_table) = client_table(server, id) else {
            break 'entry KeyStep::Done;
        };
        let table = if is_default_key_table(server, id) {
            mode_key_table(server, wp).unwrap_or(client_table)
        } else {
            client_table
        };
        run.table = Some(table);
        run.first = Some(table);
        KeyStep::TableChanged
    };

    loop {
        step = match step {
            KeyStep::TableChanged => step_table_changed(server, &mut run),
            KeyStep::TryAgain => step_try_again(server, &mut run),
            KeyStep::Forward => step_forward(server, &mut run),
            KeyStep::Paste => step_paste(server, &mut run),
            KeyStep::Done => break,
        };
    }
    key_out(server, id, ec, true, run.key);
}

/// `server_client_handle_menu_key` (`server-client.c:1591-1625`).
fn handle_menu_key(server: &mut Server, id: ClientId, ev: &KeyEvent) -> bool {
    let Some(w) = current_window(server, id) else {
        return false;
    };
    if server.windows.get(w).is_none_or(|w| w.menu.is_none()) {
        return false;
    }

    let mut new_event = ev.clone();
    if ev.key.is_mouse() {
        let status_at = status_at_line(server, id);
        new_event.target.status_at = status_at;
        new_event.target.status_lines = status_line_size(server, id);
        let (_, ox, oy, _, _) = server
            .clients
            .get(id)
            .and_then(|c| c.tty.as_ref())
            .map_or((false, 0, 0, 0, 0), |tty| tty.window_offset());
        let m = &mut new_event.mouse;
        let status_lines = new_event.target.status_lines;
        m.x = m.x.wrapping_add(ox);
        if status_at == 0 {
            if m.y < status_lines {
                m.x = u32::MAX;
                m.y = u32::MAX;
            } else {
                m.y = (m.y - status_lines).wrapping_add(oy);
            }
        } else if status_at > 0 && m.y >= status_at as u32 {
            m.x = u32::MAX;
            m.y = u32::MAX;
        } else {
            m.y = m.y.wrapping_add(oy);
        }
    }

    if menu_key(server, id, w, &new_event) {
        menu_close(server, w);
    }
    true
}

/// `c->session->curw->window`.
fn current_window(server: &Server, id: ClientId) -> Option<WindowId> {
    let s = server.clients.get(id)?.session?;
    let wl = server.sessions.get(s)?.current?;
    Some(server.winlinks.get(wl)?.window)
}

/// `s->curw->window->active`.
fn active_pane(server: &Server, id: ClientId) -> Option<PaneId> {
    server.windows.get(current_window(server, id)?)?.active
}

/// `window_pane_has_prompt(wp)`.
fn pane_has_prompt(server: &Server, wp: PaneId) -> bool {
    server.panes.get(wp).is_some_and(|p| p.prompt.is_some())
}

/// `server_client_handle_key0` (`server-client.c:1627-1735`). Returns the
/// queued item (`*next`) when the event was queued, else `None` and the
/// event is dropped.
fn handle_key0(
    server: &mut Server,
    id: ClientId,
    mut ev: KeyEvent,
    after: Option<QueueItemId>,
) -> Option<QueueItemId> {
    // Check the client is good to accept input (server-client.c:1636-1638).
    let c = server.clients.get(id)?;
    if c.session.is_none() || c.flags.intersects(ClientFlags::UNATTACHEDFLAGS) {
        return None;
    }
    let read_only = c.flags.intersects(ClientFlags::READONLY);
    let message = c.message.text.is_some();
    let message_ignore_keys = c.message.ignore_keys;

    if ev.key.0 == SpecialKey::REPORT_LIGHT_THEME {
        crate::client::theme::report_theme(server, id, ClientTheme::Light);
        return None;
    }
    if ev.key.0 == SpecialKey::REPORT_DARK_THEME {
        crate::client::theme::report_theme(server, id, ClientTheme::Dark);
        return None;
    }

    // Dead panes waiting for a key, modal cancel keys, panes capturing all
    // keys and the command prompt are special cases. The queue might be
    // blocked so they need to be processed immediately rather than queued
    // (server-client.c:1649-1718).
    if !read_only {
        if message {
            if message_ignore_keys {
                return None;
            }
            status_message_clear(server, id);
        }

        let wp = active_pane(server, id);
        if handle_dead_key(server, wp, ev.key) {
            return None;
        }
        if let Some((wp, window, flags, no_modes)) = wp.and_then(|wp| {
            server
                .panes
                .get(wp)
                .map(|p| (wp, p.window, p.flags, p.modes.is_empty()))
        }) {
            let modal = server.windows.get(window).and_then(|w| w.modal);
            if modal == Some(wp)
                && flags.contains(PaneFlags::CLOSEONCANCEL)
                && (ev.key.0 == 0x1b || ev.key.0 == u64::from(b'c') | KeyModifiers::CTRL.0)
            {
                let _ = server_kill_pane(server, wp);
                return None;
            }
            if flags.contains(PaneFlags::CAPTUREALLKEYS)
                && no_modes
                && !ev.key.is_mouse()
                && !flags.contains(PaneFlags::EXITED)
            {
                let policy = key_policy(server);
                let _ = pane_key(server, wp, Some(id), ev.key, Some(&ev.mouse), &policy);
                return None;
            }
        }

        if handle_menu_key(server, id, &ev) {
            return None;
        }
        if server.clients.get(id).is_some_and(|c| c.prompt.is_some()) {
            match status_prompt_key(server, id, ev.key, Some(&ev.mouse)) {
                PromptKeyResult::Handled | PromptKeyResult::Close => return None,
                PromptKeyResult::NotHandled | PromptKeyResult::Move => {}
            }
        }

        // The active pane first, else the first visible pane with a prompt
        // (server-client.c:1695-1702).
        let mut wp = active_pane(server, id).filter(|wp| pane_has_prompt(server, *wp));
        if wp.is_none() {
            if let Some(w) = current_window(server, id) {
                let panes = server
                    .windows
                    .get(w)
                    .map(|w| w.panes.clone())
                    .unwrap_or_default();
                wp = panes
                    .into_iter()
                    .find(|wp| pane_has_prompt(server, *wp) && pane_is_visible(server, *wp));
            }
        }
        if let Some(wp) =
            wp.filter(|wp| pane_has_prompt(server, *wp) && pane_is_visible(server, *wp))
        {
            let status_at_top = status_at_line(server, id) == 0;
            match pane_prompt_key(server, wp, id, ev.key, Some(&ev.mouse), status_at_top) {
                Ok(true) => return None,
                Ok(false) | Err(_) => {
                    if ev.key.is_mouse() {
                        return None;
                    }
                }
            }
        }
    }

    // Add the key to the queue so it happens after any commands queued by
    // previous keys (server-client.c:1720-1734).
    if let Some(after) = after {
        ev.client = Some(id);
        lifecycle::retain(server, id).ok()?;
        let queued = key_callback_batch(server, ev)
            .and_then(|batch| queue::insert_after(server, after, batch).ok());
        if queued.is_none() {
            let _ = lifecycle::release(server, id);
        }
        return queued;
    }
    let batch = key_callback_batch(server, ev)?;
    queue::append(server, Some(id), batch).ok()
}

/// `cmdq_get_callback(server_client_key_callback, event)`.
fn key_callback_batch(server: &mut Server, ev: KeyEvent) -> Option<QueueBatch> {
    server
        .queue
        .get_callback(
            "server_client_key_callback",
            queue::callback_for::<Server>(move |server, item| {
                key_callback(server, item, ev);
                CmdReturn::Normal
            }),
        )
        .ok()
}

/// `server_client_handle_key` (`server-client.c:1737-1742`): true when the
/// event was queued.
pub fn handle_key(server: &mut Server, id: ClientId, ev: KeyEvent) -> bool {
    handle_key0(server, id, ev, None).is_some()
}

/// `server_client_handle_key_after` (`server-client.c:1744-1750`): the item
/// inserted after `after`, or `None` when the event was dropped.
pub fn handle_key_after(
    server: &mut Server,
    id: ClientId,
    ev: KeyEvent,
    after: QueueItemId,
) -> Option<QueueItemId> {
    handle_key0(server, id, ev, Some(after))
}

/// `server_client_repeat_timer` (`server-client.c:2180-2191`).
pub fn repeat_timer(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    c.repeat_timer = None;
    if c.flags.intersects(ClientFlags::REPEAT) {
        set_key_table(server, id, None);
        if let Some(c) = server.clients.get_mut(id) {
            c.flags.remove(ClientFlags::REPEAT);
        }
        server_status_client(server, id);
    }
}

/// `key_bindings_remove_table` client reset loop (`key-bindings.c`): every
/// client on the removed table goes back to its default table.
impl KeyTableRuntime for Server {
    fn reset_clients_using_table(
        &mut self,
        bindings: &mut KeyBindings,
        table: KeyTableId,
    ) -> Result<(), ArenaError> {
        let now = now();
        for id in self.client_order.clone() {
            if self
                .clients
                .get(id)
                .is_some_and(|c| c.keytable == Some(table))
            {
                let name = get_key_table(self, id);
                set_key_table_in(&mut self.clients, bindings, id, &name, now);
            }
        }
        Ok(())
    }
}

/// `key_bindings_dispatch` services (`key-bindings.c`): read-only check, the
/// command batch with a fresh state, and queue placement.
impl KeyDispatchRuntime for Server {
    fn client_read_only(&self, client: ClientId) -> bool {
        client_flags(self, client).intersects(ClientFlags::READONLY)
    }
    fn binding_commands(
        &mut self,
        list: Rc<CommandList>,
        current: &CmdFindState,
        event: Option<&QueueEvent>,
        flags: QueueStateFlags,
    ) -> QueueBatch {
        let mut store = std::mem::take(&mut self.queue);
        let state = store.new_state(self, Some(current), event, flags);
        self.queue = store;
        let state = state.expect("queue state arena");
        let batch = self
            .queue
            .get_command(list, Some(state))
            .expect("key binding command arena");
        self.queue.free_state(state).expect("key binding state");
        batch
    }
    fn binding_read_only(&mut self) -> QueueBatch {
        self.queue
            .get_callback(
                "key_bindings_read_only",
                queue::callback_for::<Server>(|server, item| key_bindings::read_only(server, item)),
            )
            .expect("read-only callback arena")
    }
    fn append_binding(
        &mut self,
        client: Option<ClientId>,
        batch: QueueBatch,
    ) -> Option<QueueItemId> {
        queue::append(self, client, batch).ok()
    }
    fn insert_binding(&mut self, item: QueueItemId, batch: QueueBatch) -> Option<QueueItemId> {
        queue::insert_after(self, item, batch).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;

    const CTRL: u64 = KeyModifiers::CTRL.0;

    fn client(server: &mut Server) -> ClientId {
        let id = server.clients.insert(Client::new(None, (100, 0))).unwrap();
        server.clients.retain(id).unwrap();
        server.client_order.push_back(id);
        id
    }

    // server-client.c:1194-1211
    #[test]
    fn bracket_paste_delimiters_toggle_flag_and_return_false() {
        let mut flags = ClientFlags::default();
        let mut paste_time = 0;
        assert!(!bracket_paste_step(
            &mut flags,
            &mut paste_time,
            KeyCode(u64::from(b'a')),
            10
        ));
        assert!(!bracket_paste_step(
            &mut flags,
            &mut paste_time,
            KeyCode(SpecialKey::PASTE_START),
            11
        ));
        assert!(flags.intersects(ClientFlags::BRACKETPASTING));
        assert_eq!(paste_time, 11);
        assert!(bracket_paste_step(
            &mut flags,
            &mut paste_time,
            KeyCode(u64::from(b'a')),
            12
        ));
        assert_eq!(paste_time, 11);
        // Delimiter with modifier/flag bits still matches on KEYC_MASK_KEY.
        assert!(!bracket_paste_step(
            &mut flags,
            &mut paste_time,
            KeyCode(SpecialKey::PASTE_END | CTRL),
            13
        ));
        assert!(!flags.intersects(ClientFlags::BRACKETPASTING));
        assert!(!bracket_paste_step(
            &mut flags,
            &mut paste_time,
            KeyCode(u64::from(b'a')),
            14
        ));
    }

    // server-client.c:1214-1242
    #[test]
    fn assume_paste_first_fast_key_arms_later_fast_keys_return_true() {
        let mut flags = ClientFlags::default();
        let mut paste_time = 0;
        // First fast key: arm but return false.
        assert!(!assume_paste_step(
            &mut flags,
            &mut paste_time,
            1,
            false,
            (5, 500),
            (5, 0),
            7
        ));
        assert!(flags.intersects(ClientFlags::ASSUMEPASTING));
        assert_eq!(paste_time, 7);
        // Second fast key: paste.
        assert!(assume_paste_step(
            &mut flags,
            &mut paste_time,
            1,
            false,
            (5, 900),
            (5, 500),
            8
        ));
        assert_eq!(paste_time, 7);
        // Exactly the limit is not fast (usec < t * 1000).
        assert!(!assume_paste_step(
            &mut flags,
            &mut paste_time,
            1,
            false,
            (6, 1000),
            (6, 0),
            9
        ));
        assert!(!flags.intersects(ClientFlags::ASSUMEPASTING));
        // A slow key with a whole second elapsed never arms.
        assert!(!assume_paste_step(
            &mut flags,
            &mut paste_time,
            1,
            false,
            (7, 0),
            (5, 999_999),
            9
        ));
        assert!(!flags.intersects(ClientFlags::ASSUMEPASTING));
        // Borrow across the second boundary: 5.999_900 -> 6.000_100 is 200us.
        assert!(!assume_paste_step(
            &mut flags,
            &mut paste_time,
            1,
            false,
            (6, 100),
            (5, 999_900),
            9
        ));
        assert!(flags.intersects(ClientFlags::ASSUMEPASTING));
    }

    // server-client.c:1221-1226
    #[test]
    fn assume_paste_disabled_by_bracket_option_or_enbp() {
        let mut paste_time = 0;
        let mut flags = ClientFlags::BRACKETPASTING | ClientFlags::ASSUMEPASTING;
        assert!(!assume_paste_step(
            &mut flags,
            &mut paste_time,
            1,
            false,
            (5, 100),
            (5, 0),
            1
        ));
        assert!(
            flags.intersects(ClientFlags::ASSUMEPASTING),
            "bracket pasting returns before touching the flag"
        );
        let mut flags = ClientFlags::ASSUMEPASTING;
        assert!(!assume_paste_step(
            &mut flags,
            &mut paste_time,
            0,
            false,
            (5, 100),
            (5, 0),
            1
        ));
        assert!(flags.intersects(ClientFlags::ASSUMEPASTING));
        assert!(!assume_paste_step(
            &mut flags,
            &mut paste_time,
            1,
            true,
            (5, 100),
            (5, 0),
            1
        ));
        assert!(flags.intersects(ClientFlags::ASSUMEPASTING));
        assert_eq!(paste_time, 0);
    }

    // server-client.c:1265-1282
    #[test]
    fn repeat_time_selects_initial_for_first_key_and_repeat_after() {
        let a = KeyCode(u64::from(b'a'));
        let b = KeyCode(u64::from(b'b'));
        let none = KeyBindingFlags::default();
        let repeat = KeyBindingFlags::REPEAT;
        assert_eq!(
            repeat_time_for(ClientFlags::REPEAT, a, a, none, 500, 300),
            0
        );
        assert_eq!(
            repeat_time_for(ClientFlags::REPEAT, a, a, repeat, 0, 300),
            0
        );
        // Not repeating: initial wins when nonzero.
        assert_eq!(
            repeat_time_for(ClientFlags::default(), a, a, repeat, 500, 300),
            300
        );
        assert_eq!(
            repeat_time_for(ClientFlags::default(), a, a, repeat, 500, 0),
            500
        );
        // Repeating the same key: repeat-time.
        assert_eq!(
            repeat_time_for(ClientFlags::REPEAT, a, a, repeat, 500, 300),
            500
        );
        // Repeating but a different key: initial again.
        assert_eq!(
            repeat_time_for(ClientFlags::REPEAT, a, b, repeat, 500, 300),
            300
        );
    }

    // server-client.c:125-132
    #[test]
    fn activity_diff_is_milliseconds_with_unsigned_wrap() {
        assert_eq!(activity_diff_ms((10, 500_000), (9, 250_000)), 1250);
        assert_eq!(activity_diff_ms((10, 0), (9, 999_000)), 1);
        assert_eq!(activity_diff_ms((10, 0), (10, 0)), 0);
        // Table stamped after the activity: C computes a huge unsigned value,
        // which exceeds any prefix-timeout.
        assert!(activity_diff_ms((9, 0), (10, 0)) > u64::from(u32::MAX));
    }

    // server-client.c:1419-1424, 1529-1534
    #[test]
    fn prefix_and_mouse_move_predicates() {
        let prefix = KeyCode(u64::from(b'b') | CTRL);
        let prefix2 = KeyCode(SpecialKey::NONE);
        assert!(is_prefix_key(
            key_and_modifiers(KeyCode(u64::from(b'b') | CTRL | KeyFlags::SENT.0)),
            prefix,
            prefix2
        ));
        assert!(!is_prefix_key(
            key_and_modifiers(KeyCode(u64::from(b'b'))),
            prefix,
            prefix2
        ));
        assert!(is_prefix_key(
            key_and_modifiers(KeyCode(SpecialKey::NONE)),
            prefix,
            prefix2
        ));
        assert!(is_mouse_move_key(KeyCode(SpecialKey::MOUSEMOVE_BORDER)));
        assert!(!is_mouse_move_key(KeyCode(
            SpecialKey::MOUSEMOVE_PANE | CTRL
        )));
    }

    // server-client.c:112-123, 134-148, 150-155
    #[test]
    fn key_table_lease_and_default_name_without_session() {
        let mut server = Server::new();
        let id = client(&mut server);
        assert_eq!(get_key_table(&server, id), b"root");
        assert!(!is_default_key_table(&server, id), "no table yet");

        set_key_table(&mut server, id, None);
        let root = server.clients.get(id).unwrap().keytable.unwrap();
        assert_eq!(table_name(&server, root), b"root");
        assert!(is_default_key_table(&server, id));
        assert_ne!(
            server.key_bindings.tables.get(root).unwrap().activity_time,
            (0, 0)
        );

        set_key_table(&mut server, id, Some(b"prefix"));
        let prefix = server.clients.get(id).unwrap().keytable.unwrap();
        assert_ne!(prefix, root);
        assert!(!is_default_key_table(&server, id));
        // The root table keeps its index lease after the client left it.
        assert!(server.key_bindings.find_table(b"root").is_some());
        assert!(server.key_bindings.tables.get(root).is_some());

        // Dropping the index lease (key_bindings_remove_table without the
        // client loop) while the client still holds the prefix table keeps
        // it addressable; the client's unref removes it.
        let mut bindings = std::mem::take(&mut server.key_bindings);
        bindings.index.remove(&ByteString::from(&b"prefix"[..]));
        bindings.tables.request_remove(prefix).unwrap();
        bindings.unref_table(prefix).unwrap();
        assert!(bindings.tables.get(prefix).is_some());
        server.key_bindings = bindings;
        set_key_table(&mut server, id, None);
        assert!(server.key_bindings.tables.get(prefix).is_none());
        assert!(is_default_key_table(&server, id));
    }

    // key-bindings.c remove_table loop through KeyTableRuntime
    #[test]
    fn reset_clients_using_table_moves_clients_back_to_default() {
        let mut server = Server::new();
        let a = client(&mut server);
        let b = client(&mut server);
        set_key_table(&mut server, a, Some(b"copy-mode"));
        set_key_table(&mut server, b, Some(b"prefix"));
        let copy = server.clients.get(a).unwrap().keytable.unwrap();
        let prefix = server.clients.get(b).unwrap().keytable.unwrap();

        let mut bindings = std::mem::take(&mut server.key_bindings);
        bindings.remove_table(&mut server, b"copy-mode").unwrap();
        server.key_bindings = bindings;

        assert!(server.key_bindings.tables.get(copy).is_none());
        assert_eq!(
            table_name(&server, server.clients.get(a).unwrap().keytable.unwrap()),
            b"root"
        );
        assert_eq!(server.clients.get(b).unwrap().keytable, Some(prefix));
    }

    // server-client.c:2181-2191
    #[test]
    fn repeat_timer_resets_table_only_while_repeating() {
        let mut server = Server::new();
        let id = client(&mut server);
        set_key_table(&mut server, id, Some(b"prefix"));
        repeat_timer(&mut server, id);
        assert!(
            !is_default_key_table(&server, id),
            "not repeating: table untouched"
        );

        server
            .clients
            .get_mut(id)
            .unwrap()
            .flags
            .insert(ClientFlags::REPEAT);
        repeat_timer(&mut server, id);
        let c = server.clients.get(id).unwrap();
        assert!(!c.flags.intersects(ClientFlags::REPEAT));
        assert!(c.flags.intersects(ClientFlags::REDRAWSTATUS));
        assert!(is_default_key_table(&server, id));
    }

    // server-client.c:1636-1638: no session drops the key before queueing.
    #[test]
    fn handle_key_without_session_drops_event() {
        let mut server = Server::new();
        let id = client(&mut server);
        assert!(!handle_key(
            &mut server,
            id,
            KeyEvent::new(KeyCode(u64::from(b'a')))
        ));
        assert!(
            server
                .queue
                .queue(Some(id))
                .is_none_or(|q| q.head.is_none())
        );
        let item = QueueItemId::from_parts(7, 0);
        assert_eq!(
            handle_key_after(
                &mut server,
                id,
                KeyEvent::new(KeyCode(u64::from(b'a'))),
                item
            ),
            None
        );
    }

    // server-client.c:1601-1619
    #[test]
    fn mouse_input_copies_target_and_geometry() {
        let mut ev = KeyEvent::new(KeyCode(SpecialKey::MOUSE));
        ev.mouse.x = 3;
        ev.mouse.y = 4;
        ev.mouse.lx = 1;
        ev.mouse.ly = 2;
        ev.target.valid = true;
        ev.target.ox = 10;
        ev.target.oy = 20;
        ev.target.status_at = -1;
        ev.target.status_lines = 1;
        let m = mouse_input(&ev);
        assert_eq!((m.x, m.y, m.last_x, m.last_y), (3, 4, 1, 2));
        assert_eq!(
            (m.offset_x, m.offset_y, m.status_at, m.status_lines),
            (10, 20, -1, 1)
        );
        assert!(m.valid && m.session.is_none());
    }
}
