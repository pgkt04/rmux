// Ported from tmux options.c @ 8f25579c
//! Scope selection (`options_scope_from_name`, `options_scope_from_flags`)
//! over resolved targets instead of `struct args` and `cmd_find_state`.

use super::parse::search;
use super::store::OptionsStore;
use super::{OptionsError, OptionsScope};
use crate::ids::OptionsId;

/// The flags `set-option`/`show-option` read from `Args`
/// (`options.c:1042,1055,1068,1093,1098,1108,1109,1123`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptionsScopeFlags {
    /// `-g`
    pub global: bool,
    /// `-s`
    pub server: bool,
    /// `-w`
    pub window: bool,
    /// `-p`
    pub pane: bool,
    /// The `window` argument: the command is `set-window-option` or
    /// `show-window-options`.
    pub window_command: bool,
}

/// The resolved `cmd_find_state` option trees plus the raw `-t` text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptionsScopeTarget<'a> {
    /// `fs->s->options`
    pub session: Option<OptionsId>,
    /// `fs->wl->window->options`
    pub window: Option<OptionsId>,
    /// `fs->wp->options`
    pub pane: Option<OptionsId>,
    /// `args_get(args, 't')`
    pub target: Option<&'a [u8]>,
}

fn missing(what: &[u8], target: Option<&[u8]>) -> OptionsError {
    match target {
        Some(t) => {
            let mut prefix = Vec::with_capacity(what.len() + 10);
            prefix.extend_from_slice(b"no such ");
            prefix.extend_from_slice(what);
            prefix.extend_from_slice(b": ");
            OptionsError::new(&prefix, t)
        }
        None => {
            let mut text = Vec::with_capacity(what.len() + 11);
            text.extend_from_slice(b"no current ");
            text.extend_from_slice(what);
            OptionsError::text(&text)
        }
    }
}

/// `options_scope_from_name` (`options.c:1013-1082`).
pub fn scope_from_name(
    flags: OptionsScopeFlags,
    name: &[u8],
    target: &OptionsScopeTarget<'_>,
    store: &OptionsStore,
) -> Result<(OptionsScope, OptionsId), OptionsError> {
    if name.first() == Some(&b'@') {
        return scope_from_flags(flags, target, store);
    }
    let Some(oe) = search(name) else {
        return Err(OptionsError::new(b"unknown option: ", name));
    };
    let window_or_pane = OptionsScope::WINDOW | OptionsScope::PANE;
    if oe.scope == OptionsScope::SERVER {
        return Ok((OptionsScope::SERVER, store.global));
    }
    if oe.scope == OptionsScope::SESSION {
        if flags.global {
            return Ok((OptionsScope::SESSION, store.global_s));
        }
        return match target.session {
            Some(id) => Ok((OptionsScope::SESSION, id)),
            None => Err(missing(b"session", target.target)),
        };
    }
    if oe.scope == window_or_pane && flags.pane {
        return match target.pane {
            Some(id) => Ok((OptionsScope::PANE, id)),
            None => Err(missing(b"pane", target.target)),
        };
    }
    if oe.scope == window_or_pane || oe.scope == OptionsScope::WINDOW {
        if flags.global {
            return Ok((OptionsScope::WINDOW, store.global_w));
        }
        return match target.window {
            Some(id) => Ok((OptionsScope::WINDOW, id)),
            None => Err(missing(b"window", target.target)),
        };
    }
    // A table entry with another scope combination has no tree in C either:
    // the switch falls through with OPTIONS_TABLE_NONE and no cause. No live
    // entry has such a scope (checked by the table tests).
    Err(OptionsError::new(b"unknown option: ", name))
}

