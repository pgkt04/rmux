// Ported from tmux cmd-find-window.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        arguments::{Args, ArgsEntryFlags, ArgsValue},
        queue::CmdReturn,
    },
    ids::QueueItemId,
    modes::{self, CommandModeRequest},
    server::Server,
};

fn term(kind: u8, suffix: &[u8], pattern: &[u8], stars: bool, field: &[u8]) -> Vec<u8> {
    let mut out = b"#{".to_vec();
    out.push(kind);
    out.extend_from_slice(suffix);
    out.push(b':');
    if stars {
        out.push(b'*');
    }
    out.extend_from_slice(pattern);
    if stars {
        out.push(b'*');
    }
    if !field.is_empty() {
        out.extend_from_slice(b",#{");
        out.extend_from_slice(field);
        out.push(b'}');
    }
    out.push(b'}');
    out
}
fn either(left: Vec<u8>, right: Vec<u8>) -> Vec<u8> {
    let mut out = b"#{||:".to_vec();
    out.extend_from_slice(&left);
    out.push(b',');
    out.extend_from_slice(&right);
    out.push(b'}');
    out
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let mut content = args.has(b'C') != 0;
    let mut name = args.has(b'N') != 0;
    let mut title = args.has(b'T') != 0;
    if !content && !name && !title {
        content = true;
        name = true;
        title = true;
    }
    let suffix: &[u8] = match (args.has(b'r') != 0, args.has(b'i') != 0) {
        (true, true) => b"/ri",
        (true, false) => b"/r",
        (false, true) => b"/i",
        _ => b"",
    };
    let pattern = args.string(0).unwrap_or_default();
    let mut terms = Vec::new();
    if content {
        terms.push(term(b'C', suffix, pattern, false, b""));
    }
    if name {
        terms.push(term(
            b'm',
            suffix,
            pattern,
            args.has(b'r') == 0,
            b"window_name",
        ));
    }
    if title {
        terms.push(term(
            b'm',
            suffix,
            pattern,
            args.has(b'r') == 0,
            b"pane_title",
        ));
    }
    let filter = terms
        .into_iter()
        .rev()
        .reduce(|right, left| either(left, right))
        .unwrap_or_default();
    let mut new_args = Args::create();
    new_args.set(
        b'f',
        Some(ArgsValue::string(filter.into())),
        ArgsEntryFlags::default(),
    );
    if args.has(b'Z') != 0 {
        new_args.set(b'Z', None, ArgsEntryFlags::default());
    }
    let Some(queued) = server.queue.items.get(item) else {
        return CmdReturn::Error;
    };
    let request = CommandModeRequest {
        command: b"choose-tree".as_slice().into(),
        args: new_args,
        target: queued.target,
        source: queued.source,
        client: queued.client,
        item,
    };
    modes::run_mode_command(server, request)
}

#[cfg(test)]
mod tests {
    #[test]
    fn content_search_has_no_glob_stars() {
        assert_eq!(super::term(b'C', b"/i", b"x", false, b""), b"#{C/i:x}");
        assert_eq!(
            super::term(b'm', b"", b"x", true, b"window_name"),
            b"#{m:*x*,#{window_name}}"
        );
    }
}
