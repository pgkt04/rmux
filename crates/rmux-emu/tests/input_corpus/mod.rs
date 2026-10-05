//! Shared harness for the G05 end-to-end tests: a standalone pane-backed
//! emulator (`InputCtx` + G04 writer) and a dictionary of VT sequence tokens
//! covering every parser family for random streams.

#![allow(dead_code)]

use rmux_emu::colour::ColourPalette;
use rmux_emu::hyperlinks::HyperlinkRegistry;
#[cfg(feature = "sixel")]
use rmux_emu::image::ImageRegistry;
use rmux_emu::input::dump;
use rmux_emu::input::{InputCtx, InputPolicy, NullSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};

use super::common::Rng;

/// One pane as `-f /dev/null` tmux configures it: `allow-passthrough off`,
/// `allow-rename off`, `allow-set-title on`, `alternate-screen on`,
/// `scroll-on-clear on` (`options-table.c:1693-1696`), `extended-keys off`.
pub struct Emu {
    pub registry: HyperlinkRegistry,
    #[cfg(feature = "sixel")]
    pub images: ImageRegistry,
    pub screen: Screen,
    pub palette: ColourPalette,
    pub ictx: InputCtx,
    pub policy: InputPolicy,
}

impl Emu {
    pub fn new(sx: u32, sy: u32, hlimit: u32) -> Emu {
        let mut registry = HyperlinkRegistry::new();
        let screen = Screen::new(sx, sy, hlimit, ScreenResetPolicy::default(), &mut registry)
            .expect("screen");
        #[cfg(feature = "sixel")]
        let (screen, images) = {
            let mut screen = screen;
            let mut images = ImageRegistry::default();
            screen.bind_images(&mut images);
            (screen, images)
        };
        Emu {
            registry,
            #[cfg(feature = "sixel")]
            images,
            screen,
            palette: ColourPalette::new(),
            ictx: InputCtx::new(),
            policy: InputPolicy::default(),
        }
    }

    fn write_policy() -> ScreenWritePolicy {
        ScreenWritePolicy {
            pane_backed: true,
            ..ScreenWritePolicy::default()
        }
    }

    /// One `input_parse_buffer` call: start a writer, parse, finish.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut tty = ScreenOnlySink;
        let mut sw = ScreenWriteCtx::start(
            &mut self.screen,
            &mut tty,
            Self::write_policy(),
            &mut self.registry,
            #[cfg(feature = "sixel")]
            Some(&mut self.images),
        );
        self.ictx.parse(
            &mut sw,
            Some(&mut self.palette),
            &self.policy,
            &mut NullSink,
            bytes,
        );
        sw.finish();
    }

    /// `capture-pane -p -e -N -S -` (depends on write batching).
    pub fn capture(&self) -> Vec<u8> {
        dump::capture_pane(&self.screen, &self.registry)
    }

    /// `capture-pane -p -e -N -T -S -`.
    pub fn capture_used(&self) -> Vec<u8> {
        dump::capture_pane_used(&self.screen, &self.registry)
    }

    /// `capture-pane -p -F -N -T -S -`.
    pub fn flags(&self) -> Vec<u8> {
        dump::capture_pane_flags(&self.screen, &self.registry)
    }

    /// `display -p` of [`dump::STATE_FORMAT`].
    pub fn state(&self) -> String {
        dump::state_line(&self.screen)
    }
}

