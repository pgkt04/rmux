// rmux extension: a native client sees its status line as a TSP status strip
// in the projected surface's dock, since the projection covers the whole tty.
use super::wire::Hello;
use crate::{
    client::{Client, ClientFlags, KeyEvent, keys::handle_key},
    ids::{ClientId, SessionId},
    model::Server,
    ui::status::{
        STATUS_LINES_LIMIT, status_at_line, status_get_range, status_line_size,
        status_message_redraw, status_prompt_line_at, status_prompt_native, status_prompt_redraw,
        status_redraw_at,
    },
};
use rmux_emu::{
    cell::{GridAttributes as A, GridCell, GridCellFlags},
    colour::{ClientTheme, Colour, ColourFlags},
    grid::Grid,
    style::{StyleRange, StyleRangeType},
};
use rmux_util::key::{KeyCode, MouseEvent, SpecialKey};
use serde_json::{Map, Value, json};
use std::fmt::Write;

pub const BAR_ID: &str = "rmux:bar";
pub const BAR_SHEET: &str = "rmux-bar";
/// The `click` action of a segment drawn from a status range.
pub const BAR_ACTION: &str = "status";
/// The chat skins hold dock strips to the transcript measure; the bar spans
/// the pane like a status line.
pub const BAR_CSS: &str = "[data-id='rmux:bar']>.sf-status{max-width:none!important;width:100%!important;margin:0!important}";
/// The status line is drawn this wide for the bar, not at the tty width, so
/// the window list is whole (no `<`/`>` markers) and the host scrolls the
/// strip. 1024 cells hold some 60 windows of a dozen-cell name each.
const BAR_WIDTH: u32 = 1024;

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
            status_redraw_at(server, id, BAR_WIDTH);
        }
        build(server, id, prompt, styles)
    } else {
        None
    };
    let (node, css, segs) = bar.map_or((None, BAR_CSS.to_owned(), Vec::new()), |bar| {
        (Some(bar.node), bar.css, bar.segs)
    });
    let Some(c) = server.clients.get_mut(id) else {
        return false;
    };
    c.tsp.bar_segs = segs;
    c.tsp.projection.as_mut().is_some_and(|p| {
        p.bar_styles = styles;
        let changed = p.set_bar_css(css);
        p.set_bar(node) || changed || p.bar_pending()
    })
}

/// A click on the bar segment `seg`: with `mouse` on, the press and release
/// a tty sends on that range of the status line, so the status mouse
/// bindings run as they do there. A segment no longer drawn is ignored.
pub fn click(server: &mut Server, id: ClientId, seg: &str) {
    let Some(c) = server.clients.get(id) else {
        return;
    };
    let Some(&BarSeg { y, range, .. }) = c.tsp.bar_segs.iter().find(|s| s.id == seg) else {
        return;
    };
    let mouse = c
        .session
        .and_then(|s| server.sessions.get(s))
        .is_some_and(|s| server.options.get_number(s.options, b"mouse") != 0);
    // The status line is redrawn before the bar is, so the range is still
    // there unless the terminal clicked a bar since replaced.
    if !mouse || status_get_range(c, range.start, y) != Some(&range) {
        return;
    }
    let Ok(at) = u32::try_from(status_at_line(server, id)) else {
        return;
    };
    let (x, y) = (range.start, at + y);
    let press = MouseEvent {
        x,
        y,
        b: 0,
        lx: x,
        ly: y,
        lb: 3,
        sgr_type: b'M',
        sgr_b: 0,
    };
    let release = MouseEvent {
        b: 3,
        lb: 0,
        sgr_type: b'm',
        ..press
    };
    for m in [press, release] {
        handle_key(
            server,
            id,
            KeyEvent::with_mouse(KeyCode(SpecialKey::MOUSE), m),
        );
    }
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

/// A bar segment drawn from a status range: what a click on it resolves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarSeg {
    pub id: String,
    /// The status line the range is on.
    pub y: u32,
    pub range: StyleRange,
}

fn seg_id(y: u32, n: usize) -> String {
    format!("{BAR_ID}:{y}:{n}")
}

