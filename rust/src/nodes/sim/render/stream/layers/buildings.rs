// SPDX-License-Identifier: GPL-3.0-or-later

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
//! A chunk's changes are applied whole or not at all, as the bridge published them: a building
//! changes tier inside its chunk in one frame, never drawn twice or not at all. A group's mesh and
//! material are Godot resources `building_lods.gd` builds as a group is first needed; while a
//! changed batch's group has none, [`BuildingLayer::missing`] names it and the frame waits for the
//! registration. The removals of chunks that left are always applied. A chunk whose changes need
//! more slots than the pool can give (every slot of the limit live: new batches, or batches that
//! outgrew their slot's size class) sends nothing and waits (`RENDER-42`, AUDIT-stream-2 finding
//! 6): the bridge keeps that chunk's changes unacknowledged and offers them again every frame,
//! with whatever later frames added, until they fit, as the vegetation keeps a refused product
//! pending. Chunks are admitted in the frame's publication order after every chunk that gives up
//! at least the slots it takes, so a view needing more than the limit draws what fits and keeps
//! updating it instead of freezing every building.
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
#[derive(Clone, Copy)]
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
    /// Slots the waiting chunks are short of: nothing of them was drawn (`RENDER-42`).
    pub(crate) blocked: usize,
}

// A changed chunk of a frame: the slots it takes and gives up, and whether it is drawn.
#[derive(Clone, Copy, Default)]
struct Admission {
    takes: usize,
    gives: usize,
    drawn: Option<bool>,
}

/// The building batches' slots.
pub(crate) struct BuildingLayer {
    pool: SlotPool,
    looks: Map<BatchKey, Look>,
    drawn: Map<(ChunkKey, BatchKey), SlotId>,
    missing: Vec<BatchKey>,
    // The frame's changed chunks: the slots each takes and gives up, and whether it is drawn
    // (`None` until decided). Reused storage; after a frame, the chunks that wait.
    chunks: Map<ChunkKey, Admission>,
    // Some chunk of the last frame offered could not be drawn and waits (`RENDER-42`).
    waiting: bool,
    // Whether the batches cast the sun's shadow (the shadow benchmark's caster classes,
    // `RENDER-15`); the game never clears it.
    cast_shadows: bool,
}

impl BuildingLayer {
    /// A layer of at most `limit` slots, `spare_limit` of them spare.
    pub(crate) fn new(limit: usize, spare_limit: usize) -> Self {
        Self {
            pool: SlotPool::new(limit, spare_limit),
            looks: Map::default(),
            drawn: Map::default(),
            missing: Vec::new(),
            chunks: Map::default(),
            waiting: false,
            cast_shadows: true,
        }
    }

    /// Whether some chunk of the last frame offered waits for slots: the bridge offers its
    /// changes again, unchanged or grown by later frames, until they are drawn whole.
    pub(crate) fn waiting(&self) -> bool {
        self.waiting
    }

    /// Whether `chunk`'s changes of the last frame offered wait for slots: the bridge keeps them
    /// unacknowledged. `O(1)`.
    pub(crate) fn waits(&self, chunk: ChunkKey) -> bool {
        self.waiting && self.chunks.get(&chunk).is_some_and(|chunk| chunk.drawn == Some(false))
    }

    /// The pool (tests and status).
    pub(crate) fn pool(&self) -> &SlotPool {
        &self.pool
    }

    /// Whether reused slots get fresh storage (motion vectors on).
    pub(crate) fn set_fresh_buffers(&mut self, fresh: bool) {
        self.pool.set_fresh(fresh);
    }

