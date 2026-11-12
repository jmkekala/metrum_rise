// SPDX-License-Identifier: GPL-2.0-only

//! The render backend (`RENDER-32-E`, `docs/terrain.md` "Streaming core"): the only code of the
//! streaming core that calls `RenderingServer`, behind [`RenderBackend`], and the pools of
//! `MultiMesh` and instance objects the layers draw with.
//!
//! A layer draws a batch through a [`Slot`]: one `MultiMesh`, its instance in the main scenario
//! and, lazily, a shadow caster instance and a water reflection instance on the same
//! `MultiMesh`. Slots are pooled by [`SlotKind`] (mesh, capacity class, buffer format and the
//! canopy's instance parameter), so an install writes a buffer into an object that exists and a
//! removal is a visibility flag. A slot's capacity is a size class: the buffer is padded with
//! copies of its first instance, which leaves the engine's bounds of the `MultiMesh` bit for bit
//! what the exact count gives (a merge with a box already merged is the same box), and only the
//! batch's own instances draw (`multimesh_set_visible_instances`). A slot keeps its mesh, so a
//! reused slot never pays the engine's buffer read-back that a mesh change on a written buffer
//! costs.
//!
//! The pool is bounded at setup: at most `limit` slots exist, live and spare together (every
//! slot holds instance uniform slots of the engine's global buffer, 65,536 in all), at most
//! `spare_limit` of them spare. Over the limit the oldest spare is freed for a new kind, and when
//! no spare is left [`SlotPool::acquire`] refuses: the layer keeps the work pending.
//!
//! [`GodotBackend`] is the `RenderingServer` implementation; `Recorder` (tests) records every
//! command and keeps the state they leave, so the layers are tested without Godot. Everything
//! runs on Godot's main thread.

use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use std::collections::{HashMap, VecDeque};

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// A backend object: a `RenderingServer` RID's id, or a recorder's number.
pub(crate) type Handle = u64;

/// What an instance does in the shadow pass (`RenderingServer.ShadowCastingSetting`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Shadow {
    /// Draws, casts nothing.
    Off,
    /// Draws and casts.
    On,
    /// Casts only.
    ShadowsOnly,
}

/// The scenario an instance draws in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Scenario {
    /// The main world.
    Main,
    /// The water reflection's world (`water_reflection.gd`).
    Reflection,
}

/// An instance shader parameter's value beyond a float (`RENDER-32-K`: the graded cells'
/// `cell_patch_rect_hm` and `cell_lattice_anchor_hm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Vector {
    /// An `ivec2`.
    Int2([i32; 2]),
    /// An `ivec4`.
    Int4([i32; 4]),
}

