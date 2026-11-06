// SPDX-License-Identifier: GPL-2.0-only

//! Exact rectangular zoning coverage behind a coarse, conservative ground-mask lookup.
//! Rendering data only: no terrain samples, simulation mutations or persistent spatial index.

use glam::DVec2;
use rayon::prelude::*;
use std::collections::HashMap;

/// World metres per mask texel; analytic coverage is independent of this resolution.
pub(crate) const TEXEL_M: usize = 2;
/// Mask side for the existing 512 m render chunk.
#[cfg(test)]
const MASK_SIDE: usize = 512 / TEXEL_M;

/// One cell or manual parcel, in authoritative order, with a render-only style.
pub(crate) struct MaskRectangle {
    /// Ordered rectangular XZ footprint, before conversion to chunk-local floats.
    pub(crate) corners: [DVec2; 4],
    /// Linear RGB and style: occupied=0, unpainted=0.5, painted=1, preview=2.
    pub(crate) color_style: [f32; 4],
}

/// RGBA8 mask indices and RGBA32F analytic rows for one nonempty chunk.
pub(crate) struct GroundMask {
    /// Little-endian u32: zero empty, odd rectangle row*2+1, even candidate-list row*2.
    pub(crate) indices: Vec<u8>,
    /// Three rows per rectangle: origin/axis, extent/style, linear RGB; then candidate lists.
    pub(crate) rows: Vec<[f32; 4]>,
}

/// Builds only the requested chunk: O(T + C + B), where T is its fixed texel count,
/// C its intersecting rectangles and B the sum of their clipped texel AABB areas.
/// Shared-bin counting/prefix packing and deterministic list interning are serial; independent
/// rectangle preparation uses the shared Rayon pool. Scratch buffers live only for this
/// rebuild; candidate-list storage is allocated once per distinct multi-candidate list.
pub(crate) fn build_mask(origin: DVec2, rectangles: &[MaskRectangle]) -> GroundMask {
    build_mask_grid::<TEXEL_M>(origin, rectangles)
}

