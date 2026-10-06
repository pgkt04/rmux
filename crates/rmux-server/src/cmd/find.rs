// Ported from tmux cmd-find.c, cmd.c, tmux.h @ 8f25579c
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
pub enum CmdFindType {
    Pane = 0,
    Window = 1,
    Session = 2,
}
impl TryFrom<i32> for CmdFindType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Pane),
            1 => Ok(Self::Window),
            2 => Ok(Self::Session),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CmdFindFlags(pub u32);
impl CmdFindFlags {
    pub const PREFER_UNATTACHED: Self = Self(1);
    pub const QUIET: Self = Self(2);
    pub const WINDOW_INDEX: Self = Self(4);
    pub const DEFAULT_MARKED: Self = Self(8);
    pub const EXACT_SESSION: Self = Self(16);
    pub const EXACT_WINDOW: Self = Self(32);
    pub const CANFAIL: Self = Self(64);
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
impl std::ops::BitOr for CmdFindFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for CmdFindFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for CmdFindFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use crate::ids::{ClientId, PaneId, SessionId, WindowId, WinlinkId};
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::strtonum::strtonum;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CmdFindState {
    pub flags: CmdFindFlags,
    pub s: Option<SessionId>,
    pub wl: Option<WinlinkId>,
    pub w: Option<WindowId>,
    pub wp: Option<PaneId>,
    pub idx: i32,
}
impl Default for CmdFindState {
    fn default() -> Self {
        Self::clear(CmdFindFlags::default())
    }
}
impl CmdFindState {
    pub const fn clear(flags: CmdFindFlags) -> Self {
        Self {
            flags,
            s: None,
            wl: None,
            w: None,
            wp: None,
            idx: -1,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.s.is_none() && self.wl.is_none() && self.w.is_none() && self.wp.is_none()
    }
    pub fn copy_target_from(&mut self, src: &Self) {
        self.s = src.s;
        self.wl = src.wl;
        self.w = src.w;
        self.wp = src.wp;
        self.idx = src.idx;
    }
    pub fn is_valid(&self, model: &dyn ModelView) -> bool {
        let (Some(s), Some(wl), Some(w), Some(wp)) = (self.s, self.wl, self.w, self.wp) else {
            return false;
        };
        let (Some(sv), Some(lv), Some(wv), Some(pv)) = (
            model.session(s),
            model.winlink(wl),
            model.window(w),
            model.pane(wp),
        ) else {
            return false;
        };
        sv.alive
            && sv.winlinks.contains(&wl)
            && lv.window == w
            && lv.session == s
            && wv.panes.contains(&wp)
            && pv.window == w
    }
}

#[derive(Clone, Copy)]
pub struct SessionView<'a> {
    pub public_id: u32,
    pub name: &'a [u8],
    pub alive: bool,
    pub attached: u32,
    pub activity: (i64, i64),
    pub current: Option<WinlinkId>,
    pub winlinks: &'a [WinlinkId],
    pub last: Option<WinlinkId>,
}
#[derive(Clone, Copy)]
pub struct WinlinkView {
    pub session: SessionId,
    pub window: WindowId,
    pub index: i32,
}
#[derive(Clone, Copy)]
pub struct WindowView<'a> {
    pub public_id: u32,
    pub name: &'a [u8],
    pub active: Option<PaneId>,
    pub panes: &'a [PaneId],
    pub last: Option<PaneId>,
    pub modal: Option<PaneId>,
    pub pane_base_index: i32,
}
#[derive(Clone, Copy)]
pub struct PaneView<'a> {
    pub public_id: u32,
    pub window: WindowId,
    pub tty: &'a [u8],
    pub live_tty: bool,
}
#[derive(Clone, Copy)]
pub struct ClientView<'a> {
    pub name: &'a [u8],
    pub tty: &'a [u8],
    pub session: Option<SessionId>,
    pub activity: (i64, i64),
    pub rmux_pane: Option<&'a [u8]>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Sessions visit in byte-name order, panes/windows in public-id order, clients in client-list order.
/// Session winlinks are index ordered; window panes are in pane-list order.
pub trait ModelView {
    fn session(&self, id: SessionId) -> Option<SessionView<'_>>;
    fn winlink(&self, id: WinlinkId) -> Option<WinlinkView>;
    fn window(&self, id: WindowId) -> Option<WindowView<'_>>;
    fn pane(&self, id: PaneId) -> Option<PaneView<'_>>;
    fn client(&self, id: ClientId) -> Option<ClientView<'_>>;
    fn sessions(&self, visit: &mut dyn FnMut(SessionId));
    fn windows(&self, visit: &mut dyn FnMut(WindowId));
    fn panes(&self, visit: &mut dyn FnMut(PaneId));
    fn clients(&self, visit: &mut dyn FnMut(ClientId));
    fn marked(&self) -> Option<CmdFindState>;
    fn adjacent_pane(&self, pane: PaneId, direction: PaneDirection) -> Option<PaneId>;
    fn pane_description(&self, window: WindowId, description: &[u8]) -> Option<PaneId>;
}

/// G15 adapters supply resolved ids plus the raw geometry, without constructing model objects.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseInput {
    pub valid: bool,
    pub session: Option<SessionId>,
    pub window: Option<WindowId>,
    pub pane: Option<PaneId>,
    pub x: u32,
    pub y: u32,
    pub last_x: u32,
    pub last_y: u32,
    pub b: u32,
    pub lb: u32,
    pub sgr_type: u8,
    pub sgr_b: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub status_at: i32,
    pub status_lines: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
pub fn mouse_at(pane: PaneGeometry, m: &MouseInput, last: bool) -> Option<(u32, u32)> {
    let x = (if last { m.last_x } else { m.x }).wrapping_add(m.offset_x);
    let mut y = (if last { m.last_y } else { m.y }).wrapping_add(m.offset_y);
    if m.status_at == 0 && y >= m.status_lines {
        y -= m.status_lines;
    }
    let (sx, sy) = (i64::from(x as i32), i64::from(y as i32));
    if sx < i64::from(pane.x)
        || sx >= i64::from(pane.x) + i64::from(pane.width)
        || sy < i64::from(pane.y)
        || sy >= i64::from(pane.y) + i64::from(pane.height)
    {
        return None;
    }
    Some((x.wrapping_sub(pane.x as u32), y.wrapping_sub(pane.y as u32)))
}
pub fn mouse_window(
    model: &dyn ModelView,
    m: &MouseInput,
) -> Option<(SessionId, Option<WinlinkId>)> {
    if !m.valid {
        return None;
    }
    let s = m.session?;
    let sv = model.session(s)?;
    let wl = if let Some(w) = m.window {
        model.window(w)?;
        sv.winlinks
            .iter()
            .copied()
            .find(|wl| model.winlink(*wl).is_some_and(|v| v.window == w))
    } else {
        sv.current
    };
    Some((s, wl))
}
pub fn mouse_pane(model: &dyn ModelView, m: &MouseInput) -> Option<(SessionId, WinlinkId, PaneId)> {
    let (s, wl) = mouse_window(model, m)?;
    let wl = wl?;
    let w = model.winlink(wl)?.window;
    let wv = model.window(w)?;
    let pane = m.pane.or(wv.active)?;
    model.pane(pane)?;
    if !wv.panes.contains(&pane) || wv.modal.is_some_and(|modal| modal != pane) {
        return None;
    }
    Some((s, wl, pane))
}