/// The `RenderingServer` calls the streaming layers need. Meshes, materials and instance shader
/// parameters are indices into what the backend was given at registration.
pub(crate) trait RenderBackend {
    /// A `MultiMesh` of `capacity` 3D transforms, with or without colour and custom lanes.
    fn multimesh_create(&mut self, capacity: u32, colors: bool, custom: bool) -> Handle;
    /// Gives a `MultiMesh` fresh storage of the same shape, so the engine's previous-frame half
    /// (motion vectors) holds nothing of the slot's last batch.
    fn multimesh_reallocate(
        &mut self,
        multimesh: Handle,
        capacity: u32,
        colors: bool,
        custom: bool,
    );
    /// Sets a `MultiMesh`'s mesh.
    fn multimesh_set_mesh(&mut self, multimesh: Handle, mesh: u32);
    /// Writes `count` instances of `stride` floats from `rows`, padded to `capacity` with copies
    /// of the first.
    fn multimesh_set_buffer(
        &mut self,
        multimesh: Handle,
        rows: &[f32],
        stride: usize,
        capacity: u32,
    );
    /// Draws the first `count` instances.
    fn multimesh_set_visible_instances(&mut self, multimesh: Handle, count: u32);
    /// Sets the `MultiMesh`'s own bounds (zero size: the engine computes them).
    fn multimesh_set_custom_aabb(&mut self, multimesh: Handle, aabb: Aabb);
    /// An instance of `base` in `scenario`; `None` while the scenario is not registered.
    fn instance_create(&mut self, base: Handle, scenario: Scenario) -> Option<Handle>;
    /// Moves an instance to `origin` (no rotation or scale).
    fn instance_set_origin(&mut self, instance: Handle, origin: [f32; 3]);
    /// Shows or hides an instance.
    fn instance_set_visible(&mut self, instance: Handle, visible: bool);
    /// Sets an instance's shadow casting.
    fn instance_set_shadow(&mut self, instance: Handle, shadow: Shadow);
    /// Sets an instance's visibility range, metres, without fade or margins.
    fn instance_set_range(&mut self, instance: Handle, range: [f32; 2]);
    /// Sets an instance's custom bounds (zero size: none).
    fn instance_set_custom_aabb(&mut self, instance: Handle, aabb: Aabb);
    /// Sets an instance's material override (`None`: the mesh's own).
    fn instance_set_material(&mut self, instance: Handle, material: Option<u32>);
    /// Sets an instance shader parameter to a float.
    fn instance_set_parameter(&mut self, instance: Handle, parameter: u32, value: f32);
    /// Frees an object.
    fn free(&mut self, handle: Handle);
    // The graded cells' calls (`RENDER-32-K`). A backend that serves no layer with materials of
    // its own (a test's tally) keeps the defaults: no material, nothing sent.
    /// A material of `shader` (a handle the layer was given) with no parameter set, owned by
    /// the layer that made it.
    fn material_create(&mut self, _shader: Handle) -> Handle {
        0
    }
    /// Sets (or clears, `None`) a material's texture parameter (a catalogue parameter index) to
    /// a texture handle.
    fn material_set_texture(&mut self, _material: Handle, _parameter: u32, _texture: Option<Handle>) {}
    /// Sets an instance's material override to a material handle (`None`: the mesh's own), for
    /// materials that are not catalogue entries (a layer's own, or a resource's handed over).
    fn instance_set_material_handle(&mut self, _instance: Handle, _material: Option<Handle>) {}
    /// Sets an instance shader parameter (a catalogue index) to an integer vector.
    fn instance_set_vector(&mut self, _instance: Handle, _parameter: u32, _value: Vector) {}
    // The graded pages' contents (`RENDER-32-K` part 2). A page's layered textures are
    // resources made by GDScript and handed over by handle; their layers are written here. A
    // backend that serves no such layer keeps the defaults: nothing is written.
    /// Writes layer `layer` of the RGBAF layered texture `texture`, whose layers are `size`
    /// texels wide and high (`size[0] * size[1] * 16` bytes): `fill` writes those bytes into
    /// the backend's upload array, which is reused across calls.
    fn texture_write_layer(
        &mut self,
        _texture: Handle,
        _layer: u32,
        _size: [u32; 2],
        _fill: &mut dyn FnMut(&mut [u8]),
    ) {
    }
    /// Copies layers `0..layers` of `from` into the same layers of `to`, both RGBAF layered
    /// textures of `size` texels a layer.
    fn texture_copy_layers(&mut self, _from: Handle, _to: Handle, _size: [u32; 2], _layers: u32) {}
    /// Drops what the backend keeps of `texture`'s written layers (a CPU copy where the
    /// renderer cannot read a texture back): the texture holds nothing the caller needs.
    fn texture_forget(&mut self, _texture: Handle) {}
}

/// Where a layer's entries start in a shared catalogue ([`GodotBackend::add_catalogue`]): its
/// mesh, material and parameter indices are these plus its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CatalogueRange {
    /// The layer's first mesh index.
    pub(crate) mesh: u32,
    /// The layer's first material index.
    pub(crate) material: u32,
    /// The layer's first parameter index.
    pub(crate) parameter: u32,
}

/// Smallest pooled capacity.
const MIN_CAPACITY: u32 = 16;

/// The capacity class of `count` instances: the smallest of 16, 24, 32, 48, 64, ... (powers of
/// two and one and a half times them) that holds them, so a padded buffer is at most a third
/// larger than the batch past the smallest class.
pub(crate) fn capacity_class(count: u32) -> (u8, u32) {
    let mut class = 0u8;
    let mut capacity = MIN_CAPACITY;
    while capacity < count {
        class += 1;
        capacity = class_capacity(class);
    }
    (class, capacity)
}

fn class_capacity(class: u8) -> u32 {
    let base = MIN_CAPACITY << (class / 2);
    if class.is_multiple_of(2) {
        base
    } else {
        base + base / 2
    }
}

/// What a slot can be reused for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SlotKind {
    /// The mesh index.
    pub(crate) mesh: u32,
    /// The capacity class ([`capacity_class`]).
    pub(crate) class: u8,
    /// Colour lane.
    pub(crate) colors: bool,
    /// Custom lane.
    pub(crate) custom: bool,
    /// The instance shader parameter set on the main instance at creation, if any (index and
    /// value), which a reuse must not lose or inherit.
    pub(crate) parameter: Option<(u32, u32)>,
}

impl SlotKind {
    /// Floats per instance.
    pub(crate) fn stride(&self) -> usize {
        12 + 4 * usize::from(self.colors) + 4 * usize::from(self.custom)
    }
}

/// The instances of a slot, all on its `MultiMesh`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// The batch drawn in the main scenario.
    Main = 0,
    /// A second main-scenario instance (a shadow caster sharing the buffer), made on first use.
    Caster = 1,
    /// The batch in the water reflection's scenario, made on first use.
    Reflection = 2,
}

/// The state an instance is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drawn {
    /// Where the instance stands.
    pub(crate) origin: [f32; 3],
    /// The instance's custom bounds, or zero size.
    pub(crate) aabb: Aabb,
    /// Visibility range.
    pub(crate) range: [f32; 2],
    /// Material override.
    pub(crate) material: Option<u32>,
    /// Shadow casting.
    pub(crate) shadow: Shadow,
    /// Shown.
    pub(crate) visible: bool,
}

