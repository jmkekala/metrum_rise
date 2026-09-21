// SPDX-License-Identifier: GPL-2.0-only

//! Fixed-resolution crown coverage from the renderer's accepted canopy decisions.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

// Independent of the saved candidate spacing. One world-aligned 1 m sample per bit produces
// the area fraction of a crown-disc union in an 8 m texel; bilinear filtering joins texels.
const TEXEL_M: f32 = 8.0;
const SAMPLES: usize = 8;
// Species-level crown radii, scaled by the accepted stem's saved/generated size. These are
// substrate footprints, independent of cosmetic mesh variant, wind and distance level.
/// Unscaled conifer and broadleaf crown radii, also used by the bulldoze preview.
pub(super) const CROWN_RADII_M: [f32; 2] = [3.0, 3.5];
const MAX_CROWN_M: f32 = 3.5 * 1.35;

/// A single patch allocation shared by the parallel candidate walk.
pub(super) struct Coverage {
    origin: Vector2,
    width: usize,
    height: usize,
    samples: Vec<AtomicU64>,
}

impl Coverage {
    /// Evaluates O(K + A) local candidates/additions with indexed footprint queries.
    /// Each accepted crown touches at most nine texels and 64 fixed samples per texel.
    pub(super) fn build(core: &SimCore, origin: Vector2, span: f32) -> Self {
        let first = (origin / TEXEL_M).floor() - Vector2::ONE;
        let end = ((origin + Vector2::splat(span)) / TEXEL_M).ceil() + Vector2::ONE;
        let width = (end.x - first.x) as usize;
        let height = (end.y - first.y) as usize;
        let coverage = Self {
            origin: first * TEXEL_M,
            width,
            height,
            samples: (0..width * height).map(|_| AtomicU64::new(0)).collect(),
        };
        let (cell_m, salt) = grid(core, VegetationLayer::Canopy);
        let low = cell_at(
            coverage.origin - Vector2::splat(MAX_CROWN_M),
            VegetationLayer::Canopy,
            cell_m,
        );
        let high = cell_at(
            end * TEXEL_M + Vector2::splat(MAX_CROWN_M),
            VegetationLayer::Canopy,
            cell_m,
        );
        let nx = high.x - low.x + 1;
        let nz = high.z - low.z + 1;
        let edited = core.vegetation_edits.any_in_cell_range(
            VegetationLayer::Canopy,
            (low.x, high.x),
            (low.z, high.z),
        );
        if edited || core.vegetation.config.enabled {
            (0..nx * nz).into_par_iter().for_each(|i| {
                let cell = VegetationCell {
                    layer: VegetationLayer::Canopy,
                    x: low.x + i % nx,
                    z: low.z + i / nx,
                };
                let (removed, added): (bool, &[Plant]) = if edited {
                    core.vegetation_edits.cell(cell)
                } else {
                    (false, &[])
                };
                if !removed && let Some(plant) = evaluate_cell(core, cell, cell_m, salt) {
                    coverage.splat(&plant);
                }
                for plant in added {
                    // Authored bushes/rocks occupy the canopy grid but do not make forest floor.
                    if plant.species < 2 && placement_clear(core, plant.x, plant.z, cell.layer) {
                        coverage.splat(plant);
                    }
                }
            });
        }
        coverage
    }

    fn splat(&self, plant: &Plant) {
        let radius = CROWN_RADII_M[plant.species as usize] * plant.scale;
        let center = Vector2::new(plant.x, plant.z);
        let low = ((center - Vector2::splat(radius) - self.origin) / TEXEL_M).floor();
        let high = ((center + Vector2::splat(radius) - self.origin) / TEXEL_M).floor();
        for z in (low.y as i32).max(0)..=(high.y as i32).min(self.height as i32 - 1) {
            for x in (low.x as i32).max(0)..=(high.x as i32).min(self.width as i32 - 1) {
                let base = self.origin + Vector2::new(x as f32, z as f32) * TEXEL_M;
                let mut mask = 0;
                for sz in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let sample = base + Vector2::new(sx as f32 + 0.5, sz as f32 + 0.5);
                        if sample.distance_squared_to(center) <= radius * radius {
                            mask |= 1u64 << (sz * SAMPLES + sx);
                        }
                    }
                }
                // OR is commutative: overlap is a union, independent of scheduling or order.
                self.samples[z as usize * self.width + x as usize]
                    .fetch_or(mask, Ordering::Relaxed);
            }
        }
    }

    fn byte(&self, index: usize) -> u8 {
        ((self.samples[index].load(Ordering::Relaxed).count_ones() * 255 + 32) / 64) as u8
    }

    /// Converts the completed patch to the existing dictionary/packed-array bridge format.
    pub(super) fn payload(&self, generations: [i64; 18]) -> VarDictionary {
        let mut bytes = PackedByteArray::new();
        bytes.resize(self.samples.len());
        for (i, byte) in bytes.as_mut_slice().iter_mut().enumerate() {
            *byte = self.byte(i);
        }
        let mut data = VarDictionary::new();
        data.set("width", self.width as i32);
        data.set("height", self.height as i32);
        data.set("bytes", bytes);
        data.set(
            "world_bounds",
            Vector4::new(
                self.origin.x,
                self.origin.y,
                self.width as f32 * TEXEL_M,
                self.height as f32 * TEXEL_M,
            ),
        );
        data.set(
            "generations",
            PackedInt64Array::from(generations.as_slice()),
        );
        data
    }
}

