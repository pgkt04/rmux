// Ported from tmux tty-term.c @ 8f25579c; ncurses 6.6 read_entry.c, db_iterator.c, lib_setup.c, include/Caps
/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER IN AN ACTION
 * OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
 * CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 *
 * Copyright 2018-2024,2025 Thomas E. Dickey
 * Copyright 1998-2016,2017 Free Software Foundation, Inc.
 * Permission is hereby granted, free of charge, to any person obtaining a
 * copy of this software and associated documentation files (the "Software"),
 * to deal in the Software without restriction, including without limitation
 * the rights to use, copy, modify, merge, publish, distribute, distribute
 * with modifications, sublicense, and/or sell copies of the Software, and to
 * permit persons to whom the Software is furnished to do so, subject to the
 * following conditions: The above copyright notice and this permission notice
 * shall be included in all copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
 * THE ABOVE COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
 * IN THE SOFTWARE. Except as contained in this notice, the names of the above
 * copyright holders shall not be used in advertising or otherwise to promote
 * the sale, use or other dealings in this Software without prior written
 * authorization.
 */

use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use rmux_util::bytes::ByteString;

use super::{CODES, CodeKind};

pub type CapList = Vec<ByteString>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminfoError {
    Hardcopy,
    Missing,
    NoDatabase,
    Unknown,
    Malformed(&'static str),
}

impl TerminfoError {
    pub fn cause(&self, name: &[u8]) -> ByteString {
        let prefix: &[u8] = match self {
            Self::Hardcopy => b"can't use hardcopy terminal: ",
            Self::Missing | Self::Malformed(_) => b"missing or unsuitable terminal: ",
            Self::NoDatabase => return ByteString::from("can't find terminfo database"),
            Self::Unknown => return ByteString::from("unknown error"),
        };
        let mut cause = Vec::with_capacity(prefix.len() + name.len());
        cause.extend_from_slice(prefix);
        cause.extend_from_slice(rmux_util::bytes::cstr(name));
        cause.into()
    }
}

impl fmt::Display for TerminfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "malformed terminfo: {reason}"),
            _ => fmt::Display::fmt(&self.cause(b""), f),
        }
    }
}

impl std::error::Error for TerminfoError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Capability {
    Flag(bool),
    Number(i32),
    String(Vec<u8>),
    Missing,
}

#[derive(Clone, Debug)]
pub struct TerminfoEntry {
    names: Vec<u8>,
    booleans: Vec<bool>,
    numbers: Vec<Option<i32>>,
    strings: Vec<Option<Vec<u8>>>,
    extended: Vec<(Vec<u8>, Capability)>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], TerminfoError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(TerminfoError::Malformed("section size overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(TerminfoError::Malformed("truncated section"))?;
        self.offset = end;
        Ok(bytes)
    }

    fn word(&mut self) -> Result<u16, TerminfoError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn count(&mut self) -> Result<usize, TerminfoError> {
        let value = self.word()?;
        if value > i16::MAX as u16 {
            return Err(TerminfoError::Malformed("negative section count"));
        }
        Ok(usize::from(value))
    }

    fn align(&mut self) -> Result<(), TerminfoError> {
        if self.offset % 2 != 0 {
            self.take(1)?;
        }
        Ok(())
    }

    fn numbers(&mut self, count: usize, wide: bool) -> Result<Vec<Option<i32>>, TerminfoError> {
        (0..count)
            .map(|_| {
                let n = if wide {
                    let bytes = self.take(4)?;
                    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                } else {
                    i32::from(self.word()? as i16)
                };
                Ok(if n < 0 { None } else { Some(n) })
            })
            .collect()
    }

    fn offsets(&mut self, count: usize) -> Result<Vec<i16>, TerminfoError> {
        (0..count).map(|_| self.word().map(|n| n as i16)).collect()
    }
}

