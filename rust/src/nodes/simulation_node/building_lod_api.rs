// SPDX-License-Identifier: GPL-2.0-only

//! Thin main-thread bridge for shared building resources and coherent spatial LOD batches.

use super::*;
use crate::nodes::lod_policy::quality_from_id;
use crate::nodes::sim::render::building_lod::spatial::{Pose, query_bounds};
use crate::nodes::sim::render::building_lod::{BatchKey, Part, View};
use crate::nodes::sim::render::stream::backend::{GodotBackend, slot_census};
use crate::nodes::sim::render::stream::layers::buildings::{
    Applied, BuildingLayer, Look, SLOT_LIMIT, SPARE_LIMIT,
};
use glam::{Mat3, Mat4, Vec2, Vec3};

#[godot_api(secondary)]
impl SimulationNode {
    /// Enumerate complete tier chains once at world loading/pack refresh. Returned slot IDs
    /// are presentation-local and valid only for the accompanying catalog generation.
    #[func]
    pub fn get_building_lod_catalog(&mut self) -> VarDictionary {
        let (shared, turnstile) = (Arc::clone(&self.core), Arc::clone(&self.core_turnstile));
        let core = Self::wait_for_shared_core(&turnstile, &shared);
        self.building_lods.load_catalog(&core.allocator.registry);
        drop(core);
        self.reset_building_draw();
        let mut parts = VarArray::new();
        for (id, part) in self.building_lods.catalog.iter().enumerate() {
            let mut item = VarDictionary::new();
            item.set("id", id as i64);
            item.set("asset_id", part.asset.as_str());
            item.set("part_index", part.index as i64);
            item.set(
                "paths",
                PackedStringArray::from_iter(
                    part.paths.iter().map(|path| GString::from(path.as_str())),
                ),
            );
            item.set("schemes", scheme_bindings(part));
            item.set("window_brightness", part.window_brightness);
            parts.push(&item.to_variant());
        }
        let mut result = VarDictionary::new();
        result.set("generation", self.building_lods.generation as i64);
        result.set("parts", parts);
        result
    }

    /// Install import availability once, preserving authored ordinals across failed tiers.
    /// Godot supplies a visible placeholder at slot zero if no source tier can be imported.
    #[func]
    pub fn configure_building_lod_resources(
        &mut self,
        generation: i64,
        availability: Array<PackedByteArray>,
        resource_bounds: Array<Aabb>,
    ) -> bool {
        if generation as u64 != self.building_lods.generation
            || availability.len() != self.building_lods.parts.len()
            || resource_bounds.len() != availability.len()
        {
            return false;
        }
        // Validate the complete transaction before replacing any table.
        for (id, loaded) in availability.iter_shared().enumerate() {
            let bounds = resource_bounds.at(id);
            if loaded.len() != self.building_lods.parts[id].fallback.len()
                || !loaded.as_slice().iter().any(|&value| value != 0)
                || !bounds.position.is_finite()
                || !bounds.size.is_finite()
                || bounds.size.x < 0.0
                || bounds.size.y < 0.0
                || bounds.size.z < 0.0
            {
                return false;
            }
        }
        for (id, loaded) in availability.iter_shared().enumerate() {
            let available: Vec<_> = loaded.as_slice().iter().map(|&value| value != 0).collect();
            let resource = resource_bounds.at(id);
            let to_vec = |p: Vector3| Vec3::new(p.x, p.y, p.z);
            let cull_bounds = [
                to_vec(resource.position),
                to_vec(resource.position + resource.size),
            ];
            let mut bounds = self.building_lods.parts[id].bounds;
            // If LOD0 itself failed, retain the available geometry rather than a zero-size
            // policy reference. Normal successful chains always keep imported LOD0 bounds.
            if bounds == [Vec3::ZERO; 2] {
                bounds = cull_bounds;
            }
            self.building_lods.parts[id] = Part::new(bounds, &available).unwrap();
            self.building_lods.parts[id].cull_bounds = cull_bounds;
        }
        let (shared, turnstile) = (Arc::clone(&self.core), Arc::clone(&self.core_turnstile));
        let core = Self::wait_for_shared_core(&turnstile, &shared);
        self.building_lods
            .refresh_geometry_margins(&core.allocator.registry);
        self.building_lods.invalidate_world();
        true
    }

