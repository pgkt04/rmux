//! Parser unit checks from spec 6.3: pending bytes, discard limits, UTF-8,
//! SGR, OSC 8/133, reset retention, effect order and byte-split continuation.

use rmux_emu::cell::{GridAttributes, GridCell};
use rmux_emu::colour::{Colour, ColourFlags, ColourPalette};
use rmux_emu::grid::GridLineFlags;
use rmux_emu::hyperlinks::HyperlinkRegistry;
#[cfg(feature = "sixel")]
use rmux_emu::image::ImageRegistry;
use rmux_emu::input::dump::{capture_pane, state_line};
use rmux_emu::input::{
    ColourQueryKind, InputCtx, InputEffect, InputEnd, InputPolicy, InputRequestKind, InputSink,
    InputStep, NullSink, Osc133Event, Passthrough,
};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};

/// Owned copy of one effect for assertions.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Rec {
    Reply(Vec<u8>),
    TspMessage(Vec<u8>),
    TerminalReset,
    Request(InputRequestKind, InputEnd),
    ClipboardQuery(u8, InputEnd),
    ClipboardReceived(Vec<u8>, Vec<u8>),
    ColourQuery(ColourQueryKind, InputEnd),
    ThemeReport,
    ThemeUpdates(bool),
    Bell,
    TitleChanged(Vec<u8>),
    TitlePopped(Vec<u8>),
    PathChanged,
    Rename(Option<Vec<u8>>),
    ProgressChanged,
    StyleChanged(bool),
    SyncStart,
    SyncEnd,
    Osc133(Osc133Event),
    GroundTimer(bool),
    AlternateChanged(bool),
}

#[derive(Default)]
struct Recorder(Vec<Rec>);

impl InputSink for Recorder {
    fn effect(&mut self, effect: InputEffect<'_>) {
        self.0.push(match effect {
            InputEffect::Reply(b) => Rec::Reply(b.to_vec()),
            InputEffect::TspMessage { payload } => Rec::TspMessage(payload.to_vec()),
            InputEffect::TerminalReset => Rec::TerminalReset,
            InputEffect::Request { kind, end } => Rec::Request(kind, end),
            InputEffect::ClipboardQuery { clip, end } => Rec::ClipboardQuery(clip, end),
            InputEffect::ClipboardReceived { clip, data } => {
                Rec::ClipboardReceived(clip.to_vec(), data.to_vec())
            }
            InputEffect::ColourQuery { which, end } => Rec::ColourQuery(which, end),
            InputEffect::ThemeReport => Rec::ThemeReport,
            InputEffect::ThemeUpdatesEnabled => Rec::ThemeUpdates(true),
            InputEffect::ThemeUpdatesDisabled => Rec::ThemeUpdates(false),
            InputEffect::Bell => Rec::Bell,
            InputEffect::TitleChanged(t) => Rec::TitleChanged(t.to_vec()),
            InputEffect::TitlePopped(t) => Rec::TitlePopped(t.to_vec()),
            InputEffect::PathChanged => Rec::PathChanged,
            InputEffect::Rename(n) => Rec::Rename(n.map(<[u8]>::to_vec)),
            InputEffect::ProgressChanged => Rec::ProgressChanged,
            InputEffect::StyleChanged { theme } => Rec::StyleChanged(theme),
            InputEffect::SyncStart => Rec::SyncStart,
            InputEffect::SyncEnd => Rec::SyncEnd,
            InputEffect::Osc133(e) => Rec::Osc133(e),
            InputEffect::GroundTimer(arm) => Rec::GroundTimer(arm),
            InputEffect::AlternateChanged { entering } => Rec::AlternateChanged(entering),
        });
    }
    fn reply(&mut self, _: &[u8]) {}
}

struct Fixture {
    registry: HyperlinkRegistry,
    #[cfg(feature = "sixel")]
    images: ImageRegistry,
    screen: Screen,
    palette: ColourPalette,
    ictx: InputCtx,
    policy: InputPolicy,
}

