// SPDX-License-Identifier: GPL-2.0-only

//! The graded terrain's tiles in the road tool's preview poses, the commit and undo
//! (`TERRAIN-07-E`, `docs/terrain.md` "Preview, commit, undo and load"): the Rust half.
//!
//! - A pose selects the dirty tiles of the preview roadbed against the published one and bakes
//!   only those whose inputs differ from the last pose's staged tiles, so a drag pays per pose
//!   for what the pose changed.
//! - The commit selects its own dirty tiles over the live roadbed and sites, adopts the matching
//!   preview's staged tiles whose inputs equal the committed ones and bakes the rest. Its undo
//!   record keeps the tiles the commit replaced (T20 (a)): a pure job bakes them from the
//!   pre-commit inputs after the commit, off the frame.
//! - Undo selects its own dirty tiles and adopts the kept tiles whose inputs equal the restored
//!   ones; a tile whose inputs changed meanwhile outside the undo stack (a site the demand pass
//!   placed) is baked again, so a kept tile is never stale.
//!
//! Inputs decide every reuse, so each staged tile is bitwise the fresh bake of the state it
//! stands for (invariant 9). The tiles of the last commit or undo stay on the core for the
//! Godot install (the Godot half, after `TERRAIN-07-D` lands).

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use super::road_terrain_inputs::RoadTerrainInputs;
use super::state::SimCore;
use crate::simulation::buildings::allocator::BuildingSiteTerrainSnapshot;
use crate::simulation::network::roadbed::{RoadbedSystem, TerrainChanges};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{
    BakeError, Level, SiteBuffer, SiteSource, StageScratch, StageStats, TileStage, bake_window,
    stage_selected, stage_tiles,
};

/// The levels the Rust half stages an edit's dirty tiles at. Godot's residency (the Godot
/// half) gives each resident tile its own level; until then every dirty tile is staged at
/// `GL0`, the finest and dearest level and the one the budgets were measured at.
pub(crate) const GRADED_EDIT_LEVELS: [Level; 1] = [Level::Gl0];

/// The building sites the road tool's view captured per render patch
/// ([`RoadTerrainInputs`]), answered with the allocator's own test: a graded bake from them
/// equals the bake from the allocator, inputs included, while the captured patches are current.
pub(crate) struct PreviewSites<'a> {
    terrain: &'a TerrainSystem,
    inputs: &'a RoadTerrainInputs,
}

impl<'a> PreviewSites<'a> {
    pub(crate) fn new(terrain: &'a TerrainSystem, inputs: &'a RoadTerrainInputs) -> Self {
        Self { terrain, inputs }
    }
}

impl SiteSource for PreviewSites<'_> {
    // The inputs hold every patch once they are first rebuilt.
    fn ready(&self) -> bool {
        !self.inputs.patches.is_empty()
    }

    fn sites_in_bounds(&self, rect: [f64; 4], out: &mut SiteBuffer) {
        thread_local! {
            static KEYS: std::cell::RefCell<Vec<(usize, usize)>> =
                const { std::cell::RefCell::new(Vec::new()) };
            static SNAPSHOTS: std::cell::RefCell<Vec<Arc<BuildingSiteTerrainSnapshot>>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }
        // A site meeting the rectangle holds a point of it, in a patch whose bounds the
        // rectangle meets, and that patch captured every site meeting its bounds.
        KEYS.with_borrow_mut(|keys| {
            SNAPSHOTS.with_borrow_mut(|snapshots| {
                keys.clear();
                let [min_x, min_z, max_x, max_z] = rect;
                self.terrain.append_render_patch_keys_for_world_bounds(
                    (min_x - 1.0) as f32,
                    (min_z - 1.0) as f32,
                    (max_x + 1.0) as f32,
                    (max_z + 1.0) as f32,
                    keys,
                );
                snapshots.clear();
                snapshots.extend(
                    keys.iter()
                        .filter_map(|key| self.inputs.patch(*key))
                        .map(|patch| Arc::clone(&patch.sites)),
                );
                BuildingSiteTerrainSnapshot::push_graded_sites(snapshots, rect, out);
                // Nothing outlives the query.
                snapshots.clear();
            })
        });
    }
}

