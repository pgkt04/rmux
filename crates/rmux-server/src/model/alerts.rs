// Ported from tmux alerts.c, tmux.h @ 8f25579c
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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlertPolicy {
    None = 0,
    Any = 1,
    Current = 2,
    Other = 3,
}
impl TryFrom<i32> for AlertPolicy {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Any),
            2 => Ok(Self::Current),
            3 => Ok(Self::Other),
            _ => Err(value),
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VisualPolicy {
    Off = 0,
    On = 1,
    Both = 2,
}
impl TryFrom<i32> for VisualPolicy {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::On),
            2 => Ok(Self::Both),
            _ => Err(value),
        }
    }
}

use super::{WindowFlags, WinlinkFlags};
use crate::ids::{SessionId, WindowId, WinlinkId};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertKind {
    Bell,
    Activity,
    Silence,
}
impl AlertKind {
    pub const fn flag(self) -> WindowFlags {
        match self {
            Self::Bell => WindowFlags::BELL,
            Self::Activity => WindowFlags::ACTIVITY,
            Self::Silence => WindowFlags::SILENCE,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bell => "Bell",
            Self::Activity => "Activity",
            Self::Silence => "Silence",
        }
    }
}

/// The graph adapter owns records, timer tokens and client delivery. Delivery
/// suppresses control clients and maps visual policy to bell/status or both.
pub trait AlertHost {
    fn window_flags(&self, window: WindowId) -> WindowFlags;
    fn set_window_flags(&mut self, window: WindowId, flags: WindowFlags);
    fn monitor(&self, window: WindowId, kind: AlertKind) -> bool;
    fn reset_silence_timer(&mut self, window: WindowId);
    fn retain_window(&mut self, window: WindowId);
    fn release_window(&mut self, window: WindowId);
    fn defer_alert_check(&mut self);
    fn links(&self, window: WindowId) -> Vec<WinlinkId>;
    fn session(&self, link: WinlinkId) -> SessionId;
    fn link_flags(&self, link: WinlinkId) -> WinlinkFlags;
    fn mark_link(&mut self, link: WinlinkId, flag: WinlinkFlags);
    fn current(&self, link: WinlinkId) -> bool;
    fn attached(&self, session: SessionId) -> bool;
    fn set_alerted(&mut self, session: SessionId, alerted: bool);
    fn alerted(&self, session: SessionId) -> bool;
    fn action(&self, session: SessionId, kind: AlertKind) -> AlertPolicy;
    fn status(&mut self, session: SessionId);
    fn hook(&mut self, link: WinlinkId, kind: AlertKind);
    fn message(&mut self, link: WinlinkId, kind: AlertKind);
}

#[derive(Debug, Default)]
pub struct AlertQueue {
    pending: VecDeque<WindowId>,
    fired: bool,
}
impl AlertQueue {
    pub fn is_pending(&self, window: WindowId) -> bool {
        self.pending.contains(&window)
    }
    pub fn reset(host: &mut impl AlertHost, window: WindowId) {
        let mut flags = host.window_flags(window);
        flags.remove(WindowFlags::SILENCE);
        host.set_window_flags(window, flags);
        host.reset_silence_timer(window);
    }
    pub fn reset_all(host: &mut impl AlertHost, windows: impl IntoIterator<Item = WindowId>) {
        for window in windows {
            Self::reset(host, window);
        }
    }
    pub fn queue(&mut self, host: &mut impl AlertHost, window: WindowId, flags: WindowFlags) {
        Self::reset(host, window);
        let mut pending = host.window_flags(window);
        pending.insert(flags);
        host.set_window_flags(window, pending);
        if ![AlertKind::Bell, AlertKind::Activity, AlertKind::Silence]
            .into_iter()
            .any(|kind| flags.contains(kind.flag()) && host.monitor(window, kind))
        {
            return;
        }
        if !self.is_pending(window) {
            self.pending.push_back(window);
            host.retain_window(window);
        }
        if !self.fired {
            self.fired = true;
            host.defer_alert_check();
        }
    }
    pub fn silence_expired(&mut self, host: &mut impl AlertHost, window: WindowId) {
        self.queue(host, window, WindowFlags::SILENCE);
    }
    pub fn dispatch(&mut self, host: &mut impl AlertHost) {
        while let Some(window) = self.pending.front().copied() {
            check_window(host, window);
            self.pending.pop_front();
            let mut flags = host.window_flags(window);
            flags.remove(WindowFlags::ALERTFLAGS);
            host.set_window_flags(window, flags);
            host.release_window(window);
        }
        self.fired = false;
    }
}

