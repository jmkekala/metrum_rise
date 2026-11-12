// SPDX-License-Identifier: GPL-2.0-only

//! The streaming core's frame (`RENDER-32-E`, `docs/terrain.md` "Streaming core"): a node after
//! the camera in `Main.tscn` that, every frame, takes the products the background jobs finished,
//! follows the camera inside installed chunks and installs what waits under one streaming budget,
//! once in its `_process` and again just before the frame draws (`frame_pre_draw`), so a product
//! that landed meanwhile still draws this frame.
//!
//! Vegetation is its first layer: `vegetation.gd` registers its meshes and materials by RID and
//! its settings; `terrain.gd` pushes its resident patches and surface commits. The vegetation's
//! residency (`RENDER-32-F`) runs once a frame here, after `CameraNode` published the view: it
//! requests patches from the `SimulationNode`'s jobs, and shows, hides and frees installed patches
//! itself; no node is created per patch and no GDScript runs per patch. Every `RenderingServer`
//! call of the layer goes through [`GodotBackend`] on the main thread.
//!
//! The terrain and water patches (`RENDER-32-M`) run first in the frame's first pass: their
//! residency, levels of detail and prewarm are decided by [`PatchStream`] and executed one unit
//! at a time by `terrain.gd` and `water.gd` on their pooled nodes ([`ScriptExecutor`]), under
//! the same frame budget; the terrain's residency changes feed the water's wanted set and the
//! vegetation's tiers in the same pass.

use crate::nodes::camera_node::CameraNode;
use crate::nodes::sim::render::stream::backend::{GodotBackend, Role, Shadow};
use crate::nodes::sim::render::stream::layers::terrain::{
    Activation, PatchExecutor, PatchGrid, PatchKey as TerrainKey, PatchStream, PatchWork, Payload,
    TERRAIN_COSTS, TERRAIN_LIMITS, WATER_COSTS, WATER_LIMITS,
};
use crate::nodes::sim::render::stream::layers::vegetation::residency::{
    PatchJobs, PatchOrder, ResidencySettings, Standing, VegetationResidency,
};
use crate::nodes::sim::render::stream::layers::vegetation::{
    Delivery, MATERIAL_COUNT, MESH_COUNT, PatchKey, SLOT_LIMIT, SPARE_LIMIT, VegetationLayer,
    VegetationRules,
};
use crate::nodes::sim::render::stream::view::CameraSample;
use crate::nodes::sim::render::stream::schedule::{FrameBudget, MonotonicClock, STREAM_BUDGET_US};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use crate::nodes::sim::render::vegetation_product::{Batch, VegetationProduct};
use crate::nodes::simulation_node::SimulationNode;
use crate::nodes::simulation_node::vegetation_api::jobs::{self, Spawner};
use godot::classes::{INode, Node, RenderingServer, ResourceLoader, Script};
use godot::prelude::*;

/// Runs the streaming core's frame: pulls prepared products, follows the camera and installs
/// within the frame's streaming budget through the `RenderingServer` backend.
#[derive(GodotClass)]
#[class(base=Node)]
pub struct StreamNode {
    base: Base<Node>,
    backend: Option<GodotBackend>,
    vegetation: VegetationLayer,
    residency: VegetationResidency,
    spawner: Option<Spawner>,
    view: Option<CameraSample>,
    budget: FrameBudget,
    clock: MonotonicClock,
    // The perf channel: this frame's main-thread time and counters, recorded before the draw.
    perf: bool,
    perf_script: Option<Gd<Script>>,
    frame_us: f64,
    frame_installs: usize,
    frame_refreshed: usize,
    frame_copy_us: f64,
    frame_pulled: usize,
    frame_residency_us: f64,
    // Terrain and water patches (`RENDER-32-M`).
    terrain: PatchStream,
    water: PatchStream,
    terrain_scripts: Option<Gd<Node>>,
    water_scripts: Option<Gd<Node>>,
    full_world: bool,
    load_settling: bool,
    camera_far: f32,
    delta: f64,
    frame_patch_us: f64,
    frame_patch_units_us: f64,
    terrain_work: PatchWork,
    water_work: PatchWork,
}

#[godot_api]
impl INode for StreamNode {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            backend: None,
            vegetation: VegetationLayer::new(SLOT_LIMIT, SPARE_LIMIT),
            residency: VegetationResidency::default(),
            spawner: None,
            view: None,
            budget: FrameBudget::new(STREAM_BUDGET_US),
            clock: MonotonicClock::default(),
            perf: false,
            perf_script: None,
            frame_us: 0.0,
            frame_installs: 0,
            frame_refreshed: 0,
            frame_copy_us: 0.0,
            frame_pulled: 0,
            frame_residency_us: 0.0,
            terrain: PatchStream::new(TERRAIN_LIMITS, TERRAIN_COSTS),
            water: PatchStream::new(WATER_LIMITS, WATER_COSTS),
            terrain_scripts: None,
            water_scripts: None,
            full_world: false,
            load_settling: false,
            camera_far: f32::INFINITY,
            delta: 0.0,
            frame_patch_us: 0.0,
            frame_patch_units_us: 0.0,
            terrain_work: PatchWork::default(),
            water_work: PatchWork::default(),
        }
    }

    fn ready(&mut self) {
        let callable = self.base().callable("pre_draw");
        RenderingServer::singleton().connect("frame_pre_draw", &callable);
    }

    fn exit_tree(&mut self) {
        let callable = self.base().callable("pre_draw");
        let mut server = RenderingServer::singleton();
        if server.is_connected("frame_pre_draw", &callable) {
            server.disconnect("frame_pre_draw", &callable);
        }
        self.clear_vegetation();
    }

    fn process(&mut self, delta: f64) {
        self.delta = delta;
        self.budget.begin_frame();
        self.frame_us = 0.0;
        self.frame_installs = 0;
        self.frame_refreshed = 0;
        self.frame_copy_us = 0.0;
        self.frame_pulled = 0;
        self.frame_residency_us = 0.0;
        self.run_pass(true);
    }
}

