// Ported from tmux tty.c, server-client.c @ 8f25579c
use super::{
    broker::{Renderer, now_ms},
    client::{ClientTspState, RequestOwner, Slot},
    projection::Projection,
    surface::{SurfaceId, SurfaceMode},
    wire::{APC_LIMIT, WireMessage},
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
/// `x` for `projection`; an inline one also gives the cell view its screen
/// back. A hidden one first leaves the alternate screen its grid is on, so the
/// anchor row goes from the main screen; while another inline projection is
/// live on the main screen (`main_live`) there is no grid to return to, so
/// only the `x` goes.
fn close_bytes(projection: &Projection, tty: &Tty, main_live: bool) -> Vec<u8> {
    let close = WireMessage::json(
        b'x',
        &serde_json::json!({"id": projection.outer, "keep": false}),
    )
    .encode();
    if projection.is_hidden() && main_live {
        return close;
    }
    let mut bytes = if projection.is_hidden() {
        tty.alternate_screen(false)
    } else {
        Vec::new()
    };
    bytes.extend(close);
    if projection.inline {
        bytes.extend(back_to_grid(tty));
    }
    bytes
}
/// An inline surface is live on the main screen: the shown inline projection
/// past its blob wait, or the inline one under a screen projection.
fn main_live(tsp: &ClientTspState) -> bool {
    tsp.covered.is_some()
        || tsp
            .projection
            .as_ref()
            .is_some_and(|p| p.inline && p.awaiting_blobs().is_none())
}
/// What `Tty::stop` sends for the open projections: the screen first, then
/// the inline surface under it (the grid is up after it), then the hidden
/// ones. One still waiting for the host's blobs has sent nothing.
fn teardown(tsp: &ClientTspState, tty: &Tty) -> Vec<u8> {
    tsp.projections()
        .filter(|projection| projection.awaiting_blobs().is_none())
        .flat_map(|projection| close_bytes(projection, tty, false))
        .collect()
}
fn shows(projection: &Projection, pane: PaneId, logical: &str, handle: SurfaceId) -> bool {
    projection.pane == pane
        && projection.logical == logical
        && projection.outer.ends_with(&format!(":s{}", handle.0))
}
/// Close every projection of the client: shown, covered and hidden.
pub fn close_projection(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let closing: Vec<_> = c
        .tsp
        .projection
        .take()
        .into_iter()
        .chain(c.tsp.covered.take())
        .chain(std::mem::take(&mut c.tsp.hidden))
        .collect();
    close_projections(server, id, closing);
}
/// Close the client's projections of `pane`, hidden ones too; the rest stay.
pub fn close_pane_projection(server: &mut Server, id: ClientId, pane: PaneId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let mut closing: Vec<_> = c
        .tsp
        .projection
        .take_if(|p| p.pane == pane)
        .into_iter()
        .chain(c.tsp.covered.take_if(|p| p.pane == pane))
        .collect();
    let (hidden, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut c.tsp.hidden)
        .into_iter()
        .partition(|p| p.pane == pane);
    c.tsp.hidden = kept;
    closing.extend(hidden);
    close_projections(server, id, closing);
}
/// Close what the client sees; a hidden projection waits for its window.
pub fn close_shown(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let closing: Vec<_> = c
        .tsp
        .projection
        .take()
        .into_iter()
        .chain(c.tsp.covered.take())
        .collect();
    close_projections(server, id, closing);
}
/// Close the hidden projection of `generation` with its `x`; the grid is up
/// already.
pub fn close_hidden(server: &mut Server, id: ClientId, generation: u64) {
    close_hidden_where(server, id, |p| p.generation == generation);
}
fn close_hidden_where(server: &mut Server, id: ClientId, closing: impl Fn(&Projection) -> bool) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let (closing, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut c.tsp.hidden)
        .into_iter()
        .partition(closing);
    c.tsp.hidden = kept;
    close_projections(server, id, closing);
}
fn close_projections(server: &mut Server, id: ClientId, closing: Vec<Projection>) {
    if closing.is_empty() {
        return;
    }
    // One still waiting for the host's blobs has queued nothing, not even its `o`.
    let (waiting, closing): (Vec<_>, Vec<_>) = closing
        .into_iter()
        .partition(|projection| projection.awaiting_blobs().is_some());
    for timeout in waiting
        .into_iter()
        .filter_map(|mut projection| projection.end_blob_wait())
    {
        super::client_runtime::cancel_timer(server, timeout);
    }
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    if let Some(tty) = c.tty.as_mut() {
        if !tty.flags().contains(rmux_tty::tty::TtyFlags::STARTED) {
            return;
        }
        let main_live = main_live(&c.tsp);
        let mut close = Vec::new();
        for mut projection in closing {
            tty.cancel_protocol(projection.generation);
            projection.close();
            close.extend(close_bytes(&projection, tty, main_live));
        }
        let bytes = teardown(&c.tsp, tty);
        tty.set_teardown(bytes);
        // A close with nothing left drops every unsent transaction; one with a
        // surviving projection must keep its queued frames and goes behind
        // them, cancelling only when it would not fit otherwise.
        if c.tsp.projections().next().is_some()
            && tty
                .queue_protocol(ProtocolTransaction::new(close.clone()).teardown())
                .is_ok()
        {
            if !main_live {
                restore_grid(server, id);
            }
            return;
        }
        let _ = tty.close_protocol(ProtocolTransaction::new(close).teardown());
    }
    restore_grid(server, id);
}
/// The host answered `gone` for the hidden projection of `generation`: forget
/// it, no `x`.
pub fn drop_hidden(server: &mut Server, id: ClientId, generation: u64) {
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    let Some(at) = c.tsp.hidden.iter().position(|p| p.generation == generation) else {
        return;
    };
    let hidden = c.tsp.hidden.remove(at);
    if let Some(tty) = c.tty.as_mut() {
        tty.cancel_protocol(hidden.generation);
        let bytes = teardown(&c.tsp, tty);
        tty.set_teardown(bytes);
    }
}
/// Keep the client's open inline projection instead of closing it: the host
/// gets `["suspend"]` and keeps the surface, the cells go back to the
/// alternate screen. Anything else closes: a screen surface, a covered pair,
/// a projection the host has not opened yet, a host that keeps part of a
/// suspended surface over the grid. The hidden entry of the pane the client
/// is going to is never evicted on the way.
pub fn hide_projection(server: &mut Server, id: ClientId) {
    if !hideable(server, id) {
        close_shown(server, id);
        return;
    }
    let keep = super::contract::client_pane(server, id).and_then(|pane| {
        server
            .clients
            .get(id)?
            .tsp
            .hidden
            .iter()
            .find(|p| p.pane == pane)
            .map(|p| p.generation)
    });
    hide_shown(server, id, keep);
}
fn hideable(server: &Server, id: ClientId) -> bool {
    server.clients.get(id).is_some_and(|c| {
        c.tsp.covered.is_none()
            && c.tsp.hidden_cap() > 0
            && c.tty
                .as_ref()
                .is_some_and(|tty| tty.flags().contains(rmux_tty::tty::TtyFlags::STARTED))
            && c.tsp
                .projection
                .as_ref()
                .is_some_and(|p| coverable(server, p, p.pane))
    })
}
/// Move the shown projection to the back of the hidden LRU with its
/// `["suspend"]` frame, then put the grid up. First a hidden entry of the same
/// pane, which is stale, and over the cap the least recently hidden ones close
/// with a bare `x` behind the live surface (never `keep`, the entry about to
/// be revealed).
fn hide_shown(server: &mut Server, id: ClientId, keep: Option<u64>) {
    let c = server.clients.get(id).expect("checked client");
    let shown = c.tsp.projection.as_ref().expect("checked projection");
    let pane = shown.pane;
    let cap = c.tsp.hidden_cap();
    let revealing = keep.is_some_and(|k| c.tsp.hidden.iter().any(|p| p.generation == k));
    let mut kept = c.tsp.hidden.len() - usize::from(revealing);
    let evicted: Vec<u64> = c
        .tsp
        .hidden
        .iter()
        .filter(|p| Some(p.generation) != keep)
        .filter(|p| {
            let out = p.pane == pane || kept >= cap;
            kept -= usize::from(out);
            out
        })
        .map(|p| p.generation)
        .collect();
    close_hidden_where(server, id, |p| evicted.contains(&p.generation));
    let c = server.clients.get_mut(id).expect("checked client");
    let mut projection = c.tsp.projection.take().expect("checked projection");
    let generation = projection.generation;
    let handle = super::client_runtime::outer_surface(&projection.outer).expect("coverable");
    projection.hide();
    c.tsp.hidden.push(projection);
    send_pending(server, id, pane, handle, Slot::Hidden(generation));
    let Some(c) = server
        .clients
        .get_mut(id)
        .filter(|c| c.tsp.hidden.iter().any(|p| p.generation == generation))
    else {
        return;
    };
    // A host that parks several surfaces ends an unsuspended one on the next
    // `o` without a word, so a suspend that waits for credit cannot stay
    // hidden behind an open of the same pass: it closes instead.
    let unsuspended = c.tsp.hides_many()
        && c.tsp
            .slot(Slot::Hidden(generation))
            .is_some_and(|p| p.visibility_pending());
    let tty = c.tty.as_mut().expect("checked tty");
    let enter = ProtocolTransaction::new(tty.alternate_screen(true)).control();
    if unsuspended || tty.queue_protocol(enter).is_err() {
        close_hidden(server, id, generation);
        return;
    }
    let bytes = teardown(&c.tsp, tty);
    tty.set_teardown(bytes);
    restore_grid(server, id);
}
/// Show the hidden projection of `generation` again: leave the alternate
/// screen, then `["resume"]` with the frames queued meanwhile, no `o`. False
/// when the screen switch does not fit the queue.
fn reveal_hidden(
    server: &mut Server,
    id: ClientId,
    generation: u64,
    pane: PaneId,
    handle: SurfaceId,
) -> bool {
    let Some(c) = server.clients.get_mut(id) else {
        return false;
    };
    let Some(at) = c.tsp.hidden.iter().position(|p| p.generation == generation) else {
        return false;
    };
    let Some(tty) = c.tty.as_mut() else {
        return false;
    };
    let leave = ProtocolTransaction::new(tty.alternate_screen(false)).control();
    if tty.queue_protocol(leave).is_err() {
        return false;
    }
    let mode = tty.mode() & !rmux_emu::screen::ScreenMode::ALL_MOUSE_MODES;
    tty.update_mode(&mut server.tparm, mode, None);
    let mut projection = c.tsp.hidden.remove(at);
    projection.reveal();
    c.tsp.projection = Some(projection);
    let bytes = teardown(&c.tsp, tty);
    tty.set_teardown(bytes);
    super::status_bar::refresh(server, id);
    send_pending(server, id, pane, handle, Slot::Shown);
    super::client_runtime::release_drawn(server, pane);
    true
}
/// The hidden projections' `["suspend"]` frames that waited for credit or
/// queue room when they hid.
fn send_hidden(server: &mut Server, id: ClientId) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    let waiting: Vec<_> = c
        .tsp
        .hidden
        .iter()
        .filter(|p| p.visibility_pending() || p.pending_sequence().is_some())
        .filter_map(|p| {
            let handle = super::client_runtime::outer_surface(&p.outer)?;
            Some((p.generation, p.pane, handle))
        })
        .collect();
    for (generation, pane, handle) in waiting {
        send_pending(server, id, pane, handle, Slot::Hidden(generation));
    }
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
    // One still waiting for the host's blobs has queued nothing, not even its `o`.
    if let Some(timeout) = screen.awaiting_blobs() {
        c.tsp.projection = None;
        super::client_runtime::cancel_timer(server, timeout);
        return true;
    }
    let close = ProtocolTransaction::new(close_bytes(screen, tty, false)).teardown();
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
/// In many mode an inline projection of another pane hides when the client
/// moves on: the host parks it next to the others. Everything else closes.
fn leaves_other_pane(server: &Server, id: ClientId, pane: Option<PaneId>) -> bool {
    server.clients.get(id).is_some_and(|c| {
        c.tsp.hides_many()
            && c.tsp
                .projection
                .as_ref()
                .is_some_and(|p| Some(p.pane) != pane)
    }) && hideable(server, id)
}
/// The client's window has no native pane for it: keep a projection of another
/// pane hidden where the host parks it, close anything else it sees.
fn leave_shown(server: &mut Server, id: ClientId, pane: Option<PaneId>) {
    if leaves_other_pane(server, id, pane) {
        hide_shown(server, id, None);
    } else {
        close_shown(server, id);
    }
}
pub fn project_pending(server: &mut Server, id: ClientId) {
    send_hidden(server, id);
    let Some(pane) = super::contract::client_pane(server, id) else {
        leave_shown(server, id, None);
        return;
    };
    if !super::contract::native_viewers(server, pane).contains(&id) {
        leave_shown(server, id, Some(pane));
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
        leave_shown(server, id, Some(pane));
        return;
    };
    if server.clients.get(id).and_then(|c| c.tsp.hello()).is_none() {
        close_shown(server, id);
        return;
    }
    if !super::contract::has_display_contract(server, pane) {
        leave_shown(server, id, Some(pane));
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
    let reveal = c
        .tsp
        .hidden
        .iter()
        .find(|p| shows(p, pane, &logical, handle) && coverable(server, p, pane))
        .map(|p| p.generation);
    let many = c.tsp.hides_many();
    let hide = leaves_other_pane(server, id, Some(pane));
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
        if many {
            // A hidden projection of another surface of this pane is stale.
            close_hidden_where(server, id, |p| {
                p.pane == pane && Some(p.generation) != reveal
            });
            if hide {
                hide_shown(server, id, reveal);
            } else if started && reveal.is_some() {
                // The shown one cannot park (a screen page, a covered pair, a
                // blob wait): it goes before the resume takes its place.
                close_shown(server, id);
            }
        }
        if started && let Some(generation) = reveal {
            // A full queue: the entry stays hidden and the next pass resumes
            // it. The single-slot host closes it and replays instead, as before.
            if reveal_hidden(server, id, generation, pane, handle) || many {
                return;
            }
        }
        if !many {
            // A host with one parked slot ends the parked surface on the next
            // `o` without a word, so the entry goes; the grid is up already.
            close_hidden_where(server, id, |_| true);
        }
        if started && uncover && close_over_covered(server, id) {
            let c = server.clients.get_mut(id).expect("checked client");
            c.tsp.projection = c.tsp.covered.take();
            super::status_bar::refresh(server, id);
            send_pending(server, id, pane, handle, Slot::Shown);
            super::client_runtime::release_drawn(server, pane);
            return;
        }
        if started && cover {
            let c = server.clients.get_mut(id).expect("checked client");
            c.tsp.covered = c.tsp.projection.take();
        } else if !(started && keep_covered && close_over_covered(server, id)) {
            close_shown(server, id);
        }
        if !open_projection(server, id, pane, logical, handle, inline) {
            return;
        }
    }
    send_pending(server, id, pane, handle, Slot::Shown);
}
/// A new outer surface for `handle`, sent from the next `send_pending`. A host
/// with a terminal-wide blob store first says which of the surface's blobs it
/// has, so the replay leaves them out.
fn open_projection(
    server: &mut Server,
    id: ClientId,
    pane: PaneId,
    logical: String,
    handle: SurfaceId,
    inline: bool,
) -> bool {
    let state = server.panes.get(pane).and_then(|p| p.tsp.as_ref());
    let hello = state.map(|s| s.program_hello.clone()).unwrap_or_default();
    let blobs = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.hello())
        .filter(|h| h.features.contains("blobstore"))
        .zip(state)
        .and_then(|(host, state)| {
            let held = state
                .surfaces
                .get(handle)?
                .document
                .blob_references()
                .into_iter()
                .filter(|digest| state.blobs.peek(digest).is_some())
                .collect();
            blobs_query(held, host.apc)
        });
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
    let send_hello = c.tsp.projection_hello.as_ref() != Some(&hello);
    let mut awaiting = None;
    if send_hello || blobs.is_some() {
        let token = c.tsp.request(RequestOwner::Replay {
            pane,
            projection: generation,
        });
        let mut bytes = if send_hello {
            WireMessage::json(b'q', &hello).encode()
        } else {
            Vec::new()
        };
        if let Some((query, _)) = &blobs {
            bytes.extend_from_slice(query);
        }
        if c.tty.as_mut().is_none_or(|tty| {
            tty.queue_da1(Da1Owner::Token(token), ProtocolTransaction::new(bytes))
                .is_err()
        }) {
            c.tsp.requests.pop_back();
            return false;
        }
        if send_hello {
            c.tsp.projection_hello = Some(hello);
        }
        if let Some((_, asked)) = blobs {
            c.tsp.requests.back_mut().expect("queued request").blobs = asked;
            awaiting = Some(token);
        }
    }
    let mut projection = Projection::new(pane, logical, outer, generation, credits);
    projection.inline = inline;
    c.tsp.projection = Some(projection);
    if let Some(token) = awaiting {
        let timeout = super::client_runtime::time_blob_wait(server, id, token);
        if let Some(projection) = server
            .clients
            .get_mut(id)
            .and_then(|c| c.tsp.projection.as_mut())
        {
            projection.wait_for_blobs(timeout);
        }
    }
    super::status_bar::refresh(server, id);
    true
}

