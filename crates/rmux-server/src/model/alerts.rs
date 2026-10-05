// Ported from tmux tmux.h @ 8f25579c
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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlertPolicy {
    None = 0,
    Any = 1,
    Current = 2,
    Other = 3,
}
impl TryFrom<i32> for AlertPolicy {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Any),
            2 => Ok(Self::Current),
            3 => Ok(Self::Other),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VisualPolicy {
    Off = 0,
    On = 1,
    Both = 2,
}
impl TryFrom<i32> for VisualPolicy {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::On),
            2 => Ok(Self::Both),
            _ => Err(value),
        }
    }
}
