// Ported from tmux input.c @ 8f25579c
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

//! The 17 parser states and their transition tables (`input.c:349-763`),
//! indexed by byte at compile time.

/// `struct input_state` identity (`input.c:392-508`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum StateId {
    Ground,
    EscEnter,
    EscIntermediate,
    CsiEnter,
    CsiParameter,
    CsiIntermediate,
    CsiIgnore,
    DcsEnter,
    DcsParameter,
    DcsIntermediate,
    DcsHandler,
    DcsEscape,
    DcsIgnore,
    OscString,
    ApcString,
    RenameString,
    ConsumeSt,
}

pub(crate) const STATE_COUNT: usize = 17;

impl StateId {
    pub(crate) const ALL: [StateId; STATE_COUNT] = [
        StateId::Ground,
        StateId::EscEnter,
        StateId::EscIntermediate,
        StateId::CsiEnter,
        StateId::CsiParameter,
        StateId::CsiIntermediate,
        StateId::CsiIgnore,
        StateId::DcsEnter,
        StateId::DcsParameter,
        StateId::DcsIntermediate,
        StateId::DcsHandler,
        StateId::DcsEscape,
        StateId::DcsIgnore,
        StateId::OscString,
        StateId::ApcString,
        StateId::RenameString,
        StateId::ConsumeSt,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            StateId::Ground => "ground",
            StateId::EscEnter => "esc_enter",
            StateId::EscIntermediate => "esc_intermediate",
            StateId::CsiEnter => "csi_enter",
            StateId::CsiParameter => "csi_parameter",
            StateId::CsiIntermediate => "csi_intermediate",
            StateId::CsiIgnore => "csi_ignore",
            StateId::DcsEnter => "dcs_enter",
            StateId::DcsParameter => "dcs_parameter",
            StateId::DcsIntermediate => "dcs_intermediate",
            StateId::DcsHandler => "dcs_handler",
            StateId::DcsEscape => "dcs_escape",
            StateId::DcsIgnore => "dcs_ignore",
            StateId::OscString => "osc_string",
            StateId::ApcString => "apc_string",
            StateId::RenameString => "rename_string",
            StateId::ConsumeSt => "consume_st",
        }
    }

    /// The state enter handler (`input.c:392-508`).
    pub(crate) const fn enter(self) -> Option<Enter> {
        match self {
            StateId::Ground => Some(Enter::Ground),
            StateId::EscEnter | StateId::CsiEnter => Some(Enter::Clear),
            StateId::DcsEnter => Some(Enter::Dcs),
            StateId::OscString => Some(Enter::Osc),
            StateId::ApcString => Some(Enter::Apc),
            StateId::RenameString | StateId::ConsumeSt => Some(Enter::Rename),
            _ => None,
        }
    }

    /// The state exit handler.
    pub(crate) const fn exit(self) -> Option<Exit> {
        match self {
            StateId::OscString => Some(Exit::Osc),
            StateId::ApcString => Some(Exit::Apc),
            StateId::RenameString => Some(Exit::Rename),
            _ => None,
        }
    }
}

/// Enter handlers (`input_ground`, `input_clear`, `input_enter_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Enter {
    Ground,
    Clear,
    Dcs,
    Osc,
    Apc,
    Rename,
}

/// Exit handlers (`input_exit_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    Osc,
    Apc,
    Rename,
}

/// Transition handlers (`input.c:189-207`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handler {
    C0Dispatch,
    Print,
    Intermediate,
    Parameter,
    Input,
    EscDispatch,
    CsiDispatch,
    DcsDispatch,
    TopBitSet,
    EndBel,
}

/// `struct input_transition` (`input.c:350-356`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Transition {
    pub first: u8,
    pub last: u8,
    pub handler: Option<Handler>,
    pub next: Option<StateId>,
}

const fn t(first: u8, last: u8, handler: Option<Handler>, next: Option<StateId>) -> Transition {
    Transition {
        first,
        last,
        handler,
        next,
    }
}

