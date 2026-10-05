// Ported from tmux utf8.c @ 8f25579c (utf8_from_data, utf8_to_data, utf8_build_one, utf8_set, utf8_copy, utf8_towc, utf8_has_whitespace, utf8_fromwc, utf8_open, utf8_append, utf8_strvis, utf8_stravis, utf8_stravisx, utf8_isvalid, utf8_sanitize, utf8_strlen, utf8_strwidth, utf8_fromcstr, utf8_tocstr, utf8_cstrwidth, utf8_padcstr, utf8_rpadcstr, utf8_cstrhas)
//! UTF-8 characters, the byte-at-a-time decoder, compact characters, and the
//! string helpers of `utf8.c`.

pub mod combined;
pub mod table;
pub mod tables;
pub mod width;

#[cfg(test)]
mod tests;

use std::fmt;

use crate::bytes::{ByteString, cstr};
use crate::vis::{VisFlags, vis};
pub use width::{WidthCache, width_of, with_width_cache};

/// Bytes of one character and its combining characters (`tmux.h:722`).
pub const UTF8_SIZE: usize = 32;
/// `width` of an invalid sequence (`tmux.h:729`).
pub const INVALID_WIDTH: u8 = 0xff;

/// `enum utf8_state` (`tmux.h:731-735`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Utf8State {
    More,
    Done,
    Error,
}

/// `struct utf8_data` (`tmux.h:723-730`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utf8Data {
    pub data: [u8; UTF8_SIZE],
    pub have: u8,
    pub size: u8,
    pub width: u8,
}

impl Default for Utf8Data {
    fn default() -> Utf8Data {
        Utf8Data {
            data: [0; UTF8_SIZE],
            have: 0,
            size: 0,
            width: 0,
        }
    }
}

/// Lossy display of raw bytes for `log_debug` messages (`%.*s` in C).
struct Raw<'a>(&'a [u8]);

impl fmt::Display for Raw<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.0.utf8_chunks() {
            f.write_str(chunk.valid())?;
            if !chunk.invalid().is_empty() {
                f.write_str("\u{FFFD}")?;
            }
        }
        Ok(())
    }
}

impl Utf8Data {
    /// `utf8_set`: one byte with size 1 and width 1.
    pub fn set(ch: u8) -> Utf8Data {
        let mut ud = Utf8Data {
            have: 1,
            size: 1,
            width: 1,
            ..Utf8Data::default()
        };
        ud.data[0] = ch;
        ud
    }

    /// `utf8_copy`: copy, then zero the bytes after `size`.
    pub fn copy_from(&mut self, from: &Utf8Data) {
        *self = *from;
        let size = usize::from(self.size).min(UTF8_SIZE);
        self.data[size..].fill(0);
    }

