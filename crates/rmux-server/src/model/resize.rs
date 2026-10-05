// Ported from tmux resize.c @ 8f25579c
use super::WindowSizePolicy;
use crate::client::ClientFlags;
use crate::ids::{ClientId, SessionId, WindowId};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowSize {
    pub sx: u32,
    pub sy: u32,
    pub xpixel: u32,
    pub ypixel: u32,
}
#[derive(Clone, Debug)]
pub struct ResizeClient {
    pub id: ClientId,
    pub session: Option<SessionId>,
    pub flags: ClientFlags,
    pub size: WindowSize,
    pub status_lines: u32,
    pub current: Option<WindowId>,
    pub windows: Vec<WindowId>,
    pub window_sizes: Vec<(WindowId, u32, u32)>,
}
#[derive(Clone, Copy, Debug)]
pub struct ResizeWindow {
    pub id: WindowId,
    pub manual: WindowSize,
    pub latest: Option<ClientId>,
}
impl ResizeClient {
    fn status_size(&self) -> u32 {
        if self.flags.contains(ClientFlags::STATUSOFF) {
            0
        } else {
            self.status_lines
        }
    }
}

pub fn ignore_client_size(client: &ResizeClient, clients: &[ResizeClient]) -> bool {
    client.session.is_none()
        || client.flags.intersects(ClientFlags::NOSIZEFLAGS)
        || (client.flags.contains(ClientFlags::IGNORESIZE)
            && clients.iter().any(|c| {
                c.session.is_some()
                    && !c.flags.intersects(ClientFlags::NOSIZEFLAGS)
                    && !c.flags.contains(ClientFlags::IGNORESIZE)
            }))
        || (client.flags.contains(ClientFlags::CONTROL)
            && !client
                .flags
                .intersects(ClientFlags::SIZECHANGED | ClientFlags::WINDOWSIZECHANGED))
}

pub fn clients_calculate_size(
    clients: &[ResizeClient],
    policy: WindowSizePolicy,
    creating: Option<ClientId>,
    window: Option<ResizeWindow>,
    mut skip: impl FnMut(&ResizeClient) -> bool,
) -> Option<WindowSize> {
    let mut size = match (policy, window) {
        (WindowSizePolicy::Largest, _) => WindowSize::default(),
        (WindowSizePolicy::Manual, Some(w)) => w.manual,
        _ => WindowSize {
            sx: u32::MAX,
            sy: u32::MAX,
            ..WindowSize::default()
        },
    };
    size.xpixel = 0;
    size.ypixel = 0;
    let multiple = window.is_some_and(|w| {
        clients
            .iter()
            .filter(|c| !ignore_client_size(c, clients) && c.windows.contains(&w.id))
            .take(2)
            .count()
            > 1
    });
    if policy != WindowSizePolicy::Manual {
        for c in clients {
            if Some(c.id) != creating && (ignore_client_size(c, clients) || skip(c)) {
                continue;
            }
            if policy == WindowSizePolicy::Latest
                && multiple
                && window.is_some_and(|w| Some(c.id) != w.latest)
            {
                continue;
            }
            let control = window
                .and_then(|w| c.window_sizes.iter().find(|s| s.0 == w.id))
                .filter(|s| s.1 != 0 && s.2 != 0);
            let (sx, sy) = control
                .map_or((c.size.sx, c.size.sy.wrapping_sub(c.status_size())), |s| {
                    (s.1, s.2)
                });
            if policy == WindowSizePolicy::Largest {
                size.sx = size.sx.max(sx);
                size.sy = size.sy.max(sy);
            } else {
                size.sx = size.sx.min(sx);
                size.sy = size.sy.min(sy);
            }
            if c.size.xpixel > size.xpixel && c.size.ypixel > size.ypixel {
                size.xpixel = c.size.xpixel;
                size.ypixel = c.size.ypixel;
            }
        }
    }
    if let Some(w) = window {
        for c in clients {
            if Some(c.id) != creating && (ignore_client_size(c, clients) || skip(c)) {
                continue;
            }
            if !c.flags.contains(ClientFlags::WINDOWSIZECHANGED) {
                continue;
            }
            if let Some((_, sx, sy)) = c.window_sizes.iter().find(|s| s.0 == w.id) {
                if *sx != 0 {
                    size.sx = size.sx.min(*sx);
                }
                if *sy != 0 {
                    size.sy = size.sy.min(*sy);
                }
            }
        }
    }
    let found = match policy {
        WindowSizePolicy::Manual => window.is_some(),
        WindowSizePolicy::Largest => size.sx != 0 && size.sy != 0,
        _ => size.sx != u32::MAX && size.sy != u32::MAX,
    };
    found.then_some(size)
}

