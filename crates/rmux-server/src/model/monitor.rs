// Ported from tmux monitor.c, tmux.h @ 8f25579c
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
pub enum MonitorType {
    AllWindows = 4,
    Session = 0,
    Pane = 1,
    AllPanes = 2,
    Window = 3,
}
impl TryFrom<i32> for MonitorType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            4 => Ok(Self::AllWindows),
            0 => Ok(Self::Session),
            1 => Ok(Self::Pane),
            2 => Ok(Self::AllPanes),
            3 => Ok(Self::Window),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MonitorFlags(pub u32);
impl MonitorFlags {
    pub const INITIAL: Self = Self(1);
    pub const TRUE: Self = Self(2);
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
impl std::ops::BitOr for MonitorFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for MonitorFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for MonitorFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use super::state::{ModelError, Server};
use crate::ids::{ClientId, MonitorSetId, PaneId, SessionId, WinlinkId};
use rmux_util::bytes::{ByteString, cstr};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct MonitorSpec {
    pub name: ByteString,
    pub kind: MonitorType,
    pub target: Option<u32>,
    pub format: ByteString,
    pub flags: MonitorFlags,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MonitorContext {
    pub client: Option<ClientId>,
    pub session: Option<SessionId>,
    pub winlink: Option<WinlinkId>,
    pub pane: Option<PaneId>,
}

pub struct MonitorChange<'a> {
    pub name: &'a [u8],
    pub value: &'a [u8],
    pub last: Option<&'a [u8]>,
    pub context: MonitorContext,
}

pub type MonitorCallback = fn(&mut Server, MonitorSetId, &MonitorChange<'_>);

pub trait MonitorRuntime {
    fn client_session(&self, client: ClientId) -> Option<SessionId>;
    /// Must expand with FORMAT_NOJOBS and the supplied default object context.
    fn expand(&mut self, server: &mut Server, context: MonitorContext, format: &[u8]) -> Vec<u8>;
    fn timer(&mut self, set: MonitorSetId, pending: bool);
}

#[derive(Default)]
struct MonitorCache {
    last: Option<Vec<u8>>,
    generation: u32,
}

struct MonitorItem {
    spec: MonitorSpec,
    serial: u64,
    session: MonitorCache,
    targets: BTreeMap<(u32, i32), MonitorCache>,
    fire_count: u32,
    fire_time: i64,
}

pub struct MonitorSet {
    pub client: Option<ClientId>,
    pub session: Option<SessionId>,
    pub callback: MonitorCallback,
    items: BTreeMap<ByteString, MonitorItem>,
    pub timer_pending: bool,
    generation: u32,
    next_serial: u64,
}

fn numeric_target(target: &[u8]) -> Option<u32> {
    let mut at = 1;
    while target.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    let negative = target.get(at) == Some(&b'-');
    if matches!(target.get(at), Some(b'+' | b'-')) {
        at += 1;
    }
    let start = at;
    let mut value = 0u32;
    while let Some(byte @ b'0'..=b'9') = target.get(at) {
        value = value.wrapping_mul(10).wrapping_add(u32::from(*byte - b'0'));
        at += 1;
    }
    if at == start {
        return None;
    }
    let signed = if negative {
        value.wrapping_neg() as i32
    } else {
        value as i32
    };
    (signed >= 0).then_some(signed as u32)
}

pub fn monitor_parse(value: &[u8]) -> Result<MonitorSpec, ModelError> {
    let mut parts = cstr(value).splitn(3, |b| *b == b':');
    let name = parts.next().unwrap_or_default();
    let target = parts
        .next()
        .ok_or_else(|| ModelError::message(b"invalid subscription"))?;
    let format = parts
        .next()
        .ok_or_else(|| ModelError::message(b"invalid subscription"))?;
    let (kind, id) = match target {
        b"" => (MonitorType::Session, None),
        b"%*" => (MonitorType::AllPanes, None),
        b"@*" => (MonitorType::AllWindows, None),
        _ if target.first() == Some(&b'%') => (
            MonitorType::Pane,
            Some(
                numeric_target(target)
                    .ok_or_else(|| ModelError::message(b"invalid subscription"))?,
            ),
        ),
        _ if target.first() == Some(&b'@') => (
            MonitorType::Window,
            Some(
                numeric_target(target)
                    .ok_or_else(|| ModelError::message(b"invalid subscription"))?,
            ),
        ),
        _ => return Err(ModelError::message(b"invalid subscription")),
    };
    Ok(MonitorSpec {
        name: name.into(),
        kind,
        target: id,
        format: format.into(),
        flags: MonitorFlags::default(),
    })
}

fn create(
    server: &mut Server,
    client: Option<ClientId>,
    session: Option<SessionId>,
    callback: MonitorCallback,
) -> Result<MonitorSetId, ModelError> {
    if let Some(session) = session {
        if !super::session::session_retain(server, session) {
            return Err(ModelError::StaleId);
        }
    }
    let result = server.monitors.insert(MonitorSet {
        client,
        session,
        callback,
        items: BTreeMap::new(),
        timer_pending: false,
        generation: 0,
        next_serial: 0,
    });
    match result {
        Ok(id) => Ok(id),
        Err(error) => {
            if let Some(session) = session {
                super::session::session_release(server, session);
            }
            Err(error.into())
        }
    }
}

pub fn monitor_create_client(
    server: &mut Server,
    client: ClientId,
    callback: MonitorCallback,
) -> Result<MonitorSetId, ModelError> {
    create(server, Some(client), None, callback)
}
pub fn monitor_create_session(
    server: &mut Server,
    session: Option<SessionId>,
    callback: MonitorCallback,
) -> Result<MonitorSetId, ModelError> {
    create(server, None, session, callback)
}

pub fn monitor_destroy(
    server: &mut Server,
    id: MonitorSetId,
    runtime: &mut impl MonitorRuntime,
) -> Result<(), ModelError> {
    let set = server.monitors.get_mut(id).ok_or(ModelError::StaleId)?;
    let session = set.session.take();
    set.items.clear();
    set.timer_pending = false;
    runtime.timer(id, false);
    server.monitors.request_remove(id)?;
    if let Some(session) = session {
        super::session::session_release(server, session);
    }
    Ok(())
}

pub fn monitor_add(
    server: &mut Server,
    id: MonitorSetId,
    spec: MonitorSpec,
    runtime: &mut impl MonitorRuntime,
) -> Result<(), ModelError> {
    let set = server.monitors.get_mut(id).ok_or(ModelError::StaleId)?;
    let serial = set.next_serial;
    set.next_serial = set.next_serial.wrapping_add(1);
    set.items.insert(
        spec.name.clone(),
        MonitorItem {
            spec,
            serial,
            session: MonitorCache::default(),
            targets: BTreeMap::new(),
            fire_count: 0,
            fire_time: 0,
        },
    );
    if !set.timer_pending {
        set.timer_pending = true;
        runtime.timer(id, true);
    }
    Ok(())
}

pub fn monitor_remove(
    server: &mut Server,
    id: MonitorSetId,
    name: &[u8],
    runtime: &mut impl MonitorRuntime,
) -> Result<(), ModelError> {
    let set = server.monitors.get_mut(id).ok_or(ModelError::StaleId)?;
    set.items.remove(cstr(name));
    if set.items.is_empty() {
        set.timer_pending = false;
        runtime.timer(id, false);
    }
    Ok(())
}

pub fn monitor_get_fire_count(server: &Server, id: MonitorSetId, name: &[u8]) -> u32 {
    server
        .monitors
        .get(id)
        .and_then(|s| s.items.get(cstr(name)))
        .map_or(0, |i| i.fire_count)
}
pub fn monitor_get_fire_time(server: &Server, id: MonitorSetId, name: &[u8]) -> i64 {
    server
        .monitors
        .get(id)
        .and_then(|s| s.items.get(cstr(name)))
        .map_or(0, |i| i.fire_time)
}

fn resolved_session(
    server: &Server,
    set: &MonitorSet,
    runtime: &impl MonitorRuntime,
) -> Option<SessionId> {
    if let Some(client) = set.client {
        return runtime.client_session(client);
    }
    if let Some(session) = set.session {
        let s = server.sessions.get(session)?;
        return (server.session_names.get(&s.name) == Some(&session)).then_some(session);
    }
    server.session_names.first_key_value().map(|(_, id)| *id)
}

fn true_value(value: &[u8]) -> bool {
    let value = cstr(value);
    !value.is_empty() && value != b"0"
}

#[allow(clippy::too_many_arguments)] // Keep the value/context/cache callback boundary together.
fn check_value(
    server: &mut Server,
    id: MonitorSetId,
    name: &[u8],
    serial: u64,
    key: Option<(u32, i32)>,
    generation: u32,
    context: MonitorContext,
    value: Vec<u8>,
) {
    let Some(set) = server.monitors.get_mut(id) else {
        return;
    };
    let Some(item) = set.items.get_mut(name).filter(|item| item.serial == serial) else {
        return;
    };
    let cache = match key {
        None => &mut item.session,
        Some(key) => item.targets.entry(key).or_default(),
    };
    cache.generation = generation;
    if cache.last.as_ref() == Some(&value) {
        return;
    }
    let report = (cache.last.is_some() || item.spec.flags.contains(MonitorFlags::INITIAL))
        && (!item.spec.flags.contains(MonitorFlags::TRUE) || true_value(&value));
    let last = cache.last.replace(value);
    if !report {
        return;
    }
    item.fire_count = item.fire_count.wrapping_add(1);
    item.fire_time = server.current_time.0;
    let value = match key {
        None => item.session.last.as_ref(),
        Some(key) => item.targets.get(&key).and_then(|c| c.last.as_ref()),
    }
    .cloned()
    .unwrap_or_default();
    let callback = set.callback;
    callback(
        server,
        id,
        &MonitorChange {
            name,
            value: &value,
            last: last.as_deref(),
            context,
        },
    );
}

fn next_item(
    server: &Server,
    id: MonitorSetId,
    previous: Option<&[u8]>,
    kind: impl Fn(MonitorType) -> bool,
) -> Option<(MonitorSpec, u64)> {
    let set = server.monitors.get(id)?;
    set.items
        .iter()
        .find(|(name, item)| {
            previous.is_none_or(|previous| name.as_bytes() > previous) && kind(item.spec.kind)
        })
        .map(|(_, item)| (item.spec.clone(), item.serial))
}

#[allow(clippy::too_many_arguments)] // Shares the value/context/cache boundary with check_value.
fn scan_item(
    server: &mut Server,
    id: MonitorSetId,
    spec: &MonitorSpec,
    serial: u64,
    generation: u32,
    context: MonitorContext,
    key: Option<(u32, i32)>,
    runtime: &mut impl MonitorRuntime,
) {
    let value = runtime.expand(server, context, &spec.format);
    check_value(
        server, id, &spec.name, serial, key, generation, context, value,
    );
}

fn scan(server: &mut Server, id: MonitorSetId, runtime: &mut impl MonitorRuntime) {
    let Some(set) = server.monitors.get(id) else {
        return;
    };
    let Some(session) = resolved_session(server, set, runtime) else {
        return;
    };
    let client = set.client;
    let context = MonitorContext {
        client,
        session: Some(session),
        ..MonitorContext::default()
    };
    let mut previous: Option<ByteString> = None;
    while let Some((spec, serial)) = next_item(
        server,
        id,
        previous.as_deref().map(|v| v.as_slice()),
        |kind| kind == MonitorType::Session,
    ) {
        scan_item(server, id, &spec, serial, 0, context, None, runtime);
        previous = Some(spec.name);
    }
    previous = None;
    while let Some((spec, serial)) = next_item(
        server,
        id,
        previous.as_deref().map(|v| v.as_slice()),
        |kind| matches!(kind, MonitorType::Pane | MonitorType::Window),
    ) {
        let pane = if spec.kind == MonitorType::Pane {
            spec.target
                .and_then(|target| server.pane_ids.get(&target).copied())
                .filter(|p| server.panes.get(*p).is_some_and(|p| p.has_fd()))
        } else {
            None
        };
        let window = if spec.kind == MonitorType::Pane {
            pane.and_then(|p| server.panes.get(p).map(|p| p.window))
        } else {
            spec.target
                .and_then(|target| server.window_ids.get(&target).copied())
        };
        let links = window
            .and_then(|w| server.windows.get(w))
            .map(|w| w.links.clone())
            .unwrap_or_default();
        for link in links {
            let Some(wl) = server.winlinks.get(link).filter(|wl| wl.session == session) else {
                continue;
            };
            let key = (spec.target.unwrap_or_default(), wl.index);
            scan_item(
                server,
                id,
                &spec,
                serial,
                0,
                MonitorContext {
                    winlink: Some(link),
                    pane,
                    ..context
                },
                Some(key),
                runtime,
            );
        }
        previous = Some(spec.name);
    }
    for kind in [MonitorType::AllPanes, MonitorType::AllWindows] {
        if next_item(server, id, None, |k| k == kind).is_none() {
            continue;
        }
        let Some(set) = server.monitors.get_mut(id) else {
            return;
        };
        set.generation = set.generation.wrapping_add(1).max(1);
        let generation = set.generation;
        let links = server
            .sessions
            .get(session)
            .map(|s| s.windows.values().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for link in links {
            let Some(wl) = server.winlinks.get(link) else {
                continue;
            };
            let index = wl.index;
            let Some(window) = server.windows.get(wl.window) else {
                continue;
            };
            let targets = if kind == MonitorType::AllPanes {
                window
                    .panes
                    .iter()
                    .filter_map(|id| server.panes.get(*id).map(|p| (p.public_id, Some(*id))))
                    .collect::<Vec<_>>()
            } else {
                vec![(window.public_id, None)]
            };
            for (target, pane) in targets {
                previous = None;
                while let Some((spec, serial)) =
                    next_item(server, id, previous.as_deref().map(|v| v.as_slice()), |k| {
                        k == kind
                    })
                {
                    scan_item(
                        server,
                        id,
                        &spec,
                        serial,
                        generation,
                        MonitorContext {
                            winlink: Some(link),
                            pane,
                            ..context
                        },
                        Some((target, index)),
                        runtime,
                    );
                    previous = Some(spec.name);
                }
            }
        }
        if let Some(set) = server.monitors.get_mut(id) {
            for item in set.items.values_mut().filter(|item| item.spec.kind == kind) {
                item.targets
                    .retain(|_, cache| cache.generation == generation);
            }
        }
    }
}

pub fn monitor_check(
    server: &mut Server,
    id: MonitorSetId,
    runtime: &mut impl MonitorRuntime,
) -> Result<(), ModelError> {
    server.monitors.retain(id)?;
    if let Some(set) = server.monitors.get_mut(id) {
        set.timer_pending = true;
    }
    runtime.timer(id, true);
    scan(server, id, runtime);
    server.monitors.release(id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inert(_: &mut Server, _: MonitorSetId, _: &MonitorChange<'_>) {}
    struct Runtime;
    impl MonitorRuntime for Runtime {
        fn client_session(&self, _: ClientId) -> Option<SessionId> {
            None
        }
        fn expand(&mut self, _: &mut Server, _: MonitorContext, format: &[u8]) -> Vec<u8> {
            format.to_vec()
        }
        fn timer(&mut self, _: MonitorSetId, _: bool) {}
    }
    struct ClientRuntime {
        session: Option<SessionId>,
    }
    impl MonitorRuntime for ClientRuntime {
        fn client_session(&self, _: ClientId) -> Option<SessionId> {
            self.session
        }
        fn expand(&mut self, server: &mut Server, context: MonitorContext, _: &[u8]) -> Vec<u8> {
            context
                .session
                .and_then(|id| server.sessions.get(id))
                .unwrap()
                .name
                .clone()
        }
        fn timer(&mut self, _: MonitorSetId, _: bool) {}
    }
    #[test]
    fn client_switch_and_removed_explicit_session() {
        use super::super::session;
        use crate::ids::ArenaId;
        let mut server = Server::new();
        let mut make = |name: &[u8]| {
            let options = server.options.create(None);
            session::session_create(
                &mut server,
                session::SessionCreate {
                    prefix: None,
                    name: Some(name.to_vec()),
                    cwd: Vec::new(),
                    environment: crate::options::environment::Environment::default(),
                    options,
                    termios: None,
                },
            )
        };
        let b = make(b"b");
        let a = make(b"a");
        let mut runtime = ClientRuntime { session: Some(b) };
        let client = ClientId::from_parts(0, 0);
        let follow = monitor_create_client(&mut server, client, inert).unwrap();
        let first = monitor_create_session(&mut server, None, inert).unwrap();
        let fixed = monitor_create_session(&mut server, Some(b), inert).unwrap();
        let mut spec = monitor_parse(b"item::f").unwrap();
        spec.flags = MonitorFlags::INITIAL;
        for id in [follow, first, fixed] {
            monitor_add(&mut server, id, spec.clone(), &mut runtime).unwrap();
            monitor_check(&mut server, id, &mut runtime).unwrap();
        }
        assert_eq!(
            server.monitors.get(first).unwrap().items[b"item".as_slice()]
                .session
                .last
                .as_deref(),
            Some(b"a".as_slice())
        );
        runtime.session = Some(a);
        monitor_check(&mut server, follow, &mut runtime).unwrap();
        assert_eq!(monitor_get_fire_count(&server, follow, b"item"), 2);
        server.session_names.remove(b"b".as_slice());
        monitor_check(&mut server, fixed, &mut runtime).unwrap();
        assert_eq!(monitor_get_fire_count(&server, fixed, b"item"), 1);
        assert_eq!(
            server.monitors.get(fixed).unwrap().items[b"item".as_slice()]
                .session
                .last
                .as_deref(),
            Some(b"b".as_slice())
        );
        assert!(server.monitors.get(fixed).unwrap().timer_pending);
    }
    #[test]
    fn wildcard_dead_panes_generations_and_missing_specific_cache() {
        use super::super::{pane, session, window};
        let mut server = Server::new();
        let options = server.options.create(None);
        let session = session::session_create(
            &mut server,
            session::SessionCreate {
                prefix: None,
                name: Some(b"s".to_vec()),
                cwd: b"/".to_vec(),
                environment: crate::options::environment::Environment::default(),
                options,
                termios: None,
            },
        );
        let window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = pane::pane_create(&mut server, window, 80, 24, 0).unwrap();
        server.windows.get_mut(window).unwrap().panes.push(pane);
        let public_pane = server.panes.get(pane).unwrap().public_id;
        let public_window = server.windows.get(window).unwrap().public_id;
        session::session_attach(&mut server, session, window, 1).unwrap();
        session::session_attach(&mut server, session, window, 2).unwrap();
        let id = monitor_create_session(&mut server, Some(session), inert).unwrap();
        assert_eq!(server.sessions.get(session).unwrap().references, 2);
        for text in [
            "p:%*:f".to_owned(),
            "w:@*:f".to_owned(),
            format!("specific:@{public_window}:f"),
            format!("dead:%{public_pane}:f"),
        ] {
            let mut spec = monitor_parse(text.as_bytes()).unwrap();
            spec.flags = MonitorFlags::INITIAL;
            monitor_add(&mut server, id, spec, &mut Runtime).unwrap();
        }
        server.monitors.get_mut(id).unwrap().generation = u32::MAX;
        monitor_check(&mut server, id, &mut Runtime).unwrap();
        assert_eq!(server.monitors.get(id).unwrap().generation, 2);
        assert_eq!(monitor_get_fire_count(&server, id, b"p"), 2);
        assert_eq!(monitor_get_fire_count(&server, id, b"w"), 2);
        assert_eq!(monitor_get_fire_count(&server, id, b"specific"), 2);
        assert_eq!(monitor_get_fire_count(&server, id, b"dead"), 0);
        server.window_ids.remove(&public_window);
        server.windows.get_mut(window).unwrap().panes.clear();
        monitor_check(&mut server, id, &mut Runtime).unwrap();
        let set = server.monitors.get(id).unwrap();
        assert!(set.items[b"p".as_slice()].targets.is_empty());
        assert_eq!(set.items[b"specific".as_slice()].targets.len(), 2);
        assert_eq!(set.items[b"w".as_slice()].targets.len(), 2);
        monitor_destroy(&mut server, id, &mut Runtime).unwrap();
        assert_eq!(server.sessions.get(session).unwrap().references, 1);
    }
    #[test]
    fn cache_identity_true_only_and_report_clock() {
        let mut server = Server::new();
        let mut runtime = Runtime;
        let id = monitor_create_session(&mut server, None, inert).unwrap();
        let mut spec = monitor_parse(b"item:%*:f").unwrap();
        spec.flags = MonitorFlags::INITIAL | MonitorFlags::TRUE;
        monitor_add(&mut server, id, spec, &mut runtime).unwrap();
        let context = MonitorContext::default();
        server.current_time = (12, 0);
        check_value(
            &mut server,
            id,
            b"item",
            0,
            Some((7, 1)),
            1,
            context,
            b"0".to_vec(),
        );
        assert_eq!(monitor_get_fire_count(&server, id, b"item"), 0);
        check_value(
            &mut server,
            id,
            b"item",
            0,
            Some((7, 1)),
            1,
            context,
            b"1".to_vec(),
        );
        assert_eq!(monitor_get_fire_count(&server, id, b"item"), 1);
        check_value(
            &mut server,
            id,
            b"item",
            0,
            Some((7, 1)),
            1,
            context,
            b"1".to_vec(),
        );
        server.current_time = (13, 0);
        check_value(
            &mut server,
            id,
            b"item",
            0,
            Some((7, 1)),
            1,
            context,
            b"0".to_vec(),
        );
        assert_eq!(monitor_get_fire_time(&server, id, b"item"), 12);
        check_value(
            &mut server,
            id,
            b"item",
            0,
            Some((7, 2)),
            1,
            context,
            b"000".to_vec(),
        );
        assert_eq!(monitor_get_fire_count(&server, id, b"item"), 2);
        let item = server
            .monitors
            .get(id)
            .unwrap()
            .items
            .get(b"item".as_slice())
            .unwrap();
        assert_eq!(item.targets.len(), 2);
        assert_eq!(item.targets[&(7, 1)].last.as_deref(), Some(b"0".as_slice()));
        monitor_remove(&mut server, id, b"item", &mut runtime).unwrap();
        assert_eq!(monitor_get_fire_count(&server, id, b"item"), 0);
        assert!(!server.monitors.get(id).unwrap().timer_pending);
    }
    fn replace_self(server: &mut Server, id: MonitorSetId, change: &MonitorChange<'_>) {
        assert_eq!(monitor_get_fire_count(server, id, change.name), 1);
        monitor_add(
            server,
            id,
            monitor_parse(b"item::new").unwrap(),
            &mut Runtime,
        )
        .unwrap();
    }
    fn destroy_self(server: &mut Server, id: MonitorSetId, _: &MonitorChange<'_>) {
        monitor_destroy(server, id, &mut Runtime).unwrap();
    }
    #[test]
    fn callback_replacement_and_destruction_do_not_restore_cache() {
        let mut server = Server::new();
        let id = monitor_create_session(&mut server, None, replace_self).unwrap();
        let mut spec = monitor_parse(b"item::f").unwrap();
        spec.flags = MonitorFlags::INITIAL;
        monitor_add(&mut server, id, spec.clone(), &mut Runtime).unwrap();
        check_value(
            &mut server,
            id,
            b"item",
            0,
            None,
            0,
            MonitorContext::default(),
            b"old".to_vec(),
        );
        assert_eq!(monitor_get_fire_count(&server, id, b"item"), 0);
        assert!(
            server.monitors.get(id).unwrap().items[b"item".as_slice()]
                .session
                .last
                .is_none()
        );
        check_value(
            &mut server,
            id,
            b"item",
            0,
            None,
            0,
            MonitorContext::default(),
            b"stale".to_vec(),
        );
        assert!(
            server.monitors.get(id).unwrap().items[b"item".as_slice()]
                .session
                .last
                .is_none()
        );
        let id = monitor_create_session(&mut server, None, destroy_self).unwrap();
        monitor_add(&mut server, id, spec, &mut Runtime).unwrap();
        server.monitors.retain(id).unwrap();
        check_value(
            &mut server,
            id,
            b"item",
            0,
            None,
            0,
            MonitorContext::default(),
            b"old".to_vec(),
        );
        assert!(server.monitors.get(id).unwrap().items.is_empty());
        server.monitors.release(id).unwrap();
        assert!(server.monitors.get(id).is_none());
    }
    #[test]
    fn parser_matches_pinned_c() {
        use std::process::Command;
        let source = Command::new("git")
            .args(["-C", "/Users/j/fun/tmux", "show", "8f25579c:monitor.c"])
            .output();
        let Ok(source) = source else {
            eprintln!("skipping monitor C reference: pinned source unavailable");
            return;
        };
        if !source.status.success() {
            eprintln!("skipping monitor C reference: pinned source unavailable");
            return;
        }
        let source = String::from_utf8(source.stdout).unwrap();
        let start = source.find("int\nmonitor_parse(").unwrap();
        let end = source[start..].find("\n/* Add a subscription. */").unwrap() + start;
        let harness = format!(
            "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n#define xstrdup strdup\nenum monitor_type {{ MONITOR_SESSION, MONITOR_PANE, MONITOR_ALL_PANES, MONITOR_WINDOW, MONITOR_ALL_WINDOWS }};\n{}\nint main(int argc,char **argv) {{ for(int i=1;i<argc;i++) {{ char *name,*fmt; enum monitor_type type; int id; int r=monitor_parse(argv[i],&name,&type,&id,&fmt); if(r) puts(\"err\"); else {{ printf(\"%d %d\\n\",type,id); free(name);free(fmt); }} }} }}\n",
            &source[start..end]
        );
        let dir = std::env::temp_dir().join(format!("rmux-monitor-cref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.c"), harness).unwrap();
        let build = Command::new("cc")
            .arg(dir.join("probe.c"))
            .arg("-o")
            .arg(dir.join("probe"))
            .output()
            .expect("C compiler required with pinned source");
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let fixtures = [
            "::",
            "n:%*:x",
            "n:@*:x",
            "n:%+12tail:a:b",
            "n:@ 2junk:f",
            "n:%-0:f",
            "n:%-1:f",
            "n:%2147483647:f",
            "n:bad:f",
            "n:%*:more:colons",
            "missing",
        ];
        let output = Command::new(dir.join("probe"))
            .args(fixtures)
            .output()
            .unwrap();
        assert!(output.status.success());
        for (fixture, expected) in fixtures
            .iter()
            .zip(String::from_utf8(output.stdout).unwrap().lines())
        {
            let actual = monitor_parse(fixture.as_bytes())
                .map(|s| format!("{} {}", s.kind as i32, s.target.map_or(-1, |n| n as i32)))
                .unwrap_or_else(|_| "err".to_owned());
            assert_eq!(actual, expected, "{fixture}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn permissive_parser() {
        let spec = monitor_parse(b":% +12suffix:a:b:c").unwrap();
        assert!(spec.name.is_empty());
        assert_eq!(spec.target, Some(12));
        assert_eq!(spec.format, b"a:b:c");
        assert_eq!(monitor_parse(b"x::").unwrap().kind, MonitorType::Session);
        assert_eq!(
            monitor_parse(b"x:%*:f").unwrap().kind,
            MonitorType::AllPanes
        );
        assert_eq!(
            monitor_parse(b"x:@*:f").unwrap().kind,
            MonitorType::AllWindows
        );
        for invalid in [
            b"x".as_slice(),
            b"x:y",
            b"x:%-1:f",
            b"x:@:f",
            b"x:%*:missing:ok\0",
        ] {
            if invalid == b"x:%*:missing:ok\0" {
                assert!(monitor_parse(invalid).is_ok());
            } else {
                assert!(monitor_parse(invalid).is_err());
            }
        }
    }
    #[test]
    fn true_only_exact_zero() {
        for value in [b"".as_slice(), b"0"] {
            assert!(!true_value(value));
        }
        for value in [b"1".as_slice(), b"000", b"01", b"false", b"-0"] {
            assert!(true_value(value));
        }
    }
}
