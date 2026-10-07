// Ported from tmux tty.c, tty-keys.c, server-client.c @ 8f25579c
use super::{
    broker::{self, Renderer},
    client::{Capability, RequestOwner},
    surface::SurfaceId,
    wire::{Hello, WireMessage},
};
use crate::{
    client::ClientFlags,
    ids::{ClientId, PaneId},
    model::Server,
};
use rmux_tty::{
    keys::{Da1Owner, Recognition},
    tty::{ProtocolTransaction, TtyFlags},
};
use serde_json::{Value, json};
use std::{collections::VecDeque, time::Duration};

fn cancel_probe_timer(server: &mut Server, client: ClientId) {
    let timer = server
        .clients
        .get_mut(client)
        .and_then(|c| c.tsp.timer.take());
    if let Some(timer) = timer {
        if let Some(crate::server::event_loop::LoopAction::Deferred(id)) =
            server.event_loop.timer_action(timer)
        {
            server.deferred.remove(id);
        }
        server.event_loop.cancel(timer);
    }
}

pub fn probe_client(server: &mut Server, client: ClientId) {
    if !server.tsp_broker_enabled {
        return;
    }
    let Some(c) = server.clients.get(client) else {
        return;
    };
    if c.tsp.projection.is_some()
        || c.flags.contains(ClientFlags::CONTROL)
        || !c
            .tty
            .as_ref()
            .is_some_and(|tty| tty.flags().contains(TtyFlags::STARTED))
    {
        return;
    }
    cancel_probe_timer(server, client);
    let c = server.clients.get_mut(client).expect("client checked");
    let tty = c.tty.as_mut().expect("tty checked");
    let generation = tty.protocol_generation();
    if c.tsp.generation != generation {
        c.tsp.invalidate(generation);
    }
    for request in &mut c.tsp.requests {
        if matches!(request.owner, RequestOwner::Detection) {
            request.completed = true;
            request.hello = None;
        }
    }
    c.tsp.rebuild_attempted = false;
    c.tsp.failed_logical = None;
    c.tsp.diagnostic = None;
    c.tsp.visible = true;
    let token = c.tsp.request(RequestOwner::Detection);
    let query = WireMessage::json(b'q', &json!({"q":"hello","v":[1],"app":"rmux"}));
    if tty
        .queue_da1(
            Da1Owner::Token(token),
            ProtocolTransaction::new(query.encode()).control(),
        )
        .is_err()
    {
        c.tsp.requests.pop_back();
        c.tsp.capability = Capability::Unsupported;
        c.tsp.diagnostic = Some("TSP probe output queue full".into());
        broker::recompute(server);
        return;
    }
    let (timer, _) = crate::server::event_loop::schedule_deferred(
        server,
        Duration::from_secs(1),
        Box::new(move |server| {
            let Some(c) = server.clients.get_mut(client) else {
                return;
            };
            if c.tsp.generation != generation
                || !c
                    .tty
                    .as_ref()
                    .is_some_and(|tty| tty.protocol_generation() == generation)
            {
                return;
            }
            if !c
                .tsp
                .requests
                .iter()
                .any(|r| r.token == token && r.generation == generation && !r.completed)
            {
                return;
            }
            c.tsp.timer = None;
            c.tsp.expire(token);
            broker::recompute(server);
        }),
    );
    if let Some(c) = server.clients.get_mut(client) {
        c.tsp.timer = Some(timer);
    }
}

pub fn client_sync(server: &mut Server, client: ClientId) {
    let Some(c) = server.clients.get(client) else {
        return;
    };
    let generation = c
        .tty
        .as_ref()
        .map_or(c.tsp.generation, |tty| tty.protocol_generation());
    let started = !c.flags.intersects(
        ClientFlags::DEAD | ClientFlags::EXIT | ClientFlags::EXITED | ClientFlags::SUSPENDED,
    ) && c
        .tty
        .as_ref()
        .is_some_and(|tty| tty.flags().contains(TtyFlags::STARTED));
    let changed = generation != c.tsp.generation;
    if changed || !started {
        cancel_probe_timer(server, client);
        broker::close_projection(server, client);
        if let Some(c) = server.clients.get_mut(client) {
            c.tsp.invalidate(generation);
        }
        broker::recompute(server);
    }
    let physical = server.clients.get(client).is_some_and(|c| {
        c.tty.is_some()
            && !c.flags.intersects(
                ClientFlags::DEAD
                    | ClientFlags::EXIT
                    | ClientFlags::EXITED
                    | ClientFlags::SUSPENDED,
            )
    });
    if started
        && physical
        && server
            .clients
            .get(client)
            .is_some_and(|c| c.tsp.capability == Capability::Unknown)
    {
        probe_client(server, client);
    }
    if started && physical {
        broker::project_pending(server, client);
    }
}

