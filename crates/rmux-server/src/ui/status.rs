// Ported from tmux tmux.h, status.c @ 8f25579c
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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PaneStatusPosition {
    Off = 0,
    Top = 1,
    Bottom = 2,
    TopFloating = 3,
    BottomFloating = 4,
}
impl TryFrom<i32> for PaneStatusPosition {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Top),
            2 => Ok(Self::Bottom),
            3 => Ok(Self::TopFloating),
            4 => Ok(Self::BottomFloating),
            _ => Err(value),
        }
    }
}

use crate::client::Client;
use crate::client::ClientFlags;
use crate::ids::{ClientId, SessionId, TimerId};
use crate::model::Server;
use crate::options::OptionsArrayKey;
use crate::server::event_loop::LoopAction;
use crate::ui::prompt::{
    PromptCreateData, PromptDrawData, PromptFlags, PromptHost, PromptKeyResult, PromptResult,
    PromptType, prompt_closed, prompt_create, prompt_draw, prompt_incremental_start, prompt_key,
    prompt_mouse, prompt_set_options, prompt_update, status_slot_restore,
};
use crate::ui::styles;
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_emu::style::{StyleAlign, StyleLineEntry, StyleRange};
use rmux_tty::tty::TtyFlags;
use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, MouseButtonBits, MouseEvent};
use rmux_util::log_debug;
use std::time::Duration;

pub const STATUS_LINES_LIMIT: usize = 5;

/// Status line state of a client. The status screens own a private
/// hyperlink registry so they can be drawn while the server is borrowed.
pub struct StatusLine {
    pub timer: Option<TimerId>,
    pub registry: HyperlinkRegistry,
    pub screen: Screen,
    /// The pushed overlay screen; None means `screen` is active.
    pub active: Option<Screen>,
    pub references: u32,
    pub prompt_cx: u32,
    pub style: GridCell,
    pub entries: [StyleLineEntry; STATUS_LINES_LIMIT],
}

impl Default for StatusLine {
    fn default() -> Self {
        let mut registry = HyperlinkRegistry::new();
        let screen = Screen::new(1, 1, 0, ScreenResetPolicy::default(), &mut registry)
            .expect("status screen");
        Self {
            timer: None,
            registry,
            screen,
            active: None,
            references: 0,
            prompt_cx: 0,
            style: DEFAULT_CELL,
            entries: Default::default(),
        }
    }
}

impl StatusLine {
    /// `c->status.active`.
    pub fn active(&self) -> &Screen {
        self.active.as_ref().unwrap_or(&self.screen)
    }
}

/// Client-lifetime message state; only the text comes and goes.
#[derive(Default)]
pub struct StatusMessage {
    pub text: Option<ByteString>,
    pub ignore_keys: bool,
    pub ignore_styles: bool,
    pub timer: Option<TimerId>,
}

fn client_session(srv: &Server, c: ClientId) -> Option<SessionId> {
    srv.clients.get(c)?.session
}

/// Status timer callback.
pub fn status_timer_fire(srv: &mut Server, c: ClientId) {
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    if let Some(t) = client.status.timer.take() {
        srv.event_loop.cancel(t);
    }
    let Some(s) = client_session(srv, c) else {
        return;
    };
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    if client.message.text.is_none() && !client.prompt.is_some() {
        client.flags.insert(ClientFlags::REDRAWSTATUS);
    }
    let oo = srv.sessions.get(s).map(|s| s.options);
    let secs = oo.map_or(0, |oo| srv.options.get_number(oo, b"status-interval"));
    if secs != 0 {
        let id = srv.event_loop.schedule(
            Duration::from_secs(secs.max(0) as u64),
            LoopAction::StatusTimer(c),
        );
        if let Some(client) = srv.clients.get_mut(c) {
            client.status.timer = Some(id);
        }
    }
    log_debug!("client {:?}, status interval {}", c, secs);
}

/// Start status timer for client.
pub fn status_timer_start(srv: &mut Server, c: ClientId) {
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    if let Some(t) = client.status.timer.take() {
        srv.event_loop.cancel(t);
    }
    let Some(s) = client_session(srv, c) else {
        return;
    };
    let oo = srv.sessions.get(s).map(|s| s.options);
    if oo.is_some_and(|oo| srv.options.get_number(oo, b"status") != 0) {
        status_timer_fire(srv, c);
    }
}

