// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic vegetation queries and bounded player edits over the saved generator.

use super::*;
use crate::simulation::vegetation::edits::{
    AuthoredPlant as Plant, VegetationCell, VegetationLayer, pack_patch_key,
};
use crate::simulation::vegetation::{hash, unit};

mod land_cover;

/// Species ordinals shared with the renderer's mesh table.
const SPECIES_CONIFER: f32 = 0.0;
const SPECIES_BROADLEAF: f32 = 1.0;
const SPECIES_BUSH: f32 = 2.0;
const SPECIES_ROCK: f32 = 3.0;

// Layer salts keep the two grids independent, so understory placement is not a scaled copy of
// the canopy. The world seed is mixed in on top so two worlds on the same terrain arrange their
// plants differently; the multiply spreads small seeds across the whole word before the per-use
// offsets below are added.
const CANOPY_SALT: u32 = 0;
const UNDERSTORY_SALT: u32 = 64;

// A regular 4 m planting grid gives 625 points/ha, independent of generator density.
const BRUSH_SPACING_M: f32 = 4.0;
const MAX_PAINT_RADIUS_M: f32 = 256.0;

fn brush_candidate(x: i32, z: i32, salt: u32) -> [f32; 4] {
    let salt = salt.wrapping_add(128);
    let [_, _, yaw, scale] = candidate(x, z, BRUSH_SPACING_M, salt);
    [
        (x as f32 + 0.5) * BRUSH_SPACING_M + (unit(x, z, salt.wrapping_add(1)) - 0.5) * 0.4,
        (z as f32 + 0.5) * BRUSH_SPACING_M + (unit(x, z, salt.wrapping_add(2)) - 0.5) * 0.4,
        yaw,
        scale,
    ]
}

fn layer_base(layer_salt: u32, seed: u32) -> u32 {
    layer_salt ^ seed.wrapping_mul(0x9e37_79b9)
}

// Returns [world x, world z, yaw, scale] for one cell of the given grid.
fn candidate(x: i32, z: i32, cell_m: f32, salt: u32) -> [f32; 4] {
    [
        (x as f32 + 0.15 + unit(x, z, salt.wrapping_add(1)) * 0.7) * cell_m,
        (z as f32 + 0.15 + unit(x, z, salt.wrapping_add(2)) * 0.7) * cell_m,
        unit(x, z, salt.wrapping_add(3)) * std::f32::consts::TAU,
        0.75 + unit(x, z, salt.wrapping_add(4)) * 0.6,
    ]
}

