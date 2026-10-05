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

//! `server_client_update_theme_colours` and `server_client_report_theme`
//! (`server-client.c:1146-1191,3151-3180`).

use std::time::SystemTime;

use crate::client::COLOUR_THEME_COUNT;
use crate::format::{FormatContext, FormatFlags, FormatTree};
use crate::ids::ClientId;
use crate::server::Server;
use crate::server::events;
use crate::server::operations::server_redraw_client;
use rmux_emu::colour::{self, ClientTheme, Colour, ColourFlags};
use rmux_tty::tty::TtyFlags;

/// Copy the client theme and colours into the tty host view so G07 draws
/// with the same values C reads from `c->theme_colours`.
fn sync_tty_theme(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let (theme, colours) = (c.theme, c.theme_colours);
    if let Some(tty) = c.tty.as_mut() {
        let host = tty.host_mut();
        host.theme = theme;
        host.theme_colours = colours;
    }
}

/// `server_client_update_theme_colours` (`server-client.c:1146-1191`).
/// Option `theme` 1 takes the terminal theme defaults; otherwise each theme
/// colour option is expanded (no jobs) and kept unless invalid or itself a
/// theme colour.
pub fn update_theme_colours(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    let option = server.options.get_number(server.options.global, b"theme");
    if option == 1 {
        let c = server.clients.get_mut(id).expect("client checked above");
        for (i, slot) in c.theme_colours.iter_mut().enumerate() {
            *slot = colour::theme_terminal_colour(i as u32).0;
        }
        sync_tty_theme(server, id);
        return;
    }

    let mut theme = c.theme;
    if theme == ClientTheme::Unknown {
        let bg = c.tty.as_ref().map_or(-1, |tty| tty.reported_colours().1);
        theme = Colour(bg).theme();
    }
    if option == 2 {
        theme = ClientTheme::Light;
    } else if option == 3 {
        theme = ClientTheme::Dark;
    }
    let session = c.session;

    let mut ft = FormatTree::create(Some(id), None, 0, FormatFlags::NOJOBS, server);
    ft.defaults(
        server,
        FormatContext {
            evaluated_client: Some(id),
            session,
            ..FormatContext::default()
        },
    );
    let mut colours = [8i32; COLOUR_THEME_COUNT];
    for (i, slot) in colours.iter_mut().enumerate() {
        let Some(name) = colour::theme_option(i as u32, theme) else {
            continue;
        };
        let value = server
            .options
            .get_string(server.options.global, name.as_bytes())
            .to_vec();
        let expanded = ft.expand(server, &value);
        let Ok(colour) = colour::parse_colour(&expanded) else {
            continue;
        };
        if colour == Colour::NONE || colour.0 & ColourFlags::THEME.bits() as i32 != 0 {
            continue;
        }
        *slot = colour.0;
    }
    ft.release(server);

    if let Some(c) = server.clients.get_mut(id) {
        c.theme_colours = colours;
    }
    sync_tty_theme(server, id);
}

/// `server_client_report_theme` (`server-client.c:3151-3180`): fire the
/// light/dark event on every report; only a changed theme recomputes
/// colours, invalidates an opened tty and redraws; always repeat the
/// foreground and background requests.
pub fn report_theme(server: &mut Server, id: ClientId, theme: ClientTheme) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let old = c.theme;
    if theme == ClientTheme::Light {
        c.theme = ClientTheme::Light;
        events::fire_client(server, b"client-light-theme", id);
    } else {
        c.theme = ClientTheme::Dark;
        events::fire_client(server, b"client-dark-theme", id);
    }

    let Some(c) = server.clients.get(id) else {
        return;
    };
    if c.theme != old {
        update_theme_colours(server, id);
        let Server { clients, tparm, .. } = server;
        if let Some(tty) = clients.get_mut(id).and_then(|c| c.tty.as_mut())
            && tty.flags().contains(TtyFlags::OPENED)
        {
            tty.invalidate(tparm);
        }
        server_redraw_client(server, id);
    }

    if let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) {
        tty.repeat_requests(true, SystemTime::now());
    }
}

/// `window_pane_get_theme` (`window.c:2810-2850`): prefer a theme reported
/// by an attached client whose session links the pane's window; when they
/// disagree or none reported, guess from the pane background colour.
pub fn pane_theme(server: &Server, pane: crate::ids::PaneId) -> ClientTheme {
    let Some(window) = server.panes.get(pane).map(|p| p.window) else {
        return ClientTheme::Unknown;
    };
    let (mut found_light, mut found_dark) = (false, false);
    for id in &server.client_order {
        let Some(c) = server.clients.get(*id) else {
            continue;
        };
        if c.flags
            .intersects(crate::client::ClientFlags::UNATTACHEDFLAGS)
        {
            continue;
        }
        let Some(session) = c.session else {
            continue;
        };
        if !crate::model::session::session_has(server, session, window) {
            continue;
        }
        match c.theme {
            ClientTheme::Light => found_light = true,
            ClientTheme::Dark => found_dark = true,
            ClientTheme::Unknown => {}
        }
    }
    if found_dark && !found_light {
        return ClientTheme::Dark;
    }
    if found_light && !found_dark {
        return ClientTheme::Light;
    }
    Colour(pane_get_bg(server, pane)).theme()
}

/// `window_pane_get_bg` (`window.c:2714-2728`): a control client's reported
/// background, else the pane style background from the cached style cells
/// (`tty_default_colours`, `tty.c:3004-3012`), else the first attached
/// client's reported terminal background (`window_get_bg_client`,
/// `window.c:2731-2747`).
pub fn pane_get_bg(server: &Server, pane: crate::ids::PaneId) -> i32 {
    let Some(p) = server.panes.get(pane) else {
        return -1;
    };
    if let Some(c) = pane_bg_control_client(server, pane) {
        return c;
    }
    let active = server
        .windows
        .get(p.window)
        .is_some_and(|w| w.active == Some(pane));
    let bg = if active && p.cached_active_gc.bg != Colour::DEFAULT {
        p.cached_active_gc.bg
    } else {
        p.cached_gc.bg
    };
    if bg.is_default() {
        window_get_bg_client(server, p.window)
    } else {
        bg.0
    }
}

/// `window_get_bg_client` (`window.c:2731-2747`).
fn window_get_bg_client(server: &Server, window: crate::ids::WindowId) -> i32 {
    for id in &server.client_order {
        let Some(c) = server.clients.get(*id) else {
            continue;
        };
        if c.flags
            .intersects(crate::client::ClientFlags::UNATTACHEDFLAGS)
        {
            continue;
        }
        let Some(session) = c.session else {
            continue;
        };
        if !crate::model::session::session_has(server, session, window) {
            continue;
        }
        let Some(bg) = c.tty.as_ref().map(|t| t.reported_colours().1) else {
            continue;
        };
        if bg == -1 {
            continue;
        }
        return bg;
    }
    -1
}

/// `window_pane_get_bg_control_client` (`window.c:2753-2767`).
fn pane_bg_control_client(server: &Server, pane: crate::ids::PaneId) -> Option<i32> {
    let bg = server.panes.get(pane)?.control_bg;
    if bg == -1 {
        return None;
    }
    server
        .client_order
        .iter()
        .filter_map(|id| server.clients.get(*id))
        .any(|c| c.flags.intersects(crate::client::ClientFlags::CONTROL))
        .then_some(bg)
}
