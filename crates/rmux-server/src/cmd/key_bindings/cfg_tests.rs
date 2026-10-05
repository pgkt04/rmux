// Ported from tmux cfg.c @ 8f25579c
/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
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
use crate::cmd::cfg::CfgViewError;
use crate::cmd::key_bindings::test_support::Context;
use crate::ids::ArenaId;
use std::collections::{BTreeMap, BTreeSet};

fn client(n: u32) -> ClientId {
    ClientId::from_parts(n, 0)
}
fn session(n: u32) -> SessionId {
    SessionId::from_parts(n, 0)
}
fn item(n: u32) -> QueueItemId {
    QueueItemId::from_parts(n, 0)
}
fn pane() -> PaneId {
    PaneId::from_parts(0, 0)
}

#[derive(Default)]
struct Runtime {
    parser: Context,
    clients: Vec<ClientId>,
    dead: BTreeSet<ClientId>,
    control: BTreeSet<ClientId>,
    client_sessions: BTreeMap<ClientId, SessionId>,
    sessions: Vec<SessionId>,
    attached: BTreeSet<SessionId>,
    callbacks: Vec<(Option<ClientId>, CfgCallback, QueueItemId)>,
    continued: Vec<QueueItemId>,
    history: usize,
    effects: Vec<&'static str>,
    view: bool,
    entered: usize,
    view_unavailable: bool,
    view_append_failure: bool,
    fallback: Vec<(Option<ClientId>, ByteString)>,
    viewed: Vec<ByteString>,
    notifications: Vec<(ClientId, ByteString)>,
    printed: Vec<(QueueItemId, ByteString)>,
    items: BTreeMap<QueueItemId, Option<ClientId>>,
    state_formats: BTreeMap<ByteString, ByteString>,
    copied: Option<(QueueItemId, Option<CmdFindState>)>,
    new_states: usize,
    freed: usize,
    commands: Vec<Rc<CommandList>>,
    appended: usize,
    inserted: Option<QueueItemId>,
    next_item: u32,
}
impl Runtime {
    fn id(&mut self) -> QueueItemId {
        self.next_item += 1;
        item(self.next_item)
    }
}
impl ParseContext for Runtime {
    fn environment(&self, n: &[u8]) -> Option<&[u8]> {
        self.parser.environment(n)
    }
    fn put_environment(&mut self, a: &[u8], h: bool) {
        self.parser.put_environment(a, h);
    }
    fn alias(&self, n: &[u8]) -> Option<ByteString> {
        self.parser.alias(n)
    }
    fn condition(&mut self, f: &[u8], i: &CmdParseInput) -> bool {
        self.parser.condition(f, i)
    }
    fn home(&mut self, u: Option<&[u8]>) -> Option<ByteString> {
        self.parser.home(u)
    }
    fn next_group(&mut self) -> u32 {
        self.parser.next_group()
    }
    fn print(&mut self, m: &[u8], i: &CmdParseInput) {
        self.parser.print(m, i);
    }
}
impl CfgRuntime for Runtime {
    fn first_client(&self) -> Option<ClientId> {
        self.clients.first().copied()
    }
    fn client_dead(&self, c: ClientId) -> bool {
        self.dead.contains(&c)
    }
    fn client_control(&self, c: ClientId) -> bool {
        self.control.contains(&c)
    }
    fn client_session(&self, c: ClientId) -> Option<SessionId> {
        self.client_sessions.get(&c).copied()
    }
    fn item_client(&self, i: QueueItemId) -> Option<ClientId> {
        self.items.get(&i).copied().flatten()
    }
    fn first_session_by_name(&self) -> Option<SessionId> {
        self.sessions.first().copied()
    }
    fn session_attached(&self, s: SessionId) -> bool {
        self.attached.contains(&s)
    }
    fn session_active_pane(&self, _s: SessionId) -> PaneId {
        pane()
    }
    fn pane_top_is_view(&self, _p: PaneId) -> bool {
        self.view
    }
    fn enter_view_mode(&mut self, p: PaneId) -> Result<(), CfgViewError> {
        assert_eq!(p, pane());
        if self.view_unavailable {
            return Err(CfgViewError::Unavailable);
        }
        self.view = true;
        self.entered += 1;
        Ok(())
    }
    fn append_view_line(&mut self, p: PaneId, line: &[u8]) -> Result<(), CfgViewError> {
        assert_eq!(p, pane());
        if self.view_append_failure {
            return Err(CfgViewError::Model(crate::model::ModelError::StaleId));
        }
        self.effects.push("cause");
        self.viewed.push(line.into());
        Ok(())
    }
    fn print_cfg_fallback(&mut self, client: Option<ClientId>, cause: &[u8]) {
        self.fallback.push((client, cause.into()));
    }
    fn notify_config_error(&mut self, c: ClientId, cause: &[u8]) {
        self.notifications.push((c, cause.into()));
    }
    fn print_cfg_cause(&mut self, i: QueueItemId, cause: &[u8]) {
        self.printed.push((i, cause.into()));
    }
    fn load_prompt_history(&mut self) {
        self.effects.push("history");
        self.history += 1;
    }
    fn append_cfg_callback(&mut self, c: Option<ClientId>, callback: CfgCallback) -> QueueItemId {
        let id = self.id();
        self.callbacks.push((c, callback, id));
        self.items.insert(id, c);
        id
    }
    fn continue_cfg_item(&mut self, id: QueueItemId) {
        self.effects.push("continue");
        self.continued.push(id);
    }
    fn new_cfg_state(&mut self) -> QueueStateId {
        self.new_states += 1;
        self.state_formats.clear();
        QueueStateId::from_parts(1, 0)
    }
    fn copy_cfg_state(&mut self, id: QueueItemId, current: Option<&CmdFindState>) -> QueueStateId {
        self.copied = Some((id, current.copied()));
        self.state_formats.clear();
        QueueStateId::from_parts(2, 0)
    }
    fn add_cfg_format(&mut self, _s: QueueStateId, n: &[u8], v: &[u8]) {
        self.state_formats.insert(n.into(), v.into());
    }
    fn cfg_commands(&mut self, list: Rc<CommandList>, _s: QueueStateId) -> QueueBatch {
        let ids = (0..list.commands.len()).map(|_| self.id()).collect();
        self.commands.push(list);
        QueueBatch { items: ids }
    }
    fn free_cfg_state(&mut self, _s: QueueStateId) {
        self.freed += 1;
    }
    fn append_cfg_commands(&mut self, batch: QueueBatch) -> Option<QueueItemId> {
        self.appended += 1;
        batch.items.last().copied()
    }
    fn insert_cfg_commands(&mut self, id: QueueItemId, batch: QueueBatch) -> Option<QueueItemId> {
        self.inserted = Some(id);
        batch.items.last().copied()
    }
}

