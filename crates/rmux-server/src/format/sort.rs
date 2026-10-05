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
pub enum SortOrder {
    Activity = 0,
    Creation = 1,
    Index = 2,
    Modifier = 3,
    Name = 4,
    Order = 5,
    Size = 6,
    Z = 7,
    End = 8,
}
impl TryFrom<i32> for SortOrder {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Activity),
            1 => Ok(Self::Creation),
            2 => Ok(Self::Index),
            3 => Ok(Self::Modifier),
            4 => Ok(Self::Name),
            5 => Ok(Self::Order),
            6 => Ok(Self::Size),
            7 => Ok(Self::Z),
            8 => Ok(Self::End),
            _ => Err(value),
        }
    }
}
