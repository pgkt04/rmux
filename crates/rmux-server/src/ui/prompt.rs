// Ported from tmux tmux.h, prompt.c, window.c (window_pane_set_prompt) @ 8f25579c
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
pub enum PromptType {
    Command = 0,
    Search = 1,
    Invalid = 255,
}
impl TryFrom<i32> for PromptType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Command),
            1 => Ok(Self::Search),
            255 => Ok(Self::Invalid),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PromptResult {
    Close = 1,
    Continue = 0,
}
impl TryFrom<i32> for PromptResult {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            1 => Ok(Self::Close),
            0 => Ok(Self::Continue),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PromptKeyResult {
    Move = 3,
    NotHandled = 0,
    Handled = 1,
    Close = 2,
}
impl TryFrom<i32> for PromptKeyResult {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            3 => Ok(Self::Move),
            0 => Ok(Self::NotHandled),
            1 => Ok(Self::Handled),
            2 => Ok(Self::Close),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PromptFlags(pub u32);
impl PromptFlags {
    pub const SINGLE: Self = Self(1);
    pub const NUMERIC: Self = Self(2);
    pub const INCREMENTAL: Self = Self(4);
    pub const NOFORMAT: Self = Self(8);
    pub const KEY: Self = Self(16);
    pub const ACCEPT: Self = Self(32);
    pub const QUOTENEXT: Self = Self(64);
    pub const BSPACE_EXIT: Self = Self(128);
    pub const NOFREEZE: Self = Self(256);
    pub const COMMANDMODE: Self = Self(512);
    pub const ISPANE: Self = Self(1024);
    pub const ISMODE: Self = Self(2048);
    pub const EDITARROWS: Self = Self(4096);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for PromptFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for PromptFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for PromptFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

pub mod history;

use crate::cmd::find::{CmdFindFlags, CmdFindState};
use crate::format::FormatTree;
use crate::ids::{ClientId, OptionsId, PaneId};
use crate::model::Server;
use crate::model::pane::{PanePrompt, PanePromptEngine};
use crate::ui::styles;
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::screen::write::ScreenWriteCtx;
use rmux_emu::screen::{ScreenCursorStyle, ScreenMode};
use rmux_emu::style::{Style, StyleAlign};
use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, KeyFlags, KeyMasks, KeyModifiers, ModeKeys, SpecialKey};
use rmux_util::log_debug;
use rmux_util::utf8::{self, Utf8Data, Utf8String};

pub const PROMPT_NTYPES: usize = 2;

/// Owned prompt continuation (prompt_input_cb + prompt_free_cb).
pub trait PromptHost {
    fn fire(&mut self, srv: &mut Server, text: Option<&[u8]>, key: PromptKeyResult)
    -> PromptResult;
    fn take_update(&mut self) -> Option<(ByteString, ByteString)> {
        None
    }
    /// prompt_free_cb: called exactly once before the host drops.
    fn free(&mut self, _srv: &mut Server) {}
}

/// Prompt create data: option snapshots plus owned inputs.
#[derive(Clone)]
pub struct PromptCreateData {
    pub fs: Option<CmdFindState>,
    pub prompt: ByteString,
    pub input: Option<ByteString>,
    pub ty: PromptType,
    pub flags: PromptFlags,
    pub style: GridCell,
    pub command_style: GridCell,
    pub style_str: ByteString,
    pub command_style_str: ByteString,
    pub cstyle: ScreenCursorStyle,
    pub command_cstyle: ScreenCursorStyle,
    pub ccolour: Colour,
    pub command_ccolour: Colour,
    pub cmode: ScreenMode,
    pub command_cmode: ScreenMode,
    pub message_format: ByteString,
    pub keys: ModeKeys,
    pub word_separators: ByteString,
}

impl Default for PromptCreateData {
    fn default() -> Self {
        Self {
            fs: None,
            prompt: ByteString::new(),
            input: None,
            ty: PromptType::Command,
            flags: PromptFlags::default(),
            style: DEFAULT_CELL,
            command_style: DEFAULT_CELL,
            style_str: ByteString::new(),
            command_style_str: ByteString::new(),
            cstyle: ScreenCursorStyle::Default,
            command_cstyle: ScreenCursorStyle::Default,
            ccolour: Colour::DEFAULT,
            command_ccolour: Colour::DEFAULT,
            cmode: ScreenMode(0),
            command_cmode: ScreenMode(0),
            message_format: ByteString::new(),
            keys: ModeKeys::Emacs,
            word_separators: ByteString::new(),
        }
    }
}

/// Prompt draw data.
pub struct PromptDrawData<'a> {
    pub cursor_x: &'a mut u32,
    pub area_x: u32,
    pub area_width: u32,
    pub prompt_line: u32,
}

/// The prompt engine state.
pub struct Prompt {
    host: Option<Box<dyn PromptHost>>,
    string: ByteString,
    buffer: Utf8String,
    state: CmdFindState,
    last: Option<ByteString>,
    index: usize,

    message_format: ByteString,
    keys: ModeKeys,
    word_separators: ByteString,
    style: GridCell,
    command_style: GridCell,
    style_str: ByteString,
    command_style_str: ByteString,
    cstyle: ScreenCursorStyle,
    command_cstyle: ScreenCursorStyle,
    ccolour: Colour,
    command_ccolour: Colour,
    cmode: ScreenMode,
    command_cmode: ScreenMode,

    ty: PromptType,
    flags: PromptFlags,
    closed: bool,

    hindex: [u32; PROMPT_NTYPES],
    copied: Option<Utf8String>,
    native_undo: std::collections::VecDeque<(Utf8String, usize)>,

