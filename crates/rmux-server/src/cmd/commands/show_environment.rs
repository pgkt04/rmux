// Ported from tmux cmd-show-environment.c @ 8f25579c
use super::support::{concat, fail, item_target};
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::ids::QueueItemId;
use crate::options::environment::{Environment, EnvironmentEntry, EnvironmentFlags};
use crate::server::Server;
use rmux_util::bytes::cstr;

/// cmd-show-environment.c:50-66 (`cmd_show_environment_escape`): backslash
/// the bytes POSIX interprets inside double quotes.
pub fn escape(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 2);
    for &c in cstr(value) {
        if c == b'$' || c == b'`' || c == b'"' || c == b'\\' {
            out.push(b'\\');
        }
        out.push(c);
    }
    out
}

/// cmd-show-environment.c:68-96 (`cmd_show_environment_print`): the printed
/// line for one entry, or `None` when the hidden filter drops it.
fn line(hidden: bool, shell: bool, name: &[u8], entry: &EnvironmentEntry) -> Option<Vec<u8>> {
    let is_hidden = entry.flags.contains(EnvironmentFlags::HIDDEN);
    if hidden != is_hidden {
        return None;
    }
    let value = entry.value.as_deref().map(|v| cstr(v));
    Some(match (shell, value) {
        (false, Some(value)) => concat(&[name, b"=", value]),
        (false, None) => concat(&[b"-", name]),
        (true, Some(value)) => concat(&[name, b"=\"", &escape(value), b"\"; export ", name, b";"]),
        (true, None) => concat(&[b"unset ", name, b";"]),
    })
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let name = args.string(0);
    let hidden = args.has(b'h') != 0;
    let shell = args.has(b's') != 0;

    if let Some(tflag) = args.get(b't')
        && target.s.is_none()
    {
        return fail(server, item, concat(&[b"no such session: ", tflag]));
    }
    let env: &Environment = if args.has(b'g') != 0 {
        &server.global_environment
    } else {
        let session = target.s.and_then(|s| server.sessions.get(s));
        let Some(session) = session else {
            return match args.get(b't') {
                Some(tflag) => fail(server, item, concat(&[b"no such session: ", tflag])),
                None => fail(server, item, b"no current session"),
            };
        };
        &session.environment
    };

    let lines: Vec<Vec<u8>> = if let Some(name) = name {
        let Some(entry) = env.find(name) else {
            return fail(server, item, concat(&[b"unknown variable: ", name]));
        };
        line(hidden, shell, name, entry).into_iter().collect()
    } else {
        env.iter()
            .filter_map(|(name, entry)| line(hidden, shell, name, entry))
            .collect()
    };
    for text in &lines {
        queue::print(server, item, text);
    }
    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::escape;

    /// Spec section 6 unit test 2: the four escaped bytes, others unchanged.
    #[test]
    fn escape_backslashes_dollar_backquote_dquote_and_backslash() {
        assert_eq!(escape(b"$"), b"\\$");
        assert_eq!(escape(b"`"), b"\\`");
        assert_eq!(escape(b"\""), b"\\\"");
        assert_eq!(escape(b"\\"), b"\\\\");
        assert_eq!(escape(b"a$b`c\"d\\e"), b"a\\$b\\`c\\\"d\\\\e");
    }

    #[test]
    fn escape_leaves_other_bytes_unchanged() {
        assert_eq!(escape(b""), b"");
        assert_eq!(
            escape(b"plain 'text' {x} #!~\n\t"),
            b"plain 'text' {x} #!~\n\t"
        );
        assert_eq!(escape(b"\xff\xfe\x80"), b"\xff\xfe\x80");
        assert_eq!(escape(b"abc\0$dropped"), b"abc");
    }
}
