// Ported from tmux tmux.h, layout.c @ 8f25579c
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

use rmux_emu::screen::PaneLines;
use rmux_util::bytes::ByteString;

use crate::ids::{Arena, LayoutCellId, PaneId, WindowId};
use crate::model::PaneFlags;
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use crate::ui::status::PaneStatusPosition;

pub mod custom;
pub mod host;
pub mod set;
pub mod tree;

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod tests;

pub use custom::{dump, parse};
pub use set::{set_lookup, set_next, set_previous, set_select};
pub use tree::*;

/// `PANE_MINIMUM` (`tmux.h:112`).
pub const PANE_MINIMUM: u32 = 1;
/// `PANE_MAXIMUM` (`tmux.h:113`).
pub const PANE_MAXIMUM: u32 = 10000;
/// `WINDOW_MAXIMUM` (`tmux.h:117`).
pub const WINDOW_MAXIMUM: u32 = 10000;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LayoutType {
    Windowpane = 2,
    Leftright = 0,
    Topbottom = 1,
}
impl TryFrom<i32> for LayoutType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            2 => Ok(Self::Windowpane),
            0 => Ok(Self::Leftright),
            1 => Ok(Self::Topbottom),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct LayoutCellFlags(pub u32);
impl LayoutCellFlags {
    pub const FLOATING: Self = Self(1);
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
impl std::ops::BitOr for LayoutCellFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for LayoutCellFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for LayoutCellFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct LayoutDumpFlags(pub u32);
impl LayoutDumpFlags {
    /// `LAYOUT_CUSTOM_OLD_FORMAT` (`tmux.h:3917`).
    pub const OLD_FORMAT: Self = Self(1);
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
impl std::ops::BitOr for LayoutDumpFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for LayoutDumpFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for LayoutDumpFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// `struct layout_geometry` (`tmux.h:1580-1585`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LayoutGeometry {
    pub sx: u32,
    pub sy: u32,
    pub xoff: i32,
    pub yoff: i32,
}
impl LayoutGeometry {
    /// `layout_geometry_init` sentinels (`layout.c:59-66`).
    pub const UNSET: Self = Self {
        sx: u32::MAX,
        sy: u32::MAX,
        xoff: i32::MAX,
        yoff: i32::MAX,
    };
    pub const fn new(sx: u32, sy: u32, xoff: i32, yoff: i32) -> Self {
        Self { sx, sy, xoff, yoff }
    }
}
impl Default for LayoutGeometry {
    fn default() -> Self {
        Self::UNSET
    }
}

pub type LayoutChildren = Vec<LayoutCellId>;

/// `struct layout_cell` (`tmux.h:1591-1605`).
#[derive(Clone, Debug)]
pub struct LayoutCell {
    pub kind: LayoutType,
    pub flags: LayoutCellFlags,
    pub parent: Option<LayoutCellId>,
    pub g: LayoutGeometry,
    pub fg: LayoutGeometry,
    pub pane: Option<PaneId>,
    pub children: LayoutChildren,
}
impl LayoutCell {
    pub fn is_floating(&self) -> bool {
        self.flags.contains(LayoutCellFlags::FLOATING)
    }
    pub fn is_leaf(&self) -> bool {
        self.kind == LayoutType::Windowpane
    }
}

pub type Cells = Arena<LayoutCell, LayoutCellId>;

/// Index into the preset table of `layout-set.c:39-50`; `Window.lastlayout` is
/// `Option<LayoutSetIndex>` in place of the C `-1`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayoutSetIndex(pub u8);

/// `layout_split_sizes` results (`layout.c:1338-1365`): top/left, bottom/right
/// and the size of the cell before the split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SplitSizes {
    pub size1: u32,
    pub size2: u32,
    pub saved: u32,
}

/// The `*cause` bytes of a failed layout operation, byte-exact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutError {
    pub cause: ByteString,
}
impl LayoutError {
    pub fn new(cause: impl Into<ByteString>) -> Self {
        Self {
            cause: cause.into(),
        }
    }
}
impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cause)
    }
}
impl std::error::Error for LayoutError {}

/// What `layout_resize_check` and its callers read from `struct window`
/// (`layout.c:511-521`): the root, pane-border-status, the scrollbar mode and
/// the *active* pane scrollbar width and pad. Width and pad keep the signed
/// style values; the tree code converts them like C `u_int` arithmetic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutEnv {
    pub root: Option<LayoutCellId>,
    pub pane_status: PaneStatusPosition,
    pub scrollbars: PaneScrollbarPolicy,
    pub scrollbar_width: i32,
    pub scrollbar_pad: i32,
    pub window_sx: u32,
    pub window_sy: u32,
}