pub fn client_sentinel(server: &mut Server, client: ClientId, raw: &[u8]) {
    let owner = server
        .clients
        .get_mut(client)
        .and_then(|c| c.tty.as_mut())
        .and_then(|tty| tty.resolve_da1());
    match owner {
        Some(Da1Owner::Discovery) => {
            if let Recognition::Complete(_, discovery) = rmux_tty::keys::parse_primary_da(raw) {
                crate::client::tty_io::apply_discovery(server, client, discovery);
            }
        }
        Some(Da1Owner::Token(token)) => {
            let detection = server.clients.get(client).is_some_and(|c| {
                c.tsp.requests.front().is_some_and(|r| {
                    r.token == token && r.owner == RequestOwner::Detection && !r.completed
                })
            });
            let resolved = server
                .clients
                .get_mut(client)
                .and_then(|c| c.tsp.sentinel(token));
            if detection {
                cancel_probe_timer(server, client);
            }
            if resolved.is_some() {
                broker::recompute(server);
            }
        }
        None => {}
    }
}

fn outer_surface(outer: &str) -> Option<SurfaceId> {
    outer.rsplit_once(":s")?.1.parse().ok().map(SurfaceId)
}

fn mapped_surface(server: &Server, client: ClientId, sf: &str) -> Option<(PaneId, SurfaceId)> {
    let c = server.clients.get(client)?;
    let tty = c.tty.as_ref()?;
    if !tty.flags().contains(TtyFlags::STARTED)
        || tty.protocol_generation() != c.tsp.generation
        || !c.flags.contains(ClientFlags::ATTACHED)
        || c.flags.intersects(
            ClientFlags::CONTROL
                | ClientFlags::DEAD
                | ClientFlags::EXIT
                | ClientFlags::EXITED
                | ClientFlags::SUSPENDED,
        )
    {
        return None;
    }
    let projection = c.tsp.projection.as_ref()?;
    if !projection.is_open()
        || projection.generation != c.tsp.projection_generation
        || projection.outer != sf
    {
        return None;
    }
    let session = server.sessions.get(c.session?)?;
    let window = server.winlinks.get(session.current?)?.window;
    if server.windows.get(window)?.active != Some(projection.pane) {
        return None;
    }
    let pane = server.panes.get(projection.pane)?;
    if pane.window != window {
        return None;
    }
    let state = pane.tsp.as_ref()?;
    if !state.registered || state.renderer != Renderer::Native || state.switch.is_some() {
        return None;
    }
    let id = outer_surface(&projection.outer)?;
    let surface = state.surfaces.get(id)?;
    if state.surfaces.selected_id() != Some(id)
        || !surface.listens()
        || surface.wire_id != projection.logical
    {
        return None;
    }
    Some((projection.pane, id))
}

pub fn client_protocol_fault(server: &mut Server, client: ClientId) {
    cancel_probe_timer(server, client);
    broker::close_projection(server, client);
    if let Some(c) = server.clients.get_mut(client) {
        c.tsp.capability = Capability::Unsupported;
        c.tsp.confirmed_blobs.clear();
        for request in &mut c.tsp.requests {
            request.completed = true;
            request.hello = None;
        }
        c.tsp.diagnostic = Some("TSP client protocol failure".into());
    }
    broker::recompute(server);
}

fn rebuild_projection(server: &mut Server, client: ClientId) {
    let Some(c) = server.clients.get_mut(client) else {
        return;
    };
    let Some(projection) = c.tsp.projection.as_mut() else {
        return;
    };
    let identity = projection
        .outer
        .rsplit_once(":v")
        .map(|(prefix, _)| prefix)
        .unwrap_or(&projection.outer);
    let identity = format!(
        "{identity}:s{}",
        outer_surface(&projection.outer).map_or(0, |id| id.0)
    );
    if c.tsp.rebuild_attempted && c.tsp.failed_logical.as_deref() == Some(&identity) {
        client_protocol_fault(server, client);
        return;
    }
    c.tsp.rebuild_attempted = true;
    c.tsp.failed_logical = Some(identity);
    projection.failed = true;
    broker::close_projection(server, client);
    broker::project_pending(server, client);
}

