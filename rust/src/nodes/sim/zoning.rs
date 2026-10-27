// SPDX-License-Identifier: GPL-2.0-only

//! Cell gestures, local paint inverses and event-driven lot preparation for private demand:
//! a full drain for editing paths and the per-slot `cell lots` queue (`SIM-02-H`).

use super::core::SimCore;
use crate::simulation::agriculture::{FieldClearanceIndex, PolygonFootprint};
use crate::simulation::buildings::allocator::{BuildingAllocator, ExplicitSiteSnapshot};
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::zoning::cells::{
    CellBounds, CellLotSize, CellSelection, CellSelectionShape, CellStore, road_contact_interior,
};
use crate::simulation::zoning::{CellLotGeneration, CellLotPlanner, CellZoningEdit};
use glam::DVec2;

/// One immutable selection plus all authoring dependencies observed by its preview.
pub(crate) struct CellToolPreview {
    /// Exact cell identities shown to the player and later submitted for commit.
    pub(crate) selection: CellSelection,
    /// Epoch tuple preventing a gesture from silently targeting replacement geometry.
    pub(crate) dependencies: [u64; 11],
}

impl SimCore {
    /// Tests the visible chunk against the current world without preparing derived queries.
    pub(crate) fn cell_chunk_in_world(&self, chunk: (i32, i32)) -> bool {
        let bounds = CellStore::chunk_bounds(chunk);
        let half_world = DVec2::new(
            f64::from(self.config.width_m),
            f64::from(self.config.height_m),
        ) * 0.5;
        bounds.is_valid()
            && bounds.min.x <= half_world.x
            && bounds.min.y <= half_world.y
            && bounds.max.x >= -half_world.x
            && bounds.max.y >= -half_world.y
    }

    /// Executes queued derived work against current authority, discarding pre-reset requests.
    pub(crate) fn prepare_requested_cell_chunk(
        &mut self,
        chunk: (i32, i32),
        world_generation: u64,
    ) -> bool {
        self.terrain_payload_global_generation == world_generation
            && self.prepare_cell_chunk_internal(chunk)
    }

    /// Regenerates warm chunks that local edits invalidated while the overlay is shown, so a
    /// published edit already carries its cells; hidden overlays leave them for a later request.
    /// O(edited chunks); no work or allocation without local invalidations.
    pub(crate) fn prepare_stale_cell_chunks_internal(&mut self) {
        // The chunks a commit made stale wait with its deferred lots, which generate them first
        // (`ROAD-68-G` part 7).
        if self.cell_lots_pending() {
            return;
        }
        let chunks = self.zoning.cells.take_stale_chunks();
        if self.cell_overlay_visible {
            for chunk in chunks {
                self.prepare_cell_chunk_internal(chunk);
            }
        }
    }

