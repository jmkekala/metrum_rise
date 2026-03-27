//! Road mesh generation for lanes, sidewalks, junction footprints, and bridge structures.
//!
//! [`RoadRenderer`] converts the graph-level road representation into triangle soup for Godot.
//! Junctions own their full local envelope: edge asphalt stops at the node's road handoff,
//! edge sidewalks stop at the node's outer handoff, and the node-owned footprint fills the
//! asphalt and sidewalk band between those two boundaries.

use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use godot::prelude::*;
use std::collections::HashMap;

use super::{NetworkMeshData, TransitRenderer};

const JUNCTION_SIDEWALK_BIAS: f32 = 0.0005;
const JUNCTION_ROAD_BIAS: f32 = 0.001;
const MIN_EDGE_REMAINDER: f32 = 0.5;
const MIN_JUNCTION_RADIUS: f32 = 0.5;

#[derive(Clone, Copy, Default)]
struct EndpointTrim {
    start_handoff: f32,
    end_handoff: f32,
}

#[derive(Clone, Copy)]
struct SegmentSample {
    point: Vector3,
    tangent: Vector3,
}

#[derive(Clone, Copy)]
struct EndpointFrame2d {
    node_xz: Vector2,
    outward: Vector2,
    side: Vector2,
}

#[derive(Clone, Copy)]
struct JunctionCorridor {
    handoff_dist: f32,
    tangent_xz: Vector2,
    road_half: f32,
    outer_half: f32,
}

struct JunctionFootprint {
    road_triangles: Vec<[Vector3; 3]>,
    sidewalk_triangles: Vec<TexturedTriangle>,
    road_loops: Vec<Vec<Vector3>>,
    outer_loops: Vec<Vec<Vector3>>,
}

struct TexturedTriangle {
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
}

fn normalize_angle(angle: f32) -> f32 {
    angle.rem_euclid(std::f32::consts::TAU)
}

fn polygon_signed_area_2d(boundary: &[Vector2]) -> f32 {
    let mut area = 0.0f32;
    for i in 0..boundary.len() {
        let a = boundary[i];
        let b = boundary[(i + 1) % boundary.len()];
        area += a.x * b.y - a.y * b.x;
    }
    area * 0.5
}

fn remove_collinear_loop2(boundary: &mut Vec<Vector2>) {
    loop {
        if boundary.len() < 3 {
            return;
        }

        let mut removed_any = false;
        for i in 0..boundary.len() {
            let prev = boundary[(i + boundary.len() - 1) % boundary.len()];
            let current = boundary[i];
            let next = boundary[(i + 1) % boundary.len()];
            let cross = (current.x - prev.x) * (next.y - current.y)
                - (current.y - prev.y) * (next.x - current.x);
            if cross.abs() <= 0.0001 {
                boundary.remove(i);
                removed_any = true;
                break;
            }
        }

        if !removed_any {
            return;
        }
    }
}

fn segment_intersection_parameters(
    a0: Vector2,
    a1: Vector2,
    b0: Vector2,
    b1: Vector2,
) -> Option<(f32, f32)> {
    let da = a1 - a0;
    let db = b1 - b0;
    let denom = da.x * db.y - da.y * db.x;
    if denom.abs() <= 1e-6 {
        return None;
    }

    let rel = b0 - a0;
    let ta = (rel.x * db.y - rel.y * db.x) / denom;
    let tb = (rel.x * da.y - rel.y * da.x) / denom;
    if (-0.0001..=1.0001).contains(&ta) && (-0.0001..=1.0001).contains(&tb) {
        Some((ta.clamp(0.0, 1.0), tb.clamp(0.0, 1.0)))
    } else {
        None
    }
}

fn dedup_sorted_scalars(values: &mut Vec<f32>) {
    values.sort_by(|lhs, rhs| lhs.partial_cmp(rhs).unwrap_or(std::cmp::Ordering::Equal));
    values.dedup_by(|lhs, rhs| (*lhs - *rhs).abs() <= 0.0001);
}

fn corridor_polygon(corridor: &JunctionCorridor, outer: bool) -> Vec<Vector2> {
    let tangent = corridor.tangent_xz;
    let normal = Vector2::new(-tangent.y, tangent.x);
    let half_width = if outer {
        corridor.outer_half
    } else {
        corridor.road_half
    };
    let end = tangent * corridor.handoff_dist;
    vec![
        -normal * half_width,
        end - normal * half_width,
        end + normal * half_width,
        normal * half_width,
    ]
}

fn loop_to_world_xz(loop_points: &[Vector2], node_pos: Vector3, y: f32) -> Vec<Vector3> {
    loop_points
        .iter()
        .map(|point| Vector3::new(node_pos.x + point.x, y, node_pos.z + point.y))
        .collect()
}

fn segment_intersection_point(
    a0: Vector2,
    a1: Vector2,
    b0: Vector2,
    b1: Vector2,
) -> Option<Vector2> {
    segment_intersection_parameters(a0, a1, b0, b1).map(|(ta, _)| a0 + (a1 - a0) * ta)
}

fn remove_duplicate_adjacent_points(boundary: &mut Vec<Vector2>) {
    loop {
        if boundary.len() < 2 {
            return;
        }

        let mut removed_any = false;
        for i in 0..boundary.len() {
            let next = (i + 1) % boundary.len();
            if boundary[i].distance_to(boundary[next]) <= 0.0001 {
                boundary.remove(next);
                removed_any = true;
                break;
            }
        }

        if !removed_any {
            return;
        }
    }
}

fn radial_extent_on_polygon(polygon: &[Vector2], angle: f32) -> Option<f32> {
    let ray_dir = Vector2::new(angle.cos(), angle.sin());
    let mut max_t = None;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let Some(t) = ray_segment_intersection_distance_vec2(Vector2::ZERO, ray_dir, a, b) else {
            continue;
        };
        max_t = Some(max_t.map_or(t, |current: f32| current.max(t)));
    }
    max_t.filter(|t| *t > 0.0001)
}

fn build_union_boundary_from_polygons(polygons: &[Vec<Vector2>]) -> Option<Vec<Vector2>> {
    let mut angles = Vec::new();
    for polygon in polygons {
        if polygon.len() < 3 {
            continue;
        }
        for &point in polygon {
            if point.length_squared() > 1e-6 {
                angles.push(normalize_angle(point.y.atan2(point.x)));
            }
        }
    }

    for i in 0..polygons.len() {
        let polygon_a = &polygons[i];
        if polygon_a.len() < 3 {
            continue;
        }
        for j in (i + 1)..polygons.len() {
            let polygon_b = &polygons[j];
            if polygon_b.len() < 3 {
                continue;
            }

            for edge_a in 0..polygon_a.len() {
                let a0 = polygon_a[edge_a];
                let a1 = polygon_a[(edge_a + 1) % polygon_a.len()];
                for edge_b in 0..polygon_b.len() {
                    let b0 = polygon_b[edge_b];
                    let b1 = polygon_b[(edge_b + 1) % polygon_b.len()];
                    let Some(point) = segment_intersection_point(a0, a1, b0, b1) else {
                        continue;
                    };
                    if point.length_squared() > 1e-6 {
                        angles.push(normalize_angle(point.y.atan2(point.x)));
                    }
                }
            }
        }
    }

    dedup_sorted_scalars(&mut angles);
    if angles.len() < 3 {
        return None;
    }

    let mut boundary = Vec::with_capacity(angles.len());
    for angle in angles {
        let extent = polygons
            .iter()
            .filter_map(|polygon| radial_extent_on_polygon(polygon, angle))
            .max_by(|lhs, rhs| lhs.partial_cmp(rhs).unwrap_or(std::cmp::Ordering::Equal))?;
        let ray_dir = Vector2::new(angle.cos(), angle.sin());
        boundary.push(ray_dir * extent);
    }

    remove_duplicate_adjacent_points(&mut boundary);
    remove_collinear_loop2(&mut boundary);
    if boundary.len() < 3 {
        return None;
    }

    let area = polygon_signed_area_2d(&boundary);
    if area.abs() <= 0.001 {
        return None;
    }
    if area < 0.0 {
        boundary.reverse();
    }
    Some(boundary)
}

