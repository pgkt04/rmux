// Ported from tmux window-copy.c, window.c @ 8f25579c
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
    self, CopyLineNumbers, CopyModeData, CursorDrag, ModeKeys, SearchDirection, SelectionMode,
};
use crate::format::{FormatTree, FormatValue};
use crate::ids::{ModeId, OptionsId, PaneId};
use crate::model::pane::{pane_scrollbar_overlay, pane_scrollbar_visible};
use crate::model::{PaneFlags, Server};
use crate::ui::{fanout, styles};
use rmux_emu::cell::{DEFAULT_CELL, GridCell, GridCellFlags};
use rmux_emu::colour::Colour;
use rmux_emu::screen::Screen;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_util::bytes::ByteString;
use rmux_util::utf8::Utf8Data;

fn options(server: &Server, mode: ModeId) -> OptionsId {
    let pane = server.panes.get(mode.owner).expect("copy pane exists");
    server
        .windows
        .get(pane.window)
        .expect("copy window exists")
        .options
}

fn effective_line_number_mode(setting: CopyLineNumbers, option: i32) -> i32 {
    match setting {
        CopyLineNumbers::Off => 0,
        CopyLineNumbers::Default if option == 0 => 1,
        CopyLineNumbers::Option | CopyLineNumbers::Default => option,
    }
}

pub fn line_number_mode(server: &Server, mode: ModeId) -> i32 {
    let Some(data) = state::data(server, mode) else {
        return 0;
    };
    effective_line_number_mode(
        data.line_numbers,
        server
            .options
            .get_number(options(server, mode), b"copy-mode-line-numbers") as i32,
    )
}

pub fn line_number_is_absolute(server: &Server, mode: ModeId) -> bool {
    match line_number_mode(server, mode) {
        0 | 1 => false,
        2..=4 => true,
        _ => panic!("bad line number mode"),
    }
}

pub fn line_numbers_active(server: &Server, mode: ModeId) -> bool {
    line_number_mode(server, mode) != 0
}

pub fn cursor_line_active(server: &Server, mode: ModeId) -> bool {
    state::data(server, mode).is_some()
        && server
            .options
            .get_string(options(server, mode), b"copy-mode-current-line-style")
            != b"default"
}

fn gutter_width(history: u32, height: u32) -> u32 {
    let mut lines = history
        .checked_add(height)
        .and_then(|n| n.checked_add(1))
        .expect("copy line count overflow");
    let mut digits = 1;
    while lines >= 10 {
        lines /= 10;
        digits += 1;
    }
    digits.max(3) + 1
}

pub fn line_number_width(server: &Server, mode: ModeId) -> u32 {
    if !line_numbers_active(server, mode) {
        return 0;
    }
    let grid = &state::data(server, mode)
        .expect("copy data exists")
        .backing
        .screen()
        .grid;
    gutter_width(grid.hsize(), grid.sy())
}

fn content_width(width: u32, sx: u32) -> u32 {
    if width >= sx { 1 } else { sx - width }
}

pub fn offset_for_gutter(width: u32, cx: u32, sx: u32) -> u32 {
    assert!(sx != 0, "copy screen width is nonzero");
    if width == 0 {
        return cx;
    }
    if cx >= content_width(width, sx) {
        sx - 1
    } else {
        width + cx
    }
}

fn unoffset_for_gutter(width: u32, mut vx: u32, sx: u32) -> u32 {
    assert!(sx != 0, "copy screen width is nonzero");
    if width == 0 {
        return vx;
    }
    if vx < width {
        return 0;
    }
    vx -= width;
    vx.min(content_width(width, sx) - 1)
}

pub fn cursor_offset(server: &Server, mode: ModeId, cx: u32, sx: u32) -> u32 {
    offset_for_gutter(line_number_width(server, mode), cx, sx)
}

pub fn cursor_unoffset(server: &Server, mode: ModeId, vx: u32, sx: u32) -> u32 {
    unoffset_for_gutter(line_number_width(server, mode), vx, sx)
}

pub fn set_line_numbers(server: &mut Server, pane: PaneId, enabled: bool) {
    let Some(mode) = server
        .panes
        .get(pane)
        .and_then(|p| p.modes.first())
        .filter(|m| m.name == b"copy-mode")
        .map(|m| m.id)
    else {
        return;
    };
    set_line_numbers1(server, mode, enabled, false);
}

pub fn set_line_numbers1(server: &mut Server, mode: ModeId, enabled: bool, force: bool) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let active = line_numbers_active(server, mode);
    let setting = if !enabled {
        CopyLineNumbers::Off
    } else if force
        && server
            .options
            .get_number(options(server, mode), b"copy-mode-line-numbers")
            == 0
    {
        CopyLineNumbers::Default
    } else {
        CopyLineNumbers::Option
    };
    if data.line_numbers == setting && active == enabled {
        return;
    }
    state::data_mut(server, mode)
        .expect("copy data exists")
        .line_numbers = setting;
    redraw_screen(server, mode);
}

/// Options and expanded position bytes are captured before borrowing the two screens.
pub struct CopyRenderStyle {
    pub position: GridCell,
    pub matched: GridCell,
    pub current_match: GridCell,
    pub mark: GridCell,
    pub current_line: GridCell,
    pub line_number: GridCell,
    pub current_line_number: GridCell,
    pub gutter_width: u32,
    pub line_number_mode: i32,
    pub modekeys: ModeKeys,
    pub position_format: ByteString,
}

fn apply_style(server: &mut Server, oo: OptionsId, tree: &mut FormatTree, name: &[u8]) -> GridCell {
    let mut cell = DEFAULT_CELL;
    styles::style_apply(server, &mut cell, oo, name, Some(tree));
    cell.flags.insert(GridCellFlags::NOPALETTE);
    cell
}

pub fn render_style(server: &mut Server, mode: ModeId) -> CopyRenderStyle {
    let oo = options(server, mode);
    let width = line_number_width(server, mode);
    let mut tree = styles::create_defaults(server, None, None, None, None, Some(mode.owner));
    formats(server, mode, &mut tree);
    let position = apply_style(server, oo, &mut tree, b"copy-mode-position-style");
    let matched = apply_style(server, oo, &mut tree, b"copy-mode-match-style");
    let current_match = apply_style(server, oo, &mut tree, b"copy-mode-current-match-style");
    let mark = apply_style(server, oo, &mut tree, b"copy-mode-mark-style");
    let current_line = apply_style(server, oo, &mut tree, b"copy-mode-current-line-style");
    let (line_number, current_line_number) = if width != 0 {
        (
            apply_style(server, oo, &mut tree, b"copy-mode-line-number-style"),
            apply_style(
                server,
                oo,
                &mut tree,
                b"copy-mode-current-line-number-style",
            ),
        )
    } else {
        (DEFAULT_CELL, DEFAULT_CELL)
    };
    let data = state::data(server, mode).expect("copy data exists");
    let position_visible = !data.hide_position
        && state::screen(server, mode).map_or_else(
            || {
                server
                    .panes
                    .get(mode.owner)
                    .expect("copy pane exists")
                    .base
                    .grid
                    .sy()
                    > 1
            },
            |screen| screen.rupper < screen.rlower,
        );
    let value = server.options.get_string(oo, b"copy-mode-position-format");
    let position_format = if !position_visible || value.is_empty() {
        ByteString::default()
    } else {
        let value = value.to_vec();
        tree.expand(server, &value)
    };
    tree.release(server);
    CopyRenderStyle {
        position,
        matched,
        current_match,
        mark,
        current_line,
        line_number,
        current_line_number,
        gutter_width: width,
        line_number_mode: line_number_mode(server, mode),
        modekeys: if server.options.get_number(oo, b"mode-keys") == 0 {
            ModeKeys::Emacs
        } else {
            ModeKeys::Vi
        },
        position_format,
    }
}

