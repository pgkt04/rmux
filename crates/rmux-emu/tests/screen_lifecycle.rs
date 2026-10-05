// Ported from tmux screen.c @ 8f25579c
use rmux_emu::cell::{GridAttributes, GridCell, GridCellFlags};
use rmux_emu::colour::Colour;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::{
    ProgressBarState, Screen, ScreenCursorStyle, ScreenMode, ScreenResetPolicy, ScreenSelection,
    mode_to_string,
};

fn screen(registry: &mut HyperlinkRegistry, history: u32) -> Screen {
    Screen::new(20, 4, history, ScreenResetPolicy::default(), registry).unwrap()
}

#[test]
fn construction_reset_tabs_and_release() {
    let mut registry = HyperlinkRegistry::new();
    let mut s = screen(&mut registry, 10);
    assert_eq!((s.cx, s.cy, s.rupper, s.rlower), (0, 0, 0, 3));
    assert_eq!(s.mode, ScreenMode::CURSOR | ScreenMode::WRAP);
    assert_eq!(
        s.tabs
            .iter()
            .enumerate()
            .filter_map(|(x, &b)| b.then_some(x))
            .collect::<Vec<_>>(),
        [8, 16]
    );
    s.tabs[2] = true;
    s.mode.insert(ScreenMode::CRLF);
    s.push_title();
    s.reinit(
        false,
        ScreenResetPolicy {
            extended_keys: true,
        },
        &mut registry,
    )
    .unwrap();
    assert!(
        s.mode
            .contains(ScreenMode::CRLF | ScreenMode::KEYS_EXTENDED)
    );
    assert!(!s.tabs[2]);
    assert!(s.titles.0.is_empty());
    s.release(&mut registry).unwrap();
    s.release(&mut registry).unwrap();
    assert!(s.hyperlinks.is_none());
}

#[test]
fn metadata_title_bound_progress_and_cursor_styles() {
    let mut registry = HyperlinkRegistry::new();
    let mut s = screen(&mut registry, 0);
    assert!(s.set_title(b"#(hello)", true));
    assert_eq!(s.title, b"_(hello)");
    assert!(!s.set_title(b"bad\nname", false));
    assert_eq!(s.title, b"_(hello)");
    assert!(s.set_path(b"/tmp/example", false));
    assert!(!s.set_path(&[0xff], false));
    for n in 0..12 {
        s.set_title(n.to_string().as_bytes(), false);
        s.push_title();
    }
    assert_eq!(s.titles.0.len(), 10);
    for n in (2..12).rev() {
        s.pop_title();
        assert_eq!(s.title, n.to_string().as_bytes());
    }
    s.pop_title();
    assert_eq!(s.title, b"2");
    s.set_progress_bar(ProgressBarState::Normal, 150);
    s.set_progress_bar(ProgressBarState::Indeterminate, 2);
    assert_eq!(s.progress_bar.progress, 150);
    s.set_progress_bar(ProgressBarState::Paused, -1);
    assert_eq!(s.progress_bar.progress, 150);
    for (code, shape, blink) in [
        (1, ScreenCursorStyle::Block, true),
        (2, ScreenCursorStyle::Block, false),
        (3, ScreenCursorStyle::Underline, true),
        (4, ScreenCursorStyle::Underline, false),
        (5, ScreenCursorStyle::Bar, true),
        (6, ScreenCursorStyle::Bar, false),
    ] {
        s.set_cursor_style(code);
        assert_eq!(s.cstyle, shape);
        assert_eq!(s.mode.contains(ScreenMode::CURSOR_BLINKING), blink);
    }
    s.set_cursor_style(1);
    s.set_cursor_style(0);
    assert_eq!(s.cstyle, ScreenCursorStyle::Default);
    assert!(s.mode.contains(ScreenMode::CURSOR_BLINKING));
    s.set_cursor_style(99);
    assert_eq!(s.cstyle, ScreenCursorStyle::Default);
    s.set_default_cursor(Colour(42), 4);
    assert_eq!(s.default_cstyle, ScreenCursorStyle::Underline);
    assert_eq!(s.default_ccolour, Colour(42));
    s.release(&mut registry).unwrap();
}

