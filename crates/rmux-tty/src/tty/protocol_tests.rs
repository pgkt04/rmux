// Ported from tmux tty.c and tty-keys.c @ 8f25579c
use super::*;
use crate::keys::{Da1Owner, DecodeStep, KeyDecodeContext, TtyInput};
use crate::tty::protocol::write_queued;

fn drain(tty: &mut Tty, limit: usize) -> Vec<u8> {
    let mut result = Vec::new();
    while !tty.out.is_empty() || !tty.protocol_out.is_empty() {
        write_queued(&mut tty.protocol_out, &mut tty.out, |bytes| {
            let n = bytes.len().min(limit);
            result.extend_from_slice(&bytes[..n]);
            Ok(n)
        })
        .unwrap();
    }
    result
}

#[test]
fn ordered_chunks_short_writes_eagain_and_cells() {
    let (mut tty, _master, _) = fixture();
    tty.add(b"before");
    tty.queue_protocol(ProtocolTransaction::chunks(vec![
        b"\x1b_tsp;o;{}\x1b\\".to_vec(),
        b"\x1b_tsp;f;{}\x1b\\".to_vec(),
    ]))
    .unwrap();
    tty.add(b"after");
    let before = tty.out_len();
    let error = write_queued(&mut tty.protocol_out, &mut tty.out, |_| {
        Err(io::ErrorKind::WouldBlock.into())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(tty.out_len(), before);
    assert_eq!(
        drain(&mut tty, 1),
        b"before\x1b_tsp;o;{}\x1b\\\x1b_tsp;f;{}\x1b\\after"
    );
}

#[test]
fn block_and_close_preserve_started_apc() {
    let (mut tty, _master, _) = fixture();
    tty.set_size(1, 1, 0, 0);
    tty.queue_protocol(ProtocolTransaction::new(b"\x1b_tsp;f;{}\x1b\\".to_vec()).projection(7))
        .unwrap();
    let mut emitted = Vec::new();
    write_queued(&mut tty.protocol_out, &mut tty.out, |bytes| {
        emitted.extend_from_slice(&bytes[..4]);
        Ok(4)
    })
    .unwrap();
    tty.add(b"discard these cells");
    assert!(tty.block_maybe());
    tty.queue_protocol(ProtocolTransaction::new(b"obsolete".to_vec()).projection(7))
        .unwrap();
    tty.cancel_protocol(7);
    tty.close_protocol(ProtocolTransaction::new(b"\x1b_tsp;x;{}\x1b\\".to_vec()))
        .unwrap();
    emitted.extend(drain(&mut tty, 2));
    assert_eq!(emitted, b"\x1b_tsp;f;{}\x1b\\\x1b_tsp;x;{}\x1b\\");
    assert_eq!(tty.queued_protocol_bytes(), 0);
}

#[test]
fn queue_bound_streaming_and_control_reserve() {
    let (mut tty, _master, _) = fixture();
    let count = MAX_PROTOCOL_BYTES;
    tty.queue_protocol(ProtocolTransaction::stream(
        count,
        (0..count / 1024).map(|_| vec![b'x'; 1024]),
    ))
    .unwrap();
    assert_eq!(
        tty.queue_protocol(ProtocolTransaction::new(vec![1])),
        Err(QueueFull)
    );
    tty.queue_protocol(ProtocolTransaction::new(vec![b'c'; PROTOCOL_CONTROL_RESERVE]).control())
        .unwrap();
    assert_eq!(
        tty.queue_protocol(ProtocolTransaction::new(vec![1]).control()),
        Err(QueueFull)
    );
    let output = drain(&mut tty, 8192);
    assert_eq!(output.len(), count + PROTOCOL_CONTROL_RESERVE);
    assert!(output[..count].iter().all(|&byte| byte == b'x'));
    assert!(output[count..].iter().all(|&byte| byte == b'c'));
}

#[test]
fn da1_fifo_late_haveda_and_cancelled_probe() {
    let (mut tty, _master, _) = fixture();
    tty.queue_da1(Da1Owner::Discovery, ProtocolTransaction::new(Vec::new()))
        .unwrap();
    tty.queue_da1(
        Da1Owner::Token(3),
        ProtocolTransaction::new(b"hello".to_vec()).projection(3),
    )
    .unwrap();
    tty.cancel_protocol(3);
    tty.queue_da1(
        Da1Owner::Token(4),
        ProtocolTransaction::new(b"replay".to_vec()),
    )
    .unwrap();
    assert_eq!(drain(&mut tty, 2), b"\x1b[creplay\x1b[c");
    let ctx = KeyDecodeContext {
        flags: TtyFlags::HAVEDA,
        has_session: false,
        ..Default::default()
    };
    for owner in [Some(Da1Owner::Discovery), Some(Da1Owner::Token(4)), None] {
        tty.in_buf.add(b"\x1b[?1;2c");
        assert!(matches!(
            tty.decode_next(&ctx),
            DecodeStep::Complete {
                consumed: 7,
                input: TtyInput::Da1Sentinel { .. },
                ..
            }
        ));
        tty.consume_input(7);
        assert_eq!(tty.resolve_da1(), owner);
    }
    let generation = tty.protocol_generation();
    tty.reset_protocol();
    assert_ne!(generation, tty.protocol_generation());
    assert!(tty.flags.contains(TtyFlags::OPENED));
}

#[test]
fn stop_drains_partial_apc_before_termios_restore() {
    let (mut tty, _master, mut state) = fixture();
    tty.flags.insert(TtyFlags::STARTED);
    tty.queue_protocol(ProtocolTransaction::new(b"\x1b_tsp;f;{}\x1b\\".to_vec()).projection(9))
        .unwrap();
    let mut emitted = Vec::new();
    write_queued(&mut tty.protocol_out, &mut tty.out, |bytes| {
        emitted.extend_from_slice(&bytes[..2]);
        Ok(2)
    })
    .unwrap();
    let generation = tty.protocol_generation();
    tty.stop(&mut state, &TtyOptions::default());
    assert!(tty.protocol_stop);
    assert!(tty.wants_write());
    assert!(!tty.wants_read());
    assert_ne!(generation, tty.protocol_generation());
    emitted.extend(drain(&mut tty, 1));
    assert!(emitted.starts_with(b"\x1b_tsp;f;{}\x1b\\"));
    tty.finish_protocol_stop();
    assert!(!tty.protocol_stop);
    assert!(!tty.flags.contains(TtyFlags::STARTED));
}

#[test]
fn bounded_reads_include_pending_input_and_pause() {
    let (master, slave, _) = rmux_sys::pty::openpty().unwrap();
    let tio = TermiosState::get(slave.as_fd()).unwrap();
    let mut raw = tio;
    raw.make_tty_raw();
    raw.set(slave.as_fd()).unwrap();
    rmux_sys::fd::set_blocking(slave.as_fd(), false);
    let mut tty = Tty::new(slave, tio, TtyHostInfo::default());
    rmux_sys::fd::write(master.as_fd(), b"abcdefgh").unwrap();
    tty.in_buf.add(b"prior");
    tty.set_read_limit(Some(8));
    tty.set_read_paused(true);
    assert_eq!(tty.on_readable(), ReadOutcome::Bytes(0));
    tty.set_read_paused(false);
    assert_eq!(tty.on_readable(), ReadOutcome::Bytes(3));
    assert_eq!(tty.input_len(), 8);
    assert_eq!(tty.input_bytes(), b"priorabc");
}

#[test]
fn protocol_queue_count_bound_leaves_control_slots() {
    let (mut tty, _master, _) = fixture();
    for _ in 0..1024 {
        tty.queue_protocol(ProtocolTransaction::new(vec![b'x']))
            .unwrap();
    }
    assert_eq!(
        tty.queue_protocol(ProtocolTransaction::new(vec![b'x'])),
        Err(QueueFull)
    );
    tty.queue_protocol(ProtocolTransaction::new(b"close".to_vec()).control())
        .unwrap();
    assert_eq!(drain(&mut tty, 1).len(), 1029);
}

#[test]
fn stream_invalid_length_is_an_explicit_io_error() {
    let (mut tty, _master, _) = fixture();
    tty.queue_protocol(ProtocolTransaction::stream(
        8,
        std::iter::once(vec![b'x'; 2]),
    ))
    .unwrap();
    write_queued(&mut tty.protocol_out, &mut tty.out, |bytes| Ok(bytes.len())).unwrap();
    let error =
        write_queued(&mut tty.protocol_out, &mut tty.out, |bytes| Ok(bytes.len())).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(tty.queued_protocol_bytes(), 6);
}
