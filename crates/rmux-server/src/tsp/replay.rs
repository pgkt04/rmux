// Ported from tmux server-client.c, tty.c @ 8f25579c
use super::{
    blobs::TspBlobStore,
    document::TspDocument,
    wire::{APC_LIMIT, JOINED_LIMIT, WireMessage},
};
use base64::Engine;
use serde_json::{Value, json};
use std::sync::Arc;

pub type EncodedStream = (usize, Box<dyn Iterator<Item = Vec<u8>>>);

#[derive(Debug, PartialEq)]
pub enum ReplayError {
    FullFrameTooLarge { bytes: usize },
}

#[derive(Clone, Debug)]
pub struct ReplayPlan {
    pub messages: Vec<WireMessage>,
    pub blobs: Vec<ReplayBlob>,
    pub frame_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct ReplayBlob {
    pub id: String,
    pub mime: Option<String>,
    pub bytes: Arc<[u8]>,
}

pub fn snapshot_ops(document: &TspDocument, outer_id: &str, now_ms: u64) -> Vec<Value> {
    let mut tree = document.snapshot(now_ms);
    let mut ops = Vec::new();
    if let Some(props) = tree.as_object_mut().and_then(|tree| tree.remove("p")) {
        ops.push(Value::Array(vec!["set".into(), outer_id.into(), props]));
    }
    if let Some(Value::Array(children)) = tree.as_object_mut().and_then(|tree| tree.remove("c")) {
        for child in children {
            ops.push(Value::Array(vec![
                "add".into(),
                child["id"].clone(),
                outer_id.into(),
                Value::Null,
                child,
            ]));
        }
    }
    for id in &document.settled {
        ops.push(json!([
            "settle",
            if id == &document.surface {
                outer_id
            } else {
                id
            }
        ]));
    }
    if let Some(focus) = &document.focus {
        ops.push(json!([
            "focus",
            if focus == &document.surface {
                outer_id
            } else {
                focus
            }
        ]));
    }
    if document.suspended {
        ops.push(json!(["suspend"]));
    }
    ops
}

/// Metadata and the full frame, with no blob bodies. Frame size is checked
/// before any `o` is produced so an oversized document never opens a surface.
pub fn snapshot(
    document: &TspDocument,
    outer_id: &str,
    sequence: u64,
    now_ms: u64,
) -> Result<Vec<WireMessage>, ReplayError> {
    plan(
        document,
        &TspBlobStore::new(),
        &std::collections::BTreeSet::new(),
        outer_id,
        sequence,
        now_ms,
    )
    .map(|plan| plan.messages)
}

pub fn snapshot_with_blobs(
    document: &TspDocument,
    blobs: &mut TspBlobStore,
    confirmed: &std::collections::BTreeSet<String>,
    outer_id: &str,
    sequence: u64,
    now_ms: u64,
) -> Result<Vec<WireMessage>, ReplayError> {
    let mut plan = plan(document, blobs, confirmed, outer_id, sequence, now_ms)?;
    let mut messages = Vec::with_capacity(plan.messages.len() + plan.blobs.len());
    let frame = plan.messages.pop().expect("snapshot ends with frame");
    messages.extend(plan.messages);
    for blob in plan.blobs {
        messages.push(blob_message(&blob));
    }
    messages.push(frame);
    Ok(messages)
}

pub fn plan(
    document: &TspDocument,
    blobs: &TspBlobStore,
    confirmed: &std::collections::BTreeSet<String>,
    outer_id: &str,
    sequence: u64,
    now_ms: u64,
) -> Result<ReplayPlan, ReplayError> {
    plan_with_ops(
        document,
        blobs,
        confirmed,
        outer_id,
        sequence,
        snapshot_ops(document, outer_id, now_ms),
    )
}

pub fn plan_with_ops(
    document: &TspDocument,
    blobs: &TspBlobStore,
    confirmed: &std::collections::BTreeSet<String>,
    outer_id: &str,
    sequence: u64,
    ops: Vec<Value>,
) -> Result<ReplayPlan, ReplayError> {
    let frame = frame_message(outer_id, sequence, ops);
    if frame.body.len() > JOINED_LIMIT {
        return Err(ReplayError::FullFrameTooLarge {
            bytes: frame.body.len(),
        });
    }
    let mut messages = vec![WireMessage::json(
        b'o',
        &json!({"id":outer_id,"mode":"screen","listen":true}),
    )];
    if let Some(p) = &document.palette {
        let mut p = p.clone();
        p["sf"] = outer_id.into();
        messages.push(WireMessage::json(b't', &p));
    }
    for (name, css) in &document.sheets {
        messages.push(WireMessage::json(
            b's',
            &json!({"sf":outer_id,"name":name,"css":css}),
        ));
    }
    let mut replay_blobs = Vec::new();
    for id in document.blob_references().difference(confirmed) {
        if let Some(blob) = blobs.peek(id) {
            replay_blobs.push(ReplayBlob {
                id: id.clone(),
                mime: blob.mime.clone(),
                bytes: Arc::clone(&blob.bytes),
            });
        }
    }
    messages.push(frame);
    Ok(ReplayPlan {
        frame_bytes: messages.last().map_or(0, |m| m.body.len()),
        messages,
        blobs: replay_blobs,
    })
}

pub fn frame_message(outer_id: &str, sequence: u64, ops: Vec<Value>) -> WireMessage {
    let value = Value::Object(serde_json::Map::from_iter([
        ("sf".into(), outer_id.into()),
        ("s".into(), sequence.into()),
        ("ops".into(), Value::Array(ops)),
    ]));
    WireMessage::json(b'f', &value)
}

pub fn blob_message(blob: &ReplayBlob) -> WireMessage {
    let mut params = std::collections::BTreeMap::from([("id".into(), blob.id.clone())]);
    if let Some(mime) = &blob.mime {
        params.insert("mime".into(), mime.clone());
    }
    WireMessage {
        verb: b'b',
        params,
        body: base64::engine::general_purpose::STANDARD
            .encode(&blob.bytes)
            .into_bytes(),
    }
}

/// Supplies logical surface metadata without copying daemon-only open parameters.
pub fn set_open_metadata(messages: &mut [WireMessage], logical_open: &Value) {
    let Some(open) = messages.iter_mut().find(|m| m.verb == b'o') else {
        return;
    };
    let mut value: Value = serde_json::from_slice(&open.body).expect("generated open JSON");
    for key in ["title", "role", "listen"] {
        if let Some(v) = logical_open.get(key) {
            value[key] = v.clone();
        }
    }
    open.body = serde_json::to_vec(&value).expect("JSON value serialization");
}

/// The next not-yet-sent replay message. Index order is open, palette, sheets,
/// then each referenced blob, then the frame. Each piece is one wire message
/// whose joined body is at most `JOINED_LIMIT`.
pub fn replay_len(plan: &ReplayPlan) -> usize {
    plan.messages.iter().filter(|m| m.verb != b'f').count()
        + plan.blobs.len()
        + usize::from(plan.messages.iter().any(|m| m.verb == b'f'))
}

pub fn replay_piece(plan: &Arc<ReplayPlan>, index: usize) -> Option<ReplayPiece> {
    let mut at = 0usize;
    for (message_index, message) in plan.messages.iter().enumerate() {
        if message.verb == b'f' {
            continue;
        }
        if at == index {
            return Some(ReplayPiece::Message {
                plan: Arc::clone(plan),
                index: message_index,
            });
        }
        at += 1;
    }
    for blob in &plan.blobs {
        if at == index {
            return Some(ReplayPiece::Blob(blob.clone()));
        }
        at += 1;
    }
    if at == index {
        return plan
            .messages
            .iter()
            .rposition(|m| m.verb == b'f')
            .map(|index| ReplayPiece::Message {
                plan: Arc::clone(plan),
                index,
            });
    }
    None
}

#[derive(Clone, Debug)]
pub enum ReplayPiece {
    Message { plan: Arc<ReplayPlan>, index: usize },
    Blob(ReplayBlob),
}

impl ReplayPiece {
    pub fn message(&self) -> WireMessage {
        match self {
            Self::Message { plan, index } => plan.messages[*index].clone(),
            Self::Blob(blob) => blob_message(blob),
        }
    }
    pub fn is_frame(&self) -> bool {
        matches!(self, Self::Message { plan, index } if plan.messages[*index].verb == b'f')
    }
    pub fn token(&self) -> String {
        match self {
            Self::Message { plan, index } => plan.messages[*index].verb.to_string(),
            Self::Blob(blob) => format!("b{}", blob.id),
        }
    }

