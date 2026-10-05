// Ported from tmux grid.c, grid-view.c and grid-reader.c @ 8f25579c
//! Unit tests for the G03 work-item checks that the C-reference and oracle
//! comparisons do not reach directly.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

use rmux_emu::cell::*;
use rmux_emu::colour::*;
use rmux_emu::grid::names::*;
use rmux_emu::grid::reader::GridReader;
use rmux_emu::grid::*;
use rmux_emu::hyperlinks::*;
use rmux_util::utf8::Utf8Data;

const RGB: i32 = 0x2000000;
const C256: i32 = 0x1000000;
const THEME: i32 = 0x4000000;

fn ch(text: &str, width: u8) -> GridCell {
    let mut gc = DEFAULT_CELL;
    gc.data = Utf8Data {
        size: text.len() as u8,
        have: text.len() as u8,
        width,
        ..Utf8Data::default()
    };
    gc.data.data[..text.len()].copy_from_slice(text.as_bytes());
    gc
}

fn text(gd: &mut Grid, py: u32, s: &str) {
    for (i, c) in s.chars().enumerate() {
        let mut b = [0u8; 4];
        let w = if (c as u32) >= 0x1100 { 2 } else { 1 };
        gd.set_cell(i as u32, py, &ch(c.encode_utf8(&mut b), w));
    }
}

fn row(gd: &Grid, py: u32) -> String {
    String::from_utf8(gd.string_cells(
        0,
        py,
        gd.sx(),
        &mut StringCellsCtx {
            last: None,
            flags: GridStringFlags::TRIM_SPACES,
            hyperlinks: None,
        },
    ))
    .unwrap()
}

#[test]
fn types_sizes_and_flag_values() {
    assert_eq!(std::mem::size_of::<GridCellEntry>(), 5);
    assert_eq!(std::mem::align_of::<GridCellEntry>(), 1);
    assert_eq!(GridLineFlags::WRAPPED.bits(), 0x1);
    assert_eq!(GridLineFlags::EXTENDED.bits(), 0x2);
    assert_eq!(GridLineFlags::DEAD.bits(), 0x4);
    assert_eq!(GridLineFlags::START_PROMPT.bits(), 0x8);
    assert_eq!(GridLineFlags::SECOND_PROMPT.bits(), 0x10);
    assert_eq!(GridLineFlags::START_COMMAND.bits(), 0x20);
    assert_eq!(GridLineFlags::START_OUTPUT.bits(), 0x40);
    assert_eq!(GridLineFlags::END_OUTPUT.bits(), 0x80);
    assert_eq!(GridLineFlags::HYPERLINK.bits(), 0x100);
    assert_eq!(
        GridLineFlags::OSC133_FLAGS.bits(),
        0x8 | 0x10 | 0x20 | 0x40 | 0x80
    );
    assert_eq!(GridStringFlags::WITH_SEQUENCES.bits(), 0x1);
    assert_eq!(GridStringFlags::ESCAPE_SEQUENCES.bits(), 0x2);
    assert_eq!(GridStringFlags::TRIM_SPACES.bits(), 0x4);
    assert_eq!(GridStringFlags::USED_ONLY.bits(), 0x8);
    assert_eq!(GridStringFlags::EMPTY_CELLS.bits(), 0x10);
    assert_eq!(GridFlags::HISTORY.bits(), 0x1);
    assert_eq!(LineTime(0).to_wall(1000), 0);
    assert_eq!(LineTime(5).to_wall(1000), 1004);
    let gd = Grid::new(3, 2, 0);
    assert_eq!(gd.flags, GridFlags(0));
    assert_eq!(Grid::new(3, 2, 7).flags, GridFlags::HISTORY);
    assert_eq!(
        Osc133Data::default(),
        Osc133Data {
            prompt_col: 0,
            cmd_col: 0,
            out_start_col: 0,
            out_end_col: 0,
            exit_status: 0
        }
    );
}

