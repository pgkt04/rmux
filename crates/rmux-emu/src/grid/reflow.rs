// Ported from tmux grid.c @ 8f25579c
/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
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
//! Reflow to a new width and the wrap-position conversions
//! (`grid.c:1316-1665`). The target is a private line list created with the
//! OLD width, so `expand_line` rounding during reflow follows `grid.c:1539`.

use super::{Grid, GridLine, GridLineFlags, move_cells_in_line, set_cell_in_line};
use crate::colour::Colour;

struct ReflowTarget {
    sx: u32,
    lines: Vec<GridLine>,
}

impl ReflowTarget {
    /// `grid_reflow_add`: returns the index of the first new line
    /// (`grid.c:1325-1336`).
    fn add(&mut self, n: usize) -> usize {
        let first = self.lines.len();
        self.lines.resize_with(first + n, GridLine::default);
        first
    }

    /// `grid_reflow_move` (`grid.c:1339-1348`): moves the buffers, leaves
    /// the source dead.
    fn mv(&mut self, from: &mut GridLine) -> usize {
        self.lines.push(std::mem::take(from));
        dead(from);
        self.lines.len() - 1
    }

    fn set_cell(&mut self, px: u32, py: usize, gc: &crate::cell::GridCell) {
        set_cell_in_line(self.sx, &mut self.lines[py], px, gc);
    }
}

/// `grid_reflow_dead` (`grid.c:1317-1322`).
fn dead(gl: &mut GridLine) {
    *gl = GridLine::default();
    gl.flags = GridLineFlags::DEAD;
}

/// `grid_reflow_join` (`grid.c:1351-1456`).
fn join(
    target: &mut ReflowTarget,
    src: &mut [GridLine],
    hscrolled: &mut u32,
    sx: u32,
    yy: usize,
    mut width: u32,
    already: bool,
) {
    let to = if already {
        target.lines.len() - 1
    } else {
        target.mv(&mut src[yy])
    };
    let mut at = target.lines[to].cellused();

    let mut lines = 0usize;
    let mut from: Option<usize> = None;
    let mut want = 0u32;
    let mut wrapped = true;
    loop {
        if yy + 1 + lines == src.len() {
            break;
        }
        let line = yy + 1 + lines;

        if !src[line].flags.intersects(GridLineFlags::WRAPPED) {
            wrapped = false;
        }
        if src[line].cellused() == 0 {
            if !wrapped {
                break;
            }
            lines += 1;
            continue;
        }

        let gc = src[line].get_cell1(0);
        if width + u32::from(gc.data.width) > sx {
            break;
        }
        width += u32::from(gc.data.width);
        target.set_cell(at, to, &gc);
        at += 1;

        from = Some(line);
        want = 1;
        while want < src[line].cellused() {
            let gc = src[line].get_cell1(want as usize);
            if width + u32::from(gc.data.width) > sx {
                break;
            }
            width += u32::from(gc.data.width);
            target.set_cell(at, to, &gc);
            at += 1;
            want += 1;
        }
        lines += 1;

        if !wrapped || want != src[line].cellused() || width == sx {
            break;
        }
    }
    let Some(from) = from else {
        return;
    };
    if lines == 0 {
        return;
    }

    let left = src[from].cellused() - want;
    if left != 0 {
        let fl = &mut src[yy + lines];
        move_cells_in_line(target.sx, fl, 0, want, left, Colour::DEFAULT);
        fl.cellsize = left as u16;
        fl.used = left as u16;
        lines -= 1;
    } else if !wrapped {
        target.lines[to].flags.remove(GridLineFlags::WRAPPED);
    }

    for gl in &mut src[yy + 1..yy + 1 + lines] {
        dead(gl);
    }

    let to = to as u32;
    let lines = lines as u32;
    if *hscrolled > to + lines {
        *hscrolled -= lines;
    } else if *hscrolled > to {
        *hscrolled = to;
    }
}

/// `grid_reflow_split` (`grid.c:1459-1524`).
fn split(
    target: &mut ReflowTarget,
    src: &mut [GridLine],
    hscrolled: &mut u32,
    sx: u32,
    yy: usize,
    at: u32,
) {
    let gl = &src[yy];
    let used = gl.cellused();
    let flags = gl.flags;

    let lines = if !gl.flags.intersects(GridLineFlags::EXTENDED) {
        1 + (used - 1) / sx
    } else {
        let mut lines = 2;
        let mut width = 0;
        for i in at..used {
            let gc = gl.get_cell1(i as usize);
            if width + u32::from(gc.data.width) > sx {
                lines += 1;
                width = 0;
            }
            width += u32::from(gc.data.width);
        }
        lines
    } as usize;

    let mut line = target.lines.len() + 1;
    let first = target.add(lines);

    let mut width = 0;
    let mut xx = 0;
    for i in at..used {
        let gc = src[yy].get_cell1(i as usize);
        if width + u32::from(gc.data.width) > sx {
            target.lines[line].flags.insert(GridLineFlags::WRAPPED);
            line += 1;
            width = 0;
            xx = 0;
        }
        width += u32::from(gc.data.width);
        target.set_cell(xx, line, &gc);
        xx += 1;
    }
    if flags.intersects(GridLineFlags::WRAPPED) {
        target.lines[line].flags.insert(GridLineFlags::WRAPPED);
    }

    let gl = &mut src[yy];
    gl.cellsize = at as u16;
    gl.used = at as u16;
    gl.flags.insert(GridLineFlags::WRAPPED);
    target.lines[first] = std::mem::take(gl);
    dead(gl);

    if yy as u32 <= *hscrolled {
        *hscrolled += lines as u32 - 1;
    }

    if width < sx && flags.intersects(GridLineFlags::WRAPPED) {
        join(target, src, hscrolled, sx, yy, width, true);
    }
}

