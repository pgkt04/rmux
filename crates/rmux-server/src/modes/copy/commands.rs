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

//! The copy-mode command table, its handlers and the copy, pipe and append
//! effects (`window-copy.c:1256-3054,3165-3891,6096-6215`).

use super::motion;
use super::mouse;
use super::render;
use super::search;
use super::select;
use super::state::{
    self, CursorDrag, JumpKind, LineSelectionDirection, ModeKeys, RecentreState, SearchDirection,
    SelectionMode,
};
use crate::cmd::arguments::{Args, ArgsParse};
use crate::cmd::find::MouseInput;
use crate::format::FormatContext;
use crate::ids::{ClientId, ModeId, SessionId, WinlinkId};
use crate::model::paste::{
    paste_add, paste_buffer_data, paste_buffer_name, paste_get_top, paste_set,
};
use crate::model::{PaneFlags, Server};
use crate::server::events::fire_pane;
use crate::server::job::{self, JobCommand, JobFlags, JobLaunch};
use crate::ui::fanout::pane_set_selection;
use rmux_emu::grid::GridLineFlags;
use rmux_emu::grid::reader::{GridReader, WHITESPACE};

/// `enum window_copy_cmd_action`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyCommandAction {
    Nothing,
    Move,
    Redraw,
    Cancel,
}

/// `enum window_copy_cmd_clear`: search-mark clearing after a command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyMarkClear {
    Always,
    Never,
    EmacsOnly,
}

/// `struct window_copy_cmd_state`.
pub struct CopyCommandContext<'a> {
    pub mode: ModeId,
    /// The outer `send-keys` arguments (`cs->args`).
    pub args: &'a Args,
    /// The arguments parsed with the command rule (`cs->wargs`).
    pub wargs: Args,
    pub mouse: Option<MouseInput>,
    pub client: Option<ClientId>,
    pub session: Option<SessionId>,
    pub winlink: Option<WinlinkId>,
}

