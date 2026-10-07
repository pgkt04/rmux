// Ported from tmux tty.c @ 8f25579c
// TSP extension: bounded transactions sharing cell-output order.
use super::{Tty, TtyEffect, TtyFlags, TtyTimer};
use crate::keys::Da1Owner;
use std::collections::VecDeque;
use std::io;

pub const MAX_PROTOCOL_BYTES: usize = 24 * 1024 * 1024 + 64 * 1024;
pub const PROTOCOL_CONTROL_RESERVE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueFull;

impl std::fmt::Display for QueueFull {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tty protocol queue is full or closing")
    }
}

impl std::error::Error for QueueFull {}

pub struct ProtocolTransaction {
    chunks: VecDeque<Vec<u8>>,
    stream: Option<Box<dyn Iterator<Item = Vec<u8>>>>,
    remaining: usize,
    offset: usize,
    started: bool,
    projection: Option<u64>,
    control: bool,
    teardown: bool,
    da1: Option<Da1Owner>,
}

impl ProtocolTransaction {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self::chunks(vec![bytes])
    }

    pub fn chunks(chunks: Vec<Vec<u8>>) -> Self {
        Self {
            remaining: chunks.iter().map(Vec::len).sum(),
            chunks: chunks.into(),
            stream: None,
            offset: 0,
            started: false,
            projection: None,
            control: false,
            teardown: false,
            da1: None,
        }
    }

    /// The encoder owns its stable snapshot. `encoded_len` is the exact wire
    /// size; chunks are generated only when earlier output has drained.
    pub fn stream(encoded_len: usize, chunks: impl Iterator<Item = Vec<u8>> + 'static) -> Self {
        let mut transaction = Self::chunks(Vec::new());
        transaction.remaining = encoded_len;
        transaction.stream = Some(Box::new(chunks));
        transaction
    }

    pub fn projection(mut self, token: u64) -> Self {
        self.projection = Some(token);
        self
    }

    pub fn control(mut self) -> Self {
        self.control = true;
        self
    }

    /// A close the terminal must see even when the tty stops before it drains,
    /// for example a TSP `x` that hands the main screen back.
    pub fn teardown(mut self) -> Self {
        self.control = true;
        self.teardown = true;
        self
    }

    pub fn encoded_len(&self) -> usize {
        self.remaining
    }

    fn bytes(&mut self) -> io::Result<&[u8]> {
        while self
            .chunks
            .front()
            .is_some_and(|chunk| chunk.len() == self.offset)
        {
            self.chunks.pop_front();
            self.offset = 0;
        }
        if self.chunks.is_empty() {
            if let Some(stream) = &mut self.stream {
                loop {
                    match stream.next() {
                        Some(chunk) if chunk.is_empty() => continue,
                        Some(chunk) => {
                            if chunk.len() > self.remaining || chunk.len() > 262_144 + 64 {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "protocol encoder exceeds declared bound",
                                ));
                            }
                            self.chunks.push_back(chunk);
                            break;
                        }
                        None => {
                            self.stream = None;
                            break;
                        }
                    }
                }
            }
        }
        if self.chunks.is_empty() && self.remaining != 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete protocol encoder",
            ));
        }
        Ok(self
            .chunks
            .front()
            .map_or(&[], |chunk| &chunk[self.offset..]))
    }
}

pub(super) struct QueuedProtocol {
    cells_before: usize,
    transaction: ProtocolTransaction,
}

#[derive(Default)]
pub(super) struct ProtocolQueue {
    transactions: VecDeque<QueuedProtocol>,
    pub(super) bytes: usize,
    closing: bool,
}
impl ProtocolQueue {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    pub(super) fn discard_cells(&mut self) {
        for entry in &mut self.transactions {
            entry.cells_before = 0;
        }
    }

    fn cancel(&mut self, token: Option<u64>) -> Vec<Da1Owner> {
        let mut cancelled = Vec::new();
        self.transactions.retain(|entry| {
            let transaction = &entry.transaction;
            let remove = !transaction.started
                && token.map_or(transaction.projection.is_some(), |token| {
                    transaction.projection == Some(token)
                });
            if remove {
                self.bytes -= transaction.remaining;
                if let Some(owner) = transaction.da1 {
                    cancelled.push(owner);
                }
            }
            !remove
        });
        cancelled
    }
}

impl Tty {
    pub fn queue_protocol(&mut self, transaction: ProtocolTransaction) -> Result<(), QueueFull> {
        let limit = MAX_PROTOCOL_BYTES
            + if transaction.control {
                PROTOCOL_CONTROL_RESERVE
            } else {
                0
            };
        if self.protocol_out.closing
            || self.protocol_out.transactions.len() >= if transaction.control { 1152 } else { 1024 }
            || transaction.remaining > limit.saturating_sub(self.protocol_out.bytes)
        {
            return Err(QueueFull);
        }
        if transaction.remaining == 0 {
            return Ok(());
        }
        self.protocol_out.bytes += transaction.remaining;
        self.protocol_out.transactions.push_back(QueuedProtocol {
            cells_before: self.out.len(),
            transaction,
        });
        self.write_pending = true;
        Ok(())
    }

