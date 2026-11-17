// SPDX-License-Identifier: GPL-2.0-only

//! The street lights' batches on pooled `RenderingServer` slots (`RENDER-52` part 2, round CB3;
//! chunk batches, `RENDER-46` lever 5, round SL1): per road chunk, pole kind and level of detail
//! one batch (the lamp material, the static casters' shadow and extra cull margin on the first
//! two levels, a receiver's on the last; no visibility range) and per chunk one glow batch (the
//! quad's own material, no shadow, a 2 m margin), drawn through the stream backend's
//! [`SlotPool`]. Instances stand at the chunk's frame origin. Which cells' poles a level's batch
//! holds is [`PoleLods`]' choice from the main camera before each frame draws
//! ([`LampDraw::select`]): the level each 64 m cell's own batch drew under Godot's visibility
//! range, as the batches drew before (`poles.rs`).
//!
//! A chunk's slots are made in parts ([`LampDraw::build_part`]) and drawn together once all are
//! made, as the root entered the tree whole: the glow shows then, the level batches at the next
//! selection. A chunk's slots go back to the pool when it is replaced or removed (hidden at
//! once); a world reset ([`LampDraw::clear`]) and the renderer's exit free every slot, spares
//! included ([`LampDraw::ledger`] counts them). No node, resource or object of the engine's
//! object table is made.
//!
//! Costs: a slot is one acquire (a spare of its mesh and size class, else a new `MultiMesh` and
//! instance) and its drawn state; a chunk `O(kinds + lamps)` (its cells' centres); a selection
//! [`PoleLods::select`]'s plus, per changed batch, one buffer write of its poles; a removal
//! `O(chunk slots)`; a reset `O(slots)`.

use super::poles::{ChunkPoles, Level, PoleLods};
use super::{ChunkBatches, KINDS, Map};
use crate::nodes::sim::render::stream::backend::{
    Drawn, RenderBackend, Role, Shadow, SlotId, SlotKind, SlotPool, capacity_class,
};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use std::time::Instant;

pub(crate) use super::poles::LODS;

/// The catalogue's mesh index of the glow quad; kind `k`'s level `l` is `k * LODS + l`.
pub(crate) const GLOW_MESH: u32 = (KINDS * LODS) as u32;
/// Meshes the catalogue holds.
pub(crate) const MESHES: usize = KINDS * LODS + 1;
// The distance bands from each 64 m cell's batch centre and their hysteresis, as the cell
// batches had them (`street_lights.gd`'s `LOD_RANGES`).
const LOD_RANGES: [[f32; 2]; LODS] = [[0.0, 140.0], [140.0, 420.0], [420.0, 1400.0]];
const LOD_HYSTERESIS_M: f32 = 10.0;
// `SceneLighting`'s extra cull margins: a static caster's (levels 0 and 1), a shadow receiver's
// (level 2; the glow sets the same 2 m itself).
const CASTER_MARGIN_M: f32 = 32.0;
const RECEIVER_MARGIN_M: f32 = 2.0;
// Each level's range, margins and cull margin.
const LEVELS: [Level; LODS] = [level(0), level(1), level(2)];
// The lamp material, the catalogue's only one (the glow draws with its quad's own).
const LAMP_MATERIAL: u32 = 0;
// The lamp shader has no instance uniforms, so the bound is the slots' memory only: far above
// the 30k city's lamp slots and the 100k city's. A load's prebuild (`LampDraw::prebuild`) frees
// the previous world's spares first; an edit releases its chunk's slots and takes the same
// kinds back.
const SLOT_LIMIT: usize = 1 << 18;
const SPARE_LIMIT: usize = 1 << 16;
// Floats an instance: a 3D transform; the glow's adds its colour.
const STRIDE: usize = 12;
const GLOW_STRIDE: usize = 16;

