// Ported from tmux cmd-queue.c, cmd.c (shared command glue) @ 8f25579c
//! Helpers shared by the G21 command executors: queue item accessors and
//! byte-string error formatting. Keep this file free of command logic.

use crate::cmd::find::CmdFindState;
use crate::cmd::queue::{self, CmdReturn, QueueEvent, QueueStateFlags};
use crate::ids::{ClientId, QueueItemId};
use crate::server::Server;

/// `cmdq_get_client`.
pub fn item_client(server: &Server, item: QueueItemId) -> Option<ClientId> {
    server.queue.items.get(item).and_then(|i| i.client)
}
/// `cmdq_get_target_client`.
pub fn item_target_client(server: &Server, item: QueueItemId) -> Option<ClientId> {
    server.queue.items.get(item).and_then(|i| i.target_client)
}
/// `cmdq_get_target`.
pub fn item_target(server: &Server, item: QueueItemId) -> CmdFindState {
    server
        .queue
        .items
        .get(item)
        .map(|i| i.target)
        .unwrap_or_default()
}
/// `cmdq_get_source`.
pub fn item_source(server: &Server, item: QueueItemId) -> CmdFindState {
    server
        .queue
        .items
        .get(item)
        .map(|i| i.source)
        .unwrap_or_default()
}
/// `cmdq_get_current`.
pub fn item_current(server: &Server, item: QueueItemId) -> CmdFindState {
    server
        .queue
        .items
        .get(item)
        .and_then(|i| server.queue.states.get(i.state))
        .map(|s| s.current)
        .unwrap_or_default()
}
/// `cmdq_get_event`.
pub fn item_event(server: &Server, item: QueueItemId) -> QueueEvent {
    server
        .queue
        .items
        .get(item)
        .and_then(|i| server.queue.states.get(i.state))
        .map(|s| s.event)
        .unwrap_or_default()
}
/// `cmdq_get_flags`.
pub fn item_flags(server: &Server, item: QueueItemId) -> QueueStateFlags {
    server
        .queue
        .items
        .get(item)
        .and_then(|i| server.queue.states.get(i.state))
        .map(|s| s.flags)
        .unwrap_or_default()
}
/// Set the shared `current` state of the item (`cmd_find_copy_state(current, &fs)`).
pub fn set_item_current(server: &mut Server, item: QueueItemId, state: &CmdFindState) {
    if let Some(sid) = server.queue.items.get(item).map(|i| i.state)
        && let Some(st) = server.queue.states.get_mut(sid)
    {
        st.current = *state;
    }
}

/// `cmdq_error(item, fmt, ...)` with a prebuilt message; returns `CmdReturn::Error`.
pub fn fail(server: &mut Server, item: QueueItemId, message: impl AsRef<[u8]>) -> CmdReturn {
    queue::error(server, item, message.as_ref());
    CmdReturn::Error
}

/// Concatenate byte pieces (replacement for `xasprintf("%s%s", ...)`).
pub fn concat(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len()).sum());
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

/// `c->name` or "" (`cmd-wait-for.c:149-156`).
pub fn client_name(server: &Server, client: Option<ClientId>) -> &[u8] {
    client
        .and_then(|c| server.clients.get(c))
        .and_then(|c| c.name.as_deref())
        .unwrap_or(b"")
}
