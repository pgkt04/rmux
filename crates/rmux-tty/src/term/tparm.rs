// Ported from tmux tty-term.c @ 8f25579c; ncurses 6.6 lib_tparm.c
/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER IN AN ACTION
 * OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
 * CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 *
 * Copyright 2018-2024,2025 Thomas E. Dickey
 * Copyright 1998-2016,2017 Free Software Foundation, Inc.
 * Permission is hereby granted, free of charge, to any person obtaining a
 * copy of this software and associated documentation files (the "Software"),
 * to deal in the Software without restriction, including without limitation
 * the rights to use, copy, modify, merge, publish, distribute, distribute
 * with modifications, sublicense, and/or sell copies of the Software, and to
 * permit persons to whom the Software is furnished to do so, subject to the
 * following conditions: The above copyright notice and this permission notice
 * shall be included in all copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
 * THE ABOVE COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
 * IN THE SOFTWARE. Except as contained in this notice, the names of the above
 * copyright holders shall not be used in advertising or otherwise to promote
 * the sale, use or other dealings in this Software without prior written
 * authorization.
 */

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TparmArg<'a> {
    Int(i64),
    Str(&'a [u8]),
}

#[derive(Clone, Debug, Default)]
pub struct TparmState {
    static_vars: [i32; 26],
    branches: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TparmError {
    Malformed(usize),
    StackUnderflow,
    StackOverflow,
    ArgumentType,
    ArgumentCount,
    ArithmeticOverflow,
}

impl fmt::Display for TparmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(at) => write!(f, "malformed parameter expression at byte {at}"),
            Self::StackUnderflow => f.write_str("parameter stack underflow"),
            Self::StackOverflow => f.write_str("parameter stack overflow"),
            Self::ArgumentType => f.write_str("wrong parameter type"),
            Self::ArgumentCount => f.write_str("wrong parameter count"),
            Self::ArithmeticOverflow => f.write_str("parameter arithmetic overflow"),
        }
    }
}

impl std::error::Error for TparmError {}

#[derive(Clone, Copy)]
enum Value<'a> {
    Int(i32),
    Str(&'a [u8]),
}

impl Value<'_> {
    fn number(self) -> Result<i32, TparmError> {
        match self {
            Self::Int(n) => Ok(n),
            Self::Str(_) => Err(TparmError::ArgumentType),
        }
    }
}

struct Stack<'a> {
    values: [Value<'a>; 20],
    len: usize,
}

