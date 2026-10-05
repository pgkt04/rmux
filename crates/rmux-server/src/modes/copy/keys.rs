// Ported from tmux window-copy.c @ 8f25579c
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

//! Key-table selection and the command dispatch transaction
//! (`window-copy.c:1246-1254,3893-3979`).

use super::commands::{self, CopyCommandAction, CopyCommandContext, CopyMarkClear};
use super::motion;
use super::mouse;
use super::render;
use super::search;
use super::state::{self, ModeKeys};
use crate::client::ClientFlags;
use crate::cmd::arguments::Args;
use crate::cmd::queue::QueueEvent;
use crate::ids::{ClientId, ModeId, SessionId, WinlinkId};
use crate::model::Server;
use crate::model::pane::pane_reset_mode;
use crate::ui::status::status_message_set;
use rmux_util::key::KeyCodeType;

/// `window_copy_key_table`: reads `mode-keys` on every call.
pub fn key_table(server: &Server, mode: ModeId) -> &'static [u8] {
    if motion::mode_keys(server, mode) == ModeKeys::Vi {
        b"copy-mode-vi"
    } else {
        b"copy-mode"
    }
}

/// `MOUSE_WHEEL(m->b)` for the queue event that carried the command.
fn is_wheel(event: &QueueEvent) -> bool {
    event.key.is_type(KeyCodeType::Wheelup) || event.key.is_type(KeyCodeType::Wheeldown)
}

/// `window_copy_command`: the dispatch transaction.
pub fn command(
    server: &mut Server,
    mode: ModeId,
    client: Option<ClientId>,
    session: Option<SessionId>,
    winlink: Option<WinlinkId>,
    args: &Args,
    event: Option<&QueueEvent>,
) {
    if args.count() == 0 {
        return;
    }
    let Some(name) = args.string(0).map(<[u8]>::to_vec) else {
        return;
    };
    if let Some(event) = event
        && event.mouse.valid
        && !is_wheel(event)
    {
        mouse::move_mouse(server, &event.mouse);
    }

    let mut cs = CopyCommandContext {
        mode,
        args,
        wargs: Args::create(),
        mouse: event.map(|e| e.mouse),
        client,
        session,
        winlink,
    };
    let mut action = CopyCommandAction::Move;
    let mut clear = CopyMarkClear::Never;
    if let Some(spec) = commands::lookup(&name) {
        if let Some(c) = client
            && server
                .clients
                .get(c)
                .is_some_and(|c| c.flags.intersects(ClientFlags::READONLY))
            && !spec.read_only
        {
            status_message_set(
                server,
                Some(c),
                -1,
                true,
                false,
                false,
                b"client is read-only",
            );
            return;
        }
        // Parse errors are discarded, like the freed `error` in C.
        if let Ok(Some(wargs)) = spec.args.parse(args.values()) {
            cs.wargs = wargs;
            clear = spec.clear;
            action = (spec.handler)(server, &mut cs);
        }
    }

    let has_marks = state::data(server, mode).is_some_and(|d| d.search.marks.is_some());
    if !name.starts_with(b"search-") && has_marks {
        if clear == CopyMarkClear::EmacsOnly && motion::mode_keys(server, mode) == ModeKeys::Vi {
            clear = CopyMarkClear::Never;
        }
        if clear != CopyMarkClear::Never {
            search::clear_marks(server, mode);
            if let Some(data) = state::data_mut(server, mode) {
                data.search.x = None;
                data.search.y = None;
            }
        }
        if action == CopyCommandAction::Move {
            action = CopyCommandAction::Redraw;
        }
    }
    commands::set_prefix(server, mode, 1);

    match action {
        CopyCommandAction::Cancel => {
            let _ = pane_reset_mode(server, mode.owner);
        }
        CopyCommandAction::Redraw => render::redraw_screen(server, mode),
        // A cursor move or similar trivial change only redraws the first
        // line to update the indicator.
        CopyCommandAction::Move => render::redraw_lines(server, mode, 0, 1),
        CopyCommandAction::Nothing => {}
    }
}
