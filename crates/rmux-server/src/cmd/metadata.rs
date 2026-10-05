// Ported from tmux cmd.c, cmd-*.c, tmux.h @ 8f25579c
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

use super::arguments::{Args, ArgsParse, ArgsParseType};
use super::find::{CmdFindFlags, CmdFindType};
use super::{CommandEntry, CommandEntryFlag, CommandFlags};
use rmux_util::bytes::ByteString;

fn commands_or_string(_: &Args, _: u32) -> Result<ArgsParseType, ByteString> {
    Ok(ArgsParseType::CommandsOrString)
}
fn if_arguments(_: &Args, idx: u32) -> Result<ArgsParseType, ByteString> {
    Ok(if idx == 1 || idx == 2 {
        ArgsParseType::CommandsOrString
    } else {
        ArgsParseType::String
    })
}
fn run_arguments(args: &Args, _: u32) -> Result<ArgsParseType, ByteString> {
    Ok(if args.has(b'C') != 0 {
        ArgsParseType::CommandsOrString
    } else {
        ArgsParseType::String
    })
}
fn option_arguments(args: &Args, idx: u32) -> Result<ArgsParseType, ByteString> {
    Ok(if args.has(b'B') != 0 || idx == 1 {
        ArgsParseType::CommandsOrString
    } else {
        ArgsParseType::String
    })
}
fn menu_arguments(args: &Args, idx: u32) -> Result<ArgsParseType, ByteString> {
    let mut i = 0;
    loop {
        if i == idx {
            return Ok(ArgsParseType::String);
        }
        let empty = args.string(i).is_some_and(|s| s.is_empty());
        i += 1;
        if empty {
            continue;
        }
        if i == idx {
            return Ok(ArgsParseType::String);
        }
        i += 1;
        if i == idx {
            return Ok(ArgsParseType::CommandsOrString);
        }
        i += 1;
    }
}

