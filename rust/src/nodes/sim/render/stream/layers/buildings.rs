// SPDX-License-Identifier: GPL-2.0-only

//! Building LOD batches drawn through the backend's pools (`RENDER-32-L`, `RENDER-34`,
//! `docs/terrain.md` "Streaming core"): every changed batch of a building LOD frame
//! (`building_lod::spatial`) writes its rows into a pooled slot and draws exactly what
//! `building_lods.gd`'s `MultiMeshInstance3D` per batch drew: the group's mesh and material
//! override, the batch's own bounds on its `MultiMesh`, the buildings' shadow policy (casting,
//! a 32 m extra visibility margin) and dynamic global illumination; and, in the water
//! reflection's world, the proxy `water_reflection_sources.gd` kept for it (no shadow, the
//! shader's `water_reflection_proxy` flag, visible to 1,000 m with a 32 m end margin). No node is
//! created or freed per batch, and the reflection's proxies change with their batches instead of
//! being polled every frame.
//!
//! A frame is applied whole or not at all, as the bridge published it: a building changes tier
//! in one frame, never drawn twice or not at all. A group's mesh and material are Godot resources
//! `building_lods.gd` builds as a group is first needed; while a changed batch's group has none,
//! [`BuildingLayer::missing`] names it and the frame waits for the registration.
//!
//! Residency follows the building LOD's chunk query (its rule decides the tiers, history and all);
//! a batch whose chunk leaves returns its slot to the pool hidden, and a chunk that comes back
//! takes a spare slot of its mesh and size class again, so a turn writes rows into objects that
//! exist rather than making and freeing objects. Costs: a frame `O(removed + changed batches +
//! their instances)`; nothing allocates once the maps and the pool have grown.

use super::super::backend::{
    Drawn, RenderBackend, Role, Shadow, SlotId, SlotKind, SlotPool, capacity_class,
};
use crate::nodes::sim::render::building_lod::BatchKey;
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use std::collections::HashMap;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// A building chunk key (the 512 m building chunk grid).
pub(crate) type ChunkKey = (i32, i32);

/// Slots the layer may hold, live and spare (two instances each).
pub(crate) const SLOT_LIMIT: usize = 16_384;
/// Spare slots kept for reuse.
pub(crate) const SPARE_LIMIT: usize = 4_096;
/// The catalogue parameter index of the reflection proxy flag (`water_reflection_proxy`).
pub(crate) const REFLECTION_FLAG: u32 = 0;

// `SceneLightingConfig.STATIC_CASTER_EXTRA_CULL_MARGIN_M`, the buildings' shadow policy.
const EXTRA_MARGIN_M: f32 = 32.0;
// `water_reflection_sources.gd`'s mirror-only range and its end margin.
const REFLECTION_RANGE_M: f32 = 1000.0;
const REFLECTION_MARGIN_M: f32 = 32.0;

/// What a group draws with: catalogue indices of its mesh and its material override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    /// Mesh index.
    pub(crate) mesh: u32,
    /// Material override index (`None`: the mesh's own materials).
    pub(crate) material: Option<u32>,
}

/// One changed batch of a frame.
pub(crate) struct Change<'a> {
    /// Its chunk.
    pub(crate) chunk: ChunkKey,
    /// Its group.
    pub(crate) key: BatchKey,
    /// Its instances: 12 transform floats and 4 window parameters each.
    pub(crate) rows: &'a [[f32; 16]],
    /// The bounds of its instances (the `MultiMesh`'s own; unused when empty).
    pub(crate) bounds: Aabb,
}

/// What a frame did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    /// Batches written.
    pub(crate) written: usize,
    /// Batches hidden (emptied or their chunk left).
    pub(crate) hidden: usize,
    /// Bytes of rows written.
    pub(crate) bytes: usize,
    /// Batches not drawn: every slot of the limit was live.
    pub(crate) blocked: usize,
}

/// The building batches' slots.
pub(crate) struct BuildingLayer {
    pool: SlotPool,
    looks: Map<BatchKey, Look>,
    drawn: Map<(ChunkKey, BatchKey), SlotId>,
    missing: Vec<BatchKey>,
}