pub type CopyCommandHandler = fn(&mut Server, &mut CopyCommandContext<'_>) -> CopyCommandAction;

/// One row of `window_copy_cmd_table`.
pub struct CopyCommandSpec {
    pub name: &'static [u8],
    pub args: ArgsParse,
    pub read_only: bool,
    pub clear: CopyMarkClear,
    pub handler: CopyCommandHandler,
}

use CopyCommandAction::{Cancel, Move, Nothing, Redraw};
use CopyMarkClear::{Always, EmacsOnly, Never};

const fn rule(template: &'static [u8], lower: i32, upper: i32) -> ArgsParse {
    ArgsParse {
        template,
        lower,
        upper,
        cb: None,
    }
}

const fn row(
    name: &'static [u8],
    args: ArgsParse,
    read_only: bool,
    clear: CopyMarkClear,
    handler: CopyCommandHandler,
) -> CopyCommandSpec {
    CopyCommandSpec {
        name,
        args,
        read_only,
        clear,
        handler,
    }
}

const NONE: ArgsParse = rule(b"", 0, 0);
const CP01: ArgsParse = rule(b"CP", 0, 1);
const CP02: ArgsParse = rule(b"CP", 0, 2);
const ONE: ArgsParse = rule(b"", 1, 1);
const OPT1: ArgsParse = rule(b"", 0, 1);

/// `window_copy_cmd_table` (`window-copy.c:3288-3891`), in source order.
pub static COMMAND_TABLE: [CopyCommandSpec; 99] = [
    row(
        b"append-selection",
        NONE,
        false,
        Always,
        cmd_append_selection,
    ),
    row(
        b"append-selection-and-cancel",
        NONE,
        false,
        Always,
        cmd_append_selection_and_cancel,
    ),
    row(
        b"back-to-indentation",
        NONE,
        true,
        Always,
        cmd_back_to_indentation,
    ),
    row(b"begin-selection", NONE, false, Always, cmd_begin_selection),
    row(b"bottom-line", NONE, true, EmacsOnly, cmd_bottom_line),
    row(b"cancel", NONE, true, Always, cmd_cancel),
    row(b"clear-selection", NONE, false, Always, cmd_clear_selection),
    row(
        b"copy-end-of-line",
        CP01,
        false,
        Always,
        cmd_copy_end_of_line,
    ),
    row(
        b"copy-end-of-line-and-cancel",
        CP01,
        false,
        Always,
        cmd_copy_end_of_line_and_cancel,
    ),
    row(
        b"copy-pipe-end-of-line",
        CP02,
        false,
        Always,
        cmd_copy_pipe_end_of_line,
    ),
    row(
        b"copy-pipe-end-of-line-and-cancel",
        CP02,
        false,
        Always,
        cmd_copy_pipe_end_of_line_and_cancel,
    ),
    row(b"copy-line", CP01, false, Always, cmd_copy_line),
    row(
        b"copy-line-and-cancel",
        CP01,
        false,
        Always,
        cmd_copy_line_and_cancel,
    ),
    row(b"copy-pipe-line", CP02, false, Always, cmd_copy_pipe_line),
    row(
        b"copy-pipe-line-and-cancel",
        CP02,
        false,
        Always,
        cmd_copy_pipe_line_and_cancel,
    ),
    row(
        b"copy-pipe-no-clear",
        CP02,
        false,
        Never,
        cmd_copy_pipe_no_clear,
    ),
    row(b"copy-pipe", CP02, false, Always, cmd_copy_pipe),
    row(
        b"copy-pipe-and-cancel",
        CP02,
        false,
        Always,
        cmd_copy_pipe_and_cancel,
    ),
    row(
        b"copy-selection-no-clear",
        CP01,
        false,
        Never,
        cmd_copy_selection_no_clear,
    ),
    row(b"copy-selection", CP01, false, Always, cmd_copy_selection),
    row(
        b"copy-selection-and-cancel",
        CP01,
        false,
        Always,
        cmd_copy_selection_and_cancel,
    ),
    row(b"cursor-down", NONE, true, EmacsOnly, cmd_cursor_down),
    row(
        b"cursor-down-and-cancel",
        NONE,
        true,
        Always,
        cmd_cursor_down_and_cancel,
    ),
    row(b"cursor-left", NONE, true, EmacsOnly, cmd_cursor_left),
    row(b"cursor-right", NONE, true, EmacsOnly, cmd_cursor_right),
    row(b"cursor-up", NONE, true, EmacsOnly, cmd_cursor_up),
    row(
        b"cursor-centre-vertical",
        NONE,
        true,
        EmacsOnly,
        cmd_centre_vertical,
    ),
    row(
        b"cursor-centre-horizontal",
        NONE,
        true,
        EmacsOnly,
        cmd_centre_horizontal,
    ),
    row(b"end-of-line", NONE, true, EmacsOnly, cmd_end_of_line),
    row(b"goto-line", ONE, true, EmacsOnly, cmd_goto_line),
    row(b"halfpage-down", NONE, true, EmacsOnly, cmd_halfpage_down),
    row(
        b"halfpage-down-and-cancel",
        NONE,
        true,
        Always,
        cmd_halfpage_down_and_cancel,
    ),
    row(b"halfpage-up", NONE, true, EmacsOnly, cmd_halfpage_up),
    row(b"history-bottom", NONE, true, EmacsOnly, cmd_history_bottom),
    row(b"history-top", NONE, true, EmacsOnly, cmd_history_top),
    row(b"jump-again", NONE, false, EmacsOnly, cmd_jump_again),
    row(b"jump-backward", ONE, false, EmacsOnly, cmd_jump_backward),
    row(b"jump-forward", ONE, false, EmacsOnly, cmd_jump_forward),
    row(b"jump-reverse", NONE, false, EmacsOnly, cmd_jump_reverse),
    row(
        b"jump-to-backward",
        ONE,
        false,
        EmacsOnly,
        cmd_jump_to_backward,
    ),
    row(
        b"jump-to-forward",
        ONE,
        false,
        EmacsOnly,
        cmd_jump_to_forward,
    ),
    row(b"jump-to-mark", NONE, true, Always, cmd_jump_to_mark),
    row(b"line-numbers-on", NONE, true, Never, cmd_line_numbers_on),
    row(b"line-numbers-off", NONE, true, Never, cmd_line_numbers_off),
    row(
        b"line-numbers-toggle",
        NONE,
        true,
        Never,
        cmd_line_numbers_toggle,
    ),
    row(
        b"next-prompt",
        rule(b"o", 0, 0),
        true,
        Always,
        cmd_next_prompt,
    ),
    row(
        b"previous-prompt",
        rule(b"o", 0, 0),
        true,
        Always,
        cmd_previous_prompt,
    ),
    row(b"middle-line", NONE, true, EmacsOnly, cmd_middle_line),
    row(
        b"next-matching-bracket",
        NONE,
        true,
        Always,
        cmd_next_matching_bracket,
    ),
    row(b"next-paragraph", NONE, true, EmacsOnly, cmd_next_paragraph),
    row(b"next-space", NONE, true, EmacsOnly, cmd_next_space),
    row(b"next-space-end", NONE, true, EmacsOnly, cmd_next_space_end),
    row(b"next-word", NONE, true, EmacsOnly, cmd_next_word),
    row(b"next-word-end", NONE, true, EmacsOnly, cmd_next_word_end),
    row(b"other-end", NONE, false, EmacsOnly, cmd_other_end),
    row(b"page-down", NONE, true, EmacsOnly, cmd_page_down),
    row(
        b"page-down-and-cancel",
        NONE,
        true,
        Always,
        cmd_page_down_and_cancel,
    ),
    row(b"page-up", NONE, true, EmacsOnly, cmd_page_up),
    row(b"pipe-no-clear", OPT1, false, Never, cmd_pipe_no_clear),
    row(b"pipe", OPT1, false, Always, cmd_pipe),
    row(b"pipe-and-cancel", OPT1, false, Always, cmd_pipe_and_cancel),
    row(
        b"previous-matching-bracket",
        NONE,
        true,
        Always,
        cmd_previous_matching_bracket,
    ),
    row(
        b"previous-paragraph",
        NONE,
        true,
        EmacsOnly,
        cmd_previous_paragraph,
    ),
    row(b"previous-space", NONE, true, EmacsOnly, cmd_previous_space),
    row(b"previous-word", NONE, true, EmacsOnly, cmd_previous_word),
    row(
        b"recentre-top-bottom",
        NONE,
        true,
        Always,
        cmd_recentre_top_bottom,
    ),
    row(b"rectangle-on", NONE, false, Always, cmd_rectangle_on),
    row(b"rectangle-off", NONE, false, Always, cmd_rectangle_off),
    row(
        b"rectangle-toggle",
        NONE,
        false,
        Always,
        cmd_rectangle_toggle,
    ),
    row(b"refresh-on", NONE, true, Never, cmd_refresh_on),
    row(b"refresh-off", NONE, true, Never, cmd_refresh_off),
    row(b"refresh-now", NONE, true, Never, cmd_refresh_now),
    row(b"refresh-toggle", NONE, true, Never, cmd_refresh_toggle),
    row(b"scroll-bottom", NONE, true, Always, cmd_scroll_bottom),
    row(b"scroll-down", NONE, true, EmacsOnly, cmd_scroll_down),
    row(
        b"scroll-down-and-cancel",
        NONE,
        true,
        Always,
        cmd_scroll_down_and_cancel,
    ),
    row(b"scroll-exit-on", NONE, false, Always, cmd_scroll_exit_on),
    row(b"scroll-exit-off", NONE, false, Always, cmd_scroll_exit_off),
    row(
        b"scroll-exit-toggle",
        NONE,
        false,
        Always,
        cmd_scroll_exit_toggle,
    ),
    row(b"scroll-middle", NONE, true, Always, cmd_scroll_middle),
    row(
        b"scroll-to-mouse",
        rule(b"e", 0, 0),
        true,
        EmacsOnly,
        cmd_scroll_to_mouse,
    ),
    row(b"scroll-top", NONE, true, Always, cmd_scroll_top),
    row(b"scroll-up", NONE, true, EmacsOnly, cmd_scroll_up),
    row(b"search-again", NONE, false, Always, cmd_search_again),
    row(b"search-backward", OPT1, false, Always, cmd_search_backward),
    row(
        b"search-backward-text",
        OPT1,
        false,
        Always,
        cmd_search_backward_text,
    ),
    row(
        b"search-backward-incremental",
        ONE,
        false,
        Always,
        cmd_search_backward_incremental,
    ),
    row(b"search-forward", OPT1, false, Always, cmd_search_forward),
    row(
        b"search-forward-text",
        OPT1,
        false,
        Always,
        cmd_search_forward_text,
    ),
    row(
        b"search-forward-incremental",
        ONE,
        false,
        Always,
        cmd_search_forward_incremental,
    ),
    row(b"search-reverse", NONE, false, Always, cmd_search_reverse),
    row(b"select-line", NONE, false, Always, cmd_select_line),
    row(b"select-word", NONE, false, Always, cmd_select_word),
    row(b"selection-mode", OPT1, false, Always, cmd_selection_mode),
    row(b"set-mark", NONE, true, Always, cmd_set_mark),
    row(b"start-of-line", NONE, true, EmacsOnly, cmd_start_of_line),
    row(b"stop-selection", NONE, false, Always, cmd_stop_selection),
    row(b"toggle-position", NONE, true, Never, cmd_toggle_position),
    row(b"top-line", NONE, true, EmacsOnly, cmd_top_line),
];

pub fn lookup(name: &[u8]) -> Option<&'static CopyCommandSpec> {
    COMMAND_TABLE.iter().find(|spec| spec.name == name)
}

/// `wme->prefix`.
pub fn prefix(server: &Server, mode: ModeId) -> u32 {
    server
        .panes
        .get(mode.owner)
        .and_then(|p| p.modes.iter().find(|m| m.id == mode))
        .map_or(1, |m| m.prefix)
}

pub fn set_prefix(server: &mut Server, mode: ModeId, prefix: u32) {
    if let Some(m) = server
        .panes
        .get_mut(mode.owner)
        .and_then(|p| p.modes.iter_mut().find(|m| m.id == mode))
    {
        m.prefix = prefix;
    }
}

fn visible_sy(server: &Server, mode: ModeId) -> u32 {
    state::screen(server, mode).map_or(0, |s| s.grid.sy())
}

fn pane_size(server: &Server, mode: ModeId) -> (u32, u32) {
    server
        .panes
        .get(mode.owner)
        .map_or((0, 0), |p| (p.sx, p.sy))
}

/// `format_single(NULL, fmt, c, s, wl, wp)`.
fn format_single(server: &mut Server, cs: &CopyCommandContext<'_>, input: &[u8]) -> Vec<u8> {
    let window = cs
        .winlink
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.window);
    let context = FormatContext {
        evaluated_client: cs.client,
        session: cs.session,
        winlink: cs.winlink,
        window,
        pane: Some(cs.mode.owner),
        ..FormatContext::default()
    };
    crate::format::single(server, None, context, input).0
}

/// `options_get_string(cs->s->options, "word-separators")`; C dereferences
/// the session without a check, so a missing session is no effect here.
fn session_separators(server: &Server, cs: &CopyCommandContext<'_>) -> Option<Vec<u8>> {
    let options = server.sessions.get(cs.session?)?.options;
    Some(
        server
            .options
            .get_string(options, b"word-separators")
            .to_vec(),
    )
}

