// SPDX-License-Identifier: GPL-2.0-only

//! Bounded brush proposals and class-local occupancy over the existing edit cells.

use super::*;
use crate::simulation::buildings::allocator::yard::YardEvent;
use brush::{BrushPreset, HEDGE_LOW_PRESET, PlantClass};

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
    if class == PlantClass::Tree && core.allocator.in_yard_planting(Vector2::new(plant.x, plant.z)) {
        // An asset's planting area is lawn its author meant for trees, so a yard tree there
        // needs only its trunk's room off walls, paving and roads, not a wild tree's.
        return clear_site(core, plant.x, plant.z, YARD_TREE_CLEAR_RADIUS_M, 3.0, true);
    }
    if class == PlantClass::Landscape {
        // A yard shrub or hedge stands beside a kerb or a wall by design, so only its own
        // stem position has to be clear, not a canopy tree's room.
        return clear_site(core, plant.x, plant.z, LANDSCAPE_CLEAR_RADIUS_M, 1.0, true);
    }
    placement_clear_for(core, plant.x, plant.z, class.clearance_layer(), true)
}

// Both layers can own authored plants. Generated neighbors are evaluated locally and honor
// tombstones; hidden authored entries reserve their space because they can become visible again.
pub(super) fn occupied(core: &SimCore, plant: &Plant) -> bool {
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
pub(super) const HEDGE_MODULE_M: f32 = 1.0;
/// Longest hedge one gesture lays, which bounds a call to 256 modules.
pub(super) const MAX_LINE_M: f32 = 256.0;
// Radius around a landscape plant's stem that must be open ground.
const LANDSCAPE_CLEAR_RADIUS_M: f32 = 0.3;
// Room a tree in a yard planting area keeps off walls, paving and roads.
const YARD_TREE_CLEAR_RADIUS_M: f32 = 1.5;

/// How far a drawn hedge end reaches for a hedge already standing, in metres.
const HEDGE_SNAP_M: f32 = 1.25;
// Body widths of the low, medium and tall modules, as tools/model_landscape.py builds them.
const HEDGE_WIDTH_M: [f32; 3] = [0.6, 0.8, 0.9];
// A face on another module's centreline is a joint, not an end. Inner faces of a row all are,
// because its modules are never more than one module length apart.
const HEDGE_FACE_TOLERANCE_M: f32 = 0.05;
// Rows closer to parallel than this, as |cos| of the angle between them, continue each other
// rather than meet, and need no fill at the joint.
const HEDGE_COLLINEAR_COS: f32 = 0.97;

// How far across its row a yard hedge module counts a parallel hedge as its own: a neighbour's
// hedge on or just beyond the shared lot line, so adjoining yards share one hedge between them.
const YARD_SHARE_M: f32 = 2.5;

/// Lays hedge modules end to end from `from` to `to`, each facing along the row, and returns
/// how many it planted. An end drawn within `HEDGE_SNAP_M` of a hedge already standing moves
/// onto it, and where the rows meet at an angle the new one runs on to close the corner (see
/// `plan_row`). One undo step. O(L) in the row length.
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
    if !preset.is_hedge() || !valid_disc(core, from, 0.0) || !valid_disc(core, to, 0.0) {
        return 0;
    }
    prepare_sites(core);
    let modules = plan_row(core, preset, from, to, [true, true], None);
    let layout = PatchLayout::new(core);
    let mut undo = VegetationEditUndo::for_stroke(stroke);
    for &(cell, plant) in &modules {
        undo.record_cell_with(cell, || core.vegetation_edits.snapshot_cell(cell));
        core.vegetation_edits.add(cell, plant);
        let key = layout.key(plant.x, plant.z);
        core.vegetation_edits.bump_patch(key);
        undo.record_patch(key);
    }
    core.push_vegetation_undo(undo);
    modules.len()
}