pub fn check_session(host: &mut impl AlertHost, windows: impl IntoIterator<Item = WindowId>) {
    for window in windows {
        check_window(host, window);
    }
}
pub fn check_window(host: &mut impl AlertHost, window: WindowId) {
    for kind in [AlertKind::Bell, AlertKind::Activity, AlertKind::Silence] {
        if !host.window_flags(window).contains(kind.flag()) || !host.monitor(window, kind) {
            continue;
        }
        let links = host.links(window);
        for &link in &links {
            let session = host.session(link);
            host.set_alerted(session, false);
        }
        let flag = WinlinkFlags::from_bits_retain(kind.flag().bits());
        for link in links {
            if kind != AlertKind::Bell && host.link_flags(link).contains(flag) {
                continue;
            }
            let session = host.session(link);
            let current = host.current(link);
            if !current || !host.attached(session) {
                host.mark_link(link, flag);
                host.status(session);
            }
            let applies = match host.action(session, kind) {
                AlertPolicy::Any => true,
                AlertPolicy::Current => current,
                AlertPolicy::Other => !current,
                AlertPolicy::None => false,
            };
            if !applies {
                continue;
            }
            host.hook(link, kind);
            if host.alerted(session) {
                continue;
            }
            host.set_alerted(session, true);
            host.message(link, kind);
        }
    }
}

pub fn visual_delivery(policy: VisualPolicy, control: bool) -> (bool, bool) {
    if control {
        return (false, false);
    }
    (policy != VisualPolicy::On, policy != VisualPolicy::Off)
}
pub fn alert_message(kind: AlertKind, current: bool, index: i32) -> String {
    if current {
        format!("{} in current window", kind.label())
    } else {
        format!("{} in window {index}", kind.label())
    }
}