fn ray_segment_intersection_distance_vec2(
    origin: Vector2,
    ray_dir: Vector2,
    a: Vector2,
    b: Vector2,
) -> Option<f32> {
    let edge = b - a;
    let rel_a = a - origin;
    let denom = ray_dir.x * edge.y - ray_dir.y * edge.x;
    if denom.abs() <= 1e-6 {
        return None;
    }

    let t = (rel_a.x * edge.y - rel_a.y * edge.x) / denom;
    let u = (rel_a.x * ray_dir.y - rel_a.y * ray_dir.x) / denom;
    if t >= -0.0001 && (-0.0001..=1.0001).contains(&u) {
        Some(t.max(0.0))
    } else {
        None
    }
}

fn max_outward_extent_in_band(
    boundaries: &[Vec<Vector3>],
    frame: EndpointFrame2d,
    band_min: f32,
    band_max: f32,
) -> Option<f32> {
    let (band_min, band_max) = if band_min <= band_max {
        (band_min, band_max)
    } else {
        (band_max, band_min)
    };
    let epsilon = 0.0001;
    let mut max_u = None;

    for boundary in boundaries {
        if boundary.len() < 2 {
            continue;
        }

        for i in 0..boundary.len() {
            let a = boundary[i];
            let b = boundary[(i + 1) % boundary.len()];
            let a_local = Vector2::new(a.x - frame.node_xz.x, a.z - frame.node_xz.y);
            let b_local = Vector2::new(b.x - frame.node_xz.x, b.z - frame.node_xz.y);
            let u0 = a_local.dot(frame.outward);
            let u1 = b_local.dot(frame.outward);
            let v0 = a_local.dot(frame.side);
            let v1 = b_local.dot(frame.side);

            let mut consider = |u: f32| {
                if u <= epsilon {
                    return;
                }
                max_u = Some(max_u.map_or(u, |current: f32| current.max(u)));
            };

            if (band_min - epsilon..=band_max + epsilon).contains(&v0) {
                consider(u0);
            }
            if (band_min - epsilon..=band_max + epsilon).contains(&v1) {
                consider(u1);
            }

            if (v1 - v0).abs() <= epsilon {
                continue;
            }

            for target_v in [band_min, band_max] {
                let min_v = v0.min(v1) - epsilon;
                let max_v = v0.max(v1) + epsilon;
                if !(min_v..=max_v).contains(&target_v) {
                    continue;
                }

                let t = (target_v - v0) / (v1 - v0);
                if !(-epsilon..=1.0 + epsilon).contains(&t) {
                    continue;
                }

                let u = u0 + (u1 - u0) * t.clamp(0.0, 1.0);
                consider(u);
            }
        }
    }

    max_u
}

fn collect_loop_angles(loop_points: &[Vector2], angles: &mut Vec<f32>) {
    for &point in loop_points {
        if point.length_squared() > 1e-6 {
            angles.push(normalize_angle(point.y.atan2(point.x)));
        }
    }
}

fn point_on_polygon_at_angle(polygon: &[Vector2], angle: f32) -> Option<Vector2> {
    let extent = radial_extent_on_polygon(polygon, angle)?;
    let ray_dir = Vector2::new(angle.cos(), angle.sin());
    Some(ray_dir * extent)
}

fn triangulate_sidewalk_difference(
    outer_loop: &[Vector2],
    road_loop: &[Vector2],
    node_pos: Vector3,
    y: f32,
) -> Option<Vec<TexturedTriangle>> {
    if outer_loop.len() < 3 || road_loop.len() < 3 {
        return None;
    }

    let mut angles = Vec::with_capacity(outer_loop.len() + road_loop.len());
    collect_loop_angles(outer_loop, &mut angles);
    collect_loop_angles(road_loop, &mut angles);
    dedup_sorted_scalars(&mut angles);
    if angles.len() < 2 {
        return None;
    }

    let mut triangles = Vec::new();
    let mut u_acc = 0.0f32;

    for idx in 0..angles.len() {
        let angle_a = angles[idx];
        let mut angle_b = angles[(idx + 1) % angles.len()];
        if idx + 1 == angles.len() {
            angle_b += std::f32::consts::TAU;
        }
        if (angle_b - angle_a).abs() <= 0.0001 {
            continue;
        }

        let outer_a = point_on_polygon_at_angle(outer_loop, angle_a)?;
        let outer_b = point_on_polygon_at_angle(outer_loop, angle_b)?;
        let road_a = point_on_polygon_at_angle(road_loop, angle_a)?;
        let road_b = point_on_polygon_at_angle(road_loop, angle_b)?;

        let outer_len = outer_a.distance_to(outer_b);
        let road_len = road_a.distance_to(road_b);
        let u_next = u_acc + outer_len.max(road_len);

        let outer_a_world = Vector3::new(node_pos.x + outer_a.x, y, node_pos.z + outer_a.y);
        let outer_b_world = Vector3::new(node_pos.x + outer_b.x, y, node_pos.z + outer_b.y);
        let road_a_world = Vector3::new(node_pos.x + road_a.x, y, node_pos.z + road_a.y);
        let road_b_world = Vector3::new(node_pos.x + road_b.x, y, node_pos.z + road_b.y);

        let tri0_cross = triangle_cross_xz(road_a_world, road_b_world, outer_b_world);
        if tri0_cross.abs() > 0.001 {
            triangles.push(TexturedTriangle {
                vertices: [road_a_world, road_b_world, outer_b_world],
                uvs: [
                    Vector2::new(u_acc, 0.0),
                    Vector2::new(u_next, 0.0),
                    Vector2::new(u_next, 1.0),
                ],
            });
        }

        let tri1_cross = triangle_cross_xz(road_a_world, outer_b_world, outer_a_world);
        if tri1_cross.abs() > 0.001 {
            triangles.push(TexturedTriangle {
                vertices: [road_a_world, outer_b_world, outer_a_world],
                uvs: [
                    Vector2::new(u_acc, 0.0),
                    Vector2::new(u_next, 1.0),
                    Vector2::new(u_acc, 1.0),
                ],
            });
        }

        u_acc = u_next;
    }

    if triangles.is_empty() {
        None
    } else {
        Some(triangles)
    }
}

fn build_junction_footprint(
    node_pos: Vector3,
    corridors: &[JunctionCorridor],
) -> Option<JunctionFootprint> {
    let road_polygons: Vec<Vec<Vector2>> = corridors
        .iter()
        .map(|corridor| corridor_polygon(corridor, false))
        .collect();
    let outer_polygons: Vec<Vec<Vector2>> = corridors
        .iter()
        .map(|corridor| corridor_polygon(corridor, true))
        .collect();

    let road_loop = build_union_boundary_from_polygons(&road_polygons)?;
    let outer_loop = build_union_boundary_from_polygons(&outer_polygons)?;
    let road_world_loop = loop_to_world_xz(
        &road_loop,
        node_pos,
        node_pos.y + config::ROAD_H_OFFSET + JUNCTION_ROAD_BIAS,
    );
    let outer_world_loop = loop_to_world_xz(
        &outer_loop,
        node_pos,
        node_pos.y + config::ROAD_H_OFFSET + JUNCTION_SIDEWALK_BIAS,
    );

    let road_indices = triangulate_boundary_xz(&road_world_loop);
    if road_indices.is_empty() {
        return None;
    }

    let road_triangles = road_indices
        .iter()
        .map(|[a, b, c]| {
            [
                road_world_loop[*a],
                road_world_loop[*b],
                road_world_loop[*c],
            ]
        })
        .collect();

    let sidewalk_triangles = triangulate_sidewalk_difference(
        &outer_loop,
        &road_loop,
        node_pos,
        node_pos.y + config::ROAD_H_OFFSET + JUNCTION_SIDEWALK_BIAS,
    )
    .unwrap_or_default();

    Some(JunctionFootprint {
        road_triangles,
        sidewalk_triangles,
        road_loops: vec![road_world_loop],
        outer_loops: vec![outer_world_loop],
    })
}

