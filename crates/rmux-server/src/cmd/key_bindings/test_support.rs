// Ported from tmux key-bindings.c, cfg.c @ 8f25579c
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

use crate::cmd::parse::{CmdParseInput, ParseContext};
use rmux_util::bytes::ByteString;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Context {
    pub group: u32,
    pub environment: BTreeMap<ByteString, ByteString>,
    pub aliases: BTreeMap<ByteString, ByteString>,
    pub conditions: Vec<CmdParseInput>,
    pub printed: Vec<ByteString>,
}
impl ParseContext for Context {
    fn environment(&self, name: &[u8]) -> Option<&[u8]> {
        self.environment.get(name).map(|s| s.as_ref())
    }
    fn put_environment(&mut self, assignment: &[u8], _hidden: bool) {
        let equal = assignment
            .iter()
            .position(|&b| b == b'=')
            .expect("assignment");
        self.environment
            .insert(assignment[..equal].into(), assignment[equal + 1..].into());
    }
    fn alias(&self, name: &[u8]) -> Option<ByteString> {
        self.aliases.get(name).cloned()
    }
    fn condition(&mut self, format: &[u8], input: &CmdParseInput) -> bool {
        self.conditions.push(input.clone());
        format != b"0" && !format.is_empty()
    }
    fn home(&mut self, _user: Option<&[u8]>) -> Option<ByteString> {
        Some("/tmp".into())
    }
    fn next_group(&mut self) -> u32 {
        self.group = self.group.wrapping_add(1);
        self.group
    }
    fn print(&mut self, message: &[u8], _input: &CmdParseInput) {
        self.printed.push(message.into());
    }
}
