// Ported from tmux style.c (style_apply, style_add) and options.c
// (options_string_to_style with a format tree) @ 8f25579c
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

use crate::format::{FormatContext, FormatFlags, FormatTree};
use crate::ids::{ClientId, OptionsId, PaneId, QueueItemId, SessionId, WinlinkId};
use crate::model::Server;
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::style::Style;

/// options_string_to_style with an optional format tree: a value containing
/// `#{` is expanded through `ft` and parsed without caching.
pub fn option_style(
    srv: &mut Server,
    oo: OptionsId,
    name: &[u8],
    ft: Option<&mut FormatTree>,
) -> Option<Style> {
    let raw = {
        let (_, o) = srv.options.get(oo, name)?;
        if !o.is_string() || o.cached_style().is_some() {
            None
        } else {
            let s = o.value().as_string();
            if s.windows(2).any(|w| w == b"#{") {
                Some(s.to_vec())
            } else {
                None
            }
        }
    };
    match (raw, ft) {
        (Some(s), Some(ft)) => {
            let expanded = ft.expand(srv, &s);
            let mut style = Style::from_cell(DEFAULT_CELL);
            style
                .parse(&DEFAULT_CELL, &expanded, &mut srv.hyperlinks)
                .ok()
                .map(|_| style)
        }
        _ => srv
            .options
            .string_to_style(oo, name, None, &mut srv.hyperlinks),
    }
}

/// style_add: overlay the option style on `gc`. Without a tree, a temporary
/// empty NOJOBS tree is used, as in C.
pub fn style_add(
    srv: &mut Server,
    gc: &mut GridCell,
    oo: OptionsId,
    name: &[u8],
    ft: Option<&mut FormatTree>,
) {
    let sy = match ft {
        Some(ft) => option_style(srv, oo, name, Some(ft)),
        None => {
            let mut tmp = FormatTree::create(None, None, 0, FormatFlags::NOJOBS, srv);
            let sy = option_style(srv, oo, name, Some(&mut tmp));
            tmp.release(srv);
            sy
        }
    }
    .unwrap_or_else(Style::option_fallback);
    sy.overlay_cell(gc);
}

/// style_apply: reset to the default cell, then style_add.
pub fn style_apply(
    srv: &mut Server,
    gc: &mut GridCell,
    oo: OptionsId,
    name: &[u8],
    ft: Option<&mut FormatTree>,
) {
    *gc = DEFAULT_CELL;
    style_add(srv, gc, oo, name, ft);
}

/// format_create_defaults(item, c, s, wl, wp).
pub fn create_defaults(
    srv: &mut Server,
    item: Option<QueueItemId>,
    c: Option<ClientId>,
    s: Option<SessionId>,
    wl: Option<WinlinkId>,
    wp: Option<PaneId>,
) -> FormatTree {
    let w = wl.and_then(|wl| srv.winlinks.get(wl).map(|l| l.window));
    crate::format::create_defaults(
        srv,
        item,
        FormatContext {
            evaluated_client: c,
            session: s,
            winlink: wl,
            window: w,
            pane: wp,
            ..FormatContext::default()
        },
    )
}

/// The session's options, falling back to the global session options when
/// the client has no session.
pub fn session_options(srv: &Server, c: ClientId) -> OptionsId {
    srv.clients
        .get(c)
        .and_then(|c| c.session)
        .and_then(|s| srv.sessions.get(s))
        .map_or(srv.options.global_s, |s| s.options)
}

/// The window of the client's current session window, if any.
pub fn client_window(srv: &Server, c: ClientId) -> Option<crate::ids::WindowId> {
    let s = srv.sessions.get(srv.clients.get(c)?.session?)?;
    srv.winlinks.get(s.current?).map(|wl| wl.window)
}

/// The client's current winlink, if any.
pub fn client_winlink(srv: &Server, c: ClientId) -> Option<WinlinkId> {
    srv.sessions.get(srv.clients.get(c)?.session?)?.current
}

/// Terminal size of a client: the tty size, or the recorded size before a
/// tty exists.
pub fn client_size(srv: &Server, c: ClientId) -> (u32, u32) {
    let Some(client) = srv.clients.get(c) else {
        return (0, 0);
    };
    match &client.tty {
        Some(tty) => tty.size(),
        None => (client.tty_sx, client.tty_sy),
    }
}
