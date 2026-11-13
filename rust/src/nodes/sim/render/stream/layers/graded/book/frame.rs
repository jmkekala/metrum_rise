// SPDX-License-Identifier: GPL-2.0-only

//! The graded renderer's frame, its preview frames and its counters in Rust (`RENDER-32-K` part
//! 7, `docs/terrain.md` "Streaming core"): the order of a frame's steps (the camera's re-level,
//! the edit batch, the result path, the changed patches, the cell set builds, the variant waits,
//! the lists and the hand-over, the material sync, the growth, the cell and wall rewrites), each
//! step's time for the perf channel, and every counter `terrain_graded_tiles.gd` kept, as its
//! `_frame`, `show_preview` and `release_preview` did them, in the same order (the traces pin it).
//!
//! What stays the renderer's is called back in its place, with the book unbound: the traced
//! simulation calls (`_sim_view`, `_request_patch_lists`, `_request_tiles`) and any take whose
//! effects need its Godot objects (`_apply_effects`). Bounds: the frame adds `O(1)` work and
//! calls to the steps (each with its own bound); the perf channel's parts are a dictionary built
//! only while the channel is on or on an edit batch's frame, as before.

use super::results::{Run, ScriptHost};
use super::*;
use crate::nodes::simulation_node::SimulationNode;
use godot::classes::Time;

/// The renderer's counters (`get_pool_counters`), cumulative from setup.
#[derive(Default)]
pub(super) struct Counters {
    pub(super) textures_created: i64,
    pub(super) images_created: i64,
    pub(super) meshes_created: i64,
    // The wall material the renderer made, handed at attach.
    pub(super) materials_made: i64,
    pub(super) pages_created: i64,
    pub(super) page_moves: i64,
    pub(super) page_growths: i64,
    pub(super) pool_exhausted: i64,
    results_deferred: i64,
    variant_waits: i64,
    evictions: i64,
    same_level_uploads: i64,
    bake_failures: i64,
    edits: i64,
    edit_adopted: i64,
    preview_page_waits: i64,
    edit_batches: i64,
    edit_tiles_last: i64,
    edit_install_ms_last: f64,
    edit_install_ms_max: f64,
    preview_install_ms_max: f64,
    preview_frames: i64,
    preview_install_ms_last: f64,
    preview_tiles_last: i64,
    install_ms_last: f64,
    install_ms_max: f64,
    stream_install_ms_max: f64,
    poll_ms_max: f64,
    pub(super) cell_sets_prewarmed: i64,
    pub(super) cell_prewarm_ms: f64,
    pub(super) ready_ms_max: i64,
    pub(super) ready_frames_max: i64,
    pub(super) unready_in_view: i64,
    pub(super) unready_in_view_max: i64,
    // The last preview frame's parts, every frame's parts while the perf channel is on, and the
    // last edit batch frame's parts (null until made).
    preview_parts: Option<VarDictionary>,
    frame_parts: Option<VarDictionary>,
    edit_frame_parts: Option<VarDictionary>,
}

/// One frame's own state: the camera last taken, the tiles re-levelled, the wanted tiles the
/// hand-over took and handed, the result path's split (nanoseconds in its polls, its tiles'
/// checks and placing, their effects, the other callbacks, and the tiles taken).
#[derive(Default)]
pub(super) struct FrameState {
    pub(super) camera: Option<Vector3>,
    relevelled: i64,
    pub(super) considered: i64,
    pub(super) handed: i64,
    split: [i64; 5],
}

fn now_us() -> i64 {
    Time::singleton().get_ticks_usec() as i64
}

fn ms(from_us: i64, to_us: i64) -> f64 {
    (to_us - from_us) as f64 / 1000.0
}

impl GradedTileBook {
    // The result path's and the edit path's counts since the last collection, into the counters
    // (as `_results_taken` and `_edits_taken` took them); the split into the frame's.
    fn collect_counts(&mut self) {
        let taken = self.result_counts();
        let edits = self.edit_counts();
        let failures = self.take_bake_failures();
        let counters = &mut self.counters;
        counters.results_deferred += taken[0];
        counters.variant_waits += taken[1];
        counters.evictions += taken[2];
        counters.same_level_uploads += taken[3];
        counters.poll_ms_max = counters.poll_ms_max.max(taken[4] as f64 / 1000.0);
        for (split, taken) in self.frame.split.iter_mut().zip(&taken[5..]) {
            *split += taken;
        }
        counters.edits += edits[0];
        counters.edit_adopted += edits[1];
        counters.preview_page_waits += edits[2];
        counters.pool_exhausted += edits[3];
        counters.bake_failures += failures;
    }

