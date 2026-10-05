// Ported from tmux cmd-parse.y @ 8f25579c
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

use std::rc::Rc;

use rmux_util::bytes::ByteString;

use super::lexer::{Lexer, Token};
use super::{CmdParseError, CmdParseFlags, CmdParseInput, ParseContext};
use crate::cmd::CommandList;

#[derive(Clone, Debug)]
pub enum ParsedArgument {
    String(ByteString),
    Commands(Vec<ParsedCommand>),
    ParsedCommands(Rc<CommandList>),
}

#[derive(Clone, Debug)]
pub struct ParsedCommand {
    pub line: u32,
    pub arguments: Vec<ParsedArgument>,
}

pub fn parse(
    lexer: Lexer<'_>,
    context: &mut impl ParseContext,
    input: &mut CmdParseInput,
) -> Result<Vec<ParsedCommand>, CmdParseError> {
    let mut parser = Parser {
        lexer,
        context,
        input,
        lookahead: None,
        scopes: Vec::new(),
    };
    parser.statements(false)
}

struct Parser<'a, 'b, C> {
    lexer: Lexer<'a>,
    context: &'b mut C,
    input: &'b mut CmdParseInput,
    lookahead: Option<Token>,
    scopes: Vec<bool>,
}

impl<C: ParseContext> Parser<'_, '_, C> {
    fn peek(&mut self) -> &Token {
        if self.lookahead.is_none() {
            self.lookahead = Some(self.lexer.next(self.context, self.input));
        }
        self.lookahead.as_ref().expect("initialized lookahead")
    }

    fn take(&mut self) -> Token {
        self.peek();
        self.lookahead.take().expect("initialized lookahead")
    }

    fn error(&self) -> CmdParseError {
        self.lexer
            .error
            .clone()
            .unwrap_or_else(|| CmdParseError::at(self.input, b"syntax error"))
    }

    fn expect(&mut self, token: Token) -> Result<(), CmdParseError> {
        if self.take() == token {
            Ok(())
        } else {
            Err(self.error())
        }
    }

    fn active(&self) -> bool {
        self.scopes.last().copied().unwrap_or(true)
    }

    fn assignment(&mut self, hidden: bool) -> Result<(), CmdParseError> {
        let Token::Equals(value) = self.take() else {
            return Err(self.error());
        };
        if value.as_ref().len() > 16384 {
            return Err(CmdParseError::at(
                self.input,
                b"environment variable is too long",
            ));
        }
        if !self.input.flags.contains(CmdParseFlags::PARSEONLY)
            && self.scopes.iter().all(|flag| *flag)
        {
            self.context.put_environment(value.as_ref(), hidden);
        }
        Ok(())
    }

    fn statements(&mut self, braces: bool) -> Result<Vec<ParsedCommand>, CmdParseError> {
        let mut result = Vec::new();
        loop {
            match self.peek() {
                Token::Eof if !braces => return Ok(result),
                Token::RBrace if braces => {
                    self.take();
                    return Ok(result);
                }
                Token::Eof | Token::RBrace => return Err(self.error()),
                _ => {}
            }
            result.extend(self.statement()?);
            if braces && self.peek() == &Token::RBrace {
                self.take();
                return Ok(result);
            }
            self.expect(Token::Newline)?;
        }
    }

    fn statement(&mut self) -> Result<Vec<ParsedCommand>, CmdParseError> {
        let result = match self.peek() {
            Token::Newline | Token::RBrace => Vec::new(),
            Token::Hidden => {
                self.take();
                self.assignment(true)?;
                Vec::new()
            }
            Token::If => {
                let (commands, multiline) = self.condition(true)?;
                if multiline {
                    commands
                } else {
                    self.commands_tail(commands)?
                }
            }
            _ => self.commands()?,
        };
        Ok(if self.active() { result } else { Vec::new() })
    }

    fn commands(&mut self) -> Result<Vec<ParsedCommand>, CmdParseError> {
        let commands = if self.peek() == &Token::If {
            self.condition(false)?.0
        } else {
            let command = self.command()?;
            if self.active() && !command.arguments.is_empty() {
                vec![command]
            } else {
                Vec::new()
            }
        };
        self.commands_tail(commands)
    }

