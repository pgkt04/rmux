// Ported from tmux tty.c (tty_write, tty_client_ready, tty_default_colours,
// tty_style_changed) and screen-write.c (screen_write_set_client_cb,
// screen_write_initctx, screen_write_redraw_cb, synchronized-update dirty
// tracking and flush) @ 8f25579c
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

use crate::client::{Client, ClientFlags};
use crate::format::{FormatFlags, FormatTagFlags, FormatTree};
use crate::ids::{Arena, ClientId, PaneId, WindowId};
use crate::model::pane::{pane_floating_overlaps, pane_scrollbar_overlay, pane_scrollbar_visible};
use crate::model::{PaneFlags, Server};
use crate::ui::redraw::redraw_damage_window;
use crate::ui::status;
use crate::ui::styles;
use crate::ui::visible::{VisibilityModel, VisibleRanges};
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::{Colour, ColourPalette};
use rmux_emu::screen::write::{DrawCommand, DrawOp, DrawSnapshot, ScreenRenderEffects, TtySink};
use rmux_emu::screen::{Screen, ScreenMode};
use rmux_tty::draw::{TtyCommand, TtyCommandData, TtyCtx, TtyCtxFlags, TtyStyleCtx};
use rmux_tty::term::tparm::TparmState;
use rmux_tty::tty::TtyFlags;
use rmux_util::bitset::BitSet;
use rmux_util::log_debug;
use std::ops::Range;

/// tty_window_default_style.
fn tty_window_default_style(gc: &mut GridCell, palette: &ColourPalette) {
    *gc = DEFAULT_CELL;
    gc.fg = palette.fg;
    gc.bg = palette.bg;
}

/// tty_style_changed: refresh the cached window-style cells of a pane.
fn tty_style_changed(srv: &mut Server, wp: PaneId) {
    let Some(p) = srv.panes.get_mut(wp) else {
        return;
    };
    log_debug!("%{}: style changed", p.public_id);
    p.flags.remove(PaneFlags::STYLECHANGED);
    let (public_id, oo) = (p.public_id, p.options);
    let mut active_gc = DEFAULT_CELL;
    let mut gc = DEFAULT_CELL;
    tty_window_default_style(&mut active_gc, &p.palette);
    tty_window_default_style(&mut gc, &p.palette);

    let mut ft = FormatTree::create(
        None,
        None,
        FormatTagFlags::PANE.bits() | public_id,
        FormatFlags::NOJOBS,
        srv,
    );
    ft.defaults(
        srv,
        crate::format::FormatContext {
            pane: Some(wp),
            ..Default::default()
        },
    );
    let active_dim = styles::option_style(srv, oo, b"window-active-style", Some(&mut ft))
        .map(|sy| {
            sy.overlay_cell(&mut active_gc);
            sy.dim
        })
        .unwrap_or(0);
    let dim = styles::option_style(srv, oo, b"window-style", Some(&mut ft))
        .map(|sy| {
            sy.overlay_cell(&mut gc);
            sy.dim
        })
        .unwrap_or(0);
    ft.release(srv);
    if let Some(p) = srv.panes.get_mut(wp) {
        p.cached_active_gc = active_gc;
        p.cached_active_dim = active_dim;
        p.cached_gc = gc;
        p.cached_dim = dim;
    }
}

/// tty_default_colours: (default cell, dim) for a pane.
pub fn tty_default_colours(srv: &mut Server, wp: PaneId) -> (GridCell, u32) {
    if srv
        .panes
        .get(wp)
        .is_some_and(|p| p.flags.contains(PaneFlags::STYLECHANGED))
    {
        tty_style_changed(srv, wp);
    }
    let Some(p) = srv.panes.get(wp) else {
        return (DEFAULT_CELL, 0);
    };
    let active = srv
        .windows
        .get(p.window)
        .is_some_and(|w| w.active == Some(wp));
    let mut gc = DEFAULT_CELL;
    gc.fg = if active && p.cached_active_gc.fg != Colour::DEFAULT {
        p.cached_active_gc.fg
    } else {
        p.cached_gc.fg
    };
    gc.bg = if active && p.cached_active_gc.bg != Colour::DEFAULT {
        p.cached_active_gc.bg
    } else {
        p.cached_gc.bg
    };
    let dim = if active {
        p.cached_active_dim
    } else {
        p.cached_dim
    };
    (gc, dim)
}

