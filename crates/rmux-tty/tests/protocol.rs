// Ported from tmux tty-keys.c @ 8f25579c
use rmux_tty::keys::{
    DecodeStep, KeyDecodeContext, MAX_TSP_INPUT_BODY, ProtocolFault, TtyInput, TtyKeyDecoder,
};
use rmux_tty::tty::{TtyFlags, TtyTimer};
use std::time::Duration;

fn ctx() -> KeyDecodeContext {
    KeyDecodeContext {
        flags: TtyFlags::ALL_REQUEST_FLAGS,
        has_session: false,
        escape_time_ms: 1,
        ..Default::default()
    }
}

#[test]
fn every_apc_split_before_session_with_unicode() {
    let body = "{\"ev\":\"edit\",\"text\":\"🙂;a=b;\u{009c}\"}".as_bytes();
    let mut message = b"\x1b_tsp;e;".to_vec();
    message.extend_from_slice(body);
    message.extend_from_slice(b"\x1b\\");
    for split in 1..message.len() {
        let mut decoder = TtyKeyDecoder::new();
        assert!(
            matches!(
                decoder.next(&message[..split], &ctx()),
                DecodeStep::Partial { .. }
            ),
            "split {split}"
        );
        assert!(
            matches!(decoder.next(&message, &ctx()), DecodeStep::Complete { consumed, input: TtyInput::Tsp { verb: b'e', body: parsed }, .. } if consumed == message.len() && parsed == body),
            "split {split}"
        );
    }
}

#[test]
fn idle_timer_is_dedicated_and_tail_is_consumed() {
    let mut decoder = TtyKeyDecoder::new();
    assert!(
        matches!(decoder.next(b"\x1b_tsp;e;{", &ctx()), DecodeStep::Partial { timer: Some(timer), .. } if timer.timer == TtyTimer::Protocol && timer.after == Some(Duration::from_secs(1)))
    );
    decoder.timer_fired();
    assert!(matches!(
        decoder.next(b"\x1b_tsp;e;{", &ctx()),
        DecodeStep::Partial { timer: None, .. }
    ));
    decoder.protocol_timer_fired();
    assert!(matches!(
        decoder.next(b"\x1b_tsp;e;{", &ctx()),
        DecodeStep::Complete {
            consumed: 9,
            input: TtyInput::ProtocolFault(ProtocolFault::Timeout),
            ..
        }
    ));
    assert!(matches!(
        decoder.next(b"JSON\x1b\\key", &ctx()),
        DecodeStep::Discard { consumed: 6, .. }
    ));
}

#[test]
fn malformed_controls_and_unknown_verb_never_become_keys() {
    for bytes in [
        b"\x1b_tsp;e;bad\x18tail\x1b\\".as_slice(),
        b"\x1b_tsp;e;bad\x1b[Atail\x1b\\",
        b"\x1b_tsp;bad\x1b\\",
    ] {
        let mut decoder = TtyKeyDecoder::new();
        assert!(
            matches!(decoder.next(bytes, &ctx()), DecodeStep::Complete { consumed, input: TtyInput::ProtocolFault(ProtocolFault::Malformed), .. } if consumed == bytes.len())
        );
    }
    let mut decoder = TtyKeyDecoder::new();
    assert!(matches!(
        decoder.next(b"\x1b_tsp;z;unknown\x1b\\", &ctx()),
        DecodeStep::Complete {
            input: TtyInput::Tsp {
                verb: b'z',
                body: b"unknown"
            },
            ..
        }
    ));
}

#[test]
fn exact_bound_accepts_and_overflow_drains_to_st() {
    let mut bytes = b"\x1b_tsp;e;".to_vec();
    bytes.resize(bytes.len() + MAX_TSP_INPUT_BODY, b'x');
    bytes.extend_from_slice(b"\x1b\\");
    let mut decoder = TtyKeyDecoder::new();
    assert!(
        matches!(decoder.next(&bytes, &ctx()), DecodeStep::Complete { input: TtyInput::Tsp { body, .. }, .. } if body.len() == MAX_TSP_INPUT_BODY)
    );
    bytes.insert(bytes.len() - 2, b'x');
    let mut decoder = TtyKeyDecoder::new();
    let consumed = match decoder.next(&bytes, &ctx()) {
        DecodeStep::Complete {
            consumed,
            input: TtyInput::ProtocolFault(ProtocolFault::Overflow),
            ..
        } => consumed,
        step => panic!("expected overflow: {step:?}"),
    };
    assert!(matches!(
        decoder.next(&bytes[consumed..], &ctx()),
        DecodeStep::Discard { consumed: 2, .. }
    ));
}

