// Ported from tmux tty.c @ 8f25579c
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
use super::{Tty, TtyFlags, TtyTimer};
use crate::draw::TtyStyleCtx;
use crate::term::tparm::TparmState;
use crate::term::{TtyCodeCode as C, TtyTermFlags as F};
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes as A, GridCell, GridCellFlags};
use rmux_emu::colour::{ClientTheme, Colour, ColourFlags, ColourPalette, find_rgb, indexed_to_16};
use rmux_emu::hyperlinks::HyperlinkId;
use rmux_emu::screen::{ProgressBar, Screen, ScreenCursorStyle as S, ScreenMode as M};
use std::time::Duration;

fn tagged(c: Colour, flag: ColourFlags) -> bool {
    c.0 & flag.bits() as i32 != 0
}
fn palette_colour(palette: Option<&ColourPalette>, c: Colour) -> Colour {
    palette.and_then(|p| p.get(c)).unwrap_or(c)
}
impl Tty {
    pub(crate) fn map_theme_colour(&self, colour: Colour) -> Colour {
        if !tagged(colour, ColourFlags::THEME) {
            return colour;
        }
        let n = (colour.0 & 255) as usize;
        let mapped = self.host.theme_colours.get(n).copied().unwrap_or(-1);
        if mapped == -1 || mapped & ColourFlags::THEME.bits() as i32 != 0 {
            Colour::DEFAULT
        } else {
            Colour(mapped)
        }
    }
    fn dim_default_colour(&self, c: Colour, foreground: bool) -> Colour {
        if !c.is_default() {
            return c;
        }
        let reported = if foreground { self.fg } else { self.bg };
        if reported != -1 {
            return Colour(reported);
        }
        match self.host.theme {
            ClientTheme::Dark => Colour(if foreground { 7 } else { 0 }),
            ClientTheme::Light => Colour(if foreground { 0 } else { 7 }),
            ClientTheme::Unknown => c,
        }
    }
    pub fn attributes(
        &mut self,
        state: &mut TparmState,
        gc: &GridCell,
        style: Option<&TtyStyleCtx<'_>>,
    ) {
        let defaults = style.map_or(&DEFAULT_CELL, |s| s.defaults);
        let palette = style.and_then(|s| s.palette);
        let dim = style.map_or(0, |s| s.dim);
        let mut adjusted = *gc;
        if !gc.flags.contains(GridCellFlags::NOPALETTE) {
            if adjusted.fg == Colour::DEFAULT {
                adjusted.fg = defaults.fg;
            }
            if adjusted.bg == Colour::DEFAULT {
                adjusted.bg = defaults.bg;
            }
            adjusted.fg = palette_colour(palette, adjusted.fg);
            adjusted.bg = palette_colour(palette, adjusted.bg);
        }
        adjusted.fg = self.map_theme_colour(adjusted.fg);
        adjusted.bg = self.map_theme_colour(adjusted.bg);
        adjusted.us = self.map_theme_colour(adjusted.us);
        if dim != 0 {
            adjusted.fg = self.dim_default_colour(adjusted.fg, true);
            adjusted.bg = self.dim_default_colour(adjusted.bg, false);
            adjusted.fg = adjusted.fg.dim(dim).unwrap_or(adjusted.fg);
            adjusted.bg = adjusted.bg.dim(dim).unwrap_or(adjusted.bg);
        }
        if adjusted.attr == self.last_cell.attr
            && adjusted.fg == self.last_cell.fg
            && adjusted.bg == self.last_cell.bg
            && adjusted.us == self.last_cell.us
            && adjusted.link == self.last_cell.link
        {
            return;
        }
        if !self.term().has(C::Setab) {
            if adjusted.attr.contains(A::REVERSE) {
                if adjusted.fg != Colour(7) && !adjusted.fg.is_default() {
                    adjusted.attr.remove(A::REVERSE);
                }
            } else if adjusted.bg != Colour(0) && !adjusted.bg.is_default() {
                adjusted.attr.insert(A::REVERSE);
            }
        }
        self.check_fg(palette, &mut adjusted);
        self.check_bg(palette, &mut adjusted);
        self.check_us(palette, &mut adjusted);
        if (self.cell.attr & !adjusted.attr).bits() != 0
            || self.cell.us != adjusted.us && adjusted.us == Colour(0)
        {
            self.reset(state);
        }
        self.colours(state, &adjusted);
        let changed = adjusted.attr & !self.cell.attr;
        self.cell.attr = adjusted.attr;
        if changed.contains(A::BRIGHT) {
            self.putcode(C::Bold);
        }
        if changed.contains(A::DIM) {
            self.putcode(C::Dim);
        }
        if changed.contains(A::ITALICS) {
            let name = self.opts.default_terminal.cstr();
            if self.term().has(C::Sitm) && name != b"screen" && !name.starts_with(b"screen-") {
                self.putcode(C::Sitm);
            } else {
                self.putcode(C::Smso);
            }
        }
        if changed.intersects(A::ALL_UNDERSCORE) {
            if changed.contains(A::UNDERSCORE) {
                self.putcode(C::Smul);
            } else {
                for (attr, value) in [
                    (A::UNDERSCORE_2, 2),
                    (A::UNDERSCORE_3, 3),
                    (A::UNDERSCORE_4, 4),
                    (A::UNDERSCORE_5, 5),
                ] {
                    if changed.contains(attr) {
                        self.putcode_i(state, C::Smulx, value);
                        break;
                    }
                }
            }
        }
        if changed.contains(A::BLINK) {
            self.putcode(C::Blink);
        }
        if changed.contains(A::REVERSE) {
            if self.term().has(C::Rev) {
                self.putcode(C::Rev);
            } else if self.term().has(C::Smso) {
                self.putcode(C::Smso);
            }
        }
        if changed.contains(A::HIDDEN) {
            self.putcode(C::Invis);
        }
        if changed.contains(A::STRIKETHROUGH) {
            self.putcode(C::Smxx);
        }
        if changed.contains(A::OVERLINE) {
            self.putcode(C::Smol);
        }
        if changed.contains(A::CHARSET) && crate::acs::acs_needed(self.term(), self.host.utf8) {
            self.putcode(C::Smacs);
        }
        if gc.link != self.cell.link {
            self.cell.link = gc.link;
            if let Some((registry, links)) = style.and_then(|s| s.hyperlinks) {
                if let Some(link) = registry
                    .get(links, gc.link)
                    .filter(|_| gc.link != HyperlinkId::NONE)
                {
                    self.putcode_ss(state, C::Hls, link.external_id(), link.uri());
                } else {
                    self.putcode_ss(state, C::Hls, b"", b"");
                }
            }
        }
        self.last_cell = adjusted;
    }
    pub fn default_attributes(
        &mut self,
        state: &mut TparmState,
        bg: u32,
        style: Option<&TtyStyleCtx<'_>>,
    ) {
        let gc = GridCell {
            bg: Colour(bg as i32),
            ..DEFAULT_CELL
        };
        self.attributes(state, &gc, style);
    }
    pub(crate) fn check_fg(&self, palette: Option<&ColourPalette>, gc: &mut GridCell) {
        if !gc.flags.contains(GridCellFlags::NOPALETTE) {
            let mut c = gc.fg;
            if c.0 < 8 && gc.attr.contains(A::BRIGHT) && !self.term().has(C::Nobr) {
                c.0 += 90;
            }
            if let Some(c) = palette.and_then(|p| p.get(c)) {
                gc.fg = c;
            }
        }
        gc.fg = self.map_theme_colour(gc.fg);
        if tagged(gc.fg, ColourFlags::RGB) {
            if self.term().flags().contains(F::RGBCOLOURS) {
                return;
            }
            let (r, g, b) = gc.fg.split_rgb();
            gc.fg = find_rgb(r, g, b);
        }
        let colours = self.colour_count();
        if tagged(gc.fg, ColourFlags::_256) {
            if colours >= 256 {
                return;
            }
            gc.fg = Colour(i32::from(indexed_to_16(gc.fg)));
            if gc.fg.0 & 8 == 0 {
                return;
            }
            gc.fg.0 &= 7;
            if colours >= 16 {
                gc.fg.0 += 90;
            } else if gc.fg == Colour(0) && gc.bg == Colour(0) {
                gc.fg = Colour(7);
            } else if gc.fg == Colour(7) && gc.bg == Colour(7) {
                gc.fg = Colour(0);
            }
            return;
        }
        if (90..=97).contains(&gc.fg.0) && colours < 16 {
            gc.fg.0 -= 90;
            gc.attr.insert(A::BRIGHT);
        }
    }
    pub(crate) fn check_bg(&self, palette: Option<&ColourPalette>, gc: &mut GridCell) {
        if !gc.flags.contains(GridCellFlags::NOPALETTE) {
            gc.bg = palette_colour(palette, gc.bg);
        }
        gc.bg = self.map_theme_colour(gc.bg);
        if tagged(gc.bg, ColourFlags::RGB) {
            if self.term().flags().contains(F::RGBCOLOURS) {
                return;
            }
            let (r, g, b) = gc.bg.split_rgb();
            gc.bg = find_rgb(r, g, b);
        }
        let colours = self.colour_count();
        if tagged(gc.bg, ColourFlags::_256) {
            if colours >= 256 {
                return;
            }
            gc.bg = Colour(i32::from(indexed_to_16(gc.bg)));
            if gc.bg.0 & 8 == 0 {
                return;
            }
            gc.bg.0 &= 7;
            if colours >= 16 {
                gc.bg.0 += 90;
            }
            return;
        }
        if (90..=97).contains(&gc.bg.0) && colours < 16 {
            gc.bg.0 -= 90;
        }
    }
    pub(crate) fn check_us(&self, palette: Option<&ColourPalette>, gc: &mut GridCell) {
        if !gc.flags.contains(GridCellFlags::NOPALETTE) {
            gc.us = palette_colour(palette, gc.us);
        }
        gc.us = self.map_theme_colour(gc.us);
        if !self.term().has(C::Setulc1) {
            gc.us = gc.us.force_rgb().unwrap_or(Colour::DEFAULT);
        }
    }
    fn colour_count(&self) -> u32 {
        if self.term().flags().contains(F::_256COLOURS) {
            256
        } else {
            self.term().number(C::Colors) as u32
        }
    }
    fn colours(&mut self, state: &mut TparmState, gc: &GridCell) {
        if gc.fg == self.cell.fg && gc.bg == self.cell.bg && gc.us == self.cell.us {
            return;
        }
        if gc.fg.is_default() || gc.bg.is_default() {
            if !self.term().flag(C::Ax) {
                self.reset(state);
            } else {
                if gc.fg.is_default() && !self.cell.fg.is_default() {
                    self.puts(b"\x1b[39m");
                    self.cell.fg = gc.fg;
                }
                if gc.bg.is_default() && !self.cell.bg.is_default() {
                    self.puts(b"\x1b[49m");
                    self.cell.bg = gc.bg;
                }
            }
        }
        if !gc.fg.is_default() && gc.fg != self.cell.fg {
            self.colours_fg(state, gc);
        }
        if !gc.bg.is_default() && gc.bg != self.cell.bg {
            self.colours_bg(state, gc);
        }
        if gc.us != self.cell.us {
            self.colours_us(state, gc);
        }
    }
    fn colours_fg(&mut self, state: &mut TparmState, gc: &GridCell) {
        // Pinned C deliberately compares the old background to 97.
        if self.cell.fg.0 >= 90 && self.cell.bg.0 <= 97 && !(90..=97).contains(&gc.fg.0) {
            self.reset(state);
        }
        if tagged(gc.fg, ColourFlags::RGB) || tagged(gc.fg, ColourFlags::_256) {
            self.try_colour(state, gc.fg, true);
        } else if (90..=97).contains(&gc.fg.0) {
            if self.term().flags().contains(F::_256COLOURS) {
                self.add(&[0x1b, b'[', b'9', b'0' + (gc.fg.0 - 90) as u8, b'm']);
            } else {
                self.putcode_i(state, C::Setaf, gc.fg.0 - 90 + 8);
            }
        } else {
            self.putcode_i(state, C::Setaf, gc.fg.0);
        }
        self.cell.fg = gc.fg;
    }
    fn colours_bg(&mut self, state: &mut TparmState, gc: &GridCell) {
        if tagged(gc.bg, ColourFlags::RGB) || tagged(gc.bg, ColourFlags::_256) {
            self.try_colour(state, gc.bg, false);
        } else if (90..=97).contains(&gc.bg.0) {
            if self.term().flags().contains(F::_256COLOURS) {
                self.add(&[0x1b, b'[', b'1', b'0', b'0' + (gc.bg.0 - 90) as u8, b'm']);
            } else {
                self.putcode_i(state, C::Setab, gc.bg.0 - 90 + 8);
            }
        } else {
            self.putcode_i(state, C::Setab, gc.bg.0);
        }
        self.cell.bg = gc.bg;
    }
    fn colours_us(&mut self, state: &mut TparmState, gc: &GridCell) {
        if gc.us.is_default() {
            self.putcode(C::Ol);
        } else if !tagged(gc.us, ColourFlags::RGB) {
            let mut c = gc.us.0;
            if !tagged(gc.us, ColourFlags::_256) && (90..=97).contains(&c) {
                c -= 82;
            }
            self.putcode_i(state, C::Setulc1, c & !(ColourFlags::_256.bits() as i32));
            return; // Pinned C leaves the underline cache unchanged on this path.
        } else {
            let c = gc.us.0 & 0xffffff;
            if self.term().has(C::Setulc) {
                self.putcode_i(state, C::Setulc, c);
            } else if self.term().has(C::Setal) && self.term().has(C::Rgb) {
                self.putcode_i(state, C::Setal, c);
            }
        }
        self.cell.us = gc.us;
    }
    fn try_colour(&mut self, state: &mut TparmState, c: Colour, foreground: bool) {
        if tagged(c, ColourFlags::_256) {
            if foreground && self.term().has(C::Setaf) {
                self.putcode_i(state, C::Setaf, c.0 & 255);
            } else if self.term().has(C::Setab) {
                self.putcode_i(state, C::Setab, c.0 & 255);
            }
        } else if tagged(c, ColourFlags::RGB) {
            let (r, g, b) = c.split_rgb();
            if foreground && self.term().has(C::Setrgbf) {
                self.putcode_iii(state, C::Setrgbf, i32::from(r), i32::from(g), i32::from(b));
            } else if self.term().has(C::Setrgbb) {
                self.putcode_iii(state, C::Setrgbb, i32::from(r), i32::from(g), i32::from(b));
            }
        }
    }
    pub(crate) fn force_cursor_colour(&mut self, state: &mut TparmState, c: i32) {
        let c = if c == -1 {
            -1
        } else {
            self.map_theme_colour(Colour(c))
                .force_rgb()
                .map_or(-1, |c| c.0)
        };
        if c == self.ccolour {
            return;
        }
        if c == -1 {
            self.putcode(C::Cr);
        } else {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let (r, g, b) = Colour(c).split_rgb();
            let mut text = *b"rgb:00/00/00";
            for (offset, byte) in [(4, r), (7, g), (10, b)] {
                text[offset] = HEX[(byte >> 4) as usize];
                text[offset + 1] = HEX[(byte & 15) as usize];
            }
            self.putcode_s(state, C::Cs, &text);
        }
        self.ccolour = c;
    }
    fn update_cursor(&mut self, state: &mut TparmState, mut mode: M, screen: Option<&Screen>) -> M {
        if let Some(s) = screen {
            self.force_cursor_colour(
                state,
                if s.ccolour == Colour::NONE {
                    s.default_ccolour.0
                } else {
                    s.ccolour.0
                },
            );
        }
        if !mode.contains(M::CURSOR) {
            if self.mode.contains(M::CURSOR) {
                self.putcode(C::Civis);
            }
            return mode;
        }
        let style = screen.map_or(self.cstyle, |s| {
            if s.cstyle == S::Default {
                if !mode.contains(M::CURSOR_BLINKING_SET) {
                    if s.default_mode.contains(M::CURSOR_BLINKING) {
                        mode.insert(M::CURSOR_BLINKING);
                    } else {
                        mode.remove(M::CURSOR_BLINKING);
                    }
                }
                s.default_cstyle
            } else {
                s.cstyle
            }
        });
        if (mode.bits() ^ self.mode.bits()) & M::CURSOR_MODES.bits() == 0 && style == self.cstyle {
            return mode;
        }
        self.putcode(C::Cnorm);
        if style == S::Default {
            if self.cstyle != S::Default {
                if self.term().has(C::Se) {
                    self.putcode(C::Se);
                } else {
                    self.putcode_i(state, C::Ss, 0);
                }
            }
            if mode.intersects(M::CURSOR_BLINKING | M::CURSOR_VERY_VISIBLE) {
                self.putcode(C::Cvvis);
            }
        } else if self.term().has(C::Ss) {
            let base = match style {
                S::Block => 1,
                S::Underline => 3,
                S::Bar => 5,
                S::Default => unreachable!(),
            };
            self.putcode_i(
                state,
                C::Ss,
                base + i32::from(!mode.contains(M::CURSOR_BLINKING)),
            );
        } else if mode.contains(M::CURSOR_BLINKING) {
            self.putcode(C::Cvvis);
        }
        self.cstyle = style;
        mode
    }
    pub fn update_mode(&mut self, state: &mut TparmState, mut mode: M, screen: Option<&Screen>) {
        if self.flags.contains(TtyFlags::NOCURSOR) {
            mode.remove(M::CURSOR);
        }
        if self
            .update_cursor(state, mode, screen)
            .contains(M::CURSOR_BLINKING)
        {
            mode.insert(M::CURSOR_BLINKING);
        } else {
            mode.remove(M::CURSOR_BLINKING);
        }
        if (mode.bits() ^ self.mode.bits()) & M::ALL_MOUSE_MODES.bits() != 0
            && self.term().has(C::Kmous)
        {
            self.puts(b"\x1b[?1006l\x1b[?1000l\x1b[?1002l\x1b[?1003l");
            if mode.intersects(M::ALL_MOUSE_MODES) {
                self.puts(b"\x1b[?1006h");
            }
            if mode.contains(M::MOUSE_ALL) {
                self.puts(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h");
            } else if mode.contains(M::MOUSE_BUTTON) {
                self.puts(b"\x1b[?1000h\x1b[?1002h");
            } else if mode.contains(M::MOUSE_STANDARD) {
                self.puts(b"\x1b[?1000h");
            }
        }
        self.mode = mode;
    }
    pub fn set_title(&mut self, title: &[u8]) {
        if !self.term().has(C::Tsl) || !self.term().has(C::Fsl) {
            return;
        }
        self.putcode(C::Tsl);
        self.puts(title);
        self.putcode(C::Fsl);
    }
    pub fn set_path(&mut self, path: &[u8]) {
        if !self.term().has(C::Swd) || !self.term().has(C::Fsl) {
            return;
        }
        self.putcode(C::Swd);
        self.puts(path);
        self.putcode(C::Fsl);
    }
    pub fn set_progress_bar(&mut self, state: &mut TparmState, bar: &ProgressBar) {
        if self.term().has(C::Spb) {
            self.putcode_ii(state, C::Spb, bar.state as i32, bar.progress);
        }
    }
    pub fn set_selection(&mut self, state: &mut TparmState, clip: &str, data: &[u8]) {
        if !self.flags.contains(TtyFlags::STARTED) || !self.term().has(C::Ms) {
            return;
        }
        let encoded = rmux_util::base64::ntop(data);
        self.flags.insert(TtyFlags::NOBLOCK);
        self.putcode_ss(state, C::Ms, clip.as_bytes(), encoded.as_bytes());
    }
    pub fn clipboard_query(&mut self, state: &mut TparmState) {
        if self.flags.contains(TtyFlags::STARTED) && !self.flags.contains(TtyFlags::OSC52QUERY) {
            self.putcode_ss(state, C::Ms, b"", b"?");
            self.flags.insert(TtyFlags::OSC52QUERY);
            self.timer(TtyTimer::Clipboard, Some(Duration::from_secs(5)));
        }
    }
}