#[test]
fn spec_section_2_11_barrier_first_client_replacement_load_once_and_finish_order() {
    let mut cfg = CfgState::default();
    let mut runtime = Runtime {
        clients: vec![client(1), client(2)],
        ..Runtime::default()
    };
    cfg.start_cfg(&mut runtime);
    let initial = cfg.item.unwrap();
    assert_eq!(cfg.client, Some(client(1)));
    assert_eq!(
        runtime.callbacks,
        vec![
            (Some(client(1)), CfgCallback::ClientDone, initial),
            (None, CfgCallback::Done, item(2))
        ]
    );
    assert_eq!(cfg.client_done(&runtime, client(1)), CmdReturn::Wait);
    // G15 does not call start_cfg for a later client; only the initial client has a barrier.
    assert!(
        !runtime
            .callbacks
            .iter()
            .any(|(c, _, _)| *c == Some(client(2)))
    );
    cfg.client_lost(&mut runtime, client(2));
    assert_eq!(cfg.item, Some(initial));
    cfg.client_lost(&mut runtime, client(1));
    assert!(cfg.client.is_none());
    assert!(cfg.item.is_none());
    assert_eq!(runtime.continued, vec![initial]);
    runtime.clients.remove(0);
    runtime.dead.insert(client(1));
    assert_eq!(cfg.client_done(&runtime, client(1)), CmdReturn::Normal);
    cfg.start_cfg(&mut runtime);
    let replacement = cfg.item.unwrap();
    assert_eq!(cfg.client, Some(client(2)));
    assert_eq!(runtime.callbacks.len(), 3);
    assert_eq!(
        runtime.callbacks[2],
        (Some(client(2)), CfgCallback::ClientDone, replacement)
    );
    assert_eq!(cfg.done(&mut runtime), CmdReturn::Normal);
    assert!(cfg.finished);
    assert!(cfg.item.is_none());
    assert_eq!(runtime.continued, vec![initial, replacement]);
    assert_eq!(runtime.history, 1);
    assert_eq!(cfg.client_done(&runtime, client(2)), CmdReturn::Normal);
    cfg.done(&mut runtime);
    assert_eq!(runtime.history, 1);
}