fn build_mask_grid<const TEXEL: usize>(origin: DVec2, rectangles: &[MaskRectangle]) -> GroundMask {
    let side = 512 / TEXEL;
    let prepared: Vec<_> = crate::thread_pools::install(|| {
        rectangles
            .par_iter()
            .map(|rectangle| {
                let p = rectangle.corners.map(|point| point - origin);
                let u = p[1] - p[0];
                let v = p[3] - p[0];
                let width = u.length();
                let axis = u / width;
                let min = p
                    .iter()
                    .copied()
                    .fold(DVec2::splat(f64::INFINITY), DVec2::min);
                let max = p
                    .iter()
                    .copied()
                    .fold(DVec2::splat(f64::NEG_INFINITY), DVec2::max);
                let lo = (min / TEXEL as f64)
                    .floor()
                    .max(DVec2::ZERO)
                    .min(DVec2::splat(side as f64));
                let hi = ((max / TEXEL as f64).floor() + DVec2::ONE)
                    .max(DVec2::ZERO)
                    .min(DVec2::splat(side as f64));
                let [r, g, b, style] = rectangle.color_style;
                (
                    [
                        [p[0].x as f32, p[0].y as f32, axis.x as f32, axis.y as f32],
                        [
                            width as f32,
                            v.length() as f32,
                            style,
                            axis.perp_dot(v).signum() as f32,
                        ],
                        [r, g, b, 0.0],
                    ],
                    [lo.x as usize, lo.y as usize, hi.x as usize, hi.y as usize],
                )
            })
            .collect()
    });
    let texels = side * side;
    let mut counts = vec![0usize; texels];
    let visit = |bounds: [usize; 4], f: &mut dyn FnMut(usize)| {
        for y in bounds[1]..bounds[3] {
            for x in bounds[0]..bounds[2] {
                f(y * side + x);
            }
        }
    };
    for &(_, bounds) in &prepared {
        visit(bounds, &mut |i| counts[i] += 1);
    }
    let mut offsets = Vec::with_capacity(texels + 1);
    offsets.push(0);
    for &count in &counts {
        offsets.push(offsets.last().copied().unwrap_or(0) + count);
    }
    let mut candidates = vec![0u32; offsets[texels]];
    counts.fill(0);
    let mut rows = Vec::with_capacity(rectangles.len() * 3);
    for (id, &(data, bounds)) in prepared.iter().enumerate() {
        rows.extend(data);
        visit(bounds, &mut |i| {
            candidates[offsets[i] + counts[i]] = (id * 3) as u32;
            counts[i] += 1;
        });
    }
    let mut lists: HashMap<Vec<u32>, u32> = HashMap::new();
    let mut indices = vec![0u8; texels * 4];
    for (i, pixel) in indices.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let list = &candidates[offsets[i]..offsets[i + 1]];
        let value = match list {
            [] => 0,
            &[id] => id * 2 + 1,
            _ => {
                if let Some(&row) = lists.get(list) {
                    row * 2
                } else {
                    let row = rows.len() as u32;
                    rows.push([list.len() as f32, 0.0, 0.0, 0.0]);
                    for ids in list.chunks(4) {
                        let mut packed = [0.0; 4];
                        for (target, &id) in packed.iter_mut().zip(ids) {
                            *target = id as f32;
                        }
                        rows.push(packed);
                    }
                    lists.insert(list.to_vec(), row);
                    row * 2
                }
            }
        };
        pixel.copy_from_slice(&value.to_le_bytes());
    }
    GroundMask { indices, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(origin: DVec2, angle: f64, size: DVec2) -> MaskRectangle {
        let u = DVec2::new(angle.cos(), angle.sin()) * size.x;
        let v = DVec2::new(-angle.sin(), angle.cos()) * size.y;
        MaskRectangle {
            corners: [origin, origin + u, origin + u + v, origin + v],
            color_style: [0.2, 0.8, 0.3, 1.0],
        }
    }

    fn has_rectangle(mask: &GroundMask, point: DVec2, row: u32) -> bool {
        let x = (point.x / TEXEL_M as f64).floor() as usize;
        let y = (point.y / TEXEL_M as f64).floor() as usize;
        let offset = (y * MASK_SIDE + x) * 4;
        let value = u32::from_le_bytes(mask.indices[offset..offset + 4].try_into().unwrap());
        if value == 0 {
            return false;
        }
        if value & 1 != 0 {
            return value / 2 == row;
        }
        let first = (value / 2) as usize;
        (0..mask.rows[first][0] as usize).any(|i| mask.rows[first + 1 + i / 4][i % 4] as u32 == row)
    }

    #[test]
    fn rotated_subtexel_cells_and_cross_chunk_parcels_have_no_missing_candidates() {
        let rectangles = [
            rectangle(DVec2::new(0.1, 0.2), 0.37, DVec2::splat(1.0)),
            rectangle(DVec2::new(-15.0, 490.0), -0.7, DVec2::new(80.0, 30.0)),
            rectangle(DVec2::new(4.0, 5.0), 0.3, DVec2::splat(10.0)),
        ];
        let mask = build_mask(DVec2::ZERO, &rectangles);
        for (id, rectangle) in rectangles.iter().enumerate() {
            for y in 0..101 {
                for x in 0..101 {
                    let p = rectangle.corners[0]
                        + (rectangle.corners[1] - rectangle.corners[0]) * (x as f64 / 100.0)
                        + (rectangle.corners[3] - rectangle.corners[0]) * (y as f64 / 100.0);
                    if p.cmpge(DVec2::ZERO).all() && p.cmplt(DVec2::splat(512.0)).all() {
                        assert!(has_rectangle(&mask, p, (id * 3) as u32), "{id} at {p:?}");
                    }
                }
            }
        }
        let repeated = build_mask(DVec2::ZERO, &rectangles);
        assert_eq!(mask.indices, repeated.indices);
        assert_eq!(mask.rows, repeated.rows);
    }

    #[test]
    fn paint_changes_only_analytic_style_rows() {
        let mut rectangles = [rectangle(DVec2::new(2.0, 3.0), 0.2, DVec2::splat(10.0))];
        let before = build_mask(DVec2::ZERO, &rectangles);
        rectangles[0].color_style = [0.2, 0.3, 0.8, 2.0];
        let after = build_mask(DVec2::ZERO, &rectangles);
        assert_eq!(before.indices, after.indices);
        assert_eq!(before.rows[0], after.rows[0]);
        assert_eq!(after.rows[1][2], 2.0);
        assert_eq!(after.rows[2], [0.2, 0.3, 0.8, 0.0]);
    }

    #[test]
    #[ignore = "opt-in release mask resolution measurement; writes METRUM_MASK_BENCH_CSV"]
    fn zoning_mask_resolution_measurement() {
        use std::fmt::Write;
        use std::hint::black_box;
        use std::time::Instant;

        fn measure<const TEXEL: usize>(name: &str, rectangles: &[MaskRectangle], csv: &mut String) {
            let mut samples = Vec::new();
            for iteration in 0..111 {
                let start = Instant::now();
                let mask = build_mask_grid::<TEXEL>(DVec2::ZERO, black_box(rectangles));
                black_box(&mask);
                if iteration >= 10 {
                    samples.push(start.elapsed().as_nanos());
                }
            }
            samples.sort_unstable();
            let mask = build_mask_grid::<TEXEL>(DVec2::ZERO, rectangles);
            let mut nonempty = 0;
            let mut multiple = 0;
            let mut candidates = 0;
            let mut max_candidates = 0;
            for &bytes in mask.indices.as_chunks::<4>().0 {
                let code = u32::from_le_bytes(bytes);
                if code == 0 {
                    continue;
                }
                nonempty += 1;
                let count = if code & 1 != 0 {
                    1
                } else {
                    multiple += 1;
                    mask.rows[(code / 2) as usize][0] as usize
                };
                candidates += count;
                max_candidates = max_candidates.max(count);
            }
            writeln!(
                csv,
                "{name},{TEXEL},{},{},{},{},{},{nonempty},{multiple},{candidates},{max_candidates}",
                rectangles.len(),
                samples[50],
                samples[95],
                mask.indices.len(),
                mask.rows.len() * 16
            )
            .unwrap();
        }
        let mut csv = String::from(
            "fixture,texel_m,rectangles,p50_ns,p95_ns,mask_bytes,row_bytes,nonempty_texels,multi_texels,candidates,max_candidates\n",
        );
        for (name, width, depth, angle) in [
            ("roadside", 50, 6, 0.0_f64),
            ("dense", 50, 50, 0.0),
            ("rotated", 30, 30, 0.37),
            ("hover", 1, 1, 0.37),
        ] {
            let u = DVec2::new(angle.cos(), angle.sin());
            let v = DVec2::new(-angle.sin(), angle.cos());
            let mut rectangles = Vec::new();
            for y in 0..depth {
                for x in 0..width {
                    rectangles.push(rectangle(
                        DVec2::new(if angle == 0.0 { 1.0 } else { 120.0 }, 1.0)
                            + u * (x as f64 * 10.0)
                            + v * (y as f64 * 10.0),
                        angle,
                        DVec2::splat(10.0),
                    ));
                }
            }
            measure::<1>(name, &rectangles, &mut csv);
            measure::<2>(name, &rectangles, &mut csv);
        }
        std::fs::write(
            std::env::var("METRUM_MASK_BENCH_CSV").expect("set benchmark output path"),
            csv,
        )
        .unwrap();
    }
}
