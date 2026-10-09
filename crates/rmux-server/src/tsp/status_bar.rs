// rmux extension: a native client sees its status line as a TSP status strip
// in the projected surface's dock, since the projection covers the whole tty.
use super::wire::Hello;
use crate::{
    client::{Client, ClientFlags},
    ids::{ClientId, SessionId},
    model::Server,
    ui::status::{
        STATUS_LINES_LIMIT, status_line_size, status_message_redraw, status_prompt_line_at,
        status_prompt_native, status_prompt_redraw, status_redraw,
    },
};
use rmux_emu::{
    cell::{GridAttributes as A, GridCell, GridCellFlags},
    colour::{Colour, ColourFlags},
    grid::Grid,
    style::{StyleRange, StyleRangeType},
};
use serde_json::{Map, Value, json};
use std::fmt::Write;

pub const BAR_ID: &str = "rmux:bar";
pub const BAR_SHEET: &str = "rmux-bar";
/// The chat skins hold dock strips to the transcript measure; the bar spans
/// the pane like a status line.
pub const BAR_CSS: &str = "[data-id='rmux:bar']>.sf-status{max-width:none!important;width:100%!important;margin:0!important}";

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
    let wanted = !c.tsp.bar_failed && c.tsp.hello().is_some_and(supported);
    let styles = c
        .tsp
        .hello()
        .is_some_and(|h| h.features.contains("styles") && h.kinds.contains("el"));
    let app_edit = server
        .panes
        .get(c.tsp.projection.as_ref().expect("projection checked").pane)
        .and_then(|p| p.tsp.as_ref())
        .and_then(|state| state.program_hello.get("features"))
        .and_then(Value::as_array)
        .is_some_and(|features| features.iter().any(|feature| feature == "edit"));
    let bar = if wanted {
        let mut prompt = if server.clients.get(id).is_some_and(|c| c.prompt.is_some()) {
            status_prompt_redraw(server, id);
            status_prompt_native(server, id)
        } else {
            None
        };
        if !app_edit && let Some(props) = prompt.as_mut() {
            props["readonly"] = true.into();
        }
        if server
            .clients
            .get(id)
            .is_some_and(|c| c.message.text.is_some())
        {
            status_message_redraw(server, id);
        } else if server.clients.get(id).is_some_and(|c| !c.prompt.is_some()) {
            status_redraw(server, id);
        }
        build(server, id, prompt, styles)
    } else {
        None
    };
    server
        .clients
        .get_mut(id)
        .and_then(|c| c.tsp.projection.as_mut())
        .is_some_and(|p| {
            p.bar_styles = styles;
            let (node, css) =
                bar.map_or((None, BAR_CSS.to_owned()), |bar| (Some(bar.node), bar.css));
            let changed = p.set_bar_css(css);
            p.set_bar(node) || changed || p.bar_pending()
        })
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

struct Bar {
    node: Value,
    css: String,
}

