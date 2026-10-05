// Ported from tmux screen-write.c @ 8f25579c
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::write::{
    DrawCommand, DrawOp, DrawSnapshot, ScreenRenderEffects, ScreenWriteCtx, ScreenWritePolicy,
    TtySink,
};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use std::ops::Range;
#[derive(Default)]
struct Recorder {
    calls: Vec<(String, DrawSnapshot)>,
    effects: Vec<ScreenRenderEffects>,
    visible: bool,
}
impl TtySink for Recorder {
    fn draw(&mut self, op: DrawOp<'_>, snap: &DrawSnapshot) {
        let name = match op.command {
            DrawCommand::Cells { data, .. } => String::from_utf8_lossy(data).into_owned(),
            DrawCommand::ClearCharacter { count, bg } => format!("clear:{count}:{}", bg.0),
            DrawCommand::ScrollUp { count, .. } => format!("scroll:{count}"),
            DrawCommand::SyncStart => "sync".into(),
            _ => "other".into(),
        };
        self.calls.push((name, *snap));
    }
    fn visible_columns(&mut self, x: u32, _: u32, n: u32, out: &mut Vec<Range<u32>>) {
        out.clear();
        if self.visible && n != 0 {
            out.push(x..x + n);
        }
    }
    fn obscured(&mut self) -> bool {
        false
    }
    fn redraw_pending(&self) -> bool {
        false
    }
    fn effect(&mut self, e: ScreenRenderEffects, _: &Screen) {
        self.effects.push(e);
    }
    fn begin_write(&mut self) {}
}
fn ascii(ch: u8) -> GridCell {
    let mut c = DEFAULT_CELL;
    c.data.data[0] = ch;
    c
}
#[test]
fn collection_commits_only_at_boundary_and_survives_invisible_transactions() {
    let mut reg = HyperlinkRegistry::new();
    let mut screen = Screen::new(8, 3, 10, ScreenResetPolicy::default(), &mut reg).unwrap();
    let mut sink = Recorder::default();
    {
        let mut w = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut reg,
        );
        w.collect_add(&ascii(b'a'));
        w.collect_add(&ascii(b'b'));
        assert_eq!(w.screen.cx, 0);
        w.collect_end();
        assert_eq!(w.screen.cx, 2);
        w.finish();
    }
    assert_eq!(screen.write_list[0].items.len(), 1);
    // screen_reinit leaves the write list alone; only a resize frees and
    // remakes it (screen.c:359-360,398-399).
    screen
        .reinit(false, ScreenResetPolicy::default(), &mut reg)
        .unwrap();
    assert_eq!(screen.write_list[0].items.len(), 1);
    screen.resize(8, 4, false);
    assert_eq!(screen.write_list.len(), 4);
    assert!(screen.write_list.iter().all(|row| row.items.is_empty()));
    screen.resize(8, 3, false);
    screen.cx = 0;
    screen.cy = 0;
    {
        let mut w = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut reg,
        );
        w.collect_add(&ascii(b'a'));
        w.collect_add(&ascii(b'b'));
        w.finish();
    }
    sink.visible = true;
    ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut reg,
    )
    .finish();
    assert!(screen.write_list[0].items.is_empty());
    assert!(sink.calls.iter().any(|(s, _)| s == "ab"));
    screen.release(&mut reg).unwrap();
}
#[test]
fn overlap_split_keeps_order_attributes_and_exact_bytes() {
    let mut reg = HyperlinkRegistry::new();
    let mut screen = Screen::new(10, 2, 0, ScreenResetPolicy::default(), &mut reg).unwrap();
    let mut sink = Recorder::default();
    {
        let mut w = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut reg,
        );
        for ch in b"abcdefghij" {
            w.collect_add(&ascii(*ch));
        }
        w.collect_end();
        w.cursormove(3, 0, false);
        w.collect_add(&ascii(b'X'));
        w.collect_add(&ascii(b'Y'));
        w.collect_end();
        w.finish();
    }
    let items = &screen.write_list[0].items;
    assert_eq!(items.len(), 3);
    assert_eq!((items[0].x, items[0].used), (0, 3));
    assert_eq!((items[1].x, items[1].used), (3, 2));
    assert_eq!((items[2].x, items[2].used), (5, 5));
    screen.release(&mut reg).unwrap();
}
#[test]
fn scrolling_rotates_row_buffers_and_draws_scroll_first() {
    let mut reg = HyperlinkRegistry::new();
    let mut screen = Screen::new(5, 3, 10, ScreenResetPolicy::default(), &mut reg).unwrap();
    let mut sink = Recorder {
        visible: true,
        ..Recorder::default()
    };
    {
        let mut w = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut reg,
        );
        w.scrollregion(1, 2);
        w.cursormove(0, 2, false);
        w.collect_add(&ascii(b'X'));
        w.collect_end();
        w.linefeed(false, Colour(3));
        w.collect_add(&ascii(b'Y'));
        w.finish();
    }
    let scroll = sink
        .calls
        .iter()
        .position(|(s, _)| s == "scroll:1")
        .unwrap();
    let text = sink.calls.iter().position(|(s, _)| s == "X").unwrap();
    assert!(scroll < text);
    assert_eq!(screen.grid.view_get_cell(0, 1).data.data[0], b'X');
    screen.release(&mut reg).unwrap();
}
#[test]
fn transaction_splits_preserve_plain_print_grid() {
    for split in 0..=30 {
        let mut reg = HyperlinkRegistry::new();
        let mut screen = Screen::new(7, 3, 10, ScreenResetPolicy::default(), &mut reg).unwrap();
        let mut sink = Recorder {
            visible: true,
            ..Recorder::default()
        };
        let input = b"abcdefghijklmnopqrstuvwxyzABCD";
        for part in [&input[..split], &input[split..]] {
            let mut w = ScreenWriteCtx::start(
                &mut screen,
                &mut sink,
                ScreenWritePolicy::default(),
                &mut reg,
            );
            for ch in part {
                w.collect_add(&ascii(*ch));
            }
            w.finish();
        }
        assert_eq!((screen.cx, screen.cy), (2, 2));
        assert_eq!(screen.grid.view_get_cell(0, 2).data.data[0], b'C');
        screen.release(&mut reg).unwrap();
    }
}
#[test]
fn raw_and_clipboard_bypass_application_sync_without_grid_changes() {
    let mut reg = HyperlinkRegistry::new();
    let mut screen = Screen::new(5, 2, 0, ScreenResetPolicy::default(), &mut reg).unwrap();
    screen.mode.insert(ScreenMode::SYNC);
    let mut sink = Recorder {
        visible: true,
        ..Recorder::default()
    };
    {
        let mut w = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy {
                pane_backed: true,
                ..ScreenWritePolicy::default()
            },
            &mut reg,
        );
        w.rawstring(b"\0\x1braw", true);
        w.setselection(b"cp", b"\0clip");
        w.finish();
    }
    assert_eq!(sink.calls.iter().filter(|(s, _)| s == "other").count(), 2);
    assert_eq!(screen.grid.view_get_cell(0, 0), DEFAULT_CELL);
    screen.release(&mut reg).unwrap();
}

