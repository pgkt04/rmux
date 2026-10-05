// Ported from tmux grid.c and tmux.h @ 8f25579c
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
pub struct GridAttributes(pub u16);
impl GridAttributes {
    pub const BRIGHT: Self = Self(1);
    pub const DIM: Self = Self(2);
    pub const UNDERSCORE: Self = Self(4);
    pub const BLINK: Self = Self(8);
    pub const REVERSE: Self = Self(16);
    pub const HIDDEN: Self = Self(32);
    pub const ITALICS: Self = Self(64);
    pub const CHARSET: Self = Self(128);
    pub const STRIKETHROUGH: Self = Self(256);
    pub const UNDERSCORE_2: Self = Self(512);
    pub const UNDERSCORE_3: Self = Self(1024);
    pub const UNDERSCORE_4: Self = Self(2048);
    pub const UNDERSCORE_5: Self = Self(4096);
    pub const OVERLINE: Self = Self(8192);
    pub const NOATTR: Self = Self(16384);
    pub const ALL_UNDERSCORE: Self = Self(7684);
    pub const fn bits(self) -> u16 {
        self.0
    }
    pub const fn from_bits_retain(bits: u16) -> Self {
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
impl std::ops::BitOr for GridAttributes {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for GridAttributes {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for GridAttributes {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct GridCellFlags(pub u8);
impl GridCellFlags {
    pub const FG256: Self = Self(1);
    pub const BG256: Self = Self(2);
    pub const PADDING: Self = Self(4);
    pub const EXTENDED: Self = Self(8);
    pub const SELECTED: Self = Self(16);
    pub const NOPALETTE: Self = Self(32);
    pub const CLEARED: Self = Self(64);
    pub const TAB: Self = Self(128);
    pub const fn bits(self) -> u8 {
        self.0
    }
    pub const fn from_bits_retain(bits: u8) -> Self {
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
impl std::ops::BitOr for GridCellFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for GridCellFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for GridCellFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use crate::colour::Colour;
use crate::hyperlinks::HyperlinkId;
use rmux_util::utf8::Utf8Data;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridCell {
    pub data: Utf8Data,
    pub attr: GridAttributes,
    pub flags: GridCellFlags,
    pub fg: Colour,
    pub bg: Colour,
    pub us: Colour,
    pub link: HyperlinkId,
}

pub const DEFAULT_CELL: GridCell = GridCell {
    data: Utf8Data {
        data: {
            let mut bytes = [0; 32];
            bytes[0] = b' ';
            bytes
        },
        have: 0,
        size: 1,
        width: 1,
    },
    attr: GridAttributes(0),
    flags: GridCellFlags(0),
    fg: Colour::DEFAULT,
    bg: Colour::DEFAULT,
    us: Colour::DEFAULT,
    link: HyperlinkId::NONE,
};

impl Default for GridCell {
    fn default() -> Self {
        DEFAULT_CELL
    }
}
