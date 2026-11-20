// SPDX-License-Identifier: GPL-3.0-or-later

//! The preview's road chunks displayed over the resident chunks (`ROAD-112` round R2-4b, the
//! bookkeeping `road_preview_display.gd` kept until then; `ROAD-68-H` round 3). A preview's
//! overlays draw in the pool ([`ChunkOverlays`]) while the resident chunk stays drawn with the
//! runs they replace collapsed in its vertex buffer (three equal positions draw nothing); a run
//! shown again gets its published positions back. The runs are checked against the resident
//! chunks before anything visible changes, and the overlays' swap, the collapses and the
//! restores happen in one call, so no frame shows a replaced run beside its rebuilt piece or
//! neither.
//!
//! The chunk renderer (`road_chunk_renderer.gd`, which keeps the resident set, `RENDER-52`)
//! reports each resident chunk's mesh and layer table on install, its retirement from the index
//! and its mesh's free ([`Resident`]), so the display never walks the scene. A preview of a road
//! mesh generation not installed yet, or while an install is held (T61 (2)), waits and shows on
//! the install ([`ChunkDisplay::resident_changed`]); an install that replaced chunks the shown
//! preview indexes shows it again. An accepted commit's display is held with its hue until
//! `RoadDrawHolds` releases it ([`ChunkDisplay::hold`], [`ChunkDisplay::release_held`]). Main
//! thread only.

use super::super::backend::{DrawBackend, Handle};
use super::{ChunkOverlays, PreviewOverlay};
use crate::nodes::sim::render::stream::layers::roads::surfaces::LAYER_COUNT;
use crate::simulation::network::roadbed::SurfaceChunkKey;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A surface's positions are their own `f32` stream ahead of its normals (Godot 4.7, checked on
/// the GPU): a run's positions are 12 bytes a vertex from `12 * start`.
pub(crate) const POSITION_BYTES: u32 = 12;

/// One layer of a resident chunk's mesh as the chunk renderer built it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResidentLayer {
    /// The layer's surface in the mesh, -1 when the layer is empty.
    pub(crate) surface: i32,
    /// Its vertices.
    pub(crate) vertices: u32,
    /// Whether its vertex stream holds only positions (stride 12), so a run's positions are
    /// written in place.
    pub(crate) positions_only: bool,
}

impl Default for ResidentLayer {
    fn default() -> Self {
        Self {
            surface: -1,
            vertices: 0,
            positions_only: false,
        }
    }
}

/// A resident chunk: its mesh and each payload layer's surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResidentChunk {
    /// The mesh (the chunk renderer's).
    pub(crate) mesh: Handle,
    /// Each payload layer, in payload order.
    pub(crate) layers: [ResidentLayer; LAYER_COUNT],
}

/// The chunk renderer's resident chunks as it reported them, its installed generation and grid,
/// and whether an install is held.
#[derive(Debug)]
pub(crate) struct Resident {
    chunks: BTreeMap<SurfaceChunkKey, ResidentChunk>,
    generation: i64,
    span_m: f32,
    origin_m: (f32, f32),
    /// Whether an install is held for a graded edit (T61 (2), `RoadDrawHolds::is_holding`):
    /// its chunks are not drawn yet. The bridge sets it before each display call.
    pub(crate) holding: bool,
}

impl Default for Resident {
    fn default() -> Self {
        Self {
            chunks: BTreeMap::new(),
            generation: -1,
            span_m: 0.0,
            origin_m: (0.0, 0.0),
            holding: false,
        }
    }
}

impl Resident {
    /// Chunk `key` installed with `chunk` (replacing the one indexed there). `O(log chunks)`.
    pub(crate) fn install(&mut self, key: SurfaceChunkKey, chunk: ResidentChunk) {
        self.chunks.insert(key, chunk);
    }

    /// Chunk `key` left the index (its mesh may still draw while a held install keeps it,
    /// until it is freed). `O(log chunks)`.
    pub(crate) fn retire(&mut self, key: SurfaceChunkKey) {
        self.chunks.remove(&key);
    }

    /// The installed road mesh generation (-1 while the renderer needs a full snapshot) and the
    /// chunks' grid.
    pub(crate) fn set_grid(&mut self, generation: i64, span_m: f32, origin_m: (f32, f32)) {
        self.generation = generation;
        self.span_m = span_m;
        self.origin_m = origin_m;
    }

