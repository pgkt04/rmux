// Ported from tmux tty.c, server-client.c @ 8f25579c
use super::{
    broker::{Renderer, now_ms},
    client::RequestOwner,
    projection::Projection,
    surface::SurfaceMode,
    wire::WireMessage,
};
use crate::{
    client::ClientFlags,
    ids::{ArenaId, ClientId, PaneId},
    model::Server,
};
use rmux_tty::{
    keys::Da1Owner,
    tty::{ProtocolTransaction, QueueFull, Tty},
};

pub fn restore_grid(server: &mut Server, id: ClientId) {
    if let Some(c) = server.clients.get_mut(id) {
        c.flags.insert(ClientFlags::ALLREDRAWFLAGS);
        c.redraw_scene = None;
        if let Some(t) = c.tty.as_mut() {
            t.invalidate(&mut server.tparm);
        }
    }
}
/// DECSC right after an inline `o`, where Tern leaves the cursor: the row under
/// the surface's anchor.
const SAVE_CURSOR: &[u8] = b"\x1b7";
/// Tern keeps a closed inline surface's anchor row. Cell output may have moved
/// the cursor and scroll region since the open, so reset the region, return to
/// the saved cursor, delete the anchor row, then go back to the alternate
/// screen of the cell view.
fn back_to_grid(tty: &Tty) -> Vec<u8> {
    let mut bytes = b"\x1b[r\x1b8\x1b[A\x1b[M".to_vec();
    bytes.extend(tty.alternate_screen(true));
    bytes
}
pub fn close_projection(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    if let Some(mut projection) = c.tsp.projection.take() {
        if let Some(tty) = c.tty.as_mut() {
            tty.cancel_protocol(projection.generation);
            tty.set_teardown(Vec::new());
            let mut close = projection.close().encode();
            if projection.inline {
                close.extend(back_to_grid(tty));
            }
            let _ = tty.close_protocol(ProtocolTransaction::new(close).teardown());
        }
        restore_grid(server, id);
    }
}
pub fn project_pending(server: &mut Server, id: ClientId) {
    let Some(pane) = super::contract::client_pane(server, id) else {
        close_projection(server, id);
        return;
    };
    if !super::contract::viewers(server, pane).contains(&id) {
        close_projection(server, id);
        return;
    }
    let selected = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .filter(|s| matches!(s.renderer, Renderer::Native | Renderer::Detached))
        .and_then(|s| {
            s.surfaces
                .selected()
                .map(|sf| (sf.id, sf.wire_id.clone(), sf.mode == SurfaceMode::Inline))
        });
    let Some((handle, logical, inline)) = selected else {
        close_projection(server, id);
        return;
    };
    if server.clients.get(id).and_then(|c| c.tsp.hello()).is_none() {
        close_projection(server, id);
        return;
    }
    if super::contract::display_contract(server, pane).0.is_none() {
        close_projection(server, id);
        return;
    }
    let replace = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.projection.as_ref())
        .is_none_or(|p| {
            p.pane != pane || p.logical != logical || !p.outer.ends_with(&format!(":s{}", handle.0))
        });
    if replace {
        close_projection(server, id);
        let hello = server
            .panes
            .get(pane)
            .and_then(|p| p.tsp.as_ref())
            .map(|s| s.program_hello.clone())
            .unwrap_or_default();
        let c = server.clients.get_mut(id).expect("checked client");
        c.tsp.projection_generation = c.tsp.projection_generation.wrapping_add(1);
        let generation = c.tsp.projection_generation;
        let outer = format!(
            "rmux:c{}g{}:p{}:v{}:s{}",
            id.parts().0,
            c.tsp.generation,
            pane.parts().0,
            generation,
            handle.0
        );
        let credits = c.tsp.hello().map_or(1, |h| h.credits.min(2));
        let token = c.tsp.request(RequestOwner::Replay {
            pane,
            projection: generation,
        });
        if let Some(tty) = c.tty.as_mut() {
            if tty
                .queue_da1(
                    Da1Owner::Token(token),
                    ProtocolTransaction::new(WireMessage::json(b'q', &hello).encode()),
                )
                .is_err()
            {
                return;
            }
        }
        let mut projection = Projection::new(pane, logical, outer, generation, credits);
        projection.inline = inline;
        c.tsp.projection = Some(projection);
        super::status_bar::refresh(server, id);
    }
    send_pending(server, id, pane, handle);
}

/// Canonical `t`/`s`. Each open projection of `handle` keeps the latest palette
/// and each sheet (including a deletion) until that exact message is admitted.
/// Nothing is dropped when the tty queue is full. Retry is `project_pending`.
pub fn queue_metadata(
    server: &mut Server,
    pane: PaneId,
    handle: super::surface::SurfaceId,
    verb: u8,
    value: &serde_json::Value,
) {
    let logical = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .and_then(|s| s.surfaces.get(handle))
        .map(|s| s.document.surface.clone());
    let Some(logical) = logical else { return };
    let ids: Vec<ClientId> = server
        .client_order
        .iter()
        .copied()
        .filter(|id| {
            server
                .clients
                .get(*id)
                .and_then(|c| c.tsp.projection.as_ref())
                .is_some_and(|p| p.pane == pane && p.logical == logical && !p.failed)
        })
        .collect();
    for id in ids {
        if let Some(p) = server
            .clients
            .get_mut(id)
            .and_then(|c| c.tsp.projection.as_mut())
        {
            let _ = p.queue_metadata(verb, value);
        }
        send_pending(server, id, pane, handle);
    }
}

