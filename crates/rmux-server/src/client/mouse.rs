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

//! Mouse resolution (`server_client_check_mouse` and helpers), the
//! double-click timer and `server_client_remove_pane`.

use std::time::Duration;

use crate::client::{ClientFlags, KeyEvent};
use crate::ids::*;
use crate::model::PaneFlags;
use crate::model::pane::{
    pane_contains, pane_find_by_public_id, pane_get_pane_lines, pane_get_pane_status,
    pane_is_floating, pane_is_visible, pane_scrollbar_overlay, pane_scrollbar_reserve,
    pane_scrollbar_show, pane_scrollbar_visible,
};
use crate::model::session::session_find_by_id;
use crate::model::window::{
    window_get_active_at, window_redraw_active_switch, window_set_active_pane,
    winlink_find_by_index,
};
use crate::server::Server;
use crate::server::event_loop::LoopAction;
use crate::server::operations::{
    server_kill_pane, server_redraw_window_borders, server_status_window,
};
use crate::ui::scrollbar::PaneScrollbarPosition;
use crate::ui::status::{status_at_line, status_get_range, status_line_size};
use rmux_emu::style::StyleRangeType;
use rmux_util::key::{
    KeyCode, KeyCodeType, KeyModifiers, MouseButton, MouseButtonBits, MouseEvent, MouseLocation,
    SpecialKey,
};
use rmux_util::log_debug;

/// `PANE_STATUS_OFF/TOP/BOTTOM` (`tmux.h:1548-1550`) as `pane_get_pane_status` returns them.
const PANE_STATUS_OFF: i64 = 0;
const PANE_STATUS_TOP: i64 = 1;
const PANE_STATUS_BOTTOM: i64 = 2;
/// `PANE_LINES_NONE` (`tmux.h:1156`).
const PANE_LINES_NONE: i64 = 6;

/// Resolved fields of `struct mouse_event` (`tmux.h:1699-1720`): `m->s`,
/// `m->w`, `m->wp`, `statusat`, `statuslines`, `ox`, `oy`, `ignore`, `valid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseTarget {
    pub session: Option<SessionId>,
    pub window: Option<WindowId>,
    pub pane: Option<PaneId>,
    /// `m->statusat`; -1 when there is no status line.
    pub status_at: i32,
    pub status_lines: u32,
    pub ox: u32,
    pub oy: u32,
    pub ignore: bool,
    pub valid: bool,
    pub key: KeyCode,
}
impl Default for MouseTarget {
    fn default() -> Self {
        Self {
            session: None,
            window: None,
            pane: None,
            status_at: -1,
            status_lines: 0,
            ox: 0,
            oy: 0,
            ignore: false,
            valid: false,
            key: KeyCode(SpecialKey::UNKNOWN),
        }
    }
}

/// The value `cmd_find_from_mouse`, modes and drag actions receive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolvedMouseEvent {
    pub event: MouseEvent,
    pub target: MouseTarget,
}

/// `tty.mouse_drag_update` / `mouse_drag_release` (`tmux.h:1852-1855`) as
/// typed actions. A mode (copy mode) installs `Mode`; G21 installs the
/// resize and split actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseDragAction {
    Mode(ModeId),
    ResizeTiled,
    ResizeMoveFloating,
    SplitWindowResize,
    /// `move-pane -M` (`cmd-join-pane.c`); C installs only the update action.
    MoveFloating,
}
impl MouseDragAction {
    /// `c->tty.mouse_drag_update(c, m)`.
    pub fn update(self, server: &mut Server, client: ClientId, ev: &ResolvedMouseEvent) {
        match self {
            Self::Mode(mode) => crate::model::pane::pane_mode_drag_update(server, mode, client, ev),
            Self::ResizeTiled => {
                crate::cmd::commands::resize_pane::drag_update_tiled(server, client, ev)
            }
            Self::ResizeMoveFloating => {
                crate::cmd::commands::resize_pane::drag_update_floating(server, client, ev)
            }
            Self::SplitWindowResize => {
                crate::cmd::commands::split_window::drag_update(server, client, ev)
            }
            Self::MoveFloating => crate::cmd::commands::join_pane::drag_update(server, client, ev),
        }
    }
    /// `c->tty.mouse_drag_release(c, m)`.
    pub fn release(self, server: &mut Server, client: ClientId, ev: &ResolvedMouseEvent) {
        match self {
            Self::Mode(mode) => {
                crate::model::pane::pane_mode_drag_release(server, mode, client, ev)
            }
            Self::ResizeTiled => {
                crate::cmd::commands::resize_pane::drag_release_tiled(server, client, ev)
            }
            Self::ResizeMoveFloating => {
                crate::cmd::commands::resize_pane::drag_release_floating(server, client, ev)
            }
            Self::SplitWindowResize => {
                crate::cmd::commands::split_window::drag_release(server, client, ev)
            }
            Self::MoveFloating => {}
        }
    }
}

/// `tty.mouse_drag_flag/x/y`, `mouse_scrolling_flag`, `mouse_slider_mpos`,
/// `mouse_last_pane` and the two callbacks (`tmux.h:1846-1855`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseDragState {
    pub flag: u32,
    pub x: u32,
    pub y: u32,
    pub scrolling: bool,
    pub slider_mpos: Option<u32>,
    pub last_pane: Option<PaneId>,
    pub update: Option<MouseDragAction>,
    pub release: Option<MouseDragAction>,
}

/// `c->click_timer`, `click_button`, `click_loc`, `click_wp`, `click_event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClickState {
    pub timer: Option<TimerId>,
    pub button: u32,
    pub location: MouseLocation,
    pub pane: Option<PaneId>,
    pub event: ResolvedMouseEvent,
}
impl Default for ClickState {
    fn default() -> Self {
        Self {
            timer: None,
            button: 0,
            location: MouseLocation::Nowhere,
            pane: None,
            event: ResolvedMouseEvent::default(),
        }
    }
}

