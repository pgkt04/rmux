// Ported from tmux image-sixel.c @ 8f25579c
/*
 * Copyright (c) 2019 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER IN
 * AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT
 * OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
use std::io::Write;
use std::num::NonZeroU32;

const LIMIT: u32 = 10000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SixelError {
    InvalidPayload,
}

impl std::fmt::Display for SixelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid SIXEL payload or pixel growth beyond limits")
    }
}

impl std::error::Error for SixelError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SixelImage {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) xpixel: u32,
    pub(crate) ypixel: u32,
    pub(crate) set_ra: bool,
    pub(crate) ra_x: u32,
    pub(crate) ra_y: u32,
    pub(crate) colours: Vec<u32>,
    pub(crate) used_colours: u32,
    pub(crate) p2: u32,
    dx: u32,
    dy: u32,
    dc: u32,
    lines: Vec<Vec<u16>>,
}

// strtoul's LP64 saturation happens before assignment to the C unsigned int.
fn number(bytes: &[u8], start: usize) -> (u32, usize) {
    let mut p = start;
    while bytes.get(p).is_some_and(u8::is_ascii_whitespace) {
        p += 1;
    }
    let negative = bytes.get(p) == Some(&b'-');
    if negative || bytes.get(p) == Some(&b'+') {
        p += 1;
    }
    let first = p;
    let mut value = 0u64;
    let mut overflow = false;
    while let Some(&ch) = bytes.get(p).filter(|ch| ch.is_ascii_digit()) {
        match value
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(ch - b'0')))
        {
            Some(n) => value = n,
            None => {
                value = u64::MAX;
                overflow = true;
            }
        }
        p += 1;
    }
    if first == p {
        return (0, start);
    }
    if negative && !overflow {
        value = value.wrapping_neg();
    }
    (value as u32, p)
}

impl SixelImage {
    fn empty(p2: u32, xpixel: u32, ypixel: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            xpixel,
            ypixel,
            set_ra: false,
            ra_x: 0,
            ra_y: 0,
            colours: Vec::new(),
            used_colours: 0,
            p2,
            dx: 0,
            dy: 0,
            dc: 0,
            lines: Vec::new(),
        }
    }

    fn expand_rows(&mut self, y: u32) -> Result<(), SixelError> {
        if y <= self.y {
            return Ok(());
        }
        if y > LIMIT {
            return Err(SixelError::InvalidPayload);
        }
        self.lines.resize_with(y as usize, Vec::new);
        self.y = y;
        Ok(())
    }

    fn set_pixel(&mut self, x: u32, y: u32, colour: u32) -> Result<(), SixelError> {
        self.expand_rows(y.wrapping_add(1))?;
        let row = &mut self.lines[y as usize];
        let width = x.wrapping_add(1);
        if width > row.len() as u32 {
            if width > LIMIT {
                return Err(SixelError::InvalidPayload);
            }
            self.x = self.x.max(width);
            row.resize(self.x as usize, 0);
        }
        row[x as usize] = colour as u16;
        Ok(())
    }

    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        self.lines
            .get(y as usize)
            .and_then(|row| row.get(x as usize))
            .copied()
            .unwrap_or(0)
            .into()
    }

    fn write_pattern(&mut self, pattern: u8) -> Result<(), SixelError> {
        for i in 0..6 {
            if pattern & (1 << i) != 0 {
                self.set_pixel(self.dx, self.dy.wrapping_add(i), self.dc)?;
            }
        }
        Ok(())
    }

    pub fn parse(
        bytes: &[u8],
        p2: u32,
        xpixel: NonZeroU32,
        ypixel: NonZeroU32,
    ) -> Result<Self, SixelError> {
        if bytes.len() < 2 || bytes[0] != b'q' {
            return Err(SixelError::InvalidPayload);
        }
        let mut image = Self::empty(p2, xpixel.get(), ypixel.get());
        let mut p = 1;
        while p < bytes.len() {
            let ch = bytes[p];
            p += 1;
            match ch {
                b'"' | b'#' => {
                    let mut last = p;
                    while bytes
                        .get(last)
                        .is_some_and(|c| c.is_ascii_digit() || *c == b';')
                    {
                        last += 1;
                    }
                    let (first, mut end) = number(bytes, p);
                    if ch == b'#' {
                        if first > 1024 {
                            return Err(SixelError::InvalidPayload);
                        }
                        image.used_colours = image.used_colours.max(first + 1);
                        image.dc = first + 1;
                    }
                    if end == last || bytes.get(end) != Some(&b';') {
                        p = last;
                        continue;
                    }
                    let (second, next) = number(bytes, end + 1);
                    end = next;
                    if ch == b'"' && end == last {
                        p = last;
                        continue;
                    }
                    if end == last || bytes.get(end) != Some(&b';') {
                        return Err(SixelError::InvalidPayload);
                    }
                    let (third, next) = number(bytes, end + 1);
                    end = next;
                    if end == last || bytes.get(end) != Some(&b';') {
                        return Err(SixelError::InvalidPayload);
                    }
                    let (fourth, next) = number(bytes, end + 1);
                    end = next;
                    if ch == b'"' {
                        if end != last || third > LIMIT || fourth > LIMIT {
                            return Err(SixelError::InvalidPayload);
                        }
                        image.x = third;
                        image.expand_rows(fourth)?;
                        image.set_ra = true;
                        image.ra_x = third;
                        image.ra_y = fourth;
                    } else {
                        if end == last || bytes.get(end) != Some(&b';') {
                            return Err(SixelError::InvalidPayload);
                        }
                        let (fifth, end) = number(bytes, end + 1);
                        if end != last
                            || !matches!(second, 1 | 2)
                            || third > if second == 1 { 360 } else { 100 }
                            || fourth > 100
                            || fifth > 100
                        {
                            return Err(SixelError::InvalidPayload);
                        }
                        if image.colours.len() <= first as usize {
                            image.colours.resize(first as usize + 1, 0);
                        }
                        image.colours[first as usize] =
                            (second << 25) | (third << 16) | (fourth << 8) | fifth;
                    }
                    p = last;
                }
                b'!' => {
                    let start = p;
                    let mut count = 0u32;
                    while bytes.get(p).is_some_and(u8::is_ascii_digit) {
                        count = count
                            .saturating_mul(10)
                            .saturating_add(u32::from(bytes[p] - b'0'));
                        p += 1;
                        if p - start == 31 {
                            return Err(SixelError::InvalidPayload);
                        }
                    }
                    if p == start || p == bytes.len() || !(1..=LIMIT).contains(&count) {
                        return Err(SixelError::InvalidPayload);
                    }
                    let pattern = bytes[p].wrapping_sub(0x3f);
                    p += 1;
                    for _ in 0..count {
                        image.write_pattern(pattern)?;
                        image.dx = image.dx.wrapping_add(1);
                    }
                }
                b'-' => {
                    image.dx = 0;
                    image.dy = image.dy.wrapping_add(6);
                }
                b'$' => image.dx = 0,
                0..=0x1f | 0x80..=0xff => (),
                0x3f..=0x7e => {
                    image.write_pattern(ch - 0x3f)?;
                    image.dx = image.dx.wrapping_add(1);
                }
                _ => return Err(SixelError::InvalidPayload),
            }
        }
        if image.x == 0 || image.y == 0 {
            return Err(SixelError::InvalidPayload);
        }
        Ok(image)
    }

    pub fn size_in_cells(&self) -> (u32, u32) {
        (self.x.div_ceil(self.xpixel), self.y.div_ceil(self.ypixel))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scale(
        &self,
        xpixel: Option<NonZeroU32>,
        ypixel: Option<NonZeroU32>,
        ox: u32,
        oy: u32,
        mut sx: u32,
        mut sy: u32,
        colours: bool,
    ) -> Option<Self> {
        let (cx, cy) = self.size_in_cells();
        if ox >= cx || oy >= cy {
            return None;
        }
        if ox.wrapping_add(sx) >= cx {
            sx = cx - ox;
        }
        if oy.wrapping_add(sy) >= cy {
            sy = cy - oy;
        }
        let xpixel = xpixel.map_or(self.xpixel, NonZeroU32::get);
        let ypixel = ypixel.map_or(self.ypixel, NonZeroU32::get);
        let pox = ox.wrapping_mul(self.xpixel);
        let poy = oy.wrapping_mul(self.ypixel);
        let psx = sx.wrapping_mul(self.xpixel);
        let psy = sy.wrapping_mul(self.ypixel);
        let tsx = sx.wrapping_mul(xpixel);
        let tsy = sy.wrapping_mul(ypixel);
        let mut new = Self::empty(self.p2, xpixel, ypixel);
        new.set_ra = self.set_ra;
        new.ra_x = self.ra_x.saturating_sub(pox).min(psx).wrapping_mul(xpixel) / self.xpixel;
        new.ra_y = self.ra_y.saturating_sub(poy).min(psy).wrapping_mul(ypixel) / self.ypixel;
        new.used_colours = self.used_colours;
        if tsx != 0 {
            new.x = tsx.min(LIMIT);
            new.y = tsy.min(LIMIT);
            if new.y == 0 {
                new.x = 0;
            }
            new.lines = (0..new.y).map(|_| vec![0; new.x as usize]).collect();
            for (y, row) in new.lines.iter_mut().enumerate() {
                let py = (f64::from(poy) + y as f64 * f64::from(psy) / f64::from(tsy)) as u32;
                for (x, pixel) in row.iter_mut().enumerate() {
                    let px = (f64::from(pox) + x as f64 * f64::from(psx) / f64::from(tsx)) as u32;
                    *pixel = self.pixel(px, py) as u16;
                }
            }
        }
        if colours {
            new.colours.clone_from(&self.colours);
        }
        Some(new)
    }

    pub fn log(&self) {
        if rmux_util::log::level().0 == 0 {
            return;
        }
        let (cx, cy) = self.size_in_cells();
        rmux_util::log_debug!("sixel_log: image {}x{} ({cx}x{cy})", self.x, self.y);
        for (i, c) in self.colours.iter().enumerate() {
            rmux_util::log_debug!("sixel_log: colour {i} is {c:07x}");
        }
        let mut line = String::with_capacity(self.x as usize);
        for (y, row) in self.lines.iter().enumerate() {
            line.clear();
            for x in 0..self.x as usize {
                line.push(match row.get(x) {
                    None => '_',
                    Some(0) => '.',
                    Some(c) => char::from(b'0' + ((c - 1) % 10) as u8),
                });
            }
            rmux_util::log_debug!("sixel_log: {y:4}: {line}");
        }
    }

    pub fn print(&self, map: Option<&Self>) -> Option<Vec<u8>> {
        if self.used_colours == 0 {
            return None;
        }
        let mut out = Vec::with_capacity(8192);
        write!(out, "\x1bP9;{}q", self.p2).unwrap();
        if self.set_ra {
            write!(out, "\"1;1;{};{}", self.ra_x, self.ra_y).unwrap();
        }
        for (i, c) in map.unwrap_or(self).colours.iter().enumerate() {
            write!(
                out,
                "#{i};{};{};{};{}",
                c >> 25,
                (c >> 16) & 0x1ff,
                (c >> 8) & 0xff,
                c & 0xff
            )
            .unwrap();
        }
        let mut chunks: Vec<Chunk> = (0..self.used_colours).map(|_| Chunk::default()).collect();
        let mut active = Vec::with_capacity(chunks.len());
        for y in (0..self.y).step_by(6) {
            active.clear();
            for x in 0..self.x {
                let mut colors = [0; 6];
                for (i, c) in colors.iter_mut().enumerate() {
                    *c = self.pixel(x, y + i as u32);
                    if *c != 0 {
                        chunks[(*c - 1) as usize].next_pattern |= 1 << i;
                    }
                }
                for c in colors.into_iter().filter(|c| *c != 0) {
                    let index = (c - 1) as usize;
                    let chunk = &mut chunks[index];
                    if chunk.next_x == x + 1 {
                        continue;
                    }
                    if chunk.next_y < y + 1 {
                        chunk.next_y = y + 1;
                        active.push(index);
                    }
                    let gap = x - chunk.next_x;
                    if chunk.pattern != chunk.next_pattern || gap != 0 {
                        repeat(&mut chunk.data, chunk.count, chunk.pattern + 0x3f);
                        repeat(&mut chunk.data, gap, b'?');
                        chunk.pattern = chunk.next_pattern;
                        chunk.count = 0;
                    }
                    chunk.count += 1;
                    chunk.next_pattern = 0;
                    chunk.next_x = x + 1;
                }
            }
            for &c in &active {
                let chunk = &mut chunks[c];
                write!(out, "#{c}").unwrap();
                out.extend_from_slice(&chunk.data);
                repeat(&mut out, chunk.count, chunk.pattern + 0x3f);
                out.push(b'$');
                chunk.data.clear();
                chunk.next_x = 0;
                chunk.count = 0;
            }
            if out.last() == Some(&b'$') {
                out.pop();
            }
            out.push(b'-');
        }
        if out.last() == Some(&b'-') {
            out.pop();
        }
        out.extend_from_slice(b"\x1b\\");
        Some(out)
    }
}

#[derive(Default)]
struct Chunk {
    next_x: u32,
    next_y: u32,
    count: u32,
    pattern: u8,
    next_pattern: u8,
    data: Vec<u8>,
}
fn repeat(out: &mut Vec<u8>, count: u32, pattern: u8) {
    if count <= 3 {
        for _ in 0..count {
            out.push(pattern);
        }
    } else {
        let mut digits = [0u8; 10];
        let mut at = digits.len();
        let mut n = count;
        while n != 0 {
            at -= 1;
            digits[at] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        out.push(b'!');
        out.extend_from_slice(&digits[at..]);
        out.push(pattern);
    }
}

#[cfg(test)]
#[path = "sixel_tests.rs"]
mod tests;
