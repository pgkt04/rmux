// Ported from tmux cmd.c, tmux.h @ 8f25579c
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandFlags(pub u32);
impl CommandFlags {
    pub const STARTSERVER: Self = Self(1);
    pub const READONLY: Self = Self(2);
    pub const AFTERHOOK: Self = Self(4);
    pub const CLIENT_CFLAG: Self = Self(8);
    pub const CLIENT_TFLAG: Self = Self(16);
    pub const CLIENT_CANFAIL: Self = Self(32);
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
impl std::ops::BitOr for CommandFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for CommandFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for CommandFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandListPrintFlags(pub u32);
impl CommandListPrintFlags {
    pub const ESCAPED: Self = Self(1);
    pub const NO_GROUPS: Self = Self(2);
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
impl std::ops::BitOr for CommandListPrintFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for CommandListPrintFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for CommandListPrintFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

pub mod arguments;
pub mod cfg;
pub mod commands;
pub mod find;
pub mod hooks;
pub mod key_bindings;
mod metadata;
pub mod parse;
pub mod queue;
pub use find::{mouse_at, mouse_pane, mouse_window};
pub use metadata::COMMAND_TABLE;

use arguments::{Args, ArgsParse};
use find::{CmdFindFlags, CmdFindType};
use parse::CmdParseFlags;
use rmux_util::bytes::ByteString;

pub const MAX_COMMAND_ARGV: usize = 1000;
pub type CommandName = ByteString;
pub type Commands = Vec<Command>;
pub type CommandTable = &'static [&'static CommandEntry];

#[derive(Clone, Copy, Debug)]
pub struct CommandEntryFlag {
    pub flag: u8,
    pub kind: CmdFindType,
    pub flags: CmdFindFlags,
}

pub struct CommandEntry {
    pub name: &'static [u8],
    pub alias: Option<&'static [u8]>,
    pub args: ArgsParse,
    pub usage: &'static [u8],
    pub source: CommandEntryFlag,
    pub target: CommandEntryFlag,
    pub flags: CommandFlags,
}
impl std::fmt::Debug for CommandEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandEntry")
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Debug)]
pub struct Command {
    pub entry: &'static CommandEntry,
    pub args: Args,
    pub group: u32,
    pub file: Option<ByteString>,
    pub line: u32,
    pub parse_flags: CmdParseFlags,
}
impl Command {
    pub fn entry(&self) -> &'static CommandEntry {
        self.entry
    }
    pub fn args(&self) -> &Args {
        &self.args
    }
    pub fn group(&self) -> u32 {
        self.group
    }
    pub fn source(&self) -> (Option<&[u8]>, u32) {
        (self.file.as_ref().map(|s| s.as_ref()), self.line)
    }
    pub fn parse_flags(&self) -> CmdParseFlags {
        self.parse_flags
    }
    pub fn print(&self) -> ByteString {
        let args = self.args.print();
        let mut out = Vec::with_capacity(self.entry.name.len() + args.len() + 1);
        out.extend_from_slice(self.entry.name);
        if !args.is_empty() {
            out.push(b' ');
            out.extend_from_slice(&args);
        }
        out.into()
    }
}

#[derive(Debug, Default)]
pub struct CommandList {
    pub group: u32,
    pub commands: Vec<Command>,
}
impl CommandList {
    pub fn print(&self, flags: CommandListPrintFlags) -> ByteString {
        let mut out = Vec::new();
        for (index, command) in self.commands.iter().enumerate() {
            if index != 0 {
                let grouped = !flags.contains(CommandListPrintFlags::NO_GROUPS)
                    && self.commands[index - 1].group != command.group;
                out.extend_from_slice(
                    match (flags.contains(CommandListPrintFlags::ESCAPED), grouped) {
                        (false, false) => b" ; ",
                        (false, true) => b" ;; ",
                        (true, false) => b" \\; ",
                        (true, true) => b" \\;\\; ",
                    },
                );
            }
            out.extend_from_slice(&command.print());
        }
        out.into()
    }
    pub fn copy(&self, argv: &[ByteString], next_group: &mut u32) -> Self {
        let mut group = *next_group;
        *next_group = next_group.wrapping_add(1);
        let mut previous = self.group;
        let mut commands = Vec::with_capacity(self.commands.len());
        for command in &self.commands {
            if command.group != previous {
                group = *next_group;
                *next_group = next_group.wrapping_add(1);
                previous = command.group;
            }
            commands.push(Command {
                entry: command.entry,
                args: command.args.copy(argv, next_group),
                group,
                file: command.file.clone(),
                line: command.line,
                parse_flags: CmdParseFlags::default(),
            });
        }
        Self { group, commands }
    }
    pub fn all_have(&self, flags: CommandFlags) -> bool {
        self.commands.iter().all(|c| c.entry.flags.contains(flags))
    }
    pub fn any_have(&self, flags: CommandFlags) -> bool {
        self.commands
            .iter()
            .any(|c| c.entry.flags.intersects(flags))
    }
}

