// Ported from tmux window-panes.c @ 8f25579c
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

use crate::client::ResolvedMouseEvent;
use crate::cmd::Command;
use crate::cmd::arguments::{self, Args, ArgsCommandState};
use crate::cmd::find::CmdFindState;
use crate::cmd::queue;
use crate::format::draw::draw as format_draw;
use crate::ids::{
    ClientId, LayoutCellId, ModeId, PaneId, QueueItemId, SessionId, TimerId, WindowId, WinlinkId,
};
use crate::layout::{Cells, LayoutCell, LayoutType, add_horizontal_border};
use crate::model::pane::{
    PaneMode, PaneModeDriver, pane_at_index, pane_index, pane_is_visible, pane_reset_mode,
};
use crate::model::window::{window_get_pane_status, window_unzoom, window_zoom};
use crate::model::winlink::winlink_find_by_window;
use crate::model::{ModelError, PaneFlags, WindowFlags};
use crate::modes::WindowModeFlags;
use crate::modes::clock::CLOCK_TABLE;
use crate::server::Server;
use crate::server::event_loop::schedule_deferred;
use crate::server::operations::{
    server_redraw_window, server_redraw_window_borders, server_status_window, server_unzoom_window,
};
use crate::ui::status::PaneStatusPosition;
use crate::ui::styles::{create_defaults, style_apply};
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::screen::borders::border_cell;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{BorderCell, BoxLines, Screen, ScreenMode, ScreenResetPolicy};
use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, KeyMasks, SpecialKey};
use rmux_util::utf8::Utf8Data;
use std::time::Duration;

pub const NAME: &[u8] = b"panes-mode";
pub const FLAGS: WindowModeFlags = WindowModeFlags(
    WindowModeFlags::HIDE_PANE_STATUS.0
        | WindowModeFlags::NO_STACK.0
        | WindowModeFlags::FILL_WINDOW.0
        | WindowModeFlags::HIDE_SCROLLBARS.0,
);
const DEFAULT_TEMPLATE: &[u8] = b"select-pane -t \"%%%\"";

/// `WINDOW_PANES_BORDER_*` bits in the border map.
pub struct PanesBorderBits;
impl PanesBorderBits {
    pub const L: u8 = 0x1;
    pub const R: u8 = 0x2;
    pub const U: u8 = 0x4;
    pub const D: u8 = 0x8;
}
use PanesBorderBits as B;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanesArea {
    pub pane: PaneId,
    pub x: u32,
    pub y: u32,
    pub sx: u32,
    pub sy: u32,
}

pub struct PanesModeData {
    pub wp: PaneId,
    pub session: Option<SessionId>,
    pub source_session: Option<SessionId>,
    pub source_window: WindowId,
    pub preview: Option<Screen>,
    pub timer: Option<(TimerId, u64)>,
    pub state: Option<ArgsCommandState>,
    pub delay: u32,
    pub ignore_keys: bool,
    pub zoomed: Option<bool>,
    pub areas: Vec<PanesArea>,
    map: Vec<u8>,
}

/// `display-panes` driver: carries the command arguments into `init`.
pub struct PanesMode {
    args: Args,
    item: QueueItemId,
    target: CmdFindState,
    source: CmdFindState,
}

impl PanesMode {
    pub fn new(args: Args, item: QueueItemId, target: CmdFindState, source: CmdFindState) -> Self {
        Self {
            args,
            item,
            target,
            source,
        }
    }
}

fn mode_mut(server: &mut Server, id: ModeId) -> Option<&mut PaneMode> {
    server
        .panes
        .get_mut(id.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == id)
}

fn take_data(server: &mut Server, id: ModeId) -> Option<Box<PanesModeData>> {
    let mode = mode_mut(server, id)?;
    let data = mode.data.take()?;
    match data.downcast::<PanesModeData>() {
        Ok(data) => Some(data),
        Err(other) => {
            mode.data = Some(other);
            None
        }
    }
}

fn restore_data(
    server: &mut Server,
    id: ModeId,
    data: Box<PanesModeData>,
) -> Option<Box<PanesModeData>> {
    match mode_mut(server, id) {
        Some(mode) => {
            mode.data = Some(data);
            None
        }
        None => Some(data),
    }
}

/// `window_panes_get_source`: `(session, winlink, window)`.
fn get_source(
    server: &Server,
    data: &PanesModeData,
) -> Option<(Option<SessionId>, Option<WinlinkId>, WindowId)> {
    let w = data.source_window;
    server.windows.get(w)?;
    let mut s = data
        .source_session
        .filter(|s| server.sessions.get(*s).is_some());
    let mut wl = s.and_then(|s| winlink_find_by_window(server, s, w));
    if wl.is_none() {
        s = data.session.filter(|s| server.sessions.get(*s).is_some());
        wl = s.and_then(|s| winlink_find_by_window(server, s, w));
    }
    Some((s, wl, w))
}

fn set_preview(server: &mut Server, wp: PaneId) -> Option<Screen> {
    let p = server.panes.get(wp)?;
    let (sx, sy) = (p.base.grid.sx(), p.base.grid.sy());
    let mut dst = Screen::new(
        sx,
        sy,
        0,
        ScreenResetPolicy::default(),
        &mut server.hyperlinks,
    )
    .ok()?;
    let Server {
        panes, hyperlinks, ..
    } = server;
    let src = &panes.get(wp)?.base;
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut dst,
        &mut sink,
        ScreenWritePolicy::default(),
        hyperlinks,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.fast_copy(src, 0, src.grid.hsize(), sx, sy);
    ctx.finish();
    dst.mode = src.mode;
    dst.cx = src.cx;
    dst.cy = src.cy;
    Some(dst)
}

fn pane_cell(server: &Server, wp: PaneId) -> Option<LayoutCellId> {
    let p = server.panes.get(wp)?;
    p.saved_layout_cell.or(p.layout_cell)
}

fn pane_floating(server: &Server, wp: PaneId) -> bool {
    pane_cell(server, wp)
        .and_then(|lc| server.layout_cells.get(lc))
        .is_some_and(LayoutCell::is_floating)
}

fn pane_visible(server: &Server, wp: PaneId) -> bool {
    server
        .panes
        .get(wp)
        .is_some_and(|p| p.saved_layout_cell.is_some())
        || pane_is_visible(server, wp)
}

fn status_position(status: i64) -> PaneStatusPosition {
    i32::try_from(status)
        .ok()
        .and_then(|v| PaneStatusPosition::try_from(v).ok())
        .unwrap_or(PaneStatusPosition::Off)
}

/// `window_panes_get_geometry`: the pane cell mapped into the mode screen.
#[allow(clippy::too_many_arguments)]
pub fn get_geometry(
    cells: &Cells,
    root: LayoutCellId,
    lc: LayoutCellId,
    status: PaneStatusPosition,
    osx: u32,
    osy: u32,
    dsx: u32,
    dsy: u32,
) -> Option<(u32, u32, u32, u32)> {
    let cell = cells.get(lc)?;
    if osx == 0 || osy == 0 || dsx == 0 || dsy == 0 {
        return None;
    }
    let (xoff, yoff) = (cell.g.xoff as u32, cell.g.yoff as u32);
    let (x, y, mut x2, mut y2) = if osx <= dsx && osy <= dsy {
        (xoff, yoff, xoff + cell.g.sx, yoff + cell.g.sy)
    } else {
        (
            xoff * dsx / osx,
            yoff * dsy / osy,
            (xoff + cell.g.sx) * dsx / osx,
            (yoff + cell.g.sy) * dsy / osy,
        )
    };
    if x >= dsx || y >= dsy {
        return None;
    }
    if x2 <= x {
        x2 = x + 1;
    }
    if y2 <= y {
        y2 = y + 1;
    }
    x2 = x2.min(dsx);
    y2 = y2.min(dsy);
    let (sx, mut sy) = (x2 - x, y2 - y);
    if sx == 0 || sy == 0 {
        return None;
    }
    let mut y = y;
    if add_horizontal_border(cells, Some(root), lc, status) && sy > 1 {
        if status == PaneStatusPosition::Top {
            y += 1;
        }
        sy -= 1;
    }
    Some((x, y, sx, sy))
}

