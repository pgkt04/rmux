// Ported from tmux window-tree.c @ 8f25579c
/*
 * Copyright (c) 2017 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
use super::tree::*;
use crate::format::{
    FormatContext,
    draw::{draw as format_draw, trim_left, width},
};
use crate::ui::{
    prompt::{PromptCreateData, PromptFlags, PromptType},
    styles::style_apply,
};
use crate::{
    cmd::{arguments::Args, find::CmdFindState},
    format::sort::SortCriteria,
    ids::{ClientId, ModeId, PaneId, SessionId, WinlinkId},
    model::{
        ModelError, Server,
        pane::{PaneMode, PaneModeDriver},
    },
};
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell};
use rmux_emu::screen::BoxLines;
use rmux_emu::screen::{Screen, ScreenResetPolicy, write::ScreenWriteCtx};
use rmux_util::key::SpecialKey;
use rmux_util::{bytes::ByteString, key::KeyCode};
use std::os::fd::AsFd;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowTreeType {
    #[default]
    None,
    Session,
    Window,
    Pane,
}
#[derive(Clone, Copy, Debug)]
pub struct WindowTreeItem {
    pub kind: WindowTreeType,
    pub session: SessionId,
    pub winlink: Option<WinlinkId>,
    pub pane: Option<PaneId>,
}
pub struct WindowTreeBackend {
    pub items: Vec<WindowTreeItem>,
    pub fs: CmdFindState,
    pub kind: WindowTreeType,
    pub template: ByteString,
    pub format: ByteString,
    pub key_format: ByteString,
    pub owner: PaneId,
    pub squash_groups: bool,
    pub hide_preview_this_pane: bool,
    pub preview_is_info: bool,
    pub prompt_flags: PromptFlags,
    pub offset: i32,
    cache: DrawCache,
}
#[derive(Default)]
struct DrawCache {
    strip: Option<PreviewStrip>,
    previews: Vec<PreviewItem>,
    info: Vec<Option<ByteString>>,
    border: GridCell,
}
struct PreviewItem {
    pane: PaneId,
    label: ByteString,
    border: GridCell,
    label_cell: GridCell,
}
impl WindowTreeItem {
    fn resolve(&self, s: &Server) -> Option<(WinlinkId, PaneId)> {
        let session = s.sessions.get(self.session)?;
        let wl = if self.kind == WindowTreeType::Session {
            session.current?
        } else {
            self.winlink?
        };
        let link = s.winlinks.get(wl)?;
        if link.session != self.session || session.windows.get(&link.index) != Some(&wl) {
            return None;
        }
        let window = s.windows.get(link.window)?;
        let pane = if self.kind == WindowTreeType::Pane {
            self.pane?
        } else {
            window.active?
        };
        if !window.panes.contains(&pane) || s.panes.get(pane).is_none() {
            return None;
        }
        Some((wl, pane))
    }
    fn context(&self) -> FormatContext {
        FormatContext {
            session: Some(self.session),
            winlink: self.winlink,
            pane: self.pane,
            ..Default::default()
        }
    }
    fn target(&self, s: &Server) -> Option<(ByteString, CmdFindState)> {
        let (wl, pane) = self.resolve(s)?;
        let mut target = b"=".to_vec();
        target.extend_from_slice(&s.sessions.get(self.session)?.name);
        target.push(b':');
        if self.kind != WindowTreeType::Session {
            target.extend_from_slice(s.winlinks.get(wl)?.index.to_string().as_bytes());
            target.push(b'.');
        }
        if self.kind == WindowTreeType::Pane {
            target.extend_from_slice(format!("%{}", s.panes.get(pane)?.public_id).as_bytes());
        }
        Some((
            target.into(),
            crate::cmd::find::from_winlink_pane(s, wl, pane, Default::default()),
        ))
    }
}
impl ModeTreeCallbacks for WindowTreeBackend {
    type Item = WindowTreeItem;
    fn build(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        sort: &SortCriteria,
        tag: &mut ModeTreeTag,
        filter: Option<&[u8]>,
    ) {
        self.items.clear();
        let mut sessions = Vec::new();
        crate::format::sort::get_sessions(s, sort, &mut sessions);
        let current_group = self
            .fs
            .s
            .and_then(|sid| crate::model::session::session_group_contains(s, sid));
        for sid in sessions {
            let Some(session) = s.sessions.get(sid) else {
                continue;
            };
            if self.squash_groups
                && let Some(group) = session.group
                && ((Some(group) == current_group && Some(sid) != self.fs.s)
                    || (Some(group) != current_group
                        && s.groups
                            .get(group)
                            .and_then(|g| g.sessions.first())
                            .copied()
                            != Some(sid)))
            {
                continue;
            }
            let name = session.name.clone();
            let mut windows = Vec::new();
            crate::format::sort::get_winlinks_session(s, sid, sort, &mut windows);
            let mut root = None;
            for wlid in windows {
                let Some(wl) = s.winlinks.get(wlid) else {
                    continue;
                };
                let number = wl.index;
                let window_id = wl.window;
                if s.windows.get(window_id).is_none() {
                    continue;
                }
                let mut panes = Vec::new();
                crate::format::sort::get_panes_window(s, window_id, sort, &mut panes);
                if let Some(filter) = filter {
                    panes.retain(|pid| {
                        let text = crate::format::single(
                            s,
                            None,
                            crate::format::FormatContext {
                                session: Some(sid),
                                winlink: Some(wlid),
                                pane: Some(*pid),
                                ..Default::default()
                            },
                            filter,
                        );
                        crate::format::true_value(Some(&text))
                    });
                }
                if panes.is_empty() {
                    continue;
                }
                let root = *root.get_or_insert_with(|| {
                    let index = self.items.len() as u32;
                    self.items.push(WindowTreeItem {
                        kind: WindowTreeType::Session,
                        session: sid,
                        winlink: None,
                        pane: None,
                    });
                    let text = crate::format::single(
                        s,
                        None,
                        FormatContext {
                            session: Some(sid),
                            ..Default::default()
                        },
                        &self.format,
                    );
                    tree.add(
                        None,
                        Some(index),
                        ModeTreeTag::Session(sid),
                        &name,
                        Some(&text),
                        if self.kind == WindowTreeType::Session {
                            0
                        } else {
                            -1
                        },
                    )
                });
                let index = self.items.len() as u32;
                self.items.push(WindowTreeItem {
                    kind: WindowTreeType::Window,
                    session: sid,
                    winlink: Some(wlid),
                    pane: None,
                });
                let text = crate::format::single(
                    s,
                    None,
                    FormatContext {
                        session: Some(sid),
                        winlink: Some(wlid),
                        ..Default::default()
                    },
                    &self.format,
                );
                let wi = tree.add(
                    Some(root),
                    Some(index),
                    ModeTreeTag::Winlink(wlid),
                    number.to_string().as_bytes(),
                    Some(&text),
                    if matches!(self.kind, WindowTreeType::Session | WindowTreeType::Window) {
                        0
                    } else {
                        -1
                    },
                );
                tree.items.get_mut(wi).expect("window tree item").align = true;
                for pid in panes {
                    if s.panes.get(pid).is_none()
                        || (self.hide_preview_this_pane && pid == self.owner)
                    {
                        continue;
                    }
                    let text = crate::format::single(
                        s,
                        None,
                        crate::format::FormatContext {
                            session: Some(sid),
                            winlink: Some(wlid),
                            pane: Some(pid),
                            ..Default::default()
                        },
                        &self.format,
                    );
                    let number = crate::model::pane::pane_index(s, pid).unwrap_or_default();
                    let index = self.items.len() as u32;
                    self.items.push(WindowTreeItem {
                        kind: WindowTreeType::Pane,
                        session: sid,
                        winlink: Some(wlid),
                        pane: Some(pid),
                    });
                    let pi = tree.add(
                        Some(wi),
                        Some(index),
                        ModeTreeTag::Pane(pid),
                        number.to_string().as_bytes(),
                        Some(&text),
                        -1,
                    );
                    tree.items.get_mut(pi).expect("pane tree item").align = true;
                }
            }
        }
        if self.kind != WindowTreeType::None {
            *tag = match self.kind {
                WindowTreeType::Session => self.fs.s.map(ModeTreeTag::Session),
                WindowTreeType::Window => self.fs.wl.map(ModeTreeTag::Winlink),
                _ => {
                    let single = self
                        .fs
                        .wl
                        .and_then(|wl| s.winlinks.get(wl))
                        .and_then(|wl| s.windows.get(wl.window))
                        .is_some_and(|w| w.panes.len() == 1);
                    if single {
                        self.fs.wl.map(ModeTreeTag::Winlink)
                    } else {
                        self.fs.wp.map(ModeTreeTag::Pane)
                    }
                }
            }
            .unwrap_or_default();
        }
    }
    fn prepare_draw(&mut self, s: &mut Server, item: Option<u32>, sx: u32, _sy: u32) {
        self.cache.previews.clear();
        self.cache.info.clear();
        self.cache.strip = None;
        self.cache.border = DEFAULT_CELL;
        if let Some(options) = s
            .panes
            .get(self.owner)
            .and_then(|p| s.windows.get(p.window))
            .map(|w| w.options)
        {
            style_apply(
                s,
                &mut self.cache.border,
                options,
                b"tree-mode-border-style",
                None,
            );
        }
        let Some(item) = item.and_then(|n| self.items.get(n as usize)).copied() else {
            return;
        };
        let Some((wl, pane)) = item.resolve(s) else {
            return;
        };
        if self.preview_is_info {
            let mut ft = crate::format::create_defaults(
                s,
                None,
                FormatContext {
                    session: Some(item.session),
                    winlink: Some(wl),
                    pane: Some(pane),
                    ..Default::default()
                },
            );
            let groups: &[&[&str]] = match item.kind {
                WindowTreeType::Pane => &[PANE_INFO_LINES, WINDOW_INFO_LINES, SESSION_INFO_LINES],
                WindowTreeType::Window => &[WINDOW_INFO_LINES, SESSION_INFO_LINES],
                _ => &[SESSION_INFO_LINES],
            };
            for (n, group) in groups.iter().enumerate() {
                if n != 0 {
                    self.cache.info.push(None);
                }
                for line in *group {
                    self.cache.info.push(Some(ft.expand(s, line.as_bytes())));
                }
            }
            ft.release(s);
            return;
        }
        if item.kind == WindowTreeType::Pane {
            return;
        }
        let mut entries = Vec::new();
        let current;
        if item.kind == WindowTreeType::Session {
            let Some(session) = s.sessions.get(item.session) else {
                return;
            };
            current = session
                .windows
                .values()
                .position(|&id| id == wl)
                .unwrap_or_default();
            for &link in session.windows.values() {
                if let Some(window) = s.winlinks.get(link).and_then(|wl| s.windows.get(wl.window))
                    && let Some(active) = window.active
                {
                    entries.push((link, active, window.options, false));
                }
            }
        } else {
            let Some(window) = s.winlinks.get(wl).and_then(|wl| s.windows.get(wl.window)) else {
                return;
            };
            for &pid in &window.panes {
                if self.hide_preview_this_pane && pid == self.owner {
                    continue;
                }
                if let Some(p) = s.panes.get(pid) {
                    entries.push((wl, pid, p.options, true));
                }
            }
            current = entries
                .iter()
                .position(|entry| Some(entry.1) == window.active)
                .unwrap_or(entries.len());
        }
        let Some(strip) = preview_strip(entries.len(), current, sx, self.offset) else {
            return;
        };
        self.offset = strip.offset;
        self.cache.strip = Some(strip);
        for &(link, pid, options, pane_format) in &entries[strip.start..strip.end] {
            let mut ft = crate::format::create_defaults(
                s,
                None,
                FormatContext {
                    session: Some(item.session),
                    winlink: Some(link),
                    pane: pane_format.then_some(pid),
                    ..Default::default()
                },
            );
            let mut border = DEFAULT_CELL;
            style_apply(
                s,
                &mut border,
                options,
                b"tree-mode-border-style",
                Some(&mut ft),
            );
            let mut label_cell = DEFAULT_CELL;
            style_apply(
                s,
                &mut label_cell,
                options,
                b"tree-mode-preview-style",
                Some(&mut ft),
            );
            label_cell.bg = border.bg;
            let label_format = s
                .options
                .get_string(options, b"tree-mode-preview-format")
                .to_vec();
            let label = ft.expand(s, &label_format);
            ft.release(s);
            self.cache.previews.push(PreviewItem {
                pane: pid,
                label,
                border,
                label_cell,
            });
        }
    }
    fn draw(
        &self,
        s: &Server,
        item: Option<&Self::Item>,
        ctx: &mut ScreenWriteCtx<'_>,
        sx: u32,
        sy: u32,
    ) {
        let Some(item) = item else {
            return;
        };
        let Some((_, pid)) = item.resolve(s) else {
            return;
        };
        let (cx, cy) = (ctx.screen.cx, ctx.screen.cy);
        if self.preview_is_info {
            let mut row = 0;
            for line in &self.cache.info {
                if row == sy {
                    break;
                }
                ctx.cursormove(cx as i32, (cy + row) as i32, false);
                if let Some(line) = line {
                    format_draw(ctx, &DEFAULT_CELL, sx, line, None, false);
                } else {
                    ctx.hline(
                        sx,
                        false,
                        false,
                        BoxLines::Default,
                        Some(&self.cache.border),
                    );
                    if sx > 14 {
                        let mut cell = self.cache.border;
                        cell.attr.insert(GridAttributes::CHARSET);
                        ctx.cursormove((cx + 14) as i32, (cy + row) as i32, false);
                        ctx.puts(&cell, b"n");
                    }
                }
                row += 1;
            }
            if sx > 14 && row < sy {
                ctx.cursormove((cx + 14) as i32, (cy + row) as i32, false);
                ctx.vline(sy - row, false, false, Some(&self.cache.border));
            }
            return;
        }
        if item.kind == WindowTreeType::Pane {
            if !self.hide_preview_this_pane || pid != self.owner {
                if let Some(pane) = s.panes.get(pid) {
                    ctx.preview(&pane.base, sx, sy);
                }
            }
            return;
        }
        let Some(strip) = self.cache.strip else {
            return;
        };
        if strip.left {
            ctx.cursormove((cx + 2) as i32, cy as i32, false);
            ctx.vline(sy, false, false, Some(&self.cache.border));
            ctx.cursormove(cx as i32, (cy + sy / 2) as i32, false);
            ctx.puts(&self.cache.border, b"<");
        }
        if strip.right {
            ctx.cursormove((cx + sx - 3) as i32, cy as i32, false);
            ctx.vline(sy, false, false, Some(&self.cache.border));
            ctx.cursormove((cx + sx - 1) as i32, (cy + sy / 2) as i32, false);
            ctx.puts(&self.cache.border, b">");
        }
        for (n, preview) in self.cache.previews.iter().enumerate() {
            let x = cx + 3 * u32::from(strip.left) + n as u32 * strip.each;
            let last = n + 1 == self.cache.previews.len();
            let columns = if last {
                strip.each + strip.remaining
            } else {
                strip.each - 1
            };
            if columns == 0 {
                continue;
            }
            if let Some(pane) = s.panes.get(preview.pane) {
                ctx.cursormove(x as i32, cy as i32, false);
                ctx.preview(&pane.base, columns, sy);
            }
            draw_label(ctx, x, cy, columns, sy, preview);
            if !last {
                ctx.cursormove((x + columns) as i32, cy as i32, false);
                ctx.vline(sy, false, false, Some(&preview.border));
            }
        }
    }
    fn has_draw(&self) -> bool {
        true
    }
    fn search(
        &self,
        s: &Server,
        item: Option<&Self::Item>,
        needle: &[u8],
        icase: bool,
    ) -> Option<bool> {
        let Some(item) = item else {
            return Some(false);
        };
        let Some((wl, pane)) = item.resolve(s) else {
            return Some(false);
        };
        Some(match item.kind {
            WindowTreeType::Session => s
                .sessions
                .get(item.session)
                .is_some_and(|session| contains(&session.name, needle, icase)),
            WindowTreeType::Window => s
                .winlinks
                .get(wl)
                .and_then(|wl| s.windows.get(wl.window))
                .is_some_and(|window| contains(&window.name, needle, icase)),
            WindowTreeType::Pane => s
                .panes
                .get(pane)
                .and_then(|p| p.fd.as_ref())
                .and_then(|fd| rmux_sys::osdep::get_name(fd.as_fd()))
                .is_some_and(|name| !name.is_empty() && contains(&name, needle, icase)),
            WindowTreeType::None => false,
        })
    }
    fn menu(&mut self, mode: ModeId, client: Option<ClientId>, key: KeyCode) -> ModeAction {
        Box::new(move |s| {
            if let Some(client) = client {
                if let Some(driver) = s
                    .panes
                    .get(mode.owner)
                    .and_then(|p| p.modes.first().filter(|m| m.id == mode))
                    .map(|m| std::rc::Rc::clone(&m.driver))
                {
                    driver.key(s, mode, client, key, None);
                }
            }
        })
    }
    fn height(&self, _sy: u32) -> Option<u32> {
        None
    }
    fn key(&mut self, s: &mut Server, item: Option<u32>, line: u32) -> Option<KeyCode> {
        let item = item.and_then(|n| self.items.get(n as usize));
        let context = item.map_or_else(FormatContext::default, WindowTreeItem::context);
        let mut tree = crate::format::create_defaults(s, None, context);
        tree.add(b"line", line.to_string().as_bytes().into());
        let key = tree.expand(s, &self.key_format);
        tree.release(s);
        Some(rmux_tty::key_string::parse_key_name(&key))
    }
    fn can_swap(
        &self,
        s: &Server,
        cur: &Self::Item,
        other: &Self::Item,
        sort: &SortCriteria,
    ) -> bool {
        can_swap(s, cur, other, sort)
    }
    fn swap(&self, cur: &Self::Item, other: &Self::Item, sort: &SortCriteria) -> ModeAction {
        let (cur, other, sort) = (*cur, *other, *sort);
        Box::new(move |s| {
            if !can_swap(s, &cur, &other, &sort) {
                return;
            }
            let (Some(a), Some(b)) = (cur.winlink, other.winlink) else {
                return;
            };
            let session = cur.session;
            let (Some(wa), Some(wb)) = (
                s.winlinks.get(a).map(|w| w.window),
                s.winlinks.get(b).map(|w| w.window),
            ) else {
                return;
            };
            if let Some(w) = s.windows.get_mut(wa) {
                w.links.retain(|&wl| wl != a);
            }
            if let Some(w) = s.windows.get_mut(wb) {
                w.links.retain(|&wl| wl != b);
            }
            if let Some(w) = s.windows.get_mut(wa) {
                w.links.push(b);
            }
            if let Some(w) = s.windows.get_mut(wb) {
                w.links.push(a);
            }
            if let Some(w) = s.winlinks.get_mut(a) {
                w.window = wb;
            }
            if let Some(w) = s.winlinks.get_mut(b) {
                w.window = wa;
            }
            if let Some(current) = s.sessions.get(session).and_then(|s| s.current) {
                if current == a {
                    let _ = crate::model::session::session_set_current(s, session, Some(b));
                } else if current == b {
                    let _ = crate::model::session::session_set_current(s, session, Some(a));
                }
            }
            crate::model::session::session_group_synchronize_from(s, session);
            crate::server::operations::server_redraw_session_group(s, session);
            s.effects
                .push_back(crate::model::ModelEffect::RecalculateSizes);
        })
    }
    fn sort(&self, sort: &mut SortCriteria) -> bool {
        use crate::format::sort::SortOrder;
        sort.order_seq = Some(&[
            SortOrder::Index,
            SortOrder::Name,
            SortOrder::Activity,
            SortOrder::Z,
        ]);
        if sort.order == SortOrder::End {
            sort.order = SortOrder::Index;
        }
        true
    }
    fn help(&self) -> Option<(u32, &'static str, &'static [&'static str])> {
        Some((51, "item", HELP_LINES))
    }
    fn items(&self) -> &[Self::Item] {
        &self.items
    }
}
pub struct WindowTreeDriver {
    pub args: Args,
    pub target: CmdFindState,
}
impl PaneModeDriver for WindowTreeDriver {
    fn init(&self, s: &mut Server, id: ModeId) -> Option<Screen> {
        let pane = s.panes.get(id.owner)?;
        let (sx, sy) = (pane.sx, pane.sy);
        let preview = if self.args.has(b'N') == 1 {
            ModeTreePreview::Off
        } else if self.args.has(b'N') > 1 {
            ModeTreePreview::Big
        } else {
            ModeTreePreview::Normal
        };
        let mut state = TreeModeState {
            tree: ModeTreeData::start(sx, sy, preview),
            backend: WindowTreeBackend {
                items: Vec::new(),
                fs: self.target,
                kind: if self.args.has(b's') > 0 {
                    WindowTreeType::Session
                } else if self.args.has(b'w') > 0 {
                    WindowTreeType::Window
                } else {
                    WindowTreeType::Pane
                },
                template: self
                    .args
                    .string(0)
                    .unwrap_or(b"switch-client -Zt '%%'")
                    .into(),
                format: self.args.get(b'F').unwrap_or(DEFAULT_FORMAT).into(),
                key_format: self.args.get(b'K').unwrap_or(DEFAULT_KEY_FORMAT).into(),
                owner: id.owner,
                squash_groups: self.args.has(b'G') == 0,
                hide_preview_this_pane: self.args.has(b'h') != 0,
                preview_is_info: false,
                prompt_flags: if self.args.has(b'y') != 0 {
                    PromptFlags::ACCEPT
                } else {
                    PromptFlags::default()
                },
                offset: 0,
                cache: DrawCache::default(),
            },
        };
        state.tree.filter = self.args.get(b'f').map(ByteString::from);
        state.tree.sort.reversed = self.args.has(b'r') > 0;
        if self.args.has(b'O') != 0 {
            state.tree.sort.order = crate::format::sort::order_from_string(self.args.get(b'O'));
        }
        state.tree.menu_items = MENU_ITEMS;
        state.tree.view_name(b"preview");
        let mut screen = Screen::new(sx, sy, 0, ScreenResetPolicy::default(), &mut s.hyperlinks)
            .expect("tree screen");
        screen.mode.remove(rmux_emu::screen::ScreenMode::CURSOR);
        let (mut state, mut screen) =
            super::tree::init_zoom(s, id, state, screen, Some(&self.args))?;
        state.build(s);
        state.backend.kind = WindowTreeType::None;
        state.draw(s, &mut screen);
        s.panes
            .get_mut(id.owner)?
            .modes
            .iter_mut()
            .find(|m| m.id == id)?
            .data = Some(Box::new(state));
        Some(screen)
    }
    fn free(&self, s: &mut Server, mut mode: PaneMode) {
        if let Some(data) = mode.data.take() {
            data.downcast::<TreeModeState<WindowTreeBackend>>()
                .expect("tree state")
                .free(s);
        }
        if let Some(mut screen) = mode.screen.take() {
            let _ = screen.release(
                &mut s.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
        }
    }
    fn resize(&self, s: &mut Server, id: ModeId, sx: u32, sy: u32) {
        with_state(s, id, |s, state, screen| state.resize(s, screen, sx, sy));
    }
    fn update(&self, s: &mut Server, id: ModeId) {
        with_state(s, id, |s, state, screen| {
            state.build(s);
            state.draw(s, screen);
        });
    }
    fn key(
        &self,
        s: &mut Server,
        id: ModeId,
        c: ClientId,
        key: KeyCode,
        m: Option<&crate::client::ResolvedMouseEvent>,
    ) {
        let mut finished = false;
        let mut after: Option<ModeAction> = None;
        let mut pending: Option<ModeAction> = None;
        with_state(s, id, |s, state, screen| {
            let old = state.tree.get_current().map(|i| i.tag);
            let result = state.key(s, id, Some(c), key, m, screen);
            pending = state.tree.pending_action.take();
            let mut key = result.key;
            if old != state.tree.get_current().map(|i| i.tag) {
                state.backend.offset = 0;
            }
            if m.is_some() && key.0 == SpecialKey::MOUSEDOWN1_PANE {
                key = preview_mouse(s, state, result.mouse_x);
                if old != state.tree.get_current().map(|i| i.tag) {
                    state.backend.offset = 0;
                }
            }
            let current = current_item(state);
            match key.0 {
                60 => state.backend.offset -= 1,
                62 => state.backend.offset += 1,
                72 => {
                    if let Some(session) = state.backend.fs.s {
                        expand_tag(&mut state.tree, ModeTreeTag::Session(session));
                    }
                    if let Some(wl) = state.backend.fs.wl {
                        expand_tag(&mut state.tree, ModeTreeTag::Winlink(wl));
                    }
                    let tag = ModeTreeTag::Pane(id.owner);
                    if state
                        .tree
                        .lines
                        .iter()
                        .any(|l| state.tree.items.get(l.item).is_some_and(|i| i.tag == tag))
                    {
                        state.tree.set_current(tag);
                    } else if let Some(wl) = state.backend.fs.wl {
                        state.tree.set_current(ModeTreeTag::Winlink(wl));
                    }
                }
                109 => {
                    after = Some(Box::new(move |s| {
                        if let Some(item) = current
                            && let Some((wl, pane)) = item.resolve(s)
                        {
                            crate::server::operations::server_set_marked(
                                s,
                                Some(item.session),
                                Some(wl),
                                Some(pane),
                            );
                        }
                        rebuild_live(s, id);
                    }));
                }
                77 => {
                    after = Some(Box::new(move |s| {
                        crate::server::operations::server_clear_marked(s);
                        rebuild_live(s, id);
                    }))
                }
                105 => {
                    state.backend.preview_is_info = !state.backend.preview_is_info;
                    state.tree.view_name(if state.backend.preview_is_info {
                        b"info"
                    } else {
                        b"preview"
                    });
                }
                120 | 88 => {
                    let count = state.tree.count_tagged();
                    let message = if key.0 == 88 {
                        (count != 0).then(|| format!("Kill {count} tagged? ").into_bytes())
                    } else {
                        current.and_then(|item| kill_message(s, item))
                    };
                    if let Some(message) = message {
                        let flags = PromptFlags::SINGLE
                            | PromptFlags::NOFORMAT
                            | state.backend.prompt_flags;
                        let tagged = key.0 == 88;
                        state.set_prompt(
                            s,
                            id,
                            Some(c),
                            PromptCreateData {
                                prompt: message.into(),
                                flags,
                                ty: PromptType::Command,
                                ..Default::default()
                            },
                            move |text, _| {
                                if !text.is_some_and(|text| text == b"y" || text == b"Y") {
                                    return None;
                                }
                                Some(Box::new(move |s| kill_selected(s, id, Some(c), tagged)))
                            },
                        );
                    }
                }
                58 => {
                    let count = state.tree.count_tagged();
                    let message = if count == 0 {
                        "(current) ".to_owned()
                    } else {
                        format!("({count} tagged) ")
                    };
                    state.set_prompt(
                        s,
                        id,
                        Some(c),
                        PromptCreateData {
                            prompt: message.into(),
                            flags: PromptFlags::NOFORMAT,
                            ty: PromptType::Command,
                            ..Default::default()
                        },
                        move |text, _| {
                            let text = text.filter(|text| !text.is_empty())?.to_vec();
                            Some(Box::new(move |s| command_selected(s, id, Some(c), &text)))
                        },
                    );
                }
                13 => {
                    if let Some((name, _)) = current.and_then(|item| item.target(s)) {
                        let template = state.backend.template.clone();
                        after = Some(Box::new(move |s| {
                            run_command(s, Some(c), None, &template, &name)
                        }));
                    }
                    finished = true;
                }
                _ => {}
            }
            finished |= result.finished;
            if !finished {
                state.draw(s, screen);
            }
        });
        if let Some(action) = pending {
            action(s);
        }
        if let Some(action) = after {
            action(s);
        }
        if finished && first_mode(s, id) {
            let _ = crate::model::pane::pane_reset_mode(s, id.owner);
        }
    }
    fn append_output(&self, _s: &mut Server, _id: ModeId, _bytes: &[u8]) -> Result<(), ModelError> {
        Ok(())
    }
}
type WindowTreeState = TreeModeState<WindowTreeBackend>;
fn first_mode(s: &Server, id: ModeId) -> bool {
    s.panes
        .get(id.owner)
        .and_then(|p| p.modes.first())
        .is_some_and(|m| m.id == id)
}
fn with_state(
    s: &mut Server,
    id: ModeId,
    f: impl FnOnce(&mut Server, &mut WindowTreeState, &mut Screen),
) {
    let Some(entry) = s
        .panes
        .get_mut(id.owner)
        .and_then(|p| p.modes.iter_mut().find(|m| m.id == id))
    else {
        return;
    };
    if entry.data.is_none() || entry.screen.is_none() {
        return;
    }
    let mut state = *entry
        .data
        .take()
        .expect("tree data")
        .downcast::<WindowTreeState>()
        .expect("tree state");
    let mut screen = entry.screen.take().expect("tree screen");
    f(s, &mut state, &mut screen);
    if let Some(entry) = s
        .panes
        .get_mut(id.owner)
        .and_then(|p| p.modes.iter_mut().find(|m| m.id == id))
    {
        entry.data = Some(Box::new(state));
        entry.screen = Some(screen);
    }
    if let Some(pane) = s.panes.get_mut(id.owner) {
        pane.flags.insert(crate::model::PaneFlags::REDRAW);
    }
}
fn current_item(state: &WindowTreeState) -> Option<WindowTreeItem> {
    state
        .tree
        .get_current()
        .and_then(|i| i.item)
        .and_then(|n| state.backend.items.get(n as usize))
        .copied()
}
fn selected_items(s: &mut Server, id: ModeId, tagged: bool) -> Vec<WindowTreeItem> {
    let mut selected = Vec::new();
    if first_mode(s, id) {
        with_state(s, id, |_, state, _| {
            if tagged {
                selected.extend(
                    state
                        .tree
                        .tagged_items(true)
                        .into_iter()
                        .filter_map(|n| state.backend.items.get(n as usize))
                        .copied(),
                );
            } else {
                selected.extend(current_item(state));
            }
        });
    }
    selected
}
fn rebuild_live(s: &mut Server, id: ModeId) {
    if first_mode(s, id) {
        with_state(s, id, |s, state, screen| {
            state.build(s);
            state.draw(s, screen);
        });
    }
}
fn queue_done(s: &mut Server, id: ModeId, c: Option<ClientId>) {
    let callback = crate::cmd::queue::callback_for::<Server>(move |s, _| {
        rebuild_live(s, id);
        crate::cmd::queue::CmdReturn::Normal
    });
    if let Ok(batch) = s.queue.get_callback("window_tree_command_done", callback) {
        let _ = crate::cmd::queue::append(s, c, batch);
    }
}
fn command_selected(s: &mut Server, id: ModeId, c: Option<ClientId>, command: &[u8]) {
    let selected = selected_items(s, id, true);
    for item in selected {
        if let Some((name, fs)) = item.target(s) {
            run_command(s, c, Some(&fs), command, &name);
        }
    }
    if first_mode(s, id) {
        queue_done(s, id, c);
    }
}
fn kill_selected(s: &mut Server, id: ModeId, c: Option<ClientId>, tagged: bool) {
    let selected = selected_items(s, id, tagged);
    if selected.is_empty() {
        return;
    }
    for item in selected {
        let Some((wl, pane)) = item.resolve(s) else {
            continue;
        };
        match item.kind {
            WindowTreeType::Session => {
                crate::server::operations::server_destroy_session(s, item.session);
                crate::model::session::session_destroy(s, item.session, true);
            }
            WindowTreeType::Window => {
                if let Some(window) = s.winlinks.get(wl).map(|wl| wl.window) {
                    let _ = crate::server::operations::server_kill_window(s, window, false);
                }
            }
            WindowTreeType::Pane => {
                let _ = crate::server::operations::server_kill_pane(s, pane);
            }
            WindowTreeType::None => {}
        }
    }
    crate::server::operations::server_renumber_all(s);
    queue_done(s, id, c);
}
fn kill_message(s: &Server, item: WindowTreeItem) -> Option<Vec<u8>> {
    let (wl, pane) = item.resolve(s)?;
    match item.kind {
        WindowTreeType::Session => {
            let mut msg = b"Kill session ".to_vec();
            msg.extend_from_slice(&s.sessions.get(item.session)?.name);
            msg.extend_from_slice(b"? ");
            Some(msg)
        }
        WindowTreeType::Window => {
            Some(format!("Kill window {}? ", s.winlinks.get(wl)?.index).into_bytes())
        }
        WindowTreeType::Pane => {
            Some(format!("Kill pane {}? ", crate::model::pane::pane_index(s, pane)?).into_bytes())
        }
        WindowTreeType::None => None,
    }
}
fn expand_tag(tree: &mut ModeTreeData, tag: ModeTreeTag) {
    fn visit(
        tree: &mut ModeTreeData,
        ids: &[crate::ids::ModeTreeItemId],
        tag: ModeTreeTag,
    ) -> bool {
        for &id in ids {
            let Some(item) = tree.items.get_mut(id) else {
                continue;
            };
            if item.tag == tag {
                item.expanded = true;
                return true;
            }
            let children = item.children.clone();
            if visit(tree, &children, tag) {
                return true;
            }
        }
        false
    }
    visit(tree, &tree.children.clone(), tag);
    tree.build_lines();
}
fn preview_mouse(s: &Server, state: &mut WindowTreeState, x: u32) -> KeyCode {
    let Some(item) = current_item(state) else {
        return KeyCode(SpecialKey::NONE);
    };
    let Some(strip) = state.backend.cache.strip else {
        return KeyCode(SpecialKey::NONE);
    };
    if strip.left && x <= 3 {
        return KeyCode(60);
    }
    if strip.right && x >= state.tree.width.saturating_sub(4) {
        return KeyCode(62);
    }
    let x = if strip.left {
        x.saturating_sub(3)
    } else {
        x.saturating_sub(1)
    };
    let mut column = x / strip.each;
    if strip.start + column as usize >= strip.end {
        column = (strip.end - 1) as u32;
    }
    let index = strip.start + column as usize;
    let tag = match item.kind {
        WindowTreeType::Session => s
            .sessions
            .get(item.session)
            .and_then(|session| session.windows.values().nth(index))
            .copied()
            .map(ModeTreeTag::Winlink),
        WindowTreeType::Window => item
            .resolve(s)
            .and_then(|(wl, _)| s.winlinks.get(wl))
            .and_then(|wl| s.windows.get(wl.window))
            .and_then(|w| w.panes.get(index))
            .copied()
            .map(ModeTreeTag::Pane),
        _ => return KeyCode(SpecialKey::NONE),
    };
    state.tree.expand_current();
    if let Some(tag) = tag {
        state.tree.set_current(tag);
    }
    KeyCode(13)
}
fn can_swap(s: &Server, cur: &WindowTreeItem, other: &WindowTreeItem, sort: &SortCriteria) -> bool {
    if cur.kind != WindowTreeType::Window
        || other.kind != WindowTreeType::Window
        || cur.session != other.session
    {
        return false;
    }
    let (Some((a, _)), Some((b, _))) = (cur.resolve(s), other.resolve(s)) else {
        return false;
    };
    !crate::format::sort::would_window_tree_swap(s, sort, a, b)
}
fn contains(hay: &[u8], needle: &[u8], icase: bool) -> bool {
    let hay = rmux_util::bytes::cstr(hay);
    let needle = rmux_util::bytes::cstr(needle);
    needle.is_empty()
        || hay.windows(needle.len()).any(|part| {
            part.iter().zip(needle).all(|(&a, &b)| {
                if icase {
                    tolower(a) == tolower(b)
                } else {
                    a == b
                }
            })
        })
}
fn draw_label(
    ctx: &mut ScreenWriteCtx<'_>,
    x: u32,
    y: u32,
    sx: u32,
    sy: u32,
    preview: &PreviewItem,
) {
    if sx < 5 || sy < 3 {
        return;
    }
    let trimmed;
    let label = if width(&preview.label) > sx - 4 {
        trimmed = trim_left(&preview.label, sx - 4);
        trimmed.as_ref()
    } else {
        preview.label.as_ref()
    };
    let columns = width(label);
    if columns == 0 {
        return;
    }
    let ox = (sx - columns).div_ceil(2);
    let oy = sy.div_ceil(2);
    ctx.cursormove((x + ox - 2) as i32, (y + oy - 1) as i32, false);
    ctx.draw_box(columns + 4, 3, BoxLines::Default, Some(&preview.border));
    ctx.cursormove((x + ox - 1) as i32, (y + oy) as i32, false);
    ctx.clearcharacter(columns + 2, preview.border.bg);
    ctx.cursormove((x + ox) as i32, (y + oy) as i32, false);
    format_draw(ctx, &preview.label_cell, columns, label, None, false);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewStrip {
    pub start: usize,
    pub end: usize,
    pub each: u32,
    pub remaining: u32,
    pub left: bool,
    pub right: bool,
    pub offset: i32,
}
pub fn preview_strip(total: usize, current: usize, sx: u32, offset: i32) -> Option<PreviewStrip> {
    if total == 0 {
        return None;
    }
    let visible = if sx as usize / total < 24 {
        (sx / 24).max(1) as usize
    } else {
        total
    };
    let start = if current < visible {
        0
    } else if current >= total - visible {
        total - visible
    } else {
        current - visible / 2
    };
    let end = start + visible;
    let offset = offset.clamp(-(start as i32), (total - end) as i32);
    let start = (start as i32 + offset) as usize;
    let end = (end as i32 + offset) as usize;
    let mut left = start != 0;
    let mut right = end != total;
    if (left && right && sx <= 6) || ((left || right) && sx <= 3) {
        left = false;
        right = false;
    }
    let width = sx - 3 * u32::from(left) - 3 * u32::from(right);
    let each = width / visible as u32;
    (each != 0).then_some(PreviewStrip {
        start,
        end,
        each,
        remaining: width - visible as u32 * each,
        left,
        right,
        offset,
    })
}
pub const DEFAULT_COMMAND: &[u8] = b"switch-client -Zt '%%'";
pub const DEFAULT_FORMAT:&[u8]=b"#{?pane_format,#{?pane_marked,#[fg=thememagenta],}#{?pane_floating_flag,#[underscore],}#{pane_current_command}#[fg=themelightgrey]#{pane_flags}#{?#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}},: \"#{pane_title}\",},window_format,#{?window_marked_flag,#[fg=thememagenta],}#{window_name}#[fg=themelightgrey]#{window_flags}#{?#{&&:#{==:#{window_panes},1},#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}}},: \"#{pane_title}\",},#[fg=themelightgrey]#{session_windows} windows#{?session_grouped, (group #{session_group}: #{session_group_list}),}#{?session_attached, (attached),}}";
pub const DEFAULT_KEY_FORMAT: &[u8] =
    b"#{?#{e|<:#{line},10},#{line},#{e|<:#{line},36},M-#{a:#{e|+:97,#{e|-:#{line},10}}}}";

pub const PANE_INFO_LINES: &[&str] = &[
    "#[fg=themelightgrey]Pane          #[#{E:tree-mode-border-style},acs]x#[default] #{pane_index} #[fg=themelightgrey](#{pane_id})#[default]",
    "#[fg=themelightgrey]Title         #[#{E:tree-mode-border-style},acs]x#[default] #{pane_title}",
    "#[fg=themelightgrey]Command       #[#{E:tree-mode-border-style},acs]x#[default] #{pane_current_command} #[fg=themelightgrey](PID #{pane_pid})#[default]",
    "#[fg=themelightgrey]Path          #[#{E:tree-mode-border-style},acs]x#[default] #{pane_current_path}",
    "#[fg=themelightgrey]TTY           #[#{E:tree-mode-border-style},acs]x#[default] #{pane_tty}",
    "#[fg=themelightgrey]Position      #[#{E:tree-mode-border-style},acs]x#[default] #{pane_x},#{pane_y} #{pane_width}x#{pane_height}",
    "#[fg=themelightgrey]Mode          #[#{E:tree-mode-border-style},acs]x#[default] #{?pane_in_mode,#{pane_mode},none}",
    "#[fg=themelightgrey]Flags         #[#{E:tree-mode-border-style},acs]x#[default] #{?pane_active,#[fg=themegreen],#[fg=themelightgrey]}active#[default] #{?window_zoomed_flag,#[fg=themegreen],#[fg=themelightgrey]}zoomed#[default] #{?pane_marked,#[fg=themegreen],#[fg=themelightgrey]}marked#[default] #{?pane_synchronized,#[fg=themegreen],#[fg=themelightgrey]}sync#[default] #{?pane_dead,#[fg=themegreen],#[fg=themelightgrey]}dead#[default] #{?pane_pipe,#[fg=themegreen],#[fg=themelightgrey]}piped#[default]",
];
pub const WINDOW_INFO_LINES: &[&str] = &[
    "#[fg=themelightgrey]Window        #[#{E:tree-mode-border-style},acs]x#[default] #{window_index}: #{window_name} #[fg=themelightgrey](#{window_id})#[default]",
    "#[fg=themelightgrey]Size          #[#{E:tree-mode-border-style},acs]x#[default] #{window_width}x#{window_height}",
    "#[fg=themelightgrey]Panes         #[#{E:tree-mode-border-style},acs]x#[default] #{window_panes}",
    "#[fg=themelightgrey]Activity Time #[#{E:tree-mode-border-style},acs]x#[default] #{t:window_activity} #[fg=themelightgrey](#{t/r:window_activity})#[default]",
    "#[fg=themelightgrey]Sessions      #[#{E:tree-mode-border-style},acs]x#[default] #{s/,/ /:window_linked_sessions_list}",
    "#[fg=themelightgrey]Flags         #[#{E:tree-mode-border-style},acs]x#[default] #{?window_active,#[fg=themegreen],#[fg=themelightgrey]}active#[default] #{?window_last_flag,#[fg=themegreen],#[fg=themelightgrey]}last#[default] #{?window_bell_flag,#[fg=themegreen],#[fg=themelightgrey]}bell#[default] #{?window_activity_flag,#[fg=themegreen],#[fg=themelightgrey]}activity#[default] #{?window_silence_flag,#[fg=themegreen],#[fg=themelightgrey]}silence#[default] #{?window_zoomed_flag,#[fg=themegreen],#[fg=themelightgrey]}zoomed#[default] #{?window_marked_flag,#[fg=themegreen],#[fg=themelightgrey]}marked#[default]",
];
pub const SESSION_INFO_LINES: &[&str] = &[
    "#[fg=themelightgrey]Session       #[#{E:tree-mode-border-style},acs]x#[default] #{session_name} #[fg=themelightgrey](#{session_id})#[default]",
    "#[fg=themelightgrey]Created Time  #[#{E:tree-mode-border-style},acs]x#[default] #{t:session_created} #[fg=themelightgrey](#{t/r:session_created})#[default]",
    "#[fg=themelightgrey]Activity Time #[#{E:tree-mode-border-style},acs]x#[default] #{t:session_activity} #[fg=themelightgrey](#{t/r:session_activity})#[default]",
    "#[fg=themelightgrey]Attached Time #[#{E:tree-mode-border-style},acs]x#[default] #{?#{t:session_last_attached},#{t:session_last_attached} #[fg=themelightgrey](#{t/r:session_last_attached})#[default],never}",
    "#[fg=themelightgrey]Clients       #[#{E:tree-mode-border-style},acs]x#[default] #{s/,/ /:session_attached_list}",
    "#[fg=themelightgrey]Windows       #[#{E:tree-mode-border-style},acs]x#[default] #{session_windows}",
    "#[fg=themelightgrey]Path          #[#{E:tree-mode-border-style},acs]x#[default] #{session_path}",
    "#[fg=themelightgrey]Group         #[#{E:tree-mode-border-style},acs]x#[default] #{?session_grouped,#{session_group} (#{session_group_size}),none}",
    "#[fg=themelightgrey]Flags         #[#{E:tree-mode-border-style},acs]x#[default] #{?session_attached,#[fg=themegreen],#[fg=themelightgrey]}attached#[default] #{?session_grouped,#[fg=themegreen],#[fg=themelightgrey]}grouped#[default] #{?session_marked,#[fg=themegreen],#[fg=themelightgrey]}marked#[default] #{?session_bell_flag,#[fg=themegreen],#[fg=themelightgrey]}bell#[default] #{?session_activity_flag,#[fg=themegreen],#[fg=themelightgrey]}activity#[default] #{?session_silence_flag,#[fg=themegreen],#[fg=themelightgrey]}silence#[default]",
];
pub const HELP_LINES: &[&str] = &[
    "#[fg=themelightgrey]      Enter #[#{E:tree-mode-border-style},acs]x#[default] Choose selected item",
    "#[fg=themelightgrey]       S-Up #[#{E:tree-mode-border-style},acs]x#[default] Swap current and previous window",
    "#[fg=themelightgrey]     S-Down #[#{E:tree-mode-border-style},acs]x#[default] Swap current and next window",
    "#[fg=themelightgrey]          x #[#{E:tree-mode-border-style},acs]x#[default] Kill selected item",
    "#[fg=themelightgrey]          X #[#{E:tree-mode-border-style},acs]x#[default] Kill tagged items",
    "#[fg=themelightgrey]          < #[#{E:tree-mode-border-style},acs]x#[default] Scroll previews left",
    "#[fg=themelightgrey]          > #[#{E:tree-mode-border-style},acs]x#[default] Scroll previews right",
    "#[fg=themelightgrey]          m #[#{E:tree-mode-border-style},acs]x#[default] Set the marked pane",
    "#[fg=themelightgrey]          M #[#{E:tree-mode-border-style},acs]x#[default] Clear the marked pane",
    "#[fg=themelightgrey]          i #[#{E:tree-mode-border-style},acs]x#[default] Toggle session, window and pane information",
    "#[fg=themelightgrey]          : #[#{E:tree-mode-border-style},acs]x#[default] Run a command for each tagged item",
    "#[fg=themelightgrey]          f #[#{E:tree-mode-border-style},acs]x#[default] Enter a format",
    "#[fg=themelightgrey]          H #[#{E:tree-mode-border-style},acs]x#[default] Jump to the starting pane",
];
pub const MENU_ITEMS: &[ModeTreeMenuItem] = &[
    ModeTreeMenuItem {
        name: "Select",
        key: 13,
    },
    ModeTreeMenuItem {
        name: "Expand",
        key: SpecialKey::RIGHT,
    },
    ModeTreeMenuItem {
        name: "Mark",
        key: b'm' as u64,
    },
    ModeTreeMenuItem {
        name: "",
        key: SpecialKey::NONE,
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
        key: SpecialKey::NONE,
    },
    ModeTreeMenuItem {
        name: "Kill",
        key: b'x' as u64,
    },
    ModeTreeMenuItem {
        name: "Kill Tagged",
        key: b'X' as u64,
    },
    ModeTreeMenuItem {
        name: "",
        key: SpecialKey::NONE,
    },
    ModeTreeMenuItem {
        name: "Cancel",
        key: b'q' as u64,
    },
];
#[cfg(test)]
mod tests {
    use super::*;
    struct Parser;
    impl crate::options::CommandParser for Parser {
        fn parse_from_string(&mut self, _: &[u8]) -> crate::cmd::parse::CmdParseResult {
            Ok(std::rc::Rc::new(crate::cmd::CommandList::default()))
        }
    }
    fn server() -> Server {
        let mut s = Server::new();
        s.options.load_defaults(&mut Parser);
        s
    }
    fn session(s: &mut Server, name: &[u8]) -> SessionId {
        let options = s.options.create(Some(s.options.global_s));
        crate::model::session::session_create(
            s,
            crate::model::session::SessionCreate {
                prefix: None,
                name: Some(name.to_vec()),
                cwd: b"/tmp".to_vec(),
                environment: Default::default(),
                options,
                termios: None,
            },
        )
    }
    fn window(s: &mut Server, sid: SessionId, index: i32, count: usize) -> (WinlinkId, PaneId) {
        let window = crate::model::window::window_create(s, 80, 24, 0, 0).unwrap();
        let wl = crate::model::winlink::winlink_add(s, sid, window, index).unwrap();
        let mut panes = Vec::new();
        for _ in 0..count {
            panes.push(crate::model::pane::pane_create(s, window, 80, 24, 0).unwrap());
        }
        let pane = panes[0];
        let w = s.windows.get_mut(window).unwrap();
        w.panes = panes;
        w.active = Some(pane);
        s.sessions.get_mut(sid).unwrap().current.get_or_insert(wl);
        (wl, pane)
    }
    fn backend(s: &Server, _sid: SessionId, wl: WinlinkId, pane: PaneId) -> WindowTreeBackend {
        WindowTreeBackend {
            items: Vec::new(),
            fs: crate::cmd::find::from_winlink_pane(s, wl, pane, Default::default()),
            kind: WindowTreeType::Pane,
            template: DEFAULT_COMMAND.into(),
            format: b"#{session_format}:#{window_format}:#{pane_format}:#{pane_title}"
                .as_slice()
                .into(),
            key_format: DEFAULT_KEY_FORMAT.into(),
            owner: pane,
            squash_groups: true,
            hide_preview_this_pane: false,
            preview_is_info: false,
            prompt_flags: PromptFlags::default(),
            offset: 0,
            cache: DrawCache::default(),
        }
    }
    #[test]
    fn groups_choose_target_member_and_first_other_member() {
        let mut s = server();
        let a = session(&mut s, b"a");
        let b = session(&mut s, b"b");
        let c = session(&mut s, b"c");
        let d = session(&mut s, b"d");
        let _ = window(&mut s, a, 0, 1);
        let (wl, pane) = window(&mut s, b, 0, 1);
        let _ = window(&mut s, c, 0, 1);
        let _ = window(&mut s, d, 0, 1);
        let first = crate::model::session::session_group_new(&mut s, b"first");
        crate::model::session::session_group_add(&mut s, first, a);
        crate::model::session::session_group_add(&mut s, first, b);
        let second = crate::model::session::session_group_new(&mut s, b"second");
        crate::model::session::session_group_add(&mut s, second, d);
        crate::model::session::session_group_add(&mut s, second, c);
        let mut backend = backend(&s, b, wl, pane);
        let mut tree = ModeTreeData::start(80, 24, ModeTreePreview::Off);
        let mut tag = ModeTreeTag::Unset;
        backend.build(&mut s, &mut tree, &SortCriteria::default(), &mut tag, None);
        let roots: Vec<_> = tree
            .children
            .iter()
            .map(|id| tree.items.get(*id).unwrap().name.clone())
            .collect();
        assert_eq!(roots, vec![ByteString::from("b"), ByteString::from("d")]);
        backend.squash_groups = false;
        tree.save();
        backend.build(&mut s, &mut tree, &SortCriteria::default(), &mut tag, None);
        assert_eq!(tree.children.len(), 4);
    }
    #[test]
    fn hidden_pane_still_matches_and_parents_use_parent_formats() {
        let mut s = server();
        let sid = session(&mut s, b"fixture");
        let (wl, pane) = window(&mut s, sid, 0, 1);
        s.panes
            .get_mut(pane)
            .unwrap()
            .base
            .set_title(b"title", false);
        let mut backend = backend(&s, sid, wl, pane);
        backend.hide_preview_this_pane = true;
        let mut tree = ModeTreeData::start(80, 24, ModeTreePreview::Off);
        let mut tag = ModeTreeTag::Unset;
        backend.build(
            &mut s,
            &mut tree,
            &SortCriteria::default(),
            &mut tag,
            Some(b"1"),
        );
        assert_eq!(tag, ModeTreeTag::Winlink(wl));
        assert_eq!(backend.items.len(), 2);
        let root = tree.items.get(tree.children[0]).unwrap();
        assert!(root.text.as_ref().unwrap().starts_with(b"1:0:0:"));
        let window = tree.items.get(root.children[0]).unwrap();
        assert_eq!(
            window.text.as_ref().map(|text| text.as_slice()),
            Some(b"0:1:0:title".as_slice())
        );
        assert!(window.children.is_empty());
        assert!(window.align);
        backend.prepare_draw(&mut s, Some(1), 78, 6);
        assert!(backend.cache.strip.is_none());
    }
    #[test]
    fn targets_prompts_search_and_swap_validate_live_objects() {
        let mut s = server();
        let sid = session(&mut s, b"Alpha");
        let (a, pane) = window(&mut s, sid, 0, 2);
        let (b, _) = window(&mut s, sid, 1, 1);
        let backend = backend(&s, sid, a, pane);
        let session_item = WindowTreeItem {
            kind: WindowTreeType::Session,
            session: sid,
            winlink: None,
            pane: None,
        };
        let window_item = WindowTreeItem {
            kind: WindowTreeType::Window,
            session: sid,
            winlink: Some(a),
            pane: None,
        };
        let other = WindowTreeItem {
            winlink: Some(b),
            ..window_item
        };
        let pane_item = WindowTreeItem {
            kind: WindowTreeType::Pane,
            pane: Some(pane),
            ..window_item
        };
        assert_eq!(session_item.target(&s).unwrap().0.as_ref(), b"=Alpha:");
        assert_eq!(window_item.target(&s).unwrap().0.as_ref(), b"=Alpha:0.");
        assert_eq!(
            pane_item.target(&s).unwrap().0.as_ref(),
            format!("=Alpha:0.%{}", s.panes.get(pane).unwrap().public_id).as_bytes()
        );
        assert_eq!(
            kill_message(&s, session_item).unwrap(),
            b"Kill session Alpha? "
        );
        assert_eq!(kill_message(&s, window_item).unwrap(), b"Kill window 0? ");
        assert_eq!(kill_message(&s, pane_item).unwrap(), b"Kill pane 0? ");
        assert_eq!(
            backend.search(&s, Some(&session_item), b"alpha", true),
            Some(true)
        );
        assert_eq!(
            backend.search(&s, Some(&session_item), b"alpha", false),
            Some(false)
        );
        assert_eq!(
            backend.search(&s, Some(&pane_item), b"sh", true),
            Some(false)
        );
        let sort = SortCriteria {
            order: crate::format::sort::SortOrder::Index,
            ..Default::default()
        };
        assert!(backend.can_swap(&s, &window_item, &other, &sort));
        assert!(!backend.can_swap(&s, &pane_item, &other, &sort));
        let wa = s.winlinks.get(a).unwrap().window;
        let wb = s.winlinks.get(b).unwrap().window;
        s.windows.get_mut(wa).unwrap().name = b"alpha".to_vec();
        s.windows.get_mut(wb).unwrap().name = b"beta".to_vec();
        let sort_name = SortCriteria {
            order: crate::format::sort::SortOrder::Name,
            ..sort
        };
        assert!(!backend.can_swap(&s, &window_item, &other, &sort_name));
        backend.swap(&window_item, &other, &sort)(&mut s);
        assert_eq!(s.winlinks.get(a).unwrap().window, wb);
        assert_eq!(s.winlinks.get(b).unwrap().window, wa);
        assert_eq!(s.sessions.get(sid).unwrap().current, Some(b));
        let stale = WindowTreeItem {
            pane: Some(pane),
            winlink: Some(a),
            ..pane_item
        };
        assert!(stale.target(&s).is_none());
    }
    #[test]
    fn info_groups_and_narrow_strip_boundaries_match_source() {
        let mut s = server();
        let sid = session(&mut s, b"fixture");
        let (wl, pane) = window(&mut s, sid, 0, 2);
        let mut backend = backend(&s, sid, wl, pane);
        backend.items.push(WindowTreeItem {
            kind: WindowTreeType::Pane,
            session: sid,
            winlink: Some(wl),
            pane: Some(pane),
        });
        backend.preview_is_info = true;
        backend.prepare_draw(&mut s, Some(0), 78, 20);
        assert_eq!(
            backend.cache.info.len(),
            PANE_INFO_LINES.len() + WINDOW_INFO_LINES.len() + SESSION_INFO_LINES.len() + 2
        );
        assert_eq!(
            backend
                .cache
                .info
                .iter()
                .filter(|line| line.is_none())
                .count(),
            2
        );
        assert!(
            backend.cache.info[0]
                .as_ref()
                .unwrap()
                .windows(4)
                .any(|part| part == b"Pane")
        );
        let strip = preview_strip(8, 4, 6, 0).unwrap();
        assert!(!strip.left && !strip.right);
        assert_eq!((strip.start, strip.end), (4, 5));
        let strip = preview_strip(8, 4, 7, 0).unwrap();
        assert!(strip.left && strip.right);
        assert_eq!(strip.each, 1);
        assert!(preview_strip(8, 0, 0, 0).is_none());
    }
    #[test]
    fn strips_clamp_offsets_and_distribute_remainder() {
        let strip = preview_strip(12, 6, 80, 100).unwrap();
        assert_eq!((strip.start, strip.end), (9, 12));
        assert!(strip.left);
        assert!(!strip.right);
        assert_eq!(strip.each, 25);
        assert_eq!(strip.remaining, 2);
        let strip = preview_strip(3, 1, 200, -100).unwrap();
        assert_eq!((strip.start, strip.end), (0, 3));
        assert_eq!(strip.each, 66);
        assert_eq!(strip.remaining, 2);
    }
    #[test]
    fn preview_mouse_selects_children_and_scroll_markers() {
        let mut s = server();
        let sid = session(&mut s, b"fixture");
        let (wl, pane) = window(&mut s, sid, 0, 4);
        let mut state = TreeModeState {
            tree: ModeTreeData::start(80, 30, ModeTreePreview::Normal),
            backend: backend(&s, sid, wl, pane),
        };
        state.build(&mut s);
        state.backend.kind = WindowTreeType::None;
        state.tree.set_current(ModeTreeTag::Winlink(wl));
        let current = state.tree.get_current().unwrap().item;
        state.backend.prepare_draw(&mut s, current, 78, 8);
        assert_eq!(preview_mouse(&s, &mut state, 76), KeyCode(62));
        state.backend.offset = 1;
        state.backend.prepare_draw(&mut s, current, 78, 8);
        assert_eq!(preview_mouse(&s, &mut state, 2), KeyCode(60));
        assert_eq!(preview_mouse(&s, &mut state, 4), KeyCode(13));
        let window = s.winlinks.get(wl).unwrap().window;
        assert_eq!(
            current_item(&state).unwrap().pane,
            Some(s.windows.get(window).unwrap().panes[1])
        );
        state.backend.key_format = b"#{?window_format,F2,F3}".as_slice().into();
        let index = state
            .backend
            .items
            .iter()
            .position(|i| i.kind == WindowTreeType::Window)
            .unwrap();
        assert_eq!(
            state.backend.key(&mut s, Some(index as u32), 1),
            Some(KeyCode(SpecialKey::F2))
        );
        assert!(contains(b"Alpha\0hidden", b"alpha", true));
        assert!(!contains(b"Alpha\0hidden", b"hidden", false));
    }
}
