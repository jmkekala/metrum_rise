// SPDX-License-Identifier: GPL-2.0-only

//! Event-driven lot work grouped by the existing 512 m world chunk convention.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::cells::{CELL_GROUP_COLUMNS, CellBounds, CellKey};
use crate::simulation::zoning::{MAX_PARCEL_FRONTAGE_M, cells::CELL_DEPTH};
use glam::DVec2;
use std::collections::HashSet;

impl ZoningSystem {
    /// External geometry/eligibility epoch used to reject a stale cell-tool preview.
    pub(crate) fn cell_external_revision(&self) -> u64 {
        self.cell_external_revision
    }

    /// Invalidates compatible asset sizes and schedules all painted tiles after a catalog refresh.
    pub(crate) fn invalidate_cell_lot_assets(&mut self) {
        self.cell_lot_sizes = None;
        self.cells.invalidate_all_generated_cells();
        let dirty = &mut self.cell_lot_dirty;
        self.cells
            .visit_painted_block_bounds(|bounds| mark(dirty, bounds));
    }

    /// Marks an externally edited area without scanning background roads, cells or parcels.
    pub(crate) fn mark_cell_lots_dirty(&mut self, bounds: CellBounds) {
        self.cell_external_revision = self.cell_external_revision.wrapping_add(1);
        // A lost column cell affects up to five rear rows beyond its direct conflict halo.
        self.cells.invalidate_generated_cells(
            bounds.expanded((CELL_DEPTH + 1) as f64 * f64::from(self.config.zone_cell_m)),
        );
        mark(&mut self.cell_lot_dirty, bounds);
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
                mark(&mut self.cell_lot_dirty, bounds);
            }
        }
    }

    /// True only for queued authoring work; idle demand updates do not enumerate cells.
    pub(crate) fn has_dirty_cell_lots(&self) -> bool {
        !self.cell_lot_dirty.is_empty()
    }

    /// Drains local work for paint and retained occupied claims, including completely erased lots.
    pub(crate) fn take_dirty_cell_lot_regions(&mut self) -> Vec<CellBounds> {
        if !self.cells.has_reservations() {
            self.cell_lot_dirty.clear();
            return Vec::new();
        }
        let mut chunks: Vec<_> = self.cell_lot_dirty.drain().collect();
        chunks.sort_unstable();
        let size = f64::from(RegionGraph::CHUNK_SIZE);
        chunks
            .into_iter()
            .map(|(x, y)| CellBounds {
                min: DVec2::new(f64::from(x), f64::from(y)) * size,
                max: DVec2::new(f64::from(x) + 1.0, f64::from(y) + 1.0) * size,
            })
            .collect()
    }
}

fn mark(dirty: &mut HashSet<(i32, i32)>, bounds: CellBounds) {
    if !bounds.is_valid() {
        return;
    }
    let size = f64::from(RegionGraph::CHUNK_SIZE);
    let min = (bounds.min / size).floor();
    let max = (bounds.max / size).floor();
    for x in min.x as i32..=max.x as i32 {
        for y in min.y as i32..=max.y as i32 {
            dirty.insert((x, y));
        }
    }
}
