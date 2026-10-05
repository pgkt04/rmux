// Ported from tmux compat/strtonum.c @ 8f25579c
//! `strtonum`: bounded decimal parse with the three OpenBSD error classes.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StrtonumError {
    Invalid,
    TooSmall,
    TooLarge,
}

impl fmt::Display for StrtonumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StrtonumError::Invalid => "invalid",
            StrtonumError::TooSmall => "too small",
            StrtonumError::TooLarge => "too large",
        })
    }
}

impl std::error::Error for StrtonumError {}

/// libc `isspace` in the C locale, as `strtoll` skips it.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `strtoll(numstr, &ep, 10)` on a C string: value (saturated on overflow),
/// end offset, and whether `ERANGE` would be set. `end == 0` means no digits.
fn strtoll(s: &[u8]) -> (i64, usize, bool) {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    let digits_start = i;
    let mut value: i64 = 0;
    let mut overflow = false;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = i64::from(s[i] - b'0');
        if !overflow {
            let next = if negative {
                value.checked_mul(10).and_then(|v| v.checked_sub(d))
            } else {
                value.checked_mul(10).and_then(|v| v.checked_add(d))
            };
            match next {
                Some(v) => value = v,
                None => overflow = true,
            }
        }
        i += 1;
    }
    if i == digits_start {
        return (0, 0, false);
    }
    if overflow {
        value = if negative { i64::MIN } else { i64::MAX };
    }
    (value, i, overflow)
}

/// `strtonum(3)`: leading whitespace and a sign are accepted, trailing bytes
/// are not; `min > max` is `Invalid`. Stops at the first NUL like a C string.
pub fn strtonum(s: &[u8], min: i64, max: i64) -> Result<i64, StrtonumError> {
    if min > max {
        return Err(StrtonumError::Invalid);
    }
    let s = crate::bytes::cstr(s);
    let (ll, end, erange) = strtoll(s);
    if end == 0 || end != s.len() {
        Err(StrtonumError::Invalid)
    } else if (ll == i64::MIN && erange) || ll < min {
        Err(StrtonumError::TooSmall)
    } else if (ll == i64::MAX && erange) || ll > max {
        Err(StrtonumError::TooLarge)
    } else {
        Ok(ll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: (i64, i64) = (i64::MIN, i64::MAX);

    #[test]
    fn fixtures() {
        assert_eq!(strtonum(b"", FULL.0, FULL.1), Err(StrtonumError::Invalid));
        assert_eq!(strtonum(b"1x", FULL.0, FULL.1), Err(StrtonumError::Invalid));
        assert_eq!(strtonum(b"-1", 0, FULL.1), Err(StrtonumError::TooSmall));
        assert_eq!(strtonum(b"3", FULL.0, 2), Err(StrtonumError::TooLarge));
        assert_eq!(strtonum(b" 1", FULL.0, FULL.1), Ok(1));
        assert_eq!(strtonum(b"\t\n\x0b\x0c\r 1", FULL.0, FULL.1), Ok(1));
        assert_eq!(strtonum(b"+1", FULL.0, FULL.1), Ok(1));
        assert_eq!(
            strtonum(b"-9223372036854775808", FULL.0, FULL.1),
            Ok(i64::MIN)
        );
        assert_eq!(
            strtonum(b"9223372036854775807", FULL.0, FULL.1),
            Ok(i64::MAX)
        );
        assert_eq!(
            strtonum(b"9223372036854775808", FULL.0, FULL.1),
            Err(StrtonumError::TooLarge)
        );
        assert_eq!(
            strtonum(b"-9223372036854775809", FULL.0, FULL.1),
            Err(StrtonumError::TooSmall)
        );
        assert_eq!(strtonum(b"1", 2, 1), Err(StrtonumError::Invalid));
    }

    #[test]
    fn overflow_with_trailing_bytes_is_invalid() {
        assert_eq!(
            strtonum(b"99999999999999999999x", FULL.0, FULL.1),
            Err(StrtonumError::Invalid)
        );
        assert_eq!(strtonum(b"1 ", FULL.0, FULL.1), Err(StrtonumError::Invalid));
        assert_eq!(strtonum(b"-", FULL.0, FULL.1), Err(StrtonumError::Invalid));
        assert_eq!(strtonum(b"+", FULL.0, FULL.1), Err(StrtonumError::Invalid));
        assert_eq!(strtonum(b" ", FULL.0, FULL.1), Err(StrtonumError::Invalid));
        assert_eq!(
            strtonum(b"0x10", FULL.0, FULL.1),
            Err(StrtonumError::Invalid)
        );
    }

    #[test]
    fn nul_terminates() {
        assert_eq!(strtonum(b"12\0x", FULL.0, FULL.1), Ok(12));
        assert_eq!(strtonum(b"\0", FULL.0, FULL.1), Err(StrtonumError::Invalid));
    }

    #[test]
    fn overflow_wins_over_range() {
        // LLONG_MIN with ERANGE is "too small" even when min is LLONG_MIN.
        assert_eq!(
            strtonum(b"-99999999999999999999", i64::MIN, 0),
            Err(StrtonumError::TooSmall)
        );
        assert_eq!(
            strtonum(b"99999999999999999999", 0, i64::MAX),
            Err(StrtonumError::TooLarge)
        );
    }

    #[test]
    fn display() {
        assert_eq!(StrtonumError::Invalid.to_string(), "invalid");
        assert_eq!(StrtonumError::TooSmall.to_string(), "too small");
        assert_eq!(StrtonumError::TooLarge.to_string(), "too large");
    }
}