struct Bar {
    node: Value,
    css: String,
    segs: Vec<BarSeg>,
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
    let wide = &c.status.screen.grid;
    let active = &c.status.active().grid;
    let overlay = c.message.text.is_some() || c.prompt.is_some();
    let lines = (status_line_size(server, id).max(u32::from(overlay)) as usize)
        .min(STATUS_LINES_LIMIT)
        .min(active.sy() as usize);
    let overlay_line = status_prompt_line_at(server, id).min(lines.saturating_sub(1) as u32);
    let mut strips = Vec::new();
    let mut clicks = Vec::new();
    let mut css = BAR_CSS.to_owned();
    if styled {
        // Tern collapses white space by the segment's own `white-space`, not
        // the runs', so padding inside a range survives only from here.
        css.push_str("[data-id='rmux:bar'] .sf-seg{padding:0;gap:0;border-radius:0;color:inherit;white-space:pre}[data-id='rmux:bar'] .sf-seg-k{display:block;gap:0}");
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
        // An overlay copies the status line only at the tty width; the
        // lines it does not cover come from the wide status screen.
        let (grid, ranges) = if overlay && y as u32 == overlay_line {
            (active, &[][..])
        } else {
            (wide, &c.status.entries[y].ranges.0[..])
        };
        let (segs, line_clicks) = segments(
            server,
            session,
            c,
            grid,
            ranges,
            y as u32,
            styled.then_some(&mut css),
        );
        clicks.extend(line_clicks);
        if !segs.is_empty() {
            let strip_id = format!("{BAR_ID}:{y}");
            let base = if overlay && y as u32 == overlay_line {
                active.view_get_cell(0, y as u32)
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
                .write_css(&mut css, light_page(c));
                css.push_str("border:0!important}");
            }
            strips.push(json!({
                "id": strip_id, "k": "status",
                "p": {"transparent": transparent, "overflow": "scroll"},
                "c": segs,
            }));
        }
    }
    (!strips.is_empty()).then(|| Bar {
        node: json!({"id": BAR_ID, "k": "col", "c": strips}),
        css,
        segs: clicks,
    })
}

/// One segment per style range, plus any text drawn outside the ranges,
/// and the segments a click resolves to their range.
fn segments(
    server: &Server,
    session: SessionId,
    client: &Client,
    grid: &Grid,
    ranges: &[StyleRange],
    y: u32,
    mut css: Option<&mut String>,
) -> (Vec<Value>, Vec<BarSeg>) {
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
        page: light_page(client),
        mouse: server
            .sessions
            .get(session)
            .is_some_and(|s| server.options.get_number(s.options, b"mouse") != 0),
        // `status-justify right` (choice 2) is the only one that puts the
        // window list on the right; left, centre and absolute-centre start
        // it after status-left.
        list_right: server
            .sessions
            .get(session)
            .is_some_and(|s| server.options.get_number(s.options, b"status-justify") == 2),
    };
    let mut segs = Vec::new();
    let mut clicks = Vec::new();
    let mut x = 0;
    for (i, &range) in ranges.iter().enumerate() {
        if range.start < x {
            continue;
        }
        line.push(&mut segs, x, range.start, None, css.as_deref_mut());
        let mut end = range.end.min(width);
        // The status format draws `window-status-separator` after every
        // window but the last. The window's segment owns it, so the list
        // keeps its spacing and a dropped window takes its separator along.
        if range.range_type == StyleRangeType::Window
            && let Some(next) = ranges.get(i + 1)
            && next.range_type == StyleRangeType::Window
        {
            end = end.max(next.start);
        }
        if line.push(&mut segs, range.start, end, Some(range), css.as_deref_mut())
            && line.clickable(range)
        {
            clicks.push(BarSeg {
                id: seg_id(y, segs.len() - 1),
                y,
                range: *range,
            });
        }
        x = end;
    }
    line.push(&mut segs, x, width, None, css);
    (segs, clicks)
}

