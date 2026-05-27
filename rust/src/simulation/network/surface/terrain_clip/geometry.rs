//! Local terrain-clip overlay geometry helpers.

use super::super::backend::RoadVec3;
use super::super::{
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour,
    NodeOverlayPoint, RoadSurfaceSystem,
    backend::ROAD_OVERLAY_COORDINATE_SCALE,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
    segments::{exact_line_parameter, overlay_point_key},
};
use super::model::{
    OverlaySegmentParameter, RoadSurfaceTerrainClipLoop, TerrainClipContourCompactError,
};

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

    fn overlay_contour_from_world_points(points_world: &[RoadVec3]) -> NodeOverlayContour {
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

    fn overlay_point_from_world_point(point: RoadVec3) -> NodeOverlayPoint {
        [
            (point.x * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (point.z * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    pub(super) fn compact_overlay_contour_by_key(
        contour: &NodeOverlayContour,
    ) -> Result<NodeOverlayContour, TerrainClipContourCompactError> {
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
        remove_repeated_overlay_point_spurs(&mut compact)?;
        Ok(compact)
    }

    pub(super) fn world_points_same_for_boundary(a: RoadVec3, b: RoadVec3) -> bool {
        Self::terrain_clip_world_key(a) == Self::terrain_clip_world_key(b)
    }

    pub(super) fn canonical_numeric_dust_boundary_point(
        a: RoadVec3,
        b: RoadVec3,
    ) -> Option<RoadVec3> {
        if !Self::overlay_heights_equal(a.y, b.y) {
            return None;
        }
        let dx = a.x - b.x;
        let dz = a.z - b.z;
        let dust_width_m = f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M);
        if dx * dx + dz * dz > dust_width_m * dust_width_m {
            return None;
        }

        let a_key = Self::terrain_clip_world_key(a);
        let b_key = Self::terrain_clip_world_key(b);
        let key = if a_key <= b_key { a_key } else { b_key };
        let point = key.to_road_xz();
        Some(RoadVec3::new(
            point.x,
            SurfaceHeightMmKey::from_m_f64(a.y).as_i64() as f64 / 1000.0,
            point.y,
        ))
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

    pub(super) fn overlay_height_key(height_m: f64) -> i64 {
        (height_m * NODE_OVERLAY_SCALE).round() as i64
    }

    pub(super) fn overlay_heights_equal(a: f64, b: f64) -> bool {
        Self::overlay_height_key(a) == Self::overlay_height_key(b)
    }

    pub(super) fn terrain_clip_overlay_key(point: NodeOverlayPoint) -> SurfaceXzKey {
        overlay_point_key(point)
    }

    pub(super) fn terrain_clip_world_key(point: RoadVec3) -> SurfaceXzKey {
        SurfaceXzKey::from_world_xz(point)
    }
}

pub(super) fn interpolate_height_f64(start_y: f64, end_y: f64, t: f64) -> f64 {
    start_y + (end_y - start_y) * t
}

pub(super) fn overlay_segment_length_m(start: NodeOverlayPoint, end: NodeOverlayPoint) -> f64 {
    let dx = end[0] - start[0];
    let dz = end[1] - start[1];
    (dx * dx + dz * dz).sqrt()
}

fn overlay_points_same_for_boundary(a: NodeOverlayPoint, b: NodeOverlayPoint) -> bool {
    overlay_point_key(a) == overlay_point_key(b)
}

fn remove_repeated_overlay_point_spurs(
    points: &mut NodeOverlayContour,
) -> Result<(), TerrainClipContourCompactError> {
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
        let dust_budget = f64::from(RoadSurfaceSystem::overlay_numeric_area_budget_m2(
            RoadSurfaceSystem::overlay_contour_perimeter_m(points),
            points.len(),
        ));
        let keep_remainder = remainder.len() >= 3
            && (cycle.len() < 3
                || cycle_area <= dust_budget
                || (remainder_area <= dust_budget && remainder_area >= cycle_area));
        let keep_cycle = cycle.len() >= 3
            && (remainder.len() < 3
                || remainder_area <= dust_budget
                || (cycle_area <= dust_budget && cycle_area >= remainder_area));
        if keep_remainder {
            *points = remainder;
        } else if keep_cycle {
            *points = cycle;
        } else {
            let key = RoadSurfaceSystem::terrain_clip_overlay_key(points[first]);
            return Err(TerrainClipContourCompactError {
                x_key: key.x_key(),
                z_key: key.z_key(),
                cycle_area_m2: cycle_area,
                remainder_area_m2: remainder_area,
                dust_budget_m2: dust_budget,
            });
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_overlay_contour_rejects_large_repeated_point_cycle() {
        let contour = vec![
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 4.0],
            [0.0, 0.0],
            [-4.0, 4.0],
            [-4.0, 0.0],
        ];

        let error = RoadSurfaceSystem::compact_overlay_contour_by_key(&contour)
            .expect_err("large repeated-point cycles must be geometry errors");

        assert!(
            error.cycle_area_m2 > error.dust_budget_m2
                && error.remainder_area_m2 > error.dust_budget_m2,
            "large repeated-point cycles should reject instead of being chosen by area preference: {error:?}"
        );
    }

    #[test]
    fn compact_overlay_contour_discards_subbudget_repeated_point_dust() {
        let contour = vec![
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 0.000001],
            [2.0, 0.0],
            [2.0, 1.0],
            [0.0, 1.0],
        ];

        let compact = RoadSurfaceSystem::compact_overlay_contour_by_key(&contour)
            .expect("sub-budget repeated-point dust may be discarded");

        assert!(
            first_repeated_overlay_point_pair(&compact).is_none(),
            "sub-budget cleanup must remove the repeated point"
        );
        assert!(
            RoadSurfaceSystem::overlay_contour_area_f64(&compact).abs() > 1.0,
            "sub-budget cleanup must preserve the real contour area"
        );
    }
}
