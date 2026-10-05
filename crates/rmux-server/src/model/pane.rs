// Ported from tmux window.c, input-keys.c @ 8f25579c
use super::state::{ModelEffect, ModelError, Pane, Server};
use super::window::{self, WindowEffect, option_number};
use super::{PaneFlags, WindowFlags};
use crate::ids::{ClientId, ModeId, PaneId, QueueItemId, WindowId};
use crate::modes::WindowModeFlags;
use crate::ui::prompt::{PromptKeyResult, PromptType};
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use rmux_emu::colour::{Colour, ColourPalette};
use rmux_emu::input::InputCtx;
use rmux_emu::input::keys::{KeyPolicy, encode_key, encode_mouse};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_util::key::{KeyCode, KeyCodeType, KeyMasks, MouseEvent};
use std::any::Any;
use std::collections::VecDeque;
use std::os::fd::AsFd;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneResize {
    pub sx: u32,
    pub sy: u32,
    pub osx: u32,
    pub osy: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneOffset {
    pub used: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneEffect {
    WaitFinished {
        pane: PaneId,
        item: QueueItemId,
        retval: i32,
    },
    CancelTimers(PaneId),
    ClearSync(PaneId),
    StopSync(PaneId),
    Kill(PaneId),
    Resized {
        pane: PaneId,
        size: PaneResize,
    },
    ScrollbarTimer {
        pane: PaneId,
        milliseconds: Option<u64>,
    },
    ModeChanged {
        pane: PaneId,
        previous: Option<Vec<u8>>,
        current: Option<Vec<u8>>,
        entered: bool,
    },
    PromptChanged {
        pane: PaneId,
        kind: PromptType,
    },
    TitleChanged {
        pane: PaneId,
        new: Vec<u8>,
    },
}
fn effect(server: &mut Server, value: PaneEffect) {
    server.effects.push_back(ModelEffect::Pane(value));
}

pub trait PaneModeDriver {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen>;
    fn free(&self, server: &mut Server, mode: PaneMode);
    fn resize(&self, server: &mut Server, id: ModeId, sx: u32, sy: u32);
    fn key(
        &self,
        server: &mut Server,
        id: ModeId,
        client: ClientId,
        key: KeyCode,
        mouse: Option<&crate::client::ResolvedMouseEvent>,
    );
    fn append_output(
        &self,
        server: &mut Server,
        id: ModeId,
        bytes: &[u8],
    ) -> Result<(), ModelError>;
    /// window_copy_get_current_offset: (offset, size) for the scrollbar.
    /// Modes without a scrollable history report nothing and no slider is
    /// drawn (screen-redraw.c:1437-1440).
    fn current_offset(&self, _server: &Server, _id: ModeId) -> Option<(u32, u32)> {
        None
    }
    /// `wme->mode->key_table` (`tmux.h:1319`); `None` when the mode has no table.
    fn key_table(&self, _server: &Server, _id: ModeId) -> Option<Vec<u8>> {
        None
    }
    /// `wme->mode->command != NULL` (`cmd-send-keys.c:192,201`).
    fn has_command(&self) -> bool {
        false
    }
    /// `wme->mode->command(wme, c, s, wl, args, m)` (`tmux.h:1310-1312`).
    #[allow(clippy::too_many_arguments)]
    fn command(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _client: Option<ClientId>,
        _session: Option<crate::ids::SessionId>,
        _winlink: Option<crate::ids::WinlinkId>,
        _args: &crate::cmd::arguments::Args,
        _event: Option<&crate::cmd::queue::QueueEvent>,
    ) {
    }
    /// `wme->mode->update` (`tmux.h:1314`).
    fn update(&self, _server: &mut Server, _id: ModeId) {}
    /// `wme->mode->style_changed` (`tmux.h:1315`).
    fn style_changed(&self, _server: &mut Server, _id: ModeId) {}
    /// `c->tty.mouse_drag_update` installed by a mode (`tmux.h:1852-1855`).
    fn drag_update(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _client: ClientId,
        _event: &crate::client::ResolvedMouseEvent,
    ) {
    }
    /// `c->tty.mouse_drag_release` installed by a mode.
    fn drag_release(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _client: ClientId,
        _event: &crate::client::ResolvedMouseEvent,
    ) {
    }
}

fn mode_driver(server: &Server, mode: ModeId) -> Option<Rc<dyn PaneModeDriver>> {
    server
        .panes
        .get(mode.owner)?
        .modes
        .iter()
        .find(|m| m.id == mode)
        .map(|m| m.driver.clone())
}
pub fn pane_mode_drag_update(
    server: &mut Server,
    mode: ModeId,
    client: ClientId,
    event: &crate::client::ResolvedMouseEvent,
) {
    if let Some(driver) = mode_driver(server, mode) {
        driver.drag_update(server, mode, client, event);
    }
}
pub fn pane_mode_drag_release(
    server: &mut Server,
    mode: ModeId,
    client: ClientId,
    event: &crate::client::ResolvedMouseEvent,
) {
    if let Some(driver) = mode_driver(server, mode) {
        driver.drag_release(server, mode, client, event);
    }
}

fn first_mode(server: &Server, wp: PaneId) -> Option<(Rc<dyn PaneModeDriver>, ModeId)> {
    server
        .panes
        .get(wp)?
        .modes
        .first()
        .map(|m| (m.driver.clone(), m.id))
}
/// `wme->mode->key_table(wme)` for the first mode (`cmd-send-keys.c:95-100`).
pub fn pane_mode_key_table(server: &Server, wp: PaneId) -> Option<Vec<u8>> {
    let (driver, id) = first_mode(server, wp)?;
    driver.key_table(server, id)
}
/// `wme != NULL && wme->mode->command != NULL`.
pub fn pane_mode_has_command(server: &Server, wp: PaneId) -> bool {
    first_mode(server, wp).is_some_and(|(driver, _)| driver.has_command())
}
/// `wme->mode->command(wme, tc, s, wl, args, m)` (`cmd-send-keys.c:207`).
pub fn pane_mode_command(
    server: &mut Server,
    wp: PaneId,
    client: Option<ClientId>,
    session: Option<crate::ids::SessionId>,
    winlink: Option<crate::ids::WinlinkId>,
    args: &crate::cmd::arguments::Args,
    event: Option<&crate::cmd::queue::QueueEvent>,
) {
    if let Some((driver, id)) = first_mode(server, wp) {
        driver.command(server, id, client, session, winlink, args, event);
    }
}
/// `wme->mode->update(wme)` for the first mode (`server-client.c`).
pub fn pane_mode_update(server: &mut Server, wp: PaneId) {
    if let Some((driver, id)) = first_mode(server, wp) {
        driver.update(server, id);
    }
}
/// `wme->mode->style_changed(wme)` for the first mode.
pub fn pane_mode_style_changed(server: &mut Server, wp: PaneId) {
    if let Some((driver, id)) = first_mode(server, wp) {
        driver.style_changed(server, id);
    }
}
pub struct PaneMode {
    pub id: ModeId,
    pub name: Vec<u8>,
    pub flags: WindowModeFlags,
    pub prefix: u32,
    pub kill: bool,
    pub screen: Option<Screen>,
    pub data: Option<Box<dyn Any>>,
    pub driver: Rc<dyn PaneModeDriver>,
}

// The prompt engine owns editing/history; this adapter owns only callback lifetime.
pub trait PanePromptEngine {
    fn start(&mut self, server: &mut Server, pane: PaneId);
    fn key(
        &mut self,
        server: &mut Server,
        pane: PaneId,
        client: ClientId,
        key: KeyCode,
        mouse: Option<(u32, u32)>,
    ) -> PromptKeyResult;
    fn is_open(&self) -> bool;
    fn update(&mut self, server: &mut Server, message: &[u8], input: &[u8]);
    fn draw(
        &mut self,
        server: &mut Server,
        pane: PaneId,
        ctx: &mut rmux_emu::screen::write::ScreenWriteCtx<'_>,
        pdd: &mut crate::ui::prompt::PromptDrawData<'_>,
    );
    fn free(self: Box<Self>, server: &mut Server);
}
pub struct PanePrompt {
    pub kind: PromptType,
    pub engine: Box<dyn PanePromptEngine>,
}

impl Pane {
    pub fn displayed_screen(&self) -> &Screen {
        self.modes
            .first()
            .and_then(|m| m.screen.as_ref())
            .unwrap_or(&self.base)
    }
}
pub trait CfgModelRuntime {
    fn pane_top_is_view(&self, pane: PaneId) -> bool;
    fn enter_view_mode(
        &mut self,
        pane: PaneId,
        driver: Rc<dyn PaneModeDriver>,
    ) -> Result<Option<ModeId>, ModelError>;
    fn append_view_line(&mut self, pane: PaneId, line: &[u8]) -> Result<(), ModelError>;
}
impl CfgModelRuntime for Server {
    fn pane_top_is_view(&self, pane: PaneId) -> bool {
        self.panes
            .get(pane)
            .and_then(|p| p.modes.first())
            .is_some_and(|m| m.name == b"view-mode")
    }
    fn enter_view_mode(
        &mut self,
        pane: PaneId,
        driver: Rc<dyn PaneModeDriver>,
    ) -> Result<Option<ModeId>, ModelError> {
        pane_set_mode(
            self,
            pane,
            b"view-mode",
            WindowModeFlags::default(),
            driver,
            false,
        )
    }
    fn append_view_line(&mut self, pane: PaneId, line: &[u8]) -> Result<(), ModelError> {
        let mode = self
            .panes
            .get(pane)
            .ok_or(ModelError::StaleId)?
            .modes
            .first()
            .ok_or_else(|| ModelError::message(b"pane is not in view mode"))?;
        if mode.name != b"view-mode" {
            return Err(ModelError::message(b"pane is not in view mode"));
        }
        let id = mode.id;
        let driver = Rc::clone(&mode.driver);
        driver.append_output(self, id, line)
    }
}

pub fn pane_create(
    server: &mut Server,
    window: WindowId,
    sx: u32,
    sy: u32,
    hlimit: u32,
) -> Result<PaneId, ModelError> {
    let parent = server
        .windows
        .get(window)
        .ok_or(ModelError::StaleId)?
        .options;
    let options = server.options.create(Some(parent));
    let policy = ScreenResetPolicy {
        extended_keys: option_number(server, server.options.global, b"extended-keys", 0) == 2,
    };
    let mut base = Screen::new(sx, sy, hlimit, policy, &mut server.hyperlinks)
        .map_err(|e| ModelError::Sys(e.to_string()))?;
    #[cfg(feature = "sixel")]
    base.bind_images(&mut server.images);
    let status_screen = Screen::new(1, 1, 0, policy, &mut server.hyperlinks)
        .map_err(|e| ModelError::Sys(e.to_string()))?;
    let mut cursor_cell = rmux_emu::cell::DEFAULT_CELL;
    if let Some(style) =
        server
            .options
            .string_to_style(options, b"cursor-colour", None, &mut server.hyperlinks)
    {
        style.overlay_cell(&mut cursor_cell);
    }
    base.set_default_cursor(
        cursor_cell.fg,
        option_number(server, options, b"cursor-style", 0) as u32,
    );
    let hostname = rmux_sys::osdep::hostname().map_err(|e| ModelError::Sys(e.to_string()))?;
    base.set_title(&hostname, false);
    let mut scrollbar_style = rmux_emu::style::Style::default();
    let mut format = crate::format::FormatTree::create(
        None,
        None,
        0,
        crate::format::FormatFlags::NOJOBS,
        server,
    );
    let default_style = format.expand(server, b"bg=themedarkgrey,fg=themelightgrey,width=1,pad=0");
    scrollbar_style
        .parse(
            &rmux_emu::cell::DEFAULT_CELL,
            &default_style,
            &mut server.hyperlinks,
        )
        .map_err(|e| ModelError::Sys(format!("scrollbar style: {e:?}")))?;
    if server
        .options
        .get(options, b"pane-scrollbars-style")
        .is_some()
    {
        let configured = server
            .options
            .get_string(options, b"pane-scrollbars-style")
            .to_vec();
        let expanded = format.expand(server, &configured);
        if scrollbar_style
            .parse(
                &rmux_emu::cell::DEFAULT_CELL,
                &expanded,
                &mut server.hyperlinks,
            )
            .is_err()
        {
            scrollbar_style
                .parse(
                    &rmux_emu::cell::DEFAULT_CELL,
                    &default_style,
                    &mut server.hyperlinks,
                )
                .map_err(|e| ModelError::Sys(format!("scrollbar style: {e:?}")))?;
        }
    }
    format.release(server);
    scrollbar_style.width = scrollbar_style.width.max(1);
    scrollbar_style.pad = scrollbar_style.pad.max(0);
    scrollbar_style.gc.data = rmux_util::utf8::Utf8Data {
        data: {
            let mut bytes = [0; 32];
            bytes[0] = b' ';
            bytes
        },
        have: 0,
        size: 1,
        width: 1,
    };
    let mut palette = ColourPalette::new();
    if let Some((_, entry)) = server.options.get(options, b"pane-colours") {
        let mut defaults = [Colour::NONE; 256];
        for (key, item) in entry.array_items() {
            if let crate::options::OptionsArrayKey::Index(index) = key {
                if let Some(slot) = defaults.get_mut(*index as usize) {
                    if let Some(value) = item.value().as_number() {
                        *slot = Colour(value as i32);
                    }
                }
            }
        }
        palette.replace_defaults(Some(defaults));
    }
    let public_id = server.next_pane_id;
    server.next_pane_id = server.next_pane_id.wrapping_add(1);
    let pane = Pane {
        public_id,
        window,
        options,
        flags: PaneFlags::STYLECHANGED,
        references: 1,
        sx,
        sy,
        xoff: 0,
        yoff: 0,
        active_point: 0,
        base,
        status_screen,
        parser: InputCtx::new(),
        argv: Vec::new(),
        shell: Vec::new(),
        cwd: Vec::new(),
        tty: Vec::new(),
        pid: None,
        fd: None,
        status: -1,
        editor: None,
        input: Vec::new(),
        output: Vec::new(),
        base_offset: 0,
        parser_offset: 0,
        layout_cell: None,
        saved_layout_cell: None,
        scrollbar_style,
        dead_time: (0, 0),
        last_output_time: 0,
        last_prompt_time: 0,
        cmd_start_time: 0,
        cmd_end_time: 0,
        output_generation: 0,
        cmd_status: -1,
        searchstr: None,
        searchregex: false,
        palette,
        resizes: VecDeque::new(),
        wait_item: None,
        modes: Vec::new(),
        next_mode_id: 0,
        prompt: None,
        prompt_generation: 0,
        input_state: super::pane_input::PaneInputState::default(),
        tsp: None,
        pipe: None,
        control_fg: -1,
        control_bg: -1,
        scrollbar_visible: false,
        scrollbar_hover: false,
        cached_gc: rmux_emu::cell::DEFAULT_CELL,
        cached_active_gc: rmux_emu::cell::DEFAULT_CELL,
        cached_dim: 0,
        cached_active_dim: 0,
        sync: crate::ui::fanout::SyncOutputState::default(),
        status_generation: 0,
        border_status_line: rmux_emu::style::StyleLineEntry::default(),
        border_gc: rmux_emu::cell::DEFAULT_CELL,
        border_gc_set: false,
        active_border_gc: rmux_emu::cell::DEFAULT_CELL,
        active_border_gc_set: false,
        sb_slider_y: 0,
        sb_slider_h: 0,
        prompt_cx: 0,
        visible_ranges: crate::ui::visible::VisibleRanges::default(),
    };
    let id = server.panes.insert(pane)?;
    server.panes.retain(id)?;
    server.pane_ids.insert(public_id, id);
    Ok(id)
}

pub fn pane_find_by_public_id(server: &Server, public_id: u32) -> Option<PaneId> {
    server.pane_ids.get(&public_id).copied()
}
pub fn pane_find(server: &Server, text: &[u8]) -> Option<PaneId> {
    let text = rmux_util::bytes::cstr(text).strip_prefix(b"%")?;
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return None;
    }
    pane_find_by_public_id(server, std::str::from_utf8(text).ok()?.parse().ok()?)
}
pub fn pane_retain(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    let refs = server
        .panes
        .get(id)
        .ok_or(ModelError::StaleId)?
        .references
        .checked_add(1)
        .ok_or(crate::ids::ArenaError::LeaseOverflow)?;
    server.panes.retain(id)?;
    server
        .panes
        .get_mut(id)
        .ok_or(ModelError::StaleId)?
        .references = refs;
    Ok(())
}
pub fn pane_release(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    p.references = p
        .references
        .checked_sub(1)
        .ok_or(crate::ids::ArenaError::LeaseUnderflow)?;
    let zero = p.references == 0;
    if zero {
        server.panes.request_remove(id)?;
    }
    if let Some(mut pane) = server.panes.release(id)? {
        pane.base
            .release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                Some(&mut server.images),
            )
            .map_err(|e| ModelError::Sys(e.to_string()))?;
        pane.status_screen
            .release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            )
            .map_err(|e| ModelError::Sys(e.to_string()))?;
        server.free_options(pane.options);
    }
    Ok(())
}

