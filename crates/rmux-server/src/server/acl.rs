// Ported from tmux server-acl.c, tmux.h @ 8f25579c
/*
 * Copyright (c) 2021 Holland Schutte, Jayson Morberg
 * Copyright (c) 2021 Dallas Lyons <dallasdlyons@gmail.com>
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
pub struct ServerAclFlags(pub u32);
impl ServerAclFlags {
    pub const READONLY: Self = Self(1);
    pub const IS_GROUP: Self = Self(2);
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
impl std::ops::BitOr for ServerAclFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for ServerAclFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for ServerAclFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use crate::client::ClientFlags;
use crate::ids::ClientId;
use crate::model::Server;
use rmux_sys::{GroupId, PrincipalId, UserId};
use rmux_util::bytes::ByteString;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerAclEntry {
    pub id: PrincipalId,
    pub flags: ServerAclFlags,
}

#[derive(Default)]
pub struct ServerAcl {
    entries: BTreeMap<(bool, u32), ServerAclEntry>,
}

impl ServerAcl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, owner: UserId) {
        self.entries.clear();
        if owner.0 != 0 {
            self.allow(PrincipalId(0), ServerAclFlags::default());
        }
        self.allow(PrincipalId(owner.0), ServerAclFlags::default());
    }

    fn key(id: PrincipalId, flags: ServerAclFlags) -> (bool, u32) {
        (flags.contains(ServerAclFlags::IS_GROUP), id.0)
    }

    pub fn find(&self, id: PrincipalId, flags: ServerAclFlags) -> bool {
        self.entries.contains_key(&Self::key(id, flags))
    }

    pub fn entry(&self, id: PrincipalId, flags: ServerAclFlags) -> Option<&ServerAclEntry> {
        self.entries.get(&Self::key(id, flags))
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &ServerAclEntry> {
        self.entries.values()
    }

    pub fn check(&self, uid: Option<UserId>, gid: Option<GroupId>) -> Option<&ServerAclEntry> {
        let uid = uid?;
        self.entry(PrincipalId(uid.0), ServerAclFlags::default())
            .or_else(|| self.entry(PrincipalId(gid?.0), ServerAclFlags::IS_GROUP))
    }

    pub fn allow(&mut self, id: PrincipalId, flags: ServerAclFlags) {
        self.entries
            .entry(Self::key(id, flags))
            .or_insert(ServerAclEntry {
                id,
                flags: flags & ServerAclFlags::IS_GROUP,
            });
    }

    pub fn deny(&mut self, id: PrincipalId, flags: ServerAclFlags) -> bool {
        self.entries.remove(&Self::key(id, flags)).is_some()
    }

    pub fn allow_write(&mut self, id: PrincipalId, flags: ServerAclFlags) -> bool {
        let Some(entry) = self.entries.get_mut(&Self::key(id, flags)) else {
            return false;
        };
        entry.flags.remove(ServerAclFlags::READONLY);
        true
    }

    pub fn deny_write(&mut self, id: PrincipalId, flags: ServerAclFlags) -> bool {
        let Some(entry) = self.entries.get_mut(&Self::key(id, flags)) else {
            return false;
        };
        entry.flags.insert(ServerAclFlags::READONLY);
        true
    }

    pub fn display_with(
        &self,
        mut lookup: impl FnMut(PrincipalId, bool) -> Option<Vec<u8>>,
    ) -> Vec<ByteString> {
        self.iter()
            .filter_map(|entry| {
                let group = entry.flags.contains(ServerAclFlags::IS_GROUP);
                if !group && entry.id.0 == 0 {
                    return None;
                }
                let mut name = lookup(entry.id, group).unwrap_or_else(|| b"unknown".to_vec());
                name.extend_from_slice(if group { b" (G," } else { b" (U," });
                name.extend_from_slice(if entry.flags.contains(ServerAclFlags::READONLY) {
                    b"R)"
                } else {
                    b"W)"
                });
                Some(name.into())
            })
            .collect()
    }

    pub fn display(&self) -> Vec<ByteString> {
        self.display_with(|id, group| {
            if group {
                rmux_sys::server::group_name(GroupId(id.0))
            } else {
                rmux_sys::server::user_name(UserId(id.0))
            }
        })
    }
}

pub fn init(server: &mut Server) {
    server.acl.init(rmux_sys::proc::getuid());
}

pub fn find(server: &Server, id: PrincipalId, flags: ServerAclFlags) -> bool {
    server.acl.find(id, flags)
}

pub fn display(server: &Server) -> Vec<ByteString> {
    server.acl.display()
}

pub fn allow(server: &mut Server, id: PrincipalId, flags: ServerAclFlags) {
    server.acl.allow(id, flags);
}

pub fn deny(server: &mut Server, id: PrincipalId, flags: ServerAclFlags) {
    if server.acl.deny(id, flags) {
        update(server);
    }
}

pub fn allow_write(server: &mut Server, id: PrincipalId, flags: ServerAclFlags) {
    if server.acl.allow_write(id, flags) {
        update(server);
    }
}

pub fn deny_write(server: &mut Server, id: PrincipalId, flags: ServerAclFlags) {
    if server.acl.deny_write(id, flags) {
        update(server);
    }
}

fn apply_access(
    flags: &mut ClientFlags,
    exit_message: &mut Option<Vec<u8>>,
    access: Option<ServerAclFlags>,
) {
    match access {
        None => {
            *exit_message = Some(b"access not allowed".to_vec());
            flags.insert(ClientFlags::EXIT);
        }
        Some(access) if access.contains(ServerAclFlags::READONLY) => {
            flags.insert(ClientFlags::READONLY)
        }
        Some(_) => flags.remove(ClientFlags::READONLY),
    }
}

pub fn update(server: &mut Server) {
    for id in &server.client_order {
        let Some(client) = server.clients.get(*id) else {
            continue;
        };
        let uid = client.peer.and_then(|peer| server.process.peer_uid(peer));
        let gid = client.peer.and_then(|peer| server.process.peer_gid(peer));
        let access = server.acl.check(uid, gid).map(|entry| entry.flags);
        let client = server.clients.get_mut(*id).expect("ACL client");
        apply_access(&mut client.flags, &mut client.exit_message, access);
    }
}

pub fn join(server: &mut Server, id: ClientId) -> bool {
    let Some(client) = server.clients.get(id) else {
        return false;
    };
    let uid = client.peer.and_then(|peer| server.process.peer_uid(peer));
    let gid = client.peer.and_then(|peer| server.process.peer_gid(peer));
    let Some(entry) = server.acl.check(uid, gid) else {
        return false;
    };
    if entry.flags.contains(ServerAclFlags::READONLY) {
        server
            .clients
            .get_mut(id)
            .expect("ACL joining client")
            .flags
            .insert(ClientFlags::READONLY);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Client;
    use std::os::unix::net::UnixStream;

    const USER: ServerAclFlags = ServerAclFlags(0);
    const GROUP: ServerAclFlags = ServerAclFlags::IS_GROUP;

    #[test]
    fn startup_allows_only_root_and_real_owner_and_root_is_not_duplicated() {
        let mut acl = ServerAcl::new();
        acl.init(UserId(100));
        assert_eq!(
            acl.iter().map(|entry| entry.id.0).collect::<Vec<_>>(),
            [0, 100]
        );
        assert!(
            acl.iter()
                .all(|entry| !entry.flags.contains(ServerAclFlags::READONLY))
        );
        acl.init(UserId(0));
        assert_eq!(acl.iter().map(|entry| entry.id.0).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn users_precede_groups_then_numeric_order_ignoring_readonly_in_key() {
        let mut acl = ServerAcl::new();
        for (id, flags) in [(9, GROUP), (5, USER), (1, GROUP), (2, USER), (0, USER)] {
            acl.allow(PrincipalId(id), flags | ServerAclFlags::READONLY);
        }
        assert_eq!(
            acl.iter()
                .map(|entry| (entry.flags.contains(GROUP), entry.id.0))
                .collect::<Vec<_>>(),
            [(false, 0), (false, 2), (false, 5), (true, 1), (true, 9)]
        );
        assert!(
            !acl.entry(PrincipalId(1), GROUP)
                .unwrap()
                .flags
                .contains(ServerAclFlags::READONLY)
        );
        acl.deny_write(PrincipalId(1), GROUP);
        acl.allow(PrincipalId(1), GROUP);
        assert!(
            acl.entry(PrincipalId(1), GROUP | ServerAclFlags::READONLY)
                .unwrap()
                .flags
                .contains(ServerAclFlags::READONLY)
        );
    }

    #[test]
    fn unavailable_credentials_uid_priority_and_primary_gid_only() {
        let mut acl = ServerAcl::new();
        acl.allow(PrincipalId(10), GROUP);
        assert!(acl.check(None, Some(GroupId(10))).is_none());
        assert!(acl.check(Some(UserId(20)), None).is_none());
        assert!(acl.check(Some(UserId(20)), Some(GroupId(11))).is_none());
        assert_eq!(
            acl.check(Some(UserId(20)), Some(GroupId(10)))
                .unwrap()
                .flags,
            GROUP
        );
        acl.allow(PrincipalId(20), USER);
        acl.deny_write(PrincipalId(20), USER);
        let entry = acl.check(Some(UserId(20)), Some(GroupId(10))).unwrap();
        assert_eq!(entry.id, PrincipalId(20));
        assert_eq!(entry.flags, ServerAclFlags::READONLY);
        assert_eq!(acl.check(Some(UserId(20)), None), Some(entry));
        acl.deny(PrincipalId(20), USER);
        assert_eq!(
            acl.check(Some(UserId(20)), Some(GroupId(10)))
                .unwrap()
                .flags,
            GROUP
        );
    }

    #[test]
    fn display_omits_root_user_not_root_group_and_preserves_byte_names() {
        let mut acl = ServerAcl::new();
        for (id, flags) in [(0, USER), (10, USER), (20, USER), (0, GROUP), (10, GROUP)] {
            acl.allow(PrincipalId(id), flags);
        }
        acl.deny_write(PrincipalId(10), USER);
        acl.deny_write(PrincipalId(10), GROUP);
        let mut visited = Vec::new();
        let lines = acl.display_with(|id, group| {
            visited.push((group, id.0));
            match (group, id.0) {
                (false, 10) => Some(b"user\xff".to_vec()),
                (true, 0) => Some(b"root-group".to_vec()),
                (true, 10) => Some(b"group".to_vec()),
                _ => None,
            }
        });
        assert_eq!(visited, [(false, 10), (false, 20), (true, 0), (true, 10)]);
        assert_eq!(
            lines,
            [
                ByteString::from(b"user\xff (U,R)".as_slice()),
                ByteString::from("unknown (U,W)"),
                ByteString::from("root-group (G,W)"),
                ByteString::from("group (G,R)")
            ]
        );
    }

    #[test]
    fn live_access_flags_preserve_unrelated_flags_and_never_clear_previous_exit() {
        let mut flags = ClientFlags::CONTROL;
        let mut message = None;
        apply_access(&mut flags, &mut message, Some(ServerAclFlags::READONLY));
        assert!(flags.contains(ClientFlags::READONLY | ClientFlags::CONTROL));
        apply_access(&mut flags, &mut message, Some(USER));
        assert_eq!(flags, ClientFlags::CONTROL);
        apply_access(&mut flags, &mut message, None);
        assert!(flags.contains(ClientFlags::EXIT));
        assert_eq!(message.as_deref(), Some(b"access not allowed".as_slice()));
        apply_access(&mut flags, &mut message, Some(USER));
        assert!(flags.contains(ClientFlags::EXIT));
        assert_eq!(message.as_deref(), Some(b"access not allowed".as_slice()));
    }

    fn fixture(
        server: &mut Server,
        uid: Option<UserId>,
        gid: Option<GroupId>,
    ) -> (ClientId, UnixStream) {
        let (socket, remote) = UnixStream::pair().unwrap();
        let peer = server.process.add_peer(socket.into()).unwrap();
        let record = server.process.peers.get_mut(peer).unwrap();
        record.uid = uid;
        record.gid = gid;
        let id = server
            .clients
            .insert(Client::new(Some(peer), (0, 0)))
            .unwrap();
        server.clients.retain(id).unwrap();
        server.client_order.push_back(id);
        (id, remote)
    }

    #[test]
    fn join_only_sets_readonly_and_denial_has_no_mutation() {
        let mut server = Server::new();
        let (client, _socket) = fixture(&mut server, Some(UserId(50)), Some(GroupId(70)));
        server.clients.get_mut(client).unwrap().retval = 17;
        assert!(!join(&mut server, client));
        assert_eq!(server.clients.get(client).unwrap().retval, 17);
        assert!(server.clients.get(client).unwrap().exit_message.is_none());
        assert!(
            !server
                .clients
                .get(client)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        allow(&mut server, PrincipalId(50), USER);
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .insert(ClientFlags::READONLY);
        assert!(join(&mut server, client));
        assert!(
            server
                .clients
                .get(client)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        deny_write(&mut server, PrincipalId(50), USER);
        assert!(join(&mut server, client));
    }

    #[test]
    fn command_mutations_update_all_clients_even_unchanged_write_state_but_allow_does_not() {
        let mut server = Server::new();
        let (user, _socket1) = fixture(&mut server, Some(UserId(50)), Some(GroupId(70)));
        let (group, _socket2) = fixture(&mut server, Some(UserId(51)), Some(GroupId(70)));
        allow(&mut server, PrincipalId(70), GROUP);
        allow(&mut server, PrincipalId(50), USER);
        assert!(server.clients.get(user).unwrap().exit_message.is_none());
        assert!(server.clients.get(group).unwrap().exit_message.is_none());
        deny_write(&mut server, PrincipalId(70), GROUP);
        assert!(
            !server
                .clients
                .get(user)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        assert!(
            server
                .clients
                .get(group)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        deny_write(&mut server, PrincipalId(50), USER);
        assert!(
            server
                .clients
                .get(user)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        allow_write(&mut server, PrincipalId(50), USER);
        assert!(
            !server
                .clients
                .get(user)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        server
            .clients
            .get_mut(user)
            .unwrap()
            .flags
            .insert(ClientFlags::READONLY);
        allow(&mut server, PrincipalId(50), USER);
        assert!(
            server
                .clients
                .get(user)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        allow_write(&mut server, PrincipalId(50), USER);
        assert!(
            !server
                .clients
                .get(user)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        server.clients.get_mut(user).unwrap().retval = 19;
        deny(&mut server, PrincipalId(50), USER);
        assert!(
            server
                .clients
                .get(user)
                .unwrap()
                .flags
                .contains(ClientFlags::READONLY)
        );
        assert!(
            !server
                .clients
                .get(user)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        deny(&mut server, PrincipalId(70), GROUP);
        for id in [user, group] {
            let client = server.clients.get(id).unwrap();
            assert!(client.flags.contains(ClientFlags::EXIT));
            assert_eq!(
                client.exit_message.as_deref(),
                Some(b"access not allowed".as_slice())
            );
        }
        assert_eq!(server.clients.get(user).unwrap().retval, 19);
        let (late, _socket3) = fixture(&mut server, Some(UserId(99)), None);
        deny(&mut server, PrincipalId(999), USER);
        deny_write(&mut server, PrincipalId(999), USER);
        allow_write(&mut server, PrincipalId(999), USER);
        assert!(
            !server
                .clients
                .get(late)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
        allow(&mut server, PrincipalId(99), USER);
        assert!(
            !server
                .clients
                .get(late)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
    }

    #[test]
    fn unknown_uid_cannot_gain_access_through_group_live_update() {
        let mut server = Server::new();
        let (client, _socket) = fixture(&mut server, None, Some(GroupId(10)));
        allow(&mut server, PrincipalId(10), GROUP);
        assert!(!join(&mut server, client));
        deny_write(&mut server, PrincipalId(10), GROUP);
        assert!(
            server
                .clients
                .get(client)
                .unwrap()
                .flags
                .contains(ClientFlags::EXIT)
        );
    }

    struct Oracle {
        binary: std::path::PathBuf,
        socket: std::path::PathBuf,
        directory: std::path::PathBuf,
    }

    impl Oracle {
        fn command(&self, args: &[&std::ffi::OsStr]) -> Vec<u8> {
            let output = std::process::Command::new(&self.binary)
                .arg("-S")
                .arg(&self.socket)
                .args(args)
                .env_remove("TMUX")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "oracle {:?}: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        }
    }

    impl Drop for Oracle {
        fn drop(&mut self) {
            let _ = std::process::Command::new(&self.binary)
                .arg("-S")
                .arg(&self.socket)
                .arg("kill-server")
                .env_remove("TMUX")
                .output();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn server_access_display_and_readonly_transitions_match_platform_oracle() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
        if !binary.exists() {
            eprintln!(
                "skipping ACL oracle comparison: {} is missing",
                binary.display()
            );
            return;
        }
        let directory = std::env::temp_dir().join(format!(
            "rmux-acl-oracle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let oracle = Oracle {
            binary,
            socket: directory.join("socket"),
            directory,
        };
        oracle.command(&[
            OsStr::new("-f"),
            OsStr::new("/dev/null"),
            OsStr::new("new-session"),
            OsStr::new("-d"),
            OsStr::new("-s"),
            OsStr::new("acl-check"),
        ]);
        let owner = rmux_sys::proc::getuid();
        let mut acl = ServerAcl::new();
        acl.init(owner);
        let compare = |acl: &ServerAcl| {
            let actual = oracle.command(&[OsStr::new("server-access"), OsStr::new("-l")]);
            let mut expected = Vec::new();
            for line in acl.display() {
                expected.extend_from_slice(line.as_bytes());
                expected.push(b'\n');
            }
            assert_eq!(actual, expected);
        };
        compare(&acl);
        let (id, name) = (0..256)
            .find_map(|id| rmux_sys::server::group_name(GroupId(id)).map(|name| (id, name)))
            .expect("platform must have a named system group");
        let name = OsStr::from_bytes(&name);
        oracle.command(&[OsStr::new("server-access"), OsStr::new("-ag"), name]);
        acl.allow(PrincipalId(id), GROUP);
        compare(&acl);
        oracle.command(&[OsStr::new("server-access"), OsStr::new("-rg"), name]);
        acl.deny_write(PrincipalId(id), GROUP);
        compare(&acl);
        oracle.command(&[OsStr::new("server-access"), OsStr::new("-wg"), name]);
        acl.allow_write(PrincipalId(id), GROUP);
        compare(&acl);
        oracle.command(&[OsStr::new("server-access"), OsStr::new("-dg"), name]);
        acl.deny(PrincipalId(id), GROUP);
        compare(&acl);
        if let Some((id, name)) = (1..256)
            .filter(|id| *id != owner.0)
            .find_map(|id| rmux_sys::server::user_name(UserId(id)).map(|name| (id, name)))
        {
            let name = OsStr::from_bytes(&name);
            oracle.command(&[OsStr::new("server-access"), OsStr::new("-ar"), name]);
            acl.allow(PrincipalId(id), USER);
            acl.deny_write(PrincipalId(id), USER);
            compare(&acl);
            oracle.command(&[OsStr::new("server-access"), OsStr::new("-w"), name]);
            acl.allow_write(PrincipalId(id), USER);
            compare(&acl);
            oracle.command(&[OsStr::new("server-access"), OsStr::new("-d"), name]);
            acl.deny(PrincipalId(id), USER);
            compare(&acl);
        }
    }
}
