// Ported from tmux cmd-refresh-client.c @ 8f25579c
use super::support::{fail, item_target_client};
use crate::client::ClientFlags;
use crate::cmd::Command;
use crate::cmd::queue::CmdReturn;
use crate::ids::{ClientId, QueueItemId};
use crate::model::PaneFlags;
use crate::model::monitor::monitor_parse;
use crate::model::pane::pane_find_by_public_id;
use crate::server::Server;
use crate::server::operations::{server_redraw_client, server_status_client};
use rmux_tty::keys::{ColourTarget, Recognition, parse_colour_response};
use rmux_tty::tty::TtyFlags;
use rmux_util::strtonum::strtonum;

/// `WINDOW_MINIMUM` (`tmux.h:115`) is `PANE_MINIMUM`.
const WINDOW_MINIMUM: u32 = crate::layout::PANE_MINIMUM;
const WINDOW_MAXIMUM: u32 = crate::layout::WINDOW_MAXIMUM;

/// The branch `cmd_refresh_client_control_client_size` selects
/// (`cmd-refresh-client.c:70-104`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeAction {
    /// `@%u:%ux%u` converted three values.
    WindowSize {
        window: u32,
        width: u32,
        height: u32,
    },
    /// `@%u:` converted the window id (the colon need not be present).
    ClearWindow { window: u32 },
    /// `%u,%u` or `%ux%u` converted two values.
    ClientSize { width: u32, height: u32 },
}

/// `sscanf` cursor: `%u` skips whitespace and takes an optional sign, then
/// digits; literals match one byte without skipping anything.
struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
}
impl Scanner<'_> {
    fn literal(&mut self, c: u8) -> Option<()> {
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Some(())
        } else {
            None
        }
    }
    /// `%u`: `strtoul` into `unsigned int` (negative wraps, overflow
    /// saturates to `ULONG_MAX` before truncation).
    fn unsigned(&mut self) -> Option<u32> {
        let s = self.s;
        let mut i = self.i;
        while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            i += 1;
        }
        let mut negative = false;
        if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
            negative = s[i] == b'-';
            i += 1;
        }
        let start = i;
        let mut n: u64 = 0;
        while i < s.len() && s[i].is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add(u64::from(s[i] - b'0'));
            i += 1;
        }
        if i == start {
            return None;
        }
        self.i = i;
        if negative {
            n = n.wrapping_neg();
        }
        Some(n as u32)
    }
}

/// `sscanf(size, "@%u:%ux%u", ...) == 3`.
fn scan_window_size(s: &[u8]) -> Option<(u32, u32, u32)> {
    let mut sc = Scanner { s, i: 0 };
    sc.literal(b'@')?;
    let w = sc.unsigned()?;
    sc.literal(b':')?;
    let x = sc.unsigned()?;
    sc.literal(b'x')?;
    let y = sc.unsigned()?;
    Some((w, x, y))
}

/// `sscanf(size, "@%u:", &w) == 1`: the colon does not affect the count.
fn scan_window_clear(s: &[u8]) -> Option<u32> {
    let mut sc = Scanner { s, i: 0 };
    sc.literal(b'@')?;
    sc.unsigned()
}

/// `sscanf(size, "%u<sep>%u", ...) == 2`.
fn scan_pair(s: &[u8], sep: u8) -> Option<(u32, u32)> {
    let mut sc = Scanner { s, i: 0 };
    let x = sc.unsigned()?;
    sc.literal(sep)?;
    let y = sc.unsigned()?;
    Some((x, y))
}

fn size_in_range(x: u32, y: u32) -> bool {
    (WINDOW_MINIMUM..=WINDOW_MAXIMUM).contains(&x) && (WINDOW_MINIMUM..=WINDOW_MAXIMUM).contains(&y)
}

/// The scanning and validation of `cmd_refresh_client_control_client_size`
/// (`cmd-refresh-client.c:70-100`); `Err` carries the exact error text.
pub fn parse_size(size: &[u8]) -> Result<SizeAction, &'static [u8]> {
    let size = rmux_util::bytes::cstr(size);
    if let Some((window, width, height)) = scan_window_size(size) {
        if !size_in_range(width, height) {
            return Err(b"size too small or too big");
        }
        return Ok(SizeAction::WindowSize {
            window,
            width,
            height,
        });
    }
    if let Some(window) = scan_window_clear(size) {
        return Ok(SizeAction::ClearWindow { window });
    }
    let Some((width, height)) = scan_pair(size, b',').or_else(|| scan_pair(size, b'x')) else {
        return Err(b"bad size argument");
    };
    if !size_in_range(width, height) {
        return Err(b"size too small or too big");
    }
    Ok(SizeAction::ClientSize { width, height })
}