// A missing sample means water or an engineered surface owns the position. The radius is
// the plant's own clearance: a bush does not need a canopy tree's room.
fn clear_footprint(
    x: f32,
    z: f32,
    y: f32,
    radius: f32,
    max_relief: f32,
    sample: impl Fn(f32, f32) -> Option<f32>,
) -> bool {
    [-radius, 0.0, radius].into_iter().all(|dz| {
        [-radius, 0.0, radius]
            .into_iter()
            .all(|dx| sample(x + dx, z + dz).is_some_and(|h| (h - y).abs() <= max_relief))
    })
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Publishes an R8 forest-floor texture on a fixed 8 m world grid, including a filter border.
    /// The payload contains world bounds, dimensions, bytes and the existing terrain/vegetation
    /// revisions of the nine contributing patches. This is derived data, never saved or ticked.
    #[func]
    pub fn get_vegetation_land_cover(&self, key: Vector2i) -> VarDictionary {
        let mut core = self.lock_core();
        let layout = PatchLayout::new(&core);
        if key.x < 0
            || key.y < 0
            || key.x as f32 * layout.span >= layout.half_w * 2.0
            || key.y as f32 * layout.span >= layout.half_h * 2.0
        {
            return VarDictionary::new();
        }
        prepare_sites(&mut core);
        let origin = Vector2::new(
            key.x as f32 * layout.span - layout.half_w,
            key.y as f32 * layout.span - layout.half_h,
        );
        land_cover::Coverage::build(&core, origin, layout.span)
            .payload(land_cover::generations(&core, key))
    }

    /// Compares the two existing revision streams, including neighboring crown contributors.
    /// Performs nine bounded revision lookups without allocating or evaluating any plants.
    #[func]
    pub fn is_vegetation_land_cover_current(
        &self,
        key: Vector2i,
        generations: PackedInt64Array,
    ) -> bool {
        generations.as_slice() == land_cover::generations(&self.lock_core(), key)
    }

    /// Returns scatter records `[local x, height, local z, yaw, scale, species]` for a patch.
    ///
    /// Species are 0 conifer, 1 broadleaf, 2 bush, 3 rock. Both grids are sized by the world's
    /// saved [`VegetationGenerator`], so the same save generates the same plants everywhere. The
    /// denser understory grid is generated only when `include_understory` is set, which the
    /// renderer does for near patches, because it draws that layer over a much shorter range.
    /// Sparse edits add one expected O(1) lookup per cell and O(A) authored output work.
    /// Indexed queries cost O(K * (log N + local hits)) for K bounded candidates
    /// after index preparation. A dirty building index can require one existing O(N) rebuild.
    /// Never called per tick.
    #[func]
    pub fn get_decorative_tree_patch(
        &self,
        origin: Vector2,
        span: f32,
        include_understory: bool,
    ) -> PackedFloat32Array {
        if !span.is_finite() || span <= 0.0 || span > 1024.0 || !origin.is_finite() {
            return PackedFloat32Array::new();
        }
        let mut core = self.lock_core();
        let zone_cell_m = core.zoning.config.zone_cell_m;
        core.allocator
            .prepare_building_site_query_index(zone_cell_m);
        let core = &*core;
        let generator = core.vegetation;
        let mut packed: Vec<f32> = Vec::new();
        packed.extend(scatter_layer(
            core,
            origin,
            span,
            generator.canopy_cell_m,
            CANOPY_SALT,
            true,
        ));
        if include_understory {
            packed.extend(scatter_layer(
                core,
                origin,
                span,
                generator.understory_cell_m,
                UNDERSTORY_SALT,
                false,
            ));
        }
        PackedFloat32Array::from(packed.as_slice())
    }
    /// Removes generated and authored plants in a bounded disc, returning the actual count.
    /// Visits O((radius / cell_m + 1)^2) cells per layer, plus their authored edits.
    #[func]
    pub fn remove_vegetation_at(&self, pos: Vector2, radius: f32) -> i64 {
        remove_at(&mut self.lock_core(), pos, radius) as i64
    }

    /// Plants one renderer species (0..=3), refusing obstructed footprints immediately.
    /// Uses one expected O(1) cell lookup plus the generator's indexed footprint queries.
    #[func]
    pub fn add_vegetation_at(&self, pos: Vector2, species: i64) -> bool {
        add_at(&mut self.lock_core(), pos, species)
    }

    /// Fills a 4 m planting lattice plus generator candidates, returning plants added.
    /// Rejects radii above 256 m: at most 129² fine and 65² canopy candidate slots
    /// (20,866 total at the minimum 8 m canopy spacing), before disc/footprint rejection.
    /// Planning allocates no memory per candidate. Cost is O(K * (A + log N + H)) for
    /// K bounded slots, A authored plants in the owning cell, and H local surface hits;
    /// serialized commits cost expected O(K), independent of remote vegetation edits.
    #[func]
    pub fn paint_vegetation(&self, pos: Vector2, radius: f32, species: i64) -> i64 {
        paint_at(&mut self.lock_core(), pos, radius, species) as i64
    }

    /// Returns a vegetation-only patch revision; terrain payload generations are unaffected.
    #[func]
    pub fn get_vegetation_patch_generation(&self, key: Vector2i) -> i64 {
        self.lock_core()
            .vegetation_edits
            .patch_generation(pack_patch_key(key.x, key.y)) as i64
    }
}

// One layout calculation, using exactly the fields published by get_terrain_patch_layout.
struct PatchLayout {
    span: f32,
    half_w: f32,
    half_h: f32,
}

