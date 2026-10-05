// Ported from tmux environ.c, tmux.h @ 8f25579c
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct EnvironmentFlags(pub u32);
impl EnvironmentFlags {
    pub const HIDDEN: Self = Self(1);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for EnvironmentFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for EnvironmentFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for EnvironmentFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

// Ported from tmux environ.c @ 8f25579c
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use rmux_sys::fnmatch::{FnmatchFlags, fnmatch};
use rmux_util::bytes::ByteString;
use rmux_util::log_debug;

use super::store::OptionsStore;
use crate::ids::OptionsId;

/// The pinned tmux version reported as `TERM_PROGRAM_VERSION` and
/// `#{version}` (`configure.ac:3`, `tmux.c:429-432`).
pub const TMUX_VERSION: &[u8] = rmux_emu::input::TMUX_VERSION.as_bytes();

static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

/// `struct environ_entry` (`tmux.h:1609-1617`); `value == None` is a
/// cleared name that masks the variable when copied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentEntry {
    pub value: Option<ByteString>,
    pub flags: EnvironmentFlags,
    serial: u64,
}

impl EnvironmentEntry {
    fn new(value: Option<ByteString>, flags: EnvironmentFlags) -> EnvironmentEntry {
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        assert!(serial != 0, "environment serial counter exhausted");
        EnvironmentEntry {
            value,
            flags,
            serial,
        }
    }
    /// Creation identity for G19 tags; an overwrite keeps it.
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

/// `struct environ`: entries in `strcmp` order (`environ.c:32-40`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Environment {
    entries: BTreeMap<ByteString, EnvironmentEntry>,
}

impl Environment {
    /// `environ_create`
    pub fn new() -> Environment {
        Environment::default()
    }

    /// `environ_first/next`
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &EnvironmentEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_bytes(), v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `environ_find`
    pub fn find(&self, name: &[u8]) -> Option<&EnvironmentEntry> {
        self.entries.get(name)
    }

    /// `environ_set` (`environ.c:111-131`): create or overwrite, always
    /// setting the flags.
    pub fn set(&mut self, name: &[u8], flags: EnvironmentFlags, value: &[u8]) {
        match self.entries.get_mut(name) {
            Some(e) => {
                e.flags = flags;
                e.value = Some(ByteString::from(value));
            }
            None => {
                self.entries.insert(
                    ByteString::from(name),
                    EnvironmentEntry::new(Some(ByteString::from(value)), flags),
                );
            }
        }
    }

    /// `environ_clear` (`environ.c:134-149`): an existing entry keeps its
    /// flags, a new one gets flags 0.
    pub fn clear(&mut self, name: &[u8]) {
        match self.entries.get_mut(name) {
            Some(e) => e.value = None,
            None => {
                self.entries.insert(
                    ByteString::from(name),
                    EnvironmentEntry::new(None, EnvironmentFlags::default()),
                );
            }
        }
    }

    /// `environ_put` (`environ.c:152-168`): `NAME=VALUE`; no `=` is ignored.
    pub fn put(&mut self, var: &[u8], flags: EnvironmentFlags) {
        let Some(eq) = var.iter().position(|&b| b == b'=') else {
            return;
        };
        self.set(&var[..eq], flags, &var[eq + 1..]);
    }

    /// `environ_unset` (`environ.c:171-182`)
    pub fn unset(&mut self, name: &[u8]) {
        self.entries.remove(name);
    }

    /// `environ_copy` (`environ.c:85-98`): a cleared source clears in `dst`.
    pub fn copy_into(&self, dst: &mut Environment) {
        for (name, e) in &self.entries {
            match &e.value {
                None => dst.clear(name),
                Some(v) => dst.set(name, e.flags, v),
            }
        }
    }

