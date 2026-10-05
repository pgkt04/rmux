// Ported from tmux prompt-history.c @ 8f25579c
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

use super::{PROMPT_NTYPES, PromptType, prompt_type, prompt_type_string};
use crate::model::Server;
use rmux_util::bytes::ByteString;
use rmux_util::log_debug;
use std::io::{BufRead, Write};
use std::os::unix::ffi::OsStrExt;

/// The two per-type history lists, owned by `Server`.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct PromptHistory {
    lists: [Vec<ByteString>; PROMPT_NTYPES],
}

fn slot(ty: PromptType) -> Option<usize> {
    match ty {
        PromptType::Command => Some(0),
        PromptType::Search => Some(1),
        PromptType::Invalid => None,
    }
}

const TYPES: [PromptType; PROMPT_NTYPES] = [PromptType::Command, PromptType::Search];

/// Find the history file to load/save from/to.
fn find_history_file(srv: &Server) -> Option<Vec<u8>> {
    let history_file = srv.options.get_string(srv.options.global, b"history-file");
    if history_file.is_empty() {
        return None;
    }
    if history_file[0] == b'/' {
        return Some(history_file.to_vec());
    }
    if history_file.len() < 2 || history_file[0] != b'~' || history_file[1] != b'/' {
        return None;
    }
    let mut home = rmux_sys::proc::home_directory(None)?;
    home.extend_from_slice(&history_file[1..]);
    Some(home)
}

/// Add loaded history item to the appropriate list.
fn add_typed_history(h: &mut PromptHistory, limit: u32, line: &[u8]) {
    let (typestr, rest) = rmux_util::bytes::strsep(line, b":");
    let ty = match rest {
        Some(_) => prompt_type(typestr),
        None => PromptType::Invalid,
    };
    if ty == PromptType::Invalid {
        // Old history files have no type: keep the whole line, colon included.
        add(h, limit, line, PromptType::Command);
    } else {
        add(h, limit, rest.unwrap_or(b""), ty);
    }
}

fn history_limit(srv: &Server) -> u32 {
    srv.options
        .get_number(srv.options.global, b"prompt-history-limit")
        .max(0) as u32
}

/// Load prompt history from file.
pub fn load(srv: &mut Server) {
    let Some(path) = find_history_file(srv) else {
        return;
    };
    let display = String::from_utf8_lossy(&path).into_owned();
    log_debug!("loading history from {}", display);
    let file = match std::fs::File::open(std::ffi::OsStr::from_bytes(&path)) {
        Ok(f) => f,
        Err(e) => {
            log_debug!("{}: {}", display, e);
            return;
        }
    };
    let limit = history_limit(srv);
    let mut reader = std::io::BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.is_empty() && !line.is_empty() {
            continue;
        }
        add_typed_history(&mut srv.prompt_history, limit, &line);
    }
}

/// Save prompt history to file.
pub fn save(srv: &Server) {
    let Some(path) = find_history_file(srv) else {
        return;
    };
    let display = String::from_utf8_lossy(&path).into_owned();
    log_debug!("saving history to {}", display);
    let mut file = match std::fs::File::create(std::ffi::OsStr::from_bytes(&path)) {
        Ok(f) => f,
        Err(e) => {
            log_debug!("{}: {}", display, e);
            return;
        }
    };
    let mut out = Vec::new();
    for ty in TYPES {
        for line in &srv.prompt_history.lists[slot(ty).unwrap()] {
            out.extend_from_slice(prompt_type_string(ty).as_bytes());
            out.push(b':');
            out.extend_from_slice(line);
            out.push(b'\n');
        }
    }
    let _ = file.write_all(&out);
}

/// Get previous line from the history. History runs from 0 to size - 1;
/// index is from 0 to size, zero is empty.
pub fn up<'a>(
    h: &'a PromptHistory,
    idx: &mut [u32; PROMPT_NTYPES],
    ty: PromptType,
) -> Option<&'a [u8]> {
    let t = slot(ty)?;
    let size = h.lists[t].len() as u32;
    if size == 0 || idx[t] == size {
        return None;
    }
    idx[t] += 1;
    Some(&h.lists[t][(size - idx[t]) as usize])
}

/// Get next line from the history.
pub fn down<'a>(h: &'a PromptHistory, idx: &mut [u32; PROMPT_NTYPES], ty: PromptType) -> &'a [u8] {
    let Some(t) = slot(ty) else {
        return b"";
    };
    let size = h.lists[t].len() as u32;
    if size == 0 || idx[t] == 0 {
        return b"";
    }
    idx[t] -= 1;
    if idx[t] == 0 {
        return b"";
    }
    &h.lists[t][(size - idx[t]) as usize]
}

