// Ported from tmux cmd-set-environment.c @ 8f25579c
use super::support::{concat, fail, item_target};
use crate::cmd::{Command, queue::CmdReturn};
use crate::format;
use crate::ids::QueueItemId;
use crate::options::environment::EnvironmentFlags;
use crate::server::Server;
use rmux_util::bytes::cstr;

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let name = cstr(args.string(0).unwrap_or_default());

    if name.is_empty() {
        return fail(server, item, b"empty variable name");
    }
    if name.contains(&b'=') {
        return fail(server, item, b"variable name contains =");
    }

    let value = if args.count() < 2 {
        None
    } else {
        args.string(1)
    };
    let expanded;
    let value: Option<&[u8]> = match value {
        Some(v) if args.has(b'F') != 0 => {
            expanded = format::single_from_target(server, item, v);
            Some(expanded.as_bytes())
        }
        other => other,
    };

    let env = if args.has(b'g') != 0 {
        &mut server.global_environment
    } else {
        let Some(s) = target.s else {
            return match args.get(b't') {
                Some(tflag) => fail(server, item, concat(&[b"no such session: ", tflag])),
                None => fail(server, item, b"no current session"),
            };
        };
        let Some(session) = server.sessions.get_mut(s) else {
            return fail(server, item, b"no current session");
        };
        &mut session.environment
    };

    if args.has(b'u') != 0 {
        if value.is_some() {
            return fail(server, item, b"can't specify a value with -u");
        }
        env.unset(name);
    } else if args.has(b'r') != 0 {
        if value.is_some() {
            return fail(server, item, b"can't specify a value with -r");
        }
        env.clear(name);
    } else {
        let Some(value) = value else {
            return fail(server, item, b"no value specified");
        };
        let flags = if args.has(b'h') != 0 {
            EnvironmentFlags::HIDDEN
        } else {
            EnvironmentFlags::default()
        };
        env.set(name, flags, cstr(value));
    }
    CmdReturn::Normal
}