impl Fixture {
    fn new(sx: u32, sy: u32) -> Fixture {
        let mut registry = HyperlinkRegistry::new();
        let screen =
            Screen::new(sx, sy, 2000, ScreenResetPolicy::default(), &mut registry).unwrap();
        #[cfg(feature = "sixel")]
        let (screen, images) = {
            let mut screen = screen;
            let mut images = ImageRegistry::default();
            screen.bind_images(&mut images);
            (screen, images)
        };
        Fixture {
            registry,
            #[cfg(feature = "sixel")]
            images,
            screen,
            palette: ColourPalette::new(),
            ictx: InputCtx::new(),
            policy: InputPolicy::default(),
        }
    }
    fn feed(&mut self, sink: &mut dyn InputSink, bytes: &[u8]) {
        let mut tty = ScreenOnlySink;
        let mut sw = ScreenWriteCtx::start(
            &mut self.screen,
            &mut tty,
            ScreenWritePolicy {
                pane_backed: self.policy.writer_has_pane,
                ..ScreenWritePolicy::default()
            },
            &mut self.registry,
            #[cfg(feature = "sixel")]
            Some(&mut self.images),
        );
        self.ictx
            .parse(&mut sw, Some(&mut self.palette), &self.policy, sink, bytes);
        sw.finish();
    }
    fn run(&mut self, bytes: &[u8]) -> Vec<Rec> {
        let mut rec = Recorder::default();
        self.feed(&mut rec, bytes);
        rec.0
    }
    fn text(&self, y: u32) -> String {
        let line = self.screen.grid.get_line(self.screen.grid.hsize() + y);
        let mut s = String::new();
        for x in 0..line.cellused() {
            let gc = line.get_cell(x);
            if gc.flags.contains(rmux_emu::cell::GridCellFlags::PADDING) {
                continue;
            }
            s.push_str(&String::from_utf8_lossy(gc.data.bytes()));
        }
        s.trim_end().to_owned()
    }
    fn dump(&self) -> (Vec<u8>, String) {
        (
            capture_pane(&self.screen, &self.registry),
            state_line(&self.screen),
        )
    }
}

#[test]
fn since_ground_tracks_unfinished_sequence() {
    let mut f = Fixture::new(10, 4);
    f.run(b"\x1b[31");
    assert_eq!(f.ictx.pending(), b"\x1b[31");
    assert_eq!(f.ictx.state_name(), "csi_parameter");
    f.run(b"m");
    assert_eq!(f.ictx.pending(), b"");
    assert_eq!(f.ictx.state_name(), "ground");
    assert_eq!(f.ictx.cell().fg, Colour(1));
}

#[test]
fn discard_limits() {
    let mut f = Fixture::new(10, 4);
    f.run(b"\x1b[   !m");
    assert_eq!(f.ictx.cell().attr, GridAttributes(0));
    f.run(b"\x1b[1    m");
    assert_eq!(f.ictx.cell().attr, GridAttributes(0));
    f.run(b"\x1b[1  m");
    assert_eq!(f.ictx.cell().attr, GridAttributes(0));
    let params = "1;".repeat(31) + "1m";
    f.run(format!("\x1b[{params}").as_bytes());
    assert_eq!(f.ictx.cell().attr, GridAttributes(0));
    let params = "1;".repeat(30) + "1m";
    f.run(format!("\x1b[{params}").as_bytes());
    assert_eq!(f.ictx.cell().attr, GridAttributes(0));
    f.run(b"\x1b[1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1m");
    assert_eq!(f.ictx.cell().attr, GridAttributes::BRIGHT);
    f.run(b"\x1b[0;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1m");
    assert_eq!(f.ictx.cell().attr, GridAttributes::BRIGHT);
    f.policy.buffer_limit = 64;
    let title = "x".repeat(70);
    let rec = f.run(format!("\x1b]2;{title}\x07").as_bytes());
    assert!(rec.iter().all(|r| !matches!(r, Rec::TitleChanged(_))));
    assert!(f.screen.title.is_empty());
    let rec = f.run(b"\x1b]2;short\x07");
    assert_eq!(
        rec,
        vec![
            Rec::GroundTimer(true),
            Rec::TitleChanged(b"short".to_vec()),
            Rec::GroundTimer(false)
        ]
    );
    assert_eq!(f.screen.title, b"short");
}

#[test]
fn utf8_replacement_and_split() {
    let mut f = Fixture::new(10, 4);
    f.run(b"\xf0\x80\x80\x80A\xed\xa0\x80B");
    assert_eq!(f.text(0), "\u{fffd}A\u{fffd}B");
    let mut g = Fixture::new(10, 4);
    g.run(b"\xe4");
    g.run(b"\xb8");
    g.run(b"\xad");
    assert_eq!(g.text(0), "中");
    let mut h = Fixture::new(10, 4);
    h.run(b"\xe4\x1b[1m\xb8\xadX");
    assert_eq!(h.text(0), "中X");
    let mut i = Fixture::new(10, 4);
    i.run(b"\xe4\rA");
    assert_eq!(i.text(0), "A");
    assert_eq!(i.screen.cx, 1);
}

