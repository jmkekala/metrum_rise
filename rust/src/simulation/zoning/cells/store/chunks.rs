// SPDX-License-Identifier: GPL-2.0-only

//! Generation completeness and paint revisions on the existing cell spatial chunks.

use super::{CellBounds, CellKey, CellStore, DVec2, RegionGraph, chunk_at, chunks_for_bounds};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::zoning::cells::CELL_DEPTH;
use std::collections::BTreeSet;

impl CellStore {
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
        for edge in graph.edges() {
            if edge.deleted
                || edge.no_building_spawn
                || edge.primary_type != TransitType::Road
                || edge.class != EdgeClass::Standard
                || edge.physical_geometry.len() < 2
            {
                continue;
            }
            let reach = f64::from(edge.width) * 0.5
                + f64::from(crate::config::SIDEWALK_WIDTH)
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
            return Some(Self::chunk_bounds(chunk));
        };
        if entry.generated {
            return None;
        }
        let Some(local) = entry.dirty_bounds else {
            return Some(Self::chunk_bounds(chunk));
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
        let entry = self.chunks.entry(chunk).or_default();
        entry.generated = true;
        entry.dirty_bounds = None;
        for neighbour in chunks_for_bounds(bounds) {
            if let Some(entry) = self.chunks.get_mut(&neighbour)
                && let Some(dirty) = entry.dirty_bounds
                && dirty.min.cmpge(bounds.min).all()
                && dirty.max.cmple(bounds.max).all()
            {
                entry.generated = true;
                entry.dirty_bounds = None;
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
                    let chunk_bounds = Self::chunk_bounds(chunk);
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
        for entry in self.chunks.values_mut() {
            entry.generated = false;
            entry.dirty_bounds = None;
        }
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

    /// Assigns each full cell to its centre's chunk so packed overlay uploads contain no duplicates.
    pub(crate) fn visit_chunk_cells(&self, chunk: (i32, i32), mut visit: impl FnMut(CellKey)) {
        self.visit_in_bounds(Self::chunk_bounds(chunk), |key| {
            if self.cell_chunk(key) == Some(chunk) {
                visit(key);
            }
        });
    }

    /// World envelope for one cell overlay/generation chunk.
    pub(crate) fn chunk_bounds(chunk: (i32, i32)) -> CellBounds {
        let size = f64::from(RegionGraph::CHUNK_SIZE);
        let min = DVec2::new(f64::from(chunk.0), f64::from(chunk.1)) * size;
        CellBounds {
            min,
            max: min + DVec2::splat(size),
        }
    }

    fn cell_chunk(&self, key: CellKey) -> Option<(i32, i32)> {
        let frame = self.frame(key.grid)?;
        Some(chunk_at(
            frame.world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5),
        ))
    }

    /// Stamps a changed full cell's single rendering owner after its authority epoch advances.
    pub(super) fn mark_cell_chunk_changed(&mut self, key: CellKey) {
        if let Some(chunk) = self.cell_chunk(key) {
            self.chunks.entry(chunk).or_default().revision = self.revision;
        }
    }
}
