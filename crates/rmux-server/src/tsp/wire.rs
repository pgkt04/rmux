use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const APC_LIMIT: usize = 262_144;
pub const JOINED_LIMIT: usize = 24 * 1024 * 1024;
pub const DA1: &[u8] = b"\x1b[c";
pub const PANE_DA1: &[u8] = b"\x1b[?1;2c";
pub const BROKER_ENV: &str = "RMUX_TSP";
pub const BROKER_FEATURE: &str = "rmux-reprobe";
pub const KINDS: &[&str] = &[
    "col",
    "row",
    "card",
    "section",
    "rule",
    "spacer",
    "text",
    "md",
    "code",
    "diff",
    "ansi",
    "math",
    "image",
    "kv",
    "table",
    "tree",
    "badge",
    "kbd",
    "icon",
    "spinner",
    "shimmer",
    "elapsed",
    "progress",
    "rate",
    "list",
    "item",
    "tabs",
    "editor",
    "input",
    "status",
    "seg",
    "overlay",
    "toast",
    "rows",
    "picker",
    "prefs",
    "tool",
    "checklist",
    "agent",
    "chart",
    "meter",
    "effort",
    "block",
    "el",
];
pub const TEXT_KINDS: &[&str] = &[
    "text", "md", "code", "ansi", "math", "editor", "input", "shimmer", "el",
];
pub const FEATURES: &[&str] = &[
    "blobs",
    "settle",
    "adopt",
    "dock",
    "program-palette",
    "reduce-motion",
    "scroll",
    "styles",
];
const _: () = assert!(KINDS.len() <= 64 && FEATURES.len() <= 64);

