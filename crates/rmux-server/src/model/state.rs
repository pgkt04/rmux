// Ported from tmux session.c, window.c, tmux.c, tmux.h @ 8f25579c
use super::{PaneFlags, SessionFlags, WindowFlags, WinlinkFlags};
use crate::ids::*;
use crate::options::{OptionsStore, environment::Environment};
use rmux_emu::{hyperlinks::HyperlinkRegistry, input::InputCtx, screen::Screen};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug)]
pub enum ModelError {
    StaleId,
    Message(Vec<u8>),
    Arena(ArenaError),
    Sys(String),
}
impl ModelError {
    pub fn message(bytes: &[u8]) -> Self {
        Self::Message(bytes.to_vec())
    }
}
impl From<ArenaError> for ModelError {
    fn from(e: ArenaError) -> Self {
        Self::Arena(e)
    }
}
impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(b) => write!(f, "{}", String::from_utf8_lossy(b)),
            other => write!(f, "{other:?}"),
        }
    }
}
impl std::error::Error for ModelError {}

pub struct Session {
    pub public_id: u32,
    pub name: Vec<u8>,
    pub cwd: Vec<u8>,
    pub created: (i64, i64),
    pub activity: (i64, i64),
    pub last_attached: (i64, i64),
    pub options: OptionsId,
    pub environment: Environment,
    pub termios: Option<rmux_sys::TermiosState>,
    pub windows: BTreeMap<i32, WinlinkId>,
    pub ordered_winlinks: Vec<WinlinkId>,
    pub current: Option<WinlinkId>,
    pub last: Vec<WinlinkId>,
    pub group: Option<SessionGroupId>,
    pub attached: u32,
    pub flags: SessionFlags,
    pub references: u32,
    pub lock_timer_initialized: bool,
    pub lock_timer_pending: bool,
}
pub struct SessionGroup {
    pub name: Vec<u8>,
    pub sessions: Vec<SessionId>,
}
pub struct Winlink {
    pub index: i32,
    pub session: SessionId,
    pub window: WindowId,
    pub flags: WinlinkFlags,
}
pub struct Window {
    pub public_id: u32,
    pub name: Vec<u8>,
    pub created: (i64, i64),
    pub activity: (i64, i64),
    pub options: OptionsId,
    pub panes: Vec<PaneId>,
    pub last: Vec<PaneId>,
    pub z_order: Vec<PaneId>,
    pub links: Vec<WinlinkId>,
    pub active: Option<PaneId>,
    pub modal: Option<PaneId>,
    pub modal_last: Option<PaneId>,
    pub previous_zoom: Option<PaneId>,
    pub sx: u32,
    pub sy: u32,
    pub xpixel: u32,
    pub ypixel: u32,
    pub manual_sx: u32,
    pub manual_sy: u32,
    pub pending: Option<super::resize::WindowSize>,
    pub latest: Option<ClientId>,
    pub flags: WindowFlags,
    pub references: u32,
    pub layout_root: Option<LayoutCellId>,
    pub saved_layout_root: Option<LayoutCellId>,
    pub lastlayout: Option<crate::layout::LayoutSetIndex>,
    pub old_layout: Option<rmux_util::bytes::ByteString>,
    pub last_new_pane_x: i32,
    pub last_new_pane_y: i32,
    pub sb: crate::ui::scrollbar::PaneScrollbarPolicy,
    pub sb_pos: crate::ui::scrollbar::PaneScrollbarPosition,
    pub name_time: (i64, i64),
    pub name_timer_pending: bool,
    pub focused: bool,
    pub menu_active: bool,
}
pub struct Pane {
    pub public_id: u32,
    pub window: WindowId,
    pub options: OptionsId,
    pub flags: PaneFlags,
    pub references: u32,
    pub sx: u32,
    pub sy: u32,
    pub xoff: i32,
    pub yoff: i32,
    pub active_point: u64,
    pub base: Screen,
    pub status_screen: Screen,
    pub parser: InputCtx,
    pub argv: Vec<Vec<u8>>,
    pub shell: Vec<u8>,
    pub cwd: Vec<u8>,
    pub tty: Vec<u8>,
    pub pid: Option<rmux_sys::ProcessId>,
    pub fd: Option<rmux_sys::OwnedFd>,
    pub status: i32,
    pub editor: Option<EditorId>,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
    pub base_offset: u64,
    pub parser_offset: u64,
    pub layout_cell: Option<LayoutCellId>,
    pub saved_layout_cell: Option<LayoutCellId>,
    pub scrollbar_style: rmux_emu::style::Style,
    pub dead_time: (i64, i64),
    pub last_output_time: i64,
    pub last_prompt_time: i64,
    pub cmd_start_time: i64,
    pub cmd_end_time: i64,
    pub output_generation: u64,
    pub cmd_status: i32,
    pub searchstr: Option<Vec<u8>>,
    pub palette: rmux_emu::colour::ColourPalette,
    pub resizes: VecDeque<super::pane::PaneResize>,
    pub wait_item: Option<QueueItemId>,
    pub modes: Vec<super::pane::PaneMode>,
    pub next_mode_id: u32,
    pub prompt: Option<super::pane::PanePrompt>,
    pub prompt_generation: u64,
    pub input_state: super::pane_input::PaneInputState,
    pub scrollbar_visible: bool,
    pub scrollbar_hover: bool,
    pub cached_gc: rmux_emu::cell::GridCell,
    pub cached_active_gc: rmux_emu::cell::GridCell,
    pub cached_dim: u32,
    pub cached_active_dim: u32,
}
impl Pane {
    pub fn has_fd(&self) -> bool {
        self.fd.is_some()
    }
    pub fn screen(&self) -> &Screen {
        self.displayed_screen()
    }
}
pub enum TimerRequest {
    Session(super::session::SessionTimerRequest),
}
pub enum ModelEffect {
    Event {
        name: Vec<u8>,
        session: Option<SessionId>,
        window: Option<WindowId>,
        pane: Option<PaneId>,
    },
    Session(super::session::SessionEffect),
    Timer(TimerRequest),
    Paste(super::paste::PasteEvent),
    Alert(super::alerts::AlertEffect),
    Spawn(super::spawn::SpawnEffect),
    Format(crate::format::runtime::FormatAction),
    Window(super::window::WindowEffect),
    Pane(super::pane::PaneEffect),
    Input(super::pane_input::InputAction),
    Resize(super::resize::ResizeEffect),
    RedrawWindow(WindowId),
    InvalidateScene(WindowId),
    Store(super::store_runtime::StoreEffect),
    RecalculateSizes,
}
pub type ModelEventCallback =
    fn(&mut Server, &[u8], Option<SessionId>, Option<WindowId>, Option<PaneId>);
