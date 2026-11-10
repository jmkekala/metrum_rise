// SPDX-License-Identifier: GPL-2.0-only

//! The land the road tool's hover preview takes (`ZONE-09`, round RZ3): the parcels with their
//! occupants, the explicit sites' lots, the field and extraction reservations, published by the
//! simulation thread when one of them changed, so the preview judges a stroke's pieces with the
//! commit's own rules ([`RoadLandTake::of`]) without the core.
//!
//! Parcels are held per 512 m parcel chunk and follow the parcel store's chunk revisions: a
//! publication rebuilds only the chunks whose revision moved and shares the others. Occupants are
//! named by their stable building handles, which a swap-remove does not rename. The explicit sites
//! are the allocator's snapshot, rebuilt when the allocator's sites differ from it; the field and
//! extraction indices are the allocator's own `Arc`s.

use std::collections::HashMap;
use std::sync::Arc;

use glam::DVec2;
use godot::prelude::Vector2;

use super::road_edit_plan::{RoadLandSource, RoadLandTake};
use super::state::SimCore;
use crate::simulation::agriculture::FieldClearanceIndex;
use crate::simulation::buildings::allocator::ExplicitSiteSnapshot;
use crate::simulation::network::roadbed::PieceMesh;
use crate::simulation::zoning::cells::{CellStore, SharedMaps};
use crate::simulation::zoning::parcels::{ZoningParcel, chunk_key};

/// One parcel as the road tool's land holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LandParcel {
    id: u64,
    corners: [Vector2; 4],
    aabb_min: Vector2,
    aabb_max: Vector2,
    // An authored parcel (`ZONE-01`): one without a cell lot.
    authored: bool,
    // The occupant's building handle bits.
    occupant: Option<u64>,
}

/// The parcels one parcel chunk lists, ascending by id, at the chunk's revision.
#[derive(Debug, Default)]
struct LandChunk {
    revision: u64,
    parcels: Vec<LandParcel>,
}

impl LandChunk {
    fn parcel(&self, id: u64) -> Option<&LandParcel> {
        let index = self
            .parcels
            .binary_search_by_key(&id, |parcel| parcel.id)
            .ok()?;
        Some(&self.parcels[index])
    }
}

/// The land a road-tool preview takes, as the simulation last published it.
#[derive(Clone, Debug, Default)]
pub(crate) struct RoadToolLand {
    // The publication's order: a slot keeps the newer of two lands published out of order.
    sequence: u64,
    chunks: HashMap<(i32, i32), Arc<LandChunk>>,
    authored: bool,
    fields: Arc<FieldClearanceIndex>,
    extraction: Arc<FieldClearanceIndex>,
    sites: Arc<ExplicitSiteSnapshot>,
    zone_cell_m: f32,
}

impl RoadToolLand {
    /// The publication's sequence number; a later publication's is greater.
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The corners of the parcel `lot`, found by `at`, a point inside it.
    pub(crate) fn lot_corners(&self, lot: u64, at: DVec2) -> Option<[Vector2; 4]> {
        self.parcel(lot, at).map(|parcel| parcel.corners)
    }

    fn parcel(&self, lot: u64, at: DVec2) -> Option<&LandParcel> {
        let chunk = chunk_key(Vector2::new(at.x as f32, at.y as f32));
        self.chunks.get(&chunk)?.parcel(lot)
    }
}

/// The road tool's land with the view's cells: the preview's [`RoadLandSource`].
pub(crate) struct ViewLand<'a> {
    /// The published land.
    pub(crate) land: &'a RoadToolLand,
    /// The view's copy of the cell store.
    pub(crate) cells: &'a CellStore<SharedMaps>,
}

impl ViewLand<'_> {
    /// The land the view's pieces take: [`RoadLandTake::of`] on this source. Buildings are
    /// named by their handles.
    pub(crate) fn take(&self, pieces: &[&PieceMesh], before: &[&PieceMesh]) -> RoadLandTake<u64> {
        RoadLandTake::of(self, pieces, before, &[])
    }
}

impl RoadLandSource for ViewLand<'_> {
    type Maps = SharedMaps;
    type Building = u64;

    fn fields(&self) -> &FieldClearanceIndex {
        &self.land.fields
    }

    fn extraction(&self) -> &FieldClearanceIndex {
        &self.land.extraction
    }

    fn cells(&self) -> &CellStore<SharedMaps> {
        self.cells
    }

    fn first_explicit_site_lot(
        &self,
        min: Vector2,
        max: Vector2,
        test: impl FnMut(&[Vector2; 4]) -> bool,
    ) -> Option<usize> {
        self.land
            .sites
            .first_lot(min, max, self.land.zone_cell_m, test)
    }

    fn visit_authored_parcels(
        &self,
        min: Vector2,
        max: Vector2,
        mut visit: impl FnMut(u64, &[Vector2; 4]),
    ) {
        if !self.land.authored {
            return;
        }
        // A parcel listed in several chunks is visited once per chunk; the take deduplicates.
        for chunk in crate::simulation::zoning::parcels::chunks_for_aabb(min, max) {
            let Some(chunk) = self.land.chunks.get(&chunk) else {
                continue;
            };
            for parcel in chunk.parcels.iter().filter(|parcel| parcel.authored) {
                let (pmin, pmax) = (parcel.aabb_min, parcel.aabb_max);
                if pmin.x > max.x || pmin.y > max.y || pmax.x < min.x || pmax.y < min.y {
                    continue;
                }
                visit(parcel.id, &parcel.corners);
            }
        }
    }

    fn occupant(&self, lot: u64, at: DVec2) -> Option<u64> {
        self.land.parcel(lot, at)?.occupant
    }
}