#[test]
fn spec_section_2_11_client_lost_before_barrier_fire_and_while_waiting() {
    for fire_first in [false, true] {
        let mut cfg = CfgState::default();
        let mut runtime = Runtime {
            clients: vec![client(1)],
            ..Runtime::default()
        };
        cfg.start_cfg(&mut runtime);
        let barrier = cfg.item.unwrap();
        if fire_first {
            assert_eq!(cfg.client_done(&runtime, client(1)), CmdReturn::Wait);
        }
        cfg.client_lost(&mut runtime, client(1));
        assert_eq!(runtime.continued, vec![barrier]);
        runtime.dead.insert(client(1));
        assert_eq!(cfg.client_done(&runtime, client(1)), CmdReturn::Normal);
        cfg.done(&mut runtime);
        assert_eq!(runtime.continued, vec![barrier]);
    }
}

#[test]
fn spec_section_2_11_quiet_enoent_other_open_errors_and_parse_error_causes() {
    let mut cfg = CfgState::default();
    let mut runtime = Runtime::default();
    let path = std::path::PathBuf::from(format!(
        "/tmp/rmx-absent-cfg-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    assert_eq!(
        cfg.load_cfg(&mut runtime, &path, None, None, None, CmdParseFlags::QUIET),
        Ok(None)
    );
    assert!(cfg.causes.is_empty());
    assert_eq!(
        cfg.load_cfg(
            &mut runtime,
            &path,
            None,
            None,
            None,
            CmdParseFlags::default()
        ),
        Err(CfgLoadError)
    );
    let mut expected = ByteString::from(path.as_os_str().as_bytes());
    expected.extend_from_slice(b": ");
    expected.extend_from_slice(&rmux_sys::errno::strerror(2));
    assert_eq!(cfg.causes, vec![expected]);
    cfg.causes.clear();
    assert_eq!(
        cfg.load_cfg(
            &mut runtime,
            Path::new("/dev/null/not-a-file"),
            None,
            None,
            None,
            CmdParseFlags::QUIET
        ),
        Err(CfgLoadError)
    );
    assert_eq!(cfg.causes.len(), 1);
    cfg.causes.clear();
    assert_eq!(
        cfg.load_cfg_from_buffer(
            &mut runtime,
            b"unknown-g11-command",
            b"bad.conf",
            None,
            None,
            None,
            CmdParseFlags::default()
        ),
        Err(CfgLoadError)
    );
    assert!(cfg.causes[0].starts_with(b"bad.conf:1: unknown command:"));
    assert!(runtime.commands.is_empty());
}

#[test]
fn spec_section_2_11_parseonly_validates_without_queue_and_context_is_not_execution_current() {
    let mut cfg = CfgState::default();
    let mut runtime = Runtime::default();
    let current = CmdFindState {
        s: Some(session(9)),
        idx: 99,
        ..CmdFindState::default()
    };
    let flags = CmdParseFlags::PARSEONLY;
    assert_eq!(
        cfg.load_cfg_from_buffer(
            &mut runtime,
            b"display-message ok",
            b"parse.conf",
            Some(client(1)),
            Some(item(10)),
            Some(&current),
            flags
        ),
        Ok(None)
    );
    assert!(runtime.commands.is_empty());
    assert_eq!(runtime.new_states, 0);
    assert!(runtime.copied.is_none());
    assert_eq!(
        cfg.load_cfg_from_buffer(
            &mut runtime,
            b"unknown-g11-command",
            b"parse.conf",
            None,
            None,
            None,
            flags
        ),
        Err(CfgLoadError)
    );
    runtime
        .state_formats
        .insert("extra_format".into(), "must-drop".into());
    let result = cfg
        .load_cfg_from_buffer(
            &mut runtime,
            b"%if #{condition}\ndisplay-message first\ndisplay-message second\n%endif\n",
            b"exec.conf",
            Some(client(1)),
            Some(item(10)),
            Some(&current),
            CmdParseFlags::default(),
        )
        .unwrap();
    assert_eq!(result, Some(item(2)));
    assert_eq!(runtime.commands[0].commands.len(), 2);
    assert_eq!(runtime.inserted, Some(item(10)));
    assert_eq!(runtime.appended, 0);
    assert_eq!(runtime.copied, Some((item(10), Some(current))));
    assert_eq!(runtime.freed, 1);
    assert_eq!(runtime.state_formats.len(), 1);
    assert_eq!(
        runtime
            .state_formats
            .get(b"current_file".as_slice())
            .unwrap()
            .as_ref(),
        b"exec.conf"
    );
    let parse_input = &runtime.parser.conditions[0];
    assert_eq!(parse_input.client, Some(client(1)));
    assert_eq!(parse_input.item, Some(item(10)));
    assert!(parse_input.target.is_empty());
    assert_ne!(parse_input.target, current);
    cfg.load_cfg_from_buffer(
        &mut runtime,
        b"display-message ok",
        b"global.conf",
        Some(client(1)),
        None,
        Some(&current),
        CmdParseFlags::default(),
    )
    .unwrap();
    assert_eq!(runtime.new_states, 1);
    assert_eq!(runtime.appended, 1);
}

#[test]
fn spec_section_2_11_file_loading_queues_globally_loads_once_and_returns_last_item() {
    let path = std::path::PathBuf::from(format!(
        "/tmp/rmx-cfg-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"display-message first\ndisplay-message last\n").unwrap();
    let mut cfg = CfgState {
        files: vec![path.as_os_str().as_bytes().into()],
        ..CfgState::default()
    };
    let mut runtime = Runtime {
        clients: vec![client(1)],
        ..Runtime::default()
    };
    cfg.start_cfg(&mut runtime);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(runtime.commands.len(), 1);
    assert_eq!(runtime.commands[0].commands.len(), 2);
    assert_eq!(runtime.appended, 1);
    assert!(runtime.inserted.is_none());
    assert_eq!(
        runtime.commands[0].commands[0]
            .file
            .as_ref()
            .unwrap()
            .as_ref(),
        path.as_os_str().as_bytes()
    );
    cfg.client_lost(&mut runtime, client(1));
    runtime.clients = vec![client(2)];
    cfg.start_cfg(&mut runtime);
    assert_eq!(runtime.commands.len(), 1);
    assert!(cfg.causes.is_empty());
}

#[test]
fn spec_section_2_11_selected_unattached_session_retains_causes_and_view_mode_is_top_only() {
    let mut cfg = CfgState::default();
    cfg.add_cause("one".into());
    cfg.add_cause("two".into());
    let mut runtime = Runtime {
        clients: vec![client(1)],
        sessions: vec![session(1), session(2)],
        attached: BTreeSet::from([session(2)]),
        ..Runtime::default()
    };
    runtime.client_sessions.insert(client(1), session(1));
    cfg.show_causes(&mut runtime, None);
    assert_eq!(cfg.causes.len(), 2);
    assert!(runtime.viewed.is_empty());
    runtime.clients.clear();
    cfg.show_causes(&mut runtime, None);
    assert_eq!(cfg.causes.len(), 2);
    cfg.show_causes(&mut runtime, Some(session(2)));
    assert!(cfg.causes.is_empty());
    assert_eq!(runtime.entered, 1);
    assert_eq!(
        runtime.viewed,
        vec![ByteString::from("one"), ByteString::from("two")]
    );
    cfg.add_cause("three".into());
    cfg.show_causes(&mut runtime, Some(session(2)));
    assert_eq!(runtime.entered, 1);
    runtime.view = false;
    cfg.add_cause("four".into());
    cfg.show_causes(&mut runtime, Some(session(2)));
    assert_eq!(runtime.entered, 2);
}

#[test]
fn spec_section_2_11_control_causes_use_actual_first_client_notification_and_print_routes() {
    let mut cfg = CfgState {
        client: Some(client(2)),
        ..CfgState::default()
    };
    let mut runtime = Runtime {
        clients: vec![client(1), client(2)],
        control: BTreeSet::from([client(1)]),
        ..Runtime::default()
    };
    cfg.add_cause("first".into());
    cfg.show_causes(&mut runtime, Some(session(9)));
    assert_eq!(
        runtime.notifications,
        vec![(client(1), ByteString::from("first"))]
    );
    assert!(runtime.viewed.is_empty());
    assert!(cfg.causes.is_empty());
    runtime.items.insert(item(1), Some(client(1)));
    runtime.items.insert(item(2), Some(client(2)));
    cfg.add_cause("control".into());
    cfg.print_causes(&mut runtime, item(1));
    assert_eq!(
        runtime.notifications.last(),
        Some(&(client(1), ByteString::from("control")))
    );
    cfg.add_cause("ordinary".into());
    cfg.print_causes(&mut runtime, item(2));
    cfg.add_cause("global".into());
    cfg.print_causes(&mut runtime, item(3));
    assert_eq!(
        runtime.printed,
        vec![
            (item(2), ByteString::from("ordinary")),
            (item(3), ByteString::from("global"))
        ]
    );
    assert!(cfg.causes.is_empty());
}

#[test]
fn spec_section_2_11_finish_shows_causes_before_unblocking_and_prompt_history() {
    let mut cfg = CfgState {
        item: Some(item(9)),
        ..CfgState::default()
    };
    let mut runtime = Runtime {
        sessions: vec![session(1)],
        attached: BTreeSet::from([session(1)]),
        ..Runtime::default()
    };
    cfg.add_cause("error".into());
    assert_eq!(cfg.done(&mut runtime), CmdReturn::Normal);
    assert!(cfg.finished);
    assert_eq!(runtime.effects, vec!["cause", "continue", "history"]);
    assert!(cfg.causes.is_empty());
    assert!(cfg.item.is_none());
    assert_eq!(runtime.entered, 1);
    let count = runtime.callbacks.len();
    let mut empty = CfgState::default();
    empty.start_cfg(&mut runtime);
    assert!(empty.client.is_none());
    assert!(empty.item.is_none());
    assert_eq!(runtime.callbacks.len(), count + 1);
    assert_eq!(runtime.callbacks.last().unwrap().1, CfgCallback::Done);
}

#[test]
fn unavailable_view_prints_every_cause_instead_of_losing_them() {
    let mut cfg = CfgState::default();
    cfg.add_cause("first".into());
    cfg.add_cause("second".into());
    let mut runtime = Runtime {
        clients: vec![client(1)],
        sessions: vec![session(1)],
        attached: BTreeSet::from([session(1)]),
        view_unavailable: true,
        ..Runtime::default()
    };
    cfg.show_causes(&mut runtime, None);
    assert!(cfg.causes.is_empty());
    assert!(runtime.viewed.is_empty());
    assert_eq!(
        runtime.fallback,
        vec![
            (Some(client(1)), ByteString::from("first")),
            (Some(client(1)), ByteString::from("second")),
        ]
    );
    assert_eq!(
        CfgViewError::Unavailable.to_string(),
        "view mode unavailable"
    );
    runtime.clients.clear();
    cfg.add_cause("without-client".into());
    cfg.show_causes(&mut runtime, Some(session(1)));
    assert_eq!(
        runtime.fallback.last(),
        Some(&(None, ByteString::from("without-client")))
    );
    assert!(cfg.causes.is_empty());
    runtime.view = true;
    runtime.view_append_failure = true;
    cfg.add_cause("append-failed".into());
    cfg.show_causes(&mut runtime, Some(session(1)));
    assert_eq!(
        runtime.fallback.last(),
        Some(&(None, ByteString::from("append-failed")))
    );
    assert!(cfg.causes.is_empty());
}