    /// Queue hello/replay and its DA1 as one indivisible output transaction.
    pub fn queue_da1(
        &mut self,
        owner: Da1Owner,
        mut transaction: ProtocolTransaction,
    ) -> Result<(), QueueFull> {
        if transaction.stream.is_some() {
            return Err(QueueFull);
        }
        transaction.chunks.push_back(b"\x1b[c".to_vec());
        transaction.remaining += 3;
        transaction.da1 = Some(owner);
        self.queue_protocol(transaction)?;
        self.keys.own_da1(owner);
        Ok(())
    }

    pub fn resolve_da1(&mut self) -> Option<Da1Owner> {
        self.keys.resolve_da1()
    }

    pub fn cancel_protocol(&mut self, projection: u64) {
        let cancelled = self.protocol_out.cancel(Some(projection));
        for owner in cancelled {
            self.keys.cancel_da1(owner);
        }
        self.write_pending = !self.out.is_empty() || !self.protocol_out.is_empty();
    }

    /// Discard complete unsent transactions and cells, retain the current
    /// partial transaction, then queue close bytes immediately behind it.
    pub fn close_protocol(&mut self, transaction: ProtocolTransaction) -> Result<(), QueueFull> {
        if self.protocol_out.closing || transaction.encoded_len() > PROTOCOL_CONTROL_RESERVE {
            return Err(QueueFull);
        }
        if transaction.remaining == 0 {
            return Ok(());
        }
        let cancelled = self.protocol_out.cancel(None);
        for owner in cancelled {
            self.keys.cancel_da1(owner);
        }
        self.out.clear();
        self.protocol_out.discard_cells();
        let mut transaction = transaction.control();
        let limit = MAX_PROTOCOL_BYTES + PROTOCOL_CONTROL_RESERVE;
        if transaction.remaining > limit.saturating_sub(self.protocol_out.bytes) {
            return Err(QueueFull);
        }
        transaction.control = true;
        self.protocol_out.bytes += transaction.remaining;
        let index = usize::from(
            self.protocol_out
                .transactions
                .front()
                .is_some_and(|entry| entry.transaction.started),
        );
        self.protocol_out.transactions.insert(
            index,
            QueuedProtocol {
                cells_before: 0,
                transaction,
            },
        );
        self.write_pending = true;
        Ok(())
    }

    pub fn queued_protocol_bytes(&self) -> usize {
        self.protocol_out.bytes
    }

    pub fn protocol_generation(&self) -> u64 {
        self.protocol_generation
    }

    /// `smcup` when `enter`, else `rmcup`, if the tty started on the alternate
    /// screen; empty otherwise.
    pub fn alternate_screen(&self, enter: bool) -> Vec<u8> {
        use crate::term::TtyCodeCode as C;
        match &self.term {
            Some(term) if self.opts.clear_on_attach => {
                rmux_util::bytes::cstr(term.string(if enter { C::Smcup } else { C::Rmcup }))
                    .to_vec()
            }
            _ => Vec::new(),
        }
    }

    /// Replaces the bytes `stop` sends ahead of its restore; empty for none.
    pub fn set_teardown(&mut self, bytes: Vec<u8>) {
        self.teardown = bytes;
    }

    /// Hard tty loss/reset: forget protocol bytes and all late query owners.
    pub fn reset_protocol(&mut self) {
        self.protocol_generation = self.protocol_generation.wrapping_add(1);
        self.protocol_out.clear();
        self.teardown.clear();
        self.keys.reset_protocol();
        self.timer(TtyTimer::Protocol, None);
        self.effects.push(TtyEffect::ProtocolInvalidated {
            generation: self.protocol_generation,
        });
    }

    pub fn input_len(&self) -> usize {
        self.in_buf.len()
    }

    /// Total allowance including already buffered bytes, not a per-read cap.
    pub fn set_read_limit(&mut self, limit: Option<usize>) {
        self.read_limit = limit;
    }

    pub fn set_read_paused(&mut self, paused: bool) {
        self.read_paused = paused;
    }

    pub fn protocol_partial(&self) -> bool {
        self.keys.protocol_partial()
    }

