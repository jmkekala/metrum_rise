// SPDX-License-Identifier: GPL-2.0-only

//! Road render chunk frames and chunk-line clipping (`ROAD-68-D`, open questions 23 and 24).
//!
//! A chunk's vertices are local to its centre, the frame of the terrain render patch over it
//! ([`render_frame_centre`]), and are converted from f64 world metres by [`render_frame_local`],
//! as the terrain converts its own, so a road vertex and a terrain vertex at one world point have
//! the same bits. A triangle that crosses chunk lines is clipped at them
//! ([`clip_triangle_to_rect`]) instead of being routed whole into one chunk, so both sides of a
//! chunk line take the same vertices, and every vertex two chunks share lies on their common
//! line, exact in both frames. `O(c)` per triangle for the `c` chunks its bounds touch, without
//! allocation.

use super::super::{NetworkMeshChunkPartition, NetworkMeshData};
use crate::simulation::network::roadbed::{RoadVec3, SurfaceChunkKey};
use crate::simulation::terrain::rect_clip::{ClipVertex, clip_triangle_to_rect};
use crate::simulation::terrain::{render_frame_centre, render_frame_local};
use godot::prelude::{Vector2, Vector3, Vector4};

/// One triangle in world metres with its per-vertex attributes.
pub(super) struct WorldTriangle {
    pub(super) points: [RoadVec3; 3],
    pub(super) uvs: [Vector2; 3],
    pub(super) paint: [Vector4; 3],
}

impl WorldTriangle {
    // Attributes at a clipped vertex; a triangle vertex keeps its own exactly.
    fn uv(&self, vertex: &ClipVertex) -> Vector2 {
        let w = vertex.weights.map(|weight| weight as f32);
        self.uvs[0] * w[0] + self.uvs[1] * w[1] + self.uvs[2] * w[2]
    }

    fn paint(&self, vertex: &ClipVertex) -> Vector4 {
        let w = vertex.weights.map(|weight| weight as f32);
        self.paint[0] * w[0] + self.paint[1] * w[1] + self.paint[2] * w[2]
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
        // Most triangles lie in one chunk: no clip, the same output as clipping by its rectangle.
        if (x0, z0) == (x1, z1) {
            let chunk = (x0, z0);
            if !self.target_chunks.contains(&chunk) {
                return;
            }
            let (centre_x, centre_z) = self.frame_centre(chunk);
            let local = triangle.points.map(|point| {
                Vector3::new(
                    render_frame_local(point.x, centre_x),
                    point.y as f32,
                    render_frame_local(point.z, centre_z),
                )
            });
            let [a, b, c] = triangle.points;
            if (b - a).cross(c - a) != RoadVec3::ZERO {
                emit(
                    self.chunks.entry(chunk).or_default(),
                    local,
                    triangle.uvs,
                    triangle.paint,
                );
            }
            return;
        }
        for kx in x0..=x1 {
            for kz in z0..=z1 {
                let chunk = (kx, kz);
                if !self.target_chunks.contains(&chunk) {
                    continue;
                }
                let (centre_x, centre_z) = self.frame_centre(chunk);
                let local = |vertex: &ClipVertex| {
                    Vector3::new(
                        render_frame_local(vertex.point.x, centre_x),
                        vertex.point.y as f32,
                        render_frame_local(vertex.point.z, centre_z),
                    )
                };
                let rect = [
                    self.axis_line(kx, origin_x),
                    self.axis_line(kz, origin_z),
                    self.axis_line(kx + 1, origin_x),
                    self.axis_line(kz + 1, origin_z),
                ];
                let polygon = clip_triangle_to_rect(triangle.points, rect);
                let mesh = self.chunks.entry(chunk).or_default();
                polygon.fan(|face| {
                    emit(
                        mesh,
                        face.map(|vertex| local(&vertex)),
                        face.map(|vertex| triangle.uv(&vertex)),
                        face.map(|vertex| triangle.paint(&vertex)),
                    );
                });
            }
        }
    }
}