impl PatchLayout {
    fn new(core: &SimCore) -> Self {
        let (w, h) = core.heightmap.world_size();
        Self {
            span: core.config.terrain_cell_m * core.heightmap.render_patch_interval_cells() as f32,
            half_w: w * 0.5,
            half_h: h * 0.5,
        }
    }

    fn key(&self, x: f32, z: f32) -> i64 {
        pack_patch_key(
            ((x + self.half_w) / self.span).floor() as i32,
            ((z + self.half_h) / self.span).floor() as i32,
        )
    }

    fn reserve_count(&self, radius: f32) -> usize {
        ((radius * 2.0 / self.span).ceil() as usize + 2).pow(2)
    }
}

fn prepare_sites(core: &mut SimCore) {
    core.allocator
        .prepare_building_site_query_index(core.zoning.config.zone_cell_m);
}

fn grid(core: &SimCore, layer: VegetationLayer) -> (f32, u32) {
    let (cell_m, salt) = match layer {
        VegetationLayer::Canopy => (core.vegetation.canopy_cell_m, CANOPY_SALT),
        VegetationLayer::Understory => (core.vegetation.understory_cell_m, UNDERSTORY_SALT),
    };
    (cell_m, layer_base(salt, core.vegetation.config.seed))
}

fn cell_at(pos: Vector2, layer: VegetationLayer, cell_m: f32) -> VegetationCell {
    VegetationCell {
        layer,
        x: (pos.x / cell_m).floor() as i32,
        z: (pos.y / cell_m).floor() as i32,
    }
}

// Reject non-finite/unbounded bridge inputs before integer conversion or work reservation.
fn valid_disc(core: &SimCore, pos: Vector2, radius: f32) -> bool {
    let (w, h) = core.heightmap.world_size();
    pos.is_finite()
        && radius.is_finite()
        && (0.0..=1024.0).contains(&radius)
        && pos.x.abs() <= w * 0.5 + radius
        && pos.y.abs() <= h * 0.5 + radius
}

fn in_disc(plant: &Plant, pos: Vector2, radius: f32) -> bool {
    (plant.x - pos.x).powi(2) + (plant.z - pos.y).powi(2) <= radius * radius
}

fn disc_cells(
    pos: Vector2,
    radius: f32,
    cell_m: f32,
    layer: VegetationLayer,
) -> impl IndexedParallelIterator<Item = VegetationCell> {
    let first = cell_at(pos - Vector2::splat(radius), layer, cell_m);
    let last = cell_at(pos + Vector2::splat(radius), layer, cell_m);
    let nx = last.x - first.x + 1;
    let nz = last.z - first.z + 1;
    (0..nx * nz).into_par_iter().map(move |i| VegetationCell {
        layer,
        x: first.x + i % nx,
        z: first.z + i / nx,
    })
}

fn remove_at(core: &mut SimCore, pos: Vector2, radius: f32) -> usize {
    if !valid_disc(core, pos, radius) {
        return 0;
    }
    prepare_sites(core);
    let layout = PatchLayout::new(core);
    let mut removed = 0;
    for layer in [VegetationLayer::Canopy, VegetationLayer::Understory] {
        let (cell_m, salt) = grid(core, layer);
        // Read-only parallel planning never allocates inside a candidate body. Serialized
        // commits preserve deterministic authored order and exclusive indexed-store ownership.
        let plans: Vec<_> = disc_cells(pos, radius, cell_m, layer)
            .map(|cell| {
                let (tombstoned, added) = core.vegetation_edits.cell(cell);
                let generated = if tombstoned {
                    None
                } else {
                    evaluate_cell(core, cell, cell_m, salt).filter(|p| in_disc(p, pos, radius))
                };
                (generated.is_some() || added.iter().any(|p| in_disc(p, pos, radius)))
                    .then_some((cell, generated))
            })
            .collect();
        let changed_cells = plans.iter().flatten().count();
        if changed_cells == 0 {
            continue;
        }
        core.vegetation_edits
            .reserve(changed_cells, layout.reserve_count(radius));
        for (cell, generated) in plans.into_iter().flatten() {
            if let Some(plant) = generated {
                core.vegetation_edits.set_removed(cell, true);
                core.vegetation_edits
                    .bump_patch(layout.key(plant.x, plant.z));
                removed += 1;
            }
            removed += core.vegetation_edits.remove_added(cell, |plant| {
                in_disc(plant, pos, radius).then(|| layout.key(plant.x, plant.z))
            });
        }
    }
    removed
}

