// Ported from tmux cmd-unbind-key.c @ 8f25579c
use super::support::{concat, fail};
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::QueueItemId;
use crate::server::Server;
use rmux_tty::key_string::parse_key_name;
use rmux_util::key::SpecialKey;

/// `cmdq_error` unless `-q`; the return value is always `CMD_RETURN_ERROR`.
fn quiet_fail(
    server: &mut Server,
    item: QueueItemId,
    quiet: bool,
    message: impl AsRef<[u8]>,
) -> CmdReturn {
    if quiet {
        return CmdReturn::Error;
    }
    fail(server, item, message)
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let keystr = args.string(0);
    let quiet = args.has(b'q') != 0;
    let default_table: &[u8] = if args.has(b'n') != 0 {
        b"root"
    } else {
        b"prefix"
    };

    if args.has(b'a') != 0 {
        if keystr.is_some() {
            return quiet_fail(server, item, quiet, b"key given with -a");
        }
        let tablename = args.get(b'T').unwrap_or(default_table);
        if server.key_bindings.find_table(tablename).is_none() {
            return quiet_fail(
                server,
                item,
                quiet,
                concat(&[b"table ", tablename, b" doesn't exist"]),
            );
        }
        // `remove_table` resets the clients that use the table through the
        // runtime, so the store is lent out for the duration of the call.
        let mut bindings = std::mem::take(&mut server.key_bindings);
        let result = bindings.remove_table(server, tablename);
        server.key_bindings = bindings;
        if let Err(cause) = result {
            return fail(server, item, cause.to_string().as_bytes());
        }
        return CmdReturn::Normal;
    }

    let Some(keystr) = keystr else {
        return quiet_fail(server, item, quiet, b"missing key");
    };
    let key = parse_key_name(keystr);
    if key.0 == SpecialKey::NONE || key.0 == SpecialKey::UNKNOWN {
        return quiet_fail(server, item, quiet, concat(&[b"unknown key: ", keystr]));
    }
    let tablename = match args.get(b'T') {
        Some(tablename) => {
            if server.key_bindings.find_table(tablename).is_none() {
                return quiet_fail(
                    server,
                    item,
                    quiet,
                    concat(&[b"table ", tablename, b" doesn't exist"]),
                );
            }
            tablename
        }
        None => default_table,
    };
    if let Err(cause) = server.key_bindings.remove(tablename, key) {
        return fail(server, item, cause.to_string().as_bytes());
    }
    CmdReturn::Normal
}
