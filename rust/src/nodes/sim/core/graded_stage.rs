// SPDX-License-Identifier: GPL-2.0-only

//! The graded terrain's tiles in the road tool's preview poses, the commit and undo
//! (`TERRAIN-07-E`, `docs/terrain.md` "Preview, commit, undo and load"): the Rust half.
//!
//! - A pose's whole selection is staged as one set by a job on the background pool
//!   (`TERRAIN-14`, superseding T27 (d)'s per-frame share): the job selects the dirty tiles of
//!   the pose's roadbed against the published one and stages them, adopting a tile of the last
//!   landed set whose inputs equal the pose's and baking the rest. The frame only takes the
//!   pose's inputs and polls for a landed set, so its terrain work is `O(1)` plus the hand-over.
//! - The commit selects its own dirty tiles over the live roadbed and sites, adopts the matching
//!   preview's staged tiles whose inputs equal the committed ones and bakes the rest. Its undo
//!   record keeps the tiles the commit replaced (T20 (a)): a pure job bakes them from the
//!   pre-commit inputs, off the frame, queued when the commit's last stage ends.
//! - Undo selects its own dirty tiles and adopts the kept tiles whose inputs equal the restored
//!   ones; a tile whose inputs changed meanwhile outside the undo stack (a site the demand pass
//!   placed) is baked again, so a kept tile is never stale.
//!
//! Inputs decide every reuse, so each staged tile is bitwise the fresh bake of the state it
//! stands for (invariant 9); a shown preview tile is the exact bake of some pose's inputs,
//! never a blend or a part of one. The renderer is handed one pose's whole set, each tile its
//! exact bake, or none (`TERRAIN-12`, `docs/terrain.md` T60 and T61, the user, 2026-10-29): the
//! latest pose's set once it lands, and a pose that draws no road draws no preview terrain. Which
//! frame a set lands in depends on its job's time; what it holds does not. Each tile is staged at
//! the level the renderer's distance rule gives it ([`LevelRule`], T42). The renderer takes each
//! frame's changed displayed tiles ([`GradedPreview::take_shown_changes`]); the tiles of a commit or
//! undo ride on the graded edit journal's record of the install, so the renderer installs them
//! instead of rebaking.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use super::road_terrain_inputs::RoadTerrainInputs;
use super::state::SimCore;
use crate::simulation::buildings::allocator::BuildingSiteTerrainSnapshot;
use crate::simulation::core::pure_job::{JobStart, PureJob};
use crate::simulation::network::roadbed::{RoadbedDelta, RoadbedSystem, TerrainChanges};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{
    BakeError, LevelRule, SiteBuffer, SiteSource, StageScratch, StageStats, StagedTile, TileKey,
    TileStage, bake_window, stage_selected, stage_tiles,
};
use crate::thread_pools::lane::{FrameLane, Taken};

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

/// The tiles whose published road cut the drawn road may leave open (T65 option (c), `ROAD-112`
/// round G2): every tile meeting the bounds of a published piece `delta` replaces. A preview
/// draws `delta`'s pieces in place of those (their runs collapsed), so a published tile's cut
/// opens where they covered the ground and the pose's pieces do not; outside these tiles every
/// published cut lies under a piece the road still draws. Into `out`, sorted and unique.
/// `O(replaced pieces x their tiles + out log out)`, independent of the city.
pub(crate) fn replaced_cut_tiles(
    published: &RoadbedSystem,
    delta: &RoadbedDelta,
    out: &mut Vec<TileKey>,
) {
    out.clear();
    for (id, _) in delta.pieces() {
        let Some(mesh) = published.piece_mesh(id) else {
            continue;
        };
        let [min_x, min_z, max_x, max_z] = mesh.bounds_xz();
        let (low, high) = (TileKey::at(min_x, min_z), TileKey::at(max_x, max_z));
        for x in low.x..=high.x {
            out.extend((low.z..=high.z).map(|z| TileKey { x, z }));
        }
    }
    out.sort_unstable();
    out.dedup();
}