fn send_pending(
    server: &mut Server,
    id: ClientId,
    pane: PaneId,
    handle: super::surface::SurfaceId,
) {
    let limit = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.hello())
        .map_or(65536, |h| h.apc);
    let generation = server
        .clients
        .get(id)
        .map(|c| c.tsp.projection_generation)
        .unwrap_or(0);
    let confirmed = server
        .clients
        .get(id)
        .map(|c| c.tsp.confirmed_blobs.clone())
        .unwrap_or_default();
    let outcome = {
        let Some(state) = server.panes.get(pane).and_then(|p| p.tsp.as_ref()) else {
            return;
        };
        let Some(surface) = state.surfaces.get(handle) else {
            return;
        };
        let mut open = surface.open.clone();
        if state.program_exited {
            open["listen"] = false.into();
        }
        let now = now_ms();
        let Some(projection) = server
            .clients
            .get_mut(id)
            .and_then(|c| c.tsp.projection.as_mut())
        else {
            return;
        };
        projection.prepare_send(&surface.document, &state.blobs, &confirmed, &open, now)
    };
    let prepared = match outcome {
        Ok(prepared) => prepared,
        Err(error) => {
            if matches!(error, super::projection::ProjectionError::Replay(_)) {
                if let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) {
                    state.replay_failed = true;
                }
            }
            if let Some(c) = server.clients.get_mut(id) {
                c.tsp.diagnostic = Some(error.to_string());
            }
            super::client_runtime::client_protocol_fault(server, id);
            return;
        }
    };
    if prepared.is_none() {
        send_metadata(server, id, limit, generation);
        return;
    }
    // Old snapshot dependencies and its frame finish before newer metadata.
    while let Some(piece) = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.projection.as_ref())
        .and_then(|p| p.take_piece())
    {
        let sequence = server
            .clients
            .get(id)
            .and_then(|c| c.tsp.projection.as_ref())
            .and_then(|p| p.pending_sequence())
            .unwrap_or(0);
        let token = format!("{sequence}-{}", piece.token());
        let opening = piece.is_open();
        let leave_alternate = opening
            && server
                .clients
                .get(id)
                .and_then(|c| c.tsp.projection.as_ref())
                .is_some_and(|p| p.inline);
        let transaction = match piece.stream(limit, &token) {
            Ok((bytes, chunks)) => {
                match server.clients.get(id).and_then(|c| c.tty.as_ref()) {
                    // An inline surface on the alternate screen is an error. The
                    // saved cursor marks the row under the anchor for the close.
                    Some(tty) if leave_alternate => {
                        let leave = tty.alternate_screen(false);
                        let size = leave.len() + bytes + SAVE_CURSOR.len();
                        let chunks = std::iter::once(leave)
                            .chain(chunks)
                            .chain(std::iter::once(SAVE_CURSOR.to_vec()));
                        ProtocolTransaction::stream(size, chunks)
                    }
                    _ => ProtocolTransaction::stream(bytes, chunks),
                }
                .projection(generation)
            }
            Err(error) => {
                if let Some(c) = server.clients.get_mut(id) {
                    c.tsp.diagnostic = Some(error);
                }
                super::client_runtime::client_protocol_fault(server, id);
                return;
            }
        };
        let queued = server.clients.get_mut(id).and_then(|c| {
            let tty = c.tty.as_mut()?;
            if opening {
                let mode = tty.mode() & !rmux_emu::screen::ScreenMode::ALL_MOUSE_MODES;
                tty.update_mode(&mut server.tparm, mode, None);
            }
            let queued = tty.queue_protocol(transaction);
            if queued.is_ok() && leave_alternate {
                let outer = &c.tsp.projection.as_ref()?.outer;
                let mut teardown =
                    WireMessage::json(b'x', &serde_json::json!({"id": outer, "keep": false}))
                        .encode();
                teardown.extend(back_to_grid(tty));
                tty.set_teardown(teardown);
            }
            Some(queued)
        });
        match queued {
            Some(Ok(())) => {
                let finished = server
                    .clients
                    .get_mut(id)
                    .and_then(|c| c.tsp.projection.as_mut())
                    .is_some_and(|p| p.note_piece(sequence));
                if finished {
                    send_metadata(server, id, limit, generation);
                    break;
                }
            }
            Some(Err(QueueFull)) | None => break,
        }
    }
}
fn send_metadata(server: &mut Server, id: ClientId, limit: usize, generation: u64) {
    while let Some(message) = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.projection.as_ref())
        .and_then(|p| p.peek_metadata())
    {
        let transaction = match message.chunks(limit, &format!("m{}", message.verb)) {
            Ok(chunks) => ProtocolTransaction::chunks(chunks).projection(generation),
            Err(error) => {
                if let Some(c) = server.clients.get_mut(id) {
                    c.tsp.diagnostic = Some(error);
                }
                super::client_runtime::client_protocol_fault(server, id);
                return;
            }
        };
        let queued = server
            .clients
            .get_mut(id)
            .and_then(|c| c.tty.as_mut())
            .map(|tty| tty.queue_protocol(transaction));
        match queued {
            Some(Ok(())) => {
                let _ = server
                    .clients
                    .get_mut(id)
                    .and_then(|c| c.tsp.projection.as_mut())
                    .is_some_and(|p| p.note_metadata());
            }
            Some(Err(QueueFull)) | None => return,
        }
    }
}