    fn commands_tail(
        &mut self,
        mut commands: Vec<ParsedCommand>,
    ) -> Result<Vec<ParsedCommand>, CmdParseError> {
        while self.peek() == &Token::Semicolon {
            self.take();
            match self.peek() {
                Token::Semicolon
                | Token::Newline
                | Token::RBrace
                | Token::Else
                | Token::Elif
                | Token::Endif
                | Token::Eof => continue,
                Token::If => commands.extend(self.condition(false)?.0),
                _ => {
                    let command = self.command()?;
                    if self.active() && !command.arguments.is_empty() {
                        commands.push(command);
                    } else {
                        commands.clear();
                    }
                }
            }
        }
        Ok(commands)
    }

    fn command(&mut self) -> Result<ParsedCommand, CmdParseError> {
        let assigned = self.peek().matches_equals();
        if assigned {
            self.assignment(false)?;
        }
        let mut arguments = Vec::new();
        match self.peek() {
            Token::Word(_) => {
                let Token::Word(word) = self.take() else {
                    unreachable!()
                };
                arguments.push(ParsedArgument::String(word));
            }
            Token::Newline
            | Token::Semicolon
            | Token::RBrace
            | Token::Elif
            | Token::Else
            | Token::Endif
            | Token::Eof
                if assigned =>
            {
                return Ok(ParsedCommand {
                    line: self.input.line,
                    arguments,
                });
            }
            _ => return Err(self.error()),
        }
        loop {
            match self.peek() {
                Token::Word(_) | Token::Equals(_) => {
                    let word = match self.take() {
                        Token::Word(word) | Token::Equals(word) => word,
                        _ => unreachable!(),
                    };
                    arguments.push(ParsedArgument::String(word));
                }
                Token::LBrace => {
                    self.take();
                    arguments.push(ParsedArgument::Commands(self.statements(true)?));
                }
                _ => break,
            }
        }
        Ok(ParsedCommand {
            line: self.input.line,
            arguments,
        })
    }

    fn format(&mut self) -> Result<bool, CmdParseError> {
        let format = match self.take() {
            Token::Word(format) | Token::Format(format) => format,
            _ => return Err(self.error()),
        };
        Ok(self.context.condition(format.as_ref(), self.input))
    }

    fn condition_body(&mut self, multiline: bool) -> Result<Vec<ParsedCommand>, CmdParseError> {
        if !multiline {
            return self.commands();
        }
        self.expect(Token::Newline)?;
        let mut result = Vec::new();
        let mut count = 0;
        while !matches!(self.peek(), Token::Elif | Token::Else | Token::Endif) {
            if self.peek() == &Token::Eof {
                return Err(self.error());
            }
            result.extend(self.statement()?);
            self.expect(Token::Newline)?;
            count += 1;
        }
        if count == 0 {
            return Err(self.error());
        }
        Ok(result)
    }

    fn condition(
        &mut self,
        allow_multiline: bool,
    ) -> Result<(Vec<ParsedCommand>, bool), CmdParseError> {
        self.expect(Token::If)?;
        let first = self.format()?;
        self.scopes.push(first);
        let multiline = self.peek() == &Token::Newline;
        if multiline && !allow_multiline {
            return Err(self.error());
        }
        let mut selected = if first {
            Some(self.condition_body(multiline)?)
        } else {
            self.condition_body(multiline)?;
            None
        };
        while self.peek() == &Token::Elif {
            self.take();
            let flag = self.format()?;
            *self.scopes.last_mut().expect("condition scope") = flag;
            let commands = self.condition_body(multiline)?;
            if selected.is_none() && flag {
                selected = Some(commands);
            }
        }
        if self.peek() == &Token::Else {
            self.take();
            let scope = self.scopes.last_mut().expect("condition scope");
            *scope = !*scope;
            let commands = self.condition_body(multiline)?;
            if selected.is_none() {
                selected = Some(commands);
            }
        }
        self.expect(Token::Endif)?;
        self.scopes.pop();
        Ok((selected.unwrap_or_default(), multiline))
    }
}

impl Token {
    fn matches_equals(&self) -> bool {
        matches!(self, Self::Equals(_))
    }
}