/// The encoded `q blobs` for as many `ids` as fit one APC of `apc` bytes, and
/// the ids it asks; the host is not asked about the rest.
fn blobs_query(mut ids: Vec<String>, apc: usize) -> Option<(Vec<u8>, Vec<String>)> {
    let limit = apc.min(APC_LIMIT);
    // The APC payload: `tsp;q;` and the JSON. Each id is a hex digest, a
    // JSON string of its length plus quotes.
    let mut size = br#"tsp;q;{"q":"blobs","ids":[]}"#.len();
    let mut fit = 0;
    for id in &ids {
        size += id.len() + 2 + usize::from(fit > 0);
        if size > limit {
            break;
        }
        fit += 1;
    }
    ids.truncate(fit);
    if ids.is_empty() {
        return None;
    }
    let query = WireMessage::json(b'q', &serde_json::json!({"q":"blobs","ids":ids})).encode();
    Some((query, ids))
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
                    .projections()
                    .any(|p| p.pane == pane && p.logical == logical && !p.failed)
            })
        })
        .collect();
    for id in ids {
        let Some(c) = server.clients.get_mut(id) else {
            continue;
        };
        let shown = |p: &&mut Projection| p.pane == pane && p.logical == logical && !p.failed;
        for aside in c
            .tsp
            .covered
            .iter_mut()
            .chain(&mut c.tsp.hidden)
            .filter(shown)
        {
            let _ = aside.queue_metadata(verb, value);
        }
        if let Some(p) = c.tsp.projection.as_mut().filter(shown) {
            let _ = p.queue_metadata(verb, value);
            send_pending(server, id, pane, handle, Slot::Shown);
        }
    }
}

