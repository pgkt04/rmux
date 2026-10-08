// Ported from tmux tmux.h, server-client.c @ 8f25579c
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
pub mod dispatch;
pub mod exit;
pub mod flags;
pub mod input_requests;
pub mod keys;
pub mod lifecycle;
pub mod mouse;
pub mod print;
pub mod registry;
pub mod theme;
pub mod tick;
pub mod tty_io;

use crate::ids::*;
use crate::options::environment::Environment;
use crate::server::protocol::ProtocolMessageKind;
use rmux_emu::colour::ClientTheme;
use rmux_emu::screen::ProgressBar;
use rmux_sys::{OwnedFd, ProcessId};
use rmux_tty::features::TtyFeatures;
use rmux_tty::tty::Tty;
use rmux_util::key::{KeyCode, MouseEvent, SpecialKey};

pub use mouse::{ClickState, MouseDragAction, MouseDragState, MouseTarget, ResolvedMouseEvent};

/// `COLOUR_THEME_COUNT` (`tmux.h`): the ten theme colour slots.
pub const COLOUR_THEME_COUNT: usize = 10;
/// `CLIENT_PASTE_TIME_LIMIT` (`tmux.h:2126`), seconds.
pub const PASTE_TIME_LIMIT: i64 = 5;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClientExitType {
    Detach = 2,
    Return = 0,
    Shutdown = 1,
}
impl TryFrom<i32> for ClientExitType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            2 => Ok(Self::Detach),
            0 => Ok(Self::Return),
            1 => Ok(Self::Shutdown),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ClientFlags(pub u64);
