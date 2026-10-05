// Ported from tmux proc.c, tmux-protocol.h, cmd.c, client.c @ 8f25579c
// Copyright (c) 2021 Nicholas Marriott <nicholas.marriott@gmail.com>
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
use std::{
    collections::VecDeque,
    io,
    os::fd::{AsFd, OwnedFd},
};
pub const VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 16;
pub const MAX_FRAME: usize = 32768;
pub const LEGACY_PAYLOAD: usize = 16368;
#[derive(Debug)]
pub enum ProtocolError {
    Transport(io::Error),
    Shape(&'static str),
    Version(u16),
    Closed,
}
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "{e}"),
            Self::Shape(s) => write!(f, "{s}"),
            Self::Version(v) => write!(f, "protocol version mismatch: {v}"),
            Self::Closed => write!(f, "peer is closed"),
        }
    }
}
impl std::error::Error for ProtocolError {}
impl From<io::Error> for ProtocolError {
    fn from(e: io::Error) -> Self {
        Self::Transport(e)
    }
}
impl From<ProtocolError> for io::Error {
    fn from(e: ProtocolError) -> Self {
        io::Error::other(e)
    }
}
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolMessageKind {
    Version = 12,
    IdentifyFlags = 100,
    IdentifyTerm = 101,
    IdentifyTtyname = 102,
    IdentifyStdin = 104,
    IdentifyEnviron = 105,
    IdentifyDone = 106,
    IdentifyClientpid = 107,
    IdentifyCwd = 108,
    IdentifyFeatures = 109,
    IdentifyStdout = 110,
    IdentifyLongflags = 111,
    IdentifyTerminfo = 112,
    Command = 200,
    Detach = 201,
    DetachKill = 202,
    Exit = 203,
    Exited = 204,
    Exiting = 205,
    Lock = 206,
    Ready = 207,
    Resize = 208,
    Shell = 209,
    Shutdown = 210,
    Suspend = 214,
    Unlock = 215,
    Wakeup = 216,
    Exec = 217,
    Flags = 218,
    ReadOpen = 300,
    ReadData = 301,
    ReadDone = 302,
    WriteOpen = 303,
    WriteData = 304,
    WriteReady = 305,
    WriteClose = 306,
    ReadCancel = 307,
    WriteDone = 308,
}
impl TryFrom<u16> for ProtocolMessageKind {
    type Error = ProtocolError;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        Ok(match v {
            12 => Self::Version,
            100 => Self::IdentifyFlags,
            101 => Self::IdentifyTerm,
            102 => Self::IdentifyTtyname,
            104 => Self::IdentifyStdin,
            105 => Self::IdentifyEnviron,
            106 => Self::IdentifyDone,
            107 => Self::IdentifyClientpid,
            108 => Self::IdentifyCwd,
            109 => Self::IdentifyFeatures,
            110 => Self::IdentifyStdout,
            111 => Self::IdentifyLongflags,
            112 => Self::IdentifyTerminfo,
            200 => Self::Command,
            201 => Self::Detach,
            202 => Self::DetachKill,
            203 => Self::Exit,
            204 => Self::Exited,
            205 => Self::Exiting,
            206 => Self::Lock,
            207 => Self::Ready,
            208 => Self::Resize,
            209 => Self::Shell,
            210 => Self::Shutdown,
            214 => Self::Suspend,
            215 => Self::Unlock,
            216 => Self::Wakeup,
            217 => Self::Exec,
            218 => Self::Flags,
            300 => Self::ReadOpen,
            301 => Self::ReadData,
            302 => Self::ReadDone,
            303 => Self::WriteOpen,
            304 => Self::WriteData,
            305 => Self::WriteReady,
            306 => Self::WriteClose,
            307 => Self::ReadCancel,
            308 => Self::WriteDone,
            _ => return Err(ProtocolError::Shape("unknown message kind")),
        })
    }
}
#[derive(Debug)]
pub struct ProtocolMessage {
    pub kind: ProtocolMessageKind,
    pub data: Vec<u8>,
    pub fd: Option<OwnedFd>,
}
impl ProtocolMessage {
    pub fn new(kind: ProtocolMessageKind, data: impl Into<Vec<u8>>) -> Self {
        Self {
            kind,
            data: data.into(),
            fd: None,
        }
    }
    pub fn with_fd(kind: ProtocolMessageKind, data: impl Into<Vec<u8>>, fd: OwnedFd) -> Self {
        Self {
            kind,
            data: data.into(),
            fd: Some(fd),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerState {
    Active,
    BadDraining,
    Closed,
}
pub fn encode_string(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}
pub fn encode_i32s(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
pub fn decode_i32(bytes: &[u8]) -> Result<i32, ProtocolError> {
    Ok(i32::from_le_bytes(
        bytes
            .try_into()
            .map_err(|_| ProtocolError::Shape("bad integer"))?,
    ))
}
pub fn take_string<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8], ProtocolError> {
    if bytes.len() < 4 {
        return Err(ProtocolError::Shape("short string"));
    }
    let len = u32::from_le_bytes(bytes[..4].try_into().expect("length")) as usize;
    if len > bytes.len() - 4 {
        return Err(ProtocolError::Shape("short string"));
    }
    let value = &bytes[4..4 + len];
    *bytes = &bytes[4 + len..];
    Ok(value)
}
pub fn decode_string(bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let mut rest = bytes;
    let out = take_string(&mut rest)?.to_vec();
    if !rest.is_empty() {
        return Err(ProtocolError::Shape("trailing string bytes"));
    }
    Ok(out)
}
pub fn pack_argv(argv: &[Vec<u8>]) -> Result<Vec<u8>, ProtocolError> {
    if argv.len() > 1000 {
        return Err(ProtocolError::Shape("too many arguments"));
    }
    let packed = argv
        .iter()
        .try_fold(0usize, |n, arg| n.checked_add(arg.len() + 1))
        .ok_or(ProtocolError::Shape("command too long"))?;
    if packed > 16380 {
        return Err(ProtocolError::Shape("command too long"));
    }
    if packed > 16364 {
        return Err(ProtocolError::Shape("failed to send command"));
    }
    let mut out = (argv.len() as u32).to_le_bytes().to_vec();
    for arg in argv {
        if arg.contains(&0) {
            return Err(ProtocolError::Shape("NUL in command argument"));
        }
        out.extend_from_slice(&encode_string(arg));
    }
    Ok(out)
}
pub fn unpack_argv(bytes: &[u8]) -> Result<Vec<Vec<u8>>, ProtocolError> {
    if bytes.len() < 4 {
        return Err(ProtocolError::Shape("short command"));
    }
    let count = u32::from_le_bytes(bytes[..4].try_into().expect("count")) as usize;
    if count > 1000 {
        return Err(ProtocolError::Shape("too many arguments"));
    }
    let mut rest = &bytes[4..];
    let mut out = Vec::with_capacity(count);
    let mut packed = 0usize;
    for _ in 0..count {
        let arg = take_string(&mut rest)?;
        if arg.contains(&0) {
            return Err(ProtocolError::Shape("NUL in command argument"));
        }
        packed += arg.len() + 1;
        out.push(arg.to_vec());
    }
    if !rest.is_empty() || packed > 16364 {
        return Err(ProtocolError::Shape("bad command size"));
    }
    Ok(out)
}
fn one_string(data: &[u8]) -> Result<usize, ProtocolError> {
    let mut rest = data;
    let value = take_string(&mut rest)?;
    if !rest.is_empty() || value.contains(&0) {
        return Err(ProtocolError::Shape("bad byte string"));
    }
    Ok(value.len())
}
pub fn validate(message: &ProtocolMessage) -> Result<(), ProtocolError> {
    use ProtocolMessageKind::*;
    let data = &message.data;
    let fd = message.fd.is_some();
    if matches!(message.kind, IdentifyStdin | IdentifyStdout) {
        if !fd || !data.is_empty() {
            return Err(ProtocolError::Shape("identify fd shape"));
        }
        return Ok(());
    }
    if fd {
        return Err(ProtocolError::Shape("unexpected descriptor"));
    }
    match message.kind {
        IdentifyFlags | IdentifyLongflags | Flags => {
            if data.len() != 8 {
                return Err(ProtocolError::Shape("flags shape"));
            }
        }
        IdentifyFeatures | IdentifyClientpid => {
            if data.len() != 4 {
                return Err(ProtocolError::Shape("integer shape"));
            }
        }
        Version => {
            if data.len() != 2 {
                return Err(ProtocolError::Shape("version shape"));
            }
        }
        IdentifyTerm | IdentifyTtyname | IdentifyCwd | IdentifyTerminfo | IdentifyEnviron
        | Detach | DetachKill | Lock => {
            let len = one_string(data)?;
            if len + 1 > LEGACY_PAYLOAD {
                return Err(ProtocolError::Shape("string too long"));
            }
        }
        Command => {
            unpack_argv(data)?;
        }
        Exec => {
            let mut rest = data.as_slice();
            take_string(&mut rest)?;
            take_string(&mut rest)?;
            if !rest.is_empty() {
                return Err(ProtocolError::Shape("exec shape"));
            }
        }
        Exit => {
            if !data.is_empty() && data.len() < 4 {
                return Err(ProtocolError::Shape("exit shape"));
            }
            if data.len() > 4 {
                one_string(&data[4..])?;
            }
        }
        Shell => {
            if !data.is_empty() {
                one_string(data)?;
            }
        }
        ReadOpen | WriteOpen => {
            let offset = if message.kind == ReadOpen { 8 } else { 12 };
            if data.len() < offset {
                return Err(ProtocolError::Shape("open shape"));
            }
            let len = one_string(&data[offset..])?;
            if offset + len + 1 > LEGACY_PAYLOAD {
                return Err(ProtocolError::Shape("path too long"));
            }
        }
        ReadData | WriteData => {
            if data.len() < 4 || data.len() > LEGACY_PAYLOAD {
                return Err(ProtocolError::Shape("file data shape"));
            }
        }
        ReadDone | WriteReady | WriteDone => {
            if data.len() != 8 {
                return Err(ProtocolError::Shape("file reply shape"));
            }
        }
        WriteClose | ReadCancel => {
            if data.len() != 4 {
                return Err(ProtocolError::Shape("file close shape"));
            }
        }
        _ => {
            if !data.is_empty() {
                return Err(ProtocolError::Shape("expected empty message"));
            }
        }
    }
    Ok(())
}
struct Frame {
    bytes: Vec<u8>,
    offset: usize,
    fd: Option<OwnedFd>,
}
pub trait ProtocolBackend {
    fn enqueue(&mut self, message: ProtocolMessage) -> Result<(), ProtocolError>;
    fn receive(&mut self, fd: &OwnedFd) -> Result<Vec<ProtocolMessage>, ProtocolError>;
    fn flush(&mut self, fd: &OwnedFd) -> Result<(), ProtocolError>;
    fn queued_output(&self) -> bool;
    fn close(&mut self);
}
#[derive(Default)]
pub struct RmuxBackend {
    input: Vec<u8>,
    expected: usize,
    input_fd: Option<OwnedFd>,
    output: VecDeque<Frame>,
}
impl RmuxBackend {
    fn frame(message: ProtocolMessage) -> Result<Frame, ProtocolError> {
        validate(&message)?;
        if message.data.len() + HEADER_SIZE > MAX_FRAME {
            return Err(ProtocolError::Shape("frame too large"));
        }
        let mut bytes = Vec::with_capacity(HEADER_SIZE + message.data.len());
        bytes.extend_from_slice(b"RMUX");
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&(message.kind as u16).to_le_bytes());
        bytes.extend_from_slice(&(message.data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&u16::from(message.fd.is_some()).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&message.data);
        Ok(Frame {
            bytes,
            offset: 0,
            fd: message.fd,
        })
    }
    fn header(&mut self) -> Result<(), ProtocolError> {
        if &self.input[..4] != b"RMUX" {
            return Err(ProtocolError::Shape("bad frame magic"));
        }
        let version = u16::from_le_bytes(self.input[4..6].try_into().expect("version"));
        let kind = u16::from_le_bytes(self.input[6..8].try_into().expect("kind"));
        if version != VERSION && kind != ProtocolMessageKind::Version as u16 {
            return Err(ProtocolError::Version(version));
        }
        let len = u32::from_le_bytes(self.input[8..12].try_into().expect("length")) as usize;
        let count = u16::from_le_bytes(self.input[12..14].try_into().expect("fds"));
        let reserved = u16::from_le_bytes(self.input[14..16].try_into().expect("reserved"));
        if len > MAX_FRAME - HEADER_SIZE
            || count > 1
            || reserved != 0
            || usize::from(count) != usize::from(self.input_fd.is_some())
        {
            return Err(ProtocolError::Shape("bad frame header"));
        }
        self.expected = HEADER_SIZE + len;
        Ok(())
    }
}
impl ProtocolBackend for RmuxBackend {
    fn enqueue(&mut self, message: ProtocolMessage) -> Result<(), ProtocolError> {
        self.output.push_back(Self::frame(message)?);
        Ok(())
    }
    fn receive(&mut self, fd: &OwnedFd) -> Result<Vec<ProtocolMessage>, ProtocolError> {
        let mut messages = Vec::new();
        let mut scratch = [0u8; 8192];
        loop {
            let target = if self.input.len() < HEADER_SIZE {
                HEADER_SIZE
            } else {
                self.expected
            };
            let need = target - self.input.len();
            if need == 0 {
                let kind = ProtocolMessageKind::try_from(u16::from_le_bytes(
                    self.input[6..8].try_into().expect("kind"),
                ))?;
                let data = self.input.split_off(HEADER_SIZE);
                self.input.clear();
                self.expected = 0;
                let message = ProtocolMessage {
                    kind,
                    data,
                    fd: self.input_fd.take(),
                };
                validate(&message)?;
                messages.push(message);
                return Ok(messages);
            }
            let mut rights = Vec::new();
            let limit = need.min(scratch.len());
            match rmux_sys::fd::recv_fds(fd.as_fd(), &mut scratch[..limit], &mut rights) {
                Ok(0) => {
                    if messages.is_empty() {
                        return Err(ProtocolError::Closed);
                    }
                    return Ok(messages);
                }
                Ok(n) => {
                    if !rights.is_empty() {
                        if !self.input.is_empty() || self.input_fd.is_some() || rights.len() != 1 {
                            return Err(ProtocolError::Shape("misplaced descriptors"));
                        }
                        self.input_fd = rights.pop();
                    }
                    self.input.extend_from_slice(&scratch[..n]);
                    if self.input.len() == HEADER_SIZE {
                        self.header()?;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(messages),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
    fn flush(&mut self, fd: &OwnedFd) -> Result<(), ProtocolError> {
        while let Some(frame) = self.output.front_mut() {
            let rights = frame.fd.as_ref().map(|f| [f.as_fd()]);
            match rmux_sys::fd::send_fds(
                fd.as_fd(),
                &frame.bytes[frame.offset..],
                rights.as_ref().map_or(&[], |fds| fds.as_slice()),
            ) {
                Ok(0) => return Err(ProtocolError::Closed),
                Ok(n) => {
                    frame.fd.take();
                    frame.offset += n;
                    if frame.offset == frame.bytes.len() {
                        self.output.pop_front();
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    fn queued_output(&self) -> bool {
        !self.output.is_empty()
    }
    fn close(&mut self) {
        self.input.clear();
        self.input_fd.take();
        self.output.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_argv_boundaries_and_cap() {
        assert!(pack_argv(&[vec![b'x'; 16363]]).is_ok());
        assert_eq!(
            pack_argv(&[vec![b'x'; 16364]]).unwrap_err().to_string(),
            "failed to send command"
        );
        assert_eq!(
            pack_argv(&[vec![b'x'; 16380]]).unwrap_err().to_string(),
            "command too long"
        );
        assert!(pack_argv(&vec![vec![]; 1001]).is_err());
        let argv = vec![b"new".to_vec(), b"a b".to_vec()];
        assert_eq!(unpack_argv(&pack_argv(&argv).unwrap()).unwrap(), argv);
    }
    #[test]
    fn inactive_slots_and_malformed_shapes_rejected() {
        for kind in [103, 211, 212, 213, 999] {
            assert!(ProtocolMessageKind::try_from(kind).is_err());
        }
        assert!(
            validate(&ProtocolMessage::new(
                ProtocolMessageKind::ReadData,
                vec![0; 3]
            ))
            .is_err()
        );
        assert!(decode_string(&[255, 255, 255, 255]).is_err());
    }
    #[test]
    fn wire_roundtrip_preserves_fd_and_frame_boundaries() {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        b.set_nonblocking(true).unwrap();
        let a: OwnedFd = a.into();
        let b: OwnedFd = b.into();
        let mut tx = RmuxBackend::default();
        let mut rx = RmuxBackend::default();
        let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        tx.enqueue(ProtocolMessage::with_fd(
            ProtocolMessageKind::IdentifyStdin,
            vec![],
            fd,
        ))
        .unwrap();
        tx.enqueue(ProtocolMessage::new(
            ProtocolMessageKind::IdentifyDone,
            vec![],
        ))
        .unwrap();
        tx.flush(&a).unwrap();
        let mut got = rx.receive(&b).unwrap();
        got.extend(rx.receive(&b).unwrap());
        assert_eq!(got.len(), 2);
        assert!(got[0].fd.is_some());
        assert!(got[1].fd.is_none());
        assert_eq!(got[1].kind, ProtocolMessageKind::IdentifyDone);
    }
    #[test]
    fn oversized_header_fails_before_payload_allocation() {
        let mut rx = RmuxBackend {
            input: b"RMUX".to_vec(),
            ..Default::default()
        };
        rx.input.extend_from_slice(&VERSION.to_le_bytes());
        rx.input
            .extend_from_slice(&(ProtocolMessageKind::Ready as u16).to_le_bytes());
        rx.input.extend_from_slice(&u32::MAX.to_le_bytes());
        rx.input.extend_from_slice(&[0; 4]);
        assert!(rx.header().is_err());
        assert_eq!(rx.input.len(), 16);
    }
}