pub fn pane_wait_finish(server: &mut Server, id: PaneId) {
    let Some(pane) = server.panes.get_mut(id) else {
        return;
    };
    let Some(item) = pane.wait_item.take() else {
        return;
    };
    let retval = if pane.flags.contains(PaneFlags::STATUSREADY) {
        rmux_sys::proc::exit_code(pane.status)
    } else {
        129
    };
    effect(
        server,
        PaneEffect::WaitFinished {
            pane: id,
            item,
            retval,
        },
    );
}

pub fn pane_destroy_ready(
    server: &Server,
    id: PaneId,
    pipe_output_empty: bool,
    unread_pty: usize,
) -> bool {
    server.panes.get(id).is_some_and(|p| {
        pipe_output_empty
            && unread_pty == 0
            && p.flags.contains(PaneFlags::EXITED)
            && ((p.wait_item.is_none() && p.editor.is_none())
                || p.flags.contains(PaneFlags::STATUSREADY))
    })
}

pub fn pane_destroy(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    if server
        .panes
        .get(id)
        .ok_or(ModelError::StaleId)?
        .flags
        .contains(PaneFlags::DESTROYED)
    {
        return Ok(());
    }
    crate::tsp::broker::pane_reset(server, id);
    crate::server::run::close_pane_io(server, id);
    pane_wait_finish(server, id);
    super::spawn::spawn_editor_finish(server, id);
    let Some(p) = server.panes.get_mut(id) else {
        return Ok(());
    };
    server.pane_ids.remove(&p.public_id);
    p.flags.insert(PaneFlags::DESTROYED);
    pane_clear_prompt(server, id)?;
    while let Some(mode) = server.panes.get_mut(id).and_then(|p| {
        if p.modes.is_empty() {
            None
        } else {
            Some(p.modes.remove(0))
        }
    }) {
        let driver = Rc::clone(&mode.driver);
        driver.free(server, mode);
    }
    if server.panes.get(id).is_some() {
        super::pane_input::cancel_pane_requests(server, id)?;
    }
    effect(server, PaneEffect::ClearSync(id));
    if let Some(p) = server.panes.get_mut(id) {
        p.fd = None;
        p.resizes.clear();
        p.input_state.clear();
    }
    effect(server, PaneEffect::CancelTimers(id));
    if server.panes.get(id).is_some() {
        pane_release(server, id)?;
    }
    Ok(())
}