#[test]
fn sgr_cases() {
    let mut f = Fixture::new(10, 4);
    f.run(b"\x1b[38;5;999m");
    assert_eq!(f.ictx.cell().fg, Colour::DEFAULT);
    f.run(b"\x1b[38;2;1;2m");
    assert_eq!(
        f.ictx.cell().attr,
        GridAttributes::BRIGHT | GridAttributes::DIM
    );
    f.run(b"\x1b]8;;http://x\x1b\\\x1b[0m");
    assert_ne!(f.ictx.cell().link.0, 0);
    f.run(b"\x1b[m");
    assert_eq!(f.ictx.cell().link.0, 0);
    f.run(b"\x1b[38:2::1:2:3m");
    assert_eq!(f.ictx.cell().fg, Colour::rgb(1, 2, 3));
    f.run(b"\x1b[38:2:1:2:3m");
    assert_eq!(f.ictx.cell().fg, Colour::rgb(1, 2, 3));
    f.run(b"\x1b[38:5:7m");
    assert_eq!(
        f.ictx.cell().fg,
        Colour(7 | ColourFlags::_256.bits() as i32)
    );
    f.run(b"\x1b[0;4:3m");
    assert_eq!(f.ictx.cell().attr, GridAttributes::UNDERSCORE_3);
    f.run(b"\x1b[4:1:2m");
    assert_eq!(f.ictx.cell().attr, GridAttributes::UNDERSCORE_3);
    f.run(b"\x1b[0;1:2:3:4:5:6:7m");
    assert_eq!(f.ictx.cell().attr, GridAttributes(0));
    f.run(b"\x1b[0;58;2;9;9;9;4m");
    assert_eq!(f.ictx.cell().us, Colour::rgb(9, 9, 9));
    assert_eq!(f.ictx.cell().attr, GridAttributes::UNDERSCORE);
    f.run(b"\x1b[21;22;1;2m");
    assert_eq!(
        f.ictx.cell().attr,
        GridAttributes::UNDERSCORE_2 | GridAttributes::BRIGHT | GridAttributes::DIM
    );
}

#[test]
fn osc_8_malformed_cases() {
    let mut f = Fixture::new(10, 4);
    f.run(b"\x1b]8;id=a;http://a\x1b\\");
    let a = f.ictx.cell().link;
    assert_ne!(a.0, 0);
    f.run(b"\x1b]8;id=a:id=b;http://b\x1b\\");
    assert_eq!(f.ictx.cell().link, a);
    f.run(b"\x1b]8;id=;http://c\x1b\\");
    let c = f.ictx.cell().link;
    assert_ne!(c, a);
    f.run(b"\x1b]8;id=a;http://a\x1b\\");
    assert_eq!(f.ictx.cell().link, a);
    f.run(b"\x1b]8;;\x1b\\");
    assert_eq!(f.ictx.cell().link.0, 0);
    f.run(b"\x1b]8;nosemi\x1b\\");
    assert_eq!(f.ictx.cell().link.0, 0);
}

#[test]
fn osc_133_order_and_status() {
    let mut f = Fixture::new(10, 4);
    let rec = f.run(b"\x1b]133;A\x07x\x1b]133;C\x07\x1b]133;D;7\x07");
    let effects: Vec<_> = rec
        .into_iter()
        .filter(|r| matches!(r, Rec::Osc133(_)))
        .collect();
    assert_eq!(
        effects,
        vec![
            Rec::Osc133(Osc133Event::Prompt),
            Rec::Osc133(Osc133Event::CommandStarted),
            Rec::Osc133(Osc133Event::CommandFinished { status: 7 })
        ]
    );
    let gl = f.screen.grid.get_line(0);
    assert!(gl.flags.contains(GridLineFlags::START_PROMPT));
    assert!(gl.flags.contains(GridLineFlags::START_OUTPUT));
    assert!(gl.flags.contains(GridLineFlags::END_OUTPUT));
    assert_eq!(gl.osc133.out_start_col, 1);
    assert_eq!(gl.osc133.out_end_col, 1);
    assert_eq!(gl.osc133.exit_status, 7);
    f.run(b"\x1b]133;A\x07");
    assert_eq!(f.screen.grid.get_line(0).osc133.exit_status, 0);
    assert!(
        f.screen
            .grid
            .get_line(0)
            .flags
            .contains(GridLineFlags::END_OUTPUT)
    );
    f.run(b"\r\n\x1b]133;D;300\x07");
    assert_eq!(f.screen.grid.get_line(1).osc133.exit_status, 255);
    f.run(b"\r\n\x1b]133;D;-1\x07");
    assert_eq!(f.screen.grid.get_line(2).osc133.exit_status, 255);
    f.run(b"\r\n\x1b]133;D\x07");
    assert_eq!(f.screen.grid.get_line(3).osc133.exit_status, 0);
    f.run(b"\x1b]133;P;k=s\x07");
    assert!(
        f.screen
            .grid
            .get_line(3)
            .flags
            .contains(GridLineFlags::SECOND_PROMPT)
    );
}

