// SPDX-License-Identifier: GPL-2.0-only

//! Spatial-index adapter for the pure LOD batcher. Cached state belongs to the render
//! consumer, never the simulation/save. Residency visits the existing 512 m center index.

use super::{BatchKey, Chunk, Instance, Part, View};
use crate::assets::AssetRegistry;
use crate::nodes::sim::render::buildings::{
    building_part_pose, construction_rise_offset_m, construction_visual_progress,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use glam::{Mat4, Vec3};
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

/// Stable resource identity within a catalog generation.
pub(crate) struct CatalogPart {
    /// Shared asset-wide HDR emission strength, bound once per material variant.
    pub window_brightness: f32,
    /// Qualified pack/asset identifier.
    pub asset: String,
    /// Authored mesh-part ordinal, unaffected by another part failing to import.
    pub index: usize,
    /// Complete authored tier chain in native filesystem paths.
    pub paths: Vec<String>,
    /// Texture replacements for each authored scheme, in manifest order so that a
    /// [`crate::assets::asset::BuildingAppearance::scheme_for`] ordinal indexes it directly.
    pub schemes: Vec<SchemeBinding>,
}

/// One scheme's resolved replacements for a single catalog part.
pub(crate) struct SchemeBinding {
    /// Authored scheme identifier, published for diagnostics only.
    pub id: String,
    /// Replacements for each authored tier, nearest first. An empty tier keeps its source
    /// materials, which is also what an asset without schemes produces.
    pub lods: Vec<Vec<MaterialBinding>>,
}

/// Replacement textures for one source material of one tier.
pub(crate) struct MaterialBinding {
    /// Exact source material name inside the imported tier.
    pub material: String,
    /// Absolute channel paths; an absent channel retains the source texture.
    pub albedo: Option<String>,
    /// Packed occlusion/roughness/metallic replacement.
    pub orm: Option<String>,
    /// Tangent-space normal replacement.
    pub normal: Option<String>,
    /// Emission replacement.
    pub emission: Option<String>,
}

/// Candidate inputs and GPU groups retained while a spatial chunk is resident.
#[derive(Default)]
pub(crate) struct Resident {
    /// Pure packed-buffer cache.
    pub batches: Chunk,
    instances: Vec<Instance>,
    revision: Option<(u64, u64)>,
    construction: bool,
}

/// Render-consumer state. No field is simulation authority or serialized save data.
#[derive(Default)]
pub(crate) struct SpatialBatches {
    /// Stable resource description, loaded only at registry boundaries.
    pub catalog: Vec<CatalogPart>,
    /// Policy and visibility geometry for each catalog slot.
    pub parts: Vec<Part>,
    /// Invalidates stale resource availability and frame requests.
    pub generation: u64,
    /// Registry revision from which this catalog was built.
    pub registry_revision: u64,
    /// Only chunks in the current conservative visibility query.
    pub chunks: HashMap<(i32, i32), Resident>,
    /// Deterministic publication order independent of Rayon/hash iteration.
    pub ordered_chunks: Vec<(i32, i32)>,
    /// GPU groups to retire before publishing replacement groups.
    pub removed: Vec<((i32, i32), BatchKey)>,
    /// Number of evaluated parts in the latest update.
    pub evaluated_parts: usize,
    /// Number of spatial hash cells queried in the latest update.
    pub queried_chunks: usize,
    /// Maximum geometry reach from an indexed building center.
    pub horizontal_margin: f32,
    /// Maximum geometry height above support, for shadow-caster extrusion.
    pub top_margin: f32,
    assets: HashMap<String, Range<usize>>,
    last_view: Option<View>,
    last_bounds: Option<[i32; 4]>,
    last_revision: Option<(u64, u64)>,
    last_hour: f32,
    construction: bool,
}

impl SpatialBatches {
    /// Rebuild only at catalog boundaries; runtime ordinals never leak into authored data.
    pub fn load_catalog(&mut self, registry: &AssetRegistry) {
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            generation,
            registry_revision: registry.revision(),
            ..Self::default()
        };
        // Slot zero is the existing missing-asset marker, not an authored tier.
        self.catalog.push(CatalogPart {
            window_brightness: 0.0,
            asset: "broken:error".into(),
            index: 0,
            paths: vec![String::new()],
            schemes: Vec::new(),
        });
        self.parts.push(
            Part::new(
                [Vec3::new(-0.45, 0.0, -0.45), Vec3::new(0.45, 4.0, 0.45)],
                &[true],
            )
            .unwrap(),
        );
        self.horizontal_margin =
            0.45 * std::f32::consts::SQRT_2 * crate::config::BUILDING_VISUAL_SCALE;
        self.top_margin = 4.0 * crate::config::BUILDING_VISUAL_SCALE;
        let mut ids: Vec<_> = registry.qualified_ids().collect();
        ids.sort_unstable();
        for id in ids {
            let entry = registry.get(id).unwrap();
            let start = self.parts.len();
            for (index, part) in entry.manifest.mesh_parts.iter().enumerate() {
                let bounds = part.imported_bounds.unwrap_or([[0.0; 3]; 2]);
                let bounds = bounds.map(Vec3::from_array);
                self.parts
                    .push(Part::new(bounds, &vec![true; part.lods.len()]).unwrap());
                self.catalog.push(CatalogPart {
                    window_brightness: entry.manifest.building.as_ref().map_or(
                        crate::assets::asset::BuildingData::default_window_brightness(),
                        |building| building.window_brightness,
                    ),
                    asset: id.into(),
                    index,
                    paths: part
                        .lods
                        .iter()
                        .map(|lod| {
                            Path::new(&entry.asset_dir)
                                .join(&lod.file)
                                .to_string_lossy()
                                .into_owned()
                        })
                        .collect(),
                    schemes: scheme_bindings(entry, part),
                });
            }
            if self.parts.len() != start {
                self.assets.insert(id.into(), start..self.parts.len());
            }
        }
        self.refresh_geometry_margins(registry);
    }

    /// Expand residency for all successfully imported tiers, including oversized coarse
    /// geometry. This is a catalog-time operation, never camera-time asset traversal.
    pub fn refresh_geometry_margins(&mut self, registry: &AssetRegistry) {
        for (catalog, part) in self.catalog.iter().zip(&self.parts) {
            let Some(entry) = registry.get(&catalog.asset) else {
                continue;
            };
            let local = entry.manifest.mesh_parts[catalog.index].local_transform();
            for corner in 0..8 {
                let p = Vec3::new(
                    part.cull_bounds[corner & 1].x,
                    part.cull_bounds[(corner >> 1) & 1].y,
                    part.cull_bounds[(corner >> 2) & 1].z,
                );
                let p = local.transform_point3(p);
                self.horizontal_margin = self.horizontal_margin.max(p.x.hypot(p.z));
                self.top_margin = self.top_margin.max(p.y);
            }
        }
    }

    /// Discard per-world references after load/reset without reimporting unchanged assets.
    pub fn invalidate_world(&mut self) {
        for &key in &self.ordered_chunks {
            let chunk = &self.chunks[&key];
            for batch in &chunk.batches.batches {
                self.removed.push((key, batch.key));
            }
        }
        self.chunks.clear();
        self.ordered_chunks.clear();
        self.last_revision = None;
        self.last_view = None;
        self.construction = false;
    }

    /// Returns false for an unchanged frame. Rectangle lookup is O(queried chunks);
    /// candidate preparation/LOD work is O(parts + batch groups), independent of background N.
    pub fn update(
        &mut self,
        allocator: &BuildingAllocator,
        bounds: [i32; 4],
        view: View,
        hour: f32,
    ) -> bool {
        self.evaluated_parts = 0;
        self.queried_chunks = 0;
        let revision = (
            allocator.building_ref_revision(),
            allocator.building_visual_revision,
        );
        if self.last_view == Some(view)
            && self.last_bounds == Some(bounds)
            && self.last_revision == Some(revision)
            && (!self.construction || self.last_hour == hour)
        {
            return false;
        }
        let reset_history = self.last_revision.is_none_or(|last| last.0 != revision.0);
        let quality_changed = self
            .last_view
            .is_some_and(|last| last.quality != view.quality);
        let camera_changed = self.last_view != Some(view);
        self.last_view = Some(view);
        self.last_bounds = Some(bounds);
        self.last_revision = Some(revision);
        self.last_hour = hour;
        for &(x, z) in &self.ordered_chunks {
            if x < bounds[0] || z < bounds[1] || x > bounds[2] || z > bounds[3] {
                let chunk = self.chunks.remove(&(x, z)).unwrap();
                for batch in &chunk.batches.batches {
                    self.removed.push(((x, z), batch.key));
                }
            }
        }
        self.ordered_chunks.clear();
        for x in bounds[0]..=bounds[2] {
            for z in bounds[1]..=bounds[3] {
                self.queried_chunks += 1;
                if allocator.building_chunks.contains_key(&(x, z))
                    || self.chunks.contains_key(&(x, z))
                {
                    self.chunks.entry((x, z)).or_default();
                    self.ordered_chunks.push((x, z));
                }
            }
        }
        let parts = &self.parts;
        let catalog = &self.catalog;
        let assets = &self.assets;
        // par_bridge borrows the map; HashMap::par_iter_mut would first allocate a staging Vec.
        self.chunks
            .iter_mut()
            .par_bridge()
            .for_each(|(key, chunk)| {
                let changed = chunk.revision != Some(revision) || chunk.construction;
                if changed {
                    chunk.instances.clear();
                    chunk.construction = false;
                    if let Some(indices) = allocator.building_chunks.get(key) {
                        for &id in indices {
                            let building = &allocator.buildings[id];
                            chunk.construction |= building.is_under_construction();
                            let entry = allocator.registry.get(&building.asset_id);
                            let range = assets.get(&building.asset_id);
                            if building.broken || entry.is_none() {
                                if !building.is_under_construction() {
                                    let scale = crate::config::BUILDING_VISUAL_SCALE;
                                    chunk.instances.push(Instance {
                                        building: id,
                                        part: 0,
                                        deserted: false,
                                        scheme: 0,
                                        lighting: [0.0; 4],
                                        transform: Mat4::from_scale_rotation_translation(
                                            Vec3::splat(scale),
                                            glam::Quat::from_rotation_y(
                                                building.facing_dir.x.atan2(building.facing_dir.y),
                                            ),
                                            Vec3::new(
                                                building.center_x,
                                                building.support_height_m,
                                                building.center_y,
                                            ),
                                        ),
                                    });
                                }
                                continue;
                            }
                            // Authored meshless sites (for example production fields) are
                            // valid; their ground geometry must not become a missing-asset marker.
                            let Some(range) = range else { continue };
                            let entry = entry.unwrap();
                            let deserted =
                                building.is_deserted && !building.is_under_construction();
                            // A deserted building draws one flat override material, so a
                            // scheme group would differ only in bindings nothing samples.
                            // Collapsing them keeps the extra groups to occupied schemes.
                            let scheme = if deserted {
                                0
                            } else {
                                entry
                                    .manifest
                                    .building
                                    .as_ref()
                                    .and_then(|data| data.appearance.as_ref())
                                    .map_or(0, |appearance| {
                                        appearance.scheme_for(building.appearance_key())
                                    }) as u16
                            };
                            let mut y = building.support_height_m;
                            if building.is_under_construction() {
                                y -= construction_rise_offset_m(
                                    building,
                                    construction_visual_progress(building, hour),
                                );
                            }
                            let lighting = super::window_lighting::parameters(
                                building.appearance_key(),
                                building.zone_type == ZoneType::Residential,
                                building.is_deserted || building.is_under_construction(),
                            );
                            for part in range.clone() {
                                chunk.instances.push(Instance {
                                    building: id,
                                    part,
                                    deserted,
                                    scheme,
                                    lighting,
                                    transform: building_part_pose(
                                        Vector2::new(building.center_x, building.center_y),
                                        y,
                                        building.facing_dir,
                                        Some(entry),
                                        &entry.manifest.mesh_parts[catalog[part].index],
                                    ),
                                });
                            }
                        }
                    }
                    chunk
                        .batches
                        .synchronize(&chunk.instances, parts, reset_history);
                    chunk.revision = Some(revision);
                }
                if changed || camera_changed {
                    chunk.batches.update(parts, view, quality_changed);
                }
            });
        self.construction = self.chunks.values().any(|chunk| chunk.construction);
        self.evaluated_parts = self
            .chunks
            .values()
            .map(|chunk| chunk.batches.evaluations)
            .sum();
        true
    }

    /// Acknowledge only after a complete coherent frame has been copied to the Godot bridge.
    pub fn acknowledge(&mut self) {
        self.removed.clear();
        for chunk in self.chunks.values_mut() {
            chunk.batches.acknowledge();
        }
    }
}