#[godot_api]
impl StreamNode {
    /// The second pass of the frame, just before it draws; then the frame's perf record.
    #[func]
    fn pre_draw(&mut self) {
        self.run_pass(false);
        if self.perf {
            self.record_perf();
        }
    }

    /// Takes the vegetation's prepared products from `simulation`'s background jobs from now on.
    #[func]
    pub fn attach_simulation(&mut self, simulation: Gd<SimulationNode>) {
        self.spawner = Some(simulation.bind().vegetation_spawner());
    }

    /// Registers the vegetation's catalogue: the main scenario; `meshes`, near meshes per species
    /// and variant (three levels each: full, reduced, both; one-level species repeat theirs),
    /// then the impostor quad and the three distant hedge boxes; `materials`, the impostor
    /// material per canopy species then their shadow casters; and `rules` (near detail, detail
    /// fade, shadow handover, first landscape variant, first hedge variant). Returns false,
    /// keeping the previous catalogue, when malformed. Installed patches keep their objects.
    #[func]
    pub fn set_vegetation_catalogue(
        &mut self,
        scenario: Rid,
        meshes: Array<Rid>,
        materials: Array<Rid>,
        rules: PackedFloat64Array,
    ) -> bool {
        let (Some(rules), true, true, true) = (
            VegetationRules::from_lanes(rules.as_slice()),
            meshes.len() == MESH_COUNT,
            materials.len() == MATERIAL_COUNT,
            scenario.is_valid(),
        ) else {
            godot_error!("set_vegetation_catalogue: malformed catalogue");
            return false;
        };
        let backend = self
            .backend
            .get_or_insert_with(|| GodotBackend::new(scenario));
        backend.set_catalogue(
            meshes.iter_shared().collect(),
            materials.iter_shared().collect(),
            vec![StringName::from("hands_over")],
        );
        self.vegetation.set_rules(rules);
        true
    }

