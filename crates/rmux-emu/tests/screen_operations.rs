// Ported from tmux screen-write.c @ 8f25579c
use std::ops::Range;

use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::grid::{GridLineFlags, Osc133Data};
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::write::{
    DrawCommand, DrawOp, DrawSnapshot, ScreenRenderEffects, ScreenWriteCtx, ScreenWriteItem,
    ScreenWriteItemKind, ScreenWritePolicy, TtySink,
};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_util::utf8::Utf8Data;

#[derive(Debug)]
struct Draw {
    name: &'static str,
    count: u32,
    bg: Colour,
    snapshot: DrawSnapshot,
    rows: Vec<Vec<u8>>,
}

#[derive(Default)]
struct Recorder {
    draws: Vec<Draw>,
    effects: Vec<ScreenRenderEffects>,
    queries: Vec<(u32, u32, u32)>,
    obscured: bool,
    redraw_pending: bool,
    visible_end: Option<u32>,
    scrollbar: bool,
}

impl TtySink for Recorder {
    fn draw(&mut self, op: DrawOp<'_>, snapshot: &DrawSnapshot) {
        let (name, count, bg) = match op.command {
            DrawCommand::SyncStart => ("sync", 0, Colour::DEFAULT),
            DrawCommand::AlignmentTest => ("alignment", 0, Colour::DEFAULT),
            DrawCommand::InsertCharacter { count, bg } => ("insert-character", count, bg),
            DrawCommand::DeleteCharacter { count, bg } => ("delete-character", count, bg),
            DrawCommand::ClearCharacter { count, bg } => ("clear-character", count, bg),
            DrawCommand::InsertLine { count, bg } => ("insert-line", count, bg),
            DrawCommand::DeleteLine { count, bg } => ("delete-line", count, bg),
            DrawCommand::ScrollUp { count, bg } => ("scroll-up", count, bg),
            DrawCommand::ScrollDown { count, bg } => ("scroll-down", count, bg),
            DrawCommand::ReverseIndex { bg } => ("reverse-index", 1, bg),
            DrawCommand::ClearEndOfScreen { bg } => ("clear-end-screen", 0, bg),
            DrawCommand::ClearStartOfScreen { bg } => ("clear-start-screen", 0, bg),
            DrawCommand::ClearScreen { bg } => ("clear-screen", 0, bg),
            DrawCommand::RedrawLine { count, .. } => ("redraw", count, Colour::DEFAULT),
            DrawCommand::Cells { data, .. } => ("text", data.len() as u32, Colour::DEFAULT),
            DrawCommand::Cell(cell) => ("cell", u32::from(cell.data.data[0]), cell.bg),
            _ => ("other", 0, Colour::DEFAULT),
        };
        self.draws.push(Draw {
            name,
            count,
            bg,
            snapshot: *snapshot,
            rows: (0..op.screen.grid.sy())
                .map(|y| {
                    (0..op.screen.grid.sx())
                        .map(|x| op.screen.grid.view_get_cell(x, y).data.data[0])
                        .collect()
                })
                .collect(),
        });
    }

    fn visible_columns(&mut self, x: u32, y: u32, count: u32, out: &mut Vec<Range<u32>>) {
        self.queries.push((x, y, count));
        out.clear();
        let end = (x + count).min(self.visible_end.unwrap_or(u32::MAX));
        if x < end {
            out.push(x..end);
        }
    }

    fn obscured(&mut self) -> bool {
        self.obscured
    }

    fn scrollbar_overlay_visible(&mut self) -> bool {
        self.scrollbar
    }

    fn redraw_pending(&self) -> bool {
        self.redraw_pending
    }

    fn effect(&mut self, effect: ScreenRenderEffects, _: &Screen) {
        self.effects.push(effect);
    }

    fn begin_write(&mut self) {}
}

fn screen(width: u32, height: u32, registry: &mut HyperlinkRegistry) -> Screen {
    Screen::new(width, height, 100, ScreenResetPolicy::default(), registry).unwrap()
}

