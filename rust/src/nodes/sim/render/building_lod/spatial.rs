// SPDX-License-Identifier: GPL-2.0-only

//! Spatial-index adapter for the pure LOD batcher. Cached state belongs to the render
//! consumer, never the simulation/save. Residency visits the existing 512 m center index.

use super::{BatchKey, Chunk, Instance, Motion, Part, View};
use crate::assets::AssetRegistry;
use crate::nodes::sim::render::stream::layers::buildings::Change;
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use crate::nodes::sim::render::buildings::{
    building_part_pose, construction_rise_offset_m, construction_visual_progress,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use glam::{DMat3, DVec3, Mat3, Mat4, Vec2, Vec3};
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Chunks kept with their instances after they leave the query, so a turn that brings them back
/// does not gather their buildings again (`RENDER-34`): about a ring of 512 m chunks around the
/// rule's view-shaped query.
const RETAINED: usize = 64;

/// Real-time length of the rise that presents an in-place level change. Presentation only:
/// the simulation swapped the asset instantly and keeps running underneath.
const LEVEL_CHANGE_RISE: Duration = Duration::from_millis(2500);

/// Ease-out share of the construction rise completed `elapsed` after a level change.
pub(super) fn level_change_rise_progress(elapsed: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / LEVEL_CHANGE_RISE.as_secs_f32()).clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

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

/// The camera as the travel buckets see it (`RENDER-34`): where it is, how it is turned, and the
/// projection `View::world_to_clip` was made with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pose {
    /// The eye.
    pub eye: Vec3,
    /// The camera's basis (columns: its x, y and z axes in the world).
    pub basis: Mat3,
    /// The camera's projection.
    pub projection: Mat4,
}

// What of a projection the sizes depend on: x and y scale and offset, and the near plane.
fn projection_terms(projection: &Mat4) -> Option<[f32; 5]> {
    let [x, y, z, w] = projection.to_cols_array_2d();
    // Perspective only: the clip w is the depth.
    if z[3] != -1.0 || w[3] != 0.0 || x[3] != 0.0 || y[3] != 0.0 || z[2] >= 1.0 {
        return None;
    }
    let near = -w[2] / (1.0 - z[2]);
    (near.is_finite() && near > 0.0).then_some([x[0], y[1], z[0], z[1], near])
}

// The camera's rotation, orthonormalised (Gram-Schmidt, x then y).
fn orthonormal(basis: Mat3) -> DMat3 {
    let x = basis.x_axis.as_dvec3().normalize();
    let y = basis.y_axis.as_dvec3();
    let y = (y - x * x.dot(y)).normalize();
    DMat3::from_cols(x, y, x.cross(y))
}

// The path lengths the travel buckets read, and the last pose they were measured from.
#[derive(Default)]
pub(super) struct Travel {
    last: Option<(Pose, DMat3, [f32; 5], Vec2, crate::assets::lod_policy::LodQuality)>,
    travel: f64,
    turn: f64,
}

impl Travel {
    // This frame's motion: the path lengths advanced by the move from the last pose, or every
    // part evaluated when what the sizes depend on besides the camera's place changed.
    pub(super) fn advance(&mut self, pose: Option<Pose>, view: View) -> Motion {
        let Some(pose) = pose else {
            self.last = None;
            return Motion::ALL;
        };
        let Some(terms) = projection_terms(&pose.projection) else {
            self.last = None;
            return Motion::ALL;
        };
        let rotation = orthonormal(pose.basis);
        let mut all = true;
        if let Some((last, last_rotation, last_terms, size, quality)) = self.last
            && size == view.render_size
            && quality == view.quality
            && last_terms[..4] == terms[..4]
            && ((last_terms[4] - terms[4]) / terms[4]).abs() < 1e-5
        {
            all = false;
            if last.eye != pose.eye {
                self.travel += f64::from(last.eye.distance(pose.eye)) + 1e-6;
            }
            if last.basis != pose.basis {
                // |R - R0| = 2 sin(angle / 2) = sqrt(3 - tr(R R0^T)) for rotations.
                let trace = DVec3::new(
                    rotation.x_axis.dot(last_rotation.x_axis),
                    rotation.y_axis.dot(last_rotation.y_axis),
                    rotation.z_axis.dot(last_rotation.z_axis),
                )
                .element_sum();
                self.turn += (3.0 - trace).max(0.0).sqrt() + 1e-6;
            }
        }
        self.last = Some((pose, rotation, terms, view.render_size, view.quality));
        Motion {
            all,
            bounded: true,
            travel: self.travel,
            turn: self.turn,
            eye: pose.eye,
            rotation: rotation.transpose().as_mat3(),
            scale: [
                terms[0].abs() * view.render_size.x * 0.5,
                terms[1].abs() * view.render_size.y * 0.5,
            ],
            near: terms[4],
        }
    }
}

