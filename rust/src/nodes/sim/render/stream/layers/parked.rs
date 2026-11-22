// SPDX-License-Identifier: GPL-3.0-or-later

//! Parked car chunks drawn through the backend's pools (`RENDER-32-P`, `docs/terrain.md`
//! "Streaming core"): every vehicle bucket of a parked chunk writes its poses into a pooled
//! slot and draws what `parked_cars.gd`'s `MultiMeshInstance3D` per bucket drew: the bucket's
//! mesh with its own materials, the engine's bounds of the poses, shadows on with the dynamic
//! casters' 12 m extra visibility margin. No node is created or freed per chunk.
//!
//! The cars' draw distance (`RENDER-48`, the moving cars' `CAR_DRAW_DISTANCE_M` and fade): the
//! cars' materials fade each car out by its own distance from the main camera (`parked_cars.gd`
//! builds them with the moving cars' band), and each bucket's batch carries a visibility range
//! that ends where no car of it can be within the draw distance, so the engine skips a bucket
//! beyond the range in the main and the shadow passes alike, before anything of it is drawn. The
//! range is measured, as the engine measures it, from the centre of the batch's bounds: this
//! layer computes those bounds the engine's way and sends them as the `MultiMesh`'s own, so the
//! centre is known exactly. Nothing runs a frame for it: the engine checks the range of a batch
//! only as it culls, `O(buckets in view)`, and no slot is shown or hidden for it.
//!
//! Tiers (`RENDER-65`, `RENDER-69`): a vehicle look's four tier meshes are registered as four
//! buckets, each with its band of camera distance, and a chunk's cars of that look are written to
//! each of them. Each tier's material draws a car only while that car's own distance from the main
//! camera is in the tier's band, half-open (`vehicle_body.gdshader`, the fade's distance): a car
//! outside it collapses in the vertex stage and rasterizes nothing in any pass, so exactly one tier
//! draws each car, at the tier the moving cars' rule gives it. The batch's range only skips
//! batches none of whose cars can be in the band: it is the band widened by the batch's reach, the
//! farthest a car origin stands from the batch's centre, on both sides. The two near tiers split a
//! chunk's cars by a fixed cell of the chunk ([`NEAR_CELLS`] a side), so their detailed meshes are
//! shaded only for the cars of the few cells round the camera; the far tiers keep one batch a
//! chunk. Nothing runs a frame for it; the cost is the rows uploaded four times when a chunk
//! changes, the near tiers' through a counting sort by cell.
//!
//! A chunk installs whole or not at all: its batches that left are hidden and its new ones drawn
//! in the same call, and a chunk the pool cannot take (every slot of the limit live) sends
//! nothing and is refused, so its publication is not acknowledged and the snapshot offers it
//! again, as the vegetation keeps a refused product pending (`RENDER-42`'s rule). A chunk's
//! batches give up their slots before new ones are taken, so a batch moving to a larger size
//! class fits the room its old slot leaves. Costs: an install `O(B log B + poses)` for the
//! chunk's `B` batches, a reset `O(slots)`; nothing allocates once the maps, the install's
//! scratch and the pool have grown.
//!
//! The lots' bay paint (`PARK-30`) draws through a second layer of this kind, one bucket a chunk
//! (the paint quad, [`PAINT_BUCKET`]), its own draw distance ([`PAINT_DRAW_DISTANCE_M`]), no
//! shadow casting and rows of the pose alone: a line has no custom data.

use super::super::backend::{
    Drawn, RenderBackend, Role, Shadow, SlotId, SlotKind, SlotPool, capacity_class,
};
use crate::config::CAR_DRAW_DISTANCE_M;
use crate::nodes::sim::render::vegetation_product::godot_math::{Aabb, length3, max3, min3, sub3};
use crate::simulation::network::graph::RegionGraph;
use std::collections::HashMap;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// A parked chunk's key.
pub(crate) type ChunkKey = (i32, i32);

