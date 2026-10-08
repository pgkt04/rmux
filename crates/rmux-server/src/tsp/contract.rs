// Ported from tmux server-client.c, window-visible.c @ 8f25579c
// Renderer epochs and the ready barrier are an rmux TSP extension.
use super::{
    broker::{self, PaneTspState, Renderer, SwitchState},
    wire::{DisplayContract, WireMessage},
};
use crate::{
    client::ClientFlags,
    ids::{ClientId, PaneId},
    model::{Pane, Server, pane::pane_is_visible},
    ui::visible::{VisibilityModel, VisibleRanges},
};
use rmux_tty::tty::TtyFlags;
use serde_json::json;
use std::time::Duration;

pub fn client_pane(server: &Server, client: ClientId) -> Option<PaneId> {
    let session = server.clients.get(client)?.session?;
    let link = server.sessions.get(session)?.current?;
    server
        .windows
        .get(server.winlinks.get(link)?.window)?
        .active
}

pub fn viewers(server: &Server, pane: PaneId) -> Vec<ClientId> {
    let Some(p) = server.panes.get(pane) else {
        return Vec::new();
    };
    if !pane_is_visible(server, pane) {
        return Vec::new();
    }
    let Some(model) = VisibilityModel::capture(server, pane) else {
        return Vec::new();
    };
    let mut ranges = VisibleRanges::default();
    server
        .client_order
        .iter()
        .copied()
        .filter(|id| {
            let Some(c) = server.clients.get(*id) else {
                return false;
            };
            if !c.flags.contains(ClientFlags::ATTACHED)
                || c.flags.intersects(
                    ClientFlags::CONTROL
                        | ClientFlags::SUSPENDED
                        | ClientFlags::DEAD
                        | ClientFlags::EXITED
                        | ClientFlags::EXIT,
                )
            {
                return false;
            }
            let Some(tty) = c
                .tty
                .as_ref()
                .filter(|t| t.flags().contains(TtyFlags::STARTED))
            else {
                return false;
            };
            let selected = c
                .session
                .and_then(|s| server.sessions.get(s)?.current)
                .and_then(|l| server.winlinks.get(l));
            if selected.is_none_or(|l| l.window != p.window) {
                return false;
            }
            let (_, mut ox, mut oy, _, _) = tty.window_offset();
            if c.pan_window == Some(p.window) {
                ox = c.pan_ox;
                oy = c.pan_oy;
            }
            let (sx, mut sy) = tty.size();
            if !broker::native_client(server, *id) {
                sy = sy.saturating_sub(crate::ui::status::status_line_size(server, *id));
            }
            let left = i64::from(ox);
            let right = left + i64::from(sx);
            let top = i64::from(oy).max(i64::from(p.yoff));
            let bottom = (i64::from(oy) + i64::from(sy)).min(i64::from(p.yoff) + i64::from(p.sy));
            (top..bottom).any(|y| {
                ranges.ranges.clear();
                model.visible_ranges(p.xoff, y as i32, p.sx, &mut ranges);
                ranges.ranges.iter().any(|r| {
                    r.nx != 0 && i64::from(r.px) < right && i64::from(r.px) + i64::from(r.nx) > left
                })
            })
        })
        .collect()
}

/// Desired renderer for physical viewers, independent of the program's renderer.
pub(crate) fn desired_view(server: &Server, pane: PaneId) -> &'static str {
    let ids = viewers(server, pane);
    if ids.is_empty() {
        return "detached";
    }
    // Disabled brokers never start a terminal probe.
    if !server.tsp_broker_enabled {
        return "ansi";
    }
    if ids.iter().any(|id| {
        server.clients.get(*id).is_some_and(|c| {
            matches!(
                c.tsp.capability,
                super::client::Capability::Unknown | super::client::Capability::Probing
            )
        })
    }) {
        return "pending";
    }
    let Some(p) = server.panes.get(pane) else {
        return "detached";
    };
    let Some(state) = p.tsp.as_ref() else {
        return "ansi";
    };
    if display_contract_for_viewers(server, pane, p, state, &ids)
        .0
        .is_some()
    {
        "native"
    } else {
        "ansi"
    }
}

pub fn display_contract(
    server: &Server,
    pane: PaneId,
) -> (Option<DisplayContract>, Option<ClientId>) {
    let Some(p) = server.panes.get(pane) else {
        return (None, None);
    };
    let Some(state) = p.tsp.as_ref() else {
        return (None, None);
    };
    let ids = viewers(server, pane);
    display_contract_for_viewers(server, pane, p, state, &ids)
}

fn display_contract_for_viewers(
    server: &Server,
    pane: PaneId,
    p: &Pane,
    state: &PaneTspState,
    ids: &[ClientId],
) -> (Option<DisplayContract>, Option<ClientId>) {
    let eligible: Vec<_> = ids
        .iter()
        .copied()
        .filter(|id| viewer_eligible(server, pane, *id))
        .collect();
    let leader = eligible
        .iter()
        .copied()
        .filter(|id| {
            !server
                .clients
                .get(*id)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        })
        .max_by(|a, b| {
            server
                .clients
                .get(*a)
                .unwrap()
                .activity_time
                .cmp(&server.clients.get(*b).unwrap().activity_time)
                .then_with(|| b.cmp(a))
        })
        .or_else(|| eligible.iter().copied().min());
    let offers_v1 = state.program_hello["v"]
        .as_array()
        .is_some_and(|v| v.iter().any(|version| version.as_u64() == Some(1)));
    if ids.is_empty()
        || (!state.registered && !state.program_exited)
        || !offers_v1
        || state.replay_failed
        || state.ui_pending
        || state.switch.as_ref().is_some_and(|s| s.failed)
        || !server.tsp_broker_enabled
        || !p.modes.is_empty()
        || p.prompt.is_some()
    {
        return (None, leader);
    }
    let Some(w) = server.windows.get(p.window) else {
        return (None, leader);
    };
    if w.menu_active
        || w.menu.is_some()
        || p.xoff != 0
        || p.yoff != 0
        || p.sx != w.sx
        || p.sy != w.sy
        || w.panes
            .iter()
            .any(|id| *id != pane && pane_is_visible(server, *id))
        || crate::model::pane::pane_is_floating(server, pane)
    {
        return (None, leader);
    }
    if eligible.len() != ids.len() {
        return (None, leader);
    }
    let hellos: Vec<_> = ids
        .iter()
        .filter_map(|id| server.clients.get(*id)?.tsp.hello().cloned())
        .collect();
    let index = ids.iter().position(|id| Some(*id) == leader).unwrap();
    (DisplayContract::intersect(&hellos, index, p.sx), leader)
}

fn viewer_eligible(server: &Server, pane: PaneId, id: ClientId) -> bool {
    let Some(p) = server.panes.get(pane) else {
        return false;
    };
    let Some(w) = server.windows.get(p.window) else {
        return false;
    };
    let Some(c) = server.clients.get(id) else {
        return false;
    };
    let Some(tty) = c.tty.as_ref() else {
        return false;
    };
    let (_, ox, oy, _, _) = tty.window_offset();
    let (sx, sy) = tty.size();
    c.pan_window != Some(p.window)
        && ox == 0
        && oy == 0
        && sx >= w.sx
        && sy >= w.sy
        && !c.prompt.is_some()
        && c.message.text.is_none()
        && c.tsp
            .hello()
            .is_some_and(|h| h.v == 1 && h.credits > 0 && h.apc >= 64)
}

fn same_renderer_contract(a: Option<&DisplayContract>, b: Option<&DisplayContract>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.kinds == b.kinds
                && a.features == b.features
                && a.apc == b.apc
                && a.credits == b.credits
                && a.cols == b.cols
        }
        (None, None) => true,
        _ => false,
    }
}

/// Close every projection of `pane`; returns the clients that had one.
fn close_pane_projections(server: &mut Server, pane: PaneId) -> Vec<ClientId> {
    let ids: Vec<_> = server
        .client_order
        .iter()
        .copied()
        .filter(|id| {
            server
                .clients
                .get(*id)
                .is_some_and(|c| c.tsp.projection.as_ref().is_some_and(|p| p.pane == pane))
        })
        .collect();
    for id in &ids {
        broker::close_projection(server, *id);
    }
    ids
}