// ncurses convert_strings: absent/cancelled markers, offsets outside the
// table and unterminated strings all read as an absent capability.
fn table_string(table: &[u8], offset: i16) -> Option<&[u8]> {
    let value = table.get(usize::try_from(offset).ok()?..)?;
    let end = value.iter().position(|&ch| ch == 0)?;
    Some(&value[..end])
}

impl TerminfoEntry {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TerminfoError> {
        let mut reader = Reader { bytes, offset: 0 };
        let wide = match reader.word()? {
            0o432 => false,
            0o1036 => true,
            _ => return Err(TerminfoError::Malformed("invalid magic")),
        };
        let name_size = reader.count()?;
        let bool_count = reader.count()?;
        let num_count = reader.count()?;
        let str_count = reader.count()?;
        let str_size = reader.count()?;
        let names = reader.take(name_size)?;
        let names = names[..names.iter().position(|&ch| ch == 0).unwrap_or(names.len())].to_vec();
        let booleans = reader.take(bool_count)?.iter().map(|&n| n == 1).collect();
        reader.align()?;
        let numbers = reader.numbers(num_count, wide)?;
        let offsets = reader.offsets(str_count)?;
        let table = reader.take(str_size)?;
        let strings = offsets
            .iter()
            .map(|&offset| table_string(table, offset).map(<[u8]>::to_vec))
            .collect();
        let mut result = Self {
            names,
            booleans,
            numbers,
            strings,
            extended: Vec::new(),
        };
        if reader.offset == bytes.len() {
            return Ok(result);
        }
        reader.align()?;
        // ncurses ignores a missing or all-zero extended header (read_shorts/valid_shorts).
        let header: Vec<u16> = bytes[reader.offset.min(bytes.len())..]
            .chunks_exact(2)
            .take(5)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if header.len() < 5 || !header.iter().any(|&n| (n as i16) > 0) {
            return Ok(result);
        }
        let bool_count = reader.count()?;
        let num_count = reader.count()?;
        let str_count = reader.count()?;
        // The header's string usage count is read but not checked, as in ncurses 6.6.
        reader.count()?;
        let size = reader.count()?;
        let name_count = bool_count + num_count + str_count;
        let bools = reader.take(bool_count)?;
        reader.align()?;
        let numbers = reader.numbers(num_count, wide)?;
        // All strings have offset slots, including cancelled/absent values.
        let offsets = reader.offsets(str_count + name_count)?;
        let table = reader.take(size)?;
        let mut values = Vec::with_capacity(str_count);
        let mut name_base = 0;
        for &offset in &offsets[..str_count] {
            let value = table_string(table, offset);
            if let Some(value) = value {
                name_base += value.len() + 1;
            }
            values.push(value);
        }
        let name_table = table
            .get(name_base..)
            .ok_or(TerminfoError::Malformed("extended name base outside table"))?;
        for (index, &offset) in offsets[str_count..].iter().enumerate() {
            let name = table_string(name_table, offset)
                .filter(|s| !s.is_empty())
                .ok_or(TerminfoError::Malformed("missing extended capability name"))?;
            let value = if index < bool_count {
                Capability::Flag(bools[index] == 1)
            } else if index < bool_count + num_count {
                numbers[index - bool_count].map_or(Capability::Missing, Capability::Number)
            } else {
                values[index - bool_count - num_count]
                    .map_or(Capability::Missing, |s| Capability::String(s.to_vec()))
            };
            result.extended.push((name.to_vec(), value));
        }
        Ok(result)
    }

    pub fn names(&self) -> &[u8] {
        &self.names
    }

    pub fn flag(&self, name: &[u8]) -> Option<bool> {
        if let Some(index) = BOOL_NAMES.iter().position(|n| n.as_bytes() == name) {
            return Some(self.booleans.get(index).copied().unwrap_or(false));
        }
        self.extended.iter().find_map(|(n, v)| match v {
            Capability::Flag(b) if n == name => Some(*b),
            _ => None,
        })
    }

