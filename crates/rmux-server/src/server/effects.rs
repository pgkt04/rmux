// Ported from tmux options.c, session.c, window.c, input.c, alerts.c, names.c, resize.c, spawn.c @ 8f25579c
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

//! Model effects compose the live server without replaying named events. The
//! model's event callback already delivers those synchronously, while the
//! object and its pre-change metadata are still available to event sinks.

use super::{event_loop::LoopAction, events, operations, pane_runtime};
use crate::{
    client::ClientFlags,
    cmd::queue,
    ids::{ClientId, PaneId, SessionId, WindowId},
    model::{
        self, ModelEffect, PaneFlags, Server, WindowFlags,
        alerts::{AlertEffect, AlertKind, visual_delivery},
        pane::PaneEffect,
        pane_input::{InputAction, InputTimer, OwnedInputEffect, PaneStdinInput, StdinReadState},
        resize::{ResizeClient, ResizeEffect},
        session::{SessionEffect, SessionTimerRequest},
        spawn::SpawnEffect,
        state::TimerRequest,
        store_runtime::StoreEffect,
        window::WindowEffect,
    },
    options::{OptionsArrayKey, push::OptionsChange},
};
use rmux_emu::{cell::DEFAULT_CELL, colour::Colour};
use rmux_tty::tty::TtyFlags;
use std::{io, time::Duration};

fn model_error(error: model::ModelError) -> io::Error {
    io::Error::other(error)
}
fn clients(server: &Server) -> Vec<ClientId> {
    server.client_order.iter().copied().collect()
}
fn panes(server: &Server) -> Vec<PaneId> {
    server.pane_ids.values().copied().collect()
}
fn windows(server: &Server) -> Vec<WindowId> {
    server.window_ids.values().copied().collect()
}
fn sessions(server: &Server) -> Vec<SessionId> {
    server.session_names.values().copied().collect()
}
fn pane_timer_key(pane: PaneId, timer: InputTimer) -> Vec<u8> {
    format!("input:{pane:?}:{timer:?}").into_bytes()
}
fn session_timer_key(session: SessionId, free: bool) -> Vec<u8> {
    format!("session:{session:?}:{}", if free { "free" } else { "lock" }).into_bytes()
}
fn window_timer_key(window: WindowId, name: bool) -> Vec<u8> {
    format!(
        "window:{window:?}:{}",
        if name { "name" } else { "silence" }
    )
    .into_bytes()
}
fn scrollbar_timer_key(pane: PaneId) -> Vec<u8> {
    format!("scrollbar:{pane:?}").into_bytes()
}

fn cancel_timer(server: &mut Server, key: &[u8]) {
    if let Some(timer) = server.runtime_timers.remove(key) {
        server.event_loop.cancel(timer);
    }
}
fn set_timer(server: &mut Server, key: Vec<u8>, after: Option<Duration>, action: LoopAction) {
    cancel_timer(server, &key);
    if let Some(after) = after {
        let timer = server.event_loop.schedule(after, action);
        server.runtime_timers.insert(key, timer);
    }
}
fn window_offset(server: &mut Server, window: WindowId) {
    for client in clients(server) {
        if server
            .clients
            .get(client)
            .and_then(|c| c.session)
            .is_some_and(|s| model::session::session_has(server, s, window))
        {
            crate::client::lifecycle::update_offset(server, client);
        }
    }
}
fn invalidate_scene(server: &mut Server, window: WindowId) {
    if let Some(window) = server.windows.get_mut(window) {
        crate::ui::redraw::redraw_invalidate_scene(window);
    }
}

/// The complete G15 snapshot, in connection order, including control sizes.
pub fn resize_clients(server: &Server) -> Vec<ResizeClient> {
    let mut clients = crate::client::registry::resize_clients(server);
    for client in &mut clients {
        client.status_lines = client
            .session
            .and_then(|id| server.sessions.get(id))
            .map_or(0, |s| s.statuslines);
    }
    clients
}
pub fn recalculate_sizes(server: &mut Server) {
    recalculate_sizes_now(server, false);
}
pub fn recalculate_sizes_now(server: &mut Server, now: bool) {
    // C updates the status cache before reading any client's status height.
    for session in sessions(server) {
        server
            .sessions
            .get_mut(session)
            .expect("session order")
            .attached = 0;
        crate::ui::status::status_update_cache(server, session);
    }
    let mut snapshot = resize_clients(server);
    for i in 0..snapshot.len() {
        if model::resize::ignore_client_size(&snapshot[i], &snapshot) {
            continue;
        }
        if snapshot[i].size.sy <= snapshot[i].status_lines
            || snapshot[i].flags.contains(ClientFlags::CONTROL)
        {
            snapshot[i].flags.insert(ClientFlags::STATUSOFF);
        } else {
            snapshot[i].flags.remove(ClientFlags::STATUSOFF);
        }
        if let Some(live) = server.clients.get_mut(snapshot[i].id) {
            if snapshot[i].flags.contains(ClientFlags::STATUSOFF) {
                live.flags.insert(ClientFlags::STATUSOFF);
            } else {
                live.flags.remove(ClientFlags::STATUSOFF);
            }
        }
    }
    model::resize::recalculate_sizes(server, &mut snapshot, now).expect("live resize snapshot");
}

