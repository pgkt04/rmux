// Ported from tmux names.c, monitor.c, sort.c, hooks.c, events-payload.c @ 8f25579c
use super::state::{ModelEffect, ModelError, Server};
use super::view::ClientModelView;
use super::{monitor, names};
use crate::format::runtime::{FormatExternal, ServerFormatRuntime};
use crate::format::sort::{SortModel, TimeVal};
use crate::format::{FormatContext, FormatFlags, FormatTagFlags, FormatTree};
use crate::ids::{ClientId, MonitorSetId, PaneId, PasteBufferId, SessionId, WindowId, WinlinkId};

pub type HookMonitorDispatch =
    fn(&mut Server, crate::ids::HooksMonitorId, &crate::cmd::hooks::MonitorChange<'_>);

pub trait HooksModelRuntime {
    fn create_hook_monitor_set(
        &mut self,
        session: SessionId,
        hook: crate::ids::HooksMonitorId,
    ) -> Result<MonitorSetId, ModelError>;
    fn destroy_hook_monitor_set(&mut self, set: MonitorSetId) -> Result<(), ModelError>;
    fn add_hook_model_monitor(
        &mut self,
        set: MonitorSetId,
        spec: monitor::MonitorSpec,
    ) -> Result<(), ModelError>;
    fn hook_model_monitor_stats(&self, set: MonitorSetId, name: &[u8]) -> (u32, i64);
    fn hook_monitor_formats(
        &self,
        change: &crate::cmd::hooks::MonitorChange<'_>,
        clients: Option<&dyn ClientModelView>,
    ) -> std::collections::BTreeMap<rmux_util::bytes::ByteString, rmux_util::bytes::ByteString>;
}

fn hook_monitor_callback(
    server: &mut Server,
    set: MonitorSetId,
    change: &monitor::MonitorChange<'_>,
) {
    let Some(hook) = server.hook_monitor_targets.get(&set).copied() else {
        return;
    };
    let Some(dispatch) = server.hook_monitor_dispatch else {
        return;
    };
    let change = crate::cmd::hooks::MonitorChange {
        name: change.name,
        winlink: change.context.winlink,
        pane: change.context.pane,
        session: change.context.session,
        client: change.context.client,
        value: Some(change.value),
        last: change.last,
    };
    dispatch(server, hook, &change);
}

impl HooksModelRuntime for Server {
    fn create_hook_monitor_set(
        &mut self,
        session: SessionId,
        hook: crate::ids::HooksMonitorId,
    ) -> Result<MonitorSetId, ModelError> {
        if self.hook_monitor_dispatch.is_none() {
            return Err(ModelError::message(
                b"hook monitor dispatcher is not installed",
            ));
        }
        let set = monitor::monitor_create_session(self, Some(session), hook_monitor_callback)?;
        self.hook_monitor_targets.insert(set, hook);
        Ok(set)
    }
    fn destroy_hook_monitor_set(&mut self, set: MonitorSetId) -> Result<(), ModelError> {
        self.destroy_monitor(set)
    }
    fn add_hook_model_monitor(
        &mut self,
        set: MonitorSetId,
        spec: monitor::MonitorSpec,
    ) -> Result<(), ModelError> {
        self.add_monitor(set, spec)
    }
    fn hook_model_monitor_stats(&self, set: MonitorSetId, name: &[u8]) -> (u32, i64) {
        (
            self.monitor_fire_count(set, name),
            self.monitor_fire_time(set, name),
        )
    }
    fn hook_monitor_formats(
        &self,
        change: &crate::cmd::hooks::MonitorChange<'_>,
        clients: Option<&dyn ClientModelView>,
    ) -> std::collections::BTreeMap<rmux_util::bytes::ByteString, rmux_util::bytes::ByteString>
    {
        let mut formats = std::collections::BTreeMap::new();
        if let Some(client) = change.client.and_then(|id| clients?.client(id)) {
            formats.insert("client".into(), client.name.into());
        }
        let link = change.winlink.and_then(|id| self.winlinks.get(id));
        if let Some(session) = change
            .session
            .or_else(|| link.map(|wl| wl.session))
            .and_then(|id| self.sessions.get(id))
        {
            formats.insert("session".into(), format!("${}", session.public_id).into());
            formats.insert("session_name".into(), session.name.clone().into());
        }
        let pane = change.pane.and_then(|id| self.panes.get(id));
        if let Some(window) = link
            .map(|wl| wl.window)
            .or_else(|| pane.map(|p| p.window))
            .and_then(|id| self.windows.get(id))
        {
            formats.insert("window".into(), format!("@{}", window.public_id).into());
            formats.insert("window_name".into(), window.name.clone().into());
        }
        if let Some(link) = link {
            formats.insert("window_index".into(), link.index.to_string().into());
        }
        if let Some(pane) = pane {
            formats.insert("pane".into(), format!("%{}", pane.public_id).into());
        }
        formats
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreEffect {
    NameTimer {
        window: WindowId,
        delay_usec: Option<u64>,
    },
    MonitorTimer {
        set: MonitorSetId,
        pending: bool,
    },
    Borders(WindowId),
    Status(WindowId),
}

#[derive(Default)]
pub struct StoreRuntime<'a> {
    pub clients: Option<&'a dyn ClientModelView>,
    effects: Vec<StoreEffect>,
}

fn expand(
    server: &mut Server,
    context: FormatContext,
    flags: FormatFlags,
    tag: u32,
    format: &[u8],
) -> Vec<u8> {
    let mut tree = FormatTree::create(None, None, tag, flags, server);
    tree.defaults(server, context);
    let value = tree.expand(server, format).into_vec();
    tree.release(server);
    value
}

impl StoreRuntime<'_> {
    pub fn apply(&mut self, server: &mut Server) {
        server
            .effects
            .extend(self.effects.drain(..).map(ModelEffect::Store));
    }
}

impl names::NamesRuntime for StoreRuntime<'_> {
    fn automatic_rename(&self, server: &Server, window: WindowId) -> bool {
        server
            .windows
            .get(window)
            .and_then(|w| server.options.get(w.options, b"automatic-rename"))
            .is_some_and(|(_, entry)| entry.value().as_number().unwrap_or(0) != 0)
    }
    fn expand_name(&mut self, server: &mut Server, window: WindowId, pane: PaneId) -> Vec<u8> {
        let Some(w) = server.windows.get(window) else {
            return Vec::new();
        };
        let format = server
            .options
            .get(w.options, b"automatic-rename-format")
            .map(|(_, entry)| entry.to_string(None, true))
            .unwrap_or_else(|| "#{pane_current_command}".into());
        let tag = FormatTagFlags::WINDOW.bits() | w.public_id;
        expand(
            server,
            FormatContext {
                window: Some(window),
                pane: Some(pane),
                ..FormatContext::default()
            },
            FormatFlags::NONE,
            tag,
            &format,
        )
    }
    fn name_timer(&mut self, window: WindowId, delay_usec: Option<u64>) {
        self.effects
            .push(StoreEffect::NameTimer { window, delay_usec });
    }
    fn redraw_name(&mut self, window: WindowId) {
        self.effects.push(StoreEffect::Borders(window));
        self.effects.push(StoreEffect::Status(window));
    }
}