fn add_at(core: &mut SimCore, pos: Vector2, species: i64) -> bool {
    if !(0..=3).contains(&species) || !pos.is_finite() {
        return false;
    }
    prepare_sites(core);
    // Player placements always use the addressable canopy grid, including bushes and rocks;
    // otherwise those placements would disappear outside the renderer's near-only layer.
    let layer = VegetationLayer::Canopy;
    if !placement_clear(core, pos.x, pos.y, layer) {
        return false;
    }
    let (cell_m, salt) = grid(core, layer);
    let cell = cell_at(pos, layer, cell_m);
    let [_, _, yaw, scale] = candidate(cell.x, cell.z, cell_m, salt);
    let key = PatchLayout::new(core).key(pos.x, pos.y);
    core.vegetation_edits.add(
        cell,
        Plant {
            x: pos.x,
            z: pos.y,
            yaw,
            scale,
            species: species as u8,
        },
    );
    core.vegetation_edits.bump_patch(key);
    true
}

fn paint_at(core: &mut SimCore, pos: Vector2, radius: f32, species: i64) -> usize {
    if !(0..=3).contains(&species) || !valid_disc(core, pos, radius) || radius > MAX_PAINT_RADIUS_M
    {
        return 0;
    }
    prepare_sites(core);
    let layer = VegetationLayer::Canopy;
    let (cell_m, salt) = grid(core, layer);
    // Both streams have stable indexed order. Fine-grid indices are temporary coordinates,
    // never edit-store identities: every position is mapped back to its owning canopy cell.
    let generated_points = disc_cells(pos, radius, cell_m, layer)
        .map(|cell| (candidate(cell.x, cell.z, cell_m, salt), true));
    let brush_points = disc_cells(pos, radius, BRUSH_SPACING_M, layer)
        .map(|cell| (brush_candidate(cell.x, cell.z, salt), false));
    let plans: Vec<_> = generated_points
        .chain(brush_points)
        .map(|([x, z, yaw, scale], generator_point)| {
            let cell = cell_at(Vector2::new(x, z), layer, cell_m);
            let plant = Plant {
                x,
                z,
                yaw,
                scale,
                species: species as u8,
            };
            if !in_disc(&plant, pos, radius) {
                return None;
            }
            let (removed, added) = core.vegetation_edits.cell(cell);
            let occupied = added.iter().any(|p| p.x == x && p.z == z);
            if !generator_point {
                // The generator stream owns its exact position even when rejected, avoiding
                // a duplicate if a jittered lattice point ever coincides with that position.
                let [gx, gz, _, _] = candidate(cell.x, cell.z, cell_m, salt);
                return (!occupied && (x != gx || z != gz) && placement_clear(core, x, z, layer))
                    .then_some((cell, plant, false, true));
            }
            let generated = evaluate_cell(core, cell, cell_m, salt);
            if removed {
                // The brush plants the species it was given. Clearing the tombstone regrows
                // whatever the generator chose instead, so that is the same edit only when the
                // two species already agree: painting rock over a cleared spruce regrew the
                // spruce. When they agree the restore is preferred, because it reproduces the
                // identical plant and costs no authored entry for a cleared stand replanted
                // as it was. Otherwise the tombstone stays and the chosen species is authored
                // over it, which is also what a cell hidden by a later surface edit needs.
                match generated {
                    Some(p) if p.species == plant.species => Some((cell, plant, true, true)),
                    _ => (!occupied && placement_clear(core, x, z, layer))
                        .then_some((cell, plant, false, true)),
                }
            } else if generated.is_none() && !occupied && placement_clear(core, x, z, layer) {
                Some((cell, plant, false, true))
            } else {
                None
            }
        })
        .collect();
    let changed_cells = plans.iter().flatten().count();
    if changed_cells == 0 {
        return 0;
    }
    let layout = PatchLayout::new(core);
    core.vegetation_edits
        .reserve(changed_cells, layout.reserve_count(radius));
    let mut count = 0;
    for (cell, plant, restore, visible) in plans.into_iter().flatten() {
        if restore {
            core.vegetation_edits.set_removed(cell, false);
        } else {
            core.vegetation_edits.add(cell, plant);
        }
        core.vegetation_edits
            .bump_patch(layout.key(plant.x, plant.z));
        count += usize::from(visible);
    }
    count
}

