// Ported from tmux tty-keys.c and key-string.c @ 8f25579c
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

//! libc numeric scanning as the key parsers use it: `sscanf` `%u`/`%x`
//! conversions that accept a parsed prefix (`tty-keys.c:720,726,1134,1137`;
//! `key-string.c:205,261`), and `strtoul`/`strtol` with their end offset
//! (`tty-keys.c:1511,1642,1879`). Overflow saturates like libc; the results
//! are then narrowed to the C variable width by the caller.

/// libc `isspace` under tmux's UTF-8 locale: the macOS rune table also
/// treats byte 0xa0 (U+00A0) as a space; glibc classifies no high byte.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
        || (cfg!(target_os = "macos") && c == 0xa0)
}

fn digit(c: u8, base: u64) -> Option<u64> {
    let v = match c {
        b'0'..=b'9' => u64::from(c - b'0'),
        b'a'..=b'z' => u64::from(c - b'a') + 10,
        b'A'..=b'Z' => u64::from(c - b'A') + 10,
        _ => return None,
    };
    (v < base).then_some(v)
}

/// `strtoul`-style unsigned conversion. Returns the magnitude saturated at
/// `u64::MAX`, negated (wrapping) for a `-` sign, and the end offset; no
/// digits give `(0, 0)` like `endptr = nptr`. `%x` and base 16 accept an
/// optional `0x` prefix.
fn unsigned(s: &[u8], base: u64) -> (u64, usize) {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    if base == 16
        && i + 1 < s.len()
        && s[i] == b'0'
        && (s[i + 1] == b'x' || s[i + 1] == b'X')
        && s.get(i + 2).is_some_and(|&c| digit(c, 16).is_some())
    {
        i += 2;
    }
    let start = i;
    let mut value: u64 = 0;
    let mut overflow = false;
    while i < s.len() {
        let Some(d) = digit(s[i], base) else {
            break;
        };
        match value.checked_mul(base).and_then(|v| v.checked_add(d)) {
            Some(v) => value = v,
            None => overflow = true,
        }
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    if overflow {
        return (u64::MAX, i);
    }
    (
        if negative {
            value.wrapping_neg()
        } else {
            value
        },
        i,
    )
}

/// `strtoul(s, &end, 10)` as `(value, end)`.
pub fn strtoul(s: &[u8]) -> (u64, usize) {
    unsigned(s, 10)
}

/// `strtol(s, &end, 10)` as `(value, end)` with `long` saturation.
pub fn strtol(s: &[u8]) -> (i64, usize) {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut value: i64 = 0;
    let mut overflow = false;
    while i < s.len() {
        let Some(d) = digit(s[i], 10) else {
            break;
        };
        let step = if negative {
            value.checked_mul(10).and_then(|v| v.checked_sub(d as i64))
        } else {
            value.checked_mul(10).and_then(|v| v.checked_add(d as i64))
        };
        match step {
            Some(v) => value = v,
            None => overflow = true,
        }
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    if overflow {
        return (if negative { i64::MIN } else { i64::MAX }, i);
    }
    (value, i)
}

/// One `sscanf` `%u` conversion: `(value as u_int, end)`, or `None` on a
/// matching failure (no digits).
pub fn scan_u(s: &[u8]) -> Option<(u32, usize)> {
    let (value, end) = unsigned(s, 10);
    (end != 0).then_some((value as u32, end))
}

/// One `sscanf` `%x` conversion: `(value as u_int, end)`, or `None`.
pub fn scan_x(s: &[u8]) -> Option<(u32, usize)> {
    let (value, end) = unsigned(s, 16);
    (end != 0).then_some((value as u32, end))
}

/// `sscanf(s, "<prefix>%u;%u", &a, &b) == 2`: the literal prefix, two
/// unsigned fields and the separator; trailing input is not checked.
pub fn scan_u_pair(s: &[u8], prefix: &[u8]) -> Option<(u32, u32)> {
    let rest = s.strip_prefix(prefix)?;
    let (a, used) = scan_u(rest)?;
    let rest = rest[used..].strip_prefix(b";")?;
    let (b, _) = scan_u(rest)?;
    Some((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_pairs() {
        assert_eq!(scan_u_pair(b"27;5;13", b"27;"), Some((5, 13)));
        assert_eq!(scan_u_pair(b"27;5;13;9", b"27;"), Some((5, 13)));
        assert_eq!(scan_u_pair(b"27;;13", b"27;"), None);
        assert_eq!(scan_u_pair(b"27;5;", b"27;"), None);
        assert_eq!(scan_u_pair(b"", b""), None);
        assert_eq!(scan_u_pair(b"1;2", b""), Some((1, 2)));
        assert_eq!(scan_u(b"99999999999999999999"), Some((u32::MAX, 20)));
        assert_eq!(scan_u(b"-1"), Some((u32::MAX, 2)));
        assert_eq!(scan_u(b" +7x"), Some((7, 3)));
        assert_eq!(scan_u(b"x"), None);
    }

    #[test]
    fn hex_and_strto() {
        assert_eq!(scan_x(b"41"), Some((0x41, 2)));
        assert_eq!(scan_x(b"0x41"), Some((0x41, 4)));
        assert_eq!(scan_x(b"0x"), Some((0, 1)));
        assert_eq!(scan_x(b"g"), None);
        assert_eq!(strtoul(b"61"), (61, 2));
        assert_eq!(strtoul(b"61x"), (61, 2));
        assert_eq!(strtoul(b""), (0, 0));
        assert_eq!(strtoul(b"-3"), (3u64.wrapping_neg(), 2));
        assert_eq!(strtol(b" -12;"), (-12, 4));
        assert_eq!(strtol(b";"), (0, 0));
        assert_eq!(strtol(b"99999999999999999999"), (i64::MAX, 20));
    }
}