const fn level(lod: usize) -> Level {
    Level {
        begin: LOD_RANGES[lod][0],
        end: LOD_RANGES[lod][1],
        begin_margin: if lod > 0 { LOD_HYSTERESIS_M } else { 0.0 },
        end_margin: LOD_HYSTERESIS_M,
        cull_margin: if lod < 2 {
            CASTER_MARGIN_M
        } else {
            RECEIVER_MARGIN_M
        },
    }
}

// A chunk whose slots are being made: its poles, its origin, its glow rows (taken once written)
// and the slots made so far (each pole kind's levels in slot order, then the glow).
struct Building {
    origin: [f32; 3],
    poles: ChunkPoles,
    glow: Vec<f32>,
    next: usize,
    slots: Vec<SlotId>,
}

// A chunk built ahead of its load (`LampDraw::prebuild`): its slots made and hidden, the glow
// written, kept with what they draw so a `begin` with the same batches takes them as they are.
struct Prebuilt {
    origin: [f32; 3],
    batches: ChunkBatches,
    poles: ChunkPoles,
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
    // Per drawn chunk its slots: slot `s * LODS + l` draws pole kind slot `s` at level `l`, the
    // glow's last.
    drawn: Map<(i32, i32), Vec<SlotId>>,
    building: Map<(i32, i32), Building>,
    prebuilt: Map<(i32, i32), Prebuilt>,
    // The drawn chunks' cell levels.
    poles: PoleLods,
    // Per kind and level, the mesh's bounds (`set_mesh_bounds`).
    meshes: [[Option<Aabb>; LODS]; KINDS],
    cast_shadows: bool,
    // The prewarm's spares were made (once a draw: a reset keeps spares).
    warm: bool,
    // A selection's changed batches and one batch's rows.
    changed: Vec<i32>,
    rows: Vec<f32>,
}

impl Default for LampDraw {
    fn default() -> Self {
        Self {
            pool: SlotPool::new(SLOT_LIMIT, SPARE_LIMIT),
            drawn: Map::default(),
            building: Map::default(),
            prebuilt: Map::default(),
            poles: PoleLods::new(LEVELS),
            meshes: [[None; LODS]; KINDS],
            cast_shadows: true,
            warm: false,
            changed: Vec::new(),
            rows: Vec::new(),
        }
    }
}

impl LampDraw {
    /// Whether reused slots take fresh storage (the viewport draws motion vectors: a new batch
    /// node's `MultiMesh` had no previous upload either).
    pub(crate) fn set_fresh(&mut self, fresh: bool) {
        self.pool.set_fresh(fresh);
    }

    /// Sets each kind's mesh bounds per level (`Mesh.get_aabb`, the bounds the engine's batches
    /// take; None for a kind without models), before any chunk: the cells' centres follow
    /// from them.
    pub(crate) fn set_mesh_bounds(&mut self, meshes: [[Option<Aabb>; LODS]; KINDS]) {
        self.meshes = meshes;
    }

