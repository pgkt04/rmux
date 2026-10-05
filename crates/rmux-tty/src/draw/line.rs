// Ported from tmux tty-draw.c @ 8f25579c
/*
 * Copyright (c) 2026 Nicholas Marriott <nicholas.marriott@gmail.com>
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
use super::TtyStyleCtx;
use crate::term::{TtyCodeCode as Code, tparm::TparmState};
use crate::tty::{Tty, TtyFlags};
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell, GridCellFlags};
use rmux_emu::grid::GridLineFlags;
use rmux_emu::hyperlinks::{HyperlinkId, HyperlinkRegistry};
use rmux_emu::screen::Screen;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    First,
    Flush,
    New1,
    New2,
    Empty,
    Same,
    Done,
}

fn get_empty(gc: &GridCell, last: &GridCell, nx: u32) -> u32 {
    if u32::from(gc.data.width) > nx {
        nx
    } else if gc.flags.contains(GridCellFlags::PADDING) || gc.data.width == 0 {
        1
    } else if gc.flags.contains(GridCellFlags::SELECTED) {
        0
    } else if gc.bg == last.bg && gc.attr.bits() == 0 && gc.link == HyperlinkId::NONE {
        if gc.flags.contains(GridCellFlags::CLEARED) {
            1
        } else if gc.flags.contains(GridCellFlags::TAB) {
            u32::from(gc.data.width)
        } else if gc.data.size == 1 && gc.data.data[0] == b' ' {
            1
        } else {
            0
        }
    } else {
        0
    }
}

impl Tty {
    #[allow(clippy::too_many_arguments)]
    fn draw_line_clear(
        &mut self,
        state: &mut TparmState,
        px: u32,
        py: u32,
        nx: u32,
        defaults: &GridCell,
        bg: u32,
        wrapped: bool,
    ) {
        if nx == 0 {
            return;
        }
        if !wrapped && nx >= 10 && !self.fake_bce(defaults, bg) {
            if px + nx >= self.sx && self.term().has(Code::El) {
                self.cursor(state, px, py);
                self.putcode(Code::El);
                return;
            }
            if px == 0 && self.term().has(Code::El1) {
                self.cursor(state, px + nx - 1, py);
                self.putcode(Code::El1);
                return;
            }
            if self.term().has(Code::Ech) {
                self.cursor(state, px, py);
                self.putcode_i(state, Code::Ech, nx as i32);
                return;
            }
        }
        if px != 0 || !wrapped {
            self.cursor(state, px, py);
        }
        if nx == 1 {
            self.putc(state, b' ');
        } else if nx == 2 {
            self.putn(state, b"  ", 2);
        } else {
            self.repeat_space(state, nx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_line(
        &mut self,
        state: &mut TparmState,
        registry: &HyperlinkRegistry,
        s: &Screen,
        px: u32,
        py: u32,
        nx: u32,
        atx: u32,
        aty: u32,
        style: Option<&TtyStyleCtx<'_>>,
    ) {
        let default_style = TtyStyleCtx {
            hyperlinks: s.hyperlinks.as_ref().map(|store| (registry, store)),
            ..TtyStyleCtx::default()
        };
        self.draw_line_styled(
            state,
            s,
            px,
            py,
            nx,
            atx,
            aty,
            style.unwrap_or(&default_style),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_line_styled(
        &mut self,
        state: &mut TparmState,
        s: &Screen,
        mut px: u32,
        py: u32,
        mut nx: u32,
        mut atx: u32,
        aty: u32,
        style: &TtyStyleCtx<'_>,
    ) {
        let defaults = style.defaults;
        if atx >= self.sx {
            return;
        }
        if atx.wrapping_add(nx) >= self.sx {
            nx = self.sx - atx;
        }
        if nx == 0 {
            return;
        }
        let ex = s
            .grid
            .get_line(s.grid.hsize() + py)
            .cellsize()
            .min(s.grid.sx());
        let saved = self.flags & TtyFlags::NOCURSOR;
        self.flags.insert(TtyFlags::NOCURSOR);
        self.update_mode(state, self.mode, Some(s));
        self.region_off(state);
        self.margin_off(state);
        let mut last = DEFAULT_CELL;
        last.bg = defaults.bg;
        self.default_attributes(state, 8, Some(style));

        let mut padding = 0;
        for x in px..px + nx {
            if !s
                .grid
                .view_get_cell(x, py)
                .flags
                .contains(GridCellFlags::PADDING)
            {
                break;
            }
            padding += 1;
        }
        if padding != 0 {
            let mut bg = defaults.bg;
            for x in (0..=px).rev() {
                let gc = s.grid.view_get_cell(x, py);
                if !gc.flags.contains(GridCellFlags::PADDING) {
                    bg = if gc.flags.contains(GridCellFlags::SELECTED) {
                        s.select_cell(&gc).bg
                    } else {
                        gc.bg
                    };
                    break;
                }
            }
            self.attributes(state, &last, Some(style));
            self.draw_line_clear(state, atx, aty, padding, defaults, bg.0 as u32, false);
            if padding == ex {
                self.flags = (self.flags & !TtyFlags::NOCURSOR) | saved;
                self.update_mode(state, self.mode, Some(s));
                return;
            }
            atx += padding;
            px += padding;
            nx -= padding;
        }
        let mut wrapped = py != 0
            && atx == 0
            && self.cx >= self.sx
            && self.cy != u32::MAX
            && self.cy.wrapping_add(1) == aty
            && nx == self.sx
            && s.grid
                .get_line(s.grid.hsize() + py - 1)
                .flags
                .contains(GridLineFlags::WRAPPED);
        let mut i = 0;
        let mut last_i = 0;
        let mut len = 0;
        let mut width = 0;
        let mut buf = [0; 1000];
        let mut current = State::First;
        loop {
            let (gc, empty, next) = if i == nx {
                (DEFAULT_CELL, 0, State::Done)
            } else {
                assert!(i < nx, "position {i} exceeds width {nx}");
                let (gc, empty) = if px >= ex || i >= ex - px {
                    (DEFAULT_CELL, nx - i)
                } else {
                    let gc = s.grid.view_get_cell(px + i, py);
                    let empty = get_empty(&gc, &last, nx - i);
                    if empty != 0 {
                        (gc, empty)
                    } else {
                        let mut scratch = DEFAULT_CELL;
                        let gc = *self.check_codeset(&gc, &mut scratch);
                        let gc = if gc.flags.contains(GridCellFlags::SELECTED) {
                            s.select_cell(&gc)
                        } else {
                            gc
                        };
                        (gc, 0)
                    }
                };
                let next = if empty != 0 {
                    State::Empty
                } else if current == State::First {
                    State::Same
                } else if gc.look_equal(&last) {
                    if usize::from(gc.data.size) > buf.len() - len {
                        State::Flush
                    } else {
                        State::Same
                    }
                } else if current == State::New1 {
                    State::New2
                } else {
                    State::New1
                };
                (gc, empty, next)
            };
            if next != current {
                if current == State::Empty {
                    self.attributes(state, &last, Some(style));
                    self.draw_line_clear(
                        state,
                        atx + last_i,
                        aty,
                        i - last_i,
                        defaults,
                        last.bg.0 as u32,
                        wrapped,
                    );
                    wrapped = false;
                } else if next != State::Same && len != 0 {
                    self.attributes(state, &last, Some(style));
                    if atx + i - width != 0 || !wrapped {
                        self.cursor(state, atx + i - width, aty);
                    }
                    if !last.attr.contains(GridAttributes::CHARSET) {
                        self.putn(state, &buf[..len], width);
                    } else {
                        for ch in &buf[..len] {
                            self.putc(state, *ch);
                        }
                    }
                    len = 0;
                    width = 0;
                    wrapped = false;
                }
                last_i = i;
            }
            if next != State::Empty {
                let bytes = gc.data.bytes();
                buf[len..len + bytes.len()].copy_from_slice(bytes);
                len += bytes.len();
                width += u32::from(gc.data.width);
            }
            if next == State::Done {
                break;
            }
            current = next;
            last = gc;
            i += if empty != 0 {
                empty
            } else {
                u32::from(gc.data.width)
            };
        }
        self.flags = (self.flags & !TtyFlags::NOCURSOR) | saved;
        self.update_mode(state, self.mode, Some(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_order_padding_zero_width_and_selected() {
        let mut c = DEFAULT_CELL;
        c.flags.insert(GridCellFlags::SELECTED);
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 5), 0);
        c.flags.insert(GridCellFlags::PADDING);
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 5), 1);
        c.flags.remove(GridCellFlags::PADDING);
        c.data.width = 0;
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 5), 1);
        c.data.width = 2;
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 1), 1);
    }
    #[test]
    fn empty_tabs_background_attributes_and_links() {
        let mut c = DEFAULT_CELL;
        c.flags.insert(GridCellFlags::TAB);
        c.data.width = 7;
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 8), 7);
        c.bg = rmux_emu::colour::Colour(1);
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 8), 0);
        c.bg = DEFAULT_CELL.bg;
        c.attr = GridAttributes::BRIGHT;
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 8), 0);
        c.attr = GridAttributes(0);
        c.link = HyperlinkId(1);
        assert_eq!(get_empty(&c, &DEFAULT_CELL, 8), 0);
    }
}