#[test]
fn reset_retains_pending_and_utf8() {
    let mut f = Fixture::new(10, 4);
    f.run(b"\xe4\x1b[3");
    assert_eq!(f.ictx.pending(), b"\x1b[3");
    let mut rec = Recorder::default();
    f.ictx.reset(None, &mut rec);
    assert_eq!(rec.0, vec![Rec::GroundTimer(false)]);
    assert_eq!(f.ictx.pending(), b"\x1b[3");
    assert_eq!(f.ictx.state_name(), "ground");
    f.run(b"\xb8\xad");
    assert_eq!(f.text(0), "中");
    f.run(b"\x1b[2");
    assert_eq!(f.ictx.pending(), b"\x1b[3\x1b[2");
    f.run(b"J");
    assert!(f.ictx.pending().is_empty());
}

#[test]
fn effects_in_c_order() {
    let mut f = Fixture::new(10, 4);
    let rec = f.run(b"\x07\x1b]0;t\x1b[H");
    assert_eq!(
        rec,
        vec![
            Rec::Bell,
            Rec::GroundTimer(true),
            Rec::TitleChanged(b"t".to_vec()),
            Rec::GroundTimer(false),
        ]
    );
    let rec = f.run(b"\x1b[?2031;2026;1049h\x1b[?2026;1049l");
    assert_eq!(
        rec,
        vec![
            Rec::ThemeUpdates(true),
            Rec::SyncStart,
            Rec::AlternateChanged(true),
            Rec::SyncEnd,
            Rec::AlternateChanged(false),
        ]
    );
    assert!(!f.screen.mode.contains(ScreenMode::SYNC));
    let rec = f.run(b"\x1b[c\x1b[>c\x1b[6n\x1b[?6$p\x1b[>q\x1b[?996n\x1b[18t\x1b[22;0t\x1b[23;0t");
    assert_eq!(
        rec,
        vec![
            #[cfg(not(feature = "sixel"))]
            Rec::Reply(b"\x1b[?1;2c".to_vec()),
            #[cfg(feature = "sixel")]
            Rec::Reply(b"\x1b[?1;2;4c".to_vec()),
            Rec::Reply(b"\x1b[>84;0;0c".to_vec()),
            Rec::Reply(b"\x1b[1;1R".to_vec()),
            Rec::Reply(b"\x1b[?6;2$y".to_vec()),
            Rec::Reply(b"\x1bP>|tmux next-3.9\x1b\\".to_vec()),
            Rec::ThemeReport,
            Rec::Reply(b"\x1b[8;4;10t".to_vec()),
            Rec::TitlePopped(b"t".to_vec()),
        ]
    );
    f.policy.set_clipboard_on = true;
    let rec = f.run(b"\x1b]52;c;aGk=\x07\x1b]52;pc;?\x1b\\\x1b]11;red\x07\x1b]11;?\x07\x1b]4;1;?\x07\x1b]4;1;?;2;blue\x07\x1b]10;?\x07");
    assert_eq!(
        rec,
        vec![
            Rec::GroundTimer(true),
            Rec::ClipboardReceived(b"c".to_vec(), b"hi".to_vec()),
            Rec::GroundTimer(false),
            Rec::GroundTimer(true),
            Rec::ClipboardQuery(b'p', InputEnd::St),
            Rec::GroundTimer(false),
            Rec::GroundTimer(true),
            Rec::StyleChanged(true),
            Rec::GroundTimer(false),
            Rec::GroundTimer(true),
            Rec::ColourQuery(ColourQueryKind::Background, InputEnd::Bel),
            Rec::GroundTimer(false),
            Rec::GroundTimer(true),
            Rec::Request(InputRequestKind::Palette { idx: 1 }, InputEnd::Bel),
            Rec::GroundTimer(false),
            Rec::GroundTimer(true),
            Rec::Request(InputRequestKind::Palette { idx: 1 }, InputEnd::Bel),
            Rec::GroundTimer(false),
            Rec::GroundTimer(true),
            Rec::ColourQuery(ColourQueryKind::Foreground, InputEnd::Bel),
            Rec::GroundTimer(false),
        ]
    );
    assert_eq!(f.palette.bg, Colour::rgb(255, 0, 0));
    assert_eq!(
        f.palette.get(Colour(2 | ColourFlags::_256.bits() as i32)),
        Some(Colour::rgb(0, 0, 255))
    );
    let rec = f.run(b"\x1b]4;2;?\x1b\\");
    assert!(rec.contains(&Rec::Reply(b"\x1b]4;2;rgb:0000/0000/ffff\x1b\\".to_vec())));
}

