// SPDX-License-Identifier: GPL-2.0-only

//! Generation completeness and paint revisions on the existing cell spatial chunks.

#[cfg(test)]
use super::chunk_at;
use super::maps::StoreMap;
use super::{CellBounds, CellKey, CellMaps, CellStore, DVec2, RegionGraph, chunks_for_bounds};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::types::TransitType;
use crate::simulation::zoning::cells::CELL_DEPTH;
use std::collections::BTreeSet;

impl CellStore {
    /// World envelope for one cell overlay/generation chunk.
    pub(crate) fn chunk_bounds(chunk: (i32, i32)) -> CellBounds {
        chunk_bounds(chunk)
    }
}

// [`CellStore::chunk_bounds`], for the store's generic methods.
fn chunk_bounds(chunk: (i32, i32)) -> CellBounds {
    let size = f64::from(RegionGraph::CHUNK_SIZE);
    let min = DVec2::new(f64::from(chunk.0), f64::from(chunk.1)) * size;
    CellBounds {
        min,
        max: min + DVec2::splat(size),
    }
}

impl<M: CellMaps> CellStore<M> {
    /// Generates at load every cold chunk within reach of a road that grows cells or holding
    /// reserved cells, in chunk order, and completes it (`SIM-02-K`). Saves leave generated
    /// cells out; without this the first lot region served next to a chunk generates it whole,
    /// 25–96 ms a region at 100k residents. A chunk whose cells would need a grid frame the
    /// store has not registered stays cold ([`Self::generate_on_known_frames`]). Returns the
    /// chunks generated and those left cold. O(edges) plus each chunk's generation.
    pub(crate) fn warm_cold_chunks(
        &mut self,
        graph: &RegionGraph,
        config: &WorldConfig,
        blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
    ) -> (usize, usize) {
        let cell_m = f64::from(config.zone_cell_m);
        // A road's candidates lie within its half-width, the sidewalk and its rear rows.
        let mut chunks = BTreeSet::new();
        for (id, edge) in graph.edges().iter().enumerate() {
            if !edge.zones_any_side()
                || edge.primary_type != TransitType::Road
                || !graph.fronts_ground(id)
                || edge.physical_geometry.len() < 2
            {
                continue;
            }
            let reach = f64::from(edge.width) * 0.5
                + f64::from(crate::config::SIDEWALK_WIDTH)
                + f64::from(edge.kerb_shift_m().abs())
                + (CELL_DEPTH + 2) as f64 * cell_m;
            let bounds = CellBounds::from_points(
                edge.physical_geometry
                    .iter()
                    .map(|point| DVec2::new(f64::from(point.x), f64::from(point.z))),
            )
            .expanded(reach);
            chunks.extend(chunks_for_bounds(bounds));
        }
        chunks.extend(
            self.chunks
                .iter()
                .filter(|(_, entry)| !entry.blocks.is_empty())
                .map(|(&chunk, _)| chunk),
        );
        let [mut warmed, mut cold] = [0; 2];
        for chunk in chunks {
            let Some(bounds) = self.chunk_generation_bounds(chunk) else {
                continue;
            };
            if self
                .generate_on_known_frames(graph, config, bounds, &blocked)
                .is_some()
            {
                self.complete_generated_chunk(chunk, bounds);
                warmed += 1;
            } else {
                cold += 1;
            }
        }
        (warmed, cold)
    }

    /// Returns the packed overlay version and whether all empty geometry is current.
    pub(crate) fn chunk_state(&self, chunk: (i32, i32)) -> (u64, bool) {
        self.chunks
            .get(&chunk)
            .map_or((0, false), |entry| (entry.revision, entry.generated))
    }

    /// Returns a bounded batch of nearby dirty envelopes, or the full chunk for a cold cache.
    /// Warm batches never exceed one chunk's width/height. Ready lookup uses one hash query;
    /// dirty lookup uses at most nine more, with no allocation or city-wide traversal.
    pub(crate) fn chunk_generation_bounds(&self, chunk: (i32, i32)) -> Option<CellBounds> {
        let Some(entry) = self.chunks.get(&chunk) else {
            return Some(chunk_bounds(chunk));
        };
        if entry.generated {
            return None;
        }
        let Some(local) = entry.dirty_bounds else {
            return Some(chunk_bounds(chunk));
        };
        let mut joined = local;
        for x in chunk.0.saturating_sub(1)..=chunk.0.saturating_add(1) {
            for y in chunk.1.saturating_sub(1)..=chunk.1.saturating_add(1) {
                if let Some(dirty) = self
                    .chunks
                    .get(&(x, y))
                    .and_then(|entry| entry.dirty_bounds)
                {
                    joined.min = joined.min.min(dirty.min);
                    joined.max = joined.max.max(dirty.max);
                }
            }
        }
        Some(
            if (joined.max - joined.min).max_element() <= f64::from(RegionGraph::CHUNK_SIZE) {
                joined
            } else {
                local
            },
        )
    }

