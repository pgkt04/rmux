// Ported from tmux screen.c, tmux.c and tmux.h @ 8f25579c
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

pub mod borders;
mod cell_write;
mod collect;
mod operations;
mod text;
pub mod write;

use crate::cell::{GridAttributes, GridCell, GridCellFlags};
use crate::colour::Colour;
use crate::grid::{Grid, GridFlags};
use crate::hyperlinks::{HyperlinkError, HyperlinkRegistry, Hyperlinks};
use rmux_util::{utf8, vis::VisFlags};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScreenResetPolicy {
    pub extended_keys: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressBar {
    pub state: ProgressBarState,
    pub progress: i32,
}

#[derive(Clone, Debug)]
pub struct ScreenSelection {
    pub hidden: bool,
    pub rectangle: bool,
    pub modekeys: i32,
    pub sx: u32,
    pub sy: u32,
    pub ex: u32,
    pub ey: u32,
    pub clipx: u32,
    pub cell: GridCell,
}

#[derive(Debug, Default)]
pub struct ScreenTitles(pub VecDeque<Vec<u8>>);

#[derive(Debug)]
pub struct Screen {
    pub grid: Grid,
    pub cx: u32,
    pub cy: u32,
    pub rupper: u32,
    pub rlower: u32,
    pub mode: ScreenMode,
    pub default_mode: ScreenMode,
    pub cstyle: ScreenCursorStyle,
    pub default_cstyle: ScreenCursorStyle,
    pub ccolour: Colour,
    pub default_ccolour: Colour,
    pub tabs: Vec<bool>,
    pub selection: Option<ScreenSelection>,
    pub title: Vec<u8>,
    pub path: Option<Vec<u8>>,
    pub titles: ScreenTitles,
    pub progress_bar: ProgressBar,
    pub saved_grid: Option<Grid>,
    pub saved_cursor: Option<(u32, u32)>,
    pub saved_cell: GridCell,
    pub saved_flags: GridFlags,
    pub hyperlinks: Option<Hyperlinks>,
    pub write_list: Vec<write::ScreenWriteLine>,
    pub reset_policy: ScreenResetPolicy,
}

impl Screen {
    pub fn new(
        sx: u32,
        sy: u32,
        hlimit: u32,
        policy: ScreenResetPolicy,
        registry: &mut HyperlinkRegistry,
    ) -> Result<Self, HyperlinkError> {
        let mut screen = Self {
            grid: Grid::new(sx, sy, hlimit),
            cx: 0,
            cy: 0,
            rupper: 0,
            rlower: sy - 1,
            mode: ScreenMode::CURSOR,
            default_mode: ScreenMode(0),
            cstyle: ScreenCursorStyle::Default,
            default_cstyle: ScreenCursorStyle::Default,
            ccolour: Colour(-1),
            default_ccolour: Colour(-1),
            tabs: Vec::new(),
            selection: None,
            title: Vec::new(),
            path: None,
            titles: ScreenTitles::default(),
            progress_bar: ProgressBar {
                state: ProgressBarState::Hidden,
                progress: 0,
            },
            saved_grid: None,
            saved_cursor: None,
            saved_cell: GridCell::default(),
            saved_flags: GridFlags(0),
            hyperlinks: Some(registry.create()?),
            write_list: Vec::new(),
            reset_policy: policy,
        };
        screen.reinit(true, policy, registry)?;
        Ok(screen)
    }

    pub fn release(&mut self, registry: &mut HyperlinkRegistry) -> Result<(), HyperlinkError> {
        if let Some(links) = self.hyperlinks.take() {
            registry.release(links)?;
        }
        self.saved_grid = None;
        self.selection = None;
        self.tabs.clear();
        self.title.clear();
        self.path = None;
        self.titles.0.clear();
        self.write_list.clear();
        Ok(())
    }

    pub fn reinit(
        &mut self,
        check: bool,
        policy: ScreenResetPolicy,
        registry: &mut HyperlinkRegistry,
    ) -> Result<(), HyperlinkError> {
        self.reset_policy = policy;
        self.cx = 0;
        self.cy = 0;
        self.rupper = 0;
        self.rlower = self.grid.sy() - 1;
        self.mode = ScreenMode::CURSOR | ScreenMode::WRAP | (self.mode & ScreenMode::CRLF);
        if policy.extended_keys {
            self.mode.insert(ScreenMode::KEYS_EXTENDED);
        }
        if self.is_alternate() {
            self.alternate_off(None, false);
        }
        self.saved_cursor = None;
        self.reset_tabs();
        if check {
            self.grid.check_is_clear();
        }
        self.grid
            .clear_lines(self.grid.hsize(), self.grid.sy(), Colour(8));
        self.clear_selection();
        self.titles.0.clear();
        self.set_progress_bar(ProgressBarState::Hidden, 0);
        self.reset_hyperlinks(registry)
    }

    pub fn reset_hyperlinks(
        &mut self,
        registry: &mut HyperlinkRegistry,
    ) -> Result<(), HyperlinkError> {
        if let Some(links) = &self.hyperlinks {
            registry.reset(links)
        } else {
            self.hyperlinks = Some(registry.create()?);
            Ok(())
        }
    }
    pub fn reset_tabs(&mut self) {
        self.tabs.clear();
        self.tabs.resize(self.grid.sx() as usize, false);
        for x in (8..self.grid.sx()).step_by(8) {
            self.tabs[x as usize] = true;
        }
    }
    pub fn set_default_cursor(&mut self, colour: Colour, style: u32) {
        self.default_ccolour = colour;
        self.default_mode = ScreenMode(0);
        set_cursor_style(style, &mut self.default_cstyle, &mut self.default_mode);
    }
    pub fn set_cursor_style(&mut self, style: u32) {
        set_cursor_style(style, &mut self.cstyle, &mut self.mode);
    }
    pub fn set_cursor_colour(&mut self, colour: Colour) {
        self.ccolour = colour;
    }
    pub fn set_title(&mut self, title: &[u8], untrusted: bool) -> bool {
        let Some(title) = clean_name(title, untrusted) else {
            return false;
        };
        self.title = title;
        true
    }
    pub fn set_path(&mut self, path: &[u8], untrusted: bool) -> bool {
        let Some(path) = clean_name(path, untrusted) else {
            return false;
        };
        self.path = Some(path);
        true
    }
    pub fn push_title(&mut self) {
        while self.titles.0.len() >= 10 {
            self.titles.0.pop_back();
        }
        self.titles.0.push_front(self.title.clone());
    }
    pub fn pop_title(&mut self) {
        if let Some(title) = self.titles.0.pop_front() {
            self.title = title;
        }
    }
    pub fn set_progress_bar(&mut self, state: ProgressBarState, progress: i32) {
        self.progress_bar.state = state;
        if progress >= 0 && state != ProgressBarState::Indeterminate {
            self.progress_bar.progress = progress;
        }
    }

    pub fn resize(&mut self, sx: u32, sy: u32, reflow: bool) {
        self.resize_cursor(sx, sy, reflow, true, true);
    }
    pub fn resize_cursor(
        &mut self,
        sx: u32,
        sy: u32,
        mut reflow: bool,
        eat_empty: bool,
        cursor: bool,
    ) {
        let (mut cx, mut cy) = (self.cx, self.grid.hsize() + self.cy);
        let had_write_list = !self.write_list.is_empty();
        self.write_list.clear();
        let (sx, sy) = (sx.max(1), sy.max(1));
        if sx != self.grid.sx() {
            self.grid.set_sx(sx);
            self.reset_tabs();
        } else {
            reflow = false;
        }
        if sy != self.grid.sy() {
            self.resize_y(sy, eat_empty, &mut cy);
        }
        if reflow {
            let position = cursor.then(|| self.grid.wrap_position(cx, cy));
            self.grid.reflow(sx);
            (cx, cy) = position.map_or((0, self.grid.hsize()), |(wx, wy)| {
                self.grid.unwrap_position(wx, wy)
            });
        }
        if cy >= self.grid.hsize() {
            self.cx = cx;
            self.cy = cy - self.grid.hsize();
        } else {
            self.cx = 0;
            self.cy = 0;
        }
        if had_write_list {
            self.write_list
                .resize_with(self.grid.sy() as usize, write::ScreenWriteLine::default);
        }
    }
    fn resize_y(&mut self, sy: u32, eat_empty: bool, cy: &mut u32) {
        let oldy = self.grid.sy();
        if sy < oldy {
            let mut needed = oldy - sy;
            if eat_empty {
                let available = (oldy - 1 - self.cy).min(needed);
                if available > 0 {
                    self.grid
                        .view_delete_lines(oldy - available, available, Colour(8));
                }
                needed -= available;
            }
            if self.grid.flags.contains(GridFlags::HISTORY) {
                self.grid.hscrolled += needed;
                self.grid.set_hsize_unchecked(self.grid.hsize() + needed);
            } else {
                let available = self.cy.min(needed);
                if available > 0 {
                    self.grid.view_delete_lines(0, available, Colour(8));
                    *cy -= available;
                }
            }
        }
        self.grid.adjust_lines(self.grid.hsize() + sy);
        if sy > oldy {
            let mut needed = sy - oldy;
            let available = if self.grid.flags.contains(GridFlags::HISTORY) {
                self.grid.hscrolled.min(needed)
            } else {
                0
            };
            self.grid.hscrolled -= available;
            self.grid.set_hsize_unchecked(self.grid.hsize() - available);
            needed -= available;
            for y in self.grid.hsize() + sy - needed..self.grid.hsize() + sy {
                self.grid.empty_line(y, Colour(8));
            }
        }
        self.grid.set_sy_unchecked(sy);
        self.rupper = 0;
        self.rlower = sy - 1;
    }

    pub fn is_alternate(&self) -> bool {
        self.saved_grid.is_some()
    }
    pub fn alternate_on(&mut self, cell: &GridCell, cursor: bool) -> bool {
        if self.is_alternate() {
            return false;
        }
        let mut saved = Grid::new(self.grid.sx(), self.grid.sy(), 0);
        saved.duplicate_lines(0, &self.grid, self.grid.hsize(), self.grid.sy());
        self.saved_grid = Some(saved);
        if cursor {
            self.saved_cursor = Some((self.cx, self.cy));
        }
        self.saved_cell = *cell;
        self.grid
            .view_clear(0, 0, self.grid.sx(), self.grid.sy(), Colour(8));
        self.saved_flags = self.grid.flags;
        self.grid.flags.remove(GridFlags::HISTORY);
        true
    }
    pub fn alternate_off(&mut self, cell: Option<&mut GridCell>, cursor: bool) -> bool {
        let (sx, sy) = (self.grid.sx(), self.grid.sy());
        if let Some(saved) = &self.saved_grid {
            let size = (saved.sx(), saved.sy());
            self.resize(size.0, size.1, false);
        }
        if cursor && let Some((cx, cy)) = self.saved_cursor {
            self.cx = cx;
            self.cy = cy;
            if let Some(cell) = cell {
                *cell = self.saved_cell;
            }
        }
        let alternate = self.is_alternate();
        if let Some(saved) = &self.saved_grid {
            self.grid
                .duplicate_lines(self.grid.hsize(), saved, 0, saved.sy());
            if self.saved_flags.contains(GridFlags::HISTORY) {
                self.grid.flags.insert(GridFlags::HISTORY);
            }
            self.resize(sx, sy, true);
            self.saved_grid = None;
        }
        self.cx = self.cx.min(self.grid.sx() - 1);
        self.cy = self.cy.min(self.grid.sy() - 1);
        alternate
    }

    pub fn set_selection(&mut self, selection: ScreenSelection) {
        self.selection = Some(ScreenSelection {
            hidden: false,
            ..selection
        });
    }
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }
    pub fn hide_selection(&mut self) {
        if let Some(selection) = &mut self.selection {
            selection.hidden = true;
        }
    }
    pub fn check_selection(&self, px: u32, py: u32) -> bool {
        let Some(s) = &self.selection else {
            return false;
        };
        if s.hidden || px < s.clipx {
            return false;
        }
        if s.rectangle {
            return py >= s.sy.min(s.ey)
                && py <= s.sy.max(s.ey)
                && px >= s.sx.min(s.ex)
                && px <= s.sx.max(s.ex);
        }
        let emacs = s.modekeys == 0;
        if s.sy < s.ey {
            let xx = if emacs { s.ex.saturating_sub(1) } else { s.ex };
            !(py < s.sy || py > s.ey || (py == s.sy && px < s.sx) || (py == s.ey && px > xx))
        } else if s.sy > s.ey {
            let xx = if emacs { s.sx.wrapping_sub(1) } else { s.sx };
            !(py > s.sy
                || py < s.ey
                || (py == s.ey && px < s.ex)
                || (py == s.sy && (s.sx == 0 || px > xx)))
        } else if py != s.sy {
            false
        } else if s.ex < s.sx {
            let xx = if emacs { s.sx.wrapping_sub(1) } else { s.sx };
            px <= xx && px >= s.ex
        } else {
            let xx = if emacs { s.ex.saturating_sub(1) } else { s.ex };
            px >= s.sx && px <= xx
        }
    }
    pub fn select_cell(&self, source: &GridCell) -> GridCell {
        let Some(s) = self.selection.as_ref().filter(|s| !s.hidden) else {
            return *source;
        };
        let mut cell = s.cell;
        if cell.fg.is_default() {
            cell.fg = source.fg;
        }
        if cell.bg.is_default() {
            cell.bg = source.bg;
        }
        cell.data = source.data;
        cell.flags = source.flags;
        cell.attr
            .insert(if cell.attr.contains(GridAttributes::NOATTR) {
                source.attr & GridAttributes::CHARSET
            } else {
                source.attr
            });
        cell
    }

    pub fn print<'a>(
        &self,
        line: Option<u32>,
        out: &'a mut Vec<u8>,
        acs: impl Fn(u8) -> Option<&'static [u8]>,
    ) -> &'a [u8] {
        const LIMIT: usize = 16384;
        out.clear();
        for y in 0..self.grid.hsize() + self.grid.sy() {
            if line.is_some_and(|line| line != y) {
                continue;
            }
            let header = format!("{y:04} \"");
            if out.len() + header.len() >= LIMIT {
                break;
            }
            out.extend_from_slice(header.as_bytes());
            let row = self.grid.get_line(y);
            for entry in row.entries().iter().take(row.cellused() as usize) {
                let flags = entry.flags();
                if flags.contains(GridCellFlags::PADDING) {
                    continue;
                }
                let compact = [entry.compact().data];
                let data;
                let bytes: &[u8] = if !entry.is_extended() {
                    &compact
                } else if flags.contains(GridCellFlags::TAB) {
                    b"\t"
                } else if flags.bits() & GridAttributes::CHARSET.bits() as u8 != 0 {
                    acs(compact[0]).unwrap_or(&compact)
                } else {
                    data = utf8::to_data(row.extended_entries()[entry.offset() as usize].data);
                    &data.data[..data.size as usize]
                };
                if out.len() + bytes.len() + 1 >= LIMIT {
                    return out;
                }
                out.extend_from_slice(bytes);
            }
            if out.len() + 3 >= LIMIT {
                break;
            }
            out.extend_from_slice(b"\"\n");
        }
        out
    }
}

