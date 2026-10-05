// Ported from tmux cmd-resize-window.c @ 8f25579c
use super::support::{concat, fail, item_target};
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::QueueItemId;
use crate::layout::{PANE_MINIMUM, WINDOW_MAXIMUM};
use crate::model::WindowSizePolicy;
use crate::model::resize::{
    ResizeWindow, WindowSize, default_window_size, recalculate_window_size,
};
use crate::server::Server;

/// `WINDOW_MINIMUM` (`tmux.h:116`) is `PANE_MINIMUM`.
const WINDOW_MINIMUM: u32 = PANE_MINIMUM;

/// cmd-resize-window.c:90-99: at most one direction, in `L R U D` precedence.
pub fn apply_direction(sx: u32, sy: u32, adjust: u32, flags: &[u8]) -> (u32, u32) {
    let has = |f: u8| flags.contains(&f);
    if has(b'L') {
        (if sx >= adjust { sx - adjust } else { sx }, sy)
    } else if has(b'R') {
        (sx.wrapping_add(adjust), sy)
    } else if has(b'U') {
        (sx, if sy >= adjust { sy - adjust } else { sy })
    } else if has(b'D') {
        (sx, sy.wrapping_add(adjust))
    } else {
        (sx, sy)
    }
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let Some(w) = target
        .wl
        .and_then(|wl| server.winlinks.get(wl))
        .map(|wl| wl.window)
    else {
        return fail(server, item, b"no current window");
    };
    let Some(s) = target.s else {
        return fail(server, item, b"no current session");
    };

    let adjust = if args.count() == 0 {
        1
    } else {
        match rmux_util::strtonum::strtonum(args.string(0).unwrap_or(b""), 1, i64::from(i32::MAX)) {
            Ok(n) => n as u32,
            Err(e) => {
                return fail(
                    server,
                    item,
                    concat(&[b"adjustment ", e.to_string().as_bytes()]),
                );
            }
        }
    };

    let Some((mut sx, mut sy)) = server.windows.get(w).map(|w| (w.sx, w.sy)) else {
        return fail(server, item, b"no current window");
    };
    if args.has(b'x') != 0 {
        sx = match args.strtonum(b'x', i64::from(WINDOW_MINIMUM), i64::from(WINDOW_MAXIMUM)) {
            Ok(n) => n as u32,
            Err(cause) => return fail(server, item, concat(&[b"width ", &cause])),
        };
    }
    if args.has(b'y') != 0 {
        sy = match args.strtonum(b'y', i64::from(WINDOW_MINIMUM), i64::from(WINDOW_MAXIMUM)) {
            Ok(n) => n as u32,
            Err(cause) => return fail(server, item, concat(&[b"height ", &cause])),
        };
    }

    let mut flags = Vec::with_capacity(4);
    for f in *b"LRUD" {
        if args.has(f) != 0 {
            flags.push(f);
        }
    }
    (sx, sy) = apply_direction(sx, sy, adjust, &flags);

    // cmd-resize-window.c:101-107
    let policy = if args.has(b'A') != 0 {
        Some(WindowSizePolicy::Largest)
    } else if args.has(b'a') != 0 {
        Some(WindowSizePolicy::Smallest)
    } else {
        None
    };
    if let Some(policy) = policy {
        let clients = crate::server::run::resize_clients(server);
        let window = server.windows.get(w).map(|win| ResizeWindow {
            id: w,
            manual: WindowSize {
                sx: win.manual_sx,
                sy: win.manual_sy,
                ..WindowSize::default()
            },
            latest: win.latest,
        });
        let default = server
            .options
            .get_string(server.options.global_s, b"default-size")
            .to_vec();
        let size = default_window_size(&clients, None, s, window, policy, &default);
        sx = size.sx;
        sy = size.sy;
    }

    // cmd-resize-window.c:109-112
    let Some(window) = server.windows.get_mut(w) else {
        return fail(server, item, b"no current window");
    };
    let options = window.options;
    window.manual_sx = sx;
    window.manual_sy = sy;
    server
        .options
        .set_number_value(options, b"window-size", WindowSizePolicy::Manual as i64);
    let clients = crate::server::run::resize_clients(server);
    let _ = recalculate_window_size(server, &clients, w, true);

    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::apply_direction;

    #[test]
    fn single_direction_in_lrud_precedence() {
        assert_eq!(apply_direction(80, 24, 5, b"L"), (75, 24));
        assert_eq!(apply_direction(80, 24, 5, b"R"), (85, 24));
        assert_eq!(apply_direction(80, 24, 5, b"U"), (80, 19));
        assert_eq!(apply_direction(80, 24, 5, b"D"), (80, 29));
        assert_eq!(apply_direction(80, 24, 5, b"LRUD"), (75, 24));
        assert_eq!(apply_direction(80, 24, 5, b"RUD"), (85, 24));
        assert_eq!(apply_direction(80, 24, 5, b"UD"), (80, 19));
        assert_eq!(apply_direction(80, 24, 5, b""), (80, 24));
    }

    #[test]
    fn subtract_only_when_at_least_adjust() {
        assert_eq!(apply_direction(4, 24, 5, b"L"), (4, 24));
        assert_eq!(apply_direction(5, 24, 5, b"L"), (0, 24));
        assert_eq!(apply_direction(80, 3, 5, b"U"), (80, 3));
    }
}