// One grid pass. Kept out of the entry point so the two layers cannot share acceptance state,
// the caller reads as two independent passes, and a test can drive a layer without an engine.
pub(super) fn scatter_layer(
    core: &SimCore,
    origin: Vector2,
    span: f32,
    cell_m: f32,
    layer_salt: u32,
    canopy: bool,
) -> Vec<f32> {
    let generator = &core.vegetation;
    let salt = layer_base(layer_salt, generator.config.seed);
    let x0 = (origin.x / cell_m).floor() as i32;
    let z0 = (origin.y / cell_m).floor() as i32;
    let nx = ((origin.x + span) / cell_m).ceil() as i32 - x0;
    let nz = ((origin.y + span) / cell_m).ceil() as i32 - z0;
    let layer = if canopy {
        VegetationLayer::Canopy
    } else {
        VegetationLayer::Understory
    };
    // An unedited grid range pays nothing for the edit store: a handful of coarse block tests
    // replace a lookup per cell, which is the whole per-cell cost of an untouched patch. An
    // authored plant lives in the cell that contains it, so the cell range covers both kinds.
    let edited =
        core.vegetation_edits
            .any_in_cell_range(layer, (x0, x0 + nx - 1), (z0, z0 + nz - 1));
    if !edited && !generator.config.enabled {
        return Vec::new();
    }
    // Indexed collect preserves generator order regardless of Rayon scheduling. Edited patches
    // add one lookup per cell; neither branch allocates in the parallel per-cell body.
    let records: Vec<_> = (0..nx * nz)
        .into_par_iter()
        .map(|i| {
            let cell = VegetationCell {
                layer,
                x: x0 + i % nx,
                z: z0 + i / nx,
            };
            let (removed, added): (bool, &[Plant]) = if edited {
                core.vegetation_edits.cell(cell)
            } else {
                (false, &[])
            };
            let generated = if removed {
                None
            } else {
                evaluate_cell(core, cell, cell_m, salt)
            };
            (generated, added)
        })
        .collect();
    let capacity = records
        .iter()
        .map(|(generated, added)| usize::from(generated.is_some()) + added.len())
        .sum::<usize>()
        * 6;
    // A road, building or terraform placed after an authored plant must clear it, exactly as one
    // placed before it does: clearance is re-evaluated every fetch rather than destroying the
    // stored placement, so a plant under a new surface is hidden and returns if that surface is
    // removed, which is already how a generated candidate behaves. Generated candidates were
    // cleared inside evaluate_cell, so only the authored ones are tested here: one parallel pass
    // and one allocation for the whole patch, and neither for a patch with no authored plant.
    let clearances: Vec<bool> = if records.iter().any(|(_, added)| !added.is_empty()) {
        records
            .par_iter()
            .flat_map_iter(|(_, added)| added.iter())
            .map(|plant| placement_clear(core, plant.x, plant.z, layer))
            .collect()
    } else {
        Vec::new()
    };
    let mut remaining = clearances.as_slice();
    let mut packed = Vec::with_capacity(capacity);
    // Flatten in cell/insertion order into one reserved payload. Only current terrain height is
    // sampled for an authored plant's render Y; it keeps its authored position, yaw and scale.
    for (generated, added) in &records {
        // Both walks cover the same records in the same order, so this hands each cell exactly
        // its own verdicts; a desynchronised pass would panic here rather than misplace one.
        let (cell_clearances, rest) = remaining.split_at(added.len());
        remaining = rest;
        for (plant, clear) in generated
            .iter()
            .map(|plant| (plant, true))
            .chain(added.iter().zip(cell_clearances).map(|(p, &c)| (p, c)))
        {
            let &Plant {
                x,
                z,
                yaw,
                scale,
                species,
            } = plant;
            if !clear {
                continue;
            }
            if x < origin.x || z < origin.y || x >= origin.x + span || z >= origin.y + span {
                continue;
            }
            let y = core.heightmap.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE;
            packed.extend_from_slice(&[
                x - origin.x,
                y - 0.25,
                z - origin.y,
                yaw,
                scale,
                species as f32,
            ]);
        }
    }
    packed
}

