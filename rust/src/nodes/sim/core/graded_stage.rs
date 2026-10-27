// SPDX-License-Identifier: GPL-2.0-only

//! The graded terrain's tiles in the road tool's preview poses, the commit and undo
//! (`TERRAIN-07-E`, `docs/terrain.md` "Preview, commit, undo and load"): the Rust half.
//!
//! - A pose selects the dirty tiles of the preview roadbed against the published one and
//!   stages a bounded share of them per frame (T27 (d), decided by the user, 2026-10-27),
//!   nearest the stroke's moving end first, adopting a shown tile whose inputs equal the pose's
//!   and baking the rest; the frames after it stage the rest, the earlier poses' tiles shown
//!   until then, so a frame's terrain work stays within its share whatever the edit.
//! - The commit selects its own dirty tiles over the live roadbed and sites, adopts the matching
//!   preview's staged tiles whose inputs equal the committed ones and bakes the rest. Its undo
//!   record keeps the tiles the commit replaced (T20 (a)): a pure job bakes them from the
//!   pre-commit inputs after the commit, off the frame.
//! - Undo selects its own dirty tiles and adopts the kept tiles whose inputs equal the restored
//!   ones; a tile whose inputs changed meanwhile outside the undo stack (a site the demand pass
//!   placed) is baked again, so a kept tile is never stale.
//!
//! Inputs decide every reuse, so each staged tile is bitwise the fresh bake of the state it
//! stands for (invariant 9); a shown preview tile is the exact bake of some pose's inputs,
//! never a blend or a part of one. The tiles of the last commit or undo stay on the core for the
//! Godot install (the Godot half, after `TERRAIN-07-D` lands).

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use super::road_terrain_inputs::RoadTerrainInputs;
use super::state::SimCore;
use crate::simulation::buildings::allocator::BuildingSiteTerrainSnapshot;
use crate::simulation::network::roadbed::{RoadbedSystem, TerrainChanges};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{
    BakeError, Level, SiteBuffer, SiteSource, StageScratch, StageStats, TILE_M, TileKey, TileStage,
    bake_window, stage_selected, stage_tiles,
};

/// The levels the Rust half stages an edit's dirty tiles at. Godot's residency (the Godot
/// half) gives each resident tile its own level; until then every dirty tile is staged at
/// `GL0`, the finest and dearest level and the one the budgets were measured at.
pub(crate) const GRADED_EDIT_LEVELS: [Level; 1] = [Level::Gl0];

/// The building sites the road tool's view captured per render patch
/// ([`RoadTerrainInputs`]), answered with the allocator's own test: a graded bake from them
/// equals the bake from the allocator, inputs included, while the captured patches are current.
struct PreviewSites<'a> {
    terrain: &'a TerrainSystem,
    inputs: &'a RoadTerrainInputs,
}

