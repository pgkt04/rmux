// Ported from tmux spawn.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SpawnFlags(pub u32);
impl SpawnFlags {
    pub const KILL: Self = Self(1);
    pub const DETACHED: Self = Self(2);
    pub const RESPAWN: Self = Self(4);
    pub const BEFORE: Self = Self(8);
    pub const NONOTIFY: Self = Self(16);
    pub const FULLSIZE: Self = Self(32);
    pub const EMPTY: Self = Self(64);
    pub const ZOOM: Self = Self(128);
    pub const FLOATING: Self = Self(256);
    pub const HORIZONTAL: Self = Self(512);
    pub const SPLIT: Self = Self(1024);
    pub const MODAL: Self = Self(2048);
    pub const FLOATOVERZOOM: Self = Self(4096);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for SpawnFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for SpawnFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for SpawnFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use super::{ModelEffect, ModelError, PaneFlags, Server, WindowFlags};
use crate::ids::{
    ClientId, EditorId, LayoutCellId, PaneId, QueueItemId, SessionId, WindowId, WinlinkId,
};
use crate::options::environment::{
    Environment, EnvironmentFlags, SessionEnvironmentContext, environ_for_session,
};
use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;

pub struct SpawnContext {
    pub session: SessionId,
    pub winlink: Option<WinlinkId>,
    pub pane: Option<PaneId>,
    pub layout_cell: Option<LayoutCellId>,
    pub item: Option<QueueItemId>,
    pub client: Option<ClientId>,
    pub format_target: Option<SessionId>,
    pub client_cwd: Option<Vec<u8>>,
    pub client_environment: Option<Environment>,
    pub client_attached: bool,
    pub initial_size: Option<super::resize::WindowSize>,
    pub argv: Vec<Vec<u8>>,
    pub environment: Environment,
    pub name: Option<Vec<u8>>,
    pub cwd: Option<Vec<u8>>,
    pub index: i32,
    pub flags: SpawnFlags,
}

impl SpawnContext {
    pub fn new(session: SessionId) -> Self {
        Self {
            session,
            winlink: None,
            pane: None,
            layout_cell: None,
            item: None,
            client: None,
            format_target: None,
            client_cwd: None,
            client_environment: None,
            client_attached: false,
            initial_size: None,
            argv: Vec::new(),
            environment: Environment::new(),
            name: None,
            cwd: None,
            index: -1,
            flags: SpawnFlags::default(),
        }
    }
}

pub enum SpawnEffect {
    PaneCreated {
        session: SessionId,
        winlink: WinlinkId,
        window: WindowId,
        pane: PaneId,
        window_index: i32,
        command: Vec<u8>,
        cwd: Vec<u8>,
        empty: bool,
        respawn: bool,
    },
}

fn spawn_error(prefix: &[u8], error: std::io::Error) -> ModelError {
    let mut message = prefix.to_vec();
    message.extend_from_slice(&rmux_sys::strerror(error.raw_os_error().unwrap_or(22)));
    ModelError::Message(message)
}

fn resolve_cwd(server: &mut Server, sc: &SpawnContext) -> Result<Option<Vec<u8>>, ModelError> {
    let target = sc.format_target.unwrap_or(sc.session);
    let source = sc
        .client_cwd
        .as_deref()
        .unwrap_or(&server.sessions.get(target).ok_or(ModelError::StaleId)?.cwd)
        .to_vec();
    let Some(explicit) = &sc.cwd else {
        return Ok((!sc.flags.contains(SpawnFlags::RESPAWN)).then_some(source));
    };
    let cwd = if sc.item.is_some() {
        let state = crate::cmd::find::CmdFindState {
            s: Some(target),
            ..Default::default()
        };
        crate::format::runtime::single_from_state(server, sc.item, sc.client, &state, explicit)
            .to_vec()
    } else {
        explicit.clone()
    };
    if cwd.starts_with(b"/") {
        return Ok(Some(cwd));
    }
    let mut joined = source;
    if !cwd.is_empty() {
        joined.push(b'/');
    }
    joined.extend_from_slice(&cwd);
    Ok(Some(joined))
}

fn launch_policy(
    server: &mut Server,
    sc: &SpawnContext,
    pane: PaneId,
) -> Result<LaunchOptions, ModelError> {
    let session = server.sessions.get(sc.session).ok_or(ModelError::StaleId)?;
    let options = session.options;
    let mut environment = environ_for_session(
        &server.global_environment,
        Some(&session.environment),
        SessionEnvironmentContext {
            default_terminal: server
                .options
                .get_string(server.options.global, b"default-terminal"),
            socket_path: &server.socket_path,
            pid: i64::from(rmux_sys::proc::getpid().0),
            session_id: Some(session.public_id),
        },
        false,
    );
    sc.environment.copy_into(&mut environment);
    let none = EnvironmentFlags::default();
    environment.set(b"TERM_PROGRAM", none, b"rmux");
    environment.set(
        b"TERM_PROGRAM_VERSION",
        none,
        crate::options::environment::RMUX_VERSION,
    );
    if server.tsp_broker_enabled {
        environment.set(b"RMUX_TSP", none, b"1");
        // Released omp never probes TSP under a tmux `TERM`; its own override
        // asks it to, and the broker answers. An explicit value wins.
        if environment.find(b"PI_TUI_NATIVE").is_none() {
            environment.set(b"PI_TUI_NATIVE", none, b"1");
        }
    } else {
        environment.unset(b"RMUX_TSP");
    }
    let p = server.panes.get_mut(pane).ok_or(ModelError::StaleId)?;
    if !sc.argv.is_empty() {
        p.argv.clone_from(&sc.argv);
    } else if !sc.flags.contains(SpawnFlags::RESPAWN) {
        let command = server.options.get_string(options, b"default-command");
        if !command.is_empty() {
            p.argv = vec![command.to_vec()];
        }
    }
    if !sc.flags.contains(SpawnFlags::RESPAWN) {
        let shell = server.options.get_string(options, b"default-shell");
        p.shell = if rmux_util::shell::check_shell(shell, b"rmux") {
            shell.to_vec()
        } else {
            b"/bin/sh".to_vec()
        };
    }
    environment.set(b"RMUX_PANE", none, format!("%{}", p.public_id).as_bytes());
    if !sc.client_attached {
        if let Some(entry) = sc.client_environment.as_ref().and_then(|e| e.find(b"PATH")) {
            if let Some(path) = &entry.value {
                environment.set(b"PATH", none, path);
            } else {
                environment.clear(b"PATH");
            }
        }
    }
    if environment.find(b"PATH").is_none() {
        environment.set(b"PATH", none, rmux_sys::pty::default_search_path());
    }
    environment.set(b"SHELL", none, &p.shell);
    let w = server.windows.get(p.window).ok_or(ModelError::StaleId)?;
    let home = server
        .global_environment
        .find(b"HOME")
        .and_then(|e| e.value.as_ref())
        .map(|v| v.to_vec())
        .or_else(|| std::env::var_os("HOME").map(|v| v.as_os_str().as_bytes().to_vec()));
    Ok(LaunchOptions {
        shell: p.shell.clone(),
        argv: p.argv.clone(),
        environment: environment
            .to_envp()
            .into_iter()
            .map(|v| v.to_vec())
            .collect(),
        cwd: p.cwd.clone(),
        home,
        termios: session.termios,
        backspace: server
            .options
            .get_number(server.options.global, b"backspace") as u64,
        size: Winsize {
            cols: p.base.grid.sx() as u16,
            rows: p.base.grid.sy() as u16,
            xpixel: w.xpixel.wrapping_mul(p.base.grid.sx()) as u16,
            ypixel: w.ypixel.wrapping_mul(p.base.grid.sy()) as u16,
        },
    })
}

pub fn spawn_pane(server: &mut Server, sc: &mut SpawnContext) -> Result<PaneId, ModelError> {
    let link = sc.winlink.ok_or(ModelError::StaleId)?;
    let (window, index) = {
        let link = server.winlinks.get(link).ok_or(ModelError::StaleId)?;
        (link.window, link.index)
    };
    let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
    if sc.flags.contains(SpawnFlags::MODAL) {
        if !sc.flags.contains(SpawnFlags::FLOATING) {
            return Err(ModelError::message(b"modal pane must be floating"));
        }
        if w.modal.is_some() {
            return Err(ModelError::message(b"window already has a modal pane"));
        }
    }
    let cwd = resolve_cwd(server, sc)?;
    let respawn = sc.flags.contains(SpawnFlags::RESPAWN);
    let pane = if respawn {
        let pane = sc.pane.ok_or(ModelError::StaleId)?;
        let p = server.panes.get(pane).ok_or(ModelError::StaleId)?;
        if p.fd.is_some() && !sc.flags.contains(SpawnFlags::KILL) {
            let name = &server
                .sessions
                .get(sc.session)
                .ok_or(ModelError::StaleId)?
                .name;
            let position = server
                .windows
                .get(window)
                .ok_or(ModelError::StaleId)?
                .panes
                .iter()
                .position(|&id| id == pane)
                .ok_or(ModelError::StaleId)?;
            let base = server.options.get_number(
                server.windows.get(window).unwrap().options,
                b"pane-base-index",
            );
            let mut message = b"pane ".to_vec();
            message.extend_from_slice(name);
            message.extend_from_slice(
                format!(":{index}.{} still active", position as i64 + base).as_bytes(),
            );
            return Err(ModelError::Message(message));
        }
        crate::tsp::broker::pane_respawn(server, pane);
        super::pane_input::pane_reset_io(server, pane)?;
        super::pane::pane_reset_mode_all(server, pane)?;
        let reset = rmux_emu::screen::ScreenResetPolicy {
            extended_keys: server
                .options
                .get_number(server.options.global, b"extended-keys")
                == 2,
        };
        let p = server.panes.get_mut(pane).ok_or(ModelError::StaleId)?;
        p.base
            .reinit(
                false,
                reset,
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                Some(&mut server.images),
            )
            .map_err(|e| ModelError::Sys(e.to_string()))?;
        p.flags
            .remove(PaneFlags::STATUSREADY | PaneFlags::STATUSDRAWN);
        pane
    } else {
        let session = server.sessions.get(sc.session).ok_or(ModelError::StaleId)?;
        let hlimit = server.options.get_number(session.options, b"history-limit") as u32;
        let pane = super::window::window_add_pane(server, window, sc.pane, hlimit, sc.flags)?;
        if let Some(cell) = sc.layout_cell {
            crate::layout::assign_pane(server, cell, pane, sc.flags.contains(SpawnFlags::ZOOM));
        } else {
            crate::layout::init(server, window, pane);
        }
        let p = server.panes.get_mut(pane).ok_or(ModelError::StaleId)?;
        if sc.flags.contains(SpawnFlags::FLOATING) {
            if let Some(cell) = p.layout_cell.and_then(|id| server.layout_cells.get_mut(id)) {
                cell.flags.insert(crate::layout::LayoutCellFlags::FLOATING);
            }
        }
        if sc.flags.contains(SpawnFlags::FLOATOVERZOOM) {
            p.flags.insert(PaneFlags::FLOATOVERZOOM);
        }
        if server
            .windows
            .get(window)
            .unwrap()
            .flags
            .contains(WindowFlags::ZOOMED)
        {
            p.saved_layout_cell = p.layout_cell;
        }
        pane
    };
    sc.pane = Some(pane);
    if let Some(cwd) = cwd {
        server.panes.get_mut(pane).unwrap().cwd = cwd;
    }
    let options = launch_policy(server, sc, pane)?;
    let empty = sc.flags.contains(SpawnFlags::EMPTY);
    if empty {
        let p = server.panes.get_mut(pane).unwrap();
        p.flags.insert(PaneFlags::EMPTY);
        p.base.mode.remove(rmux_emu::screen::ScreenMode::CURSOR);
        p.base.mode.insert(rmux_emu::screen::ScreenMode::CRLF);
        p.fd = None;
    } else {
        server
            .panes
            .get_mut(pane)
            .unwrap()
            .flags
            .remove(PaneFlags::EMPTY);
        let result = PreparedLaunch::new(options).and_then(PreparedLaunch::launch);
        match result {
            Ok(process) => {
                let p = server.panes.get_mut(pane).unwrap();
                p.pid = Some(process.pid);
                p.fd = Some(process.master);
                p.tty = process.tty;
            }
            Err(error) => {
                if !respawn {
                    crate::layout::close_pane(server, pane);
                    super::window::window_remove_pane(server, window, pane)?;
                }
                return Err(spawn_error(b"fork failed: ", error));
            }
        }
    }
    let p = server.panes.get_mut(pane).unwrap();
    p.flags.remove(PaneFlags::EXITED);
    let mut command = Vec::new();
    for (i, arg) in p.argv.iter().enumerate() {
        if i != 0 {
            command.push(b' ');
        }
        command.extend_from_slice(&crate::cmd::arguments::escape(arg));
    }
    if command.is_empty() {
        command.clone_from(&p.shell);
    }
    let effect = SpawnEffect::PaneCreated {
        session: sc.session,
        winlink: link,
        window,
        pane,
        window_index: index,
        command,
        cwd: p.cwd.clone(),
        empty,
        respawn,
    };
    server.effects.push_back(ModelEffect::Spawn(effect));
    server.emit(b"pane-created", Some(sc.session), Some(window), Some(pane));
    if respawn {
        return Ok(pane);
    }
    let w = server.windows.get_mut(window).unwrap();
    let select = if sc.flags.contains(SpawnFlags::MODAL) {
        w.modal_last = w.active;
        w.modal = Some(pane);
        true
    } else {
        (!sc.flags.contains(SpawnFlags::DETACHED) || w.active.is_none()) && w.modal.is_none()
    };
    if sc.flags.contains(SpawnFlags::MODAL) {
        super::window::window_redraw_active_switch(server, window, Some(pane))?;
    }
    if select {
        super::window::window_set_active_pane(
            server,
            window,
            pane,
            !sc.flags.contains(SpawnFlags::NONOTIFY),
        )?;
    }
    if !sc.flags.contains(SpawnFlags::NONOTIFY) {
        server.emit(b"window-layout-changed", None, Some(window), None);
    }
    Ok(pane)
}

pub fn spawn_window(server: &mut Server, sc: &mut SpawnContext) -> Result<WinlinkId, ModelError> {
    let respawn = sc.flags.contains(SpawnFlags::RESPAWN);
    let window = if respawn {
        let link = sc.winlink.ok_or(ModelError::StaleId)?;
        let wl = server.winlinks.get(link).ok_or(ModelError::StaleId)?;
        let (window, index) = (wl.window, wl.index);
        let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
        if !sc.flags.contains(SpawnFlags::KILL)
            && w.panes
                .iter()
                .any(|p| server.panes.get(*p).is_some_and(|p| p.fd.is_some()))
        {
            let mut message = b"window ".to_vec();
            message.extend_from_slice(
                &server
                    .sessions
                    .get(sc.session)
                    .ok_or(ModelError::StaleId)?
                    .name,
            );
            message.extend_from_slice(format!(":{index} still active").as_bytes());
            return Err(ModelError::Message(message));
        }
        let pane = w
            .panes
            .first()
            .copied()
            .ok_or_else(|| ModelError::message(b"no pane"))?;
        sc.pane = Some(pane);
        crate::layout::free(server, window, false);
        let panes = server.windows.get(window).unwrap().panes.clone();
        for other in panes {
            if other != pane {
                super::window::window_remove_pane(server, window, other)?;
            }
        }
        let w = server.windows.get(window).unwrap();
        super::pane::pane_resize(server, pane, w.sx, w.sy)?;
        crate::layout::init(server, window, pane);
        server.windows.get_mut(window).unwrap().active = None;
        super::window::window_set_active_pane(server, window, pane, false)?;
        window
    } else {
        let existing = if sc.index != -1 {
            server
                .sessions
                .get(sc.session)
                .ok_or(ModelError::StaleId)?
                .windows
                .get(&sc.index)
                .copied()
        } else {
            None
        };
        if let Some(old) = existing {
            if !sc.flags.contains(SpawnFlags::KILL) {
                return Err(ModelError::Message(
                    format!("index {} in use", sc.index).into_bytes(),
                ));
            }
            let window = server.winlinks.get(old).ok_or(ModelError::StaleId)?.window;
            server
                .winlinks
                .get_mut(old)
                .unwrap()
                .flags
                .remove(super::WinlinkFlags::ALERTFLAGS);
            server.emit(b"window-unlinked", Some(sc.session), Some(window), None);
            let session = server.sessions.get_mut(sc.session).unwrap();
            session.last.retain(|&link| link != old);
            if session.current == Some(old) {
                session.current = None;
                sc.flags.remove(SpawnFlags::DETACHED);
            }
            super::window::winlink_remove(server, old);
        }
        let clients = crate::server::run::resize_clients(server);
        let s = server.sessions.get(sc.session).ok_or(ModelError::StaleId)?;
        let policy = super::WindowSizePolicy::try_from(
            server
                .options
                .get_number(server.options.global_w, b"window-size") as i32,
        )
        .map_err(|_| ModelError::message(b"invalid window size policy"))?;
        let size = sc.initial_size.unwrap_or_else(|| {
            super::resize::default_window_size(
                &clients,
                sc.client,
                sc.session,
                None,
                policy,
                server.options.get_string(s.options, b"default-size"),
            )
        });
        let index = if sc.index == -1 {
            -1 - server.options.get_number(s.options, b"base-index") as i32
        } else {
            sc.index
        };
        let window =
            super::window::window_create(server, size.sx, size.sy, size.xpixel, size.ypixel)?;
        let link = match super::window::winlink_add(server, sc.session, window, index) {
            Ok(link) => link,
            Err(error) => {
                super::window::window_destroy(server, window)?;
                return Err(error);
            }
        };
        sc.winlink = Some(link);
        let s = server.sessions.get_mut(sc.session).unwrap();
        if s.current.is_none() {
            s.current = Some(link);
        }
        server.windows.get_mut(window).unwrap().latest = sc.client;
        window
    };
    sc.flags.insert(SpawnFlags::NONOTIFY);
    if let Err(error) = spawn_pane(server, sc) {
        if !respawn {
            super::window::winlink_remove(server, sc.winlink.unwrap());
        }
        return Err(error);
    }
    let link = sc.winlink.ok_or(ModelError::StaleId)?;
    if !respawn {
        let name = sc
            .name
            .clone()
            .unwrap_or_else(|| super::names::default_window_name(server, window));
        let w = server.windows.get_mut(window).unwrap();
        w.name = name;
        if sc.name.is_some() {
            server
                .options
                .set_number_value(w.options, b"automatic-rename", 0);
        }
        crate::ui::border::window_set_fill_cells(server, window);
    }
    if !sc.flags.contains(SpawnFlags::DETACHED) {
        super::session::session_set_current(server, sc.session, Some(link));
    }
    if !respawn {
        server.emit(b"window-created", Some(sc.session), Some(window), None);
        server.emit(b"window-linked", Some(sc.session), Some(window), None);
    }
    super::session::session_group_synchronize_from(server, sc.session);
    Ok(link)
}

pub type SpawnFinishEdit = Box<dyn FnOnce(&mut Server, EditorId, Option<Vec<u8>>)>;
pub struct SpawnEditorState {
    pub path: Vec<u8>,
    pub pid: rmux_sys::ProcessId,
    pub pane: PaneId,
    pub callback: Option<SpawnFinishEdit>,
}

impl Drop for SpawnEditorState {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(std::ffi::OsStr::from_bytes(&self.path));
    }
}

