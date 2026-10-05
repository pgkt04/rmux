// Ported from tmux server-client.c @ 8f25579c
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

//! Client registry queries: `server_client_how_many`,
//! `server_client_check_nested`, `server_client_get_cwd` and the
//! `ResizeClient` view that `recalculate_sizes` consumes
//! (`server-client.c:56-109,2945-2961`).
//!
//! `ModelView::client`/`clients` for `Server` already live in
//! `model/view.rs` (P7Server); no second `ClientModelView` adapter here.

use crate::client::ClientFlags;
use crate::ids::{ClientId, PaneId, SessionId};
use crate::model::pane_input::InputClient;
use crate::model::resize::{ResizeClient, WindowSize};
use crate::server::Server;
use rmux_tty::tty::TtyFlags;

/// The client loop of `input_add_request` (`input.c:3588-3600`) as data for
/// `pane_input::select_request_client`: every client in connection order
/// with `attached` = not `CLIENT_UNATTACHEDFLAGS`, whether its tty is
/// started, its activity time and whether its session links the pane's
/// window.
pub fn input_clients(server: &Server, pane: PaneId) -> Vec<InputClient> {
    let Some(window) = server.panes.get(pane).map(|p| p.window) else {
        return Vec::new();
    };
    server
        .client_order
        .iter()
        .filter_map(|id| server.clients.get(*id).map(|c| (*id, c)))
        .map(|(id, c)| InputClient {
            id,
            attached: !c.flags.intersects(ClientFlags::UNATTACHEDFLAGS),
            tty_started: c
                .tty
                .as_ref()
                .is_some_and(|tty| tty.flags().contains(TtyFlags::STARTED)),
            activity: c.activity_time,
            has_window: c
                .session
                .is_some_and(|s| crate::model::session::session_has(server, s, window)),
        })
        .collect()
}

/// `server_client_how_many` (`server-client.c:56-68`). The C test is
/// `~c->flags & CLIENT_UNATTACHEDFLAGS`: a client counts unless every one of
/// EXIT, DEAD and SUSPENDED is set.
pub fn how_many(server: &Server) -> u32 {
    server
        .client_order
        .iter()
        .filter_map(|id| server.clients.get(*id))
        .filter(|c| c.session.is_some() && !c.flags.contains(ClientFlags::UNATTACHEDFLAGS))
        .count() as u32
}

/// `server_client_check_nested` (`server-client.c:94-109`): true when the
/// client environment has a non-empty `RMUX` and any pane's tty equals the
/// client tty name (spec 2.9: rmux reads `RMUX`, never `TMUX`).
pub fn check_nested(server: &Server, id: ClientId) -> bool {
    let Some(c) = server.clients.get(id) else {
        return false;
    };
    let nested = c
        .environ
        .find(b"RMUX")
        .and_then(|entry| entry.value.as_ref())
        .is_some_and(|value| !value.is_empty());
    if !nested {
        return false;
    }
    let ttyname = c.ttyname.as_deref().unwrap_or(b"");
    server
        .pane_ids
        .values()
        .filter_map(|id| server.panes.get(*id))
        .any(|pane| pane.tty == ttyname)
}

/// `server_client_get_cwd` (`server-client.c:2945-2961`): the config
/// client's cwd while config loads, then an unattached client's cwd, then
/// the given session's, then the client session's, then home, then `/`.
pub fn get_cwd(server: &Server, c: Option<ClientId>, s: Option<SessionId>) -> Vec<u8> {
    if !server.cfg.finished
        && let Some(cfg_client) = server.cfg.client
        && let Some(cwd) = server.clients.get(cfg_client).and_then(|c| c.cwd.clone())
    {
        return cwd;
    }
    let client = c.and_then(|id| server.clients.get(id));
    if let Some(client) = client
        && client.session.is_none()
        && let Some(cwd) = &client.cwd
    {
        return cwd.clone();
    }
    if let Some(session) = s.and_then(|id| server.sessions.get(id)) {
        return session.cwd.clone();
    }
    if let Some(session) = client
        .and_then(|c| c.session)
        .and_then(|id| server.sessions.get(id))
    {
        return session.cwd.clone();
    }
    if let Some(home) = rmux_sys::proc::home_directory(None) {
        return home;
    }
    b"/".to_vec()
}

