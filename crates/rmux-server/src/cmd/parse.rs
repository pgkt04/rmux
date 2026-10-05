// Ported from tmux cmd-parse.y, tmux.h @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CmdParseStatus {
    Success = 1,
    Error = 0,
}
impl TryFrom<i32> for CmdParseStatus {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            1 => Ok(Self::Success),
            0 => Ok(Self::Error),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CmdParseFlags(pub u32);
impl CmdParseFlags {
    pub const QUIET: Self = Self(1);
    pub const PARSEONLY: Self = Self(2);
    pub const NOALIAS: Self = Self(4);
    pub const VERBOSE: Self = Self(8);
    pub const ONEGROUP: Self = Self(16);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for CmdParseFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for CmdParseFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for CmdParseFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

pub mod build;
pub mod grammar;
pub mod lexer;
pub mod options_context;
pub use options_context::{OptionsParseContext, ParseServices};

use std::io::Read;
use std::rc::Rc;

use rmux_util::bytes::ByteString;

use super::CommandList;
use super::arguments::ArgsValue;
use super::find::CmdFindState;
use crate::ids::{ClientId, QueueItemId, QueueStateId};

#[derive(Clone, Debug, Default)]
pub struct CmdParseInput {
    pub flags: CmdParseFlags,
    pub file: Option<ByteString>,
    pub line: u32,
    pub item: Option<QueueItemId>,
    pub client: Option<ClientId>,
    pub target: CmdFindState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CmdParseError {
    message: ByteString,
}

impl CmdParseError {
    pub fn new(message: ByteString) -> Self {
        Self { message }
    }

    pub fn message(&self) -> &[u8] {
        self.message.as_ref()
    }

    pub(crate) fn at(input: &CmdParseInput, message: &[u8]) -> Self {
        let mut text = Vec::new();
        if let Some(file) = &input.file {
            text.extend_from_slice(file.as_ref());
            text.extend_from_slice(format!(":{}: ", input.line).as_bytes());
        }
        text.extend_from_slice(message);
        Self::new(text.into())
    }
}

impl std::fmt::Display for CmdParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(self.message()))
    }
}

impl std::error::Error for CmdParseError {}

pub type CmdParseResult = Result<Rc<CommandList>, CmdParseError>;

pub trait CommandParser {
    fn parse_from_string(&mut self, s: &[u8]) -> CmdParseResult;
}

pub trait ParseContext {
    fn environment(&self, name: &[u8]) -> Option<&[u8]>;
    fn put_environment(&mut self, assignment: &[u8], hidden: bool);
    fn alias(&self, name: &[u8]) -> Option<ByteString>;
    fn condition(&mut self, format: &[u8], input: &CmdParseInput) -> bool;
    fn home(&mut self, user: Option<&[u8]>) -> Option<ByteString>;
    fn next_group(&mut self) -> u32;
    fn print(&mut self, message: &[u8], input: &CmdParseInput);
}

pub trait ParseQueueContext: ParseContext {
    fn insert_commands(&mut self, list: Rc<CommandList>, after: QueueItemId, state: QueueStateId);
    fn append_commands(
        &mut self,
        list: Rc<CommandList>,
        client: Option<ClientId>,
        state: QueueStateId,
    );
}

pub fn from_buffer(
    context: &mut impl ParseContext,
    buffer: &[u8],
    input: &mut CmdParseInput,
) -> CmdParseResult {
    if buffer.is_empty() {
        return build::commands(context, Vec::new(), input);
    }
    let commands = grammar::parse(lexer::Lexer::new(buffer), context, input)?;
    build::commands(context, commands, input)
}

pub fn from_file(
    context: &mut impl ParseContext,
    reader: &mut dyn Read,
    input: &mut CmdParseInput,
) -> CmdParseResult {
    let commands = grammar::parse(lexer::Lexer::from_reader(reader), context, input)?;
    build::commands(context, commands, input)
}

pub fn from_string(
    context: &mut impl ParseContext,
    string: &[u8],
    input: &mut CmdParseInput,
) -> CmdParseResult {
    input.flags.insert(CmdParseFlags::ONEGROUP);
    let length = string
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(string.len());
    from_buffer(context, &string[..length], input)
}

pub fn from_arguments(
    context: &mut impl ParseContext,
    values: &[ArgsValue],
    input: &mut CmdParseInput,
) -> CmdParseResult {
    build::from_arguments(context, values, input)
}

pub fn and_insert(
    context: &mut impl ParseQueueContext,
    string: &[u8],
    input: &mut CmdParseInput,
    after: QueueItemId,
    state: QueueStateId,
) -> Result<(), CmdParseError> {
    let list = from_string(context, string, input)?;
    context.insert_commands(list, after, state);
    Ok(())
}

pub fn and_append(
    context: &mut impl ParseQueueContext,
    string: &[u8],
    input: &mut CmdParseInput,
    client: Option<ClientId>,
    state: QueueStateId,
) -> Result<(), CmdParseError> {
    let list = from_string(context, string, input)?;
    context.append_commands(list, client, state);
    Ok(())
}

#[cfg(test)]
mod tests;