fn monitor_context(server: &Server, context: monitor::MonitorContext) -> FormatContext {
    FormatContext {
        evaluated_client: context.client,
        session: context.session,
        winlink: context.winlink,
        window: context
            .winlink
            .and_then(|id| server.winlinks.get(id).map(|wl| wl.window)),
        pane: context.pane,
        ..FormatContext::default()
    }
}

impl monitor::MonitorRuntime for StoreRuntime<'_> {
    fn client_session(&self, client: ClientId) -> Option<SessionId> {
        self.clients?.client(client)?.session
    }
    fn expand(
        &mut self,
        server: &mut Server,
        context: monitor::MonitorContext,
        format: &[u8],
    ) -> Vec<u8> {
        expand(
            server,
            monitor_context(server, context),
            FormatFlags::NOJOBS,
            0,
            format,
        )
    }
    fn timer(&mut self, set: MonitorSetId, pending: bool) {
        self.effects
            .push(StoreEffect::MonitorTimer { set, pending });
    }
}

/// G15 lends live clients; G14/G15 lend the full format facts without another client store.
pub struct ExternalMonitorRuntime<'a, E> {
    pub clients: &'a dyn ClientModelView,
    pub external: &'a mut E,
    effects: Vec<StoreEffect>,
}
impl<'a, E: FormatExternal> ExternalMonitorRuntime<'a, E> {
    pub fn new(clients: &'a dyn ClientModelView, external: &'a mut E) -> Self {
        Self {
            clients,
            external,
            effects: Vec::new(),
        }
    }
    pub fn apply(&mut self, server: &mut Server) {
        server
            .effects
            .extend(self.effects.drain(..).map(ModelEffect::Store));
    }
}
impl<E: FormatExternal> monitor::MonitorRuntime for ExternalMonitorRuntime<'_, E> {
    fn client_session(&self, client: ClientId) -> Option<SessionId> {
        self.clients.client(client)?.session
    }
    fn expand(
        &mut self,
        server: &mut Server,
        context: monitor::MonitorContext,
        format: &[u8],
    ) -> Vec<u8> {
        let context = monitor_context(server, context);
        let mut runtime = ServerFormatRuntime {
            server,
            external: self.external,
        };
        let mut tree = FormatTree::create(None, None, 0, FormatFlags::NOJOBS, &mut runtime);
        tree.defaults(&mut runtime, context);
        let value = tree.expand(&mut runtime, format).into_vec();
        tree.release(&mut runtime);
        value
    }
    fn timer(&mut self, set: MonitorSetId, pending: bool) {
        self.effects
            .push(StoreEffect::MonitorTimer { set, pending });
    }
}

