// Ported from tmux cmd-parse.y, cmd.c @ 8f25579c
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

use super::grammar::{ParsedArgument, ParsedCommand};
use super::lexer::{Lexer, c_string};
use super::{CmdParseError, CmdParseFlags, CmdParseInput, CmdParseResult, ParseContext, grammar};
use crate::cmd::arguments::{self, ArgsValue, ArgsValueData};
use crate::cmd::{Command, CommandList, find_entry};

struct CommandListBuilder {
    group: u32,
    commands: Vec<Command>,
}

impl CommandListBuilder {
    fn new(context: &mut impl ParseContext) -> Self {
        Self {
            group: context.next_group(),
            commands: Vec::new(),
        }
    }

    fn append_all(&mut self, list: CommandListBuilder) {
        self.commands.reserve(list.commands.len());
        for mut command in list.commands {
            command.group = self.group;
            self.commands.push(command);
        }
    }

    fn move_all(&mut self, context: &mut impl ParseContext, list: CommandListBuilder) {
        self.commands.extend(list.commands);
        self.group = context.next_group();
    }

    fn publish(self) -> Rc<CommandList> {
        Rc::new(CommandList {
            group: self.group,
            commands: self.commands,
        })
    }

    fn print(&self) -> rmux_util::bytes::ByteString {
        let mut text = Vec::new();
        for command in &self.commands {
            if !text.is_empty() {
                text.extend_from_slice(b" ; ");
            }
            text.extend_from_slice(command.print().as_ref());
        }
        text.into()
    }
}

fn verbose(context: &mut impl ParseContext, input: &CmdParseInput, list: &CommandListBuilder) {
    if input.item.is_none() || !input.flags.contains(CmdParseFlags::VERBOSE) {
        return;
    }
    let mut message = Vec::new();
    if let Some(file) = &input.file {
        message.extend_from_slice(file.as_ref());
        message.push(b':');
    }
    message.extend_from_slice(format!("{}: ", input.line).as_bytes());
    message.extend_from_slice(list.print().as_ref());
    context.print(&message, input);
}

pub fn commands(
    context: &mut impl ParseContext,
    parsed: Vec<ParsedCommand>,
    input: &mut CmdParseInput,
) -> CmdParseResult {
    Ok(build_commands(context, parsed, input)?.publish())
}

fn build_commands(
    context: &mut impl ParseContext,
    parsed: Vec<ParsedCommand>,
    input: &mut CmdParseInput,
) -> Result<CommandListBuilder, CmdParseError> {
    let mut result = CommandListBuilder::new(context);
    if parsed.is_empty() {
        return Ok(result);
    }
    let mut line = u32::MAX;
    let mut current: Option<CommandListBuilder> = None;
    for command in parsed {
        if !input.flags.contains(CmdParseFlags::ONEGROUP) && command.line != line {
            if let Some(previous) = current.take() {
                verbose(context, input, &previous);
                result.move_all(context, previous);
            }
            current = Some(CommandListBuilder::new(context));
        }
        if current.is_none() {
            current = Some(CommandListBuilder::new(context));
        }
        line = command.line;
        input.line = line;
        let list = build_command(context, command, input)?;
        current.as_mut().expect("current group").append_all(list);
    }
    if let Some(current) = current {
        verbose(context, input, &current);
        result.move_all(context, current);
    }
    Ok(result)
}

fn build_command(
    context: &mut impl ParseContext,
    mut parsed: ParsedCommand,
    input: &mut CmdParseInput,
) -> Result<CommandListBuilder, CmdParseError> {
    if !input.flags.contains(CmdParseFlags::NOALIAS) {
        let Some(ParsedArgument::String(name)) = parsed.arguments.first() else {
            return Ok(CommandListBuilder::new(context));
        };
        if let Some(alias) = context.alias(name.as_ref()) {
            let mut commands =
                grammar::parse(Lexer::new(c_string(alias.as_ref())), context, input)?;
            let Some(last) = commands.last_mut() else {
                return Ok(CommandListBuilder::new(context));
            };
            last.arguments.extend(parsed.arguments.drain(1..));
            input.flags.insert(CmdParseFlags::NOALIAS);
            let result = build_commands(context, commands, input);
            input.flags.remove(CmdParseFlags::NOALIAS);
            return result;
        }
    }
    let mut values = Vec::with_capacity(parsed.arguments.len());
    for argument in parsed.arguments {
        values.push(match argument {
            ParsedArgument::String(string) => ArgsValue::string(string),
            ParsedArgument::Commands(commands) => {
                ArgsValue::commands(build_commands(context, commands, input)?.publish())
            }
            ParsedArgument::ParsedCommands(commands) => ArgsValue::commands(commands),
        });
    }
    let Some(ArgsValue {
        data: ArgsValueData::String(name),
        ..
    }) = values.first()
    else {
        return Err(CmdParseError::at(input, b"no command"));
    };
    let entry =
        find_entry(name.as_ref()).map_err(|error| CmdParseError::at(input, error.as_ref()))?;
    let args = match arguments::parse(&entry.args, &values) {
        Ok(Some(args)) => args,
        Ok(None) => {
            let mut message = b"usage: ".to_vec();
            message.extend_from_slice(entry.name);
            message.push(b' ');
            message.extend_from_slice(entry.usage);
            return Err(CmdParseError::at(input, &message));
        }
        Err(error) => {
            let mut message = b"command ".to_vec();
            message.extend_from_slice(entry.name);
            message.extend_from_slice(b": ");
            message.extend_from_slice(error.as_ref());
            return Err(CmdParseError::at(input, &message));
        }
    };
    let mut result = CommandListBuilder::new(context);
    result.commands.push(Command {
        entry,
        args,
        group: result.group,
        file: input.file.clone(),
        line: input.line,
        parse_flags: input.flags,
    });
    Ok(result)
}

pub fn from_arguments(
    context: &mut impl ParseContext,
    values: &[ArgsValue],
    input: &mut CmdParseInput,
) -> CmdParseResult {
    let mut parsed = Vec::new();
    let mut command = ParsedCommand {
        line: input.line,
        arguments: Vec::new(),
    };
    for value in values {
        let mut end = false;
        match &value.data {
            ArgsValueData::String(string) => {
                let mut copy = c_string(string.as_ref()).to_vec();
                if copy.last() == Some(&b';') {
                    copy.pop();
                    if copy.last() == Some(&b'\\') {
                        *copy.last_mut().expect("final backslash") = b';';
                    } else {
                        end = true;
                    }
                }
                if !end || !copy.is_empty() {
                    command.arguments.push(ParsedArgument::String(copy.into()));
                }
            }
            ArgsValueData::Commands(commands) => command
                .arguments
                .push(ParsedArgument::ParsedCommands(Rc::clone(commands))),
            ArgsValueData::None => panic!("unknown argument type"),
        }
        if end {
            parsed.push(command);
            command = ParsedCommand {
                line: input.line,
                arguments: Vec::new(),
            };
        }
    }
    if !command.arguments.is_empty() {
        parsed.push(command);
    }
    commands(context, parsed, input)
}
