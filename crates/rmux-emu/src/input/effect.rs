// Ported from tmux input.c and tmux.h @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

//! Server-visible parser effects (`input.c` points that touch the pane,
//! window, clients or options) and the per-batch option snapshot.

use crate::colour::Colour;
use rmux_util::bytes::ByteString;

/// `enum input_end_type` (`input.c:55-58`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputEnd {
    #[default]
    St,
    Bel,
}

impl InputEnd {
    /// The terminator bytes a reply echoes back (`input.c:2908-2911`).
    pub const fn bytes(self) -> &'static [u8] {
        match self {
            InputEnd::St => b"\x1b\\",
            InputEnd::Bel => b"\x07",
        }
    }
}

/// `enum input_request_type` (`tmux.h:1244-1248`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InputRequestType {
    Palette = 0,
    Clipboard = 1,
    Queue = 2,
}

impl TryFrom<i32> for InputRequestType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Palette),
            1 => Ok(Self::Clipboard),
            2 => Ok(Self::Queue),
            _ => Err(value),
        }
    }
}

/// `struct input_request_palette_data` (`tmux.h:1251-1254`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputRequestPaletteData {
    pub idx: i32,
    pub c: Colour,
}

/// `struct input_request_clipboard_data` (`tmux.h:1257-1261`); the length
/// is `buf.len()`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestClipboardData {
    pub buf: ByteString,
    pub clip: u8,
}

/// A decoded terminal reply delivered by the G08 key decoders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputReply {
    Palette(InputRequestPaletteData),
    Clipboard(InputRequestClipboardData),
}

/// The request a pane asks the server to send to a client
/// (`input.c:3609-3616`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputRequestKind {
    Palette { idx: u8 },
    Clipboard { clip: u8 },
}

/// Pane side of OSC 133 (`input.c:3285-3327`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Osc133Event {
    Prompt,
    CommandStarted,
    CommandFinished { status: u8 },
}

/// OSC 10 and 11 `?` queries that need pane state (`input.c:3074-3085,
/// 3124-3128`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColourQueryKind {
    Foreground,
    Background,
}

/// `allow-passthrough` (`options-table.c`): off, on, all.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Passthrough {
    #[default]
    Off,
    On,
    All,
}

/// `extended-keys`: off, on, always.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExtendedKeys {
    #[default]
    Off,
    On,
    Always,
}

/// `extended-keys-format`: csi-u (0) or xterm (1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExtendedKeysFormat {
    CsiU,
    #[default]
    Xterm,
}

/// `get-clipboard`: off, buffer, request, both.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GetClipboard {
    Off,
    #[default]
    Buffer,
    Request,
    Both,
}

/// Option and owner snapshot for one parse batch (spec 3.4). The server
/// refreshes it after every barrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputPolicy {
    pub allow_passthrough: Passthrough,
    pub allow_set_title: bool,
    pub allow_rename: bool,
    pub extended_keys: ExtendedKeys,
    pub cursor_style: i32,
    pub set_clipboard_on: bool,
    pub get_clipboard: GetClipboard,
    pub buffer_limit: usize,
    pub sixel: bool,
    pub pixels: Option<(u32, u32)>,
    pub has_pane: bool,
    pub writer_has_pane: bool,
    pub reset_extended_keys: bool,
}

/// `INPUT_BUF_DEFAULT_SIZE` (`tmux.h:3423`).
pub const INPUT_BUF_DEFAULT_SIZE: usize = 1_048_576;

impl Default for InputPolicy {
    /// tmux default options for a pane-backed parser with a pane-backed
    /// writer.
    fn default() -> Self {
        Self {
            allow_passthrough: Passthrough::Off,
            allow_set_title: true,
            allow_rename: false,
            extended_keys: ExtendedKeys::Off,
            cursor_style: 0,
            set_clipboard_on: false,
            get_clipboard: GetClipboard::Buffer,
            buffer_limit: INPUT_BUF_DEFAULT_SIZE,
            sixel: false,
            pixels: None,
            has_pane: true,
            writer_has_pane: true,
            reset_extended_keys: false,
        }
    }
}

