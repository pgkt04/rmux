// Ported from tmux job.c, tmux.h @ 8f25579c
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
pub struct JobFlags(pub u32);
impl JobFlags {
    pub const NOWAIT: Self = Self(1);
    pub const KEEPWRITE: Self = Self(2);
    pub const PTY: Self = Self(4);
    pub const DEFAULTSHELL: Self = Self(8);
    pub const SHOWSTDERR: Self = Self(16);
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
impl std::ops::BitOr for JobFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for JobFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for JobFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use super::event_loop::{EventLoop, LoopAction};
use super::io::BufferedIo;
use crate::ids::{Arena, ArenaError, EventToken, JobId, SessionId};
use crate::model::Server;
use crate::options::environment::{
    Environment, EnvironmentFlags, SessionEnvironmentContext, environ_for_session,
};
use rmux_sys::ProcessId;
use rmux_sys::server::{ExecCommand, JobLaunchOptions, PreparedJob};
use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

pub type JobUpdate = fn(&mut Server, JobId);
pub type JobComplete = Box<dyn FnOnce(&mut Server, JobId)>;
pub type JobDrop = Box<dyn FnOnce(&mut Server, JobId)>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobCommand {
    Shell(Vec<u8>),
    Argv(Vec<Vec<u8>>),
}
pub struct JobLaunch {
    pub command: JobCommand,
    pub environment: Environment,
    pub cwd: Option<Vec<u8>>,
    pub session: Option<SessionId>,
    pub flags: JobFlags,
    pub size: (u32, u32),
    pub update: Option<JobUpdate>,
    pub complete: Option<JobComplete>,
    pub free: Option<JobDrop>,
}
impl JobLaunch {
    pub fn new(command: JobCommand) -> Self {
        Self {
            command,
            environment: Environment::new(),
            cwd: None,
            session: None,
            flags: JobFlags::default(),
            size: (80, 24),
            update: None,
            complete: None,
            free: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobState {
    Running,
    Dead,
    Closed,
}
pub struct Job {
    pub command: Vec<u8>,
    pub flags: JobFlags,
    pub state: JobState,
    pub pid: Option<ProcessId>,
    pub status: i32,
    pub tty: Vec<u8>,
    pub io: BufferedIo,
    event: Option<EventToken>,
    update: Option<JobUpdate>,
    complete: Option<JobComplete>,
    pub free: Option<JobDrop>,
    disposing: bool,
    completing: bool,
    write_started: bool,
}
impl Job {
    pub fn input(&self) -> &[u8] {
        self.io.input()
    }
    pub fn take_input(&mut self) -> Vec<u8> {
        self.io.take_input()
    }
    pub fn consume_input(&mut self, count: usize) {
        self.io.consume_input(count);
    }
    pub fn get_status(&self) -> i32 {
        self.status
    }
    pub fn get_event(&mut self) -> &mut BufferedIo {
        &mut self.io
    }
    pub fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.io.fd()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.complete.take();
        self.free.take();
        if let Some(pid) = self.pid.take() {
            let _ = rmux_sys::client::kill(pid, libc::SIGTERM);
        }
    }
}
pub struct JobTransfer {
    pub fd: OwnedFd,
    pub pid: Option<ProcessId>,
    pub tty: Vec<u8>,
}
#[derive(Default)]
pub struct Jobs {
    arena: Arena<Job, JobId>,
    order: VecDeque<JobId>,
}
impl Jobs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get(&self, id: JobId) -> Option<&Job> {
        self.arena.get(id)
    }
    pub fn get_mut(&mut self, id: JobId) -> Option<&mut Job> {
        self.arena.get_mut(id)
    }
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (JobId, &Job)> {
        self.order
            .iter()
            .filter_map(|id| self.get(*id).map(|job| (*id, job)))
    }
    pub fn run(server: &mut Server, launch: JobLaunch) -> io::Result<JobId> {
        let session = launch
            .session
            .map(|id| {
                server
                    .sessions
                    .get(id)
                    .ok_or_else(|| io::Error::other(ArenaError::StaleId))
            })
            .transpose()?;
        let mut environment = environ_for_session(
            &server.global_environment,
            session.map(|s| &s.environment),
            SessionEnvironmentContext {
                default_terminal: server
                    .options
                    .get_string(server.options.global, b"default-terminal"),
                socket_path: &server.socket_path,
                pid: i64::from(rmux_sys::proc::getpid().0),
                session_id: session.map(|s| s.public_id),
            },
            !server.cfg.finished,
        );
        launch.environment.copy_into(&mut environment);
        let shell = if launch.flags.contains(JobFlags::DEFAULTSHELL) {
            let shell = server.options.get_string(
                session.map_or(server.options.global_s, |s| s.options),
                b"default-shell",
            );
            if rmux_util::shell::check_shell(shell, b"rmux") {
                shell
            } else {
                b"/bin/sh"
            }
        } else {
            b"/bin/sh"
        };
        let (description, command) = match launch.command {
            JobCommand::Shell(command) => {
                if launch.flags.contains(JobFlags::DEFAULTSHELL) {
                    environment.set(b"SHELL", EnvironmentFlags::default(), shell);
                }
                (
                    command.clone(),
                    ExecCommand::Shell {
                        shell: shell.to_vec(),
                        command,
                    },
                )
            }
            JobCommand::Argv(argv) => {
                let mut description = Vec::new();
                for (index, argument) in argv.iter().enumerate() {
                    if index != 0 {
                        description.push(b' ');
                    }
                    description.extend_from_slice(&crate::cmd::arguments::escape(argument));
                }
                (description, ExecCommand::Argv(argv))
            }
        };
        let process = PreparedJob::new(JobLaunchOptions {
            command,
            environment: environment
                .to_envp()
                .into_iter()
                .map(rmux_util::bytes::ByteString::into_vec)
                .collect(),
            cwd: launch.cwd,
            home: rmux_sys::proc::home_directory(None),
            pty: launch.flags.contains(JobFlags::PTY),
            show_stderr: launch.flags.contains(JobFlags::SHOWSTDERR),
            size: rmux_sys::pty::Winsize {
                cols: launch.size.0 as u16,
                rows: launch.size.1 as u16,
                ..Default::default()
            },
        })?
        .launch()?;
        let mut io = BufferedIo::new(process.fd);
        io.set_pty(launch.flags.contains(JobFlags::PTY));
        let id = server
            .jobs
            .arena
            .insert(Job {
                command: description,
                flags: launch.flags,
                state: JobState::Running,
                pid: Some(process.pid),
                status: 0,
                tty: process.tty,
                io,
                event: None,
                update: launch.update,
                complete: launch.complete,
                free: launch.free,
                disposing: false,
                completing: false,
                write_started: false,
            })
            .map_err(io::Error::other)?;
        server.jobs.order.push_front(id);
        let result = server.event_loop.register(
            server.jobs.get(id).expect("new job").io.fd(),
            true,
            true,
            LoopAction::Job(id),
        );
        match result {
            Ok(event) => {
                server.jobs.get_mut(id).expect("new job").event = Some(event);
                Ok(id)
            }
            Err(error) => {
                let _ = free(server, id);
                Err(error)
            }
        }
    }
    fn remove_job(&mut self, id: JobId, event_loop: &mut EventLoop) -> Result<(), ArenaError> {
        let job = self.get_mut(id).ok_or(ArenaError::StaleId)?;
        if let Some(event) = job.event.take() {
            event_loop.deregister(event);
        }
        drop(self.remove(id)?);
        Ok(())
    }
    fn transfer_job(
        &mut self,
        id: JobId,
        event_loop: &mut EventLoop,
    ) -> Result<JobTransfer, ArenaError> {
        let job = self.get_mut(id).ok_or(ArenaError::StaleId)?;
        if let Some(event) = job.event.take() {
            event_loop.deregister(event);
        }
        let fd = job.io.take_fd().expect("live job fd");
        let pid = job.pid.take();
        let tty = std::mem::take(&mut job.tty);
        drop(self.remove(id)?);
        Ok(JobTransfer { fd, pid, tty })
    }
    fn remove(&mut self, id: JobId) -> Result<Job, ArenaError> {
        let job = self
            .arena
            .request_remove(id)?
            .expect("jobs do not retain arena leases");
        self.order.retain(|candidate| *candidate != id);
        Ok(job)
    }
    pub fn still_running(&self) -> bool {
        self.iter()
            .any(|(_, job)| job.state == JobState::Running && !job.flags.contains(JobFlags::NOWAIT))
    }
    pub fn kill_all(&self) {
        for (_, job) in self.iter() {
            if let Some(pid) = job.pid {
                let _ = rmux_sys::client::kill(pid, libc::SIGTERM);
            }
        }
    }
    pub fn print_summary(&self, blank: bool) -> Vec<Vec<u8>> {
        let mut lines = Vec::with_capacity(self.len() + usize::from(blank));
        if blank && !self.is_empty() {
            lines.push(Vec::new());
        }
        for (index, (_, job)) in self.iter().enumerate() {
            let mut line = format!("Job {index}: ").into_bytes();
            line.extend_from_slice(&job.command);
            line.extend_from_slice(
                format!(
                    " [fd={}, pid={}, status={}]",
                    job.io.fd().as_raw_fd(),
                    job.pid.map_or(-1, |pid| pid.0),
                    job.status
                )
                .as_bytes(),
            );
            lines.push(line);
        }
        lines
    }
}
pub fn run(server: &mut Server, launch: JobLaunch) -> io::Result<JobId> {
    Jobs::run(server, launch)
}
pub fn free(server: &mut Server, id: JobId) -> Result<(), ArenaError> {
    if !run_cleanup(server, id)? {
        return Ok(());
    }
    server.jobs.remove_job(id, &mut server.event_loop)
}
pub fn transfer(server: &mut Server, id: JobId) -> Result<JobTransfer, ArenaError> {
    if !run_cleanup(server, id)? {
        return Err(ArenaError::StaleId);
    }
    server.jobs.transfer_job(id, &mut server.event_loop)
}
fn run_cleanup(server: &mut Server, id: JobId) -> Result<bool, ArenaError> {
    let job = server.jobs.get_mut(id).ok_or(ArenaError::StaleId)?;
    if job.disposing {
        return Ok(false);
    }
    job.disposing = true;
    let cleanup = job.free.take();
    if let Some(cleanup) = cleanup {
        cleanup(server, id);
    }
    Ok(server.jobs.get(id).is_some())
}
pub fn resize(server: &Server, id: JobId, sx: u32, sy: u32) -> io::Result<()> {
    let job = server
        .jobs
        .get(id)
        .ok_or_else(|| io::Error::other(ArenaError::StaleId))?;
    if job.flags.contains(JobFlags::PTY) {
        rmux_sys::pty::set_winsize(
            job.io.fd(),
            rmux_sys::pty::Winsize {
                cols: sx as u16,
                rows: sy as u16,
                ..Default::default()
            },
        )?;
    }
    Ok(())
}
pub fn still_running(server: &Server) -> bool {
    server.jobs.still_running()
}
pub fn kill_all(server: &Server) {
    server.jobs.kill_all();
}
pub fn queue_input(server: &mut Server, id: JobId, bytes: Vec<u8>) -> io::Result<()> {
    let job = server
        .jobs
        .get_mut(id)
        .ok_or_else(|| io::Error::other(ArenaError::StaleId))?;
    if job.write_started && !job.flags.contains(JobFlags::KEEPWRITE) && job.io.output_len() == 0 {
        return Err(io::Error::from_raw_os_error(libc::EPIPE));
    }
    job.io.queue(bytes);
    sync_interest(server, id)
}
fn sync_interest(server: &mut Server, id: JobId) -> io::Result<()> {
    if let Some(job) = server.jobs.get(id) {
        let (read, write) = job.io.interests();
        if let Some(event) = job.event {
            server
                .event_loop
                .reregister(event, read, write || !job.write_started)?;
        }
    }
    Ok(())
}
fn complete(server: &mut Server, id: JobId) {
    let Some(job) = server.jobs.get_mut(id) else {
        return;
    };
    if job.completing || job.disposing {
        return;
    }
    job.completing = true;
    let callback = job.complete.take();
    if let Some(callback) = callback {
        callback(server, id);
    }
    if server.jobs.get(id).is_some() {
        let _ = free(server, id);
    }
}
fn output_closed(server: &mut Server, id: JobId) -> io::Result<()> {
    let Some(job) = server.jobs.get_mut(id) else {
        return Ok(());
    };
    if job.state == JobState::Dead {
        complete(server, id);
    } else {
        job.state = JobState::Closed;
        job.io.enable_read(false);
    }
    sync_interest(server, id)
}
pub fn on_ready(server: &mut Server, id: JobId, readable: bool, writable: bool) -> io::Result<()> {
    if readable {
        let Some(job) = server.jobs.get_mut(id) else {
            return Ok(());
        };
        let before = job.io.input_len();
        let result = job.io.read_ready();
        let changed = job.io.input_len() != before;
        let update = if changed { job.update.take() } else { None };
        let closed = result.as_ref().map_or(true, |progress| progress.eof);
        if let Some(update) = update {
            update(server, id);
            if let Some(job) = server.jobs.get_mut(id) {
                if job.update.is_none() {
                    job.update = Some(update);
                }
            }
        }
        if closed {
            output_closed(server, id)?;
        }
    }
    if writable {
        let Some(job) = server.jobs.get_mut(id) else {
            return Ok(());
        };
        job.write_started = true;
        match job.io.write_ready() {
            Ok(progress) if progress.drained && !job.flags.contains(JobFlags::KEEPWRITE) => {
                // shutdown on a pty is ENOTSOCK; C ignores that error and disables writes.
                let _ = rmux_sys::server::shutdown_write(job.io.fd());
                job.io.enable_write(false);
            }
            Ok(_) => {}
            Err(_) => {
                output_closed(server, id)?;
            }
        }
    }
    sync_interest(server, id)
}
pub fn check_died(server: &mut Server, pid: ProcessId, status: i32) -> io::Result<()> {
    let id = server
        .jobs
        .iter()
        .find_map(|(id, job)| (job.pid == Some(pid)).then_some(id));
    let Some(id) = id else {
        return Ok(());
    };
    if rmux_sys::server::stop_signal(status).is_some() {
        if !rmux_sys::server::terminal_stop(status) {
            let _ = rmux_sys::server::continue_process_group(pid);
        }
        return Ok(());
    }
    let job = server.jobs.get_mut(id).expect("resolved job");
    job.status = status;
    job.pid = None;
    if job.state == JobState::Closed {
        complete(server, id);
    } else {
        job.state = JobState::Dead;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn fixture(
        server: &mut Server,
        name: &[u8],
        flags: JobFlags,
        callback: Option<JobComplete>,
    ) -> (JobId, UnixStream) {
        let (left, right) = UnixStream::pair().unwrap();
        let id = server
            .jobs
            .arena
            .insert(Job {
                command: name.to_vec(),
                flags,
                state: JobState::Running,
                pid: Some(ProcessId(i32::MAX)),
                status: 0,
                tty: Vec::new(),
                io: BufferedIo::new(left.into()),
                event: None,
                update: None,
                complete: callback,
                free: None,
                disposing: false,
                completing: false,
                write_started: false,
            })
            .unwrap();
        server.jobs.order.push_front(id);
        let event = server
            .event_loop
            .register(
                server.jobs.get(id).unwrap().io.fd(),
                true,
                true,
                LoopAction::Job(id),
            )
            .unwrap();
        server.jobs.get_mut(id).unwrap().event = Some(event);
        (id, right)
    }
    #[test]
    fn completion_requires_both_halves_in_either_order_and_frees_once() {
        for status_first in [true, false] {
            let mut server = Server::new();
            let calls = Rc::new(Cell::new(0));
            let observed = Rc::clone(&calls);
            let (id, right) = fixture(
                &mut server,
                b"fixture",
                JobFlags::default(),
                Some(Box::new(move |server, id| {
                    let job = server.jobs.get(id).unwrap();
                    assert_eq!(job.status, 7 << 8);
                    observed.set(observed.get() + 1);
                })),
            );
            if status_first {
                check_died(&mut server, ProcessId(i32::MAX), 7 << 8).unwrap();
                assert_eq!(server.jobs.get(id).unwrap().state, JobState::Dead);
                assert!(server.jobs.get(id).unwrap().pid.is_none());
                assert!(!still_running(&server));
                assert_eq!(calls.get(), 0);
                right.shutdown(std::net::Shutdown::Both).unwrap();
                drop(right);
                on_ready(&mut server, id, true, false).unwrap();
            } else {
                right.shutdown(std::net::Shutdown::Both).unwrap();
                drop(right);
                on_ready(&mut server, id, true, false).unwrap();
                assert_eq!(server.jobs.get(id).unwrap().state, JobState::Closed);
                assert!(!still_running(&server));
                assert_eq!(calls.get(), 0);
                check_died(&mut server, ProcessId(i32::MAX), 7 << 8).unwrap();
            }
            assert_eq!(calls.get(), 1);
            assert!(server.jobs.get(id).is_none());
            on_ready(&mut server, id, true, true).unwrap();
            check_died(&mut server, ProcessId(i32::MAX), 7 << 8).unwrap();
            assert_eq!(calls.get(), 1);
        }
    }
    struct DropCounter(Rc<Cell<usize>>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    #[test]
    fn transfer_moves_fd_pid_and_tty_without_killing_and_drops_callback_once() {
        use std::io::{Read, Write};
        let mut server = Server::new();
        let drops = Rc::new(Cell::new(0));
        let state = DropCounter(Rc::clone(&drops));
        let (id, mut right) = fixture(
            &mut server,
            b"transfer",
            JobFlags::default(),
            Some(Box::new(move |_, _| {
                drop(state);
            })),
        );
        server.jobs.get_mut(id).unwrap().tty = b"/dev/test-tty".to_vec();
        let transfer = transfer(&mut server, id).unwrap();
        assert_eq!(transfer.pid, Some(ProcessId(i32::MAX)));
        assert_eq!(transfer.tty, b"/dev/test-tty");
        assert_eq!(drops.get(), 1);
        assert!(server.jobs.get(id).is_none());
        right.write_all(b"still open").unwrap();
        let mut bytes = [0u8; 10];
        assert_eq!(
            rmux_sys::fd::read(transfer.fd.as_fd(), &mut bytes).unwrap(),
            10
        );
        assert_eq!(&bytes, b"still open");
        drop(transfer);
        let mut bytes = Vec::new();
        assert_eq!(right.read_to_end(&mut bytes).unwrap(), 0);
        assert_eq!(drops.get(), 1);
    }
    #[test]
    fn completion_can_free_itself_and_create_new_jobs_in_newest_order() {
        let mut server = Server::new();
        let created = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&created);
        let (older, _older_peer) = fixture(&mut server, b"older", JobFlags::NOWAIT, None);
        let (id, peer) = fixture(
            &mut server,
            b"completing",
            JobFlags::default(),
            Some(Box::new(move |server, id| {
                free(server, id).unwrap();
                let (new, peer) = fixture(server, b"new", JobFlags::NOWAIT, None);
                *seen.borrow_mut() = Some((new, peer));
            })),
        );
        drop(peer);
        output_closed(&mut server, id).unwrap();
        check_died(&mut server, ProcessId(i32::MAX), 0).unwrap();
        let new = created.borrow().as_ref().unwrap().0;
        assert_ne!(new, id);
        assert!(server.jobs.get(id).is_none());
        assert_eq!(
            server.jobs.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            vec![new, older]
        );
        assert!(!still_running(&server));
        let lines = server.jobs.print_summary(true);
        assert_eq!(lines[0], b"");
        assert!(lines[1].starts_with(b"Job 0: new [fd="));
        assert!(lines[2].starts_with(b"Job 1: older [fd="));
    }
    #[test]
    fn stopped_statuses_do_not_finish_or_save_wait_status() {
        let mut server = Server::new();
        let (id, _peer) = fixture(&mut server, b"stopped", JobFlags::default(), None);
        for signal in [libc::SIGTTIN, libc::SIGTTOU, libc::SIGSTOP] {
            check_died(&mut server, ProcessId(i32::MAX), (signal << 8) | 0x7f).unwrap();
            let job = server.jobs.get(id).unwrap();
            assert_eq!(job.state, JobState::Running);
            assert_eq!(job.status, 0);
            assert_eq!(job.pid, Some(ProcessId(i32::MAX)));
        }
        assert!(still_running(&server));
        server
            .jobs
            .get_mut(id)
            .unwrap()
            .flags
            .insert(JobFlags::NOWAIT);
        assert!(!still_running(&server));
    }
    #[test]
    fn free_closes_fd_drops_state_and_stale_ids_do_not_touch_replacement() {
        use std::io::Read;
        let mut server = Server::new();
        let drops = Rc::new(Cell::new(0));
        let state = DropCounter(Rc::clone(&drops));
        let (old, mut peer) = fixture(
            &mut server,
            b"old",
            JobFlags::NOWAIT,
            Some(Box::new(move |_, _| {
                drop(state);
            })),
        );
        free(&mut server, old).unwrap();
        assert_eq!(drops.get(), 1);
        let mut byte = [0u8; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
        let (new, _peer) = fixture(&mut server, b"replacement", JobFlags::NOWAIT, None);
        assert_ne!(new, old);
        on_ready(&mut server, old, true, true).unwrap();
        assert!(free(&mut server, old).is_err());
        assert!(server.jobs.get(new).is_some());
    }
    fn pump(server: &mut Server, id: JobId) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while server.jobs.get(id).is_some() {
            for ready in server
                .event_loop
                .poll(Some(Duration::from_millis(5)))
                .unwrap()
            {
                if let LoopAction::Job(job) = ready.action {
                    on_ready(server, job, ready.readable, ready.writable).unwrap();
                }
            }
            let pid = server.jobs.get(id).and_then(|job| job.pid);
            if let Some(pid) = pid {
                if let Some(status) = rmux_sys::proc::wait_process(pid, true).unwrap() {
                    check_died(server, pid, status).unwrap();
                }
            }
            if Instant::now() >= deadline {
                if let Some(pid) = server.jobs.get(id).and_then(|job| job.pid) {
                    let _ = rmux_sys::proc::terminate_process(pid);
                    let _ = rmux_sys::proc::wait_process(pid, false);
                }
                panic!("job completion timed out");
            }
        }
    }
    #[test]
    fn launch_duplex_input_and_update_can_create_job_without_arena_borrow() {
        let mut server = Server::new();
        server.cfg.finished = true;
        let observed = Rc::new(RefCell::new(Vec::new()));
        let result = Rc::clone(&observed);
        let mut launch = JobLaunch::new(JobCommand::Shell(b"cat; exit 7".to_vec()));
        launch.complete = Some(Box::new(move |server, id| {
            let job = server.jobs.get_mut(id).unwrap();
            assert_eq!(job.status, 7 << 8);
            *result.borrow_mut() = job.take_input();
        }));
        let id = run(&mut server, launch).unwrap();
        queue_input(&mut server, id, b"hello\0world".to_vec()).unwrap();
        pump(&mut server, id);
        assert_eq!(&*observed.borrow(), b"hello\0world");
        fn update(server: &mut Server, id: JobId) {
            server.jobs.get_mut(id).unwrap().update = None;
            let mut launch = JobLaunch::new(JobCommand::Argv(vec![b"true".to_vec()]));
            launch.flags = JobFlags::NOWAIT;
            let _ = run(server, launch).unwrap();
            free(server, id).unwrap();
        }
        let mut launch = JobLaunch::new(JobCommand::Shell(b"printf update".to_vec()));
        launch.update = Some(update);
        let id = run(&mut server, launch).unwrap();
        let pid = server.jobs.get(id).unwrap().pid.unwrap();
        pump(&mut server, id);
        let _ = rmux_sys::proc::wait_process(pid, false);
        let remaining = server.jobs.iter().map(|(id, _)| id).collect::<Vec<_>>();
        for id in remaining {
            pump(&mut server, id);
        }
    }
    #[test]
    fn configuration_retains_outside_term_and_default_shell_sets_shell_only_for_string() {
        let mut server = Server::new();
        let mut parser = Server::new();
        server.options.set_string(
            server.options.global_s,
            b"default-shell",
            false,
            b"/bin/sh",
            &mut parser,
        );
        server
            .global_environment
            .set(b"TERM", EnvironmentFlags::default(), b"outside-term");
        server
            .global_environment
            .set(b"SHELL", EnvironmentFlags::default(), b"outside-shell");
        for finished in [false, true] {
            server.cfg.finished = finished;
            let bytes = Rc::new(RefCell::new(Vec::new()));
            let observed = Rc::clone(&bytes);
            let mut launch = JobLaunch::new(JobCommand::Shell(
                b"printf '%s|%s|%s' \"$TERM\" \"$SHELL\" \"$0\"".to_vec(),
            ));
            launch.flags = JobFlags::DEFAULTSHELL;
            launch.complete = Some(Box::new(move |server, id| {
                *observed.borrow_mut() = server.jobs.get_mut(id).unwrap().take_input();
            }));
            let id = run(&mut server, launch).unwrap();
            pump(&mut server, id);
            let term = if finished {
                server
                    .options
                    .get_string(server.options.global, b"default-terminal")
            } else {
                b"outside-term"
            };
            assert!(bytes.borrow().starts_with(&[term, b"|"].concat()));
            assert!(bytes.borrow().ends_with(b"|sh"));
            assert!(
                !bytes
                    .borrow()
                    .windows(b"outside-shell".len())
                    .any(|s| s == b"outside-shell")
            );
        }
        let bytes = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&bytes);
        let mut launch = JobLaunch::new(JobCommand::Argv(vec![
            b"/bin/sh".to_vec(),
            b"-c".to_vec(),
            b"printf '%s' \"$SHELL\"".to_vec(),
        ]));
        launch.flags = JobFlags::DEFAULTSHELL;
        launch.complete = Some(Box::new(move |server, id| {
            *observed.borrow_mut() = server.jobs.get_mut(id).unwrap().take_input();
        }));
        let id = run(&mut server, launch).unwrap();
        pump(&mut server, id);
        assert_eq!(&*bytes.borrow(), b"outside-shell");
    }
    #[test]
    fn resize_only_pty_and_keepwrite_preserves_write_half() {
        use std::io::Write;
        let mut server = Server::new();
        let (id, mut peer) = fixture(&mut server, b"keepwrite", JobFlags::KEEPWRITE, None);
        on_ready(&mut server, id, false, true).unwrap();
        queue_input(&mut server, id, b"x".to_vec()).unwrap();
        on_ready(&mut server, id, false, true).unwrap();
        assert!(!server.jobs.get(id).unwrap().io.write_closed());
        peer.write_all(b"reply").unwrap();
        resize(&server, id, 42, 13).unwrap();
        let (master, slave, tty) = rmux_sys::pty::openpty().unwrap();
        {
            let job = server.jobs.get_mut(id).unwrap();
            job.flags = JobFlags::PTY;
            job.tty = tty;
            if let Some(event) = job.event.take() {
                server.event_loop.deregister(event);
            }
            job.io = BufferedIo::new(master);
        }
        resize(&server, id, 123, 45).unwrap();
        let size = rmux_sys::pty::get_winsize(slave.as_fd()).unwrap();
        assert_eq!((size.cols, size.rows), (123, 45));
        let (other, _peer) = UnixStream::pair().unwrap();
        server.jobs.get_mut(id).unwrap().io = BufferedIo::new(other.into());
        assert!(resize(&server, id, 40, 20).is_err());
    }

    #[test]
    fn kill_all_signals_live_children_and_free_terminates_owned_process() {
        let mut server = Server::new();
        let mut launch = JobLaunch::new(JobCommand::Shell(b"exec /bin/cat".to_vec()));
        launch.flags = JobFlags::KEEPWRITE;
        let id = run(&mut server, launch).unwrap();
        let pid = server.jobs.get(id).unwrap().pid.unwrap();
        kill_all(&server);
        let status = rmux_sys::proc::wait_process(pid, false).unwrap().unwrap();
        assert_eq!(rmux_sys::server::status_signal(status), Some(libc::SIGTERM));
        check_died(&mut server, pid, status).unwrap();
        on_ready(&mut server, id, true, false).unwrap();
        assert!(server.jobs.get(id).is_none());
        let mut launch = JobLaunch::new(JobCommand::Shell(b"exec /bin/cat".to_vec()));
        launch.flags = JobFlags::KEEPWRITE;
        let id = run(&mut server, launch).unwrap();
        let pid = server.jobs.get(id).unwrap().pid.unwrap();
        free(&mut server, id).unwrap();
        let status = rmux_sys::proc::wait_process(pid, false).unwrap().unwrap();
        assert_eq!(rmux_sys::server::status_signal(status), Some(libc::SIGTERM));
    }

    #[test]
    fn server_cleanup_runs_once_before_cancel_or_transfer_and_allows_reentry() {
        for transferring in [false, true] {
            let mut server = Server::new();
            let calls = Rc::new(Cell::new(0));
            let observed = Rc::clone(&calls);
            let (id, peer) = fixture(&mut server, b"cleanup", JobFlags::NOWAIT, None);
            server.jobs.get_mut(id).unwrap().free = Some(Box::new(move |server, id| {
                assert!(server.jobs.get(id).is_some());
                observed.set(observed.get() + 1);
                free(server, id).unwrap();
                let (_new, _peer) = fixture(server, b"created in cleanup", JobFlags::NOWAIT, None);
            }));
            if transferring {
                drop(transfer(&mut server, id).unwrap());
            } else {
                free(&mut server, id).unwrap();
            }
            drop(peer);
            assert_eq!(calls.get(), 1);
            assert!(server.jobs.get(id).is_none());
            assert_eq!(server.jobs.len(), 1);
        }
    }

    #[test]
    fn normal_completion_can_consume_cleanup_or_leave_it_for_auto_free() {
        for consumed in [false, true] {
            let mut server = Server::new();
            let calls = Rc::new(Cell::new(0));
            let observed = Rc::clone(&calls);
            let (id, peer) = fixture(
                &mut server,
                b"cleanup completion",
                JobFlags::NOWAIT,
                Some(Box::new(move |server, id| {
                    if consumed {
                        server.jobs.get_mut(id).unwrap().free.take();
                    }
                })),
            );
            server.jobs.get_mut(id).unwrap().free = Some(Box::new(move |_, _| {
                observed.set(observed.get() + 1);
            }));
            drop(peer);
            on_ready(&mut server, id, true, false).unwrap();
            check_died(&mut server, ProcessId(i32::MAX), 0).unwrap();
            assert_eq!(calls.get(), usize::from(!consumed));
            assert!(server.jobs.get(id).is_none());
        }
    }
}