    /// Ensures one visible chunk is complete, keeping cold/dirty work independent of city size.
    pub(crate) fn prepare_cell_chunk_internal(&mut self, chunk: (i32, i32)) -> bool {
        if !self.cell_chunk_in_world(chunk) {
            return false;
        }
        // A commit's deferred lots first: generation writes the cells they plan.
        self.adopt_pending_cell_lots();
        // Site-index repair and cell generation are parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            self.prepare_cell_site_queries();
            if let Some(bounds) = self.zoning.cells.chunk_generation_bounds(chunk) {
                let allocator = &self.allocator;
                let roads = &self.transit_network.roadbed;
                let cell_m = self.config.zone_cell_m;
                self.zoning
                    .generate_cells(&self.region_graph, bounds, |corners| {
                        cell_site_blocked(allocator, roads, cell_m, corners)
                    });
                self.zoning.cells.complete_generated_chunk(chunk, bounds);
            }
        });
        true
    }

    /// Generates the cells a loaded save left out, near every road, so the first lot regions
    /// served after loading find them warm (`SIM-02-K`). Call once the loaded allocator, road
    /// surface and parcels are in place.
    pub(crate) fn warm_loaded_cells(&mut self) {
        self.prepare_cell_site_queries();
        let allocator = &self.allocator;
        let roads = &self.transit_network.roadbed;
        let cell_m = self.config.zone_cell_m;
        let (warmed, cold) = self
            .zoning
            .warm_generated_cells(&self.region_graph, |corners| {
                cell_site_blocked(allocator, roads, cell_m, corners)
            });
        crate::debug_log!(
            "save",
            "load generated cells in {} chunks; {} left cold for frames not yet registered",
            warmed,
            cold
        );
    }

    /// Prepares shared local queries without modifying paint, terrain or building ownership.
    pub(super) fn prepare_cell_site_queries(&mut self) {
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);
        self.transit_network
            .roadbed
            .compile_dirty(&self.region_graph);
    }

    /// Captures the authoring inputs that can invalidate a retained cell gesture or overlay.
    pub(crate) fn cell_tool_dependencies(&self) -> [u64; 11] {
        [
            self.road_tool_surface_generation,
            self.heightmap.source_generation(),
            self.heightmap.source_generation(),
            self.transit_network.roadbed.generation(),
            self.allocator.building_ref_revision(),
            self.allocator.registry.revision(),
            self.zoning.overlay_revision(),
            self.zoning.overlay_occupancy_revision(),
            self.zoning.cells.revision(),
            self.zoning.cell_external_revision(),
            self.agriculture.visual_revision(),
        ]
    }

    /// Materializes and selects exact gesture cells. A preview never changes their designation.
    pub(crate) fn preview_cell_selection_internal(
        &mut self,
        shape: CellSelectionShape,
        path: &[DVec2],
    ) -> CellToolPreview {
        self.adopt_pending_cell_lots();
        self.prepare_cell_site_queries();
        let allocator = &self.allocator;
        let roads = &self.transit_network.roadbed;
        let cell_m = self.config.zone_cell_m;
        let selection = self
            .zoning
            .select_road_cells(&self.region_graph, shape, path, |corners| {
                cell_site_blocked(allocator, roads, cell_m, corners)
            });
        CellToolPreview {
            selection,
            dependencies: self.cell_tool_dependencies(),
        }
    }

    /// Revalidates and commits precisely the previewed cells, recording one local undo entry.
    pub(crate) fn apply_cell_selection_internal(
        &mut self,
        preview: &CellToolPreview,
        profile: u16,
    ) -> bool {
        self.adopt_pending_cell_lots();
        self.prepare_cell_site_queries();
        // Index 8 is the rebuildable geometry-cache epoch. Loading another visible chunk
        // cannot invalidate a gesture; authoritative road/paint/site/occupancy epochs still do.
        if preview
            .dependencies
            .iter()
            .zip(self.cell_tool_dependencies())
            .enumerate()
            .any(|(index, (&before, now))| index != 8 && before != now)
        {
            return false;
        }
        let allocator = &self.allocator;
        let roads = &self.transit_network.roadbed;
        let cell_m = self.config.zone_cell_m;
        let selection = CellSelection {
            revision: self.zoning.cells.revision(),
            cells: preview.selection.cells.clone(),
        };
        let Some(edit) = self.zoning.paint_cells(&selection, profile, |corners| {
            cell_site_blocked(allocator, roads, cell_m, corners)
        }) else {
            return false;
        };
        if edit.paint.previous.is_empty() {
            return false;
        }
        // Pending spawns resolve their current parcel/profile/site when dequeued. Removed
        // empty lots never reuse their ids, so editing needs no scan of the city's queue.
        self.allocator.dirty = true;
        self.push_cell_zoning_undo(edit);
        true
    }

    /// Restores paint against present reservations; growth and occupied buildings keep running.
    pub(crate) fn restore_cell_gesture_internal(&mut self, edit: &CellZoningEdit) -> bool {
        self.adopt_pending_cell_lots();
        self.prepare_cell_site_queries();
        let allocator = &self.allocator;
        let roads = &self.transit_network.roadbed;
        let cell_m = self.config.zone_cell_m;
        if self
            .zoning
            .restore_cell_gesture(edit, |corners| {
                cell_site_blocked(allocator, roads, cell_m, corners)
            })
            .is_none()
        {
            return false;
        }
        self.allocator.dirty = true;
        true
    }

    /// Rebuilds every queued painted neighborhood at once and installs asset-sized lots for
    /// demand, for editing paths that must publish their lots before returning. With no
    /// authoring/catalog changes this is an allocation-free constant-time check.
    pub(crate) fn prepare_cell_lots_internal(&mut self) -> CellLotGeneration {
        self.adopt_pending_cell_lots();
        if self.transit_network.road_edit_is_staged() || !self.zoning.has_dirty_cell_lots() {
            return CellLotGeneration::default();
        }
        let mut regions = self.zoning.take_dirty_cell_lot_regions().into_iter();
        self.regenerate_cell_lot_regions(|_| regions.next())
    }

    /// One slot of the `cell lots` queue (`SIM-02-H`): rebuilds up to `quota` pending regions,
    /// oldest first, and none while a road edit is staged. O(1) and allocation-free with
    /// nothing queued; otherwise O(region) per region served, plus repairing site indices an
    /// earlier system in the slot left stale.
    pub(crate) fn serve_cell_lot_queue(&mut self, quota: usize) -> CellLotGeneration {
        if self.transit_network.road_edit_is_staged() || !self.zoning.has_dirty_cell_lots() {
            return CellLotGeneration::default();
        }
        let mut left = quota;
        self.regenerate_cell_lot_regions(|planner| {
            left = left.checked_sub(1)?;
            planner.pop_dirty_cell_lot_region()
        })
    }

    // Rebuilds the regions `next` takes from the lot queue with one query preparation and the
    // cached asset sizes, as one lot plan on the live cell store installed at once, then
    // publishes reattached occupants once.
    fn regenerate_cell_lot_regions(
        &mut self,
        mut next: impl FnMut(&mut CellLotPlanner) -> Option<CellBounds>,
    ) -> CellLotGeneration {
        // Taking the first region changes only the queue (or empties it), so this plan has
        // nothing to install.
        let Some(first) = next(&mut self.zoning.cell_lot_planner()) else {
            return CellLotGeneration::default();
        };
        #[cfg(test)]
        let _contract_phase = crate::debug::work::phases::enter("lots");
        self.prepare_cell_site_queries();
        // Ownership of this event cache is moved out while the zoning authority is mutated;
        // neither the asset catalog nor its shape list is cloned for each dirty region.
        let sizes = self.zoning.cell_lot_sizes.take().unwrap_or_else(|| {
            self.allocator
                .cell_lot_sizes(&self.zoning, self.demand.runtime_catalog())
        });
        let mut result = CellLotGeneration::default();
        let allocator = &self.allocator;
        let roads = &self.transit_network.roadbed;
        let cell_m = self.config.zone_cell_m;
        let blocked = |corners: &[DVec2; 4]| cell_site_blocked(allocator, roads, cell_m, corners);
        let mut planner = self.zoning.cell_lot_planner();
        let mut region = Some(first);
        while let Some(bounds) = region {
            result.absorb(planner.plan_region(&self.region_graph, bounds, &sizes, &blocked));
            region = next(&mut planner);
        }
        let plan = planner.finish();
        self.zoning.install_cell_lot_plan(plan);
        self.zoning.cell_lot_sizes = Some(sizes);
        self.attach_regenerated_cell_lots(&result);
        result
    }

    // Reattaches the occupants of the lots a plan reattached and marks the allocator dirty when
    // the plan created or retired lots: the end of every installed lot plan.
    pub(super) fn attach_regenerated_cell_lots(&mut self, result: &CellLotGeneration) {
        self.allocator.sync_cell_lot_attachments(
            &result.reattached,
            &self.region_graph,
            &mut self.zoning,
        );
        if !result.created.is_empty() || !result.retired.is_empty() {
            self.allocator.dirty = true;
        }
    }

    /// The asset sizes a lot plan derives with, cached until the asset registry refreshes them.
    pub(super) fn cached_cell_lot_sizes(&mut self) -> &[CellLotSize] {
        if self.zoning.cell_lot_sizes.is_none() {
            let sizes = self
                .allocator
                .cell_lot_sizes(&self.zoning, self.demand.runtime_catalog());
            self.zoning.cell_lot_sizes = Some(sizes);
        }
        self.zoning.cell_lot_sizes.as_deref().unwrap_or_default()
    }
}

