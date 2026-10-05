// Ported from tmux compat/vis.c, compat/unvis.c, compat/vis.h @ 8f25579c
//! Raw `vis(3)` byte encoding and `strunvis` decoding. These process bytes
//! individually; the UTF-8 preserving variant lives in `utf8::strvis`.

use crate::bytes::{ByteString, cstr};

/// `vis(3)` flag bits with the `compat/vis.h` values.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct VisFlags(pub u32);

impl VisFlags {
    pub const NONE: VisFlags = VisFlags(0);
    pub const OCTAL: VisFlags = VisFlags(0x01);
    pub const CSTYLE: VisFlags = VisFlags(0x02);
    pub const SP: VisFlags = VisFlags(0x04);
    pub const TAB: VisFlags = VisFlags(0x08);
    pub const NL: VisFlags = VisFlags(0x10);
    pub const WHITE: VisFlags = VisFlags(0x04 | 0x08 | 0x10);
    pub const SAFE: VisFlags = VisFlags(0x20);
    pub const NOSLASH: VisFlags = VisFlags(0x40);
    pub const GLOB: VisFlags = VisFlags(0x100);
    pub const DQ: VisFlags = VisFlags(0x200);
    pub const ALL: VisFlags = VisFlags(0x400);

    pub const fn contains(self, other: VisFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: VisFlags) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for VisFlags {
    type Output = VisFlags;
    fn bitor(self, rhs: VisFlags) -> VisFlags {
        VisFlags(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for VisFlags {
    fn bitor_assign(&mut self, rhs: VisFlags) {
        self.0 |= rhs.0;
    }
}

const fn is_octal(c: u8) -> bool {
    c >= b'0' && c <= b'7'
}

const fn is_glob_magic(c: u8) -> bool {
    matches!(c, b'*' | b'?' | b'[' | b'#')
}

// isgraph is evaluated on single bytes under the UTF-8 locale tmux sets. The
// macOS table treats 0x80-0xff as Latin-1 (graphic from 0xa1, except the soft
// hyphen 0xad); glibc UTF-8 locales report no properties above 0x7f.
const fn is_graph(c: u8) -> bool {
    (c > b' ' && c < 0x7f) || (cfg!(target_os = "macos") && c >= 0xa1 && c != 0xad)
}

const fn is_cntrl(c: u8) -> bool {
    c < 0x20 || c == 0x7f
}

/// `isvisible(c, flag)` from `compat/vis.c:41-52`.
fn is_visible(c: u8, flag: VisFlags) -> bool {
    if c != b'\\' && flag.contains(VisFlags::ALL) {
        return false;
    }
    (c.is_ascii() && (!is_glob_magic(c) || !flag.contains(VisFlags::GLOB)) && is_graph(c))
        || (!flag.contains(VisFlags::SP) && c == b' ')
        || (!flag.contains(VisFlags::TAB) && c == b'\t')
        || (!flag.contains(VisFlags::NL) && c == b'\n')
        || (flag.contains(VisFlags::SAFE) && (c == 0x08 || c == 0x07 || c == b'\r' || is_graph(c)))
}

/// `vis(3)`: append the visual encoding of `c` to `dst`. `next` is the byte
/// that follows `c` in the source (0 at the end).
pub fn vis(dst: &mut Vec<u8>, c: u8, flags: VisFlags, next: u8) {
    if is_visible(c, flags) {
        if (c == b'"' && flags.contains(VisFlags::DQ))
            || (c == b'\\' && !flags.contains(VisFlags::NOSLASH))
        {
            dst.push(b'\\');
        }
        dst.push(c);
        return;
    }
    if flags.contains(VisFlags::CSTYLE) {
        let short = match c {
            b'\n' => Some(b'n'),
            b'\r' => Some(b'r'),
            0x08 => Some(b'b'),
            0x07 => Some(b'a'),
            0x0b => Some(b'v'),
            b'\t' => Some(b't'),
            0x0c => Some(b'f'),
            b' ' => Some(b's'),
            0 => Some(b'0'),
            _ => None,
        };
        if let Some(s) = short {
            dst.push(b'\\');
            dst.push(s);
            if c == 0 && is_octal(next) {
                dst.extend_from_slice(b"00");
            }
            return;
        }
    }
    if (c & 0o177) == b' '
        || flags.contains(VisFlags::OCTAL)
        || (flags.contains(VisFlags::GLOB) && is_glob_magic(c))
    {
        dst.push(b'\\');
        dst.push(((c >> 6) & 7) + b'0');
        dst.push(((c >> 3) & 7) + b'0');
        dst.push((c & 7) + b'0');
        return;
    }
    if !flags.contains(VisFlags::NOSLASH) {
        dst.push(b'\\');
    }
    let mut c = c;
    if c & 0o200 != 0 {
        c &= 0o177;
        dst.push(b'M');
    }
    if is_cntrl(c) {
        dst.push(b'^');
        dst.push(if c == 0o177 { b'?' } else { c + b'@' });
    } else {
        dst.push(b'-');
        dst.push(c);
    }
}

/// `strvis`/`stravis`: encode `src` up to its first NUL, appending to `dst`.
pub fn strvis(dst: &mut Vec<u8>, src: &[u8], flags: VisFlags) {
    let src = cstr(src);
    dst.reserve(src.len() * 4);
    for (i, &c) in src.iter().enumerate() {
        vis(dst, c, flags, src.get(i + 1).copied().unwrap_or(0));
    }
}

/// `strvisx`: encode exactly `src.len()` bytes, including embedded NUL.
pub fn strvisx(dst: &mut Vec<u8>, src: &[u8], flags: VisFlags) {
    dst.reserve(src.len() * 4);
    for (i, &c) in src.iter().enumerate() {
        vis(dst, c, flags, src.get(i + 1).copied().unwrap_or(0));
    }
}

/// `strnvis`: encode `src` (to its first NUL) into at most `max - 1` bytes,
/// never splitting an escape. `max == 0` gives an empty result. The C
/// terminator is not part of the returned bytes.
pub fn strnvis(src: &[u8], max: usize, flags: VisFlags) -> ByteString {
    let src = cstr(src);
    let mut out = Vec::new();
    let end = max.saturating_sub(1);
    let mut tbuf = Vec::with_capacity(5);
    for (i, &c) in src.iter().enumerate() {
        if out.len() >= end {
            break;
        }
        if is_visible(c, flags) {
            if (c == b'"' && flags.contains(VisFlags::DQ))
                || (c == b'\\' && !flags.contains(VisFlags::NOSLASH))
            {
                if out.len() + 1 >= end {
                    break;
                }
                out.push(b'\\');
            }
            out.push(c);
        } else {
            tbuf.clear();
            vis(&mut tbuf, c, flags, src.get(i + 1).copied().unwrap_or(0));
            if out.len() + tbuf.len() <= end {
                out.extend_from_slice(&tbuf);
            } else {
                break;
            }
        }
    }
    ByteString(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnvisState {
    Ground,
    Start,
    Meta,
    Meta1,
    Ctrl,
    Octal2,
    Octal3,
}

enum Unvis {
    Valid,
    ValidPush,
    NoChar,
    More,
    SynBad,
}

/// `unvis()` one step (`compat/unvis.c:57-203`).
fn unvis(cp: &mut u8, c: u8, state: &mut UnvisState) -> Unvis {
    match *state {
        UnvisState::Ground => {
            *cp = 0;
            if c == b'\\' {
                *state = UnvisState::Start;
                return Unvis::More;
            }
            *cp = c;
            Unvis::Valid
        }
        UnvisState::Start => {
            let (value, next) = match c {
                b'\\' => (c, UnvisState::Ground),
                b'0'..=b'7' => {
                    *cp = c - b'0';
                    *state = UnvisState::Octal2;
                    return Unvis::More;
                }
                b'M' => {
                    *cp = 0o200;
                    *state = UnvisState::Meta;
                    return Unvis::More;
                }
                b'^' => {
                    *state = UnvisState::Ctrl;
                    return Unvis::More;
                }
                b'n' => (b'\n', UnvisState::Ground),
                b'r' => (b'\r', UnvisState::Ground),
                b'b' => (0x08, UnvisState::Ground),
                b'a' => (0x07, UnvisState::Ground),
                b'v' => (0x0b, UnvisState::Ground),
                b't' => (b'\t', UnvisState::Ground),
                b'f' => (0x0c, UnvisState::Ground),
                b's' => (b' ', UnvisState::Ground),
                b'E' => (0x1b, UnvisState::Ground),
                b'\n' | b'$' => {
                    *state = UnvisState::Ground;
                    return Unvis::NoChar;
                }
                _ => {
                    *state = UnvisState::Ground;
                    return Unvis::SynBad;
                }
            };
            *cp = value;
            *state = next;
            Unvis::Valid
        }
        UnvisState::Meta => {
            match c {
                b'-' => *state = UnvisState::Meta1,
                b'^' => *state = UnvisState::Ctrl,
                _ => {
                    *state = UnvisState::Ground;
                    return Unvis::SynBad;
                }
            }
            Unvis::More
        }
        UnvisState::Meta1 => {
            *state = UnvisState::Ground;
            *cp |= c;
            Unvis::Valid
        }
        UnvisState::Ctrl => {
            if c == b'?' {
                *cp |= 0o177;
            } else {
                *cp |= c & 0o37;
            }
            *state = UnvisState::Ground;
            Unvis::Valid
        }
        UnvisState::Octal2 => {
            if is_octal(c) {
                *cp = (*cp << 3).wrapping_add(c - b'0');
                *state = UnvisState::Octal3;
                return Unvis::More;
            }
            *state = UnvisState::Ground;
            Unvis::ValidPush
        }
        UnvisState::Octal3 => {
            *state = UnvisState::Ground;
            if is_octal(c) {
                *cp = (*cp << 3).wrapping_add(c - b'0');
                return Unvis::Valid;
            }
            Unvis::ValidPush
        }
    }
}

/// `strunvis`: decode `src` (to its first NUL). `None` on a bad escape inside
/// the string. An unfinished trailing escape returns the decoded prefix, as
/// the pinned `compat/unvis.c:235-238` does.
pub fn strunvis(src: &[u8]) -> Option<ByteString> {
    let src = cstr(src);
    let mut out = Vec::with_capacity(src.len());
    let mut state = UnvisState::Ground;
    let mut cp = 0u8;
    for &c in src {
        loop {
            match unvis(&mut cp, c, &mut state) {
                Unvis::Valid => {
                    out.push(cp);
                    break;
                }
                Unvis::ValidPush => {
                    out.push(cp);
                    continue;
                }
                Unvis::More | Unvis::NoChar => break,
                Unvis::SynBad => return None,
            }
        }
    }
    if matches!(state, UnvisState::Octal2 | UnvisState::Octal3) {
        out.push(cp);
    }
    Some(ByteString(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(src: &[u8], flags: VisFlags) -> Vec<u8> {
        let mut out = Vec::new();
        strvis(&mut out, src, flags);
        out
    }

    #[test]
    fn cstyle_and_octal_forms() {
        let f = VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL;
        assert_eq!(enc(b"a\tb\n\x1b\xc3\xa9", f), b"a\\tb\\n\\033\\303\\251");
        assert_eq!(enc(b"\\\"", f), b"\\\\\"");
        assert_eq!(enc(b"\"", f | VisFlags::DQ), b"\\\"");
        assert_eq!(enc(b"\x01", VisFlags::NONE), b"\\^A");
        assert_eq!(enc(b"\x7f", VisFlags::NONE), b"\\^?");
        assert_eq!(enc(b"\x81", VisFlags::NONE), b"\\M^A");
        assert_eq!(enc(b"\xe9", VisFlags::NONE), b"\\M-i");
        assert_eq!(enc(b"\xa0", VisFlags::NONE), b"\\240");
        assert_eq!(enc(b"\x01", VisFlags::NOSLASH), b"^A");
        assert_eq!(enc(b"*", VisFlags::GLOB), b"\\052");
        assert_eq!(enc(b"a b", VisFlags::ALL), b"\\-a\\040\\-b");
    }

    #[test]
    fn cstyle_nul_in_strvisx() {
        let mut out = Vec::new();
        strvisx(&mut out, b"\x001\x00x\x00", VisFlags::CSTYLE);
        assert_eq!(out, b"\\0001\\0x\\0");
        assert_eq!(enc(b"a\x00b", VisFlags::CSTYLE), b"a");
    }

    #[test]
    fn strnvis_never_splits_escapes() {
        let f = VisFlags::OCTAL;
        assert_eq!(strnvis(b"a\x01b", 3, f), b"a");
        assert_eq!(strnvis(b"a\x01b", 5, f), b"a");
        assert_eq!(strnvis(b"a\x01b", 6, f), b"a\\001");
        assert_eq!(strnvis(b"a\x01b", 7, f), b"a\\001b");
        assert_eq!(strnvis(b"\\", 2, f), b"");
        assert_eq!(strnvis(b"\\", 3, f), b"\\\\");
        assert_eq!(strnvis(b"abc", 0, f), b"");
        assert_eq!(strnvis(b"abc", 1, f), b"");
    }

    #[test]
    fn strunvis_decodes_and_reports_errors() {
        assert_eq!(
            strunvis(b"a\\tb\\n\\033\\303\\251\\\\").unwrap(),
            b"a\tb\n\x1b\xc3\xa9\\"
        );
        assert_eq!(
            strunvis(b"\\1x\\12y\\123z\\1234").unwrap(),
            b"\x01x\x0ay\x53z\x53\x34"
        );
        assert_eq!(
            strunvis(b"\\M-i\\M^A\\^?\\^A\\E").unwrap(),
            b"\xe9\x81\x7f\x01\x1b"
        );
        assert_eq!(strunvis(b"a\\\nb\\$c").unwrap(), b"abc");
        assert!(strunvis(b"\\q").is_none());
        assert!(strunvis(b"\\Mx").is_none());
        assert_eq!(strunvis(b"ab\\").unwrap(), b"ab");
        assert_eq!(strunvis(b"ab\\M").unwrap(), b"ab");
        assert_eq!(strunvis(b"ab\\^").unwrap(), b"ab");
        assert_eq!(strunvis(b"ab\\7").unwrap(), b"ab\x07");
        assert_eq!(strunvis(b"ab\\77").unwrap(), b"ab\x3f");
    }
}
