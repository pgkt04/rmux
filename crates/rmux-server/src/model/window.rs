// Ported from tmux window.c @ 8f25579c
use super::pane::*;
use super::resize::PixelUpdate;
use super::state::{ModelEffect, ModelError, Server, Window};
pub use super::winlink::*;
use super::{PaneFlags, WindowFlags, WinlinkFlags};
use crate::ids::{PaneId, WindowId, WinlinkId};
use crate::layout;
use crate::model::spawn::SpawnFlags;
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowEffect {
    InvalidateScene(WindowId),
    Redraw(WindowId),
    Borders(WindowId),
    Status(WindowId),
    UpdateOffset(WindowId),
    CancelTimers(WindowId),
    Renamed {
        window: WindowId,
        old: Vec<u8>,
        new: Vec<u8>,
    },
    PaneChanged {
        window: WindowId,
        old: Option<PaneId>,
        new: PaneId,
    },
    PaneMoved {
        pane: PaneId,
        old_window: WindowId,
        new_window: WindowId,
        old_index: Option<i32>,
        new_index: Option<i32>,
    },
}

pub(crate) fn effect(server: &mut Server, value: WindowEffect) {
    server.effects.push_back(ModelEffect::Window(value));
}

pub(crate) fn option_number(
    server: &Server,
    options: crate::ids::OptionsId,
    name: &[u8],
    default: i64,
) -> i64 {
    if server.options.get(options, name).is_some() {
        server.options.get_number(options, name)
    } else {
        default
    }
}

pub fn window_create(
    server: &mut Server,
    sx: u32,
    sy: u32,
    xpixel: u32,
    ypixel: u32,
) -> Result<WindowId, ModelError> {
    let options = server.options.create(Some(server.options.global_w));
    let public_id = server.next_window_id;
    server.next_window_id = server.next_window_id.wrapping_add(1);
    let sb =
        PaneScrollbarPolicy::try_from(option_number(server, options, b"pane-scrollbars", 0) as i32)
            .unwrap_or(PaneScrollbarPolicy::Off);
    let sb_pos = PaneScrollbarPosition::try_from(option_number(
        server,
        options,
        b"pane-scrollbars-position",
        0,
    ) as i32)
    .unwrap_or(PaneScrollbarPosition::Right);
    let w = Window {
        public_id,
        name: Vec::new(),
        created: server.current_time,
        activity: server.current_time,
        options,
        panes: Vec::new(),
        last: Vec::new(),
        z_order: Vec::new(),
        links: Vec::new(),
        active: None,
        modal: None,
        modal_last: None,
        previous_zoom: None,
        sx,
        sy,
        xpixel: if xpixel == 0 { 16 } else { xpixel },
        ypixel: if ypixel == 0 { 32 } else { ypixel },
        manual_sx: sx,
        manual_sy: sy,
        pending: None,
        latest: None,
        flags: WindowFlags::default(),
        references: 0,
        layout_root: None,
        saved_layout_root: None,
        lastlayout: None,
        old_layout: None,
        last_new_pane_x: 0,
        last_new_pane_y: 0,
        sb,
        sb_pos,
        name_time: (0, 0),
        name_timer_pending: false,
        focused: false,
        menu_active: false,
        menu: None,
        menu_last_px: 0,
        menu_last_py: 0,
        inside_cell: rmux_emu::cell::DEFAULT_CELL,
        outside_cell: rmux_emu::cell::DEFAULT_CELL,
        redraw_scene_generation: 0,
        damage: crate::ui::redraw::RedrawDamages::new(),
    };
    let id = server.windows.insert(w)?;
    server.windows.retain(id)?;
    server.window_ids.insert(public_id, id);
    Ok(id)
}

pub fn window_find_by_public_id(server: &Server, public_id: u32) -> Option<WindowId> {
    server.window_ids.get(&public_id).copied()
}

pub fn window_find(server: &Server, text: &[u8]) -> Option<WindowId> {
    let text = rmux_util::bytes::cstr(text).strip_prefix(b"@")?;
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return None;
    }
    window_find_by_public_id(server, std::str::from_utf8(text).ok()?.parse().ok()?)
}

pub fn window_retain(server: &mut Server, id: WindowId) -> Result<(), ModelError> {
    let references = server
        .windows
        .get(id)
        .ok_or(ModelError::StaleId)?
        .references
        .checked_add(1)
        .ok_or(crate::ids::ArenaError::LeaseOverflow)?;
    server.windows.retain(id)?;
    server
        .windows
        .get_mut(id)
        .ok_or(ModelError::StaleId)?
        .references = references;
    Ok(())
}

pub fn window_release(server: &mut Server, id: WindowId) -> Result<(), ModelError> {
    if server
        .windows
        .get(id)
        .ok_or(ModelError::StaleId)?
        .references
        == 1
    {
        server.emit(b"window-closed", None, Some(id), None);
    }
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    w.references = w
        .references
        .checked_sub(1)
        .ok_or(crate::ids::ArenaError::LeaseUnderflow)?;
    let destroy = w.references == 0;
    server.windows.release(id)?;
    if destroy {
        window_destroy(server, id)?;
    }
    Ok(())
}