pub fn find_entry(name: &[u8]) -> Result<&'static CommandEntry, ByteString> {
    let mut found = None;
    let mut ambiguous = false;
    for &entry in COMMAND_TABLE {
        if entry.alias == Some(name) {
            return Ok(entry);
        }
        if !entry.name.starts_with(name) {
            continue;
        }
        if found.is_some() {
            ambiguous = true;
        }
        found = Some(entry);
        if entry.name == name {
            break;
        }
    }
    if ambiguous {
        let mut possibilities = Vec::new();
        for entry in COMMAND_TABLE.iter().filter(|e| e.name.starts_with(name)) {
            for part in [entry.name, b", "] {
                let remaining = 8191 - possibilities.len();
                possibilities.extend_from_slice(&part[..part.len().min(remaining)]);
                if part.len() >= remaining {
                    break;
                }
            }
            if possibilities.len() == 8191 {
                break;
            }
        }
        possibilities.truncate(possibilities.len().saturating_sub(2));
        let mut error = b"ambiguous command: ".to_vec();
        error.extend_from_slice(name);
        error.extend_from_slice(b", could be: ");
        error.extend_from_slice(&possibilities);
        return Err(error.into());
    }
    found.ok_or_else(|| {
        let mut error = b"unknown command: ".to_vec();
        error.extend_from_slice(name);
        error.into()
    })
}

pub fn stringify_argv(argv: &[ByteString]) -> ByteString {
    let mut out = Vec::new();
    for (i, arg) in argv.iter().enumerate() {
        if i != 0 {
            out.push(b' ');
        }
        out.extend_from_slice(&arguments::escape(arg));
    }
    out.into()
}

pub fn template_replace(template: &[u8], value: &[u8], index: u32) -> ByteString {
    let mut out = Vec::with_capacity(template.len());
    let mut at = 0;
    let mut replaced = false;
    while at < template.len() {
        let ch = template[at];
        at += 1;
        if ch != b'%' {
            out.push(ch);
            continue;
        }
        let mut quote = 0;
        if template
            .get(at)
            .is_some_and(|ch| (b'1'..=b'9').contains(ch) && u32::from(*ch - b'0') == index)
        {
            at += 1;
            if template.get(at) == Some(&b'%') {
                quote = 2;
                at += 1;
            }
        } else if template.get(at) == Some(&b'%') && !replaced {
            replaced = true;
            at += 1;
            quote = 1;
            if template.get(at) == Some(&b'%') {
                quote = 2;
                at += 1;
            }
        } else {
            out.push(ch);
            continue;
        }
        for &ch in value {
            if quote == 1 && ch == b'\'' {
                out.extend_from_slice(b"'\\''");
                continue;
            }
            if quote == 2 && b"\"\\$;~".contains(&ch) {
                out.push(b'\\');
            }
            out.push(ch);
        }
    }
    out.into()
}