    complete_list: Vec<ByteString>,
    complete_display: Option<ByteString>,
    complete_display_ud: Option<Utf8String>,
}

impl Prompt {
    pub fn flags(&self) -> PromptFlags {
        self.flags
    }
    pub fn prompt_type(&self) -> PromptType {
        self.ty
    }
    pub fn input(&self) -> ByteString {
        self.buffer.to_bytes()
    }
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn complete_display(&self) -> Option<&[u8]> {
        self.complete_display.as_ref().map(|v| v.as_bytes())
    }
}

/// Generation-aware prompt slot. The value is taken out while the engine
/// runs so callbacks may clear or replace the prompt; a stale value is
/// freed rather than restored.
#[derive(Default)]
pub struct PromptSlot {
    generation: u64,
    present: bool,
    value: Option<Prompt>,
}

impl PromptSlot {
    /// `c->prompt != NULL`.
    pub fn is_some(&self) -> bool {
        self.present
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn as_ref(&self) -> Option<&Prompt> {
        self.value.as_ref()
    }
    /// Mark a new prompt present (its value arrives with `restore`).
    pub fn install_pending(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.present = true;
        self.value = None;
        self.generation
    }
    /// Take the value out for dispatch.
    pub fn take_value(&mut self) -> Option<(Prompt, u64)> {
        let p = self.value.take()?;
        Some((p, self.generation))
    }
    /// Put a value back; returns it when the slot moved on.
    pub fn restore(&mut self, p: Prompt, generation: u64) -> Option<Prompt> {
        if self.present && self.generation == generation && self.value.is_none() {
            self.value = Some(p);
            None
        } else {
            Some(p)
        }
    }
    /// Clear logical presence; returns the value if it was not in flight.
    pub fn clear(&mut self) -> Option<Prompt> {
        self.generation = self.generation.wrapping_add(1);
        self.present = false;
        self.value.take()
    }
}

/// Restore a status prompt value, freeing it when the slot has moved on.
pub fn status_slot_restore(srv: &mut Server, c: ClientId, prompt: Prompt, generation: u64) {
    let rejected = match srv.clients.get_mut(c) {
        Some(client) => client.prompt.restore(prompt, generation),
        None => Some(prompt),
    };
    if let Some(p) = rejected {
        prompt_free(srv, p);
    }
}

fn prompt_flags_to_string(flags: PromptFlags) -> String {
    let names = [
        (PromptFlags::SINGLE, "SINGLE"),
        (PromptFlags::NUMERIC, "NUMERIC"),
        (PromptFlags::INCREMENTAL, "INCREMENTAL"),
        (PromptFlags::NOFORMAT, "NOFORMAT"),
        (PromptFlags::KEY, "KEY"),
        (PromptFlags::ACCEPT, "ACCEPT"),
        (PromptFlags::QUOTENEXT, "QUOTENEXT"),
        (PromptFlags::BSPACE_EXIT, "BSPACE_EXIT"),
        (PromptFlags::NOFREEZE, "NOFREEZE"),
        (PromptFlags::COMMANDMODE, "COMMANDMODE"),
        (PromptFlags::ISPANE, "ISPANE"),
        (PromptFlags::ISMODE, "ISMODE"),
        (PromptFlags::EDITARROWS, "EDITARROWS"),
    ];
    names
        .iter()
        .filter(|(f, _)| flags.contains(*f))
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join(",")
}

/// Set prompt options from session (or global session) options.
pub fn prompt_set_options(srv: &mut Server, pd: &mut PromptCreateData, oo: OptionsId) {
    styles::style_apply(srv, &mut pd.style, oo, b"message-style", None);
    styles::style_apply(
        srv,
        &mut pd.command_style,
        oo,
        b"message-command-style",
        None,
    );
    pd.style_str = srv.options.get_string(oo, b"message-style").into();
    pd.command_style_str = srv.options.get_string(oo, b"message-command-style").into();
    let n = srv.options.get_number(oo, b"prompt-cursor-style") as u32;
    rmux_emu::screen::set_cursor_style(n, &mut pd.cstyle, &mut pd.cmode);
    let n = srv.options.get_number(oo, b"prompt-command-cursor-style") as u32;
    rmux_emu::screen::set_cursor_style(n, &mut pd.command_cstyle, &mut pd.command_cmode);
    let mut gc = DEFAULT_CELL;
    styles::style_apply(srv, &mut gc, oo, b"prompt-cursor-colour", None);
    pd.ccolour = gc.fg;
    styles::style_apply(srv, &mut gc, oo, b"prompt-command-cursor-colour", None);
    pd.command_ccolour = gc.fg;
    pd.message_format = srv.options.get_string(oo, b"message-format").into();
    pd.keys = ModeKeys::try_from(srv.options.get_number(oo, b"status-keys") as i32)
        .unwrap_or(ModeKeys::Emacs);
    pd.word_separators = srv.options.get_string(oo, b"word-separators").into();
}

fn state_tree(srv: &mut Server, state: &CmdFindState) -> FormatTree {
    if state.is_valid(srv) {
        crate::format::create_from_state(srv, None, None, state)
    } else {
        styles::create_defaults(srv, None, None, None, None, None)
    }
}

/// Create prompt.
pub fn prompt_create(srv: &mut Server, pd: PromptCreateData, host: Box<dyn PromptHost>) -> Prompt {
    let (mut ft, state) = match pd.fs {
        Some(fs) => {
            let mut state = CmdFindState::clear(CmdFindFlags(0));
            state.copy_target_from(&fs);
            (
                crate::format::create_from_state(srv, None, None, &fs),
                state,
            )
        }
        None => (
            styles::create_defaults(srv, None, None, None, None, None),
            CmdFindState::clear(CmdFindFlags(0)),
        ),
    };
    let input = pd.input.unwrap_or_default();
    let tmp = if pd.flags.contains(PromptFlags::NOFORMAT) {
        input
    } else {
        ft.expand_time(srv, &input)
    };
    let (last, buffer) = if pd.flags.contains(PromptFlags::INCREMENTAL) {
        (Some(tmp), utf8::from_cstr(b""))
    } else {
        (None, utf8::from_cstr(&tmp))
    };
    ft.release(srv);
    let index = buffer.len();
    Prompt {
        host: Some(host),
        string: pd.prompt,
        buffer,
        state,
        last,
        index,
        message_format: pd.message_format,
        keys: pd.keys,
        word_separators: pd.word_separators,
        style: pd.style,
        command_style: pd.command_style,
        style_str: pd.style_str,
        command_style_str: pd.command_style_str,
        cstyle: pd.cstyle,
        command_cstyle: pd.command_cstyle,
        ccolour: pd.ccolour,
        command_ccolour: pd.command_ccolour,
        cmode: pd.cmode,
        command_cmode: pd.command_cmode,
        ty: pd.ty,
        flags: pd.flags,
        closed: false,
        hindex: [0; PROMPT_NTYPES],
        copied: None,
        native_undo: std::collections::VecDeque::new(),
        complete_list: Vec::new(),
        complete_display: None,
        complete_display_ud: None,
    }
}

/// Free prompt: runs the host free callback once.
pub fn prompt_free(srv: &mut Server, mut pr: Prompt) {
    if let Some(mut host) = pr.host.take() {
        host.free(srv);
    }
}

/// Fire the input callback. Returns true if the prompt is finished.
fn prompt_fire_callback(
    srv: &mut Server,
    pr: &mut Prompt,
    s: Option<&[u8]>,
    ty: PromptKeyResult,
    redraw: Option<&mut bool>,
) -> bool {
    let result = match pr.host.as_mut() {
        Some(host) => host.fire(srv, s, ty),
        None => PromptResult::Close,
    };
    if result == PromptResult::Close {
        pr.closed = true;
        return true;
    }
    if let Some((message, input)) = pr.host.as_mut().and_then(|host| host.take_update()) {
        prompt_update(pr, srv, &message, Some(&input));
    }
    if let Some(r) = redraw {
        *r = true;
    }
    false
}

fn prefixed(prefix: u8, s: &[u8]) -> ByteString {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.push(prefix);
    v.extend_from_slice(s);
    v.into()
}

/// Start incremental prompt.
pub fn prompt_incremental_start(srv: &mut Server, pr: &mut Prompt) {
    if pr.flags.contains(PromptFlags::INCREMENTAL) {
        let cp = prefixed(b'=', &pr.buffer.to_bytes());
        prompt_fire_callback(srv, pr, Some(&cp), PromptKeyResult::Handled, None);
    }
}

/// Update prompt.
pub fn prompt_update(pr: &mut Prompt, srv: &mut Server, msg: &[u8], input: Option<&[u8]>) {
    let mut ft = state_tree(srv, &pr.state);
    pr.string = msg.into();
    let input = input.unwrap_or(b"");
    let tmp = if pr.flags.contains(PromptFlags::NOFORMAT) {
        input.into()
    } else {
        ft.expand_time(srv, input)
    };
    pr.buffer = utf8::from_cstr(&tmp);
    pr.index = pr.buffer.len();
    pr.hindex = [0; PROMPT_NTYPES];
    pr.closed = false;
    pr.native_undo.clear();
    prompt_clear_complete(pr);
    ft.release(srv);
}

/// Is this prompt closed?
pub fn prompt_closed(pr: &Prompt) -> bool {
    pr.closed
}

/// Redraw character. Return true if drawing can continue.
fn prompt_redraw_character(
    ctx: &mut ScreenWriteCtx<'_>,
    offset: u32,
    pwidth: u32,
    width: &mut u32,
    gc: &mut GridCell,
    ud: &Utf8Data,
) -> bool {
    if *width < offset {
        *width += u32::from(ud.width);
        return true;
    }
    if *width >= offset + pwidth {
        return false;
    }
    *width += u32::from(ud.width);
    if *width > offset + pwidth {
        return false;
    }
    let ch = ud.data[0];
    if ud.size == 1 && (ch <= 0x1f || ch == 0x7f) {
        gc.data.data[0] = b'^';
        gc.data.data[1] = if ch == 0x7f { b'?' } else { ch | 0x40 };
        gc.data.size = 2;
        gc.data.have = 2;
        gc.data.width = 2;
    } else {
        gc.data.copy_from(ud);
    }
    ctx.cell(gc);
    true
}

/// Redraw quote indicator '^' if necessary.
#[allow(clippy::too_many_arguments)]
fn prompt_redraw_quote(
    pr: &Prompt,
    pcursor: u32,
    input_x: u32,
    ctx: &mut ScreenWriteCtx<'_>,
    offset: u32,
    pw: u32,
    w: &mut u32,
    gc: &mut GridCell,
) -> bool {
    if pr.flags.contains(PromptFlags::QUOTENEXT)
        && pcursor >= offset
        && ctx.screen.cx == input_x + pcursor - offset
    {
        let ud = Utf8Data::set(b'^');
        return prompt_redraw_character(ctx, offset, pw, w, gc, &ud);
    }
    true
}

/// Draw the stored completion matches.
fn prompt_draw_complete(
    pr: &Prompt,
    ctx: &mut ScreenWriteCtx<'_>,
    ax: u32,
    aw: u32,
    cx: u32,
    py: u32,
    base: &GridCell,
) {
    let Some(ud) = pr.complete_display_ud.as_ref() else {
        return;
    };
    if pr.index != pr.buffer.len() {
        return;
    }
    if cx < ax || cx - ax >= aw {
        return;
    }
    let avail = aw - (cx - ax);
    let mut gc = *base;
    gc.attr.insert(GridAttributes::UNDERSCORE);
    ctx.cursormove(cx as i32, py as i32, false);
    let mut width = 0;
    for u in ud.iter() {
        if width + u32::from(u.width) > avail {
            break;
        }
        gc.data.copy_from(u);
        ctx.cell(&gc);
        width += u32::from(u.width);
    }
}

/// Create the prompt format tree using the current input.
fn prompt_format_tree(pr: &Prompt, srv: &mut Server) -> FormatTree {
    let mut ft = state_tree(srv, &pr.state);
    ft.add(b"prompt_input", pr.buffer.to_bytes());
    ft.add(b"prompt_flags", prompt_flags_to_string(pr.flags).into());
    ft.add(b"prompt_type", prompt_type_string(pr.ty).into());
    let cp: &[u8] = if pr.flags.contains(PromptFlags::COMMANDMODE) {
        b"1"
    } else {
        b"0"
    };
    ft.add(b"command_prompt", cp.into());
    ft
}

/// Expand prompt string using the current input.
fn prompt_expand1(pr: &Prompt, srv: &mut Server, ft: &mut FormatTree) -> ByteString {
    let prompt = ft.expand_time(srv, &pr.string);
    ft.add(b"message", prompt);
    ft.expand_time(srv, &pr.message_format)
}

/// Get the effective message style for the current prompt.
fn prompt_effective_style(pr: &Prompt, srv: &mut Server, ft: &mut FormatTree) -> Style {
    let (s, gc) = if pr.flags.contains(PromptFlags::COMMANDMODE) {
        (&pr.command_style_str, &pr.command_style)
    } else {
        (&pr.style_str, &pr.style)
    };
    let mut sy = Style::from_cell(*gc);
    let expanded = ft.expand_time(srv, s);
    if sy
        .parse(&DEFAULT_CELL, &expanded, &mut srv.hyperlinks)
        .is_err()
    {
        sy = Style::from_cell(*gc);
    }
    sy
}

/// The geometry result of prompt_layout.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PromptLayout {
    pub area_x: u32,
    pub area_width: u32,
    pub content_x: u32,
    pub content_width: u32,
    pub label_width: u32,
    pub input_x: u32,
    pub cursor_x: u32,
    pub input_offset: u32,
    pub input_width: u32,
}

/// Pure part of prompt_layout, after formats and style are resolved.
pub fn prompt_layout_geometry(
    pr: &Prompt,
    ax: u32,
    aw: u32,
    expanded: &[u8],
    align: Option<StyleAlign>,
) -> PromptLayout {
    let mut pl = PromptLayout {
        area_x: ax,
        area_width: aw,
        ..Default::default()
    };
    if aw == 0 {
        return pl;
    }
    pl.label_width = crate::format::draw::width(expanded);
    if pl.label_width > aw {
        pl.label_width = aw;
    }
    let pcursor = pr.buffer.width(Some(pr.index));
    let mut pwidth = pr.buffer.width(None);
    if pr.flags.contains(PromptFlags::QUOTENEXT) {
        pwidth += 1;
    }
    let avail = aw - pl.label_width;
    if avail == 0 {
        pl.input_offset = 0;
        pl.input_width = 0;
        pl.cursor_x = pl.label_width;
    } else {
        let (offset, mut width) = if pcursor >= avail {
            ((pcursor - avail) + 1, avail)
        } else {
            (0, pwidth)
        };
        if width > avail {
            width = avail;
        }
        pl.input_offset = offset;
        pl.input_width = width;
        pl.cursor_x = pl.label_width + pcursor - offset;
    }
    pl.content_width = pl.label_width + pl.input_width;
    if let Some(display) = &pr.complete_display {
        if pr.index == pr.buffer.len() && pl.cursor_x < aw {
            let avail = aw - pl.cursor_x;
            let mut width = utf8::cstr_width(display);
            if width > avail {
                width = avail;
            }
            let end = pl.cursor_x + width;
            if end > pl.content_width {
                pl.content_width = end;
            }
        }
    }
    if pl.content_width > aw {
        pl.content_width = aw;
    }
    pl.content_x = match align {
        Some(StyleAlign::Centre) | Some(StyleAlign::AbsoluteCentre) => {
            ax + (aw - pl.content_width) / 2
        }
        Some(StyleAlign::Right) => ax + aw - pl.content_width,
        _ => ax,
    };
    pl.input_x = pl.content_x + pl.label_width;
    pl.cursor_x += pl.content_x;
    pl
}

/// Work out where the editable prompt content appears.
fn prompt_layout(
    pr: &Prompt,
    srv: &mut Server,
    ax: u32,
    aw: u32,
) -> (PromptLayout, ByteString, Style) {
    let mut ft = prompt_format_tree(pr, srv);
    let sy = prompt_effective_style(pr, srv, &mut ft);
    let expanded = prompt_expand1(pr, srv, &mut ft);
    ft.release(srv);
    let pl = prompt_layout_geometry(pr, ax, aw, &expanded, Some(sy.align));
    (pl, expanded, sy)
}

/// Everything prompt_draw needs from the server, resolved up front so the
/// target screen can be written while the server is borrowed elsewhere.
pub struct PromptDrawPlan {
    layout: PromptLayout,
    expanded: ByteString,
    style: Style,
}

/// Resolve formats and styles for drawing a prompt in the given area.
pub fn prompt_draw(pr: &Prompt, srv: &mut Server, ax: u32, aw: u32) -> PromptDrawPlan {
    let (layout, expanded, style) = prompt_layout(pr, srv, ax, aw);
    PromptDrawPlan {
        layout,
        expanded,
        style,
    }
}

impl PromptDrawPlan {
    pub fn layout(&self) -> PromptLayout {
        self.layout
    }

