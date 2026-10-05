// Ported from tmux cfg.c @ 8f25579c
/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
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

use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::rc::Rc;

use rmux_util::bytes::ByteString;

use super::CommandList;
use super::find::CmdFindState;
use super::parse::{self, CmdParseFlags, CmdParseInput, ParseContext};
use super::queue::{CmdReturn, QueueBatch};
use crate::ids::{ClientId, PaneId, QueueItemId, QueueStateId, SessionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CfgCallback {
    ClientDone,
    Done,
}

/// Boundaries implemented by the server, client, model, and queue owners.
pub trait CfgRuntime: ParseContext {
    fn first_client(&self) -> Option<ClientId>;
    fn client_dead(&self, client: ClientId) -> bool;
    fn client_control(&self, client: ClientId) -> bool;
    fn client_session(&self, client: ClientId) -> Option<SessionId>;
    fn item_client(&self, item: QueueItemId) -> Option<ClientId>;
    fn first_session_by_name(&self) -> Option<SessionId>;
    fn session_attached(&self, session: SessionId) -> bool;
    fn session_active_pane(&self, session: SessionId) -> PaneId;
    fn pane_top_is_view(&self, pane: PaneId) -> bool;
    fn enter_view_mode(&mut self, pane: PaneId);
    fn append_view_line(&mut self, pane: PaneId, line: &[u8]);
    fn notify_config_error(&mut self, client: ClientId, cause: &[u8]);
    fn print_cfg_cause(&mut self, item: QueueItemId, cause: &[u8]);
    fn load_prompt_history(&mut self);
    fn append_cfg_callback(
        &mut self,
        client: Option<ClientId>,
        callback: CfgCallback,
    ) -> QueueItemId;
    fn continue_cfg_item(&mut self, item: QueueItemId);
    fn new_cfg_state(&mut self) -> QueueStateId;
    /// Copy flags/event/target, overriding target when supplied, but never copy extra formats.
    fn copy_cfg_state(&mut self, item: QueueItemId, current: Option<&CmdFindState>)
    -> QueueStateId;
    fn add_cfg_format(&mut self, state: QueueStateId, name: &[u8], value: &[u8]);
    fn cfg_commands(&mut self, list: Rc<CommandList>, state: QueueStateId) -> QueueBatch;
    fn free_cfg_state(&mut self, state: QueueStateId);
    fn append_cfg_commands(&mut self, batch: QueueBatch) -> Option<QueueItemId>;
    fn insert_cfg_commands(&mut self, item: QueueItemId, batch: QueueBatch) -> Option<QueueItemId>;
}

pub struct CfgState {
    pub files: Vec<ByteString>,
    pub quiet: bool,
    pub started: bool,
    pub finished: bool,
    pub client: Option<ClientId>,
    pub item: Option<QueueItemId>,
    pub causes: Vec<ByteString>,
}

impl Default for CfgState {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            quiet: true,
            started: false,
            finished: false,
            client: None,
            item: None,
            causes: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CfgLoadError;

impl CfgState {
    pub fn client_done(&self, runtime: &impl CfgRuntime, client: ClientId) -> CmdReturn {
        if runtime.client_dead(client) || self.finished {
            CmdReturn::Normal
        } else {
            CmdReturn::Wait
        }
    }

    pub fn done(&mut self, runtime: &mut impl CfgRuntime) -> CmdReturn {
        if self.finished {
            return CmdReturn::Normal;
        }
        self.finished = true;
        self.show_causes(runtime, None);
        if let Some(item) = self.item.take() {
            runtime.continue_cfg_item(item);
        }
        runtime.load_prompt_history();
        CmdReturn::Normal
    }

    pub fn client_lost(&mut self, runtime: &mut impl CfgRuntime, client: ClientId) {
        if self.client != Some(client) {
            return;
        }
        self.client = None;
        if let Some(item) = self.item.take() {
            runtime.continue_cfg_item(item);
        }
    }

    /// Called only for the first eligible identifying client while config is unfinished.
    pub fn start_cfg(&mut self, runtime: &mut impl CfgRuntime) {
        self.client = runtime.first_client();
        if let Some(client) = self.client {
            self.item = Some(runtime.append_cfg_callback(Some(client), CfgCallback::ClientDone));
        }
        if self.started {
            return;
        }
        self.started = true;
        let flags = if self.quiet {
            CmdParseFlags::QUIET
        } else {
            CmdParseFlags::default()
        };
        let files = std::mem::take(&mut self.files);
        for path in &files {
            let path = Path::new(std::ffi::OsStr::from_bytes(path.as_ref()));
            let _ = self.load_cfg(runtime, path, self.client, None, None, flags);
        }
        self.files = files;
        runtime.append_cfg_callback(None, CfgCallback::Done);
    }

    pub fn load_cfg(
        &mut self,
        runtime: &mut impl CfgRuntime,
        path: &Path,
        client: Option<ClientId>,
        item: Option<QueueItemId>,
        current: Option<&CmdFindState>,
        flags: CmdParseFlags,
    ) -> Result<Option<QueueItemId>, CfgLoadError> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound
                    && flags.contains(CmdParseFlags::QUIET)
                {
                    return Ok(None);
                }
                let mut cause = ByteString::from(path.as_os_str().as_bytes());
                cause.extend_from_slice(b": ");
                match error.raw_os_error() {
                    Some(errno) => cause.extend_from_slice(&rmux_sys::errno::strerror(errno)),
                    None => cause.extend_from_slice(error.to_string().as_bytes()),
                }
                self.add_cause(cause);
                return Err(CfgLoadError);
            }
        };
        let mut input = Self::parse_input(path.as_os_str().as_bytes(), client, item, flags);
        let result = parse::from_file(runtime, &mut file, &mut input);
        self.loaded(runtime, result, &input, item, current, flags)
    }

    #[allow(clippy::too_many_arguments)] // Preserve the public load_cfg_from_buffer boundary.
    pub fn load_cfg_from_buffer(
        &mut self,
        runtime: &mut impl CfgRuntime,
        buffer: &[u8],
        path: &[u8],
        client: Option<ClientId>,
        item: Option<QueueItemId>,
        current: Option<&CmdFindState>,
        flags: CmdParseFlags,
    ) -> Result<Option<QueueItemId>, CfgLoadError> {
        let mut input = Self::parse_input(path, client, item, flags);
        let result = parse::from_buffer(runtime, buffer, &mut input);
        self.loaded(runtime, result, &input, item, current, flags)
    }

    fn parse_input(
        path: &[u8],
        client: Option<ClientId>,
        item: Option<QueueItemId>,
        flags: CmdParseFlags,
    ) -> CmdParseInput {
        CmdParseInput {
            flags,
            file: Some(path.into()),
            line: 1,
            client,
            item,
            ..CmdParseInput::default()
        }
    }

    fn loaded(
        &mut self,
        runtime: &mut impl CfgRuntime,
        result: parse::CmdParseResult,
        input: &CmdParseInput,
        item: Option<QueueItemId>,
        current: Option<&CmdFindState>,
        flags: CmdParseFlags,
    ) -> Result<Option<QueueItemId>, CfgLoadError> {
        let list = match result {
            Ok(list) => list,
            Err(error) => {
                self.add_cause(error.message().into());
                return Err(CfgLoadError);
            }
        };
        if flags.contains(CmdParseFlags::PARSEONLY) {
            return Ok(None);
        }
        let state = match item {
            Some(item) => runtime.copy_cfg_state(item, current),
            None => runtime.new_cfg_state(),
        };
        runtime.add_cfg_format(
            state,
            b"current_file",
            input
                .file
                .as_ref()
                .expect("configuration file name")
                .as_ref(),
        );
        let batch = runtime.cfg_commands(list, state);
        let last = match item {
            Some(item) => runtime.insert_cfg_commands(item, batch),
            None => runtime.append_cfg_commands(batch),
        };
        runtime.free_cfg_state(state);
        Ok(last)
    }

    pub fn add_cause(&mut self, message: ByteString) {
        self.causes.push(message);
    }

    pub fn print_causes(&mut self, runtime: &mut impl CfgRuntime, item: QueueItemId) {
        let client = runtime
            .item_client(item)
            .filter(|c| runtime.client_control(*c));
        for cause in self.causes.drain(..) {
            if let Some(client) = client {
                runtime.notify_config_error(client, cause.as_ref());
            } else {
                runtime.print_cfg_cause(item, cause.as_ref());
            }
        }
    }

    pub fn show_causes(&mut self, runtime: &mut impl CfgRuntime, session: Option<SessionId>) {
        if self.causes.is_empty() {
            return;
        }
        let client = runtime.first_client();
        if let Some(client) = client.filter(|c| runtime.client_control(*c)) {
            for cause in self.causes.drain(..) {
                runtime.notify_config_error(client, cause.as_ref());
            }
            return;
        }
        let session = session
            .or_else(|| client.and_then(|c| runtime.client_session(c)))
            .or_else(|| runtime.first_session_by_name());
        let Some(session) = session.filter(|s| runtime.session_attached(*s)) else {
            return;
        };
        let pane = runtime.session_active_pane(session);
        if !runtime.pane_top_is_view(pane) {
            runtime.enter_view_mode(pane);
        }
        for cause in self.causes.drain(..) {
            runtime.append_view_line(pane, cause.as_ref());
        }
    }
}

#[cfg(test)]
#[path = "key_bindings/cfg_tests.rs"]
mod tests;
