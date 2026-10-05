// Ported from tmux window.c @ 8f25579c
use super::state::{ModelEffect, ModelError, Server, Winlink};
use super::{WindowFlags, WinlinkFlags, session::SessionEffect, window};
use crate::ids::{SessionId, WindowId, WinlinkId};
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

pub fn winlink_cmp(left: &Winlink, right: &Winlink) -> std::cmp::Ordering {
    left.index.cmp(&right.index)
}

pub fn winlink_find_by_window(
    server: &Server,
    session: SessionId,
    window: WindowId,
) -> Option<WinlinkId> {
    server
        .sessions
        .get(session)?
        .windows
        .values()
        .copied()
        .find(|id| {
            server
                .winlinks
                .get(*id)
                .is_some_and(|wl| wl.window == window)
        })
}

pub fn winlink_find_by_index(server: &Server, session: SessionId, index: i32) -> Option<WinlinkId> {
    assert!(index >= 0, "bad index");
    server.sessions.get(session)?.windows.get(&index).copied()
}

pub fn winlink_find_by_window_id(
    server: &Server,
    session: SessionId,
    public_id: u32,
) -> Option<WinlinkId> {
    server
        .sessions
        .get(session)?
        .windows
        .values()
        .copied()
        .find(|id| {
            server
                .winlinks
                .get(*id)
                .and_then(|wl| server.windows.get(wl.window))
                .is_some_and(|w| w.public_id == public_id)
        })
}

fn next_index(windows: &BTreeMap<i32, WinlinkId>, start: i32) -> Option<i32> {
    let mut index = start;
    loop {
        if !windows.contains_key(&index) {
            return Some(index);
        }
        index = if index == i32::MAX { 0 } else { index + 1 };
        if index == start {
            return None;
        }
    }
}

pub fn winlink_next_index(server: &Server, session: SessionId, start: i32) -> Option<i32> {
    assert!(start >= 0, "bad index");
    next_index(&server.sessions.get(session)?.windows, start)
}

pub fn winlink_count(server: &Server, session: SessionId) -> u32 {
    server
        .sessions
        .get(session)
        .map_or(0, |s| s.windows.len() as u32)
}

// Allocation and window assignment are atomic because Winlink always has a valid owner.
pub fn winlink_add(
    server: &mut Server,
    session: SessionId,
    owner: WindowId,
    request: i32,
) -> Result<WinlinkId, ModelError> {
    let s = server.sessions.get(session).ok_or(ModelError::StaleId)?;
    if server.windows.get(owner).is_none() {
        return Err(ModelError::StaleId);
    }
    let index = if request < 0 {
        next_index(&s.windows, (-i64::from(request) - 1) as i32)
    } else if s.windows.contains_key(&request) {
        None
    } else {
        Some(request)
    };
    let index = index
        .ok_or_else(|| ModelError::Message(format!("index in use: {request}").into_bytes()))?;
    let link = server.winlinks.insert(Winlink {
        index,
        session,
        window: owner,
        flags: WinlinkFlags::default(),
    })?;
    window::window_retain(server, owner).expect("winlink window reference");
    server
        .windows
        .get_mut(owner)
        .expect("winlink window")
        .links
        .push(link);
    let s = server.sessions.get_mut(session).expect("winlink session");
    s.windows.insert(index, link);
    let position = s
        .ordered_winlinks
        .partition_point(|id| server.winlinks.get(*id).expect("ordered winlink").index < index);
    s.ordered_winlinks.insert(position, link);
    Ok(link)
}

pub fn winlink_set_window(
    server: &mut Server,
    link: WinlinkId,
    owner: WindowId,
) -> Result<(), ModelError> {
    let old = server.winlinks.get(link).ok_or(ModelError::StaleId)?.window;
    window::window_retain(server, owner)?;
    server
        .windows
        .get_mut(old)
        .ok_or(ModelError::StaleId)?
        .links
        .retain(|id| *id != link);
    server
        .windows
        .get_mut(owner)
        .ok_or(ModelError::StaleId)?
        .links
        .push(link);
    server
        .winlinks
        .get_mut(link)
        .expect("winlink assignment")
        .window = owner;
    window::window_release(server, old)?;
    Ok(())
}

pub fn winlink_remove(server: &mut Server, link: WinlinkId) {
    let Some(wl) = server.winlinks.get(link) else {
        return;
    };
    let (session, owner, index) = (wl.session, wl.window, wl.index);
    if let Some(w) = server.windows.get_mut(owner) {
        w.links.retain(|id| *id != link);
    }
    if let Some(s) = server.sessions.get_mut(session) {
        // A rebuild may already have installed a different link at this index.
        if s.windows.get(&index) == Some(&link) {
            s.windows.remove(&index);
        }
        s.ordered_winlinks.retain(|id| *id != link);
    }
    window::window_release(server, owner).expect("winlink window release");
    let _ = server
        .winlinks
        .request_remove(link)
        .expect("winlink removal");
}