    /// Sets the water reflection's world (an invalid RID: none); installed impostors move into it.
    #[func]
    pub fn set_vegetation_reflection_scenario(&mut self, scenario: Rid) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        self.vegetation.drop_reflections(backend);
        backend.set_reflection_scenario(scenario.is_valid().then_some(scenario));
        self.vegetation.redraw_reflections(backend);
    }

    /// Shows or hides every vegetation patch (`vegetation.gd`'s `enabled`); a hidden layer's
    /// residency does no work.
    #[func]
    pub fn set_vegetation_enabled(&mut self, enabled: bool) {
        self.residency.set_enabled(enabled);
        if let Some(backend) = self.backend.as_mut() {
            self.vegetation.set_enabled(backend, enabled);
        }
    }

    /// The vegetation's runtime shadow toggle.
    #[func]
    pub fn set_vegetation_cast_shadows(&mut self, cast: bool) {
        if let Some(backend) = self.backend.as_mut() {
            self.vegetation.set_cast_shadows(backend, cast);
        }
    }

    /// Whether reused slots take fresh storage: true while the viewport draws motion vectors
    /// (FSR 2 or TAA), whose previous-frame half would otherwise hold the slot's last batch.
    #[func]
    pub fn set_vegetation_fresh_buffers(&mut self, fresh: bool) {
        self.vegetation.set_fresh_buffers(fresh);
    }

    /// Starts the vegetation over for a loaded world with `settings` (`vegetation.gd`'s
    /// `rebuild_from_simulation_state`): density, canopy near, crossfade begin, canopy far, bush
    /// range, rock range, subdivision (1-8), upload budget (0: the subdivision squared), the sun's
    /// shadow range, world width and depth. Frees every patch and drops every request. Returns
    /// false, changing nothing, when malformed or before the catalogue.
    #[func]
    pub fn set_vegetation_settings(&mut self, settings: PackedFloat64Array) -> bool {
        let Some(settings) = ResidencySettings::from_lanes(settings.as_slice()) else {
            godot_error!("set_vegetation_settings: malformed settings");
            return false;
        };
        let Some(backend) = self.backend.as_mut() else {
            return false;
        };
        self.vegetation.set_subdivision(settings.subdivision);
        let mut jobs = Jobs(self.spawner.as_mut());
        self.residency
            .reset(settings, &mut self.vegetation, backend, &mut jobs);
        self.recycle_spent();
        true
    }

    /// The terrain's render patch side, metres (`terrain.gd`'s `get_render_patch_span_m`; 0
    /// before a world).
    #[func]
    pub fn set_terrain_patch_span(&mut self, span: f64) {
        self.residency.set_terrain_span(span);
    }

    /// A terrain patch became resident or left (`terrain.gd`). `O(1)`.
    #[func]
    pub fn set_terrain_patch_resident(&mut self, key: Vector2i, resident: bool) {
        self.residency.set_resident([key.x, key.y], resident);
    }

    /// The terrain's residency revision moved (`terrain.gd`): the vegetation's tiers are evaluated
    /// next frame, as they were when the script compared the revision.
    #[func]
    pub fn revise_terrain_residency(&mut self) {
        self.residency.revise_residency();
    }

    /// Every terrain patch left (`terrain.gd` cleared its patches).
    #[func]
    pub fn clear_terrain_patches(&mut self) {
        self.residency.clear_resident();
    }

    /// A terrain patch committed a payload of surface `generation` (-1: none left): the vegetation
    /// patches it owns are checked for staleness. `O(1)`.
    #[func]
    pub fn set_terrain_surface_generation(&mut self, key: Vector2i, generation: i64) {
        self.residency
            .set_surface_generation([key.x, key.y], generation);
    }

    /// Whether the vegetation has work waiting: a first evaluation, a queued or requested patch.
    #[func]
    pub fn has_vegetation_pending_work(&self) -> bool {
        self.residency.has_pending_work()
    }

    /// The vegetation's counts: `wanted`, `patches` (drawn), `standins`, `cached`, `queue`,
    /// `requested`, `fine`, `coarse`, `trees`, `generated` (installs) and `generation_max_us`
    /// (the longest install). `O(drawn patches)` (the fine count).
    #[func]
    pub fn get_vegetation_metrics(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        let names = [
            "wanted",
            "patches",
            "standins",
            "cached",
            "queue",
            "requested",
            "fine",
            "coarse",
            "trees",
            "generated",
            "generation_max_us",
        ];
        for (name, value) in names.iter().zip(self.residency.counts()) {
            out.set(*name, value);
        }
        out
    }

    /// The wanted vegetation patches (the benchmark's wanted-to-drawn). `O(wanted)`.
    #[func]
    pub fn get_vegetation_wanted(&self) -> Array<Vector3i> {
        self.residency
            .wanted()
            .map(|[x, y, z]| Vector3i::new(x, y, z))
            .collect()
    }

    /// Every vegetation record by key (tests, probes): `tree_count`, `surface_generation`,
    /// `vegetation_generation`, `understory`, `near_band`, `shadow_caster`, `standing` (0 drawn,
    /// 1 stand-in, 2 cached), `stale` and `queued`. `O(records)`.
    #[func]
    pub fn get_vegetation_records(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        for (key, record, standing) in self.residency.records() {
            let mut entry = VarDictionary::new();
            entry.set("tree_count", record.tree_count);
            entry.set("surface_generation", record.surface);
            entry.set("vegetation_generation", record.vegetation);
            entry.set("understory", record.bands.understory);
            entry.set("near_band", record.bands.near_band);
            entry.set("shadow_caster", i64::from(record.bands.caster));
            entry.set(
                "standing",
                match standing {
                    Standing::Drawn => 0i64,
                    Standing::StandIn => 1,
                    Standing::Cached => 2,
                },
            );
            entry.set("stale", self.residency.stale(key, &record));
            entry.set("queued", self.residency.is_queued(key));
            out.set(Vector3i::new(key[0], key[1], key[2]), entry);
        }
        out
    }

    /// The bands `key` takes from the viewport camera now, as `Vector3i(understory, near band,
    /// shadow caster)` (1: yes; -1 each before the settings and the terrain span), with its
    /// record's hysteresis (tests building a patch's product themselves).
    #[func]
    pub fn get_vegetation_bands(&mut self, key: Vector3i) -> Vector3i {
        self.update_camera();
        let camera = self.view.map(|view| [view.eye[0], view.eye[2]]);
        self.residency
            .bands_for(patch_key(key), camera)
            .map_or(Vector3i::new(-1, -1, -1), |bands| {
                Vector3i::new(
                    i32::from(bands.understory),
                    i32::from(bands.near_band),
                    i32::from(bands.caster),
                )
            })
    }

    /// Queues `key` for a request check, as a band change or an edit would (tests).
    #[func]
    pub fn queue_vegetation_patch(&mut self, key: Vector3i) {
        self.residency.queue(patch_key(key));
    }

    /// Takes a drawn patch out of the drawn set, hidden into the cache or freed (tests of the
    /// restore path).
    #[func]
    pub fn retire_vegetation_patch(&mut self, key: Vector3i) {
        if let Some(backend) = self.backend.as_mut() {
            self.residency
                .retire(patch_key(key), &mut self.vegetation, backend);
        }
    }

    /// The view the core measured from this frame (`CameraNode`'s, or the viewport camera's
    /// position): `eye` and, from a `CameraNode`, `pivot`, `yaw`, `pitch`, `distance`. Empty
    /// without a camera.
    #[func]
    pub fn get_stream_view(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        if let Some(view) = self.view {
            out.set("eye", Vector3::from_array(view.eye));
            out.set("pivot", Vector3::from_array(view.pivot));
            out.set("yaw", view.yaw);
            out.set("pitch", view.pitch);
            out.set("distance", view.distance);
        }
        out
    }

    /// Frees every vegetation patch and drops what waits (the renderer's reset).
    #[func]
    pub fn clear_vegetation(&mut self) {
        if let Some(backend) = self.backend.as_mut() {
            self.vegetation.clear(backend);
        }
        self.recycle_spent();
    }

    /// Hands over a product built by `build_vegetation_product` for the patch `key` at `origin`
    /// with side `span` (tests and previews; the game's products come from the jobs). Returns
    /// false when the dictionary is malformed.
    #[func]
    pub fn deliver_vegetation_product(
        &mut self,
        key: Vector3i,
        serial: i64,
        product: VarDictionary,
        origin: Vector2,
        span: f32,
    ) -> bool {
        let Some(product) = product_from_dictionary(&product) else {
            godot_error!("deliver_vegetation_product: malformed product");
            return false;
        };
        self.vegetation.deliver(Delivery {
            key: patch_key(key),
            serial: serial as u64,
            product,
            origin: [origin.x, origin.y],
            span,
        });
        true
    }

    /// Takes what the jobs finished and installs everything waiting, outside the budget (tests
    /// and settle waits). Returns the installs.
    #[func]
    pub fn flush_vegetation(&mut self) -> i64 {
        self.pull();
        self.update_camera();
        let Some(backend) = self.backend.as_mut() else {
            return 0;
        };
        self.vegetation.refresh(backend);
        let mut budget = FrameBudget::new(STREAM_BUDGET_US);
        let installs =
            self.vegetation
                .install(backend, &mut budget, &self.clock, &|_, _| true, true);
        self.residency.take_installs(&mut self.vegetation, backend);
        self.recycle_spent();
        installs as i64
    }

    /// Whether `key` has an installed vegetation patch that draws.
    #[func]
    pub fn is_vegetation_drawn(&self, key: Vector3i) -> bool {
        self.vegetation.is_drawn(patch_key(key))
    }

    /// An installed patch as the layer drew it (tests): `serial`, `visible`, `detail`,
    /// `branched`, `has_caster`, `canopy_box`, `origin`, and `batches`, one dictionary each:
    /// `species`, `lod`, `variant`, `mesh`, `count`, `colors`, `custom`, `hands_over`, `multimesh`
    /// (its RID, for a probe's read-back), `multimesh_aabb`, and per instance
    /// (`main`, `caster`, `reflection`, when drawn) `origin`, `aabb`, `range`, `material`
    /// (-1: none), `shadow` (`RenderingServer.ShadowCastingSetting`) and `visible`. Empty when
    /// the patch has no installed version.
    #[func]
    pub fn get_vegetation_patch(&self, key: Vector3i) -> VarDictionary {
        let mut out = VarDictionary::new();
        let Some(patch) = self.vegetation.installed(patch_key(key)) else {
            return out;
        };
        let pool = self.vegetation.pool();
        let mut batches = VarArray::new();
        for batch in &patch.batches {
            let slot = pool.slot(batch.slot);
            let kind = slot.kind();
            let mut entry = VarDictionary::new();
            entry.set("species", i64::from(batch.species));
            entry.set("lod", i64::from(batch.lod));
            entry.set("variant", i64::from(batch.variant));
            entry.set("mesh", i64::from(kind.mesh));
            entry.set("colors", kind.colors);
            entry.set("custom", kind.custom);
            entry.set("hands_over", kind.parameter.is_some());
            entry.set("count", i64::from(batch.count));
            entry.set("multimesh", Rid::new(slot.multimesh));
            entry.set("multimesh_aabb", godot_aabb(slot.multimesh_aabb()));
            for (name, role) in [
                ("main", Role::Main),
                ("caster", Role::Caster),
                ("reflection", Role::Reflection),
            ] {
                if role == Role::Caster && !batch.caster {
                    continue;
                }
                if let Some(drawn) = slot.drawn(role) {
                    let mut state = VarDictionary::new();
                    state.set("origin", Vector3::from_array(drawn.origin));
                    state.set("aabb", godot_aabb(drawn.aabb));
                    state.set("range", Vector2::new(drawn.range[0], drawn.range[1]));
                    state.set("material", drawn.material.map_or(-1, i64::from));
                    state.set(
                        "shadow",
                        match drawn.shadow {
                            Shadow::Off => 0i64,
                            Shadow::On => 1,
                            Shadow::ShadowsOnly => 3,
                        },
                    );
                    state.set("visible", drawn.visible);
                    entry.set(name, state);
                }
            }
            batches.push(&entry.to_variant());
        }
        out.set("serial", patch.serial as i64);
        out.set("visible", patch.visible);
        out.set("detail", i64::from(patch.detail));
        out.set("branched", patch.branched);
        out.set("has_caster", patch.has_caster);
        out.set("canopy_box", godot_aabb(patch.canopy_box));
        let origin = patch.origin();
        out.set("origin", Vector2::new(origin[0], origin[1]));
        out.set("batches", batches);
        out
    }

    /// The streaming status: per layer (`vegetation`) installed, visible, pending, blocked,
    /// slots live, spare, created and freed, installs, and this frame's units with their
    /// predicted and measured microseconds and the longest unit.
    #[func]
    pub fn get_stream_status(&self) -> VarDictionary {
        let status = self.vegetation.status();
        let mut layer = VarDictionary::new();
        layer.set("installed", status.installed as i64);
        layer.set("visible", status.visible as i64);
        layer.set("pending", status.pending as i64);
        layer.set("blocked", status.blocked as i64);
        layer.set("slots_live", status.slots_live as i64);
        layer.set("slots_spare", status.slots_spare as i64);
        layer.set("slots_created", status.slots_created as i64);
        layer.set("slots_freed", status.slots_freed as i64);
        layer.set("installs_total", status.installs_total as i64);
        layer.set("frame_units", i64::from(status.frame.units));
        layer.set("frame_predicted_us", status.frame.predicted_us);
        layer.set("frame_measured_us", status.frame.measured_us);
        layer.set("frame_max_unit_us", status.frame.max_unit_us);
        let mut out = VarDictionary::new();
        out.set("vegetation", layer);
        out.set("budget_us", STREAM_BUDGET_US);
        out
    }

    /// Starts the terrain and water patches over on a world's patch grid (`terrain.gd`'s layout:
    /// `cols` x `rows` patches of `interval` cells of `cell` metres over `world`), executed by
    /// `terrain` and `water` (their `stream_*` methods). Nothing is wanted, resident or built.
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn reset_terrain_patches(
        &mut self,
        terrain: Gd<Node>,
        water: Option<Gd<Node>>,
        cols: i64,
        rows: i64,
        interval: i64,
        cell: f64,
        world: Vector2,
    ) {
        let grid = (cols > 0 && rows > 0 && interval > 0 && cell > 0.0).then_some(PatchGrid {
            cols: cols as i32,
            rows: rows as i32,
            interval: interval as i32,
            cell: cell as f32,
            world: [world.x, world.y],
        });
        self.terrain.reset(grid);
        self.water.reset(grid);
        self.terrain_scripts = Some(terrain);
        self.water_scripts = water;
    }

    /// The `terrain_force_full_world` and `terrain_force_lod1` debug modes.
    #[func]
    pub fn set_terrain_modes(&mut self, full_world: bool, force_lod1: bool) {
        self.full_world = full_world;
        self.terrain.set_force_lod1(force_lod1);
        self.water.set_force_lod1(force_lod1);
    }

    /// While a world load waits for the terrain to settle (the game is held, the frame time
    /// does not matter), the patches' units are not limited by the frame budget, as the graded
    /// renderer's work is not (`terrain.gd`'s `set_load_settling`).
    #[func]
    pub fn set_terrain_load_settling(&mut self, settling: bool) {
        self.load_settling = settling;
    }

    /// A terrain patch's payload news from `terrain.gd`'s poll: 1 arrived, 0 dropped (ask
    /// again), 2 refused. `O(1)`.
    #[func]
    pub fn terrain_payload(&mut self, key: Vector2i, news: i64) {
        self.terrain.payload([key.x, key.y], Payload::from_code(news));
    }

    /// A water patch's payload news from `water.gd`'s poll (as [`Self::terrain_payload`]).
    #[func]
    pub fn water_payload(&mut self, key: Vector2i, news: i64) {
        self.water.payload([key.x, key.y], Payload::from_code(news));
    }

    /// `terrain.gd` freed a patch's node itself (a failed upload): it is wanted again as a hole.
    #[func]
    pub fn terrain_patch_removed(&mut self, key: Vector2i) {
        self.terrain.patch_removed([key.x, key.y]);
        self.follow_terrain();
    }

    /// Starts the water's patches over (`water.gd` cleared its patches): it is executed by
    /// `water` and wants the terrain's resident patches. `O(grid)`.
    #[func]
    pub fn reset_water_patches(&mut self, water: Gd<Node>) {
        self.water.reset(self.terrain.grid());
        for key in self.terrain.resident_keys().collect::<Vec<_>>() {
            self.water.set_wanted(key, true);
        }
        self.water.set_halo(self.terrain.halo());
        self.water_scripts = Some(water);
    }

    /// `water.gd` freed a patch's node itself.
    #[func]
    pub fn water_patch_removed(&mut self, key: Vector2i) {
        self.water.patch_removed([key.x, key.y]);
        self.water.take_changes().for_each(drop);
        self.water.take_generations().for_each(drop);
    }

    /// Whether the terrain patches have work a settled view waits for: wanted patches not
    /// resident, resident ones not wanted, or level checks queued.
    #[func]
    pub fn has_terrain_pending_work(&self) -> bool {
        self.terrain.has_pending_work()
    }

    /// Whether the water patches have such work.
    #[func]
    pub fn has_water_pending_work(&self) -> bool {
        self.water.has_pending_work()
    }

    /// The terrain's prewarm square, `(min_x, min_z, max_x, max_z)` in patches (the wanted
    /// square and one patch beyond); `(0, 0, -1, -1)` before a view.
    #[func]
    pub fn get_terrain_halo(&self) -> Vector4i {
        self.terrain
            .halo()
            .map_or(Vector4i::new(0, 0, -1, -1), |[a, b, c, d]| {
                Vector4i::new(a, b, c, d)
            })
    }

    /// The terrain's and the water's patch counts: `<layer>_missing`, `_leaving`, `_lod_queue`,
    /// `_resident`.
    #[func]
    pub fn get_terrain_patch_counts(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        for (name, layer) in [("terrain", &self.terrain), ("water", &self.water)] {
            let [missing, leaving, queue] = layer.pending();
            out.set(format!("{name}_missing"), missing as i64);
            out.set(format!("{name}_leaving"), leaving as i64);
            out.set(format!("{name}_lod_queue"), queue as i64);
            out.set(format!("{name}_resident"), layer.resident_count() as i64);
        }
        out
    }

    /// Turns the perf channel's records on or off (`PerfDebug`, recorded as `vegetation`).
    #[func]
    pub fn set_perf_enabled(&mut self, enabled: bool) {
        self.perf = enabled;
    }
}