pub static CMD_ATTACH_SESSION: CommandEntry = CommandEntry {
    name: b"attach-session",
    alias: Some(b"attach"),
    args: ArgsParse {
        template: b"c:dEf:rt:x",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-dErx] [-c working-directory] [-f flags] [-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(3),
};
pub static CMD_BIND_KEY: CommandEntry = CommandEntry {
    name: b"bind-key",
    alias: Some(b"bind"),
    args: ArgsParse {
        template: b"nrN:T:",
        lower: 1,
        upper: -1,
        cb: Some(commands_or_string),
    },
    usage: b"[-nr] [-T key-table] [-N note] key [command [argument ...]]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_BREAK_PANE: CommandEntry = CommandEntry {
 name: b"break-pane", alias: Some(b"breakp"),
 args: ArgsParse { template: b"abdPF:n:s:t:Wx:X:y:Y:", lower: 0, upper: 0, cb: None },
 usage: b"[-abdPW] [-F format] [-n window-name] [-s src-pane] [-t dst-window] [-x width] [-y height] [-X x-position] [-Y y-position]",
 source: CommandEntryFlag { flag: b's', kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Window, flags: CmdFindFlags(4) },
 flags: CommandFlags(0),
};
pub static CMD_CAPTURE_PANE: CommandEntry = CommandEntry {
    name: b"capture-pane",
    alias: Some(b"capturep"),
    args: ArgsParse {
        template: b"ab:CeE:FHIJLMNpPqRS:Tt:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-aCeFHIJLMNpPqRT] [-b buffer-name] [-E end-line] [-S start-line] [-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_CHOOSE_BUFFER: CommandEntry = CommandEntry {
 name: b"choose-buffer", alias: None,
 args: ArgsParse { template: b"F:f:K:kNO:rt:yZ", lower: 0, upper: 1, cb: Some(commands_or_string) },
 usage: b"[-kNrZ] [-F format] [-f filter] [-K key-format] [-O sort-order] [-t target-pane] [template]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_CHOOSE_CLIENT: CommandEntry = CommandEntry {
 name: b"choose-client", alias: None,
 args: ArgsParse { template: b"F:f:hiK:kNO:rt:yZ", lower: 0, upper: 1, cb: Some(commands_or_string) },
 usage: b"[-hikNrZ] [-F format] [-f filter] [-K key-format] [-O sort-order] [-t target-pane] [template]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_CHOOSE_TREE: CommandEntry = CommandEntry {
 name: b"choose-tree", alias: None,
 args: ArgsParse { template: b"F:f:GhK:kNO:rst:wyZ", lower: 0, upper: 1, cb: Some(commands_or_string) },
 usage: b"[-GhkNrswZ] [-F format] [-f filter] [-K key-format] [-O sort-order] [-t target-pane] [template]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_CLEAR_HISTORY: CommandEntry = CommandEntry {
    name: b"clear-history",
    alias: Some(b"clearhist"),
    args: ArgsParse {
        template: b"Ht:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-H] [-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_CLEAR_PROMPT_HISTORY: CommandEntry = CommandEntry {
    name: b"clear-prompt-history",
    alias: Some(b"clearphist"),
    args: ArgsParse {
        template: b"T:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-T prompt-type]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_CLOCK_MODE: CommandEntry = CommandEntry {
    name: b"clock-mode",
    alias: None,
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_COMMAND_PROMPT: CommandEntry = CommandEntry {
    name: b"command-prompt",
    alias: None,
    args: ArgsParse {
        template: b"1CbeFiklI:NPp:t:T:",
        lower: 0,
        upper: 1,
        cb: Some(commands_or_string),
    },
    usage: b"[-1CbeFiklNP] [-I inputs] [-p prompts] [-t target-client] [-T prompt-type] [template]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(16),
};
pub static CMD_CONFIRM_BEFORE: CommandEntry = CommandEntry {
    name: b"confirm-before",
    alias: Some(b"confirm"),
    args: ArgsParse {
        template: b"bc:p:t:y",
        lower: 1,
        upper: 1,
        cb: Some(commands_or_string),
    },
    usage: b"[-by] [-c confirm-key] [-p prompt] [-t target-client] command",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(16),
};
pub static CMD_COPY_MODE: CommandEntry = CommandEntry {
    name: b"copy-mode",
    alias: None,
    args: ArgsParse {
        template: b"dekHMqSs:t:u",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-dekHMqSu] [-s src-pane] [-t target-pane]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(6),
};
pub static CMD_CUSTOMIZE_MODE: CommandEntry = CommandEntry {
    name: b"customize-mode",
    alias: None,
    args: ArgsParse {
        template: b"F:f:kNt:yZ",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-kNZ] [-F format] [-f filter] [-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_DELETE_BUFFER: CommandEntry = CommandEntry {
    name: b"delete-buffer",
    alias: Some(b"deleteb"),
    args: ArgsParse {
        template: b"b:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-b buffer-name]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_DETACH_CLIENT: CommandEntry = CommandEntry {
    name: b"detach-client",
    alias: Some(b"detach"),
    args: ArgsParse {
        template: b"aE:s:t:P",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-aP] [-E shell-command] [-s target-session] [-t target-client]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(64),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(18),
};
pub static CMD_DISPLAY_MENU: CommandEntry = CommandEntry {
 name: b"display-menu", alias: Some(b"menu"),
 args: ArgsParse { template: b"b:c:C:H:s:S:MOt:T:x:y:", lower: 1, upper: -1, cb: Some(menu_arguments) },
 usage: b"[-MO] [-b border-lines] [-c target-client] [-C starting-choice] [-H selected-style] [-s style] [-S border-style] [-t target-pane] [-T title] [-x position] [-y position] name [key] [command] ...",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(12),
};
pub static CMD_DISPLAY_MESSAGE: CommandEntry = CommandEntry {
    name: b"display-message",
    alias: Some(b"display"),
    args: ArgsParse {
        template: b"aCc:d:jlINpt:F:v",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-aCIjlNpv] [-c target-client] [-d delay] [-F format] [-t target-pane] [message]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(44),
};
pub static CMD_DISPLAY_POPUP: CommandEntry = CommandEntry {
 name: b"display-popup", alias: Some(b"popup"),
 args: ArgsParse { template: b"Bb:Cc:d:e:Eh:ks:S:t:T:w:x:y:", lower: 0, upper: -1, cb: None },
 usage: b"[-BCEk] [-b border-lines] [-c target-client] [-d start-directory] [-e environment] [-h height] [-s style] [-S border-style] [-t target-pane] [-T title] [-w width] [-x position] [-y position] [shell-command [argument ...]]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(12),
};
pub static CMD_DISPLAY_PANES: CommandEntry = CommandEntry {
    name: b"display-panes",
    alias: Some(b"displayp"),
    args: ArgsParse {
        template: b"d:kNs:t:Z",
        lower: 0,
        upper: 1,
        cb: Some(commands_or_string),
    },
    usage: b"[-kNZ] [-d duration] [-s source-window] [-t target-pane] [template]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_FIND_WINDOW: CommandEntry = CommandEntry {
    name: b"find-window",
    alias: Some(b"findw"),
    args: ArgsParse {
        template: b"CiNrt:TZ",
        lower: 1,
        upper: 1,
        cb: None,
    },
    usage: b"[-CiNrTZ] [-t target-pane] match-string",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_HAS_SESSION: CommandEntry = CommandEntry {
    name: b"has-session",
    alias: Some(b"has"),
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_IF_SHELL: CommandEntry = CommandEntry {
    name: b"if-shell",
    alias: Some(b"if"),
    args: ArgsParse {
        template: b"bFt:",
        lower: 2,
        upper: 3,
        cb: Some(if_arguments),
    },
    usage: b"[-bF] [-t target-pane] shell-command command [command]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(0),
};
pub static CMD_JOIN_PANE: CommandEntry = CommandEntry {
    name: b"join-pane",
    alias: Some(b"joinp"),
    args: ArgsParse {
        template: b"bdfhvp:l:s:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-bdfhv] [-l size] [-s src-pane] [-t dst-pane]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(8),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_KILL_PANE: CommandEntry = CommandEntry {
    name: b"kill-pane",
    alias: Some(b"killp"),
    args: ArgsParse {
        template: b"af:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-a] [-f filter] [-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_KILL_SERVER: CommandEntry = CommandEntry {
    name: b"kill-server",
    alias: None,
    args: ArgsParse {
        template: b"",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_KILL_SESSION: CommandEntry = CommandEntry {
    name: b"kill-session",
    alias: None,
    args: ArgsParse {
        template: b"aCgf:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-aCg] [-f filter] [-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_KILL_WINDOW: CommandEntry = CommandEntry {
    name: b"kill-window",
    alias: Some(b"killw"),
    args: ArgsParse {
        template: b"af:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-a] [-f filter] [-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_LAST_PANE: CommandEntry = CommandEntry {
    name: b"last-pane",
    alias: Some(b"lastp"),
    args: ArgsParse {
        template: b"det:Z",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-deZ] [-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_LAST_WINDOW: CommandEntry = CommandEntry {
    name: b"last-window",
    alias: Some(b"last"),
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_LINK_WINDOW: CommandEntry = CommandEntry {
    name: b"link-window",
    alias: Some(b"linkw"),
    args: ArgsParse {
        template: b"abdks:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-abdk] [-s src-window] [-t dst-window]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_LIST_BUFFERS: CommandEntry = CommandEntry {
    name: b"list-buffers",
    alias: Some(b"lsb"),
    args: ArgsParse {
        template: b"F:f:O:r",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-F format] [-f filter] [-O order]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_LIST_CLIENTS: CommandEntry = CommandEntry {
    name: b"list-clients",
    alias: Some(b"lsc"),
    args: ArgsParse {
        template: b"F:f:O:rt:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-F format] [-f filter] [-O order][-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(6),
};
pub static CMD_LIST_COMMANDS: CommandEntry = CommandEntry {
    name: b"list-commands",
    alias: Some(b"lscm"),
    args: ArgsParse {
        template: b"F:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-F format] [command]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(5),
};
pub static CMD_LIST_KEYS: CommandEntry = CommandEntry {
    name: b"list-keys",
    alias: Some(b"lsk"),
    args: ArgsParse {
        template: b"1aF:NO:P:rT:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-1aNr] [-F format] [-O order] [-P prefix-string][-T key-table] [key]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(5),
};
pub static CMD_LIST_PANES: CommandEntry = CommandEntry {
    name: b"list-panes",
    alias: Some(b"lsp"),
    args: ArgsParse {
        template: b"aF:f:O:rst:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-asr] [-F format] [-f filter] [-O order][-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_LIST_SESSIONS: CommandEntry = CommandEntry {
    name: b"list-sessions",
    alias: Some(b"ls"),
    args: ArgsParse {
        template: b"F:f:O:r",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-r] [-F format] [-f filter] [-O order]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_LIST_WINDOWS: CommandEntry = CommandEntry {
    name: b"list-windows",
    alias: Some(b"lsw"),
    args: ArgsParse {
        template: b"aF:f:O:rt:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-ar] [-F format] [-f filter] [-O order][-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_LOAD_BUFFER: CommandEntry = CommandEntry {
    name: b"load-buffer",
    alias: Some(b"loadb"),
    args: ArgsParse {
        template: b"b:t:w",
        lower: 1,
        upper: 1,
        cb: None,
    },
    usage: b"[-b buffer-name] [-t target-client] path",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(52),
};
pub static CMD_LOCK_CLIENT: CommandEntry = CommandEntry {
    name: b"lock-client",
    alias: Some(b"lockc"),
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-client]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(20),
};
pub static CMD_LOCK_SERVER: CommandEntry = CommandEntry {
    name: b"lock-server",
    alias: Some(b"lock"),
    args: ArgsParse {
        template: b"",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_LOCK_SESSION: CommandEntry = CommandEntry {
    name: b"lock-session",
    alias: Some(b"locks"),
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_MOVE_PANE: CommandEntry = CommandEntry {
 name: b"move-pane", alias: Some(b"movep"),
 args: ArgsParse { template: b"bdD::fhMvl:L::P:R::s:t:U::X:Y:z:", lower: 0, upper: 0, cb: None },
 usage: b"[-bdfhMv] [-D lines] [-l size] [-L columns] [-P position] [-R columns] [-s src-pane] [-t dst-pane] [-U lines] [-X x-position] [-Y y-position] [-z z-index]",
 source: CommandEntryFlag { flag: b's', kind: CmdFindType::Pane, flags: CmdFindFlags(8) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_MOVE_WINDOW: CommandEntry = CommandEntry {
    name: b"move-window",
    alias: Some(b"movew"),
    args: ArgsParse {
        template: b"abdkrs:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-abdkr] [-s src-window] [-t dst-window]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_NEW_PANE: CommandEntry = CommandEntry {
 name: b"new-pane", alias: Some(b"newp"),
 args: ArgsParse { template: b"AbB:Cc:Dde:EfF:hIkl:KLMm:Op:PR:s:S:t:T:vWx:X:y:Y:Z", lower: 0, upper: -1, cb: None },
 usage: b"[-AbCDefhIkKLMOPvWZ] [-B border-lines] [-c start-directory] [-e environment] [-F format] [-l size] [-m message] [-p percentage] [-s style] [-S active-border-style] [-R inactive-border-style] [-T title] [-x width] [-y height] [-X x-position] [-Y y-position] [-t target-pane] [shell-command [argument ...]]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_NEW_SESSION: CommandEntry = CommandEntry {
 name: b"new-session", alias: Some(b"new"),
 args: ArgsParse { template: b"Ac:dDe:EF:f:n:Ps:t:x:Xy:", lower: 0, upper: -1, cb: None },
 usage: b"[-AdDEPX] [-c start-directory] [-e environment] [-F format] [-f flags] [-n window-name] [-s session-name] [-t target-session] [-x width] [-y height] [shell-command [argument ...]]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Session, flags: CmdFindFlags(64) },
 flags: CommandFlags(1),
};
pub static CMD_NEW_WINDOW: CommandEntry = CommandEntry {
 name: b"new-window", alias: Some(b"neww"),
 args: ArgsParse { template: b"abc:de:EF:kn:PSt:", lower: 0, upper: -1, cb: None },
 usage: b"[-abdEkPS] [-c start-directory] [-e environment] [-F format] [-n window-name] [-t target-window] [shell-command [argument ...]]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Window, flags: CmdFindFlags(4) },
 flags: CommandFlags(0),
};
pub static CMD_NEXT_LAYOUT: CommandEntry = CommandEntry {
    name: b"next-layout",
    alias: Some(b"nextl"),
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_NEXT_WINDOW: CommandEntry = CommandEntry {
    name: b"next-window",
    alias: Some(b"next"),
    args: ArgsParse {
        template: b"at:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-a] [-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_PASTE_BUFFER: CommandEntry = CommandEntry {
    name: b"paste-buffer",
    alias: Some(b"pasteb"),
    args: ArgsParse {
        template: b"db:prSs:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-dprS] [-s separator] [-b buffer-name] [-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_PIPE_PANE: CommandEntry = CommandEntry {
    name: b"pipe-pane",
    alias: Some(b"pipep"),
    args: ArgsParse {
        template: b"IOot:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-IOo] [-t target-pane] [shell-command]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_PREVIOUS_LAYOUT: CommandEntry = CommandEntry {
    name: b"previous-layout",
    alias: Some(b"prevl"),
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_PREVIOUS_WINDOW: CommandEntry = CommandEntry {
    name: b"previous-window",
    alias: Some(b"prev"),
    args: ArgsParse {
        template: b"at:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-a] [-t target-session]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_REFRESH_CLIENT: CommandEntry = CommandEntry {
 name: b"refresh-client", alias: Some(b"refresh"),
 args: ArgsParse { template: b"A:B:cC:Df:r:F:lLRSt:U", lower: 0, upper: 1, cb: None },
 usage: b"[-cDlLRSU] [-A pane:state] [-B name:what:format] [-C XxY] [-f flags] [-r pane:report] [-t target-client] [adjustment]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(20),
};
pub static CMD_RENAME_SESSION: CommandEntry = CommandEntry {
    name: b"rename-session",
    alias: Some(b"rename"),
    args: ArgsParse {
        template: b"t:",
        lower: 1,
        upper: 1,
        cb: None,
    },
    usage: b"[-t target-session] new-name",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_RENAME_WINDOW: CommandEntry = CommandEntry {
    name: b"rename-window",
    alias: Some(b"renamew"),
    args: ArgsParse {
        template: b"t:",
        lower: 1,
        upper: 1,
        cb: None,
    },
    usage: b"[-t target-window] new-name",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_RESIZE_PANE: CommandEntry = CommandEntry {
 name: b"resize-pane", alias: Some(b"resizep"),
 args: ArgsParse { template: b"D::L::MR::Tt:U::x:y:Z", lower: 0, upper: 1, cb: None },
 usage: b"[-MTZ] [-D lines] [-L columns] [-R columns] [-U lines] [-x width] [-y height] [-t target-pane]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(4),
};
pub static CMD_RESIZE_WINDOW: CommandEntry = CommandEntry {
    name: b"resize-window",
    alias: Some(b"resizew"),
    args: ArgsParse {
        template: b"aADLRt:Ux:y:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-aADLRU] [-x width] [-y height] [-t target-window] [adjustment]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_RESPAWN_PANE: CommandEntry = CommandEntry {
 name: b"respawn-pane", alias: Some(b"respawnp"),
 args: ArgsParse { template: b"c:e:Ekt:", lower: 0, upper: -1, cb: None },
 usage: b"[-Ek] [-c start-directory] [-e environment] [-t target-pane] [shell-command [argument ...]]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_RESPAWN_WINDOW: CommandEntry = CommandEntry {
 name: b"respawn-window", alias: Some(b"respawnw"),
 args: ArgsParse { template: b"c:e:Ekt:", lower: 0, upper: -1, cb: None },
 usage: b"[-Ek] [-c start-directory] [-e environment] [-t target-window] [shell-command [argument ...]]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Window, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_ROTATE_WINDOW: CommandEntry = CommandEntry {
    name: b"rotate-window",
    alias: Some(b"rotatew"),
    args: ArgsParse {
        template: b"Dt:UZ",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-DUZ] [-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_RUN_SHELL: CommandEntry = CommandEntry {
    name: b"run-shell",
    alias: Some(b"run"),
    args: ArgsParse {
        template: b"bd:Ct:Es:c:",
        lower: 0,
        upper: -1,
        cb: Some(run_arguments),
    },
    usage:
        b"[-bCE] [-c start-directory] [-d delay] [-t target-pane] [shell-command [argument ...]]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(0),
};
pub static CMD_SAVE_BUFFER: CommandEntry = CommandEntry {
    name: b"save-buffer",
    alias: Some(b"saveb"),
    args: ArgsParse {
        template: b"ab:",
        lower: 1,
        upper: 1,
        cb: None,
    },
    usage: b"[-a] [-b buffer-name] path",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_SELECT_LAYOUT: CommandEntry = CommandEntry {
    name: b"select-layout",
    alias: Some(b"selectl"),
    args: ArgsParse {
        template: b"Enopt:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-Enop] [-t target-pane] [layout-name]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_SELECT_PANE: CommandEntry = CommandEntry {
    name: b"select-pane",
    alias: Some(b"selectp"),
    args: ArgsParse {
        template: b"DdegLlMmP:RT:t:UZ",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-DdeLlMmRUZ] [-T title] [-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_SELECT_WINDOW: CommandEntry = CommandEntry {
    name: b"select-window",
    alias: Some(b"selectw"),
    args: ArgsParse {
        template: b"lnpTt:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-lnpT] [-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_SEND_KEYS: CommandEntry = CommandEntry {
    name: b"send-keys",
    alias: Some(b"send"),
    args: ArgsParse {
        template: b"c:FHKlMN:Rt:X",
        lower: 0,
        upper: -1,
        cb: None,
    },
    usage: b"[-FHKlMRX] [-c target-client] [-N repeat-count] [-t target-pane] [key ...]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(46),
};
pub static CMD_SEND_PREFIX: CommandEntry = CommandEntry {
    name: b"send-prefix",
    alias: None,
    args: ArgsParse {
        template: b"2t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-2] [-t target-pane]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_SERVER_ACCESS: CommandEntry = CommandEntry {
    name: b"server-access",
    alias: None,
    args: ArgsParse {
        template: b"adglrw",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-adglrw] [-t target-pane] [user|group]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(32),
};
pub static CMD_SET_BUFFER: CommandEntry = CommandEntry {
    name: b"set-buffer",
    alias: Some(b"setb"),
    args: ArgsParse {
        template: b"ab:t:n:w",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-aw] [-b buffer-name] [-n new-buffer-name] [-t target-client] [data]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(52),
};
pub static CMD_SET_ENVIRONMENT: CommandEntry = CommandEntry {
    name: b"set-environment",
    alias: Some(b"setenv"),
    args: ArgsParse {
        template: b"Fhgrt:u",
        lower: 1,
        upper: 2,
        cb: None,
    },
    usage: b"[-Fhgru] [-t target-session] variable [value]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SET_HOOK: CommandEntry = CommandEntry {
    name: b"set-hook",
    alias: None,
    args: ArgsParse {
        template: b"agpERTt:uB:w",
        lower: 0,
        upper: 2,
        cb: Some(option_arguments),
    },
    usage: b"[-agpERTuw] [-B name:what:format] [-t target-pane] [hook] [command]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SET_OPTION: CommandEntry = CommandEntry {
    name: b"set-option",
    alias: Some(b"set"),
    args: ArgsParse {
        template: b"aFgopqst:uUw",
        lower: 1,
        upper: 2,
        cb: Some(option_arguments),
    },
    usage: b"[-aFgopqsuUw] [-t target-pane] option [value]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SET_WINDOW_OPTION: CommandEntry = CommandEntry {
    name: b"set-window-option",
    alias: Some(b"setw"),
    args: ArgsParse {
        template: b"aFgoqt:u",
        lower: 1,
        upper: 2,
        cb: Some(option_arguments),
    },
    usage: b"[-aFgoqu] [-t target-window] option [value]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SHOW_BUFFER: CommandEntry = CommandEntry {
    name: b"show-buffer",
    alias: Some(b"showb"),
    args: ArgsParse {
        template: b"b:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-b buffer-name]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_SHOW_ENVIRONMENT: CommandEntry = CommandEntry {
    name: b"show-environment",
    alias: Some(b"showenv"),
    args: ArgsParse {
        template: b"hgst:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-hgs] [-t target-session] [variable]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Session,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SHOW_HOOKS: CommandEntry = CommandEntry {
    name: b"show-hooks",
    alias: None,
    args: ArgsParse {
        template: b"BF:gpt:w",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-Bgpw] [-F format] [-t target-pane] [hook]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SHOW_MESSAGES: CommandEntry = CommandEntry {
    name: b"show-messages",
    alias: Some(b"showmsgs"),
    args: ArgsParse {
        template: b"JTt:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-JT] [-t target-client]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(52),
};
pub static CMD_SHOW_OPTIONS: CommandEntry = CommandEntry {
    name: b"show-options",
    alias: Some(b"show"),
    args: ArgsParse {
        template: b"AgF:Hpqst:vw",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-AgHpqsvw] [-F format] [-t target-pane] [option]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SHOW_PROMPT_HISTORY: CommandEntry = CommandEntry {
    name: b"show-prompt-history",
    alias: Some(b"showphist"),
    args: ArgsParse {
        template: b"T:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-T prompt-type]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_SHOW_WINDOW_OPTIONS: CommandEntry = CommandEntry {
    name: b"show-window-options",
    alias: Some(b"showw"),
    args: ArgsParse {
        template: b"F:gvt:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-gv] [-F format] [-t target-window] [option]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(4),
};
pub static CMD_SOURCE_FILE: CommandEntry = CommandEntry {
    name: b"source-file",
    alias: Some(b"source"),
    args: ArgsParse {
        template: b"t:Fnqv",
        lower: 1,
        upper: -1,
        cb: None,
    },
    usage: b"[-Fnqv] [-t target-pane] path ...",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(64),
    },
    flags: CommandFlags(0),
};
pub static CMD_SPLIT_WINDOW: CommandEntry = CommandEntry {
 name: b"split-window", alias: Some(b"splitw"),
 args: ArgsParse { template: b"bB:c:de:EfF:hIkl:m:p:PR:s:S:t:T:vWZ", lower: 0, upper: -1, cb: None },
 usage: b"[-bdefhIklPvWZ] [-B border-lines] [-c start-directory] [-e environment] [-F format] [-l size] [-m message] [-p percentage] [-s style] [-S active-border-style] [-R inactive-border-style] [-T title] [-t target-pane] [shell-command [argument ...]]",
 source: CommandEntryFlag { flag: 0, kind: CmdFindType::Pane, flags: CmdFindFlags(0) }, target: CommandEntryFlag { flag: b't', kind: CmdFindType::Pane, flags: CmdFindFlags(0) },
 flags: CommandFlags(0),
};
pub static CMD_START_SERVER: CommandEntry = CommandEntry {
    name: b"start-server",
    alias: Some(b"start"),
    args: ArgsParse {
        template: b"",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(1),
};
pub static CMD_SUSPEND_CLIENT: CommandEntry = CommandEntry {
    name: b"suspend-client",
    alias: Some(b"suspendc"),
    args: ArgsParse {
        template: b"t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-t target-client]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(16),
};
pub static CMD_SWAP_PANE: CommandEntry = CommandEntry {
    name: b"swap-pane",
    alias: Some(b"swapp"),
    args: ArgsParse {
        template: b"dDs:t:UZ",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-dDUZ] [-s src-pane] [-t dst-pane]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(8),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_SWAP_WINDOW: CommandEntry = CommandEntry {
    name: b"swap-window",
    alias: Some(b"swapw"),
    args: ArgsParse {
        template: b"ds:t:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-d] [-s src-window] [-t dst-window]",
    source: CommandEntryFlag {
        flag: b's',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(8),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_SWITCH_CLIENT: CommandEntry = CommandEntry {
    name: b"switch-client",
    alias: Some(b"switchc"),
    args: ArgsParse {
        template: b"c:EFlnO:pt:rT:Z",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-ElnprZ] [-c target-client] [-t target-session] [-T key-table] [-O order]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(10),
};
pub static CMD_SWITCH_MODE: CommandEntry = CommandEntry {
    name: b"switch-mode",
    alias: None,
    args: ArgsParse {
        template: b"F:kst:wZ",
        lower: 0,
        upper: 1,
        cb: Some(commands_or_string),
    },
    usage: b"[-kswZ] [-F format] [-t target-pane] [command]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_UNBIND_KEY: CommandEntry = CommandEntry {
    name: b"unbind-key",
    alias: Some(b"unbind"),
    args: ArgsParse {
        template: b"anqT:",
        lower: 0,
        upper: 1,
        cb: None,
    },
    usage: b"[-anq] [-T key-table] key",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(4),
};
pub static CMD_UNLINK_WINDOW: CommandEntry = CommandEntry {
    name: b"unlink-window",
    alias: Some(b"unlinkw"),
    args: ArgsParse {
        template: b"kt:",
        lower: 0,
        upper: 0,
        cb: None,
    },
    usage: b"[-k] [-t target-window]",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: b't',
        kind: CmdFindType::Window,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};
pub static CMD_WAIT_FOR: CommandEntry = CommandEntry {
    name: b"wait-for",
    alias: Some(b"wait"),
    args: ArgsParse {
        template: b"EF:LSUlvw:",
        lower: 1,
        upper: 1,
        cb: None,
    },
    usage: b"[-ELSUlv] [-F format] [-w waiter] name",
    source: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    target: CommandEntryFlag {
        flag: 0,
        kind: CmdFindType::Pane,
        flags: CmdFindFlags(0),
    },
    flags: CommandFlags(0),
};

pub static COMMAND_TABLE: super::CommandTable = &[
    &CMD_ATTACH_SESSION,
    &CMD_BIND_KEY,
    &CMD_BREAK_PANE,
    &CMD_CAPTURE_PANE,
    &CMD_CHOOSE_BUFFER,
    &CMD_CHOOSE_CLIENT,
    &CMD_CHOOSE_TREE,
    &CMD_CLEAR_HISTORY,
    &CMD_CLEAR_PROMPT_HISTORY,
    &CMD_CLOCK_MODE,
    &CMD_COMMAND_PROMPT,
    &CMD_CONFIRM_BEFORE,
    &CMD_COPY_MODE,
    &CMD_CUSTOMIZE_MODE,
    &CMD_DELETE_BUFFER,
    &CMD_DETACH_CLIENT,
    &CMD_DISPLAY_MENU,
    &CMD_DISPLAY_MESSAGE,
    &CMD_DISPLAY_POPUP,
    &CMD_DISPLAY_PANES,
    &CMD_FIND_WINDOW,
    &CMD_HAS_SESSION,
    &CMD_IF_SHELL,
    &CMD_JOIN_PANE,
    &CMD_KILL_PANE,
    &CMD_KILL_SERVER,
    &CMD_KILL_SESSION,
    &CMD_KILL_WINDOW,
    &CMD_LAST_PANE,
    &CMD_LAST_WINDOW,
    &CMD_LINK_WINDOW,
    &CMD_LIST_BUFFERS,
    &CMD_LIST_CLIENTS,
    &CMD_LIST_COMMANDS,
    &CMD_LIST_KEYS,
    &CMD_LIST_PANES,
    &CMD_LIST_SESSIONS,
    &CMD_LIST_WINDOWS,
    &CMD_LOAD_BUFFER,
    &CMD_LOCK_CLIENT,
    &CMD_LOCK_SERVER,
    &CMD_LOCK_SESSION,
    &CMD_MOVE_PANE,
    &CMD_MOVE_WINDOW,
    &CMD_NEW_PANE,
    &CMD_NEW_SESSION,
    &CMD_NEW_WINDOW,
    &CMD_NEXT_LAYOUT,
    &CMD_NEXT_WINDOW,
    &CMD_PASTE_BUFFER,
    &CMD_PIPE_PANE,
    &CMD_PREVIOUS_LAYOUT,
    &CMD_PREVIOUS_WINDOW,
    &CMD_REFRESH_CLIENT,
    &CMD_RENAME_SESSION,
    &CMD_RENAME_WINDOW,
    &CMD_RESIZE_PANE,
    &CMD_RESIZE_WINDOW,
    &CMD_RESPAWN_PANE,
    &CMD_RESPAWN_WINDOW,
    &CMD_ROTATE_WINDOW,
    &CMD_RUN_SHELL,
    &CMD_SAVE_BUFFER,
    &CMD_SELECT_LAYOUT,
    &CMD_SELECT_PANE,
    &CMD_SELECT_WINDOW,
    &CMD_SEND_KEYS,
    &CMD_SEND_PREFIX,
    &CMD_SERVER_ACCESS,
    &CMD_SET_BUFFER,
    &CMD_SET_ENVIRONMENT,
    &CMD_SET_HOOK,
    &CMD_SET_OPTION,
    &CMD_SET_WINDOW_OPTION,
    &CMD_SHOW_BUFFER,
    &CMD_SHOW_ENVIRONMENT,
    &CMD_SHOW_HOOKS,
    &CMD_SHOW_MESSAGES,
    &CMD_SHOW_OPTIONS,
    &CMD_SHOW_PROMPT_HISTORY,
    &CMD_SHOW_WINDOW_OPTIONS,
    &CMD_SOURCE_FILE,
    &CMD_SPLIT_WINDOW,
    &CMD_START_SERVER,
    &CMD_SUSPEND_CLIENT,
    &CMD_SWAP_PANE,
    &CMD_SWAP_WINDOW,
    &CMD_SWITCH_CLIENT,
    &CMD_SWITCH_MODE,
    &CMD_UNBIND_KEY,
    &CMD_UNLINK_WINDOW,
    &CMD_WAIT_FOR,
];
