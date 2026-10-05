// Ported from tmux screen.c, screen-write.c and input.c @ 8f25579c
#![cfg(feature = "sixel")]

use std::num::NonZeroU32;
use std::ops::Range;

use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::image::{ImageId, ImageRegistry, SixelImage};
use rmux_emu::input::{InputCtx, InputPolicy, NullSink};
use rmux_emu::screen::write::{
    DrawCommand, DrawOp, DrawSnapshot, ScreenRenderEffects, ScreenWriteCtx, ScreenWritePolicy,
    TtySink,
};
use rmux_emu::screen::{Screen, ScreenResetPolicy};
use rmux_util::utf8::Utf8Data;

#[derive(Debug, PartialEq, Eq)]
enum Output {
    Text,
    Image {
        origin: (u32, u32),
        size: (u32, u32),
        snapshot: DrawSnapshot,
    },
    Redraw,
}

#[derive(Default)]
struct Recorder(Vec<Output>);
impl TtySink for Recorder {
    fn draw(&mut self, op: DrawOp<'_>, snapshot: &DrawSnapshot) {
        match op.command {
            DrawCommand::SixelImage { image } => self.0.push(Output::Image {
                origin: (image.px, image.py),
                size: (image.sx, image.sy),
                snapshot: *snapshot,
            }),
            DrawCommand::Cells { .. } | DrawCommand::Cell(_) => self.0.push(Output::Text),
            _ => {}
        }
    }
    fn visible_columns(&mut self, x: u32, _: u32, n: u32, out: &mut Vec<Range<u32>>) {
        out.clear();
        if n != 0 {
            out.push(x..x + n);
        }
    }
    fn obscured(&mut self) -> bool {
        false
    }
    fn redraw_pending(&self) -> bool {
        false
    }
    fn effect(&mut self, effect: ScreenRenderEffects, _: &Screen) {
        if effect == ScreenRenderEffects::RequirePaneRedraw {
            self.0.push(Output::Redraw);
        }
    }
    fn begin_write(&mut self) {}
}

fn image(width: u32, height: u32) -> SixelImage {
    SixelImage::parse(
        format!("q\"1;1;{width};{height}#0").as_bytes(),
        0,
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(1).unwrap(),
    )
    .unwrap()
}

