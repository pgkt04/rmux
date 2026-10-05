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
use super::{Tty, TtyFlags, queue_bytes};
use crate::term::tparm::TparmState;
use crate::term::{TtyCodeCode as C, TtyTermFlags as F};
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes};
use rmux_emu::hyperlinks::HyperlinkId;
use rmux_emu::screen::ScreenMode;
use rmux_util::bytes::cstr;

impl Tty {
    pub(crate) fn add(&mut self, bytes: &[u8]) {
        queue_bytes(
            &mut self.out,
            self.flags,
            &mut self.discarded,
            &mut self.effects,
            &mut self.write_pending,
            bytes,
        );
    }
    pub fn puts(&mut self, bytes: &[u8]) {
        let bytes = cstr(bytes);
        if !bytes.is_empty() {
            self.add(bytes);
        }
    }
    pub fn putcode(&mut self, code: C) {
        let bytes = cstr(
            self.term
                .as_ref()
                .expect("capability requires open tty")
                .string(code),
        );
        if !bytes.is_empty() {
            queue_bytes(
                &mut self.out,
                self.flags,
                &mut self.discarded,
                &mut self.effects,
                &mut self.write_pending,
                bytes,
            );
        }
    }
    fn add_scratch(&mut self) {
        let bytes = cstr(&self.scratch);
        if !bytes.is_empty() {
            queue_bytes(
                &mut self.out,
                self.flags,
                &mut self.discarded,
                &mut self.effects,
                &mut self.write_pending,
                bytes,
            );
        }
    }
    pub fn putcode_i(&mut self, state: &mut TparmState, code: C, a: i32) {
        if a < 0 {
            return;
        }
        self.term
            .as_ref()
            .expect("capability requires open tty")
            .string_i(state, code, a, &mut self.scratch);
        self.add_scratch();
    }
    pub fn putcode_ii(&mut self, state: &mut TparmState, code: C, a: i32, b: i32) {
        if a < 0 || b < 0 {
            return;
        }
        self.term
            .as_ref()
            .expect("capability requires open tty")
            .string_ii(state, code, a, b, &mut self.scratch);
        self.add_scratch();
    }
    pub fn putcode_iii(&mut self, state: &mut TparmState, code: C, a: i32, b: i32, c: i32) {
        if a < 0 || b < 0 || c < 0 {
            return;
        }
        self.term
            .as_ref()
            .expect("capability requires open tty")
            .string_iii(state, code, a, b, c, &mut self.scratch);
        self.add_scratch();
    }
    pub fn putcode_s(&mut self, state: &mut TparmState, code: C, a: &[u8]) {
        self.term
            .as_ref()
            .expect("capability requires open tty")
            .string_s(state, code, cstr(a), &mut self.scratch);
        self.add_scratch();
    }
    pub fn putcode_ss(&mut self, state: &mut TparmState, code: C, a: &[u8], b: &[u8]) {
        self.term
            .as_ref()
            .expect("capability requires open tty")
            .string_ss(state, code, cstr(a), cstr(b), &mut self.scratch);
        self.add_scratch();
    }
    pub fn putc(&mut self, state: &mut TparmState, ch: u8) {
        let printable = ch >= 0x20 && ch != 0x7f;
        let noam = self.term().flags().contains(F::NOAM);
        if noam
            && printable
            && self.cy == self.sy.wrapping_sub(1)
            && self.cx.wrapping_add(1) >= self.sx
        {
            return;
        }
        if self.cell.attr.contains(GridAttributes::CHARSET) {
            if let Some(bytes) = crate::acs::acs_get(
                self.term.as_ref().expect("character requires term"),
                self.host.utf8,
                ch,
            ) {
                queue_bytes(
                    &mut self.out,
                    self.flags,
                    &mut self.discarded,
                    &mut self.effects,
                    &mut self.write_pending,
                    cstr(bytes),
                );
            } else {
                self.add(&[ch]);
            }
        } else {
            self.add(&[ch]);
        }
        if printable {
            if self.cx >= self.sx {
                self.cx = 1;
                if self.cy != self.rlower {
                    self.cy = self.cy.wrapping_add(1);
                }
                if noam {
                    self.putcode_ii(state, C::Cup, self.cy as i32, self.cx as i32);
                }
            } else {
                self.cx = self.cx.wrapping_add(1);
            }
        }
    }
    pub fn putn(&mut self, _state: &mut TparmState, bytes: &[u8], width: u32) {
        let mut len = bytes.len();
        if self.term().flags().contains(F::NOAM)
            && self.cy == self.sy.wrapping_sub(1)
            && (self.cx as usize).wrapping_add(len) >= self.sx as usize
        {
            len = self.sx.wrapping_sub(self.cx).wrapping_sub(1) as usize;
        }
        assert!(len <= bytes.len(), "NOAM truncation exceeded input length");
        self.add(&bytes[..len]);
        let end = self.cx.wrapping_add(width);
        if end > self.sx {
            self.cx = end.wrapping_sub(self.sx);
            if self.cx <= self.sx {
                self.cy = self.cy.wrapping_add(1);
            } else {
                self.cx = u32::MAX;
                self.cy = u32::MAX;
            }
        } else {
            self.cx = end;
        }
    }
    pub fn repeat_space(&mut self, state: &mut TparmState, mut n: u32) {
        const SPACES: [u8; 500] = [b' '; 500];
        while n > 500 {
            self.putn(state, &SPACES, 500);
            n -= 500;
        }
        if n != 0 {
            self.putn(state, &SPACES[..n as usize], n);
        }
    }
    pub fn reset(&mut self, state: &mut TparmState) {
        if !self.cell.cells_equal(&DEFAULT_CELL) {
            if self.cell.link != HyperlinkId::NONE {
                self.putcode_ss(state, C::Hls, b"", b"");
            }
            if self.cell.attr.contains(GridAttributes::CHARSET)
                && crate::acs::acs_needed(self.term(), self.host.utf8)
            {
                self.putcode(C::Rmacs);
            }
            self.putcode(C::Sgr0);
            self.cell = DEFAULT_CELL;
        }
        self.last_cell = DEFAULT_CELL;
    }
    pub fn invalidate(&mut self, state: &mut TparmState) {
        self.cell = DEFAULT_CELL;
        self.last_cell = DEFAULT_CELL;
        self.cx = u32::MAX;
        self.cy = u32::MAX;
        self.rupper = u32::MAX;
        self.rlower = u32::MAX;
        self.rleft = u32::MAX;
        self.rright = u32::MAX;
        if self.flags.contains(TtyFlags::STARTED) {
            if self.term().flags().contains(F::DECSLRM) {
                self.putcode(C::Enmg);
            }
            self.putcode(C::Sgr0);
            self.mode = ScreenMode::ALL_MODES;
            self.update_mode(state, ScreenMode::CURSOR, None);
            self.cursor(state, 0, 0);
            self.region_off(state);
            self.margin_off(state);
        } else {
            self.mode = ScreenMode::CURSOR;
        }
    }
    pub fn region_off(&mut self, state: &mut TparmState) {
        self.region(state, 0, self.sy.wrapping_sub(1));
    }
    pub(crate) fn region(&mut self, state: &mut TparmState, upper: u32, lower: u32) {
        if self.rlower == lower && self.rupper == upper || !self.term().has(C::Csr) {
            return;
        }
        self.rupper = upper;
        self.rlower = lower;
        if self.cx >= self.sx {
            self.cursor(state, 0, if self.cy == u32::MAX { 0 } else { self.cy });
        }
        self.putcode_ii(state, C::Csr, upper as i32, lower as i32);
        self.cx = u32::MAX;
        self.cy = u32::MAX;
    }
    pub fn margin_off(&mut self, state: &mut TparmState) {
        self.margin(state, 0, self.sx.wrapping_sub(1));
    }
    pub(crate) fn margin(&mut self, state: &mut TparmState, left: u32, right: u32) {
        if !self.term().flags().contains(F::DECSLRM) || self.rleft == left && self.rright == right {
            return;
        }
        self.putcode_ii(state, C::Csr, self.rupper as i32, self.rlower as i32);
        self.rleft = left;
        self.rright = right;
        if left == 0 && right == self.sx.wrapping_sub(1) {
            self.putcode(C::Clmg);
        } else {
            self.putcode_ii(state, C::Cmg, left as i32, right as i32);
        }
        self.cx = u32::MAX;
        self.cy = u32::MAX;
    }
    pub fn cursor(&mut self, state: &mut TparmState, mut cx: u32, cy: u32) {
        if self.flags.contains(TtyFlags::BLOCK) {
            return;
        }
        let (thisx, thisy) = (self.cx, self.cy);
        if cx == thisx && cy == thisy && cx == self.sx {
            return;
        }
        cx = cx.min(self.sx.wrapping_sub(1));
        if cx == thisx && cy == thisy {
            return;
        }
        let margin = self.term().flags().contains(F::DECSLRM);
        let moved = 'movement: {
            if thisx > self.sx.wrapping_sub(1) {
                break 'movement false;
            }
            if cx == 0 && cy == 0 && self.term().has(C::Home) {
                self.putcode(C::Home);
                break 'movement true;
            }
            if cx == 0
                && cy == thisy.wrapping_add(1)
                && thisy != self.rlower
                && (!margin || self.rleft == 0)
            {
                self.putc(state, b'\r');
                self.putc(state, b'\n');
                break 'movement true;
            }
            if cy == thisy {
                if cx == 0 && (!margin || self.rleft == 0) {
                    self.putc(state, b'\r');
                    break 'movement true;
                }
                if cx == thisx.wrapping_sub(1) && self.term().has(C::Cub1) {
                    self.putcode(C::Cub1);
                    break 'movement true;
                }
                if cx == thisx.wrapping_add(1) && self.term().has(C::Cuf1) {
                    self.putcode(C::Cuf1);
                    break 'movement true;
                }
                let change = thisx.wrapping_sub(cx) as i32;
                if change.unsigned_abs() > cx && self.term().has(C::Hpa) {
                    self.putcode_i(state, C::Hpa, cx as i32);
                    break 'movement true;
                } else if change > 0 && self.term().has(C::Cub) && !margin {
                    if change == 2 && self.term().has(C::Cub1) {
                        self.putcode(C::Cub1);
                        self.putcode(C::Cub1);
                    } else {
                        self.putcode_i(state, C::Cub, change);
                    }
                    break 'movement true;
                } else if change < 0 && self.term().has(C::Cuf) && !margin {
                    self.putcode_i(state, C::Cuf, change.wrapping_neg());
                    break 'movement true;
                }
            } else if cx == thisx {
                if thisy != self.rupper && cy == thisy.wrapping_sub(1) && self.term().has(C::Cuu1) {
                    self.putcode(C::Cuu1);
                    break 'movement true;
                }
                if thisy != self.rlower && cy == thisy.wrapping_add(1) && self.term().has(C::Cud1) {
                    self.putcode(C::Cud1);
                    break 'movement true;
                }
                let change = thisy.wrapping_sub(cy) as i32;
                if change.unsigned_abs() > cy
                    || change < 0 && cy.wrapping_sub(change as u32) > self.rlower
                    || change > 0 && cy.wrapping_sub(change as u32) < self.rupper
                {
                    if self.term().has(C::Vpa) {
                        self.putcode_i(state, C::Vpa, cy as i32);
                        break 'movement true;
                    }
                } else if change > 0 && self.term().has(C::Cuu) {
                    self.putcode_i(state, C::Cuu, change);
                    break 'movement true;
                } else if change < 0 && self.term().has(C::Cud) {
                    self.putcode_i(state, C::Cud, change.wrapping_neg());
                    break 'movement true;
                }
            }
            false
        };
        if !moved {
            self.putcode_ii(state, C::Cup, cy as i32, cx as i32);
        }
        self.cx = cx;
        self.cy = cy;
    }
}