#[test]
fn no_pane_blocks_server_effects() {
    let mut f = Fixture::new(10, 4);
    f.policy = InputPolicy::screen_only();
    f.policy.allow_rename = true;
    let rec = f.run(b"\x07\x1b]0;t\x07\x1bkname\x1b\\\x1b[?996n\x1b]133;A\x07\x1b[31mX");
    assert!(rec.iter().all(|r| matches!(r, Rec::GroundTimer(_))));
    assert_eq!(f.ictx.cell().fg, Colour(1));
    assert!(
        f.screen
            .grid
            .get_line(0)
            .flags
            .contains(GridLineFlags::START_PROMPT)
    );
    assert!(f.screen.title.is_empty());
}

#[test]
fn rename_and_passthrough() {
    let mut f = Fixture::new(10, 4);
    let rec = f.run(b"\x1bkname\x1b\\");
    assert!(!rec.iter().any(|r| matches!(r, Rec::Rename(_))));
    f.policy.allow_rename = true;
    let rec = f.run(b"\x1bkname\x1b\\\x1bk\x1b\\");
    assert!(rec.contains(&Rec::Rename(Some(b"name".to_vec()))));
    assert!(rec.contains(&Rec::Rename(None)));
    f.policy.allow_passthrough = Passthrough::On;
    f.run(b"\x1bPtmux;\x1b\x1b[1m\x1b\\");
    f.run(b"\x1bP$q q\x1b\\");
    let rec = f.run(b"\x1bP$q q\x1b\\\x1bP$qm\x1b\\");
    assert_eq!(
        rec.iter()
            .filter(|r| matches!(r, Rec::Reply(_)))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            Rec::Reply(b"\x1bP1$r q0 q\x1b\\".to_vec()),
            Rec::Reply(b"\x1bP0$r\x1b\\".to_vec()),
        ]
    );
}

#[test]
fn tab_cells_and_rep() {
    let mut f = Fixture::new(20, 4);
    f.run(b"\tX");
    assert_eq!(f.screen.cx, 9);
    let gc = f.screen.grid.get_cell(0, 0);
    assert!(gc.flags.contains(rmux_emu::cell::GridCellFlags::TAB));
    assert_eq!(gc.data.width, 8);
    f.run(b"\r\x1b[2K    Q\rX\tY");
    assert!(
        !f.screen
            .grid
            .get_cell(1, 0)
            .flags
            .contains(rmux_emu::cell::GridCellFlags::TAB)
    );
    assert_eq!(f.text(0), "X   Q   Y");
    f.run(b"\r\nA\x1b[3b\x1b[1m\x1b[2b\x1b[H\x1b[2b");
    assert_eq!(f.text(1), "AAAA");
    f.run(b"\x1b[2;1H\x1b[K\x1b[31m\xe4\xb8\xad\x1b[2b");
    assert_eq!(f.text(1), "中中中");
}

#[test]
fn byte_split_matches_whole_stream() {
    let stream: &[u8] = b"\x1b]0;ti\x1b[1;31mAB\x1b[?1049h\x1b[2;3Hx\x1b[?1049l\xe4\xb8\xad\x1b]4;1;?;3;#010203\x07\x1b[?2031h\x1b[0m\x1bP$q q\x1b\\\x1b]133;D;3\x07\x1b[?2031l";
    let mut whole = Fixture::new(12, 5);
    let expected = whole.run(stream);
    let whole_dump = whole.dump();
    let mut split = Fixture::new(12, 5);
    let mut got = Vec::new();
    for b in stream {
        got.extend(split.run(&[*b]));
    }
    assert_eq!(got, expected);
    assert_eq!(split.dump(), whole_dump);
    assert_eq!(split.ictx.cell(), whole.ictx.cell());
}

#[test]
fn parse_step_consumed_counts() {
    let mut f = Fixture::new(10, 4);
    let mut tty = ScreenOnlySink;
    let mut sw = ScreenWriteCtx::start(
        &mut f.screen,
        &mut tty,
        ScreenWritePolicy {
            pane_backed: true,
            ..ScreenWritePolicy::default()
        },
        &mut f.registry,
        #[cfg(feature = "sixel")]
        Some(&mut f.images),
    );
    let bytes = b"ab\x07cd\x1b[?2031;2031h";
    let step = f.ictx.parse_step(&mut sw, None, &f.policy, bytes);
    assert_eq!(
        step,
        InputStep::Effect {
            consumed: 3,
            effect: InputEffect::Bell
        }
    );
    let step = f.ictx.parse_step(&mut sw, None, &f.policy, &bytes[3..]);
    assert_eq!(
        step,
        InputStep::Effect {
            consumed: 15,
            effect: InputEffect::ThemeUpdatesEnabled
        }
    );
    let step = f.ictx.parse_step(&mut sw, None, &f.policy, &bytes[18..]);
    assert_eq!(
        step,
        InputStep::Effect {
            consumed: 0,
            effect: InputEffect::ThemeUpdatesEnabled
        }
    );
    let step = f.ictx.parse_step(&mut sw, None, &f.policy, &bytes[18..]);
    assert_eq!(step, InputStep::Complete { consumed: 0 });
    sw.finish();
    assert_eq!(f.text(0), "abcd");
}