/// Event type chosen by step 1 of `server_client_check_mouse`
/// (`server-client.c:772-836`), plus `DragEnd` (`:1056`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseType {
    Move,
    Down,
    Up,
    Drag,
    DragEnd,
    WheelUp,
    WheelDown,
    Second,
    Double,
    Triple,
    /// Drag update at the same position as the last event (`:792-793`).
    Unknown,
}
impl MouseType {
    pub const fn key_type(self) -> KeyCodeType {
        match self {
            Self::Move => KeyCodeType::Mousemove,
            Self::Down => KeyCodeType::Mousedown,
            Self::Up => KeyCodeType::Mouseup,
            Self::Drag => KeyCodeType::Mousedrag,
            Self::DragEnd => KeyCodeType::Mousedragend,
            Self::WheelUp => KeyCodeType::Wheelup,
            Self::WheelDown => KeyCodeType::Wheeldown,
            Self::Second => KeyCodeType::Secondclick,
            Self::Double => KeyCodeType::Doubleclick,
            Self::Triple => KeyCodeType::Tripleclick,
            Self::Unknown => KeyCodeType::Notype,
        }
    }
}

/// Result of [`classify`]: the chosen type, the event position and button
/// it uses, `ignore`, the client flags after the click bookkeeping and
/// whether the click timer must be deleted (`evtimer_del`, `:814,821`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseTyping {
    pub kind: MouseType,
    pub x: u32,
    pub y: u32,
    pub b: u32,
    pub ignore: bool,
    pub flags: ClientFlags,
    pub cancel_click_timer: bool,
}

/// Step 1 of `server_client_check_mouse` (`server-client.c:772-836`) as a
/// pure function over the client flags, the drag state and the event.
pub fn classify(
    flags: ClientFlags,
    drag: &MouseDragState,
    m: &MouseEvent,
    key: KeyCode,
) -> MouseTyping {
    let b = MouseButtonBits(m.b);
    let sgr_b = MouseButtonBits(m.sgr_b);
    let lb = MouseButtonBits(m.lb);
    let mut out = MouseTyping {
        kind: MouseType::Down,
        x: m.x,
        y: m.y,
        b: m.b,
        ignore: false,
        flags,
        cancel_click_timer: false,
    };
    if key == KeyCode(SpecialKey::DOUBLECLICK) {
        out.kind = MouseType::Double;
        out.ignore = true;
        log_debug!("double-click at {},{}", out.x, out.y);
    } else if (m.sgr_type != b' ' && sgr_b.is_drag() && sgr_b.is_release())
        || (m.sgr_type == b' ' && b.is_drag() && b.is_release() && lb.is_release())
    {
        out.kind = MouseType::Move;
        out.b = 0;
        log_debug!("move at {},{}", out.x, out.y);
    } else if b.is_drag() {
        out.kind = MouseType::Drag;
        if drag.flag != 0 {
            if m.x == m.lx && m.y == m.ly {
                out.kind = MouseType::Unknown;
                return out;
            }
            log_debug!("drag update at {},{}", out.x, out.y);
        } else {
            out.x = m.lx;
            out.y = m.ly;
            out.b = m.lb;
            log_debug!("drag start at {},{}", out.x, out.y);
        }
    } else if b.is_wheel() {
        out.kind = if b.buttons() == MouseButton::WheelUp as u32 {
            MouseType::WheelUp
        } else {
            MouseType::WheelDown
        };
        log_debug!("wheel at {},{}", out.x, out.y);
    } else if b.is_release() {
        out.kind = MouseType::Up;
        out.b = m.lb;
        if m.sgr_type == b'm' {
            out.b = m.sgr_b;
        }
        log_debug!("up at {},{}", out.x, out.y);
    } else if flags.intersects(ClientFlags::DOUBLECLICK) {
        out.cancel_click_timer = true;
        out.flags.remove(ClientFlags::DOUBLECLICK);
        out.kind = MouseType::Second;
        log_debug!("second-click at {},{}", out.x, out.y);
        out.flags.insert(ClientFlags::TRIPLECLICK);
    } else if flags.intersects(ClientFlags::TRIPLECLICK) {
        out.cancel_click_timer = true;
        out.flags.remove(ClientFlags::TRIPLECLICK);
        out.kind = MouseType::Triple;
        log_debug!("triple-click at {},{}", out.x, out.y);
    } else {
        out.kind = MouseType::Down;
        log_debug!("down at {},{}", out.x, out.y);
        out.flags.insert(ClientFlags::DOUBLECLICK);
    }
    out
}

/// Geometry of one pane and its scrollbar, extracted so that the boundary
/// predicates below stay pure (`server-client.c:545-741`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneGeometry {
    pub xoff: i32,
    pub yoff: i32,
    pub sx: u32,
    pub sy: u32,
    /// `scrollbar_style.width` / `.pad`.
    pub sb_width: i32,
    pub sb_pad: i32,
    /// `w->sb_pos == PANE_SCROLLBARS_LEFT`.
    pub sb_left: bool,
    pub slider_y: u32,
    pub slider_h: u32,
}

fn pane_geometry(server: &Server, pane: PaneId) -> Option<PaneGeometry> {
    let p = server.panes.get(pane)?;
    let w = server.windows.get(p.window)?;
    Some(PaneGeometry {
        xoff: p.xoff,
        yoff: p.yoff,
        sx: p.sx,
        sy: p.sy,
        sb_width: p.scrollbar_style.width,
        sb_pad: p.scrollbar_style.pad,
        sb_left: w.sb_pos == PaneScrollbarPosition::Left,
        slider_y: p.sb_slider_y,
        slider_h: p.sb_slider_h,
    })
}

/// Auto-hide scrollbar interaction strip, overlay already known
/// (`server-client.c:554-570`): width plus pad, clipped to the pane width.
pub fn scrollbar_area_contains(g: &PaneGeometry, px: i32, py: i32) -> bool {
    if py < g.yoff || py >= g.yoff + g.sy as i32 {
        return false;
    }
    let width = g.sb_width as u32;
    let pad = g.sb_pad as u32;
    let mut total = width.wrapping_add(pad);
    if total == 0 || total > g.sx {
        total = g.sx;
    }
    let (start, end) = if g.sb_left {
        (g.xoff, g.xoff + total as i32 - 1)
    } else {
        let end = g.xoff + g.sx as i32 - 1;
        (end - total as i32 + 1, end)
    };
    px >= start && px <= end
}

/// Is this point inside the auto-hide scrollbar interaction area?
/// (`server-client.c:545-571`)
fn in_scrollbar_area(server: &Server, pane: PaneId, px: i32, py: i32) -> bool {
    if !pane_scrollbar_overlay(server, pane) {
        return false;
    }
    let Some(g) = pane_geometry(server, pane) else {
        return false;
    };
    scrollbar_area_contains(&g, px, py)
}

