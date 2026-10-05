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

//! `server_client_print` (`server-client.c:3081-3149`).

use crate::client::ClientFlags;
use crate::ids::ClientId;
use crate::server::Server;
use rmux_util::bytes::cstr;
use rmux_util::log_debug;
use rmux_util::vis::VisFlags;

/// The message text as C builds it (`server-client.c:3090-3101`): escaped
/// bytes without `parse`; with `parse` the data up to its first NUL (an empty
/// input is an empty string).
fn message_text(parse: bool, data: &[u8]) -> Vec<u8> {
    if parse {
        cstr(data).to_vec()
    } else {
        let mut out = Vec::with_capacity(data.len());
        rmux_util::utf8::strvis(
            &mut out,
            data,
            VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::NOSLASH,
        );
        out
    }
}

/// The parsed-input buffer as the C `evbuffer` holds it after
/// `server-client.c:3094-3100`: untouched when empty, otherwise NUL-ended.
fn parsed_buffer(data: &[u8]) -> Vec<u8> {
    let mut buffer = data.to_vec();
    if !buffer.is_empty() && buffer.last() != Some(&0) {
        buffer.push(0);
    }
    buffer
}

/// The lines `evbuffer_readln(EVBUFFER_EOL_LF)` yields followed by the
/// final partial line printed with `%.*s` (`server-client.c:3130-3142`).
fn parsed_lines(buffer: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut rest = buffer;
    while let Some(at) = rest.iter().position(|b| *b == b'\n') {
        lines.push(&rest[..at]);
        rest = &rest[at + 1..];
    }
    if !rest.is_empty() {
        lines.push(cstr(rest));
    }
    lines
}

/// `server_client_print` (`server-client.c:3081-3149`). No client: nothing
/// is written. An unattached or control client gets the text (sanitized
/// unless UTF-8) through control or file output; an attached client gets it
/// in view mode on the active pane, line by line when `parse` is set.
pub fn print(server: &mut Server, id: Option<ClientId>, parse: bool, data: &[u8]) {
    let msg = message_text(parse, data);
    log_debug!("server_client_print: {}", String::from_utf8_lossy(&msg));

    let Some(id) = id else {
        return;
    };
    let Some(c) = server.clients.get(id) else {
        return;
    };

    if c.session.is_none() || c.flags.intersects(ClientFlags::CONTROL) {
        let control = c.flags.intersects(ClientFlags::CONTROL);
        let text = if c.flags.intersects(ClientFlags::UTF8) {
            msg
        } else {
            rmux_util::utf8::sanitize(&msg).into_vec()
        };
        if control {
            crate::control::write(server, id, &text);
        } else {
            let mut line = text;
            line.push(b'\n');
            crate::server::file::print(server, id, &line);
        }
        return;
    }

    let Some(pane) = c
        .session
        .and_then(|s| server.sessions.get(s))
        .and_then(|s| s.current)
        .and_then(|wl| server.winlinks.get(wl))
        .and_then(|wl| server.windows.get(wl.window))
        .and_then(|w| w.active)
    else {
        return;
    };
    // window_pane_set_mode(view) + window_copy_add (3124-3144). View mode is
    // G18; until it lands the runtime reports Unavailable and the text goes
    // to the client's stdout instead of being dropped.
    use crate::cmd::cfg::CfgRuntime;
    let lines: Vec<Vec<u8>> = if parse {
        let buffer = parsed_buffer(data);
        parsed_lines(&buffer)
            .iter()
            .map(|line| line.to_vec())
            .collect()
    } else {
        vec![msg]
    };
    if !CfgRuntime::pane_top_is_view(server, pane)
        && CfgRuntime::enter_view_mode(server, pane).is_err()
    {
        let mut out = Vec::new();
        for line in &lines {
            out.extend_from_slice(line);
            out.push(b'\n');
        }
        crate::server::file::print_unchecked(server, id, &out);
        return;
    }
    for line in &lines {
        let _ = CfgRuntime::append_view_line(server, pane, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unparsed_text_is_escaped_byte_for_byte() {
        assert_eq!(message_text(false, b"a\x1bb\x00c"), b"a\\033b\\0c");
        // Without VIS_TAB a tab stays raw (server-client.c:3091 flags).
        assert_eq!(message_text(false, b"tab\there"), b"tab\there");
        assert_eq!(message_text(false, b"back\\slash"), b"back\\slash");
    }

    #[test]
    fn parsed_text_stops_at_nul() {
        assert_eq!(message_text(true, b""), b"");
        assert_eq!(message_text(true, b"abc\0def"), b"abc");
        assert_eq!(message_text(true, b"abc"), b"abc");
    }

    #[test]
    fn parsed_lines_follow_evbuffer_readln() {
        assert!(parsed_lines(&parsed_buffer(b"")).is_empty());
        assert_eq!(parsed_lines(&parsed_buffer(b"hello")), vec![&b"hello"[..]]);
        assert_eq!(
            parsed_lines(&parsed_buffer(b"one\ntwo\nthree")),
            vec![&b"one"[..], &b"two"[..], &b"three"[..]]
        );
        // A trailing LF leaves the appended NUL as one empty final line.
        assert_eq!(
            parsed_lines(&parsed_buffer(b"one\n")),
            vec![&b"one"[..], &b""[..]]
        );
        // Data that already ends in NUL gets no second NUL.
        assert_eq!(parsed_buffer(b"x\0"), b"x\0");
        assert_eq!(parsed_lines(&parsed_buffer(b"x\0")), vec![&b"x"[..]]);
    }
}
