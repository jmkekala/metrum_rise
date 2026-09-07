// SPDX-License-Identifier: GPL-2.0-only

//! Exact display-triangle contacts with the resident terrain cutout boundary.

use super::NetworkMeshData;
use crate::simulation::network::surface::RoadVec3;
use godot::prelude::{Color, Vector2, Vector3};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct Vertex {
    point: Vector3,
    normal: Vector3,
    uv: Vector2,
    color: Color,
}

struct ContactPlane {
    boundary: [RoadVec3; 2],
    distances: [f64; 3],
    single_contact: Option<(u32, u32)>,
}

impl Vertex {
    fn between(a: Self, b: Self, t: f64, point: Vector3) -> Self {
        Self {
            point,
            normal: a.normal.lerp(b.normal, t as f32),
            uv: a.uv.lerp(b.uv, t as f32),
            // Color::lerp calls Godot's builtin API. Geometry must remain engine-free.
            color: Color::from_rgba(
                a.color.r + (b.color.r - a.color.r) * t as f32,
                a.color.g + (b.color.g - a.color.g) * t as f32,
                a.color.b + (b.color.b - a.color.b) * t as f32,
                a.color.a + (b.color.a - a.color.a) * t as f32,
            ),
        }
    }
}

fn key(point: Vector3) -> (u32, u32) {
    (point.x.to_bits(), point.z.to_bits())
}

/// Inserts resident cutout contacts into display triangles without changing their source positions.
pub(super) fn split_mesh(
    mesh: &mut NetworkMeshData,
    offsets: &mut HashMap<(u32, u32), f32>,
    boundaries: &[[RoadVec3; 2]],
    origin_x: f32,
    origin_z: f32,
) {
    // Filter the local cutout boundary once per chunk, using its existing XZ-offset keys.
    // Fully lifted triangles can also enclose a cutout; vertex-only overlap is insufficient.
    let (mut min_x, mut min_z, mut max_x, mut max_z) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &(x, z) in offsets.keys() {
        let x = f64::from(f32::from_bits(x)) + f64::from(origin_x);
        let z = f64::from(f32::from_bits(z)) + f64::from(origin_z);
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_z = min_z.min(z);
        max_z = max_z.max(z);
    }
    let boundaries: Vec<_> = boundaries
        .iter()
        .filter(|[a, b]| {
            a.x.min(b.x) <= max_x
                && a.x.max(b.x) >= min_x
                && a.z.min(b.z) <= max_z
                && a.z.max(b.z) >= min_z
        })
        .map(|&boundary @ [a, b]| (boundary, a.min(b), a.max(b)))
        .collect();
    // Source positions remain unchanged. Add exact collinear seam breakpoints as well as
    // line crossings: otherwise differing edge tessellation creates raster T-junction cracks.
    let mut triangles = Vec::new();
    let mut next = Vec::new();
    macro_rules! layer {
        ($vertices:ident, $normals:ident, $uvs:ident, $colors:ident) => {{
            let original_len = mesh.$vertices.len();
            for start in (0..original_len).step_by(3) {
                let triangle: [Vertex; 3] = std::array::from_fn(|i| Vertex {
                    point: mesh.$vertices[start + i],
                    normal: mesh.$normals[start + i],
                    uv: mesh.$uvs[start + i],
                    color: mesh.$colors[start + i],
                });
                let points = triangle.map(|vertex| {
                    RoadVec3::new(
                        f64::from(vertex.point.x) + f64::from(origin_x),
                        f64::from(vertex.point.y),
                        f64::from(vertex.point.z) + f64::from(origin_z),
                    )
                });
                let min = points[0].min(points[1]).min(points[2]);
                let max = points[0].max(points[1]).max(points[2]);
                triangles.clear();
                triangles.push(triangle);
                for &(boundary, boundary_min, boundary_max) in &boundaries {
                    // Every piece remains inside its source triangle. Reject unrelated local
                    // segments once here, before visiting or reconstructing any of its pieces.
                    if max.x < boundary_min.x
                        || min.x > boundary_max.x
                        || max.z < boundary_min.z
                        || min.z > boundary_max.z
                    {
                        continue;
                    }
                    next.clear();
                    for &piece in &triangles {
                        split_triangle(piece, boundary, offsets, origin_x, origin_z, &mut next);
                    }
                    std::mem::swap(&mut triangles, &mut next);
                }
                for (index, piece) in triangles.iter().enumerate() {
                    for (i, vertex) in piece.iter().enumerate() {
                        if index == 0 {
                            mesh.$vertices[start + i] = vertex.point;
                            mesh.$normals[start + i] = vertex.normal;
                            mesh.$uvs[start + i] = vertex.uv;
                            mesh.$colors[start + i] = vertex.color;
                        } else {
                            mesh.$vertices.push(vertex.point);
                            mesh.$normals.push(vertex.normal);
                            mesh.$uvs.push(vertex.uv);
                            mesh.$colors.push(vertex.color);
                        }
                    }
                }
            }
        }};
    }
    layer!(
        earthwork_vertices,
        earthwork_normals,
        earthwork_uvs,
        earthwork_colors
    );
    layer!(curb_vertices, curb_normals, curb_uvs, curb_colors);
    layer!(
        raised_step_vertices,
        raised_step_normals,
        raised_step_uvs,
        raised_step_colors
    );
    layer!(
        sidewalk_vertices,
        sidewalk_normals,
        sidewalk_uvs,
        sidewalk_colors
    );
    layer!(road_vertices, road_normals, road_uvs, road_colors);
    layer!(
        marking_vertices,
        marking_normals,
        marking_uvs,
        marking_colors
    );
    layer!(
        concrete_vertices,
        concrete_normals,
        concrete_uvs,
        concrete_colors
    );
    // Planned meshes are display-only, not source-owner caches. Splitting invalidates the
    // original ranges; never leave misleading ownership attached to the new triangle order.
    mesh.owner_ranges = Default::default();
}

