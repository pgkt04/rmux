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

//! Per-tick client work: `server_client_loop` and the helpers it calls
//! (`server-client.c:1753-1990, 1993-2178, 2308-2540`).

use crate::client::ClientFlags;
use crate::format::{FormatContext, FormatFlags, FormatTree};
use crate::ids::*;
use crate::model::pane::{
    PaneResize, pane_clear_resizes, pane_exited, pane_scrollbar_overlay, pane_scrollbar_visible,
    pane_send_resize,
};
use crate::model::resize::{PixelUpdate, resize_window};
use crate::model::{PaneFlags, WindowFlags};
use crate::server::Server;
use crate::server::event_loop::LoopAction;
use crate::ui::menu::{menu_get_cursor, menu_screen};
use crate::ui::redraw::{
    redraw_client_damage, redraw_free_damage, redraw_pane, redraw_pane_scrollbar, redraw_screen,
};
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use crate::ui::status::{status_at_line, status_line_size, status_prompt_cursor};
use crate::ui::visible::{VisibleRanges, window_position_is_visible, window_visible_ranges};
use rmux_emu::screen::{Screen, ScreenMode};
use rmux_tty::tty::TtyFlags;
use rmux_util::log_debug;
use std::collections::VecDeque;
use std::time::Duration;

/// Key of the shared redraw timer in `Server.runtime_timers`
/// (`static struct event ev`, `server-client.c:2367`).
pub const REDRAW_TIMER_KEY: &[u8] = b"client-redraw";

/// Key of a pane's resize timer in `Server.runtime_timers`
/// (`wp->resize_timer`, `server-client.c:1852-1855`). Keyed by the arena id
/// (slot and generation) so it can be cancelled after the pane is removed.
pub fn pane_resize_timer_key(pane: PaneId) -> Vec<u8> {
    let (slot, generation) = pane.parts();
    format!("pane-resize-{slot}.{generation}").into_bytes()
}

/// `evtimer_del(&wp->resize_timer)` on pane destruction: cancel and forget a
/// pending pane resize timer. Safe to call for an already removed pane.
pub fn cancel_pane_resize_timer(server: &mut Server, pane: PaneId) {
    if let Some(timer) = server.runtime_timers.remove(&pane_resize_timer_key(pane)) {
        server.event_loop.cancel(timer);
    }
}

/// `RB_FOREACH(w, windows, &windows)`: windows in id order.
fn window_ids(server: &Server) -> Vec<WindowId> {
    server.window_ids.values().copied().collect()
}

/// `c->session->curw->window` when both exist (`server-client.c:1780`).
fn current_window(server: &Server, id: ClientId) -> Option<WindowId> {
    let session = server.clients.get(id)?.session?;
    let current = server.sessions.get(session)?.current?;
    Some(server.winlinks.get(current)?.window)
}

/// `server_client_loop` (`server-client.c:1753-1810`).
pub fn tick(server: &mut Server) {
    // Check for window resize. This is done before redrawing.
    for w in window_ids(server) {
        check_window_resize(server, w);
    }
    crate::tsp::lifetime::reap_programs(server);
    crate::tsp::broker::recompute(server);

    // Notify modes that pane styles may have changed.
    for w in window_ids(server) {
        let Some(panes) = server.windows.get(w).map(|w| w.panes.clone()) else {
            continue;
        };
        for wp in panes {
            let changed = server.panes.get(wp).is_some_and(|p| {
                p.flags.intersects(PaneFlags::STYLECHANGED) && !p.modes.is_empty()
            });
            if changed {
                crate::model::pane::pane_mode_style_changed(server, wp);
            }
        }
    }

    // Check clients.
    for c in server.client_order.clone() {
        if server.clients.get(c).is_none() {
            continue;
        }
        crate::client::exit::check_exit(server, c, false);
        if current_window(server, c).is_some() {
            check_modes(server, c);
            check_redraw(server, c);
            reset_state(server, c);
        }
        // event_add(&tty->event_out) for output queued above (tty_write).
        crate::client::tty_io::sync(server, c);
    }

    // Clear window redraw state after processing all clients. Deferred
    // redraws are preserved in client flags.
    for w in window_ids(server) {
        let Some(panes) = server.windows.get(w).map(|w| w.panes.clone()) else {
            continue;
        };
        for wp in panes {
            if server.panes.get(wp).is_some_and(|p| p.has_fd()) {
                check_pane_resize(server, wp);
                check_pane_buffer(server, wp);
            }
            if let Some(p) = server.panes.get_mut(wp) {
                p.flags
                    .remove(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR | PaneFlags::ACTIVITY);
            }
        }
        if let Some(window) = server.windows.get_mut(w) {
            redraw_free_damage(window);
        }
        // check_window_name only fails for a stale id (server-client.c:1802).
        let _ = server.check_window_name(w);
    }

    // Send theme updates.
    for w in window_ids(server) {
        let Some(panes) = server.windows.get(w).map(|w| w.panes.clone()) else {
            continue;
        };
        for wp in panes {
            send_theme_update(server, wp);
        }
    }
}

