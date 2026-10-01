// SPDX-License-Identifier: GPL-2.0-only

//! Bounded brush proposals and class-local occupancy over the existing edit cells.

use super::*;
use brush::PlantClass;

const ATTEMPTS: usize = 2;

// Candidate identity is (class, signed cell x/z, attempt). All properties have distinct salts.
fn dart(x: i32, z: i32, attempt: usize, class: PlantClass, seed: u32) -> (Plant, u32) {
    let salt = layer_base(128 + class as u32 * 64 + attempt as u32 * 16, seed);
    let step = class.spacing() * std::f32::consts::FRAC_1_SQRT_2;
    (
        Plant {
            x: (x as f32 + unit(x, z, salt.wrapping_add(1))) * step,
            z: (z as f32 + unit(x, z, salt.wrapping_add(2))) * step,
            yaw: unit(x, z, salt.wrapping_add(3)) * std::f32::consts::TAU,
            scale: 0.75 + unit(x, z, salt.wrapping_add(4)) * 0.6,
            species: 0,
            variant: VARIANT_FROM_SEED,
        },
        salt,
    )
}

// The outer fifth fades with smoothstep. The threshold is fixed per dart, so stamps do not
// reroll density. A later stamp can expose a previous edge to its full interior influence.
fn influence(plant: &Plant, pos: Vector2, radius: f32) -> f32 {
    if radius == 0.0 {
        return if in_disc(plant, pos, radius) {
            1.0
        } else {
            0.0
        };
    }
    let distance = ((plant.x - pos.x).powi(2) + (plant.z - pos.y).powi(2)).sqrt();
    let t = ((radius - distance) / (radius * 0.2)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Uses species clearance while keeping the authored canopy owner cell.
pub(super) fn authored_clear(core: &SimCore, plant: &Plant) -> bool {
    let class = PlantClass::of(plant.species, plant.variant);
    if class == PlantClass::Landscape {
        // A yard shrub or hedge stands beside a kerb or a wall by design, so only its own
        // stem position has to be clear, not a canopy tree's room.
        return clear_site(core, plant.x, plant.z, LANDSCAPE_CLEAR_RADIUS_M, 1.0);
    }
    placement_clear(core, plant.x, plant.z, class.clearance_layer())
}

// Both layers can own authored plants. Generated neighbors are evaluated locally and honor
// tombstones; hidden authored entries reserve their space because they can become visible again.
fn occupied(core: &SimCore, plant: &Plant) -> bool {
    let class = PlantClass::of(plant.species, plant.variant);
    let radius = class.spacing();
    let pos = Vector2::new(plant.x, plant.z);
    let conflicts = |other: &Plant| {
        PlantClass::of(other.species, other.variant) == class
            && (other.x - plant.x).powi(2) + (other.z - plant.z).powi(2) < radius * radius
    };
    for layer in [VegetationLayer::Canopy, VegetationLayer::Understory] {
        let (step, salt) = grid(core, layer);
        let first = cell_at(pos - Vector2::splat(radius), layer, step);
        let last = cell_at(pos + Vector2::splat(radius), layer, step);
        for z in first.z..=last.z {
            for x in first.x..=last.x {
                let cell = VegetationCell { layer, x, z };
                let (removed, added) = core.vegetation_edits.cell(cell);
                if added.iter().any(conflicts) {
                    return true;
                }
                if !removed
                    && layer == class.clearance_layer()
                    && generated_candidate(core, cell, step, salt)
                        .filter(conflicts)
                        .is_some_and(|p| placement_clear(core, p.x, p.z, layer))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Places one explicit plant subject to the same occupancy as a brush.
pub(super) fn add_at(core: &mut SimCore, pos: Vector2, option: i64) -> bool {
    let Some(preset) = preset(option) else {
        return false;
    };
    if !valid_disc(core, pos, 0.0) {
        return false;
    }
    prepare_sites(core);
    let (cell_m, salt) = grid(core, VegetationLayer::Canopy);
    let cell = cell_at(pos, VegetationLayer::Canopy, cell_m);
    let [_, _, yaw, scale] = candidate(cell.x, cell.z, cell_m, salt);
    let (species, variant) = preset.plant(cell.x, cell.z, salt);
    let plant = Plant {
        x: pos.x,
        z: pos.y,
        yaw,
        scale: preset.size(scale),
        species,
        variant,
    };
    if !authored_clear(core, &plant) || occupied(core, &plant) {
        return false;
    }
    let key = PatchLayout::new(core).key(pos.x, pos.y);
    let mut undo = VegetationEditUndo::for_stroke(0);
    undo.record_cell(cell, core.vegetation_edits.snapshot_cell(cell));
    undo.record_patch(key);
    core.vegetation_edits.add(cell, plant);
    core.vegetation_edits.bump_patch(key);
    core.push_vegetation_undo(undo);
    true
}

/// Length of one hedge module along its row, in metres; tools/model_landscape.py builds each
/// module from -0.5 m to +0.5 m along its own X axis.
const HEDGE_MODULE_M: f32 = 1.0;
/// Longest hedge one gesture lays, which bounds a call to 256 modules.
pub(super) const MAX_LINE_M: f32 = 256.0;
// Radius around a landscape plant's stem that must be open ground.
const LANDSCAPE_CLEAR_RADIUS_M: f32 = 0.3;

/// Lays hedge modules end to end from `from` to `to`, each facing along the row, and returns
/// how many it planted. O(L) in the row length: one clearance test and one bounded lookup per
/// module. A module that would stand on a road, building or water, or on a module of the same
/// hedge already there, is skipped, so redrawing a row does not stack it.
pub(super) fn line_at(
    core: &mut SimCore,
    from: Vector2,
    to: Vector2,
    option: i64,
    stroke: i64,
) -> usize {
    let Some(preset) = preset(option) else {
        return 0;
    };
    let span = to - from;
    let length = span.length();
    if !preset.is_hedge()
        || !(length <= MAX_LINE_M)
        || !valid_disc(core, from, 0.0)
        || !valid_disc(core, to, 0.0)
    {
        return 0;
    }
    prepare_sites(core);
    // Never more than one module length apart, so the row closes; the overlap is hidden inside.
    let modules = (length / HEDGE_MODULE_M).ceil().max(1.0) as usize;
    // The renderer turns +X by this yaw about +Y, which carries it to (cos, -sin) on the ground.
    let yaw = (-span.y).atan2(span.x);
    let (cell_m, salt) = grid(core, VegetationLayer::Canopy);
    let layout = PatchLayout::new(core);
    let mut undo = VegetationEditUndo::for_stroke(stroke);
    let mut count = 0;
    for i in 0..modules {
        let at = from + span * ((i as f32 + 0.5) / modules as f32);
        let cell = cell_at(at, VegetationLayer::Canopy, cell_m);
        let (species, variant) = preset.plant(cell.x, cell.z, salt);
        let plant = Plant {
            x: at.x,
            z: at.y,
            yaw,
            scale: 1.0,
            species,
            variant,
        };
        if !authored_clear(core, &plant) || module_taken(core, &plant, cell_m) {
            continue;
        }
        undo.record_cell_with(cell, || core.vegetation_edits.snapshot_cell(cell));
        core.vegetation_edits.add(cell, plant);
        let key = layout.key(at.x, at.y);
        core.vegetation_edits.bump_patch(key);
        undo.record_patch(key);
        count += 1;
    }
    core.push_vegetation_undo(undo);
    count
}

// Whether a module of the same hedge already stands within half a module of this one. Authored
// plants live in canopy cells, so this visits the few cells around it.
fn module_taken(core: &SimCore, plant: &Plant, cell_m: f32) -> bool {
    let reach = HEDGE_MODULE_M * 0.5;
    let pos = Vector2::new(plant.x, plant.z);
    let first = cell_at(pos - Vector2::splat(reach), VegetationLayer::Canopy, cell_m);
    let last = cell_at(pos + Vector2::splat(reach), VegetationLayer::Canopy, cell_m);
    (first.z..=last.z).any(|z| {
        (first.x..=last.x).any(|x| {
            let cell = VegetationCell {
                layer: VegetationLayer::Canopy,
                x,
                z,
            };
            core.vegetation_edits.cell(cell).1.iter().any(|other| {
                other.species == plant.species
                    && other.variant == plant.variant
                    && (other.x - plant.x).powi(2) + (other.z - plant.z).powi(2) < reach * reach
            })
        })
    })
}

struct Proposal {
    plant: Plant,
    owner: VegetationCell,
    restore: Option<VegetationCell>,
    // Restoration/replacement sites precede new darts in hash order, and new darts follow in
    // rising rank. Integer tie-breakers follow.
    order: (u8, u32, u8, i32, i32, usize),
}

/// Most plants one interactive stamp of `option` at `radius` adds.
pub(super) fn stamp_limit(option: i64, radius: f32) -> usize {
    preset(option).map_or(0, |p| p.class().stamp_limit(radius))
}

/// Evaluates bounded proposals in parallel, then accepts in canonical priority order until
/// `limit` plants are placed.
pub(super) fn paint_at(
    core: &mut SimCore,
    pos: Vector2,
    radius: f32,
    option: i64,
    stroke: i64,
    limit: usize,
) -> usize {
    let Some(preset) = preset(option) else {
        return 0;
    };
    let class = preset.class();
    if preset.is_hedge() || !valid_disc(core, pos, radius) || radius > class.max_radius() {
        return 0;
    }
    prepare_sites(core);
    let seed = core.vegetation.config.seed;
    let owner_step = core.vegetation.canopy_cell_m;
    let step = class.spacing() * std::f32::consts::FRAC_1_SQRT_2;
    let cells = disc_cells(pos, radius, step, VegetationLayer::Canopy);
    let first = cell_at(pos - Vector2::splat(radius), VegetationLayer::Canopy, step);
    let last = cell_at(pos + Vector2::splat(radius), VegetationLayer::Canopy, step);
    let nx = last.x - first.x + 1;
    let mut plans: Vec<_> = (0..cells.len() * ATTEMPTS)
        .into_par_iter()
        .filter_map(|i| {
            let x = first.x + (i / ATTEMPTS) as i32 % nx;
            let z = first.z + (i / ATTEMPTS) as i32 / nx;
            let attempt = i % ATTEMPTS;
            let (mut plant, salt) = dart(x, z, attempt, class, seed);
            let clump = 0.25
                + 1.5
                    * crate::simulation::vegetation::value_noise(
                        plant.x / 40.0,
                        plant.z / 40.0,
                        layer_base(0x51a7_32b9, seed),
                    );
            let rank = preset.rank(x, z, salt, clump * influence(&plant, pos, radius));
            if !(rank < 1.0) {
                return None;
            }
            (plant.species, plant.variant) = preset.plant(x, z, salt);
            plant.scale = preset.size(plant.scale);
            if !authored_clear(core, &plant) {
                return None;
            }
            Some(Proposal {
                owner: cell_at(
                    Vector2::new(plant.x, plant.z),
                    VegetationLayer::Canopy,
                    owner_step,
                ),
                plant,
                restore: None,
                // A non-negative float orders like its bit pattern.
                order: (
                    1,
                    rank.to_bits(),
                    class as u8,
                    z,
                    x,
                    attempt,
                ),
            })
        })
        .collect();
    // Only tombstones propose generator positions. Empty generator slots no longer add a
    // second lattice to the brush, and restore/replacement retains the original exact site.
    for layer in [VegetationLayer::Canopy, VegetationLayer::Understory] {
        if layer == VegetationLayer::Understory && class == PlantClass::Tree {
            continue;
        }
        let (cell_m, salt) = grid(core, layer);
        let restores: Vec<_> = disc_cells(pos, radius, cell_m, layer)
            .filter_map(|cell| {
                if !core.vegetation_edits.cell(cell).0 {
                    return None;
                }
                let [x, z, yaw, scale] = candidate(cell.x, cell.z, cell_m, salt);
                let (species, variant) = preset.plant(cell.x, cell.z, salt);
                let authored = Plant {
                    x,
                    z,
                    yaw,
                    scale: preset.size(scale),
                    species,
                    variant,
                };
                if !in_disc(&authored, pos, radius) {
                    return None;
                }
                let generated = evaluate_cell(core, cell, cell_m, salt);
                if layer == VegetationLayer::Understory
                    && generated.is_none_or(|p| PlantClass::of(p.species, p.variant) != class)
                {
                    return None;
                }
                let restore =
                    generated.filter(|p| p.species == species && variant == VARIANT_FROM_SEED);
                let plant = restore.unwrap_or(authored);
                if !authored_clear(core, &plant) {
                    return None;
                }
                Some(Proposal {
                    plant,
                    owner: cell_at(Vector2::new(x, z), VegetationLayer::Canopy, owner_step),
                    restore: restore.map(|_| cell),
                    order: (
                        0,
                        hash(cell.x, cell.z, salt.wrapping_add(10)),
                        layer as u8,
                        cell.z,
                        cell.x,
                        0,
                    ),
                })
            })
            .collect();
        plans.extend(restores);
    }
    if plans.is_empty() {
        return 0;
    }
    let layout = PatchLayout::new(core);
    core.vegetation_edits
        .reserve(plans.len().min(limit), layout.reserve_count(radius));
    plans.sort_unstable_by_key(|p| p.order);
    let mut undo = VegetationEditUndo::for_stroke(stroke);
    let mut count = 0;
    // Acceptance is deliberately serial: each accepted plant constrains later proposals.
    for plan in &plans {
        if count == limit {
            break;
        }
        if occupied(core, &plan.plant) {
            continue;
        }
        let cell = plan.restore.unwrap_or(plan.owner);
        undo.record_cell_with(cell, || core.vegetation_edits.snapshot_cell(cell));
        if plan.restore.is_some() {
            core.vegetation_edits.set_removed(cell, false);
        } else {
            core.vegetation_edits.add(cell, plan.plant);
        }
        let key = layout.key(plan.plant.x, plan.plant.z);
        core.vegetation_edits.bump_patch(key);
        undo.record_patch(key);
        count += 1;
    }
    core.push_vegetation_undo(undo);
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darts_fill_their_cells_with_stable_independent_properties() {
        for class in [PlantClass::Tree, PlantClass::Ground, PlantClass::Rock] {
            let step = class.spacing() * std::f32::consts::FRAC_1_SQRT_2;
            let mut range = (1.0_f32, 0.0_f32);
            for x in -100..100 {
                for attempt in 0..ATTEMPTS {
                    let (p, _) = dart(x, -7, attempt, class, 123);
                    assert_eq!(dart(x, -7, attempt, class, 123).0, p);
                    assert_ne!(dart(x, -7, attempt, class, 124).0, p);
                    assert!(p.x >= x as f32 * step && p.x < (x + 1) as f32 * step);
                    assert!(p.z >= -7.0 * step && p.z < -6.0 * step);
                    let fraction = p.x / step - x as f32;
                    range = (range.0.min(fraction), range.1.max(fraction));
                    assert!((0.75..1.35).contains(&p.scale));
                    assert!((0.0..std::f32::consts::TAU).contains(&p.yaw));
                }
            }
            assert!(range.0 < 0.05 && range.1 > 0.95);
        }
    }
}
