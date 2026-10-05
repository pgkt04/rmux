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

//! Copy-mode mouse endpoint dragging and edge-scroll timers.

use super::state::{CopyTimerAction, CursorDrag, LineSelectionDirection, SelectionMode};
use super::{motion, render, select, state};
use crate::client::{MouseDragAction, ResolvedMouseEvent};
use crate::cmd::find::{self, MouseInput, PaneGeometry};
use crate::ids::{ClientId, ModeId};
use crate::model::Server;
use crate::server::event_loop::LoopAction;
use std::time::Duration;

fn target(server: &Server, m: &MouseInput) -> Option<ModeId> {
    let (_, _, pane) = find::mouse_pane(server, m)?;
    let mode = server.panes.get(pane)?.modes.first()?.id;
    state::data(server, mode)?;
    Some(mode)
}
fn position(server: &Server, mode: ModeId, m: &MouseInput, last: bool) -> Option<(u32, u32)> {
    let p = server.panes.get(mode.owner)?;
    find::mouse_at(
        PaneGeometry {
            x: p.xoff,
            y: p.yoff,
            width: p.sx,
            height: p.sy,
        },
        m,
        last,
    )
}
fn input(ev: &ResolvedMouseEvent) -> MouseInput {
    MouseInput {
        valid: ev.target.valid,
        session: ev.target.session,
        window: ev.target.window,
        pane: ev.target.pane,
        x: ev.event.x,
        y: ev.event.y,
        last_x: ev.event.lx,
        last_y: ev.event.ly,
        b: ev.event.b,
        lb: ev.event.lb,
        sgr_type: ev.event.sgr_type,
        sgr_b: ev.event.sgr_b,
        offset_x: ev.target.ox,
        offset_y: ev.target.oy,
        status_at: ev.target.status_at,
        status_lines: ev.target.status_lines,
    }
}
pub fn move_mouse(server: &mut Server, m: &MouseInput) {
    let Some(mode) = target(server, m) else {
        return;
    };
    let Some((x, y)) = position(server, mode, m, false) else {
        return;
    };
    let width = state::screen(server, mode).unwrap().grid.sx();
    select::update_cursor(
        server,
        mode,
        render::cursor_unoffset(server, mode, x, width),
        y,
    );
}
pub fn start_drag(server: &mut Server, client: Option<ClientId>, m: &MouseInput) {
    let Some(client) = client.filter(|c| server.clients.get(*c).is_some()) else {
        return;
    };
    let Some(mode) = target(server, m) else {
        return;
    };
    let Some((x, y)) = position(server, mode, m, true) else {
        return;
    };
    let drag = &mut server.clients.get_mut(client).unwrap().drag;
    drag.update = Some(MouseDragAction::Mode(mode));
    drag.release = Some(MouseDragAction::Mode(mode));
    let inside = select::mouse_in_selection(server, mode, x, y);
    let width = state::screen(server, mode).unwrap().grid.sx();
    let x = render::cursor_unoffset(server, mode, x, width);
    let d = state::data_mut(server, mode).unwrap();
    // The hit test always chooses one endpoint for an inside hit; C resets
    // granularity for that endpoint or for an outside hit.
    d.selection.lineflag = LineSelectionDirection::None;
    d.selection.selflag = SelectionMode::Char;
    select::update_cursor(server, mode, x, y);
    if let Some(endpoint) = inside {
        state::data_mut(server, mode).unwrap().selection.cursordrag = endpoint;
        select::update_selection(server, mode, true, false);
    } else {
        select::start_selection(server, mode);
    }
    render::redraw_screen(server, mode);
    update(server, mode, m);
}
fn cancel(server: &mut Server, mode: ModeId) {
    if let Some(timer) = state::data_mut(server, mode).and_then(|d| d.dragtimer.take()) {
        server.event_loop.cancel(timer);
    }
}
fn arm(server: &mut Server, mode: ModeId) {
    let timer = server.event_loop.schedule(
        Duration::from_millis(50),
        LoopAction::CopyTimer(CopyTimerAction::Drag(mode)),
    );
    if let Some(d) = state::data_mut(server, mode) {
        d.dragtimer = Some(timer);
    }
}
fn update(server: &mut Server, mode: ModeId, m: &MouseInput) {
    if target(server, m) != Some(mode) {
        return;
    }
    cancel(server, mode);
    let Some((x, y)) = position(server, mode, m, false) else {
        return;
    };
    let width = state::screen(server, mode).unwrap().grid.sx();
    let x = render::cursor_unoffset(server, mode, x, width);
    let d = state::data(server, mode).unwrap();
    let (oldx, oldy) = (d.cx, d.cy);
    select::update_cursor(server, mode, x, y);
    if select::update_selection(server, mode, true, false) {
        render::redraw_selection(server, mode, oldy);
    }
    let d = state::data(server, mode).unwrap();
    if oldy != d.cy || oldx == d.cx {
        let sy = state::screen(server, mode).unwrap().grid.sy();
        if y == 0 {
            arm(server, mode);
            motion::cursor_up(server, mode, true);
        } else if y == sy - 1 {
            arm(server, mode);
            motion::cursor_down(server, mode, true);
        }
    }
}
pub fn drag_update(server: &mut Server, mode: ModeId, _client: ClientId, ev: &ResolvedMouseEvent) {
    update(server, mode, &input(ev));
}
pub fn drag_release(server: &mut Server, mode: ModeId, _client: ClientId, ev: &ResolvedMouseEvent) {
    let m = input(ev);
    if target(server, &m) != Some(mode) {
        return;
    }
    if render::line_numbers_active(server, mode) {
        update(server, mode, &m);
    }
    state::data_mut(server, mode).unwrap().selection.cursordrag = CursorDrag::None;
    cancel(server, mode);
}
pub fn scroll_timer(server: &mut Server, mode: ModeId) {
    cancel(server, mode);
    if server
        .panes
        .get(mode.owner)
        .and_then(|p| p.modes.first())
        .map(|m| m.id)
        != Some(mode)
    {
        return;
    }
    let Some(d) = state::data(server, mode) else {
        return;
    };
    let cy = d.cy;
    let sy = state::screen(server, mode).unwrap().grid.sy();
    if cy == 0 {
        arm(server, mode);
        motion::cursor_up(server, mode, true);
    } else if cy == sy - 1 {
        arm(server, mode);
        motion::cursor_down(server, mode, true);
    }
}
pub fn jump_to_mark(server: &mut Server, mode: ModeId) {
    let Some(d) = state::data_mut(server, mode) else {
        return;
    };
    let old = (d.cx, d.backing_y());
    d.cx = d.mx;
    let h = d.backing.screen().grid.hsize();
    if d.my < h {
        d.cy = 0;
        d.oy = h - d.my;
    } else {
        d.cy = d.my - h;
        d.oy = 0;
    }
    (d.mx, d.my) = old;
    d.showmark = true;
    select::update_selection(server, mode, false, false);
    render::redraw_screen(server, mode);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MouseTarget;
    use rmux_util::key::MouseEvent;

    #[test]
    fn resolved_geometry_preserves_offsets_status_and_drag_origin() {
        let ev = ResolvedMouseEvent {
            event: MouseEvent {
                x: 12,
                y: 9,
                lx: 11,
                ly: 8,
                ..MouseEvent::default()
            },
            target: MouseTarget {
                valid: true,
                ox: 3,
                oy: 2,
                status_at: 0,
                status_lines: 1,
                ..MouseTarget::default()
            },
        };
        let m = input(&ev);
        let pane = PaneGeometry {
            x: 10,
            y: 5,
            width: 20,
            height: 10,
        };
        assert_eq!(find::mouse_at(pane, &m, false), Some((5, 5)));
        assert_eq!(find::mouse_at(pane, &m, true), Some((4, 4)));
        assert!(m.valid);
    }
    #[test]
    fn absent_mouse_target_and_stale_timer_have_no_effect() {
        use crate::ids::ArenaId;
        let mut server = Server::default();
        let m = MouseInput::default();
        move_mouse(&mut server, &m);
        start_drag(&mut server, None, &m);
        let mode = ModeId::new(crate::ids::PaneId::from_parts(0, 0), 0, 0);
        scroll_timer(&mut server, mode);
        jump_to_mark(&mut server, mode);
        assert!(server.panes.is_empty());
    }
    #[test]
    fn drag_installs_callbacks_steps_at_edge_and_release_stops_timer() {
        use crate::model::{pane, session, window};
        use crate::modes::WindowModeFlags;
        use crate::modes::copy::{CopyModeDriver, CopyModeKind};
        use crate::options::environment::Environment;
        use std::rc::Rc;
        let mut server = Server::default();
        let options = server.options.create(Some(server.options.global_s));
        let session = session::session_create(
            &mut server,
            session::SessionCreate {
                prefix: None,
                name: Some(b"mouse".to_vec()),
                cwd: b"/".to_vec(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        );
        let window = window::window_create(&mut server, 12, 3, 0, 0).unwrap();
        let pane = pane::pane_create(&mut server, window, 12, 3, 100).unwrap();
        server.windows.get_mut(window).unwrap().panes.push(pane);
        server.windows.get_mut(window).unwrap().active = Some(pane);
        let link = session::session_attach(&mut server, session, window, 0).unwrap();
        session::session_set_current(&mut server, session, Some(link));
        let mode = pane::pane_set_mode(
            &mut server,
            pane,
            b"view-mode",
            WindowModeFlags::default(),
            Rc::new(CopyModeDriver {
                kind: CopyModeKind::View,
            }),
            false,
        )
        .unwrap()
        .unwrap();
        for text in [b"one".as_slice(), b"two", b"three", b"four"] {
            super::super::view::append_output(&mut server, mode, text).unwrap();
        }
        let client = server
            .clients
            .insert(crate::client::Client::new(None, (0, 0)))
            .unwrap();
        server.clients.get_mut(client).unwrap().session = Some(session);
        let m = MouseInput {
            valid: true,
            session: Some(session),
            window: Some(window),
            pane: Some(pane),
            x: 1,
            y: 0,
            last_x: 1,
            last_y: 1,
            status_at: -1,
            ..MouseInput::default()
        };
        state::data_mut(&mut server, mode)
            .unwrap()
            .selection
            .selflag = SelectionMode::Word;
        start_drag(&mut server, Some(client), &m);
        assert_eq!(
            state::data(&server, mode).unwrap().selection.selflag,
            SelectionMode::Char
        );
        assert_eq!(
            server.clients.get(client).unwrap().drag.update,
            Some(MouseDragAction::Mode(mode))
        );
        assert!(state::data(&server, mode).unwrap().selection.active);
        assert!(state::data(&server, mode).unwrap().dragtimer.is_some());
        let ev = ResolvedMouseEvent {
            event: MouseEvent {
                x: 1,
                y: 0,
                lx: 1,
                ly: 1,
                ..MouseEvent::default()
            },
            target: MouseTarget {
                valid: true,
                session: Some(session),
                window: Some(window),
                pane: Some(pane),
                ..MouseTarget::default()
            },
        };
        drag_release(&mut server, mode, client, &ev);
        let d = state::data(&server, mode).unwrap();
        assert_eq!(d.selection.cursordrag, CursorDrag::None);
        assert!(d.dragtimer.is_none());
        pane::pane_reset_mode(&mut server, pane).unwrap();
        drag_update(&mut server, mode, client, &ev);
        scroll_timer(&mut server, mode);
    }
}
