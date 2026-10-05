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

//! VT parser (`input.c`) and pane key encoding (`input-keys.c`).
//!
//! Pinned quirks kept on purpose: an open UTF-8 sequence survives escape
//! sequences until print or C0 stops it (2.6); `0x7f-0xff` is ignored in ESC,
//! CSI and DCS prefix states; legacy `since_ground` grows without limit while
//! unterminated (2.4); reset and timeout keep pending bytes and partial UTF-8.
//! TSP APC has its own bounded collection, commits only at 7-bit ST and drains
//! cancelled/overlong payloads without feeding them into titles or grid cells.

mod c0_esc;
mod csi;
pub use csi::TMUX_VERSION;
mod dcs;
pub mod dump;
pub mod effect;
pub mod keys;
mod osc;
mod params;
pub mod reply;
mod sgr;
mod states;

pub use effect::*;

use crate::cell::{DEFAULT_CELL, GridCell};
use crate::colour::ColourPalette;
use crate::screen::ScreenMode;
use crate::screen::write::ScreenWriteCtx;
use params::Params;
use rmux_util::utf8::{Utf8Data, Utf8State};
use states::{Enter, Exit, Handler, StateId, Transition};

/// `INPUT_BUF_START` (`input.c:117`).
const STRING_START: usize = 32;
/// Standard TSP payload limit, independent of `input-buffer-size`.
pub const TSP_APC_LIMIT: usize = 262_144;
const FLAG_DISCARD: u8 = 0x1;
const FLAG_LAST: u8 = 0x2;

/// Fixed C byte buffer with a length (`interm_buf`, `param_buf`): `N - 1`
/// payload bytes plus the terminating NUL.
#[derive(Clone, Copy, Debug)]
struct SmallBuf<const N: usize> {
    bytes: [u8; N],
    len: u8,
}

impl<const N: usize> Default for SmallBuf<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> SmallBuf<N> {
    fn as_slice(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
    fn clear(&mut self) {
        self.len = 0;
    }
    /// `false` when the buffer holds `N - 1` bytes already.
    fn push(&mut self, ch: u8) -> bool {
        if usize::from(self.len) == N - 1 {
            return false;
        }
        self.bytes[usize::from(self.len)] = ch;
        self.len += 1;
        true
    }
}

/// `struct input_cell` (`input.c:78-83`).
#[derive(Clone, Copy, Debug)]
struct InputCell {
    cell: GridCell,
    set: u8,
    g0set: bool,
    g1set: bool,
}

impl Default for InputCell {
    fn default() -> Self {
        Self {
            cell: DEFAULT_CELL,
            set: 0,
            g0set: false,
            g1set: false,
        }
    }
}

/// Owned description of a yielded effect; payload bytes live in parser
/// scratch and are borrowed when the step returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Reply,
    TspMessage,
    TerminalReset,
    Request(InputRequestKind, InputEnd),
    ClipboardQuery(u8, InputEnd),
    ClipboardReceived,
    ColourQuery(ColourQueryKind, InputEnd),
    ThemeReport,
    ThemeUpdatesEnabled,
    ThemeUpdatesDisabled,
    Bell,
    /// Title bytes start at this offset of the string buffer.
    TitleChanged(usize),
    TitlePopped,
    PathChanged,
    Rename(bool),
    ProgressChanged,
    StyleChanged(bool),
    SyncStart,
    SyncEnd,
    Osc133(Osc133Event),
    GroundTimer(bool),
    AlternateChanged(bool),
}

/// Handler continuation points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sub {
    Start,
    /// Private SM/RM: continue at parameter `i`.
    Modes {
        set: bool,
        i: u8,
    },
    /// WINOPS: continue at parameter `m`.
    Winops {
        m: u8,
    },
    /// OSC 4: continue at byte `off` of the string.
    Osc4 {
        off: usize,
        redraw: bool,
    },
    /// OSC 10/11/110/111: style flags applied, full redraw outstanding.
    Redraw,
    /// OSC 133 D: event fired, line end marker outstanding.
    Osc133End {
        status: u8,
    },
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Handler(Sub),
    Exit(Sub),
    Enter,
    Append,
}

/// Where the current byte stopped (`InputResume` in the spec).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Resume {
    phase: Phase,
    transition: Transition,
}

