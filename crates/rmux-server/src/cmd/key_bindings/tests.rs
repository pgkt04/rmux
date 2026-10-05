// Ported from tmux key-bindings.c, cmd-bind-key.c, cmd-list-keys.c @ 8f25579c
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

use super::test_support::Context;
use super::*;
use crate::cmd::CommandListPrintFlags;
use crate::cmd::arguments::{self, ArgsValueData};
use crate::ids::ArenaId;
use rmux_tty::key_string::{key_name, parse_key_name};
use std::process::Command as Process;

fn list(context: &mut Context, text: &[u8]) -> Rc<CommandList> {
    parse::from_string(context, text, &mut CmdParseInput::default()).unwrap()
}

struct Clients {
    table: Option<KeyTableId>,
    resets: usize,
}
impl KeyTableRuntime for Clients {
    fn reset_clients_using_table(
        &mut self,
        bindings: &mut KeyBindings,
        table: KeyTableId,
    ) -> Result<(), ArenaError> {
        assert!(!bindings.index.values().any(|id| *id == table));
        assert!(bindings.tables.get(table).is_some());
        if self.table == Some(table) {
            self.resets += 1;
            self.table = None;
            bindings.unref_table(table)?;
        }
        Ok(())
    }
}

#[test]
fn spec_section_2_10_null_list_add_masks_flags_and_never_clears_repeat() {
    let mut bindings = KeyBindings::new();
    bindings
        .add(b"prefix", KeyCode(65), Some(b"ignored"), true, None)
        .unwrap();
    let table = bindings.find_table(b"prefix").unwrap();
    assert!(bindings.tables.get(table).unwrap().bindings.is_empty());
    let commands = Rc::new(CommandList::default());
    bindings
        .add(
            b"prefix",
            KeyCode(65 | KeyMasks::FLAGS),
            Some(b"old"),
            true,
            Some(commands),
        )
        .unwrap();
    let serial = bindings.get(table, KeyCode(65)).unwrap().serial();
    bindings
        .add(b"prefix", KeyCode(65), None, false, None)
        .unwrap();
    let binding = bindings.get(table, KeyCode(65)).unwrap();
    assert_eq!(binding.note.as_ref().unwrap().as_ref(), b"old");
    assert!(binding.flags.contains(KeyBindingFlags::REPEAT));
    bindings
        .add(b"prefix", KeyCode(65), Some(b"new"), false, None)
        .unwrap();
    assert_eq!(
        bindings
            .get(table, KeyCode(65))
            .unwrap()
            .note
            .as_ref()
            .unwrap()
            .as_ref(),
        b"new"
    );
    assert_eq!(bindings.get(table, KeyCode(65)).unwrap().serial(), serial);
    bindings
        .add(
            b"prefix",
            KeyCode(65),
            None,
            false,
            Some(Rc::new(CommandList::default())),
        )
        .unwrap();
    assert_ne!(bindings.get(table, KeyCode(65)).unwrap().serial(), serial);
    assert!(
        !bindings
            .get(table, KeyCode(65))
            .unwrap()
            .flags
            .contains(KeyBindingFlags::REPEAT)
    );
    bindings
        .add(b"prefix", KeyCode(65), None, true, None)
        .unwrap();
    assert!(
        bindings
            .get(table, KeyCode(65))
            .unwrap()
            .flags
            .contains(KeyBindingFlags::REPEAT)
    );
    assert!(bindings.get(table, KeyCode(65 | KeyMasks::FLAGS)).is_none());
    bindings.remove(b"prefix", KeyCode(99)).unwrap();
    bindings.remove(b"absent", KeyCode(65)).unwrap();
    assert_eq!(bindings.find_table(b"prefix"), Some(table));
    bindings.reset(b"absent", KeyCode(65)).unwrap();
}

