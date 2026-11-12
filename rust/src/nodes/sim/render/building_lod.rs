// SPDX-License-Identifier: GPL-2.0-only

//! Engine-independent, reusable building LOD batches. The bridge supplies spatial-index
//! candidates; this module never queries simulation state or calls Godot. Only topology
//! changes allocate. Warm camera updates use two packed buffers and counting scatter.
//!
//! Travel buckets (`RENDER-34`): a part is evaluated by the per-frame rule (its projected size,
//! [`select_lod`] with its previous tier) only on frames where that rule could choose another
//! tier. After each evaluation the part gets the camera motion it can take before its size can
//! leave the interval over which its tier is held ([`hold_interval`]): a bound on how far a rigid
//! move of the view changes the projected extent of its box (see [`hold_distance`]), split into
//! eye travel and rotation, the two path lengths the bridge accumulates ([`Motion`]). A frame
//! whose motion has not reached a part's thresholds would give it the tier it has, so skipping it
//! leaves every tier and every hysteresis history what the per-frame rule gives. A chunk is
//! evaluated only once its least threshold is passed, and re-packed only when a tier moved.

use crate::assets::lod_policy::{
    LodQuality, hold_interval, projected_size_pixels, select_lod,
};
use glam::{Mat3, Mat4, Vec2, Vec3};
use rayon::prelude::*;
use std::collections::BTreeMap;

pub(crate) mod spatial;
mod window_lighting;

/// Immutable policy bounds and the resource-generation fallback table for one part.
#[derive(Clone)]
pub(crate) struct Part {
    /// Placed LOD0 reference box; never follows the selected tier.
    pub bounds: [Vec3; 2],
    /// Union of every usable tier, used only for conservative visibility/shadow bounds.
    pub cull_bounds: [Vec3; 2],
    /// Authored ordinal to nearest usable finer tier (first usable if none is finer).
    pub fallback: Vec<usize>,
}

impl Part {
    /// Return no part when every import failed; the caller supplies a visible placeholder.
    pub fn new(bounds: [Vec3; 2], available: &[bool]) -> Option<Self> {
        let first = available.iter().position(|&loaded| loaded)?;
        let mut previous = first;
        let fallback = available
            .iter()
            .enumerate()
            .map(|(index, &loaded)| {
                if loaded {
                    previous = index;
                }
                previous
            })
            .collect();
        Some(Self {
            bounds,
            cull_bounds: bounds,
            fallback,
        })
    }
}

/// Camera projection in actual 3D render pixels and the shared quality preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// Godot/OpenGL clip-space projection, including the camera inverse.
    pub world_to_clip: Mat4,
    /// Resolution after render scaling, not UI logical dimensions.
    pub render_size: Vec2,
    /// Global player preference.
    pub quality: LodQuality,
}

/// One placed part; building ordinals are valid only within the allocator revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Instance {
    /// Allocator-local building ordinal.
    pub building: usize,
    /// Catalog-local part ordinal.
    pub part: usize,
    /// Whether the gray deserted material replaces source materials.
    pub deserted: bool,
    /// Authored colour-scheme ordinal; `0` when the asset authors no schemes.
    pub scheme: u16,
    /// Fixed window schedule: solar threshold, bedtime, wake hour, profile.
    pub lighting: [f32; 4],
    /// Complete local-mesh to world transform, including construction rise.
    pub transform: Mat4,
}

/// Stable group identity inside one spatial chunk and catalog generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct BatchKey {
    /// Catalog-local part ordinal.
    pub part: usize,
    /// Material override group.
    pub deserted: bool,
    /// Authored colour-scheme ordinal; surface overrides differ, geometry does not.
    pub scheme: u16,
    /// Usable authored tier ordinal.
    pub lod: usize,
}

/// The camera's motion for the travel buckets: path lengths since the bridge began, and the
/// view the thresholds are measured in (`RENDER-34`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Motion {
    /// Evaluate every part: a first frame, a projection, viewport or quality change.
    pub all: bool,
    /// Whether the fields below describe the view; without them no part is ever held.
    pub bounded: bool,
    /// Eye travel summed over frames, metres.
    pub travel: f64,
    /// Rotation summed over frames: each frame's spectral norm of the change of the view
    /// rotation, `2 sin(angle / 2)`, which bounds how far it moves a point per metre of distance.
    pub turn: f64,
    /// The eye.
    pub eye: Vec3,
    /// World to view rotation (rows: the camera's x, y and z axes; the view looks along -z).
    pub rotation: Mat3,
    /// Render pixels per unit of `x / depth` and of `y / depth`.
    pub scale: [f32; 2],
    /// The depth at and before which a point projects to infinity (the near plane).
    pub near: f32,
}

