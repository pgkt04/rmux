// Ported from tmux tty-keys.c @ 8f25579c
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

//! Keys from the outer terminal (`tty-keys.c`). `TtyKeyDecoder` owns the
//! key tree, the bracket-paste flag, the last mouse state and the escape
//! timer phase. `next` decodes one step from a borrowed input slice and
//! returns a `DecodeStep`; the server adapter applies the step (features,
//! replies, colours, size, focus, key dispatch), drains the consumed bytes,
//! rebuilds `KeyDecodeContext` and calls `next` again.

mod keyboard;
mod mouse;
mod reply;
pub(crate) mod scan;
pub mod tables;
mod tree;

use std::time::Duration;

use crate::term::{TtyCodeCode, TtyTerm};
use crate::tty::{TimerRequest, TtyFlags, TtyTimer};
use rmux_emu::colour::Colour;
use rmux_emu::input::InputRequestPaletteData;
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::key::{KeyCode, KeyFlags, KeyMasks, KeyModifiers, MouseEvent, SpecialKey};
use rmux_util::utf8::{self, Utf8Data, Utf8State};

pub use mouse::MouseLast;
pub use tree::{TtyKey, UNKNOWN};

/// `KEYC_MOUSE`: the unclassified mouse key the decoder emits.
pub const MOUSE: KeyCode = KeyCode(SpecialKey::MOUSE);

/// `tty->mouse_last_*` plus the paste and timer state the decoder owns.
#[derive(Clone, Debug, Default)]
pub struct TtyKeyDecoder {
    tree: tree::KeyTree,
    bracket_paste: bool,
    mouse_last: MouseLast,
    timer: TimerPhase,
}

/// `TTY_TIMER` and the libevent pending state (`tty-keys.c:983-989`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TimerPhase {
    /// `TTY_TIMER` clear.
    #[default]
    Idle,
    /// `TTY_TIMER` set and the timer pending.
    Waiting,
    /// `TTY_TIMER` set and the timer has fired: the next partial lookup
    /// runs with `expired`.
    Fired,
}

/// Immutable snapshot of the tty and client state `tty_keys_next` reads.
/// `TIMER` and `BRACKETPASTE` bits in `flags` are ignored: the decoder owns
/// them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyDecodeContext {
    pub flags: TtyFlags,
    /// `c->session != NULL`.
    pub has_session: bool,
    /// The `escape-time` option in milliseconds.
    pub escape_time_ms: u32,
    /// `tio.c_cc[VERASE]` unless it is `_POSIX_VDISABLE`.
    pub verase: Option<u8>,
    pub sx: u32,
    pub sy: u32,
    pub xpixel: u32,
    pub ypixel: u32,
    /// `!TAILQ_EMPTY(&c->input_requests)`.
    pub has_input_requests: bool,
}

/// One decoded key (G00 `DecodedKey`): the key, the mouse fields for
/// `KEYC_MOUSE`, and the exact consumed bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedKey<'a> {
    pub key: KeyCode,
    pub mouse: Option<MouseEvent>,
    pub raw: &'a [u8],
}

/// Internal parser result (`0`, `1`, `-1` in C).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Recognition<T> {
    Complete(usize, T),
    Partial,
    NoMatch,
}

/// Features a primary DA reply adds (`tty-keys.c:1527-1534`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DaFeatures {
    pub sixel: bool,
    pub margins: bool,
    pub rectfill: bool,
    pub clipboard: bool,
}

impl DaFeatures {
    /// The `tty_parse_client_features` names, in table order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        [
            (self.sixel, "sixel"),
            (self.margins, "margins"),
            (self.rectfill, "rectfill"),
            (self.clipboard, "clipboard"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
    }
}

/// A terminal discovery reply. Every variant updates features
/// (`tty_update_features`) and then sets its `have` flag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Discovery {
    /// `ESC [ ? ... c`: add `features`, set `HAVEDA`.
    PrimaryDa { features: DaFeatures },
    /// `ESC [ > ... c`: apply `tty_default_features(defaults)` if any, set
    /// `HAVEDA2`.
    SecondaryDa { defaults: Option<&'static str> },
    /// `ESC [ ? 2026 ; s $ y`: add `sync` when set (features update only
    /// then), set `HAVESYNC`.
    Sync { sync: bool },
    /// `ESC P > | text ST`: apply defaults if any, replace the client's
    /// terminal type with `term_type` when present, set `HAVEXDA`.
    ExtendedDa {
        defaults: Option<&'static str>,
        term_type: Option<ByteString>,
    },
}