fn cancel_timer(server: &mut Server, timer: Option<(crate::ids::TimerId, u64)>) {
    if let Some((timer, callback)) = timer {
        server.event_loop.cancel(timer);
        server.deferred.remove(&callback);
    }
}

fn arm_timer(server: &mut Server, pane: PaneId) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let generation = state.generation;
    let Some(switch) = state.switch.as_mut() else {
        return;
    };
    let epoch = switch.probing_epoch.unwrap_or(switch.requested_epoch);
    let old = switch.timer.take();
    cancel_timer(server, old);
    let timer = crate::server::event_loop::schedule_deferred(
        server,
        Duration::from_secs(5),
        Box::new(move |server| timeout(server, pane, generation, epoch)),
    );
    if let Some(switch) = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .and_then(|s| s.switch.as_mut())
    {
        switch.timer = Some(timer);
    }
}

fn timeout(server: &mut Server, pane: PaneId, generation: u64, epoch: u64) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    if state.generation != generation {
        return;
    }
    let Some(switch) = state.switch.as_mut() else {
        return;
    };
    if switch.failed || switch.probing_epoch != Some(epoch) {
        return;
    }
    switch.failed = true;
    rmux_util::log_debug!("tsp {pane:?}: switch to epoch {epoch} timed out");
    switch.timer = None;
    state.ui_pending = false;
    let deferred = std::mem::take(&mut state.deferred_ui);
    clear_transition_clients(server, pane);
    for callback in deferred {
        callback(server);
    }
    close_pane_projections(server, pane);
    for client in viewers(server, pane) {
        let text = "TSP renderer transition timed out; the program did not provide a complete view. Refresh to retry.";
        if let Some(c) = server.clients.get_mut(client) {
            c.tsp.diagnostic = Some(text.into());
        }
        broker::restore_grid(server, client);
        crate::ui::status::status_message_set(
            server,
            Some(client),
            0,
            true,
            false,
            true,
            text.as_bytes(),
        );
    }
}

fn request_view(server: &mut Server, pane: PaneId, reason: &str) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let epoch = state.epoch;
    let Some(switch) = state.switch.as_mut() else {
        return;
    };
    if switch.failed || switch.probing_epoch.is_some() {
        return;
    }
    switch.probing_epoch = Some(epoch);
    switch.native = state.contract.is_some();
    switch.probe_observed = false;
    rmux_util::log_debug!("tsp {pane:?}: request view epoch {epoch} ({reason})");
    broker::event(server, pane, &super::wire::view_event(epoch, reason));
    arm_timer(server, pane);
}

pub fn begin_initial(server: &mut Server, pane: PaneId, native: bool) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let baseline = state
        .surfaces
        .selected()
        .map(|s| (s.id, s.document.revision));
    let old = state.switch.take().and_then(|s| s.timer);
    state.renderer = Renderer::Switching;
    state.switch = Some(SwitchState {
        requested_epoch: state.epoch,
        probing_epoch: Some(state.epoch),
        native,
        timer: None,
        failed: false,
        surface_revision: baseline,
        probe_observed: true,
    });
    cancel_timer(server, old);
    close_pane_projections(server, pane);
    for client in viewers(server, pane) {
        if let Some(c) = server.clients.get_mut(client) {
            c.tsp.transition_pane = Some(pane);
        }
        super::input::client_read_bound(server, client);
    }
    arm_timer(server, pane);
}

/// A [`PaneTspState::stock`] program takes its renderer from the viewers at
/// its hello and keeps it until its next hello: native when it starts with only
/// eligible viewers, else ANSI.
pub fn begin_stock(server: &mut Server, pane: PaneId, native: bool) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let old = state.switch.take().and_then(|s| s.timer);
    state.renderer = if native {
        Renderer::Native
    } else {
        Renderer::Ansi
    };
    cancel_timer(server, old);
    clear_transition_clients(server, pane);
    if !native {
        for client in close_pane_projections(server, pane) {
            broker::restore_grid(server, client);
        }
    }
}

/// Viewer changes for a stock program, which cannot be asked to switch. A
/// native program stays native: rmux projects it while the pane is eligible,
/// shows the plain grid while it is not, and acks its frames itself while no
/// viewer draws them.
fn recompute_stock(
    server: &mut Server,
    pane: PaneId,
    ids: Vec<ClientId>,
    contract: Option<DisplayContract>,
    leader: Option<ClientId>,
) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    state.leader = leader;
    if !matches!(state.renderer, Renderer::Native | Renderer::Detached) {
        return;
    }
    if ids.is_empty() {
        let notify = state.renderer == Renderer::Native;
        state.renderer = Renderer::Detached;
        close_pane_projections(server, pane);
        if notify {
            visible_event(server, pane, false);
        }
        super::client_runtime::release_undrawn(server, pane);
        return;
    }
    let returning = state.renderer == Renderer::Detached;
    state.renderer = Renderer::Native;
    let Some(contract) = contract else {
        for client in close_pane_projections(server, pane) {
            broker::restore_grid(server, client);
        }
        super::client_runtime::release_undrawn(server, pane);
        return;
    };
    let changed = !same_renderer_contract(state.contract.as_ref(), Some(&contract));
    state.contract = Some(contract);
    if changed {
        close_pane_projections(server, pane);
    }
    if returning {
        visible_event(server, pane, true);
    }
    super::client_runtime::report_view(server, pane);
    for client in ids {
        broker::project_pending(server, client);
    }
    super::client_runtime::release_drawn(server, pane);
}

pub fn note_probe(server: &mut Server, pane: PaneId, epoch: u64, native: bool) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let baseline = state
        .surfaces
        .selected()
        .map(|s| (s.id, s.document.revision));
    let Some(switch) = state.switch.as_mut() else {
        return;
    };
    if switch.failed || switch.requested_epoch != epoch || switch.probing_epoch != Some(epoch) {
        return;
    }
    switch.native = native;
    switch.surface_revision = baseline;
    switch.probe_observed = true;
}

fn visible_event(server: &mut Server, pane: PaneId, visible: bool) {
    let Some(p) = server.panes.get(pane) else {
        return;
    };
    let Some(state) = p.tsp.as_ref().filter(|s| s.registered) else {
        return;
    };
    let Some(surface) = state.surfaces.selected().filter(|s| s.listens()) else {
        return;
    };
    let value = json!({"ev":"resize","sf":surface.wire_id,"cols":p.sx,"cell":state.contract.as_ref().and_then(|c| c.cell.as_ref()),"visible":visible});
    broker::event(server, pane, &value);
}

pub fn recompute(server: &mut Server) {
    let panes: Vec<_> = server.pane_ids.values().copied().collect();
    for pane in panes {
        recompute_pane(server, pane, false);
        let note = broker::native_only(server, pane).is_some();
        if let Some(p) = server.panes.get_mut(pane)
            && let Some(state) = p.tsp.as_mut()
            && state.native_note != note
        {
            state.native_note = note;
            p.flags.insert(crate::model::PaneFlags::REDRAW);
        }
    }
}