/// One pooled `MultiMesh` with its instances.
#[derive(Debug)]
pub(crate) struct Slot {
    /// The `MultiMesh`.
    pub(crate) multimesh: Handle,
    // By role; the main instance always exists.
    instances: [Option<Handle>; 3],
    // What the backend was last told of each instance, so an unchanged value is not sent again.
    drawn: [Option<Drawn>; 3],
    kind: SlotKind,
    capacity: u32,
    spare: bool,
    // The `MultiMesh`'s visible instance count and own bounds as last sent (`None`: unknown).
    count: Option<u32>,
    multimesh_aabb: Option<Aabb>,
}

impl Slot {
    /// The kind the slot is pooled as (its current mesh).
    pub(crate) fn kind(&self) -> SlotKind {
        self.kind
    }

    /// An instance of the slot, if made.
    #[cfg(test)]
    pub(crate) fn instance(&self, role: Role) -> Option<Handle> {
        self.instances[role as usize]
    }

    /// What an instance was last drawn with.
    pub(crate) fn drawn(&self, role: Role) -> Option<Drawn> {
        self.drawn[role as usize]
    }

    /// The `MultiMesh`'s own bounds as last set (zero size: the engine's).
    pub(crate) fn multimesh_aabb(&self) -> Aabb {
        self.multimesh_aabb.unwrap_or_default()
    }
}

/// Index of a slot in its pool.
pub(crate) type SlotId = u32;

/// Pooled slots, bounded at setup.
pub(crate) struct SlotPool {
    slots: Vec<Option<Slot>>,
    vacant: Vec<SlotId>,
    spare: Map<SlotKind, Vec<SlotId>>,
    // Spares oldest first, for eviction; a slot taken back out stays here until met.
    spare_order: VecDeque<SlotId>,
    live: usize,
    spares: usize,
    limit: usize,
    spare_limit: usize,
    // Reused slots get fresh storage (motion vectors on).
    fresh: bool,
    created: u64,
    freed: u64,
}

