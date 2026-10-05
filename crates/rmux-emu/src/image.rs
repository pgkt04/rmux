// Ported from tmux image.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

pub mod sixel;
pub use sixel::{SixelError, SixelImage};
use std::collections::VecDeque;

#[cfg(test)]
#[path = "image/registry_tests.rs"]
mod registry_tests;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImageOwnerId {
    slot: usize,
    generation: u64,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImageId {
    slot: usize,
    generation: u64,
}
pub type Images = Vec<ImageId>;
#[derive(Debug)]
pub struct Image {
    pub px: u32,
    pub py: u32,
    pub sx: u32,
    pub sy: u32,
    pub data: SixelImage,
    pub fallback: Vec<u8>,
}
impl Image {
    fn log(&self, from: &str) {
        if rmux_util::log::level().0 != 0 {
            rmux_util::log_debug!(
                "{}: {:p} ({}x{} {},{})",
                from,
                self,
                self.sx,
                self.sy,
                self.px,
                self.py
            );
        }
    }
}
#[derive(Debug, Default)]
struct ImageSlot {
    generation: u64,
    image: Option<Image>,
}
#[derive(Debug, Default)]
struct Owner {
    slots: Vec<ImageSlot>,
    ordered: Images,
}
#[derive(Debug, Default)]
struct OwnerSlot {
    generation: u64,
    owner: Option<Owner>,
}
#[derive(Debug, Default)]
pub struct ImageRegistry {
    owners: Vec<OwnerSlot>,
    fifo: VecDeque<(ImageOwnerId, ImageId)>,
}

pub fn fallback(sx: u32, sy: u32) -> Vec<u8> {
    if sy == 0 {
        return Vec::new();
    }
    let mut out = format!("SIXEL IMAGE ({sx}x{sy})").into_bytes();
    if out.len() < sx as usize {
        out.resize(sx as usize, b'+');
    }
    out.extend_from_slice(b"\r\n");
    for _ in 1..sy {
        let end = out.len() + sx as usize;
        out.resize(end, b'+');
        out.extend_from_slice(b"\r\n");
    }
    out
}

impl ImageRegistry {
    pub fn create_owner(&mut self) -> ImageOwnerId {
        if let Some((slot, entry)) = self
            .owners
            .iter_mut()
            .enumerate()
            .find(|(_, e)| e.owner.is_none() && e.generation != u64::MAX)
        {
            entry.generation += 1;
            entry.owner = Some(Owner::default());
            return ImageOwnerId {
                slot,
                generation: entry.generation,
            };
        }
        let slot = self.owners.len();
        self.owners.push(OwnerSlot {
            generation: 1,
            owner: Some(Owner::default()),
        });
        ImageOwnerId {
            slot,
            generation: 1,
        }
    }
    fn owner(&self, id: ImageOwnerId) -> Option<&Owner> {
        self.owners
            .get(id.slot)
            .filter(|s| s.generation == id.generation)?
            .owner
            .as_ref()
    }
    fn owner_mut(&mut self, id: ImageOwnerId) -> Option<&mut Owner> {
        self.owners
            .get_mut(id.slot)
            .filter(|s| s.generation == id.generation)?
            .owner
            .as_mut()
    }
    pub fn release_owner(&mut self, owner: ImageOwnerId) {
        self.free_all(owner);
        if let Some(entry) = self
            .owners
            .get_mut(owner.slot)
            .filter(|s| s.generation == owner.generation)
        {
            entry.owner = None;
        }
    }
    pub fn store(&mut self, owner: ImageOwnerId, data: SixelImage, px: u32, py: u32) -> ImageId {
        let (sx, sy) = data.size_in_cells();
        let image = Image {
            px,
            py,
            sx,
            sy,
            data,
            fallback: fallback(sx, sy),
        };
        let list = self
            .owner_mut(owner)
            .expect("image store requires live owner");
        let id = if let Some((slot, entry)) = list
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.image.is_none() && s.generation != u64::MAX)
        {
            entry.generation += 1;
            entry.image = Some(image);
            ImageId {
                slot,
                generation: entry.generation,
            }
        } else {
            let slot = list.slots.len();
            list.slots.push(ImageSlot {
                generation: 1,
                image: Some(image),
            });
            ImageId {
                slot,
                generation: 1,
            }
        };
        list.ordered.push(id);
        list.slots[id.slot]
            .image
            .as_ref()
            .unwrap()
            .log("image_store");
        self.fifo.push_back((owner, id));
        if self.fifo.len() == 20 {
            let (old_owner, old_id) = self.fifo[0];
            self.remove(old_owner, old_id);
        }
        id
    }
    pub fn get(&self, owner: ImageOwnerId, id: ImageId) -> Option<&Image> {
        self.owner(owner)?
            .slots
            .get(id.slot)
            .filter(|s| s.generation == id.generation)?
            .image
            .as_ref()
    }
    pub fn ordered(&self, owner: ImageOwnerId) -> &[ImageId] {
        self.owner(owner).map_or(&[], |o| o.ordered.as_slice())
    }
    pub fn remove(&mut self, owner: ImageOwnerId, id: ImageId) -> bool {
        let Some(list) = self.owner_mut(owner) else {
            return false;
        };
        let Some(slot) = list
            .slots
            .get_mut(id.slot)
            .filter(|s| s.generation == id.generation && s.image.is_some())
        else {
            return false;
        };
        slot.image.as_ref().unwrap().log("image_free");
        slot.image = None;
        list.ordered.retain(|&other| other != id);
        self.fifo.retain(|&pair| pair != (owner, id));
        true
    }
    pub fn free_all(&mut self, owner: ImageOwnerId) -> bool {
        let nonempty = !self.ordered(owner).is_empty();
        while let Some(&id) = self.ordered(owner).first() {
            self.remove(owner, id);
        }
        nonempty
    }
    fn invalidate(&mut self, owner: ImageOwnerId, overlaps: impl Fn(&Image) -> bool) -> bool {
        let mut index = 0;
        let mut redraw = false;
        while let Some(&id) = self.ordered(owner).get(index) {
            if self.get(owner, id).is_some_and(&overlaps) {
                self.remove(owner, id);
                redraw = true;
            } else {
                index += 1;
            }
        }
        redraw
    }
    pub fn check_line(&mut self, owner: ImageOwnerId, py: u32, ny: u32) -> bool {
        self.invalidate(owner, |im| {
            py.wrapping_add(ny) > im.py && py < im.py.wrapping_add(im.sy)
        })
    }
    pub fn check_area(&mut self, owner: ImageOwnerId, px: u32, py: u32, nx: u32, ny: u32) -> bool {
        self.invalidate(owner, |im| {
            py < im.py.wrapping_add(im.sy)
                && py.wrapping_add(ny) > im.py
                && px < im.px.wrapping_add(im.sx)
                && px.wrapping_add(nx) > im.px
        })
    }
    pub fn scroll_up(&mut self, owner: ImageOwnerId, lines: u32) -> bool {
        let redraw = !self.ordered(owner).is_empty();
        let mut index = 0;
        while let Some(&id) = self.ordered(owner).get(index) {
            let im = self.get(owner, id).expect("ordered image is live");
            if im.py < lines && im.py.wrapping_add(im.sy) <= lines {
                self.remove(owner, id);
                continue;
            }
            let im = self.owner_mut(owner).unwrap().slots[id.slot]
                .image
                .as_mut()
                .unwrap();
            if im.py >= lines {
                im.py -= lines;
            } else {
                let sy = im.py.wrapping_add(im.sy).wrapping_sub(lines);
                im.data = im
                    .data
                    .scale(None, None, 0, im.sy - sy, im.sx, sy, true)
                    .expect("partial image crop has valid origin");
                im.py = 0;
                (im.sx, im.sy) = im.data.size_in_cells();
                im.fallback = fallback(im.sx, im.sy);
            }
            index += 1;
        }
        redraw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    fn image() -> SixelImage {
        SixelImage::parse(
            b"q\"1;1;32;64#0;2;1;2;3~",
            0,
            NonZeroU32::new(16).unwrap(),
            NonZeroU32::new(32).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn fallback_exact() {
        assert_eq!(fallback(2, 2), b"SIXEL IMAGE (2x2)\r\n++\r\n");
        assert_eq!(fallback(20, 1), b"SIXEL IMAGE (20x1)++\r\n");
        assert!(fallback(20, 0).is_empty());
    }
    #[test]
    fn global_fifo_and_generations() {
        let mut r = ImageRegistry::default();
        let a = r.create_owner();
        let b = r.create_owner();
        let first = r.store(a, image(), 0, 0);
        for _ in 0..19 {
            r.store(b, image(), 0, 0);
        }
        assert!(r.get(a, first).is_none());
        assert_eq!(r.ordered(b).len(), 19);
        assert!(!r.free_all(a));
        r.release_owner(b);
        let new = r.create_owner();
        assert_ne!(new, b);
        assert!(r.ordered(b).is_empty());
        assert!(!r.remove(b, first));
    }
    #[test]
    fn exhausted_generations_are_retired() {
        let mut r = ImageRegistry::default();
        let owner = r.create_owner();
        let id = r.store(owner, image(), 0, 0);
        r.remove(owner, id);
        r.owner_mut(owner).unwrap().slots[id.slot].generation = u64::MAX;
        let fresh_image = r.store(owner, image(), 0, 0);
        assert_ne!(fresh_image.slot, id.slot);
        r.release_owner(owner);
        r.owners[owner.slot].generation = u64::MAX;
        let fresh_owner = r.create_owner();
        assert_ne!(fresh_owner.slot, owner.slot);
        assert!(r.get(owner, id).is_none());
    }
    #[test]
    fn overlap_preserves_order_and_local_reuse_stays_dead() {
        let mut r = ImageRegistry::default();
        let owner = r.create_owner();
        let a = r.store(owner, image(), 0, 0);
        let b = r.store(owner, image(), 0, 0);
        assert_eq!(r.ordered(owner), &[a, b]);
        assert!(r.remove(owner, a));
        let c = r.store(owner, image(), 0, 0);
        assert_ne!(a, c);
        assert!(r.get(owner, a).is_none());
        assert_eq!(r.ordered(owner), &[b, c]);
        assert!(r.free_all(owner));
        assert!(!r.free_all(owner));
    }
    #[test]
    fn crop_does_not_refresh_global_age() {
        let mut r = ImageRegistry::default();
        let saved = r.create_owner();
        let alternate = r.create_owner();
        let old = r.store(saved, image(), 0, 0);
        assert!(r.scroll_up(saved, 1));
        for _ in 0..18 {
            r.store(alternate, image(), 0, 0);
        }
        assert!(r.get(saved, old).is_some());
        r.store(alternate, image(), 0, 0);
        assert!(r.get(saved, old).is_none());
        r.release_owner(saved);
        r.release_owner(alternate);
        assert!(r.fifo.is_empty());
    }
    #[test]
    fn overlap_edges_zero_spans_and_wrapping() {
        let mut r = ImageRegistry::default();
        let owner = r.create_owner();
        r.store(owner, image(), 3, 4);
        assert!(!r.check_area(owner, 5, 4, 1, 1));
        assert!(!r.check_line(owner, 6, 1));
        assert!(r.check_area(owner, 4, 5, 0, 0));
        r.store(owner, image(), 3, 4);
        assert!(!r.check_line(owner, u32::MAX, 6));
        assert!(r.check_line(owner, 5, 0));
    }
    #[test]
    fn scroll_crop_retains_fifo_and_rebuilds_fallback() {
        let mut r = ImageRegistry::default();
        let owner = r.create_owner();
        let id = r.store(owner, image(), 7, 1);
        assert!(r.scroll_up(owner, 0));
        assert!(r.scroll_up(owner, 2));
        let im = r.get(owner, id).unwrap();
        assert_eq!((im.px, im.py, im.sx, im.sy), (7, 0, 2, 1));
        assert_eq!(im.fallback, fallback(2, 1));
        assert!(r.scroll_up(owner, 1));
        assert!(r.get(owner, id).is_none());
    }
}