/// One pose's graded tiles: the staged tiles, or why the pose could not bake them, with what
/// the staging did.
#[derive(Clone, Debug)]
pub(crate) struct GradedPose {
    pub(crate) stage: Result<Arc<TileStage>, BakeError>,
    pub(crate) stats: StageStats,
}

/// The road tool's staged graded tiles between poses, and their buffers.
#[derive(Default)]
pub(crate) struct GradedPreview {
    stage: Arc<TileStage>,
    scratch: StageScratch,
}

impl GradedPreview {
    /// Stages one pose: the dirty tiles of `changes` from the published roadbed `before` to the
    /// pose's roadbed `after`, baking only those whose inputs differ from the last pose's.
    /// `O(selection + dirty tiles' gathers + changed tiles' bakes)`.
    pub(crate) fn stage_pose(
        &mut self,
        terrain: &TerrainSystem,
        before: &RoadbedSystem,
        after: &RoadbedSystem,
        sites: &impl SiteSource,
        changes: &TerrainChanges,
    ) -> GradedPose {
        let select_start = Instant::now();
        let mut stats = StageStats::default();
        let staged = if sites.ready() {
            self.scratch.select_road_edit(
                terrain,
                before,
                after,
                sites,
                changes,
                &GRADED_EDIT_LEVELS,
            );
            stats.select_ms = select_start.elapsed().as_secs_f64() * 1000.0;
            stage_selected(terrain, after, sites, &self.stage, &mut self.scratch)
        } else {
            Err(BakeError::SitesUnavailable)
        };
        let stage = staged.map(|(stage, staged)| {
            stats = StageStats {
                select_ms: stats.select_ms,
                ..staged
            };
            let stage = Arc::new(stage);
            let old = std::mem::replace(&mut self.stage, Arc::clone(&stage));
            let old = Arc::try_unwrap(old).unwrap_or_else(|shared| TileStage::clone(&shared));
            self.scratch.retire(old, &stage);
            stage
        });
        GradedPose { stage, stats }
    }
}

/// What a road commit's undo record keeps of the graded tiles (T20 (a)): the tiles the commit
/// replaced, baked from the pre-commit inputs by a pure job after the commit. Undo adopts those
/// whose inputs equal the restored ones; until the job finishes it has none to adopt and bakes.
pub(crate) struct GradedUndo {
    kept: Arc<OnceLock<Result<TileStage, BakeError>>>,
    // When the job started and finished (benchmarks: the frames after the commit share the
    // pool with it).
    #[cfg(test)]
    span: Arc<OnceLock<(Instant, Instant)>>,
}

impl GradedUndo {
    /// The kept tiles once the job has baked them.
    pub(crate) fn kept(&self) -> Option<&TileStage> {
        self.kept.get().and_then(|kept| kept.as_ref().ok())
    }

    /// Waits for the job (tests: a pool worker must never wait for a pure job).
    #[cfg(test)]
    pub(crate) fn wait(&self) -> Result<&TileStage, BakeError> {
        self.kept.wait().as_ref().map_err(|error| *error)
    }

    /// When the job started and finished, once it has.
    #[cfg(test)]
    pub(crate) fn span(&self) -> Option<(Instant, Instant)> {
        self.span.get().copied()
    }
}

impl SimCore {
    // The sites the graded bakes read, with the 512 m site index current.
    fn prepare_graded_sites(&mut self) {
        if !self.allocator.building_site_query_index_ready() {
            self.allocator
                .prepare_building_site_query_index(self.config.zone_cell_m);
        }
    }

