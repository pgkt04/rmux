// Ported from tmux grid.c and tmux.h @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! Grid data: a list of lines, rows `0..hsize` are history and
//! `hsize..hsize + sy` are the visible area. Everything here works in
//! absolute rows; `view.rs` adds `hsize`.

pub mod names;
pub mod reader;
pub mod reflow;
pub mod string;
pub mod view;

pub use string::StringCellsCtx;

use crate::cell::{DEFAULT_CELL, GridAttributes, GridCell, GridCellFlags};
use crate::colour::{Colour, ColourFlags};
use crate::hyperlinks::HyperlinkId;
use rmux_util::log_debug;
use rmux_util::utf8::{self, UTF8_SIZE, Utf8Char};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct GridLineFlags(pub u16);
impl GridLineFlags {
    pub const WRAPPED: Self = Self(1);
    pub const EXTENDED: Self = Self(2);
    pub const DEAD: Self = Self(4);
    pub const START_PROMPT: Self = Self(8);
    pub const SECOND_PROMPT: Self = Self(16);
    pub const START_COMMAND: Self = Self(32);
    pub const START_OUTPUT: Self = Self(64);
    pub const END_OUTPUT: Self = Self(128);
    pub const HYPERLINK: Self = Self(256);
    pub const OSC133_FLAGS: Self = Self(248);
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
impl std::ops::BitOr for GridLineFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for GridLineFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for GridLineFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct GridStringFlags(pub u32);
impl GridStringFlags {
    pub const WITH_SEQUENCES: Self = Self(1);
    pub const ESCAPE_SEQUENCES: Self = Self(2);
    pub const TRIM_SPACES: Self = Self(4);
    pub const USED_ONLY: Self = Self(8);
    pub const EMPTY_CELLS: Self = Self(16);
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
impl std::ops::BitOr for GridStringFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for GridStringFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for GridStringFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct GridFlags(pub u32);
impl GridFlags {
    pub const HISTORY: Self = Self(1);
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
impl std::ops::BitOr for GridFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for GridFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for GridFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// `grid_line.time`: seconds since server start plus one; 0 means unset
/// (`grid.c:246-253`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineTime(pub u32);

impl LineTime {
    /// `grid_line_time`: wall-clock seconds, given the server start seconds.
    pub fn to_wall(self, start_secs: i64) -> i64 {
        if self.0 == 0 {
            0
        } else {
            start_secs + i64::from(self.0) - 1
        }
    }
}

/// `struct osc133_data` (`tmux.h:899-905`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Osc133Data {
    pub prompt_col: u16,
    pub cmd_col: u16,
    pub out_start_col: u16,
    pub out_end_col: u16,
    pub exit_status: u8,
}

/// The four data bytes of a compact entry (`tmux.h:888-893`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompactCellData {
    pub attr: u8,
    pub fg: u8,
    pub bg: u8,
    pub data: u8,
}

/// Safe view of the `grid_cell_entry` union (`tmux.h:886-894`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridCellStorage {
    Compact(CompactCellData),
    Extended(u32),
}

/// `struct grid_cell_entry`: 5 bytes, compact data or an extended offset
/// (`tmux.h:885-896`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridCellEntry {
    data: [u8; 4],
    flags: GridCellFlags,
}

impl GridCellEntry {
    /// `grid_cleared_entry` (`grid.c:58-60`).
    const CLEARED: Self = Self {
        data: [0, 8, 8, b' '],
        flags: GridCellFlags::CLEARED,
    };

    pub fn flags(&self) -> GridCellFlags {
        self.flags
    }
    pub fn is_extended(&self) -> bool {
        self.flags.contains(GridCellFlags::EXTENDED)
    }
    pub fn offset(&self) -> u32 {
        u32::from_ne_bytes(self.data)
    }
    fn set_offset(&mut self, at: u32) {
        self.data = at.to_ne_bytes();
    }
    pub fn compact(&self) -> CompactCellData {
        CompactCellData {
            attr: self.data[0],
            fg: self.data[1],
            bg: self.data[2],
            data: self.data[3],
        }
    }
    pub fn storage(&self) -> GridCellStorage {
        if self.is_extended() {
            GridCellStorage::Extended(self.offset())
        } else {
            GridCellStorage::Compact(self.compact())
        }
    }
}

/// `struct grid_extd_entry` (`tmux.h:874-882`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridExtdEntry {
    pub data: Utf8Char,
    pub attr: GridAttributes,
    pub flags: GridCellFlags,
    pub fg: Colour,
    pub bg: Colour,
    pub us: Colour,
    pub link: HyperlinkId,
}

