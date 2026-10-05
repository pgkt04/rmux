// Ported from tmux window-switch.c @ 8f25579c
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
use crate::cmd::arguments::Args;
use crate::cmd::find::{self, CmdFindFlags, CmdFindState};
use crate::cmd::parse;
use crate::cmd::queue::{QueueStateFlags, QueueStore};
use crate::format::draw::draw as format_draw;
use crate::format::fuzzy::{FuzzyMatch, fuzzy_match};
use crate::format::sort::{self, SortCriteria, SortOrder};
use crate::format::{FormatContext, FormatFlags, FormatTree};
use crate::ids::{ClientId, ModeId, PaneId, SessionId, WinlinkId};
use crate::model::pane::{PaneMode, PaneModeDriver, pane_reset_mode};
use crate::model::window::window_zoom;
use crate::model::winlink::winlink_find_by_index;
use crate::model::{ModelError, PaneFlags, WindowFlags};
use crate::modes::tree::mouse_at;
use crate::server::Server;
use crate::server::operations::{server_redraw_window, server_unzoom_window};
use crate::ui::prompt::{
    Prompt, PromptCreateData, PromptFlags, PromptHost, PromptKeyResult, PromptResult, PromptType,
    prompt_create, prompt_draw, prompt_free, prompt_incremental_start, prompt_key, prompt_mouse,
    prompt_set_options, prompt_update,
};
use crate::ui::status::status_message_set;
use crate::ui::styles::style_apply;
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_util::bytes::ByteString;
use rmux_util::key::{C0, KeyCode, KeyModifiers, MouseButtonBits, SpecialKey};
use std::cell::RefCell;
use std::cmp::Reverse;
use std::rc::Rc;

pub const NAME: &[u8] = b"switch-mode";
pub const DEFAULT_COMMAND: &[u8] = b"switch-client -Zt '%%'";
pub const DEFAULT_FORMAT: &[u8] = b"#{?window_format,\
#{window_name} \
#[dim]#{session_name}:#{window_index}#{window_flags}#[default] \
#[dim]#{pane_current_command}#[default] \
#[dim]#{?#{!=:#{pane_title},#{host_short}},#{pane_title},}#[default]\
,\
#{session_name} \
#[dim]#{session_windows} windows#[default] \
#{?session_attached,attached,#[dim]detached#[default]} \
#[dim]#{window_name}#[default]\
}";
const PROMPT: &[u8] = b"(search) ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitchType {
    Session,
    Window,
}

pub struct SwitchItem {
    pub kind: SwitchType,
    pub session: SessionId,
    pub winlink: i32,
    pub text: ByteString,
    pub matched: Option<FuzzyMatch>,
    pub score: u32,
    pub order: u32,
}

/// Filter text delivered by the prompt callback while the prompt itself is
/// borrowed by `prompt_key`; applied once the key returns.
type PendingFilter = Rc<RefCell<Option<Vec<u8>>>>;

pub struct SwitchModeData {
    pub wp: PaneId,
    pub zoomed: Option<bool>,
    pub format: Vec<u8>,
    pub command: Vec<u8>,
    pub kind: SwitchType,
    pub filter: Vec<u8>,
    pub prompt: Option<Prompt>,
    pub prompt_cx: u32,
    pending: PendingFilter,
    pub items: Vec<SwitchItem>,
    pub matches: Vec<u32>,
    pub current: usize,
    pub offset: usize,
    pub sx: u32,
    pub sy: u32,
}

pub struct SwitchMode {
    args: Args,
    fs: CmdFindState,
}

impl SwitchMode {
    pub fn new(args: Args, fs: CmdFindState) -> Self {
        Self { args, fs }
    }
}

struct SwitchPromptHost {
    pending: PendingFilter,
}