    // Stages the dirty tiles of the last roadbed install from `before` (`last_road_terrain_
    // changes`) over the live roadbed, sites and terrain, adopting `held`'s matching tiles;
    // the result becomes the core's last graded tiles.
    fn stage_live_graded_tiles(
        &mut self,
        before: &RoadbedSystem,
        held: Option<&TileStage>,
    ) -> StageStats {
        self.prepare_graded_sites();
        let select_start = Instant::now();
        let Self {
            heightmap,
            transit_network,
            allocator,
            last_road_terrain_changes,
            graded_scratch,
            ..
        } = self;
        let roadbed = &transit_network.roadbed;
        graded_scratch.select_road_edit(
            heightmap,
            before,
            roadbed,
            &*allocator,
            last_road_terrain_changes,
            &GRADED_EDIT_LEVELS,
        );
        let select_ms = select_start.elapsed().as_secs_f64() * 1000.0;
        let empty = TileStage::default();
        let staged = stage_selected(
            heightmap,
            roadbed,
            &*allocator,
            held.unwrap_or(&empty),
            graded_scratch,
        );
        let stats = match staged {
            Ok((stage, stats)) => {
                let stage = Arc::new(stage);
                let old = std::mem::replace(&mut self.last_graded_tiles, Ok(Arc::clone(&stage)));
                if let Ok(old) = old {
                    let old =
                        Arc::try_unwrap(old).unwrap_or_else(|shared| TileStage::clone(&shared));
                    self.graded_scratch.retire(old, &stage);
                }
                StageStats { select_ms, ..stats }
            }
            Err(error) => {
                self.last_graded_tiles = Err(error);
                StageStats {
                    select_ms,
                    ..StageStats::default()
                }
            }
        };
        self.last_graded_stats = stats;
        stats
    }

    /// The commit's graded tiles (`TERRAIN-07-E`): the dirty tiles of its install from
    /// `before`, the preview's staged `held` tiles adopted where their inputs equal the
    /// committed ones, the rest baked. No terrain failure rejects the road: a tile the payload
    /// cannot represent (`T7`) is reported in [`SimCore::last_graded_tiles`]. Returns what the
    /// undo record keeps, `None` in benchmark mode, which keeps no undo record.
    pub(crate) fn stage_committed_graded_tiles(
        &mut self,
        before: RoadbedSystem,
        held: Option<&TileStage>,
    ) -> Option<GradedUndo> {
        self.stage_live_graded_tiles(&before, held);
        if self.benchmark_mode {
            return None;
        }
        // The tiles the commit replaced: the same dirty tiles over the pre-commit roadbed and
        // the same sites (a commit changes none), baked off the frame on the background pool,
        // which the commit's own later stages and the next frames do not share.
        let wanted = self.graded_scratch.selected().to_vec();
        let windows = wanted
            .iter()
            .map(|&(level, tile)| bake_window(&self.heightmap, tile, level))
            .collect();
        let sites = self.allocator.graded_site_snapshot(windows);
        let terrain = self.heightmap.clone();
        let kept = Arc::new(OnceLock::new());
        let result = Arc::clone(&kept);
        #[cfg(test)]
        let (span, job_span) = {
            let span = Arc::new(OnceLock::new());
            (Arc::clone(&span), span)
        };
        crate::thread_pools::spawn(move || {
            #[cfg(test)]
            let started = Instant::now();
            let staged = match &sites {
                Some(sites) => stage_tiles(
                    &terrain,
                    &before,
                    sites,
                    &wanted,
                    &TileStage::default(),
                    &mut StageScratch::default(),
                )
                .map(|(stage, _)| stage),
                None => Err(BakeError::SitesUnavailable),
            };
            #[cfg(test)]
            let _ = job_span.set((started, Instant::now()));
            let _ = result.set(staged);
        });
        Some(GradedUndo {
            kept,
            #[cfg(test)]
            span,
        })
    }

    /// Undo's graded tiles once the roadbed is restored and published (`TERRAIN-07-E`): the
    /// dirty tiles of the restore from `before` (the roadbed the undo replaced), the kept tiles
    /// adopted where their inputs equal the restored ones, the rest baked. Never waits for the
    /// kept tiles' job.
    pub(crate) fn restage_graded_tiles_for_undo(
        &mut self,
        before: &RoadbedSystem,
        undo: GradedUndo,
    ) {
        self.stage_live_graded_tiles(before, undo.kept());
    }
}