/// Captures existing revisions without creating a coverage revision or changing edit ownership.
pub(super) fn generations(core: &SimCore, key: Vector2i) -> [i64; 18] {
    let mut result = [0; 18];
    for (i, pair) in result.chunks_exact_mut(2).enumerate() {
        let x = key.x + i as i32 % 3 - 1;
        let z = key.y + i as i32 / 3 - 1;
        if x >= 0 && z >= 0 {
            pair[0] = core.terrain_payload_generation_for_patch(x as usize, z as usize) as i64;
            pair[1] = core.vegetation_edits.patch_generation(pack_patch_key(x, z)) as i64;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent oracle: query the renderer's packed products over both patches and their
    // border, then count covered sample points per texel. Never call evaluate_cell or splat.
    fn check_products(core: &SimCore) -> [Coverage; 2] {
        let source_origin = Vector2::new(-544.0, -288.0);
        let products = scatter_layer(
            core,
            source_origin,
            1088.0,
            core.vegetation.canopy_cell_m,
            CANOPY_SALT,
            true,
        );
        let covers = [-510.0, 0.0].map(|x| Coverage::build(core, Vector2::new(x, -255.0), 510.0));
        for cover in &covers {
            for i in 0..cover.samples.len() {
                let base = cover.origin
                    + Vector2::new((i % cover.width) as f32, (i / cover.width) as f32) * 8.0;
                let local: Vec<_> = products
                    .chunks_exact(6)
                    .filter(|p| {
                        p[5] < 2.0
                            && (p[0] + source_origin.x - base.x - 4.0).abs() < 10.0
                            && (p[2] + source_origin.y - base.y - 4.0).abs() < 10.0
                    })
                    .collect();
                let mut covered_min = 0;
                let mut covered_max = 0;
                for z in 0..8 {
                    for x in 0..8 {
                        let sample = base + Vector2::new(x as f32 + 0.5, z as f32 + 0.5);
                        // Packing patch-local f32 positions and restoring world coordinates
                        // can round by <0.1 mm on this fixture. Only samples within that
                        // bound of the circle edge may differ; all others must agree exactly.
                        let inside = |epsilon: f32| {
                            local.iter().any(|p| {
                                let center = source_origin + Vector2::new(p[0], p[2]);
                                let radius = if p[5] == 0.0 { 3.0 } else { 3.5 } * p[4] + epsilon;
                                sample.distance_squared_to(center) <= radius * radius
                            })
                        };
                        covered_min += u32::from(inside(-0.0001));
                        covered_max += u32::from(inside(0.0001));
                    }
                }
                let minimum = ((covered_min * 255 + 32) / 64) as u8;
                let maximum = ((covered_max * 255 + 32) / 64) as u8;
                assert!(
                    (minimum..=maximum).contains(&cover.byte(i)),
                    "texel {base:?}"
                );
            }
        }
        // Every shared border texel must agree, including the samples used by bilinear
        // filtering on both sides of the non-8-m-aligned 510 m patch boundary.
        let [left, right] = &covers;
        let offset = ((right.origin.x - left.origin.x) / 8.0) as usize;
        for z in 0..left.height {
            for x in offset..left.width {
                assert_eq!(
                    left.byte(z * left.width + x),
                    right.byte(z * right.width + x - offset)
                );
            }
        }
        covers
    }

    #[test]
    fn coverage_matches_accepted_canopy_across_edits_surfaces_and_patch_boundary() {
        let mut core = super::super::super::tests::vegetation_test_core(Default::default());
        prepare_sites(&mut core);
        let before = check_products(&core);
        assert!(
            before
                .iter()
                .any(|c| (0..c.samples.len()).any(|i| c.byte(i) > 0))
        );
        let layout = PatchLayout::new(&core);
        let key = Vector2i::new(
            (layout.half_w / layout.span) as i32 - 1,
            (layout.half_h / layout.span) as i32,
        );
        let revisions = generations(&core, key);
        assert!(remove_at(&mut core, Vector2::ZERO, 40.0, 0) > 0);
        assert!(add_at(&mut core, Vector2::new(0.25, 1.0), 0));
        assert!(add_at(&mut core, Vector2::new(-0.25, 1.0), 1));
        assert!(add_at(&mut core, Vector2::new(0.0, 15.0), 3));
        assert_ne!(generations(&core, key), revisions);
        let edited = check_products(&core);
        assert!(
            before
                .iter()
                .zip(&edited)
                .any(|(a, b)| (0..a.samples.len()).any(|i| a.byte(i) != b.byte(i)))
        );
        let revisions = generations(&core, key);
        let drained = core.watermap.clone_baseline_depth_dense();
        core.watermap
            .replace_baseline_depth_from_dense(&vec![1.0; drained.len()])
            .unwrap();
        core.bump_terrain_payload_patch_generations(&[(key.x as usize + 1, key.y as usize)]);
        assert_ne!(generations(&core, key), revisions);
        assert!(
            check_products(&core)
                .iter()
                .all(|c| (0..c.samples.len()).all(|i| c.byte(i) == 0))
        );
        core.watermap
            .replace_baseline_depth_from_dense(&drained)
            .unwrap();
        let restored = check_products(&core);
        for (a, b) in edited.iter().zip(&restored) {
            assert!((0..a.samples.len()).all(|i| a.byte(i) == b.byte(i)));
        }
        assert!(remove_at(&mut core, Vector2::ZERO, 1024.0, 0) > 0);
        assert!(
            check_products(&core)
                .iter()
                .all(|c| (0..c.samples.len()).all(|i| c.byte(i) == 0))
        );
        // Saved density can move candidate spacing, never the coverage layout.
        core.vegetation.canopy_cell_m *= 0.5;
        let dense = Coverage::build(&core, Vector2::new(-510.0, -255.0), 510.0);
        assert_eq!(
            (dense.origin, dense.width, dense.height),
            (before[0].origin, before[0].width, before[0].height)
        );
    }
}