fn session_usable(model: &dyn ModelView, s: SessionId) -> bool {
    model.session(s).is_some_and(|v| {
        v.alive
            && v.current
                .and_then(|wl| model.winlink(wl))
                .and_then(|wl| model.window(wl.window))
                .is_some_and(|w| w.active.is_some())
    })
}
fn best_session(
    model: &dyn ModelView,
    flags: CmdFindFlags,
    window: Option<WindowId>,
) -> Option<SessionId> {
    let mut best = None;
    model.sessions(&mut |id| {
        if !session_usable(model, id) {
            return;
        }
        let sv = model.session(id).expect("visited session");
        if window.is_some_and(|w| {
            !sv.winlinks
                .iter()
                .any(|wl| model.winlink(*wl).is_some_and(|v| v.window == w))
        }) {
            return;
        }
        let better = best.and_then(|b| model.session(b)).is_none_or(|b| {
            if flags.contains(CmdFindFlags::PREFER_UNATTACHED)
                && (sv.attached == 0) != (b.attached == 0)
            {
                sv.attached == 0
            } else {
                sv.activity > b.activity
            }
        });
        if better {
            best = Some(id)
        }
    });
    best
}
pub fn best_client(model: &dyn ModelView, session: SessionId) -> Option<ClientId> {
    let filter = model
        .session(session)
        .filter(|s| s.attached != 0)
        .map(|_| session);
    let mut best = None;
    model.clients(&mut |id| {
        let Some(v) = model.client(id) else { return };
        if v.session.is_none() || filter.is_some_and(|s| v.session != Some(s)) {
            return;
        }
        if best
            .and_then(|b| model.client(b))
            .is_none_or(|b| v.activity > b.activity)
        {
            best = Some(id)
        }
    });
    best
}
fn set_link(model: &dyn ModelView, fs: &mut CmdFindState, wl: WinlinkId, index: bool) -> bool {
    let Some(v) = model.winlink(wl) else {
        return false;
    };
    fs.wl = Some(wl);
    fs.w = Some(v.window);
    if index {
        fs.idx = v.index;
    }
    true
}
fn active(model: &dyn ModelView, fs: &mut CmdFindState) {
    fs.wp = fs.w.and_then(|w| model.window(w)).and_then(|w| w.active);
}
fn best_link(model: &dyn ModelView, fs: &mut CmdFindState) -> bool {
    let (Some(s), Some(w)) = (fs.s, fs.w) else {
        return false;
    };
    let Some(sv) = model.session(s) else {
        return false;
    };
    let wl = sv
        .current
        .filter(|wl| model.winlink(*wl).is_some_and(|v| v.window == w))
        .or_else(|| {
            sv.winlinks
                .iter()
                .copied()
                .find(|wl| model.winlink(*wl).is_some_and(|v| v.window == w))
        });
    wl.is_some_and(|wl| set_link(model, fs, wl, true))
}
fn best_session_window(model: &dyn ModelView, fs: &mut CmdFindState) -> bool {
    fs.s = best_session(model, fs.flags, fs.w);
    best_link(model, fs)
}
pub fn from_session(model: &dyn ModelView, s: SessionId, flags: CmdFindFlags) -> CmdFindState {
    let mut fs = CmdFindState::clear(flags);
    fs.s = Some(s);
    if let Some(wl) = model.session(s).and_then(|s| s.current) {
        set_link(model, &mut fs, wl, false);
        active(model, &mut fs);
    }
    fs
}
pub fn from_winlink(model: &dyn ModelView, wl: WinlinkId, flags: CmdFindFlags) -> CmdFindState {
    let mut fs = CmdFindState::clear(flags);
    fs.s = model.winlink(wl).map(|wl| wl.session);
    set_link(model, &mut fs, wl, false);
    active(model, &mut fs);
    fs
}
pub fn from_winlink_pane(
    model: &dyn ModelView,
    wl: WinlinkId,
    wp: PaneId,
    flags: CmdFindFlags,
) -> CmdFindState {
    let mut fs = from_winlink(model, wl, flags);
    fs.idx = model.winlink(wl).map_or(-1, |v| v.index);
    fs.wp = Some(wp);
    fs
}
pub fn from_session_window(
    model: &dyn ModelView,
    s: SessionId,
    w: WindowId,
    flags: CmdFindFlags,
) -> Option<CmdFindState> {
    let mut fs = CmdFindState::clear(flags);
    fs.s = Some(s);
    fs.w = Some(w);
    if !best_link(model, &mut fs) {
        return None;
    }
    active(model, &mut fs);
    Some(fs)
}
pub fn from_window(
    model: &dyn ModelView,
    w: WindowId,
    flags: CmdFindFlags,
) -> Option<CmdFindState> {
    let mut fs = CmdFindState::clear(flags);
    fs.w = Some(w);
    if !best_session_window(model, &mut fs) {
        return None;
    }
    active(model, &mut fs);
    Some(fs)
}
pub fn from_pane(model: &dyn ModelView, wp: PaneId, flags: CmdFindFlags) -> Option<CmdFindState> {
    let mut fs = from_window(model, model.pane(wp)?.window, flags)?;
    fs.wp = Some(wp);
    Some(fs)
}
pub fn from_nothing(model: &dyn ModelView, flags: CmdFindFlags) -> Option<CmdFindState> {
    let s = best_session(model, flags, None)?;
    let mut fs = from_session(model, s, flags);
    fs.idx = fs
        .wl
        .and_then(|wl| model.winlink(wl))
        .map_or(-1, |wl| wl.index);
    Some(fs)
}
fn inside_pane(model: &dyn ModelView, client: ClientId) -> Option<PaneId> {
    let cv = model.client(client)?;
    let mut pane = None;
    model.panes(&mut |id| {
        if pane.is_none()
            && model
                .pane(id)
                .is_some_and(|v| v.live_tty && v.tty == cv.tty)
        {
            pane = Some(id)
        }
    });
    pane.or_else(|| cv.rmux_pane.and_then(|s| pane_id(model, s)))
}
pub fn from_client(
    model: &dyn ModelView,
    client: Option<ClientId>,
    flags: CmdFindFlags,
) -> Option<CmdFindState> {
    let Some(c) = client else {
        return from_nothing(model, flags);
    };
    let cv = model.client(c)?;
    if let Some(s) = cv.session {
        return Some(from_session(model, s, flags));
    }
    if let Some(w) = inside_pane(model, c)
        .and_then(|p| model.pane(p))
        .map(|p| p.window)
        && let Some(mut fs) = from_window(model, w, flags)
    {
        let wl = model.session(fs.s?)?.current?;
        set_link(model, &mut fs, wl, false);
        active(model, &mut fs);
        return Some(fs);
    }
    from_nothing(model, flags)
}
pub fn from_mouse(
    model: &dyn ModelView,
    m: &MouseInput,
    flags: CmdFindFlags,
) -> Option<CmdFindState> {
    let (s, wl, wp) = mouse_pane(model, m)?;
    let mut fs = from_winlink(model, wl, flags);
    fs.s = Some(s);
    fs.wp = Some(wp);
    Some(fs)
}