    /// The active bytes `data[..size]`.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..usize::from(self.size).min(UTF8_SIZE)]
    }

    /// `utf8_open`: start a sequence from its lead byte. `Err` is
    /// `UTF8_ERROR`; `Ok` holds the `UTF8_MORE` state with `have == 1`.
    #[allow(clippy::result_unit_err)]
    pub fn open(ch: u8) -> Result<Utf8Data, ()> {
        let size = match ch {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Err(()),
        };
        let mut ud = Utf8Data {
            size,
            ..Utf8Data::default()
        };
        ud.append(ch);
        Ok(ud)
    }

    fn append_byte(&mut self, ch: u8) -> bool {
        if self.have >= self.size {
            crate::fatalx!("UTF-8 character overflow");
        }
        if usize::from(self.size) > UTF8_SIZE {
            crate::fatalx!("UTF-8 character size too large");
        }

        if self.have != 0 && (ch & 0xc0) != 0x80 {
            self.width = INVALID_WIDTH;
        }

        self.data[usize::from(self.have)] = ch;
        self.have += 1;
        self.have == self.size
    }

    /// `utf8_append`: add a byte, computing the width on the last one.
    pub fn append(&mut self, ch: u8) -> Utf8State {
        if !self.append_byte(ch) {
            return Utf8State::More;
        }
        if self.width == INVALID_WIDTH {
            return Utf8State::Error;
        }
        match width_of(self) {
            Some(width) => {
                self.width = width;
                Utf8State::Done
            }
            None => Utf8State::Error,
        }
    }

    /// `utf8_append` with `utf8_no_width` set: no width check or lookup.
    pub fn append_no_width(&mut self, ch: u8) -> Utf8State {
        if self.append_byte(ch) {
            Utf8State::Done
        } else {
            Utf8State::More
        }
    }

    /// `utf8_towc`: the first code point of the buffer.
    pub fn to_wc(&self) -> Option<u32> {
        match rmux_sys::locale::mbtowc(self.bytes()) {
            Some(wc) => {
                crate::log_debug!("UTF-8 {} is U+{wc:06X}", Raw(self.bytes()));
                Some(wc)
            }
            None => {
                crate::log_debug!("UTF-8 {}, mbtowc() failed", Raw(self.bytes()));
                None
            }
        }
    }

    /// `utf8_fromwc`: encode a code point and compute its width.
    pub fn from_wc(wc: u32) -> Option<Utf8Data> {
        let mut ud = Utf8Data::default();
        let size = match rmux_sys::locale::wctomb(wc, &mut ud.data) {
            None => {
                crate::log_debug!("UTF-8 {wc}, wctomb() failed");
                return None;
            }
            Some(0) => return None,
            Some(size) => size,
        };
        ud.size = size as u8;
        ud.have = ud.size;
        ud.width = width_of(&ud)?;
        Some(ud)
    }

    /// `utf8_has_whitespace`: any code point in the buffer is one of the 25
    /// Unicode whitespace characters.
    pub fn has_whitespace(&self) -> bool {
        let bytes = self.bytes();
        let mut offset = 0;
        while offset < bytes.len() {
            let ch = bytes[offset];
            let (wc, size) = if ch < 0x80 {
                (u32::from(ch), 1)
            } else {
                let size = match ch {
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => return false,
                };
                if size > bytes.len() - offset {
                    return false;
                }
                let mut tmp = Utf8Data::default();
                tmp.data[..size].copy_from_slice(&bytes[offset..offset + size]);
                tmp.size = size as u8;
                tmp.have = tmp.size;
                let Some(wc) = tmp.to_wc() else {
                    return false;
                };
                (wc, size)
            };
            offset += size;
            if tables::WHITESPACE.contains(&wc) {
                return true;
            }
        }
        false
    }
}

/// `utf8_char` (`tmux.h:715`): bits 29-31 `width + 1`, bits 24-28 `size`,
/// bits 0-23 inline bytes or an intern table index (`utf8.c:256-260`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Utf8Char(pub u32);

impl Utf8Char {
    /// `UTF8_GET_SIZE`.
    pub const fn size(self) -> u8 {
        ((self.0 >> 24) & 0x1f) as u8
    }

    /// `UTF8_GET_WIDTH`, truncated to `u_char` like the C store.
    pub const fn width(self) -> u8 {
        (self.0 >> 29).wrapping_sub(1) as u8
    }

    /// Low 24 bits: inline bytes for `size <= 3`, else a table index.
    pub const fn payload(self) -> u32 {
        self.0 & 0xff_ffff
    }

    const fn pack(size: u32, width: u32, payload: u32) -> Utf8Char {
        Utf8Char((size << 24) | ((width + 1) << 29) | payload)
    }
}

/// `utf8_build_one`: one ASCII byte with size 1 and width 1.
pub fn build_one(ch: u8) -> Utf8Char {
    Utf8Char::pack(1, 1, u32::from(ch))
}

/// `utf8_from_data`: `Done` with the compact character, or `Error` with the
/// width-matched replacement (`utf8.c:490-497`).
pub fn from_data(ud: &Utf8Data) -> (Utf8Char, Utf8State) {
    if ud.width > 2 {
        crate::fatalx!("invalid UTF-8 width: {}", ud.width);
    }

    let index = if usize::from(ud.size) > UTF8_SIZE {
        None
    } else if ud.size <= 3 {
        Some((u32::from(ud.data[2]) << 16) | (u32::from(ud.data[1]) << 8) | u32::from(ud.data[0]))
    } else {
        table::with_table(|table| table.put(ud.bytes()))
    };
    match index {
        Some(index) => {
            let uc = Utf8Char::pack(u32::from(ud.size), u32::from(ud.width), index);
            crate::log_debug!(
                "utf8_from_data: ({} {} {}) -> {:08x}",
                ud.width,
                ud.size,
                Raw(ud.bytes()),
                uc.0
            );
            (uc, Utf8State::Done)
        }
        None => {
            let uc = match ud.width {
                0 => Utf8Char::pack(0, 0, 0),
                1 => Utf8Char::pack(1, 1, 0x20),
                _ => Utf8Char::pack(1, 1, 0x2020),
            };
            (uc, Utf8State::Error)
        }
    }
}