    /// The installed road mesh generation.
    pub(crate) fn generation(&self) -> i64 {
        self.generation
    }

    fn get(&self, key: SurfaceChunkKey) -> Option<&ResidentChunk> {
        self.chunks.get(&key)
    }

    // Whether `overlay` is on the installed grid.
    fn same_grid(&self, overlay: &PreviewOverlay) -> bool {
        overlay.span_m == self.span_m && overlay.origin_m == self.origin_m
    }
}

// One resident chunk's collapsed runs: its mesh's surfaces and each run with the byte offset of
// its published positions in `bytes`, sorted by run.
#[derive(Default)]
struct Collapsed {
    key: SurfaceChunkKey,
    mesh: Handle,
    surfaces: [i32; LAYER_COUNT],
    runs: Vec<([u32; 3], usize)>,
    bytes: Vec<u8>,
}

impl Collapsed {
    fn has(&self, run: [u32; 3]) -> bool {
        self.runs
            .binary_search_by(|&(collapsed, _)| collapsed.cmp(&run))
            .is_ok()
    }
}

// A chunk of the preview being shown: the resident chunk its runs index and its entry in the
// overlay's `runs`.
struct Target {
    key: SurfaceChunkKey,
    chunk: ResidentChunk,
    runs: usize,
}

// The current preview's collapsed runs and the held commit's.
const CURRENT: usize = 0;
const HELD: usize = 1;

/// One collapsed chunk, for tests and probes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CollapsedState {
    /// The chunk.
    pub(crate) key: SurfaceChunkKey,
    /// The resident mesh whose runs are collapsed.
    pub(crate) mesh: Handle,
    /// Its collapsed runs.
    pub(crate) runs: usize,
    /// Whether it is the held commit's (else the current preview's).
    pub(crate) held: bool,
}

/// The preview's road chunks over the resident chunks: the overlay pool and the collapsed runs,
/// the current preview's and an accepted commit's held one. See the module header.
pub(crate) struct ChunkDisplay<B: DrawBackend> {
    pool: ChunkOverlays<B>,
    collapsed: [Vec<Collapsed>; 2],
    // Kept buffers: the next current set, spent entries, the validated targets, a zero run.
    next: Vec<Collapsed>,
    spare: Vec<Collapsed>,
    targets: Vec<Target>,
    zeros: Vec<u8>,
    // The shown preview (re-shown when an install replaces chunks it indexes), and the preview
    // waiting for an install with its number.
    shown: Option<Arc<PreviewOverlay>>,
    pending: Option<(i64, Arc<PreviewOverlay>)>,
    // An accepted commit's display is held.
    held: bool,
}

impl<B: DrawBackend> ChunkDisplay<B> {
    /// A display drawing its overlays in `pool`.
    pub(crate) fn new(pool: ChunkOverlays<B>) -> Self {
        Self {
            pool,
            collapsed: [Vec::new(), Vec::new()],
            next: Vec::new(),
            spare: Vec::new(),
            targets: Vec::new(),
            zeros: Vec::new(),
            shown: None,
            pending: None,
            held: false,
        }
    }

    /// The overlay pool.
    pub(crate) fn pool(&self) -> &ChunkOverlays<B> {
        &self.pool
    }

    /// The shown preview's number (0: none).
    pub(crate) fn sequence(&self) -> i64 {
        self.pool.sequence()
    }

    /// The number of the preview waiting for an install (0: none).
    pub(crate) fn pending(&self) -> i64 {
        self.pending.as_ref().map_or(0, |(sequence, _)| *sequence)
    }

    /// Whether an accepted commit's display is held.
    pub(crate) fn held(&self) -> bool {
        self.held
    }

