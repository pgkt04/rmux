// Ported from tmux cmd-swap-window.c @ 8f25579c
use super::support::{fail, item_source, item_target};
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::QueueItemId;
use crate::model::session::{
    session_group_contains, session_group_synchronize_from, session_select,
};
use crate::server::Server;
use crate::server::operations::server_redraw_session_group;

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let source = item_source(server, item);
    let target = item_target(server, item);
    let (Some(src), Some(wl_src)) = (source.s, source.wl) else {
        return fail(server, item, b"no current session");
    };
    let (Some(dst), Some(wl_dst)) = (target.s, target.wl) else {
        return fail(server, item, b"no current session");
    };

    // cmd-swap-window.c:56-65
    let sg_src = session_group_contains(server, src);
    let sg_dst = session_group_contains(server, dst);
    if src != dst && sg_src.is_some() && sg_dst.is_some() && sg_src == sg_dst {
        return fail(server, item, b"can't move window, sessions are grouped");
    }

    let (Some(w_src), Some(w_dst)) = (
        server.winlinks.get(wl_src).map(|wl| wl.window),
        server.winlinks.get(wl_dst).map(|wl| wl.window),
    ) else {
        return fail(server, item, b"no current window");
    };
    if w_dst == w_src {
        return CmdReturn::Normal;
    }

    // cmd-swap-window.c:70-78: the winlinks keep their sessions, indexes and
    // window references; only the window side of each link changes.
    if let Some(w) = server.windows.get_mut(w_dst) {
        w.links.retain(|l| *l != wl_dst);
    }
    if let Some(w) = server.windows.get_mut(w_src) {
        w.links.retain(|l| *l != wl_src);
    }
    if let Some(wl) = server.winlinks.get_mut(wl_dst) {
        wl.window = w_src;
    }
    if let Some(w) = server.windows.get_mut(w_src) {
        w.links.push(wl_dst);
    }
    if let Some(wl) = server.winlinks.get_mut(wl_src) {
        wl.window = w_dst;
    }
    if let Some(w) = server.windows.get_mut(w_dst) {
        w.links.push(wl_src);
    }

    // cmd-swap-window.c:80-86
    if server.marked_winlink == Some(wl_src) {
        server.marked_winlink = Some(wl_dst);
    }
    if args.has(b'd') != 0 {
        let dst_index = server.winlinks.get(wl_dst).map_or(-1, |wl| wl.index);
        session_select(server, dst, dst_index);
        if src != dst {
            let src_index = server.winlinks.get(wl_src).map_or(-1, |wl| wl.index);
            session_select(server, src, src_index);
        }
    }

    // cmd-swap-window.c:87-93
    session_group_synchronize_from(server, src);
    server_redraw_session_group(server, src);
    if src != dst {
        session_group_synchronize_from(server, dst);
        server_redraw_session_group(server, dst);
    }
    crate::server::run::recalculate_sizes(server);

    CmdReturn::Normal
}