/// `cmd_refresh_client_control_client_size` (`cmd-refresh-client.c:62-105`).
fn control_client_size(
    server: &mut Server,
    item: QueueItemId,
    tc: ClientId,
    size: &[u8],
) -> CmdReturn {
    match parse_size(size) {
        Err(message) => fail(server, item, message),
        Ok(SizeAction::WindowSize {
            window,
            width,
            height,
        }) => {
            crate::control::set_window_size(server, tc, window, width, height);
            if let Some(c) = server.clients.get_mut(tc) {
                c.flags.insert(ClientFlags::WINDOWSIZECHANGED);
            }
            crate::server::run::recalculate_sizes_now(server, true);
            CmdReturn::Normal
        }
        Ok(SizeAction::ClearWindow { window }) => {
            crate::control::clear_window_size(server, tc, window);
            crate::server::run::recalculate_sizes_now(server, true);
            CmdReturn::Normal
        }
        Ok(SizeAction::ClientSize { width, height }) => {
            if let Some(c) = server.clients.get_mut(tc) {
                match c.tty.as_mut() {
                    Some(tty) => tty.set_size(width, height, 0, 0),
                    None => {
                        c.tty_sx = width;
                        c.tty_sy = height;
                    }
                }
                c.flags.insert(ClientFlags::SIZECHANGED);
            }
            crate::server::run::recalculate_sizes_now(server, true);
            CmdReturn::Normal
        }
    }
}

/// The `%<pane>:<rest>` split shared by `-A` and `-r`
/// (`cmd-refresh-client.c:114-125, 149-160`): `sscanf(copy, "%%%u", &pane)`.
fn split_pane_value(server: &Server, value: &[u8]) -> Option<(crate::ids::PaneId, Vec<u8>)> {
    let value = rmux_util::bytes::cstr(value);
    if value.first() != Some(&b'%') {
        return None;
    }
    let colon = value.iter().position(|b| *b == b':')?;
    let (head, rest) = (&value[..colon], &value[colon + 1..]);
    let mut sc = Scanner { s: head, i: 0 };
    sc.literal(b'%')?;
    let pane = sc.unsigned()?;
    let wp = pane_find_by_public_id(server, pane)?;
    Some((wp, rest.to_vec()))
}

/// `cmd_refresh_client_update_offset` (`cmd-refresh-client.c:107-138`).
fn update_offset(server: &mut Server, tc: ClientId, value: &[u8]) {
    let Some((wp, state)) = split_pane_value(server, value) else {
        return;
    };
    match state.as_slice() {
        b"on" => crate::control::set_pane_on(server, tc, wp),
        b"off" => crate::control::set_pane_off(server, tc, wp),
        b"continue" => crate::control::continue_pane(server, tc, wp),
        b"pause" => crate::control::pause_pane(server, tc, wp),
        _ => {}
    }
}

/// `cmd_refresh_client_update_subscription` (`cmd-refresh-client.c:46-60`).
fn update_subscription(server: &mut Server, tc: ClientId, value: &[u8]) {
    match monitor_parse(value) {
        Ok(spec) => crate::control::add_sub(server, tc, spec),
        Err(_) => crate::control::remove_sub(server, tc, value),
    }
}

/// `cmd_refresh_report` (`cmd-refresh-client.c:140-173`) with
/// `tty_keys_colours` (`tty-keys.c:1754-1826`).
fn report(server: &mut Server, tc: ClientId, value: &[u8]) {
    let Some((wp, reply)) = split_pane_value(server, value) else {
        return;
    };
    let Recognition::Complete(_, reply) = parse_colour_response(&reply) else {
        return;
    };
    let Some(p) = server.panes.get(wp) else {
        return;
    };
    let (mut fg, mut bg) = (p.control_fg, p.control_bg);
    if let Some(colour) = reply.colour {
        let wait = match reply.target {
            ColourTarget::Foreground => {
                fg = colour.0;
                TtyFlags::WAITFG
            }
            ColourTarget::Background => {
                bg = colour.0;
                TtyFlags::WAITBG
            }
        };
        if let Some(tty) = server.clients.get_mut(tc).and_then(|c| c.tty.as_mut()) {
            tty.flags_mut().remove(wait);
        }
    }
    if let Some(p) = server.panes.get_mut(wp) {
        if bg != p.control_bg {
            p.flags.insert(PaneFlags::THEMECHANGED);
        }
        p.control_fg = fg;
        p.control_bg = bg;
    }
}