/// Update auto-hide scrollbars for a mouse movement
/// (`server-client.c:574-594`). `point` is `None` for the C `-1, -1` call.
fn update_scrollbar_hover(
    server: &mut Server,
    window: WindowId,
    kind: MouseType,
    point: Option<(i32, i32)>,
) {
    if kind != MouseType::Move {
        return;
    }
    let (px, py) = point.unwrap_or((-1, -1));
    let panes = match server.windows.get(window) {
        Some(w) => w.panes.clone(),
        None => return,
    };
    for pane in panes {
        if !pane_is_visible(server, pane) {
            continue;
        }
        if in_scrollbar_area(server, pane, px, py) {
            if let Some(p) = server.panes.get_mut(pane) {
                p.scrollbar_hover = true;
            }
            let _ = pane_scrollbar_show(server, pane, true);
        } else {
            if let Some(p) = server.panes.get_mut(pane) {
                p.scrollbar_hover = false;
            }
            scrollbar_start_timer(server, pane);
        }
    }
}

/// `window_pane_scrollbar_start_timer` (`window.c:2666-2680`): only an
/// auto-hide scrollbar that is currently shown restarts its hide timer.
/// `pane_scrollbar_show(.., true)` re-arms the same `pane-scrollbars-timeout`
/// timer without changing visibility when the bar is already visible.
fn scrollbar_start_timer(server: &mut Server, pane: PaneId) {
    if pane_scrollbar_overlay(server, pane)
        && server.panes.get(pane).is_some_and(|p| p.scrollbar_visible)
    {
        let _ = pane_scrollbar_show(server, pane, true);
    }
}

/// `window_pane_status_get_range` (`window.c:2886-2909`): the range at `x`
/// on the pane's border status row; the border formats start two cells in
/// but the stored bounds do not reflect that.
fn pane_status_get_range(
    server: &Server,
    pane: PaneId,
    x: u32,
    y: u32,
) -> Option<rmux_emu::style::StyleRange> {
    let p = server.panes.get(pane)?;
    let line = match pane_get_pane_status(server, pane) {
        1 => p.yoff - 1,
        2 => p.yoff + p.sy as i32,
        _ => return None,
    };
    if line != y as i32 {
        return None;
    }
    p.border_status_line
        .ranges
        .get_range(x.wrapping_sub(p.xoff as u32).wrapping_sub(2))
        .cloned()
}

/// Where inside a scrollbar strip the row falls; the slider includes both
/// its top and bottom row and reports `py - slider_y - yoff`
/// (`server-client.c:642-651`, `673-682`).
pub fn slider_location(g: &PaneGeometry, py: i32) -> (MouseLocation, Option<u32>) {
    let sl_top = g.yoff + g.slider_y as i32;
    let sl_bottom = g.yoff + g.slider_y as i32 + g.slider_h as i32 - 1;
    if py < sl_top {
        (MouseLocation::ScrollbarUp, None)
    } else if py <= sl_bottom {
        (
            MouseLocation::ScrollbarSlider,
            Some((py - g.slider_y as i32 - g.yoff) as u32),
        )
    } else {
        (MouseLocation::ScrollbarDown, None)
    }
}

/// Overlay scrollbar test (`server-client.c:631-654`): `sb_w` is the width
/// already clipped to the pane width. `None` when the point is not inside
/// the pane rectangle (the overlay case does not apply); `Some(Pane)` when
/// inside the pane but off the strip.
pub fn overlay_scrollbar_hit(
    g: &PaneGeometry,
    sb_w: i32,
    px: i32,
    py: i32,
) -> Option<(MouseLocation, Option<u32>)> {
    if sb_w == 0
        || !(py >= g.yoff && py < g.yoff + g.sy as i32 && px >= g.xoff && px < g.xoff + g.sx as i32)
    {
        return None;
    }
    let (sb_start, sb_end) = if g.sb_left {
        (g.xoff, g.xoff + sb_w - 1)
    } else {
        let end = g.xoff + g.sx as i32 - 1;
        (end - sb_w + 1, end)
    };
    if px >= sb_start && px <= sb_end {
        Some(slider_location(g, py))
    } else {
        Some((MouseLocation::Pane, None))
    }
}

/// Row test of the pane-or-reserved-scrollbar rectangle
/// (`server-client.c:657-660`).
pub fn in_pane_rows(g: &PaneGeometry, pane_status: i64, pane_status_line: i32, py: i32) -> bool {
    (pane_status != PANE_STATUS_OFF && py != pane_status_line && py != g.yoff + g.sy as i32)
        || (g.yoff == 0 && py < g.sy as i32)
        || (py >= g.yoff && py < g.yoff + g.sy as i32)
}

/// Column test of the pane-or-reserved-scrollbar rectangle
/// (`server-client.c:661-664`).
pub fn in_pane_columns(g: &PaneGeometry, sb_w: i32, sb_pad: i32, px: i32) -> bool {
    if g.sb_left {
        px < g.xoff + g.sx as i32 - sb_pad - sb_w
    } else {
        px < g.xoff + g.sx as i32 + sb_pad + sb_w
    }
}

/// Reserved (non-overlay) scrollbar strip (`server-client.c:666-671`).
pub fn in_reserved_scrollbar(g: &PaneGeometry, sb_w: i32, sb_pad: i32, px: i32) -> bool {
    if g.sb_left {
        px >= g.xoff - sb_pad - sb_w && px < g.xoff - sb_pad
    } else {
        px >= g.xoff + g.sx as i32 + sb_pad && px < g.xoff + g.sx as i32 + sb_pad + sb_w
    }
}