fn numeric(s: &[u8], min: i64, max: i64) -> Option<i64> {
    strtonum(s, min, max).ok()
}
fn pane_id(model: &dyn ModelView, text: &[u8]) -> Option<PaneId> {
    let n = numeric(text.strip_prefix(b"%")?, 0, i64::from(u32::MAX))? as u32;
    let mut found = None;
    model.panes(&mut |id| {
        if model.pane(id).is_some_and(|v| v.public_id == n) {
            found = Some(id)
        }
    });
    found
}
fn window_id(model: &dyn ModelView, text: &[u8]) -> Option<WindowId> {
    let n = numeric(text.strip_prefix(b"@")?, 0, i64::from(u32::MAX))? as u32;
    let mut found = None;
    model.windows(&mut |id| {
        if model.window(id).is_some_and(|v| v.public_id == n) {
            found = Some(id)
        }
    });
    found
}
fn named_client(model: &dyn ModelView, target: &[u8]) -> Option<ClientId> {
    let target = target.strip_suffix(b":").unwrap_or(target);
    let mut found = None;
    model.clients(&mut |id| {
        let Some(c) = model.client(id) else { return };
        if found.is_none()
            && c.session.is_some()
            && (target == c.name
                || (!c.tty.is_empty()
                    && (target == c.tty
                        || c.tty
                            .strip_prefix(b"/dev/")
                            .is_some_and(|tty| target == tty))))
        {
            found = Some(id)
        }
    });
    found
}
pub fn client(
    model: &dyn ModelView,
    current: Option<ClientId>,
    target: Option<&[u8]>,
    quiet: bool,
) -> Result<Option<ClientId>, ByteString> {
    let found = if let Some(target) = target {
        named_client(model, cstr(target))
    } else if current.is_some_and(|c| model.client(c).is_some_and(|c| c.session.is_some())) {
        current
    } else {
        let s = if let Some(pane) = current.and_then(|c| inside_pane(model, c)) {
            best_session(
                model,
                CmdFindFlags::QUIET,
                model.pane(pane).map(|p| p.window),
            )
        } else {
            best_session(model, CmdFindFlags::QUIET, None)
        };
        s.and_then(|s| best_client(model, s))
    };
    if found.is_some() || quiet {
        Ok(found)
    } else if let Some(target) = target {
        let mut msg = b"can't find client: ".to_vec();
        msg.extend_from_slice(cstr(target).strip_suffix(b":").unwrap_or(cstr(target)));
        Err(ByteString(msg))
    } else {
        Err(ByteString::from("no current client"))
    }
}