/// A `b` that arrived after a frame referencing it went out (omp sends its
/// first blob after the `blobs` query round trip, so the frame is projected
/// first). Each projection of a surface that references `digest` sends it on
/// its own; a frame prepared first carries it instead.
pub fn queue_blob(server: &mut Server, pane: PaneId, digest: &str) {
    let Some(state) = server.panes.get(pane).and_then(|p| p.tsp.as_ref()) else {
        return;
    };
    let Some(blob) = state.blobs.peek(digest) else {
        return;
    };
    // The id as the frame wrote it, so the host matches it to the node.
    let referencing: Vec<(super::surface::SurfaceId, String, String)> = state
        .surfaces
        .iter()
        .filter_map(|surface| {
            let id = surface
                .document
                .blob_references()
                .into_iter()
                .find(|id| id.eq_ignore_ascii_case(digest))?;
            Some((surface.id, surface.document.surface.clone(), id))
        })
        .collect();
    if referencing.is_empty() {
        return;
    }
    let mime = blob.mime.clone();
    let bytes = std::sync::Arc::clone(&blob.bytes);
    let ids: Vec<ClientId> = server.client_order.iter().copied().collect();
    for client in ids {
        for (handle, logical, id) in &referencing {
            let Some(c) = server.clients.get_mut(client) else {
                break;
            };
            let blob = || super::replay::ReplayBlob {
                id: id.clone(),
                mime: mime.clone(),
                bytes: std::sync::Arc::clone(&bytes),
            };
            let shown = |p: &&mut Projection| p.pane == pane && p.logical == *logical && !p.failed;
            let ClientTspState {
                confirmed_blobs,
                projection,
                covered,
                hidden,
                ..
            } = &mut c.tsp;
            for aside in covered.iter_mut().chain(hidden).filter(shown) {
                aside.queue_blob(blob(), confirmed_blobs);
            }
            if projection
                .as_mut()
                .filter(shown)
                .is_some_and(|p| p.queue_blob(blob(), confirmed_blobs))
            {
                send_pending(server, client, pane, *handle, Slot::Shown);
            }
        }
    }
}

