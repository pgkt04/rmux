// Ported from tmux arguments.c, tmux.h @ 8f25579c
/*
 * Copyright (c) 2010 Nicholas Marriott <nicholas.marriott@gmail.com>
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
pub enum ArgsType {
    Commands = 2,
    None = 0,
    String = 1,
}
impl TryFrom<i32> for ArgsType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            2 => Ok(Self::Commands),
            0 => Ok(Self::None),
            1 => Ok(Self::String),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArgsParseType {
    Commands = 3,
    Invalid = 0,
    String = 1,
    CommandsOrString = 2,
}
impl TryFrom<i32> for ArgsParseType {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            3 => Ok(Self::Commands),
            0 => Ok(Self::Invalid),
            1 => Ok(Self::String),
            2 => Ok(Self::CommandsOrString),
            _ => Err(value),
        }
    }
}

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use rmux_util::bytes::{ByteString, cstr};
use rmux_util::strtonum::{StrtonumError, strtonum};
use rmux_util::vis::VisFlags;

use super::find::CmdFindState;
use super::parse::{self, CmdParseInput, CmdParseResult, ParseContext};
use super::{Command, CommandList, CommandListPrintFlags, template_replace};
use crate::ids::{ClientId, QueueItemId};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ArgsEntryFlags(pub u32);

impl ArgsEntryFlags {
    pub const OPTIONAL_VALUE: Self = Self(1);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Clone)]
pub enum ArgsValueData {
    None,
    String(ByteString),
    Commands(Rc<CommandList>),
}

pub struct ArgsValue {
    pub data: ArgsValueData,
    pub cached: OnceCell<ByteString>,
}

impl Clone for ArgsValue {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            cached: OnceCell::new(),
        }
    }
}

impl fmt::Debug for ArgsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArgsValue")
            .field("kind", &self.kind())
            .field("value", &self.as_string())
            .finish()
    }
}

impl ArgsValue {
    pub fn none() -> Self {
        Self {
            data: ArgsValueData::None,
            cached: OnceCell::new(),
        }
    }

    pub fn string(value: ByteString) -> Self {
        Self {
            data: ArgsValueData::String(value),
            cached: OnceCell::new(),
        }
    }

    pub fn commands(value: Rc<CommandList>) -> Self {
        Self {
            data: ArgsValueData::Commands(value),
            cached: OnceCell::new(),
        }
    }

    pub fn kind(&self) -> ArgsType {
        match self.data {
            ArgsValueData::None => ArgsType::None,
            ArgsValueData::String(_) => ArgsType::String,
            ArgsValueData::Commands(_) => ArgsType::Commands,
        }
    }

    pub fn as_string(&self) -> &[u8] {
        match &self.data {
            ArgsValueData::None => b"",
            ArgsValueData::String(value) => value.cstr(),
            ArgsValueData::Commands(list) => self
                .cached
                .get_or_init(|| list.print(CommandListPrintFlags::default()))
                .as_bytes(),
        }
    }

    fn copy(&self, argv: &[ByteString], next_group: &mut u32) -> Self {
        match &self.data {
            ArgsValueData::None => Self::none(),
            ArgsValueData::String(value) => Self::string(expand_template(value, argv)),
            ArgsValueData::Commands(list) => Self::commands(Rc::new(list.copy(argv, next_group))),
        }
    }
}

pub type ArgsTree = BTreeMap<u8, ArgsEntry>;
pub type ArgsParseCallback = fn(&Args, u32) -> Result<ArgsParseType, ByteString>;

#[derive(Clone, Copy, Debug)]
pub struct ArgsParse {
    pub template: &'static [u8],
    pub lower: i32,
    pub upper: i32,
    pub cb: Option<ArgsParseCallback>,
}

impl ArgsParse {
    pub fn parse(&self, values: &[ArgsValue]) -> Result<Option<Args>, ByteString> {
        parse(self, values)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ArgsEntry {
    pub values: Vec<ArgsValue>,
    pub count: u32,
    pub flags: ArgsEntryFlags,
}

#[derive(Clone, Debug, Default)]
pub struct Args {
    pub flags: BTreeMap<u8, ArgsEntry>,
    pub values: Vec<ArgsValue>,
}

pub fn parse(spec: &ArgsParse, values: &[ArgsValue]) -> Result<Option<Args>, ByteString> {
    let mut args = Args::create();
    if values.is_empty() {
        return Ok(Some(args));
    }
    let mut i = 1;
    while i < values.len() {
        let ArgsValueData::String(value) = &values[i].data else {
            break;
        };
        let string = value.cstr();
        if string.first() != Some(&b'-') || string.len() == 1 {
            break;
        }
        i += 1;
        if string == b"--" {
            break;
        }
        let mut j = 1;
        while j < string.len() {
            let flag = string[j];
            j += 1;
            if flag == b'?' {
                return Ok(None);
            }
            if !rmux_sys::locale::is_alnum(flag) {
                return Err(flag_error(b"invalid flag -", flag, b""));
            }
            let Some(found) = cstr(spec.template).iter().position(|&c| c == flag) else {
                return Err(flag_error(b"unknown flag -", flag, b""));
            };
            if spec.template.get(found + 1) != Some(&b':') {
                args.set(flag, None, ArgsEntryFlags::default());
                continue;
            }
            let optional = spec.template.get(found + 2) == Some(&b':');
            if j < string.len() {
                args.set(
                    flag,
                    Some(ArgsValue::string(ByteString::from(&string[j..]))),
                    ArgsEntryFlags::default(),
                );
                break;
            }
            let argument = values.get(i);
            let next = match argument {
                Some(ArgsValue {
                    data: ArgsValueData::String(value),
                    ..
                }) => Some(value.cstr()),
                Some(_) => return Err(flag_error(b"-", flag, b" argument must be a string")),
                None => None,
            };
            let next_is_flag = next.is_some_and(|s| {
                s.first() == Some(&b'-')
                    && s.get(1)
                        .is_some_and(|c| *c == b'-' || rmux_sys::locale::is_alpha(*c))
            });
            if optional && (next.is_none() || next_is_flag) {
                args.set(flag, None, ArgsEntryFlags::OPTIONAL_VALUE);
            } else if let Some(argument) = argument {
                args.set(flag, Some(argument.clone()), ArgsEntryFlags::default());
                i += 1;
            } else {
                return Err(flag_error(b"-", flag, b" expects an argument"));
            }
            break;
        }
    }
    for value in &values[i..] {
        let kind = match spec.cb {
            Some(cb) => cb(&args, args.count())?,
            None => ArgsParseType::String,
        };
        let expected = match kind {
            ArgsParseType::String if value.kind() != ArgsType::String => Some("\"string\""),
            ArgsParseType::Commands if value.kind() != ArgsType::Commands => Some("{ commands }"),
            ArgsParseType::Invalid => panic!("unexpected argument type"),
            _ => None,
        };
        if let Some(expected) = expected {
            return Err(format!("argument {} must be {expected}", args.count() + 1).into());
        }
        args.values.push(value.clone());
    }
    if spec.lower != -1 && args.count() < spec.lower as u32 {
        return Err(format!("too few arguments (need at least {})", spec.lower as u32).into());
    }
    if spec.upper != -1 && args.count() > spec.upper as u32 {
        return Err(format!("too many arguments (need at most {})", spec.upper as u32).into());
    }
    Ok(Some(args))
}

fn flag_error(prefix: &[u8], flag: u8, suffix: &[u8]) -> ByteString {
    let mut error = ByteString::with_capacity(prefix.len() + 1 + suffix.len());
    error.extend_from_slice(prefix);
    error.push(flag);
    error.extend_from_slice(suffix);
    error
}

fn expand_template(template: &[u8], argv: &[ByteString]) -> ByteString {
    let Some((first, rest)) = argv.split_first() else {
        return ByteString::from(cstr(template));
    };
    let mut expanded = template_replace(template, first, 1);
    for (i, value) in rest.iter().enumerate() {
        expanded = template_replace(&expanded, value, i as u32 + 2);
    }
    expanded
}

impl Args {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn set(&mut self, flag: u8, value: Option<ArgsValue>, flags: ArgsEntryFlags) {
        let entry = self.flags.entry(flag).or_insert_with(|| ArgsEntry {
            flags,
            ..ArgsEntry::default()
        });
        entry.count = entry.count.wrapping_add(1);
        if let Some(value) = value.filter(|value| value.kind() != ArgsType::None) {
            entry.values.push(value);
        }
    }

    pub fn copy(&self, argv: &[ByteString], next_group: &mut u32) -> Self {
        let mut args = Self::create();
        for (&flag, entry) in &self.flags {
            let values: Vec<_> = entry
                .values
                .iter()
                .map(|value| value.copy(argv, next_group))
                .collect();
            let count = if values.is_empty() {
                entry.count
            } else {
                values.len() as u32
            };
            args.flags.insert(
                flag,
                ArgsEntry {
                    values,
                    count,
                    flags: ArgsEntryFlags::default(),
                },
            );
        }
        args.values = self
            .values
            .iter()
            .map(|value| value.copy(argv, next_group))
            .collect();
        args
    }

    pub fn has(&self, flag: u8) -> u32 {
        self.flags.get(&flag).map_or(0, |entry| entry.count)
    }

    pub fn get(&self, flag: u8) -> Option<&[u8]> {
        self.flags
            .get(&flag)?
            .values
            .last()
            .and_then(|value| match &value.data {
                ArgsValueData::String(string) => Some(string.cstr()),
                _ => None,
            })
    }

    pub fn first_value(&self, flag: u8) -> Option<&ArgsValue> {
        self.flags.get(&flag)?.values.first()
    }

    pub fn values_of(&self, flag: u8) -> impl Iterator<Item = &ArgsValue> {
        self.flags
            .get(&flag)
            .into_iter()
            .flat_map(|entry| &entry.values)
    }

    pub fn count(&self) -> u32 {
        self.values.len() as u32
    }

    pub fn values(&self) -> &[ArgsValue] {
        &self.values
    }

    pub fn value(&self, idx: u32) -> Option<&ArgsValue> {
        self.values.get(idx as usize)
    }

    pub fn string(&self, idx: u32) -> Option<&[u8]> {
        self.value(idx).map(ArgsValue::as_string)
    }

    pub fn flags(&self) -> impl Iterator<Item = (u8, &ArgsEntry)> {
        self.flags.iter().map(|(&flag, entry)| (flag, entry))
    }

    pub fn to_vector(&self) -> Vec<ByteString> {
        self.values
            .iter()
            .filter(|value| value.kind() != ArgsType::None)
            .map(|value| ByteString::from(value.as_string()))
            .collect()
    }

    pub fn print(&self) -> ByteString {
        let mut out = ByteString::new();
        for (&flag, entry) in &self.flags {
            if entry.flags.contains(ArgsEntryFlags::OPTIONAL_VALUE) || !entry.values.is_empty() {
                continue;
            }
            if out.is_empty() {
                out.push(b'-');
            }
            out.extend(std::iter::repeat_n(flag, entry.count as usize));
        }
        let mut last_optional = false;
        for (&flag, entry) in &self.flags {
            if entry.flags.contains(ArgsEntryFlags::OPTIONAL_VALUE) {
                print_flag(&mut out, flag);
                last_optional = true;
                continue;
            }
            if entry.values.is_empty() {
                continue;
            }
            for value in &entry.values {
                print_flag(&mut out, flag);
                print_value(&mut out, value);
            }
            last_optional = false;
        }
        if last_optional {
            out.extend_from_slice(b" --");
        }
        for value in &self.values {
            print_value(&mut out, value);
        }
        out
    }
}

fn print_flag(out: &mut ByteString, flag: u8) {
    if !out.is_empty() {
        out.push(b' ');
    }
    out.extend_from_slice(&[b'-', flag]);
}

fn print_value(out: &mut ByteString, value: &ArgsValue) {
    if !out.is_empty() {
        out.push(b' ');
    }
    match &value.data {
        ArgsValueData::None => {}
        ArgsValueData::String(string) => out.extend_from_slice(&escape(string)),
        ArgsValueData::Commands(_) => {
            out.extend_from_slice(b"{ ");
            out.extend_from_slice(value.as_string());
            out.extend_from_slice(b" }");
        }
    }
}

pub fn from_vector(argv: &[ByteString]) -> Vec<ArgsValue> {
    argv.iter().cloned().map(ArgsValue::string).collect()
}

pub fn escape(value: &[u8]) -> ByteString {
    let value = cstr(value);
    if value.is_empty() {
        return ByteString::from("''");
    }
    let quote = if value.iter().any(|c| b" #';${}%".contains(c)) {
        Some(b'"')
    } else if value.contains(&b'"') {
        Some(b'\'')
    } else {
        None
    };
    if value.len() == 1 && value[0] != b' ' && (quote.is_some() || value[0] == b'~') {
        return ByteString::from(vec![b'\\', value[0]]);
    }
    let mut flags = VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL;
    if quote == Some(b'"') {
        flags |= VisFlags::DQ;
    }
    let mut out = ByteString::with_capacity(value.len() + 2);
    if let Some(quote) = quote {
        out.push(quote);
    }
    if value[0] == b'~' && quote != Some(b'\'') {
        out.push(b'\\');
    }
    rmux_util::utf8::strvis(&mut out.0, value, flags);
    if let Some(quote) = quote {
        out.push(quote);
    }
    out
}

pub trait ArgumentFormatRuntime {
    fn expand_from_target(&mut self, item: QueueItemId, value: &[u8]) -> ByteString;
}

fn number(value: &[u8], min: i64, max: i64) -> Result<i64, ByteString> {
    strtonum(value, min, max).map_err(|error| {
        ByteString::from(match error {
            StrtonumError::Invalid => "invalid",
            StrtonumError::TooSmall => "too small",
            StrtonumError::TooLarge => "too large",
        })
    })
}

impl Args {
    pub fn strtonum(&self, flag: u8, min: i64, max: i64) -> Result<i64, ByteString> {
        number(
            self.get(flag).ok_or_else(|| ByteString::from("missing"))?,
            min,
            max,
        )
    }

    pub fn strtonum_and_expand(
        &self,
        runtime: &mut impl ArgumentFormatRuntime,
        flag: u8,
        min: i64,
        max: i64,
        item: QueueItemId,
    ) -> Result<i64, ByteString> {
        let value = self.get(flag).ok_or_else(|| ByteString::from("missing"))?;
        number(&runtime.expand_from_target(item, value), min, max)
    }

    fn percentage_value(&self, flag: u8) -> Result<&[u8], ByteString> {
        let entry = self
            .flags
            .get(&flag)
            .ok_or_else(|| ByteString::from("missing"))?;
        let value = entry
            .values
            .last()
            .ok_or_else(|| ByteString::from("empty"))?;
        match &value.data {
            ArgsValueData::String(value) => Ok(value.cstr()),
            _ => Err(ByteString::from("invalid")),
        }
    }

    pub fn percentage(&self, flag: u8, min: i64, max: i64, cur: i64) -> Result<i64, ByteString> {
        string_percentage(self.percentage_value(flag)?, min, max, cur)
    }

    pub fn percentage_and_expand(
        &self,
        runtime: &mut impl ArgumentFormatRuntime,
        flag: u8,
        min: i64,
        max: i64,
        cur: i64,
        item: QueueItemId,
    ) -> Result<i64, ByteString> {
        string_percentage_and_expand(runtime, self.percentage_value(flag)?, min, max, cur, item)
    }
}

fn scale_percentage(value: i64, min: i64, max: i64, cur: i64) -> Result<i64, ByteString> {
    let scaled = cur.wrapping_mul(value) / 100;
    if scaled < min {
        Err(ByteString::from("too small"))
    } else if scaled > max {
        Err(ByteString::from("too large"))
    } else {
        Ok(scaled)
    }
}

pub fn string_percentage(value: &[u8], min: i64, max: i64, cur: i64) -> Result<i64, ByteString> {
    let value = cstr(value);
    if value.is_empty() {
        return Err(ByteString::from("empty"));
    }
    if let Some(value) = value.strip_suffix(b"%") {
        scale_percentage(number(value, 0, 1000)?, min, max, cur)
    } else {
        number(value, min, max)
    }
}

pub fn string_percentage_and_expand(
    runtime: &mut impl ArgumentFormatRuntime,
    value: &[u8],
    min: i64,
    max: i64,
    cur: i64,
    item: QueueItemId,
) -> Result<i64, ByteString> {
    let value = cstr(value);
    if let Some(value) = value.strip_suffix(b"%") {
        let formatted = runtime.expand_from_target(item, value);
        scale_percentage(number(&formatted, 0, 1000)?, min, max, cur)
    } else {
        number(&runtime.expand_from_target(item, value), min, max)
    }
}

/// Server-owned target snapshots, formatting, client leases, and parser group allocation.
pub trait ArgumentsRuntime: ParseContext + ArgumentFormatRuntime {
    fn item_target(&self, item: QueueItemId) -> CmdFindState;
    fn item_target_client(&self, item: QueueItemId) -> Option<ClientId>;
    fn retain_client(&mut self, client: ClientId);
    fn release_client(&mut self, client: ClientId);
    /// The same counter used by `ParseContext::next_group`.
    fn command_group_counter(&mut self) -> &mut u32;
    fn queue_error(&mut self, item: QueueItemId, message: &[u8]);
}

enum PreparedCommand {
    Commands(Rc<CommandList>),
    String {
        command: ByteString,
        input: RefCell<CmdParseInput>,
    },
}

/// Retains a client only for string commands. Completion and cancellation must call
/// `make_commands_free`; dropping the Rust value cannot release a server-arena lease.
#[must_use = "prepared command state must be released with make_commands_free"]
pub struct ArgsCommandState {
    command: PreparedCommand,
}

pub fn make_commands_prepare(
    runtime: &mut impl ArgumentsRuntime,
    command: &Command,
    item: QueueItemId,
    idx: u32,
    default: Option<&[u8]>,
    wait: bool,
    expand: bool,
) -> ArgsCommandState {
    let value = match command.args.value(idx) {
        Some(ArgsValue {
            data: ArgsValueData::Commands(list),
            ..
        }) => {
            return ArgsCommandState {
                command: PreparedCommand::Commands(Rc::clone(list)),
            };
        }
        Some(ArgsValue {
            data: ArgsValueData::String(value),
            ..
        }) => value.cstr(),
        Some(_) => panic!("unexpected argument type"),
        None => default.expect("argument out of range"),
    };
    let client = runtime.item_target_client(item);
    let text = if expand {
        runtime.expand_from_target(item, value)
    } else {
        ByteString::from(cstr(value))
    };
    if let Some(client) = client {
        runtime.retain_client(client);
    }
    let mut input = CmdParseInput {
        file: command.file.clone(),
        line: command.line,
        item: wait.then_some(item),
        client,
        ..CmdParseInput::default()
    };
    input.target.copy_target_from(&runtime.item_target(item));
    ArgsCommandState {
        command: PreparedCommand::String {
            command: text,
            input: RefCell::new(input),
        },
    }
}

pub fn make_commands(
    runtime: &mut impl ArgumentsRuntime,
    state: &ArgsCommandState,
    argv: &[ByteString],
) -> CmdParseResult {
    match &state.command {
        PreparedCommand::Commands(list) if argv.is_empty() => Ok(Rc::clone(list)),
        PreparedCommand::Commands(list) => {
            Ok(Rc::new(list.copy(argv, runtime.command_group_counter())))
        }
        PreparedCommand::String { command, input } if argv.is_empty() => {
            parse::from_string(runtime, command, &mut input.borrow_mut())
        }
        PreparedCommand::String { command, input } => {
            let expanded = expand_template(command, argv);
            parse::from_string(runtime, &expanded, &mut input.borrow_mut())
        }
    }
}

pub fn make_commands_get_command(state: &ArgsCommandState) -> ByteString {
    match &state.command {
        PreparedCommand::Commands(list) => ByteString::from(
            list.commands
                .first()
                .map_or(&b""[..], |command| command.entry.name),
        ),
        PreparedCommand::String { command, .. } => {
            let command = command.cstr();
            let end = command
                .iter()
                .position(|c| *c == b' ' || *c == b',')
                .unwrap_or(command.len());
            ByteString::from(&command[..end])
        }
    }
}

pub fn make_commands_free(runtime: &mut impl ArgumentsRuntime, state: ArgsCommandState) {
    let client = match state.command {
        PreparedCommand::String { input, .. } => input.into_inner().client,
        PreparedCommand::Commands(_) => None,
    };
    if let Some(client) = client {
        runtime.release_client(client);
    }
}

pub fn make_commands_now(
    runtime: &mut impl ArgumentsRuntime,
    command: &Command,
    item: QueueItemId,
    idx: u32,
    expand: bool,
) -> Option<Rc<CommandList>> {
    let state = make_commands_prepare(runtime, command, item, idx, None, false, expand);
    let result = make_commands(runtime, &state, &[]);
    if let Err(error) = &result {
        runtime.queue_error(item, error.message());
    }
    make_commands_free(runtime, state);
    result.ok()
}

#[cfg(test)]
mod tests {
    use super::super::find::CmdFindFlags;
    use super::super::parse::CmdParseFlags;
    use super::*;
    use crate::ids::{ArenaId, PaneId, SessionId, WindowId, WinlinkId};

    fn values(words: &[&[u8]]) -> Vec<ArgsValue> {
        words
            .iter()
            .map(|word| ArgsValue::string(ByteString::from(*word)))
            .collect()
    }

    fn spec(template: &'static [u8]) -> ArgsParse {
        ArgsParse {
            template,
            lower: -1,
            upper: -1,
            cb: None,
        }
    }

    fn parsed(template: &'static [u8], words: &[&[u8]]) -> Args {
        parse(&spec(template), &values(words)).unwrap().unwrap()
    }

    fn empty_list() -> Rc<CommandList> {
        Rc::new(CommandList {
            group: 1,
            commands: Vec::new(),
        })
    }

    fn command(args: Args) -> Command {
        Command {
            entry: super::super::find_entry(b"run-shell").unwrap(),
            args,
            group: 1,
            file: Some(ByteString::from("input.conf")),
            line: 7,
            parse_flags: CmdParseFlags::VERBOSE,
        }
    }

    #[test]
    fn flag_error_and_usage_corpus() {
        for (word, expected) in [
            (&b"-!"[..], &b"invalid flag -!"[..]),
            (b"-z", b"unknown flag -z"),
            (b"-n", b"-n expects an argument"),
            (b"--x", b"invalid flag --"),
        ] {
            assert_eq!(
                parse(&spec(b"vn:"), &values(&[b"cmd", word])).unwrap_err(),
                expected
            );
        }
        for word in [b"-?".as_slice(), b"-v?", b"-?z"] {
            assert!(
                parse(&spec(b"v"), &values(&[b"cmd", word]))
                    .unwrap()
                    .is_none()
            );
        }
        let args = parsed(b"n:", &[b"cmd", b"-n-?", b"rest"]);
        assert_eq!(args.get(b'n'), Some(&b"-?"[..]));
        assert_eq!(args.string(0), Some(&b"rest"[..]));
    }

    #[test]
    fn packed_counts_attached_values_and_stopping_rules() {
        let args = parsed(b"abvn:", &[b"cmd", b"-vbv", b"-an5", b"--", b"-b"]);
        assert_eq!(args.has(b'v'), 2);
        assert_eq!(args.has(b'b'), 1);
        assert_eq!(args.get(b'n'), Some(&b"5"[..]));
        assert_eq!(args.print(), b"-abvv -n 5 -b");
        for stop in [b"-".as_slice(), b"positional", b""] {
            let args = parsed(b"v", &[b"cmd", stop, b"-v"]);
            assert_eq!(args.has(b'v'), 0);
            assert_eq!(args.count(), 2);
        }
        let args = parsed(b"v", &[b"cmd", b"--", b"-v"]);
        assert_eq!(args.count(), 1);
        assert_eq!(args.string(0), Some(&b"-v"[..]));
    }

    #[test]
    fn optional_first_occurrence_is_retained_and_copy_discards_it() {
        let args = parsed(
            b"o::v",
            &[b"cmd", b"-o", b"-v", b"-o", b"first", b"-o", b"last"],
        );
        assert_eq!(args.has(b'o'), 3);
        assert_eq!(args.get(b'o'), Some(&b"last"[..]));
        assert_eq!(args.first_value(b'o').unwrap().as_string(), b"first");
        assert_eq!(
            args.values_of(b'o')
                .map(ArgsValue::as_string)
                .collect::<Vec<_>>(),
            [b"first", b"last".as_slice()]
        );
        assert_eq!(args.print(), b"-v -o --");
        let copy = args.copy(&[], &mut 30);
        assert_eq!(copy.has(b'o'), 2);
        assert_eq!(copy.print(), b"-v -o first -o last");
        assert!(
            !copy.flags[&b'o']
                .flags
                .contains(ArgsEntryFlags::OPTIONAL_VALUE)
        );
        let bare = parsed(b"o::", &[b"cmd", b"-o"]);
        assert_eq!(bare.copy(&[], &mut 0).print(), b"-o");
    }

    #[test]
    fn valued_first_optional_later_uses_values_only() {
        let args = parsed(b"o::v", &[b"cmd", b"-oA", b"-o", b"-v"]);
        assert_eq!(args.has(b'o'), 2);
        assert_eq!(args.print(), b"-v -o A");
        assert_eq!(args.copy(&[], &mut 0).has(b'o'), 1);
        let args = parsed(b"a::z:", &[b"cmd", b"-a", b"-z", b"value"]);
        assert_eq!(args.print(), b"-a -z value");
        let args = parsed(b"o::", &[b"cmd", b"-o", b"-12"]);
        assert_eq!(args.get(b'o'), Some(&b"-12"[..]));
        let args = parsed(b"o::", &[b"cmd", b"-o", b"-?"]);
        assert_eq!(args.get(b'o'), Some(&b"-?"[..]));
        let args = parsed(b"o::", &[b"cmd", b"-o", b"--", b"tail"]);
        assert_eq!(args.string(0), Some(&b"tail"[..]));
        assert_eq!(args.print(), b"-o -- tail");
    }

    #[test]
    fn optional_flags_reject_command_values_before_optional_detection() {
        for template in [b"o::".as_slice(), b"o:"] {
            let input = vec![
                ArgsValue::string("cmd".into()),
                ArgsValue::string("-o".into()),
                ArgsValue::commands(empty_list()),
            ];
            assert_eq!(
                parse(&spec(template), &input).unwrap_err(),
                b"-o argument must be a string"
            );
        }
    }

    #[test]
    fn positional_callbacks_and_count_error_corpus() {
        let mut parse_spec = spec(b"");
        parse_spec.lower = 1;
        parse_spec.upper = 2;
        assert_eq!(
            parse(&parse_spec, &values(&[b"cmd"])).unwrap_err(),
            b"too few arguments (need at least 1)"
        );
        assert_eq!(
            parse(&parse_spec, &values(&[b"cmd", b"1", b"2", b"3"])).unwrap_err(),
            b"too many arguments (need at most 2)"
        );
        assert_eq!(parse(&parse_spec, &[]).unwrap().unwrap().count(), 0);
        let input = vec![
            ArgsValue::string("cmd".into()),
            ArgsValue::commands(empty_list()),
        ];
        assert_eq!(
            parse(&parse_spec, &input).unwrap_err(),
            b"argument 1 must be \"string\""
        );
        parse_spec.cb = Some(|_, _| Ok(ArgsParseType::Commands));
        assert_eq!(
            parse(&parse_spec, &values(&[b"cmd", b"text"])).unwrap_err(),
            b"argument 1 must be { commands }"
        );
        assert_eq!(parse(&parse_spec, &input).unwrap().unwrap().count(), 1);
        parse_spec.cb = Some(|args, idx| {
            assert_eq!(args.count(), idx);
            Ok(if idx == 0 {
                ArgsParseType::String
            } else {
                ArgsParseType::CommandsOrString
            })
        });
        let input = vec![
            ArgsValue::string("cmd".into()),
            ArgsValue::string("a".into()),
            ArgsValue::none(),
        ];
        assert_eq!(parse(&parse_spec, &input).unwrap().unwrap().count(), 2);
        parse_spec.cb = Some(|_, _| Err("callback cause".into()));
        assert_eq!(parse(&parse_spec, &input).unwrap_err(), b"callback cause");
    }

    #[test]
    fn flag_iteration_set_none_and_vector_export() {
        let mut args = Args::create();
        args.set(b'z', None, ArgsEntryFlags::default());
        args.set(b'a', Some(ArgsValue::none()), ArgsEntryFlags::default());
        args.set(
            b'a',
            Some(ArgsValue::string("first".into())),
            ArgsEntryFlags::OPTIONAL_VALUE,
        );
        args.set(
            b'a',
            Some(ArgsValue::string("last".into())),
            ArgsEntryFlags::default(),
        );
        assert_eq!(
            args.flags().map(|(flag, _)| flag).collect::<Vec<_>>(),
            b"az"
        );
        assert_eq!(args.has(b'a'), 3);
        assert_eq!(args.has(b'x'), 0);
        assert_eq!(args.get(b'a'), Some(&b"last"[..]));
        assert!(args.first_value(b'x').is_none());
        assert_eq!(args.values_of(b'x').count(), 0);
        let list = empty_list();
        args.values = vec![
            ArgsValue::none(),
            ArgsValue::string("text".into()),
            ArgsValue::commands(Rc::clone(&list)),
        ];
        assert_eq!(args.string(0), Some(&b""[..]));
        assert!(args.string(3).is_none());
        assert_eq!(
            args.to_vector(),
            vec![ByteString::from("text"), ByteString::new()]
        );
        assert_eq!(from_vector(&args.to_vector()).len(), 2);
        assert_eq!(args.values().len(), 3);
        let cloned = args.value(2).unwrap().clone();
        assert!(cloned.cached.get().is_none());
        let ArgsValueData::Commands(shared) = cloned.data else {
            panic!("expected commands")
        };
        assert!(Rc::ptr_eq(&shared, &list));
    }

    #[test]
    fn hook_flag_text_matches_pinned_expectation() {
        let args = parsed(
            b"de:t:",
            &[
                b"cmd",
                b"-d",
                b"-e",
                b"A=1",
                b"-e",
                b"B=2",
                b"-t",
                b"one:0",
                b"sleep 60",
            ],
        );
        assert_eq!(args.print(), b"-d -e A=1 -e B=2 -t one:0 \"sleep 60\"");
        assert_eq!(
            args.values_of(b'e')
                .map(ArgsValue::as_string)
                .collect::<Vec<_>>(),
            [b"A=1", b"B=2"]
        );
    }

    #[test]
    fn escape_corpus() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"", b"''"),
            (b" ", b"\" \""),
            (b"#", b"\\#"),
            (b"'", b"\\'"),
            (b"\"", b"\\\""),
            (b";", b"\\;"),
            (b"$", b"\\$"),
            (b"%", b"\\%"),
            (b"{", b"\\{"),
            (b"}", b"\\}"),
            (b"~", b"\\~"),
            (b"~user", b"\\~user"),
            (b"~a b", b"\"\\~a b\""),
            (b"~a\"b", b"'~a\"b'"),
            (b"a\"b", b"'a\"b'"),
            (b"a'b", b"\"a'b\""),
            (b"a\\b", b"a\\\\b"),
            (b"a\n\tb\r", b"a\\n\\tb\\r"),
            (b"\x01\xff", b"\\001\\377"),
            (b"$HOME", b"\"\\$HOME\""),
            (b"${HOME}", b"\"\\${HOME}\""),
            (b"$9", b"\"$9\""),
            (b"a\0ignored", b"a"),
            (
                "\u{754c}\u{1f642}".as_bytes(),
                "\u{754c}\u{1f642}".as_bytes(),
            ),
        ];
        for &(input, expected) in cases {
            assert_eq!(escape(input), expected, "input {input:?}");
        }
    }

    #[derive(Default)]
    struct Runtime {
        env: BTreeMap<ByteString, ByteString>,
        substitutions: BTreeMap<ByteString, ByteString>,
        expansions: Vec<ByteString>,
        conditions: Vec<CmdParseInput>,
        errors: Vec<ByteString>,
        printed: Vec<ByteString>,
        target: CmdFindState,
        client: Option<ClientId>,
        leases: i32,
        next_group: u32,
    }

    impl ParseContext for Runtime {
        fn environment(&self, name: &[u8]) -> Option<&[u8]> {
            self.env.get(name).map(ByteString::as_bytes)
        }
        fn put_environment(&mut self, assignment: &[u8], _: bool) {
            let equals = assignment.iter().position(|&c| c == b'=').unwrap();
            self.env.insert(
                ByteString::from(&assignment[..equals]),
                ByteString::from(&assignment[equals + 1..]),
            );
        }
        fn alias(&self, _: &[u8]) -> Option<ByteString> {
            None
        }
        fn condition(&mut self, format: &[u8], input: &CmdParseInput) -> bool {
            self.conditions.push(input.clone());
            format == b"1"
        }
        fn home(&mut self, _: Option<&[u8]>) -> Option<ByteString> {
            Some("/home/test".into())
        }
        fn next_group(&mut self) -> u32 {
            let group = self.next_group;
            self.next_group = group.wrapping_add(1);
            group
        }
        fn print(&mut self, message: &[u8], _: &CmdParseInput) {
            self.printed.push(message.into());
        }
    }

    impl ArgumentFormatRuntime for Runtime {
        fn expand_from_target(&mut self, _: QueueItemId, value: &[u8]) -> ByteString {
            self.expansions.push(value.into());
            self.substitutions
                .get(value)
                .cloned()
                .unwrap_or_else(|| value.into())
        }
    }

    impl ArgumentsRuntime for Runtime {
        fn item_target(&self, _: QueueItemId) -> CmdFindState {
            self.target
        }
        fn item_target_client(&self, _: QueueItemId) -> Option<ClientId> {
            self.client
        }
        fn retain_client(&mut self, client: ClientId) {
            assert_eq!(Some(client), self.client);
            self.leases += 1;
        }
        fn release_client(&mut self, client: ClientId) {
            assert_eq!(Some(client), self.client);
            self.leases -= 1;
        }
        fn command_group_counter(&mut self) -> &mut u32 {
            &mut self.next_group
        }
        fn queue_error(&mut self, _: QueueItemId, message: &[u8]) {
            self.errors.push(message.into());
        }
    }

    fn item() -> QueueItemId {
        QueueItemId::from_parts(2, 1)
    }

    #[test]
    fn numeric_missing_empty_and_range_corpus() {
        let mut args = Args::create();
        assert_eq!(args.strtonum(b'n', 0, 100).unwrap_err(), b"missing");
        assert_eq!(args.percentage(b'n', 0, 100, 80).unwrap_err(), b"missing");
        args.set(b'n', None, ArgsEntryFlags::default());
        assert_eq!(args.strtonum(b'n', 0, 100).unwrap_err(), b"missing");
        assert_eq!(args.percentage(b'n', 0, 100, 80).unwrap_err(), b"empty");
        args.set(
            b'n',
            Some(ArgsValue::string(ByteString::new())),
            ArgsEntryFlags::default(),
        );
        assert_eq!(args.strtonum(b'n', 0, 100).unwrap_err(), b"invalid");
        assert_eq!(args.percentage(b'n', 0, 100, 80).unwrap_err(), b"empty");
        for (value, expected) in [
            (b"bad".as_slice(), b"invalid".as_slice()),
            (b"-1", b"too small"),
            (b"101", b"too large"),
            (b"9223372036854775808", b"too large"),
            (b"-9223372036854775809", b"too small"),
            (b"1 ", b"invalid"),
        ] {
            args.set(
                b'n',
                Some(ArgsValue::string(value.into())),
                ArgsEntryFlags::default(),
            );
            assert_eq!(args.strtonum(b'n', 0, 100).unwrap_err(), expected);
        }
        args.set(
            b'n',
            Some(ArgsValue::string(" +12".into())),
            ArgsEntryFlags::default(),
        );
        assert_eq!(args.strtonum(b'n', 0, 100), Ok(12));
        args.set(
            b'n',
            Some(ArgsValue::commands(empty_list())),
            ArgsEntryFlags::default(),
        );
        assert_eq!(args.strtonum(b'n', 0, 100).unwrap_err(), b"missing");
        assert_eq!(args.percentage(b'n', 0, 100, 80).unwrap_err(), b"invalid");
    }

    #[test]
    fn percentage_scale_and_pre_expansion_selection() {
        for (value, expected) in [
            (b"50%".as_slice(), 40),
            (b"1000%", 800),
            (b"0%", 0),
            (b"27", 27),
        ] {
            assert_eq!(string_percentage(value, 0, 1000, 80), Ok(expected));
        }
        for (value, min, max, expected) in [
            (b"".as_slice(), 0, 100, b"empty".as_slice()),
            (b"%", 0, 100, b"invalid"),
            (b"1001%", 0, 10000, b"too large"),
            (b"-1%", -100, 100, b"too small"),
            (b"50%", 41, 100, b"too small"),
            (b"50%", 0, 39, b"too large"),
        ] {
            assert_eq!(
                string_percentage(value, min, max, 80).unwrap_err(),
                expected
            );
        }
        assert_eq!(string_percentage(b"50%", -100, 0, -81), Ok(-40));
        let mut runtime = Runtime::default();
        runtime.substitutions.insert("#{n}".into(), "50".into());
        runtime.substitutions.insert("#{p}".into(), "50%".into());
        assert_eq!(
            string_percentage_and_expand(&mut runtime, b"#{n}%", 0, 100, 80, item()),
            Ok(40)
        );
        assert_eq!(runtime.expansions.last().unwrap(), b"#{n}");
        assert_eq!(
            string_percentage_and_expand(&mut runtime, b"#{p}", 0, 100, 80, item()).unwrap_err(),
            b"invalid"
        );
        assert_eq!(
            string_percentage_and_expand(&mut runtime, b"", 0, 100, 80, item()).unwrap_err(),
            b"invalid"
        );
        let args = parsed(b"n:", &[b"cmd", b"-n", b"#{n}%"]);
        assert_eq!(
            args.percentage_and_expand(&mut runtime, b'n', 0, 100, 80, item()),
            Ok(40)
        );
        let args = parsed(b"n:", &[b"cmd", b"-n", b"#{n}"]);
        assert_eq!(
            args.strtonum_and_expand(&mut runtime, b'n', 0, 100, item()),
            Ok(50)
        );
        assert_eq!(
            args.percentage_and_expand(&mut runtime, b'x', 0, 100, 80, item())
                .unwrap_err(),
            b"missing"
        );
        assert_eq!(
            args.strtonum_and_expand(&mut runtime, b'x', 0, 100, item())
                .unwrap_err(),
            b"missing"
        );
        let args = parsed(b"n", &[b"cmd", b"-n"]);
        assert_eq!(
            args.percentage_and_expand(&mut runtime, b'n', 0, 100, 80, item())
                .unwrap_err(),
            b"empty"
        );
    }

    #[test]
    fn template_substitution_corpus_and_sequential_passes() {
        type TemplateCase<'a> = (&'a [u8], &'a [u8], u32, &'a [u8]);
        let cases: &[TemplateCase<'_>] = &[
            (b"literal", b"x", 1, b"literal"),
            (b"%1/%2", b"x", 1, b"x/%2"),
            (b"%10", b"x", 1, b"x0"),
            (b"%1", b"x", 10, b"%1"),
            (b"%% %%", b"a'b", 1, b"a'\\''b %%"),
            (b"%1%", b"\"\\$;~'", 1, b"\\\"\\\\\\$\\;\\~'"),
            (b"%%%", b"$x", 1, b"\\$x"),
            (b"%9", b"x", 9, b"x"),
        ];
        for &(template, value, index, expected) in cases {
            assert_eq!(template_replace(template, value, index), expected);
        }
        assert_eq!(
            expand_template(b"%% %% %2", &["one".into(), "two".into()]),
            b"one two two"
        );
        assert_eq!(expand_template(b"%1", &["%2".into(), "two".into()]), b"two");
        let mut args = Args::create();
        args.set(
            b'f',
            Some(ArgsValue::string("%1".into())),
            ArgsEntryFlags::default(),
        );
        args.values = values(&[b"%2"]);
        let copy = args.copy(&["one".into(), "two".into()], &mut 0);
        assert_eq!(copy.get(b'f'), Some(&b"one"[..]));
        assert_eq!(copy.string(0), Some(&b"two"[..]));
    }

    #[test]
    fn printer_preserving_round_trip_corpus() {
        use super::super::parse::lexer::{Lexer, Token};
        let mut runtime = Runtime::default();
        let mut cases = vec![
            Args::create(),
            parsed(b"vo::n:", &[b"cmd", b"-vv", b"-n", b"5", b"-o"]),
            parsed(b"vo::n:", &[b"cmd", b"-o", b"value", b"--", b"tail"]),
        ];
        let mut args = Args::create();
        args.values = values(&[
            b"", b" ", b"#", b"'", b"\"", b";", b"$", b"%", b"{", b"}", b"~", b"a b", b"$NAME",
            b"${NAME}", b"a\\b", b"a\n\tb", b"\xff",
        ]);
        cases.push(args);
        for args in cases {
            let printed = args.print();
            let mut lexer = Lexer::new(&printed);
            let mut input = CmdParseInput::default();
            let mut argv = values(&[b"cmd"]);
            loop {
                match lexer.next(&mut runtime, &mut input) {
                    Token::Word(word) | Token::Equals(word) => argv.push(ArgsValue::string(word)),
                    Token::Newline | Token::Eof => break,
                    unexpected => panic!("unexpected token {unexpected:?} in {printed:?}"),
                }
            }
            let round_trip = parse(&spec(b"vo::n:"), &argv).unwrap().unwrap();
            assert_eq!(round_trip.print(), printed);
            assert_eq!(round_trip.to_vector(), args.to_vector());
            for (flag, entry) in args.flags() {
                assert_eq!(round_trip.has(flag), entry.count);
                assert_eq!(round_trip.flags[&flag].flags, entry.flags);
            }
        }
    }

    #[test]
    fn command_values_cache_vectors_braces_and_copy_groups() {
        let mut runtime = Runtime {
            next_group: 40,
            ..Runtime::default()
        };
        let list = parse::from_buffer(
            &mut runtime,
            b"display first\ndisplay second\n",
            &mut CmdParseInput::default(),
        )
        .unwrap();
        let mut args = Args::create();
        args.values.push(ArgsValue::commands(Rc::clone(&list)));
        assert_eq!(
            args.string(0),
            Some(&b"display-message first ;; display-message second"[..])
        );
        assert!(args.value(0).unwrap().cached.get().is_some());
        assert_eq!(
            args.to_vector(),
            vec![ByteString::from(
                "display-message first ;; display-message second"
            )]
        );
        assert_eq!(
            args.print(),
            b"{ display-message first ;; display-message second }"
        );
        assert_eq!(
            list.print(CommandListPrintFlags::ESCAPED),
            b"display-message first \\;\\; display-message second"
        );
        assert_eq!(
            list.print(CommandListPrintFlags::ESCAPED | CommandListPrintFlags::NO_GROUPS),
            b"display-message first \\; display-message second"
        );
        let copy = args.copy(&[], &mut 100);
        assert!(copy.value(0).unwrap().cached.get().is_none());
        let ArgsValueData::Commands(copied) = &copy.values[0].data else {
            panic!("expected commands")
        };
        assert!(!Rc::ptr_eq(copied, &list));
        assert_ne!(copied.commands[0].group, list.commands[0].group);
        assert_ne!(copied.commands[0].group, copied.commands[1].group);
        assert!(
            copied
                .commands
                .iter()
                .all(|command| command.parse_flags == CmdParseFlags::default())
        );
        let mut input = CmdParseInput::default();
        let outer = parse::from_string(
            &mut runtime,
            b"run-shell -C { display first ; display second }",
            &mut input,
        )
        .unwrap();
        let text = outer.print(CommandListPrintFlags::default());
        let repeated =
            parse::from_string(&mut runtime, &text, &mut CmdParseInput::default()).unwrap();
        assert_eq!(repeated.print(CommandListPrintFlags::default()), text);
    }

    #[test]
    fn prepared_string_snapshots_context_and_releases_on_completion_or_cancel() {
        let target = CmdFindState {
            flags: CmdFindFlags::QUIET,
            s: Some(SessionId::from_parts(1, 1)),
            wl: Some(WinlinkId::from_parts(2, 1)),
            w: Some(WindowId::from_parts(3, 1)),
            wp: Some(PaneId::from_parts(4, 1)),
            idx: 9,
        };
        let mut runtime = Runtime {
            target,
            client: Some(ClientId::from_parts(5, 1)),
            next_group: 60,
            ..Runtime::default()
        };
        runtime
            .substitutions
            .insert("#{command}".into(), "%if 1 display %1 %endif".into());
        let mut args = Args::create();
        args.values = values(&[b"#{command}"]);
        let command = command(args);
        let state = make_commands_prepare(&mut runtime, &command, item(), 0, None, true, true);
        assert_eq!(runtime.leases, 1);
        assert_eq!(runtime.expansions, vec![ByteString::from("#{command}")]);
        let PreparedCommand::String { input, .. } = &state.command else {
            panic!("expected string")
        };
        assert_eq!(input.borrow().file, command.file);
        assert_eq!(input.borrow().line, 7);
        assert_eq!(input.borrow().item, Some(item()));
        assert_eq!(input.borrow().target.wp, target.wp);
        assert_eq!(input.borrow().target.idx, 9);
        assert_eq!(input.borrow().target.flags, CmdFindFlags::default());
        assert_eq!(input.borrow().flags, CmdParseFlags::default());
        runtime.target = CmdFindState::default();
        let list = make_commands(&mut runtime, &state, &["hello".into()]).unwrap();
        assert_eq!(
            list.print(CommandListPrintFlags::default()),
            b"display-message hello"
        );
        assert_eq!(runtime.conditions[0].client, runtime.client);
        assert_eq!(runtime.conditions[0].target.wp, target.wp);
        assert_eq!(runtime.conditions[0].item, Some(item()));
        assert!(runtime.printed.is_empty());
        make_commands_free(&mut runtime, state);
        assert_eq!(runtime.leases, 0);
        let state = make_commands_prepare(
            &mut runtime,
            &command,
            item(),
            8,
            Some(b"display,extra ignored"),
            false,
            false,
        );
        assert_eq!(make_commands_get_command(&state), b"display");
        let PreparedCommand::String { input, .. } = &state.command else {
            panic!("expected string")
        };
        assert!(input.borrow().item.is_none());
        make_commands_free(&mut runtime, state);
        assert_eq!(runtime.leases, 0);
    }

    #[test]
    fn prepared_commands_share_or_copy_and_skip_client_leases() {
        let mut runtime = Runtime {
            client: Some(ClientId::from_parts(5, 1)),
            next_group: 10,
            ..Runtime::default()
        };
        let list = parse::from_string(
            &mut runtime,
            b"display %1 ; display %2",
            &mut CmdParseInput::default(),
        )
        .unwrap();
        let mut args = Args::create();
        args.values = vec![ArgsValue::commands(Rc::clone(&list))];
        let state =
            make_commands_prepare(&mut runtime, &command(args), item(), 0, None, true, true);
        assert_eq!(runtime.leases, 0);
        assert!(runtime.expansions.is_empty());
        assert_eq!(make_commands_get_command(&state), b"display-message");
        let shared = make_commands(&mut runtime, &state, &[]).unwrap();
        assert!(Rc::ptr_eq(&shared, &list));
        let copy = make_commands(&mut runtime, &state, &["one".into(), "two".into()]).unwrap();
        assert_eq!(
            copy.print(CommandListPrintFlags::default()),
            b"display-message one ; display-message two"
        );
        assert!(!Rc::ptr_eq(&copy, &list));
        make_commands_free(&mut runtime, state);
        assert_eq!(runtime.leases, 0);
        let mut args = Args::create();
        args.values = vec![ArgsValue::commands(empty_list())];
        let state =
            make_commands_prepare(&mut runtime, &command(args), item(), 0, None, false, false);
        assert_eq!(make_commands_get_command(&state), b"");
        make_commands_free(&mut runtime, state);
    }

    #[test]
    fn prepared_errors_are_returned_now_reports_and_releases() {
        let mut runtime = Runtime {
            client: Some(ClientId::from_parts(5, 1)),
            ..Runtime::default()
        };
        let mut args = Args::create();
        args.values = values(&[b"not-a-command"]);
        let command = command(args);
        let state = make_commands_prepare(&mut runtime, &command, item(), 0, None, false, false);
        let error = make_commands(&mut runtime, &state, &[]).unwrap_err();
        assert!(error.message().ends_with(b"unknown command: not-a-command"));
        assert!(error.message().starts_with(b"input.conf:"));
        assert!(runtime.errors.is_empty());
        make_commands_free(&mut runtime, state);
        assert_eq!(runtime.leases, 0);
        assert!(make_commands_now(&mut runtime, &command, item(), 0, false).is_none());
        assert_eq!(runtime.errors.len(), 1);
        assert_eq!(runtime.leases, 0);
        let mut args = Args::create();
        args.values = values(&[b"display hello"]);
        let result =
            make_commands_now(&mut runtime, &self::command(args), item(), 0, false).unwrap();
        assert_eq!(
            result.print(CommandListPrintFlags::default()),
            b"display-message hello"
        );
        assert_eq!(runtime.leases, 0);
    }

    #[test]
    #[should_panic(expected = "argument out of range")]
    fn preparing_out_of_range_without_default_is_an_invariant_failure() {
        let mut runtime = Runtime::default();
        let _state = make_commands_prepare(
            &mut runtime,
            &command(Args::create()),
            item(),
            0,
            None,
            false,
            false,
        );
    }

    #[test]
    fn prepared_templates_quote_values_and_preserve_source_on_multiple_makes() {
        let mut runtime = Runtime::default();
        let mut args = Args::create();
        args.values = values(&[b"display \"%1%\""]);
        let command = command(args);
        let state = make_commands_prepare(&mut runtime, &command, item(), 0, None, false, false);
        for value in [b"a\"b$HOME;~'".as_slice(), b"next"] {
            let list = make_commands(&mut runtime, &state, &[ByteString::from(value)]).unwrap();
            assert_eq!(list.commands[0].args.string(0), Some(value));
            assert_eq!(list.commands[0].file, command.file);
            assert!(
                list.commands[0]
                    .parse_flags
                    .contains(parse::CmdParseFlags::ONEGROUP)
            );
        }
        make_commands_free(&mut runtime, state);
    }

    #[test]
    fn expanded_numeric_failures_keep_range_causes() {
        let mut runtime = Runtime::default();
        runtime.substitutions.insert("#{n}".into(), "50".into());
        for (min, max, cause) in [(41, 100, "too small"), (0, 39, "too large")] {
            assert_eq!(
                string_percentage_and_expand(&mut runtime, b"#{n}%", min, max, 80, item())
                    .unwrap_err(),
                cause
            );
        }
        runtime.substitutions.insert("#{n}".into(), "1001".into());
        assert_eq!(
            string_percentage_and_expand(&mut runtime, b"#{n}%", 0, 10000, 80, item()).unwrap_err(),
            b"too large"
        );
        let mut args = Args::create();
        args.set(
            b'n',
            Some(ArgsValue::commands(Rc::new(CommandList::default()))),
            ArgsEntryFlags::default(),
        );
        assert_eq!(
            args.strtonum_and_expand(&mut runtime, b'n', 0, 100, item())
                .unwrap_err(),
            b"missing"
        );
        assert_eq!(
            args.percentage_and_expand(&mut runtime, b'n', 0, 100, 80, item())
                .unwrap_err(),
            b"invalid"
        );
    }
}

#[cfg(test)]
mod command_value_tests {
    use super::*;

    #[test]
    fn command_position_stops_flag_scanning_and_shares_original_list() {
        let list = Rc::new(CommandList::default());
        let input = vec![
            ArgsValue::string("cmd".into()),
            ArgsValue::commands(Rc::clone(&list)),
            ArgsValue::string("-v".into()),
        ];
        let spec = ArgsParse {
            template: b"v",
            lower: -1,
            upper: -1,
            cb: Some(|_, _| Ok(ArgsParseType::CommandsOrString)),
        };
        let args = parse(&spec, &input).unwrap().unwrap();
        assert_eq!(args.has(b'v'), 0);
        assert_eq!(args.count(), 2);
        assert_eq!(args.string(1), Some(&b"-v"[..]));
        let ArgsValueData::Commands(shared) = &args.value(0).unwrap().data else {
            panic!("expected commands")
        };
        assert!(Rc::ptr_eq(shared, &list));
    }

    #[test]
    fn byte_flag_errors_keep_original_bytes() {
        rmux_sys::locale::setup_ctype().expect("UTF-8 locale");
        let spec = ArgsParse {
            template: b"v",
            lower: -1,
            upper: -1,
            cb: None,
        };
        let input = [
            ArgsValue::string("cmd".into()),
            ArgsValue::string(ByteString::from(&b"-\xff"[..])),
        ];
        let error = parse(&spec, &input).unwrap_err();
        assert_eq!(error.last(), Some(&0xff));
        assert_eq!(
            error,
            if cfg!(target_os = "macos") {
                ByteString::from(&b"unknown flag -\xff"[..])
            } else {
                ByteString::from(&b"invalid flag -\xff"[..])
            }
        );
    }
}
