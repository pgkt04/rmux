// Ported from tmux window-copy.c @ 8f25579c (tests for the motion adapters
// and the scrollbar inverse geometry)
use super::*;
use crate::ids::ArenaId;
use crate::modes::copy::state::CopyBacking;
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_util::utf8::Utf8Data;

fn backing(rows: &[&[u8]], width: u32) -> CopyModeData {
    let mut screen = Screen::new(
        width,
        rows.len() as u32,
        100,
        ScreenResetPolicy::default(),
        &mut HyperlinkRegistry::new(),
    )
    .unwrap();
    for (y, bytes) in rows.iter().enumerate() {
        for (x, &byte) in bytes.iter().enumerate() {
            screen.grid.set_cell(
                x as u32,
                y as u32,
                &GridCell {
                    data: Utf8Data::set(byte),
                    ..DEFAULT_CELL
                },
            );
        }
    }
    CopyModeData::new(
        CopyBacking::Snapshot(screen),
        PaneId::from_parts(0, 0),
        ModeKeys::Emacs,
    )
}

#[test]
fn slider_offset_uses_single_precision_truncation() {
    // 5 * ((1000 + 24) / 24.0f) = 5 * 42.666668 = 213.33334 -> 213
    assert_eq!(slider_offset(5, 1000, 24), 213);
    assert_eq!(slider_offset(0, 1000, 24), 0);
    // 23 * 42.666668 = 981.3334 -> 981
    assert_eq!(slider_offset(23, 1000, 24), 981);
    assert_eq!(slider_offset(10, 100, 10), 110);
    // 7 * ((3 + 7) / 7.0f) = 7 * 1.4285715 = 10.000001 -> 10
    assert_eq!(slider_offset(7, 3, 7), 10);
    // Large histories exercise the float rounding: 100000 + 50 over 50.
    assert_eq!(
        slider_offset(49, 100_000, 50),
        (49.0f32 * (100_050.0f32 / 50.0f32)) as u32
    );
}

#[test]
fn page_rows_follow_window_copy_c() {
    assert_eq!(page_rows(false, 24), 22);
    assert_eq!(page_rows(true, 24), 12);
    assert_eq!(page_rows(true, 3), 1);
    assert_eq!(page_rows(false, 3), 1);
    assert_eq!(page_rows(false, 2), 1);
    assert_eq!(page_rows(true, 1), 1);
}

#[test]
fn find_length_and_in_set_read_the_backing_grid() {
    let data = backing(&[b"ab  ", b"\t x"], 6);
    assert_eq!(find_length(&data, 0), 2, "trailing spaces are trimmed");
    assert_eq!(find_length(&data, 1), 3);
    assert!(in_set(&data, 1, 1, WHITESPACE));
    assert!(in_set(&data, 0, 1, WHITESPACE));
    assert!(!in_set(&data, 2, 1, WHITESPACE));
    assert!(!in_set(&data, 0, 0, WHITESPACE));
}

#[test]
fn single_byte_skips_wide_and_padding_cells() {
    let mut data = backing(&[b"(x)"], 6);
    assert_eq!(single_byte(&data, 0, 0), Some(b'('));
    assert_eq!(single_byte(&data, 2, 0), Some(b')'));
    let mut wide = DEFAULT_CELL;
    wide.data = rmux_util::utf8::from_cstr("日".as_bytes()).0[0];
    data.backing.screen_mut().grid.set_cell(1, 0, &wide);
    assert_eq!(
        single_byte(&data, 1, 0),
        None,
        "multi-byte cells never match"
    );
    let mut padding = DEFAULT_CELL;
    padding.flags = padding.flags | rmux_emu::cell::GridCellFlags::PADDING;
    data.backing.screen_mut().grid.set_cell(2, 0, &padding);
    assert_eq!(single_byte(&data, 2, 0), None, "padding cells never match");
}

#[test]
fn reader_origin_uses_backing_coordinates() {
    let mut data = backing(&[b"a", b"b", b"c"], 4);
    data.cx = 1;
    data.cy = 2;
    data.oy = 0;
    assert_eq!(reader_origin(&data), (0, 0, 2, 1, 2));
}
