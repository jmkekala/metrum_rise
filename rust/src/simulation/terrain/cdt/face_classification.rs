// SPDX-License-Identifier: GPL-2.0-only

//! Road-footprint ownership and post-triangulation face rejection.

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

// Inside a footprint group when an odd number of its loops contain the point: the boundary
// loops of one footprint never cross, so the innermost containing loop decides, whether it is
// an outer boundary, a hole (a closed block) or an island inside a hole (`ROAD-68-D`).
pub(super) fn point_inside_any_road_footprint(
    point: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    let contains = |road_loop: &CanonicalTerrainCdtRoadLoop| {
        road_loop_bounds_contain_point(road_loop, point)
            && point_in_polygon(point, &road_loop.vertices)
    };
    road_loops
        .iter()
        .filter(|road_loop| !road_loop.is_hole && contains(road_loop))
        .any(|outer_loop| {
            road_loops
                .iter()
                .filter(|road_loop| road_loop.footprint_group_id == outer_loop.footprint_group_id)
                .filter(|road_loop| contains(road_loop))
                .count()
                % 2
                == 1
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