pub fn spawn_cancel_editor(server: &mut Server, editor: EditorId) {
    if let Some(editor) = server.editors.get_mut(editor) {
        editor.callback = None;
    }
}

pub fn spawn_get_editor_pid(
    server: &Server,
    editor: Option<EditorId>,
) -> Option<rmux_sys::ProcessId> {
    server.editors.get(editor?).map(|e| e.pid)
}

pub fn spawn_editor_finish(server: &mut Server, pane: PaneId) {
    let Some(p) = server.panes.get_mut(pane) else {
        return;
    };
    let Some(editor) = p.editor.take() else {
        return;
    };
    let status = if p.flags.contains(PaneFlags::STATUSREADY) {
        rmux_sys::proc::exit_code(p.status)
    } else {
        129
    };
    let Ok(Some(mut editor_state)) = server.editors.request_remove(editor) else {
        return;
    };
    let Some(callback) = editor_state.callback.take() else {
        return;
    };
    let data = if status == 0 {
        std::fs::File::open(std::ffi::OsStr::from_bytes(&editor_state.path))
            .ok()
            .and_then(|mut file| {
                let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
                if len == 0 {
                    return None;
                }
                let mut bytes = Vec::new();
                bytes.try_reserve_exact(len).ok()?;
                file.read_to_end(&mut bytes).ok()?;
                if bytes.len() != len {
                    return None;
                }
                Some(bytes)
            })
    } else {
        None
    };
    callback(server, editor, data);
}