/// One frame's graded tiles of the road tool's preview: the newest landed set, or why it could
/// not be staged, with what its job staged.
#[derive(Clone, Debug)]
pub(crate) struct GradedPose {
    /// The newest landed set of the epoch: one pose's whole selection, each tile its exact bake
    /// of that pose's inputs (no tile before the first set lands). What a commit adopts from.
    pub(crate) stage: Result<Arc<TileStage>, BakeError>,
    /// That set's staging: `wanted` its pose's tiles, `adopted` and `baked` its job's, with the
    /// selection's and the bakes' times on the background pool.
    pub(crate) stats: StageStats,
    /// Sets still to land, in flight or queued (at most two); 0 once the newest landed set is
    /// the latest pose's.
    pub(crate) pending: usize,
    /// Poses the newest landed set is behind the latest pose (0: the latest pose's own set);
    /// `None` before any set landed in the epoch.
    pub(crate) behind: Option<u64>,
    /// The job of the set that landed in this frame, milliseconds from its start on a frame to
    /// its landing; 0 when none landed.
    pub(crate) job_ms: f64,
    /// The frame's terrain work on the calling thread, milliseconds: the pose's snapshots, the
    /// poll and the landed set's hand-over.
    pub(crate) frame_ms: f64,
    /// The level rule the latest pose's tiles are selected and staged by; a commit of the pose
    /// stages by it too, so its tiles meet the preview's at the same levels.
    pub(crate) rule: LevelRule,
    /// The preview epoch the shown tiles belong to ([`GradedPreview::release`]).
    pub(crate) epoch: u64,
}

// What a pose's set reads, owned by its job: `O(1)` copies, the changes moved in.
struct PoseInputs {
    generation: u64,
    terrain: Arc<TerrainSystem>,
    before: RoadbedSystem,
    after: RoadbedSystem,
    sites: RoadTerrainInputs,
    changes: TerrainChanges,
    rule: LevelRule,
}

// The preview's lane payload: a pose's inputs and adoption pool in, its set out, the staging's
// buffers travelling with them (reused from set to set).
#[derive(Default)]
struct PoseSet {
    pose: Option<(PoseInputs, Arc<TileStage>)>,
    // The landed set and the generation of the pose it is of.
    staged: Option<Result<(Arc<TileStage>, StageStats), BakeError>>,
    generation: u64,
    // When a frame started the job, and when it landed.
    started: Option<Instant>,
    job_ms: f64,
    scratch: StageScratch,
}

impl PoseSet {
    // The job, a pure function of its inputs: the pose's selection, then its whole set staged
    // over the adoption pool (a tile whose inputs equal a held one's is adopted, the rest are
    // baked; the bakes' parallel passes stay on the background pool). `O(one selection +
    // wanted gathers + rebaked bakes)`.
    fn stage(&mut self) {
        let Some((pose, held)) = self.pose.take() else {
            return;
        };
        let select_start = Instant::now();
        let sites = PreviewSites::new(&pose.terrain, &pose.sites);
        self.scratch.select_road_edit(
            &pose.terrain,
            &pose.before,
            &pose.after,
            &sites,
            &pose.changes,
            &pose.rule,
        );
        let select_ms = select_start.elapsed().as_secs_f64() * 1000.0;
        let staged = stage_selected(&pose.terrain, &pose.after, &sites, &held, &mut self.scratch);
        self.staged = Some(staged.map(|(stage, stats)| {
            (Arc::new(stage), StageStats { select_ms, ..stats })
        }));
        self.generation = pose.generation;
        self.job_ms = self
            .started
            .map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0);
    }
}