/// Apply in the source's order, reloading each option at its own step. Tty
/// changes finish here, before the caller inserts its after-hook.
pub fn apply_option_changes(server: &mut Server, changes: Vec<OptionsChange>) {
    for change in changes {
        match change {
            OptionsChange::ClientThemeColours => {
                for client in clients(server) {
                    crate::client::theme::update_theme_colours(server, client);
                    if let Some(tty) = server
                        .clients
                        .get_mut(client)
                        .and_then(|c| c.tty.as_mut())
                        .filter(|tty| tty.flags().contains(TtyFlags::OPENED))
                    {
                        tty.invalidate(&mut server.tparm);
                    }
                    operations::server_redraw_client(server, client);
                }
            }
            OptionsChange::WindowAutomaticRename => {
                for window in windows(server) {
                    let active = server.windows.get(window).and_then(|w| {
                        (server.options.get_number(w.options, b"automatic-rename") != 0)
                            .then_some(w.active)
                            .flatten()
                    });
                    if let Some(pane) = active.and_then(|p| server.panes.get_mut(p)) {
                        pane.flags.insert(PaneFlags::CHANGED);
                    }
                }
            }
            OptionsChange::PaneDefaultCursor => {
                for pane in panes(server) {
                    default_cursor(server, pane);
                }
            }
            OptionsChange::WindowFillCells => {
                for window in windows(server) {
                    crate::ui::border::window_set_fill_cells(server, window);
                }
            }
            OptionsChange::ClientKeyTable => {
                for client in clients(server) {
                    crate::client::keys::set_key_table(server, client, None);
                }
            }
            OptionsChange::ClientTtyKeysBuild => {
                for client in clients(server) {
                    crate::client::lifecycle::rebuild_tty_keys(server, client);
                }
            }
            OptionsChange::StatusTimerStartAll => crate::ui::status::status_timer_start_all(server),
            OptionsChange::RedrawInvalidateAllScenes => {
                crate::ui::redraw::redraw_invalidate_all_scenes(server)
            }
            OptionsChange::AlertsResetAll => model::alerts::alerts_reset_all(server),
            OptionsChange::PaneStyleAndThemeChanged => {
                for pane in panes(server) {
                    server
                        .panes
                        .get_mut(pane)
                        .expect("pane order")
                        .flags
                        .insert(PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
                }
            }
            OptionsChange::PaneStyleChanged => {
                for pane in panes(server) {
                    server
                        .panes
                        .get_mut(pane)
                        .expect("pane order")
                        .flags
                        .insert(PaneFlags::STYLECHANGED);
                }
            }
            OptionsChange::PanePaletteFromOption => {
                for pane in panes(server) {
                    reload_palette(server, pane);
                }
            }
            OptionsChange::WindowScrollbarsReload => {
                for window in windows(server) {
                    let options = server.windows.get(window).expect("window order").options;
                    let policy = crate::ui::scrollbar::PaneScrollbarPolicy::try_from(
                        server.options.get_number(options, b"pane-scrollbars") as i32,
                    )
                    .expect("scrollbar policy");
                    let position = crate::ui::scrollbar::PaneScrollbarPosition::try_from(
                        server
                            .options
                            .get_number(options, b"pane-scrollbars-position")
                            as i32,
                    )
                    .expect("scrollbar position");
                    let w = server.windows.get_mut(window).expect("window order");
                    w.sb = policy;
                    w.sb_pos = position;
                    crate::layout::fix_panes(server, window, None);
                }
            }
            OptionsChange::PaneScrollbarHide => {
                for pane in panes(server) {
                    model::pane::pane_scrollbar_hide(server, pane).expect("pane order");
                }
            }
            OptionsChange::PaneScrollbarStyle => {
                for pane in panes(server) {
                    reload_scrollbar_style(server, pane);
                }
                for window in windows(server) {
                    crate::layout::fix_panes(server, window, None);
                }
            }
            OptionsChange::Utf8UpdateWidthCache => {
                let entries = server
                    .options
                    .get(server.options.global, b"codepoint-widths")
                    .map(|(_, e)| e);
                rmux_util::utf8::width::with_width_cache(|cache| {
                    cache.rebuild(
                        entries
                            .into_iter()
                            .flat_map(|entry| entry.array_items())
                            .map(|(_, item)| item.value().as_string()),
                    );
                });
            }
            OptionsChange::InputSetBufferSize => {
                // Unlike C's global variable, input_policy reloads this option
                // for every parser entry; no cached parser limit can go stale.
            }
            OptionsChange::SessionUpdateHistory => {
                for session in sessions(server) {
                    model::session::session_update_history(server, session);
                }
            }
            OptionsChange::SessionStatusUpdateCache => {
                for session in sessions(server) {
                    crate::ui::status::status_update_cache(server, session);
                }
            }
            OptionsChange::RecalculateSizes => recalculate_sizes(server),
            OptionsChange::ClientRedrawWithSession => {
                for client in clients(server) {
                    if server
                        .clients
                        .get(client)
                        .is_some_and(|c| c.session.is_some())
                    {
                        operations::server_redraw_client(server, client);
                    }
                }
            }
        }
    }
    drain_effects(server).expect("applying option effects");
}
fn default_cursor(server: &mut Server, pane: PaneId) {
    let Some(options) = server.panes.get(pane).map(|p| p.options) else {
        return;
    };
    let mut cell = DEFAULT_CELL;
    crate::ui::styles::style_apply(server, &mut cell, options, b"cursor-colour", None);
    let style = server.options.get_number(options, b"cursor-style") as u32;
    let Some(pane) = server.panes.get_mut(pane) else {
        return;
    };
    let screen = pane
        .modes
        .first_mut()
        .and_then(|mode| mode.screen.as_mut())
        .unwrap_or(&mut pane.base);
    screen.set_default_cursor(cell.fg, style);
}
fn reload_palette(server: &mut Server, pane: PaneId) {
    let Some(options) = server.panes.get(pane).map(|p| p.options) else {
        return;
    };
    let defaults = server
        .options
        .get(options, b"pane-colours")
        .map(|(_, entry)| {
            let mut defaults = [Colour::NONE; 256];
            for (key, item) in entry.array_items() {
                if let OptionsArrayKey::Index(index) = key
                    && let Some(slot) = defaults.get_mut(*index as usize)
                    && let Some(value) = item.value().as_number()
                {
                    *slot = Colour(value as i32);
                }
            }
            defaults
        });
    server
        .panes
        .get_mut(pane)
        .expect("pane palette")
        .palette
        .replace_defaults(defaults);
}
fn reload_scrollbar_style(server: &mut Server, pane: PaneId) {
    let Some(options) = server.panes.get(pane).map(|p| p.options) else {
        return;
    };
    let mut tree = crate::format::FormatTree::create(
        None,
        None,
        0,
        crate::format::FormatFlags::NOJOBS,
        server,
    );
    let default = tree.expand(server, b"bg=themedarkgrey,fg=themelightgrey,width=1,pad=0");
    let configured = server
        .options
        .get_string(options, b"pane-scrollbars-style")
        .to_vec();
    let configured = tree.expand(server, &configured);
    tree.release(server);
    let mut style = rmux_emu::style::Style::default();
    style
        .parse(&DEFAULT_CELL, &default, &mut server.hyperlinks)
        .expect("default scrollbar style");
    if style
        .parse(&DEFAULT_CELL, &configured, &mut server.hyperlinks)
        .is_err()
    {
        style
            .parse(&DEFAULT_CELL, &default, &mut server.hyperlinks)
            .expect("default scrollbar style");
    }
    style.width = style.width.max(1);
    style.pad = style.pad.max(0);
    style.gc.data = rmux_util::utf8::Utf8Data::set(b' ');
    server
        .panes
        .get_mut(pane)
        .expect("pane scrollbar")
        .scrollbar_style = style;
}

pub fn drain_effects(server: &mut Server) -> io::Result<()> {
    while let Some(effect) = server.effects.pop_front() {
        match effect {
            // These are records of the event already fired at the mutation.
            ModelEffect::Event {
                name: _,
                session: _,
                window: _,
                pane: _,
            } => {}
            ModelEffect::Paste(model::paste::PasteEvent { event: _, name: _ }) => {}
            ModelEffect::Session(effect) => session_effect(server, effect)?,
            ModelEffect::Timer(TimerRequest::Session(request)) => session_timer(server, request),
            ModelEffect::Alert(effect) => alert_effect(server, effect),
            ModelEffect::Spawn(SpawnEffect::PaneCreated {
                session: _,
                winlink: _,
                window: _,
                pane: _,
                window_index: _,
                command: _,
                cwd: _,
                empty: _,
                respawn: _,
            }) => {}
            ModelEffect::Format(action) => {
                super::format_live::apply_action(server, action);
            }
            ModelEffect::Window(effect) => window_effect(server, effect),
            ModelEffect::Pane(effect) => pane_effect(server, effect)?,
            ModelEffect::Input(action) => input_action(server, action)?,
            ModelEffect::Resize(effect) => resize_effect(server, effect),
            ModelEffect::RedrawWindow(window) => operations::server_redraw_window(server, window),
            ModelEffect::InvalidateScene(window) => invalidate_scene(server, window),
            ModelEffect::Store(effect) => store_effect(server, effect),
            ModelEffect::RecalculateSizes => recalculate_sizes(server),
        }
    }
    crate::tsp::broker::recompute(server);
    Ok(())
}
fn session_effect(server: &mut Server, effect: SessionEffect) -> io::Result<()> {
    match effect {
        SessionEffect::StatusCache(session) => {
            crate::ui::status::status_update_cache(server, session)
        }
        SessionEffect::Status(session) => operations::server_status_session(server, session),
        SessionEffect::LockSession(session) => {
            operations::lock_session(server, session).map_err(io::Error::other)?
        }
        SessionEffect::RecalculateSizes => recalculate_sizes(server),
        SessionEffect::WindowOffset(window) => window_offset(server, window),
        // Synchronous event metadata, not deferred notifications.
        SessionEffect::SessionClosed(_)
        | SessionEffect::WindowLinked {
            session: _,
            window: _,
            winlink: _,
            index: _,
        }
        | SessionEffect::WindowUnlinked {
            session: _,
            window: _,
            winlink: _,
            index: _,
        }
        | SessionEffect::WindowChanged {
            session: _,
            new_window: _,
            new_index: _,
            old: _,
        }
        | SessionEffect::GroupChanged {
            event: _,
            session: _,
            group: _,
            size: _,
            target: _,
        } => {}
    }
    Ok(())
}
fn session_timer(server: &mut Server, request: SessionTimerRequest) {
    match request {
        SessionTimerRequest::Free(session) => set_timer(
            server,
            session_timer_key(session, true),
            Some(Duration::ZERO),
            LoopAction::SessionFree(session),
        ),
        SessionTimerRequest::Lock { session, seconds } => set_timer(
            server,
            session_timer_key(session, false),
            Some(Duration::from_secs(seconds as u64)),
            LoopAction::SessionLock(session),
        ),
        SessionTimerRequest::CancelLock(session) => {
            cancel_timer(server, &session_timer_key(session, false))
        }
    }
}
fn window_effect(server: &mut Server, effect: WindowEffect) {
    match effect {
        WindowEffect::InvalidateScene(window) => invalidate_scene(server, window),
        WindowEffect::Redraw(window) => operations::server_redraw_window(server, window),
        WindowEffect::Borders(window) => operations::server_redraw_window_borders(server, window),
        WindowEffect::Status(window) => operations::server_status_window(server, window),
        WindowEffect::UpdateOffset(window) => window_offset(server, window),
        WindowEffect::CancelTimers(window) => {
            cancel_timer(server, &window_timer_key(window, true));
            cancel_timer(server, &window_timer_key(window, false));
        }
        WindowEffect::Renamed {
            window: _,
            old: _,
            new: _,
        }
        | WindowEffect::PaneChanged {
            window: _,
            old: _,
            new: _,
        }
        | WindowEffect::PaneMoved {
            pane: _,
            old_window: _,
            new_window: _,
            old_index: _,
            new_index: _,
        } => {}
    }
}
fn pane_effect(server: &mut Server, effect: PaneEffect) -> io::Result<()> {
    match effect {
        PaneEffect::WaitFinished {
            pane: _,
            item,
            retval,
        } => {
            if let Some(client) = server
                .queue
                .items
                .get(item)
                .and_then(|item| item.client)
                .and_then(|id| server.clients.get_mut(id))
                && client.session.is_none()
            {
                client.retval = retval;
            }
            queue::continue_item(&mut server.queue, item);
        }
        PaneEffect::CancelTimers(pane) => {
            for timer in [InputTimer::Ground, InputTimer::Requests, InputTimer::Sync] {
                cancel_timer(server, &pane_timer_key(pane, timer));
            }
            cancel_timer(server, &scrollbar_timer_key(pane));
            crate::client::tick::cancel_pane_resize_timer(server, pane);
            pane_runtime::close_pane_io(server, pane);
        }
        PaneEffect::ClearSync(pane) => {
            crate::ui::fanout::screen_write_sync_clear_dirty(server, pane)
        }
        PaneEffect::StopSync(pane) => pane_runtime::stop_sync(server, pane),
        PaneEffect::Kill(pane) => {
            if server.panes.get(pane).is_some() {
                operations::server_kill_pane(server, pane).map_err(model_error)?;
            }
        }
        // The model queued the resize; G15's tick coalesces and sends ioctls.
        PaneEffect::Resized { pane: _, size: _ } => {}
        PaneEffect::ScrollbarTimer { pane, milliseconds } => set_timer(
            server,
            scrollbar_timer_key(pane),
            milliseconds.map(Duration::from_millis),
            LoopAction::PaneScrollbar(pane),
        ),
        PaneEffect::ModeChanged {
            pane: _,
            previous: _,
            current: _,
            entered: _,
        }
        | PaneEffect::PromptChanged { pane: _, kind: _ }
        | PaneEffect::TitleChanged { pane: _, new: _ } => {}
    }
    Ok(())
}
fn store_effect(server: &mut Server, effect: StoreEffect) {
    match effect {
        StoreEffect::NameTimer { window, delay_usec } => set_timer(
            server,
            window_timer_key(window, true),
            delay_usec.map(Duration::from_micros),
            LoopAction::WindowName(window),
        ),
        StoreEffect::MonitorTimer { set, pending } => {
            use model::monitor::MonitorRuntime;
            let mut adapter = crate::control::monitor::MonitorAdapter::new(server);
            adapter.timer(set, pending);
            adapter.apply(server);
        }
        StoreEffect::Borders(window) => operations::server_redraw_window_borders(server, window),
        StoreEffect::Status(window) => operations::server_status_window(server, window),
    }
}
fn resize_effect(server: &mut Server, effect: ResizeEffect) {
    match effect {
        ResizeEffect::Offset(window) => window_offset(server, window),
        ResizeEffect::Redraw(window) => operations::server_redraw_window(server, window),
        ResizeEffect::StatusCache(session) => {
            crate::ui::status::status_update_cache(server, session)
        }
        // window-resized is delivered by the producer while its old size
        // metadata and target are still valid, just like pane-resized.
        ResizeEffect::Resized {
            window: _,
            old_sx: _,
            old_sy: _,
            sx: _,
            sy: _,
        } => {}
    }
}
fn alert_effect(server: &mut Server, effect: AlertEffect) {
    match effect {
        AlertEffect::SilenceTimer { window, seconds } => set_timer(
            server,
            window_timer_key(window, false),
            (seconds != 0).then(|| Duration::from_secs(seconds)),
            LoopAction::WindowSilence(window),
        ),
        AlertEffect::DeferredCheck => set_timer(
            server,
            b"alerts-check".to_vec(),
            Some(Duration::ZERO),
            LoopAction::AlertsCheck,
        ),
        AlertEffect::Status(session) => operations::server_status_session(server, session),
        AlertEffect::Hook { link, kind } => {
            if server.model_event.is_some() && server.winlinks.get(link).is_some() {
                let name: &[u8] = match kind {
                    AlertKind::Bell => b"alert-bell",
                    AlertKind::Activity => b"alert-activity",
                    AlertKind::Silence => b"alert-silence",
                };
                events::fire_winlink(server, name, link);
            }
        }
        AlertEffect::Delivery {
            session,
            kind,
            current,
            index,
            visual,
        } => {
            for client in clients(server) {
                let Some(source) = server
                    .clients
                    .get(client)
                    .filter(|c| c.session == Some(session))
                else {
                    continue;
                };
                let (bell, message) =
                    visual_delivery(visual, source.flags.contains(ClientFlags::CONTROL));
                if bell
                    && let Some(tty) = server
                        .clients
                        .get_mut(client)
                        .and_then(|c| c.tty.as_mut())
                        .filter(|tty| tty.flags().contains(TtyFlags::OPENED))
                {
                    tty.putcode(rmux_tty::term::TtyCodeCode::Bel);
                }
                if message {
                    let message = if current {
                        format!("{} in current window", kind.label())
                    } else {
                        format!("{} in window {index}", kind.label())
                    };
                    crate::ui::status::status_message_set(
                        server,
                        Some(client),
                        -1,
                        true,
                        false,
                        false,
                        message.as_bytes(),
                    );
                }
            }
        }
    }
}

/// Extra host delivery only: the model has already applied the parser effect.
pub fn input_effect(server: &mut Server, pane: PaneId, effect: &OwnedInputEffect) {
    match effect {
        OwnedInputEffect::Bell => {
            if let Some(window) = server.panes.get(pane).map(|p| p.window) {
                model::alerts::alerts_queue(server, window, WindowFlags::BELL);
            }
        }
        OwnedInputEffect::ClipboardReceived { clip, data } => {
            if server
                .options
                .get_number(server.options.global, b"set-clipboard")
                == 2
            {
                crate::ui::fanout::pane_set_selection(server, pane, clip, data);
                if server.model_event.is_some() && server.panes.get(pane).is_some() {
                    events::fire_pane(server, b"pane-set-clipboard", pane);
                }
            }
        }
        // Reply bytes, requests, flags, renames and named notifications were
        // applied synchronously by pane_input::apply_effect, before this call.
        OwnedInputEffect::Reply(_)
        | OwnedInputEffect::TspMessage(_)
        | OwnedInputEffect::TerminalReset
        | OwnedInputEffect::Request { kind: _, end: _ }
        | OwnedInputEffect::ClipboardQuery { clip: _, end: _ }
        | OwnedInputEffect::ColourQuery { which: _, end: _ }
        | OwnedInputEffect::ThemeReport
        | OwnedInputEffect::ThemeUpdatesEnabled
        | OwnedInputEffect::ThemeUpdatesDisabled
        | OwnedInputEffect::TitleChanged(_)
        | OwnedInputEffect::TitlePopped(_)
        | OwnedInputEffect::PathChanged
        | OwnedInputEffect::Rename(_)
        | OwnedInputEffect::ProgressChanged
        | OwnedInputEffect::StyleChanged { theme: _ }
        | OwnedInputEffect::SyncStart
        | OwnedInputEffect::SyncEnd
        | OwnedInputEffect::Osc133(_)
        | OwnedInputEffect::GroundTimer(_)
        | OwnedInputEffect::AlternateChanged { entering: _ } => {}
    }
}
fn input_action(server: &mut Server, action: InputAction) -> io::Result<()> {
    match action {
        InputAction::Effect { pane, effect } => {
            pane_runtime::apply_input_effect(server, pane, effect).map_err(model_error)?
        }
        InputAction::Timer { pane, timer, after } => set_timer(
            server,
            pane_timer_key(pane, timer),
            after,
            LoopAction::PaneInputTimer(pane, timer),
        ),
        InputAction::CancelRequest {
            pane: _,
            request,
            client,
        } => {
            if let Some(client) = server.clients.get_mut(client) {
                client.input_requests.remove(request);
            }
        }
        InputAction::ResetConsumers(pane) => {
            for client in clients(server) {
                crate::control::reset_pane(server, client, pane);
            }
            crate::cmd::commands::pipe_pane::reset_offset(server, pane);
        }
        InputAction::StdinStart { pane, client, item } => {
            let mut input = PaneStdinInput::new(pane, client, item);
            let callback = Box::new(
                move |server: &mut Server, file, notice, bytes: &[u8], error| {
                    let mut buffer = bytes.to_vec();
                    let state = StdinReadState {
                        client_dead: server
                            .clients
                            .get(client)
                            .is_none_or(|c| c.flags.contains(ClientFlags::DEAD)),
                        file_present: server.files.get(file).is_some(),
                        closed: notice == super::file::FileNotice::Done,
                        error: error != 0,
                    };
                    let result =
                        pane_runtime::stdin_input_chunk(server, &mut input, &mut buffer, state);
                    server.files.consume(file, bytes.len());
                    if let Err(error) = result {
                        super::file::error(server, client, error.to_string().as_bytes());
                        if let Some(client) = server.clients.get_mut(client) {
                            client.retval = 1;
                            client.flags.insert(ClientFlags::EXIT);
                        }
                        super::file::cancel(server, file);
                        server.pane_stdin_files.remove(&(pane, client, item));
                        queue::continue_item(&mut server.queue, item);
                    }
                },
            );
            if let Some(file) = super::file::read(server, Some(client), b"-", callback) {
                server.pane_stdin_files.insert((pane, client, item), file);
            }
        }
        InputAction::StdinCancel {
            pane,
            client,
            item,
            exit_status,
        } => {
            if let Some(status) = exit_status
                && let Some(client) = server.clients.get_mut(client)
            {
                client.retval = status;
                client.flags.insert(ClientFlags::EXIT);
            }
            if let Some(file) = server.pane_stdin_files.remove(&(pane, client, item)) {
                super::file::cancel(server, file);
            }
        }
        InputAction::StdinFinished { client, item } => {
            server
                .pane_stdin_files
                .retain(|(_, c, q), _| *c != client || *q != item);
            queue::continue_item(&mut server.queue, item);
        }
    }
    Ok(())
}

/// Dispatch only the effect-owned typed timers; unrelated actions are an error.
pub fn timer_ready(server: &mut Server, action: LoopAction) -> io::Result<()> {
    let key = match &action {
        LoopAction::SessionFree(session) => session_timer_key(*session, true),
        LoopAction::SessionLock(session) => session_timer_key(*session, false),
        LoopAction::WindowName(window) => window_timer_key(*window, true),
        LoopAction::WindowSilence(window) => window_timer_key(*window, false),
        LoopAction::PaneInputTimer(pane, timer) => pane_timer_key(*pane, *timer),
        LoopAction::PaneScrollbar(pane) => scrollbar_timer_key(*pane),
        LoopAction::AlertsCheck => b"alerts-check".to_vec(),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a model effect timer",
            ));
        }
    };
    let Some(timer) = server.runtime_timers.get(&key).copied() else {
        return Ok(());
    };
    // A previous poll batch may still hold the old action after a callback
    // cancelled and replaced its timer. Do not consume the replacement.
    if server.event_loop.timer_action(timer).is_some() {
        return Ok(());
    }
    server.runtime_timers.remove(&key);
    match action {
        LoopAction::SessionFree(session) => {
            model::session::session_free(server, session);
        }
        LoopAction::SessionLock(session) => model::session::session_lock_timer(server, session),
        LoopAction::WindowName(window) => {
            if server.windows.get(window).is_some() {
                model::names::name_timer_fired(server, window);
                server.check_window_name(window).map_err(model_error)?;
            }
        }
        LoopAction::WindowSilence(window) => {
            if server.windows.get(window).is_some() {
                model::alerts::alerts_queue(server, window, WindowFlags::SILENCE);
            }
        }
        LoopAction::PaneInputTimer(pane, timer) => {
            if server.panes.get(pane).is_some() {
                match timer {
                    InputTimer::Ground => model::pane_input::input_ground_timer(server, pane),
                    InputTimer::Requests => {
                        let now = rmux_util::time::Timestamp::now();
                        let millis = (now.sec as u64)
                            .wrapping_mul(1000)
                            .wrapping_add((now.usec as u64) / 1000);
                        model::pane_input::input_request_timer(server, pane, millis)
                            .map_err(model_error)?;
                    }
                    InputTimer::Sync => pane_runtime::input_sync_timer(server, pane),
                }
            }
        }
        LoopAction::PaneScrollbar(pane) => model::pane::pane_scrollbar_timer(server, pane),
        LoopAction::AlertsCheck => model::alerts::alerts_dispatch(server),
        _ => unreachable!("validated model effect timer"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{client::Client, ids::ArenaId};
    fn pane(server: &mut Server) -> (WindowId, PaneId) {
        let window = model::window::window_create(server, 20, 6, 0, 0).unwrap();
        let pane = model::window::window_add_pane(
            server,
            window,
            None,
            10,
            model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        model::window::window_set_active_pane(server, window, pane, false).unwrap();
        crate::layout::tree::init(server, window, pane);
        server.effects.clear();
        (window, pane)
    }
    fn client(server: &mut Server) -> ClientId {
        let id = server.clients.insert(Client::new(None, (0, 0))).unwrap();
        server.client_order.push_back(id);
        id
    }
    #[test]
    fn named_event_is_not_replayed_by_drain() {
        fn callback(
            server: &mut Server,
            name: &[u8],
            _: Option<SessionId>,
            _: Option<WindowId>,
            _: Option<PaneId>,
        ) {
            if name == b"probe" {
                server.source_file_depth += 1;
            }
        }
        let mut server = Server::new();
        server.model_event = Some(callback);
        server.emit(b"probe", None, None, None);
        assert_eq!(server.source_file_depth, 1);
        drain_effects(&mut server).unwrap();
        assert_eq!(server.source_file_depth, 1);
        assert!(server.effects.is_empty());
    }
    #[test]
    fn replaced_timer_and_destroyed_pane_are_cancelled() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        input_action(
            &mut server,
            InputAction::Timer {
                pane,
                timer: InputTimer::Ground,
                after: Some(Duration::from_secs(20)),
            },
        )
        .unwrap();
        let key = pane_timer_key(pane, InputTimer::Ground);
        let old = server.runtime_timers[&key];
        input_action(
            &mut server,
            InputAction::Timer {
                pane,
                timer: InputTimer::Ground,
                after: Some(Duration::from_secs(10)),
            },
        )
        .unwrap();
        assert_ne!(old, server.runtime_timers[&key]);
        assert!(server.event_loop.timer_action(old).is_none());
        pane_effect(&mut server, PaneEffect::CancelTimers(pane)).unwrap();
        assert!(!server.runtime_timers.contains_key(&key));
        timer_ready(
            &mut server,
            LoopAction::PaneInputTimer(pane, InputTimer::Ground),
        )
        .unwrap();
    }
    #[test]
    fn monitor_effect_uses_control_timer_and_cancels() {
        let mut server = Server::new();
        let set = crate::ids::MonitorSetId::from_parts(3, 2);
        store_effect(
            &mut server,
            StoreEffect::MonitorTimer { set, pending: true },
        );
        let key = format!("monitor:{set:?}").into_bytes();
        let timer = server.runtime_timers[&key];
        assert_eq!(
            server.event_loop.timer_action(timer),
            Some(&LoopAction::ControlMonitor(set))
        );
        store_effect(
            &mut server,
            StoreEffect::MonitorTimer {
                set,
                pending: false,
            },
        );
        assert!(server.event_loop.timer_action(timer).is_none());
        assert!(server.runtime_timers.is_empty());
    }
    #[test]
    fn ordered_cursor_palette_style_and_theme_reload() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        let options = server.panes.get(pane).unwrap().options;
        server.options.set_number_value(options, b"cursor-style", 5);
        let mut store = std::mem::take(&mut server.options);
        store.set_string(
            options,
            b"pane-scrollbars-style",
            false,
            b"width=4,pad=2",
            &mut server,
        );
        server.options = store;
        server.panes.get_mut(pane).unwrap().flags = PaneFlags::default();
        apply_option_changes(
            &mut server,
            vec![
                OptionsChange::PaneDefaultCursor,
                OptionsChange::PaneScrollbarStyle,
                OptionsChange::PaneStyleAndThemeChanged,
            ],
        );
        let p = server.panes.get(pane).unwrap();
        assert_eq!(
            p.base.default_cstyle,
            rmux_emu::screen::ScreenCursorStyle::Bar
        );
        assert_eq!((p.scrollbar_style.width, p.scrollbar_style.pad), (4, 2));
        assert!(
            p.flags
                .contains(PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED)
        );
    }
    #[test]
    fn wait_finished_changes_only_unattached_client_return_value() {
        let mut server = Server::new();
        let c = client(&mut server);
        let batch = server
            .queue
            .get_callback("wait", Box::new(|_, _| queue::CmdReturn::Normal))
            .unwrap();
        let item = batch.items[0];
        server.queue.items.get_mut(item).unwrap().client = Some(c);
        server
            .queue
            .items
            .get_mut(item)
            .unwrap()
            .flags
            .insert(queue::QueueItemFlags::WAITING);
        pane_effect(
            &mut server,
            PaneEffect::WaitFinished {
                pane: PaneId::from_parts(99, 0),
                item,
                retval: 37,
            },
        )
        .unwrap();
        assert_eq!(server.clients.get(c).unwrap().retval, 37);
        assert!(
            !server
                .queue
                .items
                .get(item)
                .unwrap()
                .flags
                .contains(queue::QueueItemFlags::WAITING)
        );
        server.clients.get_mut(c).unwrap().session = Some(SessionId::from_parts(99, 0));
        pane_effect(
            &mut server,
            PaneEffect::WaitFinished {
                pane: PaneId::from_parts(99, 0),
                item,
                retval: 41,
            },
        )
        .unwrap();
        assert_eq!(server.clients.get(c).unwrap().retval, 37);
    }
    #[test]
    fn bell_delivery_skips_control_clients() {
        let mut server = Server::new();
        let c = client(&mut server);
        let s = SessionId::from_parts(99, 0);
        server.clients.get_mut(c).unwrap().session = Some(s);
        server
            .clients
            .get_mut(c)
            .unwrap()
            .flags
            .insert(ClientFlags::CONTROL);
        alert_effect(
            &mut server,
            AlertEffect::Delivery {
                session: s,
                kind: AlertKind::Bell,
                current: true,
                index: 0,
                visual: model::alerts::VisualPolicy::Both,
            },
        );
        assert!(server.clients.get(c).unwrap().message.text.is_none());
    }
    #[test]
    fn cancelled_expired_action_does_not_consume_replacement() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        let key = scrollbar_timer_key(pane);
        set_timer(
            &mut server,
            key.clone(),
            Some(Duration::ZERO),
            LoopAction::PaneScrollbar(pane),
        );
        let ready = server.event_loop.poll(Some(Duration::ZERO)).unwrap();
        set_timer(
            &mut server,
            key.clone(),
            Some(Duration::from_secs(20)),
            LoopAction::PaneScrollbar(pane),
        );
        let replacement = server.runtime_timers[&key];
        for ready in ready {
            timer_ready(&mut server, ready.action).unwrap();
        }
        assert_eq!(server.runtime_timers.get(&key), Some(&replacement));
    }
    #[test]
    fn options_cache_is_reloaded_before_resize_snapshot() {
        let mut server = Server::new();
        let (window, _) = pane(&mut server);
        let options = server.options.create(Some(server.options.global_s));
        let s = model::session::session_create(
            &mut server,
            model::session::SessionCreate {
                prefix: None,
                name: Some(b"snapshot".to_vec()),
                cwd: b"/".to_vec(),
                environment: crate::options::environment::Environment::default(),
                options,
                termios: None,
            },
        );
        let link = model::session::session_attach(&mut server, s, window, 0).unwrap();
        model::session::session_set_current(&mut server, s, Some(link));
        let c = client(&mut server);
        server.clients.get_mut(c).unwrap().session = Some(s);
        server
            .clients
            .get_mut(c)
            .unwrap()
            .flags
            .insert(ClientFlags::STATUSOFF);
        server.clients.get_mut(c).unwrap().tty_sy = 3;
        server.options.set_number_value(options, b"status", 4);
        server.effects.clear();
        apply_option_changes(
            &mut server,
            vec![
                OptionsChange::SessionStatusUpdateCache,
                OptionsChange::RecalculateSizes,
            ],
        );
        assert_eq!(resize_clients(&server)[0].status_lines, 4);
        assert!(
            server
                .clients
                .get(c)
                .unwrap()
                .flags
                .contains(ClientFlags::STATUSOFF)
        );
        server.options.set_number_value(options, b"status", 1);
        recalculate_sizes_now(&mut server, true);
        assert!(
            !server
                .clients
                .get(c)
                .unwrap()
                .flags
                .contains(ClientFlags::STATUSOFF)
        );
        assert_eq!(server.sessions.get(s).unwrap().attached, 1);
    }
    #[test]
    fn input_limit_change_is_read_at_next_parser_entry() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        server
            .options
            .set_number_value(server.options.global, b"input-buffer-size", 8192);
        apply_option_changes(&mut server, vec![OptionsChange::InputSetBufferSize]);
        assert_eq!(
            model::pane_input::input_policy(&server, pane)
                .unwrap()
                .buffer_limit,
            8192
        );
    }
    #[test]
    fn palette_reload_preserves_runtime_overrides() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        let options = server.panes.get(pane).unwrap().options;
        let mut store = std::mem::take(&mut server.options);
        let entry = store.empty(options, crate::options::search(b"pane-colours").unwrap());
        entry
            .array_set(&OptionsArrayKey::Index(1), Some(b"red"), false, &mut server)
            .unwrap();
        server.options = store;
        server
            .panes
            .get_mut(pane)
            .unwrap()
            .palette
            .set(1, Colour(2));
        apply_option_changes(&mut server, vec![OptionsChange::PanePaletteFromOption]);
        let palette = &mut server.panes.get_mut(pane).unwrap().palette;
        assert_eq!(palette.get(Colour(1)), Some(Colour(2)));
        palette.clear_runtime();
        assert_eq!(palette.get(Colour(1)), Some(Colour(1)));
    }
    #[test]
    fn input_ground_timer_runs_only_after_poll() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        server.panes.get_mut(pane).unwrap().input_state.ground_timer = true;
        input_action(
            &mut server,
            InputAction::Timer {
                pane,
                timer: InputTimer::Ground,
                after: Some(Duration::ZERO),
            },
        )
        .unwrap();
        assert!(server.panes.get(pane).unwrap().input_state.ground_timer);
        for ready in server.event_loop.poll(Some(Duration::ZERO)).unwrap() {
            timer_ready(&mut server, ready.action).unwrap();
        }
        assert!(!server.panes.get(pane).unwrap().input_state.ground_timer);
        assert!(server.runtime_timers.is_empty());
    }
    #[test]
    fn removed_pane_cancels_resize_and_input_generations() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        set_timer(
            &mut server,
            crate::client::tick::pane_resize_timer_key(pane),
            Some(Duration::from_secs(1)),
            LoopAction::PaneResizeTimer(pane),
        );
        input_action(
            &mut server,
            InputAction::Timer {
                pane,
                timer: InputTimer::Sync,
                after: Some(Duration::from_secs(1)),
            },
        )
        .unwrap();
        model::pane::pane_destroy(&mut server, pane).unwrap();
        drain_effects(&mut server).unwrap();
        assert!(server.panes.get(pane).is_none());
        assert!(server.runtime_timers.is_empty());
    }
    #[test]
    fn mirror_effects_do_not_add_runtime_events_without_model_callback() {
        let mut server = Server::new();
        let (window, pane) = pane(&mut server);
        events::add_sink(&mut server, b"window-renamed", |server, _| {
            server.source_file_depth += 1
        });
        model::window::window_set_name(&mut server, window, b"renamed", true).unwrap();
        server.fire_paste_event("paste-buffer-changed", b"named");
        server
            .effects
            .push_back(ModelEffect::Spawn(SpawnEffect::PaneCreated {
                session: SessionId::from_parts(9, 0),
                winlink: crate::ids::WinlinkId::from_parts(9, 0),
                window,
                pane,
                window_index: 0,
                command: b"command".to_vec(),
                cwd: b"/".to_vec(),
                empty: false,
                respawn: false,
            }));
        drain_effects(&mut server).unwrap();
        assert_eq!(server.source_file_depth, 0);
        assert!(server.effects.is_empty());
    }
    #[test]
    fn stdin_request_error_eventually_continues_waiter() {
        let mut server = Server::new();
        let (_, pane) = pane(&mut server);
        let c = client(&mut server);
        let batch = server
            .queue
            .get_callback("stdin", Box::new(|_, _| queue::CmdReturn::Normal))
            .unwrap();
        let item = batch.items[0];
        server
            .queue
            .items
            .get_mut(item)
            .unwrap()
            .flags
            .insert(queue::QueueItemFlags::WAITING);
        input_action(
            &mut server,
            InputAction::StdinStart {
                pane,
                client: c,
                item,
            },
        )
        .unwrap();
        let ready = server.event_loop.poll(Some(Duration::ZERO)).unwrap();
        for ready in ready {
            if let LoopAction::FileDone(file) = ready.action {
                super::super::file::fire_done(&mut server, file);
            }
        }
        drain_effects(&mut server).unwrap();
        assert!(
            !server
                .queue
                .items
                .get(item)
                .unwrap()
                .flags
                .contains(queue::QueueItemFlags::WAITING)
        );
        assert!(server.pane_stdin_files.is_empty());
    }
    #[test]
    fn resize_notification_is_synchronous_and_not_replayed() {
        fn callback(
            server: &mut Server,
            name: &[u8],
            _: Option<SessionId>,
            _: Option<WindowId>,
            _: Option<PaneId>,
        ) {
            if name == b"window-resized" {
                server.source_file_depth += 1;
            }
        }
        let mut server = Server::new();
        let (window, _) = pane(&mut server);
        server.model_event = Some(callback);
        model::resize::resize_window(
            &mut server,
            window,
            21,
            7,
            model::resize::PixelUpdate::Default,
            model::resize::PixelUpdate::Default,
        )
        .unwrap();
        assert_eq!(server.source_file_depth, 1);
        drain_effects(&mut server).unwrap();
        assert_eq!(server.source_file_depth, 1);
    }
}