impl Server {
    pub fn check_window_name(&mut self, window: WindowId) -> Result<(), ModelError> {
        let mut runtime = StoreRuntime::default();
        let result = names::check_window_name(self, window, &mut runtime);
        runtime.apply(self);
        result
    }
    pub fn check_monitors(
        &mut self,
        id: MonitorSetId,
        clients: Option<&dyn ClientModelView>,
    ) -> Result<(), ModelError> {
        let mut runtime = StoreRuntime {
            clients,
            effects: Vec::new(),
        };
        let result = monitor::monitor_check(self, id, &mut runtime);
        runtime.apply(self);
        result
    }
    pub fn check_monitors_external<E: FormatExternal>(
        &mut self,
        id: MonitorSetId,
        clients: &dyn ClientModelView,
        external: &mut E,
    ) -> Result<(), ModelError> {
        let mut runtime = ExternalMonitorRuntime::new(clients, external);
        let result = monitor::monitor_check(self, id, &mut runtime);
        runtime.apply(self);
        result
    }
    pub fn create_session_monitor(
        &mut self,
        session: Option<SessionId>,
        callback: monitor::MonitorCallback,
    ) -> Result<MonitorSetId, ModelError> {
        monitor::monitor_create_session(self, session, callback)
    }
    pub fn create_client_monitor(
        &mut self,
        client: ClientId,
        callback: monitor::MonitorCallback,
    ) -> Result<MonitorSetId, ModelError> {
        monitor::monitor_create_client(self, client, callback)
    }
    pub fn monitor_fire_count(&self, id: MonitorSetId, name: &[u8]) -> u32 {
        monitor::monitor_get_fire_count(self, id, name)
    }
    pub fn monitor_fire_time(&self, id: MonitorSetId, name: &[u8]) -> i64 {
        monitor::monitor_get_fire_time(self, id, name)
    }
    pub fn add_monitor(
        &mut self,
        id: MonitorSetId,
        spec: monitor::MonitorSpec,
    ) -> Result<(), ModelError> {
        let mut runtime = StoreRuntime::default();
        let result = monitor::monitor_add(self, id, spec, &mut runtime);
        runtime.apply(self);
        result
    }
    pub fn remove_monitor(&mut self, id: MonitorSetId, name: &[u8]) -> Result<(), ModelError> {
        let mut runtime = StoreRuntime::default();
        let result = monitor::monitor_remove(self, id, name, &mut runtime);
        runtime.apply(self);
        result
    }
    pub fn destroy_monitor(&mut self, id: MonitorSetId) -> Result<(), ModelError> {
        let mut runtime = StoreRuntime::default();
        let result = monitor::monitor_destroy(self, id, &mut runtime);
        self.hook_monitor_targets.remove(&id);
        runtime.apply(self);
        result
    }
}