/// `Flow` of one handler run.
enum Flow {
    Done,
    Yield(Pending, Sub),
}

/// Writer, palette and policy borrows for one handler run.
pub(crate) struct Env<'a, 'b> {
    pub(crate) sw: &'a mut ScreenWriteCtx<'b>,
    pub(crate) palette: Option<&'a mut ColourPalette>,
    pub(crate) policy: &'a InputPolicy,
}

/// `struct input_ctx` (`input.c:99-147`) without the server fields.
#[derive(Debug)]
pub struct InputCtx {
    state: StateId,
    flags: u8,
    cell: InputCell,
    old_cell: InputCell,
    old_cx: u32,
    old_cy: u32,
    old_mode: ScreenMode,
    interm: SmallBuf<4>,
    param_buf: SmallBuf<64>,
    params: Params,
    string: Vec<u8>,
    string_space: usize,
    string_end: InputEnd,
    apc_prefix: Option<u8>,
    utf8: Utf8Data,
    utf8_started: bool,
    ch: u8,
    last: Utf8Data,
    since_ground: Vec<u8>,
    timer_armed: bool,
    resume: Option<Resume>,
    scratch: Vec<u8>,
    clip: SmallBuf<13>,
    decoded: Vec<u8>,
}

impl Default for InputCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl InputCtx {
    /// `input_init` without pane, event or timers (`input.c:875-898`).
    pub fn new() -> InputCtx {
        let mut ictx = InputCtx {
            state: StateId::Ground,
            flags: 0,
            cell: InputCell::default(),
            old_cell: InputCell::default(),
            old_cx: 0,
            old_cy: 0,
            old_mode: ScreenMode(0),
            interm: SmallBuf::default(),
            param_buf: SmallBuf::default(),
            params: Params::default(),
            string: Vec::with_capacity(STRING_START),
            string_space: STRING_START,
            string_end: InputEnd::St,
            apc_prefix: None,
            utf8: Utf8Data::default(),
            utf8_started: false,
            ch: 0,
            last: Utf8Data::default(),
            since_ground: Vec::new(),
            timer_armed: false,
            resume: None,
            scratch: Vec::new(),
            clip: SmallBuf::default(),
            decoded: Vec::new(),
        };
        ictx.reset_cell();
        ictx.clear();
        ictx
    }

    /// `input_reset` (`input.c:927-947`): `Some` performs the writer reset.
    /// Pending bytes, partial UTF-8 state and the saved mode stay intact.
    pub fn reset(&mut self, sw: Option<&mut ScreenWriteCtx<'_>>, sink: &mut dyn InputSink) {
        let tsp_incomplete = self.state.is_tsp().then_some(self.state);
        self.reset_cell();
        if let Some(sw) = sw {
            let policy = sw.screen.reset_policy;
            sw.reset(policy);
        }
        self.clear();
        self.timer_armed = false;
        sink.effect(InputEffect::GroundTimer(false));
        self.state = StateId::Ground;
        self.flags = 0;
        self.resume = None;
        if let Some(state) = tsp_incomplete {
            self.state = if matches!(state, StateId::TspEscape | StateId::TspDiscardEscape) {
                StateId::TspDiscardEscape
            } else {
                StateId::TspDiscard
            };
        }
    }
    /// Ground timeout drops an incomplete string; recognized TSP drains to ST.
    pub fn ground_timeout(&mut self) {
        let tsp_incomplete = self.state.is_tsp().then_some(self.state);
        self.reset_cell();
        self.clear();
        self.timer_armed = false;
        self.state = StateId::Ground;
        self.flags = 0;
        self.resume = None;
        if let Some(state) = tsp_incomplete {
            self.state = if matches!(state, StateId::TspEscape | StateId::TspDiscardEscape) {
                StateId::TspDiscardEscape
            } else {
                StateId::TspDiscard
            };
        }
    }

    /// `input_pending` (`input.c:951-954`).
    pub fn pending(&self) -> &[u8] {
        &self.since_ground
    }