fn buffer_limit(server: &Server) -> u32 {
    server
        .options
        .get_number(server.options.global, b"buffer-limit") as u32
}

fn set_clipboard(server: &Server) -> bool {
    server
        .options
        .get_number(server.options.global, b"set-clipboard")
        != 0
}

/// `window_copy_copy_buffer`: clipboard before paste insertion.
pub fn copy_buffer(
    server: &mut Server,
    mode: ModeId,
    prefix: Option<&[u8]>,
    buf: Vec<u8>,
    set_paste: bool,
    set_clip: bool,
) {
    let pane = mode.owner;
    if set_clip && set_clipboard(server) {
        let mut redraw = false;
        if render::line_numbers_active(server, mode)
            && let Some(p) = server.panes.get_mut(pane)
            && p.flags.contains(PaneFlags::REDRAW)
        {
            // Clear PANE_REDRAW so the clipboard write is not skipped.
            redraw = true;
            p.flags.remove(PaneFlags::REDRAW);
        }
        pane_set_selection(server, pane, b"", &buf);
        if redraw && let Some(p) = server.panes.get_mut(pane) {
            p.flags.insert(PaneFlags::REDRAW);
        }
        fire_pane(server, b"pane-set-clipboard", pane);
    }
    if set_paste {
        let limit = buffer_limit(server);
        let _ = paste_add(server, prefix, buf, limit);
    }
}

/// `window_copy_pipe_run`: launch the job, feed it the selection, and hand
/// the serialized bytes back.
fn pipe_run(
    server: &mut Server,
    mode: ModeId,
    session: Option<SessionId>,
    cmd: Option<&[u8]>,
) -> Option<Vec<u8>> {
    let buf = select::get_selection(server, mode);
    let cmd = match cmd {
        Some(cmd) if !cmd.is_empty() => cmd.to_vec(),
        _ => server
            .options
            .get_string(server.options.global, b"copy-command")
            .to_vec(),
    };
    if !cmd.is_empty() {
        let mut launch = JobLaunch::new(JobCommand::Shell(cmd));
        launch.session = session;
        launch.flags = JobFlags::NOWAIT;
        if let Ok(id) = job::run(server, launch)
            && let Some(buf) = &buf
        {
            let _ = job::queue_input(server, id, buf.clone());
        }
    }
    buf
}

/// `window_copy_pipe`.
fn pipe(server: &mut Server, mode: ModeId, session: Option<SessionId>, cmd: Option<&[u8]>) {
    pipe_run(server, mode, session, cmd);
}

/// `window_copy_copy_pipe`.
fn copy_pipe(
    server: &mut Server,
    mode: ModeId,
    session: Option<SessionId>,
    prefix: Option<&[u8]>,
    cmd: Option<&[u8]>,
    set_paste: bool,
    set_clip: bool,
) {
    if let Some(buf) = pipe_run(server, mode, session, cmd) {
        copy_buffer(server, mode, prefix, buf, set_paste, set_clip);
    }
}

/// `window_copy_copy_selection`.
fn copy_selection(
    server: &mut Server,
    mode: ModeId,
    prefix: Option<&[u8]>,
    set_paste: bool,
    set_clip: bool,
) {
    if let Some(buf) = select::get_selection(server, mode) {
        copy_buffer(server, mode, prefix, buf, set_paste, set_clip);
    }
}

/// `window_copy_append_selection`: only the new bytes reach the clipboard;
/// the top buffer is prefixed and set under its own name.
fn append_selection(server: &mut Server, mode: ModeId) {
    let Some(buf) = select::get_selection(server, mode) else {
        return;
    };
    let pane = mode.owner;
    if set_clipboard(server) {
        pane_set_selection(server, pane, b"", &buf);
        fire_pane(server, b"pane-set-clipboard", pane);
    }
    let top = paste_get_top(server);
    let name = top.and_then(|id| paste_buffer_name(server, id).map(<[u8]>::to_vec));
    let old = top.and_then(|id| paste_buffer_data(server, id));
    let combined = match old {
        Some(old) => {
            let mut combined = Vec::with_capacity(old.len() + buf.len());
            combined.extend_from_slice(old);
            combined.extend_from_slice(&buf);
            combined
        }
        None => buf,
    };
    let limit = buffer_limit(server);
    let _ = paste_set(server, combined, name.as_deref(), limit);
}

fn cmd_append_selection(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if cs.session.is_some() {
        append_selection(server, cs.mode);
    }
    select::clear_selection(server, cs.mode);
    Redraw
}

fn cmd_append_selection_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    if cs.session.is_some() {
        append_selection(server, cs.mode);
    }
    select::clear_selection(server, cs.mode);
    Cancel
}

fn cmd_back_to_indentation(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    motion::cursor_back_to_indentation(server, cs.mode);
    Move
}

fn cmd_begin_selection(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(m) = cs.mouse {
        mouse::start_drag(server, cs.client, &m);
        return Move;
    }
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.selection.lineflag = LineSelectionDirection::None;
        data.selection.selflag = SelectionMode::Char;
    }
    select::start_selection(server, cs.mode);
    Redraw
}

fn cmd_stop_selection(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.selection.cursordrag = CursorDrag::None;
        data.selection.lineflag = LineSelectionDirection::None;
        data.selection.selflag = SelectionMode::Char;
    }
    Move
}

fn cmd_bottom_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let sy = visible_sy(server, cs.mode);
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.cx = 0;
        data.cy = sy - 1;
    }
    select::update_selection(server, cs.mode, true, false);
    Redraw
}

fn cmd_cancel(_server: &mut Server, _cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    Cancel
}

fn cmd_clear_selection(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    select::clear_selection(server, cs.mode);
    Redraw
}

/// The `-C`/`-P` flags, prefix and command of the copy-line family.
struct CopyArgs {
    prefix: Option<Vec<u8>>,
    command: Option<Vec<u8>>,
    set_paste: bool,
    set_clip: bool,
}

fn copy_args(server: &mut Server, cs: &CopyCommandContext<'_>, pipe: bool) -> CopyArgs {
    let count = cs.wargs.count();
    let arg0 = cs.wargs.string(0).map(<[u8]>::to_vec);
    let arg1 = cs.wargs.string(1).map(<[u8]>::to_vec);
    let (mut prefix, mut command) = (None, None);
    if pipe {
        if count == 2 {
            prefix = arg1.map(|a| format_single(server, cs, &a));
        }
        if cs.session.is_some()
            && count > 0
            && let Some(arg0) = &arg0
            && !arg0.is_empty()
        {
            command = Some(format_single(server, cs, arg0));
        }
    } else if count == 1 {
        prefix = arg0.map(|a| format_single(server, cs, &a));
    }
    CopyArgs {
        prefix,
        command,
        set_paste: cs.wargs.has(b'P') == 0,
        set_clip: cs.wargs.has(b'C') == 0,
    }
}

/// `window_copy_do_copy_end_of_line` and `window_copy_do_copy_line`.
fn do_copy_line(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
    whole_line: bool,
    pipe: bool,
    cancel: bool,
) -> CopyCommandAction {
    let mode = cs.mode;
    let np = prefix(server, mode);
    let copy = copy_args(server, cs, pipe);
    let Some(data) = state::data_mut(server, mode) else {
        return Move;
    };
    let (ocx, ocy, ooy) = (data.cx, data.cy, data.oy);
    if whole_line {
        data.selection.selflag = SelectionMode::Char;
        motion::cursor_start_of_line(server, mode);
    }
    select::start_selection(server, mode);
    for _ in 1..np {
        motion::cursor_down(server, mode, false);
    }
    motion::cursor_end_of_line(server, mode);
    if cs.session.is_some() {
        if pipe {
            copy_pipe(
                server,
                mode,
                cs.session,
                copy.prefix.as_deref(),
                copy.command.as_deref(),
                copy.set_paste,
                copy.set_clip,
            );
        } else {
            copy_selection(
                server,
                mode,
                copy.prefix.as_deref(),
                copy.set_paste,
                copy.set_clip,
            );
        }
        if cancel {
            return Cancel;
        }
    }
    select::clear_selection(server, mode);
    if let Some(data) = state::data_mut(server, mode) {
        data.cx = ocx;
        data.cy = ocy;
        data.oy = ooy;
    }
    Redraw
}