fn cell(byte: u8) -> GridCell {
    GridCell {
        data: Utf8Data::set(byte),
        ..DEFAULT_CELL
    }
}

fn fill_rows(screen: &mut Screen) {
    for y in 0..screen.grid.sy() {
        for x in 0..screen.grid.sx() {
            screen.grid.view_set_cell(x, y, &cell(b'A' + y as u8));
        }
    }
}

fn clear_item(x: u32, count: u32, bg: Colour) -> ScreenWriteItem {
    ScreenWriteItem {
        x,
        used: count,
        kind: ScreenWriteItemKind::Clear,
        bg,
        ..ScreenWriteItem::default()
    }
}

fn pane_policy() -> ScreenWritePolicy {
    ScreenWritePolicy {
        pane_backed: true,
        ..ScreenWritePolicy::default()
    }
}

#[test]
fn relative_cursors_retain_pinned_pending_wrap_arithmetic() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 6, &mut registry);
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.scrollregion(2, 4);
    for (x, count, expected) in [(5, 0, 4), (5, 1, 4), (5, u32::MAX, 4), (4, 0, 4), (0, 0, 1)] {
        ctx.screen.cx = x;
        ctx.cursorright(count);
        assert_eq!(ctx.screen.cx, expected);
    }
    ctx.screen.cx = 5;
    ctx.cursorleft(0);
    assert_eq!(ctx.screen.cx, 4);
    ctx.screen.cx = 5;
    ctx.screen.cy = 2;
    ctx.cursorup(99);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (4, 2));
    ctx.screen.cy = 1;
    ctx.cursorup(99);
    assert_eq!(ctx.screen.cy, 0);
    ctx.screen.cy = 4;
    ctx.screen.cx = 5;
    ctx.cursordown(99);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (4, 4));
    ctx.screen.cy = 5;
    ctx.cursordown(99);
    assert_eq!(ctx.screen.cy, 5);
    ctx.screen.cx = 3;
    ctx.carriagereturn();
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 5));
    ctx.finish();
}

#[test]
fn absolute_origin_and_region_rejection_keep_distinct_clamps() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 6, &mut registry);
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.scrollregion(2, 4);
    ctx.screen.mode.insert(ScreenMode::ORIGIN);
    ctx.cursormove(5, 1, true);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (4, 3));
    ctx.cursormove(-1, 99, true);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (4, 4));
    ctx.cursormove(-2, -2, false);
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (4, 5));
    ctx.scrollregion(3, 3);
    assert_eq!((ctx.screen.rupper, ctx.screen.rlower), (2, 4));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (4, 5));
    ctx.scrollregion(1, 99);
    assert_eq!((ctx.screen.rupper, ctx.screen.rlower), (1, 5));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 0));
    ctx.finish();
}

#[test]
fn backspace_crosses_only_wrapped_visible_rows() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 3, &mut registry);
    let mut sink = Recorder::default();
    screen
        .grid
        .get_line_mut(screen.grid.hsize())
        .flags
        .insert(GridLineFlags::WRAPPED);
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.cursormove(0, 2, false);
    ctx.backspace();
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 2));
    ctx.cursormove(0, 1, false);
    ctx.backspace();
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (4, 0));
    ctx.cursormove(0, 0, false);
    ctx.backspace();
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (0, 0));
    ctx.finish();
}

#[test]
fn writer_reset_drops_crlf_and_uses_only_requested_extended_mode() {
    for extended_keys in [false, true] {
        for height in [1, 4] {
            let mut registry = HyperlinkRegistry::default();
            let mut screen = screen(17, height, &mut registry);
            fill_rows(&mut screen);
            screen.tabs.fill(true);
            screen.mode = ScreenMode::CRLF | ScreenMode::INSERT | ScreenMode::KEYS_EXTENDED_2;
            let mut sink = Recorder::default();
            let mut ctx =
                ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
            ctx.cursormove(16, height as i32 - 1, false);
            ctx.reset(ScreenResetPolicy { extended_keys });
            let expected = ScreenMode::CURSOR
                | ScreenMode::WRAP
                | if extended_keys {
                    ScreenMode::KEYS_EXTENDED
                } else {
                    ScreenMode(0)
                };
            assert_eq!(ctx.screen.mode, expected);
            assert_eq!(
                (
                    ctx.screen.cx,
                    ctx.screen.cy,
                    ctx.screen.rupper,
                    ctx.screen.rlower
                ),
                (0, 0, 0, height - 1)
            );
            for x in 0..17 {
                assert_eq!(ctx.screen.tabs[x], x == 8 || x == 16);
            }
            for y in 0..height {
                assert_eq!(ctx.screen.grid.view_get_cell(0, y).data.data[0], b' ');
            }
            ctx.finish();
        }
    }
}