/// Slots the layer may hold, live and spare (one instance each, no instance uniform).
pub(crate) const SLOT_LIMIT: usize = 8_192;
/// Spare slots kept for reuse.
pub(crate) const SPARE_LIMIT: usize = 2_048;
/// The paint layer's slots (one a parking chunk with paint) and spares.
pub(crate) const PAINT_SLOT_LIMIT: usize = 4_096;
/// The paint layer's spare slots.
pub(crate) const PAINT_SPARE_LIMIT: usize = 512;
/// The paint layer's one bucket: the bay line quad.
pub(crate) const PAINT_BUCKET: u32 = 0;
/// How far the bay paint draws, metres: a 0.10 m line is about a quarter pixel there at a
/// 1080-line, 60 degree view, so its batch is dropped unseen.
pub(crate) const PAINT_DRAW_DISTANCE_M: f32 = 400.0;
/// Cells a side the near tiers split a parking chunk into (`RENDER-69`): 4, 128 m cells. Measured
/// on the 30k city (`docs/vehicles.md`): 64 m cells shade 0.05 ms less on the GPU but take 3.3
/// times the slots (5,860 of the pool's 8,192) and more render CPU.
pub(crate) const NEAR_CELLS: u16 = 4;
/// The tiers split by [`NEAR_CELLS`]: the two detailed ones.
pub(crate) const NEAR_TIERS: u32 = 2;
// `SceneLightingConfig.DYNAMIC_CASTER_EXTRA_CULL_MARGIN_M`, the cars' shadow policy.
const EXTRA_MARGIN_M: f32 = 12.0;
// A 3D transform's floats: a row longer than this carries instance custom data.
const POSE: usize = 12;
// A car's row: its pose, then the car's custom data (`RENDER-65`).
const STRIDE: usize = crate::nodes::sim::render::car_visual::ROW;
// A paint line's row: its pose alone (`PARK-30`).
const PAINT_STRIDE: usize = crate::simulation::parking::paint::STRIDE;
const _: () = assert!(PAINT_STRIDE == POSE && STRIDE > POSE);
// A batch of a chunk: its bucket and its cell, row-major across the chunk (`WHOLE` for a
// bucket drawn as one batch a chunk).
type Batch = (u32, u16);
const WHOLE: u16 = u16::MAX;

/// The cells a side a parked car tier's bucket splits a chunk into (`RENDER-69`): the near
/// tiers [`NEAR_CELLS`], the far ones 1 (one batch a chunk).
pub(crate) fn tier_cells(tier: u32) -> u16 {
    if tier < NEAR_TIERS { NEAR_CELLS } else { 1 }
}

// A registered bucket.
#[derive(Clone, Copy)]
struct Bucket {
    // The catalogue mesh and its bounds.
    mesh: u32,
    bounds: Aabb,
    // The tier's band of each car's camera distance, `[begin, end)`.
    band: [f32; 2],
    // Cells a side it splits a chunk into; 1 draws the chunk as one batch.
    cells: u16,
}

// One batch of the chunk being installed: its key, its cars, and its rows: the named bucket's
// own (`None`) or a float range of the split rows.
#[derive(Clone, Copy)]
struct Part {
    batch: Batch,
    count: u32,
    split: Option<(usize, usize)>,
}

/// The parked chunks' slots.
pub(crate) struct ParkedLayer {
    pool: SlotPool,
    buckets: Map<u32, Bucket>,
    drawn: Map<(ChunkKey, Batch), SlotId>,
    // Each drawn chunk's batches, so a chunk's update finds the batches that left.
    batches: Map<ChunkKey, Vec<Batch>>,
    // Whether the cars cast the sun's shadow (the shadow benchmark's caster classes,
    // `RENDER-15`); the game never clears it. The paint layer never casts.
    cast_shadows: bool,
    // How far an instance draws: the cars' `CAR_DRAW_DISTANCE_M`, the paint's own.
    draw_distance: f32,
    // Floats a row: a car's pose and custom data (`STRIDE`), a paint line's pose
    // (`PAINT_STRIDE`). The slots carry custom data when the rows do.
    stride: usize,
    // The install's scratch, kept for its capacity: its batches sorted by key, the split
    // buckets' rows grouped by cell, and the counting sort's cell starts.
    parts: Vec<Part>,
    split_rows: Vec<f32>,
    cell_starts: Vec<usize>,
}