impl PromptHost for SwitchPromptHost {
    fn fire(
        &mut self,
        _srv: &mut Server,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        if key != PromptKeyResult::Handled {
            return PromptResult::Continue;
        }
        // The incremental prefix (prompt.c:269,1534) is one byte.
        let filter = match text {
            None => Vec::new(),
            Some([]) => Vec::new(),
            Some(s) => s[1..].to_vec(),
        };
        *self.pending.borrow_mut() = Some(filter);
        PromptResult::Continue
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

fn take_data(server: &mut Server, id: ModeId) -> Option<Box<SwitchModeData>> {
    let mode = mode_mut(server, id)?;
    let data = mode.data.take()?;
    match data.downcast::<SwitchModeData>() {
        Ok(data) => Some(data),
        Err(other) => {
            mode.data = Some(other);
            None
        }
    }
}

fn release_data(server: &mut Server, mut data: Box<SwitchModeData>) {
    if let Some(prompt) = data.prompt.take() {
        prompt_free(server, prompt);
    }
}

fn with_screen_and_data(
    server: &mut Server,
    id: ModeId,
    f: impl FnOnce(&mut Server, &mut SwitchModeData, &mut Screen),
) {
    let Some(mut data) = take_data(server, id) else {
        return;
    };
    let Some(mut screen) = mode_mut(server, id).and_then(|m| m.screen.take()) else {
        if let Some(mode) = mode_mut(server, id) {
            mode.data = Some(data);
        } else {
            release_data(server, data);
        }
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

fn add_item(
    server: &mut Server,
    data: &mut SwitchModeData,
    context: FormatContext,
    kind: SwitchType,
    session: SessionId,
    winlink: i32,
    order: &mut u32,
) {
    let mut ft = FormatTree::create(None, None, 0, FormatFlags::NONE, server);
    ft.defaults(server, context);
    let text = ft.expand(server, &data.format);
    ft.release(server);
    data.items.push(SwitchItem {
        kind,
        session,
        winlink,
        text,
        matched: None,
        score: 0,
        order: *order,
    });
    *order += 1;
}

/// `window_switch_build`: rebuild the item list and the filtered, sorted matches.
pub fn build(server: &mut Server, data: &mut SwitchModeData) {
    let sort_crit = SortCriteria::new(SortOrder::Name, false);
    data.items.clear();
    let mut order = 0;
    match data.kind {
        SwitchType::Session => {
            let mut sessions = Vec::new();
            sort::get_sessions(&*server, &sort_crit, &mut sessions);
            for s in sessions {
                let context = FormatContext {
                    session: Some(s),
                    ..FormatContext::default()
                };
                add_item(
                    server,
                    data,
                    context,
                    SwitchType::Session,
                    s,
                    -1,
                    &mut order,
                );
            }
        }
        SwitchType::Window => {
            let mut winlinks: Vec<WinlinkId> = Vec::new();
            sort::get_winlinks(&*server, &sort_crit, &mut winlinks);
            for wl in winlinks {
                let Some((session, index, window)) = server
                    .winlinks
                    .get(wl)
                    .map(|l| (l.session, l.index, l.window))
                else {
                    continue;
                };
                let context = FormatContext {
                    session: Some(session),
                    winlink: Some(wl),
                    window: Some(window),
                    ..FormatContext::default()
                };
                add_item(
                    server,
                    data,
                    context,
                    SwitchType::Window,
                    session,
                    index,
                    &mut order,
                );
            }
        }
    }
    data.matches = sort_matches(&mut data.items, &data.filter, data.sx);
}

/// Filter `items` by `filter` into indexes sorted by score descending, then
/// original order.
pub fn sort_matches(items: &mut [SwitchItem], filter: &[u8], sx: u32) -> Vec<u32> {
    let mut matches: Vec<u32> = Vec::new();
    for (n, item) in items.iter_mut().enumerate() {
        item.matched = None;
        item.score = 0;
        if filter.is_empty() {
            matches.push(n as u32);
            continue;
        }
        item.matched = fuzzy_match(filter, &item.text, sx);
        let Some(m) = &item.matched else {
            continue;
        };
        item.score = m.score;
        matches.push(n as u32);
    }
    if matches.len() > 1 {
        matches.sort_by_key(|&n| (Reverse(items[n as usize].score), items[n as usize].order));
    }
    matches
}

fn visible(data: &SwitchModeData) -> usize {
    if data.sy <= 1 {
        0
    } else {
        data.sy as usize - 1
    }
}

pub fn set_current(data: &mut SwitchModeData, current: usize) {
    let visible = visible(data);
    if data.matches.is_empty() {
        data.current = 0;
        data.offset = 0;
        return;
    }
    data.current = current.min(data.matches.len() - 1);
    if data.current < data.offset {
        data.offset = data.current;
    } else if visible != 0 && data.current >= data.offset + visible {
        data.offset = data.current - visible + 1;
    }
}

fn draw_screen(server: &mut Server, data: &mut SwitchModeData, screen: &mut Screen) {
    let sx = screen.grid.sx();
    let sy = screen.grid.sy();
    data.sx = sx;
    data.sy = sy;
    let Some(oo) = server.panes.get(data.wp).map(|p| p.options) else {
        return;
    };
    let mut mgc = GridCell::default();
    let mut sgc = GridCell::default();
    style_apply(server, &mut mgc, oo, b"switch-mode-match-style", None);
    style_apply(server, &mut sgc, oo, b"mode-style", None);
    let plan = data
        .prompt
        .as_ref()
        .map(|pr| prompt_draw(pr, server, 0, sx));

    let Server { hyperlinks, .. } = server;
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
    if sy <= 1 {
        ctx.finish();
        return;
    }
    let visible = visible(data);
    for i in 0..visible {
        let idx = data.offset + i;
        let Some(&item) = data.matches.get(idx) else {
            break;
        };
        let item = &data.items[item as usize];
        ctx.cursormove(0, i as i32, false);
        if idx != data.current {
            format_draw(&mut ctx, &DEFAULT_CELL, sx, &item.text, None, false);
        } else {
            ctx.clearendofline(sgc.bg);
            format_draw(&mut ctx, &sgc, sx, &item.text, None, false);
        }
        let Some(m) = &item.matched else {
            continue;
        };
        for j in 0..sx {
            if !m.columns.test(j as usize) {
                continue;
            }
            let mut gc = ctx.screen.grid.view_get_cell(j, i as u32);
            gc.attr = mgc.attr;
            gc.fg = mgc.fg;
            gc.bg = mgc.bg;
            ctx.cursormove(j as i32, i as i32, false);
            ctx.cell(&gc);
        }
    }
    if let (Some(plan), Some(pr)) = (plan, data.prompt.as_ref()) {
        let mut pdd = crate::ui::prompt::PromptDrawData {
            cursor_x: &mut data.prompt_cx,
            area_x: 0,
            area_width: sx,
            prompt_line: sy - 1,
        };
        ctx.screen.mode.insert(ScreenMode::CURSOR);
        plan.render(pr, &mut ctx, &mut pdd);
        ctx.cursormove(data.prompt_cx as i32, sy as i32 - 1, false);
    }
    ctx.finish();
}

/// Apply a filter delivered by the prompt callback (`window_switch_prompt_callback`).
fn apply_pending(server: &mut Server, data: &mut SwitchModeData) {
    let pending = data.pending.borrow_mut().take();
    if let Some(filter) = pending {
        data.filter = filter;
        build(server, data);
        data.current = 0;
        data.offset = 0;
    }
}

/// `window_switch_run_command`: true when the mode should exit.
fn run_command(server: &mut Server, data: &SwitchModeData, c: Option<ClientId>) -> bool {
    let Some(&n) = data.matches.get(data.current) else {
        return false;
    };
    let item = &data.items[n as usize];
    let Some(session) = server.sessions.get(item.session) else {
        return false;
    };
    let (target, fs) = match item.kind {
        SwitchType::Session => {
            let mut target = b"=".to_vec();
            target.extend_from_slice(&session.name);
            target.push(b':');
            (
                target,
                find::from_session(&*server, item.session, CmdFindFlags(0)),
            )
        }
        SwitchType::Window => {
            if item.winlink < 0 {
                return false;
            }
            let Some(wl) = winlink_find_by_index(server, item.session, item.winlink) else {
                return false;
            };
            let mut target = b"=".to_vec();
            target.extend_from_slice(&session.name);
            target.extend_from_slice(format!(":{}.", item.winlink).as_bytes());
            (target, find::from_winlink(&*server, wl, CmdFindFlags(0)))
        }
    };
    let command = crate::cmd::template_replace(&data.command, &target, 1);
    if !command.is_empty() {
        let mut queue = std::mem::replace(&mut server.queue, QueueStore::new());
        let state = queue.new_state(&*server, Some(&fs), None, QueueStateFlags::default());
        server.queue = queue;
        if let Ok(state) = state {
            let mut input = parse::CmdParseInput::default();
            if let Err(error) = parse::and_append(server, &command, &mut input, c, state) {
                if c.is_some() {
                    let mut message = error.message().to_vec();
                    if let Some(first) = message.first_mut() {
                        first.make_ascii_uppercase();
                    }
                    status_message_set(server, c, -1, true, false, false, &message);
                }
            }
            let _ = server.queue.free_state(state);
        }
    }
    true
}

fn redraw(server: &mut Server, data: &mut SwitchModeData, screen: &mut Screen) {
    draw_screen(server, data, screen);
    if let Some(p) = server.panes.get_mut(data.wp) {
        p.flags.insert(PaneFlags::REDRAW);
    }
}

/// Whether `key` finishes the mode. Draws when the key was handled locally.
fn handle_key(
    server: &mut Server,
    data: &mut SwitchModeData,
    screen: &mut Screen,
    c: ClientId,
    mut key: KeyCode,
    m: Option<&ResolvedMouseEvent>,
) -> bool {
    let mut current = data.current;
    let mut size = data.matches.len();
    let visible_rows = visible(data);

    if key.is_mouse() {
        let Some(m) = m else {
            return false;
        };
        let Some((x, y)) = mouse_at(server, data.wp, m) else {
            return false;
        };
        let b = MouseButtonBits(m.event.b);
        if data.prompt.is_some()
            && data.sy != 0
            && y == data.sy - 1
            && b.buttons() == 0
            && !b.is_drag()
            && !b.is_release()
        {
            let mut redraw_flag = false;
            let result = {
                let pr = data.prompt.as_mut().expect("prompt present");
                prompt_mouse(pr, server, x, 0, data.sx, &mut redraw_flag)
            };
            apply_pending(server, data);
            if redraw_flag || result == PromptKeyResult::Handled {
                redraw(server, data, screen);
            }
            return false;
        }
        match key.0 {
            SpecialKey::WHEELUP_PANE => {
                if size != 0 && current != 0 {
                    set_current(data, current - 1);
                }
            }
            SpecialKey::WHEELDOWN_PANE => {
                if size != 0 && current != size - 1 {
                    set_current(data, current + 1);
                }
            }
            SpecialKey::MOUSEDOWN1_PANE | SpecialKey::DOUBLECLICK1_PANE => {
                if y as usize >= visible_rows || data.offset + y as usize >= size {
                    return false;
                }
                set_current(data, data.offset + y as usize);
                if key.0 == SpecialKey::DOUBLECLICK1_PANE {
                    return run_command(server, data, Some(c));
                }
            }
            _ => return false,
        }
        redraw(server, data, screen);
        return false;
    }

    const CTRL: u64 = KeyModifiers::CTRL.0;
    if key.0 == u64::from(b'p') | CTRL || key.0 == u64::from(b'k') | CTRL {
        key = KeyCode(SpecialKey::UP);
    } else if key.0 == u64::from(b'n') | CTRL || key.0 == u64::from(b'j') | CTRL {
        key = KeyCode(SpecialKey::DOWN);
    }

    if key.0 == u64::from(b'\r') {
        return run_command(server, data, Some(c));
    }
    if key.0 == u64::from(C0::ESC)
        || key.0 == u64::from(b'[') | CTRL
        || key.0 == u64::from(b'c') | CTRL
        || key.0 == u64::from(b'g') | CTRL
    {
        return true;
    }

    if data.prompt.is_some() {
        let mut redraw_flag = false;
        let result = {
            let pr = data.prompt.as_mut().expect("prompt present");
            prompt_key(server, pr, key, &mut redraw_flag)
        };
        apply_pending(server, data);
        if redraw_flag {
            redraw(server, data, screen);
        }
        if result == PromptKeyResult::Handled || result == PromptKeyResult::NotHandled {
            return false;
        }
        current = data.current;
        size = data.matches.len();
    }

    match key.0 {
        SpecialKey::UP => {
            if size != 0 {
                set_current(data, if current == 0 { size - 1 } else { current - 1 });
            }
        }
        SpecialKey::DOWN => {
            if size != 0 {
                set_current(data, if current == size - 1 { 0 } else { current + 1 });
            }
        }
        SpecialKey::PPAGE => {
            set_current(data, current.saturating_sub(visible_rows));
        }
        SpecialKey::NPAGE => {
            set_current(data, current + visible_rows);
        }
        SpecialKey::HOME => set_current(data, 0),
        SpecialKey::END if size > 0 => set_current(data, size - 1),
        _ => {}
    }
    redraw(server, data, screen);
    false
}

impl PaneModeDriver for SwitchMode {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
        let wp = id.owner;
        let (sx, sy, w) = {
            let p = server.panes.get(wp)?;
            (p.base.grid.sx(), p.base.grid.sy(), p.window)
        };
        let kind = if self.args.has(b'w') != 0 {
            SwitchType::Window
        } else {
            SwitchType::Session
        };
        let format = self.args.get(b'F').map_or(DEFAULT_FORMAT, |f| f).to_vec();
        let command = if self.args.count() == 0 {
            DEFAULT_COMMAND.to_vec()
        } else {
            self.args.string(0).unwrap_or(DEFAULT_COMMAND).to_vec()
        };

        let pending: PendingFilter = Rc::new(RefCell::new(None));
        let oo = self
            .fs
            .s
            .and_then(|s| server.sessions.get(s))
            .map_or(server.options.global_s, |s| s.options);
        let mut pd = PromptCreateData::default();
        prompt_set_options(server, &mut pd, oo);
        pd.fs = Some(self.fs);
        pd.prompt = PROMPT.into();
        pd.input = Some(ByteString::new());
        pd.ty = PromptType::Search;
        pd.flags = PromptFlags::INCREMENTAL
            | PromptFlags::NOFORMAT
            | PromptFlags::ISMODE
            | PromptFlags::EDITARROWS;
        let mut prompt = prompt_create(
            server,
            pd,
            Box::new(SwitchPromptHost {
                pending: Rc::clone(&pending),
            }),
        );
        prompt_update(&mut prompt, server, PROMPT, Some(b""));

        let mut screen = Screen::new(
            sx,
            sy,
            0,
            ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .ok()?;

        let mut data = Box::new(SwitchModeData {
            wp,
            zoomed: None,
            format,
            command,
            kind,
            filter: Vec::new(),
            prompt: Some(prompt),
            prompt_cx: 0,
            pending,
            items: Vec::new(),
            matches: Vec::new(),
            current: 0,
            offset: 0,
            sx,
            sy,
        });

        if self.args.has(b'Z') != 0 {
            let zoomed = server
                .windows
                .get(w)
                .is_some_and(|w| w.flags.contains(WindowFlags::ZOOMED));
            data.zoomed = Some(zoomed);
            if !zoomed {
                let mode = mode_mut(server, id)?;
                mode.data = Some(data);
                mode.screen = Some(screen);
                if window_zoom(server, w, wp).unwrap_or(false) {
                    server_redraw_window(server, w);
                }
                let mode = mode_mut(server, id)?;
                data = mode.data.take()?.downcast::<SwitchModeData>().ok()?;
                screen = mode.screen.take()?;
            }
        }

        build(server, &mut data);
        {
            let pr = data.prompt.as_mut().expect("prompt present");
            prompt_incremental_start(server, pr);
        }
        apply_pending(server, &mut data);
        draw_screen(server, &mut data, &mut screen);

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
            .and_then(|d| d.downcast::<SwitchModeData>().ok())
        {
            if data.zoomed == Some(false)
                && let Some(w) = w
            {
                let _ = server_unzoom_window(server, w);
            }
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
            data.sx = sx;
            data.sy = sy;
            build(server, data);
            let current = data.current;
            set_current(data, current);
            draw_screen(server, data, screen);
        });
    }

    fn key(
        &self,
        server: &mut Server,
        id: ModeId,
        client: ClientId,
        key: KeyCode,
        mouse: Option<&ResolvedMouseEvent>,
    ) {
        let mut finished = false;
        with_screen_and_data(server, id, |server, data, screen| {
            finished = handle_key(server, data, screen, client, key, mouse);
        });
        if finished {
            let _ = pane_reset_mode(server, id.owner);
        }
    }

    fn append_output(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _bytes: &[u8],
    ) -> Result<(), ModelError> {
        Err(ModelError::Message(b"switch-mode has no output".to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(order: u32, text: &str) -> SwitchItem {
        SwitchItem {
            kind: SwitchType::Session,
            session: <SessionId as crate::ids::ArenaId>::from_parts(order, 0),
            winlink: -1,
            text: text.into(),
            matched: None,
            score: 0,
            order,
        }
    }

    #[test]
    fn equal_scores_keep_original_order() {
        let mut items = vec![
            item(0, "alpha"),
            item(1, "alpha"),
            item(2, "alpha"),
            item(3, "zzz"),
        ];
        let matches = sort_matches(&mut items, b"alp", 80);
        assert_eq!(matches, vec![0, 1, 2]);
        assert!(items[0].matched.is_some());
        assert!(items[3].matched.is_none());
        // No filter: everything in order and no match data.
        let all = sort_matches(&mut items, b"", 80);
        assert_eq!(all, vec![0, 1, 2, 3]);
        assert!(
            items
                .iter()
                .all(|item| item.matched.is_none() && item.score == 0)
        );
    }

    #[test]
    fn better_score_sorts_first() {
        let mut items = vec![item(0, "x a b c"), item(1, "abc")];
        let matches = sort_matches(&mut items, b"abc", 80);
        assert_eq!(matches.len(), 2);
        assert!(items[1].score >= items[0].score);
        assert_eq!(
            matches[0],
            if items[1].score > items[0].score {
                1
            } else {
                0
            }
        );
    }

    fn data(n: usize, sy: u32) -> SwitchModeData {
        SwitchModeData {
            wp: <PaneId as crate::ids::ArenaId>::from_parts(0, 0),
            zoomed: None,
            format: Vec::new(),
            command: Vec::new(),
            kind: SwitchType::Session,
            filter: Vec::new(),
            prompt: None,
            prompt_cx: 0,
            pending: Rc::new(RefCell::new(None)),
            items: Vec::new(),
            matches: (0..n as u32).collect(),
            current: 0,
            offset: 0,
            sx: 80,
            sy,
        }
    }

    #[test]
    fn set_current_clamps_and_scrolls() {
        let mut d = data(10, 5);
        set_current(&mut d, 20);
        assert_eq!((d.current, d.offset), (9, 6));
        set_current(&mut d, 2);
        assert_eq!((d.current, d.offset), (2, 2));
        let mut empty = data(0, 5);
        empty.current = 3;
        set_current(&mut empty, 1);
        assert_eq!((empty.current, empty.offset), (0, 0));
    }
    struct StringOnlyParser;

    impl crate::cmd::parse::CommandParser for StringOnlyParser {
        fn parse_from_string(&mut self, _: &[u8]) -> crate::cmd::parse::CmdParseResult {
            panic!("string option must not parse commands")
        }
    }

    #[test]
    fn fuzzy_highlight_replaces_rendition_without_changing_text() {
        use rmux_emu::cell::GridAttributes;
        let mut server = Server::default();
        let window = crate::model::window::window_create(&mut server, 40, 5, 0, 0).unwrap();
        let wp = crate::model::pane::pane_create(&mut server, window, 40, 5, 0).unwrap();
        let oo = server.panes.get(wp).unwrap().options;
        server.options.set_string(
            oo,
            b"switch-mode-match-style",
            false,
            b"fg=red,bg=blue,bold",
            &mut StringOnlyParser,
        );
        server.options.set_string(
            oo,
            b"mode-style",
            false,
            b"fg=yellow,bg=green",
            &mut StringOnlyParser,
        );
        let mut d = data(0, 5);
        d.wp = wp;
        d.items = vec![item(0, "#[dim]alpha#[default] beta"), item(1, "alpha")];
        d.matches = sort_matches(&mut d.items, b"alp", 40);
        let mut screen = Screen::new(
            40,
            5,
            0,
            ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .unwrap();
        draw_screen(&mut server, &mut d, &mut screen);
        for y in 0..2 {
            for x in 0..3 {
                let gc = screen.grid.view_get_cell(x, y);
                assert_eq!(gc.data.data[0], b"alp"[x as usize]);
                assert_eq!(gc.fg, Colour::from_raw(1));
                assert_eq!(gc.bg, Colour::from_raw(4));
                assert_eq!(gc.attr, GridAttributes::BRIGHT);
            }
        }
        assert_eq!(screen.grid.view_get_cell(39, 0).bg, Colour::from_raw(2));
        assert_eq!(screen.grid.view_get_cell(39, 1).bg, Colour::DEFAULT);
        screen
            .release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
    }

    #[test]
    fn masked_control_keys_cancel_and_keyboard_navigation_wraps() {
        use crate::ids::ArenaId;
        let mut server = Server::default();
        let window = crate::model::window::window_create(&mut server, 40, 5, 0, 0).unwrap();
        let wp = crate::model::pane::pane_create(&mut server, window, 40, 5, 0).unwrap();
        let client = ClientId::from_parts(0, 0);
        let mut d = data(2, 5);
        d.wp = wp;
        d.items = vec![item(0, "a"), item(1, "b")];
        let mut screen = Screen::new(
            40,
            5,
            0,
            ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .unwrap();
        for key in [b"Escape".as_slice(), b"C-[", b"C-c", b"C-g"] {
            assert!(handle_key(
                &mut server,
                &mut d,
                &mut screen,
                client,
                rmux_tty::key_string::parse_key_name(key),
                None
            ));
        }
        for (key, expected) in [
            (b"C-p".as_slice(), 1),
            (b"C-n", 0),
            (b"C-k", 1),
            (b"C-j", 0),
        ] {
            assert!(!handle_key(
                &mut server,
                &mut d,
                &mut screen,
                client,
                rmux_tty::key_string::parse_key_name(key),
                None
            ));
            assert_eq!(d.current, expected);
        }
        assert!(
            !run_command(&mut server, &d, Some(client)),
            "stale target keeps mode active"
        );
        d.matches.clear();
        assert!(
            !run_command(&mut server, &d, Some(client)),
            "no match keeps mode active"
        );
        screen
            .release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
    }
}