/// Is the mouse inside a pane? (`server-client.c:596-741`)
fn check_mouse_in_pane(
    server: &Server,
    pane: PaneId,
    px: i32,
    py: i32,
    sl_mpos: &mut u32,
) -> MouseLocation {
    let Some(p) = server.panes.get(pane) else {
        return MouseLocation::Nowhere;
    };
    let window = p.window;
    let Some(g) = pane_geometry(server, pane) else {
        return MouseLocation::Nowhere;
    };
    let pane_status = pane_get_pane_status(server, pane);
    let sb_overlay = pane_scrollbar_overlay(server, pane);

    let (mut sb_w, sb_pad) = if pane_scrollbar_visible(server, pane) {
        (g.sb_width, g.sb_pad)
    } else {
        (0, 0)
    };
    if sb_overlay && sb_w > g.sx as i32 {
        sb_w = g.sx as i32;
    }

    let pane_status_line = if pane_status == PANE_STATUS_TOP {
        g.yoff - 1
    } else if pane_status == PANE_STATUS_BOTTOM {
        g.yoff + g.sy as i32
    } else {
        -1 /* not used */
    };
    let mut bdr_left = g.xoff - 1;
    if !sb_overlay && g.sb_left {
        bdr_left -= sb_pad + sb_w;
    }

    if sb_overlay {
        if let Some((loc, mpos)) = overlay_scrollbar_hit(&g, sb_w, px, py) {
            if let Some(mpos) = mpos {
                *sl_mpos = mpos;
            }
            return loc;
        }
    }

    /* Check if point is within the pane or scrollbar. */
    if in_pane_rows(&g, pane_status, pane_status_line, py) && in_pane_columns(&g, sb_w, sb_pad, px)
    {
        if in_reserved_scrollbar(&g, sb_w, sb_pad, px) {
            /* Check where inside the scrollbar. */
            let (loc, mpos) = slider_location(&g, py);
            if let Some(mpos) = mpos {
                *sl_mpos = mpos;
            }
            return loc;
        } else if pane_is_floating(server, pane)
            && pane_get_pane_lines(server, pane) != PANE_LINES_NONE
            && (px == bdr_left || py == g.yoff - 1 || py == g.yoff + g.sy as i32)
        {
            /* Floating pane left, bottom or top border. */
            return MouseLocation::Border;
        } else {
            /* Must be inside the pane. */
            return MouseLocation::Pane;
        }
    }

    /* Try the pane borders (`:695-738`): first hit in pane-list order. */
    let Some(w) = server.windows.get(window) else {
        return MouseLocation::Nowhere;
    };
    let wp_floating = pane_is_floating(server, pane);
    for &fwp in &w.panes {
        if !pane_is_visible(server, fwp) {
            continue;
        }
        if pane_is_floating(server, fwp) && pane_get_pane_lines(server, fwp) == PANE_LINES_NONE {
            continue;
        }
        let Some(fg) = pane_geometry(server, fwp) else {
            continue;
        };
        let (sb_w, sb_pad) = if pane_scrollbar_reserve(server, fwp) {
            (fg.sb_width, fg.sb_pad)
        } else {
            (0, 0)
        };
        if border_hit(&fg, sb_w, sb_pad, wp_floating, px, py) {
            return MouseLocation::Border;
        }
    }
    MouseLocation::Nowhere
}

/// One iteration of the border search (`server-client.c:709-735`) for the
/// pane `fg`; `wp_floating` is `window_pane_is_floating(wp)` of the pane
/// being tested, as in C.
pub fn border_hit(
    fg: &PaneGeometry,
    sb_w: i32,
    sb_pad: i32,
    wp_floating: bool,
    px: i32,
    py: i32,
) -> bool {
    let bdr_top = fg.yoff - 1;
    let mut bdr_left = fg.xoff - 1;
    let bdr_right = if fg.sb_left {
        bdr_left -= sb_pad + sb_w;
        fg.xoff + fg.sx as i32
    } else {
        /* PANE_SCROLLBARS_RIGHT or none. */
        fg.xoff + fg.sx as i32 + sb_pad + sb_w
    };
    if py >= fg.yoff - 1 && py <= fg.yoff + fg.sy as i32 {
        if px == bdr_right {
            return true;
        }
        if wp_floating && px == bdr_left {
            return true;
        }
    }
    if px >= bdr_left && px <= fg.xoff + fg.sx as i32 {
        let bdr_bottom = fg.yoff + fg.sy as i32;
        if py == bdr_bottom || py == bdr_top {
            return true;
        }
    }
    false
}

/// `KEYC_MOUSE_LOCATION_CONTROL0 + n`.
fn control_location(n: u32, fallback: MouseLocation) -> MouseLocation {
    MouseLocation::try_from(MouseLocation::Control0 as i32 + n as i32).unwrap_or(fallback)
}

/// Clear all drag state (`server-client.c:952-957`).
fn reset_drag(drag: &mut MouseDragState) {
    drag.update = None;
    drag.release = None;
    drag.flag = 0;
    drag.scrolling = false;
    drag.slider_mpos = None;
    drag.last_pane = None;
}