pub fn window_destroy(server: &mut Server, id: WindowId) -> Result<(), ModelError> {
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    w.flags.remove(WindowFlags::ZOOMED);
    let public_id = w.public_id;
    for pane in &w.panes {
        if let Some(p) = server.panes.get_mut(*pane) {
            p.flags.remove(PaneFlags::ZOOMED);
            p.saved_layout_cell = None;
        }
    }
    server.window_ids.remove(&public_id);
    let (root, saved) = {
        let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
        (w.layout_root.take(), w.saved_layout_root.take())
    };
    layout::free_cell(server, root, false);
    layout::free_cell(server, saved, false);
    crate::ui::menu::menu_destroy(server, id);
    window_destroy_panes(server, id)?;
    effect(server, WindowEffect::CancelTimers(id));
    let options = server.windows.get(id).ok_or(ModelError::StaleId)?.options;
    server.options.free(options);
    server.windows.request_remove(id)?;
    server.windows.release(id)?;
    Ok(())
}

pub fn window_set_name(
    server: &mut Server,
    id: WindowId,
    name: &[u8],
    untrusted: bool,
) -> Result<bool, ModelError> {
    let Some(name) = super::state::clean_name(name, untrusted) else {
        return Ok(false);
    };
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    let old = std::mem::replace(&mut w.name, name.clone());
    effect(
        server,
        WindowEffect::Renamed {
            window: id,
            old,
            new: name,
        },
    );
    server.emit(b"window-renamed", None, Some(id), None);
    Ok(true)
}

pub fn window_update_activity(server: &mut Server, id: WindowId) {
    if let Some(w) = server.windows.get_mut(id) {
        w.activity = server.current_time;
    }
}

pub fn window_resize(
    server: &mut Server,
    id: WindowId,
    sx: u32,
    sy: u32,
    xpixel: PixelUpdate,
    ypixel: PixelUpdate,
) -> Result<(), ModelError> {
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    w.sx = sx;
    w.sy = sy;
    match xpixel {
        PixelUpdate::Keep => (),
        PixelUpdate::Default => w.xpixel = 16,
        PixelUpdate::Set(v) => w.xpixel = v,
    }
    match ypixel {
        PixelUpdate::Keep => (),
        PixelUpdate::Default => w.ypixel = 32,
        PixelUpdate::Set(v) => w.ypixel = v,
    }
    if let Some(md) = w.menu.as_mut() {
        crate::ui::menu::menu_resize(md, sx, sy);
        effect(server, WindowEffect::Redraw(id));
    }
    effect(server, WindowEffect::InvalidateScene(id));
    Ok(())
}

pub fn window_has_pane(server: &Server, id: WindowId, pane: PaneId) -> bool {
    server
        .windows
        .get(id)
        .is_some_and(|w| w.panes.contains(&pane))
}

pub fn window_count_panes(server: &Server, id: WindowId, with_floating: bool) -> u32 {
    server.windows.get(id).map_or(0, |w| {
        w.panes
            .iter()
            .filter(|p| with_floating || !pane_is_floating(server, **p))
            .count() as u32
    })
}

pub fn window_has_floating_panes(server: &Server, id: WindowId) -> bool {
    server
        .windows
        .get(id)
        .is_some_and(|w| w.panes.iter().any(|p| pane_is_floating(server, *p)))
}

pub fn window_add_pane(
    server: &mut Server,
    id: WindowId,
    other: Option<PaneId>,
    hlimit: u32,
    flags: SpawnFlags,
) -> Result<PaneId, ModelError> {
    let w = server.windows.get(id).ok_or(ModelError::StaleId)?;
    let target = other.or(w.active);
    let position = if w.panes.is_empty() {
        0
    } else if flags.contains(SpawnFlags::BEFORE) {
        if flags.contains(SpawnFlags::FULLSIZE) {
            0
        } else {
            w.panes
                .iter()
                .position(|p| Some(*p) == target)
                .ok_or(ModelError::StaleId)?
        }
    } else if flags.intersects(SpawnFlags::FULLSIZE | SpawnFlags::FLOATING) {
        w.panes.len()
    } else {
        w.panes
            .iter()
            .position(|p| Some(*p) == target)
            .ok_or(ModelError::StaleId)?
            + 1
    };
    let (sx, sy) = (w.sx, w.sy);
    let pane = pane_create(server, id, sx, sy, hlimit)?;
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    w.panes.insert(position, pane);
    if !flags.contains(SpawnFlags::FLOATING) {
        w.z_order.push(pane);
    } else {
        let z = w
            .modal
            .and_then(|m| w.z_order.iter().position(|p| *p == m))
            .map_or(0, |z| z + 1);
        w.z_order.insert(z, pane);
    }
    effect(server, WindowEffect::InvalidateScene(id));
    Ok(pane)
}