pub fn selection_style(server: &mut Server, mode: ModeId) -> GridCell {
    let oo = options(server, mode);
    let mut tree = styles::create_defaults(server, None, None, None, None, Some(mode.owner));
    formats(server, mode, &mut tree);
    let style = apply_style(server, oo, &mut tree, b"mode-style");
    tree.release(server);
    style
}

fn replace_style(cell: &mut GridCell, style: &GridCell, inverse: bool) {
    cell.attr = style.attr;
    (cell.fg, cell.bg) = if inverse {
        (style.bg, style.fg)
    } else {
        (style.fg, style.bg)
    };
}

fn update_style(
    data: &CopyModeData,
    fx: u32,
    fy: u32,
    cell: &mut GridCell,
    style: &CopyRenderStyle,
) {
    let cy = data.backing_y();
    if fy == cy {
        if style.current_line.fg != Colour::DEFAULT {
            cell.fg = style.current_line.fg
        }
        if style.current_line.bg != Colour::DEFAULT {
            cell.bg = style.current_line.bg
        }
        cell.attr.insert(style.current_line.attr);
    }
    let inverse = data.showmark && fy == data.my && fx == data.mx;
    if data.showmark && fy == data.my {
        replace_style(cell, &style.mark, inverse)
    }
    let Some(marks) = data.search.marks.as_ref() else {
        return;
    };
    let Some(current) = super::search::mark_at(data, fx, fy) else {
        return;
    };
    let mark = marks[current];
    if mark == 0 {
        return;
    }
    if let Some(mut cursor) = super::search::mark_at(data, data.cx, cy) {
        let found = if cursor != 0
            && style.modekeys == ModeKeys::Emacs
            && data.search.searchdirection == SearchDirection::Down
        {
            if marks[cursor - 1] == mark {
                cursor -= 1;
                true
            } else {
                false
            }
        } else {
            marks[cursor] == mark
        };
        if found
            && let Some((start, end)) = super::search::match_start_end(data, cursor)
            && current >= start
            && current <= end
        {
            replace_style(cell, &style.current_match, inverse);
            return;
        }
    }
    replace_style(cell, &style.matched, inverse);
}

fn putc(ctx: &mut ScreenWriteCtx<'_>, cell: &GridCell, ch: u8) {
    let mut cell = *cell;
    cell.data = Utf8Data::set(ch);
    ctx.cell(&cell);
}

fn write_one(
    data: &CopyModeData,
    ctx: &mut ScreenWriteCtx<'_>,
    px: u32,
    py: u32,
    fy: u32,
    nx: u32,
    style: &CopyRenderStyle,
) {
    ctx.cursormove(px as i32, py as i32, false);
    for fx in 0..nx {
        let mut cell = data.backing.screen().grid.get_cell(fx, fy);
        if u32::from(cell.data.width) <= nx - fx {
            update_style(data, fx, fy, &mut cell, style);
            if cell.flags.contains(GridCellFlags::PADDING) {
                if ctx.screen.cy == py && ctx.screen.cx <= px + fx {
                    cell.flags.remove(GridCellFlags::PADDING);
                    ctx.cursormove((px + fx) as i32, py as i32, false);
                    putc(ctx, &cell, b' ');
                }
            } else if cell.flags.contains(GridCellFlags::TAB) {
                let width = cell.data.width;
                cell.flags.remove(GridCellFlags::TAB);
                for _ in 0..width {
                    putc(ctx, &cell, b' ')
                }
            } else {
                ctx.cell(&cell)
            }
        } else {
            putc(ctx, &DEFAULT_CELL, b' ')
        }
    }
}

fn row_number(data: &CopyModeData, mode: i32, py: u32) -> u32 {
    match mode {
        1 => py.abs_diff(data.oy),
        2 => data.backing.screen().grid.hsize() - data.oy + py + 1,
        4 if py == data.cy => data.backing.screen().grid.hsize() - data.oy + py + 1,
        _ => py.abs_diff(data.cy),
    }
}

