// Ported from tmux window-copy.c @ 8f25579c
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
use super::state::{self, CopyModeData, ModeKeys, SearchDirection};
use crate::ids::ModeId;
use crate::model::{Server, pane::pane_scrollbar_show};
use rmux_emu::cell::{DEFAULT_CELL, GridCellFlags};
use rmux_emu::grid::{Grid, GridLineFlags};
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_sys::regex::{ExecFlags, PosixRegex, RegexFlags, RegexMatch};
use rmux_util::utf8::Utf8Data;
use std::time::Instant;

const MAX_LINE: u32 = 2000;
const ALL_TIMEOUT: u64 = 200;
const SEARCH_TIMEOUT: u64 = 10000;

fn cbytes(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]
}

pub fn is_lowercase(bytes: &[u8]) -> bool {
    cbytes(bytes)
        .iter()
        .all(|&b| b == rmux_sys::locale::to_lower(b))
}

fn effective_regex(term: &[u8], regex: bool) -> bool {
    regex && cbytes(term).iter().any(|b| b"^$*+()?[].\\".contains(b))
}

fn term_grid(term: &[u8]) -> Option<Grid> {
    let width = u32::try_from(ScreenWriteCtx::strlen(term)).ok()?;
    if width == 0 {
        return None;
    }
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(width, 1, 0, ScreenResetPolicy::default(), &mut registry).ok()?;
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.nputs(-1, &DEFAULT_CELL, term);
    ctx.finish();
    screen
        .release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .ok()?;
    Some(screen.grid)
}

// Fixed-size cell data keeps the C suffix conversion allocation-free per cell.
fn cellstring(gd: &Grid, x: u32, y: u32) -> Utf8Data {
    let gc = gd.get_cell(x, y);
    if gc.flags.contains(GridCellFlags::PADDING) {
        Utf8Data::default()
    } else if gc.flags.contains(GridCellFlags::TAB) {
        Utf8Data::set(b'\t')
    } else {
        gc.data
    }
}

fn stringify(gd: &Grid, y: u32, first: u32, last: u32, buf: &mut Vec<u8>) {
    if gd.peek_line(y).is_none() {
        return;
    }
    for x in first..last {
        let data = cellstring(gd, x, y);
        buf.extend_from_slice(data.bytes());
    }
}

fn coordinate(gd: &Grid, x: u32, y: u32, offset: u32) -> (u32, u32) {
    let x = x.wrapping_add(offset);
    (x % gd.sx(), y.wrapping_add(x / gd.sx()))
}

fn cstrtocellpos(gd: &Grid, ncells: u32, x: &mut u32, y: &mut u32, suffix: &[u8]) {
    if gd.peek_line(*y).is_none() || gd.sx() == 0 {
        return;
    }
    let suffix = cbytes(suffix);
    let available = (gd.hsize() + gd.sy() - *y)
        .saturating_mul(gd.sx())
        .saturating_sub(*x);
    let ncells = ncells.min(available);
    let mut cell = 0;
    while cell < ncells {
        let mut pos = 0;
        let mut matched = true;
        for ccell in cell..ncells {
            if pos == suffix.len() {
                matched = false;
                break;
            }
            let (px, py) = coordinate(gd, *x, *y, ccell);
            let data = cellstring(gd, px, py);
            let bytes = data.bytes();
            let len = if bytes.len() == 1 {
                1
            } else {
                bytes.len().min(suffix.len() - pos)
            };
            if suffix[pos..pos + len] != bytes[..len] {
                matched = false;
                break;
            }
            pos += len;
        }
        if matched {
            break;
        }
        cell += 1;
    }
    (*x, *y) = coordinate(gd, *x, *y, cell);
}

fn search_compare(gd: &Grid, x: u32, y: u32, needle: &Grid, nx: u32, cis: bool) -> bool {
    let gc = gd.get_cell(x, y);
    let ngc = needle.get_cell(nx, 0);
    if ngc.data.bytes() == b"\t" && gc.flags.contains(GridCellFlags::TAB) {
        return true;
    }
    if gc.data.size != ngc.data.size || gc.data.width != ngc.data.width {
        return false;
    }
    if cis && gc.data.size == 1 {
        rmux_sys::locale::to_lower(gc.data.data[0]) == ngc.data.data[0]
    } else {
        gc.data.bytes() == ngc.data.bytes()
    }
}

fn literal_at(gd: &Grid, needle: &Grid, ax: u32, y: u32, cis: bool) -> bool {
    let endline = gd.hsize() + gd.sy() - 1;
    let mut padding = 0u32;
    for bx in 0..needle.sx() {
        let mut x = ax.wrapping_add(bx).wrapping_add(padding);
        let mut py = y;
        while x >= gd.sx() && py < endline {
            if !gd.get_line(py).flags.contains(GridLineFlags::WRAPPED) {
                break;
            }
            x -= gd.sx();
            py += 1;
        }
        if x.wrapping_sub(padding) >= gd.sx() {
            return false;
        }
        let gc = gd.get_cell(x, py);
        if gc.flags.contains(GridCellFlags::TAB) {
            padding += u32::from(gc.data.width).saturating_sub(1);
        }
        if !search_compare(gd, x, py, needle, bx, cis) {
            return false;
        }
    }
    true
}

struct Matcher {
    needle: Grid,
    regex: Option<PosixRegex>,
    captures: RegexMatch,
    scratch: Vec<u8>,
    cis: bool,
}

#[derive(Debug)]
enum MatcherError {
    #[cfg(test)]
    Empty,
    Regex,
}

impl Matcher {
    #[cfg(test)]
    fn new(term: &[u8], regex: bool, scratch: Vec<u8>) -> Result<Self, MatcherError> {
        Self::with_needle(
            term_grid(term).ok_or(MatcherError::Empty)?,
            term,
            regex,
            scratch,
        )
    }

    fn with_needle(
        needle: Grid,
        term: &[u8],
        regex: bool,
        mut scratch: Vec<u8>,
    ) -> Result<Self, MatcherError> {
        let cis = is_lowercase(term);
        let regex = if regex {
            scratch.clear();
            stringify(&needle, 0, 0, needle.sx(), &mut scratch);
            let mut flags = RegexFlags::EXTENDED;
            if cis {
                flags |= RegexFlags::ICASE;
            }
            Some(PosixRegex::new(&scratch, flags).map_err(|_| MatcherError::Regex)?)
        } else {
            None
        };
        Ok(Self {
            needle,
            regex,
            captures: RegexMatch::new(1),
            scratch,
            cis,
        })
    }

    fn serialize(&mut self, gd: &Grid, y: u32, first: u32) -> u32 {
        self.scratch.clear();
        stringify(gd, y, first, gd.sx(), &mut self.scratch);
        let mut len = gd.sx() - first;
        let mut py = y;
        while py < gd.hsize() + gd.sy() - 1 && len < MAX_LINE {
            if !gd.get_line(py).flags.contains(GridLineFlags::WRAPPED) {
                break;
            }
            py += 1;
            stringify(gd, py, 0, gd.sx(), &mut self.scratch);
            len += gd.sx();
        }
        len
    }

