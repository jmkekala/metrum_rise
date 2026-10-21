// SPDX-License-Identifier: GPL-2.0-only

//! Event-driven lot work grouped by the existing 512 m world chunk convention, queued for the
//! `cell lots` scheduler system (`SIM-02-H`, `docs/simulation.md#request-queues`).

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::cells::{CELL_GROUP_COLUMNS, CellBounds, CellKey, CellLotSize};
use crate::simulation::zoning::{MAX_PARCEL_FRONTAGE_M, cells::CELL_DEPTH};
use glam::DVec2;
use std::collections::{BTreeSet, HashSet, VecDeque};

/// Pending dirty lot regions, one per 512 m world chunk, served oldest first.
///
/// A chunk marked again while pending keeps its place. The chunks marked between two services
/// form one batch, which the next service queues in coordinate order, so the order never
/// depends on the iteration order of the store that marked them. Painted chunks a catalog
/// change must re-derive wait in a separate refresh set that is served only when nothing else
/// is queued: the refresh a load runs never delays the regions its save had queued.
#[derive(Clone, Default)]
pub(crate) struct CellLotQueue {
    // Every chunk in `queue` or `batch`.
    pending: HashSet<(i32, i32)>,
    queue: VecDeque<(i32, i32)>,
    batch: Vec<(i32, i32)>,
    // Not saved: every load refreshes the catalog again.
    refresh: BTreeSet<(i32, i32)>,
}

impl CellLotQueue {
    fn mark(&mut self, bounds: CellBounds) {
        for_each_chunk(bounds, |chunk| {
            if self.pending.insert(chunk) {
                self.batch.push(chunk);
            }
        });
    }

    fn mark_refresh(&mut self, bounds: CellBounds) {
        for_each_chunk(bounds, |chunk| {
            self.refresh.insert(chunk);
        });
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.refresh.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.queue.clear();
        self.batch.clear();
        self.refresh.clear();
    }

    // The oldest queued chunk, else the first chunk awaiting a catalog refresh. Amortized O(1)
    // plus sorting the chunks marked since the last call; buffers keep their capacity.
    fn pop(&mut self) -> Option<(i32, i32)> {
        self.batch.sort_unstable();
        self.queue.extend(self.batch.drain(..));
        if let Some(chunk) = self.queue.pop_front() {
            self.pending.remove(&chunk);
            // Derived with the current catalog, so a pending refresh of it is redundant.
            self.refresh.remove(&chunk);
            return Some(chunk);
        }
        self.refresh.pop_first()
    }

    // Every pending chunk once, in coordinate order.
    fn drain_sorted(&mut self) -> Vec<(i32, i32)> {
        let mut chunks: Vec<_> = self
            .queue
            .drain(..)
            .chain(self.batch.drain(..))
            .chain(std::mem::take(&mut self.refresh))
            .collect();
        chunks.sort_unstable();
        chunks.dedup();
        self.pending.clear();
        chunks
    }
}

impl ZoningSystem {
    /// External geometry/eligibility epoch used to reject a stale cell-tool preview.
    pub(crate) fn cell_external_revision(&self) -> u64 {
        self.cell_external_revision
    }

    /// The asset sizes every lot was derived with, for saving: `None` before the first
    /// derivation and while a catalog refresh is still re-deriving painted chunks.
    pub(crate) fn saved_cell_lot_sizes(&self) -> Option<&[CellLotSize]> {
        if !self.cell_lot_queue.refresh.is_empty() {
            return None;
        }
        self.cell_lot_sizes.as_deref()
    }

    /// Checks the sizes a loaded save derived its lots with (left in `cell_lot_sizes` by the
    /// loader) against the live catalog's `live` sizes. Equal sizes derive the same lots, so
    /// only the generated cells, which saves leave out, are invalidated; otherwise every painted
    /// chunk is re-derived as after a catalog refresh. O(sizes).
    pub(crate) fn restore_cell_lot_assets(&mut self, live: Vec<CellLotSize>) {
        if self.cell_lot_sizes.as_ref() == Some(&live) {
            self.cells.invalidate_all_generated_cells();
            self.cell_lot_sizes = Some(live);
        } else {
            self.invalidate_cell_lot_assets();
        }
    }

    /// Invalidates compatible asset sizes and schedules all painted tiles after a catalog refresh.
    /// They are served after every other queued region, or by the next full drain.
    pub(crate) fn invalidate_cell_lot_assets(&mut self) {
        self.cell_lot_sizes = None;
        self.cells.invalidate_all_generated_cells();
        let queue = &mut self.cell_lot_queue;
        self.cells
            .visit_painted_block_bounds(|bounds| queue.mark_refresh(bounds));
    }

    /// Marks an externally edited area without scanning background roads, cells or parcels.
    pub(crate) fn mark_cell_lots_dirty(&mut self, bounds: CellBounds) {
        self.cell_external_revision = self.cell_external_revision.wrapping_add(1);
        // A lost column cell affects up to five rear rows beyond its direct conflict halo.
        self.cells.invalidate_generated_cells(
            bounds.expanded((CELL_DEPTH + 1) as f64 * f64::from(self.config.zone_cell_m)),
        );
        self.cell_lot_queue.mark(bounds);
    }

    /// Queues an area for lot derivation alone, keeping its generated cells: for a change that
    /// alters no input of cell generation (roads, paint, lot claims, authored parcels, explicit
    /// sites and fields), such as a building leaving a lot it keeps (`SIM-02-K`). Regenerating
    /// those cells would reproduce them; the region still refreshes the lots' frontage, as
    /// [`Self::mark_cell_lots_dirty`] queues it. O(touched chunks).
    pub(in crate::simulation::zoning::system) fn mark_cell_lots_for_derivation(
        &mut self,
        bounds: CellBounds,
    ) {
        self.cell_external_revision = self.cell_external_revision.wrapping_add(1);
        self.cell_lot_queue.mark(bounds);
    }

