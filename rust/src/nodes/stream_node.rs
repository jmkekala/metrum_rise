// SPDX-License-Identifier: GPL-3.0-or-later

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
//! vegetation's tiers in the same pass, and its resident entries (residency, mesh steps, surface
//! generations) the graded tile renderer's book (`RENDER-32-K` part 6).
//!
//! A load's road chunks (`RENDER-32-P`) hydrate after the patches in the first pass: a surface a
//! part onto a chunk's hidden mesh, then its swap (`RENDER-52` part 2), nearest first with the view
//! and the near ring first ([`RoadHydration`]), executed by `road_chunk_hydration.gd` through the
//! road tool's commit, under a hydration budget a frame.
//! The parked cars' chunks (`RENDER-32-P`) draw through pooled slots of their own backend
//! ([`ParkedLayer`]): `parked_cars.gd` hands each changed chunk's poses over and creates no node.
//! The lots' bay paint (`PARK-30`) draws through a second such layer, one slot a parking chunk.
//! While the zoning overlay shows, its ground masks' resident chunks are a 2 km ring about the
//! eye ([`ZoningRing`]), which the overlay moves to its camera as it asks for them.

use crate::nodes::camera_node::CameraNode;
use crate::nodes::sim::render::agent_frame::SharedCasterFeed;
use crate::nodes::sim::render::stream::backend::{GodotBackend, Role, Shadow, slot_census};
use crate::nodes::sim::render::stream::layers::backdrop::{
    BLOCK_PATCHES, BackdropLayer, COVER_STEP_CELLS, HEIGHT_STEP_CELLS,
};
use crate::nodes::sim::render::stream::layers::agent_casters::{
    AgentCasterLayer, AgentKind, CasterView, SLOT_LIMIT as AGENT_SLOT_LIMIT,
    SPARE_LIMIT as AGENT_SPARE_LIMIT,
};
use crate::nodes::sim::render::stream::layers::graded::book::GradedTileBook;
use crate::nodes::sim::render::stream::layers::parked::{
    PAINT_BUCKET, ParkedLayer, tier_cells,
};
use crate::nodes::sim::render::stream::layers::roads::{
    ROAD_HYDRATION_BUDGET_US, RoadExecutor, RoadHydration, RoadInstall, RoadKey, RoadPart,
    RoadWork,
};
use crate::nodes::sim::render::stream::layers::terrain::{
    Activation, LayerFrame, PatchExecutor, PatchGrid, PatchKey as TerrainKey, PatchStream,
    PatchWork, Payload, POST_LOAD_PATCH_BUDGET_US, Release, TERRAIN_COSTS, TERRAIN_LIMITS,
    UNIT_KIND_NAMES, UNIT_KINDS, WATER_COSTS, WATER_LIMITS, frame_layers, place_bytes, place_kinds,
    places,
};
use crate::nodes::sim::render::stream::layers::zoning::ZoningRing;
use crate::nodes::sim::render::stream::layers::vegetation::residency::{
    PREPARED_CAP_BYTES, PatchJobs, PatchOrder, ResidencySettings, Standing, VegetationResidency,
};
use crate::nodes::sim::render::stream::layers::vegetation::{
    Delivery, InstallWaits, MATERIAL_COUNT, MESH_COUNT, PatchKey, SLOT_LIMIT, SPARE_LIMIT,
    VegetationUnits, VegetationLayer, VegetationRules,
};
use crate::nodes::sim::render::stream::probe;
use crate::nodes::sim::render::stream::view::CameraSample;
use crate::nodes::sim::render::stream::schedule::{
    FrameBudget, UnitSource, MonotonicClock, OWNERS, Owner, STREAM_BUDGET_US, with_ledger,
};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use crate::nodes::sim::render::vegetation_product::{Batch, VegetationProduct};
use crate::nodes::simulation_node::SimulationNode;
use crate::nodes::simulation_node::async_backdrop::{self, BackdropSpawner};
use crate::nodes::simulation_node::vegetation_api::jobs::{self, Spawner};
use godot::classes::{
    DirectionalLight3D, Engine, INode, Node, RenderingServer, ResourceLoader, Script,
};
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
    // The pull's landed products, taken under the jobs' lock (reused storage).
    landed: Vec<jobs::Delivered>,
    // Product storage the jobs' spare limit refused, dropped outside their lock (reused).
    refused: Vec<VegetationProduct>,
    view: Option<CameraSample>,
    // This pass's account inside the frame's ledger (`RENDER-32-Q`): what every owner of the
    // frame spent, the node's own units; the node's own spend of the frame apart.
    budget: FrameBudget,
    ledger_frame: u64,
    frame_spent: [f64; 2],
    // The frame's whole install ledger as the node's last pass closed it: measured, predicted.
    frame_ledger: [f64; 2],
    // The perf channel's ledger at the node's last pass (`TERRAIN-41`): microseconds charged by
    // each owner, in `Owner` order, then the predicted total the admissions compared.
    frame_charged: [f64; OWNERS + 1],
    clock: MonotonicClock,
    // The perf channel: this frame's main-thread time and counters, recorded before the draw.
    perf: bool,
    perf_script: Option<Gd<Script>>,
    frame_us: f64,
    frame_installs: usize,
    frame_refreshed: usize,
    frame_copy_us: f64,
    // The frame's frees through the backend (the `free` class, AUDIT-stream finding 6), and the
    // pass's, charged to the ledger at its end.
    frame_free_us: f64,
    frame_free_pass_us: f64,
    // The frame's prepared-set walks (`RENDER-32-N`), measured.
    frame_prepare_us: f64,
    frame_pulled: usize,
    frame_residency_us: f64,
    // Terrain and water patches (`RENDER-32-M`).
    terrain: PatchStream,
    water: PatchStream,
    terrain_scripts: Option<Gd<Node>>,
    water_scripts: Option<Gd<Node>>,
    full_world: bool,
    load_settling: bool,
    // A world load's preload (`TERRAIN-59`, `set_terrain_preload`), and its time slice a frame.
    preload: Preload,
    preload_budget: FrameBudget,
    // The post-load window (`POST_LOAD_PATCH_BUDGET_US`): from a world reset until the patches
    // first settle, their units run under their own budget.
    post_load_window: bool,
    // Seconds the window has been open outside the load's hold (closed at `POST_LOAD_WINDOW_S`).
    post_load_s: f64,
    post_load_budget: FrameBudget,
    camera_far: f32,
    delta: f64,
    frame_patch_us: f64,
    frame_patch_units_us: f64,
    // The last patch frame's budget held a terrain or water hole back (`TERRAIN-41` L2).
    patch_holes_deferred: bool,
    terrain_work: PatchWork,
    water_work: PatchWork,
    // The base terrain's memory budget, bytes (`TERRAIN-41` B1; none: every patch fits), and
    // the extras the scripts report a layer (terrain, water): bytes of spare textures and water
    // meshes, and resource sets in their pools (each a place of its layer).
    terrain_budget: Option<u64>,
    patch_extras: [[i64; 2]; 2],
    // The releases since the patch grid was set, a layer (terrain, water; the debug view's).
    releases_total: [u64; 2],
    // A load's road chunk hydration (`RENDER-32-P`) and its executor, budget and frame.
    roads: RoadHydration,
    road_scripts: Option<Gd<Node>>,
    road_budget: FrameBudget,
    road_work: RoadWork,
    frame_road_us: f64,
    // The parked cars' slots (`RENDER-32-P`), once `parked_cars.gd` gave the scenario.
    parked: Option<ParkedDraw>,
    // The lots' bay paint (`PARK-30`), with the parked cars' scenario.
    paint: Option<ParkedDraw>,
    // The moving agents' shadow-only twins (`RENDER-15`, R47), once `agents.gd` attached them.
    agents: Option<AgentDraw>,
    // The zoning masks' ring (`RENDER-32-P`) with the chunk span and world size it is for.
    zoning: Option<(ZoningRing, f64, [f64; 2])>,
    // The graded tile renderer's book, which follows the terrain's resident entries.
    graded: Option<Gd<GradedTileBook>>,
    // The backdrop (`TERRAIN-41` D1): its sampling job's spawner, once a simulation is attached,
    // and its textures.
    backdrop: Option<BackdropSpawner>,
    backdrop_layer: BackdropLayer,
    // The backdrop's own backend (`TERRAIN-41` D2): its textures, its cells and the one-mesh,
    // three-parameter catalogue they draw with, apart from the vegetation's catalogue (which
    // `set_vegetation_catalogue` replaces whole). Made by `set_backdrop_drawing`.
    backdrop_backend: Option<GodotBackend>,
}

/// A world load's preload of the base terrain (`TERRAIN-59`): while the load holds the game,
/// the patches the terrain memory target keeps are built before gameplay is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preload {
    /// No preload: the load hands over once the near ring draws, the fill follows in the
    /// background (the tools' and tests' loads).
    Off,
    /// The view's own patches first; the fill is held, so the extras the places are counted
    /// from (the drawn patches' water meshes) are final before the keep rectangle is filled.
    View,
    /// The fill, under the preload's slice and without the vegetation's hold.
    Fill,
}

/// The preload's time slice a frame, microseconds (`TERRAIN-59`): the patch units run until
/// their measured or predicted time reaches it, then the frame returns to the engine, so the
/// window stays responsive (progress shown, quit possible) while frames are long. It protects
/// the window, not a result: what is built and in which order does not depend on it.
const PRELOAD_SLICE_US: f64 = 50_000.0;

// The agents' twins with their own backend (their own mesh and material catalogue), the feed
// of candidates from the agents' frame, the sun whose shadow range they follow, and the frame's
// time keeping them (the perf channel).
struct AgentDraw {
    backend: GodotBackend,
    layer: AgentCasterLayer,
    feed: SharedCasterFeed,
    sun: Gd<DirectionalLight3D>,
    frame_us: f64,
}

