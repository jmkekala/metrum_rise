// SPDX-License-Identifier: GPL-2.0-only

//! Road render chunk frames and chunk-line clipping (`ROAD-68-D`, open questions 23 and 24).
//!
//! A chunk's vertices are local to its centre, the frame of the terrain render patch over it
//! ([`render_frame_centre`]), and are converted from f64 world metres by [`render_frame_local`],
//! as the terrain converts its own, so a road vertex and a terrain vertex at one world point have
//! the same bits. A triangle that crosses chunk lines is clipped at them instead of being routed
//! whole into one chunk: each split point is a function of its edge alone (interpolated from the
//! edge's lexicographically lower end, the line coordinate exact) and each chunk corner inside a
//! triangle a function of that triangle's plane, so both sides of a chunk line take the same
//! vertices, and every vertex two chunks share lies on their common line, exact in both frames.
//! `O(c)` per triangle for the `c` chunks its bounds touch, without allocation.

use super::super::{NetworkMeshChunkPartition, NetworkMeshData};
use crate::simulation::network::roadbed::{RoadVec3, SurfaceChunkKey};
use crate::simulation::terrain::{render_frame_centre, render_frame_local};
use godot::prelude::{Vector2, Vector3, Vector4};

/// One triangle in world metres with its per-vertex attributes.
pub(super) struct WorldTriangle {
    pub(super) points: [RoadVec3; 3],
    pub(super) uvs: [Vector2; 3],
    pub(super) paint: [Vector4; 3],
}

// A clipped polygon vertex: world position, attributes and the triangle edges it lies on (bit
// `i`: vertex `i` to `i + 1`).
#[derive(Clone, Copy, Default)]
struct ClipVertex {
    point: RoadVec3,
    uv: Vector2,
    paint: Vector4,
    edges: u8,
}

// A triangle clipped by four lines has at most seven vertices.
#[derive(Clone, Copy, Default)]
struct ClipPolygon {
    vertices: [ClipVertex; 8],
    len: usize,
}

impl ClipPolygon {
    fn push(&mut self, vertex: ClipVertex) {
        self.vertices[self.len] = vertex;
        self.len += 1;
    }
}

impl NetworkMeshChunkPartition {
    /// World XZ centre of a chunk's frame: its vertices, lights and signs are local to it.
    pub(super) fn frame_centre(&self, chunk: SurfaceChunkKey) -> (f32, f32) {
        let span = self.chunk_span_m;
        (
            render_frame_centre(self.chunk_origin_x_m + chunk.0 as f32 * span, span),
            render_frame_centre(self.chunk_origin_z_m + chunk.1 as f32 * span, span),
        )
    }

    // Chunk index along one axis of a world coordinate.
    fn axis_key(&self, world: f64, origin: f32) -> i32 {
        ((world - f64::from(origin)) / f64::from(self.chunk_span_m)).floor() as i32
    }

    // World coordinate of the chunk line `key` along one axis.
    fn axis_line(&self, key: i32, origin: f32) -> f64 {
        f64::from(origin + key as f32 * self.chunk_span_m)
    }

    /// Calls `emit` with each target chunk's mesh and the triangle's part in it, in that chunk's
    /// frame: the triangle itself when its bounds lie in one chunk, else its clipped part, fanned.
    pub(super) fn route(
        &mut self,
        triangle: &WorldTriangle,
        mut emit: impl FnMut(&mut NetworkMeshData, [Vector3; 3], [Vector2; 3], [Vector4; 3]),
    ) {
        let (origin_x, origin_z) = (self.chunk_origin_x_m, self.chunk_origin_z_m);
        let range = |axis: fn(&RoadVec3) -> f64, origin: f32| {
            let values = triangle.points.map(|point| axis(&point));
            (
                self.axis_key(values.into_iter().fold(f64::INFINITY, f64::min), origin),
                self.axis_key(values.into_iter().fold(f64::NEG_INFINITY, f64::max), origin),
            )
        };
        let (x0, x1) = range(|point| point.x, origin_x);
        let (z0, z1) = range(|point| point.z, origin_z);
        let whole = triangle_polygon(triangle);
        if (x0, z0) == (x1, z1) {
            self.emit_piece((x0, z0), &whole, &mut emit);
            return;
        }
        for kx in x0..=x1 {
            for kz in z0..=z1 {
                if !self.target_chunks.contains(&(kx, kz)) {
                    continue;
                }
                let mut polygon = whole;
                for (axis_x, line, keep_low) in [
                    (true, self.axis_line(kx, origin_x), false),
                    (true, self.axis_line(kx + 1, origin_x), true),
                    (false, self.axis_line(kz, origin_z), false),
                    (false, self.axis_line(kz + 1, origin_z), true),
                ] {
                    polygon = clip(&polygon, axis_x, line, keep_low, triangle);
                    if polygon.len < 3 {
                        break;
                    }
                }
                self.emit_piece((kx, kz), &polygon, &mut emit);
            }
        }
    }