pub fn winlink_next(server: &Server, link: WinlinkId) -> Option<WinlinkId> {
    let wl = server.winlinks.get(link)?;
    server
        .sessions
        .get(wl.session)?
        .windows
        .range((Excluded(wl.index), Unbounded))
        .next()
        .map(|(_, id)| *id)
}

pub fn winlink_previous(server: &Server, link: WinlinkId) -> Option<WinlinkId> {
    let wl = server.winlinks.get(link)?;
    server
        .sessions
        .get(wl.session)?
        .windows
        .range((Unbounded, Excluded(wl.index)))
        .next_back()
        .map(|(_, id)| *id)
}

pub fn winlink_next_by_number(
    server: &Server,
    mut link: WinlinkId,
    session: SessionId,
    count: i32,
) -> Option<WinlinkId> {
    for _ in 0..count {
        link = winlink_next(server, link).or_else(|| {
            server
                .sessions
                .get(session)?
                .windows
                .first_key_value()
                .map(|(_, id)| *id)
        })?;
    }
    server.winlinks.get(link).map(|_| link)
}

pub fn winlink_previous_by_number(
    server: &Server,
    mut link: WinlinkId,
    session: SessionId,
    count: i32,
) -> Option<WinlinkId> {
    for _ in 0..count {
        link = winlink_previous(server, link).or_else(|| {
            server
                .sessions
                .get(session)?
                .windows
                .last_key_value()
                .map(|(_, id)| *id)
        })?;
    }
    server.winlinks.get(link).map(|_| link)
}

pub fn winlink_stack_push(server: &mut Server, session: SessionId, link: Option<WinlinkId>) {
    let Some(link) = link else {
        return;
    };
    winlink_stack_remove(server, session, link);
    let Some(wl) = server.winlinks.get_mut(link) else {
        return;
    };
    assert_eq!(wl.session, session, "winlink history owner");
    wl.flags.insert(WinlinkFlags::VISITED);
    server
        .sessions
        .get_mut(session)
        .expect("winlink history session")
        .last
        .insert(0, link);
}

pub fn winlink_stack_remove(server: &mut Server, session: SessionId, link: WinlinkId) {
    let Some(wl) = server.winlinks.get_mut(link) else {
        return;
    };
    if wl.flags.contains(WinlinkFlags::VISITED) {
        if let Some(s) = server.sessions.get_mut(session) {
            s.last.retain(|id| *id != link);
        }
        wl.flags.remove(WinlinkFlags::VISITED);
    }
}

pub fn winlink_clear_flags(server: &mut Server, link: WinlinkId) {
    let Some(owner) = server.winlinks.get(link).map(|wl| wl.window) else {
        return;
    };
    let Some(w) = server.windows.get_mut(owner) else {
        return;
    };
    w.flags.remove(WindowFlags::ALERTFLAGS);
    for id in &w.links {
        if let Some(wl) = server.winlinks.get_mut(*id) {
            if wl.flags.intersects(WinlinkFlags::ALERTFLAGS) {
                wl.flags.remove(WinlinkFlags::ALERTFLAGS);
                server
                    .effects
                    .push_back(ModelEffect::Session(SessionEffect::Status(wl.session)));
            }
        }
    }
}

