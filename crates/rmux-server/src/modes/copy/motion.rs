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

//! Grid-reader motion adapters, vertical and page motion, optimized viewport
//! scrolling and the external scroll entry points (`window-copy.c:764-1032,
//! 6298-6390,6446-7057,7248-7301`).

use super::render;
use super::search;
use super::select;
use super::state::{self, CopyModeData, CursorDrag, LineSelectionDirection, ModeKeys};
use crate::ids::{ModeId, OptionsId, PaneId};
use crate::model::pane::{
    pane_reset_mode, pane_scrollbar_overlay, pane_scrollbar_show, pane_scrollbar_visible,
};
use crate::model::window::{window_redraw_active_switch, window_set_active_pane};
use crate::model::{PaneFlags, Server};
use rmux_emu::cell::GridCellFlags;
use rmux_emu::colour::Colour;
use rmux_emu::grid::GridLineFlags;
use rmux_emu::grid::reader::{GridReader, WHITESPACE};
use rmux_util::utf8::{Utf8Data, from_cstr};

/// `wp->window->options`.
pub fn window_options(server: &Server, pane: PaneId) -> Option<OptionsId> {
    let window = server.panes.get(pane)?.window;
    Some(server.windows.get(window)?.options)
}

/// `options_get_number(wp->window->options, "mode-keys")`, read live.
pub fn mode_keys(server: &Server, mode: ModeId) -> ModeKeys {
    match window_options(server, mode.owner) {
        Some(options) if server.options.get_number(options, b"mode-keys") == 1 => ModeKeys::Vi,
        _ => ModeKeys::Emacs,
    }
}

fn top_mode(server: &Server, pane: PaneId) -> Option<ModeId> {
    let mode = server.panes.get(pane)?.modes.first()?.id;
    state::data(server, mode)?;
    Some(mode)
}

fn is_top(server: &Server, mode: ModeId) -> bool {
    server
        .panes
        .get(mode.owner)
        .is_some_and(|p| p.modes.first().is_some_and(|m| m.id == mode))
}

/// `window_copy_in_set`.
pub fn in_set(data: &CopyModeData, px: u32, py: u32, set: &[u8]) -> bool {
    data.backing.screen().grid.in_set(px, py, set) != 0
}

/// `window_copy_find_length`.
pub fn find_length(data: &CopyModeData, py: u32) -> u32 {
    data.backing.screen().grid.line_length(py)
}

/// `window_copy_cursor_limit`.
pub fn cursor_limit(server: &Server, mode: ModeId, py: u32, allow_onemore: bool) -> u32 {
    let Some(data) = state::data(server, mode) else {
        return 0;
    };
    if allow_onemore || mode_keys(server, mode) != ModeKeys::Vi {
        return find_length(data, py);
    }
    data.backing.screen().grid.line_limit(py)
}

/// Cursor position in backing coordinates plus the facts every adapter
/// snapshots before it moves: `(hsize, oy, oldy, px, py)`.
fn reader_origin(data: &CopyModeData) -> (u32, u32, u32, u32, u32) {
    let hsize = data.backing.screen().grid.hsize();
    (hsize, data.oy, data.cy, data.cx, hsize - data.oy + data.cy)
}

fn backing_sy(data: &CopyModeData) -> u32 {
    data.backing.screen().grid.sy()
}

fn visible_sy(server: &Server, mode: ModeId) -> u32 {
    state::screen(server, mode).map_or(0, |s| s.grid.sy())
}

/// `window_copy_cursor_start_of_line`.
pub fn cursor_start_of_line(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_start_of_line(true);
    let (px, py) = gr.cursor();
    acquire_cursor_up(server, mode, hsize, oy, oldy, px, py);
}

/// `window_copy_cursor_back_to_indentation`.
pub fn cursor_back_to_indentation(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_back_to_indentation();
    let (px, py) = gr.cursor();
    acquire_cursor_up(server, mode, hsize, oy, oldy, px, py);
}