#[test]
fn character_edits_capture_old_cursor_and_borrow_new_grid() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 3, &mut registry);
    for (x, byte) in b"ABCDE".iter().enumerate() {
        screen.grid.view_set_cell(x as u32, 1, &cell(*byte));
    }
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.scrollregion(1, 2);
    ctx.cursormove(2, 1, false);
    ctx.insertcharacter(0, Colour(4));
    ctx.deletecharacter(1, Colour(3));
    ctx.clearcharacter(99, Colour(2));
    ctx.screen.cx = 5;
    ctx.insertcharacter(1, Colour(1));
    ctx.deletecharacter(1, Colour(1));
    ctx.clearcharacter(1, Colour(1));
    ctx.finish();
    let edits: Vec<_> = sink
        .draws
        .iter()
        .filter(|draw| draw.name.ends_with("character"))
        .collect();
    assert_eq!(edits.len(), 3);
    assert_eq!(edits[0].rows[1], b"AB CD");
    assert_eq!(edits[1].rows[1], b"ABCD ");
    assert_eq!(edits[2].rows[1], b"AB   ");
    for draw in &edits {
        assert_eq!((draw.snapshot.old_cx, draw.snapshot.old_cy), (2, 1));
        assert_eq!((draw.snapshot.rupper, draw.snapshot.rlower), (1, 2));
        assert!(!draw.snapshot.sync);
    }
    assert_eq!((edits[0].count, edits[0].bg), (1, Colour(4)));
    assert_eq!((edits[2].count, edits[2].bg), (3, Colour(2)));
}

#[test]
fn line_edits_respect_region_and_keep_outside_dirty_span_asymmetry() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(3, 6, &mut registry);
    fill_rows(&mut screen);
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.scrollregion(1, 3);
    ctx.cursormove(2, 2, false);
    ctx.insertline(1, Colour::DEFAULT);
    assert_eq!(ctx.screen.grid.view_get_cell(0, 3).data.data[0], b'C');
    assert_eq!(ctx.screen.grid.view_get_cell(0, 4).data.data[0], b'E');
    ctx.deleteline(99, Colour::DEFAULT);
    assert_eq!(ctx.screen.grid.view_get_cell(0, 2).data.data[0], b' ');
    assert_eq!(ctx.screen.grid.view_get_cell(0, 4).data.data[0], b'E');
    ctx.screen.mode.insert(ScreenMode::SYNC);
    ctx.cursormove(0, 4, false);
    ctx.insertline(0, Colour::DEFAULT);
    ctx.deleteline(0, Colour::DEFAULT);
    ctx.finish();
    let dirty: Vec<_> = sink
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ScreenRenderEffects::DirtyRows { start, count } => Some((*start, *count)),
            _ => None,
        })
        .collect();
    assert_eq!(dirty, [(4, 2), (1, 3)]);
}

