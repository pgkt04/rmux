// Ported from tmux window-buffer.c @ 8f25579c
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

use crate::client::ResolvedMouseEvent;
use crate::cmd::arguments::Args;
use crate::cmd::find::CmdFindState;
use crate::format::sort::{self, SortCriteria, SortOrder};
use crate::format::{self, FormatContext, FormatFlags, FormatTree};
use crate::ids::{ClientId, EditorId, ModeId, PaneId, PasteBufferId};
use crate::model::pane::{PaneMode, PaneModeDriver, pane_reset_mode};
use crate::model::paste::{
    paste_buffer_data, paste_get_name, paste_is_empty, paste_remove, paste_replace,
};
use crate::model::spawn::{SpawnContext, spawn_cancel_editor, spawn_editor, spawn_get_editor_pid};
use crate::model::{ModelError, PaneFlags};
use crate::modes::tree::{
    self, ModeAction, ModeTreeCallbacks, ModeTreeData, ModeTreeMenuItem, ModeTreeTag,
    TreeModeState, run_command,
};
use crate::server::Server;
use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{BoxLines, Screen};
use rmux_util::bytes::ByteString;
use rmux_util::key::{C0, KeyCode, KeyModifiers};
use rmux_util::vis::VisFlags;
use std::cell::RefCell;

pub const NAME: &[u8] = b"buffer-mode";
pub const DEFAULT_COMMAND: &[u8] = b"paste-buffer -p -b '%%'";
pub const DEFAULT_FORMAT: &[u8] = b"#{t/p:buffer_created}: #{buffer_sample}";
pub const DEFAULT_KEY_FORMAT: &[u8] = b"#{?#{e|<:#{line},10},\
#{line}\
,#{e|<:#{line},36},\
M-#{a:#{e|+:97,#{e|-:#{line},10}}}\
}";

