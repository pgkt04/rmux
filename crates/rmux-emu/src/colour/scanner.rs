// Ported from tmux colour.c @ 8f25579c
/* $OpenBSD: colour.c,v 1.35 2026/07/06 14:29:10 nicm Exp $ */

/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2016 Avi Halachmi <avihpit@yahoo.com>
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
use super::{Colour, ColourParseError, colour_by_name};
use rmux_sys::number::{RgbScan, scan_cmyk, scan_rgb};

/// `colour_parseX11` (`colour.c:1175-1213`).
pub(super) fn parse(bytes: &[u8]) -> Result<Colour, ColourParseError> {
    let short = (bytes.len() == 12)
        .then(|| scan_rgb(bytes, RgbScan::Rgb2))
        .flatten()
        .or_else(|| {
            (bytes.len() == 7)
                .then(|| scan_rgb(bytes, RgbScan::Hash2))
                .flatten()
        })
        .or_else(|| scan_rgb(bytes, RgbScan::Decimal));
    if let Some([r, g, b]) = short {
        return Ok(Colour::rgb(r as u8, g as u8, b as u8));
    }
    let long = (bytes.len() == 18)
        .then(|| scan_rgb(bytes, RgbScan::Rgb4))
        .flatten()
        .or_else(|| {
            (bytes.len() == 13)
                .then(|| scan_rgb(bytes, RgbScan::Hash4))
                .flatten()
        });
    if let Some([r, g, b]) = long {
        return Ok(Colour::rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8));
    }
    if let Some([c, m, y, k]) = scan_cmyk(bytes, true).or_else(|| scan_cmyk(bytes, false))
        && [c, m, y, k].iter().all(|n| (0.0..=1.0).contains(n))
    {
        return Ok(Colour::rgb(
            ((1.0 - c) * (1.0 - k) * 255.0) as u8,
            ((1.0 - m) * (1.0 - k) * 255.0) as u8,
            ((1.0 - y) * (1.0 - k) * 255.0) as u8,
        ));
    }
    let start = bytes.iter().position(|b| *b != b' ').unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| *b != b' ')
        .map_or(start, |n| n + 1);
    colour_by_name(&bytes[start..end])
}