struct Line<'a> {
    server: &'a Server,
    session: SessionId,
    client: &'a Client,
    grid: &'a Grid,
    y: u32,
    page: Colour,
    /// The session's `mouse`: status ranges take clicks.
    mouse: bool,
    /// The window list is right-justified (`status-justify right`).
    list_right: bool,
}
impl Line<'_> {
    /// Whether a click on the range's segment reaches the status mouse
    /// bindings, as a tty click on it would.
    fn clickable(&self, range: &StyleRange) -> bool {
        self.mouse && range.range_type != StyleRangeType::None
    }

    /// True when a segment was pushed.
    fn push(
        &self,
        segs: &mut Vec<Value>,
        start: u32,
        end: u32,
        range: Option<&StyleRange>,
        css: Option<&mut String>,
    ) -> bool {
        if start >= end {
            return false;
        }
        let styled = css.is_some();
        let nonblank = |x| {
            let cell = self.grid.view_get_cell(x, self.y);
            !cell.flags.contains(GridCellFlags::PADDING) && cell.data.bytes() != b" "
        };
        let first = (start..end).find(|&x| nonblank(x));
        if first.is_none() && !(styled && range.is_some()) {
            return false;
        }
        let kind = range.map(|r| r.range_type);
        // Status-left and status-right name their side; the window, session
        // and pane lists follow `status-justify`, not where the tty happened
        // to draw them (a long list crosses the middle of the line); only
        // text outside every range goes by position.
        let right = match kind {
            Some(StyleRangeType::Left) => false,
            Some(StyleRangeType::Right) => true,
            Some(StyleRangeType::Window | StyleRangeType::Session | StyleRangeType::Pane) => {
                self.list_right
            }
            _ => start + end > self.grid.sx(),
        };
        let window = range
            .filter(|r| r.range_type == StyleRangeType::Window)
            .and_then(|r| self.window(r.argument));
        // A scrolling strip keeps the highest priority in view; a host that
        // cannot scroll drops the lowest first. As on the tty, status-left
        // and status-right keep their place and the window list gives way,
        // the current window last.
        let priority = match (kind, window) {
            (_, Some(true)) => 4,
            (Some(StyleRangeType::Left), _) => 3,
            (Some(StyleRangeType::Right), _) => 2,
            (_, Some(false)) => 1,
            _ => 0,
        };
        let mut p = Map::new();
        let id = seg_id(self.y, segs.len());
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
                style.write_css(css, self.page);
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
        if range.is_some_and(|r| self.clickable(r)) {
            p.insert("actions".into(), json!({"click": BAR_ACTION}));
        }
        segs.push(json!({
            "id": id,
            "k": "seg",
            "p": p,
            "c": children,
        }));
        true
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

    /// `page` is the light theme's background behind a default background.
    fn write_css(self, css: &mut String, page: Colour) {
        let reverse = self.attr.contains(A::REVERSE);
        let (text, back) = if reverse {
            (self.bg, self.fg)
        } else {
            (self.fg, self.bg)
        };
        css.push_str("color:");
        let text_default = if reverse {
            "var(--tv-bg,var(--bg))"
        } else {
            "var(--tv-fg,var(--fg))"
        };
        // A reversed default background shows the terminal's foreground,
        // which only Tern knows.
        let behind = if back.is_default() {
            (!reverse).then_some(page)
        } else {
            fixed_rgb(back)
        };
        let lit = fixed_rgb(text)
            .zip(behind)
            .map(|(fixed, behind)| (fixed, legible(fixed, behind)));
        match lit {
            Some((fixed, lit)) if lit != fixed => write!(
                css,
                "light-dark(#{:06x},#{:06x})",
                lit.raw() & 0xffffff,
                fixed.raw() & 0xffffff
            )
            .unwrap(),
            _ => write_colour(css, text, text_default),
        }
        css.push_str("!important;background:");
        write_colour(
            css,
            back,
            if reverse {
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

/// ANSI 0–15, which Tern's theme supplies.
fn ansi_index(colour: Colour) -> Option<i32> {
    let raw = colour.raw();
    if (0..8).contains(&raw) {
        Some(raw)
    } else if (90..=97).contains(&raw) {
        Some(raw - 90 + 8)
    } else if raw & ColourFlags::_256.bits() as i32 != 0 && raw & 255 < 16 {
        Some(raw & 255)
    } else {
        None
    }
}

/// The colour as RGB when the program fixed it, not Tern's theme.
fn fixed_rgb(colour: Colour) -> Option<Colour> {
    if ansi_index(colour).is_some() {
        None
    } else {
        colour.force_rgb()
    }
}

fn write_colour(css: &mut String, colour: Colour, default: &str) {
    if let Some(index) = ansi_index(colour) {
        let rgb = rmux_emu::colour::indexed_to_rgb(Colour(index));
        write!(css, "var(--ansi{index},#{:06x})", rgb.raw() & 0xffffff).unwrap();
    } else if let Some(rgb) = colour.force_rgb() {
        write!(css, "#{:06x}", rgb.raw() & 0xffffff).unwrap();
    } else {
        css.push_str(default);
    }
}

/// Tern relights text a program coloured too faintly for a light theme
/// (its default `contrast` setting) until it reaches WCAG AA.
const LEGIBLE: f64 = 4.5;

/// What a default background shows in a light theme: the terminal's
/// reported background while it is light, else white.
fn light_page(client: &Client) -> Colour {
    client
        .tty
        .as_ref()
        .map(|tty| Colour(tty.reported_colours().1))
        .filter(|bg| bg.theme() == ClientTheme::Light)
        .and_then(Colour::force_rgb)
        .unwrap_or(Colour::rgb(255, 255, 255))
}

/// RGB `fg` moved just far enough from RGB `bg` to reach [`LEGIBLE`], towards
/// whichever of black and white contrasts more with `bg`. Darkening scales
/// linear light, keeping the hue; lightening mixes in white.
fn legible(fg: Colour, bg: Colour) -> Colour {
    let linear = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let channels = |c: Colour| {
        let (r, g, b) = c.split_rgb();
        [linear(r), linear(g), linear(b)]
    };
    let luminance = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (text, back) = (channels(fg), luminance(channels(bg)));
    let y = luminance(text);
    if (y.max(back) + 0.05) / (y.min(back) + 0.05) >= LEGIBLE {
        return fg;
    }
    let encode = |c: f64| {
        255.0
            * if c <= 0.003_130_8 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }
    };
    // Either end passes, so the target is reachable; rounding away from the
    // background keeps it reached.
    let [r, g, b] = if (back + 0.05) / 0.05 >= 1.05 / (back + 0.05) {
        let k = ((back + 0.05) / LEGIBLE - 0.05) / y;
        text.map(|c| encode(c * k).floor() as u8)
    } else {
        let t = (LEGIBLE * (back + 0.05) - 0.05 - y) / (1.0 - y);
        text.map(|c| encode(c + (1.0 - c) * t).ceil().min(255.0) as u8)
    };
    Colour::rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::Client,
        ids::ArenaId,
        model::{session, spawn::SpawnFlags, window},
        ui::status::status_redraw,
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

    /// Session `work` showing window 0 `edit` (current) and window 1 `logs`
    /// (bell) to an attached 80x24 client.
    fn work_session() -> (Server, ClientId, SessionId) {
        let (mut server, id, s, links) = session_with(&[b"edit", b"logs"], 80);
        server.winlinks.get_mut(links[1]).unwrap().flags = crate::model::WinlinkFlags::BELL;
        crate::ui::status::status_update_cache(&mut server, s);
        status_redraw(&mut server, id);
        (server, id, s)
    }

    /// Session `work` with the windows `names` from index 0, the first
    /// current, attached to a client `sx` columns wide whose status line is
    /// drawn at its tty width.
    fn session_with(
        names: &[&[u8]],
        sx: u32,
    ) -> (Server, ClientId, SessionId, Vec<crate::ids::WinlinkId>) {
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
        for (index, name) in names.iter().enumerate() {
            let w = window::window_create(&mut server, sx, 23, 0, 0).unwrap();
            server.windows.get_mut(w).unwrap().name = name.to_vec();
            let pane =
                window::window_add_pane(&mut server, w, None, 0, SpawnFlags::default()).unwrap();
            server.windows.get_mut(w).unwrap().active = Some(pane);
            crate::layout::init(&mut server, w, pane);
            links.push(session::session_attach(&mut server, s, w, index as i32).unwrap());
        }
        session::session_set_current(&mut server, s, Some(links[0]));
        crate::ui::status::status_update_cache(&mut server, s);
        let mut c = Client::new(None, (0, 0));
        c.session = Some(s);
        c.flags.insert(ClientFlags::ATTACHED);
        c.tty_sx = sx;
        c.tty_sy = 24;
        let id = server.clients.insert(c).unwrap();
        status_redraw(&mut server, id);
        (server, id, s, links)
    }

    const EIGHT: [&[u8]; 8] = [
        b"edit", b"logs", b"build", b"tests", b"notes", b"shell", b"music", b"mail",
    ];
    /// A phone's width: status-left and status-right leave the window list
    /// some cells, too few for eight windows.
    const PHONE: u32 = 40;

    fn window_label(index: usize) -> String {
        format!("{index}:{}", String::from_utf8_lossy(EIGHT[index]))
    }

    fn status_text(server: &Server, id: ClientId) -> String {
        let grid = &server.clients.get(id).unwrap().status.screen.grid;
        String::from_utf8_lossy(&grid.view_string_cells(0, 0, grid.sx())).into_owned()
    }

    #[test]
    fn narrow_client_bar_holds_every_window_and_scrolls() {
        let (mut server, id, _, _) = session_with(&EIGHT, PHONE);
        // The tty's own status line keeps a few windows and a list marker.
        let tty = status_text(&server, id);
        assert!(
            tty.contains('>') && !tty.contains(&window_label(7)),
            "{tty}"
        );

        status_redraw_at(&mut server, id, BAR_WIDTH);
        let bar = build(&mut server, id, None, false).unwrap().node;
        assert_eq!(bar["c"][0]["p"]["overflow"], "scroll");
        for index in 0..EIGHT.len() {
            seg(&bar, &window_label(index));
        }
        assert!(
            segs(&bar)
                .iter()
                .all(|seg| !seg["p"]["text"].as_str().unwrap().contains(['<', '>'])),
            "{bar}"
        );

        // Off the native view, the cell redraw draws at the tty width again.
        status_redraw(&mut server, id);
        assert_eq!(status_text(&server, id), tty);
    }

    #[test]
    fn narrow_client_bar_click_selects_a_window_the_tty_could_not_draw() {
        let (mut server, id, s, links) = session_with(&EIGHT, PHONE);
        let tty = status_text(&server, id);
        let hidden = (0..EIGHT.len())
            .rev()
            .find(|&i| !tty.contains(&window_label(i)))
            .unwrap();
        // `MouseDown1Status { switch-client -t= }`, `=` the clicked window.
        crate::cmd::key_bindings::init(&mut server).unwrap();
        crate::cmd::queue::next(&mut server, None);
        let options = server.sessions.get(s).unwrap().options;
        server.options.set_number_value(options, b"mouse", 1);

        status_redraw_at(&mut server, id, BAR_WIDTH);
        let bar = build(&mut server, id, None, false).unwrap();
        let target = seg(&bar.node, &window_label(hidden))["id"]
            .as_str()
            .unwrap()
            .to_owned();
        server.clients.get_mut(id).unwrap().tsp.bar_segs = bar.segs;
        click(&mut server, id, &target);
        crate::cmd::queue::next(&mut server, Some(id));
        assert_eq!(server.sessions.get(s).unwrap().current, Some(links[hidden]));
    }

    #[test]
    fn bar_keeps_status_sides_and_window_drop_priorities() {
        let (mut server, id, s) = work_session();
        let bar = build(&mut server, id, None, false).unwrap().node;
        assert_eq!(bar["id"], BAR_ID);
        assert_eq!(bar["c"].as_array().unwrap().len(), 1);

        let session = seg(&bar, "[work]");
        assert!(session["p"].get("side").is_none());
        let current = seg(&bar, "0:edit");
        let bell = seg(&bar, "1:logs");
        // Narrow strips drop the other windows before the current one, and
        // before status-right (the clock), as the tty's window list does.
        assert!(current["p"]["priority"].as_u64() > bell["p"]["priority"].as_u64());
        let right = segs(&bar).last().unwrap();
        assert_eq!(right["p"]["side"], "right");
        assert!(right["p"]["priority"].as_u64() > bell["p"]["priority"].as_u64());
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
    fn styled_window_segments_own_the_separator_after_them() {
        let (mut server, id, _) = work_session();
        let bar = build(&mut server, id, None, true).unwrap().node;
        let texts = segs(&bar)
            .iter()
            .map(|seg| {
                seg["c"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|run| run["p"]["text"].as_str().unwrap())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        // The default separator is one space: it stays between the windows,
        // with the window before it rather than alone, and the last window
        // has none.
        let current = texts.iter().position(|t| t.starts_with("0:edit")).unwrap();
        assert_eq!(texts[current], "0:edit* ");
        assert_eq!(texts[current + 1], "1:logs!");
    }

    #[test]
    fn range_segments_take_clicks_only_with_mouse_on() {
        let (mut server, id, s) = work_session();
        // rmux defaults `mouse` on, as the pin's Makefile does.
        let options = server.sessions.get(s).unwrap().options;
        server.options.set_number_value(options, b"mouse", 0);
        let bar = build(&mut server, id, None, false).unwrap();
        assert!(
            segs(&bar.node)
                .iter()
                .all(|seg| seg["p"].get("actions").is_none())
        );
        assert!(bar.segs.is_empty());

        server.options.set_number_value(options, b"mouse", 1);
        let bar = build(&mut server, id, None, false).unwrap();
        for (prefix, kind, argument) in [
            ("[work]", StyleRangeType::Left, None),
            ("0:edit", StyleRangeType::Window, Some(0)),
            ("1:logs", StyleRangeType::Window, Some(1)),
        ] {
            let node = seg(&bar.node, prefix);
            assert_eq!(node["p"]["actions"], json!({"click": BAR_ACTION}));
            let click = bar
                .segs
                .iter()
                .find(|click| node["id"].as_str() == Some(click.id.as_str()))
                .unwrap_or_else(|| panic!("{prefix} not clickable"));
            assert_eq!((click.y, click.range.range_type), (0, kind));
            if let Some(argument) = argument {
                assert_eq!(click.range.argument, argument);
            }
        }
    }

    #[test]
    fn text_outside_ranges_takes_no_clicks() {
        let (mut server, id, s) = work_session();
        let options = server.sessions.get(s).unwrap().options;
        server.options.set_number_value(options, b"mouse", 1);
        let mut grid = Grid::new(4, 1, 0);
        let mut cell = rmux_emu::cell::DEFAULT_CELL;
        cell.data = rmux_util::utf8::Utf8Data::set(b'a');
        grid.view_set_cell(0, 0, &cell);
        cell.data = rmux_util::utf8::Utf8Data::set(b'b');
        grid.view_set_cell(2, 0, &cell);
        let range = StyleRange {
            range_type: StyleRangeType::Window,
            argument: 1,
            string: [0; 16],
            start: 0,
            end: 1,
        };
        let client = server.clients.get(id).unwrap();
        let (segs, clicks) = segments(&server, s, client, &grid, &[range], 0, None);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0]["p"]["actions"]["click"], BAR_ACTION);
        assert_eq!(segs[1]["p"]["text"], "b");
        assert!(segs[1]["p"].get("actions").is_none());
        assert_eq!(
            clicks,
            [BarSeg {
                id: segs[0]["id"].as_str().unwrap().to_owned(),
                y: 0,
                range,
            }]
        );
    }

    #[test]
    fn window_list_segments_take_the_status_justify_side() {
        let (mut server, id, s) = work_session();
        // A window drawn past the middle of the line still belongs to the
        // left-justified list, not the right group.
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = rmux_emu::cell::DEFAULT_CELL;
        cell.data = rmux_util::utf8::Utf8Data::set(b'w');
        for x in 6..9 {
            grid.view_set_cell(x, 0, &cell);
        }
        let range = StyleRange {
            range_type: StyleRangeType::Window,
            argument: 1,
            string: [0; 16],
            start: 6,
            end: 9,
        };
        let side = |server: &Server| {
            let client = server.clients.get(id).unwrap();
            let (segs, _) = segments(server, s, client, &grid, &[range], 0, None);
            segs[0]["p"].get("side").cloned()
        };
        assert_eq!(side(&server), None);

        let options = server.sessions.get(s).unwrap().options;
        server
            .options
            .set_number_value(options, b"status-justify", 2);
        assert_eq!(side(&server), Some(json!("right")));
    }

    #[test]
    fn styled_segments_preserve_wide_text_spaces_and_inline_color_boundaries() {
        let server = Server::new();
        let client = Client::new(None, (0, 0));
        let mut grid = Grid::new(8, 1, 0);
        let mut cell = rmux_emu::cell::DEFAULT_CELL;
        cell.fg = Colour::rgb(0xfe, 0xdc, 0xba);
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
            page: Colour::rgb(255, 255, 255),
            mouse: false,
            list_right: false,
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
        assert!(css.contains("color:#fedcba!important;background:#654321!important"));
        assert!(
            css.contains("color:var(--tv-fg,var(--fg))!important;background:#654321!important")
        );
    }

    fn ratio(a: Colour, b: Colour) -> f64 {
        let luminance = |c: Colour| {
            let (r, g, b) = c.split_rgb();
            let [r, g, b] = [r, g, b].map(|c| {
                let c = f64::from(c) / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn faint_fixed_colours_are_relit_just_to_aa_against_their_background() {
        for bg in [
            Colour::rgb(255, 255, 255),
            Colour::rgb(0xf8, 0xf8, 0xfa),
            Colour::rgb(0x8a, 0x8a, 0x8a),
            Colour::rgb(0x74, 0x74, 0x74),
            Colour::rgb(0x62, 0x62, 0x62),
            Colour::rgb(0, 0, 0),
        ] {
            for r in (0..=255).step_by(17) {
                for g in (0..=255).step_by(17) {
                    for b in (0..=255).step_by(17) {
                        let fg = Colour::rgb(r, g, b);
                        let lit = legible(fg, bg);
                        if ratio(fg, bg) >= LEGIBLE {
                            assert_eq!(lit, fg);
                            continue;
                        }
                        let reached = ratio(lit, bg);
                        assert!(
                            (LEGIBLE - 1e-9..LEGIBLE + 0.2).contains(&reached),
                            "{fg:?} on {bg:?} -> {lit:?}: {reached}"
                        );
                    }
                }
            }
        }
        // Darkening keeps the hue: colour50 stays aquamarine, not teal-blue.
        let (r, g, b) =
            legible(Colour::rgb(0x00, 0xff, 0xd7), Colour::rgb(255, 255, 255)).split_rgb();
        assert!(r == 0 && g > b, "{r} {g} {b}");
    }

    #[test]
    fn light_themes_get_relit_fixed_colours_and_dark_ones_the_original() {
        let client = Client::new(None, (0, 0));
        let css = |fg: Colour, bg: Colour, attr: A| {
            let mut css = String::new();
            CellStyle::new(
                &client,
                GridCell {
                    fg,
                    bg,
                    attr,
                    ..rmux_emu::cell::DEFAULT_CELL
                },
            )
            .write_css(&mut css, Colour::rgb(0xf8, 0xf8, 0xfa));
            css
        };
        let colour255 = Colour(ColourFlags::_256.bits() as i32 | 255);
        let faint = css(colour255, Colour::DEFAULT, A::default());
        let lit = legible(Colour::rgb(0xee, 0xee, 0xee), Colour::rgb(0xf8, 0xf8, 0xfa));
        assert!(
            faint.starts_with(&format!(
                "color:light-dark(#{:06x},#eeeeee)!important;",
                lit.raw() & 0xffffff
            )),
            "{faint}"
        );
        // Reversed, the text takes the background colour and sits on the
        // foreground colour.
        let reversed = css(
            Colour::rgb(0x62, 0x62, 0x62),
            Colour::rgb(0x12, 0x12, 0x12),
            A::REVERSE,
        );
        assert!(reversed.starts_with("color:light-dark(#"), "{reversed}");
        // Tern's theme supplies ANSI 0–15 and the default foreground.
        assert!(css(Colour(7), Colour::DEFAULT, A::default()).starts_with("color:var(--ansi7,"));
        assert!(
            css(Colour::DEFAULT, Colour::DEFAULT, A::default())
                .starts_with("color:var(--tv-fg,var(--fg))!important")
        );
        // Reversed over a default foreground, the text sits on the terminal's
        // foreground, which only Tern knows.
        assert!(css(Colour::DEFAULT, colour255, A::REVERSE).starts_with("color:#eeeeee!important"));
    }
}
