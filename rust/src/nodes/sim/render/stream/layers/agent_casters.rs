// SPDX-License-Identifier: GPL-3.0-or-later

//! The moving agents' shadow casters (`RENDER-15`'s `agents_near`, round R47): a shadow-only
//! twin per walker and car bucket, drawn through the backend's pools, holding only the agents
//! that can shade ground the sun's shadow reaches. `agents.gd`'s bucket nodes draw every agent
//! in the main pass and cast nothing; before, every shadow split drew every car of the city out
//! to the 2.5 km draw distance, because the engine culls a `MultiMesh` whole (SH2: 52-63 % of
//! the shadow pass's primitives, about 4 % of the cars within its 420 m).
//!
//! A twin draws the bucket's mesh with the bucket node's material override (the walkers' VAT
//! shader; the cars' meshes carry their own) and the dynamic casters' extra margin, so the shadow
//! pass draws each kept agent exactly as the bucket node cast it. The agents' frame job keeps
//! the candidates about its eye ([`agent_frame::candidate_radius_m`]); just before the frame
//! draws, the exact test ([`agent_frame::within_reach`]) runs over them with the drawing camera
//! and the sun's range, and a twin whose kept rows changed bit for bit is written again. Costs:
//! `O(candidates)` a frame in which the candidates, the camera or the range changed, nothing
//! otherwise; a write `O(kept agents)`. Slots grow a capacity class when the kept agents
//! outgrow it and shrink when they need a quarter of it, so a count wandering about a class
//! boundary does not swap slots each frame; no node is made, and the pool's spares serve the
//! swaps, so steady frames allocate nothing.

use super::super::backend::{
    Drawn, RenderBackend, Role, Shadow, SlotId, SlotKind, SlotPool, capacity_class,
};
use crate::nodes::sim::render::agent_frame::{self, CAR_ROW, CasterFeed, WALKER_STRIDE};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use glam::Vec3;

/// Slots the layer may hold, live and spare (a twin per bucket, swaps in flight).
pub(crate) const SLOT_LIMIT: usize = 256;
/// Spare slots kept for the capacity swaps.
pub(crate) const SPARE_LIMIT: usize = 64;

/// The kind of agent a bucket holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentKind {
    /// Walkers: 16 floats a row (transform, then the walk phase).
    Walkers = 0,
    /// Moving cars: 12 floats a row.
    Cars = 1,
}

impl AgentKind {
    fn stride(self) -> usize {
        match self {
            Self::Walkers => WALKER_STRIDE,
            Self::Cars => CAR_ROW,
        }
    }
}

// One bucket's twin: what it draws with and the rows it holds.
struct Twin {
    kind: AgentKind,
    key: u8,
    mesh: u32,
    material: Option<u32>,
    margin: f32,
    slot: Option<SlotId>,
    rows: Vec<f32>,
    visible: bool,
}

/// The view the twins are kept for: the drawing camera and the sun's shadow range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CasterView {
    /// The camera's position.
    pub(crate) eye: [f32; 3],
    /// Its unit forward direction.
    pub(crate) forward: [f32; 3],
    /// The sun's shadow range, metres of view depth; zero or less: the sun casts no shadow.
    pub(crate) range: f32,
}

/// The twins of every registered bucket.
pub(crate) struct AgentCasterLayer {
    pool: SlotPool,
    twins: Vec<Twin>,
    // Casting per kind (the shadow benchmark's caster classes; the game never clears them),
    // and the twins as a whole (cleared by the benchmark's `agents_all`: the bucket nodes cast).
    cast: [bool; 2],
    enabled: bool,
    // What the twins were last kept for: the feed's serial and the view.
    kept: Option<(u64, CasterView)>,
    scratch: Vec<f32>,
}