    pub fn number(&self, name: &[u8]) -> Option<i32> {
        if let Some(index) = NUM_NAMES.iter().position(|n| n.as_bytes() == name) {
            return self.numbers.get(index).copied().flatten();
        }
        self.extended.iter().find_map(|(n, v)| match v {
            Capability::Number(i) if n == name => Some(*i),
            _ => None,
        })
    }

    pub fn string(&self, name: &[u8]) -> Option<&[u8]> {
        if let Some(index) = STR_NAMES.iter().position(|n| n.as_bytes() == name) {
            return self.strings.get(index).and_then(Option::as_deref);
        }
        self.extended.iter().find_map(|(n, v)| match v {
            Capability::String(s) if n == name => Some(s.as_slice()),
            _ => None,
        })
    }

    fn usable(&self) -> Result<(), TerminfoError> {
        if self.flag(b"gn") == Some(true) {
            if (self.string(b"cup").is_some()
                || self.string(b"cud1").is_some() && self.string(b"home").is_some())
                && self.string(b"clear").is_some()
            {
                return Err(TerminfoError::Hardcopy);
            }
            return Err(TerminfoError::Missing);
        }
        if self.flag(b"hc") == Some(true) {
            return Err(TerminfoError::Hardcopy);
        }
        Ok(())
    }

    pub fn capabilities(&self) -> Result<CapList, TerminfoError> {
        self.usable()?;
        let mut caps = Vec::with_capacity(CODES.len());
        for code in &CODES {
            let name = code.name.as_bytes();
            let value = match code.kind {
                CodeKind::String => self.string(name).map(<[u8]>::to_vec),
                CodeKind::Number => self.number(name).map(|n| n.to_string().into_bytes()),
                CodeKind::Flag => self.flag(name).map(|b| vec![if b { b'1' } else { b'0' }]),
            };
            if let Some(value) = value {
                let mut cap = Vec::with_capacity(name.len() + 1 + value.len());
                cap.extend_from_slice(name);
                cap.push(b'=');
                cap.extend_from_slice(&value);
                caps.push(cap.into());
            }
        }
        Ok(caps)
    }
}

// Homebrew ncursesw6-config --terminfo-dirs (6.6); no Apple database fallback.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const DEFAULT_DIRS: &[&str] = &["/opt/homebrew/opt/ncurses/share/terminfo"];
#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
const DEFAULT_DIRS: &[&str] = &["/usr/local/opt/ncurses/share/terminfo"];
#[cfg(not(target_os = "macos"))]
const DEFAULT_DIRS: &[&str] = &["/etc/terminfo", "/lib/terminfo", "/usr/share/terminfo"];

fn search_paths(
    terminfo: Option<&[u8]>,
    home: Option<&[u8]>,
    dirs: Option<&[u8]>,
    defaults: &[&str],
) -> Vec<Vec<u8>> {
    let mut paths = Vec::new();
    if let Some(path) = terminfo.filter(|s| !s.is_empty()) {
        paths.push(path.to_vec());
    }
    if let Some(home) = home.filter(|s| !s.is_empty()) {
        let mut path = home.to_vec();
        path.extend_from_slice(b"/.terminfo");
        paths.push(path);
    }
    if let Some(dirs) = dirs.filter(|s| !s.is_empty()) {
        for path in dirs.split(|&ch| ch == b':') {
            if path.is_empty() {
                paths.extend(defaults.iter().map(|s| s.as_bytes().to_vec()));
            } else {
                paths.push(path.to_vec());
            }
        }
    }
    paths.extend(defaults.iter().map(|s| s.as_bytes().to_vec()));
    paths
}

