// Ported from tmux cmd-source-file.c @ 8f25579c
use super::support::{concat, fail, item_client, item_target};
use crate::client::ClientFlags;
use crate::cmd::Command;
use crate::cmd::cfg::CfgState;
use crate::cmd::parse::CmdParseFlags;
use crate::cmd::queue::{self, CmdReturn};
use crate::format;
use crate::ids::{ClientId, QueueItemId};
use crate::server::Server;
use crate::server::file::{self, FileNotice};
use rmux_sys::server::GlobError;
use rmux_util::bytes::cstr;

/// `CMD_SOURCE_FILE_DEPTH_LIMIT` (`cmd-source-file.c:33`).
pub const DEPTH_LIMIT: u32 = 50;

/// `struct cmd_source_file_data` (`cmd-source-file.c:50-61`). Owned by the
/// pending read callback, then by the completion callback item.
pub struct SourceFileState {
    pub item: QueueItemId,
    pub client: Option<ClientId>,
    pub flags: CmdParseFlags,
    pub after: QueueItemId,
    pub retval: CmdReturn,
    pub files: Vec<Vec<u8>>,
    pub index: usize,
}

/// `cmd_source_file_quote_for_glob` (`cmd-source-file.c:159-172`): backslash
/// before every ASCII byte that is neither alphanumeric nor `/`.
pub fn quote_for_glob(path: &[u8]) -> Vec<u8> {
    let path = cstr(path);
    let mut quoted = Vec::with_capacity(2 * path.len());
    for &b in path {
        if b < 128 && !b.is_ascii_alphanumeric() && b != b'/' {
            quoted.push(b'\\');
        }
        quoted.push(b);
    }
    quoted
}

/// `cmd-source-file.c:241-249`: the errno reported for a glob failure.
pub fn glob_errno(error: GlobError) -> i32 {
    match error {
        GlobError::NoMatch => libc::ENOENT,
        GlobError::NoSpace => libc::ENOMEM,
        GlobError::Other => libc::EINVAL,
    }
}

/// `strerror(error): path` (`cmd-source-file.c:128, 250`).
fn errno_message(error: i32, path: &[u8]) -> Vec<u8> {
    concat(&[&rmux_sys::strerror(error), b": ", cstr(path)])
}

impl SourceFileState {
    /// `cmd_source_file_free_data` (`cmd-source-file.c:63-74`).
    fn free(self, server: &mut Server) {
        if let Some(c) = self.client {
            let _ = crate::client::lifecycle::release(server, c);
        }
    }

    /// `cmd_source_file_complete_cb` (`cmd-source-file.c:76-94`).
    fn complete_cb(self, server: &mut Server, item: QueueItemId) -> CmdReturn {
        match self.client.and_then(|c| server.clients.get_mut(c)) {
            None => server.source_file_depth = server.source_file_depth.saturating_sub(1),
            Some(client) => client.source_file_depth = client.source_file_depth.saturating_sub(1),
        }
        let mut causes = CfgState {
            causes: std::mem::take(&mut server.cfg.causes),
            ..CfgState::default()
        };
        causes.print_causes(server, item);
        server.cfg.causes.extend(causes.causes);
        self.free(server);
        CmdReturn::Normal
    }

    /// `cmd_source_file_complete` (`cmd-source-file.c:96-113`).
    fn complete(self, server: &mut Server) {
        if !server.cfg.finished {
            self.free(server);
            return;
        }
        if self.retval == CmdReturn::Error
            && let Some(client) = self.client.and_then(|c| server.clients.get_mut(c))
            && client.session.is_none()
        {
            client.retval = 1;
        }
        let after = self.after;
        let Ok(batch) = server.queue.get_callback(
            "cmd_source_file_complete_cb",
            queue::callback_for::<Server>(move |server, item| self.complete_cb(server, item)),
        ) else {
            return;
        };
        let _ = queue::insert_after(server, after, batch);
    }

    /// `file_read(c, cdata->files[n], cmd_source_file_done, cdata)`
    /// (`cmd-source-file.c:142, 267`). Acts once, on `Done` only.
    fn read_current(self, server: &mut Server) {
        let client = self.client;
        let path = self.files[self.index].clone();
        let mut slot = Some(self);
        file::read(
            server,
            client,
            &path,
            Box::new(move |server, _file, notice, data, error| {
                if notice != FileNotice::Done {
                    return;
                }
                if let Some(state) = slot.take() {
                    state.done(server, data, error);
                }
            }),
        );
    }

