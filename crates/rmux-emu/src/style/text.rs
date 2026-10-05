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
use crate::{attributes::write_attributes, colour::write_colour};
use std::io::Write;
pub(super) fn write(style: &Style, links: &HyperlinkRegistry, out: &mut Vec<u8>) {
    let start = out.len();
    let buf = out;
    let mut comma = false;
    let mut field = |buf: &mut Vec<u8>, text: &[u8]| {
        if comma {
            buf.push(b',');
        }
        buf.extend_from_slice(text);
        comma = true;
    };
    let label = match style.list {
        StyleList::Off => b"".as_slice(),
        StyleList::On => b"on",
        StyleList::Focus => b"focus",
        StyleList::LeftMarker => b"left-marker",
        StyleList::RightMarker => b"right-marker",
    };
    if style.list != StyleList::Off {
        field(buf, b"list=");
        buf.extend_from_slice(label);
    }
    if style.range_type != StyleRangeType::None {
        field(buf, b"range=");
        match style.range_type {
            StyleRangeType::Left => buf.extend_from_slice(b"left"),
            StyleRangeType::Right => buf.extend_from_slice(b"right"),
            StyleRangeType::Pane => write!(buf, "pane|%{}", style.range_argument).unwrap(),
            StyleRangeType::Window => write!(buf, "window|{}", style.range_argument).unwrap(),
            StyleRangeType::Session => write!(buf, "session|${}", style.range_argument).unwrap(),
            StyleRangeType::User => {
                buf.extend_from_slice(b"user|");
                let string = cstr(&style.range_string);
                buf.extend_from_slice(&string[..string.len().min(15)]);
            }
            StyleRangeType::Control => buf.extend_from_slice(label),
            StyleRangeType::None => {}
        }
    }
    if style.align != StyleAlign::Default {
        field(buf, b"align=");
        buf.extend_from_slice(match style.align {
            StyleAlign::Left => b"left",
            StyleAlign::Centre => b"centre",
            StyleAlign::Right => b"right",
            StyleAlign::AbsoluteCentre => b"absolute-centre",
            StyleAlign::Default => b"",
        });
    }
    if style.default_type != StyleDefaultType::Base {
        field(
            buf,
            match style.default_type {
                StyleDefaultType::Push => b"push-default",
                StyleDefaultType::Pop => b"pop-default",
                StyleDefaultType::Set => b"set-default",
                StyleDefaultType::Base => b"",
            },
        );
    }
    if style.fill != Colour::DEFAULT {
        field(buf, b"fill=");
        write_colour(style.fill, buf);
    }
    if style.dim != 0 {
        field(buf, b"dim=");
        write!(buf, "{}%", style.dim).unwrap();
    }
    for (name, colour) in [
        (b"fg=".as_slice(), style.gc.fg),
        (b"bg=", style.gc.bg),
        (b"us=", style.gc.us),
    ] {
        if colour != Colour::DEFAULT {
            field(buf, name);
            write_colour(colour, buf);
        }
    }
    if style.gc.attr.bits() != 0 {
        field(buf, b"");
        write_attributes(style.gc.attr, buf);
    }
    if style.width >= 0 {
        field(buf, b"width=");
        write!(
            buf,
            "{}{}",
            style.width,
            if style.width_percentage { "%" } else { "" }
        )
        .unwrap();
    }
    if style.pad >= 0 {
        field(buf, b"pad=");
        write!(buf, "{}", style.pad).unwrap();
    }
    if let Some(uri) = style.link_uri(links) {
        field(buf, b"link=");
        buf.extend_from_slice(uri);
    }
    if buf.len() == start {
        buf.extend_from_slice(b"default");
    } else {
        buf.truncate((start + 2047).min(buf.len()));
    }
}