#[test]
fn late_da1_with_haveda_preserves_flag_and_no_keys() {
    let context = KeyDecodeContext {
        flags: TtyFlags::ALL_REQUEST_FLAGS,
        has_session: true,
        escape_time_ms: 1,
        ..Default::default()
    };
    for split in 1..7 {
        let mut decoder = TtyKeyDecoder::new();
        let step = decoder.next(&b"\x1b[?1;2c"[..split], &context);
        if split < 3 {
            assert!(
                matches!(step, DecodeStep::Partial { timer: Some(timer), .. } if timer.timer == TtyTimer::Key),
                "{step:?}"
            );
        } else {
            assert!(
                matches!(step, DecodeStep::Partial { timer: Some(timer), .. } if timer.timer == TtyTimer::Protocol),
                "{step:?}"
            );
        }
    }
    let mut decoder = TtyKeyDecoder::new();
    assert!(matches!(
        decoder.next(b"\x1b[?1;2ckey", &context),
        DecodeStep::Complete {
            consumed: 7,
            input: TtyInput::Da1Sentinel { raw: b"\x1b[?1;2c" },
            ..
        }
    ));
    assert!(context.flags.contains(TtyFlags::HAVEDA));
}

#[test]
fn owned_da1_uses_protocol_idle_not_escape_timeout() {
    let mut decoder = TtyKeyDecoder::new();
    let context = KeyDecodeContext {
        flags: TtyFlags::HAVEDA,
        has_session: true,
        escape_time_ms: 1,
        ..Default::default()
    };
    assert!(
        matches!(decoder.next(b"\x1b[?1;", &context), DecodeStep::Partial { timer: Some(timer), .. } if timer.timer == TtyTimer::Protocol && timer.after == Some(Duration::from_secs(1)))
    );
    decoder.timer_fired();
    assert!(matches!(
        decoder.next(b"\x1b[?1;", &context),
        DecodeStep::Partial { timer: None, .. }
    ));
    decoder.protocol_timer_fired();
    assert!(matches!(
        decoder.next(b"\x1b[?1;", &context),
        DecodeStep::Complete {
            consumed: 5,
            input: TtyInput::ProtocolFault(ProtocolFault::Timeout),
            ..
        }
    ));
    assert!(matches!(
        decoder.next(b"2cKEY", &context),
        DecodeStep::Complete {
            consumed: 2,
            input: TtyInput::Da1Sentinel { raw: b"2c" },
            ..
        }
    ));
    assert!(matches!(
        decoder.next(b"KEY", &context),
        DecodeStep::Complete {
            input: TtyInput::Key(_),
            consumed: 1,
            ..
        }
    ));
}

#[test]
fn sync_reply_is_not_an_owned_da1_partial() {
    let mut decoder = TtyKeyDecoder::new();
    let context = KeyDecodeContext {
        flags: TtyFlags::HAVEDA,
        has_session: true,
        ..Default::default()
    };
    assert!(matches!(
        decoder.next(b"\x1b[?2026;1$y", &context),
        DecodeStep::Complete {
            input: TtyInput::Discovery(_),
            ..
        }
    ));
}

#[test]
fn ordinary_keys_before_protocol_without_session_are_discarded_separately() {
    let mut decoder = TtyKeyDecoder::new();
    let bytes = b"keys\x1b_tsp;r;{}\x1b\\";
    assert!(matches!(
        decoder.next(bytes, &ctx()),
        DecodeStep::Discard { consumed: 4, .. }
    ));
    assert!(matches!(
        decoder.next(&bytes[4..], &ctx()),
        DecodeStep::Complete {
            input: TtyInput::Tsp {
                verb: b'r',
                body: b"{}"
            },
            ..
        }
    ));
}