fn map_x(x: u32, osx: u32, dsx: u32) -> i32 {
    if osx <= dsx {
        x as i32
    } else {
        (x * dsx / osx) as i32
    }
}

fn map_y(y: u32, osy: u32, dsy: u32) -> i32 {
    if osy <= dsy {
        y as i32
    } else {
        (y * dsy / osy) as i32
    }
}

fn next_tiled_cell(cells: &Cells, siblings: &[LayoutCellId], index: usize) -> Option<LayoutCellId> {
    siblings[index + 1..]
        .iter()
        .copied()
        .find(|id| cells.get(*id).is_some_and(|c| !c.is_floating()))
}

fn mark_border(map: &mut [u8], dsx: u32, dsy: u32, x: i32, y: i32, mask: u8) {
    if x >= 0 && (x as u32) < dsx && y >= 0 && (y as u32) < dsy {
        map[(y as u32 * dsx + x as u32) as usize] |= mask;
    }
}

fn mark_vline(map: &mut [u8], dsx: u32, dsy: u32, x: i32, mut y: i32, mut y2: i32) {
    if x < 0 || x as u32 >= dsx || y2 <= y {
        return;
    }
    if y < 0 {
        y = 0;
    }
    if y2 as u32 > dsy {
        y2 = dsy as i32;
    }
    for yy in y..y2 {
        let mut mask = 0;
        if yy > y {
            mask |= B::U;
        }
        if yy + 1 < y2 {
            mask |= B::D;
        }
        if mask == 0 {
            mask = B::U | B::D;
        }
        mark_border(map, dsx, dsy, x, yy, mask);
    }
}

fn mark_hline(map: &mut [u8], dsx: u32, dsy: u32, mut x: i32, mut x2: i32, y: i32) {
    if y < 0 || y as u32 >= dsy || x2 <= x {
        return;
    }
    if x < 0 {
        x = 0;
    }
    if x2 as u32 > dsx {
        x2 = dsx as i32;
    }
    for xx in x..x2 {
        let mut mask = 0;
        if xx > x {
            mask |= B::L;
        }
        if xx + 1 < x2 {
            mask |= B::R;
        }
        if mask == 0 {
            mask = B::L | B::R;
        }
        mark_border(map, dsx, dsy, xx, y, mask);
    }
}

