// Ported from tmux format-draw.c @ 8f25579c
/*
 * Copyright (c) 2019 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! Styled one-line drawing of an expanded format into a screen, plus the
//! width and trim scanners that share its `#` and style rules but keep their
//! own invalid-input behaviour (`format-draw.c:1098-1266`).

use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::hyperlinks::HyperlinkId;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_emu::style::{
    Style, StyleAlign, StyleDefaultType, StyleList, StyleRange, StyleRangeType, StyleRanges,
};
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::utf8::{Utf8Data, Utf8State};

use super::skip;

const LEFT: usize = 0;
const CENTRE: usize = 1;
const RIGHT: usize = 2;
const ABSOLUTE_CENTRE: usize = 3;
const LIST: usize = 4;
const LIST_LEFT: usize = 5;
const LIST_RIGHT: usize = 6;
const AFTER: usize = 7;
const TOTAL: usize = 8;

/// `struct format_range`: a range still measured in a section screen.
#[derive(Clone, Debug)]
struct Range {
    index: usize,
    start: u32,
    end: u32,
    range_type: StyleRangeType,
    argument: u32,
    string: [u8; 16],
}

impl Range {
    /// `format_is_type` (`format-draw.c:43-62`).
    fn is_type(&self, sy: &Style) -> bool {
        if self.range_type != sy.range_type {
            return false;
        }
        match self.range_type {
            StyleRangeType::Pane | StyleRangeType::Window | StyleRangeType::Session => {
                self.argument == sy.range_argument
            }
            StyleRangeType::User => cstr(&self.string) == cstr(&sy.range_string),
            _ => true,
        }
    }
}

/// `format_update_ranges`: clip the ranges of one section to the copied
/// part and move them to the target offset (`format-draw.c:73-106`).
fn update_ranges(frs: &mut Vec<Range>, index: usize, offset: u32, start: u32, width: u32) {
    frs.retain_mut(|fr| {
        if fr.index != index {
            return true;
        }
        if fr.end <= start || fr.start >= start + width {
            return false;
        }
        if fr.start < start {
            fr.start = start;
        }
        if fr.end > start + width {
            fr.end = start + width;
        }
        if fr.start == fr.end {
            return false;
        }
        fr.start -= start;
        fr.end -= start;
        fr.start += offset;
        fr.end += offset;
        true
    });
}

struct Target<'a, 'b> {
    octx: &'a mut ScreenWriteCtx<'b>,
    ocx: u32,
    ocy: u32,
}

impl Target<'_, '_> {
    /// `format_draw_put`: copy `width` cells from `start` of a section to
    /// `offset` from the original cursor (`format-draw.c:109-121`).
    fn put(
        &mut self,
        s: &[Screen],
        index: usize,
        frs: &mut Vec<Range>,
        offset: u32,
        start: u32,
        width: u32,
    ) {
        self.octx
            .cursormove((self.ocx + offset) as i32, self.ocy as i32, false);
        self.octx.fast_copy(&s[index], start, 0, width, 1);
        update_ranges(frs, index, offset, start, width);
    }

    fn copy_marker(&mut self, s: &[Screen], index: usize, offset: u32) {
        self.octx
            .cursormove((self.ocx + offset) as i32, self.ocy as i32, false);
        self.octx.fast_copy(&s[index], 0, 0, s[index].cx, 1);
    }

    /// `format_draw_put_list` (`format-draw.c:124-165`).
    #[allow(clippy::too_many_arguments)]
    fn put_list(
        &mut self,
        s: &[Screen],
        frs: &mut Vec<Range>,
        mut offset: u32,
        mut width: u32,
        focus_start: u32,
        focus_end: u32,
    ) {
        let list = &s[LIST];
        if width >= list.cx {
            self.put(s, LIST, frs, offset, 0, width);
            return;
        }

        // The list needs to be trimmed. Try to keep the focus visible.
        let focus_centre = focus_start + (focus_end - focus_start) / 2;
        let mut start = focus_centre.saturating_sub(width / 2);
        if start + width > list.cx {
            start = list.cx - width;
        }

        // Draw <> markers at either side if needed.
        if start != 0 && width > s[LIST_LEFT].cx {
            self.copy_marker(s, LIST_LEFT, offset);
            offset += s[LIST_LEFT].cx;
            start += s[LIST_LEFT].cx;
            width -= s[LIST_LEFT].cx;
        }
        if start + width < list.cx && width > s[LIST_RIGHT].cx {
            self.copy_marker(s, LIST_RIGHT, offset + width - s[LIST_RIGHT].cx);
            width -= s[LIST_RIGHT].cx;
        }

        self.put(s, LIST, frs, offset, start, width);
    }

    /// `format_draw_none` (`format-draw.c:168-224`).
    fn draw_none(&mut self, available: u32, s: &[Screen], frs: &mut Vec<Range>) {
        let mut width_left = s[LEFT].cx;
        let mut width_centre = s[CENTRE].cx;
        let mut width_right = s[RIGHT].cx;
        let mut width_abs_centre = s[ABSOLUTE_CENTRE].cx;

        // Try to keep as much of the left and right as possible at the
        // expense of the centre.
        while width_left + width_centre + width_right > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else {
                width_left -= 1;
            }
        }

        self.put(s, LEFT, frs, 0, 0, width_left);
        self.put(
            s,
            RIGHT,
            frs,
            available - width_right,
            s[RIGHT].cx - width_right,
            width_right,
        );
        self.put(
            s,
            CENTRE,
            frs,
            width_left + ((available - width_right) - width_left) / 2 - width_centre / 2,
            s[CENTRE].cx / 2 - width_centre / 2,
            width_centre,
        );

        if width_abs_centre > available {
            width_abs_centre = available;
        }
        self.put(
            s,
            ABSOLUTE_CENTRE,
            frs,
            (available - width_abs_centre) / 2,
            0,
            width_abs_centre,
        );
    }

    fn put_abs_centre(&mut self, available: u32, s: &[Screen], frs: &mut Vec<Range>) {
        let width_abs_centre = s[ABSOLUTE_CENTRE].cx.min(available);
        self.put(
            s,
            ABSOLUTE_CENTRE,
            frs,
            (available - width_abs_centre) / 2,
            0,
            width_abs_centre,
        );
    }

    /// `format_draw_left` (`format-draw.c:227-327`).
    fn draw_left(
        &mut self,
        available: u32,
        s: &mut [Screen],
        focus: Option<(u32, u32)>,
        frs: &mut Vec<Range>,
    ) {
        let mut width_left = s[LEFT].cx;
        let mut width_centre = s[CENTRE].cx;
        let mut width_right = s[RIGHT].cx;
        let mut width_list = s[LIST].cx;
        let mut width_after = s[AFTER].cx;

        // Trim first the centre, then the list, then the right, then after
        // the list, then the left.
        while width_left + width_centre + width_right + width_list + width_after > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_list > 0 {
                width_list -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else {
                width_left -= 1;
            }
        }

        if width_list == 0 {
            merge_after(s, LEFT, width_after, self.octx);
            self.draw_none(available, s, frs);
            return;
        }

        self.put(s, LEFT, frs, 0, 0, width_left);
        self.put(
            s,
            RIGHT,
            frs,
            available - width_right,
            s[RIGHT].cx - width_right,
            width_right,
        );
        self.put(s, AFTER, frs, width_left + width_list, 0, width_after);

        // Write centre halfway between width_left + width_list + width_after
        // and available - width_right.
        let before = width_left + width_list + width_after;
        self.put(
            s,
            CENTRE,
            frs,
            before + ((available - width_right) - before) / 2 - width_centre / 2,
            s[CENTRE].cx / 2 - width_centre / 2,
            width_centre,
        );

        // If there is no focus given, keep the left in focus.
        let (focus_start, focus_end) = focus.unwrap_or((0, 0));
        self.put_list(s, frs, width_left, width_list, focus_start, focus_end);

        self.put_abs_centre(available, s, frs);
    }

    /// `format_draw_centre` (`format-draw.c:330-435`).
    fn draw_centre(
        &mut self,
        available: u32,
        s: &mut [Screen],
        focus: Option<(u32, u32)>,
        frs: &mut Vec<Range>,
    ) {
        let mut width_left = s[LEFT].cx;
        let mut width_centre = s[CENTRE].cx;
        let mut width_right = s[RIGHT].cx;
        let mut width_list = s[LIST].cx;
        let mut width_after = s[AFTER].cx;

        // Trim first the list, then after the list, then the centre, then
        // the right, then the left.
        while width_left + width_centre + width_right + width_list + width_after > available {
            if width_list > 0 {
                width_list -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else if width_centre > 0 {
                width_centre -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else {
                width_left -= 1;
            }
        }

        if width_list == 0 {
            merge_after(s, CENTRE, width_after, self.octx);
            self.draw_none(available, s, frs);
            return;
        }

        self.put(s, LEFT, frs, 0, 0, width_left);
        self.put(
            s,
            RIGHT,
            frs,
            available - width_right,
            s[RIGHT].cx - width_right,
            width_right,
        );

        // All three centre sections are offset from the middle of the
        // available space.
        let middle = width_left + ((available - width_right) - width_left) / 2;

        self.put(
            s,
            CENTRE,
            frs,
            middle - width_list / 2 - width_centre,
            0,
            width_centre,
        );
        self.put(
            s,
            AFTER,
            frs,
            middle - width_list / 2 + width_list,
            0,
            width_after,
        );

        // If there is no focus given, keep the centre in focus.
        let (focus_start, focus_end) = focus.unwrap_or((s[LIST].cx / 2, s[LIST].cx / 2));
        self.put_list(
            s,
            frs,
            middle - width_list / 2,
            width_list,
            focus_start,
            focus_end,
        );

        self.put_abs_centre(available, s, frs);
    }

    /// `format_draw_right` (`format-draw.c:438-542`).
    fn draw_right(
        &mut self,
        available: u32,
        s: &mut [Screen],
        focus: Option<(u32, u32)>,
        frs: &mut Vec<Range>,
    ) {
        let mut width_left = s[LEFT].cx;
        let mut width_centre = s[CENTRE].cx;
        let mut width_right = s[RIGHT].cx;
        let mut width_list = s[LIST].cx;
        let mut width_after = s[AFTER].cx;

        // Trim first the centre, then the list, then the right, then after
        // the list, then the left.
        while width_left + width_centre + width_right + width_list + width_after > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_list > 0 {
                width_list -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else {
                width_left -= 1;
            }
        }

        if width_list == 0 {
            merge_after(s, RIGHT, width_after, self.octx);
            self.draw_none(available, s, frs);
            return;
        }

        self.put(s, LEFT, frs, 0, 0, width_left);
        self.put(
            s,
            AFTER,
            frs,
            available - width_after,
            s[AFTER].cx - width_after,
            width_after,
        );
        self.put(
            s,
            RIGHT,
            frs,
            available - width_right - width_list - width_after,
            0,
            width_right,
        );

        // Write centre halfway between width_left and
        // available - width_right - width_list - width_after.
        let after_start = available - width_right - width_list - width_after;
        self.put(
            s,
            CENTRE,
            frs,
            width_left + (after_start - width_left) / 2 - width_centre / 2,
            s[CENTRE].cx / 2 - width_centre / 2,
            width_centre,
        );

        // If there is no focus given, keep the right in focus.
        let (focus_start, focus_end) = focus.unwrap_or((0, 0));
        self.put_list(
            s,
            frs,
            available - width_list - width_after,
            width_list,
            focus_start,
            focus_end,
        );

        self.put_abs_centre(available, s, frs);
    }

    /// `format_draw_absolute_centre` (`format-draw.c:544-644`).
    fn draw_absolute_centre(
        &mut self,
        available: u32,
        s: &[Screen],
        focus: Option<(u32, u32)>,
        frs: &mut Vec<Range>,
    ) {
        let mut width_left = s[LEFT].cx;
        let mut width_centre = s[CENTRE].cx;
        let mut width_right = s[RIGHT].cx;
        let mut width_abs_centre = s[ABSOLUTE_CENTRE].cx;
        let mut width_list = s[LIST].cx;
        let mut width_after = s[AFTER].cx;

        // Trim first centre, then the right, then the left.
        while width_left + width_centre + width_right > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else {
                width_left -= 1;
            }
        }

        // List, after and abs_centre are trimmed independently, as they are
        // drawn over the rest: first the list, then after the list, then
        // abs_centre.
        while width_list + width_after + width_abs_centre > available {
            if width_list > 0 {
                width_list -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else {
                width_abs_centre -= 1;
            }
        }

        self.put(s, LEFT, frs, 0, 0, width_left);
        self.put(
            s,
            RIGHT,
            frs,
            available - width_right,
            s[RIGHT].cx - width_right,
            width_right,
        );

        // Keep writing centre at the relative centre. Only the list is
        // written in the absolute centre of the horizontal space.
        let middle = width_left + ((available - width_right) - width_left) / 2;
        self.put(s, CENTRE, frs, middle - width_centre, 0, width_centre);

        // If there is no focus given, keep the centre in focus.
        let (focus_start, focus_end) = focus.unwrap_or((s[LIST].cx / 2, s[LIST].cx / 2));

        // abs_centre and the list are centred together, so their shared
        // centre is in the perfect centre of horizontal space.
        let mut abs_centre_offset = (available - width_list - width_abs_centre) / 2;
        self.put(
            s,
            ABSOLUTE_CENTRE,
            frs,
            abs_centre_offset,
            0,
            width_abs_centre,
        );
        abs_centre_offset += width_abs_centre;

        self.put_list(
            s,
            frs,
            abs_centre_offset,
            width_list,
            focus_start,
            focus_end,
        );
        abs_centre_offset += width_list;

        self.put(s, AFTER, frs, abs_centre_offset, 0, width_after);
    }
}

/// The no-list fallback copies `after` into its alignment section at that
/// section's cursor; `fast_copy` leaves the cursor, so the copied cells lie
/// beyond the section width (`format-draw.c:268-270,371-373,479-481`).
fn merge_after(s: &mut [Screen], index: usize, width_after: u32, octx: &mut ScreenWriteCtx<'_>) {
    let (front, back) = s.split_at_mut(AFTER);
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut front[index],
        &mut sink,
        ScreenWritePolicy::default(),
        octx.registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.fast_copy(&back[0], 0, 0, width_after, 1);
    ctx.finish();
}

/// Write the buffered cells of one section through a screen-only context.
fn flush_cells(
    s: &mut [Screen],
    index: usize,
    cells: &mut Vec<GridCell>,
    octx: &mut ScreenWriteCtx<'_>,
) {
    if cells.is_empty() {
        return;
    }
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut s[index],
        &mut sink,
        ScreenWritePolicy::default(),
        octx.registry,
        #[cfg(feature = "sixel")]
        None,
    );
    for cell in cells.iter() {
        ctx.cell(cell);
    }
    ctx.finish();
    cells.clear();
}

/// `format_draw`: draw `expanded` into `available` columns at the cursor of
/// `octx`, restoring the cursor afterwards. Links are inserted into the
/// target screen store through `octx.registry`. Ranges are appended to
/// `ranges` with exclusive ends (`format-draw.c:689-1096`).
pub fn draw(
    octx: &mut ScreenWriteCtx<'_>,
    base: &GridCell,
    available: u32,
    expanded: &[u8],
    ranges: Option<&mut StyleRanges>,
    default_colours: bool,
) {
    let expanded = cstr(expanded);
    let size = expanded.len();
    let ocx = octx.screen.cx;
    let ocy = octx.screen.cy;

    let mut base_default = *base;
    let mut current_default = *base;
    let mut sy = Style::from_cell(current_default);

    let mut current = LEFT;
    let mut map = [LEFT, LEFT, CENTRE, RIGHT, ABSOLUTE_CENTRE];
    let mut focus_start: Option<u32> = None;
    let mut focus_end: Option<u32> = None;
    let mut list_state: Option<bool> = None; // None outside, Some(false) in list, Some(true) after
    let mut fill: Option<Colour> = None;
    let mut list_align = StyleAlign::Default;
    let mut fr: Option<Range> = None;
    let mut frs: Vec<Range> = Vec::new();

    // Eight one-row screens: left, centre, right and absolute centre
    // alignment, the list, anything after the list and the two list
    // markers.
    let width = u32::try_from(size).unwrap_or(u32::MAX).max(1);
    let mut s: Vec<Screen> = Vec::with_capacity(TOTAL);
    for _ in 0..TOTAL {
        let mut screen = Screen::new(width, 1, 0, ScreenResetPolicy::default(), octx.registry)
            .expect("hyperlink store creation cannot fail");
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            octx.registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.clearendofline(current_default.bg);
        ctx.finish();
        s.push(screen);
    }
    let mut cells: Vec<GridCell> = Vec::new();
    let mut aborted = false;

    // Walk the string and add to the corresponding screens, parsing styles
    // as we go.
    let mut cp = 0usize;
    while cp < size {
        let c = expanded[cp];
        let next = expanded.get(cp + 1).copied();

        // Handle sequences of #.
        if c == b'#' && next != Some(b'[') && next.is_some() {
            let n = expanded[cp..].iter().take_while(|&&b| b == b'#').count();
            let even = n % 2 == 0;
            if expanded.get(cp + n) != Some(&b'[') {
                cp += n;
                let count = if even { n / 2 } else { n / 2 + 1 };
                sy.gc.data = Utf8Data::set(b'#');
                cells.extend(std::iter::repeat_n(sy.gc, count));
                continue;
            }
            cp += if even { n + 1 } else { n - 1 };
            if sy.ignore {
                continue;
            }
            sy.gc.data = Utf8Data::set(b'#');
            cells.extend(std::iter::repeat_n(sy.gc, n / 2));
            if even {
                sy.gc.data = Utf8Data::set(b'[');
                cells.push(sy.gc);
            }
            continue;
        }

        // Is this not a style?
        if c != b'#' || next != Some(b'[') || sy.ignore {
            let mut done = None;
            if let Ok(mut ud) = Utf8Data::open(c) {
                let mut more = Utf8State::More;
                loop {
                    cp += 1;
                    if cp >= size || more != Utf8State::More {
                        break;
                    }
                    more = ud.append(expanded[cp]);
                }
                if more == Utf8State::Done {
                    done = Some(ud);
                } else {
                    cp -= usize::from(ud.have);
                }
            }
            let ud = match done {
                Some(ud) => ud,
                None => {
                    let b = expanded[cp];
                    if !(0x20..=0x7e).contains(&b) {
                        // Ignore nonprintable characters.
                        cp += 1;
                        continue;
                    }
                    cp += 1;
                    Utf8Data::set(b)
                }
            };
            sy.gc.data = ud;
            cells.push(sy.gc);
            continue;
        }

        // This is a style. Work out where the end is and parse it.
        let body = cp + 2;
        let Some(end) = skip(&expanded[body..], b"]").map(|e| body + e) else {
            aborted = true;
            break;
        };
        flush_cells(&mut s, current, &mut cells, octx);
        let saved_sy = sy;
        if sy
            .parse(&current_default, &expanded[body..end], octx.registry)
            .is_err()
        {
            cp = end + 1;
            continue;
        }
        if default_colours {
            sy.gc.bg = base_default.bg;
            sy.gc.fg = base_default.fg;
        }

        // Resolve any hyperlink and store it in the cell. The URI doubles as
        // the internal ID so repeated links share one entry.
        sy.gc.link = match octx.screen.hyperlinks.as_ref() {
            Some(store) => octx
                .registry
                .copy_style_link_to_store(&sy, store)
                .unwrap_or(HyperlinkId::NONE),
            None => HyperlinkId::NONE,
        };

        // If this style has a fill colour, store it for later.
        if sy.fill != Colour::DEFAULT {
            fill = Some(sy.fill);
        }

        // If this style pushed or popped the default, update it.
        match sy.default_type {
            StyleDefaultType::Push => {
                current_default = saved_sy.gc;
                sy.default_type = StyleDefaultType::Base;
            }
            StyleDefaultType::Pop => {
                current_default = base_default;
                sy.default_type = StyleDefaultType::Base;
            }
            StyleDefaultType::Set => {
                base_default = saved_sy.gc;
                current_default = saved_sy.gc;
                sy.default_type = StyleDefaultType::Base;
            }
            StyleDefaultType::Base => {}
        }

        // Check the list state.
        match sy.list {
            StyleList::On => {
                // Entering the list, exiting a marker, or exiting the focus.
                if list_state != Some(false) {
                    fr = None;
                    list_state = Some(false);
                    list_align = sy.align;
                }
                if focus_start.is_some() && focus_end.is_none() {
                    focus_end = Some(s[LIST].cx);
                }
                current = LIST;
            }
            StyleList::Focus => {
                if list_state == Some(false) && focus_start.is_none() {
                    focus_start = Some(s[LIST].cx);
                }
            }
            StyleList::Off => {
                if list_state == Some(false) {
                    fr = None;
                    if focus_start.is_some() && focus_end.is_none() {
                        focus_end = Some(s[LIST].cx);
                    }
                    map[list_align as usize] = AFTER;
                    if list_align == StyleAlign::Left {
                        map[StyleAlign::Default as usize] = AFTER;
                    }
                    list_state = Some(true);
                }
                current = map[sy.align as usize];
            }
            StyleList::LeftMarker => {
                if list_state == Some(false) && s[LIST_LEFT].cx == 0 {
                    fr = None;
                    if focus_start.is_some() && focus_end.is_none() {
                        focus_start = None;
                        focus_end = None;
                    }
                    current = LIST_LEFT;
                }
            }
            StyleList::RightMarker => {
                if list_state == Some(false) && s[LIST_RIGHT].cx == 0 {
                    fr = None;
                    if focus_start.is_some() && focus_end.is_none() {
                        focus_start = None;
                        focus_end = None;
                    }
                    current = LIST_RIGHT;
                }
            }
        }

        // Check if the range style has changed and if so end the current
        // range and start a new one if needed.
        if ranges.is_some() {
            if let Some(open) = fr.take() {
                if open.is_type(&sy) {
                    fr = Some(open);
                } else if s[current].cx != open.start {
                    frs.push(Range {
                        end: s[current].cx,
                        ..open
                    });
                }
            }
            if fr.is_none() && sy.range_type != StyleRangeType::None {
                fr = Some(Range {
                    index: current,
                    start: s[current].cx,
                    end: 0,
                    range_type: sy.range_type,
                    argument: sy.range_argument,
                    string: sy.range_string,
                });
            }
        }

        cp = end + 1;
    }

    if !aborted {
        flush_cells(&mut s, current, &mut cells, octx);

        // Clear the available area.
        if let Some(fill) = fill {
            let mut gc = DEFAULT_CELL;
            gc.bg = fill;
            gc.data = Utf8Data::set(b' ');
            for _ in 0..available {
                octx.cell(&gc);
            }
        }

        let focus = match (focus_start, focus_end) {
            (Some(start), Some(end)) => Some((start, end)),
            _ => None,
        };
        let mut target = Target { octx, ocx, ocy };
        match list_align {
            StyleAlign::Default => target.draw_none(available, &s, &mut frs),
            StyleAlign::Left => target.draw_left(available, &mut s, focus, &mut frs),
            StyleAlign::Centre => target.draw_centre(available, &mut s, focus, &mut frs),
            StyleAlign::Right => target.draw_right(available, &mut s, focus, &mut frs),
            StyleAlign::AbsoluteCentre => {
                target.draw_absolute_centre(available, &s, focus, &mut frs)
            }
        }

        if let Some(srs) = ranges {
            for fr in frs.drain(..) {
                srs.push(StyleRange {
                    range_type: fr.range_type,
                    argument: fr.argument,
                    string: fr.string,
                    start: fr.start,
                    end: fr.end,
                });
            }
        }
    }

    for mut screen in s {
        let _ = screen.release(
            octx.registry,
            #[cfg(feature = "sixel")]
            None,
        );
    }
    octx.cursormove(ocx as i32, ocy as i32, false);
}

/// `format_leading_hashes`: count a run of `#` and its drawn width
/// (`format-draw.c:647-674`). Returns the count, the width and the offset
/// to continue from: past the run, or at the `#` of `#[` when the run is
/// odd and introduces a style.
fn leading_hashes(s: &[u8], cp: usize) -> (usize, u32, usize) {
    let n = s[cp..].iter().take_while(|&&b| b == b'#').count();
    if n == 0 {
        return (0, 0, cp);
    }
    let half = (n / 2) as u32;
    if s.get(cp + n) != Some(&b'[') {
        let width = if n % 2 == 0 { half } else { half + 1 };
        return (n, width, cp + n);
    }
    if n % 2 == 0 {
        // An even number of #s means that all #s are escaped, so not a
        // style. Return pointing to the [.
        return (n, half, cp + n);
    }
    (n, half, cp + n - 1)
}

/// `utf8_open` plus `utf8_append` over `s` from the lead byte at `cp`.
/// Returns the decoded character (or the failed state) and the offset the C
/// scanner reaches: past the sequence or at its stopping byte.
fn open_utf8(s: &[u8], mut cp: usize) -> Option<(Utf8Data, Utf8State, usize)> {
    let mut ud = Utf8Data::open(s[cp]).ok()?;
    let mut more = Utf8State::More;
    loop {
        cp += 1;
        if cp >= s.len() || more != Utf8State::More {
            break;
        }
        more = ud.append(s[cp]);
    }
    Some((ud, more, cp))
}

/// `format_width`: the display width of `expanded`, ignoring style bodies;
/// zero when a style has no terminator (`format-draw.c:1099-1131`).
pub fn width(expanded: &[u8]) -> u32 {
    let s = cstr(expanded);
    let mut width = 0u32;
    let mut cp = 0usize;
    while cp < s.len() {
        let c = s[cp];
        if c == b'#' {
            let (_, leading_width, end) = leading_hashes(s, cp);
            width += leading_width;
            cp = end;
            if s.get(cp) == Some(&b'#') {
                let body = cp + 2;
                let Some(end) = skip(s.get(body..).unwrap_or(&[]), b"]") else {
                    return 0;
                };
                cp = body + end + 1;
            }
        } else if let Some((ud, more, next)) = open_utf8(s, cp) {
            cp = next;
            if more == Utf8State::Done {
                width += u32::from(ud.width);
            }
        } else if (0x20..0x7f).contains(&c) {
            width += 1;
            cp += 1;
        } else {
            cp += 1;
        }
    }
    width
}

fn push_hashes(out: &mut Vec<u8>, n: usize, count: u32) {
    if n == 1 {
        out.push(b'#');
    } else {
        out.extend(std::iter::repeat_n(b'#', 2 * count as usize));
    }
}

/// `format_trim_left`: the prefix of `expanded` that fits in `limit`
/// columns, keeping styles seen before the limit (`format-draw.c:1139-1196`).
pub fn trim_left(expanded: &[u8], limit: u32) -> ByteString {
    let s = cstr(expanded);
    let mut out = Vec::with_capacity(2 * s.len());
    let mut width = 0u32;
    let mut cp = 0usize;
    while cp < s.len() {
        if width >= limit {
            break;
        }
        let c = s[cp];
        if c == b'#' {
            let (n, mut leading_width, end) = leading_hashes(s, cp);
            if leading_width > limit - width {
                leading_width = limit - width;
            }
            if leading_width != 0 {
                push_hashes(&mut out, n, leading_width);
                width += leading_width;
            }
            cp = end;
            if s.get(cp) == Some(&b'#') {
                let body = cp + 2;
                let Some(end) = skip(s.get(body..).unwrap_or(&[]), b"]") else {
                    break;
                };
                let style_end = body + end + 1;
                out.extend_from_slice(&s[cp..style_end]);
                cp = style_end;
            }
        } else if let Some((ud, more, next)) = open_utf8(s, cp) {
            if more == Utf8State::Done {
                if width + u32::from(ud.width) <= limit {
                    out.extend_from_slice(ud.bytes());
                }
                width += u32::from(ud.width);
                cp = next;
            } else {
                cp = next - usize::from(ud.have) + 1;
            }
        } else if (0x20..0x7f).contains(&c) {
            if width < limit {
                out.push(c);
            }
            width += 1;
            cp += 1;
        } else {
            cp += 1;
        }
    }
    out.into()
}

/// `format_trim_right`: the suffix of `expanded` that fits in `limit`
/// columns, keeping styles; the whole input when it already fits
/// (`format-draw.c:1200-1266`).
pub fn trim_right(expanded: &[u8], limit: u32) -> ByteString {
    let s = cstr(expanded);
    let total_width = width(s);
    if total_width <= limit {
        return s.into();
    }
    let skip_width = total_width - limit;

    let mut out = Vec::with_capacity(2 * s.len());
    let mut width = 0u32;
    let mut cp = 0usize;
    while cp < s.len() {
        let c = s[cp];
        if c == b'#' {
            let (n, leading_width, end) = leading_hashes(s, cp);
            let mut copy_width = leading_width;
            if width <= skip_width {
                if skip_width - width >= copy_width {
                    copy_width = 0;
                } else {
                    copy_width -= skip_width - width;
                }
            }
            if copy_width != 0 {
                push_hashes(&mut out, n, copy_width);
            }
            width += leading_width;
            cp = end;
            if s.get(cp) == Some(&b'#') {
                let body = cp + 2;
                let Some(end) = skip(s.get(body..).unwrap_or(&[]), b"]") else {
                    break;
                };
                let style_end = body + end + 1;
                out.extend_from_slice(&s[cp..style_end]);
                cp = style_end;
            }
        } else if let Some((ud, more, next)) = open_utf8(s, cp) {
            if more == Utf8State::Done {
                if width >= skip_width {
                    out.extend_from_slice(ud.bytes());
                }
                width += u32::from(ud.width);
                cp = next;
            } else {
                cp = next - usize::from(ud.have) + 1;
            }
        } else if (0x20..0x7f).contains(&c) {
            if width >= skip_width {
                out.push(c);
            }
            width += 1;
            cp += 1;
        } else {
            cp += 1;
        }
    }
    out.into()
}

#[cfg(test)]
#[path = "draw/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "draw/differential.rs"]
mod differential;