/// C accounting sizes (`docs/p0-probes.md`).
pub const CELL_ENTRY_BYTES: u32 = 5;
pub const EXTD_ENTRY_BYTES: u32 = 23;
pub const LINE_BYTES: u32 = 40;

/// `struct grid_line` (`tmux.h:908-919`). `cellsize` and `cellused` are the
/// logical C counts (16-bit); the `Vec` is the allocation.
#[derive(Debug, Default)]
pub struct GridLine {
    cells: Vec<GridCellEntry>,
    used: u16,
    cellsize: u16,
    extended: Vec<GridExtdEntry>,
    pub time: LineTime,
    pub osc133: Osc133Data,
    pub flags: GridLineFlags,
}

impl GridLine {
    pub fn cellsize(&self) -> u32 {
        u32::from(self.cellsize)
    }
    pub fn cellused(&self) -> u32 {
        u32::from(self.used)
    }
    pub fn extdsize(&self) -> u32 {
        self.extended.len() as u32
    }
    /// The logical cell entries (`celldata[0..cellsize]`).
    pub fn entries(&self) -> &[GridCellEntry] {
        &self.cells[..usize::from(self.cellsize).min(self.cells.len())]
    }
    pub fn extended_entries(&self) -> &[GridExtdEntry] {
        &self.extended
    }

    /// `grid_duplicate_lines` body: copy metadata and only the logical
    /// counts of entries (`grid.c:1294-1307`).
    fn duplicate(&self) -> GridLine {
        GridLine {
            cells: self.entries().to_vec(),
            used: self.used,
            cellsize: self.cellsize,
            extended: self.extended.clone(),
            time: self.time,
            osc133: self.osc133,
            flags: self.flags,
        }
    }

    /// `grid_get_cell1` (`grid.c:610-647`). `px` must be below `cellsize`.
    pub(crate) fn get_cell1(&self, px: usize) -> GridCell {
        let gce = &self.cells[px];
        if gce.is_extended() {
            let Some(gee) = self.extended.get(gce.offset() as usize) else {
                return DEFAULT_CELL;
            };
            let mut gc = GridCell {
                data: Default::default(),
                attr: gee.attr,
                flags: gee.flags,
                fg: gee.fg,
                bg: gee.bg,
                us: gee.us,
                link: gee.link,
            };
            if gc.flags.contains(GridCellFlags::TAB) {
                gc.set_tab(gee.data.0);
            } else {
                gc.data = utf8::to_data(gee.data);
            }
            return gc;
        }
        let c = gce.compact();
        let mut fg = Colour(i32::from(c.fg));
        if gce.flags.contains(GridCellFlags::FG256) {
            fg.0 |= ColourFlags::_256.bits() as i32;
        }
        let mut bg = Colour(i32::from(c.bg));
        if gce.flags.contains(GridCellFlags::BG256) {
            bg.0 |= ColourFlags::_256.bits() as i32;
        }
        GridCell {
            data: utf8::Utf8Data::set(c.data),
            attr: GridAttributes(u16::from(c.attr)),
            flags: gce.flags & !(GridCellFlags::FG256 | GridCellFlags::BG256),
            fg,
            bg,
            us: Colour::DEFAULT,
            link: HyperlinkId::NONE,
        }
    }

    /// `grid_get_cell` on one line: default when `px >= cellsize`.
    pub fn get_cell(&self, px: u32) -> GridCell {
        if px >= self.cellsize() {
            DEFAULT_CELL
        } else {
            self.get_cell1(px as usize)
        }
    }

    /// `grid_compact_line` (`grid.c:188-227`).
    fn compact(&mut self) {
        if self.extended.is_empty() {
            return;
        }
        let size = usize::from(self.cellsize).min(self.cells.len());
        let count = self.cells[..size]
            .iter()
            .filter(|e| e.is_extended())
            .count();
        if count == 0 {
            self.extended = Vec::new();
            return;
        }
        let mut new = Vec::with_capacity(count);
        for gce in &mut self.cells[..size] {
            if gce.is_extended() {
                new.push(self.extended[gce.offset() as usize]);
                gce.set_offset(new.len() as u32 - 1);
            }
        }
        self.extended = new;
    }
}

