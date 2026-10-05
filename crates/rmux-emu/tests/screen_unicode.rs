// Ported from tmux screen-write.c @ 8f25579c
use std::ops::Range;

use rmux_emu::cell::{DEFAULT_CELL, GridCell, GridCellFlags};
use rmux_emu::colour::Colour;
use rmux_emu::grid::GridLineFlags;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::write::{
    DrawCommand, DrawOp, DrawSnapshot, ScreenRenderEffects, ScreenWriteCtx, ScreenWritePolicy,
    TtySink,
};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_util::utf8::Utf8Data;

#[derive(Default)]
struct RecordingSink {
    cells: Vec<(GridCell, DrawSnapshot, (u32, u32))>,
    inserts: Vec<(u32, DrawSnapshot)>,
    redraws: Vec<u32>,
    visible: Option<Range<u32>>,
}

impl TtySink for RecordingSink {
    fn draw(&mut self, op: DrawOp<'_>, snapshot: &DrawSnapshot) {
        match op.command {
            DrawCommand::Cell(cell) => {
                self.cells
                    .push((*cell, *snapshot, (op.screen.cx, op.screen.cy)));
            }
            DrawCommand::InsertCharacter { count, .. } => self.inserts.push((count, *snapshot)),
            DrawCommand::RedrawLine { row, .. } => self.redraws.push(row),
            _ => {}
        }
    }

    fn visible_columns(&mut self, x: u32, _: u32, n: u32, out: &mut Vec<Range<u32>>) {
        out.clear();
        let start = self.visible.as_ref().map_or(x, |range| x.max(range.start));
        let end = self
            .visible
            .as_ref()
            .map_or(x + n, |range| (x + n).min(range.end));
        if start < end {
            out.push(start..end);
        }
    }

    fn obscured(&mut self) -> bool {
        self.visible.is_some()
    }
    fn redraw_pending(&self) -> bool {
        false
    }
    fn effect(&mut self, _: ScreenRenderEffects, _: &Screen) {}
    fn begin_write(&mut self) {}
}

fn cell(text: &str, width: u8) -> GridCell {
    let mut data = Utf8Data {
        have: text.len() as u8,
        size: text.len() as u8,
        width,
        ..Utf8Data::default()
    };
    data.data[..text.len()].copy_from_slice(text.as_bytes());
    GridCell {
        data,
        ..DEFAULT_CELL
    }
}

fn screen(width: u32, registry: &mut HyperlinkRegistry) -> Screen {
    Screen::new(width, 3, 0, ScreenResetPolicy::default(), registry).unwrap()
}

#[test]
fn dependent_input_and_padding_at_left_are_discarded() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    for input in [
        cell("\u{200d}", 0),
        cell("\u{fe0f}", 0),
        cell("\u{301}", 0),
        cell("\u{3164}", 2),
    ] {
        ctx.cell(&input);
    }
    let mut padding = cell("x", 1);
    padding.flags.insert(GridCellFlags::PADDING);
    ctx.cell(&padding);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 0));
    ctx.finish();
    assert!(sink.cells.is_empty());
}

#[test]
fn accents_combine_after_narrow_and_wide_cells_without_advancing() {
    for (base, width) in [("a", 1), ("界", 2)] {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = screen(8, &mut registry);
        let mut sink = RecordingSink::default();
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.cell(&cell(base, width));
        ctx.cell(&cell("\u{301}", 0));
        let stored = ctx.screen.grid.view_get_cell(0, 0);
        assert_eq!(stored.data.bytes(), format!("{base}\u{301}").as_bytes());
        assert_eq!(stored.data.width, width);
        assert_eq!(ctx.screen.cx, u32::from(width));
        ctx.finish();
        let (_, snapshot, cursor) = sink.cells.last().unwrap();
        assert_eq!(snapshot.old_cx, 0);
        assert_eq!(*cursor, (0, 0));
        assert!(!snapshot.invalidate_cursor);
    }
}

#[test]
fn variation_selector_policy_controls_padding_and_invalidation() {
    for wide in [false, true] {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = screen(8, &mut registry);
        let mut sink = RecordingSink::default();
        let policy = ScreenWritePolicy {
            variation_selector_always_wide: wide,
            ..ScreenWritePolicy::default()
        };
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            policy,
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        let mut base = cell("♥", 1);
        base.bg = Colour(4);
        ctx.cell(&base);
        ctx.cell(&cell("\u{fe0f}", 0));
        assert_eq!(ctx.screen.cx, if wide { 2 } else { 1 });
        assert_eq!(
            ctx.screen.grid.view_get_cell(0, 0).data.width,
            if wide { 2 } else { 1 }
        );
        if wide {
            let padding = ctx.screen.grid.view_get_cell(1, 0);
            assert!(padding.flags.contains(GridCellFlags::PADDING));
            assert_eq!(padding.bg, base.bg);
        }
        ctx.finish();
        assert_eq!(sink.cells.last().unwrap().1.invalidate_cursor, wide);
    }
}

