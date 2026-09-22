// SPDX-License-Identifier: GPL-2.0-only

//! Thin main-thread bridge for shared building resources and coherent spatial LOD batches.

use super::*;
use crate::nodes::lod_policy::quality_from_id;
use crate::nodes::sim::render::building_lod::{Part, View, spatial::query_bounds};
use glam::{Mat4, Vec2, Vec3};

#[godot_api(secondary)]
impl SimulationNode {
    /// Enumerate complete tier chains once at world loading/pack refresh. Returned slot IDs
    /// are presentation-local and valid only for the accompanying catalog generation.
    #[func]
    pub fn get_building_lod_catalog(&mut self) -> VarDictionary {
        let shared = Arc::clone(&self.core);
        let core = shared.lock().expect("simulation core lock poisoned");
        self.building_lods.load_catalog(&core.allocator.registry);
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
        let shared = Arc::clone(&self.core);
        let core = shared.lock().expect("simulation core lock poisoned");
        self.building_lods
            .refresh_geometry_margins(&core.allocator.registry);
        self.building_lods.invalidate_world();
        true
    }

    /// Nonblocking, camera-driven building rendering. Returns only changed chunk groups;
    /// a busy core retains the previous complete frame. Resource loading never occurs here.
    #[func]
    pub fn try_get_building_lod_frame(
        &mut self,
        generation: i64,
        world_to_clip: Projection,
        render_size: Vector2,
        camera_position: Vector3,
        camera_forward: Vector3,
        quality: i32,
        shadow_to_light: Vector3,
        shadow_distance: f32,
    ) -> VarDictionary {
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
        if core.allocator.dirty_index {
            core.allocator.rebuild_zone_index();
        }
        let view = View {
            world_to_clip: Mat4::from_cols_array_2d(
                &world_to_clip.cols.map(|c| [c.x, c.y, c.z, c.w]),
            ),
            render_size: Vec2::new(render_size.x, render_size.y),
            quality: quality_from_id(quality),
        };
        let to_vec = |p: Vector3| Vec3::new(p.x, p.y, p.z);
        let bounds = if let Some(occupied) = core.allocator.building_chunk_bounds {
            let Some(bounds) = query_bounds(
                view,
                to_vec(camera_position),
                to_vec(camera_forward),
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
        let changed = lods.update(
            &core.allocator,
            bounds,
            view,
            core.operational_hour_fraction(),
        );
        result.set("busy", false);
        result.set("evaluated_parts", lods.evaluated_parts as i64);
        result.set("queried_chunks", lods.queried_chunks as i64);
        if !changed && lods.removed.is_empty() {
            return result;
        }
        let mut updates = VarArray::new();
        // World replacement can reuse the same chunk/group key. Retire the old
        // generation first, then install the new groups within this same frame.
        for &(key, batch) in &lods.removed {
            let mut item = batch_identity(key, batch);
            item.set("transforms", PackedFloat32Array::new());
            item.set("retired", true);
            updates.push(&item.to_variant());
        }
        for &key in &lods.ordered_chunks {
            let chunk = &lods.chunks[&key].batches;
            for batch in &chunk.batches {
                if !batch.changed {
                    continue;
                }
                let mut item = batch_identity(key, batch.key);
                if batch.count != 0 {
                    let [lo, hi] = chunk.bounds(batch, &lods.parts[batch.key.part]);
                    item.set(
                        "bounds",
                        Aabb::new(
                            Vector3::new(lo.x, lo.y, lo.z),
                            Vector3::new(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z),
                        ),
                    );
                }
                item.set(
                    "transforms",
                    PackedFloat32Array::from(chunk.transforms(batch).as_flattened()),
                );
                updates.push(&item.to_variant());
            }
        }
        result.set("updates", updates);
        lods.acknowledge();
        result
    }
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
