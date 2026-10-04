// SPDX-License-Identifier: GPL-2.0-only

//! Planting a spawned building's yard areas: a jittered grid in the lot's frame, seeded by the
//! yard's own key, so two copies of one asset grow different yards and a reload grows none.

use super::*;
use crate::assets::asset::YardPlantKind;
use crate::simulation::buildings::allocator::yard::{YardKey, YardPlantingArea};

// One planting pass over an area: a candidate per `spacing_m` grid cell, kept with chance
// `accept` and planted from one of `presets`. The pick is shared by each `drift` by `drift`
// block of cells, so a flower bed grows in drifts of one kind rather than as confetti. A plant
// keeps `radius_m` to the plants this area planted before it, beyond the brush's spacing.
struct Pass {
    spacing_m: f32,
    accept: f32,
    presets: &'static [i64],
    drift: i32,
    radius_m: f32,
}

// The brush's scattered broadleaf-led trees, its six yard shrubs, its two flowering shrubs
// (hydrangea and rhododendron) and its four perennial flowers (peony, lupine, phlox and a
// clump of summer bedding plants).
const TREE_PRESETS: &[i64] = &[9];
const SHRUB_PRESETS: &[i64] = &[11, 12, 13, 14, 15, 16];
const FLOWERING_SHRUB_PRESETS: &[i64] = &[23, 24];
const PERENNIAL_PRESETS: &[i64] = &[20, 21, 22, 25];

const TREE_RADIUS_M: f32 = 1.5;
const SHRUB_RADIUS_M: f32 = 0.7;
const PERENNIAL_RADIUS_M: f32 = 0.35;

// About one tree per 50 m² and one shrub per 14 m²; mixed is a few trees among fewer shrubs
// and a few drifts of flowers. A flower bed is a perennial per 1.6 m² in two-metre drifts, with
// a flowering shrub about every 35 m².
const TREES: &[Pass] = &[Pass {
    spacing_m: 5.0,
    accept: 0.5,
    presets: TREE_PRESETS,
    drift: 1,
    radius_m: TREE_RADIUS_M,
}];
const BUSHES: &[Pass] = &[Pass {
    spacing_m: 2.5,
    accept: 0.45,
    presets: SHRUB_PRESETS,
    drift: 1,
    radius_m: SHRUB_RADIUS_M,
}];
const MIXED: &[Pass] = &[
    Pass {
        spacing_m: 8.0,
        accept: 0.5,
        presets: TREE_PRESETS,
        drift: 1,
        radius_m: TREE_RADIUS_M,
    },
    Pass {
        spacing_m: 3.0,
        accept: 0.3,
        presets: SHRUB_PRESETS,
        drift: 1,
        radius_m: SHRUB_RADIUS_M,
    },
    Pass {
        spacing_m: 1.0,
        accept: 0.25,
        presets: PERENNIAL_PRESETS,
        drift: 3,
        radius_m: PERENNIAL_RADIUS_M,
    },
];
const FLOWERS: &[Pass] = &[
    Pass {
        spacing_m: 4.0,
        accept: 0.45,
        presets: FLOWERING_SHRUB_PRESETS,
        drift: 1,
        radius_m: SHRUB_RADIUS_M,
    },
    Pass {
        spacing_m: 1.0,
        accept: 0.65,
        presets: PERENNIAL_PRESETS,
        drift: 2,
        radius_m: PERENNIAL_RADIUS_M,
    },
];

// How far a candidate may stray from its grid cell's centre, as a share of the spacing.
const JITTER: f32 = 0.35;

