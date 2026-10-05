// Ported from tmux grid.c and grid-view.c @ 8f25579c
//! TSP row ownership. Ordinary line duplication deliberately omits ownership.

use super::{Grid, GridLineFlags, SurfaceAnchorId};
use std::ops::Range;

impl Grid {
    /// Absolute history/view row of an owned anchor.
    pub fn surface_anchor_row(&self, id: SurfaceAnchorId) -> Option<u32> {
        self.lines
            .iter()
            .position(|line| line.surface_anchor == Some(id))
            .map(|row| row as u32)
    }

    /// Attach to an existing empty absolute row; insertion/cursor policy is the writer's.
    pub fn attach_surface_anchor(&mut self, row: u32, id: SurfaceAnchorId) -> bool {
        if row >= self.hsize + self.sy || self.surface_anchor_row(id).is_some() {
            return false;
        }
        let line = &mut self.lines[row as usize];
        if line.cellused() != 0 || line.surface_anchor.is_some() {
            return false;
        }
        line.flags.remove(GridLineFlags::WRAPPED);
        line.surface_anchor = Some(id);
        if row != 0 {
            self.lines[row as usize - 1]
                .flags
                .remove(GridLineFlags::WRAPPED);
        }
        true
    }

    /// Remove ownership without changing ordinary grid rows or cursor arithmetic.
    pub fn remove_surface_anchor(&mut self, id: SurfaceAnchorId) -> bool {
        let Some(row) = self.surface_anchor_row(id) else {
            return false;
        };
        self.remove_surface_anchors_in(row as usize..row as usize + 1);
        true
    }

    pub fn clear_surface_anchors(&mut self) {
        self.remove_surface_anchors_in(0..self.lines.len());
    }

    /// Drain exactly-once removals after parsing, history collection or resize.
    pub fn drain_surface_anchor_removals(&mut self) -> impl Iterator<Item = SurfaceAnchorId> + '_ {
        self.surface_anchor_removals.drain(..)
    }

    pub(super) fn remove_surface_anchors_in(&mut self, rows: Range<usize>) {
        for line in &mut self.lines[rows] {
            if let Some(id) = line.surface_anchor.take() {
                self.surface_anchor_removals.push(id);
            }
        }
    }

    pub(crate) fn record_surface_anchor_removals(
        &mut self,
        removals: impl Iterator<Item = SurfaceAnchorId>,
    ) {
        self.surface_anchor_removals.extend(removals);
    }

    /// Alternate-screen snapshots transfer ownership; copy-mode duplication does not.
    pub(crate) fn transfer_surface_anchors(
        &mut self,
        destination: u32,
        source: &mut Grid,
        first: u32,
        count: u32,
    ) {
        self.surface_anchor_removals
            .extend(source.drain_surface_anchor_removals());
        for offset in 0..count {
            let id = source.lines[(first + offset) as usize]
                .surface_anchor
                .take();
            self.lines[(destination + offset) as usize].surface_anchor = id;
        }
    }
}