impl StreamNode {
    // The terrain and water patches' frame: the terrain's square from the view, its units, the
    // water's wanted set and halo from the terrain's residency, the water's units; the
    // vegetation's tiers follow the terrain's residency.
    fn patch_frame(&mut self) {
        self.frame_patch_us = 0.0;
        self.frame_patch_units_us = 0.0;
        let Some(grid) = self.terrain.grid() else {
            return;
        };
        let start = std::time::Instant::now();
        let units_before = self.budget.measured_us;
        let eye = self.view.map(|view| view.eye);
        let square = match eye {
            Some(eye) => grid.target(eye, self.camera_far, self.full_world),
            None => grid.target([0.0; 3], 0.0, true),
        };
        self.terrain.set_square(square);
        let planes = (self.terrain.has_pending_work() || self.water.has_pending_work())
            .then(|| self.camera_frustum())
            .flatten();
        let span = grid.span() as f32;
        let in_view = |key: TerrainKey| {
            planes.as_ref().is_none_or(|planes| {
                let centre = grid.centre(key);
                let half = span * 0.5;
                box_in_view(
                    planes,
                    [centre[0] - half, PATCH_LOW_M, centre[1] - half],
                    [centre[0] + half, PATCH_HIGH_M, centre[1] + half],
                )
            })
        };
        let unbudgeted = self.load_settling;
        if let Some(scripts) = self.terrain_scripts.as_mut() {
            let mut exec = ScriptExecutor(scripts);
            self.terrain_work = self.terrain.frame(
                eye,
                self.delta,
                &mut self.budget,
                &self.clock,
                unbudgeted,
                &in_view,
                &mut exec,
            );
        }
        self.follow_terrain();
        self.water.set_halo(self.terrain.halo());
        if let Some(scripts) = self.water_scripts.as_mut() {
            let mut exec = ScriptExecutor(scripts);
            self.water_work = self.water.frame(
                eye,
                self.delta,
                &mut self.budget,
                &self.clock,
                unbudgeted,
                &in_view,
                &mut exec,
            );
            self.water.take_changes().for_each(drop);
            self.water.take_generations().for_each(drop);
        }
        self.frame_patch_us = start.elapsed().as_secs_f64() * 1e6;
        self.frame_patch_units_us = self.budget.measured_us - units_before;
    }