pub fn default_window_size(
    clients: &[ResizeClient],
    creating: Option<ClientId>,
    session: SessionId,
    window: Option<ResizeWindow>,
    policy: WindowSizePolicy,
    default: &[u8],
) -> WindowSize {
    let client = creating.and_then(|id| clients.iter().find(|c| c.id == id));
    let mut size = if policy == WindowSizePolicy::Latest
        && client.is_some_and(|c| !ignore_client_size(c, clients))
    {
        let c = client.unwrap();
        WindowSize {
            sy: c.size.sy.wrapping_sub(c.status_size()),
            ..c.size
        }
    } else {
        let creating = client
            .filter(|c| !c.flags.contains(ClientFlags::CONTROL))
            .map(|c| c.id);
        clients_calculate_size(clients, policy, creating, window, |c| {
            window.map_or(c.session != Some(session), |w| !c.windows.contains(&w.id))
        })
        .unwrap_or_else(|| {
            let (sx, sy) = parse_default_size(default).unwrap_or((80, 24));
            WindowSize {
                sx,
                sy,
                ..WindowSize::default()
            }
        })
    };
    size.sx = size.sx.clamp(1, 10000);
    size.sy = size.sy.clamp(1, 10000);
    size
}

fn parse_default_size(bytes: &[u8]) -> Option<(u32, u32)> {
    fn number(bytes: &[u8], at: &mut usize) -> Option<u32> {
        while bytes.get(*at).is_some_and(u8::is_ascii_whitespace) {
            *at += 1;
        }
        let negative = bytes.get(*at) == Some(&b'-');
        if negative || bytes.get(*at) == Some(&b'+') {
            *at += 1;
        }
        let start = *at;
        let mut n = 0u32;
        while let Some(b'0'..=b'9') = bytes.get(*at) {
            n = n.wrapping_mul(10).wrapping_add((bytes[*at] - b'0') as u32);
            *at += 1;
        }
        if start == *at {
            None
        } else {
            Some(if negative { n.wrapping_neg() } else { n })
        }
    }
    let mut at = 0;
    let sx = number(bytes, &mut at)?;
    if bytes.get(at) != Some(&b'x') {
        return None;
    }
    at += 1;
    Some((sx, number(bytes, &mut at)?))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeDecision {
    Skip,
    Offset,
    Immediate(WindowSize),
    Deferred(WindowSize),
}
#[allow(clippy::too_many_arguments)]
pub fn recalculate_size(
    clients: &[ResizeClient],
    window: ResizeWindow,
    policy: WindowSizePolicy,
    aggressive: bool,
    active: bool,
    actual: WindowSize,
    pending: Option<WindowSize>,
    now: bool,
) -> ResizeDecision {
    if !active {
        return ResizeDecision::Skip;
    }
    let size = clients_calculate_size(clients, policy, None, Some(window), |c| {
        c.current.is_none()
            || if aggressive {
                c.current != Some(window.id)
            } else {
                !c.windows.contains(&window.id)
            }
    });
    let Some(size) = size else {
        return ResizeDecision::Offset;
    };
    let old = pending.unwrap_or(actual);
    if !now && old.sx == size.sx && old.sy == size.sy {
        ResizeDecision::Offset
    } else if now || policy == WindowSizePolicy::Manual {
        ResizeDecision::Immediate(size)
    } else {
        ResizeDecision::Deferred(size)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelUpdate {
    Keep,
    Default,
    Set(u32),
}
impl PixelUpdate {
    pub fn from_source(value: i32) -> Self {
        match value {
            -1 => Self::Keep,
            0 => Self::Default,
            _ => Self::Set(value as u32),
        }
    }
}
pub fn resize_window(
    server: &mut super::state::Server,
    id: WindowId,
    sx: u32,
    sy: u32,
    xpixel: PixelUpdate,
    ypixel: PixelUpdate,
) -> Result<(), super::state::ModelError> {
    let w = server
        .windows
        .get(id)
        .ok_or(super::state::ModelError::StaleId)?;
    let (old_sx, old_sy) = (w.sx, w.sy);
    let zoomed = super::window::window_zoomed_pane(server, id);
    if zoomed.is_some() {
        super::window::window_unzoom(server, id, true)?;
    }
    let (mut sx, mut sy) = (sx.clamp(1, 10000), sy.clamp(1, 10000));
    crate::layout::resize(server, id, sx, sy);
    if let Some(root) = server.windows.get(id).unwrap().layout_root {
        let geometry = server
            .layout_cells
            .get(root)
            .ok_or(super::state::ModelError::StaleId)?
            .g;
        sx = sx.max(geometry.sx);
        sy = sy.max(geometry.sy);
    }
    super::window::window_resize(server, id, sx, sy, xpixel, ypixel)?;
    if let Some(pane) = zoomed {
        if server.windows.get(id).unwrap().panes.contains(&pane) {
            super::window::window_zoom(server, id, pane)?;
        }
    }
    server
        .effects
        .push_back(super::state::ModelEffect::Resize(ResizeEffect::Offset(id)));
    server
        .effects
        .push_back(super::state::ModelEffect::Resize(ResizeEffect::Redraw(id)));
    server.emit(b"window-layout-changed", None, Some(id), None);
    server
        .effects
        .push_back(super::state::ModelEffect::Resize(ResizeEffect::Resized {
            window: id,
            old_sx,
            old_sy,
            sx,
            sy,
        }));
    server.emit(b"window-resized", None, Some(id), None);
    let w = server.windows.get_mut(id).unwrap();
    w.pending = None;
    w.flags.remove(super::WindowFlags::RESIZE);
    Ok(())
}

#[derive(Clone, Debug)]
pub enum ResizeEffect {
    Offset(WindowId),
    Redraw(WindowId),
    StatusCache(SessionId),
    Resized {
        window: WindowId,
        old_sx: u32,
        old_sy: u32,
        sx: u32,
        sy: u32,
    },
}
pub fn recalculate_window_size(
    server: &mut super::state::Server,
    clients: &[ResizeClient],
    id: WindowId,
    now: bool,
) -> Result<(), super::state::ModelError> {
    let w = server
        .windows
        .get(id)
        .ok_or(super::state::ModelError::StaleId)?;
    let policy =
        WindowSizePolicy::try_from(server.options.get_number(w.options, b"window-size") as i32)
            .expect("valid size policy");
    let input = ResizeWindow {
        id,
        manual: WindowSize {
            sx: w.manual_sx,
            sy: w.manual_sy,
            ..WindowSize::default()
        },
        latest: w.latest,
    };
    let decision = recalculate_size(
        clients,
        input,
        policy,
        server.options.get_number(w.options, b"aggressive-resize") != 0,
        w.active.is_some(),
        WindowSize {
            sx: w.sx,
            sy: w.sy,
            xpixel: w.xpixel,
            ypixel: w.ypixel,
        },
        w.pending,
        now,
    );
    match decision {
        ResizeDecision::Skip => {}
        ResizeDecision::Offset => server
            .effects
            .push_back(super::state::ModelEffect::Resize(ResizeEffect::Offset(id))),
        ResizeDecision::Deferred(size) => {
            let w = server.windows.get_mut(id).unwrap();
            w.pending = Some(size);
            w.flags.insert(super::WindowFlags::RESIZE);
            server
                .effects
                .push_back(super::state::ModelEffect::Resize(ResizeEffect::Offset(id)));
        }
        ResizeDecision::Immediate(size) => resize_window(
            server,
            id,
            size.sx,
            size.sy,
            if size.xpixel == 0 {
                PixelUpdate::Default
            } else {
                PixelUpdate::Set(size.xpixel)
            },
            if size.ypixel == 0 {
                PixelUpdate::Default
            } else {
                PixelUpdate::Set(size.ypixel)
            },
        )?,
    }
    Ok(())
}
pub fn recalculate_sizes(
    server: &mut super::state::Server,
    clients: &mut [ResizeClient],
    now: bool,
) -> Result<(), super::state::ModelError> {
    let sessions: Vec<_> = server.session_names.values().copied().collect();
    for session in sessions {
        server.sessions.get_mut(session).unwrap().attached = 0;
        server.effects.push_back(super::state::ModelEffect::Resize(
            ResizeEffect::StatusCache(session),
        ));
    }
    for i in 0..clients.len() {
        if let Some(session) = clients[i].session {
            if !clients[i].flags.intersects(ClientFlags::UNATTACHEDFLAGS) {
                server
                    .sessions
                    .get_mut(session)
                    .ok_or(super::state::ModelError::StaleId)?
                    .attached += 1;
            }
        }
        if ignore_client_size(&clients[i], clients) {
            continue;
        }
        if clients[i].size.sy <= clients[i].status_lines
            || clients[i].flags.contains(ClientFlags::CONTROL)
        {
            clients[i].flags.insert(ClientFlags::STATUSOFF);
        } else {
            clients[i].flags.remove(ClientFlags::STATUSOFF);
        }
    }
    let windows: Vec<_> = server.window_ids.values().copied().collect();
    for window in windows {
        recalculate_window_size(server, clients, window, now)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;
    fn client(n: u32, sx: u32, sy: u32, xp: u32, yp: u32) -> ResizeClient {
        ResizeClient {
            id: ClientId::from_parts(n, 0),
            session: Some(SessionId::from_parts(0, 0)),
            flags: ClientFlags::default(),
            size: WindowSize {
                sx,
                sy,
                xpixel: xp,
                ypixel: yp,
            },
            status_lines: 1,
            current: Some(WindowId::from_parts(0, 0)),
            windows: vec![WindowId::from_parts(0, 0)],
            window_sizes: vec![],
        }
    }
    #[test]
    fn independent_cells_paired_pixels() {
        let clients = [client(0, 100, 20, 10, 20), client(1, 80, 40, 20, 10)];
        let size =
            clients_calculate_size(&clients, WindowSizePolicy::Largest, None, None, |_| false)
                .unwrap();
        assert_eq!(
            size,
            WindowSize {
                sx: 100,
                sy: 39,
                xpixel: 10,
                ypixel: 20
            }
        );
        let size =
            clients_calculate_size(&clients, WindowSizePolicy::Smallest, None, None, |_| false)
                .unwrap();
        assert_eq!((size.sx, size.sy), (80, 19));
    }
    #[test]
    fn creation_override_and_global_ignore() {
        let mut clients = [client(0, 100, 20, 0, 0), client(1, 80, 40, 0, 0)];
        clients[0].flags = ClientFlags::IGNORESIZE;
        assert!(ignore_client_size(&clients[0], &clients));
        let size = clients_calculate_size(
            &clients,
            WindowSizePolicy::Largest,
            Some(clients[0].id),
            None,
            |_| true,
        )
        .unwrap();
        assert_eq!(size.sx, 100);
    }
    #[test]
    fn manual_control_clamp_and_latest() {
        let mut clients = [client(0, 100, 20, 0, 0), client(1, 80, 40, 0, 0)];
        let w = ResizeWindow {
            id: clients[0].windows[0],
            manual: WindowSize {
                sx: 120,
                sy: 50,
                ..WindowSize::default()
            },
            latest: Some(clients[1].id),
        };
        let size =
            clients_calculate_size(&clients, WindowSizePolicy::Latest, None, Some(w), |_| false)
                .unwrap();
        assert_eq!((size.sx, size.sy), (80, 39));
        clients[0].flags = ClientFlags::WINDOWSIZECHANGED;
        clients[0].window_sizes.push((w.id, 0, 10));
        let size =
            clients_calculate_size(&clients, WindowSizePolicy::Manual, None, Some(w), |_| false)
                .unwrap();
        assert_eq!((size.sx, size.sy), (120, 10));
    }
    #[test]
    fn default_scan_is_not_strict() {
        assert_eq!(
            parse_default_size(b" +90x-2suffix"),
            Some((90, u32::MAX - 1))
        );
        assert_eq!(parse_default_size(b"90 x2"), None);
    }
    #[test]
    fn control_requires_size_and_complete_override() {
        let mut c = client(0, 80, 24, 0, 0);
        c.flags = ClientFlags::CONTROL;
        assert!(ignore_client_size(&c, std::slice::from_ref(&c)));
        c.flags.insert(ClientFlags::WINDOWSIZECHANGED);
        let w = ResizeWindow {
            id: c.windows[0],
            manual: WindowSize::default(),
            latest: None,
        };
        c.window_sizes.push((w.id, 60, 0));
        let size = clients_calculate_size(
            std::slice::from_ref(&c),
            WindowSizePolicy::Smallest,
            None,
            Some(w),
            |_| false,
        )
        .unwrap();
        assert_eq!((size.sx, size.sy), (60, 23));
        c.window_sizes[0].2 = 10;
        let size =
            clients_calculate_size(&[c], WindowSizePolicy::Smallest, None, Some(w), |_| false)
                .unwrap();
        assert_eq!((size.sx, size.sy), (60, 10));
    }
    #[test]
    fn pending_cells_not_pixels_and_now_forces_resize() {
        let c = client(0, 80, 24, 20, 30);
        let w = ResizeWindow {
            id: c.windows[0],
            manual: WindowSize::default(),
            latest: None,
        };
        let actual = WindowSize {
            sx: 80,
            sy: 23,
            ..WindowSize::default()
        };
        assert_eq!(
            recalculate_size(
                std::slice::from_ref(&c),
                w,
                WindowSizePolicy::Smallest,
                false,
                true,
                actual,
                None,
                false
            ),
            ResizeDecision::Offset
        );
        assert!(matches!(
            recalculate_size(
                std::slice::from_ref(&c),
                w,
                WindowSizePolicy::Smallest,
                false,
                true,
                actual,
                None,
                true
            ),
            ResizeDecision::Immediate(_)
        ));
        let pending = WindowSize {
            sx: 80,
            sy: 23,
            ..WindowSize::default()
        };
        assert_eq!(
            recalculate_size(
                &[c],
                w,
                WindowSizePolicy::Smallest,
                false,
                true,
                WindowSize::default(),
                Some(pending),
                false
            ),
            ResizeDecision::Offset
        );
    }
}
