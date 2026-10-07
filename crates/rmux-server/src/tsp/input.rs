// Ported from tmux server-client.c, window.c @ 8f25579c
use super::broker::{PaneTspState, Renderer};
use crate::{
    ids::{ClientId, PaneId},
    model::Server,
};
pub const INPUT_LIMIT: usize = 64 * 1024;
pub fn hold_bytes(
    server: &mut Server,
    pane: PaneId,
    client: Option<ClientId>,
    bytes: &[u8],
) -> bool {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return false;
    };
    if state.renderer != Renderer::Switching {
        return false;
    }
    state.held_bytes += bytes.len();
    state.held.push_back((client, bytes.to_vec()));
    let ids: Vec<_> = server
        .client_order
        .iter()
        .copied()
        .filter(|id| input_pane(server, *id) == Some(pane))
        .collect();
    for id in ids {
        client_read_bound(server, id);
    }
    true
}
pub fn release_input(server: &mut Server, pane: PaneId) {
    let Some(p) = server.panes.get_mut(pane) else {
        return;
    };
    let Some(state) = p.tsp.as_mut() else { return };
    while let Some((_, bytes)) = state.held.pop_front() {
        p.output.extend_from_slice(&bytes);
    }
    state.held_bytes = 0;
    let deferred = std::mem::take(&mut state.deferred_input);
    for callback in deferred {
        callback(server);
    }
    let ids: Vec<_> = server.client_order.iter().copied().collect();
    for id in ids {
        client_read_bound(server, id);
        crate::server::event_loop::schedule_deferred(
            server,
            std::time::Duration::ZERO,
            Box::new(move |server| crate::client::tty_io::drain_input(server, id)),
        );
    }
}
fn input_pane(server: &Server, id: ClientId) -> Option<PaneId> {
    super::contract::client_pane(server, id)
}
fn ordinary_len(tty: &rmux_tty::tty::Tty) -> usize {
    if tty.protocol_partial() {
        0
    } else {
        tty.input_len()
    }
}
fn ordinary_buffered(server: &Server, pane: PaneId) -> usize {
    server
        .client_order
        .iter()
        .copied()
        .filter(|id| input_pane(server, *id) == Some(pane))
        .filter_map(|id| server.clients.get(id)?.tty.as_ref())
        .map(ordinary_len)
        .sum()
}
pub fn input_available(server: &Server, pane: PaneId) -> usize {
    let Some(state) = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .filter(|s| s.renderer == Renderer::Switching)
    else {
        return usize::MAX;
    };
    INPUT_LIMIT.saturating_sub(
        state.held_bytes + state.pending_input_bytes + ordinary_buffered(server, pane),
    )
}
pub fn reserve_input(server: &mut Server, pane: PaneId, bytes: usize) -> bool {
    if bytes > input_available(server, pane) {
        return false;
    }
    if let Some(state) = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .filter(|s| s.renderer == Renderer::Switching)
    {
        state.pending_input_bytes += bytes;
    }
    true
}
pub fn reserve_decoded_input(
    server: &mut Server,
    pane: PaneId,
    client: ClientId,
    consumed: usize,
    encoded_bytes: usize,
) -> bool {
    let own = server
        .clients
        .get(client)
        .and_then(|c| c.tty.as_ref())
        .map_or(0, ordinary_len);
    let buffered = ordinary_buffered(server, pane).saturating_sub(consumed.min(own));
    let Some(state) = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .filter(|s| s.renderer == Renderer::Switching)
    else {
        return true;
    };
    if encoded_bytes
        > INPUT_LIMIT.saturating_sub(state.held_bytes + state.pending_input_bytes + buffered)
    {
        return false;
    }
    state.pending_input_bytes += encoded_bytes;
    true
}
pub fn defer_input(server: &mut Server, pane: PaneId, callback: super::broker::DeferredUi) -> bool {
    let Some(state) = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .filter(|s| s.renderer == Renderer::Switching)
    else {
        return false;
    };
    state.deferred_input.push(callback);
    true
}
pub fn release_reservation(server: &mut Server, pane: PaneId, bytes: usize) {
    if let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) {
        state.pending_input_bytes = state.pending_input_bytes.saturating_sub(bytes);
    }
}
pub fn client_can_decode(server: &Server, id: ClientId) -> bool {
    let Some(pane) = input_pane(server, id) else {
        return true;
    };
    let Some(state) = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .filter(|s| s.renderer == Renderer::Switching)
    else {
        return true;
    };
    server
        .clients
        .get(id)
        .and_then(|c| c.tty.as_ref())
        .is_some_and(|tty| tty.protocol_partial())
        || state.held_bytes + state.pending_input_bytes < INPUT_LIMIT
}
pub fn client_read_bound(server: &mut Server, id: ClientId) {
    let budget = input_pane(server, id).and_then(|pane| {
        let state = server
            .panes
            .get(pane)?
            .tsp
            .as_ref()
            .filter(|s| s.renderer == Renderer::Switching)?;
        let buffered = ordinary_buffered(server, pane);
        let own = server.clients.get(id)?.tty.as_ref()?.input_len();
        let remaining =
            INPUT_LIMIT.saturating_sub(state.held_bytes + state.pending_input_bytes + buffered);
        Some((own + remaining, remaining == 0))
    });
    if let Some(tty) = server.clients.get_mut(id).and_then(|c| c.tty.as_mut()) {
        let protocol = tty.protocol_partial();
        tty.set_read_limit(if protocol {
            None
        } else {
            budget.map(|(limit, _)| limit)
        });
        tty.set_read_paused(!protocol && budget.is_some_and(|(_, full)| full));
    }
}
pub fn pane_cell_ready(server: &Server, pane: PaneId) -> bool {
    server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .is_none_or(|s| s.stock || s.renderer == Renderer::Ansi)
}
pub fn defer_cell_ui(
    server: &mut Server,
    pane: PaneId,
    callback: Box<dyn FnOnce(&mut Server)>,
) -> bool {
    if pane_cell_ready(server, pane) {
        return false;
    }
    if server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .is_some_and(|s| s.program_exited)
    {
        if let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) {
            state.renderer = Renderer::Ansi;
            state.contract = None;
        }
        let clients: Vec<_> = server
            .client_order
            .iter()
            .copied()
            .filter(|id| {
                server
                    .clients
                    .get(*id)
                    .and_then(|c| c.tsp.projection.as_ref())
                    .is_some_and(|p| p.pane == pane)
            })
            .collect();
        for client in clients {
            super::project::close_projection(server, client);
        }
        callback(server);
        return true;
    }
    let Some(p) = server.panes.get_mut(pane) else {
        return false;
    };
    let s = p.tsp.get_or_insert_with(PaneTspState::default);
    s.ui_pending = true;
    s.deferred_ui.push(callback);
    super::contract::recompute(server);
    true
}
pub fn native_client(server: &Server, id: ClientId) -> bool {
    server.clients.get(id).is_some_and(|c| {
        c.tsp.transition_pane.is_some()
            || c.tsp
                .projection
                .as_ref()
                .is_some_and(|p| p.is_open() && !p.failed)
    })
}
pub fn native_pane_client(client: &crate::client::Client, pane: PaneId) -> bool {
    client.tsp.transition_pane == Some(pane)
        || client
            .tsp
            .projection
            .as_ref()
            .is_some_and(|p| p.pane == pane && p.is_open() && !p.failed)
}