fn get_session(model: &dyn ModelView, fs: &mut CmdFindState, text: &[u8]) -> bool {
    fs.s = None;
    if let Some(rest) = text.strip_prefix(b"$") {
        let Some(n) = numeric(rest, 0, i64::from(u32::MAX)) else {
            return false;
        };
        model.sessions(&mut |id| {
            if model
                .session(id)
                .is_some_and(|s| i64::from(s.public_id) == n)
            {
                fs.s = Some(id)
            }
        });
        return fs.s.is_some();
    }
    model.sessions(&mut |id| {
        if model.session(id).is_some_and(|s| s.name == text) {
            fs.s = Some(id)
        }
    });
    if fs.s.is_some() {
        return true;
    }
    fs.s = named_client(model, text)
        .and_then(|c| model.client(c))
        .and_then(|c| c.session);
    if fs.s.is_some() {
        return true;
    }
    if fs.flags.contains(CmdFindFlags::EXACT_SESSION) {
        return false;
    }
    for pattern in [false, true] {
        let mut ambiguous = false;
        model.sessions(&mut |id| {
            let Some(s) = model.session(id) else { return };
            let matches = if pattern {
                rmux_sys::fnmatch::fnmatch(text, s.name, rmux_sys::fnmatch::FnmatchFlags::NONE)
            } else {
                s.name.starts_with(text)
            };
            if matches {
                if fs.s.is_some() {
                    ambiguous = true
                } else {
                    fs.s = Some(id)
                }
            }
        });
        if ambiguous {
            fs.s = None;
            return false;
        }
        if fs.s.is_some() {
            return true;
        }
    }
    false
}
fn offset(text: &[u8]) -> Option<(bool, usize)> {
    let first = *text.first()?;
    if first != b'+' && first != b'-' {
        return None;
    }
    let n = if text.len() == 1 {
        1
    } else {
        numeric(&text[1..], 1, i64::from(i32::MAX))? as usize
    };
    Some((first == b'+', n))
}
fn wrap_index(index: usize, len: usize, forward: bool, n: usize) -> usize {
    let n = n % len;
    if forward {
        (index + n) % len
    } else {
        (index + len - n) % len
    }
}
fn get_window_session(model: &dyn ModelView, fs: &mut CmdFindState, text: &[u8]) -> bool {
    let Some(sv) = fs.s.and_then(|s| model.session(s)) else {
        return false;
    };
    let Some(current) = sv.current else {
        return false;
    };
    set_link(model, fs, current, false);
    if text.starts_with(b"@") {
        fs.w = window_id(model, text);
        return fs.w.is_some() && best_link(model, fs);
    }
    let exact = fs.flags.contains(CmdFindFlags::EXACT_WINDOW);
    if !exact && (text.starts_with(b"+") || text.starts_with(b"-")) {
        let Some((forward, n)) = offset(text) else {
            return false;
        };
        let Some(cv) = model.winlink(current) else {
            return false;
        };
        if fs.flags.contains(CmdFindFlags::WINDOW_INDEX) {
            fs.idx = if forward {
                cv.index.checked_add(n as i32)
            } else {
                cv.index.checked_sub(n as i32).filter(|n| *n >= 0)
            }
            .unwrap_or(-1);
            return fs.idx >= 0;
        }
        if let Some(i) = sv.winlinks.iter().position(|wl| *wl == current) {
            let wl = sv.winlinks[wrap_index(i, sv.winlinks.len(), forward, n)];
            return set_link(model, fs, wl, true);
        }
    }
    if !exact {
        let special = match text {
            b"!" => Some(sv.last),
            b"^" => Some(sv.winlinks.first().copied()),
            b"$" => Some(sv.winlinks.last().copied()),
            _ => None,
        };
        if let Some(wl) = special {
            fs.wl = wl;
            return wl.is_some_and(|wl| set_link(model, fs, wl, true));
        }
    }
    if !text.starts_with(b"+")
        && !text.starts_with(b"-")
        && let Some(index) = numeric(text, 0, i64::from(i32::MAX))
    {
        fs.wl = sv.winlinks.iter().copied().find(|wl| {
            model
                .winlink(*wl)
                .is_some_and(|wl| i64::from(wl.index) == index)
        });
        if let Some(wl) = fs.wl {
            return set_link(model, fs, wl, true);
        }
        if fs.flags.contains(CmdFindFlags::WINDOW_INDEX) {
            fs.idx = index as i32;
            return true;
        }
    }
    for mode in 0..3 {
        fs.wl = None;
        for wl in sv.winlinks {
            let Some(name) = model
                .winlink(*wl)
                .and_then(|wl| model.window(wl.window))
                .map(|w| w.name)
            else {
                continue;
            };
            let matches = match mode {
                0 => name == text,
                1 => name.starts_with(text),
                _ => rmux_sys::fnmatch::fnmatch(text, name, rmux_sys::fnmatch::FnmatchFlags::NONE),
            };
            if matches {
                if fs.wl.is_some() {
                    return false;
                }
                fs.wl = Some(*wl);
            }
        }
        if let Some(wl) = fs.wl {
            return set_link(model, fs, wl, true);
        }
        if exact {
            return false;
        }
    }
    false
}
fn get_window(
    model: &dyn ModelView,
    fs: &mut CmdFindState,
    current: &CmdFindState,
    text: &[u8],
    only: bool,
) -> bool {
    if text.starts_with(b"@") {
        fs.w = window_id(model, text);
        return fs.w.is_some() && best_session_window(model, fs);
    }
    fs.s = current.s;
    if get_window_session(model, fs, text) {
        return true;
    }
    if !only && get_session(model, fs, text) {
        if let Some(wl) = fs.s.and_then(|s| model.session(s)).and_then(|s| s.current) {
            return set_link(
                model,
                fs,
                wl,
                !fs.flags.contains(CmdFindFlags::WINDOW_INDEX),
            );
        }
    }
    false
}
fn get_pane_window(model: &dyn ModelView, fs: &mut CmdFindState, text: &[u8]) -> bool {
    let Some(w) = fs.w else { return false };
    let Some(wv) = model.window(w) else {
        return false;
    };
    if text.starts_with(b"%") {
        fs.wp = pane_id(model, text);
        return fs
            .wp
            .and_then(|p| model.pane(p))
            .is_some_and(|p| p.window == w);
    }
    if text == b"!" {
        fs.wp = wv.last;
        return fs.wp.is_some();
    }
    let direction = match text {
        b"{up-of}" => Some(PaneDirection::Up),
        b"{down-of}" => Some(PaneDirection::Down),
        b"{left-of}" => Some(PaneDirection::Left),
        b"{right-of}" => Some(PaneDirection::Right),
        _ => None,
    };
    if let Some(direction) = direction {
        fs.wp = wv.active.and_then(|p| model.adjacent_pane(p, direction));
        return fs.wp.is_some();
    }
    if text.starts_with(b"+") || text.starts_with(b"-") {
        let Some((forward, n)) = offset(text) else {
            return false;
        };
        fs.wp = wv
            .active
            .and_then(|p| wv.panes.iter().position(|id| *id == p))
            .map(|i| wv.panes[wrap_index(i, wv.panes.len(), forward, n)]);
        if fs.wp.is_some() {
            return true;
        }
    }
    if let Some(index) = numeric(text, 0, i64::from(i32::MAX)) {
        fs.wp = index
            .checked_sub(i64::from(wv.pane_base_index))
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| wv.panes.get(i))
            .copied();
        if fs.wp.is_some() {
            return true;
        }
    }
    fs.wp = model.pane_description(w, text);
    fs.wp.is_some()
}
fn get_pane_session(model: &dyn ModelView, fs: &mut CmdFindState, text: &[u8]) -> bool {
    if text.starts_with(b"%") {
        fs.wp = pane_id(model, text);
        fs.w = fs.wp.and_then(|p| model.pane(p)).map(|p| p.window);
        return fs.w.is_some() && best_link(model, fs);
    }
    if let Some(wl) = fs.s.and_then(|s| model.session(s)).and_then(|s| s.current) {
        set_link(model, fs, wl, true);
    }
    get_pane_window(model, fs, text)
}
fn get_pane(
    model: &dyn ModelView,
    fs: &mut CmdFindState,
    current: &CmdFindState,
    text: &[u8],
    only: bool,
) -> bool {
    if text.starts_with(b"%") {
        fs.wp = pane_id(model, text);
        fs.w = fs.wp.and_then(|p| model.pane(p)).map(|p| p.window);
        return fs.w.is_some() && best_session_window(model, fs);
    }
    fs.copy_target_from(current);
    if get_pane_window(model, fs, text) {
        return true;
    }
    if !only && get_window(model, fs, current, text, false) {
        active(model, fs);
        return true;
    }
    false
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FindContext {
    pub client: Option<ClientId>,
    pub current: CmdFindState,
    pub mouse: MouseInput,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindError {
    pub state: CmdFindState,
    pub message: Option<ByteString>,
}
fn failure(fs: CmdFindState, message: &[u8], force: bool) -> Result<CmdFindState, FindError> {
    if fs.flags.contains(CmdFindFlags::CANFAIL) {
        return Ok(fs);
    }
    Err(FindError {
        state: fs,
        message: (force || !fs.flags.contains(CmdFindFlags::QUIET))
            .then(|| ByteString::from(message)),
    })
}
fn missing(fs: CmdFindState, kind: &[u8], text: &[u8]) -> Result<CmdFindState, FindError> {
    let mut message = b"can't find ".to_vec();
    message.extend_from_slice(kind);
    message.extend_from_slice(b": ");
    message.extend_from_slice(text);
    failure(fs, &message, false)
}
fn map_window(s: &[u8]) -> &[u8] {
    match s {
        b"{start}" => b"^",
        b"{last}" => b"!",
        b"{end}" => b"$",
        b"{next}" => b"+",
        b"{previous}" => b"-",
        _ => s,
    }
}
fn map_pane(s: &[u8]) -> &[u8] {
    match s {
        b"{last}" => b"!",
        b"{next}" => b"+",
        b"{previous}" => b"-",
        b"{top}" => b"top",
        b"{bottom}" => b"bottom",
        b"{left}" => b"left",
        b"{right}" => b"right",
        b"{top-left}" => b"top-left",
        b"{top-right}" => b"top-right",
        b"{bottom-left}" => b"bottom-left",
        b"{bottom-right}" => b"bottom-right",
        _ => s,
    }
}
pub fn target(
    model: &dyn ModelView,
    context: &FindContext,
    target: Option<&[u8]>,
    kind: CmdFindType,
    mut flags: CmdFindFlags,
) -> Result<CmdFindState, FindError> {
    if flags.contains(CmdFindFlags::CANFAIL) {
        flags.insert(CmdFindFlags::QUIET)
    }
    let mut fs = CmdFindState::clear(flags);
    let current = if flags.contains(CmdFindFlags::DEFAULT_MARKED) && model.marked().is_some() {
        model.marked()
    } else if context.current.is_valid(model) {
        Some(context.current)
    } else {
        from_client(model, context.client, flags)
    };
    let Some(current) = current.filter(|c| c.is_valid(model)) else {
        return failure(fs, b"no current target", false);
    };
    let text = target.map(cstr).unwrap_or_default();
    if text.is_empty() {
        fs.copy_target_from(&current);
        if flags.contains(CmdFindFlags::WINDOW_INDEX) {
            fs.idx = -1
        }
        return Ok(fs);
    }
    if matches!(text, b"@" | b"{active}" | b"{current}") {
        let Some(s) = context
            .client
            .and_then(|c| model.client(c))
            .and_then(|c| c.session)
        else {
            return failure(fs, b"no current client", true);
        };
        if let Some(wl) = model.session(s).and_then(|s| s.current) {
            set_link(model, &mut fs, wl, false);
            active(model, &mut fs);
        }
        return Ok(fs);
    }
    if matches!(text, b"=" | b"{mouse}") {
        if kind == CmdFindType::Pane
            && let Some((s, wl, pane)) = mouse_pane(model, &context.mouse)
        {
            fs.s = Some(s);
            set_link(model, &mut fs, wl, false);
            fs.wp = Some(pane);
            return Ok(fs);
        }
        if let Some((s, wl)) = mouse_window(model, &context.mouse) {
            fs.s = Some(s);
            if let Some(wl) = wl.or_else(|| model.session(s).and_then(|s| s.current)) {
                set_link(model, &mut fs, wl, false);
                active(model, &mut fs);
            }
        }
        return if fs.wp.is_some() {
            Ok(fs)
        } else {
            failure(fs, b"no mouse target", false)
        };
    }
    if matches!(text, b"~" | b"{marked}") {
        if let Some(marked) = model.marked() {
            fs.copy_target_from(&marked);
            return Ok(fs);
        }
        return failure(fs, b"no marked target", false);
    }
    let colon = text.iter().position(|b| *b == b':');
    let start = colon.map_or(0, |i| i + 1);
    let period = text[start..]
        .iter()
        .position(|b| *b == b'.')
        .map(|i| i + start);
    let (mut session, mut window, mut pane) = match (colon, period) {
        (Some(c), Some(p)) => (
            Some(&text[..c]),
            Some(&text[c + 1..p]),
            Some(&text[p + 1..]),
        ),
        (Some(c), None) => (Some(&text[..c]), Some(&text[c + 1..]), None),
        (None, Some(p)) => (None, Some(&text[..p]), Some(&text[p + 1..])),
        (None, None) => match text[0] {
            b'$' => (Some(text), None, None),
            b'@' => (None, Some(text), None),
            b'%' => (None, None, Some(text)),
            _ => match kind {
                CmdFindType::Session => (Some(text), None, None),
                CmdFindType::Window => (None, Some(text), None),
                CmdFindType::Pane => (None, None, Some(text)),
            },
        },
    };
    if session.is_some_and(|s| s.starts_with(b"=")) {
        session = session.map(|s| &s[1..]);
        fs.flags.insert(CmdFindFlags::EXACT_SESSION);
    }
    if window.is_some_and(|s| s.starts_with(b"=")) {
        window = window.map(|s| &s[1..]);
        fs.flags.insert(CmdFindFlags::EXACT_WINDOW);
    }
    session = session.filter(|s| !s.is_empty());
    window = window.filter(|s| !s.is_empty()).map(map_window);
    pane = pane.filter(|s| !s.is_empty()).map(map_pane);
    if pane.is_some() && flags.contains(CmdFindFlags::WINDOW_INDEX) {
        return failure(fs, b"can't specify pane here", false);
    }
    if let Some(s) = session {
        if !get_session(model, &mut fs, s) {
            return missing(fs, b"session", s);
        }
        match (window, pane) {
            (None, None) => {
                if let Some(wl) = fs.s.and_then(|s| model.session(s)).and_then(|s| s.current) {
                    set_link(model, &mut fs, wl, false);
                    fs.idx = -1;
                    active(model, &mut fs);
                }
            }
            (Some(w), p) => {
                if !get_window_session(model, &mut fs, w) {
                    return missing(fs, b"window", w);
                }
                if let Some(p) = p {
                    if !get_pane_window(model, &mut fs, p) {
                        return missing(fs, b"pane", p);
                    }
                } else if fs.wl.is_some() {
                    active(model, &mut fs);
                }
            }
            (None, Some(p)) => {
                if !get_pane_session(model, &mut fs, p) {
                    return missing(fs, b"pane", p);
                }
            }
        }
    } else if let Some(w) = window {
        if !get_window(model, &mut fs, &current, w, colon.is_some()) {
            return missing(fs, b"window", w);
        }
        if let Some(p) = pane {
            if !get_pane_window(model, &mut fs, p) {
                return missing(fs, b"pane", p);
            }
        } else if fs.wl.is_some() {
            active(model, &mut fs);
        }
    } else if let Some(p) = pane {
        if !get_pane(model, &mut fs, &current, p, period.is_some()) {
            return missing(fs, b"pane", p);
        }
    } else {
        fs.copy_target_from(&current);
        if flags.contains(CmdFindFlags::WINDOW_INDEX) {
            fs.idx = -1
        }
    }
    Ok(fs)
}

pub fn target_with_error(
    model: &dyn ModelView,
    context: &FindContext,
    text: Option<&[u8]>,
    kind: CmdFindType,
    flags: CmdFindFlags,
    report: &mut dyn FnMut(&[u8]),
) -> Result<CmdFindState, FindError> {
    let canfail = flags.contains(CmdFindFlags::CANFAIL);
    let mut lookup_flags = flags;
    if canfail {
        lookup_flags.insert(CmdFindFlags::QUIET);
        lookup_flags.remove(CmdFindFlags::CANFAIL);
    }
    match target(model, context, text, kind, lookup_flags) {
        Ok(mut state) => {
            if canfail {
                state.flags.insert(CmdFindFlags::CANFAIL);
            }
            Ok(state)
        }
        Err(mut cause) => {
            if let Some(message) = &cause.message {
                report(message);
            }
            if canfail {
                cause.state.flags.insert(CmdFindFlags::CANFAIL);
                Ok(cause.state)
            } else {
                Err(cause)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ids::ArenaId;

    pub(crate) fn sid(n: u32) -> SessionId {
        SessionId::from_parts(n, 0)
    }
    pub(crate) fn wid(n: u32) -> WindowId {
        WindowId::from_parts(n, 0)
    }
    pub(crate) fn lid(n: u32) -> WinlinkId {
        WinlinkId::from_parts(n, 0)
    }
    pub(crate) fn pid(n: u32) -> PaneId {
        PaneId::from_parts(n, 0)
    }
    pub(crate) fn cid(n: u32) -> ClientId {
        ClientId::from_parts(n, 0)
    }
    pub(crate) struct Fixture {
        pub sessions: Vec<SessionView<'static>>,
        pub links: Vec<WinlinkView>,
        pub windows: Vec<WindowView<'static>>,
        pub panes: Vec<PaneView<'static>>,
        pub clients: Vec<ClientView<'static>>,
        pub mark: Option<CmdFindState>,
    }
    impl Fixture {
        pub fn new() -> Self {
            static LINKS: std::sync::LazyLock<[[WinlinkId; 2]; 2]> =
                std::sync::LazyLock::new(|| [[lid(0), lid(1)], [lid(2), lid(3)]]);
            static PANES: std::sync::LazyLock<[PaneId; 5]> =
                std::sync::LazyLock::new(|| [pid(0), pid(1), pid(2), pid(3), pid(4)]);
            let links = &*LINKS;
            let panes = &*PANES;
            let (l0, l1, p0, p1, p2) = (
                &links[0][..],
                &links[1][..],
                &panes[..3],
                &panes[3..4],
                &panes[4..],
            );
            Self {
                sessions: vec![
                    SessionView {
                        public_id: 10,
                        name: b"alpha",
                        alive: true,
                        attached: 1,
                        activity: (5, 0),
                        current: Some(lid(0)),
                        winlinks: l0,
                        last: Some(lid(1)),
                    },
                    SessionView {
                        public_id: 11,
                        name: b"beta",
                        alive: true,
                        attached: 1,
                        activity: (5, 0),
                        current: Some(lid(3)),
                        winlinks: l1,
                        last: Some(lid(2)),
                    },
                ],
                links: vec![
                    WinlinkView {
                        session: sid(0),
                        window: wid(0),
                        index: 2,
                    },
                    WinlinkView {
                        session: sid(0),
                        window: wid(1),
                        index: 5,
                    },
                    WinlinkView {
                        session: sid(1),
                        window: wid(0),
                        index: 3,
                    },
                    WinlinkView {
                        session: sid(1),
                        window: wid(2),
                        index: 7,
                    },
                ],
                windows: vec![
                    WindowView {
                        public_id: 20,
                        name: b"editor",
                        active: Some(pid(0)),
                        panes: p0,
                        last: Some(pid(2)),
                        modal: None,
                        pane_base_index: 1,
                    },
                    WindowView {
                        public_id: 21,
                        name: b"logs",
                        active: Some(pid(3)),
                        panes: p1,
                        last: None,
                        modal: None,
                        pane_base_index: 0,
                    },
                    WindowView {
                        public_id: 22,
                        name: b"other",
                        active: Some(pid(4)),
                        panes: p2,
                        last: None,
                        modal: None,
                        pane_base_index: 0,
                    },
                ],
                panes: vec![
                    PaneView {
                        public_id: 30,
                        window: wid(0),
                        tty: b"/dev/ttyp0",
                        live_tty: true,
                    },
                    PaneView {
                        public_id: 31,
                        window: wid(0),
                        tty: b"/dev/ttyp1",
                        live_tty: true,
                    },
                    PaneView {
                        public_id: 32,
                        window: wid(0),
                        tty: b"/dev/ttyp2",
                        live_tty: true,
                    },
                    PaneView {
                        public_id: 33,
                        window: wid(1),
                        tty: b"/dev/ttyp3",
                        live_tty: true,
                    },
                    PaneView {
                        public_id: 34,
                        window: wid(2),
                        tty: b"/dev/ttyp4",
                        live_tty: true,
                    },
                ],
                clients: vec![
                    ClientView {
                        name: b"first",
                        tty: b"/dev/ttyA",
                        session: Some(sid(0)),
                        activity: (4, 0),
                        rmux_pane: None,
                    },
                    ClientView {
                        name: b"second",
                        tty: b"/dev/ttyB",
                        session: Some(sid(1)),
                        activity: (4, 0),
                        rmux_pane: None,
                    },
                    ClientView {
                        name: b"inside",
                        tty: b"/dev/ttyp0",
                        session: None,
                        activity: (9, 0),
                        rmux_pane: Some(b"%33"),
                    },
                ],
                mark: None,
            }
        }
        pub fn context(&self) -> FindContext {
            FindContext {
                client: Some(cid(0)),
                current: from_session(self, sid(0), CmdFindFlags::default()),
                mouse: MouseInput::default(),
            }
        }
        fn resolve(
            &self,
            text: &[u8],
            kind: CmdFindType,
            flags: CmdFindFlags,
        ) -> Result<CmdFindState, FindError> {
            target(self, &self.context(), Some(text), kind, flags)
        }
    }
    impl ModelView for Fixture {
        fn session(&self, id: SessionId) -> Option<SessionView<'_>> {
            self.sessions.get(id.parts().0 as usize).copied()
        }
        fn winlink(&self, id: WinlinkId) -> Option<WinlinkView> {
            self.links.get(id.parts().0 as usize).copied()
        }
        fn window(&self, id: WindowId) -> Option<WindowView<'_>> {
            self.windows.get(id.parts().0 as usize).copied()
        }
        fn pane(&self, id: PaneId) -> Option<PaneView<'_>> {
            self.panes.get(id.parts().0 as usize).copied()
        }
        fn client(&self, id: ClientId) -> Option<ClientView<'_>> {
            self.clients.get(id.parts().0 as usize).copied()
        }
        fn sessions(&self, visit: &mut dyn FnMut(SessionId)) {
            for i in 0..self.sessions.len() {
                visit(sid(i as u32));
            }
        }
        fn windows(&self, visit: &mut dyn FnMut(WindowId)) {
            for i in 0..self.windows.len() {
                visit(wid(i as u32));
            }
        }
        fn panes(&self, visit: &mut dyn FnMut(PaneId)) {
            for i in 0..self.panes.len() {
                visit(pid(i as u32));
            }
        }
        fn clients(&self, visit: &mut dyn FnMut(ClientId)) {
            for i in 0..self.clients.len() {
                visit(cid(i as u32));
            }
        }
        fn marked(&self) -> Option<CmdFindState> {
            self.mark
        }
        fn adjacent_pane(&self, pane: PaneId, direction: PaneDirection) -> Option<PaneId> {
            if pane != pid(0) {
                return None;
            }
            Some(match direction {
                PaneDirection::Up => pid(1),
                PaneDirection::Down => pid(2),
                PaneDirection::Left => pid(1),
                PaneDirection::Right => pid(2),
            })
        }
        fn pane_description(&self, window: WindowId, description: &[u8]) -> Option<PaneId> {
            if window != wid(0) {
                return None;
            }
            match description {
                b"top" | b"left" | b"top-left" | b"top-right" => Some(pid(0)),
                b"bottom" | b"right" | b"bottom-left" | b"bottom-right" => Some(pid(2)),
                _ => None,
            }
        }
    }
    #[test]
    fn state_membership_copies_and_constructors() {
        let f = Fixture::new();
        let state = from_session(&f, sid(0), CmdFindFlags::QUIET);
        assert!(state.is_valid(&f));
        assert_eq!(state.idx, -1);
        assert_eq!(from_winlink(&f, lid(1), CmdFindFlags::default()).idx, -1);
        assert_eq!(
            from_winlink_pane(&f, lid(1), pid(3), CmdFindFlags::default()).idx,
            5
        );
        assert_eq!(
            from_session_window(&f, sid(1), wid(0), CmdFindFlags::default())
                .unwrap()
                .wl,
            Some(lid(2))
        );
        assert!(from_session_window(&f, sid(0), wid(2), CmdFindFlags::default()).is_none());
        assert_eq!(
            from_pane(&f, pid(1), CmdFindFlags::default()).unwrap().wp,
            Some(pid(1))
        );
        let mut copy = CmdFindState::clear(CmdFindFlags::EXACT_WINDOW);
        copy.copy_target_from(&state);
        assert_eq!(copy.flags, CmdFindFlags::EXACT_WINDOW);
        for invalid in [
            CmdFindState {
                wl: Some(lid(2)),
                ..state
            },
            CmdFindState {
                w: Some(wid(1)),
                ..state
            },
            CmdFindState {
                wp: Some(pid(4)),
                ..state
            },
        ] {
            assert!(!invalid.is_valid(&f));
        }
        assert!(CmdFindState::default().is_empty());
    }
    #[test]
    fn session_and_client_choices_and_inside_pane() {
        let mut f = Fixture::new();
        assert_eq!(
            from_nothing(&f, CmdFindFlags::default()).unwrap().s,
            Some(sid(0))
        );
        f.sessions[1].attached = 0;
        assert_eq!(
            from_nothing(&f, CmdFindFlags::PREFER_UNATTACHED).unwrap().s,
            Some(sid(1))
        );
        assert_eq!(best_client(&f, sid(1)), Some(cid(0)));
        f.sessions[1].activity = (8, 0);
        let inside = from_client(&f, Some(cid(2)), CmdFindFlags::default()).unwrap();
        assert_eq!(inside.s, Some(sid(1)));
        assert_eq!(inside.w, Some(wid(2)));
        assert_eq!(inside.wp, Some(pid(4)));
        f.clients[2].tty = b"unknown";
        assert_eq!(
            from_client(&f, Some(cid(2)), CmdFindFlags::default())
                .unwrap()
                .s,
            Some(sid(0))
        );
        for name in [b"first".as_slice(), b"/dev/ttyA", b"ttyA", b"first:"] {
            assert_eq!(client(&f, None, Some(name), false).unwrap(), Some(cid(0)));
        }
        assert_eq!(client(&f, None, None, false).unwrap(), Some(cid(0)));
        assert!(client(&f, None, Some(b"first::"), false).is_err());
        f.sessions.iter_mut().for_each(|s| s.alive = false);
        assert!(from_nothing(&f, CmdFindFlags::default()).is_none());
        assert_eq!(
            client(&f, None, None, false).unwrap_err(),
            ByteString::from("no current client")
        );
    }
    #[test]
    fn session_target_ids_names_clients_prefix_patterns_exact() {
        let mut f = Fixture::new();
        for text in [
            b"$10".as_slice(),
            b"alpha",
            b"first",
            b"al",
            b"a*",
            b"=alpha",
        ] {
            assert_eq!(
                f.resolve(text, CmdFindType::Session, CmdFindFlags::default())
                    .unwrap()
                    .s,
                Some(sid(0))
            );
        }
        assert!(
            f.resolve(b"=al", CmdFindType::Session, CmdFindFlags::default())
                .is_err()
        );
        f.sessions[1].name = b"alpine";
        assert!(
            f.resolve(b"al", CmdFindType::Session, CmdFindFlags::default())
                .is_err()
        );
        assert!(
            f.resolve(b"a*", CmdFindType::Session, CmdFindFlags::default())
                .is_err()
        );
        assert!(
            f.resolve(
                b"$4294967296",
                CmdFindType::Session,
                CmdFindFlags::default()
            )
            .is_err()
        );
    }
    #[test]
    fn window_target_all_lookup_branches_and_index_limits() {
        let mut f = Fixture::new();
        for (text, expected) in [
            (b"@21".as_slice(), lid(1)),
            (b"alpha:@21", lid(1)),
            (b"alpha:5", lid(1)),
            (b"alpha:=5", lid(1)),
            (b"alpha:logs", lid(1)),
            (b"alpha:lo", lid(1)),
            (b"alpha:l*", lid(1)),
            (b"alpha:{next}", lid(1)),
            (b"alpha:{previous}", lid(1)),
            (b"alpha:{last}", lid(1)),
            (b"alpha:{end}", lid(1)),
            (b"alpha:{start}", lid(0)),
            (b"alpha:+2", lid(0)),
            (b"beta", lid(3)),
        ] {
            assert_eq!(
                f.resolve(text, CmdFindType::Window, CmdFindFlags::default())
                    .unwrap()
                    .wl,
                Some(expected),
                "{text:?}"
            );
        }
        assert!(
            f.resolve(b"alpha:@22", CmdFindType::Window, CmdFindFlags::default())
                .is_err()
        );
        assert!(
            f.resolve(b":beta", CmdFindType::Window, CmdFindFlags::default())
                .is_err()
        );
        assert!(
            f.resolve(b"alpha:=lo", CmdFindType::Window, CmdFindFlags::default())
                .is_err()
        );
        assert!(
            f.resolve(b"alpha:+0", CmdFindType::Window, CmdFindFlags::default())
                .is_err()
        );
        let index = f
            .resolve(b"alpha:99", CmdFindType::Window, CmdFindFlags::WINDOW_INDEX)
            .unwrap();
        assert_eq!(index.idx, 99);
        assert_eq!(index.wl, None);
        assert_eq!(index.w, Some(wid(0)));
        assert_eq!(
            f.resolve(b"alpha:+3", CmdFindType::Window, CmdFindFlags::WINDOW_INDEX)
                .unwrap()
                .idx,
            5
        );
        assert!(
            f.resolve(b"alpha:-3", CmdFindType::Window, CmdFindFlags::WINDOW_INDEX)
                .is_err()
        );
        f.links[0].index = i32::MAX;
        assert!(
            f.resolve(b"alpha:+", CmdFindType::Window, CmdFindFlags::WINDOW_INDEX)
                .is_err()
        );
        f.windows[1].name = b"editor";
        assert!(
            f.resolve(
                b"alpha:editor",
                CmdFindType::Window,
                CmdFindFlags::default()
            )
            .is_err()
        );
        assert!(
            f.resolve(b"alpha:ed", CmdFindType::Window, CmdFindFlags::default())
                .is_err()
        );
        assert!(
            f.resolve(b"alpha:e*", CmdFindType::Window, CmdFindFlags::default())
                .is_err()
        );
    }
    #[test]
    fn pane_target_all_parts_offsets_directions_descriptions() {
        let f = Fixture::new();
        for (text, expected) in [
            (b"%31".as_slice(), pid(1)),
            (b"alpha:.%31", pid(1)),
            (b"alpha:editor.%31", pid(1)),
            (b"editor.2", pid(1)),
            (b"2", pid(1)),
            (b"+", pid(1)),
            (b"-", pid(2)),
            (b"+3", pid(0)),
            (b"{last}", pid(2)),
            (b"{next}", pid(1)),
            (b"{previous}", pid(2)),
            (b"{up-of}", pid(1)),
            (b"{down-of}", pid(2)),
            (b"{left-of}", pid(1)),
            (b"{right-of}", pid(2)),
            (b"logs", pid(3)),
            (b"beta", pid(4)),
            (b"alpha:", pid(0)),
            (b"alpha:.1", pid(0)),
        ] {
            assert_eq!(
                f.resolve(text, CmdFindType::Pane, CmdFindFlags::default())
                    .unwrap()
                    .wp,
                Some(expected),
                "{text:?}"
            );
        }
        for text in [
            b"{top}".as_slice(),
            b"{bottom}",
            b"{left}",
            b"{right}",
            b"{top-left}",
            b"{top-right}",
            b"{bottom-left}",
            b"{bottom-right}",
        ] {
            assert!(
                f.resolve(text, CmdFindType::Pane, CmdFindFlags::default())
                    .is_ok()
            );
        }
        for text in [
            b".logs".as_slice(),
            b"alpha:logs.%31",
            b"alpha:.%34",
            b"+0",
            b"%999",
        ] {
            assert!(
                f.resolve(text, CmdFindType::Pane, CmdFindFlags::default())
                    .is_err()
            );
        }
        assert_eq!(
            f.resolve(b".1", CmdFindType::Window, CmdFindFlags::WINDOW_INDEX)
                .unwrap_err()
                .message
                .unwrap(),
            ByteString::from("can't specify pane here")
        );
    }
    #[test]
    fn current_mark_mouse_and_partial_can_fail() {
        let mut f = Fixture::new();
        assert_eq!(
            f.resolve(b"", CmdFindType::Pane, CmdFindFlags::WINDOW_INDEX)
                .unwrap()
                .idx,
            -1
        );
        for text in [b"@".as_slice(), b"{active}", b"{current}"] {
            let fs = f
                .resolve(text, CmdFindType::Pane, CmdFindFlags::default())
                .unwrap();
            assert_eq!(fs.s, None);
            assert_eq!(fs.idx, -1);
            assert_eq!(fs.wp, Some(pid(0)));
        }
        assert!(
            f.resolve(b"~", CmdFindType::Pane, CmdFindFlags::default())
                .is_err()
        );
        f.mark = Some(from_winlink_pane(
            &f,
            lid(1),
            pid(3),
            CmdFindFlags::default(),
        ));
        for text in [b"~".as_slice(), b"{marked}", b""] {
            assert_eq!(
                f.resolve(text, CmdFindType::Pane, CmdFindFlags::DEFAULT_MARKED)
                    .unwrap()
                    .wp,
                Some(pid(3))
            );
        }
        let partial = f
            .resolve(b"alpha:nope", CmdFindType::Pane, CmdFindFlags::CANFAIL)
            .unwrap();
        assert_eq!(partial.s, Some(sid(0)));
        assert_eq!(partial.w, Some(wid(0)));
        assert_eq!(partial.wl, None);
        assert!(partial.flags.contains(CmdFindFlags::QUIET));
        let mut context = f.context();
        context.mouse = MouseInput {
            valid: true,
            session: Some(sid(0)),
            window: Some(wid(2)),
            ..MouseInput::default()
        };
        assert_eq!(mouse_window(&f, &context.mouse), Some((sid(0), None)));
        assert_eq!(
            target(
                &f,
                &context,
                Some(b"="),
                CmdFindType::Pane,
                CmdFindFlags::default()
            )
            .unwrap()
            .w,
            Some(wid(0))
        );
        context.mouse.window = Some(wid(0));
        context.mouse.pane = Some(pid(1));
        assert_eq!(
            from_mouse(&f, &context.mouse, CmdFindFlags::default())
                .unwrap()
                .idx,
            -1
        );
        assert_eq!(
            target(
                &f,
                &context,
                Some(b"{mouse}"),
                CmdFindType::Pane,
                CmdFindFlags::default()
            )
            .unwrap()
            .wp,
            Some(pid(1))
        );
        f.windows[0].modal = Some(pid(2));
        assert!(mouse_pane(&f, &context.mouse).is_none());
        context.mouse.valid = false;
        assert!(
            target(
                &f,
                &context,
                Some(b"="),
                CmdFindType::Pane,
                CmdFindFlags::default()
            )
            .is_err()
        );
        context.client = None;
        assert_eq!(
            target(
                &f,
                &context,
                Some(b"@"),
                CmdFindType::Pane,
                CmdFindFlags::QUIET
            )
            .unwrap_err()
            .message
            .unwrap(),
            ByteString::from("no current client")
        );
        let mut diagnostics = Vec::new();
        let state = target_with_error(
            &f,
            &context,
            Some(b"@"),
            CmdFindType::Pane,
            CmdFindFlags::CANFAIL,
            &mut |message| diagnostics.push(ByteString::from(message)),
        )
        .unwrap();
        assert!(state.flags.contains(CmdFindFlags::CANFAIL));
        assert_eq!(diagnostics, vec![ByteString::from("no current client")]);
        f.sessions.iter_mut().for_each(|s| s.alive = false);
        context.current = CmdFindState::default();
        assert_eq!(
            target(
                &f,
                &context,
                None,
                CmdFindType::Pane,
                CmdFindFlags::default()
            )
            .unwrap_err()
            .message
            .unwrap(),
            ByteString::from("no current target")
        );
    }
    #[test]
    fn mouse_geometry_viewport_status_signed_edges() {
        let pane = PaneGeometry {
            x: 10,
            y: 5,
            width: 4,
            height: 3,
        };
        let mut mouse = MouseInput {
            x: 8,
            y: 5,
            last_x: 9,
            last_y: 6,
            offset_x: 2,
            offset_y: 2,
            status_at: 0,
            status_lines: 2,
            ..MouseInput::default()
        };
        assert_eq!(mouse_at(pane, &mouse, false), Some((0, 0)));
        assert_eq!(mouse_at(pane, &mouse, true), Some((1, 1)));
        mouse.x = 12;
        assert_eq!(mouse_at(pane, &mouse, false), None);
        mouse.x = u32::MAX;
        mouse.offset_x = 0;
        assert_eq!(mouse_at(pane, &mouse, false), None);
        mouse.x = 0;
        mouse.y = 0;
        mouse.offset_y = 0;
        mouse.status_lines = 2;
        assert_eq!(
            mouse_at(
                PaneGeometry {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2
                },
                &mouse,
                false
            ),
            Some((0, 0))
        );
    }
    #[test]
    fn oracle_target_grammar_corpus() {
        use std::path::{Path, PathBuf};
        use std::process::Command as Process;
        let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
        if !oracle.is_file() {
            eprintln!(
                "skip target oracle comparison: {} missing",
                oracle.display()
            );
            return;
        }
        struct Socket {
            oracle: PathBuf,
            path: PathBuf,
        }
        impl Drop for Socket {
            fn drop(&mut self) {
                let _ = Process::new(&self.oracle)
                    .args(["-S"])
                    .arg(&self.path)
                    .arg("kill-server")
                    .output();
            }
        }
        let socket = Socket {
            oracle,
            // A short /tmp root keeps the path under the sun_path limit.
            path: PathBuf::from(format!(
                "/tmp/rmux-find-oracle-{}-{}.sock",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
        };
        let run = |args: &[&str]| {
            let output = Process::new(&socket.oracle)
                .arg("-S")
                .arg(&socket.path)
                .args(["-f", "/dev/null"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "oracle {args:?}: {:?}",
                output.stderr
            );
            output.stdout
        };
        run(&["new-session", "-d", "-s", "alpha", "-n", "editor"]);
        run(&["split-window", "-d", "-t", "alpha:0.0"]);
        run(&["split-window", "-d", "-t", "alpha:0.1"]);
        run(&["new-window", "-d", "-t", "alpha:2", "-n", "logs"]);
        run(&["select-window", "-t", "alpha:2"]);
        run(&["select-window", "-t", "alpha:0"]);
        run(&["select-pane", "-t", "alpha:0.2"]);
        run(&["select-pane", "-t", "alpha:0.0"]);
        let mut f = Fixture::new();
        f.sessions[1].alive = false;
        f.links[0].index = 0;
        f.links[1].index = 2;
        f.windows[0].pane_base_index = 0;
        f.windows[0].public_id = 0;
        f.windows[1].public_id = 1;
        f.panes[0].public_id = 0;
        f.panes[1].public_id = 1;
        f.panes[2].public_id = 2;
        for text in [
            "alpha:0.0",
            "alpha:0.1",
            "alpha:0.2",
            "alpha:2",
            "alpha:editor",
            "alpha:ed",
            "alpha:e*",
            "alpha:=0",
            "alpha:=editor",
            "alpha:^",
            "alpha:$",
            "alpha:!",
            "alpha:+",
            "alpha:-",
            "alpha:+2",
            "alpha:0.+",
            "alpha:0.-",
            "alpha:0.!",
            "alpha:0.%1",
            "alpha:@0.1",
        ] {
            let own = f
                .resolve(text.as_bytes(), CmdFindType::Pane, CmdFindFlags::default())
                .unwrap();
            let wl = f.winlink(own.wl.unwrap()).unwrap();
            let w = f.window(own.w.unwrap()).unwrap();
            let pane = w.panes.iter().position(|p| Some(*p) == own.wp).unwrap()
                + w.pane_base_index as usize;
            let expected = format!("alpha:{}.{}\n", wl.index, pane);
            assert_eq!(
                run(&[
                    "display-message",
                    "-p",
                    "-t",
                    text,
                    "#{session_name}:#{window_index}.#{pane_index}"
                ]),
                expected.as_bytes(),
                "{text}"
            );
        }
        for text in [
            "alpha:missing",
            "alpha:=ed",
            "alpha:0.%999",
            "alpha:2.%1",
            "alpha:+0",
            "alpha:0.+0",
            "missing:0",
            "alpha:0.99",
        ] {
            assert!(
                f.resolve(text.as_bytes(), CmdFindType::Pane, CmdFindFlags::default())
                    .is_err()
            );
            let output = Process::new(&socket.oracle)
                .arg("-S")
                .arg(&socket.path)
                .args(["select-pane", "-t", text])
                .output()
                .unwrap();
            assert!(!output.status.success(), "{text}");
        }
    }
}
