// SPDX-License-Identifier: GPL-2.0-only

//! The street lights' batches on pooled `RenderingServer` slots (`RENDER-52` part 2, round CB3):
//! what `street_lights.gd` made as one `MultiMeshInstance3D` node, `MultiMesh` resource and
//! chunk root a batch, drawn through the stream backend's [`SlotPool`] instead, with the settings
//! those nodes gave the server: per 64 m cell and kind three level-of-detail batches (the lamp
//! material, the band's visibility range and hysteresis margins, the static casters' shadow and
//! extra cull margin on the first two, a receiver's on the last), and per chunk one glow batch
//! (the quad's own material, no shadow, a 2 m margin). Instances stand at the chunk's frame
//! origin, as the batch nodes did under their chunk root.
//!
//! A chunk's batches are made in parts ([`LampDraw::build_part`]) and shown together once all
//! are made, as the root entered the tree whole. A chunk's slots go back to the pool when it is
//! replaced or removed (hidden at once); a world reset ([`LampDraw::clear`]) and the renderer's
//! exit free every slot, spares included ([`LampDraw::ledger`] counts them). No node, resource
//! or object of the engine's object table is made.
//!
//! Costs: a batch is one slot acquire (a spare of its mesh and size class, else a new
//! `MultiMesh` and instance), one buffer write and its drawn state, `O(lamps)` floats copied; a
//! chunk `O(batches + lamps)`; a removal `O(chunk batches)`; a reset `O(slots)`.

use super::{ChunkBatches, KINDS};
use crate::nodes::sim::render::stream::backend::{
    Drawn, RenderBackend, Role, Shadow, SlotId, SlotKind, SlotPool, capacity_class,
};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use std::collections::HashMap;
use std::time::Instant;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// Levels of detail a lamp model has (the GLBs' `_LOD0`.. `_LOD2`).
pub(crate) const LODS: usize = 3;
/// The catalogue's mesh index of the glow quad; kind `k`'s level `l` is `k * LODS + l`.
pub(crate) const GLOW_MESH: u32 = (KINDS * LODS) as u32;
/// Meshes the catalogue holds.
pub(crate) const MESHES: usize = KINDS * LODS + 1;
// `street_lights.gd`'s distance bands from each batch's centre and their hysteresis.
const LOD_RANGES: [[f32; 2]; LODS] = [[0.0, 140.0], [140.0, 420.0], [420.0, 1400.0]];
const LOD_HYSTERESIS_M: f32 = 10.0;
// `SceneLighting`'s extra cull margins: a static caster's (levels 0 and 1), a shadow receiver's
// (level 2; the glow sets the same 2 m itself).
const CASTER_MARGIN_M: f32 = 32.0;
const RECEIVER_MARGIN_M: f32 = 2.0;
// The lamp material, the catalogue's only one (the glow draws with its quad's own).
const LAMP_MATERIAL: u32 = 0;
// The lamp shader has no instance uniforms, so the bound is the slots' memory only: far above
// the 30k city's 11,670 lamp slots and the 100k city's. A load's prebuild (`LampDraw::prebuild`)
// frees the previous world's spares first; an edit releases its chunk's slots and takes the same
// kinds back.
const SLOT_LIMIT: usize = 1 << 18;
const SPARE_LIMIT: usize = 1 << 16;
// Floats an instance: a 3D transform; the glow's adds its colour.
const STRIDE: usize = 12;
const GLOW_STRIDE: usize = 16;

// A chunk whose slots are being made: its batches, its origin and the slots made so far (in
// batch order, each batch's levels then the glow).
struct Building {
    origin: [f32; 3],
    batches: ChunkBatches,
    next: usize,
    slots: Vec<SlotId>,
}

// A chunk built ahead of its load (`LampDraw::prebuild`): its slots written and hidden, kept
// with what they draw so a `begin` with the same batches takes them as they are.
struct Prebuilt {
    origin: [f32; 3],
    batches: ChunkBatches,
    slots: Vec<SlotId>,
}

/// Counts of the slots the draw made and freed: every slot is one `MultiMesh` and one instance,
/// so `made - freed = live + spares` holds the handles alive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ledger {
    /// Slots made since the draw began.
    pub made: u64,
    /// Slots freed since the draw began.
    pub freed: u64,
    /// Slots drawing a chunk or being made for one.
    pub live: usize,
    /// Slots kept for reuse.
    pub spares: usize,
}

/// The street lights' slots by chunk.
pub(crate) struct LampDraw {
    pool: SlotPool,
    drawn: Map<(i32, i32), Vec<SlotId>>,
    building: Map<(i32, i32), Building>,
    prebuilt: Map<(i32, i32), Prebuilt>,
    cast_shadows: bool,
    // The prewarm's spares were made (once a draw: a reset keeps spares).
    warm: bool,
}