/// `server_client_check_window_resize` (`server-client.c:1813-1830`).
pub fn check_window_resize(server: &mut Server, w: WindowId) {
    let Some(window) = server.windows.get(w) else {
        return;
    };
    if !window.flags.intersects(WindowFlags::RESIZE) {
        return;
    }
    let shown = window.links.iter().any(|&wl| {
        server.winlinks.get(wl).is_some_and(|link| {
            server
                .sessions
                .get(link.session)
                .is_some_and(|s| s.attached != 0 && s.current == Some(wl))
        })
    });
    if !shown {
        return;
    }
    let Some(size) = window.pending else {
        return;
    };
    log_debug!("check_window_resize: resizing window @{}", window.public_id);
    let _ = resize_window(
        server,
        w,
        size.sx,
        size.sy,
        PixelUpdate::Set(size.xpixel),
        PixelUpdate::Set(size.ypixel),
    );
}

/// `server_client_resize_timer` (`server-client.c:1833-1840`): the timer
/// only logs and deletes itself.
pub fn resize_timer(server: &mut Server, wp: PaneId) {
    if let Some(p) = server.panes.get(wp) {
        log_debug!("resize_timer: %{} resize timer expired", p.public_id);
    }
    server.runtime_timers.remove(&pane_resize_timer_key(wp));
}

/// Which queued resize `check_pane_resize` sends (`server-client.c:1863-1899`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeDecision {
    /// Size to send to the pane.
    pub send: (u32, u32),
    /// Keep the last entry on the queue for the next check.
    pub keep_last: bool,
    /// Delay before the next check in milliseconds.
    pub timer_ms: u64,
}

/// Pure reduction of a non-empty resize queue to one resize
/// (`server-client.c:1876-1899`).
pub fn reduce_resizes(queue: &VecDeque<PaneResize>) -> Option<ResizeDecision> {
    let first = queue.front()?;
    let last = queue.back()?;
    if queue.len() == 1 {
        // Only one resize.
        Some(ResizeDecision {
            send: (first.sx, first.sy),
            keep_last: false,
            timer_ms: 250,
        })
    } else if last.sx != first.osx || last.sy != first.osy {
        // Multiple resizes ending up with a different size.
        Some(ResizeDecision {
            send: (last.sx, last.sy),
            keep_last: false,
            timer_ms: 250,
        })
    } else {
        // Multiple resizes ending up with the same size. There will not be
        // more than one to the same size in succession so use the
        // last-but-one and leave the last for later, with a shorter delay.
        let r = queue[queue.len() - 2];
        Some(ResizeDecision {
            send: (r.sx, r.sy),
            keep_last: true,
            timer_ms: 10,
        })
    }
}

/// `server_client_check_pane_resize` (`server-client.c:1843-1901`).
pub fn check_pane_resize(server: &mut Server, wp: PaneId) {
    let Some(p) = server.panes.get(wp) else {
        return;
    };
    if p.resizes.is_empty() {
        return;
    }
    let public_id = p.public_id;
    let key = pane_resize_timer_key(wp);
    if server.runtime_timers.contains_key(&key) {
        return;
    }

    log_debug!("check_pane_resize: %{} needs to be resized", public_id);
    for r in &p.resizes {
        log_debug!("queued resize: {}x{} -> {}x{}", r.osx, r.osy, r.sx, r.sy);
    }

    let Some(decision) = reduce_resizes(&p.resizes) else {
        return;
    };
    let last_index = p.resizes.len() - 1;
    let _ = pane_send_resize(server, wp, decision.send.0, decision.send.1);
    let _ = pane_clear_resizes(server, wp, decision.keep_last.then_some(last_index));

    let timer = server.event_loop.schedule(
        Duration::from_millis(decision.timer_ms),
        LoopAction::PaneResizeTimer(wp),
    );
    server.runtime_timers.insert(key, timer);
}