pub fn pane_clear_resizes(
    server: &mut Server,
    id: PaneId,
    except: Option<usize>,
) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    if let Some(keep) = except.and_then(|i| p.resizes.get(i).copied()) {
        p.resizes.clear();
        p.resizes.push_back(keep);
    } else {
        p.resizes.clear();
    }
    Ok(())
}

pub fn pane_resize(server: &mut Server, id: PaneId, sx: u32, sy: u32) -> Result<(), ModelError> {
    let p = server.panes.get(id).ok_or(ModelError::StaleId)?;
    if (p.sx, p.sy) == (sx, sy) {
        return Ok(());
    }
    super::pane_input::pane_stop_sync(server, id)?;
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let size = PaneResize {
        sx,
        sy,
        osx: p.sx,
        osy: p.sy,
    };
    p.resizes.push_back(size);
    p.sx = sx;
    p.sy = sy;
    p.base.resize(
        sx,
        sy,
        p.base.saved_grid.is_none(),
        #[cfg(feature = "sixel")]
        Some(&mut server.images),
    );
    let window = p.window;
    let mode = p.modes.first().map(|m| (m.id, Rc::clone(&m.driver)));
    crate::tsp::broker::drain_anchors(server, id);
    if let Some((mode, driver)) = mode {
        driver.resize(server, mode, sx, sy);
    }
    if server.panes.get(id).is_some() {
        effect(server, PaneEffect::Resized { pane: id, size });
        server.emit(b"pane-resized", None, Some(window), Some(id));
    }
    Ok(())
}

pub fn pane_send_resize(server: &Server, id: PaneId, sx: u32, sy: u32) -> Result<(), ModelError> {
    let p = server.panes.get(id).ok_or(ModelError::StaleId)?;
    let Some(fd) = &p.fd else {
        return Ok(());
    };
    let w = server.windows.get(p.window).ok_or(ModelError::StaleId)?;
    rmux_sys::pty::set_winsize(
        fd.as_fd(),
        rmux_sys::pty::Winsize {
            cols: sx as u16,
            rows: sy as u16,
            xpixel: w.xpixel.wrapping_mul(sx as u16 as u32) as u16,
            ypixel: w.ypixel.wrapping_mul(sy as u16 as u32) as u16,
        },
    )
    .map_err(|e| ModelError::Sys(e.to_string()))
}

pub fn pane_is_visible(server: &Server, id: PaneId) -> bool {
    server.panes.get(id).is_some_and(|p| {
        server
            .windows
            .get(p.window)
            .is_some_and(|w| !w.flags.contains(WindowFlags::ZOOMED) || p.layout_cell.is_some())
    })
}
pub fn pane_is_floating(server: &Server, id: PaneId) -> bool {
    server
        .panes
        .get(id)
        .and_then(|p| p.layout_cell)
        .and_then(|c| server.layout_cells.get(c))
        .is_some_and(|c| c.is_floating())
}
pub fn pane_is_floating_with_hidden(server: &Server, id: PaneId) -> bool {
    server
        .panes
        .get(id)
        .and_then(|p| p.layout_cell.or(p.saved_layout_cell))
        .and_then(|c| server.layout_cells.get(c))
        .is_some_and(|c| c.is_floating())
}
pub fn pane_exited(server: &Server, id: PaneId) -> bool {
    server
        .panes
        .get(id)
        .is_none_or(|p| p.fd.is_none() || p.flags.contains(PaneFlags::EXITED))
}

pub fn pane_at_index(server: &Server, window: WindowId, index: u32) -> Option<PaneId> {
    let w = server.windows.get(window)?;
    let base = option_number(server, w.options, b"pane-base-index", 0) as u32;
    w.panes.get(index.checked_sub(base)? as usize).copied()
}
pub fn pane_index(server: &Server, id: PaneId) -> Option<u32> {
    let w = server.windows.get(server.panes.get(id)?.window)?;
    Some(
        (w.panes.iter().position(|p| *p == id)? as u32).wrapping_add(option_number(
            server,
            w.options,
            b"pane-base-index",
            0,
        ) as u32),
    )
}
pub fn pane_last_index(server: &Server, id: PaneId) -> Option<u32> {
    let w = server.windows.get(server.panes.get(id)?.window)?;
    w.last.iter().position(|p| *p == id).map(|i| i as u32)
}
pub fn pane_z_index(server: &Server, id: PaneId) -> Option<u32> {
    let w = server.windows.get(server.panes.get(id)?.window)?;
    let mut index = 0;
    for pane in &w.z_order {
        if *pane == id {
            return Some(index + u32::from(!pane_is_floating(server, id)));
        }
        if pane_is_floating(server, *pane) {
            index += 1;
        }
    }
    None
}
pub fn pane_next_by_number(server: &Server, id: PaneId, count: u32) -> Option<PaneId> {
    traverse(server, id, count, false)
}
pub fn pane_previous_by_number(server: &Server, id: PaneId, count: u32) -> Option<PaneId> {
    traverse(server, id, count, true)
}
fn traverse(server: &Server, id: PaneId, count: u32, previous: bool) -> Option<PaneId> {
    let panes = &server.windows.get(server.panes.get(id)?.window)?.panes;
    let at = panes.iter().position(|p| *p == id)?;
    let step = count as usize % panes.len();
    panes
        .get(if previous {
            (at + panes.len() - step) % panes.len()
        } else {
            (at + step) % panes.len()
        })
        .copied()
}