impl GridCell {
    /// `grid_cells_look_equal`: same style, ignoring the character
    /// (`grid.c:304-318`).
    pub fn look_equal(&self, other: &GridCell) -> bool {
        self.fg == other.fg
            && self.bg == other.bg
            && self.attr == other.attr
            && (self.flags & !GridCellFlags::CLEARED) == (other.flags & !GridCellFlags::CLEARED)
            && self.link == other.link
    }

    /// `grid_cells_equal` (`grid.c:321-331`).
    pub fn cells_equal(&self, other: &GridCell) -> bool {
        self.look_equal(other)
            && self.data.width == other.data.width
            && self.data.size == other.data.size
            && self.data.bytes() == other.data.bytes()
    }

    /// `grid_set_tab` (`grid.c:334-342`). The C store truncates `width` to
    /// `u_char`; the fill is bounded by the data buffer.
    pub fn set_tab(&mut self, width: u32) {
        self.data.data = [0; UTF8_SIZE];
        self.flags.insert(GridCellFlags::TAB);
        self.flags.remove(GridCellFlags::PADDING);
        let w = width as u8;
        self.data.width = w;
        self.data.size = w;
        self.data.have = w;
        self.data.data[..usize::from(w).min(UTF8_SIZE)].fill(b' ');
    }
}

/// `grid_padding_cell` (`grid.c:46-52`).
const PADDING_CELL: GridCell = GridCell {
    data: utf8::Utf8Data {
        data: {
            let mut bytes = [0; UTF8_SIZE];
            bytes[0] = b'!';
            bytes
        },
        have: 0,
        size: 0,
        width: 0,
    },
    attr: GridAttributes(0),
    flags: GridCellFlags::PADDING,
    fg: Colour::DEFAULT,
    bg: Colour::DEFAULT,
    us: Colour::DEFAULT,
    link: HyperlinkId::NONE,
};

/// `grid_cleared_cell` (`grid.c:55-57`).
const CLEARED_CELL: GridCell = GridCell {
    flags: GridCellFlags::CLEARED,
    ..DEFAULT_CELL
};

fn colour_has(c: Colour, flags: ColourFlags) -> bool {
    c.0 & flags.bits() as i32 != 0
}

/// `grid_store_cell` (`grid.c:98-115`).
fn store_cell(gce: &mut GridCellEntry, gc: &GridCell, c: u8) {
    gce.flags = gc.flags & !GridCellFlags::CLEARED;
    if colour_has(gc.fg, ColourFlags::_256) {
        gce.flags.insert(GridCellFlags::FG256);
    }
    if colour_has(gc.bg, ColourFlags::_256) {
        gce.flags.insert(GridCellFlags::BG256);
    }
    gce.data = [gc.attr.bits() as u8, gc.fg.0 as u8, gc.bg.0 as u8, c];
}

/// `grid_need_extended_cell` (`grid.c:118-138`).
fn need_extended(gce: &GridCellEntry, gc: &GridCell) -> bool {
    gce.is_extended()
        || gc.attr.bits() > 0xff
        || gc.data.size > 1
        || gc.data.width > 1
        || colour_has(gc.fg, ColourFlags::RGB | ColourFlags::THEME)
        || colour_has(gc.bg, ColourFlags::RGB | ColourFlags::THEME)
        || gc.us != Colour::DEFAULT
        || gc.link != HyperlinkId::NONE
        || gc.flags.contains(GridCellFlags::TAB)
}

/// `grid_get_extended_cell` (`grid.c:141-152`).
fn get_extended_cell(gl: &mut GridLine, px: usize, flags: GridCellFlags) {
    gl.extended.push(GridExtdEntry::default());
    let gce = &mut gl.cells[px];
    gce.set_offset(gl.extended.len() as u32 - 1);
    gce.flags = flags | GridCellFlags::EXTENDED;
}

/// `grid_extended_cell` (`grid.c:155-185`).
fn extended_cell<'a>(gl: &'a mut GridLine, px: usize, gc: &GridCell) -> &'a mut GridExtdEntry {
    let flags = gc.flags & !GridCellFlags::CLEARED;
    if !gl.cells[px].is_extended() {
        get_extended_cell(gl, px, flags);
    } else if gl.cells[px].offset() as usize >= gl.extended.len() {
        panic!("offset too big");
    }
    gl.flags.insert(GridLineFlags::EXTENDED);
    if gc.link != HyperlinkId::NONE {
        gl.flags.insert(GridLineFlags::HYPERLINK);
    }
    let uc = if gc.flags.contains(GridCellFlags::TAB) {
        Utf8Char(u32::from(gc.data.width))
    } else {
        utf8::from_data(&gc.data).0
    };
    let offset = gl.cells[px].offset() as usize;
    let gee = &mut gl.extended[offset];
    *gee = GridExtdEntry {
        data: uc,
        attr: gc.attr,
        flags,
        fg: gc.fg,
        bg: gc.bg,
        us: gc.us,
        link: gc.link,
    };
    gee
}

