// Ported from tmux cmd-display-menu.c @ 8f25579c
use super::support::{concat, fail, item_event, item_target, item_target_client};
use crate::client::ClientFlags;
use crate::cmd::{
    Command,
    arguments::Args,
    find::{self, CmdFindFlags},
    queue::CmdReturn,
};
use crate::format;
use crate::ids::{OptionsId, QueueItemId};
use crate::layout::{self, LayoutGeometry};
use crate::model::{
    PaneFlags,
    spawn::{self, SpawnContext, SpawnFlags},
    window,
};
use crate::options::environment::EnvironmentFlags;
use crate::server::{
    Server,
    events::{self, EventPayload},
    operations,
};
use crate::ui::{menu, status};
use rmux_emu::{
    cell::DEFAULT_CELL,
    hyperlinks::HyperlinkRegistry,
    screen::BoxLines,
    style::{Style, StyleRangeType},
};
use rmux_tty::key_string::parse_key_name;

/// strtol(..., NULL, 10): whitespace, optional sign and a decimal prefix.
fn decimal_prefix(bytes: &[u8]) -> i64 {
    let mut i = 0;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    let negative = bytes.get(i) == Some(&b'-');
    if negative || bytes.get(i) == Some(&b'+') {
        i += 1;
    }
    let mut value = 0i64;
    while let Some(b) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
        let digit = i64::from(*b - b'0');
        value = if negative {
            value.saturating_mul(10).saturating_sub(digit)
        } else {
            value.saturating_mul(10).saturating_add(digit)
        };
        i += 1;
    }
    value
}

fn clamp_position(x: i64, y: i64, width: u32, height: u32, sx: u32, sy: u32) -> (u32, u32) {
    let h = i64::from(height);
    let x = x.clamp(0, i64::from(sx.saturating_sub(width)));
    let y = if y < h { 0 } else { y - h }.clamp(0, i64::from(sy.saturating_sub(height)));
    (x as u32, y as u32)
}