pub fn pane_get_pane_lines(server: &Server, id: PaneId) -> i64 {
    let Some(p) = server.panes.get(id) else {
        return 0;
    };
    let options = if pane_is_floating(server, id) {
        p.options
    } else {
        let Some(w) = server.windows.get(p.window) else {
            return 0;
        };
        w.options
    };
    option_number(server, options, b"pane-border-lines", 0)
}
pub fn pane_get_pane_status(server: &Server, id: PaneId) -> i64 {
    let Some(p) = server.panes.get(id) else {
        return 0;
    };
    if p.flags.contains(PaneFlags::ZOOMED)
        && p.modes
            .first()
            .is_some_and(|m| m.flags.contains(WindowModeFlags::HIDE_PANE_STATUS))
    {
        return 0;
    }
    if !pane_is_floating(server, id) {
        return window::window_get_pane_status(server, p.window);
    }
    if pane_get_pane_lines(server, id) == 6 {
        return 0;
    }
    match option_number(server, p.options, b"pane-border-status", 0) {
        3 => 1,
        4 => 2,
        other => other,
    }
}

pub fn pane_show_scrollbar(server: &Server, id: PaneId) -> bool {
    let Some(p) = server.panes.get(id) else {
        return false;
    };
    let Some(w) = server.windows.get(p.window) else {
        return false;
    };
    if p.base.is_alternate() {
        return false;
    }
    if w.flags.contains(WindowFlags::ZOOMED)
        && w.active
            .and_then(|p| server.panes.get(p))
            .and_then(|p| p.modes.first())
            .is_some_and(|m| m.flags.contains(WindowModeFlags::HIDE_SCROLLBARS))
    {
        return false;
    }
    matches!(
        w.sb,
        PaneScrollbarPolicy::Always | PaneScrollbarPolicy::Autohide
    ) || (w.sb == PaneScrollbarPolicy::Modal
        && p.modes
            .first()
            .is_some_and(|m| m.name == b"copy-mode" || m.name == b"view-mode"))
}
pub fn pane_scrollbar_reserve(server: &Server, id: PaneId) -> bool {
    pane_show_scrollbar(server, id)
        && server
            .panes
            .get(id)
            .and_then(|p| server.windows.get(p.window))
            .is_some_and(|w| w.sb == PaneScrollbarPolicy::Always)
}
pub fn pane_scrollbar_overlay(server: &Server, id: PaneId) -> bool {
    pane_show_scrollbar(server, id)
        && server
            .panes
            .get(id)
            .and_then(|p| server.windows.get(p.window))
            .is_some_and(|w| {
                matches!(
                    w.sb,
                    PaneScrollbarPolicy::Modal | PaneScrollbarPolicy::Autohide
                )
            })
}
pub fn pane_scrollbar_visible(server: &Server, id: PaneId) -> bool {
    pane_show_scrollbar(server, id)
        && (!pane_scrollbar_overlay(server, id)
            || server.panes.get(id).is_some_and(|p| p.scrollbar_visible))
}
pub fn pane_scrollbar_show(
    server: &mut Server,
    id: PaneId,
    start_timer: bool,
) -> Result<(), ModelError> {
    if !pane_scrollbar_overlay(server, id) {
        return Ok(());
    }
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let changed = !p.scrollbar_visible;
    p.scrollbar_visible = true;
    let window = p.window;
    let w = server.windows.get(window).ok_or(ModelError::StaleId)?;
    let delay = option_number(server, w.options, b"pane-scrollbars-timeout", 5000) as u64;
    effect(
        server,
        PaneEffect::ScrollbarTimer {
            pane: id,
            milliseconds: start_timer.then_some(delay),
        },
    );
    if changed {
        scrollbar_redraw_visibility(server, id);
    }
    Ok(())
}
pub fn pane_scrollbar_hide(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    p.scrollbar_hover = false;
    let changed = std::mem::replace(&mut p.scrollbar_visible, false);
    effect(
        server,
        PaneEffect::ScrollbarTimer {
            pane: id,
            milliseconds: None,
        },
    );
    if changed {
        scrollbar_redraw_visibility(server, id);
    }
    Ok(())
}
fn scrollbar_redraw_visibility(server: &mut Server, id: PaneId) {
    if let Some(p) = server.panes.get_mut(id) {
        p.flags.insert(PaneFlags::REDRAW);
        let window = p.window;
        window::effect(server, WindowEffect::InvalidateScene(window));
        window::effect(server, WindowEffect::Redraw(window));
    }
}
pub fn pane_scrollbar_timer(server: &mut Server, id: PaneId) {
    if server.panes.get(id).is_some() {
        let _ = pane_scrollbar_hide(server, id);
    }
}

pub fn pane_full_size_offset(server: &Server, id: PaneId) -> Option<(i64, i64, u32, u32)> {
    let p = server.panes.get(id)?;
    let w = server.windows.get(p.window)?;
    let width = if pane_scrollbar_reserve(server, id) {
        p.scrollbar_style.width.wrapping_add(p.scrollbar_style.pad) as u32
    } else {
        0
    };
    Some((
        i64::from(p.xoff)
            - if w.sb_pos == PaneScrollbarPosition::Left {
                i64::from(width)
            } else {
                0
            },
        i64::from(p.yoff),
        p.sx.wrapping_add(width),
        p.sy,
    ))
}
pub fn pane_contains(server: &Server, id: PaneId, x: u32, y: u32) -> bool {
    if !pane_is_visible(server, id) {
        return false;
    }
    let Some((px, py, sx, sy)) = pane_full_size_offset(server, id) else {
        return false;
    };
    let (x, y) = (i64::from(x), i64::from(y));
    if !pane_is_floating(server, id) {
        x >= px && x <= px + i64::from(sx) && y >= py && y <= py + i64::from(sy)
    } else if pane_get_pane_lines(server, id) == 6 {
        x >= px && x < px + i64::from(sx) && y >= py && y < py + i64::from(sy)
    } else {
        x >= px - 1 && x <= px + i64::from(sx) && y >= py - 1 && y <= py + i64::from(sy)
    }
}
pub fn pane_floating_overlaps(server: &Server, floating: PaneId, other: PaneId) -> bool {
    if !pane_is_floating(server, floating) {
        return false;
    }
    let Some((fx, fy, fsx, fsy)) = pane_full_size_offset(server, floating) else {
        return false;
    };
    let Some((x, y, sx, sy)) = pane_full_size_offset(server, other) else {
        return false;
    };
    let border = i64::from(pane_get_pane_lines(server, floating) != 6);
    fx - border < x + i64::from(sx)
        && fx + i64::from(fsx) + border > x
        && fy - border < y + i64::from(sy)
        && fy + i64::from(fsy) + border > y
}

pub fn pane_update_focus(server: &mut Server, id: PaneId) {
    let Some(p) = server.panes.get(id) else {
        return;
    };
    if p.flags.contains(PaneFlags::EXITED) {
        return;
    }
    let focused = server
        .windows
        .get(p.window)
        .is_some_and(|w| w.active == Some(id) && w.focused && !w.menu_active);
    if focused == p.flags.contains(PaneFlags::FOCUSED) {
        return;
    }
    let window = p.window;
    if p.base.mode.contains(ScreenMode::FOCUSON) {
        server
            .panes
            .get_mut(id)
            .expect("resolved pane")
            .output
            .extend_from_slice(if focused { b"\x1b[I" } else { b"\x1b[O" });
    }
    server.emit(
        if focused {
            b"pane-focus-in"
        } else {
            b"pane-focus-out"
        },
        None,
        Some(window),
        Some(id),
    );
    if let Some(p) = server.panes.get_mut(id) {
        if focused {
            p.flags.insert(PaneFlags::FOCUSED);
        } else {
            p.flags.remove(PaneFlags::FOCUSED);
        }
    }
}