pub fn write_line(
    data: &CopyModeData,
    ctx: &mut ScreenWriteCtx<'_>,
    py: u32,
    style: &CopyRenderStyle,
) {
    let sx = ctx.screen.grid.sx();
    let width = style.gutter_width;
    let content_sx = content_width(width, sx);
    ctx.cursormove(0, py as i32, false);
    if width != 0 {
        let cell = if py == data.cy {
            &style.current_line_number
        } else {
            &style.line_number
        };
        let number = row_number(data, style.line_number_mode, py);
        let mut digits = [b' '; 11];
        let mut at = 10;
        let mut n = number;
        loop {
            at -= 1;
            digits[at] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        let field = (width - 1) as usize;
        for _ in 0..field.saturating_sub(10 - at) {
            putc(ctx, cell, b' ')
        }
        ctx.nputs(width as isize, cell, &digits[at..]);
    }
    write_one(
        data,
        ctx,
        width,
        py,
        data.backing.screen().grid.hsize() - data.oy + py,
        content_sx,
        style,
    );
    if py == 0
        && ctx.screen.rupper < ctx.screen.rlower
        && !data.hide_position
        && !style.position_format.is_empty()
    {
        ctx.cursormove(width as i32, 0, false);
        crate::format::draw::draw(
            ctx,
            &style.position,
            content_sx,
            &style.position_format,
            None,
            false,
        );
    }
    if py == data.cy && data.cx >= content_sx {
        ctx.cursormove(
            offset_for_gutter(width, data.cx, sx) as i32,
            py as i32,
            false,
        );
        putc(ctx, &DEFAULT_CELL, b'$');
    }
}

pub fn write_lines(
    data: &CopyModeData,
    ctx: &mut ScreenWriteCtx<'_>,
    py: u32,
    ny: u32,
    style: &CopyRenderStyle,
) {
    for row in py..py.checked_add(ny).expect("copy redraw range overflow") {
        write_line(data, ctx, row, style)
    }
}

fn with_sink<R>(
    server: &mut Server,
    mode: ModeId,
    sink: &mut dyn rmux_emu::screen::write::TtySink,
    pane_backed: bool,
    f: impl FnOnce(&mut CopyModeData, &mut ScreenWriteCtx<'_>) -> R,
) -> Option<R> {
    let entry = server
        .panes
        .get_mut(mode.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == mode)?;
    let data = entry.data.as_mut()?.downcast_mut::<CopyModeData>()?;
    let screen = entry.screen.as_mut()?;
    let mut ctx = ScreenWriteCtx::start(
        screen,
        sink,
        ScreenWritePolicy {
            pane_backed,
            ..ScreenWritePolicy::default()
        },
        &mut server.hyperlinks,
        #[cfg(feature = "sixel")]
        None,
    );
    let result = f(data, &mut ctx);
    ctx.finish();
    Some(result)
}

pub fn with_pane_write<R>(
    server: &mut Server,
    mode: ModeId,
    screen_only: bool,
    f: impl FnOnce(&mut CopyModeData, &mut ScreenWriteCtx<'_>) -> R,
) -> Option<R> {
    if screen_only {
        return with_sink(server, mode, &mut ScreenOnlySink, false, f);
    }
    let snapshot = fanout::PaneDrawSnapshot::capture(server, mode.owner)?;
    let mut sink = fanout::PaneSink::new(
        snapshot,
        std::mem::take(&mut server.clients),
        std::mem::take(&mut server.tparm),
    );
    let result = with_sink(server, mode, &mut sink, true, f);
    let (clients, tparm, effects) = sink.into_parts();
    server.clients = clients;
    server.tparm = tparm;
    fanout::apply_effects(server, mode.owner, effects);
    result
}

pub fn draw_initial(server: &mut Server, mode: ModeId, screen: &mut Screen) {
    let style = render_style(server, mode);
    let entry = server
        .panes
        .get(mode.owner)
        .expect("initial copy pane exists")
        .modes
        .iter()
        .find(|m| m.id == mode)
        .expect("initial copy mode exists");
    let data = entry
        .data
        .as_ref()
        .and_then(|d| d.downcast_ref::<CopyModeData>())
        .expect("initial copy data exists");
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut server.hyperlinks,
        #[cfg(feature = "sixel")]
        None,
    );
    let height = ctx.screen.grid.sy();
    write_lines(data, &mut ctx, 0, height, &style);
    let x = offset_for_gutter(style.gutter_width, data.cx, ctx.screen.grid.sx());
    ctx.cursormove(x as i32, data.cy as i32, false);
    ctx.finish();
}

pub fn redraw_lines(server: &mut Server, mode: ModeId, py: u32, ny: u32) {
    let Some(screen) = state::screen(server, mode) else {
        return;
    };
    let ny = ny.min(screen.grid.sy().saturating_sub(py));
    let style = render_style(server, mode);
    let visible = pane_scrollbar_visible(server, mode.owner);
    let overlay = visible && pane_scrollbar_overlay(server, mode.owner);
    with_pane_write(
        server,
        mode,
        style.gutter_width != 0 || overlay,
        |data, ctx| {
            write_lines(data, ctx, py, ny, &style);
            let x = offset_for_gutter(style.gutter_width, data.cx, ctx.screen.grid.sx());
            ctx.cursormove(x as i32, data.cy as i32, false);
        },
    );
    if let Some(pane) = server.panes.get_mut(mode.owner) {
        if style.gutter_width != 0 {
            pane.flags
                .insert(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR)
        } else if overlay {
            pane.flags.insert(PaneFlags::REDRAW)
        } else if visible {
            pane.flags.insert(PaneFlags::REDRAWSCROLLBAR)
        }
    }
}

pub fn redraw_screen(server: &mut Server, mode: ModeId) {
    if let Some(height) = state::screen(server, mode).map(|s| s.grid.sy()) {
        redraw_lines(server, mode, 0, height)
    }
}

pub fn redraw_selection(server: &mut Server, mode: ModeId, old_y: u32) {
    let Some(data) = state::data(server, mode) else {
        return;
    };
    let start = old_y.min(data.cy);
    let mut end = old_y.max(data.cy);
    if data.selection.selflag == SelectionMode::Word
        && end < data.backing.screen().grid.sy() + data.oy - 1
    {
        end += 1
    }
    redraw_lines(server, mode, start, end - start + 1);
}

pub fn style_changed(server: &mut Server, mode: ModeId) {
    if state::screen(server, mode).is_some_and(|s| s.selection.is_some()) {
        super::select::set_selection(server, mode, false, true);
    }
    redraw_screen(server, mode);
}

fn top_mode(server: &Server, pane: PaneId) -> Option<ModeId> {
    let mode = server.panes.get(pane)?.modes.first()?;
    state::data(server, mode.id)?;
    Some(mode.id)
}

pub fn get_word(server: &Server, pane: PaneId, x: u32, y: u32) -> Option<ByteString> {
    let data = state::data(server, top_mode(server, pane)?)?;
    let grid = &data.backing.screen().grid;
    crate::format::grid::word(
        grid,
        x,
        grid.hsize().checked_sub(data.oy)?.checked_add(y)?,
        server
            .options
            .get_string(server.options.global_s, b"word-separators"),
    )
}

pub fn get_line(server: &Server, pane: PaneId, y: u32) -> Option<ByteString> {
    let data = state::data(server, top_mode(server, pane)?)?;
    let grid = &data.backing.screen().grid;
    crate::format::grid::line(grid, grid.hsize().checked_sub(data.oy)?.checked_add(y)?)
}

pub fn get_hyperlink(server: &Server, pane: PaneId, x: u32, y: u32) -> Option<ByteString> {
    let screen = state::screen(server, top_mode(server, pane)?)?;
    crate::format::grid::hyperlink(
        &screen.grid,
        x,
        screen.grid.hsize().checked_add(y)?,
        server.panes.get(pane)?.displayed_screen(),
        &server.hyperlinks,
    )
}

pub fn current_offset(server: &Server, pane: PaneId) -> Option<(u32, u32)> {
    let data = state::data(server, top_mode(server, pane)?)?;
    let history = data.backing.screen().grid.hsize();
    Some((history.checked_sub(data.oy)?, history))
}

fn match_text(data: &CopyModeData) -> Option<ByteString> {
    let (sx, sy, ex, ey) = super::search::match_at_cursor(data)?;
    let grid = &data.backing.screen().grid;
    let mut bytes = Vec::new();
    for y in sy..=ey {
        let first = if y == sy { sx } else { 0 };
        let last = if y == ey { ex } else { grid.sx() - 1 };
        for x in first..=last {
            let cell = grid.get_cell(x, y);
            if cell.flags.contains(GridCellFlags::TAB) {
                bytes.push(b'\t')
            } else if !cell.flags.contains(GridCellFlags::PADDING) {
                bytes.extend_from_slice(cell.data.bytes())
            }
        }
    }
    if bytes.is_empty() {
        None
    } else {
        Some(ByteString(bytes))
    }
}

const SCALAR_FORMATS: &[&[u8]] = &[
    b"top_line_time",
    b"scroll_position",
    b"copy_position",
    b"copy_position_limit",
    b"copy_line_numbers",
    b"refresh_active",
    b"rectangle_toggle",
    b"copy_cursor_x",
    b"copy_cursor_y",
    b"selection_start_x",
    b"selection_start_y",
    b"selection_end_x",
    b"selection_end_y",
    b"selection_active",
    b"selection_present",
    b"selection_mode",
    b"search_present",
    b"search_timed_out",
    b"search_count",
    b"search_count_partial",
];
const LAZY_FORMATS: &[&[u8]] = &[
    b"search_match",
    b"copy_cursor_word",
    b"copy_cursor_line",
    b"copy_cursor_hyperlink",
];

pub fn format_value(server: &Server, mode: ModeId, key: &[u8]) -> Option<FormatValue> {
    let data = state::data(server, mode)?;
    let grid = &data.backing.screen().grid;
    let sel = &data.selection;
    let unsigned = |v: u32| FormatValue::Unsigned(u64::from(v));
    let boolean = |v: bool| FormatValue::Signed(i64::from(v));
    Some(match key {
        b"top_line_time" => FormatValue::Unsigned(
            grid.peek_line(grid.hsize().checked_sub(data.oy)?)?
                .time
                .to_wall(server.start_time.0) as u64,
        ),
        b"scroll_position" => FormatValue::Signed(i64::from(data.oy as i32)),
        b"copy_position" => unsigned(if line_number_is_absolute(server, mode) {
            grid.hsize() - data.oy + 1
        } else {
            data.oy
        }),
        b"copy_position_limit" => unsigned(if line_number_is_absolute(server, mode) {
            grid.hsize() + grid.sy()
        } else {
            grid.hsize()
        }),
        b"copy_line_numbers" => boolean(line_numbers_active(server, mode)),
        b"refresh_active" => boolean(data.refresh_active),
        b"rectangle_toggle" => boolean(sel.rectflag),
        b"copy_cursor_x" => FormatValue::Signed(i64::from(data.cx as i32)),
        b"copy_cursor_y" => FormatValue::Signed(i64::from(data.cy as i32)),
        b"selection_start_x" if sel.active => FormatValue::Signed(i64::from(sel.selx as i32)),
        b"selection_start_y" if sel.active => FormatValue::Signed(i64::from(sel.sely as i32)),
        b"selection_end_x" if sel.active => FormatValue::Signed(i64::from(sel.endselx as i32)),
        b"selection_end_y" if sel.active => FormatValue::Signed(i64::from(sel.endsely as i32)),
        b"selection_active" => boolean(sel.active && sel.cursordrag != CursorDrag::None),
        b"selection_present" => {
            boolean(sel.active && (sel.selx != sel.endselx || sel.sely != sel.endsely))
        }
        b"selection_mode" => FormatValue::Bytes(ByteString::from(match sel.selflag {
            SelectionMode::Char => b"char".as_slice(),
            SelectionMode::Word => b"word",
            SelectionMode::Line => b"line",
        })),
        b"search_present" => boolean(data.search.marks.is_some()),
        b"search_timed_out" => boolean(data.timeout),
        b"search_count" if data.search.count != -1 => {
            FormatValue::Signed(i64::from(data.search.count))
        }
        b"search_count_partial" if data.search.count != -1 => boolean(data.search.more),
        b"search_match" => FormatValue::Bytes(match_text(data)?),
        b"copy_cursor_word" => FormatValue::Bytes(get_word(server, mode.owner, data.cx, data.cy)?),
        b"copy_cursor_line" => FormatValue::Bytes(get_line(server, mode.owner, data.cy)?),
        b"copy_cursor_hyperlink" => {
            let screen = state::screen(server, mode)?;
            FormatValue::Bytes(crate::format::grid::hyperlink(
                &screen.grid,
                data.cx,
                screen.grid.hsize() + data.cy,
                screen,
                &server.hyperlinks,
            )?)
        }
        _ => return None,
    })
}

pub fn formats(server: &Server, mode: ModeId, tree: &mut FormatTree) {
    for &key in SCALAR_FORMATS {
        if let Some(value) = format_value(server, mode, key) {
            tree.add(key, value.bytes())
        }
    }
    for &key in LAZY_FORMATS {
        tree.add_callback(
            key,
            Box::new(move |context, runtime| runtime.builtin(context, key)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::CopyBacking;
    use super::*;
    use crate::ids::ArenaId;
    use rmux_emu::cell::GridAttributes;
    use rmux_emu::hyperlinks::HyperlinkRegistry;
    use rmux_emu::screen::{ScreenMode, ScreenResetPolicy};

    fn fixture(width: u32, height: u32) -> (Server, ModeId) {
        let mut server = Server::default();
        let window = crate::model::window::window_create(&mut server, width, height, 0, 0).unwrap();
        let pane = crate::model::pane::pane_create(&mut server, window, width, height, 20).unwrap();
        let driver = std::rc::Rc::new(super::super::CopyModeDriver {
            kind: super::super::CopyModeKind::Copy {
                source: None,
                args: crate::cmd::arguments::Args::default(),
            },
        });
        let mode = crate::model::pane::pane_set_mode(
            &mut server,
            pane,
            b"copy-mode",
            crate::modes::WindowModeFlags::default(),
            driver,
            false,
        )
        .unwrap()
        .unwrap();
        (server, mode)
    }

    fn set_string(server: &mut Server, mode: ModeId, name: &[u8], value: &[u8]) {
        let oo = options(server, mode);
        let mut store = std::mem::take(&mut server.options);
        store.set_string(oo, name, false, value, server);
        server.options = store;
    }

    fn pure_data(width: u32, height: u32, registry: &mut HyperlinkRegistry) -> CopyModeData {
        let screen =
            Screen::new(width, height, 20, ScreenResetPolicy::default(), registry).unwrap();
        CopyModeData::new(
            CopyBacking::Snapshot(screen),
            PaneId::from_parts(0, 0),
            ModeKeys::Emacs,
        )
    }

    fn plain_style() -> CopyRenderStyle {
        CopyRenderStyle {
            position: DEFAULT_CELL,
            matched: DEFAULT_CELL,
            current_match: DEFAULT_CELL,
            mark: DEFAULT_CELL,
            current_line: DEFAULT_CELL,
            line_number: DEFAULT_CELL,
            current_line_number: DEFAULT_CELL,
            gutter_width: 0,
            line_number_mode: 0,
            modekeys: ModeKeys::Emacs,
            position_format: ByteString::default(),
        }
    }

    fn row(screen: &Screen, y: u32) -> Vec<u8> {
        (0..screen.grid.sx())
            .flat_map(|x| screen.grid.view_get_cell(x, y).data.bytes().to_vec())
            .collect()
    }

    #[test]
    fn gutter_width_digits_and_option_override() {
        for (history, height, expected) in [
            (0, 1, 4),
            (95, 4, 4),
            (994, 4, 4),
            (995, 4, 5),
            (99995, 4, 7),
        ] {
            assert_eq!(gutter_width(history, height), expected);
        }
        for option in 0..=4 {
            assert_eq!(effective_line_number_mode(CopyLineNumbers::Off, option), 0);
            assert_eq!(
                effective_line_number_mode(CopyLineNumbers::Option, option),
                option
            );
            assert_eq!(
                effective_line_number_mode(CopyLineNumbers::Default, option),
                option.max(1)
            );
        }
    }

    #[test]
    fn gutter_offsets_and_inverse_clip_like_c() {
        for sx in 1..32 {
            for width in 0..40 {
                for x in 0..40 {
                    let vx = offset_for_gutter(width, x, sx);
                    let expected = if width == 0 {
                        x
                    } else if x >= content_width(width, sx) {
                        sx - 1
                    } else {
                        width + x
                    };
                    assert_eq!(vx, expected);
                    assert_eq!(
                        unoffset_for_gutter(width, vx, sx),
                        if width == 0 {
                            x
                        } else {
                            x.min(content_width(width, sx) - 1)
                        }
                    );
                    assert_eq!(
                        unoffset_for_gutter(width, x, sx),
                        if width == 0 {
                            x
                        } else {
                            x.saturating_sub(width).min(content_width(width, sx) - 1)
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn all_gutter_modes_have_pinned_row_numbers() {
        let mut registry = HyperlinkRegistry::new();
        let mut data = pure_data(12, 4, &mut registry);
        data.backing.screen_mut().grid.adjust_lines(24);
        data.backing.screen_mut().grid.set_hsize_unchecked(20);
        data.oy = 10;
        data.cy = 2;
        for (mode, expected) in [
            (1, [10, 9, 8, 7]),
            (2, [11, 12, 13, 14]),
            (3, [2, 1, 0, 1]),
            (4, [2, 1, 13, 1]),
        ] {
            assert_eq!(
                (0..4)
                    .map(|py| row_number(&data, mode, py))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn style_precedence_and_mark_inverse_do_not_change_other_cell_fields() {
        let mut registry = HyperlinkRegistry::new();
        let mut data = pure_data(5, 2, &mut registry);
        data.cx = 2;
        data.showmark = true;
        data.mx = 2;
        data.my = 0;
        data.search.marks = Some(vec![0, 1, 1, 0, 1, 0, 0, 0, 0, 0]);
        let mut style = plain_style();
        style.current_line.fg = Colour(1);
        style.current_line.attr = GridAttributes::UNDERSCORE;
        style.mark = GridCell {
            fg: Colour(2),
            bg: Colour(3),
            attr: GridAttributes::BRIGHT,
            ..DEFAULT_CELL
        };
        style.matched = GridCell {
            fg: Colour(4),
            bg: Colour(5),
            attr: GridAttributes::ITALICS,
            ..DEFAULT_CELL
        };
        style.current_match = GridCell {
            fg: Colour(6),
            bg: Colour(7),
            attr: GridAttributes::REVERSE,
            ..DEFAULT_CELL
        };
        let original = GridCell {
            data: Utf8Data::set(b'x'),
            fg: Colour(0),
            bg: Colour(9),
            us: Colour(11),
            flags: GridCellFlags::NOPALETTE,
            ..DEFAULT_CELL
        };
        for (x, fg, bg, attr) in [
            (0, 2, 3, GridAttributes::BRIGHT),
            (1, 6, 7, GridAttributes::REVERSE),
            (2, 7, 6, GridAttributes::REVERSE),
            (4, 4, 5, GridAttributes::ITALICS),
        ] {
            let mut cell = original;
            update_style(&data, x, 0, &mut cell, &style);
            assert_eq!(
                (cell.fg, cell.bg, cell.attr),
                (Colour(fg), Colour(bg), attr)
            );
            assert_eq!(
                (cell.data, cell.us, cell.flags, cell.link),
                (original.data, original.us, original.flags, original.link)
            );
        }
        data.showmark = false;
        data.search.marks = None;
        let mut cell = original;
        update_style(&data, 0, 0, &mut cell, &style);
        assert_eq!(
            (cell.fg, cell.bg, cell.attr),
            (Colour(1), original.bg, GridAttributes::UNDERSCORE)
        );
    }

    #[test]
    fn emacs_direction_prefers_previous_generation_not_current_cell() {
        let mut registry = HyperlinkRegistry::new();
        let mut data = pure_data(5, 2, &mut registry);
        data.cx = 2;
        data.search.marks = Some(vec![1, 1, 2, 2, 0, 0, 0, 0, 0, 0]);
        let mut style = plain_style();
        style.matched.fg = Colour(1);
        style.current_match.fg = Colour(2);
        for (direction, keys, expected) in [
            (SearchDirection::Down, ModeKeys::Emacs, [2, 2, 1, 1]),
            (SearchDirection::Down, ModeKeys::Vi, [1, 1, 2, 2]),
            (SearchDirection::Up, ModeKeys::Emacs, [1, 1, 2, 2]),
            (SearchDirection::Off, ModeKeys::Emacs, [1, 1, 2, 2]),
        ] {
            data.search.searchdirection = direction;
            style.modekeys = keys;
            for x in 0..4 {
                let mut cell = DEFAULT_CELL;
                update_style(&data, x, 0, &mut cell, &style);
                assert_eq!(cell.fg, Colour(expected[x as usize]));
            }
        }
    }

    #[test]
    fn tabs_padding_and_truncated_wide_spans_are_repaired() {
        let mut registry = HyperlinkRegistry::new();
        let mut data = pure_data(8, 2, &mut registry);
        let mut visible =
            Screen::new(6, 2, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let grid = &mut data.backing.screen_mut().grid;
        let mut tab = DEFAULT_CELL;
        tab.data = Utf8Data::set(b'\t');
        tab.data.width = 3;
        tab.flags = GridCellFlags::TAB;
        grid.set_cell(0, 0, &tab);
        for x in 1..3 {
            grid.set_cell(
                x,
                0,
                &GridCell {
                    flags: GridCellFlags::PADDING,
                    ..DEFAULT_CELL
                },
            );
        }
        grid.set_cell(
            3,
            0,
            &GridCell {
                data: Utf8Data::set(b'A'),
                ..DEFAULT_CELL
            },
        );
        grid.set_cell(
            4,
            0,
            &GridCell {
                flags: GridCellFlags::PADDING,
                fg: Colour(1),
                ..DEFAULT_CELL
            },
        );
        let mut wide = DEFAULT_CELL;
        wide.data = rmux_util::utf8::from_cstr("界".as_bytes())[0];
        grid.set_cell(5, 0, &wide);
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut visible,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        write_line(&data, &mut ctx, 0, &plain_style());
        ctx.finish();
        assert_eq!(row(&visible, 0), b"   A  ");
        assert_eq!(visible.grid.view_get_cell(4, 0).fg, Colour(1));
        assert!(!(0..6).any(|x| {
            visible
                .grid
                .view_get_cell(x, 0)
                .flags
                .intersects(GridCellFlags::TAB | GridCellFlags::PADDING)
        }));
        assert_eq!(visible.grid.view_get_cell(5, 0).fg, Colour::DEFAULT);
    }

    #[test]
    fn position_format_shrinks_and_end_cursor_dollar_wins() {
        let mut registry = HyperlinkRegistry::new();
        let mut data = pure_data(12, 3, &mut registry);
        data.cx = 8;
        let mut visible =
            Screen::new(12, 3, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let mut style = plain_style();
        style.gutter_width = 4;
        style.line_number_mode = 2;
        style.position_format = b"#[align=right]123456789ABC".as_slice().into();
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut visible,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        write_line(&data, &mut ctx, 0, &style);
        ctx.finish();
        assert_eq!(&row(&visible, 0)[..4], b"  1 ");
        assert_eq!(&row(&visible, 0)[4..], b"56789AB$");
        assert_eq!(visible.grid.view_get_cell(11, 0).data.bytes(), b"$");
        assert_eq!(visible.grid.view_get_cell(11, 0).fg, Colour::DEFAULT);
        data.hide_position = true;
        let mut ctx = ScreenWriteCtx::start(
            &mut visible,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        write_line(&data, &mut ctx, 0, &style);
        ctx.finish();
        assert_eq!(&row(&visible, 0)[4..11], b"       ");
    }

    #[test]
    fn format_snapshot_options_and_stopped_selection_style_refresh() {
        let (mut server, mode) = fixture(12, 4);
        set_string(
            &mut server,
            mode,
            b"copy-mode-current-line-style",
            b"fg=#{?selection_present,red,blue},bold",
        );
        set_string(&mut server, mode, b"mode-style", b"fg=yellow,bg=red");
        let data = state::data_mut(&mut server, mode).unwrap();
        data.selection.active = true;
        data.selection.selx = 1;
        data.selection.endselx = 5;
        data.selection.cursordrag = CursorDrag::None;
        super::super::select::set_selection(&mut server, mode, false, true);
        let style = render_style(&mut server, mode);
        assert_eq!(style.current_line.fg, Colour(1));
        for cell in [
            style.position,
            style.matched,
            style.current_match,
            style.mark,
            style.current_line,
        ] {
            assert!(cell.flags.contains(GridCellFlags::NOPALETTE));
        }
        let before = {
            let data = state::data(&server, mode).unwrap();
            (
                data.selection.selx,
                data.selection.sely,
                data.selection.endselx,
                data.selection.endsely,
            )
        };
        style_changed(&mut server, mode);
        let data = state::data(&server, mode).unwrap();
        assert_eq!(
            (
                data.selection.selx,
                data.selection.sely,
                data.selection.endselx,
                data.selection.endsely
            ),
            before
        );
        assert_eq!(
            state::screen(&server, mode)
                .unwrap()
                .selection
                .as_ref()
                .unwrap()
                .cell
                .fg,
            Colour(3)
        );
        assert_eq!(
            format_value(&server, mode, b"selection_active")
                .unwrap()
                .bytes(),
            b"0"
        );
        assert_eq!(
            format_value(&server, mode, b"selection_present")
                .unwrap()
                .bytes(),
            b"1"
        );
        assert_eq!(
            format_value(&server, mode, b"search_count")
                .unwrap()
                .bytes(),
            b"0"
        );
        super::super::search::clear_marks(&mut server, mode);
        assert!(format_value(&server, mode, b"search_count").is_none());
        state::data_mut(&mut server, mode).unwrap().search.count = 7;
        assert_eq!(
            format_value(&server, mode, b"search_count")
                .unwrap()
                .bytes(),
            b"7"
        );
    }

    #[test]
    fn line_number_external_setter_does_not_force_option_or_touch_view() {
        let (mut server, mode) = fixture(12, 4);
        set_line_numbers(&mut server, mode.owner, true);
        assert!(!line_numbers_active(&server, mode));
        set_line_numbers1(&mut server, mode, true, true);
        assert_eq!(line_number_mode(&server, mode), 1);
        assert_eq!(line_number_width(&server, mode), 4);
        for option in 1..=4 {
            let oo = options(&server, mode);
            server
                .options
                .set_number_value(oo, b"copy-mode-line-numbers", option);
            assert_eq!(line_number_is_absolute(&server, mode), option >= 2);
        }
        server.panes.get_mut(mode.owner).unwrap().modes[0].name = b"view-mode".to_vec();
        set_line_numbers(&mut server, mode.owner, false);
        assert!(line_numbers_active(&server, mode));
    }

    #[test]
    fn queries_use_backing_for_text_visible_for_links_and_optional_counts() {
        let (mut server, mode) = fixture(12, 4);
        state::data_mut(&mut server, mode)
            .unwrap()
            .backing
            .screen_mut()
            .grid
            .set_cells(0, 0, &DEFAULT_CELL, b"hello world");
        assert_eq!(get_word(&server, mode.owner, 1, 0).unwrap(), b"hello");
        assert_eq!(get_line(&server, mode.owner, 0).unwrap(), b"hello world");
        assert_eq!(current_offset(&server, mode.owner), Some((0, 0)));
        let pane = server.panes.get_mut(mode.owner).unwrap();
        let visible = pane.modes[0].screen.as_mut().unwrap();
        let link = server
            .hyperlinks
            .put(
                visible.hyperlinks.as_ref().unwrap(),
                b"https://example.test",
                Some(b"id"),
            )
            .unwrap();
        visible.grid.set_cell(
            1,
            0,
            &GridCell {
                link,
                data: Utf8Data::set(b'L'),
                ..DEFAULT_CELL
            },
        );
        state::data_mut(&mut server, mode).unwrap().cx = 1;
        assert_eq!(
            get_hyperlink(&server, mode.owner, 1, 0).unwrap(),
            b"https://example.test"
        );
        assert_eq!(
            format_value(&server, mode, b"copy_cursor_hyperlink")
                .unwrap()
                .bytes(),
            b"https://example.test"
        );
        assert!(get_word(&server, PaneId::from_parts(u32::MAX, 0), 0, 0).is_none());
    }

    #[test]
    fn visible_mode_drawing_does_not_inherit_base_sync_or_lease_server_fields() {
        let (mut server, mode) = fixture(12, 4);
        server
            .panes
            .get_mut(mode.owner)
            .unwrap()
            .base
            .mode
            .insert(ScreenMode::SYNC);
        state::data_mut(&mut server, mode)
            .unwrap()
            .backing
            .screen_mut()
            .grid
            .set_cells(0, 1, &DEFAULT_CELL, b"visible");
        let clients = server.clients.len();
        redraw_lines(&mut server, mode, 1, 1);
        assert_eq!(server.clients.len(), clients);
        assert_eq!(
            &row(state::screen(&server, mode).unwrap(), 1)[..7],
            b"visible"
        );
        assert!(
            !state::screen(&server, mode)
                .unwrap()
                .mode
                .contains(ScreenMode::SYNC)
        );
        assert!(
            server
                .panes
                .get(mode.owner)
                .unwrap()
                .base
                .mode
                .contains(ScreenMode::SYNC)
        );
    }

    #[test]
    fn word_selection_redraw_includes_following_row_and_gutter_sets_damage_flags() {
        let (mut server, mode) = fixture(12, 4);
        let data = state::data_mut(&mut server, mode).unwrap();
        data.cy = 1;
        data.selection.selflag = SelectionMode::Word;
        data.backing
            .screen_mut()
            .grid
            .set_cells(0, 2, &DEFAULT_CELL, b"next row");
        redraw_selection(&mut server, mode, 0);
        assert_eq!(
            &row(state::screen(&server, mode).unwrap(), 2)[..8],
            b"next row"
        );
        set_line_numbers1(&mut server, mode, true, true);
        server
            .panes
            .get_mut(mode.owner)
            .unwrap()
            .flags
            .remove(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR);
        redraw_lines(&mut server, mode, 1, 1);
        assert!(
            server
                .panes
                .get(mode.owner)
                .unwrap()
                .flags
                .contains(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR)
        );
        assert_eq!(state::screen(&server, mode).unwrap().cx, 4);
    }

    #[test]
    fn initial_draw_works_before_screen_publication_and_preserves_wide_padding() {
        let (mut server, mode) = fixture(12, 4);
        let mut visible = server.panes.get_mut(mode.owner).unwrap().modes[0]
            .screen
            .take()
            .unwrap();
        let data = state::data_mut(&mut server, mode).unwrap();
        data.hide_position = true;
        let wide = GridCell {
            data: rmux_util::utf8::from_cstr("界".as_bytes())[0],
            ..DEFAULT_CELL
        };
        data.backing.screen_mut().grid.set_cell(0, 1, &wide);
        data.backing.screen_mut().grid.set_cell(
            1,
            1,
            &GridCell {
                flags: GridCellFlags::PADDING,
                ..DEFAULT_CELL
            },
        );
        draw_initial(&mut server, mode, &mut visible);
        assert_eq!(
            visible.grid.view_get_cell(0, 1).data.bytes(),
            "界".as_bytes()
        );
        assert!(
            visible
                .grid
                .view_get_cell(1, 1)
                .flags
                .contains(GridCellFlags::PADDING)
        );
        assert!(state::screen(&server, mode).is_none());
        server.panes.get_mut(mode.owner).unwrap().modes[0].screen = Some(visible);
    }

    #[test]
    fn backing_times_offsets_match_text_and_format_selection_fields() {
        let (mut server, mode) = fixture(12, 4);
        server.start_time = (1000, 0);
        let data = state::data_mut(&mut server, mode).unwrap();
        let grid = &mut data.backing.screen_mut().grid;
        grid.adjust_lines(7);
        grid.set_hsize_unchecked(3);
        grid.get_line_mut(2).time = rmux_emu::grid::LineTime(5);
        grid.set_cells(0, 2, &DEFAULT_CELL, b"match text");
        data.oy = 1;
        data.cx = 5;
        data.search.marks = Some(vec![0; 48]);
        data.search.marks.as_mut().unwrap()[0..5].fill(1);
        data.search.count = 1;
        data.search.more = true;
        assert_eq!(current_offset(&server, mode.owner), Some((2, 3)));
        assert_eq!(
            format_value(&server, mode, b"top_line_time")
                .unwrap()
                .bytes(),
            b"1004"
        );
        assert_eq!(
            format_value(&server, mode, b"search_match")
                .unwrap()
                .bytes(),
            b"match"
        );
        assert_eq!(
            format_value(&server, mode, b"search_count_partial")
                .unwrap()
                .bytes(),
            b"1"
        );
        assert!(format_value(&server, mode, b"selection_start_x").is_none());
        let oo = options(&server, mode);
        server
            .options
            .set_number_value(oo, b"copy-mode-line-numbers", 3);
        assert_eq!(
            format_value(&server, mode, b"copy_position")
                .unwrap()
                .bytes(),
            b"3"
        );
        assert_eq!(
            format_value(&server, mode, b"copy_position_limit")
                .unwrap()
                .bytes(),
            b"7"
        );
        server
            .options
            .set_number_value(oo, b"copy-mode-line-numbers", 1);
        assert_eq!(
            format_value(&server, mode, b"copy_position")
                .unwrap()
                .bytes(),
            b"1"
        );
        assert_eq!(
            format_value(&server, mode, b"copy_position_limit")
                .unwrap()
                .bytes(),
            b"3"
        );
    }

    #[test]
    fn copy_format_callbacks_are_lazy_and_cached_without_screen_borrows() {
        struct Runtime {
            calls: u32,
        }
        impl crate::format::runtime::FormatRuntime for Runtime {
            fn builtin(
                &mut self,
                _: &crate::format::FormatContext,
                key: &[u8],
            ) -> Option<FormatValue> {
                self.calls += 1;
                Some(FormatValue::Bytes(key.into()))
            }
        }
        let (mut server, mode) = fixture(12, 4);
        let mut tree =
            styles::create_defaults(&mut server, None, None, None, None, Some(mode.owner));
        formats(&server, mode, &mut tree);
        let mut runtime = Runtime { calls: 0 };
        assert_eq!(runtime.calls, 0);
        assert_eq!(
            tree.expand(
                &mut runtime,
                b"#{copy_cursor_word}:#{copy_cursor_word}:#{search_match}"
            ),
            b"copy_cursor_word:copy_cursor_word:search_match"
        );
        assert_eq!(runtime.calls, 2);
        tree.release(&mut server);
    }

    #[test]
    fn distinct_mode_screen_emits_synchronous_drawing_even_when_base_is_sync() {
        use rmux_emu::screen::write::{DrawOp, DrawSnapshot, ScreenRenderEffects, TtySink};
        #[derive(Default)]
        struct Sink {
            draws: u32,
            begins: u32,
        }
        impl TtySink for Sink {
            fn draw(&mut self, _: DrawOp<'_>, _: &DrawSnapshot) {
                self.draws += 1
            }
            fn visible_columns(
                &mut self,
                x: u32,
                _: u32,
                n: u32,
                out: &mut Vec<std::ops::Range<u32>>,
            ) {
                out.clear();
                if n != 0 {
                    out.push(x..x + n)
                }
            }
            fn obscured(&mut self) -> bool {
                false
            }
            fn redraw_pending(&self) -> bool {
                false
            }
            fn effect(&mut self, _: ScreenRenderEffects, _: &Screen) {}
            fn begin_write(&mut self) {
                self.begins += 1
            }
        }
        let (mut server, mode) = fixture(12, 4);
        server
            .panes
            .get_mut(mode.owner)
            .unwrap()
            .base
            .mode
            .insert(ScreenMode::SYNC);
        state::data_mut(&mut server, mode)
            .unwrap()
            .backing
            .screen_mut()
            .grid
            .set_cells(0, 2, &DEFAULT_CELL, b"new");
        let style = render_style(&mut server, mode);
        let mut sink = Sink::default();
        with_sink(&mut server, mode, &mut sink, true, |data, ctx| {
            write_line(data, ctx, 2, &style)
        })
        .unwrap();
        assert_eq!(sink.begins, 1);
        assert!(sink.draws != 0);
        assert_eq!(&row(state::screen(&server, mode).unwrap(), 2)[..3], b"new");
    }

    #[test]
    fn overlay_redraw_requires_pane_nonoverlay_requires_scrollbar() {
        let (mut server, mode) = fixture(12, 4);
        let window = server.panes.get(mode.owner).unwrap().window;
        server.windows.get_mut(window).unwrap().sb =
            crate::ui::scrollbar::PaneScrollbarPolicy::Modal;
        server.panes.get_mut(mode.owner).unwrap().scrollbar_visible = true;
        server
            .panes
            .get_mut(mode.owner)
            .unwrap()
            .flags
            .remove(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR);
        redraw_lines(&mut server, mode, 1, 1);
        assert!(
            server
                .panes
                .get(mode.owner)
                .unwrap()
                .flags
                .contains(PaneFlags::REDRAW)
        );
        assert!(
            !server
                .panes
                .get(mode.owner)
                .unwrap()
                .flags
                .contains(PaneFlags::REDRAWSCROLLBAR)
        );
        server.windows.get_mut(window).unwrap().sb =
            crate::ui::scrollbar::PaneScrollbarPolicy::Always;
        server
            .panes
            .get_mut(mode.owner)
            .unwrap()
            .flags
            .remove(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR);
        redraw_lines(&mut server, mode, 1, 1);
        assert!(
            server
                .panes
                .get(mode.owner)
                .unwrap()
                .flags
                .contains(PaneFlags::REDRAWSCROLLBAR)
        );
        assert!(
            !server
                .panes
                .get(mode.owner)
                .unwrap()
                .flags
                .contains(PaneFlags::REDRAW)
        );
    }

    #[test]
    fn matched_text_preserves_tab_skips_padding_and_joins_wrapped_rows() {
        let mut registry = HyperlinkRegistry::new();
        let mut data = pure_data(4, 2, &mut registry);
        data.cx = 3;
        data.cy = 1;
        data.search.marks = Some(vec![0, 0, 1, 1, 1, 1, 1, 0]);
        let grid = &mut data.backing.screen_mut().grid;
        grid.set_cell(
            2,
            0,
            &GridCell {
                data: Utf8Data::set(b'\t'),
                flags: GridCellFlags::TAB,
                ..DEFAULT_CELL
            },
        );
        grid.set_cell(
            3,
            0,
            &GridCell {
                flags: GridCellFlags::PADDING,
                ..DEFAULT_CELL
            },
        );
        grid.set_cells(0, 1, &DEFAULT_CELL, b"end");
        assert_eq!(match_text(&data).unwrap(), b"\tend");
    }

    #[test]
    fn position_indicator_is_suppressed_without_scroll_region_or_when_hidden() {
        let (mut server, mode) = fixture(12, 4);
        set_string(
            &mut server,
            mode,
            b"copy-mode-position-format",
            b"#[align=right]POSITION",
        );
        state::data_mut(&mut server, mode).unwrap().hide_position = true;
        assert!(render_style(&mut server, mode).position_format.is_empty());
        state::data_mut(&mut server, mode).unwrap().hide_position = false;
        state::parts_mut(&mut server, mode).unwrap().1.rlower = 0;
        assert!(render_style(&mut server, mode).position_format.is_empty());
        state::parts_mut(&mut server, mode).unwrap().1.rlower = 3;
        assert_eq!(
            render_style(&mut server, mode).position_format,
            b"#[align=right]POSITION"
        );
    }

    #[test]
    fn pinned_c_gutter_style_and_span_differential() {
        use std::fmt::Write as _;
        use std::path::PathBuf;
        use std::process::Command;
        let source_dir = std::env::var_os("RMUX_TMUX_SOURCE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/Users/j/fun/tmux"));
        if !source_dir.exists() {
            eprintln!(
                "SKIP copy render C differential: pinned source missing at {}",
                source_dir.display()
            );
            return;
        }
        let source = Command::new("git")
            .arg("-C")
            .arg(&source_dir)
            .args(["show", "8f25579c:window-copy.c"])
            .output()
            .expect("read pinned copy source");
        assert!(
            source.status.success(),
            "{}",
            String::from_utf8_lossy(&source.stderr)
        );
        let source = String::from_utf8(source.stdout).unwrap();
        let mut c = String::from(
            r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef unsigned int u_int;
typedef unsigned char u_char;
#define GRID_FLAG_PADDING 4
#define GRID_FLAG_TAB 128
#define MODEKEY_EMACS 0
#define WINDOW_COPY_LINE_NUMBERS_OFF 0
#define WINDOW_COPY_LINE_NUMBERS_DEFAULT 1
#define WINDOW_COPY_LINE_NUMBERS_ABSOLUTE 2
#define WINDOW_COPY_LINE_NUMBERS_RELATIVE 3
#define WINDOW_COPY_LINE_NUMBERS_HYBRID 4
struct utf8_data { unsigned char data[32]; unsigned char size, width; };
struct grid_cell { struct utf8_data data; u_int attr, flags; int fg,bg,us; };
static const struct grid_cell grid_default_cell = { {{' '},1,1},0,0,8,8,8 };
struct grid { u_int sx,sy,hsize; struct grid_cell cells[2][8]; };
struct screen { struct grid *grid; u_int cx,cy; struct grid_cell cells[2][8]; };
struct options { int mode,keys; const char *current_style; };
struct window { struct options *options; };
struct window_pane { struct window *window; };
struct window_copy_mode_data { struct screen *backing; u_int cx,cy,oy,mx,my; int showmark,line_numbers,searchdirection; unsigned char *searchmark; };
struct window_mode_entry { struct window_pane *wp; struct window_copy_mode_data *data; };
struct screen_write_ctx { struct screen *s; };
#define screen_hsize(s) ((s)->grid->hsize)
#define screen_size_x(s) ((s)->grid->sx)
#define screen_size_y(s) ((s)->grid->sy)
static int options_get_number(struct options *o, const char *name) { return !strcmp(name,"mode-keys") ? o->keys : o->mode; }
static const char *options_get_string(struct options *o, const char *name) { (void)name; return o->current_style; }
static void fatalx(const char *s) { (void)s; abort(); }
static void grid_get_cell(struct grid *g,u_int x,u_int y,struct grid_cell *c) { *c = x<g->sx && y<2 ? g->cells[y][x] : grid_default_cell; }
static void screen_write_cursormove(struct screen_write_ctx *c,u_int x,u_int y,int origin) { (void)origin; c->s->cx=x; c->s->cy=y; }
static void screen_write_cell(struct screen_write_ctx *c,const struct grid_cell *gc) { if(c->s->cx<8) c->s->cells[c->s->cy][c->s->cx]=*gc; c->s->cx+=gc->data.width; }
static void screen_write_putc(struct screen_write_ctx *c,const struct grid_cell *gc,char ch) { struct grid_cell cell=*gc; cell.data.data[0]=ch;cell.data.size=cell.data.width=1;screen_write_cell(c,&cell); }
"#,
        );
        for (start, end) in [
            (
                "static int\nwindow_copy_search_mark_at(",
                "static u_int\nwindow_copy_clip_width(",
            ),
            (
                "static void\nwindow_copy_match_start_end(",
                "static char *\nwindow_copy_match_at_cursor(",
            ),
            (
                "static void\nwindow_copy_update_style(",
                "void\nwindow_copy_set_line_numbers(",
            ),
        ] {
            let rest = &source[source.find(start).expect("pinned renderer start")..];
            c.push_str(&rest[..rest.find(end).expect("pinned renderer end")]);
        }
        c.push_str(r#"
int main(void) {
struct grid gd={.sx=5,.sy=2}; struct screen backing={.grid=&gd}, visible={.grid=&gd};
struct options oo={.current_style="default"}; struct window w={.options=&oo}; struct window_pane wp={.window=&w};
unsigned char marks[10]={0,1,1,0,1,0,0,0,0,0};
struct window_copy_mode_data d={.backing=&backing,.cx=2,.showmark=1,.mx=2,.searchmark=marks,.line_numbers=1};
struct window_mode_entry me={.wp=&wp,.data=&d};
struct grid_cell mgc=grid_default_cell,cgc=grid_default_cell,mkgc=grid_default_cell,clgc=grid_default_cell,gc;
mgc.fg=4;mgc.bg=5;mgc.attr=64;cgc.fg=6;cgc.bg=7;cgc.attr=16;mkgc.fg=2;mkgc.bg=3;mkgc.attr=1;clgc.fg=1;clgc.attr=4;
for(u_int x=0;x<5;x++) { gc=grid_default_cell;gc.us=11;gc.flags=32;window_copy_update_style(&me,x,0,&gc,&mgc,&cgc,&mkgc,&clgc);printf("%d %d %u %d %u\n",gc.fg,gc.bg,gc.attr,gc.us,gc.flags); }
for(int setting=0;setting<3;setting++) for(int mode=0;mode<5;mode++) {
d.line_numbers=setting;oo.mode=mode;printf("%d %d %d\n",window_copy_line_number_mode(&me),window_copy_line_number_is_absolute(&me),window_copy_line_numbers_active(&me));
}
d.line_numbers=1;oo.mode=1;
for(u_int history=0;history<1100;history+=19) { gd.hsize=history;printf("%u\n",window_copy_line_number_width(&me)); }
gd.hsize=0;
for(u_int sx=1;sx<16;sx++) for(u_int x=0;x<20;x++) printf("%u %u\n",window_copy_cursor_offset(&me,x,sx),window_copy_cursor_unoffset(&me,x,sx));
d.showmark=0;d.searchmark=NULL;gd.sx=8;
for(u_int x=0;x<8;x++) gd.cells[0][x]=grid_default_cell;
gd.cells[0][0].data=(struct utf8_data){{'\t'},1,3};gd.cells[0][0].flags=128;
gd.cells[0][1].flags=gd.cells[0][2].flags=gd.cells[0][4].flags=4;
gd.cells[0][3].data.data[0]='A';gd.cells[0][5].data=(struct utf8_data){{'W'},1,2};
struct screen_write_ctx ctx={.s=&visible};
window_copy_write_one(&me,&ctx,0,0,0,6,&grid_default_cell,&grid_default_cell,&grid_default_cell,&grid_default_cell);
for(u_int x=0;x<6;x++) putchar(visible.cells[0][x].data.data[0]);putchar('\n');
return 0;
}
"#);
        let mut registry = HyperlinkRegistry::new();
        let mut data = pure_data(5, 2, &mut registry);
        data.cx = 2;
        data.showmark = true;
        data.mx = 2;
        data.search.marks = Some(vec![0, 1, 1, 0, 1, 0, 0, 0, 0, 0]);
        let mut style = plain_style();
        style.matched = GridCell {
            fg: Colour(4),
            bg: Colour(5),
            attr: GridAttributes::ITALICS,
            ..DEFAULT_CELL
        };
        style.current_match = GridCell {
            fg: Colour(6),
            bg: Colour(7),
            attr: GridAttributes::REVERSE,
            ..DEFAULT_CELL
        };
        style.mark = GridCell {
            fg: Colour(2),
            bg: Colour(3),
            attr: GridAttributes::BRIGHT,
            ..DEFAULT_CELL
        };
        style.current_line = GridCell {
            fg: Colour(1),
            attr: GridAttributes::UNDERSCORE,
            ..DEFAULT_CELL
        };
        let mut expected = String::new();
        for x in 0..5 {
            let mut cell = GridCell {
                us: Colour(11),
                flags: GridCellFlags::NOPALETTE,
                ..DEFAULT_CELL
            };
            update_style(&data, x, 0, &mut cell, &style);
            writeln!(
                expected,
                "{} {} {} {} {}",
                cell.fg.0,
                cell.bg.0,
                cell.attr.bits(),
                cell.us.0,
                cell.flags.bits()
            )
            .unwrap();
        }
        for setting in [
            CopyLineNumbers::Off,
            CopyLineNumbers::Option,
            CopyLineNumbers::Default,
        ] {
            for option in 0..5 {
                let mode = effective_line_number_mode(setting, option);
                writeln!(
                    expected,
                    "{mode} {} {}",
                    i32::from(mode >= 2),
                    i32::from(mode != 0)
                )
                .unwrap();
            }
        }
        for history in (0..1100).step_by(19) {
            writeln!(expected, "{}", gutter_width(history, 2)).unwrap();
        }
        for sx in 1..16 {
            for x in 0..20 {
                writeln!(
                    expected,
                    "{} {}",
                    offset_for_gutter(4, x, sx),
                    unoffset_for_gutter(4, x, sx)
                )
                .unwrap();
            }
        }
        let mut data = pure_data(8, 2, &mut registry);
        let grid = &mut data.backing.screen_mut().grid;
        let mut tab = GridCell {
            data: Utf8Data::set(b'\t'),
            flags: GridCellFlags::TAB,
            ..DEFAULT_CELL
        };
        tab.data.width = 3;
        grid.set_cell(0, 0, &tab);
        for x in [1, 2, 4] {
            grid.set_cell(
                x,
                0,
                &GridCell {
                    flags: GridCellFlags::PADDING,
                    ..DEFAULT_CELL
                },
            );
        }
        grid.set_cell(
            3,
            0,
            &GridCell {
                data: Utf8Data::set(b'A'),
                ..DEFAULT_CELL
            },
        );
        let mut wide = GridCell {
            data: Utf8Data::set(b'W'),
            ..DEFAULT_CELL
        };
        wide.data.width = 2;
        grid.set_cell(5, 0, &wide);
        let mut visible =
            Screen::new(6, 2, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut visible,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        write_one(&data, &mut ctx, 0, 0, 0, 6, &plain_style());
        ctx.finish();
        writeln!(expected, "{}", String::from_utf8(row(&visible, 0)).unwrap()).unwrap();
        let dir = PathBuf::from("/tmp/swarm-rmux-build")
            .join(format!("copy-render-c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfile = dir.join("render.c");
        let binary = dir.join("render");
        std::fs::write(&cfile, c).unwrap();
        let output = match Command::new("cc")
            .args(["-std=c99", "-Wno-unused-function"])
            .arg(&cfile)
            .arg("-o")
            .arg(&binary)
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("SKIP copy render C differential: cc unavailable");
                std::fs::remove_dir_all(dir).unwrap();
                return;
            }
            Err(error) => panic!("C compiler: {error}"),
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = Command::new(&binary).output().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if std::env::var_os("RMUX_COPY_RENDER_MUTATE").is_some() {
            expected.insert(0, '!');
        }
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected);
    }
}
