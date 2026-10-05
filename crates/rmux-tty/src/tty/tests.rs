// Ported from tmux tty.c @ 8f25579c
use super::*;
use crate::term::TtyCodeCode as C;
use rmux_emu::cell::{GridAttributes as A, GridCellFlags};
use rmux_emu::colour::{Colour, ColourFlags, ColourPalette, indexed_to_16};
use rmux_emu::hyperlinks::{HyperlinkId, HyperlinkRegistry};
use rmux_emu::screen::{ProgressBar, ProgressBarState, Screen, ScreenResetPolicy};

const CAPS: &[&str] = &[
    "am=1",
    "clear=C",
    "cup=<p%p1%d,%p2%d>",
    "csr=<r%p1%d,%p2%d>",
    "smcup=A",
    "rmcup=a",
    "smkx=K",
    "rmkx=k",
    "cnorm=N",
    "civis=I",
    "cvvis=V",
    "sgr0=Z",
    "home=H",
    "cub1=L",
    "cuf1=R",
    "cuu1=U",
    "cud1=D",
    "cub=<l%p1%d>",
    "cuf=<r%p1%d>",
    "cuu=<u%p1%d>",
    "cud=<d%p1%d>",
    "hpa=<x%p1%d>",
    "vpa=<y%p1%d>",
    "setaf=<f%p1%d>",
    "setab=<b%p1%d>",
    "colors=8",
    "AX=1",
    "bold=B",
    "dim=d",
    "sitm=i",
    "smso=o",
    "smul=u",
    "Smulx=<u%p1%d>",
    "blink=b",
    "rev=v",
    "invis=h",
    "smxx=x",
    "Smol=t",
    "smacs=s",
    "rmacs=e",
    "Ss=<s%p1%d>",
    "Se=E",
    "Cr=c",
    "Cs=<c%p1%s>",
    "Ms=<m%p1%s,%p2%s>",
    "Hls=<h%p1%s,%p2%s>",
    "Setulc1=<a%p1%d>",
    "ol=O",
    "indn=<i%p1%d>",
];
fn caps() -> CapList {
    CAPS.iter().map(|s| ByteString::from(*s)).collect()
}
fn pty_tty() -> (Tty, OwnedFd) {
    let (master, slave, _) = rmux_sys::pty::openpty().unwrap();
    rmux_sys::pty::set_winsize(
        slave.as_fd(),
        rmux_sys::pty::Winsize {
            rows: 24,
            cols: 80,
            xpixel: 640,
            ypixel: 384,
        },
    )
    .unwrap();
    let tio = TermiosState::get(slave.as_fd()).unwrap();
    let mut tty = Tty::new(slave, tio, TtyHostInfo::default());
    tty.set_size(80, 24, 8, 16);
    (tty, master)
}
fn fixture() -> (Tty, OwnedFd, TparmState) {
    let (mut tty, master) = pty_tty();
    let mut state = TparmState::default();
    tty.term = Some(
        TtyTerm::create(
            &mut state,
            b"fixture",
            &caps(),
            &mut tty.host,
            &tty.opts,
            None,
        )
        .unwrap(),
    );
    tty.flags.insert(TtyFlags::OPENED);
    tty.rupper = 0;
    tty.rlower = 23;
    tty.rleft = 0;
    tty.rright = 79;
    (tty, master, state)
}
fn bytes(tty: &mut Tty) -> Vec<u8> {
    tty.out.drain(..).collect()
}
fn remove(tty: &mut Tty, cap: &[u8]) {
    tty.term_mut().apply(cap, false, TtyTermFlags(0));
}

#[test]
fn start_stop_bytes_and_raw_termios() {
    for clear in [true, false] {
        let (mut tty, master) = pty_tty();
        let original = tty.tio;
        let opts = TtyOptions {
            clear_on_attach: clear,
            ..TtyOptions::default()
        };
        let mut state = TparmState::default();
        tty.open(&mut state, b"fixture", &caps(), &opts, None)
            .unwrap();
        let mut expected = if clear {
            b"AC".to_vec()
        } else {
            b"<r0,23><p0,23><i25>".to_vec()
        };
        expected.extend_from_slice(b"KNZN<p0,0><r0,23>");
        assert_eq!(bytes(&mut tty), expected);
        assert!(tty.wants_read());
        let current = TermiosState::get(tty.fd()).unwrap();
        let mut raw = original;
        raw.make_tty_raw();
        assert_eq!(current.iflag(), raw.iflag());
        assert_eq!(current.oflag(), raw.oflag());
        assert_eq!(current.lflag(), raw.lflag());
        assert_eq!(current.cflag(), original.cflag());
        assert_eq!(current.cc(), raw.cc());
        tty.add(b"pending");
        tty.cstyle = ScreenCursorStyle::Bar;
        tty.ccolour = 0;
        tty.stop(&mut state, &opts);
        let mut recorded = [0; 4096];
        let n = rmux_sys::fd::read(master.as_fd(), &mut recorded).unwrap();
        let expected = if clear {
            &b"<r0,23>eZkCEcNa"[..]
        } else {
            &b"<r0,23>eZkEcNC"[..]
        };
        assert_eq!(&recorded[..n], expected);
        assert_eq!(bytes(&mut tty), b"pending");
        let (_baseline_master, baseline_slave, _) = rmux_sys::pty::openpty().unwrap();
        raw.set(baseline_slave.as_fd()).unwrap();
        original.set(baseline_slave.as_fd()).unwrap();
        let restoration_baseline = TermiosState::get(baseline_slave.as_fd()).unwrap();
        let restored = TermiosState::get(tty.fd()).unwrap();
        assert_eq!(restored.iflag(), original.iflag());
        // Darwin sets PENDIN when TCSANOW re-enables ICANON (xnu bsd/kern/tty.c).
        assert_eq!(restored.lflag(), restoration_baseline.lflag());
        assert!(!tty.wants_read());
        assert!(!tty.wants_write());
        let timers: Vec<_> = tty.pending_timers().collect();
        assert!(timers.contains(&TimerRequest {
            timer: TtyTimer::Start,
            after: Some(Duration::from_secs(5))
        }));
        for timer in [TtyTimer::Start, TtyTimer::Clipboard, TtyTimer::Block] {
            assert!(timers.contains(&TimerRequest { timer, after: None }));
        }
    }
}