fn build(
    server: &mut Server,
    id: ClientId,
    mut prompt: Option<Value>,
    styled: bool,
) -> Option<Bar> {
    if server.clients.get(id)?.message.text.is_some()
        && let Some(props) = prompt.as_mut()
    {
        props["readonly"] = true.into();
        props["ghost"] = Value::Null;
    }
    let c = server.clients.get(id)?;
    let session = c.session?;
    let grid = &c.status.active().grid;
    let overlay = c.message.text.is_some() || c.prompt.is_some();
    let lines = (status_line_size(server, id).max(u32::from(overlay)) as usize)
        .min(STATUS_LINES_LIMIT)
        .min(grid.sy() as usize);
    let overlay_line = status_prompt_line_at(server, id).min(lines.saturating_sub(1) as u32);
    let mut strips = Vec::new();
    let mut css = BAR_CSS.to_owned();
    if styled {
        css.push_str("[data-id='rmux:bar'] .sf-seg{padding:0;gap:0;border-radius:0;color:inherit}[data-id='rmux:bar'] .sf-seg-k{display:block;gap:0}[data-id='rmux:bar'] .sf-el{white-space:pre}");
    }
    for y in 0..lines {
        if y as u32 == overlay_line
            && let Some(props) = &prompt
        {
            strips.push(json!({
                "id": format!("rmux:prompt:{}", c.prompt.generation()),
                "k": "input", "p": props,
            }));
            if c.message.text.is_none() {
                continue;
            }
        }
        let ranges = if overlay && y as u32 == overlay_line {
            &[][..]
        } else {
            &c.status.entries[y].ranges.0
        };
        let segs = segments(
            server,
            session,
            c,
            grid,
            ranges,
            y as u32,
            styled.then_some(&mut css),
        );
        if !segs.is_empty() {
            let strip_id = format!("{BAR_ID}:{y}");
            let base = if overlay && y as u32 == overlay_line {
                grid.view_get_cell(0, y as u32)
            } else {
                c.status.style
            };
            let style = CellStyle::new(c, base);
            let transparent = style.bg.is_default() && !style.attr.contains(A::REVERSE);
            if styled {
                write!(css, "[data-id='{strip_id}']{{").unwrap();
                CellStyle {
                    attr: style.attr & A::REVERSE,
                    ..style
                }
                .write_css(&mut css);
                css.push_str("border:0!important}");
            }
            strips.push(json!({"id": strip_id, "k": "status", "p": {"transparent": transparent}, "c": segs}));
        }
    }
    (!strips.is_empty()).then(|| Bar {
        node: json!({"id": BAR_ID, "k": "col", "c": strips}),
        css,
    })
}

/// One segment per style range, plus any text drawn outside the ranges.
fn segments(
    server: &Server,
    session: SessionId,
    client: &Client,
    grid: &Grid,
    ranges: &[StyleRange],
    y: u32,
    mut css: Option<&mut String>,
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
        client,
        grid,
        y,
    };
    let mut segs = Vec::new();
    let mut x = 0;
    for range in ranges {
        if range.start < x {
            continue;
        }
        line.push(&mut segs, x, range.start, None, css.as_deref_mut());
        let end = range.end.min(width);
        line.push(&mut segs, range.start, end, Some(range), css.as_deref_mut());
        x = end;
    }
    line.push(&mut segs, x, width, None, css);
    segs
}

struct Line<'a> {
    server: &'a Server,
    session: SessionId,
    client: &'a Client,
    grid: &'a Grid,
    y: u32,
}
impl Line<'_> {
    fn push(
        &self,
        segs: &mut Vec<Value>,
        start: u32,
        end: u32,
        range: Option<&StyleRange>,
        css: Option<&mut String>,
    ) {
        if start >= end {
            return;
        }
        let styled = css.is_some();
        let nonblank = |x| {
            let cell = self.grid.view_get_cell(x, self.y);
            !cell.flags.contains(GridCellFlags::PADDING) && cell.data.bytes() != b" "
        };
        let first = (start..end).find(|&x| nonblank(x));
        if first.is_none() && !(styled && range.is_some()) {
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
            (_, Some(true)) => 4,
            (Some(StyleRangeType::Left), _) => 3,
            (_, Some(false)) => 2,
            (Some(StyleRangeType::Right), _) => 1,
            _ => 0,
        };
        let mut p = Map::new();
        let id = format!("{BAR_ID}:{}:{}", self.y, segs.len());
        let mut children = Vec::new();
        if let Some(css) = css {
            let (start, end) = if range.is_some() {
                (start, end)
            } else {
                let first = first.unwrap_or(end);
                let last = (first..end)
                    .rfind(|&x| nonblank(x))
                    .map_or(first, |x| {
                        x + u32::from(self.grid.view_get_cell(x, self.y).data.width)
                    })
                    .min(end);
                (first, last)
            };
            let mut x = start;
            while x < end {
                if self
                    .grid
                    .view_get_cell(x, self.y)
                    .flags
                    .contains(GridCellFlags::PADDING)
                {
                    x += 1;
                    continue;
                }
                let style = CellStyle::new(self.client, self.grid.view_get_cell(x, self.y));
                let from = x;
                x += 1;
                while x < end {
                    let cell = self.grid.view_get_cell(x, self.y);
                    if !cell.flags.contains(GridCellFlags::PADDING)
                        && CellStyle::new(self.client, cell) != style
                    {
                        break;
                    }
                    x += 1;
                }
                let bytes = self.grid.view_string_cells(from, self.y, x - from);
                let run_id = format!("{id}:{}", children.len());
                write!(css, "[data-id='{run_id}']{{").unwrap();
                style.write_css(css);
                css.push('}');
                children.push(json!({"id": run_id, "k": "el", "p": {
                    "tag": "span", "text": String::from_utf8_lossy(&bytes),
                }}));
            }
        } else {
            let bytes = self.grid.view_string_cells(start, self.y, end - start);
            p.insert("text".into(), String::from_utf8_lossy(&bytes).trim().into());
        }
        if right {
            p.insert("side".into(), "right".into());
        }
        p.insert("priority".into(), priority.into());
        segs.push(json!({
            "id": id,
            "k": "seg",
            "p": p,
            "c": children,
        }));
    }

    fn window(&self, index: u32) -> Option<bool> {
        let session = self.server.sessions.get(self.session)?;
        let wl = *session.windows.get(&i32::try_from(index).ok()?)?;
        Some(session.current == Some(wl))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    fg: Colour,
    bg: Colour,
    us: Colour,
    attr: A,
}