/// The road tool's graded preview between frames (`TERRAIN-14`): each pose's whole selection is
/// staged as one set by a job on the background pool, and the frame only polls for it. At most
/// one set is in flight and one pose queued behind it, the newest (a newer pose replaces the
/// queued one, never the set in flight, which is the next to land). A landed set becomes the
/// adoption pool of the next set and of a commit; it is displayed only while it is the latest
/// pose's (the strict display: no preview terrain under a moving road, the pose's terrain one
/// frame after the road pauses). A set of a pose a release, a cancel, a pose that draws no road
/// or a publication invalidated is dropped as it lands. Which frame shows a set depends on when
/// its job lands; every shown set is one pose's exact bake, and a converged set, the commit
/// and the undo are bitwise as before (T27 (d)'s per-frame share superseded).
#[derive(Default)]
pub(crate) struct GradedPreview {
    // The newest landed set of this epoch: what the next set and a commit adopt from.
    shown: Arc<TileStage>,
    // Its staging, the generation of its pose (0: none) and why the newest set failed, if it
    // did.
    shown_stats: StageStats,
    shown_generation: u64,
    shown_error: Option<BakeError>,
    // What the renderer draws (T61 (1) (a)): one pose's whole set, each tile its exact bake, or
    // nothing (the published tiles), never a part.
    display: Arc<TileStage>,
    // The display the renderer holds: as of the last `take_shown_changes`.
    sent: Arc<TileStage>,
    // The sets' carrier between the frame and the background pool.
    lane: FrameLane<PoseSet>,
    // The latest pose's generation, the oldest generation whose set still counts, whether a set
    // is in flight and the pose queued behind it.
    latest: u64,
    valid_from: u64,
    in_flight: bool,
    queued: Option<PoseInputs>,
    // The level rule the latest pose was selected by.
    latest_rule: LevelRule,
    // Counts releases and cancels: the renderer tags the tiles it shows with it, and a commit's
    // install releases the tiles of the epoch its pose belonged to.
    epoch: u64,
    // The renderer's view the poses select for.
    rule: LevelRule,
    // The tile keys of `display`, sorted (scratch of `take_shown_changes`).
    keys: Vec<TileKey>,
    // The latest pose's tiles whose published cut its road may leave open (scratch of
    // `cut_tiles`).
    cut: Vec<TileKey>,
    // Tests: jobs are held here instead of queued, and run by the test.
    #[cfg(test)]
    deferred: Option<Vec<Box<dyn FnOnce() + Send>>>,
}

impl GradedPreview {
    /// One pose's frame: takes the pose's inputs (the terrain, the published roadbed `before`,
    /// the pose's roadbed `after`, the site patches, `O(1)` copies, and the roadbed's `changes`,
    /// moved in), lands a set that finished, and starts the pose's set or queues the pose behind
    /// the set in flight. Nothing an earlier pose staged is displayed under this pose. `O(1)` plus
    /// a landed set's hand-over.
    pub(crate) fn stage_pose(
        &mut self,
        terrain: &Arc<TerrainSystem>,
        before: &RoadbedSystem,
        after: &RoadbedSystem,
        sites: &RoadTerrainInputs,
        changes: TerrainChanges,
    ) -> GradedPose {
        let start = Instant::now();
        if !PreviewSites::new(terrain, sites).ready() {
            self.supersede();
            return self.failed(BakeError::SitesUnavailable, start);
        }
        self.latest += 1;
        self.latest_rule = self.rule;
        self.queued = Some(PoseInputs {
            generation: self.latest,
            terrain: Arc::clone(terrain),
            before: before.snapshot(),
            after: after.snapshot(),
            sites: sites.clone(),
            changes,
            rule: self.rule,
        });
        self.display = Arc::default();
        let job_ms = self.poll();
        self.pose(start, job_ms)
    }

    /// One frame without a new pose: lands the set in flight if its job finished, and starts the
    /// queued pose's. `None` (no allocation) when no set that counts landed. `O(1)` plus a
    /// landed set's hand-over.
    pub(crate) fn catch_up(&mut self) -> Option<GradedPose> {
        if !self.in_flight {
            return None;
        }
        let start = Instant::now();
        let job_ms = self.poll()?;
        Some(self.pose(start, Some(job_ms)))
    }

    /// Sets still to land, in flight or queued (0 once the newest landed set is the latest
    /// pose's, or a release dropped the rest).
    pub(crate) fn pending_sets(&self) -> usize {
        usize::from(self.in_flight) + usize::from(self.queued.is_some())
    }