fn drawn_prefix(
    debts: &mut VecDeque<super::broker::DrawDebt>,
    viewers: impl Iterator<Item = Option<u64>>,
) -> Option<u64> {
    let mut minimum = None;
    for viewer in viewers {
        let revision = viewer?;
        minimum = Some(minimum.map_or(revision, |old: u64| old.min(revision)));
    }
    let revision = minimum?;
    let mut sequence = None;
    while debts.front().is_some_and(|debt| debt.revision <= revision) {
        sequence = debts.pop_front().map(|debt| debt.sequence);
    }
    sequence
}

pub fn release_drawn(server: &mut Server, pane: PaneId) {
    let Some(state) = server.panes.get(pane).and_then(|p| p.tsp.as_ref()) else {
        return;
    };
    if !state.registered || state.renderer != Renderer::Native || state.switch.is_some() {
        return;
    }
    let Some(surface) = state
        .surfaces
        .selected()
        .filter(|surface| surface.listens())
    else {
        return;
    };
    let surface_id = surface.id;
    let logical = surface.wire_id.clone();
    let viewers = broker::viewers(server, pane);
    let mut minimum = None;
    for client in viewers {
        let Some(projection) = server
            .clients
            .get(client)
            .and_then(|c| c.tsp.projection.as_ref())
        else {
            return;
        };
        if mapped_surface(server, client, &projection.outer) != Some((pane, surface_id)) {
            return;
        }
        minimum = Some(minimum.map_or(projection.drawn_revision, |old: u64| {
            old.min(projection.drawn_revision)
        }));
    }
    let Some(minimum) = minimum else { return };
    let state = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .expect("pane checked");
    let Some(debts) = state.debts.get_mut(&surface_id) else {
        return;
    };
    if let Some(sequence) = drawn_prefix(debts, std::iter::once(Some(minimum))) {
        let ack = json!({"ev":"ack","sf":logical,"s":sequence});
        broker::event(server, pane, &ack);
    }
}

/// Ack every outstanding frame without a viewer draw. A stock program is not
/// drawn by anyone (no viewer, or one that needs the grid) and cannot be told
/// to wait, so it must not stall on its credits.
pub fn release_undrawn(server: &mut Server, pane: PaneId) {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let mut acks = Vec::new();
    for (handle, debts) in &mut state.debts {
        let Some(last) = debts.drain(..).next_back() else {
            continue;
        };
        if let Some(surface) = state.surfaces.get(*handle).filter(|s| s.listens()) {
            acks.push(json!({"ev":"ack","sf":surface.wire_id,"s":last.sequence}));
        }
    }
    for ack in acks {
        broker::event(server, pane, &ack);
    }
}

fn accept_blobs(server: &mut Server, client: ClientId, value: &Value) {
    let Some(c) = server.clients.get(client) else {
        return;
    };
    let Some(request) = c.tsp.requests.front() else {
        return;
    };
    if request.completed || request.generation != c.tsp.generation {
        return;
    }
    let RequestOwner::Replay { pane, projection } = request.owner else {
        return;
    };
    if !c
        .tsp
        .projection
        .as_ref()
        .is_some_and(|p| p.pane == pane && p.generation == projection)
    {
        return;
    }
    let Some(have) = value.get("have").and_then(Value::as_array) else {
        return;
    };
    let ids: Vec<String> = have
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let Some(state) = server.panes.get(pane).and_then(|p| p.tsp.as_ref()) else {
        return;
    };
    let verified = state.blobs.have(&ids);
    if let Some(c) = server.clients.get_mut(client) {
        c.tsp
            .confirmed_blobs
            .extend(verified.into_iter().map(|id| id.to_ascii_lowercase()));
    }
}