// The explicit cell sites a footprint test reads: the allocator's own, or the snapshot a
// deferred lot plan took of them (`ROAD-68-G` part 7).
pub(super) trait ExplicitCellSites: Sync {
    fn any(&self) -> bool;
    fn overlaps(&self, footprint: &PolygonFootprint, cell_m: f32) -> bool;
}

impl ExplicitCellSites for BuildingAllocator {
    fn any(&self) -> bool {
        self.has_explicit_cell_sites()
    }

    fn overlaps(&self, footprint: &PolygonFootprint, cell_m: f32) -> bool {
        self.cell_footprint_overlaps_explicit_site(footprint, cell_m)
    }
}

impl ExplicitCellSites for ExplicitSiteSnapshot {
    fn any(&self) -> bool {
        ExplicitSiteSnapshot::any(self)
    }

    fn overlaps(&self, footprint: &PolygonFootprint, cell_m: f32) -> bool {
        self.cell_footprint_overlaps(footprint, cell_m)
    }
}

fn cell_site_blocked(
    allocator: &BuildingAllocator,
    roads: &RoadbedSystem,
    cell_m: f32,
    corners: &[DVec2; 4],
) -> bool {
    footprint_blocked(&allocator.field_clearance, allocator, roads, cell_m, corners)
}