#[test]
fn linefeed_batches_scrolls_and_splits_on_background_change() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(4, 5, &mut registry);
    fill_rows(&mut screen);
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.scrollregion(1, 3);
    ctx.cursormove(2, 2, false);
    ctx.linefeed(true, Colour(1));
    let absolute = ctx.screen.grid.hsize() + 2;
    assert!(
        ctx.screen
            .grid
            .get_line(absolute)
            .flags
            .contains(GridLineFlags::WRAPPED)
    );
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 3));
    ctx.linefeed(false, Colour(1));
    ctx.scrollup(0, Colour(1));
    ctx.scrollup(1, Colour(2));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 3));
    assert_eq!(ctx.screen.grid.view_get_cell(0, 0).data.data[0], b'A');
    assert_eq!(ctx.screen.grid.view_get_cell(0, 4).data.data[0], b'E');
    ctx.finish();
    let scrolls: Vec<_> = sink
        .draws
        .iter()
        .filter(|draw| draw.name == "scroll-up")
        .collect();
    assert_eq!(scrolls.len(), 2);
    assert_eq!((scrolls[0].count, scrolls[0].bg), (2, Colour(1)));
    assert_eq!((scrolls[1].count, scrolls[1].bg), (1, Colour(2)));
}

#[test]
fn reverse_index_and_scroll_down_preserve_old_structural_coordinates() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(4, 5, &mut registry);
    fill_rows(&mut screen);
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.scrollregion(1, 3);
    ctx.cursormove(2, 2, false);
    ctx.reverseindex(Colour(4));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 1));
    ctx.reverseindex(Colour(4));
    ctx.scrolldown(99, Colour(5));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 1));
    ctx.finish();
    let draws: Vec<_> = sink
        .draws
        .iter()
        .filter(|draw| draw.name == "reverse-index" || draw.name == "scroll-down")
        .collect();
    assert_eq!(draws.len(), 2);
    assert_eq!(draws[0].rows[2], b"BBBB");
    assert_eq!(draws[1].count, 3);
    for draw in &draws {
        assert_eq!((draw.snapshot.old_cx, draw.snapshot.old_cy), (2, 1));
        assert_eq!((draw.snapshot.rupper, draw.snapshot.rlower), (1, 3));
    }
    // TTY_CTX_SYNC is set only on the ttyctx that starts the physical
    // transaction (screen-write.c:332-346).
    assert!(draws[0].snapshot.sync);
    assert!(!draws[1].snapshot.sync);
}

#[test]
fn whole_line_clear_preserves_osc133_and_suffix_clear_skips_implicit_blanks() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 3, &mut registry);
    screen.grid.view_set_cell(0, 1, &cell(b'A'));
    let flags = GridLineFlags::OSC133_FLAGS | GridLineFlags::WRAPPED;
    let metadata = Osc133Data {
        prompt_col: 2,
        cmd_col: 3,
        out_start_col: 4,
        ..Osc133Data::default()
    };
    let line = screen.grid.get_line_mut(screen.grid.hsize() + 1);
    line.flags = flags;
    line.osc133 = metadata;
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.cursormove(3, 1, false);
    ctx.clearline(Colour::DEFAULT);
    let line = ctx.screen.grid.get_line(ctx.screen.grid.hsize() + 1);
    assert_eq!(line.osc133, metadata);
    assert_eq!(
        line.flags & GridLineFlags::OSC133_FLAGS,
        GridLineFlags::OSC133_FLAGS
    );
    assert!(!line.flags.contains(GridLineFlags::WRAPPED));
    assert_eq!(ctx.screen.write_list[1].items.len(), 1);
    ctx.cursormove(3, 2, false);
    ctx.clearendofline(Colour::DEFAULT);
    assert!(ctx.screen.write_list[2].items.is_empty());
    ctx.clearendofline(Colour(4));
    assert_eq!(
        (
            ctx.screen.write_list[2].items[0].x,
            ctx.screen.write_list[2].items[0].used
        ),
        (3, 2)
    );
    ctx.screen.cx = 5;
    ctx.clearendofline(Colour(1));
    assert_eq!(ctx.screen.write_list[2].items.len(), 1);
    ctx.clearstartofline(Colour(2));
    assert_eq!(
        (
            ctx.screen.write_list[2].items[0].x,
            ctx.screen.write_list[2].items[0].used
        ),
        (0, 5)
    );
    ctx.finish();
}