impl ClientFlags {
    pub const TERMINAL: Self = Self(1);
    pub const LOGIN: Self = Self(2);
    pub const EXIT: Self = Self(4);
    pub const REDRAWWINDOW: Self = Self(8);
    pub const REDRAWSTATUS: Self = Self(16);
    pub const REPEAT: Self = Self(32);
    pub const SUSPENDED: Self = Self(64);
    pub const ATTACHED: Self = Self(128);
    pub const EXITED: Self = Self(256);
    pub const DEAD: Self = Self(512);
    pub const REDRAWBORDERS: Self = Self(1024);
    pub const READONLY: Self = Self(2048);
    pub const NOSTARTSERVER: Self = Self(4096);
    pub const CONTROL: Self = Self(8192);
    pub const CONTROLCONTROL: Self = Self(16384);
    pub const FOCUSED: Self = Self(32768);
    pub const UTF8: Self = Self(65536);
    pub const IGNORESIZE: Self = Self(131072);
    pub const IDENTIFIED: Self = Self(262144);
    pub const STATUSFORCE: Self = Self(524288);
    pub const DOUBLECLICK: Self = Self(1048576);
    pub const TRIPLECLICK: Self = Self(2097152);
    pub const SIZECHANGED: Self = Self(4194304);
    pub const STATUSOFF: Self = Self(8388608);
    pub const REDRAWSTATUSALWAYS: Self = Self(16777216);
    pub const CONTROL_NOOUTPUT: Self = Self(67108864);
    pub const DEFAULTSOCKET: Self = Self(134217728);
    pub const STARTSERVER: Self = Self(268435456);
    pub const REDRAWMENU: Self = Self(536870912);
    pub const NOFORK: Self = Self(1073741824);
    pub const REDRAWSCROLLBARS: Self = Self(2147483648);
    pub const CONTROL_PAUSEAFTER: Self = Self(4294967296);
    pub const CONTROL_WAITEXIT: Self = Self(8589934592);
    pub const WINDOWSIZECHANGED: Self = Self(17179869184);
    pub const CONTROL_NEWLAYOUTS: Self = Self(34359738368);
    pub const BRACKETPASTING: Self = Self(68719476736);
    pub const ASSUMEPASTING: Self = Self(137438953472);
    pub const WRITE_ACK: Self = Self(274877906944);
    pub const NO_DETACH_ON_DESTROY: Self = Self(549755813888);
    pub const CONTROL_DISCARD: Self = Self(1099511627776);
    pub const ALLREDRAWFLAGS: Self = Self(553649176);
    pub const UNATTACHEDFLAGS: Self = Self(580);
    pub const NODETACHFLAGS: Self = Self(516);
    pub const NOSIZEFLAGS: Self = Self(580);
    pub const fn bits(self) -> u64 {
        self.0
    }
    pub const fn from_bits_retain(bits: u64) -> Self {
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
impl std::ops::BitOr for ClientFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for ClientFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for ClientFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// `struct key_event` (`tmux.h:1727-1735`).
#[derive(Clone, Debug)]
pub struct KeyEvent {
    pub client: Option<ClientId>,
    pub key: KeyCode,
    pub mouse: MouseEvent,
    pub target: MouseTarget,
    pub paste: Option<Vec<u8>>,
}
impl KeyEvent {
    pub fn new(key: KeyCode) -> Self {
        Self {
            client: None,
            key,
            mouse: MouseEvent::default(),
            target: MouseTarget::default(),
            paste: None,
        }
    }
    pub fn with_mouse(key: KeyCode, mouse: MouseEvent) -> Self {
        Self {
            mouse,
            ..Self::new(key)
        }
    }
    pub fn resolved(&self) -> ResolvedMouseEvent {
        ResolvedMouseEvent {
            event: self.mouse,
            target: self.target,
        }
    }
}

/// `server_client_create` failure during partial setup (spec 3.1).
#[derive(Debug)]
pub enum ClientCreateError {
    Arena(ArenaError),
    Loop(crate::server::event_loop::LoopError),
    Protocol(crate::server::protocol::ProtocolError),
}
impl From<ArenaError> for ClientCreateError {
    fn from(e: ArenaError) -> Self {
        Self::Arena(e)
    }
}
impl From<crate::server::protocol::ProtocolError> for ClientCreateError {
    fn from(e: crate::server::protocol::ProtocolError) -> Self {
        Self::Protocol(e)
    }
}
impl From<crate::server::event_loop::LoopError> for ClientCreateError {
    fn from(e: crate::server::event_loop::LoopError) -> Self {
        Self::Loop(e)
    }
}
impl std::fmt::Display for ClientCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arena(e) => write!(f, "{e}"),
            Self::Loop(e) => write!(f, "{e}"),
            Self::Protocol(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for ClientCreateError {}

/// `struct client` (`tmux.h:2212-2369`).
pub struct Client {
    pub name: Option<Vec<u8>>,
    pub peer: Option<PeerId>,
    pub user: Option<Vec<u8>>,
    pub control: Option<crate::control::ControlState>,
    pub pause_age: u32,
    pub pid: Option<ProcessId>,
    pub fd: Option<OwnedFd>,
    pub out_fd: Option<OwnedFd>,
    pub retval: i32,
    pub creation_time: (i64, i64),
    pub activity_time: (i64, i64),
    pub last_activity_time: (i64, i64),
    pub environ: Environment,
    pub title: Option<Vec<u8>>,
    pub path: Option<Vec<u8>>,
    pub cwd: Option<Vec<u8>>,
    pub progress_bar: ProgressBar,
    pub term_name: Option<Vec<u8>>,
    pub term_features: TtyFeatures,
    pub term_type: Option<Vec<u8>>,
    pub term_caps: Vec<Vec<u8>>,
    pub ttyname: Option<Vec<u8>>,
    /// `c->tty`; present once `tty_init` succeeded at `IDENTIFY_DONE`.
    pub tty: Option<Tty>,
    /// `tty.sx`/`tty.sy` before a tty exists (`server-client.c:188-189`).
    pub tty_sx: u32,
    pub tty_sy: u32,
    /// `tty->event_in`/`event_out` registration (`tty.c:tty_start_tty`).
    pub tty_token: Option<EventToken>,
    /// `tty->timer`, `start_timer`, `clipboard_timer`, `key_timer`.
    pub tty_timers: tty_io::TtyTimers,
    pub tsp: crate::tsp::client::ClientTspState,
    pub written: usize,
    pub discarded: usize,
    pub redraw: usize,
    pub redraw_scene: Option<crate::ui::redraw::RedrawScene>,
    pub repeat_timer: Option<TimerId>,
    pub click: ClickState,
    pub exit_timer: Option<TimerId>,
    pub drag: MouseDragState,
    pub status: crate::ui::status::StatusLine,
    pub cycle_timer: Option<TimerId>,
    pub theme: ClientTheme,
    pub input_requests: input_requests::InputRequests,
    pub flags: ClientFlags,
    pub exit_type: ClientExitType,
    pub exit_msgtype: ProtocolMessageKind,
    pub exit_session: Option<Vec<u8>>,
    pub exit_message: Option<Vec<u8>>,
    pub(crate) exit_pending: bool,
    pub keytable: Option<KeyTableId>,
    pub last_key: KeyCode,
    pub paste_time: i64,
    pub message: crate::ui::status::StatusMessage,
    pub prompt: crate::ui::prompt::PromptSlot,
    pub session: Option<SessionId>,
    pub last_session: Option<SessionId>,
    pub theme_colours: [i32; COLOUR_THEME_COUNT],
    pub pan_window: Option<WindowId>,
    pub pan_ox: u32,
    pub pan_oy: u32,
    pub source_file_depth: u32,
    pub clipboard_panes: Vec<u32>,
    /// `server_client_free` pending (`server-client.c:457-465`).
    pub free_scheduled: Option<TimerId>,
}

impl Client {
    /// The field values of `server_client_create` (`server-client.c:158-206`)
    /// that need no server: timers, the key table and the peer are added by
    /// `lifecycle::create`.
    pub fn new(peer: Option<PeerId>, now: (i64, i64)) -> Self {
        Self {
            name: None,
            peer,
            user: None,
            control: None,
            pause_age: 0,
            pid: None,
            fd: None,
            out_fd: None,
            retval: 0,
            creation_time: now,
            activity_time: now,
            last_activity_time: (0, 0),
            environ: Environment::new(),
            title: None,
            path: None,
            cwd: None,
            progress_bar: ProgressBar {
                state: rmux_emu::screen::ProgressBarState::Hidden,
                progress: 0,
            },
            term_name: None,
            term_features: TtyFeatures::default(),
            term_type: None,
            term_caps: Vec::new(),
            ttyname: None,
            tty: None,
            tty_sx: 80,
            tty_sy: 24,
            written: 0,
            discarded: 0,
            redraw: 0,
            tty_token: None,
            tty_timers: tty_io::TtyTimers::default(),
            tsp: crate::tsp::client::ClientTspState::default(),
            redraw_scene: None,
            repeat_timer: None,
            click: ClickState::default(),
            exit_timer: None,
            drag: MouseDragState::default(),
            status: crate::ui::status::StatusLine::default(),
            cycle_timer: None,
            theme: ClientTheme::Unknown,
            input_requests: input_requests::InputRequests::default(),
            flags: ClientFlags::FOCUSED,
            exit_type: ClientExitType::Return,
            exit_msgtype: ProtocolMessageKind::Exit,
            exit_session: None,
            exit_message: None,
            exit_pending: false,
            keytable: None,
            last_key: KeyCode(SpecialKey::NONE),
            paste_time: 0,
            message: crate::ui::status::StatusMessage::default(),
            prompt: crate::ui::prompt::PromptSlot::default(),
            session: None,
            last_session: None,
            theme_colours: [8; COLOUR_THEME_COUNT],
            pan_window: None,
            pan_ox: 0,
            pan_oy: 0,
            source_file_depth: 0,
            clipboard_panes: Vec::new(),
            free_scheduled: None,
        }
    }

    pub fn is_control(&self) -> bool {
        self.flags.intersects(ClientFlags::CONTROL)
    }
    pub fn is_dead(&self) -> bool {
        self.flags.intersects(ClientFlags::DEAD)
    }
    /// `c->tty.sx`, `c->tty.sy`.
    pub fn tty_size(&self) -> (u32, u32) {
        match &self.tty {
            Some(tty) => tty.size(),
            None => (self.tty_sx, self.tty_sy),
        }
    }
    /// `c->name` as shown in formats and logs; empty before identify.
    pub fn name_bytes(&self) -> &[u8] {
        self.name.as_deref().unwrap_or(b"")
    }
}
