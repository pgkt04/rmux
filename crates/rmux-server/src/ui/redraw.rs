// Ported from tmux screen-redraw.c @ 8f25579c
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

use crate::client::ClientFlags;
use crate::ids::{ClientId, PaneId, WindowId};
use crate::layout::LayoutType;
use crate::model::pane::{pane_is_floating, pane_is_visible};
use crate::model::{PaneFlags, Server, Window};
use crate::options::PaneBorderIndicator;
use crate::ui::border;
use crate::ui::menu;
use crate::ui::prompt::PromptDrawData;
use crate::ui::scrollbar;
use crate::ui::status::{self, PaneStatusPosition};
use crate::ui::styles;
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{BorderCell, PaneLines, Screen, ScreenMode, ScreenResetPolicy};
use rmux_tty::draw::TtyStyleCtx;
use rmux_tty::term::TtyCodeCode;
use rmux_tty::term::tparm::TparmState;
use rmux_tty::tty::Tty;
use rmux_util::log_debug;
use rmux_util::utf8::Utf8Data;
use std::collections::VecDeque;

/// Type of span in the scene.
#[repr(usize)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedrawSpanKind {
    Pane = 0,
    Outside = 1,
    Empty = 2,
    Status = 3,
    Border = 4,
    Scrollbar = 5,
    Menu = 6,
}
pub const REDRAW_SPAN_TYPES: usize = 7;
const SPAN_KINDS: [RedrawSpanKind; REDRAW_SPAN_TYPES] = [
    RedrawSpanKind::Pane,
    RedrawSpanKind::Outside,
    RedrawSpanKind::Empty,
    RedrawSpanKind::Status,
    RedrawSpanKind::Border,
    RedrawSpanKind::Scrollbar,
    RedrawSpanKind::Menu,
];

/// Border connections to adjacent cells.
pub const REDRAW_BORDER_L: u8 = 0x1;
pub const REDRAW_BORDER_R: u8 = 0x2;
pub const REDRAW_BORDER_U: u8 = 0x4;
pub const REDRAW_BORDER_D: u8 = 0x8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BorderSpanFlags(pub u8);
impl BorderSpanFlags {
    pub const IS_ARROW: Self = Self(0x1);
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
    pub fn insert(&mut self, o: Self) {
        self.0 |= o.0;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScrollbarSpanFlags(pub u8);
impl ScrollbarSpanFlags {
    pub const LEFT: Self = Self(0x2);
    pub const RIGHT: Self = Self(0x4);
    pub const OVERLAY: Self = Self(0x8);
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
    pub fn insert(&mut self, o: Self) {
        self.0 |= o.0;
    }
}

/// Draw operations. All uses exact equality (REDRAW_IS_ALL).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RedrawOps(pub u32);
impl RedrawOps {
    pub const PANE: Self = Self(0x1);
    pub const OUTSIDE: Self = Self(0x2);
    pub const EMPTY: Self = Self(0x4);
    pub const PANE_BORDER: Self = Self(0x8);
    pub const PANE_STATUS: Self = Self(0x10);
    pub const PANE_SCROLLBAR: Self = Self(0x20);
    pub const STATUS: Self = Self(0x40);
    pub const MENU: Self = Self(0x80);
    pub const ALL: Self = Self(0x7fff_ffff);
    pub const fn is_all(self) -> bool {
        self.0 == Self::ALL.0
    }
    pub const fn intersects(self, o: Self) -> bool {
        self.0 & o.0 != 0
    }
    pub fn insert(&mut self, o: Self) {
        self.0 |= o.0;
    }
    pub fn remove(&mut self, o: Self) {
        self.0 &= !o.0;
    }
}
impl std::ops::BitOr for RedrawOps {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::fmt::Display for RedrawOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.intersects(Self::STATUS) {
            parts.push("status");
        }
        if self.intersects(Self::PANE) {
            parts.push("pane");
        }
        if self.intersects(Self::PANE_BORDER) {
            parts.push("border");
        }
        if self.intersects(Self::PANE_STATUS) {
            parts.push("pane-status");
        }
        if self.intersects(Self::PANE_SCROLLBAR) {
            parts.push("scrollbar");
        }
        if self.intersects(Self::MENU) {
            parts.push("menu");
        }
        if self.is_all() {
            parts.push("all");
        }
        f.write_str(&parts.join(" "))
    }
}

/// UTF-8 isolate characters.
pub const REDRAW_START_ISOLATE: &[u8] = b"\xe2\x81\xa6";
pub const REDRAW_END_ISOLATE: &[u8] = b"\xe2\x81\xa9";

/// Data for a span.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RedrawSpanData {
    Pane {
        wp: PaneId,
        px: u32,
        py: u32,
    },
    Outside,
    #[default]
    Empty,
    Status {
        wp: PaneId,
        offset: u32,
        cell_type: BorderCell,
    },
    Border {
        top: Option<PaneId>,
        bottom: Option<PaneId>,
        left: Option<PaneId>,
        right: Option<PaneId>,
        style_wp: Option<PaneId>,
        cell_type: BorderCell,
        cell_mask: u8,
        top_lines: PaneLines,
        bottom_lines: PaneLines,
        left_lines: PaneLines,
        right_lines: PaneLines,
        flags: BorderSpanFlags,
    },
    Scrollbar {
        wp: PaneId,
        y: u32,
        height: u32,
        flags: ScrollbarSpanFlags,
    },
    Menu {
        px: u32,
        py: u32,
    },
}

impl RedrawSpanData {
    pub const fn kind(&self) -> RedrawSpanKind {
        match self {
            Self::Pane { .. } => RedrawSpanKind::Pane,
            Self::Outside => RedrawSpanKind::Outside,
            Self::Empty => RedrawSpanKind::Empty,
            Self::Status { .. } => RedrawSpanKind::Status,
            Self::Border { .. } => RedrawSpanKind::Border,
            Self::Scrollbar { .. } => RedrawSpanKind::Scrollbar,
            Self::Menu { .. } => RedrawSpanKind::Menu,
        }
    }

    const fn empty_border() -> Self {
        Self::Border {
            top: None,
            bottom: None,
            left: None,
            right: None,
            style_wp: None,
            cell_type: BorderCell::Inside,
            cell_mask: 0,
            top_lines: PaneLines::Single,
            bottom_lines: PaneLines::Single,
            left_lines: PaneLines::Single,
            right_lines: PaneLines::Single,
            flags: BorderSpanFlags(0),
        }
    }

    /// Is the cell adjacent to this pane?
    fn has_pane(&self, wp: PaneId) -> bool {
        match self {
            Self::Border {
                top,
                bottom,
                left,
                right,
                ..
            } => *top == Some(wp) || *bottom == Some(wp) || *left == Some(wp) || *right == Some(wp),
            _ => false,
        }
    }
}

/// Scratch cell for building the scene.
pub type RedrawBuildCell = RedrawSpanData;

/// A span of cells of the same type inside a line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedrawSpan {
    pub x: u32,
    pub width: u32,
    pub data: RedrawSpanData,
}

/// A visible line on the client.
#[derive(Clone, Debug, Default)]
pub struct RedrawLine {
    spans: [Vec<RedrawSpan>; REDRAW_SPAN_TYPES],
}

/// A scene representing all the spans on the client.
#[derive(Clone, Debug)]
pub struct RedrawScene {
    pub window: WindowId,
    pub generation: u64,
    pub sx: u32,
    pub sy: u32,
    pub ox: u32,
    pub oy: u32,
    lines: Vec<RedrawLine>,
}

impl RedrawScene {
    pub fn spans(&self, y: u32, kind: RedrawSpanKind) -> &[RedrawSpan] {
        &self.lines[y as usize].spans[kind as usize]
    }
}

/// A damaged rectangle in a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedrawDamage {
    pub x: u32,
    pub y: u32,
    pub sx: u32,
    pub sy: u32,
}
pub type RedrawDamages = VecDeque<RedrawDamage>;

/// If there are more damage rectangles than this, they are collapsed.
pub const REDRAW_DAMAGE_MAX: usize = 16;

/// Context for building the scene.
struct RedrawBuildCtx {
    w: WindowId,
    wsx: u32,
    wsy: u32,
    ox: u32,
    oy: u32,
    sx: u32,
    sy: u32,
    ind: PaneBorderIndicator,
    cells: Vec<RedrawBuildCell>,
}

/// Context for redrawing.
struct RedrawDrawCtx {
    c: ClientId,
    active: Option<PaneId>,
    marked: Option<PaneId>,
    status_lines: u32,
    pane_lines: PaneLines,
    default_gc: GridCell,
    isolates: bool,
    default_set: bool,
    status_top: bool,
}

/// Current session window of a client.
fn client_window(srv: &Server, c: ClientId) -> Option<WindowId> {
    styles::client_window(srv, c)
}

/// Get window offset and expand size to cover any part outside the window.
fn redraw_get_window_offset(srv: &Server, c: ClientId) -> (u32, u32, u32, u32) {
    let Some(client) = srv.clients.get(c) else {
        return (0, 0, 0, 0);
    };
    let (_, ox, oy, mut sx, mut sy) = match &client.tty {
        Some(tty) => tty.window_offset(),
        None => (false, 0, 0, 0, 0),
    };
    let (tty_sx, tty_sy) = styles::client_size(srv, c);
    let tty_sy = tty_sy.saturating_sub(status::status_line_size(srv, c));
    if sx < tty_sx {
        sx = tty_sx;
    }
    if sy < tty_sy {
        sy = tty_sy;
    }
    (ox, oy, sx, sy)
}