pub fn spawn_editor(
    server: &mut Server,
    context: &SpawnContext,
    initial: &[u8],
    callback: SpawnFinishEdit,
) -> Result<EditorId, ModelError> {
    let link = server
        .sessions
        .get(context.session)
        .ok_or(ModelError::StaleId)?
        .current
        .ok_or_else(|| ModelError::message(b"no current window"))?;
    let window = server.winlinks.get(link).ok_or(ModelError::StaleId)?.window;
    let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
    if w.modal.is_some() {
        return Err(ModelError::message(b"window already has a modal pane"));
    }
    let geometry = crate::layout::LayoutGeometry {
        sx: w.sx * 9 / 10,
        sy: w.sy * 9 / 10,
        xoff: (w.sx / 2 - (w.sx * 9 / 10) / 2) as i32,
        yoff: (w.sy / 2 - (w.sy * 9 / 10) / 2) as i32,
    };
    let (mut file, path) = rmux_sys::proc::temporary_file(b"/tmp/rmux.XXXXXXXX")
        .map_err(|e| spawn_error(b"temporary file: ", e))?;
    // fwrite(buf, 0, 1) returns zero at the pin; creation must fail for empty input.
    if initial.is_empty() {
        drop(file);
        let _ = std::fs::remove_file(std::ffi::OsStr::from_bytes(&path));
        return Err(ModelError::message(b"couldn't write editor file"));
    }
    if let Err(error) = file.write_all(initial) {
        drop(file);
        let _ = std::fs::remove_file(std::ffi::OsStr::from_bytes(&path));
        return Err(spawn_error(b"couldn't write editor file: ", error));
    }
    drop(file);
    let mut command = server
        .options
        .get_string(server.options.global, b"editor")
        .to_vec();
    command.push(b' ');
    command.extend_from_slice(&path);
    if let Err(error) = super::window::window_push_zoom(server, window, false, true) {
        let _ = std::fs::remove_file(std::ffi::OsStr::from_bytes(&path));
        return Err(error);
    }
    let cell = crate::layout::floating_pane(server, window, None, &geometry);
    let mut sc = SpawnContext::new(context.session);
    sc.winlink = Some(link);
    sc.layout_cell = Some(cell);
    sc.pane = server.windows.get(window).unwrap().active;
    sc.client = context.client;
    sc.client_cwd.clone_from(&context.client_cwd);
    sc.client_environment
        .clone_from(&context.client_environment);
    sc.client_attached = context.client_attached;
    sc.argv = vec![command];
    sc.cwd = Some(b"/tmp".to_vec());
    sc.flags = SpawnFlags::FLOATING | SpawnFlags::MODAL | SpawnFlags::FLOATOVERZOOM;
    let spawned = spawn_pane(server, &mut sc);
    let pop = super::window::window_pop_zoom(server, window, false);
    let pane = match spawned {
        Ok(pane) => {
            if let Err(error) = pop {
                let _ = std::fs::remove_file(std::ffi::OsStr::from_bytes(&path));
                return Err(error);
            }
            pane
        }
        Err(error) => {
            let _ = std::fs::remove_file(std::ffi::OsStr::from_bytes(&path));
            return Err(error);
        }
    };
    let p = server.panes.get(pane).ok_or(ModelError::StaleId)?;
    let pid = p.pid.ok_or(ModelError::StaleId)?;
    server
        .options
        .set_number_value(p.options, b"remain-on-exit", 0);
    let editor = server.editors.insert(SpawnEditorState {
        path,
        pid,
        pane,
        callback: Some(callback),
    })?;
    server.panes.get_mut(pane).unwrap().editor = Some(editor);
    Ok(editor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_window_takes_the_size_of_the_creating_client() {
        let mut server = Server::new();
        let options = server.options.create(Some(server.options.global_s));
        let session = super::super::session::session_create(
            &mut server,
            super::super::session::SessionCreate {
                prefix: None,
                name: Some(b"size".to_vec()),
                cwd: b"/tmp".to_vec(),
                environment: Environment::new(),
                options,
                termios: None,
            },
        );
        server.sessions.get_mut(session).unwrap().statuslines = 1;
        let mut client = crate::client::Client::new(None, (0, 0));
        client.session = Some(session);
        client.tty_sx = 120;
        client.tty_sy = 40;
        let client = server.clients.insert(client).unwrap();
        server.client_order.push_back(client);
        let mut context = SpawnContext::new(session);
        context.client = Some(client);
        context.flags = SpawnFlags::EMPTY;
        let link = spawn_window(&mut server, &mut context).unwrap();
        let window = server.winlinks.get(link).unwrap().window;
        let window = server.windows.get(window).unwrap();
        assert_eq!((window.sx, window.sy), (120, 39));
    }

    #[test]
    fn pane_environment_uses_start_broker_flag_not_inherited_capabilities() {
        let mut server = Server::new();
        let global = server.options.global;
        let mut options = std::mem::take(&mut server.options);
        options.set_string(
            global,
            b"default-terminal",
            false,
            b"screen-256color",
            &mut server,
        );
        server.options = options;
        let none = EnvironmentFlags::default();
        server
            .global_environment
            .set(b"TERM_PROGRAM", none, b"tern");
        server.global_environment.set(b"RMUX_TSP", none, b"stale");
        server.global_environment.set(b"PI_TUI_NATIVE", none, b"0");
        let mut session_environment = Environment::new();
        session_environment.set(b"RMUX_TSP", none, b"session-marker");
        let options = server.options.create(Some(server.options.global_s));
        let session = super::super::session::session_create(
            &mut server,
            super::super::session::SessionCreate {
                prefix: None,
                name: Some(b"environment".to_vec()),
                cwd: b"/tmp".to_vec(),
                environment: session_environment,
                options,
                termios: None,
            },
        );
        let window = super::super::window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = super::super::pane::pane_create(&mut server, window, 80, 24, 0).unwrap();
        let mut context = SpawnContext::new(session);
        context.environment.set(b"RMUX_TSP", none, b"spawn-marker");
        context.environment.set(b"TERM_PROGRAM", none, b"tern");
        context
            .environment
            .set(b"TERM_PROGRAM_VERSION", none, b"outer-version");
        for enabled in [true, false] {
            server.tsp_broker_enabled = enabled;
            let launch = launch_policy(&mut server, &context, pane).unwrap();
            let value = |name: &[u8]| {
                launch.environment.iter().find_map(|entry| {
                    let (key, value) = entry.split_at(entry.iter().position(|&byte| byte == b'=')?);
                    (key == name).then_some(&value[1..])
                })
            };
            assert_eq!(value(b"TERM"), Some(b"screen-256color".as_slice()));
            assert_eq!(value(b"TERM_PROGRAM"), Some(b"rmux".as_slice()));
            assert_eq!(
                value(b"TERM_PROGRAM_VERSION"),
                Some(crate::options::environment::RMUX_VERSION)
            );
            assert_eq!(value(b"RMUX_TSP"), enabled.then_some(b"1".as_slice()));
            assert_eq!(value(b"PI_TUI_NATIVE"), Some(b"0".as_slice()));
        }
        // Without the user's override, a brokered pane asks released omp to
        // probe TSP (it skips the probe under a tmux TERM otherwise).
        server.global_environment.unset(b"PI_TUI_NATIVE");
        for enabled in [true, false] {
            server.tsp_broker_enabled = enabled;
            let launch = launch_policy(&mut server, &context, pane).unwrap();
            let native = launch
                .environment
                .iter()
                .find_map(|entry| entry.strip_prefix(b"PI_TUI_NATIVE=".as_slice()));
            assert_eq!(native, enabled.then_some(b"1".as_slice()));
        }
    }
}