#[test]
fn partial_screen_erases_flush_but_whole_screen_discards_without_flush() {
    for name in ["clear-end-screen", "clear-start-screen", "clear-screen"] {
        let mut registry = HyperlinkRegistry::default();
        let mut screen = screen(5, 3, &mut registry);
        fill_rows(&mut screen);
        let mut sink = Recorder::default();
        let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
        ctx.cursormove(2, 1, false);
        for row in &mut ctx.screen.write_list {
            row.items.push(clear_item(0, 1, Colour(7)));
        }
        match name {
            "clear-end-screen" => ctx.clearendofscreen(Colour(4)),
            "clear-start-screen" => ctx.clearstartofscreen(Colour(4)),
            _ => ctx.clearscreen(Colour(4)),
        }
        ctx.finish();
        let actual: Vec<_> = sink
            .draws
            .iter()
            .filter(|draw| draw.name != "sync")
            .map(|draw| draw.name)
            .collect();
        if name == "clear-screen" {
            assert_eq!(actual, [name]);
        } else {
            assert_eq!(actual, ["clear-character", "clear-character", name]);
        }
        let draw = sink.draws.iter().find(|draw| draw.name == name).unwrap();
        assert_eq!((draw.snapshot.old_cx, draw.snapshot.old_cy), (2, 1));
        if name == "clear-end-screen" {
            assert_eq!(
                draw.rows,
                [b"AAAAA".to_vec(), b"BB   ".to_vec(), b"     ".to_vec()]
            );
        } else if name == "clear-start-screen" {
            assert_eq!(
                draw.rows,
                [b"     ".to_vec(), b"   BB".to_vec(), b"CCCCC".to_vec()]
            );
        }
    }
}

#[test]
fn scroll_on_clear_requires_history_pane_policy_and_end_erase_home() {
    for pane_backed in [false, true] {
        for full in [false, true] {
            let mut registry = HyperlinkRegistry::default();
            let mut screen = screen(4, 3, &mut registry);
            fill_rows(&mut screen);
            let mut sink = Recorder::default();
            let policy = ScreenWritePolicy {
                pane_backed,
                scroll_on_clear: true,
                ..ScreenWritePolicy::default()
            };
            let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, policy, &mut registry);
            if full {
                ctx.cursormove(2, 1, false);
                ctx.clearscreen(Colour::DEFAULT);
            } else {
                ctx.clearendofscreen(Colour::DEFAULT);
            }
            assert_eq!(ctx.screen.grid.hsize(), if pane_backed { 3 } else { 0 });
            ctx.clearhistory();
            assert_eq!(ctx.screen.grid.hsize(), 0);
            ctx.finish();
        }
    }
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(4, 3, &mut registry);
    fill_rows(&mut screen);
    let mut sink = Recorder::default();
    let policy = ScreenWritePolicy {
        scroll_on_clear: true,
        ..pane_policy()
    };
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, policy, &mut registry);
    ctx.cursormove(1, 0, false);
    ctx.clearendofscreen(Colour::DEFAULT);
    assert_eq!(ctx.screen.grid.hsize(), 0);
    ctx.finish();
}

#[test]
fn obscured_erases_collect_visible_spans_and_restore_cursor() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 4, &mut registry);
    fill_rows(&mut screen);
    let mut sink = Recorder {
        obscured: true,
        visible_end: Some(3),
        ..Recorder::default()
    };
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.cursormove(2, 1, false);
    ctx.clearendofscreen(Colour(6));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (2, 1));
    assert_eq!(
        (
            ctx.screen.write_list[1].items[0].x,
            ctx.screen.write_list[1].items[0].used
        ),
        (2, 1)
    );
    assert_eq!(ctx.screen.write_list[2].items[0].used, 3);
    assert_eq!(ctx.screen.write_list[3].items[0].used, 3);
    ctx.finish();
    assert!(
        !sink
            .draws
            .iter()
            .any(|draw| draw.name == "clear-end-screen")
    );
    let clears: Vec<_> = sink
        .draws
        .iter()
        .filter(|draw| draw.name == "clear-character")
        .collect();
    assert_eq!(clears.len(), 3);
    assert_eq!(
        (clears[0].snapshot.old_cx, clears[0].snapshot.old_cy),
        (2, 1)
    );
}

