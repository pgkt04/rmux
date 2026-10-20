// Ported from tmux tty.c, server-client.c @ 8f25579c
use super::{
    broker::{Renderer, now_ms},
    client::{ClientTspState, RequestOwner},
    projection::Projection,
    surface::{SurfaceId, SurfaceMode},
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
/// `x` for `projection`; an inline one also gives the cell view its screen back.
fn close_bytes(projection: &Projection, tty: &Tty) -> Vec<u8> {
    let mut bytes = WireMessage::json(
        b'x',
        &serde_json::json!({"id": projection.outer, "keep": false}),
    )
    .encode();
    if projection.inline {
        bytes.extend(back_to_grid(tty));
    }
    bytes
}
/// What `Tty::stop` sends for the open projections: the screen first, then
/// the inline surface under it.
fn teardown(tsp: &ClientTspState, tty: &Tty) -> Vec<u8> {
    tsp.projection
        .iter()
        .chain(&tsp.covered)
        .flat_map(|projection| close_bytes(projection, tty))
        .collect()
}
fn shows(projection: &Projection, pane: PaneId, logical: &str, handle: SurfaceId) -> bool {
    projection.pane == pane
        && projection.logical == logical
        && projection.outer.ends_with(&format!(":s{}", handle.0))
}
pub fn close_projection(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    if c.tsp.projection.is_none() && c.tsp.covered.is_none() {
        return;
    }
    let mut closing: Vec<_> = c.tsp.projection.take().into_iter().collect();
    closing.extend(c.tsp.covered.take());
    if let Some(tty) = c.tty.as_mut() {
        if !tty.flags().contains(rmux_tty::tty::TtyFlags::STARTED) {
            return;
        }
        let mut close = Vec::new();
        for mut projection in closing {
            tty.cancel_protocol(projection.generation);
            projection.close();
            close.extend(close_bytes(&projection, tty));
        }
        tty.set_teardown(Vec::new());
        let _ = tty.close_protocol(ProtocolTransaction::new(close).teardown());
    }
    restore_grid(server, id);
}
/// Close the screen projection over `covered` and leave the inline one as it
/// is: the host shows it again with no resend. False when the close does not
/// fit the queue.
fn close_over_covered(server: &mut Server, id: ClientId) -> bool {
    let Some(c) = server.clients.get_mut(id) else {
        return false;
    };
    let (Some(tty), Some(screen)) = (c.tty.as_mut(), c.tsp.projection.as_ref()) else {
        return false;
    };
    let close = ProtocolTransaction::new(close_bytes(screen, tty)).teardown();
    let generation = screen.generation;
    tty.cancel_protocol(generation);
    if tty.queue_protocol(close).is_err() {
        return false;
    }
    c.tsp.projection = None;
    let bytes = teardown(&c.tsp, tty);
    tty.set_teardown(bytes);
    true
}
/// The open inline projection of `pane` stays live under a screen surface
/// while the program's inline surface does.
fn coverable(server: &Server, projection: &Projection, pane: PaneId) -> bool {
    let Some(handle) = super::client_runtime::outer_surface(&projection.outer) else {
        return false;
    };
    projection.inline
        && projection.is_open()
        && projection.pane == pane
        && server
            .panes
            .get(pane)
            .and_then(|p| p.tsp.as_ref())
            .is_some_and(|state| {
                state.surfaces.get(handle).is_some_and(|surface| {
                    surface.is_open()
                        && surface.mode == SurfaceMode::Inline
                        && surface.buffer == state.surfaces.buffer()
                        && surface.wire_id == projection.logical
                })
            })
}
pub fn project_pending(server: &mut Server, id: ClientId) {
    let Some(pane) = super::contract::client_pane(server, id) else {
        close_projection(server, id);
        return;
    };
    if !super::contract::native_viewers(server, pane).contains(&id) {
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
    if !super::contract::has_display_contract(server, pane) {
        close_projection(server, id);
        return;
    }
    let Some(c) = server.clients.get(id) else {
        return;
    };
    let started = c
        .tty
        .as_ref()
        .is_some_and(|tty| tty.flags().contains(rmux_tty::tty::TtyFlags::STARTED));
    let current = c
        .tsp
        .projection
        .as_ref()
        .is_some_and(|p| shows(p, pane, &logical, handle));
    let uncover = c
        .tsp
        .covered
        .as_ref()
        .is_some_and(|p| shows(p, pane, &logical, handle) && coverable(server, p, pane));
    let keep_covered = !inline
        && c.tsp
            .covered
            .as_ref()
            .is_some_and(|p| coverable(server, p, pane));
    let cover = !inline
        && c.tsp.covered.is_none()
        && c.tsp
            .projection
            .as_ref()
            .is_some_and(|p| coverable(server, p, pane));
    if !current {
        if started && uncover && close_over_covered(server, id) {
            let c = server.clients.get_mut(id).expect("checked client");
            c.tsp.projection = c.tsp.covered.take();
            super::status_bar::refresh(server, id);
            send_pending(server, id, pane, handle);
            super::client_runtime::release_drawn(server, pane);
            return;
        }
        if started && cover {
            let c = server.clients.get_mut(id).expect("checked client");
            c.tsp.covered = c.tsp.projection.take();
        } else if !(started && keep_covered && close_over_covered(server, id)) {
            close_projection(server, id);
        }
        if !open_projection(server, id, pane, logical, handle, inline) {
            return;
        }
    }
    send_pending(server, id, pane, handle);
}
/// A new outer surface for `handle`, sent from the next `send_pending`.
fn open_projection(
    server: &mut Server,
    id: ClientId,
    pane: PaneId,
    logical: String,
    handle: SurfaceId,
    inline: bool,
) -> bool {
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
    if c.tsp.projection_hello.as_ref() != Some(&hello) {
        let token = c.tsp.request(RequestOwner::Replay {
            pane,
            projection: generation,
        });
        if c.tty.as_mut().is_none_or(|tty| {
            tty.queue_da1(
                Da1Owner::Token(token),
                ProtocolTransaction::new(WireMessage::json(b'q', &hello).encode()),
            )
            .is_err()
        }) {
            c.tsp.requests.pop_back();
            return false;
        }
        c.tsp.projection_hello = Some(hello);
    }
    let mut projection = Projection::new(pane, logical, outer, generation, credits);
    projection.inline = inline;
    c.tsp.projection = Some(projection);
    super::status_bar::refresh(server, id);
    true
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
            server.clients.get(*id).is_some_and(|c| {
                c.tsp
                    .projection
                    .iter()
                    .chain(&c.tsp.covered)
                    .any(|p| p.pane == pane && p.logical == logical && !p.failed)
            })
        })
        .collect();
    for id in ids {
        let Some(c) = server.clients.get_mut(id) else {
            continue;
        };
        let shown = |p: &&mut Projection| p.pane == pane && p.logical == logical && !p.failed;
        if let Some(covered) = c.tsp.covered.as_mut().filter(shown) {
            let _ = covered.queue_metadata(verb, value);
        }
        if let Some(p) = c.tsp.projection.as_mut().filter(shown) {
            let _ = p.queue_metadata(verb, value);
            send_pending(server, id, pane, handle);
        }
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
        .and_then(|c| c.tsp.projection.as_ref())
        .map_or(0, |p| p.generation);
    let outcome = {
        let Some(state) = server.panes.get(pane).and_then(|p| p.tsp.as_ref()) else {
            return;
        };
        let Some(surface) = state.surfaces.get(handle) else {
            return;
        };
        let now = now_ms();
        let Some(client) = server.clients.get_mut(id) else {
            return;
        };
        let Some(projection) = client.tsp.projection.as_mut() else {
            return;
        };
        projection.prepare_send(
            &surface.document,
            &state.blobs,
            &client.tsp.confirmed_blobs,
            &surface.open,
            now,
        )
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
            if queued.is_ok() && opening {
                let bytes = teardown(&c.tsp, tty);
                tty.set_teardown(bytes);
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