pub fn window_lost_pane(server: &mut Server, id: WindowId, pane: PaneId) -> Result<(), ModelError> {
    if server.marked_pane == Some(pane) {
        server.marked_pane = None;
        server.marked_winlink = None;
    }
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    if w.modal_last == Some(pane) {
        w.modal_last = None;
    }
    if w.previous_zoom == Some(pane) {
        w.previous_zoom = None;
    }
    w.last.retain(|p| *p != pane);
    let mut changed = None;
    if w.active == Some(pane) {
        let modal_last = if w.modal == Some(pane) {
            let last = w.modal_last.take();
            w.modal = None;
            last.filter(|p| w.panes.contains(p))
        } else {
            None
        };
        let position = w.panes.iter().position(|p| *p == pane);
        let neighbour = position
            .and_then(|pos| {
                if pos != 0 {
                    w.panes.get(pos - 1)
                } else {
                    w.panes.get(pos + 1)
                }
            })
            .copied();
        w.active = modal_last.or_else(|| w.last.first().copied()).or(neighbour);
        if let Some(active) = w.active {
            w.last.retain(|p| *p != active);
            changed = Some(active);
        }
    } else if w.modal == Some(pane) {
        w.modal = None;
        w.modal_last = None;
    }
    if let Some(active) = changed {
        if let Some(p) = server.panes.get_mut(active) {
            p.flags.insert(PaneFlags::CHANGED);
        }
        fire_pane_changed(server, id, active, Some(pane));
        window_update_focus(server, id);
    }
    effect(server, WindowEffect::InvalidateScene(id));
    Ok(())
}

pub fn window_remove_pane(
    server: &mut Server,
    id: WindowId,
    pane: PaneId,
) -> Result<(), ModelError> {
    if !window_has_pane(server, id, pane) {
        return Err(ModelError::StaleId);
    }
    window_lost_pane(server, id, pane)?;
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    w.panes.retain(|p| *p != pane);
    w.z_order.retain(|p| *p != pane);
    effect(server, WindowEffect::InvalidateScene(id));
    pane_destroy(server, pane)
}

pub fn window_destroy_panes(server: &mut Server, id: WindowId) -> Result<(), ModelError> {
    let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
    w.last.clear();
    while let Some(pane) = server
        .windows
        .get_mut(id)
        .ok_or(ModelError::StaleId)?
        .panes
        .first()
        .copied()
    {
        let w = server.windows.get_mut(id).ok_or(ModelError::StaleId)?;
        w.panes.remove(0);
        w.z_order.retain(|p| *p != pane);
        pane_destroy(server, pane)?;
    }
    if let Some(w) = server.windows.get_mut(id) {
        w.active = None;
        w.modal = None;
        w.modal_last = None;
    }
    Ok(())
}

fn fire_pane_changed(server: &mut Server, window: WindowId, new: PaneId, old: Option<PaneId>) {
    effect(server, WindowEffect::PaneChanged { window, old, new });
    server.emit(b"window-pane-changed", None, Some(window), Some(new));
}

pub fn window_set_active_pane(
    server: &mut Server,
    window: WindowId,
    pane: PaneId,
    notify: bool,
) -> Result<bool, ModelError> {
    let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
    if w.active == Some(pane) || w.modal.is_some_and(|modal| modal != pane) {
        return Ok(false);
    }
    if !w.panes.contains(&pane) {
        return Err(ModelError::StaleId);
    }
    let unzoomed = w.flags.contains(WindowFlags::ZOOMED) && !pane_is_visible(server, pane);
    if unzoomed {
        window_unzoom(server, window, true)?;
    }
    window_pane_stack_remove(server, pane);
    let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
    let old = w.active;
    if let Some(old) = old {
        w.last.retain(|p| *p != old);
        w.last.insert(0, old);
        if let Some(p) = server.panes.get_mut(old) {
            p.flags.insert(PaneFlags::VISITED);
        }
    }
    w.active = Some(pane);
    let p = server.panes.get_mut(pane).ok_or(ModelError::StaleId)?;
    p.flags.remove(PaneFlags::VISITED);
    p.flags.insert(PaneFlags::CHANGED);
    p.active_point = server.next_active_point;
    server.next_active_point = server.next_active_point.wrapping_add(1);
    if option_number(server, server.options.global, b"focus-events", 0) != 0 {
        if let Some(old) = old {
            pane_update_focus(server, old);
        }
        pane_update_focus(server, pane);
    }
    effect(server, WindowEffect::UpdateOffset(window));
    if unzoomed {
        effect(server, WindowEffect::Redraw(window));
    } else {
        effect(server, WindowEffect::Borders(window));
        effect(server, WindowEffect::Status(window));
    }
    if notify {
        fire_pane_changed(server, window, pane, old);
    }
    Ok(true)
}

pub fn window_update_focus(server: &mut Server, window: WindowId) {
    if let Some(pane) = server.windows.get(window).and_then(|w| w.active) {
        pane_update_focus(server, pane);
    }
}

