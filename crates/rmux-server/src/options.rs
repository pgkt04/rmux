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
pub enum OptionsTableType {
    Command = 6,
    String = 0,
    Number = 1,
    Key = 2,
    Colour = 3,
    Flag = 4,
    Choice = 5,
}
impl TryFrom<i32> for OptionsTableType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            6 => Ok(Self::Command),
            0 => Ok(Self::String),
            1 => Ok(Self::Number),
            2 => Ok(Self::Key),
            3 => Ok(Self::Colour),
            4 => Ok(Self::Flag),
            5 => Ok(Self::Choice),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct OptionsScope(pub u32);
impl OptionsScope {
    pub const NONE: Self = Self(0);
    pub const SERVER: Self = Self(1);
    pub const SESSION: Self = Self(2);
    pub const WINDOW: Self = Self(4);
    pub const PANE: Self = Self(8);
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
impl std::ops::BitOr for OptionsScope {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for OptionsScope {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for OptionsScope {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct OptionsTableFlags(pub u32);
impl OptionsTableFlags {
    pub const ARRAY: Self = Self(1);
    pub const HOOK: Self = Self(2);
    pub const STYLE: Self = Self(4);
    pub const COLOUR: Self = Self(8);
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
impl std::ops::BitOr for OptionsTableFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for OptionsTableFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for OptionsTableFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PaneBorderIndicator {
    Off = 0,
    Colour = 1,
    Arrows = 2,
    Both = 3,
}
impl TryFrom<i32> for PaneBorderIndicator {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Colour),
            2 => Ok(Self::Arrows),
            3 => Ok(Self::Both),
            _ => Err(value),
        }
    }
}

pub mod environment;
