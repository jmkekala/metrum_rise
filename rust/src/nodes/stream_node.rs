// SPDX-License-Identifier: GPL-2.0-only

//! The streaming core's frame (`RENDER-32-E`, `docs/terrain.md` "Streaming core"): a node after
//! the camera in `Main.tscn` that, every frame, takes the products the background jobs finished,
//! follows the camera inside installed chunks and installs what waits under one streaming budget,
//! once in its `_process` and again just before the frame draws (`frame_pre_draw`), so a product
//! that landed meanwhile still draws this frame.
//!
//! Vegetation is its first layer: `vegetation.gd` registers its meshes and materials by RID,
//! requests patches from the `SimulationNode`'s jobs, and shows, hides and frees installed
//! patches here by key; no node is created per patch. Every `RenderingServer` call of the layer
//! goes through [`GodotBackend`] on the main thread.

use crate::nodes::sim::render::stream::backend::{GodotBackend, Role, Shadow};
use crate::nodes::sim::render::stream::layers::vegetation::{
    Delivery, MATERIAL_COUNT, MESH_COUNT, PatchKey, SLOT_LIMIT, SPARE_LIMIT, VegetationLayer,
    VegetationRules,
};
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
    spawner: Option<Spawner>,
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
}

#[godot_api]
impl INode for StreamNode {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            backend: None,
            vegetation: VegetationLayer::new(SLOT_LIMIT, SPARE_LIMIT),
            spawner: None,
            budget: FrameBudget::new(STREAM_BUDGET_US),
            clock: MonotonicClock::default(),
            perf: false,
            perf_script: None,
            frame_us: 0.0,
            frame_installs: 0,
            frame_refreshed: 0,
            frame_copy_us: 0.0,
            frame_pulled: 0,
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

    fn process(&mut self, _delta: f64) {
        self.budget.begin_frame();
        self.frame_us = 0.0;
        self.frame_installs = 0;
        self.frame_refreshed = 0;
        self.frame_copy_us = 0.0;
        self.frame_pulled = 0;
        self.run_pass();
    }
}

#[godot_api]
impl StreamNode {
    /// The second pass of the frame, just before it draws; then the frame's perf record.
    #[func]
    fn pre_draw(&mut self) {
        self.run_pass();
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

    /// Shows or hides every vegetation patch (`vegetation.gd`'s `enabled`).
    #[func]
    pub fn set_vegetation_enabled(&mut self, enabled: bool) {
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

    /// Drops a prepared product of `key` waiting for its install: call before a new request for
    /// the key and with its cancel. `O(waiting)`.
    #[func]
    pub fn forget_vegetation_patch(&mut self, key: Vector3i) {
        self.vegetation.forget(patch_key(key));
    }

    /// Shows or hides an installed patch. `O(batches)`.
    #[func]
    pub fn set_vegetation_patch_visible(&mut self, key: Vector3i, visible: bool) {
        if let Some(backend) = self.backend.as_mut() {
            self.vegetation
                .set_visible(backend, patch_key(key), visible);
        }
    }

    /// Frees an installed patch back to the pools. `O(batches)`.
    #[func]
    pub fn free_vegetation_patch(&mut self, key: Vector3i) {
        if let Some(backend) = self.backend.as_mut() {
            self.vegetation.free(backend, patch_key(key));
        }
    }

    /// Frees every vegetation patch and drops what waits (the renderer's reset).
    #[func]
    pub fn clear_vegetation(&mut self) {
        if let Some(backend) = self.backend.as_mut() {
            self.vegetation.clear(backend);
        }
        self.recycle_spent();
    }

    /// The vegetation installs since the last call, six lanes each: key x, y, z, the request's
    /// serial, the plants drawn (-1: the core was busy, request the patch again) and the install's
    /// microseconds.
    #[func]
    pub fn take_vegetation_installs(&mut self) -> PackedInt64Array {
        let events = PackedInt64Array::from(self.vegetation.events());
        self.vegetation.clear_events();
        events
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

    /// Turns the perf channel's records on or off (`PerfDebug`, recorded as `vegetation`).
    #[func]
    pub fn set_perf_enabled(&mut self, enabled: bool) {
        self.perf = enabled;
    }
}

impl StreamNode {
    // One pass: pull, follow the camera, install within what is left of the frame's budget.
    fn run_pass(&mut self) {
        let start = std::time::Instant::now();
        self.pull();
        self.update_camera();
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
        self.frame_copy_us += backend.take_copy_us();
        self.recycle_spent();
        self.frame_us += start.elapsed().as_secs_f64() * 1e6;
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

    fn update_camera(&mut self) {
        let camera = self
            .base()
            .get_viewport()
            .and_then(|viewport| viewport.get_camera_3d())
            .map(|camera| camera.get_global_position().to_array());
        self.vegetation.set_camera(camera);
    }

    // The camera's frustum planes, only while something waits for an install.
    fn frustum(&self) -> Option<[[f32; 4]; 6]> {
        if self.vegetation.pending() == 0 {
            return None;
        }
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