impl Default for LampDraw {
    fn default() -> Self {
        Self {
            pool: SlotPool::new(SLOT_LIMIT, SPARE_LIMIT),
            drawn: Map::default(),
            building: Map::default(),
            prebuilt: Map::default(),
            cast_shadows: true,
            warm: false,
        }
    }
}

impl LampDraw {
    /// Whether reused slots take fresh storage (the viewport draws motion vectors: a new batch
    /// node's `MultiMesh` had no previous upload either).
    pub(crate) fn set_fresh(&mut self, fresh: bool) {
        self.pool.set_fresh(fresh);
    }

    /// Starts chunk `key`'s batches at its frame `origin`, replacing what it drew (released at
    /// once, as the old root was freed). Nothing draws until [`LampDraw::build_part`] made them
    /// all. A chunk without batches only releases.
    pub(crate) fn begin(
        &mut self,
        backend: &mut impl RenderBackend,
        key: (i32, i32),
        origin: [f32; 3],
        batches: ChunkBatches,
    ) {
        let prebuilt = self.prebuilt.remove(&key);
        self.remove(backend, key);
        if let Some(prebuilt) = prebuilt {
            if prebuilt.origin == origin && prebuilt.batches == batches {
                // Every batch made: the next part shows them.
                let made = ChunkBatches {
                    kinds: prebuilt.batches.kinds,
                    ..ChunkBatches::default()
                };
                let next = made.kinds.len();
                self.building.insert(
                    key,
                    Building {
                        origin,
                        batches: made,
                        next,
                        slots: prebuilt.slots,
                    },
                );
                return;
            }
            for slot in prebuilt.slots {
                self.pool.release(backend, slot);
            }
        }
        if batches.kinds.is_empty() {
            return;
        }
        let slots = Vec::with_capacity(batches.kinds.len() * LODS + 1);
        self.building.insert(
            key,
            Building {
                origin,
                batches,
                next: 0,
                slots,
            },
        );
    }

