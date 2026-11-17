// SPDX-License-Identifier: GPL-2.0-only

//! The preview's road chunks drawn by Rust (`ROAD-112` round R2-4a): the overlay of each chunk
//! a stroke changes (`ROAD-68-H` round 3: the rebuilt pieces drawn over the resident chunk, whose
//! replaced runs `road_preview_display.gd` collapses) encoded in the `RenderingServer`'s own
//! layout by `surfaces.rs` in the preview's payload stage ([`PreviewOverlay::encode`]) and added
//! with `mesh_add_surface` to pooled meshes ([`ChunkOverlays`]) instead of `fill_chunk_mesh`'s
//! `add_surface_from_arrays` on GDScript-built arrays.
//!
//! The pool keeps one mesh and one instance a slot. A preview is staged into spare (hidden)
//! slots first and swapped in by visibility in the same call, so no frame shows a half-staged
//! preview, and the shown slots go back to the spares (trimmed to the shown count, as the
//! display's node pool was). An accepted commit's overlay is held with its hue until its chunks
//! install ([`ChunkOverlays::hold`], [`ChunkOverlays::release_held`]). Main thread only.

use super::backend::{DrawBackend, Handle};
use crate::nodes::sim::render::stream::layers::roads::surfaces::{
    LAYER_COUNT, RoadChunkSurfaces, RoadSurface,
};
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::render::road::PreviewChunk;
use crate::simulation::network::roadbed::SurfaceChunkKey;
use rayon::prelude::*;
use std::collections::BTreeMap;

/// A preview's changed chunks with an overlay, encoded for the server: what [`ChunkOverlays`]
/// draws. Made once a preview (a stroke's pose or a batch's settled overlay), shared by `Arc`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct PreviewOverlay {
    /// Each chunk with overlay triangles and its surfaces, by chunk key ascending.
    pub(crate) chunks: Vec<(SurfaceChunkKey, RoadChunkSurfaces)>,
    /// The chunk grid's span, metres.
    pub(crate) span_m: f32,
    /// The corner of chunk (0, 0), metres (x, z).
    pub(crate) origin_m: (f32, f32),
}

impl PreviewOverlay {
    /// Encodes the overlays of `chunks` (a preview's payload) on the grid of `span_m` and
    /// `origin_m`; a chunk that only hides runs has no overlay and is left out (the display
    /// still collapses its runs). Every (chunk, layer) is an independent job on the shared pool
    /// (the preview's caller waits for it): `O(overlay vertices / workers)` with one allocation
    /// a surface buffer and two job lists, counted with the preview.
    pub(crate) fn encode(
        chunks: &BTreeMap<SurfaceChunkKey, PreviewChunk>,
        span_m: f32,
        origin_m: (f32, f32),
    ) -> Self {
        let jobs: Vec<(SurfaceChunkKey, &NetworkMeshData, usize)> = chunks
            .iter()
            .flat_map(|(&key, chunk)| {
                (0..LAYER_COUNT).map(move |layer| (key, &*chunk.overlay, layer))
            })
            .collect();
        let encoded: Vec<Option<RoadSurface>> = crate::thread_pools::install(|| {
            jobs.par_iter()
                .map(|&(_, mesh, layer)| RoadChunkSurfaces::encode_layer(mesh, layer))
                .collect()
        });
        // Back into chunks, in key and layer order.
        let mut out: Vec<(SurfaceChunkKey, RoadChunkSurfaces)> = Vec::new();
        for (&(key, _, layer), surface) in jobs.iter().zip(encoded) {
            let Some(surface) = surface else {
                continue;
            };
            match out.last_mut() {
                Some((last, surfaces)) if *last == key => surfaces.layers.push((layer, surface)),
                _ => out.push((
                    key,
                    RoadChunkSurfaces {
                        layers: vec![(layer, surface)],
                        lining_shadow: None,
                    },
                )),
            }
        }
        Self {
            chunks: out,
            span_m,
            origin_m,
        }
    }