#[test]
fn obscured_start_erase_preserves_pinned_mutable_loop_and_final_span() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 4, &mut registry);
    fill_rows(&mut screen);
    let mut sink = Recorder {
        obscured: true,
        ..Recorder::default()
    };
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.cursormove(3, 3, false);
    ctx.clearstartofscreen(Colour(6));
    assert_eq!((ctx.screen.cx, ctx.screen.cy), (3, 3));
    assert!(!ctx.screen.write_list[0].items.is_empty());
    assert!(ctx.screen.write_list[1].items.is_empty());
    assert!(ctx.screen.write_list[2].items.is_empty());
    assert!(ctx.screen.write_list[3].items.is_empty());
    ctx.finish();
    assert_eq!(&sink.queries[..2], &[(0, 0, 5), (0, 3, 1)]);
    assert!(
        !sink
            .draws
            .iter()
            .any(|draw| draw.name == "clear-start-screen")
    );
    assert!(
        sink.draws
            .iter()
            .filter(|draw| draw.name == "clear-character")
            .all(|draw| draw.snapshot.old_cy == 0)
    );
}

#[test]
fn obscured_structural_redraw_preserves_direct_old_cursor() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 3, &mut registry);
    fill_rows(&mut screen);
    let mut sink = Recorder {
        obscured: true,
        ..Recorder::default()
    };
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.cursormove(3, 1, false);
    ctx.insertcharacter(1, Colour::DEFAULT);
    ctx.insertline(1, Colour::DEFAULT);
    ctx.finish();
    let redraws: Vec<_> = sink
        .draws
        .iter()
        .filter(|draw| draw.name == "redraw")
        .collect();
    assert_eq!(redraws.len(), 4);
    assert!(
        redraws
            .iter()
            .all(|draw| (draw.snapshot.old_cx, draw.snapshot.old_cy) == (3, 1))
    );
}

#[test]
fn alignment_keeps_last_collected_row_and_fullredraw_flushes_before_damage() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(4, 3, &mut registry);
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    for row in &mut ctx.screen.write_list {
        row.items.push(clear_item(0, 1, Colour(7)));
    }
    ctx.cursormove(3, 1, false);
    ctx.alignmenttest();
    assert!(ctx.screen.write_list[0].items.is_empty());
    assert!(ctx.screen.write_list[1].items.is_empty());
    assert_eq!(ctx.screen.write_list[2].items.len(), 1);
    assert_eq!(
        (
            ctx.screen.cx,
            ctx.screen.cy,
            ctx.screen.rupper,
            ctx.screen.rlower
        ),
        (0, 0, 0, 2)
    );
    ctx.fullredraw();
    assert!(ctx.screen.write_list[2].items.is_empty());
    ctx.finish();
    let names: Vec<_> = sink
        .draws
        .iter()
        .filter(|draw| draw.name != "sync")
        .map(|draw| draw.name)
        .collect();
    assert_eq!(names, ["alignment", "clear-character"]);
    let alignment = sink
        .draws
        .iter()
        .find(|draw| draw.name == "alignment")
        .unwrap();
    assert_eq!(
        alignment.rows,
        [b"EEEE".to_vec(), b"EEEE".to_vec(), b"EEEE".to_vec()]
    );
    assert!(
        sink.effects
            .contains(&ScreenRenderEffects::DamageRows { start: 0, count: 3 })
    );
}

