// Ported from tmux input.c, server-client.c, screen-write.c @ 8f25579c
use super::{
    blobs::TspBlobStore,
    surface::{SurfaceId, SurfaceStore},
    wire::{DisplayContract, WireMessage},
};
use crate::{
    ids::{ClientId, PaneId, TimerId},
    model::Server,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Renderer {
    #[default]
    Ansi,
    Switching,
    Native,
    Detached,
}
pub struct SwitchState {
    pub requested_epoch: u64,
    pub probing_epoch: Option<u64>,
    pub native: bool,
    pub timer: Option<(TimerId, u64)>,
    pub failed: bool,
    pub surface_revision: Option<(SurfaceId, u64)>,
    pub probe_observed: bool,
}
#[derive(Clone, Debug)]
pub struct DrawDebt {
    pub revision: u64,
    pub sequence: u64,
}
pub type DeferredUi = Box<dyn FnOnce(&mut Server)>;
pub struct PaneTspState {
    pub epoch: u64,
    pub generation: u64,
    pub registered: bool,
    pub renderer: Renderer,
    pub contract: Option<DisplayContract>,
    pub leader: Option<ClientId>,
    pub switch: Option<SwitchState>,
    pub surfaces: SurfaceStore,
    pub blobs: TspBlobStore,
    pub debts: BTreeMap<SurfaceId, VecDeque<DrawDebt>>,
    pub program_hello: serde_json::Value,
    pub replay_failed: bool,
    pub held: VecDeque<(Option<ClientId>, Vec<u8>)>,
    pub held_bytes: usize,
    pub next_anchor: u64,
    pub chunk: Option<super::transport::ChunkSequence>,
    pub chunk_timer: Option<(TimerId, u64)>,
    pub answers_enabled: bool,
    pub program_exited: bool,
    pub pending_input_bytes: usize,
    pub ui_pending: bool,
    pub deferred_ui: Vec<DeferredUi>,
    pub deferred_input: Vec<DeferredUi>,
    pub reported_view: Option<serde_json::Value>,
    pub reported_theme: Option<bool>,
    pub reported_motion: Option<bool>,
}
impl Default for PaneTspState {
    fn default() -> Self {
        Self {
            epoch: 1,
            generation: 1,
            registered: false,
            renderer: Renderer::Ansi,
            contract: None,
            leader: None,
            switch: None,
            surfaces: SurfaceStore::new(),
            blobs: TspBlobStore::new(),
            debts: BTreeMap::new(),
            program_hello: serde_json::Value::Null,
            replay_failed: false,
            held: VecDeque::new(),
            held_bytes: 0,
            pending_input_bytes: 0,
            next_anchor: 0,
            chunk: None,
            chunk_timer: None,
            answers_enabled: false,
            program_exited: false,
            ui_pending: false,
            deferred_ui: Vec::new(),
            deferred_input: Vec::new(),
            reported_view: None,
            reported_theme: None,
            reported_motion: None,
        }
    }
}
impl PaneTspState {
    pub fn format(&self) -> &'static str {
        match self.renderer {
            Renderer::Ansi => "ansi",
            Renderer::Switching => "switching",
            Renderer::Native => "native",
            Renderer::Detached => "detached",
        }
    }
}
pub fn now_ms() -> u64 {
    let n = rmux_util::time::Timestamp::now();
    (n.sec as u64).saturating_mul(1000) + (n.usec as u64) / 1000
}
pub fn reply(server: &mut Server, pane: PaneId, message: WireMessage) {
    let _ = crate::model::pane_input::queue_reply(server, pane, message.encode(), true, now_ms());
}
pub fn event(server: &mut Server, pane: PaneId, value: &serde_json::Value) {
    reply(server, pane, WireMessage::json(b'e', value));
}
pub use super::client_runtime::{
    client_message, client_protocol_fault, client_sentinel, client_sync, probe_client,
};
pub use super::contract::{recompute, viewers};
pub use super::input::{
    client_read_bound, defer_cell_ui, hold_bytes, native_client, native_pane_client,
    pane_cell_ready, release_input,
};
pub use super::lifetime::{
    drain_anchors, pane_alternate, pane_exited, pane_prompt, pane_reset, pane_respawn,
};
pub use super::pane_message::pane_message;
pub use super::project::{close_projection, project_pending, restore_grid};
pub fn refresh_client(server: &mut Server, client: ClientId) {
    close_projection(server, client);
    super::client_runtime::probe_client(server, client);
    super::contract::refresh(server, client);
}