impl ParkedLayer {
    /// A layer of cars' rows (pose and custom data) of at most `limit` slots, `spare_limit` of
    /// them spare, drawing each instance to `draw_distance` metres.
    pub(crate) fn new(limit: usize, spare_limit: usize, draw_distance: f32) -> Self {
        Self {
            pool: SlotPool::new(limit, spare_limit),
            buckets: Map::default(),
            drawn: Map::default(),
            batches: Map::default(),
            cast_shadows: true,
            draw_distance,
            stride: STRIDE,
            parts: Vec::new(),
            split_rows: Vec::new(),
            cell_starts: Vec::new(),
        }
    }

    /// The parked cars' layer.
    pub(crate) fn cars() -> Self {
        Self::new(SLOT_LIMIT, SPARE_LIMIT, CAR_DRAW_DISTANCE_M)
    }

    /// The bay paint's layer: rows of the pose alone, no shadow casting.
    pub(crate) fn paint() -> Self {
        let mut layer = Self::new(PAINT_SLOT_LIMIT, PAINT_SPARE_LIMIT, PAINT_DRAW_DISTANCE_M);
        layer.cast_shadows = false;
        layer.stride = PAINT_STRIDE;
        layer
    }

    /// The pool (tests and status).
    pub(crate) fn pool(&self) -> &SlotPool {
        &self.pool
    }

    /// Whether reused slots get fresh storage (motion vectors on).
    pub(crate) fn set_fresh_buffers(&mut self, fresh: bool) {
        self.pool.set_fresh(fresh);
    }

    /// Whether the cars cast the sun's shadow, every drawn bucket at once and later ones as
    /// drawn (the shadow benchmark's caster classes, `RENDER-15`). `O(drawn buckets)`.
    pub(crate) fn set_cast_shadows(&mut self, backend: &mut impl RenderBackend, cast: bool) {
        self.cast_shadows = cast;
        let shadow = if cast { Shadow::On } else { Shadow::Off };
        for &slot in self.drawn.values() {
            self.pool.set_shadow(backend, slot, Role::Main, shadow, true);
        }
    }

    /// Registers a vehicle bucket's mesh (a catalogue index), the mesh's bounds (`Mesh.get_aabb`,
    /// what the engine bounds a `MultiMesh` of it with), the band `[begin, end)` of each car's
    /// camera distance its tier draws in (`[0, inf]` for every distance; the material draws
    /// only the cars in it) and the cells a side it splits a chunk into ([`tier_cells`]; 1 for
    /// one batch a chunk).
    pub(crate) fn register(&mut self, bucket: u32, mesh: u32, bounds: Aabb, band: [f32; 2], cells: u16) {
        let cells = cells.max(1);
        self.buckets.insert(bucket, Bucket { mesh, bounds, band, cells });
    }

    /// Frees every slot and forgets every mesh: the buckets now name other meshes (a rebuilt
    /// vehicle catalogue, `RENDER-65`). `O(slots)`.
    pub(crate) fn forget_meshes(&mut self, backend: &mut impl RenderBackend) {
        self.reset(backend);
        self.buckets.clear();
    }

    /// Whether a bucket has its mesh.
    pub(crate) fn has_mesh(&self, bucket: u32) -> bool {
        self.buckets.contains_key(&bucket)
    }

    /// Frees every slot (a world replaced, the renderer's end); the meshes stay registered.
    pub(crate) fn reset(&mut self, backend: &mut impl RenderBackend) {
        self.pool.clear(backend);
        self.drawn.clear();
        self.batches.clear();
    }

    /// Installs chunk `chunk` as `buckets` (bucket and its rows, the layer's stride each; every
    /// bucket with poses registered), whole: returns false and sends nothing when the pool cannot
    /// give the slots it needs. A bucket split by cell draws a batch for each cell with cars. The
    /// chunk's batches not named, or named without poses, are hidden. `O(B log B + poses)` for
    /// the chunk's `B` batches.
    pub(crate) fn install<'a>(
        &mut self,
        backend: &mut impl RenderBackend,
        chunk: ChunkKey,
        buckets: impl Iterator<Item = (u32, &'a [f32])> + Clone,
    ) -> bool {
        self.split(buckets.clone());
        if !self.fits(chunk) {
            return false;
        }
        // The chunk's slots that leave or no longer fit go first.
        let mut kept = self.batches.remove(&chunk).unwrap_or_default();
        kept.retain(|&batch| {
            let slot = self.drawn[&(chunk, batch)];
            if self.part(batch).is_some_and(|part| self.keeps(slot, batch.0, part.count)) {
                return true;
            }
            self.drawn.remove(&(chunk, batch));
            self.pool.release(backend, slot);
            false
        });
        // Taken out for the loop, which draws through `self`; put back for their capacity.
        let (parts, split_rows) = (std::mem::take(&mut self.parts), std::mem::take(&mut self.split_rows));
        for (bucket, rows) in buckets {
            let from = parts.partition_point(|part| part.batch.0 < bucket);
            for part in parts[from..].iter().take_while(|part| part.batch.0 == bucket) {
                let rows = part.split.map_or(rows, |(start, end)| &split_rows[start..end]);
                self.draw_batch(backend, chunk, part, rows, &mut kept);
            }
        }
        (self.parts, self.split_rows) = (parts, split_rows);
        if !kept.is_empty() {
            self.batches.insert(chunk, kept);
        }
        true
    }