/// Cell storage plus the pane side of the leaf link. A bare `Cells` arena is a
/// detached tree (layout parse builds one before a window owns it): there are
/// no pane links, so the pane accessors are no-ops.
pub trait LayoutCells {
    fn cells(&self) -> &Cells;
    fn cells_mut(&mut self) -> &mut Cells;
    /// `wp->layout_cell`.
    fn pane_layout_cell(&self, wp: PaneId) -> Option<LayoutCellId>;
    fn set_pane_layout_cell(&mut self, wp: PaneId, lc: Option<LayoutCellId>);
}

impl LayoutCells for Cells {
    fn cells(&self) -> &Cells {
        self
    }
    fn cells_mut(&mut self) -> &mut Cells {
        self
    }
    fn pane_layout_cell(&self, _wp: PaneId) -> Option<LayoutCellId> {
        None
    }
    fn set_pane_layout_cell(&mut self, _wp: PaneId, _lc: Option<LayoutCellId>) {}
}

/// Window and pane state that `layout.c`, `layout-custom.c` and
/// `layout-set.c` read and write (G12 `window.c`, `resize.c`, G14 events,
/// G17 redraw). Each method is one C field access or call; the G12 `Server`
/// implements it. Default methods are the C helpers that only read the
/// primitives below (`window.c:628,1291,1328,1343,2970`).
pub trait LayoutHost: LayoutCells {
    // struct window
    fn window_public_id(&self, w: WindowId) -> u32;
    /// `w->sx`, `w->sy`.
    fn window_size(&self, w: WindowId) -> (u32, u32);
    fn window_layout_root(&self, w: WindowId) -> Option<LayoutCellId>;
    fn set_window_layout_root(&mut self, w: WindowId, root: Option<LayoutCellId>);
    /// `w->panes` in list order.
    fn window_panes(&self, w: WindowId) -> &[PaneId];
    /// `w->z_index`, front first.
    fn window_z_index(&self, w: WindowId) -> &[PaneId];
    fn window_z_index_mut(&mut self, w: WindowId) -> &mut Vec<PaneId>;
    /// `w->last_panes`, top first.
    fn window_last_panes(&self, w: WindowId) -> &[PaneId];
    fn window_active(&self, w: WindowId) -> Option<PaneId>;
    /// `w->sb`.
    fn window_scrollbars(&self, w: WindowId) -> PaneScrollbarPolicy;
    /// `w->sb_pos`.
    fn window_scrollbar_position(&self, w: WindowId) -> PaneScrollbarPosition;
    /// `window_get_pane_status` (`window.c:2933-2943`).
    fn window_pane_status(&self, w: WindowId) -> PaneStatusPosition;
    fn window_lastlayout(&self, w: WindowId) -> Option<LayoutSetIndex>;
    fn set_window_lastlayout(&mut self, w: WindowId, layout: Option<LayoutSetIndex>);
    /// `w->last_new_pane_x`, `w->last_new_pane_y`.
    fn window_last_new_pane(&self, w: WindowId) -> (i32, i32);
    fn set_window_last_new_pane(&mut self, w: WindowId, x: i32, y: i32);
    /// `options_get_string(w->options, name)`.
    fn window_option_string(&self, w: WindowId, name: &[u8]) -> &[u8];
    /// `options_get_number(w->options, name)`.
    fn window_option_number(&self, w: WindowId, name: &[u8]) -> i64;

    // struct window_pane
    fn pane_window(&self, wp: PaneId) -> WindowId;
    /// `wp->id` (the `%n` number).
    fn pane_public_id(&self, wp: PaneId) -> u32;
    fn pane_saved_layout_cell(&self, wp: PaneId) -> Option<LayoutCellId>;
    /// `(wp->xoff, wp->yoff, wp->sx, wp->sy)`.
    fn pane_geometry(&self, wp: PaneId) -> (i32, i32, u32, u32);
    fn set_pane_offset(&mut self, wp: PaneId, xoff: i32, yoff: i32);
    /// `(wp->scrollbar_style.width, wp->scrollbar_style.pad)`.
    fn pane_scrollbar_style(&self, wp: PaneId) -> (i32, i32);
    fn pane_flags(&self, wp: PaneId) -> PaneFlags;
    fn pane_flags_insert(&mut self, wp: PaneId, flags: PaneFlags);
    /// `window_pane_get_pane_status` (`window.c:2945-2968`).
    fn pane_status(&self, wp: PaneId) -> PaneStatusPosition;
    /// `window_pane_get_pane_lines` (`window.c:2921-2931`).
    fn pane_lines(&self, wp: PaneId) -> PaneLines;
    /// `window_pane_scrollbar_reserve` (`window.c:2640-2645`).
    fn pane_scrollbar_reserve(&self, wp: PaneId) -> bool;