impl RedrawBuildCtx {
    fn new(srv: &mut Server, c: ClientId, w: WindowId) -> Self {
        let (ox, oy, sx, sy) = redraw_get_window_offset(srv, c);
        let win = srv.windows.get(w).expect("scene window");
        let ind = PaneBorderIndicator::try_from(
            srv.options
                .get_number(win.options, b"pane-border-indicators") as i32,
        )
        .unwrap_or(PaneBorderIndicator::Colour);
        Self {
            w,
            wsx: win.sx,
            wsy: win.sy,
            ox,
            oy,
            sx,
            sy,
            ind,
            cells: std::mem::take(&mut srv.redraw_cells),
        }
    }

    fn cell(&mut self, x: u32, y: u32) -> &mut RedrawBuildCell {
        &mut self.cells[(y as usize * self.sx as usize) + x as usize]
    }

    fn cell_ref(&self, x: u32, y: u32) -> &RedrawBuildCell {
        &self.cells[(y as usize * self.sx as usize) + x as usize]
    }

    /// Reset cell to either empty or outside the window.
    fn reset_cell(&mut self, x: u32, y: u32) {
        let inside = self.ox + x < self.wsx && self.oy + y < self.wsy;
        *self.cell(x, y) = if inside {
            RedrawSpanData::Empty
        } else {
            RedrawSpanData::Outside
        };
    }

    /// Convert window position to scene position.
    fn window_to_scene(&self, wx: i32, wy: i32) -> Option<(u32, u32)> {
        if wx < 0 || wy < 0 {
            return None;
        }
        if wx as u32 > self.wsx || wy as u32 > self.wsy {
            return None;
        }
        if wx < self.ox as i32 || wy < self.oy as i32 {
            return None;
        }
        let sx = wx - self.ox as i32;
        let sy = wy - self.oy as i32;
        if sx as u32 >= self.sx || sy as u32 >= self.sy {
            return None;
        }
        Some((sx as u32, sy as u32))
    }
}

struct PaneGeom {
    xoff: i32,
    yoff: i32,
    sx: u32,
    sy: u32,
    floating: bool,
}

fn pane_geom(srv: &Server, wp: PaneId) -> Option<PaneGeom> {
    let p = srv.panes.get(wp)?;
    Some(PaneGeom {
        xoff: p.xoff,
        yoff: p.yoff,
        sx: p.sx,
        sy: p.sy,
        floating: pane_is_floating(srv, wp),
    })
}

/// Convert pane position to scene position. A floating pane is clipped to
/// the window edge.
fn redraw_pane_to_scene(
    bctx: &RedrawBuildCtx,
    g: &PaneGeom,
    px: i32,
    py: i32,
) -> Option<(u32, u32)> {
    let wx = g.xoff + px;
    let wy = g.yoff + py;
    if g.floating {
        let left = g.xoff - 1;
        let right = g.xoff + g.sx as i32;
        let top = g.yoff - 1;
        let bottom = g.yoff + g.sy as i32;
        if left < 0 && wx < 0 {
            return None;
        }
        if right > bctx.wsx as i32 && wx >= bctx.wsx as i32 {
            return None;
        }
        if top < 0 && wy < 0 {
            return None;
        }
        if bottom > bctx.wsy as i32 && wy >= bctx.wsy as i32 {
            return None;
        }
    }
    bctx.window_to_scene(wx, wy)
}

