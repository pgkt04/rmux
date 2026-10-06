// Ported from tmux input.c, screen-write.c @ 8f25579c
use super::{
    broker::{DrawDebt, PaneTspState, now_ms, reply},
    wire::{self, Frame, WireMessage},
};
use crate::{ids::PaneId, model::Server};
use serde_json::{Value, json};
pub fn pane_message(server: &mut Server, pane: PaneId, payload: &[u8]) {
    let message = match wire::parse(payload) {
        Ok(m) => m,
        Err(e) => {
            super::lifetime::protocol_error(server, pane, &e);
            return;
        }
    };
    let mut initial_value = None;
    if server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .is_none()
    {
        if !matches!(message.verb, b'q' | b'o') {
            return;
        }
        if message.verb == b'q' && !message.params.get("m").is_some_and(|more| more == "1") {
            let Ok(value) = serde_json::from_slice::<Value>(&message.body) else {
                return;
            };
            if value["q"] != "hello" {
                return;
            }
            initial_value = Some(value);
        }
        let Some(p) = server.panes.get_mut(pane) else {
            return;
        };
        p.tsp = Some(PaneTspState::default());
    }
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let interruption = state
        .chunk
        .as_ref()
        .filter(|c| c.interrupted_by(&message))
        .map(|c| c.dropped_error("interrupted"));
    let joined = super::transport::join(&mut state.chunk, message, now_ms());
    if let Some(error) = interruption {
        super::lifetime::protocol_error(server, pane, &error);
    }
    let message = match joined {
        Ok(Some(m)) => m,
        Ok(None) => {
            schedule_chunk(server, pane);
            return;
        }
        Err(e) => {
            cancel_chunk_timer(server, pane);
            super::lifetime::protocol_error(server, pane, &e);
            return;
        }
    };
    cancel_chunk_timer(server, pane);
    if message.verb == b'b' {
        blob(server, pane, message);
        return;
    }
    if !matches!(message.verb, b'q' | b'o' | b'x' | b'f' | b't' | b's') {
        return;
    }
    let value = match initial_value {
        Some(value) => value,
        None => match serde_json::from_slice::<Value>(&message.body) {
            Ok(v) => v,
            Err(_) => {
                super::lifetime::protocol_error(server, pane, "invalid JSON");
                return;
            }
        },
    };
    let provisional = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .is_some_and(|s| {
            s.generation == 1
                && !s.registered
                && !s.answers_enabled
                && s.program_hello.is_null()
                && s.surfaces.is_empty()
        });
    if provisional && message.verb == b'q' && value["q"] != "hello" {
        server.panes.get_mut(pane).unwrap().tsp = None;
        return;
    }
    if message.verb == b'q' && value["q"] != "hello" {
        let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
        state.answers_enabled = true;
        state.program_exited = false;
    }
    match message.verb {
        b'q' => query(server, pane, value),
        b'o' => open(server, pane, value),
        b'x' => {
            let Some(sf) = value["id"].as_str() else {
                return;
            };
            let keep = value["keep"].as_bool().unwrap_or(true);
            let change = server
                .panes
                .get_mut(pane)
                .and_then(|p| p.tsp.as_mut())
                .map(|s| s.surfaces.close(sf, keep));
            if let Some(c) = change {
                super::lifetime::apply_change(server, pane, c)
            }
        }
        b'f' => frame(server, pane, value),
        b't' | b's' => metadata(server, pane, message.verb, value),
        _ => {}
    }
}
fn opted_hello(value: &Value) -> bool {
    value["q"] == "hello"
        && value["features"].as_array().is_some_and(|features| {
            features
                .iter()
                .any(|feature| feature == wire::BROKER_FEATURE)
        })
}
fn hello(server: &mut Server, pane: PaneId, value: Value) {
    let Some(epoch) = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .map(|s| s.epoch)
    else {
        return;
    };
    if let Some(requested) = value["rmuxEpoch"].as_u64().filter(|e| *e != epoch) {
        reply(
            server,
            pane,
            WireMessage::json(b'r', &wire::probe_reply(requested, false, Some(false))),
        );
        return;
    }
    let opt = opted_hello(&value);
    let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
    state.answers_enabled = true;
    state.program_exited = false;
    state.registered = true;
    state.stock = !opt;
    state.program_hello = value;
    let (contract, leader) = super::contract::display_contract(server, pane);
    let hello = contract.as_ref().map(|c| c.hello(epoch));
    let native = hello.is_some();
    let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
    state.contract = contract;
    state.leader = leader;
    if !opt {
        super::contract::begin_stock(server, pane, native);
        if let Some(hello) = hello {
            reply(server, pane, WireMessage::json(b'r', &hello));
        }
        return;
    }
    if state.switch.is_none() {
        super::contract::begin_initial(server, pane, native);
    }
    reply(
        server,
        pane,
        WireMessage::json(b'r', &wire::probe_reply(epoch, native, None)),
    );
    if let Some(hello) = hello {
        reply(server, pane, WireMessage::json(b'r', &hello));
    }
    super::contract::note_probe(server, pane, epoch, native);
}
fn open(server: &mut Server, pane: PaneId, value: Value) {
    if value["listen"].as_bool() != Some(false) {
        let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
        state.answers_enabled = true;
        state.program_exited = false;
    }
    let need = server
        .panes
        .get(pane)
        .and_then(|p| p.tsp.as_ref())
        .map(|s| s.surfaces.needs_anchor(&value));
    let needs_anchor = match need {
        Some(Ok(n)) => n,
        Some(Err(e)) => {
            super::lifetime::protocol_error_for(server, pane, &e.to_string(), Some(&value));
            return;
        }
        None => {
            rmux_util::log_debug!("tsp {pane:?}: open dropped, pane has no TSP state");
            return;
        }
    };
    let mut anchor = None;
    if needs_anchor {
        let p = server.panes.get_mut(pane).unwrap();
        let state = p.tsp.as_mut().unwrap();
        state.next_anchor += 1;
        let id = rmux_emu::grid::SurfaceAnchorId(state.next_anchor);
        let mut sink = rmux_emu::screen::write::ScreenOnlySink;
        let mut writer = rmux_emu::screen::write::ScreenWriteCtx::start(
            &mut p.base,
            &mut sink,
            Default::default(),
            &mut server.hyperlinks,
            #[cfg(feature = "sixel")]
            Some(&mut server.images),
        );
        if !writer.insert_surface_anchor(id) {
            rmux_util::log_debug!(
                "tsp {pane:?}: open dropped, no anchor row (alternate {}, cursor {},{})",
                writer.screen.is_alternate(),
                writer.screen.cx,
                writer.screen.cy
            );
            return;
        }
        writer.finish();
        anchor = Some(id.0);
    }
    let result = server
        .panes
        .get_mut(pane)
        .unwrap()
        .tsp
        .as_mut()
        .unwrap()
        .surfaces
        .open(&value, anchor);
    match result {
        Ok(c) => {
            let state = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
            state.answers_enabled = value["listen"].as_bool().unwrap_or(true);
            state.program_exited = false;
            if let (Some(id), Some(contract)) = (c.opened, state.contract.as_ref()) {
                if let Some(surface) = state.surfaces.get_mut(id) {
                    surface.document.set_kinds(contract.kinds.clone());
                }
            }
            super::lifetime::apply_change(server, pane, c)
        }
        Err(e) => super::lifetime::protocol_error_for(server, pane, &e.to_string(), Some(&value)),
    }
}
fn frame(server: &mut Server, pane: PaneId, value: Value) {
    let context = json!({"sf":value.get("sf"),"s":value.get("s")});
    let frame = match serde_json::from_value::<Frame>(value) {
        Ok(f) => f,
        Err(_) => {
            super::lifetime::protocol_error_for(server, pane, "invalid frame", Some(&context));
            return;
        }
    };
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let Some(handle) = state.surfaces.find_open(&frame.sf) else {
        super::lifetime::protocol_error_for(server, pane, "unknown surface", Some(&context));
        return;
    };
    let surface = state.surfaces.get_mut(handle).unwrap();
    let result = surface.document.apply_frame(&frame, now_ms());
    let listens = surface.listens();
    let refs = surface.document.blob_references();
    let applied = match result {
        Ok(a) => a,
        Err(e) => {
            super::lifetime::protocol_error_for(server, pane, &e, Some(&context));
            return;
        }
    };
    state.blobs.set_references(&frame.sf, refs);
    if listens {
        state.debts.entry(handle).or_default().push_back(DrawDebt {
            revision: applied.revision,
            sequence: frame.s,
        });
    }
    let stock = state.stock;
    let mut drawn = false;
    for id in &server.client_order {
        let Some(client) = server.clients.get_mut(*id) else {
            continue;
        };
        if let Some(projection) = client
            .tsp
            .projection
            .as_mut()
            .filter(|p| p.pane == pane && p.logical == frame.sf)
        {
            drawn = true;
            if projection.queue_frame(&frame, &applied).is_err() {
                projection.failed = true;
            }
        }
    }
    if stock && !drawn {
        super::client_runtime::release_undrawn(server, pane);
    }
    if listens {
        for error in &applied.errors {
            super::lifetime::protocol_error_for(
                server,
                pane,
                &error.msg,
                Some(&json!({"sf":frame.sf,"s":error.s,"op":error.op})),
            );
        }
    }
    let change = server
        .panes
        .get_mut(pane)
        .unwrap()
        .tsp
        .as_mut()
        .unwrap()
        .surfaces
        .retain();
    super::lifetime::apply_change(server, pane, change);
}
fn query(server: &mut Server, pane: PaneId, value: Value) {
    match value["q"].as_str() {
        Some("hello") => hello(server, pane, value),
        Some("rmux-ready") => {
            if let (Some(epoch), Some(renderer)) =
                (value["epoch"].as_u64(), value["renderer"].as_str())
            {
                super::contract::ready(server, pane, epoch, renderer);
            }
        }
        Some("blobs") => {
            let ids: Vec<String> = value["ids"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let have = server
                .panes
                .get(pane)
                .and_then(|p| p.tsp.as_ref())
                .map(|s| s.blobs.have(&ids))
                .unwrap_or_default();
            reply(
                server,
                pane,
                WireMessage::json(b'r', &json!({"r":"blobs","have":have})),
            );
        }
        _ => {}
    }
}
fn blob(server: &mut Server, pane: PaneId, message: WireMessage) {
    let Some(id) = message.params.get("id") else {
        return;
    };
    let result = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .map(|s| {
            s.blobs.insert(
                id,
                message.params.get("mime").map(String::as_str),
                &message.body,
            )
        });
    if let Some(Err(error)) = result {
        super::lifetime::protocol_error(server, pane, &error);
    }
}
fn metadata(server: &mut Server, pane: PaneId, verb: u8, value: Value) {
    let Some(sf) = value["sf"].as_str() else {
        return;
    };
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) else {
        return;
    };
    let Some(handle) = state.surfaces.find_open(sf) else {
        super::lifetime::protocol_error_for(server, pane, "unknown surface", Some(&value));
        return;
    };
    let document = &mut state.surfaces.get_mut(handle).unwrap().document;
    let result = if verb == b't' {
        document.set_palette(value.clone())
    } else {
        let Some(name) = value["name"].as_str() else {
            return;
        };
        document.set_sheet(name, value.get("css"))
    };
    if let Err(error) = result {
        let context = if verb == b's' {
            json!({"sf":sf,"sheet":value.get("name")})
        } else {
            json!({"sf":sf})
        };
        super::lifetime::protocol_error_for(server, pane, &error, Some(&context));
        return;
    }
    super::project::queue_metadata(server, pane, handle, verb, &value);
    let change = server
        .panes
        .get_mut(pane)
        .unwrap()
        .tsp
        .as_mut()
        .unwrap()
        .surfaces
        .retain();
    super::lifetime::apply_change(server, pane, change);
}
pub fn cancel_chunk_timer(server: &mut Server, pane: PaneId) {
    if let Some((timer, id)) = server
        .panes
        .get_mut(pane)
        .and_then(|p| p.tsp.as_mut())
        .and_then(|s| s.chunk_timer.take())
    {
        server.event_loop.cancel(timer);
        server.deferred.remove(&id);
    }
}
fn schedule_chunk(server: &mut Server, pane: PaneId) {
    cancel_chunk_timer(server, pane);
    let Some(state) = server.panes.get(pane).and_then(|p| p.tsp.as_ref()) else {
        return;
    };
    let generation = state.generation;
    let Some(deadline) = state.chunk.as_ref().map(|c| c.deadline()) else {
        return;
    };
    let timer = crate::server::event_loop::schedule_deferred(
        server,
        std::time::Duration::from_millis(deadline.saturating_sub(now_ms())),
        Box::new(move |server| {
            let Some(state) = server
                .panes
                .get_mut(pane)
                .and_then(|p| p.tsp.as_mut())
                .filter(|s| s.generation == generation)
            else {
                return;
            };
            state.chunk_timer = None;
            if super::transport::expire(&mut state.chunk, now_ms()) {
                super::lifetime::protocol_error(server, pane, "chunk sequence timed out");
            }
        }),
    );
    if let Some(state) = server.panes.get_mut(pane).and_then(|p| p.tsp.as_mut()) {
        state.chunk_timer = Some(timer);
    }
}