    /// `cmd_source_file_done` (`cmd-source-file.c:115-148`). Read errors only
    /// print; a configuration load failure sets `retval`.
    fn done(mut self, server: &mut Server, data: &[u8], error: i32) {
        let item = self.item;
        if error != 0 {
            let message = errno_message(error, &self.files[self.index]);
            queue::error(server, item, &message);
        } else if !data.is_empty() {
            let target = item_target(server, item);
            let mut cfg = CfgState::default();
            let result = cfg.load_cfg_from_buffer(
                server,
                data,
                &self.files[self.index],
                self.client,
                Some(self.after),
                Some(&target),
                self.flags,
            );
            server.cfg.causes.extend(cfg.causes);
            match result {
                Err(_) => self.retval = CmdReturn::Error,
                Ok(Some(new_item)) => self.after = new_item,
                Ok(None) => {}
            }
        }
        self.index += 1;
        if self.index < self.files.len() {
            self.read_current(server);
        } else {
            self.complete(server);
            queue::continue_item(&mut server.queue, item);
        }
    }
}

/// `cmd_source_file_exec` (`cmd-source-file.c:174-273`).
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let c = item_client(server, item);
    let mut retval = CmdReturn::Normal;

    match c.and_then(|c| server.clients.get_mut(c)) {
        None => {
            if server.source_file_depth >= DEPTH_LIMIT {
                return fail(server, item, b"too many nested files");
            }
            server.source_file_depth += 1;
        }
        Some(client) => {
            if client.source_file_depth >= DEPTH_LIMIT {
                return fail(server, item, b"too many nested files");
            }
            client.source_file_depth += 1;
        }
    }

    let client = c.filter(|c| crate::client::lifecycle::retain(server, *c).is_ok());

    let mut flags = CmdParseFlags::default();
    if args.has(b'q') != 0 {
        flags.insert(CmdParseFlags::QUIET);
    }
    if args.has(b'n') != 0 {
        flags.insert(CmdParseFlags::PARSEONLY);
    }
    let control = c
        .and_then(|c| server.clients.get(c))
        .is_some_and(|c| c.flags.contains(ClientFlags::CONTROL));
    if !control && (args.has(b'v') != 0 || command.parse_flags.contains(CmdParseFlags::VERBOSE)) {
        flags.insert(CmdParseFlags::VERBOSE);
    }

    let cwd = quote_for_glob(&crate::client::registry::get_cwd(server, c, None));

    let mut files: Vec<Vec<u8>> = Vec::new();
    for i in 0..args.count() {
        let raw = args.string(i).unwrap_or(b"");
        let expanded;
        let path: &[u8] = if args.has(b'F') != 0 {
            expanded = format::single_from_target(server, item, raw);
            cstr(&expanded)
        } else {
            raw
        };
        if path == b"-" {
            files.push(b"-".to_vec());
            continue;
        }
        let pattern = if path.first() == Some(&b'/') {
            path.to_vec()
        } else {
            concat(&[&cwd, b"/", path])
        };
        match rmux_sys::server::glob(&pattern) {
            Err(error) => {
                if error != GlobError::NoMatch || !flags.contains(CmdParseFlags::QUIET) {
                    let message = errno_message(glob_errno(error), path);
                    queue::error(server, item, &message);
                    retval = CmdReturn::Error;
                }
            }
            Ok(matches) => files.extend(matches),
        }
    }

    let state = SourceFileState {
        item,
        client,
        flags,
        after: item,
        retval,
        files,
        index: 0,
    };
    if !state.files.is_empty() {
        state.read_current(server);
        return CmdReturn::Wait;
    }
    state.complete(server);
    retval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_for_glob_escapes_ascii_punctuation_only() {
        assert_eq!(
            quote_for_glob(b"/home/j/a b*[x]?-_.txt"),
            b"/home/j/a\\ b\\*\\[x\\]\\?\\-\\_\\.txt"
        );
        assert_eq!(quote_for_glob(b"/plain/Path09"), b"/plain/Path09");
        assert_eq!(
            quote_for_glob("/caf\u{e9}/\u{4e2d}".as_bytes()),
            "/caf\u{e9}/\u{4e2d}".as_bytes()
        );
        assert_eq!(quote_for_glob(b"a\\b\0ignored"), b"a\\\\b");
        assert_eq!(quote_for_glob(b""), b"");
    }

    #[test]
    fn glob_errors_map_to_errno() {
        assert_eq!(glob_errno(GlobError::NoMatch), libc::ENOENT);
        assert_eq!(glob_errno(GlobError::NoSpace), libc::ENOMEM);
        assert_eq!(glob_errno(GlobError::Other), libc::EINVAL);
    }
}