/// Start status timer for all clients.
pub fn status_timer_start_all(srv: &mut Server) {
    let clients: Vec<ClientId> = srv.client_order.iter().copied().collect();
    for c in clients {
        status_timer_start(srv, c);
    }
}

/// Update status cache.
pub fn status_update_cache(srv: &mut Server, s: SessionId) {
    let Some(session) = srv.sessions.get(s) else {
        return;
    };
    let oo = session.options;
    // Model fixtures build option trees without the table defaults loaded;
    // fall back to the table values (status on, position bottom) like
    // model::window::option_number does.
    let lines = crate::model::window::option_number(srv, oo, b"status", 1).max(0) as u32;
    let at = if lines == 0 {
        -1
    } else if crate::model::window::option_number(srv, oo, b"status-position", 1) == 0 {
        0
    } else {
        1
    };
    if let Some(session) = srv.sessions.get_mut(s) {
        session.statuslines = lines;
        session.statusat = at;
    }
}

/// Get screen line of status line. -1 means off.
pub fn status_at_line(srv: &Server, c: ClientId) -> i32 {
    let Some(client) = srv.clients.get(c) else {
        return -1;
    };
    if client
        .flags
        .intersects(ClientFlags::STATUSOFF | ClientFlags::CONTROL)
    {
        return -1;
    }
    let Some(s) = client.session.and_then(|s| srv.sessions.get(s)) else {
        return -1;
    };
    if s.statusat != 1 {
        return s.statusat;
    }
    let (_, sy) = styles::client_size(srv, c);
    sy as i32 - status_line_size(srv, c) as i32
}

/// Get size of status line for client's session. 0 means off.
pub fn status_line_size(srv: &Server, c: ClientId) -> u32 {
    let Some(client) = srv.clients.get(c) else {
        return 0;
    };
    if client
        .flags
        .intersects(ClientFlags::STATUSOFF | ClientFlags::CONTROL)
    {
        return 0;
    }
    match client.session.and_then(|s| srv.sessions.get(s)) {
        None => srv
            .options
            .get_number(srv.options.global_s, b"status")
            .max(0) as u32,
        Some(s) => s.statuslines,
    }
}

/// Get the prompt line number for client's session.
pub fn status_prompt_line_at(srv: &Server, c: ClientId) -> u32 {
    let lines = status_line_size(srv, c);
    if lines == 0 {
        return 0;
    }
    let oo = styles::session_options(srv, c);
    let line = srv.options.get_number(oo, b"message-line").max(0) as u32;
    if line >= lines {
        return lines - 1;
    }
    line
}

/// Get the style range at a status-screen position.
pub fn status_get_range(c: &Client, x: u32, y: u32) -> Option<&StyleRange> {
    if y as usize >= STATUS_LINES_LIMIT {
        return None;
    }
    c.status.entries[y as usize].ranges.get_range(x)
}

/// Save old status line.
fn status_push_screen(srv: &mut Server, c: ClientId) {
    let lines = status_line_size(srv, c);
    let (sx, _) = styles::client_size(srv, c);
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    let sl = &mut client.status;
    if sl.active.is_none() {
        sl.active = Some(
            Screen::new(
                sx.max(1),
                lines.max(1),
                0,
                ScreenResetPolicy::default(),
                &mut sl.registry,
            )
            .expect("status overlay screen"),
        );
    }
    sl.references += 1;
}

/// Restore old status line.
fn status_pop_screen(srv: &mut Server, c: ClientId) {
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    let sl = &mut client.status;
    sl.references = sl.references.saturating_sub(1);
    if sl.references == 0 {
        if let Some(mut s) = sl.active.take() {
            let _ = s.release(
                &mut sl.registry,
                #[cfg(feature = "sixel")]
                None,
            );
        }
    }
}

