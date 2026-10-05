// Ported from tmux cmd-rename-session.c @ 8f25579c
use super::support::{concat, fail, item_target};
use crate::cmd::find::{self, CmdFindFlags};
use crate::cmd::{Command, queue::CmdReturn};
use crate::format;
use crate::ids::QueueItemId;
use crate::model::session;
use crate::server::Server;
use crate::server::events::{self, EventPayload};
use crate::server::operations::server_status_session;
use rmux_util::shell::clean_name;

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let Some(s) = item_target(server, item).s else {
        return fail(server, item, b"no current session");
    };
    let template = command.args.string(0).unwrap_or(b"");
    let tmp = format::single_from_target(server, item, template);
    // check_name is utf8_isvalid; clean_name fails only on invalid UTF-8 (tmux.c:345-367).
    let Some(newname) = clean_name(&tmp, false) else {
        return fail(server, item, concat(&[b"invalid session name: ", &tmp]));
    };
    let Some(oldname) = server.sessions.get(s).map(|s| s.name.clone()) else {
        return fail(server, item, b"no current session");
    };
    if newname == oldname {
        return CmdReturn::Normal;
    }
    if session::session_find(server, &newname).is_some() {
        return fail(server, item, concat(&[b"duplicate session: ", &newname]));
    }

    let mut ep = EventPayload::new();
    let fs = find::from_session(server, s, CmdFindFlags::default());
    ep.set_target(server, &fs);
    ep.set_session(server, b"session", s);
    ep.set_string(server, b"old_name", &oldname);
    ep.set_string(server, b"new_name", &newname);

    // RB_REMOVE / RB_INSERT on the sessions tree (cmd-rename-session.c:81-84).
    server.session_names.remove(&oldname);
    if let Some(session) = server.sessions.get_mut(s) {
        session.name = newname.clone();
    }
    server.session_names.insert(newname, s);

    server_status_session(server, s);
    events::fire(server, b"session-renamed", ep);

    CmdReturn::Normal
}