    // Lists the install's batches in `parts`, sorted by key: a bucket drawn whole as itself, a
    // split one as its cells with cars, their rows copied to `split_rows` by cell.
    // `O(poses + cells + B log B)`.
    fn split<'a>(&mut self, buckets: impl Iterator<Item = (u32, &'a [f32])>) {
        self.parts.clear();
        self.split_rows.clear();
        for (bucket, rows) in buckets {
            if rows.is_empty() {
                continue;
            }
            let cells = self.buckets[&bucket].cells;
            if cells == 1 {
                let count = (rows.len() / self.stride) as u32;
                self.parts.push(Part { batch: (bucket, WHOLE), count, split: None });
            } else {
                self.split_bucket(bucket, rows, cells);
            }
        }
        self.parts.sort_unstable_by_key(|part| part.batch);
    }

    // A counting sort of `rows` by the cell of each car's origin, stable, so a cell's cars keep
    // the chunk's order. The cell is taken on the world's grid of `CHUNK_SIZE / cells`, which
    // the chunks share: a car whose origin stands just past its chunk's edge falls in a cell on
    // the far side, which costs only that batch's reach (its range covers every car it holds).
    fn split_bucket(&mut self, bucket: u32, rows: &[f32], cells: u16) {
        let stride = self.stride;
        let side = usize::from(cells);
        let size = RegionGraph::CHUNK_SIZE / f32::from(cells);
        let cell = |row: &[f32]| {
            let at = |metres: f32| ((metres / size).floor() as i64).rem_euclid(side as i64) as usize;
            at(row[11]) * side + at(row[3])
        };
        let starts = &mut self.cell_starts;
        starts.clear();
        starts.resize(side * side + 1, 0);
        for row in rows.chunks_exact(stride) {
            starts[cell(row) + 1] += 1;
        }
        for at in 0..side * side {
            starts[at + 1] += starts[at];
        }
        let base = self.split_rows.len();
        for (at, ends) in starts.windows(2).enumerate() {
            if ends[1] > ends[0] {
                self.parts.push(Part {
                    batch: (bucket, at as u16),
                    count: (ends[1] - ends[0]) as u32,
                    split: Some((base + ends[0] * stride, base + ends[1] * stride)),
                });
            }
        }
        self.split_rows.resize(base + rows.len(), 0.0);
        for row in rows.chunks_exact(stride) {
            let at = cell(row);
            let to = base + starts[at] * stride;
            self.split_rows[to..to + stride].copy_from_slice(row);
            starts[at] += 1;
        }
    }

    // The install's batch `batch`, if it has cars. `O(log B)`.
    fn part(&self, batch: Batch) -> Option<&Part> {
        self.parts
            .binary_search_by_key(&batch, |part| part.batch)
            .ok()
            .map(|at| &self.parts[at])
    }

    // Whether the pool can give the slots the chunk's install needs once its own releases are
    // counted. `O(B log B)`.
    fn fits(&self, chunk: ChunkKey) -> bool {
        let mut room = self.pool.available();
        let mut needed = 0usize;
        for part in &self.parts {
            match self.drawn.get(&(chunk, part.batch)) {
                Some(&slot) if self.keeps(slot, part.batch.0, part.count) => {}
                Some(_) => {
                    needed += 1;
                    room += 1;
                }
                None => needed += 1,
            }
        }
        for &batch in self.batches.get(&chunk).into_iter().flatten() {
            room += usize::from(self.part(batch).is_none());
        }
        needed <= room
    }

    // Whether a drawn batch keeps its slot: its mesh and a size class that holds its cars.
    fn keeps(&self, slot: SlotId, bucket: u32, count: u32) -> bool {
        let kind = self.pool.slot(slot).kind();
        kind.mesh == self.buckets[&bucket].mesh && kind.class >= capacity_class(count).0
    }

    // Writes one batch of the install into its slot (taken now when it has none) and draws it
    // with its range: the tier's band widened by the batch's reach on both sides, the end no
    // farther than the draw distance plus the reach. The material draws each car only within
    // its band, so a car the engine draws in another tier's batch never draws here.
    fn draw_batch(
        &mut self,
        backend: &mut impl RenderBackend,
        chunk: ChunkKey,
        part: &Part,
        rows: &[f32],
        kept: &mut Vec<Batch>,
    ) {
        let bucket = self.buckets[&part.batch.0];
        let slot = match self.drawn.get(&(chunk, part.batch)) {
            Some(&slot) => slot,
            None => {
                let kind = SlotKind {
                    mesh: bucket.mesh,
                    class: capacity_class(part.count).0,
                    colors: false,
                    custom: self.stride > POSE,
                    parameter: None,
                };
                let slot = self
                    .pool
                    .acquire(backend, kind)
                    .expect("the chunk was checked to fit the pool");
                self.drawn.insert((chunk, part.batch), slot);
                kept.push(part.batch);
                slot
            }
        };
        let (bounds, reach) = drawn_bounds(&bucket.bounds, rows, self.stride);
        self.pool.fill(backend, slot, rows, part.count, bounds);
        self.draw(backend, slot, batch_range(bucket.band, reach, self.draw_distance));
    }

    fn draw(&mut self, backend: &mut impl RenderBackend, slot: SlotId, range: [f32; 2]) {
        let fresh = self.pool.slot(slot).drawn(Role::Main).is_none();
        let main = Drawn {
            origin: [0.0; 3],
            aabb: Aabb::default(),
            range,
            material: None,
            shadow: if self.cast_shadows {
                Shadow::On
            } else {
                Shadow::Off
            },
            visible: true,
        };
        self.pool.draw(backend, slot, Role::Main, main);
        if fresh && let Some(instance) = self.pool.slot(slot).instance(Role::Main) {
            backend.instance_set_extra_margin(instance, EXTRA_MARGIN_M);
        }
    }

    /// The drawn batches: chunk, bucket, cell (row-major across the chunk; `u16::MAX` for a
    /// bucket drawn whole), slot. `O(drawn batches)`.
    pub(crate) fn drawn(&self) -> impl Iterator<Item = (ChunkKey, u32, u16, SlotId)> + '_ {
        self.drawn
            .iter()
            .map(|(&(chunk, (bucket, cell)), &slot)| (chunk, bucket, cell, slot))
    }
}