/// Initialize status line.
pub fn status_init(c: &mut Client) {
    let sx = c.tty.as_ref().map_or(c.tty_sx, |t| t.size().0);
    let sl = &mut c.status;
    for e in &mut sl.entries {
        e.ranges.clear();
    }
    sl.screen.resize(
        sx.max(1),
        1,
        false,
        #[cfg(feature = "sixel")]
        None,
    );
    if let Some(mut s) = sl.active.take() {
        let _ = s.release(
            &mut sl.registry,
            #[cfg(feature = "sixel")]
            None,
        );
    }
    sl.references = 0;
}

/// Free status line.
pub fn status_free(srv: &mut Server, c: ClientId) {
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    let sl = &mut client.status;
    for e in &mut sl.entries {
        e.ranges.clear();
        e.expanded = ByteString::new();
    }
    if let Some(t) = sl.timer.take() {
        srv.event_loop.cancel(t);
    }
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    let sl = &mut client.status;
    if let Some(mut s) = sl.active.take() {
        let _ = s.release(
            &mut sl.registry,
            #[cfg(feature = "sixel")]
            None,
        );
    }
    let _ = sl.screen.release(
        &mut sl.registry,
        #[cfg(feature = "sixel")]
        None,
    );
}

/// Draw status line for client. Returns true when it changed.
pub fn status_redraw(srv: &mut Server, c: ClientId) -> bool {
    log_debug!("status_redraw enter");
    let Some(client) = srv.clients.get(c) else {
        return false;
    };
    if client.status.active.is_some() {
        rmux_util::fatalx!("not the active screen");
    }
    let Some(s) = client.session else {
        return true;
    };
    let (width, tty_sy) = styles::client_size(srv, c);
    let lines = status_line_size(srv, c);
    if tty_sy == 0 || lines == 0 {
        return true;
    }
    let oo = srv
        .sessions
        .get(s)
        .map(|s| s.options)
        .unwrap_or(srv.options.global_s);

    let mut flags = crate::format::FormatFlags::STATUS;
    if client.flags.contains(ClientFlags::STATUSFORCE) {
        flags.insert(crate::format::FormatFlags::FORCE);
    }
    let mut ft = crate::format::FormatTree::create(Some(c), None, 0, flags, srv);
    ft.defaults(
        srv,
        crate::format::FormatContext {
            evaluated_client: Some(c),
            ..Default::default()
        },
    );

    let mut gc = DEFAULT_CELL;
    styles::style_apply(srv, &mut gc, oo, b"status-style", Some(&mut ft));
    let fg = Colour(srv.options.get_number(oo, b"status-fg") as i32);
    if !fg.is_default() {
        gc.fg = fg;
    }
    let bg = Colour(srv.options.get_number(oo, b"status-bg") as i32);
    if !bg.is_default() {
        gc.bg = bg;
    }

    // Expand every present status-format line first so the server is free
    // while the status screen is written.
    let mut expanded: Vec<Option<ByteString>> = Vec::with_capacity(lines as usize);
    let has_format = srv.options.get(oo, b"status-format").is_some();
    if has_format {
        for i in 0..lines {
            let value = srv
                .options
                .get(oo, b"status-format")
                .and_then(|(_, o)| o.array_get(&OptionsArrayKey::Index(i)))
                .map(|v| v.as_string().to_vec());
            expanded.push(value.map(|v| ft.expand_time(srv, &v)));
        }
    }
    ft.release(srv);

    let Some(client) = srv.clients.get_mut(c) else {
        return false;
    };
    let sl = &mut client.status;
    let mut force = false;
    let mut changed = false;
    if !gc.cells_equal(&sl.style) {
        force = true;
        sl.style = gc;
    }
    if sl.screen.grid.sx() != width || sl.screen.grid.sy() != lines {
        sl.screen.resize(
            width,
            lines,
            false,
            #[cfg(feature = "sixel")]
            None,
        );
        changed = true;
        force = true;
    }
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut sl.screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut sl.registry,
        #[cfg(feature = "sixel")]
        None,
    );
    let blank = |ctx: &mut ScreenWriteCtx<'_>, n: u32| {
        let mut sp = gc;
        sp.data = rmux_util::utf8::Utf8Data::set(b' ');
        for _ in 0..n {
            ctx.cell(&sp);
        }
    };
    if !has_format {
        blank(&mut ctx, width * lines);
    } else {
        for i in 0..lines {
            ctx.cursormove(0, i as i32, false);
            let Some(text) = expanded[i as usize].take() else {
                blank(&mut ctx, width);
                continue;
            };
            let sle = &mut sl.entries[i as usize];
            if !force && !sle.expanded.is_empty() && sle.expanded.as_bytes() == text.as_bytes() {
                continue;
            }
            changed = true;
            blank(&mut ctx, width);
            ctx.cursormove(0, i as i32, false);
            sle.ranges.clear();
            crate::format::draw::draw(&mut ctx, &gc, width, &text, Some(&mut sle.ranges), false);
            sle.expanded = text;
        }
    }
    ctx.finish();
    log_debug!("status_redraw exit: force={}, changed={}", force, changed);
    force || changed
}