fn cmd_copy_end_of_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    do_copy_line(server, cs, false, false, false)
}

fn cmd_copy_end_of_line_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    do_copy_line(server, cs, false, false, true)
}

fn cmd_copy_pipe_end_of_line(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    do_copy_line(server, cs, false, true, false)
}

fn cmd_copy_pipe_end_of_line_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    do_copy_line(server, cs, false, true, true)
}

fn cmd_copy_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    do_copy_line(server, cs, true, false, false)
}

fn cmd_copy_line_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    do_copy_line(server, cs, true, false, true)
}

fn cmd_copy_pipe_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    do_copy_line(server, cs, true, true, false)
}

fn cmd_copy_pipe_line_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    do_copy_line(server, cs, true, true, true)
}

fn cmd_copy_selection_no_clear(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    let prefix = cs
        .wargs
        .string(0)
        .map(<[u8]>::to_vec)
        .map(|arg0| format_single(server, cs, &arg0));
    let set_paste = cs.wargs.has(b'P') == 0;
    let set_clip = cs.wargs.has(b'C') == 0;
    if cs.session.is_some() {
        copy_selection(server, cs.mode, prefix.as_deref(), set_paste, set_clip);
    }
    Nothing
}

fn cmd_copy_selection(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    cmd_copy_selection_no_clear(server, cs);
    select::clear_selection(server, cs.mode);
    Redraw
}

fn cmd_copy_selection_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    cmd_copy_selection_no_clear(server, cs);
    select::clear_selection(server, cs.mode);
    Cancel
}

fn cmd_cursor_down(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_down(server, cs.mode, false);
    }
    Move
}

fn cmd_cursor_down_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    let Some(cy) = state::data(server, cs.mode).map(|d| d.cy) else {
        return Move;
    };
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_down(server, cs.mode, false);
    }
    if state::data(server, cs.mode).is_some_and(|d| cy == d.cy && d.oy == 0) {
        return Cancel;
    }
    Move
}

fn cmd_cursor_left(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_left(server, cs.mode);
    }
    Move
}

fn cmd_cursor_right(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        let all = state::data(server, cs.mode)
            .is_some_and(|d| d.selection.active && d.selection.rectflag);
        motion::cursor_right(server, cs.mode, all);
    }
    Move
}

/// `window_copy_cmd_scroll_to`: scroll the line containing the cursor to
/// the given visible row.
fn cmd_scroll_to(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
    to: u32,
) -> CopyCommandAction {
    let Some(data) = state::data(server, cs.mode) else {
        return Redraw;
    };
    let scroll_up = data.cy as i64 - to as i64;
    let delta = scroll_up.unsigned_abs() as u32;
    let oy = data.backing.screen().grid.hsize() - data.oy;
    if scroll_up > 0 && data.oy >= delta {
        motion::scroll_up(server, cs.mode, delta);
        if let Some(data) = state::data_mut(server, cs.mode) {
            data.cy -= delta;
        }
    } else if scroll_up < 0 && oy >= delta {
        motion::scroll_down(server, cs.mode, delta);
        if let Some(data) = state::data_mut(server, cs.mode) {
            data.cy += delta;
        }
    }
    select::update_selection_view(server, cs.mode, false, false);
    Redraw
}

fn cmd_scroll_bottom(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let bottom = visible_sy(server, cs.mode) - 1;
    cmd_scroll_to(server, cs, bottom)
}

fn cmd_scroll_middle(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let mid = (visible_sy(server, cs.mode) - 1) / 2;
    cmd_scroll_to(server, cs, mid)
}

/// `window_copy_cmd_scroll_to_mouse`: scroll the pane to the mouse in the
/// scrollbar. The client and mouse geometry come from the caller.
fn cmd_scroll_to_mouse(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let scroll_exit = cs.wargs.has(b'e') != 0;
    let (Some(client), Some(m)) = (cs.client, cs.mouse) else {
        return Move;
    };
    let (_, _, tty_oy, _, _) = crate::client::lifecycle::window_offset(server, client);
    let sl_mpos = server
        .clients
        .get(client)
        .and_then(|c| c.drag.slider_mpos)
        .map_or(-1, |v| v as i32);
    motion::scrollbar_scroll(server, cs.mode.owner, sl_mpos, m.y, tty_oy, scroll_exit);
    Move
}

fn cmd_scroll_top(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    cmd_scroll_to(server, cs, 0)
}

fn cmd_cursor_up(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_up(server, cs.mode, false);
    }
    Move
}

fn cmd_centre_vertical(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let (_, sy) = pane_size(server, cs.mode);
    if let Some(cx) = state::data(server, cs.mode).map(|d| d.cx) {
        select::update_cursor(server, cs.mode, cx, sy / 2);
    }
    select::update_selection(server, cs.mode, true, false);
    Redraw
}

fn cmd_centre_horizontal(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    let (sx, _) = pane_size(server, cs.mode);
    if let Some(cy) = state::data(server, cs.mode).map(|d| d.cy) {
        select::update_cursor(server, cs.mode, sx / 2, cy);
    }
    select::update_selection(server, cs.mode, true, false);
    Redraw
}

fn cmd_end_of_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    motion::cursor_end_of_line(server, cs.mode);
    Move
}

fn page_down_repeat(
    server: &mut Server,
    cs: &CopyCommandContext<'_>,
    half_page: bool,
    scroll_exit: Option<bool>,
) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        let exit = scroll_exit
            .unwrap_or_else(|| state::data(server, cs.mode).is_some_and(|d| d.scroll_exit));
        if motion::pagedown1(server, cs.mode, half_page, exit) {
            return Cancel;
        }
    }
    Move
}

fn cmd_halfpage_down(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    page_down_repeat(server, cs, true, None)
}

fn cmd_halfpage_down_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    page_down_repeat(server, cs, true, Some(true))
}

fn cmd_halfpage_up(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::pageup1(server, cs.mode, true);
    }
    Move
}

fn cmd_toggle_position(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.hide_position = !data.hide_position;
    }
    Redraw
}

/// Visible-mark rescan plus selection update shared by the history and
/// top/bottom jumps.
fn history_jump_finish(server: &mut Server, cs: &CopyCommandContext<'_>, old_oy: u32) {
    let Some(data) = state::data(server, cs.mode) else {
        return;
    };
    if data.search.marks.is_some() && !data.timeout {
        search::search_marks(server, cs.mode, true);
    }
    select::update_selection(server, cs.mode, true, false);
    if state::data(server, cs.mode).is_some_and(|d| d.oy != old_oy) {
        let _ = crate::model::pane::pane_scrollbar_show(server, cs.mode.owner, true);
    }
}

fn cmd_history_bottom(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let sy = visible_sy(server, cs.mode);
    let Some(data) = state::data(server, cs.mode) else {
        return Redraw;
    };
    let old_oy = data.oy;
    let hsize = data.backing.screen().grid.hsize();
    let oy = hsize + data.cy - data.oy;
    if data.selection.lineflag == LineSelectionDirection::RightToLeft
        && oy == data.selection.endsely
    {
        select::other_end(server, cs.mode);
    }
    let cx = motion::cursor_limit(server, cs.mode, hsize + sy - 1, false);
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.cy = sy - 1;
        data.cx = cx;
        data.oy = 0;
    }
    history_jump_finish(server, cs, old_oy);
    Redraw
}