/// Add line to the history.
pub fn add(h: &mut PromptHistory, limit: u32, line: &[u8], ty: PromptType) {
    let Some(t) = slot(ty) else {
        return;
    };
    let list = &mut h.lists[t];
    let oldsize = list.len() as u32;
    let new = !(oldsize > 0 && list[oldsize as usize - 1].as_bytes() == line);
    let newsize;
    if limit > oldsize {
        if !new {
            return;
        }
        newsize = oldsize + 1;
    } else {
        newsize = limit;
        let mut freecount = oldsize + u32::from(new) - newsize;
        if freecount > oldsize {
            freecount = oldsize;
        }
        if freecount == 0 {
            return;
        }
        list.drain(..freecount as usize);
    }
    if new && newsize > 0 {
        list.push(line.into());
    }
    list.truncate(newsize as usize);
}

/// Get history size.
pub fn size(h: &PromptHistory, ty: PromptType) -> u32 {
    slot(ty).map_or(0, |t| h.lists[t].len() as u32)
}

/// Get history entry.
pub fn get(h: &PromptHistory, ty: PromptType, idx: u32) -> Option<&[u8]> {
    h.lists[slot(ty)?].get(idx as usize).map(|b| b.as_bytes())
}

/// Clear prompt history.
pub fn clear(h: &mut PromptHistory, ty: PromptType) {
    if let Some(t) = slot(ty) {
        h.lists[t].clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(h: &PromptHistory, ty: PromptType) -> Vec<&[u8]> {
        (0..size(h, ty)).filter_map(|i| get(h, ty, i)).collect()
    }

    #[test]
    fn add_limits_and_duplicates() {
        let mut h = PromptHistory::default();
        let c = PromptType::Command;
        add(&mut h, 3, b"a", c);
        add(&mut h, 3, b"b", c);
        add(&mut h, 3, b"b", c);
        assert_eq!(lines(&h, c), [b"a", b"b"]);
        add(&mut h, 3, b"c", c);
        add(&mut h, 3, b"d", c);
        assert_eq!(lines(&h, c), [b"b", b"c", b"d"]);
        // Duplicate last line still trims when the limit shrinks.
        add(&mut h, 2, b"d", c);
        assert_eq!(lines(&h, c), [b"c", b"d"]);
        // Limit zero removes everything.
        add(&mut h, 0, b"e", c);
        assert_eq!(size(&h, c), 0);
        add(&mut h, 5, b"x", PromptType::Invalid);
        assert_eq!(size(&h, PromptType::Invalid), 0);
        assert_eq!(size(&h, PromptType::Search), 0);
    }

    #[test]
    fn up_and_down_traversal() {
        let mut h = PromptHistory::default();
        let c = PromptType::Command;
        for l in [b"one", b"two"] {
            add(&mut h, 10, l, c);
        }
        let mut idx = [0u32; PROMPT_NTYPES];
        assert_eq!(up(&h, &mut idx, c), Some(&b"two"[..]));
        assert_eq!(up(&h, &mut idx, c), Some(&b"one"[..]));
        assert_eq!(up(&h, &mut idx, c), None);
        assert_eq!(down(&h, &mut idx, c), b"two");
        assert_eq!(down(&h, &mut idx, c), b"");
        assert_eq!(down(&h, &mut idx, c), b"");
        assert_eq!(up(&h, &mut idx, PromptType::Search), None);
        assert_eq!(down(&h, &mut idx, PromptType::Invalid), b"");
    }

    #[test]
    fn typed_lines_and_legacy_lines() {
        let mut h = PromptHistory::default();
        add_typed_history(&mut h, 10, b"command:ls -la");
        add_typed_history(&mut h, 10, b"search:foo:bar");
        add_typed_history(&mut h, 10, b"bogus:keep me");
        add_typed_history(&mut h, 10, b"plain line");
        assert_eq!(
            lines(&h, PromptType::Command),
            [&b"ls -la"[..], b"bogus:keep me", b"plain line"]
        );
        assert_eq!(lines(&h, PromptType::Search), [b"foo:bar"]);
        clear(&mut h, PromptType::Search);
        assert_eq!(size(&h, PromptType::Search), 0);
    }
}