/// Escape # characters in a string so format_draw treats them as literal.
pub fn status_message_escape(s: &[u8]) -> ByteString {
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        if b == b'#' {
            out.push(b'#');
        }
        out.push(b);
    }
    out.into()
}

/// Set a status line message.
pub fn status_message_set(
    srv: &mut Server,
    c: Option<ClientId>,
    mut delay: i32,
    ignore_styles: bool,
    ignore_keys: bool,
    no_freeze: bool,
    text: &[u8],
) {
    log_debug!("status_message_set: {}", String::from_utf8_lossy(text));
    let Some(c) = c else {
        let mut m = b"message: ".to_vec();
        m.extend_from_slice(text);
        crate::server::run::add_message(srv, &m);
        return;
    };
    if srv.clients.get(c).is_none() {
        return;
    }
    status_message_clear(srv, c);
    status_push_screen(srv, c);
    let name = srv
        .clients
        .get(c)
        .and_then(|cl| cl.name.clone())
        .unwrap_or_default();
    if let Some(client) = srv.clients.get_mut(c) {
        client.message.text = Some(text.into());
    }
    let mut m = name;
    m.extend_from_slice(b" message: ");
    m.extend_from_slice(text);
    crate::server::run::add_message(srv, &m);

    if delay == -1 {
        let oo = styles::session_options(srv, c);
        delay = srv.options.get_number(oo, b"display-time") as i32;
    }
    if delay > 0 {
        if let Some(t) = srv
            .clients
            .get_mut(c)
            .and_then(|cl| cl.message.timer.take())
        {
            srv.event_loop.cancel(t);
        }
        let id = srv.event_loop.schedule(
            Duration::from_millis(delay as u64),
            LoopAction::MessageTimer(c),
        );
        if let Some(client) = srv.clients.get_mut(c) {
            client.message.timer = Some(id);
        }
    }
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    if delay != 0 {
        client.message.ignore_keys = ignore_keys;
    }
    client.message.ignore_styles = ignore_styles;
    if let Some(tty) = client.tty.as_mut() {
        if !no_freeze {
            tty.flags_mut().insert(TtyFlags::FREEZE);
        }
        tty.flags_mut().insert(TtyFlags::NOCURSOR);
    }
    client.flags.insert(ClientFlags::REDRAWSTATUS);
}

/// Clear status line message.
pub fn status_message_clear(srv: &mut Server, c: ClientId) {
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    if client.message.text.take().is_none() {
        return;
    }
    if !client.prompt.is_some() {
        if let Some(tty) = client.tty.as_mut() {
            tty.flags_mut()
                .remove(TtyFlags::NOCURSOR | TtyFlags::FREEZE);
        }
    }
    client.flags.insert(ClientFlags::ALLREDRAWFLAGS); /* was frozen and may have changed */
    status_pop_screen(srv, c);
}

/// Message timer expiry.
pub fn status_message_expire(srv: &mut Server, c: ClientId) {
    if let Some(client) = srv.clients.get_mut(c) {
        client.message.timer = None;
    }
    status_message_clear(srv, c);
}

