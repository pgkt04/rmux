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

use std::io::{BufReader, Read};

use rmux_util::bytes::ByteString;

use super::{CmdParseError, CmdParseInput, ParseContext};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Token {
    Newline,
    Semicolon,
    LBrace,
    RBrace,
    Word(ByteString),
    Equals(ByteString),
    Format(ByteString),
    If,
    Elif,
    Else,
    Endif,
    Hidden,
    Error,
    Eof,
}

enum Source<'a> {
    Buffer { data: &'a [u8], offset: usize },
    Reader(BufReader<&'a mut dyn Read>),
}

pub struct Lexer<'a> {
    source: Source<'a>,
    pushed: [u8; 2],
    pushed_len: usize,
    escapes: u32,
    eol: bool,
    eof: bool,
    condition: bool,
    pub(crate) error: Option<CmdParseError>,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self::with_source(Source::Buffer { data, offset: 0 })
    }

    pub fn from_reader(reader: &'a mut dyn Read) -> Self {
        Self::with_source(Source::Reader(BufReader::new(reader)))
    }

    fn with_source(source: Source<'a>) -> Self {
        Self {
            source,
            pushed: [0; 2],
            pushed_len: 0,
            escapes: 0,
            eol: false,
            eof: false,
            condition: false,
            error: None,
        }
    }

    fn raw(&mut self) -> Option<u8> {
        if self.pushed_len != 0 {
            self.pushed_len -= 1;
            return Some(self.pushed[self.pushed_len]);
        }
        match &mut self.source {
            Source::Buffer { data, offset } => {
                let byte = data.get(*offset).copied();
                if byte.is_some() {
                    *offset += 1;
                }
                // cmd-parse.y reads a signed char buffer; 0xff becomes EOF.
                byte.filter(|byte| *byte != 0xff)
            }
            Source::Reader(reader) => {
                let mut byte = [0];
                match reader.read(&mut byte) {
                    Ok(1) => Some(byte[0]),
                    _ => None,
                }
            }
        }
    }

    fn unget(&mut self, byte: Option<u8>) {
        if let Some(byte) = byte {
            match &mut self.source {
                Source::Buffer { offset, .. } => *offset = offset.saturating_sub(1),
                Source::Reader(_) => {
                    self.pushed[self.pushed_len] = byte;
                    self.pushed_len += 1;
                }
            }
        }
    }

    fn get(&mut self, input: &mut CmdParseInput) -> Option<u8> {
        if self.escapes != 0 {
            self.escapes -= 1;
            return Some(b'\\');
        }
        loop {
            let byte = self.raw();
            if byte == Some(b'\\') {
                self.escapes = self.escapes.wrapping_add(1);
                continue;
            }
            if byte == Some(b'\n') && self.escapes % 2 == 1 {
                input.line = input.line.wrapping_add(1);
                self.escapes -= 1;
                continue;
            }
            if self.escapes != 0 {
                self.unget(byte);
                self.escapes -= 1;
                return Some(b'\\');
            }
            return byte;
        }
    }

    fn fail(&mut self, input: &CmdParseInput, message: &[u8]) {
        if self.error.is_none() {
            self.error = Some(CmdParseError::at(input, message));
        }
    }

    pub fn next(&mut self, context: &mut impl ParseContext, input: &mut CmdParseInput) -> Token {
        if self.eol {
            input.line = input.line.wrapping_add(1);
        }
        self.eol = false;
        let condition = std::mem::take(&mut self.condition);
        loop {
            let Some(mut byte) = self.get(input) else {
                if self.eof {
                    return Token::Eof;
                }
                self.eof = true;
                return Token::Newline;
            };
            if matches!(byte, b' ' | b'\t') {
                continue;
            }
            if byte == b'\r' {
                let next = self.get(input);
                if next == Some(b'\n') {
                    byte = b'\n';
                } else {
                    self.unget(next);
                }
            }
            match byte {
                b'\n' => {
                    self.eol = true;
                    return Token::Newline;
                }
                b';' => return Token::Semicolon,
                b'{' => return Token::LBrace,
                b'}' => return Token::RBrace,
                b'#' => {
                    let mut next = self.get(input);
                    if condition && next == Some(b'{') {
                        return self
                            .format(input)
                            .map(Token::Format)
                            .unwrap_or(Token::Error);
                    }
                    while next.is_some() && next != Some(b'\n') {
                        next = self.get(input);
                    }
                    if next == Some(b'\n') {
                        input.line = input.line.wrapping_add(1);
                        return Token::Newline;
                    }
                }
                b'%' => {
                    // strchr(" \t\n", ch) also matches the terminating NUL.
                    let mut word = vec![byte];
                    loop {
                        let next = self.get(input);
                        if next.is_none() || matches!(next, Some(b' ' | b'\t' | b'\n' | 0)) {
                            self.unget(next);
                            break;
                        }
                        word.push(next.expect("non-EOF word byte"));
                    }
                    if word
                        .iter()
                        .all(|byte| *byte == b'%' || byte.is_ascii_digit())
                    {
                        return Token::Word(word.into());
                    }
                    self.condition = true;
                    return match word.as_slice() {
                        b"%hidden" => Token::Hidden,
                        b"%if" => Token::If,
                        b"%else" => Token::Else,
                        b"%elif" => Token::Elif,
                        b"%endif" => Token::Endif,
                        _ => Token::Error,
                    };
                }
                _ => {
                    let Some(word) = self.word(byte, context, input) else {
                        return Token::Error;
                    };
                    let assignment = word
                        .as_ref()
                        .iter()
                        .position(|byte| *byte == b'=')
                        .is_some_and(|equals| {
                            equals > 0
                                && is_var(word.as_ref()[0], true)
                                && word.as_ref()[1..equals]
                                    .iter()
                                    .all(|byte| is_var(*byte, false))
                        });
                    return if assignment {
                        Token::Equals(word)
                    } else {
                        Token::Word(word)
                    };
                }
            }
        }
    }

    fn format(&mut self, input: &mut CmdParseInput) -> Option<ByteString> {
        let mut result = b"#{".to_vec();
        let mut depth = 1u32;
        loop {
            let mut byte = self.get(input)?;
            if byte == b'\n' {
                return None;
            }
            if byte == b'#' {
                byte = self.get(input)?;
                if byte == b'\n' {
                    return None;
                }
                if byte == b'{' {
                    depth = depth.wrapping_add(1);
                }
                result.push(b'#');
            } else if byte == b'}' {
                depth -= 1;
                if depth == 0 {
                    result.push(byte);
                    truncate_nul(&mut result);
                    return Some(result.into());
                }
            }
            result.push(byte);
        }
    }

    fn escape(&mut self, result: &mut Vec<u8>, input: &mut CmdParseInput) -> Option<()> {
        let byte = self.get(input)?;
        if matches!(byte, b'0'..=b'7') {
            if byte <= b'3' {
                if let Some(second @ b'0'..=b'7') = self.get(input) {
                    if let Some(third @ b'0'..=b'7') = self.get(input) {
                        result.push(64 * (byte - b'0') + 8 * (second - b'0') + (third - b'0'));
                        return Some(());
                    }
                }
            }
            self.fail(input, b"invalid octal escape");
            return None;
        }
        let translated = match byte {
            b'a' => 7,
            b'b' => 8,
            b'e' => 27,
            b'f' => 12,
            b's' => b' ',
            b'v' => 11,
            b'r' => b'\r',
            b'n' => b'\n',
            b't' => b'\t',
            b'u' | b'U' => {
                let size = if byte == b'u' { 4 } else { 8 };
                let mut value = 0u32;
                for _ in 0..size {
                    let digit = self.get(input)?;
                    if digit == b'\n' {
                        return None;
                    }
                    let Some(digit) = (digit as char).to_digit(16) else {
                        self.fail(
                            input,
                            if byte == b'u' {
                                b"invalid \\u argument"
                            } else {
                                b"invalid \\U argument"
                            },
                        );
                        return None;
                    };
                    value = (value << 4) | digit;
                }
                let mut converted = [0; 32];
                let Some(length) = rmux_sys::locale::wctomb(value, &mut converted) else {
                    self.fail(
                        input,
                        if byte == b'u' {
                            b"invalid \\u argument"
                        } else {
                            b"invalid \\U argument"
                        },
                    );
                    return None;
                };
                result.extend_from_slice(&converted[..length]);
                return Some(());
            }
            _ => byte,
        };
        result.push(translated);
        Some(())
    }

    fn variable(
        &mut self,
        result: &mut Vec<u8>,
        context: &mut impl ParseContext,
        input: &mut CmdParseInput,
    ) -> Option<()> {
        let first = self.get(input)?;
        let braced = first == b'{';
        let mut name = Vec::new();
        if !braced {
            if !is_var(first, true) {
                result.push(b'$');
                self.unget(Some(first));
                return Some(());
            }
            name.push(first);
        }
        loop {
            let byte = self.get(input);
            if braced && byte == Some(b'}') {
                break;
            }
            if byte.is_none_or(|byte| !is_var(byte, false)) {
                if !braced {
                    self.unget(byte);
                    break;
                }
                self.fail(input, b"invalid environment variable");
                return None;
            }
            if name.len() == 1022 {
                self.fail(input, b"environment variable is too long");
                return None;
            }
            name.push(byte.expect("variable byte"));
        }
        if let Some(value) = context.environment(&name) {
            result.extend_from_slice(c_string(value));
        }
        Some(())
    }

    fn tilde(
        &mut self,
        result: &mut Vec<u8>,
        context: &mut impl ParseContext,
        input: &mut CmdParseInput,
    ) -> Option<()> {
        let mut name = Vec::new();
        loop {
            let byte = self.get(input);
            // strchr("/ \t\n\"'", ch) also matches the terminating NUL.
            if byte.is_none()
                || matches!(byte, Some(b'/' | b' ' | b'\t' | b'\n' | b'"' | b'\'' | 0))
            {
                self.unget(byte);
                break;
            }
            if name.len() == 1022 {
                self.fail(input, b"user name is too long");
                return None;
            }
            name.push(byte.expect("user name byte"));
        }
        if name.is_empty() {
            if let Some(home) = context
                .environment(b"HOME")
                .filter(|value| !c_string(value).is_empty())
            {
                result.extend_from_slice(c_string(home));
                return Some(());
            }
        }
        let home = context.home(if name.is_empty() { None } else { Some(&name) })?;
        result.extend_from_slice(c_string(home.as_ref()));
        Some(())
    }

    fn word(
        &mut self,
        first: u8,
        context: &mut impl ParseContext,
        input: &mut CmdParseInput,
    ) -> Option<ByteString> {
        #[derive(Clone, Copy, PartialEq)]
        enum State {
            Start,
            None,
            Double,
            Single,
        }
        let mut state = State::None;
        let mut last = State::Start;
        let mut byte = Some(first);
        let mut result = Vec::new();
        while let Some(mut ch) = byte {
            if state == State::None && ch == b'\r' {
                let next = self.get(input);
                if next == Some(b'\n') {
                    ch = b'\n';
                } else {
                    self.unget(next);
                }
            }
            if ch == b'\n' {
                if state == State::None {
                    byte = Some(ch);
                    break;
                }
                input.line = input.line.wrapping_add(1);
            }
            if state == State::None && matches!(ch, b' ' | b'\t' | b';' | b'}') {
                byte = Some(ch);
                break;
            }
            if ch == b'\n' && state != State::None {
                result.push(b'\n');
                byte = self.get(input);
                while matches!(byte, Some(b' ' | b'\t')) {
                    byte = self.get(input);
                }
                if byte != Some(b'#') {
                    continue;
                }
                byte = self.get(input);
                // strchr(",#{}:", ch) also matches the terminating NUL.
                if matches!(byte, Some(b',' | b'#' | b'{' | b'}' | b':' | 0)) {
                    self.unget(byte);
                    byte = Some(b'#');
                } else {
                    loop {
                        byte = self.get(input);
                        if byte.is_none() || byte == Some(b'\n') {
                            break;
                        }
                    }
                }
                continue;
            }
            match ch {
                b'\\' if state != State::Single => self.escape(&mut result, input)?,
                b'~' if last != state && state != State::Single => {
                    self.tilde(&mut result, context, input)?
                }
                b'$' if state != State::Single => self.variable(&mut result, context, input)?,
                b'\'' if state == State::None || state == State::Single => {
                    state = if state == State::None {
                        State::Single
                    } else {
                        State::None
                    };
                    byte = self.get(input);
                    continue;
                }
                b'"' if state == State::None || state == State::Double => {
                    state = if state == State::None {
                        State::Double
                    } else {
                        State::None
                    };
                    byte = self.get(input);
                    continue;
                }
                _ => result.push(ch),
            }
            last = state;
            byte = self.get(input);
        }
        self.unget(byte);
        truncate_nul(&mut result);
        Some(result.into())
    }
}

fn is_var(byte: u8, first: bool) -> bool {
    (!first || !rmux_sys::locale::is_digit(byte))
        && (rmux_sys::locale::is_alnum(byte) || byte == b'_')
}

pub(crate) fn c_string(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())]
}

fn truncate_nul(bytes: &mut Vec<u8>) {
    if let Some(position) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(position);
    }
}