/// Check for mouse keys (`server-client.c:743-1143`). Fills `event.target`
/// and returns the mouse key, or `KEYC_UNKNOWN` to drop the event.
pub fn check_mouse(server: &mut Server, id: ClientId, event: &mut KeyEvent) -> KeyCode {
    let unknown = KeyCode(SpecialKey::UNKNOWN);
    let Some(c) = server.clients.get(id) else {
        return unknown;
    };
    let Some(s) = c.session else {
        return unknown;
    };
    let Some(w) = server
        .sessions
        .get(s)
        .and_then(|session| session.current)
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.window)
    else {
        return unknown;
    };
    let m = event.mouse;
    log_debug!(
        "{} mouse {:02x} at {},{} (last {},{}) ({})",
        String::from_utf8_lossy(c.name_bytes()),
        m.b,
        m.x,
        m.y,
        m.lx,
        m.ly,
        c.drag.flag
    );

    /* Find last pane, if any. */
    let lwp = c.drag.last_pane.filter(|p| server.panes.get(*p).is_some());
    if let Some(p) = lwp.and_then(|p| server.panes.get(p)) {
        log_debug!(
            "{} mouse last pane %{}",
            String::from_utf8_lossy(c.name_bytes()),
            p.public_id
        );
    }

    /* What type of event is this? */
    let typing = classify(c.flags, &c.drag, &m, event.key);
    if typing.kind == MouseType::Unknown {
        return unknown;
    }
    {
        let Server {
            clients,
            event_loop,
            ..
        } = server;
        let Some(c) = clients.get_mut(id) else {
            return unknown;
        };
        if typing.cancel_click_timer {
            if let Some(t) = c.click.timer.take() {
                event_loop.cancel(t);
            }
        }
        c.flags = typing.flags;
    }
    let MouseTyping {
        mut kind,
        x,
        y,
        b,
        ignore,
        ..
    } = typing;

    /* Save the session. */
    let mut target = MouseTarget {
        session: Some(s),
        window: None,
        pane: None,
        ignore,
        valid: true,
        ..MouseTarget::default()
    };
    let mut loc = MouseLocation::Nowhere;
    let mut sl_mpos: u32 = 0;
    let mut wp: Option<PaneId> = None;
    let mut modal_drag = false;
    let (mut px, mut py) = (0u32, 0u32);

    /* Is this on the status line? */
    target.status_at = status_at_line(server, id);
    target.status_lines = status_line_size(server, id);
    if target.status_at != -1
        && y >= target.status_at as u32
        && y < target.status_at as u32 + target.status_lines
    {
        let range = server
            .clients
            .get(id)
            .and_then(|c| status_get_range(c, x, y - target.status_at as u32).cloned());
        match range {
            None => loc = MouseLocation::StatusDefault,
            Some(sr) => match sr.range_type {
                StyleRangeType::None => return unknown,
                StyleRangeType::Left => {
                    log_debug!("mouse range: left");
                    loc = MouseLocation::StatusLeft;
                }
                StyleRangeType::Right => {
                    log_debug!("mouse range: right");
                    loc = MouseLocation::StatusRight;
                }
                StyleRangeType::Pane => {
                    let Some(fwp) = pane_find_by_public_id(server, sr.argument) else {
                        return unknown;
                    };
                    target.pane = Some(fwp);
                    log_debug!("mouse range: pane %{}", sr.argument);
                    loc = MouseLocation::Status;
                }
                StyleRangeType::Window => {
                    let Ok(index) = i32::try_from(sr.argument) else {
                        return unknown;
                    };
                    let Some(fwl) = winlink_find_by_index(server, s, index) else {
                        return unknown;
                    };
                    let Some(fw) = server.winlinks.get(fwl).map(|wl| wl.window) else {
                        return unknown;
                    };
                    target.window = Some(fw);
                    log_debug!(
                        "mouse range: window @{}",
                        server.windows.get(fw).map_or(0, |w| w.public_id)
                    );
                    loc = MouseLocation::Status;
                }
                StyleRangeType::Session => {
                    let Some(fs) = session_find_by_id(server, sr.argument) else {
                        return unknown;
                    };
                    target.session = Some(fs);
                    log_debug!("mouse range: session ${}", sr.argument);
                    loc = MouseLocation::Status;
                }
                StyleRangeType::User => {
                    log_debug!("mouse range: user");
                    loc = MouseLocation::Status;
                }
                StyleRangeType::Control => {
                    let n = sr.argument; /* parsing keeps this < 10 */
                    log_debug!("mouse range: control {}", n);
                    loc = control_location(n, loc);
                }
            },
        }
    }

    /*
     * Not on status line. Adjust position and check for border, pane, or
     * scrollbar.
     */
    let scrolling = server.clients.get(id).is_some_and(|c| c.drag.scrolling);
    if loc == MouseLocation::Nowhere && scrolling {
        if let Some(l) = lwp {
            loc = MouseLocation::ScrollbarSlider;
            target.pane = Some(l);
            target.window = server.panes.get(l).map(|p| p.window);
        }
    } else if loc == MouseLocation::Nowhere {
        px = x;
        py = if target.status_at == 0 && y >= target.status_lines {
            y - target.status_lines
        } else if target.status_at > 0 && y >= target.status_at as u32 {
            target.status_at as u32 - 1
        } else {
            y
        };

        /* `tty_window_offset(&c->tty, ...)` (`:929`). */
        let (_, ox, oy, sx, sy) = crate::client::lifecycle::window_offset(server, id);
        target.ox = ox;
        target.oy = oy;
        log_debug!(
            "mouse window @{} at {},{} ({}x{})",
            server.windows.get(w).map_or(0, |w| w.public_id),
            target.ox,
            target.oy,
            sx,
            sy
        );
        if px > sx || py > sy {
            update_scrollbar_hover(server, w, kind, None);
            return unknown;
        }
        px += target.ox;
        py += target.oy;
        let modal = server.windows.get(w).and_then(|w| w.modal);
        if let Some(modal) = modal.filter(|modal| !pane_contains(server, *modal, px, py)) {
            let drag_flag = server.clients.get(id).map_or(0, |c| c.drag.flag);
            if lwp == Some(modal)
                && drag_flag != 0
                && (kind == MouseType::Drag || kind == MouseType::Up)
            {
                modal_drag = true;
                wp = Some(modal);
                loc = MouseLocation::Pane;
                target.pane = Some(modal);
                target.window = server.panes.get(modal).map(|p| p.window);
            } else {
                update_scrollbar_hover(server, w, kind, None);
                if let Some(c) = server.clients.get_mut(id) {
                    reset_drag(&mut c.drag);
                }
                let close = server
                    .panes
                    .get(modal)
                    .is_some_and(|p| p.flags.contains(PaneFlags::CLOSEONCLICK));
                if close
                    && (kind == MouseType::Down
                        || kind == MouseType::Second
                        || kind == MouseType::Triple)
                {
                    let _ = server_kill_pane(server, modal);
                }
                return unknown;
            }
        }
        update_scrollbar_hover(server, w, kind, Some((px as i32, py as i32)));

        if modal_drag {
            /* Keep the drag with the modal pane. */
        } else if kind == MouseType::Drag && lwp.is_some() {
            /* Use pane from last mouse event. */
            wp = lwp;
        } else {
            /* Try inside the pane. */
            wp = window_get_active_at(server, w, px, py);
        }
        match wp {
            None => {
                loc = MouseLocation::Empty;
                target.window = Some(w);
                log_debug!("mouse {},{} on empty area", x, y);
            }
            Some(pane) => {
                if !modal_drag {
                    loc = check_mouse_in_pane(server, pane, px as i32, py as i32, &mut sl_mpos);
                }
                let public_id = server.panes.get(pane).map_or(0, |p| p.public_id);
                if loc == MouseLocation::Pane {
                    log_debug!("mouse {},{} on pane %{}", x, y, public_id);
                } else if loc == MouseLocation::Border {
                    if let Some(sr) = pane_status_get_range(server, pane, px, py) {
                        loc = control_location(sr.argument, loc);
                    }
                    log_debug!("mouse on pane %{} border", public_id);
                } else if matches!(
                    loc,
                    MouseLocation::ScrollbarUp
                        | MouseLocation::ScrollbarSlider
                        | MouseLocation::ScrollbarDown
                ) {
                    log_debug!("mouse on pane %{} scrollbar", public_id);
                }
                target.pane = Some(pane);
                target.window = server.panes.get(pane).map(|p| p.window);
            }
        }
    } else {
        update_scrollbar_hover(server, w, kind, None);
    }

    /* Reset click type or add a click timer if needed (`:1008-1034`). */
    if kind == MouseType::Down || kind == MouseType::Second || kind == MouseType::Triple {
        let Server {
            clients,
            event_loop,
            ..
        } = server;
        let Some(c) = clients.get_mut(id) else {
            return unknown;
        };
        if kind != MouseType::Down
            && (m.b != c.click.button || loc != c.click.location || target.pane != c.click.pane)
        {
            kind = MouseType::Down;
            log_debug!("click sequence reset at {},{}", x, y);
            c.flags.remove(ClientFlags::TRIPLECLICK);
            c.flags.insert(ClientFlags::DOUBLECLICK);
        }

        if kind != MouseType::Triple && KeyCode::CLICK_TIMEOUT != 0 {
            c.click.event = ResolvedMouseEvent { event: m, target };
            c.click.button = m.b;
            c.click.location = loc;
            c.click.pane = target.pane;

            log_debug!("click timer started");
            if let Some(t) = c.click.timer.take() {
                event_loop.cancel(t);
            }
            c.click.timer = Some(event_loop.schedule(
                Duration::from_millis(u64::from(KeyCode::CLICK_TIMEOUT)),
                LoopAction::ClientClickTimer(id),
            ));
        }
    }

    let mut key = unknown;

    /* Stop dragging if needed (`:1038-1060`). */
    let drag_flag = server.clients.get(id).map_or(0, |c| c.drag.flag);
    if kind != MouseType::Drag
        && kind != MouseType::WheelUp
        && kind != MouseType::WheelDown
        && kind != MouseType::Double
        && kind != MouseType::Triple
        && drag_flag != 0
    {
        let release = server.clients.get(id).and_then(|c| c.drag.release);
        if let Some(action) = release {
            action.release(server, id, &ResolvedMouseEvent { event: m, target });
        }
        let Some(c) = server.clients.get_mut(id) else {
            return unknown;
        };
        c.drag.update = None;
        c.drag.release = None;
        c.drag.scrolling = false;

        /*
         * End a mouse drag by passing a MouseDragEnd key corresponding
         * to the button that started the drag.
         */
        kind = MouseType::DragEnd;
        c.drag.flag = 0;
        c.drag.slider_mpos = None;
        c.drag.last_pane = None;
    }

    /* Convert to a key binding (`:1062-1105`). */
    if kind == MouseType::Move && loc == MouseLocation::Pane {
        key = KeyCode(SpecialKey::MOUSEMOVE_PANE);
        let active = server.windows.get(w).and_then(|w| w.active);
        if let Some(pane) = wp.filter(|p| Some(*p) != active) {
            let follows = server.sessions.get(s).is_some_and(|session| {
                server
                    .options
                    .get_number(session.options, b"focus-follows-mouse")
                    != 0
            });
            if follows {
                let _ = window_redraw_active_switch(server, w, Some(pane));
                let _ = window_set_active_pane(server, w, pane, true);
                server_redraw_window_borders(server, w);
                server_status_window(server, w);
            }
        }
    }
    if kind == MouseType::Drag {
        let Some(c) = server.clients.get_mut(id) else {
            return unknown;
        };
        if c.drag.update.is_some() {
            key = KeyCode(SpecialKey::DRAGGING);
        }

        /*
         * Begin a drag by setting the flag to a non-zero value that
         * corresponds to the mouse button in use. If starting to drag
         * the scrollbar, store the relative position in the slider
         * where the user grabbed.
         */
        if c.drag.flag == 0 {
            c.drag.x = px;
            c.drag.y = py;
        }
        c.drag.flag = MouseButtonBits(b).buttons() + 1;

        /* Only change pane if not already dragging a pane border. */
        if lwp.is_none() {
            let active = window_get_active_at(server, w, px, py);
            if let Some(c) = server.clients.get_mut(id) {
                c.drag.last_pane = active;
            }
        }
        if let Some(c) = server.clients.get_mut(id) {
            if !c.drag.scrolling && loc == MouseLocation::ScrollbarSlider {
                c.drag.scrolling = true;
                c.drag.slider_mpos = Some(if target.status_at == 0 {
                    sl_mpos + target.status_lines
                } else {
                    sl_mpos
                });
            }
        }
    }

    if key == unknown {
        /* Adjust the button number (`:1107-1129`). */
        let bn = match MouseButtonBits(b).buttons() {
            x if x == MouseButton::Button1 as u32 => 1,
            x if x == MouseButton::Button2 as u32 => 2,
            x if x == MouseButton::Button3 as u32 => 3,
            x if x == MouseButton::Button6 as u32 => 6,
            x if x == MouseButton::Button7 as u32 => 7,
            x if x == MouseButton::Button8 as u32 => 8,
            x if x == MouseButton::Button9 as u32 => 9,
            x if x == MouseButton::Button10 as u32 => 10,
            x if x == MouseButton::Button11 as u32 => 11,
            _ => 0,
        };
        key = KeyCode::mouse(kind.key_type(), bn, loc);
    }

    /* Apply modifiers if any (`:1132-1138`). */
    key = apply_modifiers(key, b);

    if rmux_util::log::level() != rmux_util::log::LogLevel(0) {
        log_debug!("mouse key is {:#x}", key.0);
    }
    target.key = key;
    event.target = target;
    key
}