/// `server_client_check_pane_buffer` (`server-client.c:1904-1990`). The
/// consumer-minimum drain, control rebase and read enable/disable are owned
/// by `server::pane_runtime::drain_consumers` (one policy for the pty read
/// path and the tick).
pub fn check_pane_buffer(server: &mut Server, wp: PaneId) {
    crate::server::pane_runtime::drain_consumers(server, wp);
}

/// `window_pane_send_theme_update` (`window.c:2853-2883`).
pub fn send_theme_update(server: &mut Server, wp: PaneId) {
    let Some(p) = server.panes.get(wp) else {
        return;
    };
    if pane_exited(server, wp) {
        return;
    }
    if !p.flags.intersects(PaneFlags::THEMECHANGED) {
        return;
    }
    if !p.screen().mode.contains(ScreenMode::THEME_UPDATES) {
        return;
    }
    let theme = crate::client::theme::pane_theme(server, wp);
    let Some(p) = server.panes.get_mut(wp) else {
        return;
    };
    if p.input_state.last_theme == Some(theme) {
        return;
    }
    p.input_state.last_theme = Some(theme);
    p.flags.remove(PaneFlags::THEMECHANGED);
    match rmux_emu::input::reply::theme(theme) {
        Some(bytes) => {
            log_debug!("send_theme_update: %{} {:?} theme", p.public_id, theme);
            p.output.extend_from_slice(bytes);
        }
        None => log_debug!("send_theme_update: %{} unknown theme", p.public_id),
    }
}

/// Where the mode screen in `reset_state` comes from
/// (`server-client.c:2057-2064`).
#[derive(Clone, Copy)]
enum ModeScreen {
    Menu(WindowId),
    Pane(PaneId),
    Status,
    None,
}

/// `server_client_prompt_cursor` (`server-client.c:1993-2026`). Returns
/// `(handled, mode, cx, cy)`.
fn prompt_cursor(
    server: &Server,
    c: ClientId,
    wp: PaneId,
    mut mode: ScreenMode,
    mut cx: u32,
    mut cy: u32,
) -> (bool, ScreenMode, u32, u32) {
    let Some(p) = server.panes.get(wp) else {
        return (false, mode, cx, cy);
    };
    if p.prompt.is_none() {
        return (false, mode, cx, cy);
    }
    mode.remove(ScreenMode::CURSOR);

    let Some(tty) = server.clients.get(c).and_then(|c| c.tty.as_ref()) else {
        return (true, mode, cx, cy);
    };
    let (_, ox, oy, sx, sy) = tty.window_offset();
    let py = if status_at_line(server, c) == 0 {
        p.yoff
    } else {
        p.yoff + p.sy as i32 - 1
    };
    let px = p.xoff + p.prompt_cx as i32;
    if px < ox as i32 || px > (ox + sx) as i32 || py < oy as i32 || py > (oy + sy) as i32 {
        return (true, mode, cx, cy);
    }

    cx = (px - ox as i32) as u32;
    cy = (py - oy as i32) as u32;

    let mut r = VisibleRanges::default();
    window_visible_ranges(server, Some(wp), cx as i32, cy as i32, 1, &mut r, true);
    if window_position_is_visible(Some(&r), cx) {
        if status_at_line(server, c) == 0 {
            cy += status_line_size(server, c);
        }
        mode.insert(ScreenMode::CURSOR);
    }
    (true, mode, cx, cy)
}

