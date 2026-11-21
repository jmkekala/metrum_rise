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
//! Tiers (`RENDER-65`): a vehicle look's four tier meshes are registered as four buckets, each
//! with its band of camera distance, and a chunk's cars of that look are written to each of them.
//! The engine draws the tier whose band holds the camera's distance from the batch's centre, so
//! the tier switches per chunk and bucket, by the same range test as the draw distance, with no
//! work a frame; the cost is the rows uploaded four times when a chunk changes.
//!
//! A chunk installs whole or not at all: its buckets that left are hidden and its new ones drawn
//! in the same call, and a chunk the pool cannot take (every slot of the limit live) sends
//! nothing and is refused, so its publication is not acknowledged and the snapshot offers it
//! again, as the vegetation keeps a refused product pending (`RENDER-42`'s rule). A chunk's
//! buckets give up their slots before new ones are taken, so a bucket moving to a larger size
//! class fits the room its old slot leaves. Costs: an install `O(buckets of the chunk + their
//! poses)`, a reset `O(slots)`; nothing allocates once the maps and the pool have grown.

use super::super::backend::{
    Drawn, RenderBackend, Role, Shadow, SlotId, SlotKind, SlotPool, capacity_class,
};
use crate::config::CAR_DRAW_DISTANCE_M;
use crate::nodes::sim::render::vegetation_product::godot_math::{Aabb, length3, max3, min3, sub3};
use std::collections::HashMap;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// A parked chunk's key.
pub(crate) type ChunkKey = (i32, i32);

/// Slots the layer may hold, live and spare (one instance each, no instance uniform).
pub(crate) const SLOT_LIMIT: usize = 8_192;
/// Spare slots kept for reuse.
pub(crate) const SPARE_LIMIT: usize = 2_048;
// `SceneLightingConfig.DYNAMIC_CASTER_EXTRA_CULL_MARGIN_M`, the cars' shadow policy.
const EXTRA_MARGIN_M: f32 = 12.0;
// A row: a 3D transform's 12 floats, then the car's custom data (`RENDER-65`).
const STRIDE: usize = crate::nodes::sim::render::car_visual::ROW;

/// The parked chunks' slots.
pub(crate) struct ParkedLayer {
    pool: SlotPool,
    // Each bucket's catalogue mesh, the mesh's bounds and its tier's band of camera distance.
    meshes: Map<u32, (u32, Aabb, [f32; 2])>,
    drawn: Map<(ChunkKey, u32), SlotId>,
    // Each drawn chunk's buckets, so a chunk's update finds the buckets that left.
    buckets: Map<ChunkKey, Vec<u32>>,
    // Whether the cars cast the sun's shadow (the shadow benchmark's caster classes,
    // `RENDER-15`); the game never clears it.
    cast_shadows: bool,
}