/// `grid_clear_cell` (`grid.c:263-290`).
fn clear_cell(gl: &mut GridLine, px: usize, bg: Colour, moved: bool) {
    let old = gl.cells[px];
    let gce = &mut gl.cells[px];
    *gce = GridCellEntry::CLEARED;
    if !moved && old.is_extended() && (old.offset() as usize) < gl.extended.len() {
        gce.flags.insert(GridCellFlags::EXTENDED);
        gce.set_offset(old.offset());
        let gee = extended_cell(gl, px, &CLEARED_CELL);
        if bg != Colour::DEFAULT {
            gee.bg = bg;
        }
    } else if bg != Colour::DEFAULT {
        if colour_has(bg, ColourFlags::RGB | ColourFlags::THEME) {
            let flags = gce.flags;
            get_extended_cell(gl, px, flags);
            extended_cell(gl, px, &CLEARED_CELL).bg = bg;
        } else {
            if colour_has(bg, ColourFlags::_256) {
                gce.flags.insert(GridCellFlags::BG256);
            }
            gce.data[2] = bg.0 as u8;
        }
    }
}

/// `grid_expand_line` (`grid.c:563-589`): grow the logical size with the
/// quarter/half/full-width thresholds of the grid width `gsx`.
pub(crate) fn expand_line(gsx: u32, gl: &mut GridLine, sx: u32, bg: Colour) {
    let mut sx = sx;
    if sx <= gl.cellsize() {
        return;
    }
    if sx < gsx / 4 {
        sx = gsx / 4;
    } else if sx < gsx / 2 {
        sx = gsx / 2;
    } else if gsx > sx {
        sx = gsx;
    }
    let old = usize::from(gl.cellsize);
    let n = sx as usize;
    gl.cells.resize(n, GridCellEntry::default());
    gl.cells[old..n].fill(GridCellEntry::default());
    for xx in old..n {
        clear_cell(gl, xx, bg, false);
    }
    gl.cellsize = sx as u16;
}

/// `grid_empty_line` (`grid.c:592-598`).
pub(crate) fn empty_line(gsx: u32, gl: &mut GridLine, bg: Colour) {
    *gl = GridLine::default();
    if !bg.is_default() {
        expand_line(gsx, gl, gsx, bg);
    }
}

/// `grid_set_cell` body on one line (`grid.c:661-681`).
pub(crate) fn set_cell_in_line(gsx: u32, gl: &mut GridLine, px: u32, gc: &GridCell) {
    expand_line(gsx, gl, px + 1, Colour::DEFAULT);
    if px + 1 > gl.cellused() {
        gl.used = (px + 1) as u16;
    }
    let px = px as usize;
    if need_extended(&gl.cells[px], gc) {
        extended_cell(gl, px, gc);
    } else {
        store_cell(&mut gl.cells[px], gc, gc.data.data[0]);
    }
}

/// `grid_move_cells` body on one line (`grid.c:838-854`).
pub(crate) fn move_cells_in_line(
    gsx: u32,
    gl: &mut GridLine,
    dx: u32,
    px: u32,
    nx: u32,
    bg: Colour,
) {
    expand_line(gsx, gl, px + nx, Colour::DEFAULT);
    expand_line(gsx, gl, dx + nx, Colour::DEFAULT);
    let (dx, px, nx) = (dx as usize, px as usize, nx as usize);
    gl.cells.copy_within(px..px + nx, dx);
    if dx + nx > usize::from(gl.used) {
        gl.used = (dx + nx) as u16;
    }
    for xx in px..px + nx {
        if xx >= dx && xx < dx + nx {
            continue;
        }
        clear_cell(gl, xx, bg, true);
    }
}

fn in_utf8_set(set: &[u8], data: &utf8::Utf8Data) -> bool {
    let mut at = 0;
    while at < set.len() {
        let mut next = at + 1;
        let mut candidate = utf8::Utf8Data::set(set[at]);
        if let Ok(mut decoded) = utf8::Utf8Data::open(set[at]) {
            let mut state = utf8::Utf8State::More;
            while next < set.len() && state == utf8::Utf8State::More {
                state = decoded.append(set[next]);
                next += 1;
            }
            if state == utf8::Utf8State::Done {
                candidate = decoded;
            } else {
                next = at + 1;
            }
        }
        if candidate.size == data.size && candidate.bytes() == data.bytes() {
            return true;
        }
        at = next;
    }
    false
}

