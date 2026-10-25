// SPDX-License-Identifier: GPL-2.0-only

//! Canonical clipping of triangles by axis-aligned grid rectangles (`ROAD-68-D`): road render
//! chunks and building foundations clip their triangles to chunk and patch rectangles with it.
//!
//! The rectangle is closed: a vertex on a side belongs to both cells. A vertex is added only
//! where an edge strictly crosses a side, and each such split point is a function of the edge
//! alone (interpolated from its lexicographically lower end, the side coordinate exact), so the
//! two faces sharing an edge, clipped by the same grid in either cell, split it at the same
//! point; a rectangle corner inside a triangle is a function of that triangle's plane. Every
//! vertex two neighbouring cells share therefore lies on their common side with equal bits.
//! `O(1)` per triangle and rectangle, without allocation.

use glam::DVec3;

/// A vertex of a clipped triangle: its position and its barycentric weights in the triangle,
/// with which the caller interpolates its attributes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClipVertex {
    /// World position in metres.
    pub(crate) point: DVec3,
    /// Weights of the triangle's three vertices.
    pub(crate) weights: [f64; 3],
    // The triangle edges the vertex lies on (bit `i`: vertex `i` to `i + 1`).
    edges: u8,
}

// The part of a triangle inside a rectangle: a convex polygon of at most seven vertices.
#[derive(Clone, Copy, Debug, Default)]
struct ClipPolygon {
    vertices: [ClipVertex; 8],
    len: usize,
}

impl ClipPolygon {
    fn push(&mut self, vertex: ClipVertex) {
        self.vertices[self.len] = vertex;
        self.len += 1;
    }

    // Calls `emit` with each face of a fan over the polygon, skipping exactly degenerate ones.
    fn fan(&self, mut emit: impl FnMut([ClipVertex; 3])) {
        for index in 1..self.len.saturating_sub(1) {
            let face = [0, index, index + 1].map(|corner| self.vertices[corner]);
            let [a, b, c] = face.map(|vertex| vertex.point);
            if (b - a).cross(c - a) != DVec3::ZERO {
                emit(face);
            }
        }
    }
}

/// Calls `emit` with each face of a fan over the part of `triangle` inside
/// `rect = [min_x, min_z, max_x, max_z]`, skipping exactly degenerate faces. The passes clip
/// between two buffers in place, so no polygon is copied or returned by value: whether the
/// optimiser removed such copies followed the crate's codegen-unit partition (`ROAD-68-F`).
pub(crate) fn clip_triangle_to_rect(
    triangle: [DVec3; 3],
    rect: [f64; 4],
    emit: impl FnMut([ClipVertex; 3]),
) {
    let mut polygons = [ClipPolygon::default(); 2];
    for index in 0..3 {
        let mut weights = [0.0; 3];
        weights[index] = 1.0;
        polygons[0].push(ClipVertex {
            point: triangle[index],
            weights,
            edges: (1 << index) | (1 << ((index + 2) % 3)),
        });
    }
    let mut current = 0;
    // x sides first: a run on no triangle edge then lies along an x side.
    for (axis_x, line, keep_low) in [
        (true, rect[0], false),
        (true, rect[2], true),
        (false, rect[1], false),
        (false, rect[3], true),
    ] {
        let [even, odd] = &mut polygons;
        let (polygon, kept) = if current == 0 {
            (&*even, odd)
        } else {
            (&*odd, even)
        };
        clip(polygon, kept, axis_x, line, keep_low, triangle);
        current ^= 1;
        if polygons[current].len < 3 {
            return;
        }
    }
    polygons[current].fan(emit);
}

// Writes into `kept` the part of `polygon` on one side of a line.
fn clip(
    polygon: &ClipPolygon,
    kept: &mut ClipPolygon,
    axis_x: bool,
    line: f64,
    keep_low: bool,
    triangle: [DVec3; 3],
) {
    let coordinate = |vertex: &ClipVertex| {
        if axis_x {
            vertex.point.x
        } else {
            vertex.point.z
        }
    };
    kept.len = 0;
    for index in 0..polygon.len {
        let a = polygon.vertices[index];
        let b = polygon.vertices[(index + 1) % polygon.len];
        let (ca, cb) = (coordinate(&a), coordinate(&b));
        if (keep_low && ca <= line) || (!keep_low && ca >= line) {
            kept.push(a);
        }
        if (ca < line && cb > line) || (ca > line && cb < line) {
            let common = a.edges & b.edges;
            kept.push(if common != 0 {
                edge_crossing(triangle, common.trailing_zeros() as usize, axis_x, line)
            } else {
                // A run on no triangle edge lies along an x side, and only a z side crosses
                // it: a rectangle corner.
                debug_assert!(!axis_x && a.point.x == b.point.x);
                corner(triangle, a.point.x, line)
            });
        }
    }
}