use Handler as H;
use StateId as S;

/// `INPUT_STATE_ANYWHERE` (`input.c:367-370`).
const ANYWHERE: [Transition; 3] = [
    t(0x18, 0x18, Some(H::C0Dispatch), Some(S::Ground)),
    t(0x1a, 0x1a, Some(H::C0Dispatch), Some(S::Ground)),
    t(0x1b, 0x1b, None, Some(S::EscEnter)),
];

const fn c0_executes() -> [Transition; 3] {
    [
        t(0x00, 0x17, Some(H::C0Dispatch), None),
        t(0x19, 0x19, Some(H::C0Dispatch), None),
        t(0x1c, 0x1f, Some(H::C0Dispatch), None),
    ]
}

const fn c0_ignored() -> [Transition; 3] {
    [
        t(0x00, 0x17, None, None),
        t(0x19, 0x19, None, None),
        t(0x1c, 0x1f, None, None),
    ]
}

const fn cat<const A: usize, const B: usize, const N: usize>(
    a: [Transition; A],
    b: [Transition; B],
) -> [Transition; N] {
    assert!(A + B == N);
    let mut out = [t(0, 0, None, None); N];
    let mut i = 0;
    while i < A {
        out[i] = a[i];
        i += 1;
    }
    let mut j = 0;
    while j < B {
        out[A + j] = b[j];
        j += 1;
    }
    out
}

const GROUND: [Transition; 9] = cat(
    ANYWHERE,
    cat::<3, 3, 6>(
        c0_executes(),
        [
            t(0x20, 0x7e, Some(H::Print), None),
            t(0x7f, 0x7f, None, None),
            t(0x80, 0xff, Some(H::TopBitSet), None),
        ],
    ),
);

