// Ported from tmux control.c @ 8f25579c
use crate::ids::EventToken;
use crate::server::io::BufferedIo;
pub enum ControlIo {
    Shared {
        io: BufferedIo,
        token: Option<EventToken>,
    },
    Separate {
        read: BufferedIo,
        write: BufferedIo,
        read_token: Option<EventToken>,
        write_token: Option<EventToken>,
    },
}
impl ControlIo {
    pub fn reader(&mut self) -> &mut BufferedIo {
        match self {
            Self::Shared { io, .. } => io,
            Self::Separate { read, .. } => read,
        }
    }
    pub fn writer(&mut self) -> &mut BufferedIo {
        match self {
            Self::Shared { io, .. } => io,
            Self::Separate { write, .. } => write,
        }
    }
    pub fn output_len(&self) -> usize {
        match self {
            Self::Shared { io, .. } => io.output_len(),
            Self::Separate { write, .. } => write.output_len(),
        }
    }
}