/// `MOUSE_MASK_META/CTRL/SHIFT` to `KEYC_META/CTRL/SHIFT` (`server-client.c:1132-1138`).
pub fn apply_modifiers(mut key: KeyCode, b: u32) -> KeyCode {
    let bits = MouseButtonBits(b);
    if bits.intersects(MouseButtonBits::META) {
        key.0 |= KeyModifiers::META.bits();
    }
    if bits.intersects(MouseButtonBits::CTRL) {
        key.0 |= KeyModifiers::CTRL.bits();
    }
    if bits.intersects(MouseButtonBits::SHIFT) {
        key.0 |= KeyModifiers::SHIFT.bits();
    }
    key
}

/// Double-click callback (`server-client.c:2194-2216`), run by
/// `LoopAction::ClientClickTimer`.
pub fn click_timer(server: &mut Server, id: ClientId) {
    log_debug!("click timer expired");

    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    c.click.timer = None;
    if c.flags.intersects(ClientFlags::TRIPLECLICK) {
        /*
         * Waiting for a third click that hasn't happened, so this must
         * have been a double click.
         */
        let event = KeyEvent {
            client: None,
            key: KeyCode(SpecialKey::DOUBLECLICK),
            mouse: c.click.event.event,
            target: c.click.event.target,
            paste: None,
        };
        crate::client::keys::handle_key(server, id, event);
    }
    if let Some(c) = server.clients.get_mut(id) {
        c.flags
            .remove(ClientFlags::DOUBLECLICK | ClientFlags::TRIPLECLICK);
    }
}

