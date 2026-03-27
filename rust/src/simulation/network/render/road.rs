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
const JUNCTION_MASK_CELL_SIZE: f32 = 0.125;
const JUNCTION_MASK_MARGIN: f32 = 0.5;
const JUNCTION_MASK_OVERLAP: f32 = JUNCTION_MASK_CELL_SIZE;
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
struct JunctionCorridor {
    handoff_dist: f32,
    tangent_xz: Vector2,
    road_half: f32,
    outer_half: f32,
}

struct JunctionFootprint {
    road_triangles: Vec<[Vector3; 3]>,
    sidewalk_quads: Vec<[Vector3; 4]>,
    road_loops: Vec<Vec<Vector3>>,
    outer_loops: Vec<Vec<Vector3>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GridPoint {
    x: i32,
    z: i32,
}

struct JunctionMaskContext {
    min_local: Vector2,
    width: usize,
    height: usize,
}

fn point_in_strip(local_point: Vector2, tangent: Vector2, length: f32, half_width: f32) -> bool {
    let normal = Vector2::new(-tangent.y, tangent.x);
    let forward = local_point.dot(tangent);
    let lateral = local_point.dot(normal).abs();
    forward >= -JUNCTION_MASK_CELL_SIZE
        && forward <= length + JUNCTION_MASK_OVERLAP
        && lateral <= half_width
}

fn build_junction_mask_context(corridors: &[JunctionCorridor]) -> Option<JunctionMaskContext> {
    let extent = corridors
        .iter()
        .map(|corridor| {
            (corridor.outer_handoff_dist + corridor.outer_half)
                .max(corridor.road_handoff_dist + corridor.road_half)
        })
        .fold(0.0f32, f32::max)
        + JUNCTION_MASK_MARGIN
        + JUNCTION_MASK_OVERLAP;
    if extent < MIN_JUNCTION_RADIUS {
        return None;
    }

    let min_local = Vector2::new(
        -extent - JUNCTION_MASK_CELL_SIZE,
        -extent - JUNCTION_MASK_CELL_SIZE,
    );
    let span = (extent + JUNCTION_MASK_CELL_SIZE) * 2.0;
    let width = ((span / JUNCTION_MASK_CELL_SIZE).ceil() as usize).max(1);
    let height = width;
    Some(JunctionMaskContext {
        min_local,
        width,
        height,
    })
}

fn rasterize_corridor_mask(
    ctx: &JunctionMaskContext,
    corridors: &[JunctionCorridor],
    outer: bool,
) -> Vec<bool> {
    let mut mask = vec![false; ctx.width * ctx.height];

    for z in 0..ctx.height {
        for x in 0..ctx.width {
            let local_point = ctx.min_local
                + Vector2::new(
                    (x as f32 + 0.5) * JUNCTION_MASK_CELL_SIZE,
                    (z as f32 + 0.5) * JUNCTION_MASK_CELL_SIZE,
                );
            let strip_occupied = |use_outer: bool| {
                corridors.iter().any(|corridor| {
                    let (length, half_width) = if use_outer {
                        (corridor.outer_handoff_dist, corridor.outer_half)
                    } else {
                        (corridor.road_handoff_dist, corridor.road_half)
                    };
                    point_in_strip(local_point, corridor.tangent_xz, length, half_width)
                })
            };

            mask[z * ctx.width + x] = strip_occupied(outer);
        }
    }

    mask
}

fn erode_mask(ctx: &JunctionMaskContext, mask: &[bool], steps: usize) -> Vec<bool> {
    let mut current = mask.to_vec();
    for _ in 0..steps {
        let mut next = current.clone();
        for z in 0..ctx.height {
            for x in 0..ctx.width {
                let index = z * ctx.width + x;
                if !current[index] {
                    next[index] = false;
                    continue;
                }

                let mut survives = true;
                'neighbors: for dz in -1isize..=1 {
                    for dx in -1isize..=1 {
                        if dx == 0 && dz == 0 {
                            continue;
                        }
                        let nx = x as isize + dx;
                        let nz = z as isize + dz;
                        if nx < 0
                            || nz < 0
                            || nx >= ctx.width as isize
                            || nz >= ctx.height as isize
                            || !current[nz as usize * ctx.width + nx as usize]
                        {
                            survives = false;
                            break 'neighbors;
                        }
                    }
                }
                next[index] = survives;
            }
        }
        current = next;
    }
    current
}

