// Ported from tmux screen.c @ 8f25579c
//! Main-screen anchor ownership across alternate-screen snapshots.

use super::Screen;
use crate::grid::SurfaceAnchorId;

impl Screen {
    /// Absolute main-grid row, including the saved main view while alternate.
    pub fn surface_anchor_row(&self, id: SurfaceAnchorId) -> Option<u32> {
        self.grid.surface_anchor_row(id).or_else(|| {
            self.saved_grid
                .as_ref()
                .and_then(|saved| saved.surface_anchor_row(id))
                .map(|row| self.grid.hsize() + row)
        })
    }

    pub fn remove_surface_anchor(&mut self, id: SurfaceAnchorId) -> bool {
        if self.grid.remove_surface_anchor(id) {
            return true;
        }
        self.saved_grid
            .as_mut()
            .is_some_and(|saved| saved.remove_surface_anchor(id))
    }

    /// RIS and prompt owners can discard anchors in both buffers without grid changes.
    pub fn clear_surface_anchors(&mut self) {
        self.grid.clear_surface_anchors();
        if let Some(saved) = &mut self.saved_grid {
            saved.clear_surface_anchors();
        }
    }

    pub fn drain_surface_anchor_removals(&mut self) -> impl Iterator<Item = SurfaceAnchorId> + '_ {
        self.grid.drain_surface_anchor_removals().chain(
            self.saved_grid
                .iter_mut()
                .flat_map(|saved| saved.drain_surface_anchor_removals()),
        )
    }
}