/// Lays the yard hedges and plants the yard planting areas of the buildings placed since the
/// last call, and clears the yards of the buildings removed, in that order. A yard records what
/// it laid and planted under its parcel and build generation; nothing a player placed is ever
/// recorded. Removal takes the hedge away only while every module still stands as laid, so a
/// yard whose hedge the player cut or rebuilt keeps what is left and nothing joined to it goes;
/// each planted plant still standing goes with its building. O(row length) per row, O(area /
/// spacing²) per planting area and O(n) per removed yard of n plants; nothing when no building
/// changed.
pub(crate) fn publish_yards(core: &mut SimCore) {
    let events = std::mem::take(&mut core.allocator.pending_yards);
    if events.is_empty() {
        return;
    }
    prepare_sites(core);
    let layout = PatchLayout::new(core);
    for event in events {
        match event {
            YardEvent::Placed { key, hedge, rows, planting } => {
                if let Some(preset) = hedge.and_then(|hedge| preset(HEDGE_LOW_PRESET + hedge.index() as i64)) {
                    let mut laid = Vec::new();
                    // Row by row, so each row sees the ones before it and the corners join.
                    for row in rows {
                        let join = [row.join_from, row.join_to];
                        for (cell, plant) in plan_row(core, preset, row.from, row.to, join, Some(YARD_SHARE_M)) {
                            core.vegetation_edits.add(cell, plant);
                            core.vegetation_edits.bump_patch(layout.key(plant.x, plant.z));
                            laid.push((cell, plant));
                        }
                    }
                    core.vegetation_edits.record_yard_hedge(key, laid);
                }
                // After the hedge, so plants keep their room off it.
                let mut planted = Vec::new();
                for (index, area) in planting.iter().enumerate() {
                    planted.extend(super::yard_planting::plant_area(core, key, index, area, &layout));
                }
                core.vegetation_edits.record_yard_planting(key, planted);
            }
            YardEvent::Removed(key) => {
                let remove = |core: &mut SimCore, laid: Vec<(VegetationCell, Plant)>| {
                    for (cell, plant) in laid {
                        core.vegetation_edits.remove_added(cell, |other| {
                            (*other == plant).then(|| layout.key(plant.x, plant.z))
                        });
                    }
                };
                if let Some(laid) = core.vegetation_edits.take_yard_hedge(key)
                    && laid
                        .iter()
                        .all(|(cell, plant)| core.vegetation_edits.cell(*cell).1.contains(plant))
                {
                    remove(core, laid);
                }
                if let Some(planted) = core.vegetation_edits.take_yard_planting(key) {
                    // A plant the player already removed is simply not found.
                    remove(core, planted);
                }
            }
        }
    }
}

// The modules a hedge row from `from` to `to` adds, each checked against what already stands but
// none added yet. An end whose `join` flag is set and that lies within `HEDGE_SNAP_M` of a hedge
// already standing moves onto that hedge's free end, or else onto its side; where the rows meet
// at an angle the new one runs on by half the old one's width, which fills the corner a square
// end would leave open. A yard row (`share_across_m`) is planned on its lot line and keeps to it:
// its end joins only where that line crosses a hedge or at a hedge end on it, never at an end off
// it, which would turn the whole row onto a neighbour's corner. The end modules sit flush with
// the row's ends. A module that would stand on a road, building or water is skipped, as is one
// standing on the same hedge facing the same way (a redraw), or, with `share_across_m`, on any
// hedge facing the same way within that distance across the row (a yard line a neighbour
// already hedged). O(L) in the row length: one
// clearance test and one bounded lookup per module, plus two bounded joint searches.
fn plan_row(
    core: &SimCore,
    preset: &BrushPreset,
    from: Vector2,
    to: Vector2,
    join: [bool; 2],
    share_across_m: Option<f32>,
) -> Vec<(VegetationCell, Plant)> {
    let line = share_across_m.and((to - from).try_normalized());
    let join_at =
        |pos: Vector2, enabled: bool| enabled.then(|| hedge_join(core, pos, line)).flatten();
    let (from_join, to_join) = (join_at(from, join[0]), join_at(to, join[1]));
    let (mut from, mut to) = (
        from_join.as_ref().map_or(from, |join| join.at),
        to_join.as_ref().map_or(to, |join| join.at),
    );
    let along = (to - from).try_normalized();
    if let Some(along) = along {
        let fill = |join: &Option<HedgeJoin>| {
            join.as_ref().map_or(0.0, |join| {
                if along.dot(join.along).abs() < HEDGE_COLLINEAR_COS {
                    join.half_width
                } else {
                    0.0
                }
            })
        };
        from -= along * fill(&from_join);
        to += along * fill(&to_join);
    }
    let span = to - from;
    let length = span.length();
    if !(length <= MAX_LINE_M) {
        return Vec::new();
    }
    // Never more than one module length apart, so the row closes; the overlap is hidden inside.
    let modules = (length / HEDGE_MODULE_M).ceil().max(1.0) as usize;
    // The two end modules sit flush with the row's ends and the rest share the length evenly,
    // so a row stops exactly where it was drawn and a joint has a known face to meet.
    let pitch = if modules > 1 {
        (length - HEDGE_MODULE_M) / (modules - 1) as f32
    } else {
        0.0
    };
    let (first, step) = match along {
        Some(along) if modules > 1 => (from + along * (HEDGE_MODULE_M * 0.5), along * pitch),
        _ => (from + span * 0.5, Vector2::ZERO),
    };
    // The renderer turns +X by this yaw about +Y, which carries it to (cos, -sin) on the ground.
    let yaw = (-span.y).atan2(span.x);
    let (cell_m, salt) = grid(core, VegetationLayer::Canopy);
    (0..modules)
        .filter_map(|i| {
            let at = first + step * i as f32;
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
            let taken = match share_across_m {
                Some(across) => hedge_alongside(core, &plant, across),
                None => module_taken(core, &plant, cell_m),
            };
            (authored_clear(core, &plant) && !taken).then_some((cell, plant))
        })
        .collect()
}

