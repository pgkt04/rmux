// Ported from tmux input.c, window.c, screen.c @ 8f25579c
use super::{
    broker::{Renderer, event, reply},
    surface::SurfaceChange,
    wire::WireMessage,
};
use crate::{ids::PaneId, model::Server};
pub fn apply_change(server: &mut Server, pane: PaneId, change: SurfaceChange) {
    let answers = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .is_some_and(|s| s.answers_enabled && !s.program_exited);
    if let Some(p) = server.panes.get_mut(pane) {
        for removed in &change.removed {
            if let Some(anchor) = removed.anchor {
                p.base
                    .remove_surface_anchor(rmux_emu::grid::SurfaceAnchorId(anchor));
            }
            if let Some(s) = p.tsp.as_mut() {
                s.debts.remove(&removed.id);
                s.blobs.remove_owner(&removed.wire_id);
            }
        }
        if let Some(s) = p.tsp.as_mut() {
            for evicted in &change.evicted {
                if let Some(surface) = s.surfaces.get(evicted.surface) {
                    s.blobs
                        .set_references(&surface.wire_id, surface.document.blob_references());
                }
            }
        }
        if let Some(s) = p.tsp.as_mut() {
            for closed in &change.closed {
                s.debts.remove(closed);
            }
        }
    }
    let clients: Vec<_> = server.client_order.iter().copied().collect();
    for client in &clients {
        let affected = server
            .clients
            .get(*client)
            .and_then(|c| c.tsp.projection.as_ref())
            .is_some_and(|p| {
                p.pane == pane
                    && change
                        .evicted
                        .iter()
                        .any(|e| p.outer.ends_with(&format!(":s{}", e.surface.0)))
            });
        if affected {
            super::project::close_projection(server, *client);
        }
    }
    if answers {
        for gone in &change.gone {
            if gone.listen {
                event(server, pane, &gone.json());
            }
        }
    }
    for client in clients {
        super::project::project_pending(server, client);
    }
}
pub fn drain_anchors(server: &mut Server, pane: PaneId) {
    let Some(p) = server.panes.get_mut(pane) else {
        return;
    };
    let removals: Vec<_> = p.base.drain_surface_anchor_removals().collect();
    let Some(s) = p.tsp.as_mut() else { return };
    let mut change = SurfaceChange::default();
    for anchor in removals {
        change.append(s.surfaces.remove_anchor(anchor.0));
    }
    apply_change(server, pane, change);
}
fn end_reader(server: &mut Server, pane: PaneId, kind: u8) {
    super::pane_message::cancel_chunk_timer(server, pane);
    let Some(p) = server.panes.get_mut(pane) else {
        return;
    };
    let Some(s) = p.tsp.as_mut() else { return };
    let was_native = matches!(s.renderer, Renderer::Native | Renderer::Detached);
    s.registered = false;
    s.answers_enabled = false;
    s.program_exited = kind == 2;
    s.chunk = None;
    s.debts.clear();
    s.generation = s.generation.wrapping_add(1);
    s.epoch = s.epoch.checked_add(1).expect("TSP epoch exhaustion");
    s.reported_view = None;
    s.reported_theme = None;
    s.reported_motion = None;
    if kind != 2 {
        s.replay_failed = false;
        s.program_hello = serde_json::Value::Null;
        s.leader = None;
    }
    if let Some(switch) = s.switch.take() {
        if let Some((timer, id)) = switch.timer {
            server.event_loop.cancel(timer);
            server.deferred.remove(&id);
        }
    }
    s.renderer = if kind == 2 && was_native {
        Renderer::Native
    } else {
        Renderer::Ansi
    };
    if kind != 2 {
        s.contract = None;
    }
    let change = match kind {
        0 => s.surfaces.prompt(),
        1 => s.surfaces.reset(),
        2 => s.surfaces.process_exit(),
        _ => s.surfaces.respawn(),
    };
    let callbacks = std::mem::take(&mut s.deferred_ui);
    s.ui_pending = false;
    let clients: Vec<_> = server.client_order.iter().copied().collect();
    for id in clients {
        if let Some(c) = server
            .clients
            .get_mut(id)
            .filter(|c| c.tsp.transition_pane == Some(pane))
        {
            c.tsp.transition_pane = None;
        }
        if server
            .clients
            .get(id)
            .and_then(|c| c.tsp.projection.as_ref())
            .is_some_and(|p| p.pane == pane)
        {
            super::project::close_projection(server, id);
        }
        super::input::client_read_bound(server, id);
    }
    apply_change(server, pane, change);
    super::input::release_input(server, pane);
    for callback in callbacks {
        callback(server);
    }
}
pub fn pane_prompt(server: &mut Server, pane: PaneId) {
    end_reader(server, pane, 0)
}
pub fn pane_reset(server: &mut Server, pane: PaneId) {
    end_reader(server, pane, 1)
}
pub fn pane_exited(server: &mut Server, pane: PaneId) {
    end_reader(server, pane, 2)
}
pub fn pane_respawn(server: &mut Server, pane: PaneId) {
    end_reader(server, pane, 3)
}
pub fn pane_alternate(server: &mut Server, pane: PaneId, entering: bool) {
    let Some(s) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let change = if entering {
        s.surfaces.alternate_enter()
    } else {
        s.surfaces.alternate_exit()
    };
    apply_change(server, pane, change);
}
pub fn protocol_error(server: &mut Server, pane: PaneId, error: &str) {
    protocol_error_for(server, pane, error, None);
}
pub fn protocol_error_for(
    server: &mut Server,
    pane: PaneId,
    error: &str,
    context: Option<&serde_json::Value>,
) {
    let Some(state) = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .filter(|s| s.answers_enabled && !s.program_exited)
    else {
        return;
    };
    let sf = context.and_then(|v| v["sf"].as_str());
    if sf.is_some_and(|sf| {
        state
            .surfaces
            .find(sf)
            .and_then(|id| state.surfaces.get(id))
            .is_some_and(|s| !s.listen)
    }) {
        return;
    }
    let mut value = serde_json::json!({"ev":"error","msg":error});
    if let Some(context) = context {
        for name in ["sf", "s", "op", "sheet", "id"] {
            if let Some(field) = context.get(name).filter(|v| !v.is_null()) {
                value[name] = field.clone();
            }
        }
    }
    reply(server, pane, WireMessage::json(b'e', &value));
}
