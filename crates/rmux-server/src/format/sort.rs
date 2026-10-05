// Ported from tmux sort.c, tmux.h @ 8f25579c
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

//! Shared ordering of buffers, clients, sessions, panes, winlinks and key
//! bindings. Comparators read model facts through [`SortModel`] and
//! [`SortClients`]; collectors fill caller-owned id vectors in the C
//! traversal order and then sort them in place. Ties keep collection order
//! (stable sort); `qsort` gives no tie contract.

use std::cmp::Ordering;

use rmux_util::bytes::cstr;
use rmux_util::key::{KeyCode, KeyMasks};

use crate::cmd::key_bindings::KeyBindings;
use crate::ids::{ClientId, KeyTableId, PaneId, PasteBufferId, SessionId, WindowId, WinlinkId};

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SortOrder {
    Activity = 0,
    Creation = 1,
    Index = 2,
    Modifier = 3,
    Name = 4,
    Order = 5,
    Size = 6,
    Z = 7,
    End = 8,
}
impl TryFrom<i32> for SortOrder {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Activity),
            1 => Ok(Self::Creation),
            2 => Ok(Self::Index),
            3 => Ok(Self::Modifier),
            4 => Ok(Self::Name),
            5 => Ok(Self::Order),
            6 => Ok(Self::Size),
            7 => Ok(Self::Z),
            8 => Ok(Self::End),
            _ => Err(value),
        }
    }
}

/// `struct sort_criteria` (`tmux.h:2570-2574`). `order_seq` is the caller's
/// available orders; a trailing `End` or the slice end terminates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortCriteria {
    pub order: SortOrder,
    pub reversed: bool,
    pub order_seq: Option<&'static [SortOrder]>,
}

impl Default for SortCriteria {
    fn default() -> Self {
        Self {
            order: SortOrder::Index,
            reversed: false,
            order_seq: None,
        }
    }
}

impl SortCriteria {
    pub const fn new(order: SortOrder, reversed: bool) -> Self {
        Self {
            order,
            reversed,
            order_seq: None,
        }
    }
}

/// A `struct timeval` as (seconds, microseconds), compared like `timercmp`.
pub type TimeVal = (i64, i64);

/// Model facts the session, winlink, pane and buffer comparators and
/// collectors read. Traversal methods push ids in the C container order;
/// stale ids give zero or empty facts.
pub trait SortModel {
    /// `RB_FOREACH(s, sessions, &sessions)`: name order.
    fn sessions(&self, out: &mut Vec<SessionId>);
    /// `RB_FOREACH(wl, winlinks, &s->windows)`: index order.
    fn winlinks(&self, session: SessionId, out: &mut Vec<WinlinkId>);
    /// `TAILQ_FOREACH(wp, &w->panes, entry)`.
    fn panes(&self, window: WindowId, out: &mut Vec<PaneId>);
    fn winlink_window(&self, winlink: WinlinkId) -> Option<WindowId>;
    /// `paste_walk` order (`paste.c:104-109`).
    fn buffers(&self, out: &mut Vec<PasteBufferId>);

    fn session_public_id(&self, session: SessionId) -> u32;
    fn session_created(&self, session: SessionId) -> TimeVal;
    fn session_activity(&self, session: SessionId) -> TimeVal;
    fn session_name(&self, session: SessionId) -> &[u8];

    fn winlink_index(&self, winlink: WinlinkId) -> i32;
    fn window_created(&self, window: WindowId) -> TimeVal;
    fn window_activity(&self, window: WindowId) -> TimeVal;
    fn window_name(&self, window: WindowId) -> &[u8];
    fn window_size(&self, window: WindowId) -> (u32, u32);

    fn pane_active_point(&self, pane: PaneId) -> u64;
    fn pane_public_id(&self, pane: PaneId) -> u32;
    fn pane_size(&self, pane: PaneId) -> (u32, u32);
    /// `window_pane_index` (`window.c:1291-1305`).
    fn pane_index(&self, pane: PaneId) -> u32;
    /// `window_pane_zindex` (`window.c:1308-1325`).
    fn pane_zindex(&self, pane: PaneId) -> u32;
    /// The pane screen title (`wp->screen->title`).
    fn pane_title(&self, pane: PaneId) -> &[u8];