    /// Chunk `key`'s centre, the frame its payload is local to (`road_chunk_renderer.gd`
    /// `chunk_frame_centre`, the same `f64` sums rounded once).
    pub(crate) fn centre(&self, key: SurfaceChunkKey) -> [f32; 3] {
        let span = f64::from(self.span_m);
        let along =
            |origin: f32, index: i32| f64::from(origin) + f64::from(index) * span + span * 0.5;
        [
            along(self.origin_m.0, key.0) as f32,
            0.0,
            along(self.origin_m.1, key.1) as f32,
        ]
    }
}

// One pooled mesh with its instance.
#[derive(Default)]
struct Slot {
    mesh: Handle,
    instance: Handle,
    key: SurfaceChunkKey,
    origin: [f32; 3],
    surfaces: u32,
    visible: bool,
}

/// One drawn overlay chunk, for tests and probes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ChunkState {
    /// The chunk.
    pub(crate) key: SurfaceChunkKey,
    /// Its mesh.
    pub(crate) mesh: Handle,
    /// Its instance's origin (the chunk's centre).
    pub(crate) origin: [f32; 3],
    /// Its surfaces.
    pub(crate) surfaces: u32,
    /// Whether it is the held commit's (else the current preview's).
    pub(crate) held: bool,
}

/// The preview's overlay chunks over a backend: the current preview's and an accepted commit's
/// held one. See the module header.
pub(crate) struct ChunkOverlays<B: DrawBackend> {
    backend: B,
    // Each payload layer's material.
    materials: [Handle; LAYER_COUNT],
    slots: Vec<Slot>,
    // The slots shown: the current preview's and the held commit's, by `current`.
    shown: [Vec<usize>; 2],
    current: usize,
    // Never shown; staging fills only these.
    spare: Vec<usize>,
    // Freed slots' places.
    freed: Vec<usize>,
    // The staged slots, a kept buffer.
    staged: Vec<usize>,
    // The current preview's number, 0 when none is shown.
    sequence: i64,
    // Chunks staged so far (tests: an unchanged stroke uploads nothing).
    staged_total: u64,
}

impl<B: DrawBackend> ChunkOverlays<B> {
    /// An empty pool on `backend`, drawing each payload layer with its material.
    pub(crate) fn new(backend: B, materials: [Handle; LAYER_COUNT]) -> Self {
        Self {
            backend,
            materials,
            slots: Vec::new(),
            shown: [Vec::new(), Vec::new()],
            current: 0,
            spare: Vec::new(),
            freed: Vec::new(),
            staged: Vec::new(),
            sequence: 0,
            staged_total: 0,
        }
    }

    /// Shows `overlay` as preview `sequence`: every chunk staged into a spare slot (made when
    /// none is left), then swapped in for the shown ones in this call. Nothing when `sequence`
    /// is already shown. `O(overlay bytes + chunks)`; once the pool holds a pose's chunks, the
    /// only allocations are the engine's own surface buffers.
    pub(crate) fn show(&mut self, sequence: i64, overlay: &PreviewOverlay) {
        if sequence > 0 && sequence == self.sequence {
            return;
        }
        self.staged.clear();
        for (key, surfaces) in &overlay.chunks {
            let slot = match self.spare.pop() {
                Some(slot) => slot,
                None => self.make_slot(),
            };
            let entry = &mut self.slots[slot];
            if entry.surfaces > 0 {
                self.backend.mesh_clear(entry.mesh);
            }
            for (layer, surface) in &surfaces.layers {
                self.backend
                    .mesh_add_road_surface(entry.mesh, surface, self.materials[*layer]);
            }
            entry.key = *key;
            entry.surfaces = surfaces.layers.len() as u32;
            entry.origin = overlay.centre(*key);
            self.backend
                .instance_set_origin(entry.instance, entry.origin);
            self.staged.push(slot);
        }
        self.staged_total += self.staged.len() as u64;
        // The swap: the staged slots shown, the shown ones hidden, in one call.
        for &slot in &self.staged {
            set_visible(&mut self.backend, &mut self.slots[slot], true);
        }
        self.hide(self.current);
        std::mem::swap(&mut self.shown[self.current], &mut self.staged);
        self.sequence = sequence;
        self.trim();
    }