fn cmd_history_top(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let Some(data) = state::data(server, cs.mode) else {
        return Redraw;
    };
    let old_oy = data.oy;
    let hsize = data.backing.screen().grid.hsize();
    let oy = hsize + data.cy - data.oy;
    if data.selection.lineflag == LineSelectionDirection::LeftToRight && oy == data.selection.sely {
        select::other_end(server, cs.mode);
    }
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.cy = 0;
        data.cx = 0;
        data.oy = hsize;
    }
    history_jump_finish(server, cs, old_oy);
    Redraw
}

fn jump_repeat(server: &mut Server, cs: &CopyCommandContext<'_>, kind: JumpKind) {
    let np = prefix(server, cs.mode);
    let step: fn(&mut Server, ModeId) = match kind {
        JumpKind::Forward => motion::cursor_jump,
        JumpKind::Backward => motion::cursor_jump_back,
        JumpKind::ToForward => motion::cursor_jump_to,
        JumpKind::ToBackward => motion::cursor_jump_to_back,
        JumpKind::Off => return,
    };
    for _ in 0..np {
        step(server, cs.mode);
    }
}

fn cmd_jump_again(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(kind) = state::data(server, cs.mode).map(|d| d.jump.kind) {
        jump_repeat(server, cs, kind);
    }
    Move
}

fn cmd_jump_reverse(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(kind) = state::data(server, cs.mode).map(|d| d.jump.kind) {
        let reverse = match kind {
            JumpKind::Forward => JumpKind::Backward,
            JumpKind::Backward => JumpKind::Forward,
            JumpKind::ToForward => JumpKind::ToBackward,
            JumpKind::ToBackward => JumpKind::ToForward,
            JumpKind::Off => JumpKind::Off,
        };
        jump_repeat(server, cs, reverse);
    }
    Move
}

fn cmd_middle_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let sy = visible_sy(server, cs.mode);
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.cx = 0;
        data.cy = (sy - 1) / 2;
    }
    select::update_selection(server, cs.mode, true, false);
    Redraw
}

const OPEN_BRACKETS: &[u8] = b"{[(";
const CLOSE_BRACKETS: &[u8] = b"}])";

fn bracket_index(set: &[u8], c: u8) -> Option<usize> {
    set.iter().position(|&b| b == c)
}

/// One step of `window_copy_cmd_previous_matching_bracket`.
fn previous_matching_bracket_step(server: &mut Server, cs: &CopyCommandContext<'_>) -> bool {
    let emacs = motion::mode_keys(server, cs.mode) == ModeKeys::Emacs;
    let Some(data) = state::data(server, cs.mode) else {
        return false;
    };
    let mut px = data.cx;
    let mut py = data.backing.screen().grid.hsize() + data.cy - data.oy;
    let mut xx = motion::find_length(data, py);
    if xx == 0 {
        return false;
    }
    // Get the current character. If not on a bracket, try the previous. If
    // still not, then behave like previous-word.
    let mut tried = false;
    let found = loop {
        let cell = motion::single_byte(data, px, py);
        let close = cell.and_then(|c| bracket_index(CLOSE_BRACKETS, c).map(|_| c));
        match close {
            Some(c) => break Some(c),
            None => {
                if emacs {
                    if !tried && px > 0 {
                        px -= 1;
                        tried = true;
                        continue;
                    }
                    break None;
                }
                return true;
            }
        }
    };
    let Some(found) = found else {
        motion::cursor_previous_word(server, cs.mode, CLOSE_BRACKETS, true);
        return true;
    };
    let start = OPEN_BRACKETS[bracket_index(CLOSE_BRACKETS, found).expect("closing bracket")];

    // Walk backward until the matching bracket is reached.
    let mut n = 1u32;
    let mut failed = false;
    loop {
        if px == 0 {
            if py == 0 {
                failed = true;
                break;
            }
            loop {
                py -= 1;
                xx = motion::find_length(data, py);
                if !(xx == 0 && py > 0) {
                    break;
                }
            }
            if xx == 0 && py == 0 {
                failed = true;
                break;
            }
            px = xx - 1;
        } else {
            px -= 1;
        }
        if let Some(c) = motion::single_byte(data, px, py) {
            if c == found {
                n += 1;
            } else if c == start {
                n -= 1;
            }
        }
        if n == 0 {
            break;
        }
    }
    if !failed {
        search::scroll_to(server, cs.mode, px, py);
    }
    true
}

fn cmd_previous_matching_bracket(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        if !previous_matching_bracket_step(server, cs) {
            break;
        }
    }
    Move
}

/// One step of `window_copy_cmd_next_matching_bracket`; false ends the
/// repeat loop.
fn next_matching_bracket_step(server: &mut Server, cs: &CopyCommandContext<'_>) -> bool {
    let emacs = motion::mode_keys(server, cs.mode) == ModeKeys::Emacs;
    let Some(data) = state::data(server, cs.mode) else {
        return false;
    };
    let grid = &data.backing.screen().grid;
    let hsize = grid.hsize();
    let mut px = data.cx;
    let mut py = hsize + data.cy - data.oy;
    let mut xx = motion::find_length(data, py);
    let yy = hsize + grid.sy() - 1;
    if xx == 0 {
        return false;
    }
    // Get the current character. If not on a bracket, try the next. If still
    // not, then behave like next-word.
    let mut tried = false;
    let found = loop {
        let cell = motion::single_byte(data, px, py);
        if let Some(c) = cell {
            // In vi mode, attempt to move to the previous bracket if a
            // closing bracket is found first. If this fails, return to the
            // original cursor position.
            if !emacs && bracket_index(CLOSE_BRACKETS, c).is_some() {
                let (sx, sy) = (data.cx, hsize + data.cy - data.oy);
                search::scroll_to(server, cs.mode, px, py);
                previous_matching_bracket_step(server, cs);
                let Some(data) = state::data(server, cs.mode) else {
                    return false;
                };
                let (px, py) = (data.cx, hsize + data.cy - data.oy);
                if motion::single_byte(data, px, py)
                    .is_some_and(|c| bracket_index(CLOSE_BRACKETS, c).is_some())
                {
                    search::scroll_to(server, cs.mode, sx, sy);
                }
                return false;
            }
            if bracket_index(OPEN_BRACKETS, c).is_some() {
                break c;
            }
        }
        if emacs {
            if !tried && px <= xx {
                px += 1;
                tried = true;
                continue;
            }
            motion::cursor_next_word_end(server, cs.mode, OPEN_BRACKETS, false);
            return true;
        }
        // For vi, continue searching for a bracket until the end of line.
        if px > xx {
            if py == yy {
                return true;
            }
            let line = grid.get_line(py);
            if !line.flags.contains(GridLineFlags::WRAPPED) || line.cellsize() > grid.sx() {
                return true;
            }
            px = 0;
            py += 1;
            xx = motion::find_length(data, py);
        } else {
            px += 1;
        }
    };
    let end = CLOSE_BRACKETS[bracket_index(OPEN_BRACKETS, found).expect("opening bracket")];

    // Walk forward until the matching bracket is reached.
    let mut n = 1u32;
    let mut failed = false;
    loop {
        if px > xx {
            if py == yy {
                failed = true;
                break;
            }
            px = 0;
            py += 1;
            xx = motion::find_length(data, py);
        } else {
            px += 1;
        }
        if let Some(c) = motion::single_byte(data, px, py) {
            if c == found {
                n += 1;
            } else if c == end {
                n -= 1;
            }
        }
        if n == 0 {
            break;
        }
    }
    if !failed {
        search::scroll_to(server, cs.mode, px, py);
    }
    true
}