impl SlotPool {
    /// A pool of at most `limit` slots, at most `spare_limit` of them spare.
    pub(crate) fn new(limit: usize, spare_limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            vacant: Vec::new(),
            spare: Map::default(),
            spare_order: VecDeque::new(),
            live: 0,
            spares: 0,
            limit,
            spare_limit: spare_limit.min(limit),
            fresh: false,
            created: 0,
            freed: 0,
        }
    }

    /// Whether a reused slot gets fresh storage, for a viewport that draws motion vectors (FSR 2,
    /// TAA): the engine keeps the previous upload as the previous frame's transforms.
    pub(crate) fn set_fresh(&mut self, fresh: bool) {
        self.fresh = fresh;
    }

    /// Slots in use.
    pub(crate) fn live(&self) -> usize {
        self.live
    }

    /// Spare slots.
    pub(crate) fn spares(&self) -> usize {
        self.spares
    }

    /// Slots that can still be acquired: every spare can be reused or evicted.
    pub(crate) fn available(&self) -> usize {
        self.limit - self.live
    }

    /// Slots created and freed since the pool was made.
    pub(crate) fn churn(&self) -> (u64, u64) {
        (self.created, self.freed)
    }

    /// The capacities of the pool's storage, which a steady frame must leave unchanged (tests).
    #[cfg(test)]
    pub(crate) fn footprint(&self) -> Vec<usize> {
        let mut out = vec![
            self.slots.capacity(),
            self.vacant.capacity(),
            self.spare.capacity(),
            self.spare_order.capacity(),
        ];
        let mut lists: Vec<_> = self
            .spare
            .iter()
            .map(|(kind, list)| (kind.mesh, kind.class, list.capacity()))
            .collect();
        lists.sort_unstable();
        out.extend(lists.into_iter().map(|(_, _, capacity)| capacity));
        out
    }

    /// A live slot.
    pub(crate) fn slot(&self, id: SlotId) -> &Slot {
        self.slots[id as usize].as_ref().expect("live slot")
    }

    fn slot_mut(&mut self, id: SlotId) -> &mut Slot {
        self.slots[id as usize].as_mut().expect("live slot")
    }

    /// A slot of `kind`: a spare of that kind, a new one, or a new one in place of the oldest
    /// spare. `None` when every slot of the limit is live. `O(1)` amortised.
    pub(crate) fn acquire(
        &mut self,
        backend: &mut impl RenderBackend,
        kind: SlotKind,
    ) -> Option<SlotId> {
        if let Some(id) = self.spare.get_mut(&kind).and_then(Vec::pop) {
            self.spares -= 1;
            self.live += 1;
            let fresh = self.fresh;
            let slot = self.slot_mut(id);
            slot.spare = false;
            if fresh {
                backend.multimesh_reallocate(
                    slot.multimesh,
                    slot.capacity,
                    kind.colors,
                    kind.custom,
                );
                slot.count = None;
                slot.multimesh_aabb = None;
            }
            return Some(id);
        }
        if self.live + self.spares >= self.limit {
            if self.spares == 0 {
                return None;
            }
            self.evict_oldest_spare(backend);
        }
        Some(self.create(backend, kind))
    }

    fn create(&mut self, backend: &mut impl RenderBackend, kind: SlotKind) -> SlotId {
        let capacity = class_capacity(kind.class);
        let multimesh = backend.multimesh_create(capacity, kind.colors, kind.custom);
        // The mesh before any buffer: a mesh set on a written buffer reads it back.
        backend.multimesh_set_mesh(multimesh, kind.mesh);
        let instance = backend
            .instance_create(multimesh, Scenario::Main)
            .expect("the main scenario is registered before any install");
        backend.instance_set_visible(instance, false);
        if let Some((parameter, value)) = kind.parameter {
            backend.instance_set_parameter(instance, parameter, f32::from_bits(value));
        }
        let slot = Slot {
            multimesh,
            instances: [Some(instance), None, None],
            drawn: [None; 3],
            kind,
            capacity,
            spare: false,
            // A new `MultiMesh` draws all its instances, without bounds of its own.
            count: Some(capacity),
            multimesh_aabb: Some(Aabb::default()),
        };
        self.live += 1;
        self.created += 1;
        if let Some(id) = self.vacant.pop() {
            self.slots[id as usize] = Some(slot);
            id
        } else {
            self.slots.push(Some(slot));
            (self.slots.len() - 1) as SlotId
        }
    }

    fn evict_oldest_spare(&mut self, backend: &mut impl RenderBackend) {
        while let Some(id) = self.spare_order.pop_front() {
            let Some(slot) = self.slots[id as usize].as_ref() else {
                continue;
            };
            if !slot.spare {
                continue;
            }
            let kind = slot.kind;
            if let Some(list) = self.spare.get_mut(&kind)
                && let Some(at) = list.iter().position(|&spare| spare == id)
            {
                list.swap_remove(at);
            }
            self.spares -= 1;
            self.free_slot(backend, id);
            return;
        }
    }

    fn free_slot(&mut self, backend: &mut impl RenderBackend, id: SlotId) {
        let slot = self.slots[id as usize].take().expect("slot to free");
        for handle in slot.instances.into_iter().flatten() {
            backend.free(handle);
        }
        backend.free(slot.multimesh);
        self.vacant.push(id);
        self.freed += 1;
    }

    /// Returns a live slot: hidden at once, then kept as a spare or freed past the spare limit.
    /// `O(1)` amortised.
    pub(crate) fn release(&mut self, backend: &mut impl RenderBackend, id: SlotId) {
        self.live -= 1;
        let full = self.spares >= self.spare_limit;
        let slot = self.slot_mut(id);
        for role in [Role::Main, Role::Caster, Role::Reflection] {
            if let (Some(handle), Some(drawn)) = (
                slot.instances[role as usize],
                slot.drawn[role as usize].as_mut(),
            ) && drawn.visible
            {
                drawn.visible = false;
                backend.instance_set_visible(handle, false);
            }
        }
        if full {
            self.free_slot(backend, id);
            return;
        }
        slot.spare = true;
        let kind = slot.kind;
        self.spare.entry(kind).or_default().push(id);
        self.spare_order.push_back(id);
        self.spares += 1;
        // Stale entries (slots taken back out) are dropped once they outnumber the spares.
        if self.spare_order.len() > 2 * self.spares.max(16) {
            let slots = &self.slots;
            self.spare_order
                .retain(|&id| slots[id as usize].as_ref().is_some_and(|slot| slot.spare));
        }
    }

    /// Writes `count` instances of `rows` into a live slot's `MultiMesh`, with its own bounds
    /// (zero size: the engine's). The bounds go before the buffer: a `MultiMesh` with custom
    /// bounds skips the engine's pass over the instances.
    pub(crate) fn fill(
        &mut self,
        backend: &mut impl RenderBackend,
        id: SlotId,
        rows: &[f32],
        count: u32,
        multimesh_aabb: Aabb,
    ) {
        let slot = self.slot_mut(id);
        if slot.multimesh_aabb != Some(multimesh_aabb) {
            backend.multimesh_set_custom_aabb(slot.multimesh, multimesh_aabb);
            slot.multimesh_aabb = Some(multimesh_aabb);
        }
        backend.multimesh_set_buffer(slot.multimesh, rows, slot.kind.stride(), slot.capacity);
        if slot.count != Some(count) {
            backend.multimesh_set_visible_instances(slot.multimesh, count);
            slot.count = Some(count);
        }
    }

    /// Draws one of a live slot's instances as `drawn`, sending only what changed; the caster
    /// and reflection instances are made on first use. Returns false when the role's scenario
    /// is not registered (no reflection world).
    pub(crate) fn draw(
        &mut self,
        backend: &mut impl RenderBackend,
        id: SlotId,
        role: Role,
        drawn: Drawn,
    ) -> bool {
        let slot = self.slot_mut(id);
        let at = role as usize;
        let instance = match slot.instances[at] {
            Some(instance) => instance,
            None => {
                let scenario = if role == Role::Reflection {
                    Scenario::Reflection
                } else {
                    Scenario::Main
                };
                let Some(instance) = backend.instance_create(slot.multimesh, scenario) else {
                    return false;
                };
                slot.instances[at] = Some(instance);
                instance
            }
        };
        let last = slot.drawn[at];
        if last.is_none_or(|last| last.origin != drawn.origin) {
            backend.instance_set_origin(instance, drawn.origin);
        }
        apply_instance(backend, instance, last, &drawn);
        slot.drawn[at] = Some(drawn);
        true
    }

    /// Changes a live slot's mesh (the engine reads its buffer back to rebound it); the slot is
    /// pooled under the new mesh from then on.
    pub(crate) fn set_mesh(&mut self, backend: &mut impl RenderBackend, id: SlotId, mesh: u32) {
        let slot = self.slot_mut(id);
        if slot.kind.mesh != mesh {
            slot.kind.mesh = mesh;
            backend.multimesh_set_mesh(slot.multimesh, mesh);
        }
    }

    /// Changes one instance's shadow casting and visibility, if drawn.
    pub(crate) fn set_shadow(
        &mut self,
        backend: &mut impl RenderBackend,
        id: SlotId,
        role: Role,
        shadow: Shadow,
        visible: bool,
    ) {
        let slot = self.slot_mut(id);
        let at = role as usize;
        if let (Some(instance), Some(drawn)) = (slot.instances[at], slot.drawn[at].as_mut()) {
            if drawn.shadow != shadow {
                drawn.shadow = shadow;
                backend.instance_set_shadow(instance, shadow);
            }
            if drawn.visible != visible {
                drawn.visible = visible;
                backend.instance_set_visible(instance, visible);
            }
        }
    }

    /// Shows or hides one instance, if drawn.
    pub(crate) fn set_visible(
        &mut self,
        backend: &mut impl RenderBackend,
        id: SlotId,
        role: Role,
        visible: bool,
    ) {
        let slot = self.slot_mut(id);
        let at = role as usize;
        if let (Some(instance), Some(drawn)) = (slot.instances[at], slot.drawn[at].as_mut())
            && drawn.visible != visible
        {
            drawn.visible = visible;
            backend.instance_set_visible(instance, visible);
        }
    }

    /// Drops every slot's reflection instance (the reflection's world changed).
    pub(crate) fn drop_reflections(&mut self, backend: &mut impl RenderBackend) {
        for slot in self.slots.iter_mut().flatten() {
            let at = Role::Reflection as usize;
            if let Some(reflection) = slot.instances[at].take() {
                backend.free(reflection);
            }
            slot.drawn[at] = None;
        }
    }

    /// Frees every slot (a renderer reset or the end of the backend).
    pub(crate) fn clear(&mut self, backend: &mut impl RenderBackend) {
        for id in 0..self.slots.len() {
            if self.slots[id].is_some() {
                self.free_slot(backend, id as SlotId);
            }
        }
        self.slots.clear();
        self.vacant.clear();
        self.spare.clear();
        self.spare_order.clear();
        self.live = 0;
        self.spares = 0;
    }
}

