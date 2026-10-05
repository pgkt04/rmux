// Ported from tmux server-client.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! `server_client_set_flags`, `server_client_get_flags` and the control-only
//! flag names (`server-client.c:2964-3063`).

use crate::client::{Client, ClientFlags};
use crate::ids::ClientId;
use crate::server::Server;
use crate::server::proc::proc_send;
use crate::server::protocol::{ProtocolMessage, ProtocolMessageKind};
use rmux_util::bytes::cstr;
use rmux_util::log_debug;

/// C `sscanf("%u")`: optional leading whitespace, optional sign, at least
/// one digit, unsigned wrap; anything after the digits is accepted and
/// ignored. None when no number was scanned.
fn scan_unsigned(input: &[u8]) -> Option<u32> {
    let mut rest = input;
    while let Some((first, tail)) = rest.split_first() {
        if first.is_ascii_whitespace() {
            rest = tail;
        } else {
            break;
        }
    }
    let negative = match rest.first() {
        Some(b'-') => {
            rest = &rest[1..];
            true
        }
        Some(b'+') => {
            rest = &rest[1..];
            false
        }
        _ => false,
    };
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let mut value: u32 = 0;
    for digit in &rest[..digits] {
        value = value.wrapping_mul(10).wrapping_add(u32::from(digit - b'0'));
    }
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

/// `server_client_control_flags` (`server-client.c:2964-2982`): returns the
/// control flag a token names and updates `pause_age` for `pause-after`.
fn control_flags(c: &mut Client, next: &[u8]) -> ClientFlags {
    if next == b"pause-after" {
        c.pause_age = 0;
        return ClientFlags::CONTROL_PAUSEAFTER;
    }
    if let Some(value) = next.strip_prefix(b"pause-after=")
        && let Some(age) = scan_unsigned(value)
    {
        c.pause_age = age.wrapping_mul(1000);
        return ClientFlags::CONTROL_PAUSEAFTER;
    }
    if next == b"no-output" {
        return ClientFlags::CONTROL_NOOUTPUT;
    }
    if next == b"wait-exit" {
        return ClientFlags::CONTROL_WAITEXIT;
    }
    if next == b"new-layouts" {
        return ClientFlags::CONTROL_NEWLAYOUTS;
    }
    ClientFlags::default()
}

/// The pure part of `server_client_set_flags` (`server-client.c:2985-3020`):
/// apply every `,`-separated token to the client and report whether
/// `no-output` changed (so the caller resets control offsets).
fn apply_flags(c: &mut Client, spec: &[u8]) -> bool {
    let mut reset_offsets = false;
    for token in cstr(spec).split(|b| *b == b',') {
        let (not, next) = match token.strip_prefix(b"!") {
            Some(rest) => (true, rest),
            None => (false, token),
        };
        let mut flag = if c.flags.intersects(ClientFlags::CONTROL) {
            control_flags(c, next)
        } else {
            ClientFlags::default()
        };
        if next == b"read-only" {
            flag = ClientFlags::READONLY;
        } else if next == b"ignore-size" {
            flag = ClientFlags::IGNORESIZE;
        } else if next == b"no-detach-on-destroy" {
            flag = ClientFlags::NO_DETACH_ON_DESTROY;
        }
        if flag.bits() == 0 {
            continue;
        }
        log_debug!(
            "client {} set flag {}",
            String::from_utf8_lossy(c.name_bytes()),
            String::from_utf8_lossy(next)
        );
        if not {
            // Read-only is sticky: a set READONLY leaves the mask (`:3013-3014`).
            if c.flags.intersects(ClientFlags::READONLY) {
                flag.remove(ClientFlags::READONLY);
            }
            c.flags.remove(flag);
        } else {
            c.flags.insert(flag);
        }
        if flag == ClientFlags::CONTROL_NOOUTPUT {
            reset_offsets = true;
        }
    }
    reset_offsets
}

/// `server_client_set_flags` (`server-client.c:2985-3023`). Unknown tokens
/// have no effect; the full 64-bit flags are always sent back as `MSG_FLAGS`.
pub fn set_flags(server: &mut Server, id: ClientId, spec: &[u8]) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let reset_offsets = apply_flags(c, spec);
    let (flags, peer) = (c.flags, c.peer);
    if reset_offsets {
        crate::control::reset_offsets(server, id);
    }
    if let Some(peer) = peer {
        let _ = proc_send(
            server,
            peer,
            ProtocolMessage::new(
                ProtocolMessageKind::Flags,
                flags.bits().to_le_bytes().to_vec(),
            ),
        );
    }
}

