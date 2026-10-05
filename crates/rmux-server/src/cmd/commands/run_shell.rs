// Ported from tmux cmd-run-shell.c @ 8f25579c
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use super::support::{concat, fail, item_client, item_target, item_target_client};
use crate::cmd::Command;
use crate::cmd::arguments::{
    ArgsCommandState, make_commands, make_commands_free, make_commands_prepare,
};
use crate::cmd::cfg::CfgRuntime;
use crate::cmd::find::{self, CmdFindFlags};
use crate::cmd::queue::{self, CmdReturn, QueueRuntime};
use crate::format;
use crate::ids::{ClientId, JobId, PaneId, QueueItemId, SessionId};
use crate::model::pane::{CfgModelRuntime, pane_find_by_public_id};
use crate::model::session::{session_release, session_retain};
use crate::server::Server;
use crate::server::job::{self, JobCommand, JobFlags, JobLaunch};
use rmux_util::bytes::cstr;

/// `cmd->cmd` / `cmd->state` of `struct cmd_run_shell_data` (`cmd-run-shell.c:57-67`).
pub enum RunShellCommand {
    None,
    Shell(Vec<u8>),
    Commands(ArgsCommandState),
}

/// `struct cmd_run_shell_data` (`cmd-run-shell.c:57-67`). Owned by the deferred
/// timer closure, then by the job completion closure; `free` releases the leases.
pub struct RunShellState {
    pub client: Option<ClientId>,
    pub command: RunShellCommand,
    pub cwd: Option<Vec<u8>>,
    pub item: Option<QueueItemId>,
    pub session: Option<SessionId>,
    /// Pane public id, -1 when none (`wp_id`).
    pub pane_id: i32,
    pub flags: JobFlags,
}

/// `cmd-run-shell.c:126-131`: `strtod` with trailing text rejected.
pub fn parse_delay(delay: &[u8]) -> Option<f64> {
    let delay = cstr(delay);
    let (value, consumed) = rmux_sys::number::strtod(delay);
    (consumed == delay.len()).then_some(value)
}

/// `cmd-run-shell.c:180-182`: seconds and microseconds of the delay; invalid
/// (negative, NaN, overflowing) values fire at once instead of crashing.
fn delay_duration(delay: f64) -> Duration {
    Duration::try_from_secs_f64(delay).unwrap_or(Duration::ZERO)
}

/// `cmd-run-shell.c:271-285`: exit message and return code for a wait status.
pub fn exit_message(cmd: &[u8], status: i32) -> (Option<Vec<u8>>, i32) {
    if let Some(code) = rmux_sys::proc::wait_exit_status(status) {
        let msg = (code != 0)
            .then(|| concat(&[b"'", cstr(cmd), b"' returned ", code.to_string().as_bytes()]));
        (msg, code)
    } else if let Some(signal) = rmux_sys::server::status_signal(status) {
        let msg = concat(&[
            b"'",
            cstr(cmd),
            b"' terminated by signal ",
            signal.to_string().as_bytes(),
        ]);
        (Some(msg), signal + 128)
    } else {
        (None, 0)
    }
}

/// `cmd-run-shell.c:258-269`: LF-terminated lines, then the unterminated rest.
pub fn split_output(input: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = input;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        match rest.iter().position(|b| *b == b'\n') {
            Some(at) => {
                let line = &rest[..at];
                rest = &rest[at + 1..];
                Some(line)
            }
            None => Some(std::mem::take(&mut rest)),
        }
    })
}

impl RunShellState {
    /// `cmd_run_shell_print` (`cmd-run-shell.c:78-106`).
    fn print(&self, server: &mut Server, msg: &[u8]) {
        let msg = cstr(msg);
        let mut wp: Option<PaneId> = None;
        if self.pane_id != -1 {
            wp = pane_find_by_public_id(server, self.pane_id as u32);
        }
        if wp.is_none() {
            if let Some(item) = self.item {
                queue::print(server, item, msg);
                return;
            }
            if let Some(s) = self
                .client
                .and_then(|c| server.clients.get(c))
                .and_then(|c| c.session)
            {
                wp = server
                    .sessions
                    .get(s)
                    .and_then(|s| s.current)
                    .and_then(|wl| server.winlinks.get(wl))
                    .and_then(|wl| server.windows.get(wl.window))
                    .and_then(|w| w.active);
            }
            if wp.is_none() {
                wp = find::from_nothing(&*server, CmdFindFlags::default()).and_then(|fs| fs.wp);
            }
        }
        let Some(wp) = wp else {
            return;
        };
        let mut view = CfgModelRuntime::pane_top_is_view(server, wp);
        if !view {
            view = CfgRuntime::enter_view_mode(server, wp).is_ok();
        }
        if !view || crate::modes::copy::view::add(server, wp, true, msg).is_err() {
            CfgRuntime::print_cfg_fallback(server, self.client, msg);
        }
    }