fn split_triangle(
    triangle: [Vertex; 3],
    boundary: [RoadVec3; 2],
    offsets: &mut HashMap<(u32, u32), f32>,
    origin_x: f32,
    origin_z: f32,
    output: &mut Vec<[Vertex; 3]>,
) {
    let world = |point: Vector3| {
        RoadVec3::new(
            f64::from(point.x) + f64::from(origin_x),
            f64::from(point.y),
            f64::from(point.z) + f64::from(origin_z),
        )
    };
    let points = triangle.map(|vertex| world(vertex.point));
    let min = points[0].min(points[1]).min(points[2]);
    let max = points[0].max(points[1]).max(points[2]);
    if max.x < boundary[0].x.min(boundary[1].x)
        || min.x > boundary[0].x.max(boundary[1].x)
        || max.z < boundary[0].z.min(boundary[1].z)
        || min.z > boundary[0].z.max(boundary[1].z)
    {
        output.push(triangle);
        return;
    }
    for point in boundary {
        for i in 0..3 {
            let j = (i + 1) % 3;
            let (a, b) = if key(triangle[i].point) < key(triangle[j].point) {
                (i, j)
            } else {
                (j, i)
            };
            let edge = points[b] - points[a];
            let relative = point - points[a];
            let length_squared = edge.x * edge.x + edge.z * edge.z;
            // Only true collinear points are inserted; this is not boundary snapping.
            if length_squared == 0.0 || relative.x * edge.z - relative.z * edge.x != 0.0 {
                continue;
            }
            let t = (relative.x * edge.x + relative.z * edge.z) / length_squared;
            if t <= 0.0 || t >= 1.0 {
                continue;
            }
            let local = Vector3::new(
                (point.x - f64::from(origin_x)) as f32,
                (points[a].y + (points[b].y - points[a].y) * t) as f32,
                (point.z - f64::from(origin_z)) as f32,
            );
            if key(local) == key(triangle[a].point) || key(local) == key(triangle[b].point) {
                continue;
            }
            // Cut through the breakpoint across the whole face, including both sides of a
            // vertical triangle. No recursive reinsertion at float-rounded endpoints.
            let distances = points.map(|p| edge.x * (p.x - point.x) + edge.z * (p.z - point.z));
            let mut pieces = [triangle; 3];
            let mut count = 0;
            clip_triangle(
                triangle,
                points,
                ContactPlane {
                    boundary,
                    distances,
                    single_contact: Some(key(local)),
                },
                offsets,
                origin_x,
                origin_z,
                &mut |piece| {
                    pieces[count] = piece;
                    count += 1;
                },
            );
            // The perpendicular cut adds the breakpoint; the actual cutout plane must still
            // split every resulting piece (including triangles enclosing an old footprint).
            let delta = boundary[1] - boundary[0];
            for &piece in &pieces[..count] {
                let points = piece.map(|vertex| world(vertex.point));
                let distances = points
                    .map(|p| delta.x * (p.z - boundary[0].z) - delta.z * (p.x - boundary[0].x));
                clip_triangle(
                    piece,
                    points,
                    ContactPlane {
                        boundary,
                        distances,
                        single_contact: None,
                    },
                    offsets,
                    origin_x,
                    origin_z,
                    &mut |triangle| output.push(triangle),
                );
            }
            return;
        }
    }
    if triangle
        .iter()
        .all(|vertex| offsets[&key(vertex.point)] == 0.0)
    {
        output.push(triangle);
        return;
    }
    let delta = boundary[1] - boundary[0];
    let distance =
        |point: RoadVec3| delta.x * (point.z - boundary[0].z) - delta.z * (point.x - boundary[0].x);
    let distances = points.map(distance);
    clip_triangle(
        triangle,
        points,
        ContactPlane {
            boundary,
            distances,
            single_contact: None,
        },
        offsets,
        origin_x,
        origin_z,
        &mut |triangle| output.push(triangle),
    );
}