/// `utf8_to_data`: expand a compact character; an unknown index gives
/// `size` spaces.
pub fn to_data(uc: Utf8Char) -> Utf8Data {
    let mut ud = Utf8Data {
        size: uc.size(),
        have: uc.size(),
        width: uc.width(),
        ..Utf8Data::default()
    };
    if ud.size <= 3 {
        ud.data[2] = (uc.0 >> 16) as u8;
        ud.data[1] = (uc.0 >> 8) as u8;
        ud.data[0] = uc.0 as u8;
    } else {
        let size = usize::from(ud.size);
        table::with_table(|table| match table.get(uc.payload()) {
            Some(bytes) => ud.data[..size].copy_from_slice(&bytes[..size]),
            None => ud.data[..size].fill(b' '),
        });
    }
    crate::log_debug!(
        "utf8_to_data: {:08x} -> ({} {} {})",
        uc.0,
        ud.width,
        ud.size,
        Raw(ud.bytes())
    );
    ud
}

/// Decode one sequence starting at `s[i]` the way the string helpers do:
/// `Some((ud, next))` on `UTF8_DONE`, `None` when the lead byte is bad or the
/// sequence is incomplete or invalid (the caller then handles `s[i]` alone).
fn decode_at(s: &[u8], i: usize, no_width: bool) -> Option<(Utf8Data, usize)> {
    let mut ud = Utf8Data::open(s[i]).ok()?;
    let mut more = Utf8State::More;
    let mut j = i + 1;
    while j < s.len() && more == Utf8State::More {
        more = if no_width {
            ud.append_no_width(s[j])
        } else {
            ud.append(s[j])
        };
        j += 1;
    }
    (more == Utf8State::Done).then_some((ud, j))
}

/// libc `isalpha((u_char)c)` under the UTF-8 locale tmux selects: the macOS
/// rune table classifies bytes 0x80-0xff as the code points U+0080-U+00FF;
/// glibc classifies them as non-alpha.
fn is_alpha(c: u8) -> bool {
    if cfg!(target_os = "macos") {
        char::from(c).is_alphabetic()
    } else {
        c.is_ascii_alphabetic()
    }
}

/// `utf8_strvis` / `utf8_stravis` / `utf8_stravisx`: encode `src` into `dst`,
/// copying valid UTF-8 unchanged. Processes the whole slice, including NUL.
pub fn strvis(dst: &mut Vec<u8>, src: &[u8], flags: VisFlags) {
    dst.reserve(4 * (src.len() + 1));
    let mut i = 0;
    while i < src.len() {
        if let Some((ud, next)) = decode_at(src, i, false) {
            dst.extend_from_slice(ud.bytes());
            i = next;
            continue;
        }
        let last = i + 1 == src.len();
        if flags.contains(VisFlags::DQ) && src[i] == b'$' && !last {
            let next = src[i + 1];
            if is_alpha(next) || next == b'_' || next == b'{' {
                dst.push(b'\\');
            }
            dst.push(b'$');
        } else if !last {
            vis(dst, src[i], flags, src[i + 1]);
        } else {
            vis(dst, src[i], flags, 0);
        }
        i += 1;
    }
}