fn mode_changed(server: &mut Server, id: PaneId, previous: Option<Vec<u8>>, entered: bool) {
    let Some(p) = server.panes.get_mut(id) else {
        return;
    };
    p.flags
        .insert(PaneFlags::REDRAW | PaneFlags::REDRAWSCROLLBAR | PaneFlags::CHANGED);
    let current = p.modes.first().map(|m| m.name.clone());
    let window = p.window;
    crate::layout::fix_panes(server, window, None);
    window::effect(server, WindowEffect::Borders(window));
    window::effect(server, WindowEffect::Status(window));
    effect(
        server,
        PaneEffect::ModeChanged {
            pane: id,
            previous,
            current,
            entered,
        },
    );
    server.emit(
        if entered {
            b"pane-mode-entered"
        } else {
            b"pane-mode-exited"
        },
        None,
        Some(window),
        Some(id),
    );
    server.emit(b"pane-mode-changed", None, Some(window), Some(id));
}

pub fn pane_set_mode(
    server: &mut Server,
    id: PaneId,
    name: &[u8],
    flags: WindowModeFlags,
    driver: Rc<dyn PaneModeDriver>,
    kill: bool,
) -> Result<Option<ModeId>, ModelError> {
    let p = server.panes.get(id).ok_or(ModelError::StaleId)?;
    if p.modes.first().is_some_and(|m| m.name == name) {
        return Ok(None);
    }
    if !crate::tsp::broker::pane_cell_ready(server, id) {
        let name = name.to_vec();
        crate::tsp::broker::defer_cell_ui(
            server,
            id,
            Box::new(move |server| {
                if server.panes.get(id).is_some_and(|pane| {
                    !pane
                        .tsp
                        .as_ref()
                        .and_then(|state| state.switch.as_ref())
                        .is_some_and(|switch| switch.failed)
                }) {
                    let _ = pane_set_mode(server, id, &name, flags, driver, kill);
                }
            }),
        );
        return Ok(None);
    }
    if p.modes
        .first()
        .is_some_and(|m| m.flags.contains(WindowModeFlags::NO_STACK))
    {
        pane_reset_mode(server, id)?;
    }
    let Some(p) = server.panes.get_mut(id) else {
        return Ok(None);
    };
    let previous = p.modes.first().map(|m| m.name.clone());
    let mode = if let Some(position) = p.modes.iter().position(|m| m.name == name) {
        let mut mode = p.modes.remove(position);
        mode.kill = kill;
        let mode_id = mode.id;
        p.modes.insert(0, mode);
        mode_id
    } else {
        let mode_id = ModeId::new(id, p.next_mode_id, 0);
        p.next_mode_id = p
            .next_mode_id
            .checked_add(1)
            .ok_or(crate::ids::ArenaError::CapacityExhausted)?;
        p.modes.insert(
            0,
            PaneMode {
                id: mode_id,
                name: name.to_vec(),
                flags,
                prefix: 1,
                kill,
                screen: None,
                data: None,
                driver: Rc::clone(&driver),
            },
        );
        let mut screen = driver.init(server, mode_id);
        let Some(p) = server.panes.get_mut(id) else {
            if let Some(screen) = &mut screen {
                screen
                    .release(
                        &mut server.hyperlinks,
                        #[cfg(feature = "sixel")]
                        Some(&mut server.images),
                    )
                    .map_err(|e| ModelError::Sys(e.to_string()))?;
            }
            return Ok(None);
        };
        let Some(position) = p.modes.iter().position(|m| m.id == mode_id) else {
            if let Some(screen) = &mut screen {
                screen
                    .release(
                        &mut server.hyperlinks,
                        #[cfg(feature = "sixel")]
                        Some(&mut server.images),
                    )
                    .map_err(|e| ModelError::Sys(e.to_string()))?;
            }
            return Ok(None);
        };
        if screen.is_none() {
            p.modes.remove(position);
            return Ok(None);
        }
        p.modes[position].screen = screen;
        mode_id
    };
    mode_changed(server, id, previous, true);
    Ok(Some(mode))
}
pub fn pane_reset_mode(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    if p.modes.is_empty() {
        return Ok(());
    }
    let mode = p.modes.remove(0);
    let name = mode.name.clone();
    let kill = mode.kill;
    let driver = Rc::clone(&mode.driver);
    driver.free(server, mode);
    let Some(p) = server.panes.get_mut(id) else {
        return Ok(());
    };
    if p.modes.is_empty() {
        p.flags.remove(PaneFlags::UNSEENCHANGES);
    }
    let next = p.modes.first().map(|m| (m.id, Rc::clone(&m.driver)));
    let (sx, sy) = (p.sx, p.sy);
    if let Some((next, driver)) = next {
        driver.resize(server, next, sx, sy);
    }
    mode_changed(server, id, Some(name), false);
    if kill && server.panes.get(id).is_some() {
        effect(server, PaneEffect::Kill(id));
    }
    Ok(())
}
pub fn pane_reset_mode_all(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    while server.panes.get(id).is_some_and(|p| !p.modes.is_empty()) {
        pane_reset_mode(server, id)?;
    }
    Ok(())
}

pub fn pane_set_prompt(
    server: &mut Server,
    id: PaneId,
    mut prompt: PanePrompt,
) -> Result<(), ModelError> {
    pane_clear_prompt(server, id)?;
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    p.prompt_generation = p.prompt_generation.wrapping_add(1);
    let generation = p.prompt_generation;
    let window = p.window;
    prompt.engine.start(server, id);
    let Some(p) = server.panes.get_mut(id) else {
        prompt.engine.free(server);
        return Ok(());
    };
    if p.prompt_generation != generation || p.flags.contains(PaneFlags::DESTROYED) {
        prompt.engine.free(server);
        return Ok(());
    }
    let kind = prompt.kind;
    p.prompt = Some(prompt);
    p.flags.insert(PaneFlags::REDRAW);
    effect(server, PaneEffect::PromptChanged { pane: id, kind });
    server.emit(b"pane-prompt-opened", None, Some(window), Some(id));
    Ok(())
}
pub fn pane_clear_prompt(server: &mut Server, id: PaneId) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    p.prompt_generation = p.prompt_generation.wrapping_add(1);
    let Some(prompt) = p.prompt.take() else {
        return Ok(());
    };
    let notify = !p.flags.contains(PaneFlags::DESTROYED);
    let window = p.window;
    p.flags.insert(PaneFlags::REDRAW);
    let kind = prompt.kind;
    prompt.engine.free(server);
    if notify && server.panes.get(id).is_some() {
        effect(server, PaneEffect::PromptChanged { pane: id, kind });
        server.emit(b"pane-prompt-closed", None, Some(window), Some(id));
    }
    Ok(())
}
pub fn pane_update_prompt(
    server: &mut Server,
    id: PaneId,
    message: &[u8],
    input: &[u8],
) -> Result<(), ModelError> {
    pane_with_prompt(server, id, |server, prompt| {
        prompt.engine.update(server, message, input);
    });
    if let Some(p) = server.panes.get_mut(id) {
        if p.prompt.is_some() {
            p.flags.insert(PaneFlags::REDRAW);
        }
    }
    Ok(())
}
/// Run `f` with the pane prompt taken out of the pane so the engine may use
/// the server. The prompt is restored only if the pane and its prompt
/// generation survived; otherwise it is freed once.
pub fn pane_with_prompt(
    server: &mut Server,
    id: PaneId,
    f: impl FnOnce(&mut Server, &mut PanePrompt),
) {
    let Some(p) = server.panes.get_mut(id) else {
        return;
    };
    let Some(mut prompt) = p.prompt.take() else {
        return;
    };
    let generation = p.prompt_generation;
    f(server, &mut prompt);
    let valid = server.panes.get(id).is_some_and(|p| {
        p.prompt_generation == generation && !p.flags.contains(PaneFlags::DESTROYED)
    });
    if valid {
        if let Some(p) = server.panes.get_mut(id) {
            p.prompt = Some(prompt);
            return;
        }
    }
    prompt.engine.free(server);
}
pub fn pane_prompt_key(
    server: &mut Server,
    id: PaneId,
    client: ClientId,
    key: KeyCode,
    mouse: Option<&MouseEvent>,
    status_at_top: bool,
) -> Result<bool, ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let Some(mut prompt) = p.prompt.take() else {
        return Ok(false);
    };
    let generation = p.prompt_generation;
    let position = if key.is_mouse() {
        let Some(mouse) = mouse else {
            p.prompt = Some(prompt);
            return Ok(false);
        };
        let row = if status_at_top {
            0
        } else {
            p.sy.wrapping_sub(1)
        };
        let button = rmux_util::key::MouseButtonBits(mouse.b);
        if button.is_drag()
            || button.is_release()
            || mouse.b & 3 != 0
            || mouse.y as i64 - i64::from(p.yoff) != i64::from(row)
        {
            p.prompt = Some(prompt);
            return Ok(false);
        }
        Some((mouse.x.wrapping_sub(p.xoff as u32), p.sx))
    } else {
        None
    };
    let result = prompt.engine.key(server, id, client, key, position);
    let close = result == PromptKeyResult::Close || !prompt.engine.is_open();
    let valid = server.panes.get(id).is_some_and(|p| {
        p.prompt_generation == generation && !p.flags.contains(PaneFlags::DESTROYED)
    });
    if close || !valid {
        let window = server.panes.get(id).map(|p| p.window);
        let kind = prompt.kind;
        prompt.engine.free(server);
        if valid {
            if let Some(p) = server.panes.get_mut(id) {
                p.prompt_generation = p.prompt_generation.wrapping_add(1);
                p.flags.insert(PaneFlags::REDRAW);
            }
            effect(server, PaneEffect::PromptChanged { pane: id, kind });
            server.emit(b"pane-prompt-closed", None, window, Some(id));
        }
    } else if let Some(p) = server.panes.get_mut(id) {
        p.prompt = Some(prompt);
        p.flags.insert(PaneFlags::REDRAW);
    }
    Ok(result != PromptKeyResult::NotHandled)
}

