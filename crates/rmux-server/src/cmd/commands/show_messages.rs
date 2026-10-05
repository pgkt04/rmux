// Ported from tmux cmd-show-messages.c @ 8f25579c
use super::support::item_target_client;
use crate::cmd::Command;
use crate::cmd::queue::{self, CmdReturn};
use crate::format;
use crate::ids::{ClientId, QueueItemId};
use crate::server::Server;
use rmux_tty::term::TtyCodeCode;
use rmux_tty::tty::TtyFlags;
use rmux_util::time::Timestamp;

const SHOW_MESSAGES_TEMPLATE: &[u8] = b"#{t/p:message_time}: #{message_text}";

/// `LIST_FOREACH(term, &tty_terms, entry)` (`tty-term.c:572-576` inserts at
/// the head): clients with an open terminal, newest open first.
fn terminals(server: &Server) -> Vec<ClientId> {
    let mut found: Vec<(u64, ClientId)> = server
        .client_order
        .iter()
        .copied()
        .filter_map(|id| {
            let tty = server.clients.get(id)?.tty.as_ref()?;
            if !tty.flags().contains(TtyFlags::OPENED) {
                return None;
            }
            Some((tty.term().open_serial(), id))
        })
        .collect();
    found.sort_by_key(|b| std::cmp::Reverse(b.0));
    found.into_iter().map(|(_, id)| id).collect()
}

/// `cmd_show_messages_terminals` (`cmd-show-messages.c:48-71`); true when any
/// terminal was printed.
fn show_terminals(
    server: &mut Server,
    command: &Command,
    item: QueueItemId,
    mut blank: bool,
) -> bool {
    let tc = item_target_client(server, item);
    let filter = (command.args.has(b't') != 0).then_some(tc).flatten();
    let mut n: u32 = 0;
    for id in terminals(server) {
        if filter.is_some_and(|tc| tc != id) {
            continue;
        }
        let mut lines: Vec<Vec<u8>> = Vec::with_capacity(TtyCodeCode::COUNT + 1);
        {
            let Some(c) = server.clients.get(id) else {
                continue;
            };
            let Some(tty) = c.tty.as_ref() else {
                continue;
            };
            let term = tty.term();
            let mut heading = format!("Terminal {n}: ").into_bytes();
            heading.extend_from_slice(term.name());
            heading.extend_from_slice(b" for ");
            heading.extend_from_slice(c.name_bytes());
            heading.extend_from_slice(format!(", flags=0x{:x}:", term.flags().bits()).as_bytes());
            lines.push(heading);
            for code in 0..TtyCodeCode::COUNT {
                if let Ok(code) = TtyCodeCode::try_from(code as i32) {
                    lines.push(term.describe(code).into_bytes());
                }
            }
        }
        if blank {
            queue::print(server, item, b"");
            blank = false;
        }
        n += 1;
        for line in lines {
            queue::print(server, item, &line);
        }
    }
    n != 0
}

/// `cmd_show_messages_exec` (`cmd-show-messages.c:73-107`).
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let mut done = false;
    let mut blank = false;
    if args.has(b'T') != 0 {
        blank = show_terminals(server, command, item, blank);
        done = true;
    }
    if args.has(b'J') != 0 {
        for line in server.jobs.print_summary(blank) {
            queue::print(server, item, &line);
        }
        done = true;
    }
    if done {
        return CmdReturn::Normal;
    }

    let mut tree = format::create_from_target(server, item);
    let messages: Vec<((i64, i64), u32, Vec<u8>)> = server
        .message_log
        .iter()
        .rev()
        .map(|m| (m.msg_time, m.msg_num, m.msg.clone()))
        .collect();
    for (time, number, text) in messages {
        tree.add(b"message_text", text.into());
        tree.add(b"message_number", number.to_string().into());
        tree.add_time(b"message_time", Timestamp::new(time.0, time.1 as i32));
        let s = tree.expand(server, SHOW_MESSAGES_TEMPLATE);
        queue::print(server, item, &s);
    }
    tree.release(server);
    CmdReturn::Normal
}
