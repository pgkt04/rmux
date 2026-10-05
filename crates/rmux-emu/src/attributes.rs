// Ported from tmux attributes.c @ 8f25579c
/* $OpenBSD: attributes.c,v 1.13 2026/07/02 08:51:05 nicm Exp $ */
/*
 * Copyright (c) 2009 Joshua Elsasser <josh@elsasser.org>
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
use crate::cell::GridAttributes;
use rmux_util::bytes::cstr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttributeParseError;
impl std::fmt::Display for AttributeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid attributes")
    }
}
impl std::error::Error for AttributeParseError {}

const NAMES: &[(&[u8], GridAttributes)] = &[
    (b"acs", GridAttributes::CHARSET),
    (b"bright", GridAttributes::BRIGHT),
    (b"dim", GridAttributes::DIM),
    (b"underscore", GridAttributes::UNDERSCORE),
    (b"blink", GridAttributes::BLINK),
    (b"reverse", GridAttributes::REVERSE),
    (b"hidden", GridAttributes::HIDDEN),
    (b"italics", GridAttributes::ITALICS),
    (b"strikethrough", GridAttributes::STRIKETHROUGH),
    (b"double-underscore", GridAttributes::UNDERSCORE_2),
    (b"curly-underscore", GridAttributes::UNDERSCORE_3),
    (b"dotted-underscore", GridAttributes::UNDERSCORE_4),
    (b"dashed-underscore", GridAttributes::UNDERSCORE_5),
    (b"overline", GridAttributes::OVERLINE),
    (b"noattr", GridAttributes::NOATTR),
];
fn delimiter(byte: u8) -> bool {
    matches!(byte, b' ' | b',' | b'|')
}
pub fn parse_attributes(bytes: &[u8]) -> Result<GridAttributes, AttributeParseError> {
    let bytes = cstr(bytes);
    if bytes.is_empty() || delimiter(bytes[0]) || delimiter(bytes[bytes.len() - 1]) {
        return Err(AttributeParseError);
    }
    if bytes.eq_ignore_ascii_case(b"none") || bytes.eq_ignore_ascii_case(b"default") {
        return Ok(GridAttributes(0));
    }
    let mut attr = GridAttributes(0);
    for token in bytes.split(|b| delimiter(*b)).filter(|s| !s.is_empty()) {
        let bit = if token.eq_ignore_ascii_case(b"bold") {
            GridAttributes::BRIGHT
        } else {
            NAMES[..NAMES.len() - 1]
                .iter()
                .find(|(name, _)| token.eq_ignore_ascii_case(name))
                .ok_or(AttributeParseError)?
                .1
        };
        attr.insert(bit);
    }
    Ok(attr)
}
pub fn write_attributes(attributes: GridAttributes, out: &mut Vec<u8>) {
    if attributes.bits() == 0 {
        out.extend_from_slice(b"none");
        return;
    }
    let mut comma = false;
    for &(name, bit) in NAMES {
        if attributes.intersects(bit) {
            if comma {
                out.push(b',');
            }
            out.extend_from_slice(name);
            comma = true;
        }
    }
}