    // Applies the effects of the last calls: the book's own, or the renderer's `_apply_effects`
    // where one needs its objects.
    fn effects(book: &mut Gd<Self>, renderer: &mut Gd<RefCounted>) {
        let theirs = book.bind_mut().apply_own_effects();
        if theirs {
            renderer.call("_apply_effects", &[]);
        }
    }

    // The queued cell changes, then the walls, with their effects; the patches each rewrote.
    fn cells_and_walls(book: &mut Gd<Self>, renderer: &mut Gd<RefCounted>) -> (i64, i64, i64) {
        let cell_patches = book.bind().dirty_counts().x;
        book.bind_mut().apply_cell_changes();
        Self::effects(book, renderer);
        let cells_us = now_us();
        let wall_patches = book.bind().dirty_counts().y;
        book.bind_mut().apply_wall_changes();
        Self::effects(book, renderer);
        (i64::from(cell_patches), i64::from(wall_patches), cells_us)
    }
}

#[godot_api(secondary)]
impl GradedTileBook {
    /// One renderer frame (`_frame`): re-levels the tiles for a moved `camera` (the renderer's
    /// `_sim_view` tells the simulation first), installs the edit batch or polls and installs
    /// finished work within the install budget, follows the changed resident patches `news`
    /// (flat `(x, z)`), builds the queued cell sets, takes the variant waits, asks for lists and
    /// hands the wanted tiles over (the renderer's `_request_patch_lists` and `_request_tiles`),
    /// syncs the `changed_materials` keys' materials, advances the growth and rewrites the
    /// changed cells and walls. `simulation` polls without dictionaries (null: through the
    /// renderer's `_sim_poll`); `perf` records the frame's parts. Returns the tiles the edit
    /// batch installed. O(changes) a frame, as each step's bound.
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        mut book: Gd<GradedTileBook>,
        mut renderer: Gd<RefCounted>,
        simulation: Option<Gd<SimulationNode>>,
        camera: Vector3,
        news: PackedInt32Array,
        changed_materials: VarDictionary,
        perf: bool,
    ) -> i64 {
        let start_us = now_us();
        let uploads_before = {
            let mut this = book.bind_mut();
            this.collect_counts();
            let frame = &mut this.frame;
            frame.relevelled = 0;
            frame.considered = 0;
            frame.handed = 0;
            frame.split = [0; 5];
            this.reset_frame_counters();
            this.uploads()
        };
        // A camera move re-levels the tiles due first, so the tiles this frame adds take the
        // camera's position too. O(tiles due + buckets passed).
        if book.bind().frame.camera != Some(camera) {
            renderer.call("_sim_view", &[camera.to_variant()]);
            let mut this = book.bind_mut();
            this.frame.camera = Some(camera);
            this.frame.relevelled = this.move_camera(camera);
        }
        let relevel_us = now_us();
        // An edit's batch installs in a frame of its own: the poll that completes it leaves it
        // for the next frame, which installs it and polls nothing.
        let batch_tiles =
            Run::unbudgeted(ScriptHost::new(book.clone(), renderer.clone())).install_edit_batch();
        let batch_us = now_us();
        if batch_tiles == 0 {
            Run::budgeted(
                ScriptHost::timed(book.clone(), renderer.clone(), simulation),
                start_us,
            )
            .results();
        }
        let polled_us = now_us();
        {
            let counters = &mut book.bind_mut().counters;
            counters.stream_install_ms_max =
                counters.stream_install_ms_max.max(ms(batch_us, polled_us));
            counters.install_ms_last = ms(start_us, now_us());
            counters.install_ms_max = counters.install_ms_max.max(counters.install_ms_last);
        }
        for key in news.as_slice().as_chunks::<2>().0 {
            book.bind_mut().follow_news(Vector2i::new(key[0], key[1]));
            Self::effects(&mut book, &mut renderer);
        }
        let changes_us = now_us();
        Self::advance_builds(book.clone(), renderer.clone(), start_us);
        let builds_us = now_us();
        if book.bind().has_variant_waits() {
            Self::take_variant_waits(book.clone(), renderer.clone());
        }
        if book.bind().has_preview_waits() {
            Self::take_preview_waits(book.clone(), renderer.clone());
        }
        if book.bind().has_lists_wanted() {
            renderer.call("_request_patch_lists", &[]);
        }
        // The settle flags changed: parked blocks whose lists are in release their tiles.
        book.bind_mut().release_parked();
        if book.bind().has_wanted() {
            renderer.call("_request_tiles", &[]);
        }
        let residency_us = now_us();
        // A patch's cells follow its material's parameters (a changed patch's LOD step in its
        // news). O(changed materials).
        for (key, _) in changed_materials.iter_shared() {
            book.bind_mut().material_changed(key.to::<Vector2i>());
        }
        let watched_us = now_us();
        book.bind_mut().advance_growth();
        Self::effects(&mut book, &mut renderer);
        let (cell_patches, wall_patches, cells_us) =
            Self::cells_and_walls(&mut book, &mut renderer);
        let mut this = book.bind_mut();
        this.count_unready_in_view();
        if batch_tiles > 0 || perf {
            let end_us = now_us();
            this.collect_counts();
            let parts = this.frame_parts_of(
                [
                    start_us,
                    relevel_us,
                    batch_us,
                    polled_us,
                    changes_us,
                    builds_us,
                    residency_us,
                    watched_us,
                    cells_us,
                    end_us,
                ],
                cell_patches,
                wall_patches,
                uploads_before,
            );
            if perf {
                this.counters.frame_parts = Some(parts.clone());
            }
            if batch_tiles > 0 {
                this.counters.edit_frame_parts = Some(parts);
            }
        }
        if batch_tiles > 0 {
            // The edit's frame: its uploads, the cell rewrites they caused and the frame's changes.
            let counters = &mut this.counters;
            counters.edit_batches += 1;
            counters.edit_tiles_last = batch_tiles;
            counters.edit_install_ms_last = ms(start_us, now_us());
            counters.edit_install_ms_max = counters
                .edit_install_ms_max
                .max(counters.edit_install_ms_last);
        }
        batch_tiles
    }

    /// One road preview frame (`show_preview`'s work after its trace entry): `removed` (flat
    /// `(x, z)`) drop back to their installed tile, then each of `tiles` (packed as
    /// `poll_graded_tile_results` packs its tiles, the token the preview epoch) takes slots of
    /// its own, and the cells and walls follow in this frame. Returns the milliseconds it took.
    /// O(changed tiles x slot bytes).
    #[func]
    pub fn preview_changes(
        mut book: Gd<GradedTileBook>,
        mut renderer: Gd<RefCounted>,
        removed: PackedInt32Array,
        tiles: VarArray,
    ) -> f64 {
        if removed.is_empty() && tiles.is_empty() {
            return 0.0;
        }
        let start_us = now_us();
        let shown = tiles.len() as i64;
        Self::show_preview_frame(book.clone(), renderer.clone(), removed, tiles);
        let tiles_us = now_us();
        let (marks, flip_us) = {
            let this = book.bind();
            (this.state.cells.frame, this.cell_counts[FLIP_US])
        };
        let (cell_patches, wall_patches, cells_us) =
            Self::cells_and_walls(&mut book, &mut renderer);
        let end_us = now_us();
        let mut this = book.bind_mut();
        let frame = this.state.cells.frame;
        let since = |at: usize| (frame[at] - marks[at]) as i64;
        let mut parts = VarDictionary::new();
        // The frame's slot writes and drops, then the cell and wall rewrites they caused.
        parts.set("tiles_ms", ms(start_us, tiles_us));
        parts.set("cells_ms", ms(tiles_us, cells_us));
        parts.set("walls_ms", ms(cells_us, end_us));
        parts.set("cell_patches", cell_patches);
        parts.set("wall_patches", wall_patches);
        parts.set("cell_tiles", since(0));
        parts.set("flips", since(1));
        parts.set(
            "flip_ms",
            (this.cell_counts[FLIP_US] - flip_us) as f64 / 1000.0,
        );
        parts.set("written", since(2));
        parts.set("moved", since(3));
        let total = ms(start_us, end_us);
        let counters = &mut this.counters;
        counters.preview_install_ms_max = counters.preview_install_ms_max.max(total);
        counters.preview_frames += 1;
        counters.preview_install_ms_last = total;
        counters.preview_tiles_last = shown;
        counters.preview_parts = Some(parts);
        total
    }

    /// `release_preview`'s work after its trace entry: the preview tiles of epochs `first` to
    /// `last` drop, the installed tiles draw again, and the cells and walls follow.
    #[func]
    pub fn release_preview_frame(
        mut book: Gd<GradedTileBook>,
        mut renderer: Gd<RefCounted>,
        first: i64,
        last: i64,
    ) {
        Self::release_preview_epochs(book.clone(), renderer.clone(), first, last);
        Self::cells_and_walls(&mut book, &mut renderer);
    }

    /// The wanted tiles: wanted, parked on a page, or pending in Rust's queue (as of the last
    /// poll).
    #[func]
    pub fn wanted_total(&self) -> i64 {
        self.wanted_count() + self.tiles_pending()
    }

    /// The renderer's counters (`counters`): creations, uploads, the result and edit paths'
    /// counts, the install and preview times, the last preview, edit batch and perf frame's
    /// parts (`preview_parts`, `edit_frame_parts`, `frame_parts`), the cell sets' counts, the
    /// mask revisions given and the readiness latency. For tests, the probe and the benchmarks,
    /// which may read it every frame: O(1), no walk.
    #[func]
    pub fn counters(&mut self) -> VarDictionary {
        self.collect_counts();
        let mut out = VarDictionary::new();
        let cell = self.cell_counters();
        let counters = &self.counters;
        macro_rules! put {
            ($($name:ident),*) => { $(out.set(stringify!($name), counters.$name);)* };
        }
        put!(
            textures_created,
            images_created,
            meshes_created,
            pages_created,
            page_moves,
            page_growths,
            pool_exhausted,
            results_deferred,
            variant_waits,
            evictions,
            same_level_uploads,
            bake_failures,
            edits,
            edit_adopted,
            preview_page_waits,
            edit_batches,
            edit_tiles_last,
            edit_install_ms_last,
            edit_install_ms_max,
            preview_install_ms_max,
            preview_frames,
            preview_install_ms_last,
            preview_tiles_last,
            install_ms_last,
            install_ms_max,
            stream_install_ms_max,
            poll_ms_max,
            cell_sets_prewarmed,
            cell_prewarm_ms,
            ready_ms_max,
            ready_frames_max,
            unready_in_view,
            unready_in_view_max
        );
        for (name, parts) in [
            ("preview_parts", &counters.preview_parts),
            ("frame_parts", &counters.frame_parts),
            ("edit_frame_parts", &counters.edit_frame_parts),
        ] {
            out.set(name, parts.clone().unwrap_or_default());
        }
        out.set("uploads", self.uploads());
        out.set("upload_bytes", self.upload_bytes());
        out.set("upload_ms_max", self.upload_us_max() as f64 / 1000.0);
        out.set("walls_truncated", cell.get(3).unwrap_or(0));
        out.set("cell_variants_created", cell.get(4).unwrap_or(0));
        out.set("multimeshes_created", cell.get(5).unwrap_or(0));
        out.set(
            "materials_created",
            counters.materials_made + cell.get(6).unwrap_or(0),
        );
        out.set("cell_builds", cell.get(7).unwrap_or(0));
        out.set("mask_revision_bumps", self.state.cells.mask_bumps as i64);
        out
    }

    /// [`Self::counters`] with the pools' sizes and use ([`Self::pool_counters`]), the refusals,
    /// the layer limit and block side, and the cell sets held, queued and free
    /// (`get_pool_counters`, for tests and the memory record). O(levels' bound pages + resident
    /// patches).
    #[func]
    pub fn pool_report(&mut self) -> VarDictionary {
        let mut out = self.counters();
        for (name, value) in self.pool_counters().iter_shared() {
            out.set(name, value);
        }
        out.set(
            "refused",
            self.state.requests.refused.len() as i64 + self.lists_refused_count(),
        );
        out.set("layer_limit", i64::from(self.state.shape.layer_limit));
        out.set("block_patches", i64::from(self.state.shape.block_patches));
        let with_cells = self.patches_with_cells();
        out.set("patches_with_cells", with_cells);
        out.set("cell_builds_queued", self.builds_queued());
        out.set(
            "cell_sets_free",
            self.counters.cell_sets_prewarmed - with_cells,
        );
        let by_code: VarArray = self
            .state
            .levels
            .by_code()
            .iter()
            .map(|&count| (count as i64).to_variant())
            .collect();
        out.set("desired_by_level", by_code);
        out
    }

    /// The last perf frame's parts (the perf channel's, `terrain.gd`), empty before one.
    #[func]
    pub fn frame_parts(&self) -> VarDictionary {
        self.counters.frame_parts.clone().unwrap_or_default()
    }

    /// The camera the tiles' levels last took (XZ, its height above the ground as `y`; zero
    /// before a frame). For the stream benchmark's digests.
    #[func]
    pub fn level_camera(&self) -> Vector3 {
        self.frame.camera.unwrap_or(Vector3::ZERO)
    }

    /// The cell sets' prewarm at setup took `us` microseconds (T25's load cost).
    #[func]
    pub fn note_prewarm(&mut self, us: i64) {
        self.counters.cell_prewarm_ms = us as f64 / 1000.0;
    }

    /// Restarts the named maxima (`install_ms_max`, `unready_in_view_max`, `ready_ms_max`,
    /// `ready_frames_max`): a test or benchmark measures a span of frames alone.
    #[func]
    pub fn reset_maxima(&mut self, names: PackedStringArray) {
        let counters = &mut self.counters;
        for name in names.as_slice() {
            match name.to_string().as_str() {
                "install_ms_max" => counters.install_ms_max = 0.0,
                "unready_in_view_max" => counters.unready_in_view_max = 0,
                "ready_ms_max" => counters.ready_ms_max = 0,
                "ready_frames_max" => counters.ready_frames_max = 0,
                other => {
                    godot_error!("graded counters: no maximum named {other}");
                }
            }
        }
    }
}