/// Where a hedge end drawn at `pos` would join a hedge already standing, or `pos` itself.
/// The tool previews a row with this, so the preview ends where `line_at` will.
pub(super) fn hedge_end_at(core: &SimCore, pos: Vector2) -> Vector2 {
    if !pos.is_finite() {
        return pos;
    }
    hedge_join(core, pos, None).map_or(pos, |join| join.at)
}

// The point a new row's end moves to, with the direction and half width of the row it meets.
struct HedgeJoin {
    at: Vector2,
    along: Vector2,
    half_width: f32,
}

// One hedge module as a segment of its row's centreline.
struct HedgeModule {
    centre: Vector2,
    along: Vector2,
    half_width: f32,
}

impl HedgeModule {
    fn of(plant: &Plant) -> Option<Self> {
        let width = HEDGE_WIDTH_M[usize::from(brush::hedge_index(plant.species, plant.variant)?)];
        Some(Self {
            centre: Vector2::new(plant.x, plant.z),
            along: Vector2::new(plant.yaw.cos(), -plant.yaw.sin()),
            half_width: width * 0.5,
        })
    }

    fn faces(&self) -> [Vector2; 2] {
        let half = self.along * (HEDGE_MODULE_M * 0.5);
        [self.centre - half, self.centre + half]
    }

    fn nearest(&self, pos: Vector2) -> Vector2 {
        let reach = HEDGE_MODULE_M * 0.5;
        self.centre + self.along * (pos - self.centre).dot(self.along).clamp(-reach, reach)
    }

    // Where the line through `pos` along `dir` crosses this module's centreline, if it does; a
    // line running with the module never crosses it.
    fn crossing(&self, pos: Vector2, dir: Vector2) -> Option<Vector2> {
        if self.along.dot(dir).abs() >= HEDGE_COLLINEAR_COS {
            return None;
        }
        let s = (pos - self.centre).cross(dir) / self.along.cross(dir);
        (s.abs() <= HEDGE_MODULE_M * 0.5).then(|| self.centre + self.along * s)
    }
}