impl BuildingLayer {
    /// A layer of at most `limit` slots, `spare_limit` of them spare.
    pub(crate) fn new(limit: usize, spare_limit: usize) -> Self {
        Self {
            pool: SlotPool::new(limit, spare_limit),
            looks: Map::default(),
            drawn: Map::default(),
            missing: Vec::new(),
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

    /// Registers a group's look (`building_lods.gd`'s mesh and override for it).
    pub(crate) fn register(&mut self, key: BatchKey, look: Look) {
        self.looks.insert(key, look);
    }

    /// Frees every slot and forgets every look (a new catalogue).
    pub(crate) fn reset(&mut self, backend: &mut impl RenderBackend) {
        self.pool.clear(backend);
        self.looks.clear();
        self.drawn.clear();
        self.missing.clear();
    }

    /// The groups of non-empty changed batches that have no look yet, each once, in key order.
    /// `O(changed batches)`.
    pub(crate) fn missing<'a>(&mut self, changes: impl Iterator<Item = Change<'a>>) -> &[BatchKey] {
        self.missing.clear();
        for change in changes {
            if !change.rows.is_empty() && !self.looks.contains_key(&change.key) {
                self.missing.push(change.key);
            }
        }
        self.missing.sort_unstable();
        self.missing.dedup();
        &self.missing
    }

    /// Applies a frame: the removed batches' slots return to the pool hidden, every changed
    /// batch is written and drawn, or hidden when empty. Every non-empty group must have its
    /// look ([`BuildingLayer::missing`] empty).
    pub(crate) fn apply<'a>(
        &mut self,
        backend: &mut impl RenderBackend,
        removed: &[(ChunkKey, BatchKey)],
        changes: impl Iterator<Item = Change<'a>>,
    ) -> Applied {
        let mut applied = Applied::default();
        for at in removed {
            if let Some(slot) = self.drawn.remove(at) {
                self.pool.release(backend, slot);
                applied.hidden += 1;
            }
        }
        for change in changes {
            let at = (change.chunk, change.key);
            if change.rows.is_empty() {
                if let Some(slot) = self.drawn.remove(&at) {
                    self.pool.release(backend, slot);
                    applied.hidden += 1;
                }
                continue;
            }
            let look = self.looks[&change.key];
            let count = change.rows.len() as u32;
            let slot = match self.slot_for(backend, at, look, count) {
                Some(slot) => slot,
                None => {
                    applied.blocked += 1;
                    continue;
                }
            };
            let rows = change.rows.as_flattened();
            self.pool.fill(backend, slot, rows, count, change.bounds);
            self.draw(backend, slot, look);
            applied.written += 1;
            applied.bytes += std::mem::size_of_val(rows);
        }
        applied
    }

    // The batch's slot: its own while it holds `count`, else one of the size class (the old one
    // released). `None` when the pool is full.
    fn slot_for(
        &mut self,
        backend: &mut impl RenderBackend,
        at: (ChunkKey, BatchKey),
        look: Look,
        count: u32,
    ) -> Option<SlotId> {
        let (class, _) = capacity_class(count);
        if let Some(&slot) = self.drawn.get(&at) {
            let kind = self.pool.slot(slot).kind();
            // A slot keeps its peak size while its batch is drawn, as the nodes did.
            if kind.mesh == look.mesh && kind.class >= class {
                return Some(slot);
            }
        }
        let kind = SlotKind {
            mesh: look.mesh,
            class,
            colors: false,
            custom: true,
            parameter: None,
        };
        let slot = self.pool.acquire(backend, kind)?;
        if let Some(old) = self.drawn.insert(at, slot) {
            self.pool.release(backend, old);
        }
        Some(slot)
    }

    // Draws a written slot in the world and in the reflection's; settings the pool does not
    // track are sent to an instance as it is made.
    fn draw(&mut self, backend: &mut impl RenderBackend, slot: SlotId, look: Look) {
        let fresh = self.pool.slot(slot).drawn(Role::Main).is_none();
        let main = Drawn {
            origin: [0.0; 3],
            aabb: Aabb::default(),
            range: [0.0; 2],
            material: look.material,
            shadow: Shadow::On,
            visible: true,
        };
        self.pool.draw(backend, slot, Role::Main, main);
        if fresh && let Some(instance) = self.pool.slot(slot).instance(Role::Main) {
            backend.instance_set_extra_margin(instance, EXTRA_MARGIN_M);
            backend.instance_set_dynamic_gi(instance, true);
        }
        self.draw_reflection(backend, slot, look);
    }

    fn draw_reflection(&mut self, backend: &mut impl RenderBackend, slot: SlotId, look: Look) {
        let fresh = self.pool.slot(slot).drawn(Role::Reflection).is_none();
        let reflection = Drawn {
            origin: [0.0; 3],
            aabb: Aabb::default(),
            range: [0.0, REFLECTION_RANGE_M],
            material: look.material,
            shadow: Shadow::Off,
            visible: true,
        };
        if !self.pool.draw(backend, slot, Role::Reflection, reflection) || !fresh {
            return;
        }
        if let Some(instance) = self.pool.slot(slot).instance(Role::Reflection) {
            backend.instance_set_flag_parameter(instance, REFLECTION_FLAG, true);
            backend.instance_set_range_margins(
                instance,
                [0.0, REFLECTION_RANGE_M],
                [0.0, REFLECTION_MARGIN_M],
            );
            backend.instance_set_extra_margin(instance, EXTRA_MARGIN_M);
        }
    }

    /// Drops every reflection proxy (the reflection's world changes).
    pub(crate) fn drop_reflections(&mut self, backend: &mut impl RenderBackend) {
        self.pool.drop_reflections(backend);
    }

    /// Draws every drawn batch's reflection proxy again (a new reflection world). `O(batches)`.
    pub(crate) fn redraw_reflections(&mut self, backend: &mut impl RenderBackend) {
        let drawn: Vec<_> = self.drawn.iter().map(|(at, &slot)| (at.1, slot)).collect();
        for (key, slot) in drawn {
            let look = self.looks[&key];
            self.draw_reflection(backend, slot, look);
        }
    }

    /// The drawn batches: chunk, group, slot. `O(batches)`.
    pub(crate) fn batches(&self) -> impl Iterator<Item = (ChunkKey, BatchKey, SlotId)> + '_ {
        self.drawn.iter().map(|(&(chunk, key), &slot)| (chunk, key, slot))
    }

    /// The batches drawn now.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.drawn.len()
    }
}

#[cfg(test)]
mod tests;
