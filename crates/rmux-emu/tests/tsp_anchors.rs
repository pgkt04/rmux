// Ported from tmux grid.c, grid-view.c, screen.c and screen-write.c @ 8f25579c
//! Oracle-free TSP anchor lifetime matrix; ANSI rows remain ordinary grid rows.

use rmux_emu::cell::DEFAULT_CELL;
use rmux_emu::colour::Colour;
use rmux_emu::grid::{Grid, GridLineFlags, SurfaceAnchorId};
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::input::{InputCtx, InputPolicy, NullSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};

struct Fixture {
    screen: Screen,
    registry: HyperlinkRegistry,
    parser: InputCtx,
}

impl Fixture {
    fn new(width: u32, height: u32, history: u32) -> Self {
        let mut registry = HyperlinkRegistry::new();
        let screen = Screen::new(
            width,
            height,
            history,
            ScreenResetPolicy::default(),
            &mut registry,
        )
        .unwrap();
        Self {
            screen,
            registry,
            parser: InputCtx::new(),
        }
    }

    fn insert(&mut self, id: u64) -> bool {
        let mut tty = ScreenOnlySink;
        let mut writer = ScreenWriteCtx::start(
            &mut self.screen,
            &mut tty,
            ScreenWritePolicy::default(),
            &mut self.registry,
            #[cfg(feature = "sixel")]
            None,
        );
        let inserted = writer.insert_surface_anchor(SurfaceAnchorId(id));
        writer.finish();
        inserted
    }

    fn feed(&mut self, bytes: &[u8]) {
        let mut tty = ScreenOnlySink;
        let mut writer = ScreenWriteCtx::start(
            &mut self.screen,
            &mut tty,
            ScreenWritePolicy::default(),
            &mut self.registry,
            #[cfg(feature = "sixel")]
            None,
        );
        self.parser.parse(
            &mut writer,
            None,
            &InputPolicy::default(),
            &mut NullSink,
            bytes,
        );
        writer.finish();
    }

    fn removed(&mut self) -> Vec<SurfaceAnchorId> {
        self.screen.drain_surface_anchor_removals().collect()
    }
}

#[test]
fn insertion_ends_nonempty_rows_and_places_cursor_below_one_unwrapped_row() {
    for content in [b"".as_slice(), b"text", b"\x1b[4G", b"wide-wide"] {
        let mut f = Fixture::new(8, 5, 100);
        f.feed(content);
        let previous_y = f.screen.grid.hsize() + f.screen.cy;
        let nonempty = f.screen.cx != 0 || f.screen.grid.get_line(previous_y).cellused() != 0;
        assert!(f.insert(1));
        let row = f.screen.surface_anchor_row(SurfaceAnchorId(1)).unwrap();
        assert_eq!(row, previous_y + u32::from(nonempty));
        let line = f.screen.grid.get_line(row);
        assert_eq!(line.cellused(), 0);
        assert!(!line.flags.contains(GridLineFlags::WRAPPED));
        assert_eq!(f.screen.cx, 0);
        assert_eq!(f.screen.grid.hsize() + f.screen.cy, row + 1);
        assert!(f.removed().is_empty());
        assert!(!f.insert(1));
        assert!(f.insert(2));
        assert_ne!(
            f.screen.surface_anchor_row(SurfaceAnchorId(1)),
            f.screen.surface_anchor_row(SurfaceAnchorId(2))
        );
    }
}

#[test]
fn insertion_above_text_opens_a_blank_row_and_keeps_the_text() {
    // omp's ANSI paint leaves its status line right below the editor row,
    // where the cursor sits when it reopens a native surface.
    let mut f = Fixture::new(8, 5, 100);
    f.feed(b"top\r\neditor\r\nstatus\x1b[2;1H");
    assert!(f.insert(1));
    let row = f.screen.surface_anchor_row(SurfaceAnchorId(1)).unwrap();
    assert_eq!(row, 2);
    assert_eq!(f.screen.grid.get_line(row).cellused(), 0);
    let text = |y: u32| {
        let line = f.screen.grid.get_line(y);
        (0..line.cellused())
            .map(|x| f.screen.grid.get_cell(x, y).data.data[0] as char)
            .collect::<String>()
    };
    assert_eq!(text(1), "editor");
    assert_eq!(text(3), "status");
}