fn position(
    server: &mut Server,
    item: QueueItemId,
    args: &Args,
    width: u32,
    height: u32,
) -> (u32, u32) {
    let fs = item_target(server, item);
    let tc = item_target_client(server, item).expect("resolved target client");
    let w = server
        .windows
        .get(fs.w.expect("resolved window"))
        .expect("target window");
    let p = server
        .panes
        .get(fs.wp.expect("resolved pane"))
        .expect("target pane");
    let (sx, sy, last_x, last_y, px, py, psx, psy) = (
        w.sx,
        w.sy,
        w.menu_last_px,
        w.menu_last_py,
        p.xoff,
        p.yoff,
        p.sx,
        p.sy,
    );
    let (_, ox, oy, _, viewport_height) = crate::client::lifecycle::window_offset(server, tc);
    let mouse = item_event(server, item).mouse;
    let max_y = i64::from(sy.saturating_sub(height));
    let h = i64::from(height);
    let ww = i64::from(width);
    let mut values: Vec<(&[u8], i64)> = vec![
        (b"popup_width", ww),
        (b"popup_height", h),
        (b"popup_last_x", i64::from(last_x)),
        (b"popup_last_y", i64::from(last_y.wrapping_add(height))),
        (b"popup_centre_x", ((i64::from(sx) - 1) / 2 - ww / 2).max(0)),
        (b"popup_centre_y", {
            let n = (i64::from(sy) - 1) / 2 + h / 2;
            if n >= i64::from(sy) { max_y } else { n }
        }),
        (b"popup_pane_top", {
            let n = i64::from(py) + h;
            if n >= i64::from(sy) { max_y } else { n }
        }),
        (
            b"popup_pane_bottom",
            i64::from((py as u32).wrapping_add(psy)),
        ),
        (b"popup_pane_left", i64::from(px)),
        (
            b"popup_pane_right",
            (i64::from(px) + i64::from(psx) - ww).max(0),
        ),
    ];
    if mouse.valid {
        let mx = i64::from(mouse.x.wrapping_add(ox));
        let my = if mouse.status_at == 0 {
            i64::from(oy) + i64::from(mouse.y.saturating_sub(mouse.status_lines))
        } else if mouse.status_at > 0 && mouse.y >= mouse.status_at as u32 {
            i64::from(oy) + i64::from(viewport_height) - 1
        } else {
            i64::from(mouse.y) + i64::from(oy)
        };
        let centre = my - h / 2;
        values.extend([
            (b"popup_mouse_x" as &[u8], mx),
            (b"popup_mouse_y", my),
            (b"popup_mouse_centre_x", (mx - ww / 2).max(0)),
            (
                b"popup_mouse_centre_y",
                if centre + h >= i64::from(sy) {
                    max_y
                } else {
                    centre
                },
            ),
            (
                b"popup_mouse_top",
                if my + h >= i64::from(sy) {
                    i64::from(sy) - 1
                } else {
                    my + h
                },
            ),
            (b"popup_mouse_bottom", (my - h).max(0)),
        ]);
    }
    let lines = status::status_line_size(server, tc);
    if status::status_at_line(server, tc) != -1 && lines != 0 {
        let c = server.clients.get(tc).expect("target client");
        let session = c.session.expect("attached target client");
        let top = server.options.get_number(
            server
                .sessions
                .get(session)
                .expect("client session")
                .options,
            b"status-position",
        ) == 0;
        let y = if top { h } else { i64::from(sy) };
        values.push((b"popup_status_line_y", y));
        let index = fs
            .wl
            .and_then(|wl| server.winlinks.get(wl))
            .map(|wl| wl.index as u32);
        if let Some(range) = c
            .status
            .entries
            .iter()
            .take(lines as usize)
            .flat_map(|entry| &entry.ranges.0)
            .find(|range| {
                range.range_type == StyleRangeType::Window && Some(range.argument) == index
            })
        {
            values.push((
                b"popup_window_status_line_x",
                i64::from(range.start.wrapping_add(ox)),
            ));
            values.push((b"popup_window_status_line_y", y));
        }
    }
    let mut ft = format::create_from_target(server, item);
    for (name, value) in values {
        ft.add(name, value.to_string().into());
    }
    let x: &[u8] = match args.get(b'x') {
        None | Some(b"C") => b"#{popup_centre_x}",
        Some(b"R") => b"#{popup_pane_right}",
        Some(b"P") => b"#{popup_pane_left}",
        Some(b"M") => b"#{popup_mouse_centre_x}",
        Some(b"L") => b"#{popup_last_x}",
        Some(b"W") => b"#{popup_window_status_line_x}",
        Some(x) => x,
    };
    let y: &[u8] = match args.get(b'y') {
        None | Some(b"C") => b"#{popup_centre_y}",
        Some(b"P") => b"#{popup_pane_bottom}",
        Some(b"M") => b"#{popup_mouse_top}",
        Some(b"L") => b"#{popup_last_y}",
        Some(b"S") => b"#{popup_status_line_y}",
        Some(b"W") => b"#{popup_window_status_line_y}",
        Some(y) => y,
    };
    let x = decimal_prefix(&ft.expand(server, x));
    let y = decimal_prefix(&ft.expand(server, y));
    ft.release(server);
    clamp_position(x, y, width, height, sx, sy)
}

pub fn execute_menu(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let fs = item_target(server, item);
    let Some(tc) = item_target_client(server, item) else {
        return CmdReturn::Normal;
    };
    let choice = if args.get(b'C') == Some(b"-".as_slice()) {
        -1
    } else if args.has(b'C') != 0 {
        match args.strtonum(b'C', 0, i64::from(u32::MAX)) {
            Ok(n) => n as i32,
            Err(cause) => return fail(server, item, concat(&[b"starting choice ", &cause])),
        }
    } else {
        0
    };
    let title = args
        .get(b'T')
        .map(|title| format::single_from_target(server, item, title))
        .unwrap_or_default();
    let mut menu = menu::menu_create(&title);
    let mut i = 0;
    while i < args.count() {
        let name = args.string(i).unwrap_or_default();
        i += 1;
        if name.is_empty() {
            menu::menu_add_item(server, &mut menu, None, Some(item), tc, Some(&fs));
            continue;
        }
        if args.count() - i < 2 {
            return fail(server, item, b"not enough arguments");
        }
        let entry = menu::MenuItem {
            name: Some(name.into()),
            key: parse_key_name(args.string(i).unwrap_or_default()),
            command: Some(args.string(i + 1).unwrap_or_default().into()),
        };
        i += 2;
        menu::menu_add_item(server, &mut menu, Some(&entry), Some(item), tc, Some(&fs));
    }
    if menu.count() == 0 {
        return CmdReturn::Normal;
    }
    let o = server
        .windows
        .get(fs.w.expect("resolved window"))
        .expect("target window")
        .options;
    let lines = match args.get(b'b') {
        Some(value) => match crate::options::find_choice(
            crate::options::search(b"menu-border-lines").expect("menu option"),
            value,
        ) {
            Ok(lines) => lines,
            Err(cause) => {
                return fail(
                    server,
                    item,
                    concat(&[b"menu-border-lines ", cause.as_bytes()]),
                );
            }
        },
        None => server.options.get_number(o, b"menu-border-lines"),
    };
    let lines = BoxLines::try_from(lines as i32).expect("menu border choice");
    let (sx, sy) = menu::menu_get_size(&menu, lines);
    let (px, py) = position(server, item, args, sx, sy);
    let mut flags = menu::MenuFlags::default();
    if args.has(b'O') != 0 {
        flags = flags | menu::MenuFlags::STAYOPEN;
    }
    if !item_event(server, item).mouse.valid && args.has(b'M') == 0 {
        flags = flags | menu::MenuFlags::NOMOUSE;
    }
    menu::menu_display(
        server,
        menu,
        flags,
        choice,
        Some(item),
        px,
        py,
        tc,
        lines,
        args.get(b's'),
        args.get(b'H'),
        args.get(b'S'),
        Some(&fs),
        None,
    );
    CmdReturn::Normal
}

