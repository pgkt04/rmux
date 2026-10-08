// Ported from tmux control.c, monitor.c @ 8f25579c
use crate::format::{FormatContext, FormatFlags, FormatTree};
use crate::ids::{ClientId, MonitorSetId, SessionId};
use crate::model::{
    Server,
    monitor::{self, MonitorChange, MonitorContext, MonitorFlags, MonitorRuntime, MonitorSpec},
};
use crate::server::event_loop::LoopAction;
use std::{collections::BTreeMap, time::Duration};
pub struct MonitorAdapter {
    sessions: BTreeMap<ClientId, SessionId>,
    changes: Vec<(MonitorSetId, bool)>,
}
impl MonitorAdapter {
    pub fn new(server: &Server) -> Self {
        Self {
            sessions: server
                .client_order
                .iter()
                .filter_map(|id| {
                    server
                        .clients
                        .get(*id)
                        .and_then(|c| c.session)
                        .map(|s| (*id, s))
                })
                .collect(),
            changes: Vec::new(),
        }
    }
    pub fn apply(self, server: &mut Server) {
        for (set, pending) in self.changes {
            let key = format!("monitor:{set:?}").into_bytes();
            if let Some(timer) = server.runtime_timers.remove(&key) {
                server.event_loop.cancel(timer);
            }
            if pending {
                let timer = server
                    .event_loop
                    .schedule(Duration::from_secs(1), LoopAction::ControlMonitor(set));
                server.runtime_timers.insert(key, timer);
            }
        }
    }
}
impl MonitorRuntime for MonitorAdapter {
    fn client_session(&self, client: ClientId) -> Option<SessionId> {
        self.sessions.get(&client).copied()
    }
    fn expand(&mut self, server: &mut Server, c: MonitorContext, format: &[u8]) -> Vec<u8> {
        let mut tree = FormatTree::create(c.client, None, 0, FormatFlags::NOJOBS, server);
        tree.defaults(
            server,
            FormatContext {
                evaluated_client: c.client,
                session: c.session,
                winlink: c.winlink,
                pane: c.pane,
                ..FormatContext::default()
            },
        );
        let value = tree.expand(server, format).0;
        tree.release(server);
        value
    }
    fn timer(&mut self, set: MonitorSetId, pending: bool) {
        self.changes.push((set, pending));
    }
}
pub fn changed(server: &mut Server, _set: MonitorSetId, change: &MonitorChange<'_>) {
    let (Some(client), Some(session)) = (change.context.client, change.context.session) else {
        return;
    };
    let Some(s) = server.sessions.get(session) else {
        return;
    };
    let session = s.public_id;
    let winlink = change
        .context
        .winlink
        .and_then(|l| server.winlinks.get(l))
        .and_then(|l| server.windows.get(l.window).map(|w| (w.public_id, l.index)));
    let pane = change
        .context
        .pane
        .and_then(|p| server.panes.get(p))
        .map(|p| p.public_id);
    if let Some(state) = server
        .clients
        .get_mut(client)
        .and_then(|c| c.control.as_mut())
    {
        super::notify::subscription(state, change.name, session, winlink, pane, change.value);
    }
    super::runtime::sync(server, client);
}
pub fn add_sub(server: &mut Server, client: ClientId, mut spec: MonitorSpec) {
    let Some(set) = server
        .clients
        .get(client)
        .and_then(|c| c.control.as_ref())
        .and_then(|s| s.monitors)
    else {
        return;
    };
    spec.flags.insert(MonitorFlags::INITIAL);
    let mut runtime = MonitorAdapter::new(server);
    monitor::monitor_add(server, set, spec, &mut runtime).expect("live control monitor set");
    monitor::monitor_check_tsp(server, set, &mut runtime).expect("live control monitor set");
    runtime.apply(server);
}
pub fn remove_sub(server: &mut Server, client: ClientId, name: &[u8]) {
    let Some(set) = server
        .clients
        .get(client)
        .and_then(|c| c.control.as_ref())
        .and_then(|s| s.monitors)
    else {
        return;
    };
    let mut runtime = MonitorAdapter::new(server);
    monitor::monitor_remove(server, set, name, &mut runtime).expect("live control monitor set");
    runtime.apply(server);
}
pub fn monitor_timer(server: &mut Server, set: MonitorSetId) {
    server
        .runtime_timers
        .remove(&format!("monitor:{set:?}").into_bytes());
    if server.monitors.get(set).is_none() {
        return;
    }
    let mut runtime = MonitorAdapter::new(server);
    monitor::monitor_check(server, set, &mut runtime).expect("live monitor timer set");
    runtime.apply(server);
}

pub(crate) fn check_tsp(server: &mut Server) {
    let sets: Vec<_> = server
        .client_order
        .iter()
        .filter_map(|id| {
            let set = server.clients.get(*id)?.control.as_ref()?.monitors?;
            server.monitors.get(set)?.has_tsp_view.then_some(set)
        })
        .collect();
    if sets.is_empty() {
        return;
    }
    let mut runtime = MonitorAdapter::new(server);
    for set in sets {
        if server.monitors.get(set).is_some() {
            monitor::monitor_check_tsp(server, set, &mut runtime)
                .expect("live control monitor set");
        }
    }
    runtime.apply(server);
}
