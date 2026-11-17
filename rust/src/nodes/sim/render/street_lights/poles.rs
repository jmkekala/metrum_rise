// SPDX-License-Identifier: GPL-2.0-only

//! The street light poles' levels of detail in chunk batches (`RENDER-46` lever 5): one batch per
//! road chunk, pole kind and level of detail instead of one per 64 m cell, kind and level, with
//! every cell drawn at the level its own batch had.
//!
//! Godot picks the level a batch draws by the batch's visibility range, measured from the camera
//! to the centre of the instance's bounds (`RendererSceneCull::_visibility_range_check`: the
//! `MultiMesh`'s bounds, grown by the instance's extra cull margin, in world space), with the
//! range's margins as hysteresis. A chunk-wide batch would measure every cell from the chunk's
//! centre, so the choice is made here for the engine: each cell keeps the centre its own batch
//! had on its pooled slot (the engine's bounds of the slot's padded buffer, computed in the
//! engine's single-precision operation order), and each drawn frame the camera's distance to it
//! decides, by the engine's rule, which cells' poles a chunk's batch of that kind and level
//! holds. Those batches draw with no range of their own.
//!
//! What differs from the cell batches (accepted by the user on 2026-11-17): the engine checks a
//! batch's range only while the batch passes a view or shadow-split frustum, so inside the 10 m
//! margins a cell batch's level depended on when it was last in a frustum; here it follows the
//! main camera every frame. The camera is the main viewport's.
//!
//! A frame evaluates the chunks near the camera and those with a drawn cell: `O(c)` for their
//! `c` cells, nothing while the camera and the chunks hold still; a batch whose cells changed is
//! rebuilt from its cells, `O(its poles)`.

use super::super::stream::backend::capacity_class;
use super::super::vegetation_product::godot_math::{Aabb, length3, sub3};
use super::{ChunkBatches, KINDS, Map};
use std::collections::BTreeSet;

/// Levels of detail of a pole (the models' `_LOD0`.. `_LOD2`).
pub(crate) const LODS: usize = 3;
// Floats a pole's transform.
const STRIDE: usize = 12;

/// A level's visibility range as the cell batches had it, and the extra cull margin its shadow
/// role gives the instance (it grows the bounds the range measures from).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Level {
    /// Range begin, metres (0: none).
    pub begin: f32,
    /// Range end, metres (0: none).
    pub end: f32,
    /// Begin hysteresis margin.
    pub begin_margin: f32,
    /// End hysteresis margin.
    pub end_margin: f32,
    /// The instance's extra cull margin.
    pub cull_margin: f32,
}

impl Level {
    // `_visibility_range_check` with fade disabled: a shown batch keeps its range widened by the
    // margins, a hidden one enters only inside the range narrowed by them.
    fn shows(&self, shown: bool, distance: f32) -> bool {
        let (mut begin_offset, mut end_offset) = (-self.begin_margin, self.end_margin);
        if !shown {
            begin_offset = -begin_offset;
            end_offset = -end_offset;
        }
        !(self.end > 0.0 && distance > self.end + end_offset)
            && !(self.begin > 0.0 && distance < self.begin + begin_offset)
    }

    // The farthest distance at which the level can show.
    fn reach(&self) -> f32 {
        if self.end > 0.0 {
            self.end + self.end_margin
        } else {
            f32::INFINITY
        }
    }
}

// A 64 m cell's poles of one kind: their run in the kind's buffer, the centre each level's batch
// measured from, and the levels shown (a bit each).
struct Cell {
    slot: u8,
    first: u32,
    count: u32,
    centres: [[f32; 3]; LODS],
    shown: u8,
}

/// A chunk's poles: its kinds in first-seen order (a slot each), each kind's transforms with the
/// cells in order, the cells, the ground rectangle of their centres, and its drawn cell levels.
#[derive(Default)]
pub(crate) struct ChunkPoles {
    kinds: Vec<u8>,
    transforms: Vec<Vec<f32>>,
    cells: Vec<Cell>,
    low: [f32; 2],
    high: [f32; 2],
    shown: u32,
    stamp: u64,
}