pub struct Server {
    pub sessions: Arena<Session, SessionId>,
    pub groups: Arena<SessionGroup, SessionGroupId>,
    pub windows: Arena<Window, WindowId>,
    pub winlinks: Arena<Winlink, WinlinkId>,
    pub panes: Arena<Pane, PaneId>,
    pub session_names: BTreeMap<Vec<u8>, SessionId>,
    pub group_names: BTreeMap<Vec<u8>, SessionGroupId>,
    pub window_ids: BTreeMap<u32, WindowId>,
    pub pane_ids: BTreeMap<u32, PaneId>,
    pub next_session_id: u32,
    pub next_window_id: u32,
    pub next_pane_id: u32,
    pub next_active_point: u64,
    pub next_input_request: u64,
    pub next_command_group: u32,
    pub options: OptionsStore,
    pub hyperlinks: HyperlinkRegistry,
    pub layout_cells: crate::layout::Cells,
    pub paste: super::paste::PasteStore,
    pub monitors: Arena<super::monitor::MonitorSet, MonitorSetId>,
    pub editors: Arena<super::spawn::SpawnEditorState, EditorId>,
    pub alerts: super::alerts::AlertQueue,
    pub global_environment: Environment,
    pub socket_path: Vec<u8>,
    pub format_jobs: crate::format::jobs::FormatJobs,
    pub current_time: (i64, i64),
    pub marked_winlink: Option<WinlinkId>,
    pub marked_pane: Option<PaneId>,
    pub effects: VecDeque<ModelEffect>,
    pub model_event: Option<ModelEventCallback>,
    pub option_monitor_removed: Option<fn(&mut Server, HooksMonitorId)>,
    pub hooks: crate::cmd::hooks::HooksStore,
    pub hook_monitor_targets: BTreeMap<MonitorSetId, HooksMonitorId>,
    pub hook_monitor_dispatch: Option<super::store_runtime::HookMonitorDispatch>,
}
impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
impl Server {
    pub fn new() -> Self {
        let mut server = Self {
            sessions: Arena::new(),
            groups: Arena::new(),
            windows: Arena::new(),
            winlinks: Arena::new(),
            panes: Arena::new(),
            session_names: BTreeMap::new(),
            group_names: BTreeMap::new(),
            window_ids: BTreeMap::new(),
            pane_ids: BTreeMap::new(),
            next_session_id: 0,
            next_window_id: 0,
            next_pane_id: 0,
            next_active_point: 0,
            next_input_request: 0,
            next_command_group: 0,
            options: OptionsStore::new(),
            hyperlinks: HyperlinkRegistry::default(),
            layout_cells: Arena::new(),
            paste: super::paste::PasteStore::default(),
            monitors: Arena::new(),
            editors: Arena::new(),
            alerts: super::alerts::AlertQueue::default(),
            global_environment: Environment::default(),
            socket_path: Vec::new(),
            current_time: (0, 0),
            marked_winlink: None,
            marked_pane: None,
            effects: VecDeque::new(),
            format_jobs: crate::format::jobs::FormatJobs::new(),
            model_event: None,
            option_monitor_removed: None,
            hooks: crate::cmd::hooks::HooksStore::default(),
            hook_monitor_targets: BTreeMap::new(),
            hook_monitor_dispatch: None,
        };
        crate::format::runtime::initialize_defaults(&mut server);
        server
    }
    pub fn emit(
        &mut self,
        name: &[u8],
        session: Option<SessionId>,
        window: Option<WindowId>,
        pane: Option<PaneId>,
    ) {
        self.effects.push_back(ModelEffect::Event {
            name: name.to_vec(),
            session,
            window,
            pane,
        });
        if let Some(callback) = self.model_event {
            callback(self, name, session, window, pane);
        }
    }
    pub fn fire_paste_event(&mut self, event: &'static str, name: &[u8]) {
        self.effects
            .push_back(ModelEffect::Paste(super::paste::PasteEvent {
                event,
                name: name.into(),
            }));
        if let Some(callback) = self.model_event {
            callback(self, event.as_bytes(), None, None, None);
        }
    }
}
pub use rmux_util::shell::clean_name;
