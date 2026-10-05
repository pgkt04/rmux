// Ported from tmux hooks.c @ 8f25579c
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

use super::*;
use crate::cmd::find::{ClientView, PaneDirection, PaneView, SessionView, WindowView, WinlinkView};
use crate::cmd::parse::{CmdParseError, CmdParseResult};
use crate::ids::{ArenaId, WindowId};
use crate::options::OptionsTableFlags;
fn id<I: ArenaId>(n: u32) -> I {
    I::from_parts(n, 1)
}
#[derive(Default)]
struct Fixture {
    hooks: HooksStore,
    options: OptionsStore,
    actions: Vec<String>,
    inserted: Vec<(Option<QueueItemId>, bool, BTreeMap<ByteString, ByteString>)>,
    nohooks: bool,
    error: bool,
    initial: bool,
    sequence: u32,
    state_event: bool,
    state_formats: BTreeMap<ByteString, ByteString>,
}
impl ModelView for Fixture {
    fn session(&self, _: SessionId) -> Option<SessionView<'_>> {
        None
    }
    fn winlink(&self, _: WinlinkId) -> Option<WinlinkView> {
        None
    }
    fn window(&self, _: WindowId) -> Option<WindowView<'_>> {
        None
    }
    fn pane(&self, _: PaneId) -> Option<PaneView<'_>> {
        None
    }
    fn client(&self, _: ClientId) -> Option<ClientView<'_>> {
        None
    }
    fn sessions(&self, _: &mut dyn FnMut(SessionId)) {}
    fn windows(&self, _: &mut dyn FnMut(WindowId)) {}
    fn panes(&self, _: &mut dyn FnMut(PaneId)) {}
    fn clients(&self, _: &mut dyn FnMut(ClientId)) {}
    fn marked(&self) -> Option<CmdFindState> {
        None
    }
    fn adjacent_pane(&self, _: PaneId, _: PaneDirection) -> Option<PaneId> {
        None
    }
    fn pane_description(&self, _: WindowId, _: &[u8]) -> Option<PaneId> {
        None
    }
}
impl CommandParser for Fixture {
    fn parse_from_string(&mut self, text: &[u8]) -> CmdParseResult {
        if self.error || text == b"bad" {
            Err(CmdParseError::new("bad command".into()))
        } else {
            Ok(Rc::new(CommandList::default()))
        }
    }
}
impl HooksRuntime for Fixture {
    fn hooks(&self) -> &HooksStore {
        &self.hooks
    }
    fn hooks_mut(&mut self) -> &mut HooksStore {
        &mut self.hooks
    }
    fn options(&self) -> &OptionsStore {
        &self.options
    }
    fn options_mut(&mut self) -> &mut OptionsStore {
        &mut self.options
    }
    fn model(&self) -> &dyn ModelView {
        self
    }
    fn session_options(&self, _: SessionId) -> OptionsId {
        self.options.global_s
    }
    fn pane_options(&self, _: PaneId) -> OptionsId {
        self.options.global_w
    }
    fn window_options(&self, _: &CmdFindState) -> Option<OptionsId> {
        Some(self.options.global_w)
    }
    fn now(&self) -> Timestamp {
        Timestamp { sec: 123, usec: 0 }
    }
    fn item_flags(&self, _: QueueItemId) -> QueueStateFlags {
        if self.nohooks {
            QueueStateFlags::NOHOOKS
        } else {
            QueueStateFlags::default()
        }
    }
    fn item_event(&self, _: QueueItemId) -> QueueEvent {
        QueueEvent::default()
    }
    fn item_formats(&self, _: QueueItemId) -> BTreeMap<ByteString, ByteString> {
        BTreeMap::from([
            ("command".into(), "outer".into()),
            ("current_file".into(), "x.conf".into()),
            ("hook".into(), "stale".into()),
        ])
    }
    fn item_target(&self, _: QueueItemId) -> CmdFindState {
        CmdFindState::default()
    }
    fn item_client(&self, _: QueueItemId) -> Option<ClientId> {
        None
    }
    fn global_running(&self) -> Option<QueueItemId> {
        Some(id(9))
    }
    fn expand_hook(
        &mut self,
        value: &[u8],
        _: &CmdFindState,
        _: Option<ClientId>,
        _: &BTreeMap<ByteString, ByteString>,
    ) -> ByteString {
        value.into()
    }
    fn new_hook_state(
        &mut self,
        _: &CmdFindState,
        event: Option<&QueueEvent>,
        flags: QueueStateFlags,
        formats: &BTreeMap<ByteString, ByteString>,
    ) -> crate::ids::QueueStateId {
        assert_eq!(flags, QueueStateFlags::NOHOOKS);
        self.state_event = event.is_some();
        self.state_formats = formats.clone();
        id(1)
    }
    fn free_hook_state(&mut self, _: crate::ids::QueueStateId) {}
    fn insert_hook_commands(
        &mut self,
        after: Option<QueueItemId>,
        _: Rc<CommandList>,
        state: crate::ids::QueueStateId,
    ) -> QueueItemId {
        assert_eq!(state, id(1));
        self.sequence += 1;
        self.inserted
            .push((after, self.state_event, self.state_formats.clone()));
        id(self.sequence)
    }
    fn hook_parse_error(&mut self, _: Option<QueueItemId>, _: &[u8], debug: bool) {
        self.actions.push(format!("error:{debug}"));
    }
    fn add_hook_sink(&mut self, _: &[u8], _: Option<HooksMonitorId>) -> EventSinkId {
        self.actions.push("sink:add".into());
        id(1)
    }
    fn remove_hook_sink(&mut self, _: EventSinkId) {
        self.actions.push("sink:remove".into());
    }
    fn create_monitor_set(&mut self, _: Option<SessionId>, _: HooksMonitorId) -> MonitorSetId {
        self.actions.push("set:create".into());
        id(1)
    }
    fn destroy_monitor_set(&mut self, _: MonitorSetId) {
        self.actions.push("set:destroy".into());
    }
    fn add_model_monitor(
        &mut self,
        _: MonitorSetId,
        name: &[u8],
        _: MonitorType,
        _: i32,
        _: &[u8],
        _: MonitorFlags,
    ) {
        self.actions.push("monitor:add".into());
        let monitor = self
            .options
            .get_only(self.options.global_s, name)
            .unwrap()
            .monitor()
            .unwrap();
        assert!(self.hooks.monitors.get(monitor).unwrap().is_some());
        self.initial = true;
    }
    fn model_monitor_stats(&self, _: MonitorSetId, _: &[u8]) -> (u32, i64) {
        (4, 123)
    }
    fn ensure_monitor_option(&mut self, options: OptionsId, name: &[u8]) {
        let mut parser = Parser;
        self.options
            .set_string(options, name, false, b"", &mut parser);
    }
    fn fire_hook_event(&mut self, _: &[u8], payload: HookPayload) {
        self.actions.push(format!(
            "value:{} last:{}",
            payload.formats[b"value".as_slice()],
            payload.formats[b"last".as_slice()]
        ));
    }
    fn monitor_formats(&self, _: &MonitorChange<'_>) -> BTreeMap<ByteString, ByteString> {
        BTreeMap::new()
    }
}
struct Parser;
impl CommandParser for Parser {
    fn parse_from_string(&mut self, _: &[u8]) -> CmdParseResult {
        Ok(Rc::new(CommandList::default()))
    }
}
fn user(f: &mut Fixture, name: &[u8], value: &[u8]) {
    let global = f.options.global_s;
    f.options
        .set_string(global, name, false, value, &mut Parser);
}
#[test]
fn nohooks_explicit_run_and_parse_error_routes() {
    let mut f = Fixture::default();
    user(&mut f, b"@test", b"ok");
    f.nohooks = true;
    let item = id(9);
    let payload = HookPayload {
        item: Some(item),
        ..Default::default()
    };
    event(&mut f, b"@test", &payload);
    assert!(f.inserted.is_empty());
    run(&mut f, item, b"@test");
    assert_eq!(f.inserted.len(), 1);
    assert!(f.inserted[0].1);
    f.error = true;
    run(&mut f, item, b"@test");
    assert_eq!(f.actions.last().unwrap(), "error:true");
}
#[test]
fn events_prefix_formats_and_ignore_monitor_payload() {
    let mut f = Fixture::default();
    user(&mut f, b"@test", b"ok");
    let mut payload = HookPayload::default();
    payload.formats.insert("value".into(), "v".into());
    event(&mut f, b"@test", &payload);
    assert_eq!(f.inserted[0].2[b"hook_value".as_slice()], b"v");
    assert_eq!(f.inserted[0].2[b"hook".as_slice()], b"@test");
    payload.monitor = Some(id(1));
    event(&mut f, b"@test", &payload);
    assert_eq!(f.inserted.len(), 1);
}
#[test]
fn item_events_merge_outer_item_formats_before_hook_formats() {
    let mut f = Fixture::default();
    user(&mut f, b"@test", b"ok");
    let mut payload = HookPayload {
        item: Some(id(9)),
        ..Default::default()
    };
    payload.formats.insert("arguments".into(), "-a".into());
    event(&mut f, b"@test", &payload);
    let formats = &f.inserted[0].2;
    assert_eq!(formats[b"command".as_slice()], b"outer");
    assert_eq!(formats[b"current_file".as_slice()], b"x.conf");
    assert_eq!(formats[b"hook_arguments".as_slice()], b"-a");
    assert_eq!(formats[b"hook".as_slice()], b"@test");
    run(&mut f, id(9), b"@test");
    assert!(!f.inserted[1].2.contains_key(b"command".as_slice()));
}
#[test]
fn monitor_publication_replacement_and_cleanup_order() {
    let mut f = Fixture::default();
    let options = f.options.global_s;
    let target = CmdFindState::default();
    let spec = || MonitorSpec {
        options,
        name: b"@m",
        kind: MonitorType::Pane,
        public_id: 3,
        format: b"format",
        flags: MonitorFlags::INITIAL,
        target: &target,
        session: Some(id(1)),
    };
    let first = monitor_add(&mut f, spec()).unwrap();
    assert!(f.initial);
    assert_eq!(f.actions, vec!["set:create", "sink:add", "monitor:add"]);
    let monitor = f.hooks.monitors.get(first).unwrap().as_ref().unwrap();
    assert_eq!(monitor_to_string(b"@m", monitor), b"@m:%3:format");
    assert_eq!(monitor_get_fire_count(&f, first, b"@m"), 4);
    assert_eq!(monitor_get_fire_time(&f, first, b"@m"), 123);
    monitor_add(&mut f, spec()).unwrap();
    assert_eq!(
        &f.actions[3..],
        &[
            "sink:remove",
            "set:destroy",
            "set:create",
            "sink:add",
            "monitor:add"
        ]
    );
    assert!(f.hooks.monitors.get(first).is_none());
    let current = f
        .options
        .get_only(options, b"@m")
        .unwrap()
        .monitor()
        .unwrap();
    let change = MonitorChange {
        name: b"@m",
        winlink: None,
        pane: None,
        session: None,
        client: None,
        value: None,
        last: None,
    };
    monitor_change(&mut f, current, &change);
    assert_eq!(f.actions.last().unwrap(), "value: last:");
    monitor_event(
        &mut f,
        first,
        b"@m",
        &HookPayload {
            monitor: Some(current),
            ..Default::default()
        },
    );
    assert!(f.inserted.is_empty());
}
#[test]
fn event_registration_and_validity() {
    let mut f = Fixture::default();
    add_event(&mut f, b"@custom");
    add_event(&mut f, b"@custom");
    assert_eq!(f.hooks.events.len(), 1);
    assert!(valid_event_name(b"@any"));
    assert!(valid_event_name(b"after-new-session"));
    assert!(!valid_event_name(b"status"));
    build_events(&mut f);
    assert_eq!(
        f.hooks.events.len(),
        1 + crate::options::OPTIONS_TABLE
            .iter()
            .filter(|e| e.flags.contains(OptionsTableFlags::HOOK))
            .count()
    );
}
#[test]
fn array_order_expanded_errors_and_explicit_lookup() {
    let mut f = Fixture::default();
    let global = f.options.global_s;
    let entry = crate::options::search(b"after-new-session").unwrap();
    f.options.default(global, entry, &mut Parser);
    let hook = f
        .options
        .get_mut_only(global, b"after-new-session")
        .unwrap();
    hook.array_set(
        &crate::options::OptionsArrayKey::Index(0),
        Some(b"one"),
        false,
        &mut Parser,
    )
    .unwrap();
    hook.array_set(
        &crate::options::OptionsArrayKey::Index(1),
        Some(b"two"),
        false,
        &mut Parser,
    )
    .unwrap();
    insert(
        &mut f,
        Some(id(9)),
        b"after-new-session",
        &HookPayload::default(),
        None,
        false,
    );
    assert_eq!(
        f.inserted.iter().map(|v| v.0).collect::<Vec<_>>(),
        vec![Some(id(9)), Some(id(1))]
    );
    assert_eq!(
        f.options
            .get_only(global, b"after-new-session")
            .unwrap()
            .fire_count(),
        1
    );
    let local = f.options.create(Some(global));
    let before = f.inserted.len();
    insert(
        &mut f,
        None,
        b"after-new-session",
        &HookPayload::default(),
        Some(local),
        false,
    );
    assert_eq!(f.inserted.len(), before);
    let user = crate::options::search(b"@test");
    assert!(user.is_none());
    user_option_array(&mut f);
}
fn user_option_array(f: &mut Fixture) {
    static STRING_HOOK: crate::options::OptionsTableEntry = crate::options::OptionsTableEntry {
        name: b"test-hook",
        kind: crate::options::OptionsTableType::String,
        scope: crate::options::OptionsScope::SESSION,
        flags: OptionsTableFlags(OptionsTableFlags::ARRAY.0 | OptionsTableFlags::HOOK.0),
        minimum: 0,
        maximum: 0,
        choices: None,
        default_str: None,
        default_num: 0,
        default_arr: None,
        separator: None,
        pattern: None,
        text: b"",
        unit: None,
    };
    let options = f.options.global_s;
    f.options.empty(options, &STRING_HOOK);
    let entry = f.options.get_mut_only(options, b"test-hook").unwrap();
    entry
        .array_set(
            &crate::options::OptionsArrayKey::Index(0),
            Some(b"bad"),
            false,
            &mut Parser,
        )
        .unwrap();
    entry
        .array_set(
            &crate::options::OptionsArrayKey::Index(1),
            Some(b"ok"),
            false,
            &mut Parser,
        )
        .unwrap();
    let before = f.inserted.len();
    insert(
        f,
        Some(id(9)),
        b"test-hook",
        &HookPayload::default(),
        None,
        true,
    );
    assert_eq!(f.inserted.len(), before + 1);
    assert_eq!(f.actions.last().unwrap(), "error:false");
}
#[test]
fn option_value_monitor_unlink_order() {
    let mut f = Fixture::default();
    let options = f.options.global_s;
    let target = CmdFindState::default();
    let monitor = monitor_add(
        &mut f,
        MonitorSpec {
            options,
            name: b"@m",
            kind: MonitorType::Session,
            public_id: 0,
            format: b"x",
            flags: MonitorFlags::default(),
            target: &target,
            session: Some(id(1)),
        },
    )
    .unwrap();
    let token = f.options.prepare_removal(options, b"@m").unwrap();
    assert_eq!(token.monitor, Some(monitor));
    assert!(f.options.get_only(options, b"@m").is_some());
    monitor_free(&mut f, monitor);
    assert!(f.options.get_only(options, b"@m").is_some());
    f.options.finish_removal(token);
    assert!(f.options.get_only(options, b"@m").is_none());
    assert_eq!(&f.actions[3..], &["sink:remove", "set:destroy"]);
}

