// Ported from tmux compat.h @ 8f25579c (strcasestr, strsep, strlcpy, strlcat, strnlen, strndup)
//! `ByteString`: owned arbitrary bytes for everything that comes from a
//! terminal, a file, or an option.

use std::borrow::Borrow;
use std::fmt;
use std::ops::{Deref, DerefMut};

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteString(pub Vec<u8>);

impl ByteString {
    pub const fn new() -> ByteString {
        ByteString(Vec::new())
    }

    pub fn with_capacity(n: usize) -> ByteString {
        ByteString(Vec::with_capacity(n))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }

    /// Bytes up to the first NUL, like a C string view of the buffer.
    pub fn cstr(&self) -> &[u8] {
        cstr(&self.0)
    }
}

impl Deref for ByteString {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl DerefMut for ByteString {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }
}

impl Borrow<[u8]> for ByteString {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for ByteString {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for ByteString {
    fn from(v: Vec<u8>) -> ByteString {
        ByteString(v)
    }
}

impl From<&[u8]> for ByteString {
    fn from(v: &[u8]) -> ByteString {
        ByteString(v.to_vec())
    }
}

impl From<&str> for ByteString {
    fn from(v: &str) -> ByteString {
        ByteString(v.as_bytes().to_vec())
    }
}

impl From<String> for ByteString {
    fn from(v: String) -> ByteString {
        ByteString(v.into_bytes())
    }
}

impl From<ByteString> for Vec<u8> {
    fn from(v: ByteString) -> Vec<u8> {
        v.0
    }
}

impl PartialEq<[u8]> for ByteString {
    fn eq(&self, other: &[u8]) -> bool {
        self.0 == other
    }
}

impl PartialEq<&[u8]> for ByteString {
    fn eq(&self, other: &&[u8]) -> bool {
        self.0 == *other
    }
}

impl PartialEq<str> for ByteString {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<&str> for ByteString {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl<const N: usize> PartialEq<[u8; N]> for ByteString {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.0 == other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for ByteString {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.0 == *other
    }
}

impl fmt::Debug for ByteString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b\"")?;
        for &b in &self.0 {
            for e in std::ascii::escape_default(b) {
                write!(f, "{}", e as char)?;
            }
        }
        write!(f, "\"")
    }
}

/// Display renders bytes lossily as UTF-8 for diagnostics only.
impl fmt::Display for ByteString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

/// `strnlen` view: the bytes before the first NUL.
pub fn cstr(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// `strcasestr`: byte offset of `needle` in `hay`, ASCII case-insensitive.
pub fn find_case_insensitive(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// `strsep`: split at the first byte in `delims`; returns the head and the
/// remaining tail (None when no delimiter was found).
pub fn strsep<'a>(s: &'a [u8], delims: &[u8]) -> (&'a [u8], Option<&'a [u8]>) {
    match s.iter().position(|b| delims.contains(b)) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cstr_stops_at_nul() {
        assert_eq!(cstr(b"ab\0cd"), b"ab");
        assert_eq!(cstr(b"abcd"), b"abcd");
        assert_eq!(cstr(b""), b"");
    }

    #[test]
    fn case_insensitive_find() {
        assert_eq!(find_case_insensitive(b"Hello World", b"WORLD"), Some(6));
        assert_eq!(find_case_insensitive(b"abc", b""), Some(0));
        assert_eq!(find_case_insensitive(b"abc", b"abcd"), None);
    }

    #[test]
    fn strsep_splits_once() {
        assert_eq!(strsep(b"a,b,c", b","), (&b"a"[..], Some(&b"b,c"[..])));
        assert_eq!(strsep(b"abc", b","), (&b"abc"[..], None));
        assert_eq!(strsep(b",", b","), (&b""[..], Some(&b""[..])));
    }

    #[test]
    fn ordering_is_unsigned_strcmp() {
        let (a, e, ten, two) = (
            ByteString::from("a"),
            ByteString::from("\u{e9}"),
            ByteString::from("10"),
            ByteString::from("2"),
        );
        assert!(a < e);
        assert!(ten < two);
        assert_eq!(
            format!("{:?}", ByteString::from(&b"a\n\xff"[..])),
            "b\"a\\n\\xff\""
        );
    }
}