pub fn window_redraw_active_switch(
    server: &mut Server,
    window: WindowId,
    previous: Option<PaneId>,
) -> Result<(), ModelError> {
    let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
    let active = w.active;
    if previous == active || w.modal.is_some_and(|modal| previous != Some(modal)) {
        return Ok(());
    }
    for id in previous.into_iter().chain(active) {
        if let Some(p) = server.panes.get_mut(id) {
            let a = p.cached_gc;
            let b = p.cached_active_gc;
            let visually_equal = a.fg == b.fg
                && a.bg == b.bg
                && a.attr == b.attr
                && a.link == b.link
                && (a.flags.bits() & !rmux_emu::cell::GridCellFlags::CLEARED.bits())
                    == (b.flags.bits() & !rmux_emu::cell::GridCellFlags::CLEARED.bits());
            if !visually_equal
                || p.cached_dim != p.cached_active_dim
                || p.palette.get(a.fg) != p.palette.get(b.fg)
                || p.palette.get(a.bg) != p.palette.get(b.bg)
            {
                p.flags.insert(PaneFlags::REDRAW);
            }
        }
    }
    if let Some(previous) = previous.filter(|p| pane_is_floating(server, *p)) {
        let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
        w.z_order.retain(|p| *p != previous);
        w.z_order.insert(0, previous);
        if let Some(p) = server.panes.get_mut(previous) {
            p.flags.insert(PaneFlags::REDRAW);
        }
        effect(server, WindowEffect::InvalidateScene(window));
    }
    Ok(())
}

pub fn window_get_pane_status(server: &Server, id: WindowId) -> i64 {
    let Some(w) = server.windows.get(id) else {
        return 0;
    };
    match option_number(server, w.options, b"pane-border-status", 0) {
        3 | 4 => 0,
        value => value,
    }
}

pub fn window_get_active_at(server: &Server, id: WindowId, x: u32, y: u32) -> Option<PaneId> {
    let w = server.windows.get(id)?;
    if let Some(modal) = w.modal {
        return pane_contains(server, modal, x, y).then_some(modal);
    }
    for pane in &w.z_order {
        if pane_is_floating(server, *pane) && pane_contains(server, *pane, x, y) {
            return Some(*pane);
        }
    }
    let status = window_get_pane_status(server, id);
    if status == 1 {
        for pane in &w.z_order {
            if !pane_is_visible(server, *pane) || pane_is_floating(server, *pane) {
                continue;
            }
            let (px, py, sx, _) = pane_full_size_offset(server, *pane)?;
            if i64::from(x) >= px && i64::from(x) <= px + i64::from(sx) && i64::from(y) == py - 1 {
                return Some(*pane);
            }
        }
    }
    for pane in &w.z_order {
        if !pane_is_visible(server, *pane) {
            continue;
        }
        if pane_is_floating(server, *pane) {
            if pane_contains(server, *pane, x, y) {
                return Some(*pane);
            }
        } else {
            let (px, py, sx, sy) = pane_full_size_offset(server, *pane)?;
            let top = py - i64::from(status == 1);
            if i64::from(x) >= px
                && i64::from(x) <= px + i64::from(sx)
                && i64::from(y) >= top
                && i64::from(y) <= py + i64::from(sy)
            {
                return Some(*pane);
            }
        }
    }
    None
}

pub fn pane_description(server: &Server, id: WindowId, name: &[u8]) -> Option<PaneId> {
    let w = server.windows.get(id)?;
    let mut x = w.sx / 2;
    let mut y = w.sy / 2;
    let status = window_get_pane_status(server, id);
    let top = u32::from(status == 1);
    let bottom = w.sy.wrapping_sub(1 + u32::from(status == 2));
    let name = rmux_util::bytes::cstr(name);
    if name.eq_ignore_ascii_case(b"top") {
        y = top;
    } else if name.eq_ignore_ascii_case(b"bottom") {
        y = bottom;
    } else if name.eq_ignore_ascii_case(b"left") {
        x = 0;
    } else if name.eq_ignore_ascii_case(b"right") {
        x = w.sx.wrapping_sub(1);
    } else if name.eq_ignore_ascii_case(b"top-left") {
        x = 0;
        y = top;
    } else if name.eq_ignore_ascii_case(b"top-right") {
        x = w.sx.wrapping_sub(1);
        y = top;
    } else if name.eq_ignore_ascii_case(b"bottom-left") {
        x = 0;
        y = bottom;
    } else if name.eq_ignore_ascii_case(b"bottom-right") {
        x = w.sx.wrapping_sub(1);
        y = bottom;
    } else {
        return None;
    }
    window_get_active_at(server, id, x, y)
}

