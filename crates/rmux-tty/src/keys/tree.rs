// Ported from tmux tty-keys.c @ 8f25579c
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

//! The ternary key tree (`struct tty_key`, `tty_keys_add`, `tty_keys_add1`,
//! `tty_keys_find`, `tty_keys_find1`; `tty-keys.c:65-74,433-487,563-604`).
//! Nodes live in one `Vec` and link by index; traversal is iterative.

use rmux_util::key::{KeyCode, SpecialKey};

pub const UNKNOWN: KeyCode = KeyCode(SpecialKey::UNKNOWN);

/// One tree node (`tty-keys.c:66-74`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TtyKey {
    pub ch: u8,
    pub key: KeyCode,
    pub left: Option<u32>,
    pub right: Option<u32>,
    pub next: Option<u32>,
}

/// Which pointer `tty_keys_add1` is filling (`struct tty_key **tkp`).
#[derive(Clone, Copy)]
enum Slot {
    Root,
    Left(usize),
    Right(usize),
    Next(usize),
}

#[derive(Clone, Debug, Default)]
pub struct KeyTree {
    nodes: Vec<TtyKey>,
    root: Option<u32>,
}

/// C compares `char`, which is signed on every supported target
/// (`tty-keys.c:479-482,596-599`); the shape follows that order.
fn less(a: u8, b: u8) -> bool {
    (a as i8) < (b as i8)
}

impl KeyTree {
    /// `tty_keys_free`: drop every node but keep the allocation for rebuild.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = None;
    }

    pub fn nodes(&self) -> &[TtyKey] {
        &self.nodes
    }

    pub fn node(&self, index: u32) -> &TtyKey {
        &self.nodes[index as usize]
    }

    fn slot(&self, slot: Slot) -> Option<u32> {
        match slot {
            Slot::Root => self.root,
            Slot::Left(i) => self.nodes[i].left,
            Slot::Right(i) => self.nodes[i].right,
            Slot::Next(i) => self.nodes[i].next,
        }
    }

    fn set_slot(&mut self, slot: Slot, index: u32) {
        match slot {
            Slot::Root => self.root = Some(index),
            Slot::Left(i) => self.nodes[i].left = Some(index),
            Slot::Right(i) => self.nodes[i].right = Some(index),
            Slot::Next(i) => self.nodes[i].next = Some(index),
        }
    }

    /// `tty_keys_add` (`tty-keys.c:435-449`): a lookup hit replaces that
    /// node's key, whatever its depth; otherwise insert. An empty sequence is
    /// inert (C reads past the terminator there, `tty-keys.c:459-471`).
    pub fn add(&mut self, s: &[u8], key: KeyCode) {
        if s.is_empty() {
            return;
        }
        match self.find(s) {
            (Some(index), _) => self.nodes[index as usize].key = key,
            (None, _) => self.add1(s, key),
        }
    }

    /// `tty_keys_add1` (`tty-keys.c:453-487`).
    fn add1(&mut self, s: &[u8], key: KeyCode) {
        let mut slot = Slot::Root;
        let mut i = 0;
        loop {
            let index = match self.slot(slot) {
                Some(index) => index as usize,
                None => {
                    let index = self.nodes.len();
                    self.nodes.push(TtyKey {
                        ch: s[i],
                        key: UNKNOWN,
                        left: None,
                        right: None,
                        next: None,
                    });
                    self.set_slot(slot, index as u32);
                    index
                }
            };
            let ch = self.nodes[index].ch;
            if s[i] == ch {
                i += 1;
                if i == s.len() {
                    self.nodes[index].key = key;
                    return;
                }
                slot = Slot::Next(index);
            } else if less(s[i], ch) {
                slot = Slot::Left(index);
            } else {
                slot = Slot::Right(index);
            }
        }
    }

    /// `tty_keys_find` / `tty_keys_find1` (`tty-keys.c:565-604`): the node
    /// where input ends, or a leaf with a key even if input remains; `size`
    /// is the number of matched bytes.
    pub fn find(&self, buf: &[u8]) -> (Option<u32>, usize) {
        let mut size = 0;
        let mut node = self.root;
        let mut buf = buf;
        loop {
            if buf.is_empty() {
                return (None, size);
            }
            let Some(index) = node else {
                return (None, size);
            };
            let tk = &self.nodes[index as usize];
            if tk.ch == buf[0] {
                buf = &buf[1..];
                size += 1;
                if buf.is_empty() || (tk.next.is_none() && tk.key != UNKNOWN) {
                    return (Some(index), size);
                }
                node = tk.next;
            } else if less(buf[0], tk.ch) {
                node = tk.left;
            } else {
                node = tk.right;
            }
        }
    }
}