// The generator decision has one implementation for rendering, picking and painting.
fn evaluate_cell(core: &SimCore, cell: VegetationCell, cell_m: f32, salt: u32) -> Option<Plant> {
    let generator = &core.vegetation;
    if !generator.config.enabled {
        return None;
    }
    let (cx, cz) = (cell.x, cell.z);
    let canopy = cell.layer == VegetationLayer::Canopy;
    let [x, z, yaw, scale] = candidate(cx, cz, cell_m, salt);
    if !inside_world(core, x, z) {
        return None;
    }
    let in_stand = generator.in_stand(x, z);
    let (species, density) = if canopy {
        let species = if hash(cx, cz, salt.wrapping_add(5)) % 2 == 0 {
            SPECIES_CONIFER
        } else {
            SPECIES_BROADLEAF
        };
        (species, generator.canopy_accept(in_stand))
    } else if hash(cx, cz, salt.wrapping_add(5)) % 8 == 0 {
        // Rocks are an accent, not ground cover: at bush density they read as
        // scattered debris across every open field. They are geology rather than
        // forest, so the density dial does not move them.
        (SPECIES_ROCK, generator.rock_accept())
    } else {
        // Undergrowth fills the forest floor, which is where the sparse canopy
        // currently reads as isolated trees standing on lawn.
        (SPECIES_BUSH, generator.understory_accept(in_stand))
    };
    if unit(cx, cz, salt.wrapping_add(6)) > density {
        return None;
    }

    if !placement_clear(core, x, z, cell.layer) {
        return None;
    }
    Some(Plant {
        x,
        z,
        yaw,
        scale,
        species: species as u8,
    })
}

fn inside_world(core: &SimCore, x: f32, z: f32) -> bool {
    let (world_w, world_h) = core.heightmap.world_size();
    x.is_finite() && z.is_finite() && x.abs() < world_w * 0.5 - 8.0 && z.abs() < world_h * 0.5 - 8.0
}

fn placement_clear(core: &SimCore, x: f32, z: f32, layer: VegetationLayer) -> bool {
    if !inside_world(core, x, z) {
        return false;
    }
    let (radius, max_relief) = if layer == VegetationLayer::Canopy {
        (6.0, 3.0)
    } else {
        (2.5, 1.6)
    };
    let y = core.heightmap.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE;
    // A 3x3 footprint rejects submerged roots and leaves clearance at shorelines
    // and built surfaces. Check native terrain samples, not a global sea level.
    if !clear_footprint(x, z, y, radius, max_relief, |sx, sz| {
        let p = Vector2::new(sx, sz);
        if core.watermap.visible_depth_world(p.x, p.y) > 0.001
            || core
                .transit_network
                .road_surface
                .sample_visible_surface_height(&core.region_graph, &core.heightmap, p.x, p.y)
                .is_some()
            || core.allocator.sample_building_site_height(p).is_some()
        {
            return None;
        }
        Some(core.heightmap.sample_visual_height_world(p.x, p.y) * crate::config::HEIGHT_SCALE)
    }) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests;