#[test]
fn spec_section_2_10_default_snapshot_identity_and_live_only_reset() {
    let mut bindings = KeyBindings::new();
    let original = Rc::new(CommandList::default());
    for key in [65, 66] {
        bindings
            .add(
                b"prefix",
                KeyCode(key),
                Some(b"note"),
                true,
                Some(Rc::clone(&original)),
            )
            .unwrap();
    }
    let table = bindings.find_table(b"prefix").unwrap();
    let live_serial = bindings.get(table, KeyCode(65)).unwrap().serial();
    bindings.init_done();
    let default = bindings.get_default(table, KeyCode(65)).unwrap();
    assert!(Rc::ptr_eq(&default.list, &original));
    assert_ne!(default.serial(), live_serial);
    bindings
        .remove(b"prefix", KeyCode(66 | KeyMasks::FLAGS))
        .unwrap();
    bindings.reset(b"prefix", KeyCode(66)).unwrap();
    assert!(bindings.get(table, KeyCode(66)).is_none());
    bindings
        .add(
            b"prefix",
            KeyCode(65),
            None,
            false,
            Some(Rc::new(CommandList::default())),
        )
        .unwrap();
    let replacement_serial = bindings.get(table, KeyCode(65)).unwrap().serial();
    bindings
        .add(
            b"prefix",
            KeyCode(67),
            None,
            false,
            Some(Rc::new(CommandList::default())),
        )
        .unwrap();
    let mut clients = Clients {
        table: None,
        resets: 0,
    };
    bindings.reset_table(&mut clients, b"prefix").unwrap();
    let restored = bindings.get(table, KeyCode(65)).unwrap();
    assert!(Rc::ptr_eq(&restored.list, &original));
    assert_eq!(restored.serial(), replacement_serial);
    assert_eq!(restored.note.as_ref().unwrap().as_ref(), b"note");
    assert!(restored.flags.contains(KeyBindingFlags::REPEAT));
    assert!(bindings.get(table, KeyCode(66)).is_none());
    assert!(bindings.get(table, KeyCode(67)).is_none());
}

#[test]
fn spec_section_2_10_table_index_removal_preserves_leases_and_resets_clients() {
    let mut bindings = KeyBindings::new();
    bindings
        .add(
            b"z",
            KeyCode(65),
            None,
            false,
            Some(Rc::new(CommandList::default())),
        )
        .unwrap();
    let table = bindings.find_table(b"z").unwrap();
    bindings.retain_table(table).unwrap();
    bindings.remove(b"z", KeyCode(65)).unwrap();
    assert!(bindings.find_table(b"z").is_none());
    assert!(bindings.tables.get(table).is_some());
    bindings.unref_table(table).unwrap();
    assert!(bindings.tables.get(table).is_none());
    let replacement = bindings.get_table(b"z", true).unwrap().unwrap();
    assert_ne!(table, replacement);
    bindings.retain_table(replacement).unwrap();
    bindings.retain_table(replacement).unwrap();
    let mut clients = Clients {
        table: Some(replacement),
        resets: 0,
    };
    bindings.reset_table(&mut clients, b"z").unwrap();
    assert_eq!(clients.resets, 1);
    assert!(bindings.tables.get(replacement).is_some());
    bindings.unref_table(replacement).unwrap();
    assert!(bindings.tables.get(replacement).is_none());
}

#[derive(Default)]
struct Dispatch {
    readonly: bool,
    blocked: bool,
    current: Option<CmdFindState>,
    event: Option<QueueEvent>,
    flags: QueueStateFlags,
    inserted: Option<QueueItemId>,
    appended: Option<Option<ClientId>>,
    command_count: usize,
}
impl KeyDispatchRuntime for Dispatch {
    fn client_read_only(&self, _client: ClientId) -> bool {
        self.readonly
    }
    fn binding_commands(
        &mut self,
        list: Rc<CommandList>,
        current: &CmdFindState,
        event: Option<&QueueEvent>,
        flags: QueueStateFlags,
    ) -> QueueBatch {
        self.current = Some(*current);
        self.event = event.copied();
        self.flags = flags;
        self.command_count = list.commands.len();
        QueueBatch {
            items: vec![QueueItemId::from_parts(1, 0)],
        }
    }
    fn binding_read_only(&mut self) -> QueueBatch {
        self.blocked = true;
        QueueBatch {
            items: vec![QueueItemId::from_parts(2, 0)],
        }
    }
    fn append_binding(
        &mut self,
        client: Option<ClientId>,
        batch: QueueBatch,
    ) -> Option<QueueItemId> {
        self.appended = Some(client);
        batch.items.last().copied()
    }
    fn insert_binding(&mut self, item: QueueItemId, batch: QueueBatch) -> Option<QueueItemId> {
        self.inserted = Some(item);
        batch.items.last().copied()
    }
}