impl AgentCasterLayer {
    /// A layer of at most `limit` slots, `spare_limit` of them spare.
    pub(crate) fn new(limit: usize, spare_limit: usize) -> Self {
        Self {
            pool: SlotPool::new(limit, spare_limit),
            twins: Vec::new(),
            cast: [true; 2],
            enabled: true,
            kept: None,
            scratch: Vec::new(),
        }
    }

    /// The pool (tests and the census).
    pub(crate) fn pool(&self) -> &SlotPool {
        &self.pool
    }

    /// Registers bucket `key` of `kind`: its mesh, the bucket node's material override (a
    /// catalogue index, `None` for the mesh's own) and its extra cull margin. A bucket registered
    /// again keeps its first registration.
    pub(crate) fn register(
        &mut self,
        kind: AgentKind,
        key: u8,
        mesh: u32,
        material: Option<u32>,
        margin: f32,
    ) {
        if self.twin_index(kind, key).is_none() {
            self.twins.push(Twin {
                kind,
                key,
                mesh,
                material,
                margin,
                slot: None,
                rows: Vec::new(),
                visible: false,
            });
        }
    }

    /// Forgets every bucket of `kind` and frees its slots: the buckets' keys now name other meshes
    /// (a rebuilt vehicle catalogue, `RENDER-65`). Every spare goes too, the earlier resizes'
    /// included: the caller frees the old meshes next, and the engine updates each instance of
    /// a `MultiMesh` naming a freed mesh, logging `Parameter "mesh" is null` for it
    /// (`RENDER-74`, as `RENDER-39` for the building slots). The other kind's spares are only
    /// made again as needed. `O(twins + spares)`, on a catalogue rebuild only.
    pub(crate) fn unregister(&mut self, backend: &mut impl RenderBackend, kind: AgentKind) {
        for twin in self.twins.iter().filter(|twin| twin.kind == kind) {
            if let Some(slot) = twin.slot {
                self.pool.release(backend, slot);
            }
        }
        self.pool.free_spares(backend);
        self.twins.retain(|twin| twin.kind != kind);
    }

    /// Whether any bucket is registered (the twins draw).
    pub(crate) fn is_active(&self) -> bool {
        self.enabled && !self.twins.is_empty()
    }

    /// Whether `kind`'s twins cast (the benchmark's `no_cars`, `no_walkers`). `O(twins)`.
    pub(crate) fn set_cast(
        &mut self,
        backend: &mut impl RenderBackend,
        kind: AgentKind,
        cast: bool,
    ) {
        self.cast[kind as usize] = cast;
        self.show(backend);
    }

    /// Whether the twins draw at all (cleared by the benchmark's `agents_all`). `O(twins)`.
    pub(crate) fn set_enabled(&mut self, backend: &mut impl RenderBackend, enabled: bool) {
        self.enabled = enabled;
        self.show(backend);
    }

    /// Frees every slot (the renderer's end); the registrations stay.
    pub(crate) fn reset(&mut self, backend: &mut impl RenderBackend) {
        self.pool.clear(backend);
        for twin in &mut self.twins {
            twin.slot = None;
            twin.rows.clear();
            twin.visible = false;
        }
        self.kept = None;
    }

    /// Keeps every twin for `view` from the feed's candidates: the exact test over them, and a
    /// write of each twin whose kept rows changed. Returns whether anything was tested.
    /// `O(candidates)` when the feed's serial or the view changed, else `O(1)`.
    pub(crate) fn update(
        &mut self,
        backend: &mut impl RenderBackend,
        feed: &CasterFeed,
        view: CasterView,
    ) -> bool {
        if !self.is_active() || self.kept == Some((feed.serial, view)) {
            return false;
        }
        self.kept = Some((feed.serial, view));
        let eye = Vec3::from_array(view.eye);
        let forward = Vec3::from_array(view.forward);
        for index in 0..self.twins.len() {
            let twin = &self.twins[index];
            let stride = twin.kind.stride();
            let candidates = match twin.kind {
                AgentKind::Walkers => &feed.candidates.walkers,
                AgentKind::Cars => &feed.candidates.cars,
            }
            .get(usize::from(twin.key))
            .map_or(&[][..], Vec::as_slice);
            self.scratch.clear();
            if view.range > 0.0 {
                for row in candidates.chunks_exact(stride) {
                    let origin = Vec3::new(row[3], row[7], row[11]);
                    if agent_frame::within_reach(origin - eye, forward, view.range) {
                        self.scratch.extend_from_slice(row);
                    }
                }
            }
            if !same_bits(&self.scratch, &self.twins[index].rows) {
                std::mem::swap(&mut self.scratch, &mut self.twins[index].rows);
                self.write(backend, index);
            }
        }
        true
    }