/// Deferred synchronized-update output state (tmux.h sync_dirty fields).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncOutputState {
    pub dirty: Option<BitSet>,
    pub dirty_size: u32,
    pub scrolled: u32,
    pub rupper: u32,
    pub rlower: u32,
    pub bg: Colour,
}

impl SyncOutputState {
    /// screen_write_sync_allocate_dirty.
    fn allocate_dirty(&mut self, sy: u32) -> &mut BitSet {
        if self.dirty.is_none() || self.dirty_size != sy {
            let mut bs = BitSet::new(sy as usize);
            if self.dirty_size != 0 && sy != 0 {
                bs.set_range(0, sy as usize - 1);
                self.scrolled = 0;
            }
            self.dirty = Some(bs);
            self.dirty_size = sy;
        }
        self.dirty.as_mut().expect("dirty set")
    }
    fn clear(&mut self) {
        self.dirty = None;
        self.dirty_size = 0;
        self.scrolled = 0;
    }
}

/// Per-client facts fixed for one parse step.
#[derive(Clone, Copy, Debug)]
struct ClientDrawInfo {
    id: ClientId,
    /// Session attached and not suspended.
    ready: bool,
    /// session_has(c->session, wp->window).
    has_window: bool,
    /// The client's current window is the pane's window.
    shows_window: bool,
    /// status_line_size when the status line is at the top, else 0.
    top_lines: u32,
}

/// Everything tty_write needs about the pane, taken before a parse step.
pub struct PaneDrawSnapshot {
    pub pane: PaneId,
    pub window: WindowId,
    pub xoff: i32,
    pub yoff: i32,
    pub sx: u32,
    pub sy: u32,
    pub tiled: bool,
    pub redraw_pending: bool,
    pub obscured: bool,
    pub scrollbar_overlay_visible: bool,
    /// Always use synchronized updates: not the active pane or showing a
    /// mode screen (screen-write.c initctx).
    pub sync_always: bool,
    pub defaults: GridCell,
    pub dim: u32,
    pub palette: ColourPalette,
    pub visibility: VisibilityModel,
    clients: Vec<ClientDrawInfo>,
}

fn session_has(srv: &Server, session: Option<crate::ids::SessionId>, w: WindowId) -> bool {
    let Some(s) = session.and_then(|s| srv.sessions.get(s)) else {
        return false;
    };
    s.windows
        .values()
        .any(|wl| srv.winlinks.get(*wl).is_some_and(|wl| wl.window == w))
}

/// screen_write_pane_is_obscured.
fn pane_is_obscured(srv: &Server, wp: PaneId) -> bool {
    let Some(p) = srv.panes.get(wp) else {
        return false;
    };
    let Some(w) = srv.windows.get(p.window) else {
        return false;
    };
    if let Some(md) = &w.menu {
        if (md.x() as i32) < p.xoff + p.sx as i32
            && (md.x() + md.width()) as i32 > p.xoff
            && (md.y() as i32) < p.yoff + p.sy as i32
            && (md.y() + md.height()) as i32 > p.yoff
        {
            return true;
        }
    }
    if p.xoff < 0
        || p.yoff < 0
        || p.xoff + p.sx as i32 > w.sx as i32
        || p.yoff + p.sy as i32 > w.sy as i32
    {
        return true;
    }
    for above in &w.z_order {
        if *above == wp {
            break;
        }
        if pane_floating_overlaps(srv, *above, wp) {
            return true;
        }
    }
    false
}

impl PaneDrawSnapshot {
    pub fn capture(srv: &mut Server, wp: PaneId) -> Option<PaneDrawSnapshot> {
        let (defaults, dim) = tty_default_colours(srv, wp);
        let p = srv.panes.get(wp)?;
        let w = srv.windows.get(p.window)?;
        let window = p.window;
        let in_mode = !p.modes.is_empty();
        let sync_always = w.active != Some(wp) || in_mode;
        let mut clients = Vec::with_capacity(srv.client_order.len());
        for &id in &srv.client_order {
            let Some(c) = srv.clients.get(id) else {
                continue;
            };
            let ready = c.session.is_some() && !c.flags.contains(ClientFlags::SUSPENDED);
            let shows_window = styles::client_window(srv, id) == Some(window);
            let top_lines = if status::status_at_line(srv, id) == 0 {
                status::status_line_size(srv, id)
            } else {
                0
            };
            clients.push(ClientDrawInfo {
                id,
                ready,
                has_window: session_has(srv, c.session, window),
                shows_window,
                top_lines,
            });
        }
        let visibility = VisibilityModel::capture(srv, wp)?;
        let p = srv.panes.get(wp)?;
        Some(PaneDrawSnapshot {
            pane: wp,
            window,
            xoff: p.xoff,
            yoff: p.yoff,
            sx: p.sx,
            sy: p.sy,
            tiled: p.layout_cell.is_some(),
            redraw_pending: p.flags.intersects(PaneFlags::REDRAW | PaneFlags::DROP),
            obscured: pane_is_obscured(srv, wp),
            scrollbar_overlay_visible: pane_scrollbar_overlay(srv, wp)
                && pane_scrollbar_visible(srv, wp),
            sync_always,
            defaults,
            dim,
            palette: p.palette.clone(),
            visibility,
            clients,
        })
    }
}

