// SPDX-License-Identifier: GPL-2.0-only

//! The store's reserved cells as a persistent index (`ROAD-68-E`): each block's reserved mask and
//! grid frame, under every 512 m chunk its bounds reach, in `imbl` maps. The road tool's view
//! shares it in `O(1)` per publication, and a paint or claim that changes a block's mask updates
//! it in `O(chunks the block reaches * log blocks)`. It answers the same footprint query as the
//! store ([`CellStore::visit_reserved_footprints_in_bounds`]) over the same cells.
//!
//! [`CellStore::visit_reserved_footprints_in_bounds`]: super::CellStore::visit_reserved_footprints_in_bounds

use super::{BlockKey, CellBounds, CellKey, GridFrame, block_bounds, chunk_at, chunks_for_bounds};
use glam::DVec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReservedBlock {
    frame: GridFrame,
    mask: u64,
}

/// Every reserved cell (painted, or claimed by a lot in redevelopment grace), by block and
/// 512 m chunk; cheap to copy and safe to read off the simulation thread.
#[derive(Clone, Debug, Default)]
pub(crate) struct CellReservations {
    chunks: imbl::OrdMap<(i32, i32), imbl::OrdMap<BlockKey, ReservedBlock>>,
}

impl CellReservations {
    /// Whether no cell is reserved.
    pub(crate) fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    // Sets block `key`'s reserved mask on `frame`; zero removes the block.
    pub(super) fn set(&mut self, key: BlockKey, frame: GridFrame, mask: u64) {
        for chunk in chunks_for_bounds(block_bounds(&frame.geometry(), key)) {
            if mask == 0 {
                let Some(blocks) = self.chunks.get_mut(&chunk) else {
                    continue;
                };
                blocks.remove(&key);
                if blocks.is_empty() {
                    self.chunks.remove(&chunk);
                }
            } else if let Some(blocks) = self.chunks.get_mut(&chunk) {
                blocks.insert(key, ReservedBlock { frame, mask });
            } else {
                self.chunks.insert(
                    chunk,
                    imbl::OrdMap::unit(key, ReservedBlock { frame, mask }),
                );
            }
        }
    }

    /// Visits each reserved cell whose footprint bounds meet `bounds`, with its corners, once:
    /// a block in several chunks is visited in the first chunk shared by its bounds and the
    /// query, as the store does. `O(blocks in the query's chunks + their reserved cells)`, no
    /// allocation.
    pub(crate) fn visit_footprints_in_bounds(
        &self,
        bounds: CellBounds,
        mut visit: impl FnMut(CellKey, [DVec2; 4]),
    ) {
        if !bounds.is_valid() {
            return;
        }
        for chunk in chunks_for_bounds(bounds) {
            let Some(blocks) = self.chunks.get(&chunk) else {
                continue;
            };
            for (&key, block) in blocks {
                let geometry = block.frame.geometry();
                let block_bounds = block_bounds(&geometry, key);
                if !block_bounds.intersects(bounds)
                    || chunk_at(block_bounds.min.max(bounds.min)) != chunk
                {
                    continue;
                }
                let mut mask = block.mask;
                while mask != 0 {
                    let offset = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let cell = key.cell(offset);
                    let corners =
                        geometry.rectangle_corners(f64::from(cell.x), f64::from(cell.y), 1.0, 1.0);
                    if CellBounds::from_points(corners).intersects(bounds) {
                        visit(cell, corners);
                    }
                }
            }
        }
    }
}
