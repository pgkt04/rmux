// Ported from tmux cmd-break-pane.c @ 8f25579c
/*
 * Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
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
use super::support::{
    concat, fail, item_source, item_target, item_target_client, set_item_current,
};
use crate::cmd::{
    Command,
    find::{self, CmdFindFlags},
    queue::{self, CmdReturn},
};
use crate::format::{self, FormatContext};
use crate::ids::{PaneId, QueueItemId, WindowId};
use crate::layout::{self, LayoutCellFlags};
use crate::model::{
    PaneFlags, WindowFlags, names, pane, session, state::clean_name, window, winlink,
};
use crate::server::{Server, events, operations};
use rmux_emu::screen::PaneLines;

fn float(
    server: &mut Server,
    command: &Command,
    item: QueueItemId,
    w: WindowId,
    wp: PaneId,
) -> CmdReturn {
    if pane::pane_is_floating(server, wp) {
        return fail(server, item, b"pane is already floating");
    }
    if server
        .windows
        .get(w)
        .is_some_and(|w| w.flags.contains(WindowFlags::ZOOMED))
    {
        return fail(server, item, b"can't float a pane while window is zoomed");
    }
    let lc = server
        .panes
        .get(wp)
        .and_then(|p| p.layout_cell)
        .expect("resolved pane layout");
    let mut geometry = server.layout_cells.get(lc).expect("pane layout cell").fg;
    let lines = PaneLines::try_from(pane::pane_get_pane_lines(server, wp) as i32)
        .unwrap_or(PaneLines::Single);
    if let Err(error) =
        layout::floating_args_parse(server, item, &command.args, lines, w, &mut geometry)
    {
        return fail(
            server,
            item,
            concat(&[b"failed to float pane: ", &error.cause]),
        );
    }
    server
        .layout_cells
        .get_mut(lc)
        .expect("pane layout cell")
        .fg = geometry;
    layout::remove_tile(server, w, lc);
    layout::set_size(
        &mut server.layout_cells,
        lc,
        geometry.sx,
        geometry.sy,
        geometry.xoff,
        geometry.yoff,
    );
    server
        .layout_cells
        .get_mut(lc)
        .expect("pane layout cell")
        .flags
        .insert(LayoutCellFlags::FLOATING);
    let window = server.windows.get_mut(w).expect("resolved window");
    window.z_order.retain(|p| *p != wp);
    window.z_order.insert(0, wp);
    if command.args.has(b'd') == 0 {
        let _ = window::window_set_active_pane(server, w, wp, true);
    }
    layout::fix_offsets(server, w);
    layout::fix_panes(server, w, None);
    events::fire_window(server, b"window-layout-changed", w);
    operations::server_redraw_window(server, w);
    CmdReturn::Normal
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let source = item_source(server, item);
    let target = item_target(server, item);
    let (Some(src_s), Some(src_wl), Some(wp), Some(dst_s)) =
        (source.s, source.wl, source.wp, target.s)
    else {
        return CmdReturn::Error;
    };
    let Some((old_w, old_idx)) = server.winlinks.get(src_wl).map(|wl| (wl.window, wl.index)) else {
        return CmdReturn::Error;
    };
    if server
        .windows
        .get(old_w)
        .is_some_and(|w| w.modal == Some(wp))
    {
        return fail(server, item, b"pane is modal");
    }
    if args.has(b'W') != 0 {
        return float(server, command, item, old_w, wp);
    }
    let name = args.get(b'n');
    if let Some(name) = name
        && !rmux_util::utf8::is_valid(name)
    {
        return fail(server, item, concat(&[b"invalid window name: ", name]));
    }
    let mut index = target.idx;
    if args.has(b'a') != 0 || args.has(b'b') != 0 {
        let link = target
            .wl
            .or_else(|| server.sessions.get(dst_s).and_then(|s| s.current));
        let Some(shuffled) = winlink::winlink_shuffle_up(server, dst_s, link, args.has(b'b') != 0)
        else {
            return CmdReturn::Error;
        };
        index = shuffled;
    }
    let _ = operations::server_unzoom_window(server, old_w);
    let detached = args.has(b'd') != 0;
    let wl = if window::window_count_panes(server, old_w, true) == 1 {
        if let Err(cause) =
            operations::server_link_window(server, src_s, src_wl, dst_s, index, false, !detached)
        {
            return fail(server, item, cause);
        }
        if let Some(name) = name {
            let _ = window::window_set_name(server, old_w, name, false);
            let options = server.windows.get(old_w).expect("source window").options;
            server
                .options
                .set_number_value(options, b"automatic-rename", 0);
        }
        let _ = operations::server_unlink_window(server, src_s, src_wl);
        let Some(wl) = winlink::winlink_find_by_window(server, dst_s, old_w) else {
            return CmdReturn::Error;
        };
        let index = server.winlinks.get(wl).expect("destination link").index;
        window::window_fire_pane_moved(server, wp, old_w, old_idx, old_w, index);
        wl
    } else {
        if index != -1
            && server
                .sessions
                .get(dst_s)
                .is_some_and(|s| s.windows.contains_key(&index))
        {
            return fail(server, item, format!("index in use: {index}"));
        }
        let old = server.windows.get(old_w).expect("source window");
        let (sx, sy, xpixel, ypixel) = (old.sx, old.sy, old.xpixel, old.ypixel);
        crate::client::mouse::remove_pane(server, wp);
        // The model chooses the replacement from the still-linked neighbour.
        let _ = window::window_lost_pane(server, old_w, wp);
        let old = server.windows.get_mut(old_w).expect("source window");
        old.panes.retain(|p| *p != wp);
        old.z_order.retain(|p| *p != wp);
        layout::close_pane(server, wp);
        let w =
            window::window_create(server, sx, sy, xpixel, ypixel).expect("new window allocation");
        window::window_retain(server, w).expect("new window lease");
        let options = server.windows.get(w).expect("new window").options;
        let pane = server.panes.get_mut(wp).expect("source pane");
        pane.window = w;
        pane.flags
            .insert(PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
        server.options.set_parent(pane.options, Some(options));
        let latest = item_target_client(server, item);
        let new = server.windows.get_mut(w).expect("new window");
        new.panes.push(wp);
        new.z_order.push(wp);
        new.active = Some(wp);
        new.latest = latest;
        let window_name = name
            .map(|name| clean_name(name, false).expect("validated window name"))
            .unwrap_or_else(|| names::default_window_name(server, w));
        server.windows.get_mut(w).expect("new window").name = window_name;
        if name.is_some() {
            server
                .options
                .set_number_value(options, b"automatic-rename", 0);
        }
        crate::ui::border::window_set_fill_cells(server, w);
        if index == -1 {
            let options = server
                .sessions
                .get(dst_s)
                .expect("destination session")
                .options;
            index = -1 - server.options.get_number(options, b"base-index") as i32;
        }
        let wl =
            session::session_attach(server, dst_s, w, index).expect("available destination index");
        layout::init(server, w, wp);
        server
            .panes
            .get_mut(wp)
            .expect("source pane")
            .flags
            .insert(PaneFlags::CHANGED);
        super::swap_pane::palette_from_option(server, wp);
        window::window_release(server, w).expect("attached window lease");
        events::fire_window(server, b"window-created", w);
        let index = server.winlinks.get(wl).expect("destination link").index;
        window::window_fire_pane_moved(server, wp, old_w, old_idx, w, index);
        if !detached {
            session::session_select(server, dst_s, index);
            let current = find::from_session(server, dst_s, CmdFindFlags::default());
            set_item_current(server, item, &current);
        }
        operations::server_redraw_session(server, src_s);
        if src_s != dst_s {
            operations::server_redraw_session(server, dst_s);
        }
        operations::server_status_session_group(server, src_s);
        if src_s != dst_s {
            operations::server_status_session_group(server, dst_s);
        }
        wl
    };
    if args.has(b'P') != 0 {
        let w = server.winlinks.get(wl).expect("destination link").window;
        let evaluated_client = item_target_client(server, item);
        let output = format::single(
            server,
            Some(item),
            FormatContext {
                evaluated_client,
                session: Some(dst_s),
                winlink: Some(wl),
                window: Some(w),
                pane: Some(wp),
                ..FormatContext::default()
            },
            args.get(b'F')
                .unwrap_or(b"#{session_name}:#{window_index}.#{pane_index}"),
        );
        queue::print(server, item, &output);
    }
    CmdReturn::Normal
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cmd::{
        CommandEntry,
        arguments::{Args, ArgsEntryFlags, ArgsValue},
        metadata,
    };
    use crate::ids::{SessionId, WinlinkId};
    use crate::model::{session::SessionCreate, spawn::SpawnFlags};
    use crate::options::environment::Environment;

    pub(crate) fn command(entry: &'static CommandEntry, flags: &[(u8, Option<&[u8]>)]) -> Command {
        let mut args = Args::default();
        for (flag, value) in flags {
            args.set(
                *flag,
                value.map(|value| ArgsValue::string(value.into())),
                ArgsEntryFlags::default(),
            );
        }
        Command {
            entry,
            args,
            group: 0,
            file: None,
            line: 0,
            parse_flags: Default::default(),
        }
    }

    pub(crate) fn create_session(server: &mut Server, name: &[u8]) -> SessionId {
        let options = server.options.create(Some(server.options.global_s));
        session::session_create(
            server,
            SessionCreate {
                name: Some(name.to_vec()),
                prefix: None,
                cwd: b"/tmp".to_vec(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        )
    }

    pub(crate) fn create_window(
        server: &mut Server,
        s: SessionId,
        index: i32,
    ) -> (WindowId, WinlinkId, PaneId) {
        let w = window::window_create(server, 80, 24, 12, 20).unwrap();
        let p = window::window_add_pane(server, w, None, 10, SpawnFlags::default()).unwrap();
        window::window_set_active_pane(server, w, p, false).unwrap();
        layout::init(server, w, p);
        let wl = session::session_attach(server, s, w, index).unwrap();
        session::session_select(server, s, index);
        (w, wl, p)
    }

    pub(crate) fn split(server: &mut Server, w: WindowId, anchor: PaneId) -> PaneId {
        let lc = layout::split_pane(
            server,
            anchor,
            layout::LayoutType::Topbottom,
            -1,
            SpawnFlags::default(),
        )
        .unwrap();
        let p =
            window::window_add_pane(server, w, Some(anchor), 10, SpawnFlags::default()).unwrap();
        layout::assign_pane(server, lc, p, false);
        p
    }

    pub(crate) fn item(
        server: &mut Server,
        source: find::CmdFindState,
        target: find::CmdFindState,
    ) -> QueueItemId {
        let batch = server
            .queue
            .get_callback("pane-move-test", Box::new(|_, _| CmdReturn::Normal))
            .unwrap();
        let item = batch.items[0];
        let queued = server.queue.items.get_mut(item).unwrap();
        queued.source = source;
        queued.target = target;
        let state = queued.state;
        server.queue.states.get_mut(state).unwrap().current = source;
        item
    }

    pub(crate) fn source(server: &Server, wl: WinlinkId, p: PaneId) -> find::CmdFindState {
        let mut state = find::from_winlink(server, wl, CmdFindFlags::default());
        state.wp = Some(p);
        state
    }

    fn fixture() -> (Server, SessionId, WindowId, WinlinkId, PaneId) {
        let mut server = Server::default();
        let s = create_session(&mut server, b"source");
        let (w, wl, p) = create_window(&mut server, s, 0);
        (server, s, w, wl, p)
    }

    #[test]
    fn several_panes_break_without_spawning_and_reparent_options() {
        let (mut server, s, w, wl, p) = fixture();
        let other = split(&mut server, w, p);
        window::window_set_active_pane(&mut server, w, other, false).unwrap();
        let current = source(&server, wl, other);
        let target = find::CmdFindState {
            s: Some(s),
            idx: 4,
            ..Default::default()
        };
        let item = item(&mut server, current, target);
        let pid = server.panes.get(other).unwrap().pid;
        let count = server.pane_ids.len();
        let command = command(&metadata::CMD_BREAK_PANE, &[(b'n', Some(b"#{literal}"))]);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        let new_w = server.panes.get(other).unwrap().window;
        assert_ne!(w, new_w);
        assert_eq!(server.pane_ids.len(), count);
        assert_eq!(server.panes.get(other).unwrap().pid, pid);
        assert_eq!(server.windows.get(w).unwrap().panes, [p]);
        assert_eq!(server.windows.get(w).unwrap().active, Some(p));
        let new = server.windows.get(new_w).unwrap();
        assert_eq!((new.sx, new.sy, new.xpixel, new.ypixel), (80, 24, 12, 20));
        assert_eq!(new.name, b"#{literal}");
        assert_eq!(new.panes, [other]);
        assert_eq!(new.z_order, [other]);
        assert_eq!(
            server.options.get_number(new.options, b"automatic-rename"),
            0
        );
        let queued = server.queue.items.get(item).unwrap();
        assert_eq!(
            server.queue.states.get(queued.state).unwrap().current.wp,
            Some(other)
        );
        let selected = server
            .queue
            .states
            .get(queued.state)
            .unwrap()
            .current
            .wl
            .unwrap();
        assert_eq!(server.winlinks.get(selected).unwrap().index, 4);
    }

    #[test]
    fn occupied_index_rejects_before_pane_removal() {
        let (mut server, _, w, wl, p) = fixture();
        split(&mut server, w, p);
        let current = source(&server, wl, p);
        let target = find::CmdFindState { idx: 0, ..current };
        let item = item(&mut server, current, target);
        let command = command(&metadata::CMD_BREAK_PANE, &[]);
        let panes = server.windows.get(w).unwrap().panes.clone();
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"index in use: 0"
        );
        assert_eq!(server.windows.get(w).unwrap().panes, panes);
    }

    #[test]
    fn single_pane_relinks_existing_window_and_leaves_current_target_unchanged() {
        let (mut server, _, w, wl, p) = fixture();
        let dst = create_session(&mut server, b"destination");
        let current = source(&server, wl, p);
        let target = find::CmdFindState {
            s: Some(dst),
            idx: 2,
            ..Default::default()
        };
        let item = item(&mut server, current, target);
        let command = command(&metadata::CMD_BREAK_PANE, &[(b'n', Some(b"renamed"))]);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert_eq!(server.panes.get(p).unwrap().window, w);
        assert!(server.winlinks.get(wl).is_none());
        assert_eq!(server.windows.get(w).unwrap().name, b"renamed");
        let new_wl = server.sessions.get(dst).unwrap().windows[&2];
        assert_eq!(server.winlinks.get(new_wl).unwrap().window, w);
        let queued = server.queue.items.get(item).unwrap();
        assert_eq!(
            server.queue.states.get(queued.state).unwrap().current,
            current
        );
    }

    #[test]
    fn detached_break_uses_destination_base_index_without_selecting() {
        let (mut server, s, w, wl, p) = fixture();
        split(&mut server, w, p);
        let options = server.sessions.get(s).unwrap().options;
        server.options.set_number_value(options, b"base-index", 3);
        let current = source(&server, wl, p);
        let target = find::CmdFindState {
            s: Some(s),
            idx: -1,
            ..Default::default()
        };
        let item = item(&mut server, current, target);
        let command = command(&metadata::CMD_BREAK_PANE, &[(b'd', None)]);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert!(server.sessions.get(s).unwrap().windows.contains_key(&3));
        assert_eq!(server.sessions.get(s).unwrap().current, Some(wl));
        let queued = server.queue.items.get(item).unwrap();
        assert_eq!(
            server.queue.states.get(queued.state).unwrap().current,
            current
        );
    }

    #[test]
    fn floating_bypasses_name_validation_and_window_creation() {
        let (mut server, _, w, wl, p) = fixture();
        let current = source(&server, wl, p);
        let item = item(&mut server, current, current);
        let command = command(
            &metadata::CMD_BREAK_PANE,
            &[
                (b'W', None),
                (b'n', Some(b"\xff")),
                (b'X', Some(b"10")),
                (b'Y', Some(b"5")),
            ],
        );
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Normal);
        assert_eq!(server.panes.get(p).unwrap().window, w);
        assert_eq!(server.window_ids.len(), 1);
        assert!(pane::pane_is_floating(&server, p));
        let lc = server.panes.get(p).unwrap().layout_cell.unwrap();
        let g = server.layout_cells.get(lc).unwrap().g;
        assert_eq!((g.xoff, g.yoff), (11, 6));
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"pane is already floating"
        );
    }

    #[test]
    fn modal_and_zoom_errors_precede_float_geometry_errors() {
        let (mut server, _, w, wl, p) = fixture();
        let current = source(&server, wl, p);
        let item = item(&mut server, current, current);
        let command = command(
            &metadata::CMD_BREAK_PANE,
            &[(b'W', None), (b'x', Some(b"invalid"))],
        );
        server.windows.get_mut(w).unwrap().modal = Some(p);
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"pane is modal"
        );
        server.windows.get_mut(w).unwrap().modal = None;
        split(&mut server, w, p);
        assert!(window::window_zoom(&mut server, w, p).unwrap());
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"can't float a pane while window is zoomed"
        );
        operations::server_unzoom_window(&mut server, w).unwrap();
        assert_eq!(execute(&mut server, &command, item), CmdReturn::Error);
        assert_eq!(
            server.cfg.causes.last().unwrap().as_bytes(),
            b"failed to float pane: position invalid"
        );
        assert!(!pane::pane_is_floating(&server, p));
    }
}