#[test]
fn codec_extended_conditions() {
    let mut gd = Grid::new(10, 1, 0);
    let mut c = ch("a", 1);
    c.flags = GridCellFlags::CLEARED | GridCellFlags::SELECTED;
    gd.set_cell(0, 0, &c);
    let e = gd.get_line(0).entries()[0];
    assert!(matches!(e.storage(), GridCellStorage::Compact(_)));
    assert_eq!(
        gd.get_cell(0, 0).flags,
        GridCellFlags::SELECTED,
        "CLEARED is stripped"
    );

    let extended = [
        GridCell {
            attr: GridAttributes(0x100),
            ..ch("a", 1)
        },
        ch("é", 1),
        ch("中", 2),
        GridCell {
            fg: Colour(RGB | 1),
            ..ch("a", 1)
        },
        GridCell {
            bg: Colour(THEME | 1),
            ..ch("a", 1)
        },
        GridCell {
            us: Colour(C256 | 1),
            ..ch("a", 1)
        },
        GridCell {
            link: HyperlinkId(3),
            ..ch("a", 1)
        },
        {
            let mut t = ch(" ", 1);
            t.set_tab(4);
            t
        },
    ];
    for (i, gc) in extended.iter().enumerate() {
        let px = i as u32 + 1;
        gd.set_cell(px, 0, gc);
        let e = gd.get_line(0).entries()[px as usize];
        assert!(
            matches!(e.storage(), GridCellStorage::Extended(_)),
            "cell {i}"
        );
        let back = gd.get_cell(px, 0);
        assert!(back.cells_equal(gc), "cell {i}");
        assert_eq!(back.us, gc.us);
    }
    assert!(
        gd.get_line(0)
            .flags
            .contains(GridLineFlags::EXTENDED | GridLineFlags::HYPERLINK)
    );
    assert_eq!(gd.get_line(0).extdsize(), 8);

    // 256-colour stays compact and decodes with COLOUR_FLAG_256.
    let c = GridCell {
        fg: Colour(C256 | 200),
        bg: Colour(C256 | 17),
        ..ch("b", 1)
    };
    gd.set_cell(9, 0, &c);
    let e = gd.get_line(0).entries()[9];
    assert_eq!(e.flags(), GridCellFlags::FG256 | GridCellFlags::BG256);
    assert_eq!(
        e.compact(),
        CompactCellData {
            attr: 0,
            fg: 200,
            bg: 17,
            data: b'b'
        }
    );
    let back = gd.get_cell(9, 0);
    assert_eq!(back.flags, GridCellFlags(0));
    assert_eq!((back.fg, back.bg), (Colour(C256 | 200), Colour(C256 | 17)));

    // Fresh padding is compact and reads with width 1; a reused extended
    // slot keeps width 0.
    gd.set_padding(0, 0, Colour(4));
    let p = gd.get_cell(0, 0);
    assert_eq!(p.flags, GridCellFlags::PADDING);
    assert_eq!((p.data.width, p.data.data[0], p.bg), (1, b'!', Colour(4)));
    gd.set_padding(3, 0, Colour::DEFAULT);
    let p = gd.get_cell(3, 0);
    assert_eq!(p.flags, GridCellFlags::PADDING);
    assert_eq!((p.data.width, p.data.size), (0, 0));

    // An existing extended entry keeps its slot.
    let before = gd.get_line(0).extdsize();
    gd.set_cell(2, 0, &ch("z", 1));
    assert_eq!(gd.get_line(0).extdsize(), before);
    assert!(gd.get_line(0).entries()[2].is_extended());

    // Equality ignores the underscore colour and CLEARED.
    let a = ch("x", 1);
    let b = GridCell {
        us: Colour(RGB),
        flags: GridCellFlags::CLEARED,
        ..a
    };
    assert!(a.look_equal(&b) && a.cells_equal(&b));
    assert!(!a.cells_equal(&ch("y", 1)));
    assert!(a.look_equal(&ch("y", 1)));
}

#[test]
fn expand_line_rounding_and_truncation() {
    let mut gd = Grid::new(80, 4, 0);
    for (py, (px, size)) in [(5, 20), (25, 40), (45, 80), (85, 86)]
        .into_iter()
        .enumerate()
    {
        gd.set_cell(px, py as u32, &ch("x", 1));
        assert_eq!(gd.get_line(py as u32).cellsize(), size);
        assert_eq!(gd.get_line(py as u32).cellused(), px + 1);
    }
    for (req, size) in [(19, 20), (20, 40), (39, 40), (40, 80)] {
        let mut gd = Grid::new(80, 1, 0);
        gd.set_cells(0, 0, &ch(" ", 1), &vec![b'a'; req]);
        assert_eq!(gd.get_line(0).cellsize(), size as u32);
    }
    let mut gd = Grid::new(1, 1, 0);
    gd.set_cells(0, 0, &ch(" ", 1), &vec![b'a'; 70000]);
    assert_eq!(gd.get_line(0).cellsize(), 70000 % 65536);
    assert_eq!(gd.get_line(0).cellused(), 70000 % 65536);
    assert_eq!(gd.get_line(0).entries().len(), 70000 % 65536);
}

#[test]
fn clear_cell_cases() {
    let mut gd = Grid::new(8, 1, 0);
    gd.set_cell(
        0,
        0,
        &GridCell {
            fg: Colour(RGB | 1),
            ..ch("a", 1)
        },
    );
    gd.set_cell(
        1,
        0,
        &GridCell {
            fg: Colour(RGB | 1),
            ..ch("b", 1)
        },
    );
    gd.set_cell(2, 0, &ch("c", 1));
    gd.set_cell(3, 0, &ch("d", 1));
    gd.set_cell(4, 0, &ch("e", 1));
    // Case 1: extended slot reused, bg replaced.
    gd.clear(0, 0, 1, 1, Colour(3));
    let e = gd.get_line(0).entries()[0];
    assert!(e.is_extended() && e.flags().contains(GridCellFlags::CLEARED));
    let gc = gd.get_cell(0, 0);
    assert_eq!(
        (gc.bg, gc.fg, gc.data.data[0]),
        (Colour(3), Colour::DEFAULT, b' ')
    );
    assert_eq!(gc.flags, GridCellFlags(0));
    // Case 2: new extended slot for RGB bg.
    let before = gd.get_line(0).extdsize();
    gd.clear(2, 0, 1, 1, Colour(RGB | 0x55));
    assert_eq!(gd.get_line(0).extdsize(), before + 1);
    assert_eq!(gd.get_cell(2, 0).bg, Colour(RGB | 0x55));
    // Case 3: compact bg with the 256 flag.
    gd.clear(3, 0, 1, 1, Colour(C256 | 9));
    let e = gd.get_line(0).entries()[3];
    assert_eq!(e.flags(), GridCellFlags::CLEARED | GridCellFlags::BG256);
    assert_eq!(gd.get_cell(3, 0).bg, Colour(C256 | 9));
    gd.clear(4, 0, 1, 1, Colour(5));
    assert_eq!(
        gd.get_line(0).entries()[4].compact(),
        CompactCellData {
            attr: 0,
            fg: 8,
            bg: 5,
            data: b' '
        }
    );
    assert_eq!(gd.get_cell(4, 0).flags, GridCellFlags::CLEARED);
    // moved = true: the source does not reuse the slot.
    gd.move_cells(5, 1, 0, 1, Colour(2));
    let src = gd.get_line(0).entries()[1];
    assert!(!src.is_extended());
    assert_eq!(gd.get_cell(1, 0).bg, Colour(2));
    assert_eq!(gd.get_cell(5, 0).fg, Colour(RGB | 1));
    assert_eq!(gd.get_cell(5, 0).data.data[0], b'b');
}