    // Writes twin `index`'s rows into a slot that holds them and shows it, or hides it empty.
    fn write(&mut self, backend: &mut impl RenderBackend, index: usize) {
        let twin = &self.twins[index];
        let stride = twin.kind.stride();
        let count = (twin.rows.len() / stride) as u32;
        if count > 0 {
            let wanted = capacity_class(count).0;
            let held = twin.slot.map(|slot| self.pool.slot(slot).kind().class);
            if held.is_none_or(|class| class < wanted || class > wanted + 4) {
                let kind = SlotKind {
                    mesh: twin.mesh,
                    class: wanted,
                    colors: false,
                    custom: true,
                    parameter: None,
                };
                if let Some(old) = twin.slot {
                    self.pool.release(backend, old);
                }
                let slot = self.pool.acquire(backend, kind);
                self.twins[index].slot = slot;
                if let Some(slot) = slot {
                    self.draw(backend, index, slot);
                }
            }
            let twin = &self.twins[index];
            if let Some(slot) = twin.slot {
                self.pool
                    .fill(backend, slot, &twin.rows, count, Aabb::default());
            }
        }
        self.show_twin(backend, index);
    }

    // Draws a fresh slot as twin `index`'s shadow-only caster (hidden until shown).
    fn draw(&mut self, backend: &mut impl RenderBackend, index: usize, slot: SlotId) {
        let twin = &self.twins[index];
        let fresh = self.pool.slot(slot).drawn(Role::Main).is_none();
        let drawn = Drawn {
            origin: [0.0; 3],
            aabb: Aabb::default(),
            range: [0.0, 0.0],
            material: twin.material,
            shadow: Shadow::ShadowsOnly,
            visible: false,
        };
        let margin = twin.margin;
        self.pool.draw(backend, slot, Role::Main, drawn);
        self.twins[index].visible = false;
        if fresh && let Some(instance) = self.pool.slot(slot).instance(Role::Main) {
            backend.instance_set_extra_margin(instance, margin);
        }
    }

    // Shows every twin that holds agents and may cast, hides the rest. `O(twins)`.
    fn show(&mut self, backend: &mut impl RenderBackend) {
        for index in 0..self.twins.len() {
            self.show_twin(backend, index);
        }
    }

    fn show_twin(&mut self, backend: &mut impl RenderBackend, index: usize) {
        let twin = &self.twins[index];
        let visible = self.enabled && self.cast[twin.kind as usize] && !twin.rows.is_empty();
        if let Some(slot) = twin.slot
            && twin.visible != visible
        {
            self.pool.set_visible(backend, slot, Role::Main, visible);
            self.twins[index].visible = visible;
        }
    }

    fn twin_index(&self, kind: AgentKind, key: u8) -> Option<usize> {
        self.twins
            .iter()
            .position(|twin| twin.kind == kind && twin.key == key)
    }

    /// The drawn twins: kind, bucket key and the rows they cast (tests and probes). `O(twins)`.
    pub(crate) fn drawn(&self) -> impl Iterator<Item = (AgentKind, u8, &[f32])> + '_ {
        self.twins
            .iter()
            .filter(|twin| twin.visible)
            .map(|twin| (twin.kind, twin.key, twin.rows.as_slice()))
    }
}

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

#[cfg(test)]
mod tests;