/// Remove pane from client state (`server-client.c:3066-3078`).
pub fn remove_pane(server: &mut Server, pane: PaneId) {
    for id in server.client_order.clone() {
        if let Some(c) = server.clients.get_mut(id) {
            if c.drag.last_pane == Some(pane) {
                c.drag.last_pane = None;
                c.drag.update = None;
                c.drag.scrolling = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(b: u32, x: u32, y: u32) -> MouseEvent {
        MouseEvent {
            x,
            y,
            b,
            lx: 0,
            ly: 0,
            lb: 0,
            sgr_type: b' ',
            sgr_b: 0,
        }
    }
    const MOUSE: KeyCode = KeyCode(SpecialKey::MOUSE);

    #[test]
    fn down_sets_doubleclick_flag() {
        let t = classify(
            ClientFlags(0),
            &MouseDragState::default(),
            &ev(0, 5, 6),
            MOUSE,
        );
        assert_eq!(t.kind, MouseType::Down);
        assert_eq!((t.x, t.y, t.b), (5, 6, 0));
        assert!(t.flags.contains(ClientFlags::DOUBLECLICK));
        assert!(!t.cancel_click_timer);
        assert!(!t.ignore);
    }

    #[test]
    fn second_and_triple_click_flags() {
        let t = classify(
            ClientFlags::DOUBLECLICK,
            &MouseDragState::default(),
            &ev(0, 1, 1),
            MOUSE,
        );
        assert_eq!(t.kind, MouseType::Second);
        assert!(t.cancel_click_timer);
        assert!(!t.flags.contains(ClientFlags::DOUBLECLICK));
        assert!(t.flags.contains(ClientFlags::TRIPLECLICK));

        let t = classify(
            ClientFlags::TRIPLECLICK,
            &MouseDragState::default(),
            &ev(0, 1, 1),
            MOUSE,
        );
        assert_eq!(t.kind, MouseType::Triple);
        assert!(t.cancel_click_timer);
        assert_eq!(t.flags, ClientFlags(0));
    }

    #[test]
    fn doubleclick_key_is_ignored_double() {
        let t = classify(
            ClientFlags(0),
            &MouseDragState::default(),
            &ev(2, 3, 4),
            KeyCode(SpecialKey::DOUBLECLICK),
        );
        assert_eq!(t.kind, MouseType::Double);
        assert!(t.ignore);
        assert_eq!((t.x, t.y, t.b), (3, 4, 2));
    }

    #[test]
    fn drag_start_uses_last_position_and_button() {
        let mut m = ev(32, 10, 10);
        m.lx = 4;
        m.ly = 5;
        m.lb = 1;
        let t = classify(ClientFlags(0), &MouseDragState::default(), &m, MOUSE);
        assert_eq!(t.kind, MouseType::Drag);
        assert_eq!((t.x, t.y, t.b), (4, 5, 1));
    }

    #[test]
    fn drag_update_same_position_is_unknown() {
        let mut m = ev(32, 10, 10);
        m.lx = 10;
        m.ly = 10;
        let drag = MouseDragState {
            flag: 1,
            ..MouseDragState::default()
        };
        assert_eq!(
            classify(ClientFlags(0), &drag, &m, MOUSE).kind,
            MouseType::Unknown
        );
        m.lx = 9;
        let t = classify(ClientFlags(0), &drag, &m, MOUSE);
        assert_eq!(t.kind, MouseType::Drag);
        assert_eq!((t.x, t.y, t.b), (10, 10, 32));
    }

    #[test]
    fn wheel_direction() {
        assert_eq!(
            classify(
                ClientFlags(0),
                &MouseDragState::default(),
                &ev(64, 0, 0),
                MOUSE
            )
            .kind,
            MouseType::WheelUp
        );
        assert_eq!(
            classify(
                ClientFlags(0),
                &MouseDragState::default(),
                &ev(65 | 16, 0, 0),
                MOUSE
            )
            .kind,
            MouseType::WheelDown
        );
    }

    #[test]
    fn release_uses_last_button_or_sgr_button() {
        let mut m = ev(3, 1, 1);
        m.lb = 2;
        let t = classify(ClientFlags(0), &MouseDragState::default(), &m, MOUSE);
        assert_eq!(t.kind, MouseType::Up);
        assert_eq!(t.b, 2);
        m.sgr_type = b'm';
        m.sgr_b = 66;
        let t = classify(ClientFlags(0), &MouseDragState::default(), &m, MOUSE);
        assert_eq!(t.kind, MouseType::Up);
        assert_eq!(t.b, 66);
    }

    #[test]
    fn move_detection() {
        // SGR release with drag bit.
        let mut m = ev(0, 7, 8);
        m.sgr_type = b'm';
        m.sgr_b = 32 | 3;
        let t = classify(ClientFlags(0), &MouseDragState::default(), &m, MOUSE);
        assert_eq!(t.kind, MouseType::Move);
        assert_eq!(t.b, 0);
        // Non-SGR drag+release with released last button.
        let mut m = ev(32 | 3, 7, 8);
        m.lb = 3;
        assert_eq!(
            classify(ClientFlags(0), &MouseDragState::default(), &m, MOUSE).kind,
            MouseType::Move
        );
        // Non-SGR drag+release without released last button is a drag.
        m.lb = 0;
        assert_eq!(
            classify(ClientFlags(0), &MouseDragState::default(), &m, MOUSE).kind,
            MouseType::Drag
        );
    }

    fn geom(left: bool) -> PaneGeometry {
        PaneGeometry {
            xoff: 10,
            yoff: 2,
            sx: 20,
            sy: 10,
            sb_width: 1,
            sb_pad: 1,
            sb_left: left,
            slider_y: 3,
            slider_h: 4,
        }
    }

    #[test]
    fn scrollbar_area_bounds() {
        let g = geom(false);
        // Strip is width+pad = 2 columns at the right edge: 28..=29.
        assert!(scrollbar_area_contains(&g, 28, 5));
        assert!(scrollbar_area_contains(&g, 29, 5));
        assert!(!scrollbar_area_contains(&g, 27, 5));
        assert!(!scrollbar_area_contains(&g, 30, 5));
        assert!(!scrollbar_area_contains(&g, 29, 1));
        assert!(!scrollbar_area_contains(&g, 29, 12));
        assert!(!scrollbar_area_contains(&g, -1, -1));
        let g = geom(true);
        assert!(scrollbar_area_contains(&g, 10, 5));
        assert!(scrollbar_area_contains(&g, 11, 5));
        assert!(!scrollbar_area_contains(&g, 12, 5));
        // Zero total clips to the pane width.
        let g = PaneGeometry {
            sb_width: 0,
            sb_pad: 0,
            ..geom(false)
        };
        assert!(scrollbar_area_contains(&g, 10, 5));
        assert!(scrollbar_area_contains(&g, 29, 5));
    }

    #[test]
    fn slider_rows_include_top_and_bottom() {
        let g = geom(false);
        // slider rows: yoff+3 .. yoff+3+4-1 = 5..=8
        assert_eq!(slider_location(&g, 4), (MouseLocation::ScrollbarUp, None));
        assert_eq!(
            slider_location(&g, 5),
            (MouseLocation::ScrollbarSlider, Some(0))
        );
        assert_eq!(
            slider_location(&g, 8),
            (MouseLocation::ScrollbarSlider, Some(3))
        );
        assert_eq!(slider_location(&g, 9), (MouseLocation::ScrollbarDown, None));
    }

    #[test]
    fn overlay_strip_excludes_pad() {
        let g = geom(false);
        // Overlay strip is only the width column (29), not the pad (28).
        assert_eq!(
            overlay_scrollbar_hit(&g, 1, 29, 6),
            Some((MouseLocation::ScrollbarSlider, Some(1)))
        );
        assert_eq!(
            overlay_scrollbar_hit(&g, 1, 28, 6),
            Some((MouseLocation::Pane, None))
        );
        assert_eq!(overlay_scrollbar_hit(&g, 1, 30, 6), None);
        assert_eq!(overlay_scrollbar_hit(&g, 0, 29, 6), None);
        let g = geom(true);
        assert_eq!(
            overlay_scrollbar_hit(&g, 1, 10, 4),
            Some((MouseLocation::ScrollbarUp, None))
        );
        assert_eq!(
            overlay_scrollbar_hit(&g, 1, 11, 4),
            Some((MouseLocation::Pane, None))
        );
    }

    #[test]
    fn reserved_strip_offsets_differ_by_side() {
        let g = geom(false);
        // Right: [xoff+sx+pad, xoff+sx+pad+w) = [31, 32)
        assert!(in_reserved_scrollbar(&g, 1, 1, 31));
        assert!(!in_reserved_scrollbar(&g, 1, 1, 30));
        assert!(!in_reserved_scrollbar(&g, 1, 1, 32));
        assert!(in_pane_columns(&g, 1, 1, 31));
        assert!(!in_pane_columns(&g, 1, 1, 32));
        let g = geom(true);
        // Left: [xoff-pad-w, xoff-pad) = [8, 9)
        assert!(in_reserved_scrollbar(&g, 1, 1, 8));
        assert!(!in_reserved_scrollbar(&g, 1, 1, 9));
        assert!(!in_reserved_scrollbar(&g, 1, 1, 7));
        assert!(in_pane_columns(&g, 1, 1, 27));
        assert!(!in_pane_columns(&g, 1, 1, 28));
    }

    #[test]
    fn pane_row_tests() {
        let g = geom(false);
        assert!(in_pane_rows(&g, PANE_STATUS_OFF, -1, 2));
        assert!(in_pane_rows(&g, PANE_STATUS_OFF, -1, 11));
        assert!(!in_pane_rows(&g, PANE_STATUS_OFF, -1, 12));
        assert!(!in_pane_rows(&g, PANE_STATUS_OFF, -1, 1));
        // With a top status line, every row except the status and bottom border counts.
        assert!(in_pane_rows(&g, PANE_STATUS_TOP, 1, 0));
        assert!(!in_pane_rows(&g, PANE_STATUS_TOP, 1, 1));
        assert!(!in_pane_rows(&g, PANE_STATUS_TOP, 1, 12));
        // yoff == 0 accepts rows below sy.
        let g0 = PaneGeometry { yoff: 0, ..g };
        assert!(in_pane_rows(&g0, PANE_STATUS_OFF, -1, 9));
        assert!(!in_pane_rows(&g0, PANE_STATUS_OFF, -1, 10));
    }

    #[test]
    fn border_hits() {
        let g = geom(false);
        // Right border with reserved scrollbar: xoff+sx+pad+w = 32.
        assert!(border_hit(&g, 1, 1, false, 32, 5));
        assert!(!border_hit(&g, 1, 1, false, 32, 13));
        // Left border only for floating panes.
        assert!(!border_hit(&g, 1, 1, false, 9, 5));
        assert!(border_hit(&g, 1, 1, true, 9, 5));
        // Top and bottom borders.
        assert!(border_hit(&g, 0, 0, false, 15, 1));
        assert!(border_hit(&g, 0, 0, false, 15, 12));
        assert!(!border_hit(&g, 0, 0, false, 15, 13));
        let g = geom(true);
        // Left position: bdr_left = xoff-1-pad-w = 7, bdr_right = xoff+sx = 30.
        assert!(border_hit(&g, 1, 1, true, 7, 5));
        assert!(border_hit(&g, 1, 1, false, 30, 5));
    }

    #[test]
    fn modifiers_from_button_bits() {
        let key = apply_modifiers(
            KeyCode::mouse(KeyCodeType::Mousedown, 1, MouseLocation::Pane),
            8 | 16 | 4,
        );
        assert_eq!(
            key.0 & KeyModifiers::META.bits()
                | key.0 & KeyModifiers::CTRL.bits()
                | key.0 & KeyModifiers::SHIFT.bits(),
            (KeyModifiers::META | KeyModifiers::CTRL | KeyModifiers::SHIFT).bits()
        );
        assert_eq!(
            key.0 & rmux_util::key::KeyMasks::KEY,
            KeyCode::mouse(KeyCodeType::Mousedown, 1, MouseLocation::Pane).0
        );
    }
}