/// A subset of [`KINDS`] or [`FEATURES`], one bit per entry. The broker
/// recomputes the display contract on every loop turn, so it must compare and
/// copy without allocating.
#[derive(Clone, Copy, Debug)]
pub struct NameSet {
    table: &'static [&'static str],
    bits: u64,
}
impl NameSet {
    pub fn all(table: &'static [&'static str]) -> Self {
        Self::matching(table, |_| true)
    }
    pub fn matching(table: &'static [&'static str], mut keep: impl FnMut(&str) -> bool) -> Self {
        let bits = table
            .iter()
            .enumerate()
            .filter(|(_, name)| keep(name))
            .fold(0, |bits, (i, _)| bits | 1 << i);
        Self { table, bits }
    }
    pub fn contains(&self, name: &str) -> bool {
        self.table
            .iter()
            .position(|entry| *entry == name)
            .is_some_and(|i| self.bits & 1 << i != 0)
    }
    pub fn iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.table
            .iter()
            .enumerate()
            .filter(|(i, _)| self.bits & 1 << i != 0)
            .map(|(_, name)| *name)
    }
}
impl PartialEq for NameSet {
    fn eq(&self, other: &Self) -> bool {
        self.bits == other.bits && std::ptr::eq(self.table, other.table)
    }
}
impl Serialize for NameSet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub sf: String,
    pub s: u64,
    pub ops: Vec<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Hello {
    pub v: u8,
    /// The host's name in its `hello` reply (`tern`, `rsttyd`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    /// The inline surfaces the host keeps parked per terminal; absent or 1
    /// is the single slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspend: Option<u32>,
    #[serde(default)]
    pub kinds: BTreeSet<String>,
    #[serde(default)]
    pub features: BTreeSet<String>,
    #[serde(default = "default_apc")]
    pub apc: usize,
    #[serde(default = "default_credits")]
    pub credits: usize,
    #[serde(default)]
    pub cell: Option<Value>,
    #[serde(default)]
    pub dark: Option<bool>,
    #[serde(default, rename = "reduceMotion")]
    pub reduce_motion: Option<bool>,
}
fn default_apc() -> usize {
    65536
}
fn default_credits() -> usize {
    2
}
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayContract {
    pub kinds: NameSet,
    pub features: NameSet,
    pub apc: usize,
    pub credits: usize,
    pub cols: u32,
    pub cell: Option<Value>,
    pub dark: Option<bool>,
    pub reduce_motion: Option<bool>,
}
impl DisplayContract {
    /// [`Self::intersect`] gives a contract for these viewers.
    pub fn accepts(viewers: &[&Hello], leader: usize) -> bool {
        leader < viewers.len()
            && viewers
                .iter()
                .all(|h| h.v == 1 && h.credits > 0 && h.apc >= 64)
    }
    pub fn intersect(viewers: &[&Hello], leader: usize, cols: u32) -> Option<Self> {
        if !Self::accepts(viewers, leader) {
            return None;
        }
        let first = viewers[leader];
        Some(Self {
            kinds: NameSet::matching(KINDS, |k| viewers.iter().all(|h| h.kinds.contains(k))),
            features: NameSet::matching(FEATURES, |k| {
                viewers.iter().all(|h| h.features.contains(k))
            }),
            apc: viewers.iter().map(|h| h.apc).min()?.min(65536),
            credits: viewers.iter().map(|h| h.credits).min()?.min(2),
            cols,
            cell: first.cell.clone(),
            dark: first.dark,
            reduce_motion: first.reduce_motion,
        })
    }
    pub fn hello(&self, epoch: u64) -> Value {
        json!({"r":"hello","v":1,"term":"rmux","version":env!("CARGO_PKG_VERSION"),"kinds":self.kinds,"features":self.features,"apc":self.apc,"credits":self.credits,"cols":self.cols,"cell":self.cell,"dark":self.dark,"reduceMotion":self.reduce_motion,"rmux":{"broker":1,"epoch":epoch,"strictCredits":true}})
    }
}
pub fn probe_reply(epoch: u64, native: bool, accepted: Option<bool>) -> Value {
    let mut v = json!({"r":"rmux-probe","epoch":epoch,"native":native});
    if let Some(a) = accepted {
        v["accepted"] = a.into();
    }
    v
}
pub fn ready_reply(epoch: u64, accepted: bool) -> Value {
    json!({"r":"rmux-ready","epoch":epoch,"accepted":accepted})
}
pub fn view_event(epoch: u64, reason: &str) -> Value {
    json!({"ev":"rmux-view","epoch":epoch,"reason":reason})
}
pub fn hello_query(app: &str, features: &[String], epoch: Option<u64>) -> Value {
    let mut q = json!({"q":"hello","v":[1],"app":app,"features":features});
    if let Some(e) = epoch {
        q["rmuxEpoch"] = e.into();
    }
    q
}
#[derive(Clone, Debug, PartialEq)]
pub struct WireMessage {
    pub verb: u8,
    pub params: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
impl WireMessage {
    pub fn json(verb: u8, body: &Value) -> Self {
        Self {
            verb,
            params: BTreeMap::new(),
            body: serde_json::to_vec(body).expect("JSON value serialization"),
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = b"\x1b_tsp;".to_vec();
        out.push(self.verb);
        out.push(b';');
        for (k, v) in &self.params {
            out.extend_from_slice(k.as_bytes());
            out.push(b'=');
            out.extend_from_slice(v.as_bytes());
            out.push(b';');
        }
        out.extend_from_slice(&self.body);
        out.extend_from_slice(b"\x1b\\");
        out
    }
    pub fn chunks(&self, limit: usize, token: &str) -> Result<Vec<Vec<u8>>, String> {
        if self.body.len() > JOINED_LIMIT {
            return Err("joined body limit".into());
        }
        if self.encode().len().saturating_sub(4) <= limit.min(APC_LIMIT) {
            return Ok(vec![self.encode()]);
        }
        let mut out = Vec::new();
        let mut at = 0;
        while at < self.body.len() {
            let mut chunk = Self {
                verb: self.verb,
                params: if at == 0 {
                    self.params.clone()
                } else {
                    BTreeMap::new()
                },
                body: Vec::new(),
            };
            chunk.params.insert("c".into(), token.into());
            chunk.params.insert("m".into(), "1".into());
            let overhead = chunk.encode().len() - 4;
            let available = limit
                .min(APC_LIMIT)
                .checked_sub(overhead)
                .filter(|n| *n > 0)
                .ok_or("APC limit too small")?;
            let mut end = (at + available).min(self.body.len());
            if end < self.body.len() {
                while end > at && unsafe_prefix(&self.body[end..]) {
                    end -= 1;
                }
                if end == at {
                    return Err("no safe chunk boundary within APC limit".into());
                }
            } else {
                chunk.params.remove("m");
            }
            chunk.body.extend_from_slice(&self.body[at..end]);
            out.push(chunk.encode());
            at = end;
        }
        Ok(out)
    }
}
pub fn parse(payload: &[u8]) -> Result<WireMessage, String> {
    if payload.len() > APC_LIMIT || !payload.starts_with(b"tsp;") || payload.get(5) != Some(&b';') {
        return Err("invalid TSP framing".into());
    }
    let mut at = 6;
    let mut params = BTreeMap::new();
    while let Some(end) = payload[at..]
        .iter()
        .position(|b| *b == b';')
        .map(|n| n + at)
    {
        let segment = &payload[at..end];
        let Some(eq) = segment.iter().position(|b| *b == b'=') else {
            break;
        };
        if eq == 0
            || !segment[..eq]
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
            || !segment[eq + 1..].iter().all(|b| (33..=126).contains(b))
        {
            break;
        }
        params.insert(
            String::from_utf8_lossy(&segment[..eq]).into_owned(),
            String::from_utf8_lossy(&segment[eq + 1..]).into_owned(),
        );
        at = end + 1;
    }
    Ok(WireMessage {
        verb: payload[4],
        params,
        body: payload[at..].to_vec(),
    })
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
    #[test]
    fn chunks_roundtrip_parameter_shaped_and_utf8() {
        let m = WireMessage::json(b'f', &json!({"text":"é;x=1;y".repeat(100)}));
        for limit in 40..100 {
            let chunks = m.chunks(limit, "k").unwrap();
            let mut body = Vec::new();
            for c in chunks {
                assert!(c.len() - 4 <= limit);
                let p = parse(&c[2..c.len() - 2]).unwrap();
                body.extend(p.body);
            }
            assert_eq!(body, m.body);
        }
    }
    #[test]
    fn capability_intersection_excludes_flow() {
        let mut h: Hello = serde_json::from_value(json!({"v":1,"kinds":["col","el"],"features":["flow","styles"],"credits":7,"apc":100000})).unwrap();
        let contract = DisplayContract::intersect(&[&h], 0, 80).unwrap();
        assert_eq!(contract.credits, 2);
        assert_eq!(contract.apc, 65536);
        assert!(!contract.features.contains("flow"));
        assert!(contract.features.contains("styles"));
        assert_eq!(contract.kinds.iter().collect::<Vec<_>>(), ["col", "el"]);
        h.credits = 0;
        assert!(DisplayContract::intersect(&[&h], 0, 80).is_none());
    }
    #[test]
    fn golden() {
        assert_eq!(
            WireMessage::json(b'r', &probe_reply(17, false, None)).encode(),
            b"\x1b_tsp;r;{\"epoch\":17,\"native\":false,\"r\":\"rmux-probe\"}\x1b\\"
        );
        assert_eq!(DA1, b"\x1b[c");
        assert_eq!(PANE_DA1, b"\x1b[?1;2c");
    }
    #[test]
    fn lookahead() {
        let m = parse(b"tsp;f;c=a;{\"text\":\"x=1;y\"}").unwrap();
        assert_eq!(m.params["c"], "a");
        assert_eq!(m.body, b"{\"text\":\"x=1;y\"}");
        assert!(serde_json::from_value::<Hello>(json!({"v":1,"future":true})).is_ok());
    }
}