/// Resolve one part's authored schemes into absolute texture paths. Catalog-time only:
/// scheme lookup happens once per registry revision, never per frame or per instance.
fn scheme_bindings(
    entry: &crate::assets::AssetEntry,
    part: &crate::assets::asset::MeshPart,
) -> Vec<SchemeBinding> {
    let Some(appearance) = entry
        .manifest
        .building
        .as_ref()
        .and_then(|building| building.appearance.as_ref())
    else {
        return Vec::new();
    };
    let absolute = |relative: &Option<String>| {
        relative.as_ref().map(|path| {
            Path::new(&entry.asset_dir)
                .join(path)
                .to_string_lossy()
                .into_owned()
        })
    };
    appearance
        .schemes
        .iter()
        .map(|scheme| {
            let mut lods: Vec<Vec<MaterialBinding>> =
                part.lods.iter().map(|_| Vec::new()).collect();
            for entry in scheme.overrides.iter().filter(|o| o.part == part.name) {
                // A stale manifest can name fewer materials than the part has tiers; those
                // tiers simply keep their source materials rather than failing the catalog.
                for (lod, material) in entry.materials.iter().enumerate().take(lods.len()) {
                    lods[lod].push(MaterialBinding {
                        material: material.clone(),
                        albedo: absolute(&entry.albedo),
                        orm: absolute(&entry.orm),
                        normal: absolute(&entry.normal),
                        emission: absolute(&entry.emission),
                    });
                }
            }
            SchemeBinding {
                id: scheme.id.clone(),
                lods,
            }
        })
        .collect()
}