    /// Draws the building batches in `scenario` from now on (`building_lods.gd`, once): the
    /// `RenderingServer` slots of `RENDER-32-L`. A second call keeps the slots.
    #[func]
    pub fn set_building_lod_scenario(&mut self, scenario: Rid) {
        if self.building_draw.is_none() && scenario.is_valid() {
            let mut backend = GodotBackend::new(scenario);
            backend.set_catalogue(Vec::new(), Vec::new(), reflection_parameters());
            self.building_draw = Some(BuildingDraw {
                backend,
                layer: BuildingLayer::new(SLOT_LIMIT, SPARE_LIMIT),
                applied: Applied::default(),
            });
        }
    }

    /// Sets the water reflection's world (an invalid RID: none); the batches' proxies move
    /// into it.
    #[func]
    pub fn set_building_lod_reflection_scenario(&mut self, scenario: Rid) {
        if let Some(draw) = self.building_draw.as_mut() {
            draw.layer.drop_reflections(&mut draw.backend);
            draw.backend
                .set_reflection_scenario(scenario.is_valid().then_some(scenario));
            draw.layer.redraw_reflections(&mut draw.backend);
        }
    }

    /// Whether reused slots take fresh storage (the viewport draws motion vectors).
    #[func]
    pub fn set_building_lod_fresh_buffers(&mut self, fresh: bool) {
        if let Some(draw) = self.building_draw.as_mut() {
            draw.layer.set_fresh_buffers(fresh);
        }
    }

    /// Whether the building batches cast the sun's shadow (true unless a shadow benchmark's
    /// caster variant clears it, `RENDER-15`).
    #[func]
    pub fn set_building_cast_shadows(&mut self, cast: bool) {
        if let Some(draw) = self.building_draw.as_mut() {
            draw.layer.set_cast_shadows(&mut draw.backend, cast);
        }
    }

    /// The draw census of the building batches' live slots (`RENDER-44`, the stream benchmark's
    /// census only; see `backend::slot_census`). Empty before the batches draw. `O(slots)`.
    #[func]
    pub fn get_building_draw_census(&self) -> VarDictionary {
        self.building_draw
            .as_ref()
            .map_or_else(VarDictionary::new, |draw| slot_census(draw.layer.pool()))
    }

    /// Registers the mesh and material override (an invalid RID: none) a group draws with,
    /// built by `building_lods.gd` when [`Self::update_building_lods`] named it missing.
    #[func]
    pub fn register_building_lod_look(
        &mut self,
        part: i64,
        lod: i64,
        deserted: bool,
        scheme: i64,
        mesh: Rid,
        material: Rid,
    ) {
        let Some(draw) = self.building_draw.as_mut() else {
            return;
        };
        let look = Look {
            mesh: draw.backend.push_mesh(mesh),
            material: material
                .is_valid()
                .then(|| draw.backend.push_material(material)),
        };
        let key = BatchKey {
            part: part as usize,
            deserted,
            scheme: scheme as u16,
            lod: lod as usize,
        };
        draw.layer.register(key, look);
    }

