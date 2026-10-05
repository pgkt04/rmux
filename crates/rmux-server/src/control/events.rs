// Ported from tmux control-notify.c @ 8f25579c
use super::notify::{Notification, Recipient};
use crate::client::ClientFlags;
use crate::format::{FormatContext, FormatFlags, FormatTree};
use crate::ids::{ClientId, WindowId};
use crate::model::Server;
use crate::server::events::{self, EventPayload};

fn linked(server: &Server, client: ClientId, window: WindowId) -> bool {
    server
        .clients
        .get(client)
        .and_then(|c| c.session)
        .and_then(|s| server.sessions.get(s))
        .is_some_and(|s| {
            s.windows
                .values()
                .any(|id| server.winlinks.get(*id).is_some_and(|l| l.window == window))
        })
}
fn send(server: &mut Server, payload: &EventPayload, kind: usize) {
    let order: Vec<_> = server.client_order.iter().copied().collect();
    for id in order {
        let Some(c) = server.clients.get(id) else {
            continue;
        };
        if !c.flags.contains(ClientFlags::CONTROL)
            || c.flags.contains(ClientFlags::EXIT)
            || c.control.is_none()
        {
            continue;
        }
        let session = c.session;
        let subject = payload.get_client(b"client");
        let window = payload.get_window(b"window");
        let recipient = Recipient {
            eligible: true,
            attached: session.is_some(),
            linked: window.is_some_and(|w| linked(server, id, w)),
            is_subject: subject == Some(id),
            layout: None,
        };
        let pane = payload
            .get_pane(b"pane")
            .and_then(|p| server.panes.get(p))
            .map(|p| p.public_id);
        let fallback = payload.print(server, b"pane");
        let w = window.and_then(|w| server.windows.get(w));
        let public_window = w.map(|w| w.public_id);
        let session_subject = payload
            .get_session(b"session")
            .and_then(|s| server.sessions.get(s));
        let client_subject = subject.and_then(|c| server.clients.get(c));
        let name = payload.get_string(b"name");
        let event = match kind {
            0 => Notification::PaneMode {
                pane,
                fallback: fallback.as_ref().map(|s| s.as_ref()),
            },
            1 => {
                if !recipient.attached
                    || !recipient.linked
                    || w.is_none_or(|w| w.links.is_empty() || w.layout_root.is_none())
                {
                    continue;
                }
                let winlink = session.and_then(|s| server.sessions.get(s)).and_then(|s| {
                    s.windows.values().copied().find(|l| {
                        server
                            .winlinks
                            .get(*l)
                            .is_some_and(|l| Some(l.window) == window)
                    })
                });
                let mut tree = FormatTree::create(Some(id), None, 0, FormatFlags::NONE, server);
                tree.defaults(
                    server,
                    FormatContext {
                        evaluated_client: Some(id),
                        session,
                        winlink,
                        ..FormatContext::default()
                    },
                );
                let line = tree.expand(server,b"%layout-change #{window_id} #{window_layout} #{window_visible_layout} #{window_raw_flags}");
                tree.release(server);
                super::notify_write(server, id, &line);
                continue;
            }
            2 => Notification::WindowPane {
                window: public_window,
                pane: w
                    .and_then(|w| w.active)
                    .and_then(|p| server.panes.get(p))
                    .map(|p| p.public_id),
            },
            3 => Notification::WindowUnlinked(public_window),
            4 => Notification::WindowLinked(public_window),
            5 => Notification::WindowRenamed {
                window: public_window,
                name: w.map_or(b"", |w| w.name.as_slice()),
            },
            6 => {
                let s = client_subject
                    .and_then(|c| c.session)
                    .and_then(|s| server.sessions.get(s));
                Notification::ClientSession {
                    client: client_subject.and_then(|c| c.name.as_deref()),
                    session: s.map(|s| s.public_id),
                    name: s.map_or(b"", |s| s.name.as_slice()),
                }
            }
            7 => Notification::ClientDetached(client_subject.and_then(|c| c.name.as_deref())),
            8 => Notification::SessionRenamed {
                session: session_subject.map(|s| s.public_id),
                name: session_subject.map_or(b"", |s| s.name.as_slice()),
            },
            9 => Notification::SessionCreated,
            10 => Notification::SessionClosed,
            11 => Notification::SessionWindow {
                session: session_subject.map(|s| s.public_id),
                window: session_subject
                    .and_then(|s| s.current)
                    .and_then(|l| server.winlinks.get(l))
                    .and_then(|l| server.windows.get(l.window))
                    .map(|w| w.public_id),
            },
            12 => Notification::PasteChanged(name),
            13 => Notification::PasteDeleted(name),
            _ => unreachable!(),
        };
        // Format the line before borrowing client state; subjects remain leased.
        if let Some(line) = super::notify::render(&recipient, event) {
            super::notify_write(server, id, &line);
        }
    }
}
macro_rules! callbacks { ($($name:ident:$index:expr),*) => { $(fn $name(server:&mut Server,payload:&mut EventPayload) { send(server,payload,$index); })* }; }
callbacks!(pane_mode:0,layout:1,window_pane:2,unlinked:3,linked_event:4,renamed:5,client_session:6,detached:7,session_renamed:8,created:9,closed:10,session_window:11,paste_changed:12,paste_deleted:13);
pub fn build_events(server: &mut Server) {
    let callbacks: [events::EventCallback; 14] = [
        pane_mode,
        layout,
        window_pane,
        unlinked,
        linked_event,
        renamed,
        client_session,
        detached,
        session_renamed,
        created,
        closed,
        session_window,
        paste_changed,
        paste_deleted,
    ];
    for (name, callback) in super::notify::EVENT_NAMES.iter().zip(callbacks) {
        events::add_sink(server, name, callback);
    }
}
