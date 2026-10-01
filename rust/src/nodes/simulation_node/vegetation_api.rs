// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic vegetation queries and bounded player edits over the saved generator.

use super::*;
use crate::nodes::sim::core::VegetationEditUndo;
use crate::simulation::vegetation::edits::{
    AuthoredPlant as Plant, VARIANT_FROM_SEED, VegetationCell, VegetationLayer, pack_patch_key,
};
use crate::simulation::vegetation::{hash, unit};
use brush::preset;

mod brush;
mod land_cover;
mod placement;
use placement::{add_at, line_at, paint_at, stamp_limit};

// Lane five of a packed placement carries both the species ordinal and the renderer mesh
// variant pinned over it, because widening the stride would cost the whole scatter buffer a
// seventh float for a field that is zero on every generated plant. The species occupies the
// low two bits and the biased variant everything above them, so an unpinned plant packs to
// the bare species ordinal exactly as it did before pinning existed. The largest value this
// can reach is 12 * 4 + 3, which an f32 carries exactly.
const SPECIES_BITS: u8 = 2;

fn pack_species_variant(species: u8, variant: u8) -> f32 {
    f32::from((variant << SPECIES_BITS) | species)
}

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

/// Room a canopy plant is cleared over, in metres. Also the margin a field polygon grows by
/// before its patches are restaled, because a tree this far outside it can still be affected.
pub(crate) const CANOPY_CLEAR_RADIUS_M: f32 = 6.0;

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

    /// Advances whenever any terrain payload generation or vegetation patch generation does, so
    /// a renderer can skip its per-patch currency checks while it is unchanged. O(1). Only
    /// comparable within one loaded world: a load restarts both counters.
    #[func]
    pub fn get_land_cover_epoch(&self) -> i64 {
        let core = self.lock_core();
        core.terrain_payload_generation_counter
            .wrapping_add(core.vegetation_edits.epoch()) as i64
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

    /// Returns scatter records `[x, height, z, yaw, scale, species]` for a patch, in world
    /// coordinates. The renderer draws one plant from patches of more than one size, and its
    /// cosmetic seed is keyed on the position it receives here; a position relative to the patch
    /// gave the same plant a different form in each.
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
    /// Pass the gesture's own `stroke` id on every stamp of a held drag, so the whole drag
    /// reverses as one undo step. Zero is a standalone edit and folds into nothing.
    #[func]
    pub fn remove_vegetation_at(&self, pos: Vector2, radius: f32, stroke: i64) -> i64 {
        remove_at(&mut self.lock_core(), pos, radius, stroke) as i64
    }

    /// Plants one plant of a brush preset, refusing obstructed footprints immediately.
    /// Checks a bounded neighborhood in both layers, including same-class generated plants.
    /// `option` indexes the preset table in `brush.rs`; an unknown ordinal plants nothing.
    #[func]
    pub fn add_vegetation_at(&self, pos: Vector2, option: i64) -> bool {
        add_at(&mut self.lock_core(), pos, option)
    }

    /// Scatters class-spaced plants immediately and returns the number added or restored.
    /// Tree/rock radii are bounded at 256 m; ground cover at 64 m. Two hashed darts per
    /// proposal cell plus local tombstone visits bound work independently of world size.
    /// Cost is O(P log P + P G + Q + local clearance), for P bounded proposals, G nearby
    /// owner cells and Q authored entries inspected. Candidate evaluation allocates nothing.
    /// One stamp adds at most one plant per 160 spacing squared, 10 stems/ha for trees, taken in
    /// a fixed rank over the 40 m clumping field and soft edge. Repeated stamps therefore thicken
    /// the stand step by step toward the preset's density and add nothing once it is reached.
    /// Calls are ordered input; each call resolves conflicts by rank and integer ties.
    /// Pass the gesture's own `stroke` id on every stamp so the drag reverses as one undo step.
    /// Zero is standalone. `option` selects the preset mix, acceptance and size band.
    #[func]
    pub fn paint_vegetation(&self, pos: Vector2, radius: f32, option: i64, stroke: i64) -> i64 {
        let limit = stamp_limit(option, radius);
        paint_at(&mut self.lock_core(), pos, radius, option, stroke, limit) as i64
    }

    /// Lays a clipped hedge from `from` to `to`, one module per metre facing along the row, and
    /// returns the number of modules planted. `option` must name a hedge preset in `brush.rs`;
    /// any other ordinal, or a row longer than 256 m, plants nothing. O(row length). Pass the
    /// gesture's `stroke` id so the row reverses as one undo step.
    #[func]
    pub fn plant_vegetation_line(
        &self,
        from: Vector2,
        to: Vector2,
        option: i64,
        stroke: i64,
    ) -> i64 {
        line_at(&mut self.lock_core(), from, to, option, stroke) as i64
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

/// Exact source and value of a selected plant; vector compaction cannot retarget a command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlantTarget {
    cell: VegetationCell,
    generated: bool,
    /// Stored placement used for hover geometry and validation.
    pub(crate) plant: Plant,
}

/// Removes exactly one validated source, journaling the same cell and patch as area edits.
pub(crate) fn remove_target(core: &mut SimCore, target: PlantTarget) -> bool {
    let (removed, added) = core.vegetation_edits.cell(target.cell);
    let (step, salt) = grid(core, target.cell.layer);
    let valid = if target.generated {
        !removed && evaluate_cell(core, target.cell, step, salt) == Some(target.plant)
    } else {
        added.contains(&target.plant)
    };
    if !valid {
        return false;
    }
    let key = PatchLayout::new(core).key(target.plant.x, target.plant.z);
    let mut undo = VegetationEditUndo::for_stroke(0);
    undo.record_cell(
        target.cell,
        core.vegetation_edits.snapshot_cell(target.cell),
    );
    undo.record_patch(key);
    if target.generated {
        core.vegetation_edits.set_removed(target.cell, true);
        core.vegetation_edits.bump_patch(key);
    } else {
        let mut found = false;
        core.vegetation_edits.remove_added(target.cell, |p| {
            if !found && *p == target.plant {
                found = true;
                Some(key)
            } else {
                None
            }
        });
    }
    core.push_vegetation_undo(undo);
    true
}

/// Finds the nearest visible plant and its hover radius within a fixed 4 m cursor disc.
/// Visits O(K + A) local candidates/additions with indexed footprint queries, where K is
/// bounded by the two grid spacings and A counts additions in those cells, not the world.
/// Allocates nothing per candidate; ties use layer, row-major cell, then authored order.
/// The caller must prepare the building-site query index before this immutable lookup.
pub(crate) fn plant_at(core: &SimCore, pos: Vector2) -> Option<(PlantTarget, f32)> {
    const PICK_RADIUS_M: f32 = 4.0;
    if !valid_disc(core, pos, PICK_RADIUS_M) {
        return None;
    }
    let mut best = None;
    for layer in [VegetationLayer::Canopy, VegetationLayer::Understory] {
        let (cell_m, salt) = grid(core, layer);
        let nearest = disc_cells(pos, PICK_RADIUS_M, cell_m, layer)
            .enumerate()
            .filter_map(|(cell_order, cell)| {
                let (removed, added) = core.vegetation_edits.cell(cell);
                let generated = if removed {
                    None
                } else {
                    evaluate_cell(core, cell, cell_m, salt)
                };
                generated
                    .iter()
                    .map(|p| (0, p))
                    .chain(added.iter().enumerate().map(|(i, p)| (i + 1, p)))
                    .filter(|(order, p)| {
                        in_disc(p, pos, PICK_RADIUS_M)
                            && (*order == 0 || placement::authored_clear(core, p))
                    })
                    .map(|(order, p)| {
                        let distance = (p.x - pos.x).powi(2) + (p.z - pos.y).powi(2);
                        (
                            distance,
                            cell_order,
                            order,
                            PlantTarget {
                                cell,
                                generated: order == 0,
                                plant: *p,
                            },
                        )
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        if let Some((distance, _, _, plant)) = nearest {
            // Strict replacement retains canopy first on a cross-layer distance tie.
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, plant));
            }
        }
    }
    let (_, target) = best?;
    let plant = target.plant;
    // Bushes and rocks have no entry in the canopy coverage table.
    let radius = land_cover::CROWN_RADII_M
        .get(plant.species as usize)
        .copied()
        .unwrap_or(1.0)
        * plant.scale;
    Some((target, radius))
}

/// Clears O(K + A) local candidates/additions through the brush's edit and undo journal.
pub(crate) fn remove_at(core: &mut SimCore, pos: Vector2, radius: f32, stroke: i64) -> usize {
    if !valid_disc(core, pos, radius) {
        return 0;
    }
    prepare_sites(core);
    let layout = PatchLayout::new(core);
    let mut removed = 0;
    let mut undo = VegetationEditUndo::for_stroke(stroke);
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
            // Recorded before the cell is touched, so the journal holds the state this
            // stroke found rather than the one it leaves behind.
            undo.record_cell(cell, core.vegetation_edits.snapshot_cell(cell));
            if let Some(plant) = generated {
                core.vegetation_edits.set_removed(cell, true);
                let key = layout.key(plant.x, plant.z);
                core.vegetation_edits.bump_patch(key);
                undo.record_patch(key);
                removed += 1;
            }
            removed += core.vegetation_edits.remove_added(cell, |plant| {
                in_disc(plant, pos, radius).then(|| {
                    let key = layout.key(plant.x, plant.z);
                    undo.record_patch(key);
                    key
                })
            });
        }
    }
    core.push_vegetation_undo(undo);
    removed
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
            .map(|plant| placement::authored_clear(core, plant))
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
                variant,
            } = plant;
            if !clear {
                continue;
            }
            if x < origin.x || z < origin.y || x >= origin.x + span || z >= origin.y + span {
                continue;
            }
            let height = |x: f32, z: f32| {
                core.heightmap.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE
            };
            let y = height(x, z);
            // A tree's root flare is sunk to cover a slope; a clipped hedge or a yard shrub
            // keeps its authored height and only settles into the lawn.
            let sink = if brush::PlantClass::of(species, variant) == brush::PlantClass::Landscape {
                0.05
            } else {
                0.25
            };
            // A hedge module is never scaled, so its scale lane carries the ground's rise along
            // the row across its one metre instead, and the renderer shears the module to it.
            // Without it every module sat level at its own centre and a row climbed in steps.
            let lane_four = if species == SPECIES_BUSH as u8
                && variant > brush::HEDGE_FIRST_VARIANT
            {
                let (dx, dz) = (yaw.cos() * 0.5, -yaw.sin() * 0.5);
                height(x + dx, z + dz) - height(x - dx, z - dz)
            } else {
                scale
            };
            packed.extend_from_slice(&[
                x,
                y - sink,
                z,
                yaw,
                lane_four,
                pack_species_variant(species, variant),
            ]);
        }
    }
    packed
}