#[test]
fn application_sync_and_redraw_drop_suppress_structural_output() {
    for redraw_pending in [false, true] {
        let mut registry = HyperlinkRegistry::default();
        let mut screen = screen(4, 3, &mut registry);
        fill_rows(&mut screen);
        screen.mode.insert(ScreenMode::SYNC);
        let mut sink = Recorder {
            redraw_pending,
            ..Recorder::default()
        };
        let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
        ctx.cursormove(1, 1, false);
        ctx.clearcharacter(1, Colour::DEFAULT);
        ctx.clearscreen(Colour::DEFAULT);
        ctx.finish();
        assert!(sink.draws.iter().all(|draw| draw.name == "sync"));
        let dirty: Vec<_> = sink
            .effects
            .iter()
            .filter_map(|effect| match effect {
                ScreenRenderEffects::DirtyRows { start, count } => Some((*start, *count)),
                _ => None,
            })
            .collect();
        if redraw_pending {
            assert!(dirty.is_empty());
        } else {
            assert_eq!(dirty, [(1, 1), (0, 3)]);
        }
    }
}

#[test]
fn alternate_exit_inside_a_transaction_remakes_the_write_list() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 3, &mut registry);
    let mut sink = Recorder::default();
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    for byte in b"abc" {
        ctx.collect_add(&cell(*byte));
    }
    ctx.collect_end();
    assert!(ctx.alternateon(&DEFAULT_CELL, true));
    ctx.collect_add(&cell(b'x'));
    ctx.collect_end();
    assert!(ctx.alternateoff(None, true));
    assert_eq!(ctx.screen.write_list.len(), 3);
    for byte in b"yz" {
        ctx.collect_add(&cell(*byte));
    }
    ctx.collect_end();
    assert_eq!(ctx.screen.grid.view_get_cell(3, 0).data.bytes(), b"y");
    assert_eq!(ctx.screen.grid.view_get_cell(4, 0).data.bytes(), b"z");
    ctx.finish();
    assert_eq!(
        sink.effects
            .iter()
            .filter(|effect| matches!(effect, ScreenRenderEffects::AlternateChanged { .. }))
            .count(),
        2
    );
}

#[test]
fn scroll_flush_with_overlay_scrollbar_requires_pane_redraw_and_discards() {
    let mut registry = HyperlinkRegistry::default();
    let mut screen = screen(5, 3, &mut registry);
    let mut sink = Recorder {
        scrollbar: true,
        ..Recorder::default()
    };
    let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
    ctx.cursormove(0, 2, false);
    ctx.collect_add(&cell(b'q'));
    ctx.collect_end();
    ctx.linefeed(false, Colour(3));
    ctx.finish();
    assert!(sink.draws.iter().all(|draw| draw.name == "sync"));
    let effects: Vec<_> = sink
        .effects
        .iter()
        .filter(|effect| !matches!(effect, ScreenRenderEffects::CursorMoved { .. }))
        .collect();
    assert_eq!(effects, [&ScreenRenderEffects::RequirePaneRedraw]);
    assert!(screen.write_list.iter().all(|row| row.items.is_empty()));
    assert_eq!(screen.grid.view_get_cell(0, 1).data.bytes(), b"q");
}

#[test]
fn obscured_single_column_redraw_uses_cell_only_for_plain_ascii() {
    let mut registry = HyperlinkRegistry::default();
    for plain in [true, false] {
        let mut screen = screen(4, 2, &mut registry);
        fill_rows(&mut screen);
        if !plain {
            let mut tab = cell(b' ');
            tab.set_tab(1);
            screen.grid.view_set_cell(0, 0, &tab);
        }
        let mut sink = Recorder {
            obscured: true,
            visible_end: Some(1),
            ..Recorder::default()
        };
        let mut ctx = ScreenWriteCtx::start(&mut screen, &mut sink, pane_policy(), &mut registry);
        ctx.cursormove(2, 0, false);
        ctx.deletecharacter(1, Colour::DEFAULT);
        ctx.finish();
        let draws: Vec<_> = sink
            .draws
            .iter()
            .filter(|draw| draw.name != "sync")
            .map(|draw| {
                (
                    draw.name,
                    draw.count,
                    draw.snapshot.old_cx,
                    draw.snapshot.old_cy,
                )
            })
            .collect();
        if plain {
            assert_eq!(draws, [("cell", u32::from(b'A'), 0, 0)]);
        } else {
            assert_eq!(draws, [("redraw", 1, 2, 0)]);
        }
    }
}