/// Calculate prompt/message area geometry from the style's width and align
/// directives: x offset and available width within the status line.
fn status_message_area(srv: &mut Server, c: ClientId) -> (u32, u32) {
    let (tty_sx, _) = styles::client_size(srv, c);
    let oo = styles::session_options(srv, c);
    let sy = styles::option_style(srv, oo, b"message-style", None);
    let mut w = match sy {
        Some(sy) if sy.width >= 0 => {
            if sy.width_percentage {
                (tty_sx * sy.width as u32) / 100
            } else {
                sy.width as u32
            }
        }
        _ => tty_sx,
    };
    if w == 0 || w > tty_sx {
        w = tty_sx;
    }
    let area_x = match sy.map(|s| s.align) {
        Some(StyleAlign::Centre) | Some(StyleAlign::AbsoluteCentre) => (tty_sx - w) / 2,
        Some(StyleAlign::Right) => tty_sx - w,
        _ => 0,
    };
    (area_x, w)
}

/// Reset the pushed overlay screen to the client size.
fn status_begin_overlay(sl: &mut StatusLine, tty_sx: u32, lines: u32) -> Screen {
    let fresh = Screen::new(
        tty_sx,
        lines,
        0,
        ScreenResetPolicy::default(),
        &mut sl.registry,
    )
    .expect("status overlay screen");
    std::mem::replace(
        sl.active
            .as_mut()
            .expect("overlay owns a pushed status screen"),
        fresh,
    )
}

fn status_finish_overlay(sl: &mut StatusLine, mut old: Screen) -> bool {
    let changed = sl.active().grid.compare(&old.grid);
    let _ = old.release(
        &mut sl.registry,
        #[cfg(feature = "sixel")]
        None,
    );
    changed
}

/// Draw client message on status line of present else on last line.
pub fn status_message_redraw(srv: &mut Server, c: ClientId) -> bool {
    let (tty_sx, tty_sy) = styles::client_size(srv, c);
    if tty_sx == 0 || tty_sy == 0 {
        return false;
    }
    let Some(client) = srv.clients.get(c) else {
        return false;
    };
    let Some(message) = client.message.text.clone() else {
        return false;
    };
    let ignore_styles = client.message.ignore_styles;
    let mut lines = status_line_size(srv, c);
    if lines <= 1 {
        lines = 1;
    }
    let mut messageline = status_prompt_line_at(srv, c);
    if messageline > lines - 1 {
        messageline = lines - 1;
    }
    let (ax, aw) = status_message_area(srv, c);
    let oo = styles::session_options(srv, c);

    let mut ft = styles::create_defaults(srv, None, Some(c), None, None, None);
    let mut gc = DEFAULT_CELL;
    styles::style_apply(srv, &mut gc, oo, b"message-style", Some(&mut ft));
    if ignore_styles {
        ft.add(b"message", status_message_escape(&message));
    } else {
        ft.add(b"message", message);
    }
    ft.add(b"command_prompt", ByteString::from(&b"0"[..]));
    let msgfmt = srv.options.get_string(oo, b"message-format").to_vec();
    let expanded = ft.expand_time(srv, &msgfmt);
    ft.release(srv);

    let Some(client) = srv.clients.get_mut(c) else {
        return false;
    };
    let sl = &mut client.status;
    let old = status_begin_overlay(sl, tty_sx, lines);
    {
        let StatusLine {
            registry,
            screen,
            active,
            ..
        } = sl;
        let active = active
            .as_mut()
            .expect("message owns a pushed status screen");
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            active,
            &mut sink,
            ScreenWritePolicy::default(),
            registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.fast_copy(screen, 0, 0, tty_sx, lines);
        ctx.cursormove(ax as i32, messageline as i32, false);
        crate::format::draw::draw(&mut ctx, &gc, aw, &expanded, None, false);
        ctx.finish();
    }
    status_finish_overlay(sl, old)
}

