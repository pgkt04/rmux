// Ported from tmux window-copy.c @ 8f25579c
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
use crate::ids::{ModeId, PaneId, TimerId};
use crate::model::Server;
use rmux_emu::input::InputCtx;
use rmux_emu::screen::Screen;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ModeKeys {
    #[default]
    Emacs,
    Vi,
}
impl ModeKeys {
    pub fn as_i32(self) -> i32 {
        i32::from(self == Self::Vi)
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorDrag {
    #[default]
    None,
    Start,
    End,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LineSelectionDirection {
    #[default]
    None,
    LeftToRight,
    RightToLeft,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SelectionMode {
    #[default]
    Char,
    Word,
    Line,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CopyLineNumbers {
    Off,
    #[default]
    Option,
    Default,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchDirection {
    #[default]
    Off,
    Up,
    Down,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JumpKind {
    #[default]
    Off,
    Forward,
    Backward,
    ToForward,
    ToBackward,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecentreState {
    Top,
    #[default]
    Middle,
    Bottom,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyTimerAction {
    Drag(ModeId),
    Refresh(ModeId),
    ParserGround(ModeId),
}

#[derive(Debug)]
pub enum CopyBacking {
    Snapshot(Screen),
    Output {
        screen: Screen,
        parser: Box<InputCtx>,
        written: bool,
        ground_timer: Option<TimerId>,
    },
}
impl CopyBacking {
    pub fn screen(&self) -> &Screen {
        match self {
            Self::Snapshot(s) | Self::Output { screen: s, .. } => s,
        }
    }
    pub fn screen_mut(&mut self) -> &mut Screen {
        match self {
            Self::Snapshot(s) | Self::Output { screen: s, .. } => s,
        }
    }
    pub fn is_view(&self) -> bool {
        matches!(self, Self::Output { .. })
    }
}
#[derive(Debug, Default)]
pub struct CopySelection {
    pub active: bool,
    pub selx: u32,
    pub sely: u32,
    pub endselx: u32,
    pub endsely: u32,
    pub selrx: u32,
    pub selry: u32,
    pub endselrx: u32,
    pub endselry: u32,
    pub dx: u32,
    pub dy: u32,
    pub cursordrag: CursorDrag,
    pub lineflag: LineSelectionDirection,
    pub selflag: SelectionMode,
    pub rectflag: bool,
    pub separators: Vec<u8>,
}
#[derive(Debug)]
pub struct CopySearchState {
    pub term: Option<Vec<u8>>,
    pub regex: bool,
    pub searchtype: SearchDirection,
    pub searchdirection: SearchDirection,
    pub marks: Option<Vec<u8>>,
    pub count: i32,
    pub more: bool,
    pub all: bool,
    pub x: Option<u32>,
    pub y: Option<u32>,
    pub o: Option<u32>,
    pub generation: u8,
    pub scratch: Vec<u8>,
}
impl Default for CopySearchState {
    fn default() -> Self {
        Self {
            term: None,
            regex: false,
            searchtype: SearchDirection::Off,
            searchdirection: SearchDirection::Off,
            marks: None,
            count: 0,
            more: false,
            all: true,
            x: None,
            y: None,
            o: None,
            generation: 0,
            scratch: Vec::new(),
        }
    }
}
#[derive(Debug, Default)]
pub struct CopyJumpState {
    pub kind: JumpKind,
    pub character: Vec<u8>,
}
#[derive(Debug)]
pub struct CopyModeData {
    pub backing: CopyBacking,
    pub source: PaneId,
    pub cx: u32,
    pub cy: u32,
    pub oy: u32,
    pub lastcx: u32,
    pub lastsx: u32,
    pub selection: CopySelection,
    pub modekeys: ModeKeys,
    pub scroll_exit: bool,
    pub hide_position: bool,
    pub timeout: bool,
    pub search: CopySearchState,
    pub jump: CopyJumpState,
    pub mx: u32,
    pub my: u32,
    pub showmark: bool,
    pub line_numbers: CopyLineNumbers,
    pub recentre_state: RecentreState,
    pub recentre_line: u32,
    pub refresh_active: bool,
    pub refresh_timer: Option<TimerId>,
    pub dragtimer: Option<TimerId>,
    pub sync_added: u32,
    pub sync_collected: u32,
    pub sync_generation: u32,
}
impl CopyModeData {
    pub fn new(backing: CopyBacking, source: PaneId, modekeys: ModeKeys) -> Self {
        let s = backing.screen();
        let (cx, cy, oy) = (s.cx, s.cy, 0);
        Self {
            backing,
            source,
            cx,
            cy,
            oy,
            lastcx: 0,
            lastsx: 0,
            selection: CopySelection::default(),
            modekeys,
            scroll_exit: false,
            hide_position: false,
            timeout: false,
            search: CopySearchState::default(),
            jump: CopyJumpState::default(),
            mx: cx,
            my: cy,
            showmark: false,
            line_numbers: CopyLineNumbers::Option,
            recentre_state: RecentreState::Middle,
            recentre_line: 0,
            refresh_active: false,
            refresh_timer: None,
            dragtimer: None,
            sync_added: 0,
            sync_collected: 0,
            sync_generation: 0,
        }
    }
    pub fn backing_y(&self) -> u32 {
        self.backing.screen().grid.hsize().saturating_sub(self.oy) + self.cy
    }
}
pub fn data(server: &Server, mode: ModeId) -> Option<&CopyModeData> {
    server
        .panes
        .get(mode.owner)?
        .modes
        .iter()
        .find(|m| m.id == mode)?
        .data
        .as_ref()?
        .downcast_ref()
}
pub fn data_mut(server: &mut Server, mode: ModeId) -> Option<&mut CopyModeData> {
    server
        .panes
        .get_mut(mode.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == mode)?
        .data
        .as_mut()?
        .downcast_mut()
}
pub fn parts_mut(server: &mut Server, mode: ModeId) -> Option<(&mut CopyModeData, &mut Screen)> {
    let m = server
        .panes
        .get_mut(mode.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == mode)?;
    Some((m.data.as_mut()?.downcast_mut()?, m.screen.as_mut()?))
}
pub fn screen(server: &Server, mode: ModeId) -> Option<&Screen> {
    server
        .panes
        .get(mode.owner)?
        .modes
        .iter()
        .find(|m| m.id == mode)?
        .screen
        .as_ref()
}