    /// The body of `environ_update` (`environ.c:198-211`): for each
    /// pattern, every matching `src` name is set here with flags 0; a
    /// pattern with no match is cleared here.
    pub fn update_from<'a>(&mut self, patterns: impl Iterator<Item = &'a [u8]>, src: &Environment) {
        for pattern in patterns {
            let mut found = false;
            for (name, e) in &src.entries {
                if fnmatch(pattern, name, FnmatchFlags::NONE) {
                    let value = e
                        .value
                        .as_ref()
                        .expect("environ_update: cleared source entry has no C formatting");
                    self.set(name, EnvironmentFlags::default(), value);
                    found = true;
                }
            }
            if !found {
                self.clear(pattern);
            }
        }
    }

    /// `environ_push` before `fork` (`environ.c:215-227`): `NAME=VALUE` in
    /// name order for entries with a value, a non-empty name without `=`,
    /// and no `HIDDEN` flag (`setenv` rejects the other names).
    pub fn to_envp(&self) -> Vec<ByteString> {
        let mut out = Vec::with_capacity(self.entries.len());
        for (name, e) in &self.entries {
            let Some(value) = &e.value else {
                continue;
            };
            if name.is_empty() || name.contains(&b'=') || e.flags.contains(EnvironmentFlags::HIDDEN)
            {
                continue;
            }
            let mut s = ByteString::with_capacity(name.len() + 1 + value.len());
            s.extend_from_slice(name);
            s.push(b'=');
            s.extend_from_slice(value);
            out.push(s);
        }
        out
    }

    /// `environ_log` (`environ.c:230-249`): hidden entries are logged.
    pub fn log(&self, prefix: &[u8]) {
        for (name, e) in &self.entries {
            if let Some(value) = &e.value {
                if !name.is_empty() {
                    log_debug!("{}{}={}", ByteString::from(prefix), name, value);
                }
            }
        }
    }
}

/// `environ_update(oo, src, dst)` (`environ.c:185-212`): the patterns are
/// the items of the inherited `update-environment` array in `oo`.
pub fn environ_update(
    store: &OptionsStore,
    oo: OptionsId,
    src: &Environment,
    dst: &mut Environment,
) {
    let Some((_, o)) = store.get(oo, b"update-environment") else {
        return;
    };
    dst.update_from(
        o.array_items().map(|(_, item)| item.value().as_string()),
        src,
    );
}

/// The server facts `environ_for_session` reads besides the environments.
#[derive(Clone, Copy, Debug)]
pub struct SessionEnvironmentContext<'a> {
    /// `options_get_string(global_options, "default-terminal")`
    pub default_terminal: &'a [u8],
    /// `socket_path`
    pub socket_path: &'a [u8],
    /// `getpid()`
    pub pid: i64,
    /// `s->id`, the public session id.
    pub session_id: Option<u32>,
}