    fn buffer_name(&self, buffer: PasteBufferId) -> &[u8];
    fn buffer_order(&self, buffer: PasteBufferId) -> u32;
    fn buffer_size(&self, buffer: PasteBufferId) -> usize;
}

/// Client facts for the client comparator and collector.
pub trait SortClients {
    /// `TAILQ_FOREACH(c, &clients, entry)`, unfiltered.
    fn clients(&self, out: &mut Vec<ClientId>);
    /// `!(flags & CLIENT_UNATTACHEDFLAGS) && (flags & CLIENT_ATTACHED)`
    /// (`sort.c:461-464`).
    fn client_sortable(&self, client: ClientId) -> bool;
    fn client_name(&self, client: ClientId) -> &[u8];
    /// `(tty.sx, tty.sy)`.
    fn client_size(&self, client: ClientId) -> (u32, u32);
    fn client_created(&self, client: ClientId) -> TimeVal;
    fn client_activity(&self, client: ClientId) -> TimeVal;
}

fn number_cmp<T: Ord>(a: T, b: T) -> Ordering {
    a.cmp(&b)
}

fn strcmp(a: &[u8], b: &[u8]) -> Ordering {
    cstr(a).cmp(cstr(b))
}

fn strcasecmp(a: &[u8], b: &[u8]) -> Ordering {
    cstr(a)
        .iter()
        .map(u8::to_ascii_lowercase)
        .cmp(cstr(b).iter().map(u8::to_ascii_lowercase))
}

fn finish(result: Ordering, crit: &SortCriteria) -> Ordering {
    if crit.reversed {
        result.reverse()
    } else {
        result
    }
}

/// `sort_qsort` (`sort.c:38-61`).
fn qsort<T>(l: &mut [T], crit: &SortCriteria, cmp: impl FnMut(&T, &T) -> Ordering) {
    if l.len() < 2 || crit.order == SortOrder::End {
        return;
    }
    if crit.order == SortOrder::Order {
        if crit.reversed {
            l.reverse();
        }
    } else {
        l.sort_by(cmp);
    }
}

/// `sort_buffer_cmp` (`sort.c:63-103`).
pub fn buffer_cmp<M: SortModel + ?Sized>(
    m: &M,
    crit: &SortCriteria,
    a: PasteBufferId,
    b: PasteBufferId,
) -> Ordering {
    let mut result = match crit.order {
        SortOrder::Name => strcmp(m.buffer_name(a), m.buffer_name(b)),
        SortOrder::Creation => m.buffer_order(b).cmp(&m.buffer_order(a)),
        SortOrder::Size => number_cmp(m.buffer_size(a), m.buffer_size(b)),
        _ => Ordering::Equal,
    };
    if result == Ordering::Equal {
        result = strcmp(m.buffer_name(a), m.buffer_name(b));
    }
    finish(result, crit)
}

/// `sort_client_cmp` (`sort.c:105-150`).
pub fn client_cmp<C: SortClients + ?Sized>(
    c: &C,
    crit: &SortCriteria,
    a: ClientId,
    b: ClientId,
) -> Ordering {
    let mut result = match crit.order {
        SortOrder::Name => strcmp(c.client_name(a), c.client_name(b)),
        SortOrder::Size => {
            let (ax, ay) = c.client_size(a);
            let (bx, by) = c.client_size(b);
            number_cmp(ax, bx).then_with(|| number_cmp(ay, by))
        }
        SortOrder::Creation => c.client_created(a).cmp(&c.client_created(b)),
        SortOrder::Activity => c.client_activity(b).cmp(&c.client_activity(a)),
        _ => Ordering::Equal,
    };
    if result == Ordering::Equal {
        result = strcmp(c.client_name(a), c.client_name(b));
    }
    finish(result, crit)
}

/// `sort_session_cmp` (`sort.c:152-203`).
pub fn session_cmp<M: SortModel + ?Sized>(
    m: &M,
    crit: &SortCriteria,
    a: SessionId,
    b: SessionId,
) -> Ordering {
    let mut result = match crit.order {
        SortOrder::Index => number_cmp(m.session_public_id(a), m.session_public_id(b)),
        SortOrder::Creation => m.session_created(a).cmp(&m.session_created(b)),
        SortOrder::Activity => m.session_activity(b).cmp(&m.session_activity(a)),
        SortOrder::Name => strcmp(m.session_name(a), m.session_name(b)),
        _ => Ordering::Equal,
    };
    if result == Ordering::Equal {
        result = strcmp(m.session_name(a), m.session_name(b));
    }
    finish(result, crit)
}