    // The terrain's residency changes: the water's wanted set and the vegetation's tiers.
    fn follow_terrain(&mut self) {
        for (key, generation) in self.terrain.take_generations() {
            self.residency.set_surface_generation(key, generation);
        }
        let mut changed = false;
        for (key, resident) in self.terrain.take_changes() {
            self.water.set_wanted(key, resident);
            self.residency.set_resident(key, resident);
            changed = true;
        }
        if changed {
            self.residency.revise_residency();
        }
    }

    // One pass: the residency (the frame's first pass), pull, follow the camera, install within
    // what is left of the frame's budget, take the installs.
    fn run_pass(&mut self, residency: bool) {
        let start = std::time::Instant::now();
        self.update_camera();
        if residency {
            self.patch_frame();
        }
        if residency && let Some(backend) = self.backend.as_mut() {
            let eye = self.view.map(|view| view.eye);
            let mut jobs = Jobs(self.spawner.as_mut());
            self.residency
                .frame(eye, &mut self.vegetation, backend, &mut jobs);
            self.frame_residency_us = start.elapsed().as_secs_f64() * 1e6;
        }
        self.pull();
        let in_view = self.frustum();
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        self.frame_refreshed += self.vegetation.refresh(backend);
        let test = |low: [f32; 3], high: [f32; 3]| {
            in_view
                .as_ref()
                .is_none_or(|planes| box_in_view(planes, low, high))
        };
        self.frame_installs +=
            self.vegetation
                .install(backend, &mut self.budget, &self.clock, &test, false);
        self.residency.take_installs(&mut self.vegetation, backend);
        self.frame_copy_us += backend.take_copy_us();
        self.recycle_spent();
        // The patches' share is their own record (`terrain_stream`).
        let patches_us = if residency { self.frame_patch_us } else { 0.0 };
        self.frame_us += start.elapsed().as_secs_f64() * 1e6 - patches_us;
    }

