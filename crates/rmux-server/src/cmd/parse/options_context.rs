// Ported from tmux cmd-parse.y, cmd.c @ 8f25579c
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
use super::{CmdParseInput, CmdParseResult, CommandParser, ParseContext};
use crate::options::{
    OptionsStore,
    environment::{Environment, EnvironmentFlags},
};
use rmux_util::bytes::ByteString;

pub trait ParseServices {
    fn condition(&mut self, format: &[u8], input: &CmdParseInput) -> bool;
    fn home(&mut self, user: Option<&[u8]>) -> Option<ByteString>;
    fn print(&mut self, message: &[u8], input: &CmdParseInput);
}

pub struct OptionsParseContext<'a> {
    pub environment: &'a mut Environment,
    pub options: &'a OptionsStore,
    pub services: &'a mut dyn ParseServices,
    pub next_group: &'a mut u32,
}
impl ParseContext for OptionsParseContext<'_> {
    fn environment(&self, name: &[u8]) -> Option<&[u8]> {
        self.environment
            .find(name)?
            .value
            .as_ref()
            .map(|value| value.as_ref())
    }
    fn put_environment(&mut self, assignment: &[u8], hidden: bool) {
        self.environment.put(
            assignment,
            if hidden {
                EnvironmentFlags::HIDDEN
            } else {
                EnvironmentFlags::default()
            },
        );
    }
    fn alias(&self, name: &[u8]) -> Option<ByteString> {
        crate::cmd::get_alias(self.options, name)
    }
    fn condition(&mut self, format: &[u8], input: &CmdParseInput) -> bool {
        self.services.condition(format, input)
    }
    fn home(&mut self, user: Option<&[u8]>) -> Option<ByteString> {
        self.services.home(user)
    }
    fn next_group(&mut self) -> u32 {
        let group = *self.next_group;
        *self.next_group = group.wrapping_add(1);
        group
    }
    fn print(&mut self, message: &[u8], input: &CmdParseInput) {
        self.services.print(message, input);
    }
}
impl CommandParser for OptionsParseContext<'_> {
    fn parse_from_string(&mut self, string: &[u8]) -> CmdParseResult {
        super::from_string(self, string, &mut CmdParseInput::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Services {
        conditions: u32,
        prints: Vec<ByteString>,
    }
    impl ParseServices for Services {
        fn condition(&mut self, format: &[u8], _: &CmdParseInput) -> bool {
            self.conditions += 1;
            format == b"1"
        }
        fn home(&mut self, _: Option<&[u8]>) -> Option<ByteString> {
            Some("/home/test".into())
        }
        fn print(&mut self, message: &[u8], _: &CmdParseInput) {
            self.prints.push(message.into());
        }
    }
    struct Parser;
    impl CommandParser for Parser {
        fn parse_from_string(&mut self, _: &[u8]) -> CmdParseResult {
            Ok(std::rc::Rc::new(crate::cmd::CommandList::default()))
        }
    }
    #[test]
    fn actual_options_and_environment_feed_the_handwritten_parser() {
        let mut options = OptionsStore::new();
        let global = options.global;
        options.default(
            global,
            crate::options::search(b"command-alias").unwrap(),
            &mut Parser,
        );
        options
            .get_mut_only(global, b"command-alias")
            .unwrap()
            .array_set(
                &crate::options::OptionsArrayKey::Index(0),
                Some(b"hello=display-message"),
                false,
                &mut Parser,
            )
            .unwrap();
        let mut environment = Environment::new();
        let mut next_group = 1;
        let mut services = Services {
            conditions: 0,
            prints: Vec::new(),
        };
        let mut context = OptionsParseContext {
            environment: &mut environment,
            options: &options,
            services: &mut services,
            next_group: &mut next_group,
        };
        let list = context.parse_from_string(b"A=foo\nhello $A").unwrap();
        assert_eq!(
            list.print(crate::cmd::CommandListPrintFlags::default()),
            b"display-message foo"
        );
        assert_eq!(context.environment(b"A"), Some(b"foo".as_slice()));
        let mut input = CmdParseInput {
            flags: super::super::CmdParseFlags::PARSEONLY,
            ..Default::default()
        };
        super::super::from_buffer(
            &mut context,
            b"A=bar\n%if 1\nhello $A\n%endif\n",
            &mut input,
        )
        .unwrap();
        assert_eq!(context.environment(b"A"), Some(b"foo".as_slice()));
        assert_eq!(services.conditions, 1);
    }
}