/// `options_scope_from_flags` (`options.c:1084-1137`).
pub fn scope_from_flags(
    flags: OptionsScopeFlags,
    target: &OptionsScopeTarget<'_>,
    store: &OptionsStore,
) -> Result<(OptionsScope, OptionsId), OptionsError> {
    if flags.server {
        return Ok((OptionsScope::SERVER, store.global));
    }
    if flags.pane {
        return match target.pane {
            Some(id) => Ok((OptionsScope::PANE, id)),
            None => Err(missing(b"pane", target.target)),
        };
    }
    if flags.window_command || flags.window {
        if flags.global {
            return Ok((OptionsScope::WINDOW, store.global_w));
        }
        return match target.window {
            Some(id) => Ok((OptionsScope::WINDOW, id)),
            None => Err(missing(b"window", target.target)),
        };
    }
    if flags.global {
        return Ok((OptionsScope::SESSION, store.global_s));
    }
    match target.session {
        Some(id) => Ok((OptionsScope::SESSION, id)),
        None => Err(missing(b"session", target.target)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: OptionsScopeFlags = OptionsScopeFlags {
        global: true,
        server: false,
        window: false,
        pane: false,
        window_command: false,
    };
    const P: OptionsScopeFlags = OptionsScopeFlags {
        pane: true,
        global: false,
        server: false,
        window: false,
        window_command: false,
    };
    const NONE: OptionsScopeFlags = OptionsScopeFlags {
        global: false,
        server: false,
        window: false,
        pane: false,
        window_command: false,
    };

    fn err(r: Result<(OptionsScope, OptionsId), OptionsError>) -> String {
        String::from_utf8(r.unwrap_err().0.into_vec()).unwrap()
    }

    #[test]
    fn six_rows_of_scope_from_name() {
        let mut store = OptionsStore::new();
        let s = store.create(Some(store.global_s));
        let w = store.create(Some(store.global_w));
        let p = store.create(Some(w));
        let full = OptionsScopeTarget {
            session: Some(s),
            window: Some(w),
            pane: Some(p),
            target: None,
        };
        let empty = OptionsScopeTarget::default();
        let with_t = OptionsScopeTarget {
            target: Some(b"T"),
            ..empty
        };

        // SERVER: always global, even without a target.
        assert_eq!(
            scope_from_name(NONE, b"escape-time", &with_t, &store),
            Ok((OptionsScope::SERVER, store.global))
        );
        // SESSION -g / none.
        assert_eq!(
            scope_from_name(G, b"status", &empty, &store),
            Ok((OptionsScope::SESSION, store.global_s))
        );
        assert_eq!(
            scope_from_name(NONE, b"status", &full, &store),
            Ok((OptionsScope::SESSION, s))
        );
        assert_eq!(
            err(scope_from_name(NONE, b"status", &with_t, &store)),
            "no such session: T"
        );
        assert_eq!(
            err(scope_from_name(NONE, b"status", &empty, &store)),
            "no current session"
        );
        // WINDOW|PANE with -p.
        assert_eq!(
            scope_from_name(P, b"cursor-colour", &full, &store),
            Ok((OptionsScope::PANE, p))
        );
        assert_eq!(
            err(scope_from_name(P, b"cursor-colour", &with_t, &store)),
            "no such pane: T"
        );
        assert_eq!(
            err(scope_from_name(P, b"cursor-colour", &empty, &store)),
            "no current pane"
        );
        // WINDOW|PANE without -p, and WINDOW: -g / none.
        assert_eq!(
            scope_from_name(G, b"cursor-colour", &empty, &store),
            Ok((OptionsScope::WINDOW, store.global_w))
        );
        assert_eq!(
            scope_from_name(NONE, b"cursor-colour", &full, &store),
            Ok((OptionsScope::WINDOW, w))
        );
        assert_eq!(
            scope_from_name(G, b"automatic-rename", &empty, &store),
            Ok((OptionsScope::WINDOW, store.global_w))
        );
        assert_eq!(
            scope_from_name(NONE, b"automatic-rename", &full, &store),
            Ok((OptionsScope::WINDOW, w))
        );
        assert_eq!(
            err(scope_from_name(NONE, b"automatic-rename", &with_t, &store)),
            "no such window: T"
        );
        assert_eq!(
            err(scope_from_name(NONE, b"automatic-rename", &empty, &store)),
            "no current window"
        );
        // -p on a plain WINDOW option is ignored.
        assert_eq!(
            scope_from_name(P, b"automatic-rename", &full, &store),
            Ok((OptionsScope::WINDOW, w))
        );
        assert_eq!(
            err(scope_from_name(NONE, b"nonsense", &full, &store)),
            "unknown option: nonsense"
        );
        // A user option defers to the flags.
        assert_eq!(
            scope_from_name(NONE, b"@x", &full, &store),
            Ok((OptionsScope::SESSION, s))
        );
        assert_eq!(
            scope_from_name(P, b"@x", &full, &store),
            Ok((OptionsScope::PANE, p))
        );
    }

    #[test]
    fn scope_from_flags_rows() {
        let mut store = OptionsStore::new();
        let s = store.create(Some(store.global_s));
        let w = store.create(Some(store.global_w));
        let p = store.create(Some(w));
        let full = OptionsScopeTarget {
            session: Some(s),
            window: Some(w),
            pane: Some(p),
            target: None,
        };
        let empty = OptionsScopeTarget {
            target: Some(b"x:1"),
            ..OptionsScopeTarget::default()
        };
        let server = OptionsScopeFlags {
            server: true,
            ..NONE
        };
        let window = OptionsScopeFlags {
            window: true,
            ..NONE
        };
        let window_cmd = OptionsScopeFlags {
            window_command: true,
            global: true,
            ..NONE
        };
        assert_eq!(
            scope_from_flags(server, &empty, &store),
            Ok((OptionsScope::SERVER, store.global))
        );
        assert_eq!(
            err(scope_from_flags(P, &empty, &store)),
            "no such pane: x:1"
        );
        assert_eq!(
            scope_from_flags(window, &full, &store),
            Ok((OptionsScope::WINDOW, w))
        );
        assert_eq!(
            err(scope_from_flags(window, &empty, &store)),
            "no such window: x:1"
        );
        assert_eq!(
            scope_from_flags(window_cmd, &empty, &store),
            Ok((OptionsScope::WINDOW, store.global_w))
        );
        assert_eq!(
            scope_from_flags(G, &empty, &store),
            Ok((OptionsScope::SESSION, store.global_s))
        );
        assert_eq!(
            scope_from_flags(NONE, &full, &store),
            Ok((OptionsScope::SESSION, s))
        );
        assert_eq!(
            err(scope_from_flags(NONE, &empty, &store)),
            "no such session: x:1"
        );
    }
}