impl Motion {
    /// Every part evaluated, none held (no camera pose: tests, an orthographic view).
    pub const ALL: Self = Self {
        all: true,
        bounded: false,
        travel: 0.0,
        turn: 0.0,
        eye: Vec3::ZERO,
        rotation: Mat3::IDENTITY,
        scale: [0.0; 2],
        near: 0.0,
    };
}

// Parts evaluated together on one worker, and the due parts below which a chunk evaluates on its
// own worker.
const BLOCK: usize = 256;
const PARALLEL_MIN: usize = 1024;

struct Entry {
    instance: Instance,
    group: usize,
    previous: Option<usize>,
    selected: usize,
    // The LOD0 box's centre in the world and a bound on the distance between two of its corners.
    center: Vec3,
    reach: f32,
}

/// Slice into a chunk's packed transform/custom-data buffer.
pub(crate) struct Batch {
    /// Group identity for GPU buffer reuse.
    pub key: BatchKey,
    /// Needs publication before the next draw.
    pub changed: bool,
    /// First transform in the packed chunk buffer.
    pub offset: usize,
    /// Number of live instances, independent of GPU capacity.
    pub count: usize,
    next_count: usize,
    next_offset: usize,
}

/// Reusable counting/scatter storage and per-instance hysteresis for a resident chunk.
#[derive(Default)]
pub(crate) struct Chunk {
    entries: Vec<Entry>,
    /// Deterministically ordered GPU group headers, including retired empty groups.
    pub batches: Vec<Batch>,
    packed: Vec<[f32; 16]>,
    scratch: Vec<[f32; 16]>,
    /// Parts evaluated since the last acknowledgement.
    pub evaluations: usize,
    // The travel and the turn at which each part is next evaluated, beside the entries so the
    // checks of a moving frame read 16 bytes a part.
    due: Vec<[f64; 2]>,
    // The least threshold of the chunk's parts; whether its instances changed since the last
    // update (every part then evaluated and the chunk re-packed); whether every non-empty batch
    // is published again at the next re-pack (the chunk came back into the query).
    due_min: [f64; 2],
    resync: bool,
    republish: bool,
}

impl Chunk {
    /// Synchronize changed simulation inputs. Ordinals may be reused after swap-removal;
    /// the bridge must request a history reset when the building-reference revision changes.
    pub fn synchronize(&mut self, instances: &[Instance], parts: &[Part], reset_history: bool) {
        let same_layout = self.entries.len() == instances.len()
            && self.entries.iter().zip(instances).all(|(old, new)| {
                old.instance.building == new.building
                    && old.instance.part == new.part
                    && old.instance.deserted == new.deserted
                    && old.instance.scheme == new.scheme
            });
        self.resync = true;
        if same_layout {
            for (entry, instance) in self.entries.iter_mut().zip(instances) {
                entry.instance = *instance;
                (entry.center, entry.reach) = placed_box(&parts[instance.part], instance);
                if reset_history {
                    entry.previous = None;
                }
            }
            return;
        }

        // Stable group order makes output independent of hash/Rayon scheduling order.
        // Keep vanished groups until the bridge has observed their zero-count update.
        let mut groups = BTreeMap::new();
        for batch in &self.batches {
            groups.insert((batch.key.part, batch.key.deserted, batch.key.scheme), 0);
        }
        for instance in instances {
            groups.insert((instance.part, instance.deserted, instance.scheme), 0);
        }
        let mut batches = Vec::new();
        for (&(part, deserted, scheme), base) in &mut groups {
            *base = batches.len();
            for lod in 0..parts[part].fallback.len() {
                let key = BatchKey {
                    part,
                    deserted,
                    scheme,
                    lod,
                };
                let count = self
                    .batches
                    .binary_search_by_key(&key, |batch| batch.key)
                    .map_or(0, |index| self.batches[index].count);
                batches.push(Batch {
                    key,
                    // Existing nonempty groups need replacement even if their newly
                    // scattered bytes happen to match the reset scratch buffer.
                    changed: count != 0,
                    offset: 0,
                    count,
                    next_count: 0,
                    next_offset: 0,
                });
            }
        }
        self.batches = batches;
        self.entries.clear();
        self.entries.extend(instances.iter().map(|&instance| {
            let (center, reach) = placed_box(&parts[instance.part], &instance);
            Entry {
                instance,
                group: groups[&(instance.part, instance.deserted, instance.scheme)],
                previous: None,
                selected: 0,
                center,
                reach,
            }
        }));
        self.due.clear();
        self.due.resize(instances.len(), [f64::NEG_INFINITY; 2]);
        self.packed.resize(instances.len(), [0.0; 16]);
        self.scratch.resize(instances.len(), [0.0; 16]);
        // Old ranges no longer refer to the same layout after a topology change.
        for batch in &mut self.batches {
            batch.count = 0;
        }
    }