/// Byte sequences from every parser family (`input.c` tables and string
/// states), for building random streams.
pub const TOKENS: &[&[u8]] = &[
    // C0.
    b"\x07",
    b"\x08",
    b"\t",
    b"\n",
    b"\x0b",
    b"\x0c",
    b"\r",
    b"\x0e",
    b"\x0f",
    b"\x00",
    b"\x7f",
    b"\x18",
    b"\x1a",
    // ESC.
    b"\x1bc",
    b"\x1bD",
    b"\x1bE",
    b"\x1bH",
    b"\x1bM",
    b"\x1b=",
    b"\x1b>",
    b"\x1b7",
    b"\x1b8",
    b"\x1b#8",
    b"\x1b(0",
    b"\x1b(B",
    b"\x1b)0",
    b"\x1b)B",
    b"\x1b\\",
    b"\x1bZ",
    b"\x1b",
    // CSI cursor and editing.
    b"\x1b[3@",
    b"\x1b[A",
    b"\x1b[2B",
    b"\x1b[5C",
    b"\x1b[D",
    b"\x1b[2E",
    b"\x1b[F",
    b"\x1b[10G",
    b"\x1b[5;10H",
    b"\x1b[100;200H",
    b"\x1b[0;0f",
    b"\x1b[J",
    b"\x1b[1J",
    b"\x1b[2J",
    b"\x1b[3J",
    b"\x1b[K",
    b"\x1b[1K",
    b"\x1b[2K",
    b"\x1b[2L",
    b"\x1b[M",
    b"\x1b[3P",
    b"\x1b[2S",
    b"\x1b[T",
    b"\x1b[4X",
    b"\x1b[Z",
    b"\x1b[2Z",
    b"\x1b[5b",
    b"\x1b[7d",
    b"\x1b[g",
    b"\x1b[3g",
    b"\x1b[5;15r",
    b"\x1b[r",
    b"\x1b[s",
    b"\x1b[u",
    b"\x1b[2 q",
    b"\x1b[0 q",
    b"\x1b[c",
    b"\x1b[>c",
    b"\x1b[5n",
    b"\x1b[6n",
    b"\x1b[?4$p",
    b"\x1b[?7$p",
    b"\x1b[>4;2m",
    b"\x1b[>4m",
    b"\x1b[?9999z",
    b"\x1b[18t",
    b"\x1b[22;0t",
    b"\x1b[23;0t",
    b"\x1b[22;2t",
    b"\x1b[23;2t",
    b"\x1b[8;24;80t",
    // SM/RM.
    b"\x1b[4h",
    b"\x1b[4l",
    b"\x1b[34h",
    b"\x1b[34l",
    b"\x1b[?1h",
    b"\x1b[?1l",
    b"\x1b[?3h",
    b"\x1b[?6h",
    b"\x1b[?6l",
    b"\x1b[?7h",
    b"\x1b[?7l",
    b"\x1b[?12h",
    b"\x1b[?12l",
    b"\x1b[?25h",
    b"\x1b[?25l",
    b"\x1b[?1000h",
    b"\x1b[?1000l",
    b"\x1b[?1001l",
    b"\x1b[?1002h",
    b"\x1b[?1003h",
    b"\x1b[?1004h",
    b"\x1b[?1004l",
    b"\x1b[?1005h",
    b"\x1b[?1005l",
    b"\x1b[?1006h",
    b"\x1b[?1006l",
    b"\x1b[?2004h",
    b"\x1b[?2004l",
    b"\x1b[?47h",
    b"\x1b[?47l",
    b"\x1b[?1047h",
    b"\x1b[?1047l",
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[?2026h",
    b"\x1b[?2026l",
    b"\x1b[?1;1004;2004h",
    // SGR.
    b"\x1b[m",
    b"\x1b[0m",
    b"\x1b[1m",
    b"\x1b[2m",
    b"\x1b[3m",
    b"\x1b[4m",
    b"\x1b[5m",
    b"\x1b[6m",
    b"\x1b[7m",
    b"\x1b[8m",
    b"\x1b[9m",
    b"\x1b[21m",
    b"\x1b[22m",
    b"\x1b[23m",
    b"\x1b[24m",
    b"\x1b[25m",
    b"\x1b[27m",
    b"\x1b[28m",
    b"\x1b[29m",
    b"\x1b[31m",
    b"\x1b[37m",
    b"\x1b[39m",
    b"\x1b[42m",
    b"\x1b[49m",
    b"\x1b[53m",
    b"\x1b[55m",
    b"\x1b[59m",
    b"\x1b[91m",
    b"\x1b[97m",
    b"\x1b[102m",
    b"\x1b[107m",
    b"\x1b[38;5;123m",
    b"\x1b[48;5;200m",
    b"\x1b[58;5;17m",
    b"\x1b[38;2;1;2;3m",
    b"\x1b[48;2;255;128;0m",
    b"\x1b[58;2;9;8;7m",
    b"\x1b[38;2;1;2m",
    b"\x1b[38;5m",
    b"\x1b[38;2;300;1;1m",
    b"\x1b[38m",
    b"\x1b[48;9m",
    b"\x1b[4:3m",
    b"\x1b[4:0m",
    b"\x1b[4:1:2m",
    b"\x1b[38:2::255:0:0m",
    b"\x1b[38:2:255:0:0m",
    b"\x1b[38:5:123m",
    b"\x1b[48:5:200:9m",
    b"\x1b[58:2::1:2:3m",
    b"\x1b[1:2:3:4:5:6:7:8m",
    b"\x1b[1;4;31;42m",
    b"\x1b[;1m",
    b"\x1b[1;;4m",
    // Discard limits and malformed CSI.
    b"\x1b[!!!!H",
    b"\x1b[    \x18",
    b"\x1b[1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1H",
    b"\x1b[1111111111111111111111111111111111111111111111111111111111111111111111H",
    b"\x1b[2\x18",
    b"\x1b[2\x1a",
    b"\x1b[2\nC",
    b"\x1b[\x075C",
    b"\x1b\n[3C",
    b"\x1b[?1;2$p",
    b"\x1b[>",
    b"\x1b[?",
    b"\x1b[1;",
    // DCS.
    b"\x1bPqab\x1b\x1bcd\x1b\\",
    b"\x1bP$q q\x1b\\",
    b"\x1bP$qBAD\x1b\\",
    b"\x1bPtmux;\x1b\x1b[31m\x1b\\",
    b"\x1bP1;2;3q\x07\x18\x00x\x1b\\",
    b"\x1bP:junk\x1b\\",
    b"\x1bPq",
    b"\x1bP$q",
    // OSC.
    b"\x1b]0;title\x07",
    b"\x1b]2;title two\x1b\\",
    b"\x1b]0;t\x1b[H",
    b"\x1b]7;file://h/tmp\x07",
    b"\x1b]8;;http://example.com\x07",
    b"\x1b]8;id=a;http://example.com/a\x07",
    b"\x1b]8;;\x07",
    b"\x1b]8;id=a:id=b;http://bad\x07",
    b"\x1b]8;id=no-separator\x07",
    b"\x1b]9;4;1;50\x07",
    b"\x1b]9;4;0\x07",
    b"\x1b]9;4;5;200\x07",
    b"\x1b]10;red\x07",
    b"\x1b]11;#102030\x07",
    b"\x1b]12;blue\x07",
    b"\x1b]104\x07",
    b"\x1b]104;1;2\x07",
    b"\x1b]110\x07",
    b"\x1b]111\x07",
    b"\x1b]112\x07",
    b"\x1b]4;1;red\x07",
    b"\x1b]4;999;red\x07",
    b"\x1b]133;A\x07",
    b"\x1b]133;B\x07",
    b"\x1b]133;C\x07",
    b"\x1b]133;D;7\x07",
    b"\x1b]133;D;-1\x07",
    b"\x1b]133;P;k=s\x07",
    b"\x1b]999;bad\x07",
    b"\x1b]abc\x07",
    b"\x1b]\x07",
    b"\x1b]2;",
    b"\x1b]0;caf\xc3\xa9\x07",
    b"\x1b]0;bad\xff\x07",
    // APC, rename, SOS, PM.
    b"\x1b_apc title\x1b\\",
    b"\x1bkname\x1b\\",
    b"\x1bXsos junk\x1b\\",
    b"\x1b^pm junk\x1b\\",
    b"\x1b_",
    b"\x1bk",
    // UTF-8.
    "日本語".as_bytes(),
    "é".as_bytes(),
    "e\u{301}".as_bytes(),
    "👨\u{200d}👩\u{200d}👧".as_bytes(),
    "❤\u{fe0f}".as_bytes(),
    "한".as_bytes(),
    b"\xf0\x80\x80\x80",
    b"\xed\xa0\x80",
    b"\x80",
    b"\xe6\x97",
    b"\xe6\x97\xa5",
    b"\xc3",
    b"\xf4\x90\x80\x80",
    // Printable runs.
    b"The quick brown fox jumps over the lazy dog. ",
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
    b"lqqqqk",
    b"x",
    b" ",
    b"  ",
    b"\\",
    b"#",
    b"~",
    b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
];

/// A stream of dictionary tokens and printable runs with roughly `len`
/// bytes.
pub fn dictionary_stream(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 128);
    while out.len() < len {
        match rng.below(4) {
            0 => {
                let n = rng.below(40) as usize + 1;
                for _ in 0..n {
                    out.push(b' ' + rng.below(95) as u8);
                }
            }
            _ => out.extend_from_slice(TOKENS[rng.below(TOKENS.len() as u64) as usize]),
        }
    }
    out
}