#[test]
fn null_sink_and_default_cell() {
    let mut f = Fixture::new(10, 4);
    f.feed(&mut NullSink, b"\x1b[1mZ");
    assert_eq!(f.text(0), "Z");
    assert_eq!(f.ictx.cell().attr, GridAttributes::BRIGHT);
    assert_ne!(f.ictx.cell(), &GridCell::default());
}

#[test]
fn combining_marks_join_previous_cell() {
    let mut f = Fixture::new(20, 4);
    f.run("e\u{301}a\u{308}\u{301}x".as_bytes());
    assert_eq!(f.screen.cx, 3);
    assert_eq!(f.text(0), "e\u{301}a\u{308}\u{301}x");
    let mut g = Fixture::new(20, 4);
    g.run("👨\u{200d}👩\u{200d}👧 ❤\u{fe0f} 👍🏽".as_bytes());
    assert_eq!(g.screen.cx, 2 + 1 + 2 + 1 + 2);
}

/// The pinned C crashes in `grid_move_cells` when insert mode pushes a tab
/// cell wider than the remaining columns past the margin; rmux keeps normal
/// behavior (crash-path policy) and must not panic.
#[test]
fn insert_mode_tab_cells_past_margin_do_not_panic() {
    for width in [10u32, 16, 17, 24, 80] {
        let mut f = Fixture::new(width, 3);
        f.run(b"\x1b[8G\t\x1b[1;1H\x1b[4h");
        for _ in 0..width + 4 {
            f.run(b"x");
        }
        f.run(b"\r\n\x1b[4h\t\tA\tB\x1b[4l");
        f.run(b"\x1b[3;1H\x1b[4h\x1b[2G\t\x1b[1G\tY\t\tZ");
        assert!(f.screen.cx <= width);
        assert!(f.screen.cy < 3);
        for x in 0..width {
            let _ = f.screen.grid.get_cell(x, 0);
        }
    }
    let mut g = Fixture::new(12, 2);
    g.run(b"\x1b[4h\x1b[3G\t\x1b[1GQ\x1b[2GR\x1b[1G\x1b[4@S\t");
    assert!(g.screen.cx <= 12);
    let dump = capture_pane(&g.screen, &g.registry);
    assert!(!dump.is_empty());
}

#[test]
fn winops_pixel_products_wrap_as_unsigned_c() {
    let mut f = Fixture::new(10, 4);
    f.policy.pixels = Some((u32::MAX, u32::MAX));
    let replies: Vec<_> = f
        .run(b"\x1b[14;15;16;18;19t")
        .into_iter()
        .filter(|r| matches!(r, Rec::Reply(_)))
        .collect();
    assert_eq!(
        replies,
        vec![
            Rec::Reply(b"\x1b[4;4294967292;4294967286t".to_vec()),
            Rec::Reply(b"\x1b[5;4294967292;4294967286t".to_vec()),
            Rec::Reply(b"\x1b[6;4294967295;4294967295t".to_vec()),
            Rec::Reply(b"\x1b[8;4;10t".to_vec()),
            Rec::Reply(b"\x1b[9;4;10t".to_vec()),
        ]
    );
}