#[test]
fn compact_line_keeps_referenced_slots_in_order() {
    let mut gd = Grid::new(6, 1, 10);
    for px in 0..6 {
        gd.set_cell(
            px,
            0,
            &GridCell {
                fg: Colour(RGB | px as i32),
                ..ch("a", 1)
            },
        );
    }
    gd.move_cells(0, 3, 0, 3, Colour::DEFAULT);
    gd.clear(1, 0, 1, 1, Colour::DEFAULT);
    assert_eq!(gd.get_line(0).extdsize(), 6);
    gd.scroll_history(Colour::DEFAULT);
    let gl = gd.get_line(0);
    assert_eq!(gl.extdsize(), 3);
    let fgs: Vec<i32> = gl.extended_entries().iter().map(|e| e.fg.0).collect();
    assert_eq!(fgs, vec![RGB | 3, 8, RGB | 5]);
    for (i, e) in gl.entries().iter().take(3).enumerate() {
        assert_eq!(e.offset(), i as u32);
    }
    assert_eq!(gd.get_cell(1, 0).fg, Colour::DEFAULT);
    assert!(gl.flags.contains(GridLineFlags::EXTENDED));
    assert_eq!(gd.lines().len() as u32, gd.hsize() + gd.sy());
}

#[test]
fn history_limit_and_counters() {
    let mut gd = Grid::new(4, 2, 20);
    for i in 0..25 {
        let h = gd.hsize();
        text(&mut gd, h, &i.to_string());
        gd.collect_history(false);
        gd.scroll_history(Colour::DEFAULT);
        assert_eq!(gd.lines().len() as u32, gd.hsize() + gd.sy());
    }
    assert_eq!(gd.hsize(), 19);
    assert_eq!(gd.scroll_added, 25);
    assert_eq!(gd.scroll_collected, 6);
    assert_eq!(gd.hscrolled, 19);
    assert_eq!(row(&gd, 0), "6");
    assert_eq!(row(&gd, 18), "24");
    // collect(all) at exactly the limit removes one row.
    gd.scroll_history(Colour::DEFAULT);
    assert_eq!(gd.hsize(), 20);
    gd.collect_history(true);
    assert_eq!(gd.hsize(), 19);
    assert_eq!(gd.scroll_collected, 7);
    assert_eq!(row(&gd, 0), "7");
    // remove_history changes neither generation nor collected.
    let (generation, col) = (gd.scroll_generation, gd.scroll_collected);
    gd.remove_history(2);
    assert_eq!(
        (gd.hsize(), gd.scroll_generation, gd.scroll_collected),
        (17, generation, col)
    );
    assert_eq!(gd.lines().len() as u32, gd.hsize() + gd.sy());
    gd.remove_history(99);
    assert_eq!(gd.hsize(), 17);
    // clear_history drops the oldest prefix and keeps visible rows.
    let h = gd.hsize();
    text(&mut gd, h, "vis");
    gd.clear_history();
    assert_eq!(
        (gd.hsize(), gd.hscrolled, gd.scroll_generation),
        (0, 0, generation + 1)
    );
    assert_eq!(row(&gd, 0), "vis");
    assert_eq!(gd.lines().len(), 2);
    // Counter wrap.
    gd.scroll_added = u32::MAX;
    gd.scroll_generation = u32::MAX;
    gd.scroll_history(Colour::DEFAULT);
    assert_eq!(gd.scroll_added, 0);
    gd.reflow(4);
    assert_eq!(gd.scroll_generation, 0);
}

#[test]
fn scroll_region_and_line_time() {
    let mut gd = Grid::new(4, 5, 100);
    for py in 0..5 {
        text(&mut gd, py, &py.to_string());
    }
    gd.set_line_clock(LineTime(42));
    gd.scroll_history_region(1, 3, Colour(2));
    assert_eq!(gd.hsize(), 1);
    assert_eq!(gd.lines().len(), 6);
    let rows: Vec<String> = (0..6).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["1", "0", "2", "3", "", "4"]);
    assert_eq!(gd.get_line(0).time, LineTime(42));
    assert_eq!(gd.get_line(1).time, LineTime(0));
    assert_eq!(gd.get_cell(0, 4).bg, Colour(2));
    assert_eq!(gd.get_line(4).cellsize(), 4);
    assert_eq!(gd.hscrolled, 1);
    gd.set_line_clock(LineTime(0));
    gd.scroll_history(Colour::DEFAULT);
    assert_eq!(gd.get_line(1).time, LineTime(0));
    assert_eq!(gd.get_line(0).time.to_wall(100), 141);
}