/// The pan branch (`cmd-refresh-client.c:186-235`).
fn pan(server: &mut Server, command: &Command, item: QueueItemId, tc: ClientId) -> CmdReturn {
    let args = &command.args;
    let adjust: u32 = match args.string(0) {
        None => 1,
        Some(s) => match strtonum(s, 1, i64::from(i32::MAX)) {
            Ok(n) => n as u32,
            Err(cause) => {
                return fail(server, item, format!("adjustment {cause}"));
            }
        },
    };

    if args.has(b'c') != 0 {
        if let Some(c) = server.clients.get_mut(tc) {
            c.pan_window = None;
        }
    } else {
        let Some(c) = server.clients.get(tc) else {
            return fail(server, item, b"no current client");
        };
        let window = c
            .session
            .and_then(|s| server.sessions.get(s))
            .and_then(|s| s.current)
            .and_then(|wl| server.winlinks.get(wl))
            .map(|wl| wl.window);
        let Some(w) = window else {
            return fail(server, item, b"no current window");
        };
        let (sx, sy) = server.windows.get(w).map_or((0, 0), |win| (win.sx, win.sy));
        let Some(c) = server.clients.get_mut(tc) else {
            return fail(server, item, b"no current client");
        };
        let (_, oox, ooy, osx, osy) = c
            .tty
            .as_ref()
            .map_or((false, 0, 0, 0, 0), |t| t.window_offset());
        if c.pan_window != Some(w) {
            c.pan_window = Some(w);
            c.pan_ox = oox;
            c.pan_oy = ooy;
        }
        if args.has(b'L') != 0 {
            c.pan_ox = c.pan_ox.saturating_sub(adjust);
        } else if args.has(b'R') != 0 {
            c.pan_ox = c.pan_ox.wrapping_add(adjust);
            let max = sx.wrapping_sub(osx);
            if c.pan_ox > max {
                c.pan_ox = max;
            }
        } else if args.has(b'U') != 0 {
            c.pan_oy = c.pan_oy.saturating_sub(adjust);
        } else if args.has(b'D') != 0 {
            c.pan_oy = c.pan_oy.wrapping_add(adjust);
            let max = sy.wrapping_sub(osy);
            if c.pan_oy > max {
                c.pan_oy = max;
            }
        }
    }
    crate::server::run::tty_update_client_offset(server, tc);
    server_redraw_client(server, tc);
    CmdReturn::Normal
}