impl ChunkPoles {
    /// A chunk's cell batches (`LampBook::chunk_batches`, in its frame at `origin`) as chunk
    /// poles, every level hidden: `meshes` holds each kind's mesh bounds per level (None: a kind
    /// without models), `levels` the levels. `O(poles)`.
    pub(crate) fn new(
        meshes: &[[Option<Aabb>; LODS]; KINDS],
        levels: &[Level; LODS],
        origin: [f32; 3],
        batches: &ChunkBatches,
    ) -> Self {
        let mut chunk = ChunkPoles {
            low: [f32::INFINITY; 2],
            high: [f32::NEG_INFINITY; 2],
            ..ChunkPoles::default()
        };
        // The batch instance's world transform: identity basis at `origin`.
        let frame = [
            1.0, 0.0, 0.0, origin[0], 0.0, 1.0, 0.0, origin[1], 0.0, 0.0, 1.0, origin[2],
        ];
        for (&kind, buffer) in batches.kinds.iter().zip(&batches.transforms) {
            let Some(meshes) = meshes.get(usize::from(kind)) else {
                continue;
            };
            let slot = match chunk.kinds.iter().position(|&k| k == kind) {
                Some(slot) => slot,
                None => {
                    chunk.kinds.push(kind);
                    chunk.transforms.push(Vec::new());
                    chunk.kinds.len() - 1
                }
            };
            let count = buffer.len() / STRIDE;
            let mut centres = [[0.0; 3]; LODS];
            for (lod, centre) in centres.iter_mut().enumerate() {
                let Some(mesh) = meshes[lod] else {
                    continue;
                };
                *centre = slot_bounds(mesh, buffer, capacity_class(count as u32).1 as usize)
                    .grow(levels[lod].cull_margin)
                    .transformed(&frame)
                    .center();
                chunk.low = [chunk.low[0].min(centre[0]), chunk.low[1].min(centre[2])];
                chunk.high = [chunk.high[0].max(centre[0]), chunk.high[1].max(centre[2])];
            }
            let transforms = &mut chunk.transforms[slot];
            chunk.cells.push(Cell {
                slot: slot as u8,
                first: (transforms.len() / STRIDE) as u32,
                count: count as u32,
                centres,
                shown: 0,
            });
            transforms.extend_from_slice(buffer);
        }
        chunk
    }

    /// The kind of each slot (a batch per slot and level).
    pub(crate) fn kinds(&self) -> &[u8] {
        &self.kinds
    }

    /// The poles of `slot`'s kind, the most its batches ever hold.
    pub(crate) fn poles(&self, slot: usize) -> usize {
        self.transforms.get(slot).map_or(0, |t| t.len() / STRIDE)
    }

    /// The transforms (12 floats a pole) the batch of `slot` and `lod` draws now: its shown
    /// cells' poles in cell order. Empty for an unknown batch.
    pub(crate) fn batch(&self, slot: usize, lod: usize, out: &mut Vec<f32>) {
        out.clear();
        let Some(transforms) = self.transforms.get(slot) else {
            return;
        };
        for cell in &self.cells {
            if usize::from(cell.slot) == slot && cell.shown & (1 << lod) != 0 {
                let first = cell.first as usize * STRIDE;
                out.extend_from_slice(&transforms[first..first + cell.count as usize * STRIDE]);
            }
        }
    }

    /// The centre each level's cell batch measured from, per cell in order (tests).
    pub(crate) fn centres(&self) -> impl Iterator<Item = &[[f32; 3]; LODS]> + '_ {
        self.cells.iter().map(|cell| &cell.centres)
    }

    // The cell levels' new states from `eye`; returns the mask of changed batches
    // (`slot * LODS + level` bits).
    fn evaluate(&mut self, levels: &[Level; LODS], eye: [f32; 3]) -> u32 {
        let mut mask = 0u32;
        for cell in &mut self.cells {
            for (lod, level) in levels.iter().enumerate() {
                let bit = 1u8 << lod;
                let shown = cell.shown & bit != 0;
                // `Vector3::distance_to`: the length of centre - camera.
                let shows = level.shows(shown, length3(sub3(cell.centres[lod], eye)));
                if shows != shown {
                    cell.shown ^= bit;
                    if shows {
                        self.shown += 1;
                    } else {
                        self.shown -= 1;
                    }
                    mask |= 1 << (usize::from(cell.slot) * LODS + lod);
                }
            }
        }
        mask
    }
}

/// The drawn chunks' cells and their levels (see the module header).
#[derive(Default)]
pub(crate) struct PoleLods {
    levels: [Level; LODS],
    chunks: Map<(i32, i32), ChunkPoles>,
    // Chunks with a drawn cell level, and chunks not yet evaluated.
    lit: BTreeSet<(i32, i32)>,
    fresh: Vec<(i32, i32)>,
    eye: Option<[f32; 3]>,
    stamp: u64,
    scratch: Vec<(i32, i32)>,
    changed: Vec<i32>,
}

