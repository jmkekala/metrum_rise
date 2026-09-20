// SPDX-License-Identifier: GPL-2.0-only

//! Engine-independent, reusable building LOD batches. The bridge supplies spatial-index
//! candidates; this module never queries simulation state or calls Godot. Only topology
//! changes allocate. Warm camera updates use two packed buffers and counting scatter.

use crate::assets::lod_policy::{LodQuality, projected_size_pixels, select_lod};
use glam::{Mat4, Vec2, Vec3};
use std::collections::BTreeMap;

pub(crate) mod spatial;

/// Immutable policy bounds and the resource-generation fallback table for one part.
#[derive(Clone)]
pub(crate) struct Part {
    /// Placed LOD0 reference box; never follows the selected tier.
    pub bounds: [Vec3; 2],
    /// Union of every usable tier, used only for conservative visibility/shadow bounds.
    pub cull_bounds: [Vec3; 2],
    /// Authored ordinal to nearest usable finer tier (first usable if none is finer).
    pub fallback: Vec<usize>,
}

impl Part {
    /// Return no part when every import failed; the caller supplies a visible placeholder.
    pub fn new(bounds: [Vec3; 2], available: &[bool]) -> Option<Self> {
        let first = available.iter().position(|&loaded| loaded)?;
        let mut previous = first;
        let fallback = available
            .iter()
            .enumerate()
            .map(|(index, &loaded)| {
                if loaded {
                    previous = index;
                }
                previous
            })
            .collect();
        Some(Self {
            bounds,
            cull_bounds: bounds,
            fallback,
        })
    }
}

/// Camera projection in actual 3D render pixels and the shared quality preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// Godot/OpenGL clip-space projection, including the camera inverse.
    pub world_to_clip: Mat4,
    /// Resolution after render scaling, not UI logical dimensions.
    pub render_size: Vec2,
    /// Global player preference.
    pub quality: LodQuality,
}

/// One placed part; building ordinals are valid only within the allocator revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Instance {
    /// Allocator-local building ordinal.
    pub building: usize,
    /// Catalog-local part ordinal.
    pub part: usize,
    /// Whether the gray deserted material replaces source materials.
    pub deserted: bool,
    /// Complete local-mesh to world transform, including construction rise.
    pub transform: Mat4,
}

/// Stable group identity inside one spatial chunk and catalog generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct BatchKey {
    /// Catalog-local part ordinal.
    pub part: usize,
    /// Material override group.
    pub deserted: bool,
    /// Usable authored tier ordinal.
    pub lod: usize,
}

struct Entry {
    instance: Instance,
    group: usize,
    previous: Option<usize>,
    selected: usize,
}

/// Slice into a chunk's packed transform buffer.
pub(crate) struct Batch {
    /// Group identity for GPU buffer reuse.
    pub key: BatchKey,
    /// Needs publication before the next draw.
    pub changed: bool,
    /// First transform in the packed chunk buffer.
    pub offset: usize,
    /// Number of live instances, independent of GPU capacity.
    pub count: usize,
    next_count: usize,
    next_offset: usize,
}

/// Reusable counting/scatter storage and per-instance hysteresis for a resident chunk.
#[derive(Default)]
pub(crate) struct Chunk {
    entries: Vec<Entry>,
    /// Deterministically ordered GPU group headers, including retired empty groups.
    pub batches: Vec<Batch>,
    packed: Vec<[f32; 12]>,
    scratch: Vec<[f32; 12]>,
    /// Parts evaluated since the last acknowledgement.
    pub evaluations: usize,
}

impl Chunk {
    /// Synchronize changed simulation inputs. Ordinals may be reused after swap-removal;
    /// the bridge must request a history reset when the building-reference revision changes.
    pub fn synchronize(&mut self, instances: &[Instance], parts: &[Part], reset_history: bool) {
        let same_layout = self.entries.len() == instances.len()
            && self.entries.iter().zip(instances).all(|(old, new)| {
                old.instance.building == new.building
                    && old.instance.part == new.part
                    && old.instance.deserted == new.deserted
            });
        if same_layout {
            for (entry, instance) in self.entries.iter_mut().zip(instances) {
                entry.instance = *instance;
                if reset_history {
                    entry.previous = None;
                }
            }
            return;
        }

        // Stable group order makes output independent of hash/Rayon scheduling order.
        // Keep vanished groups until the bridge has observed their zero-count update.
        let mut groups = BTreeMap::new();
        for batch in &self.batches {
            groups.insert((batch.key.part, batch.key.deserted), 0);
        }
        for instance in instances {
            groups.insert((instance.part, instance.deserted), 0);
        }
        let mut batches = Vec::new();
        for (&(part, deserted), base) in &mut groups {
            *base = batches.len();
            for lod in 0..parts[part].fallback.len() {
                let key = BatchKey {
                    part,
                    deserted,
                    lod,
                };
                let count = self
                    .batches
                    .binary_search_by_key(&key, |batch| batch.key)
                    .map_or(0, |index| self.batches[index].count);
                batches.push(Batch {
                    key,
                    // Existing nonempty groups need replacement even if their newly
                    // scattered bytes happen to match the reset scratch buffer.
                    changed: count != 0,
                    offset: 0,
                    count,
                    next_count: 0,
                    next_offset: 0,
                });
            }
        }
        self.batches = batches;
        self.entries.clear();
        self.entries.extend(instances.iter().map(|&instance| Entry {
            instance,
            group: groups[&(instance.part, instance.deserted)],
            previous: None,
            selected: 0,
        }));
        self.packed.resize(instances.len(), [0.0; 12]);
        self.scratch.resize(instances.len(), [0.0; 12]);
        // Old ranges no longer refer to the same layout after a topology change.
        for batch in &mut self.batches {
            batch.count = 0;
        }
    }

