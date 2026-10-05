// Ported from tmux json.c @ 8f25579c
/*
 * Copyright (c) 2026 Dane Jensen <dhcjensen@gmail.com>
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

//! The tmux JSON subset: the top value is an object, arrays hold only
//! objects, numbers are base-10 `int64_t`, there is no null, strings and keys
//! are nonempty, escapes are validated but kept encoded, duplicate raw keys
//! are rejected and objects nest at most 200 deep.
//! Bare values require a separator after their last byte; EOF is a
//! tokenization error rather than a later missing-object error.

use std::collections::BTreeMap;
use std::fmt;

use rmux_util::bytes::{ByteString, cstr};

const ERROR_CTX_LEN: usize = 8;
const PARSE_DEPTH_MAX: u32 = 200;

/// A parse or typed-find failure with the exact tmux cause bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub cause: ByteString,
}

impl JsonError {
    pub fn cause(&self) -> &[u8] {
        &self.cause
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.cause)
    }
}

impl std::error::Error for JsonError {}

/// One parsed node. Strings keep their encoded escape bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonNode {
    String(ByteString),
    Number(i64),
    Boolean(bool),
    Object(BTreeMap<ByteString, JsonNode>),
    Array(Vec<JsonNode>),
}

fn type_article(name: &str) -> &'static str {
    match name {
        "object" | "array" => "an",
        _ => "a",
    }
}

fn not_found(key: &[u8]) -> JsonError {
    let mut cause = Vec::with_capacity(key.len() + 16);
    cause.extend_from_slice(b"key \"");
    cause.extend_from_slice(key);
    cause.extend_from_slice(b"\" not found");
    JsonError {
        cause: cause.into(),
    }
}

fn expected(key: &[u8], name: &str) -> JsonError {
    let mut cause = Vec::with_capacity(key.len() + 32);
    cause.extend_from_slice(b"key \"");
    cause.extend_from_slice(key);
    cause.extend_from_slice(b"\" expected ");
    cause.extend_from_slice(type_article(name).as_bytes());
    cause.push(b' ');
    cause.extend_from_slice(name.as_bytes());
    JsonError {
        cause: cause.into(),
    }
}

impl JsonNode {
    /// `json_find`: a field of an object node (`json.c:173-183`).
    pub fn find(&self, key: &[u8]) -> Option<&JsonNode> {
        match self {
            JsonNode::Object(fields) => fields.get(cstr(key)),
            _ => None,
        }
    }

    /// `json_get_string`.
    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            JsonNode::String(s) => Some(s),
            _ => None,
        }
    }

    /// `json_get_number`.
    pub fn as_number(&self) -> Option<i64> {
        match self {
            JsonNode::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// `json_get_boolean`.
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            JsonNode::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// `json_get_object`.
    pub fn as_object(&self) -> Option<&BTreeMap<ByteString, JsonNode>> {
        match self {
            JsonNode::Object(fields) => Some(fields),
            _ => None,
        }
    }

    /// `json_get_array`; members are in source order (`json_array_first`,
    /// `json_array_next`).
    pub fn as_array(&self) -> Option<&[JsonNode]> {
        match self {
            JsonNode::Array(members) => Some(members),
            _ => None,
        }
    }

    fn find_typed<'a, T>(
        &'a self,
        key: &[u8],
        name: &str,
        get: impl FnOnce(&'a JsonNode) -> Option<T>,
    ) -> Result<T, JsonError> {
        let key = cstr(key);
        let field = self.find(key).ok_or_else(|| not_found(key))?;
        get(field).ok_or_else(|| expected(key, name))
    }

    /// `json_find_string` (`json.c:263-281`).
    pub fn find_string(&self, key: &[u8]) -> Result<&[u8], JsonError> {
        self.find_typed(key, "string", JsonNode::as_string)
    }

    /// `json_find_number` (`json.c:285-303`).
    pub fn find_number(&self, key: &[u8]) -> Result<i64, JsonError> {
        self.find_typed(key, "number", JsonNode::as_number)
    }

    /// `json_find_boolean` (`json.c:307-324`).
    pub fn find_boolean(&self, key: &[u8]) -> Result<bool, JsonError> {
        self.find_typed(key, "boolean", JsonNode::as_boolean)
    }

    /// `json_find_object` (`json.c:328-346`).
    pub fn find_object(&self, key: &[u8]) -> Result<&JsonNode, JsonError> {
        self.find_typed(key, "object", |n| n.as_object().map(|_| n))
    }

    /// `json_find_array` (`json.c:350-368`).
    pub fn find_array(&self, key: &[u8]) -> Result<&[JsonNode], JsonError> {
        self.find_typed(key, "array", JsonNode::as_array)
    }

    fn append(&self, out: &mut Vec<u8>) {
        match self {
            JsonNode::String(s) => {
                out.push(b'"');
                out.extend_from_slice(s);
                out.push(b'"');
            }
            JsonNode::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
            JsonNode::Boolean(true) => out.extend_from_slice(b"true"),
            JsonNode::Boolean(false) => out.extend_from_slice(b"false"),
            JsonNode::Object(fields) => {
                out.push(b'{');
                for (i, (key, field)) in fields.iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    out.push(b'"');
                    out.extend_from_slice(key);
                    out.extend_from_slice(b"\":");
                    field.append(out);
                }
                out.push(b'}');
            }
            JsonNode::Array(members) => {
                out.push(b'[');
                for (i, member) in members.iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    member.append(out);
                }
                out.push(b']');
            }
        }
    }

    /// `json_to_string`: canonical form without whitespace, object keys in
    /// byte order, arrays in source order, escapes unchanged
    /// (`json.c:940-1001`).
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> ByteString {
        let mut out = Vec::new();
        self.append(&mut out);
        out.into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenType {
    OpenObject,
    CloseObject,
    OpenArray,
    CloseArray,
    Comma,
    Colon,
    Quote,
    Value,
    Eof,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    kind: TokenType,
    offset: usize,
    len: usize,
}

/// `json_error`: the reason, then `: ` and up to eight context bytes with
/// `...` only when more remain (`json.c:371-392`).
fn error(reason: &str, loc: Option<&[u8]>) -> JsonError {
    let mut cause = Vec::with_capacity(reason.len() + ERROR_CTX_LEN + 8);
    cause.extend_from_slice(reason.as_bytes());
    if let Some(loc) = loc
        && !loc.is_empty()
    {
        cause.extend_from_slice(b": ");
        cause.extend_from_slice(&loc[..loc.len().min(ERROR_CTX_LEN)]);
        if loc.len() > ERROR_CTX_LEN {
            cause.extend_from_slice(b"...");
        }
    }
    JsonError {
        cause: cause.into(),
    }
}

fn is_c_space(b: u8) -> bool {
    b == b' ' || (0x09..=0x0d).contains(&b)
}

/// `json_tokenize_value`: a string body ends at `"`, a bare value after a
/// colon ends at `,`, `]`, `}` or whitespace (`json.c:470-522`).
fn tokenize_value(prev: Option<&Token>, loc: &[u8]) -> Option<usize> {
    let prev = prev?;
    let mut scan = 0usize;
    match prev.kind {
        TokenType::Quote => {
            while loc.get(scan).copied() != Some(b'"') {
                let b = *loc.get(scan)?;
                if b < 0x20 {
                    return None;
                }
                if b != b'\\' {
                    scan += 1;
                    continue;
                }
                scan += 1;
                match loc.get(scan).copied() {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => scan += 1,
                    Some(b'u') => {
                        for i in 1..=4 {
                            if !loc.get(scan + i).is_some_and(u8::is_ascii_hexdigit) {
                                return None;
                            }
                        }
                        scan += 5;
                    }
                    _ => return None,
                }
            }
            Some(scan)
        }
        TokenType::Colon => {
            loop {
                loc.get(scan)?;
                scan += 1;
                match loc.get(scan) {
                    None => return None,
                    Some(b']' | b'}' | b',') => break,
                    Some(&b) if is_c_space(b) => break,
                    Some(_) => {}
                }
            }
            Some(scan)
        }
        _ => None,
    }
}

/// `json_tokenize_input` (`json.c:395-464`).
fn tokenize(input: &[u8]) -> Result<Vec<Token>, JsonError> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut in_string = false;
    let mut pos = 0usize;
    let mut loc = 0usize;
    while pos < input.len() {
        loc = pos;
        let mut scan = 1usize;
        let kind = if in_string && input[pos] != b'"' {
            TokenType::Value
        } else {
            match input[pos] {
                b' ' | b'\t' | b'\n' | b'\r' => {
                    pos += 1;
                    continue;
                }
                b'{' => TokenType::OpenObject,
                b'}' => TokenType::CloseObject,
                b'[' => TokenType::OpenArray,
                b']' => TokenType::CloseArray,
                b'"' => TokenType::Quote,
                b':' => TokenType::Colon,
                b',' => TokenType::Comma,
                _ => TokenType::Value,
            }
        };
        if kind == TokenType::Value {
            scan = tokenize_value(tokens.last(), &input[loc..])
                .ok_or_else(|| error("tokenization error", Some(&input[loc..])))?;
            pos += scan - 1;
        }
        tokens.push(Token {
            kind,
            offset: loc,
            len: scan,
        });
        if kind == TokenType::Quote {
            in_string = !in_string;
        }
        pos += 1;
    }
    tokens.push(Token {
        kind: TokenType::Eof,
        offset: loc,
        len: 0,
    });
    Ok(tokens)
}

struct Parser<'a> {
    input: &'a [u8],
    tokens: Vec<Token>,
    pos: usize,
    depth: u32,
}

impl<'a> Parser<'a> {
    fn tok(&self) -> Token {
        self.tokens[self.pos]
    }

    fn loc(&self, tok: Token) -> Option<&'a [u8]> {
        Some(&self.input[tok.offset..])
    }

    fn fail<T>(&self, reason: &str, tok: Token) -> Result<T, JsonError> {
        Err(error(reason, self.loc(tok)))
    }

    /// `json_parse_key` (`json.c:687-715`).
    fn parse_key(&mut self) -> Result<ByteString, JsonError> {
        let start = self.tok();
        if start.kind != TokenType::Quote {
            return self.fail("invalid key", start);
        }
        self.pos += 1;
        let value = self.tok();
        if value.kind != TokenType::Value {
            return self.fail("invalid key", start);
        }
        self.pos += 1;
        if self.tok().kind != TokenType::Quote {
            return self.fail("invalid key", start);
        }
        self.pos += 1;
        Ok(self.input[value.offset..value.offset + value.len].into())
    }

    /// `json_parse_object` (`json.c:718-808`).
    fn parse_object(&mut self) -> Result<JsonNode, JsonError> {
        let open = self.tok();
        self.depth += 1;
        if self.depth > PARSE_DEPTH_MAX {
            return self.fail("parse depth exceeded", open);
        }
        self.pos += 1;

        let mut fields = BTreeMap::new();
        while self.tok().kind != TokenType::CloseObject {
            let key = self.parse_key()?;
            if fields.contains_key(&key) {
                return self.fail("duplicate key", self.tok());
            }
            if self.tok().kind != TokenType::Colon {
                return self.fail("missing colon", self.tok());
            }
            self.pos += 1;

            let value = self.tok();
            let field = match value.kind {
                TokenType::Quote => self.parse_string()?,
                TokenType::Value => {
                    let bytes = &self.input[value.offset..];
                    let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
                    if (bytes[0] == b'-' && digit(1)) || digit(0) {
                        self.parse_number()?
                    } else {
                        self.parse_boolean()?
                    }
                }
                TokenType::OpenObject => self.parse_object()?,
                TokenType::OpenArray => self.parse_array()?,
                _ => return self.fail("unexpected value when parsing object", value),
            };
            fields.insert(key, field);

            let next = self.tok();
            if next.kind == TokenType::Comma {
                if self.tokens[self.pos + 1].kind == TokenType::CloseObject {
                    return self.fail("invalid object", next);
                }
                self.pos += 1;
            } else if next.kind != TokenType::CloseObject {
                return self.fail("invalid object", next);
            }
        }
        self.pos += 1;
        self.depth -= 1;
        Ok(JsonNode::Object(fields))
    }

    /// `json_parse_array` (`json.c:812-856`).
    fn parse_array(&mut self) -> Result<JsonNode, JsonError> {
        self.pos += 1;
        let mut members = Vec::new();
        while self.tok().kind != TokenType::CloseArray {
            let tok = self.tok();
            if tok.kind != TokenType::OpenObject {
                return self.fail("invalid array member", tok);
            }
            members.push(self.parse_object()?);

            let next = self.tok();
            if next.kind == TokenType::Comma {
                if self.tokens[self.pos + 1].kind == TokenType::CloseArray {
                    return self.fail("invalid array", next);
                }
                self.pos += 1;
            } else if next.kind != TokenType::CloseArray {
                return self.fail("invalid array", next);
            }
        }
        self.pos += 1;
        Ok(JsonNode::Array(members))
    }

    /// `json_parse_string` (`json.c:860-887`).
    fn parse_string(&mut self) -> Result<JsonNode, JsonError> {
        let start = self.tok();
        self.pos += 1;
        let value = self.tok();
        if value.kind != TokenType::Value {
            return self.fail("invalid string", start);
        }
        self.pos += 1;
        if self.tok().kind != TokenType::Quote {
            return self.fail("invalid string", start);
        }
        self.pos += 1;
        Ok(JsonNode::String(
            self.input[value.offset..value.offset + value.len].into(),
        ))
    }

    /// `json_parse_number`: `strtoll` base 10 over the whole token, no
    /// excess leading zeros (`json.c:891-914`).
    fn parse_number(&mut self) -> Result<JsonNode, JsonError> {
        let tok = self.tok();
        let bytes = &self.input[tok.offset..tok.offset + tok.len];
        let fail = || error("invalid number", Some(&self.input[tok.offset..]));
        if (bytes[0] == b'0' && bytes.len() != 1)
            || (bytes[0] == b'-' && bytes.get(1) == Some(&b'0') && bytes.len() != 2)
        {
            return Err(fail());
        }
        let digits = if bytes[0] == b'-' { &bytes[1..] } else { bytes };
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err(fail());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| fail())?;
        let num: i64 = text.parse().map_err(|_| fail())?;
        self.pos += 1;
        Ok(JsonNode::Number(num))
    }

    /// `json_parse_boolean`: exactly `true` or `false` (`json.c:918-937`).
    fn parse_boolean(&mut self) -> Result<JsonNode, JsonError> {
        let tok = self.tok();
        let bytes = &self.input[tok.offset..tok.offset + tok.len];
        let value = match bytes {
            b"true" => true,
            b"false" => false,
            _ => return self.fail("invalid boolean", tok),
        };
        self.pos += 1;
        Ok(JsonNode::Boolean(value))
    }
}

/// `json_parse`: parse `input` (up to its first NUL) into a tree
/// (`json.c:152-170,653-684`).
pub fn parse(input: &[u8]) -> Result<JsonNode, JsonError> {
    let input = cstr(input);
    if input.is_empty() {
        return Err(error("empty input", None));
    }
    let tokens = tokenize(input)?;
    let mut parser = Parser {
        input,
        tokens,
        pos: 0,
        depth: 0,
    };
    let first = parser.tok();
    if first.kind != TokenType::OpenObject {
        return parser.fail("expected object", first);
    }
    let node = parser.parse_object()?;
    let trailing = parser.tok();
    if trailing.kind != TokenType::Eof {
        return parser.fail("unexpected trailing data", trailing);
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause(input: &str) -> String {
        match parse(input.as_bytes()) {
            Ok(_) => panic!("parsed {input:?}"),
            Err(e) => String::from_utf8(e.cause.into_vec()).unwrap(),
        }
    }

    fn canonical(input: &str) -> String {
        String::from_utf8(parse(input.as_bytes()).unwrap().to_string().into_vec()).unwrap()
    }

    #[test]
    fn canonical_output_sorts_keys_and_keeps_arrays_and_escapes() {
        assert_eq!(
            canonical(" { \"b\" : [ {\"y\":1}, {\"x\":true} ] , \"a\" : \"q\\\"\\u00e9\" } "),
            "{\"a\":\"q\\\"\\u00e9\",\"b\":[{\"y\":1},{\"x\":true}]}"
        );
        assert_eq!(canonical("{\"n\":-0}"), "{\"n\":0}");
        assert_eq!(canonical("{\"e\":{},\"f\":[]}"), "{\"e\":{},\"f\":[]}");
        assert_eq!(
            canonical("{\"max\":9223372036854775807,\"min\":-9223372036854775808}"),
            "{\"max\":9223372036854775807,\"min\":-9223372036854775808}"
        );
    }

    #[test]
    fn causes_match_source_text() {
        assert_eq!(cause(""), "empty input");
        assert_eq!(cause("   "), "expected object:  ");
        assert_eq!(cause("[]"), "expected object: []");
        assert_eq!(cause("{} x"), "tokenization error: x");
        assert_eq!(
            cause("{\"a\":1}123456789"),
            "tokenization error: 12345678..."
        );
        assert_eq!(cause("{}{}"), "unexpected trailing data: {}");
        assert_eq!(cause("{1:2}"), "tokenization error: 1:2}");
        assert_eq!(cause("{\"a\":\"\\x\"}"), "tokenization error: \\x\"}");
        assert_eq!(
            cause("{\"a\":\"\\u12g4\"}"),
            "tokenization error: \\u12g4\"}"
        );
        assert_eq!(cause("{\"a\":\"\t\"}"), "tokenization error: \t\"}");
        assert_eq!(cause("{a"), "tokenization error: a");
        assert_eq!(cause("{\"a\" 1}"), "tokenization error: 1}");
        assert_eq!(cause("{\"a\":1,\"a\":2}"), "duplicate key: :2}");
        assert_eq!(cause("{\"a\" , 1}"), "tokenization error: 1}");
        assert_eq!(cause("{\"a\":}"), "unexpected value when parsing object: }");
        assert_eq!(cause("{\"a\":1,}"), "invalid object: ,}");
        assert_eq!(cause("{\"a\":1 \"b\":2}"), "invalid object: \"b\":2}");
        assert_eq!(cause("{\"a\":[1]}"), "tokenization error: 1]}");
        assert_eq!(cause("{\"a\":[{},]}"), "invalid array: ,]}");
        assert_eq!(cause("{\"a\":[{} {}]}"), "invalid array: {}]}");
        assert_eq!(cause("{\"a\":\"\"}"), "invalid string: \"\"}");
        assert_eq!(cause("{\"\":1}"), "invalid key: \"\":1}");
        assert_eq!(cause("{\"a\":01}"), "invalid number: 01}");
        assert_eq!(cause("{\"a\":-00}"), "invalid number: -00}");
        assert_eq!(cause("{\"a\":1.5}"), "invalid number: 1.5}");
        assert_eq!(cause("{\"a\":1e5}"), "invalid number: 1e5}");
        assert_eq!(
            cause("{\"a\":9223372036854775808}"),
            "invalid number: 92233720..."
        );
        assert_eq!(cause("{\"a\":+1}"), "invalid boolean: +1}");
        assert_eq!(cause("{\"a\":True}"), "invalid boolean: True}");
        assert_eq!(cause("{\"a\":null}"), "invalid boolean: null}");
        assert_eq!(cause("{\"a\":"), "unexpected value when parsing object: :");
        assert_eq!(cause("{\"a\":1"), "tokenization error: 1");
        assert_eq!(cause("{\"a\":true"), "tokenization error: true");
        assert_eq!(cause("{\"a\":1 "), "invalid object:  ");
        assert_eq!(cause("{"), "invalid key: {");
    }

    #[test]
    fn depth_limit_counts_objects_only() {
        let nest = |n: usize| {
            let mut s = String::new();
            for _ in 0..n {
                s.push_str("{\"a\":");
            }
            s.push_str("{}");
            for _ in 0..n {
                s.push('}');
            }
            s
        };
        assert!(parse(nest(199).as_bytes()).is_ok());
        assert_eq!(cause(&nest(200)), "parse depth exceeded: {}}}}}}}...");
        let mut arrays = String::new();
        for _ in 0..150 {
            arrays.push_str("{\"a\":[");
        }
        arrays.push_str("{}");
        for _ in 0..150 {
            arrays.push_str("]}");
        }
        assert!(parse(arrays.as_bytes()).is_ok());
    }

    #[test]
    fn getters_and_typed_finds() {
        let node = parse(b"{\"s\":\"x\",\"n\":3,\"b\":false,\"o\":{},\"a\":[{}]}").unwrap();
        assert_eq!(node.find_string(b"s").unwrap(), b"x");
        assert_eq!(node.find_number(b"n").unwrap(), 3);
        assert!(!node.find_boolean(b"b").unwrap());
        assert_eq!(
            node.find_object(b"o").unwrap().as_object().unwrap().len(),
            0
        );
        assert_eq!(node.find_array(b"a").unwrap().len(), 1);
        assert_eq!(
            node.find_string(b"zz").unwrap_err().cause(),
            b"key \"zz\" not found"
        );
        assert_eq!(
            node.find_string(b"n").unwrap_err().cause(),
            b"key \"n\" expected a string"
        );
        assert_eq!(
            node.find_number(b"s").unwrap_err().cause(),
            b"key \"s\" expected a number"
        );
        assert_eq!(
            node.find_boolean(b"s").unwrap_err().cause(),
            b"key \"s\" expected a boolean"
        );
        assert_eq!(
            node.find_object(b"s").unwrap_err().cause(),
            b"key \"s\" expected an object"
        );
        assert_eq!(
            node.find_array(b"s").unwrap_err().cause(),
            b"key \"s\" expected an array"
        );
        assert_eq!(node.find(b"s").unwrap().as_number(), None);
        assert_eq!(node.find(b"n").unwrap().as_string(), None);
        assert_eq!(node.find(b"o").unwrap().find(b"x"), None);
        assert_eq!(node.find(b"s").unwrap().find(b"x"), None);
    }

    #[test]
    fn parse_serialize_parse_is_idempotent() {
        let text = "{\"z\":[{\"b\":2,\"a\":\"\\n\"},{}],\"m\":-5,\"t\":true}";
        let once = parse(text.as_bytes()).unwrap();
        let twice = parse(&once.to_string()).unwrap();
        assert_eq!(once, twice);
        assert_eq!(once.to_string(), twice.to_string());
    }
}