impl GradedTileBook {
    // The perf channel's parts of a frame from its step times (`at`: start, re-level, batch,
    // poll, patch changes, builds, residency, watched, cells, end).
    fn frame_parts_of(
        &mut self,
        at: [i64; 10],
        cell_patches: i64,
        wall_patches: i64,
        uploads_before: i64,
    ) -> VarDictionary {
        let [
            start,
            relevel,
            batch,
            polled,
            changes,
            builds,
            residency,
            watched,
            cells,
            end,
        ] = at;
        let frame = self.state.cells.frame;
        let cell = self.cell_counts;
        let lists = self.frame_list_counts();
        let split = self.frame.split;
        let mut parts = VarDictionary::new();
        parts.set("relevel_ms", ms(start, relevel));
        parts.set("batch_ms", ms(relevel, batch));
        parts.set("poll_ms", ms(batch, polled));
        parts.set("residency_ms", ms(polled, residency));
        parts.set("watched_ms", ms(residency, watched));
        // residency_ms's parts: the changed patches, the cell set builds and the requests
        // (variant waits, lists, parked tiles, tile requests).
        parts.set("patch_changes_ms", ms(polled, changes));
        parts.set("builds_ms", ms(changes, builds));
        parts.set("requests_ms", ms(builds, residency));
        parts.set("cells_ms", ms(watched, cells));
        parts.set("cells_sync_ms", cell[SYNC_US] as f64 / 1000.0);
        parts.set("cells_walls_ms", cell[WALLS_US] as f64 / 1000.0);
        parts.set("walls_ms", ms(cells, end));
        parts.set("total_ms", ms(start, end));
        parts.set("cell_patches", cell_patches);
        parts.set("wall_patches", wall_patches);
        parts.set("cell_tiles", frame[0] as i64);
        parts.set("relevelled", self.frame.relevelled);
        parts.set("considered", self.frame.considered);
        parts.set("handed", self.frame.handed);
        parts.set("flips", frame[1] as i64);
        parts.set("flip_ms", cell[FLIP_US] as f64 / 1000.0);
        parts.set("lists", i64::from(lists.x));
        parts.set("lists_unchanged", i64::from(lists.y));
        parts.set("uploads", self.uploads() - uploads_before);
        parts.set("wanted", self.wanted_total());
        parts.set("installed", self.entry_count(false));
        // poll_ms's parts in the book (`RENDER-32-K` part 3).
        parts.set("result_poll_ms", split[0] as f64 / 1e6);
        parts.set("result_place_ms", split[1] as f64 / 1e6);
        parts.set("result_effects_ms", split[2] as f64 / 1e6);
        parts.set("result_callback_ms", split[3] as f64 / 1e6);
        parts.set("result_tiles", split[4]);
        parts
    }
}