    /// Draw the prompt (the second half of C prompt_draw).
    pub fn render(&self, pr: &Prompt, ctx: &mut ScreenWriteCtx<'_>, pd: &mut PromptDrawData<'_>) {
        {
            let s = &mut *ctx.screen;
            if pr.flags.contains(PromptFlags::COMMANDMODE) {
                s.default_cstyle = pr.command_cstyle;
                s.default_mode = pr.command_cmode;
                s.default_ccolour = pr.command_ccolour;
            } else {
                s.default_cstyle = pr.cstyle;
                s.default_mode = pr.cmode;
                s.default_ccolour = pr.ccolour;
            }
        }
        let ax = pd.area_x;
        let aw = pd.area_width;
        let py = pd.prompt_line;
        let pl = self.layout;
        let mut gc = self.style.gc;
        *pd.cursor_x = pl.cursor_x;

        ctx.cursormove(ax as i32, py as i32, false);
        if self.style.fill != Colour::DEFAULT {
            ctx.clearcharacter(aw, self.style.fill);
        }
        let pcursor = pr.buffer.width(Some(pr.index));
        if pl.content_width != 0 {
            ctx.cursormove(pl.content_x as i32, py as i32, false);
            if pl.label_width != 0 {
                crate::format::draw::draw(ctx, &gc, pl.label_width, &self.expanded, None, false);
            }
            ctx.cursormove(pl.input_x as i32, py as i32, false);
            let mut width = 0;
            for ud in pr.buffer.iter() {
                if !prompt_redraw_quote(
                    pr,
                    pcursor,
                    pl.input_x,
                    ctx,
                    pl.input_offset,
                    pl.input_width,
                    &mut width,
                    &mut gc,
                ) {
                    break;
                }
                if !prompt_redraw_character(
                    ctx,
                    pl.input_offset,
                    pl.input_width,
                    &mut width,
                    &mut gc,
                    ud,
                ) {
                    break;
                }
            }
            prompt_redraw_quote(
                pr,
                pcursor,
                pl.input_x,
                ctx,
                pl.input_offset,
                pl.input_width,
                &mut width,
                &mut gc,
            );
            prompt_draw_complete(
                pr,
                ctx,
                pl.content_x,
                pl.content_width,
                pl.cursor_x,
                py,
                &gc,
            );
        }
    }
}

/// Choose a completion from a mouse position.
fn prompt_mouse_complete(
    pr: &mut Prompt,
    srv: &Server,
    x: u32,
    cx: u32,
    ax: u32,
    aw: u32,
    redraw: &mut bool,
) -> PromptKeyResult {
    let Some(display) = pr.complete_display.clone() else {
        return PromptKeyResult::NotHandled;
    };
    if pr.complete_list.is_empty() || pr.index != pr.buffer.len() {
        return PromptKeyResult::NotHandled;
    }
    if cx < ax || cx - ax >= aw || x < cx {
        return PromptKeyResult::NotHandled;
    }
    let avail = aw - (cx - ax);
    let clicked = x - cx;
    let mut width = utf8::cstr_width(&display);
    if width > avail {
        width = avail;
    }
    if clicked >= width {
        return PromptKeyResult::NotHandled;
    }
    let mut end = 0;
    let list = pr.complete_list.clone();
    for item in &list {
        let start = end + 1;
        end = start + utf8::cstr_width(item);
        if clicked < start || clicked >= end {
            continue;
        }
        let mut replace = item.to_vec();
        replace.push(b' ');
        if prompt_replace_complete(pr, srv, Some(&replace)) {
            prompt_clear_complete(pr);
            *redraw = true;
        }
        return PromptKeyResult::Handled;
    }
    PromptKeyResult::Handled
}

/// Move cursor in prompt from a mouse position.
pub fn prompt_mouse(
    pr: &mut Prompt,
    srv: &mut Server,
    x: u32,
    ax: u32,
    aw: u32,
    redraw: &mut bool,
) -> PromptKeyResult {
    if x < ax || x >= ax + aw {
        return PromptKeyResult::NotHandled;
    }
    let (pl, _, _) = prompt_layout(pr, srv, ax, aw);
    if pl.input_width == 0 {
        return PromptKeyResult::Handled;
    }
    let mut pwidth = pr.buffer.width(None);
    if pr.flags.contains(PromptFlags::QUOTENEXT) {
        pwidth += 1;
    }
    let result = prompt_mouse_complete(
        pr,
        srv,
        x,
        pl.cursor_x,
        pl.content_x,
        pl.content_width,
        redraw,
    );
    if result != PromptKeyResult::NotHandled {
        return result;
    }
    let mut target = if x <= pl.input_x {
        pl.input_offset
    } else {
        pl.input_offset + x - pl.input_x
    };
    if target > pwidth {
        target = pwidth;
    }
    let mut width = 0;
    let mut idx = 0;
    while idx < pr.buffer.len() {
        if width >= target {
            break;
        }
        width += u32::from(pr.buffer[idx].width);
        idx += 1;
    }
    if idx == pr.index {
        return PromptKeyResult::Handled;
    }
    pr.index = idx;
    prompt_clear_complete(pr);
    *redraw = true;
    PromptKeyResult::Handled
}

/// Is this a separator?
fn prompt_in_list(ws: &[u8], ud: &Utf8Data) -> bool {
    if ud.size != 1 || ud.width != 1 {
        return false;
    }
    ws.contains(&ud.data[0])
}

/// Is this a space?
fn prompt_space(ud: &Utf8Data) -> bool {
    ud.size == 1 && ud.width == 1 && ud.data[0] == b' '
}

/// Is this a keypad key?
fn prompt_keypad_key(key: KeyCode) -> KeyCode {
    if key.0 & KeyMasks::MODIFIERS != 0 {
        return key;
    }
    let ch = match key.0 {
        SpecialKey::KP_SLASH => b'/',
        SpecialKey::KP_STAR => b'*',
        SpecialKey::KP_MINUS => b'-',
        SpecialKey::KP_SEVEN => b'7',
        SpecialKey::KP_EIGHT => b'8',
        SpecialKey::KP_NINE => b'9',
        SpecialKey::KP_PLUS => b'+',
        SpecialKey::KP_FOUR => b'4',
        SpecialKey::KP_FIVE => b'5',
        SpecialKey::KP_SIX => b'6',
        SpecialKey::KP_ONE => b'1',
        SpecialKey::KP_TWO => b'2',
        SpecialKey::KP_THREE => b'3',
        SpecialKey::KP_ENTER => b'\r',
        SpecialKey::KP_ZERO => b'0',
        SpecialKey::KP_PERIOD => b'.',
        _ => return key,
    };
    KeyCode(u64::from(ch))
}

const fn ctrl(c: u8) -> u64 {
    c as u64 | KeyModifiers::CTRL.0
}
const fn meta(c: u8) -> u64 {
    c as u64 | KeyModifiers::META.0
}
const fn vi(c: u8) -> u64 {
    c as u64 | KeyFlags::VI.0
}
const ESC: u64 = 0x1b;
const TAB: u64 = 0x09;
const CR: u64 = b'\r' as u64;
const LF: u64 = b'\n' as u64;

/// Translate key from vi to emacs. Return 0 to drop the key, 1 to process the
/// key as an emacs key; 2 to append to the buffer.
pub fn prompt_translate_key(
    pr: &mut Prompt,
    key: KeyCode,
    new_key: &mut KeyCode,
    redraw: &mut bool,
) -> u8 {
    let k = key.0;
    if !pr.flags.contains(PromptFlags::COMMANDMODE) {
        let passthrough = [
            ctrl(b'a'),
            ctrl(b'c'),
            ctrl(b'e'),
            ctrl(b'g'),
            ctrl(b'h'),
            TAB,
            ctrl(b'k'),
            ctrl(b'n'),
            ctrl(b'p'),
            ctrl(b't'),
            ctrl(b'u'),
            ctrl(b'v'),
            ctrl(b'w'),
            ctrl(b'y'),
            LF,
            CR,
            SpecialKey::LEFT | KeyModifiers::CTRL.0,
            SpecialKey::RIGHT | KeyModifiers::CTRL.0,
            SpecialKey::BSPACE,
            SpecialKey::DC,
            SpecialKey::DOWN,
            SpecialKey::END,
            SpecialKey::HOME,
            SpecialKey::LEFT,
            SpecialKey::RIGHT,
            SpecialKey::UP,
        ];
        if passthrough.contains(&k) {
            *new_key = key;
            return 1;
        }
        if k == ESC || k == ctrl(b'[') {
            pr.flags.insert(PromptFlags::COMMANDMODE);
            if pr.index != 0 {
                pr.index -= 1;
            }
            *redraw = true;
            return 0;
        }
        *new_key = key;
        return 2;
    }

    if k == SpecialKey::BSPACE {
        *new_key = KeyCode(SpecialKey::LEFT);
        return 1;
    }
    match k {
        0x41 | 0x49 | 0x43 | 0x73 | 0x61 => {
            // A I C s a: switch mode and...
            pr.flags.remove(PromptFlags::COMMANDMODE);
            *redraw = true;
        }
        0x53 => {
            // S
            pr.flags.remove(PromptFlags::COMMANDMODE);
            *redraw = true;
            *new_key = KeyCode(ctrl(b'u'));
            return 1;
        }
        0x69 => {
            // i
            pr.flags.remove(PromptFlags::COMMANDMODE);
            *redraw = true;
            return 0;
        }
        _ if k == ESC || k == ctrl(b'[') => return 0,
        _ => {}
    }

    let mapped = match k {
        0x41 | 0x24 => SpecialKey::END,
        0x49 | 0x30 | 0x5e => SpecialKey::HOME,
        0x43 | 0x44 => ctrl(b'k'),
        0x58 => SpecialKey::BSPACE,
        0x62 => meta(b'b'),
        0x42 => vi(b'B'),
        0x64 => ctrl(b'u'),
        0x65 => vi(b'e'),
        0x45 => vi(b'E'),
        0x77 => vi(b'w'),
        0x57 => vi(b'W'),
        0x70 => ctrl(b'y'),
        0x71 => ctrl(b'c'),
        0x73 | 0x78 => SpecialKey::DC,
        _ if k == SpecialKey::DC => SpecialKey::DC,
        0x6a => SpecialKey::DOWN,
        _ if k == SpecialKey::DOWN => SpecialKey::DOWN,
        0x68 => SpecialKey::LEFT,
        _ if k == SpecialKey::LEFT => SpecialKey::LEFT,
        0x61 | 0x6c => SpecialKey::RIGHT,
        _ if k == SpecialKey::RIGHT => SpecialKey::RIGHT,
        0x6b => SpecialKey::UP,
        _ if k == SpecialKey::UP => SpecialKey::UP,
        _ if k == ctrl(b'h') || k == ctrl(b'c') || k == LF || k == CR => return 1,
        _ => return 0,
    };
    *new_key = KeyCode(mapped);
    1
}

/// Paste into prompt.
fn prompt_paste(pr: &mut Prompt, srv: &Server) -> bool {
    let size = pr.buffer.len();
    let ud: Vec<Utf8Data> = match &pr.copied {
        Some(copied) => copied.0.clone(),
        None => {
            let Some(pb) = crate::model::paste::paste_get_top(srv) else {
                return false;
            };
            let bufdata = crate::model::paste::paste_buffer_data(srv, pb).unwrap_or(b"");
            let bufsize = bufdata.len();
            let mut out = Vec::with_capacity(bufsize);
            let mut i = 0;
            while i != bufsize {
                if let Ok(mut udp) = Utf8Data::open(bufdata[i]) {
                    let mut more = utf8::Utf8State::More;
                    i += 1;
                    while i != bufsize && more == utf8::Utf8State::More {
                        more = udp.append(bufdata[i]);
                        i += 1;
                    }
                    if more == utf8::Utf8State::Done {
                        out.push(udp);
                        continue;
                    }
                    i -= udp.have as usize;
                }
                if bufdata[i] <= 31 || bufdata[i] >= 127 {
                    break;
                }
                out.push(Utf8Data::set(bufdata[i]));
                i += 1;
            }
            out
        }
    };
    let n = ud.len();
    if n != 0 {
        let idx = pr.index.min(size);
        pr.native_undo.clear();
        let tail = pr.buffer.0.split_off(idx);
        pr.buffer.0.extend(ud);
        pr.buffer.0.extend(tail);
        pr.index = idx + n;
    }
    true
}

/// Finish completion.
fn prompt_replace_complete(pr: &mut Prompt, srv: &Server, s: Option<&[u8]>) -> bool {
    let idx = pr.index.saturating_sub(1);
    let size = pr.buffer.len();
    let (first, last) = {
        let buf = &pr.buffer.0;
        let is_space = |i: usize| buf.get(i).is_some_and(prompt_space);
        let mut first = idx.min(size);
        while first > 0 && !is_space(first) {
            first -= 1;
        }
        while first < size && is_space(first) {
            first += 1;
        }
        let mut last = idx.min(size);
        while last < size && !is_space(last) {
            last += 1;
        }
        while last > 0 && is_space(last) {
            last -= 1;
        }
        if last < size {
            last += 1;
        }
        (first, last)
    };
    if last < first {
        return false;
    }
    let allocated;
    let s: &[u8] = match s {
        Some(s) => s,
        None => {
            let mut word = Vec::new();
            for ud in &pr.buffer.0[first..last] {
                if word.len() + ud.size as usize >= 64 {
                    return false;
                }
                word.extend_from_slice(ud.bytes());
            }
            match prompt_complete(pr, srv, &word, first as u32) {
                Some(out) => {
                    allocated = out;
                    &allocated
                }
                None => return false,
            }
        }
    };

    // Trim out the word and insert the new one, one byte per cell as C does.
    pr.native_undo.clear();
    let tail = pr.buffer.0.split_off(last);
    pr.buffer.0.truncate(first);
    pr.buffer.0.extend(s.iter().map(|&b| Utf8Data::set(b)));
    pr.buffer.0.extend(tail);
    pr.index = first + s.len();
    true
}

/// Prompt forward to the next beginning of a word.
pub fn prompt_forward_word(pr: &mut Prompt, size: usize, vi: bool, separators: &[u8]) {
    let buf = &pr.buffer.0;
    let sp = |i: usize| buf.get(i).is_some_and(prompt_space);
    let in_list = |i: usize| buf.get(i).is_some_and(|u| prompt_in_list(separators, u));
    let mut idx = pr.index;
    if !vi {
        while idx != size && sp(idx) {
            idx += 1;
        }
    }
    if idx == size {
        pr.index = idx;
        return;
    }
    let word_is_separators = in_list(idx) && !sp(idx);
    loop {
        idx += 1;
        if sp(idx) {
            if vi {
                while idx != size && sp(idx) {
                    idx += 1;
                }
            }
            break;
        }
        if !(idx != size && word_is_separators == in_list(idx)) {
            break;
        }
    }
    pr.index = idx;
}

/// Prompt forward to the next end of a word.
pub fn prompt_end_word(pr: &mut Prompt, size: usize, separators: &[u8]) {
    let buf = &pr.buffer.0;
    let sp = |i: usize| buf.get(i).is_some_and(prompt_space);
    let in_list = |i: usize| buf.get(i).is_some_and(|u| prompt_in_list(separators, u));
    let mut idx = pr.index;
    if idx == size {
        return;
    }
    loop {
        idx += 1;
        if idx == size {
            pr.index = idx;
            return;
        }
        if !sp(idx) {
            break;
        }
    }
    let word_is_separators = in_list(idx);
    loop {
        idx += 1;
        if idx == size {
            break;
        }
        if sp(idx) || word_is_separators != in_list(idx) {
            break;
        }
    }
    pr.index = idx - 1;
}

/// Prompt backward to the previous beginning of a word.
pub fn prompt_backward_word(pr: &mut Prompt, separators: &[u8]) {
    let buf = &pr.buffer.0;
    let sp = |i: usize| buf.get(i).is_some_and(prompt_space);
    let in_list = |i: usize| buf.get(i).is_some_and(|u| prompt_in_list(separators, u));
    let mut idx = pr.index;
    while idx != 0 {
        idx -= 1;
        if !sp(idx) {
            break;
        }
    }
    let word_is_separators = in_list(idx);
    while idx != 0 {
        idx -= 1;
        if sp(idx) || word_is_separators != in_list(idx) {
            idx += 1;
            break;
        }
    }
    pr.index = idx;
}

/// Fire input callback when done.
fn prompt_done(
    srv: &mut Server,
    pr: &mut Prompt,
    s: Option<&[u8]>,
    redraw: &mut bool,
) -> PromptKeyResult {
    if prompt_fire_callback(srv, pr, s, PromptKeyResult::Close, Some(redraw)) {
        return PromptKeyResult::Close;
    }
    PromptKeyResult::Handled
}

/// Check for a movement key.
fn prompt_check_move(srv: &mut Server, pr: &mut Prompt, key: KeyCode) -> PromptKeyResult {
    if !pr.flags.contains(PromptFlags::INCREMENTAL) {
        return PromptKeyResult::NotHandled;
    }
    let k = key.0;
    if k == SpecialKey::UP
        || k == SpecialKey::DOWN
        || k == SpecialKey::PPAGE
        || k == SpecialKey::NPAGE
    {
    } else if k == SpecialKey::LEFT || k == SpecialKey::RIGHT {
        if pr.flags.contains(PromptFlags::EDITARROWS) {
            return PromptKeyResult::NotHandled;
        }
    } else {
        return PromptKeyResult::NotHandled;
    }
    let s = pr.buffer.to_bytes();
    if prompt_fire_callback(srv, pr, Some(&s), PromptKeyResult::Move, None) {
        return PromptKeyResult::Close;
    }
    PromptKeyResult::Move
}

fn set_buffer(pr: &mut Prompt, s: &[u8]) {
    pr.native_undo.clear();
    pr.buffer = utf8::from_cstr(s);
    pr.index = pr.buffer.len();
}

pub(crate) fn prompt_native_edit(srv: &mut Server, pr: &mut Prompt, event: &serde_json::Value) {
    if prompt_native_readonly(pr) {
        return;
    }
    let offset = |field: &str| usize::try_from(event.get(field)?.as_u64()?).ok();
    let (Some(from), Some(to), Some(cursor), Some(len)) = (
        offset("from"),
        offset("to"),
        offset("cursor"),
        offset("len"),
    ) else {
        return;
    };
    let Some(insert) = event.get("text").and_then(serde_json::Value::as_str) else {
        return;
    };
    let input = pr.buffer.to_bytes();
    let Ok(text) = std::str::from_utf8(&input) else {
        return;
    };
    if text.encode_utf16().count() != len || from > to || to > len {
        return;
    }
    let Some(start) = prompt_utf16_index(text, from) else {
        return;
    };
    let Some(end) = prompt_utf16_index(text, to) else {
        return;
    };
    let Some(raw_len) = (len - (to - from)).checked_add(insert.encode_utf16().count()) else {
        return;
    };
    if cursor > raw_len {
        return;
    }
    let mut offset = 0;
    let mut index = 0;
    for (ch, keep) in text
        .chars()
        .take(start)
        .map(|ch| (ch, true))
        .chain(insert.chars().map(|ch| (ch, prompt_native_safe_char(ch))))
        .chain(text.chars().skip(end).map(|ch| (ch, true)))
    {
        if offset == cursor {
            break;
        }
        offset += ch.len_utf16();
        index += usize::from(keep);
        if offset > cursor {
            return;
        }
    }
    if offset != cursor {
        return;
    }
    let insert: String = insert
        .chars()
        .filter(|&ch| prompt_native_safe_char(ch))
        .collect();
    let replacement = utf8::from_cstr(insert.as_bytes());
    if pr.buffer.0[start..end] != replacement.0 {
        if pr.native_undo.len() == 100 {
            pr.native_undo.pop_front();
        }
        pr.native_undo.push_back((pr.buffer.clone(), pr.index));
    }
    prompt_clear_complete(pr);
    pr.buffer.0.splice(start..end, replacement.0);
    pr.index = index;
    if pr.flags.contains(PromptFlags::INCREMENTAL) {
        let cp = prefixed(b'=', &pr.buffer.to_bytes());
        prompt_fire_callback(srv, pr, Some(&cp), PromptKeyResult::Handled, None);
    }
}

pub(crate) fn prompt_native_readonly(pr: &Prompt) -> bool {
    pr.flags.intersects(
        PromptFlags::KEY
            | PromptFlags::SINGLE
            | PromptFlags::NUMERIC
            | PromptFlags::QUOTENEXT
            | PromptFlags::COMMANDMODE,
    ) || (pr.flags.contains(PromptFlags::BSPACE_EXIT) && pr.buffer.is_empty())
}

fn prompt_native_safe_char(ch: char) -> bool {
    ch > '\u{1f}' && ch != '\u{7f}'
}

pub(crate) fn prompt_native_undo(srv: &mut Server, pr: &mut Prompt) {
    if prompt_native_readonly(pr) {
        return;
    }
    let Some((buffer, index)) = pr.native_undo.pop_back() else {
        return;
    };
    prompt_clear_complete(pr);
    pr.buffer = buffer;
    pr.index = index;
    if pr.flags.contains(PromptFlags::INCREMENTAL) {
        let cp = prefixed(b'=', &pr.buffer.to_bytes());
        prompt_fire_callback(srv, pr, Some(&cp), PromptKeyResult::Handled, None);
    }
}

pub(crate) fn prompt_native_send(srv: &mut Server, pr: &mut Prompt, text: &str, redraw: &mut bool) {
    if prompt_native_readonly(pr) {
        return;
    }
    let text: String = text
        .chars()
        .filter(|&ch| prompt_native_safe_char(ch))
        .collect();
    set_buffer(pr, text.as_bytes());
    prompt_key(srv, pr, KeyCode(CR), redraw);
}

fn prompt_utf16_index(text: &str, target: usize) -> Option<usize> {
    let mut offset = 0;
    for (index, ch) in text.chars().enumerate() {
        if offset == target {
            return Some(index);
        }
        offset += ch.len_utf16();
        if offset > target {
            return None;
        }
    }
    (offset == target).then(|| text.chars().count())
}

enum KeyStep {
    Process,
    Append,
}

/// Handle keys in prompt.
pub fn prompt_key(
    srv: &mut Server,
    pr: &mut Prompt,
    key: KeyCode,
    redraw: &mut bool,
) -> PromptKeyResult {
    pr.closed = false;
    // Drop any inline completion matches; Tab rebuilds them if applicable.
    prompt_clear_complete(pr);

    if pr.flags.contains(PromptFlags::KEY) {
        let ks = rmux_tty::key_string::key_name(key, false);
        if !prompt_fire_callback(srv, pr, Some(&ks), PromptKeyResult::Close, None) {
            pr.closed = true;
        }
        return PromptKeyResult::Close;
    }
    let size = pr.buffer.len();

    let mut key = KeyCode(key.0 & !KeyMasks::FLAGS);
    key = prompt_keypad_key(key);

    let mut step = KeyStep::Process;
    let mut prefix = b'=';
    let mut result = PromptKeyResult::Handled;

    if pr.flags.contains(PromptFlags::NUMERIC) {
        if key.0 >= b'0' as u64 && key.0 <= b'9' as u64 {
            step = KeyStep::Append;
        } else {
            let s = pr.buffer.to_bytes();
            if !prompt_fire_callback(srv, pr, Some(&s), PromptKeyResult::Close, None) {
                pr.closed = true;
            }
            return PromptKeyResult::NotHandled;
        }
    } else if pr
        .flags
        .intersects(PromptFlags::SINGLE | PromptFlags::QUOTENEXT)
    {
        if key.0 & KeyMasks::KEY == SpecialKey::BSPACE {
            key = KeyCode(0x7f);
        } else if key.0 & KeyMasks::KEY > 0x7f {
            if !key.is_unicode() {
                return PromptKeyResult::Handled;
            }
            key = KeyCode(key.0 & KeyMasks::KEY);
        } else if key.0 & KeyModifiers::CTRL.0 != 0 {
            key = KeyCode(key.0 & 0x1f);
        } else {
            key = KeyCode(key.0 & KeyMasks::KEY);
        }
        pr.flags.remove(PromptFlags::QUOTENEXT);
        step = KeyStep::Append;
    } else if pr.keys == ModeKeys::Vi {
        let mut new_key = key;
        match prompt_translate_key(pr, key, &mut new_key, redraw) {
            1 => key = new_key,
            2 => {
                key = new_key;
                step = KeyStep::Append;
            }
            _ => return PromptKeyResult::Handled,
        }
    }

    if let KeyStep::Process = step {
        let r = prompt_check_move(srv, pr, key);
        if r != PromptKeyResult::NotHandled {
            return r;
        }
        let k = key.0;
        let seps = pr.word_separators.clone();
        let mut changed = false;
        let mut append = false;
        if k == SpecialKey::LEFT || k == ctrl(b'b') {
            if pr.index > 0 {
                pr.index -= 1;
            }
        } else if k == SpecialKey::RIGHT || k == ctrl(b'f') {
            if pr.index < size {
                pr.index += 1;
            }
        } else if k == SpecialKey::HOME || k == ctrl(b'a') {
            pr.index = 0;
        } else if k == SpecialKey::END || k == ctrl(b'e') {
            pr.index = size;
        } else if k == TAB {
            if prompt_replace_complete(pr, srv, None) {
                changed = true;
            }
        } else if k == SpecialKey::BSPACE || k == ctrl(b'h') {
            if pr.flags.contains(PromptFlags::BSPACE_EXIT) && size == 0 {
                return prompt_done(srv, pr, None, redraw);
            }
            if pr.index != 0 {
                pr.buffer.0.remove(pr.index - 1);
                pr.index -= 1;
                changed = true;
            }
        } else if k == SpecialKey::DC || k == ctrl(b'd') {
            if pr.index != size {
                pr.buffer.0.remove(pr.index);
                changed = true;
            }
        } else if k == ctrl(b'u') {
            pr.buffer.0.clear();
            pr.index = 0;
            changed = true;
        } else if k == ctrl(b'k') {
            if pr.index < size {
                pr.buffer.0.truncate(pr.index);
                changed = true;
            }
        } else if k == ctrl(b'w') {
            let mut idx = pr.index;
            {
                let buf = &pr.buffer.0;
                let sp = |i: usize| buf.get(i).is_some_and(prompt_space);
                let in_list = |i: usize| buf.get(i).is_some_and(|u| prompt_in_list(&seps, u));
                while idx != 0 {
                    idx -= 1;
                    if !sp(idx) {
                        break;
                    }
                }
                let word_is_separators = in_list(idx);
                while idx != 0 {
                    idx -= 1;
                    if sp(idx) || word_is_separators != in_list(idx) {
                        idx += 1;
                        break;
                    }
                }
            }
            let copied: Vec<Utf8Data> = pr.buffer.0[idx..pr.index].to_vec();
            pr.copied = Some(Utf8String(copied));
            pr.buffer.0.drain(idx..pr.index);
            pr.index = idx;
            changed = true;
        } else if k == SpecialKey::RIGHT | KeyModifiers::CTRL.0 || k == meta(b'f') {
            prompt_forward_word(pr, size, false, &seps);
            changed = true;
        } else if k == vi(b'E') {
            prompt_end_word(pr, size, b"");
            changed = true;
        } else if k == vi(b'e') {
            prompt_end_word(pr, size, &seps);
            changed = true;
        } else if k == vi(b'W') {
            prompt_forward_word(pr, size, true, b"");
            changed = true;
        } else if k == vi(b'w') {
            prompt_forward_word(pr, size, true, &seps);
            changed = true;
        } else if k == vi(b'B') {
            prompt_backward_word(pr, b"");
            changed = true;
        } else if k == SpecialKey::LEFT | KeyModifiers::CTRL.0 || k == meta(b'b') {
            prompt_backward_word(pr, &seps);
            changed = true;
        } else if k == SpecialKey::UP || k == ctrl(b'p') {
            let ty = pr.ty;
            if let Some(hist) = history::up(&srv.prompt_history, &mut pr.hindex, ty) {
                let hist = hist.to_vec();
                set_buffer(pr, &hist);
                changed = true;
            }
        } else if k == SpecialKey::DOWN || k == ctrl(b'n') {
            let ty = pr.ty;
            let hist = history::down(&srv.prompt_history, &mut pr.hindex, ty).to_vec();
            set_buffer(pr, &hist);
            changed = true;
        } else if k == ctrl(b'y') {
            if prompt_paste(pr, srv) {
                changed = true;
            }
        } else if k == ctrl(b't') {
            let mut idx = pr.index;
            if idx < size {
                idx += 1;
            }
            if idx >= 2 {
                pr.buffer.0.swap(idx - 2, idx - 1);
                pr.index = idx;
                changed = true;
            }
        } else if k == CR || k == LF {
            let s = pr.buffer.to_bytes();
            if !s.is_empty() {
                let limit = srv
                    .options
                    .get_number(srv.options.global, b"prompt-history-limit")
                    .max(0) as u32;
                history::add(&mut srv.prompt_history, limit, &s, pr.ty);
            }
            return prompt_done(srv, pr, Some(&s), redraw);
        } else if k == ESC || k == ctrl(b'[') || k == ctrl(b'c') || k == ctrl(b'g') {
            return prompt_done(srv, pr, None, redraw);
        } else if k == ctrl(b'r') || k == ctrl(b's') {
            if pr.flags.contains(PromptFlags::INCREMENTAL) {
                if pr.buffer.is_empty() {
                    prefix = b'=';
                    let last = pr.last.clone().unwrap_or_default();
                    set_buffer(pr, &last);
                } else {
                    prefix = if k == ctrl(b'r') { b'-' } else { b'+' };
                }
                changed = true;
            }
        } else if k == ctrl(b'v') {
            pr.flags.insert(PromptFlags::QUOTENEXT);
        } else {
            append = true;
        }
        if changed {
            pr.native_undo.clear();
        }
        if !changed && !append {
            *redraw = true;
            return PromptKeyResult::Handled;
        }
        if append {
            step = KeyStep::Append;
        }
    }

    if let KeyStep::Append = step {
        let tmp = if key.0 <= 0x7f {
            let mut t = Utf8Data::set(key.0 as u8);
            if key.0 <= 0x1f || key.0 == 0x7f {
                t.width = 2;
            }
            t
        } else if key.is_unicode() {
            // Keys above 0x7f are packed utf8_char values (prompt.c:1496-1499).
            let t = utf8::to_data(utf8::Utf8Char((key.0 & KeyMasks::KEY) as u32));
            if t.size == 0 {
                return PromptKeyResult::Handled;
            }
            t
        } else {
            return PromptKeyResult::Handled;
        };
        pr.native_undo.clear();
        let idx = pr.index.min(pr.buffer.len());
        pr.buffer.0.insert(idx, tmp);
        pr.index = idx + 1;

        if pr.flags.contains(PromptFlags::SINGLE) {
            if pr.buffer.len() != 1 {
                pr.closed = true;
                result = PromptKeyResult::Close;
            } else {
                let s = pr.buffer.to_bytes();
                result = prompt_done(srv, pr, Some(&s), redraw);
            }
        }
    }

    // changed:
    *redraw = true;
    if pr.flags.contains(PromptFlags::INCREMENTAL) {
        let cp = prefixed(prefix, &pr.buffer.to_bytes());
        prompt_fire_callback(srv, pr, Some(&cp), PromptKeyResult::Handled, None);
    }
    result
}

/// Add to completion list.
fn prompt_complete_add(list: &mut Vec<ByteString>, s: &[u8]) {
    if list.iter().any(|l| l.as_bytes() == s) {
        return;
    }
    list.push(s.into());
}

/// Build completion list.
pub fn prompt_complete_commands(srv: &Server, s: &[u8]) -> Vec<ByteString> {
    let mut list = Vec::new();
    for entry in crate::cmd::COMMAND_TABLE {
        if entry.name.starts_with(s) {
            prompt_complete_add(&mut list, entry.name);
        }
    }
    if let Some(o) = srv.options.get_only(srv.options.global, b"command-alias") {
        for (_, item) in o.array_items() {
            let value = item.value().as_string();
            let Some(eq) = value.iter().position(|&b| b == b'=') else {
                continue;
            };
            let name = &value[..eq];
            if s.len() > name.len() || !name.starts_with(s) {
                continue;
            }
            prompt_complete_add(&mut list, name);
        }
    }
    list
}

/// Find longest prefix.
fn prompt_complete_prefix(list: &[ByteString]) -> Option<ByteString> {
    let first = list.first()?;
    let mut out: Vec<u8> = first.to_vec();
    for item in &list[1..] {
        let mut j = 0;
        while j < out.len() && j < item.len() && out[j] == item[j] {
            j += 1;
        }
        out.truncate(j);
    }
    Some(out.into())
}

/// Free the stored inline completion matches.
fn prompt_clear_complete(pr: &mut Prompt) {
    pr.complete_list.clear();
    pr.complete_display = None;
    pr.complete_display_ud = None;
}

/// Store the match list for inline display and build the suffix string: a
/// leading space then the matches separated by spaces.
fn prompt_store_complete(pr: &mut Prompt, list: Vec<ByteString>) {
    prompt_clear_complete(pr);
    let mut display = Vec::new();
    for item in &list {
        display.push(b' ');
        display.extend_from_slice(item);
    }
    pr.complete_list = list;
    pr.complete_display_ud = Some(utf8::from_cstr(&display));
    pr.complete_display = Some(display.into());
}

/// Complete word. Returns the text to insert when a unique match or a longer
/// common prefix is available; otherwise stores the match list for inline
/// display (and returns None) or returns None if there is nothing to do.
fn prompt_complete(pr: &mut Prompt, srv: &Server, word: &[u8], offset: u32) -> Option<ByteString> {
    if pr.ty != PromptType::Command || offset != 0 || word.is_empty() {
        return None;
    }
    let mut list = prompt_complete_commands(srv, word);
    if list.is_empty() {
        return None;
    }
    list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    for (i, item) in list.iter().enumerate() {
        log_debug!("complete {}: {}", i, item);
    }
    let mut out = if list.len() == 1 {
        let mut s = list[0].to_vec();
        s.push(b' ');
        Some(ByteString::from(s))
    } else {
        prompt_complete_prefix(&list)
    };
    if out.as_ref().is_some_and(|o| o.as_bytes() == word) {
        out = None;
    }
    if out.is_some() || list.len() <= 1 {
        return out;
    }
    prompt_store_complete(pr, list);
    None
}

/// Return the type of the prompt as an enum.
pub fn prompt_type(name: &[u8]) -> PromptType {
    for ty in [PromptType::Command, PromptType::Search] {
        if prompt_type_string(ty).as_bytes() == name {
            return ty;
        }
    }
    PromptType::Invalid
}

/// Get prompt type as a string.
pub fn prompt_type_string(ty: PromptType) -> &'static str {
    match ty {
        PromptType::Command => "command",
        PromptType::Search => "search",
        PromptType::Invalid => "invalid",
    }
}

