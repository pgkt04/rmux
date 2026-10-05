use super::*;
use rmux_emu::hyperlinks::HyperlinkRegistry;

/// Draw `expanded` into a fresh `sx`-wide screen at `ocx` and return the
/// visible text (padding skipped), the final cursor and the ranges.
pub(super) struct Drawn {
    pub(super) text: String,
    pub(super) cells: Vec<GridCell>,
    pub(super) cursor: (u32, u32),
    pub(super) ranges: StyleRanges,
    pub(super) uris: Vec<Vec<u8>>,
}

pub(super) fn draw_into(
    expanded: &str,
    available: u32,
    sx: u32,
    ocx: u32,
    default_colours: bool,
) -> Drawn {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(sx, 1, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = ScreenOnlySink;
    let mut ranges = StyleRanges::new();
    {
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.cursormove(ocx as i32, 0, false);
        draw(
            &mut ctx,
            &DEFAULT_CELL,
            available,
            expanded.as_bytes(),
            Some(&mut ranges),
            default_colours,
        );
        ctx.finish();
    }
    let mut text = String::new();
    let mut cells = Vec::new();
    let mut uris = Vec::new();
    for x in 0..sx {
        let cell = screen.grid.view_get_cell(x, 0);
        if !cell.flags.contains(rmux_emu::cell::GridCellFlags::PADDING) {
            text.push_str(std::str::from_utf8(cell.data.bytes()).unwrap());
        }
        if cell.link != HyperlinkId::NONE {
            let store = screen.hyperlinks.as_ref().unwrap();
            uris.push(registry.get(store, cell.link).unwrap().uri().to_vec());
        }
        cells.push(cell);
    }
    let cursor = (screen.cx, screen.cy);
    screen
        .release(
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        )
        .unwrap();
    Drawn {
        text,
        cells,
        cursor,
        ranges,
        uris,
    }
}

#[test]
fn plain_text_and_cursor_restore() {
    let d = draw_into("abc", 6, 6, 2, false);
    assert_eq!(d.text, "  abc ");
    assert_eq!(d.cursor, (2, 0));
    let d = draw_into("", 4, 4, 1, false);
    assert_eq!(d.text, "    ");
    assert_eq!(d.cursor, (1, 0));
}

#[test]
fn alignment_sections_and_trimming() {
    assert_eq!(
        draw_into("L#[align=centre]C#[align=right]R", 7, 7, 0, false).text,
        "L  C  R"
    );
    assert_eq!(
        draw_into("LL#[align=centre]CC#[align=right]RR", 5, 5, 0, false).text,
        "LLCRR"
    );
    assert_eq!(
        draw_into("LLL#[align=right]RRR", 4, 4, 0, false).text,
        "LLLR"
    );
    assert_eq!(
        draw_into("abcdef#[align=right]xy", 3, 3, 0, false).text,
        "abc"
    );
    assert_eq!(
        draw_into(
            "#[align=absolute-centre]AB#[align=left]left",
            8,
            8,
            0,
            false
        )
        .text,
        "lefAB   "
    );
}

#[test]
fn hashes_styles_and_ignore() {
    assert_eq!(
        draw_into("a##b###c####d", 10, 10, 0, false).text,
        "a#b##c##d "
    );
    assert_eq!(draw_into("##[x]", 5, 5, 0, false).text, "#[x] ");
    assert_eq!(draw_into("###[fg=red]x", 5, 5, 0, false).text, "#x   ");
    assert_eq!(draw_into("#[bogus]x", 3, 3, 0, false).text, "x  ");
    assert_eq!(draw_into("a#[fg=red", 3, 3, 1, false).text, "   ");
    assert_eq!(
        draw_into("a#[ignore]b#[fg=red]c##d", 12, 12, 0, false).text,
        "ab#[fg=red]c"
    );
    let d = draw_into("a#[fg=red]b", 2, 2, 0, false);
    assert_eq!(d.cells[0].fg, Colour(8));
    assert_eq!(d.cells[1].fg, Colour(1));
    let d = draw_into("a#[fg=red,bg=blue,bold]b", 2, 2, 0, true);
    assert_eq!(d.cells[1].fg, Colour(8));
    assert_eq!(d.cells[1].bg, Colour(8));
    assert!(
        d.cells[1]
            .attr
            .contains(rmux_emu::cell::GridAttributes::BRIGHT)
    );
}

#[test]
fn utf8_and_control_bytes() {
    let d = draw_into("a\u{1F600}b\tc\u{0301}", 6, 6, 0, false);
    assert_eq!(d.text, "a\u{1F600}bc\u{0301} ");
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(4, 1, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
        #[cfg(feature = "sixel")]
        None,
    );
    draw(&mut ctx, &DEFAULT_CELL, 4, b"a\xe2\x82b\xffc", None, false);
    ctx.finish();
    let text: Vec<u8> = (0..4)
        .flat_map(|x| screen.grid.view_get_cell(x, 0).data.bytes().to_vec())
        .collect();
    assert_eq!(text, b"abc ");
}

#[test]
fn default_stack_and_fill() {
    let d = draw_into(
        "#[fg=red]a#[push-default]#[default]b#[pop-default]#[default]c",
        3,
        3,
        0,
        false,
    );
    assert_eq!(d.cells[0].fg, Colour(1));
    assert_eq!(
        d.cells[1].fg,
        Colour(1),
        "push-default makes red the default"
    );
    assert_eq!(d.cells[2].fg, Colour(8), "pop-default restores the base");
    let d = draw_into(
        "#[fg=red]#[set-default]a#[pop-default]#[default]b",
        2,
        2,
        0,
        false,
    );
    assert_eq!(d.cells[1].fg, Colour(1), "set-default replaces the base");
    let d = draw_into("#[fill=blue]ab", 5, 5, 0, false);
    assert_eq!(
        d.cells.iter().map(|c| c.bg).collect::<Vec<_>>(),
        [Colour(8), Colour(8), Colour(4), Colour(4), Colour(4)]
    );
    assert_eq!(d.text, "ab   ");
}

#[test]
fn links_go_to_the_target_store() {
    let d = draw_into(
        "#[link=http://x/]a#[nolink]b#[link=http://x/]c",
        3,
        3,
        0,
        false,
    );
    assert_eq!(d.uris, [b"http://x/".to_vec(), b"http://x/".to_vec()]);
    assert_ne!(d.cells[0].link, HyperlinkId::NONE);
    assert_eq!(d.cells[1].link, HyperlinkId::NONE);
    assert_eq!(
        d.cells[0].link, d.cells[2].link,
        "same URI shares one entry"
    );
}

#[test]
fn lists_focus_and_markers() {
    assert_eq!(
        draw_into("#[align=left,list=on]abcdef#[list=off]", 6, 6, 0, false).text,
        "abcdef"
    );
    assert_eq!(
        draw_into("#[align=left,list=on]abcdef#[list=off]", 4, 4, 0, false).text,
        "abcd",
        "left list keeps the start without focus"
    );
    assert_eq!(
        draw_into(
            "#[align=left,list=on]abcd#[list=focus]ef#[list=on]gh#[list=off]",
            4,
            4,
            0,
            false
        )
        .text,
        "defg"
    );
    assert_eq!(
        draw_into("#[align=left,list=on]abcdefgh#[list=left-marker]<#[list=right-marker]>#[list=on]#[list=focus]ij#[list=off]", 5, 5, 0, false).text,
        "abcd>"
    );
    assert_eq!(
        draw_into("#[align=left,list=on]ab#[list=focus]cd#[list=on]efgh#[list=left-marker]<#[list=right-marker]>#[list=off]", 5, 5, 0, false).text,
        "<cde>"
    );
    assert_eq!(
        draw_into(
            "L#[align=left,list=on]abcdef#[list=off]A#[align=right]R",
            8,
            8,
            0,
            false
        )
        .text,
        "LabcdefA"
    );
    assert_eq!(
        draw_into(
            "L#[align=centre,list=on]abc#[list=off]A#[align=right]R",
            9,
            9,
            0,
            false
        )
        .text,
        "L  abcAR "
    );
    assert_eq!(
        draw_into("L#[align=right,list=on]abc#[list=off]A", 9, 9, 0, false).text,
        "L    abcA"
    );
    assert_eq!(
        draw_into(
            "L#[align=absolute-centre,list=on]abc#[list=off]A#[align=right]R",
            9,
            9,
            0,
            false
        )
        .text,
        "L abcAR  "
    );
    assert_eq!(
        draw_into(
            "LLLL#[align=left,list=on]abc#[list=off]#[align=right]RRRR",
            8,
            8,
            0,
            false
        )
        .text,
        "LLLLabcR",
        "right-aligned text remains in the list's after section"
    );
}

#[test]
fn ranges_are_clipped_exclusive_and_offset() {
    let d = draw_into(
        "ab#[range=window|3]cd#[range=window|4]ef#[norange]g",
        7,
        7,
        0,
        false,
    );
    assert_eq!(d.ranges.0.len(), 2);
    assert_eq!(
        (d.ranges.0[0].range_type, d.ranges.0[0].argument),
        (StyleRangeType::Window, 3)
    );
    assert_eq!((d.ranges.0[0].start, d.ranges.0[0].end), (2, 4));
    assert_eq!((d.ranges.0[1].start, d.ranges.0[1].end), (4, 6));
    let d = draw_into("ab#[range=left]cd#[norange]", 10, 10, 3, false);
    assert_eq!(
        (d.ranges.0[0].start, d.ranges.0[0].end),
        (2, 4),
        "offsets are relative to the start"
    );
    let d = draw_into("ab#[range=user|xx]cd", 10, 10, 0, false);
    assert!(
        d.ranges.0.is_empty(),
        "an open range at the end is discarded"
    );
    let d = draw_into(
        "#[range=session|1]abcdef#[norange]#[align=right]#[range=pane|2]xyz#[norange]",
        4,
        4,
        0,
        false,
    );
    assert_eq!(d.text, "abcd");
    assert!(d.ranges.0.is_empty());
    let d = draw_into("#[range=left]ab#[list=on]c#[list=off]", 4, 4, 0, false);
    assert!(
        d.ranges.0.is_empty(),
        "entering the list aborts an open range"
    );
    let d = draw_into(
        "#[range=left]ab#[fg=red]#[range=right]cd#[norange]",
        4,
        4,
        0,
        false,
    );
    assert_eq!(
        d.ranges.0.len(),
        2,
        "a range type change closes and reopens"
    );
}

#[test]
fn width_counts_columns_and_hash_parity() {
    assert_eq!(width(b"abc"), 3);
    assert_eq!(width(b"a#[fg=red]b"), 2);
    assert_eq!(width(b"a#[fg=red"), 0);
    assert_eq!(width(b"##"), 1);
    assert_eq!(width(b"###"), 2);
    assert_eq!(width(b"##[x"), 3);
    assert_eq!(width(b"###[x]"), 1);
    assert_eq!(width("\u{1F600}\u{0301}".as_bytes()), 2);
    assert_eq!(width(b"a\x01b\x7fc"), 3);
    assert_eq!(
        width(b"\xe2\x82ab"),
        1,
        "a failed sequence swallows the byte after it"
    );
    assert_eq!(width(b"\xe2ab"), 0);
    assert_eq!(width(b"a\0b"), 1);
}

#[test]
fn trim_left_keeps_prefix_and_styles() {
    assert_eq!(trim_left(b"abcdef", 3), b"abc");
    assert_eq!(trim_left(b"a#[fg=red]bcd", 2), b"a#[fg=red]b");
    assert_eq!(trim_left(b"ab#[fg=red]cd", 2), b"ab");
    assert_eq!(trim_left(b"abc", 0), b"");
    assert_eq!(trim_left(b"abc", 10), b"abc");
    assert_eq!(trim_left("a\u{1F600}b".as_bytes(), 2), b"a");
    assert_eq!(
        trim_left("a\u{1F600}b".as_bytes(), 3),
        "a\u{1F600}".as_bytes()
    );
    assert_eq!(trim_left(b"##ab", 2), b"##a");
    assert_eq!(trim_left(b"####ab", 1), b"##");
    assert_eq!(trim_left(b"###ab", 1), b"##");
    assert_eq!(trim_left(b"#ab", 1), b"#");
    assert_eq!(trim_left(b"###[fg=red]ab", 2), b"###[fg=red]a");
    assert_eq!(trim_left(b"a#[fg=red", 5), b"a");
    assert_eq!(trim_left(b"a\x01\xe2\x82b", 5), b"ab");
}

#[test]
fn trim_right_keeps_suffix_and_styles() {
    assert_eq!(trim_right(b"abcdef", 3), b"def");
    assert_eq!(
        trim_right(b"ab\x01c", 5),
        b"ab\x01c",
        "unchanged input keeps nonprinting bytes"
    );
    assert_eq!(trim_right(b"a#[fg=red]bcd", 2), b"#[fg=red]cd");
    assert_eq!(trim_right(b"abc#[fg=red]d", 1), b"#[fg=red]d");
    assert_eq!(trim_right("a\u{1F600}b".as_bytes(), 2), b"b");
    assert_eq!(
        trim_right("ab\u{1F600}".as_bytes(), 2),
        "\u{1F600}".as_bytes()
    );
    assert_eq!(trim_right(b"ab####cd", 3), b"##cd");
    assert_eq!(trim_right(b"ab###cd", 3), b"##cd");
    assert_eq!(trim_right(b"a#b", 1), b"b");
    assert_eq!(trim_right(b"a#b", 2), b"#b");
    assert_eq!(trim_right(b"abc", 0), b"");
    assert_eq!(trim_right(b"abc#[fg=red", 1), b"abc#[fg=red");
}