#[test]
fn spec_section_2_10_read_only_dispatch_repeat_event_target_and_empty_list() {
    let mut context = Context::default();
    let mutable = list(&mut context, b"new-session");
    let readonly = list(&mut context, b"copy-mode");
    assert!(readonly.all_have(CommandFlags::READONLY));
    let client = ClientId::from_parts(5, 0);
    let item = QueueItemId::from_parts(6, 0);
    let current = CmdFindState {
        idx: 42,
        ..CmdFindState::default()
    };
    let event = QueueEvent {
        key: KeyCode(65),
        ..QueueEvent::default()
    };
    let mut runtime = Dispatch {
        readonly: true,
        ..Dispatch::default()
    };
    assert_eq!(
        dispatch(
            &mut runtime,
            KeyBindingDispatch {
                list: mutable,
                flags: KeyBindingFlags::REPEAT
            },
            Some(item),
            Some(client),
            Some(&event),
            &current
        ),
        Some(QueueItemId::from_parts(2, 0))
    );
    assert!(runtime.blocked);
    assert_eq!(runtime.inserted, Some(item));
    let mut runtime = Dispatch {
        readonly: true,
        ..Dispatch::default()
    };
    dispatch(
        &mut runtime,
        KeyBindingDispatch {
            list: readonly,
            flags: KeyBindingFlags::REPEAT,
        },
        None,
        Some(client),
        Some(&event),
        &current,
    );
    assert!(!runtime.blocked);
    assert_eq!(runtime.event, Some(event));
    assert_eq!(runtime.current, Some(current));
    assert_eq!(runtime.flags, QueueStateFlags::REPEAT);
    assert_eq!(runtime.appended, Some(Some(client)));
    let mut runtime = Dispatch {
        readonly: true,
        ..Dispatch::default()
    };
    dispatch(
        &mut runtime,
        KeyBindingDispatch {
            list: Rc::new(CommandList::default()),
            flags: KeyBindingFlags::default(),
        },
        None,
        Some(client),
        None,
        &current,
    );
    assert!(!runtime.blocked);
    assert_eq!(runtime.command_count, 0);
    let mut runtime = Dispatch {
        readonly: true,
        ..Dispatch::default()
    };
    dispatch(
        &mut runtime,
        KeyBindingDispatch {
            list: list(&mut context, b"new-session"),
            flags: KeyBindingFlags::default(),
        },
        None,
        None,
        None,
        &current,
    );
    assert!(!runtime.blocked);
    let mut runtime = Dispatch::default();
    dispatch(
        &mut runtime,
        KeyBindingDispatch {
            list: list(&mut context, b"new-session"),
            flags: KeyBindingFlags::default(),
        },
        None,
        Some(client),
        None,
        &current,
    );
    assert!(!runtime.blocked);
    assert_eq!(runtime.flags, QueueStateFlags::default());
    assert_eq!(runtime.appended, Some(Some(client)));
}

fn interpret_default(bindings: &mut KeyBindings, context: &mut Context, commands: &CommandList) {
    for command in &commands.commands {
        assert_eq!(command.entry.name, b"bind-key");
        let args = &command.args;
        let key = parse_key_name(args.string(0).unwrap());
        let table = args.get(b'T').unwrap_or(if args.has(b'n') != 0 {
            b"root"
        } else {
            b"prefix"
        });
        let body = if args.count() == 2 {
            match &args.value(1).unwrap().data {
                ArgsValueData::Commands(commands) => Rc::clone(commands),
                _ => list(context, args.string(1).unwrap()),
            }
        } else {
            parse::from_arguments(context, &args.values()[1..], &mut CmdParseInput::default())
                .unwrap()
        };
        bindings
            .add(table, key, args.get(b'N'), args.has(b'r') != 0, Some(body))
            .unwrap();
    }
}