    /// Every part evaluated (no travel buckets): O(instances + groups). The reference the
    /// buckets are tested against.
    #[cfg(test)]
    pub fn update(&mut self, parts: &[Part], view: View, reset_history: bool) {
        self.update_due(parts, view, reset_history, &Motion::ALL);
    }

    /// Evaluates the parts whose thresholds `motion` reached (all of them after a
    /// synchronization, a history reset or with `motion.all`), then re-packs the batches when a
    /// tier moved. O(1) when the chunk's least threshold is not reached; otherwise O(instances)
    /// threshold checks, O(due parts) evaluations in blocks across the Rayon pool, and
    /// O(instances + groups) for a re-pack. No allocations after synchronization. Callers publish
    /// all changed groups in one render frame.
    pub fn update_due(
        &mut self,
        parts: &[Part],
        view: View,
        reset_history: bool,
        motion: &Motion,
    ) {
        let all = reset_history || motion.all || self.resync;
        if !all && motion.travel < self.due_min[0] && motion.turn < self.due_min[1] {
            self.evaluations = 0;
            return;
        }
        let is_due = |due: &[f64; 2]| all || motion.travel >= due[0] || motion.turn >= due[1];
        let pass = |entries: &mut [Entry], dues: &mut [[f64; 2]]| {
            let mut evaluations = 0;
            let mut moved = false;
            let mut due_min = [f64::INFINITY; 2];
            for (entry, due) in entries.iter_mut().zip(dues) {
                if is_due(due) {
                    let before = entry.selected;
                    *due = evaluate(entry, parts, view, reset_history, motion);
                    evaluations += 1;
                    moved |= entry.selected != before;
                }
                due_min = [due_min[0].min(due[0]), due_min[1].min(due[1])];
            }
            (evaluations, moved, due_min)
        };
        let count = if all {
            self.due.len()
        } else {
            self.due.iter().filter(|due| is_due(due)).count()
        };
        let (evaluations, moved, due_min) = if count < PARALLEL_MIN {
            pass(&mut self.entries, &mut self.due)
        } else {
            self.entries
                .par_chunks_mut(BLOCK)
                .zip(self.due.par_chunks_mut(BLOCK))
                .map(|(entries, dues)| pass(entries, dues))
                .reduce(
                    || (0, false, [f64::INFINITY; 2]),
                    |a, b| (a.0 + b.0, a.1 | b.1, [a.2[0].min(b.2[0]), a.2[1].min(b.2[1])]),
                )
        };
        self.evaluations = evaluations;
        self.due_min = due_min;
        // Unmoved tiers of unchanged instances pack to the same bytes, so only a move or a
        // synchronization re-packs.
        if moved || self.resync {
            self.pack();
        }
        self.resync = false;
        self.republish = false;
    }

    /// The chunk comes back into the query with the instances it left with: its parts are
    /// evaluated without history, as a new chunk's are, and its non-empty batches are published
    /// again at the next update. O(parts).
    pub fn reenter(&mut self) {
        for entry in &mut self.entries {
            entry.previous = None;
        }
        self.resync = true;
        self.republish = true;
    }

    /// Whether the next update evaluates every part (instances changed or the chunk came back).
    pub fn needs_update(&self) -> bool {
        self.resync
    }

