// Ported from tmux grid.c, colour.c, attributes.c, style.c and hyperlinks.c @ 8f25579c
use rmux_emu::{attributes::*, cell::*, colour::*, hyperlinks::*, style::*};
#[test]
fn cell_and_style_defaults_are_distinct() {
    assert_eq!(DEFAULT_CELL.data.data[0], b' ');
    assert_eq!(
        (
            DEFAULT_CELL.data.have,
            DEFAULT_CELL.data.size,
            DEFAULT_CELL.data.width
        ),
        (0, 1, 1)
    );
    assert_eq!(
        (DEFAULT_CELL.fg, DEFAULT_CELL.bg, DEFAULT_CELL.us),
        (Colour::DEFAULT, Colour::DEFAULT, Colour::DEFAULT)
    );
    assert_eq!(DEFAULT_CELL.link, HyperlinkId::NONE);
    assert_eq!(std::mem::size_of::<GridCell>(), 56);
    let style = Style::from_cell(DEFAULT_CELL);
    assert_eq!(style.gc, DEFAULT_CELL);
    assert_eq!(style.gc.us, Colour::DEFAULT);
    assert_eq!(Style::option_fallback().gc.us, Colour(0));
    assert!(!style.ignore);
    assert_eq!(
        (style.dim, style.fill, style.align, style.list),
        (0, Colour::DEFAULT, StyleAlign::Default, StyleList::Off)
    );
    assert_eq!(
        (style.range_type, style.range_argument, style.range_string),
        (StyleRangeType::None, 0, [0; 16])
    );
    assert_eq!(
        (
            style.width,
            style.width_percentage,
            style.pad,
            style.default_type,
            style.link
        ),
        (-1, false, -1, StyleDefaultType::Base, HyperlinkId::NONE)
    );
}
#[test]
fn palette_storage_precedence_and_checked_slots() {
    let mut palette = ColourPalette::new();
    assert!(!palette.set(0, Colour::NONE));
    let mut defaults = [Colour::NONE; 256];
    defaults[1] = Colour(4);
    defaults[8] = Colour(5);
    defaults[255] = Colour(6);
    palette.replace_defaults(Some(defaults));
    assert_eq!(palette.get(Colour(1)), Some(Colour(4)));
    assert_eq!(palette.get(Colour(90)), Some(Colour(5)));
    assert_eq!(palette.get(Colour(0x10000ff)), Some(Colour(6)));
    assert!(palette.set(1, Colour(3)));
    assert!(palette.set(1, Colour(3)));
    assert_eq!(palette.get(Colour(1)), Some(Colour(3)));
    assert!(palette.set(1, Colour::NONE));
    assert_eq!(palette.get(Colour(1)), Some(Colour(4)));
    for c in [-1, 8, 9, 256, i32::MIN, 0x3000001, 0x1000100] {
        assert_eq!(palette.get(Colour(c)), None);
    }
    assert!(!palette.set(256, Colour(1)));
    palette.fg = Colour(1);
    palette.bg = Colour(2);
    palette.clear_runtime();
    assert_eq!((palette.fg, palette.bg), (Colour::DEFAULT, Colour::DEFAULT));
    assert_eq!(palette.get(Colour(1)), Some(Colour(4)));
    palette.fg = Colour(2);
    palette.clear_storage();
    assert_eq!(palette.fg, Colour(2));
    assert_eq!(palette.get(Colour(1)), None);
    palette.replace_defaults(Some([Colour::NONE; 256]));
    assert_eq!(palette.get(Colour(1)), None);
    palette.replace_defaults(None);
    assert_eq!(palette.get(Colour(1)), None);
}
#[test]
fn overlays_defaults_and_ranges_preserve_omitted_fields() {
    let mut cell = DEFAULT_CELL;
    cell.flags = GridCellFlags::SELECTED;
    cell.link = HyperlinkId(8);
    cell.data.data[0] = b'X';
    cell.attr = GridAttributes::DIM;
    let mut style = Style::default();
    style.gc.fg = Colour(2);
    style.gc.attr = GridAttributes::NOATTR;
    style.gc.flags = GridCellFlags::TAB;
    style.gc.link = HyperlinkId(20);
    style.gc.data.data[0] = b'Z';
    style.overlay_cell(&mut cell);
    assert_eq!(cell.fg, Colour(2));
    assert_eq!(cell.attr, GridAttributes::DIM | GridAttributes::NOATTR);
    assert_eq!(cell.flags, GridCellFlags::SELECTED);
    assert_eq!(cell.link, HyperlinkId(8));
    assert_eq!(cell.data.data[0], b'X');
    let range = |start, end, argument| StyleRange {
        range_type: StyleRangeType::Window,
        argument,
        string: [0; 16],
        start,
        end,
    };
    let mut ranges = StyleRanges::new();
    ranges.push(range(1, 5, 1));
    ranges.push(range(2, 6, 2));
    ranges.push(range(6, 6, 3));
    assert_eq!(ranges.get_range(2).unwrap().argument, 1);
    assert_eq!(ranges.get_range(5).unwrap().argument, 2);
    assert!(ranges.get_range(6).is_none());
    ranges.clear();
    assert!(ranges.get_range(2).is_none());
    let mut links = HyperlinkRegistry::new();
    style.ignore = true;
    style.width = 10;
    style.gc.link = HyperlinkId(42);
    style.gc.data.data[0] = b'Q';
    style.parse(&cell, b"default", &mut links).unwrap();
    assert_eq!(style.gc.flags, cell.flags);
    assert_eq!(style.gc.link, HyperlinkId(42));
    assert_eq!(style.gc.data.data[0], b'Q');
    assert_eq!(style.width, 10);
    assert!(style.ignore);
}
#[test]
fn registry_lifecycle_identity_and_second_escape_limits() {
    let mut links = HyperlinkRegistry::new();
    let store = links.create().unwrap();
    let shared = links.share(&store).unwrap();
    let first = links.put(&store, b"a", Some(b"id")).unwrap();
    assert_eq!(first, HyperlinkId(1));
    links.release(store).unwrap();
    assert!(links.get(&shared, first).is_some());
    links.reset(&shared).unwrap();
    assert!(links.get(&shared, first).is_none());
    assert_eq!(links.put(&shared, b"b", None).unwrap(), HyperlinkId(2));
    links.release(shared).unwrap();
    assert_eq!(links.record_count(), 0);
    let mut other = HyperlinkRegistry::new();
    let foreign = other.create().unwrap();
    assert_eq!(links.reset(&foreign), Err(HyperlinkError::InvalidStore));
    assert!(links.get(&foreign, HyperlinkId(1)).is_none());
    let target = links.create().unwrap();
    let mut style = Style::default();
    let mut token = b"link=".to_vec();
    token.extend_from_slice(&[1; 250]);
    style.parse(&DEFAULT_CELL, &token, &mut links).unwrap();
    assert_eq!(style.link_uri(&links).unwrap().len(), 1000);
    assert_eq!(
        links.copy_style_link_to_store(&style, &target).unwrap(),
        HyperlinkId::NONE
    );
    let saved = style;
    let before = links.record_count();
    assert!(
        style
            .parse(
                &DEFAULT_CELL,
                b"link=failure-side-effect,invalid",
                &mut links
            )
            .is_err()
    );
    assert_eq!(style, saved);
    assert_eq!(links.record_count(), before + 1);
    for _ in 0..2000 {
        let s = links.create().unwrap();
        links.put(&s, b"reset", None).unwrap();
        links.reset(&s).unwrap();
        links.release(s).unwrap();
    }
    assert_eq!(links.record_count(), before + 1);
    let absent = Style::default();
    assert_eq!(
        links.copy_style_link_to_store(&absent, &target).unwrap(),
        HyperlinkId::NONE
    );
}
#[test]
fn deterministic_arbitrary_byte_parsers_and_registry_operations() {
    let mut rng = 0x879032u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let mut links = HyperlinkRegistry::new();
    let store = links.create().unwrap();
    let mut style = Style::default();
    for _ in 0..10000 {
        let n = (next() % 300) as usize;
        let bytes: Vec<_> = (0..n).map(|_| next() as u8).collect();
        let _ = parse_colour(&bytes);
        let _ = parse_x11_colour(&bytes);
        let _ = parse_attributes(&bytes);
        let _ = style.parse(&DEFAULT_CELL, &bytes, &mut links);
        let _ = style.parse_colour(&DEFAULT_CELL, &bytes);
        let _ = links.put(&store, &bytes, Some(&bytes));
        if next() % 8 == 0 {
            links.reset(&store).unwrap();
        }
        assert!(links.record_count() < 5000);
    }
}
