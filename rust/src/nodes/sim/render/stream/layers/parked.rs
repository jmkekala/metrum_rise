// SPDX-License-Identifier: GPL-2.0-only

//! Parked car chunks drawn through the backend's pools (`RENDER-32-P`, `docs/terrain.md`
//! "Streaming core"): every vehicle bucket of a parked chunk writes its poses into a pooled
//! slot and draws what `parked_cars.gd`'s `MultiMeshInstance3D` per bucket drew: the bucket's
//! mesh with its own materials, the engine's bounds of the poses, shadows on with the dynamic
//! casters' 12 m extra visibility margin. No node is created or freed per chunk.
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
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
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
// A pose: a 3D transform's 12 floats.
const STRIDE: usize = 12;

/// The parked chunks' slots.
pub(crate) struct ParkedLayer {
    pool: SlotPool,
    meshes: Map<u32, u32>,
    drawn: Map<(ChunkKey, u32), SlotId>,
    // Each drawn chunk's buckets, so a chunk's update finds the buckets that left.
    buckets: Map<ChunkKey, Vec<u32>>,
}

impl ParkedLayer {
    /// A layer of at most `limit` slots, `spare_limit` of them spare.
    pub(crate) fn new(limit: usize, spare_limit: usize) -> Self {
        Self {
            pool: SlotPool::new(limit, spare_limit),
            meshes: Map::default(),
            drawn: Map::default(),
            buckets: Map::default(),
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

    /// Registers a vehicle bucket's mesh (a catalogue index).
    pub(crate) fn register(&mut self, bucket: u32, mesh: u32) {
        self.meshes.insert(bucket, mesh);
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
            let slot = match self.drawn.get(&(chunk, bucket)) {
                Some(&slot) => slot,
                None => {
                    let kind = SlotKind {
                        mesh: self.meshes[&bucket],
                        class: capacity_class(count).0,
                        colors: false,
                        custom: false,
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
            self.pool.fill(backend, slot, rows, count, Aabb::default());
            self.draw(backend, slot);
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
        kind.mesh == self.meshes[&bucket]
            && kind.class >= capacity_class((floats / STRIDE) as u32).0
    }

    fn draw(&mut self, backend: &mut impl RenderBackend, slot: SlotId) {
        let fresh = self.pool.slot(slot).drawn(Role::Main).is_none();
        let main = Drawn {
            origin: [0.0; 3],
            aabb: Aabb::default(),
            range: [0.0; 2],
            material: None,
            shadow: Shadow::On,
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

#[cfg(test)]
mod tests;