    // Counting scatter of every entry into its selected group, flagging the groups whose
    // instances or bytes changed.
    fn pack(&mut self) {
        for batch in &mut self.batches {
            batch.next_count = 0;
        }
        for entry in &self.entries {
            self.batches[entry.selected].next_count += 1;
        }
        let mut offset = 0;
        for batch in &mut self.batches {
            batch.next_offset = offset;
            offset += batch.next_count;
            batch.next_count = 0;
        }
        for entry in &self.entries {
            let batch = &mut self.batches[entry.selected];
            let packed = &mut self.scratch[batch.next_offset + batch.next_count];
            packed[..12].copy_from_slice(&pack_transform(entry.instance.transform));
            packed[12..].copy_from_slice(&entry.instance.lighting);
            batch.next_count += 1;
        }
        for batch in &mut self.batches {
            batch.changed |= self.republish && batch.next_count != 0
                || batch.count != batch.next_count
                || self.packed[batch.offset..batch.offset + batch.count]
                    != self.scratch[batch.next_offset..batch.next_offset + batch.next_count];
            batch.offset = batch.next_offset;
            batch.count = batch.next_count;
        }
        std::mem::swap(&mut self.packed, &mut self.scratch);
    }

    /// Borrow 12 transform floats plus four window parameters per instance without allocating.
    pub fn transforms(&self, batch: &Batch) -> &[[f32; 16]] {
        &self.packed[batch.offset..batch.offset + batch.count]
    }

    /// Conservative active-instance bounds, independent of padded GPU capacity and LOD.
    pub fn bounds(&self, batch: &Batch, part: &Part) -> [Vec3; 2] {
        let center = (part.cull_bounds[0] + part.cull_bounds[1]) * 0.5;
        let extent = (part.cull_bounds[1] - part.cull_bounds[0]) * 0.5;
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for m in self.transforms(batch) {
            let row_x = Vec3::from_slice(&m[0..3]);
            let row_y = Vec3::from_slice(&m[4..7]);
            let row_z = Vec3::from_slice(&m[8..11]);
            let placed = Vec3::new(
                row_x.dot(center) + m[3],
                row_y.dot(center) + m[7],
                row_z.dot(center) + m[11],
            );
            let radius = Vec3::new(
                row_x.abs().dot(extent),
                row_y.abs().dot(extent),
                row_z.abs().dot(extent),
            );
            lo = lo.min(placed - radius);
            hi = hi.max(placed + radius);
        }
        [lo, hi]
    }

    /// Clear dirty headers only after the complete frame has crossed the bridge.
    pub fn acknowledge(&mut self) {
        for batch in &mut self.batches {
            batch.changed = false;
        }
        self.evaluations = 0;
    }
}

// The per-frame rule for one part; returns its thresholds.
fn evaluate(
    entry: &mut Entry,
    parts: &[Part],
    view: View,
    reset_history: bool,
    motion: &Motion,
) -> [f64; 2] {
    let part = &parts[entry.instance.part];
    let pixels = projected_size_pixels(
        part.bounds[0],
        part.bounds[1],
        view.world_to_clip * entry.instance.transform,
        view.render_size,
    );
    let tiers = part.fallback.len();
    let requested = select_lod(
        pixels,
        tiers,
        if reset_history { None } else { entry.previous },
        view.quality,
    )
    .unwrap_or(0);
    // Hysteresis follows the requested ordinal even when its import failed;
    // fallback geometry must not change the policy or trigger import retries.
    entry.previous = Some(requested);
    entry.selected = entry.group + part.fallback[requested];
    if !motion.bounded {
        [f64::NEG_INFINITY; 2]
    } else if tiers <= 1 {
        // One tier: every size selects it.
        [f64::INFINITY; 2]
    } else {
        let hold = hold_interval(requested, tiers, view.quality);
        let (distance, rho) = hold_distance(pixels, hold, entry.center, entry.reach, motion);
        let distance = f64::from(distance);
        let rho = f64::from(rho);
        [
            motion.travel + distance * 0.5,
            motion.turn + distance / (2.0 * rho + distance),
        ]
    }
}

// What the rounding of the boundaries, the size and the bound's own arithmetic are allowed:
// relative and absolute shrink of the hold interval, and the bound's safety factor.
const HOLD_RELATIVE: f32 = 1e-3;
const HOLD_ABSOLUTE: f32 = 1e-3;
const BOUND_SAFETY: f32 = 1.02;

