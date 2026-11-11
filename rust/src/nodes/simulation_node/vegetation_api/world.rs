// SPDX-License-Identifier: GPL-2.0-only

//! What the vegetation queries read, live or from a snapshot (`RENDER-32-D`).
//!
//! The scatter, the stand cover and the hedge runs read the world through [`VegetationWorld`]:
//! the simulation core itself on the simulation's own paths (edits, picking, tests), or a
//! [`VegetationPatchInputs`] snapshot that a background worker queries off the simulation lock.
//! The snapshot is the graded bakes' kind (`GradedInputs` in `graded_tiles_api.rs`): the
//! heightmap, roadbed, road mesh chunks and water depth shared in `O(1)`, and for the patch's
//! window only the building sites, the terrain query index entries, the committed fields and
//! the vegetation edits copied. Inside its window every answer equals the core's bitwise, so a
//! patch built from a snapshot equals the live patch.

use super::*;
use crate::simulation::agriculture::FieldClearanceIndex;
use crate::simulation::buildings::allocator::SiteSnapshot;
use crate::simulation::network::render::RoadChunkMap;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch, TerrainQueryIndex};
use crate::simulation::vegetation::VegetationGenerator;
use crate::simulation::vegetation::edits::VegetationEdits;
use crate::simulation::water::VisibleDepth;
use std::cell::RefCell;