    /// Starts chunk `key`'s slots at its frame `origin`, replacing what it drew (released at
    /// once, as the old root was freed). Nothing draws until [`LampDraw::build_part`] made them
    /// all. A chunk without batches only releases. `O(lamps)` (its cells' centres).
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
                // Every slot made: the next part draws them.
                let next = prebuilt.poles.kinds().len();
                self.building.insert(
                    key,
                    Building {
                        origin,
                        poles: prebuilt.poles,
                        glow: Vec::new(),
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
        let poles = ChunkPoles::new(&self.meshes, &LEVELS, origin, &batches);
        let slots = Vec::with_capacity(poles.kinds().len() * LODS + 1);
        self.building.insert(
            key,
            Building {
                origin,
                poles,
                glow: batches.glow,
                next: 0,
                slots,
            },
        );
    }

    /// Makes one spare slot of every mesh in its smallest size class, written and drawn hidden
    /// with the settings its batches take, so the engine's first-use costs of each mesh (its
    /// buffers, its instance's surfaces and pipelines) are paid here, in the load call, and not
    /// by the first batch of that mesh in a load frame (4-9 ms lights parts at 30k, `RENDER-52`
    /// part 2). Those batches take the spares. `O(meshes)`; once a draw.
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
                if let Some(slot) = self.lod_slot(backend, kind, lod, 1, [0.0; 3]) {
                    self.pool
                        .fill(backend, slot, &IDENTITY[..STRIDE], 1, Aabb::default());
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

    /// Builds a world's lamp slots ahead of its load (`RENDER-52` part 2, round CB3): frees
    /// what the previous world left (its prebuilt chunks and spares), makes the prewarm's
    /// spares again, then makes every slot of `chunks` (key, frame origin, batches) hidden, the
    /// glow written. A slot's creation and a buffer's upload now and then stall the main thread
    /// for 1-3 ms (the engine's allocations and staging, not predictable from the call); done
    /// here, in the load call, a load frame's lights part only draws a chunk whose
    /// [`LampDraw::begin`] brings the same batches, and its first selection writes the level
    /// batches. Returns the slots made. `O(lamps + slots + slots freed)`.
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
            let poles = ChunkPoles::new(&self.meshes, &LEVELS, origin, &batches);
            let want = poles.kinds().len() * LODS + 1;
            let mut slots = Vec::with_capacity(want);
            for (slot, &kind) in poles.kinds().iter().enumerate() {
                for lod in 0..LODS {
                    let count = poles.poles(slot);
                    slots.extend(self.lod_slot(backend, u32::from(kind), lod, count, origin));
                }
            }
            slots.extend(self.glow_batch(backend, &batches.glow, origin));
            if slots.len() != want {
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
                    poles,
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

    /// Makes chunk `key`'s next pole kinds' slots until `budget_us` has passed (one kind at
    /// least), then its glow, and draws them all: the glow at once, the level batches from the
    /// next [`LampDraw::select`]. True when none is left (also for a chunk not building). False,
    /// leaving the rest for later, when the pool refuses a slot.
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
        let count = building.poles.kinds().len();
        while building.next < count {
            let slot = building.next;
            let kind = u32::from(building.poles.kinds()[slot]);
            let poles = building.poles.poles(slot);
            // A kind's levels are made together: a refusal undoes the kind's slots made.
            let made = building.slots.len();
            for lod in 0..LODS {
                let Some(id) = self.lod_slot(backend, kind, lod, poles, building.origin) else {
                    for id in building.slots.drain(made..) {
                        self.pool.release(backend, id);
                    }
                    self.building.insert(key, building);
                    return false;
                };
                building.slots.push(id);
            }
            building.next = slot + 1;
            if start.elapsed().as_secs_f64() * 1e6 >= budget_us && building.next < count {
                self.building.insert(key, building);
                return false;
            }
        }
        // Every chunk with poles has a glow quad a head (a chunk whose lights all lack their
        // model has no batch and never builds).
        if !building.glow.is_empty() {
            let Some(slot) = self.glow_batch(backend, &building.glow, building.origin) else {
                self.building.insert(key, building);
                return false;
            };
            building.slots.push(slot);
            building.glow = Vec::new();
        }
        if let Some(&glow) = building.slots.get(count * LODS) {
            self.pool.set_visible(backend, glow, Role::Main, true);
        }
        self.drawn.insert(key, building.slots);
        self.poles.insert(key, building.poles);
        true
    }

    /// A drawn frame from the main camera at `eye` (just before Godot draws it), on the road
    /// chunk grid of `span_m` and `origin`: fills and shows, or hides, every level batch whose
    /// cells changed level ([`PoleLods::select`]). Returns the batches written or hidden.
    /// Nothing while the camera and the chunks hold still; `O(cells evaluated + poles of the
    /// changed batches)`, no allocation once the scratch rows reached their size.
    pub(crate) fn select(
        &mut self,
        backend: &mut impl RenderBackend,
        eye: [f32; 3],
        span_m: f32,
        origin: [f32; 2],
    ) -> u32 {
        self.changed.clear();
        self.changed
            .extend_from_slice(self.poles.select(eye, span_m, origin));
        let mut filled = 0;
        for triple in self.changed.as_chunks::<3>().0 {
            let key = (triple[0], triple[1]);
            let (Some(slots), Some(chunk)) = (self.drawn.get(&key), self.poles.chunk(key)) else {
                continue;
            };
            let mut mask = triple[2] as u32;
            while mask != 0 {
                let batch = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let id = slots[batch];
                chunk.batch(batch / LODS, batch % LODS, &mut self.rows);
                let count = (self.rows.len() / STRIDE) as u32;
                if count > 0 {
                    self.pool
                        .refill(backend, id, &self.rows, count, Aabb::default());
                }
                self.pool.set_visible(backend, id, Role::Main, count > 0);
                filled += 1;
            }
        }
        filled
    }

    // One level of a pole kind's batch with room for `poles`, drawn hidden and not written.
    fn lod_slot(
        &mut self,
        backend: &mut impl RenderBackend,
        kind: u32,
        lod: usize,
        poles: usize,
        origin: [f32; 3],
    ) -> Option<SlotId> {
        let slot = self.pool.acquire(
            backend,
            slot_kind(kind * LODS as u32 + lod as u32, poles as u32, false),
        )?;
        let fresh = self.pool.slot(slot).drawn(Role::Main).is_none();
        let caster = lod < 2;
        self.pool.draw(
            backend,
            slot,
            Role::Main,
            Drawn {
                origin,
                aabb: Aabb::default(),
                range: [0.0; 2],
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
            // The margin a slot of this mesh always has (a slot keeps its mesh, so its level).
            backend.instance_set_extra_margin(instance, LEVELS[lod].cull_margin);
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

    /// Releases chunk `key`'s slots, drawn or being made (hidden at once). `O(chunk slots)`.
    pub(crate) fn remove(&mut self, backend: &mut impl RenderBackend, key: (i32, i32)) {
        self.poles.remove(key);
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
        self.poles.clear();
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
        self.poles.clear();
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

    /// Chunk `key`'s shown instances: the level batches' and the glow's, as last sent (`(0, 0)`
    /// while it is not drawn). `O(chunk slots)`.
    pub(crate) fn drawn_instances(&self, key: (i32, i32)) -> (u32, u32) {
        let mut counts = (0, 0);
        for &slot in self.drawn.get(&key).into_iter().flatten() {
            let slot = self.pool.slot(slot);
            if !slot.drawn(Role::Main).is_some_and(|drawn| drawn.visible) {
                continue;
            }
            let count = slot.visible_count().unwrap_or(0);
            if slot.kind().mesh == GLOW_MESH {
                counts.1 += count;
            } else {
                counts.0 += count;
            }
        }
        counts
    }

    /// The transforms (12 floats a pole) chunk `key`'s batch of pole kind slot `slot` and level
    /// `lod` holds now (tests).
    pub(crate) fn pole_batch(&self, key: (i32, i32), slot: usize, lod: usize, out: &mut Vec<f32>) {
        out.clear();
        if let Some(chunk) = self.poles.chunk(key) {
            chunk.batch(slot, lod, out);
        }
    }

    /// Drawn chunk `key`'s pole kind of each slot and its cells' centres, `LODS` a cell in cell
    /// order (tests: the engine's bounds of the cell batches).
    pub(crate) fn pole_cells(&self, key: (i32, i32)) -> (Vec<u8>, Vec<[f32; 3]>) {
        self.poles.chunk(key).map_or_else(Default::default, |chunk| {
            (
                chunk.kinds().to_vec(),
                chunk.centres().flatten().copied().collect(),
            )
        })
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