/// The per-client inputs of `recalculate_sizes` (`resize.c`): every live
/// client in connection order with its session, flags, tty size, status
/// line count, current window, the windows its session links, and any
/// control-mode window sizes.
pub fn resize_clients(server: &Server) -> Vec<ResizeClient> {
    let mut out = Vec::with_capacity(server.client_order.len());
    for id in &server.client_order {
        let Some(c) = server.clients.get(*id) else {
            continue;
        };
        if c.flags.contains(ClientFlags::DEAD) {
            continue;
        }
        let (sx, sy) = c.tty_size();
        let (xpixel, ypixel) = c.tty.as_ref().map_or((0, 0), |tty| tty.pixel_size());
        let session = c.session.and_then(|s| server.sessions.get(s));
        let current = session
            .and_then(|s| s.current)
            .and_then(|wl| server.winlinks.get(wl))
            .map(|wl| wl.window);
        let windows = session.map_or_else(Vec::new, |s| {
            s.windows
                .values()
                .filter_map(|wl| server.winlinks.get(*wl))
                .map(|wl| wl.window)
                .collect()
        });
        let window_sizes = c.control.as_ref().map_or_else(Vec::new, |control| {
            control
                .windows
                .iter()
                .filter_map(|(public_id, (w, h))| {
                    server.window_ids.get(public_id).map(|id| (*id, *w, *h))
                })
                .collect()
        });
        out.push(ResizeClient {
            id: *id,
            session: c.session,
            flags: c.flags,
            size: WindowSize {
                sx,
                sy,
                xpixel,
                ypixel,
            },
            status_lines: crate::ui::status::status_line_size(server, *id),
            current,
            windows,
            window_sizes,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Client;
    use crate::model::session::{self, SessionCreate};
    use crate::options::environment::{Environment, EnvironmentFlags};

    fn session(server: &mut Server, name: &[u8], cwd: &[u8]) -> SessionId {
        let options = server.options.create(Some(server.options.global_s));
        session::session_create(
            server,
            SessionCreate {
                prefix: None,
                name: Some(name.to_vec()),
                cwd: cwd.to_vec(),
                environment: Environment::default(),
                options,
                termios: None,
            },
        )
    }

    fn client(server: &mut Server, cwd: Option<&[u8]>, session: Option<SessionId>) -> ClientId {
        let mut c = Client::new(None, (0, 0));
        c.cwd = cwd.map(<[u8]>::to_vec);
        c.session = session;
        let id = server.clients.insert(c).unwrap();
        server.clients.retain(id).unwrap();
        server.client_order.push_back(id);
        id
    }

    #[test]
    fn get_cwd_precedence() {
        let mut server = Server::new();
        server.cfg.finished = true;
        let s1 = session(&mut server, b"one", b"/one");
        let s2 = session(&mut server, b"two", b"/two");
        let unattached = client(&mut server, Some(b"/client"), None);
        let attached = client(&mut server, Some(b"/client"), Some(s1));

        // Unattached client cwd wins over any session.
        assert_eq!(get_cwd(&server, Some(unattached), Some(s2)), b"/client");
        // Attached client: explicit session first, then the client session.
        assert_eq!(get_cwd(&server, Some(attached), Some(s2)), b"/two");
        assert_eq!(get_cwd(&server, Some(attached), None), b"/one");
        // No client: the session.
        assert_eq!(get_cwd(&server, None, Some(s2)), b"/two");
        // Nothing: home, else "/".
        let fallback = get_cwd(&server, None, None);
        assert_eq!(
            fallback,
            rmux_sys::proc::home_directory(None).unwrap_or_else(|| b"/".to_vec())
        );

        // Config client cwd while config is loading.
        server.cfg.finished = false;
        server.cfg.client = Some(unattached);
        assert_eq!(get_cwd(&server, Some(attached), Some(s2)), b"/client");
        // ... but only when the config client has a cwd.
        server.clients.get_mut(unattached).unwrap().cwd = None;
        assert_eq!(get_cwd(&server, Some(attached), Some(s2)), b"/two");
    }

    #[test]
    fn how_many_counts_attached_live_clients() {
        let mut server = Server::new();
        let s = session(&mut server, b"s", b"/");
        client(&mut server, None, None);
        let a = client(&mut server, None, Some(s));
        let b = client(&mut server, None, Some(s));
        assert_eq!(how_many(&server), 2);
        // `~flags & UNATTACHEDFLAGS` stays nonzero until all three bits are set.
        server
            .clients
            .get_mut(a)
            .unwrap()
            .flags
            .insert(ClientFlags::DEAD);
        assert_eq!(how_many(&server), 2);
        server
            .clients
            .get_mut(a)
            .unwrap()
            .flags
            .insert(ClientFlags::EXIT | ClientFlags::SUSPENDED);
        assert_eq!(how_many(&server), 1);
        server.clients.get_mut(b).unwrap().session = None;
        assert_eq!(how_many(&server), 0);
    }

    #[test]
    fn check_nested_needs_rmux_and_matching_pane_tty() {
        let mut server = Server::new();
        let id = client(&mut server, None, None);
        {
            let c = server.clients.get_mut(id).unwrap();
            c.ttyname = Some(b"/dev/ttys001".to_vec());
        }
        assert!(!check_nested(&server, id));
        server.clients.get_mut(id).unwrap().environ.set(
            b"RMUX",
            EnvironmentFlags::default(),
            b"/tmp/rmux-1/default,1,0",
        );
        assert!(!check_nested(&server, id));
        let window = crate::model::window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = crate::model::window::window_add_pane(
            &mut server,
            window,
            None,
            0,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        server.panes.get_mut(pane).unwrap().tty = b"/dev/ttys001".to_vec();
        assert!(check_nested(&server, id));
        server
            .clients
            .get_mut(id)
            .unwrap()
            .environ
            .set(b"RMUX", EnvironmentFlags::default(), b"");
        assert!(!check_nested(&server, id));
    }
}