/// `server_client_reset_state` (`server-client.c:2037-2178`).
///
/// The scroll region and attributes are cleared when idle (waiting for an
/// event) as this is the most likely time a user may interrupt tmux.
pub fn reset_state(server: &mut Server, c: ClientId) {
    if crate::tsp::broker::native_client(server, c) {
        return;
    }
    let Some(client) = server.clients.get(c) else {
        return;
    };
    if client
        .flags
        .intersects(ClientFlags::CONTROL | ClientFlags::SUSPENDED)
    {
        return;
    }
    let Some(tty) = client.tty.as_ref() else {
        return;
    };
    let Some(session) = client.session else {
        return;
    };
    let Some(w) = current_window(server, c) else {
        return;
    };
    let Some(window) = server.windows.get(w) else {
        return;
    };
    let wp = window.active;
    let sb_pos = window.sb_pos;
    let oo = server.sessions.get(session).map(|s| s.options);
    let has_prompt = client.prompt.is_some();
    let tty_mode = tty.mode();
    let tty_offset = tty.window_offset();

    // Disable the block flag.
    let flags = tty.flags() & TtyFlags::BLOCK;

    // Get mode from the menu if any, else from the screen.
    let mut cx = 0u32;
    let mut cy = 0u32;
    let mut mode = ScreenMode(0);
    let mut source = ModeScreen::None;
    if let Some(menu) = window.menu.as_ref() {
        (cx, cy) = menu_get_cursor(menu);
        mode = menu_screen(menu).mode;
        source = ModeScreen::Menu(w);
    } else if let (Some(wp), false) = (wp, has_prompt) {
        if let Some(p) = server.panes.get(wp) {
            mode = p.screen().mode;
            source = ModeScreen::Pane(wp);
        }
    } else {
        mode = client.status.active().mode;
        source = ModeScreen::Status;
    }
    if rmux_util::log::enabled() {
        log_debug!(
            "reset_state: client {} mode {}",
            String::from_utf8_lossy(client.name_bytes()),
            rmux_emu::screen::mode_to_string(mode)
        );
    }

    // Reset region and margin.
    {
        let Server { clients, tparm, .. } = server;
        if let Some(tty) = clients.get_mut(c).and_then(|c| c.tty.as_mut()) {
            tty.flags_mut().remove(TtyFlags::BLOCK);
            tty.region_off(tparm);
            tty.margin_off(tparm);
        }
    }

    // Move cursor to pane cursor and offset.
    let mut prompt = false;
    let mut pane_mode = ScreenMode(0);
    let has_menu = matches!(source, ModeScreen::Menu(_));
    if has_prompt {
        prompt = true;
        (cx, cy) = status_prompt_cursor(server, c);
    } else if let Some(wp) = wp {
        if has_menu {
            let (_, ox, oy, sx, sy) = tty_offset;
            if cx < ox || cx >= ox + sx || cy < oy || cy >= oy + sy {
                mode.remove(ScreenMode::CURSOR);
            } else {
                cx -= ox;
                cy -= oy;
                if status_at_line(server, c) == 0 {
                    cy += status_line_size(server, c);
                }
            }
            prompt = true;
        } else {
            (prompt, mode, cx, cy) = prompt_cursor(server, c, wp, mode, cx, cy);
        }
        if !prompt {
            let mut cursor = false;
            // Copy the pane geometry first: status_at_line/status_line_size
            // and the visibility helpers take the server.
            let geometry = server.panes.get(wp).map(|p| {
                let s = p.screen();
                (
                    p.xoff,
                    p.yoff,
                    p.sx,
                    s.cx,
                    s.cy,
                    s.mode,
                    p.scrollbar_style.width,
                )
            });
            if let Some((xoff, yoff, psx, scx, scy, smode, sb_width)) = geometry {
                pane_mode = smode;

                let (_, ox, oy, sx, sy) = tty_offset;
                let (ox, oy, sx, sy) = (ox as i32, oy as i32, sx as i32, sy as i32);
                if xoff + scx as i32 >= ox
                    && xoff + scx as i32 <= ox + sx
                    && yoff + scy as i32 >= oy
                    && yoff + scy as i32 <= oy + sy
                {
                    cursor = true;

                    cx = (xoff + scx as i32 - ox) as u32;
                    cy = (yoff + scy as i32 - oy) as u32;

                    let mut r = VisibleRanges::default();
                    window_visible_ranges(server, Some(wp), cx as i32, cy as i32, 1, &mut r, true);
                    if !window_position_is_visible(Some(&r), cx) {
                        cursor = false;
                    }

                    // window_pane_scrollbar_overlay_visible (window.c:1556-1560).
                    if pane_scrollbar_overlay(server, wp) && pane_scrollbar_visible(server, wp) {
                        let mut sb_w = sb_width.max(0) as u32;
                        if sb_w > psx {
                            sb_w = psx;
                        }
                        if sb_w != 0 && sb_pos == PaneScrollbarPosition::Left {
                            if scx < sb_w {
                                cursor = false;
                            }
                        } else if sb_w != 0 && scx >= psx - sb_w {
                            cursor = false;
                        }
                    }

                    if status_at_line(server, c) == 0 {
                        cy += status_line_size(server, c);
                    }
                }
            }

            if !cursor {
                mode.remove(ScreenMode::CURSOR);
            }
        }
    } else if matches!(source, ModeScreen::None) {
        mode.remove(ScreenMode::CURSOR);
    }
    if !pane_mode.contains(ScreenMode::SYNC) {
        log_debug!("reset_state: cursor to {},{}", cx, cy);
        let Server { clients, tparm, .. } = server;
        if let Some(tty) = clients.get_mut(c).and_then(|c| c.tty.as_mut()) {
            tty.cursor(tparm, cx, cy);
        }
    } else {
        mode.remove(ScreenMode::CURSOR_MODES);
        mode.insert(tty_mode & ScreenMode::CURSOR_MODES);
        source = ModeScreen::None;
    }

    // Set mouse mode if requested. To support dragging, always use button
    // mode. For focus-follows-mouse, we need all-motion mode to receive
    // movement events.
    if let (Some(oo), Some(window)) = (oo, server.windows.get(w)) {
        if server.options.get_number(oo, b"mouse") != 0 {
            if window.menu.is_none() {
                mode.remove(ScreenMode::ALL_MOUSE_MODES);
                for &loop_pane in &window.panes {
                    if server
                        .panes
                        .get(loop_pane)
                        .is_some_and(|p| p.screen().mode.contains(ScreenMode::MOUSE_ALL))
                    {
                        mode.insert(ScreenMode::MOUSE_ALL);
                    }
                }
            }
            if server.options.get_number(oo, b"focus-follows-mouse") != 0
                || window.sb == PaneScrollbarPolicy::Modal
                || window.sb == PaneScrollbarPolicy::Autohide
            {
                mode.insert(ScreenMode::MOUSE_ALL);
            } else if !mode.contains(ScreenMode::MOUSE_ALL) {
                mode.insert(ScreenMode::MOUSE_BUTTON);
            }
        }
    }

    // Clear bracketed paste mode if at the prompt.
    if prompt {
        mode.remove(ScreenMode::BRACKETPASTE);
    }

    // Set the terminal mode and reset attributes, then send a sync end (if
    // it was started) and restore the block flag.
    let Server {
        clients,
        tparm,
        panes,
        windows,
        ..
    } = server;
    let Some(client) = clients.get_mut(c) else {
        return;
    };
    let crate::client::Client { tty, status, .. } = client;
    let Some(tty) = tty.as_mut() else {
        return;
    };
    let screen: Option<&Screen> = match source {
        ModeScreen::Menu(w) => windows
            .get(w)
            .and_then(|w| w.menu.as_ref())
            .map(menu_screen),
        ModeScreen::Pane(wp) => panes.get(wp).map(|p| p.screen()),
        ModeScreen::Status => Some(status.active()),
        ModeScreen::None => None,
    };
    tty.update_mode(tparm, mode, screen);
    tty.reset(tparm);

    // All writing must be done, send a sync end (if it was started).
    tty.sync_end(tparm);
    tty.flags_mut().insert(flags);
}