    // Takes every finished product of a live request from the jobs; a busy one becomes an event.
    fn pull(&mut self) {
        let Some(spawner) = self.spawner.as_ref() else {
            return;
        };
        // Requests a busy core put back are tried again here at the latest, once a pass.
        jobs::pump(spawner);
        let mut jobs = spawner.jobs.lock().unwrap();
        while let Some(delivered) = jobs.poll() {
            self.frame_pulled += 1;
            match delivered.product {
                Some(product) => self.vegetation.deliver(Delivery {
                    key: delivered.key,
                    serial: delivered.serial,
                    origin: product.origin,
                    span: product.span,
                    product,
                }),
                None => self
                    .vegetation
                    .deliver_busy(delivered.key, delivered.serial),
            }
        }
    }

    // Returns installed and dropped products' storage to the jobs.
    fn recycle_spent(&mut self) {
        match self.spawner.as_ref() {
            Some(spawner) => {
                let mut jobs = spawner.jobs.lock().unwrap();
                for product in self.vegetation.take_spent() {
                    jobs.recycle(product);
                }
            }
            None => self.vegetation.take_spent().for_each(drop),
        }
    }

    // The view: `CameraNode`'s, published as it moved; another camera's position (tests).
    fn update_camera(&mut self) {
        let camera = self
            .base()
            .get_viewport()
            .and_then(|viewport| viewport.get_camera_3d());
        // The terrain's square is capped by the far plane (`get_visibility_cull_far_m`).
        self.camera_far = camera.as_ref().map_or(f32::INFINITY, |camera| camera.get_far());
        self.view = camera
            .map(|camera| match camera.try_cast::<CameraNode>() {
                Ok(node) => node.bind().published_view().unwrap_or_else(|| {
                    plain_view(node.get_global_position().to_array())
                }),
                Err(camera) => plain_view(camera.get_global_position().to_array()),
            });
        self.vegetation.set_camera(self.view.map(|view| view.eye));
    }

