//! Local terrain-clip overlay geometry helpers.

use super::super::{
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour,
    NodeOverlayPoint, RoadSurfaceSystem,
    backend::ROAD_OVERLAY_COORDINATE_SCALE,
    keys::SurfaceXzKey,
    segments::{exact_line_parameter, overlay_point_key},
};
use super::model::{OverlaySegmentParameter, RoadSurfaceTerrainClipLoop};
use godot::prelude::Vector3;

const NODE_OVERLAY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;

impl RoadSurfaceSystem {
    pub(super) fn overlay_contours_from_terrain_clip_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for boundary_loop in boundary_loops {
            let contour = Self::overlay_contour_from_world_points(&boundary_loop.points_world);
            if Self::overlay_contour_area(&contour).abs() > NODE_OVERLAY_MIN_AREA_M2 {
                contours.push(contour);
            }
        }
        contours
    }

    fn overlay_contour_from_world_points(points_world: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points_world.len());
        for point in points_world {
            let overlay_point = Self::overlay_point_from_world_point(*point);
            if contour
                .last()
                .is_none_or(|last: &NodeOverlayPoint| *last != overlay_point)
            {
                contour.push(overlay_point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    fn overlay_point_from_world_point(point: Vector3) -> NodeOverlayPoint {
        [
            (f64::from(point.x) * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (f64::from(point.z) * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    pub(super) fn compact_overlay_contour_by_key(
        contour: &NodeOverlayContour,
    ) -> NodeOverlayContour {
        let mut compact = Vec::with_capacity(contour.len());
        for &point in contour {
            if compact
                .last()
                .is_none_or(|last| !overlay_points_same_for_boundary(*last, point))
            {
                compact.push(point);
            }
        }
        while compact.len() >= 2
            && overlay_points_same_for_boundary(*compact.first().unwrap(), *compact.last().unwrap())
        {
            compact.pop();
        }
        remove_repeated_overlay_point_spurs(&mut compact);
        compact
    }

    pub(super) fn world_points_same_for_boundary(a: Vector3, b: Vector3) -> bool {
        Self::terrain_clip_world_key(a) == Self::terrain_clip_world_key(b)
    }

    pub(super) fn overlay_line_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        Self::overlay_segment_parameter_unbounded(point, start, end)
            .map(OverlaySegmentParameter::as_f64)
            .or_else(|| Self::overlay_numeric_dust_line_parameter(point, start, end))
    }

    pub(super) fn overlay_segment_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        if let Some(parameter) = Self::overlay_segment_parameter_unbounded(point, start, end) {
            return Self::overlay_segment_parameter_with_endpoint_dust(
                parameter.as_f64(),
                start,
                end,
            );
        }
        let parameter = Self::overlay_numeric_dust_line_parameter(point, start, end)?;
        Self::overlay_segment_parameter_with_endpoint_dust(parameter, start, end)
    }

    fn overlay_segment_parameter_unbounded(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<OverlaySegmentParameter> {
        let start_key = Self::terrain_clip_overlay_key(start);
        let end_key = Self::terrain_clip_overlay_key(end);
        let point_key = Self::terrain_clip_overlay_key(point);
        exact_line_parameter(point_key, start_key, end_key)
    }

    fn overlay_numeric_dust_line_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        // `i_overlay` may emit split vertices microscopically off the source edge's integer line.
        // The source edge still owns the height; this only recovers its interval parameter.
        let dx = end[0] - start[0];
        let dz = end[1] - start[1];
        let length_squared = dx * dx + dz * dz;
        if length_squared == 0.0 {
            return None;
        }
        let point_dx = point[0] - start[0];
        let point_dz = point[1] - start[1];
        let length = length_squared.sqrt();
        let cross = point_dx * dz - point_dz * dx;
        if cross.abs() > f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * length {
            return None;
        }
        Some((point_dx * dx + point_dz * dz) / length_squared)
    }

    fn overlay_segment_parameter_with_endpoint_dust(
        parameter: f64,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        let endpoint_dust = Self::overlay_endpoint_dust_parameter(start, end)?;
        (parameter >= -endpoint_dust && parameter <= 1.0 + endpoint_dust)
            .then_some(parameter.clamp(0.0, 1.0))
    }

    pub(super) fn overlay_endpoint_dust_parameter(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        let dx = end[0] - start[0];
        let dz = end[1] - start[1];
        let length = (dx * dx + dz * dz).sqrt();
        if length == 0.0 {
            return None;
        }
        Some(f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) / length)
    }

    pub(super) fn overlay_height_key(height_m: f32) -> i64 {
        (f64::from(height_m) * NODE_OVERLAY_SCALE).round() as i64
    }

    pub(super) fn overlay_heights_equal(a: f32, b: f32) -> bool {
        Self::overlay_height_key(a) == Self::overlay_height_key(b)
    }

    pub(super) fn terrain_clip_overlay_key(point: NodeOverlayPoint) -> SurfaceXzKey {
        overlay_point_key(point)
    }

    pub(super) fn terrain_clip_world_key(point: Vector3) -> SurfaceXzKey {
        SurfaceXzKey::from_godot_world_xz(point)
    }
}

pub(super) fn interpolate_height_f64(start_y: f32, end_y: f32, t: f64) -> f32 {
    (f64::from(start_y) + f64::from(end_y - start_y) * t) as f32
}

pub(super) fn overlay_segment_length_m(start: NodeOverlayPoint, end: NodeOverlayPoint) -> f64 {
    let dx = end[0] - start[0];
    let dz = end[1] - start[1];
    (dx * dx + dz * dz).sqrt()
}

fn overlay_points_same_for_boundary(a: NodeOverlayPoint, b: NodeOverlayPoint) -> bool {
    overlay_point_key(a) == overlay_point_key(b)
}

fn remove_repeated_overlay_point_spurs(points: &mut NodeOverlayContour) {
    while points.len() >= 3 {
        let Some((first, second)) = first_repeated_overlay_point_pair(points) else {
            break;
        };
        let cycle = points[first..second].to_vec();
        let mut remainder = Vec::with_capacity(points.len() - (second - first));
        remainder.extend_from_slice(&points[..=first]);
        remainder.extend_from_slice(&points[second + 1..]);

        let cycle_area = RoadSurfaceSystem::overlay_contour_area_f64(&cycle).abs();
        let remainder_area = RoadSurfaceSystem::overlay_contour_area_f64(&remainder).abs();
        if remainder.len() >= 3 && remainder_area >= cycle_area {
            *points = remainder;
        } else if cycle.len() >= 3 {
            *points = cycle;
        } else {
            break;
        }
    }
}

fn first_repeated_overlay_point_pair(points: &NodeOverlayContour) -> Option<(usize, usize)> {
    for first in 0..points.len() {
        for second in first + 2..points.len() {
            if first == 0 && second + 1 == points.len() {
                continue;
            }
            if overlay_points_same_for_boundary(points[first], points[second]) {
                return Some((first, second));
            }
        }
    }
    None
}

pub(super) fn contour_area_delta_after_removing_vertex(
    contour: &NodeOverlayContour,
    index: usize,
) -> Option<f64> {
    if contour.len() <= 3 || index >= contour.len() {
        return None;
    }
    let mut reduced = Vec::with_capacity(contour.len() - 1);
    reduced.extend_from_slice(&contour[..index]);
    reduced.extend_from_slice(&contour[index + 1..]);
    Some(RoadSurfaceSystem::overlay_contour_area_f64(&reduced).abs())
}

pub(super) fn interpolate_overlay_point(
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
    t: f64,
) -> NodeOverlayPoint {
    [
        start[0] + (end[0] - start[0]) * t,
        start[1] + (end[1] - start[1]) * t,
    ]
}