#[test]
fn query_and_command_end_barriers_precede_later_mutations() {
    let mut f = Fixture::new(10, 4);
    let mut tty = ScreenOnlySink;
    let mut sw = ScreenWriteCtx::start(
        &mut f.screen,
        &mut tty,
        ScreenWritePolicy {
            pane_backed: true,
            ..ScreenWritePolicy::default()
        },
        &mut f.registry,
        #[cfg(feature = "sixel")]
        Some(&mut f.images),
    );
    let bytes = b"\x1b]11;red\x07\x1b]11;?\x07\x1b]11;blue\x07\x1b]133;C\x07\x1b]133;D;7\x07";
    let mut offset = 0;
    let mut saw_query = false;
    let mut saw_end = false;
    loop {
        match f
            .ictx
            .parse_step(&mut sw, Some(&mut f.palette), &f.policy, &bytes[offset..])
        {
            InputStep::Complete { consumed } => {
                offset += consumed;
                break;
            }
            InputStep::Effect { consumed, effect } => {
                offset += consumed;
                match effect {
                    InputEffect::ColourQuery {
                        which: ColourQueryKind::Background,
                        ..
                    } => {
                        assert_eq!(f.palette.bg, Colour::rgb(255, 0, 0));
                        saw_query = true;
                    }
                    InputEffect::Osc133(Osc133Event::CommandStarted) => {
                        assert!(
                            sw.screen
                                .grid
                                .get_line(0)
                                .flags
                                .contains(GridLineFlags::START_OUTPUT)
                        );
                    }
                    InputEffect::Osc133(Osc133Event::CommandFinished { status: 7 }) => {
                        assert!(
                            !sw.screen
                                .grid
                                .get_line(0)
                                .flags
                                .contains(GridLineFlags::END_OUTPUT)
                        );
                        saw_end = true;
                    }
                    _ => {}
                }
            }
        }
    }
    assert_eq!(offset, bytes.len());
    assert!(saw_query && saw_end);
    assert_eq!(f.palette.bg, Colour::rgb(0, 0, 255));
    assert!(
        sw.screen
            .grid
            .get_line(0)
            .flags
            .contains(GridLineFlags::END_OUTPUT)
    );
    sw.finish();
}

#[test]
fn tsp_every_byte_split_preserves_payload_and_legacy_title() {
    let payload = "tsp;f;c=abc;{\"text\":\"中😀;a=b;\\u001b\u{009c}\"}".as_bytes();
    let mut bytes = b"\x1b_".to_vec();
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\x1b\\\x1b[cZ");
    for split in 0..=bytes.len() {
        let mut f = Fixture::new(20, 4);
        f.screen.title = b"original".to_vec();
        f.policy.buffer_limit = 1;
        let mut effects = f.run(&bytes[..split]);
        effects.extend(f.run(&bytes[split..]));
        let messages: Vec<_> = effects
            .iter()
            .filter(|effect| matches!(effect, Rec::TspMessage(_) | Rec::Reply(_)))
            .collect();
        assert_eq!(messages.len(), 2, "split {split}");
        assert_eq!(messages[0], &Rec::TspMessage(payload.to_vec()));
        assert!(matches!(messages[1], Rec::Reply(_)));
        assert_eq!(f.screen.title, b"original");
        assert_eq!(f.text(0), "Z");
    }
    for title in [
        b"tspx".as_slice(),
        b"TSP;q;{}",
        b"prefix tsp;q;{}",
        b"ts\x07p;q;{}",
    ] {
        let mut f = Fixture::new(20, 4);
        let mut bytes = b"\x1b_".to_vec();
        bytes.extend_from_slice(title);
        bytes.extend_from_slice(b"\x1b\\");
        let effects = f.run(&bytes);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Rec::TitleChanged(_)))
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Rec::TspMessage(_)))
        );
    }
    let mut f = Fixture::new(20, 4);
    assert!(
        f.run(b"\x1b_legacy\x1b[31mX")
            .contains(&Rec::TitleChanged(b"legacy".to_vec()))
    );
    assert_eq!(f.text(0), "X");
    assert_eq!(f.ictx.cell().fg, Colour(1));
}

#[test]
fn tsp_cancel_malformed_st_and_timeout_discard_without_leakage() {
    for interruption in [b"\x1b[31m".as_slice(), b"\x18", b"\x1a", b"\x1b\x1bQ"] {
        let mut bytes = b"\x1b_tsp;q;{\"q\":\"hello\"".to_vec();
        bytes.extend_from_slice(interruption);
        bytes.extend_from_slice(b"JSON-tail}\x1b\\Z");
        for split in 0..=bytes.len() {
            let mut f = Fixture::new(20, 4);
            f.screen.title = b"keep".to_vec();
            let mut effects = f.run(&bytes[..split]);
            effects.extend(f.run(&bytes[split..]));
            assert!(
                !effects
                    .iter()
                    .any(|effect| matches!(effect, Rec::TspMessage(_) | Rec::TitleChanged(_)))
            );
            assert_eq!(f.screen.title, b"keep");
            assert_eq!(f.text(0), "Z", "split {split}");
            assert_eq!(f.ictx.cell().fg, Colour::DEFAULT);
        }
    }
    let mut f = Fixture::new(20, 4);
    f.run(b"\x1b_tsp;q;{incomplete");
    f.ictx.ground_timeout();
    let effects = f.run(b"tail}\x1b\\Z");
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Rec::TspMessage(_) | Rec::TitleChanged(_) | Rec::TerminalReset
    )));
    assert_eq!(f.text(0), "Z");
    for repair in [false, true] {
        let mut f = Fixture::new(20, 4);
        f.run(b"\x1b_tsp;q;{}\x1b");
        if repair {
            f.ictx.reset(None, &mut NullSink);
        } else {
            f.ictx.ground_timeout();
        }
        let effects = f.run(b"\\Z");
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Rec::TspMessage(_) | Rec::TerminalReset))
        );
        assert_eq!(f.text(0), "Z");
    }
}