    /// The C state name (`input.c:1068`).
    pub fn state_name(&self) -> &'static str {
        self.state.name()
    }

    /// Standalone driver: every effect reaches `sink` before the parser
    /// resumes.
    pub fn parse(
        &mut self,
        sw: &mut ScreenWriteCtx<'_>,
        mut palette: Option<&mut ColourPalette>,
        policy: &InputPolicy,
        sink: &mut dyn InputSink,
        mut bytes: &[u8],
    ) {
        loop {
            match self.parse_step(sw, palette.as_deref_mut(), policy, bytes) {
                InputStep::Complete { .. } => return,
                InputStep::Effect { consumed, effect } => {
                    sink.effect(effect);
                    bytes = &bytes[consumed..];
                }
            }
        }
    }

    /// `input_parse` (`input.c:969-1024`) up to the next effect. `consumed`
    /// may be zero when the previous byte still has substeps.
    pub fn parse_step<'p>(
        &'p mut self,
        sw: &mut ScreenWriteCtx<'_>,
        palette: Option<&mut ColourPalette>,
        policy: &InputPolicy,
        bytes: &[u8],
    ) -> InputStep<'p> {
        let mut env = Env {
            sw,
            palette,
            policy,
        };
        let mut consumed = 0;
        let pending = loop {
            if self.resume.is_some() {
                if let Some(pending) = self.run_resume(&mut env) {
                    break pending;
                }
            }
            if consumed == bytes.len() {
                return InputStep::Complete { consumed };
            }
            self.ch = bytes[consumed];
            consumed += 1;
            if self.state == StateId::ApcString {
                self.apc_prefix = self.apc_prefix.and_then(|index| {
                    (b"tsp;"[usize::from(index)] == self.ch).then_some(index + 1)
                });
            }
            let transition = states::transition(self.state, self.ch);
            if transition.handler != Some(Handler::Print) {
                env.sw.collect_end();
            }
            self.resume = Some(Resume {
                phase: Phase::Handler(Sub::Start),
                transition,
            });
        };
        InputStep::Effect {
            consumed,
            effect: self.materialize(pending),
        }
    }

    fn materialize(&self, pending: Pending) -> InputEffect<'_> {
        match pending {
            Pending::Reply => InputEffect::Reply(&self.scratch),
            Pending::TspMessage => InputEffect::TspMessage {
                payload: &self.string,
            },
            Pending::TerminalReset => InputEffect::TerminalReset,
            Pending::Request(kind, end) => InputEffect::Request { kind, end },
            Pending::ClipboardQuery(clip, end) => InputEffect::ClipboardQuery { clip, end },
            Pending::ClipboardReceived => InputEffect::ClipboardReceived {
                clip: self.clip.as_slice(),
                data: &self.decoded,
            },
            Pending::ColourQuery(which, end) => InputEffect::ColourQuery { which, end },
            Pending::ThemeReport => InputEffect::ThemeReport,
            Pending::ThemeUpdatesEnabled => InputEffect::ThemeUpdatesEnabled,
            Pending::ThemeUpdatesDisabled => InputEffect::ThemeUpdatesDisabled,
            Pending::Bell => InputEffect::Bell,
            Pending::TitleChanged(off) => InputEffect::TitleChanged(&self.string[off..]),
            Pending::TitlePopped => InputEffect::TitlePopped(&self.scratch),
            Pending::PathChanged => InputEffect::PathChanged,
            Pending::Rename(named) => InputEffect::Rename(named.then_some(&self.string[..])),
            Pending::ProgressChanged => InputEffect::ProgressChanged,
            Pending::StyleChanged(theme) => InputEffect::StyleChanged { theme },
            Pending::SyncStart => InputEffect::SyncStart,
            Pending::SyncEnd => InputEffect::SyncEnd,
            Pending::Osc133(event) => InputEffect::Osc133(event),
            Pending::GroundTimer(arm) => InputEffect::GroundTimer(arm),
            Pending::AlternateChanged(entering) => InputEffect::AlternateChanged { entering },
        }
    }

    /// Continue the current byte; `Some` stops at an effect.
    fn run_resume(&mut self, env: &mut Env<'_, '_>) -> Option<Pending> {
        loop {
            let resume = self.resume.expect("resume state");
            let next_phase = match resume.phase {
                Phase::Handler(sub) => {
                    match self.run_handler(resume.transition.handler, sub, env) {
                        Flow::Yield(pending, sub) => {
                            self.set_phase(Phase::Handler(sub));
                            return Some(pending);
                        }
                        Flow::Done => {
                            if resume.transition.next.is_some() {
                                Phase::Exit(Sub::Start)
                            } else {
                                Phase::Append
                            }
                        }
                    }
                }
                Phase::Exit(sub) => match self.run_exit(self.state.exit(), sub, env) {
                    Flow::Yield(pending, sub) => {
                        self.set_phase(Phase::Exit(sub));
                        return Some(pending);
                    }
                    Flow::Done => {
                        self.state = resume.transition.next.expect("next state");
                        Phase::Enter
                    }
                },
                Phase::Enter => {
                    let pending = self.run_enter(self.state.enter());
                    self.set_phase(Phase::Append);
                    if pending.is_some() {
                        return pending;
                    }
                    continue;
                }
                Phase::Append => {
                    if self.state != StateId::Ground
                        && (!self.state.is_tsp() || self.since_ground.len() < TSP_APC_LIMIT + 6)
                    {
                        self.since_ground.push(self.ch);
                    }
                    self.resume = None;
                    return None;
                }
            };
            self.set_phase(next_phase);
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        if let Some(resume) = &mut self.resume {
            resume.phase = phase;
        }
    }

    fn run_handler(&mut self, handler: Option<Handler>, sub: Sub, env: &mut Env<'_, '_>) -> Flow {
        if sub == Sub::Done {
            return Flow::Done;
        }
        match handler {
            None => Flow::Done,
            Some(Handler::Print) => {
                self.print(env);
                Flow::Done
            }
            Some(Handler::Intermediate) => {
                if !self.interm.push(self.ch) {
                    self.flags |= FLAG_DISCARD;
                }
                Flow::Done
            }
            Some(Handler::Parameter) => {
                if !self.param_buf.push(self.ch) {
                    self.flags |= FLAG_DISCARD;
                }
                Flow::Done
            }
            Some(Handler::Input) => {
                self.input(env.policy.buffer_limit);
                Flow::Done
            }
            Some(Handler::EndBel) => {
                self.string_end = InputEnd::Bel;
                Flow::Done
            }
            Some(Handler::TopBitSet) => {
                self.top_bit_set(env);
                Flow::Done
            }
            Some(Handler::C0Dispatch) => self.c0_dispatch(env),
            Some(Handler::EscDispatch) => self.esc_dispatch(env),
            Some(Handler::TspDispatch) => Flow::Yield(Pending::TspMessage, Sub::Done),
            Some(Handler::CsiDispatch) => self.csi_dispatch(sub, env),
            Some(Handler::DcsDispatch) => self.dcs_dispatch(env),
        }
    }

    fn run_exit(&mut self, exit: Option<Exit>, sub: Sub, env: &mut Env<'_, '_>) -> Flow {
        if sub == Sub::Done {
            return Flow::Done;
        }
        match exit {
            None => Flow::Done,
            Some(Exit::Osc) => self.exit_osc(sub, env),
            Some(Exit::Apc) => self.exit_apc(env),
            Some(Exit::Rename) => self.exit_rename(env),
        }
    }

    fn run_enter(&mut self, enter: Option<Enter>) -> Option<Pending> {
        match enter {
            None => None,
            Some(Enter::Ground) => {
                self.since_ground.clear();
                if self.string_space > STRING_START {
                    self.string_space = STRING_START;
                    self.string.shrink_to(STRING_START);
                }
                self.cancel_timer()
            }
            Some(Enter::Clear) => {
                self.clear();
                self.cancel_timer()
            }
            Some(Enter::Dcs | Enter::Osc | Enter::Apc | Enter::Rename) => {
                self.clear();
                if enter == Some(Enter::Apc) {
                    self.apc_prefix = Some(0);
                }
                self.flags &= !FLAG_LAST;
                self.timer_armed = true;
                Some(Pending::GroundTimer(true))
            }
        }
    }

    /// `event_del(&ictx->ground_timer)`: a cancel only matters while armed.
    fn cancel_timer(&mut self) -> Option<Pending> {
        if self.timer_armed {
            self.timer_armed = false;
            Some(Pending::GroundTimer(false))
        } else {
            None
        }
    }

    /// `input_reset_cell` (`input.c:835-844`).
    fn reset_cell(&mut self) {
        self.cell = InputCell::default();
        self.old_cell = self.cell;
        self.old_cx = 0;
        self.old_cy = 0;
    }

    /// `input_clear` without the timer (`input.c:1195-1211`).
    fn clear(&mut self) {
        self.interm.clear();
        self.param_buf.clear();
        self.string.clear();
        self.string_end = InputEnd::St;
        self.apc_prefix = None;
        self.flags &= !FLAG_DISCARD;
    }

    /// `input_save_state` (`input.c:848-857`).
    fn save_state(&mut self, env: &mut Env<'_, '_>) {
        self.old_cell = self.cell;
        self.old_cx = env.sw.screen.cx;
        self.old_cy = env.sw.screen.cy;
        self.old_mode = env.sw.screen.mode;
    }

    /// `input_restore_state` (`input.c:861-871`).
    fn restore_state(&mut self, env: &mut Env<'_, '_>) {
        self.cell = self.old_cell;
        if self.old_mode.contains(ScreenMode::ORIGIN) {
            env.sw.mode_set(ScreenMode::ORIGIN);
        } else {
            env.sw.mode_clear(ScreenMode::ORIGIN);
        }
        env.sw
            .cursormove(self.old_cx as i32, self.old_cy as i32, false);
    }

    /// `input_stop_utf8` (`input.c:782-792`).
    fn stop_utf8(&mut self, env: &mut Env<'_, '_>) {
        if self.utf8_started {
            let mut rc = Utf8Data::default();
            rc.data[..3].copy_from_slice(b"\xef\xbf\xbd");
            rc.have = 3;
            rc.size = 3;
            rc.width = 1;
            self.cell.cell.data = rc;
            env.sw.collect_add(&self.cell.cell);
        }
        self.utf8_started = false;
    }

    /// `input_print` (`input.c:1228-1249`).
    fn print(&mut self, env: &mut Env<'_, '_>) {
        self.stop_utf8(env);
        let set = if self.cell.set == 0 {
            self.cell.g0set
        } else {
            self.cell.g1set
        };
        if set {
            self.cell
                .cell
                .attr
                .insert(crate::cell::GridAttributes::CHARSET);
        } else {
            self.cell
                .cell
                .attr
                .remove(crate::cell::GridAttributes::CHARSET);
        }
        self.cell.cell.data = Utf8Data::set(self.ch);
        env.sw.collect_add(&self.cell.cell);
        self.last.copy_from(&self.cell.cell.data);
        self.flags |= FLAG_LAST;
        self.cell
            .cell
            .attr
            .remove(crate::cell::GridAttributes::CHARSET);
    }

    /// `input_input` (`input.c:1281-1299`): logical capacity doubles until
    /// `buffer_limit`.
    fn input(&mut self, buffer_limit: usize) {
        if self.state == StateId::TspString {
            if self.string.len() == TSP_APC_LIMIT {
                self.string.clear();
                self.state = StateId::TspDiscard;
            } else {
                self.string.push(self.ch);
            }
            return;
        }
        let mut available = self.string_space;
        while self.string.len() + 1 >= available {
            available *= 2;
            if available > buffer_limit {
                self.flags |= FLAG_DISCARD;
                return;
            }
            self.string_space = available;
        }
        self.string.push(self.ch);
        if self.state == StateId::ApcString && self.apc_prefix == Some(4) {
            self.state = StateId::TspString;
        }
    }

    /// `input_top_bit_set` (`input.c:2857-2892`).
    fn top_bit_set(&mut self, env: &mut Env<'_, '_>) {
        self.flags &= !FLAG_LAST;
        if !self.utf8_started {
            self.utf8_started = true;
            match Utf8Data::open(self.ch) {
                Ok(ud) => self.utf8 = ud,
                Err(()) => self.stop_utf8(env),
            }
            return;
        }
        match self.utf8.append(self.ch) {
            Utf8State::More => return,
            Utf8State::Error => {
                self.stop_utf8(env);
                return;
            }
            Utf8State::Done => {}
        }
        self.utf8_started = false;
        self.cell.cell.data.copy_from(&self.utf8);
        env.sw.collect_add(&self.cell.cell);
        self.last.copy_from(&self.cell.cell.data);
        self.flags |= FLAG_LAST;
    }

    /// The parser cell for tests and the server adapter.
    pub fn cell(&self) -> &GridCell {
        &self.cell.cell
    }
}