/* Pane prompt adapter (window.c:1893-1931,1973-2030). */

/// Owned pane prompt continuation.
pub trait PanePromptInput {
    fn fire(
        &mut self,
        srv: &mut Server,
        wp: PaneId,
        c: Option<ClientId>,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult;
    fn take_update(&mut self) -> Option<(ByteString, ByteString)> {
        None
    }
    /// prompt_free_cb: called exactly once before the continuation drops.
    fn free(&mut self, _srv: &mut Server) {}
}

impl<F> PanePromptInput for F
where
    F: FnMut(&mut Server, PaneId, Option<ClientId>, Option<&[u8]>, PromptKeyResult) -> PromptResult,
{
    fn fire(
        &mut self,
        srv: &mut Server,
        wp: PaneId,
        c: Option<ClientId>,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        self(srv, wp, c, text, key)
    }
}

/// Host for a prompt inside a pane: identifies the pane and the transient
/// input client of the key being dispatched.
struct PanePromptHost {
    wp: PaneId,
    client: Option<ClientId>,
    input: Option<Box<dyn PanePromptInput>>,
}

impl PromptHost for PanePromptHost {
    fn fire(
        &mut self,
        srv: &mut Server,
        text: Option<&[u8]>,
        key: PromptKeyResult,
    ) -> PromptResult {
        match self.input.as_mut() {
            Some(input) => input.fire(srv, self.wp, self.client, text, key),
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

/// The G17 engine behind a model PanePrompt.
pub struct PanePromptAdapter {
    prompt: Prompt,
}

impl PanePromptAdapter {
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }
    fn set_client(&mut self, c: Option<ClientId>) {
        if let Some(host) = self.prompt.host.as_mut() {
            host.set_pane_client(c);
        }
    }
}

impl dyn PromptHost {
    fn set_pane_client(&mut self, _c: Option<ClientId>) {}
}

impl PanePromptEngine for PanePromptAdapter {
    fn start(&mut self, server: &mut Server, _pane: PaneId) {
        prompt_incremental_start(server, &mut self.prompt);
    }
    fn key(
        &mut self,
        server: &mut Server,
        _pane: PaneId,
        client: ClientId,
        key: KeyCode,
        mouse: Option<(u32, u32)>,
    ) -> PromptKeyResult {
        self.set_client(Some(client));
        let mut redraw = false;
        let result = match mouse {
            Some((x, width)) => prompt_mouse(&mut self.prompt, server, x, 0, width, &mut redraw),
            None => prompt_key(server, &mut self.prompt, key, &mut redraw),
        };
        self.set_client(None);
        result
    }
    fn is_open(&self) -> bool {
        !self.prompt.closed
    }
    fn update(&mut self, server: &mut Server, message: &[u8], input: &[u8]) {
        prompt_update(&mut self.prompt, server, message, Some(input));
    }
    fn draw(
        &mut self,
        server: &mut Server,
        _pane: PaneId,
        ctx: &mut ScreenWriteCtx<'_>,
        pdd: &mut PromptDrawData<'_>,
    ) {
        let plan = prompt_draw(&self.prompt, server, pdd.area_x, pdd.area_width);
        plan.render(&self.prompt, ctx, pdd);
    }
    fn free(mut self: Box<Self>, server: &mut Server) {
        if let Some(mut host) = self.prompt.host.take() {
            host.free(server);
        }
    }
}

/// window_pane_set_prompt: open a prompt inside a pane.
#[allow(clippy::too_many_arguments)]
pub fn pane_prompt_set(
    srv: &mut Server,
    wp: PaneId,
    c: ClientId,
    fs: Option<&CmdFindState>,
    msg: &[u8],
    input: Option<&[u8]>,
    input_cb: Option<Box<dyn PanePromptInput>>,
    flags: PromptFlags,
    ty: PromptType,
) {
    let oo = match srv
        .clients
        .get(c)
        .and_then(|c| c.session)
        .and_then(|s| srv.sessions.get(s))
    {
        Some(s) => s.options,
        None => srv.options.global_s,
    };
    let mut pd = PromptCreateData::default();
    prompt_set_options(srv, &mut pd, oo);
    pd.fs = fs.copied();
    pd.prompt = msg.into();
    pd.input = input.map(ByteString::from);
    pd.ty = ty;
    pd.flags = flags | PromptFlags::ISPANE;
    let host = Box::new(PanePromptHost {
        wp,
        client: None,
        input: input_cb,
    });
    let prompt = prompt_create(srv, pd, host);
    let _ = crate::model::pane::pane_set_prompt(
        srv,
        wp,
        PanePrompt {
            kind: ty,
            engine: Box::new(PanePromptAdapter { prompt }),
        },
    );
}

/// window_pane_update_prompt.
pub fn pane_prompt_update(srv: &mut Server, wp: PaneId, msg: &[u8], input: Option<&[u8]>) {
    let _ = crate::model::pane::pane_update_prompt(srv, wp, msg, input.unwrap_or(b""));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoHost;
    impl PromptHost for NoHost {
        fn fire(&mut self, _: &mut Server, _: Option<&[u8]>, _: PromptKeyResult) -> PromptResult {
            PromptResult::Continue
        }
    }

    fn prompt(text: &[u8], flags: PromptFlags) -> Prompt {
        let buffer = utf8::from_cstr(text);
        let index = buffer.len();
        Prompt {
            host: Some(Box::new(NoHost)),
            string: ByteString::new(),
            buffer,
            state: CmdFindState::clear(CmdFindFlags(0)),
            last: None,
            index,
            message_format: ByteString::new(),
            keys: ModeKeys::Emacs,
            word_separators: b" -_@".as_slice().into(),
            style: DEFAULT_CELL,
            command_style: DEFAULT_CELL,
            style_str: ByteString::new(),
            command_style_str: ByteString::new(),
            cstyle: ScreenCursorStyle::Default,
            command_cstyle: ScreenCursorStyle::Default,
            ccolour: Colour::DEFAULT,
            command_ccolour: Colour::DEFAULT,
            cmode: ScreenMode(0),
            command_cmode: ScreenMode(0),
            ty: PromptType::Command,
            flags,
            closed: false,
            hindex: [0; PROMPT_NTYPES],
            copied: None,
            native_undo: std::collections::VecDeque::new(),
            complete_list: Vec::new(),
            complete_display: None,
            complete_display_ud: None,
        }
    }

    #[test]
    fn native_edit_uses_utf16_offsets_and_rejects_stale_or_split_surrogates() {
        let mut server = Server::new();
        let mut pr = prompt("a😀b".as_bytes(), PromptFlags::default());
        prompt_native_edit(
            &mut server,
            &mut pr,
            &serde_json::json!({
                "from":1, "to":3, "text":"中", "cursor":2, "len":4
            }),
        );
        assert_eq!(pr.input(), "a中b".as_bytes());
        assert_eq!(pr.index(), 2);
        prompt_native_edit(
            &mut server,
            &mut pr,
            &serde_json::json!({
                "from":0, "to":3, "text":"stale", "cursor":5, "len":4
            }),
        );
        assert_eq!(pr.input(), "a中b".as_bytes());
        let mut pr = prompt("a😀b".as_bytes(), PromptFlags::default());
        prompt_native_edit(
            &mut server,
            &mut pr,
            &serde_json::json!({
                "from":2, "to":3, "text":"split", "cursor":6, "len":4
            }),
        );
        assert_eq!(pr.input(), "a😀b".as_bytes());
        prompt_native_edit(
            &mut server,
            &mut pr,
            &serde_json::json!({
                "from":1, "to":3, "text":"bye\nnow", "cursor":8, "len":4
            }),
        );
        assert_eq!(pr.input(), b"abyenowb");
        assert_eq!(pr.index(), 7);
    }

    #[test]
    fn native_paste_maps_raw_cursor_and_strips_all_c0_and_del() {
        let mut server = Server::new();
        let mut pr = prompt(b"", PromptFlags::default());
        prompt_native_edit(
            &mut server,
            &mut pr,
            &serde_json::json!({
                "from":0,"to":0,"text":"foo\n","cursor":4,"len":0
            }),
        );
        assert_eq!(pr.input(), b"foo");
        assert_eq!(pr.index(), 3);
        prompt_native_undo(&mut server, &mut pr);
        assert!(pr.input().is_empty());
        prompt_native_edit(
            &mut server,
            &mut pr,
            &serde_json::json!({
                "from":0,"to":0,"text":"\u{0}a\t\r\n\u{1b}😀\u{7f}b","cursor":9,"len":0
            }),
        );
        assert_eq!(pr.input(), "a😀b".as_bytes());
        assert_eq!(pr.index(), 2);
        prompt_native_edit(
            &mut server,
            &mut pr,
            &serde_json::json!({
                "from":0,"to":0,"text":"😀","cursor":1,"len":4
            }),
        );
        assert_eq!(pr.input(), "a😀b".as_bytes());
        let mut redraw = false;
        prompt_key(&mut server, &mut pr, KeyCode(b'x' as u64), &mut redraw);
        prompt_native_undo(&mut server, &mut pr);
        assert_eq!(pr.input(), "a😀xb".as_bytes());
    }

    #[test]
    fn callback_updates_the_in_flight_prompt_for_next_input() {
        struct NextPrompt(Option<(ByteString, ByteString)>);
        impl PromptHost for NextPrompt {
            fn fire(
                &mut self,
                _: &mut Server,
                text: Option<&[u8]>,
                key: PromptKeyResult,
            ) -> PromptResult {
                assert_eq!(text, Some(b"one".as_slice()));
                assert_eq!(key, PromptKeyResult::Close);
                self.0 = Some((b"second ".as_slice().into(), b"seed".as_slice().into()));
                PromptResult::Continue
            }
            fn take_update(&mut self) -> Option<(ByteString, ByteString)> {
                self.0.take()
            }
        }
        let mut server = Server::new();
        let mut pr = prompt(b"one", PromptFlags::default());
        pr.host = Some(Box::new(NextPrompt(None)));
        let mut redraw = false;
        assert_eq!(
            prompt_key(&mut server, &mut pr, KeyCode(CR), &mut redraw),
            PromptKeyResult::Handled
        );
        assert_eq!(pr.string, b"second ");
        assert_eq!(pr.input(), b"seed");
        assert_eq!(pr.index(), 4);
        assert!(!prompt_closed(&pr));
        assert!(redraw);
    }

    #[test]
    fn unicode_keys_are_packed_utf8_chars_not_code_points() {
        // "中" arrives as the utf8_char tty-keys/send-keys build
        // (prompt.c:1496 utf8_to_data), not as U+4E2D.
        let ud = utf8::from_cstr("中".as_bytes()).0.remove(0);
        let (uc, state) = utf8::from_data(&ud);
        assert_eq!(state, utf8::Utf8State::Done);
        let mut server = Server::new();
        let mut pr = prompt(b"a", PromptFlags::default());
        let mut redraw = false;
        assert_eq!(
            prompt_key(&mut server, &mut pr, KeyCode(u64::from(uc.0)), &mut redraw),
            PromptKeyResult::Handled
        );
        assert_eq!(pr.input(), "a中".as_bytes());
        assert_eq!(pr.buffer.0[1].width, 2);
        assert_eq!(pr.index(), 2);
        // Quote-next takes the same packed form (prompt.c:1256-1260).
        pr.flags.insert(PromptFlags::QUOTENEXT);
        prompt_key(&mut server, &mut pr, KeyCode(u64::from(uc.0)), &mut redraw);
        assert_eq!(pr.input(), "a中中".as_bytes());
    }

    #[test]
    fn layout_cases() {
        let mut pr = prompt(b"hello", PromptFlags::default());
        assert_eq!(
            prompt_layout_geometry(&pr, 0, 0, b"x", None),
            PromptLayout::default()
        );
        // Label longer than the area: input gets nothing.
        let pl = prompt_layout_geometry(&pr, 0, 4, b"label:", None);
        assert_eq!(pl.label_width, 4);
        assert_eq!(pl.input_width, 0);
        assert_eq!(pl.cursor_x, 4);
        // Normal case.
        let pl = prompt_layout_geometry(&pr, 2, 20, b":", None);
        assert_eq!((pl.content_x, pl.input_x, pl.cursor_x), (2, 3, 8));
        assert_eq!(pl.input_width, 5);
        assert_eq!(pl.content_width, 6);
        // Cursor past the area scrolls: avail 4, pcursor 5 -> offset 2.
        let pl = prompt_layout_geometry(&pr, 0, 5, b":", None);
        assert_eq!(pl.input_offset, 2);
        assert_eq!(pl.input_width, 4);
        assert_eq!(pl.cursor_x, 4);
        // Completion display widens content at buffer end; right align.
        pr.complete_display = Some(b" split-window splitw".as_slice().into());
        let pl = prompt_layout_geometry(&pr, 0, 40, b":", Some(StyleAlign::Right));
        assert_eq!(pl.content_width, 6 + 20);
        assert_eq!(pl.content_x, 40 - 26);
        pr.index = 1;
        let pl = prompt_layout_geometry(&pr, 0, 40, b":", None);
        assert_eq!(pl.content_width, 6);
    }

    #[test]
    fn vi_translation_table() {
        let mut pr = prompt(b"abc", PromptFlags::COMMANDMODE);
        let mut redraw = false;
        let cases: [(u64, u8, Option<u64>); 22] = [
            (b'A' as u64, 1, Some(SpecialKey::END)),
            (b'$' as u64, 1, Some(SpecialKey::END)),
            (b'I' as u64, 1, Some(SpecialKey::HOME)),
            (b'0' as u64, 1, Some(SpecialKey::HOME)),
            (b'^' as u64, 1, Some(SpecialKey::HOME)),
            (b'C' as u64, 1, Some(ctrl(b'k'))),
            (b'D' as u64, 1, Some(ctrl(b'k'))),
            (b'X' as u64, 1, Some(SpecialKey::BSPACE)),
            (b'b' as u64, 1, Some(meta(b'b'))),
            (b'B' as u64, 1, Some(vi(b'B'))),
            (b'd' as u64, 1, Some(ctrl(b'u'))),
            (b'e' as u64, 1, Some(vi(b'e'))),
            (b'E' as u64, 1, Some(vi(b'E'))),
            (b'w' as u64, 1, Some(vi(b'w'))),
            (b'W' as u64, 1, Some(vi(b'W'))),
            (b'p' as u64, 1, Some(ctrl(b'y'))),
            (b'q' as u64, 1, Some(ctrl(b'c'))),
            (b'x' as u64, 1, Some(SpecialKey::DC)),
            (b'j' as u64, 1, Some(SpecialKey::DOWN)),
            (b'h' as u64, 1, Some(SpecialKey::LEFT)),
            (b'l' as u64, 1, Some(SpecialKey::RIGHT)),
            (b'k' as u64, 1, Some(SpecialKey::UP)),
        ];
        for (key, want, mapped) in cases {
            pr.flags = PromptFlags::COMMANDMODE;
            let mut nk = KeyCode(0);
            let r = prompt_translate_key(&mut pr, KeyCode(key), &mut nk, &mut redraw);
            assert_eq!(r, want, "key {key:#x}");
            if let Some(m) = mapped {
                assert_eq!(nk.0, m, "key {key:#x}");
            }
        }
        // Mode switches.
        pr.flags = PromptFlags::COMMANDMODE;
        let mut nk = KeyCode(0);
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(b'i' as u64), &mut nk, &mut redraw),
            0
        );
        assert!(!pr.flags.contains(PromptFlags::COMMANDMODE));
        pr.flags = PromptFlags::COMMANDMODE;
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(b'S' as u64), &mut nk, &mut redraw),
            1
        );
        assert_eq!(nk.0, ctrl(b'u'));
        assert!(!pr.flags.contains(PromptFlags::COMMANDMODE));
        pr.flags = PromptFlags::COMMANDMODE;
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(ESC), &mut nk, &mut redraw),
            0
        );
        assert!(pr.flags.contains(PromptFlags::COMMANDMODE));
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(CR), &mut nk, &mut redraw),
            1
        );
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(b'z' as u64), &mut nk, &mut redraw),
            0
        );
        // Insert mode: Escape enters command mode and backs the cursor.
        pr.flags = PromptFlags::default();
        pr.index = 3;
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(ESC), &mut nk, &mut redraw),
            0
        );
        assert!(pr.flags.contains(PromptFlags::COMMANDMODE));
        assert_eq!(pr.index, 2);
        pr.flags = PromptFlags::default();
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(b'z' as u64), &mut nk, &mut redraw),
            2
        );
        assert_eq!(
            prompt_translate_key(&mut pr, KeyCode(SpecialKey::UP), &mut nk, &mut redraw),
            1
        );
    }

    #[test]
    fn word_motions() {
        // "foo bar-baz  qux"
        let text = b"foo bar-baz  qux";
        let seps = b" -_@";
        let mut pr = prompt(text, PromptFlags::default());
        let size = pr.buffer.len();
        pr.index = 0;
        prompt_forward_word(&mut pr, size, false, seps);
        assert_eq!(pr.index, 3);
        prompt_forward_word(&mut pr, size, false, seps);
        assert_eq!(pr.index, 7);
        prompt_forward_word(&mut pr, size, false, seps);
        assert_eq!(pr.index, 8);
        pr.index = 0;
        prompt_forward_word(&mut pr, size, true, seps);
        assert_eq!(pr.index, 4);
        prompt_forward_word(&mut pr, size, true, b"");
        assert_eq!(pr.index, 13);
        pr.index = 0;
        prompt_end_word(&mut pr, size, seps);
        assert_eq!(pr.index, 2);
        prompt_end_word(&mut pr, size, seps);
        assert_eq!(pr.index, 6);
        prompt_end_word(&mut pr, size, b"");
        assert_eq!(pr.index, 10);
        pr.index = size;
        prompt_backward_word(&mut pr, seps);
        assert_eq!(pr.index, 13);
        prompt_backward_word(&mut pr, seps);
        assert_eq!(pr.index, 8);
        prompt_backward_word(&mut pr, b"");
        assert_eq!(pr.index, 4);
        prompt_backward_word(&mut pr, b"");
        assert_eq!(pr.index, 0);
        pr.index = size;
        prompt_end_word(&mut pr, size, seps);
        assert_eq!(pr.index, size);
    }

    #[test]
    fn keypad_and_prefix_helpers() {
        assert_eq!(
            prompt_keypad_key(KeyCode(SpecialKey::KP_SEVEN)).0,
            b'7' as u64
        );
        assert_eq!(
            prompt_keypad_key(KeyCode(SpecialKey::KP_ENTER)).0,
            b'\r' as u64
        );
        assert_eq!(
            prompt_keypad_key(KeyCode(SpecialKey::KP_SEVEN | KeyModifiers::CTRL.0)).0,
            SpecialKey::KP_SEVEN | KeyModifiers::CTRL.0
        );
        assert_eq!(
            prompt_complete_prefix(&[
                b"splitw".as_slice().into(),
                b"split-window".as_slice().into()
            ])
            .unwrap()
            .as_bytes(),
            b"split"
        );
        assert_eq!(prefixed(b'-', b"abc").as_bytes(), b"-abc");
        assert_eq!(prompt_type(b"search"), PromptType::Search);
        assert_eq!(prompt_type(b"nope"), PromptType::Invalid);
        assert_eq!(prompt_type_string(PromptType::Command), "command");
    }

    #[test]
    fn slot_generations_prevent_resurrection() {
        let mut slot = PromptSlot::default();
        assert!(!slot.is_some());
        let g = slot.install_pending();
        assert!(
            slot.restore(prompt(b"", PromptFlags::default()), g)
                .is_none()
        );
        assert!(slot.is_some());
        let (p, g) = slot.take_value().unwrap();
        assert!(slot.as_ref().is_none());
        assert!(slot.clear().is_none());
        assert!(!slot.is_some());
        // Stale restore is rejected.
        assert!(slot.restore(p, g).is_some());
        let g2 = slot.install_pending();
        assert!(
            slot.restore(prompt(b"x", PromptFlags::default()), g2)
                .is_none()
        );
        assert!(
            slot.restore(prompt(b"y", PromptFlags::default()), g2)
                .is_some()
        );
        assert_eq!(slot.clear().unwrap().input().as_bytes(), b"x");
    }
}