/// The TtySink for one pane during one parse step: owns the clients and
/// the tparm state and writes to every ready client tty (tty_write).
pub struct PaneSink {
    snapshot: PaneDrawSnapshot,
    clients: Arena<Client, ClientId>,
    tparm: TparmState,
    effects: Vec<ScreenRenderEffects>,
    ranges: VisibleRanges,
    muted: bool,
}

/// Which clients a command goes to (screen_write_set_client_cb).
enum Target {
    Skip,
    Stop,
    Draw {
        wox: u32,
        woy: u32,
        wsx: u32,
        wsy: u32,
        bigger: bool,
        yoff: i32,
    },
}

fn select_client(
    snap: &PaneDrawSnapshot,
    info: &ClientDrawInfo,
    client: &Client,
    invisible_panes: bool,
) -> Target {
    if crate::tsp::broker::native_pane_client(client, snap.pane) {
        return Target::Skip;
    }
    // tty_client_ready
    if !info.ready || client.tty.is_none() {
        return Target::Skip;
    }
    let tty = client.tty.as_ref().expect("tty");
    if !invisible_panes
        && (client.flags.contains(ClientFlags::REDRAWWINDOW)
            || tty.flags().contains(TtyFlags::FREEZE))
    {
        return Target::Skip;
    }
    // screen_write_set_client_cb
    if invisible_panes {
        if !info.has_window {
            return Target::Skip;
        }
    } else {
        if !info.shows_window || !snap.tiled {
            return Target::Skip;
        }
        if snap.redraw_pending {
            return Target::Stop;
        }
        if client.flags.contains(ClientFlags::REDRAWWINDOW) {
            return Target::Stop;
        }
    }
    let (bigger, wox, woy, wsx, wsy) = tty.window_offset();
    Target::Draw {
        wox,
        woy,
        wsx,
        wsy,
        bigger,
        yoff: snap.yoff + info.top_lines as i32,
    }
}

impl PaneSink {
    pub fn new(
        snapshot: PaneDrawSnapshot,
        clients: Arena<Client, ClientId>,
        tparm: TparmState,
    ) -> PaneSink {
        PaneSink {
            snapshot,
            clients,
            tparm,
            effects: Vec::new(),
            ranges: VisibleRanges::default(),
            muted: false,
        }
    }

    /// `input_parse_buffer` (`input.c:1062-1066`) parses into the base
    /// screen with no pane while a mode is shown, so nothing reaches a tty.
    pub fn muted(mut self) -> PaneSink {
        self.muted = true;
        self
    }

    pub fn into_parts(
        self,
    ) -> (
        Arena<Client, ClientId>,
        TparmState,
        Vec<ScreenRenderEffects>,
    ) {
        (self.clients, self.tparm, self.effects)
    }