fn cmd_next_matching_bracket(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        if !next_matching_bracket_step(server, cs) {
            break;
        }
    }
    Move
}

fn cmd_next_paragraph(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::next_paragraph(server, cs.mode);
    }
    Move
}

fn cmd_next_space(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_next_word(server, cs.mode, b"");
    }
    Move
}

fn cmd_next_space_end(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_next_word_end(server, cs.mode, b"", false);
    }
    Move
}

fn cmd_next_word(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let Some(separators) = session_separators(server, cs) else {
        return Move;
    };
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_next_word(server, cs.mode, &separators);
    }
    Move
}

fn cmd_next_word_end(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let Some(separators) = session_separators(server, cs) else {
        return Move;
    };
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_next_word_end(server, cs.mode, &separators, false);
    }
    Move
}

fn cmd_other_end(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let np = prefix(server, cs.mode);
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.selection.selflag = SelectionMode::Char;
    }
    if np % 2 != 0 {
        select::other_end(server, cs.mode);
    }
    Move
}

fn cmd_selection_mode(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let arg = cs.wargs.string(0).map(<[u8]>::to_vec);
    let is = |value: &[u8]| {
        arg.as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(value))
    };
    if arg.is_none() || is(b"char") || is(b"c") {
        if let Some(data) = state::data_mut(server, cs.mode) {
            data.selection.selflag = SelectionMode::Char;
        }
    } else if is(b"word") || is(b"w") {
        let Some(separators) = session_separators(server, cs) else {
            return Move;
        };
        if let Some(data) = state::data_mut(server, cs.mode) {
            data.selection.separators = separators;
            data.selection.selflag = SelectionMode::Word;
        }
    } else if is(b"line") || is(b"l") {
        let Some(data) = state::data_mut(server, cs.mode) else {
            return Move;
        };
        data.selection.selflag = SelectionMode::Line;
        if !data.selection.active {
            return Move;
        }
        // Line selection normally starts with select-line, which sets up
        // the reset positions used when the cursor changes direction. Do
        // the same when changing an existing selection to line mode.
        let sel = &data.selection;
        let (fx, fy) = if sel.cursordrag == CursorDrag::Start {
            (sel.endselx, sel.endsely)
        } else {
            (sel.selx, sel.sely)
        };
        let (mut sx, mut sy, mut ex, mut ey) = (sel.selx, sel.sely, sel.endselx, sel.endsely);
        if ey < sy || (ey == sy && ex < sx) {
            std::mem::swap(&mut sx, &mut ex);
            std::mem::swap(&mut sy, &mut ey);
        }
        let grid = &data.backing.screen().grid;
        let mut gr = GridReader::new(grid, sx, sy);
        gr.cursor_start_of_line(true);
        let (sx, sy) = gr.cursor();
        let mut gr = GridReader::new(grid, ex, ey);
        gr.cursor_end_of_line(true, false);
        let (ex, ey) = gr.cursor();

        let sel = &mut data.selection;
        sel.rectflag = false;
        sel.selrx = sx;
        sel.selx = sx;
        sel.selry = sy;
        sel.sely = sy;
        sel.endselrx = ex;
        sel.endselx = ex;
        sel.endselry = ey;
        sel.endsely = ey;

        let x = data.cx;
        let y = data.backing.screen().grid.hsize() + data.cy - data.oy;
        sel.dx = fx;
        sel.dy = fy;
        let dragging = sel.cursordrag != CursorDrag::None;
        if dragging && (y < fy || (y == fy && x < fx)) {
            sel.lineflag = LineSelectionDirection::RightToLeft;
            sel.cursordrag = CursorDrag::Start;
            search::scroll_to_no_redraw(server, cs.mode, sx, sy);
        } else {
            sel.lineflag = LineSelectionDirection::LeftToRight;
            if dragging {
                sel.cursordrag = CursorDrag::End;
                let x = motion::cursor_limit(server, cs.mode, ey, false);
                search::scroll_to_no_redraw(server, cs.mode, x, ey);
            }
        }
        if state::data(server, cs.mode).is_some_and(|d| d.selection.cursordrag == CursorDrag::None)
        {
            select::set_selection(server, cs.mode, false, false);
        }
        return Redraw;
    }
    Move
}

fn cmd_page_down(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    page_down_repeat(server, cs, false, None)
}

fn cmd_page_down_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    page_down_repeat(server, cs, false, Some(true))
}

fn cmd_page_up(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::pageup1(server, cs.mode, false);
    }
    Move
}

fn cmd_previous_paragraph(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::previous_paragraph(server, cs.mode);
    }
    Move
}

fn cmd_previous_space(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_previous_word(server, cs.mode, b"", true);
    }
    Move
}

fn cmd_previous_word(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let Some(separators) = session_separators(server, cs) else {
        return Move;
    };
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_previous_word(server, cs.mode, &separators, true);
    }
    Move
}

fn rectangle_cmd(server: &mut Server, cs: &CopyCommandContext<'_>, rectflag: Option<bool>) {
    let Some(data) = state::data_mut(server, cs.mode) else {
        return;
    };
    data.selection.lineflag = LineSelectionDirection::None;
    let rectflag = rectflag.unwrap_or(!data.selection.rectflag);
    motion::rectangle_set(server, cs.mode, rectflag);
}

fn cmd_rectangle_on(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    rectangle_cmd(server, cs, Some(true));
    Move
}

fn cmd_rectangle_off(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    rectangle_cmd(server, cs, Some(false));
    Move
}

fn cmd_rectangle_toggle(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    rectangle_cmd(server, cs, None);
    Move
}

fn cmd_scroll_exit_on(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.scroll_exit = true;
    }
    Move
}

fn cmd_scroll_exit_off(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.scroll_exit = false;
    }
    Move
}

fn cmd_scroll_exit_toggle(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.scroll_exit = !data.scroll_exit;
    }
    Move
}

/// `cs->c != NULL && cs->c->tty.mouse_drag_flag != 0`.
fn dragging(server: &Server, cs: &CopyCommandContext<'_>) -> bool {
    cs.client
        .and_then(|c| server.clients.get(c))
        .is_some_and(|c| c.drag.flag != 0)
}

/// With a selection but no active drag, only scroll the view: freeze the
/// endpoints and move the viewport by `np` rows (`window-copy.c:2443-2451`).
fn view_only_scroll(
    server: &mut Server,
    cs: &CopyCommandContext<'_>,
    towards_bottom: bool,
) -> bool {
    if !state::data(server, cs.mode).is_some_and(|d| d.selection.active) || dragging(server, cs) {
        return false;
    }
    let np = prefix(server, cs.mode);
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.selection.cursordrag = CursorDrag::None;
        data.selection.lineflag = LineSelectionDirection::None;
    }
    if towards_bottom {
        motion::scroll_up(server, cs.mode, np);
    } else {
        motion::scroll_down(server, cs.mode, np);
    }
    true
}

fn cmd_scroll_down(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let Some(data) = state::data(server, cs.mode) else {
        return Nothing;
    };
    // At the bottom nothing can change; exit if scroll-exit applies.
    if data.oy == 0 {
        if data.scroll_exit && !data.selection.active {
            return Cancel;
        }
        return Nothing;
    }
    if view_only_scroll(server, cs, true) {
        return Nothing;
    }
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_down(server, cs.mode, true);
    }
    if state::data(server, cs.mode)
        .is_some_and(|d| d.scroll_exit && d.oy == 0 && !d.selection.active)
    {
        return Cancel;
    }
    Move
}

