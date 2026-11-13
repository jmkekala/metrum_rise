// SPDX-License-Identifier: GPL-2.0-only

//! The values Godot's per-frame queries read, published with the render snapshot (`CODE-26`):
//! the land cover's revisions, the coal pit overlay revision, the growth summary and the
//! visible surface (terrain and world-surface heights). A query reads the latest publication
//! and never takes the core lock.
//!
//! Freshness: a publication carries the core's state when it was built. The simulation thread
//! publishes with the render snapshot after every frame and edit; a bridge call that took the
//! core on Godot's main thread republishes as it releases the core (`SimulationNode`'s core
//! guard), before the frame's readers run, so an edit is visible to the frame that made it.
//! Publications are ordered by a sequence taken under the core lock, and an older one never
//! replaces a newer one ([`FrameQueries::newer`]).
//!
//! Cost: a key compare, `O(1)` with no allocation, while nothing changed. A change republishes
//! only what moved: the growth strings (the hourly assessment), the vegetation revisions
//! (`O(edited patches)`), the terrain payload revisions (`O(revised patches)`) and the surface:
//! the road tool's terrain inputs of the noted patches, an `O(1)` roadbed copy and a terrain
//! copy shared with the road tool's view (storage chunk handles, no sample copied).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use godot::prelude::{Vector2, Vector2i};

use super::super::graded_stage::PreviewSites;
use super::super::road_terrain_inputs::RoadTerrainInputs;
use super::super::state::SimCore;
use crate::config::HEIGHT_SCALE;
use crate::simulation::economy::demand::GrowthDisplayKey;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch};
use crate::simulation::vegetation::edits::pack_patch_key;

/// The visible surface as one publication saw it: the heightmap, the roadbed, and the building
/// sites the road tool's terrain inputs captured per render patch, all shared, none copied by
/// a query. Its answers equal the core's while the publication is current.
pub(crate) struct SurfaceView {
    terrain: Arc<TerrainSystem>,
    roadbed: Arc<RoadbedSystem>,
    inputs: RoadTerrainInputs,
}

impl SurfaceView {
    /// The heightmap elevation at a world X/Z position, as `SimCore::get_height_at_internal`.
    /// O(1).
    pub(crate) fn height_at(&self, pos: Vector2) -> f32 {
        self.terrain.sample_height_world(pos.x, pos.y) * HEIGHT_SCALE
    }

    /// The visible world-surface height, as `SimCore::get_world_surface_height_internal`: the
    /// road's where a road covers the point, else the graded ground. The graded query's own
    /// bound over the captured roadbed and sites (the index-free gather, as the road tool's
    /// view queries); allocation-free once this thread's scratch is warm.
    pub(crate) fn surface_height_at(&self, pos: Vector2) -> f32 {
        thread_local! {
            static SCRATCH: RefCell<QueryScratch> = RefCell::default();
        }
        let sites = PreviewSites::new(&self.terrain, &self.inputs);
        let surface = GradedSurface {
            terrain: &self.terrain,
            roadbed: &self.roadbed,
            sites: &sites,
            query_index: None,
        };
        SCRATCH.with_borrow_mut(|scratch| {
            surface.surface_height_at(f64::from(pos.x), f64::from(pos.y), scratch) as f32
        })
    }
}

/// One publication of the per-frame values. Cloning shares every part.
#[derive(Clone, Default)]
pub(crate) struct FrameQueries {
    /// Orders publications; taken under the core lock.
    pub(crate) sequence: u64,
    /// `SimulationNode::get_land_cover_epoch`'s value.
    pub(crate) land_cover_epoch: u64,
    /// The terrain payload revision every patch has at least.
    pub(crate) terrain_payload_global_generation: u64,
    /// Patch terrain payload revisions newer than the global one.
    pub(crate) terrain_payload_patch_generations: Arc<HashMap<(usize, usize), u64>>,
    /// Vegetation patch revisions by packed patch key; an absent key is zero.
    pub(crate) vegetation_patch_generations: Arc<HashMap<i64, u64>>,
    /// Revision of the committed coal pit visuals.
    pub(crate) coal_pit_overlay_revision: u64,
    /// The R/C/I growth summary (`DemandSystem::growth_display`).
    pub(crate) growth: Arc<[(usize, String); 3]>,
    /// The visible surface; `None` before the first publication.
    pub(crate) surface: Option<Arc<SurfaceView>>,
}

impl FrameQueries {
    /// Whether this publication is newer than `other`.
    pub(crate) fn newer(&self, other: &Self) -> bool {
        self.sequence > other.sequence
    }

    /// The nine (terrain payload, vegetation) revision pairs a patch's land cover depends on,
    /// as `vegetation_api::land_cover::generations` reads them from the core. Nine lookups.
    pub(crate) fn land_cover_generations(&self, key: Vector2i) -> [i64; 18] {
        let mut result = [0; 18];
        for (i, pair) in result.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let x = key.x + i as i32 % 3 - 1;
            let z = key.y + i as i32 / 3 - 1;
            if x >= 0 && z >= 0 {
                pair[0] = self
                    .terrain_payload_patch_generations
                    .get(&(x as usize, z as usize))
                    .copied()
                    .unwrap_or(0)
                    .max(self.terrain_payload_global_generation) as i64;
                pair[1] = self
                    .vegetation_patch_generations
                    .get(&pack_patch_key(x, z))
                    .copied()
                    .unwrap_or(0) as i64;
            }
        }
        result
    }
}