#[test]
fn sync_start_retains_collection_and_end_records_dirty_before_stop() {
    let mut reg = HyperlinkRegistry::new();
    let mut screen = Screen::new(5, 3, 0, ScreenResetPolicy::default(), &mut reg).unwrap();
    let mut sink = Recorder {
        visible: true,
        ..Recorder::default()
    };
    {
        let mut w = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy {
                pane_backed: true,
                ..ScreenWritePolicy::default()
            },
            &mut reg,
        );
        w.collect_add(&ascii(b'X'));
        w.collect_end();
        w.start_sync();
        assert_eq!(w.screen.write_list[0].items.len(), 1);
        w.start_sync();
        w.scrollup(1, Colour(3));
        w.end_sync();
        assert!(!w.screen.mode.contains(ScreenMode::SYNC));
        w.finish();
    }
    assert_eq!(
        sink.effects
            .iter()
            .filter(|e| matches!(e, ScreenRenderEffects::StartSyncTimer))
            .count(),
        2
    );
    let scroll = sink
        .effects
        .iter()
        .position(|e| {
            matches!(
                e,
                ScreenRenderEffects::DeferredScroll {
                    count: 1,
                    bg: Colour(3),
                    ..
                }
            )
        })
        .unwrap();
    let stop = sink
        .effects
        .iter()
        .position(|e| matches!(e, ScreenRenderEffects::StopSync))
        .unwrap();
    assert!(scroll < stop);
    assert!(
        sink.effects[..stop]
            .iter()
            .any(|e| matches!(e, ScreenRenderEffects::DirtyRows { .. }))
    );
    assert!(sink.calls.iter().all(|(name, _)| name == "sync"));
    screen.release(&mut reg).unwrap();
}

#[test]
fn sync_controls_are_noops_for_screen_only_routing() {
    let mut reg = HyperlinkRegistry::new();
    let mut screen = Screen::new(5, 2, 0, ScreenResetPolicy::default(), &mut reg).unwrap();
    let mut sink = Recorder {
        visible: true,
        ..Recorder::default()
    };
    {
        let mut w = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut reg,
        );
        w.collect_add(&ascii(b'X'));
        w.collect_end();
        w.start_sync();
        w.end_sync();
        assert_eq!(w.screen.write_list[0].items.len(), 1);
        assert!(!w.screen.mode.contains(ScreenMode::SYNC));
        w.finish();
    }
    assert!(sink.calls.iter().any(|(s, _)| s == "X"));
    assert!(sink.effects.is_empty());
    screen.release(&mut reg).unwrap();
}
