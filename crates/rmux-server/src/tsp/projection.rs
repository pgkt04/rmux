// Ported from tmux server-client.c, tty.c @ 8f25579c
// TSP projection and credits are an rmux extension; see the P12 broker contract.
use super::{
    blobs::TspBlobStore,
    document::{ApplyResult, TspDocument},
    replay,
    status_bar::{BAR_CSS, BAR_ID, BAR_SHEET},
    wire::{Frame, JOINED_LIMIT, WireMessage},
};
use crate::ids::PaneId;
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

const PENDING_OP_LIMIT: usize = 4096;
const PENDING_BYTE_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceOp {
    pub sf: String,
    pub s: u64,
    pub op: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameCoverage {
    pub sequence: u64,
    pub revision: u64,
    pub replay: bool,
    /// One entry per outer op; an empty entry belongs to the broker snapshot.
    pub operations: Vec<Vec<SourceOp>>,
    /// Outer ops that carry rmux's status bar, not program content.
    pub bar_ops: Vec<usize>,
}

#[derive(Debug, PartialEq)]
pub enum ProjectionError {
    Closed,
    Failed,
    WrongSurface,
    InvalidAcceptedMap,
    StaleRevision,
    UnknownSequence(u64),
    DuplicateAck(u64),
    SequenceExhausted,
    PendingViewLimit,
    ErrorMapping,
    Replay(replay::ReplayError),
    /// Tern rejected an op of rmux's own status bar.
    StatusBar,
}
impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProjectionError {}

#[derive(Debug)]
pub enum PreparedSend {
    Replay(Arc<replay::ReplayPlan>),
    Live(Arc<replay::ReplayPlan>, FrameCoverage),
}

#[derive(Debug)]
struct ReplayCursor {
    sequence: u64,
    revision: u64,
    listening: bool,
    coverage: FrameCoverage,
    plan: Arc<replay::ReplayPlan>,
    parents: HashMap<String, String>,
    root_props: BTreeSet<String>,
    sheets: BTreeSet<String>,
    bar_dock: bool,
    bar_sheet: bool,
    /// Next `replay_piece` index. The frame is last and is the only piece that
    /// publishes ack coverage.
    sent_prefix: usize,
}

#[derive(Debug)]
struct PendingOp {
    value: Value,
    sources: Vec<SourceOp>,
}

#[derive(Debug)]
pub struct Projection {
    pub pane: PaneId,
    pub logical: String,
    pub outer: String,
    pub generation: u64,
    pub drawn_revision: u64,
    pub failed: bool,
    /// The outer surface is `inline`, on the client's main screen, as the
    /// program's own surface; otherwise `screen`.
    pub inline: bool,
    credits: usize,
    next_sequence: u64,
    last_ack: Option<u64>,
    sent: VecDeque<FrameCoverage>,
    pending: Vec<PendingOp>,
    pending_bytes: usize,
    pending_sources: usize,
    pending_revision: Option<u64>,
    sent_revision: u64,
    snapshot_needed: bool,
    opened: bool,
    closed: bool,
    listening: bool,
    replay: Option<ReplayCursor>,
    awaiting_frame: Option<u64>,
    // Only wire topology and root prop names, not a second document or frame log.
    parents: HashMap<String, String>,
    view_parents: HashMap<String, String>,
    root_props: BTreeSet<String>,
    /// Latest palette not yet admitted. `None` means unchanged since the last send.
    pending_palette: Option<Option<Value>>,
    /// Sheet name -> latest css; `None` deletes the sheet.
    pending_sheets: Vec<(String, Option<String>)>,
    sheets: BTreeSet<String>,
    /// The client's status line, kept as the last child of the outer `dock`.
    bar: Option<Value>,
    bar_dirty: bool,
    /// The outer `dock` exists only to hold the bar: the document has none.
    bar_dock: bool,
    /// The client takes program sheets, so the bar can span the pane.
    pub bar_styles: bool,
    /// The bar's sheet is installed on the outer surface.
    bar_sheet: bool,
}
impl Projection {
    pub fn new(
        pane: PaneId,
        logical: impl Into<String>,
        outer: impl Into<String>,
        generation: u64,
        credits: usize,
    ) -> Self {
        Self {
            pane,
            logical: logical.into(),
            outer: outer.into(),
            generation,
            drawn_revision: 0,
            failed: false,
            inline: false,
            credits,
            next_sequence: 1,
            last_ack: None,
            sent: VecDeque::new(),
            pending: Vec::new(),
            pending_bytes: 0,
            pending_sources: 0,
            pending_revision: None,
            sent_revision: 0,
            snapshot_needed: true,
            opened: false,
            closed: false,
            listening: true,
            replay: None,
            awaiting_frame: None,
            parents: HashMap::new(),
            view_parents: HashMap::new(),
            root_props: BTreeSet::new(),
            pending_palette: None,
            pending_sheets: Vec::new(),
            sheets: BTreeSet::new(),
            bar: None,
            bar_dirty: false,
            bar_dock: false,
            bar_styles: false,
            bar_sheet: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.opened && !self.closed && !self.failed
    }

    pub fn in_flight(&self) -> usize {
        self.sent.len()
    }

    pub fn has_credit(&self) -> bool {
        !self.closed && !self.failed && self.sent.len() < self.credits
    }

    /// True when the bar changed and needs a frame.
    pub fn set_bar(&mut self, bar: Option<Value>) -> bool {
        if self.bar == bar {
            return false;
        }
        self.bar = bar;
        self.bar_dirty = true;
        true
    }

    pub(crate) fn invalidate_bar(&mut self) {
        self.bar_dirty = true;
    }

    pub(crate) fn bar_pending(&self) -> bool {
        self.bar_dirty
    }

    /// The bar's sheet goes with the first frame that adds the bar.
    fn bar_sheet_due(&self, bar_ops: &[usize]) -> bool {
        self.bar_styles && !self.bar_sheet && self.bar.is_some() && !bar_ops.is_empty()
    }

    fn push_before_frame(&self, messages: &mut Vec<WireMessage>) {
        let at = messages.len() - usize::from(messages.last().is_some_and(|m| m.verb == b'f'));
        messages.insert(
            at,
            WireMessage::json(
                b's',
                &json!({"sf": self.outer, "name": BAR_SHEET, "css": BAR_CSS}),
            ),
        );
    }

    pub fn reconcile(&mut self) {
        self.snapshot_needed = true;
        self.pending.retain(|op| is_transient_view(&op.value));
        self.recount_pending();
    }

    pub fn queue_frame(
        &mut self,
        frame: &Frame,
        accepted: &ApplyResult,
    ) -> Result<(), ProjectionError> {
        self.available()?;
        if frame.sf != self.logical {
            return Err(ProjectionError::WrongSurface);
        }
        if accepted.accepted_ops.len() != accepted.original_indices.len()
            || accepted
                .original_indices
                .iter()
                .enumerate()
                .any(|(i, &index)| {
                    frame.ops.get(index) != accepted.accepted_ops.get(i)
                        || (i > 0 && accepted.original_indices[i - 1] >= index)
                })
        {
            return Err(ProjectionError::InvalidAcceptedMap);
        }
        let latest = self.pending_revision.unwrap_or_else(|| {
            self.replay
                .as_ref()
                .map_or(self.sent_revision, |cursor| cursor.revision)
        });
        if accepted.revision <= latest {
            return Err(ProjectionError::StaleRevision);
        }
        if accepted.revision > latest.saturating_add(1) {
            self.reconcile();
        }
        self.pending_revision = Some(accepted.revision);
        if !self.has_credit() {
            self.reconcile();
        }
        for (value, &index) in accepted.accepted_ops.iter().zip(&accepted.original_indices) {
            match op_name(value) {
                Some("add") => {
                    if let Some(parent) = value[2].as_str() {
                        track_subtree(&mut self.view_parents, &value[4], parent);
                    }
                }
                Some("move") => {
                    if let (Some(id), Some(parent)) = (value[1].as_str(), value[2].as_str()) {
                        self.view_parents.insert(id.into(), parent.into());
                    }
                }
                Some("del") => {
                    if let Some(id) = value[1].as_str() {
                        let removed = remove_topology(&mut self.view_parents, id);
                        self.pending.retain(|op| {
                            !is_transient_view(&op.value)
                                || !op.value[1].as_str().is_some_and(|id| removed.contains(id))
                        });
                        self.recount_pending();
                    }
                }
                _ => {}
            }
            if self.snapshot_needed && !is_transient_view(value) {
                continue;
            }
            // The protocol specifies that only the latest reveal applies.
            if op_name(value) == Some("reveal") {
                self.pending
                    .retain(|op| op_name(&op.value) != Some("reveal"));
                self.recount_pending();
            }
            let source = SourceOp {
                sf: frame.sf.clone(),
                s: frame.s,
                op: index,
            };
            let bytes = encoded_size(value) + source.sf.len() + std::mem::size_of::<SourceOp>();
            if self.pending_sources >= PENDING_OP_LIMIT
                || self.pending_bytes.saturating_add(bytes) > PENDING_BYTE_LIMIT
                || self.pending.len() >= PENDING_OP_LIMIT
            {
                self.reconcile();
                if !is_transient_view(value) {
                    continue;
                }
                if self.pending_sources >= PENDING_OP_LIMIT
                    || self.pending_bytes.saturating_add(bytes) > PENDING_BYTE_LIMIT
                    || self.pending.len() >= PENDING_OP_LIMIT
                {
                    self.failed = true;
                    return Err(ProjectionError::PendingViewLimit);
                }
            }
            self.pending_bytes += bytes;
            self.pending_sources += 1;
            if let Some(last) = self.pending.last_mut()
                && merge_adjacent(&mut last.value, value)
            {
                last.sources.push(source);
            } else {
                self.pending.push(PendingOp {
                    value: value.clone(),
                    sources: vec![source],
                });
            }
        }
        Ok(())
    }

    /// Returned messages must enter the client's lossless transaction lane in order.
    /// Coverage is registered only by `note_enqueued`; rejection retains the cursor.
    pub fn next_messages(
        &mut self,
        document: &TspDocument,
        blobs: &mut TspBlobStore,
        confirmed: &BTreeSet<String>,
        open: &Value,
        now_ms: u64,
    ) -> Result<Option<(Vec<WireMessage>, FrameCoverage)>, ProjectionError> {
        let prepared = self.prepare_send(document, blobs, confirmed, open, now_ms)?;
        let Some(prepared) = prepared else {
            return Ok(None);
        };
        match prepared {
            PreparedSend::Replay(plan) => {
                let mut messages = plan.messages.clone();
                let frame = messages.pop().expect("snapshot ends with frame");
                for blob in &plan.blobs {
                    messages.push(replay::blob_message(blob));
                }
                messages.push(frame);
                let coverage = self.replay.as_ref().expect("cursor").coverage.clone();
                Ok(Some((messages, coverage)))
            }
            PreparedSend::Live(plan, coverage) => {
                let mut messages = Vec::new();
                for blob in &plan.blobs {
                    messages.push(replay::blob_message(blob));
                }
                messages.extend(plan.messages.iter().cloned());
                Ok(Some((messages, coverage)))
            }
        }
    }

    /// Snapshot or live frame ready to queue. Does not publish ack coverage.
    pub fn prepare_send(
        &mut self,
        document: &TspDocument,
        blobs: &TspBlobStore,
        confirmed: &BTreeSet<String>,
        open: &Value,
        now_ms: u64,
    ) -> Result<Option<PreparedSend>, ProjectionError> {
        self.available()?;
        if document.surface != self.logical {
            return Err(ProjectionError::WrongSurface);
        }
        if self.awaiting_frame.is_some() {
            return Ok(self.prepared_held());
        }
        let latest = self.pending_revision.unwrap_or(self.sent_revision);
        if document.revision < latest {
            return Err(ProjectionError::StaleRevision);
        }
        if document.revision > latest {
            self.reconcile();
        }
        if !self.has_credit() {
            if self.pending_revision.is_some() {
                self.reconcile();
            }
            return Ok(None);
        }
        if !self.snapshot_needed && self.pending_revision.is_none() && !self.bar_dirty {
            return Ok(None);
        }
        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(ProjectionError::SequenceExhausted)?;
        let sequence = self.next_sequence;
        if self.snapshot_needed {
            self.prepare_replay(document, blobs, confirmed, open, now_ms, next_sequence)
        } else {
            self.prepare_live(document, blobs, confirmed, open, sequence, next_sequence)
        }
    }

    fn prepared_held(&self) -> Option<PreparedSend> {
        let cursor = self.replay.as_ref()?;
        if cursor.coverage.replay {
            Some(PreparedSend::Replay(Arc::clone(&cursor.plan)))
        } else {
            Some(PreparedSend::Live(
                Arc::clone(&cursor.plan),
                cursor.coverage.clone(),
            ))
        }
    }

    fn metadata_messages(&self) -> Vec<WireMessage> {
        let mut messages = Vec::new();
        if let Some(palette) = &self.pending_palette {
            let mut body = palette
                .clone()
                .unwrap_or_else(|| Value::Object(Default::default()));
            if let Some(object) = body.as_object_mut() {
                object.insert("sf".into(), self.outer.clone().into());
            }
            messages.push(WireMessage::json(b't', &body));
        }
        for (name, css) in &self.pending_sheets {
            messages.push(WireMessage::json(
                b's',
                &json!({"sf": self.outer, "name": name, "css": css}),
            ));
        }
        messages
    }

    fn splice_pending_metadata_live(&mut self, messages: &mut Vec<WireMessage>) {
        let extra = self.metadata_messages();
        if extra.is_empty() {
            return;
        }
        let frame = messages.pop().filter(|message| message.verb == b'f');
        messages.extend(extra);
        if let Some(frame) = frame {
            messages.push(frame);
        }
        self.pending_palette = None;
        self.pending_sheets.clear();
    }

    fn prepare_replay(
        &mut self,
        document: &TspDocument,
        blobs: &TspBlobStore,
        confirmed: &BTreeSet<String>,
        open: &Value,
        now_ms: u64,
        next_sequence: u64,
    ) -> Result<Option<PreparedSend>, ProjectionError> {
        let sequence = self.next_sequence;
        let before = if self.opened {
            self.reset_ops()
        } else {
            Vec::new()
        };
        let (canonical_parents, canonical_props, canonical_ops) =
            document.replay_topology(&self.outer);
        let prefix_ops = before.len() + canonical_ops;
        let mut frame = OuterOps::new(&self.outer, canonical_parents, canonical_props, false);
        frame.place_bar(self.bar.as_ref(), true);
        for pending in &self.pending {
            if !is_transient_view(&pending.value)
                || !document.has(pending.value[1].as_str().unwrap_or(""))
            {
                continue;
            }
            let mut value = pending.value.clone();
            rewrite_operation(&mut value, &self.logical, &self.outer);
            frame.push(value, pending.sources.clone());
        }
        frame.focus_status(self.bar.as_ref(), false, document.focus.as_deref());
        let OuterOps {
            ops,
            operations,
            mut bar_ops,
            parents,
            root_props,
            bar_dock,
            ..
        } = frame;
        let encoded = document.replay_frame(&self.outer, sequence, now_ms, &before, &ops);
        let message = WireMessage {
            verb: b'f',
            params: Default::default(),
            body: encoded,
        };
        let mut plan =
            match replay::plan_with_frame(document, blobs, confirmed, &self.outer, message) {
                Ok(plan) => plan,
                Err(error) => {
                    self.failed = true;
                    return Err(ProjectionError::Replay(error));
                }
            };
        let mut replay_operations = vec![Vec::new(); prefix_ops];
        replay_operations.extend(operations);
        for index in &mut bar_ops {
            *index += prefix_ops;
        }
        replay::set_open_metadata_and_mode(
            &mut plan.messages,
            open,
            if self.inline { "inline" } else { "screen" },
        );
        if self.opened {
            plan.messages.retain(|message| message.verb != b'o');
        }
        let sheets: BTreeSet<String> = document
            .sheets
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        let mut deletions = self
            .sheets
            .difference(&sheets)
            .map(|name| WireMessage::json(b's', &json!({"sf":self.outer,"name":name,"css":null})))
            .collect::<Vec<_>>();
        if !deletions.is_empty() {
            let frame = plan.messages.pop().expect("snapshot frame");
            plan.messages.append(&mut deletions);
            plan.messages.push(frame);
        }
        let bar_sheet = self.bar_sheet || self.bar_sheet_due(&bar_ops);
        if bar_sheet && !self.bar_sheet {
            self.push_before_frame(&mut plan.messages);
        }
        // The canonical metadata at this revision subsumes the pending batch.
        self.pending_palette = None;
        self.pending_sheets.clear();
        self.view_parents = parents
            .iter()
            .map(|(id, parent)| {
                (
                    id.clone(),
                    if parent == &self.outer {
                        self.logical.clone()
                    } else {
                        parent.clone()
                    },
                )
            })
            .collect();
        let plan = Arc::new(plan);
        let listening = open.get("listen").and_then(Value::as_bool).unwrap_or(true);
        let coverage = FrameCoverage {
            sequence,
            revision: document.revision,
            replay: true,
            operations: replay_operations,
            bar_ops,
        };
        self.replay = Some(ReplayCursor {
            sequence,
            revision: document.revision,
            listening,
            coverage,
            plan: Arc::clone(&plan),
            parents,
            root_props,
            sheets,
            bar_dock,
            bar_sheet,
            sent_prefix: 0,
        });
        self.awaiting_frame = Some(sequence);
        self.next_sequence = next_sequence;
        self.detach_pending();
        Ok(Some(PreparedSend::Replay(plan)))
    }

    fn prepare_live(
        &mut self,
        document: &TspDocument,
        blobs: &TspBlobStore,
        confirmed: &BTreeSet<String>,
        open: &Value,
        sequence: u64,
        next_sequence: u64,
    ) -> Result<Option<PreparedSend>, ProjectionError> {
        let mut frame = OuterOps::new(
            &self.outer,
            self.parents.clone(),
            self.root_props.clone(),
            self.bar_dock,
        );
        for pending in &self.pending {
            let mut value = pending.value.clone();
            rewrite_operation(&mut value, &self.logical, &self.outer);
            frame.push_program(value, pending.sources.clone());
        }
        let had_prompt = frame
            .parents
            .keys()
            .any(|id| id.starts_with("rmux:prompt:"));
        frame.place_bar(self.bar.as_ref(), self.bar_dirty);
        frame.focus_status(self.bar.as_ref(), had_prompt, document.focus.as_deref());
        let OuterOps {
            ops,
            operations,
            bar_ops,
            parents,
            root_props,
            bar_dock,
            ..
        } = frame;
        let Some(revision) = self
            .pending_revision
            .or((!ops.is_empty()).then_some(self.sent_revision))
        else {
            self.bar_dirty = false;
            return Ok(None);
        };
        let blobs_out = live_blobs(document, blobs, confirmed);
        let mut messages = Vec::new();
        let frame = replay::frame_message(&self.outer, sequence, ops);
        if frame.body.len() > JOINED_LIMIT {
            self.failed = true;
            return Err(ProjectionError::Replay(
                replay::ReplayError::FullFrameTooLarge {
                    bytes: frame.body.len(),
                },
            ));
        }
        let frame_bytes = frame.body.len();
        messages.push(frame);
        let bar_sheet = self.bar_sheet || self.bar_sheet_due(&bar_ops);
        if bar_sheet && !self.bar_sheet {
            self.push_before_frame(&mut messages);
        }
        let mut sheets = self.sheets.clone();
        for (name, css) in &self.pending_sheets {
            if css.is_some() {
                sheets.insert(name.clone());
            } else {
                sheets.remove(name);
            }
        }
        self.splice_pending_metadata_live(&mut messages);
        let listening = open.get("listen").and_then(Value::as_bool).unwrap_or(true);
        let coverage = FrameCoverage {
            sequence,
            revision,
            replay: false,
            operations,
            bar_ops,
        };
        let plan = Arc::new(replay::ReplayPlan {
            messages,
            blobs: blobs_out,
            frame_bytes,
        });
        self.replay = Some(ReplayCursor {
            sequence,
            revision,
            listening,
            coverage: coverage.clone(),
            plan: Arc::clone(&plan),
            parents,
            root_props,
            sheets,
            bar_dock,
            bar_sheet,
            sent_prefix: 0,
        });
        self.awaiting_frame = Some(sequence);
        self.next_sequence = next_sequence;
        self.detach_pending();
        Ok(Some(PreparedSend::Live(plan, coverage)))
    }

    /// Remember the latest canonical palette or sheet. A snapshot already in
    /// flight carries the document at prepare time; later updates wait here and
    /// are retried after this cursor. Sheet deletions use `css: null`.
    /// The same name replaces its previous pending body, so history does not grow.
    pub fn queue_metadata(&mut self, verb: u8, value: &Value) -> bool {
        if self.closed || self.failed {
            return false;
        }
        match verb {
            b't' => {
                let body = value.as_object().map(|object| {
                    let mut palette = Value::Object(object.clone());
                    palette.as_object_mut().unwrap().remove("sf");
                    palette
                });
                self.pending_palette =
                    Some(body.filter(|palette| palette.as_object().is_some_and(|o| !o.is_empty())));
            }
            b's' => {
                let Some(name) = value
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                else {
                    return false;
                };
                let css = match value.get("css") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(css)) => (!css.is_empty()).then(|| css.clone()),
                    Some(_) => return false,
                };
                let installed = self
                    .replay
                    .as_ref()
                    .map_or(&self.sheets, |cursor| &cursor.sheets)
                    .contains(name);
                if css.is_none() && !installed {
                    self.pending_sheets.retain(|(pending, _)| pending != name);
                    return true;
                }
                if let Some(existing) = self
                    .pending_sheets
                    .iter_mut()
                    .find(|(pending, _)| pending == name)
                {
                    existing.1 = css;
                } else {
                    self.pending_sheets.push((name.to_owned(), css));
                }
            }
            _ => return false,
        }
        true
    }

    /// Next unsent palette or sheet, in canonical order: one palette, then sheets
    /// in first-install order. `None` when the metadata queue is empty.
    pub fn peek_metadata(&self) -> Option<WireMessage> {
        // An old snapshot must finish before any newer canonical metadata.
        if self.replay.is_some() || !self.opened {
            return None;
        }
        if let Some(palette) = &self.pending_palette {
            let mut body = palette
                .clone()
                .unwrap_or_else(|| Value::Object(Default::default()));
            if let Some(object) = body.as_object_mut() {
                object.insert("sf".into(), self.outer.clone().into());
            } else {
                body = json!({"sf": self.outer});
            }
            return Some(WireMessage::json(b't', &body));
        }
        let (name, css) = self.pending_sheets.first()?;
        Some(WireMessage::json(
            b's',
            &json!({"sf": self.outer, "name": name, "css": css}),
        ))
    }

    pub fn note_metadata(&mut self) -> bool {
        if self.pending_palette.is_some() {
            self.pending_palette = None;
            return true;
        }
        if let Some((name, css)) = self.pending_sheets.first() {
            if css.is_some() {
                self.sheets.insert(name.clone());
            } else {
                self.sheets.remove(name);
            }
            self.pending_sheets.remove(0);
            return true;
        }
        false
    }

    /// Sequence of the stable batch awaiting queue admission.
    pub fn pending_sequence(&self) -> Option<u64> {
        self.replay.as_ref().map(|cursor| cursor.sequence)
    }

    pub fn take_piece(&self) -> Option<replay::ReplayPiece> {
        let cursor = self.replay.as_ref()?;
        replay::replay_piece(&cursor.plan, cursor.sent_prefix)
    }

    /// Advance after this piece was admitted. Returns true when the frame itself
    /// was admitted and coverage is now in `sent`.
    pub fn note_piece(&mut self, sequence: u64) -> bool {
        let Some(cursor) = self.replay.as_mut() else {
            return false;
        };
        if cursor.sequence != sequence {
            return false;
        }
        let Some(piece) = replay::replay_piece(&cursor.plan, cursor.sent_prefix) else {
            return false;
        };
        cursor.sent_prefix += 1;
        if !piece.is_frame() {
            return false;
        }
        let cursor = self.replay.take().expect("cursor");
        self.commit_cursor(cursor);
        true
    }

    /// Publish coverage only after the tty accepted the transaction.
    pub fn note_enqueued(&mut self, sequence: u64) {
        if self.pending_sequence() != Some(sequence) {
            return;
        }
        let cursor = self.replay.take().expect("cursor");
        self.commit_cursor(cursor);
    }

    fn commit_cursor(&mut self, cursor: ReplayCursor) {
        self.awaiting_frame = None;
        self.parents = cursor.parents;
        self.root_props = cursor.root_props;
        self.sheets = cursor.sheets;
        self.bar_dock = cursor.bar_dock;
        self.bar_sheet = cursor.bar_sheet;
        self.listening = cursor.listening;
        if self.listening {
            self.sent.push_back(cursor.coverage);
        }
        self.sent_revision = cursor.revision;
        self.opened = true;
    }

    fn detach_pending(&mut self) {
        self.pending.clear();
        self.pending_bytes = 0;
        self.pending_sources = 0;
        self.pending_revision = None;
        self.snapshot_needed = false;
        self.bar_dirty = false;
    }

    pub fn ack(&mut self, sequence: u64) -> Result<u64, ProjectionError> {
        self.available()?;
        if self.last_ack.is_some_and(|last| sequence <= last) {
            return Err(ProjectionError::DuplicateAck(sequence));
        }
        let Some(index) = self
            .sent
            .iter()
            .position(|frame| frame.sequence == sequence)
        else {
            return Err(ProjectionError::UnknownSequence(sequence));
        };
        let revision = self.sent[index].revision;
        for _ in 0..=index {
            self.sent.pop_front();
        }
        self.last_ack = Some(sequence);
        self.drawn_revision = self.drawn_revision.max(revision);
        Ok(self.drawn_revision)
    }

    pub fn close(&mut self) -> WireMessage {
        self.closed = true;
        self.sent.clear();
        self.pending.clear();
        self.parents.clear();
        self.view_parents.clear();
        self.root_props.clear();
        self.pending_revision = None;
        self.pending_bytes = 0;
        self.pending_sources = 0;
        self.replay = None;
        self.awaiting_frame = None;
        self.pending_palette = None;
        self.pending_sheets.clear();
        self.sheets.clear();
        // Surfaces doc: `x` names the surface by `id`; frames and events use `sf`.
        WireMessage::json(b'x', &json!({"id":self.outer,"keep":false}))
    }

    /// The caller still validates node existence, listener lifetime and permission.
    pub fn route_event(&self, event: &Value) -> Option<Value> {
        if !self.is_open() || !self.listening || event.get("sf")?.as_str()? != self.outer {
            return None;
        }
        let mut routed = event.clone();
        routed["sf"] = self.logical.clone().into();
        if let Some(id) = routed.get_mut("id") {
            rewrite_reference(id, &self.outer, &self.logical);
        }
        if event.get("ev").and_then(Value::as_str) == Some("gone")
            && let Some(ids) = routed.get_mut("ids").and_then(Value::as_array_mut)
        {
            for id in ids {
                rewrite_reference(id, &self.outer, &self.logical);
            }
        }
        Some(routed)
    }

    pub fn map_error(&self, event: &Value) -> Result<Option<Value>, ProjectionError> {
        self.available()?;
        if !self.listening || event.get("sf").and_then(Value::as_str) != Some(&self.outer) {
            return Ok(None);
        }
        if event.get("sheet").and_then(Value::as_str) == Some(BAR_SHEET) {
            return Err(ProjectionError::StatusBar);
        }
        let sequence = event
            .get("s")
            .and_then(Value::as_u64)
            .ok_or(ProjectionError::ErrorMapping)?;
        let coverage = self
            .sent
            .iter()
            .find(|frame| frame.sequence == sequence)
            .ok_or(ProjectionError::ErrorMapping)?;
        let Some(index) = event.get("op").and_then(Value::as_u64) else {
            return if coverage.replay {
                Ok(None)
            } else {
                Err(ProjectionError::ErrorMapping)
            };
        };
        let index = usize::try_from(index).map_err(|_| ProjectionError::ErrorMapping)?;
        if coverage.bar_ops.contains(&index) {
            return Err(ProjectionError::StatusBar);
        }
        let sources = coverage
            .operations
            .get(index)
            .ok_or(ProjectionError::ErrorMapping)?;
        if sources.is_empty() && coverage.replay {
            return Ok(None);
        }
        let [source] = sources.as_slice() else {
            return Err(ProjectionError::ErrorMapping);
        };
        let mut routed = event.clone();
        routed["sf"] = source.sf.clone().into();
        routed["s"] = source.s.into();
        routed["op"] = source.op.into();
        Ok(Some(routed))
    }

    fn available(&self) -> Result<(), ProjectionError> {
        if self.closed {
            Err(ProjectionError::Closed)
        } else if self.failed {
            Err(ProjectionError::Failed)
        } else {
            Ok(())
        }
    }

    fn recount_pending(&mut self) {
        self.pending_sources = self.pending.iter().map(|op| op.sources.len()).sum();
        self.pending_bytes = self
            .pending
            .iter()
            .map(|op| {
                encoded_size(&op.value)
                    + op.sources
                        .iter()
                        .map(|source| source.sf.len() + std::mem::size_of::<SourceOp>())
                        .sum::<usize>()
            })
            .sum();
    }

    fn reset_ops(&self) -> Vec<Value> {
        let mut ops = vec![json!(["focus", null]), json!(["resume"])];
        for (id, parent) in &self.parents {
            if parent == &self.outer {
                ops.push(json!(["del", id]));
            }
        }
        if !self.root_props.is_empty() {
            let props: Map<String, Value> = self
                .root_props
                .iter()
                .map(|key| (key.clone(), Value::Null))
                .collect();
            ops.push(json!(["set", self.outer, props]));
        }
        ops
    }

    fn track_operation(
        parents: &mut HashMap<String, String>,
        root_props: &mut BTreeSet<String>,
        outer: &str,
        op: &Value,
    ) {
        match op_name(op) {
            Some("add") => {
                if let Some(parent) = op[2].as_str() {
                    track_subtree(parents, &op[4], parent);
                }
            }
            Some("move") => {
                if let (Some(id), Some(parent)) = (op[1].as_str(), op[2].as_str()) {
                    parents.insert(id.into(), parent.into());
                }
            }
            Some("del") => {
                if let Some(id) = op[1].as_str() {
                    remove_topology(parents, id);
                }
            }
            Some("set") if op[1].as_str() == Some(outer) => {
                if let Some(props) = op[2].as_object() {
                    for (key, value) in props {
                        if value.is_null() {
                            root_props.remove(key);
                        } else {
                            root_props.insert(key.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// One outer frame in the making: ops, their program sources, and the outer
/// topology after them.
struct OuterOps<'a> {
    outer: &'a str,
    ops: Vec<Value>,
    operations: Vec<Vec<SourceOp>>,
    bar_ops: Vec<usize>,
    parents: HashMap<String, String>,
    root_props: BTreeSet<String>,
    bar_dock: bool,
}
impl<'a> OuterOps<'a> {
    fn new(
        outer: &'a str,
        parents: HashMap<String, String>,
        root_props: BTreeSet<String>,
        bar_dock: bool,
    ) -> Self {
        Self {
            outer,
            ops: Vec::new(),
            operations: Vec::new(),
            bar_ops: Vec::new(),
            parents,
            root_props,
            bar_dock,
        }
    }
    fn push(&mut self, op: Value, sources: Vec<SourceOp>) {
        Projection::track_operation(&mut self.parents, &mut self.root_props, self.outer, &op);
        self.ops.push(op);
        self.operations.push(sources);
    }
    fn push_bar(&mut self, op: Value) {
        self.bar_ops.push(self.ops.len());
        self.push(op, Vec::new());
    }
    fn child_of(&self, id: &str, parent: &str) -> bool {
        self.parents.get(id).is_some_and(|p| p == parent)
    }
    /// The program's own dock replaces the bar's; its appends land above the bar.
    fn push_program(&mut self, mut op: Value, sources: Vec<SourceOp>) {
        let inserts = matches!(op_name(&op), Some("add" | "move"));
        if self.bar_dock && op_name(&op) == Some("add") && op[1] == "dock" && op[2] == self.outer {
            self.push_bar(json!(["del", "dock"]));
            self.bar_dock = false;
        }
        if inserts && op[2] == "dock" && op[3].is_null() && self.child_of(BAR_ID, "dock") {
            op[3] = BAR_ID.into();
        }
        self.push(op, sources);
    }
    /// Put `bar` last in `dock`, or take it out. `replace` resends a bar that is
    /// already there.
    fn place_bar(&mut self, bar: Option<&Value>, replace: bool) {
        let present = self.child_of(BAR_ID, "dock");
        match bar {
            Some(bar)
                if present
                    && replace
                    && bar
                        .get("c")
                        .and_then(Value::as_array)
                        .is_some_and(|children| {
                            children.iter().any(|node| {
                                node["k"] == "input"
                                    && node["id"]
                                        .as_str()
                                        .is_some_and(|id| self.child_of(id, BAR_ID))
                            })
                        }) =>
            {
                let children = bar["c"].as_array().expect("bar children checked");
                let removed: Vec<_> = self
                    .parents
                    .iter()
                    .filter(|(id, parent)| {
                        parent.as_str() == BAR_ID
                            && !children
                                .iter()
                                .any(|node| node["k"] == "input" && node["id"] == id.as_str())
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in removed {
                    self.push_bar(json!(["del", id]));
                }
                let mut before = children
                    .iter()
                    .find(|node| node["k"] == "input")
                    .map_or(Value::Null, |node| node["id"].clone());
                for node in children {
                    let Some(id) = node["id"].as_str() else {
                        continue;
                    };
                    if node["k"] == "input" && self.child_of(id, BAR_ID) {
                        self.push_bar(json!(["set", id, node["p"]]));
                        before = Value::Null;
                    } else {
                        self.push_bar(json!(["add", id, BAR_ID, before, node]));
                    }
                }
            }
            Some(_) if present && !replace => {}
            Some(bar) => {
                if present {
                    self.push_bar(json!(["del", BAR_ID]));
                }
                if self.child_of("dock", self.outer) {
                    self.push_bar(json!(["add", BAR_ID, "dock", null, bar]));
                } else {
                    self.push_bar(json!(["add", "dock", self.outer, null,
                        {"id": "dock", "k": "col", "c": [bar]}]));
                    self.bar_dock = true;
                }
            }
            None if self.bar_dock => {
                self.push_bar(json!(["del", "dock"]));
                self.bar_dock = false;
            }
            None if present => self.push_bar(json!(["del", BAR_ID])),
            None => {}
        }
    }

    fn focus_status(&mut self, bar: Option<&Value>, restore: bool, focus: Option<&str>) {
        let prompt = bar
            .and_then(|bar| bar.get("c").and_then(Value::as_array))
            .and_then(|children| children.iter().find(|node| node["k"] == "input"))
            .and_then(|node| node["id"].as_str());
        if let Some(prompt) = prompt {
            self.push_bar(json!(["focus", prompt]));
        } else if restore {
            self.push_bar(json!([
                "focus",
                focus.map(|id| {
                    if self.parents.contains_key(id) {
                        id
                    } else {
                        self.outer
                    }
                })
            ]));
        }
    }
}

fn op_name(op: &Value) -> Option<&str> {
    op.as_array()?.first()?.as_str()
}
fn is_transient_view(op: &Value) -> bool {
    matches!(op_name(op), Some("reveal" | "scroll"))
}
fn merge_adjacent(previous: &mut Value, next: &Value) -> bool {
    if previous[1] != next[1] {
        return false;
    }
    match (op_name(previous), op_name(next)) {
        (Some("set"), Some("set")) => {
            let Some(props) = next[2].as_object() else {
                return false;
            };
            let Some(target) = previous[2].as_object_mut() else {
                return false;
            };
            target.extend(
                props
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
            true
        }
        (Some("text"), Some("text")) if previous[2] == "append" && next[2] == "append" => {
            let Some(text) = next[3].as_str() else {
                return false;
            };
            let Some(Value::String(target)) = previous.get_mut(3) else {
                return false;
            };
            target.push_str(text);
            true
        }
        _ => false,
    }
}
fn rewrite_reference(value: &mut Value, from: &str, to: &str) {
    if value.as_str() == Some(from) {
        *value = to.into();
    }
}
fn rewrite_subtree(value: &mut Value, from: &str, to: &str) {
    if let Some(id) = value.get_mut("id") {
        rewrite_reference(id, from, to);
    }
    if let Some(children) = value.get_mut("c").and_then(Value::as_array_mut) {
        for child in children {
            rewrite_subtree(child, from, to);
        }
    }
}
pub fn rewrite_operation(value: &mut Value, from: &str, to: &str) {
    let Some(op) = value.as_array_mut() else {
        return;
    };
    match op.first().and_then(Value::as_str) {
        Some("add") => {
            for index in [1, 2, 3] {
                if let Some(reference) = op.get_mut(index) {
                    rewrite_reference(reference, from, to);
                }
            }
            if let Some(tree) = op.get_mut(4) {
                rewrite_subtree(tree, from, to);
            }
        }
        Some("move") => {
            for index in [1, 2, 3] {
                if let Some(reference) = op.get_mut(index) {
                    rewrite_reference(reference, from, to);
                }
            }
        }
        Some("set" | "text" | "splice" | "del" | "settle" | "focus" | "reveal" | "scroll") => {
            if let Some(reference) = op.get_mut(1) {
                rewrite_reference(reference, from, to);
            }
        }
        _ => {}
    }
}
fn track_subtree(parents: &mut HashMap<String, String>, tree: &Value, parent: &str) {
    let Some(id) = tree.get("id").and_then(Value::as_str) else {
        return;
    };
    parents.insert(id.into(), parent.into());
    if let Some(children) = tree.get("c").and_then(Value::as_array) {
        for child in children {
            track_subtree(parents, child, id);
        }
    }
}

fn remove_topology(parents: &mut HashMap<String, String>, id: &str) -> BTreeSet<String> {
    let mut removed = BTreeSet::from([id.to_owned()]);
    loop {
        let before = removed.len();
        for (child, parent) in parents.iter() {
            if removed.contains(parent) {
                removed.insert(child.clone());
            }
        }
        if removed.len() == before {
            break;
        }
    }
    parents.retain(|id, _| !removed.contains(id));
    removed
}

fn live_blobs(
    document: &TspDocument,
    blobs: &TspBlobStore,
    confirmed: &BTreeSet<String>,
) -> Vec<replay::ReplayBlob> {
    let mut out = Vec::new();
    for id in document.blob_references().difference(confirmed) {
        if let Some(blob) = blobs.peek(id) {
            out.push(replay::ReplayBlob {
                id: id.clone(),
                mime: blob.mime.clone(),
                bytes: std::sync::Arc::clone(&blob.bytes),
            });
        }
    }
    out
}

fn encoded_size(value: &Value) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).expect("JSON value serialization");
    counter.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;

    fn projection(credits: usize) -> Projection {
        Projection::new(PaneId::from_parts(4, 2), "s", "outer", 7, credits)
    }
    fn apply(p: &mut Projection, d: &mut TspDocument, s: u64, ops: Vec<Value>) {
        let frame = Frame {
            sf: "s".into(),
            s,
            ops,
        };
        let accepted = d.apply_frame(&frame, 0).unwrap();
        p.queue_frame(&frame, &accepted).unwrap();
    }
    fn send(p: &mut Projection, d: &TspDocument) -> (Vec<WireMessage>, FrameCoverage) {
        let (messages, coverage) = p
            .next_messages(d, &mut TspBlobStore::new(), &BTreeSet::new(), &json!({}), 0)
            .unwrap()
            .unwrap();
        p.note_enqueued(coverage.sequence);
        (messages, coverage)
    }
    fn frame(messages: &[WireMessage]) -> Value {
        serde_json::from_slice(&messages.last().unwrap().body).unwrap()
    }
    #[test]
    fn credits_and_cumulative_exact_acks() {
        for credits in [1, 2] {
            let mut p = projection(credits);
            let mut d = TspDocument::new("s");
            let (_, first) = send(&mut p, &d);
            assert_eq!(p.drawn_revision, 0);
            assert_eq!(
                p.ack(first.sequence + 9),
                Err(ProjectionError::UnknownSequence(10))
            );
            if credits == 2 {
                apply(&mut p, &mut d, 71, vec![json!(["set","s",{"text":"a"}])]);
                let (_, second) = send(&mut p, &d);
                assert_eq!(p.in_flight(), 2);
                assert_eq!(p.ack(second.sequence).unwrap(), d.revision);
                assert_eq!(p.in_flight(), 0);
                assert_eq!(p.ack(first.sequence), Err(ProjectionError::DuplicateAck(1)));
                assert_eq!(
                    p.ack(second.sequence),
                    Err(ProjectionError::DuplicateAck(2))
                );
            } else {
                apply(&mut p, &mut d, 900, vec![json!(["set","s",{"text":"a"}])]);
                assert!(
                    p.next_messages(
                        &d,
                        &mut TspBlobStore::new(),
                        &BTreeSet::new(),
                        &json!({}),
                        0
                    )
                    .unwrap()
                    .is_none()
                );
                assert_eq!(p.drawn_revision, 0);
                assert_eq!(p.ack(first.sequence).unwrap(), 0);
                let (_, snapshot) = send(&mut p, &d);
                assert!(snapshot.replay);
                assert_eq!(p.ack(snapshot.sequence).unwrap(), 1);
            }
        }
    }

    #[test]
    fn source_and_outer_sequences_may_skip() {
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        let (_, initial) = send(&mut p, &d);
        p.ack(initial.sequence).unwrap();
        p.next_sequence = 19;
        apply(&mut p, &mut d, 40, vec![json!(["set","s",{"one":1}])]);
        let (_, first) = send(&mut p, &d);
        p.next_sequence = 61;
        apply(&mut p, &mut d, 9000, vec![json!(["set","s",{"two":2}])]);
        let (_, second) = send(&mut p, &d);
        assert_eq!(p.ack(20), Err(ProjectionError::UnknownSequence(20)));
        assert_eq!(p.ack(60), Err(ProjectionError::UnknownSequence(60)));
        assert_eq!(p.drawn_revision, 0);
        assert_eq!(p.ack(second.sequence).unwrap(), 2);
        assert_eq!(p.in_flight(), 0);
        assert_eq!(first.operations[0][0].s, 40);
        assert_eq!(second.operations[0][0].s, 9000);
        assert_eq!(
            p.ack(first.sequence),
            Err(ProjectionError::DuplicateAck(19))
        );
    }
    #[test]
    fn accepted_indices_view_ops_and_replay_errors() {
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        let (_, initial) = send(&mut p, &d);
        assert_eq!(
            p.map_error(&json!({"ev":"error","sf":"outer","s":1}))
                .unwrap(),
            None
        );
        p.ack(initial.sequence).unwrap();
        let f = Frame {
            sf: "s".into(),
            s: 105,
            ops: vec![
                json!(["del", "absent"]),
                json!(["set","s",{"text":"s"}]),
                json!(["reveal", "s", "end"]),
                json!(["scroll", "s", "page-down"]),
                json!(["focus", "s"]),
            ],
        };
        let accepted = d.apply_frame(&f, 0).unwrap();
        assert_eq!(accepted.original_indices, [1, 2, 3, 4]);
        p.queue_frame(&f, &accepted).unwrap();
        let (messages, coverage) = send(&mut p, &d);
        assert!(!coverage.replay);
        let value = frame(&messages);
        assert_eq!(value["ops"][1], json!(["reveal", "outer", "end"]));
        assert_eq!(value["ops"][2], json!(["scroll", "outer", "page-down"]));
        let mapped = p
            .map_error(&json!({"ev":"error","sf":"outer","s":coverage.sequence,"op":2,"msg":"bad"}))
            .unwrap()
            .unwrap();
        assert_eq!(
            mapped,
            json!({"ev":"error","sf":"s","s":105,"op":3,"msg":"bad"})
        );
        assert_eq!(
            p.map_error(&json!({"ev":"error","sf":"outer","s":100,"op":0})),
            Err(ProjectionError::ErrorMapping)
        );
    }

    #[test]
    fn explicit_root_references_never_rewrite_props() {
        let mut op = json!(["add","s","s","s",{"id":"s","k":"col","p":{
            "id":"s","text":"s","href":"s","values":{"s":"s"},"css":"#s{}"
        },"c":[{"id":"node","k":"text","p":{"text":"s"}}]}]);
        let props = op[4]["p"].clone();
        rewrite_operation(&mut op, "s", "outer");
        assert_eq!(
            &op.as_array().unwrap()[1..4],
            &[json!("outer"), json!("outer"), json!("outer")]
        );
        assert_eq!(op[4]["id"], "outer");
        assert_eq!(op[4]["p"], props);
        assert_eq!(op[4]["c"][0]["id"], "node");
        assert_eq!(op[4]["c"][0]["p"]["text"], "s");
        for original in [
            json!(["set","s",{"text":"s"}]),
            json!(["text", "s", "replace", "s"]),
            json!(["splice", "s", 0, 0, "s"]),
        ] {
            let mut rewritten = original.clone();
            rewrite_operation(&mut rewritten, "s", "outer");
            assert_eq!(rewritten[1], "outer");
            assert_eq!(
                &rewritten.as_array().unwrap()[2..],
                &original.as_array().unwrap()[2..]
            );
        }
    }
    #[test]
    fn slow_client_reconciles_latest_canonical_without_duplicate_regions() {
        let mut p = projection(1);
        let mut d = TspDocument::new("s");
        apply(
            &mut p,
            &mut d,
            12,
            vec![
                json!(["set","s",{"old":"s"}]),
                json!(["add","main","s",null,{"id":"main","k":"col","c":[
                {"id":"a","k":"text","p":{"text":"first"}}]}]),
            ],
        );
        let (first, coverage) = send(&mut p, &d);
        let mut view = TspDocument::new("outer");
        view.apply_frame(&serde_json::from_value(frame(&first)).unwrap(), 0)
            .unwrap();
        for s in 20..5000 {
            apply(
                &mut p,
                &mut d,
                s,
                vec![json!(["set","a",{"text":s.to_string()}])],
            );
        }
        apply(
            &mut p,
            &mut d,
            9000,
            vec![
                json!(["set","s",{"old":null}]),
                json!(["reveal", "a", "end"]),
                json!(["scroll", "a", "line-down"]),
            ],
        );
        assert!(p.pending.len() <= 2);
        assert!(p.pending_bytes < PENDING_BYTE_LIMIT);
        assert_eq!(p.drawn_revision, 0);
        p.ack(coverage.sequence).unwrap();
        let (replayed, coverage) = send(&mut p, &d);
        assert!(coverage.replay);
        assert!(!replayed.iter().any(|message| message.verb == b'o'));
        let replayed = frame(&replayed);
        let applied = view
            .apply_frame(&serde_json::from_value(replayed.clone()).unwrap(), 0)
            .unwrap();
        assert!(applied.errors.is_empty());
        assert_eq!(view.get("a", 0).unwrap()["p"]["text"], "4999");
        assert!(view.snapshot(0)["p"].get("old").is_none());
        let ops = replayed["ops"].as_array().unwrap();
        assert_eq!(
            &ops[ops.len() - 2..],
            &[
                json!(["reveal", "a", "end"]),
                json!(["scroll", "a", "line-down"])
            ]
        );
        assert!(
            p.map_error(&json!({"ev":"error","sf":"outer","s":coverage.sequence,"op":0}))
                .unwrap()
                .is_none()
        );
        assert_eq!(p.ack(coverage.sequence).unwrap(), d.revision);
    }

    #[test]
    fn adjacent_coalescing_preserves_dependency_barriers() {
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        apply(
            &mut p,
            &mut d,
            1,
            vec![json!(["add","n","s",null,{"id":"n","k":"text"}])],
        );
        let (_, initial) = send(&mut p, &d);
        p.ack(initial.sequence).unwrap();
        apply(
            &mut p,
            &mut d,
            10,
            vec![
                json!(["set","n",{"x":1}]),
                json!(["set","n",{"x":2}]),
                json!(["text", "n", "append", "a"]),
                json!(["text", "n", "append", "b"]),
                json!(["reveal", "n", "end"]),
                json!(["set","n",{"x":3}]),
            ],
        );
        let (messages, coverage) = send(&mut p, &d);
        assert_eq!(
            frame(&messages)["ops"],
            json!([
                ["set","n",{"x":2}],["text","n","append","ab"],["reveal","n","end"],["set","n",{"x":3}]
            ])
        );
        assert_eq!(coverage.operations[0].len(), 2);
        assert_eq!(coverage.operations[1].len(), 2);
        assert_eq!(coverage.operations[3][0].op, 5);
    }
    #[test]
    fn snapshot_metadata_blobs_styles_and_time_precede_frame() {
        use sha2::{Digest, Sha256};
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        let mut blobs = TspBlobStore::new();
        let hash = format!("{:x}", Sha256::digest(b"hello"));
        blobs.insert(&hash, Some("image/png"), b"aGVsbG8=").unwrap();
        d.set_palette(json!({"dark":{}})).unwrap();
        d.set_sheet("first", Some(&json!("#s{content:'s'}")))
            .unwrap();
        d.set_sheet("second", Some(&json!("main{}"))).unwrap();
        apply(
            &mut p,
            &mut d,
            18,
            vec![
                json!(["set","s",{"text":"s"}]),
                json!(["add","main","s",null,{"id":"main","k":"col","c":[
                    {"id":"timer","k":"elapsed","p":{"age":-50,"took":19}},
                    {"id":"image","k":"image","p":{"blob":hash}}
                ]}]),
                json!(["settle", "timer"]),
                json!(["focus", "s"]),
                json!(["suspend"]),
            ],
        );
        let (messages, coverage) = p.next_messages(&d, &mut blobs, &BTreeSet::new(),
            &json!({"title":"s","role":"s","listen":true,"key":"secret","buf":"alt","seeded":true}),100)
            .unwrap().unwrap();
        p.note_enqueued(coverage.sequence);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.verb)
                .collect::<Vec<_>>(),
            b"otssbf"
        );
        let open: Value = serde_json::from_slice(&messages[0].body).unwrap();
        assert_eq!(
            open,
            json!({"id":"outer","mode":"screen","listen":true,"title":"s","role":"s"})
        );
        let sheet: Value = serde_json::from_slice(&messages[2].body).unwrap();
        assert_eq!(sheet["sf"], "outer");
        assert_eq!(sheet["css"], "#s{content:'s'}");
        assert_eq!(messages[4].params["id"], hash);
        let value = frame(&messages);
        assert_eq!(value["ops"][0], json!(["set","outer",{"text":"s"}]));
        assert_eq!(value["ops"][1][4]["c"][0]["p"]["age"], 50.0);
        assert_eq!(value["ops"][1][4]["c"][0]["p"]["took"], 19);
        assert_eq!(value["ops"][3], json!(["focus", "outer"]));
        assert_eq!(value["ops"][4], json!(["suspend"]));
        assert!(
            p.map_error(&json!({"ev":"error","sf":"outer","s":coverage.sequence,"op":1}))
                .unwrap()
                .is_none()
        );
        p.ack(coverage.sequence).unwrap();
        apply(
            &mut p,
            &mut d,
            200,
            vec![json!(["set","image",{"label":"s"}])],
        );
        let (live, coverage) = p
            .next_messages(&d, &mut blobs, &BTreeSet::from([hash]), &json!({}), 100)
            .unwrap()
            .unwrap();
        p.note_enqueued(coverage.sequence);
        assert_eq!(
            live.iter().map(|message| message.verb).collect::<Vec<_>>(),
            b"f"
        );
    }
    #[test]
    fn event_mapping_preserves_form_strings_and_close_invalidates_first() {
        let mut p = projection(2);
        let d = TspDocument::new("s");
        let (_, coverage) = send(&mut p, &d);
        let event = json!({"ev":"action","sf":"outer","id":"outer","item":"outer",
            "value":"outer","key":"outer","values":{"outer":"outer"}});
        let routed = p.route_event(&event).unwrap();
        assert_eq!(routed["sf"], "s");
        assert_eq!(routed["id"], "s");
        assert_eq!(routed["item"], "outer");
        assert_eq!(routed["values"], event["values"]);
        assert_eq!(routed["key"], "outer");
        assert_eq!(routed["value"], "outer");
        assert_eq!(
            p.route_event(&json!({"ev":"gone","sf":"outer","ids":["outer","node"]}))
                .unwrap()["ids"],
            json!(["s", "node"])
        );
        assert!(p.route_event(&json!({"sf":"stale","id":"outer"})).is_none());
        let close: Value = serde_json::from_slice(&p.close().body).unwrap();
        assert_eq!(close, json!({"id":"outer","keep":false}));
        assert!(!p.is_open());
        assert!(p.route_event(&event).is_none());
        assert_eq!(p.ack(coverage.sequence), Err(ProjectionError::Closed));
    }

    #[test]
    fn no_listener_has_neither_debt_nor_input_route() {
        let mut p = projection(1);
        let d = TspDocument::new("s");
        let (_, coverage) = p
            .next_messages(
                &d,
                &mut TspBlobStore::new(),
                &BTreeSet::new(),
                &json!({"listen":false}),
                0,
            )
            .unwrap()
            .unwrap();
        p.note_enqueued(coverage.sequence);
        assert_eq!(p.in_flight(), 0);
        assert_eq!(
            p.ack(coverage.sequence),
            Err(ProjectionError::UnknownSequence(1))
        );
        assert_eq!(p.drawn_revision, 0);
        assert!(
            p.route_event(&json!({"ev":"edit","sf":"outer","id":"outer"}))
                .is_none()
        );
    }

    #[test]
    fn pending_limit_uses_snapshot_without_erasing_relative_scroll() {
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        let (_, initial) = send(&mut p, &d);
        p.ack(initial.sequence).unwrap();
        for s in 1..=PENDING_OP_LIMIT as u64 + 1 {
            apply(
                &mut p,
                &mut d,
                s,
                vec![json!(["set","s",{"text":s.to_string()}])],
            );
        }
        assert!(p.snapshot_needed);
        assert!(p.pending.is_empty());
        let (_, snapshot) = send(&mut p, &d);
        assert!(snapshot.replay);
        p.ack(snapshot.sequence).unwrap();
        let f = Frame {
            sf: "s".into(),
            s: 99999,
            ops: vec![json!(["scroll", "s", "line-down"]); PENDING_OP_LIMIT + 1],
        };
        let accepted = d.apply_frame(&f, 0).unwrap();
        assert_eq!(
            p.queue_frame(&f, &accepted),
            Err(ProjectionError::PendingViewLimit)
        );
        assert!(p.failed);
        assert_eq!(p.ack(snapshot.sequence + 1), Err(ProjectionError::Failed));
    }
    #[test]
    fn snapshot_does_not_retarget_deleted_node_view_ops() {
        let mut p = projection(1);
        let mut d = TspDocument::new("s");
        apply(
            &mut p,
            &mut d,
            1,
            vec![json!(["add","main","s",null,
            {"id":"main","k":"col","c":[{"id":"n","k":"text"}]}])],
        );
        let (_, first) = send(&mut p, &d);
        apply(&mut p, &mut d, 2, vec![json!(["scroll", "n", "line-down"])]);
        apply(
            &mut p,
            &mut d,
            3,
            vec![
                json!(["del", "main"]),
                json!(["add","main","s",null,{"id":"main","k":"col","c":[{"id":"n","k":"text"}]}]),
                json!(["reveal", "n", "end"]),
            ],
        );
        p.ack(first.sequence).unwrap();
        let (messages, _) = send(&mut p, &d);
        let value = frame(&messages);
        assert!(
            !value["ops"]
                .as_array()
                .unwrap()
                .iter()
                .any(|op| op_name(op) == Some("scroll"))
        );
        assert_eq!(
            value["ops"].as_array().unwrap().last().unwrap(),
            &json!(["reveal", "n", "end"])
        );
    }

    #[test]
    fn snapshot_above_joined_limit_fails_before_open() {
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        apply(
            &mut p,
            &mut d,
            1,
            vec![json!(["set","s",{"text":"x".repeat(JOINED_LIMIT)}])],
        );
        let err = p
            .prepare_send(&d, &TspBlobStore::new(), &BTreeSet::new(), &json!({}), 0)
            .unwrap_err();
        assert!(matches!(
            err,
            ProjectionError::Replay(replay::ReplayError::FullFrameTooLarge { .. })
        ));
        assert!(!p.opened);
        assert!(p.sent.is_empty());
        assert!(p.failed);
    }

    #[test]
    fn queue_full_keeps_snapshot_pending_until_admitted() {
        let mut p = projection(2);
        let d = TspDocument::new("s");
        let prepared = p
            .prepare_send(&d, &TspBlobStore::new(), &BTreeSet::new(), &json!({}), 0)
            .unwrap();
        assert!(matches!(prepared, Some(PreparedSend::Replay(_))));
        assert_eq!(p.in_flight(), 0);
        assert_eq!(p.pending_sequence(), Some(1));
        let piece = p.take_piece().expect("open");
        assert!(!piece.is_frame());
        assert!(!p.note_piece(1));
        assert_eq!(p.pending_sequence(), Some(1));
        assert_eq!(p.in_flight(), 0);
        assert_eq!(p.pending_sequence(), Some(1), "admitted prefix is retained");
        while p.take_piece().is_some() {
            if p.note_piece(1) {
                break;
            }
        }
        assert!(!p.snapshot_needed);
        assert_eq!(p.in_flight(), 1);
    }

    #[test]
    fn blocked_replay_retains_later_frames_and_orders_metadata() {
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        d.set_palette(json!({"dark":{"fg":"old"}})).unwrap();
        d.set_sheet("style", Some(&json!("old{}"))).unwrap();
        apply(
            &mut p,
            &mut d,
            10,
            vec![
                json!(["add","n","s",null,{"id":"n","k":"text","p":{"text":"old"}}]),
                json!(["scroll", "n", "line-down"]),
            ],
        );
        let blobs = TspBlobStore::new();
        let confirmed = BTreeSet::new();
        let Some(PreparedSend::Replay(initial)) = p
            .prepare_send(&d, &blobs, &confirmed, &json!({}), 0)
            .unwrap()
        else {
            panic!("snapshot");
        };
        let mut messages = vec![p.take_piece().unwrap().message()];
        assert!(!p.note_piece(1));
        apply(
            &mut p,
            &mut d,
            20,
            vec![
                json!(["set","n",{"text":"new"}]),
                json!(["scroll", "n", "page-down"]),
            ],
        );
        d.set_palette(json!({"dark":{"fg":"new"}})).unwrap();
        d.set_sheet("style", None).unwrap();
        assert!(p.queue_metadata(b't', &json!({"sf":"s","dark":{"fg":"new"}})));
        assert!(p.queue_metadata(b's', &json!({"sf":"s","name":"style","css":null})));
        assert!(p.peek_metadata().is_none());
        let Some(PreparedSend::Replay(retry)) = p
            .prepare_send(&d, &blobs, &confirmed, &json!({}), 0)
            .unwrap()
        else {
            panic!("held snapshot");
        };
        assert!(
            Arc::ptr_eq(&initial, &retry),
            "queue-full retries share the immutable batch"
        );
        assert_eq!(
            p.map_error(&json!({"ev":"error","sf":"outer","s":1,"op":1})),
            Err(ProjectionError::ErrorMapping)
        );
        while let Some(piece) = p.take_piece() {
            messages.push(piece.message());
            p.note_piece(1);
        }
        let mut view = TspDocument::new("outer");
        view.apply_frame(&serde_json::from_value(frame(&messages)).unwrap(), 0)
            .unwrap();
        assert_eq!(view.get("n", 0).unwrap()["p"]["text"], "old");
        assert_eq!(p.sent.front().unwrap().revision, 1);
        let snapshot_ops = frame(&messages)["ops"].as_array().unwrap().clone();
        let scroll = snapshot_ops
            .iter()
            .position(|op| op_name(op) == Some("scroll"))
            .unwrap();
        let source = p
            .map_error(&json!({"ev":"error","sf":"outer","s":1,"op":scroll}))
            .unwrap()
            .unwrap();
        assert_eq!(
            (source["s"].as_u64(), source["op"].as_u64()),
            (Some(10), Some(1))
        );
        let palette = p.peek_metadata().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&palette.body).unwrap()["dark"]["fg"],
            "new"
        );
        assert!(p.note_metadata());
        let sheet = p.peek_metadata().unwrap();
        assert!(serde_json::from_slice::<Value>(&sheet.body).unwrap()["css"].is_null());
        assert!(p.note_metadata());
        let (later, coverage) = send(&mut p, &d);
        assert!(!coverage.replay);
        assert_eq!(coverage.revision, 2);
        assert_eq!(
            frame(&later)["ops"],
            json!([["set","n",{"text":"new"}],["scroll","n","page-down"]])
        );
        assert_eq!(
            coverage.operations[0],
            [SourceOp {
                sf: "s".into(),
                s: 20,
                op: 0
            }]
        );
        assert_eq!(
            coverage.operations[1],
            [SourceOp {
                sf: "s".into(),
                s: 20,
                op: 1
            }]
        );
        view.apply_frame(&serde_json::from_value(frame(&later)).unwrap(), 0)
            .unwrap();
        assert_eq!(view.get("n", 0).unwrap()["p"]["text"], "new");
        assert_eq!(p.ack(coverage.sequence).unwrap(), 2);
        let mut newcomer = projection(2);
        let (snapshot, coverage) = send(&mut newcomer, &d);
        assert_eq!(coverage.revision, 2);
        assert!(coverage.operations.iter().all(Vec::is_empty));
        let mut fresh_view = TspDocument::new("outer");
        fresh_view
            .apply_frame(&serde_json::from_value(frame(&snapshot)).unwrap(), 0)
            .unwrap();
        assert_eq!(fresh_view.snapshot(0), view.snapshot(0));
        assert!(!snapshot.iter().any(|message| message.verb == b's'));
        assert_eq!(
            snapshot
                .iter()
                .filter(|message| message.verb == b't')
                .count(),
            1
        );
    }

    #[test]
    fn postcursor_reconcile_keeps_views_and_latest_canonical_tree() {
        let mut p = projection(1);
        let mut d = TspDocument::new("s");
        apply(
            &mut p,
            &mut d,
            1,
            vec![json!(["add","n","s",null,{"id":"n","k":"text"}])],
        );
        p.prepare_send(&d, &TspBlobStore::new(), &BTreeSet::new(), &json!({}), 0)
            .unwrap();
        p.note_piece(1);
        apply(&mut p, &mut d, 2, vec![json!(["scroll", "n", "line-down"])]);
        p.reconcile();
        apply(
            &mut p,
            &mut d,
            3,
            vec![
                json!(["del", "n"]),
                json!(["add","n","s",null,{"id":"n","k":"text","p":{"text":"latest"}}]),
                json!(["reveal", "n", "end"]),
            ],
        );
        while p.take_piece().is_some() {
            p.note_piece(1);
        }
        assert!(p.snapshot_needed);
        p.ack(1).unwrap();
        let (messages, coverage) = send(&mut p, &d);
        assert!(coverage.replay);
        assert_eq!(coverage.revision, 3);
        let ops = frame(&messages)["ops"].as_array().unwrap().clone();
        assert!(!ops.iter().any(|op| op_name(op) == Some("scroll")));
        assert_eq!(ops.last().unwrap(), &json!(["reveal", "n", "end"]));
        assert_eq!(
            coverage.operations.last().unwrap(),
            &[SourceOp {
                sf: "s".into(),
                s: 3,
                op: 2
            }]
        );
        let mut view = TspDocument::new("outer");
        // Replay contains resets for the admitted old snapshot.
        let first = Frame {
            sf: "outer".into(),
            s: 1,
            ops: vec![json!(["add","n","outer",null,{"id":"n","k":"text"}])],
        };
        view.apply_frame(&first, 0).unwrap();
        let applied = view
            .apply_frame(&serde_json::from_value(frame(&messages)).unwrap(), 0)
            .unwrap();
        assert!(applied.errors.is_empty());
        assert_eq!(view.get("n", 0).unwrap()["p"]["text"], "latest");
    }

    #[test]
    fn blocked_live_frame_keeps_newer_ops_separate() {
        let mut p = projection(2);
        let mut d = TspDocument::new("s");
        let (_, initial) = send(&mut p, &d);
        p.ack(initial.sequence).unwrap();
        apply(
            &mut p,
            &mut d,
            10,
            vec![json!(["set","s",{"text":"first"}])],
        );
        let Some(PreparedSend::Live(plan, coverage)) = p
            .prepare_send(&d, &TspBlobStore::new(), &BTreeSet::new(), &json!({}), 0)
            .unwrap()
        else {
            panic!("live frame");
        };
        apply(
            &mut p,
            &mut d,
            20,
            vec![json!(["set","s",{"text":"second"}])],
        );
        assert_eq!(
            p.map_error(&json!({"ev":"error","sf":"outer","s":coverage.sequence,"op":0})),
            Err(ProjectionError::ErrorMapping)
        );
        let Some(PreparedSend::Live(retry, _)) = p
            .prepare_send(&d, &TspBlobStore::new(), &BTreeSet::new(), &json!({}), 0)
            .unwrap()
        else {
            panic!("held live frame");
        };
        assert!(Arc::ptr_eq(&plan, &retry));
        assert_eq!(
            frame(&plan.messages)["ops"],
            json!([["set","outer",{"text":"first"}]])
        );
        assert!(p.note_piece(coverage.sequence));
        let source = p
            .map_error(&json!({"ev":"error","sf":"outer","s":coverage.sequence,"op":0}))
            .unwrap()
            .unwrap();
        assert_eq!(source["s"], 10);
        let (messages, newer) = send(&mut p, &d);
        assert_eq!(
            frame(&messages)["ops"],
            json!([["set","outer",{"text":"second"}]])
        );
        assert_eq!(
            newer.operations[0],
            [SourceOp {
                sf: "s".into(),
                s: 20,
                op: 0
            }]
        );
        assert_eq!(p.ack(newer.sequence).unwrap(), 2);
    }

    #[test]
    fn status_bar_stays_last_in_the_outer_dock() {
        fn bar(text: &str) -> Value {
            json!({"id":BAR_ID,"k":"col","c":[{"id":"rmux:bar:0","k":"status","c":[
                {"id":"rmux:bar:0:0","k":"seg","p":{"text":text}}]}]})
        }
        // Tern's side of the projection: every outer frame must apply cleanly.
        fn draw(p: &mut Projection, d: &TspDocument, view: &mut TspDocument) -> FrameCoverage {
            let (messages, coverage) = send(p, d);
            let applied = view
                .apply_frame(&serde_json::from_value(frame(&messages)).unwrap(), 0)
                .unwrap();
            assert!(applied.errors.is_empty(), "{:?}", applied.errors);
            p.ack(coverage.sequence).unwrap();
            coverage
        }
        fn dock(view: &TspDocument) -> Vec<String> {
            view.get("dock", 0).map_or_else(Vec::new, |dock| {
                dock["c"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|child| child["id"].as_str().unwrap().to_owned())
                    .collect()
            })
        }
        fn bar_sheets(messages: &[WireMessage]) -> usize {
            messages
                .iter()
                .filter(|m| m.verb == b's')
                .filter(|m| serde_json::from_slice::<Value>(&m.body).unwrap()["name"] == BAR_SHEET)
                .count()
        }
        let mut p = projection(2);
        p.bar_styles = true;
        let mut d = TspDocument::new("s");
        let mut view = TspDocument::new("outer");
        assert!(p.set_bar(Some(bar("0:edit*"))));
        assert!(!p.set_bar(Some(bar("0:edit*"))));
        let (messages, opened) = send(&mut p, &d);
        assert!(opened.replay);
        assert_eq!(bar_sheets(&messages), 1);
        assert_eq!(messages.last().unwrap().verb, b'f');
        view.apply_frame(&serde_json::from_value(frame(&messages)).unwrap(), 0)
            .unwrap();
        p.ack(opened.sequence).unwrap();
        assert_eq!(dock(&view), [BAR_ID]);
        assert_eq!(
            p.map_error(&json!({"ev":"error","sf":"outer","sheet":BAR_SHEET,"msg":"bad"})),
            Err(ProjectionError::StatusBar)
        );

        apply(
            &mut p,
            &mut d,
            3,
            vec![json!(["add","dock","s",null,{"id":"dock","k":"col","c":[
                {"id":"ed","k":"editor"}]}])],
        );
        draw(&mut p, &d, &mut view);
        assert_eq!(dock(&view), ["ed", BAR_ID]);

        apply(
            &mut p,
            &mut d,
            4,
            vec![json!(["add","st","dock",null,{"id":"st","k":"text"}])],
        );
        let appended = draw(&mut p, &d, &mut view);
        assert_eq!(dock(&view), ["ed", "st", BAR_ID]);
        assert!(appended.bar_ops.is_empty());

        p.set_bar(Some(bar("1:logs*")));
        let updated = draw(&mut p, &d, &mut view);
        assert!(updated.operations.iter().all(Vec::is_empty));
        assert_eq!(updated.revision, appended.revision);
        assert_eq!(dock(&view), ["ed", "st", BAR_ID]);
        assert_eq!(view.get("rmux:bar:0:0", 0).unwrap()["p"]["text"], "1:logs*");

        apply(&mut p, &mut d, 5, vec![json!(["del", "dock"])]);
        let (messages, removed) = send(&mut p, &d);
        assert_eq!(bar_sheets(&messages), 0);
        view.apply_frame(&serde_json::from_value(frame(&messages)).unwrap(), 0)
            .unwrap();
        assert_eq!(dock(&view), [BAR_ID]);
        let index = *removed.bar_ops.first().unwrap();
        assert_eq!(
            p.map_error(&json!({"ev":"error","sf":"outer","s":removed.sequence,"op":index})),
            Err(ProjectionError::StatusBar)
        );
        p.ack(removed.sequence).unwrap();

        let mut newcomer = projection(2);
        newcomer.set_bar(Some(bar("1:logs*")));
        let mut fresh_view = TspDocument::new("outer");
        draw(&mut newcomer, &d, &mut fresh_view);
        assert_eq!(fresh_view.snapshot(0), view.snapshot(0));

        p.set_bar(None);
        draw(&mut p, &d, &mut view);
        assert!(!view.has("dock"));
        p.set_bar(None);
        assert!(
            p.next_messages(
                &d,
                &mut TspBlobStore::new(),
                &BTreeSet::new(),
                &json!({}),
                0
            )
            .unwrap()
            .is_none()
        );
    }
}