/// Convert redraw border mask to a border cell type.
pub fn redraw_get_cell_type(mask: u8) -> BorderCell {
    const L: u8 = REDRAW_BORDER_L;
    const R: u8 = REDRAW_BORDER_R;
    const U: u8 = REDRAW_BORDER_U;
    const D: u8 = REDRAW_BORDER_D;
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

/// Return if there are two panes for the border colour indicator.
fn redraw_check_two_pane_colours(srv: &Server, w: WindowId) -> Option<LayoutType> {
    let win = srv.windows.get(w)?;
    let mut count = 0;
    let mut kind = None;
    for wp in &win.panes {
        let Some(p) = srv.panes.get(*wp) else {
            continue;
        };
        let Some(lc) = p.layout_cell else {
            continue;
        };
        if pane_is_floating(srv, *wp) {
            continue;
        }
        count += 1;
        let parent = srv.layout_cells.get(lc).and_then(|lc| lc.parent);
        let parent = parent?;
        if count > 2 {
            return None;
        }
        kind = srv.layout_cells.get(parent).map(|p| p.kind);
    }
    if count == 2 { kind } else { None }
}

/// Mark pane inside data.
fn redraw_mark_pane_inside(bctx: &mut RedrawBuildCtx, wp: PaneId, g: &PaneGeom) {
    for py in 0..g.sy {
        for px in 0..g.sx {
            let Some((x, y)) = redraw_pane_to_scene(bctx, g, px as i32, py as i32) else {
                continue;
            };
            *bctx.cell(x, y) = RedrawSpanData::Pane { wp, px, py };
        }
    }
}

/// Mark scrollbar data.
fn redraw_mark_pane_scrollbar(
    bctx: &mut RedrawBuildCtx,
    wp: PaneId,
    g: &PaneGeom,
    sb_w: i32,
    sb_left: bool,
    overlay: bool,
) {
    if sb_w == 0 {
        return;
    }
    let (sx, ex) = if overlay && sb_left {
        (g.xoff, g.xoff + sb_w - 1)
    } else if overlay {
        let ex = g.xoff + g.sx as i32 - 1;
        (ex - sb_w + 1, ex)
    } else if sb_left {
        (g.xoff - sb_w, g.xoff - 1)
    } else {
        let sx = g.xoff + g.sx as i32;
        (sx, sx + sb_w - 1)
    };
    let mut flags = ScrollbarSpanFlags(0);
    flags.insert(if sb_left {
        ScrollbarSpanFlags::LEFT
    } else {
        ScrollbarSpanFlags::RIGHT
    });
    if overlay {
        flags.insert(ScrollbarSpanFlags::OVERLAY);
    }
    for sy in 0..g.sy {
        let wy = g.yoff + sy as i32;
        for wx in sx..=ex {
            let Some((x, y)) = bctx.window_to_scene(wx, wy) else {
                continue;
            };
            *bctx.cell(x, y) = RedrawSpanData::Scrollbar {
                wp,
                y: sy,
                height: g.sy,
                flags,
            };
        }
    }
}

/// Mark one border cell.
#[allow(clippy::too_many_arguments)]
fn redraw_mark_border_cell(
    bctx: &mut RedrawBuildCtx,
    wx: i32,
    wy: i32,
    wp: PaneId,
    g: &PaneGeom,
    top_owner: bool,
    bottom_owner: bool,
    mut mask: u8,
    pane_lines: PaneLines,
    floating: bool,
) {
    let Some((x, y)) = bctx.window_to_scene(wx, wy) else {
        return;
    };
    let bc = bctx.cell(x, y);
    let reset = if !floating {
        match bc.kind() {
            RedrawSpanKind::Empty | RedrawSpanKind::Outside => true,
            RedrawSpanKind::Border => false,
            _ => return,
        }
    } else {
        bc.kind() != RedrawSpanKind::Border || !bc.has_pane(wp)
    };
    if reset {
        *bc = RedrawSpanData::empty_border();
    }
    let RedrawSpanData::Border {
        top,
        bottom,
        left,
        right,
        cell_type,
        cell_mask,
        top_lines,
        bottom_lines,
        left_lines,
        right_lines,
        ..
    } = bc
    else {
        return;
    };
    if top_owner {
        *top = Some(wp);
        *top_lines = pane_lines;
    }
    if bottom_owner {
        *bottom = Some(wp);
        *bottom_lines = pane_lines;
    }
    if mask & (REDRAW_BORDER_U | REDRAW_BORDER_D) != 0 {
        if wx < g.xoff {
            *right = Some(wp);
            *right_lines = pane_lines;
        } else if wx >= g.xoff + g.sx as i32 {
            *left = Some(wp);
            *left_lines = pane_lines;
        }
    }
    mask |= *cell_mask;
    *cell_mask = mask;
    *cell_type = redraw_get_cell_type(mask);
}

/// Mark border cells for a pane status line, keeping the border cell type.
fn redraw_mark_border_status(
    bctx: &mut RedrawBuildCtx,
    wp: PaneId,
    g: &PaneGeom,
    pane_status: PaneStatusPosition,
    right: i32,
    top: i32,
    bottom: i32,
) {
    if pane_status == PaneStatusPosition::Off {
        return;
    }
    let wy = if pane_status == PaneStatusPosition::Top {
        top
    } else {
        bottom
    };
    let sx = g.xoff + 2;
    let ex = right - 1;
    if sx > ex {
        return;
    }
    let mut off = 0;
    for wx in sx..=ex {
        let Some((x, y)) = bctx.window_to_scene(wx, wy) else {
            off += 1;
            continue;
        };
        let bc = bctx.cell(x, y);
        if let RedrawSpanData::Border { cell_type, .. } = *bc {
            *bc = RedrawSpanData::Status {
                wp,
                offset: off,
                cell_type,
            };
        }
        off += 1;
    }
}

fn mark_arrow(bctx: &mut RedrawBuildCtx, wx: i32, wy: i32) {
    if let Some((x, y)) = bctx.window_to_scene(wx, wy) {
        if let RedrawSpanData::Border { flags, .. } = bctx.cell(x, y) {
            flags.insert(BorderSpanFlags::IS_ARROW);
        }
    }
}

/// Mark existing border cells where indicator arrows will be drawn.
fn redraw_mark_border_arrows(
    bctx: &mut RedrawBuildCtx,
    g: &PaneGeom,
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
) {
    if bctx.ind != PaneBorderIndicator::Arrows && bctx.ind != PaneBorderIndicator::Both {
        return;
    }
    let wx = g.xoff + 1;
    if wx >= left && wx <= right {
        mark_arrow(bctx, wx, top);
        mark_arrow(bctx, wx, bottom);
    }
    let wy = g.yoff + 1;
    if wy >= top && wy <= bottom {
        mark_arrow(bctx, left, wy);
        mark_arrow(bctx, right, wy);
    }
}

/// Mark pane borders.
fn redraw_mark_pane_borders(
    srv: &Server,
    bctx: &mut RedrawBuildCtx,
    wp: PaneId,
    g: &PaneGeom,
    sb_w: i32,
    sb_left: bool,
) {
    let pane_lines = border::pane_lines_of(srv, wp);
    let floating = g.floating;
    if floating && pane_lines == PaneLines::None {
        return;
    }
    let pane_status = border::pane_status_of(srv, wp);

    let mut left = g.xoff - 1;
    let mut right = g.xoff + g.sx as i32;
    if sb_w != 0 {
        if sb_left {
            left -= sb_w;
        } else {
            right += sb_w;
        }
    }
    let mut top = g.yoff - 1;
    let mut bottom = g.yoff + g.sy as i32;

    let mark_left = left >= 0;
    let mut mark_top = top >= 0;
    let mark_right;
    let mut mark_bottom;
    let wsx = bctx.wsx as i32;
    let wsy = bctx.wsy as i32;

    if floating {
        mark_right = right < wsx;
        mark_bottom = bottom < wsy;
        if left < 0 {
            left = 0;
        }
        if right >= wsx {
            right = wsx - 1;
        }
        if top < 0 {
            top = 0;
        }
        if bottom >= wsy {
            bottom = wsy - 1;
        }
    } else {
        mark_right = right <= wsx;
        mark_bottom = bottom <= wsy;
        if pane_status == PaneStatusPosition::Top && bottom < wsy {
            mark_bottom = false;
        } else if pane_status == PaneStatusPosition::Bottom {
            mark_top = false;
        }
    }

    if mark_top {
        for wx in left..=right {
            let mut mask = 0;
            if wx > left {
                mask |= REDRAW_BORDER_L;
            }
            if wx < right {
                mask |= REDRAW_BORDER_R;
            }
            redraw_mark_border_cell(
                bctx, wx, top, wp, g, false, true, mask, pane_lines, floating,
            );
        }
    }
    if mark_bottom {
        for wx in left..=right {
            let mut mask = 0;
            if wx > left {
                mask |= REDRAW_BORDER_L;
            }
            if wx < right {
                mask |= REDRAW_BORDER_R;
            }
            redraw_mark_border_cell(
                bctx, wx, bottom, wp, g, true, false, mask, pane_lines, floating,
            );
        }
    }
    if mark_left {
        for wy in top..=bottom {
            let mut mask = 0;
            if wy > top {
                mask |= REDRAW_BORDER_U;
            }
            if wy < bottom {
                mask |= REDRAW_BORDER_D;
            }
            redraw_mark_border_cell(
                bctx, left, wy, wp, g, false, false, mask, pane_lines, floating,
            );
        }
    }
    if mark_right {
        for wy in top..=bottom {
            let mut mask = 0;
            if wy > top {
                mask |= REDRAW_BORDER_U;
            }
            if wy < bottom {
                mask |= REDRAW_BORDER_D;
            }
            redraw_mark_border_cell(
                bctx, right, wy, wp, g, false, false, mask, pane_lines, floating,
            );
        }
    }

    redraw_mark_border_status(bctx, wp, g, pane_status, right, top, bottom);
    redraw_mark_border_arrows(bctx, g, left, right, top, bottom);
}

/// Mark an entire pane in the build grid.
fn redraw_mark_pane(srv: &Server, bctx: &mut RedrawBuildCtx, wp: PaneId) {
    if !pane_is_visible(srv, wp) {
        return;
    }
    let Some(g) = pane_geom(srv, wp) else {
        return;
    };
    let p = srv.panes.get(wp).expect("pane");
    let mut sb_w = 0;
    let mut overlay = false;
    if crate::model::pane::pane_scrollbar_visible(srv, wp) {
        overlay = crate::model::pane::pane_scrollbar_overlay(srv, wp);
        if overlay {
            sb_w = p.scrollbar_style.width + p.scrollbar_style.pad;
            if sb_w > g.sx as i32 {
                sb_w = p.scrollbar_style.width;
                if sb_w > g.sx as i32 {
                    sb_w = g.sx as i32;
                }
            }
        } else {
            sb_w = p.scrollbar_style.width + p.scrollbar_style.pad;
        }
    }
    let sb_left = sb_w != 0
        && srv
            .windows
            .get(bctx.w)
            .is_some_and(|w| w.sb_pos == crate::ui::scrollbar::PaneScrollbarPosition::Left);

    redraw_mark_pane_inside(bctx, wp, &g);
    redraw_mark_pane_borders(srv, bctx, wp, &g, if overlay { 0 } else { sb_w }, sb_left);
    redraw_mark_pane_scrollbar(bctx, wp, &g, sb_w, sb_left, overlay);
}

/// Choose the pane that will provide the border style for two-pane layouts.
fn redraw_mark_two_pane_colours(srv: &Server, bctx: &mut RedrawBuildCtx) {
    if bctx.ind != PaneBorderIndicator::Colour && bctx.ind != PaneBorderIndicator::Both {
        return;
    }
    let Some(kind) = redraw_check_two_pane_colours(srv, bctx.w) else {
        return;
    };
    let (wsx, wsy, ox, oy) = (bctx.wsx, bctx.wsy, bctx.ox, bctx.oy);
    for y in 0..bctx.sy {
        for x in 0..bctx.sx {
            let wx = ox + x;
            let wy = oy + y;
            let RedrawSpanData::Border {
                top,
                bottom,
                left,
                right,
                style_wp,
                ..
            } = bctx.cell(x, y)
            else {
                continue;
            };
            if kind == LayoutType::Leftright && left.is_some() && right.is_some() {
                *style_wp = if wy <= wsy / 2 { *left } else { *right };
            } else if kind == LayoutType::Topbottom && top.is_some() && bottom.is_some() {
                *style_wp = if wx <= wsx / 2 { *top } else { *bottom };
            }
        }
    }
}

/// Mark the window menu above all panes.
fn redraw_mark_menu(srv: &Server, bctx: &mut RedrawBuildCtx) {
    let Some(md) = srv.windows.get(bctx.w).and_then(|w| w.menu.as_ref()) else {
        return;
    };
    let (mx, my, sx, sy) = (md.x(), md.y(), md.width(), md.height());
    for py in 0..sy {
        for px in 0..sx {
            let Some((x, y)) = bctx.window_to_scene((mx + px) as i32, (my + py) as i32) else {
                continue;
            };
            *bctx.cell(x, y) = RedrawSpanData::Menu { px, py };
        }
    }
}

/// Return true if two adjacent build cells can be joined into one span.
pub fn redraw_compare_data(a: &RedrawBuildCell, b: &RedrawBuildCell) -> bool {
    use RedrawSpanData as D;
    match (a, b) {
        (
            D::Pane {
                wp: aw,
                px: apx,
                py: apy,
            },
            D::Pane {
                wp: bw,
                px: bpx,
                py: bpy,
            },
        ) => aw == bw && apy == bpy && apx + 1 == *bpx,
        (D::Border { flags, .. }, D::Border { .. }) => {
            a == b && !flags.contains(BorderSpanFlags::IS_ARROW)
        }
        (
            D::Status {
                wp: aw,
                offset: ao,
                cell_type: at,
            },
            D::Status {
                wp: bw,
                offset: bo,
                cell_type: bt,
            },
        ) => aw == bw && ao + 1 == *bo && at == bt,
        (D::Scrollbar { .. }, D::Scrollbar { .. }) => a == b,
        (D::Menu { px: apx, py: apy }, D::Menu { px: bpx, py: bpy }) => {
            apy == bpy && apx + 1 == *bpx
        }
        (D::Outside, D::Outside) | (D::Empty, D::Empty) => true,
        _ => false,
    }
}

/// Build the temporary cells for a redraw scene.
fn redraw_build_cells(srv: &Server, bctx: &mut RedrawBuildCtx) {
    let ncells = (bctx.sx as usize)
        .checked_mul(bctx.sy as usize)
        .unwrap_or_else(|| rmux_util::fatalx!("redraw_build_cells: too many cells"));
    if ncells > bctx.cells.len() {
        bctx.cells.resize(ncells, RedrawSpanData::Empty);
    }
    for y in 0..bctx.sy {
        for x in 0..bctx.sx {
            bctx.reset_cell(x, y);
        }
    }
    let z_order: Vec<PaneId> = srv
        .windows
        .get(bctx.w)
        .map(|w| w.z_order.clone())
        .unwrap_or_default();
    // z_order is front first; C walks the z index in reverse (back to front).
    for wp in z_order.iter().rev() {
        redraw_mark_pane(srv, bctx, *wp);
    }
    redraw_mark_two_pane_colours(srv, bctx);
    redraw_mark_menu(srv, bctx);
}

/// Build and return a redraw scene for a client.
fn redraw_make_scene(srv: &mut Server, c: ClientId, w: WindowId) -> Option<RedrawScene> {
    if srv
        .clients
        .get(c)
        .is_none_or(|cl| cl.flags.contains(ClientFlags::SUSPENDED))
    {
        return None;
    }
    let mut bctx = RedrawBuildCtx::new(srv, c, w);
    let generation = srv.windows.get(w)?.redraw_scene_generation;
    log_debug!(
        "{}: building @{} scene ({}x{} {},{}; generation {})",
        client_name(srv, c),
        srv.windows.get(w).map_or(0, |w| w.public_id),
        bctx.sx,
        bctx.sy,
        bctx.ox,
        bctx.oy,
        generation
    );
    redraw_build_cells(srv, &mut bctx);

    let mut lines: Vec<RedrawLine> = Vec::with_capacity(bctx.sy as usize);
    for y in 0..bctx.sy {
        let mut line = RedrawLine::default();
        let mut x = 0;
        while x < bctx.sx {
            let x0 = x;
            let mut last = *bctx.cell_ref(x, y);
            x += 1;
            while x < bctx.sx {
                let bc = bctx.cell_ref(x, y);
                if !redraw_compare_data(&last, bc) {
                    break;
                }
                last = *bc;
                x += 1;
            }
            let data = *bctx.cell_ref(x0, y);
            line.spans[data.kind() as usize].push(RedrawSpan {
                x: x0,
                width: x - x0,
                data,
            });
        }
        lines.push(line);
    }
    srv.redraw_cells = bctx.cells;
    log_debug!("{}: finished building scene", client_name(srv, c));
    Some(RedrawScene {
        window: w,
        generation,
        sx: bctx.sx,
        sy: bctx.sy,
        ox: bctx.ox,
        oy: bctx.oy,
        lines,
    })
}

fn client_name(srv: &Server, c: ClientId) -> String {
    srv.clients
        .get(c)
        .and_then(|c| c.name.as_ref())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_default()
}

/// Does a client's cached scene show this window?
pub fn redraw_client_has_window(srv: &Server, c: ClientId, w: WindowId) -> bool {
    srv.clients
        .get(c)
        .and_then(|c| c.redraw_scene.as_ref())
        .is_some_and(|s| s.window == w)
}

/// Mark a window's cached redraw scenes as out of date.
pub fn redraw_invalidate_scene(w: &mut Window) {
    w.redraw_scene_generation = w.redraw_scene_generation.wrapping_add(1);
}

/// Free all pending damage for a window.
pub fn redraw_free_damage(w: &mut Window) {
    w.damage.clear();
}

/// Collapse all pending damage for a window into one rectangle.
fn redraw_collapse_damage(damage: &mut RedrawDamages) {
    let Some(first) = damage.front().copied() else {
        return;
    };
    let mut x0 = first.x;
    let mut y0 = first.y;
    let mut x1 = first.x + first.sx;
    let mut y1 = first.y + first.sy;
    for rd in damage.iter() {
        x0 = x0.min(rd.x);
        y0 = y0.min(rd.y);
        x1 = x1.max(rd.x + rd.sx);
        y1 = y1.max(rd.y + rd.sy);
    }
    damage.clear();
    damage.push_back(RedrawDamage {
        x: x0,
        y: y0,
        sx: x1 - x0,
        sy: y1 - y0,
    });
}

/// Record window damage, merging nearby rectangles and limiting the count.
pub fn redraw_damage_window(w: &mut Window, x: u32, y: u32, mut sx: u32, mut sy: u32) {
    if x >= w.sx || y >= w.sy {
        return;
    }
    if x + sx > w.sx {
        sx = w.sx - x;
    }
    if y + sy > w.sy {
        sy = w.sy - y;
    }
    if sx == 0 || sy == 0 {
        return;
    }
    for rd in w.damage.iter_mut() {
        if x > rd.x + rd.sx || rd.x > x + sx || y > rd.y + rd.sy || rd.y > y + sy {
            continue;
        }
        let x0 = x.min(rd.x);
        let y0 = y.min(rd.y);
        let x1 = (x + sx).max(rd.x + rd.sx);
        let y1 = (y + sy).max(rd.y + rd.sy);
        let area = sx * sy + rd.sx * rd.sy;
        let union_area = (x1 - x0) * (y1 - y0);
        if union_area > 2 * area {
            continue;
        }
        rd.x = x0;
        rd.y = y0;
        rd.sx = x1 - x0;
        rd.sy = y1 - y0;
        return;
    }
    w.damage.push_back(RedrawDamage { x, y, sx, sy });
    if w.damage.len() > REDRAW_DAMAGE_MAX {
        redraw_collapse_damage(&mut w.damage);
    }
}

/// Mark all cached redraw scenes as out of date.
pub fn redraw_invalidate_all_scenes(srv: &mut Server) {
    let ids: Vec<WindowId> = srv.window_ids.values().copied().collect();
    for w in ids {
        if let Some(win) = srv.windows.get_mut(w) {
            redraw_invalidate_scene(win);
        }
    }
}

/// Get the cached redraw scene, rebuilding it if needed. The scene is taken
/// out of the client; the caller puts it back with `put_scene`.
fn redraw_get_scene(srv: &mut Server, c: ClientId) -> Option<RedrawScene> {
    let w = client_window(srv, c)?;
    let (ox, oy, sx, sy) = redraw_get_window_offset(srv, c);
    let generation = srv.windows.get(w)?.redraw_scene_generation;
    let scene = srv.clients.get_mut(c)?.redraw_scene.take();
    let reason = match &scene {
        None => Some("missing"),
        Some(s) if s.window != w => Some("window changed"),
        Some(s) if s.generation != generation => Some("generation changed"),
        Some(s) if s.ox != ox || s.oy != oy => Some("offset changed"),
        Some(s) if s.sx != sx || s.sy != sy => Some("size changed"),
        Some(_) => None,
    };
    match reason {
        Some(reason) => {
            log_debug!("{}: scene invalid: {}", client_name(srv, c), reason);
            drop(scene);
            redraw_make_scene(srv, c, w)
        }
        None => scene,
    }
}

fn put_scene(srv: &mut Server, c: ClientId, scene: RedrawScene) {
    if let Some(client) = srv.clients.get_mut(c) {
        client.redraw_scene = Some(scene);
    }
}

/// Per-draw terminal handle taken out of the client and server so that the
/// model stays borrowable while drawing.
struct DrawTarget {
    tty: Tty,
    tparm: TparmState,
}

impl DrawTarget {
    fn take(srv: &mut Server, c: ClientId) -> Option<Self> {
        let tty = srv.clients.get_mut(c)?.tty.take()?;
        let tparm = std::mem::take(&mut srv.tparm);
        Some(Self { tty, tparm })
    }
    fn restore(self, srv: &mut Server, c: ClientId) {
        srv.tparm = self.tparm;
        if let Some(client) = srv.clients.get_mut(c) {
            client.tty = Some(self.tty);
        }
    }
}

/// Draw a pane span.
fn redraw_draw_pane_span(
    srv: &mut Server,
    t: &mut DrawTarget,
    span: &RedrawSpan,
    x: u32,
    y: u32,
    n: u32,
) {
    let RedrawSpanData::Pane { wp, px, py } = span.data else {
        return;
    };
    let (defaults, dim) = crate::ui::fanout::tty_default_colours(srv, wp);
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    let s = p.screen();
    let style_ctx = TtyStyleCtx {
        defaults: &defaults,
        palette: Some(&p.palette),
        dim,
        hyperlinks: s.hyperlinks.as_ref().map(|h| (&srv.hyperlinks, h)),
    };
    let px = px + (x - span.x);
    t.tty.draw_line(
        &mut t.tparm,
        &srv.hyperlinks,
        s,
        px,
        py,
        n,
        x,
        y,
        Some(&style_ctx),
    );
}

/// Get default border style for spans without a pane.
fn redraw_get_default_border_style(
    srv: &mut Server,
    dctx: &mut RedrawDrawCtx,
    w: WindowId,
) -> (GridCell, PaneLines) {
    if !dctx.default_set {
        let session = srv.clients.get(dctx.c).and_then(|c| c.session);
        let curw = styles::client_winlink(srv, dctx.c);
        let oo = srv.windows.get(w).map(|w| w.options);
        let mut ft = styles::create_defaults(srv, None, Some(dctx.c), session, curw, None);
        let mut gc = DEFAULT_CELL;
        if let Some(oo) = oo {
            styles::style_add(srv, &mut gc, oo, b"pane-border-style", Some(&mut ft));
            dctx.pane_lines =
                PaneLines::try_from(srv.options.get_number(oo, b"pane-border-lines") as i32)
                    .unwrap_or(PaneLines::Single);
        }
        ft.release(srv);
        dctx.default_gc = gc;
        dctx.default_set = true;
    }
    (dctx.default_gc, dctx.pane_lines)
}

/// For this border span, pick the pane whose border style should colour it.
fn redraw_get_pane_for_border_style(dctx: &RedrawDrawCtx, span: &RedrawSpan) -> Option<PaneId> {
    let RedrawSpanData::Border {
        top,
        bottom,
        left,
        right,
        style_wp,
        ..
    } = &span.data
    else {
        return None;
    };
    if style_wp.is_some() {
        return *style_wp;
    }
    if let Some(active) = dctx.active {
        if span.data.has_pane(active) {
            return Some(active);
        }
    }
    top.or(*bottom).or(*left).or(*right)
}

/// Draw arrow indicator if this border span is an arrow cell.
fn redraw_draw_border_arrow(dctx: &RedrawDrawCtx, span: &RedrawSpan, gc: &mut GridCell) {
    let Some(active) = dctx.active else {
        return;
    };
    let RedrawSpanData::Border {
        top,
        bottom,
        left,
        right,
        flags,
        ..
    } = &span.data
    else {
        return;
    };
    if !flags.contains(BorderSpanFlags::IS_ARROW) {
        return;
    }
    let ch = if *left == Some(active) {
        b','
    } else if *right == Some(active) {
        b'+'
    } else if *top == Some(active) {
        b'-'
    } else if *bottom == Some(active) {
        b'.'
    } else {
        return;
    };
    gc.data = Utf8Data::set(ch);
    gc.attr.insert(GridAttributes::CHARSET);
}

/// Draw a border span.
#[allow(clippy::too_many_arguments)]
fn redraw_draw_border_span(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &mut RedrawDrawCtx,
    w: WindowId,
    span: &RedrawSpan,
    x: u32,
    y: u32,
    n: u32,
) {
    let (wp, cell_type) = match &span.data {
        RedrawSpanData::Border { cell_type, .. } => {
            (redraw_get_pane_for_border_style(dctx, span), *cell_type)
        }
        _ => (None, BorderCell::None),
    };
    let mut gc;
    match wp {
        None => {
            let (dgc, mut pane_lines) = redraw_get_default_border_style(srv, dctx, w);
            gc = dgc;
            match span.data.kind() {
                RedrawSpanKind::Outside => {
                    if let Some(win) = srv.windows.get(w) {
                        border::window_get_fill_cell(win, false, &mut gc);
                    }
                }
                RedrawSpanKind::Empty => {
                    if let Some(win) = srv.windows.get(w) {
                        border::window_get_fill_cell(win, true, &mut gc);
                    }
                }
                kind => {
                    if kind != RedrawSpanKind::Border {
                        pane_lines = PaneLines::Single;
                    }
                    border::window_get_border_cell(None, pane_lines, cell_type, &mut gc);
                }
            }
        }
        Some(wp) => {
            gc = border::window_pane_get_border_style(srv, wp, dctx.c);
            border::window_pane_get_border_cell(srv, wp, cell_type, &mut gc);
        }
    }
    if span.data.kind() == RedrawSpanKind::Border {
        if let Some(marked) = dctx.marked {
            if span.data.has_pane(marked) {
                gc.attr.0 ^= GridAttributes::REVERSE.0;
            }
        }
    }
    redraw_draw_border_arrow(dctx, span, &mut gc);

    let isolates = cell_type == BorderCell::Ud && dctx.isolates;
    t.tty.cursor(&mut t.tparm, x, y);
    if isolates {
        t.tty.puts(REDRAW_END_ISOLATE);
    }
    for _ in 0..n {
        t.tty.cell(&mut t.tparm, &gc, None);
    }
    if isolates {
        t.tty.puts(REDRAW_START_ISOLATE);
    }
}

/// Draw a pane status span.
fn redraw_draw_status_span(
    srv: &Server,
    t: &mut DrawTarget,
    span: &RedrawSpan,
    x: u32,
    y: u32,
    mut n: u32,
) {
    let RedrawSpanData::Status { wp, offset, .. } = span.data else {
        return;
    };
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    let s = &p.status_screen;
    let sx = s.grid.sx();
    let px = offset + (x - span.x);
    if px < sx {
        if n > sx - px {
            n = sx - px;
        }
        t.tty
            .draw_line(&mut t.tparm, &srv.hyperlinks, s, px, 0, n, x, y, None);
    }
}

/// Draw a menu span.
fn redraw_draw_menu_span(
    srv: &Server,
    t: &mut DrawTarget,
    w: WindowId,
    span: &RedrawSpan,
    x: u32,
    y: u32,
    n: u32,
) {
    let RedrawSpanData::Menu { px, py } = span.data else {
        return;
    };
    let Some(md) = srv.windows.get(w).and_then(|w| w.menu.as_ref()) else {
        return;
    };
    let s = menu::menu_screen(md);
    let px = px + (x - span.x);
    t.tty
        .draw_line(&mut t.tparm, &srv.hyperlinks, s, px, py, n, x, y, None);
}

/// Draw a span.
fn redraw_draw_span(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &mut RedrawDrawCtx,
    w: WindowId,
    span: &RedrawSpan,
    y: u32,
) {
    if let RedrawSpanData::Status { wp, .. } = span.data {
        if !srv
            .panes
            .get(wp)
            .is_some_and(|p| p.flags.contains(PaneFlags::NEWSTATUS))
        {
            return;
        }
    }
    match span.data.kind() {
        RedrawSpanKind::Pane => redraw_draw_pane_span(srv, t, span, span.x, y, span.width),
        RedrawSpanKind::Border | RedrawSpanKind::Empty | RedrawSpanKind::Outside => {
            redraw_draw_border_span(srv, t, dctx, w, span, span.x, y, span.width)
        }
        RedrawSpanKind::Status => redraw_draw_status_span(srv, t, span, span.x, y, span.width),
        RedrawSpanKind::Scrollbar => scrollbar::redraw_draw_scrollbar_span(
            srv,
            &mut t.tty,
            &mut t.tparm,
            span,
            span.x,
            y,
            span.width,
        ),
        RedrawSpanKind::Menu => redraw_draw_menu_span(srv, t, w, span, span.x, y, span.width),
    }
}

fn client_row(dctx: &RedrawDrawCtx, y: u32) -> u32 {
    if dctx.status_top {
        dctx.status_lines + y
    } else {
        y
    }
}

/// Draw pane lines.
fn redraw_draw_pane_lines(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &mut RedrawDrawCtx,
    scene: &RedrawScene,
    wp: PaneId,
    flags: RedrawOps,
) {
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    let mut top = p.yoff - scene.oy as i32;
    if top < 0 {
        top = 0;
    }
    let mut bottom = p.yoff + p.sy as i32 - scene.oy as i32;
    if bottom < 0 {
        bottom = 0;
    }
    if bottom > scene.sy as i32 {
        bottom = scene.sy as i32;
    }
    for y in top..bottom {
        let y = y as u32;
        let cy = client_row(dctx, y);
        if flags.intersects(RedrawOps::PANE) {
            for span in scene.spans(y, RedrawSpanKind::Pane) {
                if matches!(span.data, RedrawSpanData::Pane { wp: w, .. } if w == wp) {
                    redraw_draw_span(srv, t, dctx, scene.window, span, cy);
                }
            }
        }
        if flags.intersects(RedrawOps::PANE_SCROLLBAR) {
            for span in scene.spans(y, RedrawSpanKind::Scrollbar) {
                if matches!(span.data, RedrawSpanData::Scrollbar { wp: w, .. } if w == wp) {
                    redraw_draw_span(srv, t, dctx, scene.window, span, cy);
                }
            }
        }
    }
}

fn kind_allowed(flags: RedrawOps, kind: RedrawSpanKind) -> bool {
    if flags.is_all() {
        return true;
    }
    let op = match kind {
        RedrawSpanKind::Pane => RedrawOps::PANE,
        RedrawSpanKind::Outside => RedrawOps::OUTSIDE,
        RedrawSpanKind::Empty => RedrawOps::EMPTY,
        RedrawSpanKind::Border => RedrawOps::PANE_BORDER,
        RedrawSpanKind::Status => RedrawOps::PANE_STATUS,
        RedrawSpanKind::Scrollbar => RedrawOps::PANE_SCROLLBAR,
        RedrawSpanKind::Menu => RedrawOps::MENU,
    };
    flags.intersects(op)
}

/// Draw lines.
fn redraw_draw_lines(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &mut RedrawDrawCtx,
    scene: &RedrawScene,
    flags: RedrawOps,
) {
    for y in 0..scene.sy {
        let cy = client_row(dctx, y);
        for kind in SPAN_KINDS {
            if !kind_allowed(flags, kind) {
                continue;
            }
            for span in scene.spans(y, kind) {
                redraw_draw_span(srv, t, dctx, scene.window, span, cy);
            }
        }
    }
}

/// Draw menu spans.
fn redraw_draw_menu_lines(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &mut RedrawDrawCtx,
    scene: &RedrawScene,
) {
    for y in 0..scene.sy {
        let cy = client_row(dctx, y);
        for span in scene.spans(y, RedrawSpanKind::Menu) {
            redraw_draw_span(srv, t, dctx, scene.window, span, cy);
        }
    }
}

/// Get line for pane status line.
fn redraw_pane_status_line(srv: &Server, scene: &RedrawScene, wp: PaneId) -> Option<u32> {
    let pane_status = border::pane_status_of(srv, wp);
    if pane_status == PaneStatusPosition::Off {
        return None;
    }
    let p = srv.panes.get(wp)?;
    let wy = if pane_status == PaneStatusPosition::Top {
        p.yoff - 1
    } else {
        p.yoff + p.sy as i32
    };
    if wy < 0 || wy < scene.oy as i32 {
        return None;
    }
    if wy as u32 >= scene.oy + scene.sy {
        return None;
    }
    Some(wy as u32 - scene.oy)
}

/// Get available width for pane status line, plus the status span slice for
/// that line and the index of the first span owned by the pane.
fn redraw_pane_status_width<'a>(
    srv: &Server,
    scene: &'a RedrawScene,
    wp: PaneId,
) -> (u32, &'a [RedrawSpan], Option<usize>) {
    let Some(y) = redraw_pane_status_line(srv, scene, wp) else {
        return (0, &[], None);
    };
    let spans = scene.spans(y, RedrawSpanKind::Status);
    let mut width = 0;
    let mut first = None;
    for (i, span) in spans.iter().enumerate() {
        if let RedrawSpanData::Status { wp: w, offset, .. } = span.data {
            if w == wp {
                if first.is_none() {
                    first = Some(i);
                }
                let end = offset + span.width;
                if end > width {
                    width = end;
                }
            }
        }
    }
    (width, spans, first)
}