    /// Completes the requested chunk and warm neighbours whose entire dirty envelope was
    /// covered by the caller's generation bounds. Partly covered and cold neighbours stay dirty.
    pub(crate) fn complete_generated_chunk(&mut self, chunk: (i32, i32), bounds: CellBounds) {
        let entry = self.chunks.entry_or_default(chunk);
        entry.generated = true;
        entry.dirty_bounds = None;
        self.journal.chunks.push(chunk);
        for neighbour in chunks_for_bounds(bounds) {
            if let Some(entry) = self.chunks.get_mut_if(&neighbour, |entry| {
                entry.dirty_bounds.is_some_and(|dirty| {
                    dirty.min.cmpge(bounds.min).all() && dirty.max.cmple(bounds.max).all()
                })
            }) {
                entry.generated = true;
                entry.dirty_bounds = None;
                self.journal.chunks.push(neighbour);
            }
        }
    }

    /// Invalidates cached chunks within an edit's geometric influence, including paint that
    /// changes retained strip alignment. Warm chunks retain only their affected envelope;
    /// overlapping edits union in O(touched chunks) with constant storage per chunk.
    pub(crate) fn invalidate_generated_cells(&mut self, bounds: CellBounds) {
        if bounds.is_valid() {
            for chunk in chunks_for_bounds(bounds) {
                if let Some(entry) = self.chunks.get_mut(&chunk) {
                    self.journal.chunks.push(chunk);
                    let chunk_bounds = chunk_bounds(chunk);
                    let dirty = CellBounds {
                        min: bounds.min.max(chunk_bounds.min),
                        max: bounds.max.min(chunk_bounds.max),
                    };
                    if entry.generated {
                        entry.dirty_bounds = Some(dirty);
                        self.stale_chunks.push(chunk);
                    } else if let Some(previous) = &mut entry.dirty_bounds {
                        previous.min = previous.min.min(dirty.min);
                        previous.max = previous.max.max(dirty.max);
                    }
                    entry.generated = false;
                }
            }
        }
    }

    /// Full-world authoring/load operations may invalidate every viewed chunk explicitly.
    pub(crate) fn invalidate_all_generated_cells(&mut self) {
        self.chunks.for_each_mut(|entry| {
            entry.generated = false;
            entry.dirty_bounds = None;
        });
        // Every entry changed: the view's copy is rebuilt whole.
        self.journal.whole = true;
        // Whole-world rebuilds stay lazy: only chunks the overlay requests are prepared.
        self.stale_chunks.clear();
    }

    /// Drains warm chunks invalidated since the last call, sorted and unique. Each was complete
    /// before a local edit; cold and whole-world invalidations are never listed.
    pub(crate) fn take_stale_chunks(&mut self) -> Vec<(i32, i32)> {
        let mut chunks = std::mem::take(&mut self.stale_chunks);
        chunks.sort_unstable();
        chunks.dedup();
        chunks
    }

    /// Constant-time check before enumerating local road geometry for cache invalidation.
    pub(crate) fn has_cached_chunks(&self) -> bool {
        !self.chunks.is_empty()
    }

    /// Visits centre-owned cells for ownership regression checks.
    #[cfg(test)]
    pub(crate) fn visit_chunk_cells(&self, chunk: (i32, i32), mut visit: impl FnMut(CellKey)) {
        self.visit_in_bounds(chunk_bounds(chunk), |key| {
            if self.cell_chunk(key) == Some(chunk) {
                visit(key);
            }
        });
    }

    #[cfg(test)]
    fn cell_chunk(&self, key: CellKey) -> Option<(i32, i32)> {
        let frame = self.frame(key.grid)?;
        Some(chunk_at(
            frame.world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5),
        ))
    }

    /// Stamps every ground-mask chunk intersecting a changed cell, after its authority epoch
    /// advances. Uses the existing chunk index; O(intersected chunks), with no allocation.
    pub(super) fn mark_cell_chunk_changed(&mut self, key: CellKey) {
        let Some(frame) = self.frame(key.grid) else {
            return;
        };
        for chunk in chunks_for_bounds(CellBounds::from_points(frame.corners(key.x, key.y))) {
            self.chunks.entry_or_default(chunk).revision = self.revision;
            self.journal.chunks.push(chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::zoning::cells::{CellSelection, GridFrame};

    #[test]
    fn paint_stamps_all_crossed_mask_chunks_without_touching_remote_chunks() {
        let mut cells = CellStore::default();
        let grid = cells.register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
        for x in [51, -52] {
            let key = CellKey { grid, x, y: 51 };
            assert!(cells.insert(key));
            let chunks = if x > 0 {
                [(0, 0), (1, 0), (0, 1), (1, 1)]
            } else {
                [(-2, 0), (-1, 0), (-2, 1), (-1, 1)]
            };
            let before = chunks.map(|chunk| cells.chunk_state(chunk).0);
            let remote = cells.chunk_state((8, 8));
            let selection = CellSelection {
                cells: vec![key],
                revision: cells.revision(),
            };
            cells.paint(&selection, 1).unwrap();
            for (chunk, revision) in chunks.into_iter().zip(before) {
                assert!(cells.chunk_state(chunk).0 > revision, "{chunk:?}");
            }
            assert_eq!(cells.chunk_state((8, 8)), remote);
        }
    }
}
