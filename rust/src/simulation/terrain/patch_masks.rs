// SPDX-License-Identifier: GPL-2.0-only

//! Per-patch shading masks baked from a render patch's height samples.
//!
//! The terrain shader derived local relief from nine heightmap taps per pixel and ran the cliff
//! face test, about 25 more, on every pixel although cliffs cover little of any world. Both
//! depend only on the patch's height samples, so they are baked once per patch payload on the
//! heightmap's texel grid into an RG8 texture the shader reads with one bilinear fetch:
//!
//! - R: local relief, which varies smoothly enough at texel spacing to be read back filtered.
//! - G: cliff reach, nonzero within about a texel of anywhere the cliff face is. The cliff
//!   masks themselves stay per pixel in the shader, which runs them only where this is set:
//!   their edge bands are narrower than a texel, and a texel-grid bake of them stair-stepped.
//!
//! The sampling is the shader's: clamped UVs, bilinear with clamp-to-edge, the same radii and
//! thresholds (mirrored from terrain.gd's CLIFF_* constants, which must change together).

use super::TerrainPatchSnapshot;
use crate::config::HEIGHT_SCALE;

// Local relief is the height range over a ring of eight taps this many texels out.
const RELIEF_SAMPLE_RADIUS_TEXELS: f32 = 3.0;
// Relief is stored as sqrt(relief / RELIEF_ENCODE_MAX_M), which keeps about 2 cm of precision at
// the 0.1 m contour threshold; every shader use saturates by 20 m.
const RELIEF_ENCODE_MAX_M: f32 = 32.0;
const CLIFF_SLOPE_START: f32 = 0.26;
const CLIFF_SLOPE_END: f32 = 0.44;
const CLIFF_RELIEF_START_M: f32 = 4.0;
const CLIFF_RELIEF_END_M: f32 = 14.0;
const CLIFF_SAMPLE_RADIUS_TEXELS: f32 = 2.25;
const CLIFF_LATERAL_SMOOTHING_TEXELS: f32 = 1.2;
// The face is evaluated on a grid this many times finer than the texels, so a face narrower than
// a texel still marks the texels around it.
const CLIFF_REACH_SUBSAMPLES: usize = 2;

// Texels from a point that the slope tests of its face evaluation can read: the lateral offset
// (1.2), the gradient's central difference (1) and the bilinear footprint (1), rounded up.
const CLIFF_SLOPE_FOOTPRINT_TEXELS: usize = 4;

// Bytes per baked mask texel: relief, cliff reach.
const PATCH_MASK_CHANNELS: usize = 2;

impl TerrainPatchSnapshot {
    /// Bakes the shading masks over the patch's full texture grid, border ring included, as
    /// row-major RG8: encoded local relief, then cliff reach.
    ///
    /// O(texels) with a constant tap count per texel (relief, plus four face tests per texel);
    /// render payload export only, never per tick.
    pub(crate) fn shading_mask_bytes(&self) -> Vec<u8> {
        let width = self.texture_width;
        let height = self.texture_height;
        if width == 0 || height == 0 || self.height_data.len() != width * height {
            return Vec::new();
        }
        let sampler = HeightSampler {
            data: &self.height_data,
            width,
            height,
            texel: [1.0 / width as f32, 1.0 / height as f32],
            cell_m: self.world_size_x / (self.sample_width.saturating_sub(1).max(1)) as f32,
        };
        let faces = sampler.subsampled_faces();
        let fine_width = width * CLIFF_REACH_SUBSAMPLES;
        let fine_height = height * CLIFF_REACH_SUBSAMPLES;
        let mut bytes = vec![0_u8; width * height * PATCH_MASK_CHANNELS];
        for z in 0..height {
            for x in 0..width {
                let uv = [
                    (x as f32 + 0.5) * sampler.texel[0],
                    (z as f32 + 0.5) * sampler.texel[1],
                ];
                let relief = (sampler.local_relief(uv) / RELIEF_ENCODE_MAX_M)
                    .clamp(0.0, 1.0)
                    .sqrt();
                // Face over this texel and its neighbours. Bilinear filtering then reads it as
                // nonzero up to a texel further out, which covers every fragment the shader's
                // own face test can light.
                let fine_x = x * CLIFF_REACH_SUBSAMPLES;
                let fine_z = z * CLIFF_REACH_SUBSAMPLES;
                let mut reach = 0.0_f32;
                for fz in fine_z.saturating_sub(CLIFF_REACH_SUBSAMPLES)
                    ..(fine_z + 2 * CLIFF_REACH_SUBSAMPLES).min(fine_height)
                {
                    for fx in fine_x.saturating_sub(CLIFF_REACH_SUBSAMPLES)
                        ..(fine_x + 2 * CLIFF_REACH_SUBSAMPLES).min(fine_width)
                    {
                        reach = reach.max(faces[fz * fine_width + fx]);
                    }
                }
                // Any face at all must survive quantization.
                let reach = if reach > 0.0 { unorm8(reach).max(1) } else { 0 };
                let offset = (z * width + x) * PATCH_MASK_CHANNELS;
                bytes[offset..offset + PATCH_MASK_CHANNELS]
                    .copy_from_slice(&[unorm8(relief), reach]);
            }
        }
        bytes
    }
}

