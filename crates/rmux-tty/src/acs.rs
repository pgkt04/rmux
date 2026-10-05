// Ported from tmux tty-acs.c @ 8f25579c
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
use crate::term::{TtyCodeCode, TtyTerm};
const FORWARD: &[(u8, &[u8])] = &[
    (b'+', b"\xe2\x86\x92"),
    (b',', b"\xe2\x86\x90"),
    (b'-', b"\xe2\x86\x91"),
    (b'.', b"\xe2\x86\x93"),
    (b'0', b"\xe2\x96\xae"),
    (b'`', b"\xe2\x97\x86"),
    (b'a', b"\xe2\x96\x92"),
    (b'b', b"\xe2\x90\x89"),
    (b'c', b"\xe2\x90\x8c"),
    (b'd', b"\xe2\x90\x8d"),
    (b'e', b"\xe2\x90\x8a"),
    (b'f', b"\xc2\xb0"),
    (b'g', b"\xc2\xb1"),
    (b'h', b"\xe2\x90\xa4"),
    (b'i', b"\xe2\x90\x8b"),
    (b'j', b"\xe2\x94\x98"),
    (b'k', b"\xe2\x94\x90"),
    (b'l', b"\xe2\x94\x8c"),
    (b'm', b"\xe2\x94\x94"),
    (b'n', b"\xe2\x94\xbc"),
    (b'o', b"\xe2\x8e\xba"),
    (b'p', b"\xe2\x8e\xbb"),
    (b'q', b"\xe2\x94\x80"),
    (b'r', b"\xe2\x8e\xbc"),
    (b's', b"\xe2\x8e\xbd"),
    (b't', b"\xe2\x94\x9c"),
    (b'u', b"\xe2\x94\xa4"),
    (b'v', b"\xe2\x94\xb4"),
    (b'w', b"\xe2\x94\xac"),
    (b'x', b"\xe2\x94\x82"),
    (b'y', b"\xe2\x89\xa4"),
    (b'z', b"\xe2\x89\xa5"),
    (b'{', b"\xcf\x80"),
    (b'|', b"\xe2\x89\xa0"),
    (b'}', b"\xc2\xa3"),
    (b'~', b"\xc2\xb7"),
];
const REVERSE: &[(&[u8], u8)] = &[
    (b"\xc2\xb7", b'~'),
    (b"\xe2\x94\x80", b'q'),
    (b"\xe2\x94\x81", b'q'),
    (b"\xe2\x94\x82", b'x'),
    (b"\xe2\x94\x83", b'x'),
    (b"\xe2\x94\x8c", b'l'),
    (b"\xe2\x94\x8f", b'k'),
    (b"\xe2\x94\x90", b'k'),
    (b"\xe2\x94\x93", b'l'),
    (b"\xe2\x94\x94", b'm'),
    (b"\xe2\x94\x97", b'm'),
    (b"\xe2\x94\x98", b'j'),
    (b"\xe2\x94\x9b", b'j'),
    (b"\xe2\x94\x9c", b't'),
    (b"\xe2\x94\xa3", b't'),
    (b"\xe2\x94\xa4", b'u'),
    (b"\xe2\x94\xab", b'u'),
    (b"\xe2\x94\xb3", b'w'),
    (b"\xe2\x94\xb4", b'v'),
    (b"\xe2\x94\xbb", b'v'),
    (b"\xe2\x94\xbc", b'n'),
    (b"\xe2\x95\x8b", b'n'),
    (b"\xe2\x95\x90", b'q'),
    (b"\xe2\x95\x91", b'x'),
    (b"\xe2\x95\x94", b'l'),
    (b"\xe2\x95\x97", b'k'),
    (b"\xe2\x95\x9a", b'm'),
    (b"\xe2\x95\x9d", b'j'),
    (b"\xe2\x95\xa0", b't'),
    (b"\xe2\x95\xa3", b'u'),
    (b"\xe2\x95\xa6", b'w'),
    (b"\xe2\x95\xa9", b'v'),
    (b"\xe2\x95\xac", b'n'),
];

pub fn acs_needed(term: &TtyTerm, utf8: bool) -> bool {
    if term.has(TtyCodeCode::U8) && term.number(TtyCodeCode::U8) == 0 {
        true
    } else {
        !utf8
    }
}
pub fn acs_get(term: &TtyTerm, utf8: bool, ch: u8) -> Option<&[u8]> {
    if acs_needed(term, utf8) {
        let entry = &term.acs[ch as usize];
        if entry[0] == 0 {
            None
        } else {
            Some(&entry[..1])
        }
    } else {
        FORWARD
            .binary_search_by_key(&ch, |entry| entry.0)
            .ok()
            .map(|index| FORWARD[index].1)
    }
}
pub fn acs_reverse_get(s: &[u8]) -> Option<u8> {
    if s.len() != 2 && s.len() != 3 {
        return None;
    }
    REVERSE
        .binary_search_by(|entry| entry.0.cmp(s))
        .ok()
        .map(|index| REVERSE[index].1)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tables_order_round_trip() {
        assert_eq!(FORWARD.len(), 36);
        assert!(FORWARD.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(REVERSE.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for &(key, bytes) in FORWARD {
            if let Some(reverse) = acs_reverse_get(bytes) {
                assert_eq!(reverse, key);
            }
        }
        assert_eq!(acs_reverse_get("─".as_bytes()), Some(b'q'));
        assert_eq!(acs_reverse_get("█".as_bytes()), None);
    }
    #[test]
    fn needed_truth_table_and_maps() {
        for u8cap in [None, Some(0), Some(1)] {
            for utf8 in [false, true] {
                let mut host = crate::tty::TtyHostInfo::default();
                let mut state = crate::term::tparm::TparmState::default();
                let mut caps = vec![
                    rmux_util::bytes::ByteString(b"clear=x".to_vec()),
                    rmux_util::bytes::ByteString(b"cup=x".to_vec()),
                ];
                if let Some(n) = u8cap {
                    caps.push(rmux_util::bytes::ByteString(format!("U8={n}").into_bytes()));
                }
                let term = TtyTerm::create(
                    &mut state,
                    b"test",
                    &caps,
                    &mut host,
                    &crate::tty::TtyOptions::default(),
                    None,
                )
                .unwrap();
                assert_eq!(acs_needed(&term, utf8), u8cap == Some(0) || !utf8);
                assert_eq!(
                    acs_get(&term, utf8, b'q'),
                    Some(if acs_needed(&term, utf8) {
                        &b"-"[..]
                    } else {
                        "─".as_bytes()
                    })
                );
            }
        }
    }
}