#[test]
fn anchor_scrolls_to_history_then_trim_and_clear_report_once() {
    let mut f = Fixture::new(8, 2, 2);
    assert!(f.insert(1));
    f.feed(b"one\r\ntwo");
    assert_eq!(
        f.screen.grid.surface_anchor_row(SurfaceAnchorId(1)),
        Some(0)
    );
    assert!(f.screen.grid.hsize() > 0);
    assert!(f.removed().is_empty());
    f.feed(b"\r\nthree\r\nfour");
    assert_eq!(f.removed(), vec![SurfaceAnchorId(1)]);
    assert!(f.removed().is_empty());

    let mut f = Fixture::new(8, 2, 100);
    assert!(f.insert(2));
    f.feed(b"\r\n");
    f.screen.grid.clear_history();
    assert_eq!(f.removed(), vec![SurfaceAnchorId(2)]);
}

#[test]
fn anchor_reflow_is_nonwrapping_and_never_joins_neighbor_text() {
    let mut grid = Grid::new(12, 4, 100);
    grid.set_cells(0, 0, &DEFAULT_CELL, b"abcdefghijkl");
    grid.get_line_mut(0).flags.insert(GridLineFlags::WRAPPED);
    assert!(grid.attach_surface_anchor(1, SurfaceAnchorId(1)));
    grid.set_cells(0, 2, &DEFAULT_CELL, b"mnopqr");
    for width in [3, 20, 5, 12] {
        grid.reflow(width);
        grid.set_sx(width);
        let row = grid.surface_anchor_row(SurfaceAnchorId(1)).unwrap();
        let anchor = grid.get_line(row);
        assert_eq!(anchor.cellused(), 0);
        assert_eq!(grid.line_length(row), width);
        assert!(!anchor.flags.contains(GridLineFlags::WRAPPED));
        if row != 0 {
            assert!(
                !grid
                    .get_line(row - 1)
                    .flags
                    .contains(GridLineFlags::WRAPPED)
            );
        }
        let text: Vec<_> = grid
            .lines()
            .iter()
            .flat_map(|line| (0..line.cellused()).map(|column| line.get_cell(column).data.data[0]))
            .collect();
        assert_eq!(text, b"abcdefghijklmnopqr");
        assert_eq!(grid.drain_surface_anchor_removals().count(), 0);
    }
}

#[test]
fn shrink_keeps_anchor_below_cursor_instead_of_eating_blank_row() {
    let mut f = Fixture::new(12, 5, 100);
    assert!(f.screen.grid.attach_surface_anchor(4, SurfaceAnchorId(1)));
    f.screen.resize(
        5,
        2,
        true,
        #[cfg(feature = "sixel")]
        None,
    );
    assert!(f.screen.surface_anchor_row(SurfaceAnchorId(1)).is_some());
    assert!(f.removed().is_empty());
    f.screen.resize(
        20,
        6,
        true,
        #[cfg(feature = "sixel")]
        None,
    );
    assert!(f.screen.surface_anchor_row(SurfaceAnchorId(1)).is_some());
    assert!(f.removed().is_empty());
}

#[test]
fn line_moves_preserve_source_ownership_and_report_overwritten_destination() {
    for (source, destination) in [(0, 2), (2, 0), (0, 1), (1, 0)] {
        let mut grid = Grid::new(8, 5, 0);
        assert!(grid.attach_surface_anchor(source, SurfaceAnchorId(1)));
        let overwritten = if destination > source {
            destination + 1
        } else {
            destination
        };
        assert!(grid.attach_surface_anchor(overwritten, SurfaceAnchorId(2)));
        grid.move_lines(destination, source, 2, Colour::DEFAULT);
        assert_eq!(
            grid.surface_anchor_row(SurfaceAnchorId(1)),
            Some(destination)
        );
        assert_eq!(
            grid.drain_surface_anchor_removals().collect::<Vec<_>>(),
            vec![SurfaceAnchorId(2)]
        );
    }
}