impl PoleLods {
    /// Poles at `levels`.
    pub(crate) fn new(levels: [Level; LODS]) -> Self {
        Self {
            levels,
            ..Self::default()
        }
    }

    /// Takes a drawn chunk's poles, replacing what it held, every level hidden until the next
    /// frame's evaluation. `O(1)` plus its map entry.
    pub(crate) fn insert(&mut self, key: (i32, i32), chunk: ChunkPoles) {
        self.remove(key);
        self.chunks.insert(key, chunk);
        self.fresh.push(key);
    }

    /// Takes a chunk's poles out, if any. `O(1)` plus its map entry and the chunks not yet
    /// evaluated.
    pub(crate) fn remove(&mut self, key: (i32, i32)) -> Option<ChunkPoles> {
        let chunk = self.chunks.remove(&key)?;
        self.lit.remove(&key);
        self.fresh.retain(|&fresh| fresh != key);
        Some(chunk)
    }

    /// Takes every chunk out.
    pub(crate) fn clear(&mut self) {
        self.chunks.clear();
        self.lit.clear();
        self.fresh.clear();
    }

    /// A chunk's poles.
    pub(crate) fn chunk(&self, key: (i32, i32)) -> Option<&ChunkPoles> {
        self.chunks.get(&key)
    }

    /// A drawn frame from the main camera at `eye`, on the road chunk grid of `span_m` and
    /// `origin`: evaluates every cell level that may change and returns the changed batches as
    /// triples of chunk key x, z and a mask of `slot * LODS + level` bits. Empty while the
    /// camera and the chunks hold still.
    pub(crate) fn select(&mut self, eye: [f32; 3], span_m: f32, origin: [f32; 2]) -> &[i32] {
        self.changed.clear();
        if self.eye == Some(eye) && self.fresh.is_empty() {
            return &self.changed;
        }
        self.eye = Some(eye);
        self.fresh.clear();
        self.stamp += 1;
        let reach = self.levels.iter().map(Level::reach).fold(0.0f32, f32::max);
        // Chunks with a drawn level first (they may have left the reach), then the reach's keys,
        // one chunk wider: a cell centre may stand past its chunk's edge by a pole's bounds.
        self.scratch.clear();
        self.scratch.extend(self.lit.iter().copied());
        if span_m > 0.0 && reach.is_finite() {
            let key = |p: f32, o: f32| ((p - o) / span_m).floor() as i32;
            for x in key(eye[0] - reach, origin[0]) - 1..=key(eye[0] + reach, origin[0]) + 1 {
                for z in key(eye[2] - reach, origin[1]) - 1..=key(eye[2] + reach, origin[1]) + 1 {
                    self.scratch.push((x, z));
                }
            }
        } else {
            self.scratch.extend(self.chunks.keys().copied());
        }
        for index in 0..self.scratch.len() {
            let key = self.scratch[index];
            let Some(chunk) = self.chunks.get_mut(&key) else {
                continue;
            };
            if chunk.stamp == self.stamp {
                continue;
            }
            chunk.stamp = self.stamp;
            // Beyond every level's reach on the ground with nothing drawn: nothing can show.
            let gap = [
                (chunk.low[0] - eye[0]).max(eye[0] - chunk.high[0]).max(0.0),
                (chunk.low[1] - eye[2]).max(eye[2] - chunk.high[1]).max(0.0),
            ];
            if chunk.shown == 0 && (gap[0] * gap[0] + gap[1] * gap[1]).sqrt() > reach + 1.0 {
                continue;
            }
            let mask = chunk.evaluate(&self.levels, eye);
            if chunk.shown > 0 {
                self.lit.insert(key);
            } else {
                self.lit.remove(&key);
            }
            if mask != 0 {
                self.changed.extend_from_slice(&[key.0, key.1, mask as i32]);
            }
        }
        &self.changed
    }
}

// The engine's bounds of a cell batch's pooled slot (`_multimesh_re_create_aabb` over the
// slot's `capacity` instances: the first instance's transformed mesh bounds, merged with each
// next one's, the padding repeating the first, `stream/backend.rs`).
fn slot_bounds(mesh: Aabb, buffer: &[f32], capacity: usize) -> Aabb {
    let rows = buffer.as_chunks::<STRIDE>().0;
    let Some(first) = rows.first() else {
        return Aabb::default();
    };
    let first = mesh.transformed(first);
    let bounds = rows[1..]
        .iter()
        .fold(first, |bounds, row| bounds.merge(&mesh.transformed(row)));
    (rows.len()..capacity).fold(bounds, |bounds, _| bounds.merge(&first))
}

#[cfg(test)]
mod tests;
