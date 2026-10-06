// Ported from tmux file.c, window.c @ 8f25579c
/*
 * Copyright (c) 2019 Nicholas Marriott <nicholas.marriott@gmail.com>
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

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use super::event_loop::{EventLoop, LoopAction};
use super::io::BufferedIo;
use super::proc::{Process, proc_send};
use super::protocol::{self, LEGACY_PAYLOAD, ProtocolMessage, ProtocolMessageKind as Kind};
use crate::client::ClientFlags;
use crate::ids::{Arena, ClientFileId, ClientId, EventToken, PeerId, TimerId};
use crate::model::Server;

pub const FILE_CHUNK: usize = LEGACY_PAYLOAD - 4;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilePhase {
    Opening,
    Reading,
    Writing,
    Closing,
    Done,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileNotice {
    ReadProgress,
    Done,
    FlushCheck,
}
pub type ClientFileCallback = Box<dyn FnMut(&mut Server, ClientFileId, FileNotice, &[u8], i32)>;
pub type ClientFiles = BTreeMap<i32, ClientFileId>;
#[derive(Clone, Copy, Debug, Default)]
pub struct FilePolicy {
    pub allow_streams: bool,
    pub close_received: bool,
}

pub struct ClientFile {
    pub stream: i32,
    pub path: Vec<u8>,
    pub phase: FilePhase,
    pub closed: bool,
    pub error: i32,
    pub client: Option<ClientId>,
    pub peer: Option<PeerId>,
    pub io: Option<BufferedIo>,
    pub callback: Option<ClientFileCallback>,
    pub token: Option<EventToken>,
    buffer: Vec<u8>,
    buffer_start: usize,
    callback_consumed: Option<usize>,
    direct: bool,
    server_side: bool,
    done_timer: Option<TimerId>,
    push_timer: Option<TimerId>,
    io_timer: Option<TimerId>,
}
impl ClientFile {
    fn new(stream: i32, path: Vec<u8>, phase: FilePhase, server_side: bool) -> Self {
        Self {
            stream,
            path,
            phase,
            closed: false,
            error: 0,
            client: None,
            peer: None,
            io: None,
            callback: None,
            token: None,
            buffer: Vec::new(),
            buffer_start: 0,
            callback_consumed: None,
            direct: false,
            server_side,
            done_timer: None,
            push_timer: None,
            io_timer: None,
        }
    }
    pub fn data(&self) -> &[u8] {
        &self.buffer[self.buffer_start..]
    }
    fn drain(&mut self, count: usize) {
        self.buffer_start += count.min(self.data().len());
        if self.buffer_start == self.buffer.len() {
            self.buffer.clear();
            self.buffer_start = 0;
        }
    }
}

pub struct FileStore {
    pub arena: Arena<ClientFile, ClientFileId>,
    streams: BTreeMap<(PeerId, i32), ClientFileId>,
    next_stream: i32,
    flush_checks: usize,
    standard_fds: [Option<OwnedFd>; 3],
}
impl Default for FileStore {
    fn default() -> Self {
        Self::new()
    }
}
impl FileStore {
    pub fn new() -> Self {
        Self {
            arena: Arena::new(),
            streams: BTreeMap::new(),
            next_stream: 3,
            flush_checks: 0,
            standard_fds: [None, None, None],
        }
    }
    pub fn get(&self, id: ClientFileId) -> Option<&ClientFile> {
        self.arena.get(id)
    }
    pub fn get_mut(&mut self, id: ClientFileId) -> Option<&mut ClientFile> {
        self.arena.get_mut(id)
    }
    pub fn find(&self, peer: PeerId, stream: i32) -> Option<ClientFileId> {
        self.streams.get(&(peer, stream)).copied()
    }
    pub fn set_standard_fd(&mut self, index: usize, fd: OwnedFd) {
        self.standard_fds[index] = Some(fd);
    }
    pub fn take_flush_checks(&mut self) -> usize {
        std::mem::take(&mut self.flush_checks)
    }
    pub fn write_left(&self) -> bool {
        self.streams.values().any(|id| {
            self.get(*id)
                .is_some_and(|file| file.io.as_ref().is_some_and(|io| io.output_len() != 0))
        })
    }
    pub fn consume(&mut self, id: ClientFileId, count: usize) {
        if let Some(file) = self.get_mut(id) {
            if let Some(consumed) = &mut file.callback_consumed {
                *consumed = consumed.saturating_add(count);
            } else {
                file.drain(count);
            }
        }
    }
    pub fn take_buffer(&mut self, id: ClientFileId) -> Option<Vec<u8>> {
        let file = self.get_mut(id)?;
        if file.callback_consumed.is_some() {
            return None;
        }
        let mut buffer = std::mem::take(&mut file.buffer);
        let start = std::mem::take(&mut file.buffer_start);
        if start != 0 {
            buffer.drain(..start);
        }
        Some(buffer)
    }
    pub fn client_has_buffered(&self, client: ClientId) -> bool {
        self.streams.values().any(|id| {
            self.get(*id)
                .is_some_and(|file| file.client == Some(client) && !file.data().is_empty())
        })
    }
    fn insert(&mut self, file: ClientFile) -> io::Result<ClientFileId> {
        let key = file.peer.map(|peer| (peer, file.stream));
        let id = self.arena.insert(file).map_err(io::Error::other)?;
        if let Some(key) = key {
            self.streams.insert(key, id);
        }
        Ok(id)
    }
    fn allocate_stream(&mut self) -> i32 {
        let stream = self.next_stream;
        self.next_stream = self
            .next_stream
            .checked_add(1)
            .expect("file stream exhaustion");
        stream
    }
    fn remove(&mut self, event_loop: &mut EventLoop, id: ClientFileId) -> Option<ClientFile> {
        let file = self.get_mut(id)?;
        stop_io(file, event_loop);
        let done = file.done_timer.take();
        let push = file.push_timer.take();
        let key = file.peer.map(|peer| (peer, file.stream));
        if let Some(key) = key {
            self.streams.remove(&key);
        }
        if let Some(timer) = done {
            event_loop.cancel(timer);
            let _ = self.arena.release(id);
        }
        if let Some(timer) = push {
            event_loop.cancel(timer);
            let _ = self.arena.release(id);
        }
        self.arena.request_remove(id).ok().flatten()
    }
    pub fn read(
        server: &mut Server,
        client: Option<ClientId>,
        path: &[u8],
        callback: ClientFileCallback,
    ) -> Option<ClientFileId> {
        request(server, client, path, 0, None, callback)
    }
    pub fn write(
        server: &mut Server,
        client: Option<ClientId>,
        path: &[u8],
        flags: i32,
        data: Vec<u8>,
        callback: ClientFileCallback,
    ) -> Option<ClientFileId> {
        request(server, client, path, flags, Some(data), callback)
    }
    pub fn cancel(server: &mut Server, id: ClientFileId) {
        cancel(server, id);
    }

    pub fn handle_client(
        &mut self,
        process: &mut Process,
        event_loop: &mut EventLoop,
        peer: PeerId,
        message: ProtocolMessage,
        policy: FilePolicy,
    ) -> io::Result<()> {
        protocol::validate(&message)?;
        match message.kind {
            Kind::WriteOpen => self.write_open(process, event_loop, peer, &message.data, policy),
            Kind::ReadOpen => self.read_open(process, event_loop, peer, &message.data, policy),
            Kind::WriteData => {
                let (stream, _) = decode_data(&message.data)?;
                let id = self
                    .find(peer, stream)
                    .ok_or_else(|| shape("unknown write stream"))?;
                let file = self.get_mut(id).expect("indexed file");
                if file.phase != FilePhase::Writing || file.closed {
                    return Err(shape("write data after close"));
                }
                if let Some(io) = &mut file.io {
                    io.queue_from(message.data, 4);
                }
                arm(file, event_loop, id)
            }
            Kind::WriteClose => {
                let stream = decode_stream(&message.data)?;
                let id = self
                    .find(peer, stream)
                    .ok_or_else(|| shape("unknown write stream"))?;
                let file = self.get_mut(id).expect("indexed file");
                file.closed = true;
                file.phase = FilePhase::Closing;
                if file.io.as_ref().is_none_or(|io| io.output_len() == 0) {
                    self.finish_write(process, event_loop, id)
                } else {
                    arm(file, event_loop, id)
                }
            }
            Kind::ReadCancel => {
                let stream = decode_stream(&message.data)?;
                let id = self
                    .find(peer, stream)
                    .ok_or_else(|| shape("unknown read stream"))?;
                if self
                    .get(id)
                    .is_none_or(|file| file.phase != FilePhase::Reading)
                {
                    return Err(shape("cancel of nonreading stream"));
                }
                self.finish_read(process, event_loop, id, 0)
            }
            _ => Err(shape("unexpected client file message")),
        }
    }
    fn standard_fd(
        &mut self,
        descriptor: i32,
        expected: &[i32],
        policy: FilePolicy,
    ) -> io::Result<OwnedFd> {
        if !policy.allow_streams || !expected.contains(&descriptor) {
            return Err(io::Error::from_raw_os_error(libc::EBADF));
        }
        let index = descriptor as usize;
        if let Some(original) = self.standard_fds[index].as_ref() {
            let duplicate = original.as_fd().try_clone_to_owned()?;
            if policy.close_received {
                let _ = rmux_sys::server::close(
                    self.standard_fds[index].take().expect("owned standard fd"),
                );
            }
            Ok(duplicate)
        } else if policy.close_received {
            Err(io::Error::from_raw_os_error(libc::EBADF))
        } else {
            rmux_sys::server::duplicate_standard(descriptor)
        }
    }
    fn write_open(
        &mut self,
        process: &mut Process,
        event_loop: &mut EventLoop,
        peer: PeerId,
        data: &[u8],
        policy: FilePolicy,
    ) -> io::Result<()> {
        let (stream, descriptor, flags, path) = decode_open(data, true)?;
        if self.find(peer, stream).is_some() {
            return send(
                process,
                event_loop,
                peer,
                status_message(Kind::WriteReady, stream, libc::EBADF),
            );
        }
        let mut file = ClientFile::new(stream, path.to_vec(), FilePhase::Writing, false);
        file.peer = Some(peer);
        let opened = if descriptor == -1 {
            open_write(path, flags, false)
        } else {
            self.standard_fd(descriptor, &[1, 2], policy)
        };
        let error = match opened.and_then(|fd| install_io(&mut file, fd, false)) {
            Ok(()) => 0,
            Err(error) => errno(&error),
        };
        let id = self.insert(file)?;
        send(
            process,
            event_loop,
            peer,
            status_message(Kind::WriteReady, stream, error),
        )?;
        if let Err(error) = arm(self.get_mut(id).expect("new file"), event_loop, id) {
            self.get_mut(id).expect("new file").error = errno(&error);
            stop_io(self.get_mut(id).expect("new file"), event_loop);
            self.flush_checks += 1;
        }
        Ok(())
    }
    fn read_open(
        &mut self,
        process: &mut Process,
        event_loop: &mut EventLoop,
        peer: PeerId,
        data: &[u8],
        policy: FilePolicy,
    ) -> io::Result<()> {
        let (stream, descriptor, _, path) = decode_open(data, false)?;
        if self.find(peer, stream).is_some() {
            return send(
                process,
                event_loop,
                peer,
                status_message(Kind::ReadDone, stream, libc::EBADF),
            );
        }
        let mut file = ClientFile::new(stream, path.to_vec(), FilePhase::Reading, false);
        file.peer = Some(peer);
        let opened = if descriptor == -1 {
            open_read(path)
        } else {
            self.standard_fd(descriptor, &[0], policy)
        };
        if let Err(error) = opened.and_then(|fd| install_io(&mut file, fd, true)) {
            self.insert(file)?;
            return send(
                process,
                event_loop,
                peer,
                status_message(Kind::ReadDone, stream, errno(&error)),
            );
        }
        let id = self.insert(file)?;
        if arm(self.get_mut(id).expect("new file"), event_loop, id).is_err() {
            return self.finish_read(process, event_loop, id, libc::EIO);
        }
        Ok(())
    }
    fn finish_write(
        &mut self,
        process: &mut Process,
        event_loop: &mut EventLoop,
        id: ClientFileId,
    ) -> io::Result<()> {
        let Some(file) = self.get_mut(id) else {
            return Ok(());
        };
        let stream = file.stream;
        let peer = file.peer.expect("client file peer");
        let mut error = file.error;
        stop_registration(file, event_loop);
        if let Some(mut io) = file.io.take() {
            if let Some(fd) = io.take_fd() {
                if let Err(close_error) = rmux_sys::server::close(fd) {
                    if error == 0 {
                        error = errno(&close_error);
                    }
                }
            }
        }
        let result = send(
            process,
            event_loop,
            peer,
            status_message(Kind::WriteDone, stream, error),
        );
        self.flush_checks += 1;
        self.remove(event_loop, id);
        result
    }
    fn finish_read(
        &mut self,
        process: &mut Process,
        event_loop: &mut EventLoop,
        id: ClientFileId,
        error: i32,
    ) -> io::Result<()> {
        let Some(file) = self.get(id) else {
            return Ok(());
        };
        let message = status_message(Kind::ReadDone, file.stream, error);
        let peer = file.peer.expect("client file peer");
        let result = send(process, event_loop, peer, message);
        self.remove(event_loop, id);
        result
    }
    pub fn client_ready(
        &mut self,
        process: &mut Process,
        event_loop: &mut EventLoop,
        id: ClientFileId,
        readable: bool,
        writable: bool,
    ) -> io::Result<()> {
        let Some(file) = self.get_mut(id) else {
            return Ok(());
        };
        file.io_timer = None;
        let phase = file.phase;
        if phase == FilePhase::Reading {
            let result = if readable || file.direct {
                file.io.as_mut().map(|io| io.read_ready()).transpose()
            } else {
                Ok(None)
            };
            let terminal = match result {
                Ok(progress) => progress.filter(|p| p.eof).map(|_| 0),
                Err(_) => Some(libc::EIO),
            };
            loop {
                let file = self.get(id).expect("active read");
                let Some(io) = &file.io else {
                    break;
                };
                if io.input().is_empty() {
                    break;
                }
                let count = io.input().len().min(FILE_CHUNK);
                let message = data_message(Kind::ReadData, file.stream, &io.input()[..count]);
                let peer = file.peer.expect("client peer");
                send(process, event_loop, peer, message)?;
                self.get_mut(id)
                    .expect("active read")
                    .io
                    .as_mut()
                    .expect("read io")
                    .consume_input(count);
            }
            if let Some(error) = terminal {
                return self.finish_read(process, event_loop, id, error);
            }
            arm(self.get_mut(id).expect("active read"), event_loop, id)
        } else if matches!(phase, FilePhase::Writing | FilePhase::Closing) {
            let result = if writable || file.direct {
                file.io.as_mut().map(|io| io.write_ready()).transpose()
            } else {
                Ok(None)
            };
            if let Err(error) = result {
                let file = self.get_mut(id).expect("active write");
                file.error = errno(&error);
                stop_io(file, event_loop);
            }
            let file = self.get_mut(id).expect("active write");
            if file.closed && file.io.as_ref().is_none_or(|io| io.output_len() == 0) {
                return self.finish_write(process, event_loop, id);
            }
            self.flush_checks += 1;
            arm(self.get_mut(id).expect("active write"), event_loop, id)
        } else {
            Ok(())
        }
    }
    pub fn close_peer(&mut self, event_loop: &mut EventLoop, peer: PeerId) {
        let ids: Vec<_> = self
            .streams
            .range((peer, i32::MIN)..=(peer, i32::MAX))
            .map(|(_, id)| *id)
            .collect();
        for id in ids {
            self.remove(event_loop, id);
        }
    }
    pub fn handle_server(
        server: &mut Server,
        client: ClientId,
        message: ProtocolMessage,
    ) -> io::Result<()> {
        handle_server(server, client, message)
    }
}

fn shape(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn errno(error: &io::Error) -> i32 {
    error
        .raw_os_error()
        .filter(|error| *error != 0)
        .unwrap_or(libc::EIO)
}
fn send(
    process: &mut Process,
    event_loop: &mut EventLoop,
    peer: PeerId,
    message: ProtocolMessage,
) -> io::Result<()> {
    process.send(peer, message)?;
    process.update_event(peer, event_loop)?;
    Ok(())
}
fn status_message(kind: Kind, stream: i32, error: i32) -> ProtocolMessage {
    ProtocolMessage::new(kind, protocol::encode_i32s(&[stream, error]))
}
fn data_message(kind: Kind, stream: i32, bytes: &[u8]) -> ProtocolMessage {
    let mut data = Vec::with_capacity(4 + bytes.len());
    data.extend_from_slice(&stream.to_le_bytes());
    data.extend_from_slice(bytes);
    ProtocolMessage::new(kind, data)
}
fn open_message(
    kind: Kind,
    stream: i32,
    descriptor: i32,
    flags: i32,
    path: &[u8],
) -> ProtocolMessage {
    let mut data = protocol::encode_i32s(&[stream, descriptor]);
    if kind == Kind::WriteOpen {
        data.extend_from_slice(&flags.to_le_bytes());
    }
    data.extend_from_slice(&protocol::encode_string(path));
    ProtocolMessage::new(kind, data)
}
fn decode_stream(data: &[u8]) -> io::Result<i32> {
    Ok(protocol::decode_i32(data)?)
}
fn decode_data(data: &[u8]) -> io::Result<(i32, &[u8])> {
    if data.len() < 4 || data.len() > LEGACY_PAYLOAD {
        return Err(shape("bad file data size"));
    }
    Ok((decode_stream(&data[..4])?, &data[4..]))
}
fn decode_open(data: &[u8], write: bool) -> io::Result<(i32, i32, i32, &[u8])> {
    let offset = if write { 12 } else { 8 };
    if data.len() < offset {
        return Err(shape("bad file open size"));
    }
    let stream = decode_stream(&data[..4])?;
    let descriptor = decode_stream(&data[4..8])?;
    let flags = if write {
        decode_stream(&data[8..12])?
    } else {
        0
    };
    let mut tail = &data[offset..];
    let path = protocol::take_string(&mut tail)?;
    if !tail.is_empty() || path.contains(&0) || offset + path.len() + 1 > LEGACY_PAYLOAD {
        return Err(shape("bad file path"));
    }
    Ok((stream, descriptor, flags, path))
}

pub fn get_path(path: &[u8], cwd: &[u8], home: Option<&[u8]>) -> Vec<u8> {
    let expanded;
    let path = if path.starts_with(b"~/") {
        let mut bytes = home.unwrap_or_default().to_vec();
        bytes.extend_from_slice(&path[1..]);
        expanded = bytes;
        &expanded[..]
    } else {
        path
    };
    if path.starts_with(b"/") {
        return path.to_vec();
    }
    let mut full = Vec::with_capacity(cwd.len() + 1 + path.len());
    full.extend_from_slice(cwd);
    full.push(b'/');
    full.extend_from_slice(path);
    full
}
fn open_read(path: &[u8]) -> io::Result<OwnedFd> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(OsStr::from_bytes(path))?
        .into())
}
fn open_write(path: &[u8], flags: i32, local: bool) -> io::Result<OwnedFd> {
    let append = flags & libc::O_APPEND != 0;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create(true)
        .mode(if local { 0o666 } else { 0o644 });
    if local {
        options.append(append).truncate(!append);
    } else {
        options
            .append(append)
            .truncate(flags & libc::O_TRUNC != 0)
            .custom_flags(flags | libc::O_NONBLOCK);
    }
    Ok(options.open(OsStr::from_bytes(path))?.into())
}
fn install_io(file: &mut ClientFile, fd: OwnedFd, reading: bool) -> io::Result<()> {
    file.direct = rmux_sys::server::descriptor_is_regular(fd.as_fd())?;
    let mut io = BufferedIo::new(fd);
    io.enable_read(reading);
    io.enable_write(!reading);
    io.set_watermarks(0, Some(super::io::IO_BUDGET), 0);
    file.io = Some(io);
    Ok(())
}
fn arm(file: &mut ClientFile, event_loop: &mut EventLoop, id: ClientFileId) -> io::Result<()> {
    let Some(io) = &mut file.io else {
        return Ok(());
    };
    let (read, write) = io.interests();
    if file.direct {
        if (read || write) && file.io_timer.is_none() {
            file.io_timer = Some(event_loop.schedule(Duration::ZERO, LoopAction::File(id)));
        }
    } else if let Some(token) = file.token {
        event_loop.reregister(token, read, write)?;
        io.set_null(event_loop.is_null(token));
    } else {
        let token = event_loop.register(io.fd(), read, write, LoopAction::File(id))?;
        io.set_null(event_loop.is_null(token));
        file.token = Some(token);
    }
    Ok(())
}
fn stop_registration(file: &mut ClientFile, event_loop: &mut EventLoop) {
    if let Some(token) = file.token.take() {
        event_loop.deregister(token);
    }
    if let Some(timer) = file.io_timer.take() {
        event_loop.cancel(timer);
    }
}
fn stop_io(file: &mut ClientFile, event_loop: &mut EventLoop) {
    stop_registration(file, event_loop);
    file.io = None;
}
fn create(
    server: &mut Server,
    client: Option<ClientId>,
    stream: i32,
    path: Vec<u8>,
    phase: FilePhase,
    callback: Option<ClientFileCallback>,
) -> Option<ClientFileId> {
    let client = client.filter(|id| {
        server
            .clients
            .get(*id)
            .is_some_and(|c| !c.flags.contains(ClientFlags::ATTACHED))
    });
    let mut file = ClientFile::new(stream, path, phase, true);
    file.client = client;
    file.callback = callback;
    if let Some(client) = client {
        file.peer = server.clients.get(client).and_then(|client| client.peer);
        crate::client::lifecycle::retain(server, client).ok()?;
    }
    match server.files.insert(file) {
        Ok(id) => Some(id),
        Err(_) => {
            if let Some(client) = client {
                let _ = crate::client::lifecycle::release(server, client);
            }
            None
        }
    }
}
fn request(
    server: &mut Server,
    client: Option<ClientId>,
    path: &[u8],
    flags: i32,
    data: Option<Vec<u8>>,
    callback: ClientFileCallback,
) -> Option<ClientFileId> {
    let writing = data.is_some();
    let stream = server.files.allocate_stream();
    let standard = path == b"-";
    let flags_client = client
        .and_then(|id| server.clients.get(id))
        .map(|c| c.flags);
    let remote = flags_client.is_some_and(|flags| !flags.contains(ClientFlags::ATTACHED));
    let path = if standard {
        path.to_vec()
    } else {
        let cwd = crate::client::registry::get_cwd(server, client, None);
        let home = server
            .global_environment
            .find(b"HOME")
            .and_then(|entry| entry.value.as_ref())
            .map(|value| value.as_bytes());
        let fallback;
        let home = if home.is_none() {
            fallback = rmux_sys::proc::home_directory(None);
            fallback.as_deref()
        } else {
            home
        };
        get_path(path, &cwd, home)
    };
    let id = create(
        server,
        client,
        stream,
        path,
        FilePhase::Opening,
        Some(callback),
    )?;
    if standard
        && (!remote || flags_client.is_some_and(|flags| flags.contains(ClientFlags::CONTROL)))
    {
        server.files.get_mut(id).expect("new file").error = libc::EBADF;
        schedule_done(server, id);
        return None;
    }
    if remote {
        if let Some(data) = data {
            server.files.get_mut(id).expect("new file").buffer = data;
        }
        let file = server.files.get(id).expect("new file");
        let overhead = if writing { 12 } else { 8 };
        let peer = file.peer;
        let error = if file.path.len() + 1 + overhead > LEGACY_PAYLOAD {
            libc::E2BIG
        } else {
            let message = open_message(
                if writing {
                    Kind::WriteOpen
                } else {
                    Kind::ReadOpen
                },
                stream,
                if standard {
                    if writing { 1 } else { 0 }
                } else {
                    -1
                },
                flags,
                &file.path,
            );
            if peer.is_none_or(|peer| proc_send(server, peer, message).is_err()) {
                libc::EINVAL
            } else {
                0
            }
        };
        if error != 0 {
            server.files.get_mut(id).expect("new file").error = error;
            schedule_done(server, id);
            return None;
        }
        return Some(id);
    }
    let file = server.files.get_mut(id).expect("new file");
    let opened = if writing {
        open_write(&file.path, flags, true)
    } else {
        open_read(&file.path)
    };
    let result = opened.and_then(|fd| install_io(file, fd, !writing));
    if let Err(error) = result {
        file.error = errno(&error);
        schedule_done(server, id);
        return None;
    }
    file.phase = if writing {
        FilePhase::Writing
    } else {
        FilePhase::Reading
    };
    if let Some(data) = data {
        file.io.as_mut().expect("local io").queue(data);
    }
    if file.direct {
        // file_read/file_write (file.c:395-433, 321-340) read or write a
        // local file at once, so done fires on the next loop turn. Run the
        // first bounded pass now to keep that timing: a control client's
        // stdin EOF must not overtake a source-file's error output.
        on_ready(server, id, false, false);
        return Some(id);
    }
    if let Err(error) = arm(file, &mut server.event_loop, id) {
        file.error = errno(&error);
        schedule_done(server, id);
        return None;
    }
    if writing && file.io.as_ref().is_some_and(|io| io.output_len() == 0) {
        schedule_done(server, id);
    }
    Some(id)
}

pub fn can_print(server: &Server, client: Option<ClientId>) -> bool {
    client
        .and_then(|id| server.clients.get(id))
        .is_some_and(|c| {
            !c.flags
                .intersects(ClientFlags::ATTACHED | ClientFlags::DEAD | ClientFlags::CONTROL)
        })
}
pub fn print(server: &mut Server, client: ClientId, bytes: &[u8]) {
    print_stream(server, client, 1, bytes);
}
pub fn print_buffer(server: &mut Server, client: ClientId, bytes: &[u8]) {
    print(server, client, bytes);
}
pub fn error(server: &mut Server, client: ClientId, bytes: &[u8]) {
    print_stream(server, client, 2, bytes);
}

pub fn print_unchecked(server: &mut Server, client: ClientId, bytes: &[u8]) {
    let Some(source) = server.clients.get(client) else {
        return;
    };
    if source
        .flags
        .intersects(ClientFlags::DEAD | ClientFlags::CONTROL)
    {
        return;
    }
    let Some(peer) = source.peer else {
        return;
    };
    if let Some(id) = server.files.find(peer, 1) {
        let file = server.files.get_mut(id).expect("indexed stdout");
        file.buffer.extend_from_slice(bytes);
        if file.phase == FilePhase::Writing {
            push(server, id);
        }
        return;
    }
    if crate::client::lifecycle::retain(server, client).is_err() {
        return;
    }
    let mut file = ClientFile::new(1, b"-".to_vec(), FilePhase::Opening, true);
    file.peer = Some(peer);
    file.client = Some(client);
    file.buffer.extend_from_slice(bytes);
    match server.files.insert(file) {
        Ok(_) => {
            let _ = proc_send(server, peer, open_message(Kind::WriteOpen, 1, 1, 0, b"-"));
        }
        Err(_) => {
            let _ = crate::client::lifecycle::release(server, client);
        }
    }
}
fn print_stream(server: &mut Server, client: ClientId, stream: i32, bytes: &[u8]) {
    if !can_print(server, Some(client)) {
        return;
    }
    let Some(peer) = server.clients.get(client).and_then(|client| client.peer) else {
        return;
    };
    if let Some(id) = server.files.find(peer, stream) {
        let file = server.files.get_mut(id).expect("indexed file");
        file.buffer.extend_from_slice(bytes);
        if file.phase == FilePhase::Writing {
            push(server, id);
        }
    } else if let Some(id) = create(
        server,
        Some(client),
        stream,
        b"-".to_vec(),
        FilePhase::Opening,
        None,
    ) {
        server
            .files
            .get_mut(id)
            .expect("new file")
            .buffer
            .extend_from_slice(bytes);
        let _ = proc_send(
            server,
            peer,
            open_message(Kind::WriteOpen, stream, stream, 0, b"-"),
        );
    }
}
pub fn read(
    server: &mut Server,
    client: Option<ClientId>,
    path: &[u8],
    callback: ClientFileCallback,
) -> Option<ClientFileId> {
    FileStore::read(server, client, path, callback)
}
pub fn write(
    server: &mut Server,
    client: Option<ClientId>,
    path: &[u8],
    flags: i32,
    data: Vec<u8>,
    callback: ClientFileCallback,
) -> Option<ClientFileId> {
    FileStore::write(server, client, path, flags, data, callback)
}
pub fn cancel(server: &mut Server, id: ClientFileId) {
    let Some(file) = server.files.get_mut(id) else {
        return;
    };
    if file.closed {
        return;
    }
    file.closed = true;
    let peer = file.peer;
    let stream = file.stream;
    if let Some(peer) = peer {
        let _ = proc_send(
            server,
            peer,
            ProtocolMessage::new(Kind::ReadCancel, protocol::encode_i32s(&[stream])),
        );
    } else {
        schedule_done(server, id);
    }
}
fn schedule_done(server: &mut Server, id: ClientFileId) {
    let Some(file) = server.files.get_mut(id) else {
        return;
    };
    if file.done_timer.is_some() {
        return;
    }
    stop_io(file, &mut server.event_loop);
    file.phase = FilePhase::Done;
    server.files.arena.retain(id).expect("done file lease");
    let timer = server
        .event_loop
        .schedule(Duration::ZERO, LoopAction::FileDone(id));
    server.files.get_mut(id).expect("leased file").done_timer = Some(timer);
}
fn callback(server: &mut Server, id: ClientFileId, notice: FileNotice) {
    let Some(file) = server.files.get_mut(id) else {
        return;
    };
    let Some(mut callback) = file.callback.take() else {
        return;
    };
    let bytes = std::mem::take(&mut file.buffer);
    let start = std::mem::take(&mut file.buffer_start);
    let error = file.error;
    file.callback_consumed = Some(0);
    callback(server, id, notice, &bytes[start..], error);
    if let Some(file) = server.files.get_mut(id) {
        let consumed = file
            .callback_consumed
            .take()
            .unwrap_or_default()
            .min(bytes.len() - start);
        let appended = std::mem::replace(&mut file.buffer, bytes);
        file.buffer_start = start + consumed;
        file.buffer.extend_from_slice(&appended);
        file.callback = Some(callback);
    }
}
pub fn fire_done(server: &mut Server, id: ClientFileId) {
    let Some(file) = server.files.get_mut(id) else {
        return;
    };
    if file.done_timer.take().is_none() {
        return;
    }
    let deliver = file.closed
        || file.client.is_none_or(|client| {
            server
                .clients
                .get(client)
                .is_some_and(|client| !client.flags.contains(ClientFlags::DEAD))
        });
    if deliver {
        callback(server, id, FileNotice::Done);
    }
    let client = server.files.get(id).and_then(|file| file.client);
    let _ = server.files.remove(&mut server.event_loop, id);
    let _ = server.files.arena.release(id);
    if let Some(client) = client {
        let _ = crate::client::lifecycle::release(server, client);
    }
}
pub fn push(server: &mut Server, id: ClientFileId) {
    let Some(file) = server.files.get_mut(id) else {
        return;
    };
    if let Some(timer) = file.push_timer.take() {
        server.event_loop.cancel(timer);
        let _ = server.files.arena.release(id);
    }
    loop {
        let Some(file) = server.files.get(id) else {
            return;
        };
        if file.phase != FilePhase::Writing {
            return;
        }
        if file.client.is_some_and(|client| {
            server
                .clients
                .get(client)
                .is_none_or(|client| client.flags.contains(ClientFlags::DEAD))
        }) {
            return;
        }
        if file.data().is_empty() {
            break;
        }
        let count = file.data().len().min(FILE_CHUNK);
        let Some(peer) = file.peer else {
            return;
        };
        let message = data_message(Kind::WriteData, file.stream, &file.data()[..count]);
        if proc_send(server, peer, message).is_err() {
            server.files.arena.retain(id).expect("push file lease");
            let timer = server
                .event_loop
                .schedule(Duration::ZERO, LoopAction::FilePush(id));
            server.files.get_mut(id).expect("leased file").push_timer = Some(timer);
            return;
        }
        server.files.get_mut(id).expect("active file").drain(count);
    }
    let file = server.files.get_mut(id).expect("active file");
    if file.stream <= 2 {
        return;
    }
    file.phase = FilePhase::Closing;
    let peer = file.peer;
    let stream = file.stream;
    let ack = file.client.is_some_and(|client| {
        server
            .clients
            .get(client)
            .is_some_and(|client| client.flags.contains(ClientFlags::WRITE_ACK))
    });
    if let Some(peer) = peer {
        let _ = proc_send(
            server,
            peer,
            ProtocolMessage::new(Kind::WriteClose, protocol::encode_i32s(&[stream])),
        );
    }
    if !ack {
        schedule_done(server, id);
    }
}
pub fn handle_server(
    server: &mut Server,
    client: ClientId,
    message: ProtocolMessage,
) -> io::Result<()> {
    protocol::validate(&message)?;
    let data = &message.data;
    let stream = if data.len() >= 4 {
        decode_stream(&data[..4])?
    } else {
        return Err(shape("short file reply"));
    };
    let Some(peer) = server.clients.get(client).and_then(|client| client.peer) else {
        return Ok(());
    };
    let Some(id) = server.files.find(peer, stream) else {
        return Ok(());
    };
    if server
        .files
        .get(id)
        .is_none_or(|file| file.phase == FilePhase::Done)
    {
        return Ok(());
    }
    match message.kind {
        Kind::WriteReady => {
            let error = decode_stream(&data[4..])?;
            if error != 0 {
                server.files.get_mut(id).expect("active file").error = error;
                schedule_done(server, id);
            } else {
                server.files.get_mut(id).expect("active file").phase = FilePhase::Writing;
                push(server, id);
            }
        }
        Kind::WriteDone => {
            if server
                .clients
                .get(client)
                .is_some_and(|client| client.flags.contains(ClientFlags::WRITE_ACK))
            {
                server.files.get_mut(id).expect("active file").error = decode_stream(&data[4..])?;
                schedule_done(server, id);
            }
        }
        Kind::ReadData => {
            let (_, bytes) = decode_data(data)?;
            let file = server.files.get_mut(id).expect("active file");
            if file.error == 0 && !file.closed {
                file.phase = FilePhase::Reading;
                file.buffer.extend_from_slice(bytes);
                callback(server, id, FileNotice::ReadProgress);
            }
        }
        Kind::ReadDone => {
            server.files.get_mut(id).expect("active file").error = decode_stream(&data[4..])?;
            schedule_done(server, id);
        }
        _ => return Err(shape("unexpected server file message")),
    }
    Ok(())
}
pub fn on_ready(server: &mut Server, id: ClientFileId, readable: bool, writable: bool) {
    let Some(file) = server.files.get_mut(id) else {
        return;
    };
    if !file.server_side {
        return;
    }
    file.io_timer = None;
    let reading = file.phase == FilePhase::Reading;
    let Some(io) = &mut file.io else {
        return;
    };
    let result = if reading && (readable || file.direct) {
        io.read_ready()
    } else if !reading && (writable || file.direct) {
        io.write_ready()
    } else {
        return;
    };
    if reading {
        let bytes = io.take_input();
        if file.buffer.is_empty() {
            file.buffer = bytes;
        } else {
            file.buffer.extend_from_slice(&bytes);
        }
    }
    match result {
        Ok(progress) => {
            if (reading && progress.eof) || (!reading && progress.drained) {
                schedule_done(server, id);
                return;
            }
        }
        Err(error) => {
            file.error = if reading { errno(&error) } else { libc::EIO };
            schedule_done(server, id);
            return;
        }
    }
    if let Err(error) = arm(
        server.files.get_mut(id).expect("active file"),
        &mut server.event_loop,
        id,
    ) {
        server.files.get_mut(id).expect("active file").error = errno(&error);
        schedule_done(server, id);
    }
}
pub fn lost_client(server: &mut Server, client: ClientId) {
    let ids: Vec<_> = server
        .files
        .streams
        .values()
        .copied()
        .filter(|id| {
            server
                .files
                .get(*id)
                .is_some_and(|file| file.client == Some(client))
        })
        .collect();
    for id in ids {
        server.files.get_mut(id).expect("client file").error = libc::EINTR;
        schedule_done(server, id);
    }
}

pub fn start_pane_input(
    server: &mut Server,
    pane: crate::ids::PaneId,
    item: crate::ids::QueueItemId,
) -> Result<crate::cmd::queue::CmdReturn, Vec<u8>> {
    use crate::cmd::queue::{self, CmdReturn};
    use crate::model::PaneFlags;
    let target = server
        .panes
        .get(pane)
        .ok_or_else(|| b"no such pane".to_vec())?;
    if !target.flags.contains(PaneFlags::EMPTY) {
        return Err(b"pane is not empty".to_vec());
    }
    let client = server
        .queue
        .items
        .get(item)
        .and_then(|item| item.client)
        .ok_or_else(|| b"no current client".to_vec())?;
    let source = server
        .clients
        .get(client)
        .ok_or_else(|| b"no current client".to_vec())?;
    if source
        .flags
        .intersects(ClientFlags::DEAD | ClientFlags::EXITED)
        || source.session.is_some()
    {
        return Ok(CmdReturn::Normal);
    }
    let mut finished = false;
    let callback: ClientFileCallback = Box::new(move |server, file, notice, bytes, error| {
        if finished {
            return;
        }
        let alive = server
            .panes
            .get(pane)
            .is_some_and(|pane| !pane.flags.contains(PaneFlags::DESTROYED));
        let dead = server
            .clients
            .get(client)
            .is_none_or(|client| client.flags.contains(ClientFlags::DEAD));
        if notice != FileNotice::Done && (!alive || dead) {
            if !alive {
                if let Some(client) = server.clients.get_mut(client) {
                    client.retval = 1;
                    client.flags.insert(ClientFlags::EXIT);
                }
            }
            cancel(server, file);
        } else if notice == FileNotice::Done || error != 0 {
            finished = true;
            queue::continue_item(&mut server.queue, item);
        } else {
            let _ = super::run::pane_parse_buffer(server, pane, bytes);
        }
        server.files.consume(file, bytes.len());
    });
    let _ = read(server, Some(client), b"-", callback);
    Ok(CmdReturn::Wait)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::protocol::{ProtocolBackend, RmuxBackend};
    use std::cell::RefCell;
    use std::os::unix::net::UnixStream;
    use std::rc::Rc;

    fn endpoint() -> (FileStore, Process, EventLoop, PeerId, OwnedFd, RmuxBackend) {
        let (left, right) = UnixStream::pair().unwrap();
        let mut process = Process::new();
        let peer = process.add_peer(left.into()).unwrap();
        let fd: OwnedFd = right.into();
        rmux_sys::fd::set_blocking(fd.as_fd(), false);
        (
            FileStore::new(),
            process,
            EventLoop::new().unwrap(),
            peer,
            fd,
            RmuxBackend::default(),
        )
    }
    fn receive(
        process: &mut Process,
        peer: PeerId,
        fd: &OwnedFd,
        codec: &mut RmuxBackend,
    ) -> Vec<ProtocolMessage> {
        let mut messages = Vec::new();
        loop {
            process.flush_peer(peer).unwrap();
            let batch = codec.receive(fd).unwrap();
            if batch.is_empty() && !process.queued_output(peer) {
                return messages;
            }
            messages.extend(batch);
        }
    }
    fn temp_path() -> std::path::PathBuf {
        static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "rmux-file-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }
    fn server_client(ack: bool) -> (Server, ClientId, PeerId, OwnedFd, RmuxBackend) {
        let mut server = Server::new();
        let (left, right) = UnixStream::pair().unwrap();
        let peer = server.process.add_peer(left.into()).unwrap();
        let mut client = crate::client::Client::new(Some(peer), (0, 0));
        if ack {
            client.flags.insert(ClientFlags::WRITE_ACK);
        }
        let client = server.clients.insert(client).unwrap();
        server.client_order.push_back(client);
        let fd: OwnedFd = right.into();
        rmux_sys::fd::set_blocking(fd.as_fd(), false);
        (server, client, peer, fd, RmuxBackend::default())
    }
    fn drive_local(server: &mut Server, id: ClientFileId) {
        while server
            .files
            .get(id)
            .is_some_and(|file| file.phase != FilePhase::Done)
        {
            on_ready(server, id, false, false);
        }
        fire_done(server, id);
    }

    #[test]
    fn path_expansion_preserves_noncanonical_bytes() {
        assert_eq!(
            get_path(b"~/a/../b", b"/cwd", Some(b"/home/me")),
            b"/home/me/a/../b"
        );
        assert_eq!(get_path(b"~/a", b"/cwd", None), b"/a");
        assert_eq!(
            get_path(b"~other/a", b"/cwd", Some(b"/home")),
            b"/cwd/~other/a"
        );
        assert_eq!(get_path(b"a//../\xff", b"/cwd", None), b"/cwd/a//../\xff");
        assert_eq!(get_path(b"", b"/cwd", None), b"/cwd/");
    }

    #[test]
    fn chunks_fit_exact_legacy_budget_and_reject_underflow() {
        let message = data_message(Kind::WriteData, 3, &vec![0xff; FILE_CHUNK]);
        assert_eq!(message.data.len(), LEGACY_PAYLOAD);
        protocol::validate(&message).unwrap();
        assert!(decode_data(&[0, 0, 0]).is_err());
        assert!(decode_data(&vec![0; LEGACY_PAYLOAD + 1]).is_err());
        let path = vec![b'a'; LEGACY_PAYLOAD - 12 - 1];
        protocol::validate(&open_message(Kind::WriteOpen, 3, -1, 0, &path)).unwrap();
        assert!(
            protocol::validate(&open_message(
                Kind::WriteOpen,
                3,
                -1,
                0,
                &vec![b'a'; path.len() + 1]
            ))
            .is_err()
        );
    }

    #[test]
    fn local_binary_read_write_append_and_deferred_completion() {
        let path = temp_path();
        let name = path.as_os_str().as_bytes();
        let mut server = Server::new();
        let notices = Rc::new(RefCell::new(Vec::new()));
        let observed = notices.clone();
        let id = write(
            &mut server,
            None,
            name,
            0,
            b"\0\xff\n".to_vec(),
            Box::new(move |_, _, notice, _, error| {
                observed.borrow_mut().push((notice, error));
            }),
        )
        .unwrap();
        assert!(notices.borrow().is_empty());
        drive_local(&mut server, id);
        assert_eq!(*notices.borrow(), vec![(FileNotice::Done, 0)]);
        let id = write(
            &mut server,
            None,
            name,
            libc::O_APPEND,
            b"tail".to_vec(),
            Box::new(|_, _, _, _, _| {}),
        )
        .unwrap();
        drive_local(&mut server, id);
        let output = Rc::new(RefCell::new(Vec::new()));
        let observed = output.clone();
        let id = read(
            &mut server,
            None,
            name,
            Box::new(move |_, _, notice, data, error| {
                assert_eq!(notice, FileNotice::Done);
                assert_eq!(error, 0);
                observed.borrow_mut().extend_from_slice(data);
            }),
        )
        .unwrap();
        drive_local(&mut server, id);
        assert_eq!(*output.borrow(), b"\0\xff\ntail");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn null_client_writer_drains_and_finishes_after_close() {
        let (mut files, mut process, mut event_loop, peer, fd, mut codec) = endpoint();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::WriteOpen, 3, -1, 0, b"/dev/null"),
                FilePolicy::default(),
            )
            .unwrap();
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages[0].kind, Kind::WriteReady);
        let id = files.find(peer, 3).unwrap();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                data_message(Kind::WriteData, 3, b"\0\xff"),
                FilePolicy::default(),
            )
            .unwrap();
        assert!(files.write_left());
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                ProtocolMessage::new(Kind::WriteClose, protocol::encode_i32s(&[3])),
                FilePolicy::default(),
            )
            .unwrap();
        assert!(files.get(id).is_some());
        files
            .client_ready(&mut process, &mut event_loop, id, false, true)
            .unwrap();
        assert!(!files.write_left());
        assert!(files.get(id).is_none());
        assert_eq!(files.take_flush_checks(), 1);
        assert_eq!(
            receive(&mut process, peer, &fd, &mut codec)[0].kind,
            Kind::WriteDone
        );
    }

    #[test]
    fn client_remote_binary_read_chunks_and_cancel() {
        let path = temp_path();
        let name = path.as_os_str().as_bytes();
        let bytes: Vec<u8> = (0..FILE_CHUNK * 2 + 7).map(|index| index as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let (mut files, mut process, mut event_loop, peer, fd, mut codec) = endpoint();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::ReadOpen, 3, -1, 0, name),
                FilePolicy::default(),
            )
            .unwrap();
        let id = files.find(peer, 3).unwrap();
        files
            .client_ready(&mut process, &mut event_loop, id, false, false)
            .unwrap();
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages.last().unwrap().kind, Kind::ReadDone);
        let data: Vec<u8> = messages
            .iter()
            .filter(|message| message.kind == Kind::ReadData)
            .flat_map(|message| message.data[4..].iter().copied())
            .collect();
        assert_eq!(data, bytes);
        assert_eq!(messages[0].data.len(), LEGACY_PAYLOAD);
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::ReadOpen, 4, -1, 0, name),
                FilePolicy::default(),
            )
            .unwrap();
        let id = files.find(peer, 4).unwrap();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                ProtocolMessage::new(Kind::ReadCancel, protocol::encode_i32s(&[4])),
                FilePolicy::default(),
            )
            .unwrap();
        assert!(files.get(id).is_none());
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].data, protocol::encode_i32s(&[4, 0]));
        assert!(
            files
                .handle_client(
                    &mut process,
                    &mut event_loop,
                    peer,
                    ProtocolMessage::new(Kind::ReadCancel, protocol::encode_i32s(&[4])),
                    FilePolicy::default()
                )
                .is_err()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn late_write_error_waits_for_close_and_reports_errno() {
        rmux_sys::server::ignore_sigpipe().unwrap();
        let (mut files, mut process, mut event_loop, peer, fd, mut codec) = endpoint();
        let (sink, other) = UnixStream::pair().unwrap();
        drop(other);
        files.set_standard_fd(1, sink.into());
        let policy = FilePolicy {
            allow_streams: true,
            close_received: true,
        };
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::WriteOpen, 3, 1, 0, b"-"),
                policy,
            )
            .unwrap();
        receive(&mut process, peer, &fd, &mut codec);
        let id = files.find(peer, 3).unwrap();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                data_message(Kind::WriteData, 3, b"failure"),
                policy,
            )
            .unwrap();
        files
            .client_ready(&mut process, &mut event_loop, id, false, true)
            .unwrap();
        assert_eq!(files.get(id).unwrap().error, libc::EPIPE);
        assert!(files.get(id).unwrap().io.is_none());
        assert!(!files.write_left());
        assert_eq!(files.take_flush_checks(), 1);
        assert!(receive(&mut process, peer, &fd, &mut codec).is_empty());
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                ProtocolMessage::new(Kind::WriteClose, protocol::encode_i32s(&[3])),
                policy,
            )
            .unwrap();
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages[0].data, protocol::encode_i32s(&[3, libc::EPIPE]));
        assert_eq!(files.take_flush_checks(), 1);
    }

    #[test]
    fn progress_can_consume_cancel_and_done_is_deferred() {
        let (mut server, client, peer, fd, mut codec) = server_client(false);
        let notices = Rc::new(RefCell::new(Vec::new()));
        let observed = notices.clone();
        let id = read(
            &mut server,
            Some(client),
            b"-",
            Box::new(move |server, id, notice, bytes, error| {
                assert_eq!(error, 0);
                observed.borrow_mut().push((notice, bytes.to_vec()));
                if notice == FileNotice::ReadProgress {
                    server.files.consume(id, bytes.len());
                    cancel(server, id);
                    cancel(server, id);
                }
            }),
        )
        .unwrap();
        receive(&mut server.process, peer, &fd, &mut codec);
        let stream = server.files.get(id).unwrap().stream;
        handle_server(
            &mut server,
            client,
            data_message(Kind::ReadData, stream, b"\0\xff"),
        )
        .unwrap();
        let messages = receive(&mut server.process, peer, &fd, &mut codec);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, Kind::ReadCancel);
        handle_server(
            &mut server,
            client,
            data_message(Kind::ReadData, stream, b"ignored"),
        )
        .unwrap();
        handle_server(
            &mut server,
            client,
            status_message(Kind::ReadDone, stream, 0),
        )
        .unwrap();
        assert_eq!(notices.borrow().len(), 1);
        fire_done(&mut server, id);
        assert_eq!(notices.borrow()[1], (FileNotice::Done, Vec::new()));
        assert!(server.files.get(id).is_none());
        handle_server(
            &mut server,
            client,
            status_message(Kind::ReadDone, stream, libc::EIO),
        )
        .unwrap();
    }

    #[test]
    fn remote_write_ack_preserves_late_error_and_binary_shape() {
        let (mut server, client, peer, fd, mut codec) = server_client(true);
        let result = Rc::new(RefCell::new(None));
        let observed = result.clone();
        let bytes = vec![0xff; FILE_CHUNK + 3];
        let id = write(
            &mut server,
            Some(client),
            b"-",
            0,
            bytes.clone(),
            Box::new(move |_, _, notice, _, error| {
                assert_eq!(notice, FileNotice::Done);
                *observed.borrow_mut() = Some(error);
            }),
        )
        .unwrap();
        let stream = server.files.get(id).unwrap().stream;
        receive(&mut server.process, peer, &fd, &mut codec);
        handle_server(
            &mut server,
            client,
            status_message(Kind::WriteReady, stream, 0),
        )
        .unwrap();
        let messages = receive(&mut server.process, peer, &fd, &mut codec);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].data.len(), LEGACY_PAYLOAD);
        assert_eq!(messages[2].kind, Kind::WriteClose);
        let sent: Vec<u8> = messages[..2]
            .iter()
            .flat_map(|message| message.data[4..].iter().copied())
            .collect();
        assert_eq!(sent, bytes);
        assert_eq!(server.files.get(id).unwrap().phase, FilePhase::Closing);
        assert_eq!(*result.borrow(), None);
        handle_server(
            &mut server,
            client,
            status_message(Kind::WriteDone, stream, libc::EIO),
        )
        .unwrap();
        assert_eq!(*result.borrow(), None);
        fire_done(&mut server, id);
        assert_eq!(*result.borrow(), Some(libc::EIO));
    }

    #[test]
    fn client_named_binary_write_closes_only_after_drain() {
        let path = temp_path();
        let name = path.as_os_str().as_bytes();
        let (mut files, mut process, mut event_loop, peer, fd, mut codec) = endpoint();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::WriteOpen, 3, -1, libc::O_TRUNC, name),
                FilePolicy::default(),
            )
            .unwrap();
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages[0].data, protocol::encode_i32s(&[3, 0]));
        let id = files.find(peer, 3).unwrap();
        let bytes: Vec<u8> = (0..FILE_CHUNK * 5 + 3).map(|index| index as u8).collect();
        for chunk in bytes.chunks(FILE_CHUNK) {
            files
                .handle_client(
                    &mut process,
                    &mut event_loop,
                    peer,
                    data_message(Kind::WriteData, 3, chunk),
                    FilePolicy::default(),
                )
                .unwrap();
        }
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                ProtocolMessage::new(Kind::WriteClose, protocol::encode_i32s(&[3])),
                FilePolicy::default(),
            )
            .unwrap();
        assert!(files.write_left());
        assert!(receive(&mut process, peer, &fd, &mut codec).is_empty());
        while files.get(id).is_some() {
            files
                .client_ready(&mut process, &mut event_loop, id, false, false)
                .unwrap();
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages[0].data, protocol::encode_i32s(&[3, 0]));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn duplicate_stream_rejection_preserves_first_failed_record() {
        let (mut files, mut process, mut event_loop, peer, fd, mut codec) = endpoint();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::WriteOpen, 3, 7, 0, b"-"),
                FilePolicy::default(),
            )
            .unwrap();
        let first = files.find(peer, 3).unwrap();
        assert!(files.get(first).unwrap().io.is_none());
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::WriteOpen, 3, -1, 0, b"/dev/null"),
                FilePolicy::default(),
            )
            .unwrap();
        assert_eq!(files.find(peer, 3), Some(first));
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages.len(), 2);
        assert!(
            messages
                .iter()
                .all(|message| message.data == protocol::encode_i32s(&[3, libc::EBADF]))
        );
        assert!(
            files
                .handle_client(
                    &mut process,
                    &mut event_loop,
                    peer,
                    data_message(Kind::WriteData, 999, b"unknown"),
                    FilePolicy::default()
                )
                .is_err()
        );
    }

    #[test]
    fn no_ack_finishes_after_close_and_ignores_late_error() {
        let (mut server, client, peer, fd, mut codec) = server_client(false);
        let result = Rc::new(RefCell::new(None));
        let observed = result.clone();
        let id = write(
            &mut server,
            Some(client),
            b"-",
            0,
            b"binary\0".to_vec(),
            Box::new(move |_, _, _, _, error| {
                *observed.borrow_mut() = Some(error);
            }),
        )
        .unwrap();
        let stream = server.files.get(id).unwrap().stream;
        receive(&mut server.process, peer, &fd, &mut codec);
        handle_server(
            &mut server,
            client,
            status_message(Kind::WriteReady, stream, 0),
        )
        .unwrap();
        assert_eq!(server.files.get(id).unwrap().phase, FilePhase::Done);
        handle_server(
            &mut server,
            client,
            status_message(Kind::WriteDone, stream, libc::EIO),
        )
        .unwrap();
        fire_done(&mut server, id);
        assert_eq!(*result.borrow(), Some(0));
    }

    #[test]
    fn client_loss_suppresses_done_unless_read_was_cancelled() {
        for cancelled in [false, true] {
            let (mut server, client, peer, fd, mut codec) = server_client(false);
            let result = Rc::new(RefCell::new(None));
            let observed = result.clone();
            let id = read(
                &mut server,
                Some(client),
                b"-",
                Box::new(move |_, _, notice, _, error| {
                    assert_eq!(notice, FileNotice::Done);
                    *observed.borrow_mut() = Some(error);
                }),
            )
            .unwrap();
            receive(&mut server.process, peer, &fd, &mut codec);
            if cancelled {
                cancel(&mut server, id);
            }
            lost_client(&mut server, client);
            server
                .clients
                .get_mut(client)
                .unwrap()
                .flags
                .insert(ClientFlags::DEAD);
            fire_done(&mut server, id);
            assert_eq!(*result.borrow(), cancelled.then_some(libc::EINTR));
            assert!(server.files.get(id).is_none());
        }
    }

    #[test]
    fn push_and_done_timers_hold_separate_file_leases() {
        let (mut server, client, peer, fd, mut codec) = server_client(false);
        let id = write(
            &mut server,
            Some(client),
            b"-",
            0,
            b"pending".to_vec(),
            Box::new(|_, _, _, _, _| {}),
        )
        .unwrap();
        receive(&mut server.process, peer, &fd, &mut codec);
        server.files.get_mut(id).unwrap().phase = FilePhase::Writing;
        server.process.kill_peer(peer);
        push(&mut server, id);
        assert!(server.files.get(id).unwrap().push_timer.is_some());
        schedule_done(&mut server, id);
        assert!(server.files.get(id).unwrap().done_timer.is_some());
        fire_done(&mut server, id);
        assert!(server.files.get(id).is_none());
    }

    #[test]
    fn malformed_server_reply_fails_even_for_unknown_stream() {
        let (mut server, client, _, _, _) = server_client(false);
        assert!(
            handle_server(
                &mut server,
                client,
                ProtocolMessage::new(Kind::ReadDone, vec![0; 7])
            )
            .is_err()
        );
        assert!(
            handle_server(
                &mut server,
                client,
                ProtocolMessage::new(Kind::ReadData, vec![0; 3])
            )
            .is_err()
        );
        handle_server(
            &mut server,
            client,
            status_message(Kind::ReadDone, 999, libc::EIO),
        )
        .unwrap();
    }

    #[test]
    fn standard_output_opens_once_and_stays_open() {
        let (mut server, client, peer, fd, mut codec) = server_client(false);
        print(&mut server, client, b"first\0");
        print(&mut server, client, b"second\xff");
        error(&mut server, client, b"stderr");
        assert!(server.files.client_has_buffered(client));
        let messages = receive(&mut server.process, peer, &fd, &mut codec);
        assert_eq!(messages.len(), 2);
        assert!(
            messages
                .iter()
                .all(|message| message.kind == Kind::WriteOpen)
        );
        handle_server(&mut server, client, status_message(Kind::WriteReady, 1, 0)).unwrap();
        let messages = receive(&mut server.process, peer, &fd, &mut codec);
        assert_eq!(messages.len(), 1);
        assert_eq!(&messages[0].data[4..], b"first\0second\xff");
        let id = server.files.find(peer, 1).unwrap();
        assert_eq!(server.files.get(id).unwrap().phase, FilePhase::Writing);
        print(&mut server, client, b"third");
        assert_eq!(
            &receive(&mut server.process, peer, &fd, &mut codec)[0].data[4..],
            b"third"
        );
        handle_server(&mut server, client, status_message(Kind::WriteReady, 2, 0)).unwrap();
        receive(&mut server.process, peer, &fd, &mut codec);
        assert!(!server.files.client_has_buffered(client));
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .insert(ClientFlags::CONTROL);
        print(&mut server, client, b"ignored");
        assert!(receive(&mut server.process, peer, &fd, &mut codec).is_empty());
    }

    #[test]
    fn attached_control_named_file_uses_local_server_path() {
        let (mut server, client, peer, fd, mut codec) = server_client(false);
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .insert(ClientFlags::CONTROL | ClientFlags::ATTACHED);
        let path = temp_path();
        let name = path.as_os_str().as_bytes();
        let id = write(
            &mut server,
            Some(client),
            name,
            0,
            b"local\0\xff".to_vec(),
            Box::new(|_, _, _, _, error| {
                assert_eq!(error, 0);
            }),
        )
        .unwrap();
        assert!(server.files.get(id).unwrap().client.is_none());
        drive_local(&mut server, id);
        assert_eq!(std::fs::read(&path).unwrap(), b"local\0\xff");
        assert!(receive(&mut server.process, peer, &fd, &mut codec).is_empty());
        let result = Rc::new(RefCell::new(None));
        let observed = result.clone();
        assert!(
            read(
                &mut server,
                Some(client),
                b"-",
                Box::new(move |_, _, _, _, error| {
                    *observed.borrow_mut() = Some(error);
                })
            )
            .is_none()
        );
        assert_eq!(*result.borrow(), None);
        let ready = server.event_loop.poll(Some(Duration::ZERO)).unwrap();
        for ready in ready {
            if let LoopAction::FileDone(id) = ready.action {
                fire_done(&mut server, id);
            }
        }
        assert_eq!(*result.borrow(), Some(libc::EBADF));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn completion_can_start_another_file_without_borrowing_arena() {
        let mut server = Server::new();
        let next = Rc::new(RefCell::new(None));
        let observed = next.clone();
        let id = write(
            &mut server,
            None,
            b"/dev/null",
            0,
            b"one".to_vec(),
            Box::new(move |server, _, _, _, error| {
                assert_eq!(error, 0);
                *observed.borrow_mut() = write(
                    server,
                    None,
                    b"/dev/null",
                    0,
                    b"two".to_vec(),
                    Box::new(|_, _, _, _, error| {
                        assert_eq!(error, 0);
                    }),
                );
            }),
        )
        .unwrap();
        on_ready(&mut server, id, false, true);
        fire_done(&mut server, id);
        assert!(server.files.get(id).is_none());
        let next = next.borrow().unwrap();
        on_ready(&mut server, next, false, true);
        fire_done(&mut server, next);
        assert!(server.files.get(next).is_none());
    }

    #[test]
    fn fifo_late_error_is_acknowledged_only_after_write_close() {
        rmux_sys::server::ignore_sigpipe().unwrap();
        let path = temp_path();
        let name = path.as_os_str().as_bytes();
        rmux_sys::server::make_fifo(name, 0o600).unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        let (mut files, mut process, mut event_loop, peer, fd, mut codec) = endpoint();
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                open_message(Kind::WriteOpen, 3, -1, 0, name),
                FilePolicy::default(),
            )
            .unwrap();
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages[0].data, protocol::encode_i32s(&[3, 0]));
        let id = files.find(peer, 3).unwrap();
        drop(reader);
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                data_message(Kind::WriteData, 3, b"late\0\xff"),
                FilePolicy::default(),
            )
            .unwrap();
        files
            .client_ready(&mut process, &mut event_loop, id, false, true)
            .unwrap();
        assert_eq!(files.get(id).unwrap().error, libc::EPIPE);
        assert!(files.get(id).unwrap().io.is_none());
        assert!(receive(&mut process, peer, &fd, &mut codec).is_empty());
        files
            .handle_client(
                &mut process,
                &mut event_loop,
                peer,
                ProtocolMessage::new(Kind::WriteClose, protocol::encode_i32s(&[3])),
                FilePolicy::default(),
            )
            .unwrap();
        let messages = receive(&mut process, peer, &fd, &mut codec);
        assert_eq!(messages[0].kind, Kind::WriteDone);
        assert_eq!(messages[0].data, protocol::encode_i32s(&[3, libc::EPIPE]));
        assert_eq!(files.take_flush_checks(), 2);
        assert!(files.get(id).is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn attached_fallback_retains_client_and_uses_remote_stdout() {
        let (mut server, client, peer, fd, mut codec) = server_client(false);
        server
            .clients
            .get_mut(client)
            .unwrap()
            .flags
            .insert(ClientFlags::ATTACHED);
        print(&mut server, client, b"not eligible");
        assert!(receive(&mut server.process, peer, &fd, &mut codec).is_empty());
        print_unchecked(&mut server, client, b"configuration cause\n");
        let messages = receive(&mut server.process, peer, &fd, &mut codec);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, Kind::WriteOpen);
        let file = server.files.find(peer, 1).unwrap();
        assert_eq!(server.files.get(file).unwrap().client, Some(client));
        handle_server(&mut server, client, status_message(Kind::WriteReady, 1, 0)).unwrap();
        let messages = receive(&mut server.process, peer, &fd, &mut codec);
        assert_eq!(&messages[0].data[4..], b"configuration cause\n");
    }
}