    /// Only chunk headers and ranges are prepared at admission. The shared JSON
    /// or decoded blob is read when the tty actually drains each APC.
    pub fn stream(self, apc: usize, token: &str) -> Result<EncodedStream, String> {
        let limit = apc.min(APC_LIMIT);
        let (verb, params, bytes) = match &self {
            Self::Message { plan, index } => {
                let message = &plan.messages[*index];
                (message.verb, message.params.clone(), message.body.len())
            }
            Self::Blob(blob) => {
                let mut params = std::collections::BTreeMap::from([("id".into(), blob.id.clone())]);
                if let Some(mime) = &blob.mime {
                    params.insert("mime".into(), mime.clone());
                }
                (b'b', params, blob.bytes.len().div_ceil(3) * 4)
            }
        };
        if bytes > JOINED_LIMIT {
            return Err("joined body limit".into());
        }
        let header = wire_header(verb, params.clone());
        let mut chunks = Vec::new();
        if header.len().saturating_sub(2) + bytes <= limit {
            chunks.push(StreamChunk {
                header,
                start: 0,
                end: bytes,
            });
        } else {
            let mut at = 0;
            while at < bytes {
                let mut chunk_params = if at == 0 {
                    params.clone()
                } else {
                    Default::default()
                };
                chunk_params.insert("c".into(), token.into());
                chunk_params.insert("m".into(), "1".into());
                let mut header = wire_header(verb, chunk_params.clone());
                let available = limit
                    .checked_sub(header.len() - 2)
                    .filter(|n| *n > 0)
                    .ok_or("APC limit too small")?;
                let mut end = (at + available).min(bytes);
                if end < bytes {
                    match &self {
                        Self::Message { plan, index } => {
                            while end > at && unsafe_prefix(&plan.messages[*index].body[end..]) {
                                end -= 1;
                            }
                        }
                        Self::Blob(_) => end -= (end - at) % 4,
                    }
                    if end == at {
                        return Err("no safe chunk boundary within APC limit".into());
                    }
                } else {
                    chunk_params.remove("m");
                    header = wire_header(verb, chunk_params);
                }
                chunks.push(StreamChunk {
                    header,
                    start: at,
                    end,
                });
                at = end;
            }
        }
        let exact = chunks
            .iter()
            .map(|chunk| chunk.header.len() + chunk.end - chunk.start + 2)
            .sum();
        let stream = chunks.into_iter().map(move |chunk| {
            let StreamChunk {
                mut header,
                start,
                end,
            } = chunk;
            match &self {
                Self::Message { plan, index } => {
                    header.extend_from_slice(&plan.messages[*index].body[start..end])
                }
                Self::Blob(blob) => {
                    let offset = header.len();
                    header.resize(offset + end - start, 0);
                    let raw_start = start / 4 * 3;
                    let raw_end = (end / 4 * 3).min(blob.bytes.len());
                    base64::engine::general_purpose::STANDARD
                        .encode_slice(&blob.bytes[raw_start..raw_end], &mut header[offset..])
                        .expect("exact base64 chunk size");
                }
            }
            header.extend_from_slice(b"\x1b\\");
            header
        });
        Ok((exact, Box::new(stream)))
    }
}

struct StreamChunk {
    header: Vec<u8>,
    start: usize,
    end: usize,
}

fn wire_header(verb: u8, params: std::collections::BTreeMap<String, String>) -> Vec<u8> {
    let mut bytes = WireMessage {
        verb,
        params,
        body: Vec::new(),
    }
    .encode();
    bytes.truncate(bytes.len() - 2);
    bytes
}

/// One queued protocol transaction: exact joined size, chunks produced only as
/// the tty drains. Blob bytes stay behind `Arc` until that chunk is encoded.
pub fn stream_transaction(
    messages: &[WireMessage],
    blobs: &[ReplayBlob],
    apc: usize,
    token: &str,
) -> Result<EncodedStream, String> {
    let plan = Arc::new(ReplayPlan {
        messages: messages.to_vec(),
        blobs: blobs.to_vec(),
        frame_bytes: 0,
    });
    let mut exact = 0;
    let mut streams = Vec::new();
    for index in 0..replay_len(&plan) {
        let piece = replay_piece(&plan, index).expect("replay piece");
        let (bytes, stream) = piece.stream(apc, &format!("{token}-{index}"))?;
        exact += bytes;
        streams.push(stream);
    }
    Ok((exact, Box::new(streams.into_iter().flatten())))
}

fn unsafe_prefix(bytes: &[u8]) -> bool {
    let Some(end) = bytes.iter().position(|b| *b == b';') else {
        return false;
    };
    let segment = &bytes[..end];
    let Some(eq) = segment.iter().position(|b| *b == b'=') else {
        return false;
    };
    eq > 0
        && segment[..eq]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
        && segment[eq + 1..].iter().all(|b| (33..=126).contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsp::wire::Frame;
    #[test]
    fn oversized_snapshot_and_metadata() {
        let mut d = TspDocument::new("s");
        d.apply_frame(
            &Frame {
                sf: "s".into(),
                s: 1,
                ops: vec![json!(["set","s",{"text":"x".repeat(JOINED_LIMIT)}])],
            },
            0,
        )
        .unwrap();
        assert!(matches!(
            snapshot(&d, "o", 1, 0),
            Err(ReplayError::FullFrameTooLarge { .. })
        ));
        let d = TspDocument::new("s");
        let mut messages = snapshot(&d, "o", 1, 0).unwrap();
        set_open_metadata(
            &mut messages,
            &json!({"title":"title","listen":false,"key":"private","buf":"alt","seeded":true}),
        );
        let o: Value = serde_json::from_slice(&messages[0].body).unwrap();
        assert_eq!(o["listen"], false);
        assert_eq!(o["title"], "title");
        assert!(o.get("key").is_none());
    }
    #[test]
    fn replay_regions_age_order() {
        let mut d = TspDocument::new("s");
        d.apply_frame(&Frame{sf:"s".into(),s:7,ops:vec![json!(["add","main","s",null,{"id":"main","k":"col","c":[{"id":"clock","k":"elapsed","p":{"age":-500,"took":8}}]}]),json!(["settle","clock"])]},100).unwrap();
        d.set_palette(json!({"dark":{}})).unwrap();
        d.set_sheet("a", Some(&json!("a{}"))).unwrap();
        let r = snapshot(&d, "outer", 1, 300).unwrap();
        assert_eq!(r.iter().map(|m| m.verb).collect::<Vec<_>>(), b"otsf");
        let f: Value = serde_json::from_slice(&r[3].body).unwrap();
        assert_eq!(f["ops"][0][4]["c"][0]["p"]["age"], -300.0);
        assert_eq!(f["ops"][0][4]["c"][0]["p"]["took"], 8);
        assert_eq!(f["ops"][0][2], "outer");
    }
    #[test]
    fn shared_blobs_are_not_eager_base64_and_stream_matches_chunks() {
        use sha2::{Digest, Sha256};
        let mut store = TspBlobStore::new();
        let raw = vec![b'x'; 200_000];
        let hash = format!("{:x}", Sha256::digest(&raw));
        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
        store
            .insert(&hash, Some("application/octet-stream"), encoded.as_bytes())
            .unwrap();
        let mut d = TspDocument::new("s");
        d.apply_frame(
            &Frame {
                sf: "s".into(),
                s: 1,
                ops: vec![json!(["add","img","s",null,{"id":"img","k":"image","p":{"blob":hash}}])],
            },
            0,
        )
        .unwrap();
        let before = std::sync::Arc::strong_count(&store.peek(&hash).unwrap().bytes);
        let plan = plan(
            &d,
            &store,
            &std::collections::BTreeSet::new(),
            "outer",
            1,
            0,
        )
        .unwrap();
        assert_eq!(
            std::sync::Arc::strong_count(&plan.blobs[0].bytes),
            before + 1
        );
        assert!(plan.messages.iter().all(|message| message.verb != b'b'));
        let (exact, chunks) = stream_transaction(&plan.messages, &plan.blobs, 65536, "1").unwrap();
        let chunks: Vec<Vec<u8>> = chunks.collect();
        let joined = chunks.concat();
        assert_eq!(joined.len(), exact);
        let blob_body: Vec<u8> = chunks
            .iter()
            .filter_map(|chunk| {
                let message = super::super::wire::parse(&chunk[2..chunk.len() - 2]).unwrap();
                (message.verb == b'b').then_some(message.body)
            })
            .flatten()
            .collect();
        assert_eq!(blob_body, encoded.as_bytes());
        assert!(
            joined
                .windows(b"tsp;o;".len())
                .any(|window| window == b"tsp;o;")
        );
        assert!(
            joined
                .windows(b"tsp;f;".len())
                .any(|window| window == b"tsp;f;")
        );
        let blob_at = joined
            .windows(b"tsp;b;".len())
            .position(|window| window == b"tsp;b;");
        let frame_at = joined
            .windows(b"tsp;f;".len())
            .position(|window| window == b"tsp;f;");
        assert!(blob_at < frame_at);
    }

    #[test]
    fn lazy_piece_matches_message_chunks_and_blob_padding() {
        for bytes in [0, 1, 2, 3, 4, 79, 80, 81, 199] {
            let raw = vec![b'x'; bytes];
            let blob = ReplayBlob {
                id: "blob".into(),
                mime: None,
                bytes: Arc::from(raw.clone()),
            };
            let (exact, chunks) = ReplayPiece::Blob(blob).stream(64, "token").unwrap();
            let chunks: Vec<_> = chunks.collect();
            assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), exact);
            assert!(chunks.iter().all(|chunk| chunk.len() - 4 <= 64));
            let body: Vec<u8> = chunks
                .iter()
                .flat_map(|chunk| {
                    super::super::wire::parse(&chunk[2..chunk.len() - 2])
                        .unwrap()
                        .body
                })
                .collect();
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .unwrap(),
                raw
            );
        }
        let message = WireMessage::json(b'f', &json!({"text":"x".repeat(1000)}));
        let expected = message.chunks(64, "token").unwrap();
        let plan = Arc::new(ReplayPlan {
            messages: vec![message],
            blobs: Vec::new(),
            frame_bytes: 0,
        });
        let (exact, chunks) = replay_piece(&plan, 0).unwrap().stream(64, "token").unwrap();
        let chunks: Vec<_> = chunks.collect();
        assert_eq!(chunks, expected);
        assert_eq!(exact, chunks.iter().map(Vec::len).sum::<usize>());
    }
}
