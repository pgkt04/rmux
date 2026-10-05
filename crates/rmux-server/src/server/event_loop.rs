// Ported from tmux proc.c, server.c @ 8f25579c
// Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
// Permission to use, copy, modify, and distribute this software for any purpose
// with or without fee is hereby granted, provided that the above copyright
// notice and this permission notice appear in all copies.
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
use crate::ids::*;
use mio::{Events, Interest, Poll, Token, unix::SourceFd};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    io,
    os::fd::{AsRawFd, BorrowedFd, RawFd},
    time::{Duration, Instant},
};

pub type LoopError = io::Error;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoopAction {
    Accept,
    AcceptBackoff,
    Peer(PeerId),
    Pane(PaneId),
    PanePipe(PaneId),
    Job(JobId),
    File(ClientFileId),
    FileDone(ClientFileId),
    FilePush(ClientFileId),
    ControlRead(ClientId),
    ControlWrite(ClientId),
    ClientRepeatTimer(ClientId),
    ClientClickTimer(ClientId),
    ClientExitTimer(ClientId),
    ClientFree(ClientId),
    ClientCycleTimer(ClientId),
    SessionFree(SessionId),
    SessionLock(SessionId),
    WindowName(WindowId),
    WindowSilence(WindowId),
    PaneInputTimer(PaneId, crate::model::pane_input::InputTimer),
    PaneScrollbar(PaneId),
    AlertsCheck,
    PaneResizeTimer(PaneId),
    RedrawTimer,
    StatusTimer(ClientId),
    MessageTimer(ClientId),
    ControlMonitor(MonitorSetId),
    Tidy,
    Signal,
    Deferred(u64),
}
#[derive(Clone, Debug)]
pub struct LoopReady {
    pub action: LoopAction,
    pub readable: bool,
    pub writable: bool,
}
struct Registration {
    fd: RawFd,
    action: LoopAction,
    read: bool,
    write: bool,
    registered: bool,
    null: bool,
}
struct Timer {
    action: LoopAction,
}
pub struct EventLoop {
    poll: Poll,
    events: Events,
    registrations: Arena<Registration, EventToken>,
    tokens: BTreeMap<usize, EventToken>,
    timers: Arena<Timer, TimerId>,
    deadlines: BinaryHeap<Reverse<(Instant, u64, TimerId)>>,
    sequence: u64,
    immediate: Vec<EventToken>,
}
fn interest(read: bool, write: bool) -> Option<Interest> {
    match (read, write) {
        (true, true) => Some(Interest::READABLE | Interest::WRITABLE),
        (true, false) => Some(Interest::READABLE),
        (false, true) => Some(Interest::WRITABLE),
        _ => None,
    }
}
fn token(id: EventToken) -> usize {
    let (slot, generation) = id.parts();
    ((generation as usize) << 32) | (slot as usize)
}
fn arena_error(e: ArenaError) -> io::Error {
    io::Error::other(e)
}
impl EventLoop {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            poll: Poll::new()?,
            events: Events::with_capacity(256),
            registrations: Arena::new(),
            tokens: BTreeMap::new(),
            timers: Arena::new(),
            deadlines: BinaryHeap::new(),
            sequence: 0,
            immediate: Vec::new(),
        })
    }
    pub fn register(
        &mut self,
        fd: BorrowedFd<'_>,
        read: bool,
        write: bool,
        action: LoopAction,
    ) -> io::Result<EventToken> {
        let raw = fd.as_raw_fd();
        if rmux_sys::server::descriptor_is_regular(fd)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "regular files require direct bounded I/O",
            ));
        }
        let id = self
            .registrations
            .insert(Registration {
                fd: raw,
                action,
                read,
                write,
                registered: false,
                null: false,
            })
            .map_err(arena_error)?;
        let key = token(id);
        if let Some(interest) = interest(read, write) {
            match self
                .poll
                .registry()
                .register(&mut SourceFd(&raw), Token(key), interest)
            {
                Ok(()) => {
                    self.registrations
                        .get_mut(id)
                        .expect("new registration")
                        .registered = true
                }
                Err(e) if matches!(e.raw_os_error(), Some(libc::EINVAL | libc::EPERM)) => {
                    self.registrations
                        .get_mut(id)
                        .expect("new registration")
                        .null = true;
                    self.immediate.push(id);
                }
                Err(e) => {
                    let _ = self.registrations.request_remove(id);
                    return Err(e);
                }
            }
        }
        self.tokens.insert(key, id);
        Ok(id)
    }
    pub fn is_null(&self, id: EventToken) -> bool {
        self.registrations.get(id).is_some_and(|r| r.null)
    }
    pub fn reregister(&mut self, id: EventToken, read: bool, write: bool) -> io::Result<()> {
        let r = self
            .registrations
            .get_mut(id)
            .ok_or_else(|| arena_error(ArenaError::StaleId))?;
        if r.null {
            r.read = read;
            r.write = write;
            if read || write {
                self.immediate.push(id);
            }
            return Ok(());
        }
        let mut source = SourceFd(&r.fd);
        match (r.registered, interest(read, write)) {
            (true, Some(i)) => self
                .poll
                .registry()
                .reregister(&mut source, Token(token(id)), i)?,
            (false, Some(i)) => {
                match self
                    .poll
                    .registry()
                    .register(&mut source, Token(token(id)), i)
                {
                    Ok(()) => r.registered = true,
                    Err(e) if matches!(e.raw_os_error(), Some(libc::EINVAL | libc::EPERM)) => {
                        r.null = true;
                        self.immediate.push(id);
                    }
                    Err(e) => return Err(e),
                }
            }
            (true, None) => {
                self.poll.registry().deregister(&mut source)?;
                r.registered = false;
            }
            (false, None) => {}
        }
        r.read = read;
        r.write = write;
        Ok(())
    }
    pub fn deregister(&mut self, id: EventToken) {
        if let Some(r) = self.registrations.get(id) {
            if r.registered {
                let _ = self.poll.registry().deregister(&mut SourceFd(&r.fd));
            }
        }
        self.tokens.remove(&token(id));
        let _ = self.registrations.request_remove(id);
    }
    pub fn schedule(&mut self, delay: Duration, action: LoopAction) -> TimerId {
        self.schedule_at(Instant::now() + delay, action)
    }
    pub fn schedule_at(&mut self, deadline: Instant, action: LoopAction) -> TimerId {
        let id = self
            .timers
            .insert(Timer { action })
            .expect("timer arena exhaustion");
        let seq = self.sequence;
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("timer sequence exhaustion");
        self.deadlines.push(Reverse((deadline, seq, id)));
        id
    }
    pub fn cancel(&mut self, id: TimerId) {
        let _ = self.timers.request_remove(id);
    }
    pub fn timer_action(&self, id: TimerId) -> Option<&LoopAction> {
        self.timers.get(id).map(|timer| &timer.action)
    }
    fn expired(&mut self, now: Instant, out: &mut Vec<LoopReady>) {
        while self
            .deadlines
            .peek()
            .is_some_and(|Reverse((at, _, _))| *at <= now)
        {
            let Reverse((_, _, id)) = self.deadlines.pop().expect("timer heap");
            if let Ok(Some(timer)) = self.timers.request_remove(id) {
                out.push(LoopReady {
                    action: timer.action,
                    readable: false,
                    writable: false,
                });
            }
        }
    }
    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<Vec<LoopReady>> {
        let mut out = Vec::new();
        self.poll_into(timeout, &mut out)?;
        Ok(out)
    }
    pub fn poll_into(
        &mut self,
        timeout: Option<Duration>,
        out: &mut Vec<LoopReady>,
    ) -> io::Result<()> {
        out.clear();
        while self
            .deadlines
            .peek()
            .is_some_and(|Reverse((_, _, id))| self.timers.get(*id).is_none())
        {
            self.deadlines.pop();
        }
        let now = Instant::now();
        let timer_wait = self
            .deadlines
            .peek()
            .map(|Reverse((at, _, _))| at.saturating_duration_since(now));
        let timeout = match (timeout, timer_wait) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let timeout = if self.immediate.is_empty() {
            timeout
        } else {
            Some(Duration::ZERO)
        };
        if let Err(e) = self.poll.poll(&mut self.events, timeout) {
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
        for event in &self.events {
            if let Some(r) = self
                .tokens
                .get(&event.token().0)
                .and_then(|id| self.registrations.get(*id))
            {
                out.push(LoopReady {
                    action: r.action.clone(),
                    readable: r.read
                        && (event.is_readable() || event.is_read_closed() || event.is_error()),
                    writable: r.write
                        && (event.is_writable() || event.is_write_closed() || event.is_error()),
                });
            }
        }
        for id in self.immediate.drain(..) {
            if let Some(r) = self.registrations.get(id) {
                out.push(LoopReady {
                    action: r.action.clone(),
                    readable: r.read,
                    writable: r.write,
                });
            }
        }
        self.expired(Instant::now(), out);
        Ok(())
    }
}
impl Default for EventLoop {
    fn default() -> Self {
        Self::new().expect("creating mio event loop")
    }
}
pub fn schedule_deferred(
    server: &mut crate::model::Server,
    delay: Duration,
    callback: Box<dyn FnOnce(&mut crate::model::Server)>,
) -> (TimerId, u64) {
    let id = server.next_deferred;
    server.next_deferred = server
        .next_deferred
        .checked_add(1)
        .expect("deferred action exhaustion");
    server.deferred.insert(id, callback);
    let timer = server.event_loop.schedule(delay, LoopAction::Deferred(id));
    (timer, id)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deferred_interest_discovers_null_endpoint() {
        use std::os::fd::AsFd;
        let fd = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut e = EventLoop::new().unwrap();
        let id = e
            .register(fd.as_fd(), false, false, LoopAction::Deferred(1))
            .unwrap();
        e.reregister(id, false, true).unwrap();
        let ready = e.poll(Some(Duration::ZERO)).unwrap();
        assert!(ready.iter().any(|r| r.writable));
        e.deregister(id);
    }
    #[test]
    fn cancelled_timer_and_reused_slot_never_dispatch_old_action() {
        let mut e = EventLoop::new().unwrap();
        let at = Instant::now();
        let old = e.schedule_at(at, LoopAction::Deferred(1));
        e.cancel(old);
        let fresh = e.schedule_at(at, LoopAction::Deferred(2));
        assert_ne!(old, fresh);
        let mut ready = Vec::new();
        e.expired(at, &mut ready);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].action, LoopAction::Deferred(2));
    }
    #[test]
    fn equal_deadlines_follow_insertion_order_and_zero_is_deferred() {
        let mut e = EventLoop::new().unwrap();
        let at = Instant::now();
        let mut out = Vec::new();
        for n in 0..5 {
            e.schedule_at(at, LoopAction::Deferred(n));
        }
        assert!(out.is_empty());
        e.expired(at, &mut out);
        assert_eq!(
            out.iter().map(|r| r.action.clone()).collect::<Vec<_>>(),
            (0..5).map(LoopAction::Deferred).collect::<Vec<_>>()
        );
    }
    #[test]
    fn stale_registration_cannot_change_reused_fd() {
        use std::os::fd::AsFd;
        let (a, _) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut e = EventLoop::new().unwrap();
        let old = e
            .register(a.as_fd(), true, false, LoopAction::Deferred(1))
            .unwrap();
        e.deregister(old);
        let fresh = e
            .register(a.as_fd(), true, false, LoopAction::Deferred(2))
            .unwrap();
        assert_ne!(old, fresh);
        assert!(e.reregister(old, false, true).is_err());
    }
    #[test]
    fn null_endpoint_never_enters_kernel_wait() {
        use std::os::fd::AsFd;
        let fd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut e = EventLoop::new().unwrap();
        let id = e
            .register(fd.as_fd(), true, true, LoopAction::Deferred(1))
            .unwrap();
        if e.is_null(id) {
            let ready = e.poll(Some(Duration::ZERO)).unwrap();
            assert_eq!(ready.len(), 1);
            assert!(ready[0].readable && ready[0].writable);
        }
        e.deregister(id);
    }
}