/// What the simulation last published of the land, and at which revisions.
#[derive(Default)]
pub(crate) struct RoadToolLandPublisher {
    land: Arc<RoadToolLand>,
    parcel_revision: u64,
    world_epoch: u64,
    building_revision: u64,
    // False until the first publication, which takes everything.
    published: bool,
}

impl SimCore {
    /// The road tool's land when something in it changed since the last call, else `None`.
    /// Called at the end of every simulation frame; edits between frames are taken at the next.
    /// Unchanged: O(1). Otherwise O(parcel chunks) to find the changed ones plus O(parcels in
    /// the changed chunks), O(explicit sites) when a building changed, and the `Arc` shares;
    /// never a prepare of the site query index (its rebuild order is the simulation's), so the
    /// sites wait for a frame that leaves it ready.
    pub(crate) fn refresh_road_tool_land(&mut self) -> Option<Arc<RoadToolLand>> {
        let state = &self.road_tool_publisher.land;
        let parcels = &self.zoning.parcels;
        let allocator = &self.allocator;
        let new_world = !state.published || state.world_epoch != self.world_epoch;
        let parcels_changed = new_world || state.parcel_revision != parcels.overlay_revision();
        let fields_changed = !Arc::ptr_eq(&state.land.fields, &allocator.field_clearance);
        let extraction_changed =
            !Arc::ptr_eq(&state.land.extraction, &allocator.extraction_clearance);
        let sites_checked = (new_world
            || state.building_revision != allocator.building_ref_revision())
            && allocator.building_site_query_index_ready();
        let sites_changed =
            sites_checked && !allocator.explicit_site_snapshot_is_current(&state.land.sites);
        let zone_cell_m = self.config.zone_cell_m;
        if !(parcels_changed
            || fields_changed
            || extraction_changed
            || sites_changed
            || state.land.zone_cell_m != zone_cell_m)
        {
            if sites_checked {
                self.road_tool_publisher.land.building_revision = allocator.building_ref_revision();
            }
            return None;
        }
        let mut land = RoadToolLand::clone(&state.land);
        land.sequence += 1;
        land.zone_cell_m = zone_cell_m;
        land.fields = Arc::clone(&allocator.field_clearance);
        land.extraction = Arc::clone(&allocator.extraction_clearance);
        if sites_changed {
            land.sites = Arc::new(allocator.explicit_site_snapshot());
        }
        if parcels_changed {
            if new_world {
                land.chunks.clear();
            }
            // Chunks the store no longer lists (a cleared store) have no revision.
            land.chunks
                .retain(|&chunk, _| parcels.chunk_revision(chunk) != 0);
            let handles = &allocator.building_handles;
            for (chunk, revision) in parcels.chunk_revisions() {
                if land.chunks.get(&chunk).map(|held| held.revision) == Some(revision) {
                    continue;
                }
                let mut held: Vec<_> = parcels
                    .chunk_parcels(chunk)
                    .map(|parcel| land_parcel(parcel, handles))
                    .collect();
                held.sort_unstable_by_key(|parcel| parcel.id);
                land.chunks.insert(
                    chunk,
                    Arc::new(LandChunk {
                        revision,
                        parcels: held,
                    }),
                );
            }
            land.authored = parcels.has_authored_parcels();
        }
        let state = &mut self.road_tool_publisher.land;
        state.parcel_revision = parcels.overlay_revision();
        state.world_epoch = self.world_epoch;
        if sites_checked {
            state.building_revision = allocator.building_ref_revision();
        }
        state.published = true;
        state.land = Arc::new(land);
        Some(Arc::clone(&state.land))
    }
}

#[cfg(test)]
impl RoadToolLand {
    /// How many of this land's parcel chunks are `other`'s own (shared) and how many differ.
    pub(crate) fn shares_chunks_with(&self, other: &Self) -> (usize, usize) {
        self.chunks
            .iter()
            .fold((0, 0), |(shared, rebuilt), (key, chunk)| {
                if other
                    .chunks
                    .get(key)
                    .is_some_and(|held| Arc::ptr_eq(held, chunk))
                {
                    (shared + 1, rebuilt)
                } else {
                    (shared, rebuilt + 1)
                }
            })
    }
}

#[cfg(test)]
impl SimCore {
    /// The road tool's land as of now: refreshed, then the last published.
    pub(crate) fn current_road_tool_land(&mut self) -> Arc<RoadToolLand> {
        self.refresh_road_tool_land();
        Arc::clone(&self.road_tool_publisher.land.land)
    }
}

// One parcel as the land holds it; the occupant by its stable handle. A building without a
// handle yet (none is published so) keeps a name of its own: its index with the top bit set.
fn land_parcel(
    parcel: &ZoningParcel,
    handles: &crate::simulation::core::handles::HandleTable<
        crate::simulation::buildings::allocator::Building,
    >,
) -> LandParcel {
    LandParcel {
        id: parcel.id().raw(),
        corners: parcel.corners(),
        aabb_min: parcel.aabb_min(),
        aabb_max: parcel.aabb_max(),
        authored: parcel.cell_lot().is_none(),
        occupant: parcel.occupied_building().map(|index| {
            let handle = handles.handle(index);
            if handle.is_none() {
                (1 << 63) | index as u64
            } else {
                handle.to_bits()
            }
        }),
    }
}