    /// Nonblocking, camera-driven building rendering (`RENDER-34`, `RENDER-32-L`): selects the
    /// tiers of the parts the camera's motion can have changed and draws the changed batches
    /// through the stream backend. `world_to_clip` is `projection` times the inverse of
    /// `camera_transform`, as the camera renders. Returns `busy` (the core was busy: the drawn
    /// frame stays), `reload`, `evaluated_parts`, `queried_chunks`, `uploaded_bytes` and
    /// `missing`: the groups (part, tier, deserted, scheme; four lanes each) to register before
    /// [`Self::publish_building_lods`] draws this frame. Resource loading never occurs here.
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn update_building_lods(
        &mut self,
        generation: i64,
        world_to_clip: Projection,
        projection: Projection,
        camera_transform: Transform3D,
        render_size: Vector2,
        quality: i32,
        shadow_to_light: Vector3,
        shadow_distance: f32,
    ) -> VarDictionary {
        let start = std::time::Instant::now();
        let mut result = VarDictionary::new();
        result.set("busy", true);
        let shared = Arc::clone(&self.core);
        let Some(mut core) = Self::try_lock_shared_core(&shared) else {
            return result;
        };
        let lods = &mut self.building_lods;
        if generation as u64 != lods.generation
            || lods.registry_revision != core.allocator.registry.revision()
        {
            result.set("reload", true);
            return result;
        }
        // The view reads the chunk lists and their bounds: the index's site part (`ALLOC-05`).
        core.prepare_site_queries();
        let view = View {
            world_to_clip: mat4(world_to_clip),
            render_size: Vec2::new(render_size.x, render_size.y),
            quality: quality_from_id(quality),
        };
        let to_vec = |p: Vector3| Vec3::new(p.x, p.y, p.z);
        let basis = camera_transform.basis;
        let pose = Pose {
            eye: to_vec(camera_transform.origin),
            basis: Mat3::from_cols(
                to_vec(basis.col_a()),
                to_vec(basis.col_b()),
                to_vec(basis.col_c()),
            ),
            projection: mat4(projection),
        };
        let bounds = if let Some(occupied) = core.allocator.building_chunk_bounds {
            let Some(bounds) = query_bounds(
                view,
                pose.eye,
                -pose.basis.z_axis,
                to_vec(shadow_to_light),
                shadow_distance,
                lods.horizontal_margin,
                core.allocator.max_building_support_m + lods.top_margin,
                occupied,
            ) else {
                return result;
            };
            bounds
        } else {
            [0, 0, -1, -1]
        };
        let update_start = std::time::Instant::now();
        let changed = lods.update(
            &core.allocator,
            bounds,
            view,
            Some(pose),
            core.construction_clock_slots(),
            std::time::Instant::now(),
        );
        let update_us = update_start.elapsed().as_secs_f64() * 1e6;
        drop(core);
        result.set("busy", false);
        // Microseconds: the frame before the tier update (lock, site queries, bounds), the update.
        result.set("prepare_us", (update_start - start).as_secs_f64() * 1e6);
        result.set("update_us", update_us);
        result.set("placing_us", self.building_lods.placing_us);
        result.set("evaluating_us", self.building_lods.evaluating_us);
        result.set("evaluated_parts", self.building_lods.evaluated_parts as i64);
        result.set("queried_chunks", self.building_lods.queried_chunks as i64);
        let waiting = self
            .building_draw
            .as_ref()
            .is_some_and(|draw| draw.layer.waiting());
        if !changed && self.building_lods.removed.is_empty() && !waiting {
            // An unchanged frame: nothing to draw, no batch visited. A frame that waits for
            // slots is offered again instead (`RENDER-42`).
            if let Some(draw) = self.building_draw.as_mut() {
                draw.applied = Applied::default();
            }
            result.set("uploaded_bytes", 0);
            return result;
        }
        let missing = self.missing_building_looks();
        if missing.is_empty() {
            let publish_start = std::time::Instant::now();
            result.set("uploaded_bytes", self.publish_building_lods());
            result.set("publish_us", publish_start.elapsed().as_secs_f64() * 1e6);
        } else {
            result.set("missing", missing);
        }
        result
    }

    /// Draws the frame [`Self::update_building_lods`] left waiting for looks, once every group
    /// has one. Returns the bytes written, or -1 while a group still has no look.
    #[func]
    pub fn publish_building_lods(&mut self) -> i64 {
        let lods = &mut self.building_lods;
        let Some(draw) = self.building_draw.as_mut() else {
            // Nothing draws the batches: the frame is taken as published.
            lods.acknowledge();
            return 0;
        };
        if !draw.layer.missing(lods.changes()).is_empty() {
            return -1;
        }
        let applied = draw
            .layer
            .apply(&mut draw.backend, &lods.removed, lods.changes());
        draw.applied = applied;
        // A frame the pool could not take whole keeps its changes for the next offer.
        if !draw.layer.waiting() {
            lods.acknowledge();
        }
        applied.bytes as i64
    }

    /// The last published frame's work (the perf channel): batches `written`, `hidden` (emptied
    /// or their chunk left), `blocked` (slots the frame is short of: it waits whole), and the pool's `slots_live`, `slots_spare`,
    /// `slots_created` and `slots_freed` so far.
    #[func]
    pub fn get_building_lod_work(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        if let Some(draw) = self.building_draw.as_ref() {
            let pool = draw.layer.pool();
            let (created, freed) = pool.churn();
            out.set("written", draw.applied.written as i64);
            out.set("hidden", draw.applied.hidden as i64);
            out.set("blocked", draw.applied.blocked as i64);
            out.set("slots_live", pool.live() as i64);
            out.set("slots_spare", pool.spares() as i64);
            out.set("slots_created", created as i64);
            out.set("slots_freed", freed as i64);
        }
        out
    }

