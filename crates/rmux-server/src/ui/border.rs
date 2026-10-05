// Ported from tmux window-border.c @ 8f25579c
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

use crate::format::{FormatFlags, FormatTagFlags, FormatTree};
use crate::ids::{ClientId, PaneId, WindowId};
use crate::model::pane::{pane_get_pane_lines, pane_get_pane_status, pane_index};
use crate::model::{Server, Window};
use crate::ui::redraw::{RedrawSpan, redraw_get_status_border_cell_type};
use crate::ui::status::PaneStatusPosition;
use crate::ui::styles;
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::screen::borders::border_cell;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{BorderCell, BoxLines, PaneLines, Screen, ScreenResetPolicy};
use rmux_util::utf8::Utf8Data;

pub fn pane_lines_of(srv: &Server, wp: PaneId) -> PaneLines {
    PaneLines::try_from(pane_get_pane_lines(srv, wp) as i32).unwrap_or(PaneLines::Single)
}

pub fn pane_status_of(srv: &Server, wp: PaneId) -> PaneStatusPosition {
    PaneStatusPosition::try_from(pane_get_pane_status(srv, wp) as i32)
        .unwrap_or(PaneStatusPosition::Off)
}

fn window_set_fill_cell(srv: &mut Server, w: WindowId, inside: bool) -> GridCell {
    let mut gc = DEFAULT_CELL;
    gc.attr.insert(GridAttributes::CHARSET);
    border_cell(BoxLines::Single, BorderCell::None, &mut gc);

    let Some(win) = srv.windows.get(w) else {
        return gc;
    };
    let (public_id, active, options) = (win.public_id, win.active, win.options);
    let mut ft = FormatTree::create(
        None,
        None,
        FormatTagFlags::WINDOW.bits() | public_id,
        FormatFlags::NOJOBS,
        srv,
    );
    ft.defaults(
        srv,
        crate::format::FormatContext {
            window: Some(w),
            pane: active,
            ..Default::default()
        },
    );
    ft.add(b"is_inside", format!("{}", inside as i32).into());
    ft.add(b"is_outside", format!("{}", !inside as i32).into());
    let value = srv.options.get_string(options, b"fill-character").to_vec();
    let expanded = ft.expand(srv, &value);
    ft.release(srv);

    let Ok(mut s) = Screen::new(1, 1, 0, ScreenResetPolicy::default(), &mut srv.hyperlinks) else {
        return gc;
    };
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut s,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut srv.hyperlinks,
    );
    crate::format::draw::draw(&mut ctx, &DEFAULT_CELL, 1, &expanded, None, false);
    ctx.finish();
    let new_gc = s.grid.view_get_cell(0, 0);
    if new_gc.data.width == 1 {
        gc = new_gc;
    }
    let _ = s.release(&mut srv.hyperlinks);
    gc
}

/// Set window fill cells.
pub fn window_set_fill_cells(srv: &mut Server, w: WindowId) {
    let inside = window_set_fill_cell(srv, w, true);
    let outside = window_set_fill_cell(srv, w, false);
    if let Some(win) = srv.windows.get_mut(w) {
        win.inside_cell = inside;
        win.outside_cell = outside;
    }
}

/// Merge a window fill cell over an existing style.
fn window_copy_fill_cell(gc: &mut GridCell, fill: &GridCell) {
    gc.data.copy_from(&fill.data);
    gc.attr.insert(fill.attr);
    gc.flags.insert(fill.flags);
    if fill.fg != Colour::DEFAULT {
        gc.fg = fill.fg;
    }
    if fill.bg != Colour::DEFAULT {
        gc.bg = fill.bg;
    }
    if fill.us != Colour::DEFAULT {
        gc.us = fill.us;
    }
}

/// Get window fill cell.
pub fn window_get_fill_cell(w: &Window, inside: bool, gc: &mut GridCell) {
    if inside {
        window_copy_fill_cell(gc, &w.inside_cell);
    } else {
        window_copy_fill_cell(gc, &w.outside_cell);
    }
}