#[test]
fn tsp_dedicated_bound_accepts_exact_limit_and_consumes_overflow() {
    use rmux_emu::input::TSP_APC_LIMIT;
    for length in [TSP_APC_LIMIT, TSP_APC_LIMIT + 1, TSP_APC_LIMIT * 2] {
        let mut f = Fixture::new(20, 4);
        f.policy.buffer_limit = 1;
        let mut bytes = b"\x1b_tsp;".to_vec();
        bytes.resize(length + 2, b'x');
        let mut effects = f.run(&bytes);
        assert!(f.ictx.pending().len() <= TSP_APC_LIMIT + 6);
        effects.extend(f.run(b"\x1b"));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Rec::TspMessage(_)))
        );
        effects.extend(f.run(b"\\Z"));
        let messages: Vec<_> = effects
            .iter()
            .filter_map(|effect| {
                if let Rec::TspMessage(payload) = effect {
                    Some(payload)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(messages.len(), usize::from(length == TSP_APC_LIMIT));
        if let Some(payload) = messages.first() {
            assert_eq!(payload.len(), TSP_APC_LIMIT);
        }
        assert!(f.screen.title.is_empty());
        assert_eq!(f.text(0), "Z");
    }
}

#[test]
fn tsp_borrowed_barrier_stops_at_st_before_da1_and_grid_output() {
    let mut f = Fixture::new(20, 4);
    let mut tty = ScreenOnlySink;
    let mut sw = ScreenWriteCtx::start(
        &mut f.screen,
        &mut tty,
        ScreenWritePolicy::default(),
        &mut f.registry,
        #[cfg(feature = "sixel")]
        Some(&mut f.images),
    );
    let bytes = b"\x1b_tsp;q;{\"q\":\"hello\"}\x1b\\\x1b[cZ";
    let end = bytes.len() - 4;
    assert_eq!(
        f.ictx.parse_step(&mut sw, None, &f.policy, bytes),
        InputStep::Effect {
            consumed: 2,
            effect: InputEffect::GroundTimer(true),
        }
    );
    assert_eq!(
        f.ictx.parse_step(&mut sw, None, &f.policy, &bytes[2..]),
        InputStep::Effect {
            consumed: end - 2,
            effect: InputEffect::TspMessage {
                payload: b"tsp;q;{\"q\":\"hello\"}",
            },
        }
    );
    assert_eq!(sw.screen.cx, 0);
    assert_eq!(
        f.ictx.parse_step(&mut sw, None, &f.policy, &bytes[end..]),
        InputStep::Effect {
            consumed: 0,
            effect: InputEffect::GroundTimer(false),
        }
    );
    match f.ictx.parse_step(&mut sw, None, &f.policy, &bytes[end..]) {
        InputStep::Effect {
            consumed: 3,
            effect: InputEffect::Reply(_),
        } => {}
        step => panic!("unexpected DA1 barrier {step:?}"),
    }
    assert_eq!(sw.screen.cx, 0);
    assert_eq!(
        f.ictx.parse_step(&mut sw, None, &f.policy, b"Z"),
        InputStep::Complete { consumed: 1 }
    );
    sw.finish();
    assert_eq!(f.text(0), "Z");
}

#[test]
fn terminal_reset_is_ris_only_and_discards_history_anchors() {
    use rmux_emu::grid::SurfaceAnchorId;
    let mut f = Fixture::new(20, 4);
    assert!(f.screen.grid.attach_surface_anchor(0, SurfaceAnchorId(4)));
    f.screen.grid.scroll_history(Colour::DEFAULT);
    let effects = f.run(b"\x1bcZ");
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Rec::TerminalReset))
            .count(),
        1
    );
    assert_eq!(
        f.screen.drain_surface_anchor_removals().collect::<Vec<_>>(),
        vec![SurfaceAnchorId(4)]
    );
    assert_eq!(f.text(0), "Z");
    let mut rec = Recorder::default();
    f.ictx.reset(None, &mut rec);
    f.ictx.ground_timeout();
    assert!(!rec.0.contains(&Rec::TerminalReset));
}