fn synchronized_target(server: &Server, source: PaneId, target: PaneId) -> bool {
    target != source
        && server.panes.get(target).is_some_and(|p| {
            p.modes.is_empty()
                && p.fd.is_some()
                && !p.flags.contains(PaneFlags::INPUTOFF)
                && option_number(server, p.options, b"synchronize-panes", 0) != 0
        })
        && pane_is_visible(server, target)
}
pub fn pane_key(
    server: &mut Server,
    id: PaneId,
    client: Option<ClientId>,
    key: KeyCode,
    mouse: Option<&crate::client::ResolvedMouseEvent>,
    policy: &KeyPolicy,
) -> Result<(), ModelError> {
    if key.is_mouse() && mouse.is_none() {
        return Err(ModelError::message(b"mouse key without mouse event"));
    }
    let p = server.panes.get(id).ok_or(ModelError::StaleId)?;
    if let Some(mode) = p.modes.first() {
        if key.0 & KeyMasks::TYPE != (KeyCodeType::Mousemove as u64) << 32 {
            if let Some(client) = client {
                let mode_id = mode.id;
                let driver = Rc::clone(&mode.driver);
                driver.key(
                    server,
                    mode_id,
                    client,
                    KeyCode(key.0 & !KeyMasks::FLAGS),
                    mouse,
                );
            }
        }
        return Ok(());
    }
    if p.fd.is_none() || p.flags.contains(PaneFlags::INPUTOFF) {
        return Ok(());
    }
    encode_pane_key(server, id, client, key, mouse.map(|m| &m.event), policy)?;
    let p = server.panes.get(id).ok_or(ModelError::StaleId)?;
    if !key.is_mouse() && option_number(server, p.options, b"synchronize-panes", 0) != 0 {
        let window = p.window;
        let count = server
            .windows
            .get(window)
            .ok_or(ModelError::StaleId)?
            .panes
            .len();
        for index in 0..count {
            let target = server.windows.get(window).ok_or(ModelError::StaleId)?.panes[index];
            if synchronized_target(server, id, target) {
                let _ = encode_pane_key(server, target, client, key, None, policy);
            }
        }
    }
    Ok(())
}
/// Preview the exact pane encoding without retaining a second key buffer.
pub(crate) fn pane_key_len(
    server: &mut Server,
    id: PaneId,
    key: KeyCode,
    mouse: Option<&MouseEvent>,
    policy: &KeyPolicy,
) -> usize {
    let Some(p) = server.panes.get_mut(id) else {
        return 0;
    };
    if !p.modes.is_empty() || p.fd.is_none() || p.flags.contains(PaneFlags::INPUTOFF) {
        return 0;
    }
    let start = p.output.len();
    if let Some(mouse) = mouse.filter(|_| key.is_mouse()) {
        let x = mouse.x.wrapping_sub(p.xoff as u32);
        let y = mouse.y.wrapping_sub(p.yoff as u32);
        if x < p.sx && y < p.sy {
            if let Some(bytes) = encode_mouse(p.base.mode, mouse, x, y) {
                return bytes.as_ref().len();
            }
        }
        return 0;
    }
    let _ = encode_key(p.base.mode, key, policy, &mut p.output);
    let len = p.output.len() - start;
    p.output.truncate(start);
    len
}

/// Admit only the bounded prefix while switching; retain the remaining source
/// for the renderer commit rather than leaking it to the PTY or dropping it.
pub(crate) fn pane_input_bytes(
    server: &mut Server,
    id: PaneId,
    client: Option<ClientId>,
    bytes: &[u8],
) -> Result<(), ModelError> {
    let available = crate::tsp::input::input_available(server, id);
    let count = bytes.len().min(available);
    if count != 0 && !crate::tsp::broker::hold_bytes(server, id, client, &bytes[..count]) {
        server
            .panes
            .get_mut(id)
            .ok_or(ModelError::StaleId)?
            .output
            .extend_from_slice(&bytes[..count]);
    }
    if count < bytes.len() {
        let remaining = bytes[count..].to_vec();
        crate::tsp::input::defer_input(
            server,
            id,
            Box::new(move |server| {
                let _ = pane_input_bytes(server, id, client, &remaining);
            }),
        );
    }
    Ok(())
}

fn encode_pane_key(
    server: &mut Server,
    id: PaneId,
    client: Option<ClientId>,
    key: KeyCode,
    mouse: Option<&MouseEvent>,
    policy: &KeyPolicy,
) -> Result<(), ModelError> {
    let p = server.panes.get_mut(id).ok_or(ModelError::StaleId)?;
    let mode = p.base.mode;
    let start = p.output.len();
    if let Some(mouse) = mouse.filter(|_| key.is_mouse()) {
        let x = mouse.x.wrapping_sub(p.xoff as u32);
        let y = mouse.y.wrapping_sub(p.yoff as u32);
        if x < p.sx && y < p.sy {
            if let Some(bytes) = encode_mouse(mode, mouse, x, y) {
                p.output.extend_from_slice(bytes.as_ref());
            }
        }
        Ok(())
    } else {
        encode_key(mode, key, policy, &mut p.output)
            .map_err(|e| ModelError::Sys(format!("key encoding: {e:?}")))
    }?;
    hold_encoded_input(server, id, client, start);
    Ok(())
}
pub(crate) fn hold_encoded_input(
    server: &mut Server,
    id: PaneId,
    client: Option<ClientId>,
    start: usize,
) {
    let Some(pane) = server.panes.get_mut(id).filter(|pane| pane.tsp.is_some()) else {
        return;
    };
    if start == pane.output.len() {
        return;
    }
    let mut output = std::mem::take(&mut pane.output);
    let available = crate::tsp::input::input_available(server, id);
    let end = output.len().min(start.saturating_add(available));
    if crate::tsp::broker::hold_bytes(server, id, client, &output[start..end]) {
        if end < output.len() {
            let remaining = output[end..].to_vec();
            crate::tsp::input::defer_input(
                server,
                id,
                Box::new(move |server| {
                    let _ = pane_input_bytes(server, id, client, &remaining);
                }),
            );
        }
        output.truncate(start);
    }
    if let Some(pane) = server.panes.get_mut(id) {
        pane.output = output;
    }
}
pub fn pane_paste(
    server: &mut Server,
    id: PaneId,
    client: Option<ClientId>,
    key: KeyCode,
    bytes: &[u8],
) -> Result<(), ModelError> {
    let p = server.panes.get(id).ok_or(ModelError::StaleId)?;
    if !p.modes.is_empty()
        || p.fd.is_none()
        || p.flags.contains(PaneFlags::INPUTOFF)
        || (key.is_paste() && !p.displayed_screen().mode.contains(ScreenMode::BRACKETPASTE))
    {
        return Ok(());
    }
    let synchronize = option_number(server, p.options, b"synchronize-panes", 0) != 0;
    let window = p.window;
    pane_input_bytes(server, id, client, bytes)?;
    if synchronize {
        let count = server
            .windows
            .get(window)
            .ok_or(ModelError::StaleId)?
            .panes
            .len();
        for index in 0..count {
            let target = server.windows.get(window).ok_or(ModelError::StaleId)?.panes[index];
            if synchronized_target(server, id, target) {
                pane_input_bytes(server, target, client, bytes)?;
            }
        }
    }
    Ok(())
}