/// The reads of the vegetation queries ([`decorative_tree_patch`], the stand cover, the hedge
/// runs and the placement rules under them).
pub(crate) trait VegetationWorld: Sync {
    /// The saved generator.
    fn generator(&self) -> &VegetationGenerator;
    /// The player's edits.
    fn edits(&self) -> &VegetationEdits;
    /// The heightmap's world size, metres.
    fn world_size(&self) -> (f32, f32);
    /// The compiled roadbed.
    fn roadbed(&self) -> &RoadbedSystem;
    /// The road mesh chunks, whose street lights set trees back.
    fn road_chunks(&self) -> &RoadChunkMap;
    /// Runs `query` on the graded surface with this thread's scratch.
    fn with_surface<T>(&self, query: impl FnOnce(&GradedSurface<'_>, &mut QueryScratch) -> T) -> T;
    /// The visible surface height (`SimCore::get_world_surface_height_internal`).
    fn surface_height(&self, pos: Vector2) -> f32 {
        self.with_surface(|surface, scratch| {
            surface.surface_height_at(f64::from(pos.x), f64::from(pos.y), scratch) as f32
        })
    }
    /// Authored visible water depth.
    fn water_depth(&self, pos: Vector2) -> f32;
    /// Whether a committed field covers the point.
    fn field_covers(&self, pos: Vector2) -> bool;
    /// Whether a building site stops an authored plant at the point.
    fn blocks_yard_plant(&self, pos: Vector2) -> bool;
    /// Whether the point lies in a building's yard planting area.
    fn in_yard_planting(&self, pos: Vector2) -> bool;
    /// Whether the queries split their loops over the Rayon pool: the core's callers wait for
    /// the answer (Godot's main thread, the simulation's own edits); a snapshot's background job
    /// runs them sequentially, so a capped number of jobs bounds the threads the vegetation takes
    /// beside the frame. The answers are the same either way.
    fn parallel(&self) -> bool {
        true
    }
}

impl VegetationWorld for SimCore {
    fn generator(&self) -> &VegetationGenerator {
        &self.vegetation
    }

    fn edits(&self) -> &VegetationEdits {
        &self.vegetation_edits
    }

    fn world_size(&self) -> (f32, f32) {
        self.heightmap.world_size()
    }

    fn roadbed(&self) -> &RoadbedSystem {
        &self.transit_network.roadbed
    }

    fn road_chunks(&self) -> &RoadChunkMap {
        &self.cached_road_mesh_chunks
    }

    fn with_surface<T>(&self, query: impl FnOnce(&GradedSurface<'_>, &mut QueryScratch) -> T) -> T {
        self.with_graded_surface(query)
    }

    fn surface_height(&self, pos: Vector2) -> f32 {
        self.get_world_surface_height_internal(pos)
    }

    fn water_depth(&self, pos: Vector2) -> f32 {
        self.watermap.visible_depth_world(pos.x, pos.y)
    }

    fn field_covers(&self, pos: Vector2) -> bool {
        self.allocator.field_clearance.covers_point(pos)
    }

    fn blocks_yard_plant(&self, pos: Vector2) -> bool {
        self.allocator.building_site_blocks_yard_plant(pos)
    }

    fn in_yard_planting(&self, pos: Vector2) -> bool {
        self.allocator.in_yard_planting(pos)
    }
}

// How far past a patch its queries read, metres: the stand cover's 16 m margin and one 8 m
// texel, the largest crown (4.7 m), one canopy cell, the canopy clearance (6 m) and the lamp
// setback (4 m), rounded up with room to spare.
const QUERY_PAD_M: f32 = 64.0;
// The graded surface's reach past a query point: a whole 64 m tile (a quiet point prepares its
// tile), then the cap, its window pad and a lattice cell.
const TILE_PAD_M: f64 = crate::simulation::terrain::graded::TILE_M;
const SURFACE_PAD_M: f64 = crate::simulation::terrain::graded::CAP_M + 8.0;

thread_local! {
    // One graded query scratch per worker thread, as the core's own.
    static SNAPSHOT_SCRATCH: RefCell<QueryScratch> = RefCell::default();
}

/// What one vegetation patch's queries read (`RENDER-32-D`), taken under `try_lock`
/// ([`Self::take`]) and queried off the simulation lock. See the module header.
pub(crate) struct VegetationPatchInputs {
    generator: VegetationGenerator,
    edits: VegetationEdits,
    terrain: TerrainSystem,
    roadbed: RoadbedSystem,
    road_chunks: RoadChunkMap,
    sites: SiteSnapshot,
    index: TerrainQueryIndex,
    fields: FieldClearanceIndex,
    water: VisibleDepth,
    // The patch's square, which every query must stay inside (with `QUERY_PAD_M`).
    origin: Vector2,
    span: f32,
}

impl VegetationPatchInputs {
    /// Copies what the patch at `origin` (`span` metres square) reads. `None` when the roadbed
    /// does not match the graph yet or the building site index is being rebuilt; Godot then
    /// asks again. `O(1)` for the shared parts, plus the window's sites, index tiles, fields
    /// and edited cells.
    pub(crate) fn take(core: &mut SimCore, origin: Vector2, span: f32) -> Option<Self> {
        if !core.transit_network.roadbed.is_current() {
            return None;
        }
        core.prepare_site_queries();
        let low = origin - Vector2::splat(QUERY_PAD_M);
        let high = origin + Vector2::splat(span + QUERY_PAD_M);
        let grow = |by: f64| {
            [
                f64::from(low.x) - by,
                f64::from(low.y) - by,
                f64::from(high.x) + by,
                f64::from(high.y) + by,
            ]
        };
        let tiles = grow(TILE_PAD_M);
        let sites = core
            .allocator
            .site_snapshot_by_reach(grow(TILE_PAD_M + SURFACE_PAD_M))?;
        let range = |layer| {
            let (cell_m, _) = grid(core, layer);
            let (first, last) = (cell_at(low, layer, cell_m), cell_at(high, layer, cell_m));
            (layer, (first.x, last.x), (first.z, last.z))
        };
        let edits = core.vegetation_edits.window([
            range(VegetationLayer::Canopy),
            range(VegetationLayer::Understory),
        ]);
        Some(Self {
            generator: core.vegetation,
            edits,
            terrain: core.heightmap.clone(),
            roadbed: core.transit_network.roadbed.clone(),
            road_chunks: core.cached_road_mesh_chunks.clone(),
            sites,
            index: core.terrain_query_index.window(tiles),
            fields: core.allocator.field_clearance.window(low, high),
            water: core.watermap.visible_depth(),
            origin,
            span,
        })
    }

    // Whether a point is inside the window the snapshot answers for.
    fn holds(&self, pos: Vector2) -> bool {
        let low = self.origin - Vector2::splat(QUERY_PAD_M);
        let high = self.origin + Vector2::splat(self.span + QUERY_PAD_M);
        pos.x >= low.x && pos.y >= low.y && pos.x <= high.x && pos.y <= high.y
    }
}

impl VegetationWorld for VegetationPatchInputs {
    fn generator(&self) -> &VegetationGenerator {
        &self.generator
    }

    fn edits(&self) -> &VegetationEdits {
        &self.edits
    }

    fn world_size(&self) -> (f32, f32) {
        self.terrain.world_size()
    }

    fn roadbed(&self) -> &RoadbedSystem {
        &self.roadbed
    }

    fn road_chunks(&self) -> &RoadChunkMap {
        &self.road_chunks
    }

    fn with_surface<T>(&self, query: impl FnOnce(&GradedSurface<'_>, &mut QueryScratch) -> T) -> T {
        let surface = GradedSurface {
            terrain: &self.terrain,
            roadbed: &self.roadbed,
            sites: &self.sites,
            query_index: Some(&self.index),
        };
        SNAPSHOT_SCRATCH.with_borrow_mut(|scratch| query(&surface, scratch))
    }

    fn water_depth(&self, pos: Vector2) -> f32 {
        self.water.at_world(pos.x, pos.y)
    }

    fn field_covers(&self, pos: Vector2) -> bool {
        debug_assert!(
            self.holds(pos),
            "vegetation snapshot read outside its window"
        );
        self.fields.covers_point(pos)
    }

    fn blocks_yard_plant(&self, pos: Vector2) -> bool {
        debug_assert!(
            self.holds(pos),
            "vegetation snapshot read outside its window"
        );
        self.sites.blocks_yard_plant(pos)
    }

    fn in_yard_planting(&self, pos: Vector2) -> bool {
        debug_assert!(
            self.holds(pos),
            "vegetation snapshot read outside its window"
        );
        self.sites.in_yard_planting(pos)
    }

    fn parallel(&self) -> bool {
        false
    }
}
