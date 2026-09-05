// SPDX-License-Identifier: GPL-2.0-only

//! Road-footprint ownership and post-triangulation face rejection.

use std::collections::BTreeMap;

use super::*;

pub(super) fn terrain_cdt_loop_bounds(points: &[TerrainCdtVertex]) -> TerrainCdtLoopBounds {
    let mut bounds = TerrainCdtLoopBounds {
        min_x: f64::INFINITY,
        min_z: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_z: f64::NEG_INFINITY,
    };
    for point in points {
        bounds.min_x = bounds.min_x.min(point.x);
        bounds.min_z = bounds.min_z.min(point.z);
        bounds.max_x = bounds.max_x.max(point.x);
        bounds.max_z = bounds.max_z.max(point.z);
    }
    bounds
}

pub(super) fn point_in_polygon(point: TerrainCdtVertex, polygon: &[TerrainCdtVertex]) -> bool {
    let mut inside = false;
    let Some(mut previous) = polygon.len().checked_sub(1) else {
        return false;
    };
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        if (a.z > point.z) != (b.z > point.z) {
            let intersection_x = (b.x - a.x) * (point.z - a.z) / (b.z - a.z) + a.x;
            if point.x < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

pub(super) fn point_inside_any_road_footprint(
    point: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    road_loops
        .iter()
        .filter(|road_loop| !road_loop.is_hole)
        .filter(|road_loop| road_loop_bounds_contain_point(road_loop, point))
        .filter(|road_loop| point_in_polygon(point, &road_loop.vertices))
        .any(|outer_loop| {
            !road_loops.iter().any(|hole_loop| {
                hole_loop.is_hole
                    && hole_loop.footprint_group_id == outer_loop.footprint_group_id
                    && road_loop_bounds_contain_point(hole_loop, point)
                    && point_in_polygon(point, &hole_loop.vertices)
            })
        })
}

fn road_loop_bounds_contain_point(
    road_loop: &CanonicalTerrainCdtRoadLoop,
    point: TerrainCdtVertex,
) -> bool {
    point.x >= road_loop.min_x - CDT_EPSILON_M
        && point.x <= road_loop.max_x + CDT_EPSILON_M
        && point.z >= road_loop.min_z - CDT_EPSILON_M
        && point.z <= road_loop.max_z + CDT_EPSILON_M
}

pub(super) fn terrain_triangle_is_road_owned(
    triangle: [usize; 3],
    points: [TerrainCdtVertex; 3],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    terrain_triangle_overlaps_any_road_footprint(
        triangle,
        points,
        road_constraint_sources,
        road_loops,
    )
}

pub(super) fn constrained_cdt_face_is_road_owned(
    points: [TerrainCdtVertex; 3],
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    let triangle_bounds = triangle_xz_bounds(points);
    if !road_loops
        .iter()
        .any(|road_loop| bounds_overlap_loop(triangle_bounds, road_loop))
    {
        return false;
    }

    // Every road-loop boundary is a CDT constraint, so a valid face cannot cross between road and
    // terrain ownership. Its centroid therefore classifies the whole face, including the exact
    // side of a seam constraint. Builder post-processing still rejects faces crossing any
    // constraint that Spade did not preserve.
    point_inside_any_road_footprint(centroid(points), road_loops)
}

fn terrain_triangle_overlaps_any_road_footprint(
    triangle: [usize; 3],
    points: [TerrainCdtVertex; 3],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    let triangle_bounds = triangle_xz_bounds(points);
    if !road_loops
        .iter()
        .any(|road_loop| bounds_overlap_loop(triangle_bounds, road_loop))
    {
        return false;
    }
    if point_strictly_inside_any_road_footprint(centroid(points), road_loops) {
        return true;
    }
    if points
        .iter()
        .any(|point| point_strictly_inside_any_road_footprint(*point, road_loops))
    {
        return true;
    }
    if triangle_edges_enter_road_footprint(triangle, points, road_constraint_sources, road_loops) {
        return true;
    }
    for road_loop in road_loops {
        if !bounds_overlap_loop(triangle_bounds, road_loop) {
            continue;
        }
        if road_loop.vertices.iter().any(|vertex| {
            point_strictly_inside_triangle_xz(*vertex, points)
                && !road_loop_boundary_vertex_is_non_road_hole_only(*vertex, road_loop, road_loops)
        }) {
            return true;
        }
        if triangle_edges_cross_road_loop_boundary(points, road_loop) {
            return true;
        }
    }
    false
}

fn triangle_edges_enter_road_footprint(
    triangle: [usize; 3],
    points: [TerrainCdtVertex; 3],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    (0..3).any(|edge_index| {
        let edge = normalize_edge_array(triangle[edge_index], triangle[(edge_index + 1) % 3]);
        if road_constraint_sources.contains_key(&edge) {
            return false;
        }
        segment_interior_enters_road_footprint(
            points[edge_index],
            points[(edge_index + 1) % 3],
            road_loops,
        )
    })
}

fn segment_interior_enters_road_footprint(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    let segment_len_sq =
        (end.x - start.x) * (end.x - start.x) + (end.z - start.z) * (end.z - start.z);
    if segment_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M {
        return false;
    }

    let mut parameters = vec![0.0, 1.0];
    for road_loop in road_loops {
        if start.x.min(end.x) > road_loop.max_x + CDT_EPSILON_M
            || road_loop.min_x > start.x.max(end.x) + CDT_EPSILON_M
            || start.z.min(end.z) > road_loop.max_z + CDT_EPSILON_M
            || road_loop.min_z > start.z.max(end.z) + CDT_EPSILON_M
        {
            continue;
        }
        for loop_edge_index in 0..road_loop.vertices.len() {
            for intersection in segment_intersections(
                start,
                end,
                road_loop.vertices[loop_edge_index],
                road_loop.vertices[(loop_edge_index + 1) % road_loop.vertices.len()],
            )
            .into_iter()
            .flatten()
            {
                let t = segment_parameter(start, end, intersection.x, intersection.z);
                if unit_interval_contains(t) {
                    parameters.push(clamp_unit(t));
                }
            }
        }
    }
    sort_dedup_segment_parameters(&mut parameters);

    parameters.windows(2).any(|window| {
        let start_t = window[0];
        let end_t = window[1];
        if end_t - start_t <= CDT_EPSILON_M {
            return false;
        }
        let mid_t = (start_t + end_t) * 0.5;
        if mid_t <= CDT_EPSILON_M || mid_t >= 1.0 - CDT_EPSILON_M {
            return false;
        }
        point_strictly_inside_any_road_footprint(interpolate_vertex(start, end, mid_t), road_loops)
    })
}

fn sort_dedup_segment_parameters(parameters: &mut Vec<f64>) {
    parameters.sort_by(|a, b| a.total_cmp(b));
    if parameters.len() < 2 {
        return;
    }
    let mut write_index = 1;
    for read_index in 1..parameters.len() {
        if (parameters[read_index] - parameters[write_index - 1]).abs() <= CDT_EPSILON_M {
            continue;
        }
        parameters[write_index] = parameters[read_index];
        write_index += 1;
    }
    parameters.truncate(write_index);
}

pub(super) fn point_strictly_inside_any_road_footprint(
    point: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    point_inside_any_road_footprint(point, road_loops)
        && !point_on_any_road_loop_boundary(point, road_loops)
}

fn triangle_xz_bounds(points: [TerrainCdtVertex; 3]) -> TerrainCdtLoopBounds {
    terrain_cdt_loop_bounds(&points)
}

fn bounds_overlap_loop(
    bounds: TerrainCdtLoopBounds,
    road_loop: &CanonicalTerrainCdtRoadLoop,
) -> bool {
    bounds.min_x <= road_loop.max_x + CDT_EPSILON_M
        && road_loop.min_x <= bounds.max_x + CDT_EPSILON_M
        && bounds.min_z <= road_loop.max_z + CDT_EPSILON_M
        && road_loop.min_z <= bounds.max_z + CDT_EPSILON_M
}

fn road_loop_boundary_vertex_is_non_road_hole_only(
    vertex: TerrainCdtVertex,
    road_loop: &CanonicalTerrainCdtRoadLoop,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    road_loop.is_hole && !point_inside_any_road_footprint(vertex, road_loops)
}

fn triangle_edges_cross_road_loop_boundary(
    points: [TerrainCdtVertex; 3],
    road_loop: &CanonicalTerrainCdtRoadLoop,
) -> bool {
    for triangle_edge_index in 0..3 {
        let triangle_start = points[triangle_edge_index];
        let triangle_end = points[(triangle_edge_index + 1) % 3];
        if triangle_start.x.min(triangle_end.x) > road_loop.max_x + CDT_EPSILON_M
            || road_loop.min_x > triangle_start.x.max(triangle_end.x) + CDT_EPSILON_M
            || triangle_start.z.min(triangle_end.z) > road_loop.max_z + CDT_EPSILON_M
            || road_loop.min_z > triangle_start.z.max(triangle_end.z) + CDT_EPSILON_M
        {
            continue;
        }
        for loop_edge_index in 0..road_loop.vertices.len() {
            if segments_cross_at_strict_interiors(
                triangle_start,
                triangle_end,
                road_loop.vertices[loop_edge_index],
                road_loop.vertices[(loop_edge_index + 1) % road_loop.vertices.len()],
            ) {
                return true;
            }
        }
    }
    false
}

pub(super) fn triangle_crosses_any_road_constraint(
    points: [TerrainCdtVertex; 3],
    road_constraint_edges: &[[usize; 2]],
    vertices: &[TerrainCdtVertex],
) -> bool {
    road_constraint_edges
        .iter()
        .any(|edge| triangle_crosses_road_constraint(points, vertices[edge[0]], vertices[edge[1]]))
}

pub(super) fn triangle_crosses_road_constraint(
    points: [TerrainCdtVertex; 3],
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
) -> bool {
    if !triangle_bounds_overlap_segment(points, start, end) {
        return false;
    }
    if point_strictly_inside_triangle_xz(start, points)
        || point_strictly_inside_triangle_xz(end, points)
    {
        return true;
    }
    for edge_index in 0..3 {
        if segments_cross_at_strict_interiors(
            points[edge_index],
            points[(edge_index + 1) % 3],
            start,
            end,
        ) {
            return true;
        }
    }
    false
}

fn triangle_bounds_overlap_segment(
    points: [TerrainCdtVertex; 3],
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
) -> bool {
    let bounds = triangle_xz_bounds(points);
    bounds.min_x <= start.x.max(end.x) + CDT_EPSILON_M
        && start.x.min(end.x) <= bounds.max_x + CDT_EPSILON_M
        && bounds.min_z <= start.z.max(end.z) + CDT_EPSILON_M
        && start.z.min(end.z) <= bounds.max_z + CDT_EPSILON_M
}

fn segments_cross_at_strict_interiors(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> bool {
    if !segment_bounds_overlap(first_start, first_end, second_start, second_end) {
        return false;
    }
    segment_intersections(first_start, first_end, second_start, second_end)
        .into_iter()
        .flatten()
        .any(|intersection| {
            point_is_strict_segment_interior(intersection, first_start, first_end)
                && point_is_strict_segment_interior(intersection, second_start, second_end)
        })
}

fn point_is_strict_segment_interior(
    point: TerrainCdtVertex,
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
) -> bool {
    let t = segment_parameter(start, end, point.x, point.z);
    t > CDT_EPSILON_M && t < 1.0 - CDT_EPSILON_M
}

fn point_strictly_inside_triangle_xz(
    point: TerrainCdtVertex,
    triangle: [TerrainCdtVertex; 3],
) -> bool {
    let area = cross_xz(
        triangle[1].x - triangle[0].x,
        triangle[1].z - triangle[0].z,
        triangle[2].x - triangle[0].x,
        triangle[2].z - triangle[0].z,
    );
    if area.abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
        return false;
    }
    let signs = [
        cross_xz(
            triangle[1].x - triangle[0].x,
            triangle[1].z - triangle[0].z,
            point.x - triangle[0].x,
            point.z - triangle[0].z,
        ),
        cross_xz(
            triangle[2].x - triangle[1].x,
            triangle[2].z - triangle[1].z,
            point.x - triangle[1].x,
            point.z - triangle[1].z,
        ),
        cross_xz(
            triangle[0].x - triangle[2].x,
            triangle[0].z - triangle[2].z,
            point.x - triangle[2].x,
            point.z - triangle[2].z,
        ),
    ];
    if area > 0.0 {
        signs.iter().all(|sign| *sign > CDT_EPSILON_M)
    } else {
        signs.iter().all(|sign| *sign < -CDT_EPSILON_M)
    }
}

pub(super) fn road_exterior_support_point(
    point: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    !point_inside_any_road_footprint(point, road_loops)
        && !point_on_any_road_loop_boundary(point, road_loops)
}

fn point_on_any_road_loop_boundary(
    point: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    road_loops.iter().any(|road_loop| {
        road_loop
            .vertices
            .iter()
            .copied()
            .zip(road_loop.vertices.iter().copied().cycle().skip(1))
            .take(road_loop.vertices.len())
            .any(|(start, end)| {
                source_sample_parameter_on_road_constraint(start, end, point).is_some()
            })
    })
}

pub(super) fn centroid(points: [TerrainCdtVertex; 3]) -> TerrainCdtVertex {
    TerrainCdtVertex::new(
        (points[0].x + points[1].x + points[2].x) / 3.0,
        (points[0].height_m + points[1].height_m + points[2].height_m) / 3.0,
        (points[0].z + points[1].z + points[2].z) / 3.0,
    )
}