impl CellStyle {
    fn new(client: &Client, cell: GridCell) -> Self {
        let resolve = |colour: Colour| {
            if colour.raw() & ColourFlags::THEME.bits() as i32 == 0 {
                return colour;
            }
            let mapped = Colour(
                client
                    .theme_colours
                    .get((colour.raw() & 255) as usize)
                    .copied()
                    .unwrap_or(-1),
            );
            if mapped == Colour::NONE || mapped.raw() & ColourFlags::THEME.bits() as i32 != 0 {
                Colour::DEFAULT
            } else {
                mapped
            }
        };
        Self {
            fg: resolve(cell.fg),
            bg: resolve(cell.bg),
            us: resolve(cell.us),
            attr: cell.attr,
        }
    }

    fn write_css(self, css: &mut String) {
        css.push_str("color:");
        write_colour(
            css,
            if self.attr.contains(A::REVERSE) {
                self.bg
            } else {
                self.fg
            },
            if self.attr.contains(A::REVERSE) {
                "var(--tv-bg,var(--bg))"
            } else {
                "var(--tv-fg,var(--fg))"
            },
        );
        css.push_str("!important;background:");
        write_colour(
            css,
            if self.attr.contains(A::REVERSE) {
                self.fg
            } else {
                self.bg
            },
            if self.attr.contains(A::REVERSE) {
                "var(--tv-fg,var(--fg))"
            } else {
                "transparent"
            },
        );
        write!(
            css,
            "!important;font-weight:{};font-style:{};",
            if self.attr.contains(A::BRIGHT) {
                "bold"
            } else {
                "normal"
            },
            if self.attr.contains(A::ITALICS) {
                "italic"
            } else {
                "normal"
            }
        )
        .unwrap();
        if self
            .attr
            .intersects(A::ALL_UNDERSCORE | A::STRIKETHROUGH | A::OVERLINE)
        {
            css.push_str("text-decoration-line:");
            if self.attr.intersects(A::ALL_UNDERSCORE) {
                css.push_str(" underline");
            }
            if self.attr.contains(A::STRIKETHROUGH) {
                css.push_str(" line-through");
            }
            if self.attr.contains(A::OVERLINE) {
                css.push_str(" overline");
            }
            css.push_str(";text-decoration-color:");
            write_colour(css, self.us, "currentColor");
            css.push(';');
            if self.attr.contains(A::UNDERSCORE_2) {
                css.push_str("text-decoration-style:double;");
            }
        }
        if self.attr.contains(A::DIM) {
            css.push_str("opacity:0.5;");
        }
        if self.attr.contains(A::HIDDEN) {
            css.push_str("visibility:hidden;");
        }
    }
}

