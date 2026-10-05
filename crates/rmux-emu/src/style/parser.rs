// Ported from tmux style.c @ 8f25579c
/* $OpenBSD: style.c,v 1.47 2026/06/29 17:08:52 nicm Exp $ */

/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2014 Tiago Cunha <tcunha@users.sourceforge.net>
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
use super::*;
use crate::{attributes::parse_attributes, cell::GridAttributes};
use rmux_util::strtonum::strtonum;
fn eq(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}
fn prefix<'a>(a: &'a [u8], b: &[u8]) -> Option<&'a [u8]> {
    a.get(..b.len()).filter(|s| eq(s, b)).map(|_| &a[b.len()..])
}
fn number(a: &[u8], max: u32) -> Result<u32, ()> {
    strtonum(a, 0, i64::from(max))
        .map(|n| n as u32)
        .map_err(|_| ())
}
fn range(s: &mut Style, input: &[u8]) -> Result<(), ()> {
    let (name, argument) = match input.iter().position(|b| *b == b'|') {
        Some(n) => (&input[..n], Some(&input[n + 1..])),
        None => (input, None),
    };
    if argument == Some(b"".as_slice()) {
        return Err(());
    }
    let (kind, n, string) = if eq(name, b"left") || eq(name, b"right") {
        if argument.is_some() {
            return Err(());
        }
        (
            if eq(name, b"left") {
                StyleRangeType::Left
            } else {
                StyleRangeType::Right
            },
            0,
            b"".as_slice(),
        )
    } else if eq(name, b"control") {
        (
            StyleRangeType::Control,
            number(argument.ok_or(())?, 9)?,
            b"".as_slice(),
        )
    } else if eq(name, b"pane") || eq(name, b"session") {
        let a = argument.ok_or(())?;
        if a.first() != Some(&if eq(name, b"pane") { b'%' } else { b'$' }) {
            return Err(());
        }
        (
            if eq(name, b"pane") {
                StyleRangeType::Pane
            } else {
                StyleRangeType::Session
            },
            number(&a[1..], u32::MAX)?,
            b"".as_slice(),
        )
    } else if eq(name, b"window") {
        (
            StyleRangeType::Window,
            number(argument.ok_or(())?, u32::MAX)?,
            b"".as_slice(),
        )
    } else if eq(name, b"user") {
        (StyleRangeType::User, 0, argument.ok_or(())?)
    } else {
        return Ok(());
    };
    s.range_type = kind;
    s.range_argument = n;
    s.range_string = [0; 16];
    let len = string.len().min(15);
    s.range_string[..len].copy_from_slice(&string[..len]);
    Ok(())
}
fn token(
    s: &mut Style,
    base: &GridCell,
    t: &[u8],
    links: &mut HyperlinkRegistry,
) -> Result<(), Option<HyperlinkError>> {
    let invalid = || None;
    if eq(t, b"default") {
        s.gc.fg = base.fg;
        s.gc.bg = base.bg;
        s.gc.us = base.us;
        s.gc.attr = base.attr;
        s.gc.flags = base.flags;
        s.link = HyperlinkId::NONE;
    } else if eq(t, b"ignore") {
        s.ignore = true;
    } else if eq(t, b"noignore") {
        s.ignore = false;
    } else if eq(t, b"push-default") {
        s.default_type = StyleDefaultType::Push;
    } else if eq(t, b"pop-default") {
        s.default_type = StyleDefaultType::Pop;
    } else if eq(t, b"set-default") {
        s.default_type = StyleDefaultType::Set;
    } else if eq(t, b"nolist") {
        s.list = StyleList::Off;
    } else if let Some(v) = prefix(t, b"list=") {
        s.list = if eq(v, b"on") {
            StyleList::On
        } else if eq(v, b"focus") {
            StyleList::Focus
        } else if eq(v, b"left-marker") {
            StyleList::LeftMarker
        } else if eq(v, b"right-marker") {
            StyleList::RightMarker
        } else {
            return Err(None);
        };
    } else if eq(t, b"norange") {
        s.range_type = StyleRangeType::None;
        s.range_argument = 0;
        s.range_string = [0; 16];
    } else if let Some(v) = prefix(t, b"range=").filter(|v| !v.is_empty()) {
        range(s, v).map_err(|_| invalid())?;
    } else if eq(t, b"noalign") {
        s.align = StyleAlign::Default;
    } else if let Some(v) = prefix(t, b"align=").filter(|v| !v.is_empty()) {
        s.align = if eq(v, b"left") {
            StyleAlign::Left
        } else if eq(v, b"centre") {
            StyleAlign::Centre
        } else if eq(v, b"right") {
            StyleAlign::Right
        } else if eq(v, b"absolute-centre") {
            StyleAlign::AbsoluteCentre
        } else {
            return Err(None);
        };
    } else if let Some(v) = prefix(t, b"fill=").filter(|v| !v.is_empty()) {
        s.fill = parse_colour(v).map_err(|_| invalid())?;
    } else if let Some(v) = prefix(t, b"dim=").filter(|v| !v.is_empty()) {
        s.dim = number(v.strip_suffix(b"%").unwrap_or(v), 100).map_err(|_| invalid())?;
    } else if t.len() > 3 && eq(&t[1..3], b"g=") {
        let c = parse_colour(&t[3..]).map_err(|_| invalid())?;
        match t[0] {
            b'f' | b'F' => s.gc.fg = if c == Colour::DEFAULT { base.fg } else { c },
            b'b' | b'B' => s.gc.bg = if c == Colour::DEFAULT { base.bg } else { c },
            _ => return Err(None),
        }
    } else if let Some(v) = prefix(t, b"us=").filter(|v| !v.is_empty()) {
        let c = parse_colour(v).map_err(|_| invalid())?;
        s.gc.us = if c == Colour::DEFAULT { base.us } else { c };
    } else if eq(t, b"none") {
        s.gc.attr = GridAttributes(0);
    } else if let Some(v) = prefix(t, b"no").filter(|v| !v.is_empty()) {
        if v == b"link" {
            s.link = HyperlinkId::NONE;
        } else if v == b"attr" {
            s.gc.attr.insert(GridAttributes::NOATTR);
        } else {
            s.gc.attr
                .remove(parse_attributes(v).map_err(|_| invalid())?);
        }
    } else if let Some(v) = prefix(t, b"width=").filter(|v| !v.is_empty()) {
        let percent = v.len() > 1 && v.ends_with(b"%");
        s.width = number(
            if percent { &v[..v.len() - 1] } else { v },
            if percent { 100 } else { u32::MAX },
        )
        .map_err(|_| invalid())? as i32;
        s.width_percentage = percent;
    } else if let Some(v) = prefix(t, b"pad=").filter(|v| !v.is_empty()) {
        s.pad = number(v, u32::MAX).map_err(|_| invalid())? as i32;
    } else if let Some(v) = prefix(t, b"link=") {
        s.link = if v.is_empty() {
            HyperlinkId::NONE
        } else {
            links.put_style(v).map_err(Some)?
        };
    } else {
        s.gc.attr
            .insert(parse_attributes(t).map_err(|_| invalid())?);
    }
    Ok(())
}
pub(super) fn parse(
    s: &mut Style,
    base: &GridCell,
    bytes: &[u8],
    links: &mut HyperlinkRegistry,
) -> Result<(), StyleParseError> {
    let saved = *s;
    let delimiter = |b: u8| matches!(b, b' ' | b',' | b'\n');
    let mut offset = 0;
    while offset < bytes.len() {
        if delimiter(bytes[offset]) {
            offset += 1;
            continue;
        }
        let end = bytes[offset..]
            .iter()
            .position(|b| delimiter(*b))
            .map_or(bytes.len(), |n| offset + n);
        let result = if end - offset > 255 {
            Err(None)
        } else {
            token(s, base, &bytes[offset..end], links)
        };
        if let Err(error) = result {
            *s = saved;
            return Err(match error {
                Some(error) => StyleParseError::Hyperlink { offset, error },
                None => StyleParseError::InvalidToken { offset },
            });
        }
        offset = end;
    }
    Ok(())
}