    /// Drops the poses whose sets have not landed: a newer pose that draws no road (no pieces, or
    /// invalid), or a publication that replaced the view, supersedes them. The shown set stays
    /// for adoption, but the renderer is handed none (T60: a pose without a drawn road shows no
    /// preview terrain; after a publication no earlier set is known to be exact).
    pub(crate) fn supersede(&mut self) {
        self.queued = None;
        self.valid_from = self.latest + 1;
        self.display = Arc::default();
    }

    /// Sets the renderer's view the next poses select their tiles' levels by (a pose in flight
    /// or queued keeps the levels it was selected at).
    pub(crate) fn set_rule(&mut self, rule: LevelRule) {
        self.rule = rule;
    }

    /// Hands the shown tiles over and starts a new epoch with none: at a click, whose commit
    /// installs the tiles of the epoch returned and lets the renderer release that epoch's,
    /// or at cancel. The renderer keeps what it shows until then; the next poses send theirs
    /// under the new epoch, and a set in flight is dropped as it lands. Returns the released
    /// epoch.
    pub(crate) fn release(&mut self) -> u64 {
        let released = self.epoch;
        self.epoch += 1;
        self.supersede();
        self.shown = Arc::default();
        self.shown_stats = StageStats::default();
        self.shown_generation = 0;
        self.shown_error = None;
        self.sent = Arc::default();
        released
    }

    /// The preview epoch the shown tiles belong to.
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The tiles where the renderer draws the published tiles' cut ground behind every drawn
    /// surface while they show under this pose (T65 option (c), `ROAD-112` round G2): those of
    /// [`replaced_cut_tiles`] for the pose's `delta` over the `published` roadbed, none for a
    /// pose that draws no road (`None`). Not narrowed to the pose's selection: the renderer may
    /// hold a tile at another level than the selection's rule gives it, where its bake can
    /// change unselected. `O(replaced pieces x their tiles)`; no allocation once the scratch has
    /// grown.
    pub(crate) fn cut_tiles(
        &mut self,
        published: &RoadbedSystem,
        delta: Option<&RoadbedDelta>,
    ) -> &[TileKey] {
        match delta {
            Some(delta) => replaced_cut_tiles(published, delta, &mut self.cut),
            None => self.cut.clear(),
        }
        &self.cut
    }

    /// The tiles the renderer draws: one pose's whole set, each its exact bake, or none (T60,
    /// T61).
    #[cfg(test)]
    pub(crate) fn displayed(&self) -> &TileStage {
        &self.display
    }

    /// Tests' synchronous path: waits until the set in flight, if any, has landed (the next
    /// [`Self::catch_up`] or pose takes it).
    #[cfg(test)]
    pub(crate) fn wait(&self) {
        self.lane.wait_landed();
    }

    /// Tests: holds every job from now on instead of queuing it, for [`Self::run_deferred`].
    #[cfg(test)]
    pub(crate) fn defer_jobs(&mut self) {
        self.deferred = Some(Vec::new());
    }

    /// Tests: runs the held jobs on this thread; returns how many ran.
    #[cfg(test)]
    pub(crate) fn run_deferred(&mut self) -> usize {
        let jobs = std::mem::take(self.deferred.as_mut().expect("jobs deferred"));
        let ran = jobs.len();
        jobs.into_iter().for_each(|job| job());
        ran
    }

    /// The displayed tiles changed since the last call, for the renderer: into `changed` the
    /// tiles displayed now with a payload it does not hold, into `removed` the tiles it holds
    /// that are displayed no more. `O(display + sent log display)`.
    pub(crate) fn take_shown_changes(
        &mut self,
        changed: &mut Vec<StagedTile>,
        removed: &mut Vec<TileKey>,
    ) {
        changed.clear();
        removed.clear();
        changed.extend(
            (self.display.tiles().iter())
                .filter(|tile| {
                    !(self.sent.get(tile.level, tile.tile))
                        .is_some_and(|sent| Arc::ptr_eq(sent, &tile.payload))
                })
                .cloned(),
        );
        // A wanted tile takes one level, so the displayed tiles' keys are unique.
        self.keys.clear();
        self.keys
            .extend(self.display.tiles().iter().map(|tile| tile.tile));
        self.keys.sort_unstable();
        removed.extend(
            (self.sent.tiles().iter())
                .map(|tile| tile.tile)
                .filter(|tile| self.keys.binary_search(tile).is_err()),
        );
        self.sent = Arc::clone(&self.display);
    }

