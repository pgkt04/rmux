use super::wire::{Frame, KINDS, TEXT_KINDS};
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};

pub const NODE_LIMIT: usize = 200_000;
pub const DEPTH_LIMIT: usize = 64;
pub const CSS_LIMIT: usize = 256 * 1024;
pub const DOCUMENT_BUDGET: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug)]
struct Node {
    id: String,
    kind: String,
    props: Map<String, Value>,
    parent: Option<String>,
    children: Vec<String>,
    ages: HashMap<String, u64>,
    created_at_ms: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyError {
    pub s: u64,
    pub op: usize,
    pub msg: String,
}
#[derive(Debug)]
pub struct ApplyResult {
    pub accepted_ops: Vec<Value>,
    pub original_indices: Vec<usize>,
    pub errors: Vec<ApplyError>,
    pub revision: u64,
}
#[derive(Debug)]
pub struct TspDocument {
    pub surface: String,
    nodes: HashMap<String, Node>,
    pub settled: BTreeSet<String>,
    pub focus: Option<String>,
    pub suspended: bool,
    pub revision: u64,
    pub palette: Option<Value>,
    pub sheets: Vec<(String, String)>,
    kinds: BTreeSet<String>,
}
impl TspDocument {
    pub fn new(surface: impl Into<String>) -> Self {
        let surface = surface.into();
        let root = Node {
            id: surface.clone(),
            kind: "col".into(),
            props: Map::new(),
            parent: None,
            children: vec![],
            ages: HashMap::new(),
            created_at_ms: 0,
        };
        Self {
            nodes: HashMap::from([(surface.clone(), root)]),
            surface,
            settled: BTreeSet::new(),
            focus: None,
            suspended: false,
            revision: 0,
            palette: None,
            sheets: vec![],
            kinds: KINDS.iter().map(|s| (*s).into()).collect(),
        }
    }
    pub fn set_kinds(&mut self, kinds: BTreeSet<String>) {
        self.kinds = kinds;
    }
    pub fn size(&self) -> usize {
        self.nodes.len()
    }
    pub fn has(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }
    fn node(&self, id: &str) -> Result<&Node, String> {
        self.nodes.get(id).ok_or_else(|| format!("unknown id {id}"))
    }
    fn depth(&self, id: &str) -> usize {
        let mut d = 0;
        let mut at = Some(id);
        while let Some(i) = at {
            d += 1;
            at = self.nodes[i].parent.as_deref();
        }
        d
    }
    fn subtree_depth(&self, id: &str) -> usize {
        1 + self.nodes[id]
            .children
            .iter()
            .map(|c| self.subtree_depth(c))
            .max()
            .unwrap_or(0)
    }
    fn unsettle(&mut self, id: &str) {
        let mut at = Some(id.to_owned());
        while let Some(i) = at {
            self.settled.remove(&i);
            at = self.nodes.get(&i).and_then(|n| n.parent.clone());
        }
    }
    fn position(&self, parent: &str, before: &Value) -> Result<usize, String> {
        let p = self.node(parent)?;
        if before.is_null() {
            return Ok(p.children.len());
        }
        let id = before.as_str().ok_or("before must be string or null")?;
        p.children
            .iter()
            .position(|i| i == id)
            .ok_or_else(|| format!("{id} is not a child of {parent}"))
    }
    fn build(
        &self,
        v: &Value,
        parent: &str,
        depth: usize,
        out: &mut Vec<Node>,
        seen: &mut BTreeSet<String>,
        now: u64,
    ) -> Result<(), String> {
        if depth > DEPTH_LIMIT || self.size() + out.len() + 1 > NODE_LIMIT {
            return Err("node or depth limit".into());
        }
        let o = v.as_object().ok_or("node must be object")?;
        let id = o
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("node without id")?;
        let k = o
            .get("k")
            .and_then(Value::as_str)
            .ok_or("node without kind")?;
        if !self.kinds.contains(k) {
            return Err(format!("unknown kind {k}"));
        }
        if self.has(id) || !seen.insert(id.into()) {
            return Err(format!("duplicate id {id}"));
        }
        let mut props = match o.get("p") {
            None => Map::new(),
            Some(p) => p.as_object().ok_or("props must be object")?.clone(),
        };
        props.retain(|_, v| !v.is_null());
        validate_props(&props)?;
        let children = match o.get("c") {
            None => &[][..],
            Some(c) => c.as_array().ok_or("children must be array")?.as_slice(),
        };
        let mut n = Node {
            id: id.into(),
            kind: k.into(),
            props,
            parent: Some(parent.into()),
            children: vec![],
            ages: HashMap::new(),
            created_at_ms: now,
        };
        refresh_ages(&mut n, now);
        for c in children {
            n.children.push(
                c.get("id")
                    .and_then(Value::as_str)
                    .ok_or("node without id")?
                    .into(),
            );
        }
        out.push(n);
        for c in children {
            self.build(c, id, depth + 1, out, seen, now)?;
        }
        Ok(())
    }
    fn remove(&mut self, id: &str) {
        if let Some(n) = self.nodes.remove(id) {
            for c in n.children {
                self.remove(&c);
            }
            self.settled.remove(id);
            if self.focus.as_deref() == Some(id) {
                self.focus = None;
            }
        }
    }
    pub fn close(&mut self) {
        for id in ["dock", "layer"] {
            if self.nodes.get(id).and_then(|n| n.parent.as_deref()) == Some(&self.surface) {
                self.nodes
                    .get_mut(&self.surface)
                    .unwrap()
                    .children
                    .retain(|c| c != id);
                self.remove(id);
            }
        }
        self.focus = None;
    }
    pub fn apply_frame(&mut self, frame: &Frame, now_ms: u64) -> Result<ApplyResult, String> {
        if frame.sf != self.surface {
            return Err("unknown surface".into());
        }
        let mut r = ApplyResult {
            accepted_ops: vec![],
            original_indices: vec![],
            errors: vec![],
            revision: self.revision,
        };
        for (op, v) in frame.ops.iter().enumerate() {
            match self.apply(v, now_ms) {
                Ok(()) => {
                    r.accepted_ops.push(v.clone());
                    r.original_indices.push(op);
                }
                Err(msg) => r.errors.push(ApplyError {
                    s: frame.s,
                    op,
                    msg,
                }),
            }
        }
        self.revision += 1;
        r.revision = self.revision;
        Ok(r)
    }
    fn apply(&mut self, v: &Value, now: u64) -> Result<(), String> {
        let a = v.as_array().ok_or("op must be array")?;
        let verb = a.first().and_then(Value::as_str).ok_or("missing op")?;
        let arity = match verb {
            "add" | "splice" => 5,
            "text" | "move" => 4,
            "set" | "reveal" | "scroll" => 3,
            "del" | "settle" | "focus" => 2,
            "suspend" | "resume" => 1,
            _ => return Err(format!("unknown op {verb}")),
        };
        if a.len() != arity {
            return Err("invalid op arity".into());
        }
        if verb == "suspend" || verb == "resume" {
            self.suspended = verb == "suspend";
            return Ok(());
        }
        if verb == "focus" && a[1].is_null() {
            self.focus = None;
            return Ok(());
        }
        let id = a[1].as_str().ok_or("id must be string")?;
        if verb == "add" {
            let p = a[2].as_str().ok_or("parent must be string")?;
            let pos = self.position(p, &a[3])?;
            if a[4].get("id").and_then(Value::as_str) != Some(id) {
                return Err("add id mismatch".into());
            }
            let mut staged = vec![];
            self.build(
                &a[4],
                p,
                self.depth(p) + 1,
                &mut staged,
                &mut BTreeSet::new(),
                now,
            )?;
            self.nodes
                .get_mut(p)
                .unwrap()
                .children
                .insert(pos, id.into());
            for n in staged {
                self.nodes.insert(n.id.clone(), n);
            }
            self.unsettle(p);
            return Ok(());
        }
        self.node(id)?;
        match verb {
            "set" => {
                let p = a[2].as_object().ok_or("set props must be object")?;
                validate_props(p)?;
                let n = self.nodes.get_mut(id).unwrap();
                let unchanged: HashMap<String, u64> = age_paths()
                    .into_iter()
                    .filter_map(|path| {
                        let key = path.split('.').next().unwrap();
                        if !p.contains_key(key) {
                            return None;
                        }
                        let mut old = n.props.get(key)?;
                        let mut new = p.get(key)?;
                        for part in path.split('.').skip(1) {
                            old = old.get(part)?;
                            new = new.get(part)?;
                        }
                        (old == new)
                            .then(|| n.ages.get(path).map(|base| (path.to_owned(), *base)))
                            .flatten()
                    })
                    .collect();
                for (k, v) in p {
                    if v.is_null() {
                        n.props.remove(k);
                    } else {
                        n.props.insert(k.clone(), v.clone());
                    }
                    for path in age_paths() {
                        if path.split('.').next() == Some(k.as_str()) {
                            n.ages.remove(path);
                        }
                    }
                }
                refresh_ages(n, now);
                n.ages.extend(unchanged);
                self.unsettle(id);
            }
            "text" | "splice" => {
                let n = self.node(id)?;
                if !TEXT_KINDS.contains(&n.kind.as_str()) {
                    return Err(format!("{id} ({}) has no primary text", n.kind));
                }
                let old = n.props.get("text").and_then(Value::as_str).unwrap_or("");
                let text = a[arity - 1].as_str().ok_or("text must be string")?;
                let next = if verb == "text" {
                    match a[2].as_str() {
                        Some("append") => format!("{old}{text}"),
                        Some("replace") => text.into(),
                        _ => return Err("unknown text mode".into()),
                    }
                } else {
                    let at = a[2].as_u64().ok_or("invalid splice offset")?;
                    let del = a[3].as_u64().ok_or("invalid splice length")?;
                    let end = at.checked_add(del).ok_or("splice overflow")?;
                    let start = utf16_byte(old, at)?;
                    let end = utf16_byte(old, end)?;
                    format!("{}{text}{}", &old[..start], &old[end..])
                };
                self.nodes
                    .get_mut(id)
                    .unwrap()
                    .props
                    .insert("text".into(), next.into());
                self.unsettle(id);
            }
            "move" => {
                if id == self.surface {
                    return Err("cannot move the root".into());
                }
                let p = a[2].as_str().ok_or("parent must be string")?;
                self.position(p, &a[3])?;
                if a[3].as_str() == Some(id) {
                    return Err("move before itself".into());
                }
                let mut at = Some(p);
                while let Some(i) = at {
                    if i == id {
                        return Err("move cycle".into());
                    }
                    at = self.node(i)?.parent.as_deref();
                }
                if self.depth(p) + self.subtree_depth(id) > DEPTH_LIMIT {
                    return Err("depth limit".into());
                }
                let old = self.nodes[id].parent.clone().unwrap();
                self.nodes
                    .get_mut(&old)
                    .unwrap()
                    .children
                    .retain(|c| c != id);
                let pos = self.position(p, &a[3])?;
                self.nodes
                    .get_mut(p)
                    .unwrap()
                    .children
                    .insert(pos, id.into());
                self.nodes.get_mut(id).unwrap().parent = Some(p.into());
                self.unsettle(&old);
                self.unsettle(id);
            }
            "del" => {
                if id == self.surface {
                    return Err("cannot delete the root".into());
                }
                let p = self.nodes[id].parent.clone().unwrap();
                self.nodes.get_mut(&p).unwrap().children.retain(|c| c != id);
                self.remove(id);
                self.unsettle(&p);
            }
            "settle" => {
                self.settled.insert(id.into());
            }
            "focus" => self.focus = Some(id.into()),
            "reveal" => {
                if !matches!(a[2].as_str(), Some("start" | "end" | "nearest")) {
                    return Err("invalid reveal position".into());
                }
            }
            "scroll" => {
                if !matches!(
                    a[2].as_str(),
                    Some("line-up" | "line-down" | "page-up" | "page-down" | "start" | "end")
                ) {
                    return Err("invalid scroll direction".into());
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    pub fn snapshot(&self, now_ms: u64) -> Value {
        self.export(&self.surface, now_ms)
    }
    pub fn get(&self, id: &str, now_ms: u64) -> Option<Value> {
        self.has(id).then(|| self.export(id, now_ms))
    }
    fn export(&self, id: &str, now: u64) -> Value {
        let n = &self.nodes[id];
        let mut out = json!({"id":n.id,"k":n.kind});
        let mut props = n.props.clone();
        for (path, base) in &n.ages {
            if let Some(v) = path_mut(&mut props, path) {
                if let Some(age) = v.as_f64() {
                    *v = json!(age + now.saturating_sub(*base) as f64);
                }
            }
        }
        if !props.is_empty() {
            out["p"] = props.into();
        }
        if !n.children.is_empty() {
            out["c"] = n.children.iter().map(|c| self.export(c, now)).collect();
        }
        out
    }
    pub fn set_palette(&mut self, palette: Value) -> Result<(), String> {
        let p = palette.as_object().ok_or("palette must be object")?;
        for key in ["dark", "light"] {
            if let Some(v) = p.get(key) {
                let o = v.as_object().ok_or("palette variant must be object")?;
                if o.values().any(|v| !v.is_string()) {
                    return Err("palette color must be string".into());
                }
            }
        }
        self.palette = Some(palette);
        Ok(())
    }
    pub fn set_sheet(&mut self, name: &str, css: Option<&Value>) -> Result<(), String> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("invalid sheet name".into());
        }
        let css = match css {
            None | Some(Value::Null) => "",
            Some(v) => v.as_str().ok_or("css must be string")?,
        };
        let total: usize = self
            .sheets
            .iter()
            .filter(|(n, _)| n != name)
            .map(|(_, c)| c.len())
            .sum();
        if total + css.len() > CSS_LIMIT {
            return Err("CSS limit".into());
        }
        if css.is_empty() {
            self.sheets.retain(|(n, _)| n != name);
        } else if let Some((_, c)) = self.sheets.iter_mut().find(|(n, _)| n == name) {
            *c = css.into();
        } else {
            self.sheets.push((name.into(), css.into()));
        }
        Ok(())
    }
    pub fn estimated_bytes(&self) -> usize {
        self.nodes
            .values()
            .map(|n| {
                n.id.len()
                    + n.kind.len()
                    + serde_json::to_vec(&n.props).unwrap().len()
                    + 128
                    + n.children.len() * 32
            })
            .sum::<usize>()
            + self
                .sheets
                .iter()
                .map(|(n, c)| n.len() + c.len())
                .sum::<usize>()
            + self
                .palette
                .as_ref()
                .map_or(0, |p| serde_json::to_vec(p).unwrap().len())
    }
    pub fn settled_main_children(&self) -> Vec<String> {
        self.nodes
            .get("main")
            .filter(|n| n.parent.as_deref() == Some(self.surface.as_str()))
            .map(|n| {
                n.children
                    .iter()
                    .filter(|id| self.settled.contains(*id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn first_settled_main_child(&self) -> Option<&str> {
        let main = self.nodes.get("main")?;
        if main.parent.as_deref() != Some(self.surface.as_str()) {
            return None;
        }
        main.children
            .iter()
            .find(|id| self.settled.contains(*id))
            .map(String::as_str)
    }
    pub fn oldest_settled_main_child(&self) -> Option<(&str, u64)> {
        let main = self.nodes.get("main")?;
        if main.parent.as_deref() != Some(self.surface.as_str()) {
            return None;
        }
        main.children
            .iter()
            .filter(|id| self.settled.contains(*id))
            .map(|id| (id.as_str(), self.nodes[id].created_at_ms))
            .min_by_key(|(_, created)| *created)
    }
    pub fn evict_settled_main_child(&mut self, id: &str) -> Option<Vec<String>> {
        if !self.settled.contains(id)
            || self.nodes.get(id)?.parent.as_deref() != Some("main")
            || self.nodes.get("main")?.parent.as_deref() != Some(self.surface.as_str())
        {
            return None;
        }
        let mut removed = Vec::new();
        let mut pending = vec![id.to_owned()];
        while let Some(at) = pending.pop() {
            pending.extend(self.nodes[&at].children.iter().rev().cloned());
            removed.push(at);
        }
        self.nodes
            .get_mut("main")?
            .children
            .retain(|child| child != id);
        self.remove(id);
        self.unsettle("main");
        self.revision += 1;
        Some(removed)
    }
    pub fn blob_references(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for n in self.nodes.values() {
            collect_blobs(&Value::Object(n.props.clone()), &mut out);
        }
        out
    }
}
fn collect_blobs(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(o) => {
            for (k, v) in o {
                if matches!(k.as_str(), "blob" | "sha256") {
                    if let Some(s) = v.as_str() {
                        out.insert(s.to_ascii_lowercase());
                    }
                }
                collect_blobs(v, out);
            }
        }
        Value::Array(a) => {
            for v in a {
                collect_blobs(v, out);
            }
        }
        _ => {}
    }
}
fn validate_props(p: &Map<String, Value>) -> Result<(), String> {
    for k in ["text", "role", "title", "aria", "href"] {
        if let Some(v) = p.get(k) {
            if !v.is_null() && !v.is_string() {
                return Err(format!("{k} must be string"));
            }
        }
    }
    for k in ["hidden", "collapsed", "collapsible", "selected"] {
        if let Some(v) = p.get(k) {
            if !v.is_null() && !v.is_boolean() {
                return Err(format!("{k} must be boolean"));
            }
        }
    }
    Ok(())
}
fn age_paths() -> [&'static str; 5] {
    [
        "age",
        "stats.age",
        "tool.age",
        "retry.age",
        "stats.tool.age",
    ]
}
fn path_mut<'a>(p: &'a mut Map<String, Value>, path: &str) -> Option<&'a mut Value> {
    let mut parts = path.split('.');
    let mut v = p.get_mut(parts.next()?)?;
    for part in parts {
        v = v.as_object_mut()?.get_mut(part)?;
    }
    Some(v)
}
fn refresh_ages(n: &mut Node, now: u64) {
    for path in age_paths() {
        if path_mut(&mut n.props, path).is_some_and(|v| v.is_number()) {
            n.ages.entry(path.into()).or_insert(now);
        } else {
            n.ages.remove(path);
        }
    }
}
fn utf16_byte(s: &str, offset: u64) -> Result<usize, String> {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        if units == offset {
            return Ok(i);
        }
        units += c.len_utf16() as u64;
        if units > offset {
            return Err("splice splits surrogate pair".into());
        }
    }
    if units == offset {
        Ok(s.len())
    } else {
        Err("splice outside text".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_styles_and_age_bases() {
        let mut d = TspDocument::new("s");
        let mut node = json!({"id":"leaf","k":"text"});
        for i in 0..64 {
            node = json!({"id":format!("deep{i}"),"k":"col","c":[node]});
        }
        let r = d
            .apply_frame(
                &Frame {
                    sf: "s".into(),
                    s: 1,
                    ops: vec![json!(["add", "deep63", "s", null, node])],
                },
                0,
            )
            .unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(d.size(), 1);
        let f = Frame {
            sf: "s".into(),
            s: 2,
            ops: vec![
                json!(["add","timer","s",null,{"id":"timer","k":"agent","p":{"stats":{"age":10},"took":9}}]),
            ],
        };
        d.apply_frame(&f, 100).unwrap();
        d.apply_frame(
            &Frame {
                sf: "s".into(),
                s: 3,
                ops: vec![json!(["set","timer",{"stats":{"age":10,"other":true}}])],
            },
            200,
        )
        .unwrap();
        assert_eq!(d.get("timer", 300).unwrap()["p"]["stats"]["age"], 210.0);
        d.set_sheet("first", Some(&json!("a{}"))).unwrap();
        d.set_sheet("second", Some(&json!("b{}"))).unwrap();
        d.set_sheet("first", Some(&json!("c{}"))).unwrap();
        assert_eq!(d.sheets[0], ("first".into(), "c{}".into()));
        assert!(d.set_sheet("bad name", Some(&json!("x"))).is_err());
        assert!(
            d.set_sheet("big", Some(&json!("x".repeat(CSS_LIMIT))))
                .is_err()
        );
        assert_eq!(d.sheets.len(), 2);
        assert!(utf16_byte("A😀B", 2).is_err());
        assert_eq!(utf16_byte("é😀B", 3), Ok(6));
    }
    #[test]
    fn rollback_indices_and_utf16() {
        let mut d = TspDocument::new("s");
        let r = d.apply_frame(&Frame { sf: "s".into(), s: 99, ops: vec![
            json!(["add","main","s",null,{"id":"main","k":"col","c":[{"id":"t","k":"text","p":{"text":"A😀B"}}]}]),
            json!(["add","bad","s",null,{"id":"bad","k":"col","c":[{"id":"t","k":"text"}]}]),
            json!(["splice","t",2,0,"x"]),
            json!(["splice","t",1,2,"Z"]),
            json!(["move","main","t",null]),
            json!(["set","t",{"future":true,"text":null}]),
        ]}, 0).unwrap();
        assert_eq!(r.original_indices, vec![0, 3, 5]);
        assert_eq!(
            r.errors.iter().map(|e| e.op).collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
        assert!(!d.has("bad"));
        assert_eq!(d.get("t", 0).unwrap()["p"], json!({"future":true}));
    }
    #[test]
    fn reference_randomized_common_ops() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let reference = "/Users/j/fun/oh-my-pi/packages/tui/src/native/apply.ts";
        if !std::path::Path::new(reference).exists()
            || Command::new("bun").arg("--version").output().is_err()
        {
            eprintln!("SKIP: bun or omp reference applier unavailable");
            return;
        }
        let mut frames = vec![Frame {
            sf: "s".into(),
            s: 1,
            ops: vec![json!(["add","main","s",null,{"id":"main","k":"col"}])],
        }];
        let mut seed = 0x12345678u64;
        for i in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let id = format!("n{}", (seed >> 32) % 12);
            let op = match seed % 7 {
                0 => json!(["add",id,"main",null,{"id":id,"k":"text","p":{"text":"😀x"}}]),
                1 => json!(["text", id, "append", "é"]),
                2 => json!(["set",id,{"title":"test","future":null}]),
                3 => json!(["del", id]),
                4 => json!(["settle", id]),
                5 => json!(["move", id, "main", null]),
                _ => json!(["focus", id]),
            };
            frames.push(Frame {
                sf: "s".into(),
                s: i + 2,
                ops: vec![op],
            });
        }
        let mut child = Command::new("bun")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/tsp_reference.ts"
            ))
            .env("RMUX_OMP_APPLIER", reference)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&json!({"surface":"s","frames":frames})).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "reference process failed");
        let expected: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
        let mut d = TspDocument::new("s");
        for (frame, state) in frames.iter().zip(expected) {
            let result = d.apply_frame(frame, 0).unwrap();
            assert_eq!(
                d.snapshot(0),
                state["tree"],
                "seed 0x12345678 frame {}",
                frame.s
            );
            assert_eq!(
                json!(result.errors.iter().map(|e| e.op).collect::<Vec<_>>()),
                state["errors"]
            );
            assert_eq!(json!(d.focus), state["focus"]);
            assert_eq!(json!(d.settled), state["settled"]);
        }
    }
}