/// `sort_pane_cmp` (`sort.c:205-249`).
pub fn pane_cmp<M: SortModel + ?Sized>(
    m: &M,
    crit: &SortCriteria,
    a: PaneId,
    b: PaneId,
) -> Ordering {
    let area = |p: PaneId| {
        let (sx, sy) = m.pane_size(p);
        sx.wrapping_mul(sy)
    };
    let mut result = match crit.order {
        SortOrder::Activity => number_cmp(m.pane_active_point(a), m.pane_active_point(b)),
        SortOrder::Creation => number_cmp(m.pane_public_id(a), m.pane_public_id(b)),
        SortOrder::Size => number_cmp(area(a), area(b)),
        SortOrder::Index => number_cmp(m.pane_index(a), m.pane_index(b)),
        SortOrder::Name => strcmp(m.pane_title(a), m.pane_title(b)),
        SortOrder::Z => number_cmp(m.pane_zindex(a), m.pane_zindex(b)),
        _ => Ordering::Equal,
    };
    if result == Ordering::Equal {
        result = strcmp(m.pane_title(a), m.pane_title(b));
    }
    finish(result, crit)
}

/// `sort_winlink_cmp` (`sort.c:251-306`). A winlink without a window sorts
/// with empty facts.
pub fn winlink_cmp<M: SortModel + ?Sized>(
    m: &M,
    crit: &SortCriteria,
    a: WinlinkId,
    b: WinlinkId,
) -> Ordering {
    let wa = m.winlink_window(a);
    let wb = m.winlink_window(b);
    let name = |w: Option<WindowId>| w.map_or(&[][..], |w| m.window_name(w));
    let area = |w: Option<WindowId>| {
        w.map_or(0, |w| {
            let (sx, sy) = m.window_size(w);
            sx.wrapping_mul(sy)
        })
    };
    let time = |w: Option<WindowId>, f: fn(&M, WindowId) -> TimeVal| w.map_or((0, 0), |w| f(m, w));
    let mut result = match crit.order {
        SortOrder::Index => number_cmp(m.winlink_index(a), m.winlink_index(b)),
        SortOrder::Creation => time(wa, M::window_created).cmp(&time(wb, M::window_created)),
        SortOrder::Activity => time(wb, M::window_activity).cmp(&time(wa, M::window_activity)),
        SortOrder::Name => strcmp(name(wa), name(wb)),
        SortOrder::Size => number_cmp(area(wa), area(wb)),
        _ => Ordering::Equal,
    };
    if result == Ordering::Equal {
        result = strcmp(name(wa), name(wb));
    }
    finish(result, crit)
}

/// `sort_key_binding_cmp` (`sort.c:308-346`). A stale binding compares with
/// an empty table name.
pub fn key_binding_cmp(
    bindings: &KeyBindings,
    crit: &SortCriteria,
    a: (KeyTableId, KeyCode),
    b: (KeyTableId, KeyCode),
) -> Ordering {
    let table_name = |t: KeyTableId| bindings.tables.get(t).map_or(&[][..], |t| &t.name[..]);
    let mut result = match crit.order {
        SortOrder::Index => number_cmp(a.1.0, b.1.0),
        SortOrder::Modifier => number_cmp(a.1.0 & KeyMasks::MODIFIERS, b.1.0 & KeyMasks::MODIFIERS),
        SortOrder::Name => strcasecmp(table_name(a.0), table_name(b.0)),
        _ => Ordering::Equal,
    };
    if result == Ordering::Equal {
        result = strcasecmp(table_name(a.0), table_name(b.0));
    }
    if result == Ordering::Equal {
        result = number_cmp(a.1.0, b.1.0);
    }
    finish(result, crit)
}