fn cmd_scroll_down_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    if view_only_scroll(server, cs, true) {
        return Nothing;
    }
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_down(server, cs.mode, true);
    }
    if state::data(server, cs.mode).is_some_and(|d| d.oy == 0) {
        return Cancel;
    }
    Move
}

fn cmd_scroll_up(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let Some(data) = state::data(server, cs.mode) else {
        return Nothing;
    };
    // At the top nothing can change, so do not repaint anything.
    if data.oy == data.backing.screen().grid.hsize() {
        return Nothing;
    }
    if view_only_scroll(server, cs, false) {
        return Nothing;
    }
    for _ in 0..prefix(server, cs.mode) {
        motion::cursor_up(server, cs.mode, true);
    }
    Move
}

fn search_repeat(server: &mut Server, cs: &CopyCommandContext<'_>, reverse: bool) {
    let Some((searchtype, regex)) =
        state::data(server, cs.mode).map(|d| (d.search.searchtype, d.search.regex))
    else {
        return;
    };
    let up = match searchtype {
        SearchDirection::Up => !reverse,
        SearchDirection::Down => reverse,
        SearchDirection::Off => return,
    };
    for _ in 0..prefix(server, cs.mode) {
        if up {
            search::search_up(server, cs.mode, regex);
        } else {
            search::search_down(server, cs.mode, regex);
        }
    }
}

fn cmd_search_again(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    search_repeat(server, cs, false);
    Move
}

fn cmd_search_reverse(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    search_repeat(server, cs, true);
    Move
}

fn cmd_select_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let mode = cs.mode;
    let np = prefix(server, mode);
    let Some(data) = state::data_mut(server, mode) else {
        return Redraw;
    };
    data.selection.lineflag = LineSelectionDirection::LeftToRight;
    data.selection.rectflag = false;
    data.selection.selflag = SelectionMode::Line;
    data.selection.dx = data.cx;
    data.selection.dy = data.backing_y();

    motion::cursor_start_of_line(server, mode);
    if let Some(data) = state::data_mut(server, mode) {
        data.selection.selrx = data.cx;
        data.selection.selry = data.backing_y();
        data.selection.endselry = data.selection.selry;
    }
    select::start_selection(server, mode);
    motion::cursor_end_of_line(server, mode);
    if let Some(data) = state::data_mut(server, mode) {
        data.selection.endselry = data.backing_y();
        let row = data.selection.endselry;
        data.selection.endselrx = motion::find_length(data, row);
    }
    for _ in 1..np {
        motion::cursor_down(server, mode, false);
        motion::cursor_end_of_line(server, mode);
    }
    Redraw
}

fn cmd_select_word(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let mode = cs.mode;
    let Some(separators) = session_separators(server, cs) else {
        return Redraw;
    };
    let Some(data) = state::data_mut(server, mode) else {
        return Redraw;
    };
    data.selection.lineflag = LineSelectionDirection::LeftToRight;
    data.selection.rectflag = false;
    data.selection.selflag = SelectionMode::Word;
    data.selection.dx = data.cx;
    data.selection.dy = data.backing_y();
    data.selection.separators = separators.clone();

    motion::cursor_previous_word(server, mode, &separators, false);
    let Some(data) = state::data_mut(server, mode) else {
        return Redraw;
    };
    let (px, py) = (data.cx, data.backing_y());
    data.selection.selrx = px;
    data.selection.selry = py;
    select::start_selection(server, mode);

    // Handle single character words.
    let Some(data) = state::data(server, mode) else {
        return Redraw;
    };
    let backing = data.backing.screen();
    let (mut nextx, mut nexty) = (px + 1, py);
    if backing
        .grid
        .get_line(nexty)
        .flags
        .contains(GridLineFlags::WRAPPED)
        && nextx > backing.grid.sx() - 1
    {
        nextx = 0;
        nexty += 1;
    }
    if px >= motion::find_length(data, py) || !motion::in_set(data, nextx, nexty, WHITESPACE) {
        motion::cursor_next_word_end(server, mode, &separators, true);
    } else {
        let cy = data.cy;
        select::update_cursor(server, mode, px, cy);
        if select::update_selection(server, mode, true, true) {
            render::redraw_lines(server, mode, cy, 1);
        }
    }
    if let Some(data) = state::data_mut(server, mode) {
        data.selection.endselrx = data.cx;
        data.selection.endselry = data.backing_y();
        let sel = &mut data.selection;
        if sel.dy > sel.endselry {
            sel.dy = sel.endselry;
            sel.dx = sel.endselrx;
        } else if sel.dx > sel.endselrx {
            sel.dx = sel.endselrx;
        }
    }
    Redraw
}

fn cmd_set_mark(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.mx = data.cx;
        data.my = data.backing_y();
        data.showmark = true;
    }
    Redraw
}

fn cmd_start_of_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    motion::cursor_start_of_line(server, cs.mode);
    Move
}

fn cmd_top_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.cx = 0;
        data.cy = 0;
    }
    select::update_selection(server, cs.mode, true, false);
    Redraw
}

fn cmd_copy_pipe_no_clear(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    let arg0 = cs.wargs.string(0).map(<[u8]>::to_vec);
    let arg1 = cs.wargs.string(1).map(<[u8]>::to_vec);
    let set_paste = cs.wargs.has(b'P') == 0;
    let set_clip = cs.wargs.has(b'C') == 0;
    let prefix = arg1.map(|a| format_single(server, cs, &a));
    let command = match arg0 {
        Some(arg0) if cs.session.is_some() && !arg0.is_empty() => {
            Some(format_single(server, cs, &arg0))
        }
        _ => None,
    };
    copy_pipe(
        server,
        cs.mode,
        cs.session,
        prefix.as_deref(),
        command.as_deref(),
        set_paste,
        set_clip,
    );
    Nothing
}

fn cmd_copy_pipe(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    cmd_copy_pipe_no_clear(server, cs);
    select::clear_selection(server, cs.mode);
    Redraw
}

fn cmd_copy_pipe_and_cancel(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    cmd_copy_pipe_no_clear(server, cs);
    select::clear_selection(server, cs.mode);
    Cancel
}

fn cmd_pipe_no_clear(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let arg0 = cs.wargs.string(0).map(<[u8]>::to_vec);
    let command = match arg0 {
        Some(arg0) if cs.session.is_some() && !arg0.is_empty() => {
            Some(format_single(server, cs, &arg0))
        }
        _ => None,
    };
    pipe(server, cs.mode, cs.session, command.as_deref());
    Move
}

fn cmd_pipe(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    cmd_pipe_no_clear(server, cs);
    select::clear_selection(server, cs.mode);
    Redraw
}

fn cmd_pipe_and_cancel(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    cmd_pipe_no_clear(server, cs);
    select::clear_selection(server, cs.mode);
    Cancel
}

fn cmd_goto_line(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if let Some(arg0) = cs.wargs.string(0).map(<[u8]>::to_vec)
        && !arg0.is_empty()
    {
        search::goto_line(server, cs.mode, &arg0);
    }
    Move
}

/// The four explicit jump commands: store the sequence and kind, then jump.
fn jump_cmd(server: &mut Server, cs: &CopyCommandContext<'_>, kind: JumpKind) -> CopyCommandAction {
    let Some(arg0) = cs.wargs.string(0) else {
        return Move;
    };
    if arg0.is_empty() {
        return Move;
    }
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.jump.kind = kind;
        data.jump.character = arg0.to_vec();
    }
    jump_repeat(server, cs, kind);
    Move
}

fn cmd_jump_backward(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    jump_cmd(server, cs, JumpKind::Backward)
}

fn cmd_jump_forward(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    jump_cmd(server, cs, JumpKind::Forward)
}