#[test]
fn selection_branch_edges_and_style() {
    let mut registry = HyperlinkRegistry::new();
    let mut s = screen(&mut registry, 0);
    let selection = ScreenSelection {
        hidden: false,
        rectangle: false,
        modekeys: 0,
        sx: 3,
        sy: 0,
        ex: 0,
        ey: 2,
        clipx: 0,
        cell: GridCell::default(),
    };
    s.set_selection(selection.clone());
    assert!(!s.check_selection(2, 0));
    assert!(s.check_selection(3, 0));
    assert!(s.check_selection(0, 2));
    assert!(!s.check_selection(1, 2));
    s.set_selection(ScreenSelection {
        sx: 0,
        sy: 2,
        ex: 3,
        ey: 0,
        modekeys: 1,
        ..selection.clone()
    });
    assert!(!s.check_selection(0, 2));
    s.set_selection(ScreenSelection {
        rectangle: true,
        sx: 5,
        ex: 2,
        sy: 2,
        ey: 0,
        clipx: 3,
        ..selection
    });
    assert!(!s.check_selection(2, 1));
    assert!(s.check_selection(3, 1));
    assert!(s.check_selection(5, 2));
    let source = GridCell {
        fg: Colour(1),
        bg: Colour(2),
        attr: GridAttributes::BRIGHT | GridAttributes::CHARSET,
        flags: GridCellFlags::SELECTED,
        ..GridCell::default()
    };
    s.selection.as_mut().unwrap().cell = GridCell {
        fg: Colour::TERMINAL,
        attr: GridAttributes::NOATTR,
        us: Colour(7),
        ..GridCell::default()
    };
    let selected = s.select_cell(&source);
    assert_eq!(selected.fg, source.fg);
    assert_eq!(selected.bg, source.bg);
    assert_eq!(selected.flags, source.flags);
    assert!(selected.attr.contains(GridAttributes::CHARSET));
    assert!(!selected.attr.contains(GridAttributes::BRIGHT));
    assert_eq!(selected.us, Colour(7));
    s.hide_selection();
    assert!(!s.check_selection(3, 1));
    assert_eq!(s.select_cell(&source), source);
    s.clear_selection();
    assert!(s.selection.is_none());
    s.release(&mut registry).unwrap();
}

#[test]
fn resize_history_alt_and_no_alt_cursor_restore() {
    let mut registry = HyperlinkRegistry::new();
    let mut s = screen(&mut registry, 20);
    s.cy = 3;
    s.resize(20, 2, false);
    assert_eq!((s.grid.hsize(), s.grid.hscrolled, s.cy), (2, 2, 1));
    s.resize(20, 4, false);
    assert_eq!((s.grid.hsize(), s.grid.hscrolled, s.cy), (0, 0, 3));
    let cell = GridCell {
        fg: Colour(3),
        ..GridCell::default()
    };
    s.grid.view_set_cell(1, 1, &cell);
    s.cx = 7;
    s.cy = 1;
    assert!(s.alternate_on(&cell, true));
    assert!(!s.alternate_on(&cell, true));
    s.resize(10, 3, false);
    let mut restored = GridCell::default();
    assert!(s.alternate_off(Some(&mut restored), true));
    assert_eq!(restored, cell);
    assert_eq!((s.grid.sx(), s.grid.sy()), (10, 3));
    assert_eq!(s.grid.view_get_cell(1, 1).fg, cell.fg);
    s.cx = 100;
    s.cy = 100;
    assert!(!s.alternate_off(None, false));
    assert_eq!((s.cx, s.cy), (9, 2));
    assert!(!s.alternate_off(Some(&mut restored), true));
    assert_eq!((s.cx, s.cy), (7, 1));
    s.resize(0, 0, false);
    assert_eq!((s.grid.sx(), s.grid.sy()), (1, 1));
    s.release(&mut registry).unwrap();
}

