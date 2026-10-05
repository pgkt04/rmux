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

fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 11 | 12)
}
struct Scanner<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Scanner<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn literal(&mut self, literal: &[u8]) -> Option<()> {
        if !self.bytes[self.offset..].starts_with(literal) {
            return None;
        }
        self.offset += literal.len();
        Some(())
    }
    fn skip_space(&mut self) {
        while self.bytes.get(self.offset).is_some_and(|b| space(*b)) {
            self.offset += 1;
        }
    }
    fn integer(&mut self, radix: u32, width: usize) -> Option<u32> {
        self.skip_space();
        let end = self.offset.saturating_add(width).min(self.bytes.len());
        let mut i = self.offset;
        let negative = self.bytes.get(i) == Some(&b'-');
        if matches!(self.bytes.get(i), Some(b'+' | b'-')) && i < end {
            i += 1;
        }
        if radix == 16
            && i + 2 <= end
            && self.bytes[i] == b'0'
            && matches!(self.bytes[i + 1], b'x' | b'X')
        {
            i += 2;
        }
        let start = i;
        let mut n = 0u64;
        while i < end {
            let d = match self.bytes[i] {
                b'0'..=b'9' => u32::from(self.bytes[i] - b'0'),
                b'a'..=b'f' => u32::from(self.bytes[i] - b'a' + 10),
                b'A'..=b'F' => u32::from(self.bytes[i] - b'A' + 10),
                _ => break,
            };
            if d >= radix {
                break;
            }
            n = n
                .saturating_mul(u64::from(radix))
                .saturating_add(u64::from(d));
            i += 1;
        }
        if i == start {
            return None;
        }
        self.offset = i;
        let n = if radix == 10 {
            n.min(if negative {
                1u64 << 63
            } else {
                i64::MAX as u64
            })
        } else {
            n
        };
        Some(if negative {
            (n as u32).wrapping_neg()
        } else {
            n as u32
        })
    }
    fn float(&mut self) -> Option<f64> {
        self.skip_space();
        let start = self.offset;
        let mut i = start;
        if matches!(self.bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let sign = if self.bytes.get(start) == Some(&b'-') {
            -1.0
        } else {
            1.0
        };
        if self
            .bytes
            .get(i..i + 3)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"inf"))
        {
            i += 3;
            if self
                .bytes
                .get(i..i + 5)
                .is_some_and(|s| s.eq_ignore_ascii_case(b"inity"))
            {
                i += 5;
            }
            self.offset = i;
            return Some(sign * f64::INFINITY);
        }
        if self
            .bytes
            .get(i..i + 3)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"nan"))
        {
            self.offset = i + 3;
            return Some(f64::NAN);
        }
        let hex = self
            .bytes
            .get(i..i + 2)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"0x"));
        if hex {
            i += 2;
        }
        let digits = |b: u8| {
            if hex {
                b.is_ascii_hexdigit()
            } else {
                b.is_ascii_digit()
            }
        };
        let mut count = 0;
        let mut value = 0.0;
        let radix = if hex { 16.0 } else { 10.0 };
        while self.bytes.get(i).is_some_and(|b| digits(*b)) {
            value = value * radix
                + f64::from(
                    (self.bytes[i] as char)
                        .to_digit(if hex { 16 } else { 10 })
                        .unwrap(),
                );
            count += 1;
            i += 1;
        }
        if self.bytes.get(i) == Some(&b'.') {
            i += 1;
            let mut factor = 1.0 / radix;
            while self.bytes.get(i).is_some_and(|b| digits(*b)) {
                value += f64::from(
                    (self.bytes[i] as char)
                        .to_digit(if hex { 16 } else { 10 })
                        .unwrap(),
                ) * factor;
                factor /= radix;
                count += 1;
                i += 1;
            }
        }
        if count == 0 {
            return None;
        }
        let mantissa_end = i;
        if self.bytes.get(i).is_some_and(|b| {
            if hex {
                matches!(b, b'p' | b'P')
            } else {
                matches!(b, b'e' | b'E')
            }
        }) {
            i += 1;
            let exponent_start = i;
            if matches!(self.bytes.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let exponent_digits = i;
            while self.bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i > exponent_digits {
                let exponent = std::str::from_utf8(&self.bytes[exponent_start..i])
                    .ok()?
                    .parse::<i32>()
                    .unwrap_or(if self.bytes[exponent_start] == b'-' {
                        i32::MIN
                    } else {
                        i32::MAX
                    });
                value *= (if hex { 2.0_f64 } else { 10.0_f64 }).powi(exponent);
            } else {
                i = mantissa_end;
            }
        }
        self.offset = i;
        if hex {
            Some(sign * value)
        } else {
            let number_end = if matches!(
                self.bytes.get(i.wrapping_sub(1)),
                Some(b'+' | b'-' | b'e' | b'E')
            ) {
                mantissa_end
            } else {
                i
            };
            std::str::from_utf8(&self.bytes[start..number_end])
                .ok()?
                .parse()
                .ok()
        }
    }
}
fn hex(bytes: &[u8], prefix: &[u8], width: usize, separator: &[u8]) -> Option<(u32, u32, u32)> {
    let mut s = Scanner::new(bytes);
    s.literal(prefix)?;
    let r = s.integer(16, width)?;
    s.literal(separator)?;
    let g = s.integer(16, width)?;
    s.literal(separator)?;
    Some((r, g, s.integer(16, width)?))
}
fn decimal(bytes: &[u8]) -> Option<(u32, u32, u32)> {
    let mut s = Scanner::new(bytes);
    let r = s.integer(10, usize::MAX)?;
    s.literal(b",")?;
    let g = s.integer(10, usize::MAX)?;
    s.literal(b",")?;
    Some((r, g, s.integer(10, usize::MAX)?))
}
fn cmy(bytes: &[u8], four: bool) -> Option<(f64, f64, f64, f64)> {
    let mut s = Scanner::new(bytes);
    s.literal(if four { b"cmyk:" } else { b"cmy:" })?;
    let c = s.float()?;
    s.literal(b"/")?;
    let m = s.float()?;
    s.literal(b"/")?;
    let y = s.float()?;
    let k = if four {
        s.literal(b"/")?;
        s.float()?
    } else {
        0.0
    };
    Some((c, m, y, k))
}
pub(super) fn parse(bytes: &[u8]) -> Result<Colour, ColourParseError> {
    let short = (if bytes.len() == 12 {
        hex(bytes, b"rgb:", 2, b"/")
    } else {
        None
    })
    .or_else(|| {
        if bytes.len() == 7 {
            hex(bytes, b"#", 2, b"")
        } else {
            None
        }
    })
    .or_else(|| decimal(bytes));
    if let Some((r, g, b)) = short {
        return Ok(Colour::rgb(r as u8, g as u8, b as u8));
    }
    let long = (if bytes.len() == 18 {
        hex(bytes, b"rgb:", 4, b"/")
    } else {
        None
    })
    .or_else(|| {
        if bytes.len() == 13 {
            hex(bytes, b"#", 4, b"")
        } else {
            None
        }
    });
    if let Some((r, g, b)) = long {
        return Ok(Colour::rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8));
    }
    if let Some((c, m, y, k)) = cmy(bytes, true).or_else(|| cmy(bytes, false))
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
