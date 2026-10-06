// Ported from tmux cmd-parse.y, regress/conf @ 8f25579c
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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output};
use std::rc::Rc;

use rmux_util::bytes::ByteString;

use super::grammar::ParsedArgument;
use super::lexer::{Lexer, Token};
use super::*;
use crate::cmd::CommandListPrintFlags;
use crate::cmd::arguments::ArgsValue;
use crate::ids::{ArenaId, ClientId};

#[derive(Default)]
struct Context {
    environment: BTreeMap<Vec<u8>, Vec<u8>>,
    aliases: BTreeMap<Vec<u8>, ByteString>,
    homes: BTreeMap<Vec<u8>, ByteString>,
    writes: Vec<(Vec<u8>, bool)>,
    conditions: Vec<Vec<u8>>,
    output: Vec<Vec<u8>>,
    group: u32,
    oracle: Option<Oracle>,
    inserted: Vec<Rc<CommandList>>,
    appended: Vec<Rc<CommandList>>,
}

impl ParseContext for Context {
    fn environment(&self, name: &[u8]) -> Option<&[u8]> {
        self.environment.get(name).map(Vec::as_slice)
    }
    fn put_environment(&mut self, assignment: &[u8], hidden: bool) {
        let split = assignment
            .iter()
            .position(|byte| *byte == b'=')
            .expect("assignment");
        self.environment.insert(
            assignment[..split].to_vec(),
            assignment[split + 1..].to_vec(),
        );
        self.writes.push((assignment.to_vec(), hidden));
    }
    fn alias(&self, name: &[u8]) -> Option<ByteString> {
        self.aliases.get(name).cloned()
    }
    fn condition(&mut self, format: &[u8], _: &CmdParseInput) -> bool {
        self.conditions.push(format.to_vec());
        if let Some(oracle) = &self.oracle {
            let output = oracle.run(&[b"display-message", b"-p", b"-F", format]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let value = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
            return !value.is_empty() && value != b"0";
        }
        let value = format
            .strip_prefix(b"#{l:")
            .and_then(|value| value.strip_suffix(b"}"))
            .unwrap_or(format);
        !value.is_empty() && value != b"0"
    }
    fn home(&mut self, user: Option<&[u8]>) -> Option<ByteString> {
        self.homes.get(user.unwrap_or_default()).cloned()
    }
    fn next_group(&mut self) -> u32 {
        let group = self.group;
        self.group = self.group.wrapping_add(1);
        group
    }
    fn print(&mut self, message: &[u8], _: &CmdParseInput) {
        self.output.push(message.to_vec());
    }
}

impl ParseQueueContext for Context {
    fn insert_commands(&mut self, list: Rc<CommandList>, _: QueueItemId, _: QueueStateId) {
        self.inserted.push(list);
    }
    fn append_commands(&mut self, list: Rc<CommandList>, _: Option<ClientId>, _: QueueStateId) {
        self.appended.push(list);
    }
}

fn lex(bytes: &[u8], context: &mut Context) -> (Vec<Token>, CmdParseInput, Option<CmdParseError>) {
    rmux_sys::locale::setup_ctype().expect("UTF-8 locale");
    let mut input = CmdParseInput {
        line: 1,
        ..Default::default()
    };
    let mut lexer = Lexer::new(bytes);
    let mut result = Vec::new();
    loop {
        let token = lexer.next(context, &mut input);
        let finished = matches!(token, Token::Error | Token::Eof);
        result.push(token);
        if finished {
            break;
        }
    }
    (result, input, lexer.error)
}

fn word(bytes: &[u8]) -> Token {
    Token::Word(bytes.into())
}

fn printed(bytes: &[u8], context: &mut Context) -> Vec<u8> {
    from_string(context, bytes, &mut CmdParseInput::default())
        .unwrap()
        .print(CommandListPrintFlags::default())
        .0
}

#[test]
fn lexer_acceptance_and_variable_corpus() {
    let mut context = Context::default();
    context
        .environment
        .insert(b"HOME".to_vec(), b"/home/test".to_vec());
    context.environment.insert(b"1".to_vec(), b"digit".to_vec());
    context
        .environment
        .insert(Vec::new(), b"empty-name".to_vec());
    for (input, expected) in [
        (&b"'open"[..], &b"open"[..]),
        (b"\"open", b"open"),
        (b"a#b", b"a#b"),
        (b"a{b", b"a{b"),
        (b"${1}", b"digit"),
        (b"${}", b"empty-name"),
        (b"$9", b"$9"),
        (b"$!", b"$!"),
        (b"'$HOME'", b"$HOME"),
        (b"a\"~\"", b"a/home/test"),
        (b"a~", b"a~"),
        (b"\\000suffix", b""),
        (b"\\377", &[255]),
        (
            b"\\a\\b\\e\\f\\s\\v\\r\\n\\t",
            &[7, 8, 27, 12, 32, 11, 13, 10, 9],
        ),
    ] {
        assert_eq!(
            lex(input, &mut context).0,
            vec![word(expected), Token::Newline, Token::Eof],
            "input {input:?}"
        );
    }
    assert_eq!(lex(b"$", &mut context).0, vec![Token::Error]);
    assert_eq!(
        lex(b"${bad-name}", &mut context).2.unwrap().message(),
        b"invalid environment variable"
    );
    assert_eq!(
        lex(b"A=x 1=x _=x a-b=x", &mut context).0,
        vec![
            Token::Equals(b"A=x".as_slice().into()),
            word(b"1=x"),
            Token::Equals(b"_=x".as_slice().into()),
            word(b"a-b=x"),
            Token::Newline,
            Token::Eof
        ]
    );
    let name = vec![b'A'; 1022];
    context.environment.insert(name.clone(), b"long".to_vec());
    let mut expansion = vec![b'$'];
    expansion.extend_from_slice(&name);
    assert_eq!(lex(&expansion, &mut context).0[0], word(b"long"));
    expansion.push(b'A');
    assert_eq!(
        lex(&expansion, &mut context).2.unwrap().message(),
        b"environment variable is too long"
    );
}

#[test]
fn lexer_directive_latch_and_format_nesting() {
    let mut context = Context::default();
    for (directive, token) in [
        (b"%if".as_slice(), Token::If),
        (b"%elif", Token::Elif),
        (b"%else", Token::Else),
        (b"%endif", Token::Endif),
        (b"%hidden", Token::Hidden),
    ] {
        let mut bytes = directive.to_vec();
        bytes.extend_from_slice(b" #{a:#{b}} word #{ignored}\n");
        assert_eq!(
            lex(&bytes, &mut context).0,
            vec![
                token,
                Token::Format(b"#{a:#{b}}".as_slice().into()),
                word(b"word"),
                Token::Newline,
                Token::Newline,
                Token::Eof
            ]
        );
    }
    assert_eq!(
        lex(b"%% %1 %123%", &mut context).0,
        vec![
            word(b"%%"),
            word(b"%1"),
            word(b"%123%"),
            Token::Newline,
            Token::Eof
        ]
    );
    assert_eq!(lex(b"%unknown", &mut context).0, vec![Token::Error]);
    assert_eq!(
        lex(b"%if #{unclosed\n", &mut context).0,
        vec![Token::If, Token::Error]
    );
    assert_eq!(
        lex(b"%if\n#{not-format}", &mut context).0,
        vec![Token::If, Token::Newline, Token::Newline, Token::Eof]
    );
}

#[test]
fn lexer_lines_comments_quotes_and_backslash_runs() {
    let mut context = Context::default();
    let (tokens, input, _) = lex(b"a\r\nb # comment\nc\\\nd", &mut context);
    assert_eq!(
        tokens,
        vec![
            word(b"a"),
            Token::Newline,
            word(b"b"),
            Token::Newline,
            word(b"cd"),
            Token::Newline,
            Token::Eof
        ]
    );
    assert_eq!(input.line, 4);
    for quote in *b"'\"" {
        for run in 1..=6 {
            let mut bytes = vec![quote, b'a'];
            bytes.extend(std::iter::repeat_n(b'\\', run));
            bytes.extend_from_slice(b"\nb");
            bytes.push(quote);
            let mut expected = vec![b'a'];
            let retained = run - run % 2;
            expected.extend(std::iter::repeat_n(
                b'\\',
                if quote == b'\'' {
                    retained
                } else {
                    retained / 2
                },
            ));
            if run % 2 == 0 {
                expected.push(b'\n');
            }
            expected.push(b'b');
            assert_eq!(
                lex(&bytes, &mut context).0[0],
                word(&expected),
                "quote {quote}, run {run}"
            );
        }
    }
    assert_eq!(
        lex(b"\"a\n  # removed\n  b\"", &mut context).0[0],
        word(b"a\n\nb")
    );
    for prefix in *b",#{}:" {
        let mut bytes = b"\"a\n  #".to_vec();
        bytes.push(prefix);
        bytes.extend_from_slice(b"b\"");
        let mut expected = b"a\n#".to_vec();
        expected.push(prefix);
        expected.push(b'b');
        assert_eq!(lex(&bytes, &mut context).0[0], word(&expected));
    }
    let bytes = b"a\\\\\\\nb";
    let buffered = lex(bytes, &mut context).0;
    let mut reader = &bytes[..];
    let mut lexer = Lexer::from_reader(&mut reader);
    let mut input = CmdParseInput {
        line: 1,
        ..Default::default()
    };
    for expected in buffered {
        assert_eq!(lexer.next(&mut context, &mut input), expected);
    }
}

#[test]
fn lexer_escape_errors_and_home_fallback() {
    let mut context = Context::default();
    assert_eq!(
        lex(b"\\u12ab\\U000012ab", &mut context).0[0],
        word("ካካ".as_bytes())
    );
    for bytes in [b"\\4".as_slice(), b"\\7", b"\\0", b"\\12", b"\\08x"] {
        assert_eq!(
            lex(bytes, &mut context).2.unwrap().message(),
            b"invalid octal escape"
        );
    }
    for (bytes, error) in [
        (b"\\u12xz".as_slice(), b"invalid \\u argument".as_slice()),
        (b"\\uD800", b"invalid \\u argument"),
    ] {
        assert_eq!(lex(bytes, &mut context).2.unwrap().message(), error);
    }
    // utf8_fromwc: utf8proc rejects U+110000; glibc wctomb encodes it, and a
    // libc build gives a negative wcwidth width 1, so tmux there accepts it.
    let above_unicode = lex(b"\\U00110000", &mut context).2;
    if cfg!(target_os = "macos") {
        assert_eq!(above_unicode.unwrap().message(), b"invalid \\U argument");
    } else {
        assert!(above_unicode.is_none());
    }
    assert!(lex(b"\\u12", &mut context).2.is_none());
    context
        .homes
        .insert(Vec::new(), b"/fallback".as_slice().into());
    context
        .homes
        .insert(b"alice".to_vec(), b"/alice".as_slice().into());
    assert_eq!(
        lex(b"~ ~alice/path '~'", &mut context).0,
        vec![
            word(b"/fallback"),
            word(b"/alice/path"),
            word(b"~"),
            Token::Newline,
            Token::Eof
        ]
    );
    context.environment.insert(b"HOME".to_vec(), Vec::new());
    assert_eq!(lex(b"~", &mut context).0[0], word(b"/fallback"));
    assert_eq!(lex(b"~missing", &mut context).0, vec![Token::Error]);
    let mut username = vec![b'~'];
    username.extend(std::iter::repeat_n(b'a', 1023));
    assert_eq!(
        lex(&username, &mut context).2.unwrap().message(),
        b"user name is too long"
    );
}

#[test]
fn lexer_nul_ends_strchr_scans_like_c() {
    let mut context = Context::default();
    context
        .homes
        .insert(Vec::new(), b"/fallback".as_slice().into());
    // yylex_get_word: strchr(" \t\n", 0) matches, so the word is "%if" and the NUL starts a token.
    assert_eq!(
        lex(b"%if\0x 1", &mut context).0,
        vec![Token::If, word(b""), word(b"1"), Token::Newline, Token::Eof]
    );
    // yylex_token_tilde: the NUL ends the user name and then truncates the token.
    assert_eq!(
        lex(b"~\0junk next", &mut context).0,
        vec![
            word(b"/fallback"),
            word(b"next"),
            Token::Newline,
            Token::Eof
        ]
    );
    // Inside quotes after a newline, "#\0" keeps the # and the token continues to the quote.
    assert_eq!(
        lex(b"\"a\n#\0 b\" c", &mut context).0,
        vec![word(b"a\n#"), word(b"c"), Token::Newline, Token::Eof]
    );
    assert_eq!(
        lex(b"\"a\n#x b\" c", &mut context).0,
        vec![word(b"a\n"), Token::Newline, Token::Eof]
    );
}

#[test]
fn grammar_condition_forms_nested_branches_and_side_effects() {
    for multiline in [false, true] {
        for first in [false, true] {
            for middle in [false, true] {
                for last in [false, true] {
                    let mut context = Context::default();
                    let source = if multiline {
                        format!(
                            "%if {}\nA=first display first\n%elif {}\nB=middle display middle\n%elif {}\nC=last display last\n%else\nD=else display else\n%endif",
                            u8::from(first),
                            u8::from(middle),
                            u8::from(last)
                        )
                    } else {
                        format!(
                            "%if {} A=first display first %elif {} B=middle display middle %elif {} C=last display last %else D=else display else %endif",
                            u8::from(first),
                            u8::from(middle),
                            u8::from(last)
                        )
                    };
                    let selected = if first {
                        "first"
                    } else if middle {
                        "middle"
                    } else if last {
                        "last"
                    } else {
                        "else"
                    };
                    assert_eq!(
                        printed(source.as_bytes(), &mut context),
                        format!("display-message {selected}").as_bytes()
                    );
                    assert_eq!(context.environment.contains_key(b"A".as_slice()), first);
                    assert_eq!(context.environment.contains_key(b"B".as_slice()), middle);
                    assert_eq!(context.environment.contains_key(b"C".as_slice()), last);
                    assert_eq!(context.environment.contains_key(b"D".as_slice()), !last);
                    assert_eq!(context.conditions.len(), 3);
                }
            }
        }
    }
    let mut context = Context::default();
    assert_eq!(
        printed(
            b"%if 0\n%if 1\nX=no display hidden\n%endif\n%else\n%if 1 display shown %endif\n%endif",
            &mut context
        ),
        b"display-message shown"
    );
    assert!(!context.environment.contains_key(b"X".as_slice()));
    assert_eq!(
        printed(
            b"bind x { %if 1 display a %else display b %endif\n display c }",
            &mut context
        ),
        b"bind-key x { display-message a ; display-message c }"
    );
}

#[test]
fn grammar_assignment_clearing_limits_failures_and_parseonly() {
    let mut context = Context::default();
    assert!(printed(b"display first ; A=x", &mut context).is_empty());
    assert_eq!(context.environment.get(b"A".as_slice()).unwrap(), b"x");
    assert_eq!(
        printed(b"display first ; A=x ; display last", &mut context),
        b"display-message last"
    );
    let mut maximum = b"B=".to_vec();
    maximum.extend(std::iter::repeat_n(b'x', 16382));
    assert!(from_buffer(&mut context, &maximum, &mut CmdParseInput::default()).is_ok());
    maximum.push(b'x');
    assert_eq!(
        from_buffer(&mut context, &maximum, &mut CmdParseInput::default())
            .unwrap_err()
            .message(),
        b"environment variable is too long"
    );
    assert!(
        from_buffer(
            &mut context,
            b"KEPT=yes\n%bogus",
            &mut CmdParseInput::default()
        )
        .is_err()
    );
    assert_eq!(context.environment.get(b"KEPT".as_slice()).unwrap(), b"yes");
    assert!(
        from_buffer(
            &mut context,
            b"BUILD=yes\nunknown-command",
            &mut CmdParseInput::default()
        )
        .is_err()
    );
    assert_eq!(
        context.environment.get(b"BUILD".as_slice()).unwrap(),
        b"yes"
    );
    let mut input = CmdParseInput {
        flags: CmdParseFlags::PARSEONLY,
        ..Default::default()
    };
    assert!(from_buffer(&mut context, b"%if 1\nNO=x display ok\n%endif", &mut input).is_ok());
    assert!(!context.environment.contains_key(b"NO".as_slice()));
    assert!(!context.conditions.is_empty());
    printed(b"%hidden SECRET=hidden", &mut context);
    assert_eq!(
        context.writes.last().unwrap(),
        &(b"SECRET=hidden".to_vec(), true)
    );
    for bad in [
        b"%if 1\n%endif".as_slice(),
        b"; display a",
        b"%if 1 display a\n%endif",
        b"display {",
        b"%hidden x",
        b"A=x B=y display a",
        b"%else",
    ] {
        assert!(
            from_buffer(&mut context, bad, &mut CmdParseInput::default()).is_err(),
            "accepted {bad:?}"
        );
    }
}

#[test]
fn build_groups_aliases_argv_sources_and_verbose() {
    let mut context = Context::default();
    let mut input = CmdParseInput {
        line: 1,
        file: Some(b"test.conf".as_slice().into()),
        flags: CmdParseFlags::VERBOSE,
        item: Some(QueueItemId::from_parts(0, 0)),
        ..Default::default()
    };
    let list = from_buffer(&mut context, b"display a; display b\ndisplay c", &mut input).unwrap();
    assert_eq!(list.commands[0].group, list.commands[1].group);
    assert_ne!(list.commands[1].group, list.commands[2].group);
    assert_eq!(
        context.output,
        vec![
            b"test.conf:1: display-message a ; display-message b".to_vec(),
            b"test.conf:2: display-message c".to_vec()
        ]
    );
    assert_eq!(list.commands[2].line, 2);
    assert_eq!(
        list.commands[2].file.as_ref().unwrap().as_ref(),
        b"test.conf"
    );
    let unified = from_string(
        &mut context,
        b"display a\ndisplay b",
        &mut CmdParseInput::default(),
    )
    .unwrap();
    assert_eq!(unified.commands[0].group, unified.commands[1].group);
    context.aliases.insert(
        b"custom".to_vec(),
        b"display first; display".as_slice().into(),
    );
    assert_eq!(
        printed(b"custom extra", &mut context),
        b"display-message first ; display-message extra"
    );
    context
        .aliases
        .insert(b"custom".to_vec(), b"ALIAS=x display".as_slice().into());
    assert_eq!(
        printed(b"custom extra", &mut context),
        b"display-message extra"
    );
    assert_eq!(context.environment.get(b"ALIAS".as_slice()).unwrap(), b"x");
    context
        .aliases
        .insert(b"custom".to_vec(), b"custom".as_slice().into());
    assert_eq!(printed(b"custom", &mut context), b"customize-mode");
    context.aliases.insert(
        b"parser-alias-only".to_vec(),
        b"parser-alias-only".as_slice().into(),
    );
    assert_eq!(
        from_string(
            &mut context,
            b"parser-alias-only",
            &mut CmdParseInput::default()
        )
        .unwrap_err()
        .message(),
        b"unknown command: parser-alias-only"
    );
    let mut noalias = CmdParseInput {
        flags: CmdParseFlags::NOALIAS,
        ..Default::default()
    };
    assert_eq!(
        from_buffer(&mut context, b"custom", &mut noalias)
            .unwrap()
            .print(CommandListPrintFlags::default())
            .as_ref(),
        b"customize-mode"
    );
    assert!(from_buffer(&mut context, b"parser-alias-only", &mut noalias).is_err());
    context
        .aliases
        .insert(b"empty".to_vec(), b"A=empty".as_slice().into());
    assert!(printed(b"empty ignored", &mut context).is_empty());
    for values in [
        vec![";", "display", "a"],
        vec![";", ";", "display", "a", ";"],
    ] {
        let args: Vec<_> = values
            .iter()
            .map(|value| ArgsValue::string((*value).into()))
            .collect();
        assert_eq!(
            from_arguments(&mut context, &args, &mut CmdParseInput::default())
                .unwrap()
                .print(CommandListPrintFlags::default())
                .as_ref(),
            b"display-message a"
        );
        assert_eq!(
            from_arguments(&mut context, &args, &mut noalias)
                .unwrap_err()
                .message(),
            b"no command"
        );
    }
    for text in [b"\\;".as_slice(), b"\\\\;", b"a;b"] {
        let values = [
            ArgsValue::string(b"display".as_slice().into()),
            ArgsValue::string(text.into()),
        ];
        let list = from_arguments(&mut context, &values, &mut CmdParseInput::default()).unwrap();
        let expected = if text == b"\\;" {
            b";".as_slice()
        } else if text == b"\\\\;" {
            b"\\;"
        } else {
            text
        };
        assert_eq!(list.commands[0].args.values[0].as_string(), expected);
    }
    let trailing = [
        ArgsValue::string(b"display".as_slice().into()),
        ArgsValue::string(b"a;".as_slice().into()),
    ];
    assert_eq!(
        from_arguments(&mut context, &trailing, &mut CmdParseInput::default())
            .unwrap()
            .commands
            .len(),
        1
    );
    let mut counter = 1000;
    let copied = list.copy(&[], &mut counter);
    assert!(
        copied
            .commands
            .iter()
            .all(|command| command.parse_flags == CmdParseFlags::default())
    );
    assert_eq!(copied.commands[0].group, copied.commands[1].group);
    assert_eq!(copied.commands[0].file, list.commands[0].file);
    assert_eq!(copied.commands[0].line, list.commands[0].line);
    assert_ne!(copied.commands[0].group, list.commands[0].group);
    assert_ne!(copied.commands[0].group, copied.commands[2].group);
    let commands = ArgsValue::commands(unified);
    let arguments = [
        ArgsValue::string(b"bind-key".as_slice().into()),
        ArgsValue::string(b"x".as_slice().into()),
        commands,
    ];
    assert!(from_arguments(&mut context, &arguments, &mut CmdParseInput::default()).is_ok());
}

#[test]
fn entrypoints_empty_errors_and_queue_sinks() {
    let mut context = Context::default();
    assert!(
        from_buffer(&mut context, b"", &mut CmdParseInput::default())
            .unwrap()
            .commands
            .is_empty()
    );
    assert!(
        from_arguments(&mut context, &[], &mut CmdParseInput::default())
            .unwrap()
            .commands
            .is_empty()
    );
    let mut bytes = b"display file".as_slice();
    assert_eq!(
        from_file(&mut context, &mut bytes, &mut CmdParseInput::default())
            .unwrap()
            .commands
            .len(),
        1
    );
    assert_eq!(
        printed(b"display before\0invalid", &mut context),
        b"display-message before"
    );
    let mut input = CmdParseInput {
        file: Some(b"error.conf".as_slice().into()),
        line: 9,
        ..Default::default()
    };
    assert_eq!(
        from_buffer(&mut context, b"display \\4", &mut input)
            .unwrap_err()
            .message(),
        b"error.conf:9: invalid octal escape"
    );
    assert!(
        from_string(&mut context, b"display -?", &mut CmdParseInput::default())
            .unwrap_err()
            .message()
            .starts_with(b"usage: display-message ")
    );
    assert!(
        from_string(&mut context, b"display -Z", &mut CmdParseInput::default())
            .unwrap_err()
            .message()
            .starts_with(b"command display-message: ")
    );
    and_insert(
        &mut context,
        b"display inserted",
        &mut CmdParseInput::default(),
        QueueItemId::from_parts(0, 0),
        QueueStateId::from_parts(0, 0),
    )
    .unwrap();
    and_append(
        &mut context,
        b"display appended",
        &mut CmdParseInput::default(),
        None,
        QueueStateId::from_parts(0, 0),
    )
    .unwrap();
    assert_eq!(context.inserted.len(), 1);
    assert_eq!(context.appended.len(), 1);
    assert!(
        and_insert(
            &mut context,
            b"unknown",
            &mut CmdParseInput::default(),
            QueueItemId::from_parts(0, 0),
            QueueStateId::from_parts(0, 0)
        )
        .is_err()
    );
    assert_eq!(context.inserted.len(), 1);
}

#[test]
fn arbitrary_bytes_parseonly_never_panics() {
    let mut context = Context::default();
    let mut random = 0x5deece66du64;
    for length in 0..512 {
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            bytes.push(random as u8);
        }
        let _ = from_buffer(
            &mut context,
            &bytes,
            &mut CmdParseInput {
                flags: CmdParseFlags::PARSEONLY,
                ..Default::default()
            },
        );
    }
}