    /// tty_write for one command: build the per-client context and write.
    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        cmd: TtyCommand,
        screen: &Screen,
        hyperlinks: &rmux_emu::hyperlinks::HyperlinkRegistry,
        cell: &GridCell,
        data: TtyCommandData<'_>,
        mut flags: TtyCtxFlags,
        ocx: u32,
        ocy: u32,
        orupper: u32,
        orlower: u32,
        bg: Colour,
        on_redraw: &mut dyn FnMut(u32, u32),
    ) {
        let snap = &self.snapshot;
        let invisible_panes = flags.contains(TtyCtxFlags::INVISIBLE_PANES);
        for info in &snap.clients {
            let Some(client) = self.clients.get_mut(info.id) else {
                continue;
            };
            let target = select_client(snap, info, client, invisible_panes);
            let (wox, woy, wsx, wsy, bigger, yoff) = match target {
                Target::Skip => continue,
                Target::Stop => break,
                Target::Draw {
                    wox,
                    woy,
                    wsx,
                    wsy,
                    bigger,
                    yoff,
                } => (wox, woy, wsx, wsy, bigger, yoff),
            };
            if bigger {
                flags.insert(TtyCtxFlags::WINDOW_BIGGER);
            } else {
                flags.remove(TtyCtxFlags::WINDOW_BIGGER);
            }
            let style_ctx = TtyStyleCtx {
                defaults: &snap.defaults,
                palette: Some(&snap.palette),
                dim: snap.dim,
                hyperlinks: screen.hyperlinks.as_ref().map(|h| (hyperlinks, h)),
            };
            let ctx = TtyCtx {
                s: screen,
                cell,
                flags,
                data,
                ocx,
                ocy,
                orupper,
                orlower,
                xoff: snap.xoff,
                yoff,
                rxoff: snap.xoff,
                ryoff: snap.yoff,
                sx: snap.sx,
                sy: snap.sy,
                bg: bg.0 as u32,
                defaults: snap.defaults,
                style_ctx,
                wox,
                woy,
                wsx,
                wsy,
            };
            let tty = client.tty.as_mut().expect("tty");
            if let Some(redraw) = tty.command(&mut self.tparm, cmd, &ctx) {
                on_redraw(redraw.start_y, redraw.count);
            }
        }
    }
}

impl TtySink for PaneSink {
    fn draw(&mut self, op: DrawOp<'_>, snapshot: &DrawSnapshot) {
        if self.muted {
            return;
        }
        let cmd = TtyCommand::from(&op.command);
        let mut flags = TtyCtxFlags(0);
        if snapshot.wrapped {
            flags.insert(TtyCtxFlags::WRAPPED);
        }
        if snapshot.invalidate_cursor {
            flags.insert(TtyCtxFlags::CELL_INVALIDATE);
        }
        if snapshot.sync || self.snapshot.sync_always {
            flags.insert(TtyCtxFlags::SYNC);
        }
        if self.snapshot.obscured {
            flags.insert(TtyCtxFlags::PANE_OBSCURED);
        }
        let mut cell = &DEFAULT_CELL;
        let mut data = TtyCommandData::Count(0);
        let mut bg = Colour::DEFAULT;
        let mut ocx = snapshot.old_cx;
        let mut ocy = snapshot.old_cy;
        match &op.command {
            DrawCommand::SyncStart | DrawCommand::AlignmentTest => {}
            DrawCommand::Cell(c) => cell = c,
            DrawCommand::Cells { cell: c, data: d } => {
                cell = c;
                data = TtyCommandData::Bytes(d);
            }
            DrawCommand::RedrawLine { start, row, count } => {
                ocx = *start;
                ocy = *row;
                data = TtyCommandData::Count(*count);
            }
            DrawCommand::InsertCharacter { count, bg: b }
            | DrawCommand::DeleteCharacter { count, bg: b }
            | DrawCommand::ClearCharacter { count, bg: b }
            | DrawCommand::InsertLine { count, bg: b }
            | DrawCommand::DeleteLine { count, bg: b }
            | DrawCommand::ScrollUp { count, bg: b }
            | DrawCommand::ScrollDown { count, bg: b } => {
                data = TtyCommandData::Count(*count);
                bg = *b;
            }
            DrawCommand::ClearEndOfScreen { bg: b }
            | DrawCommand::ClearStartOfScreen { bg: b }
            | DrawCommand::ClearScreen { bg: b }
            | DrawCommand::ReverseIndex { bg: b } => bg = *b,
            DrawCommand::SetSelection { selector, data: d } => {
                data = TtyCommandData::Selection {
                    clip: std::str::from_utf8(selector).unwrap_or(""),
                    data: d,
                };
            }
            DrawCommand::RawString {
                data: d,
                allow_invisible,
            } => {
                data = TtyCommandData::Bytes(d);
                if *allow_invisible {
                    flags.insert(TtyCtxFlags::INVISIBLE_PANES);
                }
            }
            #[cfg(feature = "sixel")]
            DrawCommand::SixelImage { image } => {
                data = TtyCommandData::SixelImage(image);
            }
        }
        let mut damage = Vec::new();
        self.write(
            cmd,
            op.screen,
            op.hyperlinks,
            cell,
            data,
            flags,
            ocx,
            ocy,
            snapshot.rupper,
            snapshot.rlower,
            bg,
            &mut |py, ny| damage.push((py, ny)),
        );
        for (py, ny) in damage {
            self.effects.push(ScreenRenderEffects::DamageRows {
                start: py,
                count: ny,
            });
        }
    }

