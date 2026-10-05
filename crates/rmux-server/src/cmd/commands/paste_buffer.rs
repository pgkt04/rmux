// Ported from tmux cmd-paste-buffer.c @ 8f25579c
use super::support::{concat, fail, item_target};
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::QueueItemId;
use crate::model::PaneFlags;
use crate::model::pane::pane_exited;
use crate::model::paste::{paste_get_name, paste_get_top, paste_remove};
use crate::server::Server;
use rmux_emu::screen::ScreenMode;
use rmux_util::{utf8, vis::VisFlags};

/// cmd-paste-buffer.c:46-55 (`cmd_paste_buffer_paste`): escape one line into
/// the reused `scratch` buffer and append it to the pane output.
fn paste(output: &mut Vec<u8>, line: &[u8], scratch: &mut Vec<u8>) {
    scratch.clear();
    utf8::strvis(scratch, line, VisFlags::SAFE | VisFlags::NOSLASH);
    output.extend_from_slice(scratch);
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let bracket = args.has(b'p') != 0;
    let target = item_target(server, item);
    let Some(wp) = target.wp.filter(|&wp| !pane_exited(server, wp)) else {
        return fail(server, item, b"target pane has exited");
    };

    let pb = match args.get(b'b') {
        None => paste_get_top(server),
        Some(bufname) => match paste_get_name(server, bufname) {
            Some(pb) => Some(pb),
            None => return fail(server, item, concat(&[b"no buffer ", bufname])),
        },
    };

    if let Some(pb) = pb
        && let (Some(buffer), Some(pane)) = (server.paste.get(pb), server.panes.get_mut(wp))
        && !pane.flags.contains(PaneFlags::INPUTOFF)
    {
        let sepstr: &[u8] = match args.get(b's') {
            Some(sep) => sep,
            None if args.has(b'r') != 0 => b"\n",
            None => b"\r",
        };
        let raw = args.has(b'S') != 0;
        let bracketed = bracket
            && pane
                .displayed_screen()
                .mode
                .contains(ScreenMode::BRACKETPASTE);
        let start = pane.output.len();
        if bracketed {
            pane.output.extend_from_slice(b"\x1b[200~");
        }
        let mut scratch = Vec::new();
        let mut bufdata: &[u8] = &buffer.data;
        while let Some(at) = bufdata.iter().position(|&b| b == b'\n') {
            let line = &bufdata[..at];
            if raw {
                pane.output.extend_from_slice(line);
            } else {
                paste(&mut pane.output, line, &mut scratch);
            }
            pane.output.extend_from_slice(sepstr);
            bufdata = &bufdata[at + 1..];
        }
        if !bufdata.is_empty() {
            if raw {
                pane.output.extend_from_slice(bufdata);
            } else {
                paste(&mut pane.output, bufdata, &mut scratch);
            }
        }
        if bracketed {
            pane.output.extend_from_slice(b"\x1b[201~");
        }
        let client = server.queue.items.get(item).and_then(|item| item.client);
        crate::model::pane::hold_encoded_input(server, wp, client, start);
    }

    if let Some(pb) = pb
        && args.has(b'd') != 0
        && let Err(e) = paste_remove(server, pb)
    {
        return fail(server, item, e.to_string().as_bytes());
    }
    CmdReturn::Normal
}
