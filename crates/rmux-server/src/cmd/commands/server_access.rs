// Ported from tmux cmd-server-access.c @ 8f25579c
use super::support::{concat, fail, item_target_client};
use crate::cmd::Command;
use crate::cmd::queue::{self, CmdReturn};
use crate::format::{self, FormatContext};
use crate::ids::QueueItemId;
use crate::server::Server;
use crate::server::acl::{self, ServerAclFlags};
use rmux_sys::PrincipalId;

/// `cmd_server_access_deny` (`cmd-server-access.c:48-58`).
fn deny(
    server: &mut Server,
    item: QueueItemId,
    id: PrincipalId,
    flags: ServerAclFlags,
    kind: &[u8],
    name: &[u8],
) -> CmdReturn {
    if !acl::find(server, id, flags) {
        return fail(server, item, concat(&[kind, b" ", name, b" not found"]));
    }
    acl::deny(server, id, flags);
    CmdReturn::Normal
}

/// `getgrnam`/`getpwnam` (`cmd-server-access.c:82-95`): the id, the database
/// name (the argument when the reverse lookup has no name) and the ACL flags.
fn lookup(arg: &[u8], group: bool) -> Option<(PrincipalId, Vec<u8>, ServerAclFlags)> {
    if group {
        let gid = rmux_sys::server::group_by_name(arg)?;
        let name = rmux_sys::server::group_name(gid).unwrap_or_else(|| arg.to_vec());
        Some((PrincipalId(gid.0), name, ServerAclFlags::IS_GROUP))
    } else {
        let uid = rmux_sys::server::user_by_name(arg)?;
        let name = rmux_sys::server::user_name(uid).unwrap_or_else(|| arg.to_vec());
        Some((PrincipalId(uid.0), name, ServerAclFlags::default()))
    }
}

/// `cmd_server_access_exec` (`cmd-server-access.c:60-152`).
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let c = item_target_client(server, item);

    if args.has(b'l') != 0 {
        for line in acl::display(server) {
            queue::print(server, item, &line);
        }
        return CmdReturn::Normal;
    }
    let Some(template) = args.string(0) else {
        return fail(server, item, b"missing user or group argument");
    };

    let arg = format::single(
        server,
        Some(item),
        FormatContext {
            evaluated_client: c,
            ..FormatContext::default()
        },
        template,
    );
    let group = args.has(b'g') != 0;
    let kind: &[u8] = if group { b"group" } else { b"user" };
    let Some((id, name, flags)) = lookup(arg.as_bytes(), group) else {
        return fail(
            server,
            item,
            concat(&[b"unknown ", kind, b": ", arg.as_bytes()]),
        );
    };

    if !flags.contains(ServerAclFlags::IS_GROUP)
        && (id.0 == 0 || id.0 == rmux_sys::proc::getuid().0)
    {
        return fail(
            server,
            item,
            concat(&[&name, b" owns the server, can't change access"]),
        );
    }

    if args.has(b'a') != 0 && args.has(b'd') != 0 {
        return fail(server, item, b"-a and -d cannot be used together");
    }
    if args.has(b'w') != 0 && args.has(b'r') != 0 {
        return fail(server, item, b"-r and -w cannot be used together");
    }

    if args.has(b'd') != 0 {
        return deny(server, item, id, flags, kind, &name);
    }
    if args.has(b'a') != 0 {
        if acl::find(server, id, flags) {
            return fail(
                server,
                item,
                concat(&[kind, b" ", &name, b" is already added"]),
            );
        }
        acl::allow(server, id, flags);
        // Do not return - allow -r or -w with -a.
    } else if args.has(b'r') != 0 || args.has(b'w') != 0 {
        // -r or -w implies -a if the entry does not exist.
        if !acl::find(server, id, flags) {
            acl::allow(server, id, flags);
        }
    }

    if args.has(b'w') != 0 {
        if !acl::find(server, id, flags) {
            return fail(server, item, concat(&[kind, b" ", &name, b" not found"]));
        }
        acl::allow_write(server, id, flags);
        return CmdReturn::Normal;
    }

    if args.has(b'r') != 0 {
        if !acl::find(server, id, flags) {
            return fail(server, item, concat(&[kind, b" ", &name, b" not found"]));
        }
        acl::deny_write(server, id, flags);
        return CmdReturn::Normal;
    }

    CmdReturn::Normal
}
