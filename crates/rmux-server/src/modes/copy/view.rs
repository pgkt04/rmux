// Ported from tmux window-copy.c and input.c @ 8f25579c
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

//! Parser-backed and plain command-output views with anchored scrolling.

use super::state::{CopyBacking, CopyLineNumbers, CopyTimerAction};
use super::{init_common, render, state};
use crate::ids::{ModeId, PaneId};
use crate::model::{ModelError, Server};
use crate::server::event_loop::LoopAction;
use rmux_emu::cell::DEFAULT_CELL;
use rmux_emu::colour::Colour;
use rmux_emu::input::{InputCtx, InputEffect, InputPolicy, InputSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use std::time::Duration;

pub fn init_view(server: &mut Server, mode: ModeId) -> Option<Screen> {
    let pane = server.panes.get(mode.owner)?;
    let (sx, sy) = (pane.base.grid.sx(), pane.base.grid.sy());
    let backing = Screen::new(
        sx,
        sy,
        u32::MAX,
        ScreenResetPolicy::default(),
        &mut server.hyperlinks,
    )
    .ok()?;
    let visible = init_common(
        server,
        mode,
        CopyBacking::Output {
            screen: backing,
            parser: Box::new(InputCtx::new()),
            written: false,
            ground_timer: None,
        },
        mode.owner,
    )?;
    state::data_mut(server, mode)?.line_numbers = CopyLineNumbers::Off;
    Some(visible)
}
#[derive(Default)]
struct ViewEffects {
    ground: Option<bool>,
}
impl InputSink for ViewEffects {
    fn effect(&mut self, effect: InputEffect<'_>) {
        if let InputEffect::GroundTimer(arm) = effect {
            self.ground = Some(arm);
        }
    }
    fn reply(&mut self, _bytes: &[u8]) {}
}
pub fn append_output(server: &mut Server, mode: ModeId, bytes: &[u8]) -> Result<(), ModelError> {
    append(server, mode, false, bytes)
}
pub fn add(server: &mut Server, pane: PaneId, parse: bool, bytes: &[u8]) -> Result<(), ModelError> {
    let mode = server
        .panes
        .get(pane)
        .and_then(|p| p.modes.first())
        .ok_or(ModelError::StaleId)?
        .id;
    append(server, mode, parse, bytes)
}
fn append(server: &mut Server, mode: ModeId, parse: bool, bytes: &[u8]) -> Result<(), ModelError> {
    let (panes, registry) = (&mut server.panes, &mut server.hyperlinks);
    let entry = panes
        .get_mut(mode.owner)
        .and_then(|p| p.modes.iter_mut().find(|m| m.id == mode))
        .ok_or(ModelError::StaleId)?;
    let data = entry
        .data
        .as_mut()
        .and_then(|d| d.downcast_mut::<state::CopyModeData>())
        .ok_or(ModelError::StaleId)?;
    let CopyBacking::Output {
        screen,
        parser,
        written,
        ..
    } = &mut data.backing
    else {
        return Err(ModelError::StaleId);
    };
    let old_hsize = screen.grid.hsize();
    let mut sink = ScreenOnlySink;
    let mut effects = ViewEffects::default();
    let mut ctx = ScreenWriteCtx::start(
        screen,
        &mut sink,
        ScreenWritePolicy::default(),
        registry,
        #[cfg(feature = "sixel")]
        None,
    );
    if *written {
        ctx.carriagereturn();
        ctx.linefeed(false, Colour::DEFAULT);
    } else {
        *written = true;
    }
    let old_cy = ctx.screen.cy;
    if parse {
        parser.parse(
            &mut ctx,
            None,
            &InputPolicy::screen_only(),
            &mut effects,
            bytes,
        );
    } else {
        ctx.nputs(0, &DEFAULT_CELL, bytes);
    }
    let cy = ctx.screen.cy;
    ctx.finish();
    let hsize = screen.grid.hsize();
    data.oy = data.oy.wrapping_add(hsize.wrapping_sub(old_hsize));
    if let Some(arm) = effects.ground {
        let old = match &mut data.backing {
            CopyBacking::Output { ground_timer, .. } => ground_timer.take(),
            _ => None,
        };
        if let Some(timer) = old {
            server.event_loop.cancel(timer);
        }
        if arm {
            let timer = server.event_loop.schedule(
                Duration::from_secs(5),
                LoopAction::CopyTimer(CopyTimerAction::ParserGround(mode)),
            );
            if let Some(d) = state::data_mut(server, mode)
                && let CopyBacking::Output { ground_timer, .. } = &mut d.backing
            {
                *ground_timer = Some(timer);
            }
        }
    }
    if hsize != 0 {
        render::redraw_lines(server, mode, 0, 1);
    }
    render::redraw_lines(
        server,
        mode,
        old_cy,
        cy.wrapping_sub(old_cy).wrapping_add(1),
    );
    Ok(())
}
pub fn parser_ground_timer(server: &mut Server, mode: ModeId) {
    if let Some(d) = state::data_mut(server, mode)
        && let CopyBacking::Output {
            parser,
            ground_timer,
            ..
        } = &mut d.backing
    {
        *ground_timer = None;
        parser.ground_timeout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{pane, window};
    use crate::modes::WindowModeFlags;
    use crate::modes::copy::{CopyModeDriver, CopyModeKind};
    use std::rc::Rc;

    fn fixture() -> (Server, ModeId) {
        let mut server = Server::default();
        let window = window::window_create(&mut server, 8, 2, 0, 0).unwrap();
        let pane = pane::pane_create(&mut server, window, 8, 2, 10).unwrap();
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
        (server, mode)
    }
    #[test]
    fn appends_first_line_without_crlf_then_preserves_view_when_history_grows() {
        let (mut server, mode) = fixture();
        append_output(&mut server, mode, b"first").unwrap();
        let d = state::data(&server, mode).unwrap();
        assert_eq!(d.line_numbers, CopyLineNumbers::Off);
        assert_eq!(
            (d.backing.screen().cy, d.backing.screen().grid.hsize(), d.oy),
            (0, 0, 0)
        );
        append_output(&mut server, mode, b"second").unwrap();
        append_output(&mut server, mode, b"third").unwrap();
        let d = state::data(&server, mode).unwrap();
        assert_eq!(
            (d.backing.screen().cy, d.backing.screen().grid.hsize(), d.oy),
            (1, 1, 1)
        );
        assert_eq!(d.backing.screen().grid.get_cell(0, 0).data.data[0], b'f');
        assert_eq!(d.backing.screen().grid.get_cell(0, 2).data.data[0], b't');
    }
    #[test]
    fn parsed_output_uses_screen_only_policy_and_cancels_ground_timer() {
        let (mut server, mode) = fixture();
        append(&mut server, mode, true, b"\x1b[31mred\x1b]2;unfinished").unwrap();
        let d = state::data(&server, mode).unwrap();
        let CopyBacking::Output {
            parser,
            ground_timer,
            ..
        } = &d.backing
        else {
            panic!("output backing")
        };
        assert_ne!(parser.state_name(), "ground");
        assert!(ground_timer.is_some());
        parser_ground_timer(&mut server, mode);
        let d = state::data(&server, mode).unwrap();
        let CopyBacking::Output {
            parser,
            ground_timer,
            ..
        } = &d.backing
        else {
            panic!("output backing")
        };
        assert!(ground_timer.is_none());
        assert_eq!(parser.state_name(), InputCtx::new().state_name());
        pane::pane_reset_mode(&mut server, mode.owner).unwrap();
        assert!(state::data(&server, mode).is_none());
    }
}
