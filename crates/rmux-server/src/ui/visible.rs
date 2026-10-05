// Ported from tmux window-visible.c @ 8f25579c
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

use crate::ids::PaneId;
use crate::model::Server;
use crate::model::pane::{
    pane_get_pane_lines, pane_is_floating, pane_is_visible, pane_scrollbar_reserve,
};
use crate::ui::scrollbar::PaneScrollbarPosition;
use rmux_emu::screen::PaneLines;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VisibleRange {
    pub px: u32,
    pub nx: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VisibleRanges {
    pub ranges: Vec<VisibleRange>,
}

impl VisibleRanges {
    pub fn is_empty(&self) -> bool {
        self.ranges.iter().all(|r| r.nx == 0)
    }

    /// Remove width cells starting at px from the visible ranges.
    pub fn remove(&mut self, px: i32, width: u32) {
        if width == 0 {
            return;
        }
        let right = px + width as i32;
        let mut i = 0;
        while i < self.ranges.len() {
            let ri = self.ranges[i];
            let start = ri.px as i32;
            let end = start + ri.nx as i32;
            if ri.nx == 0 || right <= start || px >= end {
                i += 1;
                continue;
            }
            if px <= start {
                let np = if right < end { right } else { end };
                self.ranges[i].px = np as u32;
                self.ranges[i].nx = (end - np) as u32;
            } else {
                self.ranges[i].nx = (px - start) as u32;
                if right < end {
                    self.ranges.insert(
                        i + 1,
                        VisibleRange {
                            px: right as u32,
                            nx: (end - right) as u32,
                        },
                    );
                    i += 1;
                }
            }
            i += 1;
        }
    }
}

/// Check if a single character is within a visible range.
pub fn window_position_is_visible(r: Option<&VisibleRanges>, px: u32) -> bool {
    let Some(r) = r else {
        return true;
    };
    r.ranges
        .iter()
        .any(|ri| ri.nx != 0 && px >= ri.px && px < ri.px + ri.nx)
}

/// Geometry of one pane that may obscure a base pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObscuringPane {
    pub xoff: i32,
    pub yoff: i32,
    pub sx: u32,
    pub sy: u32,
    pub floating: bool,
    pub visible: bool,
    pub no_border: bool,
    pub sb_w: i32,
    pub sb_reserve: bool,
}

/// Static window geometry needed to compute visible ranges without a live
/// server borrow.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VisibilityModel {
    pub sx: u32,
    pub sy: u32,
    pub sb_pos: Option<PaneScrollbarPosition>,
    /// Menu rectangle (x, y, width, height).
    pub menu: Option<(u32, u32, u32, u32)>,
    /// Panes above the base pane in z order (front first).
    pub above: Vec<ObscuringPane>,
}

impl VisibilityModel {
    pub fn capture(srv: &Server, base: PaneId) -> Option<VisibilityModel> {
        let bp = srv.panes.get(base)?;
        let w = srv.windows.get(bp.window)?;
        let menu = w
            .menu
            .as_ref()
            .map(|md| (md.x(), md.y(), md.width(), md.height()));
        let mut above = Vec::new();
        for wp in &w.z_order {
            if *wp == base {
                break;
            }
            let Some(p) = srv.panes.get(*wp) else {
                continue;
            };
            let floating = pane_is_floating(srv, *wp);
            let lines = PaneLines::try_from(pane_get_pane_lines(srv, *wp) as i32)
                .unwrap_or(PaneLines::Single);
            above.push(ObscuringPane {
                xoff: p.xoff,
                yoff: p.yoff,
                sx: p.sx,
                sy: p.sy,
                floating,
                visible: pane_is_visible(srv, *wp),
                no_border: floating && lines == PaneLines::None,
                sb_w: p.scrollbar_style.width + p.scrollbar_style.pad,
                sb_reserve: pane_scrollbar_reserve(srv, *wp),
            });
        }
        Some(VisibilityModel {
            sx: w.sx,
            sy: w.sy,
            sb_pos: Some(w.sb_pos),
            menu,
            above,
        })
    }