/// Sends what changed of an instance's drawn state (everything without a last state), the
/// origin excepted.
fn apply_instance(
    backend: &mut impl RenderBackend,
    instance: Handle,
    last: Option<Drawn>,
    drawn: &Drawn,
) {
    let changed = |field: fn(&Drawn, &Drawn) -> bool| last.is_none_or(|last| field(&last, drawn));
    if changed(|last, drawn| last.aabb != drawn.aabb) {
        backend.instance_set_custom_aabb(instance, drawn.aabb);
    }
    if changed(|last, drawn| last.range != drawn.range) {
        backend.instance_set_range(instance, drawn.range);
    }
    if changed(|last, drawn| last.material != drawn.material) {
        backend.instance_set_material(instance, drawn.material);
    }
    if changed(|last, drawn| last.shadow != drawn.shadow) {
        backend.instance_set_shadow(instance, drawn.shadow);
    }
    if changed(|last, drawn| last.visible != drawn.visible) {
        backend.instance_set_visible(instance, drawn.visible);
    }
}

/// Copies `rows` to the front of `out` and fills the rest with copies of the first instance
/// (`stride` floats), which leave the engine's bounds of the buffer as the rows alone give them.
pub(crate) fn pad_rows(out: &mut [f32], rows: &[f32], stride: usize) {
    out[..rows.len()].copy_from_slice(rows);
    for pad in out[rows.len()..].chunks_exact_mut(stride) {
        pad.copy_from_slice(&rows[..stride]);
    }
}

pub(crate) use godot_backend::GodotBackend;