/// `struct grid` (`tmux.h:922-938`).
#[derive(Debug)]
pub struct Grid {
    pub flags: GridFlags,
    sx: u32,
    sy: u32,
    pub hscrolled: u32,
    hsize: u32,
    hlimit: u32,
    pub scroll_added: u32,
    pub scroll_collected: u32,
    pub scroll_generation: u32,
    lines: Vec<GridLine>,
    line_clock: LineTime,
}

impl Grid {
    /// `grid_create` (`grid.c:372-393`).
    pub fn new(sx: u32, sy: u32, hlimit: u32) -> Grid {
        let mut lines = Vec::new();
        lines.resize_with(sy as usize, GridLine::default);
        let gd = Grid {
            flags: if hlimit != 0 {
                GridFlags::HISTORY
            } else {
                GridFlags(0)
            },
            sx,
            sy,
            hscrolled: 0,
            hsize: 0,
            hlimit,
            scroll_added: 0,
            scroll_collected: 0,
            scroll_generation: 0,
            lines,
            line_clock: LineTime(0),
        };
        gd.check_is_clear();
        gd
    }

    pub fn sx(&self) -> u32 {
        self.sx
    }
    pub fn sy(&self) -> u32 {
        self.sy
    }
    pub fn hsize(&self) -> u32 {
        self.hsize
    }
    pub fn hlimit(&self) -> u32 {
        self.hlimit
    }
    /// `gd->sx = sx` (`screen.c:472`); the caller owns the width change.
    pub fn set_sx(&mut self, sx: u32) {
        self.sx = sx;
    }
    /// Caller transaction only: the line list is adjusted separately.
    pub fn set_sy_unchecked(&mut self, sy: u32) {
        self.sy = sy;
    }
    /// Caller transaction only: the line list is adjusted separately.
    pub fn set_hsize_unchecked(&mut self, hsize: u32) {
        self.hsize = hsize;
    }
    pub fn set_hlimit(&mut self, hlimit: u32) {
        self.hlimit = hlimit;
    }
    pub fn set_flags(&mut self, flags: GridFlags) {
        self.flags = flags;
    }
    pub fn line_clock(&self) -> LineTime {
        self.line_clock
    }
    /// The stamp `scroll_history*` writes into lines entering the history
    /// (`grid.c:246-253`, `server.c:273`).
    pub fn set_line_clock(&mut self, now: LineTime) {
        self.line_clock = now;
    }
    pub fn lines(&self) -> &[GridLine] {
        &self.lines
    }

    /// `grid_check_is_clear` (`grid.c:62-96`), a `debug_assert` walk.
    pub fn check_is_clear(&self) {
        if cfg!(debug_assertions) {
            for gl in &self.lines[..self.hsize.wrapping_add(self.sy) as usize] {
                debug_assert!(gl.cells.is_empty());
                debug_assert!(gl.used == 0);
                debug_assert!(gl.cellsize == 0);
                debug_assert!(gl.extended.is_empty());
                debug_assert!(gl.flags == GridLineFlags(0));
                debug_assert!(gl.time == LineTime(0));
            }
        }
    }

    /// `grid_check_y` (`grid.c:293-301`).
    fn check_y(&self, from: &str, py: u32) -> bool {
        if py >= self.hsize + self.sy {
            log_debug!("{}: y out of range: {}", from, py);
            return false;
        }
        true
    }

    /// `grid_get_line`: unchecked (`grid.c:230-233`).
    pub fn get_line(&self, py: u32) -> &GridLine {
        &self.lines[py as usize]
    }
    pub fn get_line_mut(&mut self, py: u32) -> &mut GridLine {
        &mut self.lines[py as usize]
    }

    /// `grid_peek_line` (`grid.c:601-607`).
    pub fn peek_line(&self, py: u32) -> Option<&GridLine> {
        if !self.check_y("grid_peek_line", py) {
            return None;
        }
        Some(&self.lines[py as usize])
    }

    /// `grid_adjust_lines` (`grid.c:256-260`): change the storage length
    /// only. New lines start empty.
    pub fn adjust_lines(&mut self, lines: u32) {
        self.lines.resize_with(lines as usize, GridLine::default);
    }

    /// `grid_free_lines` (`grid.c:362-369`).
    pub fn free_lines(&mut self, py: u32, ny: u32) {
        for gl in &mut self.lines[py as usize..(py + ny) as usize] {
            *gl = GridLine::default();
        }
    }