/// Owned status prompt continuation (status_prompt_input_cb + free cb).
pub trait StatusPromptInput {
    fn fire(
        &mut self,
        srv: &mut Server,
        c: ClientId,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult;
    fn take_update(&mut self) -> Option<(ByteString, ByteString)> {
        None
    }
    /// prompt_free_cb: called exactly once before the continuation drops.
    fn free(&mut self, _srv: &mut Server) {}
}

impl<F> StatusPromptInput for F
where
    F: FnMut(&mut Server, ClientId, Option<&[u8]>, PromptKeyResult) -> PromptResult,
{
    fn fire(
        &mut self,
        srv: &mut Server,
        c: ClientId,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        self(srv, c, text, key)
    }
}

struct StatusPromptHost {
    c: ClientId,
    input: Option<Box<dyn StatusPromptInput>>,
}

impl PromptHost for StatusPromptHost {
    fn fire(
        &mut self,
        srv: &mut Server,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        match self.input.as_mut() {
            Some(input) => input.fire(srv, self.c, text, key),
            None => PromptResult::Close,
        }
    }
    fn take_update(&mut self) -> Option<(ByteString, ByteString)> {
        self.input.as_mut().and_then(|input| input.take_update())
    }
    fn free(&mut self, srv: &mut Server) {
        if let Some(mut input) = self.input.take() {
            input.free(srv);
        }
    }
}

/// Enable status line prompt.
#[allow(clippy::too_many_arguments)]
pub fn status_prompt_set(
    srv: &mut Server,
    c: ClientId,
    fs: Option<&crate::cmd::find::CmdFindState>,
    msg: &[u8],
    input: Option<&[u8]>,
    input_cb: Box<dyn StatusPromptInput>,
    flags: PromptFlags,
    ty: PromptType,
) {
    if srv.clients.get(c).is_none() {
        return;
    }
    status_message_clear(srv, c);
    status_prompt_clear(srv, c);
    status_push_screen(srv, c);

    let oo = styles::session_options(srv, c);
    let mut pd = PromptCreateData::default();
    prompt_set_options(srv, &mut pd, oo);
    pd.fs = fs.copied();
    pd.prompt = msg.into();
    pd.input = input.map(ByteString::from);
    pd.ty = ty;
    pd.flags = flags;
    let host = Box::new(StatusPromptHost {
        c,
        input: Some(input_cb),
    });
    let mut prompt = prompt_create(srv, pd, host);

    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    if !flags.contains(PromptFlags::INCREMENTAL) && !flags.contains(PromptFlags::NOFREEZE) {
        if let Some(tty) = client.tty.as_mut() {
            tty.flags_mut().insert(TtyFlags::FREEZE);
        }
    }
    client.flags.insert(ClientFlags::REDRAWSTATUS);
    let generation = client.prompt.install_pending();

    prompt_incremental_start(srv, &mut prompt);
    status_slot_restore(srv, c, prompt, generation);

    if flags.contains(PromptFlags::SINGLE) && flags.contains(PromptFlags::ACCEPT) {
        let cb = crate::cmd::queue::callback_for::<Server>(move |srv, _item| {
            if srv.clients.get(c).is_some_and(|cl| cl.prompt.is_some()) {
                status_prompt_key(srv, c, KeyCode(b'y' as u64), None);
            }
            crate::cmd::queue::CmdReturn::Normal
        });
        if let Ok(batch) = srv.queue.get_callback("status_prompt_accept", cb) {
            let _ = crate::cmd::queue::append(srv, Some(c), batch);
        }
    }
}

/// Remove status line prompt.
pub fn status_prompt_clear(srv: &mut Server, c: ClientId) {
    let Some(client) = srv.clients.get_mut(c) else {
        return;
    };
    if !client.prompt.is_some() {
        return;
    }
    let old = client.prompt.clear();
    if let Some(tty) = client.tty.as_mut() {
        tty.flags_mut()
            .remove(TtyFlags::NOCURSOR | TtyFlags::FREEZE);
    }
    client.flags.insert(ClientFlags::ALLREDRAWFLAGS); /* was frozen and may have changed */
    if let Some(old) = old {
        crate::ui::prompt::prompt_free(srv, old);
    }
    status_pop_screen(srv, c);
}

/// Update status line prompt with a new prompt string.
pub fn status_prompt_update(srv: &mut Server, c: ClientId, msg: &[u8], input: Option<&[u8]>) {
    let Some((mut prompt, generation)) =
        srv.clients.get_mut(c).and_then(|cl| cl.prompt.take_value())
    else {
        return;
    };
    prompt_update(&mut prompt, srv, msg, input);
    status_slot_restore(srv, c, prompt, generation);
    if let Some(client) = srv.clients.get_mut(c) {
        client.flags.insert(ClientFlags::REDRAWSTATUS);
    }
}

/// Get the screen line on which the prompt is drawn.
fn status_prompt_screen_line(srv: &Server, c: ClientId) -> u32 {
    let oo = styles::session_options(srv, c);
    if srv.options.get_number(oo, b"status-position") == 0 {
        return status_prompt_line_at(srv, c);
    }
    let (_, tty_sy) = styles::client_size(srv, c);
    let n = status_line_size(srv, c) - status_prompt_line_at(srv, c);
    if n <= tty_sy {
        return tty_sy - n;
    }
    tty_sy - 1
}

/// Draw client prompt on status line of present else on last line.
pub fn status_prompt_redraw(srv: &mut Server, c: ClientId) -> bool {
    let (tty_sx, tty_sy) = styles::client_size(srv, c);
    if tty_sx == 0 || tty_sy == 0 {
        return false;
    }
    let mut lines = status_line_size(srv, c);
    if lines <= 1 {
        lines = 1;
    }
    let mut promptline = status_prompt_line_at(srv, c);
    if promptline > lines - 1 {
        promptline = lines - 1;
    }
    let (ax, aw) = status_message_area(srv, c);
    let Some((prompt, generation)) = srv.clients.get_mut(c).and_then(|cl| cl.prompt.take_value())
    else {
        return false;
    };
    // Resolve formats and styles before the status screen is borrowed.
    let plan = prompt_draw(&prompt, srv, ax, aw);
    let Some(client) = srv.clients.get_mut(c) else {
        return false;
    };
    let sl = &mut client.status;
    let old = status_begin_overlay(sl, tty_sx, lines);
    let mut cursor_x = 0;
    {
        let StatusLine {
            registry,
            screen,
            active,
            ..
        } = sl;
        let active = active.as_mut().expect("prompt owns a pushed status screen");
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            active,
            &mut sink,
            ScreenWritePolicy::default(),
            registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.fast_copy(screen, 0, 0, tty_sx, lines);
        let mut pdd = PromptDrawData {
            cursor_x: &mut cursor_x,
            area_x: ax,
            area_width: aw,
            prompt_line: promptline,
        };
        plan.render(&prompt, &mut ctx, &mut pdd);
        ctx.finish();
    }
    sl.prompt_cx = cursor_x;
    let changed = status_finish_overlay(sl, old);
    status_slot_restore(srv, c, prompt, generation);
    changed
}

pub(crate) fn status_prompt_native(srv: &mut Server, c: ClientId) -> Option<serde_json::Value> {
    let (ax, aw) = status_message_area(srv, c);
    let line = status_prompt_line_at(srv, c);
    let (prompt, generation) = srv.clients.get_mut(c)?.prompt.take_value()?;
    let layout = prompt_draw(&prompt, srv, ax, aw).layout();
    let input = prompt.input();
    let text = String::from_utf8_lossy(&input);
    let cursor: usize = text.chars().take(prompt.index()).map(char::len_utf16).sum();
    let label = srv.clients.get(c).map(|client| {
        let grid = &client.status.active().grid;
        let cells = grid.view_string_cells(
            layout.content_x,
            line.min(grid.sy().saturating_sub(1)),
            layout.label_width,
        );
        String::from_utf8_lossy(&cells).into_owned()
    });
    let mut props = serde_json::json!({
        "text": text,
        "cursor": cursor,
        "anchor": null,
        "prompt": label.unwrap_or_default(),
        "ghost": null,
        "mode": null,
        "readonly": false,
    });
    if let Some(ghost) = prompt
        .complete_display()
        .filter(|_| cursor == text.encode_utf16().count())
    {
        props["ghost"] = String::from_utf8_lossy(ghost).into_owned().into();
    }
    props["readonly"] = crate::ui::prompt::prompt_native_readonly(&prompt).into();
    if prompt.flags().contains(PromptFlags::COMMANDMODE) {
        props["mode"] = "COMMAND".into();
    }
    status_slot_restore(srv, c, prompt, generation);
    Some(props)
}

pub(crate) fn status_prompt_native_event(
    srv: &mut Server,
    c: ClientId,
    name: &str,
    event: &serde_json::Value,
) {
    let Some((mut prompt, generation)) =
        srv.clients.get_mut(c).and_then(|cl| cl.prompt.take_value())
    else {
        return;
    };
    let mut redraw = false;
    if srv
        .clients
        .get(c)
        .is_some_and(|cl| cl.message.text.is_none())
    {
        match name {
            "edit" => crate::ui::prompt::prompt_native_edit(srv, &mut prompt, event),
            "undo" => crate::ui::prompt::prompt_native_undo(srv, &mut prompt),
            "send" => {
                if let Some(text) = event.get("text").and_then(serde_json::Value::as_str) {
                    crate::ui::prompt::prompt_native_send(srv, &mut prompt, text, &mut redraw);
                }
            }
            _ => {}
        }
    }
    status_slot_restore(srv, c, prompt, generation);
    if let Some(client) = srv.clients.get_mut(c) {
        client.flags.insert(ClientFlags::REDRAWSTATUS);
        if let Some(projection) = client.tsp.projection.as_mut() {
            projection.invalidate_bar();
        }
    }
    if srv
        .clients
        .get(c)
        .and_then(|cl| cl.prompt.as_ref())
        .is_some_and(prompt_closed)
    {
        status_prompt_clear(srv, c);
    }
}

/// Work out the tty cursor position for the prompt.
pub fn status_prompt_cursor(srv: &Server, c: ClientId) -> (u32, u32) {
    let cy = status_prompt_screen_line(srv, c);
    let cx = srv.clients.get(c).map_or(0, |cl| cl.status.prompt_cx);
    (cx, cy)
}

/// Handle keys in prompt.
pub fn status_prompt_key(
    srv: &mut Server,
    c: ClientId,
    key: KeyCode,
    m: Option<&MouseEvent>,
) -> PromptKeyResult {
    let mut redraw = false;
    let result;
    if key.is_mouse() {
        let Some(m) = m else {
            return PromptKeyResult::NotHandled;
        };
        let b = MouseButtonBits(m.b);
        if b.buttons() != rmux_util::key::MouseButton::Button1 as u32
            || b.is_drag()
            || b.is_release()
            || m.y != status_prompt_screen_line(srv, c)
        {
            return PromptKeyResult::NotHandled;
        }
        let (ax, aw) = status_message_area(srv, c);
        let Some((mut prompt, generation)) =
            srv.clients.get_mut(c).and_then(|cl| cl.prompt.take_value())
        else {
            return PromptKeyResult::NotHandled;
        };
        result = prompt_mouse(&mut prompt, srv, m.x, ax, aw, &mut redraw);
        status_slot_restore(srv, c, prompt, generation);
    } else {
        let Some((mut prompt, generation)) =
            srv.clients.get_mut(c).and_then(|cl| cl.prompt.take_value())
        else {
            return PromptKeyResult::NotHandled;
        };
        result = prompt_key(srv, &mut prompt, key, &mut redraw);
        status_slot_restore(srv, c, prompt, generation);
    }
    let Some(client) = srv.clients.get_mut(c) else {
        return result;
    };
    if redraw && client.prompt.is_some() {
        client.flags.insert(ClientFlags::REDRAWSTATUS);
    }
    if client.prompt.as_ref().is_some_and(prompt_closed) {
        status_prompt_clear(srv, c);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_escape_doubles_hash() {
        assert_eq!(status_message_escape(b"a#b##c").as_bytes(), b"a##b####c");
        assert_eq!(status_message_escape(b"").as_bytes(), b"");
    }

    #[test]
    fn status_line_default_has_base_screen() {
        let sl = StatusLine::default();
        assert!(sl.active.is_none());
        assert_eq!(sl.active().grid.sx(), 1);
        assert_eq!(sl.references, 0);
    }
}