    /// Construct ranges for the line starting at px,py of width cells of the
    /// base pane that are unobstructed. All ranges are in window coordinates.
    pub fn visible_ranges(&self, px: i32, py: i32, width: u32, r: &mut VisibleRanges) {
        if !normalize(px, py, width, r) {
            return;
        }
        let (px, mut width) = normalized(px, width);
        if py as u32 >= self.sy || px as u32 >= self.sx {
            r.ranges.clear();
            return;
        }
        if px as u32 + width > self.sx {
            width = self.sx - px as u32;
        }
        if r.ranges.is_empty() {
            r.ranges.push(VisibleRange {
                px: px as u32,
                nx: width,
            });
        }
        if let Some((mx, my, mw, mh)) = self.menu {
            if py as u32 >= my && py as u32 - my < mh {
                r.remove(mx as i32, mw);
            }
        }
        for wp in &self.above {
            let (tb, bb) = if wp.no_border {
                (wp.yoff, wp.yoff + wp.sy as i32 - 1)
            } else {
                (
                    if wp.yoff > 0 { wp.yoff - 1 } else { 0 },
                    wp.yoff + wp.sy as i32,
                )
            };
            if !wp.visible || py < tb || py > bb {
                continue;
            }
            if !wp.floating && (py == tb || py == bb) {
                continue;
            }
            let (sb_w, sb_pos) = if wp.sb_reserve {
                (wp.sb_w, self.sb_pos)
            } else {
                (0, None)
            };
            let left_sb = sb_pos == Some(PaneScrollbarPosition::Left);
            let mut lb;
            let mut rb = 0;
            if wp.no_border {
                lb = wp.xoff;
                rb = wp.xoff + wp.sx as i32 - 1;
            } else if left_sb {
                lb = if wp.xoff > sb_w {
                    wp.xoff - 1 - sb_w
                } else {
                    0
                };
            } else {
                lb = if wp.xoff > 0 { wp.xoff - 1 } else { 0 };
            }
            if !wp.no_border {
                rb = if left_sb {
                    wp.xoff + wp.sx as i32
                } else {
                    wp.xoff + wp.sx as i32 + sb_w
                };
            }
            if lb < 0 {
                lb = 0;
            }
            if rb < 0 {
                continue;
            }
            // Borderless panes may use the last column; bordered ones may
            // not go past it.
            let limit = if wp.no_border {
                self.sx as i32 - 1
            } else {
                self.sx as i32
            };
            if rb > limit {
                rb = self.sx as i32 - 1;
            }
            if lb <= rb {
                r.remove(lb, (rb - lb + 1) as u32);
            }
        }
    }
}

/// Returns false (after clearing `r`) when the request is empty.
fn normalize(px: i32, py: i32, width: u32, r: &mut VisibleRanges) -> bool {
    if py < 0 || width == 0 {
        r.ranges.clear();
        return false;
    }
    if px < 0 && (-px) as u32 >= width {
        r.ranges.clear();
        return false;
    }
    true
}

fn normalized(px: i32, width: u32) -> (i32, u32) {
    if px < 0 {
        (0, width - (-px) as u32)
    } else {
        (px, width)
    }
}