fn write_colour(css: &mut String, colour: Colour, default: &str) {
    let raw = colour.raw();
    let index = if (0..8).contains(&raw) {
        Some(raw)
    } else if (90..=97).contains(&raw) {
        Some(raw - 90 + 8)
    } else if raw & ColourFlags::_256.bits() as i32 != 0 && raw & 255 < 16 {
        Some(raw & 255)
    } else {
        None
    };
    if let Some(index) = index {
        let rgb = rmux_emu::colour::indexed_to_rgb(Colour(index));
        write!(css, "var(--ansi{index},#{:06x})", rgb.raw() & 0xffffff).unwrap();
    } else if let Some(rgb) = colour.force_rgb() {
        write!(css, "#{:06x}", rgb.raw() & 0xffffff).unwrap();
    } else {
        css.push_str(default);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::Client,
        ids::ArenaId,
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
                    .is_some_and(|text| text.starts_with(prefix))
            })
            .unwrap_or_else(|| panic!("no segment {prefix} in {bar}"))
    }

    #[test]
    fn bar_keeps_status_sides_and_window_drop_priorities() {
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
        server.winlinks.get_mut(links[1]).unwrap().flags = crate::model::WinlinkFlags::BELL;
        crate::ui::status::status_update_cache(&mut server, s);
        let mut c = Client::new(None, (0, 0));
        c.session = Some(s);
        c.flags.insert(ClientFlags::ATTACHED);
        c.tty_sx = 80;
        c.tty_sy = 24;
        let id = server.clients.insert(c).unwrap();

        status_redraw(&mut server, id);
        let bar = build(&mut server, id, None, false).unwrap().node;
        assert_eq!(bar["id"], BAR_ID);
        assert_eq!(bar["c"].as_array().unwrap().len(), 1);

        let session = seg(&bar, "[work]");
        assert!(session["p"].get("side").is_none());
        let current = seg(&bar, "0:edit");
        let bell = seg(&bar, "1:logs");
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
        assert!(build(&mut server, id, None, false).is_none());
    }

    #[test]
    fn styled_segments_preserve_wide_text_spaces_and_inline_color_boundaries() {
        let server = Server::new();
        let client = Client::new(None, (0, 0));
        let mut grid = Grid::new(8, 1, 0);
        let mut cell = rmux_emu::cell::DEFAULT_CELL;
        cell.fg = Colour::rgb(0x12, 0x34, 0x56);
        cell.bg = Colour::rgb(0x65, 0x43, 0x21);
        cell.data = rmux_util::utf8::Utf8Data::set(b' ');
        grid.view_set_cell(0, 0, &cell);
        cell.data.data[..3].copy_from_slice("界".as_bytes());
        cell.data.size = 3;
        cell.data.width = 2;
        grid.view_set_cell(1, 0, &cell);
        grid.view_set_padding(2, 0, cell.bg);
        cell.data = rmux_util::utf8::Utf8Data::set(b'x');
        cell.fg = Colour::DEFAULT;
        grid.view_set_cell(3, 0, &cell);
        cell.data = rmux_util::utf8::Utf8Data::set(b' ');
        grid.view_set_cell(4, 0, &cell);
        let range = StyleRange {
            range_type: StyleRangeType::Left,
            argument: 0,
            string: [0; 16],
            start: 0,
            end: 5,
        };
        let line = Line {
            server: &server,
            session: SessionId::from_parts(0, 1),
            client: &client,
            grid: &grid,
            y: 0,
        };
        let mut segs = Vec::new();
        let mut css = String::new();
        line.push(&mut segs, 0, 5, Some(&range), Some(&mut css));
        let runs = segs[0]["c"].as_array().unwrap();
        assert_eq!(
            runs.iter()
                .map(|run| run["p"]["text"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [" 界", "x "]
        );
        assert!(css.contains("color:#123456!important;background:#654321!important"));
        assert!(
            css.contains("color:var(--tv-fg,var(--fg))!important;background:#654321!important")
        );
    }
}