fn recompute_pane(server: &mut Server, pane: PaneId, force: bool) {
    let ids = viewers(server, pane);
    let (contract, leader) = display_contract(server, pane);
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    if state.program_exited {
        state.leader = leader;
        if !ids.is_empty() {
            state.contract = contract;
        }
        for client in ids {
            broker::project_pending(server, client);
        }
        return;
    }
    if !state.registered || state.switch.as_ref().is_some_and(|s| s.failed) {
        rmux_util::log_debug!(
            "tsp {pane:?}: recompute skipped (registered {}, failed switch {})",
            state.registered,
            state.switch.as_ref().is_some_and(|s| s.failed)
        );
        return;
    }
    if state.stock {
        recompute_stock(server, pane, ids, contract, leader);
        return;
    }
    if ids.is_empty()
        && state.switch.is_none()
        && matches!(state.renderer, Renderer::Native | Renderer::Detached)
        && !force
        && !state.ui_pending
        && !state.replay_failed
        && server.tsp_broker_enabled
    {
        let notify = state.renderer == Renderer::Native;
        state.renderer = Renderer::Detached;
        close_pane_projections(server, pane);
        if notify {
            visible_event(server, pane, false);
        }
        return;
    }
    let changed = !same_renderer_contract(state.contract.as_ref(), contract.as_ref()) || force;
    state.leader = leader;
    if !changed {
        state.contract = contract;
        let returning = state.renderer == Renderer::Detached && !ids.is_empty();
        let switching = state.switch.is_some();
        if returning {
            state.renderer = Renderer::Native;
        }
        if returning {
            visible_event(server, pane, true);
        }
        super::client_runtime::report_view(server, pane);
        for client in ids {
            if switching {
                if let Some(c) = server.clients.get_mut(client) {
                    c.tsp.transition_pane = Some(pane);
                }
                super::input::client_read_bound(server, client);
            } else {
                broker::project_pending(server, client);
            }
        }
        super::client_runtime::release_drawn(server, pane);
        return;
    }
    let reason = if state.ui_pending {
        "ui"
    } else if state.contract.is_some() && contract.is_some() {
        "capabilities"
    } else {
        "viewers"
    };
    state.epoch = state.epoch.checked_add(1).expect("TSP epoch exhaustion");
    state.contract = contract;
    state.renderer = Renderer::Switching;
    let baseline = state
        .surfaces
        .selected()
        .map(|s| (s.id, s.document.revision));
    if let Some(switch) = state.switch.as_mut() {
        switch.requested_epoch = state.epoch;
    } else {
        state.switch = Some(SwitchState {
            requested_epoch: state.epoch,
            probing_epoch: None,
            native: state.contract.is_some(),
            timer: None,
            failed: false,
            surface_revision: baseline,
            probe_observed: false,
        });
    }
    close_pane_projections(server, pane);
    for client in ids {
        if let Some(c) = server.clients.get_mut(client) {
            c.tsp.transition_pane = Some(pane);
        }
        super::input::client_read_bound(server, client);
    }
    request_view(server, pane, reason);
}

pub fn ready(server: &mut Server, pane: PaneId, epoch: u64, renderer: &str) -> bool {
    let accepted = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .is_some_and(|state| {
            let Some(switch) = state.switch.as_ref() else {
                return false;
            };
            if !state.registered
                || switch.failed
                || !switch.probe_observed
                || state.epoch != epoch
                || switch.requested_epoch != epoch
                || switch.probing_epoch != Some(epoch)
            {
                return false;
            }
            if switch.native {
                renderer == "native"
                    && state.contract.is_some()
                    && state.surfaces.selected().is_some_and(|sf| {
                        sf.is_open()
                            && sf.document.revision > 0
                            && switch.surface_revision.is_none_or(|(id, revision)| {
                                sf.id != id || sf.document.revision > revision
                            })
                    })
            } else {
                renderer == "ansi"
            }
        });
    rmux_util::log_debug!("tsp {pane:?}: ready epoch {epoch} {renderer}: accepted {accepted}");
    broker::reply(
        server,
        pane,
        WireMessage::json(b'r', &super::wire::ready_reply(epoch, accepted)),
    );
    if !accepted {
        let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
            return false;
        };
        if !state.registered {
            return false;
        }
        let latest = state.epoch;
        let Some(switch) = state.switch.as_mut() else {
            broker::event(server, pane, &super::wire::view_event(latest, "viewers"));
            return false;
        };
        if switch.failed {
            broker::event(server, pane, &super::wire::view_event(latest, "viewers"));
            return false;
        }
        if switch.probing_epoch == Some(epoch) || switch.probing_epoch.is_none() {
            switch.probing_epoch = None;
            let old = switch.timer.take();
            cancel_timer(server, old);
            request_view(server, pane, "viewers");
        }
        return false;
    }
    let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
    let switch = state.switch.take().unwrap();
    state.renderer = if switch.native {
        Renderer::Native
    } else {
        Renderer::Ansi
    };
    let deferred = if switch.native {
        Vec::new()
    } else {
        state.ui_pending = false;
        std::mem::take(&mut state.deferred_ui)
    };
    cancel_timer(server, switch.timer);
    super::client_runtime::report_view(server, pane);
    clear_transition_clients(server, pane);
    for client in viewers(server, pane) {
        if switch.native {
            broker::project_pending(server, client);
        } else {
            broker::restore_grid(server, client);
        }
    }
    broker::release_input(server, pane);
    for callback in deferred {
        callback(server);
    }
    recompute_pane(server, pane, false);
    true
}

pub fn refresh(server: &mut Server, client: ClientId) {
    let Some(pane) = client_pane(server, client) else {
        return;
    };
    for id in viewers(server, pane) {
        let diagnostic = server
            .clients
            .get_mut(id)
            .and_then(|c| c.tsp.diagnostic.take())
            .is_some();
        if diagnostic {
            crate::ui::status::status_message_clear(server, id);
        }
    }
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    state.replay_failed = false;
    let timer = if state.switch.as_ref().is_some_and(|s| s.failed) {
        let switch = state.switch.take().unwrap();
        switch.timer
    } else {
        None
    };
    cancel_timer(server, timer);
    recompute_pane(server, pane, true);
}

fn clear_transition_clients(server: &mut Server, pane: PaneId) {
    let ids: Vec<_> = server
        .client_order
        .iter()
        .copied()
        .filter(|id| {
            server
                .clients
                .get(*id)
                .is_some_and(|c| c.tsp.transition_pane == Some(pane))
        })
        .collect();
    for id in ids {
        if let Some(c) = server.clients.get_mut(id) {
            c.tsp.transition_pane = None;
        }
        super::input::client_read_bound(server, id);
    }
}
#[cfg(test)]
mod tests {
    use super::super::{
        client::{Capability, RequestOwner},
        wire::{Frame, Hello},
    };
    use super::*;
    use crate::{
        client::Client,
        ids::SessionId,
        layout::{self, LayoutType},
        model::{session, spawn::SpawnFlags, window},
    };
    use rmux_tty::tty::{Tty, TtyHostInfo};
    use std::os::fd::AsFd;

    fn fixture() -> (Server, PaneId, SessionId) {
        let mut server = Server::new();
        let options = server.options.create(Some(server.options.global_s));
        let session = session::session_create(
            &mut server,
            session::SessionCreate {
                prefix: None,
                name: Some(b"contract".to_vec()),
                cwd: b"/".to_vec(),
                environment: crate::options::environment::Environment::default(),
                options,
                termios: None,
            },
        );
        let window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane =
            window::window_add_pane(&mut server, window, None, 0, SpawnFlags::default()).unwrap();
        server.windows.get_mut(window).unwrap().active = Some(pane);
        let link = session::session_attach(&mut server, session, window, 0).unwrap();
        session::session_set_current(&mut server, session, Some(link));
        server.panes.get_mut(pane).unwrap().tsp = Some(PaneTspState {
            registered: true,
            program_hello: json!({"v":[1],"features":[super::super::wire::BROKER_FEATURE]}),
            ..PaneTspState::default()
        });
        (server, pane, session)
    }

    fn client(server: &mut Server, session: SessionId, native: bool, readonly: bool) -> ClientId {
        client_with_master(server, session, native, readonly).0
    }

    fn client_with_master(
        server: &mut Server,
        session: SessionId,
        native: bool,
        readonly: bool,
    ) -> (ClientId, std::os::fd::OwnedFd) {
        let (master, slave, _) = rmux_sys::pty::openpty().unwrap();
        let tio = rmux_sys::TermiosState::get(slave.as_fd()).unwrap();
        let mut tty = Tty::new(slave, tio, TtyHostInfo::default());
        let caps = vec![
            rmux_util::bytes::ByteString::from("clear=\\E[H\\E[2J"),
            rmux_util::bytes::ByteString::from("cup=\\E[%i%p1%d;%p2%dH"),
        ];
        tty.open(
            &mut server.tparm,
            b"fixture",
            &caps,
            &Default::default(),
            None,
        )
        .unwrap();
        tty.set_size(80, 25, 0, 0);
        tty.flags_mut().insert(TtyFlags::STARTED);
        let mut client = Client::new(None, (0, 0));
        client.session = Some(session);
        client.flags.insert(ClientFlags::ATTACHED);
        if readonly {
            client.flags.insert(ClientFlags::READONLY);
        }
        client.tsp.generation = tty.protocol_generation();
        client.tty = Some(tty);
        client.tsp.visible = true;
        if native {
            client.tsp.capability = Capability::V1(
                serde_json::from_value::<Hello>(json!({"v":1,"kinds":["col","text"],"credits":2}))
                    .unwrap(),
            );
        }
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        (id, master)
    }