/// `environ_for_session` (`environ.c:252-286`): global copy, session copy,
/// the `TERM` group unless `no_term`, systemd clears, then `RMUX`.
pub fn environ_for_session(
    global_environ: &Environment,
    session_environ: Option<&Environment>,
    ctx: SessionEnvironmentContext<'_>,
    no_term: bool,
) -> Environment {
    let mut env = Environment::new();
    global_environ.copy_into(&mut env);
    if let Some(s) = session_environ {
        s.copy_into(&mut env);
    }
    let none = EnvironmentFlags::default();
    if !no_term {
        env.set(b"TERM", none, ctx.default_terminal);
        env.set(b"TERM_PROGRAM", none, b"tmux");
        env.set(b"TERM_PROGRAM_VERSION", none, TMUX_VERSION);
        env.set(b"COLORTERM", none, b"truecolor");
    }
    #[cfg(feature = "systemd")]
    {
        env.clear(b"LISTEN_PID");
        env.clear(b"LISTEN_FDS");
        env.clear(b"LISTEN_FDNAMES");
    }
    let idx: i64 = ctx.session_id.map_or(-1, i64::from);
    let mut value = ByteString::from(ctx.socket_path);
    value.extend_from_slice(format!(",{},{}", ctx.pid, idx).as_bytes());
    env.set(b"RMUX", none, &value);
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIDDEN: EnvironmentFlags = EnvironmentFlags::HIDDEN;
    const NONE: EnvironmentFlags = EnvironmentFlags(0);

    fn names(env: &Environment) -> Vec<String> {
        env.iter()
            .map(|(n, _)| String::from_utf8_lossy(n).into_owned())
            .collect()
    }

    #[test]
    fn put_without_equals_is_ignored_and_name_may_be_empty() {
        let mut env = Environment::new();
        env.put(b"NOEQUALS", NONE);
        assert!(env.is_empty());
        env.put(b"A=1=2", NONE);
        assert_eq!(
            env.find(b"A").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"1=2"[..])
        );
        env.put(b"=v", NONE);
        assert_eq!(
            env.find(b"").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"v"[..])
        );
    }

    #[test]
    fn clear_keeps_flags_and_copy_does_not_carry_cleared_flags() {
        let mut src = Environment::new();
        src.set(b"H", HIDDEN, b"x");
        src.clear(b"H");
        assert_eq!(src.find(b"H").unwrap().flags, HIDDEN);
        assert_eq!(src.find(b"H").unwrap().value, None);
        src.clear(b"NEW");
        assert_eq!(src.find(b"NEW").unwrap().flags, NONE);

        let mut dst = Environment::new();
        src.copy_into(&mut dst);
        assert_eq!(dst.find(b"H").unwrap().flags, NONE);
        assert_eq!(dst.find(b"H").unwrap().value, None);

        let mut dst2 = Environment::new();
        dst2.set(b"H", HIDDEN, b"keep");
        src.copy_into(&mut dst2);
        assert_eq!(dst2.find(b"H").unwrap().flags, HIDDEN);
        assert_eq!(dst2.find(b"H").unwrap().value, None);

        let mut src3 = Environment::new();
        src3.set(b"V", HIDDEN, b"1");
        src3.copy_into(&mut dst2);
        assert_eq!(dst2.find(b"V").unwrap().flags, HIDDEN);
    }

    #[test]
    fn to_envp_filters_and_sorts() {
        let mut env = Environment::new();
        env.set(b"Z", NONE, b"last");
        env.set(b"HID", HIDDEN, b"secret");
        env.set(b"A", NONE, b"first");
        env.clear(b"CLEARED");
        env.set(b"", NONE, b"empty-name");
        env.set(b"BAD=NAME", NONE, b"x");
        env.set(b"M", NONE, b"");
        let envp: Vec<String> = env
            .to_envp()
            .into_iter()
            .map(|s| String::from_utf8(s.into_vec()).unwrap())
            .collect();
        assert_eq!(envp, ["A=first", "M=", "Z=last"]);
        assert_eq!(
            names(&env),
            ["", "A", "BAD=NAME", "CLEARED", "HID", "M", "Z"]
        );
    }

    #[test]
    fn serial_survives_overwrite_but_not_unset() {
        let mut env = Environment::new();
        env.set(b"A", NONE, b"1");
        let serial = env.find(b"A").unwrap().serial();
        env.set(b"A", HIDDEN, b"2");
        assert_eq!(env.find(b"A").unwrap().serial(), serial);
        env.clear(b"A");
        assert_eq!(env.find(b"A").unwrap().serial(), serial);
        env.unset(b"A");
        assert!(env.find(b"A").is_none());
        env.set(b"A", NONE, b"3");
        assert_ne!(env.find(b"A").unwrap().serial(), serial);
    }

    #[test]
    fn update_from_patterns() {
        let mut src = Environment::new();
        src.set(b"FOO_A", HIDDEN, b"a");
        src.set(b"FOO_B", NONE, b"b");
        src.set(b"BAR", NONE, b"bar");
        let mut dst = Environment::new();
        dst.set(b"NOMATCH", NONE, b"old");
        let patterns: [&[u8]; 3] = [b"FOO_*", b"NOMATCH", b"BAR"];
        dst.update_from(patterns.iter().copied(), &src);
        assert_eq!(
            dst.find(b"FOO_A").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"a"[..])
        );
        assert_eq!(dst.find(b"FOO_A").unwrap().flags, NONE);
        assert_eq!(
            dst.find(b"FOO_B").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"b"[..])
        );
        assert_eq!(dst.find(b"NOMATCH").unwrap().value, None);
        assert_eq!(
            dst.find(b"BAR").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"bar"[..])
        );
        // Overlapping patterns repeat the same value.
        let patterns: [&[u8]; 2] = [b"BAR", b"B*"];
        dst.update_from(patterns.iter().copied(), &src);
        assert_eq!(
            dst.find(b"BAR").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"bar"[..])
        );
    }

    #[test]
    fn for_session_with_and_without_term_and_session() {
        let mut global = Environment::new();
        global.set(b"PATH", NONE, b"/bin");
        global.set(b"TERM", NONE, b"outer");
        global.set(b"RMUX", NONE, b"stale");
        let mut session = Environment::new();
        session.set(b"TERM", HIDDEN, b"session-term");
        session.set(b"SESSION_ONLY", NONE, b"1");
        let ctx = SessionEnvironmentContext {
            default_terminal: b"tmux-256color",
            socket_path: b"/tmp/rmux-501/default",
            pid: 4242,
            session_id: Some(7),
        };

        let env = environ_for_session(&global, Some(&session), ctx, false);
        let term = env.find(b"TERM").unwrap();
        assert_eq!(
            term.value.as_deref().map(|v| &v[..]),
            Some(&b"tmux-256color"[..])
        );
        assert_eq!(term.flags, NONE);
        assert_eq!(
            env.find(b"TERM_PROGRAM")
                .unwrap()
                .value
                .as_deref()
                .map(|v| &v[..]),
            Some(&b"tmux"[..])
        );
        assert_eq!(
            env.find(b"TERM_PROGRAM_VERSION")
                .unwrap()
                .value
                .as_deref()
                .map(|v| &v[..]),
            Some(&b"next-3.9"[..])
        );
        assert_eq!(
            env.find(b"COLORTERM")
                .unwrap()
                .value
                .as_deref()
                .map(|v| &v[..]),
            Some(&b"truecolor"[..])
        );
        assert_eq!(
            env.find(b"RMUX").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"/tmp/rmux-501/default,4242,7"[..])
        );
        assert_eq!(
            env.find(b"SESSION_ONLY")
                .unwrap()
                .value
                .as_deref()
                .map(|v| &v[..]),
            Some(&b"1"[..])
        );
        assert!(env.find(b"TMUX").is_none());

        let env = environ_for_session(&global, Some(&session), ctx, true);
        let term = env.find(b"TERM").unwrap();
        assert_eq!(
            term.value.as_deref().map(|v| &v[..]),
            Some(&b"session-term"[..])
        );
        assert_eq!(term.flags, HIDDEN);
        assert!(env.find(b"TERM_PROGRAM").is_none());
        assert!(env.find(b"COLORTERM").is_none());

        let ctx = SessionEnvironmentContext {
            session_id: None,
            ..ctx
        };
        let env = environ_for_session(&global, None, ctx, false);
        assert_eq!(
            env.find(b"RMUX").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"/tmp/rmux-501/default,4242,-1"[..])
        );
        assert!(env.find(b"SESSION_ONLY").is_none());
        assert_eq!(
            env.find(b"TERM").unwrap().value.as_deref().map(|v| &v[..]),
            Some(&b"tmux-256color"[..])
        );
    }

    #[test]
    fn copied_terminal_state_and_activation_flags() {
        let mut global = Environment::new();
        for name in [
            b"TERM".as_slice(),
            b"TERM_PROGRAM",
            b"TERM_PROGRAM_VERSION",
            b"COLORTERM",
        ] {
            global.set(name, HIDDEN, b"copied");
        }
        global.clear(b"TERM_PROGRAM_VERSION");
        global.set(b"TMUX", NONE, b"user-variable");
        global.set(b"RMUX_PANE", NONE, b"%9");
        global.set(b"RMUX", HIDDEN, b"stale");
        for name in [b"LISTEN_PID".as_slice(), b"LISTEN_FDS", b"LISTEN_FDNAMES"] {
            global.set(name, HIDDEN, b"activation");
        }
        let ctx = SessionEnvironmentContext {
            default_terminal: b"screen",
            socket_path: b"/socket",
            pid: 123,
            session_id: None,
        };
        let env = environ_for_session(&global, None, ctx, true);
        for name in [b"TERM".as_slice(), b"TERM_PROGRAM", b"COLORTERM"] {
            assert_eq!(
                env.find(name).unwrap().value,
                global.find(name).unwrap().value
            );
            assert_eq!(env.find(name).unwrap().flags, HIDDEN);
        }
        assert_eq!(env.find(b"TERM_PROGRAM_VERSION").unwrap().value, None);
        assert_eq!(env.find(b"RMUX").unwrap().flags, NONE);
        assert_eq!(
            env.find(b"RMUX")
                .unwrap()
                .value
                .as_deref()
                .map(Vec::as_slice),
            Some(b"/socket,123,-1".as_slice())
        );
        assert_eq!(
            env.find(b"TMUX")
                .unwrap()
                .value
                .as_deref()
                .map(Vec::as_slice),
            Some(b"user-variable".as_slice())
        );
        assert_eq!(
            env.find(b"RMUX_PANE")
                .unwrap()
                .value
                .as_deref()
                .map(Vec::as_slice),
            Some(b"%9".as_slice())
        );
        for name in [b"LISTEN_PID".as_slice(), b"LISTEN_FDS", b"LISTEN_FDNAMES"] {
            let entry = env.find(name).unwrap();
            assert_eq!(entry.flags, HIDDEN);
            assert_eq!(entry.value.is_none(), cfg!(feature = "systemd"));
        }
    }
}