mod godot_backend {
    use super::*;
    use godot::classes::image::Format;
    use godot::classes::rendering_device::TextureUsageBits;
    use godot::classes::{DisplayServer, Image, RenderingDevice, RenderingServer};
    use godot::classes::rendering_server::{
        MultimeshTransformFormat, ShadowCastingSetting, VisibilityRangeFadeMode,
    };
    use godot::obj::{EngineBitfield, Singleton};
    use godot::prelude::{
        Basis, Gd, PackedByteArray, PackedFloat32Array, Rid, StringName, ToGodot, Transform3D,
        Variant, Vector2i, Vector3, Vector4i,
    };

    /// The `RenderingServer` backend. Holds the meshes, materials and parameter names the layers
    /// index, the two scenarios, and one padded upload array per buffer length, so a steady
    /// stream of installs allocates nothing once each length has been met.
    pub(crate) struct GodotBackend {
        server: Gd<RenderingServer>,
        meshes: Vec<Rid>,
        materials: Vec<Rid>,
        parameters: Vec<StringName>,
        scenarios: [Option<Rid>; 2],
        scratch: Map<usize, PackedFloat32Array>,
        // Microseconds spent copying rows into the upload arrays since the last take.
        copy_us: f64,
        // The texture layer writes (`RENDER-32-K` part 2): one reused upload array; the
        // `RenderingDevice` where this thread may call it (Forward+ and Mobile, Godot's default
        // single-threaded rendering), else the `RenderingServer` through one reused `Image` per
        // layer size; on a renderer without device readback but with a display (Compatibility,
        // `TERRAIN-22`) a CPU copy of every written layer, which a page's growth copies.
        upload: PackedByteArray,
        device: Option<Gd<RenderingDevice>>,
        images: Map<[u32; 2], Gd<Image>>,
        mirrored: bool,
        mirror: Map<Handle, Map<u32, PackedByteArray>>,
        mirror_bytes: i64,
    }

    impl GodotBackend {
        /// A backend drawing in `scenario`.
        pub(crate) fn new(scenario: Rid) -> Self {
            let mut server = RenderingServer::singleton();
            let device = server
                .is_on_render_thread()
                .then(|| server.get_rendering_device())
                .flatten();
            let mirrored = server.get_rendering_device().is_none()
                && DisplayServer::singleton().get_name().to_string() != "headless";
            Self {
                server,
                meshes: Vec::new(),
                materials: Vec::new(),
                parameters: Vec::new(),
                scenarios: [Some(scenario), None],
                scratch: Map::default(),
                copy_us: 0.0,
                upload: PackedByteArray::new(),
                device,
                images: Map::default(),
                mirrored,
                mirror: Map::default(),
                mirror_bytes: 0,
            }
        }

        /// The bytes of the written layers' CPU copies.
        pub(crate) fn mirror_bytes(&self) -> i64 {
            self.mirror_bytes
        }

        /// The CPU copy of `texture`'s layer `layer` (empty: never written since the texture
        /// was made or forgotten).
        pub(crate) fn mirror_layer(&self, texture: Handle, layer: u32) -> PackedByteArray {
            self.mirror
                .get(&texture)
                .and_then(|layers| layers.get(&layer))
                .cloned()
                .unwrap_or_default()
        }

        // Keeps `data` as the CPU copy of `texture`'s layer `layer` (shared, not copied).
        fn keep(&mut self, texture: Handle, layer: u32, data: PackedByteArray) {
            self.mirror_bytes += data.len() as i64;
            if let Some(old) = self.mirror.entry(texture).or_default().insert(layer, data) {
                self.mirror_bytes -= old.len() as i64;
            }
        }

        // Writes `data` into layer `layer` of `texture` through the `RenderingServer` (no
        // device on this thread), from the size's reused `Image`.
        fn server_write(&mut self, texture: Handle, layer: u32, size: [u32; 2], data: &PackedByteArray) {
            let (width, height) = (size[0] as i32, size[1] as i32);
            let image = self.images.entry(size).or_insert_with(|| {
                Image::create_empty(width, height, false, Format::RGBAF).expect("an RGBAF image")
            });
            image.set_data(width, height, false, Format::RGBAF, data);
            self.server
                .texture_2d_update(Rid::new(texture), &*image, layer as i32);
        }

        /// Registers the meshes, materials and instance parameter names by index.
        pub(crate) fn set_catalogue(
            &mut self,
            meshes: Vec<Rid>,
            materials: Vec<Rid>,
            parameters: Vec<StringName>,
        ) {
            self.meshes = meshes;
            self.materials = materials;
            self.parameters = parameters;
        }

        /// Appends a layer's meshes, materials and instance parameter names to the catalogue
        /// and returns where they start, so layers sharing one backend index their own ranges.
        pub(crate) fn add_catalogue(
            &mut self,
            meshes: Vec<Rid>,
            materials: Vec<Rid>,
            parameters: Vec<StringName>,
        ) -> CatalogueRange {
            let range = CatalogueRange {
                mesh: self.meshes.len() as u32,
                material: self.materials.len() as u32,
                parameter: self.parameters.len() as u32,
            };
            self.meshes.extend(meshes);
            self.materials.extend(materials);
            self.parameters.extend(parameters);
            range
        }