fn mark_borders_cell(
    map: &mut [u8],
    cells: &Cells,
    lc: LayoutCellId,
    osx: u32,
    osy: u32,
    dsx: u32,
    dsy: u32,
) {
    let Some(cell) = cells.get(lc) else {
        return;
    };
    if cell.kind == LayoutType::Windowpane {
        return;
    }
    for (n, &child) in cell.children.iter().enumerate() {
        mark_borders_cell(map, cells, child, osx, osy, dsx, dsy);
        let Some(c) = cells.get(child) else {
            continue;
        };
        if c.is_floating() || next_tiled_cell(cells, &cell.children, n).is_none() {
            continue;
        }
        if cell.kind == LayoutType::Leftright {
            let x = map_x(c.g.xoff as u32 + c.g.sx, osx, dsx);
            let y = map_y(cell.g.yoff as u32, osy, dsy);
            let y2 = map_y(cell.g.yoff as u32 + cell.g.sy, osy, dsy);
            mark_vline(map, dsx, dsy, x, y, y2);
        } else {
            let x = map_x(cell.g.xoff as u32, osx, dsx);
            let x2 = map_x(cell.g.xoff as u32 + cell.g.sx, osx, dsx);
            let y = map_y(c.g.yoff as u32 + c.g.sy, osy, dsy);
            mark_hline(map, dsx, dsy, x, x2, y);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mark_pane_status_borders(
    map: &mut [u8],
    server: &Server,
    w: WindowId,
    root: LayoutCellId,
    status: PaneStatusPosition,
    osx: u32,
    osy: u32,
    dsx: u32,
    dsy: u32,
) {
    if status != PaneStatusPosition::Top && status != PaneStatusPosition::Bottom {
        return;
    }
    let Some(window) = server.windows.get(w) else {
        return;
    };
    let cells = &server.layout_cells;
    for &wp in &window.panes {
        if !pane_visible(server, wp) {
            continue;
        }
        let Some(lc) = pane_cell(server, wp) else {
            continue;
        };
        if !add_horizontal_border(cells, Some(root), lc, status) {
            continue;
        }
        let Some(c) = cells.get(lc) else {
            continue;
        };
        let x = map_x(c.g.xoff as u32, osx, dsx);
        let x2 = map_x(c.g.xoff as u32 + c.g.sx, osx, dsx);
        let y = if status == PaneStatusPosition::Top {
            map_y(c.g.yoff as u32, osy, dsy)
        } else {
            map_y(c.g.yoff as u32 + c.g.sy, osy, dsy) - 1
        };
        mark_hline(map, dsx, dsy, x, x2, y);
    }
}

/// `(x, y, x2, y2)` of a floating pane's border ring; `-1` is off screen.
type BorderRing = (i32, i32, i32, i32);

/// `window_panes_get_floating_borders`: `(x, y, x2, y2)` of the border ring.
fn floating_borders(
    cells: &Cells,
    lc: Option<LayoutCellId>,
    osx: u32,
    osy: u32,
    dsx: u32,
    dsy: u32,
) -> Option<BorderRing> {
    let cell = cells.get(lc?)?;
    if !cell.is_floating() {
        return None;
    }
    let (xoff, yoff) = (cell.g.xoff as u32, cell.g.yoff as u32);
    let x = if xoff == 0 {
        -1
    } else {
        map_x(xoff - 1, osx, dsx)
    };
    let y = if yoff == 0 {
        -1
    } else {
        map_y(yoff - 1, osy, dsy)
    };
    let x2 = map_x(xoff + cell.g.sx, osx, dsx);
    let y2 = map_y(yoff + cell.g.sy, osy, dsy);
    Some((x, y, x2, y2))
}

/// `window_panes_clip_floating_pane`: clip the pane area inside its border.
/// `None` when nothing remains.
pub fn clip_floating_pane(
    borders: Option<BorderRing>,
    dsx: u32,
    dsy: u32,
    (x, y, sx, sy): (u32, u32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let Some((bx, by, bx2, by2)) = borders else {
        return Some((x, y, sx, sy));
    };
    let mut px = x as i32;
    let mut py = y as i32;
    let mut px2 = px + sx as i32 - 1;
    let mut py2 = py + sy as i32 - 1;
    if bx >= 0 && px <= bx {
        px = bx + 1;
    }
    if by >= 0 && py <= by {
        py = by + 1;
    }
    if (bx2 as u32) < dsx && px2 >= bx2 {
        px2 = bx2 - 1;
    }
    if (by2 as u32) < dsy && py2 >= by2 {
        py2 = by2 - 1;
    }
    if px2 < px || py2 < py {
        return None;
    }
    Some((
        px as u32,
        py as u32,
        (px2 - px + 1) as u32,
        (py2 - py + 1) as u32,
    ))
}

/// `window_panes_border_cell_type`.
pub fn border_cell_type(mask: u8) -> BorderCell {
    const L: u8 = B::L;
    const R: u8 = B::R;
    const U: u8 = B::U;
    const D: u8 = B::D;
    match mask {
        m if m == L | R | U | D => BorderCell::Lrud,
        m if m == L | R | U => BorderCell::Lru,
        m if m == L | R | D => BorderCell::Lrd,
        m if m == L | R || m == L || m == R => BorderCell::Lr,
        m if m == L | U | D => BorderCell::Uld,
        m if m == L | U => BorderCell::Lu,
        m if m == L | D => BorderCell::Ld,
        m if m == R | U | D => BorderCell::Urd,
        m if m == R | U => BorderCell::Ru,
        m if m == R | D => BorderCell::Rd,
        m if m == U | D || m == U || m == D => BorderCell::Ud,
        _ => BorderCell::None,
    }
}

fn has_horizontal(mask: u8) -> bool {
    mask & (B::L | B::R) != 0
}

fn has_vertical(mask: u8) -> bool {
    mask & (B::U | B::D) != 0
}

/// `window_panes_mark_border_joins_cell`.
pub fn mark_border_joins_cell(
    map: &mut [u8],
    cells: &Cells,
    lc: LayoutCellId,
    osx: u32,
    osy: u32,
    dsx: u32,
    dsy: u32,
) {
    let Some(cell) = cells.get(lc) else {
        return;
    };
    if cell.kind == LayoutType::Windowpane {
        return;
    }
    for (n, &child) in cell.children.iter().enumerate() {
        mark_border_joins_cell(map, cells, child, osx, osy, dsx, dsy);
        let Some(c) = cells.get(child) else {
            continue;
        };
        if c.is_floating() || next_tiled_cell(cells, &cell.children, n).is_none() {
            continue;
        }
        if cell.kind == LayoutType::Leftright {
            let x = map_x(c.g.xoff as u32 + c.g.sx, osx, dsx);
            let y = map_y(cell.g.yoff as u32, osy, dsy);
            let y2 = map_y(cell.g.yoff as u32 + cell.g.sy, osy, dsy);
            if x < 0 || x as u32 >= dsx {
                continue;
            }
            if y > 0 && has_horizontal(map[((y - 1) as u32 * dsx + x as u32) as usize]) {
                mark_border(map, dsx, dsy, x, y - 1, B::D);
                mark_border(map, dsx, dsy, x, y, B::U);
            }
            if (y2 as u32) < dsy && has_horizontal(map[(y2 as u32 * dsx + x as u32) as usize]) {
                mark_border(map, dsx, dsy, x, y2, B::U);
                mark_border(map, dsx, dsy, x, y2 - 1, B::D);
            }
        } else {
            let x = map_x(cell.g.xoff as u32, osx, dsx);
            let x2 = map_x(cell.g.xoff as u32 + cell.g.sx, osx, dsx);
            let y = map_y(c.g.yoff as u32 + c.g.sy, osy, dsy);
            if y < 0 || y as u32 >= dsy {
                continue;
            }
            if x > 0 && has_vertical(map[(y as u32 * dsx + (x - 1) as u32) as usize]) {
                mark_border(map, dsx, dsy, x - 1, y, B::R);
                mark_border(map, dsx, dsy, x, y, B::L);
            }
            if (x2 as u32) < dsx && has_vertical(map[(y as u32 * dsx + x2 as u32) as usize]) {
                mark_border(map, dsx, dsy, x2, y, B::L);
                mark_border(map, dsx, dsy, x2 - 1, y, B::R);
            }
        }
    }
}

fn draw_map(ctx: &mut ScreenWriteCtx<'_>, map: &[u8], gc: &GridCell, dsx: u32, dsy: u32) {
    for yy in 0..dsy {
        for xx in 0..dsx {
            let mask = map[(yy * dsx + xx) as usize];
            if mask == 0 {
                continue;
            }
            let mut border_gc = *gc;
            border_cell(BoxLines::Single, border_cell_type(mask), &mut border_gc);
            ctx.cursormove(xx as i32, yy as i32, false);
            ctx.cell(&border_gc);
        }
    }
}

fn clear_map(map: &mut Vec<u8>, dsx: u32, dsy: u32) {
    map.clear();
    map.resize((dsx * dsy) as usize, 0);
}

/// One pane's drawing plan, computed before the screen is borrowed.
struct DrawPane {
    wp: PaneId,
    index: u32,
    area: (u32, u32, u32, u32),
    fgc: GridCell,
    format: Option<ByteString>,
}

#[allow(clippy::too_many_arguments)]
fn plan_pane(
    server: &mut Server,
    data: &PanesModeData,
    wp: PaneId,
    root: LayoutCellId,
    status: PaneStatusPosition,
    (osx, osy, dsx, dsy): (u32, u32, u32, u32),
    source: (Option<SessionId>, Option<WinlinkId>, WindowId),
    format: &[u8],
) -> Option<DrawPane> {
    if !pane_visible(server, wp) {
        return None;
    }
    let lc = pane_cell(server, wp)?;
    let geometry = get_geometry(&server.layout_cells, root, lc, status, osx, osy, dsx, dsy)?;
    let floating = floating_borders(&server.layout_cells, Some(lc), osx, osy, dsx, dsy);
    let area = clip_floating_pane(floating, dsx, dsy, geometry)?;
    let index = pane_index(server, wp)?;

    // window_panes_draw_number: colour and format text.
    let (s, mut wl, w) = source;
    if wl.is_none() {
        wl = s
            .and_then(|s| server.sessions.get(s))
            .and_then(|s| s.current);
    }
    let active = server.windows.get(w).and_then(|w| w.active) == Some(wp);
    let name: &[u8] = if active {
        b"display-panes-active-colour"
    } else {
        b"display-panes-colour"
    };
    let oo = server
        .windows
        .get(server.panes.get(data.wp)?.window)?
        .options;
    let mut ft = create_defaults(server, None, None, s, wl, Some(wp));
    let mut fgc = GridCell::default();
    style_apply(server, &mut fgc, oo, name, Some(&mut ft));
    ft.release(server);

    let format = if format.is_empty() || source.0.is_none() {
        None
    } else {
        let (s, wl, _) = source;
        let mut ft = create_defaults(server, None, None, s, wl, Some(wp));
        let expanded = ft.expand(server, format);
        ft.release(server);
        Some(expanded)
    };
    Some(DrawPane {
        wp,
        index,
        area,
        fgc,
        format,
    })
}

fn draw_format(
    ctx: &mut ScreenWriteCtx<'_>,
    plan: &DrawPane,
    x: u32,
    y: u32,
    sx: u32,
    gc: &GridCell,
) {
    if sx == 0 {
        return;
    }
    if let Some(expanded) = plan.format.as_ref().filter(|e| !e.is_empty()) {
        ctx.cursormove(x as i32, y as i32, false);
        format_draw(ctx, gc, sx, expanded, None, false);
    }
}

fn draw_number(ctx: &mut ScreenWriteCtx<'_>, plan: &DrawPane, has_format: bool) {
    let (x, y, sx, sy) = plan.area;
    let buf = plan.index.to_string().into_bytes();
    let len = buf.len() as u32;
    let lbuf: Option<u8> =
        (plan.index > 9 && plan.index < 35).then(|| b'a' + (plan.index - 10) as u8);
    let llen = u32::from(lbuf.is_some());
    if sx < len {
        return;
    }
    let fgc = plan.fgc;
    let mut bgc = DEFAULT_CELL;
    bgc.bg = fgc.fg;
    bgc.data = Utf8Data::set(b' ');

    let mut width = len * 6 - 1;
    if sx < width || sy < if has_format { 7 } else { 5 } {
        width = len;
        if llen != 0 && sx > len + llen {
            width += llen + 1;
        }
        let cx = x + (sx - width) / 2;
        let cy = y + sy / 2;
        ctx.cursormove(cx as i32, cy as i32, false);
        ctx.puts(&fgc, &buf);
        if let Some(l) = lbuf.filter(|_| width > len) {
            ctx.puts(&fgc, &[b' ', l]);
        }
        if has_format && sy > 1 {
            draw_format(ctx, plan, x, y, sx, &fgc);
        }
        return;
    }

    let mut px = (sx - width) / 2;
    let py = (sy - 5) / 2;
    for &ch in &buf {
        if !ch.is_ascii_digit() {
            continue;
        }
        let idx = usize::from(ch - b'0');
        for j in 0..5u32 {
            for i in 0..5u32 {
                if CLOCK_TABLE[idx][j as usize][i as usize] == 0 {
                    continue;
                }
                ctx.cursormove((x + px + i) as i32, (y + py + j) as i32, false);
                ctx.cell(&bgc);
            }
        }
        px += 6;
    }
    if sy <= 6 {
        return;
    }
    draw_format(ctx, plan, x, y, sx, &fgc);
    if let Some(l) = lbuf {
        let cx = x + px - llen - 1;
        let cy = y + py + 5;
        ctx.cursormove(cx as i32, cy as i32, false);
        ctx.puts(&fgc, &[l]);
    }
}

fn draw_pane(
    ctx: &mut ScreenWriteCtx<'_>,
    panes: &crate::ids::Arena<crate::model::Pane, PaneId>,
    data: &mut PanesModeData,
    plan: &DrawPane,
    scaled: bool,
    has_format: bool,
) {
    let (x, y, sx, sy) = plan.area;
    data.areas.push(PanesArea {
        pane: plan.wp,
        x,
        y,
        sx,
        sy,
    });
    ctx.cursormove(x as i32, y as i32, false);
    let Some(pane) = panes.get(plan.wp) else {
        return;
    };
    let mut s = &pane.base;
    if let Some(preview) = &data.preview
        && plan.wp == data.wp
        && sx <= preview.grid.sx()
        && sy <= preview.grid.sy()
    {
        s = preview;
    }
    if !scaled {
        ctx.fast_copy(s, 0, s.grid.hsize(), sx, sy);
    } else {
        ctx.preview(s, sx, sy);
    }
    draw_number(ctx, plan, has_format);
}

fn clear_floating_area(
    ctx: &mut ScreenWriteCtx<'_>,
    borders: Option<BorderRing>,
    dsx: u32,
    dsy: u32,
) {
    let Some((mut x, mut y, mut x2, mut y2)) = borders else {
        return;
    };
    let gc = DEFAULT_CELL;
    if x < 0 {
        x = 0;
    }
    if y < 0 {
        y = 0;
    }
    if x2 as u32 >= dsx {
        x2 = dsx as i32 - 1;
    }
    if y2 as u32 >= dsy {
        y2 = dsy as i32 - 1;
    }
    if x2 < x || y2 < y {
        return;
    }
    for yy in y..=y2 {
        ctx.cursormove(x, yy, false);
        for _ in x..=x2 {
            ctx.cell(&gc);
        }
    }
}

fn draw_floating_border(
    ctx: &mut ScreenWriteCtx<'_>,
    map: &mut Vec<u8>,
    borders: Option<BorderRing>,
    gc: &GridCell,
    dsx: u32,
    dsy: u32,
) {
    if dsx == 0 || dsy == 0 {
        return;
    }
    let Some((x, y, x2, y2)) = borders else {
        return;
    };
    clear_map(map, dsx, dsy);
    mark_hline(map, dsx, dsy, x, x2 + 1, y);
    mark_hline(map, dsx, dsy, x, x2 + 1, y2);
    mark_vline(map, dsx, dsy, x, y, y2 + 1);
    mark_vline(map, dsx, dsy, x2, y, y2 + 1);
    draw_map(ctx, map, gc, dsx, dsy);
}

/// `window_panes_draw_screen`.
fn draw_screen(server: &mut Server, data: &mut PanesModeData, screen: &mut Screen) {
    let Some(source) = get_source(server, data) else {
        return;
    };
    let w = source.2;
    let Some(root) = server
        .windows
        .get(w)
        .and_then(|win| win.saved_layout_root.or(win.layout_root))
    else {
        return;
    };
    let Some((osx, osy)) = server.layout_cells.get(root).map(|c| (c.g.sx, c.g.sy)) else {
        return;
    };
    let dsx = screen.grid.sx();
    let dsy = screen.grid.sy();
    let status = status_position(window_get_pane_status(server, w));
    let scaled = !(osx <= dsx && osy <= dsy);

    data.areas.clear();

    // Everything that needs the whole server is computed first.
    let Some(mode_window) = server.panes.get(data.wp).map(|p| p.window) else {
        return;
    };
    let Some(oo) = server.windows.get(mode_window).map(|w| w.options) else {
        return;
    };
    let format = server
        .options
        .get_string(oo, b"display-panes-format")
        .to_vec();
    let has_format = !format.is_empty();
    let (tiled, floating): (Vec<PaneId>, Vec<PaneId>) = {
        let Some(win) = server.windows.get(w) else {
            return;
        };
        let tiled = win
            .panes
            .iter()
            .copied()
            .filter(|wp| !pane_floating(server, *wp))
            .collect();
        let floating = win
            .z_order
            .iter()
            .rev()
            .copied()
            .filter(|wp| pane_floating(server, *wp))
            .collect();
        (tiled, floating)
    };
    let sizes = (osx, osy, dsx, dsy);
    let tiled: Vec<DrawPane> = tiled
        .into_iter()
        .filter_map(|wp| plan_pane(server, data, wp, root, status, sizes, source, &format))
        .collect();
    let floating: Vec<(Option<DrawPane>, Option<BorderRing>)> = floating
        .into_iter()
        .map(|wp| {
            let borders = floating_borders(
                &server.layout_cells,
                pane_cell(server, wp),
                osx,
                osy,
                dsx,
                dsy,
            );
            (
                plan_pane(server, data, wp, root, status, sizes, source, &format),
                borders,
            )
        })
        .collect();
    let mut border_gc = GridCell::default();
    {
        let s = data.session;
        let curw = s
            .and_then(|s| server.sessions.get(s))
            .and_then(|s| s.current);
        let mut ft = create_defaults(server, None, None, s, curw, Some(data.wp));
        style_apply(
            server,
            &mut border_gc,
            oo,
            b"display-panes-border-style",
            Some(&mut ft),
        );
        ft.release(server);
    }
    // The border map needs the model only; build it before the screen borrow.
    let has_borders = dsx != 0 && dsy != 0;
    if has_borders {
        clear_map(&mut data.map, dsx, dsy);
        mark_borders_cell(
            &mut data.map,
            &server.layout_cells,
            root,
            osx,
            osy,
            dsx,
            dsy,
        );
        mark_pane_status_borders(&mut data.map, server, w, root, status, osx, osy, dsx, dsy);
        mark_border_joins_cell(
            &mut data.map,
            &server.layout_cells,
            root,
            osx,
            osy,
            dsx,
            dsy,
        );
    }

    let Server {
        panes, hyperlinks, ..
    } = server;
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        screen,
        &mut sink,
        ScreenWritePolicy::default(),
        hyperlinks,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.clearscreen(Colour::DEFAULT);
    for plan in &tiled {
        draw_pane(&mut ctx, panes, data, plan, scaled, has_format);
    }
    if has_borders {
        draw_map(&mut ctx, &data.map, &border_gc, dsx, dsy);
    }
    for (plan, borders) in &floating {
        clear_floating_area(&mut ctx, *borders, dsx, dsy);
        if let Some(plan) = plan {
            draw_pane(&mut ctx, panes, data, plan, scaled, has_format);
        }
        draw_floating_border(&mut ctx, &mut data.map, *borders, &border_gc, dsx, dsy);
    }
    ctx.finish();
}

fn with_screen_and_data(
    server: &mut Server,
    id: ModeId,
    f: impl FnOnce(&mut Server, &mut PanesModeData, &mut Screen),
) {
    let Some(mut data) = take_data(server, id) else {
        return;
    };
    let Some(mut screen) = mode_mut(server, id).and_then(|m| m.screen.take()) else {
        restore_data(server, id, data);
        return;
    };
    f(server, &mut data, &mut screen);
    match mode_mut(server, id) {
        Some(mode) => {
            mode.screen = Some(screen);
            mode.data = Some(data);
        }
        None => {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
            release_data(server, data);
        }
    }
}

fn release_data(server: &mut Server, mut data: Box<PanesModeData>) {
    if let Some((timer, key)) = data.timer.take() {
        server.event_loop.cancel(timer);
        server.deferred.remove(&key);
    }
    if let Some(state) = data.state.take() {
        arguments::make_commands_free(server, state);
    }
    if let Some(mut preview) = data.preview.take() {
        let _ = preview.release(
            &mut server.hyperlinks,
            #[cfg(feature = "sixel")]
            None,
        );
    }
}

fn timer_callback(server: &mut Server, id: ModeId) {
    if let Some(data) = mode_mut(server, id)
        .and_then(|m| m.data.as_mut())
        .and_then(|d| d.downcast_mut::<PanesModeData>())
    {
        data.timer = None;
        let _ = pane_reset_mode(server, id.owner);
    }
}

impl PaneModeDriver for PanesMode {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
        let wp = id.owner;
        let w = server.panes.get(wp)?.window;
        let (sx, sy) = {
            let p = server.panes.get(wp)?;
            (p.base.grid.sx(), p.base.grid.sy())
        };
        let (file, line) = {
            let command = server.queue.items.get(self.item)?.command()?;
            (command.file.clone(), command.line)
        };
        let s = self.target.s;

        let delay = if self.args.has(b'd') == 0 {
            let wo = server.windows.get(w)?.options;
            server.options.get_number(wo, b"display-panes-time") as u32
        } else {
            match self.args.strtonum(b'd', 0, i64::from(u32::MAX)) {
                Ok(n) => n as u32,
                Err(cause) => {
                    let mut message = b"delay ".to_vec();
                    message.extend_from_slice(&cause);
                    queue::error(server, self.item, &message);
                    return None;
                }
            }
        };

        let mut screen = Screen::new(
            sx,
            sy,
            0,
            ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .ok()?;
        screen.mode.remove(ScreenMode::CURSOR);

        let entry = crate::cmd::COMMAND_TABLE
            .iter()
            .find(|e| e.name == b"display-panes")?;
        let command = Command {
            entry,
            args: self.args.copy(&[], &mut 0),
            group: 0,
            file,
            line,
            parse_flags: Default::default(),
        };
        let state = arguments::make_commands_prepare(
            server,
            &command,
            self.item,
            0,
            Some(DEFAULT_TEMPLATE),
            false,
            false,
        );
        let (source_session, source_window) = if self.args.has(b's') != 0 {
            (self.source.s, self.source.w)
        } else {
            (self.target.s, self.target.w)
        };
        let Some(source_window) = source_window else {
            arguments::make_commands_free(server, state);
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
            return None;
        };
        let mut data = Box::new(PanesModeData {
            wp,
            session: s,
            source_session,
            source_window,
            preview: None,
            timer: None,
            state: Some(state),
            delay,
            ignore_keys: self.args.has(b'N') != 0,
            zoomed: None,
            areas: Vec::new(),
            map: Vec::new(),
        });

        if self.args.has(b'Z') == 0 {
            let zoomed = server
                .windows
                .get(w)
                .is_some_and(|w| w.flags.contains(WindowFlags::ZOOMED));
            data.zoomed = Some(zoomed);
            if !zoomed {
                data.preview = set_preview(server, wp);
            }
            if !zoomed {
                let mode = mode_mut(server, id)?;
                mode.data = Some(data);
                mode.screen = Some(screen);
                if window_zoom(server, w, wp).unwrap_or(false) {
                    server_redraw_window(server, w);
                }
                let mode = mode_mut(server, id)?;
                data = mode.data.take()?.downcast::<PanesModeData>().ok()?;
                screen = mode.screen.take()?;
            }
        }

        if delay != 0 {
            data.timer = Some(schedule_deferred(
                server,
                Duration::from_millis(u64::from(delay)),
                Box::new(move |server| timer_callback(server, id)),
            ));
        }

        draw_screen(server, &mut data, &mut screen);
        if let Some(p) = server.panes.get_mut(wp) {
            p.flags.insert(PaneFlags::REDRAW);
        }
        match mode_mut(server, id) {
            Some(mode) => mode.data = Some(data),
            None => {
                release_data(server, data);
                let _ = screen.release(
                    &mut server.hyperlinks,
                    #[cfg(feature = "sixel")]
                    None,
                );
                return None;
            }
        }
        Some(screen)
    }

    fn free(&self, server: &mut Server, mut mode: PaneMode) {
        let w = server.panes.get(mode.id.owner).map(|p| p.window);
        if let Some(data) = mode
            .data
            .take()
            .and_then(|d| d.downcast::<PanesModeData>().ok())
        {
            if let Some((timer, key)) = data.timer {
                server.event_loop.cancel(timer);
                server.deferred.remove(&key);
            }
            if let Some(w) = w {
                if data.zoomed == Some(false) {
                    let _ = server_unzoom_window(server, w);
                }
                server_redraw_window(server, w);
                server_redraw_window_borders(server, w);
                server_status_window(server, w);
            }
            let mut data = data;
            data.timer = None;
            release_data(server, data);
        }
        if let Some(mut screen) = mode.screen.take() {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
        }
    }

    fn resize(&self, server: &mut Server, id: ModeId, sx: u32, sy: u32) {
        with_screen_and_data(server, id, |server, data, screen| {
            screen.resize(
                sx,
                sy,
                false,
                #[cfg(feature = "sixel")]
                None,
            );
            draw_screen(server, data, screen);
        });
        if let Some(p) = server.panes.get_mut(id.owner) {
            p.flags.insert(PaneFlags::REDRAW);
        }
    }

    fn key(
        &self,
        server: &mut Server,
        id: ModeId,
        client: ClientId,
        key: KeyCode,
        mouse: Option<&ResolvedMouseEvent>,
    ) {
        let wp = id.owner;
        if key.0 == u64::from(rmux_util::key::C0::ESC) || key.0 == u64::from(b'q') {
            let _ = pane_reset_mode(server, wp);
            return;
        }
        let (ignore_keys, target) = match get_target(server, id, key, mouse) {
            Some(result) => result,
            None => return,
        };
        let Some(target) = target else {
            if !ignore_keys && !key.is_mouse() {
                let _ = pane_reset_mode(server, wp);
            }
            return;
        };
        let Some(w) = server.panes.get(wp).map(|p| p.window) else {
            return;
        };
        if server
            .windows
            .get(w)
            .is_some_and(|w| w.flags.contains(WindowFlags::ZOOMED))
        {
            let _ = window_unzoom(server, w, true);
        }
        run_command(server, id, Some(client), target);
        let _ = pane_reset_mode(server, wp);
    }

    fn append_output(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _bytes: &[u8],
    ) -> Result<(), ModelError> {
        Err(ModelError::Message(b"panes-mode has no output".to_vec()))
    }
}

/// `window_panes_run_command`: expand `%%%` to `%id` and queue the result.
fn run_command(server: &mut Server, id: ModeId, c: Option<ClientId>, target: PaneId) {
    let Some(public_id) = server.panes.get(target).map(|p| p.public_id) else {
        return;
    };
    let Some(data) = take_data(server, id) else {
        return;
    };
    let Some(state) = data.state.as_ref() else {
        restore_data(server, id, data);
        return;
    };
    let expanded = ByteString::from(format!("%{public_id}").into_bytes());
    let batch = match arguments::make_commands(server, state, &[expanded]) {
        Ok(list) => server.queue.get_command(list, None),
        Err(error) => server.queue.get_error(error.message()),
    };
    if let Ok(batch) = batch {
        let _ = queue::append(server, c, batch);
    }
    if let Some(data) = restore_data(server, id, data) {
        release_data(server, data);
    }
}

/// `window_panes_find_pane`: the last recorded area under `(x, y)`.
pub fn find_pane(areas: &[PanesArea], x: u32, y: u32) -> Option<PaneId> {
    areas
        .iter()
        .rev()
        .find(|a| x >= a.x && x < a.x + a.sx && y >= a.y && y < a.y + a.sy)
        .map(|a| a.pane)
}

/// `window_panes_key_pane`: pane index for `0`-`9` and `a`-`z`.
pub fn key_index(key: KeyCode) -> Option<u32> {
    if (u64::from(b'0')..=u64::from(b'9')).contains(&key.0) {
        Some((key.0 - u64::from(b'0')) as u32)
    } else if key.0 & KeyMasks::MODIFIERS == 0 {
        let k = key.0 & KeyMasks::KEY;
        if !(u64::from(b'a')..=u64::from(b'z')).contains(&k) {
            return None;
        }
        Some(10 + (k - u64::from(b'a')) as u32)
    } else {
        None
    }
}

/// `window_panes_get_target`: `(ignore_keys, target)`; `None` when the mode
/// data is missing.
fn get_target(
    server: &Server,
    id: ModeId,
    key: KeyCode,
    m: Option<&ResolvedMouseEvent>,
) -> Option<(bool, Option<PaneId>)> {
    let data = server
        .panes
        .get(id.owner)?
        .modes
        .iter()
        .find(|m| m.id == id)?
        .data
        .as_ref()?
        .downcast_ref::<PanesModeData>()?;
    if data.ignore_keys {
        return Some((true, None));
    }
    if key.is_mouse() {
        if key.0 != SpecialKey::MOUSEDOWN1_PANE {
            return Some((false, None));
        }
        let m = m?;
        let Some((x, y)) = crate::modes::tree::mouse_at(server, id.owner, m) else {
            return Some((false, None));
        };
        return Some((
            false,
            find_pane(&data.areas, x, y).filter(|wp| server.panes.get(*wp).is_some()),
        ));
    }
    let index = key_index(key);
    let (_, _, w) = get_source(server, data)?;
    Some((
        false,
        index.and_then(|index| pane_at_index(server, w, index)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutCellFlags, LayoutGeometry};

    fn cell(
        cells: &mut Cells,
        kind: LayoutType,
        parent: Option<LayoutCellId>,
        g: (i32, i32, u32, u32),
        floating: bool,
    ) -> LayoutCellId {
        let id = cells
            .insert(LayoutCell {
                kind,
                flags: if floating {
                    LayoutCellFlags::FLOATING
                } else {
                    LayoutCellFlags::default()
                },
                parent,
                g: LayoutGeometry {
                    xoff: g.0,
                    yoff: g.1,
                    sx: g.2,
                    sy: g.3,
                },
                fg: LayoutGeometry {
                    xoff: g.0,
                    yoff: g.1,
                    sx: g.2,
                    sy: g.3,
                },
                pane: None,
                children: Vec::new(),
            })
            .unwrap();
        if let Some(parent) = parent {
            cells.get_mut(parent).unwrap().children.push(id);
        }
        id
    }

    #[test]
    fn geometry_scales_in_c_order() {
        let mut cells = Cells::new();
        let root = cell(
            &mut cells,
            LayoutType::Leftright,
            None,
            (0, 0, 100, 50),
            false,
        );
        let a = cell(
            &mut cells,
            LayoutType::Windowpane,
            Some(root),
            (0, 0, 49, 50),
            false,
        );
        let b = cell(
            &mut cells,
            LayoutType::Windowpane,
            Some(root),
            (50, 0, 50, 50),
            false,
        );
        // Same size: one to one.
        assert_eq!(
            get_geometry(&cells, root, a, PaneStatusPosition::Off, 100, 50, 100, 50),
            Some((0, 0, 49, 50))
        );
        // Smaller target: xoff*dsx/osx.
        assert_eq!(
            get_geometry(&cells, root, b, PaneStatusPosition::Off, 100, 50, 40, 10),
            Some((20, 0, 20, 10))
        );
        assert_eq!(
            get_geometry(&cells, root, a, PaneStatusPosition::Off, 100, 50, 40, 10),
            Some((0, 0, 19, 10))
        );
        // Pane status on top takes one row from the top.
        assert_eq!(
            get_geometry(&cells, root, a, PaneStatusPosition::Top, 100, 50, 100, 50),
            Some((0, 1, 49, 49))
        );
        assert_eq!(
            get_geometry(
                &cells,
                root,
                a,
                PaneStatusPosition::Bottom,
                100,
                50,
                100,
                50
            ),
            Some((0, 0, 49, 49))
        );
        // Tiny target keeps at least one cell.
        assert_eq!(
            get_geometry(&cells, root, b, PaneStatusPosition::Off, 100, 50, 1, 1),
            Some((0, 0, 1, 1))
        );
        assert_eq!(
            get_geometry(&cells, root, a, PaneStatusPosition::Off, 100, 50, 1, 1),
            Some((0, 0, 1, 1))
        );
        assert_eq!(
            get_geometry(&cells, root, a, PaneStatusPosition::Off, 0, 50, 10, 10),
            None
        );
    }

    #[test]
    fn clip_floating_pane_inside_border() {
        assert_eq!(
            clip_floating_pane(None, 80, 24, (1, 1, 5, 5)),
            Some((1, 1, 5, 5))
        );
        // Border ring at x=4,y=2,x2=15,y2=9: pane 4..15 clips to 5..14.
        assert_eq!(
            clip_floating_pane(Some((4, 2, 15, 9)), 80, 24, (4, 2, 12, 8)),
            Some((5, 3, 10, 6))
        );
        // Border at the screen edge (-1) and beyond (>= dsx) does not clip.
        assert_eq!(
            clip_floating_pane(Some((-1, -1, 80, 24)), 80, 24, (0, 0, 80, 24)),
            Some((0, 0, 80, 24))
        );
        // Nothing left.
        assert_eq!(
            clip_floating_pane(Some((4, 2, 6, 4)), 80, 24, (4, 2, 1, 1)),
            None
        );
    }

    #[test]
    fn border_cell_type_all_masks() {
        const L: u8 = B::L;
        const R: u8 = B::R;
        const U: u8 = B::U;
        const D: u8 = B::D;
        let expect = [
            (L | R | U | D, BorderCell::Lrud),
            (L | R | U, BorderCell::Lru),
            (L | R | D, BorderCell::Lrd),
            (L | R, BorderCell::Lr),
            (L, BorderCell::Lr),
            (R, BorderCell::Lr),
            (L | U | D, BorderCell::Uld),
            (L | U, BorderCell::Lu),
            (L | D, BorderCell::Ld),
            (R | U | D, BorderCell::Urd),
            (R | U, BorderCell::Ru),
            (R | D, BorderCell::Rd),
            (U | D, BorderCell::Ud),
            (U, BorderCell::Ud),
            (D, BorderCell::Ud),
        ];
        assert_eq!(expect.len(), 15);
        for (mask, cell) in expect {
            assert_eq!(border_cell_type(mask), cell, "mask {mask}");
        }
        assert_eq!(border_cell_type(0), BorderCell::None);
    }

    #[test]
    fn border_joins_on_cross_layout() {
        // Left-right root with two top-bottom columns whose split rows align:
        // the vertical rule at x=5 meets horizontal rules at y=2 on both sides.
        let mut cells = Cells::new();
        let root = cell(
            &mut cells,
            LayoutType::Leftright,
            None,
            (0, 0, 11, 5),
            false,
        );
        let left = cell(
            &mut cells,
            LayoutType::Topbottom,
            Some(root),
            (0, 0, 5, 5),
            false,
        );
        let right = cell(
            &mut cells,
            LayoutType::Topbottom,
            Some(root),
            (6, 0, 5, 5),
            false,
        );
        cell(
            &mut cells,
            LayoutType::Windowpane,
            Some(left),
            (0, 0, 5, 2),
            false,
        );
        cell(
            &mut cells,
            LayoutType::Windowpane,
            Some(left),
            (0, 3, 5, 2),
            false,
        );
        cell(
            &mut cells,
            LayoutType::Windowpane,
            Some(right),
            (6, 0, 5, 2),
            false,
        );
        cell(
            &mut cells,
            LayoutType::Windowpane,
            Some(right),
            (6, 3, 5, 2),
            false,
        );
        let (dsx, dsy) = (11, 5);
        let mut map = vec![0u8; (dsx * dsy) as usize];
        mark_borders_cell(&mut map, &cells, root, 11, 5, dsx, dsy);
        mark_border_joins_cell(&mut map, &cells, root, 11, 5, dsx, dsy);
        let at = |x: u32, y: u32| map[(y * dsx + x) as usize];
        assert_eq!(border_cell_type(at(5, 0)), BorderCell::Ud);
        assert_eq!(border_cell_type(at(5, 2)), BorderCell::Lrud);
        assert_eq!(border_cell_type(at(4, 2)), BorderCell::Lr);
        assert_eq!(border_cell_type(at(6, 2)), BorderCell::Lr);
        assert_eq!(border_cell_type(at(0, 2)), BorderCell::Lr);
        assert_eq!(border_cell_type(at(5, 4)), BorderCell::Ud);
    }

    #[test]
    fn key_index_and_area_lookup() {
        use crate::ids::ArenaId;
        assert_eq!(key_index(KeyCode(u64::from(b'0'))), Some(0));
        assert_eq!(key_index(KeyCode(u64::from(b'9'))), Some(9));
        assert_eq!(key_index(KeyCode(u64::from(b'a'))), Some(10));
        assert_eq!(key_index(KeyCode(u64::from(b'z'))), Some(35));
        assert_eq!(key_index(KeyCode(u64::from(b'A'))), None);
        assert_eq!(
            key_index(KeyCode(
                u64::from(b'a') | rmux_util::key::KeyModifiers::META.0
            )),
            None
        );
        let p1 = PaneId::from_parts(1, 0);
        let p2 = PaneId::from_parts(2, 0);
        let areas = [
            PanesArea {
                pane: p1,
                x: 0,
                y: 0,
                sx: 10,
                sy: 10,
            },
            PanesArea {
                pane: p2,
                x: 5,
                y: 5,
                sx: 2,
                sy: 2,
            },
        ];
        assert_eq!(find_pane(&areas, 6, 6), Some(p2));
        assert_eq!(find_pane(&areas, 1, 1), Some(p1));
        assert_eq!(find_pane(&areas, 10, 1), None);
    }

    use crate::cmd::commands::break_pane::tests::{create_session, create_window, source, split};
    use crate::cmd::{CommandList, arguments::ArgsEntryFlags};
    use crate::model::pane::{pane_reset_mode, pane_set_mode};
    use std::rc::Rc;

    fn enter(flags: &[u8]) -> (Server, PaneId, WindowId, ModeId, ClientId) {
        let mut server = Server::default();
        let s = create_session(&mut server, b"s");
        let (w, wl, p) = create_window(&mut server, s, 0);
        split(&mut server, w, p);
        let current = source(&server, wl, p);
        let entry = crate::cmd::COMMAND_TABLE
            .iter()
            .find(|e| e.name == b"display-panes")
            .unwrap();
        let mut args = Args::create();
        for flag in flags {
            args.set(*flag, None, ArgsEntryFlags::default());
        }
        let command = Command {
            entry,
            args: args.copy(&[], &mut 0),
            group: 0,
            file: None,
            line: 0,
            parse_flags: Default::default(),
        };
        let list = Rc::new(CommandList {
            group: 0,
            commands: vec![command],
        });
        let batch = server.queue.get_command(list, None).unwrap();
        let item = batch.items[0];
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = current.s;
        let client = server.clients.insert(client).unwrap();
        let queued = server.queue.items.get_mut(item).unwrap();
        queued.source = current;
        queued.target = current;
        queued.client = Some(client);
        let driver = Rc::new(PanesMode::new(args, item, current, current));
        let mode = pane_set_mode(&mut server, p, NAME, FLAGS, driver, false)
            .unwrap()
            .unwrap();
        (server, p, w, mode, client)
    }

    fn data(server: &Server, mode: ModeId) -> &PanesModeData {
        server
            .panes
            .get(mode.owner)
            .unwrap()
            .modes
            .iter()
            .find(|m| m.id == mode)
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .downcast_ref()
            .unwrap()
    }

    fn press(
        server: &mut Server,
        p: PaneId,
        mode: ModeId,
        c: ClientId,
        key: KeyCode,
        mouse: Option<&ResolvedMouseEvent>,
    ) {
        let driver = server.panes.get(p).unwrap().modes[0].driver.clone();
        driver.key(server, mode, c, key, mouse);
    }

    fn in_mode(server: &Server, p: PaneId) -> bool {
        server.panes.get(p).is_some_and(|p| !p.modes.is_empty())
    }

    #[test]
    fn init_zooms_records_areas_and_selection_exits() {
        let (mut server, p, w, mode, c) = enter(&[]);
        assert!(
            server
                .windows
                .get(w)
                .unwrap()
                .flags
                .contains(WindowFlags::ZOOMED)
        );
        let d = data(&server, mode);
        assert_eq!(d.zoomed, Some(false));
        assert_eq!(d.areas.len(), 2);
        assert!(d.preview.is_some());
        assert!(d.timer.is_some());
        let before = server.queue.items.len();
        press(&mut server, p, mode, c, KeyCode(u64::from(b'1')), None);
        assert!(!in_mode(&server, p));
        assert!(
            !server
                .windows
                .get(w)
                .unwrap()
                .flags
                .contains(WindowFlags::ZOOMED)
        );
        assert!(
            server.queue.items.len() > before,
            "select-pane command appended"
        );
        assert!(server.deferred.is_empty(), "timer closure removed at free");
    }

    #[test]
    fn invalid_key_exits_but_invalid_mouse_stays() {
        let (mut server, p, _, mode, c) = enter(&[]);
        let m = ResolvedMouseEvent {
            event: Default::default(),
            target: Default::default(),
        };
        press(
            &mut server,
            p,
            mode,
            c,
            KeyCode(SpecialKey::MOUSEDOWN3_PANE),
            Some(&m),
        );
        assert!(in_mode(&server, p));
        press(&mut server, p, mode, c, KeyCode(u64::from(b'#')), None);
        assert!(!in_mode(&server, p));
    }

    #[test]
    fn ignore_keys_blocks_key_and_mouse_selection() {
        let (mut server, p, _, mode, c) = enter(b"N");
        assert!(data(&server, mode).ignore_keys);
        press(&mut server, p, mode, c, KeyCode(u64::from(b'0')), None);
        assert!(in_mode(&server, p));
        let m = ResolvedMouseEvent {
            event: Default::default(),
            target: Default::default(),
        };
        press(
            &mut server,
            p,
            mode,
            c,
            KeyCode(SpecialKey::MOUSEDOWN1_PANE),
            Some(&m),
        );
        assert!(in_mode(&server, p));
        press(&mut server, p, mode, c, KeyCode(u64::from(b'#')), None);
        assert!(in_mode(&server, p));
        press(&mut server, p, mode, c, KeyCode(u64::from(b'q')), None);
        assert!(!in_mode(&server, p));
    }

    #[test]
    fn zoom_flag_leaves_zoom_alone() {
        let (mut server, p, w, mode, _) = enter(b"Z");
        assert!(
            !server
                .windows
                .get(w)
                .unwrap()
                .flags
                .contains(WindowFlags::ZOOMED)
        );
        assert_eq!(data(&server, mode).zoomed, None);
        assert!(data(&server, mode).preview.is_none());
        pane_reset_mode(&mut server, p).unwrap();
        assert!(!in_mode(&server, p));
    }
    fn fill_screen(screen: &mut Screen, byte: u8) {
        screen.mode.remove(ScreenMode::CURSOR);
        let mut gc = DEFAULT_CELL;
        gc.data = Utf8Data::set(byte);
        for y in 0..screen.grid.sy() {
            for x in 0..screen.grid.sx() {
                screen.grid.view_set_cell(x, y, &gc);
            }
        }
    }

    #[test]
    fn draw_uses_source_window_and_status_border_map_at_scaled_sizes() {
        let mut server = Server::default();
        let s = create_session(&mut server, b"source");
        let (source_window, _, first) = create_window(&mut server, s, 0);
        let second = split(&mut server, source_window, first);
        let (_, _, target) = create_window(&mut server, s, 1);
        fill_screen(&mut server.panes.get_mut(first).unwrap().base, b'A');
        fill_screen(&mut server.panes.get_mut(second).unwrap().base, b'B');
        fill_screen(&mut server.panes.get_mut(target).unwrap().base, b'T');
        let oo = server.windows.get(source_window).unwrap().options;
        server
            .options
            .set_number_value(oo, b"pane-border-status", 1);
        let mut data = PanesModeData {
            wp: target,
            session: Some(s),
            source_session: Some(s),
            source_window,
            preview: None,
            timer: None,
            state: None,
            delay: 0,
            ignore_keys: false,
            zoomed: None,
            areas: Vec::new(),
            map: Vec::new(),
        };
        for (sx, sy) in [(40, 10), (200, 50)] {
            let mut screen = Screen::new(
                sx,
                sy,
                0,
                ScreenResetPolicy::default(),
                &mut server.hyperlinks,
            )
            .unwrap();
            draw_screen(&mut server, &mut data, &mut screen);
            assert_eq!(
                data.areas.iter().map(|area| area.pane).collect::<Vec<_>>(),
                vec![first, second]
            );
            assert!(data.areas.iter().all(|area| area.pane != target));
            assert_eq!(screen.grid.view_get_cell(0, 1).data.data[0], b'A');
            let root = server
                .windows
                .get(source_window)
                .unwrap()
                .layout_root
                .unwrap();
            let root = server.layout_cells.get(root).unwrap();
            let lc = server.panes.get(first).unwrap().layout_cell.unwrap();
            let lc = server.layout_cells.get(lc).unwrap();
            let split_y = map_y(lc.g.yoff as u32 + lc.g.sy, root.g.sy, sy) as u32;
            for y in [0, split_y] {
                let border = screen.grid.view_get_cell(0, y);
                assert_eq!(border.data.data[0], b'q');
                assert!(
                    border
                        .attr
                        .contains(rmux_emu::cell::GridAttributes::CHARSET)
                );
            }
            screen
                .release(
                    &mut server.hyperlinks,
                    #[cfg(feature = "sixel")]
                    None,
                )
                .unwrap();
        }
    }

    #[test]
    fn floating_border_corners_and_clipped_edges_use_cell_borders() {
        let mut registry = rmux_emu::hyperlinks::HyperlinkRegistry::new();
        let mut screen =
            Screen::new(12, 8, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        fill_screen(&mut screen, b'A');
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        let mut map = Vec::new();
        clear_floating_area(&mut ctx, Some((2, 1, 9, 6)), 12, 8);
        draw_floating_border(&mut ctx, &mut map, Some((2, 1, 9, 6)), &DEFAULT_CELL, 12, 8);
        ctx.finish();
        for (x, y, glyph) in [
            (2, 1, b'l'),
            (9, 1, b'k'),
            (2, 6, b'm'),
            (9, 6, b'j'),
            (5, 1, b'q'),
            (2, 3, b'x'),
        ] {
            let gc = screen.grid.view_get_cell(x, y);
            assert_eq!(gc.data.data[0], glyph);
            assert!(gc.attr.contains(rmux_emu::cell::GridAttributes::CHARSET));
        }
        assert_eq!(screen.grid.view_get_cell(3, 2).data.data[0], b' ');
        assert_eq!(screen.grid.view_get_cell(0, 0).data.data[0], b'A');
        screen
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
    }
}
