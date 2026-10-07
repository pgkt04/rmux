// rmux extension: a native client sees its status line as a TSP status strip
// in the projected surface's dock, since the projection covers the whole tty.
use super::wire::Hello;
use crate::{
    client::ClientFlags,
    ids::{ClientId, SessionId},
    model::{Server, WinlinkFlags},
    ui::status::{STATUS_LINES_LIMIT, status_line_size, status_redraw},
};
use rmux_emu::{
    grid::Grid,
    style::{StyleRange, StyleRangeType},
};
use serde_json::{Map, Value, json};

pub const BAR_ID: &str = "rmux:bar";

pub fn supported(hello: &Hello) -> bool {
    hello.features.contains("dock")
        && ["col", "status", "seg"]
            .iter()
            .all(|kind| hello.kinds.contains(*kind))
}

/// Redraw the client's status line and give it to its projection. True when
/// the bar changed.
pub fn refresh(server: &mut Server, id: ClientId) -> bool {
    let Some(c) = server.clients.get(id) else {
        return false;
    };
    if c.tsp.projection.is_none() {
        return false;
    }
    let wanted =
        !c.tsp.bar_failed && c.status.active.is_none() && c.tsp.hello().is_some_and(supported);
    let bar = if wanted {
        status_redraw(server, id);
        build(server, id)
    } else {
        None
    };
    server
        .clients
        .get_mut(id)
        .and_then(|c| c.tsp.projection.as_mut())
        .is_some_and(|p| p.set_bar(bar))
}

/// The native half of `check_redraw`: of the client's cells, only the status
/// line has a native view.
pub fn redraw(server: &mut Server, id: ClientId) {
    let status = ClientFlags::REDRAWSTATUS | ClientFlags::REDRAWSTATUSALWAYS;
    let Some(c) = server.clients.get_mut(id) else {
        return;
    };
    if c.tsp.projection.is_none() || !c.flags.intersects(status) {
        return;
    }
    c.flags.remove(status);
    if refresh(server, id) {
        super::project::project_pending(server, id);
    }
}

fn build(server: &Server, id: ClientId) -> Option<Value> {
    let c = server.clients.get(id)?;
    let session = c.session?;
    let grid = &c.status.screen.grid;
    let lines = (status_line_size(server, id) as usize)
        .min(STATUS_LINES_LIMIT)
        .min(grid.sy() as usize);
    let strips: Vec<Value> = (0..lines)
        .filter_map(|y| {
            let segs = segments(
                server,
                session,
                grid,
                &c.status.entries[y].ranges.0,
                y as u32,
            );
            (!segs.is_empty())
                .then(|| json!({"id": format!("{BAR_ID}:{y}"), "k": "status", "c": segs}))
        })
        .collect();
    (!strips.is_empty()).then(|| json!({"id": BAR_ID, "k": "col", "c": strips}))
}

/// One segment per style range, plus any text drawn outside the ranges.
fn segments(
    server: &Server,
    session: SessionId,
    grid: &Grid,
    ranges: &[StyleRange],
    y: u32,
) -> Vec<Value> {
    let width = grid.sx();
    let mut ranges: Vec<&StyleRange> = ranges
        .iter()
        .filter(|r| r.start < r.end.min(width))
        .collect();
    ranges.sort_by_key(|r| r.start);
    let line = Line {
        server,
        session,
        grid,
        y,
    };
    let mut segs = Vec::new();
    let mut x = 0;
    for range in ranges {
        if range.start < x {
            continue;
        }
        line.push(&mut segs, x, range.start, None);
        let end = range.end.min(width);
        line.push(&mut segs, range.start, end, Some(range));
        x = end;
    }
    line.push(&mut segs, x, width, None);
    segs
}