pub fn pane_direction(
    server: &Server,
    id: PaneId,
    direction: crate::cmd::find::PaneDirection,
) -> Option<PaneId> {
    use crate::cmd::find::PaneDirection::*;
    let p = server.panes.get(id)?;
    let w = server.windows.get(p.window)?;
    let (x, y, sx, sy) = pane_full_size_offset(server, id)?;
    let status = window_get_pane_status(server, p.window);
    let (edge, start, end) = match direction {
        Up => (
            if (status == 1 && y == 1) || (status != 1 && y == 0) {
                i64::from(w.sy) + i64::from(status != 2)
            } else {
                y
            },
            x,
            x + i64::from(sx),
        ),
        Down => {
            let edge = y + i64::from(sy) + 1;
            (
                if edge >= i64::from(w.sy) - i64::from(status == 2) {
                    i64::from(status == 1)
                } else {
                    edge
                },
                i64::from(p.xoff),
                i64::from(p.xoff) + i64::from(p.sx),
            )
        }
        Left => (
            if x == 0 { i64::from(w.sx) + 1 } else { x },
            y,
            y + i64::from(sy),
        ),
        Right => (
            if x + i64::from(sx) + 1 >= i64::from(w.sx) {
                0
            } else {
                x + i64::from(sx) + 1
            },
            i64::from(p.yoff),
            i64::from(p.yoff) + i64::from(p.sy),
        ),
    };
    let mut best = None;
    for next in &w.panes {
        if *next == id {
            continue;
        }
        let (nx, ny, nsx, nsy) = pane_full_size_offset(server, *next)?;
        let (adjacent, low, high) = match direction {
            Up => (ny + i64::from(nsy) + 1 == edge, nx, nx + i64::from(nsx) - 1),
            Down => (ny == edge, nx, nx + i64::from(nsx) - 1),
            Left => (nx + i64::from(nsx) + 1 == edge, ny, ny + i64::from(nsy) - 1),
            Right => (nx == edge, ny, ny + i64::from(nsy) - 1),
        };
        if !adjacent
            || !((low < start && high > end)
                || (low >= start && low <= end)
                || (high >= start && high <= end))
        {
            continue;
        }
        let point = server.panes.get(*next)?.active_point;
        if best.is_none_or(|(_, previous)| point > previous) {
            best = Some((*next, point));
        }
    }
    best.map(|(pane, _)| pane)
}

pub fn window_zoom(
    server: &mut Server,
    window: WindowId,
    pane: PaneId,
) -> Result<bool, ModelError> {
    let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
    if w.flags.contains(WindowFlags::ZOOMED) || w.panes.len() == 1 {
        return Ok(false);
    }
    if !w.panes.contains(&pane) {
        return Err(ModelError::StaleId);
    }
    let preserve = w.active.is_some_and(|a| {
        server
            .panes
            .get(a)
            .is_some_and(|p| p.flags.contains(PaneFlags::FLOATOVERZOOM))
            && pane_is_floating(server, a)
    });
    if w.active != Some(pane) && !preserve {
        window_set_active_pane(server, window, pane, true)?;
    }
    let panes = server
        .windows
        .get(window)
        .ok_or(ModelError::StaleId)?
        .panes
        .clone();
    for p in &panes {
        let p = server.panes.get_mut(*p).ok_or(ModelError::StaleId)?;
        p.saved_layout_cell = p.layout_cell.take();
    }
    server
        .panes
        .get_mut(pane)
        .ok_or(ModelError::StaleId)?
        .flags
        .insert(PaneFlags::ZOOMED);
    let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
    w.saved_layout_root = w.layout_root.take();
    layout::init(server, window, pane);
    for overlay in &panes {
        if *overlay == pane {
            continue;
        }
        let p = server.panes.get(*overlay).ok_or(ModelError::StaleId)?;
        if !p.flags.contains(PaneFlags::FLOATOVERZOOM) {
            continue;
        }
        let Some(saved) = p
            .saved_layout_cell
            .and_then(|c| server.layout_cells.get(c))
            .filter(|c| c.is_floating())
        else {
            continue;
        };
        let geometry = saved.g;
        let cell = layout::floating_pane(server, window, Some(pane), &geometry);
        layout::assign_pane(server, cell, *overlay, false);
    }
    if server
        .panes
        .get(pane)
        .and_then(|p| p.saved_layout_cell)
        .and_then(|c| server.layout_cells.get(c))
        .is_some_and(|c| c.is_floating())
    {
        let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
        w.z_order.retain(|p| *p != pane);
        w.z_order.push(pane);
    }
    server
        .windows
        .get_mut(window)
        .ok_or(ModelError::StaleId)?
        .flags
        .insert(WindowFlags::ZOOMED);
    server.emit(b"window-zoomed", None, Some(window), None);
    server.emit(b"window-layout-changed", None, Some(window), None);
    effect(server, WindowEffect::InvalidateScene(window));
    Ok(true)
}