/// Get border cell. `index` is the pane index for PANE_LINES_NUMBER (None
/// stands for a missing pane or window_pane_index failure).
pub fn window_get_border_cell(
    index: Option<u32>,
    pane_lines: PaneLines,
    cell_type: BorderCell,
    gc: &mut GridCell,
) {
    // The glyph tables have no scrollbar entry; the scrollbar is never a
    // border glyph, so treat it as CELL_NONE.
    let cell_type = if cell_type == BorderCell::Scrollbar {
        BorderCell::None
    } else {
        cell_type
    };
    match pane_lines {
        PaneLines::Number => {
            if cell_type == BorderCell::None {
                gc.attr.insert(GridAttributes::CHARSET);
                border_cell(BoxLines::Single, BorderCell::None, gc);
                return;
            }
            gc.attr.remove(GridAttributes::CHARSET);
            gc.data = match index {
                Some(idx) => Utf8Data::set(b'0' + (idx % 10) as u8),
                None => Utf8Data::set(b'*'),
            };
        }
        PaneLines::Double => border_cell(BoxLines::Double, cell_type, gc),
        PaneLines::Heavy => border_cell(BoxLines::Heavy, cell_type, gc),
        PaneLines::Rounded => border_cell(BoxLines::Rounded, cell_type, gc),
        PaneLines::Simple => border_cell(BoxLines::Simple, cell_type, gc),
        PaneLines::None | PaneLines::Spaces => {
            gc.attr.remove(GridAttributes::CHARSET);
            gc.data = Utf8Data::set(b' ');
        }
        PaneLines::Single => border_cell(BoxLines::Single, cell_type, gc),
    }
}

/// Get pane border cell.
pub fn window_pane_get_border_cell(
    srv: &Server,
    wp: PaneId,
    cell_type: BorderCell,
    gc: &mut GridCell,
) {
    let pane_lines = pane_lines_of(srv, wp);
    window_get_border_cell(pane_index(srv, wp), pane_lines, cell_type, gc);
}

/// Get pane border style (cached per pane per draw).
pub fn window_pane_get_border_style(srv: &mut Server, wp: PaneId, c: ClientId) -> GridCell {
    let session = srv.clients.get(c).and_then(|c| c.session);
    let curw = styles::client_winlink(srv, c);
    let active = styles::client_window(srv, c).and_then(|w| srv.windows.get(w)?.active);
    let Some(p) = srv.panes.get(wp) else {
        return DEFAULT_CELL;
    };
    let is_active = active == Some(wp);
    let (set, saved) = if is_active {
        (p.active_border_gc_set, p.active_border_gc)
    } else {
        (p.border_gc_set, p.border_gc)
    };
    if set {
        return saved;
    }
    let option: &[u8] = if is_active {
        b"pane-active-border-style"
    } else {
        b"pane-border-style"
    };
    let options = p.options;
    let mut ft = styles::create_defaults(srv, None, Some(c), session, curw, Some(wp));
    let mut gc = DEFAULT_CELL;
    styles::style_apply(srv, &mut gc, options, option, Some(&mut ft));
    ft.release(srv);
    if let Some(p) = srv.panes.get_mut(wp) {
        if is_active {
            p.active_border_gc = gc;
            p.active_border_gc_set = true;
        } else {
            p.border_gc = gc;
            p.border_gc_set = true;
        }
    }
    gc
}

