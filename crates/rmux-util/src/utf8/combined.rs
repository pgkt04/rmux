// Ported from tmux utf8-combined.c @ 8f25579c
//! Combining predicates used by screen-write: ZWJ, variation selector, Hangul
//! filler, regional indicator and skin tone pairs, and Hangul Jamo state.

use super::Utf8Data;
use super::tables::SKIN_TONE_BASES;

/// `enum hanguljamo_state` (`tmux.h:738-743`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HangulJamoState {
    NotHangulJamo,
    Choseong,
    Composable,
    NotComposable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Subclass {
    NotHangulJamo,
    Choseong,
    OldChoseong,
    ChoseongFiller,
    JungseongFiller,
    Jungseong,
    OldJungseong,
    Jongseong,
    OldJongseong,
    ExtendedOldChoseong,
    ExtendedOldJungseong,
    ExtendedOldJongseong,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    NotHangulJamo,
    Choseong,
    Jungseong,
    Jongseong,
}

const ZWJ: [u8; 3] = [0xe2, 0x80, 0x8d];
const VS16: [u8; 3] = [0xef, 0xb8, 0x8f];
const HANGUL_FILLER: [u8; 3] = [0xe3, 0x85, 0xa4];

/// `utf8_has_zwj`: ends with U+200D.
pub fn has_zwj(ud: &Utf8Data) -> bool {
    let bytes = ud.bytes();
    bytes.len() >= 3 && bytes[bytes.len() - 3..] == ZWJ
}

/// `utf8_is_zwj`: exactly U+200D.
pub fn is_zwj(ud: &Utf8Data) -> bool {
    ud.bytes() == ZWJ
}

/// `utf8_is_vs`: exactly U+FE0F.
pub fn is_vs(ud: &Utf8Data) -> bool {
    ud.bytes() == VS16
}

/// `utf8_is_hangul_filler`: exactly U+3164.
pub fn is_hangul_filler(ud: &Utf8Data) -> bool {
    ud.bytes() == HANGUL_FILLER
}

/// `utf8_regional_count`: 4-byte regional indicator groups at any offset.
fn regional_count(ud: &Utf8Data) -> usize {
    ud.bytes()
        .windows(4)
        .filter(|w| w[0] == 0xf0 && w[1] == 0x9f && w[2] == 0x87 && (0xa6..=0xbf).contains(&w[3]))
        .count()
}

/// `utf8_should_combine`.
pub fn should_combine(with: &Utf8Data, add: &Utf8Data) -> bool {
    let Some(w) = with.to_wc() else {
        return false;
    };
    let Some(a) = add.to_wc() else {
        return false;
    };

    // Regional indicators.
    if (0x1F1E6..=0x1F1FF).contains(&a) && (0x1F1E6..=0x1F1FF).contains(&w) {
        return regional_count(with) == 1 && regional_count(add) == 1;
    }

    // Emoji skin tone modifiers.
    SKIN_TONE_BASES.contains(&a) && (0x1F3FB..=0x1F3FF).contains(&w)
}

fn subclass(s: &[u8]) -> Subclass {
    match s[0] {
        0xE1 => match s[1] {
            0x84 => match s[2] {
                0x80..=0x92 => Subclass::Choseong,
                0x93..=0xBF => Subclass::OldChoseong,
                _ => Subclass::NotHangulJamo,
            },
            0x85 => match s[2] {
                0x9F => Subclass::ChoseongFiller,
                0xA0 => Subclass::JungseongFiller,
                0x80..=0x9E => Subclass::OldChoseong,
                0xA1..=0xB5 => Subclass::Jungseong,
                0xB6..=0xBF => Subclass::OldJungseong,
                _ => Subclass::NotHangulJamo,
            },
            0x86 => match s[2] {
                0x80..=0xA7 => Subclass::OldJungseong,
                0xA8..=0xBF => Subclass::Jongseong,
                _ => Subclass::NotHangulJamo,
            },
            0x87 => match s[2] {
                0x80..=0x82 => Subclass::Jongseong,
                0x83..=0xBF => Subclass::OldJongseong,
                _ => Subclass::NotHangulJamo,
            },
            _ => Subclass::NotHangulJamo,
        },
        0xEA if s[1] == 0xA5 && (0xA0..=0xBC).contains(&s[2]) => Subclass::ExtendedOldChoseong,
        0xED => match (s[1], s[2]) {
            (0x9E, 0xB0..=0xBF) | (0x9F, 0x80..=0x86) => Subclass::ExtendedOldJungseong,
            (0x9F, 0x8B..=0xBB) => Subclass::ExtendedOldJongseong,
            _ => Subclass::NotHangulJamo,
        },
        _ => Subclass::NotHangulJamo,
    }
}

fn class(s: &[u8]) -> Class {
    match subclass(s) {
        Subclass::Choseong
        | Subclass::ChoseongFiller
        | Subclass::OldChoseong
        | Subclass::ExtendedOldChoseong => Class::Choseong,
        Subclass::Jungseong
        | Subclass::JungseongFiller
        | Subclass::OldJungseong
        | Subclass::ExtendedOldJungseong => Class::Jungseong,
        Subclass::Jongseong | Subclass::OldJongseong | Subclass::ExtendedOldJongseong => {
            Class::Jongseong
        }
        Subclass::NotHangulJamo => Class::NotHangulJamo,
    }
}

/// `hanguljamo_check_state(prev, cur)`.
pub fn hanguljamo_check_state(prev: &Utf8Data, cur: &Utf8Data) -> HangulJamoState {
    if cur.size != 3 {
        return HangulJamoState::NotHangulJamo;
    }
    let last_three = |ud: &Utf8Data| {
        let bytes = ud.bytes();
        (bytes.len() >= 3).then(|| class(&bytes[bytes.len() - 3..]))
    };
    match class(cur.bytes()) {
        Class::Choseong => HangulJamoState::Choseong,
        Class::Jungseong => match last_three(prev) {
            Some(Class::Choseong) => HangulJamoState::Composable,
            _ => HangulJamoState::NotComposable,
        },
        Class::Jongseong => match last_three(prev) {
            Some(Class::Jungseong) => HangulJamoState::Composable,
            _ => HangulJamoState::NotComposable,
        },
        Class::NotHangulJamo => HangulJamoState::NotHangulJamo,
    }
}