fn inline_bytes(path: &[u8]) -> Option<Vec<u8>> {
    let mut cleaned = Vec::with_capacity(path.len());
    let mut index = 0;
    while index < path.len() {
        let byte = path[index];
        if byte == b'\\' && path.get(index + 1) == Some(&b'\n') {
            index += 2;
            continue;
        }
        if byte != b'\n' && byte != b'\t' {
            cleaned.push(byte);
        }
        index += 1;
    }
    if let Some(hex) = cleaned.strip_prefix(b"hex:") {
        if hex.len() % 2 != 0 {
            return None;
        }
        return hex
            .chunks_exact(2)
            .map(|pair| {
                let high = (pair[0] as char).to_digit(16)?;
                let low = (pair[1] as char).to_digit(16)?;
                Some((high * 16 + low) as u8)
            })
            .collect();
    }
    let b64 = cleaned.strip_prefix(b"b64:")?;
    if b64.len() % 4 != 0 {
        return None;
    }
    let mut result = Vec::with_capacity(b64.len() / 4 * 3);
    for group in b64.chunks_exact(4) {
        let mut digits = [0_u8; 4];
        for (digit, byte) in digits.iter_mut().zip(group) {
            *digit = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'-' | b'+' => 62,
                b'_' | b'/' => 63,
                b'=' => 64,
                _ => return None,
            };
        }
        if digits[0] == 64 || digits[1] == 64 || digits[2] == 64 && digits[3] != 64 {
            return None;
        }
        result.push(digits[0] << 2 | digits[1] >> 4);
        if digits[2] != 64 {
            result.push(digits[1] << 4 | digits[2] >> 2);
            if digits[3] != 64 {
                result.push(digits[2] << 6 | digits[3]);
            }
        }
    }
    Some(result)
}

fn read_paths(name: &[u8], paths: &[Vec<u8>]) -> Result<TerminfoEntry, TerminfoError> {
    if name.is_empty()
        || name == b"."
        || name == b".."
        || name.contains(&b'/')
        || name.contains(&b':')
    {
        return Err(TerminfoError::Missing);
    }
    if name.len() > 512 {
        return Err(TerminfoError::NoDatabase);
    }
    let mut database = false;
    for path in paths {
        if path.starts_with(b"hex:") || path.starts_with(b"b64:") {
            database = true;
            if let Some(bytes) = inline_bytes(path) {
                if let Ok(entry) = TerminfoEntry::from_bytes(&bytes) {
                    if entry
                        .names
                        .split(|&ch| ch == b'|')
                        .any(|alias| alias == name)
                    {
                        return Ok(entry);
                    }
                }
            }
            continue;
        }
        let path = Path::new(std::ffi::OsStr::from_bytes(path));
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        // ncurses counts existing nonempty regular files as databases even in
        // a build without hashed database support (Homebrew ncurses 6.6).
        if metadata.is_dir() || metadata.is_file() && metadata.len() != 0 {
            database = true;
        }
        if !metadata.is_dir() {
            continue;
        }
        for leaf in [vec![name[0]], format!("{:02x}", name[0]).into_bytes()] {
            let filename = path
                .join(std::ffi::OsStr::from_bytes(&leaf))
                .join(std::ffi::OsStr::from_bytes(name));
            if let Ok(bytes) = std::fs::read(filename) {
                if let Ok(entry) = TerminfoEntry::from_bytes(&bytes) {
                    return Ok(entry);
                }
            }
        }
    }
    Err(if database {
        TerminfoError::Missing
    } else {
        TerminfoError::NoDatabase
    })
}

pub fn read_list(name: &[u8]) -> Result<CapList, TerminfoError> {
    let terminfo = std::env::var_os("TERMINFO").map(std::ffi::OsString::into_vec);
    let home = std::env::var_os("HOME").map(std::ffi::OsString::into_vec);
    let dirs = std::env::var_os("TERMINFO_DIRS").map(std::ffi::OsString::into_vec);
    let paths = search_paths(
        terminfo.as_deref(),
        home.as_deref(),
        dirs.as_deref(),
        DEFAULT_DIRS,
    );
    let mut entry = read_paths(rmux_util::bytes::cstr(name), &paths)?;
    if let (Some(proto), Some(command)) = (
        entry.string(b"cmdch").and_then(|s| s.first()).copied(),
        std::env::var_os("CC"),
    ) {
        let command = command.as_bytes();
        if command.len() == 1 {
            for s in entry.strings.iter_mut().flatten() {
                for ch in s {
                    if *ch == proto {
                        *ch = command[0];
                    }
                }
            }
            for (_, value) in &mut entry.extended {
                if let Capability::String(s) = value {
                    for ch in s {
                        if *ch == proto {
                            *ch = command[0];
                        }
                    }
                }
            }
        }
    }
    entry.capabilities()
}

