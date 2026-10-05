// TSP broker extension; https://docs.stencil.so/tern/protocol/transport.md#chunking
use super::wire::{JOINED_LIMIT, WireMessage};

const IDLE_MS: u64 = 1_000;
const OVERFLOW: &str = "over 25165824 bytes";

#[derive(Debug)]
pub struct ChunkSequence {
    message: WireMessage,
    token: String,
    deadline_ms: u64,
}

impl ChunkSequence {
    pub fn deadline(&self) -> u64 {
        self.deadline_ms
    }

    pub fn interrupted_by(&self, message: &WireMessage) -> bool {
        self.message.verb != message.verb || message.params.get("c") != Some(&self.token)
    }

    pub fn dropped_error(&self, reason: &str) -> String {
        dropped_error(self.message.verb, &self.token, reason)
    }
}

fn dropped_error(verb: u8, token: &str, reason: &str) -> String {
    format!(
        "chunked {} message c={token} dropped: {reason}",
        char::from(verb)
    )
}

/// Drop an idle sequence. The caller emits its timeout error before expiring it;
/// resetting the slot directly implements RIS's silent drop.
pub fn expire(slot: &mut Option<ChunkSequence>, now_ms: u64) -> bool {
    if slot
        .as_ref()
        .is_some_and(|sequence| now_ms >= sequence.deadline())
    {
        *slot = None;
        true
    } else {
        false
    }
}