/// Set up draw context.
fn redraw_set_draw_context(srv: &Server, c: ClientId) -> RedrawDrawCtx {
    let session = srv.clients.get(c).and_then(|c| c.session);
    let curw = styles::client_winlink(srv, c);
    let w = client_window(srv, c);
    let marked = srv
        .marked_pane
        .filter(|wp| crate::server::operations::server_is_marked(srv, session, curw, Some(*wp)));
    let active = w.and_then(|w| srv.windows.get(w)?.active);
    let lines = status::status_line_size(srv, c);
    let oo = styles::session_options(srv, c);
    let status_top = srv.options.get_number(oo, b"status-position") == 0;
    let isolates = srv.clients.get(c).is_some_and(|cl| {
        cl.flags.contains(ClientFlags::UTF8)
            && cl
                .tty
                .as_ref()
                .is_some_and(|tty| tty.term().has(TtyCodeCode::Bidi))
    });
    RedrawDrawCtx {
        c,
        active,
        marked,
        status_lines: lines,
        pane_lines: PaneLines::Single,
        default_gc: DEFAULT_CELL,
        isolates,
        default_set: false,
        status_top,
    }
}

/// Build a pane prompt into a one-line screen.
fn redraw_make_pane_prompt(
    srv: &mut Server,
    wp: PaneId,
) -> Option<(Screen, rmux_emu::hyperlinks::HyperlinkRegistry)> {
    let sx = srv.panes.get(wp)?.sx;
    // The one-line prompt screen has its own registry so the server stays
    // free for format expansion while drawing.
    let mut registry = rmux_emu::hyperlinks::HyperlinkRegistry::new();
    let mut screen = Screen::new(sx, 1, 0, ScreenResetPolicy::default(), &mut registry).ok()?;
    let mut cursor_x = 0;
    {
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        let mut pdd = PromptDrawData {
            cursor_x: &mut cursor_x,
            area_x: 0,
            area_width: sx,
            prompt_line: 0,
        };
        crate::model::pane::pane_with_prompt(srv, wp, |srv, prompt| {
            prompt.engine.draw(srv, wp, &mut ctx, &mut pdd);
        });
        ctx.finish();
    }
    if let Some(p) = srv.panes.get_mut(wp) {
        p.prompt_cx = cursor_x;
    }
    Some((screen, registry))
}