// Where triangle edge `edge` crosses a line, from the edge's lexicographically lower end.
fn edge_crossing(triangle: [DVec3; 3], edge: usize, axis_x: bool, line: f64) -> ClipVertex {
    let (i, j) = (edge, (edge + 1) % 3);
    let (low, high) = if (triangle[i].x, triangle[i].z) <= (triangle[j].x, triangle[j].z) {
        (i, j)
    } else {
        (j, i)
    };
    let (a, b) = (triangle[low], triangle[high]);
    let t = if axis_x {
        (line - a.x) / (b.x - a.x)
    } else {
        (line - a.z) / (b.z - a.z)
    };
    let mut point = a + (b - a) * t;
    if axis_x {
        point.x = line;
    } else {
        point.z = line;
    }
    let mut weights = [0.0; 3];
    weights[low] = 1.0 - t;
    weights[high] = t;
    ClipVertex {
        point,
        weights,
        edges: 1 << edge,
    }
}

// A rectangle corner inside the triangle, on the triangle's plane.
fn corner(triangle: [DVec3; 3], x: f64, z: f64) -> ClipVertex {
    let [a, b, c] = triangle;
    let area = (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x);
    let wb = ((x - a.x) * (c.z - a.z) - (z - a.z) * (c.x - a.x)) / area;
    let wc = ((b.x - a.x) * (z - a.z) - (b.z - a.z) * (x - a.x)) / area;
    let wa = 1.0 - wb - wc;
    ClipVertex {
        point: DVec3::new(x, a.y * wa + b.y * wb + c.y * wc, z),
        weights: [wa, wb, wc],
        edges: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two triangles sharing an edge across a grid line, clipped in both cells, split the edge at
    // one point and cover each cell without a gap: every edge of the pieces is shared, bitwise,
    // or lies on the triangles' outline.
    #[test]
    fn neighbouring_cells_split_a_shared_edge_alike() {
        let p = [
            DVec3::new(-0.3, 1.7, -0.1),
            DVec3::new(2.9, 0.4, 0.37),
            DVec3::new(0.11, 2.2, 3.3),
            DVec3::new(3.7, 1.1, 2.9),
        ];
        // Clockwise and counter-clockwise neighbours across the edge p[1]-p[2].
        let triangles = [[p[0], p[1], p[2]], [p[2], p[1], p[3]]];
        let mut edges = std::collections::BTreeMap::<_, usize>::new();
        let key = |v: DVec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        for triangle in triangles {
            for rect in [
                [-1.0, -1.0, 1.0, 1.0],
                [1.0, -1.0, 4.0, 1.0],
                [-1.0, 1.0, 1.0, 4.0],
                [1.0, 1.0, 4.0, 4.0],
            ] {
                clip_triangle_to_rect(triangle, rect, |face| {
                    for (u, v) in [(0, 1), (1, 2), (2, 0)] {
                        let (u, v) = (key(face[u].point), key(face[v].point));
                        *edges.entry((u.min(v), u.max(v))).or_default() += 1;
                    }
                });
            }
        }
        let on_outline = |a: [u64; 3], b: [u64; 3]| {
            let [a, b] =
                [a, b].map(|k| DVec3::new(f64::from_bits(k[0]), 0.0, f64::from_bits(k[2])));
            [(p[0], p[1]), (p[1], p[3]), (p[3], p[2]), (p[2], p[0])]
                .iter()
                .any(|&(s, e)| {
                    let side = |q: DVec3| (e.x - s.x) * (q.z - s.z) - (e.z - s.z) * (q.x - s.x);
                    side(a).abs() < 1e-9 && side(b).abs() < 1e-9
                })
        };
        for ((a, b), count) in &edges {
            assert!(
                *count == 2 || (*count == 1 && on_outline(*a, *b)),
                "edge {a:?}-{b:?} shared by {count} faces"
            );
        }
    }
}