/// `server_client_redraw_timer` (`server-client.c:2308-2313`): only wakes
/// the loop; the pending-timer bookkeeping lives in `runtime_timers`.
pub fn redraw_timer(server: &mut Server) {
    log_debug!("redraw timer fired");
    server.runtime_timers.remove(REDRAW_TIMER_KEY);
}

/// `server_client_check_modes` (`server-client.c:2319-2335`): only modes in
/// the current window are updated and only when the status line is redrawn.
pub fn check_modes(server: &mut Server, c: ClientId) {
    let Some(client) = server.clients.get(c) else {
        return;
    };
    if client
        .flags
        .intersects(ClientFlags::CONTROL | ClientFlags::SUSPENDED)
    {
        return;
    }
    if !client.flags.intersects(ClientFlags::REDRAWSTATUS) {
        return;
    }
    let Some(w) = current_window(server, c) else {
        return;
    };
    let Some(panes) = server.windows.get(w).map(|w| w.panes.clone()) else {
        return;
    };
    for wp in panes {
        if server.panes.get(wp).is_some_and(|p| !p.modes.is_empty()) {
            crate::model::pane::pane_mode_update(server, wp);
        }
    }
}

/// `server_client_any_pane_redraw` (`server-client.c:2338-2354`).
fn any_pane_redraw(server: &Server, c: ClientId, w: WindowId) -> bool {
    if server
        .clients
        .get(c)
        .is_some_and(|c| c.flags.intersects(ClientFlags::REDRAWWINDOW))
    {
        return true;
    }
    let Some(window) = server.windows.get(w) else {
        return false;
    };
    if !window.damage.is_empty() {
        return true;
    }
    window.panes.iter().any(|&wp| {
        server.panes.get(wp).is_some_and(|p| {
            p.flags
                .intersects(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR)
        })
    })
}