#[test]
fn move_lines_wrap_flags_both_directions() {
    let mut gd = Grid::new(4, 6, 0);
    for py in 0..6 {
        text(&mut gd, py, &py.to_string());
        gd.get_line_mut(py).flags.insert(GridLineFlags::WRAPPED);
    }
    gd.move_lines(1, 3, 2, Colour::DEFAULT);
    assert!(
        !gd.get_line(0).flags.contains(GridLineFlags::WRAPPED),
        "above destination"
    );
    assert!(
        !gd.get_line(2).flags.contains(GridLineFlags::WRAPPED),
        "above source after move"
    );
    assert!(gd.get_line(1).flags.contains(GridLineFlags::WRAPPED));
    let rows: Vec<String> = (0..6).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["0", "3", "4", "", "", "5"]);
    let mut gd = Grid::new(4, 6, 0);
    for py in 0..6 {
        text(&mut gd, py, &py.to_string());
        gd.get_line_mut(py).flags.insert(GridLineFlags::WRAPPED);
    }
    gd.move_lines(4, 1, 2, Colour::DEFAULT);
    assert!(!gd.get_line(3).flags.contains(GridLineFlags::WRAPPED));
    assert!(!gd.get_line(0).flags.contains(GridLineFlags::WRAPPED));
    let rows: Vec<String> = (0..6).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["0", "", "", "3", "1", "2"]);
    // Overlapping move keeps the wrap flag above a source inside the destination.
    let mut gd = Grid::new(4, 6, 0);
    for py in 0..6 {
        text(&mut gd, py, &py.to_string());
        gd.get_line_mut(py).flags.insert(GridLineFlags::WRAPPED);
    }
    gd.move_lines(1, 2, 3, Colour::DEFAULT);
    assert!(gd.get_line(1).flags.contains(GridLineFlags::WRAPPED));
    assert!(!gd.get_line(0).flags.contains(GridLineFlags::WRAPPED));
    let rows: Vec<String> = (0..6).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["0", "2", "3", "4", "", "5"]);
}

#[test]
fn clear_lines_and_view_clear_history() {
    let mut gd = Grid::new(4, 3, 10);
    text(&mut gd, 0, "a");
    gd.get_line_mut(0).flags.insert(GridLineFlags::WRAPPED);
    text(&mut gd, 1, "b");
    gd.clear_lines(1, 1, Colour::DEFAULT);
    assert!(!gd.get_line(0).flags.contains(GridLineFlags::WRAPPED));
    assert_eq!(gd.get_line(1).cellsize(), 0);
    // No used row: hscrolled retained, nothing scrolled.
    let mut gd = Grid::new(4, 3, 10);
    gd.hscrolled = 7;
    gd.view_clear_history(Colour::DEFAULT);
    assert_eq!((gd.hsize(), gd.hscrolled, gd.scroll_added), (0, 7, 0));
    text(&mut gd, 1, "x");
    gd.view_clear_history(Colour(1));
    assert_eq!((gd.hsize(), gd.hscrolled, gd.scroll_added), (2, 0, 2));
    assert_eq!(row(&gd, 1), "x");
    assert_eq!(gd.get_cell(0, 2).bg, Colour(1));
}

#[test]
fn string_cells_sgr_and_escapes() {
    let mut gd = Grid::new(10, 1, 0);
    gd.set_cell(
        0,
        0,
        &GridCell {
            attr: GridAttributes::OVERLINE | GridAttributes::BRIGHT,
            ..ch("a", 1)
        },
    );
    gd.set_cell(
        1,
        0,
        &GridCell {
            attr: GridAttributes::CHARSET,
            fg: Colour(1),
            ..ch("\\", 1)
        },
    );
    gd.set_cell(
        2,
        0,
        &GridCell {
            bg: Colour(RGB | 0x010203),
            us: Colour(C256 | 7),
            ..ch("c", 1)
        },
    );
    gd.set_cell(3, 0, &ch(" ", 1));
    gd.set_cell(4, 0, &ch(" ", 1));
    gd.set_cell(6, 0, &ch(" ", 1));
    let mut last = DEFAULT_CELL;
    let out = gd.string_cells(
        0,
        0,
        10,
        &mut StringCellsCtx {
            last: Some(&mut last),
            flags: GridStringFlags::WITH_SEQUENCES
                | GridStringFlags::ESCAPE_SEQUENCES
                | GridStringFlags::TRIM_SPACES,
            hyperlinks: None,
        },
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "\\033[1;5:3ma\\033[0m\\033[31m\\016\\\\\\033[39m\\033[48;2;1;2;3m\\033[58;5;7m\\017c\\033[0m"
    );
    assert_eq!(last.data.data[0], b' ');
    // No sequences without a previous cell; empty cells only with EMPTY_CELLS.
    let plain = gd.string_cells(
        0,
        0,
        10,
        &mut StringCellsCtx {
            last: None,
            flags: GridStringFlags::WITH_SEQUENCES,
            hyperlinks: None,
        },
    );
    assert_eq!(plain, b"a\\c    ");
    let all = gd.string_cells(
        0,
        0,
        10,
        &mut StringCellsCtx {
            last: None,
            flags: GridStringFlags::EMPTY_CELLS,
            hyperlinks: None,
        },
    );
    assert_eq!(all.len(), 10);
    assert_eq!(gd.get_line(0).cellsize(), 10);
    assert!(
        gd.string_cells(
            0,
            5,
            10,
            &mut StringCellsCtx {
                last: None,
                flags: GridStringFlags(0),
                hyperlinks: None
            }
        )
        .is_empty()
    );
}