#[test]
fn monitor_change_target_priority_and_saved_flags() {
    use crate::cmd::find::tests::{Fixture as Model, lid, pid, sid};
    let model = Model::new();
    let saved = find::from_session(&model, sid(1), CmdFindFlags::QUIET);
    let mut change = MonitorChange {
        name: b"hook",
        winlink: Some(lid(0)),
        pane: Some(pid(2)),
        session: Some(sid(1)),
        client: None,
        value: None,
        last: None,
    };
    let state = monitor_target(&model, &change, &saved);
    assert_eq!(state.s, Some(sid(0)));
    assert_eq!(state.wp, Some(pid(2)));
    assert_eq!(state.idx, 2);
    change.pane = Some(pid(4));
    let state = monitor_target(&model, &change, &saved);
    assert_eq!(state.wp, Some(pid(0)));
    assert_eq!(state.idx, -1);
    change.winlink = None;
    assert_eq!(monitor_target(&model, &change, &saved).wp, Some(pid(4)));
    change.pane = None;
    assert_eq!(monitor_target(&model, &change, &saved).s, Some(sid(1)));
    change.session = None;
    let state = monitor_target(&model, &change, &saved);
    assert_eq!(state.s, saved.s);
    assert_eq!(state.flags, CmdFindFlags::default());
}
