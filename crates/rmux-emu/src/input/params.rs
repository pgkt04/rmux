// Ported from tmux input.c @ 8f25579c
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

//! CSI parameter splitting and lookup (`input.c:1091-1160`).

use rmux_util::strtonum::strtonum;

/// `struct input_param` (`input.c:86-96`); `Colon` is a byte range of the
/// parameter buffer in place of `xstrdup`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InputParam {
    #[default]
    Missing,
    Number(i32),
    Colon {
        start: u8,
        len: u8,
    },
}

pub(crate) const PARAM_LIST_LEN: usize = 24;

/// `param_list` and `param_list_len`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    list: [InputParam; PARAM_LIST_LEN],
    len: u8,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            list: [InputParam::Missing; PARAM_LIST_LEN],
            len: 0,
        }
    }
}

impl Params {
    pub(crate) fn len(&self) -> usize {
        usize::from(self.len)
    }

    pub(crate) fn raw(&self, idx: usize) -> Option<InputParam> {
        (idx < self.len()).then(|| self.list[idx])
    }

    /// `input_split`: `Err(())` when a number is invalid or a 24th parameter
    /// appears; the caller ignores the whole sequence.
    pub(crate) fn split(&mut self, buf: &[u8]) -> Result<(), ()> {
        self.len = 0;
        if buf.is_empty() {
            return Ok(());
        }
        let mut start = 0usize;
        loop {
            let end = buf[start..]
                .iter()
                .position(|&b| b == b';')
                .map_or(buf.len(), |p| start + p);
            let field = &buf[start..end];
            let ip = if field.is_empty() {
                InputParam::Missing
            } else if field.contains(&b':') {
                InputParam::Colon {
                    start: start as u8,
                    len: field.len() as u8,
                }
            } else {
                match strtonum(field, 0, i64::from(i32::MAX)) {
                    Ok(n) => InputParam::Number(n as i32),
                    Err(_) => return Err(()),
                }
            };
            self.list[usize::from(self.len)] = ip;
            self.len += 1;
            if usize::from(self.len) == PARAM_LIST_LEN {
                return Err(());
            }
            if end == buf.len() {
                break;
            }
            start = end + 1;
        }
        Ok(())
    }

    /// `input_get` (`input.c:1143-1160`).
    pub(crate) fn get(&self, idx: usize, min: i32, def: i32) -> i32 {
        match self.raw(idx) {
            None | Some(InputParam::Missing) => def,
            Some(InputParam::Colon { .. }) => -1,
            Some(InputParam::Number(n)) => {
                if n < min {
                    min
                } else {
                    n
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_get() {
        let mut p = Params::default();
        p.split(b"1;;3").unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p.get(0, 0, 9), 1);
        assert_eq!(p.get(1, 0, 9), 9);
        assert_eq!(p.get(2, 5, 9), 5);
        assert_eq!(p.get(3, 0, 9), 9);
        p.split(b"4:3;7").unwrap();
        assert_eq!(p.raw(0), Some(InputParam::Colon { start: 0, len: 3 }));
        assert_eq!(p.get(0, 0, 0), -1);
        assert_eq!(p.get(1, 0, 0), 7);
        assert!(p.split(b"99999999999").is_err());
        assert!(p.split(b"-1").is_err());
        let twenty_three = "1;".repeat(22) + "1";
        p.split(twenty_three.as_bytes()).unwrap();
        assert_eq!(p.len(), 23);
        let twenty_four = "1;".repeat(23) + "1";
        assert!(p.split(twenty_four.as_bytes()).is_err());
        p.split(b"5;").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.raw(1), Some(InputParam::Missing));
        p.split(b"").unwrap();
        assert_eq!(p.len(), 0);
    }
}