struct Fixture {
    images: ImageRegistry,
    links: HyperlinkRegistry,
    screen: Screen,
    sink: Recorder,
}
impl Fixture {
    fn new(width: u32, height: u32) -> Self {
        let mut images = ImageRegistry::default();
        let mut links = HyperlinkRegistry::default();
        let mut screen =
            Screen::new(width, height, 20, ScreenResetPolicy::default(), &mut links).unwrap();
        screen.bind_images(&mut images);
        Self {
            images,
            links,
            screen,
            sink: Recorder::default(),
        }
    }
    fn store(&mut self, x: u32, y: u32, width: u32, height: u32) -> ImageId {
        self.images.store(
            self.screen.image_owner().unwrap(),
            image(width, height),
            x,
            y,
        )
    }
    fn write(&mut self, pane: bool, operation: impl FnOnce(&mut ScreenWriteCtx<'_>)) {
        let mut ctx = ScreenWriteCtx::start(
            &mut self.screen,
            &mut self.sink,
            ScreenWritePolicy {
                pane_backed: pane,
                ..ScreenWritePolicy::default()
            },
            &mut self.links,
            Some(&mut self.images),
        );
        operation(&mut ctx);
        ctx.finish();
    }
    fn fill_row(&mut self, row: u32) {
        self.screen.grid.view_set_cell(
            0,
            row,
            &GridCell {
                data: Utf8Data::set(b'A'),
                ..DEFAULT_CELL
            },
        );
    }
}

type Operation = fn(&mut ScreenWriteCtx<'_>);
#[test]
fn every_destructive_screen_write_hook_invalidates_and_requests_pane_redraw() {
    let operations: &[(&str, Operation)] = &[
        ("alignment", |w| w.alignmenttest()),
        ("insert-character", |w| {
            w.insertcharacter(1, Colour::DEFAULT)
        }),
        ("delete-character", |w| {
            w.deletecharacter(1, Colour::DEFAULT)
        }),
        ("clear-character", |w| w.clearcharacter(1, Colour::DEFAULT)),
        ("insert-line", |w| w.insertline(1, Colour::DEFAULT)),
        ("delete-line", |w| w.deleteline(1, Colour::DEFAULT)),
        ("clear-line", |w| w.clearline(Colour::DEFAULT)),
        ("clear-end-line", |w| w.clearendofline(Colour(1))),
        ("clear-start-line", |w| w.clearstartofline(Colour::DEFAULT)),
        ("reverse-index", |w| {
            w.screen.rupper = w.screen.cy;
            w.reverseindex(Colour::DEFAULT);
        }),
        ("scroll-down", |w| w.scrolldown(1, Colour::DEFAULT)),
        ("clear-end-screen", |w| w.clearendofscreen(Colour::DEFAULT)),
        ("clear-start-screen", |w| {
            w.screen.cy = 0;
            w.clearstartofscreen(Colour::DEFAULT);
        }),
        ("clear-screen", |w| w.clearscreen(Colour::DEFAULT)),
        ("collected-text", |w| {
            w.collect_add(&GridCell {
                data: Utf8Data::set(b'X'),
                ..DEFAULT_CELL
            });
            w.collect_end();
        }),
    ];
    for &(name, operation) in operations {
        for pane in [false, true] {
            let mut f = Fixture::new(8, 5);
            f.fill_row(2);
            let id = f.store(0, 2, 4, 1);
            f.screen.cy = 2;
            f.write(pane, operation);
            assert!(
                f.images.get(f.screen.image_owner().unwrap(), id).is_none(),
                "{name}"
            );
            assert_eq!(f.sink.0.contains(&Output::Redraw), pane, "{name}");
        }
    }
}

#[test]
fn line_hooks_ignore_horizontal_position_but_text_hook_uses_its_area() {
    let mut f = Fixture::new(8, 5);
    let id = f.store(6, 2, 1, 1);
    f.screen.cy = 2;
    f.write(false, |w| {
        w.collect_add(&GridCell {
            data: Utf8Data::set(b'X'),
            ..DEFAULT_CELL
        });
        w.collect_end();
    });
    assert!(f.images.get(f.screen.image_owner().unwrap(), id).is_some());
    f.write(false, |w| w.insertcharacter(1, Colour::DEFAULT));
    assert!(f.images.get(f.screen.image_owner().unwrap(), id).is_none());
}

#[test]
fn no_op_character_and_line_clear_guards_keep_images() {
    let mut f = Fixture::new(8, 5);
    let id = f.store(0, 2, 8, 1);
    f.screen.cy = 2;
    f.write(true, |w| {
        w.clearline(Colour::DEFAULT);
        w.screen.cx = 3;
        w.clearendofline(Colour::DEFAULT);
        w.screen.cx = 8;
        w.insertcharacter(1, Colour::DEFAULT);
        w.deletecharacter(1, Colour::DEFAULT);
        w.clearcharacter(1, Colour::DEFAULT);
        w.reverseindex(Colour::DEFAULT);
    });
    assert!(f.images.get(f.screen.image_owner().unwrap(), id).is_some());
    assert!(!f.sink.0.contains(&Output::Redraw));
}

#[test]
fn insert_and_delete_lines_check_beyond_the_scroll_region() {
    for operation in [
        (|w: &mut ScreenWriteCtx<'_>| w.insertline(1, Colour::DEFAULT)) as Operation,
        (|w: &mut ScreenWriteCtx<'_>| w.deleteline(1, Colour::DEFAULT)) as Operation,
    ] {
        let mut f = Fixture::new(8, 5);
        let id = f.store(0, 4, 1, 1);
        f.screen.cy = 2;
        f.screen.rupper = 1;
        f.screen.rlower = 3;
        f.write(false, operation);
        assert!(f.images.get(f.screen.image_owner().unwrap(), id).is_none());
    }
}

#[test]
fn scrolling_preserves_partial_region_and_unsigned_span_quirks() {
    let mut f = Fixture::new(8, 5);
    let top = f.store(0, 1, 1, 1);
    let lower = f.store(0, 3, 1, 1);
    f.screen.rupper = 1;
    f.screen.rlower = 3;
    f.screen.cy = 3;
    f.write(false, |w| w.linefeed(false, Colour::DEFAULT));
    assert!(f.images.get(f.screen.image_owner().unwrap(), top).is_none());
    assert!(
        f.images
            .get(f.screen.image_owner().unwrap(), lower)
            .is_some()
    );
    f.write(false, |w| w.scrollup(1, Colour::DEFAULT));
    assert_eq!(
        f.images
            .get(f.screen.image_owner().unwrap(), lower)
            .unwrap()
            .py,
        2
    );
    f.screen.rlower = 4;
    f.screen.cy = 4;
    f.write(false, |w| w.linefeed(false, Colour::DEFAULT));
    assert_eq!(
        f.images
            .get(f.screen.image_owner().unwrap(), lower)
            .unwrap()
            .py,
        1
    );
    f.screen.cy = 1;
    f.write(false, |w| w.clearstartofscreen(Colour::DEFAULT));
    assert!(
        f.images
            .get(f.screen.image_owner().unwrap(), lower)
            .is_some()
    );
    f.screen.cy = 0;
    f.write(false, |w| w.clearstartofscreen(Colour::DEFAULT));
    assert!(
        f.images
            .get(f.screen.image_owner().unwrap(), lower)
            .is_none()
    );
}

#[test]
fn explicit_scroll_up_passes_normalized_count_even_for_partial_regions() {
    for (requested, moved) in [(0, 1), (1, 1), (u32::MAX, 3)] {
        let mut f = Fixture::new(8, 5);
        let id = f.store(0, 4, 1, 1);
        f.screen.rupper = 1;
        f.screen.rlower = 3;
        f.write(true, |w| w.scrollup(requested, Colour::DEFAULT));
        assert_eq!(
            f.images
                .get(f.screen.image_owner().unwrap(), id)
                .unwrap()
                .py,
            4 - moved
        );
        assert!(f.sink.0.contains(&Output::Redraw));
    }
}

#[test]
fn image_write_crops_only_in_the_oversize_branch_and_keeps_original_cursor_formula() {
    let mut f = Fixture::new(8, 5);
    f.screen.cx = 7;
    f.screen.cy = 1;
    f.write(false, |w| w.sixelimage(image(3, 1), Colour::DEFAULT));
    let id = f.images.ordered(f.screen.image_owner().unwrap())[0];
    assert_eq!(
        f.images
            .get(f.screen.image_owner().unwrap(), id)
            .unwrap()
            .sx,
        3
    );
    assert!(matches!(
        f.sink.0.last(),
        Some(Output::Image {
            origin: (7, 1),
            size: (3, 1),
            snapshot: DrawSnapshot {
                old_cx: 7,
                old_cy: 1,
                ..
            },
        })
    ));
    assert_eq!((f.screen.cx, f.screen.cy), (0, 2));
    f.screen.cx = 6;
    f.screen.cy = 4;
    f.write(false, |w| w.sixelimage(image(10, 7), Colour::DEFAULT));
    let id = *f
        .images
        .ordered(f.screen.image_owner().unwrap())
        .last()
        .unwrap();
    let stored = f.images.get(f.screen.image_owner().unwrap(), id).unwrap();
    assert_eq!((stored.px, stored.py, stored.sx, stored.sy), (6, 0, 2, 4));
    assert!(
        stored
            .data
            .print(None)
            .unwrap()
            .starts_with(b"\x1bP9;0q\"1;1;2;4")
    );
    assert!(matches!(
        f.sink.0.last(),
        Some(Output::Image {
            origin: (6, 0),
            size: (2, 4),
            snapshot: DrawSnapshot {
                old_cx: 6,
                old_cy: 0,
                ..
            },
        })
    ));
    assert_eq!((f.screen.cx, f.screen.cy), (0, 4));
}

#[test]
fn oversized_image_discards_top_pixels_and_keeps_bottom_source_rows() {
    let mut f = Fixture::new(8, 5);
    let data = SixelImage::parse(
        b"q\"1;1;2;7#0@-@",
        0,
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(1).unwrap(),
    )
    .unwrap();
    f.write(false, |w| w.sixelimage(data, Colour::DEFAULT));
    let owner = f.screen.image_owner().unwrap();
    let stored = f.images.get(owner, f.images.ordered(owner)[0]).unwrap();
    assert_eq!((stored.sx, stored.sy), (2, 4));
    assert_eq!(stored.data.pixel(0, 0), 0);
    assert_eq!(stored.data.pixel(0, 3), 1);
}

#[test]
fn image_writes_keep_overlapping_predecessors_without_eviction_redraw() {
    let mut f = Fixture::new(8, 5);
    let id = f.store(0, 0, 3, 2);
    f.write(true, |w| w.sixelimage(image(2, 1), Colour::DEFAULT));
    let owner = f.screen.image_owner().unwrap();
    assert_eq!(f.images.ordered(owner).len(), 2);
    assert_eq!(f.images.ordered(owner)[0], id);
    assert!(f.images.get(owner, id).is_some());
    assert!(!f.sink.0.contains(&Output::Redraw));
}

#[test]
fn image_write_flushes_text_before_draw_and_does_not_write_fallback_to_grid() {
    let mut f = Fixture::new(8, 5);
    f.write(false, |w| {
        w.collect_add(&GridCell {
            data: Utf8Data::set(b'A'),
            ..DEFAULT_CELL
        });
        w.collect_end();
        w.sixelimage(image(2, 1), Colour::DEFAULT);
    });
    assert!(matches!(
        f.sink.0.as_slice(),
        [
            Output::Text,
            Output::Image {
                origin: (1, 0),
                size: (2, 1),
                ..
            }
        ]
    ));
    assert_eq!(f.screen.grid.view_get_cell(0, 0).data.bytes(), b"A");
    assert_eq!(f.screen.grid.view_get_cell(1, 0).data.bytes(), b" ");
}

#[test]
fn one_row_screen_consumes_image_without_store_draw_or_cursor_change() {
    let mut f = Fixture::new(8, 1);
    f.screen.cx = 3;
    f.write(false, |w| w.sixelimage(image(2, 3), Colour::DEFAULT));
    assert!(f.images.ordered(f.screen.image_owner().unwrap()).is_empty());
    assert!(f.sink.0.is_empty());
    assert_eq!((f.screen.cx, f.screen.cy), (3, 0));
}

#[test]
fn alternate_lifecycle_preserves_saved_owner_and_resize_always_clears_current() {
    let mut f = Fixture::new(8, 5);
    let normal = f.screen.image_owner().unwrap();
    let id = f.store(3, 3, 2, 1);
    assert!(
        f.screen
            .alternate_on(&DEFAULT_CELL, true, Some(&mut f.images))
    );
    let alternate = f.screen.image_owner().unwrap();
    assert_ne!(normal, alternate);
    let temporary = f.store(0, 0, 1, 1);
    f.screen.resize(8, 5, false, Some(&mut f.images));
    assert!(f.images.get(alternate, temporary).is_none());
    assert!(f.images.get(normal, id).is_some());
    f.screen.resize(6, 4, true, Some(&mut f.images));
    assert_eq!(
        (
            f.images.get(normal, id).unwrap().px,
            f.images.get(normal, id).unwrap().py
        ),
        (3, 3)
    );
    f.store(0, 0, 1, 1);
    assert!(f.screen.alternate_off(None, true, Some(&mut f.images)));
    assert_eq!(f.screen.image_owner(), Some(normal));
    assert_eq!(
        (
            f.images.get(normal, id).unwrap().px,
            f.images.get(normal, id).unwrap().py
        ),
        (3, 3)
    );
    assert!(f.images.ordered(alternate).is_empty());
    f.screen
        .resize_cursor(6, 4, false, true, true, Some(&mut f.images));
    assert!(f.images.ordered(normal).is_empty());
}

#[test]
fn reset_exits_alternate_then_clears_restored_images_and_release_frees_both_owners() {
    let mut f = Fixture::new(8, 5);
    let normal = f.screen.image_owner().unwrap();
    f.store(0, 0, 1, 1);
    f.screen
        .alternate_on(&DEFAULT_CELL, false, Some(&mut f.images));
    let alternate = f.screen.image_owner().unwrap();
    f.store(0, 0, 1, 1);
    f.screen
        .reinit(
            false,
            ScreenResetPolicy::default(),
            &mut f.links,
            Some(&mut f.images),
        )
        .unwrap();
    assert!(!f.screen.is_alternate());
    assert_eq!(f.screen.image_owner(), Some(normal));
    assert!(f.images.ordered(normal).is_empty());
    assert!(f.images.ordered(alternate).is_empty());
    f.store(0, 0, 1, 1);
    f.screen
        .alternate_on(&DEFAULT_CELL, false, Some(&mut f.images));
    f.store(0, 0, 1, 1);
    f.screen.release(&mut f.links, Some(&mut f.images)).unwrap();
    assert!(f.screen.image_owner().is_none());
    f.screen.release(&mut f.links, None).unwrap();
    assert!(f.images.ordered(normal).is_empty());
    assert!(f.images.ordered(alternate).is_empty());
    assert_ne!(f.images.create_owner(), normal);
    assert_ne!(f.images.create_owner(), alternate);
}

#[test]
fn saved_images_share_global_budget_and_can_be_evicted_while_hidden() {
    let mut f = Fixture::new(8, 5);
    let normal = f.screen.image_owner().unwrap();
    let id = f.store(0, 0, 1, 1);
    f.screen
        .alternate_on(&DEFAULT_CELL, false, Some(&mut f.images));
    for _ in 0..19 {
        f.store(0, 0, 1, 1);
    }
    assert!(f.images.get(normal, id).is_none());
    f.screen.alternate_off(None, false, Some(&mut f.images));
    assert!(f.images.ordered(normal).is_empty());
}

#[test]
fn dcs_decode_obeys_framing_pane_eligibility_and_window_metrics_at_every_split() {
    let stream = b"\x1bP0;7q\"1;1;9;3#0@\x1b\\";
    for split in 0..=stream.len() {
        let mut f = Fixture::new(8, 5);
        let mut input = InputCtx::new();
        let policy = InputPolicy {
            sixel: false,
            pixels: Some((4, 2)),
            ..InputPolicy::default()
        };
        for bytes in [&stream[..split], &stream[split..]] {
            f.write(true, |w| {
                input.parse(w, None, &policy, &mut NullSink, bytes)
            });
        }
        let owner = f.screen.image_owner().unwrap();
        assert_eq!(f.images.ordered(owner).len(), 1, "split {split}");
        let stored = f.images.get(owner, f.images.ordered(owner)[0]).unwrap();
        assert_eq!((stored.sx, stored.sy), (3, 2));
        assert!(stored.data.print(None).unwrap().starts_with(b"\x1bP9;7q"));
        assert_eq!((f.screen.cx, f.screen.cy), (0, 2));
    }
    for (stream, pane) in [
        (&b"\x1bPq#0@\x1b\\"[..], false),
        (&b"\x1bP$q#0@\x1b\\"[..], true),
        (&b"\x1bPq\x1b\\"[..], true),
        (&b"\x1bPq#1025@\x1b\\"[..], true),
        (&b"\x1bP0;2147483648q#0@\x1b\\"[..], true),
    ] {
        let mut f = Fixture::new(8, 5);
        let mut input = InputCtx::new();
        let policy = InputPolicy {
            has_pane: pane,
            ..InputPolicy::default()
        };
        f.write(false, |w| {
            input.parse(w, None, &policy, &mut NullSink, stream)
        });
        assert!(
            f.images.ordered(f.screen.image_owner().unwrap()).is_empty(),
            "stream {stream:?}"
        );
    }
}

#[test]
fn discarded_dcs_never_stores_or_draws_a_partial_image() {
    let mut f = Fixture::new(8, 5);
    let mut input = InputCtx::new();
    let policy = InputPolicy {
        buffer_limit: 32,
        ..InputPolicy::default()
    };
    let mut stream = b"\x1bPq#0@".to_vec();
    stream.extend_from_slice(&[b'?'; 64]);
    stream.extend_from_slice(b"\x1b\\");
    f.write(false, |w| {
        input.parse(w, None, &policy, &mut NullSink, &stream)
    });
    assert!(f.images.ordered(f.screen.image_owner().unwrap()).is_empty());
    assert!(
        !f.sink
            .0
            .iter()
            .any(|output| matches!(output, Output::Image { .. }))
    );
}

#[test]
fn missing_dcs_parameter_and_zero_window_metrics_use_defaults() {
    let mut f = Fixture::new(8, 5);
    let mut input = InputCtx::new();
    let policy = InputPolicy {
        pixels: Some((0, 0)),
        ..InputPolicy::default()
    };
    f.write(false, |w| {
        input.parse(
            w,
            None,
            &policy,
            &mut NullSink,
            b"\x1bPq\"1;1;17;33#0@\x1b\\",
        )
    });
    let owner = f.screen.image_owner().unwrap();
    let stored = f.images.get(owner, f.images.ordered(owner)[0]).unwrap();
    assert_eq!((stored.sx, stored.sy), (2, 2));
    assert!(stored.data.print(None).unwrap().starts_with(b"\x1bP9;0q"));
}

#[test]
fn dcs_parameter_framing_rejects_colons_and_preserves_numeric_parameter_two() {
    for parameters in ["0;1:2", "1:2;", "0;0", "0;2147483647"] {
        let mut f = Fixture::new(8, 5);
        let mut input = InputCtx::new();
        let policy = InputPolicy::default();
        let sequence = format!("\x1bP{parameters}q#0@\x1b\\");
        f.write(false, |w| {
            input.parse(w, None, &policy, &mut NullSink, sequence.as_bytes())
        });
        let owner = f.screen.image_owner().unwrap();
        if parameters.contains(':') {
            assert!(
                f.images.ordered(owner).is_empty(),
                "parameters {parameters}"
            );
            continue;
        }
        assert_eq!(f.images.ordered(owner).len(), 1, "parameters {parameters}");
        let stored = f.images.get(owner, f.images.ordered(owner)[0]).unwrap();
        let p2 = if parameters == "0;2147483647" {
            2147483647
        } else {
            0
        };
        assert!(
            stored
                .data
                .print(None)
                .unwrap()
                .starts_with(format!("\x1bP9;{p2}q").as_bytes())
        );
    }
}

#[test]
fn cancel_and_substitute_inside_dcs_payload_are_ignored_sixel_controls_not_cancellation() {
    for control in [0x18, 0x1a] {
        let mut f = Fixture::new(8, 5);
        let mut input = InputCtx::new();
        let policy = InputPolicy::default();
        let sequence = [0x1b, b'P', b'q', b'#', b'0', b'@', control, 0x1b, b'\\'];
        f.write(false, |w| {
            input.parse(w, None, &policy, &mut NullSink, &sequence)
        });
        let owner = f.screen.image_owner().unwrap();
        assert_eq!(f.images.ordered(owner).len(), 1);
        assert_eq!(
            f.images
                .get(owner, f.images.ordered(owner)[0])
                .unwrap()
                .data
                .pixel(0, 0),
            1
        );
    }
}

#[test]
fn auxiliary_screens_stay_image_free_through_lifecycle_and_mutation_hooks() {
    let mut links = HyperlinkRegistry::default();
    let mut screen = Screen::new(8, 5, 0, ScreenResetPolicy::default(), &mut links).unwrap();
    let mut sink = Recorder::default();
    assert!(screen.image_owner().is_none());
    screen.alternate_on(&DEFAULT_CELL, true, None);
    screen.resize(9, 6, true, None);
    screen.alternate_off(None, true, None);
    screen
        .reinit(false, ScreenResetPolicy::default(), &mut links, None)
        .unwrap();
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut links,
        None,
    );
    ctx.alignmenttest();
    ctx.scrollup(1, Colour::DEFAULT);
    ctx.clearscreen(Colour::DEFAULT);
    ctx.finish();
    assert!(screen.image_owner().is_none());
    screen.release(&mut links, None).unwrap();
}

#[test]
#[should_panic(expected = "image-owned screen requires its registry")]
fn bound_screen_cannot_skip_registry_at_a_write_boundary() {
    let mut f = Fixture::new(8, 5);
    ScreenWriteCtx::start(
        &mut f.screen,
        &mut f.sink,
        ScreenWritePolicy::default(),
        &mut f.links,
        None,
    );
}

#[test]
fn suspended_writer_reborrows_registry_before_collected_text_invalidation() {
    let mut f = Fixture::new(8, 5);
    let id = f.store(0, 0, 2, 1);
    let mut ctx = ScreenWriteCtx::start(
        &mut f.screen,
        &mut f.sink,
        ScreenWritePolicy::default(),
        &mut f.links,
        Some(&mut f.images),
    );
    ctx.collect_add(&GridCell {
        data: Utf8Data::set(b'A'),
        ..DEFAULT_CELL
    });
    let state = ctx.suspend();
    assert!(f.images.get(f.screen.image_owner().unwrap(), id).is_some());
    ScreenWriteCtx::resume(
        &mut f.screen,
        &mut f.sink,
        ScreenWritePolicy::default(),
        &mut f.links,
        state,
        Some(&mut f.images),
    )
    .finish();
    assert!(f.images.get(f.screen.image_owner().unwrap(), id).is_none());
}

// Ported from tmux screen.c, screen-write.c, image.c and image-sixel.c @ 8f25579c
use std::fmt::Write;
use std::path::Path;
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

#[test]
fn mutation_hooks_and_image_cursor_match_unmodified_pinned_c() {
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/screen_images_reference.c");
    let mut flags = vec!["-DENABLE_SIXEL", "-ffunction-sections"];
    if cfg!(target_os = "macos") {
        flags.extend(["-Wl,-dead_strip", "-L/opt/homebrew/opt/libevent/lib"]);
    } else {
        flags.extend(["-Wl,--gc-sections", "-Wl,--no-as-needed"]);
    }
    flags.push("-levent");
    let Some(reference) = common::build_c(
        "screen-images",
        &[
            &driver,
            Path::new("utf8.c"),
            Path::new("utf8-combined.c"),
            Path::new("xmalloc.c"),
            Path::new("compat/utf8proc.c"),
            Path::new("compat/vis.c"),
            Path::new("compat/strtonum.c"),
            Path::new("compat/reallocarray.c"),
            Path::new("compat/recallocarray.c"),
            Path::new("compat/explicit_bzero.c"),
        ],
        &flags,
        cfg!(target_os = "macos"),
    ) else {
        return;
    };
    let operations = [
        "alignment",
        "insert-character",
        "delete-character",
        "clear-character",
        "insert-line",
        "delete-line",
        "clear-line",
        "clear-end-line",
        "clear-start-line",
        "reverse",
        "linefeed",
        "scroll-up",
        "scroll-down",
        "clear-end-screen",
        "clear-start-screen",
        "clear-screen",
        "text",
    ];
    let mut script = String::new();
    let mut expected = String::new();
    for (px, py, sx, sy, x, y, upper, lower, fill) in [
        (0, 2, 4, 1, 0, 2, 0, 4, false),
        (0, 2, 4, 1, 0, 2, 0, 4, true),
        (6, 2, 1, 1, 1, 2, 0, 4, false),
        (0, 4, 1, 1, 0, 2, 1, 3, true),
        (0, 3, 1, 1, 0, 3, 1, 3, true),
        (0, 1, 1, 3, 0, 4, 0, 4, true),
        (0, 2, 1, 1, 8, 2, 0, 4, false),
        (0, 2, 1, 1, 0, 0, 0, 4, false),
        (0, 2, 1, 1, 0, 1, 0, 4, false),
    ] {
        for operation in operations {
            writeln!(
                script,
                "8 5 {px} {py} {sx} {sy} {x} {y} {upper} {lower} {} {operation}",
                u8::from(fill)
            )
            .unwrap();
            let mut f = Fixture::new(8, 5);
            f.store(px, py, sx, sy);
            if fill {
                f.fill_row(y);
            }
            f.screen.cx = x;
            f.screen.cy = y;
            f.screen.rupper = upper;
            f.screen.rlower = lower;
            f.write(false, |w| match operation {
                "alignment" => w.alignmenttest(),
                "insert-character" => w.insertcharacter(1, Colour::DEFAULT),
                "delete-character" => w.deletecharacter(1, Colour::DEFAULT),
                "clear-character" => w.clearcharacter(1, Colour::DEFAULT),
                "insert-line" => w.insertline(1, Colour::DEFAULT),
                "delete-line" => w.deleteline(1, Colour::DEFAULT),
                "clear-line" => w.clearline(Colour::DEFAULT),
                "clear-end-line" => w.clearendofline(Colour::DEFAULT),
                "clear-start-line" => w.clearstartofline(Colour::DEFAULT),
                "reverse" => w.reverseindex(Colour::DEFAULT),
                "linefeed" => w.linefeed(false, Colour::DEFAULT),
                "scroll-up" => w.scrollup(1, Colour::DEFAULT),
                "scroll-down" => w.scrolldown(1, Colour::DEFAULT),
                "clear-end-screen" => w.clearendofscreen(Colour::DEFAULT),
                "clear-start-screen" => w.clearstartofscreen(Colour::DEFAULT),
                "clear-screen" => w.clearscreen(Colour::DEFAULT),
                "text" => {
                    w.collect_add(&GridCell {
                        data: Utf8Data::set(b'A'),
                        ..DEFAULT_CELL
                    });
                    w.collect_end();
                }
                _ => unreachable!(),
            });
            append_image_state(&mut expected, &f);
        }
    }
    for (width, height, x, y, sx, sy) in [
        (8, 5, 7, 1, 3, 1),
        (8, 5, 6, 4, 10, 7),
        (8, 5, 0, 4, 2, 2),
        (8, 1, 3, 0, 2, 3),
        (8, 5, 8, 0, 10, 2),
    ] {
        writeln!(
            script,
            "{width} {height} 0 0 {sx} {sy} {x} {y} 0 {} 0 image",
            height - 1
        )
        .unwrap();
        let mut f = Fixture::new(width, height);
        f.screen.cx = x;
        f.screen.cy = y;
        f.write(false, |w| w.sixelimage(image(sx, sy), Colour::DEFAULT));
        append_image_state(&mut expected, &f);
    }
    for operation in ["resize", "alternate", "reset"] {
        writeln!(script, "8 5 3 3 2 1 2 2 0 4 0 {operation}").unwrap();
        let mut f = Fixture::new(8, 5);
        f.store(3, 3, 2, 1);
        f.screen.cx = 2;
        f.screen.cy = 2;
        if operation == "resize" {
            f.screen
                .resize_cursor(8, 5, false, true, true, Some(&mut f.images));
        } else {
            f.screen
                .alternate_on(&DEFAULT_CELL, false, Some(&mut f.images));
            f.screen.cx = 0;
            f.screen.cy = 0;
            f.store(0, 0, 1, 1);
            if operation == "reset" {
                f.screen
                    .reinit(
                        false,
                        ScreenResetPolicy::default(),
                        &mut f.links,
                        Some(&mut f.images),
                    )
                    .unwrap();
            } else {
                f.screen.alternate_off(None, false, Some(&mut f.images));
            }
        }
        append_image_state(&mut expected, &f);
    }
    if std::env::var_os("RMUX_IMAGE_HOOK_MUTATE").is_some() {
        expected.push_str("mutated\n");
    }
    assert_eq!(
        common::run(&reference, &[], script.as_bytes()),
        expected.as_bytes()
    );
}

fn append_image_state(output: &mut String, f: &Fixture) {
    write!(output, "{} {}", f.screen.cx, f.screen.cy).unwrap();
    let owner = f.screen.image_owner().unwrap();
    for &id in f.images.ordered(owner) {
        let image = f.images.get(owner, id).unwrap();
        write!(
            output,
            " {},{},{},{}:",
            image.px, image.py, image.sx, image.sy
        )
        .unwrap();
        if let Some(encoded) = image.data.print(None) {
            for byte in encoded {
                write!(output, "{byte:02x}").unwrap();
            }
        }
    }
    output.push('\n');
}