pub fn window_unzoom(
    server: &mut Server,
    window: WindowId,
    notify: bool,
) -> Result<bool, ModelError> {
    let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
    if !w.flags.contains(WindowFlags::ZOOMED) {
        return Ok(false);
    }
    let panes = w.panes.clone();
    let mut zoomed = None;
    for id in &panes {
        let p = server.panes.get(*id).ok_or(ModelError::StaleId)?;
        if p.flags.contains(PaneFlags::ZOOMED) {
            zoomed = Some(*id);
            continue;
        }
        if !p.flags.contains(PaneFlags::FLOATOVERZOOM) {
            continue;
        }
        if let (Some(saved), Some(current)) = (p.saved_layout_cell, p.layout_cell) {
            let current = server
                .layout_cells
                .get(current)
                .ok_or(ModelError::StaleId)?;
            let (g, fg) = (current.g, current.fg);
            let saved = server
                .layout_cells
                .get_mut(saved)
                .ok_or(ModelError::StaleId)?;
            saved.g = g;
            saved.fg = fg;
        }
    }
    server
        .windows
        .get_mut(window)
        .ok_or(ModelError::StaleId)?
        .flags
        .remove(WindowFlags::ZOOMED);
    layout::free(server, window, false);
    let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
    w.layout_root = w.saved_layout_root.take();
    for id in &panes {
        let p = server.panes.get_mut(*id).ok_or(ModelError::StaleId)?;
        p.layout_cell = p.saved_layout_cell.take();
        p.flags.remove(PaneFlags::ZOOMED);
    }
    if let Some(zoomed) = zoomed.filter(|p| pane_is_floating(server, *p)) {
        let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
        let at = if w.active == Some(zoomed) {
            0
        } else {
            w.z_order
                .iter()
                .filter(|p| **p != zoomed)
                .position(|p| !pane_is_floating(server, *p))
                .unwrap_or(w.z_order.len() - 1)
        };
        let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
        w.z_order.retain(|p| *p != zoomed);
        w.z_order.insert(at, zoomed);
    }
    layout::fix_panes(server, window, None);
    if notify {
        server.emit(b"window-unzoomed", None, Some(window), None);
        server.emit(b"window-layout-changed", None, Some(window), None);
    }
    effect(server, WindowEffect::InvalidateScene(window));
    Ok(true)
}

pub fn window_zoomed_pane(server: &Server, window: WindowId) -> Option<PaneId> {
    let w = server.windows.get(window)?;
    if !w.flags.contains(WindowFlags::ZOOMED) {
        return None;
    }
    w.z_order.iter().rev().copied().find(|p| {
        server
            .panes
            .get(*p)
            .is_some_and(|p| p.layout_cell.is_some())
            && !pane_is_floating(server, *p)
    })
}

pub fn window_active_pane_is_over_zoom(server: &Server, window: WindowId) -> bool {
    server.windows.get(window).is_some_and(|w| {
        w.flags.contains(WindowFlags::ZOOMED)
            && w.active.is_some_and(|p| {
                pane_is_floating(server, p)
                    && server
                        .panes
                        .get(p)
                        .is_some_and(|p| p.flags.contains(PaneFlags::FLOATOVERZOOM))
            })
    })
}

pub fn window_push_zoom(
    server: &mut Server,
    window: WindowId,
    always: bool,
    keep: bool,
) -> Result<bool, ModelError> {
    let target = window_zoomed_pane(server, window);
    let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
    if keep && (always || w.flags.contains(WindowFlags::ZOOMED)) {
        w.flags.insert(WindowFlags::WASZOOMED);
        w.previous_zoom = target;
    } else {
        w.flags.remove(WindowFlags::WASZOOMED);
        w.previous_zoom = None;
    }
    window_unzoom(server, window, true)
}

pub fn window_pop_zoom(
    server: &mut Server,
    window: WindowId,
    prefer_tiled: bool,
) -> Result<bool, ModelError> {
    let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
    if !w.flags.contains(WindowFlags::WASZOOMED) {
        return Ok(false);
    }
    w.flags.remove(WindowFlags::WASZOOMED);
    let mut target = w.previous_zoom.take();
    let active = w.active;
    if prefer_tiled
        && active.is_some_and(|p| {
            !pane_is_floating(server, p)
                || server
                    .panes
                    .get(p)
                    .is_some_and(|p| !p.flags.contains(PaneFlags::FLOATOVERZOOM))
        })
    {
        target = active;
    }
    if target.is_none_or(|p| !window_has_pane(server, window, p)) {
        target = active;
    }
    if let Some(pane) = target {
        window_zoom(server, window, pane)
    } else {
        Ok(false)
    }
}

pub fn window_printable_flags(server: &Server, link: WinlinkId, escape: bool, out: &mut Vec<u8>) {
    let Some(link_ref) = server.winlinks.get(link) else {
        return;
    };
    let Some(s) = server.sessions.get(link_ref.session) else {
        return;
    };
    let Some(w) = server.windows.get(link_ref.window) else {
        return;
    };
    if link_ref.flags.contains(WinlinkFlags::ACTIVITY) {
        out.push(b'#');
        if escape {
            out.push(b'#');
        }
    }
    if link_ref.flags.contains(WinlinkFlags::BELL) {
        out.push(b'!');
    }
    if link_ref.flags.contains(WinlinkFlags::SILENCE) {
        out.push(b'~');
    }
    if s.current == Some(link) {
        out.push(b'*');
    }
    if s.last.first() == Some(&link) {
        out.push(b'-');
    }
    if server.marked_winlink == Some(link)
        && server.marked_pane.is_some_and(|p| {
            w.panes.contains(&p) && server.pane_ids.values().any(|active| *active == p)
        })
    {
        out.push(b'M');
    }
    if w.modal.is_some() {
        out.push(b'O');
    }
    if w.flags.contains(WindowFlags::ZOOMED) {
        out.push(b'Z');
    }
}

