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
pub enum ScreenCursorStyle {
    Bar = 3,
    Default = 0,
    Block = 1,
    Underline = 2,
}
impl TryFrom<i32> for ScreenCursorStyle {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            3 => Ok(Self::Bar),
            0 => Ok(Self::Default),
            1 => Ok(Self::Block),
            2 => Ok(Self::Underline),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProgressBarState {
    Hidden = 0,
    Normal = 1,
    Error = 2,
    Indeterminate = 3,
    Paused = 4,
}
impl TryFrom<i32> for ProgressBarState {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Hidden),
            1 => Ok(Self::Normal),
            2 => Ok(Self::Error),
            3 => Ok(Self::Indeterminate),
            4 => Ok(Self::Paused),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxLines {
    None = 6,
    Default = -1,
    Single = 0,
    Double = 1,
    Heavy = 2,
    Simple = 3,
    Rounded = 4,
    Padded = 5,
}
impl TryFrom<i32> for BoxLines {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            6 => Ok(Self::None),
            -1 => Ok(Self::Default),
            0 => Ok(Self::Single),
            1 => Ok(Self::Double),
            2 => Ok(Self::Heavy),
            3 => Ok(Self::Simple),
            4 => Ok(Self::Rounded),
            5 => Ok(Self::Padded),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PaneLines {
    Rounded = 7,
    Single = 0,
    Double = 1,
    Heavy = 2,
    Simple = 3,
    Number = 4,
    Spaces = 5,
    None = 6,
}
impl TryFrom<i32> for PaneLines {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            7 => Ok(Self::Rounded),
            0 => Ok(Self::Single),
            1 => Ok(Self::Double),
            2 => Ok(Self::Heavy),
            3 => Ok(Self::Simple),
            4 => Ok(Self::Number),
            5 => Ok(Self::Spaces),
            6 => Ok(Self::None),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ScreenMode(pub u32);
impl ScreenMode {
    pub const CURSOR: Self = Self(1);
    pub const INSERT: Self = Self(2);
    pub const KCURSOR: Self = Self(4);
    pub const KKEYPAD: Self = Self(8);
    pub const WRAP: Self = Self(16);
    pub const MOUSE_STANDARD: Self = Self(32);
    pub const MOUSE_BUTTON: Self = Self(64);
    pub const CURSOR_BLINKING: Self = Self(128);
    pub const MOUSE_UTF8: Self = Self(256);
    pub const MOUSE_SGR: Self = Self(512);
    pub const BRACKETPASTE: Self = Self(1024);
    pub const FOCUSON: Self = Self(2048);
    pub const MOUSE_ALL: Self = Self(4096);
    pub const ORIGIN: Self = Self(8192);
    pub const CRLF: Self = Self(16384);
    pub const KEYS_EXTENDED: Self = Self(32768);
    pub const CURSOR_VERY_VISIBLE: Self = Self(65536);
    pub const CURSOR_BLINKING_SET: Self = Self(131072);
    pub const KEYS_EXTENDED_2: Self = Self(262144);
    pub const THEME_UPDATES: Self = Self(524288);
    pub const SYNC: Self = Self(1048576);
    pub const ALL_MODES: Self = Self(16777215);
    pub const ALL_MOUSE_MODES: Self = Self(4192);
    pub const MOTION_MOUSE_MODES: Self = Self(4160);
    pub const CURSOR_MODES: Self = Self(65665);
    pub const EXTENDED_KEY_MODES: Self = Self(294912);
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
impl std::ops::BitOr for ScreenMode {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for ScreenMode {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for ScreenMode {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BorderCell {
    Inside = 0,
    Ud = 1,
    Lr = 2,
    Rd = 3,
    Ld = 4,
    Ru = 5,
    Lu = 6,
    Lrd = 7,
    Lru = 8,
    Urd = 9,
    Uld = 10,
    Lrud = 11,
    None = 12,
    Scrollbar = 13,
}
impl TryFrom<i32> for BorderCell {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Inside),
            1 => Ok(Self::Ud),
            2 => Ok(Self::Lr),
            3 => Ok(Self::Rd),
            4 => Ok(Self::Ld),
            5 => Ok(Self::Ru),
            6 => Ok(Self::Lu),
            7 => Ok(Self::Lrd),
            8 => Ok(Self::Lru),
            9 => Ok(Self::Urd),
            10 => Ok(Self::Uld),
            11 => Ok(Self::Lrud),
            12 => Ok(Self::None),
            13 => Ok(Self::Scrollbar),
            _ => Err(value),
        }
    }
}

pub mod write;
