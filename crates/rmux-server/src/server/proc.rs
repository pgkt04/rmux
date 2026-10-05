// Ported from tmux proc.c @ 8f25579c
// Copyright (c) 2014 Nicholas Marriott <nicholas.marriott@gmail.com>
// Permission to use, copy, modify, and distribute this software for any purpose
// with or without fee is hereby granted, provided that the above copyright
// notice and this permission notice appear in all copies.
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
use super::{
    event_loop::{EventLoop, LoopAction},
    protocol::{
        PeerState, ProtocolBackend, ProtocolError, ProtocolMessage, ProtocolMessageKind,
        RmuxBackend, VERSION,
    },
};
use crate::{
    ids::{Arena, ArenaError, EventToken, PeerId},
    model::Server,
};
use std::{
    collections::VecDeque,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
};
pub struct Peer {
    pub fd: OwnedFd,
    pub codec: RmuxBackend,
    pub state: PeerState,
    pub uid: Option<rmux_sys::UserId>,
    pub gid: Option<rmux_sys::GroupId>,
    pub token: Option<EventToken>,
}
pub enum PeerDispatch {
    Message(PeerId, ProtocolMessage),
    Closed(PeerId),
}
pub type SignalAction = fn(&mut Server, i32);
#[derive(Default)]
pub struct Process {
    pub peers: Arena<Peer, PeerId>,
    pub order: VecDeque<PeerId>,
    pub exiting: bool,
    pub logging: bool,
}
impl Process {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_peer(&mut self, fd: OwnedFd) -> Result<PeerId, ProtocolError> {
        rmux_sys::fd::set_blocking(fd.as_fd(), false);
        let creds = rmux_sys::proc::getpeereid(fd.as_fd());
        let peer = self
            .peers
            .insert(Peer {
                fd,
                codec: RmuxBackend::default(),
                state: PeerState::Active,
                uid: creds.map(|v| v.0),
                gid: creds.map(|v| v.1),
                token: None,
            })
            .map_err(|_| ProtocolError::Shape("peer arena exhausted"))?;
        self.order.push_back(peer);
        Ok(peer)
    }
    pub fn fd(&self, id: PeerId) -> Option<BorrowedFd<'_>> {
        self.peers.get(id).map(|p| p.fd.as_fd())
    }
    pub fn peer_uid(&self, id: PeerId) -> Option<rmux_sys::UserId> {
        self.peers.get(id).and_then(|p| p.uid)
    }
    pub fn peer_gid(&self, id: PeerId) -> Option<rmux_sys::GroupId> {
        self.peers.get(id).and_then(|p| p.gid)
    }
    pub fn send(&mut self, id: PeerId, message: ProtocolMessage) -> Result<(), ProtocolError> {
        let peer = self.peers.get_mut(id).ok_or(ProtocolError::Closed)?;
        if peer.state != PeerState::Active {
            return Err(ProtocolError::Closed);
        }
        peer.codec.enqueue(message)
    }
    pub fn queued_output(&self, id: PeerId) -> bool {
        self.peers.get(id).is_some_and(|p| p.codec.queued_output())
    }
    pub fn kill_peer(&mut self, id: PeerId) {
        if let Some(p) = self.peers.get_mut(id) {
            p.state = PeerState::BadDraining;
        }
    }
    pub fn remove_peer(&mut self, id: PeerId) {
        self.order.retain(|p| *p != id);
        let _ = self.peers.request_remove(id);
    }
    pub fn detach_peer(&mut self, id: PeerId, event_loop: &mut EventLoop) {
        if let Some(p) = self.peers.get_mut(id) {
            if let Some(token) = p.token.take() {
                event_loop.deregister(token);
            }
            p.codec.close();
            p.state = PeerState::Closed;
        }
        self.remove_peer(id);
    }
    pub fn update_event(
        &mut self,
        id: PeerId,
        event_loop: &mut EventLoop,
    ) -> Result<(), ProtocolError> {
        let p = self.peers.get_mut(id).ok_or(ProtocolError::Closed)?;
        let read = p.state == PeerState::Active;
        let write = p.codec.queued_output();
        if let Some(token) = p.token {
            event_loop.reregister(token, read, write)?;
        } else {
            p.token = Some(event_loop.register(p.fd.as_fd(), read, write, LoopAction::Peer(id))?);
        }
        Ok(())
    }
    pub fn flush_peer(&mut self, id: PeerId) -> Result<(), ProtocolError> {
        let p = self.peers.get_mut(id).ok_or(ProtocolError::Closed)?;
        p.codec.flush(&p.fd)
    }
    pub fn ready(&mut self, id: PeerId, readable: bool, writable: bool) -> Vec<PeerDispatch> {
        let Some(peer) = self.peers.get_mut(id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while readable && peer.state == PeerState::Active {
            match peer.codec.receive(&peer.fd) {
                Ok(messages) if messages.is_empty() => break,
                Ok(messages) => {
                    for message in messages {
                        out.push(PeerDispatch::Message(id, message));
                    }
                }
                Err(ProtocolError::Version(_)) => {
                    let _ = peer.codec.enqueue(ProtocolMessage::new(
                        ProtocolMessageKind::Version,
                        VERSION.to_le_bytes().to_vec(),
                    ));
                    peer.state = PeerState::BadDraining;
                }
                Err(ProtocolError::Shape(_)) => {
                    peer.state = PeerState::BadDraining;
                }
                Err(_) => {
                    peer.state = PeerState::Closed;
                    out.push(PeerDispatch::Closed(id));
                    return out;
                }
            }
        }
        if writable && peer.codec.flush(&peer.fd).is_err() {
            peer.state = PeerState::Closed;
            out.push(PeerDispatch::Closed(id));
            return out;
        }
        if peer.state == PeerState::BadDraining && !peer.codec.queued_output() {
            peer.state = PeerState::Closed;
            out.push(PeerDispatch::Closed(id));
        }
        out
    }
    pub fn exit(&mut self) {
        let order: Vec<_> = self.order.iter().copied().collect();
        for id in order {
            let _ = self.flush_peer(id);
        }
        self.exiting = true;
    }
    pub fn toggle_log(&mut self) {
        self.logging = !self.logging;
    }
    pub fn retain_peer(&mut self, id: PeerId) -> Result<(), ArenaError> {
        self.peers.retain(id)
    }
    pub fn release_peer(&mut self, id: PeerId) -> Result<(), ArenaError> {
        self.peers.release(id).map(|_| ())
    }
}
pub fn proc_send(
    server: &mut Server,
    peer: PeerId,
    message: ProtocolMessage,
) -> Result<(), ProtocolError> {
    server.process.send(peer, message)?;
    server.process.update_event(peer, &mut server.event_loop)
}
pub fn proc_remove(server: &mut Server, peer: PeerId) {
    server.process.detach_peer(peer, &mut server.event_loop);
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::protocol::encode_string;
    #[test]
    fn bad_peers_reject_sends_but_drain_queued_data() {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut p = Process::new();
        let id = p.add_peer(a.into()).unwrap();
        p.send(
            id,
            ProtocolMessage::new(ProtocolMessageKind::Lock, encode_string(b"lock")),
        )
        .unwrap();
        p.kill_peer(id);
        assert!(
            p.send(id, ProtocolMessage::new(ProtocolMessageKind::Ready, vec![]))
                .is_err()
        );
        let dispatch = p.ready(id, false, true);
        assert!(matches!(dispatch.as_slice(), [PeerDispatch::Closed(_)]));
        drop(b);
    }
    #[test]
    fn removal_invalidates_generation() {
        let (a, _) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut p = Process::new();
        let id = p.add_peer(a.into()).unwrap();
        p.remove_peer(id);
        assert!(p.fd(id).is_none());
    }
    #[test]
    fn malformed_command_keeps_prior_identify_dispatch_before_loss() {
        use std::io::Write;
        let (a, mut b) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut process = Process::new();
        let id = process.add_peer(a.into()).unwrap();
        let frame = |kind: ProtocolMessageKind| {
            let mut bytes = b"RMUX".to_vec();
            bytes.extend_from_slice(&VERSION.to_le_bytes());
            bytes.extend_from_slice(&(kind as u16).to_le_bytes());
            bytes.extend_from_slice(&[0; 8]);
            bytes
        };
        let mut bytes = frame(ProtocolMessageKind::IdentifyDone);
        bytes.extend(frame(ProtocolMessageKind::Command));
        b.write_all(&bytes).unwrap();
        let dispatch = process.ready(id, true, false);
        assert!(
            matches!(&dispatch[0],PeerDispatch::Message(_,message) if message.kind==ProtocolMessageKind::IdentifyDone)
        );
        assert!(matches!(&dispatch[1],PeerDispatch::Closed(peer) if *peer==id));
    }
}