#[test]
fn string_cells_hyperlinks_end_close_and_limits() {
    let mut registry = HyperlinkRegistry::new();
    let store = registry.create().unwrap();
    let id = registry.put(&store, b"http://x/", Some(b"i")).unwrap();
    let long = vec![b'q'; 8192 - 17 - 9];
    let too_long = registry.put(&store, b"http://y/", Some(&long)).unwrap();
    let mut gd = Grid::new(10, 2, 0);
    gd.set_cell(
        0,
        0,
        &GridCell {
            link: id,
            ..ch("a", 1)
        },
    );
    gd.set_cell(
        0,
        1,
        &GridCell {
            link: too_long,
            attr: GridAttributes::BRIGHT,
            ..ch("b", 1)
        },
    );
    let mut last = DEFAULT_CELL;
    let flags = GridStringFlags::WITH_SEQUENCES | GridStringFlags::TRIM_SPACES;
    let out = gd.string_cells(
        0,
        0,
        10,
        &mut StringCellsCtx {
            last: Some(&mut last),
            flags,
            hyperlinks: Some((&registry, &store)),
        },
    );
    // The retained last-cell code (the open) is repeated before the close.
    assert_eq!(
        out,
        b"\x1b]8;id=i;http://x/\x1b\\a\x1b]8;id=i;http://x/\x1b\\\x1b]8;;\x1b\\".to_vec()
    );
    assert_eq!(last.link, id);
    let out = gd.string_cells(
        0,
        1,
        10,
        &mut StringCellsCtx {
            last: Some(&mut last),
            flags,
            hyperlinks: Some((&registry, &store)),
        },
    );
    assert_eq!(
        out,
        b"\x1b[1mb".to_vec(),
        "over-limit link is skipped and leaves has_link clear"
    );
    assert_eq!(last.link, too_long);
    let out = gd.string_cells(
        0,
        0,
        10,
        &mut StringCellsCtx {
            last: Some(&mut last),
            flags,
            hyperlinks: None,
        },
    );
    assert_eq!(
        out,
        b"\x1b[0ma".to_vec(),
        "no hyperlink store: no link sequences"
    );
}

#[test]
fn line_length_limit_and_in_set() {
    let mut gd = Grid::new(10, 1, 0);
    text(&mut gd, 0, "ab 中");
    gd.set_padding(4, 0, Colour::DEFAULT);
    gd.set_cell(5, 0, &ch(" ", 1));
    gd.set_cell(6, 0, &ch(" ", 1));
    assert_eq!(gd.line_length(0), 5);
    assert_eq!(gd.line_limit(0), 3);
    assert_eq!(
        gd.in_set(4, 0, b" "),
        0,
        "padding after a non-whitespace wide char"
    );
    assert_eq!(gd.in_set(3, 0, b" "), 0);
    assert_eq!(gd.in_set(3, 0, "中".as_bytes()), 1);
    assert_eq!(gd.in_set(2, 0, b"\t "), 1);
    let mut tab = ch(" ", 1);
    tab.set_tab(4);
    gd.set_cell(7, 0, &tab);
    assert_eq!(gd.in_set(7, 0, b"\t"), 4);
    assert_eq!(gd.in_set(7, 0, b"-"), 0);
    gd.set_padding(8, 0, Colour::DEFAULT);
    assert_eq!(gd.in_set(8, 0, b" "), 3);
    assert_eq!(gd.in_set(8, 0, b"-"), 0);
    text(&mut gd, 0, "\u{3000}");
    assert_eq!(gd.in_set(0, 0, b" "), 2, "unicode whitespace width");
    assert_eq!(gd.line_length(0), 9);
}

#[test]
fn reflow_cases_and_metadata() {
    // Split: metadata stays with the first part, continuation is fresh.
    let mut gd = Grid::new(6, 3, 100);
    text(&mut gd, 0, "abcdef");
    gd.get_line_mut(0).time = LineTime(9);
    gd.get_line_mut(0).osc133.prompt_col = 2;
    gd.get_line_mut(0).flags.insert(GridLineFlags::START_PROMPT);
    text(&mut gd, 1, "gh");
    text(&mut gd, 2, "ij");
    gd.hscrolled = 0;
    gd.reflow(4);
    assert_eq!(gd.sx(), 6, "reflow does not change sx");
    assert_eq!(gd.hsize(), 1);
    assert_eq!(gd.scroll_generation, 1);
    let rows: Vec<String> = (0..4).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["abcd", "ef", "gh", "ij"]);
    let first = gd.get_line(0);
    assert!(
        first
            .flags
            .contains(GridLineFlags::WRAPPED | GridLineFlags::START_PROMPT)
    );
    assert_eq!(
        (first.time, first.osc133.prompt_col, first.cellsize()),
        (LineTime(9), 2, 4)
    );
    let cont = gd.get_line(1);
    assert_eq!(
        (cont.time, cont.osc133.prompt_col, cont.flags),
        (LineTime(0), 0, GridLineFlags(0))
    );
    assert_eq!(
        cont.cellsize(),
        3,
        "target expand_line rounds with the old width (4 would give 4)"
    );
    assert_eq!(gd.hscrolled, 1);
    // Join back: consumed metadata is not merged; width == sx moves unchanged.
    gd.set_sx(4);
    gd.get_line_mut(1).time = LineTime(77);
    gd.reflow(6);
    gd.set_sx(6);
    let rows: Vec<String> = (0..3).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["abcdef", "gh", "ij"]);
    assert_eq!(gd.get_line(0).time, LineTime(9));
    assert!(!gd.get_line(0).flags.contains(GridLineFlags::WRAPPED));
    assert_eq!(gd.hsize(), 0);
    // Partial join leaves the remainder, empty wrapped line in the middle is skipped.
    let mut gd = Grid::new(8, 4, 100);
    text(&mut gd, 0, "abc");
    gd.get_line_mut(0).flags.insert(GridLineFlags::WRAPPED);
    gd.get_line_mut(1).flags.insert(GridLineFlags::WRAPPED);
    text(&mut gd, 2, "defghij");
    gd.get_line_mut(2).flags.insert(GridLineFlags::WRAPPED);
    text(&mut gd, 3, "k");
    gd.hscrolled = 3;
    gd.reflow(5);
    let rows: Vec<String> = (0..4).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["abcde", "fghij", "k", ""]);
    assert!(gd.get_line(0).flags.contains(GridLineFlags::WRAPPED));
    assert_eq!(gd.get_line(1).cellsize(), 5);
    assert_eq!(gd.hscrolled, 0);
    // Empty wrapped chain with no consumed nonempty line.
    let mut gd = Grid::new(4, 3, 10);
    text(&mut gd, 0, "ab");
    gd.get_line_mut(0).flags.insert(GridLineFlags::WRAPPED);
    gd.get_line_mut(1).flags.insert(GridLineFlags::WRAPPED);
    gd.get_line_mut(2).flags.insert(GridLineFlags::WRAPPED);
    gd.hscrolled = 0;
    gd.reflow(6);
    assert_eq!(gd.lines().len(), 3);
    assert!(gd.get_line(0).flags.contains(GridLineFlags::WRAPPED));
    assert!(gd.get_line(1).flags.contains(GridLineFlags::WRAPPED));
    // at == 0 sentinel: an oversize first cell lets a later cell set at.
    let mut gd = Grid::new(4, 2, 10);
    gd.set_cell(0, 0, &ch("中", 2));
    gd.set_padding(1, 0, Colour::DEFAULT);
    text(&mut gd, 1, "");
    gd.set_cell(2, 0, &ch("x", 1));
    gd.reflow(1);
    // The compact padding cell (width 1) sets `at = 1` after the oversize
    // first cell left it at 0, so the split keeps only the wide cell.
    assert_eq!(gd.get_line(0).cellused(), 1);
    assert_eq!(gd.get_line(1).cellused(), 1);
    assert_eq!(gd.get_line(2).cellused(), 1);
    assert_eq!(gd.lines().len(), 4);
}

