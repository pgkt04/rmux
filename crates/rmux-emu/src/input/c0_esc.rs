// Ported from tmux input.c @ 8f25579c
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

//! C0 and ESC dispatch (`input.c:1302-1459`).

use super::{Env, FLAG_DISCARD, FLAG_LAST, Flow, InputCtx, Pending, Sub};
use crate::screen::{ScreenMode, ScreenResetPolicy};
use rmux_util::utf8::UTF8_SIZE;

/// `enum input_esc_type` (`input.c:220-236`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Esc {
    Decaln,
    Deckpam,
    Deckpnm,
    Decrc,
    Decsc,
    Hts,
    Ind,
    Nel,
    Ri,
    Ris,
    Scsg0Off,
    Scsg0On,
    Scsg1Off,
    Scsg1On,
    St,
}

/// `input_esc_table` (`input.c:239-255`), sorted by final byte then
/// intermediates for the binary search.
const ESC_TABLE: [(u8, &[u8], Esc); 15] = [
    (b'0', b"(", Esc::Scsg0On),
    (b'0', b")", Esc::Scsg1On),
    (b'7', b"", Esc::Decsc),
    (b'8', b"", Esc::Decrc),
    (b'8', b"#", Esc::Decaln),
    (b'=', b"", Esc::Deckpam),
    (b'>', b"", Esc::Deckpnm),
    (b'B', b"(", Esc::Scsg0Off),
    (b'B', b")", Esc::Scsg1Off),
    (b'D', b"", Esc::Ind),
    (b'E', b"", Esc::Nel),
    (b'H', b"", Esc::Hts),
    (b'M', b"", Esc::Ri),
    (b'\\', b"", Esc::St),
    (b'c', b"", Esc::Ris),
];

/// `input_table_compare` (`input.c:770-778`) as a binary search over
/// `(final byte, intermediates)`.
pub(crate) fn table_lookup<T: Copy>(table: &[(u8, &[u8], T)], ch: u8, interm: &[u8]) -> Option<T> {
    table
        .binary_search_by(|(c, i, _)| c.cmp(&ch).then_with(|| (*i).cmp(interm)))
        .ok()
        .map(|idx| table[idx].2)
}

impl InputCtx {
    /// `input_c0_dispatch` (`input.c:1303-1383`).
    pub(super) fn c0_dispatch(&mut self, env: &mut Env<'_, '_>) -> Flow {
        self.stop_utf8(env);
        let mut flow = Flow::Done;
        let s = &mut *env.sw;
        match self.ch {
            0x00 => {}
            0x07 => {
                if env.policy.has_pane {
                    flow = Flow::Yield(Pending::Bell, super::Sub::Done);
                }
            }
            0x08 => s.backspace(),
            0x09 => {
                let sx = s.screen.grid.sx();
                let mut cx = s.screen.cx;
                if cx >= sx - 1 {
                    self.flags &= !FLAG_LAST;
                    return flow;
                }
                let line = s.screen.cy + s.screen.grid.hsize();
                let first = s.screen.grid.get_cell(cx, line);
                let mut has_content = false;
                loop {
                    if !has_content {
                        let gc = s.screen.grid.get_cell(cx, line);
                        if gc.data.size != 1 || gc.data.data[0] != b' ' || !gc.look_equal(&first) {
                            has_content = true;
                        }
                    }
                    cx += 1;
                    if s.screen.tabs[cx as usize] {
                        break;
                    }
                    if cx >= sx - 1 {
                        break;
                    }
                }
                let width = cx - s.screen.cx;
                if has_content || width as usize > UTF8_SIZE {
                    s.screen.cx = cx;
                } else {
                    let mut gc = s.screen.grid.get_cell(s.screen.cx, line);
                    gc.set_tab(width);
                    s.collect_add(&gc);
                }
            }
            0x0a..=0x0c => {
                s.linefeed(false, self.cell.cell.bg);
                if s.screen.mode.contains(ScreenMode::CRLF) {
                    s.carriagereturn();
                }
            }
            0x0d => s.carriagereturn(),
            0x0e => self.cell.set = 1,
            0x0f => self.cell.set = 0,
            _ => {}
        }
        self.flags &= !FLAG_LAST;
        flow
    }

    /// `input_esc_dispatch` (`input.c:1387-1459`).
    pub(super) fn esc_dispatch(&mut self, env: &mut Env<'_, '_>) -> Flow {
        if self.flags & FLAG_DISCARD != 0 {
            return Flow::Done;
        }
        let Some(entry) = table_lookup(&ESC_TABLE, self.ch, self.interm.as_slice()) else {
            return Flow::Done;
        };
        match entry {
            Esc::Ris => {
                if let Some(palette) = env.palette.as_deref_mut() {
                    palette.clear_runtime();
                }
                self.reset_cell();
                env.sw.screen.clear_surface_anchors();
                env.sw.reset(ScreenResetPolicy {
                    extended_keys: env.policy.reset_extended_keys,
                });
                env.sw.fullredraw();
                self.flags &= !FLAG_LAST;
                return Flow::Yield(Pending::TerminalReset, Sub::Done);
            }
            Esc::Ind => env.sw.linefeed(false, self.cell.cell.bg),
            Esc::Nel => {
                env.sw.carriagereturn();
                env.sw.linefeed(false, self.cell.cell.bg);
            }
            Esc::Hts => {
                let s = &mut *env.sw.screen;
                if s.cx < s.grid.sx() {
                    s.tabs[s.cx as usize] = true;
                }
            }
            Esc::Ri => env.sw.reverseindex(self.cell.cell.bg),
            Esc::Deckpam => env.sw.mode_set(ScreenMode::KKEYPAD),
            Esc::Deckpnm => env.sw.mode_clear(ScreenMode::KKEYPAD),
            Esc::Decsc => self.save_state(env),
            Esc::Decrc => self.restore_state(env),
            Esc::Decaln => env.sw.alignmenttest(),
            Esc::Scsg0On => self.cell.g0set = true,
            Esc::Scsg0Off => self.cell.g0set = false,
            Esc::Scsg1On => self.cell.g1set = true,
            Esc::Scsg1Off => self.cell.g1set = false,
            Esc::St => {}
        }
        self.flags &= !FLAG_LAST;
        Flow::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_table_is_sorted() {
        for pair in ESC_TABLE.windows(2) {
            assert!((pair[0].0, pair[0].1) < (pair[1].0, pair[1].1));
        }
        assert_eq!(table_lookup(&ESC_TABLE, b'8', b"#"), Some(Esc::Decaln));
        assert_eq!(table_lookup(&ESC_TABLE, b'8', b""), Some(Esc::Decrc));
        assert_eq!(table_lookup(&ESC_TABLE, b'8', b"$"), None);
    }
}