fn cmd_jump_to_backward(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    jump_cmd(server, cs, JumpKind::ToBackward)
}

fn cmd_jump_to_forward(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    jump_cmd(server, cs, JumpKind::ToForward)
}

fn cmd_jump_to_mark(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    mouse::jump_to_mark(server, cs.mode);
    Move
}

fn cmd_next_prompt(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let start_output = cs.wargs.has(b'o') != 0;
    motion::cursor_prompt(server, cs.mode, true, start_output);
    Move
}

fn cmd_previous_prompt(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    let start_output = cs.wargs.has(b'o') != 0;
    motion::cursor_prompt(server, cs.mode, false, start_output);
    Move
}

/// `window_copy_expand_search_string`: an absent or empty argument leaves
/// the stored term alone; `-F` expands it in pane context.
fn expand_search_string(server: &mut Server, cs: &CopyCommandContext<'_>) -> bool {
    let Some(ss) = cs.wargs.string(0).map(<[u8]>::to_vec) else {
        return false;
    };
    if ss.is_empty() {
        return false;
    }
    let term = if cs.args.has(b'F') != 0 {
        let context = FormatContext {
            pane: Some(cs.mode.owner),
            ..FormatContext::default()
        };
        let expanded = crate::format::single(server, None, context, &ss).0;
        if expanded.is_empty() {
            return false;
        }
        expanded
    } else {
        ss
    };
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.search.term = Some(term);
    }
    true
}

fn explicit_search(
    server: &mut Server,
    cs: &CopyCommandContext<'_>,
    direction: SearchDirection,
    regex: bool,
) -> CopyCommandAction {
    if !expand_search_string(server, cs) {
        return Move;
    }
    let Some(data) = state::data_mut(server, cs.mode) else {
        return Move;
    };
    if data.search.term.is_some() {
        data.search.searchtype = direction;
        data.search.regex = regex;
        data.timeout = false;
        for _ in 0..prefix(server, cs.mode) {
            if direction == SearchDirection::Up {
                search::search_up(server, cs.mode, regex);
            } else {
                search::search_down(server, cs.mode, regex);
            }
        }
    }
    Move
}

fn cmd_search_backward(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    explicit_search(server, cs, SearchDirection::Up, true)
}

fn cmd_search_backward_text(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    explicit_search(server, cs, SearchDirection::Up, false)
}

fn cmd_search_forward(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    explicit_search(server, cs, SearchDirection::Down, true)
}

fn cmd_search_forward_text(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    explicit_search(server, cs, SearchDirection::Down, false)
}

/// `window_copy_cmd_search_{backward,forward}_incremental`: the leading
/// `=`, `+` or `-` chooses the direction relative to `default_up`.
fn incremental_search(
    server: &mut Server,
    cs: &CopyCommandContext<'_>,
    default_up: bool,
) -> CopyCommandAction {
    let Some(arg0) = cs.wargs.string(0).map(<[u8]>::to_vec) else {
        return Move;
    };
    let mut action = Move;
    let Some(data) = state::data_mut(server, cs.mode) else {
        return Move;
    };
    data.timeout = false;
    let (prefix, term) = match arg0.split_first() {
        Some((&prefix, term)) => (prefix, term),
        None => (0, &arg0[..]),
    };
    if data.search.x.is_none() || data.search.y.is_none() {
        data.search.x = Some(data.cx);
        data.search.y = Some(data.cy);
        data.search.o = Some(data.oy);
    } else if data.search.term.as_deref().is_some_and(|ss| ss != term) {
        data.cx = data.search.x.unwrap_or(data.cx);
        data.cy = data.search.y.unwrap_or(data.cy);
        data.oy = data.search.o.unwrap_or(data.oy);
        let row = data.backing_y();
        let cx = motion::cursor_limit(server, cs.mode, row, false);
        if let Some(data) = state::data_mut(server, cs.mode) {
            data.cx = cx;
        }
        action = Redraw;
    }
    if term.is_empty() {
        search::clear_marks(server, cs.mode);
        return Redraw;
    }
    let up = match prefix {
        b'=' => default_up,
        b'-' => true,
        b'+' => false,
        _ => return action,
    };
    if let Some(data) = state::data_mut(server, cs.mode) {
        data.search.searchtype = if up {
            SearchDirection::Up
        } else {
            SearchDirection::Down
        };
        data.search.regex = false;
        data.search.term = Some(term.to_vec());
    }
    let found = if up {
        search::search_up(server, cs.mode, false)
    } else {
        search::search_down(server, cs.mode, false)
    };
    if !found {
        search::clear_marks(server, cs.mode);
        return Redraw;
    }
    action
}

fn cmd_search_backward_incremental(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    incremental_search(server, cs, true)
}

fn cmd_search_forward_incremental(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    incremental_search(server, cs, false)
}

fn cmd_refresh_now(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if super::refresh_now(server, cs.mode) {
        Redraw
    } else {
        Nothing
    }
}

fn cmd_refresh_on(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    super::refresh_start(server, cs.mode);
    Move
}

fn cmd_refresh_off(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    super::refresh_stop(server, cs.mode);
    Move
}

fn cmd_refresh_toggle(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    if state::data(server, cs.mode).is_some_and(|d| d.refresh_active) {
        super::refresh_stop(server, cs.mode);
    } else {
        super::refresh_start(server, cs.mode);
    }
    Move
}

/// `window_copy_cmd_recentre_top_bottom`: cycle middle, top, bottom.
fn cmd_recentre_top_bottom(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    let sy = visible_sy(server, cs.mode) - 1;
    let sm = sy / 2;
    let Some(data) = state::data_mut(server, cs.mode) else {
        return Redraw;
    };
    let (cy, oy) = (data.cy, data.oy);
    let backing_row = data.backing_y();
    if data.recentre_line != backing_row {
        data.recentre_state = RecentreState::Middle;
        data.recentre_line = backing_row;
    }
    let target = data.recentre_state;
    data.recentre_state = match target {
        RecentreState::Middle => RecentreState::Top,
        RecentreState::Top => RecentreState::Bottom,
        RecentreState::Bottom => RecentreState::Middle,
    };
    match target {
        RecentreState::Middle => {
            if cy < sm {
                motion::scroll_down(server, cs.mode, sm - cy);
            } else if cy > sm {
                motion::scroll_up(server, cs.mode, cy - sm);
            }
            if let Some(data) = state::data_mut(server, cs.mode)
                && data.oy != oy
            {
                data.cy = cy.wrapping_add(data.oy).wrapping_sub(oy);
            }
        }
        RecentreState::Top => {
            motion::scroll_up(server, cs.mode, cy);
            if let Some(data) = state::data_mut(server, cs.mode) {
                data.cy = cy - (oy - data.oy);
            }
        }
        RecentreState::Bottom => {
            motion::scroll_down(server, cs.mode, sy - cy);
            if let Some(data) = state::data_mut(server, cs.mode) {
                data.cy = cy + (data.oy - oy);
            }
        }
    }
    select::update_selection_view(server, cs.mode, false, false);
    Redraw
}

fn cmd_line_numbers_on(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    render::set_line_numbers1(server, cs.mode, true, true);
    Nothing
}

fn cmd_line_numbers_off(server: &mut Server, cs: &mut CopyCommandContext<'_>) -> CopyCommandAction {
    render::set_line_numbers1(server, cs.mode, false, false);
    Nothing
}

fn cmd_line_numbers_toggle(
    server: &mut Server,
    cs: &mut CopyCommandContext<'_>,
) -> CopyCommandAction {
    let active = render::line_numbers_active(server, cs.mode);
    render::set_line_numbers1(server, cs.mode, !active, true);
    Nothing
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