    /// `grid_empty_line` (`grid.c:592-598`).
    pub fn empty_line(&mut self, py: u32, bg: Colour) {
        empty_line(self.sx, &mut self.lines[py as usize], bg);
    }

    /// `grid_compare`: true when the grids differ (`grid.c:405-429`).
    /// Indexes absolute rows `0..sy`, as the C code does.
    pub fn compare(&self, other: &Grid) -> bool {
        if self.sx != other.sx || self.sy != other.sy {
            return true;
        }
        for yy in 0..self.sy {
            let gla = &self.lines[yy as usize];
            let glb = &other.lines[yy as usize];
            if gla.cellsize != glb.cellsize {
                return true;
            }
            for xx in 0..gla.cellsize() {
                if !self.get_cell(xx, yy).cells_equal(&other.get_cell(xx, yy)) {
                    return true;
                }
            }
        }
        false
    }

    /// `grid_trim_history` (`grid.c:432-442`).
    fn trim_history(&mut self, ny: u32) {
        self.lines.drain(0..ny as usize);
    }

    /// `grid_collect_history` (`grid.c:448-475`).
    pub fn collect_history(&mut self, all: bool) {
        if self.hsize == 0 || self.hsize < self.hlimit {
            return;
        }
        let mut ny = if all {
            self.hsize - self.hlimit
        } else {
            self.hlimit / 10
        };
        ny = ny.clamp(1, self.hsize);
        self.trim_history(ny);
        self.hsize -= ny;
        self.scroll_collected = self.scroll_collected.wrapping_add(ny);
        if self.hscrolled > self.hsize {
            self.hscrolled = self.hsize;
        }
    }

    /// `grid_remove_history` (`grid.c:478-490`).
    pub fn remove_history(&mut self, ny: u32) {
        if ny > self.hsize {
            return;
        }
        let start = (self.hsize + self.sy - ny) as usize;
        self.lines.truncate(start);
        self.hsize -= ny;
    }

    /// `grid_scroll_history` (`grid.c:496-511`).
    pub fn scroll_history(&mut self, bg: Colour) {
        let yy = self.hsize + self.sy;
        self.lines.push(GridLine::default());
        self.empty_line(yy, bg);

        self.hscrolled = self.hscrolled.wrapping_add(1);
        let gl = &mut self.lines[self.hsize as usize];
        gl.compact();
        gl.time = self.line_clock;
        self.hsize = self.hsize.wrapping_add(1);
        self.scroll_added = self.scroll_added.wrapping_add(1);
    }

    /// `grid_clear_history` (`grid.c:514-525`).
    pub fn clear_history(&mut self) {
        self.trim_history(self.hsize);
        self.hscrolled = 0;
        self.hsize = 0;
        self.scroll_generation = self.scroll_generation.wrapping_add(1);
        self.lines.truncate(self.sy as usize);
    }

    /// `grid_scroll_history_region` (`grid.c:528-560`), in the C order:
    /// shift the view down, copy the region top into the history slot, move
    /// the region up, empty the bottom.
    pub fn scroll_history_region(&mut self, upper: u32, lower: u32, bg: Colour) {
        let hsize = self.hsize as usize;
        self.lines.push(GridLine::default());
        self.lines[hsize..].rotate_right(1);

        let upper = upper as usize + 1;
        let lower = lower as usize + 1;
        self.lines.swap(hsize, upper);
        self.lines[hsize].time = self.line_clock;
        self.lines[upper..=lower].rotate_left(1);
        empty_line(self.sx, &mut self.lines[lower], bg);

        self.hscrolled = self.hscrolled.wrapping_add(1);
        self.hsize = self.hsize.wrapping_add(1);
        self.scroll_added = self.scroll_added.wrapping_add(1);
    }

    /// `grid_get_cell` (`grid.c:650-658`).
    pub fn get_cell(&self, px: u32, py: u32) -> GridCell {
        if !self.check_y("grid_get_cell", py) {
            return DEFAULT_CELL;
        }
        self.lines[py as usize].get_cell(px)
    }

    /// `grid_set_cell` (`grid.c:661-681`).
    pub fn set_cell(&mut self, px: u32, py: u32, gc: &GridCell) {
        if !self.check_y("grid_set_cell", py) {
            return;
        }
        set_cell_in_line(self.sx, &mut self.lines[py as usize], px, gc);
    }

