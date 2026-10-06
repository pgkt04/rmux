// Ported from tmux tty.c and tmux.h @ 8f25579c
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TtyFlags(pub u32);
impl TtyFlags {
    pub const NOCURSOR: Self = Self(1);
    pub const FREEZE: Self = Self(2);
    pub const TIMER: Self = Self(4);
    pub const NOBLOCK: Self = Self(8);
    pub const STARTED: Self = Self(16);
    pub const OPENED: Self = Self(32);
    pub const OSC52QUERY: Self = Self(64);
    pub const BLOCK: Self = Self(128);
    pub const HAVEDA: Self = Self(256);
    pub const HAVEXDA: Self = Self(512);
    pub const SYNCING: Self = Self(1024);
    pub const HAVEDA2: Self = Self(2048);
    pub const WINSIZEQUERY: Self = Self(4096);
    pub const WAITFG: Self = Self(8192);
    pub const WAITBG: Self = Self(16384);
    pub const BRACKETPASTE: Self = Self(32768);
    pub const HAVESYNC: Self = Self(65536);
    pub const ALL_REQUEST_FLAGS: Self = Self(68352);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for TtyFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for TtyFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for TtyFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

mod attr;
mod output;
mod protocol;
pub use protocol::{MAX_PROTOCOL_BYTES, PROTOCOL_CONTROL_RESERVE, ProtocolTransaction, QueueFull};
#[cfg(test)]
mod tests;

use crate::features::TtyFeatures;
use crate::keys::TtyKeyDecoder;
use crate::term::terminfo::CapList;
use crate::term::tparm::TparmState;
use crate::term::{TtyCodeCode, TtyTerm, TtyTermFlags};
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::ClientTheme;
use rmux_emu::screen::{ScreenCursorStyle, ScreenMode};
use rmux_sys::TermiosState;
use rmux_util::buffer::ByteBuffer;
use rmux_util::bytes::{ByteString, cstr};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TtyOptions {
    pub clear_on_attach: bool,
    pub extended_keys: bool,
    pub focus_events: bool,
    pub default_terminal: ByteString,
    pub terminal_overrides: Vec<ByteString>,
    pub terminal_features: Vec<ByteString>,
}
impl Default for TtyOptions {
    fn default() -> Self {
        Self {
            clear_on_attach: true,
            extended_keys: false,
            focus_events: false,
            default_terminal: ByteString::from("screen"),
            terminal_overrides: Vec::new(),
            terminal_features: Vec::new(),
        }
    }
}
pub const TTY_OPTION_NAMES: [&str; 6] = [
    "clear-on-attach",
    "extended-keys",
    "focus-events",
    "default-terminal",
    "terminal-overrides",
    "terminal-features",
];

#[derive(Clone, Debug)]
pub struct TtyHostInfo {
    pub name: ByteString,
    pub utf8: bool,
    pub theme: ClientTheme,
    pub theme_colours: [i32; 10],
    pub features: TtyFeatures,
}
impl Default for TtyHostInfo {
    fn default() -> Self {
        Self {
            name: ByteString::new(),
            utf8: false,
            theme: ClientTheme::Unknown,
            theme_colours: [-1; 10],
            features: TtyFeatures::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TtyEffect {
    ReadClosed,
    RedrawClient,
    AllRedrawFlags,
    Discarded(usize),
    Written(usize),
    ProtocolInvalidated { generation: u64 },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TtyTimer {
    Start,
    Clipboard,
    Block,
    Key,
    Protocol,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerRequest {
    pub timer: TtyTimer,
    pub after: Option<Duration>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadOutcome {
    Bytes(usize),
    Closed,
}

static OUTPUT_LOG: LazyLock<Mutex<Option<File>>> = LazyLock::new(|| Mutex::new(None));
pub struct OutLog;
impl OutLog {
    pub fn create() -> io::Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(format!("rmux-out-{}.log", std::process::id()))?;
        *OUTPUT_LOG.lock().expect("output log mutex poisoned") = Some(file);
        Ok(())
    }
}

pub struct Tty {
    pub(crate) fd: OwnedFd,
    pub(crate) tio: TermiosState,
    pub(crate) host: TtyHostInfo,
    pub(crate) opts: TtyOptions,
    pub(crate) term: Option<TtyTerm>,
    pub(crate) sx: u32,
    pub(crate) sy: u32,
    pub(crate) xpixel: u32,
    pub(crate) ypixel: u32,
    pub(crate) cx: u32,
    pub(crate) cy: u32,
    pub(crate) cstyle: ScreenCursorStyle,
    pub(crate) ccolour: i32,
    pub(crate) oflag: bool,
    pub(crate) oox: u32,
    pub(crate) ooy: u32,
    pub(crate) osx: u32,
    pub(crate) osy: u32,
    pub(crate) mode: ScreenMode,
    pub(crate) fg: i32,
    pub(crate) bg: i32,
    pub(crate) rlower: u32,
    pub(crate) rupper: u32,
    pub(crate) rleft: u32,
    pub(crate) rright: u32,
    pub(crate) in_buf: ByteBuffer,
    pub(crate) out: VecDeque<u8>,
    pub(crate) discarded: usize,
    pub(crate) sync_offset: usize,
    pub(crate) redraw_bytes: usize,
    pub(crate) last_requests: Option<SystemTime>,
    pub(crate) cell: GridCell,
    pub(crate) last_cell: GridCell,
    pub(crate) flags: TtyFlags,
    pub(crate) effects: Vec<TtyEffect>,
    pub(crate) timers: Vec<TimerRequest>,
    pub(crate) scratch: Vec<u8>,
    pub(crate) keys: TtyKeyDecoder,
    pub(crate) write_pending: bool,
    pub(crate) read_pending: bool,
    protocol_out: protocol::ProtocolQueue,
    protocol_generation: u64,
    protocol_stop: bool,
    protocol_close: bool,
    read_limit: Option<usize>,
    read_paused: bool,
}

impl Tty {
    pub fn new(fd: OwnedFd, tio: TermiosState, host: TtyHostInfo) -> Self {
        Self {
            fd,
            tio,
            host,
            opts: TtyOptions::default(),
            term: None,
            sx: 0,
            sy: 0,
            xpixel: 0,
            ypixel: 0,
            cx: 0,
            cy: 0,
            cstyle: ScreenCursorStyle::Default,
            ccolour: -1,
            oflag: false,
            oox: 0,
            ooy: 0,
            osx: 0,
            osy: 0,
            mode: ScreenMode(0),
            fg: -1,
            bg: -1,
            rlower: 0,
            rupper: 0,
            rleft: 0,
            rright: 0,
            in_buf: ByteBuffer::new(),
            out: VecDeque::new(),
            discarded: 0,
            sync_offset: 0,
            redraw_bytes: 0,
            last_requests: None,
            cell: DEFAULT_CELL,
            last_cell: DEFAULT_CELL,
            flags: TtyFlags::default(),
            effects: Vec::new(),
            timers: Vec::new(),
            scratch: Vec::new(),
            keys: TtyKeyDecoder::default(),
            write_pending: false,
            read_pending: false,
            protocol_out: protocol::ProtocolQueue::default(),
            protocol_generation: 0,
            protocol_stop: false,
            protocol_close: false,
            read_limit: None,
            read_paused: false,
        }
    }

    pub fn open(
        &mut self,
        state: &mut TparmState,
        name: &[u8],
        caps: &CapList,
        opts: &TtyOptions,
        colorterm: Option<&[u8]>,
    ) -> Result<(), ByteString> {
        self.opts = opts.clone();
        match TtyTerm::create(state, name, caps, &mut self.host, opts, colorterm) {
            Ok(term) => self.term = Some(term),
            Err(cause) => {
                self.close(state);
                return Err(cause);
            }
        }
        self.flags.insert(TtyFlags::OPENED);
        self.flags
            .remove(TtyFlags::NOCURSOR | TtyFlags::FREEZE | TtyFlags::BLOCK | TtyFlags::TIMER);
        self.in_buf = ByteBuffer::new();
        self.out.clear();
        self.start(state, opts);
        self.keys.rebuild(
            self.term.as_ref().expect("opened tty has term"),
            std::iter::empty(),
        );
        Ok(())
    }

    pub fn start(&mut self, state: &mut TparmState, opts: &TtyOptions) {
        self.opts = opts.clone();
        self.reset_protocol();
        self.protocol_stop = false;
        self.protocol_close = false;
        self.read_paused = false;
        rmux_sys::fd::set_blocking(self.fd(), false);
        self.read_pending = true;
        let mut tio = self.tio;
        tio.make_tty_raw();
        if tio.set(self.fd()).is_ok() {
            let _ = TermiosState::flush_output(self.fd());
        }
        if opts.clear_on_attach {
            self.putcode(TtyCodeCode::Smcup);
            self.putcode(TtyCodeCode::Clear);
        } else {
            self.putcode_ii(state, TtyCodeCode::Csr, 0, self.sy.wrapping_sub(1) as i32);
            self.putcode_ii(state, TtyCodeCode::Cup, 0, self.sy.wrapping_sub(1) as i32);
            if self.term().has(TtyCodeCode::Indn) {
                self.putcode_i(state, TtyCodeCode::Indn, self.sy.wrapping_add(1) as i32);
            } else if self.term().has(TtyCodeCode::Ind) {
                for _ in 0..self.sy.wrapping_add(1) {
                    self.putcode(TtyCodeCode::Ind);
                }
            } else {
                self.putcode(TtyCodeCode::Clear);
            }
        }
        self.putcode(TtyCodeCode::Smkx);
        if crate::acs::acs_needed(self.term(), self.host.utf8) {
            self.putcode(TtyCodeCode::Enacs);
        }
        self.putcode(TtyCodeCode::Cnorm);
        if self.term().has(TtyCodeCode::Kmous) {
            self.puts(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l");
            self.puts(b"\x1b[?1006l\x1b[?1005l");
        }
        if self.term().has(TtyCodeCode::Enbp) {
            self.putcode(TtyCodeCode::Enbp);
        }
        if self.term().flags().contains(TtyTermFlags::VT100LIKE) {
            self.puts(b"\x1b[?2031h\x1b[?996n");
        }
        self.start_start_timer();
        self.flags.insert(TtyFlags::STARTED);
        self.invalidate(state);
        if self.ccolour != -1 {
            self.force_cursor_colour(state, -1);
        }
    }

    pub fn stop(&mut self, state: &mut TparmState, opts: &TtyOptions) {
        if !self.flags.contains(TtyFlags::STARTED) {
            return;
        }
        if !self.protocol_out.is_empty() {
            self.stop_after_protocol(state, opts);
            return;
        }
        self.reset_protocol();
        self.flags.remove(TtyFlags::STARTED);
        for timer in [TtyTimer::Start, TtyTimer::Clipboard, TtyTimer::Block] {
            self.timer(timer, None);
        }
        self.flags.remove(TtyFlags::BLOCK);
        self.read_pending = false;
        self.write_pending = false;
        let Ok(ws) = rmux_sys::pty::get_winsize(self.fd()) else {
            return;
        };
        if self.tio.set(self.fd()).is_err() {
            return;
        }
        self.term.as_ref().expect("started tty has term").string_ii(
            state,
            TtyCodeCode::Csr,
            0,
            i32::from(ws.rows) - 1,
            &mut self.scratch,
        );
        self.raw_scratch();
        if crate::acs::acs_needed(self.term(), self.host.utf8) {
            self.raw_code(TtyCodeCode::Rmacs);
        }
        self.raw_code(TtyCodeCode::Sgr0);
        self.raw_code(TtyCodeCode::Rmkx);
        if opts.clear_on_attach {
            self.raw_code(TtyCodeCode::Clear);
        }
        if self.cstyle != ScreenCursorStyle::Default {
            if self.term().has(TtyCodeCode::Se) {
                self.raw_code(TtyCodeCode::Se);
            } else if self.term().has(TtyCodeCode::Ss) {
                self.term.as_ref().expect("started tty has term").string_i(
                    state,
                    TtyCodeCode::Ss,
                    0,
                    &mut self.scratch,
                );
                self.raw_scratch();
            }
        }
        if self.ccolour != -1 {
            self.raw_code(TtyCodeCode::Cr);
        }
        self.raw_code(TtyCodeCode::Cnorm);
        if self.term().has(TtyCodeCode::Kmous) {
            self.raw(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l");
            self.raw(b"\x1b[?1006l\x1b[?1005l");
        }
        if self.term().has(TtyCodeCode::Dsbp) {
            self.raw_code(TtyCodeCode::Dsbp);
        }
        for code in [TtyCodeCode::Dsesc, TtyCodeCode::Dsfcs, TtyCodeCode::Dseks] {
            self.raw_code(code);
        }
        if self.term().flags().contains(TtyTermFlags::DECSLRM) {
            self.raw_code(TtyCodeCode::Dsmg);
        }
        self.raw_code(if opts.clear_on_attach {
            TtyCodeCode::Rmcup
        } else {
            TtyCodeCode::Clear
        });
        if self.term().flags().contains(TtyTermFlags::VT100LIKE) {
            self.raw(b"\x1b[?2031l");
        }
        rmux_sys::fd::set_blocking(self.fd(), true);
    }

    pub fn close(&mut self, state: &mut TparmState) {
        self.timer(TtyTimer::Key, None);
        let opts = self.opts.clone();
        self.stop(state, &opts);
        if self.protocol_stop {
            self.protocol_close = true;
            return;
        }
        if self.flags.contains(TtyFlags::OPENED) {
            self.in_buf = ByteBuffer::new();
            self.out = VecDeque::new();
            self.term = None;
            self.read_pending = false;
            self.write_pending = false;
            self.keys.clear();
            self.flags.remove(TtyFlags::OPENED);
        }
    }

    pub fn resize(&mut self, state: &mut TparmState) -> Vec<TimerRequest> {
        let (sx, sy, xp, yp) = match rmux_sys::pty::get_winsize(self.fd()) {
            Ok(ws) => {
                let sx = if ws.cols == 0 { 80 } else { u32::from(ws.cols) };
                let sy = if ws.rows == 0 { 24 } else { u32::from(ws.rows) };
                let xp = if ws.cols == 0 {
                    0
                } else {
                    u32::from(ws.xpixel) / sx
                };
                let yp = if ws.rows == 0 {
                    0
                } else {
                    u32::from(ws.ypixel) / sy
                };
                if (xp == 0 || yp == 0)
                    && self.flags.contains(TtyFlags::OPENED)
                    && !self.flags.contains(TtyFlags::WINSIZEQUERY)
                    && self.term().flags().contains(TtyTermFlags::VT100LIKE)
                {
                    self.puts(b"\x1b[18t\x1b[14t");
                    self.flags.insert(TtyFlags::WINSIZEQUERY);
                }
                (sx, sy, xp, yp)
            }
            Err(_) => (80, 24, 0, 0),
        };
        self.set_size(sx, sy, xp, yp);
        self.invalidate(state);
        Vec::new()
    }
    pub fn set_size(&mut self, sx: u32, sy: u32, xp: u32, yp: u32) {
        self.sx = sx;
        self.sy = sy;
        self.xpixel = xp;
        self.ypixel = yp;
    }
    /// `tty_read_callback` read step. mio readiness is edge-triggered, so
    /// drain while reads fill the buffer; a short read means the fd is empty.
    /// `EAGAIN`/`EINTR` are not a closed tty.
    pub fn on_readable(&mut self) -> ReadOutcome {
        // tty_stop_tty deletes event_in (tty.c:468): a stopped tty is never
        // read. A ready event from the same poll can still arrive, and after
        // the stop the fd is blocking again, so a read here could hang.
        if self.read_paused || !self.read_pending {
            return ReadOutcome::Bytes(0);
        }
        let mut bytes = [0; 4096];
        let mut total = 0;
        let protocol = self.protocol_partial();
        // Protocol bodies may exceed the ordinary-input budget. Do not read
        // even one trailing ordinary byte under that framing exemption.
        if protocol && self.in_buf.data().ends_with(b"\x1b\\") {
            return ReadOutcome::Bytes(0);
        }
        loop {
            let maximum = crate::keys::MAX_TSP_INPUT_BODY + 4096;
            let allowance = self
                .read_limit
                .unwrap_or(maximum)
                .min(maximum)
                .saturating_sub(self.in_buf.len())
                .min(if protocol { 1 } else { bytes.len() });
            if allowance == 0 {
                return ReadOutcome::Bytes(total);
            }
            match rmux_sys::fd::read(self.fd(), &mut bytes[..allowance]) {
                Ok(0) => break,
                Ok(n) => {
                    self.in_buf.add(&bytes[..n]);
                    total += n;
                    if protocol && self.in_buf.data().ends_with(b"\x1b\\") {
                        return ReadOutcome::Bytes(total);
                    }
                    if n == allowance {
                        continue;
                    }
                    return ReadOutcome::Bytes(total);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return ReadOutcome::Bytes(total);
                }
                Err(_) => break,
            }
        }
        self.reset_protocol();
        self.read_pending = false;
        self.effects.push(TtyEffect::ReadClosed);
        ReadOutcome::Closed
    }
    /// `tty_write_callback`: a transient error keeps the write interest so
    /// queued output is not stranded.
    pub fn on_writable(&mut self) -> io::Result<usize> {
        self.write_pending = false;
        let fd = self.fd.as_fd();
        let (n, is_protocol) =
            match protocol::write_queued(&mut self.protocol_out, &mut self.out, |bytes| {
                rmux_sys::fd::write(fd, bytes)
            }) {
                Ok(result) => result,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    self.write_pending = !self.out.is_empty() || !self.protocol_out.is_empty();
                    return Ok(0);
                }
                Err(e) => {
                    self.reset_protocol();
                    return Err(e);
                }
            };
        self.finish_protocol_stop();
        if !is_protocol && self.redraw_bytes > 0 {
            self.redraw_bytes = self.redraw_bytes.saturating_sub(n);
        } else if !is_protocol && self.block_maybe() {
            return Ok(n);
        }
        self.write_pending = !self.out.is_empty() || !self.protocol_out.is_empty();
        Ok(n)
    }
    pub(crate) fn block_maybe(&mut self) -> bool {
        let size = self.out.len();
        if size == 0 {
            self.flags.remove(TtyFlags::NOBLOCK);
        } else if self.flags.contains(TtyFlags::NOBLOCK) {
            return false;
        }
        let threshold = 1u32.wrapping_add(self.sx.wrapping_mul(self.sy).wrapping_mul(8));
        if size < threshold as usize {
            return false;
        }
        if self.flags.contains(TtyFlags::BLOCK) {
            return true;
        }
        self.flags.insert(TtyFlags::BLOCK);
        self.out.clear();
        self.protocol_out.discard_cells();
        self.write_pending = !self.protocol_out.is_empty();
        self.effects.push(TtyEffect::Discarded(size));
        self.discarded = 0;
        self.timer(TtyTimer::Block, Some(Duration::from_millis(100)));
        true
    }
    pub fn on_timer(&mut self, state: &mut TparmState, timer: TtyTimer) {
        match timer {
            TtyTimer::Start => {
                if !self
                    .flags
                    .intersects(TtyFlags::HAVEDA | TtyFlags::HAVEDA2 | TtyFlags::HAVEXDA)
                {
                    self.update_features_inner(state);
                }
                self.flags.insert(TtyFlags::ALL_REQUEST_FLAGS);
                self.flags.remove(TtyFlags::WAITBG | TtyFlags::WAITFG);
            }
            TtyTimer::Clipboard => self.flags.remove(TtyFlags::OSC52QUERY),
            TtyTimer::Block => {
                self.effects.push(TtyEffect::AllRedrawFlags);
                self.effects.push(TtyEffect::Discarded(self.discarded));
                if self.discarded < 1u32.wrapping_add(self.sx.wrapping_mul(self.sy) / 8) as usize {
                    self.flags.remove(TtyFlags::BLOCK);
                    self.invalidate(state);
                } else {
                    self.discarded = 0;
                    self.timer(TtyTimer::Block, Some(Duration::from_millis(100)));
                }
            }
            TtyTimer::Key => self.keys.timer_fired(),
            TtyTimer::Protocol => self.keys.protocol_timer_fired(),
        }
    }
    pub fn send_requests(&mut self, now: SystemTime) {
        if !self.flags.contains(TtyFlags::STARTED) {
            return;
        }
        if self.term().flags().contains(TtyTermFlags::VT100LIKE) {
            if !self.flags.contains(TtyFlags::HAVEDA)
                && !self
                    .keys
                    .pending_da1()
                    .any(|owner| owner == crate::keys::Da1Owner::Discovery)
            {
                let _ = self.queue_da1(
                    crate::keys::Da1Owner::Discovery,
                    ProtocolTransaction::new(Vec::new()).control(),
                );
            }
            for (flag, bytes) in [
                (TtyFlags::HAVEDA2, &b"\x1b[>c"[..]),
                (TtyFlags::HAVEXDA, &b"\x1b[>q"[..]),
                (TtyFlags::HAVESYNC, &b"\x1b[?2026$p"[..]),
            ] {
                if !self.flags.contains(flag) {
                    self.puts(bytes);
                }
            }
            self.puts(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\");
            self.flags.insert(TtyFlags::WAITBG | TtyFlags::WAITFG);
        } else {
            self.flags.insert(TtyFlags::ALL_REQUEST_FLAGS);
        }
        self.last_requests = Some(now);
    }
    pub fn repeat_requests(&mut self, force: bool, now: SystemTime) {
        if !self.flags.contains(TtyFlags::STARTED) {
            return;
        }
        let seconds = |t: SystemTime| -> i128 {
            match t.duration_since(UNIX_EPOCH) {
                Ok(d) => i128::from(d.as_secs()),
                Err(e) => {
                    -i128::from(e.duration().as_secs())
                        - i128::from(e.duration().subsec_nanos() != 0)
                }
            }
        };
        let elapsed = (seconds(now) - seconds(self.last_requests.unwrap_or(UNIX_EPOCH))) as u32;
        if !force && elapsed <= 30 {
            return;
        }
        self.last_requests = Some(now);
        if self.term().flags().contains(TtyTermFlags::VT100LIKE) {
            self.puts(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\");
            self.flags.insert(TtyFlags::WAITBG | TtyFlags::WAITFG);
        }
        self.start_start_timer();
    }
    pub fn update_features(&mut self, state: &mut TparmState, opts: &TtyOptions) {
        self.opts = opts.clone();
        self.update_features_inner(state);
    }
    fn update_features_inner(&mut self, state: &mut TparmState) {
        let term = self
            .term
            .as_mut()
            .expect("feature update requires open tty");
        if term.apply_features(&mut self.host) {
            term.apply_overrides(state, &self.opts.terminal_overrides);
        }
        if self.term().flags().contains(TtyTermFlags::DECSLRM) {
            self.putcode(TtyCodeCode::Enmg);
        }
        if self.opts.extended_keys {
            self.putcode(TtyCodeCode::Eneks);
        }
        if self.opts.focus_events {
            self.putcode(TtyCodeCode::Enfcs);
        }
        self.putcode(TtyCodeCode::Enesc);
        self.effects.push(TtyEffect::RedrawClient);
        self.invalidate(state);
    }
    fn start_start_timer(&mut self) {
        self.timer(TtyTimer::Start, None);
        self.timer(TtyTimer::Start, Some(Duration::from_secs(5)));
    }
    pub(crate) fn timer(&mut self, timer: TtyTimer, after: Option<Duration>) {
        self.timers.push(TimerRequest { timer, after });
    }
    pub fn wants_write(&self) -> bool {
        self.write_pending
    }
    pub fn wants_read(&self) -> bool {
        self.read_pending && !self.read_paused
    }
    pub fn set_redraw_bytes(&mut self, n: usize) {
        self.redraw_bytes = n;
    }
    pub fn drain_effects(&mut self) -> impl Iterator<Item = TtyEffect> + '_ {
        self.effects.drain(..)
    }
    pub fn pending_timers(&mut self) -> impl Iterator<Item = TimerRequest> + '_ {
        self.timers.drain(..)
    }
    pub fn host_mut(&mut self) -> &mut TtyHostInfo {
        &mut self.host
    }
    pub fn set_options(&mut self, opts: TtyOptions) {
        self.opts = opts;
    }
    pub fn size(&self) -> (u32, u32) {
        (self.sx, self.sy)
    }
    pub fn pixel_size(&self) -> (u32, u32) {
        (self.xpixel, self.ypixel)
    }
    pub fn flags(&self) -> TtyFlags {
        self.flags
    }
    pub fn flags_mut(&mut self) -> &mut TtyFlags {
        &mut self.flags
    }
    pub fn mode(&self) -> ScreenMode {
        self.mode
    }
    pub fn term(&self) -> &TtyTerm {
        self.term.as_ref().expect("tty term requested before open")
    }
    pub fn term_mut(&mut self) -> &mut TtyTerm {
        self.term.as_mut().expect("tty term requested before open")
    }
    pub fn out_len(&self) -> usize {
        self.out.len() + self.protocol_out.bytes
    }
    /// `tty->sync_offset` (`server-client.c:2393`): queued bytes before the
    /// current synchronized frame started.
    pub fn sync_offset(&self) -> usize {
        self.sync_offset
    }
    /// One `tty_keys_next` step over the pending input bytes (`tty-keys.c:745`).
    pub fn decode_next(
        &mut self,
        ctx: &crate::keys::KeyDecodeContext,
    ) -> crate::keys::DecodeStep<'_> {
        let step = self.keys.next(self.in_buf.data(), ctx);
        if matches!(
            step,
            crate::keys::DecodeStep::Complete {
                input: crate::keys::TtyInput::Tsp { .. }
                    | crate::keys::TtyInput::Da1Sentinel { .. }
                    | crate::keys::TtyInput::ProtocolFault(_),
                ..
            } | crate::keys::DecodeStep::Discard {
                cancel_timer: true,
                ..
            }
        ) {
            self.timers.push(TimerRequest {
                timer: TtyTimer::Protocol,
                after: None,
            });
        }
        step
    }
    /// `tio.c_cc[VERASE]` unless `_POSIX_VDISABLE` (`tty-keys.c` backspace).
    pub fn verase(&self) -> Option<u8> {
        self.tio.erase()
    }
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
    pub fn keys_mut(&mut self) -> &mut TtyKeyDecoder {
        &mut self.keys
    }
    pub fn input_bytes(&self) -> &[u8] {
        self.in_buf.data()
    }
    pub fn consume_input(&mut self, n: usize) {
        assert!(n <= self.in_buf.len(), "decoded input exceeds buffer");
        self.in_buf.drain(n);
    }
    pub fn reported_colours(&self) -> (i32, i32) {
        (self.fg, self.bg)
    }
    pub fn set_reported_colour(&mut self, foreground: bool, colour: i32) {
        if foreground {
            self.fg = colour;
        } else {
            self.bg = colour;
        }
    }
    pub fn window_offset(&self) -> (bool, u32, u32, u32, u32) {
        (self.oflag, self.oox, self.ooy, self.osx, self.osy)
    }
    pub fn set_window_offset(&mut self, flag: bool, ox: u32, oy: u32, sx: u32, sy: u32) {
        self.oflag = flag;
        self.oox = ox;
        self.ooy = oy;
        self.osx = sx;
        self.osy = sy;
    }
    pub fn raw(&mut self, bytes: &[u8]) {
        raw_fd(self.fd(), cstr(bytes));
    }
    fn raw_code(&self, code: TtyCodeCode) {
        raw_fd(self.fd(), cstr(self.term().string(code)));
    }
    fn raw_scratch(&self) {
        raw_fd(self.fd(), cstr(&self.scratch));
    }
}

fn raw_fd(fd: BorrowedFd<'_>, mut bytes: &[u8]) {
    for _ in 0..5 {
        match rmux_sys::fd::write(fd, bytes) {
            Ok(n) => {
                bytes = &bytes[n..];
                if bytes.is_empty() {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_micros(100));
    }
}

fn queue_bytes(
    out: &mut VecDeque<u8>,
    flags: TtyFlags,
    discarded: &mut usize,
    effects: &mut Vec<TtyEffect>,
    write_pending: &mut bool,
    bytes: &[u8],
) {
    if flags.contains(TtyFlags::BLOCK) {
        *discarded = discarded.wrapping_add(bytes.len());
        return;
    }
    out.extend(bytes);
    effects.push(TtyEffect::Written(bytes.len()));
    log_output(bytes);
    if flags.contains(TtyFlags::STARTED) {
        *write_pending = true;
    }
}

/// `tty_log_fd` (`tty.c:92-101`): copy tty bytes to `rmux-out-<pid>.log`
/// when the server runs at `-vv`. TSP transactions are logged when written.
pub(crate) fn log_output(bytes: &[u8]) {
    if let Some(file) = OUTPUT_LOG
        .lock()
        .expect("output log mutex poisoned")
        .as_mut()
    {
        let _ = file.write(bytes);
    }
}