const ESC_ENTER: [Transition; 22] = cat(
    ANYWHERE,
    cat::<3, 16, 19>(
        c0_executes(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), Some(S::EscIntermediate)),
            t(0x30, 0x4f, Some(H::EscDispatch), Some(S::Ground)),
            t(0x50, 0x50, None, Some(S::DcsEnter)),
            t(0x51, 0x57, Some(H::EscDispatch), Some(S::Ground)),
            t(0x58, 0x58, None, Some(S::ConsumeSt)),
            t(0x59, 0x59, Some(H::EscDispatch), Some(S::Ground)),
            t(0x5a, 0x5a, Some(H::EscDispatch), Some(S::Ground)),
            t(0x5b, 0x5b, None, Some(S::CsiEnter)),
            t(0x5c, 0x5c, Some(H::EscDispatch), Some(S::Ground)),
            t(0x5d, 0x5d, None, Some(S::OscString)),
            t(0x5e, 0x5e, None, Some(S::ConsumeSt)),
            t(0x5f, 0x5f, None, Some(S::ApcString)),
            t(0x60, 0x6a, Some(H::EscDispatch), Some(S::Ground)),
            t(0x6b, 0x6b, None, Some(S::RenameString)),
            t(0x6c, 0x7e, Some(H::EscDispatch), Some(S::Ground)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const ESC_INTERMEDIATE: [Transition; 9] = cat(
    ANYWHERE,
    cat::<3, 3, 6>(
        c0_executes(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), None),
            t(0x30, 0x7e, Some(H::EscDispatch), Some(S::Ground)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const CSI_ENTER: [Transition; 13] = cat(
    ANYWHERE,
    cat::<3, 7, 10>(
        c0_executes(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), Some(S::CsiIntermediate)),
            t(0x30, 0x39, Some(H::Parameter), Some(S::CsiParameter)),
            t(0x3a, 0x3a, Some(H::Parameter), Some(S::CsiParameter)),
            t(0x3b, 0x3b, Some(H::Parameter), Some(S::CsiParameter)),
            t(0x3c, 0x3f, Some(H::Intermediate), Some(S::CsiParameter)),
            t(0x40, 0x7e, Some(H::CsiDispatch), Some(S::Ground)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const CSI_PARAMETER: [Transition; 13] = cat(
    ANYWHERE,
    cat::<3, 7, 10>(
        c0_executes(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), Some(S::CsiIntermediate)),
            t(0x30, 0x39, Some(H::Parameter), None),
            t(0x3a, 0x3a, Some(H::Parameter), None),
            t(0x3b, 0x3b, Some(H::Parameter), None),
            t(0x3c, 0x3f, None, Some(S::CsiIgnore)),
            t(0x40, 0x7e, Some(H::CsiDispatch), Some(S::Ground)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const CSI_INTERMEDIATE: [Transition; 10] = cat(
    ANYWHERE,
    cat::<3, 4, 7>(
        c0_executes(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), None),
            t(0x30, 0x3f, None, Some(S::CsiIgnore)),
            t(0x40, 0x7e, Some(H::CsiDispatch), Some(S::Ground)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const CSI_IGNORE: [Transition; 9] = cat(
    ANYWHERE,
    cat::<3, 3, 6>(
        c0_executes(),
        [
            t(0x20, 0x3f, None, None),
            t(0x40, 0x7e, None, Some(S::Ground)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const DCS_ENTER: [Transition; 13] = cat(
    ANYWHERE,
    cat::<3, 7, 10>(
        c0_ignored(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), Some(S::DcsIntermediate)),
            t(0x30, 0x39, Some(H::Parameter), Some(S::DcsParameter)),
            t(0x3a, 0x3a, None, Some(S::DcsIgnore)),
            t(0x3b, 0x3b, Some(H::Parameter), Some(S::DcsParameter)),
            t(0x3c, 0x3f, Some(H::Intermediate), Some(S::DcsParameter)),
            t(0x40, 0x7e, Some(H::Input), Some(S::DcsHandler)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const DCS_PARAMETER: [Transition; 13] = cat(
    ANYWHERE,
    cat::<3, 7, 10>(
        c0_ignored(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), Some(S::DcsIntermediate)),
            t(0x30, 0x39, Some(H::Parameter), None),
            t(0x3a, 0x3a, None, Some(S::DcsIgnore)),
            t(0x3b, 0x3b, Some(H::Parameter), None),
            t(0x3c, 0x3f, None, Some(S::DcsIgnore)),
            t(0x40, 0x7e, Some(H::Input), Some(S::DcsHandler)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const DCS_INTERMEDIATE: [Transition; 10] = cat(
    ANYWHERE,
    cat::<3, 4, 7>(
        c0_ignored(),
        [
            t(0x20, 0x2f, Some(H::Intermediate), None),
            t(0x30, 0x3f, None, Some(S::DcsIgnore)),
            t(0x40, 0x7e, Some(H::Input), Some(S::DcsHandler)),
            t(0x7f, 0xff, None, None),
        ],
    ),
);

const DCS_HANDLER: [Transition; 3] = [
    t(0x00, 0x1a, Some(H::Input), None),
    t(0x1b, 0x1b, None, Some(S::DcsEscape)),
    t(0x1c, 0xff, Some(H::Input), None),
];

const DCS_ESCAPE: [Transition; 3] = [
    t(0x00, 0x5b, Some(H::Input), Some(S::DcsHandler)),
    t(0x5c, 0x5c, Some(H::DcsDispatch), Some(S::Ground)),
    t(0x5d, 0xff, Some(H::Input), Some(S::DcsHandler)),
];

const DCS_IGNORE: [Transition; 7] = cat(
    ANYWHERE,
    cat::<3, 1, 4>(c0_ignored(), [t(0x20, 0xff, None, None)]),
);

const OSC_STRING: [Transition; 9] = cat(
    ANYWHERE,
    [
        t(0x00, 0x06, None, None),
        t(0x07, 0x07, Some(H::EndBel), Some(S::Ground)),
        t(0x08, 0x17, None, None),
        t(0x19, 0x19, None, None),
        t(0x1c, 0x1f, None, None),
        t(0x20, 0xff, Some(H::Input), None),
    ],
);

const STRING_BODY: [Transition; 7] = cat(
    ANYWHERE,
    cat::<3, 1, 4>(c0_ignored(), [t(0x20, 0xff, Some(H::Input), None)]),
);

const CONSUME_ST: [Transition; 7] = cat(
    ANYWHERE,
    cat::<3, 1, 4>(c0_ignored(), [t(0x20, 0xff, None, None)]),
);

/// The transition table of a state (`input.c:511-763`), in C order.
pub(crate) const fn table(state: StateId) -> &'static [Transition] {
    match state {
        S::Ground => &GROUND,
        S::EscEnter => &ESC_ENTER,
        S::EscIntermediate => &ESC_INTERMEDIATE,
        S::CsiEnter => &CSI_ENTER,
        S::CsiParameter => &CSI_PARAMETER,
        S::CsiIntermediate => &CSI_INTERMEDIATE,
        S::CsiIgnore => &CSI_IGNORE,
        S::DcsEnter => &DCS_ENTER,
        S::DcsParameter => &DCS_PARAMETER,
        S::DcsIntermediate => &DCS_INTERMEDIATE,
        S::DcsHandler => &DCS_HANDLER,
        S::DcsEscape => &DCS_ESCAPE,
        S::DcsIgnore => &DCS_IGNORE,
        S::OscString => &OSC_STRING,
        S::ApcString | S::RenameString => &STRING_BODY,
        S::ConsumeSt => &CONSUME_ST,
    }
}

/// Linear search as `input_parse` does (`input.c:985-995`).
pub(crate) const fn lookup(state: StateId, ch: u8) -> Option<Transition> {
    let table = table(state);
    let mut i = 0;
    while i < table.len() {
        if ch >= table[i].first && ch <= table[i].last {
            return Some(table[i]);
        }
        i += 1;
    }
    None
}

const fn build_index(state: StateId) -> [Transition; 256] {
    let mut out = [t(0, 0, None, None); 256];
    let mut ch = 0;
    while ch < 256 {
        match lookup(state, ch as u8) {
            Some(tr) => out[ch] = tr,
            // Replaces the fatal "no transition from state" (input.c:992-995).
            None => panic!("state has a byte without a transition"),
        }
        ch += 1;
    }
    out
}

const fn build_all() -> [[Transition; 256]; STATE_COUNT] {
    let mut out = [[t(0, 0, None, None); 256]; STATE_COUNT];
    let mut i = 0;
    while i < STATE_COUNT {
        out[i] = build_index(StateId::ALL[i]);
        i += 1;
    }
    out
}

/// Every state's 256-entry dispatch index, checked at compile time.
pub(crate) static INDEX: [[Transition; 256]; STATE_COUNT] = build_all();

#[inline]
pub(crate) fn transition(state: StateId, ch: u8) -> Transition {
    INDEX[state as usize][ch as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_linear_search() {
        for state in StateId::ALL {
            for ch in 0..=255u8 {
                assert_eq!(Some(transition(state, ch)), lookup(state, ch));
            }
        }
    }

    #[test]
    fn names_and_handlers() {
        assert_eq!(StateId::Ground.name(), "ground");
        assert_eq!(StateId::ConsumeSt.enter(), Some(Enter::Rename));
        assert_eq!(StateId::OscString.exit(), Some(Exit::Osc));
        assert_eq!(transition(StateId::Ground, b'A').handler, Some(H::Print));
        assert_eq!(
            transition(StateId::DcsHandler, 0x1b).next,
            Some(S::DcsEscape)
        );
        assert_eq!(
            transition(StateId::DcsEscape, b'\\').handler,
            Some(H::DcsDispatch)
        );
    }
}