/// `sort_next_order`: advance within the caller sequence, wrapping at its
/// end and starting at its first order when the current order is absent;
/// no sequence does nothing (`sort.c:348-368`).
pub fn next_order(crit: &mut SortCriteria) {
    let Some(seq) = crit.order_seq else {
        return;
    };
    let end = seq
        .iter()
        .position(|&o| o == SortOrder::End)
        .unwrap_or(seq.len());
    let seq = &seq[..end];
    let mut i = seq.iter().position(|&o| o == crit.order).unwrap_or(end);
    if i == end {
        i = 0;
    } else {
        i += 1;
        if i == end {
            i = 0;
        }
    }
    crit.order = seq.get(i).copied().unwrap_or(SortOrder::End);
}

/// `sort_order_from_string`: case-insensitive names, `key` aliases index,
/// `title` aliases name, anything else is `End` (`sort.c:370-394`).
pub fn order_from_string(order: Option<&[u8]>) -> SortOrder {
    let Some(order) = order else {
        return SortOrder::End;
    };
    let eq = |name: &str| strcasecmp(order, name.as_bytes()) == Ordering::Equal;
    if eq("activity") {
        SortOrder::Activity
    } else if eq("creation") {
        SortOrder::Creation
    } else if eq("index") || eq("key") {
        SortOrder::Index
    } else if eq("modifier") {
        SortOrder::Modifier
    } else if eq("name") || eq("title") {
        SortOrder::Name
    } else if eq("order") {
        SortOrder::Order
    } else if eq("size") {
        SortOrder::Size
    } else if eq("z") {
        SortOrder::Z
    } else {
        SortOrder::End
    }
}

/// `sort_order_to_string` (`sort.c:396-416`).
pub fn order_to_string(order: SortOrder) -> Option<&'static [u8]> {
    Some(match order {
        SortOrder::Activity => b"activity",
        SortOrder::Creation => b"creation",
        SortOrder::Index => b"index",
        SortOrder::Modifier => b"modifier",
        SortOrder::Name => b"name",
        SortOrder::Order => b"order",
        SortOrder::Size => b"size",
        SortOrder::Z => b"z",
        SortOrder::End => return None,
    })
}

/// `sort_would_window_tree_swap`: false for Index, otherwise whether the
/// winlink comparator is nonzero (`sort.c:418-426`).
pub fn would_window_tree_swap<M: SortModel + ?Sized>(
    m: &M,
    crit: &SortCriteria,
    a: WinlinkId,
    b: WinlinkId,
) -> bool {
    if crit.order == SortOrder::Index {
        return false;
    }
    winlink_cmp(m, crit, a, b) != Ordering::Equal
}

pub fn sort_buffers<M: SortModel + ?Sized>(m: &M, crit: &SortCriteria, l: &mut [PasteBufferId]) {
    qsort(l, crit, |&a, &b| buffer_cmp(m, crit, a, b));
}

pub fn sort_clients<C: SortClients + ?Sized>(c: &C, crit: &SortCriteria, l: &mut [ClientId]) {
    qsort(l, crit, |&a, &b| client_cmp(c, crit, a, b));
}

pub fn sort_sessions<M: SortModel + ?Sized>(m: &M, crit: &SortCriteria, l: &mut [SessionId]) {
    qsort(l, crit, |&a, &b| session_cmp(m, crit, a, b));
}

pub fn sort_panes<M: SortModel + ?Sized>(m: &M, crit: &SortCriteria, l: &mut [PaneId]) {
    qsort(l, crit, |&a, &b| pane_cmp(m, crit, a, b));
}

pub fn sort_winlinks<M: SortModel + ?Sized>(m: &M, crit: &SortCriteria, l: &mut [WinlinkId]) {
    qsort(l, crit, |&a, &b| winlink_cmp(m, crit, a, b));
}

pub fn sort_key_bindings(
    bindings: &KeyBindings,
    crit: &SortCriteria,
    l: &mut [(KeyTableId, KeyCode)],
) {
    qsort(l, crit, |&a, &b| key_binding_cmp(bindings, crit, a, b));
}

/// `sort_get_buffers` (`sort.c:428-449`).
pub fn get_buffers<M: SortModel + ?Sized>(
    m: &M,
    crit: &SortCriteria,
    out: &mut Vec<PasteBufferId>,
) {
    out.clear();
    m.buffers(out);
    sort_buffers(m, crit, out);
}