        /// Sets or clears the reflection scenario.
        pub(crate) fn set_reflection_scenario(&mut self, scenario: Option<Rid>) {
            self.scenarios[1] = scenario;
        }

        /// The microseconds spent copying rows into upload arrays since the last call.
        pub(crate) fn take_copy_us(&mut self) -> f64 {
            std::mem::take(&mut self.copy_us)
        }
    }

    impl RenderBackend for GodotBackend {
        fn multimesh_create(&mut self, capacity: u32, colors: bool, custom: bool) -> Handle {
            let multimesh = self.server.multimesh_create();
            self.multimesh_reallocate(multimesh.to_u64(), capacity, colors, custom);
            multimesh.to_u64()
        }

        fn multimesh_reallocate(
            &mut self,
            multimesh: Handle,
            capacity: u32,
            colors: bool,
            custom: bool,
        ) {
            let rid = Rid::new(multimesh);
            // The engine skips an allocation of the same shape, so the empty one comes first.
            self.server
                .multimesh_allocate_data(rid, 0, MultimeshTransformFormat::TRANSFORM_3D);
            self.server
                .multimesh_allocate_data_ex(
                    rid,
                    capacity as i32,
                    MultimeshTransformFormat::TRANSFORM_3D,
                )
                .color_format(colors)
                .custom_data_format(custom)
                .done();
        }

        fn multimesh_set_mesh(&mut self, multimesh: Handle, mesh: u32) {
            let mesh = self.meshes[mesh as usize];
            self.server.multimesh_set_mesh(Rid::new(multimesh), mesh);
        }

        fn multimesh_set_buffer(
            &mut self,
            multimesh: Handle,
            rows: &[f32],
            stride: usize,
            capacity: u32,
        ) {
            let start = std::time::Instant::now();
            let length = capacity as usize * stride;
            let buffer = self.scratch.entry(length).or_insert_with(|| {
                let mut buffer = PackedFloat32Array::new();
                buffer.resize(length);
                buffer
            });
            pad_rows(buffer.as_mut_slice(), rows, stride);
            self.copy_us += start.elapsed().as_secs_f64() * 1e6;
            self.server
                .multimesh_set_buffer(Rid::new(multimesh), &*buffer);
        }

        fn multimesh_set_visible_instances(&mut self, multimesh: Handle, count: u32) {
            self.server
                .multimesh_set_visible_instances(Rid::new(multimesh), count as i32);
        }

        fn multimesh_set_custom_aabb(&mut self, multimesh: Handle, aabb: Aabb) {
            self.server
                .multimesh_set_custom_aabb(Rid::new(multimesh), godot_aabb(aabb));
        }

        fn instance_create(&mut self, base: Handle, scenario: Scenario) -> Option<Handle> {
            let scenario = self.scenarios[scenario as usize]?;
            Some(
                self.server
                    .instance_create2(Rid::new(base), scenario)
                    .to_u64(),
            )
        }

        fn instance_set_origin(&mut self, instance: Handle, origin: [f32; 3]) {
            let transform = Transform3D::new(Basis::IDENTITY, Vector3::from_array(origin));
            self.server
                .instance_set_transform(Rid::new(instance), transform);
        }

        fn instance_set_visible(&mut self, instance: Handle, visible: bool) {
            self.server
                .instance_set_visible(Rid::new(instance), visible);
        }

        fn instance_set_shadow(&mut self, instance: Handle, shadow: Shadow) {
            let setting = match shadow {
                Shadow::Off => ShadowCastingSetting::OFF,
                Shadow::On => ShadowCastingSetting::ON,
                Shadow::ShadowsOnly => ShadowCastingSetting::SHADOWS_ONLY,
            };
            self.server
                .instance_geometry_set_cast_shadows_setting(Rid::new(instance), setting);
        }

        fn instance_set_range(&mut self, instance: Handle, range: [f32; 2]) {
            self.server.instance_geometry_set_visibility_range(
                Rid::new(instance),
                range[0],
                range[1],
                0.0,
                0.0,
                VisibilityRangeFadeMode::DISABLED,
            );
        }

        fn instance_set_custom_aabb(&mut self, instance: Handle, aabb: Aabb) {
            self.server
                .instance_set_custom_aabb(Rid::new(instance), godot_aabb(aabb));
        }

        fn instance_set_material(&mut self, instance: Handle, material: Option<u32>) {
            let material = material.map_or(Rid::Invalid, |index| self.materials[index as usize]);
            self.server
                .instance_geometry_set_material_override(Rid::new(instance), material);
        }