/// `server_client_check_redraw` (`server-client.c:2357-2478`).
pub fn check_redraw(server: &mut Server, c: ClientId) {
    if crate::tsp::broker::native_client(server, c) {
        crate::tsp::status_bar::redraw(server, c);
        return;
    }
    let Some(client) = server.clients.get(c) else {
        return;
    };
    if client
        .flags
        .intersects(ClientFlags::CONTROL | ClientFlags::SUSPENDED)
    {
        return;
    }
    let Some(tty) = client.tty.as_ref() else {
        return;
    };
    let Some(session) = client.session else {
        return;
    };
    let Some(w) = current_window(server, c) else {
        return;
    };
    let name = String::from_utf8_lossy(client.name_bytes()).into_owned();
    let cflags = client.flags;
    let mode = tty.mode();
    let damaged = server.windows.get(w).is_some_and(|w| !w.damage.is_empty());
    if cflags.intersects(ClientFlags::ALLREDRAWFLAGS) {
        log_debug!(
            "{}: redraw{}{}{}{}",
            name,
            if cflags.intersects(ClientFlags::REDRAWWINDOW) {
                " window"
            } else {
                ""
            },
            if cflags.intersects(ClientFlags::REDRAWSTATUS) {
                " status"
            } else {
                ""
            },
            if cflags.intersects(ClientFlags::REDRAWBORDERS) {
                " borders"
            } else {
                ""
            },
            if cflags.intersects(ClientFlags::REDRAWMENU) {
                " menu"
            } else {
                ""
            }
        );
    }

    // Work out if a redraw is actually needed.
    let needed = cflags.intersects(ClientFlags::ALLREDRAWFLAGS | ClientFlags::REDRAWSCROLLBARS)
        || any_pane_redraw(server, c, w);
    if !needed {
        if let Some(client) = server.clients.get_mut(c) {
            client.flags.remove(ClientFlags::STATUSFORCE);
        }
        return;
    }

    // Ignore output queued within the current synchronized frame.
    let mut n = tty.out_len();
    if tty.flags().intersects(TtyFlags::SYNCING) && n > tty.sync_offset() {
        n = tty.sync_offset();
    }

    // Defer until output drains, preserving damage in client flags.
    if n != 0 || tty.flags().intersects(TtyFlags::BLOCK) {
        if n != 0 {
            log_debug!("{}: redraw deferred ({} left)", name, n);
        } else {
            log_debug!("{}: redraw deferred (blocked)", name);
        }
        if !server.runtime_timers.contains_key(REDRAW_TIMER_KEY) {
            log_debug!("redraw timer started");
            let timer = server
                .event_loop
                .schedule(Duration::from_millis(1), LoopAction::RedrawTimer);
            server
                .runtime_timers
                .insert(REDRAW_TIMER_KEY.to_vec(), timer);
        }
        let Some(client) = server.clients.get_mut(c) else {
            return;
        };
        if damaged {
            client.flags.insert(ClientFlags::REDRAWWINDOW);
            return;
        }
        let Some(panes) = server.windows.get(w).map(|w| w.panes.as_slice()) else {
            return;
        };
        for &wp in panes {
            let Some(p) = server.panes.get(wp) else {
                continue;
            };
            if p.flags.intersects(PaneFlags::REDRAW) {
                client.flags.insert(ClientFlags::REDRAWWINDOW);
                break;
            }
            if p.flags.intersects(PaneFlags::REDRAWSCROLLBAR) {
                client.flags.insert(ClientFlags::REDRAWSCROLLBARS);
            }
        }
        return;
    }

    // Unfreeze the tty and turn off the cursor.
    log_debug!("{}: redraw needed", name);
    let tflags;
    {
        let Some(tty) = server.clients.get_mut(c).and_then(|c| c.tty.as_mut()) else {
            return;
        };
        tflags = tty.flags() & (TtyFlags::BLOCK | TtyFlags::FREEZE | TtyFlags::NOCURSOR);
        let f = tty.flags_mut();
        *f = (*f & !(TtyFlags::BLOCK | TtyFlags::FREEZE)) | TtyFlags::NOCURSOR;
    }

    // If not redrawing the entire window, check whether each pane needs to
    // be redrawn.
    if !cflags.intersects(ClientFlags::REDRAWWINDOW) {
        let panes = server
            .windows
            .get(w)
            .map(|w| w.panes.clone())
            .unwrap_or_default();
        for wp in panes {
            let Some(p) = server.panes.get(wp) else {
                continue;
            };
            let redraw_scrollbars = server
                .clients
                .get(c)
                .is_some_and(|c| c.flags.intersects(ClientFlags::REDRAWSCROLLBARS));
            if p.flags.intersects(PaneFlags::REDRAW) {
                log_debug!("check_redraw: redraw pane %{}", p.public_id);
                redraw_pane(server, c, wp);
            } else if p.flags.intersects(PaneFlags::REDRAWSCROLLBAR) || redraw_scrollbars {
                log_debug!("check_redraw: redraw scrollbar %{}", p.public_id);
                redraw_pane_scrollbar(server, c, wp);
            }
        }

        // Draw damage here if no client redraw flags will handle it.
        let all = server
            .clients
            .get(c)
            .is_some_and(|c| c.flags.intersects(ClientFlags::ALLREDRAWFLAGS));
        if damaged && !all {
            redraw_client_damage(server, c);
        }
    }

    // Set titles etc and do the redraw if there are redraw flags (and we
    // aren't here just to redraw panes).
    let all = server
        .clients
        .get(c)
        .is_some_and(|c| c.flags.intersects(ClientFlags::ALLREDRAWFLAGS));
    if all {
        let set_titles = server
            .sessions
            .get(session)
            .is_some_and(|s| server.options.get_number(s.options, b"set-titles") != 0);
        if set_titles {
            set_title(server, c);
            set_path(server, c);
        }
        set_progress_bar(server, c);
        redraw_screen(server, c);
        redraw_client_damage(server, c);
    }

    // Put the tty back how it was, clear the redraw flags and record how
    // many bytes were written.
    let Server { clients, tparm, .. } = server;
    let Some(client) = clients.get_mut(c) else {
        return;
    };
    let Some(tty) = client.tty.as_mut() else {
        return;
    };
    {
        let f = tty.flags_mut();
        *f = (*f & !TtyFlags::NOCURSOR) | (tflags & TtyFlags::NOCURSOR);
    }
    tty.update_mode(tparm, mode, None);
    {
        let f = tty.flags_mut();
        *f = (*f & !(TtyFlags::BLOCK | TtyFlags::FREEZE | TtyFlags::NOCURSOR)) | tflags;
    }
    client.flags.remove(
        ClientFlags::ALLREDRAWFLAGS | ClientFlags::REDRAWSCROLLBARS | ClientFlags::STATUSFORCE,
    );
    client.redraw = tty.out_len();
    log_debug!("{}: redraw added {} bytes", name, client.redraw);
}