/// window_visible_ranges. `initialize = true` models a null C ranges pointer:
/// `out` is reset to the whole normalized range before clipping. With
/// `initialize = false`, the supplied ranges are refined in place.
pub fn window_visible_ranges(
    srv: &Server,
    base: Option<PaneId>,
    px: i32,
    py: i32,
    width: u32,
    out: &mut VisibleRanges,
    initialize: bool,
) {
    if initialize {
        out.ranges.clear();
    }
    if !normalize(px, py, width, out) {
        return;
    }
    let (px, width) = normalized(px, width);
    let Some(base) = base else {
        if initialize {
            out.ranges.push(VisibleRange {
                px: px as u32,
                nx: width,
            });
        }
        return;
    };
    let Some(model) = VisibilityModel::capture(srv, base) else {
        out.ranges.clear();
        return;
    };
    model.visible_ranges(px, py, width, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(v: &[(u32, u32)]) -> VisibleRanges {
        VisibleRanges {
            ranges: v.iter().map(|&(px, nx)| VisibleRange { px, nx }).collect(),
        }
    }

    #[test]
    fn remove_split_left_right_full() {
        let mut a = r(&[(0, 10)]);
        a.remove(3, 2);
        assert_eq!(a, r(&[(0, 3), (5, 5)]));
        let mut b = r(&[(0, 10)]);
        b.remove(-2, 5);
        assert_eq!(b, r(&[(3, 7)]));
        let mut c = r(&[(0, 10)]);
        c.remove(7, 10);
        assert_eq!(c, r(&[(0, 7)]));
        let mut d = r(&[(2, 5)]);
        d.remove(0, 20);
        assert_eq!(d, r(&[(7, 0)]));
        assert!(d.is_empty());
        let mut e = r(&[(0, 10)]);
        e.remove(3, 0);
        assert_eq!(e, r(&[(0, 10)]));
    }

    #[test]
    fn position_is_visible() {
        assert!(window_position_is_visible(None, 99));
        let a = r(&[(0, 3), (5, 0), (6, 2)]);
        assert!(window_position_is_visible(Some(&a), 2));
        assert!(!window_position_is_visible(Some(&a), 3));
        assert!(!window_position_is_visible(Some(&a), 5));
        assert!(window_position_is_visible(Some(&a), 7));
        assert!(!window_position_is_visible(Some(&a), 8));
    }

    fn model(above: Vec<ObscuringPane>) -> VisibilityModel {
        VisibilityModel {
            sx: 80,
            sy: 24,
            sb_pos: Some(PaneScrollbarPosition::Right),
            menu: None,
            above,
        }
    }

    fn float(xoff: i32, yoff: i32, sx: u32, sy: u32, no_border: bool) -> ObscuringPane {
        ObscuringPane {
            xoff,
            yoff,
            sx,
            sy,
            floating: true,
            visible: true,
            no_border,
            sb_w: 1,
            sb_reserve: false,
        }
    }

    #[test]
    fn negative_and_zero_inputs() {
        let m = model(vec![]);
        let mut out = r(&[(1, 1)]);
        m.visible_ranges(0, -1, 5, &mut out);
        assert!(out.ranges.is_empty());
        let mut out = r(&[(1, 1)]);
        m.visible_ranges(0, 0, 0, &mut out);
        assert!(out.ranges.is_empty());
        let mut out = VisibleRanges::default();
        m.visible_ranges(-3, 0, 10, &mut out);
        assert_eq!(out, r(&[(0, 7)]));
        let mut out = VisibleRanges::default();
        m.visible_ranges(-10, 0, 10, &mut out);
        assert!(out.ranges.is_empty());
        let mut out = VisibleRanges::default();
        m.visible_ranges(75, 0, 10, &mut out);
        assert_eq!(out, r(&[(75, 5)]));
    }

    #[test]
    fn bordered_versus_borderless_edges() {
        // Bordered float at x=10..19, y=5..9: border columns 9 and 20 are
        // covered, plus the reserved scrollbar when present.
        let m = model(vec![float(10, 5, 10, 5, false)]);
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 7, 80, &mut out);
        assert_eq!(out, r(&[(0, 9), (21, 59)]));
        // Rows py == tb are covered too for floating panes.
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 4, 80, &mut out);
        assert_eq!(out, r(&[(0, 9), (21, 59)]));
        // Borderless covers only its own cells.
        let m = model(vec![float(10, 5, 10, 5, true)]);
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 7, 80, &mut out);
        assert_eq!(out, r(&[(0, 10), (20, 60)]));
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 4, 80, &mut out);
        assert_eq!(out, r(&[(0, 80)]));
    }

    #[test]
    fn reserved_scrollbar_and_tiled_pane() {
        let mut p = float(10, 5, 10, 5, false);
        p.sb_reserve = true;
        p.sb_w = 2;
        let m = model(vec![p]);
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 7, 80, &mut out);
        assert_eq!(out, r(&[(0, 9), (23, 57)]));
        let mut m = model(vec![p]);
        m.sb_pos = Some(PaneScrollbarPosition::Left);
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 7, 80, &mut out);
        assert_eq!(out, r(&[(0, 7), (21, 59)]));
        // A tiled pane above does not cover its own border rows.
        let mut t = float(10, 5, 10, 5, false);
        t.floating = false;
        let m = model(vec![t]);
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 4, 80, &mut out);
        assert_eq!(out, r(&[(0, 80)]));
        let mut out = VisibleRanges::default();
        m.visible_ranges(0, 10, 80, &mut out);
        assert_eq!(out, r(&[(0, 80)]));
    }

    #[test]
    fn supplied_ranges_are_refined_and_zero_length_kept() {
        let m = model(vec![float(10, 5, 10, 5, true)]);
        let mut out = r(&[(0, 15), (30, 0), (40, 10)]);
        m.visible_ranges(0, 7, 80, &mut out);
        assert_eq!(out, r(&[(0, 10), (30, 0), (40, 10)]));
        let mut m2 = m.clone();
        m2.menu = Some((2, 6, 4, 3));
        let mut out = r(&[(0, 15)]);
        m2.visible_ranges(0, 7, 80, &mut out);
        assert_eq!(out, r(&[(0, 2), (6, 4)]));
    }
}