#[test]
fn start_without_attach_clear_scroll_fallbacks() {
    for (remove_cap, expected) in [
        (b"indn@".as_slice(), b"<r0,23><p0,23>".as_slice()),
        (b"indn@:ind@".as_slice(), b"<r0,23><p0,23>C".as_slice()),
    ] {
        let (mut tty, _master, mut state) = fixture();
        tty.term_mut().apply(b"ind=J", false, TtyTermFlags(0));
        remove(&mut tty, remove_cap);
        tty.start(
            &mut state,
            &TtyOptions {
                clear_on_attach: false,
                ..TtyOptions::default()
            },
        );
        let output = bytes(&mut tty);
        assert!(output.starts_with(expected));
        if remove_cap == b"indn@" {
            assert_eq!(&output[expected.len()..expected.len() + 25], &[b'J'; 25]);
        }
    }
}

#[test]
fn failed_stop_ioctl_pipe_output_and_error_read() {
    let (mut tty, _master, mut state) = fixture();
    let (read, write) = rmux_sys::fd::pipe().unwrap();
    tty.fd = write;
    tty.flags.insert(TtyFlags::STARTED);
    tty.read_pending = true;
    tty.add(b"queued");
    tty.stop(&mut state, &TtyOptions::default());
    assert_eq!(tty.out_len(), 6);
    assert!(!tty.flags.contains(TtyFlags::STARTED));
    rmux_sys::fd::set_blocking(read.as_fd(), false);
    assert_eq!(
        rmux_sys::fd::read(read.as_fd(), &mut [0; 8])
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(tty.on_writable().unwrap(), 6);
    let mut got = [0; 8];
    assert_eq!(rmux_sys::fd::read(read.as_fd(), &mut got).unwrap(), 6);
    assert_eq!(&got[..6], b"queued");
    assert_eq!(tty.on_readable(), ReadOutcome::Closed);
    assert!(tty.drain_effects().any(|e| e == TtyEffect::ReadClosed));
    tty.close(&mut state);
    assert!(tty.term.is_none());
    assert_eq!(tty.out_len(), 0);
    assert!(
        tty.pending_timers()
            .any(|t| t.timer == TtyTimer::Key && t.after.is_none())
    );
}

#[test]
fn open_builds_key_tree_and_close_frees_it() {
    let (mut tty, _master) = pty_tty();
    let mut state = TparmState::default();
    assert!(tty.keys_mut().nodes().is_empty());
    tty.open(
        &mut state,
        b"fixture",
        &caps(),
        &TtyOptions::default(),
        None,
    )
    .unwrap();
    let nodes = tty.keys_mut().nodes().len();
    assert!(nodes > 0);
    let (found, size) = tty.keys_mut().find(b"\x1b[1;5A");
    assert!(found.is_some());
    assert_eq!(size, 6);
    tty.close(&mut state);
    assert!(tty.keys_mut().nodes().is_empty());
    assert!(tty.term.is_none());
}

#[test]
fn read_buffer_consumption_and_write_error_not_rearmed() {
    let (mut tty, _master, _state) = fixture();
    let (read, write) = rmux_sys::fd::pipe().unwrap();
    tty.fd = read;
    rmux_sys::fd::write(write.as_fd(), b"abc").unwrap();
    assert_eq!(tty.on_readable(), ReadOutcome::Bytes(3));
    assert_eq!(tty.input_bytes(), b"abc");
    tty.consume_input(1);
    assert_eq!(tty.input_bytes(), b"bc");
    tty.write_pending = true;
    tty.add(b"out");
    assert!(tty.on_writable().is_err());
    assert!(!tty.wants_write());
    assert_eq!(tty.out_len(), 3);
    drop(write);
    assert_eq!(tty.on_readable(), ReadOutcome::Closed);
}

#[test]
fn exact_flow_thresholds_noblock_redraw_and_timer() {
    let (mut tty, _master, mut state) = fixture();
    tty.set_size(4, 2, 0, 0);
    tty.add(&[b'x'; 64]);
    assert!(!tty.block_maybe());
    tty.add(b"x");
    assert!(tty.block_maybe());
    assert_eq!(tty.out_len(), 0);
    assert!(tty.flags.contains(TtyFlags::BLOCK));
    assert!(tty.drain_effects().any(|e| e == TtyEffect::Discarded(65)));
    tty.add(b"xx");
    assert_eq!(tty.discarded, 2);
    tty.on_timer(&mut state, TtyTimer::Block);
    assert!(tty.flags.contains(TtyFlags::BLOCK));
    assert_eq!(tty.discarded, 0);
    assert!(tty.pending_timers().any(|t| t
        == TimerRequest {
            timer: TtyTimer::Block,
            after: Some(Duration::from_millis(100))
        }));
    tty.on_timer(&mut state, TtyTimer::Block);
    assert!(!tty.flags.contains(TtyFlags::BLOCK));
    tty.flags.insert(TtyFlags::NOBLOCK);
    tty.add(&[b'x'; 65]);
    assert!(!tty.block_maybe());
    tty.out.clear();
    assert!(!tty.block_maybe());
    assert!(!tty.flags.contains(TtyFlags::NOBLOCK));
    let (read, write) = rmux_sys::fd::pipe().unwrap();
    tty.fd = write;
    tty.set_redraw_bytes(100);
    tty.add(b"abc");
    assert_eq!(tty.on_writable().unwrap(), 3);
    assert_eq!(tty.redraw_bytes, 97);
    drop(read);
}

#[test]
fn cursor_every_movement_branch() {
    type MoveCase = ((u32, u32), (u32, u32), &'static [u8]);
    let cases: &[MoveCase] = &[
        ((80, 4), (80, 4), b""),
        ((80, 4), (100, 4), b"<p4,79>"),
        ((u32::MAX, u32::MAX), (0, 0), b"<p0,0>"),
        ((3, 4), (0, 0), b"H"),
        ((3, 4), (0, 5), b"\r\n"),
        ((3, 4), (0, 4), b"\r"),
        ((3, 4), (2, 4), b"L"),
        ((3, 4), (4, 4), b"R"),
        ((10, 4), (2, 4), b"<x2>"),
        ((10, 4), (8, 4), b"LL"),
        ((10, 4), (7, 4), b"<l3>"),
        ((2, 4), (5, 4), b"<r3>"),
        ((5, 4), (5, 3), b"U"),
        ((5, 4), (5, 5), b"D"),
        ((5, 15), (5, 4), b"<y4>"),
        ((5, 10), (5, 7), b"<u3>"),
        ((5, 3), (5, 5), b"<d2>"),
        ((3, 4), (7, 8), b"<p8,7>"),
    ];
    for &(from, to, expected) in cases {
        let (mut tty, _master, mut state) = fixture();
        (tty.cx, tty.cy) = from;
        tty.cursor(&mut state, to.0, to.1);
        assert_eq!(bytes(&mut tty), expected, "{from:?} -> {to:?}");
    }
    let (mut tty, _master, mut state) = fixture();
    remove(&mut tty, b"vpa@");
    (tty.cx, tty.cy) = (5, 15);
    tty.cursor(&mut state, 5, 4);
    assert_eq!(bytes(&mut tty), b"<p4,5>");
    remove(&mut tty, b"cub1@");
    (tty.cx, tty.cy) = (10, 4);
    tty.cursor(&mut state, 8, 4);
    assert_eq!(bytes(&mut tty), b"<l2>");
    remove(&mut tty, b"cub@");
    (tty.cx, tty.cy) = (10, 4);
    tty.cursor(&mut state, 8, 4);
    assert_eq!(bytes(&mut tty), b"<p4,8>");
    tty.flags.insert(TtyFlags::BLOCK);
    tty.cursor(&mut state, 1, 1);
    assert_eq!((tty.cx, tty.cy), (8, 4));
}

#[test]
fn margins_region_dedupe_and_invalidation() {
    let (mut tty, _master, mut state) = fixture();
    tty.term_mut()
        .apply(b"Cmg=<m%p1%d,%p2%d>:Clmg=M", false, TtyTermFlags(0));
    tty.term_mut().apply_overrides(&mut state, &[]);
    tty.cx = 80;
    tty.cy = 3;
    tty.region(&mut state, 2, 8);
    assert_eq!(bytes(&mut tty), b"<p3,0><r2,8>");
    assert_eq!(tty.cx, u32::MAX);
    tty.region(&mut state, 2, 8);
    assert!(bytes(&mut tty).is_empty());
    tty.margin(&mut state, 2, 10);
    assert_eq!(bytes(&mut tty), b"<r2,8><m2,10>");
    tty.margin(&mut state, 2, 10);
    assert!(bytes(&mut tty).is_empty());
    (tty.cx, tty.cy) = (3, 4);
    tty.cursor(&mut state, 0, 4);
    assert_eq!(bytes(&mut tty), b"<x0>");
    (tty.cx, tty.cy) = (3, 4);
    tty.cursor(&mut state, 0, 5);
    assert_eq!(bytes(&mut tty), b"<p5,0>");
    tty.margin_off(&mut state);
    assert_eq!(bytes(&mut tty), b"<r2,8>M");
    tty.invalidate(&mut state);
    assert!(bytes(&mut tty).is_empty());
    assert_eq!(tty.cx, u32::MAX);
    assert_eq!(tty.rlower, u32::MAX);
    tty.flags.insert(TtyFlags::STARTED);
    tty.invalidate(&mut state);
    assert_eq!(bytes(&mut tty), b"ZN<p0,0><r0,23><r0,23>M");
}

#[test]
fn putc_putn_noam_wrap_and_capability_guards() {
    let (mut tty, _master, mut state) = fixture();
    tty.set_size(5, 3, 0, 0);
    tty.rlower = 2;
    tty.cx = 4;
    tty.cy = 0;
    tty.putn(&mut state, b"ab", 2);
    assert_eq!((tty.cx, tty.cy), (1, 1));
    tty.cx = 5;
    tty.cy = 1;
    tty.putc(&mut state, b'x');
    assert_eq!((tty.cx, tty.cy), (1, 2));
    tty.cx = 5;
    tty.cy = 2;
    tty.putc(&mut state, b'x');
    assert_eq!((tty.cx, tty.cy), (1, 2));
    bytes(&mut tty);
    remove(&mut tty, b"am@");
    tty.term_mut().apply_overrides(&mut state, &[]);
    tty.cx = 4;
    tty.cy = 2;
    tty.putc(&mut state, b'x');
    assert!(bytes(&mut tty).is_empty());
    tty.cx = 2;
    tty.cy = 2;
    tty.putn(&mut state, b"abcd", 4);
    assert_eq!(bytes(&mut tty), b"ab");
    assert_eq!((tty.cx, tty.cy), (1, 3));
    tty.cx = 5;
    tty.cy = 0;
    tty.putc(&mut state, b'x');
    assert_eq!(bytes(&mut tty), b"x<p1,1>");
    tty.cx = 1;
    tty.cy = 0;
    tty.putn(&mut state, b"x", 20);
    assert_eq!((tty.cx, tty.cy), (u32::MAX, u32::MAX));
    bytes(&mut tty);
    tty.putcode_i(&mut state, C::Cup, -1);
    tty.putcode_ii(&mut state, C::Cup, 0, -1);
    tty.putcode_iii(&mut state, C::Cup, 0, 0, -1);
    tty.puts(b"before\0after");
    assert_eq!(bytes(&mut tty), b"before");
}

#[test]
fn attribute_order_reset_ax_and_underline_cache() {
    let (mut tty, _master, mut state) = fixture();
    tty.opts.default_terminal = ByteString::from("tmux-256color");
    let gc = GridCell {
        attr: A::BRIGHT
            | A::DIM
            | A::ITALICS
            | A::UNDERSCORE
            | A::BLINK
            | A::REVERSE
            | A::HIDDEN
            | A::STRIKETHROUGH
            | A::OVERLINE
            | A::CHARSET,
        fg: Colour(1),
        bg: Colour(2),
        us: Colour(3),
        ..DEFAULT_CELL
    };
    tty.attributes(&mut state, &gc, None);
    assert_eq!(bytes(&mut tty), b"<f1><b2><a3>Bdiubvhxts");
    assert_eq!(tty.cell.us, Colour::DEFAULT);
    tty.attributes(&mut state, &gc, None);
    assert!(bytes(&mut tty).is_empty());
    tty.attributes(&mut state, &DEFAULT_CELL, None);
    assert_eq!(bytes(&mut tty), b"eZ");
    let gc = GridCell {
        fg: Colour(1),
        bg: Colour(2),
        ..DEFAULT_CELL
    };
    tty.attributes(&mut state, &gc, None);
    bytes(&mut tty);
    tty.attributes(
        &mut state,
        &GridCell {
            fg: Colour::DEFAULT,
            ..gc
        },
        None,
    );
    assert_eq!(bytes(&mut tty), b"\x1b[39m");
    remove(&mut tty, b"AX@");
    tty.attributes(&mut state, &DEFAULT_CELL, None);
    assert_eq!(bytes(&mut tty), b"Z");
    tty.cell.us = Colour(2);
    tty.cell.attr = A::BRIGHT;
    tty.last_cell = DEFAULT_CELL;
    tty.last_cell.fg = Colour(1);
    tty.attributes(
        &mut state,
        &GridCell {
            us: Colour(0),
            ..DEFAULT_CELL
        },
        None,
    );
    assert_eq!(bytes(&mut tty), b"Z<a0>");
    tty.cell.us = Colour(2);
    tty.last_cell.fg = Colour(1);
    tty.attributes(&mut state, &DEFAULT_CELL, None);
    assert_eq!(bytes(&mut tty), b"O");
    assert_eq!(tty.cell.us, Colour::DEFAULT);
}

#[test]
fn colour_downgrade_tables_palette_theme_and_fallback() {
    let (mut tty, _master, mut state) = fixture();
    for colours in [8, 16, 256] {
        tty.term_mut().apply(
            format!("colors={colours}").as_bytes(),
            false,
            TtyTermFlags(0),
        );
        for index in 0..256 {
            let c = Colour(ColourFlags::_256.bits() as i32 | index);
            let mut gc = GridCell {
                fg: c,
                bg: c,
                ..DEFAULT_CELL
            };
            tty.check_fg(None, &mut gc);
            tty.check_bg(None, &mut gc);
            if colours == 256 {
                assert_eq!((gc.fg, gc.bg), (c, c));
            } else {
                let mapped = i32::from(indexed_to_16(c));
                let expected = if mapped & 8 == 0 {
                    mapped
                } else if colours == 16 {
                    90 + (mapped & 7)
                } else {
                    mapped & 7
                };
                assert_eq!((gc.fg, gc.bg), (Colour(expected), Colour(expected)));
            }
        }
    }
    let mut palette = ColourPalette::new();
    palette.set(8, Colour(4));
    let mut gc = GridCell {
        fg: Colour(0),
        attr: A::BRIGHT,
        ..DEFAULT_CELL
    };
    tty.check_fg(Some(&palette), &mut gc);
    assert_eq!(gc.fg, Colour(4));
    gc.flags.insert(GridCellFlags::NOPALETTE);
    gc.fg = Colour(0);
    tty.check_fg(Some(&palette), &mut gc);
    assert_eq!(gc.fg, Colour(0));
    tty.host.theme_colours[0] = Colour::rgb(1, 2, 3).0;
    assert_eq!(
        tty.map_theme_colour(Colour(ColourFlags::THEME.bits() as i32)),
        Colour::rgb(1, 2, 3)
    );
    assert_eq!(
        tty.map_theme_colour(Colour(ColourFlags::THEME.bits() as i32 | 10)),
        Colour::DEFAULT
    );
    remove(&mut tty, b"setaf@");
    tty.term_mut().apply(b"colors=256", false, TtyTermFlags(0));
    let gc = GridCell {
        fg: Colour(ColourFlags::_256.bits() as i32 | 200),
        ..DEFAULT_CELL
    };
    tty.attributes(&mut state, &gc, None);
    assert_eq!(bytes(&mut tty), b"<b200>");
    tty.term_mut().apply(
        b"setrgbb=<b%p1%d,%p2%d,%p3%d>:setrgbf=unused",
        false,
        TtyTermFlags(0),
    );
    tty.term_mut().apply_overrides(&mut state, &[]);
    remove(&mut tty, b"setrgbf@");
    tty.attributes(
        &mut state,
        &GridCell {
            fg: Colour::rgb(1, 2, 3),
            ..DEFAULT_CELL
        },
        None,
    );
    assert_eq!(bytes(&mut tty), b"Z<b1,2,3>");
}

#[test]
fn rgb_underline_present_false_rgb_and_missing_store_hyperlinks() {
    let (mut tty, _master, mut state) = fixture();
    remove(&mut tty, b"Setulc1@");
    tty.term_mut().apply(
        b"setal=<a%p1%d>:RGB=0:setrgbf=f:setrgbb=b",
        false,
        TtyTermFlags(0),
    );
    tty.term_mut().apply_overrides(&mut state, &[]);
    let gc = GridCell {
        us: Colour::rgb(1, 2, 3),
        link: HyperlinkId(1),
        ..DEFAULT_CELL
    };
    tty.attributes(&mut state, &gc, None);
    assert_eq!(bytes(&mut tty), b"<a66051>");
    assert_eq!(tty.cell.link, HyperlinkId(1));
    let registry = HyperlinkRegistry::new();
    let style = crate::draw::TtyStyleCtx {
        defaults: &DEFAULT_CELL,
        palette: None,
        dim: 50,
        hyperlinks: None,
    };
    tty.attributes(&mut state, &DEFAULT_CELL, Some(&style));
    assert_eq!(bytes(&mut tty), b"O");
    assert_eq!(
        (tty.last_cell.fg, tty.last_cell.bg),
        (Colour::DEFAULT, Colour::DEFAULT)
    );
    drop(registry);
}

#[test]
fn cursor_styles_mouse_modes_colour_title_path_and_progress() {
    let (mut tty, _master, mut state) = fixture();
    tty.term_mut().apply(
        b"kmous=k:tsl=T:fsl=F:Swd=W:Spb=<v%p1%d,%p2%d>",
        false,
        TtyTermFlags(0),
    );
    let mut registry = HyperlinkRegistry::default();
    let mut screen = Screen::new(80, 24, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    for (style, base) in [
        (ScreenCursorStyle::Block, 1),
        (ScreenCursorStyle::Underline, 3),
        (ScreenCursorStyle::Bar, 5),
    ] {
        for blink in [false, true] {
            tty.cstyle = ScreenCursorStyle::Default;
            tty.mode = ScreenMode(0);
            screen.cstyle = style;
            let mode = if blink {
                ScreenMode::CURSOR | ScreenMode::CURSOR_BLINKING
            } else {
                ScreenMode::CURSOR
            };
            tty.update_mode(&mut state, mode, Some(&screen));
            assert_eq!(
                bytes(&mut tty),
                format!("N<s{}>", base + i32::from(!blink)).as_bytes()
            );
        }
    }
    screen.cstyle = ScreenCursorStyle::Default;
    screen.default_cstyle = ScreenCursorStyle::Default;
    tty.update_mode(&mut state, ScreenMode::CURSOR, Some(&screen));
    assert_eq!(bytes(&mut tty), b"NE");
    for (mode, suffix) in [
        (ScreenMode::MOUSE_STANDARD, b"\x1b[?1000h".as_slice()),
        (
            ScreenMode::MOUSE_BUTTON,
            b"\x1b[?1000h\x1b[?1002h".as_slice(),
        ),
        (
            ScreenMode::MOUSE_ALL,
            b"\x1b[?1000h\x1b[?1002h\x1b[?1003h".as_slice(),
        ),
    ] {
        tty.mode = ScreenMode::CURSOR;
        tty.update_mode(&mut state, ScreenMode::CURSOR | mode, None);
        let mut expected = b"\x1b[?1006l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006h".to_vec();
        expected.extend_from_slice(suffix);
        assert_eq!(bytes(&mut tty), expected);
    }
    tty.force_cursor_colour(&mut state, Colour::rgb(1, 2, 255).0);
    assert_eq!(bytes(&mut tty), b"<crgb:01/02/ff>");
    tty.force_cursor_colour(&mut state, -1);
    assert_eq!(bytes(&mut tty), b"c");
    tty.set_title(b"a\0b");
    tty.set_path(b"/x\0y");
    tty.set_progress_bar(
        &mut state,
        &ProgressBar {
            state: ProgressBarState::Normal,
            progress: 101,
        },
    );
    assert_eq!(bytes(&mut tty), b"TaFW/xF<v1,101>");
    screen.release(&mut registry).unwrap();
}

#[test]
fn clipboard_missing_duplicate_timeout_empty_and_blocked_selection() {
    let (mut tty, _master, mut state) = fixture();
    tty.flags.insert(TtyFlags::STARTED);
    tty.clipboard_query(&mut state);
    tty.clipboard_query(&mut state);
    assert_eq!(bytes(&mut tty), b"<m,?>");
    assert_eq!(
        tty.pending_timers()
            .filter(|t| t.timer == TtyTimer::Clipboard)
            .count(),
        1
    );
    tty.on_timer(&mut state, TtyTimer::Clipboard);
    assert!(!tty.flags.contains(TtyFlags::OSC52QUERY));
    tty.set_selection(&mut state, "c", b"");
    assert_eq!(bytes(&mut tty), b"<mc,>");
    assert!(tty.flags.contains(TtyFlags::NOBLOCK));
    tty.flags.insert(TtyFlags::BLOCK);
    tty.set_selection(&mut state, "c", b"\0a");
    assert!(bytes(&mut tty).is_empty());
    assert!(tty.discarded > 0);
    tty.flags.remove(TtyFlags::BLOCK);
    remove(&mut tty, b"Ms@");
    tty.clipboard_query(&mut state);
    assert!(tty.flags.contains(TtyFlags::OSC52QUERY));
    assert!(bytes(&mut tty).is_empty());
}

#[test]
fn requests_timer_features_resize_and_window_offset() {
    let (mut tty, master, mut state) = fixture();
    let now = UNIX_EPOCH + Duration::from_secs(100);
    tty.send_requests(now);
    assert!(bytes(&mut tty).is_empty());
    tty.flags.insert(TtyFlags::STARTED);
    tty.send_requests(now);
    assert!(bytes(&mut tty).is_empty());
    assert!(tty.flags.contains(TtyFlags::ALL_REQUEST_FLAGS));
    tty.repeat_requests(false, now + Duration::from_secs(30));
    assert_eq!(tty.pending_timers().count(), 0);
    tty.repeat_requests(false, now + Duration::from_secs(31));
    assert_eq!(tty.pending_timers().count(), 2);
    tty.repeat_requests(false, now);
    assert_eq!(tty.pending_timers().count(), 2);
    tty.term_mut().apply(b"XT=1", false, TtyTermFlags(0));
    let mut host = tty.host.clone();
    let mut vt_caps = caps();
    vt_caps.push(ByteString::from("XT=1"));
    tty.term = Some(
        TtyTerm::create(&mut state, b"fixture", &vt_caps, &mut host, &tty.opts, None).unwrap(),
    );
    tty.flags.remove(TtyFlags::ALL_REQUEST_FLAGS);
    tty.send_requests(now);
    assert_eq!(
        bytes(&mut tty),
        b"\x1b[c\x1b[>c\x1b[>q\x1b[?2026$p\x1b]10;?\x1b\\\x1b]11;?\x1b\\"
    );
    tty.flags.insert(TtyFlags::HAVEDA);
    tty.send_requests(now);
    assert_eq!(
        bytes(&mut tty),
        b"\x1b[>c\x1b[>q\x1b[?2026$p\x1b]10;?\x1b\\\x1b]11;?\x1b\\"
    );
    tty.on_timer(&mut state, TtyTimer::Start);
    assert!(tty.flags.contains(TtyFlags::ALL_REQUEST_FLAGS));
    assert!(!tty.flags.intersects(TtyFlags::WAITBG | TtyFlags::WAITFG));
    tty.flags.remove(TtyFlags::STARTED);
    rmux_sys::pty::set_winsize(master.as_fd(), rmux_sys::pty::Winsize::default()).unwrap();
    tty.resize(&mut state);
    assert_eq!(tty.size(), (80, 24));
    assert_eq!(tty.pixel_size(), (0, 0));
    assert_eq!(bytes(&mut tty), b"\x1b[18t\x1b[14t");
    tty.resize(&mut state);
    assert!(bytes(&mut tty).is_empty());
    tty.set_window_offset(true, 1, 2, 3, 4);
    assert_eq!(tty.window_offset(), (true, 1, 2, 3, 4));
}

#[test]
fn features_emit_before_redraw_and_start_timeout_discovery() {
    let (mut tty, _master, mut state) = fixture();
    tty.term_mut().apply(
        b"Cmg=m:Clmg=M:Enmg=g:Eneks=k:Enfcs=f:Enesc=e",
        false,
        TtyTermFlags(0),
    );
    tty.term_mut().apply_overrides(&mut state, &[]);
    let opts = TtyOptions {
        extended_keys: true,
        focus_events: true,
        ..TtyOptions::default()
    };
    tty.update_features(&mut state, &opts);
    assert_eq!(bytes(&mut tty), b"gkfe");
    assert_eq!(tty.drain_effects().last(), Some(TtyEffect::RedrawClient));
    tty.flags.insert(TtyFlags::WAITBG | TtyFlags::WAITFG);
    tty.on_timer(&mut state, TtyTimer::Start);
    assert_eq!(bytes(&mut tty), b"gkfe");
    assert!(tty.flags.contains(TtyFlags::ALL_REQUEST_FLAGS));
    assert!(!tty.flags.intersects(TtyFlags::WAITBG | TtyFlags::WAITFG));
    tty.on_timer(&mut state, TtyTimer::Start);
    assert!(bytes(&mut tty).is_empty());
}

#[test]
fn hyperlink_store_changes_do_not_bypass_numeric_cache() {
    let (mut tty, _master, mut state) = fixture();
    let mut registry = HyperlinkRegistry::default();
    let links = registry.create().unwrap();
    let id = registry.put(&links, b"https://example.test", None).unwrap();
    let gc = GridCell {
        link: id,
        ..DEFAULT_CELL
    };
    tty.attributes(&mut state, &gc, None);
    assert!(bytes(&mut tty).is_empty());
    let style = crate::draw::TtyStyleCtx {
        defaults: &DEFAULT_CELL,
        palette: None,
        dim: 0,
        hyperlinks: Some((&registry, &links)),
    };
    tty.attributes(&mut state, &gc, Some(&style));
    assert!(bytes(&mut tty).is_empty());
    tty.attributes(&mut state, &DEFAULT_CELL, Some(&style));
    assert_eq!(bytes(&mut tty), b"<h,>");
    tty.attributes(&mut state, &gc, Some(&style));
    let record = registry.get(&links, id).unwrap();
    let mut expected = b"<h".to_vec();
    expected.extend_from_slice(record.external_id());
    expected.extend_from_slice(b",https://example.test>");
    assert_eq!(bytes(&mut tty), expected);
    registry.reset(&links).unwrap();
    let style = crate::draw::TtyStyleCtx {
        defaults: &DEFAULT_CELL,
        palette: None,
        dim: 0,
        hyperlinks: Some((&registry, &links)),
    };
    let gc = GridCell {
        link: HyperlinkId(id.0 + 1),
        ..DEFAULT_CELL
    };
    tty.attributes(&mut state, &gc, Some(&style));
    assert_eq!(bytes(&mut tty), b"<h,>");
    registry.release(links).unwrap();
}

#[test]
fn cursor_region_boundaries_and_fallback_visibility() {
    let (mut tty, _master, mut state) = fixture();
    tty.rupper = 4;
    tty.rlower = 10;
    tty.cx = 5;
    tty.cy = 4;
    tty.cursor(&mut state, 5, 3);
    assert_eq!(bytes(&mut tty), b"<y3>");
    tty.cx = 5;
    tty.cy = 10;
    tty.cursor(&mut state, 5, 11);
    assert_eq!(bytes(&mut tty), b"<y11>");
    remove(&mut tty, b"Ss@:Se@");
    let mut registry = HyperlinkRegistry::default();
    let mut screen = Screen::new(80, 24, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    screen.cstyle = ScreenCursorStyle::Bar;
    tty.mode = ScreenMode::CURSOR;
    tty.cstyle = ScreenCursorStyle::Default;
    tty.update_mode(
        &mut state,
        ScreenMode::CURSOR | ScreenMode::CURSOR_BLINKING,
        Some(&screen),
    );
    assert_eq!(bytes(&mut tty), b"NV");
    tty.flags.insert(TtyFlags::NOCURSOR);
    tty.update_mode(&mut state, ScreenMode::CURSOR, Some(&screen));
    assert_eq!(bytes(&mut tty), b"I");
    screen.release(&mut registry).unwrap();
}

#[test]
fn parameter_variables_belong_to_process_not_tty() {
    let (mut a, _master, mut state) = fixture();
    let (mut b, _master_b, _unused_state) = fixture();
    a.term_mut().apply(b"hpa=%p1%PA", false, TtyTermFlags(0));
    b.term_mut()
        .apply(b"hpa=%gA%p1%+%d", false, TtyTermFlags(0));
    a.putcode_i(&mut state, C::Hpa, 42);
    b.putcode_i(&mut state, C::Hpa, 0);
    assert_eq!(bytes(&mut b), b"42");
}

use crate::test_common as common;
#[test]
fn lifecycle_cursor_and_attributes_match_pinned_c() {
    use std::fmt::Write;
    use std::path::Path;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/core_reference.c");
    let mut flags = vec!["-ffunction-sections", "-DHAVE_CURSES_H", "-DHAVE_TIPARM_S"];
    if cfg!(target_os = "macos") {
        flags.extend([
            "-Wl,-dead_strip",
            "-I/opt/homebrew/opt/ncurses/include",
            "-L/opt/homebrew/opt/ncurses/lib",
            "-L/opt/homebrew/opt/libevent/lib",
        ]);
    } else {
        flags.extend(["-Wl,--gc-sections", "-Wl,--no-as-needed", "-lutil"]);
    }
    flags.extend(["-levent", "-lncurses", "-lresolv"]);
    let Some(bin) = common::build_c(
        "tty-core",
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
    let input = "cursor 3 4\nmove 0 0\ndump\ncursor 3 4\nmove 0 5\ndump\ncursor 3 4\nmove 0 4\ndump\ncursor 3 4\nmove 2 4\ndump\ncursor 3 4\nmove 4 4\ndump\ncursor 10 4\nmove 2 4\ndump\ncursor 10 4\nmove 8 4\ndump\ncursor 10 4\nmove 7 4\ndump\ncursor 2 4\nmove 5 4\ndump\ncursor 5 4\nmove 5 3\ndump\ncursor 5 4\nmove 5 5\ndump\ncursor 5 15\nmove 5 4\ndump\ncursor 5 10\nmove 5 7\ndump\ncursor 5 3\nmove 5 5\ndump\ncursor 3 4\nmove 7 8\ndump\ncursor 4294967295 4294967295\nmove 0 0\ndump\nattrs 8703 0 1 2 3 0 1 78\ndump\nattrs 0 0 8 8 8 0 1 78\ndump\nattrs 0 0 1 2 8 0 1 78\ndump\nattrs 0 0 8 2 8 0 1 78\ndump\nreset\ndump\nstart 1\ndump\nstop\ndump\nstart 0\ndump\nstop\ndump\n";
    let input = input
        .replace("start 1\ndump", "start 1\ntermios\ndump")
        .replace("start 0\ndump", "start 0\ntermios\ndump")
        .replace("stop\ndump", "stop\ntermios\ndump");
    let input = format!(
        "{input}size 4 2\nttyflags 0\naddcount 64\nblock\nflowdump\naddcount 1\nblock\nflowdump\naddcount 2\nflowdump\nblocktimer\nflowdump\nblocktimer\nflowdump\nttyflags 8\naddcount 65\nblock\nflowdump\nclearout\nblock\nflowdump\nttyflags 16\naddcount 65\nblock\nflowdump\nblocktimer\nflowdump\nclearout\nredraw 100\naddcount 3\nwritable\nflowdump\nredraw 1\naddcount 3\nwritable\nflowdump\n"
    );
    let expected = common::run(&bin, &[], input.as_bytes());
    let (mut tty, master, mut state) = fixture();
    tty.opts.default_terminal = ByteString::from("tmux-256color");
    let mut output = String::new();
    let mut client_discarded = 0usize;
    for line in input.lines() {
        let mut tokens = line.split_whitespace();
        let op = tokens.next().unwrap();
        let n = |t: &mut std::str::SplitWhitespace<'_>| t.next().unwrap().parse::<u32>().unwrap();
        match op {
            "cursor" => {
                tty.cx = n(&mut tokens);
                tty.cy = n(&mut tokens);
            }
            "move" => {
                let x = n(&mut tokens);
                let y = n(&mut tokens);
                tty.cursor(&mut state, x, y);
            }
            "attrs" => {
                let gc = GridCell {
                    attr: A(n(&mut tokens) as u16),
                    flags: GridCellFlags(n(&mut tokens) as u8),
                    fg: Colour(n(&mut tokens) as i32),
                    bg: Colour(n(&mut tokens) as i32),
                    us: Colour(n(&mut tokens) as i32),
                    link: HyperlinkId(n(&mut tokens)),
                    ..DEFAULT_CELL
                };
                tty.attributes(&mut state, &gc, None);
            }
            "reset" => tty.reset(&mut state),
            "start" => {
                let opts = TtyOptions {
                    clear_on_attach: n(&mut tokens) != 0,
                    default_terminal: ByteString::from("tmux-256color"),
                    ..TtyOptions::default()
                };
                tty.start(&mut state, &opts);
            }
            "stop" => {
                let opts = tty.opts.clone();
                tty.stop(&mut state, &opts);
                rmux_sys::fd::set_blocking(master.as_fd(), false);
                let mut b = [0; 4096];
                let n = rmux_sys::fd::read(master.as_fd(), &mut b).unwrap_or(0);
                tty.out.extend(&b[..n]);
            }
            "termios" => {
                let tio = TermiosState::get(tty.fd()).unwrap();
                writeln!(
                    output,
                    "termios {} {} {} {} {} {}",
                    tio.iflag(),
                    tio.oflag(),
                    tio.lflag(),
                    tio.cflag(),
                    tio.minimum_read(),
                    tio.read_timeout()
                )
                .unwrap();
            }
            "size" => {
                tty.sx = n(&mut tokens);
                tty.sy = n(&mut tokens);
            }
            "ttyflags" => tty.flags = TtyFlags(n(&mut tokens)),
            "addcount" => tty.add(&vec![b'x'; n(&mut tokens) as usize]),
            "block" => {
                tty.block_maybe();
            }
            "blocktimer" => tty.on_timer(&mut state, TtyTimer::Block),
            "clearout" => tty.out.clear(),
            "redraw" => tty.set_redraw_bytes(n(&mut tokens) as usize),
            "writable" => {
                tty.on_writable().unwrap();
            }
            "flowdump" => {
                for effect in tty.drain_effects() {
                    if let TtyEffect::Discarded(n) = effect {
                        client_discarded += n;
                    }
                }
                write!(
                    output,
                    "flow {} {} {} {} {} ",
                    tty.flags.bits(),
                    tty.out.len(),
                    client_discarded,
                    tty.discarded,
                    tty.redraw_bytes
                )
                .unwrap();
                for b in &tty.out {
                    write!(output, "{b:02x}").unwrap();
                }
                output.push('\n');
            }
            "dump" => {
                write!(
                    output,
                    "{} {} {} {} {} {} {} {} {} {} {} ",
                    tty.cx,
                    tty.cy,
                    tty.rupper,
                    tty.rlower,
                    tty.rleft,
                    tty.rright,
                    tty.cell.attr.bits(),
                    tty.cell.fg.0,
                    tty.cell.bg.0,
                    tty.cell.us.0,
                    tty.cell.link.0
                )
                .unwrap();
                for b in tty.out.drain(..) {
                    write!(output, "{b:02x}").unwrap();
                }
                output.push('\n');
            }
            _ => panic!("unknown core corpus operation"),
        }
    }
    assert_eq!(output.as_bytes(), expected);
}