pub fn get_alias(store: &crate::options::OptionsStore, name: &[u8]) -> Option<ByteString> {
    let entry = store.get_only(store.global, b"command-alias")?;
    for (_, item) in entry.array_items() {
        let crate::options::OptionsValue::String(value) = item.value() else {
            continue;
        };
        if let Some(equals) = value.iter().position(|&byte| byte == b'=') {
            if &value[..equals] == name {
                return Some(value[equals + 1..].into());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_markers_and_quoting() {
        assert_eq!(
            template_replace(b"%% %% %1 %1% %%%", b"a'b$;~\\\"", 1),
            b"a'\\''b$;~\\\" %% a'b$;~\\\" a'b\\$\\;\\~\\\\\\\" %%%"
        );
        assert_eq!(template_replace(b"%10 %9 %10", b"X", 1), b"X0 %9 X0");
        assert_eq!(template_replace(b"%10 %%", b"X", 10), b"%10 X");
        assert_eq!(
            template_replace(b"%%%", b"'\"$;~\\", 1),
            b"'\\\"\\$\\;\\~\\\\"
        );
    }
    #[test]
    fn lookup_exact_alias_and_errors() {
        assert_eq!(find_entry(b"display").unwrap().name, b"display-message");
        assert_eq!(
            find_entry(b"display-message").unwrap().alias,
            Some(b"display".as_slice())
        );
        assert_eq!(
            find_entry(b"nonesuch").unwrap_err(),
            b"unknown command: nonesuch"
        );
        assert_eq!(find_entry(b"list-").unwrap_err(),b"ambiguous command: list-, could be: list-buffers, list-clients, list-commands, list-keys, list-panes, list-sessions, list-windows");
        assert_eq!(COMMAND_TABLE.len(), 92);
    }
    fn command(name: &[u8], group: u32) -> Command {
        Command {
            entry: find_entry(name).unwrap(),
            args: Args::create(),
            group,
            file: Some("file".into()),
            line: 7,
            parse_flags: CmdParseFlags::VERBOSE,
        }
    }
    #[test]
    fn print_group_copy_and_flags() {
        let list = CommandList {
            group: 1,
            commands: vec![command(b"start-server", 1), command(b"kill-server", 2)],
        };
        assert_eq!(
            list.print(CommandListPrintFlags::default()),
            b"start-server ;; kill-server"
        );
        assert_eq!(
            list.print(CommandListPrintFlags::ESCAPED),
            b"start-server \\;\\; kill-server"
        );
        assert_eq!(
            list.print(CommandListPrintFlags::NO_GROUPS),
            b"start-server ; kill-server"
        );
        let mut next = 10;
        let copied = list.copy(&[], &mut next);
        assert_eq!(
            copied.commands.iter().map(|c| c.group).collect::<Vec<_>>(),
            vec![10, 11]
        );
        assert_eq!(copied.commands[0].source(), (Some(b"file".as_slice()), 7));
        assert_eq!(copied.commands[0].parse_flags(), CmdParseFlags::default());
        assert!(list.any_have(CommandFlags::STARTSERVER));
        assert!(!list.all_have(CommandFlags::STARTSERVER));
        assert!(CommandList::default().all_have(CommandFlags::READONLY));
    }
    #[test]
    fn metadata_matches_all_pinned_initializer_rows() {
        let mut actual = String::new();
        for entry in COMMAND_TABLE {
            actual.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                String::from_utf8_lossy(entry.name),
                entry.alias.map(String::from_utf8_lossy).unwrap_or_default(),
                String::from_utf8_lossy(entry.args.template),
                entry.args.lower,
                entry.args.upper,
                entry.flags.bits(),
                String::from_utf8_lossy(entry.usage)
            ));
        }
        assert_eq!(actual, include_str!("cmd/metadata.tsv"));
    }
    #[test]
    fn metadata_list_commands_matches_oracle() {
        let oracle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
        if !oracle.exists() {
            eprintln!("skip: pinned oracle missing for list-commands");
            return;
        }
        let root = std::env::temp_dir().join(format!("rmux-g11-commands-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("oracle private directory");
        let socket = root.join("socket");
        struct Cleanup {
            oracle: std::path::PathBuf,
            socket: std::path::PathBuf,
            root: std::path::PathBuf,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::process::Command::new(&self.oracle)
                    .arg("-S")
                    .arg(&self.socket)
                    .arg("kill-server")
                    .output();
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let cleanup = Cleanup {
            oracle,
            socket,
            root,
        };
        let output = std::process::Command::new(&cleanup.oracle)
            .arg("-S")
            .arg(&cleanup.socket)
            .args(["-f/dev/null", "list-commands"])
            .output()
            .expect("oracle list-commands");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut actual = Vec::new();
        for entry in COMMAND_TABLE {
            actual.extend_from_slice(entry.name);
            if let Some(alias) = entry.alias {
                actual.extend_from_slice(b" (");
                actual.extend_from_slice(alias);
                actual.push(b')');
            }
            actual.push(b' ');
            actual.extend_from_slice(entry.usage);
            actual.push(b'\n');
        }
        assert_eq!(actual, output.stdout);
    }
}