fn triangle_cross_xz(a: Vector3, b: Vector3, c: Vector3) -> f32 {
    (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
}

fn push_triangle(
    vertices: &mut Vec<Vector3>,
    normals: &mut Vec<Vector3>,
    uvs: &mut Vec<Vector2>,
    colors: &mut Vec<Color>,
    a: Vector3,
    b: Vector3,
    c: Vector3,
    uv_a: Vector2,
    uv_b: Vector2,
    uv_c: Vector2,
    color: Color,
) {
    let cross = triangle_cross_xz(a, b, c);
    if cross.abs() < 0.001 {
        return;
    }

    let (b, c, uv_b, uv_c) = if cross >= 0.0 {
        (b, c, uv_b, uv_c)
    } else {
        (c, b, uv_c, uv_b)
    };

    vertices.push(a);
    vertices.push(b);
    vertices.push(c);
    normals.push(Vector3::UP);
    normals.push(Vector3::UP);
    normals.push(Vector3::UP);
    uvs.push(uv_a);
    uvs.push(uv_b);
    uvs.push(uv_c);
    colors.push(color);
    colors.push(color);
    colors.push(color);
}

fn push_quad(
    vertices: &mut Vec<Vector3>,
    normals: &mut Vec<Vector3>,
    uvs: &mut Vec<Vector2>,
    colors: &mut Vec<Color>,
    a: Vector3,
    b: Vector3,
    c: Vector3,
    d: Vector3,
    uv_a: Vector2,
    uv_b: Vector2,
    uv_c: Vector2,
    uv_d: Vector2,
    color: Color,
) {
    push_triangle(
        vertices, normals, uvs, colors, a, b, c, uv_a, uv_b, uv_c, color,
    );
    push_triangle(
        vertices, normals, uvs, colors, a, c, d, uv_a, uv_c, uv_d, color,
    );
}

fn sample_edge_at_distance(edge: &Edge, distance_from_start: f32) -> Option<SegmentSample> {
    if edge.physical_geometry.len() < 2 {
        return None;
    }

    let mut dist_acc = 0.0f32;
    let target = distance_from_start.clamp(0.0, edge.physical_length.max(0.0));

    for (i, segment) in edge.physical_geometry.windows(2).enumerate() {
        let p0 = segment[0];
        let p1 = segment[1];
        let delta = p1 - p0;
        let seg_len = delta.length();
        if seg_len <= 1e-6 {
            continue;
        }

        if target <= dist_acc + seg_len || i == edge.physical_geometry.len() - 2 {
            let t = ((target - dist_acc) / seg_len).clamp(0.0, 1.0);
            return Some(SegmentSample {
                point: p0 + delta * t,
                tangent: delta / seg_len,
            });
        }

        dist_acc += seg_len;
    }

    let p0 = edge.physical_geometry[edge.physical_geometry.len() - 2];
    let p1 = *edge.physical_geometry.last().unwrap();
    let tangent = (p1 - p0).normalized_or_zero();
    Some(SegmentSample { point: p1, tangent })
}

fn polygon_signed_area_xz(boundary: &[Vector3]) -> f32 {
    let mut area = 0.0f32;
    for i in 0..boundary.len() {
        let a = boundary[i];
        let b = boundary[(i + 1) % boundary.len()];
        area += a.x * b.z - a.z * b.x;
    }
    area * 0.5
}

fn point_in_triangle_xz(point: Vector3, a: Vector3, b: Vector3, c: Vector3) -> bool {
    let ab = triangle_cross_xz(a, b, point);
    let bc = triangle_cross_xz(b, c, point);
    let ca = triangle_cross_xz(c, a, point);
    let epsilon = 0.001;
    let has_neg = ab < -epsilon || bc < -epsilon || ca < -epsilon;
    let has_pos = ab > epsilon || bc > epsilon || ca > epsilon;
    !(has_neg && has_pos)
}

fn same_point_xz(a: Vector3, b: Vector3) -> bool {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx * dx + dz * dz <= 0.0001 * 0.0001
}

fn triangulate_boundary_xz(boundary: &[Vector3]) -> Vec<[usize; 3]> {
    if boundary.len() < 3 {
        return Vec::new();
    }

    let signed_area = polygon_signed_area_xz(boundary);
    if signed_area.abs() < 0.001 {
        return Vec::new();
    }

    let is_ccw = signed_area > 0.0;
    let mut remaining: Vec<usize> = (0..boundary.len()).collect();
    let mut triangles = Vec::with_capacity(boundary.len().saturating_sub(2));
    let mut stalled_passes = 0usize;

    while remaining.len() > 3 {
        let mut clipped_ear = false;

        for i in 0..remaining.len() {
            let prev_idx = remaining[(i + remaining.len() - 1) % remaining.len()];
            let curr_idx = remaining[i];
            let next_idx = remaining[(i + 1) % remaining.len()];

            let prev = boundary[prev_idx];
            let curr = boundary[curr_idx];
            let next = boundary[next_idx];
            let cross = triangle_cross_xz(prev, curr, next);
            let is_convex = if is_ccw {
                cross > 0.001
            } else {
                cross < -0.001
            };
            if !is_convex {
                continue;
            }

            let contains_vertex = remaining.iter().copied().any(|candidate_idx| {
                if candidate_idx == prev_idx
                    || candidate_idx == curr_idx
                    || candidate_idx == next_idx
                {
                    return false;
                }
                let candidate = boundary[candidate_idx];
                !same_point_xz(candidate, prev)
                    && !same_point_xz(candidate, curr)
                    && !same_point_xz(candidate, next)
                    && point_in_triangle_xz(candidate, prev, curr, next)
            });
            if contains_vertex {
                continue;
            }

            triangles.push([prev_idx, curr_idx, next_idx]);
            remaining.remove(i);
            clipped_ear = true;
            stalled_passes = 0;
            break;
        }

        if clipped_ear {
            continue;
        }

        stalled_passes += 1;
        if stalled_passes > 1 {
            return Vec::new();
        }

        let mut removed_collinear = false;
        for i in 0..remaining.len() {
            let prev_idx = remaining[(i + remaining.len() - 1) % remaining.len()];
            let curr_idx = remaining[i];
            let next_idx = remaining[(i + 1) % remaining.len()];
            if triangle_cross_xz(boundary[prev_idx], boundary[curr_idx], boundary[next_idx]).abs()
                <= 0.001
            {
                remaining.remove(i);
                removed_collinear = true;
                break;
            }
        }

        if !removed_collinear {
            return Vec::new();
        }
    }

    if remaining.len() == 3 {
        triangles.push([remaining[0], remaining[1], remaining[2]]);
    }

    triangles
}

#[cfg(test)]
mod triangulation_tests {
    use super::*;

    #[test]
    fn triangulate_boundary_xz_handles_concave_polygon() {
        let boundary = vec![
            Vector3::new(-6.0, 0.0, -4.0),
            Vector3::new(6.0, 0.0, -4.0),
            Vector3::new(6.0, 0.0, -1.0),
            Vector3::new(1.0, 0.0, -1.0),
            Vector3::new(1.0, 0.0, 6.0),
            Vector3::new(-6.0, 0.0, 6.0),
        ];

        let triangles = triangulate_boundary_xz(&boundary);
        assert_eq!(
            triangles.len(),
            boundary.len() - 2,
            "concave junction footprints should triangulate fully"
        );

        let polygon_area = polygon_signed_area_xz(&boundary).abs();
        let triangles_area: f32 = triangles
            .iter()
            .map(|[a, b, c]| {
                triangle_cross_xz(boundary[*a], boundary[*b], boundary[*c]).abs() * 0.5
            })
            .sum();
        assert!(
            (triangles_area - polygon_area).abs() <= 0.001,
            "triangulation should cover the concave polygon exactly; expected area {:.3}, got {:.3}",
            polygon_area,
            triangles_area
        );
    }

    #[test]
    fn radial_extent_on_polygon_reaches_full_corridor_distance() {
        let polygon = vec![
            Vector2::new(0.0, -5.0),
            Vector2::new(6.0, -5.0),
            Vector2::new(6.0, 5.0),
            Vector2::new(0.0, 5.0),
        ];

        let extent = radial_extent_on_polygon(&polygon, 0.0)
            .expect("a forward corridor ray should intersect the corridor boundary");
        assert!(
            (extent - 6.0).abs() <= 0.001,
            "the exact corridor contour must preserve the full handoff distance; expected 6.0, got {extent:.3}"
        );
    }

    #[test]
    fn band_extent_clips_full_ribbon_width_against_polygon() {
        let boundary = vec![
            Vector3::new(0.0, 0.0, -5.0),
            Vector3::new(6.0, 0.0, -5.0),
            Vector3::new(6.0, 0.0, 5.0),
            Vector3::new(0.0, 0.0, 5.0),
        ];
        let frame = EndpointFrame2d {
            node_xz: Vector2::ZERO,
            outward: Vector2::RIGHT,
            side: Vector2::DOWN,
        };

        let extent = max_outward_extent_in_band(&[boundary], frame, -2.0, 2.0)
            .expect("a ribbon band overlapping the polygon should produce an exit distance");
        assert!(
            (extent - 6.0).abs() <= 0.001,
            "the ribbon clip should respect the farthest outward polygon extent across the full band; expected 6.0, got {extent:.3}"
        );
    }

    #[test]
    fn triangulate_sidewalk_difference_handles_ring_with_uv_contract() {
        let outer_loop = vec![
            Vector2::new(-6.0, -4.0),
            Vector2::new(6.0, -4.0),
            Vector2::new(6.0, 4.0),
            Vector2::new(-6.0, 4.0),
        ];
        let road_loop = vec![
            Vector2::new(-2.0, -1.5),
            Vector2::new(2.0, -1.5),
            Vector2::new(2.0, 1.5),
            Vector2::new(-2.0, 1.5),
        ];

        let band_triangles =
            triangulate_sidewalk_difference(&outer_loop, &road_loop, Vector3::ZERO, 0.0)
                .expect("a valid outer-minus-road ring should triangulate");

        let outer_area = polygon_signed_area_2d(&outer_loop).abs();
        let road_area = polygon_signed_area_2d(&road_loop).abs();
        let expected_area = outer_area - road_area;
        let triangles_area: f32 = band_triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.vertices;
                triangle_cross_xz(a, b, c).abs() * 0.5
            })
            .sum();

        assert!(
            (triangles_area - expected_area).abs() <= 0.001,
            "outer-minus-road triangulation should cover the exact ring area; expected {:.3}, got {:.3}",
            expected_area,
            triangles_area
        );
        assert!(
            band_triangles.iter().all(|triangle| triangle
                .uvs
                .iter()
                .all(|uv| (uv.y - 0.0).abs() <= 0.001 || (uv.y - 1.0).abs() <= 0.001)),
            "node sidewalk UVs must match the ribbon contract: UV.y is 0 at the road edge and 1 at the outer edge"
        );
    }
}