impl Discovery {
    /// The flag this reply sets after its features update.
    pub fn have_flag(&self) -> TtyFlags {
        match self {
            Discovery::PrimaryDa { .. } => TtyFlags::HAVEDA,
            Discovery::SecondaryDa { .. } => TtyFlags::HAVEDA2,
            Discovery::Sync { .. } => TtyFlags::HAVESYNC,
            Discovery::ExtendedDa { .. } => TtyFlags::HAVEXDA,
        }
    }

    /// Whether `tty_update_features` runs (`tty-keys.c:1540,1586,1667,1744`).
    pub fn updates_features(&self) -> bool {
        !matches!(self, Discovery::Sync { sync: false })
    }
}

/// An OSC 52 reply (`tty-keys.c:1445-1457`). `data` is `None` when the
/// response was malformed and only consumed. With data: route the reply to a
/// pending pane clipboard request first; then, if `query` (`OSC52QUERY` was
/// set), create a paste buffer, cancel the clipboard timer and clear the flag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardReply {
    pub clip: u8,
    pub data: Option<ByteString>,
    pub query: bool,
}

/// An OSC 4 reply: `None` when the colour was invalid (consumed only).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaletteReply {
    pub reply: Option<InputRequestPaletteData>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColourTarget {
    Foreground,
    Background,
}

impl ColourTarget {
    /// The wait flag a valid colour clears.
    pub fn wait_flag(self) -> TtyFlags {
        match self {
            ColourTarget::Foreground => TtyFlags::WAITFG,
            ColourTarget::Background => TtyFlags::WAITBG,
        }
    }
}

/// An OSC 10/11 reply. With a colour: set the tty foreground or background
/// and clear the matching wait flag. Complete replies also notify the
/// session that its theme changed, after updating client theme colours if the
/// background changed (`tty-keys.c:830-836`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColourReply {
    pub target: ColourTarget,
    pub colour: Option<Colour>,
}

/// A window-size reply: call `tty_set_size(sx, sy, xpixel, ypixel)`; then
/// invalidate and clear `WINSIZEQUERY` for the pixel form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SizeReply {
    pub sx: u32,
    pub sy: u32,
    pub xpixel: u32,
    pub ypixel: u32,
    pub invalidate: bool,
    pub clear_query: bool,
}

/// What one complete step carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TtyInput<'a> {
    /// A key event to dispatch. `FocusIn`/`FocusOut` first apply focus:
    /// out clears focus, updates window focus, fires `client-focus-out`; in
    /// sets focus, fires `client-focus-in`, updates window focus
    /// (`tty-keys.c:1031-1039`).
    Key(DecodedKey<'a>),
    Clipboard(ClipboardReply),
    Palette(PaletteReply),
    Colour(ColourReply),
    Discovery(Discovery),
    Size(SizeReply),
}

/// The result of one `tty_keys_next` call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeStep<'a> {
    /// No input (`return 0` at `tty-keys.c:760`).
    Empty,
    /// More bytes are needed. `timer` is `Some` when the key timer must be
    /// armed now; `None` keeps a running or fired timer. `theme_changed`
    /// reports the session notification for a partial colour reply
    /// (`tty-keys.c:840-843`).
    Partial {
        timer: Option<TimerRequest>,
        theme_changed: bool,
    },
    /// `consumed` bytes produced `input`. Notify the session theme change
    /// first if `theme_changed` (a partial colour reply that the fired
    /// timer then decoded as keys, `tty-keys.c:840-843,986-987`), apply the
    /// input, cancel the key timer if `cancel_timer`, then drain.
    Complete {
        consumed: usize,
        input: TtyInput<'a>,
        cancel_timer: bool,
        theme_changed: bool,
    },
    /// Drain `consumed` bytes without an event: no session (cancels the
    /// timer) or an unwanted mouse report (does not).
    Discard { consumed: usize, cancel_timer: bool },
}