/// `server_client_get_flags` (`server-client.c:3026-3063`): the user-visible
/// flags in fixed order, comma separated.
pub fn get_flags(server: &Server, id: ClientId) -> String {
    server
        .clients
        .get(id)
        .map_or_else(String::new, format_flags)
}

fn format_flags(c: &Client) -> String {
    let mut s = String::new();
    let names: [(ClientFlags, &str); 8] = [
        (ClientFlags::ATTACHED, "attached"),
        (ClientFlags::FOCUSED, "focused"),
        (ClientFlags::CONTROL, "control-mode"),
        (ClientFlags::IGNORESIZE, "ignore-size"),
        (ClientFlags::NO_DETACH_ON_DESTROY, "no-detach-on-destroy"),
        (ClientFlags::CONTROL_NOOUTPUT, "no-output"),
        (ClientFlags::CONTROL_WAITEXIT, "wait-exit"),
        (ClientFlags::CONTROL_NEWLAYOUTS, "new-layouts"),
    ];
    for (flag, name) in names {
        if c.flags.intersects(flag) {
            s.push_str(name);
            s.push(',');
        }
    }
    if c.flags.intersects(ClientFlags::CONTROL_PAUSEAFTER) {
        s.push_str(&format!("pause-after={},", c.pause_age / 1000));
    }
    for (flag, name) in [
        (ClientFlags::READONLY, "read-only"),
        (ClientFlags::SUSPENDED, "suspended"),
        (ClientFlags::UTF8, "UTF-8"),
    ] {
        if c.flags.intersects(flag) {
            s.push_str(name);
            s.push(',');
        }
    }
    s.pop();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(flags: ClientFlags) -> Client {
        let mut c = Client::new(None, (0, 0));
        c.flags = flags;
        c
    }

    #[test]
    fn get_flags_order_is_fixed() {
        let mut c = client(
            ClientFlags::UTF8
                | ClientFlags::READONLY
                | ClientFlags::CONTROL_PAUSEAFTER
                | ClientFlags::CONTROL_NEWLAYOUTS
                | ClientFlags::CONTROL_WAITEXIT
                | ClientFlags::CONTROL_NOOUTPUT
                | ClientFlags::NO_DETACH_ON_DESTROY
                | ClientFlags::IGNORESIZE
                | ClientFlags::CONTROL
                | ClientFlags::FOCUSED
                | ClientFlags::ATTACHED
                | ClientFlags::SUSPENDED,
        );
        c.pause_age = 3000;
        assert_eq!(
            format_flags(&c),
            "attached,focused,control-mode,ignore-size,no-detach-on-destroy,no-output,wait-exit,new-layouts,pause-after=3,read-only,suspended,UTF-8"
        );
        assert_eq!(format_flags(&client(ClientFlags::default())), "");
        assert_eq!(format_flags(&client(ClientFlags::FOCUSED)), "focused");
    }

    #[test]
    fn set_flags_parses_tokens_and_negation() {
        let mut c = client(ClientFlags::FOCUSED);
        apply_flags(&mut c, b"read-only,ignore-size,no-detach-on-destroy,bogus");
        assert!(c.flags.contains(
            ClientFlags::READONLY | ClientFlags::IGNORESIZE | ClientFlags::NO_DETACH_ON_DESTROY
        ));
        apply_flags(&mut c, b"!ignore-size,!no-detach-on-destroy,!bogus");
        assert!(
            !c.flags
                .intersects(ClientFlags::IGNORESIZE | ClientFlags::NO_DETACH_ON_DESTROY)
        );
        assert!(c.flags.contains(ClientFlags::READONLY));
    }

    #[test]
    fn read_only_is_sticky() {
        let mut c = client(ClientFlags::default());
        apply_flags(&mut c, b"read-only");
        apply_flags(&mut c, b"!read-only");
        assert!(c.flags.contains(ClientFlags::READONLY));
    }

    #[test]
    fn control_only_tokens_need_a_control_client() {
        let mut c = client(ClientFlags::default());
        apply_flags(&mut c, b"pause-after=3,no-output,wait-exit,new-layouts");
        assert_eq!(c.flags, ClientFlags::default());
        assert_eq!(c.pause_age, 0);

        let mut c = client(ClientFlags::CONTROL);
        apply_flags(&mut c, b"pause-after=3");
        assert!(c.flags.contains(ClientFlags::CONTROL_PAUSEAFTER));
        assert_eq!(c.pause_age, 3000);
        assert_eq!(format_flags(&c), "control-mode,pause-after=3");

        apply_flags(&mut c, b"wait-exit,new-layouts");
        assert!(
            c.flags
                .contains(ClientFlags::CONTROL_WAITEXIT | ClientFlags::CONTROL_NEWLAYOUTS)
        );
    }

    #[test]
    fn pause_after_accepts_trailing_text_and_wraps() {
        let mut c = client(ClientFlags::CONTROL);
        apply_flags(&mut c, b"pause-after=7seconds");
        assert_eq!(c.pause_age, 7000);
        apply_flags(&mut c, b"pause-after= 2");
        assert_eq!(c.pause_age, 2000);
        apply_flags(&mut c, b"pause-after=4294967295");
        assert_eq!(c.pause_age, 4_294_967_295u32.wrapping_mul(1000));
        apply_flags(&mut c, b"pause-after=-1");
        assert_eq!(c.pause_age, u32::MAX.wrapping_mul(1000));
        // No digits: not a pause-after token, so nothing changes.
        c.pause_age = 5000;
        apply_flags(&mut c, b"pause-after=x");
        assert_eq!(c.pause_age, 5000);
        // Bare pause-after resets the age to zero.
        apply_flags(&mut c, b"pause-after");
        assert_eq!(c.pause_age, 0);
        assert!(c.flags.contains(ClientFlags::CONTROL_PAUSEAFTER));
    }

    #[test]
    fn negated_pause_after_updates_age_before_clearing() {
        let mut c = client(ClientFlags::CONTROL | ClientFlags::CONTROL_PAUSEAFTER);
        c.pause_age = 1000;
        apply_flags(&mut c, b"!pause-after=9");
        assert_eq!(c.pause_age, 9000);
        assert!(!c.flags.contains(ClientFlags::CONTROL_PAUSEAFTER));
    }

    #[test]
    fn no_output_both_transitions_reset_offsets() {
        let mut c = client(ClientFlags::CONTROL);
        assert!(apply_flags(&mut c, b"no-output"));
        assert!(c.flags.contains(ClientFlags::CONTROL_NOOUTPUT));
        assert!(apply_flags(&mut c, b"!no-output"));
        assert!(!c.flags.contains(ClientFlags::CONTROL_NOOUTPUT));
        assert!(!apply_flags(&mut c, b"wait-exit"));
        assert!(!apply_flags(&mut c, b"unknown"));
    }

    #[test]
    fn scan_unsigned_matches_sscanf() {
        assert_eq!(scan_unsigned(b"12abc"), Some(12));
        assert_eq!(scan_unsigned(b"  +3"), Some(3));
        assert_eq!(scan_unsigned(b""), None);
        assert_eq!(scan_unsigned(b"-"), None);
        assert_eq!(
            scan_unsigned(b"99999999999"),
            Some(99_999_999_999u64 as u32)
        );
    }
}