#[test]
fn erase_and_buffer_clear_lifetime_matrix() {
    for erase in [
        b"\x1b[2J".as_slice(),
        b"\x1b[1;1H\x1b[K",
        b"\x1b[1;2H\x1b[X",
        b"\x1b[1;1H\x1b[M",
        b"\x1bc",
    ] {
        let mut f = Fixture::new(8, 4, 100);
        assert!(f.insert(1));
        f.feed(erase);
        assert_eq!(f.removed(), vec![SurfaceAnchorId(1)], "{erase:?}");
        assert!(f.removed().is_empty());
    }
    let mut f = Fixture::new(8, 4, 100);
    assert!(f.insert(1));
    f.screen.grid.view_clear_history(Colour::DEFAULT);
    assert_eq!(f.removed(), vec![SurfaceAnchorId(1)]);
}
#[test]
fn scroll_region_removes_anchor_but_full_screen_history_scroll_retains_it() {
    for history in [0, 100] {
        let mut grid = Grid::new(8, 4, history);
        assert!(grid.attach_surface_anchor(1, SurfaceAnchorId(1)));
        grid.view_scroll_region_up(1, 2, Colour::DEFAULT);
        assert_eq!(grid.surface_anchor_row(SurfaceAnchorId(1)), None);
        assert_eq!(
            grid.drain_surface_anchor_removals().collect::<Vec<_>>(),
            vec![SurfaceAnchorId(1)]
        );
        assert!(grid.attach_surface_anchor(grid.hsize() + 2, SurfaceAnchorId(2)));
        grid.view_scroll_region_down(1, 2, Colour::DEFAULT);
        assert_eq!(
            grid.drain_surface_anchor_removals().collect::<Vec<_>>(),
            vec![SurfaceAnchorId(2)]
        );
    }
    let mut grid = Grid::new(8, 2, 100);
    assert!(grid.attach_surface_anchor(0, SurfaceAnchorId(3)));
    grid.view_scroll_region_up(0, 1, Colour::DEFAULT);
    assert_eq!(grid.surface_anchor_row(SurfaceAnchorId(3)), Some(0));
    assert_eq!(grid.drain_surface_anchor_removals().count(), 0);
}

#[test]
fn alternate_buffer_transfers_main_ownership_while_copy_grid_does_not() {
    let mut f = Fixture::new(8, 4, 100);
    assert!(f.insert(1));
    let mut copy = Grid::new(8, 4, 0);
    copy.duplicate_lines(0, &f.screen.grid, f.screen.grid.hsize(), 4);
    assert!(copy.surface_anchor_row(SurfaceAnchorId(1)).is_none());
    assert!(!copy.compare(&f.screen.grid));
    f.feed(b"\x1b[?1049h");
    assert!(f.screen.is_alternate());
    assert!(
        f.screen
            .grid
            .surface_anchor_row(SurfaceAnchorId(1))
            .is_none()
    );
    assert_eq!(f.screen.surface_anchor_row(SurfaceAnchorId(1)), Some(0));
    assert!(!f.insert(2));
    f.feed(b"editor\x1b[2J");
    assert!(f.removed().is_empty());
    f.feed(b"\x1b[?1049l");
    assert!(!f.screen.is_alternate());
    assert_eq!(
        f.screen.grid.surface_anchor_row(SurfaceAnchorId(1)),
        Some(0)
    );
    assert!(f.removed().is_empty());
    f.feed(b"\x1b[?1049h");
    assert!(f.screen.remove_surface_anchor(SurfaceAnchorId(1)));
    f.feed(b"\x1b[?1049l");
    assert_eq!(f.removed(), vec![SurfaceAnchorId(1)]);
}

#[test]
fn explicit_close_prompt_reset_and_storage_truncation_remove_ownership() {
    let mut f = Fixture::new(8, 4, 100);
    assert!(f.insert(1));
    assert!(f.screen.remove_surface_anchor(SurfaceAnchorId(1)));
    assert!(!f.screen.remove_surface_anchor(SurfaceAnchorId(1)));
    assert_eq!(f.removed(), vec![SurfaceAnchorId(1)]);
    assert!(f.insert(2));
    f.feed(b"\x1b[?1049h");
    f.screen.clear_surface_anchors();
    assert_eq!(f.removed(), vec![SurfaceAnchorId(2)]);
    f.feed(b"\x1b[?1049l");
    assert!(f.insert(3));
    f.screen.grid.adjust_lines(0);
    assert_eq!(f.removed(), vec![SurfaceAnchorId(3)]);
}

#[test]
fn absent_anchors_preserve_ansi_grid_and_resize_arithmetic() {
    let mut f = Fixture::new(4, 3, 100);
    f.feed(b"abcdef\r\nxy");
    assert!(
        f.screen
            .grid
            .lines()
            .iter()
            .all(|line| line.surface_anchor().is_none())
    );
    f.screen.resize(
        8,
        3,
        true,
        #[cfg(feature = "sixel")]
        None,
    );
    assert_eq!(f.screen.grid.get_line(0).cellused(), 6);
    assert_eq!(f.screen.grid.get_line(1).cellused(), 2);
    assert_eq!(f.screen.grid.hsize(), 0);
    assert_eq!((f.screen.cx, f.screen.cy), (2, 1));
    assert!(f.removed().is_empty());
}