pub fn pane_start_input(
    server: &mut Server,
    id: PaneId,
    client: ClientId,
    item: QueueItemId,
    client_attached: bool,
    client_dead: bool,
) -> Result<Option<super::pane_input::PaneStdinInput>, ModelError> {
    let p = server.panes.get(id).ok_or(ModelError::StaleId)?;
    if !p.flags.contains(PaneFlags::EMPTY) {
        return Err(ModelError::message(b"pane is not empty"));
    }
    if client_dead || client_attached {
        return Ok(None);
    }
    server.effects.push_back(ModelEffect::Input(
        super::pane_input::InputAction::StdinStart {
            pane: id,
            client,
            item,
        },
    ));
    Ok(Some(super::pane_input::PaneStdinInput::new(
        id, client, item,
    )))
}

pub fn pane_search(
    server: &Server,
    id: PaneId,
    term: &[u8],
    regex: bool,
    ignore_case: bool,
) -> u32 {
    use rmux_sys::regex::{ExecFlags, PosixRegex, RegexFlags, RegexMatch};
    let Some(p) = server.panes.get(id) else {
        return 0;
    };
    let term = rmux_util::bytes::cstr(term);
    let compiled = if regex {
        let flags = RegexFlags::EXTENDED
            | if ignore_case {
                RegexFlags::ICASE
            } else {
                RegexFlags::NONE
            };
        match PosixRegex::new(term, flags) {
            Ok(r) => Some(r),
            Err(_) => return 0,
        }
    } else {
        None
    };
    let pattern = if regex {
        Vec::new()
    } else {
        let mut pattern = Vec::with_capacity(term.len() + 2);
        pattern.push(b'*');
        pattern.extend_from_slice(term);
        pattern.push(b'*');
        pattern
    };
    let mut captures = RegexMatch::new(0);
    for row in 0..p.base.grid.sy() {
        let mut line = p.base.grid.view_string_cells(0, row, p.base.grid.sx());
        if let Some(nul) = line.iter().position(|b| *b == 0) {
            line.truncate(nul);
        }
        while line
            .last()
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        {
            line.pop();
        }
        let found = if let Some(regex) = &compiled {
            regex
                .exec(&line, &mut captures, ExecFlags::NONE)
                .unwrap_or(false)
        } else {
            rmux_sys::fnmatch::fnmatch(
                &pattern,
                &line,
                if ignore_case {
                    rmux_sys::fnmatch::FnmatchFlags::CASEFOLD
                } else {
                    rmux_sys::fnmatch::FnmatchFlags::NONE
                },
            )
        };
        if found {
            return row + 1;
        }
    }
    0
}