const NONE: u64 = rmux_util::key::SpecialKey::NONE;
pub const MENU_ITEMS: &[ModeTreeMenuItem] = &[
    ModeTreeMenuItem {
        name: "Paste",
        key: b'p' as u64,
    },
    ModeTreeMenuItem {
        name: "Paste Tagged",
        key: b'P' as u64,
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
        name: "Delete",
        key: b'd' as u64,
    },
    ModeTreeMenuItem {
        name: "Delete Tagged",
        key: b'D' as u64,
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

const ORDER_SEQ: &[SortOrder] = &[SortOrder::Creation, SortOrder::Name, SortOrder::Size];

pub const HELP_LINES: &[&str] = &[
    "#[fg=themelightgrey]      Enter #[#{E:tree-mode-border-style},acs]x#[default] Paste selected %1",
    "#[fg=themelightgrey]          p #[#{E:tree-mode-border-style},acs]x#[default] Paste selected %1",
    "#[fg=themelightgrey]          P #[#{E:tree-mode-border-style},acs]x#[default] Paste tagged %1s",
    "#[fg=themelightgrey]          d #[#{E:tree-mode-border-style},acs]x#[default] Delete selected %1",
    "#[fg=themelightgrey]          D #[#{E:tree-mode-border-style},acs]x#[default] Delete tagged %1s",
    "#[fg=themelightgrey]          e #[#{E:tree-mode-border-style},acs]x#[default] Open %1 in editor",
    "#[fg=themelightgrey]          f #[#{E:tree-mode-border-style},acs]x#[default] Enter a filter",
];

pub struct BufferItem {
    pub name: ByteString,
    pub order: u32,
    pub size: usize,
}

pub struct BufferBackend {
    pub wp: PaneId,
    pub fs: CmdFindState,
    pub editor: Option<EditorId>,
    pub command: Vec<u8>,
    pub format: Vec<u8>,
    pub key_format: Vec<u8>,
    pub items: Vec<BufferItem>,
    vis: RefCell<Vec<u8>>,
}

pub type BufferState = TreeModeState<BufferBackend>;

pub struct BufferMode {
    args: Args,
    fs: CmdFindState,
}

impl BufferMode {
    pub fn new(args: Args, fs: CmdFindState) -> Self {
        Self { args, fs }
    }
}

fn fs_context(server: &Server, fs: &CmdFindState) -> FormatContext {
    if fs.is_valid(server) {
        FormatContext {
            session: fs.s,
            winlink: fs.wl,
            window: fs.w,
            pane: fs.wp,
            ..FormatContext::default()
        }
    } else {
        FormatContext::default()
    }
}

/// `window_buffer_find`: byte search with optional `tolower` folding.
pub fn find(data: &[u8], needle: &[u8], icase: bool) -> bool {
    if needle.is_empty() || data.len() < needle.len() {
        return false;
    }
    data.windows(needle.len()).any(|w| {
        w.iter().zip(needle).all(|(&a, &b)| {
            if icase {
                tree::tolower(a) == tree::tolower(b)
            } else {
                a == b
            }
        })
    })
}

impl ModeTreeCallbacks for BufferBackend {
    type Item = BufferItem;

    fn build(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        sort: &SortCriteria,
        _tag: &mut ModeTreeTag,
        filter: Option<&[u8]>,
    ) {
        self.items.clear();
        let mut buffers: Vec<PasteBufferId> = Vec::new();
        sort::get_buffers(&*s, sort, &mut buffers);
        for id in buffers {
            let Some(pb) = s.paste.get(id) else {
                continue;
            };
            self.items.push(BufferItem {
                name: pb.name.clone(),
                order: pb.order,
                size: pb.data.len(),
            });
        }
        let context = fs_context(s, &self.fs);
        for i in 0..self.items.len() {
            let Some(pb) = paste_get_name(s, &self.items[i].name) else {
                continue;
            };
            let mut ft = FormatTree::create(None, None, 0, FormatFlags::NONE, s);
            ft.defaults(s, context);
            ft.defaults_paste_buffer(pb);
            if let Some(filter) = filter {
                let cp = ft.expand(s, filter);
                if !format::true_value(Some(&cp)) {
                    ft.release(s);
                    continue;
                }
            }
            let text = ft.expand(s, &self.format);
            ft.release(s);
            let item = &self.items[i];
            tree.add(
                None,
                Some(i as u32),
                ModeTreeTag::BufferOrder(item.order),
                &item.name,
                Some(&text),
                -1,
            );
        }
    }

    fn prepare_draw(&mut self, _s: &mut Server, _item: Option<u32>, _sx: u32, _sy: u32) {}

    fn draw(
        &self,
        s: &Server,
        item: Option<&BufferItem>,
        ctx: &mut ScreenWriteCtx<'_>,
        sx: u32,
        sy: u32,
    ) {
        let Some(item) = item else {
            return;
        };
        let Some(pdata) = paste_get_name(s, &item.name).and_then(|pb| paste_buffer_data(s, pb))
        else {
            return;
        };
        let cx = ctx.screen.cx;
        let cy = ctx.screen.cy;
        let mut buf = self.vis.borrow_mut();
        let mut rest = pdata;
        for i in 0..sy {
            let end = rest.iter().position(|&b| b == b'\n');
            let line = match end {
                Some(n) => &rest[..n],
                None => rest,
            };
            buf.clear();
            rmux_util::utf8::strvis(
                &mut buf,
                line,
                VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB,
            );
            if !buf.is_empty() {
                ctx.cursormove(cx as i32, (cy + i) as i32, false);
                ctx.nputs(sx as isize, &DEFAULT_CELL, &buf);
            }
            match end {
                None => break,
                Some(n) => rest = &rest[n + 1..],
            }
        }
    }

    fn has_draw(&self) -> bool {
        true
    }

    fn search(
        &self,
        s: &Server,
        item: Option<&BufferItem>,
        needle: &[u8],
        icase: bool,
    ) -> Option<bool> {
        let item = item?;
        let pb = paste_get_name(s, &item.name)?;
        if find(&item.name, needle, icase) {
            return Some(true);
        }
        let data = paste_buffer_data(s, pb)?;
        Some(find(data, needle, icase))
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
        let pb = paste_get_name(s, &item.name)?;
        let context = fs_context(s, &self.fs);
        let mut ft = FormatTree::create(None, None, 0, FormatFlags::NONE, s);
        ft.defaults(s, FormatContext::default());
        ft.defaults(s, context);
        ft.defaults_paste_buffer(pb);
        ft.add(b"line", ByteString::from(line.to_string()));
        let expanded = ft.expand(s, &self.key_format);
        ft.release(s);
        Some(rmux_tty::key_string::parse_key_name(&expanded))
    }

    fn swap(&self, _cur: &BufferItem, _other: &BufferItem, _sort: &SortCriteria) -> ModeAction {
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
        Some((0, "buffer", HELP_LINES))
    }

    fn items(&self) -> &[BufferItem] {
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

fn take_state(server: &mut Server, id: ModeId) -> Option<Box<BufferState>> {
    let mode = mode_mut(server, id)?;
    let data = mode.data.take()?;
    match data.downcast::<BufferState>() {
        Ok(state) => Some(state),
        Err(other) => {
            mode.data = Some(other);
            None
        }
    }
}

fn restore_state(server: &mut Server, id: ModeId, state: Box<BufferState>) {
    match mode_mut(server, id) {
        Some(mode) => mode.data = Some(state),
        None => state.free(server),
    }
}

/// Run `f` with the state and the mode screen taken out of the pane.
fn with_state(
    server: &mut Server,
    id: ModeId,
    f: impl FnOnce(&mut Server, &mut BufferState, &mut Screen),
) {
    let Some(mut state) = take_state(server, id) else {
        return;
    };
    let Some(mut screen) = mode_mut(server, id).and_then(|m| m.screen.take()) else {
        restore_state(server, id, state);
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
            state.free(server);
        }
    }
}

/// `window_buffer_draw_waiting`: the editor box in the centre of the screen.
pub fn draw_waiting(server: &mut Server, editor: Option<EditorId>, screen: &mut Screen) {
    let Some(editor) = editor else {
        return;
    };
    let sx = screen.grid.sx();
    let sy = screen.grid.sy();
    if sx == 0 || sy == 0 {
        return;
    }
    let text = match spawn_get_editor_pid(server, Some(editor)) {
        None => b"WAITING FOR EDITOR".to_vec(),
        Some(pid) => format!("WAITING FOR EDITOR (PID {})", pid.0).into_bytes(),
    };
    let textlen = text.len() as u32;
    let box_w = textlen + 4;
    let box_h = 3;
    if sx < box_w || sy < box_h {
        return;
    }
    let x = (sx - box_w) / 2;
    let y = (sy - box_h) / 2;
    let text_x = x + (box_w - textlen) / 2;
    let gc: GridCell = DEFAULT_CELL;
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut server.hyperlinks,
        #[cfg(feature = "sixel")]
        None,
    );
    ctx.cursormove(x as i32, y as i32, false);
    ctx.draw_box(box_w, box_h, BoxLines::Default, Some(&gc));
    ctx.cursormove((x + 1) as i32, (y + 1) as i32, false);
    ctx.clearcharacter(box_w - 2, gc.bg);
    ctx.cursormove(text_x as i32, (y + 1) as i32, false);
    ctx.nputs((box_w - 2) as isize, &gc, &text);
    ctx.finish();
}

fn redraw(server: &mut Server, state: &mut BufferState, screen: &mut Screen) {
    state.draw(server, screen);
    draw_waiting(server, state.backend.editor, screen);
    if let Some(p) = server.panes.get_mut(state.backend.wp) {
        p.flags.insert(PaneFlags::REDRAW);
    }
}

/// `window_buffer_do_delete`.
fn do_delete(server: &mut Server, state: &mut BufferState, item: u32) {
    if state.tree.get_current().and_then(|i| i.item) == Some(item) && !state.tree.down(false) {
        state.tree.up(false);
    }
    let Some(name) = state
        .backend
        .items
        .get(item as usize)
        .map(|i| i.name.clone())
    else {
        return;
    };
    if let Some(pb) = paste_get_name(server, &name) {
        let _ = paste_remove(server, pb);
    }
}

/// `window_buffer_do_paste`.
fn do_paste(server: &mut Server, state: &BufferState, item: u32, c: Option<ClientId>) {
    let Some(name) = state
        .backend
        .items
        .get(item as usize)
        .map(|i| i.name.clone())
    else {
        return;
    };
    if paste_get_name(server, &name).is_some() {
        run_command(server, c, None, &state.backend.command, &name);
    }
}

/// `window_buffer_edit_close_cb`.
fn edit_close(
    server: &mut Server,
    wp: PaneId,
    name: ByteString,
    original: PasteBufferId,
    editor: EditorId,
    buf: Option<Vec<u8>>,
) {
    if let Some(state) = take_state_if_first(server, wp) {
        let (id, mut state) = state;
        if state.backend.editor == Some(editor) {
            state.backend.editor = None;
        }
        restore_state(server, id, state);
    }
    let Some(mut buf) = buf.filter(|b| !b.is_empty()) else {
        return;
    };
    let Some(pb) = paste_get_name(server, &name) else {
        return;
    };
    if pb != original {
        return;
    }
    let Some(old) = paste_buffer_data(server, pb) else {
        return;
    };
    if !old.is_empty() && old[old.len() - 1] != b'\n' && buf[buf.len() - 1] == b'\n' {
        buf.pop();
    }
    if !buf.is_empty() {
        let _ = paste_replace(server, pb, buf);
    }
    if let Some((id, mut state)) = take_state_if_first(server, wp) {
        if let Some(mut screen) = mode_mut(server, id).and_then(|m| m.screen.take()) {
            state.build(server);
            state.draw(server, &mut screen);
            draw_waiting(server, state.backend.editor, &mut screen);
            if let Some(mode) = mode_mut(server, id) {
                mode.screen = Some(screen);
            } else {
                let _ = screen.release(
                    &mut server.hyperlinks,
                    #[cfg(feature = "sixel")]
                    None,
                );
            }
        }
        restore_state(server, id, state);
    }
    if let Some(p) = server.panes.get_mut(wp) {
        p.flags.insert(PaneFlags::REDRAW);
    }
}

/// The first mode of `wp` when it is a buffer mode, taken out of the pane.
fn take_state_if_first(server: &mut Server, wp: PaneId) -> Option<(ModeId, Box<BufferState>)> {
    let first = server.panes.get(wp)?.modes.first()?;
    if first.name != NAME {
        return None;
    }
    let id = first.id;
    take_state(server, id).map(|state| (id, state))
}

/// `window_buffer_start_edit`.
fn start_edit(server: &mut Server, id: ModeId, name: ByteString, c: Option<ClientId>) {
    let Some(state) = take_state(server, id) else {
        return;
    };
    let editing = state.backend.editor.is_some();
    restore_state(server, id, state);
    if editing {
        return;
    }
    let Some(pb) = paste_get_name(server, &name) else {
        return;
    };
    let Some(buf) = paste_buffer_data(server, pb).map(<[u8]>::to_vec) else {
        return;
    };
    let Some(client) = c.and_then(|c| server.clients.get(c)) else {
        return;
    };
    let Some(session) = client.session else {
        return;
    };
    let mut context = SpawnContext::new(session);
    context.client = c;
    context.client_environment = Some(client.environ.clone());
    context.client_cwd = client.cwd.clone();
    context.client_attached = true;
    let wp = id.owner;
    let callback_name = name.clone();
    let callback = Box::new(
        move |server: &mut Server, editor: EditorId, buf: Option<Vec<u8>>| {
            edit_close(server, wp, callback_name, pb, editor, buf);
        },
    );
    if let Ok(editor) = spawn_editor(server, &context, &buf, callback) {
        if let Some(mut state) = take_state(server, id) {
            state.backend.editor = Some(editor);
            restore_state(server, id, state);
            with_state(server, id, redraw);
        } else {
            spawn_cancel_editor(server, editor);
        }
    }
}

fn editor_key_exits(key: KeyCode) -> bool {
    key.0 == u64::from(b'q')
        || key.0 == u64::from(C0::ESC)
        || key.0 == 3
        || key.0 == (u64::from(b'c') | KeyModifiers::CTRL.0)
}

/// `window_buffer_key` for a mode entry; `c` may be absent from a menu.
fn key_impl(
    server: &mut Server,
    id: ModeId,
    c: Option<ClientId>,
    key: KeyCode,
    m: Option<&ResolvedMouseEvent>,
) {
    let wp = id.owner;
    let mut finished = false;
    let mut edit = None;
    with_state(server, id, |server, state, screen| {
        if paste_is_empty(server) {
            finished = true;
            return;
        }
        if state.backend.editor.is_some() {
            finished = editor_key_exits(key);
            if !finished {
                redraw(server, state, screen);
            }
            return;
        }
        let result = state.key(server, id, c, key, m, screen);
        finished = result.finished;
        let key = result.key.0;
        let current = state.tree.get_current().and_then(|i| i.item);
        match key {
            k if k == u64::from(b'e') => {
                if let Some(item) = current.and_then(|n| state.backend.items.get(n as usize)) {
                    edit = Some(item.name.clone());
                }
            }
            k if k == u64::from(b'd') => {
                if let Some(item) = current {
                    do_delete(server, state, item);
                }
                state.build(server);
            }
            k if k == u64::from(b'D') => {
                for item in state.tree.tagged_items(false) {
                    do_delete(server, state, item);
                }
                state.build(server);
            }
            k if k == u64::from(b'P') => {
                for item in state.tree.tagged_items(false) {
                    do_paste(server, state, item, c);
                }
                finished = true;
            }
            k if k == u64::from(b'p') || k == u64::from(b'\r') => {
                if let Some(item) = current {
                    do_paste(server, state, item, c);
                }
                finished = true;
            }
            _ => {}
        }
        if !finished && !paste_is_empty(server) {
            redraw(server, state, screen);
        }
    });
    if let Some(name) = edit {
        start_edit(server, id, name, c);
    }
    if finished || paste_is_empty(server) {
        let _ = pane_reset_mode(server, wp);
    }
}

impl PaneModeDriver for BufferMode {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
        let wp = id.owner;
        let (sx, sy) = {
            let p = server.panes.get(wp)?;
            (p.base.grid.sx(), p.base.grid.sy())
        };
        let args = &self.args;
        let backend = BufferBackend {
            wp,
            fs: self.fs,
            editor: None,
            command: if args.count() == 0 {
                DEFAULT_COMMAND.to_vec()
            } else {
                args.string(0).unwrap_or(DEFAULT_COMMAND).to_vec()
            },
            format: args.get(b'F').unwrap_or(DEFAULT_FORMAT).to_vec(),
            key_format: args.get(b'K').unwrap_or(DEFAULT_KEY_FORMAT).to_vec(),
            items: Vec::new(),
            vis: RefCell::new(Vec::new()),
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
            rmux_emu::screen::ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .ok()?;
        screen.mode.remove(rmux_emu::screen::ScreenMode::CURSOR);
        let (mut state, mut screen) =
            super::tree::init_zoom(server, id, state, screen, Some(args))?;
        state.build(server);
        state.draw(server, &mut screen);
        match mode_mut(server, id) {
            Some(mode) => mode.data = Some(Box::new(state)),
            None => {
                state.free(server);
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
            .and_then(|d| d.downcast::<BufferState>().ok())
        {
            if let Some(editor) = state.backend.editor {
                spawn_cancel_editor(server, editor);
            }
            state.free(server);
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
        Err(ModelError::Message(b"buffer-mode has no output".to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::commands::break_pane::tests::{create_session, create_window, source};
    use crate::ids::ArenaId;
    use crate::model::pane::pane_set_mode;
    use crate::model::paste::paste_set;
    use crate::modes::WindowModeFlags;
    use rmux_emu::screen::ScreenResetPolicy;
    use std::rc::Rc;

    #[test]
    fn editor_control_keys_exit_without_accepting_other_tree_keys() {
        for key in [b"q".as_slice(), b"Escape", b"C-c"] {
            assert!(editor_key_exits(rmux_tty::key_string::parse_key_name(key)));
        }
        assert!(editor_key_exits(KeyCode(3)));
        for key in [b"C-g".as_slice(), b"C-t", b"M-c", b"d", b"Enter"] {
            assert!(!editor_key_exits(rmux_tty::key_string::parse_key_name(key)));
        }
    }

    #[test]
    fn preview_escapes_binary_lines_and_preserves_utf8() {
        let mut server = Server::default();
        let bytes = b"a\t\0\x01\\\n\n\xc3\xa9\nend";
        let pb = paste_set(&mut server, bytes.to_vec(), Some(b"preview"), 50)
            .unwrap()
            .unwrap();
        let backend = BufferBackend {
            wp: PaneId::from_parts(0, 0),
            fs: CmdFindState::default(),
            editor: None,
            command: Vec::new(),
            format: Vec::new(),
            key_format: Vec::new(),
            items: Vec::new(),
            vis: RefCell::new(Vec::new()),
        };
        let item = BufferItem {
            name: b"preview".as_slice().into(),
            order: server.paste.get(pb).unwrap().order,
            size: bytes.len(),
        };
        let mut registry = rmux_emu::hyperlinks::HyperlinkRegistry::new();
        let mut screen =
            Screen::new(30, 7, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.cursormove(2, 1, false);
        backend.draw(&server, Some(&item), &mut ctx, 20, 5);
        ctx.finish();
        let row: Vec<_> = (2..15)
            .map(|x| screen.grid.view_get_cell(x, 1).data.data[0])
            .collect();
        assert_eq!(row, b"a\\t\\0\\001\\\\  ");
        assert_eq!(screen.grid.view_get_cell(2, 2).data.data[0], b' ');
        assert_eq!(screen.grid.view_get_cell(2, 3).data.bytes(), b"\xc3\xa9");
        assert_eq!(screen.grid.view_get_cell(2, 4).data.data[0], b'e');
        assert!(!backend.vis.borrow().is_empty());
        screen
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
    }

    #[test]
    fn editor_completion_rejects_empty_and_replaced_buffers() {
        let mut server = Server::default();
        let wp = PaneId::from_parts(0, 0);
        let editor = EditorId::from_parts(0, 0);
        let original = paste_set(&mut server, b"old".to_vec(), Some(b"edit"), 50)
            .unwrap()
            .unwrap();
        for output in [None, Some(Vec::new()), Some(b"\n".to_vec())] {
            edit_close(
                &mut server,
                wp,
                b"edit".as_slice().into(),
                original,
                editor,
                output,
            );
            assert_eq!(
                paste_buffer_data(&server, original),
                Some(b"old".as_slice())
            );
        }
        edit_close(
            &mut server,
            wp,
            b"edit".as_slice().into(),
            original,
            editor,
            Some(b"new\n".to_vec()),
        );
        assert_eq!(
            paste_buffer_data(&server, original),
            Some(b"new".as_slice())
        );
        paste_replace(&mut server, original, b"old\n".to_vec()).unwrap();
        edit_close(
            &mut server,
            wp,
            b"edit".as_slice().into(),
            original,
            editor,
            Some(b"\n".to_vec()),
        );
        assert_eq!(paste_buffer_data(&server, original), Some(b"\n".as_slice()));
        let replacement = paste_set(&mut server, b"replacement".to_vec(), Some(b"edit"), 50)
            .unwrap()
            .unwrap();
        assert_ne!(original, replacement);
        edit_close(
            &mut server,
            wp,
            b"edit".as_slice().into(),
            original,
            editor,
            Some(b"stale".to_vec()),
        );
        assert_eq!(
            paste_buffer_data(&server, replacement),
            Some(b"replacement".as_slice())
        );
    }

    struct StringOnlyParser;

    impl crate::cmd::parse::CommandParser for StringOnlyParser {
        fn parse_from_string(&mut self, _: &[u8]) -> crate::cmd::parse::CmdParseResult {
            panic!("string option must not parse commands")
        }
    }

    #[test]
    fn editor_launch_from_zoomed_mode_preserves_state_and_cancellation() {
        let mut server = Server::default();
        let session = create_session(&mut server, b"edit");
        let (window, wl, wp) = create_window(&mut server, session, 0);
        crate::cmd::commands::break_pane::tests::split(&mut server, window, wp);
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = Some(session);
        let client = server.clients.insert(client).unwrap();
        paste_set(&mut server, b"old".to_vec(), Some(b"edit"), 50).unwrap();
        let global = server.options.global;
        server
            .options
            .set_string(global, b"editor", false, b"true", &mut StringOnlyParser);
        assert!(crate::model::window::window_zoom(&mut server, window, wp).unwrap());
        let fs = source(&server, wl, wp);
        let id = pane_set_mode(
            &mut server,
            wp,
            NAME,
            WindowModeFlags::default(),
            Rc::new(BufferMode::new(Args::create(), fs)),
            false,
        )
        .unwrap()
        .unwrap();
        let metadata = ResolvedMouseEvent {
            event: Default::default(),
            target: Default::default(),
        };
        key_impl(
            &mut server,
            id,
            Some(client),
            KeyCode(u64::from(b'e')),
            Some(&metadata),
        );
        let state = take_state(&mut server, id).unwrap();
        let editor = state.backend.editor.expect("buffer editor opened");
        let screen_size = {
            let screen = mode_mut(&mut server, id).unwrap().screen.as_ref().unwrap();
            (screen.grid.sx(), screen.grid.sy())
        };
        let pane = server.panes.get(wp).unwrap();
        assert_eq!(screen_size, (pane.sx, pane.sy));
        restore_state(&mut server, id, state);
        let edit = server.editors.get(editor).unwrap();
        let (editor_pane, pid) = (edit.pane, edit.pid);
        assert_eq!(server.windows.get(window).unwrap().modal, Some(editor_pane));
        assert_eq!(
            server.windows.get(window).unwrap().active,
            Some(editor_pane)
        );
        key_impl(
            &mut server,
            id,
            Some(client),
            rmux_tty::key_string::parse_key_name(b"C-c"),
            None,
        );
        assert!(server.panes.get(wp).unwrap().modes.is_empty());
        assert!(server.editors.get(editor).unwrap().callback.is_none());
        let status = rmux_sys::proc::wait_process(pid, false).unwrap().unwrap();
        let pane = server.panes.get_mut(editor_pane).unwrap();
        pane.status = status;
        pane.flags.insert(PaneFlags::STATUSREADY);
        crate::model::spawn::spawn_editor_finish(&mut server, editor_pane);
        assert!(server.editors.get(editor).is_none());
    }

    #[test]
    fn completion_only_clears_first_matching_editor_marker_after_stack_changes() {
        let mut server = Server::default();
        let session = create_session(&mut server, b"edit-stack");
        let (_, wl, wp) = create_window(&mut server, session, 0);
        let pb = paste_set(&mut server, b"old".to_vec(), Some(b"edit"), 50)
            .unwrap()
            .unwrap();
        let fs = source(&server, wl, wp);
        let id = pane_set_mode(
            &mut server,
            wp,
            NAME,
            WindowModeFlags::default(),
            Rc::new(BufferMode::new(Args::create(), fs)),
            false,
        )
        .unwrap()
        .unwrap();
        let editor = EditorId::from_parts(10, 0);
        let other = EditorId::from_parts(11, 0);
        let mut state = take_state(&mut server, id).unwrap();
        state.backend.editor = Some(editor);
        restore_state(&mut server, id, state);
        edit_close(&mut server, wp, b"edit".as_slice().into(), pb, other, None);
        let state = take_state(&mut server, id).unwrap();
        assert_eq!(state.backend.editor, Some(editor));
        restore_state(&mut server, id, state);
        let overlay = pane_set_mode(
            &mut server,
            wp,
            crate::modes::clock::NAME,
            WindowModeFlags::default(),
            Rc::new(crate::modes::clock::ClockMode),
            false,
        )
        .unwrap()
        .unwrap();
        edit_close(
            &mut server,
            wp,
            b"edit".as_slice().into(),
            pb,
            editor,
            Some(b"new\n".to_vec()),
        );
        assert_eq!(paste_buffer_data(&server, pb), Some(b"new".as_slice()));
        let state = take_state(&mut server, id).unwrap();
        assert_eq!(state.backend.editor, Some(editor));
        restore_state(&mut server, id, state);
        assert_eq!(server.panes.get(wp).unwrap().modes[0].id, overlay);
        pane_reset_mode(&mut server, wp).unwrap();
        edit_close(&mut server, wp, b"edit".as_slice().into(), pb, editor, None);
        let state = take_state(&mut server, id).unwrap();
        assert!(state.backend.editor.is_none());
        restore_state(&mut server, id, state);
        pane_reset_mode(&mut server, wp).unwrap();
    }

    #[test]
    fn keyboard_context_mouse_pointer_does_not_consume_keys() {
        let mut server = Server::default();
        let session = create_session(&mut server, b"key-context");
        let (_, wl, wp) = create_window(&mut server, session, 0);
        for name in [b"one".as_slice(), b"two"] {
            paste_set(&mut server, name.to_vec(), Some(name), 50).unwrap();
        }
        let fs = source(&server, wl, wp);
        let id = pane_set_mode(
            &mut server,
            wp,
            NAME,
            WindowModeFlags::default(),
            Rc::new(BufferMode::new(Args::create(), fs)),
            false,
        )
        .unwrap()
        .unwrap();
        let metadata = ResolvedMouseEvent {
            event: Default::default(),
            target: Default::default(),
        };
        with_state(&mut server, id, |server, state, screen| {
            let edit = KeyCode(u64::from(b'e'));
            let result = state.key(server, id, None, edit, Some(&metadata), screen);
            assert_eq!(result.key, edit);
            assert!(!result.finished);
            let help = rmux_tty::key_string::parse_key_name(b"F1");
            state.key(server, id, None, help, Some(&metadata), screen);
            assert!(state.tree.help);
            let quit = KeyCode(u64::from(b'q'));
            let result = state.key(server, id, None, quit, Some(&metadata), screen);
            assert!(!state.tree.help);
            assert!(!result.finished, "q only closes help");
            let tag = rmux_tty::key_string::parse_key_name(b"C-t");
            let result = state.key(server, id, None, tag, Some(&metadata), screen);
            assert_eq!(result.key, tag);
            assert_eq!(state.tree.tagged_items(false).len(), 2);
            let result = state.key(server, id, None, quit, Some(&metadata), screen);
            assert!(result.finished, "q exits outside help");
        });
        pane_reset_mode(&mut server, wp).unwrap();
    }

    #[test]
    fn byte_find_with_case_folding() {
        assert!(find(b"Hello World", b"world", true));
        assert!(!find(b"Hello World", b"world", false));
        assert!(find(b"Hello World", b"World", false));
        assert!(!find(b"abc", b"", true));
        assert!(!find(b"ab", b"abc", true));
        assert!(find(b"\x00bin\x00", b"bin", false));
    }

    #[test]
    fn help_and_menu_literals() {
        assert_eq!(HELP_LINES.len(), 7);
        assert!(HELP_LINES[0].ends_with("Paste selected %1"));
        assert_eq!((MENU_ITEMS[4].name, MENU_ITEMS[4].key), ("Tag All", 0o24));
        assert_eq!(DEFAULT_FORMAT, b"#{t/p:buffer_created}: #{buffer_sample}");
    }
}