    fn row(
        &mut self,
        gd: &Grid,
        y: u32,
        first: u32,
        last: u32,
        backward: bool,
    ) -> Option<(u32, u32)> {
        if self.regex.is_none() {
            if backward {
                for x in (first..last).rev() {
                    if literal_at(gd, &self.needle, x, y, self.cis) {
                        return Some((x, self.needle.sx()));
                    }
                }
            } else {
                for x in first..last {
                    if literal_at(gd, &self.needle, x, y, self.cis) {
                        return Some((x, self.needle.sx()));
                    }
                }
            }
            return None;
        }
        if first >= last || first >= gd.sx() {
            return None;
        }
        let mut len = self.serialize(gd, y, first);
        let flags = if first != 0 {
            ExecFlags::NOTBOL
        } else {
            ExecFlags::NONE
        };
        let reg = self.regex.as_ref()?;
        let mut offset = 0;
        let (mut foundx, mut foundy, mut oldx) = (first, y, first);
        let mut saved = None;
        loop {
            if !reg
                .exec(&self.scratch[offset..], &mut self.captures, flags)
                .ok()?
            {
                break;
            }
            let range = self.captures.whole()?;
            if range.is_empty() {
                break;
            }
            cstrtocellpos(
                gd,
                len,
                &mut foundx,
                &mut foundy,
                &self.scratch[offset + range.start..],
            );
            if foundy > y || foundx >= last {
                break;
            }
            len = len.wrapping_sub(foundx.wrapping_sub(oldx));
            let savepx = foundx;
            cstrtocellpos(
                gd,
                len,
                &mut foundx,
                &mut foundy,
                &self.scratch[offset + range.end..],
            );
            let width = foundx
                .wrapping_add((foundy - y) * gd.sx())
                .wrapping_sub(savepx);
            if !backward || foundy > y || foundx >= last {
                return Some((savepx, width));
            }
            saved = (width > 0).then_some((savepx, width));
            len = len.wrapping_sub(width);
            oldx = foundx;
            offset += range.end;
        }
        saved
    }

    fn back_overlap(&mut self, gd: &Grid, x: &mut u32, width: u32, y: &mut u32, endline: u32) {
        let oldend = coordinate(gd, *x, *y, width);
        let (mut px, mut py) = (*x, *y);
        while px == 0 && py > endline && gd.get_line(py - 1).flags.contains(GridLineFlags::WRAPPED)
        {
            py -= 1;
            let Some((foundx, foundwidth)) = self.row(gd, py, 0, gd.sx(), true) else {
                break;
            };
            px = foundx;
            if coordinate(gd, px, py, foundwidth) != oldend {
                break;
            }
            (*x, *y) = (px, py);
        }
    }

    fn jump(
        &mut self,
        gd: &Grid,
        x: u32,
        y: u32,
        direction: SearchDirection,
        wrap: bool,
    ) -> Option<(u32, u32)> {
        let down = direction == SearchDirection::Down;
        let bottom = gd.hsize() + gd.sy() - 1;
        for pass in 0..=u32::from(wrap) {
            let endline = if pass == 0 {
                if down { bottom } else { 0 }
            } else {
                y
            };
            let mut fy = if pass == 0 {
                y
            } else if down {
                0
            } else {
                bottom
            };
            let mut fx = if pass == 0 {
                x
            } else if down {
                0
            } else {
                gd.sx() - 1
            };
            loop {
                let last = if down { gd.sx() } else { fx.saturating_add(1) };
                let first = if down { fx } else { 0 };
                if let Some((mut px, width)) = self.row(gd, fy, first, last, !down) {
                    if !down && self.regex.is_some() {
                        self.back_overlap(gd, &mut px, width, &mut fy, endline);
                    }
                    return Some((px, fy));
                }
                if fy == endline {
                    break;
                }
                if down {
                    fy += 1;
                    fx = 0;
                } else {
                    fy -= 1;
                    fx = gd.sx() - 1;
                }
            }
        }
        None
    }
}

pub fn move_left(s: &Screen, x: &mut u32, y: &mut u32, wrap: bool) {
    if *x == 0 {
        if *y == 0 {
            if wrap {
                *x = s.grid.sx() - 1;
                *y = s.grid.hsize() + s.grid.sy() - 1;
            }
        } else {
            *x = s.grid.sx() - 1;
            *y -= 1;
        }
    } else {
        *x -= 1;
    }
}

pub fn move_right(s: &Screen, x: &mut u32, y: &mut u32, wrap: bool) {
    if *x == s.grid.sx() - 1 {
        if *y == s.grid.hsize() + s.grid.sy() - 1 {
            if wrap {
                *x = 0;
                *y = 0;
            }
        } else {
            *x = 0;
            *y += 1;
        }
    } else {
        *x += 1;
    }
}

pub fn mark_at(data: &CopyModeData, x: u32, y: u32) -> Option<usize> {
    let gd = &data.backing.screen().grid;
    let top = gd.hsize().checked_sub(data.oy)?;
    if y < top || y >= top + gd.sy() {
        return None;
    }
    let at = usize::try_from(y - top)
        .ok()?
        .checked_mul(gd.sx() as usize)?
        .checked_add(x as usize)?;
    let size = (gd.sx() as usize).checked_mul(gd.sy() as usize)?;
    (at < size).then_some(at)
}

pub fn match_start_end(data: &CopyModeData, at: usize) -> Option<(usize, usize)> {
    let marks = data.search.marks.as_ref()?;
    let mark = *marks.get(at)?;
    let (mut start, mut end) = (at, at);
    while start > 0 && marks[start - 1] == mark {
        start -= 1;
    }
    while end + 1 < marks.len() && marks[end + 1] == mark {
        end += 1;
    }
    Some((start, end))
}

pub fn match_at_cursor(data: &CopyModeData) -> Option<(u32, u32, u32, u32)> {
    let marks = data.search.marks.as_ref()?;
    let mut at = mark_at(data, data.cx, data.backing_y())?;
    if *marks.get(at)? == 0 {
        at = at.checked_sub(1)?;
        if *marks.get(at)? == 0 {
            return None;
        }
    }
    let (start, end) = match_start_end(data, at)?;
    let gd = &data.backing.screen().grid;
    let width = gd.sx() as usize;
    let top = gd.hsize() - data.oy;
    Some((
        (start % width) as u32,
        top + (start / width) as u32,
        (end % width) as u32,
        top + (end / width) as u32,
    ))
}

pub fn move_after_search_mark(data: &CopyModeData, x: &mut u32, y: &mut u32, wrap: bool) {
    let Some(marks) = data.search.marks.as_ref() else {
        return;
    };
    let Some(start) = mark_at(data, *x, *y) else {
        return;
    };
    let Some(&generation) = marks.get(start) else {
        return;
    };
    if generation == 0 {
        return;
    }
    let origin = (*x, *y);
    let s = data.backing.screen();
    while let Some(at) = mark_at(data, *x, *y) {
        if marks.get(at) != Some(&generation) {
            break;
        }
        if !wrap && *x == s.grid.sx() - 1 && *y == s.grid.hsize() + s.grid.sy() - 1 {
            break;
        }
        move_right(s, x, y, wrap);
        if (*x, *y) == origin {
            break;
        }
    }
}