    /// `grid_set_padding` (`grid.c:684-692`).
    pub fn set_padding(&mut self, px: u32, py: u32, bg: Colour) {
        let gc = GridCell { bg, ..PADDING_CELL };
        self.set_cell(px, py, &gc);
    }

    /// `grid_set_cells` (`grid.c:695-721`).
    pub fn set_cells(&mut self, px: u32, py: u32, gc: &GridCell, s: &[u8]) {
        if !self.check_y("grid_set_cells", py) {
            return;
        }
        let slen = s.len() as u32;
        let gl = &mut self.lines[py as usize];
        expand_line(self.sx, gl, px + slen, Colour::DEFAULT);
        if px + slen > gl.cellused() {
            gl.used = (px + slen) as u16;
        }
        for (i, &c) in s.iter().enumerate() {
            let x = px as usize + i;
            if need_extended(&gl.cells[x], gc) {
                extended_cell(gl, x, gc).data = utf8::build_one(c);
            } else {
                store_cell(&mut gl.cells[x], gc, c);
            }
        }
    }

    /// `grid_clear` (`grid.c:724-761`).
    pub fn clear(&mut self, px: u32, py: u32, nx: u32, ny: u32, bg: Colour) {
        if nx == 0 || ny == 0 {
            return;
        }
        if px == 0 && nx == self.sx {
            self.clear_lines(py, ny, bg);
            return;
        }
        if !self.check_y("grid_clear", py)
            || !self.check_y("grid_clear", py.wrapping_add(ny).wrapping_sub(1))
        {
            return;
        }
        for yy in py..py.wrapping_add(ny) {
            let gl = &mut self.lines[yy as usize];
            let sx = self.sx.min(gl.cellsize());
            let mut ox = nx;
            if bg.is_default() {
                if px > sx {
                    continue;
                }
                if px.wrapping_add(nx) > sx {
                    ox = sx - px;
                }
            }
            expand_line(self.sx, gl, px.wrapping_add(ox), Colour::DEFAULT);
            for xx in px..px.wrapping_add(ox) {
                clear_cell(gl, xx as usize, bg, false);
            }
        }
    }

    /// `grid_clear_lines` (`grid.c:764-783`).
    pub fn clear_lines(&mut self, py: u32, ny: u32, bg: Colour) {
        if ny == 0 {
            return;
        }
        if !self.check_y("grid_clear_lines", py)
            || !self.check_y("grid_clear_lines", py.wrapping_add(ny).wrapping_sub(1))
        {
            return;
        }
        for yy in py..py.wrapping_add(ny) {
            self.empty_line(yy, bg);
        }
        if py != 0 {
            self.lines[py as usize - 1]
                .flags
                .remove(GridLineFlags::WRAPPED);
        }
    }

    /// `grid_move_lines` (`grid.c:786-825`): headers move, no row is cloned.
    pub fn move_lines(&mut self, dy: u32, py: u32, ny: u32, bg: Colour) {
        if ny == 0 || py == dy {
            return;
        }
        let f = "grid_move_lines";
        if !self.check_y(f, py)
            || !self.check_y(f, py.wrapping_add(ny).wrapping_sub(1))
            || !self.check_y(f, dy)
            || !self.check_y(f, dy.wrapping_add(ny).wrapping_sub(1))
        {
            return;
        }
        for yy in dy..dy.wrapping_add(ny) {
            if yy >= py && yy < py.wrapping_add(ny) {
                continue;
            }
            self.lines[yy as usize] = GridLine::default();
        }
        if dy != 0 {
            self.lines[dy as usize - 1]
                .flags
                .remove(GridLineFlags::WRAPPED);
        }

        let (dy_, py_, ny_) = (dy as usize, py as usize, ny as usize);
        if dy_ + ny_ <= py_ || py_ + ny_ <= dy_ {
            for i in 0..ny_ {
                self.lines.swap(dy_ + i, py_ + i);
            }
        } else if dy_ < py_ {
            self.lines[dy_..py_ + ny_].rotate_left(py_ - dy_);
        } else {
            self.lines[py_..dy_ + ny_].rotate_right(dy_ - py_);
        }

        for yy in py..py.wrapping_add(ny) {
            if yy < dy || yy >= dy.wrapping_add(ny) {
                self.empty_line(yy, bg);
            }
        }
        if py != 0 && (py < dy || py >= dy.wrapping_add(ny)) {
            self.lines[py as usize - 1]
                .flags
                .remove(GridLineFlags::WRAPPED);
        }
    }