pub fn report_view(server: &mut Server, pane: PaneId) {
    let Some(p) = server.panes.get(pane) else {
        return;
    };
    let Some(state) = p
        .tsp
        .as_ref()
        .filter(|s| s.registered && s.switch.is_none())
    else {
        return;
    };
    let Some(surface) = state.surfaces.selected().filter(|s| s.listens()) else {
        return;
    };
    let hello = state
        .leader
        .and_then(|id| server.clients.get(id))
        .and_then(|c| c.tsp.hello());
    let visible = broker::viewers(server, pane)
        .iter()
        .any(|id| server.clients.get(*id).is_some_and(|c| c.tsp.visible));
    let resize = json!({"ev":"resize","sf":surface.wire_id,"cols":p.sx,"cell":hello.and_then(|h| h.cell.as_ref()),"visible":visible});
    let dark = hello.and_then(|h| h.dark);
    let motion = hello.and_then(|h| h.reduce_motion);
    let state = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .expect("pane checked");
    let resized = state.reported_view.as_ref() != Some(&resize);
    let themed = dark.is_some() && state.reported_theme != dark;
    let moved = motion.is_some() && state.reported_motion != motion;
    state.reported_view = Some(resize.clone());
    state.reported_theme = dark;
    state.reported_motion = motion;
    if resized {
        broker::event(server, pane, &resize);
    }
    if themed {
        broker::event(server, pane, &json!({"ev":"theme","dark":dark}));
    }
    if moved {
        broker::event(server, pane, &json!({"ev":"motion","reduce":motion}));
    }
}

fn cache_global(server: &mut Server, client: ClientId, value: &Value) {
    let Some(c) = server.clients.get_mut(client) else {
        return;
    };
    let Capability::V1(hello) = &mut c.tsp.capability else {
        return;
    };
    match value.get("ev").and_then(Value::as_str) {
        Some("theme") => {
            if let Some(dark) = value.get("dark").and_then(Value::as_bool) {
                hello.dark = Some(dark);
            }
        }
        Some("motion") => {
            if let Some(reduce) = value.get("reduce").and_then(Value::as_bool) {
                hello.reduce_motion = Some(reduce);
            }
        }
        _ => return,
    }
    broker::recompute(server);
}

