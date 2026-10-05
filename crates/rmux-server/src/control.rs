// Ported from tmux control.c, control-notify.c @ 8f25579c
/*
 * Copyright (c) 2012 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Copyright (c) 2012 George Nachman <tmux@georgester.com>
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
use crate::ids::PaneId;
use crate::model::pane::PaneOffset;
use std::collections::{BTreeMap, VecDeque};

mod events;
pub mod input;
pub mod io;
pub mod monitor;
pub mod notify;
mod runtime;
pub use events::build_events;
pub use monitor::{add_sub, monitor_timer, remove_sub};
pub use runtime::*;

pub const BUFFER_LOW: usize = 512;
pub const BUFFER_HIGH: usize = 8192;
pub const REPLY_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlGuard {
    Begin,
    End,
    Error,
}
impl ControlGuard {
    fn spelling(self) -> &'static str {
        match self {
            Self::Begin => "begin",
            Self::End => "end",
            Self::Error => "error",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlPaneFlags {
    pub off: bool,
    pub paused: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlOffsetStatus {
    pub offset: Option<PaneOffset>,
    pub suppress_read: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockId {
    slot: usize,
    generation: u64,
}
#[derive(Debug)]
enum Payload {
    Raw {
        pane: u32,
        remaining: usize,
        time: u64,
    },
    Line(Vec<u8>),
}
#[derive(Debug)]
struct Block {
    payload: Payload,
    prev: Option<BlockId>,
    next: Option<BlockId>,
    pane_prev: Option<BlockId>,
    pane_next: Option<BlockId>,
}
#[derive(Debug)]
struct Slot {
    generation: u64,
    block: Option<Block>,
}
#[derive(Debug)]
pub struct ControlPane {
    pub pane: PaneId,
    pub sent: PaneOffset,
    pub queued: PaneOffset,
    pub flags: ControlPaneFlags,
    head: Option<BlockId>,
    tail: Option<BlockId>,
    pending: bool,
}
/// Provider resolves membership and fd state on every service call, and retains
/// the pane buffer at the consumer's sent offset until encoding completes.
pub trait ControlTransport {
    fn pane_bytes(&self, pane: PaneId, offset: PaneOffset) -> Option<&[u8]>;
    fn now_ms(&self) -> u64;
}
pub struct ControlState {
    pub panes: BTreeMap<u32, ControlPane>,
    pub windows: BTreeMap<u32, (u32, u32)>,
    slots: Vec<Slot>,
    free: Vec<usize>,
    head: Option<BlockId>,
    tail: Option<BlockId>,
    pending: VecDeque<u32>,
    deferred: VecDeque<Vec<u8>>,
    pub queued_reply_bytes: usize,
    pub guard_depth: usize,
    pub output: Vec<u8>,
    scratch: Vec<u8>,
    pub read_enabled: bool,
    pub write_enabled: bool,
    pub exiting: bool,
    pub discard_replies: bool,
    pub exit_reason: Option<&'static str>,
    pub no_output: bool,
    pub ignore_output: bool,
    pub pause_after: Option<u64>,
    pub io: Option<io::ControlIo>,
    pub input: input::ControlInput,
    pub monitors: Option<crate::ids::MonitorSetId>,
    direct_drive: Option<(crate::ids::TimerId, u64)>,
}
impl Default for ControlState {
    fn default() -> Self {
        Self::new(false)
    }
}
impl ControlState {
    pub fn new(double_control: bool) -> Self {
        Self {
            panes: BTreeMap::new(),
            windows: BTreeMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            head: None,
            tail: None,
            pending: VecDeque::new(),
            deferred: VecDeque::new(),
            queued_reply_bytes: 0,
            guard_depth: 0,
            output: if double_control {
                b"\x1bP1000p".to_vec()
            } else {
                Vec::new()
            },
            scratch: Vec::new(),
            read_enabled: false,
            write_enabled: double_control,
            exiting: false,
            discard_replies: false,
            exit_reason: None,
            no_output: false,
            ignore_output: false,
            pause_after: None,
            io: None,
            input: input::ControlInput::default(),
            monitors: None,
            direct_drive: None,
        }
    }
    pub fn ready(&mut self) {
        self.read_enabled = true;
    }
    fn block(&self, id: BlockId) -> &Block {
        let slot = &self.slots[id.slot];
        assert_eq!(slot.generation, id.generation);
        slot.block.as_ref().expect("live control block")
    }
    fn block_mut(&mut self, id: BlockId) -> &mut Block {
        let slot = &mut self.slots[id.slot];
        assert_eq!(slot.generation, id.generation);
        slot.block.as_mut().expect("live control block")
    }
    fn insert(&mut self, payload: Payload) -> BlockId {
        let pane = match &payload {
            Payload::Raw { pane, .. } => Some(*pane),
            _ => None,
        };
        let index = self.free.pop().unwrap_or_else(|| {
            self.slots.push(Slot {
                generation: 0,
                block: None,
            });
            self.slots.len() - 1
        });
        let id = BlockId {
            slot: index,
            generation: self.slots[index].generation,
        };
        let pane_prev = pane.and_then(|p| self.panes[&p].tail);
        self.slots[index].block = Some(Block {
            payload,
            prev: self.tail,
            next: None,
            pane_prev,
            pane_next: None,
        });
        if let Some(tail) = self.tail {
            self.block_mut(tail).next = Some(id);
        } else {
            self.head = Some(id);
        }
        self.tail = Some(id);
        if let Some(p) = pane {
            if let Some(tail) = pane_prev {
                self.block_mut(tail).pane_next = Some(id);
            } else {
                self.panes.get_mut(&p).unwrap().head = Some(id);
            }
            self.panes.get_mut(&p).unwrap().tail = Some(id);
        }
        id
    }
    fn remove(&mut self, id: BlockId) -> Payload {
        let b = self.slots[id.slot].block.take().expect("live block");
        if let Some(prev) = b.prev {
            self.block_mut(prev).next = b.next;
        } else {
            self.head = b.next;
        }
        if let Some(next) = b.next {
            self.block_mut(next).prev = b.prev;
        } else {
            self.tail = b.prev;
        }
        match &b.payload {
            Payload::Raw { pane, .. } => {
                if let Some(prev) = b.pane_prev {
                    self.block_mut(prev).pane_next = b.pane_next;
                } else {
                    self.panes.get_mut(pane).unwrap().head = b.pane_next;
                }
                if let Some(next) = b.pane_next {
                    self.block_mut(next).pane_prev = b.pane_prev;
                } else {
                    self.panes.get_mut(pane).unwrap().tail = b.pane_prev;
                }
            }
            Payload::Line(line) => {
                self.queued_reply_bytes = self.queued_reply_bytes.saturating_sub(line.len() + 1)
            }
        }
        self.slots[id.slot].generation = self.slots[id.slot]
            .generation
            .checked_add(1)
            .expect("control block generation overflow");
        self.free.push(id.slot);
        b.payload
    }
    fn discard_pane(&mut self, public: u32) {
        while let Some(head) = self.panes.get(&public).and_then(|p| p.head) {
            self.remove(head);
        }
    }
    fn discard_raw(&mut self) {
        let mut next = self.head;
        while let Some(id) = next {
            let b = self.block(id);
            next = b.next;
            if matches!(b.payload, Payload::Raw { .. }) {
                self.remove(id);
            }
        }
    }
    fn evict(&mut self) {
        if !self.exiting {
            self.exit_reason = Some("too far behind");
            self.exiting = true;
        }
        self.discard();
    }
    fn buffered_len(&self) -> usize {
        self.output
            .len()
            .saturating_add(self.io.as_ref().map_or(0, io::ControlIo::output_len))
    }
    pub fn write(&mut self, text: &[u8]) {
        let text = cstring(text);
        if self.discard_replies {
            return;
        }
        if self
            .buffered_len()
            .saturating_add(self.queued_reply_bytes)
            .saturating_add(text.len())
            .saturating_add(1)
            >= REPLY_LIMIT
        {
            self.evict();
            self.discard_replies = true;
            return;
        }
        if self.head.is_none() {
            self.output.extend_from_slice(text);
            self.output.push(b'\n');
        } else {
            self.queued_reply_bytes += text.len() + 1;
            self.insert(Payload::Line(text.to_vec()));
        }
        self.write_enabled = true;
    }
    pub fn notify_write(&mut self, text: &[u8]) {
        if self.guard_depth == 0 {
            self.write(text);
        } else {
            self.deferred.push_back(cstring(text).to_vec());
        }
    }
    pub fn write_guard(&mut self, guard: ControlGuard, time: i64, number: u32, flags: i32) {
        if guard == ControlGuard::Begin {
            self.guard_depth += 1;
        }
        self.write(format!("%{} {time} {number} {flags}", guard.spelling()).as_bytes());
        if guard != ControlGuard::Begin {
            self.guard_depth = self.guard_depth.saturating_sub(1);
            if self.guard_depth == 0 {
                while let Some(line) = self.deferred.pop_front() {
                    self.write(&line);
                }
            }
        }
    }
    pub fn add_pane(&mut self, public: u32, pane: PaneId, offset: PaneOffset) {
        self.panes.entry(public).or_insert(ControlPane {
            pane,
            sent: offset,
            queued: offset,
            flags: ControlPaneFlags::default(),
            head: None,
            tail: None,
            pending: false,
        });
    }
    pub fn pane_offset(&self, public: u32) -> ControlOffsetStatus {
        if self.no_output {
            return ControlOffsetStatus::default();
        }
        let Some(p) = self.panes.get(&public) else {
            return ControlOffsetStatus::default();
        };
        if p.flags.paused {
            return ControlOffsetStatus::default();
        }
        if p.flags.off {
            return ControlOffsetStatus {
                offset: None,
                suppress_read: true,
            };
        }
        ControlOffsetStatus {
            offset: Some(p.sent),
            suppress_read: self.buffered_len() >= BUFFER_LOW,
        }
    }
    pub fn reset_pane(&mut self, public: u32, offset: PaneOffset) {
        if !self.panes.contains_key(&public) {
            return;
        }
        self.discard_pane(public);
        let p = self.panes.get_mut(&public).unwrap();
        p.sent = offset;
        p.queued = offset;
    }
    pub fn set_pane_off(&mut self, public: u32, pane: PaneId, offset: PaneOffset) {
        self.add_pane(public, pane, offset);
        self.reset_pane(public, offset);
        self.panes.get_mut(&public).unwrap().flags.off = true;
    }
    pub fn set_pane_on(&mut self, public: u32, offset: PaneOffset) {
        if !self.panes.get(&public).is_some_and(|p| p.flags.off) {
            return;
        }
        let p = self.panes.get_mut(&public).unwrap();
        p.flags.off = false;
        p.sent = offset;
        p.queued = offset;
    }
    pub fn pause_pane(&mut self, public: u32, pane: PaneId, offset: PaneOffset) {
        self.add_pane(public, pane, offset);
        if self.panes[&public].flags.paused {
            return;
        }
        self.panes.get_mut(&public).unwrap().flags.paused = true;
        self.discard_pane(public);
        self.notify_write(format!("%pause %{public}").as_bytes());
    }
    pub fn continue_pane(&mut self, public: u32, offset: PaneOffset) {
        if !self.panes.get(&public).is_some_and(|p| p.flags.paused) {
            return;
        }
        let p = self.panes.get_mut(&public).unwrap();
        p.flags.paused = false;
        p.sent = offset;
        p.queued = offset;
        self.notify_write(format!("%continue %{public}").as_bytes());
    }
    pub fn reset_offsets(&mut self) {
        self.discard_raw();
        self.panes.clear();
        self.pending.clear();
    }
    pub fn rebase_offsets(&mut self, pane: PaneId, amount: u64) {
        for p in self.panes.values_mut().filter(|p| p.pane == pane) {
            p.sent.used = p.sent.used.checked_sub(amount).expect("sent offset rebase");
            p.queued.used = p
                .queued
                .used
                .checked_sub(amount)
                .expect("queued offset rebase");
        }
    }
    fn check_age(&mut self, public: u32, now: u64) -> bool {
        let Some(id) = self.panes[&public].head else {
            return false;
        };
        let Payload::Raw { time, .. } = self.block(id).payload else {
            unreachable!()
        };
        if time >= now {
            return false;
        }
        if now - time < self.pause_after.unwrap_or(300_000) {
            return false;
        }
        if self.pause_after.is_some() {
            let p = &self.panes[&public];
            let pane = p.pane;
            let offset = p.sent;
            self.pause_pane(public, pane, offset);
        } else {
            self.evict();
        }
        true
    }
    /// Called before the VT parser advances the parser offset. `end` is the
    /// absolute end of the shared pane input buffer, not the parser position.
    pub fn write_output(
        &mut self,
        public: u32,
        pane: PaneId,
        parser: PaneOffset,
        end: PaneOffset,
        now: u64,
    ) {
        if self.no_output || self.ignore_output || self.exiting {
            if let Some(p) = self.panes.get_mut(&public) {
                p.sent = end;
                p.queued = end;
            }
            return;
        }
        self.add_pane(public, pane, parser);
        if self.panes[&public].flags.off || self.panes[&public].flags.paused {
            let p = self.panes.get_mut(&public).unwrap();
            p.sent = end;
            p.queued = end;
            return;
        }
        if self.check_age(public, now) {
            return;
        }
        let queued = self.panes[&public].queued;
        let size = usize::try_from(
            end.used
                .checked_sub(queued.used)
                .expect("pane queue offset"),
        )
        .expect("pane block size");
        if size == 0 {
            return;
        }
        self.insert(Payload::Raw {
            pane: public,
            remaining: size,
            time: now,
        });
        let p = self.panes.get_mut(&public).unwrap();
        p.queued = end;
        if !p.pending {
            p.pending = true;
            self.pending.push_back(public);
        }
        self.write_enabled = true;
    }
    fn flush_lines(&mut self) {
        while let Some(id) = self.head {
            if !matches!(self.block(id).payload, Payload::Line(_)) {
                break;
            }
            let Payload::Line(line) = self.remove(id) else {
                unreachable!()
            };
            self.output.extend_from_slice(&line);
            self.output.push(b'\n');
        }
    }
    fn service_pane(&mut self, public: u32, limit: usize, host: &impl ControlTransport) {
        let pane = self.panes[&public].pane;
        if host.pane_bytes(pane, self.panes[&public].sent).is_none() {
            self.discard_pane(public);
            self.flush_lines();
            return;
        }
        let now = host.now_ms();
        let mut written = 0;
        self.scratch.clear();
        while written < limit {
            let Some(id) = self.panes[&public].head else {
                break;
            };
            if self.check_age(public, host.now_ms()) {
                self.scratch.clear();
                return;
            }
            let Payload::Raw {
                remaining, time, ..
            } = self.block(id).payload
            else {
                unreachable!()
            };
            if self.scratch.is_empty() {
                let header = if self.pause_after.is_some() {
                    format!("%extended-output %{public} {} : ", now.saturating_sub(time))
                } else {
                    format!("%output %{public} ")
                };
                self.scratch.extend_from_slice(header.as_bytes());
            }
            let n = remaining.min(limit - written);
            let bytes = host
                .pane_bytes(pane, self.panes[&public].sent)
                .expect("pane retained during service");
            assert!(bytes.len() >= n, "control block exceeds retained pane data");
            encode_into(&bytes[..n], &mut self.scratch);
            self.panes.get_mut(&public).unwrap().sent.used += n as u64;
            written += n;
            if n == remaining {
                self.remove(id);
                if self
                    .head
                    .is_some_and(|head| matches!(self.block(head).payload, Payload::Line(_)))
                {
                    self.finish_frame();
                    self.flush_lines();
                }
            } else {
                let Payload::Raw { remaining, .. } = &mut self.block_mut(id).payload else {
                    unreachable!()
                };
                *remaining -= n;
            }
        }
        self.finish_frame();
    }
    fn finish_frame(&mut self) {
        if !self.scratch.is_empty() {
            self.output.append(&mut self.scratch);
            self.output.push(b'\n');
        }
    }
    pub fn service(&mut self, host: &impl ControlTransport) {
        if !self.write_enabled {
            return;
        }
        self.flush_lines();
        while self.buffered_len() < BUFFER_HIGH && !self.pending.is_empty() {
            let limit = ((BUFFER_HIGH - self.buffered_len()) / self.pending.len() / 3).max(32);
            let mut index = 0;
            while index < self.pending.len() {
                if self.buffered_len() >= BUFFER_HIGH {
                    break;
                }
                let public = self.pending[index];
                self.service_pane(public, limit, host);
                if self.panes[&public].head.is_none() {
                    self.panes.get_mut(&public).unwrap().pending = false;
                    self.pending.remove(index);
                } else {
                    index += 1;
                }
            }
        }
        if self.buffered_len() == 0 {
            self.write_enabled = false;
        }
    }
    pub fn consume_output(&mut self, count: usize) {
        assert!(count <= self.output.len());
        self.output.drain(..count);
    }
    pub fn all_done(&self) -> bool {
        self.head.is_none() && self.output.is_empty()
    }
    pub fn discard(&mut self) {
        self.discard_raw();
        self.read_enabled = false;
    }
    pub fn discard_all(&mut self) {
        self.discard();
        while let Some(id) = self.head {
            self.remove(id);
        }
        self.queued_reply_bytes = 0;
        self.write_enabled = false;
    }
    pub fn set_window_size(&mut self, public: u32, width: u32, height: u32) {
        self.windows.insert(public, (width, height));
    }
    pub fn get_window_size(&self, public: u32) -> Option<(u32, u32)> {
        self.windows.get(&public).copied()
    }
    pub fn clear_window_size(&mut self, public: u32) {
        self.windows.remove(&public);
    }
}
fn cstring(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())]
}
pub fn encode_into(bytes: &[u8], out: &mut Vec<u8>) {
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b < b' ' || b == b'\\' {
            out.extend_from_slice(&bytes[start..i]);
            out.extend_from_slice(&[
                b'\\',
                b'0' + (b >> 6),
                b'0' + ((b >> 3) & 7),
                b'0' + (b & 7),
            ]);
            start = i + 1;
        }
    }
    out.extend_from_slice(&bytes[start..]);
}
/// Late producer calls resolve the optional state rather than retaining it.
pub fn write_state(state: &mut Option<ControlState>, text: &[u8]) {
    if let Some(s) = state {
        s.write(text);
    }
}
pub fn notify_write_state(state: &mut Option<ControlState>, text: &[u8]) {
    if let Some(s) = state {
        s.notify_write(text);
    }
}
pub fn write_guard_state(
    state: &mut Option<ControlState>,
    guard: ControlGuard,
    time: i64,
    number: u32,
    flags: i32,
) {
    if let Some(s) = state {
        s.write_guard(guard, time, number, flags);
    }
}
pub fn stop_state(state: &mut Option<ControlState>) {
    *state = None;
}
#[cfg(test)]
mod tests;