struct Initializer {
    context: Context,
    commands: Vec<Rc<CommandList>>,
    snapshot: bool,
}
impl ParseContext for Initializer {
    fn environment(&self, name: &[u8]) -> Option<&[u8]> {
        self.context.environment(name)
    }
    fn put_environment(&mut self, a: &[u8], h: bool) {
        self.context.put_environment(a, h);
    }
    fn alias(&self, name: &[u8]) -> Option<ByteString> {
        self.context.alias(name)
    }
    fn condition(&mut self, f: &[u8], i: &CmdParseInput) -> bool {
        self.context.condition(f, i)
    }
    fn home(&mut self, u: Option<&[u8]>) -> Option<ByteString> {
        self.context.home(u)
    }
    fn next_group(&mut self) -> u32 {
        self.context.next_group()
    }
    fn print(&mut self, m: &[u8], i: &CmdParseInput) {
        self.context.print(m, i);
    }
}
impl KeyInitRuntime for Initializer {
    fn append_default_commands(&mut self, list: Rc<CommandList>) {
        assert!(!self.snapshot);
        self.commands.push(list);
    }
    fn append_default_snapshot(&mut self) {
        assert_eq!(self.commands.len(), 308);
        self.snapshot = true;
    }
}

fn defaults() -> KeyBindings {
    let mut runtime = Initializer {
        context: Context::default(),
        commands: Vec::new(),
        snapshot: false,
    };
    init(&mut runtime).unwrap();
    assert!(runtime.snapshot);
    let mut bindings = KeyBindings::new();
    for commands in runtime.commands {
        interpret_default(&mut bindings, &mut runtime.context, &commands);
    }
    bindings.init_done();
    bindings
}

#[test]
fn spec_section_2_10_default_parse_failure_aborts_initialization() {
    let mut runtime = Initializer {
        context: Context::default(),
        commands: Vec::new(),
        snapshot: false,
    };
    runtime
        .context
        .aliases
        .insert("bind".into(), "unknown-g11-command".into());
    let error = init(&mut runtime).unwrap_err();
    assert_eq!(error.binding, super::defaults::DEFAULT_BINDINGS[0]);
    assert!(error.error.message().starts_with(b"unknown command:"));
    assert!(runtime.commands.is_empty());
    assert!(!runtime.snapshot);
}

#[test]
fn spec_section_2_10_all_308_defaults_and_menus_are_initialized_before_snapshot() {
    let bindings = defaults();
    // Pinned C has 93 prefix, 30 root, 19 move, 78 emacs, and 88 vi bindings.
    let counts: Vec<_> = bindings
        .tables()
        .map(|id| {
            let table = bindings.tables.get(id).unwrap();
            assert_eq!(table.bindings.len(), table.defaults.len());
            (table.name.clone(), table.bindings.len())
        })
        .collect();
    assert_eq!(
        counts,
        vec![
            ("copy-mode".into(), 78),
            ("copy-mode-vi".into(), 88),
            ("move".into(), 19),
            ("prefix".into(), 93),
            ("root".into(), 30)
        ]
    );
    assert!(!super::defaults::DEFAULT_SESSION_MENU.is_empty());
    assert!(!super::defaults::DEFAULT_WINDOW_MENU.is_empty());
    assert!(!super::defaults::DEFAULT_EMPTY_MENU.is_empty());
    assert!(!super::defaults::DEFAULT_PANE_MENU.is_empty());
    assert!(!super::defaults::DEFAULT_MOVE_MENU.is_empty());
    assert!(!super::defaults::DEFAULT_MOVE_RESIZE_MENU.is_empty());
    assert!(has_repeat(
        bindings
            .tables()
            .flat_map(|id| bindings.tables.get(id).unwrap().bindings())
    ));
}