fn visible_lines(data: &CopyModeData) -> (u32, u32) {
    let gd = &data.backing.screen().grid;
    let top = gd.hsize() - data.oy;
    let mut start = top;
    while start > 0
        && gd
            .get_line(start - 1)
            .flags
            .contains(GridLineFlags::WRAPPED)
    {
        start -= 1;
    }
    (start, top + gd.sy())
}

fn mark_match(data: &mut CopyModeData, x: u32, y: u32, width: u32, regex: bool) -> u32 {
    let Some(b) = mark_at(data, x, y) else {
        return width;
    };
    let gd = &data.backing.screen().grid;
    let Some(marks) = data.search.marks.as_mut() else {
        return width;
    };
    let mut w = width.min((marks.len() - b) as u32);
    let mut i = 0;
    while i < w {
        if !regex {
            let gc = gd.get_cell(x + i, y);
            if gc.flags.contains(GridCellFlags::TAB) {
                w = (w + u32::from(gc.data.width).saturating_sub(1)).min((marks.len() - b) as u32);
            }
        }
        if marks[b + i as usize] == 0 {
            marks[b + i as usize] = data.search.generation;
        }
        i += 1;
    }
    data.search.generation = if data.search.generation == u8::MAX {
        1
    } else {
        data.search.generation + 1
    };
    w
}

fn clear_marks_data(data: &mut CopyModeData) {
    data.search.count = -1;
    data.search.more = false;
    data.search.marks = None;
}

pub fn clear_marks(server: &mut Server, mode: ModeId) -> bool {
    let Some(data) = state::data_mut(server, mode) else {
        return false;
    };
    clear_marks_data(data);
    true
}

fn partial_count(count: u32) -> i32 {
    if count > 1000 {
        1000
    } else if count > 100 {
        100
    } else if count > 10 {
        10
    } else {
        -1
    }
}

fn marks_with_clock(
    data: &mut CopyModeData,
    matcher: &mut Matcher,
    visible_only: bool,
    mut now: impl FnMut() -> u64,
) -> bool {
    let (sx, sy, hsize) = {
        let gd = &data.backing.screen().grid;
        (gd.sx(), gd.sy(), gd.hsize())
    };
    let Some(size) = (sx as usize).checked_mul(sy as usize) else {
        return false;
    };
    let tstart = now();
    let (mut start, mut end) = if visible_only {
        visible_lines(data)
    } else {
        (0, hsize + sy)
    };
    let mut stop = (!visible_only).then(|| now().saturating_add(ALL_TIMEOUT));
    let mut stopped = false;
    let mut nfound = 0u32;
    loop {
        let marks = data.search.marks.get_or_insert_with(Vec::new);
        marks.clear();
        marks.resize(size, 0);
        data.search.generation = 1;
        for py in start..end {
            let mut px = 0;
            while let Some((x, mut width)) =
                matcher.row(&data.backing.screen().grid, py, px, sx, false)
            {
                px = x;
                if matcher.regex.is_some() {
                    let gc = data
                        .backing
                        .screen()
                        .grid
                        .get_cell(px.wrapping_add(width).wrapping_sub(1), py);
                    if gc.data.width > 2 {
                        width += u32::from(gc.data.width) - 1;
                    }
                }
                nfound = nfound.wrapping_add(1);
                px += mark_match(data, px, py, width, matcher.regex.is_some());
            }
            let t = now();
            if t.saturating_sub(tstart) > SEARCH_TIMEOUT {
                data.timeout = true;
                break;
            }
            if stop.is_some_and(|stop| t > stop) {
                stopped = true;
                break;
            }
        }
        if data.timeout {
            clear_marks_data(data);
            return true;
        }
        if stopped && stop.is_some() {
            (start, end) = visible_lines(data);
            stop = None;
            continue;
        }
        break;
    }
    if !visible_only {
        data.search.count = if stopped {
            partial_count(nfound)
        } else {
            nfound as i32
        };
        data.search.more = stopped;
    }
    true
}

fn search_marks_data(
    data: &mut CopyModeData,
    visible_only: bool,
    now: impl FnMut() -> u64,
) -> bool {
    let Some(term) = data.search.term.as_deref() else {
        return false;
    };
    let Some(needle) = term_grid(term) else {
        return false;
    };
    let scratch = std::mem::take(&mut data.search.scratch);
    let Ok(mut matcher) = Matcher::with_needle(needle, term, data.search.regex, scratch) else {
        // regcomp failure only removes the stale buffer, not its counters.
        data.search.marks = None;
        return false;
    };
    let result = marks_with_clock(data, &mut matcher, visible_only, now);
    data.search.scratch = matcher.scratch;
    result
}

pub fn search_marks(server: &mut Server, mode: ModeId, visible_only: bool) -> bool {
    let Some(data) = state::data_mut(server, mode) else {
        return false;
    };
    let clock = Instant::now();
    search_marks_data(data, visible_only, || clock.elapsed().as_millis() as u64)
}

fn scroll_position(data: &mut CopyModeData, x: u32, y: u32) -> bool {
    let gd = &data.backing.screen().grid;
    let old_oy = data.oy;
    data.cx = x;
    let top = gd.hsize() - data.oy;
    if y >= top && y < top + gd.sy() {
        data.cy = y - top;
    } else {
        let gap = gd.sy() / 4;
        let offset = if y < gd.sy() {
            data.cy = y;
            0
        } else if y > gd.hsize() + gd.sy() - gap {
            data.cy = y - gd.hsize();
            gd.hsize()
        } else {
            let offset = y + gap - gd.sy();
            data.cy = y - offset;
            offset
        };
        data.oy = gd.hsize() - offset;
    }
    data.oy != old_oy
}

pub fn scroll_to_inner(server: &mut Server, mode: ModeId, x: u32, y: u32, no_redraw: bool) {
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    let moved = scroll_position(data, x, y);
    let marks = data.search.marks.is_some() && !data.timeout;
    if !no_redraw && marks {
        search_marks(server, mode, true);
    }
    super::select::update_selection(server, mode, true, false);
    if moved {
        let _ = pane_scrollbar_show(server, mode.owner, true);
    }
    if !no_redraw {
        super::render::redraw_screen(server, mode);
    }
}

pub fn scroll_to(server: &mut Server, mode: ModeId, x: u32, y: u32) {
    scroll_to_inner(server, mode, x, y, false);
}

pub fn scroll_to_no_redraw(server: &mut Server, mode: ModeId, x: u32, y: u32) {
    scroll_to_inner(server, mode, x, y, true);
}

fn goto_position(data: &mut CopyModeData, line: i64, absolute: bool) {
    let hsize = data.backing.screen().grid.hsize();
    data.oy = if absolute {
        hsize - (line.max(1).min(i64::from(hsize) + 1) as u32 - 1)
    } else if line < 0 || line as u64 > u64::from(hsize) {
        hsize
    } else {
        line as u32
    };
}

pub fn goto_line(server: &mut Server, mode: ModeId, line: &[u8]) {
    let Ok(line) = rmux_util::strtonum::strtonum(line, -1, i64::from(i32::MAX)) else {
        return;
    };
    let absolute = super::render::line_number_is_absolute(server, mode);
    let Some(data) = state::data_mut(server, mode) else {
        return;
    };
    goto_position(data, line, absolute);
    super::select::update_selection(server, mode, true, false);
    super::render::redraw_screen(server, mode);
}

