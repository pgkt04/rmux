// Ported from tmux regsub.c @ 8f25579c
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
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

use rmux_sys::regex::{ExecFlags, PosixRegex, RegexError, RegexFlags, RegexMatch};
use rmux_util::bytes::{ByteString, cstr};

/// `regsub_expand`: append `with`, replacing `\N` with a populated nonempty
/// capture of `m` (offsets relative to `text`); other escaped bytes lose the
/// backslash and a trailing lone backslash is copied (`regsub.c:37-59`).
fn expand(buf: &mut Vec<u8>, with: &[u8], text: &[u8], m: &RegexMatch) {
    let mut i = 0;
    while i < with.len() {
        let mut c = with[i];
        if c == b'\\' && i + 1 < with.len() {
            i += 1;
            c = with[i];
            if c.is_ascii_digit() {
                let slot = usize::from(c - b'0');
                if let Some(Some(range)) = m.ranges.get(slot)
                    && range.start != range.end
                {
                    buf.extend_from_slice(&text[range.clone()]);
                    i += 1;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
}

/// `regsub`: replace every match of `pattern` in `text` with `with`. Inputs
/// end at their first NUL. `Err` is the `regcomp` failure (`regsub.c:61-126`).
pub fn substitute(
    pattern: &[u8],
    with: &[u8],
    text: &[u8],
    flags: RegexFlags,
) -> Result<ByteString, RegexError> {
    let pattern = cstr(pattern);
    let with = cstr(with);
    let text = cstr(text);

    if text.is_empty() {
        return Ok(ByteString::default());
    }
    if pattern.is_empty() {
        return Ok(ByteString::from(text));
    }
    let r = PosixRegex::new(pattern, flags)?;
    let mut m = RegexMatch::with_ten_slots();

    let mut buf = Vec::with_capacity(text.len());
    let end = text.len();
    let mut start = 0usize;
    let mut last = 0usize;
    let mut empty = false;

    while start <= end {
        let subject = &text[start..];
        // tmux treats every nonzero regexec result as the unmatched tail.
        if !matches!(r.exec(subject, &mut m, ExecFlags::NONE), Ok(true)) {
            buf.extend_from_slice(&text[start..end]);
            break;
        }
        let whole = m
            .whole()
            .expect("regexec reported a match without a whole-match range");

        // Append any text not part of this match (from the end of the last
        // match).
        buf.extend_from_slice(&text[last..start + whole.start]);

        // For anchored patterns, replace the first match only.
        if pattern[0] == b'^' {
            expand(&mut buf, with, subject, &m);
            last = start + whole.end;
            buf.extend_from_slice(&text[last..end]);
            break;
        }

        // If the last match was empty and this one isn't (it is either later
        // or has matched text), expand this match. If it is empty, move on one
        // character and try again from there.
        if empty || start + whole.start != last || whole.start != whole.end {
            expand(&mut buf, with, subject, &m);
            last = start + whole.end;
            start += whole.end;
            empty = false;
        } else {
            last = start + whole.end;
            start += whole.end + 1;
            empty = true;
        }
    }
    Ok(ByteString::from(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(pattern: &str, with: &str, text: &str) -> Option<String> {
        substitute(
            pattern.as_bytes(),
            with.as_bytes(),
            text.as_bytes(),
            RegexFlags::EXTENDED,
        )
        .ok()
        .map(|b| String::from_utf8(b.into_vec()).unwrap())
    }

    #[test]
    fn empty_text_and_pattern_short_circuit() {
        assert_eq!(sub("(", "x", ""), Some(String::new()));
        assert_eq!(sub("", "x", "abc"), Some("abc".into()));
        assert_eq!(
            substitute(b"a\0(", b"", b"abc", RegexFlags::EXTENDED).unwrap(),
            b"bc"
        );
    }

    #[test]
    fn invalid_regex_is_an_error() {
        assert!(substitute(b"(", b"x", b"abc", RegexFlags::EXTENDED).is_err());
    }

    #[test]
    fn captures_and_escapes() {
        assert_eq!(sub("(a)(b)?", "[\\1\\2]", "ac"), Some("[a2]c".into()));
        assert_eq!(sub("(a)(b)", "\\2\\1\\0", "ab"), Some("baab".into()));
        assert_eq!(sub("a", "\\x\\", "a"), Some("x\\".into()));
        assert_eq!(sub("a", "\\\\", "a"), Some("\\".into()));
        assert_eq!(sub("(a*)b", "[\\1/\\9]", "b"), Some("[1/9]".into()));
        assert_eq!(sub("a", "&\\0", "a"), Some("&a".into()));
    }

    #[test]
    fn anchored_replaces_first_only() {
        assert_eq!(sub("^a", "X", "aaa"), Some("Xaa".into()));
        assert_eq!(sub("a", "X", "aaa"), Some("XXX".into()));
        assert_eq!(sub("^", "X", "ab"), Some("Xab".into()));
        assert_eq!(sub("a|^b", "X", "abbb"), Some("XXXX".into()));
    }

    #[test]
    fn empty_matches_follow_the_source_state_machine() {
        assert_eq!(sub("x*", "-", "ab"), Some("a-b-".into()));
        assert_eq!(sub("b*", "-", "abc"), Some("a-c-".into()));
        assert_eq!(sub("$", "!", "ab"), Some("ab!".into()));
        assert_eq!(sub("", "-", "ab"), Some("ab".into()));
    }

    #[test]
    fn icase_flag() {
        assert_eq!(
            substitute(b"A", b"x", b"aA", RegexFlags::EXTENDED | RegexFlags::ICASE).unwrap(),
            b"xx"
        );
    }

    #[test]
    fn basic_regex_and_newline_flags_are_preserved() {
        assert_eq!(
            substitute(b"\\(a\\)\\(b\\)", b"\\2\\1", b"abab", RegexFlags::NONE).unwrap(),
            b"baba"
        );
        assert_eq!(
            substitute(b"a.b", b"X", b"a\nb", RegexFlags::EXTENDED).unwrap(),
            b"X"
        );
        assert_eq!(
            substitute(
                b"a.b",
                b"X",
                b"a\nb",
                RegexFlags::EXTENDED | RegexFlags::NEWLINE
            )
            .unwrap(),
            b"a\nb"
        );
    }

    #[test]
    fn replacement_and_subject_end_at_nul() {
        assert_eq!(
            substitute(b"a", b"X\0Y", b"aba\0tail", RegexFlags::EXTENDED).unwrap(),
            b"XbX"
        );
    }
}