pub fn pane_printable_flags(server: &Server, id: PaneId, out: &mut Vec<u8>) {
    let Some(p) = server.panes.get(id) else {
        return;
    };
    let Some(w) = server.windows.get(p.window) else {
        return;
    };
    if w.active == Some(id) {
        out.push(b'*');
    }
    if w.last.first() == Some(&id) {
        out.push(b'-');
    }
    if p.flags.contains(PaneFlags::ZOOMED) {
        out.push(b'Z');
    }
    if pane_is_floating(server, id) {
        out.push(b'F');
    }
    if p.flags.contains(PaneFlags::FLOATOVERZOOM) {
        out.push(b'A');
    }
    if w.modal == Some(id) {
        out.push(b'O');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pane(server: &mut Server) -> PaneId {
        let w = window::window_create(server, 10, 4, 0, 0).unwrap();
        window::window_add_pane(
            server,
            w,
            None,
            10,
            super::super::spawn::SpawnFlags::default(),
        )
        .unwrap()
    }
    #[test]
    fn exit_gate_and_wait_complete_once() {
        use crate::ids::ArenaId;
        let mut s = Server::default();
        let p = pane(&mut s);
        let item = QueueItemId::from_parts(7, 2);
        {
            let pane = s.panes.get_mut(p).unwrap();
            pane.wait_item = Some(item);
            pane.flags.insert(PaneFlags::EXITED);
        }
        assert!(!pane_destroy_ready(&s, p, true, 0));
        s.panes
            .get_mut(p)
            .unwrap()
            .flags
            .insert(PaneFlags::STATUSREADY);
        s.panes.get_mut(p).unwrap().status = 7 << 8;
        assert!(!pane_destroy_ready(&s, p, false, 0));
        assert!(!pane_destroy_ready(&s, p, true, 1));
        assert!(pane_destroy_ready(&s, p, true, 0));
        pane_wait_finish(&mut s, p);
        pane_wait_finish(&mut s, p);
        assert_eq!(
            s.effects
                .iter()
                .filter(|e| matches!(
                    e,
                    ModelEffect::Pane(PaneEffect::WaitFinished { retval: 7, .. })
                ))
                .count(),
            1
        );
    }
    #[test]
    fn resize_queue_and_saved_screen_reflow() {
        let mut s = Server::default();
        let p = pane(&mut s);
        pane_resize(&mut s, p, 10, 4).unwrap();
        assert!(s.panes.get(p).unwrap().resizes.is_empty());
        pane_resize(&mut s, p, 8, 3).unwrap();
        pane_resize(&mut s, p, 6, 2).unwrap();
        assert_eq!(
            s.panes.get(p).unwrap().resizes[0],
            PaneResize {
                osx: 10,
                osy: 4,
                sx: 8,
                sy: 3
            }
        );
        pane_clear_resizes(&mut s, p, Some(1)).unwrap();
        assert_eq!(s.panes.get(p).unwrap().resizes.len(), 1);
    }
    #[test]
    fn signed_containment_and_focus_reports() {
        let mut s = Server::default();
        let p = pane(&mut s);
        let w = s.panes.get(p).unwrap().window;
        {
            let p = s.panes.get_mut(p).unwrap();
            p.xoff = -3;
            p.yoff = -2;
            p.base.mode.insert(ScreenMode::FOCUSON);
        }
        assert!(pane_contains(&s, p, 0, 0));
        assert!(!pane_contains(&s, p, 8, 0));
        s.windows.get_mut(w).unwrap().active = Some(p);
        s.windows.get_mut(w).unwrap().focused = true;
        pane_update_focus(&mut s, p);
        pane_update_focus(&mut s, p);
        assert_eq!(s.panes.get(p).unwrap().output, b"\x1b[I");
        s.windows.get_mut(w).unwrap().menu_active = true;
        pane_update_focus(&mut s, p);
        assert_eq!(s.panes.get(p).unwrap().output, b"\x1b[I\x1b[O");
    }
    struct ModeDriver;
    impl PaneModeDriver for ModeDriver {
        fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
            let p = server.panes.get(id.owner)?;
            Screen::new(
                p.sx,
                p.sy,
                0,
                ScreenResetPolicy::default(),
                &mut server.hyperlinks,
            )
            .ok()
        }
        fn free(&self, server: &mut Server, mut mode: PaneMode) {
            if let Some(screen) = &mut mode.screen {
                screen
                    .release(
                        &mut server.hyperlinks,
                        #[cfg(feature = "sixel")]
                        None,
                    )
                    .unwrap();
            }
        }
        fn resize(&self, server: &mut Server, id: ModeId, sx: u32, sy: u32) {
            if let Some(mode) = server
                .panes
                .get_mut(id.owner)
                .and_then(|p| p.modes.iter_mut().find(|m| m.id == id))
            {
                if let Some(screen) = &mut mode.screen {
                    screen.resize(
                        sx,
                        sy,
                        false,
                        #[cfg(feature = "sixel")]
                        None,
                    );
                }
            }
        }
        fn key(
            &self,
            server: &mut Server,
            id: ModeId,
            _: ClientId,
            _: KeyCode,
            _: Option<&crate::client::ResolvedMouseEvent>,
        ) {
            pane_destroy(server, id.owner).unwrap();
        }
        fn append_output(
            &self,
            server: &mut Server,
            id: ModeId,
            bytes: &[u8],
        ) -> Result<(), ModelError> {
            let p = server.panes.get_mut(id.owner).ok_or(ModelError::StaleId)?;
            let mode = p
                .modes
                .iter_mut()
                .find(|m| m.id == id)
                .ok_or(ModelError::StaleId)?;
            let screen = mode.screen.as_mut().ok_or(ModelError::StaleId)?;
            screen
                .grid
                .view_set_cells(0, screen.cy, &rmux_emu::cell::DEFAULT_CELL, bytes);
            screen.cy = (screen.cy + 1).min(screen.grid.sy() - 1);
            Ok(())
        }
    }
    #[test]
    fn mode_stack_reuses_lower_mode_and_key_can_destroy_owner() {
        use crate::ids::ArenaId;
        let mut s = Server::default();
        let id = pane(&mut s);
        let driver: Rc<dyn PaneModeDriver> = Rc::new(ModeDriver);
        let first = pane_set_mode(
            &mut s,
            id,
            b"one",
            WindowModeFlags::default(),
            Rc::clone(&driver),
            false,
        )
        .unwrap()
        .unwrap();
        pane_set_mode(
            &mut s,
            id,
            b"two",
            WindowModeFlags::default(),
            Rc::clone(&driver),
            false,
        )
        .unwrap();
        assert_eq!(
            pane_set_mode(
                &mut s,
                id,
                b"one",
                WindowModeFlags::default(),
                Rc::clone(&driver),
                false
            )
            .unwrap(),
            Some(first)
        );
        assert_eq!(s.panes.get(id).unwrap().modes.len(), 2);
        pane_reset_mode(&mut s, id).unwrap();
        assert_eq!(s.panes.get(id).unwrap().modes[0].name, b"two");
        pane_key(
            &mut s,
            id,
            Some(ClientId::from_parts(0, 0)),
            KeyCode(b'a' as u64),
            None,
            &KeyPolicy::default(),
        )
        .unwrap();
        assert!(s.panes.get(id).is_none());
    }
    struct PromptEngine {
        open: bool,
        freed: Rc<std::cell::Cell<u32>>,
        destroy: bool,
    }
    impl PanePromptEngine for PromptEngine {
        fn start(&mut self, _: &mut Server, _: PaneId) {}
        fn key(
            &mut self,
            server: &mut Server,
            pane: PaneId,
            _: ClientId,
            _: KeyCode,
            _: Option<(u32, u32)>,
        ) -> PromptKeyResult {
            if self.destroy {
                pane_destroy(server, pane).unwrap();
            }
            PromptKeyResult::Handled
        }
        fn is_open(&self) -> bool {
            self.open
        }
        fn update(&mut self, _: &mut Server, _: &[u8], _: &[u8]) {}
        fn draw(
            &mut self,
            _: &mut Server,
            _: PaneId,
            _: &mut rmux_emu::screen::write::ScreenWriteCtx<'_>,
            _: &mut crate::ui::prompt::PromptDrawData<'_>,
        ) {
        }
        fn free(self: Box<Self>, _: &mut Server) {
            self.freed.set(self.freed.get() + 1);
        }
    }
    #[test]
    fn prompt_edit_keeps_open_and_destroy_callback_frees_once() {
        use crate::ids::ArenaId;
        let mut s = Server::default();
        let id = pane(&mut s);
        let freed = Rc::new(std::cell::Cell::new(0));
        let prompt = |destroy| PanePrompt {
            kind: PromptType::Command,
            engine: Box::new(PromptEngine {
                open: true,
                freed: Rc::clone(&freed),
                destroy,
            }),
        };
        pane_set_prompt(&mut s, id, prompt(false)).unwrap();
        let client = ClientId::from_parts(0, 0);
        assert!(pane_prompt_key(&mut s, id, client, KeyCode(b'a' as u64), None, false).unwrap());
        assert!(s.panes.get(id).unwrap().prompt.is_some());
        assert_eq!(freed.get(), 0);
        pane_set_prompt(&mut s, id, prompt(true)).unwrap();
        assert_eq!(freed.get(), 1);
        pane_prompt_key(&mut s, id, client, KeyCode(b'a' as u64), None, false).unwrap();
        assert!(s.panes.get(id).is_none());
        assert_eq!(freed.get(), 2);
    }
    #[test]
    fn visible_search_uses_glob_regex_casefold_and_whitespace_trim() {
        let mut s = Server::default();
        let id = pane(&mut s);
        s.panes.get_mut(id).unwrap().base.grid.view_set_cells(
            0,
            2,
            &rmux_emu::cell::DEFAULT_CELL,
            b"AbC    ",
        );
        assert_eq!(pane_search(&s, id, b"a?c", false, true), 3);
        assert_eq!(pane_search(&s, id, b"^AbC$", true, false), 3);
        assert_eq!(pane_search(&s, id, b"[", true, false), 0);
        assert_eq!(pane_search(&s, id, b"abc", false, false), 0);
    }
    #[test]
    fn stdin_start_requires_empty_and_eligible_client() {
        use crate::ids::ArenaId;
        let mut s = Server::default();
        let id = pane(&mut s);
        let client = ClientId::from_parts(0, 0);
        let item = QueueItemId::from_parts(0, 0);
        assert!(
            matches!(pane_start_input(&mut s, id, client, item, false, false), Err(ModelError::Message(bytes)) if bytes == b"pane is not empty")
        );
        s.panes.get_mut(id).unwrap().flags.insert(PaneFlags::EMPTY);
        assert!(
            pane_start_input(&mut s, id, client, item, true, false)
                .unwrap()
                .is_none()
        );
        assert!(
            pane_start_input(&mut s, id, client, item, false, true)
                .unwrap()
                .is_none()
        );
        assert!(
            pane_start_input(&mut s, id, client, item, false, false)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            s.effects
                .iter()
                .filter(|e| matches!(
                    e,
                    ModelEffect::Input(super::super::pane_input::InputAction::StdinStart { .. })
                ))
                .count(),
            1
        );
    }
    #[test]
    fn cfg_model_view_entry_and_line_use_installed_mode_driver() {
        let mut s = Server::default();
        let id = pane(&mut s);
        assert!(!s.pane_top_is_view(id));
        s.enter_view_mode(id, Rc::new(ModeDriver)).unwrap();
        assert!(s.pane_top_is_view(id));
        s.append_view_line(id, b"cfg cause").unwrap();
        assert_eq!(
            s.panes
                .get(id)
                .unwrap()
                .displayed_screen()
                .grid
                .view_string_cells(0, 0, 9),
            b"cfg cause"
        );
    }
    #[test]
    fn cursor_colour_uses_string_style_and_cursor_shape_choice() {
        let mut s = Server::default();
        let w = window::window_create(&mut s, 10, 4, 0, 0).unwrap();
        let options = s.windows.get(w).unwrap().options;
        let mut store = std::mem::take(&mut s.options);
        store.set_string(options, b"cursor-colour", false, b"#123456", &mut s);
        store.set_number_value(options, b"cursor-style", 3);
        s.options = store;
        let id = pane_create(&mut s, w, 10, 4, 0).unwrap();
        let screen = &s.panes.get(id).unwrap().base;
        assert_eq!(screen.default_ccolour, Colour::rgb(0x12, 0x34, 0x56));
        assert_eq!(
            screen.default_cstyle,
            rmux_emu::screen::ScreenCursorStyle::Underline
        );
    }
}