// Test-only expansion of the stock list-keys format; no G20 command body or G10 formatter.
fn rendered_defaults(bindings: &KeyBindings, notes: bool) -> Vec<u8> {
    let selected: Vec<_> = bindings
        .tables()
        .flat_map(|id| {
            let table = bindings.tables.get(id).unwrap();
            table.bindings().map(move |b| (table, b))
        })
        .filter(|(table, binding)| {
            !notes || ((table.name == "prefix" || table.name == "root") && binding.note.is_some())
        })
        .collect();
    let key_width = selected
        .iter()
        .map(|(_, b)| key_name(b.key, false).len())
        .max()
        .unwrap();
    let table_width = selected.iter().map(|(t, _)| t.name.len()).max().unwrap();
    let repeat = has_repeat(selected.iter().map(|(_, b)| *b));
    let mut out = Vec::new();
    for (table, binding) in selected {
        let key = key_name(binding.key, false);
        if notes {
            out.extend_from_slice(b"C-b ");
            out.extend_from_slice(&key);
            out.extend(std::iter::repeat_n(
                b' ',
                key_width.saturating_sub(key.len()),
            ));
            out.push(b' ');
            out.extend_from_slice(binding.note.as_ref().unwrap());
        } else {
            out.extend_from_slice(b"bind-key ");
            if repeat {
                out.extend_from_slice(if binding.flags.contains(KeyBindingFlags::REPEAT) {
                    b"-r"
                } else {
                    b"  "
                });
            }
            out.extend_from_slice(b" -T ");
            out.extend_from_slice(&table.name);
            out.extend(std::iter::repeat_n(
                b' ',
                table_width.saturating_sub(table.name.len()),
            ));
            out.push(b' ');
            let escaped = arguments::escape(&key);
            out.extend_from_slice(&escaped);
            out.extend(std::iter::repeat_n(
                b' ',
                key_width.saturating_sub(escaped.len()),
            ));
            out.push(b' ');
            out.extend_from_slice(
                &binding
                    .list
                    .print(CommandListPrintFlags::ESCAPED | CommandListPrintFlags::NO_GROUPS),
            );
        }
        out.push(b'\n');
    }
    out
}

struct Oracle {
    binary: std::path::PathBuf,
    socket: std::path::PathBuf,
    root: std::path::PathBuf,
}
impl Oracle {
    fn command(&self) -> Process {
        let mut cmd = Process::new(&self.binary);
        cmd.arg("-S").arg(&self.socket).arg("-f/dev/null");
        cmd
    }
}
impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn spec_section_2_10_oracle_default_list_keys_and_notes_differential() {
    let binary = std::env::var_os("RMUX_ORACLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux")
        });
    if !binary.exists() {
        eprintln!(
            "SKIP default list-keys/-N differential: oracle missing at {}",
            binary.display()
        );
        return;
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::path::PathBuf::from(format!("/tmp/rmx-keys-{}-{unique}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let oracle = Oracle {
        binary,
        socket: root.join("s"),
        root,
    };
    let started = oracle
        .command()
        .args(["new-session", "-d", "-s", "g11keys", "sleep 60"])
        .output()
        .unwrap();
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let bindings = defaults();
    for notes in [false, true] {
        let mut command = oracle.command();
        command.arg("list-keys");
        if notes {
            command.arg("-N");
        }
        let expected = command.output().unwrap();
        assert!(
            expected.status.success(),
            "{}",
            String::from_utf8_lossy(&expected.stderr)
        );
        assert_eq!(
            String::from_utf8(rendered_defaults(&bindings, notes)).unwrap(),
            String::from_utf8(expected.stdout).unwrap(),
            "list-keys notes={notes}"
        );
    }
}

#[test]
fn spec_section_2_10_generated_strings_and_all_menus_match_pinned_c() {
    let source = std::env::var_os("RMUX_TMUX_SOURCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/Users/j/fun/tmux"));
    if !source.join(".git").exists() {
        eprintln!(
            "SKIP generated key data pin check: set RMUX_TMUX_SOURCE to the pinned tmux checkout"
        );
        return;
    }
    let reference = Process::new("git")
        .arg("-C")
        .arg(&source)
        .args(["show", "8f25579c:key-bindings.c"])
        .output()
        .unwrap();
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::path::PathBuf::from(format!(
        "/tmp/rmx-key-data-{}-{unique}.c",
        std::process::id()
    ));
    std::fs::write(&path, reference.stdout).unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cmd/key_bindings");
    let check = Process::new("python3")
        .arg(directory.join("generate_defaults.py"))
        .arg(&path)
        .arg(directory.join("defaults.rs"))
        .arg("--check")
        .output();
    std::fs::remove_file(&path).unwrap();
    let check = check.expect("python3 is required for the maintained default-data generator test");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}