/// Build pane status line. Returns true when the grid changed.
pub fn window_make_pane_status(
    srv: &mut Server,
    wp: PaneId,
    c: ClientId,
    width: u32,
    spans: &[RedrawSpan],
    first: Option<usize>,
) -> bool {
    if pane_status_of(srv, wp) == PaneStatusPosition::Off || width == 0 {
        return false;
    }
    let Some(p) = srv.panes.get(wp) else {
        return false;
    };
    let (public_id, options) = (p.public_id, p.options);
    let session = srv.clients.get(c).and_then(|c| c.session);
    let curw = styles::client_winlink(srv, c);

    let mut ft = FormatTree::create(
        Some(c),
        None,
        FormatTagFlags::PANE.bits() | public_id,
        FormatFlags::STATUS,
        srv,
    );
    ft.defaults(
        srv,
        crate::format::FormatContext {
            evaluated_client: Some(c),
            session,
            winlink: curw,
            pane: Some(wp),
            ..Default::default()
        },
    );
    let fmt = srv
        .options
        .get_string(options, b"pane-border-format")
        .to_vec();
    let expanded = ft.expand_time(srv, &fmt);
    ft.release(srv);

    let mut gc = window_pane_get_border_style(srv, wp, c);
    let pane_lines = pane_lines_of(srv, wp);
    let index = pane_index(srv, wp);

    let Ok(fresh) = Screen::new(
        width,
        1,
        0,
        ScreenResetPolicy::default(),
        &mut srv.hyperlinks,
    ) else {
        return false;
    };
    let Some(p) = srv.panes.get_mut(wp) else {
        return false;
    };
    let mut old = std::mem::replace(&mut p.status_screen, fresh);
    p.status_screen.mode = rmux_emu::screen::ScreenMode(0);
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut p.status_screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut srv.hyperlinks,
    );
    let mut cursor = first;
    for i in 0..width {
        let cell_type = redraw_get_status_border_cell_type(spans, wp, &mut cursor, i);
        window_get_border_cell(index, pane_lines, cell_type, &mut gc);
        ctx.cell(&gc);
    }
    gc.attr.remove(GridAttributes::CHARSET);

    ctx.cursormove(0, 0, false);
    p.border_status_line.ranges.clear();
    crate::format::draw::draw(
        &mut ctx,
        &gc,
        width,
        &expanded,
        Some(&mut p.border_status_line.ranges),
        false,
    );
    ctx.finish();
    p.border_status_line.expanded = expanded;

    let changed = p.status_screen.grid.compare(&old.grid);
    let _ = old.release(&mut srv.hyperlinks);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_data(gc: &GridCell) -> &[u8] {
        &gc.data.data[..gc.data.size as usize]
    }

    #[test]
    fn number_borders_use_index_modulo_ten_or_star() {
        let mut gc = DEFAULT_CELL;
        window_get_border_cell(Some(13), PaneLines::Number, BorderCell::Lr, &mut gc);
        assert_eq!(cell_data(&gc), b"3");
        assert!(!gc.attr.contains(GridAttributes::CHARSET));
        window_get_border_cell(None, PaneLines::Number, BorderCell::Ud, &mut gc);
        assert_eq!(cell_data(&gc), b"*");
        window_get_border_cell(Some(1), PaneLines::Number, BorderCell::None, &mut gc);
        assert_eq!(cell_data(&gc), b"~");
        assert!(gc.attr.contains(GridAttributes::CHARSET));
    }

    #[test]
    fn line_styles_select_glyph_tables() {
        let mut gc = DEFAULT_CELL;
        window_get_border_cell(None, PaneLines::Single, BorderCell::Lrud, &mut gc);
        assert_eq!(cell_data(&gc), b"n");
        assert!(gc.attr.contains(GridAttributes::CHARSET));
        window_get_border_cell(None, PaneLines::Simple, BorderCell::Ud, &mut gc);
        assert_eq!(cell_data(&gc), b"|");
        assert!(!gc.attr.contains(GridAttributes::CHARSET));
        window_get_border_cell(None, PaneLines::Spaces, BorderCell::Lr, &mut gc);
        assert_eq!(cell_data(&gc), b" ");
        window_get_border_cell(None, PaneLines::None, BorderCell::Lr, &mut gc);
        assert_eq!(cell_data(&gc), b" ");
        window_get_border_cell(None, PaneLines::Double, BorderCell::Lr, &mut gc);
        assert_eq!(cell_data(&gc), "═".as_bytes());
        window_get_border_cell(None, PaneLines::Heavy, BorderCell::Ud, &mut gc);
        assert_eq!(cell_data(&gc), "┃".as_bytes());
        window_get_border_cell(None, PaneLines::Rounded, BorderCell::Rd, &mut gc);
        assert_eq!(cell_data(&gc), "╭".as_bytes());
    }

    #[test]
    fn fill_cell_merge_keeps_default_colours() {
        let mut fill = DEFAULT_CELL;
        fill.data = Utf8Data::set(b'~');
        fill.fg = Colour(3);
        fill.attr = GridAttributes::DIM;
        let mut gc = DEFAULT_CELL;
        gc.bg = Colour(5);
        gc.attr = GridAttributes::REVERSE;
        window_copy_fill_cell(&mut gc, &fill);
        assert_eq!(cell_data(&gc), b"~");
        assert_eq!(gc.fg, Colour(3));
        assert_eq!(gc.bg, Colour(5));
        assert_eq!(gc.attr, GridAttributes::DIM | GridAttributes::REVERSE);
    }
}