    fn state(server: &Server, pane: PaneId) -> &PaneTspState {
        server.panes.get(pane).unwrap().tsp.as_ref().unwrap()
    }
    fn state_mut(server: &mut Server, pane: PaneId) -> &mut PaneTspState {
        server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap()
    }

    fn assert_desired_view(server: &mut Server, pane: PaneId, expected: &str) {
        assert_eq!(desired_view(server, pane), expected);
        let context = crate::format::FormatContext {
            pane: Some(pane),
            ..crate::format::FormatContext::default()
        };
        let mut tree = crate::format::create_defaults(server, None, context);
        assert_eq!(
            &*tree.expand(server, b"#{pane_tsp_view}"),
            expected.as_bytes()
        );
    }

    #[test]
    fn same_size_native_join_keeps_epoch_but_plain_readonly_forces_ansi() {
        let (mut server, pane, session) = fixture();
        let a = client(&mut server, session, true, false);
        let (contract, leader) = display_contract(&server, pane);
        assert_eq!(leader, Some(a));
        state_mut(&mut server, pane).contract = contract;
        state_mut(&mut server, pane).renderer = Renderer::Native;
        let epoch = state(&server, pane).epoch;
        client(&mut server, session, true, false);
        recompute(&mut server);
        assert_eq!(state(&server, pane).epoch, epoch);
        assert!(state(&server, pane).switch.is_none());
        let plain = client(&mut server, session, false, true);
        assert!(viewers(&server, pane).contains(&plain));
        recompute(&mut server);
        assert_eq!(state(&server, pane).epoch, epoch + 1);
        assert_eq!(state(&server, pane).renderer, Renderer::Switching);
        assert!(state(&server, pane).contract.is_none());
    }