    /// `cmd_run_shell_timer` (`cmd-run-shell.c:192-246`).
    fn fire(mut self, server: &mut Server) {
        match &self.command {
            RunShellCommand::None => {
                if let Some(item) = self.item {
                    queue::continue_item(&mut server.queue, item);
                }
                self.free(server);
            }
            RunShellCommand::Shell(cmd) => {
                let mut launch = JobLaunch::new(JobCommand::Shell(cmd.clone()));
                launch.cwd = self.cwd.take();
                launch.session = self.session;
                launch.flags = self.flags;
                let slot: Rc<Cell<Option<RunShellState>>> = Rc::new(Cell::new(None));
                let owned = Rc::clone(&slot);
                launch.complete = Some(Box::new(move |server, job| {
                    if let Some(state) = owned.take() {
                        state.job_done(server, job);
                    }
                }));
                slot.set(Some(self));
                if job::run(server, launch).is_err() {
                    let Some(state) = slot.take() else {
                        return;
                    };
                    let RunShellCommand::Shell(cmd) = &state.command else {
                        return;
                    };
                    let msg = concat(&[b"failed to run command: ", cmd]);
                    match state.item {
                        None => {
                            if let Some(c) = state.client {
                                QueueRuntime::status_message(server, c, &msg);
                            }
                        }
                        Some(item) => {
                            queue::error(server, item, &msg);
                            queue::continue_item(&mut server.queue, item);
                        }
                    }
                    state.free(server);
                }
            }
            RunShellCommand::Commands(state) => {
                match make_commands(server, state, &[]) {
                    Err(error) => match self.item {
                        None => {
                            let mut message = error.message().to_vec();
                            if let Some(first) = message.first_mut() {
                                *first = QueueRuntime::uppercase(server, *first);
                            }
                            if let Some(c) = self.client {
                                QueueRuntime::status_message(server, c, &message);
                            }
                        }
                        Some(item) => queue::error(server, item, error.message()),
                    },
                    Ok(list) => match self.item {
                        None => {
                            if let Ok(batch) = server.queue.get_command(list, None) {
                                let _ = queue::append(server, self.client, batch);
                            }
                        }
                        Some(item) => {
                            let state = server.queue.items.get(item).map(|i| i.state);
                            if let Ok(batch) = server.queue.get_command(list, state) {
                                let _ = queue::insert_after(server, item, batch);
                            }
                        }
                    },
                }
                if let Some(item) = self.item {
                    queue::continue_item(&mut server.queue, item);
                }
                self.free(server);
            }
        }
    }

    /// `cmd_run_shell_callback` (`cmd-run-shell.c:248-297`) followed by the job's
    /// free callback (`cmd_run_shell_free`).
    fn job_done(self, server: &mut Server, job: JobId) {
        let (input, status) = match server.jobs.get_mut(job) {
            Some(j) => (j.take_input(), j.get_status()),
            None => (Vec::new(), 0),
        };
        for line in split_output(&input) {
            self.print(server, line);
        }
        let cmd: &[u8] = match &self.command {
            RunShellCommand::Shell(cmd) => cmd,
            _ => b"",
        };
        let (msg, retcode) = exit_message(cmd, status);
        if let Some(msg) = msg {
            self.print(server, &msg);
        }
        if let Some(item) = self.item {
            if let Some(c) = item_client(server, item)
                && let Some(client) = server.clients.get_mut(c)
                && client.session.is_none()
            {
                client.retval = retcode;
            }
            queue::continue_item(&mut server.queue, item);
        }
        self.free(server);
    }

    /// `cmd_run_shell_free` (`cmd-run-shell.c:299-314`); the timer is owned by
    /// the closure holding this state, so there is nothing to delete.
    fn free(self, server: &mut Server) {
        let Self {
            client,
            command,
            session,
            ..
        } = self;
        if let Some(s) = session {
            session_release(server, s);
        }
        if let Some(c) = client {
            let _ = crate::client::lifecycle::release(server, c);
        }
        if let RunShellCommand::Commands(state) = command {
            make_commands_free(server, state);
        }
    }
}

