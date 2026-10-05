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

use super::state::{
    self, CopyModeData, CursorDrag, LineSelectionDirection, ModeKeys, SelectionMode,
};
use crate::ids::ModeId;
use crate::model::Server;
use rmux_emu::cell::{GridAttributes, GridCell, GridCellFlags};
use rmux_emu::grid::GridLineFlags;
use rmux_emu::grid::reader::{GridReader, WHITESPACE};
use rmux_emu::screen::{Screen, ScreenSelection};

fn current_keys(server: &Server, mode: ModeId) -> Option<ModeKeys> {
    let pane = server.panes.get(mode.owner)?;
    let window = server.windows.get(pane.window)?;
    Some(
        if server.options.get_number(window.options, b"mode-keys") == 1 {
            ModeKeys::Vi
        } else {
            ModeKeys::Emacs
        },
    )
}

fn synchronize_cursor(data: &mut CopyModeData, keys: ModeKeys, no_reset: bool) {
    let mut begin = match data.selection.cursordrag {
        CursorDrag::None => return,
        CursorDrag::Start => true,
        CursorDrag::End => false,
    };
    let (mut x, mut y) = (data.cx, data.backing_y());
    let grid = &data.backing.screen().grid;
    match data.selection.selflag {
        SelectionMode::Word if !no_reset => {
            begin = false;
            if (data.selection.dy, data.selection.dx) > (y, x) {
                let mut reader = GridReader::new(grid, x, y);
                reader.cursor_previous_word(&data.selection.separators, false, true);
                (x, y) = reader.cursor();
                begin = true;
                data.selection.endselx = data.selection.endselrx;
                data.selection.endsely = data.selection.endselry;
            } else {
                if x >= grid.line_length(y) || grid.in_set(x + 1, y, WHITESPACE) == 0 {
                    let mut reader = GridReader::new(grid, x, y);
                    if keys == ModeKeys::Vi {
                        if reader.in_set(WHITESPACE) == 0 {
                            reader.cursor_right(false, false, false);
                        }
                        reader.cursor_next_word_end(&data.selection.separators);
                        reader.cursor_left(true);
                    } else {
                        reader.cursor_next_word_end(&data.selection.separators);
                    }
                    (x, y) = reader.cursor();
                }
                data.selection.selx = data.selection.selrx;
                data.selection.sely = data.selection.selry;
            }
        }
        SelectionMode::Line if !no_reset => {
            begin = false;
            if data.selection.dy > y {
                x = 0;
                begin = true;
                data.selection.endselx = data.selection.endselrx;
                data.selection.endsely = data.selection.endselry;
            } else {
                y = y.max(data.selection.endselry);
                x = grid.line_length(y);
                data.selection.selx = data.selection.selrx;
                data.selection.sely = data.selection.selry;
            }
        }
        _ => {}
    }
    if begin {
        data.selection.selx = x;
        data.selection.sely = y;
    } else {
        data.selection.endselx = x;
        data.selection.endsely = y;
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RelativePosition {
    Above,
    OnScreen,
    Below,
}

fn adjust_selection(
    data: &CopyModeData,
    screen: &Screen,
    x: u32,
    y: u32,
) -> (u32, u32, RelativePosition) {
    let top = data.backing.screen().grid.hsize() - data.oy;
    if y < top {
        (
            if data.selection.rectflag { x } else { 0 },
            0,
            RelativePosition::Above,
        )
    } else if y > top + screen.grid.sy() - 1 {
        (
            if data.selection.rectflag {
                x
            } else {
                screen.grid.sx() - 1
            },
            screen.grid.sy() - 1,
            RelativePosition::Below,
        )
    } else {
        (x, y - top, RelativePosition::OnScreen)
    }
}

fn project_selection(
    data: &mut CopyModeData,
    screen: &mut Screen,
    gutter: u32,
    cell: GridCell,
) -> bool {
    let (mut sx, sy, start) =
        adjust_selection(data, screen, data.selection.selx, data.selection.sely);
    let (mut ex, ey, end) =
        adjust_selection(data, screen, data.selection.endselx, data.selection.endsely);
    if start == end && start != RelativePosition::OnScreen {
        screen.hide_selection();
        data.selection.active = screen.selection.is_some();
        return false;
    }
    let width = screen.grid.sx();
    if gutter != 0 {
        sx = super::render::offset_for_gutter(gutter, sx, width);
        ex = super::render::offset_for_gutter(gutter, ex, width);
    }
    screen.set_selection(ScreenSelection {
        hidden: false,
        rectangle: data.selection.rectflag,
        modekeys: data.modekeys.as_i32(),
        sx,
        sy,
        ex,
        ey,
        clipx: gutter.min(width - 1),
        cell,
    });
    data.selection.active = true;
    true
}

pub fn set_selection(server: &mut Server, mode: ModeId, may_redraw: bool, no_reset: bool) -> bool {
    let Some(keys) = current_keys(server, mode) else {
        return false;
    };
    let gutter = super::render::line_number_width(server, mode);
    let style = super::render::selection_style(server, mode);
    let redraw = {
        let Some((data, screen)) = state::parts_mut(server, mode) else {
            return false;
        };
        synchronize_cursor(data, keys, no_reset);
        if !project_selection(data, screen, gutter, style) {
            return false;
        }
        if data.selection.rectflag && may_redraw {
            let selection = screen.selection.as_ref().expect("projected selection");
            let end = if data.selection.cursordrag == CursorDrag::End {
                selection.sy
            } else {
                selection.ey
            };
            Some((end.min(data.cy), end.abs_diff(data.cy) + 1))
        } else {
            None
        }
    };
    if let Some((row, count)) = redraw {
        super::render::redraw_lines(server, mode, row, count);
    }
    true
}

pub fn update_selection(
    server: &mut Server,
    mode: ModeId,
    may_redraw: bool,
    no_reset: bool,
) -> bool {
    let Some(data) = state::data(server, mode) else {
        return false;
    };
    if !data.selection.active && data.selection.lineflag == LineSelectionDirection::None {
        return false;
    }
    set_selection(server, mode, may_redraw, no_reset)
}

pub fn update_selection_view(
    server: &mut Server,
    mode: ModeId,
    may_redraw: bool,
    no_reset: bool,
) -> bool {
    let Some(data) = state::data(server, mode) else {
        return false;
    };
    let stopped = data.selection.cursordrag == CursorDrag::None;
    // Stopped endpoints are absolute anchors; only their screen projection changes.
    update_selection(server, mode, may_redraw, no_reset || stopped)
}

pub fn start_selection(server: &mut Server, mode: ModeId) {
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let y = data.backing_y();
    data.selection.selx = data.cx;
    data.selection.sely = y;
    data.selection.endselx = data.cx;
    data.selection.endsely = y;
    data.selection.cursordrag = CursorDrag::End;
    set_selection(server, mode, true, false);
}

pub fn clear_selection(server: &mut Server, mode: ModeId) {
    let Some((data, screen)) = state::parts_mut(server, mode) else {
        return;
    };
    screen.clear_selection();
    data.selection.active = false;
    data.selection.cursordrag = CursorDrag::None;
    data.selection.lineflag = LineSelectionDirection::None;
    data.selection.selflag = SelectionMode::Char;
    let (x, y, row, rectangle) = (data.cx, data.cy, data.backing_y(), data.selection.rectflag);
    let limit = super::motion::cursor_limit(server, mode, row, rectangle);
    if x > limit {
        update_cursor(server, mode, limit, y);
    }
}

pub fn other_end(server: &mut Server, mode: ModeId) {
    let Some((data, screen)) = state::parts_mut(server, mode) else {
        return;
    };
    if !data.selection.active && data.selection.lineflag == LineSelectionDirection::None {
        return;
    }
    data.selection.lineflag = match data.selection.lineflag {
        LineSelectionDirection::LeftToRight => LineSelectionDirection::RightToLeft,
        LineSelectionDirection::RightToLeft => LineSelectionDirection::LeftToRight,
        LineSelectionDirection::None => LineSelectionDirection::None,
    };
    data.selection.cursordrag = match data.selection.cursordrag {
        CursorDrag::End => CursorDrag::Start,
        CursorDrag::None | CursorDrag::Start => CursorDrag::End,
    };
    let (x, y) = if data.selection.cursordrag == CursorDrag::Start {
        (data.selection.selx, data.selection.sely)
    } else {
        (data.selection.endselx, data.selection.endsely)
    };
    let history = data.backing.screen().grid.hsize();
    let top = history - data.oy;
    data.cx = x;
    if y < top {
        data.oy = history - y;
        data.cy = 0;
    } else if y > top + screen.grid.sy() {
        data.oy = history.wrapping_sub(y).wrapping_add(screen.grid.sy() - 1);
        data.cy = screen.grid.sy() - 1;
    } else {
        data.cy = y - top;
    }
    let (row, rectangle) = (data.backing_y(), data.selection.rectflag);
    let limit = super::motion::cursor_limit(server, mode, row, rectangle);
    if let Some(data) = state::data_mut(server, mode) {
        data.cx = data.cx.min(limit);
    }
    update_selection(server, mode, true, true);
    super::render::redraw_screen(server, mode);
}

pub fn update_cursor(server: &mut Server, mode: ModeId, mut cx: u32, cy: u32) {
    let Some(screen) = state::screen(server, mode) else {
        return;
    };
    let (width, height) = (screen.grid.sx(), screen.grid.sy());
    let Some(data) = state::data(server, mode) else {
        return;
    };
    if !data.selection.rectflag && cy < height {
        let row = data.backing.screen().grid.hsize() + cy - data.oy;
        cx = cx.min(super::motion::cursor_limit(server, mode, row, false));
    }
    let numbers = super::render::line_numbers_active(server, mode);
    let gutter = super::render::line_number_width(server, mode);
    let cursor_line = super::render::cursor_line_active(server, mode);
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let (old_x, old_y) = (data.cx, data.cy);
    data.cx = cx;
    data.cy = cy;
    let selected = data.selection.active || data.selection.lineflag != LineSelectionDirection::None;
    if numbers {
        let content_width = width.saturating_sub(gutter).max(1);
        if selected || old_y != cy || old_x >= content_width || cx >= content_width {
            super::render::redraw_screen(server, mode);
            return;
        }
    } else {
        if old_y != cy && cursor_line {
            super::render::redraw_lines(server, mode, old_y, 1);
            super::render::redraw_lines(server, mode, cy, 1);
            return;
        }
        if old_x == width {
            super::render::redraw_lines(server, mode, old_y, 1);
        }
        if cx == width {
            super::render::redraw_lines(server, mode, cy, 1);
            return;
        }
    }
    let visible_x = super::render::cursor_offset(server, mode, cx, width);
    if let Some((_, screen)) = state::parts_mut(server, mode) {
        screen.cx = visible_x;
        screen.cy = cy;
    }
}

pub fn mouse_in_selection(server: &Server, mode: ModeId, x: u32, y: u32) -> Option<CursorDrag> {
    let data = state::data(server, mode)?;
    let screen = state::screen(server, mode)?;
    screen.selection.as_ref()?;
    let width = screen.grid.sx();
    let gutter = super::render::line_number_width(server, mode);
    let top = data.backing.screen().grid.hsize() - data.oy;
    let endpoint = |px, py| {
        py >= top
            && py - top < screen.grid.sy()
            && y == py - top
            && x == super::render::cursor_offset(server, mode, px, width)
    };
    if endpoint(data.selection.selx, data.selection.sely) {
        return Some(CursorDrag::Start);
    }
    if endpoint(data.selection.endselx, data.selection.endsely) {
        return Some(CursorDrag::End);
    }
    if (x, y)
        != (
            super::render::cursor_offset(server, mode, data.cx, width),
            data.cy,
        )
        && !screen.check_selection(x, y)
    {
        return None;
    }
    let content = width.saturating_sub(gutter).max(1);
    let mx = if gutter == 0 {
        x
    } else {
        x.saturating_sub(gutter).min(content - 1)
    };
    let stride = u64::from(width) + 1;
    let position = u64::from(top + y) * stride + u64::from(mx);
    let start = u64::from(data.selection.sely) * stride + u64::from(data.selection.selx);
    let end = u64::from(data.selection.endsely) * stride + u64::from(data.selection.endselx);
    Some(if position.abs_diff(start) <= position.abs_diff(end) {
        CursorDrag::Start
    } else {
        CursorDrag::End
    })
}

fn copy_line(data: &CopyModeData, output: &mut Vec<u8>, row: u32, mut start: u32, mut end: u32) {
    if start > end {
        return;
    }
    let grid = &data.backing.screen().grid;
    let line = grid.get_line(row);
    let wrapped = line.flags.contains(GridLineFlags::WRAPPED) && line.cellsize() <= grid.sx();
    let length = if wrapped {
        line.cellsize()
    } else {
        grid.line_length(row)
    };
    end = end.min(length);
    start = start.min(length);
    for column in start..end {
        let cell = grid.get_cell(column, row);
        if cell.flags.contains(GridCellFlags::PADDING) {
            continue;
        }
        let bytes = if cell.flags.contains(GridCellFlags::TAB) {
            &b"\t"[..]
        } else {
            cell.data.bytes()
        };
        if bytes.len() == 1
            && cell.attr.contains(GridAttributes::CHARSET)
            && let Some(acs) = rmux_emu::screen::borders::acs(bytes[0])
            && acs.len() <= cell.data.data.len()
        {
            output.extend_from_slice(acs);
        } else {
            output.extend_from_slice(bytes);
        }
    }
    if !wrapped || end != length {
        output.push(b'\n');
    }
}

fn extract_match(data: &CopyModeData) -> Option<Vec<u8>> {
    let (sx, sy, ex, ey) = super::search::match_at_cursor(data)?;
    let grid = &data.backing.screen().grid;
    let mut output = Vec::new();
    for row in sy..=ey {
        let start = if row == sy { sx } else { 0 };
        let end = if row == ey { ex } else { grid.sx() - 1 };
        for column in start..=end {
            let cell = grid.get_cell(column, row);
            if cell.flags.contains(GridCellFlags::TAB) {
                output.push(b'\t');
            } else if !cell.flags.contains(GridCellFlags::PADDING) {
                output.extend_from_slice(cell.data.bytes());
            }
        }
    }
    if output.is_empty() {
        return None;
    }
    // The C match helper returns a C string, unlike selection extraction.
    if let Some(nul) = output.iter().position(|&byte| byte == 0) {
        output.truncate(nul);
    }
    Some(output)
}

fn extract_with_keys(data: &CopyModeData, keys: ModeKeys, width: u32) -> Option<Vec<u8>> {
    if !data.selection.active && data.selection.lineflag == LineSelectionDirection::None {
        return extract_match(data);
    }
    let selection = &data.selection;
    let (sx, sy, mut ex, ey) =
        if (selection.endsely, selection.endselx) < (selection.sely, selection.selx) {
            (
                selection.endselx,
                selection.endsely,
                selection.selx,
                selection.sely,
            )
        } else {
            (
                selection.selx,
                selection.sely,
                selection.endselx,
                selection.endsely,
            )
        };
    let grid = &data.backing.screen().grid;
    let last_length = grid.line_length(ey);
    ex = ex.min(last_length);
    let (first_start, rest_start, last_end, rest_end) = if selection.rectflag {
        let anchor = if selection.cursordrag == CursorDrag::End {
            selection.selx
        } else {
            selection.endselx
        };
        if anchor < data.cx {
            let end = data.cx + u32::from(keys == ModeKeys::Vi);
            (anchor, anchor, end, end)
        } else {
            (data.cx, data.cx, anchor + 1, anchor + 1)
        }
    } else {
        (sx, 0, ex + u32::from(keys == ModeKeys::Vi), width)
    };
    let mut output = Vec::new();
    for row in sy..=ey {
        copy_line(
            data,
            &mut output,
            row,
            if row == sy { first_start } else { rest_start },
            if row == ey { last_end } else { rest_end },
        );
    }
    if output.is_empty() {
        return None;
    }
    if (keys == ModeKeys::Emacs || last_end <= last_length)
        && (!grid.get_line(ey).flags.contains(GridLineFlags::WRAPPED) || last_end != last_length)
    {
        output.pop();
    }
    Some(output)
}

pub fn extract_selection(data: &CopyModeData) -> Option<Vec<u8>> {
    extract_with_keys(data, data.modekeys, data.backing.screen().grid.sx())
}

pub fn get_selection(server: &mut Server, mode: ModeId) -> Option<Vec<u8>> {
    let keys = current_keys(server, mode)?;
    let width = state::screen(server, mode)?.grid.sx();
    extract_with_keys(state::data(server, mode)?, keys, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ArenaId, PaneId};
    use rmux_emu::cell::DEFAULT_CELL;
    use rmux_emu::colour::Colour;
    use rmux_emu::hyperlinks::HyperlinkRegistry;
    use rmux_emu::screen::ScreenResetPolicy;
    use rmux_util::utf8::Utf8Data;

    fn screen(width: u32, height: u32) -> Screen {
        Screen::new(
            width,
            height,
            100,
            ScreenResetPolicy::default(),
            &mut HyperlinkRegistry::new(),
        )
        .unwrap()
    }

    fn fixture(rows: &[&[u8]], width: u32, keys: ModeKeys) -> CopyModeData {
        let mut backing = screen(width, rows.len() as u32);
        for (y, bytes) in rows.iter().enumerate() {
            for (x, &byte) in bytes.iter().enumerate() {
                backing.grid.set_cell(
                    x as u32,
                    y as u32,
                    &GridCell {
                        data: Utf8Data::set(byte),
                        ..DEFAULT_CELL
                    },
                );
            }
        }
        CopyModeData::new(
            super::super::state::CopyBacking::Snapshot(backing),
            PaneId::from_parts(0, 0),
            keys,
        )
    }

    fn select(data: &mut CopyModeData, start: (u32, u32), end: (u32, u32)) {
        data.selection.active = true;
        (data.selection.selx, data.selection.sely) = start;
        (data.selection.endselx, data.selection.endsely) = end;
        (data.cx, data.cy) = end;
        data.selection.cursordrag = CursorDrag::End;
    }

    #[test]
    fn linear_endpoint_order_and_final_newline_matrix() {
        for keys in [ModeKeys::Emacs, ModeKeys::Vi] {
            let mut data = fixture(&[b"abcdef", b"GHIJKL"], 12, keys);
            for reverse in [false, true] {
                let (start, end) = if reverse {
                    ((3, 1), (1, 0))
                } else {
                    ((1, 0), (3, 1))
                };
                select(&mut data, start, end);
                assert_eq!(
                    extract_selection(&data).unwrap(),
                    if keys == ModeKeys::Vi {
                        b"bcdef\nGHIJ".as_slice()
                    } else {
                        b"bcdef\nGHI".as_slice()
                    }
                );
            }
            select(&mut data, (0, 0), (6, 0));
            assert_eq!(
                extract_selection(&data).unwrap(),
                if keys == ModeKeys::Vi {
                    b"abcdef\n".as_slice()
                } else {
                    b"abcdef".as_slice()
                }
            );
            select(&mut data, (2, 0), (2, 0));
            assert_eq!(
                extract_selection(&data).unwrap(),
                if keys == ModeKeys::Vi {
                    b"c".as_slice()
                } else {
                    b"".as_slice()
                }
            );
        }
        let mut empty = fixture(&[b""], 12, ModeKeys::Emacs);
        select(&mut empty, (0, 0), (0, 0));
        assert_eq!(extract_selection(&empty), Some(Vec::new()));
        empty.selection.active = false;
        assert_eq!(extract_selection(&empty), None);
    }

    #[test]
    fn rectangles_keep_the_cursor_side_rule_when_stopped_or_exchanged() {
        for keys in [ModeKeys::Emacs, ModeKeys::Vi] {
            let mut data = fixture(&[b"abcdef", b"GHIJKL"], 12, keys);
            select(&mut data, (1, 0), (3, 1));
            data.selection.rectflag = true;
            assert_eq!(
                extract_selection(&data).unwrap(),
                if keys == ModeKeys::Vi {
                    b"bcd\nHIJ".as_slice()
                } else {
                    b"bc\nHI".as_slice()
                }
            );
            select(&mut data, (3, 0), (1, 1));
            assert_eq!(extract_selection(&data).unwrap(), b"bcd\nHIJ");
            data.selection.cursordrag = CursorDrag::None;
            assert_eq!(extract_selection(&data).unwrap(), b"b\nH");
            data.selection.cursordrag = CursorDrag::Start;
            data.cx = 3;
            assert_eq!(
                extract_selection(&data).unwrap(),
                if keys == ModeKeys::Vi {
                    b"bcd\nHIJ".as_slice()
                } else {
                    b"bc\nHI".as_slice()
                }
            );
        }
    }

    #[test]
    fn copy_line_preserves_soft_wrap_spaces_and_clamps_partial_rows() {
        let mut data = fixture(&[b"ab  ", b"cd"], 4, ModeKeys::Emacs);
        data.backing
            .screen_mut()
            .grid
            .get_line_mut(0)
            .flags
            .insert(GridLineFlags::WRAPPED);
        select(&mut data, (0, 0), (2, 1));
        assert_eq!(extract_selection(&data).unwrap(), b"ab  cd");
        let mut output = Vec::new();
        copy_line(&data, &mut output, 0, 0, 2);
        assert_eq!(output, b"ab\n");
        output.clear();
        copy_line(&data, &mut output, 0, 3, 2);
        assert!(output.is_empty());
        output.clear();
        copy_line(&data, &mut output, 0, 4, 8);
        assert!(output.is_empty());
        data.backing
            .screen_mut()
            .grid
            .get_line_mut(0)
            .flags
            .remove(GridLineFlags::WRAPPED);
        output.clear();
        copy_line(&data, &mut output, 0, 0, 4);
        assert_eq!(output, b"ab\n");
        output.clear();
        copy_line(&data, &mut output, 0, 8, 9);
        assert_eq!(output, b"\n");
    }

    #[test]
    fn final_newline_uses_wrapped_flag_not_full_wrap_predicate() {
        let mut data = fixture(&[b"abcd"], 4, ModeKeys::Emacs);
        data.backing
            .screen_mut()
            .grid
            .get_line_mut(0)
            .flags
            .insert(GridLineFlags::WRAPPED);
        select(&mut data, (0, 0), (4, 0));
        assert_eq!(extract_selection(&data).unwrap(), b"abcd");
        data.backing.screen_mut().grid.set_cell(
            4,
            0,
            &GridCell {
                data: Utf8Data::set(b'z'),
                ..DEFAULT_CELL
            },
        );
        assert!(data.backing.screen().grid.get_line(0).cellsize() > 4);
        assert_eq!(extract_selection(&data).unwrap(), b"abcd\n");
        data.selection.endselx = 2;
        assert_eq!(extract_selection(&data).unwrap(), b"ab");
    }

    #[test]
    fn padding_tabs_acs_combining_and_raw_bytes_extract_without_loss() {
        let mut data = fixture(&[b""], 12, ModeKeys::Emacs);
        let grid = &mut data.backing.screen_mut().grid;
        let mut wide = DEFAULT_CELL;
        wide.data.data[..3].copy_from_slice("界".as_bytes());
        wide.data.size = 3;
        wide.data.width = 2;
        grid.set_cell(0, 0, &wide);
        grid.set_padding(1, 0, Colour::DEFAULT);
        let mut tab = DEFAULT_CELL;
        tab.set_tab(3);
        grid.set_cell(2, 0, &tab);
        grid.set_padding(3, 0, Colour::DEFAULT);
        grid.set_padding(4, 0, Colour::DEFAULT);
        grid.set_cell(
            5,
            0,
            &GridCell {
                data: Utf8Data::set(b'q'),
                attr: GridAttributes::CHARSET,
                ..DEFAULT_CELL
            },
        );
        let mut combined = DEFAULT_CELL;
        combined.data.data[..3].copy_from_slice(b"e\xcc\x81");
        combined.data.size = 3;
        grid.set_cell(6, 0, &combined);
        grid.set_cell(
            7,
            0,
            &GridCell {
                data: Utf8Data::set(0xff),
                ..DEFAULT_CELL
            },
        );
        grid.set_cell(
            8,
            0,
            &GridCell {
                data: Utf8Data::set(0),
                ..DEFAULT_CELL
            },
        );
        select(&mut data, (0, 0), (9, 0));
        assert_eq!(
            extract_selection(&data).unwrap(),
            ["界\t─e\u{301}".as_bytes(), &[0xff, 0]].concat()
        );
        select(&mut data, (1, 0), (5, 0));
        assert_eq!(extract_selection(&data).unwrap(), b"\t");
    }

    #[test]
    fn match_copy_accepts_one_after_and_does_not_translate_acs_or_newlines() {
        let mut data = fixture(&[b"aqbc", b"de"], 4, ModeKeys::Emacs);
        data.backing.screen_mut().grid.set_cell(
            1,
            0,
            &GridCell {
                data: Utf8Data::set(b'q'),
                attr: GridAttributes::CHARSET,
                ..DEFAULT_CELL
            },
        );
        data.search.marks = Some(vec![0, 1, 1, 1, 1, 1, 0, 0]);
        data.cx = 1;
        assert_eq!(extract_selection(&data).unwrap(), b"qbcde");
        data.cy = 1;
        data.cx = 2;
        assert_eq!(extract_selection(&data).unwrap(), b"qbcde");
        data.cx = 3;
        assert_eq!(extract_selection(&data), None);
    }

    #[test]
    fn stopped_projection_hides_clips_and_never_reanchors_absolute_endpoints() {
        let mut data = fixture(&[b"a", b"b", b"c"], 12, ModeKeys::Emacs);
        for _ in 0..8 {
            data.backing
                .screen_mut()
                .grid
                .scroll_history(Colour::DEFAULT);
        }
        select(&mut data, (3, 2), (7, 6));
        data.selection.cursordrag = CursorDrag::None;
        data.selection.selflag = SelectionMode::Line;
        let original = (
            data.selection.selx,
            data.selection.sely,
            data.selection.endselx,
            data.selection.endsely,
        );
        let mut visible = screen(12, 3);
        for offset in [8, 6, 5, 3, 1, 0] {
            data.oy = offset;
            synchronize_cursor(&mut data, ModeKeys::Emacs, true);
            let shown = project_selection(&mut data, &mut visible, 0, DEFAULT_CELL);
            assert_eq!(
                original,
                (
                    data.selection.selx,
                    data.selection.sely,
                    data.selection.endselx,
                    data.selection.endsely
                )
            );
            assert_eq!(shown, offset != 1 && offset != 0);
            if shown {
                let projected = visible.selection.as_ref().unwrap();
                assert!(projected.sy < 3 && projected.ey < 3);
            } else {
                assert!(visible.selection.as_ref().unwrap().hidden);
                assert!(data.selection.active);
            }
        }
        data.oy = 5;
        data.selection.rectflag = true;
        project_selection(&mut data, &mut visible, 0, DEFAULT_CELL);
        let projected = visible.selection.as_ref().unwrap();
        assert_eq!((projected.sx, projected.ex), (3, 7));
        assert_eq!((projected.sy, projected.ey), (0, 2));
    }

    #[test]
    fn projection_excludes_gutters_even_on_one_column_screen() {
        let mut data = fixture(&[b"abcdef", b"GHIJKL"], 12, ModeKeys::Vi);
        select(&mut data, (0, 0), (5, 1));
        for width in [1, 2, 4, 8, 12] {
            for gutter in [0, 1, 3, 12] {
                let mut visible = screen(width, 2);
                project_selection(&mut data, &mut visible, gutter, DEFAULT_CELL);
                let selection = visible.selection.as_ref().unwrap();
                assert_eq!(selection.clipx, gutter.min(width - 1));
                for x in 0..selection.clipx {
                    assert!(!visible.check_selection(x, 0));
                    assert!(!visible.check_selection(x, 1));
                }
                assert_eq!(extract_selection(&data).unwrap(), b"abcdef\nGHIJKL");
            }
        }
    }

    #[test]
    fn word_reversal_restores_reset_word_and_no_reset_bypasses_expansion() {
        for keys in [ModeKeys::Emacs, ModeKeys::Vi] {
            let mut data = fixture(&[b"one two three"], 20, keys);
            select(&mut data, (4, 0), (7, 0));
            data.selection.selflag = SelectionMode::Word;
            data.selection.dx = 5;
            data.selection.dy = 0;
            data.selection.selrx = 4;
            data.selection.endselrx = 7;
            data.cx = 9;
            synchronize_cursor(&mut data, keys, false);
            assert_eq!(data.selection.selx, 4);
            assert_eq!(
                data.selection.endselx,
                if keys == ModeKeys::Vi { 12 } else { 13 }
            );
            data.cx = 1;
            synchronize_cursor(&mut data, keys, false);
            assert_eq!((data.selection.selx, data.selection.endselx), (0, 7));
            data.cx = 6;
            synchronize_cursor(&mut data, keys, false);
            assert_eq!((data.selection.selx, data.selection.endselx), (4, 6));
            data.cx = 9;
            synchronize_cursor(&mut data, keys, true);
            assert_eq!((data.selection.selx, data.selection.endselx), (4, 9));
            data.selection.cursordrag = CursorDrag::Start;
            data.cx = 2;
            synchronize_cursor(&mut data, keys, true);
            assert_eq!((data.selection.selx, data.selection.endselx), (2, 9));
        }
    }

    #[test]
    fn line_reversal_restores_reset_lines_and_does_not_shrink_initial_end() {
        let mut data = fixture(&[b"one", b"two", b"three", b"four"], 12, ModeKeys::Emacs);
        select(&mut data, (0, 1), (5, 2));
        data.selection.selflag = SelectionMode::Line;
        data.selection.dy = 1;
        data.selection.selry = 1;
        data.selection.endselry = 2;
        data.selection.endselrx = 5;
        data.cy = 3;
        synchronize_cursor(&mut data, ModeKeys::Emacs, false);
        assert_eq!((data.selection.endselx, data.selection.endsely), (4, 3));
        data.cy = 0;
        synchronize_cursor(&mut data, ModeKeys::Emacs, false);
        assert_eq!(
            (
                data.selection.selx,
                data.selection.sely,
                data.selection.endselx,
                data.selection.endsely
            ),
            (0, 0, 5, 2)
        );
        data.cy = 1;
        synchronize_cursor(&mut data, ModeKeys::Emacs, false);
        assert_eq!(
            (
                data.selection.sely,
                data.selection.endselx,
                data.selection.endsely
            ),
            (1, 5, 2)
        );
    }

    fn runtime_fixture(keys: ModeKeys) -> (Server, ModeId) {
        use super::super::{CopyModeDriver, CopyModeKind};
        use crate::model::{pane, window};
        use crate::modes::WindowModeFlags;
        let mut server = Server::default();
        let window = window::window_create(&mut server, 12, 3, 0, 0).unwrap();
        let pane = pane::pane_create(&mut server, window, 12, 3, 100).unwrap();
        let options = server.windows.get(window).unwrap().options;
        server
            .options
            .set_number_value(options, b"mode-keys", i64::from(keys == ModeKeys::Vi));
        server
            .options
            .set_number_value(options, b"copy-mode-line-numbers", 0);
        let mode = ModeId::new(pane, 0, 0);
        let mut data = fixture(&[b"abcdef", b"GHIJKL", b"mnopqr"], 12, keys);
        data.source = pane;
        server
            .panes
            .get_mut(pane)
            .unwrap()
            .modes
            .push(pane::PaneMode {
                id: mode,
                name: b"copy-mode".to_vec(),
                flags: WindowModeFlags::default(),
                prefix: 1,
                kill: false,
                screen: Some(screen(12, 3)),
                data: Some(Box::new(data)),
                driver: std::rc::Rc::new(CopyModeDriver {
                    kind: CopyModeKind::Copy {
                        source: None,
                        args: crate::cmd::arguments::Args::default(),
                    },
                }),
            });
        (server, mode)
    }

    #[test]
    fn lifecycle_cursor_limits_and_live_extraction_keys() {
        let (mut server, mode) = runtime_fixture(ModeKeys::Emacs);
        assert!(!update_selection(&mut server, mode, false, false));
        update_cursor(&mut server, mode, 2, 0);
        start_selection(&mut server, mode);
        update_cursor(&mut server, mode, 4, 0);
        assert!(update_selection(&mut server, mode, false, false));
        assert_eq!(get_selection(&mut server, mode).unwrap(), b"cd");
        let window = server.panes.get(mode.owner).unwrap().window;
        let options = server.windows.get(window).unwrap().options;
        server.options.set_number_value(options, b"mode-keys", 1);
        assert_eq!(get_selection(&mut server, mode).unwrap(), b"cde");
        assert_eq!(
            state::screen(&server, mode)
                .unwrap()
                .selection
                .as_ref()
                .unwrap()
                .modekeys,
            0
        );
        state::data_mut(&mut server, mode)
            .unwrap()
            .selection
            .rectflag = true;
        update_cursor(&mut server, mode, 10, 0);
        assert_eq!(state::data(&server, mode).unwrap().cx, 10);
        clear_selection(&mut server, mode);
        let data = state::data(&server, mode).unwrap();
        assert!(data.selection.rectflag);
        assert!(!data.selection.active);
        assert_eq!(data.cx, 6);
        assert_eq!(data.selection.cursordrag, CursorDrag::None);
        assert_eq!(data.selection.selflag, SelectionMode::Char);
        assert!(state::screen(&server, mode).unwrap().selection.is_none());
        state::data_mut(&mut server, mode)
            .unwrap()
            .selection
            .rectflag = false;
        update_cursor(&mut server, mode, 10, 0);
        assert_eq!(state::data(&server, mode).unwrap().cx, 5);
    }

    #[test]
    fn endpoint_exchange_restarts_stopped_drag_and_hit_testing_ties_choose_start() {
        let (mut server, mode) = runtime_fixture(ModeKeys::Emacs);
        update_cursor(&mut server, mode, 1, 0);
        start_selection(&mut server, mode);
        update_cursor(&mut server, mode, 5, 0);
        update_selection(&mut server, mode, false, false);
        assert_eq!(
            mouse_in_selection(&server, mode, 1, 0),
            Some(CursorDrag::Start)
        );
        assert_eq!(
            mouse_in_selection(&server, mode, 5, 0),
            Some(CursorDrag::End)
        );
        assert_eq!(
            mouse_in_selection(&server, mode, 3, 0),
            Some(CursorDrag::Start)
        );
        assert_eq!(
            mouse_in_selection(&server, mode, 4, 0),
            Some(CursorDrag::End)
        );
        assert_eq!(mouse_in_selection(&server, mode, 0, 0), None);
        other_end(&mut server, mode);
        assert_eq!(state::data(&server, mode).unwrap().cx, 1);
        assert_eq!(
            state::data(&server, mode).unwrap().selection.cursordrag,
            CursorDrag::Start
        );
        state::data_mut(&mut server, mode)
            .unwrap()
            .selection
            .cursordrag = CursorDrag::None;
        other_end(&mut server, mode);
        assert_eq!(state::data(&server, mode).unwrap().cx, 5);
        assert_eq!(
            state::data(&server, mode).unwrap().selection.cursordrag,
            CursorDrag::End
        );
        assert_eq!(get_selection(&mut server, mode).unwrap(), b"bcde");
    }

    #[test]
    fn other_end_scrolls_to_absolute_stopped_endpoints_above_and_below_view() {
        let (mut server, mode) = runtime_fixture(ModeKeys::Emacs);
        let data = state::data_mut(&mut server, mode).unwrap();
        for _ in 0..8 {
            data.backing
                .screen_mut()
                .grid
                .scroll_history(Colour::DEFAULT);
        }
        select(data, (0, 2), (0, 7));
        data.selection.cursordrag = CursorDrag::End;
        data.selection.lineflag = LineSelectionDirection::LeftToRight;
        data.oy = 2;
        data.cy = 1;
        set_selection(&mut server, mode, false, true);
        other_end(&mut server, mode);
        let data = state::data(&server, mode).unwrap();
        assert_eq!((data.oy, data.cy), (6, 0));
        assert_eq!(data.selection.lineflag, LineSelectionDirection::RightToLeft);
        other_end(&mut server, mode);
        let data = state::data(&server, mode).unwrap();
        assert_eq!((data.oy, data.cy), (3, 2));
        assert_eq!(data.selection.lineflag, LineSelectionDirection::LeftToRight);
        assert_eq!((data.selection.sely, data.selection.endsely), (2, 7));
        state::data_mut(&mut server, mode)
            .unwrap()
            .selection
            .cursordrag = CursorDrag::None;
        let original = get_selection(&mut server, mode);
        state::data_mut(&mut server, mode).unwrap().oy = 0;
        assert!(!update_selection_view(&mut server, mode, false, false));
        assert_eq!(get_selection(&mut server, mode), original);
        assert!(
            state::screen(&server, mode)
                .unwrap()
                .selection
                .as_ref()
                .unwrap()
                .hidden
        );
    }

    #[test]
    fn word_granularity_follows_soft_wraps_and_unicode_whitespace() {
        let mut data = fixture(&[b"abcd", b"ef g"], 4, ModeKeys::Emacs);
        data.backing
            .screen_mut()
            .grid
            .get_line_mut(0)
            .flags
            .insert(GridLineFlags::WRAPPED);
        select(&mut data, (0, 0), (2, 0));
        data.selection.selflag = SelectionMode::Word;
        synchronize_cursor(&mut data, ModeKeys::Emacs, false);
        assert_eq!((data.selection.endselx, data.selection.endsely), (2, 1));
        data.selection.dx = 3;
        data.selection.dy = 1;
        data.selection.endselrx = 4;
        data.selection.endselry = 1;
        data.cx = 1;
        data.cy = 1;
        synchronize_cursor(&mut data, ModeKeys::Emacs, false);
        assert_eq!((data.selection.selx, data.selection.sely), (0, 0));
        let mut data = fixture(&[b"ab"], 12, ModeKeys::Emacs);
        let mut whitespace = DEFAULT_CELL;
        whitespace.data.data[..2].copy_from_slice("\u{a0}".as_bytes());
        whitespace.data.size = 2;
        data.backing.screen_mut().grid.set_cell(2, 0, &whitespace);
        data.backing.screen_mut().grid.set_cell(
            3,
            0,
            &GridCell {
                data: Utf8Data::set(b'c'),
                ..DEFAULT_CELL
            },
        );
        select(&mut data, (0, 0), (1, 0));
        data.selection.selflag = SelectionMode::Word;
        synchronize_cursor(&mut data, ModeKeys::Emacs, false);
        assert_eq!(data.selection.endselx, 1);
    }

    #[test]
    fn rectangle_adapter_clamps_and_reprojects_without_clearing_rectangle_on_clear() {
        let (mut server, mode) = runtime_fixture(ModeKeys::Vi);
        update_cursor(&mut server, mode, 1, 0);
        start_selection(&mut server, mode);
        super::super::motion::rectangle_set(&mut server, mode, true);
        update_cursor(&mut server, mode, 10, 1);
        update_selection(&mut server, mode, true, false);
        assert_eq!(state::data(&server, mode).unwrap().cx, 10);
        assert!(
            state::screen(&server, mode)
                .unwrap()
                .selection
                .as_ref()
                .unwrap()
                .rectangle
        );
        super::super::motion::rectangle_set(&mut server, mode, false);
        assert_eq!(state::data(&server, mode).unwrap().cx, 5);
        assert!(
            !state::screen(&server, mode)
                .unwrap()
                .selection
                .as_ref()
                .unwrap()
                .rectangle
        );
        assert_eq!(get_selection(&mut server, mode).unwrap(), b"bcdef\nGHIJKL");
    }

    #[test]
    fn stopped_view_updates_preserve_word_reset_coordinates_and_extraction() {
        let (mut server, mode) = runtime_fixture(ModeKeys::Emacs);
        let data = state::data_mut(&mut server, mode).unwrap();
        for _ in 0..6 {
            data.backing
                .screen_mut()
                .grid
                .scroll_history(Colour::DEFAULT);
        }
        select(data, (1, 0), (3, 1));
        data.selection.cursordrag = CursorDrag::None;
        data.selection.selflag = SelectionMode::Word;
        data.oy = 6;
        assert!(set_selection(&mut server, mode, false, false));
        let copied = get_selection(&mut server, mode).unwrap();
        for offset in [5, 3, 0, 6] {
            state::data_mut(&mut server, mode).unwrap().oy = offset;
            update_selection_view(&mut server, mode, false, false);
            let data = state::data(&server, mode).unwrap();
            assert_eq!(
                (
                    data.selection.selx,
                    data.selection.sely,
                    data.selection.endselx,
                    data.selection.endsely
                ),
                (1, 0, 3, 1)
            );
            assert_eq!(get_selection(&mut server, mode).unwrap(), copied);
        }
    }

    #[test]
    fn pinned_oracle_linear_rectangle_and_stopped_selection_bytes() {
        use std::path::PathBuf;
        use std::process::Command;
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::var_os("RMUX_ORACLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux")
            });
        if !path.is_file() {
            eprintln!(
                "skip selection differential: pinned oracle missing at {}",
                path.display()
            );
            return;
        }
        struct Oracle {
            path: PathBuf,
            directory: PathBuf,
            socket: PathBuf,
        }
        impl Oracle {
            fn command(&self, args: &[&str]) -> Vec<u8> {
                let result = Command::new(&self.path)
                    .arg("-S")
                    .arg(&self.socket)
                    .args(["-f", "/dev/null"])
                    .args(args)
                    .env_remove("TMUX")
                    .env("TERM", "screen")
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "oracle {args:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                result.stdout
            }
            fn action(&self, name: &str, count: u32) {
                self.command(&["send-keys", "-N", &count.to_string(), "-X", name]);
            }
            fn cursor(&self, x: u32, y: u32) {
                self.action("history-top", 1);
                if y != 0 {
                    self.action("cursor-down", y);
                }
                if x != 0 {
                    self.action("cursor-right", x);
                }
            }
        }
        impl Drop for Oracle {
            fn drop(&mut self) {
                let _ = Command::new(&self.path)
                    .arg("-S")
                    .arg(&self.socket)
                    .arg("kill-server")
                    .output();
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }
        let directory = std::env::temp_dir().join(format!(
            "rmux-copy-selection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let oracle = Oracle {
            path,
            socket: directory.join("s"),
            directory,
        };
        oracle.command(&[
            "new-session",
            "-d",
            "-x",
            "20",
            "-y",
            "6",
            "printf 'abcdef\\r\\nGHIJKL'; exec sleep 60",
        ]);
        let mut ready = false;
        for _ in 0..100 {
            if oracle
                .command(&["capture-pane", "-p"])
                .windows(6)
                .any(|bytes| bytes == b"GHIJKL")
            {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready, "oracle output did not become ready");
        for keys in [ModeKeys::Emacs, ModeKeys::Vi] {
            oracle.command(&[
                "set-option",
                "-w",
                "mode-keys",
                if keys == ModeKeys::Vi { "vi" } else { "emacs" },
            ]);
            oracle.command(&["copy-mode"]);
            for rectangle in [false, true] {
                for reverse in [false, true] {
                    oracle.action("clear-selection", 1);
                    oracle.action(
                        if rectangle {
                            "rectangle-on"
                        } else {
                            "rectangle-off"
                        },
                        1,
                    );
                    let (start, end) = if reverse {
                        ((3, 1), (1, 0))
                    } else {
                        ((1, 0), (3, 1))
                    };
                    oracle.cursor(start.0, start.1);
                    oracle.action("begin-selection", 1);
                    oracle.cursor(end.0, end.1);
                    oracle.action("copy-selection-no-clear", 1);
                    let mut data = fixture(&[b"abcdef", b"GHIJKL", b"", b"", b"", b""], 20, keys);
                    select(&mut data, start, end);
                    data.selection.rectflag = rectangle;
                    let mut extracted = extract_selection(&data).unwrap();
                    if std::env::var_os("RMUX_COPY_SELECTION_MUTATE").is_some() {
                        extracted[0] ^= 1;
                    }
                    assert_eq!(
                        extracted,
                        oracle.command(&["save-buffer", "-"]),
                        "keys={keys:?}, rectangle={rectangle}, reverse={reverse}"
                    );
                    oracle.action("stop-selection", 1);
                    data.selection.cursordrag = CursorDrag::None;
                    oracle.action("copy-selection-no-clear", 1);
                    assert_eq!(
                        extract_selection(&data).unwrap(),
                        oracle.command(&["save-buffer", "-"]),
                        "stopped keys={keys:?}, rectangle={rectangle}, reverse={reverse}"
                    );
                    oracle.action("other-end", 1);
                    oracle.action("other-end", 1);
                    data.selection.cursordrag = CursorDrag::Start;
                    (data.cx, data.cy) = start;
                    oracle.action("copy-selection-no-clear", 1);
                    assert_eq!(
                        extract_selection(&data).unwrap(),
                        oracle.command(&["save-buffer", "-"]),
                        "exchanged keys={keys:?}, rectangle={rectangle}, reverse={reverse}"
                    );
                }
            }
            oracle.action("clear-selection", 1);
            oracle.action("rectangle-off", 1);
            oracle.cursor(0, 0);
            oracle.action("begin-selection", 1);
            oracle.action("end-of-line", 1);
            oracle.action("copy-selection-no-clear", 1);
            let mut data = fixture(&[b"abcdef", b"GHIJKL", b"", b"", b"", b""], 20, keys);
            select(
                &mut data,
                (0, 0),
                (if keys == ModeKeys::Vi { 5 } else { 6 }, 0),
            );
            assert_eq!(
                extract_selection(&data).unwrap(),
                oracle.command(&["save-buffer", "-"])
            );
            oracle.action("cancel", 1);
        }
        oracle.command(&[
            "new-window",
            "-n",
            "cells",
            "printf '界\\t\\033(0q\\033(B e\\314\\201\\r\\nGHIJKL'; exec sleep 60",
        ]);
        let mut ready = false;
        for _ in 0..100 {
            if oracle
                .command(&["capture-pane", "-p"])
                .windows(6)
                .any(|bytes| bytes == b"GHIJKL")
            {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready, "oracle styled output did not become ready");
        for keys in [ModeKeys::Emacs, ModeKeys::Vi] {
            oracle.command(&[
                "set-option",
                "-w",
                "mode-keys",
                if keys == ModeKeys::Vi { "vi" } else { "emacs" },
            ]);
            oracle.command(&["copy-mode"]);
            oracle.cursor(0, 0);
            oracle.action("begin-selection", 1);
            oracle.action("end-of-line", 1);
            oracle.action("copy-selection-no-clear", 1);
            let mut data = fixture(&[b"", b"GHIJKL", b"", b"", b"", b""], 20, keys);
            let grid = &mut data.backing.screen_mut().grid;
            let mut wide = DEFAULT_CELL;
            wide.data.data[..3].copy_from_slice("界".as_bytes());
            wide.data.size = 3;
            wide.data.width = 2;
            grid.set_cell(0, 0, &wide);
            grid.set_padding(1, 0, Colour::DEFAULT);
            let mut tab = DEFAULT_CELL;
            tab.set_tab(6);
            grid.set_cell(2, 0, &tab);
            for column in 3..8 {
                grid.set_padding(column, 0, Colour::DEFAULT);
            }
            grid.set_cell(
                8,
                0,
                &GridCell {
                    data: Utf8Data::set(b'q'),
                    attr: GridAttributes::CHARSET,
                    ..DEFAULT_CELL
                },
            );
            grid.set_cell(9, 0, &DEFAULT_CELL);
            let mut combined = DEFAULT_CELL;
            combined.data.data[..3].copy_from_slice(b"e\xcc\x81");
            combined.data.size = 3;
            grid.set_cell(10, 0, &combined);
            select(
                &mut data,
                (0, 0),
                (if keys == ModeKeys::Vi { 10 } else { 11 }, 0),
            );
            assert_eq!(
                extract_selection(&data).unwrap(),
                oracle.command(&["save-buffer", "-"]),
                "styled cells keys={keys:?}"
            );
            oracle.action("cancel", 1);
        }
    }
}