    // Takes the lane's payload without waiting: lands the set in it, if one landed, then starts
    // the queued pose's set if there is one. Returns the landed set's job time when a set that
    // counts landed. `O(1)` plus the hand-over.
    fn poll(&mut self) -> Option<f64> {
        let (mut set, landed) = match self.lane.take(0.0).0 {
            Taken::Late => return None,
            Taken::Resident(set) => (set, false),
            Taken::Landed(set) => (set, true),
        };
        let counted = if landed {
            self.in_flight = false;
            self.land(&mut set).then_some(set.job_ms)
        } else {
            None
        };
        match self.queued.take() {
            Some(pose) => self.launch(set, pose),
            None => self.lane.keep(set),
        }
        counted
    }

    // Takes a landed set: one that counts becomes the shown set and, while it is the latest
    // pose's, the display; one of an invalidated pose is dropped, its buffers kept for reuse.
    // Returns whether it counted.
    fn land(&mut self, set: &mut PoseSet) -> bool {
        let Some(staged) = set.staged.take() else {
            return false;
        };
        if set.generation < self.valid_from {
            if let Ok((stage, _)) = staged {
                retire(&mut set.scratch, stage, &self.shown);
            }
            return false;
        }
        match staged {
            Ok((stage, stats)) => {
                let old = std::mem::replace(&mut self.shown, Arc::clone(&stage));
                retire(&mut set.scratch, old, &stage);
                self.shown_stats = stats;
                self.shown_error = None;
                self.display = if set.generation == self.latest {
                    stage
                } else {
                    Arc::default()
                };
            }
            Err(error) => {
                self.shown_stats = StageStats::default();
                self.shown_error = Some(error);
                self.display = Arc::default();
            }
        }
        self.shown_generation = set.generation;
        true
    }

    // Starts `pose`'s set on the background pool over the shown set.
    fn launch(&mut self, mut set: Box<PoseSet>, pose: PoseInputs) {
        set.pose = Some((pose, Arc::clone(&self.shown)));
        set.started = Some(Instant::now());
        self.in_flight = true;
        let job = self.lane.job(set, PoseSet::stage);
        #[cfg(test)]
        if let Some(deferred) = &mut self.deferred {
            deferred.push(Box::new(job));
            return;
        }
        crate::thread_pools::spawn(job);
    }

    fn pose(&self, start: Instant, job_ms: Option<f64>) -> GradedPose {
        GradedPose {
            stage: match self.shown_error {
                Some(error) => Err(error),
                None => Ok(Arc::clone(&self.shown)),
            },
            stats: self.shown_stats,
            pending: self.pending_sets(),
            behind: (self.shown_generation > 0).then(|| self.latest - self.shown_generation),
            job_ms: job_ms.unwrap_or(0.0),
            frame_ms: start.elapsed().as_secs_f64() * 1000.0,
            rule: self.latest_rule,
            epoch: self.epoch,
        }
    }

    fn failed(&self, error: BakeError, start: Instant) -> GradedPose {
        GradedPose {
            stage: Err(error),
            stats: StageStats::default(),
            pending: self.pending_sets(),
            behind: None,
            job_ms: 0.0,
            frame_ms: start.elapsed().as_secs_f64() * 1000.0,
            rule: self.rule,
            epoch: self.epoch,
        }
    }
}

// Hands a set no longer shown to the scratch for its buffers' reuse (its tiles `kept` does not
// share). A set the renderer still holds is copied (its tiles' handles), as before.
fn retire(scratch: &mut StageScratch, old: Arc<TileStage>, kept: &TileStage) {
    let old = Arc::try_unwrap(old).unwrap_or_else(|shared| TileStage::clone(&shared));
    scratch.retire(old, kept);
}

