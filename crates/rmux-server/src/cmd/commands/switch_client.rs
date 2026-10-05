// Ported from tmux cmd-switch-client.c @ 8f25579c
use super::support::{
    concat, fail, item_client, item_current, item_event, item_flags, item_target_client,
    set_item_current,
};
use crate::client::{self, ClientFlags};
use crate::cmd::find::{self, CmdFindFlags, CmdFindType, FindContext};
use crate::cmd::queue::QueueStateFlags;
use crate::cmd::{Command, queue::CmdReturn};
use crate::format::sort::{self, SortCriteria, SortOrder};
use crate::ids::QueueItemId;
use crate::model::{pane, session, window};
use crate::server::Server;
use crate::server::operations::{server_redraw_window, update_session_environment};

/// `tflag[strcspn(tflag, ":.%")] != '\0' || strcmp(tflag, "=") == 0` (cmd-switch-client.c:68-69).
pub fn is_pane_target(tflag: &[u8]) -> bool {
    let tflag = rmux_util::bytes::cstr(tflag);
    tflag.iter().any(|&b| b == b':' || b == b'.' || b == b'%') || tflag == b"="
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let tflag = args.get(b't');
    let zflag = args.has(b'Z') != 0;
    let c = item_client(server, item);
    let Some(tc) = item_target_client(server, item) else {
        return fail(server, item, b"no current client");
    };

    let (kind, flags) = match tflag {
        Some(t) if is_pane_target(t) => (CmdFindType::Pane, CmdFindFlags::default()),
        _ => (CmdFindType::Session, CmdFindFlags::PREFER_UNATTACHED),
    };
    let context = FindContext {
        client: c,
        current: item_current(server, item),
        mouse: item_event(server, item).mouse,
    };
    let target = match find::target(server, &context, tflag, kind, flags) {
        Ok(target) => target,
        Err(cause) => {
            if let Some(message) = cause.message {
                return fail(server, item, message);
            }
            return CmdReturn::Error;
        }
    };
    let Some(mut s) = target.s else {
        return CmdReturn::Error;
    };
    let wl = target.wl;
    let wp = target.wp;

    if args.has(b'r') != 0 {
        let read_only = server
            .clients
            .get(tc)
            .is_some_and(|c| c.flags.contains(ClientFlags::READONLY));
        if read_only {
            let uid = c
                .and_then(|c| server.clients.get(c))
                .and_then(|c| c.peer)
                .and_then(|peer| server.process.peer_uid(peer));
            if uid != Some(rmux_sys::proc::getuid()) {
                return fail(server, item, b"client is read-only");
            }
        }
        if let Some(client) = server.clients.get_mut(tc) {
            if read_only {
                client
                    .flags
                    .remove(ClientFlags::READONLY | ClientFlags::IGNORESIZE);
            } else {
                client
                    .flags
                    .insert(ClientFlags::READONLY | ClientFlags::IGNORESIZE);
            }
        }
    }

    if let Some(tablename) = args.get(b'T') {
        let Ok(Some(table)) = server.key_bindings.get_table(tablename, false) else {
            return fail(
                server,
                item,
                concat(&[b"table ", tablename, b" doesn't exist"]),
            );
        };
        let _ = server.key_bindings.retain_table(table);
        if let Some(old) = server.clients.get(tc).and_then(|c| c.keytable) {
            let _ = server.key_bindings.unref_table(old);
        }
        if let Some(client) = server.clients.get_mut(tc) {
            client.keytable = Some(table);
        }
        return CmdReturn::Normal;
    }

    let order = sort::order_from_string(args.get(b'O'));
    if order == SortOrder::End && args.has(b'O') != 0 {
        return fail(server, item, b"invalid sort order");
    }
    let sort_crit = SortCriteria::new(order, args.has(b'r') != 0);

    if args.has(b'n') != 0 {
        let mut sorted = Vec::new();
        sort::get_sessions(server, &sort_crit, &mut sorted);
        let next = server
            .clients
            .get(tc)
            .and_then(|c| c.session)
            .and_then(|cs| session::session_next_session(server, cs, &sorted));
        match next {
            Some(next) => s = next,
            None => return fail(server, item, b"can't find next session"),
        }
    } else if args.has(b'p') != 0 {
        let mut sorted = Vec::new();
        sort::get_sessions(server, &sort_crit, &mut sorted);
        let previous = server
            .clients
            .get(tc)
            .and_then(|c| c.session)
            .and_then(|cs| session::session_previous_session(server, cs, &sorted));
        match previous {
            Some(previous) => s = previous,
            None => return fail(server, item, b"can't find previous session"),
        }
    } else if args.has(b'l') != 0 {
        let last = server
            .clients
            .get(tc)
            .and_then(|c| c.last_session)
            .filter(|&ls| session::session_alive(server, ls));
        match last {
            Some(last) => s = last,
            None => return fail(server, item, b"can't find last session"),
        }
    } else {
        if c.is_none() {
            return CmdReturn::Normal;
        }
        if let (Some(wl), Some(wp)) = (wl, wp)
            && let Some(w) = server.winlinks.get(wl).map(|wl| wl.window)
            && server
                .windows
                .get(w)
                .is_some_and(|win| win.active != Some(wp))
        {
            let visible = if server
                .windows
                .get(w)
                .and_then(|w| w.modal)
                .is_some_and(|m| m != wp)
            {
                true
            } else {
                pane::pane_is_visible(server, wp)
            };
            if !visible && window::window_push_zoom(server, w, false, zflag).is_ok_and(|z| z) {
                server_redraw_window(server, w);
            }
            let _ = window::window_redraw_active_switch(server, w, Some(wp));
            let _ = window::window_set_active_pane(server, w, wp, true);
            if !visible && window::window_pop_zoom(server, w, true).is_ok_and(|z| z) {
                server_redraw_window(server, w);
            }
        }
        if let Some(wl) = wl {
            session::session_set_current(server, s, Some(wl));
            let current = find::from_session(server, s, CmdFindFlags::default());
            set_item_current(server, item, &current);
        }
    }

    if args.has(b'E') == 0 {
        update_session_environment(server, s, tc);
    }

    client::lifecycle::set_session(server, tc, Some(s));
    if !item_flags(server, item).contains(QueueStateFlags::REPEAT) {
        client::keys::set_key_table(server, tc, None);
    }

    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::is_pane_target;

    #[test]
    fn pane_target_classification() {
        assert!(is_pane_target(b"main:1"));
        assert!(is_pane_target(b"main:1.2"));
        assert!(is_pane_target(b".1"));
        assert!(is_pane_target(b"%5"));
        assert!(is_pane_target(b"="));
        assert!(is_pane_target(b"a:b\0ignored"));
        assert!(!is_pane_target(b"main"));
        assert!(!is_pane_target(b"=x"));
        assert!(!is_pane_target(b""));
        assert!(!is_pane_target(b"$3"));
        assert!(!is_pane_target(b"main\0:1"));
    }
}
