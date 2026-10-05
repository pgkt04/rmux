// Ported from tmux compat.h @ 8f25579c (evbuffer_readln EVBUFFER_EOL_LF usage)
//! `ByteBuffer`: the `struct evbuffer` replacement for buffered byte I/O.
//! tmux reads lines only with `EVBUFFER_EOL_LF` (`cmd-run-shell.c:259`,
//! `control.c:708,757`, `server-client.c:3131`), so that is the one EOL
//! style ported.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ByteBuffer {
    data: Vec<u8>,
    start: usize,
}

impl ByteBuffer {
    pub const fn new() -> ByteBuffer {
        ByteBuffer {
            data: Vec::new(),
            start: 0,
        }
    }

    /// `evbuffer_add`.
    pub fn add(&mut self, bytes: &[u8]) {
        self.compact();
        self.data.extend_from_slice(bytes);
    }

    /// `evbuffer_drain`: removes up to `n` bytes from the front.
    pub fn drain(&mut self, n: usize) {
        self.start += n.min(self.len());
        if self.start == self.data.len() {
            self.data.clear();
            self.start = 0;
        }
    }

    /// `EVBUFFER_LENGTH`.
    pub fn len(&self) -> usize {
        self.data.len() - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `EVBUFFER_DATA`: the unread bytes.
    pub fn data(&self) -> &[u8] {
        &self.data[self.start..]
    }

    /// `evbuffer_readln(buf, NULL, EVBUFFER_EOL_LF)`: the bytes before the
    /// first `\n`, which is consumed too. `None` when there is no newline.
    pub fn readln_eol(&mut self) -> Option<Vec<u8>> {
        let nl = self.data().iter().position(|&c| c == b'\n')?;
        let line = self.data()[..nl].to_vec();
        self.drain(nl + 1);
        Some(line)
    }

    /// Removes and returns everything.
    pub fn take(&mut self) -> Vec<u8> {
        let out = self.data().to_vec();
        self.data.clear();
        self.start = 0;
        out
    }

    fn compact(&mut self) {
        if self.start > 0 && self.start >= self.data.len() / 2 {
            self.data.drain(..self.start);
            self.start = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_len_data_drain() {
        let mut b = ByteBuffer::new();
        assert!(b.is_empty());
        b.add(b"hello");
        b.add(b" world");
        assert_eq!(b.len(), 11);
        assert_eq!(b.data(), b"hello world");
        b.drain(6);
        assert_eq!(b.data(), b"world");
        b.drain(100);
        assert!(b.is_empty());
        assert_eq!(b.data(), b"");
    }

    #[test]
    fn readln_lf() {
        let mut b = ByteBuffer::new();
        b.add(b"one\ntwo\r\n\nthree");
        assert_eq!(b.readln_eol().unwrap(), b"one");
        assert_eq!(b.readln_eol().unwrap(), b"two\r");
        assert_eq!(b.readln_eol().unwrap(), b"");
        assert_eq!(b.readln_eol(), None);
        assert_eq!(b.data(), b"three");
        b.add(b"\n");
        assert_eq!(b.readln_eol().unwrap(), b"three");
        assert!(b.is_empty());
        assert_eq!(b.readln_eol(), None);
    }

    #[test]
    fn take_and_compaction() {
        let mut b = ByteBuffer::new();
        for _ in 0..100 {
            b.add(b"abcd");
            b.drain(3);
        }
        // Each round keeps one more byte of the previous tail: the remaining
        // bytes are the last 100 bytes of "abcd" repeated.
        assert_eq!(b.len(), 100);
        let expected: Vec<u8> = b"abcd"
            .iter()
            .copied()
            .cycle()
            .skip(300)
            .take(100)
            .collect();
        assert_eq!(b.take(), expected);
        assert!(b.is_empty());
        b.add(b"x");
        assert_eq!(b.data(), b"x");
        b.add(b"yz");
        b.drain(2);
        b.add(b"w");
        assert_eq!(b.data(), b"zw");
    }
}