impl InputPolicy {
    /// A parser without a pane (`input_init(NULL, ...)`, `window-copy.c:668`).
    pub fn screen_only() -> Self {
        Self {
            has_pane: false,
            writer_has_pane: false,
            ..Self::default()
        }
    }
}

/// One server-visible effect at its C side-effect point (spec 4.3). Borrowed
/// bytes live in parser scratch and stay valid until the parser resumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEffect<'a> {
    /// `input_reply(ictx, 1, ...)`: queue behind pending requests, else write.
    Reply(&'a [u8]),
    /// `input_add_request` (`input.c:3576-3622`).
    Request {
        kind: InputRequestKind,
        end: InputEnd,
    },
    /// OSC 52 `?` with `get-clipboard` buffer or request (`input.c:3347-3360`).
    ClipboardQuery {
        clip: u8,
        end: InputEnd,
    },
    /// OSC 52 set after the fresh-writer selection (`input.c:3432-3433`).
    ClipboardReceived {
        clip: &'a [u8],
        data: &'a [u8],
    },
    /// OSC 10 `?` and OSC 11 `?` (`input.c:3074-3085,3124-3128`).
    ColourQuery {
        which: ColourQueryKind,
        end: InputEnd,
    },
    /// `CSI ? 996 n` (`input.c:3715-3737`).
    ThemeReport,
    /// SM 2031 (`input.c:2084-2087`).
    ThemeUpdatesEnabled,
    /// RM 2031 (`input.c:1984-1985`).
    ThemeUpdatesDisabled,
    /// BEL (`input.c:1321-1324`).
    Bell,
    /// Raw title after `screen_set_title` succeeds (`input.c:2726,2804`).
    TitleChanged(&'a [u8]),
    /// WINOPS 23 (`input.c:2206-2208`); the stored title.
    TitlePopped(&'a [u8]),
    /// OSC 7 (`input.c:2736-2737`).
    PathChanged,
    /// `ESC k ... ST` (`input.c:2841-2852`); `None` is the empty name.
    Rename(Option<&'a [u8]>),
    /// OSC 9;4 (`input.c:3020-3023`).
    ProgressChanged,
    /// `PANE_STYLECHANGED`, plus `PANE_THEMECHANGED` (`input.c:3096,3139`).
    StyleChanged {
        theme: bool,
    },
    /// SM 2026 after base `MODE_SYNC` is set (`screen-write.c:1071-1077`).
    SyncStart,
    /// RM 2026 after the G04 flush (`screen-write.c:1102-1108`).
    SyncEnd,
    Osc133(Osc133Event),
    /// Arm the five-second ground timer, or cancel it.
    GroundTimer(bool),
    /// A successful pane-backed alternate switch (`screen-write.c:3317,3349`).
    AlternateChanged {
        entering: bool,
    },
}

/// Receives effects synchronously, in C order, before the parser resumes.
pub trait InputSink {
    fn effect(&mut self, effect: InputEffect<'_>);
    /// `input_send_reply`: a direct pane write.
    fn reply(&mut self, bytes: &[u8]);
}

/// Drops every effect and reply; for pure screen tests only.
#[derive(Debug, Default)]
pub struct NullSink;

impl InputSink for NullSink {
    fn effect(&mut self, _: InputEffect<'_>) {}
    fn reply(&mut self, _: &[u8]) {}
}

/// The result of one `parse_step`.
#[derive(Debug, PartialEq, Eq)]
pub enum InputStep<'a> {
    /// Every byte and every pending handler substep is done.
    Complete { consumed: usize },
    /// Stopped at one effect; apply it, then call `parse_step` again with the
    /// bytes after `consumed`.
    Effect {
        consumed: usize,
        effect: InputEffect<'a>,
    },
}