    /// The collapsed chunks, the current preview's first, then the held commit's.
    pub(crate) fn collapsed(&self) -> impl Iterator<Item = CollapsedState> + '_ {
        [CURRENT, HELD].into_iter().flat_map(move |set| {
            self.collapsed[set].iter().map(move |chunk| CollapsedState {
                key: chunk.key,
                mesh: chunk.mesh,
                runs: chunk.runs.len(),
                held: set == HELD,
            })
        })
    }

    /// Shows `overlay` as preview `sequence` over `resident`: the overlays staged and swapped in,
    /// the runs it replaces collapsed and the ones it no longer replaces restored, in this call.
    /// True when shown, already shown, or waiting: a preview of a generation not installed yet,
    /// or while an install is held, clears the display and shows on the install
    /// ([`Self::resident_changed`]). False, with the display unchanged, when the preview is not
    /// on the installed grid or its runs do not fit the resident chunks.
    /// `O(overlay bytes + chunks * log resident + runs * log runs + hidden vertices)`; once the
    /// buffers have grown, the only allocations are the engine's own surface buffers.
    pub(crate) fn show(
        &mut self,
        sequence: i64,
        overlay: &Arc<PreviewOverlay>,
        resident: &Resident,
    ) -> bool {
        if sequence > 0 && sequence == self.sequence() {
            return true;
        }
        if resident.generation >= 0 && !resident.same_grid(overlay) {
            return false;
        }
        // The hidden runs index the resident chunks of exactly this generation; held chunks
        // are not drawn yet: wait for them as for an install.
        if overlay.generation != resident.generation || resident.holding {
            self.clear();
            self.pending = Some((sequence, Arc::clone(overlay)));
            return true;
        }
        if !self.fit(overlay, resident) {
            return false;
        }
        self.pool.show(sequence, overlay);
        self.pending = None;
        self.apply(Some(&**overlay));
        self.shown = Some(Arc::clone(overlay));
        true
    }

    /// After the chunk renderer installed (or swapped in held) resident chunks: shows the
    /// preview that waited for them, or shows the shown one again when the install replaced
    /// chunks it indexes (another generation). Cleared when it does not fit.
    pub(crate) fn resident_changed(&mut self, resident: &Resident) {
        let next = self.pending.take().or_else(|| {
            let shown = self.shown.as_ref()?;
            (self.sequence() > 0 && shown.generation != resident.generation)
                .then(|| (self.sequence(), Arc::clone(shown)))
        });
        let Some((sequence, overlay)) = next else {
            return;
        };
        self.clear();
        if !self.show(sequence, &overlay, resident) {
            self.clear();
        }
    }

    /// Restores every collapsed run of the current preview and hides its overlays; the waiting
    /// preview goes too. The held commit's display stays. `O(collapsed runs' vertices)`.
    pub(crate) fn clear(&mut self) {
        self.apply(None);
        if self.sequence() > 0 {
            self.pool.clear();
        }
        self.pending = None;
        self.shown = None;
    }

    /// An accepted commit's display stays drawn until its chunks install
    /// ([`Self::release_held`]): the current preview's overlays and collapsed runs become the
    /// held ones (a display held before goes first) and the next preview starts empty.
    pub(crate) fn hold(&mut self) {
        self.release_held();
        self.pool.hold();
        self.collapsed.swap(CURRENT, HELD);
        self.pending = None;
        self.shown = None;
        self.held = true;
    }

    /// The held commit's display goes: its runs restored on the meshes still drawn, its overlays
    /// hidden. `O(held runs' vertices)`.
    pub(crate) fn release_held(&mut self) {
        let Self {
            pool,
            collapsed,
            spare,
            ..
        } = self;
        for chunk in collapsed[HELD].drain(..) {
            restore(pool.backend_mut(), &chunk, None);
            spare.push(chunk);
        }
        pool.release_held();
        self.held = false;
    }

    /// The chunk renderer freed `mesh` (an install replaced its chunk, or a held install
    /// swapped): its collapsed runs are forgotten, never written again. `O(collapsed chunks)`.
    pub(crate) fn forget_mesh(&mut self, mesh: Handle) {
        for set in &mut self.collapsed {
            let mut at = 0;
            while at < set.len() {
                if set[at].mesh == mesh {
                    self.spare.push(set.remove(at));
                } else {
                    at += 1;
                }
            }
        }
    }

    // The targets of `overlay`'s runs on `resident`, or false when a run does not fit its
    // resident chunk: the chunk is not resident, the layer has no surface or no positions-only
    // stream, the run is empty or past the layer's vertices, or the positions do not match.
    // `O(chunks * log resident + runs)`.
    fn fit(&mut self, overlay: &PreviewOverlay, resident: &Resident) -> bool {
        self.targets.clear();
        for (index, runs) in overlay.runs.iter().enumerate() {
            let (hidden, positions) = overlay.runs_of(runs);
            if hidden.is_empty() {
                if positions.is_empty() {
                    continue;
                }
                return false;
            }
            let Some(chunk) = resident.get(runs.key) else {
                return false;
            };
            let mut vertices = 0usize;
            for &[layer, start, end] in hidden {
                let Some(target) = chunk.layers.get(layer as usize) else {
                    return false;
                };
                if target.surface < 0
                    || end <= start
                    || end > target.vertices
                    || !target.positions_only
                {
                    return false;
                }
                vertices += (end - start) as usize;
            }
            if vertices * POSITION_BYTES as usize != positions.len() {
                return false;
            }
            self.targets.push(Target {
                key: runs.key,
                chunk: *chunk,
                runs: index,
            });
        }
        true
    }

    // Collapses the runs of the targets (`overlay`'s, none when `None`) and restores the
    // current ones they no longer hide: restores first, then collapses, so a run that moved
    // within a layer ends collapsed. A collapsed chunk whose resident mesh an install replaced
    // gets its runs back on the old mesh while it still draws. The targets become the current
    // set. `O(runs * log runs + hidden vertices)`.
    fn apply(&mut self, overlay: Option<&PreviewOverlay>) {
        let Self {
            pool,
            collapsed,
            next,
            spare,
            targets,
            zeros,
            ..
        } = self;
        if overlay.is_none() {
            targets.clear();
        }
        // The next set: each target's runs, sorted, with their published positions.
        for target in targets.iter() {
            let overlay = overlay.expect("targets come from an overlay");
            let (hidden, positions) = overlay.runs_of(&overlay.runs[target.runs]);
            let mut chunk = spare.pop().unwrap_or_default();
            chunk.key = target.key;
            chunk.mesh = target.chunk.mesh;
            chunk.surfaces = target.chunk.layers.map(|layer| layer.surface);
            chunk.runs.clear();
            let mut at = 0;
            for &run in hidden {
                chunk.runs.push((run, at));
                at += (run[2] - run[1]) as usize * POSITION_BYTES as usize;
            }
            chunk.runs.sort_unstable_by_key(|&(run, _)| run);
            chunk.bytes.clear();
            chunk.bytes.extend_from_slice(positions);
            next.push(chunk);
        }
        let backend = pool.backend_mut();
        let same = |set: &[Collapsed], chunk: &Collapsed| -> Option<usize> {
            set.binary_search_by_key(&chunk.key, |other| other.key)
                .ok()
                .filter(|&at| set[at].mesh == chunk.mesh)
        };
        for chunk in &collapsed[CURRENT] {
            restore(backend, chunk, same(next, chunk).map(|at| &next[at]));
        }
        for chunk in next.iter() {
            let kept = same(&collapsed[CURRENT], chunk).map(|at| &collapsed[CURRENT][at]);
            for &(run, _) in &chunk.runs {
                if kept.is_some_and(|kept| kept.has(run)) {
                    continue;
                }
                let length = (run[2] - run[1]) as usize * POSITION_BYTES as usize;
                if zeros.len() < length {
                    zeros.resize(length, 0);
                }
                write(backend, chunk, run, &zeros[..length]);
            }
        }
        spare.append(&mut collapsed[CURRENT]);
        std::mem::swap(&mut collapsed[CURRENT], next);
    }
}

// Writes `chunk`'s collapsed runs back, but those `kept` still collapses (the same mesh's).
fn restore(backend: &mut impl DrawBackend, chunk: &Collapsed, kept: Option<&Collapsed>) {
    for &(run, at) in &chunk.runs {
        if kept.is_some_and(|kept| kept.has(run)) {
            continue;
        }
        let length = (run[2] - run[1]) as usize * POSITION_BYTES as usize;
        write(backend, chunk, run, &chunk.bytes[at..at + length]);
    }
}

// Writes `bytes` over run `run`'s positions in `chunk`'s mesh.
fn write(backend: &mut impl DrawBackend, chunk: &Collapsed, run: [u32; 3], bytes: &[u8]) {
    let surface = chunk.surfaces[run[0] as usize] as u32;
    backend.mesh_write_vertices(chunk.mesh, surface, run[1] * POSITION_BYTES, bytes);
}

#[cfg(test)]
mod tests;