        fn instance_set_parameter(&mut self, instance: Handle, parameter: u32, value: f32) {
            let name = &self.parameters[parameter as usize];
            self.server.instance_geometry_set_shader_parameter(
                Rid::new(instance),
                name,
                &value.to_variant(),
            );
        }

        fn free(&mut self, handle: Handle) {
            self.server.free_rid(Rid::new(handle));
        }

        fn material_create(&mut self, shader: Handle) -> Handle {
            let material = self.server.material_create();
            self.server.material_set_shader(material, Rid::new(shader));
            material.to_u64()
        }

        fn material_set_texture(
            &mut self,
            material: Handle,
            parameter: u32,
            texture: Option<Handle>,
        ) {
            let name = &self.parameters[parameter as usize];
            // Nil clears the parameter, as a `ShaderMaterial` given null does.
            let value = texture.map_or_else(Variant::nil, |texture| Rid::new(texture).to_variant());
            self.server
                .material_set_param(Rid::new(material), name, &value);
        }

        fn instance_set_material_handle(&mut self, instance: Handle, material: Option<Handle>) {
            self.server.instance_geometry_set_material_override(
                Rid::new(instance),
                material.map_or(Rid::Invalid, Rid::new),
            );
        }

        fn instance_set_vector(&mut self, instance: Handle, parameter: u32, value: Vector) {
            let name = &self.parameters[parameter as usize];
            let value = match value {
                Vector::Int2([x, y]) => Vector2i::new(x, y).to_variant(),
                Vector::Int4([x, y, z, w]) => Vector4i::new(x, y, z, w).to_variant(),
            };
            self.server
                .instance_geometry_set_shader_parameter(Rid::new(instance), name, &value);
        }

        fn texture_write_layer(
            &mut self,
            texture: Handle,
            layer: u32,
            size: [u32; 2],
            fill: &mut dyn FnMut(&mut [u8]),
        ) {
            let length = size[0] as usize * size[1] as usize * 16;
            if self.upload.len() != length {
                self.upload.resize(length);
            }
            // The array is unshared here (a kept CPU copy is a clone, replaced below), so this
            // writes in place.
            fill(self.upload.as_mut_slice());
            if let Some(device) = self.device.as_mut() {
                // The device copies the bytes into its staging memory before it returns.
                let target = self.server.texture_get_rd_texture(Rid::new(texture));
                device.texture_update(target, layer, &self.upload);
                return;
            }
            let data = std::mem::take(&mut self.upload);
            self.server_write(texture, layer, size, &data);
            if self.mirrored {
                self.keep(texture, layer, data.clone());
            }
            self.upload = data;
        }

        fn texture_copy_layers(&mut self, from: Handle, to: Handle, size: [u32; 2], layers: u32) {
            if let Some(device) = self.device.as_mut() {
                let source = self.server.texture_get_rd_texture(Rid::new(from));
                let target = self.server.texture_get_rd_texture(Rid::new(to));
                // A page's own arrays may be copied into on the device; the engine's layered
                // arrays (a one-layer test page) may not, so those are read back exactly.
                let copyable = device.texture_get_format(target).is_some_and(|format| {
                    format.get_usage_bits().is_set(TextureUsageBits::CAN_COPY_TO_BIT)
                });
                let extent = Vector3::new(size[0] as f32, size[1] as f32, 1.0);
                for layer in 0..layers {
                    if copyable {
                        device.texture_copy(source, target, Vector3::ZERO, Vector3::ZERO, extent, 0, 0, layer, layer);
                    } else {
                        let data = device.texture_get_data(source, layer);
                        device.texture_update(target, layer, &data);
                    }
                }
                return;
            }
            if !self.mirrored {
                // The dummy renderer (headless) stores no texels.
                return;
            }
            // Compatibility: the source's written layers from their CPU copies (`TERRAIN-22`;
            // Godot 4.7.2's GLES3 reads an RGBAF layer back clamped, so nothing is read back).
            let written: Vec<(u32, PackedByteArray)> = self
                .mirror
                .get(&from)
                .map(|copies| {
                    let mut copies: Vec<_> = copies
                        .iter()
                        .filter(|(layer, _)| **layer < layers)
                        .map(|(&layer, data)| (layer, data.clone()))
                        .collect();
                    copies.sort_unstable_by_key(|(layer, _)| *layer);
                    copies
                })
                .unwrap_or_default();
            for (layer, data) in written {
                self.server_write(to, layer, size, &data);
                self.keep(to, layer, data);
            }
        }

        fn texture_forget(&mut self, texture: Handle) {
            if let Some(copies) = self.mirror.remove(&texture) {
                self.mirror_bytes -= copies.values().map(|data| data.len() as i64).sum::<i64>();
            }
        }
    }

    fn godot_aabb(aabb: Aabb) -> godot::builtin::Aabb {
        godot::builtin::Aabb::new(
            Vector3::from_array(aabb.position),
            Vector3::from_array(aabb.size),
        )
    }
}

#[cfg(test)]
pub(crate) mod recorder;

#[cfg(test)]
mod tests;
