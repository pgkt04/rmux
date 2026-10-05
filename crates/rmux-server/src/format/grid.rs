// Ported from tmux format.c @ 8f25579c
/*
 * Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

use rmux_emu::{
    cell::{GridCell, GridCellFlags},
    grid::{Grid, GridLineFlags},
    hyperlinks::HyperlinkRegistry,
    screen::Screen,
};
use rmux_util::{
    bytes::{ByteString, cstr},
    utf8,
};

pub fn word(grid: &Grid, mut x: u32, mut y: u32, separators: &[u8]) -> Option<ByteString> {
    grid.peek_line(y)?;
    let separators = utf8::from_cstr(separators);
    let is_separator = |cell: &GridCell| {
        separators.contains(&cell.data)
            || cell.flags.contains(GridCellFlags::TAB)
            || cell.data.bytes() == b" "
    };
    let mut found = false;
    loop {
        let cell = grid.get_cell(x, y);
        if !cell.flags.contains(GridCellFlags::PADDING) && is_separator(&cell) {
            found = true;
            break;
        }
        if x == 0 {
            if y == 0 || !grid.get_line(y - 1).flags.contains(GridLineFlags::WRAPPED) {
                break;
            }
            y -= 1;
            x = grid.line_length(y);
            if x == 0 {
                break;
            }
        }
        x -= 1;
    }
    let mut result = Vec::new();
    let mut cells = 0;
    loop {
        if found {
            let end = grid.line_length(y);
            if end == 0 || x == end - 1 {
                if y == grid.hsize() + grid.sy() - 1
                    || !grid.get_line(y).flags.contains(GridLineFlags::WRAPPED)
                {
                    break;
                }
                y += 1;
                x = 0;
            } else {
                x += 1;
            }
        }
        found = true;
        let cell = grid.get_cell(x, y);
        if cell.flags.contains(GridCellFlags::PADDING) {
            continue;
        }
        if is_separator(&cell) {
            break;
        }
        result.extend_from_slice(cell.data.bytes());
        cells += 1;
    }
    if cells == 0 {
        return None;
    }
    result.truncate(cstr(&result).len());
    Some(ByteString(result))
}

pub fn line(grid: &Grid, y: u32) -> Option<ByteString> {
    grid.peek_line(y)?;
    let mut result = Vec::new();
    let mut cells = 0;
    for x in 0..grid.line_length(y) {
        let cell = grid.get_cell(x, y);
        if cell.flags.contains(GridCellFlags::PADDING) {
            continue;
        }
        if cell.flags.contains(GridCellFlags::TAB) {
            result.push(b'\t');
        } else {
            result.extend_from_slice(cell.data.bytes());
        }
        cells += 1;
    }
    if cells == 0 {
        return None;
    }
    result.truncate(cstr(&result).len());
    Some(ByteString(result))
}

pub fn hyperlink(
    grid: &Grid,
    mut x: u32,
    y: u32,
    screen: &Screen,
    registry: &HyperlinkRegistry,
) -> Option<ByteString> {
    grid.peek_line(y)?;
    let cell = loop {
        let cell = grid.get_cell(x, y);
        if !cell.flags.contains(GridCellFlags::PADDING) {
            break cell;
        }
        if x == 0 {
            return None;
        }
        x -= 1;
    };
    let store = screen.hyperlinks.as_ref()?;
    let uri = registry.get(store, cell.link)?;
    Some(ByteString::from(cstr(uri.uri())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmux_emu::screen::ScreenResetPolicy;
    use rmux_util::utf8::Utf8Data;

    fn text(grid: &mut Grid, y: u32, value: &[u8]) {
        for (x, &byte) in value.iter().enumerate() {
            let cell = GridCell {
                data: Utf8Data::set(byte),
                ..GridCell::default()
            };
            grid.set_cell(x as u32, y, &cell);
        }
    }

    #[test]
    fn word_crosses_only_wrapped_rows() {
        let mut grid = Grid::new(5, 2, 0);
        text(&mut grid, 0, b"hello");
        text(&mut grid, 1, b"world");
        assert_eq!(word(&grid, 2, 1, b"-"), Some(ByteString::from("world")));
        grid.get_line_mut(0).flags.insert(GridLineFlags::WRAPPED);
        assert_eq!(
            word(&grid, 2, 1, b"-"),
            Some(ByteString::from("helloworld"))
        );
    }

    #[test]
    fn word_separator_selects_following_word() {
        let mut grid = Grid::new(12, 1, 0);
        text(&mut grid, 0, b"one-two end");
        assert_eq!(word(&grid, 3, 0, b"-"), Some(ByteString::from("two")));
        assert_eq!(word(&grid, 7, 0, b"-"), Some(ByteString::from("end")));
        assert_eq!(word(&grid, 11, 0, b"-"), None);
    }

    #[test]
    fn line_restores_tabs_skips_padding_and_trims_spaces() {
        let mut grid = Grid::new(6, 1, 0);
        text(&mut grid, 0, b"a   b ");
        let tab = GridCell {
            flags: GridCellFlags::TAB,
            data: Utf8Data::set(b' '),
            ..GridCell::default()
        };
        grid.set_cell(1, 0, &tab);
        let padding = GridCell {
            flags: GridCellFlags::PADDING,
            ..GridCell::default()
        };
        grid.set_cell(2, 0, &padding);
        grid.set_cell(3, 0, &padding);
        assert_eq!(line(&grid, 0), Some(ByteString::from("a\tb")));
        assert_eq!(line(&Grid::new(2, 1, 0), 0), None);
    }

    #[test]
    fn hyperlink_moves_left_over_padding_and_rejects_expired_link() {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = Screen::new(4, 1, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let store = screen.hyperlinks.as_ref().unwrap();
        let link = registry
            .put(store, b"https://example.test", Some(b"id"))
            .unwrap();
        let cell = GridCell {
            link,
            data: Utf8Data::set(b'x'),
            ..GridCell::default()
        };
        screen.grid.set_cell(0, 0, &cell);
        let padding = GridCell {
            flags: GridCellFlags::PADDING,
            ..GridCell::default()
        };
        screen.grid.set_cell(1, 0, &padding);
        assert_eq!(
            hyperlink(&screen.grid, 1, 0, &screen, &registry),
            Some(ByteString::from("https://example.test"))
        );
        registry.reset(screen.hyperlinks.as_ref().unwrap()).unwrap();
        assert_eq!(hyperlink(&screen.grid, 1, 0, &screen, &registry), None);
    }
    #[test]
    fn word_handles_utf8_separator_and_wide_padding() {
        let mut grid = Grid::new(8, 1, 0);
        let mut separator = Utf8Data {
            size: 2,
            have: 2,
            width: 1,
            ..Utf8Data::default()
        };
        separator.data[..2].copy_from_slice(b"\xc3\xa9");
        grid.set_cell(
            0,
            0,
            &GridCell {
                data: separator,
                ..GridCell::default()
            },
        );
        let mut wide = Utf8Data {
            size: 3,
            have: 3,
            width: 2,
            ..Utf8Data::default()
        };
        wide.data[..3].copy_from_slice(b"\xe4\xb8\xad");
        grid.set_cell(
            1,
            0,
            &GridCell {
                data: wide,
                ..GridCell::default()
            },
        );
        grid.set_cell(
            2,
            0,
            &GridCell {
                flags: GridCellFlags::PADDING,
                ..GridCell::default()
            },
        );
        grid.set_cell(
            3,
            0,
            &GridCell {
                data: Utf8Data::set(b'x'),
                ..GridCell::default()
            },
        );
        assert_eq!(
            word(&grid, 0, 0, b"\xc3\xa9"),
            Some(ByteString::from(b"\xe4\xb8\xadx".as_slice()))
        );
        assert_eq!(
            word(&grid, 2, 0, b"\xc3\xa9"),
            Some(ByteString::from(b"\xe4\xb8\xadx".as_slice()))
        );
    }

    #[test]
    fn line_obeys_first_nul_and_invalid_row_returns_none() {
        let mut grid = Grid::new(4, 1, 0);
        text(&mut grid, 0, b"x\0yz");
        assert_eq!(line(&grid, 0), Some(ByteString::from("x")));
        assert_eq!(line(&grid, 1), None);
        assert_eq!(word(&grid, 0, 1, b"-"), None);
    }
}