// The generator decision has one implementation for rendering, picking and painting.
fn evaluate_cell(core: &SimCore, cell: VegetationCell, cell_m: f32, salt: u32) -> Option<Plant> {
    generated_candidate(core, cell, cell_m, salt)
        .filter(|plant| placement_clear(core, plant.x, plant.z, cell.layer))
}

// Keep intrinsic generation separate so occupancy rejects distant stems before querying
// their surface footprint. Rendering still evaluates the same decision and clearance.
fn generated_candidate(
    core: &SimCore,
    cell: VegetationCell,
    cell_m: f32,
    salt: u32,
) -> Option<Plant> {
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

    Some(Plant {
        x,
        z,
        yaw,
        scale,
        species: species as u8,
        // The generator names a species and leaves the modelled tree inside it to the
        // renderer, which is what every scattered plant did before the brush could pin one.
        variant: VARIANT_FROM_SEED,
    })
}

fn inside_world(core: &SimCore, x: f32, z: f32) -> bool {
    let (world_w, world_h) = core.heightmap.world_size();
    x.is_finite() && z.is_finite() && x.abs() < world_w * 0.5 - 8.0 && z.abs() < world_h * 0.5 - 8.0
}

fn placement_clear(core: &SimCore, x: f32, z: f32, layer: VegetationLayer) -> bool {
    let (radius, max_relief) = if layer == VegetationLayer::Canopy {
        (CANOPY_CLEAR_RADIUS_M, 3.0)
    } else {
        (2.5, 1.6)
    };
    clear_site(core, x, z, radius, max_relief)
}

// Whether a 3x3 footprint of `radius` around a position is open, native ground within
// `max_relief` of its centre height.
fn clear_site(core: &SimCore, x: f32, z: f32, radius: f32, max_relief: f32) -> bool {
    if !inside_world(core, x, z) {
        return false;
    }
    let y = core.heightmap.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE;
    // A 3x3 footprint rejects submerged roots and leaves clearance at shorelines
    // and built surfaces. Check native terrain samples, not a global sea level.
    if !clear_footprint(x, z, y, radius, max_relief, |sx, sz| {
        let p = Vector2::new(sx, sz);
        // A committed field is worked land, so it clears plants exactly as a road deck or a
        // building pad does: the test is the same predicate, and nothing is destroyed, so the
        // stand returns if the field is redrawn or the farm is removed.
        if core.watermap.visible_depth_world(p.x, p.y) > 0.001
            || core
                .transit_network
                .road_surface
                .sample_visible_surface_height(&core.region_graph, &core.heightmap, p.x, p.y)
                .is_some()
            || core.allocator.sample_building_site_height(p).is_some()
            || core.allocator.field_clearance.covers_point(p)
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