impl Grid {
    /// `grid_reflow` (`grid.c:1527-1610`). Does not change `sx`; the caller
    /// (screen resize) owns the width.
    pub fn reflow(&mut self, sx: u32) {
        let mut src = std::mem::take(&mut self.lines);
        src.truncate(self.hsize.wrapping_add(self.sy) as usize);
        let mut target = ReflowTarget {
            sx: self.sx,
            lines: Vec::with_capacity(src.len()),
        };
        let mut hscrolled = self.hscrolled;

        for yy in 0..src.len() {
            let gl = &src[yy];
            if gl.flags.intersects(GridLineFlags::DEAD) {
                continue;
            }

            let mut at = 0;
            let mut width = 0;
            if !gl.flags.intersects(GridLineFlags::EXTENDED) {
                width = gl.cellused();
                at = if width > sx { sx } else { width };
            } else {
                for i in 0..gl.cellused() {
                    let gc = gl.get_cell1(i as usize);
                    if at == 0 && width + u32::from(gc.data.width) > sx {
                        at = i;
                    }
                    width += u32::from(gc.data.width);
                }
            }

            if width == sx {
                target.mv(&mut src[yy]);
                continue;
            }
            if width > sx {
                split(&mut target, &mut src, &mut hscrolled, sx, yy, at);
                continue;
            }
            if src[yy].flags.intersects(GridLineFlags::WRAPPED) {
                join(&mut target, &mut src, &mut hscrolled, sx, yy, width, false);
            } else {
                target.mv(&mut src[yy]);
            }
        }

        if target.lines.len() < self.sy as usize {
            let n = self.sy as usize - target.lines.len();
            target.add(n);
        }
        self.hsize = target.lines.len() as u32 - self.sy;
        self.hscrolled = hscrolled;
        if self.hscrolled > self.hsize {
            self.hscrolled = self.hsize;
        }
        self.lines = target.lines;
        self.scroll_generation = self.scroll_generation.wrapping_add(1);
    }

    /// `grid_wrap_position` (`grid.c:1613-1632`): `(wx, wy)` on the
    /// unwrapped line; `wx == u32::MAX` past the used cells.
    pub fn wrap_position(&self, px: u32, py: u32) -> (u32, u32) {
        let mut ax = 0u32;
        let mut ay = 0u32;
        for yy in 0..py {
            let gl = &self.lines[yy as usize];
            if gl.flags.intersects(GridLineFlags::WRAPPED) {
                ax += gl.cellused();
            } else {
                ax = 0;
                ay += 1;
            }
        }
        if px >= self.lines[py as usize].cellused() {
            ax = u32::MAX;
        } else {
            ax += px;
        }
        (ax, ay)
    }

    /// `grid_unwrap_position` (`grid.c:1635-1665`): `(px, py)` from a
    /// position that `wrap_position` produced in the same reflow transaction.
    pub fn unwrap_position(&self, wx: u32, wy: u32) -> (u32, u32) {
        let total = self.hsize.wrapping_add(self.sy);
        let ey = total - 1;
        let mut ay = 0;
        let mut yy = 0;
        while yy < total - 1 {
            if ay == wy {
                break;
            }
            if !self.lines[yy as usize]
                .flags
                .intersects(GridLineFlags::WRAPPED)
            {
                ay += 1;
            }
            yy += 1;
        }

        let mut wx = wx;
        if wx == u32::MAX {
            while yy < ey
                && self.lines[yy as usize]
                    .flags
                    .intersects(GridLineFlags::WRAPPED)
            {
                yy += 1;
            }
            wx = self.lines[yy as usize].cellused();
        } else {
            while self.lines[yy as usize]
                .flags
                .intersects(GridLineFlags::WRAPPED)
            {
                if wx < self.lines[yy as usize].cellused() {
                    break;
                }
                wx -= self.lines[yy as usize].cellused();
                yy += 1;
            }
        }
        (wx, yy)
    }
}