    // G12 operations
    /// `window_pane_resize(wp, sx, sy)` (`window.c:1717`).
    fn pane_resize(&mut self, wp: PaneId, sx: u32, sy: u32);
    /// `window_resize(w, sx, sy, -1, -1)` (`window.c:575`).
    fn window_resize(&mut self, w: WindowId, sx: u32, sy: u32);
    /// `window_set_active_pane(w, wp, notify)` (`window.c:756`).
    fn window_set_active_pane(&mut self, w: WindowId, wp: PaneId, notify: bool);
    /// `window_pane_stack_push(&w->last_panes, wp)` (`window.c:2461`).
    fn window_last_panes_push(&mut self, w: WindowId, wp: PaneId);
    /// `window_pane_stack_remove(&w->last_panes, wp)` (`window.c:2472`).
    fn window_last_panes_remove(&mut self, w: WindowId, wp: PaneId);
    /// `window_push_zoom(w, always, flag)` (`window.c:1123`).
    fn window_push_zoom(&mut self, w: WindowId, always: bool, flag: bool) -> bool;
    /// `window_active_pane_is_over_zoom` (`window.c:1111`).
    fn window_active_pane_is_over_zoom(&self, w: WindowId) -> bool;
    /// `recalculate_sizes()` (`resize.c`).
    fn recalculate_sizes(&mut self);

    // effects
    /// `events_fire_window(name, w)`.
    fn fire_window_event(&mut self, w: WindowId, name: &str);
    /// `server_redraw_window(w)`.
    fn redraw_window(&mut self, w: WindowId);
    /// `redraw_invalidate_scene(w)`.
    fn invalidate_scene(&mut self, w: WindowId);

    // C helpers over the primitives above
    /// `window_pane_is_floating` (`window.c:2970-2978`).
    fn pane_is_floating(&self, wp: PaneId) -> bool {
        match self.pane_layout_cell(wp) {
            Some(lc) => self.cells().get(lc).is_some_and(LayoutCell::is_floating),
            None => false,
        }
    }
    /// `window_count_panes` (`window.c:1343-1353`).
    fn window_count_panes(&self, w: WindowId, with_floating: bool) -> u32 {
        let panes = self.window_panes(w);
        let mut n = 0;
        for i in 0..panes.len() {
            let wp = self.window_panes(w)[i];
            if with_floating || !self.pane_is_floating(wp) {
                n += 1;
            }
        }
        n
    }
    /// `window_has_floating_panes` (`window.c:628-637`).
    fn window_has_floating_panes(&self, w: WindowId) -> bool {
        let n = self.window_panes(w).len();
        (0..n).any(|i| self.pane_is_floating(self.window_panes(w)[i]))
    }
    /// `window_pane_index` (`window.c:1291-1305`): `pane-base-index` plus the
    /// list position.
    fn pane_index(&self, wp: PaneId) -> Option<u32> {
        let w = self.pane_window(wp);
        let base = self.window_option_number(w, b"pane-base-index") as u32;
        let pos = self.window_panes(w).iter().position(|&p| p == wp)?;
        Some(base.wrapping_add(pos as u32))
    }
    /// `window_pane_last_index` (`window.c:1328-1341`).
    fn pane_last_index(&self, wp: PaneId) -> Option<u32> {
        let w = self.pane_window(wp);
        self.window_last_panes(w)
            .iter()
            .position(|&p| p == wp)
            .map(|i| i as u32)
    }
    /// `Window::layout_env`: the snapshot `layout_resize_check` reads
    /// (`layout.c:511-521`).
    fn layout_env(&self, w: WindowId) -> LayoutEnv {
        let (width, pad) = match self.window_active(w) {
            Some(wp) => self.pane_scrollbar_style(wp),
            None => (0, 0),
        };
        let (window_sx, window_sy) = self.window_size(w);
        LayoutEnv {
            root: self.window_layout_root(w),
            pane_status: self.window_pane_status(w),
            scrollbars: self.window_scrollbars(w),
            scrollbar_width: width,
            scrollbar_pad: pad,
            window_sx,
            window_sy,
        }
    }
}