/// Candidate inputs and GPU groups retained while a spatial chunk is resident.
#[derive(Default)]
pub(crate) struct Resident {
    /// Pure packed-buffer cache.
    pub batches: Chunk,
    instances: Vec<Instance>,
    revision: Option<(u64, u64)>,
    construction: bool,
    // The construction clock the instances were placed at.
    clock: f64,
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
    last_clock_slots: f64,
    construction: bool,
    // Level-change log total already consumed; `None` until first seen, so a new renderer or
    // world never replays changes it did not witness.
    level_changes_seen: Option<u64>,
    // Rising level changes by (parcel, generation) with their real-time start.
    level_change_started: HashMap<(u64, u32), Instant>,
    // Whether the previous frame animated, so the frame after the last rise settles it.
    level_change_animating: bool,
    travel: Travel,
    // Chunks that left the query, at most RETAINED of them (`RENDER-34`).
    retained: HashMap<(i32, i32), Resident>,
    /// Microseconds of the latest update spent placing chunks' instances and evaluating their
    /// parts, summed over the workers (the perf channel).
    pub placing_us: f64,
    /// See `placing_us`.
    pub evaluating_us: f64,
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
        self.retained.clear();
        self.ordered_chunks.clear();
        self.last_revision = None;
        self.last_view = None;
        self.travel.last = None;
        self.construction = false;
        self.level_changes_seen = None;
        self.level_change_started.clear();
        self.level_change_animating = false;
    }

    /// Returns false for an unchanged frame. Rectangle lookup is O(queried chunks); candidate
    /// preparation is O(parts of changed chunks); LOD work is O(resident chunks) plus, in chunks
    /// whose least threshold the camera's motion passed, O(parts) threshold checks and O(due
    /// parts) evaluations (every part without a `pose`), independent of background N.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        allocator: &BuildingAllocator,
        bounds: [i32; 4],
        view: View,
        pose: Option<Pose>,
        clock_slots: f64,
        now: Instant,
    ) -> bool {
        self.evaluated_parts = 0;
        self.queried_chunks = 0;
        self.placing_us = 0.0;
        self.evaluating_us = 0.0;
        let revision = (
            allocator.building_ref_revision(),
            allocator.building_visual_revision,
        );
        let (total, recorded) = allocator
            .level_changes
            .since(self.level_changes_seen.unwrap_or(u64::MAX));
        let mut fresh = false;
        if self.level_changes_seen.is_some() {
            for &key in recorded {
                self.level_change_started.insert(key, now);
                fresh = true;
            }
        }
        self.level_changes_seen = Some(total);
        self.level_change_started
            .retain(|_, start| now.duration_since(*start) < LEVEL_CHANGE_RISE);
        let animating = !self.level_change_started.is_empty();
        let settling = std::mem::replace(&mut self.level_change_animating, animating);
        if self.last_view == Some(view)
            && self.last_bounds == Some(bounds)
            && self.last_revision == Some(revision)
            && (!self.construction || self.last_clock_slots == clock_slots)
            && !animating
            && !settling
        {
            return false;
        }
        let reset_history = self.last_revision.is_none_or(|last| last.0 != revision.0);
        let quality_changed = self
            .last_view
            .is_some_and(|last| last.quality != view.quality);
        let camera_changed = self.last_view != Some(view);
        // Chunks updated without a camera change are resynchronized ones, which evaluate every
        // part whatever the motion says.
        let motion = self.travel.advance(pose, view);
        self.last_view = Some(view);
        self.last_bounds = Some(bounds);
        self.last_revision = Some(revision);
        self.last_clock_slots = clock_slots;
        for &(x, z) in &self.ordered_chunks {
            if x < bounds[0] || z < bounds[1] || x > bounds[2] || z > bounds[3] {
                let chunk = self.chunks.remove(&(x, z)).unwrap();
                for batch in &chunk.batches.batches {
                    self.removed.push(((x, z), batch.key));
                }
                // Kept with its instances, so coming back costs no gathering (`RENDER-34`).
                self.retained.insert((x, z), chunk);
            }
        }
        self.ordered_chunks.clear();
        // A retained chunk's instances are current while the revision holds and nothing about
        // it animates; it then comes back as a new chunk would, without history.
        let reusable = |chunk: &Resident| {
            chunk.revision == Some(revision)
                && (!chunk.construction || chunk.clock == clock_slots)
                && !fresh
                && !animating
        };
        for x in bounds[0]..=bounds[2] {
            for z in bounds[1]..=bounds[3] {
                self.queried_chunks += 1;
                if allocator.building_chunks.contains_key(&(x, z))
                    || self.chunks.contains_key(&(x, z))
                {
                    if !self.chunks.contains_key(&(x, z)) {
                        let chunk = match self.retained.remove(&(x, z)) {
                            Some(mut chunk) if reusable(&chunk) => {
                                chunk.batches.reenter();
                                chunk
                            }
                            _ => Resident::default(),
                        };
                        self.chunks.insert((x, z), chunk);
                    }
                    self.ordered_chunks.push((x, z));
                }
            }
        }
        self.evict_retained(bounds);
        let parts = &self.parts;
        let catalog = &self.catalog;
        let assets = &self.assets;
        let rising = &self.level_change_started;
        // A fresh level change restarts its building's rise even without a revision bump. Sites
        // under construction move with the construction clock and a rise with real time: their
        // chunks are placed again when the clock moved or a rise animates (or just ended), not
        // on every camera move (`RENDER-34`), since the same clock places them the same.
        let placed_again = |chunk: &Resident| {
            chunk.revision != Some(revision)
                || fresh
                || chunk.construction && (chunk.clock != clock_slots || animating || settling)
        };
        // Nothing to place or evaluate: no dispatch to the pool at all. O(resident chunks).
        let work = self.chunks.values().any(|chunk| {
            placed_again(chunk)
                || chunk.batches.needs_update()
                || camera_changed && (quality_changed || chunk.batches.is_due(&motion))
        });
        // Placing chunks or evaluating whole ones goes to the pool; the parts a camera move
        // brings due are few (their thresholds), so those frames stay on the main thread, which
        // saves the pool's wake-up (`RENDER-34`).
        let heavy = self.chunks.values().any(|chunk| {
            placed_again(chunk)
                || chunk.batches.needs_update()
                || camera_changed && (quality_changed || motion.all)
        });
        let (gather_ns, evaluate_ns) = (AtomicU64::new(0), AtomicU64::new(0));
        let process = |key: &(i32, i32), chunk: &mut Resident, parallel: bool| {
                    let changed = placed_again(chunk);
                    let start = Instant::now();
                    if changed {
                        chunk.instances.clear();
                        chunk.construction = false;
                        chunk.clock = clock_slots;
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
                                                    building
                                                        .facing_dir
                                                        .x
                                                        .atan2(building.facing_dir.y),
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
                                // Authored meshless sites (for example production fields)
                                // are valid; their ground geometry must not become a
                                // missing-asset marker.
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
                                        construction_visual_progress(building, clock_slots),
                                    );
                                } else if !rising.is_empty()
                                    && let Some(start) =
                                        rising.get(&(building.parcel_id, building.build_generation))
                                {
                                    // Rebuilt every frame like construction until the rise ends.
                                    chunk.construction = true;
                                    y -= construction_rise_offset_m(
                                        building,
                                        level_change_rise_progress(now.duration_since(*start)),
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
                        gather_ns.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    }
                    if changed || camera_changed || chunk.batches.needs_update() {
                        let start = Instant::now();
                        chunk
                            .batches
                            .update_due(parts, view, quality_changed, &motion, parallel);
                        evaluate_ns.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    }
        };
        if heavy {
            // par_bridge borrows the map; HashMap::par_iter_mut would first allocate a staging
            // Vec. Godot's main thread rebuilds the changed chunks: shared pool (CODE-17).
            crate::thread_pools::install(|| {
                self.chunks
                    .iter_mut()
                    .par_bridge()
                    .for_each(|(key, chunk)| process(key, chunk, true));
            });
        } else if work {
            for (key, chunk) in self.chunks.iter_mut() {
                process(key, chunk, false);
            }
        }
        self.placing_us = gather_ns.into_inner() as f64 / 1000.0;
        self.evaluating_us = evaluate_ns.into_inner() as f64 / 1000.0;
        self.construction = self.chunks.values().any(|chunk| chunk.construction);
        self.evaluated_parts = self
            .chunks
            .values()
            .map(|chunk| chunk.batches.evaluations)
            .sum();
        true
    }

    /// The frame's changed batches in publication order (chunks in query order, groups in key
    /// order) with their instances and bounds. O(resident batches + changed instances).
    pub fn changes(&self) -> impl Iterator<Item = Change<'_>> + Clone + '_ {
        self.ordered_chunks.iter().flat_map(move |&key| {
            let chunk = &self.chunks[&key].batches;
            chunk
                .batches
                .iter()
                .filter(|batch| batch.changed)
                .map(move |batch| {
                    let bounds = if batch.count == 0 {
                        Aabb::default()
                    } else {
                        let [lo, hi] = chunk.bounds(batch, &self.parts[batch.key.part]);
                        Aabb::new(lo.to_array(), (hi - lo).to_array())
                    };
                    Change {
                        chunk: key,
                        key: batch.key,
                        rows: chunk.transforms(batch),
                        bounds,
                    }
                })
        })
    }

    // Over RETAINED, the retained chunks farthest from the query's centre go (ties by key).
    // O(retained) per chunk evicted; a turn evicts about one a frame.
    fn evict_retained(&mut self, bounds: [i32; 4]) {
        let centre = [
            i64::from(bounds[0]) + i64::from(bounds[2]),
            i64::from(bounds[1]) + i64::from(bounds[3]),
        ];
        while self.retained.len() > RETAINED {
            let farthest = self
                .retained
                .keys()
                .map(|&(x, z)| {
                    let (dx, dz) = (2 * i64::from(x) - centre[0], 2 * i64::from(z) - centre[1]);
                    (dx * dx + dz * dz, (x, z))
                })
                .max()
                .map(|(_, key)| key)
                .expect("a retained chunk");
            self.retained.remove(&farthest);
        }
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
#[allow(clippy::too_many_arguments)]
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
