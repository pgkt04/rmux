// Ported from tmux style.c and tmux.h @ 8f25579c
/* $OpenBSD: style.c,v 1.47 2026/06/29 17:08:52 nicm Exp $ */

/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2014 Tiago Cunha <tcunha@users.sourceforge.net>
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
pub enum StyleAlign {
    AbsoluteCentre = 4,
    Default = 0,
    Left = 1,
    Centre = 2,
    Right = 3,
}
impl TryFrom<i32> for StyleAlign {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            4 => Ok(Self::AbsoluteCentre),
            0 => Ok(Self::Default),
            1 => Ok(Self::Left),
            2 => Ok(Self::Centre),
            3 => Ok(Self::Right),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StyleList {
    Off = 0,
    On = 1,
    Focus = 2,
    LeftMarker = 3,
    RightMarker = 4,
}
impl TryFrom<i32> for StyleList {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::On),
            2 => Ok(Self::Focus),
            3 => Ok(Self::LeftMarker),
            4 => Ok(Self::RightMarker),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StyleRangeType {
    Control = 7,
    None = 0,
    Left = 1,
    Right = 2,
    Pane = 3,
    Window = 4,
    Session = 5,
    User = 6,
}
impl TryFrom<i32> for StyleRangeType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            7 => Ok(Self::Control),
            0 => Ok(Self::None),
            1 => Ok(Self::Left),
            2 => Ok(Self::Right),
            3 => Ok(Self::Pane),
            4 => Ok(Self::Window),
            5 => Ok(Self::Session),
            6 => Ok(Self::User),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StyleDefaultType {
    Set = 3,
    Base = 0,
    Push = 1,
    Pop = 2,
}
impl TryFrom<i32> for StyleDefaultType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            3 => Ok(Self::Set),
            0 => Ok(Self::Base),
            1 => Ok(Self::Push),
            2 => Ok(Self::Pop),
            _ => Err(value),
        }
    }
}

mod parser;
mod text;
use crate::{
    cell::{DEFAULT_CELL, GridCell},
    colour::{Colour, ColourParseError, parse_colour},
    hyperlinks::{HyperlinkError, HyperlinkId, HyperlinkRegistry},
};
use rmux_util::bytes::{ByteString, cstr};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Style {
    pub gc: GridCell,
    pub ignore: bool,
    pub dim: u32,
    pub fill: Colour,
    pub align: StyleAlign,
    pub list: StyleList,
    pub range_type: StyleRangeType,
    pub range_argument: u32,
    pub range_string: [u8; 16],
    pub width: i32,
    pub width_percentage: bool,
    pub pad: i32,
    pub default_type: StyleDefaultType,
    pub link: HyperlinkId,
}
impl Default for Style {
    fn default() -> Self {
        Self::from_cell(DEFAULT_CELL)
    }
}
impl Style {
    pub fn from_cell(gc: GridCell) -> Self {
        Self {
            gc,
            ignore: false,
            dim: 0,
            fill: Colour::DEFAULT,
            align: StyleAlign::Default,
            list: StyleList::Off,
            range_type: StyleRangeType::None,
            range_argument: 0,
            range_string: [0; 16],
            width: -1,
            width_percentage: false,
            pad: -1,
            default_type: StyleDefaultType::Base,
            link: HyperlinkId::NONE,
        }
    }
    pub fn option_fallback() -> Self {
        let mut style = Self::default();
        style.gc.us = Colour(0);
        style
    }
    pub fn parse(
        &mut self,
        base: &GridCell,
        bytes: &[u8],
        links: &mut HyperlinkRegistry,
    ) -> Result<(), StyleParseError> {
        parser::parse(self, base, cstr(bytes), links)
    }
    pub fn parse_colour(&mut self, base: &GridCell, bytes: &[u8]) -> Result<(), ColourParseError> {
        *self = Self::from_cell(*base);
        let bytes = cstr(bytes);
        let colour = if bytes.is_empty() {
            Colour::NONE
        } else {
            parse_colour(bytes)?
        };
        self.gc.fg = if colour == Colour::DEFAULT {
            base.fg
        } else {
            colour
        };
        Ok(())
    }
    pub fn overlay_cell(&self, cell: &mut GridCell) {
        if self.gc.fg != Colour::DEFAULT {
            cell.fg = self.gc.fg;
        }
        if self.gc.bg != Colour::DEFAULT {
            cell.bg = self.gc.bg;
        }
        if self.gc.us != Colour::DEFAULT {
            cell.us = self.gc.us;
        }
        cell.attr.insert(self.gc.attr);
    }
    pub fn link_uri<'a>(&self, links: &'a HyperlinkRegistry) -> Option<&'a [u8]> {
        links.style_uri(self.link).map(|link| link.uri())
    }
    pub fn write_text(&self, links: &HyperlinkRegistry, out: &mut Vec<u8>) {
        text::write(self, links, out);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StyleParseError {
    InvalidToken {
        offset: usize,
    },
    Hyperlink {
        offset: usize,
        error: HyperlinkError,
    },
}
impl std::fmt::Display for StyleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidToken { offset } => write!(f, "invalid style token at byte {offset}"),
            Self::Hyperlink { offset, error } => write!(f, "style token at byte {offset}: {error}"),
        }
    }
}
impl std::error::Error for StyleParseError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StyleRange {
    pub range_type: StyleRangeType,
    pub argument: u32,
    pub string: [u8; 16],
    pub start: u32,
    pub end: u32,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyleRanges(pub Vec<StyleRange>);
impl StyleRanges {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn push(&mut self, range: StyleRange) {
        self.0.push(range);
    }
    pub fn get_range(&self, x: u32) -> Option<&StyleRange> {
        self.0.iter().find(|r| r.start <= x && x < r.end)
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyleLineEntry {
    pub expanded: ByteString,
    pub ranges: StyleRanges,
}
