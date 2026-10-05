// Ported from tmux tty.c and tty-draw.c @ 8f25579c
use crate::test_common as common;
#[path = "../../tests/fixtures/draw_corpus.rs"]
mod corpus;
use super::*;
use rmux_emu::colour::Colour;
use rmux_emu::hyperlinks::HyperlinkId;
use rmux_emu::screen::{ScreenMode, ScreenResetPolicy, ScreenSelection};
use rmux_util::bytes::ByteString;
use std::fmt::Write;
use std::os::fd::AsFd;
use std::path::Path;

const CAPS: &[&str] = &[
    "clear=\x1b[H\x1b[2J",
    "cup=\x1b[%i%p1%d;%p2%dH",
    "csr=\x1b[%i%p1%d;%p2%dr",
    "sgr0=\x1b[0m",
    "civis=\x1b[?25l",
    "cnorm=\x1b[?25h",
    "ich=\x1b[%p1%d@",
    "ich1=\x1b[@",
    "dch=\x1b[%p1%dP",
    "dch1=\x1b[P",
    "il=\x1b[%p1%dL",
    "il1=\x1b[L",
    "dl=\x1b[%p1%dM",
    "dl1=\x1b[M",
    "el=\x1b[K",
    "el1=\x1b[1K",
    "ech=\x1b[%p1%dX",
    "ed=\x1b[J",
    "indn=\x1b[%p1%dS",
    "ri=\x1bM",
    "rin=\x1b[%p1%dT",
    "setab=\x1b[4%p1%dm",
    "setaf=\x1b[3%p1%dm",
    "bold=\x1b[1m",
    "rev=\x1b[7m",
    "smacs=\x0e",
    "rmacs=\x0f",
    "Cmg=\x1b[%i%p1%d;%p2%ds",
    "Clmg=\x1b[s",
    "Sync=\x1b[?2026%?%p1%{1}%=%th%el%;",
    "Ms=\x1b]52;%p1%s;%p2%s\x07",
    "am=1",
    "bce=1",
    "AX=1",
    "colors=8",
];
fn fixture(w: u32, h: u32, state: &mut TparmState) -> Tty {
    let (_master, slave, _) = rmux_sys::pty::openpty().unwrap();
    let tio = rmux_sys::termios::TermiosState::get(slave.as_fd()).unwrap();
    let mut tty = Tty::new(
        slave,
        tio,
        crate::tty::TtyHostInfo {
            utf8: true,
            ..Default::default()
        },
    );
    let caps = CAPS
        .iter()
        .map(|s| ByteString(s.as_bytes().to_vec()))
        .collect();
    tty.term = Some(
        crate::term::TtyTerm::create(state, b"fixture", &caps, &mut tty.host, &tty.opts, None)
            .unwrap(),
    );
    tty.sx = w;
    tty.sy = h;
    tty.cx = u32::MAX;
    tty.cy = u32::MAX;
    tty.rupper = u32::MAX;
    tty.rlower = u32::MAX;
    tty.rleft = u32::MAX;
    tty.rright = u32::MAX;
    tty.mode = ScreenMode::CURSOR;
    tty.term.as_mut().unwrap().apply_overrides(
        state,
        &[ByteString(
            b"*:bpaste@:Enbp@:Dsbp@:Enfcs@:Dsfcs@:tsl@:fsl@:XT@:Cmg@:Clmg@".to_vec(),
        )],
    );
    tty
}
fn number<T: std::str::FromStr>(t: &mut std::str::SplitWhitespace<'_>) -> T {
    t.next().unwrap().parse().ok().unwrap()
}
fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return vec![];
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}
fn cell(t: &mut std::str::SplitWhitespace<'_>) -> GridCell {
    let mut gc = DEFAULT_CELL;
    gc.attr = GridAttributes(number(t));
    gc.flags = GridCellFlags(number(t));
    gc.fg = Colour(number(t));
    gc.bg = Colour(number(t));
    gc.us = Colour(number(t));
    gc.link = HyperlinkId(number(t));
    gc.data.width = number(t);
    let b = unhex(t.next().unwrap());
    gc.data.size = b.len() as u8;
    gc.data.data[..b.len()].copy_from_slice(&b);
    gc
}
fn rust_run(input: &str) -> String {
    let mut state = TparmState::default();
    let mut registry = HyperlinkRegistry::default();
    let mut tty = fixture(40, 10, &mut state);
    let mut s = Screen::new(40, 10, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut out = String::new();
    let mut redraw = None;
    for line in input.lines() {
        let mut t = line.split_whitespace();
        match t.next().unwrap() {
            "new" => {
                let w = number(&mut t);
                let h = number(&mut t);
                s.release(
                    &mut registry,
                    #[cfg(feature = "sixel")]
                    None,
                )
                .unwrap();
                s = Screen::new(w, h, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
                tty = fixture(w, h, &mut state);
            }
            "capoff" => {
                let i: usize = number(&mut t);
                let cap = format!("{}@", crate::term::CODES[i].name);
                tty.term_mut().apply(cap.as_bytes(), false, TtyTermFlags(0));
            }
            "termflags" => {
                let f: u32 = number(&mut t);
                let mut overrides = vec![ByteString(b"*:Cmg@:Clmg@:Rect@:am=1".to_vec())];
                if f & TtyTermFlags::NOAM.bits() != 0 {
                    overrides.push(ByteString(b"*:am@".to_vec()));
                }
                if f & TtyTermFlags::DECSLRM.bits() != 0 {
                    overrides.push(ByteString(b"*:Cmg=\\E[%i%p1%d;%p2%ds:Clmg=\\E[s".to_vec()));
                }
                if f & TtyTermFlags::DECFRA.bits() != 0 {
                    overrides.push(ByteString(b"*:Rect".to_vec()));
                }
                tty.term_mut().apply_overrides(&mut state, &overrides);
            }
            "ttyflags" => tty.flags = TtyFlags(number(&mut t)),
            "cursor" => {
                tty.cx = number(&mut t);
                tty.cy = number(&mut t);
            }
            "lineflag" => {
                let y: u32 = number(&mut t);
                s.grid.get_line_mut(s.grid.hsize() + y).flags =
                    rmux_emu::grid::GridLineFlags(number(&mut t));
            }
            "grid" => {
                let x = number(&mut t);
                let y = number(&mut t);
                let gc = cell(&mut t);
                s.grid.view_set_cell(x, y, &gc);
            }
            "selection" => {
                let gc = cell(&mut t);
                s.selection = Some(ScreenSelection {
                    hidden: false,
                    rectangle: false,
                    modekeys: 0,
                    sx: 0,
                    sy: 0,
                    ex: s.grid.sx() - 1,
                    ey: s.grid.sy() - 1,
                    clipx: 0,
                    cell: gc,
                });
            }
            "line" => {
                let x = number(&mut t);
                let y = number(&mut t);
                let n = number(&mut t);
                let ax = number(&mut t);
                let ay = number(&mut t);
                tty.draw_line(&mut state, &registry, &s, x, y, n, ax, ay, None);
            }
            "syncend" => tty.sync_end(&mut state),
            "cmd" => {
                let name = t.next().unwrap();
                let ocx = number(&mut t);
                let ocy = number(&mut t);
                let orupper = number(&mut t);
                let orlower = number(&mut t);
                let xoff = number(&mut t);
                let yoff = number(&mut t);
                let rxoff = number(&mut t);
                let ryoff = number(&mut t);
                let sx = number(&mut t);
                let sy = number(&mut t);
                let wox = number(&mut t);
                let woy = number(&mut t);
                let wsx = number(&mut t);
                let wsy = number(&mut t);
                let bg = number(&mut t);
                let flags = TtyCtxFlags(number(&mut t));
                let n = number(&mut t);
                let gc = if name == "cell" || name == "cells" {
                    cell(&mut t)
                } else {
                    DEFAULT_CELL
                };
                let bytes = if matches!(name, "cells" | "rawstring" | "setselection") {
                    unhex(t.next().unwrap())
                } else {
                    vec![]
                };
                let data = if name == "setselection" {
                    TtyCommandData::Selection {
                        clip: "c",
                        data: &bytes,
                    }
                } else if matches!(name, "cells" | "rawstring") {
                    TtyCommandData::Bytes(&bytes)
                } else {
                    TtyCommandData::Count(n)
                };
                let cmd = match name {
                    "insertcharacter" => TtyCommand::InsertCharacter,
                    "deletecharacter" => TtyCommand::DeleteCharacter,
                    "clearcharacter" => TtyCommand::ClearCharacter,
                    "insertline" => TtyCommand::InsertLine,
                    "deleteline" => TtyCommand::DeleteLine,
                    "clearline" => TtyCommand::ClearLine,
                    "clearendofline" => TtyCommand::ClearEndOfLine,
                    "clearstartofline" => TtyCommand::ClearStartOfLine,
                    "reverseindex" => TtyCommand::ReverseIndex,
                    "linefeed" => TtyCommand::LineFeed,
                    "scrollup" => TtyCommand::ScrollUp,
                    "scrolldown" => TtyCommand::ScrollDown,
                    "clearendofscreen" => TtyCommand::ClearEndOfScreen,
                    "clearstartofscreen" => TtyCommand::ClearStartOfScreen,
                    "clearscreen" => TtyCommand::ClearScreen,
                    "alignmenttest" => TtyCommand::AlignmentTest,
                    "cell" => TtyCommand::Cell,
                    "cells" => TtyCommand::Cells,
                    "redrawline" => TtyCommand::RedrawLine,
                    "setselection" => TtyCommand::SetSelection,
                    "rawstring" => TtyCommand::RawString,
                    "syncstart" => TtyCommand::SyncStart,
                    _ => panic!("unknown command"),
                };
                let ctx = TtyCtx {
                    s: &s,
                    cell: &gc,
                    flags,
                    data,
                    ocx,
                    ocy,
                    orupper,
                    orlower,
                    xoff,
                    yoff,
                    rxoff,
                    ryoff,
                    sx,
                    sy,
                    bg,
                    defaults: DEFAULT_CELL,
                    style_ctx: TtyStyleCtx::default(),
                    wox,
                    woy,
                    wsx,
                    wsy,
                };
                if let Some(r) = tty.command(&mut state, cmd, &ctx) {
                    redraw = Some(r);
                }
            }
            "dump" => {
                let r = redraw.take().unwrap_or(TtyRedraw {
                    start_y: 0,
                    count: 0,
                });
                write!(
                    out,
                    "{} {} {} {} {} {} {} {} {} ",
                    tty.cx,
                    tty.cy,
                    tty.rupper,
                    tty.rlower,
                    tty.rleft,
                    tty.rright,
                    tty.flags.bits(),
                    r.start_y,
                    r.count
                )
                .unwrap();
                for b in tty.out.drain(..) {
                    write!(out, "{b:02x}").unwrap();
                }
                out.push('\n');
            }
            _ => panic!("unknown corpus operation"),
        }
    }
    out
}
fn reference() -> Option<std::path::PathBuf> {
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/draw_reference.c");
    let mut flags = vec![
        "-ffunction-sections".to_owned(),
        "-DHAVE_CURSES_H".to_owned(),
        "-DHAVE_TIPARM_S".to_owned(),
    ];
    if cfg!(target_os = "macos") {
        let include = Path::new("/opt/homebrew/opt/ncurses/include");
        let header = include.join("term.h");
        if !std::fs::read_to_string(&header).is_ok_and(|s| s.contains("tiparm_s")) {
            eprintln!(
                "C tty reference skipped: Homebrew ncurses with tiparm_s required at {}",
                header.display()
            );
            return None;
        }
        flags.extend(
            [
                "-Wl,-dead_strip",
                "-L/opt/homebrew/opt/libevent/lib",
                "-I/opt/homebrew/opt/ncurses/include",
                "-L/opt/homebrew/opt/ncurses/lib",
                "-levent",
                "-lncurses",
                "-lresolv",
            ]
            .map(str::to_owned),
        );
    } else {
        let pkg = std::process::Command::new("pkg-config")
            .args(["--cflags", "--libs", "ncurses"])
            .output()
            .ok()
            .filter(|o| o.status.success());
        let dir = std::process::Command::new("pkg-config")
            .args(["--variable=includedir", "ncurses"])
            .output()
            .ok()
            .filter(|o| o.status.success());
        let (Some(pkg), Some(dir)) = (pkg, dir) else {
            eprintln!("C tty reference skipped: pkg-config ncurses required");
            return None;
        };
        let include = std::path::PathBuf::from(String::from_utf8_lossy(&dir.stdout).trim());
        if ![include.join("term.h"), include.join("ncurses/term.h")]
            .iter()
            .any(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains("tiparm_s")))
        {
            eprintln!("C tty reference skipped: detected ncurses lacks oracle tiparm_s API");
            return None;
        }
        flags.extend(
            [
                "-Wl,--gc-sections",
                "-Wl,--no-as-needed",
                "-levent",
                "-lresolv",
            ]
            .map(str::to_owned),
        );
        flags.extend(
            String::from_utf8_lossy(&pkg.stdout)
                .split_whitespace()
                .map(str::to_owned),
        );
    }
    let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
    common::build_c(
        "tty-draw",
        &[
            &driver,
            Path::new("utf8.c"),
            Path::new("utf8-combined.c"),
            Path::new("xmalloc.c"),
            Path::new("compat/base64.c"),
            Path::new("compat/utf8proc.c"),
            Path::new("compat/vis.c"),
            Path::new("compat/strtonum.c"),
            Path::new("compat/reallocarray.c"),
            Path::new("compat/recallocarray.c"),
            Path::new("compat/explicit_bzero.c"),
        ],
        &flags,
        cfg!(target_os = "macos"),
    )
}
#[test]
fn commands_and_line_machine_match_pinned_c_bytes() {
    let Some(bin) = reference() else {
        return;
    };
    let input = corpus::corpus();
    let expected = common::run(&bin, &[], input.as_bytes());
    assert_eq!(rust_run(&input).as_bytes(), expected);
}
#[test]
fn clamp_preserves_signed_and_status_adjusted_offsets() {
    let mut state = TparmState::default();
    let mut registry = HyperlinkRegistry::default();
    let s = Screen::new(80, 24, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let _tty = fixture(40, 10, &mut state);
    let mut ctx = TtyCtx {
        s: &s,
        cell: &DEFAULT_CELL,
        flags: TtyCtxFlags::WINDOW_BIGGER,
        data: TtyCommandData::Count(0),
        ocx: 0,
        ocy: 0,
        orupper: 0,
        orlower: 23,
        xoff: 0,
        yoff: 2,
        rxoff: 0,
        ryoff: 0,
        sx: 80,
        sy: 24,
        bg: 8,
        defaults: DEFAULT_CELL,
        style_ctx: TtyStyleCtx::default(),
        wox: 10,
        woy: 3,
        wsx: 40,
        wsy: 10,
    };
    assert_eq!(
        clamp_line(&ctx, 0, 3, 80),
        Some(ClampedLine {
            skip: 10,
            x: 0,
            width: 40,
            y: 2
        })
    );
    ctx.xoff = -4;
    ctx.rxoff = -4;
    ctx.flags = TtyCtxFlags(0);
    ctx.wox = 0;
    assert_eq!(
        clamp_line(&ctx, 0, 3, 8),
        Some(ClampedLine {
            skip: 4,
            x: 0,
            width: 4,
            y: 2
        })
    );
}
#[test]
#[should_panic(expected = "clamp result")]
fn clamp_invariant_failure_is_not_skipped() {
    clamp_axis(0, 100, 5, 2, 40, true);
}

#[test]
fn every_command_fast_and_fallback_runs_without_reference() {
    let output = rust_run(&corpus::corpus());
    assert!(output.lines().count() > 90);
    assert!(output.contains("1 8 "));
    assert!(output.contains("616263"));
    assert!(output.contains("610062"));
}

#[test]
fn small_regions_draw_immediately_large_or_obscured_return_damage() {
    let mut state = TparmState::default();
    let mut registry = HyperlinkRegistry::default();
    let screen = Screen::new(40, 10, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut tty = fixture(40, 10, &mut state);
    let mut ctx = TtyCtx {
        s: &screen,
        cell: &DEFAULT_CELL,
        flags: TtyCtxFlags::WINDOW_BIGGER,
        data: TtyCommandData::Count(1),
        ocx: 0,
        ocy: 1,
        orupper: 1,
        orlower: 2,
        xoff: 0,
        yoff: 0,
        rxoff: 0,
        ryoff: 0,
        sx: 40,
        sy: 10,
        bg: 8,
        defaults: DEFAULT_CELL,
        style_ctx: TtyStyleCtx::default(),
        wox: 0,
        woy: 0,
        wsx: 40,
        wsy: 10,
    };
    assert_eq!(tty.command(&mut state, TtyCommand::InsertLine, &ctx), None);
    assert!(!tty.out.is_empty());
    ctx.orlower = 6;
    assert_eq!(
        tty.command(&mut state, TtyCommand::DeleteLine, &ctx),
        Some(TtyRedraw {
            start_y: 1,
            count: 6
        })
    );
    ctx.orlower = 2;
    ctx.flags.insert(TtyCtxFlags::PANE_OBSCURED);
    assert_eq!(
        tty.command(&mut state, TtyCommand::ScrollUp, &ctx),
        Some(TtyRedraw {
            start_y: 1,
            count: 2
        })
    );
}

#[test]
fn sync_flags_survive_missing_capabilities_and_blocked_end() {
    let mut state = TparmState::default();
    let mut tty = fixture(40, 10, &mut state);
    tty.term_mut().apply(b"Sync@", false, TtyTermFlags(0));
    tty.sync_start(&mut state);
    tty.sync_start(&mut state);
    assert!(tty.flags.contains(TtyFlags::SYNCING));
    assert!(tty.out.is_empty());
    tty.flags.insert(TtyFlags::BLOCK);
    tty.sync_end(&mut state);
    assert!(tty.flags.contains(TtyFlags::SYNCING));
    tty.flags.remove(TtyFlags::BLOCK);
    tty.sync_end(&mut state);
    assert!(!tty.flags.contains(TtyFlags::SYNCING));
}

#[test]
fn padding_cell_and_nonprintable_single_byte_are_not_output() {
    let mut state = TparmState::default();
    let mut tty = fixture(40, 10, &mut state);
    let mut gc = DEFAULT_CELL;
    gc.flags = GridCellFlags::PADDING;
    tty.cell(&mut state, &gc, None);
    assert!(tty.out.is_empty());
    gc.flags = GridCellFlags(0);
    gc.data = rmux_util::utf8::Utf8Data::set(0x1b);
    tty.cell(&mut state, &gc, None);
    assert!(tty.out.is_empty());
}

#[test]
fn margin_pane_clamps_to_viewport_including_right_endpoint() {
    let mut state = TparmState::default();
    let mut tty = fixture(40, 10, &mut state);
    tty.term_mut().apply_overrides(
        &mut state,
        &[ByteString(b"*:Cmg=\\E[%i%p1%d;%p2%ds:Clmg=\\E[s".to_vec())],
    );
    let mut registry = HyperlinkRegistry::default();
    let screen = Screen::new(80, 10, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let ctx = TtyCtx {
        s: &screen,
        cell: &DEFAULT_CELL,
        flags: TtyCtxFlags(0),
        data: TtyCommandData::Count(1),
        ocx: 0,
        ocy: 0,
        orupper: 0,
        orlower: 9,
        xoff: -4,
        yoff: 0,
        rxoff: -4,
        ryoff: 0,
        sx: 80,
        sy: 10,
        bg: 8,
        defaults: DEFAULT_CELL,
        style_ctx: TtyStyleCtx::default(),
        wox: 0,
        woy: 0,
        wsx: 40,
        wsy: 10,
    };
    tty.region(&mut state, 0, 9);
    tty.margin_pane(&mut state, &ctx);
    assert_eq!((tty.rleft, tty.rright), (0, 40));
}

#[test]
fn draw_line_default_style_resolves_screen_hyperlinks() {
    let mut state = TparmState::default();
    let mut tty = fixture(20, 6, &mut state);
    tty.term_mut()
        .apply(b"Hls=\\E]8;%p1%s;%p2%s\\E\\\\", false, TtyTermFlags(0));
    let mut registry = HyperlinkRegistry::default();
    let mut screen = Screen::new(20, 6, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut gc = DEFAULT_CELL;
    gc.data = rmux_util::utf8::Utf8Data::set(b'x');
    gc.link = registry
        .put(
            screen.hyperlinks.as_ref().unwrap(),
            b"https://example.test",
            None,
        )
        .unwrap();
    screen.grid.view_set_cell(0, 0, &gc);
    tty.draw_line(&mut state, &registry, &screen, 0, 0, 1, 0, 0, None);
    let output: Vec<u8> = tty.out.iter().copied().collect();
    assert!(
        output
            .windows(b"https://example.test".len())
            .any(|w| w == b"https://example.test")
    );
}

#[cfg(feature = "sixel")]
#[path = "image_tests.rs"]
mod image_tests;