    /// Makes one spare slot of every mesh in its smallest size class, drawn hidden with the
    /// settings its batches take, so the engine's first-use costs of each mesh (its buffers,
    /// its instance's surfaces and pipelines) are paid here, in the load call, and not by the
    /// first batch of that mesh in a load frame (4-9 ms lights parts at 30k, `RENDER-52` part
    /// 2). Those batches take the spares. `O(meshes)`; once a draw.
    pub(crate) fn prewarm(&mut self, backend: &mut impl RenderBackend) {
        if self.warm {
            return;
        }
        self.warm = true;
        const IDENTITY: [f32; GLOW_STRIDE] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0,
        ];
        let mut slots = Vec::with_capacity(MESHES);
        for kind in 0..KINDS as u32 {
            for lod in 0..LODS {
                if let Some(slot) =
                    self.lod_batch(backend, kind, lod, &IDENTITY[..STRIDE], [0.0; 3])
                {
                    slots.push(slot);
                }
            }
        }
        if let Some(slot) = self.glow_batch(backend, &IDENTITY, [0.0; 3]) {
            slots.push(slot);
        }
        for slot in slots {
            self.pool.release(backend, slot);
        }
    }

    /// Builds a world's lamp batches ahead of its load (`RENDER-52` part 2, round CB3): frees
    /// what the previous world left (its prebuilt chunks and spares), makes the prewarm's
    /// spares again, then makes, writes and draws hidden every batch of `chunks` (key, frame
    /// origin, batches). A slot's creation and a buffer's upload now and then stall the main
    /// thread for 1-3 ms (the engine's allocations and staging, not predictable from the call);
    /// done here, in the load call, a load frame's lights part only shows a chunk whose
    /// [`LampDraw::begin`] brings the same batches. Returns the slots made.
    /// `O(lamps + batches + slots freed)`.
    pub(crate) fn prebuild(
        &mut self,
        backend: &mut impl RenderBackend,
        chunks: impl Iterator<Item = ((i32, i32), [f32; 3], ChunkBatches)>,
    ) -> u64 {
        let old: Vec<SlotId> = self
            .prebuilt
            .drain()
            .flat_map(|(_, prebuilt)| prebuilt.slots)
            .collect();
        for slot in old {
            self.pool.release(backend, slot);
        }
        self.pool.free_spares(backend);
        self.warm = false;
        self.prewarm(backend);
        let made = self.pool.churn().0;
        for (key, origin, batches) in chunks {
            if batches.kinds.is_empty() {
                continue;
            }
            let mut slots = Vec::with_capacity(batches.kinds.len() * LODS + 1);
            for (&kind, rows) in batches.kinds.iter().zip(&batches.transforms) {
                for lod in 0..LODS {
                    slots.extend(self.lod_batch(backend, u32::from(kind), lod, rows, origin));
                }
            }
            slots.extend(self.glow_batch(backend, &batches.glow, origin));
            if slots.len() != batches.kinds.len() * LODS + 1 {
                // The pool refused a slot: the chunk builds in its own parts.
                for slot in slots {
                    self.pool.release(backend, slot);
                }
                continue;
            }
            self.prebuilt.insert(
                key,
                Prebuilt {
                    origin,
                    batches,
                    slots,
                },
            );
        }
        self.pool.churn().0 - made
    }

    /// Whether chunk `key`'s slots are still being made.
    pub(crate) fn is_building(&self, key: (i32, i32)) -> bool {
        self.building.contains_key(&key)
    }

    /// Makes chunk `key`'s next batches until `budget_us` has passed (one batch at least), then
    /// its glow and shows them all; true when none is left (also for a chunk not building).
    /// False, leaving the rest for later, when the pool refuses a slot.
    pub(crate) fn build_part(
        &mut self,
        backend: &mut impl RenderBackend,
        key: (i32, i32),
        budget_us: f64,
    ) -> bool {
        let Some(mut building) = self.building.remove(&key) else {
            return true;
        };
        let start = Instant::now();
        let count = building.batches.kinds.len();
        while building.next < count {
            let batch = building.next;
            let kind = u32::from(building.batches.kinds[batch]);
            let rows = std::mem::take(&mut building.batches.transforms[batch]);
            for lod in 0..LODS {
                let Some(slot) = self.lod_batch(backend, kind, lod, &rows, building.origin) else {
                    building.batches.transforms[batch] = rows;
                    self.building.insert(key, building);
                    return false;
                };
                building.slots.push(slot);
            }
            building.next = batch + 1;
            if start.elapsed().as_secs_f64() * 1e6 >= budget_us && building.next < count {
                self.building.insert(key, building);
                return false;
            }
        }
        // Every batch of a chunk with lights has a glow quad a head (a chunk whose lights all
        // lack their model has no batch and never builds).
        if !building.batches.glow.is_empty() {
            let Some(slot) = self.glow_batch(backend, &building.batches.glow, building.origin)
            else {
                self.building.insert(key, building);
                return false;
            };
            building.slots.push(slot);
            building.batches.glow = Vec::new();
        }
        for &slot in &building.slots {
            self.pool.set_visible(backend, slot, Role::Main, true);
        }
        self.drawn.insert(key, building.slots);
        true
    }

    // One level of a cell's batch, hidden.
    fn lod_batch(
        &mut self,
        backend: &mut impl RenderBackend,
        kind: u32,
        lod: usize,
        rows: &[f32],
        origin: [f32; 3],
    ) -> Option<SlotId> {
        let count = (rows.len() / STRIDE) as u32;
        let slot = self.pool.acquire(
            backend,
            slot_kind(kind * LODS as u32 + lod as u32, count, false),
        )?;
        self.pool.fill(backend, slot, rows, count, Aabb::default());
        let fresh = self.pool.slot(slot).drawn(Role::Main).is_none();
        let range = LOD_RANGES[lod];
        let caster = lod < 2;
        self.pool.draw(
            backend,
            slot,
            Role::Main,
            Drawn {
                origin,
                aabb: Aabb::default(),
                range,
                material: Some(LAMP_MATERIAL),
                shadow: if caster && self.cast_shadows {
                    Shadow::On
                } else {
                    Shadow::Off
                },
                visible: false,
            },
        );
        if fresh && let Some(instance) = self.pool.slot(slot).instance(Role::Main) {
            // The margins a slot of this mesh always has (a slot keeps its mesh, so its level).
            let begin_margin = if lod > 0 { LOD_HYSTERESIS_M } else { 0.0 };
            backend.instance_set_range_margins(instance, range, [begin_margin, LOD_HYSTERESIS_M]);
            backend.instance_set_extra_margin(
                instance,
                if caster {
                    CASTER_MARGIN_M
                } else {
                    RECEIVER_MARGIN_M
                },
            );
        }
        Some(slot)
    }

    // The chunk's glow quads, hidden.
    fn glow_batch(
        &mut self,
        backend: &mut impl RenderBackend,
        rows: &[f32],
        origin: [f32; 3],
    ) -> Option<SlotId> {
        let count = (rows.len() / GLOW_STRIDE) as u32;
        let slot = self
            .pool
            .acquire(backend, slot_kind(GLOW_MESH, count, true))?;
        self.pool.fill(backend, slot, rows, count, Aabb::default());
        let fresh = self.pool.slot(slot).drawn(Role::Main).is_none();
        self.pool.draw(
            backend,
            slot,
            Role::Main,
            Drawn {
                origin,
                aabb: Aabb::default(),
                range: [0.0; 2],
                material: None,
                shadow: Shadow::Off,
                visible: false,
            },
        );
        if fresh && let Some(instance) = self.pool.slot(slot).instance(Role::Main) {
            // The quads are expanded in the vertex shader: culling keeps them at the edges.
            backend.instance_set_extra_margin(instance, RECEIVER_MARGIN_M);
        }
        Some(slot)
    }

    /// Releases chunk `key`'s slots, drawn or being made (hidden at once). `O(chunk batches)`.
    pub(crate) fn remove(&mut self, backend: &mut impl RenderBackend, key: (i32, i32)) {
        let drawn = self.drawn.remove(&key);
        let building = self.building.remove(&key).map(|building| building.slots);
        let prebuilt = self.prebuilt.remove(&key).map(|prebuilt| prebuilt.slots);
        for slot in [drawn, building, prebuilt].into_iter().flatten().flatten() {
            self.pool.release(backend, slot);
        }
    }

    /// Releases every drawn chunk's slots and those being made (a world reset): hidden at once,
    /// kept as spares up to the spare limit, the rest freed. The prebuilt chunks stay: they are
    /// the world being loaded (the hydration's first frame resets the renderer after the load
    /// call built them); the next prebuild releases what is left of them. `O(slots)`.
    pub(crate) fn clear(&mut self, backend: &mut impl RenderBackend) {
        let drawn = self.drawn.drain().map(|(_, slots)| slots);
        let building = self.building.drain().map(|(_, building)| building.slots);
        let slots: Vec<SlotId> = drawn.chain(building).flatten().collect();
        for slot in slots {
            self.pool.release(backend, slot);
        }
    }

    /// Frees every slot, spares included (the renderer's exit: no slot outlives it, nor the
    /// meshes they name). `O(slots)`.
    pub(crate) fn free_all(&mut self, backend: &mut impl RenderBackend) {
        self.drawn.clear();
        self.building.clear();
        self.prebuilt.clear();
        self.pool.clear(backend);
        self.warm = false;
    }

    /// Whether the first two levels cast the sun's shadow (a shadow benchmark's class toggle;
    /// the game always casts): drawn and future slots.
    pub(crate) fn set_cast_shadows(&mut self, backend: &mut impl RenderBackend, cast: bool) {
        if self.cast_shadows == cast {
            return;
        }
        self.cast_shadows = cast;
        let shadow = if cast { Shadow::On } else { Shadow::Off };
        let slots: Vec<SlotId> = self
            .drawn
            .values()
            .chain(self.building.values().map(|building| &building.slots))
            .chain(self.prebuilt.values().map(|prebuilt| &prebuilt.slots))
            .flatten()
            .copied()
            .collect();
        for slot in slots {
            let kind = self.pool.slot(slot).kind();
            if kind.mesh != GLOW_MESH && (kind.mesh as usize) % LODS < 2 {
                let visible = self
                    .pool
                    .slot(slot)
                    .drawn(Role::Main)
                    .is_some_and(|drawn| drawn.visible);
                self.pool
                    .set_shadow(backend, slot, Role::Main, shadow, visible);
            }
        }
    }

    /// The draw's slot counts.
    pub(crate) fn ledger(&self) -> Ledger {
        let (made, freed) = self.pool.churn();
        Ledger {
            made,
            freed,
            live: self.pool.live(),
            spares: self.pool.spares(),
        }
    }

    /// Chunk `key`'s drawn instances: the level-of-detail batches' and the glow's, as last sent
    /// (`(0, 0)` while it is not drawn). `O(chunk batches)`.
    pub(crate) fn drawn_instances(&self, key: (i32, i32)) -> (u32, u32) {
        let mut counts = (0, 0);
        for &slot in self.drawn.get(&key).into_iter().flatten() {
            let slot = self.pool.slot(slot);
            let count = slot.visible_count().unwrap_or(0);
            if slot.kind().mesh == GLOW_MESH {
                counts.1 += count;
            } else {
                counts.0 += count;
            }
        }
        counts
    }
}

// The pool kind of a batch of `count` instances of `mesh`.
fn slot_kind(mesh: u32, count: u32, colors: bool) -> SlotKind {
    SlotKind {
        mesh,
        class: capacity_class(count).0,
        colors,
        custom: false,
        parameter: None,
    }
}

#[cfg(test)]
mod tests;
