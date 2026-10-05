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
pub enum EventPayloadType {
    Pointer = 8,
    String = 0,
    Time = 1,
    Int = 2,
    Uint = 3,
    Client = 4,
    Session = 5,
    Window = 6,
    Pane = 7,
}
impl TryFrom<i32> for EventPayloadType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            8 => Ok(Self::Pointer),
            0 => Ok(Self::String),
            1 => Ok(Self::Time),
            2 => Ok(Self::Int),
            3 => Ok(Self::Uint),
            4 => Ok(Self::Client),
            5 => Ok(Self::Session),
            6 => Ok(Self::Window),
            7 => Ok(Self::Pane),
            _ => Err(value),
        }
    }
}
