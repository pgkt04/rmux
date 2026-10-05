// Ported from tmux format-draw.c @ 8f25579c
use super::*;
use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(out, "{b:02x}").unwrap();
    }
    out
}

#[test]
fn pinned_c_draw_and_scanners() {
    let driver = std::env::var_os("RMUX_FORMAT_HELPER_DRIVER")
        .unwrap_or_else(|| "/tmp/swarm-rmux-build/P5FmtAux/driver".into());
    if !std::path::Path::new(&driver).is_file() {
        eprintln!("SKIP: pinned G10 C driver missing; set RMUX_FORMAT_HELPER_DRIVER");
        return;
    }
    let mut input = String::new();
    let mut expected = Vec::new();
    for bytes in [b"\xe2ab".as_slice(), b"\xe2\x82ab", b"a\x01b\x7fc", b"a\0b"] {
        let h = hex(bytes);
        writeln!(input, "width {h}").unwrap();
        expected.push(width(bytes).to_string());
        for limit in 0..=5 {
            writeln!(input, "trimleft {h} {limit}\ntrimright {h} {limit}").unwrap();
            expected.push(hex(&trim_left(bytes, limit)));
            expected.push(hex(&trim_right(bytes, limit)));
        }
    }
    let strings: &[&str] = &[
        "",
        "abc",
        "a##b###c####d",
        "##[x]",
        "###[fg=red]x",
        "a#[fg=red",
        "a#[bogus]b",
        "a#[ignore]b#[fg=red]c##d",
        "a😀b\t́",
        "L#[align=centre]CC#[align=right]RR",
        "left#[align=absolute-centre]AB",
        "#[fg=red]a#[push-default]#[default]b#[pop-default]#[default]c",
        "#[fg=red]#[set-default]a#[pop-default]#[default]b",
        "#[fill=blue]ab",
        "#[link=http://x/]a#[nolink]b#[link=http://x/]c",
        "#[range=window|3]abc#[range=user|xx]def#[norange]g",
        "#[list=on]abcd#[list=focus]ef#[list=on]gh#[list=off]A",
        "#[list=on]abcdefgh#[list=left-marker]<#[list=right-marker]>#[list=on]#[list=focus]ij#[list=off]",
        "L#[align=centre,list=on]abc#[list=off]A#[align=right]R",
        "L#[align=right,list=on]abc#[list=off]A",
        "L#[align=absolute-centre,list=on]abc#[list=off]A#[align=right]R",
        "#[align=left,list=on]abcdef#[list=off]",
        "#[align=left,list=on]abcd#[list=focus]ef#[list=on]gh#[list=off]",
        "#[align=left,list=on]ab#[list=focus]cd#[list=on]efgh#[list=left-marker]<#[list=right-marker]>#[list=off]",
        "#[align=left,list=on]abcdefgh#[list=left-marker]<#[list=right-marker]>#[list=on]#[list=focus]ij#[list=off]",
        "#[range=session|1]abcdef#[norange]#[align=right]#[range=pane|2]xyz#[norange]",
        "#[align=absolute-centre]AB#[align=left]left",
        "abc#[fg=red",
    ];
    for text in strings {
        let h = hex(text.as_bytes());
        writeln!(input, "width {h}").unwrap();
        expected.push(width(text.as_bytes()).to_string());
        for available in 0..=12 {
            writeln!(input, "trimleft {h} {available}\ntrimright {h} {available}").unwrap();
            expected.push(hex(&trim_left(text.as_bytes(), available)));
            expected.push(hex(&trim_right(text.as_bytes(), available)));
            for default_colours in [false, true] {
                let d = tests::draw_into(text, available, 16, 2, default_colours);
                writeln!(
                    input,
                    "draw {h} {available} 16 2 {} 1 8 8",
                    u8::from(default_colours)
                )
                .unwrap();
                let mut line = String::new();
                for (i, cell) in d.cells.iter().enumerate() {
                    if i != 0 {
                        line.push(' ');
                    }
                    write!(
                        line,
                        "{}:{}:{}:{}:{:x}:{:x}:{}:{}",
                        hex(cell.data.bytes()),
                        cell.fg.0,
                        cell.bg.0,
                        cell.us.0,
                        cell.attr.bits(),
                        cell.flags.bits(),
                        cell.data.width,
                        cell.link.0
                    )
                    .unwrap();
                    if cell.link != HyperlinkId::NONE {
                        // The URI is also the internal id in format_draw.
                        let uri = &d.uris[d.cells[..i]
                            .iter()
                            .filter(|c| c.link != HyperlinkId::NONE)
                            .count()];
                        write!(line, "={}={}", hex(uri), hex(uri)).unwrap();
                    }
                }
                write!(line, "|{},{}|", d.cursor.0, d.cursor.1).unwrap();
                for r in &d.ranges.0 {
                    write!(
                        line,
                        "{}:{}:{}:{}-{};",
                        r.range_type as u32,
                        r.argument,
                        hex(cstr(&r.string)),
                        r.start,
                        r.end
                    )
                    .unwrap();
                }
                expected.push(line);
            }
        }
    }
    let mut child = Command::new(driver)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()).unwrap());
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(output.status.success());
    let actual = String::from_utf8(output.stdout).unwrap();
    let actual: Vec<_> = actual.lines().collect();
    assert_eq!(actual.len(), expected.len());
    for (index, (a, e)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(a, e, "pinned drawing/scanner case {index}");
    }
}