/// What a road commit's undo record keeps of the graded tiles (T20 (a)): the tiles the commit
/// replaced, baked from the pre-commit inputs by a pure job after the commit. Undo adopts those
/// whose inputs equal the restored ones; until the job finishes it has none to adopt and bakes.
pub(crate) struct GradedUndo {
    kept: Arc<OnceLock<Result<TileStage, BakeError>>>,
    // The job that fills `kept`, and its start until the commit queues it after its last stage
    // (`ROAD-68-G` part 10, [`SimCore::take_kept_graded_tiles_start`]). Held for its drop: an
    // unstarted job dropped with the record never runs. Only tests read it (`wait`).
    #[cfg_attr(not(test), allow(dead_code))]
    job: PureJob<()>,
    start: Option<JobStart>,
    // The commit's level rule: undo selects by it, so the kept tiles meet its own.
    rule: LevelRule,
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

    /// Waits for the job, running it here if nobody started it (tests: a pool worker must
    /// never wait for a pure job).
    #[cfg(test)]
    pub(crate) fn wait(&self) -> Result<&TileStage, BakeError> {
        self.job.completion().wait();
        self.kept.wait().as_ref().map_err(|error| *error)
    }

    /// When the job started and finished, once it has.
    #[cfg(test)]
    pub(crate) fn span(&self) -> Option<(Instant, Instant)> {
        self.span.get().copied()
    }
}

impl SimCore {
    // The sites the graded bakes read, with the 512 m site index current. An edit's step: a
    // stale index is rebuilt whole whether or not a query readied its site part (`ALLOC-05`).
    fn prepare_graded_sites(&mut self) {
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);
    }

    // Stages the dirty tiles of the last roadbed install from `before` (`last_road_terrain_
    // changes`) over the live roadbed, sites and terrain, each at the level `rule` gives it,
    // adopting `held`'s matching tiles; the result becomes the core's last graded tiles and,
    // staged for a renderer's view, rides on the journal's record of the install, tagged with
    // the preview `epoch` whose tiles it adopted, for the renderer.
    fn stage_live_graded_tiles(
        &mut self,
        before: &RoadbedSystem,
        held: Option<&TileStage>,
        rule: &LevelRule,
        epoch: Option<u64>,
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
            rule,
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
                // Staged for a renderer's view, the tiles stand at the levels it holds them;
                // without one (a commit without a preview) its edit journal rebakes them. A
                // commit's tile adopted from the preview is the payload the renderer shows.
                if rule.view.is_some() {
                    let adopted = match (epoch, held) {
                        (Some(_), Some(held)) => (stage.tiles().iter())
                            .map(|tile| {
                                (held.get(tile.level, tile.tile))
                                    .is_some_and(|shown| Arc::ptr_eq(shown, &tile.payload))
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    self.attach_graded_install_tiles(Arc::clone(&stage), epoch, adopted);
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
        pose: Option<&GradedPose>,
    ) -> Option<GradedUndo> {
        let held = pose.and_then(|pose| pose.stage.as_ref().ok());
        let rule = pose.map(|pose| pose.rule).unwrap_or_default();
        self.stage_live_graded_tiles(
            &before,
            held.map(Arc::as_ref),
            &rule,
            pose.map(|pose| pose.epoch),
        );
        if self.benchmark_mode {
            return None;
        }
        // The tiles the commit replaced: the same dirty tiles over the pre-commit roadbed and
        // the same sites (a commit changes none), baked off the frame on the background pool.
        // That pool shares the CPUs with the commit's later stages, so the commit queues the
        // job after its last stage (`ROAD-68-G` part 10): queued here, the bake took their
        // cores, up to 9.7 ms of a 100k commit's agent pass.
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
        let (job, start) = PureJob::unstarted(move || {
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
            job,
            start: Some(start),
            rule,
            #[cfg(test)]
            span,
        })
    }

    /// The start of the kept tiles' job of the newest undo record, which the commit that made it
    /// queues after its last stage, beside its other background jobs (`ROAD-68-G` part 10).
    /// `None` when the record keeps no graded tiles (benchmark mode) or was started. O(1).
    pub(crate) fn take_kept_graded_tiles_start(&mut self) -> Option<JobStart> {
        self.undo_stack.back_mut()?.graded.as_mut()?.start.take()
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
        self.stage_live_graded_tiles(before, undo.kept(), &undo.rule, None);
    }
}
