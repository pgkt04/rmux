// Ported from tmux tty-keys.c @ 8f25579c
// TSP extension: whole APC input and owned DA1 sentinels.
use super::{DecodeStep, Recognition, TtyInput};
use crate::tty::{TimerRequest, TtyTimer};
use std::time::Duration;

pub const MAX_TSP_INPUT_BODY: usize = 24 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolFault {
    Malformed,
    Overflow,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Da1Owner {
    Discovery,
    Token(u64),
    Stop,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolInput {
    scanned: usize,
    observed: usize,
    waiting: bool,
    expired: bool,
    discarding: bool,
    escape: bool,
    malformed: bool,
}

impl ProtocolInput {
    pub(super) fn active(&self) -> bool {
        self.waiting
    }

    pub(super) fn expire(&mut self) {
        self.expired = self.waiting;
    }

    fn partial<'a>(&mut self, len: usize) -> DecodeStep<'a> {
        let arm = !self.waiting || self.observed != len;
        self.observed = len;
        self.waiting = true;
        DecodeStep::Partial {
            timer: arm.then_some(TimerRequest {
                timer: TtyTimer::Protocol,
                after: Some(Duration::from_secs(1)),
            }),
            theme_changed: false,
        }
    }

    fn fault<'a>(&mut self, consumed: usize, fault: ProtocolFault) -> DecodeStep<'a> {
        self.scanned = 0;
        self.observed = 0;
        self.waiting = false;
        self.expired = false;
        DecodeStep::Complete {
            consumed,
            input: TtyInput::ProtocolFault(fault),
            cancel_timer: true,
            theme_changed: false,
        }
    }

    /// `tsp_input`: the terminal speaks TSP or a TSP probe is pending. Any
    /// other terminal sends `ESC _` only as the `M-_` key.
    pub(super) fn next<'a>(&mut self, buf: &'a [u8], tsp_input: bool) -> Option<DecodeStep<'a>> {
        const PREFIX: &[u8] = b"\x1b_tsp;";
        if self.discarding {
            for (i, &byte) in buf.iter().enumerate() {
                if self.escape && byte == b'\\' {
                    *self = Self::default();
                    return Some(DecodeStep::Discard {
                        consumed: i + 1,
                        cancel_timer: true,
                    });
                }
                self.escape = byte == 0x1b;
            }
            return Some(DecodeStep::Discard {
                consumed: buf.len(),
                cancel_timer: false,
            });
        }
        let n = buf.len().min(PREFIX.len());
        if buf[..n] != PREFIX[..n] {
            *self = Self::default();
            return None;
        }
        // A single ESC remains subject to normal key disambiguation.
        if buf.len() == 1 {
            return None;
        }
        if !tsp_input && !self.waiting {
            return None;
        }
        if buf.len() < PREFIX.len() {
            // Not a TSP message yet: after the idle timeout these are keys.
            if self.expired {
                *self = Self::default();
                return None;
            }
            return Some(self.partial(buf.len()));
        }
        if self.expired {
            self.discarding = true;
            self.escape = buf.last() == Some(&0x1b);
            return Some(self.fault(buf.len(), ProtocolFault::Timeout));
        }
        let start = self.scanned.max(PREFIX.len());
        for (i, &byte) in buf.iter().enumerate().skip(start) {
            if self.escape && byte == b'\\' {
                let end = i - 1;
                let valid = end >= PREFIX.len() + 2 && buf[PREFIX.len() + 1] == b';';
                if self.malformed || !valid {
                    *self = Self::default();
                    return Some(self.fault(i + 1, ProtocolFault::Malformed));
                }
                let body_start = PREFIX.len() + 2;
                if end - body_start > MAX_TSP_INPUT_BODY {
                    *self = Self::default();
                    return Some(self.fault(i + 1, ProtocolFault::Overflow));
                }
                *self = Self::default();
                return Some(DecodeStep::Complete {
                    consumed: i + 1,
                    input: TtyInput::Tsp {
                        verb: buf[PREFIX.len()],
                        body: &buf[body_start..end],
                    },
                    cancel_timer: true,
                    theme_changed: false,
                });
            }
            if self.escape || matches!(byte, 0x18 | 0x1a) {
                self.malformed = true;
            }
            self.escape = byte == 0x1b;
            if i.saturating_sub(PREFIX.len() + 2) >= MAX_TSP_INPUT_BODY && !self.escape {
                self.discarding = true;
                return Some(self.fault(i + 1, ProtocolFault::Overflow));
            }
        }
        self.scanned = buf.len();
        Some(self.partial(buf.len()))
    }
}

pub(super) fn da1(buf: &[u8]) -> Recognition<usize> {
    const PREFIX: &[u8] = b"\x1b[?";
    let n = buf.len().min(PREFIX.len());
    if buf[..n] != PREFIX[..n] {
        return Recognition::NoMatch;
    }
    if buf.len() <= PREFIX.len() {
        return Recognition::Partial;
    }
    for (i, &byte) in buf.iter().enumerate().skip(PREFIX.len()).take(128) {
        if byte == b'c' {
            return Recognition::Complete(i + 1, i + 1);
        }
        if !byte.is_ascii_digit() && byte != b';' {
            return Recognition::NoMatch;
        }
    }
    if buf.len() < PREFIX.len() + 128 {
        Recognition::Partial
    } else {
        Recognition::NoMatch
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct SentinelInput {
    waiting: bool,
    observed: usize,
    expired: bool,
    discarding: bool,
}

impl SentinelInput {
    pub(super) fn expire(&mut self) {
        self.expired = self.waiting;
    }

    fn partial<'a>(&mut self, len: usize) -> DecodeStep<'a> {
        let arm = !self.waiting || self.observed != len;
        self.waiting = true;
        self.observed = len;
        DecodeStep::Partial {
            timer: arm.then_some(TimerRequest {
                timer: TtyTimer::Protocol,
                after: Some(Duration::from_secs(1)),
            }),
            theme_changed: false,
        }
    }

    pub(super) fn next<'a>(&mut self, buf: &'a [u8]) -> Option<DecodeStep<'a>> {
        if self.discarding {
            if let Some(end) = buf.iter().position(|&byte| byte == b'c') {
                *self = Self::default();
                return Some(DecodeStep::Complete {
                    consumed: end + 1,
                    input: TtyInput::Da1Sentinel {
                        raw: &buf[..end + 1],
                    },
                    cancel_timer: true,
                    theme_changed: false,
                });
            }
            return Some(DecodeStep::Discard {
                consumed: buf.len(),
                cancel_timer: false,
            });
        }
        if buf.len() < 3 || !matches!(da1(buf), Recognition::Partial) {
            if buf.len() >= 3 + 128
                && buf.starts_with(b"\x1b[?")
                && buf[3..]
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || *byte == b';')
            {
                self.discarding = true;
                return Some(DecodeStep::Complete {
                    consumed: buf.len(),
                    input: TtyInput::ProtocolFault(ProtocolFault::Overflow),
                    cancel_timer: true,
                    theme_changed: false,
                });
            }
            *self = Self::default();
            return None;
        }
        if self.expired {
            self.waiting = false;
            self.expired = false;
            self.discarding = true;
            return Some(DecodeStep::Complete {
                consumed: buf.len(),
                input: TtyInput::ProtocolFault(ProtocolFault::Timeout),
                cancel_timer: true,
                theme_changed: false,
            });
        }
        Some(self.partial(buf.len()))
    }
}