fn option_number(server: &Server, mode: ModeId, name: &[u8]) -> Option<i64> {
    let pane = server.panes.get(mode.owner)?;
    let window = server.windows.get(pane.window)?;
    Some(server.options.get_number(window.options, name))
}

pub fn search(server: &mut Server, mode: ModeId, direction: SearchDirection, regex: bool) -> bool {
    let Some(data) = state::data_mut(server, mode) else {
        return false;
    };
    let Some(term) = data.search.term.as_deref() else {
        return false;
    };
    let regex = effective_regex(term, regex);
    data.search.searchdirection = direction;
    if data.timeout {
        return false;
    }
    let term = cbytes(term).to_vec();
    let Some(pane) = server.panes.get(mode.owner) else {
        return false;
    };
    let data = state::data(server, mode).expect("copy data remains installed");
    let visible_only = if data.search.all || pane.searchstr.is_none() || pane.searchregex != regex {
        false
    } else {
        pane.searchstr.as_deref() == Some(term.as_slice())
    };
    let data = state::data_mut(server, mode).expect("copy data remains installed");
    data.search.all = false;
    if !visible_only && data.search.marks.is_some() {
        clear_marks_data(data);
    }
    let wrap = option_number(server, mode, b"wrap-search").unwrap_or(0) != 0;
    let keys = if option_number(server, mode, b"mode-keys").unwrap_or(0) != 0 {
        ModeKeys::Vi
    } else {
        ModeKeys::Emacs
    };
    let data = state::data_mut(server, mode).expect("copy data remains installed");
    let prepared = term_grid(&term).map(|needle| {
        let scratch = std::mem::take(&mut data.search.scratch);
        Matcher::with_needle(needle, &term, regex, scratch)
    });
    let pane = server
        .panes
        .get_mut(mode.owner)
        .expect("copy pane remains installed");
    pane.searchstr = Some(term);
    pane.searchregex = regex;
    let Some(prepared) = prepared else {
        return false;
    };
    let Ok(mut matcher) = prepared else {
        super::render::redraw_screen(server, mode);
        return false;
    };
    let data = state::data_mut(server, mode).expect("copy data remains installed");
    let (mut fx, mut fy) = (data.cx, data.backing_y());
    if direction == SearchDirection::Down {
        if keys == ModeKeys::Vi {
            if data.search.marks.is_some() {
                move_after_search_mark(data, &mut fx, &mut fy, wrap);
            } else {
                move_right(data.backing.screen(), &mut fx, &mut fy, wrap);
            }
        }
    } else {
        move_left(data.backing.screen(), &mut fx, &mut fy, wrap);
    }
    let hit = matcher.jump(&data.backing.screen().grid, fx, fy, direction, wrap);
    if let Some((x, y)) = hit {
        scroll_to_no_redraw(server, mode, x, y);
        let data = state::data_mut(server, mode).expect("copy data remains installed");
        let clock = Instant::now();
        marks_with_clock(data, &mut matcher, visible_only, || {
            clock.elapsed().as_millis() as u64
        });
        (fx, fy) = (data.cx, data.backing_y());
        let retry = direction == SearchDirection::Down
            && mark_at(data, fx, fy).is_some_and(|at| {
                at > 0
                    && data
                        .search
                        .marks
                        .as_ref()
                        .is_some_and(|m| m.get(at) == m.get(at - 1))
            });
        if retry {
            move_after_search_mark(data, &mut fx, &mut fy, wrap);
            if let Some((x, y)) = matcher.jump(&data.backing.screen().grid, fx, fy, direction, wrap)
            {
                scroll_to_no_redraw(server, mode, x, y);
            }
        }
        let data = state::data_mut(server, mode).expect("copy data remains installed");
        (fx, fy) = (data.cx, data.backing_y());
        if direction == SearchDirection::Down {
            if keys == ModeKeys::Emacs {
                move_after_search_mark(data, &mut fx, &mut fy, wrap);
                data.cx = fx;
                data.cy = fy
                    .wrapping_sub(data.backing.screen().grid.hsize())
                    .wrapping_add(data.oy);
            }
        } else if let Some(start) = mark_at(data, fx, fy) {
            while let Some(at) = mark_at(data, fx, fy) {
                let Some(marks) = data.search.marks.as_ref() else {
                    break;
                };
                if marks.get(at) != marks.get(start) {
                    break;
                }
                data.cx = fx;
                data.cy = fy
                    .wrapping_sub(data.backing.screen().grid.hsize())
                    .wrapping_add(data.oy);
                if at == 0 {
                    break;
                }
                move_left(data.backing.screen(), &mut fx, &mut fy, false);
            }
        }
    }
    state::data_mut(server, mode)
        .expect("copy data remains installed")
        .search
        .scratch = matcher.scratch;
    super::render::redraw_screen(server, mode);
    hit.is_some()
}

pub fn search_up(server: &mut Server, mode: ModeId, regex: bool) -> bool {
    search(server, mode, SearchDirection::Up, regex)
}

pub fn search_down(server: &mut Server, mode: ModeId, regex: bool) -> bool {
    search(server, mode, SearchDirection::Down, regex)
}

#[cfg(test)]
mod tests {
    use super::super::state::CopyBacking;
    use super::*;
    use crate::ids::{ArenaId, PaneId};
    use rmux_emu::cell::GridCell;
    use rmux_emu::colour::Colour;
    use std::fmt::Write as _;
    use std::path::PathBuf;
    use std::process::Command;

    fn data(width: u32, height: u32, history: u32) -> CopyModeData {
        let mut registry = HyperlinkRegistry::new();
        let mut s = Screen::new(
            width,
            height,
            u32::MAX,
            ScreenResetPolicy::default(),
            &mut registry,
        )
        .unwrap();
        s.release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
        for _ in 0..history {
            s.grid.scroll_history(Colour::DEFAULT);
        }
        let mut data = CopyModeData::new(
            CopyBacking::Snapshot(s),
            PaneId::from_parts(0, 1),
            ModeKeys::Emacs,
        );
        data.cy = 0;
        data.oy = 0;
        data
    }

    fn row(data: &mut CopyModeData, y: u32, text: &[u8], wrapped: bool) {
        let gd = &mut data.backing.screen_mut().grid;
        for (x, &byte) in text.iter().enumerate() {
            gd.set_cell(
                x as u32,
                y,
                &GridCell {
                    data: Utf8Data::set(byte),
                    ..DEFAULT_CELL
                },
            );
        }
        gd.get_line_mut(y).flags = if wrapped {
            GridLineFlags::WRAPPED
        } else {
            GridLineFlags::default()
        };
    }

    fn cell(
        data: &mut CopyModeData,
        x: u32,
        y: u32,
        bytes: &[u8],
        width: u8,
        flags: GridCellFlags,
    ) {
        let mut ud = Utf8Data {
            size: bytes.len() as u8,
            width,
            ..Utf8Data::default()
        };
        ud.data[..bytes.len()].copy_from_slice(bytes);
        data.backing.screen_mut().grid.set_cell(
            x,
            y,
            &GridCell {
                data: ud,
                flags,
                ..DEFAULT_CELL
            },
        );
    }