pub fn winlink_shuffle_up(
    server: &mut Server,
    session: SessionId,
    link: Option<WinlinkId>,
    before: bool,
) -> Option<i32> {
    let wl = server.winlinks.get(link?)?;
    if wl.session != session || wl.index == i32::MAX {
        return None;
    }
    let index = if before { wl.index } else { wl.index + 1 };
    let s = server.sessions.get_mut(session)?;
    let mut last = index;
    while last < i32::MAX && s.windows.contains_key(&last) {
        last += 1;
    }
    if last == i32::MAX {
        return None;
    }
    while last > index {
        let id = s
            .windows
            .remove(&(last - 1))
            .expect("shuffle occupied index");
        server.winlinks.get_mut(id).expect("shuffle link").index = last;
        s.windows.insert(last, id);
        last -= 1;
    }
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::super::session::{SessionCreate, session_create};
    use super::*;
    use crate::options::environment::Environment;

    fn fixture() -> (Server, SessionId, WindowId) {
        let mut server = Server::new();
        let options = server.options.create(Some(server.options.global_s));
        let session = session_create(
            &mut server,
            SessionCreate {
                prefix: None,
                name: Some(b"links".to_vec()),
                cwd: Vec::new(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        );
        let window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        (server, session, window)
    }

    #[test]
    fn negative_allocation_wraps_without_int_min_overflow() {
        let (mut server, session, window) = fixture();
        let max = winlink_add(&mut server, session, window, i32::MIN).unwrap();
        assert_eq!(server.winlinks.get(max).unwrap().index, i32::MAX);
        let zero = winlink_add(&mut server, session, window, i32::MIN).unwrap();
        assert_eq!(server.winlinks.get(zero).unwrap().index, 0);
        let one = winlink_add(&mut server, session, window, -1).unwrap();
        assert_eq!(server.winlinks.get(one).unwrap().index, 1);
        assert_eq!(winlink_count(&server, session), 3);
        assert_eq!(
            server.sessions.get(session).unwrap().ordered_winlinks,
            [zero, one, max]
        );
        assert_eq!(server.windows.get(window).unwrap().links, [max, zero, one]);
        assert_eq!(server.windows.get(window).unwrap().references, 3);
        assert_eq!(
            winlink_add(&mut server, session, window, 1)
                .unwrap_err()
                .to_string(),
            "index in use: 1"
        );
        assert_eq!(winlink_find_by_window(&server, session, window), Some(zero));
        assert_eq!(winlink_next_index(&server, session, i32::MAX), Some(2));
    }

    #[test]
    fn traversal_and_unique_visited_stack() {
        let (mut server, session, window) = fixture();
        let a = winlink_add(&mut server, session, window, 0).unwrap();
        let b = winlink_add(&mut server, session, window, 3).unwrap();
        let c = winlink_add(&mut server, session, window, 7).unwrap();
        assert_eq!(winlink_previous(&server, a), None);
        assert_eq!(winlink_next(&server, c), None);
        assert_eq!(winlink_next_by_number(&server, c, session, 4), Some(a));
        assert_eq!(winlink_previous_by_number(&server, a, session, 4), Some(c));
        assert_eq!(winlink_previous_by_number(&server, b, session, -1), Some(b));
        winlink_stack_push(&mut server, session, None);
        winlink_stack_push(&mut server, session, Some(a));
        winlink_stack_push(&mut server, session, Some(b));
        winlink_stack_push(&mut server, session, Some(a));
        assert_eq!(server.sessions.get(session).unwrap().last, [a, b]);
        winlink_stack_remove(&mut server, session, a);
        assert_eq!(server.sessions.get(session).unwrap().last, [b]);
        assert!(
            !server
                .winlinks
                .get(a)
                .unwrap()
                .flags
                .contains(WinlinkFlags::VISITED)
        );
    }

    #[test]
    fn shuffle_moves_keys_not_handles_and_never_uses_maximum_slot() {
        let (mut server, session, window) = fixture();
        let a = winlink_add(&mut server, session, window, 4).unwrap();
        let b = winlink_add(&mut server, session, window, 5).unwrap();
        let max = winlink_add(&mut server, session, window, i32::MAX).unwrap();
        assert_eq!(
            winlink_shuffle_up(&mut server, session, Some(a), true),
            Some(4)
        );
        assert_eq!(winlink_find_by_index(&server, session, 5), Some(a));
        assert_eq!(winlink_find_by_index(&server, session, 6), Some(b));
        assert_eq!(
            server.sessions.get(session).unwrap().ordered_winlinks,
            [a, b, max]
        );
        assert_eq!(
            winlink_shuffle_up(&mut server, session, Some(max), false),
            None
        );
        assert_eq!(winlink_shuffle_up(&mut server, session, None, true), None);
        let almost = winlink_add(&mut server, session, window, i32::MAX - 1).unwrap();
        assert_eq!(
            winlink_shuffle_up(&mut server, session, Some(almost), true),
            None
        );
        assert_eq!(
            winlink_shuffle_up(&mut server, session, Some(almost), false),
            None
        );
    }

    #[test]
    fn replacement_links_protect_windows_and_old_removal_keeps_new_index_entry() {
        let (mut server, session, window) = fixture();
        let old = winlink_add(&mut server, session, window, 0).unwrap();
        server.sessions.get_mut(session).unwrap().windows.clear();
        server
            .sessions
            .get_mut(session)
            .unwrap()
            .ordered_winlinks
            .clear();
        let new = winlink_add(&mut server, session, window, 0).unwrap();
        winlink_remove(&mut server, old);
        assert_eq!(winlink_find_by_index(&server, session, 0), Some(new));
        assert_eq!(server.windows.get(window).unwrap().references, 1);
        assert_eq!(server.windows.get(window).unwrap().links, [new]);
        assert!(server.winlinks.get(old).is_none());
        winlink_set_window(&mut server, new, window).unwrap();
        assert_eq!(server.windows.get(window).unwrap().references, 1);
        winlink_remove(&mut server, new);
        assert!(server.windows.get(window).is_none());
    }
}