fn build_junction_corridor(
    graph: &RegionGraph,
    edge: &Edge,
    node_id: u32,
    is_start: bool,
    handoff_distance: f32,
) -> Option<JunctionCorridor> {
    let target = if is_start {
        handoff_distance
    } else {
        edge.physical_length - handoff_distance
    };
    let direction_sample = sample_edge_at_distance(edge, target)?;

    let outward_tangent = if is_start {
        direction_sample.tangent
    } else {
        -direction_sample.tangent
    };

    let node_pos = graph.nodes[node_id as usize].pos;
    let mut handoff_pos = direction_sample.point;
    if handoff_pos.distance_to(node_pos) < 0.05 {
        handoff_pos = node_pos + outward_tangent * 0.05;
        handoff_pos.y = direction_sample.point.y;
    }
    let delta_xz = Vector2::new(
        direction_sample.point.x - node_pos.x,
        direction_sample.point.z - node_pos.z,
    );
    let length_xz = delta_xz.length();
    if length_xz <= 1e-6 {
        return None;
    }
    let handoff_delta_xz = Vector2::new(handoff_pos.x - node_pos.x, handoff_pos.z - node_pos.z);

    // Bias the road extent slightly outward so the extracted ownership loops do not sit exactly
    // on the road surface sampling line. That keeps the node-owned sidewalk shell from claiming
    // road points purely because of the triangle edge epsilon.
    let road_half = edge.width * 0.5 + JUNCTION_ROAD_BIAS * 2.0;
    let outer_half = match edge.class {
        EdgeClass::Standard | EdgeClass::Ramp => road_half + config::SIDEWALK_WIDTH,
        EdgeClass::Bridge => road_half,
        EdgeClass::Tunnel => road_half,
    };
    Some(JunctionCorridor {
        handoff_dist: handoff_delta_xz.length().max(0.05),
        tangent_xz: delta_xz / length_xz,
        road_half,
        outer_half,
    })
}

fn build_junction_footprints(
    graph: &RegionGraph,
    node_to_edges: &[Vec<usize>],
    endpoint_trims: &[EndpointTrim],
    junction_nodes: &[bool],
) -> HashMap<u32, JunctionFootprint> {
    let mut footprints = HashMap::new();

    for (node_idx, edge_ids) in node_to_edges.iter().enumerate() {
        if !junction_nodes.get(node_idx).copied().unwrap_or(false) {
            continue;
        }
        let node_id = node_idx as u32;
        let mut corridors = Vec::new();

        for &edge_id in edge_ids {
            let edge = &graph.edges[edge_id];
            if edge.deleted
                || edge.primary_type != TransitType::Road
                || edge.class == EdgeClass::Tunnel
            {
                continue;
            }

            let start_node = graph.get_valid_node(edge.start_node);
            let end_node = graph.get_valid_node(edge.end_node);
            let is_start = if start_node == node_id {
                true
            } else if end_node == node_id {
                false
            } else {
                continue;
            };

            let handoff_trim = if is_start {
                endpoint_trims[edge_id].start_handoff
            } else {
                endpoint_trims[edge_id].end_handoff
            };
            if handoff_trim < MIN_JUNCTION_RADIUS {
                continue;
            }

            if let Some(corridor) =
                build_junction_corridor(graph, edge, node_id, is_start, handoff_trim)
            {
                corridors.push(corridor);
            }
        }

        if corridors.len() < 2 {
            continue;
        }

        if corridors.len() == 2 && corridors[0].tangent_xz.dot(corridors[1].tangent_xz) <= -0.985 {
            continue;
        }

        let center = graph.nodes[node_idx].pos;
        if let Some(footprint) = build_junction_footprint(center, &corridors) {
            footprints.insert(node_id, footprint);
        }
    }

    footprints
}

fn endpoint_frame_xz(graph: &RegionGraph, edge: &Edge, is_start: bool) -> Option<EndpointFrame2d> {
    let offset = edge.physical_length.min(0.1);
    let sample = if is_start {
        sample_edge_at_distance(edge, offset)?
    } else {
        sample_edge_at_distance(edge, (edge.physical_length - offset).max(0.0))?
    };

    let forward = Vector2::new(sample.tangent.x, sample.tangent.z);
    if forward.length_squared() <= 1e-6 {
        return None;
    }

    let forward = forward.normalized();
    let node_id = if is_start {
        graph.get_valid_node(edge.start_node)
    } else {
        graph.get_valid_node(edge.end_node)
    };
    let node_pos = graph.nodes[node_id as usize].pos;

    Some(EndpointFrame2d {
        node_xz: Vector2::new(node_pos.x, node_pos.z),
        outward: if is_start { forward } else { -forward },
        side: Vector2::new(-forward.y, forward.x),
    })
}

fn incident_direction_xz(graph: &RegionGraph, edge: &Edge, node_id: u32) -> Option<Vector2> {
    let start_node = graph.get_valid_node(edge.start_node);
    let end_node = graph.get_valid_node(edge.end_node);
    let is_start = if start_node == node_id {
        true
    } else if end_node == node_id {
        false
    } else {
        return None;
    };

    let offset = edge.physical_length.min(0.1);
    let sample = if is_start {
        sample_edge_at_distance(edge, offset)?
    } else {
        sample_edge_at_distance(edge, (edge.physical_length - offset).max(0.0))?
    };
    let outward = if is_start {
        sample.tangent
    } else {
        -sample.tangent
    };
    let outward_xz = Vector2::new(outward.x, outward.z);
    if outward_xz.length_squared() > 1e-6 {
        Some(outward_xz.normalized())
    } else {
        None
    }
}