fn unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}

// Running maximum over CLIFF_SLOPE_FOOTPRINT_TEXELS either side along one axis, edges clamped.
fn window_max(values: &[f32], width: usize, height: usize, axis: [usize; 2]) -> Vec<f32> {
    let reach = CLIFF_SLOPE_FOOTPRINT_TEXELS;
    let mut out = vec![0.0; values.len()];
    for z in 0..height {
        for x in 0..width {
            let (pos, len) = if axis[0] == 1 {
                (x, width)
            } else {
                (z, height)
            };
            let mut max = 0.0_f32;
            for i in pos.saturating_sub(reach)..(pos + reach + 1).min(len) {
                let index = if axis[0] == 1 {
                    z * width + i
                } else {
                    i * width + x
                };
                max = max.max(values[index]);
            }
            out[z * width + x] = max;
        }
    }
    out
}

/// The shader's view of one patch heightmap: normalized UVs, bilinear, clamp-to-edge.
struct HeightSampler<'a> {
    data: &'a [f32],
    width: usize,
    height: usize,
    texel: [f32; 2],
    cell_m: f32,
}

impl HeightSampler<'_> {
    fn texel_value(&self, x: isize, z: isize) -> f32 {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let z = z.clamp(0, self.height as isize - 1) as usize;
        self.data[z * self.width + x]
    }

    // `sample_world_height`: the shader clamps UVs to [0, 1] before a linear-filtered fetch.
    fn height(&self, uv: [f32; 2]) -> f32 {
        let u = uv[0].clamp(0.0, 1.0) * self.width as f32 - 0.5;
        let v = uv[1].clamp(0.0, 1.0) * self.height as f32 - 0.5;
        let (x0, z0) = (u.floor(), v.floor());
        let (fx, fz) = (u - x0, v - z0);
        let (x0, z0) = (x0 as isize, z0 as isize);
        let top = self.texel_value(x0, z0) * (1.0 - fx) + self.texel_value(x0 + 1, z0) * fx;
        let bottom =
            self.texel_value(x0, z0 + 1) * (1.0 - fx) + self.texel_value(x0 + 1, z0 + 1) * fx;
        (top * (1.0 - fz) + bottom * fz) * HEIGHT_SCALE
    }

    fn gradient(&self, uv: [f32; 2]) -> [f32; 2] {
        let [tx, tz] = self.texel;
        let left = self.height(sub(uv, [tx, 0.0]));
        let right = self.height(add(uv, [tx, 0.0]));
        let up = self.height(sub(uv, [0.0, tz]));
        let down = self.height(add(uv, [0.0, tz]));
        let cell = self.cell_m.max(0.001);
        [(right - left) / (2.0 * cell), (down - up) / (2.0 * cell)]
    }

    // `slope_metric`: `1 - normalize(-gx, 1, -gz).y`.
    fn slope(&self, uv: [f32; 2]) -> f32 {
        let [gx, gz] = self.gradient(uv);
        1.0 - (1.0 / (gx * gx + 1.0 + gz * gz).sqrt()).clamp(0.0, 1.0)
    }

    fn local_relief(&self, uv: [f32; 2]) -> f32 {
        let [tx, tz] = scale(self.texel, RELIEF_SAMPLE_RADIUS_TEXELS);
        let samples = [
            self.height(uv),
            self.height(add(uv, [tx, 0.0])),
            self.height(sub(uv, [tx, 0.0])),
            self.height(add(uv, [0.0, tz])),
            self.height(sub(uv, [0.0, tz])),
            self.height(add(uv, [tx, tz])),
            self.height(sub(uv, [tx, tz])),
            self.height(add(uv, [tx, -tz])),
            self.height(add(uv, [-tx, tz])),
        ];
        let max = samples.iter().copied().fold(f32::MIN, f32::max);
        let min = samples.iter().copied().fold(f32::MAX, f32::min);
        max - min
    }

    fn cliff_candidate(&self, uv: [f32; 2], along: [f32; 2]) -> f32 {
        let slope_mask = smoothstep(CLIFF_SLOPE_START, CLIFF_SLOPE_END, self.slope(uv));
        let relief_m = (self.height(add(uv, along)) - self.height(sub(uv, along))).abs();
        slope_mask * smoothstep(CLIFF_RELIEF_START_M, CLIFF_RELIEF_END_M, relief_m)
    }

    // The face term of the shader's `cliff_masks`, before its edges.
    fn cliff_face(&self, uv: [f32; 2]) -> f32 {
        let gradient = self.gradient(uv);
        let length = (gradient[0] * gradient[0] + gradient[1] * gradient[1]).sqrt();
        if length <= 0.0001 {
            return 0.0;
        }
        let upslope = scale(gradient, 1.0 / length);
        let along = [
            upslope[0] * self.texel[0] * CLIFF_SAMPLE_RADIUS_TEXELS,
            upslope[1] * self.texel[1] * CLIFF_SAMPLE_RADIUS_TEXELS,
        ];
        let lateral = [
            -upslope[1] * self.texel[0] * CLIFF_LATERAL_SMOOTHING_TEXELS,
            upslope[0] * self.texel[1] * CLIFF_LATERAL_SMOOTHING_TEXELS,
        ];
        let center = self.cliff_candidate(uv, along);
        let left = self.cliff_candidate(add(uv, lateral), along);
        let right = self.cliff_candidate(sub(uv, lateral), along);
        smoothstep(0.10, 0.82, center * 0.5 + left * 0.25 + right * 0.25)
    }

    // Face at the centres of a grid CLIFF_REACH_SUBSAMPLES times finer than the texels, skipping
    // every point whose slope bound rules a face out: most ground is far from steep.
    fn subsampled_faces(&self) -> Vec<f32> {
        let may_be_cliff = self.steep_enough_texels();
        let fine_width = self.width * CLIFF_REACH_SUBSAMPLES;
        let fine_height = self.height * CLIFF_REACH_SUBSAMPLES;
        let mut faces = vec![0.0; fine_width * fine_height];
        for z in 0..fine_height {
            for x in 0..fine_width {
                let texel = (z / CLIFF_REACH_SUBSAMPLES) * self.width + x / CLIFF_REACH_SUBSAMPLES;
                if may_be_cliff[texel] {
                    faces[z * fine_width + x] = self.cliff_face([
                        (x as f32 + 0.5) / fine_width as f32,
                        (z as f32 + 0.5) / fine_height as f32,
                    ]);
                }
            }
        }
        faces
    }

    // Per texel: whether any slope the face test reads near it can reach CLIFF_SLOPE_START.
    // Each gradient component is a difference of bilinear heights two texels apart, so it is at
    // most the largest step between adjacent texels in reach, over the cell; the slope metric
    // grows with the gradient, so that step bounds it. Below the start the face is exactly zero.
    fn steep_enough_texels(&self) -> Vec<bool> {
        let (width, height) = (self.width, self.height);
        let mut steps = vec![0.0_f32; width * height];
        for z in 0..height {
            for x in 0..width {
                let here = self.data[z * width + x];
                let mut step = 0.0_f32;
                if x + 1 < width {
                    step = step.max((self.data[z * width + x + 1] - here).abs());
                }
                if z + 1 < height {
                    step = step.max((self.data[(z + 1) * width + x] - here).abs());
                }
                if x > 0 {
                    step = step.max((self.data[z * width + x - 1] - here).abs());
                }
                if z > 0 {
                    step = step.max((self.data[(z - 1) * width + x] - here).abs());
                }
                steps[z * width + x] = step;
            }
        }
        let steps = window_max(
            &window_max(&steps, width, height, [1, 0]),
            width,
            height,
            [0, 1],
        );
        let per_cell = HEIGHT_SCALE / self.cell_m.max(0.001);
        steps
            .into_iter()
            .map(|step| {
                let gradient = std::f32::consts::SQRT_2 * step * per_cell;
                1.0 - 1.0 / (1.0 + gradient * gradient).sqrt() > CLIFF_SLOPE_START
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(width: usize, height_at: impl Fn(usize, usize) -> f32) -> TerrainPatchSnapshot {
        let mut height_data = Vec::with_capacity(width * width);
        for z in 0..width {
            for x in 0..width {
                height_data.push(height_at(x, z));
            }
        }
        TerrainPatchSnapshot {
            patch_x: 0,
            patch_z: 0,
            sample_width: width - 8,
            sample_height: width - 8,
            texture_width: width,
            texture_height: width,
            inner_offset_x: 4,
            inner_offset_z: 4,
            world_origin_x: 0.0,
            world_origin_z: 0.0,
            world_size_x: (width - 9) as f32 * 10.0,
            world_size_z: (width - 9) as f32 * 10.0,
            height_data,
        }
    }

    fn texel(bytes: &[u8], width: usize, x: usize, z: usize) -> [u8; 2] {
        let offset = (z * width + x) * PATCH_MASK_CHANNELS;
        bytes[offset..offset + PATCH_MASK_CHANNELS]
            .try_into()
            .unwrap()
    }

    #[test]
    fn flat_ground_bakes_no_relief_or_cliff() {
        let patch = snapshot(24, |_, _| 0.5);
        let bytes = patch.shading_mask_bytes();
        assert_eq!(bytes.len(), 24 * 24 * PATCH_MASK_CHANNELS);
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn cliff_reach_covers_the_face_and_its_neighbours_only() {
        // A 30 m rise over two 10 m cells across x = 11..13 (heights are normalized by 20 m).
        let patch = snapshot(24, |x, _| match x {
            0..=11 => 0.0,
            12 => 0.75,
            _ => 1.5,
        });
        let sampler = HeightSampler {
            data: &patch.height_data,
            width: 24,
            height: 24,
            texel: [1.0 / 24.0; 2],
            cell_m: 10.0,
        };
        let bytes = patch.shading_mask_bytes();
        assert!(texel(&bytes, 24, 12, 12)[1] > 200);
        // Every texel whose own centre or a neighbour's lies on the face is marked.
        for x in 1..23 {
            let near_face = (x - 1..=x + 1)
                .any(|n| sampler.cliff_face([(n as f32 + 0.5) / 24.0, 12.5 / 24.0]) > 0.0);
            if near_face {
                assert!(texel(&bytes, 24, x, 12)[1] > 0, "texel {x} unmarked");
            }
        }
        assert_eq!(texel(&bytes, 24, 2, 12), [0, 0]);
        assert_eq!(texel(&bytes, 24, 21, 12)[1], 0);
    }

    #[test]
    fn slope_bound_skips_only_points_without_a_face() {
        // Rolling ground with steep scarps of several heights: every skipped point must be one
        // the full face test also leaves at zero.
        for amplitude in [0.05_f32, 0.2, 0.5, 1.0] {
            let patch = snapshot(40, |x, z| {
                let (x, z) = (x as f32, z as f32);
                amplitude * ((x * 0.45).sin() + (z * 0.3 + x * 0.1).cos())
                    + if x + z * 0.5 > 25.0 { amplitude } else { 0.0 }
            });
            let sampler = HeightSampler {
                data: &patch.height_data,
                width: 40,
                height: 40,
                texel: [1.0 / 40.0; 2],
                cell_m: 10.0,
            };
            let fine = 40 * CLIFF_REACH_SUBSAMPLES;
            let skipped = sampler.subsampled_faces();
            let mut faces = 0;
            for z in 0..fine {
                for x in 0..fine {
                    let full = sampler.cliff_face([
                        (x as f32 + 0.5) / fine as f32,
                        (z as f32 + 0.5) / fine as f32,
                    ]);
                    assert_eq!(
                        skipped[z * fine + x],
                        full,
                        "amplitude {amplitude} at {x},{z}"
                    );
                    faces += usize::from(full > 0.0);
                }
            }
            if amplitude >= 1.0 {
                assert!(
                    faces > 0,
                    "amplitude {amplitude} made no cliff to test against"
                );
            }
        }
    }

    #[test]
    fn gentle_slope_has_relief_but_no_cliff() {
        // 1 m rise per 10 m cell: slope 0.005, far below the cliff threshold.
        let patch = snapshot(24, |x, _| x as f32 * 0.05);
        let bytes = patch.shading_mask_bytes();
        let middle = texel(&bytes, 24, 12, 12);
        assert_eq!(middle[1], 0);
        // Six cells across the relief ring is 6 m: sqrt(6 / 32) * 255 = 110.
        assert!((middle[0] as i32 - 110).abs() <= 1, "relief {middle:?}");
    }
}