    /// The drawn building batches (tests and the stream benchmark's digests), one dictionary
    /// each: `chunk`, `part`, `lod`, `deserted`, `scheme`, `count` and `multimesh` (its RID).
    /// `O(batches)`.
    #[func]
    pub fn get_building_lod_batches(&self) -> VarArray {
        let mut out = VarArray::new();
        let Some(draw) = self.building_draw.as_ref() else {
            return out;
        };
        let pool = draw.layer.pool();
        for (chunk, key, slot) in draw.layer.batches() {
            let mut item = batch_identity(chunk, key);
            let slot = pool.slot(slot);
            item.set("multimesh", Rid::new(slot.multimesh));
            let count = self.building_lods.chunks.get(&chunk).map_or(0, |resident| {
                resident
                    .batches
                    .batches
                    .iter()
                    .find(|batch| batch.key == key)
                    .map_or(0, |batch| batch.count)
            });
            item.set("count", count as i64);
            out.push(&item.to_variant());
        }
        out
    }
}

/// The building batches' backend and slots.
pub(crate) struct BuildingDraw {
    backend: GodotBackend,
    layer: BuildingLayer,
    applied: Applied,
}

impl SimulationNode {
    // The groups of the waiting frame without a look, four lanes each.
    fn missing_building_looks(&mut self) -> PackedInt32Array {
        let Some(draw) = self.building_draw.as_mut() else {
            return PackedInt32Array::new();
        };
        let lods = &self.building_lods;
        let missing = draw.layer.missing(lods.changes());
        let mut out = PackedInt32Array::new();
        for key in missing {
            for lane in [
                key.part as i32,
                key.lod as i32,
                i32::from(key.deserted),
                i32::from(key.scheme),
            ] {
                out.push(lane);
            }
        }
        out
    }

    // A new catalogue or the node leaving the tree: every slot and look goes.
    pub(super) fn reset_building_draw(&mut self) {
        if let Some(draw) = self.building_draw.as_mut() {
            draw.layer.reset(&mut draw.backend);
            draw.backend
                .set_catalogue(Vec::new(), Vec::new(), reflection_parameters());
        }
    }
}

// The building layer's instance parameters: `REFLECTION_FLAG` first.
fn reflection_parameters() -> Vec<StringName> {
    vec![StringName::from("water_reflection_proxy")]
}

fn mat4(projection: Projection) -> Mat4 {
    Mat4::from_cols_array_2d(&projection.cols.map(|c| [c.x, c.y, c.z, c.w]))
}

fn batch_identity(
    chunk: (i32, i32),
    batch: crate::nodes::sim::render::building_lod::BatchKey,
) -> VarDictionary {
    let mut item = VarDictionary::new();
    item.set("chunk", Vector2i::new(chunk.0, chunk.1));
    item.set("part", batch.part as i64);
    item.set("lod", batch.lod as i64);
    item.set("deserted", batch.deserted);
    item.set("scheme", batch.scheme as i64);
    item
}

/// Publish one part's scheme table in manifest order; Godot binds the named source
/// materials without deciding which textures belong to which tier.
fn scheme_bindings(
    part: &crate::nodes::sim::render::building_lod::spatial::CatalogPart,
) -> VarArray {
    let mut schemes = VarArray::new();
    for scheme in &part.schemes {
        let mut lods = VarArray::new();
        for bindings in &scheme.lods {
            let mut materials = VarArray::new();
            for binding in bindings {
                let mut item = VarDictionary::new();
                item.set("material", binding.material.as_str());
                for (channel, path) in [
                    ("albedo", &binding.albedo),
                    ("orm", &binding.orm),
                    ("normal", &binding.normal),
                    ("emission", &binding.emission),
                ] {
                    if let Some(path) = path {
                        item.set(channel, path.as_str());
                    }
                }
                materials.push(&item.to_variant());
            }
            lods.push(&materials.to_variant());
        }
        let mut item = VarDictionary::new();
        item.set("id", scheme.id.as_str());
        item.set("lods", lods);
        schemes.push(&item.to_variant());
    }
    schemes
}