// Visits every hedge module whose centre lies within `reach` of `pos` on each axis, in canopy
// cell order. Authored plants live in canopy cells, so this is a few cells.
fn for_each_hedge(
    core: &SimCore,
    pos: Vector2,
    reach: f32,
    mut visit: impl FnMut(&Plant, HedgeModule),
) {
    let (cell_m, _) = grid(core, VegetationLayer::Canopy);
    let first = cell_at(pos - Vector2::splat(reach), VegetationLayer::Canopy, cell_m);
    let last = cell_at(pos + Vector2::splat(reach), VegetationLayer::Canopy, cell_m);
    for z in first.z..=last.z {
        for x in first.x..=last.x {
            let cell = VegetationCell {
                layer: VegetationLayer::Canopy,
                x,
                z,
            };
            for plant in core.vegetation_edits.cell(cell).1 {
                if let Some(module) = HedgeModule::of(plant) {
                    visit(plant, module);
                }
            }
        }
    }
}

// The nearest free hedge end within HEDGE_SNAP_M of `pos`, or else the nearest point on a hedge's
// centreline. With `line`, the direction of a row that must stay on its line through `pos`, only
// ends on that line count and the point on a centreline is where the line crosses it. An end is
// free when it stands outside every other module's body, which excludes the inner faces of a row
// and the ends already buried in a joint. O(k^2) in the k modules
// within a few metres of `pos`; the earliest of equally near candidates wins, so the result
// follows cell and storage order deterministically.
fn hedge_join(core: &SimCore, pos: Vector2, line: Option<Vector2>) -> Option<HedgeJoin> {
    let reach = HEDGE_SNAP_M + HEDGE_MODULE_M * 0.5;
    let mut end: Option<(f32, HedgeJoin)> = None;
    let mut side: Option<(f32, HedgeJoin)> = None;
    let keep = |best: &mut Option<(f32, HedgeJoin)>, at: Vector2, module: &HedgeModule| {
        let distance = at.distance_squared_to(pos);
        if distance <= HEDGE_SNAP_M * HEDGE_SNAP_M
            && best.as_ref().is_none_or(|(d, _)| distance < *d)
        {
            *best = Some((
                distance,
                HedgeJoin {
                    at,
                    along: module.along,
                    half_width: module.half_width,
                },
            ));
        }
    };
    for_each_hedge(core, pos, reach, |plant, module| {
        let crossing = match line {
            Some(dir) => module.crossing(pos, dir),
            None => Some(module.nearest(pos)),
        };
        if let Some(at) = crossing {
            keep(&mut side, at, &module);
        }
        for face in module.faces() {
            if face.distance_squared_to(pos) > HEDGE_SNAP_M * HEDGE_SNAP_M
                || line.is_some_and(|dir| (face - pos).cross(dir).abs() > HEDGE_FACE_TOLERANCE_M)
            {
                continue;
            }
            let mut buried = false;
            for_each_hedge(core, face, HEDGE_MODULE_M, |other_plant, other| {
                buried |= !std::ptr::eq(plant, other_plant)
                    && other.nearest(face).distance_to(face)
                        < other.half_width.max(HEDGE_FACE_TOLERANCE_M);
            });
            if !buried {
                keep(&mut end, face, &module);
            }
        }
    });
    end.or(side).map(|(_, join)| join)
}

// Whether a module of the same hedge, facing the same way, already stands within half a module
// of this one. A row that meets another at an angle crosses its modules and is not a redraw.
// Authored plants live in canopy cells, so this visits the few cells around it.
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
                    && (other.yaw - plant.yaw).cos().abs() >= HEDGE_COLLINEAR_COS
                    && (other.x - plant.x).powi(2) + (other.z - plant.z).powi(2) < reach * reach
            })
        })
    })
}

// Whether any hedge module facing the same way as `plant` stands within half a module of it
// along the row and within `across_m` across it. Visits the canopy cells around it.
fn hedge_alongside(core: &SimCore, plant: &Plant, across_m: f32) -> bool {
    let reach = (HEDGE_MODULE_M * 0.5).max(across_m);
    let pos = Vector2::new(plant.x, plant.z);
    let along = Vector2::new(plant.yaw.cos(), -plant.yaw.sin());
    let mut found = false;
    for_each_hedge(core, pos, reach, |_, other| {
        let offset = other.centre - pos;
        found |= other.along.dot(along).abs() >= HEDGE_COLLINEAR_COS
            && offset.dot(along).abs() < HEDGE_MODULE_M * 0.5
            && offset.dot(along.orthogonal()).abs() < across_m;
    });
    found
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
