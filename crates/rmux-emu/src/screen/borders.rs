// Ported from tmux tty-acs.c, screen-write.c and tmux.h @ 8f25579c
/*
 * Copyright (c) 2010 Nicholas Marriott <nicholas.marriott@gmail.com>
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

use super::{BorderCell, BoxLines};
use crate::cell::{GridAttributes, GridCell};
use rmux_util::utf8::Utf8Data;

const CELL_BORDERS: &[u8; 14] = b" xqlkmjwvtun~\0";
const SIMPLE_BORDERS: &[u8; 14] = b" |-+++++++++.\0";
const PADDED_BORDERS: &[u8; 14] = b"             \0";

const fn glyph(bytes: &[u8]) -> Utf8Data {
    let mut data = [0; 32];
    let mut i = 0;
    while i < bytes.len() {
        data[i] = bytes[i];
        i += 1;
    }
    Utf8Data {
        data,
        have: 0,
        size: bytes.len() as u8,
        width: if bytes.is_empty() { 0 } else { 1 },
    }
}

const DOUBLE: [Utf8Data; 13] = [
    glyph(b""),
    glyph("║".as_bytes()),
    glyph("═".as_bytes()),
    glyph("╔".as_bytes()),
    glyph("╗".as_bytes()),
    glyph("╚".as_bytes()),
    glyph("╝".as_bytes()),
    glyph("╦".as_bytes()),
    glyph("╩".as_bytes()),
    glyph("╠".as_bytes()),
    glyph("╣".as_bytes()),
    glyph("╬".as_bytes()),
    glyph("·".as_bytes()),
];
const HEAVY: [Utf8Data; 13] = [
    glyph(b""),
    glyph("┃".as_bytes()),
    glyph("━".as_bytes()),
    glyph("┏".as_bytes()),
    glyph("┓".as_bytes()),
    glyph("┗".as_bytes()),
    glyph("┛".as_bytes()),
    glyph("┳".as_bytes()),
    glyph("┻".as_bytes()),
    glyph("┣".as_bytes()),
    glyph("┫".as_bytes()),
    glyph("╋".as_bytes()),
    glyph("·".as_bytes()),
];
const ROUNDED: [Utf8Data; 13] = [
    glyph(b""),
    glyph("│".as_bytes()),
    glyph("─".as_bytes()),
    glyph("╭".as_bytes()),
    glyph("╮".as_bytes()),
    glyph("╰".as_bytes()),
    glyph("╯".as_bytes()),
    glyph("┳".as_bytes()),
    glyph("┻".as_bytes()),
    glyph("├".as_bytes()),
    glyph("┤".as_bytes()),
    glyph("╋".as_bytes()),
    glyph("·".as_bytes()),
];

/// Select a border glyph without changing the cell's other rendition fields.
/// Unicode styles accept Inside through None; Scrollbar is not a border glyph.
pub fn border_cell(style: BoxLines, index: BorderCell, cell: &mut GridCell) {
    let index = index as usize;
    match style {
        BoxLines::None => {}
        BoxLines::Double | BoxLines::Heavy | BoxLines::Rounded => {
            cell.attr.remove(GridAttributes::CHARSET);
            let table = match style {
                BoxLines::Double => &DOUBLE,
                BoxLines::Heavy => &HEAVY,
                BoxLines::Rounded => &ROUNDED,
                _ => unreachable!(),
            };
            cell.data.copy_from(&table[index]);
        }
        BoxLines::Simple | BoxLines::Padded => {
            cell.attr.remove(GridAttributes::CHARSET);
            let table = if style == BoxLines::Simple {
                SIMPLE_BORDERS
            } else {
                PADDED_BORDERS
            };
            cell.data = Utf8Data::set(table[index]);
        }
        BoxLines::Single | BoxLines::Default => {
            cell.attr.insert(GridAttributes::CHARSET);
            cell.data = Utf8Data::set(CELL_BORDERS[index]);
        }
    }
}

/// The pure UTF-8 fallback of tty_acs_get(NULL, ch), without a trailing NUL.
pub fn acs(ch: u8) -> Option<&'static [u8]> {
    Some(match ch {
        b'+' => "→".as_bytes(),
        b',' => "←".as_bytes(),
        b'-' => "↑".as_bytes(),
        b'.' => "↓".as_bytes(),
        b'0' => "▮".as_bytes(),
        b'`' => "◆".as_bytes(),
        b'a' => "▒".as_bytes(),
        b'b' => "␉".as_bytes(),
        b'c' => "␌".as_bytes(),
        b'd' => "␍".as_bytes(),
        b'e' => "␊".as_bytes(),
        b'f' => "°".as_bytes(),
        b'g' => "±".as_bytes(),
        b'h' => "␤".as_bytes(),
        b'i' => "␋".as_bytes(),
        b'j' => "┘".as_bytes(),
        b'k' => "┐".as_bytes(),
        b'l' => "┌".as_bytes(),
        b'm' => "└".as_bytes(),
        b'n' => "┼".as_bytes(),
        b'o' => "⎺".as_bytes(),
        b'p' => "⎻".as_bytes(),
        b'q' => "─".as_bytes(),
        b'r' => "⎼".as_bytes(),
        b's' => "⎽".as_bytes(),
        b't' => "├".as_bytes(),
        b'u' => "┤".as_bytes(),
        b'v' => "┴".as_bytes(),
        b'w' => "┬".as_bytes(),
        b'x' => "│".as_bytes(),
        b'y' => "≤".as_bytes(),
        b'z' => "≥".as_bytes(),
        b'{' => "π".as_bytes(),
        b'|' => "≠".as_bytes(),
        b'}' => "£".as_bytes(),
        b'~' => "·".as_bytes(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::GridCellFlags;
    use crate::colour::Colour;

    fn styled_cell() -> GridCell {
        GridCell {
            data: Utf8Data::set(b'Z'),
            attr: GridAttributes::BRIGHT | GridAttributes::CHARSET,
            flags: GridCellFlags::NOPALETTE | GridCellFlags::SELECTED,
            fg: Colour(2),
            bg: Colour(4),
            ..GridCell::default()
        }
    }

    #[test]
    fn none_preserves_the_entire_cell_for_every_index() {
        for index in 0..=13 {
            let mut cell = styled_cell();
            let original = cell;
            border_cell(
                BoxLines::None,
                BorderCell::try_from(index).unwrap(),
                &mut cell,
            );
            assert_eq!(cell, original);
        }
    }

    #[test]
    fn every_border_style_matches_pinned_tables() {
        let styles = [
            (BoxLines::Default, " xqlkmjwvtun~"),
            (BoxLines::Single, " xqlkmjwvtun~"),
            (BoxLines::Double, "\0║═╔╗╚╝╦╩╠╣╬·"),
            (BoxLines::Heavy, "\0┃━┏┓┗┛┳┻┣┫╋·"),
            (BoxLines::Rounded, "\0│─╭╮╰╯┳┻├┤╋·"),
            (BoxLines::Simple, " |-+++++++++."),
            (BoxLines::Padded, "             "),
        ];
        for (style, expected) in styles {
            for (index, ch) in expected.chars().enumerate() {
                let mut cell = styled_cell();
                let original = cell;
                border_cell(
                    style,
                    BorderCell::try_from(index as i32).unwrap(),
                    &mut cell,
                );
                let charset = matches!(style, BoxLines::Default | BoxLines::Single);
                let unicode = matches!(
                    style,
                    BoxLines::Double | BoxLines::Heavy | BoxLines::Rounded
                );
                let mut bytes = [0; 4];
                let expected = if unicode && index == 0 {
                    b"".as_slice()
                } else {
                    ch.encode_utf8(&mut bytes).as_bytes()
                };
                assert_eq!(&cell.data.data[..usize::from(cell.data.size)], expected);
                assert!(
                    cell.data.data[usize::from(cell.data.size)..]
                        .iter()
                        .all(|b| *b == 0)
                );
                assert_eq!(cell.data.have, u8::from(!unicode));
                assert_eq!(cell.data.width, u8::from(!expected.is_empty()));
                assert_eq!(cell.attr.contains(GridAttributes::CHARSET), charset);
                cell.data = original.data;
                cell.attr = original.attr;
                assert_eq!(cell, original);
            }
        }
    }

    #[test]
    fn byte_tables_include_the_c_string_terminator() {
        for style in [
            BoxLines::Default,
            BoxLines::Single,
            BoxLines::Simple,
            BoxLines::Padded,
        ] {
            let mut cell = styled_cell();
            border_cell(style, BorderCell::Scrollbar, &mut cell);
            assert_eq!(cell.data, Utf8Data::set(0));
        }
    }

    #[test]
    fn acs_matches_every_pinned_fallback_and_rejects_other_bytes() {
        let keys = b"+,-.0`abcdefghijklmnopqrstuvwxyz{|}~";
        let values = "→←↑↓▮◆▒␉␌␍␊°±␤␋┘┐┌└┼⎺⎻─⎼⎽├┤┴┬│≤≥π≠£·";
        assert_eq!(keys.len(), values.chars().count());
        for (&key, value) in keys.iter().zip(values.chars()) {
            let mut bytes = [0; 4];
            assert_eq!(acs(key), Some(value.encode_utf8(&mut bytes).as_bytes()));
        }
        for ch in 0..=u8::MAX {
            if !keys.contains(&ch) {
                assert_eq!(acs(ch), None);
            }
        }
    }
}
