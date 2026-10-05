// Ported from tmux window-client.c @ 8f25579c
/*
 * Copyright (c) 2017 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

use crate::client::{ClientFlags, ResolvedMouseEvent, lifecycle, registry};
use crate::cmd::arguments::Args;
use crate::format::draw::draw as format_draw;
use crate::format::sort::{self, SortCriteria, SortOrder};
use crate::format::{self, FormatContext, FormatFlags, FormatTree};
use crate::ids::{ClientId, ModeId, PaneId};
use crate::model::pane::{PaneMode, PaneModeDriver, pane_reset_mode};
use crate::model::{ModelError, PaneFlags};
use crate::modes::tree::{
    ModeAction, ModeTreeCallbacks, ModeTreeData, ModeTreeMenuItem, ModeTreeTag, TreeModeState,
    run_command,
};
use crate::server::Server;
use crate::ui::status::{status_at_line, status_line_size};
use crate::ui::styles::{create_defaults, style_apply};
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::screen::write::ScreenWriteCtx;
use rmux_emu::screen::{BoxLines, Screen, ScreenMode, ScreenResetPolicy};
use rmux_tty::term::TtyTermFlags;
use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, SpecialKey};

pub const NAME: &[u8] = b"client-mode";
pub const DEFAULT_COMMAND: &[u8] = b"detach-client -t '%%'";
pub const DEFAULT_FORMAT: &[u8] =
    b"#[fg=themelightgrey]#{t/p:client_activity}: session #[default]#{session_name}";
pub const DEFAULT_KEY_FORMAT: &[u8] = b"#{?#{e|<:#{line},10},\
#{line}\
,#{e|<:#{line},36},\
M-#{a:#{e|+:97,#{e|-:#{line},10}}}\
}";

macro_rules! feature {
    ($f:literal) => {
        concat!(
            "#{?#{I/f:",
            $f,
            "},#[fg=themegreen],#[fg=themelightgrey]}#{p/15:#{l:",
            $f,
            "}}#[default]"
        )
    };
}

/// `window_client_info_lines`.
pub const INFO_LINES: &[&str] = &[
    concat!(
        "#[fg=themelightgrey]Client Name   ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{client_name} #[fg=themelightgrey]",
        "#[fg=themelightgrey](PID #{client_pid})#[default]"
    ),
    concat!(
        "#[fg=themelightgrey]Session       ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{session_name}"
    ),
    concat!(
        "#[fg=themelightgrey]Attach Time   ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{t:client_created} ",
        "#[fg=themelightgrey](#{t/r:client_created})#[default]"
    ),
    concat!(
        "#[fg=themelightgrey]Activity Time ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{t:client_activity} ",
        "#[fg=themelightgrey](#{t/r:client_activity})#[default]"
    ),
    concat!(
        "#[fg=themelightgrey]Terminal Type ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{?client_termtype,#{client_termtype},Unknown}"
    ),
    concat!(
        "#[fg=themelightgrey]TERM          ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{client_termname}"
    ),
    concat!(
        "#[fg=themelightgrey]Size          ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{client_width}x#{client_height} ",
        "#[fg=themelightgrey](cell #{client_cell_width}x",
        "#{client_cell_height})#[default]"
    ),
    concat!(
        "#[fg=themelightgrey]Bytes Written ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{client_written} ",
        "#[fg=themelightgrey](#{client_discarded} discarded)#[default]"
    ),
    concat!(
        "#[fg=themelightgrey]Features      ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        feature!("256"),
        " ",
        feature!("RGB"),
        " ",
        feature!("bpaste"),
        " ",
        feature!("ccolour")
    ),
    concat!(
        "              ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        feature!("clipboard"),
        " ",
        feature!("cstyle"),
        " ",
        feature!("extkeys"),
        " ",
        feature!("focus")
    ),
    concat!(
        "              ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        feature!("hyperlinks"),
        " ",
        feature!("ignorefkeys"),
        " ",
        feature!("margins"),
        " ",
        feature!("mouse")
    ),
    concat!(
        "              ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        feature!("osc7"),
        " ",
        feature!("overline"),
        " ",
        feature!("progressbar"),
        " ",
        feature!("rectfill")
    ),
    concat!(
        "              ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        feature!("sixel"),
        " ",
        feature!("strikethrough"),
        " ",
        feature!("sync"),
        " ",
        feature!("title")
    ),
    concat!(
        "              ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        feature!("usstyle"),
        " ",
        feature!("utf8")
    ),
    "#[#{E:tree-mode-border-style},acs]qqqqqqqqqqqqqqn#{R:q,#{window_width}}#[default]",
    concat!(
        "#[fg=themelightgrey]prefix        ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{prefix}"
    ),
    concat!(
        "#[fg=themelightgrey]mouse         ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{?mouse,#{?#{I/c:kmous},,#[fg=themered]}on,#[fg=themelightgrey]off} ",
        "#{?#{I/c:kmous},,#[align=right]unavailable: [kmous] missing}"
    ),
    concat!(
        "#[fg=themelightgrey]set-clipboard ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{?#{!=:#{set-clipboard},off},#{?#{I/c:Ms},,",
        "#[fg=themered]}#{set-clipboard},#[fg=themelightgrey]off} ",
        "#{?#{I/c:Ms},,#[align=right]unavailable: [Ms] ",
        "#{?clipboard_invalid,invalid,missing}}"
    ),
    concat!(
        "#[fg=themelightgrey]get-clipboard ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{?#{!=:#{get-clipboard},off},#{?#{I/c:Ms},,",
        "#[fg=themered]}#{get-clipboard},#[fg=themelightgrey]off} ",
        "#{?#{I/c:Ms},,#[align=right]unavailable: [Ms] ",
        "#{?clipboard_invalid,invalid,missing}}"
    ),
    concat!(
        "#[fg=themelightgrey]focus-events  ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{?focus-events,#{?#{I/f:focus},,#[fg=themered]}on,#[fg=themelightgrey]off} ",
        "#{?#{I/f:focus},,#[align=right]unavailable: [Enfcs] or [Dcfcs] missing}"
    ),
    concat!(
        "#[fg=themelightgrey]extended-keys ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{?#{!=:#{extended-keys},off},#{?#{I/f:extkeys},,",
        "#[fg=themered]}#{extended-keys},#[fg=themelightgrey]off} ",
        "#{?#{I/f:extkeys},,#[align=right]unavailable: [Eneks] or [Dseks] missing}"
    ),
    concat!(
        "#[fg=themelightgrey]set-titles    ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{?set-titles,on,#[fg=themelightgrey]off}"
    ),
    concat!(
        "#[fg=themelightgrey]escape-time   ",
        "#[#{E:tree-mode-border-style},acs]x#[default] ",
        "#{escape-time} ms"
    ),
];

const NONE: u64 = SpecialKey::NONE;
pub const MENU_ITEMS: &[ModeTreeMenuItem] = &[
    ModeTreeMenuItem {
        name: "Detach",
        key: b'd' as u64,
    },
    ModeTreeMenuItem {
        name: "Detach Tagged",
        key: b'D' as u64,
    },
    ModeTreeMenuItem {
        name: "",
        key: NONE,
    },
    ModeTreeMenuItem {
        name: "Tag",
        key: b't' as u64,
    },
    ModeTreeMenuItem {
        name: "Tag All",
        key: 0o24,
    },
    ModeTreeMenuItem {
        name: "Tag None",
        key: b'T' as u64,
    },
    ModeTreeMenuItem {
        name: "",
        key: NONE,
    },
    ModeTreeMenuItem {
        name: "Cancel",
        key: b'q' as u64,
    },
];

const ORDER_SEQ: &[SortOrder] = &[
    SortOrder::Name,
    SortOrder::Size,
    SortOrder::Creation,
    SortOrder::Activity,
];

pub const HELP_LINES: &[&str] = &[
    "#[fg=themelightgrey]          i #[#{E:tree-mode-border-style},acs]x#[default] Toggle info view",
    "#[fg=themelightgrey]      Enter #[#{E:tree-mode-border-style},acs]x#[default] Choose selected %1",
    "#[fg=themelightgrey]          d #[#{E:tree-mode-border-style},acs]x#[default] Detach selected %1",
    "#[fg=themelightgrey]          D #[#{E:tree-mode-border-style},acs]x#[default] Detach tagged %1s",
    "#[fg=themelightgrey]          x #[#{E:tree-mode-border-style},acs]x#[default] Detach selected %1",
    "#[fg=themelightgrey]          X #[#{E:tree-mode-border-style},acs]x#[default] Detach tagged %1s",
    "#[fg=themelightgrey]          z #[#{E:tree-mode-border-style},acs]x#[default] Suspend selected %1",
    "#[fg=themelightgrey]          Z #[#{E:tree-mode-border-style},acs]x#[default] Suspend tagged %1s",
    "#[fg=themelightgrey]          f #[#{E:tree-mode-border-style},acs]x#[default] Enter a filter",
];

pub struct ClientItem {
    pub client: ClientId,
    pub ttyname: ByteString,
}

/// What `draw` needs from the whole server, computed in `prepare_draw`.
#[derive(Default)]
struct DrawCache {
    info: Vec<ByteString>,
    border_gc: GridCell,
}

pub struct ClientBackend {
    pub wp: PaneId,
    pub format: Vec<u8>,
    pub key_format: Vec<u8>,
    pub command: Vec<u8>,
    pub hide_preview_this_pane: bool,
    pub preview_is_info: bool,
    pub items: Vec<ClientItem>,
    cache: DrawCache,
}

pub type ClientState = TreeModeState<ClientBackend>;

pub struct ClientMode {
    args: Args,
}

impl ClientMode {
    pub fn new(args: Args) -> Self {
        Self { args }
    }
}

impl ClientBackend {
    /// `window_client_free_item` for every item: drop the client leases.
    fn clear_items(&mut self, s: &mut Server) {
        for item in self.items.drain(..) {
            let _ = s.clients.release(item.client);
        }
    }
}

fn client_window(s: &Server, c: ClientId) -> Option<crate::ids::WindowId> {
    let session = s.sessions.get(s.clients.get(c)?.session?)?;
    s.winlinks.get(session.current?).map(|wl| wl.window)
}

impl ModeTreeCallbacks for ClientBackend {
    type Item = ClientItem;

    fn build(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        sort: &SortCriteria,
        _tag: &mut ModeTreeTag,
        filter: Option<&[u8]>,
    ) {
        self.clear_items(s);
        let mut clients: Vec<ClientId> = Vec::new();
        sort::get_clients(&*s, sort, &mut clients);
        for c in clients {
            let Some(client) = s.clients.get(c) else {
                continue;
            };
            if client.session.is_none() || client.flags.intersects(ClientFlags::UNATTACHEDFLAGS) {
                continue;
            }
            let ttyname = client.ttyname.clone().unwrap_or_default().into();
            if s.clients.retain(c).is_err() {
                continue;
            }
            self.items.push(ClientItem { client: c, ttyname });
        }
        for i in 0..self.items.len() {
            let c = self.items[i].client;
            let context = FormatContext {
                evaluated_client: Some(c),
                ..FormatContext::default()
            };
            if let Some(filter) = filter {
                let cp = format::single(s, None, context, filter);
                if !format::true_value(Some(&cp)) {
                    continue;
                }
            }
            let text = format::single(s, None, context, &self.format);
            let name = s
                .clients
                .get(c)
                .map(|c| c.name_bytes().to_vec())
                .unwrap_or_default();
            tree.add(
                None,
                Some(i as u32),
                ModeTreeTag::Client(c),
                &name,
                Some(&text),
                -1,
            );
        }
    }

    fn prepare_draw(&mut self, s: &mut Server, item: Option<u32>, _sx: u32, _sy: u32) {
        self.cache.info.clear();
        self.cache.border_gc = DEFAULT_CELL;
        let Some(c) = item
            .and_then(|n| self.items.get(n as usize))
            .map(|i| i.client)
        else {
            return;
        };
        let Some(w) = client_window(s, c) else {
            return;
        };
        let Some(wo) = s.windows.get(w).map(|w| w.options) else {
            return;
        };
        style_apply(
            s,
            &mut self.cache.border_gc,
            wo,
            b"tree-mode-border-style",
            None,
        );
        if !self.preview_is_info {
            return;
        }
        let invalid = s
            .clients
            .get(c)
            .and_then(|c| c.tty.as_ref())
            .is_some_and(|tty| tty.term().flags().contains(TtyTermFlags::INVALIDMS));
        let mut ft = create_defaults(s, None, Some(c), None, None, None);
        ft.add(
            b"clipboard_invalid",
            ByteString::from(if invalid { "1" } else { "0" }),
        );
        for line in INFO_LINES {
            let expanded = ft.expand(s, line.as_bytes());
            self.cache.info.push(expanded);
        }
        ft.release(s);
    }

    fn draw(
        &self,
        s: &Server,
        item: Option<&ClientItem>,
        ctx: &mut ScreenWriteCtx<'_>,
        sx: u32,
        sy: u32,
    ) {
        let Some(item) = item else {
            return;
        };
        let c = item.client;
        let Some(client) = s.clients.get(c) else {
            return;
        };
        let Some(session) = client.session else {
            return;
        };
        if client.flags.intersects(ClientFlags::UNATTACHEDFLAGS) {
            return;
        }
        let cx = ctx.screen.cx;
        let cy = ctx.screen.cy;
        let gc = self.cache.border_gc;
        if self.preview_is_info {
            ctx.cursormove(cx as i32, cy as i32, false);
            let mut i = 0;
            for line in &self.cache.info {
                if i == sy {
                    break;
                }
                ctx.cursormove(cx as i32, (cy + i) as i32, false);
                format_draw(ctx, &DEFAULT_CELL, sx, line, None, false);
                i += 1;
            }
            if sx > 14 && i < sy {
                ctx.cursormove((cx + 14) as i32, (cy + i) as i32, false);
                ctx.vline(sy - i, false, false, Some(&gc));
            }
            return;
        }
        let Some(w) = s
            .sessions
            .get(session)
            .and_then(|s| s.current)
            .and_then(|wl| s.winlinks.get(wl))
            .map(|wl| wl.window)
        else {
            return;
        };
        let Some(window) = s.windows.get(w) else {
            return;
        };
        let mut wp = window.active;
        if self.hide_preview_this_pane && wp == Some(self.wp) {
            wp = window.last.first().copied();
        }
        let mut lines = status_line_size(s, c);
        if lines >= sy {
            lines = 0;
        }
        let at = if status_at_line(s, c) == 0 { lines } else { 0 };

        ctx.cursormove(cx as i32, (cy + at) as i32, false);
        if let Some(pane) = wp.and_then(|wp| s.panes.get(wp)) {
            ctx.preview(&pane.base, sx, sy.saturating_sub(2 + lines));
        }
        if at != 0 {
            ctx.cursormove(cx as i32, (cy + 2) as i32, false);
        } else {
            ctx.cursormove(cx as i32, (cy + sy - 1 - lines) as i32, false);
        }
        ctx.hline(sx, false, false, BoxLines::Default, Some(&gc));
        if at != 0 {
            ctx.cursormove(cx as i32, cy as i32, false);
        } else {
            ctx.cursormove(cx as i32, (cy + sy - lines) as i32, false);
        }
        ctx.fast_copy(&client.status.screen, 0, 0, sx, lines);
    }

    fn has_draw(&self) -> bool {
        true
    }

    fn search(
        &self,
        _s: &Server,
        _item: Option<&ClientItem>,
        _needle: &[u8],
        _icase: bool,
    ) -> Option<bool> {
        None
    }

    fn menu(&mut self, mode: ModeId, client: Option<ClientId>, key: KeyCode) -> ModeAction {
        Box::new(move |server: &mut Server| {
            let first = server
                .panes
                .get(mode.owner)
                .and_then(|p| p.modes.first())
                .map(|m| m.id);
            if first == Some(mode) {
                key_impl(server, mode, client, key, None);
            }
        })
    }

    fn height(&self, _screen_sy: u32) -> Option<u32> {
        None
    }

    fn key(&mut self, s: &mut Server, item: Option<u32>, line: u32) -> Option<KeyCode> {
        let item = self.items.get(item? as usize)?;
        let mut ft = FormatTree::create(None, None, 0, FormatFlags::NONE, s);
        ft.defaults(
            s,
            FormatContext {
                evaluated_client: Some(item.client),
                ..FormatContext::default()
            },
        );
        ft.add(b"line", ByteString::from(line.to_string()));
        let expanded = ft.expand(s, &self.key_format);
        ft.release(s);
        Some(rmux_tty::key_string::parse_key_name(&expanded))
    }

    fn swap(&self, _cur: &ClientItem, _other: &ClientItem, _sort: &SortCriteria) -> ModeAction {
        Box::new(|_| {})
    }

    fn sort(&self, sort: &mut SortCriteria) -> bool {
        sort.order_seq = Some(ORDER_SEQ);
        if sort.order == SortOrder::End {
            sort.order = ORDER_SEQ[0];
        }
        true
    }

    fn help(&self) -> Option<(u32, &'static str, &'static [&'static str])> {
        Some((0, "client", HELP_LINES))
    }

    fn items(&self) -> &[ClientItem] {
        &self.items
    }
}

fn mode_mut(server: &mut Server, id: ModeId) -> Option<&mut PaneMode> {
    server
        .panes
        .get_mut(id.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == id)
}

fn take_state(server: &mut Server, id: ModeId) -> Option<Box<ClientState>> {
    let mode = mode_mut(server, id)?;
    let data = mode.data.take()?;
    match data.downcast::<ClientState>() {
        Ok(state) => Some(state),
        Err(other) => {
            mode.data = Some(other);
            None
        }
    }
}

fn free_state(server: &mut Server, mut state: Box<ClientState>) {
    state.backend.clear_items(server);
    state.free(server);
}

fn with_state(
    server: &mut Server,
    id: ModeId,
    f: impl FnOnce(&mut Server, &mut ClientState, &mut Screen),
) {
    let Some(mut state) = take_state(server, id) else {
        return;
    };
    let Some(mut screen) = mode_mut(server, id).and_then(|m| m.screen.take()) else {
        match mode_mut(server, id) {
            Some(mode) => mode.data = Some(state),
            None => free_state(server, state),
        }
        return;
    };
    f(server, &mut state, &mut screen);
    match mode_mut(server, id) {
        Some(mode) => {
            mode.screen = Some(screen);
            mode.data = Some(state);
        }
        None => {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
            free_state(server, state);
        }
    }
}

fn redraw(server: &mut Server, state: &mut ClientState, screen: &mut Screen) {
    state.draw(server, screen);
    if let Some(p) = server.panes.get_mut(state.backend.wp) {
        p.flags.insert(PaneFlags::REDRAW);
    }
}

/// `window_client_do_detach`.
fn do_detach(server: &mut Server, state: &mut ClientState, item: u32, key: u64) {
    if state.tree.get_current().and_then(|i| i.item) == Some(item) {
        state.tree.down(false);
    }
    let Some(c) = state.backend.items.get(item as usize).map(|i| i.client) else {
        return;
    };
    match key {
        k if k == u64::from(b'd') || k == u64::from(b'D') => lifecycle::detach(server, c, false),
        k if k == u64::from(b'x') || k == u64::from(b'X') => lifecycle::detach(server, c, true),
        k if k == u64::from(b'z') || k == u64::from(b'Z') => lifecycle::suspend(server, c),
        _ => {}
    }
}

fn set_view_name(state: &mut ClientState) {
    if state.backend.preview_is_info {
        state.tree.view_name(b"info");
    } else {
        state.tree.view_name(b"preview");
    }
}

/// `window_client_key`.
fn key_impl(
    server: &mut Server,
    id: ModeId,
    c: Option<ClientId>,
    key: KeyCode,
    m: Option<&ResolvedMouseEvent>,
) {
    let wp = id.owner;
    let mut finished = false;
    with_state(server, id, |server, state, screen| {
        let result = state.key(server, id, c, key, m, screen);
        finished = result.finished;
        let key = result.key.0;
        let current = state.tree.get_current().and_then(|i| i.item);
        match key {
            k if k == u64::from(b'd') || k == u64::from(b'x') || k == u64::from(b'z') => {
                if let Some(item) = current {
                    do_detach(server, state, item, k);
                }
                state.build(server);
            }
            k if k == u64::from(b'D') || k == u64::from(b'X') || k == u64::from(b'Z') => {
                for item in state.tree.tagged_items(false) {
                    do_detach(server, state, item, k);
                }
                state.build(server);
            }
            k if k == u64::from(b'i') => {
                state.backend.preview_is_info = !state.backend.preview_is_info;
                set_view_name(state);
                state.build(server);
            }
            k if k == u64::from(b'\r') => {
                if let Some(item) = current.and_then(|n| state.backend.items.get(n as usize)) {
                    let ttyname = item.ttyname.clone();
                    run_command(server, c, None, &state.backend.command, &ttyname);
                }
                finished = true;
            }
            _ => {}
        }
        if !finished && registry::how_many(server) != 0 {
            redraw(server, state, screen);
        }
    });
    if finished || registry::how_many(server) == 0 {
        let _ = pane_reset_mode(server, wp);
    }
}

impl PaneModeDriver for ClientMode {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
        let wp = id.owner;
        let (sx, sy) = {
            let p = server.panes.get(wp)?;
            (p.base.grid.sx(), p.base.grid.sy())
        };
        let args = &self.args;
        let backend = ClientBackend {
            wp,
            hide_preview_this_pane: args.has(b'h') != 0,
            preview_is_info: args.has(b'i') != 0,
            format: args.get(b'F').unwrap_or(DEFAULT_FORMAT).to_vec(),
            key_format: args.get(b'K').unwrap_or(DEFAULT_KEY_FORMAT).to_vec(),
            command: if args.count() == 0 {
                DEFAULT_COMMAND.to_vec()
            } else {
                args.string(0).unwrap_or(DEFAULT_COMMAND).to_vec()
            },
            items: Vec::new(),
            cache: DrawCache::default(),
        };
        let preview = ModeTreeData::preview_from_args(Some(args), backend.has_draw());
        let mut tree = ModeTreeData::start(sx, sy, preview);
        tree.sort.order = ORDER_SEQ[0];
        tree.menu_items = MENU_ITEMS;
        tree.filter = args.get(b'f').map(ByteString::from);
        tree.sort.reversed = args.has(b'r') != 0;
        if args.has(b'O') != 0 {
            tree.sort.order = crate::format::sort::order_from_string(args.get(b'O'));
        }
        let state = TreeModeState { tree, backend };
        let mut screen = Screen::new(
            sx,
            sy,
            0,
            ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .ok()?;
        screen.mode.remove(ScreenMode::CURSOR);
        let (mut state, mut screen) =
            super::tree::init_zoom(server, id, state, screen, Some(args))?;
        set_view_name(&mut state);
        state.build(server);
        state.draw(server, &mut screen);
        match mode_mut(server, id) {
            Some(mode) => mode.data = Some(Box::new(state)),
            None => {
                free_state(server, Box::new(state));
                let _ = screen.release(
                    &mut server.hyperlinks,
                    #[cfg(feature = "sixel")]
                    None,
                );
                return None;
            }
        }
        Some(screen)
    }

    fn free(&self, server: &mut Server, mut mode: PaneMode) {
        if let Some(state) = mode
            .data
            .take()
            .and_then(|d| d.downcast::<ClientState>().ok())
        {
            free_state(server, state);
        }
        if let Some(mut screen) = mode.screen.take() {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
        }
    }

    fn resize(&self, server: &mut Server, id: ModeId, sx: u32, sy: u32) {
        with_state(server, id, |server, state, screen| {
            state.resize(server, screen, sx, sy)
        });
    }

    fn update(&self, server: &mut Server, id: ModeId) {
        with_state(server, id, |server, state, screen| {
            state.build(server);
            redraw(server, state, screen);
        });
    }

    fn key(
        &self,
        server: &mut Server,
        id: ModeId,
        client: ClientId,
        key: KeyCode,
        mouse: Option<&ResolvedMouseEvent>,
    ) {
        key_impl(server, id, Some(client), key, mouse);
    }

    fn append_output(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _bytes: &[u8],
    ) -> Result<(), ModelError> {
        Err(ModelError::Message(b"client-mode has no output".to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::commands::break_pane::tests::{create_session, create_window, split};
    use rmux_emu::cell::GridAttributes;
    use rmux_emu::hyperlinks::HyperlinkRegistry;
    use rmux_util::utf8::Utf8Data;

    fn backend(wp: PaneId) -> ClientBackend {
        ClientBackend {
            wp,
            format: DEFAULT_FORMAT.to_vec(),
            key_format: DEFAULT_KEY_FORMAT.to_vec(),
            command: DEFAULT_COMMAND.to_vec(),
            hide_preview_this_pane: false,
            preview_is_info: false,
            items: Vec::new(),
            cache: DrawCache::default(),
        }
    }

    fn fill(screen: &mut Screen, byte: u8) {
        screen.mode.remove(rmux_emu::screen::ScreenMode::CURSOR);
        let mut gc = DEFAULT_CELL;
        gc.data = Utf8Data::set(byte);
        for y in 0..screen.grid.sy() {
            for x in 0..screen.grid.sx() {
                screen.grid.view_set_cell(x, y, &gc);
            }
        }
    }

    fn preview(
        server: &mut Server,
        backend: &mut ClientBackend,
        client: ClientId,
        sx: u32,
        sy: u32,
    ) -> (Screen, HyperlinkRegistry) {
        backend.prepare_draw(server, Some(0), sx, sy);
        let mut registry = HyperlinkRegistry::new();
        let mut screen = Screen::new(
            sx + 4,
            sy + 4,
            0,
            rmux_emu::screen::ScreenResetPolicy::default(),
            &mut registry,
        )
        .unwrap();
        let mut sink = rmux_emu::screen::write::ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            rmux_emu::screen::write::ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.cursormove(2, 2, false);
        backend.draw(
            server,
            Some(&ClientItem {
                client,
                ttyname: Vec::new().into(),
            }),
            &mut ctx,
            sx,
            sy,
        );
        ctx.finish();
        (screen, registry)
    }

    #[test]
    fn preview_copies_status_at_top_bottom_and_hides_mode_pane() {
        let mut server = Server::default();
        let session = create_session(&mut server, b"clients");
        let (window, _, wp) = create_window(&mut server, session, 0);
        let last = split(&mut server, window, wp);
        fill(&mut server.panes.get_mut(wp).unwrap().base, b'A');
        fill(&mut server.panes.get_mut(last).unwrap().base, b'B');
        server.windows.get_mut(window).unwrap().last = vec![last];
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = Some(session);
        client.status.screen.resize(
            20,
            2,
            false,
            #[cfg(feature = "sixel")]
            None,
        );
        fill(&mut client.status.screen, b'S');
        let client = server.clients.insert(client).unwrap();
        let mut backend = backend(wp);
        backend.hide_preview_this_pane = true;
        backend.items.push(ClientItem {
            client,
            ttyname: Vec::new().into(),
        });
        for top in [false, true] {
            let s = server.sessions.get_mut(session).unwrap();
            s.statuslines = 2;
            s.statusat = if top { 0 } else { 1 };
            let (mut screen, mut registry) = preview(&mut server, &mut backend, client, 20, 8);
            let status_y = if top { 2 } else { 8 };
            let border_y = if top { 4 } else { 7 };
            let pane_y = if top { 5 } else { 2 };
            assert_eq!(screen.grid.view_get_cell(2, status_y).data.data[0], b'S');
            assert_eq!(
                screen.grid.view_get_cell(2, status_y + 1).data.data[0],
                b'S'
            );
            assert_eq!(screen.grid.view_get_cell(2, pane_y).data.data[0], b'B');
            let border = screen.grid.view_get_cell(2, border_y);
            assert_eq!(border.data.data[0], b'q');
            assert!(border.attr.contains(GridAttributes::CHARSET));
            screen
                .release(
                    &mut registry,
                    #[cfg(feature = "sixel")]
                    None,
                )
                .unwrap();
        }
    }

    #[test]
    fn info_view_draws_client_data_and_continues_column_fourteen_rule() {
        let mut server = Server::default();
        let session = create_session(&mut server, b"clients");
        let (_, _, wp) = create_window(&mut server, session, 0);
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = Some(session);
        client.name = Some(b"attached-test".to_vec());
        client.term_name = Some(b"screen".to_vec());
        let client = server.clients.insert(client).unwrap();
        let mut backend = backend(wp);
        backend.preview_is_info = true;
        backend.items.push(ClientItem {
            client,
            ttyname: Vec::new().into(),
        });
        let (mut screen, mut registry) = preview(&mut server, &mut backend, client, 80, 26);
        let first: Vec<_> = (2..82)
            .map(|x| screen.grid.view_get_cell(x, 2).data.data[0])
            .collect();
        assert!(
            first
                .windows(b"attached-test".len())
                .any(|w| w == b"attached-test")
        );
        let term: Vec<_> = (2..82)
            .map(|x| screen.grid.view_get_cell(x, 7).data.data[0])
            .collect();
        assert!(term.windows(6).any(|w| w == b"screen"));
        for y in 25..28 {
            let rule = screen.grid.view_get_cell(16, y);
            assert_eq!(rule.data.data[0], b'x');
            assert!(rule.attr.contains(GridAttributes::CHARSET));
        }
        screen
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
    }

    #[test]
    fn info_lines_match_c_table() {
        assert_eq!(INFO_LINES.len(), 23);
        assert!(INFO_LINES[0].starts_with("#[fg=themelightgrey]Client Name   #[#{E:tree-mode-border-style},acs]x#[default] #{client_name}"));
        assert_eq!(
            INFO_LINES[8],
            "#[fg=themelightgrey]Features      #[#{E:tree-mode-border-style},acs]x#[default] \
#{?#{I/f:256},#[fg=themegreen],#[fg=themelightgrey]}#{p/15:#{l:256}}#[default] \
#{?#{I/f:RGB},#[fg=themegreen],#[fg=themelightgrey]}#{p/15:#{l:RGB}}#[default] \
#{?#{I/f:bpaste},#[fg=themegreen],#[fg=themelightgrey]}#{p/15:#{l:bpaste}}#[default] \
#{?#{I/f:ccolour},#[fg=themegreen],#[fg=themelightgrey]}#{p/15:#{l:ccolour}}#[default]"
        );
        assert_eq!(
            INFO_LINES[14],
            "#[#{E:tree-mode-border-style},acs]qqqqqqqqqqqqqqn#{R:q,#{window_width}}#[default]"
        );
        assert_eq!(
            INFO_LINES[22],
            "#[fg=themelightgrey]escape-time   #[#{E:tree-mode-border-style},acs]x#[default] #{escape-time} ms"
        );
    }

    #[test]
    fn help_and_menu_literals() {
        assert_eq!(HELP_LINES.len(), 9);
        assert_eq!(MENU_ITEMS.len(), 8);
        assert_eq!(MENU_ITEMS[4].key, 0o24);
    }
}