/// `window_copy_cursor_end_of_line`.
pub fn cursor_end_of_line(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let sy = backing_sy(data);
    let rectangle = data.selection.active && data.selection.rectflag;
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_end_of_line(true, rectangle);
    let (mut px, py) = gr.cursor();
    if !rectangle {
        px = cursor_limit(server, mode, py, false);
    }
    acquire_cursor_down(server, mode, hsize, sy, oy, oldy, px, py, false);
}

/// `window_copy_cursor_left`.
pub fn cursor_left(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_left(true);
    let (px, py) = gr.cursor();
    acquire_cursor_up(server, mode, hsize, oy, oldy, px, py);
}

/// `window_copy_cursor_right`.
pub fn cursor_right(server: &mut Server, mode: ModeId, all: bool) {
    let onemore = mode_keys(server, mode) != ModeKeys::Vi;
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let sy = backing_sy(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_right(true, all, onemore);
    let (px, py) = gr.cursor();
    acquire_cursor_down(server, mode, hsize, sy, oy, oldy, px, py, false);
}

/// The preferred-column bookkeeping shared by `window_copy_cursor_up/down`.
/// Returns `(norectsel, backing row)`.
fn remember_column(data: &mut CopyModeData) -> (bool, u32) {
    let norectsel = !(data.selection.active && data.selection.rectflag);
    let oy = data.backing.screen().grid.hsize() - data.oy + data.cy;
    let ox = find_length(data, oy);
    if norectsel && data.cx != ox {
        data.lastcx = data.cx;
        data.lastsx = ox;
    }
    (norectsel, oy)
}

/// The end-of-line snap and line-selection follow-up shared by
/// `window_copy_cursor_up/down` (`window-copy.c:6538-6566`).
fn vertical_tail(server: &mut Server, mode: ModeId, norectsel: bool) {
    if norectsel {
        let Some(data) = state::data(server, mode) else {
            return;
        };
        let py = data.backing.screen().grid.hsize() - data.oy + data.cy;
        let px = find_length(data, py);
        let cy = data.cy;
        if (data.cx >= data.lastsx && data.cx != px) || data.cx > px {
            select::update_cursor(server, mode, px, cy);
            if select::update_selection(server, mode, true, false) {
                render::redraw_lines(server, mode, cy, 1);
            }
        }
    }
    let Some(data) = state::data(server, mode) else {
        return;
    };
    match data.selection.lineflag {
        LineSelectionDirection::LeftToRight => {
            let py = data.backing.screen().grid.hsize() - data.oy + data.cy;
            let px = if data.selection.rectflag {
                data.backing.screen().grid.sx()
            } else {
                find_length(data, py)
            };
            let cy = data.cy;
            select::update_cursor(server, mode, px, cy);
            if select::update_selection(server, mode, true, false) {
                render::redraw_lines(server, mode, cy, 1);
            }
        }
        LineSelectionDirection::RightToLeft => {
            let cy = data.cy;
            select::update_cursor(server, mode, 0, cy);
            if select::update_selection(server, mode, true, false) {
                render::redraw_lines(server, mode, cy, 1);
            }
        }
        LineSelectionDirection::None => {}
    }
}

/// `window_copy_cursor_up`.
pub fn cursor_up(server: &mut Server, mode: ModeId, scroll_only: bool) {
    let vi = mode_keys(server, mode) == ModeKeys::Vi;
    let sy = visible_sy(server, mode);
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let (norectsel, oy) = remember_column(data);
    if data.selection.lineflag == LineSelectionDirection::LeftToRight && oy == data.selection.sely {
        select::other_end(server, mode);
    }
    if scroll_only
        && vi
        && let Some(data) = state::data(server, mode)
        && data.cy < sy - 1
    {
        let (cx, cy) = (data.cx, data.cy);
        select::update_cursor(server, mode, cx, cy + 1);
    }
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    if scroll_only || data.cy == 0 {
        if norectsel {
            data.cx = data.lastcx;
        }
        scroll_down(server, mode, 1);
        if scroll_only {
            let Some(data) = state::data(server, mode) else {
                return;
            };
            let cy = data.cy;
            if cy == sy - 1 {
                render::redraw_lines(server, mode, cy, 1);
            } else {
                render::redraw_lines(server, mode, cy, 2);
            }
        }
    } else {
        let (cx, cy, lastcx) = (data.cx, data.cy, data.lastcx);
        if norectsel {
            select::update_cursor(server, mode, lastcx, cy - 1);
        } else {
            select::update_cursor(server, mode, cx, cy - 1);
        }
        if select::update_selection(server, mode, true, false) {
            let Some(data) = state::data(server, mode) else {
                return;
            };
            let cy = data.cy;
            if cy == sy - 1 {
                render::redraw_lines(server, mode, cy, 1);
            } else {
                render::redraw_lines(server, mode, cy, 2);
            }
        }
    }
    vertical_tail(server, mode, norectsel);
}

/// `window_copy_cursor_down`.
pub fn cursor_down(server: &mut Server, mode: ModeId, scroll_only: bool) {
    let vi = mode_keys(server, mode) == ModeKeys::Vi;
    let sy = visible_sy(server, mode);
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let (norectsel, oy) = remember_column(data);
    if data.selection.lineflag == LineSelectionDirection::RightToLeft
        && oy == data.selection.endsely
    {
        select::other_end(server, mode);
    }
    if scroll_only
        && vi
        && let Some(data) = state::data(server, mode)
        && data.cy > 0
    {
        let (cx, cy) = (data.cx, data.cy);
        select::update_cursor(server, mode, cx, cy - 1);
    }
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    if scroll_only || data.cy == sy - 1 {
        if norectsel {
            data.cx = data.lastcx;
        }
        scroll_up(server, mode, 1);
        if scroll_only {
            let Some(data) = state::data(server, mode) else {
                return;
            };
            if data.cy > 0 {
                let cy = data.cy;
                render::redraw_lines(server, mode, cy - 1, 2);
            }
        }
    } else {
        let (cx, cy, lastcx) = (data.cx, data.cy, data.lastcx);
        if norectsel {
            select::update_cursor(server, mode, lastcx, cy + 1);
        } else {
            select::update_cursor(server, mode, cx, cy + 1);
        }
        if select::update_selection(server, mode, true, false) {
            let Some(data) = state::data(server, mode) else {
                return;
            };
            let cy = data.cy;
            render::redraw_lines(server, mode, cy - 1, 2);
        }
    }
    vertical_tail(server, mode, norectsel);
}

fn jump_char(data: &CopyModeData) -> Option<Utf8Data> {
    from_cstr(&data.jump.character).0.first().copied()
}

/// `window_copy_cursor_jump`.
pub fn cursor_jump(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let Some(jc) = jump_char(data) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let sy = backing_sy(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px + 1, py);
    if gr.cursor_jump(&jc) {
        let (px, py) = gr.cursor();
        acquire_cursor_down(server, mode, hsize, sy, oy, oldy, px, py, false);
    }
}

/// `window_copy_cursor_jump_back`.
pub fn cursor_jump_back(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let Some(jc) = jump_char(data) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_left(false);
    if gr.cursor_jump_back(&jc) {
        let (px, py) = gr.cursor();
        acquire_cursor_up(server, mode, hsize, oy, oldy, px, py);
    }
}

/// `window_copy_cursor_jump_to`.
pub fn cursor_jump_to(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let Some(jc) = jump_char(data) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let sy = backing_sy(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px + 2, py);
    if gr.cursor_jump(&jc) {
        gr.cursor_left(true);
        let (px, py) = gr.cursor();
        acquire_cursor_down(server, mode, hsize, sy, oy, oldy, px, py, false);
    }
}

/// `window_copy_cursor_jump_to_back`.
pub fn cursor_jump_to_back(server: &mut Server, mode: ModeId) {
    let onemore = mode_keys(server, mode) != ModeKeys::Vi;
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let Some(jc) = jump_char(data) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_left(false);
    gr.cursor_left(false);
    if gr.cursor_jump_back(&jc) {
        gr.cursor_right(true, false, onemore);
        let (px, py) = gr.cursor();
        acquire_cursor_up(server, mode, hsize, oy, oldy, px, py);
    }
}

/// `window_copy_cursor_next_word`.
pub fn cursor_next_word(server: &mut Server, mode: ModeId, separators: &[u8]) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let sy = backing_sy(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_next_word(separators);
    let (px, py) = gr.cursor();
    acquire_cursor_down(server, mode, hsize, sy, oy, oldy, px, py, false);
}

/// `window_copy_cursor_next_word_end_pos` and the reader part of
/// `window_copy_cursor_next_word_end`.
fn next_word_end_reader(data: &CopyModeData, vi: bool, separators: &[u8]) -> (u32, u32) {
    let (_, _, _, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    if vi {
        if gr.in_set(WHITESPACE) == 0 {
            gr.cursor_right(false, false, false);
        }
        gr.cursor_next_word_end(separators);
        gr.cursor_left(true);
    } else {
        gr.cursor_next_word_end(separators);
    }
    gr.cursor()
}

/// `window_copy_cursor_next_word_end`.
pub fn cursor_next_word_end(server: &mut Server, mode: ModeId, separators: &[u8], no_reset: bool) {
    let vi = mode_keys(server, mode) == ModeKeys::Vi;
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, _, _) = reader_origin(data);
    let sy = backing_sy(data);
    let (px, py) = next_word_end_reader(data, vi, separators);
    acquire_cursor_down(server, mode, hsize, sy, oy, oldy, px, py, no_reset);
}

/// `window_copy_cursor_previous_word_pos`.
pub fn cursor_previous_word_pos(
    server: &Server,
    mode: ModeId,
    separators: &[u8],
) -> Option<(u32, u32)> {
    let data = state::data(server, mode)?;
    let (_, _, _, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_previous_word(separators, false, true);
    Some(gr.cursor())
}

/// `window_copy_cursor_previous_word`.
pub fn cursor_previous_word(server: &mut Server, mode: ModeId, separators: &[u8], already: bool) {
    let stop_at_eol = mode_keys(server, mode) == ModeKeys::Emacs;
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let (hsize, oy, oldy, px, py) = reader_origin(data);
    let mut gr = GridReader::new(&data.backing.screen().grid, px, py);
    gr.cursor_previous_word(separators, already, stop_at_eol);
    let (px, py) = gr.cursor();
    acquire_cursor_up(server, mode, hsize, oy, oldy, px, py);
}

/// `window_copy_cursor_prompt`.
pub fn cursor_prompt(server: &mut Server, mode: ModeId, down: bool, start_output: bool) {
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let grid = &data.backing.screen().grid;
    let hsize = grid.hsize();
    let mut line = hsize - data.oy + data.cy;
    let flag = if start_output {
        GridLineFlags::START_OUTPUT
    } else {
        GridLineFlags::START_PROMPT
    };
    let end_line = if down { hsize + grid.sy() - 1 } else { 0 };
    if line == end_line {
        return;
    }
    loop {
        if line == end_line {
            return;
        }
        line = if down { line + 1 } else { line - 1 };
        if grid.get_line(line).flags.contains(flag) {
            break;
        }
    }
    data.cx = 0;
    if line > hsize {
        data.cy = line - hsize;
        data.oy = 0;
    } else {
        data.cy = 0;
        data.oy = hsize - line;
    }
    select::update_selection(server, mode, true, false);
    render::redraw_screen(server, mode);
}

/// `window_copy_previous_paragraph`.
pub fn previous_paragraph(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let mut oy = data.backing.screen().grid.hsize() - data.oy + data.cy;
    while oy > 0 && find_length(data, oy) == 0 {
        oy -= 1;
    }
    while oy > 0 && find_length(data, oy) > 0 {
        oy -= 1;
    }
    search::scroll_to(server, mode, 0, oy);
}

/// `window_copy_next_paragraph`.
pub fn next_paragraph(server: &mut Server, mode: ModeId) {
    let sy = visible_sy(server, mode);
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let hsize = data.backing.screen().grid.hsize();
    let mut oy = hsize - data.oy + data.cy;
    let maxy = hsize + sy - 1;
    while oy < maxy && find_length(data, oy) == 0 {
        oy += 1;
    }
    while oy < maxy && find_length(data, oy) > 0 {
        oy += 1;
    }
    let ox = find_length(data, oy);
    search::scroll_to(server, mode, ox, oy);
}

/// `window_copy_rectangle_set`.
pub fn rectangle_set(server: &mut Server, mode: ModeId, rectflag: bool) {
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    data.selection.rectflag = rectflag;
    let py = data.backing.screen().grid.hsize() - data.oy + data.cy;
    let (cx, cy) = (data.cx, data.cy);
    let px = cursor_limit(server, mode, py, rectflag);
    if cx > px {
        select::update_cursor(server, mode, px, cy);
    }
    select::update_selection(server, mode, true, false);
    render::redraw_screen(server, mode);
}

/// `window_copy_acquire_cursor_up`: scroll up if the cursor went off the
/// visible screen.
pub fn acquire_cursor_up(
    server: &mut Server,
    mode: ModeId,
    hsize: u32,
    oy: u32,
    oldy: u32,
    px: u32,
    py: u32,
) {
    let yy = hsize - oy;
    let (mut ny, cy, nd) = if py < yy {
        (yy - py, 0, 1)
    } else {
        (0, py - yy, oldy.wrapping_sub(py - yy).wrapping_add(1))
    };
    while ny > 0 {
        cursor_up(server, mode, true);
        ny -= 1;
    }
    select::update_cursor(server, mode, px, cy);
    if select::update_selection(server, mode, true, false) {
        render::redraw_lines(server, mode, cy, nd);
    }
}

/// `window_copy_acquire_cursor_down`: scroll down if the cursor went off the
/// visible screen.
#[allow(clippy::too_many_arguments)]
pub fn acquire_cursor_down(
    server: &mut Server,
    mode: ModeId,
    hsize: u32,
    sy: u32,
    oy: u32,
    mut oldy: u32,
    px: u32,
    py: u32,
    no_reset: bool,
) {
    let cy = py.wrapping_add(oy).wrapping_sub(hsize);
    let yy = sy - 1;
    let (mut ny, nd) = if cy > yy {
        oldy = yy;
        (cy - yy, 1)
    } else {
        (0, cy.wrapping_sub(oldy).wrapping_add(1))
    };
    while ny > 0 {
        cursor_down(server, mode, true);
        ny -= 1;
    }
    select::update_cursor(server, mode, px, if cy > yy { yy } else { cy });
    if select::update_selection(server, mode, true, no_reset) {
        render::redraw_lines(server, mode, oldy, nd);
    }
}

/// `window_pane_scrollbar_redraw` (`window.c:1562-1572`).
pub fn scrollbar_redraw(server: &mut Server, pane: PaneId) {
    if !pane_scrollbar_visible(server, pane) {
        return;
    }
    let overlay = pane_scrollbar_overlay(server, pane);
    if let Some(p) = server.panes.get_mut(pane) {
        p.flags.insert(if overlay {
            PaneFlags::REDRAW
        } else {
            PaneFlags::REDRAWSCROLLBAR
        });
    }
}

/// The repaint shared by the scroll paths after the viewport moved.
/// Returns false when the whole screen was redrawn instead.
fn scroll_prepare(server: &mut Server, mode: ModeId) -> Option<bool> {
    pane_scrollbar_show(server, mode.owner, true).ok()?;
    let data = state::data(server, mode)?;
    if data.search.marks.is_some() && !data.timeout {
        search::search_marks(server, mode, true);
    }
    select::update_selection_view(server, mode, false, false);
    if render::cursor_line_active(server, mode) {
        render::redraw_screen(server, mode);
        return Some(false);
    }
    if render::line_numbers_active(server, mode) && !render::line_number_is_absolute(server, mode) {
        render::redraw_screen(server, mode);
        return Some(false);
    }
    Some(true)
}

/// `window_copy_scroll_up`: the view moves towards the bottom.
pub fn scroll_up(server: &mut Server, mode: ModeId, mut ny: u32) {
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    if data.oy < ny {
        ny = data.oy;
    }
    if ny == 0 {
        return;
    }
    data.oy -= ny;
    if scroll_prepare(server, mode) != Some(true) {
        return;
    }
    let numbers = render::line_numbers_active(server, mode);
    let overlay =
        pane_scrollbar_overlay(server, mode.owner) && pane_scrollbar_visible(server, mode.owner);
    let style = render::render_style(server, mode);
    let gutter = style.gutter_width;
    render::with_pane_write(server, mode, numbers || overlay, |data, ctx| {
        let (sx, sy) = (ctx.screen.grid.sx(), ctx.screen.grid.sy());
        ctx.cursormove(0, 0, false);
        ctx.deleteline(ny, Colour::DEFAULT);
        // C's `sy - ny` wraps when ny > sy, so its loop writes no rows; only
        // rows 0, 1 and sy - 2 below are repainted, as in the oracle.
        if ny <= sy {
            render::write_lines(data, ctx, sy - ny, ny, &style);
        }
        render::write_line(data, ctx, 0, &style);
        if sy > 1 {
            render::write_line(data, ctx, 1, &style);
        }
        if sy > 3 {
            render::write_line(data, ctx, sy - 2, &style);
        }
        if ctx.screen.selection.is_some() && sy > ny {
            render::write_line(data, ctx, sy - ny - 1, &style);
        }
        let x = render::offset_for_gutter(gutter, data.cx, sx);
        ctx.cursormove(x as i32, data.cy as i32, false);
    });
    if numbers {
        if let Some(p) = server.panes.get_mut(mode.owner) {
            p.flags
                .insert(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR);
        }
        return;
    }
    scrollbar_redraw(server, mode.owner);
}

/// `window_copy_scroll_down`: the view moves towards the history top.
pub fn scroll_down(server: &mut Server, mode: ModeId, mut ny: u32) {
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let hsize = data.backing.screen().grid.hsize();
    if ny > hsize {
        return;
    }
    if data.oy > hsize - ny {
        ny = hsize - data.oy;
    }
    if ny == 0 {
        return;
    }
    data.oy += ny;
    if scroll_prepare(server, mode) != Some(true) {
        return;
    }
    let numbers = render::line_numbers_active(server, mode);
    let overlay =
        pane_scrollbar_overlay(server, mode.owner) && pane_scrollbar_visible(server, mode.owner);
    let style = render::render_style(server, mode);
    let gutter = style.gutter_width;
    render::with_pane_write(server, mode, numbers || overlay, |data, ctx| {
        let (sx, sy) = (ctx.screen.grid.sx(), ctx.screen.grid.sy());
        ctx.cursormove(0, 0, false);
        ctx.insertline(ny, Colour::DEFAULT);
        render::write_lines(data, ctx, 0, ny.min(sy), &style);
        if ctx.screen.selection.is_some() && sy > ny {
            render::write_line(data, ctx, ny, &style);
        } else if ny == 1 {
            // Nuke the position indicator left on the old top row.
            render::write_line(data, ctx, 1, &style);
        }
        let x = render::offset_for_gutter(gutter, data.cx, sx);
        ctx.cursormove(x as i32, data.cy as i32, false);
    });
    if numbers {
        if let Some(p) = server.panes.get_mut(mode.owner) {
            p.flags
                .insert(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR);
        }
        return;
    }
    scrollbar_redraw(server, mode.owner);
}

/// The preferred-column restore shared by page motion and scrollbar
/// movement (`window-copy.c:827-834,894-901`).
fn restore_preferred_column(data: &mut CopyModeData) {
    let oy = data.backing.screen().grid.hsize() - data.oy + data.cy;
    let ox = find_length(data, oy);
    if data.cx != ox {
        data.lastcx = data.cx;
        data.lastsx = ox;
    }
    data.cx = data.lastcx;
}

/// The end-of-line snap after a viewport move (`window-copy.c:861-867`).
fn snap_end_of_line(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    if data.selection.active && data.selection.rectflag {
        return;
    }
    let py = data.backing.screen().grid.hsize() - data.oy + data.cy;
    let px = find_length(data, py);
    if (data.cx >= data.lastsx && data.cx != px) || data.cx > px {
        cursor_end_of_line(server, mode);
    }
}

/// Visible marks, selection, scrollbar and screen after a page move.
fn page_finish(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    if data.search.marks.is_some() && !data.timeout {
        search::search_marks(server, mode, true);
    }
    select::update_selection(server, mode, true, false);
    let _ = pane_scrollbar_show(server, mode.owner, true);
    render::redraw_screen(server, mode);
}

fn page_rows(half_page: bool, sy: u32) -> u32 {
    if sy > 2 {
        if half_page { sy / 2 } else { sy - 2 }
    } else {
        1
    }
}

/// `window_copy_pageup1`.
pub fn pageup1(server: &mut Server, mode: ModeId, half_page: bool) {
    let sy = visible_sy(server, mode);
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    restore_preferred_column(data);
    let n = page_rows(half_page, sy);
    let hsize = data.backing.screen().grid.hsize();
    if data.oy + n > hsize {
        data.oy = hsize;
        data.cy = data.cy.saturating_sub(n);
    } else {
        data.oy += n;
    }
    snap_end_of_line(server, mode);
    page_finish(server, mode);
}

/// `window_copy_pagedown1`; true asks the caller to leave the mode.
pub fn pagedown1(server: &mut Server, mode: ModeId, half_page: bool, scroll_exit: bool) -> bool {
    let sy = visible_sy(server, mode);
    let Some(data) = state::data_mut(server, mode) else {
        return false;
    };
    restore_preferred_column(data);
    let n = page_rows(half_page, sy);
    let backing_rows = backing_sy(data);
    if data.oy < n {
        data.oy = 0;
        if data.cy + n >= backing_rows {
            data.cy = backing_rows - 1;
        } else {
            data.cy += n;
        }
    } else {
        data.oy -= n;
    }
    snap_end_of_line(server, mode);
    let Some(data) = state::data(server, mode) else {
        return false;
    };
    if scroll_exit && data.oy == 0 && !data.selection.active {
        return true;
    }
    page_finish(server, mode);
    false
}

/// `window_copy_pageup`.
pub fn page_up(server: &mut Server, pane: PaneId, half_page: bool) {
    if let Some(mode) = top_mode(server, pane) {
        pageup1(server, mode, half_page);
    }
}

/// `window_copy_pagedown`.
pub fn page_down(server: &mut Server, pane: PaneId, half_page: bool, scroll_exit: bool) {
    if let Some(mode) = top_mode(server, pane)
        && pagedown1(server, mode, half_page, scroll_exit)
    {
        let _ = pane_reset_mode(server, pane);
    }
}

/// `window_copy_scroll`: scrollbar slider drag entry.
pub fn scrollbar_scroll(
    server: &mut Server,
    pane: PaneId,
    slider_grab_row: i32,
    tty_row: u32,
    tty_pan_row: u32,
    scroll_exit: bool,
) {
    let Some(mode) = top_mode(server, pane) else {
        return;
    };
    let Some(window) = server.panes.get(pane).map(|p| p.window) else {
        return;
    };
    let _ = window_redraw_active_switch(server, window, Some(pane));
    let _ = window_set_active_pane(server, window, pane, false);
    scroll1(
        server,
        mode,
        slider_grab_row,
        tty_row,
        tty_pan_row,
        scroll_exit,
    );
}

/// `window_copy_scroll1`.
fn scroll1(
    server: &mut Server,
    mode: ModeId,
    sl_mpos: i32,
    my: u32,
    tty_oy: u32,
    scroll_exit: bool,
) {
    let Some(p) = server.panes.get(mode.owner) else {
        return;
    };
    let (slider_height, sb_height, sb_top, yoff) = (p.sb_slider_h, p.sy, p.yoff as u32, p.yoff);
    let my_w = my.wrapping_add(tty_oy);
    let new_slider_y: i32 = if my_w <= sb_top.wrapping_add(sl_mpos as u32) {
        sb_top as i32 - yoff
    } else if my_w.wrapping_sub(sl_mpos as u32) > sb_top + sb_height - slider_height {
        sb_top as i32 - yoff + (sb_height - slider_height) as i32
    } else {
        my_w as i32 - yoff - sl_mpos
    };
    if !is_top(server, mode) {
        return;
    }
    let Some((offset, size)) = render::current_offset(server, mode.owner) else {
        return;
    };
    let new_offset = slider_offset(new_slider_y, size, sb_height);
    let delta = offset as i32 - new_offset as i32;

    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let sy = backing_sy(data);
    let hsize = data.backing.screen().grid.hsize();
    restore_preferred_column(data);
    if delta >= 0 {
        let n = delta as u32;
        if data.oy + n > hsize {
            data.oy = hsize;
            data.cy = data.cy.saturating_sub(n);
        } else {
            data.oy += n;
        }
    } else {
        let n = delta.unsigned_abs();
        if data.oy < n {
            data.oy = 0;
            if data.cy + n >= sy {
                data.cy = sy - 1;
            } else {
                data.cy += n;
            }
        } else {
            data.oy -= n;
        }
    }
    // Don't also drag the tail when dragging a scrollbar.
    data.selection.cursordrag = CursorDrag::None;
    snap_end_of_line(server, mode);
    let Some(data) = state::data(server, mode) else {
        return;
    };
    if scroll_exit && data.oy == 0 && !data.selection.active {
        let _ = pane_reset_mode(server, mode.owner);
        return;
    }
    if data.search.marks.is_some() && !data.timeout {
        search::search_marks(server, mode, true);
    }
    select::update_selection_view(server, mode, true, false);
    let _ = pane_scrollbar_show(server, mode.owner, true);
    render::redraw_screen(server, mode);
}

/// A one-byte, non-padding cell byte for bracket matching.
pub fn single_byte(data: &CopyModeData, px: u32, py: u32) -> Option<u8> {
    let gc = data.backing.screen().grid.get_cell(px, py);
    if gc.data.size != 1 || gc.flags.contains(GridCellFlags::PADDING) {
        return None;
    }
    Some(gc.data.data[0])
}

/// The inverse of `redraw_draw_pane_scrollbar`, in single precision with C
/// truncation (`window-copy.c:820`).
pub fn slider_offset(new_slider_y: i32, size: u32, sb_height: u32) -> u32 {
    (new_slider_y as f32 * ((size + sb_height) as f32 / sb_height as f32)) as u32
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