impl ParkedLayer {
    /// A layer of at most `limit` slots, `spare_limit` of them spare.
    pub(crate) fn new(limit: usize, spare_limit: usize) -> Self {
        Self {
            pool: SlotPool::new(limit, spare_limit),
            meshes: Map::default(),
            drawn: Map::default(),
            buckets: Map::default(),
            cast_shadows: true,
        }
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
    /// what the engine bounds a `MultiMesh` of it with) and the band of camera distance from the
    /// batch's centre its tier draws in (`[0, inf]` for every distance).
    pub(crate) fn register(&mut self, bucket: u32, mesh: u32, bounds: Aabb, band: [f32; 2]) {
        self.meshes.insert(bucket, (mesh, bounds, band));
    }

    /// Frees every slot and forgets every mesh: the buckets now name other meshes (a rebuilt
    /// vehicle catalogue, `RENDER-65`). `O(slots)`.
    pub(crate) fn forget_meshes(&mut self, backend: &mut impl RenderBackend) {
        self.reset(backend);
        self.meshes.clear();
    }

    /// Whether a bucket has its mesh.
    pub(crate) fn has_mesh(&self, bucket: u32) -> bool {
        self.meshes.contains_key(&bucket)
    }

    /// Frees every slot (a world replaced, the renderer's end); the meshes stay registered.
    pub(crate) fn reset(&mut self, backend: &mut impl RenderBackend) {
        self.pool.clear(backend);
        self.drawn.clear();
        self.buckets.clear();
    }

    /// Installs chunk `chunk` as `buckets` (bucket and its poses, 12 floats each; every bucket
    /// with poses registered), whole: returns false and sends nothing when the pool cannot give
    /// the slots it needs. The chunk's buckets not named, or named without poses, are hidden.
    pub(crate) fn install<'a>(
        &mut self,
        backend: &mut impl RenderBackend,
        chunk: ChunkKey,
        buckets: impl Iterator<Item = (u32, &'a [f32])> + Clone,
    ) -> bool {
        if !self.fits(chunk, buckets.clone()) {
            return false;
        }
        // The chunk's slots that leave or no longer fit go first.
        let mut kept = self.buckets.remove(&chunk).unwrap_or_default();
        kept.retain(|&bucket| {
            let rows = buckets
                .clone()
                .find(|&(named, _)| named == bucket)
                .map_or(&[][..], |(_, rows)| rows);
            let slot = self.drawn[&(chunk, bucket)];
            if !rows.is_empty() && self.keeps(slot, bucket, rows.len()) {
                return true;
            }
            self.drawn.remove(&(chunk, bucket));
            self.pool.release(backend, slot);
            false
        });
        for (bucket, rows) in buckets {
            if rows.is_empty() {
                continue;
            }
            let count = (rows.len() / STRIDE) as u32;
            let (mesh, mesh_bounds, band) = self.meshes[&bucket];
            let slot = match self.drawn.get(&(chunk, bucket)) {
                Some(&slot) => slot,
                None => {
                    let kind = SlotKind {
                        mesh,
                        class: capacity_class(count).0,
                        colors: false,
                        custom: true,
                        parameter: None,
                    };
                    let slot = self
                        .pool
                        .acquire(backend, kind)
                        .expect("the chunk was checked to fit the pool");
                    self.drawn.insert((chunk, bucket), slot);
                    kept.push(bucket);
                    slot
                }
            };
            let (bounds, range_end) = drawn_bounds(&mesh_bounds, rows);
            self.pool.fill(backend, slot, rows, count, bounds);
            self.draw(backend, slot, [band[0], band[1].min(range_end)]);
        }
        if !kept.is_empty() {
            self.buckets.insert(chunk, kept);
        }
        true
    }

    // Whether the pool can give the slots the chunk's install needs once its own releases are
    // counted. `O(buckets of the chunk²)`, a few dozen at most.
    fn fits<'a>(
        &self,
        chunk: ChunkKey,
        buckets: impl Iterator<Item = (u32, &'a [f32])> + Clone,
    ) -> bool {
        let mut room = self.pool.available();
        let mut needed = 0usize;
        for (bucket, rows) in buckets.clone() {
            if rows.is_empty() {
                continue;
            }
            match self.drawn.get(&(chunk, bucket)) {
                Some(&slot) if self.keeps(slot, bucket, rows.len()) => {}
                Some(_) => {
                    needed += 1;
                    room += 1;
                }
                None => needed += 1,
            }
        }
        for &bucket in self.buckets.get(&chunk).into_iter().flatten() {
            let named = buckets
                .clone()
                .any(|(named, rows)| named == bucket && !rows.is_empty());
            room += usize::from(!named);
        }
        needed <= room
    }

    // Whether a drawn bucket keeps its slot: its mesh and a size class that holds the poses.
    fn keeps(&self, slot: SlotId, bucket: u32, floats: usize) -> bool {
        let kind = self.pool.slot(slot).kind();
        kind.mesh == self.meshes[&bucket].0
            && kind.class >= capacity_class((floats / STRIDE) as u32).0
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

    /// The drawn buckets: chunk, bucket, slot. `O(drawn buckets)`.
    pub(crate) fn drawn(&self) -> impl Iterator<Item = (ChunkKey, u32, SlotId)> + '_ {
        self.drawn.iter().map(|(&(chunk, bucket), &slot)| (chunk, bucket, slot))
    }
}

// A bucket's bounds as the engine computes a `MultiMesh`'s (the mesh's bounds through each pose,
// merged in pose order) and its visibility range's end: the draw distance plus the farthest a car
// origin stands from the bounds' centre (the farthest corner of the origins' box). The engine
// skips the batch when the camera is farther than that from the centre (the range has no
// margins, so no state), and then every origin is beyond the draw distance, where the material
// has faded its car out whole. `O(poses)`.
fn drawn_bounds(mesh: &Aabb, rows: &[f32]) -> (Aabb, f32) {
    let bounds = mesh.through_instances(rows, rows.len() / STRIDE);
    let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for pose in rows.as_chunks::<STRIDE>().0 {
        let origin = [pose[3], pose[7], pose[11]];
        low = min3(low, origin);
        high = max3(high, origin);
    }
    let centre = bounds.center();
    let reach = Aabb::new(low, sub3(high, low))
        .corners()
        .into_iter()
        .fold(0.0f32, |reach, corner| reach.max(length3(sub3(centre, corner))));
    (bounds, CAR_DRAW_DISTANCE_M + reach)
}

#[cfg(test)]
mod tests;