    /// O(instances + groups), no allocations after synchronization. Callers parallelize
    /// independent chunks with Rayon, then publish all changed groups in one render frame.
    pub fn update(&mut self, parts: &[Part], view: View, reset_history: bool) {
        self.evaluations = self.entries.len();
        for batch in &mut self.batches {
            batch.next_count = 0;
        }
        for entry in &mut self.entries {
            let part = &parts[entry.instance.part];
            let pixels = projected_size_pixels(
                part.bounds[0],
                part.bounds[1],
                view.world_to_clip * entry.instance.transform,
                view.render_size,
            );
            let requested = select_lod(
                pixels,
                part.fallback.len(),
                if reset_history { None } else { entry.previous },
                view.quality,
            )
            .unwrap_or(0);
            // Hysteresis follows the requested ordinal even when its import failed;
            // fallback geometry must not change the policy or trigger import retries.
            entry.previous = Some(requested);
            entry.selected = entry.group + part.fallback[requested];
            self.batches[entry.selected].next_count += 1;
        }
        let mut offset = 0;
        for batch in &mut self.batches {
            batch.next_offset = offset;
            offset += batch.next_count;
            batch.next_count = 0;
        }
        for entry in &self.entries {
            let batch = &mut self.batches[entry.selected];
            self.scratch[batch.next_offset + batch.next_count] =
                pack_transform(entry.instance.transform);
            batch.next_count += 1;
        }
        for batch in &mut self.batches {
            batch.changed |= batch.count != batch.next_count
                || self.packed[batch.offset..batch.offset + batch.count]
                    != self.scratch[batch.next_offset..batch.next_offset + batch.next_count];
            batch.offset = batch.next_offset;
            batch.count = batch.next_count;
        }
        std::mem::swap(&mut self.packed, &mut self.scratch);
    }

    /// Borrow the active packed transforms without allocating an intermediate collection.
    pub fn transforms(&self, batch: &Batch) -> &[[f32; 12]] {
        &self.packed[batch.offset..batch.offset + batch.count]
    }

    /// Conservative active-instance bounds, independent of padded GPU capacity and LOD.
    pub fn bounds(&self, batch: &Batch, part: &Part) -> [Vec3; 2] {
        let center = (part.cull_bounds[0] + part.cull_bounds[1]) * 0.5;
        let extent = (part.cull_bounds[1] - part.cull_bounds[0]) * 0.5;
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for m in self.transforms(batch) {
            let row_x = Vec3::from_slice(&m[0..3]);
            let row_y = Vec3::from_slice(&m[4..7]);
            let row_z = Vec3::from_slice(&m[8..11]);
            let placed = Vec3::new(
                row_x.dot(center) + m[3],
                row_y.dot(center) + m[7],
                row_z.dot(center) + m[11],
            );
            let radius = Vec3::new(
                row_x.abs().dot(extent),
                row_y.abs().dot(extent),
                row_z.abs().dot(extent),
            );
            lo = lo.min(placed - radius);
            hi = hi.max(placed + radius);
        }
        [lo, hi]
    }

    /// Clear dirty headers only after the complete frame has crossed the bridge.
    pub fn acknowledge(&mut self) {
        for batch in &mut self.batches {
            batch.changed = false;
        }
        self.evaluations = 0;
    }
}

/// Godot MultiMesh's row-major 3×4 affine buffer layout.
pub(super) fn pack_transform(matrix: Mat4) -> [f32; 12] {
    let [x, y, z, w] = matrix.to_cols_array_2d();
    [
        x[0], y[0], z[0], w[0], x[1], y[1], z[1], w[1], x[2], y[2], z[2], w[2],
    ]
}

#[cfg(test)]
mod tests;