fn set_string(server: &mut Server, o: OptionsId, name: &[u8], value: &[u8]) {
    let mut options = std::mem::take(&mut server.options);
    options.set_string(o, name, false, value, server);
    server.options = options;
}

fn exit_policy(e: usize, keep: bool) -> i64 {
    match (e, keep) {
        (0, false) => 1,
        (0, true) => 3,
        (1, _) => 0,
        (_, false) => 2,
        (_, true) => 4,
    }
}

pub fn execute_popup(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let fs = item_target(server, item);
    let (Some(s), Some(w), Some(wl), Some(wp), Some(tc)) =
        (fs.s, fs.w, fs.wl, fs.wp, item_target_client(server, item))
    else {
        return CmdReturn::Normal;
    };
    let win = server.windows.get(w).expect("target window");
    let (sx, sy, modal) = (win.sx, win.sy, win.modal);
    if args.has(b'C') != 0 {
        if let Some(p) = modal {
            let _ = operations::server_kill_pane(server, p);
        }
        return CmdReturn::Normal;
    }
    if modal.is_some()
        || server
            .clients
            .get(tc)
            .is_some_and(|c| c.flags.contains(ClientFlags::CONTROL))
    {
        return CmdReturn::Normal;
    }
    let value = match args.get(b'b') {
        None | Some(b"rounded") => b"single".as_slice(),
        Some(b"padded") => b"spaces",
        Some(value) => value,
    };
    let lines = if args.has(b'B') != 0 {
        6
    } else {
        match crate::options::find_choice(
            crate::options::search(b"pane-border-lines").expect("pane border option"),
            value,
        ) {
            Ok(n) => n,
            Err(cause) => {
                return fail(
                    server,
                    item,
                    concat(&[b"pane-border-lines ", cause.as_bytes()]),
                );
            }
        }
    };
    let dimension = |flag, size| {
        if args.has(flag) != 0 {
            args.percentage(flag, 1, i64::from(size), i64::from(size))
                .map(|n| n as u32)
        } else {
            Ok(size / 2)
        }
    };
    let height = match dimension(b'h', sy) {
        Ok(n) => n.min(sy),
        Err(cause) => return fail(server, item, concat(&[b"height ", &cause])),
    };
    let width = match dimension(b'w', sx) {
        Ok(n) => n.min(sx),
        Err(cause) => return fail(server, item, concat(&[b"width ", &cause])),
    };
    let minimum = if lines == 6 { 1 } else { 3 };
    if width < minimum || height < minimum {
        return CmdReturn::Normal;
    }
    let (px, py) = position(server, item, args, width, height);
    let border = u32::from(lines != 6);
    let geometry = LayoutGeometry {
        sx: width - 2 * border,
        sy: height - 2 * border,
        xoff: (px + border) as i32,
        yoff: (py + border) as i32,
    };
    let _ = window::window_push_zoom(server, w, false, true);
    let lc = layout::floating_pane(server, w, Some(wp), &geometry);
    let mut sc = SpawnContext::new(s);
    sc.item = Some(item);
    sc.winlink = Some(wl);
    sc.client = Some(tc);
    sc.pane = Some(wp);
    sc.layout_cell = Some(lc);
    sc.cwd = args.get(b'd').map(<[u8]>::to_vec);
    sc.flags = SpawnFlags::FLOATING | SpawnFlags::MODAL | SpawnFlags::FLOATOVERZOOM;
    if args.count() != 1 || args.string(0) != Some(b"".as_slice()) {
        sc.argv = args
            .values()
            .iter()
            .map(|v| v.as_string().to_vec())
            .collect();
    }
    for value in args.values_of(b'e') {
        sc.environment
            .put(value.as_string(), EnvironmentFlags::default());
    }
    let result = spawn::spawn_pane(server, &mut sc);
    let _ = window::window_pop_zoom(server, w, true);
    let pane = match result {
        Ok(pane) => pane,
        Err(cause) => {
            return fail(
                server,
                item,
                concat(&[
                    b"create pane failed: ",
                    &super::split_window::spawn_cause(&cause),
                ]),
            );
        }
    };
    let p = server.panes.get_mut(pane).expect("spawned pane");
    p.flags.insert(PaneFlags::CAPTUREALLKEYS);
    if args.has(b'E') == 0 {
        p.flags.insert(PaneFlags::CLOSEONCANCEL);
    }
    let o = p.options;
    server
        .options
        .set_number_value(o, b"pane-border-lines", lines);
    server.options.set_number_value(
        o,
        b"remain-on-exit",
        exit_policy(args.has(b'E') as usize, args.has(b'k') != 0),
    );
    set_string(server, o, b"remain-on-exit-format", b"");
    for (flag, names, diagnostic) in [
        (
            b's',
            [
                b"window-style".as_slice(),
                b"window-active-style".as_slice(),
            ],
            b"bad style: ".as_slice(),
        ),
        (
            b'S',
            [
                b"pane-border-style".as_slice(),
                b"pane-active-border-style".as_slice(),
            ],
            b"bad border style: ".as_slice(),
        ),
    ] {
        if let Some(value) = args.get(flag) {
            if Style::default()
                .parse(&DEFAULT_CELL, value, &mut HyperlinkRegistry::new())
                .is_err()
            {
                crate::client::mouse::remove_pane(server, pane);
                layout::close_pane(server, pane);
                let _ = window::window_remove_pane(server, w, pane);
                return fail(server, item, concat(&[diagnostic, value]));
            }
            for name in names {
                set_string(server, o, name, value);
            }
            if flag == b's' {
                server
                    .panes
                    .get_mut(pane)
                    .expect("pane")
                    .flags
                    .insert(PaneFlags::REDRAW | PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED);
            }
        }
    }
    if let Some(title) = args.get(b'T') {
        let title = format::single_from_target(server, item, title);
        server.options.set_number_value(o, b"pane-border-status", 1);
        set_string(server, o, b"pane-border-format", b"#{pane_title}");
        server
            .panes
            .get_mut(pane)
            .expect("pane")
            .base
            .set_title(&title, false);
        let target = find::from_pane(server, pane, CmdFindFlags::default()).unwrap_or_default();
        let mut ep = EventPayload::new();
        ep.set_target(server, &target);
        ep.set_pane(server, b"pane", pane);
        ep.set_window(server, b"window", w);
        ep.set_string(server, b"new_title", &title);
        events::fire(server, b"pane-title-changed", ep);
    }
    server.panes.get_mut(pane).expect("pane").wait_item = Some(item);
    operations::server_redraw_session(server, s);
    CmdReturn::Wait
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lower_edge_coordinates_and_window_clamping() {
        assert_eq!(clamp_position(30, 20, 20, 10, 80, 24), (30, 10));
        assert_eq!(clamp_position(-1, 9, 20, 10, 80, 24), (0, 0));
        assert_eq!(clamp_position(200, 200, 20, 10, 80, 24), (60, 14));
        assert_eq!(clamp_position(10, 10, 100, 30, 80, 24), (0, 0));
    }
    #[test]
    fn positions_use_c_decimal_prefix() {
        for (input, expected) in [
            (b"  +12junk".as_slice(), 12),
            (b"-8.2", -8),
            (b"junk", 0),
            (b"+", 0),
            (b"0x20", 0),
            (b"92233720368547758080", i64::MAX),
            (b"-92233720368547758080", i64::MIN),
        ] {
            assert_eq!(decimal_prefix(input), expected);
        }
    }
    #[test]
    fn all_popup_exit_policies() {
        assert_eq!((exit_policy(0, false), exit_policy(0, true)), (1, 3));
        assert_eq!((exit_policy(1, false), exit_policy(1, true)), (0, 0));
        assert_eq!((exit_policy(2, false), exit_policy(2, true)), (2, 4));
        assert_eq!((exit_policy(3, false), exit_policy(3, true)), (2, 4));
    }
}
