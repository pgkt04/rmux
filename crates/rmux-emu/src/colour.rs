// Ported from tmux colour.c and tmux.h @ 8f25579c
/* $OpenBSD: colour.c,v 1.35 2026/07/06 14:29:10 nicm Exp $ */

/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2016 Avi Halachmi <avihpit@yahoo.com>
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
pub enum ColourTheme {
    Magenta = 9,
    Black = 0,
    White = 1,
    LightGrey = 2,
    DarkGrey = 3,
    Green = 4,
    Yellow = 5,
    Red = 6,
    Blue = 7,
    Cyan = 8,
}
impl TryFrom<i32> for ColourTheme {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            9 => Ok(Self::Magenta),
            0 => Ok(Self::Black),
            1 => Ok(Self::White),
            2 => Ok(Self::LightGrey),
            3 => Ok(Self::DarkGrey),
            4 => Ok(Self::Green),
            5 => Ok(Self::Yellow),
            6 => Ok(Self::Red),
            7 => Ok(Self::Blue),
            8 => Ok(Self::Cyan),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClientTheme {
    Dark = 2,
    Unknown = 0,
    Light = 1,
}
impl TryFrom<i32> for ClientTheme {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            2 => Ok(Self::Dark),
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Light),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ColourFlags(pub u32);
impl ColourFlags {
    pub const _256: Self = Self(16777216);
    pub const RGB: Self = Self(33554432);
    pub const THEME: Self = Self(67108864);
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
impl std::ops::BitOr for ColourFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for ColourFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for ColourFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

mod scanner;
mod tables;
use rmux_util::{bytes::cstr, strtonum::strtonum};
use std::io::Write;
pub use tables::X11_NAMES;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Colour(pub i32);
impl Colour {
    pub const NONE: Self = Self(-1);
    pub const DEFAULT: Self = Self(8);
    pub const TERMINAL: Self = Self(9);
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }
    pub const fn raw(self) -> i32 {
        self.0
    }
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(ColourFlags::RGB.bits() as i32 | ((r as i32) << 16) | ((g as i32) << 8) | b as i32)
    }
    pub const fn is_default(self) -> bool {
        self.0 == 8 || self.0 == 9
    }
    pub fn split_rgb(self) -> (u8, u8, u8) {
        ((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8)
    }
    fn tagged(self, flag: ColourFlags) -> bool {
        self.0 & flag.bits() as i32 != 0
    }
    pub fn force_rgb(self) -> Option<Self> {
        if self == Self::NONE {
            return None;
        }
        if self.tagged(ColourFlags::RGB) {
            Some(self)
        } else if self.tagged(ColourFlags::_256) || (0..=7).contains(&self.0) {
            Some(indexed_to_rgb(self))
        } else if (90..=97).contains(&self.0) {
            Some(indexed_to_rgb(Self(8 + self.0 - 90)))
        } else {
            None
        }
    }
    pub fn dim(self, percentage: u32) -> Option<Self> {
        if percentage == 0 || self.is_default() || self.tagged(ColourFlags::THEME) {
            return Some(self);
        }
        if percentage >= 100 {
            return Some(Self::rgb(0, 0, 0));
        }
        let (r, g, b) = self.force_rgb()?.split_rgb();
        let dim = |c: u8| (u32::from(c) * (100 - percentage) / 100) as u8;
        Some(Self::rgb(dim(r), dim(g), dim(b)))
    }
    pub fn theme(self) -> ClientTheme {
        if self == Self::NONE {
            return ClientTheme::Unknown;
        }
        if self.tagged(ColourFlags::RGB) {
            let (r, g, b) = self.split_rgb();
            return if u32::from(r) + u32::from(g) + u32::from(b) > 382 {
                ClientTheme::Light
            } else {
                ClientTheme::Dark
            };
        }
        if self.tagged(ColourFlags::_256) {
            return indexed_to_rgb(self).theme();
        }
        match self.0 {
            0 | 90 => ClientTheme::Dark,
            7 | 97 => ClientTheme::Light,
            1..=6 | 91..=96 => self.force_rgb().unwrap().theme(),
            _ => ClientTheme::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColourParseError;
impl std::fmt::Display for ColourParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid colour")
    }
}
impl std::error::Error for ColourParseError {}

const BASIC: &[&[u8]; 8] = &[
    b"black", b"red", b"green", b"yellow", b"blue", b"magenta", b"cyan", b"white",
];
const BRIGHT: &[&[u8]; 8] = &[
    b"brightblack",
    b"brightred",
    b"brightgreen",
    b"brightyellow",
    b"brightblue",
    b"brightmagenta",
    b"brightcyan",
    b"brightwhite",
];
const THEMES: &[(&[u8], &str, &str, i32)] = &[
    (b"themeblack", "dark-theme-black", "light-theme-black", 0),
    (b"themewhite", "dark-theme-white", "light-theme-white", 7),
    (
        b"themelightgrey",
        "dark-theme-light-grey",
        "light-theme-light-grey",
        7,
    ),
    (
        b"themedarkgrey",
        "dark-theme-dark-grey",
        "light-theme-dark-grey",
        0,
    ),
    (b"themegreen", "dark-theme-green", "light-theme-green", 2),
    (b"themeyellow", "dark-theme-yellow", "light-theme-yellow", 3),
    (b"themered", "dark-theme-red", "light-theme-red", 1),
    (b"themeblue", "dark-theme-blue", "light-theme-blue", 4),
    (b"themecyan", "dark-theme-cyan", "light-theme-cyan", 6),
    (
        b"thememagenta",
        "dark-theme-magenta",
        "light-theme-magenta",
        5,
    ),
];
pub fn theme_option(slot: u32, theme: ClientTheme) -> Option<&'static str> {
    THEMES.get(slot as usize).map(|row| {
        if theme == ClientTheme::Light {
            row.2
        } else {
            row.1
        }
    })
}
pub fn theme_terminal_colour(slot: u32) -> Colour {
    Colour(THEMES.get(slot as usize).map_or(8, |row| row.3))
}
pub fn indexed_to_rgb(raw: Colour) -> Colour {
    Colour(tables::INDEXED_RGB[(raw.0 & 255) as usize] | ColourFlags::RGB.bits() as i32)
}
pub fn indexed_to_16(raw: Colour) -> u8 {
    tables::INDEXED_16[(raw.0 & 255) as usize]
}
pub fn find_rgb(r: u8, g: u8, b: u8) -> Colour {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    fn quant(v: i32) -> i32 {
        if v < 48 {
            0
        } else if v < 114 {
            1
        } else {
            (v - 35) / 40
        }
    }
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let (qr, qg, qb) = (quant(r), quant(g), quant(b));
    let (cr, cg, cb) = (
        LEVELS[qr as usize],
        LEVELS[qg as usize],
        LEVELS[qb as usize],
    );
    let cube = 16 + 36 * qr + 6 * qg + qb;
    if (cr, cg, cb) == (r, g, b) {
        return Colour(cube | ColourFlags::_256.bits() as i32);
    }
    let avg = (r + g + b) / 3;
    let gi = if avg > 238 { 23 } else { (avg - 3) / 10 };
    let grey = 8 + 10 * gi;
    let distance = |a: i32, c: i32, d: i32| (a - r).pow(2) + (c - g).pow(2) + (d - b).pow(2);
    Colour(
        (if distance(grey, grey, grey) < distance(cr, cg, cb) {
            232 + gi
        } else {
            cube
        }) | ColourFlags::_256.bits() as i32,
    )
}
pub fn parse_colour(bytes: &[u8]) -> Result<Colour, ColourParseError> {
    let bytes = cstr(bytes);
    if bytes.len() == 7 && bytes[0] == b'#' {
        if !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
            return Err(ColourParseError);
        }
        let value = u32::from_str_radix(std::str::from_utf8(&bytes[1..]).unwrap(), 16).unwrap();
        return Ok(Colour(value as i32 | ColourFlags::RGB.bits() as i32));
    }
    for prefix in [b"colour".as_slice(), b"color"] {
        if bytes
            .get(..prefix.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
        {
            return strtonum(&bytes[prefix.len()..], 0, 255)
                .map(|n| Colour(n as i32 | ColourFlags::_256.bits() as i32))
                .map_err(|_| ColourParseError);
        }
    }
    if bytes.eq_ignore_ascii_case(b"default") {
        return Ok(Colour::DEFAULT);
    }
    if bytes.eq_ignore_ascii_case(b"terminal") {
        return Ok(Colour::TERMINAL);
    }
    if let Some(n) = THEMES
        .iter()
        .position(|row| bytes.eq_ignore_ascii_case(row.0))
    {
        return Ok(Colour(n as i32 | ColourFlags::THEME.bits() as i32));
    }
    for (n, name) in BASIC.iter().enumerate() {
        if bytes.eq_ignore_ascii_case(name) || bytes == [b'0' + n as u8] {
            return Ok(Colour(n as i32));
        }
    }
    for (n, name) in BRIGHT.iter().enumerate() {
        if bytes.eq_ignore_ascii_case(name) || bytes == [b'9', b'0' + n as u8] {
            return Ok(Colour(90 + n as i32));
        }
    }
    colour_by_name(bytes)
}
pub fn colour_by_name(bytes: &[u8]) -> Result<Colour, ColourParseError> {
    let bytes = cstr(bytes);
    if bytes
        .get(..4)
        .is_some_and(|s| s.eq_ignore_ascii_case(b"grey") || s.eq_ignore_ascii_case(b"gray"))
    {
        if bytes.len() == 4 {
            return Ok(Colour::rgb(190, 190, 190));
        }
        let n = strtonum(&bytes[4..], 0, 100).map_err(|_| ColourParseError)?;
        let channel = (2.55 * n as f64).round() as u8;
        return Ok(Colour::rgb(channel, channel, channel));
    }
    X11_NAMES
        .iter()
        .find(|(name, _)| bytes.eq_ignore_ascii_case(name))
        .map(|row| Colour(row.1 | ColourFlags::RGB.bits() as i32))
        .ok_or(ColourParseError)
}
pub fn parse_x11_colour(bytes: &[u8]) -> Result<Colour, ColourParseError> {
    scanner::parse(cstr(bytes))
}
pub fn write_colour(colour: Colour, out: &mut Vec<u8>) {
    if colour == Colour::NONE {
        out.extend_from_slice(b"none");
    } else if colour.tagged(ColourFlags::THEME) {
        out.extend_from_slice(
            THEMES
                .get((colour.0 & 255) as usize)
                .map_or(b"invalid".as_slice(), |row| row.0),
        );
    } else if colour.tagged(ColourFlags::RGB) {
        let (r, g, b) = colour.split_rgb();
        write!(out, "#{r:02x}{g:02x}{b:02x}").unwrap();
    } else if colour.tagged(ColourFlags::_256) {
        write!(out, "colour{}", colour.0 & 255).unwrap();
    } else {
        out.extend_from_slice(match colour.0 {
            0..=7 => BASIC[colour.0 as usize],
            8 => b"default",
            9 => b"terminal",
            90..=97 => BRIGHT[(colour.0 - 90) as usize],
            _ => b"invalid",
        });
    }
}
pub fn write_colour_escape(
    mut colour: Colour,
    background: bool,
    theme_colours: Option<&[Colour; 10]>,
    out: &mut Vec<u8>,
) -> bool {
    if colour.tagged(ColourFlags::THEME) {
        let slot = (colour.0 & 255) as usize;
        colour = theme_colours
            .and_then(|a| a.get(slot))
            .copied()
            .unwrap_or_else(|| theme_terminal_colour(slot as u32));
    }
    let base = if background { 40 } else { 30 };
    if colour.is_default() {
        write!(out, "\x1b[{}m", base + 9).unwrap();
    } else if colour.tagged(ColourFlags::RGB) {
        let (r, g, b) = colour.split_rgb();
        write!(out, "\x1b[{};2;{r};{g};{b}m", base + 8).unwrap();
    } else if colour.tagged(ColourFlags::_256) {
        write!(out, "\x1b[{};5;{}m", base + 8, colour.0 & 255).unwrap();
    } else if (0..=7).contains(&colour.0) {
        write!(out, "\x1b[{}m", base + colour.0).unwrap();
    } else if (90..=97).contains(&colour.0) {
        write!(out, "\x1b[{}m", base + colour.0 - 30).unwrap();
    } else {
        return false;
    }
    true
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColourPalette {
    pub fg: Colour,
    pub bg: Colour,
    palette: Option<Box<[Colour; 256]>>,
    default_palette: Option<Box<[Colour; 256]>>,
}
impl Default for ColourPalette {
    fn default() -> Self {
        Self::new()
    }
}
impl ColourPalette {
    pub fn new() -> Self {
        Self {
            fg: Colour::DEFAULT,
            bg: Colour::DEFAULT,
            palette: None,
            default_palette: None,
        }
    }
    pub fn clear_runtime(&mut self) {
        self.fg = Colour::DEFAULT;
        self.bg = Colour::DEFAULT;
        self.palette = None;
    }
    pub fn clear_storage(&mut self) {
        self.palette = None;
        self.default_palette = None;
    }
    pub fn get(&self, colour: Colour) -> Option<Colour> {
        let n = if (90..=97).contains(&colour.0) {
            colour.0 - 90 + 8
        } else if colour.tagged(ColourFlags::_256) {
            colour.0 & !(ColourFlags::_256.bits() as i32)
        } else if colour.0 >= 8 {
            return None;
        } else {
            colour.0
        };
        let n = usize::try_from(n).ok().filter(|n| *n < 256)?;
        self.palette
            .as_ref()
            .map(|p| p[n])
            .filter(|c| *c != Colour::NONE)
            .or_else(|| {
                self.default_palette
                    .as_ref()
                    .map(|p| p[n])
                    .filter(|c| *c != Colour::NONE)
            })
    }
    pub fn set(&mut self, slot: u32, colour: Colour) -> bool {
        if slot > 255 || (colour == Colour::NONE && self.palette.is_none()) {
            return false;
        }
        self.palette
            .get_or_insert_with(|| Box::new([Colour::NONE; 256]))[slot as usize] = colour;
        true
    }
    pub fn replace_defaults(&mut self, defaults: Option<[Colour; 256]>) {
        self.default_palette = defaults.map(Box::new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_allocation_tracks_c_lifecycle() {
        let mut palette = ColourPalette::new();
        assert!(!palette.set(1, Colour::NONE));
        assert!(palette.palette.is_none());
        palette.replace_defaults(Some([Colour::NONE; 256]));
        assert!(palette.default_palette.is_some());
        palette.clear_runtime();
        assert!(palette.default_palette.is_some());
        assert!(palette.set(1, Colour(2)));
        assert!(palette.palette.is_some());
        palette.replace_defaults(None);
        assert!(palette.default_palette.is_none());
        assert!(palette.palette.is_some());
        palette.clear_storage();
        assert!(palette.palette.is_none());
        assert!(Colour::NONE.force_rgb().is_none());
    }
}