fn build_node_miters(graph: &RegionGraph) -> HashMap<u32, Vector2> {
    let mut node_dirs: HashMap<u32, Vec<Vector2>> = HashMap::new();
    let mut node_has_clip: HashMap<u32, bool> = HashMap::new();

    for edge in &graph.edges {
        if edge.deleted
            || (edge.primary_type != TransitType::Road && edge.primary_type != TransitType::Foot)
        {
            continue;
        }
        if edge.physical_geometry.len() < 2 {
            continue;
        }

        let start_node = graph.get_valid_node(edge.start_node);
        let end_node = graph.get_valid_node(edge.end_node);
        if edge.start_clip > 0.001 {
            node_has_clip.insert(start_node, true);
        }
        if edge.end_clip > 0.001 {
            node_has_clip.insert(end_node, true);
        }

        let start_pos = graph.nodes[edge.start_node as usize].pos;
        let end_pos = graph.nodes[edge.end_node as usize].pos;

        let d3_s = edge.physical_geometry[1] - start_pos;
        let d2_s = Vector2::new(d3_s.x, d3_s.z);
        if d2_s.length_squared() > 1e-6 {
            node_dirs
                .entry(start_node)
                .or_default()
                .push(d2_s.normalized());
        }

        let last = edge.physical_geometry.len() - 1;
        let d3_e = edge.physical_geometry[last - 1] - end_pos;
        let d2_e = Vector2::new(d3_e.x, d3_e.z);
        if d2_e.length_squared() > 1e-6 {
            node_dirs
                .entry(end_node)
                .or_default()
                .push(d2_e.normalized());
        }
    }

    let mut node_miters = HashMap::new();
    for (&node_id, dirs) in &node_dirs {
        if dirs.len() != 2 || *node_has_clip.get(&node_id).unwrap_or(&false) {
            continue;
        }

        let d1 = dirs[0];
        let d2 = dirs[1];
        let s1 = Vector2::new(-d1.y, d1.x);
        let s2 = Vector2::new(-d2.y, d2.x);
        let diff = s1 - s2;
        if diff.length_squared() <= 1e-6 {
            continue;
        }

        let miter = diff.normalized();
        let cos_half = s1.dot(miter).abs();
        if cos_half > 0.1 {
            node_miters.insert(node_id, miter * (1.0 / cos_half).min(4.0));
        }
    }

    node_miters
}