// Whether a cell footprint meets a field, an explicit cell site or a road.
pub(super) fn footprint_blocked(
    fields: &FieldClearanceIndex,
    sites: &impl ExplicitCellSites,
    roads: &RoadbedSystem,
    cell_m: f32,
    corners: &[DVec2; 4],
) -> bool {
    // Keep the same canonical corners used by reciprocal placement and save validation.
    // Narrowing to f32 here can hide small intersections at large world coordinates.
    if !fields.is_empty() || sites.any() {
        let points = corners.map(|p| [p.x, p.y]);
        let footprint = PolygonFootprint::from_precise_slice(&points);
        if fields.overlaps(&footprint, None) || sites.overlaps(&footprint, cell_m) {
            return true;
        }
    }
    let points = road_contact_interior(corners).map(|p| [p.x, p.y]);
    PolygonFootprint::from_precise_slice(&points).overlaps_roads(roads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::zoning::cells::GridFrame;
    use godot::prelude::Vector2;

    #[test]
    fn field_blocking_retains_canonical_cell_precision() {
        let mut allocator = BuildingAllocator::new();
        allocator.field_clearance_mut().set(
            0,
            &[
                Vector2::new(100.0, 10.0),
                Vector2::new(120.0, 10.0),
                Vector2::new(120.0, 30.0),
                Vector2::new(100.0, 30.0),
            ],
        );
        let roads = RoadbedSystem::new(512.0);
        for (offset, blocked) in [(-0.000001, false), (0.0, false), (0.000001, true)] {
            let frame = GridFrame::new(DVec2::new(offset, 0.0), DVec2::X, 10.0).unwrap();
            let key = frame.local(DVec2::new(95.0, 15.0)).floor();
            let corners = frame.corners(key.x as i32, key.y as i32);
            assert_eq!(corners[1].x as f32, 100.0);
            assert_eq!(
                cell_site_blocked(&allocator, &roads, 10.0, &corners),
                blocked
            );
        }
    }
}
