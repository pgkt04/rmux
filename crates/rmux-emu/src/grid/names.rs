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
//! Flag and attribute names for `capture-pane -R` and logging
//! (`grid.c:1741-1839`).

use super::GridLineFlags;
use crate::cell::{GridAttributes, GridCellFlags};

fn join(names: &[(bool, &str)]) -> String {
    let mut s = String::new();
    for &(set, name) in names {
        if set {
            if !s.is_empty() {
                s.push(',');
            }
            s.push_str(name);
        }
    }
    if s.is_empty() {
        s.push_str("NONE");
    }
    s
}

/// `grid_line_flags_string` (`grid.c:1741-1769`).
pub fn line_flags_string(flags: GridLineFlags) -> String {
    let has = |f| flags.intersects(f);
    join(&[
        (has(GridLineFlags::WRAPPED), "WRAPPED"),
        (has(GridLineFlags::EXTENDED), "EXTENDED"),
        (has(GridLineFlags::DEAD), "DEAD"),
        (has(GridLineFlags::START_PROMPT), "START_PROMPT"),
        (has(GridLineFlags::SECOND_PROMPT), "SECOND_PROMPT"),
        (has(GridLineFlags::START_COMMAND), "START_COMMAND"),
        (has(GridLineFlags::START_OUTPUT), "START_OUTPUT"),
        (has(GridLineFlags::END_OUTPUT), "END_OUTPUT"),
        (has(GridLineFlags::HYPERLINK), "HYPERLINK"),
    ])
}

/// `grid_cell_flags_string` (`grid.c:1772-1798`).
pub fn cell_flags_string(flags: GridCellFlags) -> String {
    let has = |f| flags.intersects(f);
    join(&[
        (has(GridCellFlags::FG256), "FG256"),
        (has(GridCellFlags::BG256), "BG256"),
        (has(GridCellFlags::PADDING), "PADDING"),
        (has(GridCellFlags::EXTENDED), "EXTENDED"),
        (has(GridCellFlags::SELECTED), "SELECTED"),
        (has(GridCellFlags::CLEARED), "CLEARED"),
        (has(GridCellFlags::TAB), "TAB"),
        (has(GridCellFlags::NOPALETTE), "NOPALETTE"),
    ])
}

/// `grid_cell_attr_string` (`grid.c:1801-1839`).
pub fn cell_attr_string(attr: GridAttributes) -> String {
    let has = |f| attr.intersects(f);
    join(&[
        (has(GridAttributes::CHARSET), "CHARSET"),
        (has(GridAttributes::BRIGHT), "BRIGHT"),
        (has(GridAttributes::DIM), "DIM"),
        (has(GridAttributes::UNDERSCORE), "UNDERSCORE"),
        (has(GridAttributes::BLINK), "BLINK"),
        (has(GridAttributes::REVERSE), "REVERSE"),
        (has(GridAttributes::HIDDEN), "HIDDEN"),
        (has(GridAttributes::ITALICS), "ITALICS"),
        (has(GridAttributes::STRIKETHROUGH), "STRIKETHROUGH"),
        (has(GridAttributes::UNDERSCORE_2), "UNDERSCORE_2"),
        (has(GridAttributes::UNDERSCORE_3), "UNDERSCORE_3"),
        (has(GridAttributes::UNDERSCORE_4), "UNDERSCORE_4"),
        (has(GridAttributes::UNDERSCORE_5), "UNDERSCORE_5"),
        (has(GridAttributes::OVERLINE), "OVERLINE"),
    ])
}