    // The camera's frustum planes, only while something waits for an install.
    fn frustum(&self) -> Option<[[f32; 4]; 6]> {
        if self.vegetation.pending() == 0 {
            return None;
        }
        self.camera_frustum()
    }

    fn camera_frustum(&self) -> Option<[[f32; 4]; 6]> {
        let camera = self.base().get_viewport()?.get_camera_3d()?;
        let planes = camera.get_frustum();
        if planes.len() != 6 {
            return None;
        }
        let mut out = [[0.0; 4]; 6];
        for (at, plane) in planes.iter_shared().enumerate() {
            out[at] = [plane.normal.x, plane.normal.y, plane.normal.z, plane.d];
        }
        Some(out)
    }

    fn record_perf(&mut self) {
        let script = self.perf_script.get_or_insert_with(|| {
            ResourceLoader::singleton()
                .load("res://scripts/core/perf_debug.gd")
                .and_then(|resource| resource.try_cast::<Script>().ok())
                .expect("perf_debug.gd")
        });
        let status = self.vegetation.status();
        let mut details = VarDictionary::new();
        details.set("installs", self.frame_installs as i64);
        details.set("install_ms", self.budget.measured_us / 1000.0);
        details.set("predicted_ms", self.budget.predicted_us / 1000.0);
        details.set("upload_max_ms", self.budget.max_unit_us / 1000.0);
        details.set(
            "overrun_max_ms",
            self.budget.max_overrun_us.max(0.0) / 1000.0,
        );
        details.set("poll_ms", self.frame_copy_us / 1000.0);
        details.set("pulled", self.frame_pulled as i64);
        details.set("refreshed", self.frame_refreshed as i64);
        details.set("stream_pending", status.pending as i64);
        details.set("stream_blocked", status.blocked as i64);
        details.set("slots_live", status.slots_live as i64);
        details.set("slots_spare", status.slots_spare as i64);
        details.set("stream_ms", self.frame_us / 1000.0);
        let work = self.residency.work();
        details.set("residency_ms", self.frame_residency_us / 1000.0);
        details.set("residency_changed", i64::from(work.evaluated));
        details.set("ring_cells", work.ring_cells as i64);
        details.set("band_checks", work.band_checks as i64);
        details.set("band_buckets", work.buckets as i64);
        details.set("stale_checks", work.stale_checks as i64);
        details.set("cache_checks", work.cache_checks as i64);
        details.set("requests", work.requests as i64);
        details.set("uploads", work.installs as i64);
        details.set("busy", work.busy as i64);
        details.set("standins_released", work.released as i64);
        let counts = self.residency.counts();
        details.set("wanted", counts[0]);
        details.set("patches", counts[1]);
        details.set("queue", counts[4]);
        details.set("requested", counts[5]);
        if self.terrain.grid().is_some() {
            let mut patches = VarDictionary::new();
            patches.set("units_ms", self.frame_patch_units_us / 1000.0);
            for (name, layer, work) in [
                ("terrain", &self.terrain, self.terrain_work),
                ("water", &self.water, self.water_work),
            ] {
                let [missing, leaving, queue] = layer.pending();
                patches.set(format!("{name}_missing"), missing as i64);
                patches.set(format!("{name}_leaving"), leaving as i64);
                patches.set(format!("{name}_lod_queue"), queue as i64);
                patches.set(format!("{name}_resident"), layer.resident_count() as i64);
                patches.set(format!("{name}_wanted_changes"), work.wanted_changes as i64);
                patches.set(format!("{name}_ring_cells"), work.ring_cells as i64);
                patches.set(format!("{name}_lod_checks"), work.lod_checks as i64);
                patches.set(format!("{name}_activations"), work.activations as i64);
                patches.set(format!("{name}_lod_changes"), work.lod_changes as i64);
                patches.set(format!("{name}_frees"), work.frees as i64);
                patches.set(format!("{name}_builds"), work.builds as i64);
                patches.set(format!("{name}_requests"), work.requests as i64);
                patches.set(format!("{name}_deferred"), work.deferred as i64);
            }
            // The patches' whole frame here, their units' script time included.
            script.call(
                "record",
                &[
                    "terrain_stream".to_variant(),
                    (self.frame_patch_us / 1000.0).to_variant(),
                    patches.to_variant(),
                ],
            );
        }
        script.call(
            "record",
            &[
                "vegetation".to_variant(),
                (self.frame_us / 1000.0).to_variant(),
                details.to_variant(),
            ],
        );
    }
}

fn patch_key(key: Vector3i) -> PatchKey {
    [key.x, key.y, key.z]
}

// A camera other than `CameraNode` (tests): its position only.
fn plain_view(eye: [f32; 3]) -> CameraSample {
    CameraSample {
        eye,
        pivot: eye,
        yaw: 0.0,
        pitch: 0.0,
        distance: 0.0,
    }
}

// The vegetation's jobs, or none (a stream node without a simulation requests nothing).
struct Jobs<'a>(Option<&'a mut Spawner>);

impl PatchJobs for Jobs<'_> {
    fn request(&mut self, order: &PatchOrder, settings: &ResidencySettings) -> Option<u64> {
        self.0.as_mut()?.request(order, settings)
    }

    fn cancel(&mut self, key: PatchKey) {
        if let Some(spawner) = self.0.as_mut() {
            spawner.cancel(key);
        }
    }

    fn clear(&mut self) {
        if let Some(spawner) = self.0.as_mut() {
            spawner.clear();
        }
    }

    fn generations(&mut self, known: i64, table: &mut Vec<([i32; 2], i64)>) -> Option<i64> {
        self.0.as_mut()?.generations(known, table)
    }
}

