// Ported from tmux tty.c and tty-draw.c @ 8f25579c
use std::fmt::Write;

pub fn corpus() -> String {
    let mut out = String::new();
    for (w, h) in [(40, 10), (80, 24)] {
        for fallback in [false, true] {
            writeln!(out, "new {w} {h}").unwrap();
            if fallback {
                out.push_str("capoff 16\ncapoff 52\ncapoff 53\ncapoff 27\ncapoff 28\n");
            }
            for name in [
                "insertcharacter",
                "deletecharacter",
                "clearcharacter",
                "insertline",
                "deleteline",
                "clearline",
                "clearendofline",
                "clearstartofline",
                "reverseindex",
                "linefeed",
                "scrollup",
                "scrolldown",
                "clearendofscreen",
                "clearstartofscreen",
                "clearscreen",
                "alignmenttest",
                "cell",
                "cells",
                "redrawline",
                "setselection",
                "rawstring",
                "syncstart",
            ] {
                let y = if name == "reverseindex" { 1 } else { h - 2 };
                write!(
                    out,
                    "cmd {name} 2 {y} 1 {} 0 0 0 0 {w} {h} 0 0 {w} {h} 8 8 3",
                    h - 2
                )
                .unwrap();
                if name == "cell" || name == "cells" {
                    out.push_str(" 0 0 8 8 8 0 1 61");
                }
                if name == "cells" {
                    out.push_str(" 616263");
                }
                if name == "rawstring" {
                    out.push_str(" 610062");
                }
                if name == "setselection" {
                    out.push_str(" 0061ff");
                }
                out.push_str("\ndump\n");
            }
            out.push_str("syncend\ndump\n");
        }
    }
    for setup in [
        "",
        "capoff 3\n",
        "capoff 40\ncapoff 41\ncapoff 38\ncapoff 39\n",
        "termflags 8\n",
        "capoff 204\n",
    ] {
        out.push_str("new 40 10\n");
        out.push_str(setup);
        for (name, x, width, bg) in [
            ("clearcharacter", 3, 40, 1),
            ("clearline", 0, 40, 1),
            ("clearendofline", 10, 40, 8),
            ("clearstartofline", 10, 40, 8),
            ("clearscreen", 0, 40, 1),
            ("clearendofscreen", 3, 40, 8),
            ("clearstartofscreen", 3, 40, 8),
            ("insertcharacter", 3, 20, 8),
            ("deletecharacter", 3, 20, 8),
            ("insertline", 3, 20, 8),
            ("deleteline", 3, 20, 8),
        ] {
            writeln!(
                out,
                "cmd {name} {x} 3 1 8 0 0 0 0 {width} 10 0 0 40 10 {bg} 0 3\ndump"
            )
            .unwrap();
        }
    }
    for flags in [0, 4] {
        out.push_str("new 40 10\n");
        writeln!(out, "termflags {flags}").unwrap();
        for name in ["reverseindex", "linefeed", "scrollup", "scrolldown"] {
            let y = if name == "reverseindex" { 2 } else { 5 };
            writeln!(
                out,
                "cmd {name} 3 {y} 2 5 4 0 4 0 20 10 0 0 40 10 1 0 2\ndump"
            )
            .unwrap();
        }
    }
    out.push_str("new 40 10\nttyflags 16\ncmd setselection 0 0 0 9 0 0 0 0 40 10 0 0 40 10 8 0 0 0061ff\ndump\n");
    out.push_str("ttyflags 144\ncmd setselection 0 0 0 9 0 0 0 0 40 10 0 0 40 10 8 0 0 -\ndump\n");
    for (w, h) in [(1, 10), (40, 1)] {
        writeln!(out, "new {w} {h}").unwrap();
        for name in [
            "insertline",
            "deleteline",
            "scrollup",
            "scrolldown",
            "reverseindex",
            "linefeed",
        ] {
            writeln!(
                out,
                "cmd {name} 0 0 0 0 0 0 0 0 {w} {h} 0 0 {w} {h} 8 0 1\ndump"
            )
            .unwrap();
        }
    }
    out.push_str("new 40 10\ncmd alignmenttest 0 0 0 9 0 0 0 0 80 24 0 0 40 10 8 4 0\ndump\n");
    out.push_str("cmd cell 60 0 0 9 0 0 0 0 80 24 0 0 40 10 8 4 0 0 0 8 8 8 0 1 78\ndump\n");
    out.push_str("new 20 6\n");
    for (x, fields) in [
        (0, "1 0 1 4 8 0 1 52"),
        (1, "1 0 1 4 8 0 1 45"),
        (2, "1 0 1 4 8 0 1 44"),
        (3, "0 128 8 8 8 0 5 09"),
        (8, "0 0 8 8 8 0 1 54"),
        (9, "0 0 8 8 8 0 1 41"),
        (10, "0 0 8 8 8 0 1 49"),
        (11, "0 0 8 8 8 0 1 4c"),
        (12, "0 0 8 8 8 0 2 e7958c"),
        (13, "0 4 8 8 8 0 0 -"),
        (14, "0 0 8 8 8 0 1 65cc81"),
        (15, "0 0 8 8 8 0 0 cc81"),
    ] {
        writeln!(out, "grid {x} 0 {fields}").unwrap();
    }
    for (px, nx, atx) in [
        (0, 20, 0),
        (3, 4, 0),
        (4, 4, 0),
        (12, 1, 0),
        (13, 4, 0),
        (12, 4, 18),
        (0, 0, 0),
        (0, 20, 20),
    ] {
        writeln!(out, "line {px} 0 {nx} {atx} 0\ndump").unwrap();
    }
    out.push_str("grid 0 1 0 0 8 8 8 0 1 5a\nlineflag 0 1\ncursor 20 0\nline 0 1 20 0 1\ndump\n");
    out.push_str(
        "selection 16 0 7 1 8 0 1 20\ngrid 2 2 0 16 8 8 8 0 1 20\nline 0 2 20 0 2\ndump\n",
    );
    out.push_str("new 1100 6\n");
    for x in 0..1100 {
        writeln!(out, "grid {x} 0 0 0 8 8 8 0 1 61").unwrap();
    }
    out.push_str("line 0 0 1100 0 0\ndump\n");
    out.push_str("new 40 10\ntermflags 2\n");
    out.push_str("grid 39 9 0 0 8 8 8 0 1 58\nline 0 9 40 0 9\ndump\n");
    out.push_str(
        "cmd cells 0 1 0 9 0 0 0 0 80 10 2 0 40 10 8 4 0 0 0 8 8 8 0 1 61 6162636465\ndump\n",
    );
    out
}
