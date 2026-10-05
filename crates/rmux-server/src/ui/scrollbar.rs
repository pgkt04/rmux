// Ported from tmux tmux.h, screen-redraw.c (redraw_draw_scrollbar_span) @ 8f25579c
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
pub enum PaneScrollbarPolicy {
    Off = 0,
    Modal = 1,
    Always = 2,
    Autohide = 3,
}
impl TryFrom<i32> for PaneScrollbarPolicy {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Modal),
            2 => Ok(Self::Always),
            3 => Ok(Self::Autohide),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PaneScrollbarPosition {
    Right = 0,
    Left = 1,
}
impl TryFrom<i32> for PaneScrollbarPosition {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Right),
            1 => Ok(Self::Left),
            _ => Err(value),
        }
    }
}

use crate::model::Server;
use crate::ui::redraw::{RedrawSpan, RedrawSpanData, ScrollbarSpanFlags};
use rmux_tty::term::tparm::TparmState;
use rmux_tty::tty::Tty;

/// Slider geometry: (slider_y, slider_h). Uses f64 and truncates like C.
pub fn slider_geometry(sb_h: u32, total_height: u32, cm_y: Option<u32>) -> Option<(u32, u32)> {
    if total_height == 0 {
        return None;
    }
    let pct_view = sb_h as f64 / total_height as f64;
    let mut slider_h = (sb_h as f64 * pct_view) as u32;
    let mut slider_y = match cm_y {
        None => sb_h - slider_h,
        Some(cm_y) => ((sb_h + 1) as f64 * (cm_y as f64 / total_height as f64)) as u32,
    };
    if slider_h < 1 {
        slider_h = 1;
    }
    if slider_y >= sb_h {
        slider_y = sb_h - 1;
    }
    Some((slider_y, slider_h))
}

/// Draw a scrollbar span.
pub fn redraw_draw_scrollbar_span(
    srv: &mut Server,
    tty: &mut Tty,
    tparm: &mut TparmState,
    span: &RedrawSpan,
    x: u32,
    y: u32,
    n: u32,
) {
    let RedrawSpanData::Scrollbar {
        wp,
        y: sb_y,
        height: sb_h,
        flags,
    } = span.data
    else {
        return;
    };
    let Some(p) = srv.panes.get(wp) else {
        return;
    };
    let geometry = if p.modes.is_empty() {
        let s = &p.base;
        slider_geometry(sb_h, s.grid.sy() + s.grid.hsize(), None)
    } else {
        let mode = &p.modes[0];
        let Some((cm_y, cm_size)) = mode.driver.clone().current_offset(srv, mode.id) else {
            return;
        };
        slider_geometry(sb_h, cm_size + sb_h, Some(cm_y))
    };
    let Some((slider_y, slider_h)) = geometry else {
        return;
    };
    let (pad_gc, _) = crate::ui::fanout::tty_default_colours(srv, wp);
    let Some(p) = srv.panes.get_mut(wp) else {
        return;
    };
    p.sb_slider_y = slider_y;
    p.sb_slider_h = slider_h;

    let gc = p.scrollbar_style.gc;
    let mut slgc = gc;
    slgc.fg = gc.bg;
    slgc.bg = gc.fg;
    let sb_w = p.scrollbar_style.width.max(0) as u32;
    let sb_pad = p.scrollbar_style.pad.max(0) as u32;
    let off = x - span.x;

    tty.cursor(tparm, x, y);
    for i in 0..n {
        if flags.contains(ScrollbarSpanFlags::LEFT) {
            if off + i >= sb_w && off + i < sb_w + sb_pad {
                tty.cell(tparm, &pad_gc, None);
                continue;
            }
        } else if off + i < sb_pad {
            tty.cell(tparm, &pad_gc, None);
            continue;
        }
        let gcp = if sb_y >= slider_y && sb_y < slider_y + slider_h {
            &slgc
        } else {
            &gc
        };
        tty.cell(tparm, gcp, None);
    }
}

#[cfg(test)]
mod tests {
    use super::slider_geometry;

    #[test]
    fn slider_math_truncates_and_clamps() {
        // 24 rows visible of 24 total: full bar.
        assert_eq!(slider_geometry(24, 24, None), Some((0, 24)));
        // 24 of 100 total: 24*0.24 = 5.76 -> 5, y = 19.
        assert_eq!(slider_geometry(24, 100, None), Some((19, 5)));
        // Tiny slider clamps to 1 row; start clamps to sb_h - 1.
        assert_eq!(slider_geometry(10, 1000, None), Some((9, 1)));
        // Mode: total = cm_size + sb_h; slider_y = (sb_h+1) * cm_y/total.
        assert_eq!(slider_geometry(10, 90, Some(40)), Some((4, 1)));
        assert_eq!(slider_geometry(10, 20, Some(10)), Some((5, 5)));
        // Height not clamped to the remaining bar.
        assert_eq!(slider_geometry(10, 20, Some(19)), Some((9, 5)));
        assert_eq!(slider_geometry(10, 0, None), None);
    }
}