pub fn window_fire_pane_moved(
    server: &mut Server,
    pane: PaneId,
    old_window: WindowId,
    old_index: i32,
    new_window: WindowId,
    new_index: i32,
) {
    effect(
        server,
        WindowEffect::PaneMoved {
            pane,
            old_window,
            new_window,
            old_index: (old_index != -1).then_some(old_index),
            new_index: (new_index != -1).then_some(new_index),
        },
    );
    server.emit(b"pane-moved", None, Some(new_window), Some(pane));
}

/// `window_pane_stack_remove` (`window.c:2472-2478`). `TAILQ_REMOVE` unlinks the
/// pane from whichever window's `last_panes` list holds it (swap-pane moves a
/// visited pane between windows before removing it), so every list is checked.
pub fn window_pane_stack_remove(server: &mut Server, wp: PaneId) {
    let Some(pane) = server.panes.get_mut(wp) else {
        return;
    };
    if !pane.flags.contains(PaneFlags::VISITED) {
        return;
    }
    pane.flags.remove(PaneFlags::VISITED);
    let ids: Vec<WindowId> = server.window_ids.values().copied().collect();
    for id in ids {
        if let Some(w) = server.windows.get_mut(id) {
            w.last.retain(|&p| p != wp);
        }
    }
}

/// `window_damage_floating_pane` (`window.c:2990-3016`): border and scrollbar included.
fn window_damage_floating_pane(
    server: &mut Server,
    wp: PaneId,
    xoff: i32,
    yoff: i32,
    sx: i32,
    sy: i32,
) {
    let Some(pane) = server.panes.get(wp) else {
        return;
    };
    let window = pane.window;
    let style = pane.scrollbar_style;
    let Some(w) = server.windows.get(window) else {
        return;
    };
    let (mut sb_left, mut sb_right) = (0, 0);
    if pane_scrollbar_reserve(server, wp) {
        if w.sb_pos == PaneScrollbarPosition::Left {
            sb_left = style.width + style.pad;
        } else {
            sb_right = style.width + style.pad;
        }
    }
    let x0 = (xoff - 1 - sb_left).max(0);
    let x1 = xoff + sx + sb_right;
    let y0 = (yoff - 1).max(0);
    let y1 = yoff + sy;
    if x1 >= x0
        && y1 >= y0
        && let Some(w) = server.windows.get_mut(window)
    {
        crate::ui::redraw::redraw_damage_window(
            w,
            x0 as u32,
            y0 as u32,
            (x1 - x0) as u32 + 1,
            (y1 - y0) as u32 + 1,
        );
    }
}