/// Queue what the projection in `slot` has ready for the tty.
fn send_pending(
    server: &mut Server,
    id: ClientId,
    pane: PaneId,
    handle: super::surface::SurfaceId,
    slot: Slot,
) {
    let limit = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.hello())
        .map_or(65536, |h| h.apc);
    let generation = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.slot(slot))
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
        let ClientTspState {
            confirmed_blobs,
            projection,
            covered,
            hidden,
            ..
        } = &mut client.tsp;
        let projection = match slot {
            Slot::Shown => projection.as_mut(),
            Slot::Covered => covered.as_mut(),
            Slot::Hidden(generation) => hidden.iter_mut().find(|p| p.generation == generation),
        };
        let Some(projection) = projection else {
            return;
        };
        projection.prepare_send(
            &surface.document,
            &state.blobs,
            confirmed_blobs,
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
        send_metadata(server, id, limit, generation, slot);
        return;
    }
    // Old snapshot dependencies and its frame finish before newer metadata.
    while let Some(piece) = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.slot(slot))
        .and_then(|p| p.take_piece())
    {
        let sequence = server
            .clients
            .get(id)
            .and_then(|c| c.tsp.slot(slot))
            .and_then(|p| p.pending_sequence())
            .unwrap_or(0);
        let token = format!("{sequence}-{}", piece.token());
        let opening = piece.is_open();
        let leave_alternate = opening
            && server
                .clients
                .get(id)
                .and_then(|c| c.tsp.slot(slot))
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
                    .and_then(|c| c.tsp.slot_mut(slot))
                    .is_some_and(|p| p.note_piece(sequence));
                if finished {
                    send_metadata(server, id, limit, generation, slot);
                    break;
                }
            }
            Some(Err(QueueFull)) | None => break,
        }
    }
}
fn send_metadata(server: &mut Server, id: ClientId, limit: usize, generation: u64, slot: Slot) {
    while let Some(message) = server
        .clients
        .get(id)
        .and_then(|c| c.tsp.slot(slot))
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
                    .and_then(|c| c.tsp.slot_mut(slot))
                    .is_some_and(|p| p.note_metadata());
            }
            Some(Err(QueueFull)) | None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_query_asks_only_ids_that_fit_one_apc() {
        let ids: Vec<String> = (0..4u8).map(|n| format!("{n:064x}")).collect();
        let (query, asked) = blobs_query(ids.clone(), 228).unwrap();
        assert_eq!(query.len() - 4, 228);
        assert_eq!(asked, ids[..3]);
        let (query, asked) = blobs_query(ids.clone(), 227).unwrap();
        assert!(query.len() - 4 <= 227);
        assert_eq!(asked, ids[..2]);
        assert!(blobs_query(Vec::new(), 65536).is_none());
        assert!(blobs_query(ids, 64).is_none());
    }
}