/// `server_client_set_title` (`server-client.c:2481-2503`): emit only when
/// the expanded bytes changed.
pub fn set_title(server: &mut Server, c: ClientId) {
    let Some(client) = server.clients.get(c) else {
        return;
    };
    let Some(session) = client.session else {
        return;
    };
    let Some(options) = server.sessions.get(session).map(|s| s.options) else {
        return;
    };
    let template = server
        .options
        .get_string(options, b"set-titles-string")
        .to_vec();

    let mut ft = FormatTree::create(Some(c), None, 0, FormatFlags::NONE, server);
    ft.defaults(
        server,
        FormatContext {
            evaluated_client: Some(c),
            ..FormatContext::default()
        },
    );
    let title = ft.expand_time(server, &template).into_vec();
    ft.release(server);

    let Some(client) = server.clients.get_mut(c) else {
        return;
    };
    if client.title.as_deref() != Some(title.as_slice()) {
        if let Some(tty) = client.tty.as_mut() {
            tty.set_title(&title);
        }
        client.title = Some(title);
    }
}

/// `server_client_set_path` (`server-client.c:2506-2523`): the active pane's
/// base screen path, or empty bytes when absent; emit only on change.
pub fn set_path(server: &mut Server, c: ClientId) {
    let Some(w) = current_window(server, c) else {
        return;
    };
    let Some(active) = server.windows.get(w).and_then(|w| w.active) else {
        return;
    };
    let Some(p) = server.panes.get(active) else {
        return;
    };
    let path = p.base.path.clone().unwrap_or_default();
    let Some(client) = server.clients.get_mut(c) else {
        return;
    };
    if client.path.as_deref() != Some(path.as_slice()) {
        if let Some(tty) = client.tty.as_mut() {
            tty.set_path(&path);
        }
        client.path = Some(path);
    }
}

