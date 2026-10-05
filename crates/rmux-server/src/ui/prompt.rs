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
pub enum PromptType {
    Command = 0,
    Search = 1,
    Invalid = 255,
}
impl TryFrom<i32> for PromptType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Command),
            1 => Ok(Self::Search),
            255 => Ok(Self::Invalid),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PromptResult {
    Close = 1,
    Continue = 0,
}
impl TryFrom<i32> for PromptResult {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            1 => Ok(Self::Close),
            0 => Ok(Self::Continue),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PromptKeyResult {
    Move = 3,
    NotHandled = 0,
    Handled = 1,
    Close = 2,
}
impl TryFrom<i32> for PromptKeyResult {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            3 => Ok(Self::Move),
            0 => Ok(Self::NotHandled),
            1 => Ok(Self::Handled),
            2 => Ok(Self::Close),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PromptFlags(pub u32);
impl PromptFlags {
    pub const SINGLE: Self = Self(1);
    pub const NUMERIC: Self = Self(2);
    pub const INCREMENTAL: Self = Self(4);
    pub const NOFORMAT: Self = Self(8);
    pub const KEY: Self = Self(16);
    pub const ACCEPT: Self = Self(32);
    pub const QUOTENEXT: Self = Self(64);
    pub const BSPACE_EXIT: Self = Self(128);
    pub const NOFREEZE: Self = Self(256);
    pub const COMMANDMODE: Self = Self(512);
    pub const ISPANE: Self = Self(1024);
    pub const ISMODE: Self = Self(2048);
    pub const EDITARROWS: Self = Self(4096);
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
impl std::ops::BitOr for PromptFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for PromptFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for PromptFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}
