// Ported from tmux colour.c, attributes.c, style.c and hyperlinks.c @ 8f25579c
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;
use rmux_emu::{attributes::*, cell::*, colour::*, hyperlinks::*, style::*};
use rmux_util::bytes::cstr;
use std::{fmt::Write, path::Path};

fn reference() -> Option<std::path::PathBuf> {
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference.c");
    common::build_c(
        "cells",
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
        &[],
        cfg!(target_os = "macos"),
    )
}
fn hex(bytes: &[u8]) -> String {
    let mut s = String::new();
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}
fn optional_hex(bytes: Option<&[u8]>) -> String {
    bytes.map_or_else(|| "-".into(), hex)
}
fn command(input: &mut String, op: &str, bytes: &[u8]) {
    writeln!(input, "{op} {}", hex(bytes)).unwrap();
}
fn compare(input: &str, expected: &str) {
    let Some(bin) = reference() else {
        return;
    };
    let actual = common::run(&bin, &[], input.as_bytes());
    let actual = String::from_utf8(actual).unwrap();
    let commands: Vec<_> = input.lines().collect();
    let lines: Vec<_> = actual.lines().collect();
    let expected: Vec<_> = expected.lines().collect();
    assert_eq!(lines.len(), expected.len());
    for (n, (a, b)) in lines.iter().zip(&expected).enumerate() {
        assert_eq!(a, b, "command {}: {}", n, commands[n]);
    }
}
#[test]
fn colours_and_attributes_match_pinned_c() {
    let mut input = String::new();
    let mut expected = String::new();
    let mut corpus: Vec<Vec<u8>> = X11_NAMES
        .iter()
        .flat_map(|(name, _)| [name.to_vec(), name.to_ascii_uppercase()])
        .collect();
    for prefix in ["grey", "gray"] {
        for n in 0..=101 {
            corpus.push(format!("{prefix}{n}").into_bytes());
        }
    }
    for n in -1..=256 {
        corpus.push(format!("colour{n}").into_bytes());
        corpus.push(format!("color+{n}").into_bytes());
    }
    for s in [
        "",
        "none",
        "default",
        "terminal",
        "themeblack",
        "themewhite",
        "themelightgrey",
        "themedarkgrey",
        "themegreen",
        "themeyellow",
        "themered",
        "themeblue",
        "themecyan",
        "thememagenta",
        "0",
        "7",
        "90",
        "97",
        "+1",
        "01",
        "colour",
        "color",
        "colour 2",
        "colour\t+2",
        "colour2 ",
        "colour999999999999999999999",
        "red ",
        "#123456",
        "#abcdef",
        "#12345g",
        "grey-0",
        "gray+50",
        "grey 1",
        "red\0blue",
        "Gréén",
    ] {
        corpus.push(s.as_bytes().to_vec());
    }
    for bytes in corpus {
        for (op, result) in [
            ("C", parse_colour(&bytes)),
            ("N", colour_by_name(&bytes)),
            ("X", parse_x11_colour(&bytes)),
        ] {
            command(&mut input, op, &bytes);
            writeln!(expected, "{}", result.map_or(-1, Colour::raw)).unwrap();
        }
    }
    let scans = [
        "rgb:12/34/56",
        "rgb:1/2/3xxx",
        "rgb:1234/5678/9abc",
        "#123456",
        "#123456789abc",
        "1,2,3garbage",
        "-1,+256, 511",
        " 1,2,3",
        "1 ,2,3",
        "cmy:0/0.5/1tail",
        "cmyk:0.2/0.4/0.6/0.5",
        "cmy:0x1p-1/0/0",
        "cmy:1e-1/0/1",
        "cmy:1e+/0/0",
        "cmy:NaN/0/0",
        "cmyk:0/0/0/1.1",
        " rgb:12/34/56 ",
        " green ",
        "\tgreen\t",
        "rgb: 1/ 2/ 3",
        "#1 2 3 ",
        "rgb:+1/00/ff",
        "cmy:0/0/0junk",
        "RGB:12/34/56",
        "99999999999999999999,0,0",
        "-99999999999999999999,1,2",
        "4294967296,2147483648,-2147483649",
        "0x10,010,1",
        "rgb:0x/0x/0x",
        "rgb:0x1/0x/00",
        "#0x0x0x",
    ];
    for s in scans {
        command(&mut input, "X", s.as_bytes());
        writeln!(
            expected,
            "{}",
            parse_x11_colour(s.as_bytes()).map_or(-1, Colour::raw)
        )
        .unwrap();
    }
    let mut rng = common::Rng::new(0x6c02);
    let hex_alphabet = b"0123456789abcdefABCDEF xX+-/#:g";
    let decimal_alphabet = b"0123456789 +-,xX\t";
    fn pick(rng: &mut common::Rng, alphabet: &[u8], width: u64) -> Vec<u8> {
        let len = rng.below(width + 1);
        (0..len)
            .map(|_| alphabet[rng.below(alphabet.len() as u64) as usize])
            .collect()
    }
    for _ in 0..3000 {
        let mut forms = Vec::new();
        for sep in [b"/".as_slice(), b""] {
            for prefix in [b"rgb:".as_slice(), b"#"] {
                let mut bytes = prefix.to_vec();
                bytes.extend(pick(&mut rng, hex_alphabet, 5));
                bytes.extend_from_slice(sep);
                bytes.extend(pick(&mut rng, hex_alphabet, 5));
                bytes.extend_from_slice(sep);
                bytes.extend(pick(&mut rng, hex_alphabet, 5));
                forms.push(bytes);
            }
        }
        let mut decimal = pick(&mut rng, decimal_alphabet, 4);
        decimal.push(b',');
        decimal.extend(pick(&mut rng, decimal_alphabet, 4));
        decimal.push(b',');
        decimal.extend(pick(&mut rng, decimal_alphabet, 4));
        forms.push(decimal);
        for bytes in forms {
            command(&mut input, "X", &bytes);
            writeln!(
                expected,
                "{}",
                parse_x11_colour(&bytes).map_or(-1, Colour::raw)
            )
            .unwrap();
        }
    }
    for slot in 0..=11 {
        for theme in [ClientTheme::Unknown, ClientTheme::Dark, ClientTheme::Light] {
            writeln!(input, "theme {slot} {}", theme as i32).unwrap();
            writeln!(
                expected,
                "{} {}",
                optional_hex(theme_option(slot, theme).map(str::as_bytes)),
                theme_terminal_colour(slot).raw()
            )
            .unwrap();
        }
    }
    let floats = [
        "0", "1", ".5", "0.1", "0.3", "0.999", "1e-1", "0x1p-1", "nan", "inf", "-0", "+.25", "1e+",
        "-1", "1.1",
    ];
    for _ in 0..2000 {
        let a = floats[rng.below(floats.len() as u64) as usize];
        let b = floats[rng.below(floats.len() as u64) as usize];
        let c = floats[rng.below(floats.len() as u64) as usize];
        let bytes = format!("cmy:{a}/{b}/{c}tail");
        command(&mut input, "X", bytes.as_bytes());
        writeln!(
            expected,
            "{}",
            parse_x11_colour(bytes.as_bytes()).map_or(-1, Colour::raw)
        )
        .unwrap();
    }
    let mut raw = vec![
        -1,
        0,
        7,
        8,
        9,
        90,
        97,
        98,
        255,
        256,
        ColourFlags::THEME.bits() as i32 | 255,
        ColourFlags::THEME.bits() as i32 | ColourFlags::RGB.bits() as i32 | 1,
    ];
    raw.extend((0..256).map(|n| n | ColourFlags::_256.bits() as i32));
    raw.extend((0..10).map(|n| n | ColourFlags::THEME.bits() as i32));
    raw.extend((0..3000).map(|_| {
        Colour::rgb(
            rng.below(256) as u8,
            rng.below(256) as u8,
            rng.below(256) as u8,
        )
        .raw()
    }));
    for n in raw {
        for dim in [0, 1, 99, 100, 101] {
            writeln!(input, "R {n} {dim}").unwrap();
            let c = Colour(n);
            let mut text = Vec::new();
            write_colour(c, &mut text);
            writeln!(
                expected,
                "{} {} {} {} {} {}",
                indexed_to_rgb(c).raw(),
                indexed_to_16(c),
                c.force_rgb().map_or(-1, Colour::raw),
                c.dim(dim).map_or(-1, Colour::raw),
                c.theme() as i32,
                hex(&text)
            )
            .unwrap();
        }
        for flags in [0, 1, 16, 17] {
            writeln!(input, "Q {n} {flags}").unwrap();
            let mut fg = Vec::new();
            let mut bg = Vec::new();
            let a = write_colour_escape(Colour(n), false, Some(&[Colour(0); 10]), &mut fg);
            let b = write_colour_escape(Colour(n), true, Some(&[Colour(0); 10]), &mut bg);
            writeln!(
                expected,
                "{} {}",
                optional_hex(a.then_some(fg.as_slice())),
                optional_hex(b.then_some(bg.as_slice()))
            )
            .unwrap();
        }
    }
    for _ in 0..10000 {
        let (r, g, b) = (
            rng.below(256) as u8,
            rng.below(256) as u8,
            rng.below(256) as u8,
        );
        writeln!(input, "F {r}:{g}:{b}").unwrap();
        writeln!(expected, "{}", find_rgb(r, g, b).raw()).unwrap();
    }
    for bits in 0..=u16::MAX {
        writeln!(input, "V {bits}").unwrap();
        let mut out = Vec::new();
        write_attributes(GridAttributes(bits), &mut out);
        writeln!(expected, "{}", hex(&out)).unwrap();
    }
    for s in [
        "",
        "none",
        "default",
        "bold",
        "BRIGHT",
        "acs,bright,dim,underscore,blink,reverse,hidden,italics,strikethrough,double-underscore,curly-underscore,dotted-underscore,dashed-underscore,overline",
        "bright||dim",
        "bright, | dim",
        " bright",
        "bright ",
        "bright|",
        "noattr",
        "bright,none",
        "dim\tbright",
        "none\0bad",
    ] {
        command(&mut input, "A", s.as_bytes());
        writeln!(
            expected,
            "{}",
            parse_attributes(s.as_bytes()).map_or(-1, |a| i32::from(a.bits()))
        )
        .unwrap();
    }
    compare(&input, &expected);
}
fn base() -> GridCell {
    let mut cell = DEFAULT_CELL;
    cell.fg = Colour(1);
    cell.bg = Colour(2);
    cell.us = Colour(3);
    cell.attr = GridAttributes::BRIGHT;
    cell.flags = GridCellFlags::SELECTED;
    cell.link = HyperlinkId(77);
    cell.data.data[0] = b'x';
    cell
}
fn state(s: &Style, links: &HyperlinkRegistry) -> String {
    let mut text = Vec::new();
    s.write_text(links, &mut text);
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
        s.gc.fg.raw(),
        s.gc.bg.raw(),
        s.gc.us.raw(),
        s.gc.attr.bits(),
        s.gc.flags.bits(),
        s.gc.link.0,
        i32::from(s.ignore),
        s.dim,
        s.fill.raw(),
        s.align as i32,
        s.range_type as i32,
        s.range_argument,
        hex(cstr(&s.range_string)),
        s.width,
        i32::from(s.width_percentage),
        s.pad,
        s.default_type as i32,
        s.link.0,
        hex(&text),
        links.record_count(),
        optional_hex(s.link_uri(links))
    )
}
#[test]
fn styles_and_failed_registry_side_effects_match_c() {
    let mut input = String::new();
    let mut expected = String::new();
    let mut links = HyperlinkRegistry::new();
    let mut style = Style::from_cell(base());
    let mut corpus: Vec<Vec<u8>> = vec![
        "",
        "default",
        "ignore",
        "noignore",
        "push-default",
        "pop-default",
        "set-default",
        "list=on",
        "list=focus",
        "list=left-marker",
        "list=right-marker",
        "nolist",
        "list=",
        "list=off",
        "range=left",
        "range=right",
        "range=control|9",
        "range=control|10",
        "range=pane|%4294967295",
        "range=session|$4294967295",
        "range=window|+1",
        "range=user|abcdefghijklmnop",
        "range=user|ééééééééé",
        "range=user|a|b",
        "range=bogus",
        "range=bogus|",
        "range=",
        "norange",
        "align=left",
        "align=centre",
        "align=right",
        "align=absolute-centre",
        "align=bogus",
        "noalign",
        "fill=red",
        "dim",
        "dim=50%",
        "dim=100",
        "dim=101",
        "fg=default",
        "bg=terminal",
        "us=default",
        "xg=red",
        "none",
        "noattr",
        "NOattr",
        "noATTR",
        "NOlink",
        "noLINK",
        "nonone",
        "nodefault",
        "width=2147483647",
        "width=2147483648",
        "width=4294967295",
        "width=4294967296",
        "width=100%",
        "width=101%",
        "pad=2147483648",
        "pad=4294967295",
        "link=",
        "link=https://example",
        "link=a\\b",
        "link=new,bad-token",
        "\tbright",
        " ,\nbright,,dim, ",
        "bright\0bad",
        "default,ignore,width=42,fg=red,none,dim",
    ]
    .into_iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    corpus.push([b"link=".as_slice(), &[b'a'; 250]].concat());
    corpus.push([b"link=".as_slice(), &[b'a'; 251]].concat());
    let mut rng = common::Rng::new(0x5a02);
    let tokens: Vec<_> = corpus.clone();
    for _ in 0..3000 {
        let mut bytes = Vec::new();
        for _ in 0..rng.below(6) {
            if !bytes.is_empty() {
                bytes.push(b',');
            }
            bytes.extend_from_slice(&tokens[rng.below(tokens.len() as u64) as usize]);
        }
        corpus.push(bytes);
    }
    for bytes in corpus {
        command(&mut input, "S", &bytes);
        let result = style.parse(&base(), &bytes, &mut links);
        writeln!(
            expected,
            "{} {}",
            if result.is_ok() { 0 } else { -1 },
            state(&style, &links)
        )
        .unwrap();
    }
    for bytes in [b"".as_slice(), b"red", b"default", b"bad", b"terminal"] {
        command(&mut input, "B", bytes);
        let result = style.parse_colour(&base(), bytes);
        writeln!(
            expected,
            "{} {}",
            if result.is_ok() { 0 } else { -1 },
            state(&style, &links)
        )
        .unwrap();
    }
    writeln!(input, "D").unwrap();
    writeln!(expected, "{}", state(&Style::option_fallback(), &links)).unwrap();
    compare(&input, &expected);
}
#[test]
fn hyperlink_global_fifo_leases_and_transfer_match_c() {
    let mut input = String::new();
    let mut expected = String::new();
    let mut links = HyperlinkRegistry::new();
    let mut stores: Vec<Option<Hyperlinks>> = (0..3).map(|_| None).collect();
    for (n, store) in stores.iter_mut().enumerate().take(2) {
        *store = Some(links.create().unwrap());
        writeln!(input, "create {n}").unwrap();
        writeln!(expected, "ok").unwrap();
    }
    stores[2] = Some(links.share(stores[0].as_ref().unwrap()).unwrap());
    input.push_str("share 0 2\n");
    expected.push_str("ok\n");
    let mut corpus = vec![
        (0, b"same".to_vec(), b"id".to_vec()),
        (0, b"same".to_vec(), b"id".to_vec()),
        (0, b"same".to_vec(), vec![]),
        (0, b"same".to_vec(), vec![]),
        (1, b"same".to_vec(), b"id".to_vec()),
        (0, b"\xc3\xa9\xff\n\\".to_vec(), b"\xff\t".to_vec()),
        (0, vec![b'a'; 1024], vec![]),
        (0, vec![b'a'; 1025], vec![]),
        (0, vec![1; 256], vec![]),
        (0, vec![1; 257], vec![]),
    ];
    for n in 0..5100 {
        corpus.push((
            n % 2,
            format!("uri{n}").into_bytes(),
            format!("id{n}").into_bytes(),
        ));
    }
    for (n, uri, id) in corpus {
        writeln!(input, "put {n} {} {}", hex(&uri), hex(&id)).unwrap();
        let store = stores[n].as_ref().unwrap();
        let inner = links.put(store, &uri, Some(&id)).unwrap();
        writeln!(
            expected,
            "{} {} {}",
            inner.0,
            links.record_count(),
            optional_hex(links.get(store, inner).map(HyperlinkUri::external_id))
        )
        .unwrap();
        if inner.0 % 100 == 0 || inner.0 < 12 {
            for query in [HyperlinkId(1), inner] {
                writeln!(input, "get {n} {}", query.0).unwrap();
                if let Some(link) = links.get(store, query) {
                    writeln!(
                        expected,
                        "{} {} {}",
                        hex(link.uri()),
                        hex(link.internal_id()),
                        hex(link.external_id())
                    )
                    .unwrap();
                } else {
                    expected.push_str("-\n");
                }
            }
        }
    }
    let mut style = Style::from_cell(base());
    command(&mut input, "S", b"link=a\\b");
    style.parse(&base(), b"link=a\\b", &mut links).unwrap();
    writeln!(expected, "0 {}", state(&style, &links)).unwrap();
    input.push_str("transfer 1\n");
    let transferred = links
        .copy_style_link_to_store(&style, stores[1].as_ref().unwrap())
        .unwrap();
    writeln!(expected, "{}", transferred.0).unwrap();
    writeln!(input, "get 1 {}", transferred.0).unwrap();
    let link = links.get(stores[1].as_ref().unwrap(), transferred).unwrap();
    writeln!(
        expected,
        "{} {} {}",
        hex(link.uri()),
        hex(link.internal_id()),
        hex(link.external_id())
    )
    .unwrap();
    input.push_str("reset 2\n");
    links.reset(stores[2].as_ref().unwrap()).unwrap();
    writeln!(expected, "{}", links.record_count()).unwrap();
    for n in [0, 2, 1] {
        writeln!(input, "release {n}").unwrap();
        links.release(stores[n].take().unwrap()).unwrap();
        writeln!(expected, "{}", links.record_count()).unwrap();
    }
    compare(&input, &expected);
}