impl<'a> PreviewSites<'a> {
    fn new(terrain: &'a TerrainSystem, inputs: &'a RoadTerrainInputs) -> Self {
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

/// Tiles one pose's frame stages at most after its selection (T27 (d), decided by the user,
/// 2026-10-27): each is gathered, then adopted from the shown tiles or baked. A bound on the
/// work, never on time, measured to fit the 3.0 ms terrain stage on four workers with the
/// selection (`docs/terrain.md` "Preview, commit, undo and load"); a pose with more tiles
/// finishes them in the frames after it.
pub(crate) const POSE_FRAME_TILES: usize = 1;

/// Tiles a catch-up frame stages at most: a frame without a new pose has no selection.
pub(crate) const CATCH_UP_FRAME_TILES: usize = 3;

/// One frame's graded tiles of the road tool's preview: the tiles shown after it, or why the
/// frame could not stage them, with what the frame staged.
#[derive(Clone, Debug)]
pub(crate) struct GradedPose {
    /// Per wanted tile of the latest pose, its exact bake of this pose's inputs once staged,
    /// else the tile an earlier pose showed (no tile: the published terrain shows).
    pub(crate) stage: Result<Arc<TileStage>, BakeError>,
    /// The frame's staging: `wanted` the pose's tiles, `adopted` and `baked` this frame's.
    pub(crate) stats: StageStats,
    /// The pose's tiles still to stage in later frames; 0 once the shown tiles are the pose's.
    pub(crate) pending: usize,
    /// The frame's whole terrain work, milliseconds: the selection, the staging and the shown
    /// tiles.
    pub(crate) frame_ms: f64,
}

// What the latest pose's remaining tiles bake from: the inputs it was selected over, `O(1)`
// copies.
struct PoseInputs {
    terrain: Arc<TerrainSystem>,
    roadbed: RoadbedSystem,
    sites: RoadTerrainInputs,
}

/// The road tool's graded preview between frames (T27 (d)): the tiles it shows and the latest
/// pose's tiles still to stage, and their buffers. Each frame stages a bounded share of the
/// latest pose's tiles in a fixed order (nearest the stroke's moving end first, then `(level,
/// tile)`), so the shown tiles after every frame depend only on the pose sequence, never on
/// timing or the worker count. A new pose supersedes the older pose's unstaged tiles; once the
/// poses stop, the shown tiles become the pose's in `ceil(remaining / CATCH_UP_FRAME_TILES)`
/// frames, bitwise its fresh bake.
#[derive(Default)]
pub(crate) struct GradedPreview {
    shown: Arc<TileStage>,
    pending: Option<PoseInputs>,
    // The latest pose's tiles in `(level, tile)` order, then in staging order, of which the
    // first `next` are staged.
    wanted: Vec<(Level, TileKey)>,
    order: Vec<(Level, TileKey)>,
    next: usize,
    // One frame's share, in `(level, tile)` order.
    share: Vec<(Level, TileKey)>,
    scratch: StageScratch,
}

impl GradedPreview {
    /// One pose's frame: selects the dirty tiles of `changes` from the published roadbed
    /// `before` to the pose's roadbed `after` and stages the first [`POSE_FRAME_TILES`] of them
    /// nearest `end` (the stroke's moving end, world XZ), adopting shown tiles whose inputs
    /// equal the pose's. `O(selection + share's gathers and bakes + wanted log wanted)`.
    pub(crate) fn stage_pose(
        &mut self,
        terrain: &Arc<TerrainSystem>,
        before: &RoadbedSystem,
        after: &RoadbedSystem,
        sites: &RoadTerrainInputs,
        changes: &TerrainChanges,
        end: [f64; 2],
    ) -> GradedPose {
        let start = Instant::now();
        self.pending = None;
        let preview_sites = PreviewSites::new(terrain, sites);
        if !preview_sites.ready() {
            return self.failed(BakeError::SitesUnavailable, start);
        }
        self.scratch.select_road_edit(
            terrain,
            before,
            after,
            &preview_sites,
            changes,
            &GRADED_EDIT_LEVELS,
        );
        let select_ms = start.elapsed().as_secs_f64() * 1000.0;
        self.wanted.clear();
        self.wanted.extend_from_slice(self.scratch.selected());
        self.order.clone_from(&self.wanted);
        // Nearest the moving end first: the squared distance from `end` to the tile's square
        // (non-negative, so its order is total), then `(level, tile)`.
        let distance = |&(_, tile): &(Level, TileKey)| {
            let [x, z] = tile.origin();
            let dx = (x - end[0]).max(end[0] - x - TILE_M).max(0.0);
            let dz = (z - end[1]).max(end[1] - z - TILE_M).max(0.0);
            dx * dx + dz * dz
        };
        self.order
            .sort_by(|a, b| distance(a).total_cmp(&distance(b)).then_with(|| a.cmp(b)));
        self.next = 0;
        self.pending = Some(PoseInputs {
            terrain: Arc::clone(terrain),
            roadbed: after.snapshot(),
            sites: sites.clone(),
        });
        self.advance(POSE_FRAME_TILES, select_ms, start)
    }

    /// One frame without a new pose: stages the next [`CATCH_UP_FRAME_TILES`] of the latest
    /// pose's tiles. `None` once the shown tiles are the pose's (or no pose is pending).
    pub(crate) fn catch_up(&mut self) -> Option<GradedPose> {
        self.pending.as_ref()?;
        Some(self.advance(CATCH_UP_FRAME_TILES, 0.0, Instant::now()))
    }

    /// Drops the latest pose's unstaged tiles: a newer pose that compiled no pieces, or a
    /// publication that replaced the view, supersedes them. The shown tiles stay for adoption.
    pub(crate) fn supersede(&mut self) {
        self.pending = None;
    }

    // Stages the next `count` tiles of the pending pose against the shown tiles and shows them
    // over the shown tiles of the pose's other wanted tiles.
    fn advance(&mut self, count: usize, select_ms: f64, start: Instant) -> GradedPose {
        let inputs = self.pending.as_ref().expect("a pending pose");
        let end = (self.next + count).min(self.order.len());
        self.share.clear();
        self.share.extend_from_slice(&self.order[self.next..end]);
        self.share.sort_unstable();
        let sites = PreviewSites::new(&inputs.terrain, &inputs.sites);
        let staged = stage_tiles(
            &inputs.terrain,
            &inputs.roadbed,
            &sites,
            &self.share,
            &self.shown,
            &mut self.scratch,
        );
        let (staged, stats) = match staged {
            Ok(staged) => staged,
            Err(error) => {
                self.pending = None;
                return self.failed(error, start);
            }
        };
        self.next = end;
        let pending = self.order.len() - end;
        if pending == 0 {
            self.pending = None;
        }
        let shown = Arc::new(TileStage::overlay(&self.wanted, &staged, &self.shown));
        let old = std::mem::replace(&mut self.shown, Arc::clone(&shown));
        let old = Arc::try_unwrap(old).unwrap_or_else(|shared| TileStage::clone(&shared));
        self.scratch.retire(old, &shown);
        GradedPose {
            stage: Ok(shown),
            stats: StageStats {
                wanted: self.wanted.len(),
                select_ms,
                ..stats
            },
            pending,
            frame_ms: start.elapsed().as_secs_f64() * 1000.0,
        }
    }

    fn failed(&self, error: BakeError, start: Instant) -> GradedPose {
        GradedPose {
            stage: Err(error),
            stats: StageStats::default(),
            pending: 0,
            frame_ms: start.elapsed().as_secs_f64() * 1000.0,
        }
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