    #[test]
    fn literal_smart_case_display_width_tabs_and_soft_wrap() {
        let mut d = data(6, 2, 0);
        row(&mut d, 0, b"ABCDxy", true);
        row(&mut d, 1, b"zaBCDz", false);
        let gd = &d.backing.screen().grid;
        let mut lower = Matcher::new(b"abcd", false, Vec::new()).unwrap();
        assert_eq!(lower.row(gd, 0, 0, 6, false), Some((0, 4)));
        let mut upper = Matcher::new(b"Abcd", false, Vec::new()).unwrap();
        assert_eq!(upper.row(gd, 0, 0, 6, false), None);
        let mut wrapped = Matcher::new(b"xyz", false, Vec::new()).unwrap();
        assert_eq!(wrapped.row(gd, 0, 0, 6, false), Some((4, 3)));
        d.backing.screen_mut().grid.get_line_mut(0).flags = GridLineFlags::default();
        assert_eq!(wrapped.row(&d.backing.screen().grid, 0, 0, 6, false), None);
        let mut tab = DEFAULT_CELL;
        tab.set_tab(3);
        d.backing.screen_mut().grid.set_cell(0, 0, &tab);
        cell(&mut d, 1, 0, b"", 0, GridCellFlags::PADDING);
        cell(&mut d, 2, 0, b"", 0, GridCellFlags::PADDING);
        cell(&mut d, 3, 0, b"Q", 1, GridCellFlags::default());
        let mut needle = Matcher::new(b"\tq", false, Vec::new()).unwrap();
        assert_eq!(
            needle.row(&d.backing.screen().grid, 0, 0, 6, false),
            Some((0, 2))
        );
        let mut wide = Grid::new(1, 1, 0);
        wide.set_cell(
            0,
            0,
            &GridCell {
                data: Utf8Data {
                    width: 2,
                    ..Utf8Data::set(b'Q')
                },
                ..DEFAULT_CELL
            },
        );
        assert!(!search_compare(
            &d.backing.screen().grid,
            3,
            0,
            &wide,
            0,
            false
        ));
        assert!(is_lowercase(b"abc"));
        // Non-ASCII byte folding follows libc's current locale, not UTF-8 case.
        let bytes = b"abc\xc3\xa9";
        let folded: Vec<u8> = bytes
            .iter()
            .copied()
            .map(rmux_sys::locale::to_lower)
            .collect();
        assert_eq!(is_lowercase(bytes), bytes.as_slice() == folded.as_slice());
        assert!(!is_lowercase(b"aBc"));
        assert!(!effective_regex(b"abc", true));
        assert!(effective_regex(b"a.c", true));
    }

    #[test]
    fn regex_anchors_zero_length_last_match_and_wrapped_overlap() {
        let mut d = data(6, 2, 0);
        row(&mut d, 0, b"ababab", true);
        row(&mut d, 1, b"ababab", false);
        let gd = &d.backing.screen().grid;
        let mut anchor = Matcher::new(b"^ab", true, Vec::new()).unwrap();
        assert_eq!(anchor.row(gd, 0, 0, 6, false), Some((0, 2)));
        assert_eq!(anchor.row(gd, 0, 1, 6, false), None);
        let mut empty = Matcher::new(b"x*", true, Vec::new()).unwrap();
        assert_eq!(empty.row(gd, 0, 0, 6, false), None);
        let mut last = Matcher::new(b"ab", true, Vec::new()).unwrap();
        assert_eq!(last.row(gd, 0, 0, 6, true), Some((4, 2)));
        let mut overlap = Matcher::new(b"(ab)+", true, Vec::new()).unwrap();
        assert_eq!(
            overlap.jump(gd, 5, 1, SearchDirection::Up, false),
            Some((0, 0))
        );
        assert_eq!(overlap.row(gd, 0, 0, 6, false), Some((0, 12)));
        assert!(matches!(
            Matcher::new(b"[", true, Vec::new()),
            Err(MatcherError::Regex)
        ));
    }

    #[test]
    fn traversal_wraps_once_and_preserves_direction_order() {
        let mut d = data(5, 3, 0);
        row(&mut d, 0, b"a...a", false);
        row(&mut d, 1, b".....", false);
        row(&mut d, 2, b"a...a", false);
        let gd = &d.backing.screen().grid;
        let mut m = Matcher::new(b"a", false, Vec::new()).unwrap();
        assert_eq!(m.jump(gd, 2, 1, SearchDirection::Down, false), Some((0, 2)));
        assert_eq!(m.jump(gd, 2, 1, SearchDirection::Up, false), Some((4, 0)));
        assert_eq!(m.jump(gd, 5, 2, SearchDirection::Down, false), None);
        assert_eq!(m.jump(gd, 5, 2, SearchDirection::Down, true), Some((0, 0)));
        let mut absent = Matcher::new(b"z", false, Vec::new()).unwrap();
        assert_eq!(absent.jump(gd, 0, 0, SearchDirection::Up, true), None);
    }

    #[test]
    fn suffix_mapping_preserves_partial_cells_padding_and_repeated_suffix() {
        let mut d = data(5, 1, 0);
        cell(&mut d, 0, 0, b"ab", 1, GridCellFlags::default());
        cell(&mut d, 1, 0, b"", 0, GridCellFlags::PADDING);
        row(&mut d, 0, b"", false);
        cell(&mut d, 2, 0, b"ab", 1, GridCellFlags::default());
        cell(&mut d, 3, 0, b"x", 1, GridCellFlags::default());
        let gd = &d.backing.screen().grid;
        let (mut x, mut y) = (0, 0);
        cstrtocellpos(gd, 5, &mut x, &mut y, b"abx ");
        // C accepts the first suffix candidate: leading padding consumes no bytes.
        assert_eq!((x, y), (1, 0));
        (x, y) = (0, 0);
        cstrtocellpos(gd, 1, &mut x, &mut y, b"a");
        assert_eq!((x, y), (0, 0));
        cstrtocellpos(gd, 1, &mut x, &mut y, b"b");
        assert_eq!((x, y), (1, 0));
        let mut buf = Vec::new();
        stringify(gd, 0, 0, 5, &mut buf);
        assert_eq!(buf, b"ababx ");
    }

    #[test]
    fn regex_line_limit_is_cell_span_before_appending_not_byte_limit() {
        let mut d = data(1200, 3, 0);
        d.backing.screen_mut().grid.get_line_mut(0).flags = GridLineFlags::WRAPPED;
        d.backing.screen_mut().grid.get_line_mut(1).flags = GridLineFlags::WRAPPED;
        let mut m = Matcher::new(b".", true, Vec::new()).unwrap();
        assert_eq!(m.serialize(&d.backing.screen().grid, 0, 0), 2400);
        assert_eq!(m.scratch.len(), 2400);
        assert_eq!(m.serialize(&d.backing.screen().grid, 0, 1100), 2500);
        assert_eq!(m.scratch.len(), 2500);
    }

