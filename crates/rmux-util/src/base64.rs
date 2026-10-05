// Ported from tmux compat/base64.c @ 8f25579c
//! `b64_ntop` and `b64_pton` (RFC 1521 alphabet, `=` padding).

use crate::bytes::ByteString;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const PAD: u8 = b'=';

/// libc `isspace` in the C locale.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn value_of(ch: u8) -> Option<u8> {
    match ch {
        b'A'..=b'Z' => Some(ch - b'A'),
        b'a'..=b'z' => Some(ch - b'a' + 26),
        b'0'..=b'9' => Some(ch - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// `b64_ntop`: encode with padding to a multiple of four characters.
pub fn ntop(src: &[u8]) -> ByteString {
    let mut out = Vec::with_capacity(src.len().div_ceil(3) * 4);
    let mut chunks = src.chunks_exact(3);
    for input in &mut chunks {
        out.push(ALPHABET[usize::from(input[0] >> 2)]);
        out.push(ALPHABET[usize::from(((input[0] & 0x03) << 4) | (input[1] >> 4))]);
        out.push(ALPHABET[usize::from(((input[1] & 0x0f) << 2) | (input[2] >> 6))]);
        out.push(ALPHABET[usize::from(input[2] & 0x3f)]);
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut input = [0u8; 3];
        input[..rest.len()].copy_from_slice(rest);
        out.push(ALPHABET[usize::from(input[0] >> 2)]);
        out.push(ALPHABET[usize::from(((input[0] & 0x03) << 4) | (input[1] >> 4))]);
        if rest.len() == 1 {
            out.push(PAD);
        } else {
            out.push(ALPHABET[usize::from(((input[1] & 0x0f) << 2) | (input[2] >> 6))]);
        }
        out.push(PAD);
    }
    ByteString(out)
}

/// `b64_pton`: skips whitespace anywhere, stops at NUL or the first `=`,
/// then validates the padding and the unused low bits. tmux sizes the target
/// at `ceil(len / 4) * 3`, which never limits a decode, so no size limit is
/// modelled here.
pub fn pton(src: &[u8]) -> Option<Vec<u8>> {
    let src = crate::bytes::cstr(src);
    let mut out: Vec<u8> = Vec::with_capacity(src.len() / 4 * 3 + 3);
    let mut tarindex = 0usize;
    let mut state = 0u8;
    let mut pos = 0usize;
    let mut saw_pad = false;

    while pos < src.len() {
        let ch = src[pos];
        pos += 1;
        if is_space(ch) {
            continue;
        }
        if ch == PAD {
            saw_pad = true;
            break;
        }
        let v = value_of(ch)?;
        match state {
            0 => {
                if out.len() <= tarindex {
                    out.push(0);
                }
                out[tarindex] = v << 2;
                state = 1;
            }
            1 => {
                out[tarindex] |= v >> 4;
                out.push((v & 0x0f) << 4);
                tarindex += 1;
                state = 2;
            }
            2 => {
                out[tarindex] |= v >> 2;
                out.push((v & 0x03) << 6);
                tarindex += 1;
                state = 3;
            }
            _ => {
                out[tarindex] |= v;
                tarindex += 1;
                state = 0;
            }
        }
    }

    if saw_pad {
        let mut next = || {
            let ch = src.get(pos).copied().unwrap_or(0);
            pos += 1;
            ch
        };
        let mut ch = next();
        match state {
            0 | 1 => return None,
            2 | 3 => {
                if state == 2 {
                    while ch != 0 && is_space(ch) {
                        ch = next();
                    }
                    if ch != PAD {
                        return None;
                    }
                    ch = next();
                }
                while ch != 0 {
                    if !is_space(ch) {
                        return None;
                    }
                    ch = next();
                }
                if out[tarindex] != 0 {
                    return None;
                }
            }
            _ => unreachable!("state is 0..=3"),
        }
    } else if state != 0 {
        return None;
    }

    out.truncate(tarindex);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC4648: &[(&[u8], &[u8])] = &[
        (b"", b""),
        (b"f", b"Zg=="),
        (b"fo", b"Zm8="),
        (b"foo", b"Zm9v"),
        (b"foob", b"Zm9vYg=="),
        (b"fooba", b"Zm9vYmE="),
        (b"foobar", b"Zm9vYmFy"),
    ];

    #[test]
    fn rfc4648_round_trip() {
        for (plain, encoded) in RFC4648 {
            assert_eq!(ntop(plain).as_bytes(), *encoded);
            assert_eq!(pton(encoded).as_deref(), Some(*plain));
        }
    }

    #[test]
    fn binary_round_trip() {
        let all: Vec<u8> = (0..=255u8).collect();
        for n in 0..all.len() {
            let enc = ntop(&all[..n]);
            assert_eq!(pton(&enc).unwrap(), &all[..n]);
        }
    }

    #[test]
    fn whitespace_anywhere() {
        assert_eq!(pton(b" Z\tm\n9\x0bv\x0cY\rmFy ").unwrap(), b"foobar");
        assert_eq!(pton(b"Zm9v\nYmE=\n").unwrap(), b"fooba");
        assert_eq!(pton(b"Z g = = ").unwrap(), b"f");
    }

    #[test]
    fn whitespace_between_padding_bytes() {
        assert_eq!(pton(b"Zg= \t\n=").unwrap(), b"f");
        assert_eq!(pton(b"Zg=  "), None);
        assert_eq!(pton(b"Zg=x="), None);
    }

    #[test]
    fn invalid_characters() {
        assert_eq!(pton(b"Zm9v*"), None);
        assert_eq!(pton(b"Zm-v"), None);
        assert_eq!(pton(b"Zm9v\xc3"), None);
        assert_eq!(pton(b"Zm9v_"), None);
    }

    #[test]
    fn missing_padding() {
        assert_eq!(pton(b"Zg"), None);
        assert_eq!(pton(b"Zg="), None);
        assert_eq!(pton(b"Zm8"), None);
        assert_eq!(pton(b"Z"), None);
        assert_eq!(pton(b"Zm9vY"), None);
    }

    #[test]
    fn excess_padding_and_misplaced_padding() {
        assert_eq!(pton(b"Zm8=="), None);
        assert_eq!(pton(b"Zg==="), None);
        assert_eq!(pton(b"Zm9v="), None);
        assert_eq!(pton(b"="), None);
        assert_eq!(pton(b"Z==="), None);
        assert_eq!(pton(b"Zg==Zg=="), None);
        assert_eq!(pton(b"Zm8=Zg"), None);
    }

    #[test]
    fn nonzero_unused_pad_bits() {
        // 'h' is 33: low four bits set after the single byte of "Zh==".
        assert_eq!(pton(b"Zh=="), None);
        assert_eq!(pton(b"Zm9="), None);
        assert_eq!(pton(b"Zm8=").unwrap(), b"fo");
    }

    #[test]
    fn nul_stops() {
        assert_eq!(pton(b"Zm9v\0*****").unwrap(), b"foo");
        assert_eq!(pton(b"Zg==\0garbage").unwrap(), b"f");
        assert_eq!(pton(b"Zg=\0="), None);
        assert_eq!(pton(b"\0Zm9v").unwrap(), b"");
        assert_eq!(pton(b"").unwrap(), b"");
    }
}
