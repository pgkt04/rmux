// Ported from tmux tmux.h @ 8f25579c
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
pub struct PaneFlags(pub u32);
impl PaneFlags {
    pub const REDRAW: Self = Self(1);
    pub const DROP: Self = Self(2);
    pub const FOCUSED: Self = Self(4);
    pub const VISITED: Self = Self(8);
    pub const ZOOMED: Self = Self(16);
    pub const NEWSTATUS: Self = Self(32);
    pub const INPUTOFF: Self = Self(64);
    pub const CHANGED: Self = Self(128);
    pub const EXITED: Self = Self(256);
    pub const STATUSREADY: Self = Self(512);
    pub const STATUSDRAWN: Self = Self(1024);
    pub const EMPTY: Self = Self(2048);
    pub const STYLECHANGED: Self = Self(4096);
    pub const THEMECHANGED: Self = Self(8192);
    pub const UNSEENCHANGES: Self = Self(16384);
    pub const REDRAWSCROLLBAR: Self = Self(32768);
    pub const DESTROYED: Self = Self(65536);
    pub const CMDRUNNING: Self = Self(131072);
    pub const ACTIVITY: Self = Self(262144);
    pub const CLOSEONCLICK: Self = Self(524288);
    pub const CAPTUREALLKEYS: Self = Self(1048576);
    pub const FLOATOVERZOOM: Self = Self(2097152);
    pub const CLOSEONCANCEL: Self = Self(4194304);
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
impl std::ops::BitOr for PaneFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for PaneFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for PaneFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct WindowFlags(pub u32);
impl WindowFlags {
    pub const BELL: Self = Self(1);
    pub const ACTIVITY: Self = Self(2);
    pub const SILENCE: Self = Self(4);
    pub const ZOOMED: Self = Self(8);
    pub const WASZOOMED: Self = Self(16);
    pub const RESIZE: Self = Self(32);
    pub const ALERTFLAGS: Self = Self(7);
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
impl std::ops::BitOr for WindowFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for WindowFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for WindowFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct WinlinkFlags(pub u32);
impl WinlinkFlags {
    pub const BELL: Self = Self(1);
    pub const ACTIVITY: Self = Self(2);
    pub const SILENCE: Self = Self(4);
    pub const ALERTFLAGS: Self = Self(7);
    pub const VISITED: Self = Self(8);
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
impl std::ops::BitOr for WinlinkFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for WinlinkFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for WinlinkFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SessionFlags(pub u32);
impl SessionFlags {
    pub const ALERTED: Self = Self(1);
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
impl std::ops::BitOr for SessionFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for SessionFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for SessionFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WindowSizePolicy {
    Largest = 0,
    Smallest = 1,
    Manual = 2,
    Latest = 3,
}
impl TryFrom<i32> for WindowSizePolicy {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Largest),
            1 => Ok(Self::Smallest),
            2 => Ok(Self::Manual),
            3 => Ok(Self::Latest),
            _ => Err(value),
        }
    }
}

pub mod alerts;
pub mod monitor;
pub mod names;
pub mod pane;
pub mod pane_input;
pub mod paste;
pub mod resize;
pub mod session;
pub mod spawn;
pub mod state;
pub mod store_runtime;
pub mod view;
pub mod window;
pub mod winlink;
pub use session::{
    DetachOutcome, SelectOutcome, SessionCreate, SessionEffect, SessionTimerRequest,
};
pub use state::{
    ModelEffect, ModelError, Pane, Server, Session, SessionGroup, TimerRequest, Window, Winlink,
};
