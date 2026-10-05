// Ported from tmux cmd-find.c, session.c, window.c @ 8f25579c
use super::state::Server;
use crate::{cmd::find::*, ids::*};
impl ModelView for Server {
    fn session(&self, id: SessionId) -> Option<SessionView<'_>> {
        let s = self.sessions.get(id)?;
        Some(SessionView {
            public_id: s.public_id,
            name: &s.name,
            alive: self.session_names.get(&s.name) == Some(&id),
            attached: s.attached,
            activity: s.activity,
            current: s.current,
            winlinks: &s.ordered_winlinks,
            last: s.last.first().copied(),
        })
    }
    fn winlink(&self, id: WinlinkId) -> Option<WinlinkView> {
        let l = self.winlinks.get(id)?;
        Some(WinlinkView {
            session: l.session,
            window: l.window,
            index: l.index,
        })
    }
    fn window(&self, id: WindowId) -> Option<WindowView<'_>> {
        let w = self.windows.get(id)?;
        Some(WindowView {
            public_id: w.public_id,
            name: &w.name,
            active: w.active,
            panes: &w.panes,
            last: w.last.first().copied(),
            modal: w.modal,
            pane_base_index: self.options.get_number(w.options, b"pane-base-index") as i32,
        })
    }
    fn pane(&self, id: PaneId) -> Option<PaneView<'_>> {
        let p = self.panes.get(id)?;
        if p.flags.contains(super::PaneFlags::DESTROYED) {
            return None;
        }
        Some(PaneView {
            public_id: p.public_id,
            window: p.window,
            tty: &p.tty,
            live_tty: p.has_fd(),
        })
    }
    fn client(&self, _id: ClientId) -> Option<ClientView<'_>> {
        None
    }
    fn sessions(&self, v: &mut dyn FnMut(SessionId)) {
        for id in self.session_names.values() {
            v(*id);
        }
    }
    fn windows(&self, v: &mut dyn FnMut(WindowId)) {
        for id in self.window_ids.values() {
            v(*id);
        }
    }
    fn panes(&self, v: &mut dyn FnMut(PaneId)) {
        for id in self.pane_ids.values() {
            v(*id);
        }
    }
    fn clients(&self, _v: &mut dyn FnMut(ClientId)) {}
    fn marked(&self) -> Option<CmdFindState> {
        let wl = self.marked_winlink?;
        let l = self.winlinks.get(wl)?;
        let pane = self.marked_pane?;
        let state = CmdFindState {
            flags: CmdFindFlags::default(),
            s: Some(l.session),
            wl: Some(wl),
            w: Some(l.window),
            wp: Some(pane),
            idx: l.index,
        };
        state.is_valid(self).then_some(state)
    }
    fn adjacent_pane(&self, pane: PaneId, direction: PaneDirection) -> Option<PaneId> {
        super::window::pane_direction(self, pane, direction)
    }
    fn pane_description(&self, window: WindowId, description: &[u8]) -> Option<PaneId> {
        super::window::pane_description(self, window, description)
    }
}
/// Client ownership belongs to G15; this adapter lends its live views without a second client arena.
pub trait ClientModelView {
    fn client(&self, id: ClientId) -> Option<ClientView<'_>>;
    fn clients(&self, visit: &mut dyn FnMut(ClientId));
}
pub struct ModelWithClients<'a> {
    pub server: &'a Server,
    pub clients: &'a dyn ClientModelView,
}
impl ModelView for ModelWithClients<'_> {
    fn session(&self, id: SessionId) -> Option<SessionView<'_>> {
        self.server.session(id)
    }
    fn winlink(&self, id: WinlinkId) -> Option<WinlinkView> {
        self.server.winlink(id)
    }
    fn window(&self, id: WindowId) -> Option<WindowView<'_>> {
        self.server.window(id)
    }
    fn pane(&self, id: PaneId) -> Option<PaneView<'_>> {
        self.server.pane(id)
    }
    fn client(&self, id: ClientId) -> Option<ClientView<'_>> {
        self.clients.client(id)
    }
    fn sessions(&self, v: &mut dyn FnMut(SessionId)) {
        self.server.sessions(v)
    }
    fn windows(&self, v: &mut dyn FnMut(WindowId)) {
        self.server.windows(v)
    }
    fn panes(&self, v: &mut dyn FnMut(PaneId)) {
        self.server.panes(v)
    }
    fn clients(&self, v: &mut dyn FnMut(ClientId)) {
        self.clients.clients(v)
    }
    fn marked(&self) -> Option<CmdFindState> {
        self.server.marked()
    }
    fn adjacent_pane(&self, pane: PaneId, direction: PaneDirection) -> Option<PaneId> {
        self.server.adjacent_pane(pane, direction)
    }
    fn pane_description(&self, window: WindowId, description: &[u8]) -> Option<PaneId> {
        self.server.pane_description(window, description)
    }
}
impl Server {
    pub fn first_session_by_name(&self) -> Option<SessionId> {
        self.session_names.first_key_value().map(|(_, id)| *id)
    }
    pub fn session_attached(&self, id: SessionId) -> bool {
        self.sessions.get(id).is_some_and(|s| s.attached != 0)
    }
    pub fn session_active_pane(&self, id: SessionId) -> Option<PaneId> {
        let s = self.sessions.get(id)?;
        let l = self.winlinks.get(s.current?)?;
        self.windows.get(l.window)?.active
    }
    pub fn session_options(&self, id: SessionId) -> Option<OptionsId> {
        self.sessions.get(id).map(|s| s.options)
    }
    pub fn pane_options(&self, id: PaneId) -> Option<OptionsId> {
        self.panes.get(id).map(|p| p.options)
    }
    pub fn window_options(&self, state: &CmdFindState) -> Option<OptionsId> {
        self.windows.get(state.w?).map(|w| w.options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{pane, session, window};
    #[test]
    fn command_view_resolves_graph_and_rejects_destroyed_pane() {
        let mut server = Server::new();
        let options = server.options.create(Some(server.options.global_s));
        let session = session::session_create(
            &mut server,
            session::SessionCreate {
                prefix: None,
                name: Some(b"s".to_vec()),
                cwd: b"/".to_vec(),
                environment: crate::options::environment::Environment::default(),
                options,
                termios: None,
            },
        );
        let window = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = window::window_add_pane(
            &mut server,
            window,
            None,
            0,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        let link = session::session_attach(&mut server, session, window, 0).unwrap();
        session::session_set_current(&mut server, session, Some(link));
        let state = CmdFindState {
            s: Some(session),
            wl: Some(link),
            w: Some(window),
            wp: Some(pane),
            idx: 0,
            ..CmdFindState::default()
        };
        assert!(state.is_valid(&server));
        server.marked_winlink = Some(link);
        server.marked_pane = Some(pane);
        assert_eq!(server.marked(), Some(state));
        pane::pane_destroy(&mut server, pane).unwrap();
        assert!(!state.is_valid(&server));
        assert!(server.marked().is_none());
    }
}