/// Draw a pane's prompt over its content.
fn redraw_draw_pane_prompt(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &RedrawDrawCtx,
    scene: &RedrawScene,
    wp: PaneId,
) {
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    if p.prompt.is_none() || p.sx == 0 || p.sy == 0 {
        return;
    }
    let (xoff, yoff, psx, psy) = (p.xoff, p.yoff, p.sx as i32, p.sy as i32);
    let (ox, oy, sx, sy) = (
        scene.ox as i32,
        scene.oy as i32,
        scene.sx as i32,
        scene.sy as i32,
    );
    let wy = if !dctx.status_top {
        yoff + psy - 1
    } else {
        yoff
    };
    if wy < oy || wy >= oy + sy {
        return;
    }
    let line = wy - oy;
    let cy = if dctx.status_top {
        dctx.status_lines as i32 + line
    } else {
        line
    };
    if xoff + psx <= ox || xoff >= ox + sx {
        return;
    }
    let (offset, px) = if xoff < ox {
        (ox - xoff, 0)
    } else {
        (0, xoff - ox)
    };
    let mut width = psx - offset;
    if px + width > sx {
        width = sx - px;
    }
    let Some((mut screen, mut registry)) = redraw_make_pane_prompt(srv, wp) else {
        return;
    };
    t.tty.draw_line(
        &mut t.tparm,
        &registry,
        &screen,
        offset as u32,
        0,
        width as u32,
        px as u32,
        cy as u32,
        None,
    );
    let _ = screen.release(
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
}

/// Draw a note over a pane whose program draws only natively, so the empty
/// grid does not look like a hung program.
fn redraw_draw_pane_native(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &RedrawDrawCtx,
    scene: &RedrawScene,
    wp: PaneId,
) {
    let Some(title) = crate::tsp::broker::native_only(srv, wp) else {
        return;
    };
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    if p.sx == 0 || p.sy == 0 {
        return;
    }
    let (xoff, yoff, psx, psy) = (p.xoff, p.yoff, p.sx as i32, p.sy as i32);
    let (ox, oy, sx, sy) = (
        scene.ox as i32,
        scene.oy as i32,
        scene.sx as i32,
        scene.sy as i32,
    );
    if xoff + psx <= ox || xoff >= ox + sx {
        return;
    }
    let (offset, px) = if xoff < ox {
        (ox - xoff, 0)
    } else {
        (0, xoff - ox)
    };
    let width = (psx - offset).min(sx - px);
    let title = status::status_message_escape(title.as_bytes());
    let mut first = b"#[align=centre,bright]".to_vec();
    first.extend_from_slice(title.as_bytes());
    first.extend_from_slice(b": native view");
    let split = srv.windows.get(p.window).is_some_and(|w| {
        w.panes
            .iter()
            .any(|id| *id != wp && pane_is_visible(srv, *id))
    });
    let hint: &[u8] = if split {
        b"#[align=centre,dim]zoom this pane or close the others to see it"
    } else {
        b"#[align=centre,dim]restore a full-pane TSP view to see it"
    };
    let lines = [first, hint.to_vec()];
    let mut registry = rmux_emu::hyperlinks::HyperlinkRegistry::new();
    let Ok(mut screen) = Screen::new(
        psx as u32,
        lines.len() as u32,
        0,
        ScreenResetPolicy::default(),
        &mut registry,
    ) else {
        return;
    };
    {
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        for (i, line) in lines.iter().enumerate() {
            ctx.cursormove(0, i as i32, false);
            crate::format::draw::draw(&mut ctx, &DEFAULT_CELL, psx as u32, line, None, false);
        }
        ctx.finish();
    }
    let top = yoff + (psy - lines.len() as i32) / 2;
    for i in 0..lines.len() {
        let wy = top + i as i32;
        if wy < oy || wy >= oy + sy {
            continue;
        }
        let line = wy - oy;
        let cy = if dctx.status_top {
            dctx.status_lines as i32 + line
        } else {
            line
        };
        t.tty.draw_line(
            &mut t.tparm,
            &registry,
            &screen,
            offset as u32,
            i as u32,
            width as u32,
            px as u32,
            cy as u32,
            None,
        );
    }
    let _ = screen.release(
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
}

/// Draw scene to client.
fn redraw_draw(srv: &mut Server, c: ClientId, wp: Option<PaneId>, mut flags: RedrawOps) {
    if crate::tsp::broker::native_client(srv, c) {
        return;
    }
    let Some(client) = srv.clients.get(c) else {
        return;
    };
    if client.flags.contains(ClientFlags::SUSPENDED) {
        return;
    }
    let Some(w) = client_window(srv, c) else {
        return;
    };

    if flags.intersects(RedrawOps::STATUS) {
        let client = srv.clients.get(c).expect("client");
        let redraw = if client.message.text.is_some() {
            status::status_message_redraw(srv, c)
        } else if client.prompt.is_some() {
            status::status_prompt_redraw(srv, c)
        } else {
            status::status_redraw(srv, c)
        };
        let always = srv
            .clients
            .get(c)
            .is_some_and(|cl| cl.flags.contains(ClientFlags::REDRAWSTATUSALWAYS));
        if !redraw && !always && !flags.is_all() {
            flags.remove(RedrawOps::STATUS);
            if flags.0 == 0 {
                return;
            }
        }
    }
    if rmux_util::log::enabled() {
        log_debug!("{}: starting redraw ({})", client_name(srv, c), flags);
    }

    let Some(scene) = redraw_get_scene(srv, c) else {
        return;
    };
    let mut dctx = redraw_set_draw_context(srv, c);
    if srv.windows.get(w).is_some_and(|w| w.menu.is_some()) {
        menu::menu_update(srv, w);
    }

    let panes: Vec<PaneId> = srv
        .windows
        .get(w)
        .map(|w| w.panes.clone())
        .unwrap_or_default();
    if flags.intersects(RedrawOps::PANE_BORDER | RedrawOps::PANE_STATUS) {
        for wp in &panes {
            if let Some(p) = srv.panes.get_mut(*wp) {
                p.border_gc_set = false;
                p.active_border_gc_set = false;
            }
        }
    }

    if flags.intersects(RedrawOps::PANE_STATUS) {
        let mut redraw = false;
        for loop_wp in &panes {
            if let Some(p) = srv.panes.get_mut(*loop_wp) {
                if flags.is_all() {
                    p.flags.insert(PaneFlags::NEWSTATUS);
                } else {
                    p.flags.remove(PaneFlags::NEWSTATUS);
                }
            }
            let (width, spans, first) = redraw_pane_status_width(srv, &scene, *loop_wp);
            if width == 0 {
                continue;
            }
            if border::window_make_pane_status(srv, *loop_wp, c, width, spans, first) {
                if let Some(p) = srv.panes.get_mut(*loop_wp) {
                    p.flags.insert(PaneFlags::NEWSTATUS);
                }
                redraw = true;
            }
        }
        if !redraw && !flags.is_all() {
            flags.remove(RedrawOps::PANE_STATUS);
            if flags.0 == 0 {
                put_scene(srv, c, scene);
                return;
            }
        }
    }

    if flags.intersects(RedrawOps::PANE) {
        match wp {
            Some(wp) => {
                if srv
                    .panes
                    .get(wp)
                    .is_some_and(|p| p.base.mode.contains(ScreenMode::SYNC))
                {
                    crate::ui::fanout::screen_write_stop_sync(srv, wp);
                }
                crate::ui::fanout::screen_write_sync_clear_dirty(srv, wp);
            }
            None => {
                for loop_wp in &panes {
                    if !pane_is_visible(srv, *loop_wp) {
                        continue;
                    }
                    if srv
                        .panes
                        .get(*loop_wp)
                        .is_some_and(|p| p.base.mode.contains(ScreenMode::SYNC))
                    {
                        crate::ui::fanout::screen_write_stop_sync(srv, *loop_wp);
                    }
                    crate::ui::fanout::screen_write_sync_clear_dirty(srv, *loop_wp);
                }
            }
        }
    }

    let Some(mut t) = DrawTarget::take(srv, c) else {
        put_scene(srv, c, scene);
        return;
    };
    t.tty.sync_start(&mut t.tparm); /* end in server_client_reset_state */
    let mode = t.tty.mode();
    t.tty.update_mode(
        &mut t.tparm,
        ScreenMode(mode.0 & !ScreenMode::CURSOR_MODES.0),
        None,
    );

    match wp {
        Some(wp) => redraw_draw_pane_lines(srv, &mut t, &mut dctx, &scene, wp, flags),
        None => redraw_draw_lines(srv, &mut t, &mut dctx, &scene, flags),
    }

    if flags.intersects(RedrawOps::PANE) {
        match wp {
            Some(wp) => {
                redraw_draw_pane_native(srv, &mut t, &dctx, &scene, wp);
                redraw_draw_pane_prompt(srv, &mut t, &dctx, &scene, wp);
            }
            None => {
                for loop_wp in &panes {
                    if pane_is_visible(srv, *loop_wp) {
                        redraw_draw_pane_native(srv, &mut t, &dctx, &scene, *loop_wp);
                        redraw_draw_pane_prompt(srv, &mut t, &dctx, &scene, *loop_wp);
                    }
                }
            }
        }
    }
    if flags.intersects(RedrawOps::MENU) && srv.windows.get(w).is_some_and(|w| w.menu.is_some()) {
        redraw_draw_menu_lines(srv, &mut t, &mut dctx, &scene);
    }

    if flags.intersects(RedrawOps::STATUS) {
        let client = srv.clients.get(c).expect("client");
        let mut lines = dctx.status_lines;
        if client.message.text.is_some() || client.prompt.is_some() {
            lines = if lines == 0 { 1 } else { lines };
        }
        let (tty_sx, tty_sy) = t.tty.size();
        let y = if dctx.status_top {
            0
        } else {
            tty_sy.saturating_sub(lines)
        };
        let sl = client.status.active();
        for i in 0..lines {
            t.tty.draw_line(
                &mut t.tparm,
                &srv.hyperlinks,
                sl,
                0,
                i,
                tty_sx,
                0,
                y + i,
                None,
            );
        }
    }

    t.tty.reset(&mut t.tparm);
    #[cfg(feature = "sixel")]
    match wp {
        Some(wp) => crate::ui::fanout::tty_draw_images(&mut t.tty, &mut t.tparm, srv, c, wp),
        None => {
            for wp in &panes {
                crate::ui::fanout::tty_draw_images(&mut t.tty, &mut t.tparm, srv, c, *wp);
            }
        }
    }
    t.restore(srv, c);
    put_scene(srv, c, scene);
    log_debug!("{}: finished redraw", client_name(srv, c));
}

/// Get border cell type beneath status cell at offset x in pane status line.
/// `cursor` is the index of the current span in `spans` (None = exhausted).
pub fn redraw_get_status_border_cell_type(
    spans: &[RedrawSpan],
    wp: PaneId,
    cursor: &mut Option<usize>,
    x: u32,
) -> BorderCell {
    let Some(mut i) = *cursor else {
        return BorderCell::Lr;
    };
    if i >= spans.len() {
        *cursor = None;
        return BorderCell::Lr;
    }
    if !matches!(spans[i].data, RedrawSpanData::Status { .. }) {
        return BorderCell::Lr;
    }
    while i < spans.len() {
        let span = &spans[i];
        let RedrawSpanData::Status {
            wp: swp,
            offset,
            cell_type,
        } = span.data
        else {
            i += 1;
            continue;
        };
        if swp != wp {
            i += 1;
            continue;
        }
        let start = offset;
        let end = start + span.width;
        if x >= start && x < end {
            *cursor = Some(i);
            return cell_type;
        }
        if start > x {
            *cursor = Some(i);
            return BorderCell::Lr;
        }
        i += 1;
    }
    *cursor = None;
    BorderCell::Lr
}

/// Draw screen.
pub fn redraw_screen(srv: &mut Server, c: ClientId) {
    let Some(client) = srv.clients.get(c) else {
        return;
    };
    let cflags = client.flags;
    if cflags.contains(ClientFlags::REDRAWWINDOW) {
        redraw_draw(srv, c, None, RedrawOps::ALL);
        return;
    }
    let mut flags = RedrawOps(0);
    if cflags.contains(ClientFlags::REDRAWBORDERS) {
        flags.insert(RedrawOps::PANE_BORDER | RedrawOps::PANE_STATUS);
    }
    if cflags.contains(ClientFlags::REDRAWSTATUS)
        || cflags.contains(ClientFlags::REDRAWSTATUSALWAYS)
    {
        flags.insert(RedrawOps::STATUS | RedrawOps::PANE_STATUS);
    }
    if cflags.contains(ClientFlags::REDRAWMENU) {
        flags.insert(RedrawOps::MENU);
    }
    if client_window(srv, c)
        .and_then(|w| srv.windows.get(w))
        .is_some_and(|w| w.menu.is_some())
    {
        flags.insert(RedrawOps::MENU);
    }
    if flags.0 != 0 {
        redraw_draw(srv, c, None, flags);
    }
}

/// Draw a single pane.
pub fn redraw_pane(srv: &mut Server, c: ClientId, wp: PaneId) {
    redraw_draw(
        srv,
        c,
        Some(wp),
        RedrawOps::PANE | RedrawOps::PANE_SCROLLBAR,
    );
    if client_window(srv, c)
        .and_then(|w| srv.windows.get(w))
        .is_some_and(|w| w.menu.is_some())
    {
        redraw_draw(srv, c, None, RedrawOps::MENU);
    }
}

/// Draw a pane's scrollbar.
pub fn redraw_pane_scrollbar(srv: &mut Server, c: ClientId, wp: PaneId) {
    redraw_draw(srv, c, Some(wp), RedrawOps::PANE_SCROLLBAR);
}

/// Rebuild damaged pane status.
fn redraw_damage_refresh_status(
    srv: &mut Server,
    dctx: &RedrawDrawCtx,
    scene: &RedrawScene,
    wp: PaneId,
) {
    let g = srv.redraw_status_generation;
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    if p.flags.contains(PaneFlags::NEWSTATUS) && p.status_generation == g {
        return;
    }
    let (width, spans, first) = redraw_pane_status_width(srv, scene, wp);
    if width != 0 {
        border::window_make_pane_status(srv, wp, dctx.c, width, spans, first);
        if let Some(p) = srv.panes.get_mut(wp) {
            p.flags.insert(PaneFlags::NEWSTATUS);
            p.status_generation = g;
        }
    }
}

/// Draw a pane's prompt over a damaged span.
fn redraw_damage_draw_pane_prompt(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &RedrawDrawCtx,
    span: &RedrawSpan,
    y: u32,
) {
    let RedrawSpanData::Pane { wp, px, py } = span.data else {
        return;
    };
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    if p.prompt.is_none() || p.sx == 0 || p.sy == 0 {
        return;
    }
    let prompt_y = if dctx.status_top { 0 } else { p.sy - 1 };
    if py != prompt_y {
        return;
    }
    let Some((mut screen, mut registry)) = redraw_make_pane_prompt(srv, wp) else {
        return;
    };
    let ssx = screen.grid.sx();
    if px < ssx {
        let mut width = span.width;
        if width > ssx - px {
            width = ssx - px;
        }
        t.tty.draw_line(
            &mut t.tparm,
            &registry,
            &screen,
            px,
            0,
            width,
            span.x,
            y,
            None,
        );
    }
    let _ = screen.release(
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
}

/// Draw the spans intersecting a damaged rectangle.
#[allow(clippy::too_many_arguments)]
fn redraw_draw_damage_rectangle(
    srv: &mut Server,
    t: &mut DrawTarget,
    dctx: &mut RedrawDrawCtx,
    scene: &RedrawScene,
    x: u32,
    y: u32,
    mut sx: u32,
    mut sy: u32,
) {
    if x >= scene.sx || y >= scene.sy {
        return;
    }
    if x + sx > scene.sx {
        sx = scene.sx - x;
    }
    if y + sy > scene.sy {
        sy = scene.sy - y;
    }
    if sx == 0 || sy == 0 {
        return;
    }
    for yy in y..y + sy {
        let cy = client_row(dctx, yy);
        for kind in SPAN_KINDS {
            for span in scene.spans(yy, kind) {
                if span.x >= x + sx || span.x + span.width <= x {
                    continue;
                }
                if let RedrawSpanData::Status { wp, .. } = span.data {
                    redraw_damage_refresh_status(srv, dctx, scene, wp);
                }
                redraw_draw_span(srv, t, dctx, scene.window, span, cy);
                if kind == RedrawSpanKind::Pane {
                    redraw_damage_draw_pane_prompt(srv, t, dctx, span, cy);
                }
            }
        }
    }
}

/// Draw pending window damage on this client.
pub fn redraw_client_damage(srv: &mut Server, c: ClientId) {
    if crate::tsp::broker::native_client(srv, c) {
        return;
    }
    let Some(w) = client_window(srv, c) else {
        return;
    };
    let damage: Vec<RedrawDamage> = match srv.windows.get(w) {
        Some(win) if !win.damage.is_empty() => win.damage.iter().copied().collect(),
        _ => return,
    };
    srv.redraw_status_generation = srv.redraw_status_generation.wrapping_add(1);

    let Some(scene) = redraw_get_scene(srv, c) else {
        return;
    };
    let mut dctx = redraw_set_draw_context(srv, c);
    let (ox, oy, sx, sy) = redraw_get_window_offset(srv, c);

    let panes: Vec<PaneId> = srv
        .windows
        .get(w)
        .map(|w| w.panes.clone())
        .unwrap_or_default();
    for wp in &panes {
        if let Some(p) = srv.panes.get_mut(*wp) {
            p.border_gc_set = false;
            p.active_border_gc_set = false;
        }
    }

    let Some(mut t) = DrawTarget::take(srv, c) else {
        put_scene(srv, c, scene);
        return;
    };
    t.tty.sync_start(&mut t.tparm);
    let mode = t.tty.mode();
    t.tty.update_mode(
        &mut t.tparm,
        ScreenMode(mode.0 & !ScreenMode::CURSOR_MODES.0),
        None,
    );

    for rd in damage {
        let x0 = rd.x.max(ox);
        let y0 = rd.y.max(oy);
        let x1 = (rd.x + rd.sx).min(ox + sx);
        let y1 = (rd.y + rd.sy).min(oy + sy);
        if x0 < x1 && y0 < y1 {
            redraw_draw_damage_rectangle(
                srv,
                &mut t,
                &mut dctx,
                &scene,
                x0 - ox,
                y0 - oy,
                x1 - x0,
                y1 - y0,
            );
        }
    }
    t.restore(srv, c);
    put_scene(srv, c, scene);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_type_table_covers_all_masks() {
        use BorderCell as B;
        let expect = [
            (0, B::None),
            (1, B::Lr),
            (2, B::Lr),
            (3, B::Lr),
            (4, B::Ud),
            (5, B::Lu),
            (6, B::Ru),
            (7, B::Lru),
            (8, B::Ud),
            (9, B::Ld),
            (10, B::Rd),
            (11, B::Lrd),
            (12, B::Ud),
            (13, B::Uld),
            (14, B::Urd),
            (15, B::Lrud),
        ];
        for (mask, cell) in expect {
            assert_eq!(redraw_get_cell_type(mask), cell, "mask {mask}");
        }
    }

    fn win(sx: u32, sy: u32) -> RedrawDamages {
        let _ = (sx, sy);
        RedrawDamages::new()
    }

    fn damage(d: &mut RedrawDamages, wsx: u32, wsy: u32, x: u32, y: u32, sx: u32, sy: u32) {
        // Mirror redraw_damage_window on a bare queue.
        let mut sx = sx;
        let mut sy = sy;
        if x >= wsx || y >= wsy {
            return;
        }
        if x + sx > wsx {
            sx = wsx - x;
        }
        if y + sy > wsy {
            sy = wsy - y;
        }
        if sx == 0 || sy == 0 {
            return;
        }
        for rd in d.iter_mut() {
            if x > rd.x + rd.sx || rd.x > x + sx || y > rd.y + rd.sy || rd.y > y + sy {
                continue;
            }
            let x0 = x.min(rd.x);
            let y0 = y.min(rd.y);
            let x1 = (x + sx).max(rd.x + rd.sx);
            let y1 = (y + sy).max(rd.y + rd.sy);
            let area = sx * sy + rd.sx * rd.sy;
            if (x1 - x0) * (y1 - y0) > 2 * area {
                continue;
            }
            *rd = RedrawDamage {
                x: x0,
                y: y0,
                sx: x1 - x0,
                sy: y1 - y0,
            };
            return;
        }
        d.push_back(RedrawDamage { x, y, sx, sy });
        if d.len() > REDRAW_DAMAGE_MAX {
            redraw_collapse_damage(d);
        }
    }

    #[test]
    fn damage_merges_only_first_eligible_and_collapses() {
        let mut d = win(80, 24);
        damage(&mut d, 80, 24, 0, 0, 10, 1);
        damage(&mut d, 80, 24, 10, 0, 10, 1);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0],
            RedrawDamage {
                x: 0,
                y: 0,
                sx: 20,
                sy: 1
            }
        );
        // Far apart: no merge (union area too big).
        damage(&mut d, 80, 24, 0, 20, 1, 1);
        assert_eq!(d.len(), 2);
        // A rectangle touching the second only merges into the second, not
        // then again into the first.
        damage(&mut d, 80, 24, 1, 20, 1, 1);
        assert_eq!(d.len(), 2);
        assert_eq!(
            d[1],
            RedrawDamage {
                x: 0,
                y: 20,
                sx: 2,
                sy: 1
            }
        );
        // Clip to window and reject empty.
        damage(&mut d, 80, 24, 79, 23, 10, 10);
        assert_eq!(
            d[2],
            RedrawDamage {
                x: 79,
                y: 23,
                sx: 1,
                sy: 1
            }
        );
        damage(&mut d, 80, 24, 80, 0, 1, 1);
        assert_eq!(d.len(), 3);
        // Collapse at the seventeenth rectangle.
        let mut d = win(100, 100);
        for i in 0..16 {
            damage(&mut d, 100, 100, i * 6, i * 6, 1, 1);
        }
        assert_eq!(d.len(), 16);
        damage(&mut d, 100, 100, 99, 99, 1, 1);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0],
            RedrawDamage {
                x: 0,
                y: 0,
                sx: 100,
                sy: 100
            }
        );
    }

    #[test]
    fn compare_data_join_rules() {
        use RedrawSpanData as D;
        let wp = crate::ids::ArenaId::from_parts(1, 1);
        let wp2 = crate::ids::ArenaId::from_parts(2, 1);
        assert!(redraw_compare_data(
            &D::Pane { wp, px: 0, py: 3 },
            &D::Pane { wp, px: 1, py: 3 }
        ));
        assert!(!redraw_compare_data(
            &D::Pane { wp, px: 0, py: 3 },
            &D::Pane { wp, px: 2, py: 3 }
        ));
        assert!(!redraw_compare_data(
            &D::Pane { wp, px: 0, py: 3 },
            &D::Pane {
                wp: wp2,
                px: 1,
                py: 3
            }
        ));
        assert!(redraw_compare_data(&D::Empty, &D::Empty));
        assert!(redraw_compare_data(&D::Outside, &D::Outside));
        assert!(!redraw_compare_data(&D::Empty, &D::Outside));
        let mut b = D::empty_border();
        if let D::Border {
            top,
            cell_mask,
            cell_type,
            ..
        } = &mut b
        {
            *top = Some(wp);
            *cell_mask = 3;
            *cell_type = BorderCell::Lr;
        }
        assert!(redraw_compare_data(&b, &b));
        let mut arrow = b;
        if let D::Border { flags, .. } = &mut arrow {
            flags.insert(BorderSpanFlags::IS_ARROW);
        }
        assert!(!redraw_compare_data(&arrow, &arrow));
        assert!(!redraw_compare_data(&b, &arrow));
        let s = |o| D::Status {
            wp,
            offset: o,
            cell_type: BorderCell::Lr,
        };
        assert!(redraw_compare_data(&s(0), &s(1)));
        assert!(!redraw_compare_data(&s(0), &s(2)));
        let sb = D::Scrollbar {
            wp,
            y: 1,
            height: 5,
            flags: ScrollbarSpanFlags::RIGHT,
        };
        assert!(redraw_compare_data(&sb, &sb));
        assert!(redraw_compare_data(
            &D::Menu { px: 0, py: 0 },
            &D::Menu { px: 1, py: 0 }
        ));
        assert!(!redraw_compare_data(
            &D::Menu { px: 0, py: 0 },
            &D::Menu { px: 1, py: 1 }
        ));
    }

    #[test]
    fn status_border_cursor_skips_other_owners_and_gaps() {
        let wp = crate::ids::ArenaId::from_parts(1, 1);
        let other = crate::ids::ArenaId::from_parts(2, 1);
        let spans = vec![
            RedrawSpan {
                x: 2,
                width: 3,
                data: RedrawSpanData::Status {
                    wp,
                    offset: 0,
                    cell_type: BorderCell::Lrd,
                },
            },
            RedrawSpan {
                x: 5,
                width: 2,
                data: RedrawSpanData::Status {
                    wp: other,
                    offset: 0,
                    cell_type: BorderCell::Lru,
                },
            },
            RedrawSpan {
                x: 9,
                width: 2,
                data: RedrawSpanData::Status {
                    wp,
                    offset: 7,
                    cell_type: BorderCell::Lru,
                },
            },
        ];
        let mut cur = Some(0);
        assert_eq!(
            redraw_get_status_border_cell_type(&spans, wp, &mut cur, 0),
            BorderCell::Lrd
        );
        assert_eq!(
            redraw_get_status_border_cell_type(&spans, wp, &mut cur, 2),
            BorderCell::Lrd
        );
        assert_eq!(
            redraw_get_status_border_cell_type(&spans, wp, &mut cur, 3),
            BorderCell::Lr
        );
        assert_eq!(cur, Some(2));
        assert_eq!(
            redraw_get_status_border_cell_type(&spans, wp, &mut cur, 7),
            BorderCell::Lru
        );
        assert_eq!(
            redraw_get_status_border_cell_type(&spans, wp, &mut cur, 9),
            BorderCell::Lr
        );
        assert_eq!(cur, None);
        assert_eq!(
            redraw_get_status_border_cell_type(&spans, wp, &mut cur, 10),
            BorderCell::Lr
        );
        let mut none = None;
        assert_eq!(
            redraw_get_status_border_cell_type(&spans, wp, &mut none, 0),
            BorderCell::Lr
        );
    }

    #[test]
    fn ops_display_and_all() {
        assert!(RedrawOps::ALL.is_all());
        assert!(!(RedrawOps::PANE | RedrawOps::STATUS).is_all());
        assert_eq!(
            (RedrawOps::STATUS | RedrawOps::MENU).to_string(),
            "status menu"
        );
        assert_eq!(
            RedrawOps::ALL.to_string(),
            "status pane border pane-status scrollbar menu all"
        );
    }
}