#[test]
fn partly_hidden_forced_combination_changes_grid_but_not_cursor_or_output() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let mut sink = RecordingSink {
        visible: Some(0..1),
        ..RecordingSink::default()
    };
    let policy = ScreenWritePolicy {
        variation_selector_always_wide: true,
        pane_backed: true,
        ..ScreenWritePolicy::default()
    };
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        policy,
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("♥", 1));
    ctx.cell(&cell("\u{fe0f}", 0));
    assert_eq!(ctx.screen.cx, 1);
    assert_eq!(
        ctx.screen.grid.view_get_cell(0, 0).data.bytes(),
        "♥\u{fe0f}".as_bytes()
    );
    assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.width, 2);
    assert!(
        ctx.screen
            .grid
            .view_get_cell(1, 0)
            .flags
            .contains(GridCellFlags::PADDING)
    );
    ctx.finish();
    assert_eq!(sink.cells.len(), 1);
}

#[test]
fn forced_combination_at_last_column_preserves_pinned_cursor_clamp() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(4, &mut registry);
    screen.cx = 3;
    let mut sink = RecordingSink::default();
    let policy = ScreenWritePolicy {
        variation_selector_always_wide: true,
        ..ScreenWritePolicy::default()
    };
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        policy,
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("♥", 1));
    assert_eq!(ctx.screen.cx, 4);
    ctx.cell(&cell("\u{fe0f}", 0));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (3, 0));
    assert_eq!(ctx.screen.grid.view_get_cell(3, 0).data.width, 2);
    ctx.finish();
    assert!(sink.cells.last().unwrap().1.invalidate_cursor);
}

#[test]
fn zwj_modifier_and_regional_pairs_use_pinned_combination_rules() {
    for (parts, expected, width) in [
        (
            vec![("👩", 2), ("\u{200d}", 0), ("💻", 2)],
            "👩\u{200d}💻",
            2,
        ),
        (vec![("👋", 1), ("🏻", 2)], "👋🏻", 2),
        (vec![("🏻", 2), ("👋", 1)], "🏻👋", 2),
        (vec![("🇩", 1), ("🇪", 1)], "🇩🇪", 2),
    ] {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = screen(12, &mut registry);
        let mut sink = RecordingSink::default();
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        for (text, columns) in parts {
            ctx.cell(&cell(text, columns));
        }
        assert_eq!(
            ctx.screen.grid.view_get_cell(0, 0).data.bytes(),
            expected.as_bytes()
        );
        assert_eq!(ctx.screen.cx, width);
        ctx.finish();
    }
}

#[test]
fn third_regional_indicator_starts_another_cell() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    for text in ["🇩", "🇪", "🇫"] {
        ctx.cell(&cell(text, 1));
    }
    assert_eq!(
        ctx.screen.grid.view_get_cell(0, 0).data.bytes(),
        "🇩🇪".as_bytes()
    );
    assert_eq!(
        ctx.screen.grid.view_get_cell(2, 0).data.bytes(),
        "🇫".as_bytes()
    );
    assert_eq!(ctx.screen.cx, 3);
    ctx.finish();
}

#[test]
fn hangul_jamo_composes_or_discards_without_generic_segmentation() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(12, &mut registry);
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("x", 1));
    ctx.cell(&cell("ᅡ", 1));
    assert_eq!(ctx.screen.cx, 1);
    ctx.cell(&cell("ᄀ", 2));
    ctx.cell(&cell("ᅡ", 1));
    ctx.cell(&cell("ᆨ", 1));
    ctx.cell(&cell("\u{3164}", 2));
    assert_eq!(
        ctx.screen.grid.view_get_cell(1, 0).data.bytes(),
        "각".as_bytes()
    );
    assert_eq!(ctx.screen.cx, 3);
    ctx.cell(&cell("ᄂ", 2));
    assert_eq!(ctx.screen.cx, 5);
    ctx.finish();
}

#[test]
fn payload_capacity_discards_dependent_but_writes_independent_input() {
    for dependent in [false, true] {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = screen(12, &mut registry);
        let mut sink = RecordingSink::default();
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        let mut full = cell("a", 1);
        full.data.data[..28].fill(b'a');
        full.data.data[28..31].copy_from_slice("\u{200d}".as_bytes());
        full.data.size = 31;
        ctx.cell(&full);
        ctx.cell(&cell(
            if dependent { "\u{301}" } else { "界" },
            if dependent { 0 } else { 2 },
        ));
        assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.size, 31);
        assert_eq!(ctx.screen.cx, if dependent { 1 } else { 3 });
        if !dependent {
            assert_eq!(
                ctx.screen.grid.view_get_cell(1, 0).data.bytes(),
                "界".as_bytes()
            );
        }
        ctx.finish();
    }
}