/// `server_client_set_progress_bar` (`server-client.c:2526-2540`): compare
/// state and value before copying and emitting.
pub fn set_progress_bar(server: &mut Server, c: ClientId) {
    let Some(w) = current_window(server, c) else {
        return;
    };
    let Some(active) = server.windows.get(w).and_then(|w| w.active) else {
        return;
    };
    let Some(p) = server.panes.get(active) else {
        return;
    };
    let pane_pb = p.base.progress_bar;
    let Server { clients, tparm, .. } = server;
    let Some(client) = clients.get_mut(c) else {
        return;
    };
    if pane_pb.state == client.progress_bar.state
        && pane_pb.progress == client.progress_bar.progress
    {
        return;
    }
    client.progress_bar = pane_pb;
    if let Some(tty) = client.tty.as_mut() {
        tty.set_progress_bar(tparm, &client.progress_bar);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(osx: u32, osy: u32, sx: u32, sy: u32) -> PaneResize {
        PaneResize { sx, sy, osx, osy }
    }

    #[test]
    fn empty_queue_has_no_decision() {
        assert_eq!(reduce_resizes(&VecDeque::new()), None);
    }

    #[test]
    fn single_resize_is_sent_and_removed() {
        let queue: VecDeque<PaneResize> = [r(80, 24, 100, 30)].into();
        assert_eq!(
            reduce_resizes(&queue),
            Some(ResizeDecision {
                send: (100, 30),
                keep_last: false,
                timer_ms: 250,
            })
        );
    }

    #[test]
    fn multiple_resizes_with_changed_end_size_send_last_and_clear() {
        let queue: VecDeque<PaneResize> =
            [r(80, 24, 100, 30), r(100, 30, 90, 20), r(90, 20, 120, 40)].into();
        assert_eq!(
            reduce_resizes(&queue),
            Some(ResizeDecision {
                send: (120, 40),
                keep_last: false,
                timer_ms: 250,
            })
        );
    }

    #[test]
    fn multiple_resizes_back_to_start_send_previous_and_keep_last() {
        let queue: VecDeque<PaneResize> =
            [r(80, 24, 100, 30), r(100, 30, 90, 20), r(90, 20, 80, 24)].into();
        assert_eq!(
            reduce_resizes(&queue),
            Some(ResizeDecision {
                send: (90, 20),
                keep_last: true,
                timer_ms: 10,
            })
        );
    }

    #[test]
    fn two_resizes_back_to_start_send_first_and_keep_last() {
        let queue: VecDeque<PaneResize> = [r(80, 24, 100, 30), r(100, 30, 80, 24)].into();
        assert_eq!(
            reduce_resizes(&queue),
            Some(ResizeDecision {
                send: (100, 30),
                keep_last: true,
                timer_ms: 10,
            })
        );
    }

    #[test]
    fn resize_timer_key_uses_arena_parts() {
        let pane = PaneId::from_parts(7, 2);
        assert_eq!(pane_resize_timer_key(pane), b"pane-resize-7.2".to_vec());
    }
}