/// `cmd_run_shell_exec` (`cmd-run-shell.c:108-190`).
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let c = item_client(server, item);
    let tc = item_target_client(server, item);
    let s = target.s;
    let wait = args.has(b'b') == 0;

    let mut delay = None;
    if let Some(text) = args.get(b'd') {
        match parse_delay(text) {
            Some(d) => delay = Some(d),
            None => return fail(server, item, concat(&[b"invalid delay time: ", cstr(text)])),
        }
    } else if args.count() == 0 {
        return CmdReturn::Normal;
    }

    let run_command = if args.has(b'C') == 0 {
        match args.string(0) {
            Some(cmd) => {
                let mut ft = format::create_from_target(server, item);
                for i in 1..args.count() {
                    ft.add(
                        i.to_string().as_bytes(),
                        args.string(i).unwrap_or(b"").into(),
                    );
                }
                let expanded = ft.expand(server, cmd);
                ft.release(server);
                RunShellCommand::Shell(expanded.into_vec())
            }
            None => RunShellCommand::None,
        }
    } else {
        RunShellCommand::Commands(make_commands_prepare(
            server, command, item, 0, None, wait, true,
        ))
    };

    let pane_id = match target.wp.filter(|_| args.has(b't') != 0) {
        Some(wp) => server.panes.get(wp).map_or(-1, |p| p.public_id as i32),
        None => -1,
    };

    let mut flags = JobFlags::default();
    let (mut client, state_item) = if wait { (c, Some(item)) } else { (tc, None) };
    if let Some(cl) = client
        && crate::client::lifecycle::retain(server, cl).is_err()
    {
        client = None;
    }
    if !wait {
        flags.insert(JobFlags::NOWAIT);
    }
    let cwd = match args.get(b'c') {
        Some(value) => format::single_from_target(server, item, value).into_vec(),
        None => crate::client::registry::get_cwd(server, c, s),
    };
    if args.has(b'E') != 0 {
        flags.insert(JobFlags::SHOWSTDERR);
    }
    let session = s.filter(|s| session_retain(server, *s));

    let state = RunShellState {
        client,
        command: run_command,
        cwd: Some(cwd),
        item: state_item,
        session,
        pane_id,
        flags,
    };
    // `event_active` must not run the callback inside the executor: a zero
    // delay still goes through the loop's deferred queue.
    let duration = delay.map_or(Duration::ZERO, delay_duration);
    crate::server::event_loop::schedule_deferred(
        server,
        duration,
        Box::new(move |server| state.fire(server)),
    );

    if !wait {
        return CmdReturn::Normal;
    }
    CmdReturn::Wait
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_rejects_trailing_text_and_accepts_fractions() {
        assert_eq!(parse_delay(b"0.5"), Some(0.5));
        assert_eq!(parse_delay(b"2"), Some(2.0));
        assert_eq!(parse_delay(b"1x"), None);
        assert_eq!(parse_delay(b""), Some(0.0));
        assert_eq!(delay_duration(-1.0), Duration::ZERO);
        assert_eq!(delay_duration(f64::NAN), Duration::ZERO);
        assert_eq!(delay_duration(1.5), Duration::from_millis(1500));
    }

    #[test]
    fn output_lines_split_on_lf_with_unterminated_rest() {
        let lines: Vec<&[u8]> = split_output(b"a\nb\n\nrest").collect();
        assert_eq!(lines, vec![&b"a"[..], b"b", b"", b"rest"]);
        assert_eq!(split_output(b"").count(), 0);
        assert_eq!(split_output(b"x\n").collect::<Vec<_>>(), vec![&b"x"[..]]);
    }

    #[test]
    fn targeted_shell_output_parses_styles_and_carriage_return() {
        let mut server = Server::default();
        let window = crate::model::window::window_create(&mut server, 20, 4, 0, 0).unwrap();
        let pane = crate::model::pane::pane_create(&mut server, window, 20, 4, 10).unwrap();
        let state = RunShellState {
            client: None,
            command: RunShellCommand::None,
            cwd: None,
            item: None,
            session: None,
            pane_id: server.panes.get(pane).unwrap().public_id as i32,
            flags: JobFlags::default(),
        };
        state.print(&mut server, b"\x1b[31mstyled\x1b[0m\rreplace");
        let mode = server.panes.get(pane).unwrap().modes.first().unwrap().id;
        let screen = crate::modes::copy::state::data(&server, mode)
            .unwrap()
            .backing
            .screen();
        assert_eq!(screen.grid.get_cell(0, 0).data.bytes(), b"r");
        assert_eq!(screen.grid.get_cell(6, 0).data.bytes(), b"e");
        assert_eq!(screen.cx, 7);
        state.print(&mut server, b"\x1b[32mgreen");
        let screen = crate::modes::copy::state::data(&server, mode)
            .unwrap()
            .backing
            .screen();
        assert_eq!(screen.grid.get_cell(0, 1).data.bytes(), b"g");
        assert_eq!(screen.grid.get_cell(0, 1).fg, rmux_emu::colour::Colour(2));
    }

    #[test]
    fn exit_messages_follow_wait_status() {
        assert_eq!(exit_message(b"true", 0), (None, 0));
        assert_eq!(
            exit_message(b"false", 1 << 8),
            (Some(b"'false' returned 1".to_vec()), 1)
        );
        assert_eq!(
            exit_message(b"sleep", libc::SIGTERM),
            (Some(b"'sleep' terminated by signal 15".to_vec()), 143)
        );
        assert_eq!(
            exit_message(b"a\0b", 2 << 8),
            (Some(b"'a' returned 2".to_vec()), 2)
        );
    }
}