pub fn set_cursor_style(style: u32, shape: &mut ScreenCursorStyle, mode: &mut ScreenMode) {
    match style {
        0 => {
            *shape = ScreenCursorStyle::Default;
            return;
        }
        1 | 2 => *shape = ScreenCursorStyle::Block,
        3 | 4 => *shape = ScreenCursorStyle::Underline,
        5 | 6 => *shape = ScreenCursorStyle::Bar,
        _ => return,
    }
    if style % 2 == 1 {
        mode.insert(ScreenMode::CURSOR_BLINKING);
    } else {
        mode.remove(ScreenMode::CURSOR_BLINKING);
    }
}

fn clean_name(name: &[u8], untrusted: bool) -> Option<Vec<u8>> {
    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
    if !utf8::is_valid(name) {
        return None;
    }
    let mut copy = name.to_vec();
    if untrusted {
        for i in 0..copy.len().saturating_sub(1) {
            if copy[i] == b'#' && copy[i + 1] == b'(' {
                copy[i] = b'_';
            }
        }
    }
    let mut out = Vec::new();
    utf8::strvis(
        &mut out,
        &copy,
        VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL,
    );
    Some(out)
}

pub fn mode_to_string(mode: ScreenMode) -> String {
    if mode.0 == 0 {
        return "NONE".into();
    }
    if mode == ScreenMode::ALL_MODES {
        return "ALL".into();
    }
    let mut out = String::new();
    for (flag, name) in [
        (ScreenMode::CURSOR, "CURSOR"),
        (ScreenMode::INSERT, "INSERT"),
        (ScreenMode::KCURSOR, "KCURSOR"),
        (ScreenMode::KKEYPAD, "KKEYPAD"),
        (ScreenMode::WRAP, "WRAP"),
        (ScreenMode::MOUSE_STANDARD, "MOUSE_STANDARD"),
        (ScreenMode::MOUSE_BUTTON, "MOUSE_BUTTON"),
        (ScreenMode::CURSOR_BLINKING, "CURSOR_BLINKING"),
        (ScreenMode::CURSOR_VERY_VISIBLE, "CURSOR_VERY_VISIBLE"),
        (ScreenMode::CURSOR_BLINKING_SET, "CURSOR_BLINKING_SET"),
        (ScreenMode::MOUSE_UTF8, "MOUSE_UTF8"),
        (ScreenMode::MOUSE_SGR, "MOUSE_SGR"),
        (ScreenMode::BRACKETPASTE, "BRACKETPASTE"),
        (ScreenMode::FOCUSON, "FOCUSON"),
        (ScreenMode::MOUSE_ALL, "MOUSE_ALL"),
        (ScreenMode::ORIGIN, "ORIGIN"),
        (ScreenMode::CRLF, "CRLF"),
        (ScreenMode::KEYS_EXTENDED, "KEYS_EXTENDED"),
        (ScreenMode::KEYS_EXTENDED_2, "KEYS_EXTENDED_2"),
        (ScreenMode::THEME_UPDATES, "THEME_UPDATES"),
        (ScreenMode::SYNC, "SYNC"),
    ] {
        if mode.intersects(flag) {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(name);
        }
    }
    out
}
