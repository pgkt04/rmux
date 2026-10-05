// Ported from tmux cmd-list-keys.c @ 8f25579c
use crate::{
    client::ClientFlags,
    cmd::{
        Command, CommandListPrintFlags,
        key_bindings::KeyBindingFlags,
        queue::{self, CmdReturn},
    },
    format::{
        self, FormatContext,
        sort::{self, SortCriteria, SortOrder},
    },
    ids::QueueItemId,
    server::Server,
};
use rmux_tty::key_string::{key_name, parse_key_name};
use rmux_util::{
    key::{KeyCode, KeyMasks, SpecialKey},
    utf8,
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let only = args.string(0).map(parse_key_name);
    if only.is_some_and(|key| key.0 == SpecialKey::UNKNOWN) {
        let mut cause = b"invalid key: ".to_vec();
        cause.extend_from_slice(args.string(0).unwrap_or_default());
        queue::error(server, item, &cause);
        return CmdReturn::Error;
    }
    let order = sort::order_from_string(args.get(b'O'));
    if args.has(b'O') != 0 && order == SortOrder::End {
        queue::error(server, item, b"invalid sort order");
        return CmdReturn::Error;
    }
    let criteria = SortCriteria {
        order,
        reversed: args.has(b'r') != 0,
        ..SortCriteria::default()
    };
    let mut bindings = Vec::new();
    if let Some(name) = args.get(b'T') {
        let Some(table) = server.key_bindings.find_table(name) else {
            let mut cause = b"table ".to_vec();
            cause.extend_from_slice(name);
            cause.extend_from_slice(b" doesn't exist");
            queue::error(server, item, &cause);
            return CmdReturn::Error;
        };
        sort::get_key_bindings_table(&server.key_bindings, table, &criteria, &mut bindings);
    } else if args.has(b'N') != 0 {
        for name in [b"prefix".as_slice(), b"root"] {
            if let Some(table) = server.key_bindings.find_table(name) {
                let mut list = Vec::new();
                sort::get_key_bindings_table(&server.key_bindings, table, &criteria, &mut list);
                bindings.extend(list);
            }
        }
    } else {
        sort::get_key_bindings(&server.key_bindings, &criteria, &mut bindings);
    }
    let mask = KeyMasks::KEY | KeyMasks::MODIFIERS;
    bindings.retain(|(table, key)| {
        server
            .key_bindings
            .tables
            .get(*table)
            .and_then(|t| t.bindings.get(key))
            .is_some_and(|binding| {
                only.is_none_or(|only| only.0 & mask == key.0 & mask)
                    && (args.has(b'N') == 0 || args.has(b'a') != 0 || binding.note.is_some())
            })
    });
    if only.is_some() && bindings.is_empty() {
        let mut cause = b"unknown key: ".to_vec();
        cause.extend_from_slice(args.string(0).unwrap_or_default());
        queue::error(server, item, &cause);
        return CmdReturn::Error;
    }
    let single = args.has(b'1') != 0;
    if single {
        bindings.truncate(1);
    }
    let key_width = bindings
        .iter()
        .map(|(_, key)| utf8::cstr_width(&key_name(*key, false)))
        .max()
        .unwrap_or(0);
    let table_width = bindings
        .iter()
        .filter_map(|(table, _)| server.key_bindings.tables.get(*table))
        .map(|table| utf8::cstr_width(&table.name))
        .max()
        .unwrap_or(0);
    let repeat = bindings.iter().any(|(table, key)| {
        server
            .key_bindings
            .tables
            .get(*table)
            .and_then(|t| t.bindings.get(key))
            .is_some_and(|b| b.flags.contains(KeyBindingFlags::REPEAT))
    });
    let prefix = args.get(b'P').map(<[u8]>::to_vec).unwrap_or_else(|| {
        let key = KeyCode(
            server
                .options
                .get_number(server.options.global_s, b"prefix") as u64,
        );
        if key.0 == SpecialKey::NONE {
            Vec::new()
        } else {
            key_name(key, false)
        }
    });
    let client = server.queue.items.get(item).and_then(|i| i.target_client);
    let mut tree = format::create_defaults(
        server,
        Some(item),
        FormatContext {
            evaluated_client: client,
            ..FormatContext::default()
        },
    );
    tree.add(
        b"notes_only",
        if args.has(b'N') != 0 {
            b"1".as_slice()
        } else {
            b"0"
        }
        .into(),
    );
    tree.add(
        b"key_has_repeat",
        if repeat { b"1".as_slice() } else { b"0" }.into(),
    );
    tree.add(b"key_string_width", key_width.to_string().into());
    tree.add(b"key_table_width", table_width.to_string().into());
    let template = args.get(b'F').unwrap_or(b"#{?notes_only,#{key_prefix} #{p|#{key_string_width}:key_string} #{?key_note,#{key_note},#{key_command}},bind-key #{?key_has_repeat,#{?key_repeat,-r,  },} -T #{p|#{key_table_width}:key_table} #{p|#{key_string_width}:#{q|a:key_string}} #{key_command}}");
    for (table, key) in bindings {
        let Some(table) = server.key_bindings.tables.get(table) else {
            continue;
        };
        let Some(binding) = table.bindings.get(&key) else {
            continue;
        };
        tree.add(
            b"key_repeat",
            if binding.flags.contains(KeyBindingFlags::REPEAT) {
                b"1".as_slice()
            } else {
                b"0"
            }
            .into(),
        );
        tree.add(b"key_note", binding.note.clone().unwrap_or_default());
        tree.add(b"key_prefix", prefix.as_slice().into());
        tree.add(b"key_table", table.name.clone());
        tree.add(b"key_string", key_name(key, false).into());
        tree.add(
            b"key_command",
            binding
                .list
                .print(CommandListPrintFlags::ESCAPED | CommandListPrintFlags::NO_GROUPS),
        );
        let line = tree.expand(server, template);
        if single
            && client
                .and_then(|c| server.clients.get(c))
                .is_some_and(|c| !c.flags.contains(ClientFlags::CONTROL))
        {
            crate::ui::status::status_message_set(server, client, -1, true, false, false, &line);
        } else if !line.is_empty() {
            queue::print(server, item, &line);
        }
    }
    tree.release(server);
    CmdReturn::Normal
}