impl<'a> Stack<'a> {
    fn push(&mut self, value: Value<'a>) -> Result<(), TparmError> {
        let slot = self
            .values
            .get_mut(self.len)
            .ok_or(TparmError::StackOverflow)?;
        *slot = value;
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self) -> Result<Value<'a>, TparmError> {
        self.len = self.len.checked_sub(1).ok_or(TparmError::StackUnderflow)?;
        Ok(self.values[self.len])
    }

    fn number(&mut self) -> Result<i32, TparmError> {
        self.pop()?.number()
    }

    fn string(&mut self) -> Result<&'a [u8], TparmError> {
        match self.pop()? {
            Value::Str(s) => Ok(s),
            Value::Int(_) => Err(TparmError::ArgumentType),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Format {
    width: usize,
    precision: Option<usize>,
    left: bool,
    zero: bool,
    alternate: bool,
    space: bool,
}

#[derive(Clone, Copy)]
enum Token {
    Literal(u8),
    Print(u8, Format),
    Param(usize),
    Store(usize, bool),
    Load(usize, bool),
    Constant(i32, bool),
    Op(u8),
}

fn token(cap: &[u8], pos: &mut usize) -> Result<Token, TparmError> {
    let at = *pos;
    let byte = cap[*pos];
    *pos += 1;
    if byte != b'%' {
        return Ok(Token::Literal(byte));
    }
    let op = *cap.get(*pos).ok_or(TparmError::Malformed(at))?;
    *pos += 1;
    match op {
        b'%' => Ok(Token::Literal(b'%')),
        b'p' => {
            let p = *cap.get(*pos).ok_or(TparmError::Malformed(at))?;
            *pos += 1;
            if !(b'1'..=b'9').contains(&p) {
                return Err(TparmError::Malformed(at));
            }
            Ok(Token::Param((p - b'1') as usize))
        }
        b'P' | b'g' => {
            let var = *cap.get(*pos).ok_or(TparmError::Malformed(at))?;
            *pos += 1;
            let (index, persistent) = if var.is_ascii_lowercase() {
                ((var - b'a') as usize, false)
            } else if var.is_ascii_uppercase() {
                ((var - b'A') as usize, true)
            } else {
                return Err(TparmError::Malformed(at));
            };
            Ok(if op == b'P' {
                Token::Store(index, persistent)
            } else {
                Token::Load(index, persistent)
            })
        }
        b'\'' => {
            let ch = *cap.get(*pos).ok_or(TparmError::Malformed(at))?;
            if cap.get(*pos + 1) != Some(&b'\'') {
                return Err(TparmError::Malformed(at));
            }
            *pos += 2;
            Ok(Token::Constant(i32::from(ch), true))
        }
        b'{' => {
            let start = *pos;
            let mut value = 0_i32;
            while let Some(&ch @ b'0'..=b'9') = cap.get(*pos) {
                value = value.wrapping_mul(10).wrapping_add(i32::from(ch - b'0'));
                *pos += 1;
            }
            if *pos == start || cap.get(*pos) != Some(&b'}') {
                return Err(TparmError::Malformed(at));
            }
            *pos += 1;
            Ok(Token::Constant(value, false))
        }
        b'd' | b'o' | b'x' | b'X' | b's' | b'c' => Ok(Token::Print(op, Format::default())),
        b':' | b'#' | b' ' | b'.' | b'0'..=b'9' => {
            *pos -= 1;
            let mut format = Format::default();
            let colon = cap.get(*pos) == Some(&b':');
            if colon {
                *pos += 1;
            }
            loop {
                match cap.get(*pos) {
                    Some(b'-') if colon => format.left = true,
                    Some(b' ') => format.space = true,
                    Some(b'#') => format.alternate = true,
                    Some(b'0') => format.zero = true,
                    _ => break,
                }
                *pos += 1;
            }
            format.width = decimal(cap, pos, at)?;
            if cap.get(*pos) == Some(&b'.') {
                *pos += 1;
                format.precision = Some(decimal(cap, pos, at)?);
            }
            let conversion = *cap.get(*pos).ok_or(TparmError::Malformed(at))?;
            *pos += 1;
            // ncurses parse_format does not accept '+' as a printf flag:
            // even after ':', it executes the arithmetic operator instead.
            if conversion == b'+' {
                return Ok(Token::Op(b'+'));
            }
            if !b"doxXsc".contains(&conversion) {
                return Err(TparmError::Malformed(at));
            }
            Ok(Token::Print(conversion, format))
        }
        b'l' | b'+' | b'-' | b'*' | b'/' | b'm' | b'&' | b'|' | b'^' | b'=' | b'>' | b'<'
        | b'A' | b'O' | b'!' | b'~' | b'i' | b'?' | b't' | b'e' | b';' => Ok(Token::Op(op)),
        _ => Err(TparmError::Malformed(at)),
    }
}

fn decimal(cap: &[u8], pos: &mut usize, at: usize) -> Result<usize, TparmError> {
    let mut value = 0_usize;
    while let Some(&ch @ b'0'..=b'9') = cap.get(*pos) {
        value = value
            .checked_mul(10)
            .and_then(|n| n.checked_add((ch - b'0') as usize))
            .ok_or(TparmError::Malformed(at))?;
        if value > 10000 {
            return Err(TparmError::Malformed(at));
        }
        *pos += 1;
    }
    Ok(value)
}

// ncurses' implicit termcap parameters are inferred before execution.
fn analyze(cap: &[u8], branches: &mut Vec<u8>) -> Result<(usize, usize, u16), TparmError> {
    let mut pos = 0;
    branches.clear();
    let mut level = -1_i32;
    let mut lastpop = -1_i32;
    let mut number = 0_usize;
    let mut highest = 0;
    let mut strings = 0_u16;
    while pos < cap.len() {
        let at = pos;
        let t = token(cap, &mut pos)?;
        match t {
            Token::Param(n) => {
                highest = highest.max(n + 1);
                level += 1;
                lastpop = n as i32 + 1;
            }
            Token::Constant(_, character) => {
                level += 1;
                if character {
                    lastpop = -1;
                }
            }
            Token::Load(..) => level += 1,
            Token::Print(b's', _) | Token::Op(b'l') => {
                if lastpop > 0 {
                    level -= 1;
                    strings |= 1 << (lastpop - 1);
                }
                if level < 0 && number < 2 {
                    number += 1;
                }
            }
            Token::Print(..) => {
                if lastpop <= 0 && level < 0 && number < 2 {
                    number += 1;
                }
                level -= 1;
                lastpop = -1;
            }
            Token::Op(b'?') => branches.push(0),
            Token::Op(b';') => {
                if branches.pop().ok_or(TparmError::Malformed(at))? == 0 {
                    return Err(TparmError::Malformed(at));
                }
            }
            Token::Op(b't') => {
                let branch = branches.last_mut().ok_or(TparmError::Malformed(at))?;
                if *branch == 1 {
                    return Err(TparmError::Malformed(at));
                }
                *branch = 1;
            }
            Token::Op(b'e') => {
                let branch = branches.last_mut().ok_or(TparmError::Malformed(at))?;
                if *branch != 1 {
                    return Err(TparmError::Malformed(at));
                }
                *branch = 2;
            }
            Token::Op(op) if b"+-*/mAO&|^=<>!~".contains(&op) => {
                if level < 0 && number < 2 {
                    number += 1;
                }
                if !b"!~".contains(&op) {
                    level -= 1;
                }
                lastpop = -1;
            }
            _ => {}
        }
    }
    if !branches.is_empty() {
        return Err(TparmError::Malformed(cap.len()));
    }
    Ok((number, highest, strings))
}

fn skip(cap: &[u8], pos: &mut usize, stop_else: bool) -> Result<(), TparmError> {
    let mut depth = 0;
    while *pos < cap.len() {
        match token(cap, pos)? {
            Token::Op(b'?') => depth += 1,
            Token::Op(b';') if depth == 0 => return Ok(()),
            Token::Op(b';') => depth -= 1,
            Token::Op(b'e') if depth == 0 && stop_else => return Ok(()),
            _ => {}
        }
    }
    Err(TparmError::Malformed(cap.len()))
}

fn padding(out: &mut Vec<u8>, byte: u8, count: usize) {
    out.resize(out.len() + count, byte);
}

fn print_value(
    stack: &mut Stack<'_>,
    conversion: u8,
    format: Format,
    out: &mut Vec<u8>,
) -> Result<(), TparmError> {
    if conversion == b's' {
        let value = stack.string()?;
        let len = format.precision.unwrap_or(value.len()).min(value.len());
        let pad = format.width.saturating_sub(len);
        if !format.left {
            padding(out, b' ', pad);
        }
        out.extend_from_slice(&value[..len]);
        if format.left {
            padding(out, b' ', pad);
        }
        return Ok(());
    }
    let n = stack.number()?;
    if conversion == b'c' {
        // save_char substitutes 0200 for integer zero, before the byte cast.
        out.push(if n == 0 { 0x80 } else { n as u8 });
        return Ok(());
    }
    let base = if conversion == b'o' {
        8
    } else if conversion == b'd' {
        10
    } else {
        16
    };
    let negative = conversion == b'd' && n < 0;
    let mut value = if conversion == b'd' {
        n.unsigned_abs()
    } else {
        n as u32
    };
    let mut digits = [0_u8; 32];
    let mut first = digits.len();
    if value != 0 || format.precision != Some(0) {
        loop {
            first -= 1;
            let digit = (value % base) as u8;
            digits[first] = if digit < 10 {
                b'0' + digit
            } else if conversion == b'X' {
                b'A' + digit - 10
            } else {
                b'a' + digit - 10
            };
            value /= base;
            if value == 0 {
                break;
            }
        }
    }
    let sign = if negative {
        Some(b'-')
    } else if conversion == b'd' && format.space {
        Some(b' ')
    } else {
        None
    };
    let prefix: &[u8] = if format.alternate && n != 0 && conversion == b'x' {
        b"0x"
    } else if format.alternate && n != 0 && conversion == b'X' {
        b"0X"
    } else {
        b""
    };
    let mut zeros = format
        .precision
        .unwrap_or(0)
        .saturating_sub(digits.len() - first);
    if format.alternate && conversion == b'o' && zeros == 0 && digits.get(first) != Some(&b'0') {
        zeros = 1;
    }
    let len = digits.len() - first + zeros + prefix.len() + usize::from(sign.is_some());
    let pad = format.width.saturating_sub(len);
    let zero_pad = format.zero && !format.left && format.precision.is_none();
    if !format.left && !zero_pad {
        padding(out, b' ', pad);
    }
    if let Some(sign) = sign {
        out.push(sign);
    }
    out.extend_from_slice(prefix);
    padding(out, b'0', zeros + if zero_pad { pad } else { 0 });
    out.extend_from_slice(&digits[first..]);
    if format.left {
        padding(out, b' ', pad);
    }
    Ok(())
}

pub fn expand(
    state: &mut TparmState,
    cap: &[u8],
    args: &[TparmArg<'_>],
    out: &mut Vec<u8>,
) -> Result<(), TparmError> {
    out.clear();
    let result = expand_inner(state, rmux_util::bytes::cstr(cap), args, out);
    if result.is_err() {
        out.clear();
    } else if let Some(end) = out.iter().position(|&ch| ch == 0) {
        out.truncate(end);
    }
    result
}

fn expand_inner(
    state: &mut TparmState,
    cap: &[u8],
    args: &[TparmArg<'_>],
    out: &mut Vec<u8>,
) -> Result<(), TparmError> {
    let (implicit, highest, strings) = analyze(cap, &mut state.branches)?;
    if args.len() != implicit.max(highest) {
        return Err(TparmError::ArgumentCount);
    }
    let arg_strings = args.iter().enumerate().fold(0_u16, |mask, (index, arg)| {
        mask | if matches!(arg, TparmArg::Str(_)) {
            1 << index
        } else {
            0
        }
    });
    if strings != arg_strings {
        return Err(TparmError::ArgumentType);
    }
    let explicit = highest != 0;
    let mut params = [Value::Int(0); 9];
    for (slot, arg) in params.iter_mut().zip(args) {
        *slot = match *arg {
            TparmArg::Int(n) => Value::Int(n as i32),
            TparmArg::Str(s) => Value::Str(rmux_util::bytes::cstr(s)),
        };
    }
    let mut stack = Stack {
        values: [Value::Int(0); 20],
        len: 0,
    };
    if !explicit {
        for &value in params[..implicit].iter().rev() {
            stack.push(value)?;
        }
    }
    let mut dynamic_vars = [0_i32; 26];
    let mut incremented = false;
    let mut pos = 0;
    while pos < cap.len() {
        match token(cap, &mut pos)? {
            Token::Literal(ch) => out.push(ch),
            Token::Print(conversion, format) => print_value(&mut stack, conversion, format, out)?,
            Token::Param(n) => stack.push(params[n])?,
            Token::Constant(n, _) => stack.push(Value::Int(n))?,
            Token::Store(n, persistent) => {
                let value = stack.number()?;
                if persistent {
                    state.static_vars[n] = value;
                } else {
                    dynamic_vars[n] = value;
                }
            }
            Token::Load(n, persistent) => stack.push(Value::Int(if persistent {
                state.static_vars[n]
            } else {
                dynamic_vars[n]
            }))?,
            Token::Op(b'i') => {
                if !incremented {
                    incremented = true;
                    for (n, value) in params[..2].iter_mut().enumerate() {
                        if let Value::Int(value) = value {
                            *value = value.wrapping_add(1);
                            if !explicit {
                                stack.values[n] = Value::Int(*value);
                            }
                        }
                    }
                }
            }
            Token::Op(b'l') => {
                let n = stack.string()?.len() as i32;
                stack.push(Value::Int(n))?;
            }
            Token::Op(b'!') => {
                let n = stack.number()?;
                stack.push(Value::Int(i32::from(n == 0)))?;
            }
            Token::Op(b'~') => {
                let n = stack.number()?;
                stack.push(Value::Int(!n))?;
            }
            Token::Op(b't') => {
                if stack.number()? == 0 {
                    skip(cap, &mut pos, true)?;
                }
            }
            Token::Op(b'e') => skip(cap, &mut pos, false)?,
            Token::Op(b'?' | b';') => {}
            Token::Op(op) => {
                let y = stack.number()?;
                let x = stack.number()?;
                let n = match op {
                    b'+' => x.wrapping_add(y),
                    b'-' => x.wrapping_sub(y),
                    b'*' => x.wrapping_mul(y),
                    b'/' => {
                        if y == 0 {
                            0
                        } else {
                            x.checked_div(y).ok_or(TparmError::ArithmeticOverflow)?
                        }
                    }
                    b'm' => {
                        if y == 0 {
                            0
                        } else {
                            x.checked_rem(y).ok_or(TparmError::ArithmeticOverflow)?
                        }
                    }
                    b'&' => x & y,
                    b'|' => x | y,
                    b'^' => x ^ y,
                    b'=' => i32::from(x == y),
                    b'<' => i32::from(x < y),
                    b'>' => i32::from(x > y),
                    b'A' => i32::from(x != 0 && y != 0),
                    b'O' => i32::from(x != 0 || y != 0),
                    _ => unreachable!(),
                };
                stack.push(Value::Int(n))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expanded(cap: &[u8], args: &[TparmArg<'_>]) -> Vec<u8> {
        let mut out = Vec::new();
        expand(&mut TparmState::default(), cap, args, &mut out).unwrap();
        out
    }

    #[test]
    fn operators_and_formats() {
        assert_eq!(
            expanded(b"%%:%i%i%p1%d;%p2%d", &[TparmArg::Int(3), TparmArg::Int(9)]),
            b"%:4;10"
        );
        assert_eq!(expanded(b"%p1%{3}%+%{2}%*%d", &[TparmArg::Int(7)]), b"20");
        assert_eq!(expanded(b"%p1%: #08x", &[TparmArg::Int(42)]), b"0x00002a");
        assert_eq!(
            expanded(b"%p1%:-8.3s", &[TparmArg::Str(b"abcdef\0z")]),
            b"abc     "
        );
        assert_eq!(expanded(b"%p1%l%d", &[TparmArg::Str(b"abc\0z")]), b"3");
        assert_eq!(expanded(b"%{0}%c", &[]), [0x80]);
        assert!(expanded(b"%{256}%cafter", &[]).is_empty());
        assert_eq!(expanded(b"%{1}%{0}%/%d:%{1}%{0}%m%d", &[]), b"0:0");
        assert_eq!(expanded(b"%{2147483647}%{1}%+%d", &[]), b"-2147483648");
        assert_eq!(
            expanded(b"%d;%d", &[TparmArg::Int(5), TparmArg::Int(6)]),
            b"5;6"
        );
    }

    #[test]
    fn nested_conditionals_and_else_if() {
        let cap = b"%?%p1%{1}%=%t%?%p2%tA%eB%;%e%p1%{2}%=%tC%eD%;";
        for (a, b, expected) in [(1, 1, b"A"), (1, 0, b"B"), (2, 0, b"C"), (3, 0, b"D")] {
            assert_eq!(
                expanded(cap, &[TparmArg::Int(a), TparmArg::Int(b)]),
                expected
            );
        }
    }

    #[test]
    fn variables_are_process_owned_integers() {
        let mut state = TparmState::default();
        let mut out = Vec::new();
        expand(&mut state, b"%{42}%PA%{9}%Pa%gA%d:%ga%d", &[], &mut out).unwrap();
        assert_eq!(out, b"42:9");
        expand(&mut state, b"%gA%d:%ga%d", &[], &mut out).unwrap();
        assert_eq!(out, b"42:0");
    }

    #[test]
    fn malformed_and_wrong_types_clear_output() {
        for cap in [
            b"x%".as_slice(),
            b"%p0",
            b"%{2",
            b"%?%{1}%t",
            b"%e",
            b"%p1%s",
            b"%d%d%d",
        ] {
            let mut out = b"old".to_vec();
            assert!(
                expand(
                    &mut TparmState::default(),
                    cap,
                    &[TparmArg::Int(1)],
                    &mut out
                )
                .is_err(),
                "{cap:?}"
            );
            assert!(out.is_empty());
        }
        assert_eq!(
            expand(
                &mut TparmState::default(),
                b"%p1%d",
                &[TparmArg::Str(b"x")],
                &mut Vec::new()
            ),
            Err(TparmError::ArgumentType)
        );
        assert_eq!(
            expand(
                &mut TparmState::default(),
                b"constant",
                &[TparmArg::Int(1)],
                &mut Vec::new()
            ),
            Err(TparmError::ArgumentCount)
        );
        assert_eq!(
            expand(
                &mut TparmState::default(),
                b"%p2%d",
                &[TparmArg::Int(1)],
                &mut Vec::new()
            ),
            Err(TparmError::ArgumentCount)
        );
        assert_eq!(
            expand(
                &mut TparmState::default(),
                b"%p1%{2147483648}%/%d",
                &[TparmArg::Int(i64::from(i32::MIN))],
                &mut Vec::new()
            ),
            Ok(())
        );
        assert_eq!(
            expand(
                &mut TparmState::default(),
                b"%p1%{0}%{1}%-%/%d",
                &[TparmArg::Int(i64::from(i32::MIN))],
                &mut Vec::new()
            ),
            Err(TparmError::ArithmeticOverflow)
        );
        assert!(expand(&mut TparmState::default(), b"%?%;", &[], &mut Vec::new()).is_err());
    }
}
