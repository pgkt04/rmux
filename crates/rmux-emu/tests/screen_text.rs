// Ported from tmux screen-write.c @ 8f25579c
use rmux_emu::screen::write::ScreenWriteCtx;

#[test]
fn strlen_counts_columns_and_filters_controls() {
    assert_eq!(ScreenWriteCtx::strlen(b"a\t\x01\nb\x7f"), 3);
    assert_eq!(ScreenWriteCtx::strlen("A界e\u{301}".as_bytes()), 4);
    assert_eq!(ScreenWriteCtx::strlen(b"abc\0ignored"), 3);
}

#[test]
fn strlen_stops_on_incomplete_utf8_and_consumes_invalid_sequences() {
    assert_eq!(ScreenWriteCtx::strlen(b"a\xe7\x95"), 1);
    assert_eq!(ScreenWriteCtx::strlen(b"a\xe7xyb"), 2);
    assert_eq!(ScreenWriteCtx::strlen(b"a\x80b"), 2);
}

use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCellFlags};
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWritePolicy};
use rmux_emu::screen::{BoxLines, Screen, ScreenMode, ScreenResetPolicy};

#[test]
fn simple_text_limits_charset_and_newlines() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(12, 5, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
    );
    ctx.nputs(2, &DEFAULT_CELL, "a界z".as_bytes());
    assert_eq!(ctx.screen.cx, 2);
    assert_eq!(ctx.screen.grid.view_get_cell(1, 0).data.bytes(), b" ");
    ctx.puts(&DEFAULT_CELL, b"\n\x01q\x01r");
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 1));
    assert!(
        ctx.screen
            .grid
            .view_get_cell(0, 1)
            .attr
            .contains(GridAttributes::CHARSET)
    );
    assert!(
        !ctx.screen
            .grid
            .view_get_cell(1, 1)
            .attr
            .contains(GridAttributes::CHARSET)
    );
    ctx.nputs(0, &DEFAULT_CELL, b"abc");
    assert_eq!(ctx.screen.cx, 5);
    ctx.finish();
}

#[test]
fn wrapped_text_preserves_more_and_final_line_rule() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(12, 5, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
    );
    assert!(ctx.text(0, 5, 3, true, &DEFAULT_CELL, b"abc def"));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (3, 1));
    assert_eq!(ctx.screen.grid.view_get_cell(0, 1).data.bytes(), b"d");
    assert!(!ctx.text(0, 5, 1, false, &DEFAULT_CELL, b"x"));
    ctx.finish();
}

#[test]
fn outlines_restore_cursor_and_set_palette_policy() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(12, 6, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
    );
    ctx.draw_box(5, 4, BoxLines::Simple, None);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 0));
    for (x, y, byte) in [(0, 0, b'+'), (4, 3, b'+'), (2, 0, b'-'), (0, 2, b'|')] {
        let cell = ctx.screen.grid.view_get_cell(x, y);
        assert_eq!(cell.data.bytes(), &[byte]);
        assert!(cell.flags.contains(GridCellFlags::NOPALETTE));
    }
    ctx.hline(5, true, true, BoxLines::Single, None);
    assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.bytes(), b"t");
    assert_eq!(ctx.screen.grid.view_get_cell(4, 0).data.bytes(), b"u");
    ctx.vline(4, true, true, None);
    assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.bytes(), b"w");
    assert_eq!(ctx.screen.grid.view_get_cell(0, 3).data.bytes(), b"v");
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 0));
    ctx.finish();
}

#[test]
fn copy_avoids_split_wide_cells_and_preview_advances_cursor() {
    let mut registry = HyperlinkRegistry::new();
    let mut src = Screen::new(12, 6, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = ScreenOnlySink;
    {
        let mut ctx = ScreenWriteCtx::start(
            &mut src,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
        );
        ctx.puts(&DEFAULT_CELL, "ab界".as_bytes());
        ctx.finish();
    }
    let mut dst = Screen::new(12, 6, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut ctx = ScreenWriteCtx::start(
        &mut dst,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
    );
    ctx.fast_copy(&src, 0, 0, 3, 1);
    assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.bytes(), b"a");
    assert_eq!(ctx.screen.grid.view_get_cell(2, 0).data.bytes(), b" ");
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 0));
    src.cx = 1;
    ctx.preview(&src, 6, 3);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 0));
    assert!(
        ctx.screen
            .grid
            .view_get_cell(1, 0)
            .attr
            .contains(GridAttributes::REVERSE)
    );
    ctx.mode_clear(ScreenMode::CURSOR);
    assert!(!ctx.screen.mode.contains(ScreenMode::CURSOR));
    ctx.mode_set(ScreenMode::CURSOR);
    assert!(ctx.screen.mode.contains(ScreenMode::CURSOR));
    ctx.finish();
}

#[test]
fn fast_copy_writes_first_hidden_cell_then_stops_row() {
    use rmux_emu::screen::write::{DrawOp, DrawSnapshot, ScreenRenderEffects, TtySink};
    use std::ops::Range;
    struct HiddenSink;
    impl TtySink for HiddenSink {
        fn draw(&mut self, _: DrawOp<'_>, _: &DrawSnapshot) {}
        fn visible_columns(&mut self, _: u32, _: u32, _: u32, out: &mut Vec<Range<u32>>) {
            out.clear();
        }
        fn obscured(&mut self) -> bool {
            true
        }
        fn redraw_pending(&self) -> bool {
            false
        }
        fn effect(&mut self, _: ScreenRenderEffects, _: &Screen) {}
        fn begin_write(&mut self) {}
    }
    let mut registry = HyperlinkRegistry::new();
    let mut src = Screen::new(4, 2, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut cell = DEFAULT_CELL;
    cell.data = rmux_util::utf8::Utf8Data::set(b'a');
    src.grid.view_set_cell(0, 0, &cell);
    cell.data = rmux_util::utf8::Utf8Data::set(b'b');
    src.grid.view_set_cell(1, 0, &cell);
    let mut dst = Screen::new(4, 2, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = HiddenSink;
    let mut ctx = ScreenWriteCtx::start(
        &mut dst,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
    );
    ctx.fast_copy(&src, 0, 0, 4, 1);
    assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.bytes(), b"a");
    assert_eq!(ctx.screen.grid.view_get_cell(1, 0).data.bytes(), b" ");
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 0));
    ctx.finish();
}
