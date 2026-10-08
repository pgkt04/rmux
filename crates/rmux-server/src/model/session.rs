// Ported from tmux session.c @ 8f25579c
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

use super::state::{ModelEffect, ModelError, Server, Session, SessionGroup, TimerRequest};
use super::{PaneFlags, SessionFlags, WinlinkFlags, window};
use crate::ids::{OptionsId, SessionGroupId, SessionId, WindowId, WinlinkId};
use crate::options::environment::Environment;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectOutcome {
    Missing,
    Unchanged,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetachOutcome {
    Missing,
    Detached,
    DestroySession,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionTimerRequest {
    Free(SessionId),
    Lock { session: SessionId, seconds: i64 },
    CancelLock(SessionId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionEffect {
    StatusCache(SessionId),
    Status(SessionId),
    LockSession(SessionId),
    RecalculateSizes,
    SessionClosed(SessionId),
    WindowLinked {
        session: SessionId,
        window: WindowId,
        winlink: WinlinkId,
        index: i32,
    },
    WindowUnlinked {
        session: SessionId,
        window: WindowId,
        winlink: WinlinkId,
        index: i32,
    },
    WindowChanged {
        session: SessionId,
        new_window: WindowId,
        new_index: i32,
        old: Option<(WindowId, i32)>,
    },
    GroupChanged {
        event: &'static str,
        session: SessionId,
        group: Vec<u8>,
        size: u32,
        target: bool,
    },
    WindowOffset(WindowId),
}

pub struct SessionCreate {
    pub prefix: Option<Vec<u8>>,
    pub name: Option<Vec<u8>>,
    pub cwd: Vec<u8>,
    pub environment: Environment,
    pub options: OptionsId,
    pub termios: Option<rmux_sys::TermiosState>,
}

fn effect(server: &mut Server, effect: SessionEffect) {
    let event = match &effect {
        SessionEffect::SessionClosed(session) => Some(("session-closed", *session, None)),
        SessionEffect::WindowLinked {
            session, window, ..
        } => Some(("window-linked", *session, Some(*window))),
        SessionEffect::WindowUnlinked {
            session, window, ..
        } => Some(("window-unlinked", *session, Some(*window))),
        SessionEffect::WindowChanged {
            session,
            new_window,
            ..
        } => Some(("session-window-changed", *session, Some(*new_window))),
        SessionEffect::GroupChanged { event, session, .. } => Some((*event, *session, None)),
        _ => None,
    };
    server.effects.push_back(ModelEffect::Session(effect));
    if let (Some(callback), Some((event, session, window))) = (server.model_event, event) {
        callback(server, event.as_bytes(), Some(session), window, None);
    }
}

fn timer(server: &mut Server, request: SessionTimerRequest) {
    server
        .effects
        .push_back(ModelEffect::Timer(TimerRequest::Session(request)));
}

pub fn session_cmp(left: &Session, right: &Session) -> std::cmp::Ordering {
    rmux_util::bytes::cstr(&left.name).cmp(rmux_util::bytes::cstr(&right.name))
}

pub fn session_group_cmp(left: &SessionGroup, right: &SessionGroup) -> std::cmp::Ordering {
    rmux_util::bytes::cstr(&left.name).cmp(rmux_util::bytes::cstr(&right.name))
}

pub fn session_alive(server: &Server, session: SessionId) -> bool {
    server
        .sessions
        .get(session)
        .is_some_and(|s| server.session_names.get(&s.name) == Some(&session))
}

pub fn session_find(server: &Server, name: &[u8]) -> Option<SessionId> {
    server
        .session_names
        .get(rmux_util::bytes::cstr(name))
        .copied()
}

pub fn session_find_by_id_str(server: &Server, value: &[u8]) -> Option<SessionId> {
    let value = rmux_util::bytes::cstr(value);
    if value.first() != Some(&b'$') {
        return None;
    }
    let id = rmux_util::strtonum::strtonum(&value[1..], 0, i64::from(u32::MAX)).ok()?;
    session_find_by_id(server, id as u32)
}

pub fn session_find_by_id(server: &Server, id: u32) -> Option<SessionId> {
    server
        .session_names
        .values()
        .copied()
        .find(|sid| server.sessions.get(*sid).is_some_and(|s| s.public_id == id))
}

pub fn session_create(server: &mut Server, create: SessionCreate) -> SessionId {
    let (public_id, name) = if let Some(name) = create.name {
        let id = server.next_session_id;
        server.next_session_id = id.wrapping_add(1);
        (id, rmux_util::bytes::cstr(&name).to_vec())
    } else {
        loop {
            let id = server.next_session_id;
            server.next_session_id = id.wrapping_add(1);
            let mut name = Vec::new();
            if let Some(prefix) = &create.prefix {
                name.extend_from_slice(rmux_util::bytes::cstr(prefix));
                name.push(b'-');
            }
            name.extend_from_slice(id.to_string().as_bytes());
            if !server.session_names.contains_key(&name) {
                break (id, name);
            }
        }
    };
    let id = server
        .sessions
        .insert(Session {
            public_id,
            name: name.clone(),
            cwd: create.cwd,
            created: server.current_time,
            activity: server.current_time,
            last_attached: (0, 0),
            options: create.options,
            environment: create.environment,
            termios: create.termios,
            windows: BTreeMap::new(),
            ordered_winlinks: Vec::new(),
            current: None,
            last: Vec::new(),
            group: None,
            attached: 0,
            flags: SessionFlags::default(),
            references: 1,
            lock_timer_initialized: false,
            lock_timer_pending: false,
            statusat: -1,
            statuslines: 0,
        })
        .expect("session arena");
    server.sessions.retain(id).expect("session root reference");
    crate::ui::status::status_update_cache(server, id);
    // Explicit duplicate names are rejected by commands, not session_create.
    server.session_names.entry(name).or_insert(id);
    effect(server, SessionEffect::StatusCache(id));
    let created = server.current_time;
    session_update_activity(server, id, Some(created));
    id
}

pub fn session_retain(server: &mut Server, session: SessionId) -> bool {
    let Some(s) = server.sessions.get_mut(session) else {
        return false;
    };
    s.references = s
        .references
        .checked_add(1)
        .expect("session reference overflow");
    server.sessions.retain(session).expect("session reference");
    true
}

pub fn session_release(server: &mut Server, session: SessionId) -> bool {
    let Some(s) = server.sessions.get_mut(session) else {
        return false;
    };
    s.references = s
        .references
        .checked_sub(1)
        .expect("session reference underflow");
    let free = s.references == 0;
    assert!(
        server
            .sessions
            .release(session)
            .expect("session reference")
            .is_none()
    );
    if free {
        timer(server, SessionTimerRequest::Free(session));
    }
    true
}

pub fn session_free(server: &mut Server, session: SessionId) -> bool {
    if server
        .sessions
        .get(session)
        .is_none_or(|s| s.references != 0)
    {
        return false;
    }
    let s = server
        .sessions
        .request_remove(session)
        .expect("session free")
        .expect("unleased session");
    server.free_options(s.options);
    true
}

pub fn session_destroy(server: &mut Server, session: SessionId, notify: bool) {
    let Some(s) = server.sessions.get_mut(session) else {
        return;
    };
    if s.current.take().is_none() {
        return;
    }
    let name = s.name.clone();
    if server.session_names.get(&name) == Some(&session) {
        server.session_names.remove(&name);
    }
    if notify {
        effect(server, SessionEffect::SessionClosed(session));
    }
    let Some(s) = server.sessions.get_mut(session) else {
        return;
    };
    s.termios = None;
    let initialized = s.lock_timer_initialized;
    s.lock_timer_pending = false;
    if initialized {
        timer(server, SessionTimerRequest::CancelLock(session));
    }
    session_group_remove(server, session);
    while let Some(link) = server
        .sessions
        .get(session)
        .and_then(|s| s.last.first().copied())
    {
        window::winlink_stack_remove(server, session, link);
    }
    while let Some(link) = server
        .sessions
        .get(session)
        .and_then(|s| s.windows.first_key_value().map(|(_, id)| *id))
    {
        fire_link(server, link, false);
        window::winlink_remove(server, link);
    }
    if let Some(s) = server.sessions.get_mut(session) {
        s.cwd = Vec::new();
    }
    session_release(server, session);
}

pub fn session_update_activity(server: &mut Server, session: SessionId, from: Option<(i64, i64)>) {
    let Some(s) = server.sessions.get_mut(session) else {
        return;
    };
    s.activity = from.unwrap_or(server.current_time);
    let cancel = s.lock_timer_initialized;
    s.lock_timer_initialized = true;
    s.lock_timer_pending = false;
    let attached = s.attached != 0;
    let options = s.options;
    if cancel {
        timer(server, SessionTimerRequest::CancelLock(session));
    }
    if attached {
        let seconds = server.options.get_number(options, b"lock-after-time");
        if seconds != 0 {
            server
                .sessions
                .get_mut(session)
                .expect("lock owner")
                .lock_timer_pending = true;
            timer(server, SessionTimerRequest::Lock { session, seconds });
        }
    }
}

pub fn session_lock_timer(server: &mut Server, session: SessionId) {
    let Some(s) = server.sessions.get_mut(session) else {
        return;
    };
    s.lock_timer_pending = false;
    if s.attached == 0 {
        return;
    }
    effect(server, SessionEffect::LockSession(session));
    effect(server, SessionEffect::RecalculateSizes);
}

// G10 supplies its already sorted session list; selection does not change it.
pub fn session_next_session(
    server: &Server,
    session: SessionId,
    sorted: &[SessionId],
) -> Option<SessionId> {
    if !session_alive(server, session) || sorted.is_empty() {
        return None;
    }
    let at = sorted
        .iter()
        .position(|id| *id == session)
        .expect("active session absent from sorted list");
    Some(sorted[(at + 1) % sorted.len()])
}

pub fn session_previous_session(
    server: &Server,
    session: SessionId,
    sorted: &[SessionId],
) -> Option<SessionId> {
    if !session_alive(server, session) || sorted.is_empty() {
        return None;
    }
    let at = sorted
        .iter()
        .position(|id| *id == session)
        .expect("active session absent from sorted list");
    Some(sorted[if at == 0 { sorted.len() - 1 } else { at - 1 }])
}

fn fire_link(server: &mut Server, link: WinlinkId, linked: bool) {
    let wl = server.winlinks.get(link).expect("session winlink");
    let (session, window, index) = (wl.session, wl.window, wl.index);
    let event = if linked {
        SessionEffect::WindowLinked {
            session,
            window,
            winlink: link,
            index,
        }
    } else {
        SessionEffect::WindowUnlinked {
            session,
            window,
            winlink: link,
            index,
        }
    };
    effect(server, event);
}

pub fn session_attach(
    server: &mut Server,
    session: SessionId,
    window: WindowId,
    index: i32,
) -> Result<WinlinkId, ModelError> {
    if server.sessions.get(session).is_none() || server.windows.get(window).is_none() {
        return Err(ModelError::StaleId);
    }
    let link = window::winlink_add(server, session, window, index)?;
    fire_link(server, link, true);
    session_group_synchronize_from(server, session);
    if server.winlinks.get(link).is_none() {
        return Err(ModelError::StaleId);
    }
    Ok(link)
}

pub fn session_detach(server: &mut Server, session: SessionId, link: WinlinkId) -> DetachOutcome {
    let Some(s) = server.sessions.get(session) else {
        return DetachOutcome::Missing;
    };
    if !s.windows.values().any(|id| *id == link) {
        return DetachOutcome::Missing;
    }
    if s.windows.len() == 1 {
        return DetachOutcome::DestroySession;
    }
    if s.current == Some(link)
        && session_last(server, session) != SelectOutcome::Changed
        && session_previous(server, session, false) != SelectOutcome::Changed
    {
        session_next(server, session, false);
    }
    server
        .winlinks
        .get_mut(link)
        .expect("detached link")
        .flags
        .remove(WinlinkFlags::ALERTFLAGS);
    fire_link(server, link, false);
    window::winlink_stack_remove(server, session, link);
    window::winlink_remove(server, link);
    session_group_synchronize_from(server, session);
    DetachOutcome::Detached
}

pub fn session_has(server: &Server, session: SessionId, window: WindowId) -> bool {
    server.windows.get(window).is_some_and(|w| {
        w.links.iter().any(|id| {
            server
                .winlinks
                .get(*id)
                .is_some_and(|wl| wl.session == session)
        })
    })
}

pub fn session_is_linked(server: &Server, session: SessionId, window: WindowId) -> bool {
    let Some(w) = server.windows.get(window) else {
        return false;
    };
    let count = session_group_contains(server, session)
        .map_or(1, |group| session_group_count(server, group));
    w.references != count
}

pub fn session_next(server: &mut Server, session: SessionId, alert: bool) -> SelectOutcome {
    let Some(s) = server.sessions.get(session) else {
        return SelectOutcome::Missing;
    };
    let Some(current) = s.current.and_then(|id| server.winlinks.get(id)) else {
        return SelectOutcome::Missing;
    };
    let accepted = |id: &&WinlinkId| {
        !alert
            || server
                .winlinks
                .get(**id)
                .is_some_and(|wl| wl.flags.intersects(WinlinkFlags::ALERTFLAGS))
    };
    let destination = s
        .windows
        .range((Excluded(current.index), Unbounded))
        .map(|(_, id)| id)
        .chain(s.windows.values())
        .find(accepted)
        .copied();
    session_set_current(server, session, destination)
}

pub fn session_previous(server: &mut Server, session: SessionId, alert: bool) -> SelectOutcome {
    let Some(s) = server.sessions.get(session) else {
        return SelectOutcome::Missing;
    };
    let Some(current) = s.current.and_then(|id| server.winlinks.get(id)) else {
        return SelectOutcome::Missing;
    };
    let accepted = |id: &&WinlinkId| {
        !alert
            || server
                .winlinks
                .get(**id)
                .is_some_and(|wl| wl.flags.intersects(WinlinkFlags::ALERTFLAGS))
    };
    let destination = s
        .windows
        .range((Unbounded, Excluded(current.index)))
        .rev()
        .map(|(_, id)| id)
        .chain(s.windows.values().rev())
        .find(accepted)
        .copied();
    session_set_current(server, session, destination)
}

pub fn session_select(server: &mut Server, session: SessionId, index: i32) -> SelectOutcome {
    let destination = window::winlink_find_by_index(server, session, index);
    session_set_current(server, session, destination)
}

pub fn session_last(server: &mut Server, session: SessionId) -> SelectOutcome {
    let destination = server
        .sessions
        .get(session)
        .and_then(|s| s.last.first().copied());
    session_set_current(server, session, destination)
}

pub fn session_set_current(
    server: &mut Server,
    session: SessionId,
    destination: Option<WinlinkId>,
) -> SelectOutcome {
    let Some(destination) = destination else {
        return SelectOutcome::Missing;
    };
    let Some(s) = server.sessions.get(session) else {
        return SelectOutcome::Missing;
    };
    let Some(wl) = server.winlinks.get(destination) else {
        return SelectOutcome::Missing;
    };
    if wl.session != session || s.windows.get(&wl.index) != Some(&destination) {
        return SelectOutcome::Missing;
    }
    if s.current == Some(destination) {
        return SelectOutcome::Unchanged;
    }
    let old = s.current;
    let (new_window, new_index) = (wl.window, wl.index);
    let old_snapshot = old
        .and_then(|id| server.winlinks.get(id))
        .map(|wl| (wl.window, wl.index));
    window::winlink_stack_remove(server, session, destination);
    window::winlink_stack_push(server, session, old);
    server
        .sessions
        .get_mut(session)
        .expect("selection session")
        .current = Some(destination);
    if server
        .options
        .get_number(server.options.global, b"focus-events")
        != 0
    {
        if let Some((old_window, _)) = old_snapshot {
            window::window_update_focus(server, old_window);
        }
        window::window_update_focus(server, new_window);
        if server
            .sessions
            .get(session)
            .is_none_or(|s| s.current != Some(destination))
            || server.winlinks.get(destination).is_none()
        {
            return SelectOutcome::Changed;
        }
    }
    window::winlink_clear_flags(server, destination);
    window::window_update_activity(server, new_window);
    effect(server, SessionEffect::WindowOffset(new_window));
    effect(
        server,
        SessionEffect::WindowChanged {
            session,
            new_window,
            new_index,
            old: old_snapshot,
        },
    );
    SelectOutcome::Changed
}

pub fn session_group_contains(server: &Server, session: SessionId) -> Option<SessionGroupId> {
    server.sessions.get(session).and_then(|s| s.group)
}

pub fn session_group_find(server: &Server, name: &[u8]) -> Option<SessionGroupId> {
    server
        .group_names
        .get(rmux_util::bytes::cstr(name))
        .copied()
}

pub fn session_group_new(server: &mut Server, name: &[u8]) -> SessionGroupId {
    if let Some(group) = session_group_find(server, name) {
        return group;
    }
    let name = rmux_util::bytes::cstr(name).to_vec();
    let group = server
        .groups
        .insert(SessionGroup {
            name: name.clone(),
            sessions: Vec::new(),
        })
        .expect("session group arena");
    server.group_names.insert(name, group);
    group
}

fn session_group_fire(
    server: &mut Server,
    event: &'static str,
    group: SessionGroupId,
    session: SessionId,
) {
    let sg = server.groups.get(group).expect("session group");
    let group = sg.name.clone();
    let size = sg.sessions.len() as u32;
    let target = session_alive(server, session);
    effect(
        server,
        SessionEffect::GroupChanged {
            event,
            session,
            group,
            size,
            target,
        },
    );
}

pub fn session_group_add(server: &mut Server, group: SessionGroupId, session: SessionId) {
    let Some(s) = server.sessions.get(session) else {
        return;
    };
    if s.group.is_some() || server.groups.get(group).is_none() {
        return;
    }
    server
        .groups
        .get_mut(group)
        .expect("session group")
        .sessions
        .push(session);
    server
        .sessions
        .get_mut(session)
        .expect("group member")
        .group = Some(group);
    session_group_fire(server, "session-added-to-group", group, session);
}

pub fn session_group_remove(server: &mut Server, session: SessionId) {
    let Some(group) = session_group_contains(server, session) else {
        return;
    };
    session_group_fire(server, "session-removed-from-group", group, session);
    let Some(s) = server.sessions.get_mut(session) else {
        return;
    };
    if s.group != Some(group) {
        return;
    }
    s.group = None;
    let Some(sg) = server.groups.get_mut(group) else {
        return;
    };
    sg.sessions.retain(|id| *id != session);
    if sg.sessions.is_empty() {
        let sg = server
            .groups
            .request_remove(group)
            .expect("session group removal")
            .expect("unleased group");
        server.group_names.remove(&sg.name);
    }
}

pub fn session_group_count(server: &Server, group: SessionGroupId) -> u32 {
    server
        .groups
        .get(group)
        .map_or(0, |sg| sg.sessions.len() as u32)
}

pub fn session_group_attached_count(server: &Server, group: SessionGroupId) -> u32 {
    server.groups.get(group).map_or(0, |sg| {
        sg.sessions
            .iter()
            .filter_map(|id| server.sessions.get(*id))
            .fold(0u32, |count, s| count.wrapping_add(s.attached))
    })
}

pub fn session_group_synchronize_to(server: &mut Server, session: SessionId) {
    let Some(group) = session_group_contains(server, session) else {
        return;
    };
    let source = server
        .groups
        .get(group)
        .and_then(|sg| sg.sessions.iter().find(|id| **id != session).copied());
    if let Some(source) = source {
        session_group_synchronize1(server, source, session);
    }
}

pub fn session_group_synchronize_from(server: &mut Server, source: SessionId) {
    let Some(group) = session_group_contains(server, source) else {
        return;
    };
    let mut position = 0;
    while let Some(session) = server
        .groups
        .get(group)
        .and_then(|sg| sg.sessions.get(position).copied())
    {
        position += 1;
        if session != source {
            session_group_synchronize1(server, source, session);
        }
    }
}

fn session_group_synchronize1(server: &mut Server, source: SessionId, session: SessionId) {
    let Some(source_session) = server.sessions.get(source) else {
        return;
    };
    if source_session.windows.is_empty() || server.sessions.get(session).is_none() {
        return;
    }
    let source_current_index = source_session
        .current
        .and_then(|id| server.winlinks.get(id))
        .map(|wl| wl.index);
    let current = server
        .sessions
        .get(session)
        .and_then(|s| s.current)
        .and_then(|id| server.winlinks.get(id));
    if current.is_some_and(|wl| !source_session.windows.contains_key(&wl.index))
        && session_last(server, session) != SelectOutcome::Changed
        && session_previous(server, session, false) != SelectOutcome::Changed
    {
        session_next(server, session, false);
    }
    let Some(s) = server.sessions.get_mut(session) else {
        return;
    };
    let old_windows = std::mem::take(&mut s.windows);
    s.ordered_winlinks.clear();
    let current_index = s
        .current
        .and_then(|id| server.winlinks.get(id))
        .map(|wl| wl.index);
    let mut previous_index = None;
    loop {
        let source_link = {
            let Some(s) = server.sessions.get(source) else {
                break;
            };
            match previous_index {
                None => s.windows.first_key_value(),
                Some(index) => s.windows.range((Excluded(index), Unbounded)).next(),
            }
            .map(|(index, link)| (*index, *link))
        };
        let Some((index, source_link)) = source_link else {
            break;
        };
        previous_index = Some(index);
        let wl = server.winlinks.get(source_link).expect("group source link");
        let (window, alerts) = (wl.window, wl.flags & WinlinkFlags::ALERTFLAGS);
        let link =
            window::winlink_add(server, session, window, index).expect("group replacement index");
        fire_link(server, link, true);
        if !session_alive(server, session) {
            for (_, old) in old_windows {
                window::winlink_remove(server, old);
            }
            return;
        }
        if let Some(wl) = server.winlinks.get_mut(link) {
            wl.flags.insert(alerts);
        }
    }
    let Some(s) = server.sessions.get_mut(session) else {
        for (_, old) in old_windows {
            window::winlink_remove(server, old);
        }
        return;
    };
    s.current = current_index
        .or(source_current_index)
        .and_then(|index| s.windows.get(&index).copied())
        .or_else(|| s.windows.first_key_value().map(|(_, id)| *id));
    let old_last = std::mem::take(&mut s.last);
    for old in old_last {
        let Some(index) = server.winlinks.get(old).map(|wl| wl.index) else {
            continue;
        };
        if let Some(link) = server
            .sessions
            .get(session)
            .and_then(|s| s.windows.get(&index).copied())
        {
            server
                .sessions
                .get_mut(session)
                .expect("group history")
                .last
                .push(link);
            server
                .winlinks
                .get_mut(link)
                .expect("group history link")
                .flags
                .insert(WinlinkFlags::VISITED);
        }
    }
    // Sorted teardown is the accepted initial deviation from RB_ROOT order.
    for (_, old) in old_windows {
        let Some(old_public_id) = server
            .winlinks
            .get(old)
            .and_then(|wl| server.windows.get(wl.window))
            .map(|w| w.public_id)
        else {
            continue;
        };
        let remains = server.sessions.get(session).is_some_and(|s| {
            s.windows.values().any(|link| {
                server
                    .winlinks
                    .get(*link)
                    .and_then(|wl| server.windows.get(wl.window))
                    .is_some_and(|w| w.public_id == old_public_id)
            })
        });
        if !remains {
            fire_link(server, old, false);
        }
        window::winlink_remove(server, old);
    }
}

pub fn session_renumber_windows(server: &mut Server, session: SessionId) {
    let Some(s) = server.sessions.get(session) else {
        return;
    };
    let base = server.options.get_number(s.options, b"base-index") as i32;
    let available = (i32::MAX as u32).wrapping_sub(base as u32).wrapping_add(1);
    if s.windows.len() > available as usize {
        return;
    }
    let s = server.sessions.get_mut(session).expect("renumber session");
    let old_windows = std::mem::take(&mut s.windows);
    s.ordered_winlinks.clear();
    let old_current = s.current;
    let mut current_index = 0;
    let mut marked_index = None;
    let mut next_index = base;
    for (position, old) in old_windows.values().copied().enumerate() {
        let wl = server.winlinks.get(old).expect("renumber old link");
        let (window, alerts) = (wl.window, wl.flags & WinlinkFlags::ALERTFLAGS);
        let link =
            window::winlink_add(server, session, window, next_index).expect("renumber index");
        server
            .winlinks
            .get_mut(link)
            .expect("renumber link")
            .flags
            .insert(alerts);
        if Some(old) == server.marked_winlink {
            marked_index = Some(next_index);
        }
        if Some(old) == old_current {
            current_index = next_index;
        }
        if position + 1 < old_windows.len() {
            next_index += 1;
        }
    }
    let old_last = std::mem::take(
        &mut server
            .sessions
            .get_mut(session)
            .expect("renumber history")
            .last,
    );
    for old in old_last {
        let wl = server.winlinks.get_mut(old).expect("renumber history link");
        wl.flags.remove(WinlinkFlags::VISITED);
        let window = wl.window;
        let link = server
            .sessions
            .get(session)
            .expect("renumber session")
            .windows
            .values()
            .copied()
            .find(|id| {
                server
                    .winlinks
                    .get(*id)
                    .is_some_and(|wl| wl.window == window)
            });
        if let Some(link) = link {
            server
                .sessions
                .get_mut(session)
                .expect("renumber history")
                .last
                .push(link);
            server
                .winlinks
                .get_mut(link)
                .expect("renumber history link")
                .flags
                .insert(WinlinkFlags::VISITED);
        }
    }
    let s = server.sessions.get_mut(session).expect("renumber session");
    if let Some(index) = marked_index {
        server.marked_winlink = s.windows.get(&index).copied();
        if server.marked_winlink.is_none() {
            server.marked_pane = None;
        }
    }
    s.current = s.windows.get(&current_index).copied();
    for (_, old) in old_windows {
        window::winlink_remove(server, old);
    }
}

pub fn session_theme_changed(server: &mut Server, session: Option<SessionId>) {
    let Some(s) = session.and_then(|id| server.sessions.get(id)) else {
        return;
    };
    for link in s.windows.values() {
        let Some(w) = server
            .winlinks
            .get(*link)
            .and_then(|wl| server.windows.get(wl.window))
        else {
            continue;
        };
        for pane in &w.panes {
            if let Some(pane) = server.panes.get_mut(*pane) {
                pane.flags.insert(PaneFlags::THEMECHANGED);
            }
        }
    }
}

pub fn session_update_history(server: &mut Server, session: SessionId) {
    let Some(s) = server.sessions.get(session) else {
        return;
    };
    let limit = server.options.get_number(s.options, b"history-limit") as u32;
    for link in s.windows.values() {
        let Some(w) = server
            .winlinks
            .get(*link)
            .and_then(|wl| server.windows.get(wl.window))
        else {
            continue;
        };
        for pane in &w.panes {
            if let Some(pane) = server.panes.get_mut(*pane) {
                pane.base.grid.set_hlimit(limit);
                pane.base.grid.collect_history(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::{CommandList, parse::CmdParseResult};
    use crate::options::CommandParser;
    use std::rc::Rc;

    struct Parser;
    impl CommandParser for Parser {
        fn parse_from_string(&mut self, _: &[u8]) -> CmdParseResult {
            Ok(Rc::new(CommandList::default()))
        }
    }

    fn server() -> Server {
        let mut server = Server::new();
        server.options.load_defaults(&mut Parser);
        server.current_time = (123, 456);
        server
    }

    fn create(server: &mut Server, name: Option<&[u8]>) -> SessionId {
        let options = server.options.create(Some(server.options.global_s));
        session_create(
            server,
            SessionCreate {
                prefix: None,
                name: name.map(<[u8]>::to_vec),
                cwd: b"/tmp".to_vec(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        )
    }

    fn attach(server: &mut Server, session: SessionId, index: i32) -> WinlinkId {
        let window = window::window_create(server, 80, 24, 0, 0).unwrap();
        session_attach(server, session, window, index).unwrap()
    }

    fn session_effects(server: &Server) -> Vec<&SessionEffect> {
        server
            .effects
            .iter()
            .filter_map(|effect| match effect {
                ModelEffect::Session(effect) => Some(effect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn prefixed_automatic_name_consumes_collisions_and_public_counter_wraps() {
        let mut server = server();
        create(&mut server, Some(b"prefix-1"));
        let options = server.options.create(Some(server.options.global_s));
        let session = session_create(
            &mut server,
            SessionCreate {
                prefix: Some(b"prefix".to_vec()),
                name: None,
                cwd: Vec::new(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        );
        let s = server.sessions.get(session).unwrap();
        assert_eq!(
            (s.public_id, s.name.as_slice()),
            (2, b"prefix-2".as_slice())
        );
        server.next_session_id = u32::MAX;
        let maximum = create(&mut server, Some(b"maximum"));
        assert_eq!(
            session_find_by_id_str(&server, b"$4294967295"),
            Some(maximum)
        );
        assert_eq!(server.next_session_id, 0);
    }

    #[test]
    fn creation_names_public_ids_and_byte_order() {
        let mut server = server();
        let zero = create(&mut server, Some(b"1"));
        let auto = create(&mut server, None);
        assert_eq!(server.sessions.get(zero).unwrap().public_id, 0);
        let s = server.sessions.get(auto).unwrap();
        assert_eq!((s.public_id, s.name.as_slice()), (2, b"2".as_slice()));
        assert_eq!(s.created, (123, 456));
        assert_eq!(s.activity, s.created);
        assert_eq!(s.references, 1);
        assert!(s.current.is_none() && s.windows.is_empty() && s.last.is_empty());
        assert_eq!(s.cwd, b"/tmp");
        assert!(s.lock_timer_initialized);
        assert_eq!(session_find_by_id_str(&server, b"$2"), Some(auto));
        assert_eq!(session_find_by_id_str(&server, b"$ +2"), Some(auto));
        for invalid in [b"2".as_slice(), b"$", b"$-1", b"$4294967296", b"$2 "] {
            assert_eq!(session_find_by_id_str(&server, invalid), None);
        }
        create(&mut server, Some(b"z"));
        create(&mut server, Some(b"Z"));
        assert_eq!(
            server
                .session_names
                .keys()
                .map(Vec::as_slice)
                .collect::<Vec<_>>(),
            [b"1".as_slice(), b"2", b"Z", b"z"]
        );
        assert!(
            matches!(session_effects(&server)[0], SessionEffect::StatusCache(id) if *id == zero)
        );
    }

    #[test]
    fn creation_empty_destroy_is_not_active_removal() {
        let mut server = server();
        let session = create(&mut server, Some(b"empty"));
        server.effects.clear();
        session_destroy(&mut server, session, true);
        assert!(session_alive(&server, session));
        assert_eq!(server.sessions.get(session).unwrap().references, 1);
        assert!(server.effects.is_empty());
    }

    #[test]
    fn retain_before_deferred_free_survives_and_stale_callback_is_inert() {
        let mut server = server();
        let session = create(&mut server, Some(b"leased"));
        let link = attach(&mut server, session, 0);
        session_set_current(&mut server, session, Some(link));
        session_destroy(&mut server, session, true);
        assert!(!session_alive(&server, session));
        assert_eq!(session_find(&server, b"leased"), None);
        assert!(server.sessions.get(session).is_some());
        assert!(server.effects.iter().any(|effect| matches!(effect, ModelEffect::Timer(TimerRequest::Session(SessionTimerRequest::Free(id))) if *id == session)));
        assert!(session_retain(&mut server, session));
        assert!(!session_free(&mut server, session));
        assert_eq!(server.sessions.get(session).unwrap().references, 1);
        session_release(&mut server, session);
        assert!(session_free(&mut server, session));
        let replacement = create(&mut server, Some(b"leased"));
        assert_ne!(replacement, session);
        assert!(!session_free(&mut server, session));
        session_lock_timer(&mut server, session);
        assert!(session_alive(&server, replacement));
    }

    #[test]
    fn lock_activity_cancels_then_rearms_and_detached_callback_does_nothing() {
        let mut server = server();
        let session = create(&mut server, Some(b"lock"));
        let options = server.sessions.get(session).unwrap().options;
        server
            .options
            .set_number(options, b"lock-after-time", 12, &mut Parser);
        server.sessions.get_mut(session).unwrap().attached = 1;
        server.effects.clear();
        session_update_activity(&mut server, session, Some((77, 9)));
        assert_eq!(server.sessions.get(session).unwrap().activity, (77, 9));
        assert!(
            matches!(server.effects[0], ModelEffect::Timer(TimerRequest::Session(SessionTimerRequest::CancelLock(id))) if id == session)
        );
        assert!(
            matches!(server.effects[1], ModelEffect::Timer(TimerRequest::Session(SessionTimerRequest::Lock { session: id, seconds: 12 })) if id == session)
        );
        server.sessions.get_mut(session).unwrap().attached = 0;
        server.effects.clear();
        session_lock_timer(&mut server, session);
        assert!(server.effects.is_empty());
        server.sessions.get_mut(session).unwrap().attached = 1;
        session_lock_timer(&mut server, session);
        assert_eq!(
            session_effects(&server),
            [
                &SessionEffect::LockSession(session),
                &SessionEffect::RecalculateSizes
            ]
        );
        server
            .options
            .set_number(options, b"lock-after-time", 0, &mut Parser);
        server.effects.clear();
        session_update_activity(&mut server, session, None);
        assert_eq!(server.effects.len(), 1);
        assert!(!server.sessions.get(session).unwrap().lock_timer_pending);
    }

    #[test]
    fn sorted_session_selection_wraps_and_inactive_fails() {
        let mut server = server();
        let a = create(&mut server, Some(b"a"));
        let b = create(&mut server, Some(b"b"));
        assert_eq!(session_next_session(&server, a, &[b, a]), Some(b));
        assert_eq!(session_previous_session(&server, b, &[b, a]), Some(a));
        assert_eq!(session_next_session(&server, a, &[a]), Some(a));
        let link = attach(&mut server, a, 0);
        session_set_current(&mut server, a, Some(link));
        session_destroy(&mut server, a, false);
        assert_eq!(session_previous_session(&server, a, &[b, a]), None);
    }

    #[test]
    fn selection_wraps_alerts_and_records_old_new_payload() {
        let mut server = server();
        let session = create(&mut server, Some(b"select"));
        let a = attach(&mut server, session, 1);
        let b = attach(&mut server, session, 4);
        let c = attach(&mut server, session, 9);
        assert_eq!(
            session_next(&mut server, session, false),
            SelectOutcome::Missing
        );
        assert_eq!(
            session_set_current(&mut server, session, Some(a)),
            SelectOutcome::Changed
        );
        assert_eq!(
            session_select(&mut server, session, 1),
            SelectOutcome::Unchanged
        );
        assert_eq!(
            session_select(&mut server, session, 2),
            SelectOutcome::Missing
        );
        assert_eq!(
            session_previous(&mut server, session, false),
            SelectOutcome::Changed
        );
        assert_eq!(server.sessions.get(session).unwrap().current, Some(c));
        assert_eq!(
            session_next(&mut server, session, false),
            SelectOutcome::Changed
        );
        assert_eq!(server.sessions.get(session).unwrap().current, Some(a));
        assert_eq!(
            session_next(&mut server, session, true),
            SelectOutcome::Missing
        );
        server
            .winlinks
            .get_mut(c)
            .unwrap()
            .flags
            .insert(WinlinkFlags::BELL);
        server.effects.clear();
        assert_eq!(
            session_next(&mut server, session, true),
            SelectOutcome::Changed
        );
        assert_eq!(server.sessions.get(session).unwrap().current, Some(c));
        let old_window = server.winlinks.get(a).unwrap().window;
        let new_window = server.winlinks.get(c).unwrap().window;
        assert_eq!(
            session_effects(&server).last().copied(),
            Some(&SessionEffect::WindowChanged {
                session,
                new_window,
                new_index: 9,
                old: Some((old_window, 1)),
            })
        );
        assert!(
            !server
                .winlinks
                .get(c)
                .unwrap()
                .flags
                .intersects(WinlinkFlags::ALERTFLAGS)
        );
        assert_eq!(session_last(&mut server, session), SelectOutcome::Changed);
        assert_eq!(server.sessions.get(session).unwrap().last, [c]);
        assert_eq!(
            session_set_current(&mut server, session, Some(b)),
            SelectOutcome::Changed
        );
        assert_eq!(server.sessions.get(session).unwrap().last, [a, c]);
    }

    #[test]
    fn detach_last_requires_caller_and_current_uses_history() {
        let mut server = server();
        let session = create(&mut server, Some(b"detach"));
        let a = attach(&mut server, session, 0);
        session_set_current(&mut server, session, Some(a));
        assert_eq!(
            session_detach(&mut server, session, a),
            DetachOutcome::DestroySession
        );
        assert!(server.winlinks.get(a).is_some());
        let b = attach(&mut server, session, 2);
        session_set_current(&mut server, session, Some(b));
        assert_eq!(
            session_detach(&mut server, session, b),
            DetachOutcome::Detached
        );
        assert_eq!(server.sessions.get(session).unwrap().current, Some(a));
        assert!(server.sessions.get(session).unwrap().last.is_empty());
        assert!(server.winlinks.get(b).is_none());
        let window = server.winlinks.get(a).unwrap().window;
        let error = session_attach(&mut server, session, window, 0).unwrap_err();
        assert_eq!(error.to_string(), "index in use: 0");
    }

    #[test]
    fn group_events_snapshot_size_and_destroyed_member_has_no_target() {
        let mut server = server();
        let a = create(&mut server, Some(b"a"));
        let b = create(&mut server, Some(b"b"));
        let group = session_group_new(&mut server, b"g");
        assert_eq!(session_group_new(&mut server, b"g"), group);
        session_group_add(&mut server, group, a);
        session_group_add(&mut server, group, a);
        session_group_add(&mut server, group, b);
        assert_eq!(session_group_count(&server, group), 2);
        server.sessions.get_mut(a).unwrap().attached = 2;
        server.sessions.get_mut(b).unwrap().attached = 3;
        assert_eq!(session_group_attached_count(&server, group), 5);
        let link = attach(&mut server, a, 0);
        session_set_current(&mut server, a, Some(link));
        server.effects.clear();
        session_destroy(&mut server, a, true);
        let effects = session_effects(&server);
        assert!(matches!(effects[0], SessionEffect::SessionClosed(id) if *id == a));
        assert!(matches!(effects[1], SessionEffect::GroupChanged {
            event: "session-removed-from-group", session, group, size: 2, target: false,
        } if *session == a && group == b"g"));
        assert_eq!(session_group_count(&server, group), 1);
        assert!(session_group_find(&server, b"g").is_some());
        session_group_remove(&mut server, b);
        assert!(session_group_find(&server, b"g").is_none());
        assert!(server.groups.get(group).is_none());
    }

    #[test]
    fn three_member_rebuild_preserves_choices_and_replacement_window_lifetimes() {
        let mut server = server();
        let a = create(&mut server, Some(b"a"));
        let b = create(&mut server, Some(b"b"));
        let c = create(&mut server, Some(b"c"));
        let first = attach(&mut server, a, 0);
        let second = attach(&mut server, a, 4);
        let first_window = server.winlinks.get(first).unwrap().window;
        let second_window = server.winlinks.get(second).unwrap().window;
        session_attach(&mut server, a, first_window, 9).unwrap();
        session_set_current(&mut server, a, Some(second));
        let group = session_group_new(&mut server, b"g");
        for session in [a, b, c] {
            session_group_add(&mut server, group, session);
        }
        session_group_synchronize_to(&mut server, b);
        session_group_synchronize_to(&mut server, c);
        session_select(&mut server, b, 0);
        session_select(&mut server, b, 9);
        session_select(&mut server, c, 9);
        session_select(&mut server, c, 0);
        assert_eq!(server.windows.get(first_window).unwrap().references, 6);
        assert_eq!(server.windows.get(second_window).unwrap().references, 3);
        assert!(!session_is_linked(&server, a, second_window));
        assert!(session_is_linked(&server, a, first_window));
        let old_b = server
            .sessions
            .get(b)
            .unwrap()
            .windows
            .values()
            .copied()
            .collect::<Vec<_>>();
        server
            .winlinks
            .get_mut(first)
            .unwrap()
            .flags
            .insert(WinlinkFlags::ACTIVITY);
        server.effects.clear();
        session_group_synchronize_from(&mut server, a);
        for session in [b, c] {
            let s = server.sessions.get(session).unwrap();
            let current = server.winlinks.get(s.current.unwrap()).unwrap().index;
            assert_eq!(current, if session == b { 9 } else { 0 });
            let history = s
                .last
                .iter()
                .map(|id| server.winlinks.get(*id).unwrap().index)
                .collect::<Vec<_>>();
            assert_eq!(history, if session == b { vec![0, 4] } else { vec![9, 4] });
            assert!(
                server
                    .winlinks
                    .get(s.windows[&0])
                    .unwrap()
                    .flags
                    .contains(WinlinkFlags::ACTIVITY)
            );
        }
        assert!(old_b.iter().all(|id| server.winlinks.get(*id).is_none()));
        assert_eq!(server.windows.get(first_window).unwrap().references, 6);
        assert_eq!(server.windows.get(second_window).unwrap().references, 3);
        assert_eq!(
            session_effects(&server)
                .iter()
                .filter(|event| matches!(event, SessionEffect::WindowLinked { .. }))
                .count(),
            6
        );
        assert!(
            !session_effects(&server)
                .iter()
                .any(|event| matches!(event, SessionEffect::WindowUnlinked { .. }))
        );
        assert_eq!(
            session_detach(&mut server, a, second),
            DetachOutcome::Detached
        );
        for session in [a, b, c] {
            assert_eq!(
                server
                    .sessions
                    .get(session)
                    .unwrap()
                    .windows
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                [0, 9]
            );
        }
        assert!(server.windows.get(second_window).is_none());
    }

    #[test]
    fn rebuild_does_not_destroy_windows_held_only_by_replacement_links() {
        let mut server = server();
        let a = create(&mut server, Some(b"a"));
        let b = create(&mut server, Some(b"b"));
        let a_link = attach(&mut server, a, 0);
        session_set_current(&mut server, a, Some(a_link));
        let group = session_group_new(&mut server, b"g");
        session_group_add(&mut server, group, a);
        session_group_add(&mut server, group, b);
        session_group_synchronize_to(&mut server, b);
        let window = server.winlinks.get(a_link).unwrap().window;
        let old_b = server.sessions.get(b).unwrap().current.unwrap();
        session_group_synchronize_to(&mut server, b);
        assert!(server.winlinks.get(old_b).is_none());
        assert_eq!(server.windows.get(window).unwrap().references, 2);
        assert!(session_has(&server, b, window));
        let empty = create(&mut server, Some(b"empty-source"));
        session_group_add(&mut server, group, empty);
        let unchanged = server.sessions.get(b).unwrap().windows.clone();
        session_group_synchronize_from(&mut server, empty);
        assert_eq!(server.sessions.get(b).unwrap().windows, unchanged);
        server.effects.clear();
        session_renumber_windows(&mut server, b);
        assert_eq!(server.windows.get(window).unwrap().references, 2);
        assert!(!session_effects(&server).iter().any(|event| matches!(
            event,
            SessionEffect::WindowLinked { .. } | SessionEffect::WindowUnlinked { .. }
        )));
    }

    #[test]
    fn renumber_maps_mark_current_history_alerts_and_refuses_overflow() {
        let mut server = server();
        let session = create(&mut server, Some(b"renumber"));
        let a = attach(&mut server, session, 0);
        let b = attach(&mut server, session, 7);
        let c = attach(&mut server, session, i32::MAX);
        session_set_current(&mut server, session, Some(a));
        session_set_current(&mut server, session, Some(b));
        session_set_current(&mut server, session, Some(c));
        server.marked_winlink = Some(b);
        server
            .winlinks
            .get_mut(a)
            .unwrap()
            .flags
            .insert(WinlinkFlags::SILENCE);
        let options = server.sessions.get(session).unwrap().options;
        server
            .options
            .set_number(options, b"base-index", i64::from(i32::MAX - 1), &mut Parser);
        session_renumber_windows(&mut server, session);
        assert_eq!(
            server
                .sessions
                .get(session)
                .unwrap()
                .windows
                .values()
                .copied()
                .collect::<Vec<_>>(),
            [a, b, c]
        );
        server
            .options
            .set_number(options, b"base-index", 10, &mut Parser);
        server.effects.clear();
        session_renumber_windows(&mut server, session);
        let s = server.sessions.get(session).unwrap();
        assert_eq!(s.windows.keys().copied().collect::<Vec<_>>(), [10, 11, 12]);
        assert_eq!(s.current, Some(s.windows[&12]));
        assert_eq!(server.marked_winlink, Some(s.windows[&11]));
        assert_eq!(s.last, [s.windows[&11], s.windows[&10]]);
        assert!(
            server
                .winlinks
                .get(s.windows[&10])
                .unwrap()
                .flags
                .contains(WinlinkFlags::SILENCE)
        );
        assert!(
            [a, b, c]
                .iter()
                .all(|id| server.winlinks.get(*id).is_none())
        );
        assert!(!session_effects(&server).iter().any(|event| matches!(
            event,
            SessionEffect::WindowLinked { .. } | SessionEffect::WindowUnlinked { .. }
        )));
    }

    #[test]
    fn renumber_without_current_uses_index_zero_not_first_index() {
        let mut server = server();
        let session = create(&mut server, Some(b"empty-current"));
        attach(&mut server, session, 7);
        let options = server.sessions.get(session).unwrap().options;
        server
            .options
            .set_number(options, b"base-index", 2, &mut Parser);
        session_renumber_windows(&mut server, session);
        assert_eq!(server.sessions.get(session).unwrap().current, None);
        server
            .options
            .set_number(options, b"base-index", 0, &mut Parser);
        session_renumber_windows(&mut server, session);
        assert_eq!(
            server.sessions.get(session).unwrap().current,
            Some(server.sessions.get(session).unwrap().windows[&0])
        );
    }

    #[test]
    fn destroyed_unlinks_follow_accepted_sorted_teardown() {
        let mut server = server();
        let session = create(&mut server, Some(b"sorted"));
        let first = attach(&mut server, session, 8);
        attach(&mut server, session, 3);
        attach(&mut server, session, 12);
        attach(&mut server, session, 1);
        session_set_current(&mut server, session, Some(first));
        server.effects.clear();
        session_destroy(&mut server, session, false);
        let indexes = session_effects(&server)
            .iter()
            .filter_map(|effect| match effect {
                SessionEffect::WindowUnlinked { index, .. } => Some(*index),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(indexes, [1, 3, 8, 12]);
    }

    #[test]
    fn grouped_duplicate_links_match_pinned_oracle_after_attach_detach_and_renumber() {
        use std::path::{Path, PathBuf};
        use std::process::Command;
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let oracle = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../oracle/bin/tmux"
        ));
        if !oracle.is_file() {
            eprintln!(
                "skipping session group differential: pinned oracle missing at {}",
                oracle.display()
            );
            return;
        }
        struct Oracle<'a> {
            executable: &'a Path,
            socket: PathBuf,
        }
        impl Oracle<'_> {
            fn run(&self, args: &[&str]) -> Vec<u8> {
                let output = Command::new(self.executable)
                    .arg("-S")
                    .arg(&self.socket)
                    .args(["-f", "/dev/null"])
                    .args(args)
                    .env("TERM", "screen")
                    .env("LC_ALL", "C")
                    .output()
                    .expect("pinned oracle command");
                assert!(
                    output.status.success(),
                    "oracle {args:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                output.stdout
            }
        }
        impl Drop for Oracle<'_> {
            fn drop(&mut self) {
                let _ = Command::new(self.executable)
                    .arg("-S")
                    .arg(&self.socket)
                    .arg("kill-server")
                    .output();
                let _ = std::fs::remove_file(&self.socket);
            }
        }
        let oracle = Oracle {
            executable: oracle,
            socket: std::env::temp_dir().join(format!(
                "rmux-session-{}-{}.sock",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed),
            )),
        };
        oracle.run(&["new-session", "-d", "-s", "a", "-n", "one", "sleep 300"]);
        oracle.run(&["new-window", "-d", "-t", "a:4", "-n", "two", "sleep 300"]);
        oracle.run(&["link-window", "-d", "-s", "a:0", "-t", "a:9"]);
        oracle.run(&["new-session", "-d", "-s", "b", "-t", "a"]);
        oracle.run(&["new-session", "-d", "-s", "c", "-t", "a"]);
        let mut server = server();
        let a = create(&mut server, Some(b"a"));
        let b = create(&mut server, Some(b"b"));
        let c = create(&mut server, Some(b"c"));
        let first = attach(&mut server, a, 0);
        let second = attach(&mut server, a, 4);
        let first_window = server.winlinks.get(first).unwrap().window;
        let second_window = server.winlinks.get(second).unwrap().window;
        server.windows.get_mut(first_window).unwrap().name = b"one".to_vec();
        server.windows.get_mut(second_window).unwrap().name = b"two".to_vec();
        session_attach(&mut server, a, first_window, 9).unwrap();
        session_set_current(&mut server, a, Some(first));
        let group = session_group_new(&mut server, b"a");
        for session in [a, b, c] {
            session_group_add(&mut server, group, session);
        }
        session_group_synchronize_to(&mut server, b);
        session_group_synchronize_to(&mut server, c);
        for (name, session, index) in [("a", a, 4), ("b", b, 9), ("c", c, 0)] {
            oracle.run(&["select-window", "-t", &format!("{name}:{index}")]);
            session_select(&mut server, session, index);
        }
        let compare = |server: &Server| {
            for (name, session) in [("a", a), ("b", b), ("c", c)] {
                let s = server.sessions.get(session).unwrap();
                let mut rows = String::new();
                for (index, link) in &s.windows {
                    let wl = server.winlinks.get(*link).unwrap();
                    let window = server.windows.get(wl.window).unwrap();
                    rows.push_str(&format!(
                        "{index}:{}:{}:{}\n",
                        String::from_utf8_lossy(&window.name),
                        u8::from(s.current == Some(*link)),
                        u8::from(s.last.first() == Some(link))
                    ));
                }
                assert_eq!(
                    oracle.run(&[
                        "list-windows",
                        "-t",
                        name,
                        "-F",
                        "#{window_index}:#{window_name}:#{window_active}:#{window_last_flag}"
                    ]),
                    rows.as_bytes()
                );
            }
        };
        compare(&server);
        oracle.run(&["new-window", "-d", "-t", "a:6", "-n", "three", "sleep 300"]);
        let third_window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        server.windows.get_mut(third_window).unwrap().name = b"three".to_vec();
        session_attach(&mut server, a, third_window, 6).unwrap();
        compare(&server);
        oracle.run(&["unlink-window", "-k", "-t", "a:4"]);
        session_detach(&mut server, a, second);
        compare(&server);
        oracle.run(&["set-option", "-t", "b", "base-index", "10"]);
        oracle.run(&["move-window", "-r", "-t", "b"]);
        let options = server.sessions.get(b).unwrap().options;
        server
            .options
            .set_number(options, b"base-index", 10, &mut Parser);
        session_renumber_windows(&mut server, b);
        compare(&server);
        // Capture the documented sorted-vs-RB_ROOT teardown deviation.
        oracle.run(&["new-session", "-d", "-s", "observer", "sleep 300"]);
        oracle.run(&[
            "new-session",
            "-d",
            "-s",
            "doomed",
            "-n",
            "eight",
            "sleep 300",
        ]);
        oracle.run(&["move-window", "-s", "doomed:0", "-t", "doomed:8"]);
        for (index, name) in [(3, "three"), (12, "twelve"), (1, "one")] {
            oracle.run(&[
                "new-window",
                "-d",
                "-t",
                &format!("doomed:{index}"),
                "-n",
                name,
                "sleep 300",
            ]);
        }
        let window_ids = oracle.run(&["list-windows", "-t", "doomed", "-F", "#{window_id}"]);
        let expected_ids = String::from_utf8(window_ids)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        oracle.run(&["set-option", "-g", "@unlink-order", ""]);
        oracle.run(&[
            "set-hook",
            "-g",
            "window-unlinked",
            "set-option -agF @unlink-order '#{hook_window} '",
        ]);
        use std::io::Write;
        use std::process::Stdio;
        let mut client = Command::new(oracle.executable)
            .arg("-S")
            .arg(&oracle.socket)
            .args(["-f", "/dev/null", "-C", "attach-session", "-t", "observer"])
            .env("TERM", "screen")
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("control observer");
        client
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"kill-session -t doomed\ndetach-client\n")
            .unwrap();
        let _input = client.stdin.take().unwrap();
        let control = client.wait_with_output().expect("control unlink capture");
        assert!(
            control.status.success(),
            "{}",
            String::from_utf8_lossy(&control.stderr)
        );
        let control_text = String::from_utf8(control.stdout).unwrap();
        let control_ids = control_text
            .lines()
            .filter_map(|line| line.strip_prefix("%unlinked-window-close "))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let hooks = oracle.run(&["show-options", "-gqv", "@unlink-order"]);
        let hook_ids = String::from_utf8(hooks)
            .unwrap()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let sorted = |ids: &[String]| {
            let mut ids = ids.to_vec();
            ids.sort();
            ids
        };
        assert_eq!(sorted(&control_ids), sorted(&expected_ids));
        assert_eq!(sorted(&hook_ids), sorted(&expected_ids));
        eprintln!(
            "session teardown accepted deviation: sorted {expected_ids:?}; pinned control {control_ids:?}; pinned hooks {hook_ids:?}"
        );
    }

    #[test]
    fn theme_and_history_visit_every_duplicate_window_link() {
        let mut server = server();
        let session = create(&mut server, Some(b"panes"));
        let link = attach(&mut server, session, 0);
        let window = server.winlinks.get(link).unwrap().window;
        session_attach(&mut server, session, window, 4).unwrap();
        let pane = window::window_add_pane(
            &mut server,
            window,
            None,
            20,
            super::super::spawn::SpawnFlags::EMPTY,
        )
        .unwrap();
        session_theme_changed(&mut server, Some(session));
        assert!(
            server
                .panes
                .get(pane)
                .unwrap()
                .flags
                .contains(PaneFlags::THEMECHANGED)
        );
        session_theme_changed(&mut server, None);
        for _ in 0..4 {
            server
                .panes
                .get_mut(pane)
                .unwrap()
                .base
                .grid
                .scroll_history(rmux_emu::colour::Colour::DEFAULT);
        }
        let options = server.sessions.get(session).unwrap().options;
        server
            .options
            .set_number(options, b"history-limit", 3, &mut Parser);
        session_update_history(&mut server, session);
        let grid = &server.panes.get(pane).unwrap().base.grid;
        assert_eq!(grid.hlimit(), 3);
        // grid_collect_history also collects one line when size equals limit.
        assert_eq!(grid.hsize(), 2);
    }

    #[test]
    fn selection_clears_alerts_on_every_link_and_offset_precedes_changed() {
        let mut server = server();
        let a = create(&mut server, Some(b"a"));
        let b = create(&mut server, Some(b"b"));
        let link = attach(&mut server, a, 0);
        let window = server.winlinks.get(link).unwrap().window;
        let duplicate = session_attach(&mut server, a, window, 7).unwrap();
        let other = session_attach(&mut server, b, window, 2).unwrap();
        for id in [link, duplicate, other] {
            server
                .winlinks
                .get_mut(id)
                .unwrap()
                .flags
                .insert(WinlinkFlags::ALERTFLAGS);
        }
        server
            .windows
            .get_mut(window)
            .unwrap()
            .flags
            .insert(super::super::WindowFlags::ALERTFLAGS);
        server.current_time = (888, 4);
        server.effects.clear();
        session_set_current(&mut server, a, Some(link));
        for id in [link, duplicate, other] {
            assert!(
                !server
                    .winlinks
                    .get(id)
                    .unwrap()
                    .flags
                    .intersects(WinlinkFlags::ALERTFLAGS)
            );
        }
        // session_set_current clears WINDOW_ALERTFLAGS, then window_update_activity
        // queues WINDOW_ACTIVITY. monitor-activity defaults off, so the flag stays
        // until a later alerts_dispatch only when monitoring is enabled.
        assert_eq!(
            server.windows.get(window).unwrap().flags & super::super::WindowFlags::ALERTFLAGS,
            super::super::WindowFlags::ACTIVITY
        );
        assert!(!server.alerts.is_pending(window));
        assert_eq!(server.windows.get(window).unwrap().activity, (888, 4));
        assert!(server.effects.iter().any(|effect| matches!(
            effect,
            ModelEffect::Alert(super::super::alerts::AlertEffect::SilenceTimer {
                window: id,
                seconds: 0,
            }) if *id == window
        )));
        let effects = session_effects(&server);
        assert_eq!(
            effects[effects.len() - 2],
            &SessionEffect::WindowOffset(window)
        );
        assert_eq!(
            effects.last().copied(),
            Some(&SessionEffect::WindowChanged {
                session: a,
                new_window: window,
                new_index: 0,
                old: None,
            })
        );
    }

    #[test]
    fn session_events_dispatch_once_and_closed_callback_can_retain() {
        fn retain_closed(
            server: &mut Server,
            name: &[u8],
            session: Option<SessionId>,
            _: Option<WindowId>,
            _: Option<crate::ids::PaneId>,
        ) {
            if name == b"session-closed" {
                let session = session.unwrap();
                assert!(!session_alive(server, session));
                assert!(session_retain(server, session));
            }
        }
        let mut server = server();
        let session = create(&mut server, Some(b"callback"));
        let link = attach(&mut server, session, 0);
        session_set_current(&mut server, session, Some(link));
        server.model_event = Some(retain_closed);
        server.effects.clear();
        session_destroy(&mut server, session, true);
        assert_eq!(server.sessions.get(session).unwrap().references, 1);
        assert!(!session_free(&mut server, session));
        assert_eq!(
            session_effects(&server)
                .iter()
                .filter(|event| matches!(event, SessionEffect::SessionClosed(_)))
                .count(),
            1
        );
        assert!(!server.effects.iter().any(
            |effect| matches!(effect, ModelEffect::Event { name, .. } if name == b"session-closed")
        ));
        session_release(&mut server, session);
        assert!(session_free(&mut server, session));
    }

    #[test]
    fn window_changed_callback_can_destroy_selected_session() {
        fn destroy_changed(
            server: &mut Server,
            name: &[u8],
            session: Option<SessionId>,
            _: Option<WindowId>,
            _: Option<crate::ids::PaneId>,
        ) {
            if name == b"session-window-changed" {
                session_destroy(server, session.unwrap(), false);
            }
        }
        let mut server = server();
        let session = create(&mut server, Some(b"callback"));
        let link = attach(&mut server, session, 0);
        server.model_event = Some(destroy_changed);
        assert_eq!(
            session_set_current(&mut server, session, Some(link)),
            SelectOutcome::Changed
        );
        assert!(!session_alive(&server, session));
        assert!(server.winlinks.get(link).is_none());
        assert!(session_free(&mut server, session));
    }

    #[test]
    fn linked_callback_can_remove_link_without_stale_attachment_success() {
        fn remove_linked(
            server: &mut Server,
            name: &[u8],
            session: Option<SessionId>,
            window: Option<WindowId>,
            _: Option<crate::ids::PaneId>,
        ) {
            if name == b"window-linked" {
                if let Some(link) =
                    window::winlink_find_by_window(server, session.unwrap(), window.unwrap())
                {
                    window::winlink_remove(server, link);
                }
            }
        }
        let mut server = server();
        let session = create(&mut server, Some(b"callback"));
        let window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        server.model_event = Some(remove_linked);
        assert!(matches!(
            session_attach(&mut server, session, window, 0),
            Err(ModelError::StaleId)
        ));
        assert!(server.sessions.get(session).unwrap().windows.is_empty());
    }
}