    /// Queues the supplied topology edit's road corridors, including retained deleted geometry.
    pub(crate) fn mark_cell_lots_for_roads(
        &mut self,
        graph: &RegionGraph,
        edges: impl IntoIterator<Item = usize>,
    ) {
        // Empty generated grids also become stale when a road changes eligibility.
        self.cell_external_revision = self.cell_external_revision.wrapping_add(1);
        let edges = self
            .cells
            .refresh_road_alignments(graph, &self.config, edges);
        if !self.cells.has_paint() && !self.cells.has_cached_chunks() {
            return;
        }
        for id in edges {
            let Some(edge) = graph.get_edge(id) else {
                continue;
            };
            if edge.physical_geometry.is_empty() {
                continue;
            }
            let margin = f64::from(edge.width) * 0.5
                + f64::from(crate::config::SIDEWALK_WIDTH)
                + f64::from(self.config.zone_cell_m) * CELL_DEPTH as f64
                + f64::from(MAX_PARCEL_FRONTAGE_M);
            let bounds = CellBounds::from_points(
                edge.physical_geometry
                    .iter()
                    .map(|p| DVec2::new(f64::from(p.x), f64::from(p.z))),
            )
            .expanded(margin);
            self.mark_cell_lots_dirty(bounds);
        }
    }

    /// Queues the changed strips and their direct conflict halo after paint or an inverse.
    pub(in crate::simulation::zoning::system) fn mark_cell_keys_dirty(
        &mut self,
        keys: impl Iterator<Item = CellKey>,
    ) {
        for key in keys {
            if let Some(frame) = self.cells.frame(key.grid) {
                let bounds = CellBounds::from_points(frame.corners(key.x, key.y))
                    .expanded((CELL_DEPTH + CELL_GROUP_COLUMNS + 2) as f64 * frame.cell_m());
                self.cells.invalidate_generated_cells(bounds);
                self.cell_lot_queue.mark(bounds);
            }
        }
    }

    /// True only for queued authoring work; idle demand updates do not enumerate cells.
    pub(crate) fn has_dirty_cell_lots(&self) -> bool {
        !self.cell_lot_queue.is_empty()
    }

    /// Drains local work for paint and retained occupied claims, including completely erased lots:
    /// every pending region at once, in coordinate order, for editing paths that must publish
    /// their lots before returning.
    pub(crate) fn take_dirty_cell_lot_regions(&mut self) -> Vec<CellBounds> {
        if !self.cells.has_reservations() {
            self.cell_lot_queue.clear();
            return Vec::new();
        }
        self.cell_lot_queue
            .drain_sorted()
            .into_iter()
            .map(chunk_bounds)
            .collect()
    }

    /// Takes the next region of the `cell lots` queue (`SIM-02-H`): the oldest queued chunk,
    /// chunks marked together in coordinate order, then chunks awaiting a catalog refresh.
    /// Without reservations nothing needs lots and the queue empties. O(1) amortized and
    /// allocation-free once its buffers have grown.
    pub(crate) fn pop_dirty_cell_lot_region(&mut self) -> Option<CellBounds> {
        if !self.cells.has_reservations() {
            self.cell_lot_queue.clear();
            return None;
        }
        self.cell_lot_queue.pop().map(chunk_bounds)
    }

    /// Pending chunks as saved: queued ones in service order (`true`), then the ones marked
    /// since the last service in coordinate order (`false`). Catalog refreshes are not saved.
    pub(crate) fn saved_cell_lot_queue(&self) -> Vec<((i32, i32), bool)> {
        let queue = &self.cell_lot_queue;
        let mut batch = queue.batch.clone();
        batch.sort_unstable();
        queue
            .queue
            .iter()
            .map(|&chunk| (chunk, true))
            .chain(batch.into_iter().map(|chunk| (chunk, false)))
            .collect()
    }

    /// Replaces every pending region with saved entries in saved order; `false` on a chunk
    /// saved twice.
    pub(crate) fn restore_cell_lot_queue(
        &mut self,
        entries: impl IntoIterator<Item = ((i32, i32), bool)>,
    ) -> bool {
        let queue = &mut self.cell_lot_queue;
        queue.clear();
        for (chunk, queued) in entries {
            if !queue.pending.insert(chunk) {
                return false;
            }
            if queued {
                queue.queue.push_back(chunk);
            } else {
                queue.batch.push(chunk);
            }
        }
        true
    }
}

fn chunk_bounds((x, y): (i32, i32)) -> CellBounds {
    let size = f64::from(RegionGraph::CHUNK_SIZE);
    CellBounds {
        min: DVec2::new(f64::from(x), f64::from(y)) * size,
        max: DVec2::new(f64::from(x) + 1.0, f64::from(y) + 1.0) * size,
    }
}

// Visits the 512 m chunks `bounds` touches, in coordinate order.
fn for_each_chunk(bounds: CellBounds, mut visit: impl FnMut((i32, i32))) {
    if !bounds.is_valid() {
        return;
    }
    let size = f64::from(RegionGraph::CHUNK_SIZE);
    let min = (bounds.min / size).floor();
    let max = (bounds.max / size).floor();
    for x in min.x as i32..=max.x as i32 {
        for y in min.y as i32..=max.y as i32 {
            visit((x, y));
        }
    }
}
