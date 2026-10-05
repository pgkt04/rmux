// Ported from tmux cmd-select-window.c @ 8f25579c
use super::support::{fail, item_client, item_current, item_target, set_item_current};
use crate::cmd::find::{self, CmdFindFlags};
use crate::cmd::metadata as m;
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::ids::QueueItemId;
use crate::model::session::{self, SelectOutcome};
use crate::server::Server;
use crate::server::operations::server_redraw_session;

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let c = item_client(server, item);
    let target = item_target(server, item);
    let Some(s) = target.s else {
        return fail(server, item, b"no current session");
    };

    let next = std::ptr::eq(command.entry, &m::CMD_NEXT_WINDOW) || args.has(b'n') != 0;
    let previous = std::ptr::eq(command.entry, &m::CMD_PREVIOUS_WINDOW) || args.has(b'p') != 0;
    let last = std::ptr::eq(command.entry, &m::CMD_LAST_WINDOW) || args.has(b'l') != 0;

    if next || previous || last {
        let activity = args.has(b'a') != 0;
        if next {
            if !matches!(
                session::session_next(server, s, activity),
                SelectOutcome::Changed
            ) {
                return fail(server, item, b"no next window");
            }
        } else if previous {
            if !matches!(
                session::session_previous(server, s, activity),
                SelectOutcome::Changed
            ) {
                return fail(server, item, b"no previous window");
            }
        } else if !matches!(session::session_last(server, s), SelectOutcome::Changed) {
            return fail(server, item, b"no last window");
        }
        let current = find::from_session(server, s, CmdFindFlags::default());
        set_item_current(server, item, &current);
        server_redraw_session(server, s);
        queue::insert_hook(server, item, Some(&current), b"after-select-window");
    } else {
        let Some(wl) = target.wl else {
            return fail(server, item, b"no current window");
        };
        let curw = server.sessions.get(s).and_then(|s| s.current);
        // -T on the current window switches to the previous window (cmd-select-window.c:126-139).
        if args.has(b'T') != 0 && curw == Some(wl) {
            if !matches!(session::session_last(server, s), SelectOutcome::Changed) {
                return fail(server, item, b"no last window");
            }
            if item_current(server, item).s == Some(s) {
                let current = find::from_session(server, s, CmdFindFlags::default());
                set_item_current(server, item, &current);
            }
            server_redraw_session(server, s);
        } else {
            let idx = server.winlinks.get(wl).map_or(-1, |wl| wl.index);
            if matches!(
                session::session_select(server, s, idx),
                SelectOutcome::Changed
            ) {
                let current = find::from_session(server, s, CmdFindFlags::default());
                set_item_current(server, item, &current);
                server_redraw_session(server, s);
            }
        }
        let current = item_current(server, item);
        queue::insert_hook(server, item, Some(&current), b"after-select-window");
    }

    if let Some(c) = c
        && server
            .clients
            .get(c)
            .is_some_and(|client| client.session.is_some())
        && let Some(w) = server
            .sessions
            .get(s)
            .and_then(|s| s.current)
            .and_then(|wl| server.winlinks.get(wl))
            .map(|wl| wl.window)
        && let Some(window) = server.windows.get_mut(w)
    {
        window.latest = Some(c);
    }
    crate::server::run::recalculate_sizes(server);
    CmdReturn::Normal
}
