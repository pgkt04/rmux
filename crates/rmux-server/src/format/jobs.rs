// Ported from tmux format.c @ 8f25579c
/*
 * Copyright (c) 2011 Nicholas Marriott <nicholas.marriott@gmail.com>
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

use std::collections::BTreeMap;
use std::time::Duration;

use rmux_util::buffer::ByteBuffer;
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::strtonum::strtonum;

use crate::ids::{ClientId, JobId};
use crate::server::job::JobFlags;

use super::FormatFlags;

pub const CYCLE_PERIOD: Duration = Duration::from_millis(100);
pub const JOB_MAX_AGE: i64 = 3600;

/// G14 retains this token with both job callbacks. A restart or eviction
/// invalidates it even when the same owner, tag and command are reused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatJobToken {
    pub owner: Option<ClientId>,
    pub tag: u32,
    pub raw: ByteString,
    pub generation: u64,
}

/// Launch a shell command, with no argv, session, environment override or pty.
/// The runtime resolves cwd using server_client_get_cwd(owner, None).
#[derive(Debug)]
pub struct FormatJobLaunch<'a> {
    pub owner: Option<ClientId>,
    pub command: &'a [u8],
    pub flags: JobFlags,
    pub callback: FormatJobToken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatCycleToken {
    pub owner: ClientId,
    pub generation: u64,
}

/// Runtime operations are synchronous, but job and timer callbacks must be
/// dispatched after returning to the event loop, without a borrowed cache.
/// `status` is server_status_client; `redraw_status` only sets REDRAWSTATUS.
pub trait FormatJobRuntime {
    fn run(&mut self, launch: FormatJobLaunch<'_>) -> Option<JobId>;
    fn cancel(&mut self, job: JobId);
    fn status(&mut self, owner: ClientId);
    fn cycle_start(&mut self, token: FormatCycleToken, delay: Duration);
    fn cycle_cancel(&mut self, token: FormatCycleToken);
    fn redraw_status(&mut self, owner: ClientId);
}

#[derive(Debug)]
pub struct FormatJob {
    pub expanded: Option<ByteString>,
    pub last: i64,
    pub out: Option<ByteString>,
    pub updated: bool,
    pub job: Option<JobId>,
    pub status: bool,
    pub generation: u64,
}

impl FormatJob {
    fn new() -> Self {
        Self {
            expanded: None,
            last: 0,
            out: None,
            updated: false,
            job: None,
            status: false,
            generation: 0,
        }
    }
}

type CommandJobs = BTreeMap<ByteString, FormatJob>;
type TaggedJobs = BTreeMap<u32, CommandJobs>;

/// Owns global and owner-client caches without requiring fields on Server or
/// Client. Call `lost_client` before releasing the client's environment; call
/// `tidy_jobs` from hourly maintenance. Callbacks use the originating token.
#[derive(Debug, Default)]
pub struct FormatJobs {
    jobs: BTreeMap<Option<ClientId>, TaggedJobs>,
    cycles: BTreeMap<ClientId, u64>,
    generation: u64,
}

pub struct FormatCycleRequest<'a> {
    pub owner: Option<ClientId>,
    pub flags: FormatFlags,
    pub no_cycle: bool,
    pub frames: &'a [u8],
    pub count: Option<&'a [u8]>,
    pub start_ms: u64,
}

impl FormatJobs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.jobs
            .values()
            .flat_map(BTreeMap::values)
            .map(BTreeMap::len)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn record(&self, owner: Option<ClientId>, tag: u32, raw: &[u8]) -> Option<&FormatJob> {
        self.jobs.get(&owner)?.get(&tag)?.get(cstr(raw))
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("format generation exhausted");
        self.generation
    }

    /// `expanded` must be expanded with NoJobs and NoCycle, and Time cleared.
    /// The returned cached bytes must be expanded with those same restrictions.
    /// Recursive NoJobs suppression belongs to the scanner, before this call.
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &mut self,
        runtime: &mut impl FormatJobRuntime,
        owner: Option<ClientId>,
        tag: u32,
        flags: FormatFlags,
        raw: &[u8],
        expanded: &[u8],
        now: i64,
    ) -> ByteString {
        if flags.contains(FormatFlags::NOJOBS) {
            return ByteString::new();
        }
        let raw = cstr(raw);
        let expanded = cstr(expanded);
        let existing = self.record(owner, tag, raw);
        let is_new = existing.is_none();
        let force = flags.contains(FormatFlags::FORCE)
            || existing
                .and_then(|job| job.expanded.as_ref())
                .map(ByteString::as_bytes)
                != Some(expanded);
        let launch = force || existing.is_some_and(|job| job.job.is_none() && job.last != now);
        let generation = if launch { self.next_generation() } else { 0 };
        let commands = self.jobs.entry(owner).or_default().entry(tag).or_default();
        if is_new {
            commands.insert(ByteString::from(raw), FormatJob::new());
        }
        let record = commands.get_mut(raw).expect("inserted format job");
        if record.expanded.as_ref().map(ByteString::as_bytes) != Some(expanded) {
            record.expanded = Some(ByteString::from(expanded));
        }
        if launch {
            record.generation = generation;
            if force && let Some(job) = record.job.take() {
                runtime.cancel(job);
            }
            record.job = runtime.run(FormatJobLaunch {
                owner,
                command: expanded,
                flags: JobFlags::NOWAIT,
                callback: FormatJobToken {
                    owner,
                    tag,
                    raw: ByteString::from(raw),
                    generation,
                },
            });
            if record.job.is_none() {
                record.out = Some(failure(raw, b" didn't start>"));
            }
            record.last = now;
            record.updated = false;
        } else if record.job.is_some()
            && i128::from(now) - i128::from(record.last) > 1
            && record.out.is_none()
        {
            record.out = Some(failure(raw, b" not ready>"));
        }
        if flags.contains(FormatFlags::STATUS) {
            record.status = true;
        }
        record.out.clone().unwrap_or_default()
    }

    fn callback_record(&mut self, token: &FormatJobToken) -> Option<&mut FormatJob> {
        let record = self
            .jobs
            .get_mut(&token.owner)?
            .get_mut(&token.tag)?
            .get_mut(token.raw.as_bytes())?;
        (record.generation == token.generation && record.job.is_some()).then_some(record)
    }

    /// Consume all complete EVBUFFER_EOL_ANY lines; retain only the last.
    /// A stale callback does not consume its input buffer.
    pub fn update(
        &mut self,
        runtime: &mut impl FormatJobRuntime,
        token: &FormatJobToken,
        input: &mut ByteBuffer,
        now: i64,
    ) -> bool {
        let Some(record) = self.callback_record(token) else {
            return false;
        };
        let bytes = input.data();
        let mut consumed = 0;
        let mut last = None;
        while let Some((length, used)) = line_end(&bytes[consumed..]) {
            last = Some(&bytes[consumed..consumed + length]);
            consumed += used;
        }
        let Some(line) = last else {
            return true;
        };
        record.out = Some(ByteString::from(cstr(line)));
        record.updated = true;
        input.drain(consumed);
        if record.status && record.last != now {
            if let Some(owner) = token.owner {
                runtime.status(owner);
            }
            record.last = now;
        }
        true
    }

    /// Clear the running handle first, then use the first remaining complete
    /// line, or copy all incomplete bytes without draining them, as in C.
    pub fn complete(
        &mut self,
        runtime: &mut impl FormatJobRuntime,
        token: &FormatJobToken,
        input: &mut ByteBuffer,
    ) -> bool {
        let Some(record) = self.callback_record(token) else {
            return false;
        };
        record.job = None;
        let (bytes, consumed) = if let Some((length, used)) = line_end(input.data()) {
            (&input.data()[..length], used)
        } else {
            (input.data(), 0)
        };
        let bytes = cstr(bytes);
        if !bytes.is_empty() || !record.updated {
            record.out = Some(ByteString::from(bytes));
        }
        input.drain(consumed);
        if record.status {
            if let Some(owner) = token.owner {
                runtime.status(owner);
            }
            record.status = false;
        }
        true
    }

    /// Call per owner in server client-list order when cancellation ordering
    /// across clients matters. Within each cache order is tag, then raw bytes.
    pub fn tidy_owner(
        &mut self,
        runtime: &mut impl FormatJobRuntime,
        owner: Option<ClientId>,
        now: i64,
        force: bool,
    ) {
        if force {
            if let Some(tags) = self.jobs.remove(&owner) {
                for commands in tags.into_values() {
                    for record in commands.into_values() {
                        if let Some(job) = record.job {
                            runtime.cancel(job);
                        }
                    }
                }
            }
            return;
        }
        if let Some(tags) = self.jobs.get_mut(&owner) {
            tidy_tags(tags, runtime, now);
            if tags.is_empty() {
                self.jobs.remove(&owner);
            }
        }
    }

    /// Global cache first, then owner caches in generational-id order.
    pub fn tidy_jobs(&mut self, runtime: &mut impl FormatJobRuntime, now: i64) {
        self.jobs.retain(|_, tags| {
            tidy_tags(tags, runtime, now);
            !tags.is_empty()
        });
    }

    pub fn lost_client(&mut self, runtime: &mut impl FormatJobRuntime, owner: ClientId) {
        self.cycle_cancel(runtime, owner);
        self.tidy_owner(runtime, Some(owner), 0, true);
    }

    pub fn cycle_start(&mut self, runtime: &mut impl FormatJobRuntime, owner: ClientId) {
        if self.cycles.contains_key(&owner) {
            return;
        }
        let generation = self.next_generation();
        self.cycles.insert(owner, generation);
        runtime.cycle_start(FormatCycleToken { owner, generation }, CYCLE_PERIOD);
    }

    pub fn cycle_pending(&self, owner: ClientId) -> bool {
        self.cycles.contains_key(&owner)
    }

    pub fn cycle_cancel(&mut self, runtime: &mut impl FormatJobRuntime, owner: ClientId) {
        if let Some(generation) = self.cycles.remove(&owner) {
            runtime.cycle_cancel(FormatCycleToken { owner, generation });
        }
    }

    /// G15 supplies current message/prompt presence after resolving token.owner.
    /// A timer expiry consumes the pending timer and never rearms itself.
    pub fn cycle_complete(
        &mut self,
        runtime: &mut impl FormatJobRuntime,
        token: FormatCycleToken,
        message: bool,
        prompt: bool,
    ) -> bool {
        if self.cycles.get(&token.owner) != Some(&token.generation) {
            return false;
        }
        self.cycles.remove(&token.owner);
        if !message && !prompt {
            runtime.redraw_status(token.owner);
        }
        true
    }

    /// Select a raw frame using the top-level expansion start, not wall time.
    pub fn cycle(
        &mut self,
        runtime: &mut impl FormatJobRuntime,
        request: FormatCycleRequest<'_>,
    ) -> ByteString {
        let frames = cstr(request.frames);
        if !request.flags.contains(FormatFlags::STATUS) || request.no_cycle || frames.is_empty() {
            return ByteString::new();
        }
        let count = request
            .count
            .and_then(|count| strtonum(count, 1, 100).ok())
            .unwrap_or(1) as u64;
        let number = frames.iter().filter(|&&byte| byte == b',').count() + 1;
        let index = (request.start_ms / (count * 100)) % number as u64;
        if number > 1
            && let Some(owner) = request.owner
        {
            self.cycle_start(runtime, owner);
        }
        ByteString::from(
            frames
                .split(|&byte| byte == b',')
                .nth(index as usize)
                .expect("counted frame"),
        )
    }
}

fn tidy_tags(tags: &mut TaggedJobs, runtime: &mut impl FormatJobRuntime, now: i64) {
    tags.retain(|_, commands| {
        commands.retain(|_, record| {
            if record.last > now
                || i128::from(now) - i128::from(record.last) < i128::from(JOB_MAX_AGE)
            {
                return true;
            }
            record.generation = 0;
            if let Some(job) = record.job.take() {
                runtime.cancel(job);
            }
            false
        });
        !commands.is_empty()
    });
}

fn failure(raw: &[u8], suffix: &[u8]) -> ByteString {
    let mut out = ByteString::with_capacity(3 + raw.len() + suffix.len());
    out.extend_from_slice(b"<'");
    out.extend_from_slice(raw);
    out.push(b'\'');
    out.extend_from_slice(suffix);
    out
}

// evbuffer_readline is EVBUFFER_EOL_ANY, not ByteBuffer's LF-only readln.
fn line_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let length = bytes
        .iter()
        .position(|&byte| matches!(byte, b'\r' | b'\n'))?;
    let mut consumed = length + 1;
    while consumed < bytes.len() && matches!(bytes[consumed], b'\r' | b'\n') {
        consumed += 1;
    }
    Some((length, consumed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;

    #[derive(Debug, Eq, PartialEq)]
    enum Action {
        Run(Option<ClientId>, ByteString, JobFlags, FormatJobToken),
        Cancel(JobId),
        Status(ClientId),
        Cycle(FormatCycleToken, Duration),
        CancelCycle(FormatCycleToken),
        Redraw(ClientId),
    }

    #[derive(Default)]
    struct Fixture {
        actions: Vec<Action>,
        next_job: u32,
        fail: bool,
    }

    impl FormatJobRuntime for Fixture {
        fn run(&mut self, launch: FormatJobLaunch<'_>) -> Option<JobId> {
            self.actions.push(Action::Run(
                launch.owner,
                ByteString::from(launch.command),
                launch.flags,
                launch.callback,
            ));
            self.next_job += 1;
            (!self.fail).then(|| JobId::from_parts(self.next_job, 1))
        }
        fn cancel(&mut self, job: JobId) {
            self.actions.push(Action::Cancel(job));
        }
        fn status(&mut self, owner: ClientId) {
            self.actions.push(Action::Status(owner));
        }
        fn cycle_start(&mut self, token: FormatCycleToken, delay: Duration) {
            self.actions.push(Action::Cycle(token, delay));
        }
        fn cycle_cancel(&mut self, token: FormatCycleToken) {
            self.actions.push(Action::CancelCycle(token));
        }
        fn redraw_status(&mut self, owner: ClientId) {
            self.actions.push(Action::Redraw(owner));
        }
    }

    fn owner() -> ClientId {
        ClientId::from_parts(3, 1)
    }
    fn get(jobs: &mut FormatJobs, rt: &mut Fixture, flags: FormatFlags, now: i64) -> ByteString {
        jobs.get(
            rt,
            Some(owner()),
            0,
            flags,
            b"echo #{value}",
            b"echo value",
            now,
        )
    }
    fn token(rt: &Fixture) -> FormatJobToken {
        rt.actions
            .iter()
            .rev()
            .find_map(|action| match action {
                Action::Run(_, _, _, token) => Some(token.clone()),
                _ => None,
            })
            .expect("launch token")
    }
    fn buffer(bytes: &[u8]) -> ByteBuffer {
        let mut buffer = ByteBuffer::new();
        buffer.add(bytes);
        buffer
    }
    fn out(jobs: &FormatJobs) -> &[u8] {
        jobs.record(Some(owner()), 0, b"echo #{value}")
            .unwrap()
            .out
            .as_ref()
            .unwrap()
            .as_bytes()
    }
    fn animate(
        jobs: &mut FormatJobs,
        rt: &mut Fixture,
        frames: &[u8],
        count: Option<&[u8]>,
        start_ms: u64,
    ) -> ByteString {
        jobs.cycle(
            rt,
            FormatCycleRequest {
                owner: Some(owner()),
                flags: FormatFlags::STATUS,
                no_cycle: false,
                frames,
                count,
                start_ms,
            },
        )
    }

    #[test]
    fn first_launch_uses_owner_expanded_shell_and_nowait() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        assert!(get(&mut jobs, &mut rt, FormatFlags::NONE, 0).is_empty());
        assert!(
            matches!(&rt.actions[0], Action::Run(Some(client), command, flags, _) if *client == owner() && command.as_bytes() == b"echo value" && *flags == JobFlags::NOWAIT)
        );
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn running_job_is_not_relaunched_and_not_ready_after_two_seconds() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        assert!(get(&mut jobs, &mut rt, FormatFlags::NONE, 11).is_empty());
        assert_eq!(
            get(&mut jobs, &mut rt, FormatFlags::NONE, 12),
            b"<'echo #{value}' not ready>".as_slice()
        );
        assert_eq!(rt.actions.len(), 1);
        assert_eq!(
            get(&mut jobs, &mut rt, FormatFlags::NONE, 13),
            b"<'echo #{value}' not ready>".as_slice()
        );
    }

    #[test]
    fn completion_suppresses_same_second_and_restarts_next_second() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        let callback = token(&rt);
        assert!(jobs.complete(&mut rt, &callback, &mut buffer(b"value")));
        assert_eq!(
            get(&mut jobs, &mut rt, FormatFlags::NONE, 10),
            b"value".as_slice()
        );
        assert_eq!(rt.actions.len(), 1);
        assert_eq!(
            get(&mut jobs, &mut rt, FormatFlags::NONE, 11),
            b"value".as_slice()
        );
        assert_eq!(rt.actions.len(), 2);
    }

    #[test]
    fn changed_command_and_force_cancel_before_launch_preserving_output() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        let first = token(&rt);
        jobs.update(&mut rt, &first, &mut buffer(b"old\n"), 10);
        let first_job = jobs
            .record(Some(owner()), 0, b"echo #{value}")
            .unwrap()
            .job
            .unwrap();
        assert_eq!(
            jobs.get(
                &mut rt,
                Some(owner()),
                0,
                FormatFlags::NONE,
                b"echo #{value}",
                b"echo changed",
                10
            ),
            b"old".as_slice()
        );
        assert_eq!(rt.actions[1], Action::Cancel(first_job));
        assert!(matches!(rt.actions[2], Action::Run(_, _, _, _)));
        let second = token(&rt);
        assert_ne!(first.generation, second.generation);
        assert!(
            !jobs
                .record(Some(owner()), 0, b"echo #{value}")
                .unwrap()
                .updated
        );
        jobs.get(
            &mut rt,
            Some(owner()),
            0,
            FormatFlags::FORCE,
            b"echo #{value}",
            b"echo changed",
            10,
        );
        assert!(matches!(rt.actions[3], Action::Cancel(_)));
        assert!(matches!(rt.actions[4], Action::Run(_, _, _, _)));
    }

    #[test]
    fn failed_launch_uses_raw_command_and_suppresses_same_second() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture {
            fail: true,
            ..Fixture::default()
        };
        assert_eq!(
            get(&mut jobs, &mut rt, FormatFlags::NONE, 10),
            b"<'echo #{value}' didn't start>".as_slice()
        );
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        assert_eq!(rt.actions.len(), 1);
        get(&mut jobs, &mut rt, FormatFlags::NONE, 11);
        assert_eq!(rt.actions.len(), 2);
    }

    #[test]
    fn nojobs_leaves_cache_and_runtime_untouched() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        assert!(get(&mut jobs, &mut rt, FormatFlags::NOJOBS, 10).is_empty());
        assert!(jobs.is_empty());
        assert!(rt.actions.is_empty());
    }

    #[test]
    fn update_consumes_all_complete_lines_and_preserves_partial_tail() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        let callback = token(&rt);
        let mut input = buffer(b"first\r\n\n\rlast\rtail");
        jobs.update(&mut rt, &callback, &mut input, 11);
        assert_eq!(out(&jobs), b"last");
        assert_eq!(input.data(), b"tail");
        assert!(
            jobs.record(Some(owner()), 0, b"echo #{value}")
                .unwrap()
                .updated
        );
        jobs.update(&mut rt, &callback, &mut input, 12);
        assert_eq!(out(&jobs), b"last");
        assert_eq!(input.data(), b"tail");
    }

    #[test]
    fn update_without_line_changes_nothing() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::STATUS, 10);
        let callback = token(&rt);
        let mut input = buffer(b"partial");
        jobs.update(&mut rt, &callback, &mut input, 12);
        let record = jobs.record(Some(owner()), 0, b"echo #{value}").unwrap();
        assert!(record.out.is_none());
        assert!(!record.updated);
        assert_eq!(record.last, 10);
        assert_eq!(rt.actions.len(), 1);
    }

    #[test]
    fn legacy_any_eol_coalesces_runs_but_split_crlf_is_two_lines() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        let callback = token(&rt);
        let mut input = buffer(b"value\r");
        jobs.update(&mut rt, &callback, &mut input, 10);
        assert_eq!(out(&jobs), b"value");
        input.add(b"\n");
        jobs.update(&mut rt, &callback, &mut input, 10);
        assert_eq!(out(&jobs), b"");
        input.add(b"one\n\n\n");
        jobs.update(&mut rt, &callback, &mut input, 10);
        assert_eq!(out(&jobs), b"one");
    }

    #[test]
    fn status_updates_are_once_per_wall_second_and_complete_always_redraws() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::STATUS, 10);
        let callback = token(&rt);
        jobs.update(&mut rt, &callback, &mut buffer(b"one\n"), 10);
        assert_eq!(rt.actions.len(), 1);
        jobs.update(&mut rt, &callback, &mut buffer(b"two\n"), 11);
        jobs.update(&mut rt, &callback, &mut buffer(b"three\n"), 11);
        assert_eq!(rt.actions[1..], [Action::Status(owner())]);
        assert_eq!(
            jobs.record(Some(owner()), 0, b"echo #{value}")
                .unwrap()
                .last,
            11
        );
        jobs.complete(&mut rt, &callback, &mut buffer(b""));
        assert_eq!(
            rt.actions[1..],
            [Action::Status(owner()), Action::Status(owner())]
        );
        assert!(
            !jobs
                .record(Some(owner()), 0, b"echo #{value}")
                .unwrap()
                .status
        );
        assert_eq!(out(&jobs), b"three");
    }

    #[test]
    fn global_status_updates_advance_last_without_client_redraw() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        jobs.get(&mut rt, None, 0, FormatFlags::STATUS, b"cmd", b"cmd", 10);
        let callback = token(&rt);
        jobs.update(&mut rt, &callback, &mut buffer(b"value\n"), 12);
        assert_eq!(jobs.record(None, 0, b"cmd").unwrap().last, 12);
        jobs.complete(&mut rt, &callback, &mut buffer(b""));
        assert_eq!(rt.actions.len(), 1);
    }

    #[test]
    fn completion_uses_first_line_not_last_and_keeps_remaining_buffer() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        let callback = token(&rt);
        let mut input = buffer(b"first\r\nsecond\npartial");
        jobs.complete(&mut rt, &callback, &mut input);
        assert_eq!(out(&jobs), b"first");
        assert_eq!(input.data(), b"second\npartial");
        assert!(
            jobs.record(Some(owner()), 0, b"echo #{value}")
                .unwrap()
                .job
                .is_none()
        );
    }

    #[test]
    fn completion_keeps_empty_after_update_but_replaces_empty_on_fresh_run() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        let first = token(&rt);
        jobs.update(&mut rt, &first, &mut buffer(b"old\n"), 10);
        jobs.complete(&mut rt, &first, &mut buffer(b"\nignored"));
        assert_eq!(out(&jobs), b"old");
        get(&mut jobs, &mut rt, FormatFlags::NONE, 11);
        let next = token(&rt);
        jobs.complete(&mut rt, &next, &mut buffer(b""));
        assert_eq!(out(&jobs), b"");
    }

    #[test]
    fn completion_copies_incomplete_binary_bytes_and_c_string_truncates() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, 10);
        let callback = token(&rt);
        let mut input = buffer(b"\xffvalue\0hidden");
        jobs.complete(&mut rt, &callback, &mut input);
        assert_eq!(out(&jobs), b"\xffvalue");
        assert_eq!(input.data(), b"\xffvalue\0hidden");
    }

    #[test]
    fn raw_command_expansion_and_lookup_end_at_first_nul() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        jobs.get(
            &mut rt,
            None,
            0,
            FormatFlags::NONE,
            b"raw\0hidden",
            b"expanded\0hidden",
            10,
        );
        assert!(
            matches!(&rt.actions[0], Action::Run(None, command, _, callback) if command.as_bytes() == b"expanded" && callback.raw == b"raw".as_slice())
        );
        jobs.get(&mut rt, None, 0, FormatFlags::NONE, b"raw", b"expanded", 10);
        assert_eq!(jobs.len(), 1);
        assert_eq!(rt.actions.len(), 1);
    }

    #[test]
    fn namespaces_tags_and_client_generations_do_not_alias() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        for (client, tag) in [
            (None, 0),
            (Some(owner()), 0),
            (Some(owner()), 1),
            (Some(ClientId::from_parts(3, 2)), 0),
        ] {
            jobs.get(&mut rt, client, tag, FormatFlags::NONE, b"cmd", b"cmd", 10);
        }
        assert_eq!(jobs.len(), 4);
        assert_eq!(rt.actions.len(), 4);
    }

    #[test]
    fn stale_and_duplicate_callbacks_never_consume_or_redraw() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::STATUS, 10);
        let old = token(&rt);
        get(
            &mut jobs,
            &mut rt,
            FormatFlags::FORCE | FormatFlags::STATUS,
            10,
        );
        let mut input = buffer(b"stale\n");
        let actions = rt.actions.len();
        assert!(!jobs.update(&mut rt, &old, &mut input, 11));
        assert!(!jobs.complete(&mut rt, &old, &mut input));
        assert_eq!(input.data(), b"stale\n");
        assert_eq!(rt.actions.len(), actions);
        let current = token(&rt);
        assert!(jobs.complete(&mut rt, &current, &mut input));
        assert!(!jobs.complete(&mut rt, &current, &mut input));
        assert!(!jobs.update(&mut rt, &current, &mut input, 12));
    }

    #[test]
    fn tidy_boundary_future_time_and_running_cancellation() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        for (tag, second) in [(0, 0), (1, 1), (2, 3601)] {
            jobs.get(
                &mut rt,
                None,
                tag,
                FormatFlags::NONE,
                b"cmd",
                b"cmd",
                second,
            );
        }
        jobs.tidy_jobs(&mut rt, 3599);
        assert_eq!(jobs.len(), 3);
        jobs.tidy_jobs(&mut rt, 3600);
        assert!(jobs.record(None, 0, b"cmd").is_none());
        assert!(jobs.record(None, 1, b"cmd").is_some());
        assert!(jobs.record(None, 2, b"cmd").is_some());
        assert_eq!(rt.actions[3], Action::Cancel(JobId::from_parts(1, 1)));
    }

    #[test]
    fn tidy_orders_tag_then_raw_bytes_and_preserves_other_owners() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        for (tag, raw) in [(1, b"a".as_slice()), (0, b"z"), (0, b"a")] {
            jobs.get(&mut rt, Some(owner()), tag, FormatFlags::NONE, raw, raw, 10);
        }
        jobs.get(
            &mut rt,
            None,
            0,
            FormatFlags::NONE,
            b"global",
            b"global",
            10,
        );
        jobs.tidy_owner(&mut rt, Some(owner()), 0, true);
        assert_eq!(
            rt.actions[4..],
            [
                Action::Cancel(JobId::from_parts(3, 1)),
                Action::Cancel(JobId::from_parts(2, 1)),
                Action::Cancel(JobId::from_parts(1, 1))
            ]
        );
        assert_eq!(jobs.len(), 1);
        assert!(jobs.record(None, 0, b"global").is_some());
    }

    #[test]
    fn cleanup_recreation_rejects_old_callback_generation() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::STATUS, 10);
        let old = token(&rt);
        jobs.lost_client(&mut rt, owner());
        assert!(jobs.is_empty());
        get(&mut jobs, &mut rt, FormatFlags::STATUS, 10);
        assert_ne!(old.generation, token(&rt).generation);
        let mut input = buffer(b"stale\n");
        assert!(!jobs.complete(&mut rt, &old, &mut input));
        assert_eq!(input.data(), b"stale\n");
    }

    #[test]
    fn backwards_clock_never_creates_not_ready_or_expires_future_records() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        get(&mut jobs, &mut rt, FormatFlags::NONE, i64::MAX);
        assert!(get(&mut jobs, &mut rt, FormatFlags::NONE, i64::MIN).is_empty());
        jobs.tidy_jobs(&mut rt, i64::MIN);
        assert_eq!(jobs.len(), 1);
        jobs.tidy_jobs(&mut rt, i64::MAX);
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn animation_boundaries_count_bounds_and_raw_empty_frames() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        for (time, expected) in [
            (0, b"a".as_slice()),
            (99, b"a"),
            (100, b""),
            (200, b"#{raw}"),
            (300, b""),
        ] {
            assert_eq!(
                animate(&mut jobs, &mut rt, b"a,,#{raw},", None, time),
                expected
            );
        }
        assert_eq!(
            animate(&mut jobs, &mut rt, b"a,b", Some(b"2"), 199),
            b"a".as_slice()
        );
        assert_eq!(
            animate(&mut jobs, &mut rt, b"a,b", Some(b"2"), 200),
            b"b".as_slice()
        );
        assert_eq!(
            animate(&mut jobs, &mut rt, b"a,b", Some(b"100"), 9999),
            b"a".as_slice()
        );
        assert_eq!(
            animate(&mut jobs, &mut rt, b"a,b", Some(b"100"), 10000),
            b"b".as_slice()
        );
        for invalid in [b"0".as_slice(), b"101", b"-1", b"junk", b"", b"1 "] {
            assert_eq!(
                animate(&mut jobs, &mut rt, b"a,b", Some(invalid), 100),
                b"b".as_slice()
            );
        }
        assert_eq!(rt.actions.len(), 1);
    }

    #[test]
    fn animation_gating_single_frame_missing_owner_and_nul() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        for (flags, no_cycle, frames) in [
            (FormatFlags::NONE, false, b"a,b".as_slice()),
            (FormatFlags::STATUS, true, b"a,b"),
            (FormatFlags::STATUS, false, b""),
        ] {
            assert!(
                jobs.cycle(
                    &mut rt,
                    FormatCycleRequest {
                        owner: Some(owner()),
                        flags,
                        no_cycle,
                        frames,
                        count: None,
                        start_ms: 100
                    }
                )
                .is_empty()
            );
        }
        assert_eq!(
            animate(&mut jobs, &mut rt, b"single\0,b", None, 100),
            b"single".as_slice()
        );
        assert_eq!(
            jobs.cycle(
                &mut rt,
                FormatCycleRequest {
                    owner: None,
                    flags: FormatFlags::STATUS,
                    no_cycle: false,
                    frames: b"a,b",
                    count: None,
                    start_ms: 100
                }
            ),
            b"b".as_slice()
        );
        assert!(rt.actions.is_empty());
    }

    #[test]
    fn animation_arms_one_timer_expiry_only_redraws_then_render_rearms() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        animate(&mut jobs, &mut rt, b"a,b", None, 0);
        animate(&mut jobs, &mut rt, b"a,b", None, 10);
        let first = match rt.actions[0] {
            Action::Cycle(token, delay) => {
                assert_eq!(delay, Duration::from_millis(100));
                token
            }
            _ => panic!("cycle action"),
        };
        assert_eq!(rt.actions.len(), 1);
        assert!(jobs.cycle_pending(owner()));
        assert!(jobs.cycle_complete(&mut rt, first, false, false));
        assert!(!jobs.cycle_pending(owner()));
        assert_eq!(rt.actions[1], Action::Redraw(owner()));
        assert_eq!(rt.actions.len(), 2);
        animate(&mut jobs, &mut rt, b"a,b", None, 100);
        assert!(matches!(rt.actions[2], Action::Cycle(_, _)));
        assert!(!jobs.cycle_complete(&mut rt, first, false, false));
        assert!(jobs.cycle_pending(owner()));
    }

    #[test]
    fn animation_message_prompt_suppression_and_client_loss_order() {
        let mut jobs = FormatJobs::new();
        let mut rt = Fixture::default();
        for (message, prompt) in [(true, false), (false, true), (true, true)] {
            jobs.cycle_start(&mut rt, owner());
            let callback = match rt.actions.last().unwrap() {
                Action::Cycle(token, _) => *token,
                _ => panic!("cycle action"),
            };
            assert!(jobs.cycle_complete(&mut rt, callback, message, prompt));
            assert!(!jobs.cycle_pending(owner()));
        }
        assert_eq!(rt.actions.len(), 3);
        get(&mut jobs, &mut rt, FormatFlags::STATUS, 10);
        let old_job = token(&rt);
        jobs.cycle_start(&mut rt, owner());
        let old_cycle = match rt.actions.last().unwrap() {
            Action::Cycle(token, _) => *token,
            _ => panic!("cycle action"),
        };
        jobs.lost_client(&mut rt, owner());
        assert_eq!(rt.actions[5], Action::CancelCycle(old_cycle));
        assert!(matches!(rt.actions[6], Action::Cancel(_)));
        assert!(!jobs.cycle_complete(&mut rt, old_cycle, false, false));
        assert!(!jobs.update(&mut rt, &old_job, &mut buffer(b"stale\n"), 11));
        assert!(jobs.is_empty());
        jobs.lost_client(&mut rt, owner());
        assert_eq!(rt.actions.len(), 7);
    }
}