/// Generates road, bridge, and sidewalk mesh data from the road graph.
pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(
        &self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        let mut mesh = NetworkMeshData::new();
        let _hw = (terrain.width as f32 - 1.0) * 0.5;
        let _hh = (terrain.height as f32 - 1.0) * 0.5;

        let mut node_to_edges = vec![Vec::new(); graph.nodes.len()];
        let mut connection_counts = HashMap::new();
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.deleted
                || (edge.primary_type != TransitType::Road
                    && edge.primary_type != TransitType::Foot)
            {
                continue;
            }
            let start_node = graph.get_valid_node(edge.start_node) as usize;
            let end_node = graph.get_valid_node(edge.end_node) as usize;
            node_to_edges[start_node].push(edge_id);
            node_to_edges[end_node].push(edge_id);
            *connection_counts.entry(start_node as u32).or_insert(0usize) += 1;
            *connection_counts.entry(end_node as u32).or_insert(0usize) += 1;
        }

        let mut junction_nodes = vec![false; graph.nodes.len()];
        for (node_idx, edge_ids) in node_to_edges.iter().enumerate() {
            let mut road_count = 0usize;
            let mut road_dirs = Vec::new();

            for &edge_id in edge_ids {
                let edge = &graph.edges[edge_id];
                if edge.deleted
                    || edge.primary_type != TransitType::Road
                    || edge.class == EdgeClass::Tunnel
                {
                    continue;
                }

                road_count += 1;
                if let Some(dir) = incident_direction_xz(graph, edge, node_idx as u32) {
                    road_dirs.push(dir);
                }
            }

            if road_count < 2 {
                continue;
            }

            let is_pass_through = road_dirs.len() == 2 && road_dirs[0].dot(road_dirs[1]) <= -0.985;
            if !is_pass_through {
                junction_nodes[node_idx] = true;
            }
        }

        let mut endpoint_trims = vec![EndpointTrim::default(); graph.edges.len()];
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            let start_node = graph.get_valid_node(edge.start_node) as usize;
            let end_node = graph.get_valid_node(edge.end_node) as usize;
            let outer_handoff = match edge.class {
                EdgeClass::Standard | EdgeClass::Ramp => edge.width * 0.5 + config::SIDEWALK_WIDTH,
                EdgeClass::Bridge => edge.width * 0.5,
                EdgeClass::Tunnel => edge.width * 0.5,
            };
            let mut start_handoff = if edge.class == EdgeClass::Tunnel {
                edge.start_clip
            } else if junction_nodes[start_node] {
                edge.start_clip.max(outer_handoff)
            } else {
                edge.start_clip
            };
            let mut end_handoff = if edge.class == EdgeClass::Tunnel {
                edge.end_clip
            } else if junction_nodes[end_node] {
                edge.end_clip.max(outer_handoff)
            } else {
                edge.end_clip
            };

            let max_total = (edge.physical_length - MIN_EDGE_REMAINDER).max(0.0);
            if start_handoff + end_handoff > max_total && start_handoff + end_handoff > 1e-6 {
                let scale = max_total / (start_handoff + end_handoff);
                start_handoff *= scale;
                end_handoff *= scale;
            }

            endpoint_trims[edge_id] = EndpointTrim {
                start_handoff,
                end_handoff,
            };
        }

        let junction_footprints =
            build_junction_footprints(graph, &node_to_edges, &endpoint_trims, &junction_nodes);
        let node_miters = build_node_miters(graph);

        for (edge_id, edge) in graph.edges.iter().enumerate() {
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 {
                continue;
            }

            let h_offset = config::ROAD_H_OFFSET;
            let z_bias = config::Z_FIGHT_BIAS;

            if edge.primary_type == TransitType::Foot {
                let lane_color = Color::from_rgb(0.4, 0.4, 0.45);
                let lane_w = 1.0;
                let start_node = graph.get_valid_node(edge.start_node);
                let end_node = graph.get_valid_node(edge.end_node);

                let mut point_side_dirs = Vec::with_capacity(resampled_count);
                for i in 0..resampled_count {
                    if i == 0 {
                        if let Some(miter) = node_miters.get(&start_node) {
                            point_side_dirs.push(Vector3::new(miter.x, 0.0, miter.y));
                        } else {
                            let d = edge.physical_geometry[1] - edge.physical_geometry[0];
                            let tangent = if d.length_squared() > 1e-6 {
                                d.normalized()
                            } else {
                                Vector3::FORWARD
                            };
                            point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                        }
                    } else if i == resampled_count - 1 {
                        if let Some(miter) = node_miters.get(&end_node) {
                            point_side_dirs.push(Vector3::new(-miter.x, 0.0, -miter.y));
                        } else {
                            let d = edge.physical_geometry[i] - edge.physical_geometry[i - 1];
                            let tangent = if d.length_squared() > 1e-6 {
                                d.normalized()
                            } else {
                                Vector3::FORWARD
                            };
                            point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                        }
                    } else {
                        let d = edge.physical_geometry[i + 1] - edge.physical_geometry[i - 1];
                        let tangent = if d.length_squared() > 1e-6 {
                            d.normalized()
                        } else {
                            Vector3::FORWARD
                        };
                        point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                    }
                }

                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i + 1];
                    if (p1 - p0).length() < 0.01 {
                        continue;
                    }

                    p0.y += h_offset + z_bias;
                    p1.y += h_offset + z_bias;

                    let v0_l = p0 - side0 * (lane_w * 0.5);
                    let v0_r = p0 + side0 * (lane_w * 0.5);
                    let v1_l = p1 - side1 * (lane_w * 0.5);
                    let v1_r = p1 + side1 * (lane_w * 0.5);

                    push_quad(
                        &mut mesh.vertices,
                        &mut mesh.normals,
                        &mut mesh.uvs,
                        &mut mesh.colors,
                        v0_l,
                        v1_l,
                        v1_r,
                        v0_r,
                        Vector2::ZERO,
                        Vector2::ZERO,
                        Vector2::ZERO,
                        Vector2::ZERO,
                        lane_color,
                    );
                }
                continue;
            }

            if edge.class == EdgeClass::Tunnel {
                for &p_idx in &[0, resampled_count - 1] {
                    let p = edge.physical_geometry[p_idx];
                    let tangent = if p_idx == 0 {
                        edge.physical_geometry[1] - edge.physical_geometry[0]
                    } else {
                        edge.physical_geometry[p_idx] - edge.physical_geometry[p_idx - 1]
                    };
                    let tangent = if tangent.length_squared() > 1e-6 {
                        tangent.normalized()
                    } else {
                        Vector3::FORWARD
                    };
                    let side = Vector3::new(-tangent.z, 0.0, tangent.x);
                    let hw = edge.width * 0.5 + config::SIDEWALK_WIDTH;
                    let hh = 4.0;

                    let p_elevated = p + Vector3::UP * 0.2;
                    let v_bl = p_elevated - side * hw;
                    let v_br = p_elevated + side * hw;
                    let v_tl = v_bl + Vector3::UP * hh;
                    let v_tr = v_br + Vector3::UP * hh;

                    mesh.vertices.push(v_bl);
                    mesh.vertices.push(v_tl);
                    mesh.vertices.push(v_tr);
                    mesh.vertices.push(v_bl);
                    mesh.vertices.push(v_tr);
                    mesh.vertices.push(v_br);
                    for _ in 0..6 {
                        mesh.normals.push(-tangent);
                        mesh.colors.push(Color::from_rgb(0.1, 0.1, 0.1));
                        mesh.uvs.push(Vector2::ZERO);
                    }
                }
                continue;
            }

            if edge.primary_type != TransitType::Road {
                continue;
            }

            let start_node = graph.get_valid_node(edge.start_node);
            let end_node = graph.get_valid_node(edge.end_node);
            let trims = endpoint_trims[edge_id];
            let base_start_handoff = trims.start_handoff;
            let base_end_handoff = trims.end_handoff;
            let total_len = edge.physical_length;
            let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;
            if total_lanes <= 0.0 {
                continue;
            }

            let lane_w = edge.width / total_lanes;
            let road_outer = edge.width * 0.5;
            let start_junction = junction_footprints.get(&start_node);
            let end_junction = junction_footprints.get(&end_node);
            let start_frame = if junction_nodes[start_node as usize] {
                endpoint_frame_xz(graph, edge, true)
            } else {
                None
            };
            let end_frame = if junction_nodes[end_node as usize] {
                endpoint_frame_xz(graph, edge, false)
            } else {
                None
            };
            let clamp_handoffs = |start: f32, end: f32| {
                let max_total = (total_len - MIN_EDGE_REMAINDER).max(0.0);
                if start + end > max_total && start + end > 1e-6 {
                    let scale = max_total / (start + end);
                    (start * scale, end * scale)
                } else {
                    (start, end)
                }
            };
            let band_handoff = |junction: Option<&JunctionFootprint>,
                                frame: Option<EndpointFrame2d>,
                                use_outer: bool,
                                band_min: f32,
                                band_max: f32| {
                match (junction, frame) {
                    (Some(junction), Some(frame)) => {
                        let boundaries = if use_outer {
                            &junction.outer_loops
                        } else {
                            &junction.road_loops
                        };
                        max_outward_extent_in_band(boundaries, frame, band_min, band_max)
                            .unwrap_or(0.0)
                    }
                    _ => 0.0,
                }
            };

            let mut point_side_dirs = Vec::with_capacity(resampled_count);
            for i in 0..resampled_count {
                let d = if i == 0 {
                    edge.physical_geometry[1] - edge.physical_geometry[0]
                } else if i == resampled_count - 1 {
                    edge.physical_geometry[i] - edge.physical_geometry[i - 1]
                } else {
                    edge.physical_geometry[i + 1] - edge.physical_geometry[i - 1]
                };
                let tangent = if d.length_squared() > 1e-6 {
                    d.normalized()
                } else {
                    Vector3::FORWARD
                };
                point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
            }

            let lane_count = (edge.fwd_lanes + edge.bkw_lanes) as usize;
            for l_idx in 0..lane_count {
                let lateral_offset = (total_lanes * 0.5 - l_idx as f32 - 0.5) * lane_w;
                let lane_band_min = lateral_offset - lane_w * 0.5;
                let lane_band_max = lateral_offset + lane_w * 0.5;
                let (lane_start_handoff, lane_end_handoff) = clamp_handoffs(
                    base_start_handoff.max(band_handoff(
                        start_junction,
                        start_frame,
                        false,
                        lane_band_min,
                        lane_band_max,
                    )),
                    base_end_handoff.max(band_handoff(
                        end_junction,
                        end_frame,
                        false,
                        lane_band_min,
                        lane_band_max,
                    )),
                );
                let mut dist_acc = 0.0f32;

                for i in 0..resampled_count - 1 {
                    let p0_raw = edge.physical_geometry[i];
                    let p1_raw = edge.physical_geometry[i + 1];
                    let segment_len = (p1_raw - p0_raw).length();
                    if segment_len <= 1e-6 {
                        continue;
                    }

                    let segment_start = dist_acc;
                    let segment_end = dist_acc + segment_len;
                    if segment_end <= lane_start_handoff
                        || segment_start >= total_len - lane_end_handoff
                    {
                        dist_acc += segment_len;
                        continue;
                    }

                    let mut t0 = 0.0f32;
                    let mut t1 = 1.0f32;
                    if segment_start < lane_start_handoff {
                        t0 = (lane_start_handoff - segment_start) / segment_len;
                    }
                    if segment_end > total_len - lane_end_handoff {
                        t1 = (total_len - lane_end_handoff - segment_start) / segment_len;
                    }
                    if t1 - t0 <= 1e-4 {
                        dist_acc += segment_len;
                        continue;
                    }

                    let mut p0 = p0_raw + (p1_raw - p0_raw) * t0;
                    let mut p1 = p0_raw + (p1_raw - p0_raw) * t1;
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i + 1];

                    p0.y += h_offset;
                    p1.y += h_offset;

                    let v0_l = p0 + side0 * (lateral_offset - lane_w * 0.5);
                    let v0_r = p0 + side0 * (lateral_offset + lane_w * 0.5);
                    let v1_l = p1 + side1 * (lateral_offset - lane_w * 0.5);
                    let v1_r = p1 + side1 * (lateral_offset + lane_w * 0.5);

                    let area1 = (v1_l - v0_l).cross(v1_r - v0_l).length() * 0.5;
                    let area2 = (v1_r - v0_l).cross(v0_r - v0_l).length() * 0.5;
                    if area1 < 0.001 && area2 < 0.001 {
                        dist_acc += segment_len;
                        continue;
                    }

                    let is_lane_boundary = if l_idx > 0 { 1.0 } else { 0.0 };
                    let is_center_boundary = if l_idx == edge.fwd_lanes as usize
                        && edge.fwd_lanes > 0
                        && edge.bkw_lanes > 0
                    {
                        1.0
                    } else {
                        0.0
                    };
                    let lane_color =
                        Color::from_rgba(1.0, is_lane_boundary, is_center_boundary, 0.0);
                    let uv_x0 = segment_start + t0 * segment_len;
                    let uv_x1 = segment_start + t1 * segment_len;

                    push_quad(
                        &mut mesh.vertices,
                        &mut mesh.normals,
                        &mut mesh.uvs,
                        &mut mesh.colors,
                        v0_l,
                        v1_l,
                        v1_r,
                        v0_r,
                        Vector2::new(uv_x0, 0.0),
                        Vector2::new(uv_x1, 0.0),
                        Vector2::new(uv_x1, 1.0),
                        Vector2::new(uv_x0, 1.0),
                        lane_color,
                    );

                    dist_acc += segment_len;
                }
            }

            if matches!(edge.class, EdgeClass::Standard | EdgeClass::Ramp) {
                let shoulder_outer = road_outer + config::SIDEWALK_WIDTH;
                let shoulder_color = Color::from_rgba(1.0, 1.0, 1.0, 1.0);
                let mut dist_acc = 0.0f32;

                for i in 0..resampled_count - 1 {
                    let p0_raw = edge.physical_geometry[i];
                    let p1_raw = edge.physical_geometry[i + 1];
                    let segment_len = (p1_raw - p0_raw).length();
                    if segment_len <= 1e-6 {
                        continue;
                    }

                    let segment_start = dist_acc;
                    let segment_end = dist_acc + segment_len;
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i + 1];

                    for sign in [-1.0f32, 1.0f32] {
                        let sidewalk_band_min = road_outer * sign;
                        let sidewalk_band_max = shoulder_outer * sign;
                        let (sidewalk_start_handoff, sidewalk_end_handoff) = clamp_handoffs(
                            base_start_handoff.max(band_handoff(
                                start_junction,
                                start_frame,
                                true,
                                sidewalk_band_min,
                                sidewalk_band_max,
                            )),
                            base_end_handoff.max(band_handoff(
                                end_junction,
                                end_frame,
                                true,
                                sidewalk_band_min,
                                sidewalk_band_max,
                            )),
                        );
                        if segment_end <= sidewalk_start_handoff
                            || segment_start >= total_len - sidewalk_end_handoff
                        {
                            continue;
                        }

                        let mut t0 = 0.0f32;
                        let mut t1 = 1.0f32;
                        if segment_start < sidewalk_start_handoff {
                            t0 = (sidewalk_start_handoff - segment_start) / segment_len;
                        }
                        if segment_end > total_len - sidewalk_end_handoff {
                            t1 = (total_len - sidewalk_end_handoff - segment_start) / segment_len;
                        }
                        if t1 - t0 <= 1e-4 {
                            continue;
                        }

                        let mut p0 = p0_raw + (p1_raw - p0_raw) * t0;
                        let mut p1 = p0_raw + (p1_raw - p0_raw) * t1;
                        p0.y += h_offset;
                        p1.y += h_offset;

                        let inner0 = p0 + side0 * road_outer * sign;
                        let inner1 = p1 + side1 * road_outer * sign;
                        let outer0 = p0 + side0 * shoulder_outer * sign;
                        let outer1 = p1 + side1 * shoulder_outer * sign;

                        let uv_x0 = segment_start + t0 * segment_len;
                        let uv_x1 = segment_start + t1 * segment_len;

                        push_quad(
                            &mut mesh.vertices,
                            &mut mesh.normals,
                            &mut mesh.uvs,
                            &mut mesh.colors,
                            inner0,
                            inner1,
                            outer1,
                            outer0,
                            Vector2::new(uv_x0, 0.0),
                            Vector2::new(uv_x1, 0.0),
                            Vector2::new(uv_x1, 1.0),
                            Vector2::new(uv_x0, 1.0),
                            shoulder_color,
                        );
                    }

                    dist_acc += segment_len;
                }
            }

            if edge.class == EdgeClass::Bridge {
                let hw = edge.width * 0.5 + config::SIDEWALK_WIDTH;
                let thickness = 1.0;
                let deck_color = Color::from_rgb(0.3, 0.3, 0.31);
                let concrete_color = Color::from_rgba(0.9, 0.9, 0.9, 1.0);

                let start_node = graph.get_valid_node(edge.start_node);
                let end_node = graph.get_valid_node(edge.end_node);
                let (bridge_start_handoff, bridge_end_handoff) = clamp_handoffs(
                    base_start_handoff.max(band_handoff(
                        start_junction,
                        start_frame,
                        false,
                        -road_outer,
                        road_outer,
                    )),
                    base_end_handoff.max(band_handoff(
                        end_junction,
                        end_frame,
                        false,
                        -road_outer,
                        road_outer,
                    )),
                );
                let mut dist_acc = 0.0f32;
                let mut dist_acc_pillars = 0.0f32;

                for i in 0..resampled_count - 1 {
                    let p0_raw = edge.physical_geometry[i];
                    let p1_raw = edge.physical_geometry[i + 1];
                    let segment_len = (p1_raw - p0_raw).length();
                    if segment_len <= 1e-6 {
                        continue;
                    }

                    let segment_start = dist_acc;
                    let segment_end = dist_acc + segment_len;
                    if segment_end <= bridge_start_handoff
                        || segment_start >= total_len - bridge_end_handoff
                    {
                        dist_acc += segment_len;
                        continue;
                    }

                    let mut t0 = 0.0f32;
                    let mut t1 = 1.0f32;
                    if segment_start < bridge_start_handoff {
                        t0 = (bridge_start_handoff - segment_start) / segment_len;
                    }
                    if segment_end > total_len - bridge_end_handoff {
                        t1 = (total_len - bridge_end_handoff - segment_start) / segment_len;
                    }
                    if t1 - t0 <= 1e-4 {
                        dist_acc += segment_len;
                        continue;
                    }

                    let p0 = p0_raw + (p1_raw - p0_raw) * t0;
                    let p1 = p0_raw + (p1_raw - p0_raw) * t1;
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i + 1];

                    let p0_l = p0 - side0 * hw;
                    let p0_r = p0 + side0 * hw;
                    let p1_l = p1 - side1 * hw;
                    let p1_r = p1 + side1 * hw;

                    let p0_lb = p0_l - Vector3::UP * thickness;
                    let p0_rb = p0_r - Vector3::UP * thickness;
                    let p1_lb = p1_l - Vector3::UP * thickness;
                    let p1_rb = p1_r - Vector3::UP * thickness;

                    mesh.vertices.push(p0_l);
                    mesh.vertices.push(p1_lb);
                    mesh.vertices.push(p0_lb);
                    mesh.vertices.push(p0_l);
                    mesh.vertices.push(p1_l);
                    mesh.vertices.push(p1_lb);
                    for _ in 0..6 {
                        mesh.normals.push(-side0);
                        mesh.colors.push(deck_color);
                        mesh.uvs.push(Vector2::ZERO);
                    }

                    mesh.vertices.push(p0_r);
                    mesh.vertices.push(p0_rb);
                    mesh.vertices.push(p1_rb);
                    mesh.vertices.push(p0_r);
                    mesh.vertices.push(p1_rb);
                    mesh.vertices.push(p1_r);
                    for _ in 0..6 {
                        mesh.normals.push(side0);
                        mesh.colors.push(deck_color);
                        mesh.uvs.push(Vector2::ZERO);
                    }

                    mesh.vertices.push(p0_lb);
                    mesh.vertices.push(p1_rb);
                    mesh.vertices.push(p1_lb);
                    mesh.vertices.push(p0_lb);
                    mesh.vertices.push(p0_rb);
                    mesh.vertices.push(p1_rb);
                    for _ in 0..6 {
                        mesh.normals.push(Vector3::DOWN);
                        mesh.colors.push(deck_color);
                        mesh.uvs.push(Vector2::ZERO);
                    }

                    let p_mid = p0.lerp(p1, 0.5);
                    let gx = p_mid.x + _hw;
                    let gz = p_mid.z + _hh;
                    let terrain_y =
                        terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
                    let clearance = p_mid.y - terrain_y;

                    let rail_h = 1.2;
                    let rail_t = 0.1;
                    let p0_lo = p0_l + side0 * rail_t;
                    let p1_lo = p1_l + side0 * rail_t;
                    let p0_lt = p0_l + Vector3::UP * rail_h;
                    let p1_lt = p1_l + Vector3::UP * rail_h;
                    let p0_lto = p0_lo + Vector3::UP * rail_h;
                    let p1_lto = p1_lo + Vector3::UP * rail_h;

                    mesh.concrete_vertices.push(p0_l);
                    mesh.concrete_vertices.push(p1_lt);
                    mesh.concrete_vertices.push(p0_lt);
                    mesh.concrete_vertices.push(p0_l);
                    mesh.concrete_vertices.push(p1_l);
                    mesh.concrete_vertices.push(p1_lt);
                    for _ in 0..6 {
                        mesh.concrete_normals.push(side0);
                        mesh.concrete_colors.push(concrete_color);
                        mesh.concrete_uvs.push(Vector2::ZERO);
                    }

                    mesh.concrete_vertices.push(p0_lo);
                    mesh.concrete_vertices.push(p0_lto);
                    mesh.concrete_vertices.push(p1_lto);
                    mesh.concrete_vertices.push(p0_lo);
                    mesh.concrete_vertices.push(p1_lto);
                    mesh.concrete_vertices.push(p1_lo);
                    for _ in 0..6 {
                        mesh.concrete_normals.push(-side0);
                        mesh.concrete_colors.push(concrete_color);
                        mesh.concrete_uvs.push(Vector2::ZERO);
                    }

                    mesh.concrete_vertices.push(p0_lt);
                    mesh.concrete_vertices.push(p1_lt);
                    mesh.concrete_vertices.push(p1_lto);
                    mesh.concrete_vertices.push(p0_lt);
                    mesh.concrete_vertices.push(p1_lto);
                    mesh.concrete_vertices.push(p0_lto);
                    for _ in 0..6 {
                        mesh.concrete_normals.push(Vector3::UP);
                        mesh.concrete_colors.push(concrete_color);
                        mesh.concrete_uvs.push(Vector2::ZERO);
                    }

                    let rail_dir_r = -side0;
                    let p0_ro = p0_r + rail_dir_r * rail_t;
                    let p1_ro = p1_r + rail_dir_r * rail_t;
                    let p0_rt = p0_r + Vector3::UP * rail_h;
                    let p1_rt = p1_r + Vector3::UP * rail_h;
                    let p0_rto = p0_ro + Vector3::UP * rail_h;
                    let p1_rto = p1_ro + Vector3::UP * rail_h;

                    mesh.concrete_vertices.push(p0_r);
                    mesh.concrete_vertices.push(p0_rt);
                    mesh.concrete_vertices.push(p1_rt);
                    mesh.concrete_vertices.push(p0_r);
                    mesh.concrete_vertices.push(p1_rt);
                    mesh.concrete_vertices.push(p1_r);
                    for _ in 0..6 {
                        mesh.concrete_normals.push(-side0);
                        mesh.concrete_colors.push(concrete_color);
                        mesh.concrete_uvs.push(Vector2::ZERO);
                    }

                    mesh.concrete_vertices.push(p0_ro);
                    mesh.concrete_vertices.push(p1_rto);
                    mesh.concrete_vertices.push(p0_rto);
                    mesh.concrete_vertices.push(p0_ro);
                    mesh.concrete_vertices.push(p1_ro);
                    mesh.concrete_vertices.push(p1_rto);
                    for _ in 0..6 {
                        mesh.concrete_normals.push(side0);
                        mesh.concrete_colors.push(concrete_color);
                        mesh.concrete_uvs.push(Vector2::ZERO);
                    }

                    mesh.concrete_vertices.push(p0_rt);
                    mesh.concrete_vertices.push(p0_rto);
                    mesh.concrete_vertices.push(p1_rto);
                    mesh.concrete_vertices.push(p0_rt);
                    mesh.concrete_vertices.push(p1_rto);
                    mesh.concrete_vertices.push(p1_rt);
                    for _ in 0..6 {
                        mesh.concrete_normals.push(Vector3::UP);
                        mesh.concrete_colors.push(concrete_color);
                        mesh.concrete_uvs.push(Vector2::ZERO);
                    }

                    if clearance <= 5.0 {
                        let sink = 1.0;
                        let p0_lg = Vector3::new(
                            p0_l.x,
                            (terrain.get_height_interpolated(p0_l.x + _hw, p0_l.z + _hh)
                                * crate::config::HEIGHT_SCALE)
                                - sink,
                            p0_l.z,
                        );
                        let p0_rg = Vector3::new(
                            p0_r.x,
                            (terrain.get_height_interpolated(p0_r.x + _hw, p0_r.z + _hh)
                                * crate::config::HEIGHT_SCALE)
                                - sink,
                            p0_r.z,
                        );
                        let p1_lg = Vector3::new(
                            p1_l.x,
                            (terrain.get_height_interpolated(p1_l.x + _hw, p1_l.z + _hh)
                                * crate::config::HEIGHT_SCALE)
                                - sink,
                            p1_l.z,
                        );
                        let p1_rg = Vector3::new(
                            p1_r.x,
                            (terrain.get_height_interpolated(p1_r.x + _hw, p1_r.z + _hh)
                                * crate::config::HEIGHT_SCALE)
                                - sink,
                            p1_r.z,
                        );

                        mesh.concrete_vertices.push(p0_l);
                        mesh.concrete_vertices.push(p0_lg);
                        mesh.concrete_vertices.push(p1_lg);
                        mesh.concrete_vertices.push(p0_l);
                        mesh.concrete_vertices.push(p1_lg);
                        mesh.concrete_vertices.push(p1_l);
                        for _ in 0..6 {
                            mesh.concrete_normals.push(-side0);
                            mesh.concrete_colors.push(concrete_color);
                            mesh.concrete_uvs.push(Vector2::ZERO);
                        }

                        mesh.concrete_vertices.push(p0_r);
                        mesh.concrete_vertices.push(p1_rg);
                        mesh.concrete_vertices.push(p0_rg);
                        mesh.concrete_vertices.push(p0_r);
                        mesh.concrete_vertices.push(p1_r);
                        mesh.concrete_vertices.push(p1_rg);
                        for _ in 0..6 {
                            mesh.concrete_normals.push(side0);
                            mesh.concrete_colors.push(concrete_color);
                            mesh.concrete_uvs.push(Vector2::ZERO);
                        }
                    } else {
                        let seg_len = (p1 - p0).length();
                        dist_acc_pillars += seg_len;
                        if dist_acc_pillars >= 15.0 || i == 0 {
                            if i > 0 {
                                dist_acc_pillars = 0.0;
                            }
                            let p_w = edge.width * 0.3;
                            let p_h_top = p_mid.y - thickness;
                            let p_h_bot = terrain_y;
                            let fwd = (p1 - p0).normalized();
                            let c_p0 = p_mid - side0 * (p_w * 0.5);
                            let c_p1 = p_mid + side0 * (p_w * 0.5);
                            let c_p2 = c_p1 + fwd * p_w;
                            let c_p3 = c_p0 + fwd * p_w;
                            let verts = [c_p0, c_p1, c_p2, c_p3];
                            for j in 0..4 {
                                let va = verts[j];
                                let vb = verts[(j + 1) % 4];
                                let va_g = Vector3::new(va.x, p_h_bot, va.z);
                                let vb_g = Vector3::new(vb.x, p_h_bot, vb.z);
                                let va_t = Vector3::new(va.x, p_h_top, va.z);
                                let vb_t = Vector3::new(vb.x, p_h_top, vb.z);
                                mesh.concrete_vertices.push(va_t);
                                mesh.concrete_vertices.push(vb_g);
                                mesh.concrete_vertices.push(va_g);
                                mesh.concrete_vertices.push(va_t);
                                mesh.concrete_vertices.push(vb_t);
                                mesh.concrete_vertices.push(vb_g);
                                let n = (vb - va).cross(Vector3::UP).normalized();
                                for _ in 0..6 {
                                    mesh.concrete_normals.push(n);
                                    mesh.concrete_colors.push(concrete_color);
                                    mesh.concrete_uvs.push(Vector2::ZERO);
                                }
                            }
                        }
                    }

                    let start_deg = *connection_counts.get(&start_node).unwrap_or(&0);
                    let end_deg = *connection_counts.get(&end_node).unwrap_or(&0);
                    let is_start_cap = (i == 0) && (t0 == 0.0) && (start_deg == 1);
                    let is_end_cap = (i == resampled_count - 2) && (t1 == 1.0) && (end_deg == 1);
                    if is_start_cap || is_end_cap {
                        let fwd = (p1 - p0).normalized();
                        let (v_l, v_r, v_lb, v_rb, norm) = if is_start_cap {
                            (p0_l, p0_r, p0_lb, p0_rb, -fwd)
                        } else {
                            (p1_l, p1_r, p1_lb, p1_rb, fwd)
                        };
                        mesh.concrete_vertices.push(v_l);
                        mesh.concrete_vertices.push(v_rb);
                        mesh.concrete_vertices.push(v_lb);
                        mesh.concrete_vertices.push(v_l);
                        mesh.concrete_vertices.push(v_r);
                        mesh.concrete_vertices.push(v_rb);
                        for _ in 0..6 {
                            mesh.concrete_normals.push(norm);
                            mesh.concrete_colors.push(concrete_color);
                            mesh.concrete_uvs.push(Vector2::ZERO);
                        }
                    }

                    dist_acc += segment_len;
                }
            }
        }

        for footprint in junction_footprints.values() {
            for triangle in &footprint.sidewalk_triangles {
                let [a, b, c] = triangle.vertices;
                let [uv_a, uv_b, uv_c] = triangle.uvs;
                push_triangle(
                    &mut mesh.vertices,
                    &mut mesh.normals,
                    &mut mesh.uvs,
                    &mut mesh.colors,
                    a,
                    b,
                    c,
                    uv_a,
                    uv_b,
                    uv_c,
                    Color::from_rgba(1.0, 1.0, 1.0, 1.0),
                );
            }

            for [a, b, c] in &footprint.road_triangles {
                push_triangle(
                    &mut mesh.vertices,
                    &mut mesh.normals,
                    &mut mesh.uvs,
                    &mut mesh.colors,
                    *a,
                    *b,
                    *c,
                    Vector2::new(a.x, a.z),
                    Vector2::new(b.x, b.z),
                    Vector2::new(c.x, c.z),
                    Color::from_rgba(1.0, 1.0, 1.0, 0.5),
                );
            }
        }

        mesh
    }
}
