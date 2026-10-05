// Ported from tmux control-notify.c, control.c @ 8f25579c
use super::ControlState;

pub const EVENT_NAMES: [&[u8]; 14] = [
    b"pane-mode-changed",
    b"window-layout-changed",
    b"window-pane-changed",
    b"window-unlinked",
    b"window-linked",
    b"window-renamed",
    b"client-session-changed",
    b"client-detached",
    b"session-renamed",
    b"session-created",
    b"session-closed",
    b"session-window-changed",
    b"paste-buffer-changed",
    b"paste-buffer-deleted",
];
/// Snapshot supplied at delivery, with layout expanded for this recipient.
#[derive(Default)]
pub struct Recipient {
    pub eligible: bool,
    pub attached: bool,
    pub linked: bool,
    pub is_subject: bool,
    pub layout: Option<Vec<u8>>,
}
pub enum Notification<'a> {
    PaneMode {
        pane: Option<u32>,
        fallback: Option<&'a [u8]>,
    },
    Layout {
        window: Option<u32>,
        valid: bool,
    },
    WindowPane {
        window: Option<u32>,
        pane: Option<u32>,
    },
    WindowUnlinked(Option<u32>),
    WindowLinked(Option<u32>),
    WindowRenamed {
        window: Option<u32>,
        name: &'a [u8],
    },
    ClientSession {
        client: Option<&'a [u8]>,
        session: Option<u32>,
        name: &'a [u8],
    },
    ClientDetached(Option<&'a [u8]>),
    SessionRenamed {
        session: Option<u32>,
        name: &'a [u8],
    },
    SessionCreated,
    SessionClosed,
    SessionWindow {
        session: Option<u32>,
        window: Option<u32>,
    },
    PasteChanged(Option<&'a [u8]>),
    PasteDeleted(Option<&'a [u8]>),
}
fn named(prefix: &str, name: &[u8]) -> Vec<u8> {
    let mut line = prefix.as_bytes().to_vec();
    line.extend_from_slice(super::cstring(name));
    line
}
pub fn render(recipient: &Recipient, event: Notification<'_>) -> Option<Vec<u8>> {
    if !recipient.eligible {
        return None;
    }
    let line = match event {
        Notification::PaneMode { pane: Some(p), .. } => {
            format!("%pane-mode-changed %{p}").into_bytes()
        }
        Notification::PaneMode {
            fallback: Some(p), ..
        } => named("%pane-mode-changed ", p),
        Notification::Layout {
            window: Some(_),
            valid: true,
        } if recipient.attached && recipient.linked => recipient.layout.as_ref()?.clone(),
        Notification::WindowPane {
            window: Some(w),
            pane: Some(p),
        } => format!("%window-pane-changed @{w} %{p}").into_bytes(),
        Notification::WindowUnlinked(Some(w)) if recipient.attached => format!(
            "%{}window-close @{w}",
            if recipient.linked { "" } else { "unlinked-" }
        )
        .into_bytes(),
        Notification::WindowLinked(Some(w)) if recipient.attached => format!(
            "%{}window-add @{w}",
            if recipient.linked { "" } else { "unlinked-" }
        )
        .into_bytes(),
        Notification::WindowRenamed {
            window: Some(w),
            name,
        } if recipient.attached => named(
            &format!(
                "%{}window-renamed @{w} ",
                if recipient.linked { "" } else { "unlinked-" }
            ),
            name,
        ),
        Notification::ClientSession {
            client: Some(client),
            session: Some(s),
            name,
        } if recipient.attached => {
            if recipient.is_subject {
                named(&format!("%session-changed ${s} "), name)
            } else {
                let mut line = named("%client-session-changed ", client);
                line.extend_from_slice(format!(" ${s} ").as_bytes());
                line.extend_from_slice(super::cstring(name));
                line
            }
        }
        Notification::ClientDetached(Some(name)) => named("%client-detached ", name),
        Notification::SessionRenamed {
            session: Some(s),
            name,
        } => named(&format!("%session-renamed ${s} "), name),
        Notification::SessionCreated | Notification::SessionClosed => b"%sessions-changed".to_vec(),
        Notification::SessionWindow {
            session: Some(s),
            window: Some(w),
        } => format!("%session-window-changed ${s} @{w}").into_bytes(),
        Notification::PasteChanged(Some(name)) => named("%paste-buffer-changed ", name),
        Notification::PasteDeleted(Some(name)) => named("%paste-buffer-deleted ", name),
        _ => return None,
    };
    Some(line)
}
pub fn deliver(state: &mut ControlState, recipient: &Recipient, event: Notification<'_>) {
    if let Some(line) = render(recipient, event) {
        state.notify_write(&line);
    }
}
pub fn subscription(
    state: &mut ControlState,
    name: &[u8],
    session: u32,
    winlink: Option<(u32, i32)>,
    pane: Option<u32>,
    value: &[u8],
) {
    let mut line = named("%subscription-changed ", name);
    line.extend_from_slice(format!(" ${session} ").as_bytes());
    if let Some((window, index)) = winlink {
        line.extend_from_slice(format!("@{window} {index} ").as_bytes());
        if let Some(pane) = pane {
            line.extend_from_slice(format!("%{pane}").as_bytes());
        } else {
            line.push(b'-');
        }
    } else {
        line.extend_from_slice(b"- - -");
    }
    line.extend_from_slice(b" : ");
    line.extend_from_slice(super::cstring(value));
    state.notify_write(&line);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_and_recipient_gates() {
        let mut s = ControlState::default();
        let r = Recipient {
            eligible: true,
            ..Recipient::default()
        };
        deliver(
            &mut s,
            &r,
            Notification::PaneMode {
                pane: None,
                fallback: Some(b"%19"),
            },
        );
        deliver(&mut s, &r, Notification::WindowLinked(Some(3)));
        deliver(
            &mut s,
            &r,
            Notification::SessionWindow {
                session: Some(2),
                window: None,
            },
        );
        deliver(
            &mut s,
            &r,
            Notification::SessionRenamed {
                session: Some(2),
                name: b"a b",
            },
        );
        assert_eq!(
            s.output,
            b"%pane-mode-changed %19\n%session-renamed $2 a b\n"
        );
    }
    #[test]
    fn subscription_forms_keep_colons() {
        let mut s = ControlState::default();
        subscription(&mut s, b"n", 1, Some((2, 3)), Some(4), b"a:b");
        subscription(&mut s, b"n", 1, Some((2, 3)), None, b"a:b");
        subscription(&mut s, b"n", 1, None, None, b"a:b");
        assert_eq!(s.output,b"%subscription-changed n $1 @2 3 %4 : a:b\n%subscription-changed n $1 @2 3 - : a:b\n%subscription-changed n $1 - - - : a:b\n");
    }
    #[test]
    fn complete_catalog_and_common_gate() {
        let r = Recipient {
            eligible: true,
            attached: true,
            linked: true,
            is_subject: true,
            layout: Some(b"%layout-change @1 L V F".to_vec()),
        };
        let cases = [
            (
                Notification::PaneMode {
                    pane: Some(2),
                    fallback: None,
                },
                "%pane-mode-changed %2",
            ),
            (
                Notification::Layout {
                    window: Some(1),
                    valid: true,
                },
                "%layout-change @1 L V F",
            ),
            (
                Notification::WindowPane {
                    window: Some(1),
                    pane: Some(2),
                },
                "%window-pane-changed @1 %2",
            ),
            (Notification::WindowUnlinked(Some(1)), "%window-close @1"),
            (Notification::WindowLinked(Some(1)), "%window-add @1"),
            (
                Notification::WindowRenamed {
                    window: Some(1),
                    name: b"n",
                },
                "%window-renamed @1 n",
            ),
            (
                Notification::ClientSession {
                    client: Some(b"c"),
                    session: Some(3),
                    name: b"s",
                },
                "%session-changed $3 s",
            ),
            (
                Notification::ClientDetached(Some(b"c")),
                "%client-detached c",
            ),
            (
                Notification::SessionRenamed {
                    session: Some(3),
                    name: b"s",
                },
                "%session-renamed $3 s",
            ),
            (Notification::SessionCreated, "%sessions-changed"),
            (Notification::SessionClosed, "%sessions-changed"),
            (
                Notification::SessionWindow {
                    session: Some(3),
                    window: Some(1),
                },
                "%session-window-changed $3 @1",
            ),
            (
                Notification::PasteChanged(Some(b"b")),
                "%paste-buffer-changed b",
            ),
            (
                Notification::PasteDeleted(Some(b"b")),
                "%paste-buffer-deleted b",
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(render(&r, event).unwrap(), expected.as_bytes());
        }
        let r = Recipient::default();
        assert!(render(&r, Notification::SessionCreated).is_none());
        let r = Recipient {
            eligible: true,
            attached: true,
            ..Recipient::default()
        };
        assert_eq!(
            render(&r, Notification::WindowLinked(Some(1))).unwrap(),
            b"%unlinked-window-add @1"
        );
        assert_eq!(
            render(&r, Notification::WindowUnlinked(Some(1))).unwrap(),
            b"%unlinked-window-close @1"
        );
        assert_eq!(
            render(
                &r,
                Notification::WindowRenamed {
                    window: Some(1),
                    name: b"n"
                }
            )
            .unwrap(),
            b"%unlinked-window-renamed @1 n"
        );
        assert_eq!(
            render(
                &r,
                Notification::ClientSession {
                    client: Some(b"c"),
                    session: Some(3),
                    name: b"s"
                }
            )
            .unwrap(),
            b"%client-session-changed c $3 s"
        );
    }
}
