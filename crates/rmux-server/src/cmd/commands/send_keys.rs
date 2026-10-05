// Ported from tmux cmd-send-keys.c @ 8f25579c
use super::support::{fail, item_event, item_target, item_target_client};
use crate::client::{ClientFlags, KeyEvent};
use crate::cmd::key_bindings;
use crate::cmd::queue::CmdReturn;
use crate::cmd::{Command, mouse_pane};
use crate::ids::{ClientId, PaneId, QueueItemId};
use crate::model::ModelEffect;
use crate::model::PaneFlags;
use crate::model::pane::{pane_key, pane_mode_command, pane_mode_has_command, pane_mode_key_table};
use crate::model::pane_input::{InputAction, InputTimer};
use crate::server::Server;
use rmux_emu::input::effect::{InputEffect, InputSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_tty::key_string::parse_key_name;
use rmux_util::key::{KeyCode, KeyFlags, KeyMasks, MouseEvent, SpecialKey};
use rmux_util::utf8::{Utf8State, from_cstr, from_data};

/// `strtol(s, &endptr, 16)` as accepted by `cmd-send-keys.c:123-126`: leading
/// whitespace, optional sign, optional `0x`, hex digits up to the end, result
/// in `0..=0xff`. `None` reproduces every rejected input (empty, no digits,
/// trailing text, negative, above `0xff`).
pub fn parse_hex_byte(s: &[u8]) -> Option<u8> {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    if i + 2 < s.len()
        && s[i] == b'0'
        && (s[i + 1] == b'x' || s[i + 1] == b'X')
        && s[i + 2].is_ascii_hexdigit()
    {
        i += 2;
    }
    let start = i;
    let mut n: i64 = 0;
    while i < s.len() && s[i].is_ascii_hexdigit() {
        let d = i64::from((s[i] as char).to_digit(16).unwrap_or(0));
        n = n.saturating_mul(16).saturating_add(d);
        i += 1;
    }
    // No digits: endptr is the original string, so a non-empty input fails
    // the `*endptr != '\0'` test (the empty input fails `*s == '\0'`).
    if i == start || i != s.len() {
        return None;
    }
    if negative {
        n = -n;
    }
    u8::try_from(n).ok()
}

/// `cmd_send_keys_inject_string` (`cmd-send-keys.c:111-158`) over an
/// injection seam `inject(after, key) -> after'`, so it needs no `Server`.
/// `None` is the C `NULL` item.
pub fn inject_string<T: Copy>(
    item: T,
    mut after: Option<T>,
    s: &[u8],
    hex: bool,
    literal: bool,
    inject: &mut impl FnMut(Option<T>, KeyCode) -> Option<T>,
) -> Option<T> {
    if hex {
        let Some(n) = parse_hex_byte(s) else {
            return Some(item);
        };
        return inject(after, KeyCode(KeyFlags::LITERAL.bits() | u64::from(n)));
    }
    if !literal {
        let key = parse_key_name(s);
        if key.0 != SpecialKey::NONE && key.0 != SpecialKey::UNKNOWN {
            after = inject(after, key);
            if after.is_some() {
                return after;
            }
        }
    }
    for ud in &from_cstr(s).0 {
        let key = if ud.size == 1 && ud.data[0] <= 0x7f {
            KeyCode(u64::from(ud.data[0]))
        } else {
            let (uc, state) = from_data(ud);
            if state != Utf8State::Done {
                continue;
            }
            KeyCode(u64::from(uc.0))
        };
        after = inject(after, key);
    }
    after
}

/// The `struct mouse_event` fields the queue event keeps (`cmd-send-keys.c:170`).
fn mouse_event(server: &Server, item: QueueItemId) -> MouseEvent {
    let m = item_event(server, item).mouse;
    MouseEvent {
        x: m.x,
        y: m.y,
        lx: m.last_x,
        ly: m.last_y,
        ..MouseEvent::default()
    }
}

/// `cmd_send_keys_inject_key` (`cmd-send-keys.c:60-109`).
fn inject_key(
    server: &mut Server,
    item: QueueItemId,
    after: Option<QueueItemId>,
    with_client: bool,
    key: KeyCode,
) -> Option<QueueItemId> {
    let target = item_target(server, item);
    let tc = item_target_client(server, item);

    if with_client {
        let tc = tc?;
        let event = KeyEvent::new(KeyCode(key.0 | KeyFlags::SENT.bits()));
        return match after {
            None => {
                crate::client::keys::handle_key(server, tc, event);
                Some(item)
            }
            Some(after) => {
                crate::client::keys::handle_key_after(server, tc, event, after).or(Some(item))
            }
        };
    }

    let wp = target.wp?;
    let has_mode = server.panes.get(wp).is_some_and(|p| !p.modes.is_empty());
    let table_name = if has_mode {
        pane_mode_key_table(server, wp)
    } else {
        None
    };
    let Some(name) = table_name else {
        let policy = crate::client::keys::key_policy(server);
        return match pane_key(server, wp, tc, key, None, &policy) {
            Ok(()) => Some(item),
            Err(_) => None,
        };
    };
    let Ok(Some(table)) = server.key_bindings.get_table(&name, true) else {
        return after;
    };
    let binding = server
        .key_bindings
        .get(table, KeyCode(key.0 & !KeyMasks::FLAGS))
        .map(|bd| bd.dispatch_snapshot());
    match binding {
        Some(snapshot) => {
            let _ = server.key_bindings.retain_table(table);
            let after = key_bindings::dispatch(server, snapshot, after, tc, None, &target);
            let _ = server.key_bindings.unref_table(table);
            after
        }
        None => after,
    }
}

/// Collects the parser reset effect (`input_reset`, `input.c:927-947`).
#[derive(Default)]
struct ResetSink {
    ground_timer: Option<bool>,
}
impl InputSink for ResetSink {
    fn effect(&mut self, effect: InputEffect<'_>) {
        if let InputEffect::GroundTimer(arm) = effect {
            self.ground_timer = Some(arm);
        }
    }
    fn reply(&mut self, _: &[u8]) {}
}

/// `cmd-send-keys.c:230-234`: `colour_palette_clear`, `input_reset(ictx, 1)`
/// and the style, theme and redraw flags.
fn reset_pane(server: &mut Server, wp: PaneId) {
    let Some(p) = server.panes.get(wp) else {
        return;
    };
    let options = p.options;
    let global = server.options.global;
    let policy = ScreenWritePolicy {
        pane_backed: p.modes.is_empty(),
        alternate_screen: server.options.get_number(options, b"alternate-screen") != 0,
        scroll_on_clear: server.options.get_number(options, b"scroll-on-clear") != 0,
        variation_selector_always_wide: server
            .options
            .get_number(global, b"variation-selector-always-wide")
            != 0,
        extended_keys: server.options.get_number(global, b"extended-keys") != 0,
    };
    let Server {
        panes, hyperlinks, ..
    } = server;
    let Some(p) = panes.get_mut(wp) else {
        return;
    };
    p.palette.clear_runtime();
    let mut tty_sink = ScreenOnlySink;
    let mut sink = ResetSink::default();
    let mut writer = ScreenWriteCtx::start(&mut p.base, &mut tty_sink, policy, hyperlinks);
    p.parser.reset(Some(&mut writer), &mut sink);
    writer.finish();
    p.flags
        .insert(PaneFlags::STYLECHANGED | PaneFlags::THEMECHANGED | PaneFlags::REDRAW);
    if let Some(arm) = sink.ground_timer {
        p.input_state.ground_timer = arm;
        server
            .effects
            .push_back(ModelEffect::Input(InputAction::Timer {
                pane: wp,
                timer: InputTimer::Ground,
                after: None,
            }));
    }
}

fn client_flags(server: &Server, client: Option<ClientId>) -> ClientFlags {
    client
        .and_then(|c| server.clients.get(c))
        .map(|c| c.flags)
        .unwrap_or_default()
}

/// `cmd_send_keys_exec` (`cmd-send-keys.c:160-254`) for `send-keys` and
/// `send-prefix`.
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let tc = item_target_client(server, item);
    let event = item_event(server, item);
    let Some(wp) = target.wp else {
        return fail(server, item, b"no current target");
    };
    let has_mode = server.panes.get(wp).is_some_and(|p| !p.modes.is_empty());
    let count = args.count();
    let mut np: u32 = 1;

    if client_flags(server, tc).intersects(ClientFlags::READONLY) && args.has(b'X') == 0 {
        return fail(server, item, b"client is read-only");
    }

    if args.has(b'N') != 0 {
        match args.strtonum_and_expand(server, b'N', 1, i64::from(u32::MAX), item) {
            Ok(n) => np = n as u32,
            Err(cause) => {
                let mut msg = b"repeat count ".to_vec();
                msg.extend_from_slice(&cause);
                return fail(server, item, msg);
            }
        }
        if has_mode && (args.has(b'X') != 0 || count == 0) {
            if !pane_mode_has_command(server, wp) {
                return fail(server, item, b"not in a mode");
            }
            if let Some(mode) = server.panes.get_mut(wp).and_then(|p| p.modes.first_mut()) {
                mode.prefix = np;
            }
        }
    }

    if args.has(b'X') != 0 {
        if !has_mode || !pane_mode_has_command(server, wp) {
            return fail(server, item, b"not in a mode");
        }
        let m = event.mouse.valid.then(|| mouse_event(server, item));
        pane_mode_command(server, wp, tc, target.s, target.wl, args, m.as_ref());
        return CmdReturn::Normal;
    }

    if args.has(b'M') != 0 {
        let Some((_, _, mwp)) = mouse_pane(server, &event.mouse) else {
            return fail(server, item, b"no mouse target");
        };
        let m = mouse_event(server, item);
        let policy = crate::client::keys::key_policy(server);
        let _ = pane_key(server, mwp, tc, event.key, Some(&m), &policy);
        return CmdReturn::Normal;
    }

    if std::ptr::eq(command.entry, &crate::cmd::metadata::CMD_SEND_PREFIX) {
        let Some(options) = target
            .s
            .and_then(|s| server.sessions.get(s))
            .map(|s| s.options)
        else {
            return fail(server, item, b"no current session");
        };
        let name: &[u8] = if args.has(b'2') != 0 {
            b"prefix2"
        } else {
            b"prefix"
        };
        let key = KeyCode(server.options.get_number(options, name) as u64);
        inject_key(server, item, Some(item), args.has(b'K') != 0, key);
        return CmdReturn::Normal;
    }

    if args.has(b'R') != 0 {
        reset_pane(server, wp);
    }

    let with_client = args.has(b'K') != 0;
    if count == 0 {
        if args.has(b'N') != 0 || args.has(b'R') != 0 {
            return CmdReturn::Normal;
        }
        let mut after = with_client.then_some(item);
        for _ in 0..np {
            after = inject_key(server, item, after, with_client, event.key);
        }
        return CmdReturn::Normal;
    }

    let hex = args.has(b'H') != 0;
    let literal = args.has(b'l') != 0;
    let mut after = Some(item);
    for _ in 0..np {
        for i in 0..count {
            let s = args.string(i).unwrap_or(b"").to_vec();
            after = inject_string(item, after, &s, hex, literal, &mut |after, key| {
                inject_key(server, item, after, with_client, key)
            });
        }
    }
    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_bounds_and_forms() {
        assert_eq!(parse_hex_byte(b"41"), Some(0x41));
        assert_eq!(parse_hex_byte(b"0x41"), Some(0x41));
        assert_eq!(parse_hex_byte(b"0X41"), Some(0x41));
        assert_eq!(parse_hex_byte(b"+41"), Some(0x41));
        assert_eq!(parse_hex_byte(b"  41"), Some(0x41));
        assert_eq!(parse_hex_byte(b"\t\n 0x7f"), Some(0x7f));
        assert_eq!(parse_hex_byte(b"-0"), Some(0));
        assert_eq!(parse_hex_byte(b"0"), Some(0));
        assert_eq!(parse_hex_byte(b"ff"), Some(0xff));
        assert_eq!(parse_hex_byte(b"100"), None);
        assert_eq!(parse_hex_byte(b"-1"), None);
        assert_eq!(parse_hex_byte(b"-41"), None);
        assert_eq!(parse_hex_byte(b""), None);
        assert_eq!(parse_hex_byte(b"   "), None);
        assert_eq!(parse_hex_byte(b"41 "), None);
        assert_eq!(parse_hex_byte(b"41x"), None);
        assert_eq!(parse_hex_byte(b"0x"), None);
        assert_eq!(parse_hex_byte(b"x41"), None);
        assert_eq!(parse_hex_byte(b"ffffffffffffffffffff"), None);
    }

    fn record(
        log: &mut Vec<(Option<u32>, u64)>,
        next: Option<u32>,
    ) -> impl FnMut(Option<u32>, KeyCode) -> Option<u32> + '_ {
        move |after, key| {
            log.push((after, key.0));
            next
        }
    }

    #[test]
    fn hex_rejections_return_item_and_send_nothing() {
        for s in [&b""[..], b"   ", b"-1", b"100", b"41junk"] {
            let mut log = Vec::new();
            let r = inject_string(
                7u32,
                Some(9),
                s,
                true,
                false,
                &mut record(&mut log, Some(3)),
            );
            assert_eq!(r, Some(7), "{:?}", s);
            assert!(log.is_empty());
        }
    }

    #[test]
    fn hex_accepts_and_injects_literal_key() {
        let mut log = Vec::new();
        let r = inject_string(
            7u32,
            Some(9),
            b" +0x41",
            true,
            true,
            &mut record(&mut log, Some(3)),
        );
        assert_eq!(r, Some(3));
        assert_eq!(log, vec![(Some(9), KeyFlags::LITERAL.bits() | 0x41)]);
        let mut log = Vec::new();
        assert_eq!(
            inject_string(7u32, None, b"-0", true, false, &mut record(&mut log, None)),
            None
        );
        assert_eq!(log, vec![(None, KeyFlags::LITERAL.bits())]);
    }

    #[test]
    fn key_name_injects_once_and_returns_tail() {
        let mut log = Vec::new();
        let r = inject_string(
            7u32,
            Some(7),
            b"Enter",
            false,
            false,
            &mut record(&mut log, Some(11)),
        );
        assert_eq!(r, Some(11));
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, Some(7));
        assert_eq!(log[0].1, parse_key_name(b"Enter").0);
    }

    #[test]
    fn null_injection_falls_through_to_literal() {
        // "ab" is not a key name, so only the two literal bytes inject.
        let mut log = Vec::new();
        let r = inject_string(
            7u32,
            Some(7),
            b"ab",
            false,
            false,
            &mut record(&mut log, None),
        );
        assert_eq!(r, None);
        assert_eq!(
            log,
            vec![(Some(7), u64::from(b'a')), (None, u64::from(b'b'))]
        );

        // A known name whose injection returns NULL falls through to literal.
        let mut log = Vec::new();
        let r = inject_string(
            7u32,
            Some(7),
            b"C-a",
            false,
            false,
            &mut record(&mut log, None),
        );
        assert_eq!(r, None);
        assert_eq!(log.len(), 4);
        assert_eq!(log[0], (Some(7), parse_key_name(b"C-a").0));
        assert_eq!(log[1], (None, u64::from(b'C')));
        assert_eq!(log[2], (None, u64::from(b'-')));
        assert_eq!(log[3], (None, u64::from(b'a')));
    }

    #[test]
    fn insertion_tail_threads_through_literal_keys() {
        let mut n = 100u32;
        let mut log = Vec::new();
        let r = inject_string(7u32, Some(7), b"xy", false, true, &mut |after, key| {
            log.push((after, key.0));
            n += 1;
            Some(n)
        });
        assert_eq!(r, Some(102));
        assert_eq!(
            log,
            vec![(Some(7), u64::from(b'x')), (Some(101), u64::from(b'y'))]
        );
    }

    #[test]
    fn literal_utf8_dispatches_code_points() {
        let mut log = Vec::new();
        let r = inject_string(
            7u32,
            Some(1),
            "aé€".as_bytes(),
            false,
            true,
            &mut record(&mut log, Some(2)),
        );
        assert_eq!(r, Some(2));
        let keys: Vec<u64> = log.iter().map(|(_, k)| *k).collect();
        let (e_acute, _) = from_data(&from_cstr("é".as_bytes()).0[0]);
        let (euro, _) = from_data(&from_cstr("€".as_bytes()).0[0]);
        assert_eq!(
            keys,
            vec![u64::from(b'a'), u64::from(e_acute.0), u64::from(euro.0)]
        );
        assert!(keys[1] > 0x7f && keys[2] > 0x7f);
    }

    #[test]
    fn literal_flag_skips_key_lookup() {
        let mut log = Vec::new();
        inject_string(7u32, None, b"Up", false, true, &mut record(&mut log, None));
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].1, u64::from(b'U'));
        assert_eq!(log[1].1, u64::from(b'p'));
    }
}