/// `sort_get_clients`: attached clients without unattached flags
/// (`sort.c:451-476`).
pub fn get_clients<C: SortClients + ?Sized>(c: &C, crit: &SortCriteria, out: &mut Vec<ClientId>) {
    out.clear();
    c.clients(out);
    out.retain(|&id| c.client_sortable(id));
    sort_clients(c, crit, out);
}

/// `sort_get_sessions` (`sort.c:478-499`).
pub fn get_sessions<M: SortModel + ?Sized>(m: &M, crit: &SortCriteria, out: &mut Vec<SessionId>) {
    out.clear();
    m.sessions(out);
    sort_sessions(m, crit, out);
}

fn collect_panes_session<M: SortModel + ?Sized>(
    m: &M,
    session: SessionId,
    winlinks: &mut Vec<WinlinkId>,
    out: &mut Vec<PaneId>,
) {
    winlinks.clear();
    m.winlinks(session, winlinks);
    for &wl in winlinks.iter() {
        if let Some(w) = m.winlink_window(wl) {
            m.panes(w, out);
        }
    }
}

/// `sort_get_panes`: every session, winlink and pane; linked windows repeat
/// their panes (`sort.c:501-530`).
pub fn get_panes<M: SortModel + ?Sized>(m: &M, crit: &SortCriteria, out: &mut Vec<PaneId>) {
    out.clear();
    let mut sessions = Vec::new();
    let mut winlinks = Vec::new();
    m.sessions(&mut sessions);
    for &s in &sessions {
        collect_panes_session(m, s, &mut winlinks, out);
    }
    sort_panes(m, crit, out);
}

/// `sort_get_panes_session` (`sort.c:532-559`).
pub fn get_panes_session<M: SortModel + ?Sized>(
    m: &M,
    session: SessionId,
    crit: &SortCriteria,
    out: &mut Vec<PaneId>,
) {
    out.clear();
    let mut winlinks = Vec::new();
    collect_panes_session(m, session, &mut winlinks, out);
    sort_panes(m, crit, out);
}

/// `sort_get_panes_window` (`sort.c:561-583`).
pub fn get_panes_window<M: SortModel + ?Sized>(
    m: &M,
    window: WindowId,
    crit: &SortCriteria,
    out: &mut Vec<PaneId>,
) {
    out.clear();
    m.panes(window, out);
    sort_panes(m, crit, out);
}

/// `sort_get_winlinks`: every winlink of every session (`sort.c:585-609`).
pub fn get_winlinks<M: SortModel + ?Sized>(m: &M, crit: &SortCriteria, out: &mut Vec<WinlinkId>) {
    out.clear();
    let mut sessions = Vec::new();
    m.sessions(&mut sessions);
    for &s in &sessions {
        m.winlinks(s, out);
    }
    sort_winlinks(m, crit, out);
}

/// `sort_get_winlinks_session` (`sort.c:611-633`).
pub fn get_winlinks_session<M: SortModel + ?Sized>(
    m: &M,
    session: SessionId,
    crit: &SortCriteria,
    out: &mut Vec<WinlinkId>,
) {
    out.clear();
    m.winlinks(session, out);
    sort_winlinks(m, crit, out);
}

fn collect_table(bindings: &KeyBindings, table: KeyTableId, out: &mut Vec<(KeyTableId, KeyCode)>) {
    if let Some(t) = bindings.tables.get(table) {
        out.extend(t.bindings().map(|bd| (table, bd.key)));
    }
}

/// `sort_get_key_bindings`: tables in name order, bindings in key order
/// (`sort.c:635-662`).
pub fn get_key_bindings(
    bindings: &KeyBindings,
    crit: &SortCriteria,
    out: &mut Vec<(KeyTableId, KeyCode)>,
) {
    out.clear();
    for table in bindings.tables() {
        collect_table(bindings, table, out);
    }
    sort_key_bindings(bindings, crit, out);
}

/// `sort_get_key_bindings_table` (`sort.c:664-687`).
pub fn get_key_bindings_table(
    bindings: &KeyBindings,
    table: KeyTableId,
    crit: &SortCriteria,
    out: &mut Vec<(KeyTableId, KeyCode)>,
) {
    out.clear();
    collect_table(bindings, table, out);
    sort_key_bindings(bindings, crit, out);
}

#[cfg(test)]
mod tests;