// Everything a publication reads from the core, compared before anything is rebuilt.
#[derive(Clone, Copy, PartialEq)]
struct PublishedKey {
    world_epoch: u64,
    terrain_payload_counter: u64,
    terrain_payload_global_generation: u64,
    vegetation_epoch: u64,
    coal_pit_overlay_revision: u64,
    growth: GrowthDisplayKey,
    terrain_source_generation: u64,
    roadbed_generation: u64,
}

/// The latest publication and what it was built from; derived, never saved.
#[derive(Default)]
pub(crate) struct FrameQueriesPublisher {
    published: FrameQueries,
    key: Option<PublishedKey>,
    // The terrain copy the published surface and the road tool's view share, with the world
    // epoch and terrain source generation it was copied at.
    terrain: Option<((u64, u64), Arc<TerrainSystem>)>,
}

impl SimCore {
    fn frame_queries_key(&self) -> PublishedKey {
        PublishedKey {
            world_epoch: self.world_epoch,
            terrain_payload_counter: self.terrain_payload_generation_counter,
            terrain_payload_global_generation: self.terrain_payload_global_generation,
            vegetation_epoch: self.vegetation_edits.epoch(),
            coal_pit_overlay_revision: self.get_coal_pit_overlay_revision_internal(),
            growth: self.demand.growth_display_key(),
            terrain_source_generation: self.heightmap.source_generation(),
            roadbed_generation: self.transit_network.roadbed.generation(),
        }
    }

    /// The latest publication of the per-frame values.
    pub(crate) fn frame_queries(&self) -> &FrameQueries {
        &self.frame_queries.published
    }

    /// An immutable copy of the heightmap at its current source generation, made once per
    /// generation and shared by the published surface and the road tool's view. The copy
    /// shares the sample chunks: `O(storage chunks)` handles when the generation moved, else
    /// `O(1)`.
    pub(crate) fn shared_terrain_copy(&mut self) -> Arc<TerrainSystem> {
        let key = (self.world_epoch, self.heightmap.source_generation());
        match &self.frame_queries.terrain {
            Some((copied, terrain)) if *copied == key => Arc::clone(terrain),
            _ => {
                let terrain = Arc::new(self.heightmap.clone());
                self.frame_queries.terrain = Some((key, Arc::clone(&terrain)));
                terrain
            }
        }
    }

    /// Brings the published per-frame values up to the core's state; the new publication when
    /// anything a reader sees moved, else `None`. `O(1)` while nothing changed (see the module
    /// docs for a change). Readies only the site part of a stale building index (`ALLOC-05`).
    pub(crate) fn refresh_frame_queries(&mut self) -> Option<FrameQueries> {
        let key = self.frame_queries_key();
        let inputs_noted = self.road_terrain_inputs.has_changes();
        let previous = self.frame_queries.key;
        if previous == Some(key) && !inputs_noted {
            return None;
        }
        // Only a site change, or a rebuild of every patch, changes the inputs' content.
        let sites_moved = self.road_terrain_inputs.has_site_changes();
        let inputs = inputs_noted.then(|| self.refresh_road_terrain_inputs_for_readers());
        let moved = |field: fn(&PublishedKey) -> u64| previous.is_none_or(|p| field(&p) != field(&key));
        let world = moved(|k| k.world_epoch);
        let surface = world
            || sites_moved
            || moved(|k| k.terrain_source_generation)
            || moved(|k| k.roadbed_generation);
        if previous == Some(key) && !surface {
            return None;
        }
        let mut queries = self.frame_queries.published.clone();
        if world || moved(|k| k.terrain_payload_counter) || moved(|k| k.terrain_payload_global_generation)
        {
            queries.terrain_payload_global_generation = self.terrain_payload_global_generation;
            queries.terrain_payload_patch_generations =
                Arc::new(self.terrain_payload_patch_generations.clone());
        }
        if world || moved(|k| k.vegetation_epoch) {
            queries.vegetation_patch_generations =
                Arc::new(self.vegetation_edits.patch_generations().collect());
        }
        if previous.is_none_or(|p| p.growth != key.growth) {
            queries.growth = Arc::new(self.demand.growth_display());
        }
        queries.land_cover_epoch = self
            .terrain_payload_generation_counter
            .wrapping_add(self.vegetation_edits.epoch());
        queries.coal_pit_overlay_revision = key.coal_pit_overlay_revision;
        if surface {
            let inputs = inputs.unwrap_or_else(|| self.road_terrain_inputs.current());
            queries.surface = Some(Arc::new(SurfaceView {
                terrain: self.shared_terrain_copy(),
                roadbed: Arc::new(self.transit_network.roadbed.snapshot()),
                inputs,
            }));
        }
        queries.sequence = self.frame_queries.published.sequence + 1;
        self.frame_queries.published = queries.clone();
        self.frame_queries.key = Some(key);
        Some(queries)
    }
}