    #[test]
    fn marks_clip_expand_tabs_reuse_generation_and_keep_existing_cells() {
        let mut d = data(8, 2, 0);
        d.search.marks = Some(vec![0; 16]);
        d.search.generation = 255;
        assert_eq!(mark_match(&mut d, 6, 1, 8, true), 2);
        assert_eq!(d.search.generation, 1);
        assert_eq!(&d.search.marks.as_ref().unwrap()[14..], &[255, 255]);
        assert_eq!(mark_match(&mut d, 5, 1, 3, true), 3);
        assert_eq!(&d.search.marks.as_ref().unwrap()[13..], &[1, 255, 255]);
        let mut tab = DEFAULT_CELL;
        tab.set_tab(4);
        d.backing.screen_mut().grid.set_cell(0, 0, &tab);
        assert_eq!(mark_match(&mut d, 0, 0, 2, false), 5);
        assert_eq!(&d.search.marks.as_ref().unwrap()[..5], &[2; 5]);
        d.cx = 5;
        d.cy = 0;
        assert_eq!(match_at_cursor(&d), Some((0, 0, 4, 0)));
        d.cx = 6;
        assert_eq!(match_at_cursor(&d), None);
        d.search.marks.as_mut().unwrap().fill(7);
        assert_eq!(match_start_end(&d, 8), Some((0, 15)));
        let (mut x, mut y) = (0, 0);
        move_after_search_mark(&d, &mut x, &mut y, true);
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn visible_marks_include_wrapped_context_and_preserve_previous_count() {
        let mut d = data(4, 2, 2);
        row(&mut d, 0, b"....", false);
        row(&mut d, 1, b"...a", true);
        row(&mut d, 2, b"bc..", false);
        d.search.count = 17;
        d.search.more = true;
        let mut m = Matcher::new(b"abc", false, Vec::new()).unwrap();
        assert_eq!(visible_lines(&d), (1, 4));
        assert!(marks_with_clock(&mut d, &mut m, true, || 0));
        // Matches starting outside the viewport are counted but not marked in C.
        assert_eq!(d.search.marks.as_ref().unwrap(), &[0; 8]);
        assert_eq!(d.search.count, 17);
        assert!(d.search.more);
        d.oy = 1;
        marks_with_clock(&mut d, &mut m, false, || 0);
        assert_eq!(d.search.count, 1);
        assert_eq!(&d.search.marks.as_ref().unwrap()[3..6], &[1; 3]);
        assert!(!d.search.more);
    }

    #[test]
    fn injected_deadlines_strict_thresholds_and_visible_retry_accumulation() {
        for (count, expected) in [
            (0, -1),
            (10, -1),
            (11, 10),
            (100, 10),
            (101, 100),
            (1000, 100),
            (1001, 1000),
        ] {
            assert_eq!(partial_count(count), expected);
        }
        let mut d = data(11, 1, 1);
        row(&mut d, 0, b"aaaaaaaaaa.", false);
        row(&mut d, 1, b"a..........", false);
        let mut m = Matcher::new(b"a", false, Vec::new()).unwrap();
        let mut times = [0, 0, 201, 201].into_iter();
        marks_with_clock(&mut d, &mut m, false, || times.next().unwrap());
        assert_eq!(d.search.count, 10);
        assert!(d.search.more);
        assert_eq!(d.search.marks.as_ref().unwrap()[0], 1);
        assert!(
            d.search.marks.as_ref().unwrap()[1..]
                .iter()
                .all(|&b| b == 0)
        );
        let mut times = [0, 0, 200, 200].into_iter();
        marks_with_clock(&mut d, &mut m, false, || times.next().unwrap());
        assert_eq!(d.search.count, 11);
        assert!(!d.search.more);
        let mut times = [0, 10000].into_iter();
        marks_with_clock(&mut d, &mut m, true, || times.next().unwrap());
        assert!(!d.timeout);
        let mut times = [0, 10001].into_iter();
        marks_with_clock(&mut d, &mut m, true, || times.next().unwrap());
        assert!(d.timeout);
        assert!(d.search.marks.is_none());
        assert_eq!(d.search.count, -1);
        assert!(!d.search.more);
    }

    #[test]
    fn generations_wrap_after_255_matches_and_clear_resets_only_marks_count() {
        let mut d = data(260, 1, 0);
        row(&mut d, 0, &vec![b'a'; 260], false);
        let mut m = Matcher::new(b"a", false, Vec::new()).unwrap();
        marks_with_clock(&mut d, &mut m, false, || 0);
        let marks = d.search.marks.as_ref().unwrap();
        assert_eq!(marks[0], 1);
        assert_eq!(marks[254], 255);
        assert_eq!(marks[255], 1);
        assert_eq!(d.search.count, 260);
        d.timeout = true;
        d.search.term = Some(b"a".to_vec());
        clear_marks_data(&mut d);
        assert!(d.timeout);
        assert_eq!(d.search.term.as_deref(), Some(b"a".as_slice()));
        assert_eq!(d.search.count, -1);
        assert!(!d.search.more);
        assert!(d.search.marks.is_none());
    }

    #[test]
    fn scroll_gap_visible_noop_and_goto_line_clamps() {
        let mut d = data(10, 8, 24);
        assert!(!scroll_position(&mut d, 3, 26));
        assert_eq!((d.cx, d.cy, d.oy), (3, 2, 0));
        assert!(scroll_position(&mut d, 4, 15));
        assert_eq!((d.cx, d.cy, d.oy), (4, 6, 15));
        assert!(scroll_position(&mut d, 0, 3));
        assert_eq!((d.cy, d.oy), (3, 24));
        assert!(scroll_position(&mut d, 0, 31));
        assert_eq!((d.cy, d.oy), (7, 0));
        for (line, absolute, oy) in [
            (-1, false, 24),
            (25, false, 24),
            (4, false, 4),
            (-1, true, 24),
            (0, true, 24),
            (4, true, 21),
            (i64::from(i32::MAX), true, 0),
        ] {
            goto_position(&mut d, line, absolute);
            assert_eq!(d.oy, oy);
        }
        let (mut x, mut y) = (0, 0);
        move_left(d.backing.screen(), &mut x, &mut y, false);
        assert_eq!((x, y), (0, 0));
        move_left(d.backing.screen(), &mut x, &mut y, true);
        assert_eq!((x, y), (9, 31));
        move_right(d.backing.screen(), &mut x, &mut y, true);
        assert_eq!((x, y), (0, 0));
        assert!(term_grid(b"\x02").is_none());
    }

    #[test]
    fn invalid_regex_removes_stale_buffer_but_preserves_counter_state() {
        let mut d = data(8, 1, 0);
        d.search.term = Some(b"[".to_vec());
        d.search.regex = true;
        d.search.marks = Some(vec![1; 8]);
        d.search.count = 19;
        d.search.more = true;
        assert!(!search_marks_data(&mut d, true, || panic!(
            "invalid regex must not start clock"
        )));
        assert!(d.search.marks.is_none());
        assert_eq!(d.search.count, 19);
        assert!(d.search.more);
    }

    fn server_mode() -> (Server, ModeId) {
        use super::super::{CopyModeDriver, CopyModeKind};
        use crate::model::{pane, window};
        let mut server = Server::new();
        let window = window::window_create(&mut server, 8, 3, 0, 0).unwrap();
        let pane = pane::pane_create(&mut server, window, 8, 3, 10).unwrap();
        server.windows.get_mut(window).unwrap().panes.push(pane);
        server.windows.get_mut(window).unwrap().active = Some(pane);
        let driver = std::rc::Rc::new(CopyModeDriver {
            kind: CopyModeKind::Copy {
                source: None,
                args: crate::cmd::arguments::Args::default(),
            },
        });
        let mode = pane::pane_set_mode(
            &mut server,
            pane,
            b"copy-mode",
            crate::modes::WindowModeFlags::default(),
            driver,
            false,
        )
        .unwrap()
        .unwrap();
        let d = state::data_mut(&mut server, mode).unwrap();
        row(d, 0, b"ab..ab..", false);
        row(d, 1, b"........", false);
        row(d, 2, b"........", false);
        d.cx = 0;
        d.cy = 0;
        d.search.term = Some(b"ab".to_vec());
        d.search.regex = true;
        (server, mode)
    }

    #[test]
    fn wrapper_search_uses_live_keys_remembers_effective_regex_and_stops_on_timeout() {
        let (mut server, mode) = server_mode();
        assert!(search_down(&mut server, mode, true));
        assert_eq!(state::data(&server, mode).unwrap().cx, 2);
        let pane = server.panes.get(mode.owner).unwrap();
        assert_eq!(pane.searchstr.as_deref(), Some(b"ab".as_slice()));
        assert!(!pane.searchregex);
        assert!(state::data(&server, mode).unwrap().search.regex);
        assert!(!state::data(&server, mode).unwrap().search.all);
        assert_eq!(state::data(&server, mode).unwrap().search.count, 2);
        let window = pane.window;
        let options = server.windows.get(window).unwrap().options;
        server.options.set_number_value(options, b"mode-keys", 1);
        assert!(search_down(&mut server, mode, false));
        assert_eq!(state::data(&server, mode).unwrap().cx, 4);
        assert!(search_up(&mut server, mode, false));
        assert_eq!(state::data(&server, mode).unwrap().cx, 0);
        assert!(search_down(&mut server, mode, false));
        assert_eq!(state::data(&server, mode).unwrap().cx, 4);
        server.options.set_number_value(options, b"wrap-search", 0);
        assert!(!search_down(&mut server, mode, false));
        server.options.set_number_value(options, b"wrap-search", 1);
        assert!(search_down(&mut server, mode, false));
        assert_eq!(state::data(&server, mode).unwrap().cx, 0);
        let d = state::data_mut(&mut server, mode).unwrap();
        d.timeout = true;
        d.search.term = Some(b"changed".to_vec());
        assert!(!search_down(&mut server, mode, false));
        assert_eq!(
            server.panes.get(mode.owner).unwrap().searchstr.as_deref(),
            Some(b"ab".as_slice())
        );
    }

    #[test]
    fn wrapper_goto_invalid_values_and_regex_mark_cleanup() {
        let (mut server, mode) = server_mode();
        let d = state::data_mut(&mut server, mode).unwrap();
        d.search.term = Some(b"[".to_vec());
        d.search.regex = true;
        d.search.marks = Some(vec![1; 24]);
        assert!(!search_marks(&mut server, mode, true));
        assert!(state::data(&server, mode).unwrap().search.marks.is_none());
        for text in [b"-2".as_slice(), b"2147483648", b"abc", b"1 "] {
            let before = state::data(&server, mode).unwrap().oy;
            goto_line(&mut server, mode, text);
            assert_eq!(state::data(&server, mode).unwrap().oy, before);
        }
    }

    fn c_function_range<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let source = &source[source.find(start).expect("pinned C start")..];
        &source[..source.find(end).expect("pinned C end")]
    }