#[test]
fn wrap_and_unwrap_positions() {
    let mut gd = Grid::new(4, 3, 10);
    text(&mut gd, 0, "abcd");
    gd.get_line_mut(0).flags.insert(GridLineFlags::WRAPPED);
    text(&mut gd, 1, "ef");
    text(&mut gd, 2, "g");
    assert_eq!(gd.wrap_position(1, 1), (5, 0));
    assert_eq!(gd.wrap_position(2, 1), (u32::MAX, 0));
    assert_eq!(gd.wrap_position(0, 2), (0, 1));
    assert_eq!(gd.unwrap_position(5, 0), (1, 1));
    assert_eq!(gd.unwrap_position(u32::MAX, 0), (2, 1));
    assert_eq!(gd.unwrap_position(u32::MAX, 1), (1, 2));
    assert_eq!(gd.unwrap_position(0, 1), (0, 2));
}

#[test]
fn reflow_ignores_resize_allocation_tail() {
    let mut gd = Grid::new(4, 2, 10);
    text(&mut gd, 0, "abcd");
    text(&mut gd, 1, "ef");
    gd.adjust_lines(4);
    text(&mut gd, 2, "tail");
    gd.clear_lines(0, 2, Colour::DEFAULT);
    gd.check_is_clear();
    text(&mut gd, 0, "abcd");
    text(&mut gd, 1, "ef");
    assert_eq!(gd.unwrap_position(u32::MAX, 2), (2, 1));
    gd.reflow(4);
    assert_eq!(gd.lines().len(), 2);
    assert_eq!(gd.hsize(), 0);
    assert_eq!(row(&gd, 0), "abcd");
    assert_eq!(row(&gd, 1), "ef");
}

#[test]
fn sets_stop_at_nul_and_padding_wraps() {
    let mut gd = Grid::new(4, 1, 0);
    gd.set_cell(0, 0, &ch(" ", 1));
    assert_eq!(gd.in_set(0, 0, b"\0 "), 0);
    gd.set_padding(0, 0, Colour::DEFAULT);
    assert_eq!(gd.in_set(0, 0, b" "), 0);
    gd.set_cell(0, 0, &ch("é", 1));
    assert_eq!(gd.in_set(0, 0, "é".as_bytes()), 1);
    assert_eq!(gd.in_set(0, 0, b"\xc3"), 0);
    gd.set_cell(
        0,
        0,
        &GridCell {
            data: Utf8Data::set(0xff),
            ..DEFAULT_CELL
        },
    );
    assert_eq!(gd.in_set(0, 0, b"\xc3\xff"), 1);
    gd.clear_lines(0, 1, Colour::DEFAULT);
    gd.scroll_history(Colour::DEFAULT);
    gd.check_is_clear();
}

