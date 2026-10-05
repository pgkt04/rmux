// New TSP broker behavior; lifetime hooks in tmux screen.c/input.c @ 8f25579c.
// Protocol: https://docs.stencil.so/tern/protocol/surfaces.md

use super::document::{DOCUMENT_BUDGET, TspDocument};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Pane-local identities never repeat, even when a program reuses a wire id.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceMode {
    Inline,
    Screen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceBuffer {
    Main,
    Alternate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceState {
    Open,
    Suspended,
    Closed,
}

#[derive(Debug)]
pub struct Surface {
    pub id: SurfaceId,
    pub wire_id: String,
    pub document: TspDocument,
    pub open: Value,
    pub listen: bool,
    pub mode: SurfaceMode,
    pub buffer: SurfaceBuffer,
    pub anchor: Option<u64>,
    closed_at: Option<u64>,
}

impl Surface {
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    pub fn state(&self) -> SurfaceState {
        if !self.is_open() {
            SurfaceState::Closed
        } else if self.document.suspended {
            SurfaceState::Suspended
        } else {
            SurfaceState::Open
        }
    }

    pub fn listens(&self) -> bool {
        self.is_open() && self.listen
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalReason {
    Close,
    Replacement,
    Anchor,
    Alternate,
    BufferClear,
    Prompt,
    Reset,
    Respawn,
    ProcessExit,
    Retention,
}

#[derive(Debug)]
pub struct SurfaceRemoval {
    pub id: SurfaceId,
    pub wire_id: String,
    pub anchor: Option<u64>,
    pub listen: bool,
    pub reason: RemovalReason,
}

#[derive(Debug)]
pub struct NodeEviction {
    pub surface: SurfaceId,
    pub wire_id: String,
    /// Preorder ids, including the removed top-level node and its descendants.
    pub ids: Vec<String>,
    pub listen: bool,
}

#[derive(Debug)]
pub struct Gone {
    pub surface: Option<SurfaceId>,
    pub wire_id: Option<String>,
    pub ids: Vec<String>,
    pub listen: bool,
}

impl Gone {
    /// The caller must also apply its pane-program listener registration gate.
    pub fn json(&self) -> Value {
        let mut event = json!({"ev":"gone","ids":self.ids});
        if let Some(id) = &self.wire_id {
            event["sf"] = json!(id);
        }
        event
    }
}

#[derive(Debug, Default)]
pub struct SurfaceChange {
    pub opened: Option<SurfaceId>,
    pub closed: Vec<SurfaceId>,
    pub removed: Vec<SurfaceRemoval>,
    pub evicted: Vec<NodeEviction>,
    /// Includes non-listening losses so callers can invalidate projections too.
    pub gone: Vec<Gone>,
}

impl SurfaceChange {
    pub fn append(&mut self, mut other: Self) {
        if other.opened.is_some() {
            self.opened = other.opened;
        }
        self.closed.append(&mut other.closed);
        self.removed.append(&mut other.removed);
        self.evicted.append(&mut other.evicted);
        self.gone.append(&mut other.gone);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceError {
    InvalidOpen,
    InvalidId,
    InvalidMode,
    InvalidMetadata,
    InlineOnAlternate,
    MissingAnchor,
    DuplicateAnchor,
    IdentityExhausted,
    UnknownSurface,
    InvalidEviction,
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidOpen => "surface open must be an object",
            Self::InvalidId => "surface id must be a nonempty string",
            Self::InvalidMode => "unsupported surface mode",
            Self::InvalidMetadata => "invalid surface open metadata",
            Self::InlineOnAlternate => "inline surface cannot open on alternate screen",
            Self::MissingAnchor => "inline surface requires an anchor",
            Self::DuplicateAnchor => "surface anchor is already owned",
            Self::IdentityExhausted => "surface identity space exhausted",
            Self::UnknownSurface => "unknown surface",
            Self::InvalidEviction => "node is not a settled top-level main child",
        };
        f.write_str(message)
    }
}

impl std::error::Error for SurfaceError {}

struct OpenRequest<'a> {
    object: &'a Map<String, Value>,
    wire_id: &'a str,
    mode: SurfaceMode,
    adopt: bool,
    listen: bool,
}

impl<'a> OpenRequest<'a> {
    fn parse(value: &'a Value) -> Result<Self, SurfaceError> {
        let object = value.as_object().ok_or(SurfaceError::InvalidOpen)?;
        let wire_id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or(SurfaceError::InvalidId)?;
        let boolean = |name: &str, default| match object.get(name) {
            None => Ok(default),
            Some(value) => value.as_bool().ok_or(SurfaceError::InvalidMetadata),
        };
        let adopt = boolean("adopt", false)?;
        let listen = boolean("listen", true)?;
        for name in ["title", "role"] {
            if object.get(name).is_some_and(|value| !value.is_string()) {
                return Err(SurfaceError::InvalidMetadata);
            }
        }
        let mode = if adopt {
            SurfaceMode::Inline
        } else {
            match object.get("mode") {
                None => SurfaceMode::Inline,
                Some(Value::String(mode)) if mode == "inline" => SurfaceMode::Inline,
                Some(Value::String(mode)) if mode == "screen" => SurfaceMode::Screen,
                _ => return Err(SurfaceError::InvalidMode),
            }
        };
        Ok(Self {
            object,
            wire_id,
            mode,
            adopt,
            listen,
        })
    }
}

/// Owns each document exactly once. Projections borrow it through SurfaceId;
/// closing/adopting never moves the document to a second retained arena.
#[derive(Debug)]
pub struct SurfaceStore {
    surfaces: BTreeMap<SurfaceId, Surface>,
    next_id: u64,
    close_serial: u64,
    buffer: SurfaceBuffer,
    budget: usize,
    exited: bool,
}

impl Default for SurfaceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceStore {
    pub fn new() -> Self {
        Self::with_budget(DOCUMENT_BUDGET)
    }

    pub fn with_budget(budget: usize) -> Self {
        Self {
            surfaces: BTreeMap::new(),
            next_id: 1,
            close_serial: 0,
            buffer: SurfaceBuffer::Main,
            budget,
            exited: false,
        }
    }

    pub fn buffer(&self) -> SurfaceBuffer {
        self.buffer
    }

    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Surface> {
        self.surfaces.values()
    }

    pub fn get(&self, id: SurfaceId) -> Option<&Surface> {
        self.surfaces.get(&id)
    }

    /// Mutations through this borrow must be followed by retain() at the frame,
    /// palette or stylesheet boundary, before dependent projection output.
    pub fn get_mut(&mut self, id: SurfaceId) -> Option<&mut Surface> {
        self.surfaces.get_mut(&id)
    }

    pub fn find_open(&self, wire_id: &str) -> Option<SurfaceId> {
        self.surfaces
            .values()
            .filter(|surface| surface.is_open() && surface.wire_id == wire_id)
            .min_by_key(|surface| (surface.buffer != self.buffer, surface.id))
            .map(|surface| surface.id)
    }

    pub fn find(&self, wire_id: &str) -> Option<SurfaceId> {
        self.find_open(wire_id).or_else(|| {
            self.surfaces
                .values()
                .filter(|surface| !surface.is_open() && surface.wire_id == wire_id)
                .max_by_key(|surface| surface.closed_at)
                .map(|surface| surface.id)
        })
    }

    pub fn selected(&self) -> Option<&Surface> {
        let visible = |surface: &&Surface| {
            surface.is_open() && surface.buffer == self.buffer && !surface.document.suspended
        };
        let screen = self
            .surfaces
            .values()
            .filter(visible)
            .find(|surface| surface.mode == SurfaceMode::Screen);
        if screen.is_some() {
            return screen;
        }
        // A suspended screen still covers the inline surface: hand the pane to
        // the real grid rather than projecting the underlying inline document.
        if self.surfaces.values().any(|surface| {
            surface.is_open()
                && surface.buffer == self.buffer
                && surface.mode == SurfaceMode::Screen
        }) {
            return None;
        }
        if self.buffer == SurfaceBuffer::Alternate {
            return None;
        }
        self.surfaces
            .values()
            .filter(visible)
            .find(|surface| surface.mode == SurfaceMode::Inline)
            .or_else(|| {
                if self.exited {
                    self.surfaces
                        .values()
                        .filter(|surface| surface.mode == SurfaceMode::Inline)
                        .max_by_key(|surface| surface.closed_at)
                } else {
                    None
                }
            })
    }

    pub fn selected_id(&self) -> Option<SurfaceId> {
        self.selected().map(|surface| surface.id)
    }

    pub fn needs_anchor(&self, value: &Value) -> Result<bool, SurfaceError> {
        let request = OpenRequest::parse(value)?;
        if request.mode == SurfaceMode::Inline && self.buffer == SurfaceBuffer::Alternate {
            return Err(SurfaceError::InlineOnAlternate);
        }
        Ok(request.mode == SurfaceMode::Inline && !request.adopt)
    }

    pub fn open(
        &mut self,
        value: &Value,
        anchor: Option<u64>,
    ) -> Result<SurfaceChange, SurfaceError> {
        let request = OpenRequest::parse(value)?;
        if request.mode == SurfaceMode::Inline && self.buffer == SurfaceBuffer::Alternate {
            return Err(SurfaceError::InlineOnAlternate);
        }
        let mut change = SurfaceChange::default();
        let id = if request.adopt {
            if self.surfaces.values().any(|surface| {
                surface.is_open()
                    && surface.mode == SurfaceMode::Inline
                    && surface.wire_id == request.wire_id
            }) {
                return Ok(change);
            }
            let candidate = self
                .surfaces
                .values()
                .filter(|surface| {
                    !surface.is_open()
                        && surface.mode == SurfaceMode::Inline
                        && surface.wire_id == request.wire_id
                })
                .max_by_key(|surface| surface.closed_at)
                .map(|surface| surface.id);
            let Some(candidate) = candidate else {
                change.gone.push(Gone {
                    surface: None,
                    wire_id: None,
                    ids: vec![request.wire_id.to_owned()],
                    listen: request.listen,
                });
                return Ok(change);
            };
            candidate
        } else {
            if request.mode == SurfaceMode::Inline {
                let anchor = anchor.ok_or(SurfaceError::MissingAnchor)?;
                if self
                    .surfaces
                    .values()
                    .any(|surface| surface.anchor == Some(anchor))
                {
                    return Err(SurfaceError::DuplicateAnchor);
                }
            }
            self.next_id
                .checked_add(1)
                .ok_or(SurfaceError::IdentityExhausted)?;
            SurfaceId(self.next_id)
        };
        let replacements: Vec<_> = self
            .surfaces
            .values()
            .filter(|surface| {
                surface.is_open()
                    && surface.buffer == self.buffer
                    && (surface.mode == request.mode
                        || request.mode == SurfaceMode::Inline
                            && surface.mode == SurfaceMode::Screen)
            })
            .map(|surface| surface.id)
            .collect();
        for replaced in replacements {
            self.close_id(replaced, true, RemovalReason::Replacement, &mut change);
        }
        if request.adopt {
            let surface = self
                .surfaces
                .get_mut(&id)
                .expect("adoption candidate exists");
            let metadata = surface
                .open
                .as_object_mut()
                .expect("validated open metadata");
            metadata.extend(request.object.clone());
            metadata.insert("mode".into(), json!("inline"));
            metadata.insert("listen".into(), json!(request.listen));
            surface.listen = request.listen;
            surface.closed_at = None;
            surface.document.suspended = false;
        } else {
            self.next_id += 1;
            self.surfaces.insert(
                id,
                Surface {
                    id,
                    wire_id: request.wire_id.to_owned(),
                    document: TspDocument::new(request.wire_id),
                    open: value.clone(),
                    listen: request.listen,
                    mode: request.mode,
                    buffer: self.buffer,
                    anchor: if request.mode == SurfaceMode::Inline {
                        anchor
                    } else {
                        None
                    },
                    closed_at: None,
                },
            );
        }
        self.exited = false;
        change.opened = Some(id);
        change.append(self.retain());
        Ok(change)
    }

    pub fn close(&mut self, wire_id: &str, keep: bool) -> SurfaceChange {
        let mut change = SurfaceChange::default();
        if let Some(id) = self.find_open(wire_id) {
            self.close_id(id, keep, RemovalReason::Close, &mut change);
            change.append(self.retain());
        }
        change
    }

    fn close_id(
        &mut self,
        id: SurfaceId,
        keep: bool,
        reason: RemovalReason,
        change: &mut SurfaceChange,
    ) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        if !surface.is_open() {
            return;
        }
        change.closed.push(id);
        if surface.mode == SurfaceMode::Inline && keep {
            self.close_serial += 1;
            surface.closed_at = Some(self.close_serial);
            surface.document.close();
            surface.document.suspended = false;
        } else {
            self.remove_id(id, reason, false, change);
        }
    }

    fn remove_id(
        &mut self,
        id: SurfaceId,
        reason: RemovalReason,
        notify: bool,
        change: &mut SurfaceChange,
    ) {
        let Some(surface) = self.surfaces.remove(&id) else {
            return;
        };
        if notify {
            change.gone.push(Gone {
                surface: Some(id),
                wire_id: Some(surface.wire_id.clone()),
                ids: vec![surface.wire_id.clone()],
                listen: surface.listen,
            });
        }
        change.removed.push(SurfaceRemoval {
            id,
            wire_id: surface.wire_id,
            anchor: surface.anchor,
            listen: surface.listen,
            reason,
        });
    }

    pub fn suspend(&mut self, id: SurfaceId) -> Result<(), SurfaceError> {
        let surface = self
            .surfaces
            .get_mut(&id)
            .filter(|surface| surface.is_open())
            .ok_or(SurfaceError::UnknownSurface)?;
        surface.document.suspended = true;
        Ok(())
    }

    pub fn resume(&mut self, id: SurfaceId) -> Result<(), SurfaceError> {
        let surface = self
            .surfaces
            .get_mut(&id)
            .filter(|surface| surface.is_open())
            .ok_or(SurfaceError::UnknownSurface)?;
        surface.document.suspended = false;
        Ok(())
    }

    pub fn remove_anchor(&mut self, anchor: u64) -> SurfaceChange {
        let mut change = SurfaceChange::default();
        let id = self
            .surfaces
            .values()
            .find(|surface| surface.anchor == Some(anchor))
            .map(|surface| surface.id);
        if let Some(id) = id {
            let notify = self.surfaces[&id].is_open();
            self.remove_id(id, RemovalReason::Anchor, notify, &mut change);
        }
        change
    }

    pub fn alternate_enter(&mut self) -> SurfaceChange {
        self.buffer = SurfaceBuffer::Alternate;
        SurfaceChange::default()
    }

    pub fn alternate_exit(&mut self) -> SurfaceChange {
        let change = self.remove_buffer(SurfaceBuffer::Alternate, RemovalReason::Alternate);
        self.buffer = SurfaceBuffer::Main;
        change
    }

    pub fn alternate_clear(&mut self) -> SurfaceChange {
        self.remove_buffer(SurfaceBuffer::Alternate, RemovalReason::Alternate)
    }

    pub fn clear_buffer(&mut self, buffer: SurfaceBuffer) -> SurfaceChange {
        self.remove_buffer(buffer, RemovalReason::BufferClear)
    }

    fn remove_buffer(&mut self, buffer: SurfaceBuffer, reason: RemovalReason) -> SurfaceChange {
        let ids: Vec<_> = self
            .surfaces
            .values()
            .filter(|surface| surface.buffer == buffer)
            .map(|surface| surface.id)
            .collect();
        let mut change = SurfaceChange::default();
        for id in ids {
            let notify = self.surfaces[&id].is_open();
            self.remove_id(id, reason, notify, &mut change);
        }
        change
    }

    fn remove_all(&mut self, reason: RemovalReason) -> SurfaceChange {
        let mut change = SurfaceChange::default();
        while let Some(id) = self.surfaces.keys().next().copied() {
            self.remove_id(id, reason, false, &mut change);
        }
        self.exited = false;
        change
    }

    pub fn prompt(&mut self) -> SurfaceChange {
        self.remove_all(RemovalReason::Prompt)
    }

    pub fn reset(&mut self) -> SurfaceChange {
        let change = self.remove_all(RemovalReason::Reset);
        self.buffer = SurfaceBuffer::Main;
        change
    }

    pub fn respawn(&mut self) -> SurfaceChange {
        let change = self.remove_all(RemovalReason::Respawn);
        self.buffer = SurfaceBuffer::Main;
        change
    }

    pub fn process_exit(&mut self) -> SurfaceChange {
        let mut change = SurfaceChange::default();
        let ids: Vec<_> = self
            .surfaces
            .values()
            .filter(|surface| surface.is_open())
            .map(|surface| surface.id)
            .collect();
        for id in ids {
            self.close_id(id, true, RemovalReason::ProcessExit, &mut change);
        }
        for surface in self.surfaces.values_mut() {
            surface.listen = false;
            surface.open["listen"] = json!(false);
        }
        self.buffer = SurfaceBuffer::Main;
        self.exited = true;
        change.append(self.retain());
        change
    }

    pub fn estimated_bytes(&self) -> usize {
        self.surfaces
            .values()
            .map(|surface| surface.document.estimated_bytes())
            .sum()
    }

    /// Soft budget: never discard unsettled live state to satisfy it.
    pub fn retain(&mut self) -> SurfaceChange {
        let mut change = SurfaceChange::default();
        let mut bytes = self.estimated_bytes();
        while bytes > self.budget {
            let closed = self
                .surfaces
                .values()
                .filter(|surface| !surface.is_open())
                .min_by_key(|surface| surface.closed_at)
                .map(|surface| surface.id);
            if let Some(id) = closed {
                bytes = bytes.saturating_sub(self.surfaces[&id].document.estimated_bytes());
                self.remove_id(id, RemovalReason::Retention, true, &mut change);
                continue;
            }
            let candidate = self
                .surfaces
                .values()
                .filter_map(|surface| {
                    surface
                        .document
                        .oldest_settled_main_child()
                        .map(|(node, created)| (surface.id, node, created))
                })
                .min_by_key(|(id, _, created)| (*created, *id))
                .map(|(id, node, _)| (id, node.to_owned()));
            let Some((id, node)) = candidate else { break };
            let before = self.surfaces[&id].document.estimated_bytes();
            self.evict_node(id, &node, &mut change);
            let after = self.surfaces[&id].document.estimated_bytes();
            bytes = bytes.saturating_sub(before.saturating_sub(after));
        }
        change
    }

    /// An outer view may canonically evict only these same retention candidates.
    /// Validate the entire request before removing anything.
    pub fn evict_nodes(
        &mut self,
        id: SurfaceId,
        nodes: &[String],
    ) -> Result<SurfaceChange, SurfaceError> {
        let surface = self.surfaces.get(&id).ok_or(SurfaceError::UnknownSurface)?;
        if !surface.is_open() {
            return Err(SurfaceError::InvalidEviction);
        }
        let candidates = surface.document.settled_main_children();
        if nodes.iter().any(|node| !candidates.contains(node)) {
            return Err(SurfaceError::InvalidEviction);
        }
        let mut change = SurfaceChange::default();
        for (index, node) in nodes.iter().enumerate() {
            if !nodes[..index].contains(node) {
                self.evict_node(id, node, &mut change);
            }
        }
        Ok(change)
    }

    fn evict_node(&mut self, id: SurfaceId, node: &str, change: &mut SurfaceChange) {
        let surface = self
            .surfaces
            .get_mut(&id)
            .expect("retention surface exists");
        if let Some(ids) = surface.document.evict_settled_main_child(node) {
            change.gone.push(Gone {
                surface: Some(id),
                wire_id: Some(surface.wire_id.clone()),
                ids: vec![node.to_owned()],
                listen: surface.listen,
            });
            change.evicted.push(NodeEviction {
                surface: id,
                wire_id: surface.wire_id.clone(),
                ids,
                listen: surface.listen,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsp::wire::Frame;

    fn open(store: &mut SurfaceStore, id: &str, mode: &str, anchor: u64) -> SurfaceId {
        store
            .open(&json!({"id":id,"mode":mode}), Some(anchor))
            .unwrap()
            .opened
            .unwrap()
    }

    fn frame(store: &mut SurfaceStore, id: SurfaceId, ops: Vec<Value>) {
        let surface = store.get_mut(id).unwrap();
        let result = surface
            .document
            .apply_frame(
                &Frame {
                    sf: surface.wire_id.clone(),
                    s: 17,
                    ops,
                },
                0,
            )
            .unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    fn regions(store: &mut SurfaceStore, id: SurfaceId) {
        let root = store.get(id).unwrap().wire_id.clone();
        frame(
            store,
            id,
            vec![
                json!(["add","main",root,null,{"id":"main","k":"col","c":[
                    {"id":"entry","k":"text","p":{"text":"transcript"}}
                ]}]),
                json!(["add","dock",root,null,{"id":"dock","k":"col","c":[
                    {"id":"editor","k":"editor","p":{"text":"draft"}}
                ]}]),
                json!(["add","layer",root,null,{"id":"layer","k":"col","c":[
                    {"id":"dialog","k":"text","p":{"text":"dialog"}}
                ]}]),
                json!(["focus", "editor"]),
            ],
        );
    }

    #[test]
    fn close_keep_listen_mode_matrix() {
        for mode in ["inline", "screen"] {
            for keep in [false, true] {
                for listen in [false, true] {
                    let mut store = SurfaceStore::new();
                    let id = store
                        .open(
                            &json!({"id":"surface","mode":mode,"listen":listen}),
                            Some(91),
                        )
                        .unwrap()
                        .opened
                        .unwrap();
                    regions(&mut store, id);
                    let change = store.close("surface", keep);
                    assert_eq!(change.closed, [id]);
                    assert!(change.gone.is_empty());
                    assert!(store.find_open("surface").is_none());
                    assert!(store.selected().is_none());
                    if mode == "inline" && keep {
                        let surface = store.get(id).unwrap();
                        assert_eq!(surface.state(), SurfaceState::Closed);
                        assert_eq!(surface.listen, listen);
                        assert!(!surface.listens());
                        assert!(surface.document.has("entry"));
                        assert!(!surface.document.has("dock"));
                        assert!(!surface.document.has("editor"));
                        assert!(!surface.document.has("layer"));
                        assert!(!surface.document.has("dialog"));
                        assert_eq!(surface.document.focus, None);
                        assert_eq!(surface.anchor, Some(91));
                        assert_eq!(store.find("surface"), Some(id));
                    } else {
                        assert!(store.get(id).is_none());
                        assert_eq!(change.removed.len(), 1);
                        assert_eq!(change.removed[0].id, id);
                    }
                }
            }
        }
    }

    #[test]
    fn screen_overlay_preserves_inline_document_focus_and_revision() {
        let mut store = SurfaceStore::new();
        let inline = open(&mut store, "inline", "inline", 1);
        regions(&mut store, inline);
        let snapshot = store.get(inline).unwrap().document.snapshot(0);
        let revision = store.get(inline).unwrap().document.revision;
        let screen = open(&mut store, "screen", "screen", 2);
        assert_eq!(store.selected_id(), Some(screen));
        assert_eq!(store.get(inline).unwrap().document.snapshot(0), snapshot);
        assert_eq!(store.get(inline).unwrap().document.revision, revision);
        assert_eq!(
            store.get(inline).unwrap().document.focus.as_deref(),
            Some("editor")
        );
        frame(
            &mut store,
            inline,
            vec![json!(["text", "entry", "append", " streamed"])],
        );
        store.close("screen", true);
        assert_eq!(store.selected_id(), Some(inline));
        assert!(store.get(screen).is_none());
        assert_eq!(
            store.get(inline).unwrap().document.get("entry", 0).unwrap()["p"]["text"],
            "transcript streamed"
        );
        assert!(store.get(inline).unwrap().document.has("dock"));
        assert!(store.get(inline).unwrap().document.has("layer"));
    }

    #[test]
    fn replacement_retains_inline_but_discards_screen() {
        let mut store = SurfaceStore::new();
        let old = open(&mut store, "old", "inline", 1);
        regions(&mut store, old);
        let screen = open(&mut store, "cover", "screen", 2);
        let replacement = store.open(&json!({"id":"new"}), Some(3)).unwrap();
        let new = replacement.opened.unwrap();
        assert_eq!(replacement.closed, [old, screen]);
        assert_eq!(replacement.removed[0].id, screen);
        assert_eq!(replacement.removed[0].reason, RemovalReason::Replacement);
        assert!(replacement.gone.is_empty());
        assert_eq!(store.get(old).unwrap().state(), SurfaceState::Closed);
        assert!(store.get(old).unwrap().document.has("entry"));
        assert!(!store.get(old).unwrap().document.has("dock"));
        assert_eq!(store.selected_id(), Some(new));
        let first = open(&mut store, "cover", "screen", 4);
        let next = open(&mut store, "cover", "screen", 5);
        assert_ne!(first, next);
        assert!(store.get(first).is_none());
        assert_eq!(store.find_open("cover"), Some(next));
    }

    #[test]
    fn adopt_owns_same_document_and_anchor_and_replaces_metadata() {
        let mut store = SurfaceStore::new();
        let id = store
            .open(
                &json!({"id":"session","title":"before","role":"session"}),
                Some(83),
            )
            .unwrap()
            .opened
            .unwrap();
        regions(&mut store, id);
        let document = &store.get(id).unwrap().document as *const TspDocument;
        store
            .get_mut(id)
            .unwrap()
            .document
            .set_palette(json!({"dark":{"accent":"red"}}))
            .unwrap();
        store
            .get_mut(id)
            .unwrap()
            .document
            .set_sheet("base", Some(&json!(".x{color:red}")))
            .unwrap();
        store.close("session", true);
        let revision = store.get(id).unwrap().document.revision;
        let request = json!({"id":"session","adopt":true,"mode":"ignored","title":"after",
            "listen":false});
        assert!(!store.needs_anchor(&request).unwrap());
        let change = store.open(&request, None).unwrap();
        assert_eq!(change.opened, Some(id));
        let surface = store.get(id).unwrap();
        assert!(std::ptr::eq(document, &surface.document));
        assert_eq!(surface.document.revision, revision);
        assert_eq!(surface.anchor, Some(83));
        assert_eq!(surface.mode, SurfaceMode::Inline);
        assert_eq!(surface.open["title"], "after");
        assert_eq!(surface.open["role"], "session");
        assert!(!surface.listen);
        assert!(surface.document.palette.is_some());
        assert_eq!(surface.document.sheets[0].0, "base");
        assert!(surface.document.has("entry"));
        assert!(!surface.document.has("dock"));
        assert_eq!(store.len(), 1);
        assert_eq!(store.selected_id(), Some(id));
    }

    #[test]
    fn adopt_newest_closed_same_id_and_live_adopt_is_noop() {
        let mut store = SurfaceStore::new();
        let first = open(&mut store, "session", "inline", 1);
        store.close("session", true);
        let newest = open(&mut store, "session", "inline", 2);
        store.close("session", true);
        let active = open(&mut store, "other", "inline", 3);
        let overlay = open(&mut store, "overlay", "screen", 4);
        let change = store
            .open(&json!({"id":"session","adopt":true}), None)
            .unwrap();
        assert_eq!(change.opened, Some(newest));
        assert_eq!(store.get(first).unwrap().state(), SurfaceState::Closed);
        assert_eq!(store.get(active).unwrap().state(), SurfaceState::Closed);
        assert!(store.get(overlay).is_none());
        let cover = open(&mut store, "new-overlay", "screen", 5);
        let noop = store
            .open(&json!({"id":"session","adopt":true,"listen":false}), None)
            .unwrap();
        assert!(noop.opened.is_none());
        assert!(noop.closed.is_empty());
        assert!(store.get(newest).unwrap().listen);
        assert_eq!(store.selected_id(), Some(cover));
    }

    #[test]
    fn adoption_missing_gone_has_no_surface_and_respects_listen() {
        for listen in [false, true] {
            let mut store = SurfaceStore::new();
            let live = open(&mut store, "live", "inline", 1);
            let change = store
                .open(&json!({"id":"missing","adopt":true,"listen":listen}), None)
                .unwrap();
            assert!(change.opened.is_none());
            assert_eq!(change.gone[0].listen, listen);
            assert_eq!(
                change.gone[0].json(),
                json!({"ev":"gone","ids":["missing"]})
            );
            assert_eq!(store.selected_id(), Some(live));
        }
    }

    #[test]
    fn suspension_hands_back_grid_and_frames_can_resume() {
        let mut store = SurfaceStore::new();
        let inline = open(&mut store, "inline", "inline", 1);
        regions(&mut store, inline);
        let revision = store.get(inline).unwrap().document.revision;
        store.suspend(inline).unwrap();
        assert!(store.selected().is_none());
        assert_eq!(store.get(inline).unwrap().state(), SurfaceState::Suspended);
        assert_eq!(store.get(inline).unwrap().document.revision, revision);
        assert_eq!(
            store.get(inline).unwrap().document.focus.as_deref(),
            Some("editor")
        );
        frame(&mut store, inline, vec![json!(["resume"])]);
        assert_eq!(store.selected_id(), Some(inline));
        let screen = open(&mut store, "screen", "screen", 2);
        store.suspend(screen).unwrap();
        assert!(store.selected().is_none());
        store.resume(screen).unwrap();
        assert_eq!(store.selected_id(), Some(screen));
        store.close("screen", true);
        assert_eq!(store.selected_id(), Some(inline));
        store.close("inline", true);
        assert_eq!(store.resume(inline), Err(SurfaceError::UnknownSurface));
    }

    #[test]
    fn alternate_buffer_ownership_overlay_and_gone_once() {
        let mut store = SurfaceStore::new();
        let inline = open(&mut store, "inline", "inline", 1);
        let main_screen = open(&mut store, "same", "screen", 2);
        assert!(store.alternate_enter().removed.is_empty());
        assert!(store.selected().is_none());
        assert_eq!(
            store.needs_anchor(&json!({"id":"bad"})),
            Err(SurfaceError::InlineOnAlternate)
        );
        assert_eq!(
            store.open(&json!({"id":"bad"}), Some(3)).unwrap_err(),
            SurfaceError::InlineOnAlternate
        );
        let alternate = open(&mut store, "same", "screen", 4);
        assert_eq!(
            store.get(alternate).unwrap().buffer,
            SurfaceBuffer::Alternate
        );
        assert_eq!(store.find_open("same"), Some(alternate));
        assert_eq!(store.selected_id(), Some(alternate));
        let change = store.alternate_exit();
        assert_eq!(change.removed[0].id, alternate);
        assert_eq!(
            change.gone[0].json(),
            json!({"ev":"gone","sf":"same","ids":["same"]})
        );
        assert!(change.gone[0].listen);
        assert!(store.alternate_exit().gone.is_empty());
        assert_eq!(store.find_open("same"), Some(main_screen));
        assert_eq!(store.selected_id(), Some(main_screen));
        store.close("same", false);
        assert_eq!(store.selected_id(), Some(inline));
    }

    #[test]
    fn alternate_clear_preserves_main_and_listen_false_never_notifies() {
        let mut store = SurfaceStore::new();
        let inline = open(&mut store, "inline", "inline", 1);
        store.alternate_enter();
        let alternate = store
            .open(&json!({"id":"alt","mode":"screen","listen":false}), None)
            .unwrap()
            .opened
            .unwrap();
        let change = store.alternate_clear();
        assert_eq!(change.removed[0].id, alternate);
        assert!(!change.gone[0].listen);
        assert_eq!(store.buffer(), SurfaceBuffer::Alternate);
        assert!(store.selected().is_none());
        assert!(store.get(inline).is_some());
        store.alternate_exit();
        assert_eq!(store.selected_id(), Some(inline));
    }

    #[test]
    fn anchor_loss_is_idempotent_live_notifies_closed_does_not() {
        for keep in [false, true] {
            for listen in [false, true] {
                let mut store = SurfaceStore::new();
                let id = store
                    .open(&json!({"id":"session","listen":listen}), Some(7))
                    .unwrap()
                    .opened
                    .unwrap();
                if keep {
                    store.close("session", true);
                }
                let change = store.remove_anchor(7);
                assert_eq!(change.removed[0].id, id);
                assert_eq!(change.removed[0].anchor, Some(7));
                assert_eq!(change.removed[0].reason, RemovalReason::Anchor);
                if keep {
                    assert!(change.gone.is_empty());
                } else {
                    assert_eq!(change.gone[0].listen, listen);
                }
                assert!(store.remove_anchor(7).removed.is_empty());
                assert!(store.get(id).is_none());
            }
        }
    }

    #[test]
    fn prompt_reset_respawn_discard_retained_and_live_without_events() {
        for reason in [
            RemovalReason::Prompt,
            RemovalReason::Reset,
            RemovalReason::Respawn,
        ] {
            let mut store = SurfaceStore::new();
            let closed = open(&mut store, "closed", "inline", 1);
            store.close("closed", true);
            let live = open(&mut store, "live", "inline", 2);
            store.alternate_enter();
            let screen = open(&mut store, "screen", "screen", 3);
            let change = match reason {
                RemovalReason::Prompt => store.prompt(),
                RemovalReason::Reset => store.reset(),
                RemovalReason::Respawn => store.respawn(),
                _ => unreachable!(),
            };
            assert_eq!(
                change
                    .removed
                    .iter()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>(),
                [closed, live, screen]
            );
            assert!(change.removed.iter().all(|entry| entry.reason == reason));
            assert!(change.gone.is_empty());
            assert!(store.is_empty());
            assert!(store.selected().is_none());
            store.alternate_exit();
            let next = open(&mut store, "live", "inline", 2);
            assert_ne!(next, live);
            assert!(store.get(live).is_none());
        }
    }

    #[test]
    fn process_exit_preserves_dead_document_but_disables_listener() {
        let mut store = SurfaceStore::new();
        let old = open(&mut store, "old", "inline", 1);
        let inline = open(&mut store, "session", "inline", 2);
        regions(&mut store, inline);
        let screen = open(&mut store, "screen", "screen", 3);
        store.alternate_enter();
        let alternate = open(&mut store, "alt", "screen", 4);
        let change = store.process_exit();
        assert_eq!(change.closed, [inline, screen, alternate]);
        assert!(change.gone.is_empty());
        assert!(store.get(screen).is_none());
        assert!(store.get(alternate).is_none());
        assert_eq!(store.selected_id(), Some(inline));
        for id in [old, inline] {
            let surface = store.get(id).unwrap();
            assert_eq!(surface.state(), SurfaceState::Closed);
            assert!(!surface.listen);
        }
        assert!(store.get(inline).unwrap().document.has("entry"));
        assert!(!store.get(inline).unwrap().document.has("dock"));
        assert!(store.process_exit().closed.is_empty());
        store.prompt();
        assert!(store.selected().is_none());
    }

    #[test]
    fn invalid_open_is_transactional_and_adopt_mode_is_ignored() {
        let mut store = SurfaceStore::new();
        let inline = open(&mut store, "session", "inline", 7);
        let invalid = [
            (json!(null), SurfaceError::InvalidOpen),
            (json!({}), SurfaceError::InvalidId),
            (json!({"id":""}), SurfaceError::InvalidId),
            (json!({"id":"new","mode":"flow"}), SurfaceError::InvalidMode),
            (
                json!({"id":"new","listen":0}),
                SurfaceError::InvalidMetadata,
            ),
            (
                json!({"id":"new","title":null}),
                SurfaceError::InvalidMetadata,
            ),
            (
                json!({"id":"new","role":false}),
                SurfaceError::InvalidMetadata,
            ),
        ];
        for (value, expected) in invalid {
            assert_eq!(store.open(&value, Some(8)).unwrap_err(), expected);
            assert_eq!(store.selected_id(), Some(inline));
            assert_eq!(store.len(), 1);
        }
        assert_eq!(
            store.open(&json!({"id":"new"}), None).unwrap_err(),
            SurfaceError::MissingAnchor
        );
        assert_eq!(
            store.open(&json!({"id":"new"}), Some(7)).unwrap_err(),
            SurfaceError::DuplicateAnchor
        );
        store.close("session", true);
        let change = store
            .open(&json!({"id":"session","adopt":true,"mode":42}), None)
            .unwrap();
        assert_eq!(change.opened, Some(inline));
    }

    fn settled_history(store: &mut SurfaceStore, id: SurfaceId) {
        let root = store.get(id).unwrap().wire_id.clone();
        frame(
            store,
            id,
            vec![
                json!(["add","main",root,null,{"id":"main","k":"col","c":[
                    {"id":"old","k":"col","c":[{"id":"old-child","k":"text","p":{"text":"old"}}]},
                    {"id":"new","k":"text","p":{"text":"new"}},
                    {"id":"live","k":"col","c":[{"id":"settled-descendant","k":"text","p":{"text":"live"}}]}
                ]}]),
                json!(["settle", "old"]),
                json!(["settle", "new"]),
                json!(["settle", "settled-descendant"]),
            ],
        );
    }

    #[test]
    fn retention_closed_oldest_first_then_self_settled_main_children() {
        let mut store = SurfaceStore::new();
        let first = open(&mut store, "first", "inline", 1);
        let second = open(&mut store, "second", "inline", 2);
        let live = open(&mut store, "session", "inline", 3);
        settled_history(&mut store, live);
        store.budget =
            store.estimated_bytes() - store.get(first).unwrap().document.estimated_bytes();
        let change = store.retain();
        assert_eq!(change.removed.len(), 1);
        assert_eq!(change.removed[0].id, first);
        assert_eq!(change.removed[0].reason, RemovalReason::Retention);
        assert!(change.evicted.is_empty());
        assert!(store.get(second).is_some());
        store.budget = 0;
        let change = store.retain();
        assert_eq!(change.removed[0].id, second);
        assert_eq!(change.evicted.len(), 2);
        assert_eq!(change.evicted[0].ids, ["old", "old-child"]);
        assert_eq!(change.evicted[1].ids, ["new"]);
        assert_eq!(
            change.gone[1].json(),
            json!({"ev":"gone","sf":"session","ids":["old"]})
        );
        let document = &store.get(live).unwrap().document;
        assert!(document.has("main"));
        assert!(document.has("live"));
        assert!(document.has("settled-descendant"));
        assert!(!document.has("old-child"));
        assert!(store.estimated_bytes() > store.budget);
        assert!(store.retain().gone.is_empty());
    }

    #[test]
    fn retention_touch_unsettles_top_level_even_with_settled_descendant() {
        let mut store = SurfaceStore::new();
        let id = open(&mut store, "session", "inline", 1);
        settled_history(&mut store, id);
        frame(
            &mut store,
            id,
            vec![
                json!(["set","old-child",{"text":"changed"}]),
                json!(["settle", "old-child"]),
            ],
        );
        store.budget = 0;
        let change = store.retain();
        assert_eq!(change.evicted.len(), 1);
        assert_eq!(change.evicted[0].ids, ["new"]);
        assert!(store.get(id).unwrap().document.has("old"));
        assert!(store.get(id).unwrap().document.has("old-child"));
        assert!(store.get(id).unwrap().document.has("settled-descendant"));
    }

    #[test]
    fn local_eviction_checks_whole_request_and_reports_subtree_ids_once() {
        let mut store = SurfaceStore::new();
        let id = open(&mut store, "session", "inline", 1);
        settled_history(&mut store, id);
        assert_eq!(
            store
                .evict_nodes(id, &["old".into(), "settled-descendant".into()])
                .unwrap_err(),
            SurfaceError::InvalidEviction
        );
        assert!(store.get(id).unwrap().document.has("old"));
        assert_eq!(
            store.evict_nodes(id, &["main".into()]).unwrap_err(),
            SurfaceError::InvalidEviction
        );
        let change = store
            .evict_nodes(id, &["old".into(), "old".into()])
            .unwrap();
        assert_eq!(change.evicted.len(), 1);
        assert_eq!(change.evicted[0].ids, ["old", "old-child"]);
        assert_eq!(change.gone.len(), 1);
        frame(
            &mut store,
            id,
            vec![json!(["text", "new", "append", " valid"])],
        );
        store.close("session", true);
        assert_eq!(
            store.evict_nodes(id, &["new".into()]).unwrap_err(),
            SurfaceError::InvalidEviction
        );
    }

    #[test]
    fn budget_includes_palette_and_sheets_and_unsettled_state_is_soft() {
        let mut store = SurfaceStore::new();
        assert_eq!(store.budget, 64 * 1024 * 1024);
        let id = open(&mut store, "session", "inline", 1);
        let initial = store.estimated_bytes();
        store
            .get_mut(id)
            .unwrap()
            .document
            .set_palette(json!({"dark":{"accent":"#ffffff"},"light":{"accent":"#000000"}}))
            .unwrap();
        let palette = store.estimated_bytes();
        assert!(palette > initial);
        store
            .get_mut(id)
            .unwrap()
            .document
            .set_sheet("base", Some(&json!(".x{color:accent}")))
            .unwrap();
        assert!(store.estimated_bytes() > palette);
        store.budget = initial;
        assert!(store.retain().removed.is_empty());
        assert!(store.estimated_bytes() > store.budget);
        let change = store.close("session", true);
        assert_eq!(change.removed[0].id, id);
        assert_eq!(change.removed[0].reason, RemovalReason::Retention);
        assert!(store.is_empty());
    }

    #[test]
    fn non_listening_retention_and_clear_report_loss_without_program_delivery() {
        let mut store = SurfaceStore::new();
        let id = store
            .open(&json!({"id":"session","listen":false}), Some(1))
            .unwrap()
            .opened
            .unwrap();
        settled_history(&mut store, id);
        store.budget = 0;
        let change = store.retain();
        assert_eq!(change.evicted.len(), 2);
        assert!(change.gone.iter().all(|gone| !gone.listen));
        let change = store.clear_buffer(SurfaceBuffer::Main);
        assert_eq!(change.removed[0].id, id);
        assert!(!change.gone[0].listen);
        assert_eq!(change.removed[0].anchor, Some(1));
        assert!(store.is_empty());
    }

    #[test]
    fn closed_eviction_order_tracks_last_close_not_creation_identity() {
        let mut store = SurfaceStore::new();
        let older = open(&mut store, "older", "inline", 1);
        let newer = open(&mut store, "newer", "inline", 2);
        store.close("newer", true);
        store
            .open(&json!({"id":"older","adopt":true}), None)
            .unwrap();
        store.close("older", true);
        store.budget =
            store.estimated_bytes() - store.get(newer).unwrap().document.estimated_bytes();
        let change = store.retain();
        assert_eq!(change.removed.len(), 1);
        assert_eq!(change.removed[0].id, newer);
        assert!(store.get(older).is_some());
    }

    #[test]
    fn nested_main_and_settled_dock_cannot_be_evicted_as_history() {
        let mut store = SurfaceStore::new();
        let id = open(&mut store, "session", "inline", 1);
        frame(
            &mut store,
            id,
            vec![
                json!(["add","dock","session",null,{"id":"dock","k":"col","c":[
                    {"id":"main","k":"col","c":[{"id":"nested","k":"text","p":{"text":"live"}}]}
                ]}]),
                json!(["settle", "nested"]),
                json!(["settle", "dock"]),
            ],
        );
        store.budget = 0;
        assert!(store.retain().evicted.is_empty());
        assert_eq!(
            store.evict_nodes(id, &["nested".into()]).unwrap_err(),
            SurfaceError::InvalidEviction
        );
        assert_eq!(
            store.evict_nodes(id, &["dock".into()]).unwrap_err(),
            SurfaceError::InvalidEviction
        );
        assert!(store.get(id).unwrap().document.has("nested"));
    }

    #[test]
    fn eviction_preserves_source_sequence_and_rejects_removed_node_only() {
        let mut store = SurfaceStore::new();
        let id = open(&mut store, "session", "inline", 1);
        settled_history(&mut store, id);
        store.evict_nodes(id, &["old".into()]).unwrap();
        let surface = store.get_mut(id).unwrap();
        let result = surface
            .document
            .apply_frame(
                &Frame {
                    sf: "session".into(),
                    s: 999,
                    ops: vec![
                        json!(["set","old-child",{"text":"late"}]),
                        json!(["text", "new", "append", " still live"]),
                    ],
                },
                0,
            )
            .unwrap();
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].s, 999);
        assert_eq!(result.errors[0].op, 0);
        assert_eq!(result.original_indices, [1]);
        assert_eq!(
            surface.document.get("new", 0).unwrap()["p"]["text"],
            "new still live"
        );
    }

    #[test]
    fn retention_uses_node_creation_age_across_live_surfaces() {
        let mut store = SurfaceStore::new();
        let inline = open(&mut store, "session", "inline", 1);
        let screen = open(&mut store, "overlay", "screen", 2);
        for (id, now) in [(inline, 200), (screen, 100)] {
            let surface = store.get_mut(id).unwrap();
            let result = surface
                .document
                .apply_frame(
                    &Frame {
                        sf: surface.wire_id.clone(),
                        s: 1,
                        ops: vec![
                            json!(["add","main",surface.wire_id,null,
                            {"id":"main","k":"col","c":[
                                {"id":"history","k":"text","p":{"text":"done"}}
                            ]}]),
                            json!(["settle", "history"]),
                        ],
                    },
                    now,
                )
                .unwrap();
            assert!(result.errors.is_empty());
        }
        store.budget = store.estimated_bytes() - 1;
        let change = store.retain();
        assert_eq!(change.evicted.len(), 1);
        assert_eq!(change.evicted[0].surface, screen);
        assert!(store.get(inline).unwrap().document.has("history"));
        assert!(!store.get(screen).unwrap().document.has("history"));
    }
}