#[derive(Clone, Debug)]
pub enum AlertEffect {
    SilenceTimer {
        window: WindowId,
        seconds: u64,
    },
    DeferredCheck,
    Status(SessionId),
    Hook {
        link: WinlinkId,
        kind: AlertKind,
    },
    Delivery {
        session: SessionId,
        kind: AlertKind,
        current: bool,
        index: i32,
        visual: VisualPolicy,
    },
}
impl AlertKind {
    fn monitor_option(self) -> &'static [u8] {
        match self {
            Self::Bell => b"monitor-bell",
            Self::Activity => b"monitor-activity",
            Self::Silence => b"monitor-silence",
        }
    }
    fn action_option(self) -> &'static [u8] {
        match self {
            Self::Bell => b"bell-action",
            Self::Activity => b"activity-action",
            Self::Silence => b"silence-action",
        }
    }
    fn visual_option(self) -> &'static [u8] {
        match self {
            Self::Bell => b"visual-bell",
            Self::Activity => b"visual-activity",
            Self::Silence => b"visual-silence",
        }
    }
}
impl AlertHost for super::state::Server {
    fn window_flags(&self, window: WindowId) -> WindowFlags {
        self.windows
            .get(window)
            .expect("alert window retained")
            .flags
    }
    fn set_window_flags(&mut self, window: WindowId, flags: WindowFlags) {
        self.windows
            .get_mut(window)
            .expect("alert window retained")
            .flags = flags;
    }
    fn monitor(&self, window: WindowId, kind: AlertKind) -> bool {
        self.options.get_number(
            self.windows.get(window).unwrap().options,
            kind.monitor_option(),
        ) != 0
    }
    fn reset_silence_timer(&mut self, window: WindowId) {
        let seconds = self.options.get_number(
            self.windows.get(window).unwrap().options,
            b"monitor-silence",
        ) as u64;
        self.effects.push_back(super::state::ModelEffect::Alert(
            AlertEffect::SilenceTimer { window, seconds },
        ));
    }
    fn retain_window(&mut self, window: WindowId) {
        super::window::window_retain(self, window).expect("valid alert window");
    }
    fn release_window(&mut self, window: WindowId) {
        super::window::window_release(self, window).expect("valid alert window");
    }
    fn defer_alert_check(&mut self) {
        self.effects
            .push_back(super::state::ModelEffect::Alert(AlertEffect::DeferredCheck));
    }
    fn links(&self, window: WindowId) -> Vec<WinlinkId> {
        self.windows.get(window).unwrap().links.clone()
    }
    fn session(&self, link: WinlinkId) -> SessionId {
        self.winlinks.get(link).unwrap().session
    }
    fn link_flags(&self, link: WinlinkId) -> WinlinkFlags {
        self.winlinks.get(link).unwrap().flags
    }
    fn mark_link(&mut self, link: WinlinkId, flag: WinlinkFlags) {
        self.winlinks.get_mut(link).unwrap().flags.insert(flag);
    }
    fn current(&self, link: WinlinkId) -> bool {
        self.sessions.get(self.session(link)).unwrap().current == Some(link)
    }
    fn attached(&self, session: SessionId) -> bool {
        self.sessions.get(session).unwrap().attached != 0
    }
    fn set_alerted(&mut self, session: SessionId, alerted: bool) {
        let flags = &mut self.sessions.get_mut(session).unwrap().flags;
        if alerted {
            flags.insert(super::SessionFlags::ALERTED);
        } else {
            flags.remove(super::SessionFlags::ALERTED);
        }
    }
    fn alerted(&self, session: SessionId) -> bool {
        self.sessions
            .get(session)
            .unwrap()
            .flags
            .contains(super::SessionFlags::ALERTED)
    }
    fn action(&self, session: SessionId, kind: AlertKind) -> AlertPolicy {
        AlertPolicy::try_from(self.options.get_number(
            self.sessions.get(session).unwrap().options,
            kind.action_option(),
        ) as i32)
        .expect("valid alert policy")
    }
    fn status(&mut self, session: SessionId) {
        self.effects
            .push_back(super::state::ModelEffect::Alert(AlertEffect::Status(
                session,
            )));
    }
    fn hook(&mut self, link: WinlinkId, kind: AlertKind) {
        self.effects
            .push_back(super::state::ModelEffect::Alert(AlertEffect::Hook {
                link,
                kind,
            }));
    }
    fn message(&mut self, link: WinlinkId, kind: AlertKind) {
        let session = self.session(link);
        let visual = VisualPolicy::try_from(self.options.get_number(
            self.sessions.get(session).unwrap().options,
            kind.visual_option(),
        ) as i32)
        .expect("valid visual policy");
        self.effects
            .push_back(super::state::ModelEffect::Alert(AlertEffect::Delivery {
                session,
                kind,
                current: self.current(link),
                index: self.winlinks.get(link).unwrap().index,
                visual,
            }));
    }
}
pub fn alerts_queue(server: &mut super::state::Server, window: WindowId, flags: WindowFlags) {
    let mut queue = std::mem::take(&mut server.alerts);
    queue.queue(server, window, flags);
    server.alerts = queue;
}
pub fn alerts_dispatch(server: &mut super::state::Server) {
    let mut queue = std::mem::take(&mut server.alerts);
    queue.dispatch(server);
    server.alerts = queue;
}
pub fn alerts_check_session(server: &mut super::state::Server, session: SessionId) {
    let windows: Vec<_> = server
        .sessions
        .get(session)
        .unwrap()
        .windows
        .values()
        .map(|id| server.winlinks.get(*id).unwrap().window)
        .collect();
    check_session(server, windows);
}
pub fn alerts_reset_all(server: &mut super::state::Server) {
    let windows: Vec<_> = server.window_ids.values().copied().collect();
    AlertQueue::reset_all(server, windows);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ArenaId;
    #[derive(Default)]
    struct Host {
        flags: WindowFlags,
        links: [WinlinkFlags; 2],
        alerted: bool,
        log: Vec<String>,
        action: Option<AlertPolicy>,
        enabled: bool,
    }
    fn window() -> WindowId {
        WindowId::from_parts(0, 0)
    }
    impl AlertHost for Host {
        fn window_flags(&self, _: WindowId) -> WindowFlags {
            self.flags
        }
        fn set_window_flags(&mut self, _: WindowId, flags: WindowFlags) {
            self.flags = flags;
        }
        fn monitor(&self, _: WindowId, _: AlertKind) -> bool {
            self.enabled
        }
        fn reset_silence_timer(&mut self, _: WindowId) {
            self.log.push("timer".into());
        }
        fn retain_window(&mut self, _: WindowId) {
            self.log.push("retain".into());
        }
        fn release_window(&mut self, _: WindowId) {
            self.log.push("release".into());
        }
        fn defer_alert_check(&mut self) {
            self.log.push("defer".into());
        }
        fn links(&self, _: WindowId) -> Vec<WinlinkId> {
            vec![WinlinkId::from_parts(0, 0), WinlinkId::from_parts(1, 0)]
        }
        fn session(&self, _: WinlinkId) -> SessionId {
            SessionId::from_parts(0, 0)
        }
        fn link_flags(&self, link: WinlinkId) -> WinlinkFlags {
            self.links[link.parts().0 as usize]
        }
        fn mark_link(&mut self, link: WinlinkId, flag: WinlinkFlags) {
            self.links[link.parts().0 as usize].insert(flag);
        }
        fn current(&self, _: WinlinkId) -> bool {
            false
        }
        fn attached(&self, _: SessionId) -> bool {
            true
        }
        fn set_alerted(&mut self, _: SessionId, alerted: bool) {
            self.alerted = alerted;
        }
        fn alerted(&self, _: SessionId) -> bool {
            self.alerted
        }
        fn action(&self, _: SessionId, _: AlertKind) -> AlertPolicy {
            self.action.unwrap_or(AlertPolicy::Any)
        }
        fn status(&mut self, _: SessionId) {
            self.log.push("status".into());
        }
        fn hook(&mut self, _: WinlinkId, kind: AlertKind) {
            self.log.push(format!("hook:{}", kind.label()));
        }
        fn message(&mut self, _: WinlinkId, kind: AlertKind) {
            self.log.push(format!("message:{}", kind.label()));
        }
    }
    #[test]
    fn queue_lease_reset_order_and_duplicate_session_messages() {
        let mut host = Host {
            enabled: true,
            flags: WindowFlags::ZOOMED | WindowFlags::SILENCE,
            ..Host::default()
        };
        let mut queue = AlertQueue::default();
        queue.queue(
            &mut host,
            window(),
            WindowFlags::BELL | WindowFlags::ACTIVITY,
        );
        queue.queue(&mut host, window(), WindowFlags::SILENCE);
        assert_eq!(&host.log, &["timer", "retain", "defer", "timer"]);
        queue.dispatch(&mut host);
        assert_eq!(
            host.log
                .iter()
                .filter(|s| s.starts_with("message:"))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["message:Bell", "message:Activity", "message:Silence"]
        );
        assert_eq!(
            host.log.iter().filter(|s| s.starts_with("hook:")).count(),
            6
        );
        assert_eq!(host.flags, WindowFlags::ZOOMED);
        assert_eq!(host.log.last().unwrap(), "release");
        assert!(!queue.is_pending(window()));
    }
    #[test]
    fn disabled_flags_retained_and_mark_before_action() {
        let mut host = Host::default();
        let mut queue = AlertQueue::default();
        queue.queue(&mut host, window(), WindowFlags::ACTIVITY);
        assert!(host.flags.contains(WindowFlags::ACTIVITY));
        assert!(!queue.is_pending(window()));
        host.enabled = true;
        host.action = Some(AlertPolicy::None);
        check_window(&mut host, window());
        assert!(
            host.links
                .iter()
                .all(|f| f.contains(WinlinkFlags::ACTIVITY))
        );
        assert!(!host.log.iter().any(|s| s.starts_with("hook:")));
    }
    #[test]
    fn activity_deduplicates_bell_does_not() {
        let mut host = Host {
            enabled: true,
            flags: WindowFlags::BELL | WindowFlags::ACTIVITY,
            ..Host::default()
        };
        check_window(&mut host, window());
        host.log.clear();
        check_window(&mut host, window());
        assert_eq!(
            host.log
                .iter()
                .filter(|s| s.starts_with("hook:"))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["hook:Bell", "hook:Bell"]
        );
    }
    #[test]
    fn visual_and_text() {
        assert_eq!(visual_delivery(VisualPolicy::Off, false), (true, false));
        assert_eq!(visual_delivery(VisualPolicy::On, false), (false, true));
        assert_eq!(visual_delivery(VisualPolicy::Both, false), (true, true));
        assert_eq!(visual_delivery(VisualPolicy::Both, true), (false, false));
        assert_eq!(
            alert_message(AlertKind::Activity, false, 12),
            "Activity in window 12"
        );
    }
}