    /// `grid_move_cells` (`grid.c:828-855`).
    pub fn move_cells(&mut self, dx: u32, px: u32, py: u32, nx: u32, bg: Colour) {
        if nx == 0 || px == dx {
            return;
        }
        if !self.check_y("grid_move_cells", py) {
            return;
        }
        move_cells_in_line(self.sx, &mut self.lines[py as usize], dx, px, nx, bg);
    }

    /// `grid_duplicate_lines` (`grid.c:1277-1314`): copy `ny` lines from
    /// `src` row `sy` into this grid at `dy`, clamped to both grids.
    pub fn duplicate_lines(&mut self, dy: u32, src: &Grid, sy: u32, ny: u32) {
        let mut ny = ny;
        if dy + ny > self.hsize + self.sy {
            ny = self.hsize + self.sy - dy;
        }
        if sy + ny > src.hsize + src.sy {
            ny = src.hsize + src.sy - sy;
        }
        self.free_lines(dy, ny);
        for yy in 0..ny as usize {
            self.lines[dy as usize + yy] = src.lines[sy as usize + yy].duplicate();
        }
    }

    /// `grid_line_length` (`grid.c:1668-1686`).
    pub fn line_length(&self, py: u32) -> u32 {
        let gl = self.get_line(py);
        let mut px = gl.cellsize().min(self.sx);
        while px > 0 {
            let gc = self.get_cell(px - 1, py);
            if gc.flags.contains(GridCellFlags::PADDING)
                || gc.data.size != 1
                || gc.data.data[0] != b' '
            {
                break;
            }
            px -= 1;
        }
        px
    }

    /// `grid_line_limit` (`grid.c:1689-1706`).
    pub fn line_limit(&self, py: u32) -> u32 {
        let mut px = self.line_length(py);
        if px == 0 {
            return 0;
        }
        px -= 1;
        while px > 0 {
            if !self.get_cell(px, py).flags.contains(GridCellFlags::PADDING) {
                break;
            }
            px -= 1;
        }
        px
    }

    /// `grid_in_set` (`grid.c:1709-1738`): a width, not a boolean.
    pub fn in_set(&self, px: u32, py: u32, set: &[u8]) -> u32 {
        let set = &set[..set.iter().position(|&byte| byte == 0).unwrap_or(set.len())];
        let has_tab = set.contains(&b'\t');
        let has_space = set.contains(&b' ');

        let gc = self.get_cell(px, py);
        if gc.flags.contains(GridCellFlags::PADDING) {
            if !has_tab && !has_space {
                return 0;
            }
            let mut pxx = px;
            let mut tmp;
            loop {
                pxx = pxx.wrapping_sub(1);
                tmp = self.get_cell(pxx, py);
                if !(pxx > 0 && tmp.flags.contains(GridCellFlags::PADDING)) {
                    break;
                }
            }
            if ((has_tab || has_space) && tmp.flags.contains(GridCellFlags::TAB))
                || (has_space && tmp.data.has_whitespace())
            {
                return u32::from(tmp.data.width).wrapping_sub(px.wrapping_sub(pxx));
            }
            return 0;
        }
        if (has_tab || has_space) && gc.flags.contains(GridCellFlags::TAB) {
            return u32::from(gc.data.width);
        }
        if has_space && gc.data.has_whitespace() {
            return if gc.data.width == 0 {
                1
            } else {
                u32::from(gc.data.width)
            };
        }
        u32::from(in_utf8_set(set, &gc.data))
    }

    /// Logical C storage counts: `(lines, cells, extended cells)` with
    /// wrapping sums (`format.c:1036-1045`).
    pub fn storage_counts(&self) -> (u32, u32, u32) {
        let lines = self.hsize.wrapping_add(self.sy);
        let mut cells = 0u32;
        let mut extended = 0u32;
        for gl in &self.lines[..lines as usize] {
            cells = cells.wrapping_add(gl.cellsize());
            extended = extended.wrapping_add(gl.extdsize());
        }
        (lines, cells, extended)
    }

    /// `history_bytes` (`format.c:1013-1016`): a `size_t` sum with C entry
    /// sizes.
    pub fn history_bytes(&self) -> u64 {
        let lines = self.hsize + self.sy;
        let mut size = u64::from(lines) * u64::from(LINE_BYTES);
        for gl in &self.lines[..lines as usize] {
            size += u64::from(gl.cellsize()) * u64::from(CELL_ENTRY_BYTES);
            size += u64::from(gl.extdsize()) * u64::from(EXTD_ENTRY_BYTES);
        }
        size
    }
}