#[test]
fn overwriting_every_tab_padding_column_retains_distinct_backgrounds() {
    for overwrite in 1..5 {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = screen(8, &mut registry);
        let mut tab = DEFAULT_CELL;
        tab.set_tab(5);
        tab.bg = Colour(1);
        screen.grid.view_set_cell(0, 0, &tab);
        for x in 1..5 {
            screen.grid.view_set_padding(x, 0, Colour(x as i32 + 1));
        }
        screen.cx = overwrite;
        let mut sink = RecordingSink::default();
        let policy = ScreenWritePolicy {
            pane_backed: true,
            ..ScreenWritePolicy::default()
        };
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            policy,
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.cell(&cell("x", 1));
        for x in 0..5 {
            let stored = ctx.screen.grid.view_get_cell(x, 0);
            assert!(
                !stored
                    .flags
                    .intersects(GridCellFlags::PADDING | GridCellFlags::TAB)
            );
            assert_eq!(
                stored.data.bytes(),
                if x == overwrite { b"x" } else { b" " }
            );
            assert_eq!(
                stored.bg,
                if x == overwrite {
                    Colour::DEFAULT
                } else {
                    Colour(x as i32 + 1)
                }
            );
        }
        ctx.finish();
        assert_eq!(sink.redraws, [0]);
    }
}

#[test]
fn wide_overwrite_repairs_trailing_padding_and_sets_new_background() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let mut old = cell("界", 2);
    old.bg = Colour(3);
    screen.grid.view_set_cell(2, 0, &old);
    screen.grid.view_set_padding(3, 0, old.bg);
    screen.cx = 1;
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    let mut new = cell("語", 2);
    new.bg = Colour(5);
    ctx.cell(&new);
    assert_eq!(ctx.screen.grid.view_get_cell(2, 0).bg, new.bg);
    assert!(
        ctx.screen
            .grid
            .view_get_cell(2, 0)
            .flags
            .contains(GridCellFlags::PADDING)
    );
    assert_eq!(ctx.screen.grid.view_get_cell(3, 0).data.bytes(), b" ");
    assert_eq!(ctx.screen.grid.view_get_cell(3, 0).bg, old.bg);
    ctx.finish();
}

#[test]
fn compact_equal_cell_skips_output_but_extended_cell_does_not() {
    for (text, width, expected) in [("a", 1, 0), ("é", 1, 1), ("界", 2, 1)] {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = screen(8, &mut registry);
        let input = cell(text, width);
        screen.grid.view_set_cell(0, 0, &input);
        if width == 2 {
            screen.grid.view_set_padding(1, 0, input.bg);
        }
        let mut sink = RecordingSink::default();
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.cell(&input);
        assert_eq!(ctx.screen.cx, u32::from(width));
        ctx.finish();
        assert_eq!(sink.cells.len(), expected);
    }
}

#[test]
fn insert_emits_prewrite_snapshot_without_shifting_twice() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    screen.mode.insert(ScreenMode::INSERT);
    screen.grid.view_set_cell(0, 0, &cell("a", 1));
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("界", 2));
    assert_eq!(ctx.screen.cx, 2);
    assert_eq!(ctx.screen.grid.view_get_cell(2, 0).data.bytes(), b"a");
    ctx.finish();
    assert_eq!(sink.inserts.len(), 1);
    assert_eq!(sink.inserts[0].0, 2);
    assert_eq!(sink.inserts[0].1.old_cx, 0);
    assert_eq!(sink.cells.last().unwrap().1.old_cx, 0);
}

#[test]
fn pending_wrap_and_nowrap_wide_rejection_match_cell_rules() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(4, &mut registry);
    screen.cx = 3;
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("a", 1));
    assert_eq!(ctx.screen.cx, 4);
    ctx.cell(&cell("界", 2));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 1));
    assert!(
        ctx.screen
            .grid
            .get_line(ctx.screen.grid.hsize())
            .flags
            .contains(GridLineFlags::WRAPPED)
    );
    ctx.screen.mode.remove(ScreenMode::WRAP);
    ctx.screen.cx = 3;
    ctx.cell(&cell("語", 2));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (3, 1));
    ctx.cell(&cell("x", 1));
    ctx.cell(&cell("y", 1));
    assert_eq!(ctx.screen.cx, 3);
    assert_eq!(ctx.screen.grid.view_get_cell(3, 1).data.bytes(), b"y");
    ctx.finish();
}