    pub(super) fn stop_after_protocol(
        &mut self,
        state: &mut crate::term::tparm::TparmState,
        opts: &super::TtyOptions,
    ) {
        use crate::term::{TtyCodeCode as C, TtyTermFlags as F};
        use rmux_util::bytes::cstr;
        let mut restore = Vec::new();
        self.term().string_ii(
            state,
            C::Csr,
            0,
            self.sy.saturating_sub(1) as i32,
            &mut restore,
        );
        if crate::acs::acs_needed(self.term(), self.host.utf8) {
            restore.extend_from_slice(cstr(self.term().string(C::Rmacs)));
        }
        for code in [C::Sgr0, C::Rmkx] {
            restore.extend_from_slice(cstr(self.term().string(code)));
        }
        if opts.clear_on_attach {
            restore.extend_from_slice(cstr(self.term().string(C::Clear)));
        }
        if self.cstyle != rmux_emu::screen::ScreenCursorStyle::Default {
            if self.term().has(C::Se) {
                restore.extend_from_slice(cstr(self.term().string(C::Se)));
            } else if self.term().has(C::Ss) {
                let mut cursor = Vec::new();
                self.term().string_i(state, C::Ss, 0, &mut cursor);
                restore.extend_from_slice(cstr(&cursor));
            }
        }
        if self.ccolour != -1 {
            restore.extend_from_slice(cstr(self.term().string(C::Cr)));
        }
        restore.extend_from_slice(cstr(self.term().string(C::Cnorm)));
        if self.term().has(C::Kmous) {
            restore.extend_from_slice(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1005l");
        }
        for code in [C::Dsbp, C::Dsesc, C::Dsfcs, C::Dseks] {
            restore.extend_from_slice(cstr(self.term().string(code)));
        }
        if self.term().flags().contains(F::DECSLRM) {
            restore.extend_from_slice(cstr(self.term().string(C::Dsmg)));
        }
        restore.extend_from_slice(cstr(self.term().string(if opts.clear_on_attach {
            C::Rmcup
        } else {
            C::Clear
        })));
        if self.term().flags().contains(F::VT100LIKE) {
            restore.extend_from_slice(b"\x1b[?2031l");
        }
        let keep = |entry: &QueuedProtocol| entry.transaction.started || entry.transaction.teardown;
        for entry in self
            .protocol_out
            .transactions
            .iter()
            .filter(|entry| !keep(entry))
        {
            if let Some(owner) = entry.transaction.da1 {
                self.keys.cancel_da1(owner);
            }
        }
        self.protocol_out.transactions.retain(keep);
        self.out.clear();
        self.protocol_out.discard_cells();
        let restore = ProtocolTransaction::new(restore).control();
        self.protocol_out.bytes = self
            .protocol_out
            .transactions
            .iter()
            .map(|entry| entry.transaction.remaining)
            .sum::<usize>()
            + restore.remaining;
        self.protocol_out.transactions.push_back(QueuedProtocol {
            cells_before: 0,
            transaction: restore,
        });
        self.write_pending = true;
        self.protocol_out.closing = true;
        self.protocol_generation = self.protocol_generation.wrapping_add(1);
        self.effects.push(TtyEffect::ProtocolInvalidated {
            generation: self.protocol_generation,
        });
        self.protocol_stop = true;
        self.read_pending = false;
        self.flags.remove(TtyFlags::STARTED | TtyFlags::BLOCK);
        for timer in [
            TtyTimer::Start,
            TtyTimer::Clipboard,
            TtyTimer::Block,
            TtyTimer::Key,
            TtyTimer::Protocol,
        ] {
            self.timer(timer, None);
        }
    }

    pub(super) fn finish_protocol_stop(&mut self) {
        if self.protocol_stop && self.out.is_empty() && self.protocol_out.is_empty() {
            self.protocol_stop = false;
            let _ = self.tio.set(self.fd());
            rmux_sys::fd::set_blocking(self.fd(), true);
            self.reset_protocol();
            self.flags.remove(TtyFlags::STARTED);
            if self.protocol_close {
                self.protocol_close = false;
                self.term = None;
                self.in_buf = rmux_util::buffer::ByteBuffer::new();
                self.keys.clear();
                self.flags.remove(TtyFlags::OPENED);
            }
        }
    }
}

pub(super) fn write_queued(
    queue: &mut ProtocolQueue,
    cells: &mut VecDeque<u8>,
    mut write: impl FnMut(&[u8]) -> io::Result<usize>,
) -> io::Result<(usize, bool)> {
    if queue
        .transactions
        .front()
        .is_some_and(|entry| entry.cells_before == 0)
    {
        let entry = queue.transactions.front_mut().expect("front transaction");
        let bytes = entry.transaction.bytes()?;
        let n = write(bytes)?;
        if n <= bytes.len() {
            super::log_output(&bytes[..n]);
        }
        if n > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "writer exceeds offered bytes",
            ));
        }
        entry.transaction.started |= n != 0;
        entry.transaction.offset += n;
        entry.transaction.remaining -= n;
        queue.bytes -= n;
        if entry.transaction.remaining == 0 {
            queue.transactions.pop_front();
        }
        return Ok((n, true));
    }
    let mut bytes = cells.as_slices().0;
    if let Some(entry) = queue.transactions.front() {
        bytes = &bytes[..bytes.len().min(entry.cells_before)];
    }
    let n = write(bytes)?;
    if n > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "writer exceeds offered bytes",
        ));
    }
    cells.drain(..n);
    for entry in &mut queue.transactions {
        entry.cells_before = entry.cells_before.saturating_sub(n);
    }
    Ok((n, false))
}