    fn visible_columns(&mut self, x: u32, y: u32, n: u32, out: &mut Vec<Range<u32>>) {
        out.clear();
        let snap = &self.snapshot;
        self.ranges.ranges.clear();
        snap.visibility.visible_ranges(
            snap.xoff + x as i32,
            snap.yoff + y as i32,
            n,
            &mut self.ranges,
        );
        for r in &self.ranges.ranges {
            if r.nx == 0 {
                continue;
            }
            let start = (r.px as i32 - snap.xoff).max(0) as u32;
            out.push(start..start + r.nx);
        }
    }

    fn obscured(&mut self) -> bool {
        self.snapshot.obscured
    }

    fn scrollbar_overlay_visible(&mut self) -> bool {
        self.snapshot.scrollbar_overlay_visible
    }

    fn redraw_pending(&self) -> bool {
        self.snapshot.redraw_pending
    }

    fn effect(&mut self, effect: ScreenRenderEffects, _screen: &Screen) {
        self.effects.push(effect);
    }

    fn begin_write(&mut self) {}
}

/// Apply the effects recorded by a PaneSink once the server is available.
pub fn apply_effects(srv: &mut Server, wp: PaneId, effects: Vec<ScreenRenderEffects>) {
    for effect in effects {
        match effect {
            ScreenRenderEffects::CursorMoved { .. } => {}
            ScreenRenderEffects::DamageRows { start, count } => {
                screen_write_redraw_cb(srv, wp, start, count);
            }
            ScreenRenderEffects::DirtyRows { start, count } => {
                sync_dirty_rows(srv, wp, start, count);
            }
            ScreenRenderEffects::DeferredScroll {
                count,
                rupper,
                rlower,
                bg,
            } => sync_scroll_dirty(srv, wp, count, rupper, rlower, bg),
            ScreenRenderEffects::ScrollbarChanged => {
                if let Some(p) = srv.panes.get_mut(wp) {
                    p.flags.insert(PaneFlags::REDRAWSCROLLBAR);
                }
            }
            ScreenRenderEffects::RequirePaneRedraw => {
                if let Some(p) = srv.panes.get_mut(wp) {
                    p.flags.insert(PaneFlags::REDRAW);
                }
            }
            ScreenRenderEffects::AlternateChanged { .. } => {}
            ScreenRenderEffects::StartSyncTimer => screen_write_start_sync(srv, wp),
            ScreenRenderEffects::StopSync => screen_write_stop_sync(srv, wp),
        }
    }
}