/// Derive a finite conservative chunk rectangle from the view frustum and its directional
/// shadow extrusion. No tiny-object culling is part of this query. Invalid views are rejected.
pub(crate) fn query_bounds(
    view: View,
    camera: Vec3,
    camera_forward: Vec3,
    shadow_to_light: Vec3,
    shadow_distance: f32,
    horizontal_margin: f32,
    max_caster_y: f32,
    occupied: [i32; 4],
) -> Option<[i32; 4]> {
    let inverse = view.world_to_clip.inverse();
    if !inverse.is_finite() {
        return None;
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let mut shadow_lo = lo;
    let mut shadow_hi = hi;
    for corner in 0..8 {
        let clip = Vec3::new(
            if corner & 1 == 0 { -1.0 } else { 1.0 },
            if corner & 2 == 0 { -1.0 } else { 1.0 },
            if corner & 4 == 0 { -1.0 } else { 1.0 },
        );
        let point = inverse.project_point3(clip);
        if !point.is_finite() {
            return None;
        }
        lo = lo.min(point);
        hi = hi.max(point);
        let near = inverse.project_point3(Vec3::new(clip.x, clip.y, -1.0));
        let depth = (point - camera).dot(camera_forward);
        let near_depth = (near - camera).dot(camera_forward);
        let shadow_point = if depth > shadow_distance && depth > near_depth {
            near + (point - near)
                * ((shadow_distance - near_depth) / (depth - near_depth)).clamp(0.0, 1.0)
        } else {
            point
        };
        shadow_lo = shadow_lo.min(shadow_point);
        shadow_hi = shadow_hi.max(shadow_point);
    }
    if shadow_distance > 0.0 && max_caster_y > shadow_lo.y {
        if shadow_to_light.y <= 0.0 {
            // Horizontal light has an unbounded conservative caster extrusion.
            return Some(occupied);
        }
        let extrusion = shadow_to_light * ((max_caster_y - shadow_lo.y) / shadow_to_light.y);
        lo = lo.min(shadow_lo + extrusion.min(Vec3::ZERO));
        hi = hi.max(shadow_hi + extrusion.max(Vec3::ZERO));
    }
    let size = RegionGraph::CHUNK_SIZE;
    Some([
        (((lo.x - horizontal_margin) / size).floor() as i32).max(occupied[0]),
        (((lo.z - horizontal_margin) / size).floor() as i32).max(occupied[1]),
        (((hi.x + horizontal_margin) / size).floor() as i32).min(occupied[2]),
        (((hi.z + horizontal_margin) / size).floor() as i32).min(occupied[3]),
    ])
}