/// `cmd_refresh_client_exec` (`cmd-refresh-client.c:175-287`).
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(tc) = item_target_client(server, item) else {
        return fail(server, item, b"no current client");
    };

    if args.has(b'c') != 0
        || args.has(b'L') != 0
        || args.has(b'R') != 0
        || args.has(b'U') != 0
        || args.has(b'D') != 0
    {
        return pan(server, command, item, tc);
    }

    if args.has(b'l') != 0 {
        let Server { clients, tparm, .. } = server;
        if let Some(tty) = clients.get_mut(tc).and_then(|c| c.tty.as_mut()) {
            tty.clipboard_query(tparm);
        }
        return CmdReturn::Normal;
    }

    // -F is an alias for -f.
    if let Some(flags) = args.get(b'F') {
        crate::client::flags::set_flags(server, tc, flags);
    }
    if let Some(flags) = args.get(b'f') {
        crate::client::flags::set_flags(server, tc, flags);
    }
    if let Some(value) = args.get(b'r') {
        report(server, tc, value);
    }

    let is_control = server
        .clients
        .get(tc)
        .is_some_and(|c| c.flags.intersects(ClientFlags::CONTROL));
    if args.has(b'A') != 0 {
        if !is_control {
            return fail(server, item, b"not a control client");
        }
        for value in args.values_of(b'A') {
            update_offset(server, tc, value.as_string());
        }
        return CmdReturn::Normal;
    }
    if args.has(b'B') != 0 {
        if !is_control {
            return fail(server, item, b"not a control client");
        }
        for value in args.values_of(b'B') {
            update_subscription(server, tc, value.as_string());
        }
        return CmdReturn::Normal;
    }
    if let Some(size) = args.get(b'C') {
        if !is_control {
            return fail(server, item, b"not a control client");
        }
        return control_client_size(server, item, tc, size);
    }

    if let Some(c) = server.clients.get_mut(tc) {
        c.flags.insert(ClientFlags::STATUSFORCE);
    }
    if args.has(b'S') != 0 {
        server_status_client(server, tc);
    } else {
        server_redraw_client(server, tc);
    }
    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_size_form() {
        assert_eq!(
            parse_size(b"@7:80x24"),
            Ok(SizeAction::WindowSize {
                window: 7,
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            parse_size(b"@ 7: +80x 24junk"),
            Ok(SizeAction::WindowSize {
                window: 7,
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            parse_size(b"@7:0x24"),
            Err(&b"size too small or too big"[..])
        );
        assert_eq!(
            parse_size(b"@7:80x10001"),
            Err(&b"size too small or too big"[..])
        );
        assert_eq!(
            parse_size(b"@7:1x10000"),
            Ok(SizeAction::WindowSize {
                window: 7,
                width: 1,
                height: 10000
            })
        );
        // `%u` of a negative number wraps and then fails the bound test.
        assert_eq!(
            parse_size(b"@7:-80x24"),
            Err(&b"size too small or too big"[..])
        );
    }

    #[test]
    fn incomplete_window_forms_clear_the_override() {
        assert_eq!(parse_size(b"@7"), Ok(SizeAction::ClearWindow { window: 7 }));
        assert_eq!(
            parse_size(b"@7:"),
            Ok(SizeAction::ClearWindow { window: 7 })
        );
        assert_eq!(
            parse_size(b"@7:bad"),
            Ok(SizeAction::ClearWindow { window: 7 })
        );
        assert_eq!(
            parse_size(b"@7:1xBAD"),
            Ok(SizeAction::ClearWindow { window: 7 })
        );
        assert_eq!(
            parse_size(b"@7:1x"),
            Ok(SizeAction::ClearWindow { window: 7 })
        );
        assert_eq!(
            parse_size(b"@7 :80x24"),
            Ok(SizeAction::ClearWindow { window: 7 })
        );
        assert_eq!(
            parse_size(b"@+7"),
            Ok(SizeAction::ClearWindow { window: 7 })
        );
        assert_eq!(parse_size(b"@"), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b"@x"), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b" @7"), Err(&b"bad size argument"[..]));
    }

    #[test]
    fn client_size_forms() {
        assert_eq!(
            parse_size(b"80x24"),
            Ok(SizeAction::ClientSize {
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            parse_size(b"80,24"),
            Ok(SizeAction::ClientSize {
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            parse_size(b"80x24junk"),
            Ok(SizeAction::ClientSize {
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            parse_size(b"+80,24"),
            Ok(SizeAction::ClientSize {
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            parse_size(b" 80x\t24"),
            Ok(SizeAction::ClientSize {
                width: 80,
                height: 24
            })
        );
        assert_eq!(
            parse_size(b"80,24x1"),
            Ok(SizeAction::ClientSize {
                width: 80,
                height: 24
            })
        );
        assert_eq!(parse_size(b"80 x24"), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b"80x"), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b"80"), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b""), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b"x24"), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b"80X24"), Err(&b"bad size argument"[..]));
        assert_eq!(parse_size(b"0x24"), Err(&b"size too small or too big"[..]));
        assert_eq!(
            parse_size(b"80,10001"),
            Err(&b"size too small or too big"[..])
        );
        assert_eq!(
            parse_size(b"-80x24"),
            Err(&b"size too small or too big"[..])
        );
        assert_eq!(
            parse_size(b"80x24\0ignored"),
            Ok(SizeAction::ClientSize {
                width: 80,
                height: 24
            })
        );
    }

    #[test]
    fn unsigned_conversion_wraps_and_saturates() {
        let mut sc = Scanner { s: b"-1", i: 0 };
        assert_eq!(sc.unsigned(), Some(u32::MAX));
        let mut sc = Scanner {
            s: b"4294967296",
            i: 0,
        };
        assert_eq!(sc.unsigned(), Some(0));
        let mut sc = Scanner {
            s: b"99999999999999999999999",
            i: 0,
        };
        assert_eq!(sc.unsigned(), Some(u32::MAX));
        let mut sc = Scanner { s: b"+", i: 0 };
        assert_eq!(sc.unsigned(), None);
    }
}