#[test]
fn partly_hidden_wide_cell_draws_only_visible_spaces() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let mut sink = RecordingSink {
        visible: Some(1..8),
        ..RecordingSink::default()
    };
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("界", 2));
    assert_eq!(
        ctx.screen.grid.view_get_cell(0, 0).data.bytes(),
        "界".as_bytes()
    );
    assert_eq!(ctx.screen.cx, 2);
    ctx.finish();
    assert_eq!(sink.cells.len(), 1);
    assert_eq!(sink.cells[0].0.data.bytes(), b" ");
    assert_eq!(sink.cells[0].1.old_cx, 1);
}

#[test]
fn exact_capacity_draw_preserves_payload_and_pinned_storage_alias() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    let mut base = cell("a", 1);
    base.data.data[..30].fill(b'a');
    base.data.size = 30;
    ctx.cell(&base);
    ctx.cell(&cell("\u{301}", 0));
    // The pinned five-bit size field aliases 32 to zero and changes width bits.
    let stored = ctx.screen.grid.view_get_cell(0, 0);
    assert_eq!(stored.data.size, 0);
    assert_eq!(stored.data.width, 2);
    assert_eq!(ctx.screen.cx, 1);
    ctx.finish();
    assert_eq!(sink.cells.len(), 2);
    assert_eq!(sink.cells[1].0.data.size, 32);
    assert_eq!(sink.cells[1].0.data.width, 1);
    assert_eq!(&sink.cells[1].0.data.data[30..], "\u{301}".as_bytes());
}

#[test]
fn invalid_preceding_padding_discards_dependent_input() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    screen.grid.view_set_cell(0, 0, &cell("a", 1));
    screen.grid.view_set_padding(1, 0, Colour::DEFAULT);
    screen.cx = 2;
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("\u{301}", 0));
    assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.bytes(), b"a");
    assert_eq!(ctx.screen.cx, 2);
    ctx.finish();
    assert!(sink.cells.is_empty());
}

#[test]
fn narrow_write_repairs_both_columns_of_old_wide_cell() {
    for x in [0, 1] {
        let mut registry = HyperlinkRegistry::new();
        let mut screen = screen(8, &mut registry);
        let mut wide = cell("界", 2);
        wide.bg = Colour(6);
        screen.grid.view_set_cell(0, 0, &wide);
        screen.grid.view_set_padding(1, 0, wide.bg);
        screen.cx = x;
        let mut sink = RecordingSink::default();
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.cell(&cell("x", 1));
        let repaired = ctx.screen.grid.view_get_cell(1 - x, 0);
        assert_eq!(repaired.data.bytes(), b" ");
        assert_eq!(repaired.bg, wide.bg);
        assert!(!repaired.flags.contains(GridCellFlags::PADDING));
        ctx.finish();
    }
}

#[test]
fn selection_disables_equal_cell_skip_and_only_styles_output() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let source = cell("a", 1);
    screen.grid.view_set_cell(0, 0, &source);
    let selection_cell = GridCell {
        fg: Colour(2),
        bg: Colour(4),
        ..DEFAULT_CELL
    };
    screen.set_selection(rmux_emu::screen::ScreenSelection {
        hidden: false,
        rectangle: true,
        modekeys: 1,
        sx: 0,
        sy: 0,
        ex: 0,
        ey: 0,
        clipx: 0,
        cell: selection_cell,
    });
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&source);
    let stored = ctx.screen.grid.view_get_cell(0, 0);
    assert!(stored.flags.contains(GridCellFlags::SELECTED));
    assert_eq!(stored.fg, source.fg);
    assert_eq!(stored.bg, source.bg);
    assert_eq!(ctx.screen.cx, 1);
    ctx.finish();
    assert_eq!(sink.cells.len(), 1);
    assert_eq!(sink.cells[0].0.fg, selection_cell.fg);
    assert_eq!(sink.cells[0].0.bg, selection_cell.bg);
    assert_eq!(sink.cells[0].0.data.bytes(), b"a");
}

#[test]
fn modifier_base_absent_from_pinned_table_stays_separate() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = screen(8, &mut registry);
    let mut sink = RecordingSink::default();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cell(&cell("☝", 1));
    ctx.cell(&cell("🏻", 2));
    assert_eq!(
        ctx.screen.grid.view_get_cell(0, 0).data.bytes(),
        "☝".as_bytes()
    );
    assert_eq!(
        ctx.screen.grid.view_get_cell(1, 0).data.bytes(),
        "🏻".as_bytes()
    );
    assert_eq!(ctx.screen.cx, 3);
    ctx.finish();
}