impl SortModel for Server {
    fn sessions(&self, out: &mut Vec<SessionId>) {
        out.extend(self.session_names.values().copied());
    }
    fn winlinks(&self, session: SessionId, out: &mut Vec<WinlinkId>) {
        if let Some(s) = self.sessions.get(session) {
            out.extend(s.windows.values().copied());
        }
    }
    fn panes(&self, window: WindowId, out: &mut Vec<PaneId>) {
        if let Some(w) = self.windows.get(window) {
            out.extend(w.panes.iter().copied());
        }
    }
    fn winlink_window(&self, id: WinlinkId) -> Option<WindowId> {
        Some(self.winlinks.get(id)?.window)
    }
    fn buffers(&self, out: &mut Vec<PasteBufferId>) {
        let mut previous = None;
        while let Some(id) = self.paste.walk(previous) {
            out.push(id);
            previous = Some(id);
        }
    }
    fn session_public_id(&self, id: SessionId) -> u32 {
        self.sessions.get(id).map_or(0, |s| s.public_id)
    }
    fn session_created(&self, id: SessionId) -> TimeVal {
        self.sessions.get(id).map_or((0, 0), |s| s.created)
    }
    fn session_activity(&self, id: SessionId) -> TimeVal {
        self.sessions.get(id).map_or((0, 0), |s| s.activity)
    }
    fn session_name(&self, id: SessionId) -> &[u8] {
        self.sessions.get(id).map_or(b"", |s| &s.name)
    }
    fn winlink_index(&self, id: WinlinkId) -> i32 {
        self.winlinks.get(id).map_or(0, |wl| wl.index)
    }
    fn window_created(&self, id: WindowId) -> TimeVal {
        self.windows.get(id).map_or((0, 0), |w| w.created)
    }
    fn window_activity(&self, id: WindowId) -> TimeVal {
        self.windows.get(id).map_or((0, 0), |w| w.activity)
    }
    fn window_name(&self, id: WindowId) -> &[u8] {
        self.windows.get(id).map_or(b"", |w| &w.name)
    }
    fn window_size(&self, id: WindowId) -> (u32, u32) {
        self.windows.get(id).map_or((0, 0), |w| (w.sx, w.sy))
    }
    fn pane_active_point(&self, id: PaneId) -> u64 {
        self.panes.get(id).map_or(0, |p| p.active_point)
    }
    fn pane_public_id(&self, id: PaneId) -> u32 {
        self.panes.get(id).map_or(0, |p| p.public_id)
    }
    fn pane_size(&self, id: PaneId) -> (u32, u32) {
        self.panes.get(id).map_or((0, 0), |p| (p.sx, p.sy))
    }
    fn pane_index(&self, id: PaneId) -> u32 {
        super::pane::pane_index(self, id).unwrap_or(0)
    }
    fn pane_zindex(&self, id: PaneId) -> u32 {
        super::pane::pane_z_index(self, id).unwrap_or(0)
    }
    fn pane_title(&self, id: PaneId) -> &[u8] {
        self.panes.get(id).map_or(b"", |p| &p.screen().title)
    }
    fn buffer_name(&self, id: PasteBufferId) -> &[u8] {
        self.paste.get(id).map_or(b"", |b| b.name.as_bytes())
    }
    fn buffer_order(&self, id: PasteBufferId) -> u32 {
        self.paste.get(id).map_or(0, |b| b.order)
    }
    fn buffer_size(&self, id: PasteBufferId) -> usize {
        self.paste.get(id).map_or(0, |b| b.data.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hook_dispatch(
        server: &mut Server,
        hook: crate::ids::HooksMonitorId,
        change: &crate::cmd::hooks::MonitorChange<'_>,
    ) {
        let set = server
            .hook_monitor_targets
            .iter()
            .find_map(|(set, id)| (*id == hook).then_some(*set))
            .unwrap();
        assert_eq!(server.hook_model_monitor_stats(set, change.name).0, 1);
        assert_eq!(change.value, Some(b"value".as_slice()));
        assert_eq!(
            server.hook_monitor_formats(change, None)[b"session".as_slice()],
            b"$0".as_slice()
        );
        server.destroy_hook_monitor_set(set).unwrap();
    }
    #[test]
    fn hook_model_bridge_dispatches_synchronously_and_releases_binding() {
        use crate::ids::ArenaId;
        let mut server = Server::new();
        let options = server.options.create(None);
        let session = super::super::session::session_create(
            &mut server,
            super::super::session::SessionCreate {
                name: Some(b"session".to_vec()),
                prefix: None,
                cwd: Vec::new(),
                options,
                environment: crate::options::environment::Environment::default(),
                termios: None,
            },
        );
        let hook = crate::ids::HooksMonitorId::from_parts(0, 0);
        assert!(server.create_hook_monitor_set(session, hook).is_err());
        server.hook_monitor_dispatch = Some(hook_dispatch);
        let set = server.create_hook_monitor_set(session, hook).unwrap();
        let mut spec = monitor::monitor_parse(b"item::value").unwrap();
        spec.flags = monitor::MonitorFlags::INITIAL;
        server.add_hook_model_monitor(set, spec).unwrap();
        server.check_monitors(set, None).unwrap();
        assert!(server.monitors.get(set).is_none());
        assert!(!server.hook_monitor_targets.contains_key(&set));
        assert_eq!(server.sessions.get(session).unwrap().references, 1);
    }
    #[test]
    fn monitor_format_no_jobs_and_timer_effects() {
        let mut server = Server::new();
        let window = super::super::window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = super::super::pane::pane_create(&mut server, window, 80, 24, 0).unwrap();
        let mut runtime = StoreRuntime::default();
        let value = monitor::MonitorRuntime::expand(
            &mut runtime,
            &mut server,
            monitor::MonitorContext {
                pane: Some(pane),
                ..monitor::MonitorContext::default()
            },
            b"#{pane_id}:#(echo forbidden)",
        );
        assert_eq!(
            value,
            format!("%{}:", server.panes.get(pane).unwrap().public_id).as_bytes()
        );
        assert!(!server.effects.iter().any(|effect| matches!(
            effect,
            ModelEffect::Format(crate::format::runtime::FormatAction::Job { .. })
        )));
        fn callback(_: &mut Server, _: MonitorSetId, _: &monitor::MonitorChange<'_>) {}
        let id = monitor::monitor_create_session(&mut server, None, callback).unwrap();
        server
            .add_monitor(id, monitor::monitor_parse(b"item::f").unwrap())
            .unwrap();
        assert!(
            matches!(server.effects.back(), Some(ModelEffect::Store(StoreEffect::MonitorTimer { set, pending: true })) if *set == id)
        );
        server.remove_monitor(id, b"item").unwrap();
        assert!(
            matches!(server.effects.back(), Some(ModelEffect::Store(StoreEffect::MonitorTimer { set, pending: false })) if *set == id)
        );
    }
    #[test]
    fn sort_reads_real_store_metadata_and_pane_title() {
        let mut server = Server::new();
        let first = super::super::paste::paste_add(&mut server, None, vec![1], 10)
            .unwrap()
            .unwrap();
        let second = super::super::paste::paste_set(&mut server, vec![2, 3], Some(b"named"), 10)
            .unwrap()
            .unwrap();
        let mut buffers = Vec::new();
        SortModel::buffers(&server, &mut buffers);
        assert_eq!(buffers, [second, first]);
        assert_eq!(SortModel::buffer_name(&server, second), b"named");
        assert_eq!(SortModel::buffer_size(&server, second), 2);
        let window = super::super::window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = super::super::pane::pane_create(&mut server, window, 80, 24, 0).unwrap();
        server.panes.get_mut(pane).unwrap().base.title = b"actual".to_vec();
        assert_eq!(SortModel::pane_title(&server, pane), b"actual");
        assert_eq!(SortModel::window_size(&server, window), (80, 24));
    }
}