fn godot_aabb(aabb: Aabb) -> godot::builtin::Aabb {
    godot::builtin::Aabb::new(
        Vector3::from_array(aabb.position),
        Vector3::from_array(aabb.size),
    )
}

// Whether a box can be in the view: not wholly outside any frustum plane (normals point out).
fn box_in_view(planes: &[[f32; 4]; 6], low: [f32; 3], high: [f32; 3]) -> bool {
    planes.iter().all(|&[nx, ny, nz, d]| {
        // The corner farthest against the normal.
        let x = if nx > 0.0 { low[0] } else { high[0] };
        let y = if ny > 0.0 { low[1] } else { high[1] };
        let z = if nz > 0.0 { low[2] } else { high[2] };
        nx * x + ny * y + nz * z <= d
    })
}

// A product from the bridge's dictionary (`build_vegetation_product`'s packing).
fn product_from_dictionary(data: &VarDictionary) -> Option<VegetationProduct> {
    let batches: PackedInt32Array = data.get("batches")?.try_to().ok()?;
    let bounds: PackedFloat32Array = data.get("bounds")?.try_to().ok()?;
    let buffers: VarArray = data.get("buffers")?.try_to().ok()?;
    let tree_count: i64 = data.get("tree_count")?.try_to().ok()?;
    let canopy: godot::builtin::Aabb = data.get("canopy_box")?.try_to().ok()?;
    let (batches, bounds) = (batches.as_slice(), bounds.as_slice());
    if batches.len() % 8 != 0 || bounds.len() != batches.len() / 8 * 14 {
        return None;
    }
    let buffers: Vec<Vec<f32>> = buffers
        .iter_shared()
        .map(|buffer| {
            buffer
                .try_to::<PackedFloat32Array>()
                .map(|buffer| buffer.to_vec())
        })
        .collect::<Result<_, _>>()
        .ok()?;
    let aabb = |lanes: &[f32]| {
        Aabb::new(
            [lanes[0], lanes[1], lanes[2]],
            [lanes[3], lanes[4], lanes[5]],
        )
    };
    let mut out = Vec::with_capacity(batches.len() / 8);
    for (ints, floats) in batches.as_chunks::<8>().0.iter().zip(bounds.as_chunks::<14>().0) {
        let buffer = usize::try_from(ints[7])
            .ok()
            .filter(|&at| at < buffers.len())?;
        out.push(Batch {
            species: ints[0] as u8,
            lod: ints[1] as u8,
            variant: ints[2] as u8,
            proxy: ints[3] == 1,
            count: ints[4] as u32,
            colors: ints[5] == 1,
            custom: ints[6] == 1,
            buffer,
            mesh_bounds: aabb(&floats[0..6]),
            shared_bounds: aabb(&floats[6..12]),
            range: [floats[12], floats[13]],
        });
    }
    Some(VegetationProduct::from_parts(
        out,
        buffers,
        tree_count,
        Aabb::new(canopy.position.to_array(), canopy.size.to_array()),
    ))
}

// The lowest and highest ground a patch's view test assumes, metres: the box only orders work.
const PATCH_LOW_M: f32 = -500.0;
const PATCH_HIGH_M: f32 = 2500.0;

// Runs a patch layer's units on `terrain.gd` / `water.gd`: their `stream_request`,
// `stream_activate`, `stream_deactivate`, `stream_build` and `stream_set_lod`, one call a unit.
struct ScriptExecutor<'a>(&'a mut Gd<Node>);

impl PatchExecutor for ScriptExecutor<'_> {
    fn request(&mut self, keys: &[i32], prewarm: bool, news: &mut Vec<i32>) {
        let keys = PackedInt32Array::from(keys);
        let answers = self
            .0
            .call("stream_request", &[keys.to_variant(), prewarm.to_variant()]);
        if let Ok(answers) = answers.try_to::<PackedInt32Array>() {
            news.extend_from_slice(answers.as_slice());
        }
    }

    // The script answers `Vector2i(status, generation)`: status 0 done, 2 blocked, else not
    // ready.
    fn activate(&mut self, key: TerrainKey, step: u8) -> (Activation, i64) {
        let done = self.0.call(
            "stream_activate",
            &[Vector2i::new(key[0], key[1]).to_variant(), i64::from(step).to_variant()],
        );
        let done = done.try_to::<Vector2i>().unwrap_or(Vector2i::new(1, -1));
        let status = match done.x {
            0 => Activation::Done,
            2 => Activation::Blocked,
            _ => Activation::NotReady,
        };
        (status, i64::from(done.y))
    }

    fn deactivate(&mut self, key: TerrainKey) {
        self.0
            .call("stream_deactivate", &[Vector2i::new(key[0], key[1]).to_variant()]);
    }

    // The script answers the built node's surface generation, or -2 when nothing was built.
    fn build(&mut self, key: TerrainKey, step: u8) -> Option<i64> {
        let built = self.0.call(
            "stream_build",
            &[Vector2i::new(key[0], key[1]).to_variant(), i64::from(step).to_variant()],
        );
        built.try_to::<i64>().ok().filter(|&generation| generation >= -1)
    }

    fn set_lod(&mut self, key: TerrainKey, step: u8) {
        self.0.call(
            "stream_set_lod",
            &[Vector2i::new(key[0], key[1]).to_variant(), i64::from(step).to_variant()],
        );
    }
}