    // RMUX_COPY_SEARCH_MUTATE=1 deliberately corrupts the comparison result.
    #[test]
    fn pinned_c_literal_regex_suffix_and_overlap_differential() {
        let source_path = std::env::var_os("RMUX_TMUX_COPY_SOURCE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/Users/j/fun/tmux/window-copy.c"));
        let Ok(source) = std::fs::read_to_string(&source_path) else {
            eprintln!(
                "SKIP copy search C differential: pinned source missing at {}",
                source_path.display()
            );
            return;
        };
        let mut c = String::from(
            r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <regex.h>
#include <stdint.h>
typedef unsigned int u_int;
typedef unsigned char u_char;
#define GRID_FLAG_TAB 128
#define GRID_FLAG_PADDING 4
#define GRID_LINE_WRAPPED 1
#define WINDOW_COPY_SEARCH_MAX_LINE 2000
struct utf8_data { unsigned char data[32]; unsigned char size, width; };
struct grid_cell { struct utf8_data data; unsigned char flags; };
struct grid_line { u_int flags; struct grid_cell cells[8]; };
struct grid { u_int sx, sy, hsize; struct grid_line lines[3]; };
static void *xmalloc(size_t n) { void *p = malloc(n ? n : 1); if (!p) abort(); return p; }
static void *xrealloc(void *p, size_t n) { p = realloc(p, n ? n : 1); if (!p) abort(); return p; }
static void *xreallocarray(void *p, size_t n, size_t s) { return xrealloc(p,n*s); }
static struct grid_line *grid_get_line(struct grid *g, u_int y) { return &g->lines[y]; }
static const struct grid_line *grid_peek_line(struct grid *g, u_int y) { return y < g->sy ? &g->lines[y] : NULL; }
static void grid_get_cell(struct grid *g, u_int x, u_int y, struct grid_cell *out) {
    memset(out, 0, sizeof *out); out->data.data[0] = ' '; out->data.size = out->data.width = 1;
    if (y < g->sy && x < g->sx) *out = g->lines[y].cells[x];
}
static const char *window_copy_cellstring(const struct grid_line *l, u_int x, size_t *size, int *allocated) {
    *allocated = 0;
    if (x >= 8) { *size = 1; return " "; }
    const struct grid_cell *c = &l->cells[x];
    if (c->flags & GRID_FLAG_PADDING) { *size = 0; return NULL; }
    if (c->flags & GRID_FLAG_TAB) { *size = 1; return "\t"; }
    *size = c->data.size; return (const char *)c->data.data;
}
static char *window_copy_stringify(struct grid *, u_int, u_int, u_int, char *, u_int *);
static void window_copy_cstrtocellpos(struct grid *, u_int, u_int *, u_int *, const char *);
static int window_copy_last_regex(struct grid *, u_int, u_int, u_int, u_int, u_int *, u_int *, const char *, const regex_t *, int);
"#,
        );
        c.push_str(c_function_range(
            &source,
            "static int\nwindow_copy_search_compare(",
            "static const char *\nwindow_copy_cellstring(",
        ));
        c.push_str(c_function_range(
            &source,
            "static int\nwindow_copy_last_regex(",
            "/* Stringify line",
        ));
        c.push_str(c_function_range(
            &source,
            "static char *\nwindow_copy_stringify(",
            "static void\nwindow_copy_move_left(",
        ));
        c.push_str(c_function_range(
            &source,
            "static void\nwindow_copy_search_back_overlap(",
            "/*\n * Search for text",
        ));
        let mut d = data(6, 3, 0);
        row(&mut d, 0, b"ababab", true);
        row(&mut d, 1, b"ababab", false);
        row(&mut d, 2, b"A.....", false);
        c.push_str("int main(void) { struct grid gd = {.sx=6,.sy=3}, needle={.sx=2,.sy=1}; u_int x=0,w=0,y=0,i=0; int f; regex_t re;\n");
        for y in 0..3 {
            writeln!(
                c,
                "gd.lines[{y}].flags={};",
                d.backing.screen().grid.get_line(y).flags.0
            )
            .unwrap();
            for x in 0..6 {
                let gc = d.backing.screen().grid.get_cell(x, y);
                writeln!(c, "gd.lines[{y}].cells[{x}].data.size={}; gd.lines[{y}].cells[{x}].data.width={}; gd.lines[{y}].cells[{x}].flags={};", gc.data.size, gc.data.width, gc.flags.0).unwrap();
                for (b, byte) in gc.data.bytes().iter().enumerate() {
                    writeln!(c, "gd.lines[{y}].cells[{x}].data.data[{b}]={byte};").unwrap();
                }
            }
        }
        c.push_str("needle.lines[0].cells[0].data=(struct utf8_data){{'a'},1,1}; needle.lines[0].cells[1].data=(struct utf8_data){{'b'},1,1};\n");
        let mut expected = String::new();
        for (pattern, y, first, last, backward) in [
            (b"ab".as_slice(), 0, 0, 6, false),
            (b"ab", 0, 0, 6, true),
            (b"^ab", 0, 1, 6, false),
            (b"(ab)+", 0, 0, 6, false),
            (b"x*", 0, 0, 6, false),
            (b"a", 2, 0, 6, false),
        ] {
            let mut m = Matcher::new(pattern, true, Vec::new()).unwrap();
            let result = m.row(&d.backing.screen().grid, y, first, last, backward);
            let (x, width) = result.unwrap_or((0, 0));
            writeln!(expected, "{} {x} {width}", i32::from(result.is_some())).unwrap();
            let name = if backward { "rl" } else { "lr" };
            writeln!(c, "regcomp(&re,\"{}\",REG_EXTENDED|REG_ICASE); x=w=0; f=window_copy_search_{name}_regex(&gd,&x,&w,{y},{first},{last},&re); printf(\"%d %u %u\\n\",f,x,w); regfree(&re);", String::from_utf8_lossy(pattern)).unwrap();
        }
        let mut literal = Matcher::new(b"ab", false, Vec::new()).unwrap();
        for backward in [false, true] {
            let hit = literal
                .row(&d.backing.screen().grid, 0, 0, 6, backward)
                .unwrap();
            writeln!(expected, "1 {}", hit.0).unwrap();
            writeln!(
                c,
                "x=0; f=window_copy_search_{}(&gd,&needle,&x,0,0,6,1); printf(\"%d %u\\n\",f,x);",
                if backward { "rl" } else { "lr" }
            )
            .unwrap();
        }
        let mut overlap = Matcher::new(b"(ab)+", true, Vec::new()).unwrap();
        let (mut x, mut y) = (0, 1);
        overlap.back_overlap(&d.backing.screen().grid, &mut x, 6, &mut y, 0);
        writeln!(expected, "{x} {}", y + 1).unwrap();
        c.push_str("regcomp(&re,\"(ab)+\",REG_EXTENDED); x=0; i=2; w=6; window_copy_search_back_overlap(&gd,&re,&x,&w,&i,0); printf(\"%u %u\\n\",x,i); regfree(&re);\n");
        // Partial multi-byte cell and repeated suffix fixtures use the original mapper.
        cell(&mut d, 0, 2, b"ab", 1, GridCellFlags::default());
        cell(&mut d, 1, 2, b"", 0, GridCellFlags::PADDING);
        cell(&mut d, 2, 2, b"ab", 1, GridCellFlags::default());
        c.push_str("gd.lines[2].cells[0].data=(struct utf8_data){{'a','b'},2,1}; gd.lines[2].cells[1].flags=GRID_FLAG_PADDING; gd.lines[2].cells[2].data=(struct utf8_data){{'a','b'},2,1};\n");
        for (ncells, suffix) in [(1, b"a".as_slice()), (1, b"b"), (6, b"ab...")] {
            let (mut x, mut y) = (0, 2);
            cstrtocellpos(&d.backing.screen().grid, ncells, &mut x, &mut y, suffix);
            writeln!(expected, "{x} {y}").unwrap();
            writeln!(c, "x=0;y=2;window_copy_cstrtocellpos(&gd,{ncells},&x,&y,\"{}\");printf(\"%u %u\\n\",x,y);", String::from_utf8_lossy(suffix)).unwrap();
        }
        for (bytes, width, flags) in [
            (b"\xc3\xa9".as_slice(), 1, GridCellFlags::default()),
            (b"e\xcc\x81", 1, GridCellFlags::default()),
            (b"\xe7\x95\x8c", 2, GridCellFlags::default()),
            (b"\t", 3, GridCellFlags::TAB),
        ] {
            if flags.contains(GridCellFlags::TAB) {
                let mut tab = DEFAULT_CELL;
                tab.set_tab(u32::from(width));
                d.backing.screen_mut().grid.set_cell(0, 2, &tab);
                writeln!(c, "gd.lines[2].cells[0].flags=GRID_FLAG_TAB; gd.lines[2].cells[0].data.size={width}; gd.lines[2].cells[0].data.width={width};").unwrap();
            } else {
                cell(&mut d, 0, 2, bytes, width, flags);
                writeln!(c, "gd.lines[2].cells[0].flags=0; gd.lines[2].cells[0].data.size={}; gd.lines[2].cells[0].data.width={width};", bytes.len()).unwrap();
                for (i, byte) in bytes.iter().enumerate() {
                    writeln!(c, "gd.lines[2].cells[0].data.data[{i}]={byte};").unwrap();
                }
            }
            for suffix in [bytes, &bytes[..1], &bytes[1..], b"".as_slice()] {
                let (mut x, mut y) = (0, 2);
                cstrtocellpos(&d.backing.screen().grid, 1, &mut x, &mut y, suffix);
                writeln!(expected, "{x} {y}").unwrap();
                let mut escaped = String::new();
                for byte in suffix {
                    write!(escaped, "\\{byte:03o}").unwrap();
                }
                writeln!(c, "x=0;y=2;window_copy_cstrtocellpos(&gd,1,&x,&y,\"{escaped}\");printf(\"%u %u\\n\",x,y);").unwrap();
            }
        }
        c.push_str("return 0;}\n");
        let dir = PathBuf::from("/tmp/swarm-rmux-build")
            .join(format!("copy-search-c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfile = dir.join("search.c");
        let binary = dir.join("search");
        std::fs::write(&cfile, c).unwrap();
        let output = match Command::new("cc")
            .args(["-std=c99", "-Wno-unused-function"])
            .arg(&cfile)
            .arg("-o")
            .arg(&binary)
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("SKIP copy search C differential: cc unavailable");
                std::fs::remove_dir_all(dir).unwrap();
                return;
            }
            Err(error) => panic!("C compiler: {error}"),
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = Command::new(&binary).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        std::fs::remove_dir_all(dir).unwrap();
        if std::env::var_os("RMUX_COPY_SEARCH_MUTATE").is_some() {
            expected.insert(0, '!');
        }
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected);
    }
}