#[test]
#[ignore = "ten-minute parser fuzz acceptance; run explicitly after integration"]
fn parser_fuzz_ten_minutes_parseonly_oracle_status() {
    rmux_sys::locale::setup_ctype().expect("UTF-8 locale");
    let mut context = Context {
        oracle: Oracle::new(),
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let seconds = std::env::var("RMUX_CMD_FUZZ_SECONDS")
        .map(|value| value.parse::<u64>().expect("numeric fuzz duration"))
        .unwrap_or(600);
    let mut random = std::env::var("RMUX_CMD_FUZZ_SEED")
        .map(|value| value.parse::<u64>().expect("numeric fuzz seed"))
        .unwrap_or(0x9e3779b97f4a7c15);
    let mut cases = 0u64;
    while started.elapsed() < std::time::Duration::from_secs(seconds) {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let length = (random & 1023) as usize;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            bytes.push(random as u8);
        }
        let mut input = CmdParseInput {
            flags: CmdParseFlags::PARSEONLY,
            ..Default::default()
        };
        let _ = from_buffer(&mut context, &bytes, &mut input);
        let mut reader = bytes.as_slice();
        let _ = from_file(&mut context, &mut reader, &mut input);
        if let Some(oracle) = &context.oracle {
            let path = oracle.directory.join("fuzz.conf");
            std::fs::write(&path, &bytes).unwrap();
            use std::os::unix::ffi::OsStrExt;
            let file = path.as_os_str().as_bytes();
            let output = oracle.run(&[b"source-file", b"-n", file]);
            let mut input = CmdParseInput {
                file: Some(file.into()),
                line: 1,
                flags: CmdParseFlags::PARSEONLY,
                ..Default::default()
            };
            let parsed = from_buffer(&mut context, &bytes, &mut input);
            assert_eq!(
                parsed.is_ok(),
                output.status.success(),
                "fuzz case {cases}, bytes {bytes:?}: Rust {parsed:?}, oracle {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        cases = cases.wrapping_add(1);
    }
    eprintln!("parser ten-minute fuzz acceptance: {cases} cases");
}

struct Oracle {
    binary: PathBuf,
    directory: PathBuf,
    socket: PathBuf,
}

impl Oracle {
    fn new() -> Option<Self> {
        let binary = std::env::var_os("RMUX_ORACLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux"));
        if !binary.is_file() {
            eprintln!(
                "parser differential skipped: build scripts/build-oracle.sh or set RMUX_ORACLE"
            );
            return None;
        }
        let directory =
            std::env::temp_dir().join(format!("rmux-g11-parser-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("socket");
        let oracle = Self {
            binary,
            directory,
            socket,
        };
        let result = oracle.run(&[b"new-session", b"-d", b"-s", b"parser", b"sleep 900"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Some(oracle)
    }
    fn run(&self, arguments: &[&[u8]]) -> Output {
        use std::os::unix::ffi::OsStrExt;
        let mut command = ProcessCommand::new(&self.binary);
        command
            .env_clear()
            .env("HOME", "/tmp/rmux-parser-home")
            .env("USER", "parser")
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm")
            .env("PATH", "/usr/bin:/bin")
            .env("LC_ALL", "en_US.UTF-8");
        command
            .arg("-S")
            .arg(&self.socket)
            .arg("-f")
            .arg("/dev/null");
        for argument in arguments {
            command.arg(std::ffi::OsStr::from_bytes(argument));
        }
        command.output().unwrap()
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = ProcessCommand::new(&self.binary)
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server")
            .output();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn all_21_pinned_configs_differential_source_file_n_v() {
    rmux_sys::locale::setup_ctype().expect("UTF-8 locale");
    let Some(oracle) = Oracle::new() else {
        return;
    };
    let mut context = Context {
        oracle: Some(oracle),
        ..Default::default()
    };
    for (name, value) in [
        (b"HOME".as_slice(), b"/tmp/rmux-parser-home".as_slice()),
        (b"USER", b"parser"),
        (b"SHELL", b"/bin/sh"),
        (b"TERM", b"xterm"),
        (b"PATH", b"/usr/bin:/bin"),
        (b"LC_ALL", b"en_US.UTF-8"),
    ] {
        context.environment.insert(name.to_vec(), value.to_vec());
    }
    for (name, value) in [
        (b"split-pane".as_slice(), b"split-window".as_slice()),
        (b"splitp", b"split-window"),
        (b"server-info", b"show-messages -JT"),
        (b"info", b"show-messages -JT"),
        (b"choose-window", b"choose-tree -w"),
        (b"choose-session", b"choose-tree -s"),
    ] {
        context.aliases.insert(name.to_vec(), value.into());
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cmd/parse/conf");
    let mut paths: Vec<_> = std::fs::read_dir(fixtures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "conf")
        })
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 21);
    for path in paths {
        use std::os::unix::ffi::OsStrExt;
        context.output.clear();
        let source = std::fs::read(&path).unwrap();
        let file = path.as_os_str().as_bytes();
        let mut input = CmdParseInput {
            file: Some(file.into()),
            line: 1,
            flags: CmdParseFlags::PARSEONLY | CmdParseFlags::VERBOSE,
            item: Some(QueueItemId::from_parts(0, 0)),
            ..Default::default()
        };
        let result = from_buffer(&mut context, &source, &mut input);
        let oracle = context
            .oracle
            .as_ref()
            .unwrap()
            .run(&[b"source-file", b"-n", b"-v", file]);
        let mut actual = Vec::new();
        for line in &context.output {
            actual.extend_from_slice(line);
            actual.push(b'\n');
        }
        if let Err(error) = &result {
            actual.extend_from_slice(error.message());
            actual.push(b'\n');
        }
        assert!(
            oracle.stderr.is_empty(),
            "unexpected stderr {}: {}",
            path.display(),
            String::from_utf8_lossy(&oracle.stderr)
        );
        assert_eq!(actual, oracle.stdout, "verbose mismatch {}", path.display());
        assert_eq!(
            result.is_ok(),
            oracle.status.success(),
            "status mismatch {}: Rust {result:?}, oracle {}",
            path.display(),
            String::from_utf8_lossy(&oracle.stderr)
        );
    }
    let corpus: &[&[u8]] = &[
        b"display 'open",
        b"display \"open",
        b"display a#b a{b",
        b"display $",
        b"display ${} ${1}",
        b"display \\4",
        b"display \\u12xz",
        b"display \\uD800",
        b"display \\U00110000",
        b"display first ; A=x",
        b"A=x display a ; B=y ; display c",
        b"A=x B=y display a",
        b"%hidden A=x\ndisplay done",
        b"%if 1 display yes %elif 1 display later %else display no %endif",
        b"%if 0\ndisplay no\n%elif 1\n%if 0 display no %else display yes %endif\n%endif",
        b"%if 1\n%endif",
        b"%if 1 display a\n%endif",
        b"display a ; %if 1 display b %endif ; display c",
        b"bind x { display a\n display b }",
        b"bind x {\n\n}",
        b"display {",
        b"display \"a\n  # comment\n b\"",
        b"display \\\\\nnext",
        b"display \\\\nnext",
        b"display a\r\ndisplay b",
        b"; display a",
        b"display a ; ;",
        b"%unknown",
        b"%if #{l:1} display format %endif",
        b"display -?",
        b"display -Z",
        b"unknown-command",
        b"%if\0x 1\ndisplay yes\n%endif",
        b"display ~\0junk next",
        b"display \"a\n#\0 b\" c",
        b"display \"a\n#x b\" c",
        b"display a\0b c",
    ];
    let regression = seed1_case305();
    let mut corpus = corpus.to_vec();
    corpus.push(&regression);
    for (index, source) in corpus.iter().enumerate() {
        use std::os::unix::ffi::OsStrExt;
        let path = context
            .oracle
            .as_ref()
            .unwrap()
            .directory
            .join(format!("edge-{index}.conf"));
        std::fs::write(&path, source).unwrap();
        let file = path.as_os_str().as_bytes();
        context.output.clear();
        let mut input = CmdParseInput {
            file: Some(file.into()),
            line: 1,
            flags: CmdParseFlags::PARSEONLY | CmdParseFlags::VERBOSE,
            item: Some(QueueItemId::from_parts(0, 0)),
            ..Default::default()
        };
        let result = from_buffer(&mut context, source, &mut input);
        let oracle = context
            .oracle
            .as_ref()
            .unwrap()
            .run(&[b"source-file", b"-n", b"-v", file]);
        let mut actual = Vec::new();
        for line in &context.output {
            actual.extend_from_slice(line);
            actual.push(b'\n');
        }
        if let Err(error) = &result {
            actual.extend_from_slice(error.message());
            actual.push(b'\n');
        }
        assert!(
            oracle.stderr.is_empty(),
            "unexpected stderr edge {index}: {}",
            String::from_utf8_lossy(&oracle.stderr)
        );
        assert_eq!(actual, oracle.stdout, "verbose edge {index}: {source:?}");
        assert_eq!(
            result.is_ok(),
            oracle.status.success(),
            "status edge {index}: Rust {result:?}, oracle {}",
            String::from_utf8_lossy(&oracle.stderr)
        );
    }
}

#[test]
fn grammar_tree_brace_empty_and_final_line() {
    let mut context = Context::default();
    let commands = grammar::parse(
        Lexer::new(b"bind x {\n\n}\ndisplay done"),
        &mut context,
        &mut CmdParseInput::default(),
    )
    .unwrap();
    assert_eq!(commands.len(), 2);
    assert!(
        matches!(&commands[0].arguments[2],ParsedArgument::Commands(commands) if commands.is_empty())
    );
}

fn seed1_case305() -> Vec<u8> {
    let hex = include_str!("seed1-case305.hex").trim();
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16).expect("hex fixture");
            let low = char::from(pair[1]).to_digit(16).expect("hex fixture");
            ((high << 4) | low) as u8
        })
        .collect()
}

#[test]
fn fuzz_regression_signed_buffer_eof_differs_from_file_bytes() {
    rmux_sys::locale::setup_ctype().expect("UTF-8 locale");
    let bytes = seed1_case305();
    assert_eq!(bytes.len(), 296);
    assert_eq!(bytes[70], 0xff);
    let mut context = Context::default();
    let mut input = CmdParseInput {
        flags: CmdParseFlags::PARSEONLY,
        ..Default::default()
    };
    assert!(from_buffer(&mut context, &bytes, &mut input).is_err());
    let mut reader = bytes.as_slice();
    let mut input = CmdParseInput {
        flags: CmdParseFlags::PARSEONLY,
        ..Default::default()
    };
    // The file bytes start an `\xdaO=G...` word. yylex_is_var asks isalpha:
    // macOS's UTF-8 ctype calls 0xda alphabetic (an assignment, no command);
    // glibc's does not, so tmux there parses an unknown command.
    let parsed = from_file(&mut context, &mut reader, &mut input);
    if cfg!(target_os = "macos") {
        assert!(parsed.unwrap().commands.is_empty());
    } else {
        assert!(parsed.is_err());
    }
}