    #[test]
    fn no_viewers_never_negotiate_and_native_detach_retains_contract() {
        let (mut server, pane, session) = fixture();
        assert!(display_contract(&server, pane).0.is_none());
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
        let id = client(&mut server, session, true, false);
        let contract = display_contract(&server, pane).0.unwrap();
        state_mut(&mut server, pane).contract = Some(contract.clone());
        state_mut(&mut server, pane).renderer = Renderer::Native;
        server.clients.get_mut(id).unwrap().session = None;
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Detached);
        assert_eq!(state(&server, pane).contract, Some(contract));
        assert_eq!(state(&server, pane).epoch, 1);
        server.clients.get_mut(id).unwrap().session = Some(session);
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Native);
        assert!(state(&server, pane).switch.is_none());
    }

    #[test]
    fn stale_ready_defers_new_probe_until_old_transaction_finishes() {
        let (mut server, pane, session) = fixture();
        let id = client(&mut server, session, true, false);
        recompute(&mut server);
        let old = state(&server, pane).epoch;
        assert_eq!(
            state(&server, pane).switch.as_ref().unwrap().probing_epoch,
            Some(old)
        );
        server.clients.get_mut(id).unwrap().tsp.capability = Capability::Unsupported;
        recompute(&mut server);
        let new = state(&server, pane).epoch;
        assert!(new > old);
        assert_eq!(
            state(&server, pane).switch.as_ref().unwrap().probing_epoch,
            Some(old)
        );
        assert!(!ready(&mut server, pane, old, "native"));
        assert_eq!(
            state(&server, pane).switch.as_ref().unwrap().probing_epoch,
            Some(new)
        );
        note_probe(&mut server, pane, new, false);
        assert!(ready(&mut server, pane, new, "ansi"));
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
    }

    #[test]
    fn native_ready_requires_first_completed_frame_not_just_open() {
        let (mut server, pane, session) = fixture();
        client(&mut server, session, true, false);
        recompute(&mut server);
        let epoch = state(&server, pane).epoch;
        note_probe(&mut server, pane, epoch, true);
        let id = state_mut(&mut server, pane)
            .surfaces
            .open(&json!({"id":"view","mode":"screen"}), None)
            .unwrap()
            .opened
            .unwrap();
        assert!(!ready(&mut server, pane, epoch, "native"));
        note_probe(&mut server, pane, epoch, true);
        let sf = state_mut(&mut server, pane).surfaces.get_mut(id).unwrap();
        sf.document
            .apply_frame(
                &Frame {
                    sf: "view".into(),
                    s: 1,
                    ops: vec![json!(["add","main","view",null,{"id":"main","k":"col"}])],
                },
                0,
            )
            .unwrap();
        assert!(ready(&mut server, pane, epoch, "native"));
        assert_eq!(state(&server, pane).renderer, Renderer::Native);
    }

    /// A hello without the broker feature, as released omp sends it.
    fn stock_hello(server: &mut Server, pane: PaneId) {
        server.panes.get_mut(pane).unwrap().tsp = None;
        pane_send(
            server,
            pane,
            b'q',
            json!({"q":"hello","v":[1],"app":"omp","features":["edit","undo","send"]}),
        );
    }

    fn broker_messages(messages: &[serde_json::Value]) -> usize {
        messages
            .iter()
            .filter(|m| m["r"] == "rmux-probe" || m["ev"] == "rmux-view")
            .count()
    }

    #[test]
    fn desired_view_tracks_stock_native_viewers_without_switching_program() {
        let (mut server, pane, session) = fixture();
        server.tsp_broker_enabled = true;
        let native = client(&mut server, session, true, false);
        let observer = client(&mut server, session, false, false);
        server
            .clients
            .get_mut(observer)
            .unwrap()
            .flags
            .insert(ClientFlags::CONTROL);
        stock_hello(&mut server, pane);
        assert_desired_view(&mut server, pane, "native");
        let epoch = state(&server, pane).epoch;
        let generation = state(&server, pane).generation;

        server.clients.get_mut(native).unwrap().session = None;
        recompute(&mut server);
        assert_desired_view(&mut server, pane, "detached");
        assert_eq!(state(&server, pane).renderer, Renderer::Detached);

        let plain = client(&mut server, session, false, false);
        recompute(&mut server);
        assert_desired_view(&mut server, pane, "pending");
        let token = server
            .clients
            .get_mut(plain)
            .unwrap()
            .tsp
            .request(RequestOwner::Detection);
        assert_desired_view(&mut server, pane, "pending");
        server.clients.get_mut(plain).unwrap().tsp.sentinel(token);
        recompute(&mut server);
        assert_desired_view(&mut server, pane, "ansi");
        assert_eq!(state(&server, pane).renderer, Renderer::Native);

        server.clients.get_mut(native).unwrap().session = Some(session);
        recompute(&mut server);
        assert_desired_view(&mut server, pane, "ansi");
        assert_eq!(state(&server, pane).renderer, Renderer::Native);
        server.clients.get_mut(plain).unwrap().session = None;
        recompute(&mut server);
        assert_desired_view(&mut server, pane, "native");
        assert_eq!(state(&server, pane).renderer, Renderer::Native);
        assert!(state(&server, pane).switch.is_none());
        assert_eq!(state(&server, pane).epoch, epoch);
        assert_eq!(state(&server, pane).generation, generation);
        assert_eq!(broker_messages(&replies(&mut server, pane)), 0);
    }

    #[test]
    fn desired_view_can_request_native_for_stock_ansi_program() {
        let (mut server, pane, session) = fixture();
        server.tsp_broker_enabled = true;
        stock_hello(&mut server, pane);
        assert_desired_view(&mut server, pane, "detached");
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
        let epoch = state(&server, pane).epoch;
        let viewer = client(&mut server, session, false, false);
        assert_desired_view(&mut server, pane, "pending");
        let token = server
            .clients
            .get_mut(viewer)
            .unwrap()
            .tsp
            .request(RequestOwner::Detection);
        let hello =
            serde_json::from_value::<Hello>(json!({"v":1,"kinds":["col","text"],"credits":2}))
                .unwrap();
        server
            .clients
            .get_mut(viewer)
            .unwrap()
            .tsp
            .accept_hello(hello);
        assert_desired_view(&mut server, pane, "pending");
        server.clients.get_mut(viewer).unwrap().tsp.sentinel(token);
        recompute(&mut server);
        assert_desired_view(&mut server, pane, "native");
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
        assert!(state(&server, pane).switch.is_none());
        assert_eq!(state(&server, pane).epoch, epoch);
        assert!(replies(&mut server, pane).is_empty());

        server
            .clients
            .get_mut(viewer)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .set_size(40, 25, 0, 0);
        assert_desired_view(&mut server, pane, "ansi");
        server
            .clients
            .get_mut(viewer)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .set_size(80, 25, 0, 0);
        assert_desired_view(&mut server, pane, "native");
        state_mut(&mut server, pane).ui_pending = true;
        assert_desired_view(&mut server, pane, "ansi");
    }

    #[test]
    fn desired_view_tracks_two_stock_panes_through_split_zoom_and_close() {
        for kind in [LayoutType::Leftright, LayoutType::Topbottom] {
            let (mut server, a, session) = fixture();
            server.tsp_broker_enabled = true;
            let window = server.panes.get(a).unwrap().window;
            layout::init(&mut server, window, a);
            let (viewer, _viewer_master) = client_with_master(&mut server, session, true, false);
            let (observer, _observer_master) =
                client_with_master(&mut server, session, false, false);
            server
                .clients
                .get_mut(observer)
                .unwrap()
                .flags
                .insert(ClientFlags::CONTROL);
            stock_hello(&mut server, a);
            assert_desired_view(&mut server, a, "native");
            assert_eq!(state(&server, a).renderer, Renderer::Native);
            assert_eq!(replies(&mut server, a)[0]["r"], "hello");

            let cell = layout::split_pane(&mut server, a, kind, -1, SpawnFlags::default())
                .expect("space for second stock pane");
            let b = window::window_add_pane(&mut server, window, Some(a), 0, SpawnFlags::default())
                .unwrap();
            layout::assign_pane(&mut server, cell, b, false);
            assert!(window::window_set_active_pane(&mut server, window, b, true).unwrap());
            stock_hello(&mut server, b);
            assert_eq!(state(&server, b).renderer, Renderer::Ansi);
            assert!(replies(&mut server, b).is_empty());
            let a_hello = state(&server, a).program_hello.clone();
            let b_hello = state(&server, b).program_hello.clone();
            assert_eq!(b_hello, a_hello);
            let a_epoch = state(&server, a).epoch;
            let b_epoch = state(&server, b).epoch;
            let a_generation = state(&server, a).generation;
            let b_generation = state(&server, b).generation;

            let assert_views = |server: &mut Server, a_view: &str, b_view: &str| {
                recompute(server);
                for (pane, view, hello, epoch, generation) in [
                    (a, a_view, &a_hello, a_epoch, a_generation),
                    (b, b_view, &b_hello, b_epoch, b_generation),
                ] {
                    assert_desired_view(server, pane, view);
                    assert_eq!(
                        viewers(server, pane),
                        if view == "detached" {
                            vec![]
                        } else {
                            vec![viewer]
                        }
                    );
                    assert_eq!(display_contract(server, pane).0.is_some(), view == "native");
                    let state = state(server, pane);
                    assert!(state.stock && state.registered);
                    assert_eq!(&state.program_hello, hello);
                    assert_eq!(state.epoch, epoch);
                    assert_eq!(state.generation, generation);
                    assert!(state.switch.is_none());
                    assert_eq!(broker_messages(&replies(server, pane)), 0);
                }
                assert_eq!(
                    state(server, a).renderer,
                    if a_view == "detached" {
                        Renderer::Detached
                    } else {
                        Renderer::Native
                    }
                );
                // ANSI hello metadata must still permit a later native reprobe.
                assert_eq!(state(server, b).renderer, Renderer::Ansi);
            };
            assert_views(&mut server, "ansi", "ansi");
            assert!(window::window_zoom(&mut server, window, a).unwrap());
            assert_views(&mut server, "native", "detached");
            assert!(window::window_unzoom(&mut server, window, true).unwrap());
            assert_views(&mut server, "ansi", "ansi");
            assert!(window::window_zoom(&mut server, window, b).unwrap());
            assert_views(&mut server, "detached", "native");
            assert!(window::window_unzoom(&mut server, window, true).unwrap());
            assert_views(&mut server, "ansi", "ansi");

            layout::close_pane(&mut server, a);
            window::window_remove_pane(&mut server, window, a).unwrap();
            recompute(&mut server);
            assert_desired_view(&mut server, b, "native");
            assert_eq!(viewers(&server, b), vec![viewer]);
            assert!(display_contract(&server, b).0.is_some());
            assert_eq!(state(&server, b).renderer, Renderer::Ansi);
            assert_eq!(state(&server, b).program_hello, b_hello);
            assert!(state(&server, b).registered);
            assert!(state(&server, b).switch.is_none());
            assert!(replies(&mut server, b).is_empty());

            pane_send(&mut server, b, b'q', b_hello);
            assert_desired_view(&mut server, b, "native");
            assert_eq!(state(&server, b).renderer, Renderer::Native);
            assert_eq!(state(&server, b).epoch, b_epoch);
            assert_eq!(state(&server, b).generation, b_generation);
            let hello = replies(&mut server, b);
            assert_eq!(hello.len(), 1);
            assert_eq!(hello[0]["r"], "hello");
        }
    }

    #[test]
    fn desired_view_resolves_expired_probe_and_disabled_broker_to_ansi() {
        let (mut server, pane, session) = fixture();
        server.tsp_broker_enabled = true;
        let viewer = client(&mut server, session, false, false);
        let token = server
            .clients
            .get_mut(viewer)
            .unwrap()
            .tsp
            .request(RequestOwner::Detection);
        assert_desired_view(&mut server, pane, "pending");
        server.clients.get_mut(viewer).unwrap().tsp.expire(token);
        assert_desired_view(&mut server, pane, "ansi");
        server.clients.get_mut(viewer).unwrap().tsp.invalidate(2);
        server.tsp_broker_enabled = false;
        assert_desired_view(&mut server, pane, "ansi");
        server.clients.get_mut(viewer).unwrap().session = None;
        assert_desired_view(&mut server, pane, "detached");
    }

    #[test]
    fn stock_program_stays_native_through_plain_viewer_and_detach() {
        let (mut server, pane, session) = fixture();
        let tern = client(&mut server, session, true, false);
        stock_hello(&mut server, pane);
        let hello = replies(&mut server, pane);
        assert_eq!(hello.len(), 1);
        assert_eq!(hello[0]["r"], "hello");
        assert_eq!(hello[0]["v"], 1);
        assert!(state(&server, pane).stock);
        assert_eq!(state(&server, pane).renderer, Renderer::Native);
        pane_send(
            &mut server,
            pane,
            b'o',
            json!({"id":"view","mode":"screen"}),
        );
        let epoch = state(&server, pane).epoch;

        // A plain viewer cannot see the native view, and the program cannot be
        // told to switch: it stays native and its frames are acked by rmux.
        let plain = client(&mut server, session, false, false);
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Native);
        assert_eq!(state(&server, pane).epoch, epoch);
        assert!(state(&server, pane).switch.is_none());
        replies(&mut server, pane);
        pane_send(
            &mut server,
            pane,
            b'f',
            json!({"sf":"view","s":1,"ops":[["add","main","view",null,{"id":"main","k":"col"}]]}),
        );
        let out = replies(&mut server, pane);
        assert!(
            out.contains(&json!({"ev":"ack","sf":"view","s":1})),
            "{out:?}"
        );
        assert_eq!(broker_messages(&out), 0);

        // Every viewer gone: detached, frames still acked; Tern back: native.
        server.clients.get_mut(plain).unwrap().session = None;
        server.clients.get_mut(tern).unwrap().session = None;
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Detached);
        replies(&mut server, pane);
        pane_send(&mut server, pane, b'f', json!({"sf":"view","s":2,"ops":[]}));
        assert!(replies(&mut server, pane).contains(&json!({"ev":"ack","sf":"view","s":2})));
        server.clients.get_mut(tern).unwrap().session = Some(session);
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Native);
        assert_eq!(state(&server, pane).epoch, epoch);
        assert_eq!(broker_messages(&replies(&mut server, pane)), 0);
    }

    #[test]
    fn native_only_pane_note_lasts_while_the_surface_is_live() {
        let (mut server, pane, session) = fixture();
        client(&mut server, session, true, false);
        stock_hello(&mut server, pane);
        let redraw = |server: &mut Server| {
            let p = server.panes.get_mut(pane).unwrap();
            let set = p.flags.contains(crate::model::PaneFlags::REDRAW);
            p.flags.remove(crate::model::PaneFlags::REDRAW);
            set
        };
        recompute(&mut server);
        redraw(&mut server);
        assert_eq!(broker::native_only(&server, pane), None);
        pane_send(
            &mut server,
            pane,
            b'o',
            json!({"id":"view","mode":"inline","title":"omp"}),
        );
        recompute(&mut server);
        assert_eq!(broker::native_only(&server, pane).as_deref(), Some("omp"));
        assert!(redraw(&mut server), "the note needs a pane redraw");
        recompute(&mut server);
        assert!(!redraw(&mut server));
        pane_send(&mut server, pane, b'x', json!({"id":"view","keep":false}));
        recompute(&mut server);
        assert_eq!(broker::native_only(&server, pane), None);
        assert!(redraw(&mut server), "the grid comes back without the note");
    }

    #[test]
    fn stock_program_without_a_tsp_viewer_gets_no_hello_and_stays_ansi() {
        let (mut server, pane, session) = fixture();
        stock_hello(&mut server, pane);
        assert!(replies(&mut server, pane).is_empty());
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
        // It already painted rows; a Tern viewer arriving later cannot switch it.
        client(&mut server, session, true, false);
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
        assert!(replies(&mut server, pane).is_empty());
    }

    #[test]
    fn timeout_is_generation_guarded_and_never_commits_fake_ansi() {
        let (mut server, pane, session) = fixture();
        let id = client(&mut server, session, true, false);
        recompute(&mut server);
        let epoch = state(&server, pane).epoch;
        let generation = state(&server, pane).generation;
        timeout(&mut server, pane, generation + 1, epoch);
        assert!(!state(&server, pane).switch.as_ref().unwrap().failed);
        timeout(&mut server, pane, generation, epoch);
        assert!(state(&server, pane).switch.as_ref().unwrap().failed);
        assert_eq!(state(&server, pane).renderer, Renderer::Switching);
        assert!(server.clients.get(id).unwrap().message.text.is_some());
        assert!(server.clients.get(id).unwrap().tsp.projection.is_none());
        refresh(&mut server, id);
        assert!(state(&server, pane).epoch > epoch);
        assert!(!state(&server, pane).switch.as_ref().unwrap().failed);
    }

    #[test]
    fn full_pane_geometry_and_stable_writable_leader_are_required() {
        let (mut server, pane, session) = fixture();
        let readonly = client(&mut server, session, true, true);
        let first = client(&mut server, session, true, false);
        let second = client(&mut server, session, true, false);
        assert_eq!(display_contract(&server, pane).1, Some(first.min(second)));
        server.clients.get_mut(second).unwrap().activity_time = (1, 0);
        assert_eq!(display_contract(&server, pane).1, Some(second));
        server
            .clients
            .get_mut(first)
            .unwrap()
            .flags
            .insert(ClientFlags::READONLY);
        server
            .clients
            .get_mut(second)
            .unwrap()
            .flags
            .insert(ClientFlags::READONLY);
        assert_eq!(
            display_contract(&server, pane).1,
            Some(readonly.min(first).min(second))
        );
        server
            .clients
            .get_mut(first)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .set_size(40, 25, 0, 0);
        assert!(display_contract(&server, pane).0.is_none());
        assert!(viewers(&server, pane).contains(&first));
    }

    fn pane_send(server: &mut Server, pane: PaneId, verb: u8, value: serde_json::Value) {
        let message = WireMessage::json(verb, &value).encode();
        super::super::pane_message::pane_message(server, pane, &message[2..message.len() - 2]);
    }

    fn replies(server: &mut Server, pane: PaneId) -> Vec<serde_json::Value> {
        let bytes = std::mem::take(&mut server.panes.get_mut(pane).unwrap().output);
        let mut messages = Vec::new();
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            assert!(remaining.starts_with(b"\x1b_tsp;"));
            let end = remaining.windows(2).position(|b| b == b"\x1b\\").unwrap();
            let message = super::super::wire::parse(&remaining[2..end]).unwrap();
            messages.push(serde_json::from_slice(&message.body).unwrap());
            remaining = &remaining[end + 2..];
        }
        messages
    }

    #[test]
    fn stale_hello_does_not_change_program_metadata_or_reader() {
        let (mut server, pane, session) = fixture();
        let leader = client(&mut server, session, true, false);
        let contract = display_contract(&server, pane).0.unwrap();
        let hello = json!({"q":"hello","v":[1],"app":"old","features":[super::super::wire::BROKER_FEATURE]});
        let s = state_mut(&mut server, pane);
        s.epoch = 7;
        s.program_hello = hello.clone();
        s.contract = Some(contract.clone());
        s.leader = Some(leader);
        s.registered = false;
        s.answers_enabled = false;
        pane_send(
            &mut server,
            pane,
            b'q',
            json!({"q":"hello","v":[1],"app":"stale","features":[super::super::wire::BROKER_FEATURE],"rmuxEpoch":6}),
        );
        let s = state(&server, pane);
        assert!(!s.registered);
        assert!(!s.answers_enabled);
        assert_eq!(s.epoch, 7);
        assert_eq!(s.program_hello, hello);
        assert_eq!(s.contract, Some(contract));
        assert_eq!(s.leader, Some(leader));
        assert!(s.switch.is_none());
        assert_eq!(
            replies(&mut server, pane),
            vec![super::super::wire::probe_reply(6, false, Some(false))]
        );
    }

    #[test]
    fn hello_without_v1_remains_ansi_after_recompute() {
        let (mut server, pane, session) = fixture();
        client(&mut server, session, true, false);
        server.panes.get_mut(pane).unwrap().tsp = None;
        pane_send(
            &mut server,
            pane,
            b'q',
            json!({"q":"hello","v":[2],"features":[super::super::wire::BROKER_FEATURE]}),
        );
        let epoch = state(&server, pane).epoch;
        assert!(state(&server, pane).contract.is_none());
        assert_eq!(
            replies(&mut server, pane),
            vec![super::super::wire::probe_reply(epoch, false, None)]
        );
        assert!(ready(&mut server, pane, epoch, "ansi"));
        replies(&mut server, pane);
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
        assert!(state(&server, pane).contract.is_none());
        assert!(state(&server, pane).switch.is_none());
        assert_eq!(state(&server, pane).epoch, epoch);
    }

    #[test]
    fn initial_hello_chunks_join_before_registration_at_every_byte() {
        let body=serde_json::to_vec(&json!({"q":"hello","v":[1],"app":"é🙂","features":[super::super::wire::BROKER_FEATURE]})).unwrap();
        for split in 0..=body.len() {
            let (mut server, pane, _) = fixture();
            server.panes.get_mut(pane).unwrap().tsp = None;
            for (index, part) in [&body[..split], &body[split..]].into_iter().enumerate() {
                let mut message = WireMessage {
                    verb: b'q',
                    params: std::collections::BTreeMap::from([("c".into(), "hello".into())]),
                    body: part.to_vec(),
                };
                if index == 0 {
                    message.params.insert("m".into(), "1".into());
                }
                let encoded = message.encode();
                super::super::pane_message::pane_message(
                    &mut server,
                    pane,
                    &encoded[2..encoded.len() - 2],
                );
                if index == 0 {
                    assert!(!state(&server, pane).registered);
                    assert!(state(&server, pane).chunk.is_some());
                    assert!(server.panes.get(pane).unwrap().output.is_empty());
                }
            }
            assert!(state(&server, pane).registered);
            assert!(state(&server, pane).chunk.is_none());
            assert!(state(&server, pane).chunk_timer.is_none());
            assert_eq!(state(&server, pane).program_hello["app"], "é🙂");
            assert_eq!(
                replies(&mut server, pane),
                vec![super::super::wire::probe_reply(1, false, None)]
            );
        }
    }

    #[test]
    fn reader_lifetime_cancels_timers_and_invalidates_every_generation() {
        let endings: [fn(&mut Server, PaneId); 4] = [
            super::super::lifetime::pane_prompt,
            super::super::lifetime::pane_reset,
            super::super::lifetime::pane_exited,
            super::super::lifetime::pane_respawn,
        ];
        for end in endings {
            let (mut server, pane, _) = fixture();
            state_mut(&mut server, pane).answers_enabled = true;
            begin_initial(&mut server, pane, false);
            let switch_callback = state(&server, pane)
                .switch
                .as_ref()
                .unwrap()
                .timer
                .unwrap()
                .1;
            super::super::pane_message::pane_message(
                &mut server,
                pane,
                b"tsp;q;c=test;m=1;{\"q\":",
            );
            let chunk_callback = state(&server, pane).chunk_timer.unwrap().1;
            let generation = state(&server, pane).generation;
            end(&mut server, pane);
            let s = state(&server, pane);
            assert!(!s.registered);
            assert!(!s.answers_enabled);
            assert!(s.switch.is_none());
            assert!(s.chunk.is_none());
            assert!(s.chunk_timer.is_none());
            assert!(s.debts.is_empty());
            assert!(s.surfaces.is_empty());
            assert!(s.generation > generation);
            assert!(!server.deferred.contains_key(&switch_callback));
            assert!(!server.deferred.contains_key(&chunk_callback));
            pane_send(&mut server, pane, b'f', json!({"sf":"gone","s":9,"ops":[]}));
            super::super::lifetime::protocol_error(&mut server, pane, "late");
            assert!(server.panes.get(pane).unwrap().output.is_empty());
        }
    }

    #[test]
    fn listener_reopens_on_query_or_open_but_silent_surfaces_never_answer() {
        let (mut server, pane, _) = fixture();
        super::super::lifetime::pane_prompt(&mut server, pane);
        pane_send(
            &mut server,
            pane,
            b'o',
            json!({"id":"quiet","mode":"screen","listen":false}),
        );
        assert!(!state(&server, pane).registered);
        assert!(!state(&server, pane).answers_enabled);
        pane_send(
            &mut server,
            pane,
            b'f',
            json!({"sf":"quiet","s":3,"ops":[["no-such-op"]]}),
        );
        assert!(replies(&mut server, pane).is_empty());
        pane_send(&mut server, pane, b'q', json!({"q":"unknown"}));
        assert!(state(&server, pane).answers_enabled);
        super::super::lifetime::protocol_error_for(
            &mut server,
            pane,
            "quiet",
            Some(&json!({"sf":"quiet","s":3})),
        );
        assert!(replies(&mut server, pane).is_empty());
        pane_send(
            &mut server,
            pane,
            b'o',
            json!({"id":"live","mode":"screen"}),
        );
        pane_send(
            &mut server,
            pane,
            b'f',
            json!({"sf":"live","s":7,"ops":[["no-such-op"]]}),
        );
        let errors = replies(&mut server, pane);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["ev"], "error");
        assert_eq!(errors[0]["sf"], "live");
        assert_eq!(errors[0]["s"], 7);
        assert_eq!(errors[0]["op"], 0);
        assert!(errors[0]["msg"].is_string());
        pane_send(&mut server, pane, b'x', json!({"id":"live","keep":false}));
        // The surfaces doc names `x`'s surface by `id`; a field rmux ignored
        // before left omp's closed surface open.
        assert!(
            state(&server, pane)
                .surfaces
                .selected()
                .is_none_or(|sf| sf.wire_id != "live")
        );
        super::super::lifetime::protocol_error(&mut server, pane, "still reading");
        assert_eq!(
            replies(&mut server, pane),
            vec![json!({"ev":"error","msg":"still reading"})]
        );
    }

    #[test]
    fn leader_reports_presentation_without_restarting_renderer() {
        let (mut server, pane, session) = fixture();
        let first = client(&mut server, session, true, false);
        let second = client(&mut server, session, true, false);
        for (id, dark, width) in [(first, true, 8), (second, false, 9)] {
            let Capability::V1(hello) = &mut server.clients.get_mut(id).unwrap().tsp.capability
            else {
                panic!("fixture hello")
            };
            hello.dark = Some(dark);
            hello.reduce_motion = Some(!dark);
            hello.cell = Some(json!({"w":width,"h":16}));
        }
        server.clients.get_mut(first).unwrap().activity_time = (2, 0);
        let (contract, leader) = display_contract(&server, pane);
        let s = state_mut(&mut server, pane);
        s.contract = contract;
        s.leader = leader;
        s.renderer = Renderer::Native;
        s.surfaces
            .open(&json!({"id":"view","mode":"screen"}), None)
            .unwrap();
        super::super::client_runtime::report_view(&mut server, pane);
        replies(&mut server, pane);
        server.clients.get_mut(second).unwrap().activity_time = (3, 0);
        recompute(&mut server);
        let s = state(&server, pane);
        assert_eq!(s.epoch, 1);
        assert!(s.switch.is_none());
        assert_eq!(s.leader, Some(second));
        assert_eq!(
            s.contract.as_ref().unwrap().cell,
            Some(json!({"w":9,"h":16}))
        );
        assert_eq!(
            replies(&mut server, pane),
            vec![
                json!({"ev":"resize","sf":"view","cols":80,"cell":{"w":9,"h":16},"visible":true}),
                json!({"ev":"theme","dark":false}),
                json!({"ev":"motion","reduce":true})
            ]
        );
        recompute(&mut server);
        assert!(replies(&mut server, pane).is_empty());
        server.clients.get_mut(first).unwrap().activity_time = (4, 0);
        server.clients.get_mut(first).unwrap().tsp.capability = Capability::Unsupported;
        assert_eq!(display_contract(&server, pane).1, Some(second));
    }

    #[test]
    fn exited_native_inline_keeps_only_nonlistening_dead_snapshot() {
        let (mut server, pane, session) = fixture();
        let viewer = client(&mut server, session, true, false);
        let (contract, leader) = display_contract(&server, pane);
        let s = state_mut(&mut server, pane);
        s.contract = contract;
        s.leader = leader;
        s.renderer = Renderer::Native;
        s.answers_enabled = true;
        let inline = s
            .surfaces
            .open(&json!({"id":"inline","mode":"inline"}), Some(1))
            .unwrap()
            .opened
            .unwrap();
        s.surfaces.get_mut(inline).unwrap().document.apply_frame(&Frame{sf:"inline".into(),s:1,ops:vec![json!(["add","main","inline",null,{"id":"main","k":"col","c":[{"id":"text","k":"text","p":{"text":"last real state"}}]}])]},0).unwrap();
        s.surfaces
            .open(&json!({"id":"overlay","mode":"screen"}), None)
            .unwrap();
        s.debts
            .entry(inline)
            .or_default()
            .push_back(super::super::broker::DrawDebt {
                revision: 1,
                sequence: 1,
            });
        super::super::lifetime::pane_exited(&mut server, pane);
        let s = state(&server, pane);
        assert!(s.program_exited);
        assert!(!s.registered);
        assert!(!s.answers_enabled);
        assert!(s.debts.is_empty());
        assert_eq!(s.renderer, Renderer::Native);
        let retained = s.surfaces.selected().unwrap();
        assert_eq!(retained.wire_id, "inline");
        assert!(!retained.is_open());
        assert!(!retained.listens());
        assert_eq!(retained.open["listen"], false);
        assert!(s.surfaces.find("overlay").is_none());
        assert!(display_contract(&server, pane).0.is_some());
        server.clients.get_mut(viewer).unwrap().tsp.capability = Capability::Unsupported;
        assert!(display_contract(&server, pane).0.is_none());
        recompute(&mut server);
        assert!(state(&server, pane).switch.is_none());
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        super::super::lifetime::pane_respawn(&mut server, pane);
        assert!(!state(&server, pane).program_exited);
        assert_eq!(state(&server, pane).renderer, Renderer::Ansi);
        assert!(state(&server, pane).surfaces.is_empty());
        assert!(state(&server, pane).program_hello.is_null());
    }

    #[test]
    fn held_input_budget_includes_all_same_pane_tty_buffers_and_queued_keys() {
        let (mut server, pane, session) = fixture();
        let (a, master_a) = client_with_master(&mut server, session, false, false);
        let (b, master_b) = client_with_master(&mut server, session, false, false);
        begin_initial(&mut server, pane, false);
        let limit = super::super::input::INPUT_LIMIT;
        let prefix = vec![b'a'; limit - 32];
        assert!(super::super::input::hold_bytes(
            &mut server,
            pane,
            Some(a),
            &prefix
        ));
        assert!(super::super::input::reserve_input(&mut server, pane, 8));
        assert_eq!(
            rmux_sys::fd::write(master_a.as_fd(), b"aaaaaaaaaaaaaaaa").unwrap(),
            16
        );
        super::super::input::client_read_bound(&mut server, a);
        server
            .clients
            .get_mut(a)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .on_readable();
        assert_eq!(
            server
                .clients
                .get(a)
                .unwrap()
                .tty
                .as_ref()
                .unwrap()
                .input_len(),
            16
        );
        assert_eq!(
            rmux_sys::fd::write(master_b.as_fd(), b"bbbbbbbbbbbbbbbb").unwrap(),
            16
        );
        super::super::input::client_read_bound(&mut server, b);
        server
            .clients
            .get_mut(b)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .on_readable();
        assert_eq!(
            server
                .clients
                .get(b)
                .unwrap()
                .tty
                .as_ref()
                .unwrap()
                .input_len(),
            8
        );
        let buffered = server
            .clients
            .get(a)
            .unwrap()
            .tty
            .as_ref()
            .unwrap()
            .input_len()
            + server
                .clients
                .get(b)
                .unwrap()
                .tty
                .as_ref()
                .unwrap()
                .input_len();
        assert_eq!(
            state(&server, pane).held_bytes + state(&server, pane).pending_input_bytes + buffered,
            limit
        );
        super::super::input::client_read_bound(&mut server, a);
        super::super::input::client_read_bound(&mut server, b);
        assert!(
            !server
                .clients
                .get(a)
                .unwrap()
                .tty
                .as_ref()
                .unwrap()
                .wants_read()
        );
        assert!(
            !server
                .clients
                .get(b)
                .unwrap()
                .tty
                .as_ref()
                .unwrap()
                .wants_read()
        );
        assert!(super::super::input::client_can_decode(&server, a));
        assert!(!super::super::input::reserve_input(&mut server, pane, 1));
        assert!(super::super::input::reserve_decoded_input(
            &mut server,
            pane,
            a,
            1,
            1
        ));
        server
            .clients
            .get_mut(a)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .consume_input(1);
        assert_eq!(
            state(&server, pane).held_bytes
                + state(&server, pane).pending_input_bytes
                + server
                    .clients
                    .get(a)
                    .unwrap()
                    .tty
                    .as_ref()
                    .unwrap()
                    .input_len()
                + server
                    .clients
                    .get(b)
                    .unwrap()
                    .tty
                    .as_ref()
                    .unwrap()
                    .input_len(),
            limit
        );
        super::super::input::release_reservation(&mut server, pane, 1);
        assert!(super::super::input::hold_bytes(
            &mut server,
            pane,
            Some(a),
            b"a"
        ));
        super::super::input::release_reservation(&mut server, pane, 8);
        let a_bytes = server
            .clients
            .get(a)
            .unwrap()
            .tty
            .as_ref()
            .unwrap()
            .input_bytes()
            .to_vec();
        server
            .clients
            .get_mut(a)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .consume_input(a_bytes.len());
        assert!(super::super::input::hold_bytes(
            &mut server,
            pane,
            Some(a),
            &a_bytes
        ));
        let b_bytes = server
            .clients
            .get(b)
            .unwrap()
            .tty
            .as_ref()
            .unwrap()
            .input_bytes()
            .to_vec();
        server
            .clients
            .get_mut(b)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .consume_input(b_bytes.len());
        assert!(super::super::input::hold_bytes(
            &mut server,
            pane,
            Some(b),
            &b_bytes
        ));
        let epoch = state(&server, pane).epoch;
        assert!(ready(&mut server, pane, epoch, "ansi"));
        let output = &server.panes.get(pane).unwrap().output;
        let ready = WireMessage::json(b'r', &super::super::wire::ready_reply(epoch, true)).encode();
        assert!(output.starts_with(&ready));
        assert_eq!(
            &output[ready.len()..],
            [
                prefix.as_slice(),
                b"a",
                a_bytes.as_slice(),
                b_bytes.as_slice()
            ]
            .concat()
        );
        assert!(
            server
                .clients
                .get(a)
                .unwrap()
                .tty
                .as_ref()
                .unwrap()
                .wants_read()
        );
        assert!(
            server
                .clients
                .get(b)
                .unwrap()
                .tty
                .as_ref()
                .unwrap()
                .wants_read()
        );
    }

    #[test]
    fn departed_viewer_releases_draw_debt_only_with_remaining_real_draw() {
        let (mut server, pane, session) = fixture();
        let first = client(&mut server, session, true, false);
        let second = client(&mut server, session, true, false);
        let (contract, leader) = display_contract(&server, pane);
        let s = state_mut(&mut server, pane);
        s.contract = contract;
        s.leader = leader;
        s.renderer = Renderer::Native;
        let surface = s
            .surfaces
            .open(&json!({"id":"view","mode":"screen"}), None)
            .unwrap()
            .opened
            .unwrap();
        s.surfaces
            .get_mut(surface)
            .unwrap()
            .document
            .apply_frame(
                &Frame {
                    sf: "view".into(),
                    s: 7,
                    ops: vec![json!(["add","main","view",null,{"id":"main","k":"col"}])],
                },
                0,
            )
            .unwrap();
        s.debts
            .entry(surface)
            .or_default()
            .push_back(super::super::broker::DrawDebt {
                revision: 1,
                sequence: 7,
            });
        for client in [first, second] {
            let mut projection = super::super::projection::Projection::new(
                pane,
                "view",
                format!("rmux:c{client:?}:v1:s{}", surface.0),
                1,
                2,
            );
            let s = state_mut(&mut server, pane);
            let (_, coverage) = projection
                .next_messages(
                    &s.surfaces.get(surface).unwrap().document,
                    &mut s.blobs,
                    &Default::default(),
                    &json!({}),
                    0,
                )
                .unwrap()
                .unwrap();
            projection.note_enqueued(coverage.sequence);
            if client == first {
                projection.ack(coverage.sequence).unwrap();
            }
            let c = server.clients.get_mut(client).unwrap();
            c.tsp.projection_generation = 1;
            c.tsp.projection = Some(projection);
        }
        super::super::client_runtime::release_drawn(&mut server, pane);
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        server.clients.get_mut(second).unwrap().session = None;
        recompute(&mut server);
        let emitted = replies(&mut server, pane);
        assert!(emitted.contains(&json!({"ev":"ack","sf":"view","s":7})));
        assert!(state(&server, pane).debts[&surface].is_empty());
        state_mut(&mut server, pane)
            .debts
            .entry(surface)
            .or_default()
            .push_back(super::super::broker::DrawDebt {
                revision: 2,
                sequence: 20,
            });
        server.clients.get_mut(first).unwrap().session = None;
        recompute(&mut server);
        assert_eq!(state(&server, pane).renderer, Renderer::Detached);
        assert_eq!(state(&server, pane).debts[&surface].len(), 1);
        assert!(
            !replies(&mut server, pane)
                .iter()
                .any(|message| message["ev"] == "ack")
        );
    }

    #[test]
    fn deferred_input_follows_held_fifo_only_after_ready() {
        let (mut server, pane, _) = fixture();
        begin_initial(&mut server, pane, false);
        assert!(super::super::input::hold_bytes(
            &mut server,
            pane,
            None,
            b"held"
        ));
        assert!(super::super::input::defer_input(
            &mut server,
            pane,
            Box::new(move |server| server
                .panes
                .get_mut(pane)
                .unwrap()
                .output
                .extend_from_slice(b"deferred"))
        ));
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        let epoch = state(&server, pane).epoch;
        assert!(ready(&mut server, pane, epoch, "ansi"));
        let reply = WireMessage::json(b'r', &super::super::wire::ready_reply(epoch, true)).encode();
        assert_eq!(
            server.panes.get(pane).unwrap().output,
            [reply.as_slice(), b"helddeferred"].concat()
        );
        assert!(state(&server, pane).deferred_input.is_empty());
        assert_eq!(state(&server, pane).held_bytes, 0);
        assert!(!super::super::input::defer_input(
            &mut server,
            pane,
            Box::new(|_| {})
        ));
    }
}