#[test]
fn reader_exact_positions() {
    let mut gd = Grid::new(8, 3, 10);
    text(&mut gd, 0, "a中");
    gd.set_padding(2, 0, Colour::DEFAULT);
    gd.set_cell(3, 0, &ch("b", 1));
    gd.get_line_mut(0).flags.insert(GridLineFlags::WRAPPED);
    text(&mut gd, 1, "cd e");
    let mut gr = GridReader::new(&gd, 3, 0);
    gr.cursor_left(false);
    assert_eq!(
        gr.cursor(),
        (2, 0),
        "left from after a wide char ends on padding"
    );
    gr.cursor_left(false);
    assert_eq!(gr.cursor(), (0, 0));
    gr.cursor_left(false);
    assert_eq!(gr.cursor(), (0, 0));
    let mut gr = GridReader::new(&gd, 0, 1);
    gr.cursor_left(false);
    assert_eq!(
        gr.cursor(),
        (4, 0),
        "wrapped line above: end at line_length"
    );
    let mut gr = GridReader::new(&gd, 1, 0);
    assert!(gr.cursor_jump(&Utf8Data::set(b'b')));
    assert_eq!(gr.cursor(), (3, 0));
    assert!(
        gr.cursor_jump(&Utf8Data::set(b'b')),
        "jump includes the current cell"
    );
    assert!(!gr.cursor_jump(&Utf8Data::set(b'z')));
    assert_eq!(gr.cursor(), (3, 0), "failed jump leaves the cursor");
    assert!(
        gr.cursor_jump(&Utf8Data::set(b'e')),
        "jump follows the wrapped chain"
    );
    assert_eq!(gr.cursor(), (3, 1));
    assert!(gr.cursor_jump_back(&Utf8Data::set(b'a')));
    assert_eq!(gr.cursor(), (0, 0));
    let mut gr = GridReader::new(&gd, 0, 1);
    gr.cursor_end_of_line(false, false);
    assert_eq!(gr.cursor(), (4, 1), "one past the used text");
    gr.cursor_end_of_line(false, true);
    assert_eq!(gr.cursor(), (8, 1));
    let mut gr = GridReader::new(&gd, 0, 1);
    gr.cursor_right(false, false, false);
    gr.cursor_right(false, false, false);
    gr.cursor_right(false, false, false);
    assert_eq!(
        gr.cursor(),
        (3, 1),
        "limit stops before the trailing position"
    );
    gr.cursor_right(true, false, false);
    assert_eq!(gr.cursor(), (0, 2), "wrap to the next row at the limit");
    let mut gr = GridReader::new(&gd, 0, 0);
    gr.cursor_next_word(b"");
    assert_eq!(gr.cursor(), (0, 1), "next word crosses the wrapped chain");
    let mut gr = GridReader::new(&gd, 3, 1);
    gr.cursor_previous_word(b"", true, true);
    assert_eq!(gr.cursor(), (0, 1));
    let mut gr = GridReader::new(&gd, 0, 1);
    gr.cursor_previous_word(b"", true, true);
    assert_eq!(
        gr.cursor(),
        (0, 0),
        "stop_at_eol only stops on trailing whitespace"
    );
    let mut gr = GridReader::new(&gd, 3, 0);
    gr.cursor_next_word_end(b"");
    assert_eq!(gr.cursor(), (4, 0));
    gr.cursor_back_to_indentation();
    assert_eq!(gr.cursor(), (0, 0));
}

#[test]
fn compare_duplicate_and_storage_counts() {
    let mut a = Grid::new(5, 2, 0);
    let mut b = Grid::new(5, 2, 0);
    assert!(!a.compare(&b));
    text(&mut a, 0, "hi");
    assert!(a.compare(&b), "cellsize differs");
    text(&mut b, 0, "hi");
    assert!(!a.compare(&b));
    text(&mut b, 0, "ho");
    assert!(a.compare(&b));
    assert!(a.compare(&Grid::new(5, 3, 0)));

    let mut src = Grid::new(5, 3, 10);
    text(&mut src, 0, "one");
    src.set_cell(
        0,
        1,
        &GridCell {
            fg: Colour(RGB),
            ..ch("t", 1)
        },
    );
    src.get_line_mut(1).time = LineTime(3);
    src.scroll_history(Colour::DEFAULT);
    let mut dst = Grid::new(5, 2, 0);
    text(&mut dst, 1, "old");
    dst.duplicate_lines(0, &src, 1, 5);
    assert_eq!(row(&dst, 0), "t");
    assert_eq!(dst.get_line(0).time, LineTime(3));
    assert_eq!(dst.get_line(0).extdsize(), 1);
    assert_eq!(dst.get_line(0).cellsize(), src.get_line(1).cellsize());
    assert_eq!(row(&dst, 1), "");
    assert_eq!(dst.lines().len(), 2);

    assert_eq!(src.storage_counts(), (4, 7, 1));
    assert_eq!(src.history_bytes(), 4 * 40 + 7 * 5 + 23);
}