// A batch's visibility range, as the engine tests it against the camera's distance from the
// centre of the batch's bounds: every car stands within `reach` of that centre, so a car whose
// own distance is in `band` has the centre within the band widened by `reach` on both sides,
// and past the draw distance plus `reach` every car has faded out whole. The engine's range has
// no margins, so no state; a begin of 0 is none.
fn batch_range(band: [f32; 2], reach: f32, draw_distance: f32) -> [f32; 2] {
    [(band[0] - reach).max(0.0), band[1].min(draw_distance) + reach]
}

// A batch's bounds as the engine computes a `MultiMesh`'s (the mesh's bounds through each pose,
// merged in pose order) and its reach: the farthest a car origin stands from the bounds' centre
// (the farthest corner of the origins' box). `O(poses)`.
fn drawn_bounds(mesh: &Aabb, rows: &[f32], stride: usize) -> (Aabb, f32) {
    let bounds = mesh.through_instances(rows, rows.len() / stride);
    let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for pose in rows.chunks_exact(stride) {
        let origin = [pose[3], pose[7], pose[11]];
        low = min3(low, origin);
        high = max3(high, origin);
    }
    let centre = bounds.center();
    let reach = Aabb::new(low, sub3(high, low))
        .corners()
        .into_iter()
        .fold(0.0f32, |reach, corner| reach.max(length3(sub3(centre, corner))));
    (bounds, reach)
}

#[cfg(test)]
mod tests;