/// The view motion `distance` (metres) a part can take before the per-frame rule could change
/// its tier, and the bound `rho` on its corners' distance from the eye; a frame's motion is split
/// as travel up to `distance / 2` and turn up to `distance / (2 rho + distance)`, so every corner
/// moves at most `distance` in view space (`RENDER-34`).
///
/// The bound: a view change moves view-space points rigidly, `v' = Q v + d`. For the screen
/// coordinate `f = x / depth` (and `y / depth`), the change of the difference between two corners
/// is `g(v_i) - g(v_j)` with `g(v) = f(Q v + d) - f(v)`, so by the mean value theorem along the
/// segment between them (inside the box) the projected extent moves by at most
/// `reach x (|Q - I| |grad f| + |v' - v| |hess f|)`, where over the region the box can reach
/// `|grad f| <= sqrt(1 + u^2) / w` and `|hess f| <= (u + sqrt(1 + u^2)) / w^2` (`w` the least
/// depth, `u` the greatest `|x| / w`). That times the render scale must stay inside the hold
/// interval's margin, and the least depth beyond the near plane (or, for a size projecting to
/// infinity, the whole box behind it). Bounds of the region are taken for a motion capped at a
/// fraction of the depth; the best of a few caps is kept.
pub(crate) fn hold_distance(
    pixels: f32,
    hold: [f32; 2],
    center: Vec3,
    reach: f32,
    motion: &Motion,
) -> (f32, f32) {
    let offset = center - motion.eye;
    let view = motion.rotation * offset;
    let depth = -view.z;
    let half = reach * 0.5;
    let rho = offset.length() + half;
    let near = motion.near * (1.0 + HOLD_RELATIVE) + HOLD_ABSOLUTE;
    if !pixels.is_finite() {
        // Infinite (or NaN) sizes select the first tier while any corner stays at or before the
        // near plane: held while the whole box is behind it.
        let behind = motion.near * (1.0 - HOLD_RELATIVE) - HOLD_ABSOLUTE - (depth + half);
        return (behind.max(0.0), rho);
    }
    let low = hold[0] * (1.0 + HOLD_RELATIVE) + HOLD_ABSOLUTE;
    let high = hold[1] * (1.0 - HOLD_RELATIVE) - HOLD_ABSOLUTE;
    let margin = (pixels - low).min(high - pixels);
    let nearest = depth - half;
    let room = nearest - near;
    if margin.is_nan() || margin <= 0.0 || room <= 0.0 {
        return (0.0, rho);
    }
    let mut best = 0.0f32;
    for fraction in [0.0625f32, 0.125, 0.25, 0.5] {
        let cap = (nearest * fraction).min(room * 0.999);
        let least = nearest - cap;
        let mut slope = 0.0f32;
        for (lateral, scale) in [(view.x.abs(), motion.scale[0]), (view.y.abs(), motion.scale[1])] {
            let u = (lateral + half + cap) / least;
            let root = (1.0 + u * u).sqrt();
            let gradient = root / least;
            let hessian = (u + root) / (least * least);
            slope = slope.max(scale * (gradient / (2.0 * rho) + hessian));
        }
        let slope = BOUND_SAFETY * reach * slope;
        best = best.max(if slope > 0.0 { cap.min(margin / slope) } else { cap });
    }
    (best, rho)
}

// The LOD0 box's centre placed in the world, and the sum of its placed edge lengths, which no
// two corners are farther apart than.
fn placed_box(part: &Part, instance: &Instance) -> (Vec3, f32) {
    let [low, high] = part.bounds;
    let center = instance.transform.transform_point3((low + high) * 0.5);
    let size = high - low;
    let reach = (0..3)
        .map(|axis| instance.transform.col(axis).truncate().length() * size[axis].abs())
        .sum();
    (center, reach)
}

/// Godot MultiMesh's row-major 3×4 affine buffer layout.
pub(super) fn pack_transform(matrix: Mat4) -> [f32; 12] {
    let [x, y, z, w] = matrix.to_cols_array_2d();
    [
        x[0], y[0], z[0], w[0], x[1], y[1], z[1], w[1], x[2], y[2], z[2], w[2],
    ]
}

#[cfg(test)]
mod tests;
