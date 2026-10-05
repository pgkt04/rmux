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

//! DCS dispatch: DECRQSS, SIXEL, and passthrough (`input.c:2540-2682`).

use super::{Env, FLAG_DISCARD, Flow, InputCtx, Passthrough};
use crate::screen::{ScreenCursorStyle, ScreenMode};

impl InputCtx {
    /// `input_handle_decrqss` (`input.c:2552-2614`).
    fn decrqss(&mut self, env: &mut Env<'_, '_>) -> Flow {
        let buf = &self.string;
        if buf.len() < 3 || buf[1] != b' ' || buf[2] != b'q' {
            return self.reply(format_args!("\x1bP0$r\x1b\\"));
        }
        let s = &env.sw.screen;
        let blinking = s.mode.contains(ScreenMode::CURSOR_BLINKING);
        let ps = match s.cstyle {
            ScreenCursorStyle::Block => {
                if blinking {
                    1
                } else {
                    2
                }
            }
            ScreenCursorStyle::Underline => {
                if blinking {
                    3
                } else {
                    4
                }
            }
            ScreenCursorStyle::Bar => {
                if blinking {
                    5
                } else {
                    6
                }
            }
            ScreenCursorStyle::Default => {
                let opt = env.policy.cursor_style;
                if !(0..=6).contains(&opt) { 0 } else { opt }
            }
        };
        self.reply(format_args!("\x1bP1$r q{ps} q\x1b\\"))
    }

    /// `input_dcs_dispatch` (`input.c:2618-2682`).
    pub(super) fn dcs_dispatch(&mut self, env: &mut Env<'_, '_>) -> Flow {
        if self.flags & FLAG_DISCARD != 0 {
            return Flow::Done;
        }
        #[cfg(feature = "sixel")]
        if env.policy.has_pane
            && self.string.first() == Some(&b'q')
            && self.interm.as_slice().is_empty()
        {
            if self.params.split(self.param_buf.as_slice()).is_err() {
                return Flow::Done;
            }
            let p2 = self.params.get(1, 0, 0).max(0) as u32;
            let (xpixel, ypixel) = env.policy.pixels.unwrap_or((16, 32));
            let xpixel =
                std::num::NonZeroU32::new(xpixel).unwrap_or(std::num::NonZeroU32::new(16).unwrap());
            let ypixel =
                std::num::NonZeroU32::new(ypixel).unwrap_or(std::num::NonZeroU32::new(32).unwrap());
            if let Ok(image) = crate::image::SixelImage::parse(&self.string, p2, xpixel, ypixel) {
                env.sw.sixelimage(image, self.cell.cell.bg);
            }
        }
        if self.interm.as_slice() == b"$" && self.string.first() == Some(&b'q') {
            return self.decrqss(env);
        }
        let all = match env.policy.allow_passthrough {
            Passthrough::Off => return Flow::Done,
            Passthrough::On => false,
            Passthrough::All => true,
        };
        if let Some(data) = self.string.strip_prefix(b"tmux;") {
            env.sw.rawstring(data, all);
        }
        Flow::Done
    }
}
