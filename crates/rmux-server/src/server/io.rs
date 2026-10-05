// Ported from tmux job.c, file.c, control.c (bufferevent usage) @ 8f25579c
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

use std::collections::VecDeque;
use std::io::{self, IoSlice};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

pub const IO_BUDGET: usize = 64 * 1024;
const READ_CHUNK: usize = 8192;
const WRITE_VECTORS: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IoProgress {
    pub bytes: usize,
    pub eof: bool,
    pub blocked: bool,
    pub drained: bool,
}

struct OutputSegment {
    bytes: Vec<u8>,
    start: usize,
}

pub struct BufferedIo {
    fd: Option<OwnedFd>,
    input: Vec<u8>,
    input_start: usize,
    output: VecDeque<OutputSegment>,
    output_start: usize,
    output_len: usize,
    read_enabled: bool,
    write_enabled: bool,
    read_closed: bool,
    write_closed: bool,
    shutdown_pending: bool,
    pty: bool,
    null: bool,
    read_low: usize,
    read_high: Option<usize>,
    write_low: usize,
}

impl BufferedIo {
    pub fn new(fd: OwnedFd) -> Self {
        rmux_sys::fd::set_blocking(fd.as_fd(), false);
        Self {
            fd: Some(fd),
            input: Vec::new(),
            input_start: 0,
            output: VecDeque::new(),
            output_start: 0,
            output_len: 0,
            read_enabled: true,
            write_enabled: true,
            read_closed: false,
            write_closed: false,
            shutdown_pending: false,
            pty: false,
            null: false,
            read_low: 0,
            read_high: None,
            write_low: 0,
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd
            .as_ref()
            .expect("buffered I/O descriptor transferred")
            .as_fd()
    }

    pub fn take_fd(&mut self) -> Option<OwnedFd> {
        self.read_enabled = false;
        self.write_enabled = false;
        self.fd.take()
    }

    pub fn input(&self) -> &[u8] {
        &self.input[self.input_start..]
    }
    pub fn input_len(&self) -> usize {
        self.input.len() - self.input_start
    }
    pub fn consume_input(&mut self, count: usize) {
        self.input_start += count.min(self.input_len());
        if self.input_start == self.input.len() {
            self.input.clear();
            self.input_start = 0;
        }
    }
    pub fn take_input(&mut self) -> Vec<u8> {
        let start = std::mem::take(&mut self.input_start);
        let mut bytes = std::mem::take(&mut self.input);
        if start != 0 {
            bytes.drain(..start);
        }
        bytes
    }
    pub fn restore_input(&mut self, bytes: Vec<u8>) {
        assert!(self.input_len() == 0, "restoring over unread input");
        self.input_start = 0;
        self.input = bytes;
    }
    pub fn queue(&mut self, bytes: Vec<u8>) {
        self.queue_from(bytes, 0);
    }
    pub fn queue_from(&mut self, bytes: Vec<u8>, start: usize) {
        assert!(
            !self.shutdown_pending && !self.write_closed,
            "write after half-close"
        );
        assert!(start <= bytes.len());
        if start != bytes.len() {
            self.output_len += bytes.len() - start;
            self.output.push_back(OutputSegment { bytes, start });
        }
    }
    pub fn output_len(&self) -> usize {
        self.output_len
    }
    pub fn enable_read(&mut self, enabled: bool) {
        self.read_enabled = enabled;
    }
    pub fn enable_write(&mut self, enabled: bool) {
        self.write_enabled = enabled;
    }
    pub fn set_pty(&mut self, pty: bool) {
        self.pty = pty;
    }
    pub fn set_null(&mut self, null: bool) {
        self.null = null;
    }
    pub fn read_closed(&self) -> bool {
        self.read_closed
    }
    pub fn write_closed(&self) -> bool {
        self.write_closed
    }
    pub fn set_watermarks(&mut self, read_low: usize, read_high: Option<usize>, write_low: usize) {
        assert!(read_high.is_none_or(|high| high >= read_low));
        self.read_low = read_low;
        self.read_high = read_high;
        self.write_low = write_low;
    }
    pub fn read_callback_ready(&self) -> bool {
        self.input_len() >= self.read_low || self.read_closed
    }
    pub fn write_callback_ready(&self) -> bool {
        self.output_len <= self.write_low
    }
    pub fn interests(&self) -> (bool, bool) {
        (
            self.fd.is_some()
                && self.read_enabled
                && !self.read_closed
                && self.read_high.is_none_or(|high| self.input_len() < high),
            self.fd.is_some() && self.write_enabled && !self.write_closed && self.output_len != 0,
        )
    }
    /// One bufferevent read: data reaches its callback before a later EOF callback.
    pub fn read_once(&mut self) -> io::Result<IoProgress> {
        self.read_with_limit(true)
    }
    pub fn read_ready(&mut self) -> io::Result<IoProgress> {
        self.read_with_limit(false)
    }
    fn read_with_limit(&mut self, once: bool) -> io::Result<IoProgress> {
        let mut progress = IoProgress::default();
        if !self.interests().0 {
            progress.eof = self.read_closed;
            return Ok(progress);
        }
        if self.null {
            self.read_closed = true;
            progress.eof = true;
            return Ok(progress);
        }
        let mut buffer = [0u8; READ_CHUNK];
        while progress.bytes < IO_BUDGET {
            let room = self
                .read_high
                .map_or(IO_BUDGET, |high| high.saturating_sub(self.input_len()));
            let count = buffer.len().min(IO_BUDGET - progress.bytes).min(room);
            if count == 0 {
                break;
            }
            match rmux_sys::fd::read(self.fd(), &mut buffer[..count]) {
                Ok(0) => {
                    self.read_closed = true;
                    progress.eof = true;
                    break;
                }
                Ok(count) => {
                    if self.input_start != 0 && self.input_start >= self.input.len() / 2 {
                        self.input.drain(..self.input_start);
                        self.input_start = 0;
                    }
                    self.input.extend_from_slice(&buffer[..count]);
                    progress.bytes += count;
                    if once {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    progress.blocked = true;
                    break;
                }
                Err(error) if self.pty && error.raw_os_error() == Some(libc::EIO) => {
                    self.read_closed = true;
                    progress.eof = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(progress)
    }
    pub fn write_ready(&mut self) -> io::Result<IoProgress> {
        let mut progress = IoProgress::default();
        if self.null && self.write_enabled && !self.write_closed {
            progress.bytes = self.output_len;
            self.output.clear();
            self.output_start = 0;
            self.output_len = 0;
        } else if self.interests().1 {
            while self.output_len != 0 && progress.bytes < IO_BUDGET {
                let mut slices = [IoSlice::new(&[]); WRITE_VECTORS];
                let mut remaining = IO_BUDGET - progress.bytes;
                let mut count = 0;
                for (index, segment) in self.output.iter().take(WRITE_VECTORS).enumerate() {
                    let start = segment.start + if index == 0 { self.output_start } else { 0 };
                    let length = (segment.bytes.len() - start).min(remaining);
                    slices[index] = IoSlice::new(&segment.bytes[start..start + length]);
                    count += 1;
                    remaining -= length;
                    if remaining == 0 {
                        break;
                    }
                }
                let result = rmux_sys::server::write_vectored(self.fd(), &slices[..count]);
                match result {
                    Ok(0) => return Err(io::Error::from_raw_os_error(libc::EIO)),
                    Ok(count) => {
                        self.drain_output(count);
                        progress.bytes += count;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        progress.blocked = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        progress.drained = self.output_len == 0;
        if progress.drained && self.shutdown_pending {
            self.finish_shutdown()?;
        }
        Ok(progress)
    }
    fn drain_output(&mut self, mut count: usize) {
        self.output_len -= count;
        while count != 0 {
            let segment = self.output.front().expect("queued write segment");
            let available = segment.bytes.len() - segment.start - self.output_start;
            if count < available {
                self.output_start += count;
                break;
            }
            count -= available;
            self.output.pop_front();
            self.output_start = 0;
        }
    }
    pub fn shutdown_when_drained(&mut self) -> io::Result<()> {
        self.shutdown_pending = true;
        if self.output_len == 0 {
            self.finish_shutdown()?;
        }
        Ok(())
    }
    fn finish_shutdown(&mut self) -> io::Result<()> {
        if self.write_closed {
            return Ok(());
        }
        if !self.null {
            rmux_sys::server::shutdown_write(self.fd())?;
        }
        self.write_closed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    #[test]
    fn bounded_reads_and_watermark_resume() {
        let (read, mut write) = UnixStream::pair().unwrap();
        write.write_all(b"abcdef").unwrap();
        let mut buffered = BufferedIo::new(read.into());
        buffered.set_watermarks(2, Some(3), 0);
        assert_eq!(buffered.read_ready().unwrap().bytes, 3);
        assert_eq!(buffered.input(), b"abc");
        assert!(!buffered.interests().0);
        buffered.consume_input(2);
        assert!(buffered.interests().0);
        assert_eq!(buffered.read_ready().unwrap().bytes, 2);
        assert_eq!(buffered.take_input(), b"cde");
    }

    #[test]
    fn vectored_segments_and_half_close_preserve_bytes() {
        let (write, mut read) = UnixStream::pair().unwrap();
        let mut buffered = BufferedIo::new(write.into());
        buffered.queue(b"one".to_vec());
        buffered.queue(b"two".to_vec());
        buffered.shutdown_when_drained().unwrap();
        assert!(!buffered.write_closed());
        assert_eq!(buffered.write_ready().unwrap().bytes, 6);
        assert!(buffered.write_closed());
        let mut bytes = Vec::new();
        read.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"onetwo");
    }

    #[test]
    fn null_endpoint_reads_eof_and_drains_all_output() {
        let fd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut buffered = BufferedIo::new(fd.into());
        buffered.set_null(true);
        buffered.queue(vec![0; IO_BUDGET + 1]);
        assert!(buffered.read_ready().unwrap().eof);
        assert_eq!(buffered.write_ready().unwrap().bytes, IO_BUDGET + 1);
        assert_eq!(buffered.output_len(), 0);
    }

    #[test]
    fn pipes_preserve_binary_and_eof() {
        let (read, write) = rmux_sys::fd::pipe().unwrap();
        rmux_sys::fd::write(write.as_fd(), b"\0\xff\n").unwrap();
        drop(write);
        let mut buffered = BufferedIo::new(read);
        assert!(buffered.read_ready().unwrap().eof);
        assert_eq!(buffered.input(), b"\0\xff\n");
    }

    #[test]
    fn single_read_delivers_control_commands_before_eof() {
        let (read, write) = rmux_sys::fd::pipe().unwrap();
        rmux_sys::fd::write(write.as_fd(), b"refresh -C 100,50\n").unwrap();
        drop(write);
        let mut buffered = BufferedIo::new(read);
        let progress = buffered.read_once().unwrap();
        assert_eq!(buffered.take_input(), b"refresh -C 100,50\n");
        assert!(!progress.eof);
        assert!(buffered.interests().0);
        assert!(buffered.read_once().unwrap().eof);
    }
}

#[cfg(test)]
mod drain_tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    #[test]
    fn queued_large_segments_survive_short_writes() {
        let (write, mut read) = UnixStream::pair().unwrap();
        read.set_nonblocking(true).unwrap();
        let mut buffered = BufferedIo::new(write.into());
        let bytes: Vec<u8> = (0..IO_BUDGET * 16).map(|index| index as u8).collect();
        for segment in bytes.chunks(IO_BUDGET / 3) {
            buffered.queue(segment.to_vec());
        }
        let mut received = Vec::new();
        let mut scratch = [0u8; 8192];
        while buffered.output_len() != 0 {
            let progress = buffered.write_ready().unwrap();
            assert!(progress.bytes <= IO_BUDGET);
            loop {
                match read.read(&mut scratch) {
                    Ok(0) => panic!("unexpected EOF"),
                    Ok(count) => received.extend_from_slice(&scratch[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("{error}"),
                }
            }
        }
        assert_eq!(received, bytes);
    }

    #[test]
    fn pty_hangup_is_eof_not_eio() {
        let (master, slave, _) = rmux_sys::pty::openpty().unwrap();
        drop(slave);
        let mut buffered = BufferedIo::new(master);
        buffered.set_pty(true);
        assert!(buffered.read_ready().unwrap().eof);
        assert!(buffered.read_closed());
    }

    #[test]
    fn moved_payload_skips_header_and_input_restores_without_copy() {
        let (write, mut read) = UnixStream::pair().unwrap();
        let mut buffered = BufferedIo::new(write.into());
        buffered.queue_from(b"HEAD\0\xffbody".to_vec(), 4);
        assert_eq!(buffered.output_len(), 6);
        buffered.write_ready().unwrap();
        let mut data = [0u8; 6];
        read.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"\0\xffbody");
        let input = b"remaining".to_vec();
        let address = input.as_ptr();
        buffered.restore_input(input);
        assert_eq!(buffered.input().as_ptr(), address);
        let returned = buffered.take_input();
        assert_eq!(returned.as_ptr(), address);
        assert_eq!(returned, b"remaining");
    }
}