// ncurses 6.6 include/Caps, $Id: Caps,v 1.62 2025/11/12 $.
const BOOL_NAMES: &[&str] = &[
    "bw", "am", "xsb", "xhp", "xenl", "eo", "gn", "hc", "km", "hs", "in", "da", "db", "mir",
    "msgr", "os", "eslok", "xt", "hz", "ul", "xon", "nxon", "mc5i", "chts", "nrrmc", "npc",
    "ndscr", "ccc", "bce", "hls", "xhpa", "crxm", "daisy", "xvpa", "sam", "cpix", "lpix", "OTbs",
    "OTns", "OTnc", "OTMT", "OTNL", "OTpt", "OTxr",
];

const NUM_NAMES: &[&str] = &[
    "cols", "it", "lines", "lm", "xmc", "pb", "vt", "wsl", "nlab", "lh", "lw", "ma", "wnum",
    "colors", "pairs", "ncv", "bufsz", "spinv", "spinh", "maddr", "mjump", "mcs", "mls", "npins",
    "orc", "orl", "orhi", "orvi", "cps", "widcs", "btns", "bitwin", "bitype", "OTug", "OTdC",
    "OTdN", "OTdB", "OTdT", "OTkn",
];

const STR_NAMES: &[&str] = &[
    "cbt", "bel", "cr", "csr", "tbc", "clear", "el", "ed", "hpa", "cmdch", "cup", "cud1", "home",
    "civis", "cub1", "mrcup", "cnorm", "cuf1", "ll", "cuu1", "cvvis", "dch1", "dl1", "dsl", "hd",
    "smacs", "blink", "bold", "smcup", "smdc", "dim", "smir", "invis", "prot", "rev", "smso",
    "smul", "ech", "rmacs", "sgr0", "rmcup", "rmdc", "rmir", "rmso", "rmul", "flash", "ff", "fsl",
    "is1", "is2", "is3", "if", "ich1", "il1", "ip", "kbs", "ktbc", "kclr", "kctab", "kdch1",
    "kdl1", "kcud1", "krmir", "kel", "ked", "kf0", "kf1", "kf10", "kf2", "kf3", "kf4", "kf5",
    "kf6", "kf7", "kf8", "kf9", "khome", "kich1", "kil1", "kcub1", "kll", "knp", "kpp", "kcuf1",
    "kind", "kri", "khts", "kcuu1", "rmkx", "smkx", "lf0", "lf1", "lf10", "lf2", "lf3", "lf4",
    "lf5", "lf6", "lf7", "lf8", "lf9", "rmm", "smm", "nel", "pad", "dch", "dl", "cud", "ich",
    "indn", "il", "cub", "cuf", "rin", "cuu", "pfkey", "pfloc", "pfx", "mc0", "mc4", "mc5", "rep",
    "rs1", "rs2", "rs3", "rf", "rc", "vpa", "sc", "ind", "ri", "sgr", "hts", "wind", "ht", "tsl",
    "uc", "hu", "iprog", "ka1", "ka3", "kb2", "kc1", "kc3", "mc5p", "rmp", "acsc", "pln", "kcbt",
    "smxon", "rmxon", "smam", "rmam", "xonc", "xoffc", "enacs", "smln", "rmln", "kbeg", "kcan",
    "kclo", "kcmd", "kcpy", "kcrt", "kend", "kent", "kext", "kfnd", "khlp", "kmrk", "kmsg", "kmov",
    "knxt", "kopn", "kopt", "kprv", "kprt", "krdo", "kref", "krfr", "krpl", "krst", "kres", "ksav",
    "kspd", "kund", "kBEG", "kCAN", "kCMD", "kCPY", "kCRT", "kDC", "kDL", "kslt", "kEND", "kEOL",
    "kEXT", "kFND", "kHLP", "kHOM", "kIC", "kLFT", "kMSG", "kMOV", "kNXT", "kOPT", "kPRV", "kPRT",
    "kRDO", "kRPL", "kRIT", "kRES", "kSAV", "kSPD", "kUND", "rfi", "kf11", "kf12", "kf13", "kf14",
    "kf15", "kf16", "kf17", "kf18", "kf19", "kf20", "kf21", "kf22", "kf23", "kf24", "kf25", "kf26",
    "kf27", "kf28", "kf29", "kf30", "kf31", "kf32", "kf33", "kf34", "kf35", "kf36", "kf37", "kf38",
    "kf39", "kf40", "kf41", "kf42", "kf43", "kf44", "kf45", "kf46", "kf47", "kf48", "kf49", "kf50",
    "kf51", "kf52", "kf53", "kf54", "kf55", "kf56", "kf57", "kf58", "kf59", "kf60", "kf61", "kf62",
    "kf63", "el1", "mgc", "smgl", "smgr", "fln", "sclk", "dclk", "rmclk", "cwin", "wingo", "hup",
    "dial", "qdial", "tone", "pulse", "hook", "pause", "wait", "u0", "u1", "u2", "u3", "u4", "u5",
    "u6", "u7", "u8", "u9", "op", "oc", "initc", "initp", "scp", "setf", "setb", "cpi", "lpi",
    "chr", "cvr", "defc", "swidm", "sdrfq", "sitm", "slm", "smicm", "snlq", "snrmq", "sshm",
    "ssubm", "ssupm", "sum", "rwidm", "ritm", "rlm", "rmicm", "rshm", "rsubm", "rsupm", "rum",
    "mhpa", "mcud1", "mcub1", "mcuf1", "mvpa", "mcuu1", "porder", "mcud", "mcub", "mcuf", "mcuu",
    "scs", "smgb", "smgbp", "smglp", "smgrp", "smgt", "smgtp", "sbim", "scsd", "rbim", "rcsd",
    "subcs", "supcs", "docr", "zerom", "csnm", "kmous", "minfo", "reqmp", "getm", "setaf", "setab",
    "pfxl", "devt", "csin", "s0ds", "s1ds", "s2ds", "s3ds", "smglr", "smgtb", "birep", "binel",
    "bicr", "colornm", "defbi", "endbi", "setcolor", "slines", "dispc", "smpch", "rmpch", "smsc",
    "rmsc", "pctrm", "scesc", "scesa", "ehhlm", "elhlm", "elohlm", "erhlm", "ethlm", "evhlm",
    "sgr1", "slength", "OTi2", "OTrs", "OTnl", "OTbc", "OTko", "OTma", "OTG2", "OTG3", "OTG1",
    "OTG4", "OTGR", "OTGL", "OTGU", "OTGD", "OTGH", "OTGV", "OTGC", "meml", "memu", "box1",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn words(out: &mut Vec<u8>, values: &[u16]) {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn fixture(wide: bool, extended: bool) -> Vec<u8> {
        let mut out = Vec::new();
        words(
            &mut out,
            &[if wide { 0o1036 } else { 0o432 }, 10, 2, 14, 6, 8],
        );
        out.extend_from_slice(b"sample|s\0\0");
        out.extend_from_slice(&[0, 0]);
        for index in 0..14 {
            let value: i32 = if index == 13 {
                if wide { 100000 } else { 256 }
            } else {
                -1
            };
            if wide {
                out.extend_from_slice(&value.to_le_bytes());
            } else {
                out.extend_from_slice(&(value as i16).to_le_bytes());
            }
        }
        words(&mut out, &[u16::MAX, 0, u16::MAX, u16::MAX, u16::MAX, 4]);
        out.extend_from_slice(b"\xff\x1bX\0abc\0");
        if extended {
            // Empty and cancelled values do not share the names' offset base.
            words(&mut out, &[1, 1, 3, 7, 23]);
            out.extend_from_slice(&[0, 0]);
            if wide {
                out.extend_from_slice(&8_i32.to_le_bytes());
            } else {
                words(&mut out, &[8]);
            }
            words(&mut out, &[0, 1, 0xfffe, 0, 3, 6, 9, 15]);
            out.extend_from_slice(b"\0x\0AX\0U8\0Ms\0Empty\0Gone\0");
        }
        out
    }

    #[test]
    fn both_numeric_widths_and_raw_strings() {
        for wide in [false, true] {
            let entry = TerminfoEntry::from_bytes(&fixture(wide, false)).unwrap();
            assert_eq!(entry.string(b"bel"), Some(b"\xff\x1bX".as_slice()));
            assert_eq!(entry.string(b"clear"), Some(b"abc".as_slice()));
            assert_eq!(
                entry.number(b"colors"),
                Some(if wide { 100000 } else { 256 })
            );
            assert_eq!(entry.number(b"cols"), None);
            assert_eq!(entry.string(b"cbt"), None);
            assert_eq!(entry.flag(b"am"), Some(false));
            assert_eq!(entry.flag(b"bce"), Some(false));
            assert_eq!(entry.flag(b"unknown"), None);
            let caps = entry.capabilities().unwrap();
            assert!(caps.iter().any(|s| s.as_bytes() == b"am=0"));
            assert!(!caps.iter().any(|s| s.starts_with(b"AX=")));
        }
    }

    #[test]
    fn extended_offsets_empty_cancelled_and_false() {
        for wide in [false, true] {
            let entry = TerminfoEntry::from_bytes(&fixture(wide, true)).unwrap();
            assert_eq!(entry.flag(b"AX"), Some(false));
            assert_eq!(entry.number(b"U8"), Some(8));
            assert_eq!(entry.string(b"Ms"), Some(b"".as_slice()));
            assert_eq!(entry.string(b"Empty"), Some(b"x".as_slice()));
            assert_eq!(entry.string(b"Gone"), None);
        }
    }

    #[test]
    fn malformed_sections_and_offsets() {
        let fixture = fixture(false, true);
        for len in 0..fixture.len() {
            let result = TerminfoEntry::from_bytes(&fixture[..len]);
            // A complete standard entry followed by fewer than five header
            // words has no extension (ncurses read_shorts fails silently).
            if (72..82).contains(&len) {
                assert!(result.unwrap().extended.is_empty(), "prefix {len}");
            } else {
                assert!(result.is_err(), "prefix {len}");
            }
        }
        // An all-zero extended header is also no extension.
        let mut zero = fixture[..72].to_vec();
        zero.extend_from_slice(&[0; 10]);
        assert!(
            TerminfoEntry::from_bytes(&zero)
                .unwrap()
                .extended
                .is_empty()
        );
        // Offsets outside the string table read as absent, as in convert_strings.
        let mut bad = fixture.clone();
        bad[54..56].copy_from_slice(&9_u16.to_le_bytes());
        let entry = TerminfoEntry::from_bytes(&bad).unwrap();
        assert_eq!(entry.string(b"bel"), None);
        assert_eq!(entry.string(b"clear"), Some(b"abc".as_slice()));
        bad[54..56].copy_from_slice(&8_u16.to_le_bytes());
        assert_eq!(
            TerminfoEntry::from_bytes(&bad).unwrap().string(b"bel"),
            None
        );
        // Extended capability names must be present and terminated.
        let mut bad = fixture;
        *bad.last_mut().unwrap() = b'x';
        assert!(TerminfoEntry::from_bytes(&bad).is_err());
    }

    #[test]
    fn pinned_standard_indices_include_second_attributes() {
        assert_eq!(
            (BOOL_NAMES.len(), NUM_NAMES.len(), STR_NAMES.len()),
            (44, 39, 414)
        );
        assert_eq!(BOOL_NAMES[1], "am");
        assert_eq!(BOOL_NAMES[28], "bce");
        assert_eq!(NUM_NAMES[13], "colors");
        assert_eq!(STR_NAMES[10], "cup");
        assert_eq!(STR_NAMES[392], "sgr1");
        let mut bytes = fixture(false, false);
        bytes[8..10].copy_from_slice(&393_u16.to_le_bytes());
        bytes.splice(
            52..64,
            (0..393).flat_map(|index| (if index == 392 { 4_u16 } else { u16::MAX }).to_le_bytes()),
        );
        let entry = TerminfoEntry::from_bytes(&bytes).unwrap();
        assert_eq!(entry.string(b"sgr1"), Some(b"abc".as_slice()));
        assert_eq!(entry.string(b"setal"), None);
    }

    #[test]
    fn directory_forms_precedence_inline_and_error_causes() {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let root: PathBuf = std::env::temp_dir().join(format!(
            "rmux-terminfo-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let alpha = root.join("alpha");
        let hex = root.join("hex");
        std::fs::create_dir_all(alpha.join("s")).unwrap();
        std::fs::create_dir_all(hex.join("73")).unwrap();
        std::fs::write(alpha.join("s/sample"), fixture(false, false)).unwrap();
        std::fs::write(hex.join("73/sample"), fixture(true, false)).unwrap();
        let paths = vec![
            alpha.as_os_str().as_bytes().to_vec(),
            hex.as_os_str().as_bytes().to_vec(),
        ];
        assert_eq!(
            read_paths(b"sample", &paths).unwrap().number(b"colors"),
            Some(256)
        );
        assert_eq!(
            read_paths(b"sample", &paths[1..])
                .unwrap()
                .number(b"colors"),
            Some(100000)
        );
        assert_eq!(
            read_paths(b"absent", &paths).unwrap_err(),
            TerminfoError::Missing
        );
        assert_eq!(
            read_paths(
                b"absent",
                &[root.join("absent").into_os_string().into_vec()]
            )
            .unwrap_err(),
            TerminfoError::NoDatabase
        );
        let encoded = fixture(false, true)
            .iter()
            .fold(String::from("hex:"), |mut s, b| {
                use std::fmt::Write;
                write!(s, "{b:02x}").unwrap();
                s
            });
        assert_eq!(
            read_paths(b"sample", &[encoded.into_bytes()])
                .unwrap()
                .number(b"U8"),
            Some(8)
        );
        assert_eq!(inline_bytes(b"b64:AAH-_w=="), Some(vec![0, 1, 254, 255]));
        assert_eq!(inline_bytes(b"hex:00\\\n\tff"), Some(vec![0, 255]));
        assert_eq!(
            search_paths(Some(b"a"), Some(b"h"), Some(b"b::c"), &["d"]),
            [
                b"a".to_vec(),
                b"h/.terminfo".to_vec(),
                b"b".to_vec(),
                b"d".to_vec(),
                b"c".to_vec(),
                b"d".to_vec()
            ]
        );
        for (error, expected) in [
            (
                TerminfoError::Hardcopy,
                b"can't use hardcopy terminal: \xff".as_slice(),
            ),
            (
                TerminfoError::Missing,
                b"missing or unsuitable terminal: \xff",
            ),
            (TerminfoError::NoDatabase, b"can't find terminfo database"),
            (TerminfoError::Unknown, b"unknown error"),
        ] {
            assert_eq!(error.cause(b"\xff").as_bytes(), expected);
        }
        let mut entry = TerminfoEntry::from_bytes(&fixture(false, false)).unwrap();
        entry.booleans.resize(8, false);
        entry.booleans[7] = true;
        assert_eq!(entry.capabilities().unwrap_err(), TerminfoError::Hardcopy);
        entry.booleans[6] = true;
        assert_eq!(entry.capabilities().unwrap_err(), TerminfoError::Missing);
        std::fs::remove_dir_all(root).unwrap();
    }
}