/// screen_write_redraw_cb: damage the window rows of a pane.
fn screen_write_redraw_cb(srv: &mut Server, wp: PaneId, py: u32, ny: u32) {
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    let (window, xoff, yoff, sx) = (p.window, p.xoff, p.yoff, p.sx);
    let mut x0 = xoff;
    let mut y0 = yoff + py as i32;
    let x1 = x0 + sx as i32;
    let y1 = y0 + ny as i32;
    if x0 < 0 {
        x0 = 0;
    }
    if y0 < 0 {
        y0 = 0;
    }
    if x1 > x0 && y1 > y0 {
        if let Some(w) = srv.windows.get_mut(window) {
            redraw_damage_window(w, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
        }
    }
}

/// screen_write_should_draw_lines under MODE_SYNC: record dirty rows.
fn sync_dirty_rows(srv: &mut Server, wp: PaneId, y: u32, mut ny: u32) {
    let Some(p) = srv.panes.get_mut(wp) else {
        return;
    };
    if p.flags.intersects(PaneFlags::REDRAW | PaneFlags::DROP) {
        return;
    }
    let sy = p.base.grid.sy();
    if y < sy && ny != 0 {
        if ny > sy - y {
            ny = sy - y;
        }
        let bs = p.sync.allocate_dirty(sy);
        bs.set_range(y as usize, (y + ny - 1) as usize);
    }
}

/// screen_write_sync_scroll_dirty: defer scrolling until the update ends.
fn sync_scroll_dirty(
    srv: &mut Server,
    wp: PaneId,
    scrolled: u32,
    rupper: u32,
    rlower: u32,
    bg: Colour,
) {
    let overlay = pane_scrollbar_overlay(srv, wp) && pane_scrollbar_visible(srv, wp);
    let sy = match srv.panes.get(wp) {
        Some(p) => p.base.grid.sy(),
        None => return,
    };
    let ry = rlower + 1 - rupper;
    let n = scrolled.min(ry);
    if n == ry || rlower >= sy || overlay {
        sync_dirty_rows(srv, wp, rupper, ry);
        return;
    }
    let Some(p) = srv.panes.get_mut(wp) else {
        return;
    };
    if p.flags.intersects(PaneFlags::REDRAW | PaneFlags::DROP) {
        return;
    }
    let public_id = p.public_id;
    let sync = &mut p.sync;
    sync.allocate_dirty(sy);
    let region_changed =
        sync.scrolled != 0 && (sync.rupper != rupper || sync.rlower != rlower || sync.bg != bg);
    if region_changed {
        log_debug!("%{}: region changed, redrawing all", public_id);
        sync.dirty
            .as_mut()
            .expect("dirty")
            .set_range(0, sy as usize - 1);
        sync.scrolled = 0;
        return;
    }
    sync.rupper = rupper;
    sync.rlower = rlower;
    sync.bg = bg;
    sync.scrolled += n;
    let wrapped = sync.scrolled >= ry;
    if wrapped {
        sync.scrolled = 0;
    }
    let bs = sync.dirty.as_mut().expect("dirty");
    // Keep dirty lines aligned with the scrolled grid.
    let mut y = rupper;
    while y + n <= rlower {
        if bs.test((y + n) as usize) {
            bs.set(y as usize);
        } else {
            bs.clear(y as usize);
        }
        y += 1;
    }
    bs.set_range((rlower + 1 - n) as usize, rlower as usize);
    if wrapped {
        bs.set_range(rupper as usize, rlower as usize);
    }
    p.flags.insert(PaneFlags::REDRAWSCROLLBAR);
    log_debug!(
        "%{}: deferred scroll of {} (region {}-{})",
        public_id,
        scrolled,
        rupper,
        rlower
    );
}

/// screen_write_start_sync: MODE_SYNC is already set by the emulator; arm
/// the one second timer through the model (screen-write.c:1065-1079).
fn screen_write_start_sync(srv: &mut Server, wp: PaneId) {
    let Some(p) = srv.panes.get_mut(wp) else {
        return;
    };
    p.base.mode.insert(ScreenMode::SYNC);
    p.input_state.sync_timer = true;
    srv.effects.push_back(crate::model::ModelEffect::Input(
        crate::model::pane_input::InputAction::Timer {
            pane: wp,
            timer: crate::model::pane_input::InputTimer::Sync,
            after: Some(std::time::Duration::from_secs(1)),
        },
    ));
}

/// screen_write_sync_clear_dirty.
pub fn screen_write_sync_clear_dirty(srv: &mut Server, wp: PaneId) {
    if let Some(p) = srv.panes.get_mut(wp) {
        p.sync.clear();
    }
}

/// screen_write_stop_sync: leave MODE_SYNC (if still set) and flush the
/// dirty rows. Also the `PaneInputHost::stop_sync` hook, which the model
/// calls after it has already cleared MODE_SYNC.
pub fn screen_write_stop_sync(srv: &mut Server, wp: PaneId) {
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    let public_id = p.public_id;
    if p.base.mode.contains(ScreenMode::SYNC) {
        let _ = crate::model::pane_input::pane_stop_sync(srv, wp);
    }
    sync_flush_dirty(srv, wp);
    log_debug!("%{}: stopped sync mode", public_id);
}

/// Write to all clients outside a parse step: take the clients and tparm
/// out, run `f` with a sink, put them back and apply the effects.
fn with_pane_sink(
    srv: &mut Server,
    wp: PaneId,
    f: impl FnOnce(&mut PaneSink, &Screen, &rmux_emu::hyperlinks::HyperlinkRegistry),
) {
    let Some(snapshot) = PaneDrawSnapshot::capture(srv, wp) else {
        return;
    };
    let clients = std::mem::take(&mut srv.clients);
    let tparm = std::mem::take(&mut srv.tparm);
    let mut sink = PaneSink::new(snapshot, clients, tparm);
    if let Some(p) = srv.panes.get(wp) {
        f(&mut sink, &p.base, &srv.hyperlinks);
    }
    let (clients, tparm, effects) = sink.into_parts();
    srv.clients = clients;
    srv.tparm = tparm;
    apply_effects(srv, wp, effects);
}

/// screen_write_sync_flush_dirty.
fn sync_flush_dirty(srv: &mut Server, wp: PaneId) {
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    if p.sync.dirty.is_none() {
        return;
    }
    let sync = p.sync.clone();
    let public_id = p.public_id;
    let redraw = p.flags.contains(PaneFlags::REDRAW);
    let overlay = pane_scrollbar_overlay(srv, wp) && pane_scrollbar_visible(srv, wp);
    let window_sy = srv.windows.get(p.window).map_or(0, |w| w.sy);
    let (yoff, psy) = (p.yoff, p.sy);

    let mut dirty = sync.dirty.clone().expect("dirty");
    let mut lines = 0;
    with_pane_sink(srv, wp, |sink, s, hyperlinks| {
        let sy = s.grid.sy();
        let mut flags = TtyCtxFlags::SYNC;
        if sink.snapshot.obscured {
            flags.insert(TtyCtxFlags::PANE_OBSCURED);
        }
        // screen_write_initctx(&ctx, &ttyctx, 1, 1): tty_cmd_syncstart opens
        // the client transaction before any dirty line (screen-write.c:1312).
        let mut ignore = |_: u32, _: u32| {};
        sink.write(
            TtyCommand::SyncStart,
            s,
            hyperlinks,
            &DEFAULT_CELL,
            TtyCommandData::Count(0),
            flags,
            s.cx,
            s.cy,
            s.rupper,
            s.rlower,
            Colour::DEFAULT,
            &mut ignore,
        );
        // screen_write_sync_apply_scroll
        if sync.scrolled != 0 && !redraw {
            if sync.rlower >= sy || sync.rupper > sync.rlower {
                dirty.set_range(0, sy as usize - 1);
            } else if sink.snapshot.obscured || overlay {
                dirty.set_range(sync.rupper as usize, sync.rlower as usize);
            } else {
                let mut orlower = sync.rlower;
                if yoff + psy as i32 > window_sy as i32 {
                    orlower -= (yoff + psy as i32 - window_sy as i32) as u32;
                }
                let mut marks = Vec::new();
                sink.write(
                    TtyCommand::ScrollUp,
                    s,
                    hyperlinks,
                    &DEFAULT_CELL,
                    TtyCommandData::Count(sync.scrolled),
                    flags,
                    s.cx,
                    s.cy,
                    sync.rupper,
                    orlower,
                    sync.bg,
                    &mut |py, ny| marks.push((py, ny)),
                );
                for (py, ny) in marks {
                    if ny != 0 {
                        dirty.set_range(py as usize, (py + ny - 1) as usize);
                    }
                }
            }
        }
        if !redraw {
            for y in 0..sy {
                if dirty.test(y as usize) {
                    sync_redraw_line(sink, s, hyperlinks, flags, y);
                    lines += 1;
                }
            }
        }
    });
    log_debug!("%{}: had {} dirty lines", public_id, lines);
    screen_write_sync_clear_dirty(srv, wp);
}

/// screen_write_redraw_line: redraw the visible parts of one pane row.
fn sync_redraw_line(
    sink: &mut PaneSink,
    s: &Screen,
    hyperlinks: &rmux_emu::hyperlinks::HyperlinkRegistry,
    flags: TtyCtxFlags,
    yy: u32,
) {
    let sx = s.grid.sx();
    let mut spans = Vec::new();
    sink.visible_columns(0, yy, sx, &mut spans);
    for r in spans {
        let cx = r.start;
        if cx >= sx {
            continue;
        }
        let n = (r.end.min(sx)) - cx;
        if n == 0 {
            continue;
        }
        let mut ignore = |_: u32, _: u32| {};
        if n != 1 {
            sink.write(
                TtyCommand::RedrawLine,
                s,
                hyperlinks,
                &DEFAULT_CELL,
                TtyCommandData::Count(n),
                flags,
                cx,
                yy,
                s.rupper,
                s.rlower,
                Colour::DEFAULT,
                &mut ignore,
            );
            continue;
        }
        let gc = s.grid.view_get_cell(cx, yy);
        let single = gc.data.size == 1 && gc.data.width == 1;
        if !single {
            sink.write(
                TtyCommand::RedrawLine,
                s,
                hyperlinks,
                &DEFAULT_CELL,
                TtyCommandData::Count(n),
                flags,
                cx,
                yy,
                s.rupper,
                s.rlower,
                Colour::DEFAULT,
                &mut ignore,
            );
            continue;
        }
        let cell = if gc.flags.contains(rmux_emu::cell::GridCellFlags::SELECTED) {
            s.select_cell(&gc)
        } else {
            gc
        };
        sink.write(
            TtyCommand::Cell,
            s,
            hyperlinks,
            &cell,
            TtyCommandData::Count(0),
            flags,
            cx,
            yy,
            s.rupper,
            s.rlower,
            Colour::DEFAULT,
            &mut ignore,
        );
    }
}

/// screen_write_setselection forwarded to the clients showing the pane.
pub fn pane_set_selection(srv: &mut Server, wp: PaneId, selector: &[u8], data: &[u8]) {
    let selector = std::str::from_utf8(selector).unwrap_or("").to_owned();
    let data = data.to_vec();
    with_pane_sink(srv, wp, |sink, s, hyperlinks| {
        let mut ignore = |_: u32, _: u32| {};
        // screen_write_initctx (screen-write.c:332-346) opens the context
        // with syncstart; a pane in a mode or not active always syncs, so
        // the OSC 52 lands inside the update, as tty_set_selection does not
        // end it (tty.c:2003-2019).
        let sync = if sink.snapshot.sync_always {
            TtyCtxFlags::SYNC
        } else {
            TtyCtxFlags(0)
        };
        sink.write(
            TtyCommand::SyncStart,
            s,
            hyperlinks,
            &DEFAULT_CELL,
            TtyCommandData::Count(0),
            sync,
            s.cx,
            s.cy,
            s.rupper,
            s.rlower,
            Colour::DEFAULT,
            &mut ignore,
        );
        sink.write(
            TtyCommand::SetSelection,
            s,
            hyperlinks,
            &DEFAULT_CELL,
            TtyCommandData::Selection {
                clip: &selector,
                data: &data,
            },
            TtyCtxFlags(0),
            s.cx,
            s.cy,
            s.rupper,
            s.rlower,
            Colour::DEFAULT,
            &mut ignore,
        );
    });
}

/// tty_draw_images: replay only the selected screen, in insertion order.
#[cfg(feature = "sixel")]
pub fn tty_draw_images(
    tty: &mut rmux_tty::tty::Tty,
    tparm: &mut TparmState,
    srv: &Server,
    c: ClientId,
    wp: PaneId,
) {
    let Some(pane) = srv.panes.get(wp) else {
        return;
    };
    if styles::client_window(srv, c) != Some(pane.window) || pane.layout_cell.is_none() {
        return;
    }
    let screen = pane.screen();
    let Some(owner) = screen.image_owner() else {
        return;
    };
    let (bigger, wox, woy, wsx, wsy) = tty.window_offset();
    let mut flags = TtyCtxFlags::INVISIBLE_PANES;
    if bigger {
        flags.insert(TtyCtxFlags::WINDOW_BIGGER);
    }
    let top_lines = if status::status_at_line(srv, c) == 0 {
        status::status_line_size(srv, c)
    } else {
        0
    };
    for &id in srv.images.ordered(owner) {
        let Some(image) = srv.images.get(owner, id) else {
            continue;
        };
        let ctx = TtyCtx {
            s: screen,
            cell: &DEFAULT_CELL,
            flags,
            data: TtyCommandData::SixelImage(image),
            ocx: image.px,
            ocy: image.py,
            orupper: screen.rupper,
            orlower: screen.rlower,
            xoff: pane.xoff,
            yoff: pane.yoff.wrapping_add(top_lines as i32),
            rxoff: pane.xoff,
            ryoff: pane.yoff,
            sx: pane.sx,
            sy: pane.sy,
            bg: Colour::DEFAULT.0 as u32,
            defaults: DEFAULT_CELL,
            style_ctx: TtyStyleCtx::default(),
            wox,
            woy,
            wsx,
            wsy,
        };
        tty.command(tparm, TtyCommand::SixelImage, &ctx);
    }
}

#[cfg(all(test, feature = "sixel"))]
#[path = "image_tests.rs"]
mod image_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_dirty_allocation_marks_all_on_resize() {
        let mut st = SyncOutputState::default();
        st.allocate_dirty(4).set(1);
        assert_eq!(st.dirty_size, 4);
        assert!(st.dirty.as_ref().unwrap().test(1));
        assert!(!st.dirty.as_ref().unwrap().test(2));
        st.scrolled = 3;
        let bs = st.allocate_dirty(6);
        assert!((0..6).all(|i| bs.test(i)));
        assert_eq!(st.scrolled, 0);
        st.clear();
        assert!(st.dirty.is_none());
        assert_eq!(st.dirty_size, 0);
    }
}