/// `utf8_isvalid`: only valid sequences and bytes `0x20-0x7e`, up to NUL.
pub fn is_valid(s: &[u8]) -> bool {
    let s = cstr(s);
    let mut i = 0;
    while i < s.len() {
        if let Some((_, next)) = decode_at(s, i, false) {
            i = next;
            continue;
        }
        if !(0x20..=0x7e).contains(&s[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// `utf8_sanitize`: `width` underscores per valid sequence, printable ASCII
/// kept, `_` for anything else.
pub fn sanitize(src: &[u8]) -> ByteString {
    let src = cstr(src);
    let mut dst = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if let Some((ud, next)) = decode_at(src, i, false) {
            dst.extend(std::iter::repeat_n(b'_', usize::from(ud.width)));
            i = next;
            continue;
        }
        if (0x20..0x7f).contains(&src[i]) {
            dst.push(src[i]);
        } else {
            dst.push(b'_');
        }
        i += 1;
    }
    ByteString(dst)
}

/// Owned characters; replaces the `size == 0` terminated array.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Utf8String(pub Vec<Utf8Data>);

fn from_cstr_mode(src: &[u8], no_width: bool) -> Utf8String {
    let src = cstr(src);
    let mut dst = Vec::new();
    let mut i = 0;
    while i < src.len() {
        if let Some((ud, next)) = decode_at(src, i, no_width) {
            dst.push(ud);
            i = next;
            continue;
        }
        dst.push(Utf8Data::set(src[i]));
        i += 1;
    }
    Utf8String(dst)
}

/// `utf8_fromcstr`: each valid sequence is one character; any other byte is
/// one character of size 1 and width 1.
pub fn from_cstr(src: &[u8]) -> Utf8String {
    from_cstr_mode(src, false)
}

/// `utf8_fromcstr` with `utf8_no_width` set (`utf8.c:385-387`).
pub(crate) fn from_cstr_no_width(src: &[u8]) -> Utf8String {
    from_cstr_mode(src, true)
}

impl Utf8String {
    /// `utf8_tocstr`.
    pub fn to_bytes(&self) -> ByteString {
        let mut dst = Vec::new();
        for ud in &self.0 {
            dst.extend_from_slice(ud.bytes());
        }
        ByteString(dst)
    }

    /// `utf8_strlen`.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `utf8_strwidth`: sum of the first `n` widths (all when `None`) as
    /// `u_int`.
    pub fn width(&self, n: Option<usize>) -> u32 {
        self.0
            .iter()
            .take(n.unwrap_or(usize::MAX))
            .fold(0u32, |acc, ud| acc.wrapping_add(u32::from(ud.width)))
    }

    /// `utf8_cstrhas`: a character with the same bytes as `ud` is present.
    pub fn contains(&self, ud: &Utf8Data) -> bool {
        self.0.iter().any(|c| c.bytes() == ud.bytes())
    }
}

impl std::ops::Deref for Utf8String {
    type Target = [Utf8Data];

    fn deref(&self) -> &[Utf8Data] {
        &self.0
    }
}

/// `utf8_cstrwidth`: widths of valid sequences plus one per other byte that
/// passes `*s > 0x1f && *s != 0x7f` as a C `char`, up to NUL.
pub fn cstr_width(s: &[u8]) -> u32 {
    let s = cstr(s);
    let mut width = 0u32;
    let mut i = 0;
    while i < s.len() {
        if let Some((ud, next)) = decode_at(s, i, false) {
            width = width.wrapping_add(u32::from(ud.width));
            i = next;
            continue;
        }
        // The comparison is on the target C `char`: signed on x86_64 and macOS
        // arm64 (bytes >= 0x80 never count), unsigned on Linux aarch64.
        let c = s[i] as std::ffi::c_char;
        if c > 0x1f && c != 0x7f {
            width = width.wrapping_add(1);
        }
        i += 1;
    }
    width
}

/// `utf8_padcstr`: append spaces until the width reaches `width`.
pub fn pad_right(s: &[u8], width: u32) -> ByteString {
    let s = cstr(s);
    let n = cstr_width(s);
    let mut out = s.to_vec();
    if n < width {
        out.extend(std::iter::repeat_n(b' ', (width - n) as usize));
    }
    ByteString(out)
}

/// `utf8_rpadcstr`: prepend spaces until the width reaches `width`.
pub fn pad_left(s: &[u8], width: u32) -> ByteString {
    let s = cstr(s);
    let n = cstr_width(s);
    let mut out = Vec::with_capacity(s.len() + width.saturating_sub(n) as usize);
    if n < width {
        out.extend(std::iter::repeat_n(b' ', (width - n) as usize));
    }
    out.extend_from_slice(s);
    ByteString(out)
}