/// Plants area `index` of yard `key` and returns what it planted, in planting order. Every
/// candidate passes the same clearance and spacing tests as a brush plant, against what stands
/// and what this area planted before it. Deterministic in the key, the area and what already
/// stands. O(A / s²) candidates for area A and grid spacing s, each one bounded clearance test
/// and a scan of what the area planted so far: a yard's few dozen plants, never the world's.
pub(super) fn plant_area(
    core: &mut SimCore,
    key: YardKey,
    index: usize,
    area: &YardPlantingArea,
    layout: &PatchLayout,
) -> Vec<(VegetationCell, Plant)> {
    let passes = match area.plants {
        YardPlantKind::Trees => TREES,
        YardPlantKind::Bushes => BUSHES,
        YardPlantKind::Mixed => MIXED,
        YardPlantKind::Flowers => FLOWERS,
    };
    let Some((min, max)) = bounds(&area.polygon) else {
        return Vec::new();
    };
    // Parcel and build generation folded into one salt per area; a rebuilt parcel replants.
    let seed = hash(
        key.0 as i32,
        (key.0 >> 32) as i32,
        key.1.wrapping_mul(0x9e37_79b9),
    ) ^ hash(index as i32, 0, 0x5eed_7a4d);
    let (cell_m, _) = grid(core, VegetationLayer::Canopy);
    let mut planted = Vec::new();
    // Each planted position and the room it keeps, for the passes after it.
    let mut kept: Vec<(Vector2, f32)> = Vec::new();
    for (pass_index, pass) in passes.iter().enumerate() {
        let salt = seed.wrapping_add(pass_index as u32 * 0x1000);
        let first = [
            (min[0] / pass.spacing_m).floor() as i32,
            (min[1] / pass.spacing_m).floor() as i32,
        ];
        let last = [
            (max[0] / pass.spacing_m).ceil() as i32,
            (max[1] / pass.spacing_m).ceil() as i32,
        ];
        for j in first[1]..=last[1] {
            for i in first[0]..=last[0] {
                if unit(i, j, salt) >= pass.accept {
                    continue;
                }
                let jitter = |n: u32| (unit(i, j, salt.wrapping_add(n)) * 2.0 - 1.0) * JITTER;
                let local = [
                    (i as f32 + 0.5 + jitter(1)) * pass.spacing_m,
                    (j as f32 + 0.5 + jitter(2)) * pass.spacing_m,
                ];
                if !inside(local, &area.polygon) {
                    continue;
                }
                let pos = area.origin + area.basis_x * local[0] + area.basis_z * local[1];
                if kept
                    .iter()
                    .any(|&(at, room)| at.distance_to(pos) < room + pass.radius_m)
                {
                    continue;
                }
                let (di, dj) = (i.div_euclid(pass.drift), j.div_euclid(pass.drift));
                let pick =
                    (unit(di, dj, salt.wrapping_add(3)) * pass.presets.len() as f32) as usize;
                let Some(preset) = brush::preset(pass.presets[pick.min(pass.presets.len() - 1)])
                else {
                    continue;
                };
                let (species, variant) = preset.plant(i, j, salt.wrapping_add(4));
                // A generator scale, mapped into the preset's own band as the brush does.
                let (lo, hi) = brush::DEFAULT_SCALE;
                let scale = lo + unit(i, j, salt.wrapping_add(5)) * (hi - lo);
                let plant = Plant {
                    x: pos.x,
                    z: pos.y,
                    yaw: unit(i, j, salt.wrapping_add(6)) * std::f32::consts::TAU,
                    scale: preset.size(scale),
                    species,
                    variant,
                };
                if !placement::authored_clear(core, &plant) || placement::occupied(core, &plant) {
                    continue;
                }
                let cell = cell_at(pos, VegetationLayer::Canopy, cell_m);
                core.vegetation_edits.add(cell, plant);
                core.vegetation_edits
                    .bump_patch(layout.key(plant.x, plant.z));
                planted.push((cell, plant));
                kept.push((pos, pass.radius_m));
            }
        }
    }
    planted
}

fn bounds(polygon: &[[f32; 2]]) -> Option<([f32; 2], [f32; 2])> {
    if polygon.len() < 3 {
        return None;
    }
    Some(polygon.iter().fold(
        ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    ))
}

// Even-odd point in polygon.
fn inside(p: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    for (i, &a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        {
            inside = !inside;
        }
    }
    inside
}