fn clip_triangle(
    triangle: [Vertex; 3],
    points: [RoadVec3; 3],
    plane: ContactPlane,
    offsets: &mut HashMap<(u32, u32), f32>,
    origin_x: f32,
    origin_z: f32,
    output: &mut impl FnMut([Vertex; 3]),
) {
    let ContactPlane {
        boundary,
        distances,
        single_contact,
    } = plane;
    let delta = boundary[1] - boundary[0];
    if single_contact.is_none() {
        let length_squared = delta.x * delta.x + delta.z * delta.z;
        if length_squared > 0.0 {
            for (point, vertex) in points.iter().zip(&triangle) {
                let relative = *point - boundary[0];
                let along = (relative.x * delta.x + relative.z * delta.z) / length_squared;
                if delta.x * relative.z - delta.z * relative.x == 0.0
                    && (0.0..=1.0).contains(&along)
                {
                    offsets.insert(key(vertex.point), 0.0);
                }
            }
        }
    }
    if !distances.iter().any(|d| *d < 0.0) || !distances.iter().any(|d| *d > 0.0) {
        output(triangle);
        return;
    }
    for side in [-1.0, 1.0] {
        // A clipped triangle is at most a quad, including vertical curb/support faces.
        let mut polygon = [triangle[0]; 4];
        let mut len = 0;
        for i in 0..3 {
            let j = (i + 1) % 3;
            if distances[i] * side >= 0.0 {
                polygon[len] = triangle[i];
                len += 1;
            }
            if (distances[i] < 0.0 && distances[j] > 0.0)
                || (distances[i] > 0.0 && distances[j] < 0.0)
            {
                // Canonical endpoint order makes shared-edge intersections bit-identical.
                let (a, b) = if key(triangle[i].point) < key(triangle[j].point) {
                    (i, j)
                } else {
                    (j, i)
                };
                let t = distances[a] / (distances[a] - distances[b]);
                let point = points[a] + (points[b] - points[a]) * t;
                let local = Vector3::new(
                    (point.x - f64::from(origin_x)) as f32,
                    point.y as f32,
                    (point.z - f64::from(origin_z)) as f32,
                );
                let anchored = if let Some(contact) = single_contact {
                    key(local) == contact
                } else {
                    let along = ((point.x - boundary[0].x) * delta.x
                        + (point.z - boundary[0].z) * delta.z)
                        / (delta.x * delta.x + delta.z * delta.z);
                    (0.0..=1.0).contains(&along)
                };
                let offset = if anchored {
                    0.0
                } else {
                    offsets[&key(triangle[a].point)]
                        + (offsets[&key(triangle[b].point)] - offsets[&key(triangle[a].point)])
                            * t as f32
                };
                offsets
                    .entry(key(local))
                    .and_modify(|value| *value = value.min(offset))
                    .or_insert(offset);
                polygon[len] = Vertex::between(triangle[a], triangle[b], t, local);
                len += 1;
            }
        }
        for i in 1..len.saturating_sub(1) {
            output([polygon[0], polygon[i], polygon[i + 1]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlifted_edge_preserves_collinear_terrain_breakpoints() {
        let points = [
            Vector3::new(0.0, 0.12, 3.65),
            Vector3::new(1.5, 0.12, 5.0),
            Vector3::new(0.0, 0.12, 5.0),
        ];
        let triangle = points.map(|point| Vertex {
            point,
            normal: Vector3::UP,
            uv: Vector2::new(point.x, point.z),
            color: Color::WHITE,
        });
        let mut offsets = points.into_iter().map(|point| (key(point), 0.0)).collect();
        let mut result = Vec::new();
        split_triangle(
            triangle,
            [
                RoadVec3::new(0.15, 0.12, 5.0),
                RoadVec3::new(1.5, 0.12, 5.0),
            ],
            &mut offsets,
            0.0,
            0.0,
            &mut result,
        );
        assert_eq!(result.len(), 3);
        assert!(
            result
                .iter()
                .flatten()
                .any(|vertex| vertex.point == Vector3::new(0.15, 0.12, 5.0))
        );
        assert!(
            result
                .iter()
                .flatten()
                .all(|vertex| offsets[&key(vertex.point)] == 0.0)
        );
    }

    #[test]
    fn lifted_triangle_enclosing_a_cutout_gets_anchored_interior() {
        let mut mesh = NetworkMeshData::new();
        mesh.road_vertices = vec![
            Vector3::new(-10.0, 0.0, -10.0),
            Vector3::new(10.0, 0.0, -10.0),
            Vector3::new(0.0, 0.0, 10.0),
        ];
        mesh.road_normals = vec![Vector3::UP; 3];
        mesh.road_uvs = vec![Vector2::ZERO; 3];
        mesh.road_colors = vec![Color::WHITE; 3];
        let mut offsets = mesh
            .road_vertices
            .iter()
            .map(|&point| (key(point), 0.15))
            .collect();
        let ring = [
            RoadVec3::new(-1.0, 0.0, -1.0),
            RoadVec3::new(1.0, 0.0, -1.0),
            RoadVec3::new(1.0, 0.0, 1.0),
            RoadVec3::new(-1.0, 0.0, 1.0),
        ];
        let boundaries = std::array::from_fn::<_, 4, _>(|i| [ring[i], ring[(i + 1) % 4]]);
        split_mesh(&mut mesh, &mut offsets, &boundaries, 0.0, 0.0);
        let mut interior = 0;
        for triangle in mesh.road_vertices.chunks_exact(3) {
            let center = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
            if center.x.abs() < 1.0 && center.z.abs() < 1.0 {
                assert!(triangle.iter().all(|&point| offsets[&key(point)] == 0.0));
                interior += 1;
            }
        }
        assert!(interior > 0);
    }

    #[test]
    fn crossing_top_and_vertical_faces_get_exact_unlifted_contacts() {
        for points in [
            [
                Vector3::new(-1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, 1.0),
            ],
            [
                Vector3::new(-1.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(1.0, 0.15, 0.0),
            ],
        ] {
            let triangle = points.map(|point| Vertex {
                point,
                normal: Vector3::UP,
                uv: Vector2::new(point.x, point.y),
                color: Color::WHITE,
            });
            let mut offsets = points
                .into_iter()
                .map(|point| (key(point), if point.x < 0.0 { 0.0 } else { 0.15 }))
                .collect();
            let mut result = Vec::new();
            split_triangle(
                triangle,
                [RoadVec3::new(0.0, 0.0, -2.0), RoadVec3::new(0.0, 0.0, 2.0)],
                &mut offsets,
                0.0,
                0.0,
                &mut result,
            );
            assert_eq!(result.len(), 3);
            let mut contacts = 0;
            for vertex in result.iter().flatten() {
                if vertex.point.x == 0.0 {
                    assert_eq!(offsets[&key(vertex.point)], 0.0);
                    assert_eq!(vertex.uv, Vector2::new(vertex.point.x, vertex.point.y));
                    contacts += 1;
                }
            }
            assert!(contacts >= 3);
        }
    }
}