/// Join bytes before UTF-8/JSON parsing. Only `m=1` means more chunks follow.
/// The caller reports an old sequence's interruption/timeout before calling;
/// this result belongs to the incoming message, which must still be handled.
pub fn join(
    slot: &mut Option<ChunkSequence>,
    mut message: WireMessage,
    now_ms: u64,
) -> Result<Option<WireMessage>, String> {
    expire(slot, now_ms);
    let more = message.params.get("m").is_some_and(|value| value == "1");
    if let Some(mut sequence) = slot.take() {
        if !sequence.interrupted_by(&message) {
            if message.body.len() > JOINED_LIMIT - sequence.message.body.len() {
                return Err(sequence.dropped_error(OVERFLOW));
            }
            sequence.message.body.extend_from_slice(&message.body);
            if more {
                sequence.deadline_ms = now_ms.saturating_add(IDLE_MS);
                *slot = Some(sequence);
                return Ok(None);
            }
            return Ok(Some(sequence.message));
        }
    }
    let Some(token) = message.params.remove("c") else {
        return if message.body.len() <= JOINED_LIMIT {
            Ok(Some(message))
        } else {
            Err("joined body limit".into())
        };
    };
    message.params.remove("m");
    if message.body.len() > JOINED_LIMIT {
        return Err(dropped_error(message.verb, &token, OVERFLOW));
    }
    if more {
        *slot = Some(ChunkSequence {
            message,
            token,
            deadline_ms: now_ms.saturating_add(IDLE_MS),
        });
        Ok(None)
    } else {
        Ok(Some(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsp::wire::parse;
    use std::collections::BTreeMap;

    fn chunk(verb: u8, token: &str, more: bool, body: &[u8]) -> WireMessage {
        let mut params = BTreeMap::from([("c".into(), token.into())]);
        if more {
            params.insert("m".into(), "1".into());
        }
        WireMessage {
            verb,
            params,
            body: body.to_vec(),
        }
    }

    #[test]
    fn golden_utf8_joins_at_every_byte_before_json() {
        let body = "{\"text\":\"é🙂\u{009c};x=1;y\"}".as_bytes();
        for split in 0..=body.len() {
            let mut slot = None;
            assert_eq!(
                join(&mut slot, chunk(b'f', "k7", true, &body[..split]), 0),
                Ok(None)
            );
            let joined = join(&mut slot, chunk(b'f', "k7", false, &body[split..]), 1)
                .unwrap()
                .unwrap();
            assert_eq!(joined.verb, b'f');
            assert_eq!(joined.body, body);
            assert!(joined.params.is_empty());
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&joined.body).unwrap(),
                serde_json::json!({"text":"é🙂\u{009c};x=1;y"})
            );
            assert!(slot.is_none());
        }
    }

    #[test]
    fn golden_first_parameters_survive_and_later_parameters_are_ignored() {
        let first = parse(b"tsp;b;c=k7;m=1;id=first;mime=image/png;YW").unwrap();
        let last = parse(b"tsp;b;c=k7;id=ignored;mime=text/plain;Jj").unwrap();
        let mut slot = None;
        assert_eq!(join(&mut slot, first, 10), Ok(None));
        let joined = join(&mut slot, last, 11).unwrap().unwrap();
        assert_eq!(
            joined,
            WireMessage {
                verb: b'b',
                params: BTreeMap::from([
                    ("id".into(), "first".into()),
                    ("mime".into(), "image/png".into())
                ]),
                body: b"YWJj".to_vec(),
            }
        );
    }

    #[test]
    fn verb_and_chunk_id_interrupt_but_replacement_is_processed() {
        for (verb, token) in [(b'f', "new"), (b'b', "old")] {
            let mut slot = None;
            join(&mut slot, chunk(b'f', "old", true, b"old-body"), 0).unwrap();
            let replacement = chunk(verb, token, true, b"new-");
            let old = slot.as_ref().unwrap();
            assert!(old.interrupted_by(&replacement));
            assert_eq!(
                old.dropped_error("interrupted"),
                "chunked f message c=old dropped: interrupted"
            );
            assert_eq!(join(&mut slot, replacement, 10), Ok(None));
            let result = join(&mut slot, chunk(verb, token, false, b"body"), 11)
                .unwrap()
                .unwrap();
            assert_eq!(result.verb, verb);
            assert_eq!(result.body, b"new-body");
            assert!(slot.is_none());
        }
    }

    #[test]
    fn nonchunk_interruption_returns_the_incoming_message_unchanged() {
        let mut slot = None;
        join(&mut slot, chunk(b'f', "old", true, b"old"), 0).unwrap();
        let incoming = parse(b"tsp;q;m=1;{\"q\":\"hello\"}").unwrap();
        assert!(slot.as_ref().unwrap().interrupted_by(&incoming));
        assert_eq!(
            join(&mut slot, incoming, 1),
            Ok(Some(WireMessage {
                verb: b'q',
                params: BTreeMap::from([("m".into(), "1".into())]),
                body: b"{\"q\":\"hello\"}".to_vec(),
            }))
        );
        assert!(slot.is_none());
    }

    #[test]
    fn interrupted_by_complete_chunk_returns_new_body() {
        let mut slot = None;
        join(&mut slot, chunk(b'f', "old", true, b"old"), 0).unwrap();
        let result = join(&mut slot, chunk(b'f', "new", false, b"new"), 1)
            .unwrap()
            .unwrap();
        assert_eq!(result.body, b"new");
        assert!(result.params.is_empty());
        assert!(slot.is_none());
    }

    #[test]
    fn idle_timeout_refreshes_per_chunk_and_expires_at_deadline() {
        let mut slot = None;
        assert!(!expire(&mut slot, 0));
        join(&mut slot, chunk(b'f', "k7", true, b"first"), 10).unwrap();
        assert_eq!(slot.as_ref().unwrap().deadline(), 1_010);
        assert!(!expire(&mut slot, 1_009));
        join(&mut slot, chunk(b'f', "k7", true, b"second"), 1_009).unwrap();
        let sequence = slot.as_ref().unwrap();
        assert_eq!(sequence.deadline(), 2_009);
        assert_eq!(
            sequence.dropped_error("timed out"),
            "chunked f message c=k7 dropped: timed out"
        );
        assert!(!expire(&mut slot, 2_008));
        assert!(expire(&mut slot, 2_009));
        assert!(slot.is_none());
        assert!(!expire(&mut slot, 2_010));
    }

    #[test]
    fn late_chunk_starts_fresh_instead_of_reviving_expired_bytes() {
        let mut slot = None;
        join(&mut slot, chunk(b'f', "k7", true, b"old"), 0).unwrap();
        let joined = join(&mut slot, chunk(b'f', "k7", false, b"new"), 1_000)
            .unwrap()
            .unwrap();
        assert_eq!(joined.body, b"new");
        assert!(slot.is_none());
    }

    #[test]
    fn only_m_equals_one_requests_more_chunks() {
        for value in ["", "0", "true", "2"] {
            let mut slot = None;
            let mut message = chunk(b'f', "k7", false, b"body");
            message.params.insert("m".into(), value.into());
            assert_eq!(join(&mut slot, message, 0).unwrap().unwrap().body, b"body");
            assert!(slot.is_none());
        }
    }

    #[test]
    fn reset_discards_sequence_without_a_join_error() {
        let mut slot = None;
        join(&mut slot, chunk(b'f', "k7", true, b"old"), 0).unwrap();
        slot = None;
        assert!(!expire(&mut slot, 1_000));
        assert_eq!(
            join(&mut slot, chunk(b'f', "k7", false, b"new"), 1_001)
                .unwrap()
                .unwrap()
                .body,
            b"new"
        );
    }

    #[test]
    fn joined_limit_is_inclusive_and_overflow_drops_the_sequence() {
        let mut slot = None;
        let mut first = chunk(b'f', "k7", true, b"");
        first.body = vec![b'a'; JOINED_LIMIT - 1];
        join(&mut slot, first, 0).unwrap();
        let joined = join(&mut slot, chunk(b'f', "k7", false, b"b"), 1)
            .unwrap()
            .unwrap();
        assert_eq!(joined.body.len(), JOINED_LIMIT);
        assert_eq!(joined.body.last(), Some(&b'b'));
        assert!(slot.is_none());
        let mut first = chunk(b'f', "k7", true, b"");
        first.body = joined.body;
        join(&mut slot, first, 2).unwrap();
        assert_eq!(
            join(&mut slot, chunk(b'f', "k7", true, b"c"), 3),
            Err("chunked f message c=k7 dropped: over 25165824 bytes".into())
        );
        assert!(slot.is_none());
        let result = join(&mut slot, chunk(b'f', "k7", false, b"new"), 4)
            .unwrap()
            .unwrap();
        assert_eq!(result.body, b"new");
    }

    #[test]
    fn oversized_initial_chunk_is_rejected_without_retention() {
        let mut slot = None;
        let mut message = chunk(b'b', "large", true, b"");
        message.body = vec![b'A'; JOINED_LIMIT + 1];
        assert_eq!(
            join(&mut slot, message, 0),
            Err("chunked b message c=large dropped: over 25165824 bytes".into())
        );
        assert!(slot.is_none());
    }

    #[test]
    fn golden_safe_parameter_prefixes_roundtrip_through_wire_parser() {
        let message = WireMessage::json(b'f', &serde_json::json!({"text":"é🙂;x=1;y".repeat(30)}));
        for limit in 40..70 {
            let mut slot = None;
            let mut joined = None;
            for encoded in message.chunks(limit, "k7").unwrap() {
                let chunk = parse(&encoded[2..encoded.len() - 2]).unwrap();
                joined = join(&mut slot, chunk, 0).unwrap();
            }
            assert_eq!(joined, Some(message.clone()));
            assert!(slot.is_none());
        }
    }
}