    // Fans `polygon` (in world metres) into the chunk's frame and emits each non-degenerate face.
    fn emit_piece(
        &mut self,
        chunk: SurfaceChunkKey,
        polygon: &ClipPolygon,
        emit: &mut impl FnMut(&mut NetworkMeshData, [Vector3; 3], [Vector2; 3], [Vector4; 3]),
    ) {
        if polygon.len < 3 || !self.target_chunks.contains(&chunk) {
            return;
        }
        let (centre_x, centre_z) = self.frame_centre(chunk);
        let local = |vertex: &ClipVertex| {
            Vector3::new(
                render_frame_local(vertex.point.x, centre_x),
                vertex.point.y as f32,
                render_frame_local(vertex.point.z, centre_z),
            )
        };
        let mesh = self
            .chunks
            .entry(chunk)
            .or_insert_with(NetworkMeshData::new);
        for index in 1..polygon.len - 1 {
            let face = [0, index, index + 1].map(|corner| polygon.vertices[corner]);
            let [a, b, c] = face.map(|vertex| vertex.point);
            if (b - a).cross(c - a) == RoadVec3::ZERO {
                continue;
            }
            emit(
                mesh,
                face.map(|vertex| local(&vertex)),
                face.map(|vertex| vertex.uv),
                face.map(|vertex| vertex.paint),
            );
        }
    }
}

// The triangle as a polygon; vertex `i` lies on edges `i` and `i + 2`.
fn triangle_polygon(triangle: &WorldTriangle) -> ClipPolygon {
    let mut polygon = ClipPolygon::default();
    for index in 0..3 {
        polygon.push(ClipVertex {
            point: triangle.points[index],
            uv: triangle.uvs[index],
            paint: triangle.paint[index],
            edges: (1 << index) | (1 << ((index + 2) % 3)),
        });
    }
    polygon
}

// Keeps the part of `polygon` on one side of a chunk line (closed: a vertex on the line stays on
// both sides); a crossing adds a vertex only where an edge strictly crosses the line.
fn clip(
    polygon: &ClipPolygon,
    axis_x: bool,
    line: f64,
    keep_low: bool,
    triangle: &WorldTriangle,
) -> ClipPolygon {
    let coordinate = |vertex: &ClipVertex| {
        if axis_x {
            vertex.point.x
        } else {
            vertex.point.z
        }
    };
    let mut kept = ClipPolygon::default();
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
                // A run on no triangle edge lies along an x chunk line (x lines are clipped
                // first), and only a z line crosses it: a chunk corner.
                debug_assert!(!axis_x && a.point.x == b.point.x);
                chunk_corner(triangle, a.point.x, line)
            });
        }
    }
    kept
}

// Where triangle edge `edge` crosses a chunk line, interpolated from the edge's lexicographically
// lower end, so every face sharing the edge computes the same point.
fn edge_crossing(triangle: &WorldTriangle, edge: usize, axis_x: bool, line: f64) -> ClipVertex {
    let (i, j) = (edge, (edge + 1) % 3);
    let (p, q) = (triangle.points[i], triangle.points[j]);
    let (low, high) = if (p.x, p.z) <= (q.x, q.z) {
        (i, j)
    } else {
        (j, i)
    };
    let (a, b) = (triangle.points[low], triangle.points[high]);
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
    let weight = t as f32;
    ClipVertex {
        point,
        uv: triangle.uvs[low] + (triangle.uvs[high] - triangle.uvs[low]) * weight,
        paint: triangle.paint[low] + (triangle.paint[high] - triangle.paint[low]) * weight,
        edges: 1 << edge,
    }
}

// A chunk corner inside the triangle, on the triangle's plane.
fn chunk_corner(triangle: &WorldTriangle, x: f64, z: f64) -> ClipVertex {
    let [a, b, c] = triangle.points;
    let area = (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x);
    let wb = ((x - a.x) * (c.z - a.z) - (z - a.z) * (c.x - a.x)) / area;
    let wc = ((b.x - a.x) * (z - a.z) - (b.z - a.z) * (x - a.x)) / area;
    let wa = 1.0 - wb - wc;
    let weights = [wa, wb, wc].map(|weight| weight as f32);
    let blend2 = |values: [Vector2; 3]| {
        values[0] * weights[0] + values[1] * weights[1] + values[2] * weights[2]
    };
    let blend4 = |values: [Vector4; 3]| {
        values[0] * weights[0] + values[1] * weights[1] + values[2] * weights[2]
    };
    ClipVertex {
        point: RoadVec3::new(x, a.y * wa + b.y * wb + c.y * wc, z),
        uv: blend2(triangle.uvs),
        paint: blend4(triangle.paint),
        edges: 0,
    }
}
