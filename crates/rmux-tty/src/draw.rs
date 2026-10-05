// Ported from tmux tty.c, tty-draw.c and tmux.h @ 8f25579c
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TtyCtxFlags(pub u32);
impl TtyCtxFlags {
    pub const WRAPPED: Self = Self(1);
    pub const INVISIBLE_PANES: Self = Self(2);
    pub const WINDOW_BIGGER: Self = Self(4);
    pub const SYNC: Self = Self(8);
    pub const CELL_INVALIDATE: Self = Self(32);
    pub const PANE_OBSCURED: Self = Self(64);
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
impl std::ops::BitOr for TtyCtxFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for TtyCtxFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for TtyCtxFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

pub mod line;
#[cfg(test)]
mod tests;

use crate::term::tparm::TparmState;
use crate::term::{TtyCodeCode as Code, TtyTermFlags};
use crate::tty::{Tty, TtyFlags};
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell, GridCellFlags};
use rmux_emu::colour::ColourPalette;
use rmux_emu::hyperlinks::{HyperlinkRegistry, Hyperlinks};
use rmux_emu::screen::Screen;

#[derive(Clone, Copy)]
pub struct TtyStyleCtx<'a> {
    pub defaults: &'a GridCell,
    pub palette: Option<&'a ColourPalette>,
    pub dim: u32,
    pub hyperlinks: Option<(&'a HyperlinkRegistry, &'a Hyperlinks)>,
}
impl Default for TtyStyleCtx<'_> {
    fn default() -> Self {
        Self {
            defaults: &DEFAULT_CELL,
            palette: None,
            dim: 0,
            hyperlinks: None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct TtyBytes<'a> {
    pub data: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub struct TtySelection<'a> {
    pub clip: &'a str,
    pub data: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub enum TtyCommandData<'a> {
    Count(u32),
    Bytes(&'a [u8]),
    Selection { clip: &'a str, data: &'a [u8] },
}
pub struct TtyCtx<'a> {
    pub s: &'a Screen,
    pub cell: &'a GridCell,
    pub flags: TtyCtxFlags,
    pub data: TtyCommandData<'a>,
    pub ocx: u32,
    pub ocy: u32,
    pub orupper: u32,
    pub orlower: u32,
    pub xoff: i32,
    pub yoff: i32,
    pub rxoff: i32,
    pub ryoff: i32,
    pub sx: u32,
    pub sy: u32,
    pub bg: u32,
    pub defaults: GridCell,
    pub style_ctx: TtyStyleCtx<'a>,
    pub wox: u32,
    pub woy: u32,
    pub wsx: u32,
    pub wsy: u32,
}
impl TtyCtx<'_> {
    fn count(&self) -> u32 {
        match self.data {
            TtyCommandData::Count(n) => n,
            _ => panic!("count command without count"),
        }
    }
    fn bytes(&self) -> &[u8] {
        match self.data {
            TtyCommandData::Bytes(b) => b,
            _ => panic!("byte command without bytes"),
        }
    }
    fn full_width(&self, tty: &Tty) -> bool {
        self.xoff == 0 && self.sx >= tty.sx
    }
    fn bigger(&self) -> bool {
        self.flags.contains(TtyCtxFlags::WINDOW_BIGGER)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TtyCommand {
    InsertCharacter,
    DeleteCharacter,
    ClearCharacter,
    InsertLine,
    DeleteLine,
    ClearLine,
    ClearEndOfLine,
    ClearStartOfLine,
    ReverseIndex,
    LineFeed,
    ScrollUp,
    ScrollDown,
    ClearEndOfScreen,
    ClearStartOfScreen,
    ClearScreen,
    AlignmentTest,
    Cell,
    Cells,
    RedrawLine,
    SetSelection,
    RawString,
    SyncStart,
}

impl From<&rmux_emu::screen::write::DrawCommand<'_>> for TtyCommand {
    fn from(command: &rmux_emu::screen::write::DrawCommand<'_>) -> Self {
        use rmux_emu::screen::write::DrawCommand as Draw;
        match command {
            Draw::SyncStart => Self::SyncStart,
            Draw::Cell(_) => Self::Cell,
            Draw::Cells { .. } => Self::Cells,
            Draw::RedrawLine { .. } => Self::RedrawLine,
            Draw::AlignmentTest => Self::AlignmentTest,
            Draw::InsertCharacter { .. } => Self::InsertCharacter,
            Draw::DeleteCharacter { .. } => Self::DeleteCharacter,
            Draw::ClearCharacter { .. } => Self::ClearCharacter,
            Draw::InsertLine { .. } => Self::InsertLine,
            Draw::DeleteLine { .. } => Self::DeleteLine,
            Draw::ClearEndOfScreen { .. } => Self::ClearEndOfScreen,
            Draw::ClearStartOfScreen { .. } => Self::ClearStartOfScreen,
            Draw::ClearScreen { .. } => Self::ClearScreen,
            Draw::ScrollUp { .. } => Self::ScrollUp,
            Draw::ScrollDown { .. } => Self::ScrollDown,
            Draw::ReverseIndex { .. } => Self::ReverseIndex,
            Draw::SetSelection { .. } => Self::SetSelection,
            Draw::RawString { .. } => Self::RawString,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TtyRedraw {
    pub start_y: u32,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClampedLine {
    skip: u32,
    x: u32,
    width: u32,
    y: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClampedArea {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn position(offset: i32, pos: u32, origin: u32) -> u32 {
    (offset as u32).wrapping_add(pos).wrapping_sub(origin)
}
fn is_visible(ctx: &TtyCtx<'_>, px: u32, py: u32, nx: u32, ny: u32) -> bool {
    if !ctx.bigger() {
        return true;
    }
    let x = (ctx.rxoff as u32).wrapping_add(px);
    let y = (ctx.ryoff as u32).wrapping_add(py);
    !(x.wrapping_add(nx) <= ctx.wox
        || x >= ctx.wox.wrapping_add(ctx.wsx)
        || y.wrapping_add(ny) <= ctx.woy
        || y >= ctx.woy.wrapping_add(ctx.wsy))
}
fn clamp_axis(
    raw: u32,
    adjusted: u32,
    n: u32,
    origin: u32,
    size: u32,
    signed: bool,
) -> (u32, u32, u32) {
    let left = if signed {
        i64::from(raw as i32) < i64::from(origin)
    } else {
        raw < origin
    };
    let end = raw.wrapping_add(n);
    let (skip, at, width) = if !left && end <= origin.wrapping_add(size) {
        (0, adjusted.wrapping_sub(origin), n)
    } else if left && end > origin.wrapping_add(size) {
        (origin, 0, size)
    } else if left {
        let skip = origin.wrapping_sub(adjusted);
        (skip, 0, n.wrapping_sub(skip))
    } else {
        let at = adjusted.wrapping_sub(origin);
        (0, at, size.wrapping_sub(at))
    };
    assert!(width <= n, "clamp result {width} exceeds input {n}");
    (skip, at, width)
}
fn clamp_line(ctx: &TtyCtx<'_>, px: u32, py: u32, nx: u32) -> Option<ClampedLine> {
    if !is_visible(ctx, px, py, nx, 1) {
        return None;
    }
    let (skip, x, width) = clamp_axis(
        position(ctx.rxoff, px, 0),
        position(ctx.xoff, px, 0),
        nx,
        ctx.wox,
        ctx.wsx,
        true,
    );
    Some(ClampedLine {
        skip,
        x,
        width,
        y: position(ctx.yoff, py, ctx.woy),
    })
}
fn clamp_area(ctx: &TtyCtx<'_>, px: u32, py: u32, nx: u32, ny: u32) -> Option<ClampedArea> {
    if !is_visible(ctx, px, py, nx, ny) {
        return None;
    }
    let (_, x, width) = clamp_axis(
        position(ctx.rxoff, px, 0),
        position(ctx.xoff, px, 0),
        nx,
        ctx.wox,
        ctx.wsx,
        false,
    );
    let (_, y, height) = clamp_axis(
        position(ctx.ryoff, py, 0),
        position(ctx.yoff, py, 0),
        ny,
        ctx.woy,
        ctx.wsy,
        false,
    );
    Some(ClampedArea {
        x,
        y,
        width,
        height,
    })
}

impl Tty {
    fn use_margin(&self) -> bool {
        self.term().flags().contains(TtyTermFlags::DECSLRM)
    }
    pub fn fake_bce(&self, gc: &GridCell, bg: u32) -> bool {
        !self.term().flag(Code::Bce) && (!matches!(bg, 8 | 9) || !gc.bg.is_default())
    }
    pub fn check_codeset<'a>(&self, gc: &'a GridCell, scratch: &'a mut GridCell) -> &'a GridCell {
        if (gc.data.size == 1 && gc.data.data[0] < 0x7f)
            || gc.flags.contains(GridCellFlags::TAB)
            || self.host.utf8
        {
            return gc;
        }
        *scratch = *gc;
        if let Some(ch) = crate::acs::acs_reverse_get(gc.data.bytes()) {
            scratch.data = rmux_util::utf8::Utf8Data::set(ch);
            scratch.attr.insert(GridAttributes::CHARSET);
        } else {
            scratch.data.size = gc.data.width.min(scratch.data.data.len() as u8);
            scratch.data.data[..scratch.data.size as usize].fill(b'_');
        }
        scratch
    }
    pub fn cell(&mut self, state: &mut TparmState, gc: &GridCell, style: Option<&TtyStyleCtx<'_>>) {
        if self.term().flags().contains(TtyTermFlags::NOAM)
            && self.cy == self.sy.wrapping_sub(1)
            && self.cx == self.sx.wrapping_sub(1)
        {
            return;
        }
        if gc.flags.contains(GridCellFlags::PADDING) {
            return;
        }
        let mut scratch = DEFAULT_CELL;
        let cell = *self.check_codeset(gc, &mut scratch);
        self.attributes(state, &cell, style);
        if cell.data.size == 1 {
            let ch = cell.data.data[0];
            if ch >= 0x20 && ch != 0x7f {
                self.putc(state, ch);
            }
        } else {
            self.putn(state, cell.data.bytes(), u32::from(cell.data.width));
        }
    }
    fn region_pane(&mut self, state: &mut TparmState, ctx: &TtyCtx<'_>, upper: u32, lower: u32) {
        self.region(
            state,
            position(ctx.yoff, upper, ctx.woy),
            position(ctx.yoff, lower, ctx.woy),
        );
    }
    fn margin_pane(&mut self, state: &mut TparmState, ctx: &TtyCtx<'_>) {
        let left = (i64::from(ctx.xoff) - i64::from(ctx.wox)).clamp(0, i64::from(ctx.wsx));
        let right = (i64::from(ctx.xoff) + i64::from(ctx.sx) - 1 - i64::from(ctx.wox))
            .clamp(0, i64::from(ctx.wsx));
        self.margin(state, left as u32, right as u32);
    }
    fn cursor_pane(&mut self, state: &mut TparmState, ctx: &TtyCtx<'_>, x: u32, y: u32) {
        self.cursor(
            state,
            position(ctx.xoff, x, ctx.wox),
            position(ctx.yoff, y, ctx.woy),
        );
    }
    fn can_wrap(&self, ctx: &TtyCtx<'_>, x: u32, y: u32) -> bool {
        ctx.flags.contains(TtyCtxFlags::WRAPPED)
            && ctx.full_width(self)
            && !self.term().flags().contains(TtyTermFlags::NOAM)
            && position(ctx.xoff, x, 0) == 0
            && position(ctx.yoff, y, 0) == self.cy.wrapping_add(1)
            && self.cx >= self.sx
            && self.cy != self.rlower
    }
    fn cursor_pane_unless_wrap(&mut self, state: &mut TparmState, ctx: &TtyCtx<'_>) {
        if !self.can_wrap(ctx, ctx.ocx, ctx.ocy) {
            self.cursor_pane(state, ctx, ctx.ocx, ctx.ocy);
        }
    }
    fn emulate_repeat(&mut self, state: &mut TparmState, count: Code, single: Code, n: u32) {
        if self.term().has(count) {
            self.putcode_i(state, count, n as i32);
        } else {
            for _ in 0..n {
                self.putcode(single);
            }
        }
    }
    fn clear_line(
        &mut self,
        state: &mut TparmState,
        defaults: &GridCell,
        y: u32,
        x: u32,
        n: u32,
        bg: u32,
    ) {
        if n == 0 {
            return;
        }
        if !self.fake_bce(defaults, bg) {
            if x.wrapping_add(n) >= self.sx && self.term().has(Code::El) {
                self.cursor(state, x, y);
                self.putcode(Code::El);
                return;
            }
            if x == 0 && self.term().has(Code::El1) {
                self.cursor(state, x + n - 1, y);
                self.putcode(Code::El1);
                return;
            }
            if self.term().has(Code::Ech) {
                self.cursor(state, x, y);
                self.putcode_i(state, Code::Ech, n as i32);
                return;
            }
        }
        self.cursor(state, x, y);
        self.repeat_space(state, n);
    }
    fn clear_pane_line(
        &mut self,
        state: &mut TparmState,
        ctx: &TtyCtx<'_>,
        y: u32,
        x: u32,
        n: u32,
    ) {
        if let Some(c) = clamp_line(ctx, x, y, n) {
            self.clear_line(state, &ctx.defaults, c.y, c.x, c.width, ctx.bg);
        }
    }
    fn clear_area(&mut self, state: &mut TparmState, ctx: &TtyCtx<'_>, c: ClampedArea) {
        let ClampedArea {
            x,
            y,
            width: nx,
            height: ny,
        } = c;
        if nx == 0 || ny == 0 {
            return;
        }
        if !self.fake_bce(&ctx.defaults, ctx.bg) {
            if x == 0 && nx >= self.sx && y.wrapping_add(ny) >= self.sy && self.term().has(Code::Ed)
            {
                self.cursor(state, 0, y);
                self.putcode(Code::Ed);
                return;
            }
            if self.term().flags().contains(TtyTermFlags::DECFRA) && !matches!(ctx.bg, 8 | 9) {
                let mut buf = [0; 64];
                let mut output = std::io::Cursor::new(buf.as_mut_slice());
                use std::io::Write;
                write!(
                    output,
                    "\x1b[32;{};{};{};{}$x",
                    y + 1,
                    x + 1,
                    y + ny,
                    x + nx
                )
                .expect("DECFRA fits buffer");
                let len = output.position() as usize;
                self.puts(&buf[..len]);
                return;
            }
            if x == 0
                && nx >= self.sx
                && ny > 2
                && self.term().has(Code::Csr)
                && self.term().has(Code::Indn)
            {
                self.region(state, y, y + ny - 1);
                self.margin_off(state);
                self.putcode_i(state, Code::Indn, ny as i32);
                return;
            }
            if nx > 2
                && ny > 2
                && self.term().has(Code::Csr)
                && self.use_margin()
                && self.term().has(Code::Indn)
            {
                self.region(state, y, y + ny - 1);
                self.margin(state, x, x + nx - 1);
                self.putcode_i(state, Code::Indn, ny as i32);
                return;
            }
        }
        for yy in y..y + ny {
            self.clear_line(state, &ctx.defaults, yy, x, nx, ctx.bg);
        }
    }
    fn clear_pane_area(
        &mut self,
        state: &mut TparmState,
        ctx: &TtyCtx<'_>,
        y: u32,
        ny: u32,
        x: u32,
        nx: u32,
    ) {
        if let Some(c) = clamp_area(ctx, x, y, nx, ny) {
            self.clear_area(state, ctx, c);
        }
    }
    fn draw_pane(&mut self, state: &mut TparmState, ctx: &TtyCtx<'_>, y: u32) {
        if !ctx.bigger() {
            self.draw_line_styled(
                state,
                ctx.s,
                0,
                y,
                ctx.sx,
                ctx.xoff as u32,
                position(ctx.yoff, y, 0),
                &ctx.style_ctx,
            );
        } else if let Some(c) = clamp_line(ctx, 0, y, ctx.sx) {
            self.draw_line_styled(state, ctx.s, c.skip, y, c.width, c.x, c.y, &ctx.style_ctx);
        }
    }
    fn redraw_region(&mut self, state: &mut TparmState, ctx: &TtyCtx<'_>) -> Option<TtyRedraw> {
        if ctx.orlower - ctx.orupper >= ctx.sy / 2 || ctx.flags.contains(TtyCtxFlags::PANE_OBSCURED)
        {
            Some(TtyRedraw {
                start_y: ctx.orupper,
                count: ctx.orlower - ctx.orupper + 1,
            })
        } else {
            for y in ctx.orupper..=ctx.orlower {
                self.draw_pane(state, ctx, y);
            }
            None
        }
    }
}

impl Tty {
    pub fn command(
        &mut self,
        state: &mut TparmState,
        cmd: TtyCommand,
        ctx: &TtyCtx<'_>,
    ) -> Option<TtyRedraw> {
        use TtyCommand::*;
        match cmd {
            InsertCharacter | DeleteCharacter => {
                let (count, single) = if cmd == InsertCharacter {
                    (Code::Ich, Code::Ich1)
                } else {
                    (Code::Dch, Code::Dch1)
                };
                if ctx.bigger()
                    || !ctx.full_width(self)
                    || self.fake_bce(&ctx.defaults, ctx.bg)
                    || (!self.term().has(count) && !self.term().has(single))
                {
                    self.draw_pane(state, ctx, ctx.ocy);
                } else {
                    self.default_attributes(state, ctx.bg, Some(&ctx.style_ctx));
                    self.cursor_pane(state, ctx, ctx.ocx, ctx.ocy);
                    self.emulate_repeat(state, count, single, ctx.count());
                }
            }
            InsertLine | DeleteLine => {
                let (count, single) = if cmd == InsertLine {
                    (Code::Il, Code::Il1)
                } else {
                    (Code::Dl, Code::Dl1)
                };
                if ctx.bigger()
                    || !ctx.full_width(self)
                    || self.fake_bce(&ctx.defaults, ctx.bg)
                    || !self.term().has(Code::Csr)
                    || !self.term().has(single)
                    || ctx.sx == 1
                    || ctx.sy == 1
                {
                    return self.redraw_region(state, ctx);
                }
                self.default_attributes(state, ctx.bg, Some(&ctx.style_ctx));
                self.region_pane(state, ctx, ctx.orupper, ctx.orlower);
                self.margin_off(state);
                self.cursor_pane(state, ctx, ctx.ocx, ctx.ocy);
                self.emulate_repeat(state, count, single, ctx.count());
                self.cx = u32::MAX;
                self.cy = u32::MAX;
            }
            ClearCharacter | ClearLine | ClearEndOfLine | ClearStartOfLine => {
                self.default_attributes(state, ctx.bg, Some(&ctx.style_ctx));
                let (x, n) = match cmd {
                    ClearCharacter => (ctx.ocx, ctx.count()),
                    ClearLine => (0, ctx.sx),
                    ClearEndOfLine => (ctx.ocx, ctx.sx - ctx.ocx),
                    _ => (0, ctx.ocx + 1),
                };
                self.clear_pane_line(state, ctx, ctx.ocy, x, n);
            }
            ReverseIndex | LineFeed | ScrollUp | ScrollDown => {
                if (cmd == ReverseIndex && ctx.ocy != ctx.orupper)
                    || (cmd == LineFeed && ctx.ocy != ctx.orlower)
                {
                    return None;
                }
                if ctx.bigger()
                    || (!ctx.full_width(self) && !self.use_margin())
                    || self.fake_bce(&ctx.defaults, 8)
                    || !self.term().has(Code::Csr)
                    || ctx.sx == 1
                    || ctx.sy == 1
                    || (matches!(cmd, ReverseIndex | ScrollDown)
                        && !self.term().has(Code::Ri)
                        && !self.term().has(Code::Rin))
                {
                    return self.redraw_region(state, ctx);
                }
                self.default_attributes(state, ctx.bg, Some(&ctx.style_ctx));
                self.region_pane(state, ctx, ctx.orupper, ctx.orlower);
                self.margin_pane(state, ctx);
                match cmd {
                    ReverseIndex => {
                        self.cursor_pane(state, ctx, ctx.ocx, ctx.orupper);
                        if self.term().has(Code::Ri) {
                            self.putcode(Code::Ri);
                        } else {
                            self.putcode_i(state, Code::Rin, 1);
                        }
                    }
                    LineFeed => {
                        if position(ctx.xoff, ctx.ocx, 0) > self.rright {
                            self.cursor(
                                state,
                                if self.use_margin() { self.rright } else { 0 },
                                position(ctx.yoff, ctx.ocy, 0),
                            );
                        } else {
                            self.cursor_pane(state, ctx, ctx.ocx, ctx.ocy);
                        }
                        self.putc(state, b'\n');
                    }
                    ScrollUp => {
                        if ctx.count() == 1 || !self.term().has(Code::Indn) {
                            self.cursor(
                                state,
                                if self.use_margin() { self.rright } else { 0 },
                                self.rlower,
                            );
                            for _ in 0..ctx.count() {
                                self.putc(state, b'\n');
                            }
                        } else {
                            self.cursor(state, 0, if self.cy == u32::MAX { 0 } else { self.cy });
                            self.putcode_i(state, Code::Indn, ctx.count() as i32);
                        }
                    }
                    ScrollDown => {
                        self.cursor_pane(state, ctx, ctx.ocx, ctx.orupper);
                        if self.term().has(Code::Rin) {
                            self.putcode_i(state, Code::Rin, ctx.count() as i32);
                        } else {
                            for _ in 0..ctx.count() {
                                self.putcode(Code::Ri);
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
            ClearEndOfScreen | ClearStartOfScreen | ClearScreen => {
                self.default_attributes(state, ctx.bg, Some(&ctx.style_ctx));
                self.region_pane(state, ctx, 0, ctx.sy - 1);
                self.margin_off(state);
                match cmd {
                    ClearEndOfScreen => {
                        self.clear_pane_area(
                            state,
                            ctx,
                            ctx.ocy + 1,
                            ctx.sy - ctx.ocy - 1,
                            0,
                            ctx.sx,
                        );
                        self.clear_pane_line(state, ctx, ctx.ocy, ctx.ocx, ctx.sx - ctx.ocx);
                    }
                    ClearStartOfScreen => {
                        self.clear_pane_area(state, ctx, 0, ctx.ocy, 0, ctx.sx);
                        self.clear_pane_line(state, ctx, ctx.ocy, 0, ctx.ocx + 1);
                    }
                    _ => self.clear_pane_area(state, ctx, 0, ctx.sy, 0, ctx.sx),
                }
            }
            AlignmentTest => {
                if ctx.bigger() {
                    return Some(TtyRedraw {
                        start_y: 0,
                        count: ctx.sy,
                    });
                }
                self.attributes(state, &DEFAULT_CELL, Some(&ctx.style_ctx));
                self.region_pane(state, ctx, 0, ctx.sy - 1);
                self.margin_off(state);
                for y in 0..ctx.sy {
                    self.cursor_pane(state, ctx, 0, y);
                    for _ in 0..ctx.sx {
                        self.putc(state, b'E');
                    }
                }
            }
            Cell => {
                if !is_visible(ctx, ctx.ocx, ctx.ocy, 1, 1) {
                    return None;
                }
                if position(ctx.xoff, ctx.ocx, ctx.wox) > self.sx.wrapping_sub(1)
                    && ctx.ocy == ctx.orlower
                    && ctx.full_width(self)
                {
                    self.region_pane(state, ctx, ctx.orupper, ctx.orlower);
                }
                self.margin_off(state);
                if ctx.flags.contains(TtyCtxFlags::CELL_INVALIDATE) {
                    self.invalidate(state);
                }
                self.cursor_pane_unless_wrap(state, ctx);
                self.cell(state, ctx.cell, Some(&ctx.style_ctx));
                if ctx.flags.contains(TtyCtxFlags::CELL_INVALIDATE) {
                    self.invalidate(state);
                }
            }
            Cells => {
                let bytes = ctx.bytes();
                let n = bytes.len() as u32;
                if !is_visible(ctx, ctx.ocx, ctx.ocy, n, 1) {
                    return None;
                }
                let x = position(ctx.xoff, ctx.ocx, 0);
                if ctx.bigger()
                    && (x < ctx.wox || x.wrapping_add(n) > ctx.wox.wrapping_add(ctx.wsx))
                {
                    if self.can_wrap(ctx, ctx.ocx, ctx.ocy) {
                        return Some(TtyRedraw {
                            start_y: ctx.ocy,
                            count: 1,
                        });
                    }
                    self.draw_pane(state, ctx, ctx.ocy);
                    return None;
                }
                self.margin_off(state);
                self.cursor_pane_unless_wrap(state, ctx);
                self.attributes(state, ctx.cell, Some(&ctx.style_ctx));
                self.putn(state, bytes, n);
            }
            RedrawLine => {
                if let Some(c) = clamp_line(ctx, ctx.ocx, ctx.ocy, ctx.count()) {
                    self.draw_line_styled(
                        state,
                        ctx.s,
                        ctx.ocx + c.skip,
                        ctx.ocy,
                        c.width,
                        c.x,
                        c.y,
                        &ctx.style_ctx,
                    );
                }
            }
            SetSelection => match ctx.data {
                TtyCommandData::Selection { clip, data } => self.set_selection(state, clip, data),
                _ => panic!("selection command without selection"),
            },
            RawString => {
                self.flags.insert(TtyFlags::NOBLOCK);
                self.add(ctx.bytes());
                self.invalidate(state);
            }
            SyncStart => {
                if ctx.flags.contains(TtyCtxFlags::SYNC) {
                    self.sync_start(state);
                }
            }
        }
        None
    }
}

impl Tty {
    pub fn sync_start(&mut self, state: &mut TparmState) {
        if self.flags.intersects(TtyFlags::BLOCK | TtyFlags::SYNCING) {
            return;
        }
        self.flags.insert(TtyFlags::SYNCING);
        self.sync_offset = self.out_len();
        if self.term().has(Code::Sync) {
            self.putcode_i(state, Code::Sync, 1);
        }
    }
    pub fn sync_end(&mut self, state: &mut TparmState) {
        if self.flags.contains(TtyFlags::BLOCK) || !self.flags.contains(TtyFlags::SYNCING) {
            return;
        }
        self.flags.remove(TtyFlags::SYNCING);
        if self.term().has(Code::Sync) {
            self.putcode_i(state, Code::Sync, 2);
        }
    }
}