/// `window_redraw_floating_pane` (`window.c:3018-3026`): damage old and new areas, then status.
pub fn window_redraw_floating_pane(
    server: &mut Server,
    wp: PaneId,
    oxoff: i32,
    oyoff: i32,
    osx: u32,
    osy: u32,
) {
    window_damage_floating_pane(server, wp, oxoff, oyoff, osx as i32, osy as i32);
    let Some(pane) = server.panes.get(wp) else {
        return;
    };
    let (window, xoff, yoff, sx, sy) = (pane.window, pane.xoff, pane.yoff, pane.sx, pane.sy);
    window_damage_floating_pane(server, wp, xoff, yoff, sx as i32, sy as i32);
    effect(server, WindowEffect::Status(window));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panes(server: &mut Server) -> (WindowId, [PaneId; 3]) {
        let w = window_create(server, 80, 24, 0, 0).unwrap();
        let a = window_add_pane(server, w, None, 10, SpawnFlags::default()).unwrap();
        window_set_active_pane(server, w, a, false).unwrap();
        let b = window_add_pane(server, w, None, 10, SpawnFlags::default()).unwrap();
        let c = window_add_pane(
            server,
            w,
            None,
            10,
            SpawnFlags::BEFORE | SpawnFlags::FULLSIZE,
        )
        .unwrap();
        (w, [a, b, c])
    }
    #[test]
    fn insertion_selection_modal_fallback() {
        let mut s = Server::default();
        let (w, [a, b, c]) = panes(&mut s);
        assert_eq!(s.windows.get(w).unwrap().panes, [c, a, b]);
        window_set_active_pane(&mut s, w, b, true).unwrap();
        assert_eq!(s.windows.get(w).unwrap().last, [a]);
        {
            let w = s.windows.get_mut(w).unwrap();
            w.modal = Some(b);
            w.modal_last = Some(c);
        }
        assert!(!window_set_active_pane(&mut s, w, a, true).unwrap());
        window_remove_pane(&mut s, w, b).unwrap();
        assert_eq!(s.windows.get(w).unwrap().active, Some(c));
        assert!(s.panes.get(b).is_none());
    }
    #[test]
    fn root_lease_and_public_identity() {
        let mut s = Server::default();
        let (w, [a, _, _]) = panes(&mut s);
        let public = s.windows.get(w).unwrap().public_id;
        window_retain(&mut s, w).unwrap();
        pane_retain(&mut s, a).unwrap();
        window_release(&mut s, w).unwrap();
        assert_eq!(window_find_by_public_id(&s, public), None);
        assert!(s.panes.get(a).unwrap().flags.contains(PaneFlags::DESTROYED));
        pane_release(&mut s, a).unwrap();
        assert!(s.panes.get(a).is_none());
        assert!(
            s.effects
                .iter()
                .any(|e| matches!(e, ModelEffect::Event { name, .. } if name == b"window-closed"))
        );
    }
    #[test]
    fn zoom_restores_roots_and_pane_links() {
        let mut s = Server::default();
        let (w, [a, _, _]) = panes(&mut s);
        layout::init(&mut s, w, a);
        let root = s.windows.get(w).unwrap().layout_root;
        assert!(window_zoom(&mut s, w, a).unwrap());
        assert_eq!(s.windows.get(w).unwrap().saved_layout_root, root);
        assert_ne!(s.windows.get(w).unwrap().layout_root, root);
        assert!(window_unzoom(&mut s, w, true).unwrap());
        assert_eq!(s.windows.get(w).unwrap().layout_root, root);
        assert!(s.windows.get(w).unwrap().saved_layout_root.is_none());
        assert_eq!(s.panes.get(a).unwrap().layout_cell, root);
    }
    #[test]
    fn closed_callback_retains_before_release_and_can_be_released_later() {
        fn retain(
            server: &mut Server,
            name: &[u8],
            _: Option<crate::ids::SessionId>,
            window: Option<WindowId>,
            _: Option<PaneId>,
        ) {
            if name == b"window-closed" {
                server.model_event = None;
                window_retain(server, window.unwrap()).unwrap();
            }
        }
        let mut s = Server::default();
        let w = window_create(&mut s, 10, 4, 0, 0).unwrap();
        window_retain(&mut s, w).unwrap();
        s.model_event = Some(retain);
        window_release(&mut s, w).unwrap();
        assert_eq!(s.windows.get(w).unwrap().references, 1);
        assert_eq!(
            window_find_by_public_id(&s, s.windows.get(w).unwrap().public_id),
            Some(w)
        );
        window_release(&mut s, w).unwrap();
        assert!(s.windows.get(w).is_none());
    }
    #[test]
    fn directional_candidates_use_list_order_on_mru_ties() {
        use crate::cmd::find::PaneDirection;
        let mut s = Server::default();
        let (w, [a, b, c]) = panes(&mut s);
        s.windows.get_mut(w).unwrap().sx = 21;
        s.windows.get_mut(w).unwrap().sy = 10;
        for (id, x, y, sx, sy) in [(a, 0, 0, 10, 10), (b, 11, 0, 10, 4), (c, 11, 5, 10, 5)] {
            let p = s.panes.get_mut(id).unwrap();
            p.xoff = x;
            p.yoff = y;
            p.sx = sx;
            p.sy = sy;
            p.active_point = 1;
        }
        assert_eq!(pane_direction(&s, a, PaneDirection::Right), Some(c));
        s.panes.get_mut(b).unwrap().active_point = 2;
        assert_eq!(pane_direction(&s, a, PaneDirection::Right), Some(b));
        assert_eq!(pane_description(&s, w, b"ToP-RiGhT"), Some(b));
    }
    #[test]
    fn overlay_geometry_and_floating_target_z_order_survive_zoom() {
        let mut s = Server::default();
        let (w, [a, b, c]) = panes(&mut s);
        layout::init(&mut s, w, a);
        let target = layout::floating_pane(
            &mut s,
            w,
            Some(a),
            &layout::LayoutGeometry {
                sx: 10,
                sy: 6,
                xoff: 5,
                yoff: 4,
            },
        );
        layout::assign_pane(&mut s, target, b, false);
        let saved_overlay = layout::floating_pane(
            &mut s,
            w,
            Some(a),
            &layout::LayoutGeometry {
                sx: 8,
                sy: 4,
                xoff: 20,
                yoff: 3,
            },
        );
        layout::assign_pane(&mut s, saved_overlay, c, false);
        s.panes
            .get_mut(c)
            .unwrap()
            .flags
            .insert(PaneFlags::FLOATOVERZOOM);
        window_set_active_pane(&mut s, w, c, false).unwrap();
        assert!(window_zoom(&mut s, w, b).unwrap());
        assert_eq!(s.windows.get(w).unwrap().active, Some(c));
        assert_eq!(s.windows.get(w).unwrap().z_order.last(), Some(&b));
        let current = s.panes.get(c).unwrap().layout_cell.unwrap();
        s.layout_cells.get_mut(current).unwrap().g.xoff = -2;
        assert!(window_unzoom(&mut s, w, false).unwrap());
        assert_eq!(s.layout_cells.get(saved_overlay).unwrap().g.xoff, -2);
        assert_eq!(s.panes.get(c).unwrap().layout_cell, Some(saved_overlay));
        assert_eq!(s.panes.get(b).unwrap().layout_cell, Some(target));
        assert!(pane_is_floating(&s, b));
        assert!(s.windows.get(w).unwrap().saved_layout_root.is_none());
    }
}
