// Ported from tmux format.c @ 8f25579c
/*
 * Copyright (c) 2011 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
 * OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
 * CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Instant;

use super::jobs::FormatJobRuntime;
use super::sort::{self, SortClients, SortCriteria, SortOrder};
use super::{FormatContext, FormatFlags, FormatKind, FormatTagFlags, FormatTree, FormatValue};
use crate::cmd::arguments::ArgumentFormatRuntime;
use crate::cmd::find::CmdFindState;
use crate::cmd::parse::CmdParseInput;
use crate::ids::{ClientId, OptionsId, QueueItemId, SessionId};
use crate::model::Server;
use crate::options::OptionsArrayKey;
use crate::options::environment::{Environment, EnvironmentFlags};
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::time::Timestamp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionScope {
    Server,
    Pane,
    Window,
    GlobalWindow,
    Session,
    GlobalSession,
}

#[derive(Clone, Debug, Default)]
pub struct FormatLoopEntry {
    pub context: FormatContext,
    pub tag: u32,
    pub active: bool,
    pub fields: BTreeMap<ByteString, ByteString>,
}

/// Explicit actions retained for generic runtime adapters and event dispatch.
#[derive(Clone, Debug)]
pub enum FormatAction {
    RetainClient(ClientId),
    ReleaseClient(ClientId),
    Cycle(ClientId),
    Job {
        owner: Option<ClientId>,
        tag: u32,
        flags: FormatFlags,
        raw: ByteString,
        expanded: ByteString,
        now: i64,
    },
    Log {
        item: Option<QueueItemId>,
        depth: u32,
        message: ByteString,
        verbose: bool,
    },
    ParsePrint {
        message: ByteString,
        input: CmdParseInput,
    },
}

/// Object-safe expansion boundary. Default methods represent absent context,
/// not model/client facts; model-backed expansion uses Server or its adapter.
pub trait FormatRuntime {
    fn defaults(&mut self, context: FormatContext) -> FormatContext {
        context
    }
    fn add_mode_formats(&mut self, _: &mut FormatTree) {}
    fn option(&mut self, _: &FormatContext, _: OptionScope, _: &[u8]) -> Option<ByteString> {
        None
    }
    fn builtin(&mut self, _: &FormatContext, _: &[u8]) -> Option<FormatValue> {
        None
    }
    fn builtin_owned(
        &mut self,
        context: &FormatContext,
        _: Option<ClientId>,
        key: &[u8],
    ) -> Option<FormatValue> {
        self.builtin(context, key)
    }
    fn environment(&mut self, _: &FormatContext, _: bool, _: &[u8]) -> Option<ByteString> {
        None
    }
    fn loop_entries(
        &mut self,
        _: &FormatContext,
        _: Option<ClientId>,
        _: u8,
        _: &[u8],
    ) -> Option<Vec<FormatLoopEntry>> {
        None
    }
    fn search(&mut self, _: &FormatContext, _: &[u8], _: &[u8]) -> u32 {
        0
    }
    fn name_exists(&mut self, _: &FormatContext, _: bool, _: &[u8]) -> bool {
        false
    }
    fn client_query(&mut self, _: &FormatContext, _: u8, _: &[u8]) -> Option<ByteString> {
        None
    }
    fn job(
        &mut self,
        _: Option<ClientId>,
        _: u32,
        _: FormatFlags,
        _: &[u8],
        _: &[u8],
        _: i64,
    ) -> ByteString {
        ByteString::default()
    }
    fn cycle(&mut self, _: ClientId) {}
    fn queue_formats(&self, _: QueueItemId) -> BTreeMap<ByteString, ByteString> {
        BTreeMap::new()
    }
    fn mouse(&self, _: QueueItemId) -> Option<crate::cmd::find::MouseInput> {
        None
    }
    fn target(&self, _: QueueItemId) -> FormatContext {
        FormatContext::default()
    }
    fn owner_client(&self, _: QueueItemId) -> Option<ClientId> {
        None
    }
    fn retain_client(&mut self, _: ClientId) {}
    fn release_client(&mut self, _: ClientId) {}
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
    fn monotonic_ms(&self) -> u64 {
        static START: LazyLock<Instant> = LazyLock::new(Instant::now);
        u64::try_from(START.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn log(&mut self, _: Option<QueueItemId>, _: u32, _: &[u8], _: bool) {}
}

fn option_id(server: &Server, context: &FormatContext, scope: OptionScope) -> Option<OptionsId> {
    match scope {
        OptionScope::Server => Some(server.options.global),
        OptionScope::Pane => Some(server.panes.get(context.pane?)?.options),
        OptionScope::Window => Some(server.windows.get(context.window?)?.options),
        OptionScope::GlobalWindow => Some(server.options.global_w),
        OptionScope::Session => Some(server.sessions.get(context.session?)?.options),
        OptionScope::GlobalSession => Some(server.options.global_s),
    }
}

impl FormatRuntime for Server {
    fn defaults(&mut self, mut context: FormatContext) -> FormatContext {
        if context.session.is_none() {
            context.session = context
                .evaluated_client
                .and_then(|id| self.clients.get(id)?.session);
        }
        if context.winlink.is_none() {
            context.winlink = context
                .session
                .and_then(|id| self.sessions.get(id)?.current);
        }
        if context.window.is_none() {
            context.window = context
                .winlink
                .and_then(|id| self.winlinks.get(id).map(|wl| wl.window))
                .or_else(|| {
                    context
                        .pane
                        .and_then(|id| self.panes.get(id).map(|p| p.window))
                });
        }
        if context.pane.is_none() && context.winlink.is_some() {
            context.pane = context.window.and_then(|id| self.windows.get(id)?.active);
        }
        if context.buffer.is_none() {
            context.buffer = self.paste.top();
        }
        context
    }
    fn add_mode_formats(&mut self, tree: &mut FormatTree) {
        crate::server::format_live::add_mode_formats(self, tree);
    }
    fn option(
        &mut self,
        context: &FormatContext,
        scope: OptionScope,
        key: &[u8],
    ) -> Option<ByteString> {
        let (_, entry, key) =
            self.options
                .parse_get(option_id(self, context, scope)?, cstr(key), false)?;
        Some(entry.to_string(key.as_ref(), true))
    }
    fn builtin(&mut self, context: &FormatContext, key: &[u8]) -> Option<FormatValue> {
        super::variables::model_value(self, context, key)
            .or_else(|| crate::server::format_live::builtin(self, context, key))
    }
    fn builtin_owned(
        &mut self,
        context: &FormatContext,
        owner: Option<ClientId>,
        key: &[u8],
    ) -> Option<FormatValue> {
        if key == b"window_layout" || key == b"window_visible_layout" {
            let flags = match owner {
                Some(id) => self.clients.get(id)?.flags,
                None => crate::client::ClientFlags::default(),
            };
            return super::variables::model_layout_value(self, context, flags, key);
        }
        self.builtin(context, key)
    }
    fn environment(
        &mut self,
        context: &FormatContext,
        global: bool,
        key: &[u8],
    ) -> Option<ByteString> {
        let env = if global {
            &self.global_environment
        } else {
            &self.sessions.get(context.session?)?.environment
        };
        env.find(cstr(key))?.value.clone()
    }
    fn loop_entries(
        &mut self,
        context: &FormatContext,
        owner: Option<ClientId>,
        kind: u8,
        flags: &[u8],
    ) -> Option<Vec<FormatLoopEntry>> {
        let flags = cstr(flags);
        if kind == b'L' {
            let mut ids = Vec::new();
            sort::get_clients(self, &criteria(kind, flags), &mut ids);
            return Some(
                ids.into_iter()
                    .map(|id| FormatLoopEntry {
                        context: FormatContext {
                            evaluated_client: Some(id),
                            buffer: None,
                            ..*context
                        },
                        ..FormatLoopEntry::default()
                    })
                    .collect(),
            );
        }
        if kind == b'V' && flags == b"c" {
            return Some(
                owner
                    .and_then(|id| self.clients.get(id))
                    .map_or_else(Vec::new, |client| {
                        environment_loop(context, &client.environ)
                    }),
            );
        }
        let mut entries = model_loop(self, context, kind, flags)?;
        if kind == b'S' {
            let active = context
                .evaluated_client
                .and_then(|id| self.clients.get(id)?.session);
            for entry in &mut entries {
                entry.active = active.is_some() && entry.context.session == active;
            }
        }
        Some(entries)
    }
    fn search(&mut self, context: &FormatContext, text: &[u8], flags: &[u8]) -> u32 {
        let Some(pane) = context.pane else {
            return 0;
        };
        crate::model::pane::pane_search(
            self,
            pane,
            cstr(text),
            flags.contains(&b'r'),
            flags.contains(&b'i'),
        )
    }
    fn name_exists(&mut self, context: &FormatContext, session: bool, name: &[u8]) -> bool {
        let name = cstr(name);
        if session {
            self.session_names.contains_key(name)
        } else {
            context
                .session
                .and_then(|id| self.sessions.get(id))
                .is_some_and(|s| {
                    s.windows.values().any(|id| {
                        self.winlinks
                            .get(*id)
                            .and_then(|wl| self.windows.get(wl.window))
                            .is_some_and(|w| w.name == name)
                    })
                })
        }
    }
    fn client_query(
        &mut self,
        context: &FormatContext,
        kind: u8,
        key: &[u8],
    ) -> Option<ByteString> {
        crate::server::format_live::client_query(self, context, kind, key)
    }
    fn queue_formats(&self, item: QueueItemId) -> BTreeMap<ByteString, ByteString> {
        let mut formats = BTreeMap::new();
        self.queue
            .merge_formats(item, &mut formats)
            .expect("live format queue item");
        formats
    }
    fn mouse(&self, item: QueueItemId) -> Option<crate::cmd::find::MouseInput> {
        let item = self.queue.items.get(item)?;
        Some(self.queue.states.get(item.state)?.event.mouse)
    }
    fn target(&self, item: QueueItemId) -> FormatContext {
        let target = self.queue.items.get(item).expect("live format queue item");
        FormatContext {
            evaluated_client: target.target_client,
            session: target.target.s,
            winlink: target.target.wl,
            window: target.target.w,
            pane: target.target.wp,
            mouse: self.mouse(item),
            ..FormatContext::default()
        }
    }
    fn owner_client(&self, item: QueueItemId) -> Option<ClientId> {
        self.queue.items.get(item)?.client
    }
    fn job(
        &mut self,
        owner: Option<ClientId>,
        tag: u32,
        flags: FormatFlags,
        raw: &[u8],
        expanded: &[u8],
        now: i64,
    ) -> ByteString {
        crate::server::format_live::job(self, owner, tag, flags, raw, expanded, now)
    }
    fn cycle(&mut self, owner: ClientId) {
        crate::server::format_live::cycle(self, owner);
    }
    fn retain_client(&mut self, client: ClientId) {
        crate::client::lifecycle::retain(self, client).expect("live format owner");
    }
    fn release_client(&mut self, client: ClientId) {
        crate::client::lifecycle::release(self, client).expect("retained format owner");
    }
    fn now(&self) -> Timestamp {
        Timestamp::new(self.current_time.0, self.current_time.1 as i32)
    }
    fn log(&mut self, item: Option<QueueItemId>, depth: u32, message: &[u8], verbose: bool) {
        crate::server::format_live::log(self, item, depth, message, verbose);
    }
}

/// G14/G15 own live clients, queue items, terminal queries, process handles,
/// leases and logging. builtin computes typed client/server/mouse facts through
/// variables::{client_value,server_value,mouse_value}; never invent defaults.
pub trait FormatExternal: FormatJobRuntime + SortClients {
    fn client_session(&self, client: ClientId) -> Option<SessionId>;
    fn client_environment(&self, client: ClientId) -> Option<&Environment>;
    fn client_facts(&self, client: ClientId) -> Option<super::variables::ClientFacts<'_>>;
    fn server_facts(&self) -> super::variables::ServerFacts<'_>;
    fn mouse_facts(&self, context: &FormatContext) -> Option<super::variables::MouseFacts<'_>>;
    fn client_links(&self) -> &[super::variables::ClientLinkFacts<'_>];
    fn pane_aux_facts(
        &self,
        pane: crate::ids::PaneId,
    ) -> Option<super::variables::PaneAuxFacts<'_>>;
    fn client_query(&mut self, context: &FormatContext, kind: u8, key: &[u8])
    -> Option<ByteString>;
    fn queue_formats(&self, item: QueueItemId) -> BTreeMap<ByteString, ByteString>;
    fn mouse(&self, _: QueueItemId) -> Option<crate::cmd::find::MouseInput> {
        None
    }
    fn target(&self, item: QueueItemId) -> FormatContext;
    fn owner_client(&self, item: QueueItemId) -> Option<ClientId>;
    fn retain_client(&mut self, client: ClientId);
    fn release_client(&mut self, client: ClientId);
    fn log(&mut self, item: Option<QueueItemId>, depth: u32, message: &[u8], verbose: bool);
    fn parse_print(&mut self, message: &[u8], input: &CmdParseInput);
}

/// Dispatch after expansion, with the real client/process transport installed.
pub fn apply_action<E: FormatExternal>(
    server: &mut Server,
    external: &mut E,
    action: FormatAction,
) -> Option<ByteString> {
    match action {
        FormatAction::RetainClient(client) => external.retain_client(client),
        FormatAction::ReleaseClient(client) => external.release_client(client),
        FormatAction::Cycle(client) => {
            server.format_jobs.cycle_start(external, client);
        }
        FormatAction::Job {
            owner,
            tag,
            flags,
            raw,
            expanded,
            now,
        } => {
            return Some(
                server
                    .format_jobs
                    .get(external, owner, tag, flags, &raw, &expanded, now),
            );
        }
        FormatAction::Log {
            item,
            depth,
            message,
            verbose,
        } => external.log(item, depth, &message, verbose),
        FormatAction::ParsePrint { message, input } => external.parse_print(&message, &input),
    }
    None
}

pub fn tidy_jobs<E: FormatExternal>(server: &mut Server, external: &mut E) {
    let mut clients = Vec::new();
    external.clients(&mut clients);
    let mut jobs = std::mem::take(&mut server.format_jobs);
    jobs.tidy_owner(external, None, server.current_time.0, false);
    for client in clients {
        jobs.tidy_owner(external, Some(client), server.current_time.0, false);
    }
    server.format_jobs = jobs;
}

pub fn lost_client<E: FormatExternal>(server: &mut Server, external: &mut E, owner: ClientId) {
    let mut jobs = std::mem::take(&mut server.format_jobs);
    jobs.lost_client(external, owner);
    server.format_jobs = jobs;
}

pub struct ServerFormatRuntime<'a, E> {
    pub server: &'a mut Server,
    pub external: &'a mut E,
}
impl<E: FormatExternal> FormatRuntime for ServerFormatRuntime<'_, E> {
    fn defaults(&mut self, mut context: FormatContext) -> FormatContext {
        if context.session.is_none() {
            context.session = context
                .evaluated_client
                .and_then(|c| self.external.client_session(c));
        }
        self.server.defaults(context)
    }
    fn add_mode_formats(&mut self, tree: &mut FormatTree) {
        self.server.add_mode_formats(tree);
    }
    fn option(&mut self, c: &FormatContext, s: OptionScope, k: &[u8]) -> Option<ByteString> {
        self.server.option(c, s, k)
    }
    fn builtin(&mut self, c: &FormatContext, k: &[u8]) -> Option<FormatValue> {
        super::variables::model_value(self.server, c, k)
            .or_else(|| crate::server::format_live::copy_value(self.server, c, k))
            .or_else(|| {
                c.evaluated_client
                    .and_then(|id| self.external.client_facts(id))
                    .and_then(|facts| super::variables::client_value(&facts, k))
            })
            .or_else(|| super::variables::server_value(&self.external.server_facts(), k))
            .or_else(|| {
                self.external
                    .mouse_facts(c)
                    .and_then(|facts| super::variables::mouse_value(&facts, k))
            })
            .or_else(|| {
                super::variables::client_model_value(
                    self.server,
                    c,
                    self.external.client_links(),
                    k,
                )
            })
            .or_else(|| {
                c.pane
                    .and_then(|id| self.external.pane_aux_facts(id))
                    .and_then(|facts| super::variables::pane_aux_value(&facts, k))
            })
    }
    fn builtin_owned(
        &mut self,
        context: &FormatContext,
        owner: Option<ClientId>,
        key: &[u8],
    ) -> Option<FormatValue> {
        if key == b"window_layout" || key == b"window_visible_layout" {
            let flags = match owner {
                Some(id) => self.external.client_facts(id)?.flags,
                None => crate::client::ClientFlags::default(),
            };
            return super::variables::model_layout_value(self.server, context, flags, key);
        }
        self.builtin(context, key)
    }
    fn environment(&mut self, c: &FormatContext, g: bool, k: &[u8]) -> Option<ByteString> {
        self.server.environment(c, g, k)
    }
    fn loop_entries(
        &mut self,
        context: &FormatContext,
        owner: Option<ClientId>,
        kind: u8,
        flags: &[u8],
    ) -> Option<Vec<FormatLoopEntry>> {
        let flags = cstr(flags);
        if kind == b'L' {
            let mut ids = Vec::new();
            sort::get_clients(self.external, &criteria(kind, flags), &mut ids);
            return Some(
                ids.into_iter()
                    .map(|id| FormatLoopEntry {
                        context: FormatContext {
                            evaluated_client: Some(id),
                            buffer: None,
                            ..*context
                        },
                        ..FormatLoopEntry::default()
                    })
                    .collect(),
            );
        }
        if kind == b'V' && flags == b"c" {
            return Some(
                owner
                    .and_then(|id| self.external.client_environment(id))
                    .map_or_else(Vec::new, |env| environment_loop(context, env)),
            );
        }
        let mut entries = self.server.loop_entries(context, owner, kind, flags)?;
        if kind == b'S' {
            let active = context
                .evaluated_client
                .and_then(|id| self.external.client_session(id));
            for entry in &mut entries {
                entry.active = entry.context.session == active && active.is_some();
            }
        }
        Some(entries)
    }
    fn search(&mut self, c: &FormatContext, t: &[u8], f: &[u8]) -> u32 {
        self.server.search(c, t, f)
    }
    fn name_exists(&mut self, c: &FormatContext, s: bool, n: &[u8]) -> bool {
        self.server.name_exists(c, s, n)
    }
    fn client_query(&mut self, c: &FormatContext, k: u8, v: &[u8]) -> Option<ByteString> {
        self.external.client_query(c, k, v)
    }
    fn job(
        &mut self,
        owner: Option<ClientId>,
        tag: u32,
        flags: FormatFlags,
        raw: &[u8],
        expanded: &[u8],
        now: i64,
    ) -> ByteString {
        self.server
            .format_jobs
            .get(self.external, owner, tag, flags, raw, expanded, now)
    }
    fn cycle(&mut self, owner: ClientId) {
        self.server.format_jobs.cycle_start(self.external, owner);
    }
    fn queue_formats(&self, item: QueueItemId) -> BTreeMap<ByteString, ByteString> {
        self.external.queue_formats(item)
    }
    fn mouse(&self, item: QueueItemId) -> Option<crate::cmd::find::MouseInput> {
        self.external.mouse(item)
    }
    fn target(&self, item: QueueItemId) -> FormatContext {
        self.external.target(item)
    }
    fn owner_client(&self, item: QueueItemId) -> Option<ClientId> {
        self.external.owner_client(item)
    }
    fn retain_client(&mut self, c: ClientId) {
        self.external.retain_client(c);
    }
    fn release_client(&mut self, c: ClientId) {
        self.external.release_client(c);
    }
    fn now(&self) -> Timestamp {
        self.server.now()
    }
    fn log(&mut self, i: Option<QueueItemId>, d: u32, m: &[u8], v: bool) {
        self.external.log(i, d, m, v);
    }
}

impl<E: FormatExternal> ArgumentFormatRuntime for ServerFormatRuntime<'_, E> {
    fn expand_from_target(&mut self, item: QueueItemId, value: &[u8]) -> ByteString {
        super::single_from_target(self, item, value)
    }
}

impl crate::options::CommandParser for Server {
    fn parse_from_string(&mut self, value: &[u8]) -> crate::cmd::parse::CmdParseResult {
        crate::cmd::parse::from_string(self, value, &mut CmdParseInput::default())
    }
}
pub fn initialize_defaults(server: &mut Server) {
    let mut options = std::mem::take(&mut server.options);
    options.load_defaults(server);
    server.options = options;
}

pub fn create_from_state(
    runtime: &mut dyn FormatRuntime,
    item: Option<QueueItemId>,
    client: Option<ClientId>,
    state: &CmdFindState,
) -> FormatTree {
    super::create_defaults(
        runtime,
        item,
        FormatContext {
            evaluated_client: client,
            session: state.s,
            winlink: state.wl,
            window: state.w,
            pane: state.wp,
            ..FormatContext::default()
        },
    )
}
pub fn single_from_state(
    runtime: &mut dyn FormatRuntime,
    item: Option<QueueItemId>,
    client: Option<ClientId>,
    state: &CmdFindState,
    value: &[u8],
) -> ByteString {
    let mut tree = create_from_state(runtime, item, client, state);
    let output = tree.expand(runtime, value);
    tree.release(runtime);
    output
}
pub fn condition(runtime: &mut dyn FormatRuntime, value: &[u8], input: &CmdParseInput) -> bool {
    super::true_value(Some(&single_from_state(
        runtime,
        input.item,
        input.client,
        &input.target,
        value,
    )))
}
pub fn expand_hook(
    runtime: &mut dyn FormatRuntime,
    value: &[u8],
    target: &CmdFindState,
    client: Option<ClientId>,
    formats: &BTreeMap<ByteString, ByteString>,
) -> ByteString {
    let mut tree = create_from_state(runtime, None, client, target);
    for (key, value) in formats {
        tree.add(key, value.clone());
    }
    let output = tree.expand(runtime, value);
    tree.release(runtime);
    output
}

fn criteria(kind: u8, flags: &[u8]) -> SortCriteria {
    let order = if kind == b'P' {
        if flags.contains(&b'i') {
            SortOrder::Index
        } else if flags.contains(&b'z') {
            SortOrder::Z
        } else {
            SortOrder::Creation
        }
    } else if flags.contains(&b'i') {
        if kind == b'S' {
            SortOrder::Index
        } else {
            SortOrder::Order
        }
    } else if flags.contains(&b'n') {
        SortOrder::Name
    } else if flags.contains(&b't') {
        SortOrder::Activity
    } else if kind == b'S' {
        SortOrder::Index
    } else {
        SortOrder::Order
    };
    SortCriteria {
        order,
        reversed: flags.contains(&b'r'),
        order_seq: None,
    }
}

fn field(entry: &mut FormatLoopEntry, key: &[u8], value: impl Into<ByteString>) {
    entry.fields.insert(key.into(), value.into());
}
fn boolean(entry: &mut FormatLoopEntry, key: &[u8], value: bool) {
    field(
        entry,
        key,
        if value {
            b"1".as_slice()
        } else {
            b"0".as_slice()
        },
    );
}

fn model_loop(
    server: &Server,
    context: &FormatContext,
    kind: u8,
    flags: &[u8],
) -> Option<Vec<FormatLoopEntry>> {
    let mut entries = Vec::new();
    match kind {
        b'S' => {
            let mut ids = Vec::new();
            sort::get_sessions(server, &criteria(kind, flags), &mut ids);
            for id in ids {
                entries.push(FormatLoopEntry {
                    context: FormatContext {
                        evaluated_client: context.evaluated_client,
                        session: Some(id),
                        kind: FormatKind::Session,
                        ..FormatContext::default()
                    },
                    ..FormatLoopEntry::default()
                });
            }
        }
        b'W' => {
            let session = context.session?;
            let s = server.sessions.get(session)?;
            let mut ids = Vec::new();
            sort::get_winlinks_session(server, session, &criteria(kind, flags), &mut ids);
            for (index, id) in ids.iter().enumerate() {
                let wl = server.winlinks.get(*id)?;
                let window = server.windows.get(wl.window)?;
                let mut entry = FormatLoopEntry {
                    context: FormatContext {
                        evaluated_client: context.evaluated_client,
                        session: Some(session),
                        winlink: Some(*id),
                        window: Some(wl.window),
                        kind: FormatKind::Window,
                        ..FormatContext::default()
                    },
                    tag: FormatTagFlags::WINDOW.bits() | window.public_id,
                    active: s.current == Some(*id),
                    fields: BTreeMap::new(),
                };
                boolean(
                    &mut entry,
                    b"window_after_active",
                    index > 0 && s.current == Some(ids[index - 1]),
                );
                boolean(
                    &mut entry,
                    b"window_before_active",
                    ids.get(index + 1)
                        .copied()
                        .is_some_and(|id| s.current == Some(id)),
                );
                for (prefix, neighbor) in [
                    (b"next".as_slice(), ids.get(index + 1)),
                    (
                        b"prev".as_slice(),
                        index.checked_sub(1).and_then(|i| ids.get(i)),
                    ),
                ] {
                    if let Some(neighbor) = neighbor {
                        let wl = server.winlinks.get(*neighbor)?;
                        let mut key = prefix.to_vec();
                        key.extend_from_slice(b"_window_index");
                        field(&mut entry, &key, (wl.index as u32).to_string());
                        key.truncate(prefix.len());
                        key.extend_from_slice(b"_window_active");
                        boolean(&mut entry, &key, s.current == Some(*neighbor));
                        let w = server.windows.get(wl.window)?;
                        for option in server
                            .options
                            .entries(w.options)
                            .filter(|o| o.name().starts_with(b"@"))
                        {
                            key.truncate(prefix.len());
                            key.push(b'_');
                            key.extend_from_slice(option.name());
                            field(&mut entry, &key, option.to_string(None, true));
                        }
                    }
                }
                entries.push(entry);
            }
        }
        b'P' => {
            let window = context.window?;
            let w = server.windows.get(window)?;
            let mut ids = Vec::new();
            sort::get_panes_window(server, window, &criteria(kind, flags), &mut ids);
            for id in ids {
                let pane = server.panes.get(id)?;
                entries.push(FormatLoopEntry {
                    context: FormatContext {
                        pane: Some(id),
                        kind: FormatKind::Pane,
                        buffer: None,
                        ..*context
                    },
                    tag: FormatTagFlags::PANE.bits() | pane.public_id,
                    active: w.active == Some(id),
                    fields: BTreeMap::new(),
                });
            }
        }
        b'O' => {
            let scope = if flags.contains(&b'v') {
                Some(OptionScope::Server)
            } else if flags.contains(&b'w') {
                Some(if flags.contains(&b'g') {
                    OptionScope::GlobalWindow
                } else {
                    OptionScope::Window
                })
            } else if flags.is_empty() || flags.contains(&b's') {
                Some(if flags.contains(&b'g') {
                    OptionScope::GlobalSession
                } else {
                    OptionScope::Session
                })
            } else if flags.contains(&b'p') {
                (!flags.contains(&b'g')).then_some(OptionScope::Pane)
            } else {
                flags.contains(&b'g').then_some(OptionScope::GlobalSession)
            };
            if let Some(id) = scope.and_then(|scope| option_id(server, context, scope)) {
                for option in server.options.entries(id) {
                    let count = option.array_items().count();
                    let mut push = |key: Option<&OptionsArrayKey>, index: usize| {
                        let mut entry = FormatLoopEntry {
                            context: FormatContext {
                                buffer: None,
                                ..*context
                            },
                            ..FormatLoopEntry::default()
                        };
                        field(&mut entry, b"option_name", option.name());
                        field(&mut entry, b"option_value", option.to_string(key, false));
                        boolean(&mut entry, b"option_is_array", option.is_array());
                        let bytes = key.map(OptionsArrayKey::to_bytes).unwrap_or_default();
                        field(&mut entry, b"option_array_key", bytes.clone());
                        field(&mut entry, b"option_array_index", bytes);
                        boolean(
                            &mut entry,
                            b"option_array_first",
                            option.is_array() && index == 0,
                        );
                        boolean(
                            &mut entry,
                            b"option_array_last",
                            option.is_array() && (count == 0 || index + 1 == count),
                        );
                        field(&mut entry, b"option_array_count", count.to_string());
                        boolean(
                            &mut entry,
                            b"option_is_hook",
                            option.table_entry().is_some_and(|o| o.is_hook()),
                        );
                        boolean(
                            &mut entry,
                            b"option_is_user",
                            option.table_entry().is_none(),
                        );
                        entries.push(entry);
                    };
                    if count == 0 {
                        push(None, 0);
                    } else {
                        for (index, (key, _)) in option.array_items().enumerate() {
                            push(Some(key), index);
                        }
                    }
                }
            }
        }
        b'V' => {
            let env = if flags == b"g" {
                Some(&server.global_environment)
            } else if flags.is_empty() || flags == b"s" {
                context
                    .session
                    .and_then(|id| server.sessions.get(id).map(|s| &s.environment))
            } else {
                None
            };
            if let Some(env) = env {
                entries = environment_loop(context, env);
            }
        }
        b'L' => return None,
        _ => return None,
    }
    Some(entries)
}

fn environment_loop(context: &FormatContext, env: &Environment) -> Vec<FormatLoopEntry> {
    env.iter()
        .map(|(name, value)| {
            let mut entry = FormatLoopEntry {
                context: FormatContext {
                    buffer: None,
                    ..*context
                },
                ..FormatLoopEntry::default()
            };
            field(&mut entry, b"environ_name", name);
            field(
                &mut entry,
                b"environ_value",
                value.value.clone().unwrap_or_default(),
            );
            boolean(
                &mut entry,
                b"environ_hidden",
                value.flags.contains(EnvironmentFlags::HIDDEN),
            );
            boolean(&mut entry, b"environ_removed", value.value.is_none());
            entry
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::session::{SessionCreate, session_create};

    #[derive(Default)]
    struct Parser {
        group: u32,
    }
    impl crate::cmd::parse::ParseContext for Parser {
        fn environment(&self, _: &[u8]) -> Option<&[u8]> {
            None
        }
        fn put_environment(&mut self, _: &[u8], _: bool) {}
        fn alias(&self, _: &[u8]) -> Option<ByteString> {
            None
        }
        fn condition(&mut self, value: &[u8], _: &CmdParseInput) -> bool {
            super::super::true_value(Some(value))
        }
        fn home(&mut self, _: Option<&[u8]>) -> Option<ByteString> {
            None
        }
        fn next_group(&mut self) -> u32 {
            self.group = self.group.wrapping_add(1);
            self.group
        }
        fn print(&mut self, _: &[u8], _: &CmdParseInput) {}
    }
    impl crate::options::CommandParser for Parser {
        fn parse_from_string(&mut self, input: &[u8]) -> crate::cmd::parse::CmdParseResult {
            crate::cmd::parse::from_string(self, input, &mut CmdParseInput::default())
        }
    }

    fn session(server: &mut Server, name: &[u8]) -> SessionId {
        let options = server.options.create(Some(server.options.global_s));
        session_create(
            server,
            SessionCreate {
                prefix: None,
                name: Some(name.to_vec()),
                cwd: b"/".to_vec(),
                environment: Environment::new(),
                options,
                termios: None,
            },
        )
    }

    #[test]
    fn object_safe_state_condition_and_hook_formats() {
        let mut server = Server::new();
        let id = session(&mut server, b"fixture");
        let state = CmdFindState {
            s: Some(id),
            ..CmdFindState::default()
        };
        let runtime: &mut dyn FormatRuntime = &mut server;
        assert_eq!(
            &*single_from_state(
                runtime,
                None,
                None,
                &state,
                b"#{session_name}:#{session_format}"
            ),
            b"fixture:1"
        );
        let input = CmdParseInput {
            target: state,
            ..CmdParseInput::default()
        };
        assert!(condition(runtime, b"#{session_format}", &input));
        assert!(!condition(runtime, b"0", &input));
        let formats = BTreeMap::from([(ByteString::from("hook"), ByteString::from("value"))]);
        assert_eq!(
            &*expand_hook(runtime, b"#{hook}:#{session_name}", &state, None, &formats),
            b"value:fixture"
        );
    }

    #[test]
    fn session_snapshots_and_sort_flag_priority() {
        let mut server = Server::new();
        let z = session(&mut server, b"z");
        let a = session(&mut server, b"a");
        let c = FormatContext::default();
        let entries = server.loop_entries(&c, None, b'S', b"").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.context.session.unwrap())
                .collect::<Vec<_>>(),
            vec![z, a]
        );
        let entries = server.loop_entries(&c, None, b'S', b"nr").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.context.session.unwrap())
                .collect::<Vec<_>>(),
            vec![z, a]
        );
        let entries = server.loop_entries(&c, None, b'S', b"n").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.context.session.unwrap())
                .collect::<Vec<_>>(),
            vec![a, z]
        );
        assert_eq!(criteria(b'S', b"int").order, SortOrder::Index);
        assert_eq!(criteria(b'P', b"iz").order, SortOrder::Index);
        assert!(server.loop_entries(&c, None, b'W', b"").is_none());
        assert!(server.loop_entries(&c, None, b'P', b"").is_none());
    }

    #[test]
    fn environment_loops_include_hidden_and_removed_entries() {
        let mut server = Server::new();
        server
            .global_environment
            .set(b"hidden", EnvironmentFlags::HIDDEN, b"secret");
        server.global_environment.clear(b"removed");
        let c = FormatContext::default();
        let entries = server.loop_entries(&c, None, b'V', b"g").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(&*entries[0].fields[b"environ_hidden".as_slice()], b"1");
        assert_eq!(&*entries[1].fields[b"environ_removed".as_slice()], b"1");
        assert!(entries[1].fields[b"environ_value".as_slice()].is_empty());
        assert!(
            server
                .loop_entries(&c, None, b'V', b"gs")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn direct_options_do_not_walk_parents_and_lookup_does() {
        let mut server = Server::new();
        let id = session(&mut server, b"options");
        let global = server.options.global_s;
        let local = server.sessions.get(id).unwrap().options;
        let mut parser = Parser::default();
        server
            .options
            .set_string(global, b"@inherited", false, b"parent", &mut parser);
        server
            .options
            .set_string(local, b"@direct", false, b"child", &mut parser);
        let context = FormatContext {
            session: Some(id),
            ..FormatContext::default()
        };
        assert_eq!(
            &*server
                .option(&context, OptionScope::Session, b"@inherited\0ignored")
                .unwrap(),
            b"parent"
        );
        let entries = server.loop_entries(&context, None, b'O', b"s").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(&*entries[0].fields[b"option_name".as_slice()], b"@direct");
        assert_eq!(&*entries[0].fields[b"option_array_first".as_slice()], b"0");
        assert!(
            server
                .loop_entries(&context, None, b'O', b"gp")
                .unwrap()
                .is_empty()
        );
        let option = server.options.empty(
            local,
            crate::options::search(b"update-environment").unwrap(),
        );
        option
            .array_set(
                &OptionsArrayKey::Name("named".into()),
                Some(b"VALUE"),
                false,
                &mut parser,
            )
            .unwrap();
        option
            .array_set(
                &OptionsArrayKey::Index(2),
                Some(b"NUMERIC"),
                false,
                &mut parser,
            )
            .unwrap();
        let entries = server.loop_entries(&context, None, b'O', b"s").unwrap();
        let array: Vec<_> = entries
            .iter()
            .filter(|entry| &*entry.fields[b"option_name".as_slice()] == b"update-environment")
            .collect();
        assert_eq!(array.len(), 2);
        assert_eq!(&*array[0].fields[b"option_array_key".as_slice()], b"2");
        assert_eq!(&*array[1].fields[b"option_array_key".as_slice()], b"named");
        assert_eq!(&*array[0].fields[b"option_array_first".as_slice()], b"1");
        assert_eq!(&*array[1].fields[b"option_array_last".as_slice()], b"1");
        assert_eq!(&*array[1].fields[b"option_array_count".as_slice()], b"2");
        server
            .options
            .get_mut_only(local, b"update-environment")
            .unwrap()
            .array_clear();
        let entries = server.loop_entries(&context, None, b'O', b"s").unwrap();
        let empty = entries
            .iter()
            .find(|entry| &*entry.fields[b"option_name".as_slice()] == b"update-environment")
            .unwrap();
        assert_eq!(&*empty.fields[b"option_array_first".as_slice()], b"1");
        assert_eq!(&*empty.fields[b"option_array_last".as_slice()], b"1");
        assert_eq!(&*empty.fields[b"option_array_count".as_slice()], b"0");
    }

    #[test]
    fn window_neighbors_follow_selected_sequence_and_direct_options() {
        use crate::model::session::session_attach;
        use crate::model::window::{window_add_pane, window_create, window_set_active_pane};
        let mut server = Server::new();
        let session = session(&mut server, b"windows");
        let mut parser = Parser::default();
        let global = server.options.global_w;
        server
            .options
            .set_string(global, b"@inherited", false, b"absent", &mut parser);
        let mut links = Vec::new();
        for (index, name) in [(7, b"z".as_slice()), (2, b"a"), (4, b"m")] {
            let window = window_create(&mut server, 20, 5, 0, 0).unwrap();
            server.windows.get_mut(window).unwrap().name = name.to_vec();
            let options = server.windows.get(window).unwrap().options;
            server
                .options
                .set_string(options, b"@neighbor", false, name, &mut parser);
            links.push(session_attach(&mut server, session, window, index).unwrap());
        }
        server.sessions.get_mut(session).unwrap().current = Some(links[2]);
        let context = FormatContext {
            session: Some(session),
            ..FormatContext::default()
        };
        let entries = server.loop_entries(&context, None, b'W', b"n").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.context.winlink.unwrap())
                .collect::<Vec<_>>(),
            vec![links[1], links[2], links[0]]
        );
        assert_eq!(&*entries[0].fields[b"next_window_index".as_slice()], b"4");
        assert_eq!(
            &*entries[0].fields[b"window_before_active".as_slice()],
            b"1"
        );
        assert_eq!(&*entries[2].fields[b"window_after_active".as_slice()], b"1");
        assert_eq!(&*entries[1].fields[b"next_@neighbor".as_slice()], b"z");
        assert!(
            !entries[1]
                .fields
                .contains_key(b"next_@inherited".as_slice())
        );
        assert!(
            !entries[0]
                .fields
                .contains_key(b"prev_window_index".as_slice())
        );
        assert!(entries[1].active);
        let window = entries[1].context.window.unwrap();
        let first = window_add_pane(
            &mut server,
            window,
            None,
            10,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        let second = window_add_pane(
            &mut server,
            window,
            Some(first),
            10,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        window_set_active_pane(&mut server, window, second, false).unwrap();
        let context = FormatContext {
            window: Some(window),
            ..context
        };
        let panes = server.loop_entries(&context, None, b'P', b"r").unwrap();
        assert_eq!(panes[0].context.pane, Some(second));
        assert!(panes[0].active);
        assert_eq!(
            panes[0].tag,
            FormatTagFlags::PANE.bits() | server.panes.get(second).unwrap().public_id
        );
        let defaults = server.defaults(FormatContext {
            session: Some(session),
            kind: FormatKind::Session,
            ..FormatContext::default()
        });
        assert_eq!(defaults.kind, FormatKind::Session);
        assert_eq!(defaults.pane, Some(second));
    }

    struct External {
        clients: Vec<super::super::variables::ClientLinkFacts<'static>>,
        environment: Environment,
        leases: i32,
        launches: usize,
        cancellations: Vec<crate::ids::JobId>,
        lifecycle: Vec<&'static str>,
    }
    impl SortClients for External {
        fn clients(&self, out: &mut Vec<ClientId>) {
            out.extend(self.clients.iter().map(|c| c.id));
        }
        fn client_sortable(&self, _: ClientId) -> bool {
            true
        }
        fn client_name(&self, id: ClientId) -> &[u8] {
            self.clients.iter().find(|c| c.id == id).unwrap().name
        }
        fn client_size(&self, _: ClientId) -> (u32, u32) {
            (80, 24)
        }
        fn client_created(&self, _: ClientId) -> (i64, i64) {
            (1, 0)
        }
        fn client_activity(&self, _: ClientId) -> (i64, i64) {
            (1, 0)
        }
    }
    impl FormatJobRuntime for External {
        fn run(&mut self, _: super::super::jobs::FormatJobLaunch<'_>) -> Option<crate::ids::JobId> {
            use crate::ids::ArenaId;
            self.launches += 1;
            Some(crate::ids::JobId::from_parts(self.launches as u32, 1))
        }
        fn cancel(&mut self, job: crate::ids::JobId) {
            self.cancellations.push(job);
            self.lifecycle.push("job");
        }
        fn status(&mut self, _: ClientId) {}
        fn cycle_start(&mut self, _: super::super::jobs::FormatCycleToken, _: std::time::Duration) {
        }
        fn cycle_cancel(&mut self, _: super::super::jobs::FormatCycleToken) {
            self.lifecycle.push("cycle");
        }
        fn redraw_status(&mut self, _: ClientId) {}
    }
    impl FormatExternal for External {
        fn parse_print(&mut self, _: &[u8], _: &CmdParseInput) {}
        fn client_session(&self, id: ClientId) -> Option<SessionId> {
            self.clients.iter().find(|c| c.id == id)?.session
        }
        fn client_environment(&self, _: ClientId) -> Option<&Environment> {
            Some(&self.environment)
        }
        fn client_facts(&self, _: ClientId) -> Option<super::super::variables::ClientFacts<'_>> {
            None
        }
        fn server_facts(&self) -> super::super::variables::ServerFacts<'_> {
            super::super::variables::ServerFacts {
                hostname: b"fixture",
                socket: b"socket",
                config_files: &[],
                next_session_id: 1,
                sessions: 1,
                start: Timestamp::new(1, 0),
                uid: 501,
                user: Some(b"user"),
                version: b"fixture",
                buffer_mode_format: b"",
                client_mode_format: b"",
                tree_mode_format: b"",
            }
        }
        fn mouse_facts(
            &self,
            _: &FormatContext,
        ) -> Option<super::super::variables::MouseFacts<'_>> {
            None
        }
        fn client_links(&self) -> &[super::super::variables::ClientLinkFacts<'_>] {
            &self.clients
        }
        fn pane_aux_facts(
            &self,
            _: crate::ids::PaneId,
        ) -> Option<super::super::variables::PaneAuxFacts<'_>> {
            None
        }
        fn client_query(&mut self, _: &FormatContext, _: u8, _: &[u8]) -> Option<ByteString> {
            None
        }
        fn queue_formats(&self, _: QueueItemId) -> BTreeMap<ByteString, ByteString> {
            BTreeMap::from([("queue_field".into(), "queue_value".into())])
        }
        fn target(&self, _: QueueItemId) -> FormatContext {
            FormatContext {
                evaluated_client: Some(self.clients[0].id),
                ..FormatContext::default()
            }
        }
        fn owner_client(&self, _: QueueItemId) -> Option<ClientId> {
            Some(self.clients[1].id)
        }
        fn retain_client(&mut self, _: ClientId) {
            self.leases += 1;
        }
        fn release_client(&mut self, _: ClientId) {
            self.leases -= 1;
        }
        fn log(&mut self, _: Option<QueueItemId>, _: u32, _: &[u8], _: bool) {}
    }

    #[test]
    fn external_client_loops_owner_environment_jobs_and_argument_glue() {
        use crate::ids::ArenaId;
        let mut server = Server::new();
        let session = session(&mut server, b"attached");
        let evaluated = ClientId::from_parts(0, 1);
        let owner = ClientId::from_parts(1, 1);
        let mut external = External {
            clients: vec![
                super::super::variables::ClientLinkFacts {
                    id: evaluated,
                    session: Some(session),
                    name: b"z",
                },
                super::super::variables::ClientLinkFacts {
                    id: owner,
                    session: None,
                    name: b"a",
                },
            ],
            environment: Environment::new(),
            leases: 0,
            launches: 0,
            cancellations: Vec::new(),
            lifecycle: Vec::new(),
        };
        external
            .environment
            .set(b"OWNER", EnvironmentFlags::default(), b"value");
        let mut runtime = ServerFormatRuntime {
            server: &mut server,
            external: &mut external,
        };
        let context = runtime.defaults(FormatContext {
            evaluated_client: Some(evaluated),
            ..FormatContext::default()
        });
        assert_eq!(context.session, Some(session));
        assert_eq!(context.kind, FormatKind::Unknown);
        let sessions = runtime
            .loop_entries(&context, Some(owner), b'S', b"")
            .unwrap();
        assert!(sessions[0].active);
        let clients = runtime
            .loop_entries(&context, Some(owner), b'L', b"n")
            .unwrap();
        assert_eq!(clients[0].context.evaluated_client, Some(owner));
        let env = runtime
            .loop_entries(&context, Some(owner), b'V', b"c")
            .unwrap();
        assert_eq!(&*env[0].fields[b"environ_name".as_slice()], b"OWNER");
        let item = QueueItemId::from_parts(0, 1);
        assert_eq!(
            &*runtime.expand_from_target(item, b"#{session_name}:#{queue_field}"),
            b"attached:queue_value"
        );
        assert_eq!(runtime.external.leases, 0);
        assert!(
            runtime
                .job(
                    Some(owner),
                    7,
                    FormatFlags::NONE,
                    b"command",
                    b"expanded",
                    1
                )
                .is_empty()
        );
        assert_eq!(runtime.external.launches, 1);
        assert_eq!(runtime.server.format_jobs.len(), 1);
        runtime.job(None, 7, FormatFlags::NONE, b"global", b"expanded", 1);
        runtime.job(
            Some(evaluated),
            7,
            FormatFlags::NONE,
            b"evaluated",
            b"expanded",
            1,
        );
        runtime.server.current_time = (4000, 0);
        tidy_jobs(runtime.server, runtime.external);
        assert!(runtime.server.format_jobs.is_empty());
        assert_eq!(
            runtime.external.cancellations,
            vec![
                crate::ids::JobId::from_parts(2, 1),
                crate::ids::JobId::from_parts(3, 1),
                crate::ids::JobId::from_parts(1, 1)
            ]
        );
        runtime.external.lifecycle.clear();
        runtime.job(
            Some(owner),
            7,
            FormatFlags::NONE,
            b"command",
            b"expanded",
            4000,
        );
        runtime.cycle(owner);
        lost_client(runtime.server, runtime.external, owner);
        assert_eq!(runtime.external.lifecycle, vec!["cycle", "job"]);
        assert!(runtime.server.format_jobs.is_empty());
    }

    #[test]
    fn native_nojobs_does_not_launch_or_enqueue_effects() {
        let mut server = Server::new();
        assert!(
            server
                .job(None, 2, FormatFlags::NOJOBS, b"raw", b"expanded", 1)
                .is_empty()
        );
        assert!(server.format_jobs.is_empty());
        assert!(server.jobs.is_empty());
        assert!(server.effects.is_empty());
    }

    #[test]
    fn native_parser_uses_environment_aliases_formats_and_groups() {
        use crate::cmd::parse::ParseContext;
        use crate::options::CommandParser;
        let mut server = Server::new();
        ParseContext::put_environment(&mut server, b"NAME=value", true);
        assert_eq!(
            ParseContext::environment(&server, b"NAME"),
            Some(b"value".as_slice())
        );
        let id = session(&mut server, b"parser");
        let window = crate::model::window::window_create(&mut server, 20, 5, 0, 0).unwrap();
        let pane = crate::model::window::window_add_pane(
            &mut server,
            window,
            None,
            10,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        crate::model::window::window_set_active_pane(&mut server, window, pane, false).unwrap();
        let link = crate::model::session::session_attach(&mut server, id, window, 0).unwrap();
        let input = CmdParseInput {
            target: CmdFindState {
                s: Some(id),
                wl: Some(link),
                w: Some(window),
                wp: Some(pane),
                ..CmdFindState::default()
            },
            ..CmdParseInput::default()
        };
        assert!(ParseContext::condition(
            &mut server,
            b"#{pane_format}",
            &input
        ));
        let first = ParseContext::next_group(&mut server);
        assert_eq!(ParseContext::next_group(&mut server), first.wrapping_add(1));
        assert!(server.parse_from_string(b"display-message hello").is_ok());
        ParseContext::print(&mut server, b"output", &input);
        assert!(
            !server
                .effects
                .iter()
                .any(|effect| matches!(effect, crate::model::ModelEffect::Format(_)))
        );
    }
}