    /// Whether the batches cast the sun's shadow, every drawn batch at once and later ones as
    /// drawn (the shadow benchmark's caster classes, `RENDER-15`). `O(drawn batches)`.
    pub(crate) fn set_cast_shadows(&mut self, backend: &mut impl RenderBackend, cast: bool) {
        self.cast_shadows = cast;
        let shadow = if cast { Shadow::On } else { Shadow::Off };
        for &slot in self.drawn.values() {
            self.pool.set_shadow(backend, slot, Role::Main, shadow, true);
        }
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
        self.chunks.clear();
        self.waiting = false;
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

    /// Applies a frame chunk by chunk, each whole or not at all: the removed batches' slots return
    /// to the pool hidden; a changed chunk is drawn when the pool can give the slots it needs once
    /// the frame's own releases are counted, its batches written and drawn, or hidden when empty;
    /// a chunk that does not fit sends nothing, [`BuildingLayer::waits`] names it,
    /// [`Applied::blocked`] counts the slots the waiting chunks are short of, and
    /// [`BuildingLayer::waiting`] holds until a frame fits whole (`RENDER-42`). Every non-empty
    /// group must have its look ([`BuildingLayer::missing`] empty).
    /// `O(removed + changed batches + their instances)`.
    pub(crate) fn apply<'a>(
        &mut self,
        backend: &mut impl RenderBackend,
        removed: &[(ChunkKey, BatchKey)],
        changes: impl Iterator<Item = Change<'a>> + Clone,
    ) -> Applied {
        let mut applied = Applied::default();
        // The chunks that left first: their slots are room for the changed ones.
        for at in removed {
            if let Some(slot) = self.drawn.remove(at) {
                self.pool.release(backend, slot);
                applied.hidden += 1;
            }
        }
        applied.blocked = self.admit(changes.clone());
        self.waiting = applied.blocked > 0;
        // Every slot the admitted chunks give up first, so a batch moving to a larger size class
        // can take the room its old slot leaves; a frame draws at its end, so the order is not
        // seen.
        for change in changes.clone() {
            let at = (change.chunk, change.key);
            let Some(&slot) = self.drawn.get(&at) else {
                continue;
            };
            if self.chunks[&change.chunk].drawn != Some(true) {
                continue;
            }
            if change.rows.is_empty() {
                applied.hidden += 1;
            } else if self.keeps(slot, self.looks[&change.key], change.rows.len() as u32) {
                continue;
            }
            self.drawn.remove(&at);
            self.pool.release(backend, slot);
        }
        for change in changes {
            if change.rows.is_empty() || self.chunks[&change.chunk].drawn != Some(true) {
                continue;
            }
            let at = (change.chunk, change.key);
            let look = self.looks[&change.key];
            let count = change.rows.len() as u32;
            let slot = match self.drawn.get(&at) {
                Some(&slot) => slot,
                None => {
                    let kind = SlotKind {
                        mesh: look.mesh,
                        class: capacity_class(count).0,
                        colors: false,
                        custom: true,
                        parameter: None,
                    };
                    let slot = self
                        .pool
                        .acquire(backend, kind)
                        .expect("the chunk was checked to fit the pool");
                    self.drawn.insert(at, slot);
                    slot
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

    // Decides which changed chunks are drawn (after the removals): a chunk needs a slot for a
    // non-empty batch not drawn or whose slot no longer fits it, and gives up the slot of an
    // emptied batch or of one that moves. Chunks that give up at least what they take are drawn
    // first, then the rest in the frame's order while the pool has room. Returns the slots the
    // waiting chunks are short of together (0: the frame is drawn whole).
    // `O(changed batches)`, nothing sent.
    fn admit<'a>(&mut self, changes: impl Iterator<Item = Change<'a>> + Clone) -> usize {
        self.chunks.clear();
        for change in changes.clone() {
            let slot = self.drawn.get(&(change.chunk, change.key)).copied();
            let (takes, gives) = match slot {
                _ if change.rows.is_empty() => (0, usize::from(slot.is_some())),
                Some(slot) if self.keeps(slot, self.looks[&change.key], change.rows.len() as u32) => {
                    (0, 0)
                }
                Some(_) => (1, 1),
                None => (1, 0),
            };
            let admission = self.chunks.entry(change.chunk).or_default();
            admission.takes += takes;
            admission.gives += gives;
        }
        let mut room = self.pool.available();
        for admission in self.chunks.values_mut() {
            if admission.takes <= admission.gives {
                admission.drawn = Some(true);
                room += admission.gives - admission.takes;
            }
        }
        let mut short = 0;
        for change in changes {
            let admission = self.chunks.get_mut(&change.chunk).expect("counted above");
            if admission.drawn.is_some() {
                continue;
            }
            let need = admission.takes - admission.gives;
            admission.drawn = Some(need <= room);
            if need <= room {
                room -= need;
            } else {
                short += need;
            }
        }
        // Each waiting chunk needed more than the room left when it was considered, and the room
        // only shrank, so a waiting frame is short of at least one slot.
        short.saturating_sub(room)
    }

    // Whether a drawn batch keeps its slot: the same mesh and a size class that holds `count`
    // (a slot keeps its peak size while its batch is drawn, as the nodes did).
    fn keeps(&self, slot: SlotId, look: Look, count: u32) -> bool {
        let kind = self.pool.slot(slot).kind();
        kind.mesh == look.mesh && kind.class >= capacity_class(count).0
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

    /// Whether batch `key` of `chunk` is drawn (the stream benchmark's drawn probe). `O(1)`.
    pub(crate) fn draws(&self, chunk: ChunkKey, key: BatchKey) -> bool {
        self.drawn.contains_key(&(chunk, key))
    }

    /// The batches drawn now.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.drawn.len()
    }
}

#[cfg(test)]
mod tests;
