// Ported from tmux control.c @ 8f25579c
use super::*;
use crate::ids::{Arena, PaneId};
struct Host {
    pane: PaneId,
    data: Vec<u8>,
    now: u64,
}
impl ControlTransport for Host {
    fn pane_bytes(&self, pane: PaneId, offset: PaneOffset) -> Option<&[u8]> {
        if pane == self.pane {
            self.data.get(offset.used as usize..)
        } else {
            None
        }
    }
    fn now_ms(&self) -> u64 {
        self.now
    }
}
fn fixture() -> (ControlState, Host) {
    let mut a: Arena<(), PaneId> = Arena::default();
    let pane = a.insert(()).unwrap();
    (
        ControlState::default(),
        Host {
            pane,
            data: b"a\nb\\c".to_vec(),
            now: 100,
        },
    )
}
#[test]
fn byte_corpus() {
    let input: Vec<u8> = (0..=255).collect();
    let mut encoded = Vec::new();
    encode_into(&input, &mut encoded);
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < encoded.len() {
        if encoded[i] == b'\\' {
            decoded.push(
                (encoded[i + 1] - b'0') * 64 + (encoded[i + 2] - b'0') * 8 + encoded[i + 3] - b'0',
            );
            i += 4;
        } else {
            decoded.push(encoded[i]);
            i += 1;
        }
    }
    assert_eq!(decoded, input);
    let n = encoded.len();
    encode_into(b"", &mut encoded);
    assert_eq!(n, encoded.len());
}
#[test]
fn typed_guards_defer_fifo_fake_guard_inert() {
    let mut s = ControlState::default();
    s.write(b"%begin fake");
    assert_eq!(s.guard_depth, 0);
    s.write_guard(ControlGuard::Begin, 1, 2, 1);
    s.notify_write(b"n1");
    s.write_guard(ControlGuard::Begin, 1, 3, 1);
    s.notify_write(b"n2");
    assert_eq!(s.queued_reply_bytes, 0);
    s.write_guard(ControlGuard::End, 1, 3, 1);
    assert_eq!(s.guard_depth, 1);
    s.write_guard(ControlGuard::Error, 1, 2, 1);
    s.write_guard(ControlGuard::End, 1, 4, 0);
    assert_eq!(
        s.output,
        b"%begin fake\n%begin 1 2 1\n%begin 1 3 1\n%end 1 3 1\n%error 1 2 1\nn1\nn2\n%end 1 4 0\n"
    );
    assert_eq!(s.guard_depth, 0);
}
#[test]
fn barrier_partial_and_accounting() {
    let (mut s, h) = fixture();
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 5 },
        100,
    );
    s.write(b"reply");
    assert_eq!(s.queued_reply_bytes, 6);
    s.service_pane(0, 2, &h);
    assert_eq!(s.output, b"%output %0 a\\012\n");
    assert_eq!(s.queued_reply_bytes, 6);
    s.service(&h);
    assert_eq!(s.output, b"%output %0 a\\012\n%output %0 b\\134c\nreply\n");
    assert_eq!(s.queued_reply_bytes, 0);
    assert_eq!(s.panes[&0].sent.used, 5);
    assert!(!s.all_done());
    let n = s.output.len();
    s.consume_output(n);
    assert!(s.all_done());
}
#[test]
fn independent_flags_resume_without_replay() {
    let (mut s, h) = fixture();
    s.set_pane_off(0, h.pane, PaneOffset { used: 0 });
    s.pause_pane(0, h.pane, PaneOffset { used: 0 });
    assert_eq!(s.pane_offset(0), ControlOffsetStatus::default());
    s.continue_pane(0, PaneOffset { used: 3 });
    assert_eq!(
        s.pane_offset(0),
        ControlOffsetStatus {
            offset: None,
            suppress_read: true
        }
    );
    s.set_pane_on(0, PaneOffset { used: 6 });
    assert_eq!(s.panes[&0].sent.used, 6);
    assert_eq!(s.output, b"%pause %0\n%continue %0\n");
}
#[test]
fn exact_lag_and_equal_time() {
    let (mut s, mut h) = fixture();
    s.pause_after = Some(10);
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 6 },
        100,
    );
    assert!(!s.check_age(0, 100));
    assert!(!s.check_age(0, 109));
    assert!(s.check_age(0, 110));
    assert!(s.panes[&0].flags.paused);
    assert_eq!(s.output, b"%pause %0\n");
    let (mut s, _) = fixture();
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 6 },
        100,
    );
    h.now = 300100;
    s.service(&h);
    assert!(s.exiting);
    assert_eq!(s.exit_reason, Some("too far behind"));
    assert!(s.head.is_none());
}
#[test]
fn reply_limit_equality_permanent_discard() {
    let mut s = ControlState {
        queued_reply_bytes: REPLY_LIMIT - 2,
        ..ControlState::default()
    };
    s.write(b"");
    assert!(!s.discard_replies);
    s.queued_reply_bytes = REPLY_LIMIT - 2;
    s.output.clear();
    s.write(b"x");
    assert!(s.discard_replies);
    assert!(s.exiting);
    s.output.clear();
    s.write(b"late");
    s.notify_write(b"late");
    s.write_guard(ControlGuard::Begin, 0, 0, 0);
    assert!(s.output.is_empty());
}
#[test]
fn discard_preserves_replies_and_forced_preserves_encoded() {
    let (mut s, h) = fixture();
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 6 },
        100,
    );
    s.write(b"reply");
    s.discard();
    assert!(!s.all_done());
    s.service(&h);
    assert_eq!(s.output, b"reply\n");
    s.discard_all();
    assert_eq!(s.output, b"reply\n");
    assert!(!s.write_enabled);
}
#[test]
fn absent_window_override_and_late_state() {
    let mut s = ControlState::default();
    s.set_window_size(999, 80, 24);
    assert_eq!(s.get_window_size(999), Some((80, 24)));
    s.clear_window_size(999);
    assert_eq!(s.get_window_size(999), None);
    let mut state = Some(s);
    stop_state(&mut state);
    write_state(&mut state, b"late");
    notify_write_state(&mut state, b"late");
    write_guard_state(&mut state, ControlGuard::Begin, 0, 0, 0);
    assert!(state.is_none());
}
#[test]
fn unlink_head_middle_tail_and_reuse_generations() {
    let (mut s, h) = fixture();
    s.add_pane(0, h.pane, PaneOffset { used: 0 });
    let a = s.insert(Payload::Raw {
        pane: 0,
        remaining: 1,
        time: 0,
    });
    let b = s.insert(Payload::Raw {
        pane: 0,
        remaining: 1,
        time: 0,
    });
    let c = s.insert(Payload::Raw {
        pane: 0,
        remaining: 1,
        time: 0,
    });
    s.remove(b);
    assert_eq!(s.block(a).next, Some(c));
    assert_eq!(s.block(c).pane_prev, Some(a));
    s.remove(a);
    assert_eq!(s.head, Some(c));
    assert_eq!(s.panes[&0].head, Some(c));
    s.remove(c);
    assert!(s.head.is_none());
    assert!(s.panes[&0].tail.is_none());
    let d = s.insert(Payload::Raw {
        pane: 0,
        remaining: 1,
        time: 0,
    });
    assert_eq!(d.slot, c.slot);
    assert_ne!(d.generation, c.generation);
}
#[test]
fn minimum_write_overshoots_high_target_without_loss() {
    let (mut s, mut h) = fixture();
    h.data = vec![0; 10000];
    s.output = vec![b'x'; BUFFER_HIGH - 1];
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 10000 },
        100,
    );
    s.service(&h);
    assert!(s.output.len() > BUFFER_HIGH);
    assert_eq!(s.panes[&0].sent.used, 32);
    let n = s.output.len();
    s.consume_output(n);
    s.service(&h);
    assert!(s.panes[&0].sent.used > 32);
}
#[test]
fn reset_preserves_flags_pending_cleanup_and_rebase() {
    let (mut s, h) = fixture();
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 5 },
        100,
    );
    s.panes.get_mut(&0).unwrap().flags.off = true;
    s.reset_pane(0, PaneOffset { used: 10 });
    assert!(s.panes[&0].flags.off);
    assert!(s.all_done());
    assert_eq!(s.pending.len(), 1);
    s.service(&h);
    assert!(s.pending.is_empty());
    s.rebase_offsets(h.pane, 8);
    assert_eq!(s.panes[&0].sent.used, 2);
    assert_eq!(s.panes[&0].queued.used, 2);
    s.reset_offsets();
    assert!(s.panes.is_empty());
}
#[test]
fn adjacent_blocks_use_first_extended_age_and_pause_defers() {
    let (mut s, h) = fixture();
    s.pause_after = Some(1000);
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 2 },
        90,
    );
    s.write_output(
        0,
        h.pane,
        PaneOffset { used: 0 },
        PaneOffset { used: 5 },
        95,
    );
    s.service(&h);
    assert_eq!(s.output, b"%extended-output %0 10 : a\\012b\\134c\n");
    s.output.clear();
    s.write_guard(ControlGuard::Begin, 1, 1, 1);
    s.pause_pane(0, h.pane, PaneOffset { used: 5 });
    s.continue_pane(0, PaneOffset { used: 5 });
    assert_eq!(s.output, b"%begin 1 1 1\n");
    s.write_guard(ControlGuard::End, 1, 1, 1);
    assert_eq!(
        s.output,
        b"%begin 1 1 1\n%end 1 1 1\n%pause %0\n%continue %0\n"
    );
}
#[test]
fn deferred_does_not_participate_in_all_done_or_reply_count() {
    let mut s = ControlState {
        guard_depth: 1,
        ..ControlState::default()
    };
    s.notify_write(b"deferred");
    assert!(s.all_done());
    assert_eq!(s.queued_reply_bytes, 0);
    s.write_guard(ControlGuard::End, 0, 0, 0);
    assert_eq!(s.output, b"%end 0 0 0\ndeferred\n");
}