#[test]
fn palette_and_overlay_match_pinned_c() {
    let mut input = String::new();
    let mut expected = String::new();
    let mut palette = ColourPalette::new();
    for (slot, value) in [
        (0, Colour::NONE),
        (1, Colour(2)),
        (1, Colour(2)),
        (255, Colour::rgb(1, 2, 3)),
        (256, Colour(1)),
        (1, Colour::NONE),
    ] {
        writeln!(input, "pset {slot} {}", value.raw()).unwrap();
        writeln!(expected, "{}", i32::from(palette.set(slot, value))).unwrap();
    }
    let mut defaults = [Colour::NONE; 256];
    defaults[1] = Colour(5);
    defaults[8] = Colour(6);
    palette.replace_defaults(Some(defaults));
    input.push_str("pdefault 1 5\npdefault 8 6\n");
    expected.push_str("ok\nok\n");
    for colour in [0, 1, 7, 8, 9, 90, 97, 0x10000ff] {
        writeln!(input, "pget {colour}").unwrap();
        writeln!(
            expected,
            "{}",
            palette.get(Colour(colour)).map_or(-1, Colour::raw)
        )
        .unwrap();
    }
    palette.clear_runtime();
    input.push_str("pclear\npget 1\n");
    expected.push_str("8 8 0 1\n5\n");
    palette.clear_storage();
    input.push_str("pfree\npget 1\n");
    expected.push_str("8 8 0 0\n-1\n");
    let mut links = HyperlinkRegistry::new();
    let mut style = Style::from_cell(base());
    command(
        &mut input,
        "S",
        b"fg=blue,bg=default,us=red,noattr,link=overlay",
    );
    style
        .parse(
            &base(),
            b"fg=blue,bg=default,us=red,noattr,link=overlay",
            &mut links,
        )
        .unwrap();
    writeln!(expected, "0 {}", state(&style, &links)).unwrap();
    for (op, overlay) in [("overlay", style), ("fallback", Style::option_fallback())] {
        let mut cell = base();
        overlay.overlay_cell(&mut cell);
        writeln!(input, "{op}").unwrap();
        writeln!(
            expected,
            "{} {} {} {} {} {} {}",
            cell.fg.raw(),
            cell.bg.raw(),
            cell.us.raw(),
            cell.attr.bits(),
            cell.flags.bits(),
            cell.link.0,
            cell.data.data[0]
        )
        .unwrap();
    }
    compare(&input, &expected);
}