    /// Hides the current preview's chunks.
    pub(crate) fn clear(&mut self) {
        self.hide(self.current);
        self.sequence = 0;
        self.trim();
    }

    /// An accepted commit's overlay stays drawn until its chunks install
    /// ([`Self::release_held`]): the current chunks become the held ones (chunks held before
    /// go) and the next preview starts empty.
    pub(crate) fn hold(&mut self) {
        self.release_held();
        self.current = 1 - self.current;
        self.sequence = 0;
    }

    /// The held commit's chunks go (its chunks installed, or the preview was reset).
    pub(crate) fn release_held(&mut self) {
        self.hide(1 - self.current);
        self.trim();
    }

    /// The drawn chunks, the current preview's first, then the held commit's.
    pub(crate) fn state(&self) -> impl Iterator<Item = ChunkState> + '_ {
        [self.current, 1 - self.current]
            .into_iter()
            .flat_map(move |set| {
                self.shown[set].iter().map(move |&slot| {
                    let entry = &self.slots[slot];
                    ChunkState {
                        key: entry.key,
                        mesh: entry.mesh,
                        origin: entry.origin,
                        surfaces: entry.surfaces,
                        held: set != self.current,
                    }
                })
            })
    }

    /// The current preview's number (0: none shown).
    pub(crate) fn sequence(&self) -> i64 {
        self.sequence
    }

    /// Chunks staged since the pool was made.
    pub(crate) fn staged_total(&self) -> u64 {
        self.staged_total
    }

    #[cfg(test)]
    pub(crate) fn backend(&self) -> &B {
        &self.backend
    }

    // Hides set `set`'s slots and returns them to the spares.
    fn hide(&mut self, set: usize) {
        for slot in self.shown[set].drain(..) {
            set_visible(&mut self.backend, &mut self.slots[slot], false);
            self.spare.push(slot);
        }
    }

    // A new slot (a freed one's place reused).
    fn make_slot(&mut self) -> usize {
        let mesh = self.backend.mesh_create();
        let slot = Slot {
            mesh,
            instance: self.backend.instance_create_plain(mesh),
            ..Slot::default()
        };
        match self.freed.pop() {
            Some(index) => {
                self.slots[index] = slot;
                index
            }
            None => {
                self.slots.push(slot);
                self.slots.len() - 1
            }
        }
    }

    // The spares beyond the current preview's count go: the pool is one staging set, not the
    // cursor's history.
    fn trim(&mut self) {
        while self.spare.len() > self.shown[self.current].len() {
            let slot = self.spare.pop().expect("a spare");
            let entry = std::mem::take(&mut self.slots[slot]);
            self.backend.free(entry.instance);
            self.backend.free(entry.mesh);
            self.freed.push(slot);
        }
    }
}

fn set_visible(backend: &mut impl DrawBackend, slot: &mut Slot, visible: bool) {
    if slot.visible != visible {
        slot.visible = visible;
        backend.instance_set_visible(slot.instance, visible);
    }
}

impl<B: DrawBackend> Drop for ChunkOverlays<B> {
    // Frees every instance and mesh the pool made (the materials are the chunk renderer's).
    fn drop(&mut self) {
        for (index, slot) in self.slots.iter().enumerate() {
            if self.freed.contains(&index) {
                continue;
            }
            self.backend.free(slot.instance);
            self.backend.free(slot.mesh);
        }
    }
}

#[cfg(test)]
mod tests;