#[test]
fn view_formulas_and_wrapping_arithmetic() {
    let mut gd = Grid::new(4, 5, 0);
    for py in 0..5 {
        text(&mut gd, py, &py.to_string());
        gd.get_line_mut(py).flags.insert(GridLineFlags::WRAPPED);
    }
    // Region insert: ny2 = rlower + 1 - py - ny; the wrapped clear count
    // still removes the wrap flag above py + ny2.
    gd.view_insert_lines_region(3, 1, 1, Colour::DEFAULT);
    let rows: Vec<String> = (0..5).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["0", "", "1", "2", "4"]);
    assert!(!gd.get_line(0).flags.contains(GridLineFlags::WRAPPED));
    assert!(
        !gd.get_line(2).flags.contains(GridLineFlags::WRAPPED),
        "wrapped clear count clears the flag above py + ny2"
    );
    assert!(gd.get_line(3).flags.contains(GridLineFlags::WRAPPED));
    assert!(gd.get_line(4).flags.contains(GridLineFlags::WRAPPED));
    let mut gd = Grid::new(4, 5, 0);
    for py in 0..5 {
        text(&mut gd, py, &py.to_string());
    }
    gd.view_delete_lines_region(3, 1, 2, Colour::DEFAULT);
    let rows: Vec<String> = (0..5).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["0", "3", "", "", "4"]);
    gd.view_insert_lines(0, 2, Colour::DEFAULT);
    let rows: Vec<String> = (0..5).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["", "", "0", "3", ""]);
    gd.view_delete_lines(1, 1, Colour::DEFAULT);
    let rows: Vec<String> = (0..5).map(|py| row(&gd, py)).collect();
    assert_eq!(rows, ["", "0", "3", "", ""]);
    // Last-column insert clears one cell.
    let mut gd = Grid::new(4, 1, 0);
    text(&mut gd, 0, "abcd");
    gd.view_insert_cells(3, 0, 2, Colour(1));
    assert_eq!(row(&gd, 0), "abc");
    assert_eq!(gd.view_get_cell(3, 0).bg, Colour(1));
    gd.view_insert_cells(0, 0, 1, Colour::DEFAULT);
    assert_eq!(row(&gd, 0), " abc");
    gd.view_delete_cells(0, 0, 2, Colour::DEFAULT);
    assert_eq!(row(&gd, 0), "bc");
    assert_eq!(gd.view_string_cells(0, 0, 4), b"bc  ");
    // Scroll without history moves lines; with history it scrolls.
    let mut gd = Grid::new(4, 3, 0);
    text(&mut gd, 0, "a");
    gd.view_scroll_region_up(0, 2, Colour::DEFAULT);
    assert_eq!(
        (gd.hsize(), row(&gd, 0), row(&gd, 2)),
        (0, String::new(), String::new())
    );
    let mut gd = Grid::new(4, 3, 5);
    text(&mut gd, 0, "a");
    gd.view_scroll_region_up(0, 2, Colour::DEFAULT);
    assert_eq!((gd.hsize(), row(&gd, 0)), (1, "a".into()));
    gd.view_scroll_region_down(0, 2, Colour::DEFAULT);
    assert_eq!(row(&gd, 1), "");
    gd.view_set_cells(1, 0, &ch(" ", 1), b"xy");
    assert_eq!(row(&gd, 1), " xy");
    gd.view_set_padding(3, 0, Colour::DEFAULT);
    assert!(
        gd.view_get_cell(3, 0)
            .flags
            .contains(GridCellFlags::PADDING)
    );
    gd.view_clear(0, 0, 4, 1, Colour::DEFAULT);
    assert_eq!(gd.get_line(1).cellsize(), 0);
}

#[test]
fn names_match_c_text() {
    assert_eq!(line_flags_string(GridLineFlags(0)), "NONE");
    assert_eq!(
        line_flags_string(GridLineFlags::WRAPPED | GridLineFlags::HYPERLINK),
        "WRAPPED,HYPERLINK"
    );
    assert_eq!(line_flags_string(GridLineFlags(0x8000)), "NONE");
    assert_eq!(
        cell_flags_string(GridCellFlags::TAB | GridCellFlags::NOPALETTE | GridCellFlags::FG256),
        "FG256,TAB,NOPALETTE"
    );
    assert_eq!(
        cell_attr_string(GridAttributes::BRIGHT | GridAttributes::CHARSET),
        "CHARSET,BRIGHT"
    );
    assert_eq!(cell_attr_string(GridAttributes::NOATTR), "NONE");
}

#[test]
fn fuzz_invariants_hold() {
    let mut rng = common::Rng::new(0xfeed_beef);
    let mut gd = Grid::new(1 + rng.below(20) as u32, 1 + rng.below(8) as u32, 10);
    let mut ops = 0u64;
    let mut n = |b: u32| rng.below(u64::from(b.max(1))) as u32;
    while ops < 1_000_000 {
        let (sx, sy) = (gd.sx(), gd.sy());
        let total = gd.hsize() + sy;
        match n(100) {
            0..=49 => {
                let gc = match n(4) {
                    0 => ch("中", 2),
                    1 => GridCell {
                        fg: Colour(RGB | 3),
                        ..ch("a", 1)
                    },
                    2 => GridCell {
                        link: HyperlinkId(2),
                        ..ch("b", 1)
                    },
                    _ => ch("c", 1),
                };
                gd.set_cell(n(sx + 1), n(total), &gc);
            }
            50..=59 => gd.clear(n(sx), n(total), 1 + n(sx), 1, Colour(n(9) as i32)),
            60..=69 => {
                let py = n(sy);
                gd.view_clear(0, py, sx, 1 + n(sy - py), Colour::DEFAULT);
            }
            70..=74 => {
                let py = n(total);
                gd.move_cells(n(sx), n(sx), py, 1 + n(sx), Colour::DEFAULT);
            }
            75..=79 => {
                let py = n(sy);
                let ny = 1 + n(sy - py);
                gd.view_insert_lines(py, ny, Colour::DEFAULT);
            }
            80..=89 => {
                gd.collect_history(false);
                gd.scroll_history(Colour::DEFAULT);
            }
            90..=94 => {
                let (u, l) = (n(sy), n(sy));
                gd.view_scroll_region_up(u.min(l), u.max(l), Colour(1));
            }
            95..=97 => {
                let nsx = 1 + n(30);
                gd.reflow(nsx);
                gd.set_sx(nsx);
            }
            _ => gd.clear_history(),
        }
        ops += 1;
        if ops % 97 == 0 {
            assert_eq!(gd.lines().len() as u32, gd.hsize() + gd.sy());
            for gl in gd.lines() {
                assert!(gl.cellused() <= gl.cellsize());
                for e in gl.entries() {
                    if e.is_extended() {
                        assert!(e.offset() < gl.extdsize());
                    }
                }
            }
        }
    }
    assert_eq!(gd.lines().len() as u32, gd.hsize() + gd.sy());
}
