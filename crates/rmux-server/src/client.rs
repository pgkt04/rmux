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