#[test]
fn mode_and_reusable_print_diagnostics() {
    assert_eq!(mode_to_string(ScreenMode(0)), "NONE");
    assert_eq!(mode_to_string(ScreenMode::ALL_MODES), "ALL");
    assert_eq!(mode_to_string(ScreenMode(1 << 31)), "");
    assert_eq!(
        mode_to_string(ScreenMode::SYNC | ScreenMode::CURSOR | ScreenMode::CURSOR_VERY_VISIBLE),
        "CURSOR,CURSOR_VERY_VISIBLE,SYNC"
    );
    let mut registry = HyperlinkRegistry::new();
    let mut s = screen(&mut registry, 0);
    let mut cell = GridCell::default();
    cell.data.data[0] = b'X';
    s.grid.view_set_cell(0, 0, &cell);
    let mut out = Vec::new();
    assert_eq!(s.print(Some(0), &mut out, |_| None), b"0000 \"X\"\n");
    assert_eq!(s.print(Some(1), &mut out, |_| None), b"0001 \"\"\n");
    s.release(&mut registry).unwrap();
}

#[test]
fn tab_width_and_custom_stop_matrix() {
    let mut registry = HyperlinkRegistry::new();
    for width in [1, 7, 8, 9, 16, 17] {
        let mut s = Screen::new(width, 2, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        for x in 0..width {
            assert_eq!(s.tabs[x as usize], x > 0 && x % 8 == 0);
        }
        s.tabs[0] = true;
        s.resize(width, 3, true);
        assert!(s.tabs[0]);
        s.resize(width + 1, 3, false);
        assert!(!s.tabs[0]);
        s.release(&mut registry).unwrap();
    }
}

#[test]
fn resize_height_history_eat_empty_matrix() {
    let mut registry = HyperlinkRegistry::new();
    for history in [0, 10] {
        for eat_empty in [false, true] {
            let mut s = screen(&mut registry, history);
            s.cy = 1;
            s.resize_cursor(20, 2, false, eat_empty, true);
            let pushed = if history != 0 && !eat_empty { 2 } else { 0 };
            assert_eq!(s.grid.hsize(), pushed);
            assert_eq!(s.grid.hscrolled, pushed);
            let cy = if eat_empty { 1 } else { 0 };
            assert_eq!(s.cy, cy);
            assert_eq!((s.rupper, s.rlower), (0, 1));
            s.resize_cursor(20, 5, false, true, true);
            assert_eq!(s.grid.hsize(), 0);
            assert_eq!(s.grid.hscrolled, 0);
            assert_eq!(s.grid.view_get_cell(0, 4).bg, Colour(8));
            s.release(&mut registry).unwrap();
        }
    }
}

#[test]
fn selection_direction_mode_and_equal_endpoint_matrix() {
    let mut registry = HyperlinkRegistry::new();
    let mut s = screen(&mut registry, 0);
    for modekeys in [0, 1] {
        for rectangle in [false, true] {
            for reverse in [false, true] {
                let (sx, sy, ex, ey) = if reverse { (5, 2, 2, 0) } else { (2, 0, 5, 2) };
                s.set_selection(ScreenSelection {
                    hidden: false,
                    rectangle,
                    modekeys,
                    sx,
                    sy,
                    ex,
                    ey,
                    clipx: 0,
                    cell: GridCell::default(),
                });
                assert!(s.check_selection(2, 0));
                assert_eq!(s.check_selection(5, 2), rectangle || modekeys == 1);
                assert!(s.check_selection(3, 1));
                assert!(!s.check_selection(1, 0));
            }
            s.set_selection(ScreenSelection {
                hidden: false,
                rectangle,
                modekeys,
                sx: 3,
                sy: 1,
                ex: 3,
                ey: 1,
                clipx: 0,
                cell: GridCell::default(),
            });
            assert_eq!(s.check_selection(3, 1), rectangle || modekeys == 1);
            s.set_selection(ScreenSelection {
                hidden: false,
                rectangle,
                modekeys,
                sx: 0,
                sy: 0,
                ex: 0,
                ey: 0,
                clipx: 0,
                cell: GridCell::default(),
            });
            assert!(s.check_selection(0, 0));
        }
    }
    s.release(&mut registry).unwrap();
}