fn client_gone(
    server: &mut Server,
    client: ClientId,
    pane: PaneId,
    surface_id: SurfaceId,
    value: &Value,
) {
    let Some(ids) = value.get("ids").and_then(Value::as_array) else {
        return;
    };
    let Some(nodes) = ids
        .iter()
        .map(|id| id.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    if nodes.is_empty() {
        return;
    }
    let Some(projection) = server
        .clients
        .get(client)
        .and_then(|c| c.tsp.projection.as_ref())
    else {
        return;
    };
    if nodes.iter().any(|id| id == &projection.outer) {
        rebuild_projection(server, client);
        return;
    }
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let change = match state.surfaces.evict_nodes(surface_id, &nodes) {
        Ok(change) => change,
        Err(_) => {
            rebuild_projection(server, client);
            return;
        }
    };
    super::lifetime::apply_change(server, pane, change);
}

pub fn client_message(server: &mut Server, client: ClientId, verb: u8, body: &[u8]) {
    if !matches!(verb, b'r' | b'e') {
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        client_protocol_fault(server, client);
        return;
    };
    if !value.is_object() {
        client_protocol_fault(server, client);
        return;
    }
    if verb == b'r' {
        match value.get("r").and_then(Value::as_str) {
            Some("hello") => {
                if let Ok(hello) = serde_json::from_value::<Hello>(value)
                    && let Some(c) = server.clients.get_mut(client)
                {
                    c.tsp.accept_hello(hello);
                }
            }
            Some("blobs") => accept_blobs(server, client, &value),
            _ => {}
        }
        return;
    }
    let Some(name) = value.get("ev").and_then(Value::as_str) else {
        return;
    };
    if matches!(name, "rmux-view" | "rmux-ready" | "rmux-probe") {
        return;
    }
    if matches!(name, "theme" | "motion") && value.get("sf").is_none() {
        cache_global(server, client, &value);
        return;
    }
    let Some(sf) = value.get("sf").and_then(Value::as_str) else {
        return;
    };
    let Some((pane, surface_id)) = mapped_surface(server, client, sf) else {
        return;
    };
    match name {
        "ack" => {
            let Some(sequence) = value.get("s").and_then(Value::as_u64) else {
                return;
            };
            let Some(projection) = server
                .clients
                .get_mut(client)
                .and_then(|c| c.tsp.projection.as_mut())
            else {
                return;
            };
            if projection.ack(sequence).is_err() {
                return;
            }
            release_drawn(server, pane);
            broker::project_pending(server, client);
        }
        "gone" => client_gone(server, client, pane, surface_id, &value),
        _ => client_event(server, client, pane, surface_id, name, &value),
    }
}

fn client_event(
    server: &mut Server,
    client: ClientId,
    pane: PaneId,
    surface_id: SurfaceId,
    name: &str,
    value: &Value,
) {
    if matches!(name, "resize" | "visible") {
        if let Some(c) = server.clients.get_mut(client) {
            if name == "visible" {
                if let Some(visible) = value.get("visible").and_then(Value::as_bool) {
                    c.tsp.visible = visible;
                }
            } else {
                c.tsp.visible = value
                    .get("visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
            }
            if name == "resize"
                && let Capability::V1(hello) = &mut c.tsp.capability
                && let Some(cell) = value.get("cell").filter(|cell| valid_cell(cell))
            {
                hello.cell = Some(cell.clone());
            }
        }
        broker::recompute(server);
        report_view(server, pane);
        return;
    }
    if name == "error" {
        let mapped = server
            .clients
            .get(client)
            .and_then(|c| c.tsp.projection.as_ref())
            .expect("projection checked")
            .map_error(value);
        match mapped {
            Ok(Some(error)) => broker::event(server, pane, &error),
            Ok(None) => {}
            Err(super::projection::ProjectionError::StatusBar) => {
                if let Some(c) = server.clients.get_mut(client) {
                    c.tsp.bar_failed = true;
                    c.tsp.diagnostic = Some("TSP status bar rejected".into());
                    if let Some(projection) = c.tsp.projection.as_mut() {
                        projection.set_bar(None);
                        projection.reconcile();
                    }
                }
                broker::project_pending(server, client);
                return;
            }
            Err(error) => {
                if let Some(c) = server.clients.get_mut(client) {
                    c.tsp.diagnostic = Some(format!("TSP projection error mapping: {error}"));
                }
            }
        }
        rebuild_projection(server, client);
        return;
    }
    if !matches!(
        name,
        "edit" | "undo" | "send" | "focus" | "action" | "change" | "select" | "activate" | "toggle"
    ) {
        return;
    }
    if server
        .clients
        .get(client)
        .is_none_or(|c| c.flags.contains(ClientFlags::READONLY))
    {
        return;
    }
    let Some(routed) = server
        .clients
        .get(client)
        .and_then(|c| c.tsp.projection.as_ref())
        .and_then(|p| p.route_event(value))
    else {
        return;
    };
    let Some(id) = routed.get("id").and_then(Value::as_str) else {
        return;
    };
    if !server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .and_then(|state| state.surfaces.get(surface_id))
        .is_some_and(|surface| surface.document.has(id))
    {
        return;
    }
    broker::event(server, pane, &routed);
}

fn valid_cell(cell: &Value) -> bool {
    ["w", "h"].iter().all(|key| {
        cell.get(key)
            .and_then(Value::as_f64)
            .is_some_and(|n| n.is_finite() && n > 0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::super::{
        broker::{DrawDebt, PaneTspState},
        projection::Projection,
        wire::Frame,
    };
    use super::*;
    use crate::{
        client::Client,
        ids::SessionId,
        model::{session, spawn::SpawnFlags, window},
    };
    use rmux_tty::tty::{Tty, TtyHostInfo};
    use std::os::fd::AsFd;

    fn fixture() -> (Server, PaneId, SessionId, SurfaceId) {
        let mut server = Server::new();
        let options = server.options.create(Some(server.options.global_s));
        let session = session::session_create(
            &mut server,
            session::SessionCreate {
                prefix: None,
                name: Some(b"routing".to_vec()),
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
        crate::layout::init(&mut server, window, pane);
        let link = session::session_attach(&mut server, session, window, 0).unwrap();
        session::session_set_current(&mut server, session, Some(link));
        let mut state = PaneTspState {
            registered: true,
            renderer: Renderer::Native,
            program_hello: json!({"q":"hello","v":[1],"features":[crate::tsp::wire::BROKER_FEATURE]}),
            ..PaneTspState::default()
        };
        let id = state
            .surfaces
            .open(&json!({"id":"logical","mode":"screen"}), None)
            .unwrap()
            .opened
            .unwrap();
        let frame = Frame {
            sf: "logical".into(),
            s: 7,
            ops: vec![
                json!(["add","edit","logical",null,{"id":"edit","k":"editor","p":{"text":"draft"}}]),
            ],
        };
        assert!(
            state
                .surfaces
                .get_mut(id)
                .unwrap()
                .document
                .apply_frame(&frame, 0)
                .unwrap()
                .errors
                .is_empty()
        );
        state.debts.insert(
            id,
            VecDeque::from([DrawDebt {
                revision: 1,
                sequence: 7,
            }]),
        );
        server.panes.get_mut(pane).unwrap().tsp = Some(state);
        (server, pane, session, id)
    }

    fn add_client(server: &mut Server, session: SessionId) -> ClientId {
        let (_master, slave, _) = rmux_sys::pty::openpty().unwrap();
        let tio = rmux_sys::TermiosState::get(slave.as_fd()).unwrap();
        let mut tty = Tty::new(slave, tio, TtyHostInfo::default());
        let caps = [
            "am=1",
            "clear=C",
            "cup=<p%p1%d,%p2%d>",
            "csr=<r%p1%d,%p2%d>",
            "cud1=D",
            "cub1=L",
            "cuf1=R",
            "cuu1=U",
            "cnorm=N",
            "civis=I",
            "sgr0=Z",
        ]
        .iter()
        .map(|s| rmux_util::bytes::ByteString::from(*s))
        .collect();
        tty.open(
            &mut server.tparm,
            b"fixture",
            &caps,
            &rmux_tty::tty::TtyOptions::default(),
            None,
        )
        .unwrap();
        tty.flags_mut().insert(TtyFlags::STARTED);
        tty.set_size(80, 24, 0, 0);
        let generation = tty.protocol_generation();
        let mut c = Client::new(None, (0, 0));
        c.session = Some(session);
        c.flags.insert(ClientFlags::ATTACHED);
        c.tsp.invalidate(generation);
        c.tsp.capability = Capability::V1(
            serde_json::from_value(json!({"v":1,"kinds":["col","editor"],"credits":2})).unwrap(),
        );
        c.tsp.visible = true;
        c.tsp.generation = generation;
        c.tty = Some(tty);
        let id = server.clients.insert(c).unwrap();
        server.client_order.push_back(id);
        id
    }

    fn projection(
        server: &mut Server,
        client: ClientId,
        pane: PaneId,
        surface: SurfaceId,
    ) -> String {
        let generation = {
            let c = server.clients.get_mut(client).unwrap();
            c.tsp.projection_generation += 1;
            c.tsp.projection_generation
        };
        let outer = format!("rmux:c{:?}:p0:v{}:s{}", client, generation, surface.0);
        let mut projection = Projection::new(pane, "logical", &outer, generation, 2);
        let confirmed = server
            .clients
            .get(client)
            .unwrap()
            .tsp
            .confirmed_blobs
            .clone();
        let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
        let revision = state.surfaces.get(surface).unwrap().document.revision;
        let (_, coverage) = projection
            .next_messages(
                &state.surfaces.get(surface).unwrap().document,
                &mut state.blobs,
                &confirmed,
                &json!({}),
                0,
            )
            .unwrap()
            .unwrap();
        projection.note_enqueued(coverage.sequence);
        if let Some(debt) = state.debts.get_mut(&surface) {
            for entry in debt.iter_mut() {
                if entry.revision == 0 || entry.revision > revision {
                    entry.revision = revision.max(1);
                }
            }
        }
        server.clients.get_mut(client).unwrap().tsp.projection = Some(projection);
        outer
    }

    fn event(server: &mut Server, client: ClientId, value: Value) {
        client_message(server, client, b'e', &serde_json::to_vec(&value).unwrap());
    }

    #[test]
    fn credit_prefix_requires_every_viewer_and_nonempty_set() {
        let mut debts = VecDeque::from([
            DrawDebt {
                revision: 1,
                sequence: 5,
            },
            DrawDebt {
                revision: 4,
                sequence: 99,
            },
        ]);
        assert_eq!(drawn_prefix(&mut debts, [].into_iter()), None);
        assert_eq!(drawn_prefix(&mut debts, [Some(4), None].into_iter()), None);
        assert_eq!(
            drawn_prefix(&mut debts, [Some(4), Some(0)].into_iter()),
            None
        );
        assert_eq!(
            drawn_prefix(&mut debts, [Some(4), Some(1)].into_iter()),
            Some(5)
        );
        assert_eq!(
            drawn_prefix(&mut debts, [Some(4), Some(4)].into_iter()),
            Some(99)
        );
        assert_eq!(drawn_prefix(&mut debts, [Some(9)].into_iter()), None);
    }

    #[test]
    fn two_draws_required_and_future_ack_never_reaches_program() {
        let (mut server, pane, session, surface) = fixture();
        let a = add_client(&mut server, session);
        let b = add_client(&mut server, session);
        let sa = projection(&mut server, a, pane, surface);
        release_drawn(&mut server, pane);
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        let sb = projection(&mut server, b, pane, surface);
        event(&mut server, a, json!({"ev":"ack","sf":sa,"s":99}));
        assert_eq!(
            server
                .clients
                .get(a)
                .unwrap()
                .tsp
                .projection
                .as_ref()
                .unwrap()
                .drawn_revision,
            0
        );
        event(&mut server, a, json!({"ev":"ack","sf":sa,"s":1}));
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        event(&mut server, b, json!({"ev":"ack","sf":sb,"s":1}));
        assert_eq!(
            server.panes.get(pane).unwrap().output,
            WireMessage::json(b'e', &json!({"ev":"ack","sf":"logical","s":7})).encode()
        );
        event(&mut server, b, json!({"ev":"ack","sf":sb,"s":1}));
        assert_eq!(
            server.panes.get(pane).unwrap().tsp.as_ref().unwrap().debts[&surface].len(),
            0
        );
    }

    #[test]
    fn absent_viewers_and_hidden_or_replaced_surface_keep_debt() {
        let (mut server, pane, session, surface) = fixture();
        release_drawn(&mut server, pane);
        assert_eq!(
            server.panes.get(pane).unwrap().tsp.as_ref().unwrap().debts[&surface].len(),
            1
        );
        let client = add_client(&mut server, session);
        let outer = projection(&mut server, client, pane, surface);
        let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
        state.surfaces.close("logical", false);
        let new_surface = state
            .surfaces
            .open(&json!({"id":"logical","mode":"screen"}), None)
            .unwrap()
            .opened
            .unwrap();
        assert_ne!(surface, new_surface);
        event(&mut server, client, json!({"ev":"ack","sf":outer,"s":1}));
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        assert_eq!(
            server
                .clients
                .get(client)
                .unwrap()
                .tsp
                .projection
                .as_ref()
                .unwrap()
                .drawn_revision,
            0
        );
    }

    #[test]
    fn mutation_checks_readonly_node_generation_and_selected_window() {
        let (mut server, pane, session, surface) = fixture();
        let client = add_client(&mut server, session);
        let sf = projection(&mut server, client, pane, surface);
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .insert(ClientFlags::READONLY);
        event(
            &mut server,
            client,
            json!({"ev":"edit","sf":sf,"id":"edit","text":"blocked"}),
        );
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .remove(ClientFlags::READONLY);
        event(
            &mut server,
            client,
            json!({"ev":"edit","sf":sf,"id":"missing","text":"blocked"}),
        );
        event(
            &mut server,
            client,
            json!({"ev":"edit","sf":"stale","id":"edit","text":"blocked"}),
        );
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        let original_generation = server
            .clients
            .get(client)
            .unwrap()
            .tsp
            .projection_generation;
        server
            .clients
            .get_mut(client)
            .unwrap()
            .tsp
            .projection_generation += 1;
        event(
            &mut server,
            client,
            json!({"ev":"edit","sf":sf,"id":"edit","text":"blocked"}),
        );
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        server
            .clients
            .get_mut(client)
            .unwrap()
            .tsp
            .projection_generation = original_generation;
        let other = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let link = session::session_attach(&mut server, session, other, 1).unwrap();
        server.sessions.get_mut(session).unwrap().current = Some(link);
        event(
            &mut server,
            client,
            json!({"ev":"edit","sf":sf,"id":"edit","text":"blocked"}),
        );
        assert!(server.panes.get(pane).unwrap().output.is_empty());
    }

    #[test]
    fn root_mapping_never_rewrites_arbitrary_values() {
        let (mut server, pane, session, surface) = fixture();
        let client = add_client(&mut server, session);
        let sf = projection(&mut server, client, pane, surface);
        event(
            &mut server,
            client,
            json!({"ev":"action","sf":sf,"id":sf,"act":"pick","value":sf,"values":{"sf":sf}}),
        );
        assert_eq!(server.panes.get(pane).unwrap().output, WireMessage::json(b'e', &json!({"ev":"action","sf":"logical","id":"logical","act":"pick","value":sf,"values":{"sf":sf}})).encode());
    }

    #[test]
    fn timed_out_probe_retains_sentinel_and_late_hello_cannot_revive() {
        let (mut server, _, session, _) = fixture();
        let client = add_client(&mut server, session);
        probe_client(&mut server, client);
        let c = server.clients.get_mut(client).unwrap();
        let token = c.tsp.requests.front().unwrap().token;
        let timer = c.tsp.timer.unwrap();
        let crate::server::event_loop::LoopAction::Deferred(id) =
            *server.event_loop.timer_action(timer).unwrap()
        else {
            panic!("typed timeout")
        };
        server.deferred.remove(&id).unwrap()(&mut server);
        assert_eq!(server.clients.get(client).unwrap().tsp.format(), "no");
        client_message(
            &mut server,
            client,
            b'r',
            br#"{"r":"hello","v":1,"kinds":["editor"],"credits":2}"#,
        );
        assert_eq!(
            server
                .clients
                .get(client)
                .unwrap()
                .tsp
                .requests
                .front()
                .unwrap()
                .token,
            token
        );
        client_sentinel(&mut server, client, b"\x1b[?1;2c");
        assert!(server.clients.get(client).unwrap().tsp.requests.is_empty());
        assert_eq!(server.clients.get(client).unwrap().tsp.format(), "no");
    }

    #[test]
    fn discovery_fifo_cannot_complete_tsp_and_live_projection_skips_probe() {
        let (mut server, pane, session, surface) = fixture();
        let client = add_client(&mut server, session);
        server
            .clients
            .get_mut(client)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .queue_da1(
                Da1Owner::Discovery,
                ProtocolTransaction::new(Vec::new()).control(),
            )
            .unwrap();
        probe_client(&mut server, client);
        client_message(
            &mut server,
            client,
            b'r',
            br#"{"r":"hello","v":1,"kinds":["editor"],"credits":2}"#,
        );
        client_sentinel(&mut server, client, b"\x1b[?1;2c");
        assert_eq!(server.clients.get(client).unwrap().tsp.format(), "unknown");
        client_sentinel(&mut server, client, b"\x1b[?1;2c");
        assert_eq!(server.clients.get(client).unwrap().tsp.format(), "v1");
        assert!(server.clients.get(client).unwrap().tsp.timer.is_none());
        projection(&mut server, client, pane, surface);
        probe_client(&mut server, client);
        assert!(server.clients.get(client).unwrap().tsp.requests.is_empty());
    }

    #[test]
    fn stale_timer_and_dead_tty_cannot_mutate_a_new_generation() {
        let (mut server, _, session, _) = fixture();
        let client = add_client(&mut server, session);
        probe_client(&mut server, client);
        let timer = server.clients.get(client).unwrap().tsp.timer.unwrap();
        let crate::server::event_loop::LoopAction::Deferred(id) =
            *server.event_loop.timer_action(timer).unwrap()
        else {
            panic!("typed timeout")
        };
        let callback = server.deferred.remove(&id).unwrap();
        server.clients.get_mut(client).unwrap().tsp.invalidate(2);
        callback(&mut server);
        assert_eq!(server.clients.get(client).unwrap().tsp.format(), "unknown");
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .insert(ClientFlags::DEAD);
        client_sync(&mut server, client);
        assert!(server.clients.get(client).unwrap().tsp.requests.is_empty());
        assert!(server.clients.get(client).unwrap().tsp.timer.is_none());
    }

    #[test]
    fn malformed_reply_unknown_event_and_unowned_hello_are_consumed() {
        let (mut server, pane, session, _) = fixture();
        let client = add_client(&mut server, session);
        client_message(
            &mut server,
            client,
            b'r',
            br#"{"r":"hello","v":1,"credits":2}"#,
        );
        assert_eq!(server.clients.get(client).unwrap().tsp.format(), "v1");
        client_message(
            &mut server,
            client,
            b'e',
            br#"{"ev":"unknown","text":"never type"}"#,
        );
        client_message(&mut server, client, b'?', b"raw text");
        assert!(server.panes.get(pane).unwrap().output.is_empty());
        client_message(
            &mut server,
            client,
            b'r',
            br#"{"r":"hello","v":1} trailing"#,
        );
        assert_eq!(server.clients.get(client).unwrap().tsp.format(), "no");
        let output = &server.panes.get(pane).unwrap().output;
        assert!(output.is_empty() || output.starts_with(b"\x1b_tsp;e;"));
        assert!(
            !output
                .windows(b"trailing".len())
                .any(|bytes| bytes == b"trailing")
        );
    }
}