fn point_in_handoff_face(local_point: Vector2, corridor: &JunctionCorridor) -> bool {
    if !matches!(corridor.edge_class, EdgeClass::Standard | EdgeClass::Ramp) {
        return false;
    }

    let normal = Vector2::new(-corridor.tangent_xz.y, corridor.tangent_xz.x);
    let forward = local_point.dot(corridor.tangent_xz);
    let lateral = local_point.dot(normal).abs();
    forward >= corridor.road_handoff_dist - JUNCTION_MASK_CELL_SIZE
        && forward <= corridor.outer_handoff_dist + JUNCTION_MASK_CELL_SIZE
        && lateral <= corridor.road_half + JUNCTION_MASK_CELL_SIZE
}

fn remove_collinear_boundary(boundary: &mut Vec<Vector3>) {
    loop {
        if boundary.len() < 3 {
            return;
        }

        let mut removed_any = false;
        for i in 0..boundary.len() {
            let prev = boundary[(i + boundary.len() - 1) % boundary.len()];
            let current = boundary[i];
            let next = boundary[(i + 1) % boundary.len()];
            if triangle_cross_xz(prev, current, next).abs() <= 0.0001 {
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

fn extract_positive_loops_from_mask(
    ctx: &JunctionMaskContext,
    mask: &[bool],
    node_pos: Vector3,
    y: f32,
) -> Vec<Vec<Vector3>> {
    let mut next_by_start: HashMap<GridPoint, Vec<GridPoint>> = HashMap::new();
    let is_occupied = |x: isize, z: isize| -> bool {
        if x < 0 || z < 0 || x >= ctx.width as isize || z >= ctx.height as isize {
            return false;
        }
        mask[z as usize * ctx.width + x as usize]
    };

    let mut add_segment = |start: GridPoint, end: GridPoint| {
        next_by_start.entry(start).or_default().push(end);
    };

    for z in 0..ctx.height as isize {
        for x in 0..ctx.width as isize {
            if !is_occupied(x, z) {
                continue;
            }

            if !is_occupied(x, z - 1) {
                add_segment(
                    GridPoint {
                        x: x as i32,
                        z: z as i32,
                    },
                    GridPoint {
                        x: x as i32 + 1,
                        z: z as i32,
                    },
                );
            }
            if !is_occupied(x + 1, z) {
                add_segment(
                    GridPoint {
                        x: x as i32 + 1,
                        z: z as i32,
                    },
                    GridPoint {
                        x: x as i32 + 1,
                        z: z as i32 + 1,
                    },
                );
            }
            if !is_occupied(x, z + 1) {
                add_segment(
                    GridPoint {
                        x: x as i32 + 1,
                        z: z as i32 + 1,
                    },
                    GridPoint {
                        x: x as i32,
                        z: z as i32 + 1,
                    },
                );
            }
            if !is_occupied(x - 1, z) {
                add_segment(
                    GridPoint {
                        x: x as i32,
                        z: z as i32 + 1,
                    },
                    GridPoint {
                        x: x as i32,
                        z: z as i32,
                    },
                );
            }
        }
    }

    let mut loops = Vec::new();
    loop {
        let Some(start) = next_by_start
            .iter()
            .find_map(|(point, edges)| (!edges.is_empty()).then_some(*point))
        else {
            break;
        };

        let mut current = start;
        let mut loop_points = Vec::new();
        let mut guard = 0usize;
        let max_steps = next_by_start.len().saturating_mul(4).max(16);

        loop {
            loop_points.push(current);
            let Some(edges) = next_by_start.get_mut(&current) else {
                loop_points.clear();
                break;
            };
            let Some(next) = edges.pop() else {
                loop_points.clear();
                break;
            };
            current = next;
            guard += 1;
            if current == start {
                break;
            }
            if guard > max_steps {
                loop_points.clear();
                break;
            }
        }

        if loop_points.len() < 3 {
            continue;
        }

        let mut boundary: Vec<Vector3> = loop_points
            .into_iter()
            .map(|point| {
                let local = ctx.min_local
                    + Vector2::new(
                        point.x as f32 * JUNCTION_MASK_CELL_SIZE,
                        point.z as f32 * JUNCTION_MASK_CELL_SIZE,
                    );
                Vector3::new(node_pos.x + local.x, y, node_pos.z + local.y)
            })
            .collect();
        remove_collinear_boundary(&mut boundary);
        if boundary.len() < 3 {
            continue;
        }
        if polygon_signed_area_xz(&boundary) <= 0.001 {
            continue;
        }
        loops.push(boundary);
    }

    loops
}

fn greedy_mesh_mask(
    ctx: &JunctionMaskContext,
    mask: &[bool],
    node_pos: Vector3,
    y: f32,
) -> Vec<[Vector3; 4]> {
    let mut visited = vec![false; mask.len()];
    let mut quads = Vec::new();

    for z in 0..ctx.height {
        for x in 0..ctx.width {
            let index = z * ctx.width + x;
            if !mask[index] || visited[index] {
                continue;
            }

            let mut rect_width = 1usize;
            while x + rect_width < ctx.width {
                let next_index = z * ctx.width + x + rect_width;
                if !mask[next_index] || visited[next_index] {
                    break;
                }
                rect_width += 1;
            }

            let mut rect_height = 1usize;
            'height_search: loop {
                if z + rect_height >= ctx.height {
                    break;
                }
                for dx in 0..rect_width {
                    let next_index = (z + rect_height) * ctx.width + x + dx;
                    if !mask[next_index] || visited[next_index] {
                        break 'height_search;
                    }
                }
                rect_height += 1;
            }

            for dz in 0..rect_height {
                for dx in 0..rect_width {
                    visited[(z + dz) * ctx.width + x + dx] = true;
                }
            }

            let local_min = ctx.min_local
                + Vector2::new(
                    x as f32 * JUNCTION_MASK_CELL_SIZE,
                    z as f32 * JUNCTION_MASK_CELL_SIZE,
                );
            let local_max = local_min
                + Vector2::new(
                    rect_width as f32 * JUNCTION_MASK_CELL_SIZE,
                    rect_height as f32 * JUNCTION_MASK_CELL_SIZE,
                );
            quads.push([
                Vector3::new(node_pos.x + local_min.x, y, node_pos.z + local_min.y),
                Vector3::new(node_pos.x + local_max.x, y, node_pos.z + local_min.y),
                Vector3::new(node_pos.x + local_max.x, y, node_pos.z + local_max.y),
                Vector3::new(node_pos.x + local_min.x, y, node_pos.z + local_max.y),
            ]);
        }
    }

    quads
}

fn triangulate_loops(loops: &[Vec<Vector3>]) -> Vec<[Vector3; 3]> {
    let mut triangles = Vec::new();
    for boundary in loops {
        for [a_idx, b_idx, c_idx] in triangulate_boundary_xz(boundary) {
            triangles.push([boundary[a_idx], boundary[b_idx], boundary[c_idx]]);
        }
    }
    triangles
}

fn quads_to_triangles(quads: &[[Vector3; 4]]) -> Vec<[Vector3; 3]> {
    let mut triangles = Vec::with_capacity(quads.len() * 2);
    for [a, b, c, d] in quads {
        if triangle_cross_xz(*a, *b, *c).abs() >= 0.001 {
            triangles.push([*a, *b, *c]);
        }
        if triangle_cross_xz(*a, *c, *d).abs() >= 0.001 {
            triangles.push([*a, *c, *d]);
        }
    }
    triangles
}

fn normalize_angle(angle: f32) -> f32 {
    angle.rem_euclid(std::f32::consts::TAU)
}

fn angle_delta_ccw(start: f32, end: f32) -> f32 {
    (end - start).rem_euclid(std::f32::consts::TAU)
}

fn angle_in_interval(angle: f32, start: f32, end: f32) -> bool {
    angle_delta_ccw(start, angle) <= angle_delta_ccw(start, end) + 0.0001
}

fn build_outer_fill_sectors(corridors: &[JunctionCorridor]) -> Vec<(f32, f32)> {
    let mut corridor_angles: Vec<f32> = corridors
        .iter()
        .map(|corridor| normalize_angle(corridor.tangent_xz.y.atan2(corridor.tangent_xz.x)))
        .collect();
    corridor_angles.sort_by(|lhs, rhs| lhs.partial_cmp(rhs).unwrap_or(std::cmp::Ordering::Equal));

    let mut sectors = Vec::new();
    for i in 0..corridor_angles.len() {
        let start = corridor_angles[i];
        let end = corridor_angles[(i + 1) % corridor_angles.len()];
        let gap = angle_delta_ccw(start, end);
        if gap < std::f32::consts::PI - 0.05 {
            sectors.push((start, end));
        }
    }

    sectors
}

fn corridor_extent_on_ray(
    corridor: &JunctionCorridor,
    ray_dir: Vector2,
    outer: bool,
) -> Option<f32> {
    let (handoff_dist, half_width) = if outer {
        match corridor.edge_class {
            EdgeClass::Standard | EdgeClass::Ramp => {
                (corridor.outer_handoff_dist, corridor.outer_half)
            }
            EdgeClass::Bridge => (corridor.road_handoff_dist, corridor.road_half),
            EdgeClass::Tunnel => return None,
        }
    } else {
        (corridor.road_handoff_dist, corridor.road_half)
    };
    let forward = ray_dir.dot(corridor.tangent_xz);
    if forward <= 1e-6 {
        return None;
    }

    let normal = Vector2::new(-corridor.tangent_xz.y, corridor.tangent_xz.x);
    let lateral = ray_dir.dot(normal).abs();
    let cap_t = handoff_dist / forward;
    let side_t = if lateral > 1e-6 {
        half_width / lateral
    } else {
        f32::INFINITY
    };
    let extent = cap_t.min(side_t);
    (extent > 0.0001).then_some(extent)
}

fn rasterize_outer_footprint(
    ctx: &JunctionMaskContext,
    corridors: &[JunctionCorridor],
) -> Vec<bool> {
    let mut outer_mask = rasterize_corridor_mask(ctx, corridors, true);
    let fill_sectors = build_outer_fill_sectors(corridors);
    if fill_sectors.is_empty() {
        return outer_mask;
    }

    for z in 0..ctx.height {
        for x in 0..ctx.width {
            let index = z * ctx.width + x;
            if outer_mask[index] {
                continue;
            }

            let local_point = ctx.min_local
                + Vector2::new(
                    (x as f32 + 0.5) * JUNCTION_MASK_CELL_SIZE,
                    (z as f32 + 0.5) * JUNCTION_MASK_CELL_SIZE,
                );
            if local_point.length_squared() <= 1e-6 {
                continue;
            }

            let angle = normalize_angle(local_point.y.atan2(local_point.x));
            if !fill_sectors
                .iter()
                .any(|&(start, end)| angle_in_interval(angle, start, end))
            {
                continue;
            }

            let ray_dir = local_point.normalized();
            let radius = local_point.length();
            if let Some(outer_extent) = corridors
                .iter()
                .filter_map(|corridor| corridor_extent_on_ray(corridor, ray_dir, true))
                .max_by(|lhs, rhs| lhs.partial_cmp(rhs).unwrap_or(std::cmp::Ordering::Equal))
            {
                if radius <= outer_extent + JUNCTION_MASK_CELL_SIZE {
                    outer_mask[index] = true;
                }
            }
        }
    }

    outer_mask
}

fn build_junction_footprint(
    node_pos: Vector3,
    corridors: &[JunctionCorridor],
) -> Option<JunctionFootprint> {
    let ctx = build_junction_mask_context(corridors)?;
    let corridor_mask = rasterize_corridor_mask(&ctx, corridors, false);
    let outer_mask = rasterize_outer_footprint(&ctx, corridors);
    let shell_steps = (config::SIDEWALK_WIDTH / JUNCTION_MASK_CELL_SIZE).ceil() as usize;
    let eroded_outer = erode_mask(&ctx, &outer_mask, shell_steps);
    let road_mask: Vec<bool> = outer_mask
        .iter()
        .zip(corridor_mask.iter())
        .zip(eroded_outer.iter())
        .map(|((outer, corridor), core)| *outer && (*corridor || *core))
        .collect();
    let mut sidewalk_mask: Vec<bool> = outer_mask
        .iter()
        .zip(road_mask.iter())
        .map(|(outer, road)| *outer && !*road)
        .collect();
    let mut road_mask = road_mask;

    for z in 0..ctx.height {
        for x in 0..ctx.width {
            let index = z * ctx.width + x;
            if !sidewalk_mask[index] {
                continue;
            }

            let local_point = ctx.min_local
                + Vector2::new(
                    (x as f32 + 0.5) * JUNCTION_MASK_CELL_SIZE,
                    (z as f32 + 0.5) * JUNCTION_MASK_CELL_SIZE,
                );
            if corridors
                .iter()
                .any(|corridor| point_in_handoff_face(local_point, corridor))
            {
                sidewalk_mask[index] = false;
                road_mask[index] = true;
            }
        }
    }

    let road_y = node_pos.y + config::ROAD_H_OFFSET + JUNCTION_ROAD_BIAS;
    let sidewalk_y = node_pos.y + config::ROAD_H_OFFSET + JUNCTION_SIDEWALK_BIAS;
    let road_loops = extract_positive_loops_from_mask(&ctx, &road_mask, node_pos, road_y);
    let outer_loops = extract_positive_loops_from_mask(&ctx, &outer_mask, node_pos, sidewalk_y);
    let mut road_triangles = triangulate_loops(&road_loops);
    if road_triangles.is_empty() {
        road_triangles = quads_to_triangles(&greedy_mesh_mask(&ctx, &road_mask, node_pos, road_y));
    }
    let sidewalk_quads = greedy_mesh_mask(&ctx, &sidewalk_mask, node_pos, sidewalk_y);

    if road_triangles.is_empty() {
        return None;
    }

    Some(JunctionFootprint {
        road_triangles,
        sidewalk_quads,
        road_loops,
        outer_loops,
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
                candidate_idx != prev_idx
                    && candidate_idx != curr_idx
                    && candidate_idx != next_idx
                    && point_in_triangle_xz(boundary[candidate_idx], prev, curr, next)
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

fn point_in_polygon_xz(boundary: &[Vector3], point: Vector2) -> bool {
    if boundary.len() < 3 {
        return false;
    }

    let mut inside = false;
    for i in 0..boundary.len() {
        let a = boundary[i];
        let b = boundary[(i + 1) % boundary.len()];
        let crosses = (a.z > point.y) != (b.z > point.y);
        if !crosses {
            continue;
        }

        let dz = b.z - a.z;
        if dz.abs() <= 1e-6 {
            continue;
        }

        let x_at_scanline = a.x + (b.x - a.x) * (point.y - a.z) / dz;
        if point.x <= x_at_scanline + 0.0001 {
            inside = !inside;
        }
    }

    inside
}

fn ray_segment_intersection_distance(
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

fn exit_distance_from_boundary(
    boundary: &[Vector3],
    origin: Vector2,
    ray_dir: Vector2,
) -> Option<f32> {
    if !point_in_polygon_xz(boundary, origin) {
        return None;
    }

    let mut max_t = None;
    for i in 0..boundary.len() {
        let a = Vector2::new(boundary[i].x, boundary[i].z);
        let b = Vector2::new(
            boundary[(i + 1) % boundary.len()].x,
            boundary[(i + 1) % boundary.len()].z,
        );
        let Some(t) = ray_segment_intersection_distance(origin, ray_dir, a, b) else {
            continue;
        };
        max_t = Some(max_t.map_or(t, |current: f32| current.max(t)));
    }

    max_t.filter(|t| *t > 0.0001)
}

fn exit_distance_from_boundaries(
    boundaries: &[Vec<Vector3>],
    origin: Vector2,
    ray_dir: Vector2,
) -> Option<f32> {
    boundaries
        .iter()
        .filter_map(|boundary| exit_distance_from_boundary(boundary, origin, ray_dir))
        .max_by(|lhs, rhs| lhs.partial_cmp(rhs).unwrap_or(std::cmp::Ordering::Equal))
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
}

fn build_junction_corridor(
    graph: &RegionGraph,
    edge: &Edge,
    node_id: u32,
    is_start: bool,
    road_endpoint_distance: f32,
    outer_endpoint_distance: f32,
) -> Option<JunctionCorridor> {
    let road_target = if is_start {
        road_endpoint_distance
    } else {
        edge.physical_length - road_endpoint_distance
    };
    let outer_target = if is_start {
        outer_endpoint_distance
    } else {
        edge.physical_length - outer_endpoint_distance
    };
    let direction_target = road_target.max(outer_target);
    let direction_sample = sample_edge_at_distance(edge, direction_target)?;
    let road_sample = sample_edge_at_distance(edge, road_target)?;
    let outer_sample = sample_edge_at_distance(edge, outer_target)?;

    let outward_tangent = if is_start {
        direction_sample.tangent
    } else {
        -direction_sample.tangent
    };

    let node_pos = graph.nodes[node_id as usize].pos;
    let mut road_handoff_pos = road_sample.point;
    if road_handoff_pos.distance_to(node_pos) < 0.05 {
        road_handoff_pos = node_pos + outward_tangent * 0.05;
        road_handoff_pos.y = road_sample.point.y;
    }
    let mut outer_handoff_pos = outer_sample.point;
    if outer_handoff_pos.distance_to(node_pos) < 0.05 {
        outer_handoff_pos = node_pos + outward_tangent * 0.05;
        outer_handoff_pos.y = outer_sample.point.y;
    }
    let delta_xz = Vector2::new(
        direction_sample.point.x - node_pos.x,
        direction_sample.point.z - node_pos.z,
    );
    let length_xz = delta_xz.length();
    if length_xz <= 1e-6 {
        return None;
    }
    let road_delta_xz = Vector2::new(
        road_handoff_pos.x - node_pos.x,
        road_handoff_pos.z - node_pos.z,
    );
    let outer_delta_xz = Vector2::new(
        outer_handoff_pos.x - node_pos.x,
        outer_handoff_pos.z - node_pos.z,
    );

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
        road_handoff_dist: road_delta_xz.length().max(0.05),
        outer_handoff_dist: outer_delta_xz.length().max(0.05),
        tangent_xz: delta_xz / length_xz,
        road_half,
        outer_half,
        edge_class: edge.class,
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

            let road_trim = if is_start {
                endpoint_trims[edge_id].road_start
            } else {
                endpoint_trims[edge_id].road_end
            };
            let outer_trim = if is_start {
                endpoint_trims[edge_id].outer_start
            } else {
                endpoint_trims[edge_id].outer_end
            };
            if road_trim < MIN_JUNCTION_RADIUS {
                continue;
            }

            if let Some(corridor) =
                build_junction_corridor(graph, edge, node_id, is_start, road_trim, outer_trim)
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

fn edge_endpoint_axes_xz(edge: &Edge, is_start: bool) -> Option<(Vector2, Vector2)> {
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
    let outward = if is_start { forward } else { -forward };
    let side = Vector2::new(-forward.y, forward.x);
    Some((outward, side))
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
            let road_handoff = edge.width * 0.5;
            let outer_handoff = match edge.class {
                EdgeClass::Standard | EdgeClass::Ramp => road_handoff + config::SIDEWALK_WIDTH,
                EdgeClass::Bridge => road_handoff,
                EdgeClass::Tunnel => edge.width * 0.5,
            };
            let clipped_road_start = if matches!(edge.class, EdgeClass::Standard | EdgeClass::Ramp)
            {
                (edge.start_clip - config::SIDEWALK_WIDTH).max(road_handoff)
            } else {
                edge.start_clip.max(road_handoff)
            };
            let clipped_road_end = if matches!(edge.class, EdgeClass::Standard | EdgeClass::Ramp) {
                (edge.end_clip - config::SIDEWALK_WIDTH).max(road_handoff)
            } else {
                edge.end_clip.max(road_handoff)
            };
            let raw_road_start = if edge.class == EdgeClass::Tunnel {
                edge.start_clip
            } else if junction_nodes[start_node] {
                clipped_road_start
            } else {
                edge.start_clip
            };
            let raw_road_end = if edge.class == EdgeClass::Tunnel {
                edge.end_clip
            } else if junction_nodes[end_node] {
                clipped_road_end
            } else {
                edge.end_clip
            };
            let mut outer_start = if edge.class == EdgeClass::Tunnel {
                edge.start_clip
            } else if junction_nodes[start_node] {
                edge.start_clip.max(outer_handoff)
            } else {
                edge.start_clip
            };
            let mut outer_end = if edge.class == EdgeClass::Tunnel {
                edge.end_clip
            } else if junction_nodes[end_node] {
                edge.end_clip.max(outer_handoff)
            } else {
                edge.end_clip
            };

            let max_total = (edge.physical_length - MIN_EDGE_REMAINDER).max(0.0);
            if outer_start + outer_end > max_total && outer_start + outer_end > 1e-6 {
                let scale = max_total / (outer_start + outer_end);
                outer_start *= scale;
                outer_end *= scale;
            }
            let road_scale = if raw_road_start + raw_road_end > 1e-6 {
                (max_total / (raw_road_start + raw_road_end)).min(1.0)
            } else {
                1.0
            };
            let road_start = (raw_road_start * road_scale).min(outer_start);
            let road_end = (raw_road_end * road_scale).min(outer_end);

            endpoint_trims[edge_id] = EndpointTrim {
                road_start,
                road_end,
                outer_start,
                outer_end,
                outer_start_neg: outer_start,
                outer_start_pos: outer_start,
                outer_end_neg: outer_end,
                outer_end_pos: outer_end,
            };
        }

        let junction_footprints =
            build_junction_footprints(graph, &node_to_edges, &endpoint_trims, &junction_nodes);
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            let start_node = graph.get_valid_node(edge.start_node);
            let end_node = graph.get_valid_node(edge.end_node);

            if junction_nodes[start_node as usize] {
                if let Some(junction) = junction_footprints.get(&start_node) {
                    if let Some((outward, side)) = edge_endpoint_axes_xz(edge, true) {
                        let node_pos = graph.nodes[start_node as usize].pos;
                        let node_xz = Vector2::new(node_pos.x, node_pos.z);
                        if let Some(exit) =
                            exit_distance_from_boundaries(&junction.road_loops, node_xz, outward)
                        {
                            endpoint_trims[edge_id].road_start =
                                endpoint_trims[edge_id].road_start.max(exit);
                        }

                        if matches!(edge.class, EdgeClass::Standard | EdgeClass::Ramp) {
                            let probe_offset = edge.width * 0.5 + config::SIDEWALK_WIDTH * 0.5;
                            if let Some(exit) = exit_distance_from_boundaries(
                                &junction.outer_loops,
                                node_xz - side * probe_offset,
                                outward,
                            ) {
                                endpoint_trims[edge_id].outer_start_neg =
                                    endpoint_trims[edge_id].outer_start_neg.max(exit);
                            }
                            if let Some(exit) = exit_distance_from_boundaries(
                                &junction.outer_loops,
                                node_xz + side * probe_offset,
                                outward,
                            ) {
                                endpoint_trims[edge_id].outer_start_pos =
                                    endpoint_trims[edge_id].outer_start_pos.max(exit);
                            }
                        }
                    }
                }
            }

            if junction_nodes[end_node as usize] {
                if let Some(junction) = junction_footprints.get(&end_node) {
                    if let Some((outward, side)) = edge_endpoint_axes_xz(edge, false) {
                        let node_pos = graph.nodes[end_node as usize].pos;
                        let node_xz = Vector2::new(node_pos.x, node_pos.z);
                        if let Some(exit) =
                            exit_distance_from_boundaries(&junction.road_loops, node_xz, outward)
                        {
                            endpoint_trims[edge_id].road_end =
                                endpoint_trims[edge_id].road_end.max(exit);
                        }

                        if matches!(edge.class, EdgeClass::Standard | EdgeClass::Ramp) {
                            let probe_offset = edge.width * 0.5 + config::SIDEWALK_WIDTH * 0.5;
                            if let Some(exit) = exit_distance_from_boundaries(
                                &junction.outer_loops,
                                node_xz - side * probe_offset,
                                outward,
                            ) {
                                endpoint_trims[edge_id].outer_end_neg =
                                    endpoint_trims[edge_id].outer_end_neg.max(exit);
                            }
                            if let Some(exit) = exit_distance_from_boundaries(
                                &junction.outer_loops,
                                node_xz + side * probe_offset,
                                outward,
                            ) {
                                endpoint_trims[edge_id].outer_end_pos =
                                    endpoint_trims[edge_id].outer_end_pos.max(exit);
                            }
                        }
                    }
                }
            }

            let max_total = (edge.physical_length - MIN_EDGE_REMAINDER).max(0.0);
            if endpoint_trims[edge_id].road_start + endpoint_trims[edge_id].road_end > max_total
                && endpoint_trims[edge_id].road_start + endpoint_trims[edge_id].road_end > 1e-6
            {
                let scale = max_total
                    / (endpoint_trims[edge_id].road_start + endpoint_trims[edge_id].road_end);
                endpoint_trims[edge_id].road_start *= scale;
                endpoint_trims[edge_id].road_end *= scale;
            }

            {
                let trims = &mut endpoint_trims[edge_id];
                if trims.outer_start_neg + trims.outer_end_neg > max_total
                    && trims.outer_start_neg + trims.outer_end_neg > 1e-6
                {
                    let scale = max_total / (trims.outer_start_neg + trims.outer_end_neg);
                    trims.outer_start_neg *= scale;
                    trims.outer_end_neg *= scale;
                }
                if trims.outer_start_pos + trims.outer_end_pos > max_total
                    && trims.outer_start_pos + trims.outer_end_pos > 1e-6
                {
                    let scale = max_total / (trims.outer_start_pos + trims.outer_end_pos);
                    trims.outer_start_pos *= scale;
                    trims.outer_end_pos *= scale;
                }
            }

            endpoint_trims[edge_id].outer_start = endpoint_trims[edge_id]
                .outer_start_neg
                .max(endpoint_trims[edge_id].outer_start_pos);
            endpoint_trims[edge_id].outer_end = endpoint_trims[edge_id]
                .outer_end_neg
                .max(endpoint_trims[edge_id].outer_end_pos);

            let start_road_limit = endpoint_trims[edge_id]
                .outer_start_neg
                .min(endpoint_trims[edge_id].outer_start_pos);
            let end_road_limit = endpoint_trims[edge_id]
                .outer_end_neg
                .min(endpoint_trims[edge_id].outer_end_pos);
            endpoint_trims[edge_id].road_start =
                endpoint_trims[edge_id].road_start.min(start_road_limit);
            endpoint_trims[edge_id].road_end = endpoint_trims[edge_id].road_end.min(end_road_limit);
        }
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

            let trims = endpoint_trims[edge_id];
            let road_start_trim = trims.road_start;
            let road_end_trim = trims.road_end;
            let sidewalk_start_neg = trims.outer_start_neg;
            let sidewalk_start_pos = trims.outer_start_pos;
            let sidewalk_end_neg = trims.outer_end_neg;
            let sidewalk_end_pos = trims.outer_end_pos;
            let total_len = edge.physical_length;
            let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;
            if total_lanes <= 0.0 {
                continue;
            }

            let lane_w = edge.width / total_lanes;
            let road_outer = edge.width * 0.5;

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
                    if segment_end <= road_start_trim || segment_start >= total_len - road_end_trim
                    {
                        dist_acc += segment_len;
                        continue;
                    }

                    let mut t0 = 0.0f32;
                    let mut t1 = 1.0f32;
                    if segment_start < road_start_trim {
                        t0 = (road_start_trim - segment_start) / segment_len;
                    }
                    if segment_end > total_len - road_end_trim {
                        t1 = (total_len - road_end_trim - segment_start) / segment_len;
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

                    for (sign, start_trim, end_trim) in [
                        (-1.0f32, sidewalk_start_neg, sidewalk_end_neg),
                        (1.0f32, sidewalk_start_pos, sidewalk_end_pos),
                    ] {
                        if segment_end <= start_trim || segment_start >= total_len - end_trim {
                            continue;
                        }

                        let mut t0 = 0.0f32;
                        let mut t1 = 1.0f32;
                        if segment_start < start_trim {
                            t0 = (start_trim - segment_start) / segment_len;
                        }
                        if segment_end > total_len - end_trim {
                            t1 = (total_len - end_trim - segment_start) / segment_len;
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
                    if segment_end <= road_start_trim || segment_start >= total_len - road_end_trim
                    {
                        dist_acc += segment_len;
                        continue;
                    }

                    let mut t0 = 0.0f32;
                    let mut t1 = 1.0f32;
                    if segment_start < road_start_trim {
                        t0 = (road_start_trim - segment_start) / segment_len;
                    }
                    if segment_end > total_len - road_end_trim {
                        t1 = (total_len - road_end_trim - segment_start) / segment_len;
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
            for [a, b, c, d] in &footprint.sidewalk_quads {
                push_quad(
                    &mut mesh.vertices,
                    &mut mesh.normals,
                    &mut mesh.uvs,
                    &mut mesh.colors,
                    *a,
                    *b,
                    *c,
                    *d,
                    Vector2::new(0.0, 1.0),
                    Vector2::new(1.0, 1.0),
                    Vector2::new(1.0, 1.0),
                    Vector2::new(0.0, 1.0),
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