struct Line<'a> {
    server: &'a Server,
    session: SessionId,
    grid: &'a Grid,
    y: u32,
}
impl Line<'_> {
    fn push(&self, segs: &mut Vec<Value>, start: u32, end: u32, range: Option<&StyleRange>) {
        if start >= end {
            return;
        }
        let cells = self.grid.view_string_cells(start, self.y, end - start);
        let text = String::from_utf8_lossy(&cells);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let kind = range.map(|r| r.range_type);
        let right = match kind {
            Some(StyleRangeType::Left) => false,
            Some(StyleRangeType::Right) => true,
            _ => start + end > self.grid.sx(),
        };
        let window = range
            .filter(|r| r.range_type == StyleRangeType::Window)
            .and_then(|r| self.window(r.argument));
        // Tern drops the lowest priority first when the strip is narrow.
        let priority = match (kind, window) {
            (_, Some((true, _))) => 4,
            (Some(StyleRangeType::Left), _) => 3,
            (_, Some(_)) => 2,
            (Some(StyleRangeType::Right), _) => 1,
            _ => 0,
        };
        let mut p = Map::new();
        match window {
            Some((true, _)) => {
                p.insert("tone".into(), "accent".into());
                p.insert("spans".into(), json!([{"t": text, "s": "accent strong"}]));
            }
            Some((false, flags)) if flags.intersects(WinlinkFlags::BELL) => {
                p.insert("spans".into(), json!([{"t": text, "s": "error"}]));
            }
            Some((false, flags))
                if flags.intersects(WinlinkFlags::ACTIVITY | WinlinkFlags::SILENCE) =>
            {
                p.insert("spans".into(), json!([{"t": text, "s": "warning"}]));
            }
            _ => {
                p.insert("text".into(), text.into());
            }
        }
        if right {
            p.insert("side".into(), "right".into());
        }
        p.insert("priority".into(), priority.into());
        segs.push(json!({
            "id": format!("{BAR_ID}:{}:{}", self.y, segs.len()),
            "k": "seg",
            "p": p,
        }));
    }

    /// Whether the window range's winlink is current, and its alert flags.
    fn window(&self, index: u32) -> Option<(bool, WinlinkFlags)> {
        let session = self.server.sessions.get(self.session)?;
        let wl = *session.windows.get(&i32::try_from(index).ok()?)?;
        let flags = self.server.winlinks.get(wl)?.flags;
        Some((session.current == Some(wl), flags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::Client,
        model::{session, spawn::SpawnFlags, window},
    };

    fn segs(bar: &Value) -> &[Value] {
        bar["c"][0]["c"].as_array().unwrap()
    }
    fn seg<'a>(bar: &'a Value, prefix: &str) -> &'a Value {
        segs(bar)
            .iter()
            .find(|seg| {
                let p = &seg["p"];
                p["text"]
                    .as_str()
                    .or(p["spans"][0]["t"].as_str())
                    .is_some_and(|text| text.starts_with(prefix))
            })
            .unwrap_or_else(|| panic!("no segment {prefix} in {bar}"))
    }

    #[test]
    fn bar_marks_the_current_window_and_keeps_the_status_sides() {
        let mut server = Server::new();
        let options = server.options.create(Some(server.options.global_s));
        let s = session::session_create(
            &mut server,
            session::SessionCreate {
                prefix: None,
                name: Some(b"work".to_vec()),
                cwd: b"/".to_vec(),
                environment: crate::options::environment::Environment::default(),
                options,
                termios: None,
            },
        );
        let mut links = Vec::new();
        for (index, name) in [b"edit", b"logs"].iter().enumerate() {
            let w = window::window_create(&mut server, 80, 23, 0, 0).unwrap();
            server.windows.get_mut(w).unwrap().name = name.to_vec();
            let pane =
                window::window_add_pane(&mut server, w, None, 0, SpawnFlags::default()).unwrap();
            server.windows.get_mut(w).unwrap().active = Some(pane);
            crate::layout::init(&mut server, w, pane);
            links.push(session::session_attach(&mut server, s, w, index as i32).unwrap());
        }
        session::session_set_current(&mut server, s, Some(links[0]));
        server.winlinks.get_mut(links[1]).unwrap().flags = WinlinkFlags::BELL;
        crate::ui::status::status_update_cache(&mut server, s);
        let mut c = Client::new(None, (0, 0));
        c.session = Some(s);
        c.flags.insert(ClientFlags::ATTACHED);
        c.tty_sx = 80;
        c.tty_sy = 24;
        let id = server.clients.insert(c).unwrap();

        status_redraw(&mut server, id);
        let bar = build(&server, id).unwrap();
        assert_eq!(bar["id"], BAR_ID);
        assert_eq!(bar["c"].as_array().unwrap().len(), 1);

        let session = seg(&bar, "[work]");
        assert!(session["p"].get("side").is_none());
        let current = seg(&bar, "0:edit");
        assert_eq!(current["p"]["tone"], "accent");
        assert_eq!(current["p"]["spans"][0]["s"], "accent strong");
        let bell = seg(&bar, "1:logs");
        assert_eq!(bell["p"]["spans"][0]["s"], "error");
        assert!(bell["p"].get("tone").is_none());
        // Narrow strips drop the other windows before the current one.
        assert!(current["p"]["priority"].as_u64() > bell["p"]["priority"].as_u64());
        let right = segs(&bar).last().unwrap();
        assert_eq!(right["p"]["side"], "right");
        assert!(
            segs(&bar)
                .iter()
                .filter(|seg| seg["p"]["side"] == "right")
                .all(|seg| seg["p"]["priority"].as_u64() < session["p"]["priority"].as_u64())
        );

        server.sessions.get_mut(s).unwrap().statuslines = 0;
        assert_eq!(build(&server, id), None);
    }
}