// The parked layer with its own backend (its own mesh catalogue, appended as buckets appear)
// and the borrowed poses of the chunk being installed (reused storage).
struct ParkedDraw {
    backend: GodotBackend,
    layer: ParkedLayer,
    poses: Vec<(u32, PackedFloat32Array)>,
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
            landed: Vec::new(),
            refused: Vec::new(),
            view: None,
            budget: FrameBudget::new(STREAM_BUDGET_US),
            ledger_frame: 0,
            frame_spent: [0.0; 2],
            frame_ledger: [0.0; 2],
            frame_charged: [0.0; OWNERS + 1],
            clock: MonotonicClock::default(),
            perf: false,
            perf_script: None,
            frame_us: 0.0,
            frame_installs: 0,
            frame_refreshed: 0,
            frame_copy_us: 0.0,
            frame_free_us: 0.0,
            frame_free_pass_us: 0.0,
            frame_prepare_us: 0.0,
            frame_pulled: 0,
            frame_residency_us: 0.0,
            terrain: PatchStream::new(TERRAIN_LIMITS, TERRAIN_COSTS),
            water: PatchStream::new(WATER_LIMITS, WATER_COSTS),
            terrain_scripts: None,
            water_scripts: None,
            full_world: false,
            load_settling: false,
            preload: Preload::Off,
            preload_budget: FrameBudget::new(PRELOAD_SLICE_US),
            post_load_window: false,
            post_load_s: 0.0,
            post_load_budget: FrameBudget::new(POST_LOAD_PATCH_BUDGET_US),
            camera_far: f32::INFINITY,
            delta: 0.0,
            frame_patch_us: 0.0,
            frame_patch_units_us: 0.0,
            patch_holes_deferred: false,
            terrain_work: PatchWork::default(),
            water_work: PatchWork::default(),
            terrain_budget: None,
            releases_total: [0; 2],
            patch_extras: [[0; 2]; 2],
            roads: RoadHydration::default(),
            road_scripts: None,
            road_budget: FrameBudget::new(ROAD_HYDRATION_BUDGET_US),
            road_work: RoadWork::default(),
            frame_road_us: 0.0,
            parked: None,
            paint: None,
            agents: None,
            zoning: None,
            graded: None,
            backdrop: None,
            backdrop_layer: BackdropLayer::default(),
            backdrop_backend: None,
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
        self.clear_road_hydration();
        self.clear_parked_chunks();
        if let Some(draw) = self.agents.as_mut() {
            draw.layer.reset(&mut draw.backend);
        }
        if let Some(backend) = self.backdrop_backend.as_mut() {
            self.backdrop_layer.clear(backend);
            self.backdrop_layer.free_cells(backend);
        }
    }

    fn process(&mut self, delta: f64) {
        self.delta = delta;
        self.budget.begin_frame();
        self.frame_spent = [0.0; 2];
        self.frame_charged = [0.0; OWNERS + 1];
        self.frame_us = 0.0;
        self.frame_installs = 0;
        self.frame_refreshed = 0;
        self.frame_copy_us = 0.0;
        self.frame_free_us = 0.0;
        self.frame_prepare_us = 0.0;
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
        self.keep_agent_casters();
        self.run_pass(false);
        if self.perf {
            self.record_perf();
        }
    }

    /// Takes the vegetation's prepared products from `simulation`'s background jobs from now on.
    #[func]
    pub fn attach_simulation(&mut self, simulation: Gd<SimulationNode>) {
        self.spawner = Some(simulation.bind().vegetation_spawner());
        self.backdrop = Some(simulation.bind().backdrop_spawner());
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
        if let Some(book) = self.graded.as_mut() {
            book.bind_mut().follow_generation([key.x, key.y], generation);
        }
    }

    /// Hands the terrain's resident entries to the graded tile renderer's `book` from now on
    /// (`RENDER-32-K` part 6): each activation, mesh step change, departure and surface commit of
    /// a terrain patch sets the book's entry as it happens. Attached at a world's setup, before
    /// any patch is resident; null detaches.
    #[func]
    pub fn attach_graded_book(&mut self, book: Option<Gd<GradedTileBook>>) {
        if book.is_some() && self.terrain.resident_count() > 0 {
            godot_error!("attach_graded_book: terrain patches are already resident");
        }
        self.terrain.set_followed(book.is_some());
        self.graded = book;
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

    /// What an owner installing upgrades ahead of this node may still spend of the frame's
    /// install ledger, microseconds (`RENDER-32-Q`; may be negative). `O(1)`.
    #[func]
    pub fn get_install_room_us(&self) -> f64 {
        with_ledger(Engine::singleton().get_process_frames(), |ledger| {
            ledger.upgrade_room_us()
        })
    }

    /// Charges a script owner's measured install work to the frame's ledger: `owner` 0 the
    /// terrain's dirty-patch commit, 2 the water's mesh pipeline, 4 the building sites
    /// (`schedule::Owner`'s order); others are ignored. `O(1)`.
    #[func]
    pub fn charge_install(&mut self, owner: i64, measured_us: f64) {
        let owner = match owner {
            0 => Owner::TerrainCommit,
            2 => Owner::Water,
            4 => Owner::BuildingSites,
            _ => return,
        };
        with_ledger(Engine::singleton().get_process_frames(), |ledger| {
            ledger.charge(owner, measured_us, measured_us)
        });
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

    /// The stream benchmark's near ring (`VEG-35` R0): of the wanted vegetation patches whose
    /// centre lies within `radius` metres of `centre` (x, z), how many there are and how many draw
    /// themselves. Benchmark only, `O(wanted)`.
    #[func]
    pub fn count_vegetation_drawn_near(&self, centre: Vector2, radius: f64) -> Vector2i {
        let mut counts = Vector2i::ZERO;
        for (key, [x0, z0, x1, z1], _) in self.residency.probe_wanted() {
            let dx = f64::from((x0 + x1) * 0.5 - centre.x);
            let dz = f64::from((z0 + z1) * 0.5 - centre.y);
            if dx * dx + dz * dz < radius * radius {
                counts.x += 1;
                counts.y += i32::from(self.vegetation.is_drawn(key));
            }
        }
        counts
    }

    /// The stream benchmark's drawn probe (`RENDER-32-N`): layer `name`'s installed set this
    /// frame as `stream::probe` records (`terrain`, `water`, `vegetation_coarse`, `vegetation_fine`,
    /// `terrain_map` (every patch of the map, `TERRAIN-41` D2), and through the attached book
    /// `graded` and `graded_tiles`); empty for another name. A terrain patch under its backdrop
    /// cell is covered. Benchmark only, `O(the layer's set)` (the terrain's and water's
    /// `O(grid)`).
    #[func]
    pub fn probe_stream_layer(&self, name: GString) -> PackedFloat32Array {
        let mut out = Vec::new();
        match name.to_string().as_str() {
            layer @ ("terrain" | "water") => {
                let stream = if layer == "terrain" { &self.terrain } else { &self.water };
                if let Some(grid) = stream.grid() {
                    for (key, resident, ready) in stream.probe_wanted() {
                        let [x, z, width, depth] = grid.rect(key);
                        // A terrain hole under a backdrop cell is covered (`TERRAIN-41` D2).
                        let covered = layer == "terrain" && self.backdrop_layer.cell_shown(key);
                        let flags = if resident { probe::DRAWN } else { 0 }
                            | if ready { probe::READY } else { 0 }
                            | if covered { probe::COVERED } else { 0 };
                        probe::push_chunk(&mut out, [key[0], key[1], 0], flags, [x, z, x + width, z + depth]);
                    }
                }
            }
            "terrain_map" => {
                // Every patch of the map, wanted or not (`TERRAIN-41` D2): drawn as a patch
                // (resident and drawing something, D3), or covered by its backdrop cell; in view
                // and neither is bare (sky).
                if let Some(grid) = self.terrain.grid() {
                    for z in 0..grid.rows {
                        for x in 0..grid.cols {
                            let [left, top, width, depth] = grid.rect([x, z]);
                            let flags = if self.backdrop_layer.patch_draws([x, z]) { probe::DRAWN } else { 0 }
                                | if self.backdrop_layer.cell_shown([x, z]) { probe::COVERED } else { 0 };
                            probe::push_chunk(&mut out, [x, z, 0], flags, [left, top, left + width, top + depth]);
                        }
                    }
                }
            }
            layer @ ("vegetation_coarse" | "vegetation_fine") => {
                let fine = layer == "vegetation_fine";
                for (key, rect, cached) in self.residency.probe_wanted() {
                    if (key[2] > 1) != fine {
                        continue;
                    }
                    let (drawn, covered, pending) = self.vegetation.probe_state(key);
                    let flags = [(drawn, probe::DRAWN), (covered, probe::COVERED), (pending || cached, probe::READY)]
                        .iter()
                        .filter(|(on, _)| *on)
                        .fold(0, |flags, (_, flag)| flags | flag);
                    probe::push_chunk(&mut out, key, flags, rect);
                }
            }
            "graded" => {
                if let Some(book) = self.graded.as_ref() {
                    return book.bind().probe_patches();
                }
            }
            "graded_tiles" => {
                if let Some(book) = self.graded.as_ref() {
                    return book.bind().probe_tiles();
                }
            }
            _ => {}
        }
        PackedFloat32Array::from(out.as_slice())
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
    /// predicted and measured microseconds and the longest unit; the vegetation's prepared set
    /// (`prepared`, `prepared_bytes`, `prepared_cap_bytes`, `preparing`, `prepared_over_cap`,
    /// `prepared_used`, `prepared_dropped`, `prepared_replaced`); the installs' waits and costs
    /// since setup (`VEG-35` R0, [`Self::take_vegetation_install_waits`]'s fields with `install_`
    /// before each).
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
        // The prepared set (`RENDER-32-N`): products held, their bytes and cap, prepares in
        // flight, evictions over the cap, products a request used, dropped and replaced.
        let [held, bytes, preparing, over_cap, used, dropped, replaced] = self.residency.prepared_counts();
        layer.set("prepared", held);
        layer.set("prepared_bytes", bytes);
        layer.set("prepared_cap_bytes", PREPARED_CAP_BYTES as i64);
        layer.set("preparing", preparing);
        layer.set("prepared_over_cap", over_cap);
        layer.set("prepared_used", used);
        layer.set("prepared_dropped", dropped);
        layer.set("prepared_replaced", replaced);
        let [waits, _] = self.vegetation.install_waits();
        set_install_waits(&mut layer, "install_", &waits);
        let mut out = VarDictionary::new();
        out.set("vegetation", layer);
        out.set("budget_us", STREAM_BUDGET_US);
        out
    }

    /// The backdrop's state (`TERRAIN-41` D1, the debug view's and the benchmark's): `built`
    /// (a product of the current world exists), `ready` (built, every texture uploaded, no
    /// sampling job running: the step a loading screen waits for), `blocks`, `block_bytes`,
    /// `product_bytes` (the system-memory copy), `texture_bytes` (the textures as created),
    /// `pending` (channels waiting for their upload), `uploads`, `uploaded_bytes`, `builds`,
    /// `build_ms` (the last build on its worker), `refreshes`, `refresh_ms`, `refresh_chunks`,
    /// `refresh_points`, `refresh_changed` (the last refresh), `covers` (land cover notes),
    /// `busy` (jobs that found the core held). `O(blocks)`.
    #[func]
    pub fn get_backdrop(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        let layer = &self.backdrop_layer;
        out.set("uploads", layer.uploads as i64);
        out.set("uploaded_bytes", layer.uploaded_bytes as i64);
        out.set("texture_bytes", layer.texture_bytes as i64);
        let Some(spawner) = self.backdrop.as_ref() else {
            out.set("built", false);
            out.set("ready", false);
            return out;
        };
        let world = spawner.shared.world();
        let state = spawner.shared.state.lock().unwrap();
        let product = state.product.as_ref().filter(|product| product.world == world);
        out.set("built", product.is_some());
        out.set(
            "ready",
            product.is_some_and(|product| !product.has_pending()) && !state.running,
        );
        if let Some(product) = product {
            out.set("blocks", product.blocks.len() as i64);
            out.set("block_bytes", product.layout.block_bytes() as i64);
            out.set("product_bytes", product.bytes() as i64);
            out.set("pending", product.pending_count() as i64);
        }
        let stats = state.stats;
        out.set("builds", stats.builds as i64);
        out.set("build_ms", stats.build_us / 1000.0);
        out.set("refreshes", stats.refreshes as i64);
        out.set("refresh_ms", stats.refresh_us / 1000.0);
        out.set("refresh_chunks", stats.refresh_chunks as i64);
        out.set("refresh_points", stats.refresh_points as i64);
        out.set("refresh_changed", stats.refresh_changed as i64);
        out.set("covers", stats.covers as i64);
        out.set("busy", stats.busy as i64);
        drop(state);
        self.backdrop_cells_status(&mut out);
        out
    }

    // The backdrop's cells (`TERRAIN-41` D2) for `get_backdrop`: the product's constants a script
    // lays the cells' materials out with (`block_patches`, `height_step_cells`,
    // `cover_step_cells`), the cells `cells_listed` (patches not drawn as patches), `cells_drawn`
    // and `cell_blocks` (blocks drawing), and the map's patches in the camera's view by what draws
    // them: `in_view_patches` (drawn as a patch: resident and drawing something, `TERRAIN-41`
    // D3), `in_view_cells` (a backdrop cell) and
    // `in_view_bare` (nothing: sky). `O(patches)`: a status read, not a frame's.
    fn backdrop_cells_status(&self, out: &mut VarDictionary) {
        out.set("block_patches", BLOCK_PATCHES as i64);
        out.set("height_step_cells", HEIGHT_STEP_CELLS as i64);
        out.set("cover_step_cells", COVER_STEP_CELLS as i64);
        let [listed, drawn, blocks] = self.backdrop_layer.cell_counts();
        out.set("cells_listed", listed as i64);
        out.set("cells_drawn", drawn as i64);
        out.set("cell_blocks", blocks as i64);
        let (Some(grid), Some(planes)) = (self.terrain.grid(), self.camera_frustum()) else {
            return;
        };
        let mut counts = [0i64; 3];
        for z in 0..grid.rows {
            for x in 0..grid.cols {
                let [left, top, width, depth] = grid.rect([x, z]);
                let low = [left, PATCH_LOW_M, top];
                let high = [left + width, PATCH_HIGH_M, top + depth];
                if !box_in_view(&planes, low, high) {
                    continue;
                }
                let at = if self.backdrop_layer.patch_draws([x, z]) {
                    0
                } else if self.backdrop_layer.cell_shown([x, z]) {
                    1
                } else {
                    2
                };
                counts[at] += 1;
            }
        }
        out.set("in_view_patches", counts[0]);
        out.set("in_view_cells", counts[1]);
        out.set("in_view_bare", counts[2]);
    }

    /// Builds the backdrop's product now and uploads it (`TERRAIN-41` D2; `terrain.gd` at a world's
    /// setup, after `set_backdrop_drawing`), so the cells draw from the load's first frame: the
    /// build on the background pool's workers, waited for here (about 1 ms on the shipped map, 4 ms
    /// at 59,904 m), the textures made at once (27 on the shipped map). Nothing when the sources
    /// cannot be read now (a busy core or a running job: the frame's own check builds it then).
    /// Returns whether the product of this world exists.
    #[func]
    pub fn build_backdrop(&mut self) -> bool {
        let Some(spawner) = self.backdrop.as_ref() else {
            return false;
        };
        let built = async_backdrop::run_now(spawner);
        if let (true, Some(backend)) = (built, self.backdrop_backend.as_mut())
            && let Ok(mut state) = spawner.shared.state.try_lock()
        {
            let world = spawner.shared.world();
            let product = state.product.as_mut().filter(|product| product.world == world);
            self.backdrop_layer
                .upload(product, backend, &mut self.budget, &self.clock, true);
        }
        built
    }

    /// Hands over what draws the backdrop's cells (`TERRAIN-41` D2, `terrain.gd` at a world's
    /// setup): `scenario`, the main world's; `mesh`, the cell's mesh (lattice indices in `x`/`z`,
    /// `y` -1 on the skirt); and `materials`, one material of the terrain shader's backdrop variant
    /// a block of the product (row-major, `get_backdrop()`'s `block_patches`), which take the
    /// blocks' textures as `heightmap`, `watermap` and `land_cover_texture`. The script keeps the
    /// materials alive. The backdrop's textures are made from the first call on. `O(blocks)`.
    #[func]
    pub fn set_backdrop_drawing(&mut self, scenario: Rid, mesh: Rid, materials: Array<Rid>) {
        if !scenario.is_valid() || !mesh.is_valid() {
            godot_error!("StreamNode.set_backdrop_drawing: no scenario or mesh");
            return;
        }
        let backend = self
            .backdrop_backend
            .get_or_insert_with(|| GodotBackend::new(scenario));
        // Its own catalogue: the cell mesh (0) and the channels' parameters (0, 1, 2).
        let names = ["heightmap", "watermap", "land_cover_texture"].map(StringName::from);
        backend.set_catalogue(vec![mesh], Vec::new(), names.to_vec());
        let materials = materials.iter_shared().map(|rid| rid.to_u64()).collect();
        self.backdrop_layer.set_drawing(0, 0, materials, backend);
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
        // Nothing is resident: every patch of the map draws its backdrop cell once its block's
        // textures exist (`TERRAIN-41` D2).
        self.backdrop_layer
            .reset_cells(grid, self.backdrop_backend.as_mut());
        self.releases_total = [0; 2];
        // A new world starts without a preload; the load flow asks for one after this.
        self.set_preload(Preload::Off);
        self.post_load_window = grid.is_some();
        self.post_load_s = 0.0;
        self.terrain_scripts = Some(terrain);
        self.water_scripts = water;
    }

    /// The base terrain's memory budget (`TERRAIN-41` B1), megabytes of 10^6 bytes; 0 or less:
    /// every patch fits (the default). Counted are the kept base terrain's texels (heights, land
    /// cover, water depth) and the extras the scripts report. It is a target, not a hard limit:
    /// the keep and release rectangles follow at the next frame, and the fill and the releases
    /// move the built set to it through their units under the frame's install ledger once the
    /// view waits for nothing (`TERRAIN-41` B2: the player's setting, changed at any time).
    #[func]
    pub fn set_terrain_budget_mb(&mut self, megabytes: f64) {
        self.terrain_budget = (megabytes > 0.0).then_some((megabytes * 1e6) as u64);
    }

    /// A layer's extras for the budget (`TERRAIN-41` B1), as totals: `layer` 0 terrain, 1 water;
    /// `bytes` of spare textures and water meshes its built patches hold; `pooled` resource sets
    /// in its pool (each counted as a place of the layer until freed). `O(1)`.
    #[func]
    pub fn set_patch_extras(&mut self, layer: i64, bytes: i64, pooled: i64) {
        if let Some(extras) = usize::try_from(layer).ok().and_then(|layer| self.patch_extras.get_mut(layer)) {
            *extras = [bytes.max(0), pooled.max(0)];
        }
    }

    /// The base terrain's budget now (`TERRAIN-41` B1 and B2, the debug view's and the
    /// benchmark's): the fields of the perf channel's record (`budget_status`), and `fill_left`
    /// (both layers' patches of their prewarm squares not yet built: the background fill a settled
    /// view does not wait for), `release_rect`, the last frame's install ledger `ledger_us`
    /// (measured, every owner) and `ledger_predicted_us`, and `ledger_budget_us`. `O(prewarm
    /// square)` for `fill_left`: a status read, not a frame's.
    #[func]
    pub fn get_terrain_budget(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        self.budget_status(&mut out);
        out.set("fill_left", (self.terrain.fill_left() + self.water.fill_left()) as i64);
        if let Some([a, b, c, d]) = self.terrain.release_rect() {
            out.set("release_rect", Vector4i::new(a, b, c, d));
        }
        out.set("ledger_us", self.frame_ledger[0]);
        out.set("ledger_predicted_us", self.frame_ledger[1]);
        out.set("ledger_budget_us", STREAM_BUDGET_US);
        out
    }

    // The budget's state by kind (`TERRAIN-41` B2's debug view and the perf record):
    // `budget_bytes` (-1: every patch fits), `counted_bytes` and its parts `height_bytes`,
    // `land_cover_bytes`, `water_depth_bytes` (built places by kind), `spare_bytes`,
    // `water_mesh_bytes`, `pooled_bytes` (the extras, `extra_bytes`); `places` (`N`), `ring`
    // (`K`), `overrun`; `wanted` (the wanted square's patches: the drawn set) and `wanted_side_m`;
    // per layer `{terrain,water}_drawn`, `_built`, `_kept` (built, not drawn) and
    // `_releases_total` since the grid was set. `O(1)`.
    fn budget_status(&self, out: &mut VarDictionary) {
        out.set("budget_bytes", self.terrain_budget.map_or(-1, |bytes| bytes as i64));
        out.set("counted_bytes", self.counted_bytes() as i64);
        out.set("extra_bytes", self.patch_extra_bytes() as i64);
        if let Some(grid) = self.terrain.grid() {
            let [height, cover, depth] = place_kinds(&grid);
            let [terrain_place, water_place] = place_bytes(&grid);
            let terrain = self.terrain.built_count() as u64;
            let water = self.water.built_count() as u64;
            out.set("height_bytes", (terrain * height) as i64);
            out.set("land_cover_bytes", (terrain * cover) as i64);
            out.set("water_depth_bytes", (water * depth) as i64);
            out.set("spare_bytes", self.patch_extras[0][0].max(0));
            out.set("water_mesh_bytes", self.patch_extras[1][0].max(0));
            let pooled = self.patch_extras[0][1].max(0) as u64 * terrain_place
                + self.patch_extras[1][1].max(0) as u64 * water_place;
            out.set("pooled_bytes", pooled as i64);
        }
        if let Some(keep) = self.terrain.keep() {
            out.set("places", i64::try_from(keep.places).unwrap_or(-1));
            out.set("ring", keep.ring as i64);
            out.set("overrun", keep.overrun);
        }
        // The wanted square (the drawn set: built patches outside it are kept hidden) and its
        // side, metres.
        if let (Some(grid), Some([a, b, c, d])) = (self.terrain.grid(), self.terrain.square()) {
            out.set("wanted", i64::from((c - a + 1) * (d - b + 1)));
            out.set("wanted_side_m", f64::from((c - a + 1).max(d - b + 1)) * grid.span());
        }
        let layers = [("terrain", &self.terrain), ("water", &self.water)];
        for (at, (name, layer)) in layers.into_iter().enumerate() {
            out.set(format!("{name}_drawn"), layer.resident_count() as i64);
            out.set(format!("{name}_built"), layer.built_count() as i64);
            out.set(
                format!("{name}_kept"),
                layer.built_count().saturating_sub(layer.resident_count()) as i64,
            );
            out.set(format!("{name}_releases_total"), self.releases_total[at] as i64);
        }
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
        if !settling {
            self.set_preload(Preload::Off);
        }
    }

    /// A world load's preload of the base terrain (`TERRAIN-59`), while the load holds the game
    /// (`set_terrain_load_settling`): `phase` 0 none (the load hands over as before), 1 the view's
    /// own patches first with the fill held, 2 the fill. During 1 and 2 the patch units run under
    /// the preload's time slice a frame (`PRELOAD_SLICE_US`, 50 ms) instead of without a limit, and
    /// each layer hands out up to `PRELOAD_REQUESTS` (256) payload requests a frame; during 2 the
    /// fill no longer waits for the vegetation. The hold's end turns it off.
    #[func]
    pub fn set_terrain_preload(&mut self, phase: i64) {
        self.set_preload(match phase {
            1 => Preload::View,
            2 => Preload::Fill,
            _ => Preload::Off,
        });
    }

    /// The preload's state (`TERRAIN-59`, the load flow's and its progress line's): `phase`
    /// (as [`Self::set_terrain_preload`]), `built` (terrain and water patches built), `left`
    /// (both layers' patches of the keep rectangle not built, and wanted ones built but not yet
    /// shown: `built + left` is the preload's whole), `in_step` (a frame has set the wanted square
    /// and no patch unit of either layer waits), and `done`: the fill phase has nothing left, the
    /// view is in step and the backdrop is ready (or there is none). `O(keep rectangle)`
    /// for `left`: read once a frame while the load holds the game.
    #[func]
    pub fn get_terrain_preload(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        let layers = [&self.terrain, &self.water];
        let built: usize = layers.iter().map(|layer| layer.built_count()).sum();
        let left: usize = layers.iter().map(|layer| layer.preload_left()).sum();
        // The view in step: a square was set by a frame (none before the load's first frame,
        // when nothing is pending yet) and no patch unit of either layer waits.
        let in_step = self.terrain.square().is_some()
            && !self.terrain.has_pending_work()
            && !self.water.has_pending_work();
        out.set("phase", self.preload as i64);
        out.set("built", built as i64);
        out.set("left", left as i64);
        out.set("in_step", in_step);
        out.set(
            "done",
            self.preload == Preload::Fill && left == 0 && in_step && self.backdrop_ready(),
        );
        out
    }

    /// Whether a world load's preload runs (`TERRAIN-59`): the scripts' payload polls take up to
    /// what the layers request then. `O(1)`.
    #[func]
    pub fn is_terrain_preloading(&self) -> bool {
        self.load_settling && self.preload != Preload::Off
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
        if let (Some(grid), Some(square)) = (self.terrain.grid(), self.terrain.square()) {
            self.water.set_halo(Some(grid.widen(square, WATER_LIMITS.halo)));
        }
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
    /// square: the terrain builds nothing beyond it); `(0, 0, -1, -1)` before a view.
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

    /// The terrain's (x) and the water's (y) patches marked requested and not yet answered by
    /// payload news (`TERRAIN-35`): each must be a request its script still tracks, so neither
    /// may exceed the script's count of tracked requests. `O(1)`.
    #[func]
    pub fn get_patch_requests(&self) -> Vector2i {
        let count = |layer: &PatchStream| i32::try_from(layer.requested_count()).unwrap_or(i32::MAX);
        Vector2i::new(count(&self.terrain), count(&self.water))
    }

    /// The vegetation's installs since the last call, then starts over (`VEG-35` R0, the stream
    /// benchmark's samples): `installs`, `wait_frames_sum`, `wait_frames_max` (process frames from a
    /// product's delivery to its install), `waited_over_bar` (more than 5 frames), `hole_in_view`,
    /// `hole_near`, `hole`, `upgrade_in_view`, `upgrade` (installs by the class of the pass that
    /// installed them), `stalled_passes` (passes that ended with a hole product waiting and no
    /// install), `predicted_us` and `measured_us` (the installs' units). `O(1)`.
    #[func]
    pub fn take_vegetation_install_waits(&mut self) -> VarDictionary {
        let mut out = VarDictionary::new();
        set_install_waits(&mut out, "", &self.vegetation.take_install_waits());
        out
    }

    /// The terrain's and the water's patch units by kind since the last call, then starts over
    /// (`TERRAIN-41` L1, the stream benchmark's samples): `<layer>_<kind>` (kinds `show`,
    /// `build`, `hide`, `step`, `prewarm`, `release`) -> `{count, mean_us, p50_us, p95_us, p99_us, max_us}`,
    /// the percentiles as one-microsecond bucket edges. `O(kinds x buckets)`.
    #[func]
    pub fn take_patch_unit_times(&mut self) -> VarDictionary {
        let mut out = VarDictionary::new();
        for (name, layer) in [("terrain", &mut self.terrain), ("water", &mut self.water)] {
            layer.take_unit_times(&mut |times| {
                for (kind, kind_name) in UNIT_KIND_NAMES.iter().enumerate() {
                    let [mean, max] = times.mean_max(kind);
                    let mut entry = VarDictionary::new();
                    entry.set("count", times.count(kind) as i64);
                    entry.set("mean_us", mean);
                    entry.set("p50_us", times.quantile(kind, 0.5));
                    entry.set("p95_us", times.quantile(kind, 0.95));
                    entry.set("p99_us", times.quantile(kind, 0.99));
                    entry.set("max_us", max);
                    out.set(format!("{name}_{kind_name}"), entry);
                }
            });
        }
        out
    }

    /// Starts a load's road chunk hydration (`RENDER-32-P`): the chunks `keys` (x, z pairs) on a
    /// grid of `span`-metre chunks whose chunk (0, 0) starts at `origin`, installed nearest first
    /// in parts (`RENDER-52` part 2) by `executor`: `stream_road_prepare(keys)` (the head of the
    /// order, x, z pairs, to encode), `stream_road_next_part(key) -> float` (the next part's
    /// predicted microseconds; -1 its surfaces not encoded yet, -2 gone) and
    /// `stream_road_part(key) -> int` (0 installed, 1 gone, 2 busy, 3 cancelled, 4 a part done);
    /// it hears `stream_road_hydration_done()` when none is left. Replaces any hydration running.
    /// `O(chunks)`.
    #[func]
    pub fn begin_road_hydration(
        &mut self,
        executor: Gd<Node>,
        span: f64,
        origin: Vector2,
        keys: PackedInt32Array,
    ) {
        let keys = keys.as_slice();
        self.roads.begin(
            span as f32,
            [origin.x, origin.y],
            keys.as_chunks::<2>().0.iter().copied(),
        );
        self.road_scripts = Some(executor);
        if self.roads.pending() == 0 {
            self.end_road_hydration();
        }
    }

    /// Drops the road chunks still to hydrate.
    #[func]
    pub fn clear_road_hydration(&mut self) {
        self.roads.clear();
        self.road_scripts = None;
    }

    /// Whether road chunks of a load are still to hydrate.
    #[func]
    pub fn has_road_hydration_pending(&self) -> bool {
        self.roads.pending() > 0
    }

    /// Road chunks still to hydrate any point of which lies within `radius` metres of the eye
    /// on the ground (the load's hand-over waits for none within its near ring). `O(pending)`.
    #[func]
    pub fn get_road_holes_near(&self, radius: f64) -> i64 {
        match self.view {
            Some(view) => self.roads.holes_near(view.eye, radius as f32) as i64,
            None => self.roads.pending() as i64,
        }
    }

    /// The road hydration's counts: `pending` and `installed` since it began.
    #[func]
    pub fn get_road_hydration_counts(&self) -> Vector2i {
        Vector2i::new(self.roads.pending() as i32, self.roads.installed() as i32)
    }

    /// Draws the parked cars in `scenario` from now on (`parked_cars.gd`, once; `RENDER-32-P`).
    /// A second call keeps the slots.
    #[func]
    pub fn set_parked_scenario(&mut self, scenario: Rid) {
        if self.parked.is_none() && scenario.is_valid() {
            let mut backend = GodotBackend::new(scenario);
            backend.set_catalogue(Vec::new(), Vec::new(), Vec::new());
            self.parked = Some(ParkedDraw {
                backend,
                layer: ParkedLayer::cars(),
                poses: Vec::new(),
            });
            let mut backend = GodotBackend::new(scenario);
            backend.set_catalogue(Vec::new(), Vec::new(), Vec::new());
            self.paint = Some(ParkedDraw {
                backend,
                layer: ParkedLayer::paint(),
                poses: Vec::new(),
            });
        }
    }

    /// Draws the moving agents' shadows from shadow-only twins in `scenario` from now on
    /// (`agents.gd`, once; `RENDER-15`, R47): they hold the agents `simulation`'s frames keep
    /// as candidates that can shade ground `sun`'s shadow reaches. A second call keeps them.
    #[func]
    pub fn attach_agent_casters(
        &mut self,
        simulation: Gd<SimulationNode>,
        sun: Gd<DirectionalLight3D>,
        scenario: Rid,
    ) {
        if self.agents.is_none() && scenario.is_valid() {
            let mut backend = GodotBackend::new(scenario);
            backend.set_catalogue(Vec::new(), Vec::new(), Vec::new());
            self.agents = Some(AgentDraw {
                backend,
                layer: AgentCasterLayer::new(AGENT_SLOT_LIMIT, AGENT_SPARE_LIMIT),
                feed: simulation.bind().agent_caster_feed(),
                sun,
                frame_us: 0.0,
            });
        }
    }

    /// Registers an agent bucket's twin: `kind` 0 walkers (by type), 1 moving cars (by bucket
    /// key), the bucket node's mesh, its material override (an invalid RID: the mesh's own) and
    /// its extra cull margin. Returns whether the twin will cast for the bucket; the bucket's
    /// node then casts nothing. A bucket registered again keeps its first registration.
    #[func]
    pub fn register_agent_caster(
        &mut self,
        kind: i64,
        key: i64,
        mesh: Rid,
        material: Rid,
        margin: f64,
    ) -> bool {
        let (Some(draw), Some(kind), Ok(key)) =
            (self.agents.as_mut(), agent_kind(kind), u8::try_from(key))
        else {
            return false;
        };
        let mesh = draw.backend.push_mesh(mesh);
        let material = material
            .is_valid()
            .then(|| draw.backend.push_material(material));
        draw.layer.register(kind, key, mesh, material, margin as f32);
        true
    }

    /// Forgets every twin of `kind` (0 walkers, 1 cars) and frees its slots: the bucket keys now
    /// name other meshes (a rebuilt vehicle catalogue, `RENDER-65`); register them again.
    #[func]
    pub fn unregister_agent_casters(&mut self, kind: i64) {
        if let (Some(draw), Some(kind)) = (self.agents.as_mut(), agent_kind(kind)) {
            draw.layer.unregister(&mut draw.backend, kind);
        }
    }

    /// Whether an agent kind's twins cast (`kind` 0 walkers, 1 cars; the shadow benchmark's
    /// caster classes, `RENDER-15`); the game never clears it.
    #[func]
    pub fn set_agent_cast_shadows(&mut self, kind: i64, cast: bool) {
        if let (Some(draw), Some(kind)) = (self.agents.as_mut(), agent_kind(kind)) {
            draw.layer.set_cast(&mut draw.backend, kind, cast);
        }
    }

    /// Whether the twins draw (the shadow benchmark's `agents_all` clears it while the bucket
    /// nodes cast instead); the game never clears it.
    #[func]
    pub fn set_agent_casters_enabled(&mut self, enabled: bool) {
        if let Some(draw) = self.agents.as_mut() {
            draw.layer.set_enabled(&mut draw.backend, enabled);
        }
    }

    /// The drawn twins (tests and the stream benchmark's census): `kind`, `key` and `count`
    /// (the agents each casts). `O(twins)`.
    #[func]
    pub fn get_agent_casters(&self) -> VarArray {
        let mut out = VarArray::new();
        for (kind, key, rows) in self.agents.iter().flat_map(|draw| draw.layer.drawn()) {
            let stride = if kind == AgentKind::Walkers { 16 } else { 12 };
            let mut item = VarDictionary::new();
            item.set("kind", kind as i64);
            item.set("key", i64::from(key));
            item.set("count", (rows.len() / stride) as i64);
            out.push(&item.to_variant());
        }
        out
    }

    /// Registers parked bucket `bucket`'s mesh (a look's tier, `look * 4 + tier`, `RENDER-65`),
    /// its bounds (`Mesh.get_aabb`; built by `parked_cars.gd` as the look is first met) and the
    /// band of each car's camera distance its tier draws in (its material's `tier_band_m`),
    /// `range_end` 0 for no end. The bounds give each drawn batch the bounds its range is
    /// measured from (`RENDER-48`, `RENDER-69`); the near tiers split a chunk by cell
    /// (`tier_cells`). A bucket registered again keeps its first mesh.
    #[func]
    pub fn register_parked_mesh(
        &mut self,
        bucket: i64,
        mesh: Rid,
        bounds: godot::builtin::Aabb,
        range_begin: f32,
        range_end: f32,
    ) {
        if let Some(draw) = self.parked.as_mut()
            && !draw.layer.has_mesh(bucket as u32)
        {
            let index = draw.backend.push_mesh(mesh);
            let bounds = Aabb::new(bounds.position.to_array(), bounds.size.to_array());
            let end = if range_end > 0.0 { range_end } else { f32::INFINITY };
            let tier = bucket as u32 % crate::nodes::sim::render::vehicle_catalogue::TIERS as u32;
            draw.layer.register(bucket as u32, index, bounds, [range_begin, end], tier_cells(tier));
        }
    }

    /// Whether the parked cars cast the sun's shadow (true unless a shadow benchmark's caster
    /// variant clears it, `RENDER-15`).
    #[func]
    pub fn set_parked_cast_shadows(&mut self, cast: bool) {
        if let Some(draw) = self.parked.as_mut() {
            draw.layer.set_cast_shadows(&mut draw.backend, cast);
        }
    }

    /// The draw census of a pooled layer's live slots, `"vegetation"`, `"parked"`, `"agents"` or
    /// `"paint"` (`RENDER-44`,
    /// the stream benchmark's census only; see `backend::slot_census`). Empty for another name
    /// or a layer not drawing. `O(slots)`.
    #[func]
    pub fn get_draw_census(&self, layer: GString) -> VarDictionary {
        match layer.to_string().as_str() {
            "vegetation" => slot_census(self.vegetation.pool()),
            "parked" => self
                .parked
                .as_ref()
                .map_or_else(VarDictionary::new, |draw| slot_census(draw.layer.pool())),
            "agents" => self
                .agents
                .as_ref()
                .map_or_else(VarDictionary::new, |draw| slot_census(draw.layer.pool())),
            "paint" => self
                .paint
                .as_ref()
                .map_or_else(VarDictionary::new, |draw| slot_census(draw.layer.pool())),
            _ => VarDictionary::new(),
        }
    }

    /// Whether reused parked slots take fresh storage (the viewport draws motion vectors).
    #[func]
    pub fn set_parked_fresh_buffers(&mut self, fresh: bool) {
        for draw in [self.parked.as_mut(), self.paint.as_mut()].into_iter().flatten() {
            draw.layer.set_fresh_buffers(fresh);
        }
    }

    /// Installs parked chunk `key` as `buckets` (vehicle look to its rows, 16 floats each, drawn
    /// in each of the look's registered tier buckets), whole; false when the pool could not take it
    /// (nothing changed: the caller offers it again). `O(buckets + poses)`.
    #[func]
    pub fn set_parked_chunk(&mut self, key: Vector2i, buckets: VarDictionary) -> bool {
        let Some(draw) = self.parked.as_mut() else {
            return false;
        };
        let ParkedDraw {
            backend,
            layer,
            poses,
        } = draw;
        poses.clear();
        for (bucket, rows) in buckets.iter_shared() {
            let (Ok(bucket), Ok(rows)) = (bucket.try_to::<i64>(), rows.try_to::<PackedFloat32Array>())
            else {
                continue;
            };
            // A look's cars go to each of its tiers' buckets (`RENDER-65`).
            for tier in 0..crate::nodes::sim::render::vehicle_catalogue::TIERS as i64 {
                let key = (bucket * 4 + tier) as u32;
                if layer.has_mesh(key) {
                    poses.push((key, rows.clone()));
                }
            }
        }
        let installed = layer.install(
            backend,
            (key.x, key.y),
            poses.iter().map(|(bucket, rows)| (*bucket, rows.as_slice())),
        );
        poses.clear();
        installed
    }

    /// Frees every parked slot and forgets every registered mesh: the buckets now name other
    /// meshes (a rebuilt vehicle catalogue, `RENDER-65`).
    #[func]
    pub fn clear_parked_meshes(&mut self) {
        if let Some(draw) = self.parked.as_mut() {
            draw.layer.forget_meshes(&mut draw.backend);
        }
    }

    /// Frees every parked slot and every paint slot (a world replaced; the renderer leaving).
    #[func]
    pub fn clear_parked_chunks(&mut self) {
        for draw in [self.parked.as_mut(), self.paint.as_mut()].into_iter().flatten() {
            draw.layer.reset(&mut draw.backend);
        }
    }

    /// Registers the bay paint's line quad (`PARK-30`; built by `parked_cars.gd` with the road
    /// marking material) and its bounds. A second registration keeps the first mesh.
    #[func]
    pub fn register_lot_paint_mesh(&mut self, mesh: Rid, bounds: godot::builtin::Aabb) {
        if let Some(draw) = self.paint.as_mut()
            && !draw.layer.has_mesh(PAINT_BUCKET)
        {
            let index = draw.backend.push_mesh(mesh);
            let bounds = Aabb::new(bounds.position.to_array(), bounds.size.to_array());
            draw.layer.register(PAINT_BUCKET, index, bounds, [0.0, f32::INFINITY], 1);
        }
    }

    /// Installs a bay paint payload (`SimulationNode.get_lot_paint`, `PARK-30`): with `full`,
    /// every paint slot is freed first; each listed chunk then draws its `rows` (12 floats a
    /// line), or nothing when they are empty. False when a chunk was refused (the pool full) or
    /// the paint has no mesh yet: the renderer asks for the same revision again.
    /// `O(listed chunks + their rows)`.
    #[func]
    pub fn apply_lot_paint(&mut self, payload: VarDictionary) -> bool {
        let Some(draw) = self.paint.as_mut() else {
            return false;
        };
        if !draw.layer.has_mesh(PAINT_BUCKET) {
            return false;
        }
        if payload.get("full").is_some_and(|full| full.try_to::<bool>().unwrap_or(false)) {
            draw.layer.reset(&mut draw.backend);
        }
        let chunks = payload.get("chunks").map(|chunks| chunks.try_to::<VarArray>());
        let Some(Ok(chunks)) = chunks else {
            return true;
        };
        let mut complete = true;
        for chunk in chunks.iter_shared() {
            let Ok(chunk) = chunk.try_to::<VarDictionary>() else {
                continue;
            };
            let (Some(Ok(key)), Some(Ok(rows))) = (
                chunk.get("chunk").map(|key| key.try_to::<Vector2i>()),
                chunk.get("rows").map(|rows| rows.try_to::<PackedFloat32Array>()),
            ) else {
                continue;
            };
            complete &= draw.layer.install(
                &mut draw.backend,
                (key.x, key.y),
                std::iter::once((PAINT_BUCKET, rows.as_slice())),
            );
        }
        complete
    }

    /// The drawn parked batches (tests and the stream benchmark's digests): `chunk`, `bucket`,
    /// `cell` (a near tier's cell of the chunk, row-major; -1 for a bucket drawn whole,
    /// `RENDER-69`), `count` (the poses drawn) and `multimesh` (its RID; its first `count`
    /// instances are the batch's poses). `O(batches)`.
    #[func]
    pub fn get_parked_drawn(&self) -> VarArray {
        let mut out = VarArray::new();
        let Some(draw) = self.parked.as_ref() else {
            return out;
        };
        for (chunk, bucket, cell, slot) in draw.layer.drawn() {
            let mut item = VarDictionary::new();
            item.set("chunk", Vector2i::new(chunk.0, chunk.1));
            item.set("bucket", i64::from(bucket));
            item.set("cell", if cell == u16::MAX { -1 } else { i64::from(cell) });
            let slot = draw.layer.pool().slot(slot);
            item.set("multimesh", Rid::new(slot.multimesh));
            item.set("count", i64::from(slot.visible_count().unwrap_or(0)));
            out.push(&item.to_variant());
        }
        out
    }

    /// The parked pool's `slots_live`, `slots_spare`, `slots_created` and `slots_freed`.
    #[func]
    pub fn get_parked_slots(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        if let Some(draw) = self.parked.as_ref() {
            let pool = draw.layer.pool();
            let (created, freed) = pool.churn();
            out.set("slots_live", pool.live() as i64);
            out.set("slots_spare", pool.spares() as i64);
            out.set("slots_created", created as i64);
            out.set("slots_freed", freed as i64);
        }
        out
    }

    /// Moves the zoning masks' ring of `span`-metre chunks over a world of `world` metres about
    /// its centre to `eye` (a new ring for another span or world) and returns its revision, which
    /// advances whenever its resident chunks change. `O(ring boundary chunks)` for a move,
    /// `O(1)` for a still eye.
    #[func]
    pub fn update_zoning_ring(&mut self, eye: Vector3, span: f64, world: Vector2) -> i64 {
        if span <= 0.0 {
            return -1;
        }
        let world = [f64::from(world.x), f64::from(world.y)];
        if self
            .zoning
            .as_ref()
            .is_none_or(|(_, at_span, at_world)| *at_span != span || *at_world != world)
        {
            self.zoning = Some((ZoningRing::new(span, world), span, world));
        }
        let (ring, _, _) = self.zoning.as_mut().expect("made above");
        ring.update([f64::from(eye.x), f64::from(eye.z)]);
        ring.revision() as i64
    }

    /// The zoning ring's resident chunks, nearest first, as x, z pairs. `O(chunks)`.
    #[func]
    pub fn get_zoning_ring(&mut self) -> PackedInt32Array {
        let mut out = PackedInt32Array::new();
        if let Some((ring, _, _)) = self.zoning.as_mut() {
            for [x, z] in ring.resident() {
                out.push(x);
                out.push(z);
            }
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
    // Sets the preload's phase on the node and both patch layers (their request limit).
    fn set_preload(&mut self, preload: Preload) {
        self.preload = preload;
        self.terrain.set_preload(preload != Preload::Off);
        self.water.set_preload(preload != Preload::Off);
    }

    // Whether the backdrop has nothing left to build or upload (`get_backdrop`'s `ready`), or
    // there is none (a scene without a simulation). `O(1)`.
    fn backdrop_ready(&self) -> bool {
        let Some(spawner) = self.backdrop.as_ref() else {
            return true;
        };
        let world = spawner.shared.world();
        // A job holding the state is not ready yet: asked again next frame, never waited for.
        let Ok(state) = spawner.shared.state.try_lock() else {
            return false;
        };
        let product = state.product.as_ref().filter(|product| product.world == world);
        product.is_some_and(|product| !product.has_pending()) && !state.running
    }

    // The extras' bytes (`TERRAIN-41` B1): reported bytes, and each pooled set a place of its
    // layer. `O(1)`.
    fn patch_extra_bytes(&self) -> u64 {
        let Some(grid) = self.terrain.grid() else {
            return 0;
        };
        let place = place_bytes(&grid);
        (0..2)
            .map(|layer| self.patch_extras[layer][0] as u64 + self.patch_extras[layer][1] as u64 * place[layer])
            .sum()
    }

    // The bytes the budget counts: the built places of both layers and the extras. `O(1)`.
    fn counted_bytes(&self) -> u64 {
        let Some(grid) = self.terrain.grid() else {
            return 0;
        };
        let [terrain, water] = place_bytes(&grid);
        self.terrain.built_count() as u64 * terrain
            + self.water.built_count() as u64 * water
            + self.patch_extra_bytes()
    }

    // The terrain and water patches' frame: the terrain's square from the view, its units, the
    // water's wanted set and halo from the terrain's residency, the water's units; the
    // vegetation's tiers follow the terrain's residency. The vegetation's waiting products take
    // their place in the patch units' order (`VEG-35` R1: holes among the holes, replacements among
    // the level changes); returns whether its cursor ran (the caller ends it and hands its
    // installs to the residency).
    fn patch_frame(&mut self) -> bool {
        self.frame_patch_us = 0.0;
        self.frame_patch_units_us = 0.0;
        self.patch_holes_deferred = false;
        let Some(grid) = self.terrain.grid() else {
            return false;
        };
        let start = std::time::Instant::now();
        let units_before = self.budget.measured_us;
        let eye = self.view.map(|view| view.eye);
        let square = match eye {
            Some(eye) => grid.target(eye, self.camera_far, self.full_world),
            None => grid.target([0.0; 3], 0.0, true),
        };
        self.terrain.set_square(square);
        // The budget's places (`TERRAIN-41` B1): the terrain's keep and release rectangles; the
        // water keeps and releases with them. `O(K)` when the places changed, nothing otherwise.
        let budget_places = places(&grid, self.terrain_budget, self.patch_extra_bytes());
        self.terrain.set_places(Some(budget_places));
        let water_ring = self.terrain.keep().map_or(0, |keep| keep.ring).max(WATER_LIMITS.halo);
        self.water.set_release(self.terrain.release_rect());
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
        // A load's preload (`TERRAIN-59`) runs the hold's units under its time slice; the hold
        // without one runs them unbudgeted.
        let preload = self.load_settling && self.preload != Preload::Off;
        let unbudgeted = self.load_settling && !preload;
        // In the post-load window the patch units run under their own budget, the stream node's
        // frame budget otherwise (vegetation shares that one).
        let window = self.post_load_window && !self.load_settling;
        if window {
            self.post_load_budget.begin_frame();
        }
        if preload {
            self.preload_budget.begin_frame();
        }
        let window_before = self.post_load_budget.measured_us;
        // Level changes keep the ledger's reserve while holes waited (none in the window or the
        // preload).
        let reserve = if window || preload {
            0.0
        } else {
            with_ledger(self.ledger_frame, |ledger| ledger.reserve_us())
        };
        self.water.set_upgrade_reserve(reserve);
        self.terrain.set_upgrade_reserve(reserve);
        // One order across the two layers (`TERRAIN-41` L1): the water's holes, the terrain's,
        // then both layers' level changes, hides, the budget's releases and prewarm (the order is
        // stated at `frame_layers`). The water's holes are terrain
        // patches resident since the last frame and its units are cheap, so a shore draws a frame
        // after its ground, as the scripts drew it, and the water's first draw (its shader's first
        // use) falls inside the load like the terrain's.
        self.water
            .set_halo(self.terrain.square().map(|square| grid.widen(square, water_ring)));
        // A water patch whose ground left the terrain's square takes no level change: it is
        // hidden with its ground (`TERRAIN-50`).
        self.water.set_step_square(self.terrain.square());
        // The budget's background work (the fill, and releases that are not a build's donor)
        // also waits while the vegetation has patches the view waits for (`TERRAIN-41` B1c): its
        // installs share this frame budget and its products the payload workers the fill's
        // requests would fill, so a fill running beside it kept a settled view waiting seconds.
        // A layer's releases also wait for the other layer's pending work, read inside
        // `frame_layers` once both layers' holes, level changes and hides ran. O(1).
        let vegetation_waits = self.residency.has_pending_work();
        // The preload (`TERRAIN-59`): its view phase holds the fill, so the places are counted
        // from the drawn patches' final extras before the keep rectangle is filled; its fill
        // phase is the load's own work and does not wait for the vegetation.
        let (fill_held, fill_waits) = match (preload, self.preload) {
            (true, Preload::View) => (true, true),
            (true, _) => (false, false),
            _ => (false, vegetation_waits),
        };
        self.water
            .hold_prewarm(fill_held || self.terrain.has_pending_work() || fill_waits);
        self.terrain.hold_prewarm(fill_held || fill_waits);
        self.water.hold_releases(vegetation_waits);
        self.terrain.hold_releases(vegetation_waits);
        // Over the budget, releases still wait for the view like any release that is not a
        // donor (`TERRAIN-41` B2: the budget is a target; the count returns to it once the view
        // is in step, `PatchStream::run_releases`). A hole build takes a donor only while one
        // more place would not fit (`TERRAIN-41` B1c).
        let [terrain_place, water_place] = place_bytes(&grid);
        let full = self.terrain_budget.is_some_and(|budget| {
            self.counted_bytes() + terrain_place + water_place > budget
        });
        self.terrain.set_budget_full(full);
        self.water.set_budget_full(full);
        // The vegetation's units in the patch units' order (`VEG-35` R1) while those run on the
        // frame's budget: the products delivered in the earlier passes, ordered once
        // (`begin_units`, `O(P log P)`), each placed by class, then squared ground distance from
        // the eye, the patch unit first on a tie (`frame_layers`). In the post-load window (its own
        // budget) and the load's hold (unbudgeted, or the preload's slice, `TERRAIN-59`) they keep
        // their place after the patch units. Their frustum is the camera's, computed only while
        // products wait.
        let frame_budget = !window && !self.load_settling;
        let vegetation_planes = (frame_budget && self.vegetation.pending() > 0)
            .then(|| self.camera_frustum())
            .flatten();
        let vegetation_in_view = |low: [f32; 3], high: [f32; 3]| {
            vegetation_planes
                .as_ref()
                .is_none_or(|planes| box_in_view(planes, low, high))
        };
        // Only products this frame's residency keeps wanted (`VEG-35` R1b): the residency's
        // evaluation runs after the patch units, at this camera, over the terrain patches resident
        // then; a patch of the terrain's wanted square stays resident through the patch units.
        // `O(subdivision²)` a waiting product.
        let camera = self.view.map(|view| [view.eye[0], view.eye[2]]);
        let square = self.terrain.square();
        let stays = |block: [i32; 2]| {
            square.is_some_and(|[x0, z0, x1, z1]| {
                (x0..=x1).contains(&block[0]) && (z0..=z1).contains(&block[1])
            })
        };
        let residency = &self.residency;
        let kept = |key: PatchKey| residency.keeps(key, camera, stays);
        let vegetation_joins = frame_budget
            && self.backend.is_some()
            && self.vegetation.begin_units(&vegetation_in_view, &kept);
        let budget = if preload {
            &mut self.preload_budget
        } else if window {
            &mut self.post_load_budget
        } else {
            &mut self.budget
        };
        let vegetation_before = self.vegetation.install_waits()[1].measured_us;
        // Holes held beyond the vegetation's reach do not hold its replacements (`VEG-35` R1b).
        let reach_sq = self.residency.reach().map_or(f64::INFINITY, |reach| reach * reach);
        let mut vegetation_units = self
            .backend
            .as_mut()
            .filter(|_| vegetation_joins)
            .map(|backend| VegetationUnits {
                layer: &mut self.vegetation,
                backend,
                reach_sq,
            });
        let mut water_exec = self.water_scripts.as_mut().map(ScriptExecutor);
        let mut terrain_exec = self.terrain_scripts.as_mut().map(ScriptExecutor);
        let ran = [water_exec.is_some(), terrain_exec.is_some()];
        self.patch_holes_deferred = frame_layers(
            &mut [
                water_exec
                    .as_mut()
                    .map(|exec| LayerFrame::new(&mut self.water, exec)),
                terrain_exec
                    .as_mut()
                    .map(|exec| LayerFrame::new(&mut self.terrain, exec)),
            ],
            eye,
            self.delta,
            budget,
            &self.clock,
            unbudgeted,
            &in_view,
            vegetation_units
                .as_mut()
                .map(|holes| holes as &mut dyn UnitSource),
        );
        // The vegetation's units are its own record's (`vegetation`), not the patches'.
        let vegetation_us = self.vegetation.install_waits()[1].measured_us - vegetation_before;
        if ran[0] {
            self.water_work = self.water.work();
            self.releases_total[1] += self.water_work.releases as u64;
            self.water.take_changes().for_each(drop);
            self.water.take_generations().for_each(drop);
            self.water.take_released().for_each(drop);
        }
        if ran[1] {
            self.terrain_work = self.terrain.work();
            self.releases_total[0] += self.terrain_work.releases as u64;
        }
        // The installs after the patch units place a product pulled since by the frame's order
        // (`VEG-35` R1b), where the patch units run on the frame's budget.
        if frame_budget {
            for (ran, work) in ran.into_iter().zip([self.water_work, self.terrain_work]) {
                if ran {
                    self.vegetation.note_patch_holes(work.last_hole);
                }
            }
        }
        self.follow_terrain();
        // The window closes when the patches first settle with residents, or after
        // `POST_LOAD_WINDOW_S` of frames outside the load's hold (a view never let settle).
        if window {
            self.post_load_s += self.delta;
        }
        if self.post_load_window
            && (self.post_load_s >= POST_LOAD_WINDOW_S
                || self.terrain.resident_count() > 0
                    && !self.terrain.has_pending_work()
                    && !self.water.has_pending_work())
        {
            self.post_load_window = false;
        }
        self.frame_patch_us = start.elapsed().as_secs_f64() * 1e6 - vegetation_us;
        self.frame_patch_units_us = self.budget.measured_us - units_before - vegetation_us
            + if window {
                self.post_load_budget.measured_us - window_before
            } else if preload {
                self.preload_budget.measured_us
            } else {
                0.0
            };
        vegetation_joins
    }

    // The road hydration's units under its own budget a frame, nearest first; the executor hears
    // when none is left.
    fn road_frame(&mut self) {
        self.road_work = RoadWork::default();
        if self.roads.pending() == 0 {
            return;
        }
        let Some(mut scripts) = self.road_scripts.clone() else {
            return;
        };
        self.road_budget.begin_frame();
        let planes = self.camera_frustum();
        let in_view = |low: [f32; 3], high: [f32; 3]| {
            planes
                .as_ref()
                .is_none_or(|planes| box_in_view(planes, low, high))
        };
        let eye = self.view.map(|view| view.eye);
        self.road_work = self.roads.frame(
            eye,
            &mut self.road_budget,
            &self.clock,
            &in_view,
            &mut RoadScriptExecutor(&mut scripts),
        );
        if self.roads.pending() == 0 {
            self.end_road_hydration();
        }
    }

    fn end_road_hydration(&mut self) {
        if let Some(mut scripts) = self.road_scripts.take() {
            scripts.call("stream_road_hydration_done", &[]);
        }
    }

    // The terrain's residency changes: the water's wanted set and the vegetation's tiers.
    fn follow_terrain(&mut self) {
        if let (Some(book), Some(grid)) = (self.graded.as_mut(), self.terrain.grid()) {
            let mut book = book.bind_mut();
            for (key, follow) in self.terrain.take_follow() {
                let arrives = matches!(
                    follow,
                    crate::nodes::sim::render::stream::layers::terrain::Follow::Resident { .. }
                );
                book.follow_terrain(key, follow, grid.rect(key));
                // A patch that arrives without its graded list draws nothing until the list
                // comes (T53 (a)), from this pass on: its backdrop cell stays (`TERRAIN-41` D3).
                if arrives {
                    book.blank_until_drawn(key);
                }
            }
        }
        for (key, generation) in self.terrain.take_generations() {
            self.residency.set_surface_generation(key, generation);
        }
        // A released patch's generation goes, as when the script frees a node itself.
        let mut book = self.graded.as_mut().map(|book| book.bind_mut());
        for key in self.terrain.take_released() {
            self.residency.set_surface_generation(key, -1);
            if let Some(book) = book.as_mut() {
                book.follow_generation(key, -1);
            }
        }
        drop(book);
        let mut changed = false;
        for (key, resident) in self.terrain.take_changes() {
            self.water.set_wanted(key, resident);
            self.residency.set_resident(key, resident);
            // The patch's backdrop cell goes or comes in the same pass (`TERRAIN-41` D2).
            self.backdrop_layer.set_resident(key, resident);
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
        self.open_ledger();
        self.vegetation.begin_pass(self.ledger_frame);
        self.run_units(residency, start);
        self.backdrop_pass(residency);
        self.close_ledger();
    }

    // The backdrop's part of a pass (`TERRAIN-41` D1, D2), after the vegetation's installs: in
    // the frame's first pass a sampling job when the sources moved, then the waiting uploads in
    // the room the vegetation left (all of them in the load's hold); in both passes the cells'
    // changed blocks, so a patch made resident or hidden in this pass (or by a script between the
    // passes) and its cell reach the same frame. Nothing without a change; `O(changed blocks x
    // 256)` rows otherwise.
    fn backdrop_pass(&mut self, residency: bool) {
        // The resident patches that started or stopped drawing nothing (the graded book's draw
        // states, applied in the terrain renderer's frame or at a residency in this pass): a
        // cell draws exactly while its patch draws nothing (`TERRAIN-41` D3). `O(news)`.
        if let Some(book) = self.graded.as_mut() {
            for (key, blank) in book.bind_mut().take_blank_news() {
                self.backdrop_layer.set_blank([key.0, key.1], blank);
            }
        }
        let Some(backend) = self.backdrop_backend.as_mut() else {
            return;
        };
        let start = std::time::Instant::now();
        if residency && let Some(spawner) = self.backdrop.as_ref() {
            async_backdrop::check(spawner);
            if let Ok(mut state) = spawner.shared.state.try_lock() {
                let world = spawner.shared.world();
                let product = state.product.as_mut().filter(|product| product.world == world);
                self.backdrop_layer.upload(
                    product,
                    backend,
                    &mut self.budget,
                    &self.clock,
                    self.load_settling,
                );
            }
        }
        self.backdrop_layer
            .flush(backend, [PATCH_LOW_M, PATCH_HIGH_M]);
        // A world change frees the old textures: their frees narrow the installs' room.
        let free_us = backend.take_free_us();
        self.budget.add_spent(free_us);
        self.frame_free_pass_us += free_us;
        self.frame_free_us += free_us;
        self.frame_us += start.elapsed().as_secs_f64() * 1e6;
    }

    // This pass's account: the frame's ledger as the owners before it left it, with the node's
    // own units so far (its first unit of the frame always runs).
    fn open_ledger(&mut self) {
        self.ledger_frame = Engine::singleton().get_process_frames();
        let units = self.budget.units;
        self.budget = with_ledger(self.ledger_frame, |ledger| {
            FrameBudget::within(&ledger.budget, units)
        });
    }

    // Charges the pass's units and frees to the ledger and says whether holes still wait.
    fn close_ledger(&mut self) {
        let budget = self.budget;
        let free_us = std::mem::take(&mut self.frame_free_pass_us);
        let holes = self.vegetation.holes_waiting()
            || self.terrain.pending()[0] > 0
            || self.water.pending()[0] > 0;
        let holes_deferred = self.patch_holes_deferred;
        let perf = self.perf;
        let mut charged = self.frame_charged;
        let spent = with_ledger(self.ledger_frame, |ledger| {
            let measured = budget.measured_us - ledger.budget.measured_us - free_us;
            let predicted = budget.predicted_us - ledger.budget.predicted_us;
            if measured > 0.0 || predicted > 0.0 {
                ledger.charge(Owner::Stream, predicted, measured.max(0.0));
            }
            if free_us > 0.0 {
                ledger.budget.add_spent(free_us);
                ledger.charged[Owner::Free as usize] += free_us;
            }
            ledger.holes_waiting = holes;
            ledger.holes_deferred = holes_deferred;
            if perf {
                charged[..OWNERS].copy_from_slice(&ledger.charged);
                charged[OWNERS] = ledger.budget.predicted_us;
            }
            [
                ledger.charged[Owner::Stream as usize],
                ledger.budget.measured_us,
                ledger.budget.predicted_us,
            ]
        });
        self.frame_spent = [spent[0], spent[1]];
        self.frame_ledger = [spent[1], spent[2]];
        self.frame_charged = charged;
    }

    // One pass's work (`run_pass`).
    fn run_units(&mut self, residency: bool, start: std::time::Instant) {
        self.update_camera();
        if residency {
            // The vegetation's installs among the patch units go to the residency before its
            // frame, which reads them as drawn (`VEG-35` R1).
            if self.patch_frame() {
                let installed = self.vegetation.end_units();
                self.frame_installs += installed;
                if installed > 0
                    && let Some(backend) = self.backend.as_mut()
                {
                    self.residency.take_installs(&mut self.vegetation, backend);
                }
            }
            let roads = std::time::Instant::now();
            self.road_frame();
            self.frame_road_us = roads.elapsed().as_secs_f64() * 1e6;
        }
        if residency && let Some(backend) = self.backend.as_mut() {
            let eye = self.view.map(|view| view.eye);
            let mut jobs = Jobs(self.spawner.as_mut());
            self.residency
                .frame(eye, &mut self.vegetation, backend, &mut jobs);
            // The residency's frees narrow the installs' room (the `free` class).
            let free_us = backend.take_free_us();
            self.budget.add_spent(free_us);
            self.frame_free_pass_us += free_us;
            self.frame_free_us += free_us;
            // While nothing wanted waits, products along the camera's reach (`RENDER-32-N`); the
            // reach walks are the ledger's (AUDIT-stream-2 finding 8), as measured time.
            let prepare = std::time::Instant::now();
            self.residency.prepare(self.view, &mut jobs);
            let prepare_us = prepare.elapsed().as_secs_f64() * 1e6;
            self.budget.add_spent(prepare_us);
            self.frame_prepare_us += prepare_us;
            self.frame_residency_us = start.elapsed().as_secs_f64() * 1e6;
        }
        self.pull();
        if residency {
            // The prepares that landed freed their places: the still view prepares once more.
            let prepare = std::time::Instant::now();
            self.residency
                .prepare_after_pull(&mut Jobs(self.spawner.as_mut()));
            let prepare_us = prepare.elapsed().as_secs_f64() * 1e6;
            self.budget.add_spent(prepare_us);
            self.frame_prepare_us += prepare_us;
        }
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
        self.vegetation.end_pass();
        self.residency.take_installs(&mut self.vegetation, backend);
        self.frame_copy_us += backend.take_copy_us();
        let free_us = backend.take_free_us();
        self.budget.add_spent(free_us);
        self.frame_free_pass_us += free_us;
        self.frame_free_us += free_us;
        self.recycle_spent();
        // The patches' and the road hydration's shares are their own records (`terrain_stream`,
        // `road_hydration`).
        let patches_us = if residency {
            self.frame_patch_us + self.frame_road_us
        } else {
            0.0
        };
        self.frame_us += start.elapsed().as_secs_f64() * 1e6 - patches_us;
    }

    // Takes every finished product of a live request from the jobs.
    fn pull(&mut self) {
        let Some(spawner) = self.spawner.as_ref() else {
            return;
        };
        // Requests a busy core put back are tried again here at the latest, once a pass.
        jobs::pump(spawner);
        // Taken under the jobs' lock, handled after it (a held product's eviction sorts the held
        // set: the workers do not wait for it; AUDIT-stream-2 finding 8). Reused storage.
        let mut landed = std::mem::take(&mut self.landed);
        {
            let mut jobs = spawner.jobs.lock().unwrap();
            landed.extend(std::iter::from_fn(|| jobs.poll()));
        }
        for delivered in landed.drain(..) {
            self.frame_pulled += 1;
            // A product prepared ahead is held by the residency, not installed (`RENDER-32-N`).
            let product = delivered.product;
            let delivery = Delivery {
                key: delivered.key,
                serial: delivered.serial,
                origin: product.origin,
                span: product.span,
                product,
            };
            if let Some(delivery) = self.residency.take_prepared(delivery) {
                self.vegetation.deliver(delivery);
            }
        }
        self.landed = landed;
    }

    // Returns installed and dropped products' storage to the jobs; what their spare limit
    // refuses is dropped after the lock (at most the products of one eviction pass, a sixteenth
    // of the held cap, and a frame's spent installs).
    fn recycle_spent(&mut self) {
        match self.spawner.as_ref() {
            Some(spawner) => {
                let mut refused = std::mem::take(&mut self.refused);
                {
                    let mut jobs = spawner.jobs.lock().unwrap();
                    let spent = self.vegetation.take_spent();
                    refused.extend(spent.filter_map(|product| jobs.recycle(product)));
                    let spent = self.residency.take_prepared_spent();
                    refused.extend(spent.filter_map(|product| jobs.recycle(product)));
                }
                refused.clear();
                self.refused = refused;
            }
            None => {
                self.vegetation.take_spent().for_each(drop);
                self.residency.take_prepared_spent().for_each(drop);
            }
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

    // Keeps the agents' twins for the drawing camera and the sun's shadow range, charged to the
    // frame's ledger, and tells the agents' next frame the range its candidates are kept for.
    // `O(candidates)` a frame in which they, the camera or the range changed (`RENDER-15`, R47).
    fn keep_agent_casters(&mut self) {
        let camera = self
            .base()
            .get_viewport()
            .and_then(|viewport| viewport.get_camera_3d());
        let Some(draw) = self.agents.as_mut() else {
            return;
        };
        draw.frame_us = 0.0;
        let Some(camera) = camera else {
            return;
        };
        let start = std::time::Instant::now();
        let transform = camera.get_global_transform();
        let sun = &draw.sun;
        let range = if sun.is_visible_in_tree() && sun.has_shadow() {
            sun.get_param(godot::classes::light_3d::Param::SHADOW_MAX_DISTANCE)
        } else {
            0.0
        };
        let view = CasterView {
            eye: transform.origin.to_array(),
            forward: (-transform.basis.col_c()).normalized().to_array(),
            range,
        };
        let mut feed = draw.feed.lock().unwrap();
        feed.range_m = if draw.layer.is_active() { range } else { 0.0 };
        draw.layer.update(&mut draw.backend, &feed, view);
        drop(feed);
        let us = start.elapsed().as_secs_f64() * 1e6;
        draw.frame_us = us;
        with_ledger(Engine::singleton().get_process_frames(), |ledger| {
            ledger.charge(Owner::AgentCasters, us, us)
        });
    }

    fn record_perf(&mut self) {
        let counted = self.counted_bytes();
        let mut budget = VarDictionary::new();
        self.budget_status(&mut budget);
        let script = self.perf_script.get_or_insert_with(|| {
            ResourceLoader::singleton()
                .load("res://scripts/core/perf_debug.gd")
                .and_then(|resource| resource.try_cast::<Script>().ok())
                .expect("perf_debug.gd")
        });
        let status = self.vegetation.status();
        let mut details = VarDictionary::new();
        details.set("installs", self.frame_installs as i64);
        // The node's own units (the ledger's `Stream` charge) and the frame's whole ledger.
        details.set("install_ms", self.frame_spent[0] / 1000.0);
        details.set("ledger_ms", self.frame_spent[1] / 1000.0);
        // The ledger by owner and its predicted total (`TERRAIN-41`): the admissions compare the
        // larger of the measured and the predicted spend with the budget.
        const OWNER_NAMES: [&str; OWNERS] = [
            "terrain_commit",
            "graded",
            "water",
            "building_publication",
            "building_sites",
            "stream",
            "free",
            "agent_casters",
        ];
        for (name, us) in OWNER_NAMES.iter().zip(self.frame_charged) {
            details.set(format!("ledger_{name}_ms"), us / 1000.0);
        }
        details.set("ledger_predicted_ms", self.frame_charged[OWNERS] / 1000.0);
        details.set("predicted_ms", self.budget.predicted_us / 1000.0);
        details.set("upload_max_ms", self.budget.max_unit_us / 1000.0);
        details.set(
            "overrun_max_ms",
            self.budget.max_overrun_us.max(0.0) / 1000.0,
        );
        details.set("poll_ms", self.frame_copy_us / 1000.0);
        details.set("free_ms", self.frame_free_us / 1000.0);
        details.set("prepare_ms", self.frame_prepare_us / 1000.0);
        details.set("pulled", self.frame_pulled as i64);
        details.set("refreshed", self.frame_refreshed as i64);
        details.set("stream_pending", status.pending as i64);
        // This frame's installs' waits and costs (`VEG-35` R0), apart from the patch units'.
        let [_, waits] = self.vegetation.install_waits();
        set_install_waits(&mut details, "install_", &waits);
        details.set("stream_blocked", status.blocked as i64);
        details.set("slots_live", status.slots_live as i64);
        details.set("slots_spare", status.slots_spare as i64);
        details.set("stream_ms", self.frame_us / 1000.0);
        details.set(
            "agent_casters_ms",
            self.agents.as_ref().map_or(0.0, |draw| draw.frame_us / 1000.0),
        );
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
        details.set("standins_released", work.released as i64);
        let counts = self.residency.counts();
        details.set("wanted", counts[0]);
        details.set("patches", counts[1]);
        details.set("queue", counts[4]);
        details.set("requested", counts[5]);
        if self.terrain.grid().is_some() {
            let mut patches = VarDictionary::new();
            patches.set("units_ms", self.frame_patch_units_us / 1000.0);
            patches.set("post_load_window", self.post_load_window);
            // The budget's counted bytes (`TERRAIN-41` B1) and its state by kind (B2).
            patches.set("budget_counted_bytes", counted as i64);
            for (key, value) in budget.iter_shared() {
                patches.set(format!("budget_{key}"), value);
            }
            for (name, layer, work) in [
                ("terrain", &self.terrain, self.terrain_work),
                ("water", &self.water, self.water_work),
            ] {
                let [missing, leaving, queue] = layer.pending();
                patches.set(format!("{name}_missing"), missing as i64);
                patches.set(format!("{name}_refused"), layer.refused_count() as i64);
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
                patches.set(
                    format!("{name}_holes_deferred_in_view"),
                    work.holes_deferred_in_view as i64,
                );
                patches.set(format!("{name}_releases"), work.releases as i64);
                patches.set(format!("{name}_donors"), work.donors as i64);
                patches.set(format!("{name}_built"), layer.built_count() as i64);
                // Units by kind (`TERRAIN-41` L1): count, measured and longest microseconds.
                for kind in 0..UNIT_KINDS {
                    let kind_name = UNIT_KIND_NAMES[kind];
                    patches.set(format!("{name}_{kind_name}_units"), work.unit_count[kind] as i64);
                    patches.set(format!("{name}_{kind_name}_us"), work.unit_us[kind]);
                    patches.set(format!("{name}_{kind_name}_max_us"), work.unit_max_us[kind]);
                }
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
        if self.road_work != RoadWork::default() || self.roads.pending() > 0 {
            let mut roads = VarDictionary::new();
            roads.set("installed", self.road_work.installed as i64);
            roads.set("parts", self.road_work.parts as i64);
            roads.set("gone", self.road_work.gone as i64);
            roads.set("busy", self.road_work.busy);
            roads.set("pending", self.roads.pending() as i64);
            roads.set("predicted_ms", self.road_budget.predicted_us / 1000.0);
            roads.set("unit_max_ms", self.road_budget.max_unit_us / 1000.0);
            script.call(
                "record",
                &[
                    "road_hydration".to_variant(),
                    (self.road_work.units_us / 1000.0).to_variant(),
                    roads.to_variant(),
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

// The vegetation's install counters (`VEG-35` R0) as `<prefix><name>` fields.
fn set_install_waits(out: &mut VarDictionary, prefix: &str, waits: &InstallWaits) {
    const CLASS_NAMES: [&str; 5] = ["hole_in_view", "hole_near", "hole", "upgrade_in_view", "upgrade"];
    out.set(format!("{prefix}installs"), waits.installs as i64);
    out.set(format!("{prefix}wait_frames_sum"), waits.wait_frames_sum as i64);
    out.set(format!("{prefix}wait_frames_max"), waits.wait_frames_max as i64);
    out.set(format!("{prefix}waited_over_bar"), waits.waited_over_bar as i64);
    for (name, count) in CLASS_NAMES.iter().zip(waits.by_class) {
        out.set(format!("{prefix}{name}"), count as i64);
    }
    out.set(format!("{prefix}stalled_passes"), waits.stalled_passes as i64);
    out.set(format!("{prefix}predicted_us"), waits.predicted_us);
    out.set(format!("{prefix}measured_us"), waits.measured_us);
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
        min_distance: 0.0,
        max_distance: 0.0,
        zoom_rate: 1.0,
    }
}

// The vegetation's jobs, or none (a stream node without a simulation requests nothing).
struct Jobs<'a>(Option<&'a mut Spawner>);

impl PatchJobs for Jobs<'_> {
    fn request(&mut self, order: &PatchOrder, settings: &ResidencySettings) -> Option<u64> {
        self.0.as_mut()?.request(order, settings)
    }

    fn promote(&mut self, key: PatchKey, serial: u64) {
        if let Some(spawner) = self.0.as_mut() {
            spawner.promote(key, serial);
        }
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
        // Lanes that do not fit their type, or a drawn batch whose count does not match its
        // buffer (the install would index past it every frame: AUDIT-stream finding 8), refuse.
        let lane = |at: usize| u8::try_from(ints[at]).ok();
        let count = u32::try_from(ints[4]).ok()?;
        let (proxy, colors, custom) = (ints[3] == 1, ints[5] == 1, ints[6] == 1);
        let stride = 12 + 4 * usize::from(colors) + 4 * usize::from(custom);
        if !proxy && buffers[buffer].len() != count as usize * stride {
            return None;
        }
        out.push(Batch {
            species: lane(0)?,
            lod: lane(1)?,
            variant: lane(2)?,
            proxy,
            count,
            colors,
            custom,
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

// The post-load window's longest life outside the load's hold, seconds (AUDIT-stream-2 finding
// 5): a load settles its patches in 0.6-8.5 s (Kuopio, 100k); a camera that never lets them
// settle no longer keeps their own budget for the session.
const POST_LOAD_WINDOW_S: f64 = 30.0;

// The lowest and highest ground a patch's view test assumes, metres: the box only orders work.
const PATCH_LOW_M: f32 = -500.0;
const PATCH_HIGH_M: f32 = 2500.0;

// Runs a patch layer's units on `terrain.gd` / `water.gd`: their `stream_request`,
// `stream_activate`, `stream_deactivate`, `stream_build` and `stream_set_lod`, one call a unit.
// `road_chunk_hydration.gd`'s parts.
struct RoadScriptExecutor<'a>(&'a mut Gd<Node>);

impl RoadExecutor for RoadScriptExecutor<'_> {
    fn prepare(&mut self, keys: &[RoadKey]) {
        let keys = PackedInt32Array::from_iter(keys.iter().flatten().copied());
        self.0.call("stream_road_prepare", &[keys.to_variant()]);
    }

    fn next_part(&mut self, key: RoadKey) -> RoadPart {
        let predicted = self
            .0
            .call("stream_road_next_part", &[Vector2i::new(key[0], key[1]).to_variant()])
            .try_to::<f64>()
            .unwrap_or(-2.0);
        if predicted >= 0.0 {
            RoadPart::Ready(predicted)
        } else if predicted == -1.0 {
            RoadPart::Waiting
        } else {
            RoadPart::Gone
        }
    }

    fn run_part(&mut self, key: RoadKey) -> RoadInstall {
        let answer = self
            .0
            .call("stream_road_part", &[Vector2i::new(key[0], key[1]).to_variant()])
            .try_to::<i64>()
            .unwrap_or(3);
        match answer {
            0 => RoadInstall::Installed,
            1 => RoadInstall::Gone,
            2 => RoadInstall::Busy,
            4 => RoadInstall::Part,
            _ => RoadInstall::Cancelled,
        }
    }
}

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

    // The script answers 0 released, 1 nothing built, 2 a publication is pending, 3 busy.
    fn release(&mut self, key: TerrainKey) -> Release {
        let done = self
            .0
            .call("stream_release", &[Vector2i::new(key[0], key[1]).to_variant()]);
        match done.try_to::<i64>().unwrap_or(2) {
            0 | 1 => Release::Done,
            3 => Release::Busy,
            _ => Release::Blocked,
        }
    }
}

// An agent kind by its bridge number: 0 walkers, 1 moving cars.
fn agent_kind(kind: i64) -> Option<AgentKind> {
    match kind {
        0 => Some(AgentKind::Walkers),
        1 => Some(AgentKind::Cars),
        _ => None,
    }
}