/// `tty_keys_partial_paste_end` (`tty-keys.c:606-615`).
fn partial_paste_end(buf: &[u8]) -> bool {
    const PASTE_END: &[u8] = b"\x1b[201~";
    !buf.is_empty() && buf.len() < PASTE_END.len() && PASTE_END.starts_with(buf)
}

/// Outcome of `tty_keys_next1`.
enum Next1 {
    Found(KeyCode, usize),
    Partial,
    NoMatch,
}

impl TtyKeyDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// `tty_keys_build` (`tty-keys.c:490-541`): xterm templates with
    /// modifiers 2-9, raw keys, terminfo keys, then user keys in the order
    /// given (the caller supplies them by ascending index; entries above
    /// `KEYC_NUSER` are never looked up). Paste, mouse and timer state are
    /// untouched.
    pub fn rebuild<'a>(
        &mut self,
        term: &TtyTerm,
        user_keys: impl Iterator<Item = (u32, &'a [u8])>,
    ) {
        self.rebuild_with(|code| term.string(code), user_keys);
    }

    /// `rebuild` with an explicit capability lookup (`tty_term_string`).
    pub fn rebuild_with<'a, 'b>(
        &mut self,
        caps: impl Fn(TtyCodeCode) -> &'b [u8],
        user_keys: impl Iterator<Item = (u32, &'a [u8])>,
    ) {
        self.tree.clear();
        let mut copy = [0u8; 16];
        for &(template, key) in tables::XTERM_KEYS {
            for (j, modifiers) in tables::XTERM_MODIFIERS.iter().enumerate().skip(2) {
                let n = template.len().min(copy.len() - 1);
                copy[..n].copy_from_slice(&template[..n]);
                if let Some(at) = copy[..n].iter().position(|&c| c == b'_') {
                    copy[at] = b'0' + j as u8;
                }
                self.tree.add(&copy[..n], KeyCode(key.0 | modifiers));
            }
        }
        for &(s, key) in tables::RAW_KEYS {
            if !s.is_empty() {
                self.tree.add(s, key);
            }
        }
        for &(code, key) in tables::CODE_KEYS {
            let s = cstr(caps(code));
            if !s.is_empty() {
                self.tree.add(s, key);
            }
        }
        for (i, s) in user_keys {
            if i <= KeyCode::NUSER {
                self.tree
                    .add(cstr(s), KeyCode(SpecialKey::USER + u64::from(i)));
            }
        }
    }

    /// `tty_keys_free`: release the tree. Timer state is reset too, as
    /// `tty_close` cancels the key timer before freeing (`tty.c:523-536`).
    pub fn clear(&mut self) {
        self.tree = tree::KeyTree::default();
        self.timer = TimerPhase::Idle;
    }

    pub fn nodes(&self) -> &[TtyKey] {
        self.tree.nodes()
    }

    /// `tty_keys_find` on the current tree: the node and matched length.
    pub fn find(&self, buf: &[u8]) -> (Option<&TtyKey>, usize) {
        let (index, size) = self.tree.find(buf);
        (index.map(|i| self.tree.node(i)), size)
    }

    /// `TTY_BRACKETPASTE`.
    pub fn bracket_paste(&self) -> bool {
        self.bracket_paste
    }

    pub fn mouse_last(&self) -> MouseLast {
        self.mouse_last
    }

    pub fn timer_phase(&self) -> TimerPhase {
        self.timer
    }

    /// The key timer fired: the next step decodes with `expired`
    /// (`tty_keys_callback`, `tty-keys.c:1073-1081`). The caller then runs
    /// `next` until it returns `Empty` or `Partial`.
    pub fn timer_fired(&mut self) {
        if self.timer == TimerPhase::Waiting {
            self.timer = TimerPhase::Fired;
        }
    }

    /// Cancel the key timer (`evtimer_del` plus clearing `TTY_TIMER`).
    pub fn reset_timer(&mut self) {
        self.timer = TimerPhase::Idle;
    }

    /// `tty_keys_next1` (`tty-keys.c:618-673`): tree lookup, then UTF-8.
    fn next1(&mut self, buf: &[u8], expired: bool) -> Next1 {
        let (node, size) = self.tree.find(buf);
        if let Some(index) = node {
            let tk = self.tree.node(index);
            if tk.key != UNKNOWN {
                if tk.next.is_some() && !expired {
                    return Next1::Partial;
                }
                let key = tk.key;
                let onlykey = key.0 & KeyMasks::KEY;
                if onlykey == SpecialKey::PASTE_START {
                    self.bracket_paste = true;
                } else if onlykey == SpecialKey::PASTE_END {
                    self.bracket_paste = false;
                }
                return Next1::Found(key, size);
            }
        }

        let Ok(mut ud) = Utf8Data::open(buf[0]) else {
            return Next1::NoMatch;
        };
        let size = usize::from(ud.size);
        if buf.len() < size {
            return if expired {
                Next1::NoMatch
            } else {
                Next1::Partial
            };
        }
        let mut more = Utf8State::More;
        for &b in &buf[1..size] {
            more = ud.append(b);
        }
        if more != Utf8State::Done {
            return Next1::NoMatch;
        }
        match utf8::from_data(&ud) {
            (uc, Utf8State::Done) => Next1::Found(KeyCode(u64::from(uc.0)), size),
            _ => Next1::NoMatch,
        }
    }

    /// `first_key` through the fallback (`tty-keys.c:892-977`): `None` is a
    /// partial key (only possible while not expired).
    fn first_key(
        &mut self,
        buf: &[u8],
        ctx: &KeyDecodeContext,
        expired: bool,
    ) -> Option<(KeyCode, usize)> {
        match self.next1(buf, expired) {
            Next1::Found(key, size) => return Some((key, size)),
            Next1::Partial => return None,
            Next1::NoMatch => {}
        }
        if buf[0] == 0x1b && buf.len() > 1 {
            match self.next1(&buf[1..], expired) {
                Next1::Found(key, size) => {
                    if key.0 & KeyFlags::IMPLIED_META.0 != 0 {
                        // The xterm sequence already includes the Escape, so
                        // ESC ESC [1;3D is Escape then M-Left, not M-Left.
                        return Some((KeyCode(0x1b), 1));
                    }
                    return Some((KeyCode(key.0 | KeyModifiers::META.0), size + 1));
                }
                Next1::Partial => return None,
                Next1::NoMatch => {}
            }
        }
        Some(keyboard::fallback(buf, ctx))
    }

    /// `partial_key` (`tty-keys.c:979-1020`): wait on a running timer, retry
    /// with `expired` after it fired, or arm it.
    fn partial<'a>(
        &mut self,
        buf: &'a [u8],
        ctx: &KeyDecodeContext,
        theme_changed: bool,
    ) -> DecodeStep<'a> {
        if self.timer != TimerPhase::Idle {
            if self.timer == TimerPhase::Fired {
                if let Some((key, size)) = self.first_key(buf, ctx, true) {
                    let mut step = self.complete_key(buf, key, size, None);
                    if let DecodeStep::Complete {
                        theme_changed: t, ..
                    } = &mut step
                    {
                        *t = theme_changed;
                    }
                    return step;
                }
            }
            return DecodeStep::Partial {
                timer: None,
                theme_changed,
            };
        }

        let mut delay = ctx.escape_time_ms;
        if delay == 0 {
            delay = 1;
        }
        if self.bracket_paste && partial_paste_end(buf) {
            delay = delay.max(500);
        }
        let flags = ctx.flags;
        if flags.intersects(TtyFlags::WAITFG | TtyFlags::WAITBG)
            || flags.intersects(TtyFlags::OSC52QUERY | TtyFlags::WINSIZEQUERY)
            || !flags.contains(TtyFlags::ALL_REQUEST_FLAGS)
            || ctx.has_input_requests
        {
            delay = delay.max(500);
        }
        self.timer = TimerPhase::Waiting;
        DecodeStep::Partial {
            timer: Some(TimerRequest {
                timer: TtyTimer::Key,
                after: Some(Duration::from_millis(u64::from(delay))),
            }),
            theme_changed,
        }
    }

    /// `complete_key` (`tty-keys.c:1022-1060`) for a key or mouse event.
    fn complete_key<'a>(
        &mut self,
        buf: &'a [u8],
        key: KeyCode,
        size: usize,
        mouse: Option<MouseEvent>,
    ) -> DecodeStep<'a> {
        self.complete(
            size,
            TtyInput::Key(DecodedKey {
                key,
                mouse,
                raw: &buf[..size],
            }),
        )
    }

    /// `complete_key` for a reply (`KEYC_UNKNOWN`: nothing is dispatched).
    fn complete<'a>(&mut self, size: usize, input: TtyInput<'a>) -> DecodeStep<'a> {
        let cancel_timer = self.timer != TimerPhase::Idle;
        self.timer = TimerPhase::Idle;
        DecodeStep::Complete {
            consumed: size,
            input,
            cancel_timer,
            theme_changed: false,
        }
    }

    /// `tty_keys_next` (`tty-keys.c:744-1069`): decode one step from the
    /// unread input. The caller applies the step before calling again.
    pub fn next<'a>(&mut self, buf: &'a [u8], ctx: &KeyDecodeContext) -> DecodeStep<'a> {
        use Recognition::{Complete, NoMatch, Partial};

        if buf.is_empty() {
            return DecodeStep::Empty;
        }

        // With no session there is nowhere to send input.
        if !ctx.has_session {
            let cancel_timer = self.timer != TimerPhase::Idle;
            self.timer = TimerPhase::Idle;
            return DecodeStep::Discard {
                consumed: buf.len(),
                cancel_timer,
            };
        }

        match reply::clipboard(buf, ctx) {
            Complete(size, r) => return self.complete(size, TtyInput::Clipboard(r)),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }
        match reply::sync(buf, ctx) {
            Complete(size, r) => return self.complete(size, TtyInput::Discovery(r)),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }
        match reply::device_attributes(buf, ctx) {
            Complete(size, r) => return self.complete(size, TtyInput::Discovery(r)),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }
        match reply::device_attributes2(buf, ctx) {
            Complete(size, r) => return self.complete(size, TtyInput::Discovery(r)),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }
        match reply::extended_device_attributes(buf, ctx) {
            Complete(size, r) => return self.complete(size, TtyInput::Discovery(r)),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }
        match reply::colours(buf) {
            Complete(size, r) => return self.complete(size, TtyInput::Colour(r)),
            Partial => return self.partial(buf, ctx, true),
            NoMatch => {}
        }
        match reply::palette(buf) {
            Complete(size, r) => return self.complete(size, TtyInput::Palette(r)),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }
        match mouse::mouse(buf, &mut self.mouse_last) {
            mouse::MouseResult::Complete(size, m) => {
                return self.complete_key(buf, MOUSE, size, Some(m));
            }
            mouse::MouseResult::Discard(size) => {
                return DecodeStep::Discard {
                    consumed: size,
                    cancel_timer: false,
                };
            }
            mouse::MouseResult::Partial => return self.partial(buf, ctx, false),
            mouse::MouseResult::NoMatch => {}
        }
        match keyboard::extended_key(buf, ctx) {
            Complete(size, key) => return self.complete_key(buf, key, size, None),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }
        match reply::winsz(buf, ctx) {
            Complete(size, r) => return self.complete(size, TtyInput::Size(r)),
            Partial => return self.partial(buf, ctx, false),
            NoMatch => {}
        }

        match self.first_key(buf, ctx, false) {
            Some((key, size)) => self.complete_key(buf, key, size, None),
            None => self.partial(buf, ctx, false),
        }
    }
}

/// `tty_keys_colours` for `refresh-client` colour reports
/// (`cmd-refresh-client.c:141-169`): the same OSC 10/11 grammar and bounds
/// as terminal input. `Complete` carries the consumed length and the parsed
/// target and colour; the caller applies the wait-flag change.
pub fn parse_colour_response(buf: &[u8]) -> Recognition<ColourReply> {
    // The C caller passes a C string; an empty one fails at `buf[0]`.
    if buf.is_empty() {
        return Recognition::NoMatch;
    }
    reply::colours(buf)
}

/// The `TimerRequest` that cancels the key timer, for adapters applying
/// `cancel_timer`.
pub fn cancel_key_timer() -> TimerRequest {
    TimerRequest {
        timer: TtyTimer::Key,
        after: None,
    }
}
