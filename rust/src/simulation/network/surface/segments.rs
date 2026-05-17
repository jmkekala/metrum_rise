//! Canonical segment membership, parameter, and interpolation helpers.

use super::{
    NodeOverlayPoint, arrangement,
    backend::RoadVec2,
    keys::{SurfaceSegmentParameter, SurfaceXzKey},
};

/// Returns whether a canonical key lies on a segment using overlay-grid tolerant collinearity.
pub(crate) fn key_lies_on_segment(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> bool {
    point.lies_on_segment(start, end)
}

/// Returns whether a canonical key lies exactly on the integer segment.
pub(crate) fn key_lies_exactly_on_segment(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> bool {
    point.lies_exactly_on_segment(start, end)
}

/// Returns whether a canonical key is exactly collinear with the integer segment line.
pub(crate) fn key_collinear_with_segment(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> bool {
    point.collinear_with_segment(start, end)
}

/// Returns whether a canonical key is collinear within the overlay-grid tolerance envelope.
pub(crate) fn key_collinear_with_overlay_grid_segment(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> bool {
    point.collinear_with_overlay_grid_segment(start, end)
}

/// Returns the unbounded exact-line parameter for a canonical point on a non-zero segment line.
pub(crate) fn exact_line_parameter(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> Option<SurfaceSegmentParameter> {
    point.exact_line_parameter(start, end)
}

/// Returns a bounded segment parameter using overlay-grid tolerant membership.
pub(crate) fn overlay_segment_parameter(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> Option<SurfaceSegmentParameter> {
    point.overlay_segment_parameter(start, end)
}

/// Returns the raw dot-product ordering key for sorting points along a segment.
pub(crate) fn segment_parameter_key(
    start: SurfaceXzKey,
    end: SurfaceXzKey,
    point: SurfaceXzKey,
) -> i128 {
    point.segment_parameter_key(start, end)
}

/// Interpolates a canonical XZ key with the same integer rounding used by surface stages.
pub(crate) fn interpolate_key(
    start: SurfaceXzKey,
    end: SurfaceXzKey,
    parameter: SurfaceSegmentParameter,
) -> SurfaceXzKey {
    SurfaceXzKey::interpolate(start, end, parameter)
}

/// Interpolates millimetre heights with the canonical segment-parameter rounding rule.
pub(crate) fn interpolate_height_i64(
    start_height: i64,
    end_height: i64,
    parameter: SurfaceSegmentParameter,
) -> i64 {
    parameter.interpolate_i64(start_height, end_height)
}

/// Converts a raw canonical `(x, z)` tuple into the shared XZ key type.
pub(crate) fn raw_tuple_key(keys: (i64, i64)) -> SurfaceXzKey {
    SurfaceXzKey::from_raw_tuple(keys)
}

/// Returns whether a raw tuple key lies on a raw tuple segment.
pub(crate) fn raw_tuple_key_lies_on_segment(
    point: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
) -> bool {
    key_lies_on_segment(
        raw_tuple_key(point),
        raw_tuple_key(start),
        raw_tuple_key(end),
    )
}

/// Returns whether a raw tuple key lies exactly on a raw tuple segment.
pub(crate) fn raw_tuple_key_lies_exactly_on_segment(
    point: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
) -> bool {
    key_lies_exactly_on_segment(
        raw_tuple_key(point),
        raw_tuple_key(start),
        raw_tuple_key(end),
    )
}

/// Returns whether a raw tuple key's quantization cell intersects a raw tuple segment.
pub(crate) fn raw_tuple_quantization_cell_intersects_segment(
    point: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
    neighbor_radius_units: i128,
) -> bool {
    raw_tuple_key(point).quantization_cell_intersects_segment(
        raw_tuple_key(start),
        raw_tuple_key(end),
        neighbor_radius_units,
    )
}

/// Returns the raw dot-product ordering key for raw tuple segment points.
pub(crate) fn raw_tuple_segment_parameter_key(
    start: (i64, i64),
    end: (i64, i64),
    point: (i64, i64),
) -> i128 {
    segment_parameter_key(
        raw_tuple_key(start),
        raw_tuple_key(end),
        raw_tuple_key(point),
    )
}

/// Converts a road-space XZ point into the shared XZ key type.
pub(crate) fn road_xz_key(point: RoadVec2) -> SurfaceXzKey {
    SurfaceXzKey::from_road_xz(point)
}

/// Returns whether a road-space XZ point lies exactly on a road-space XZ segment.
pub(crate) fn road_xz_lies_exactly_on_segment(
    point: RoadVec2,
    start: RoadVec2,
    end: RoadVec2,
) -> bool {
    key_lies_exactly_on_segment(road_xz_key(point), road_xz_key(start), road_xz_key(end))
}

/// Converts a node arrangement key into the shared XZ key type.
pub(crate) fn arrangement_key(key: arrangement::NodeArrangementKey) -> SurfaceXzKey {
    SurfaceXzKey::from_raw_keys(key.x_key(), key.z_key())
}

/// Returns whether a node arrangement key lies on a node arrangement segment.
pub(crate) fn arrangement_key_lies_on_segment(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> bool {
    key_lies_on_segment(
        arrangement_key(point),
        arrangement_key(start),
        arrangement_key(end),
    )
}

/// Returns the overlay-grid tolerant segment parameter for node arrangement keys.
pub(crate) fn arrangement_key_overlay_segment_parameter(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> Option<SurfaceSegmentParameter> {
    overlay_segment_parameter(
        arrangement_key(point),
        arrangement_key(start),
        arrangement_key(end),
    )
}

/// Converts an overlay point into the shared XZ key type.
pub(crate) fn overlay_point_key(point: NodeOverlayPoint) -> SurfaceXzKey {
    SurfaceXzKey::from_overlay_point(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_on_segment_succeeds() {
        let start = SurfaceXzKey::from_raw_keys(0, 0);
        let end = SurfaceXzKey::from_raw_keys(10, 10);
        let point = SurfaceXzKey::from_raw_keys(5, 5);

        assert!(key_lies_on_segment(point, start, end));
    }

    #[test]
    fn point_off_segment_fails() {
        let start = SurfaceXzKey::from_raw_keys(0, 0);
        let end = SurfaceXzKey::from_raw_keys(10, 10);
        let point = SurfaceXzKey::from_raw_keys(5, 10);

        assert!(!key_lies_on_segment(point, start, end));
    }

    #[test]
    fn reversed_segment_parameter_normalizes() {
        let start = SurfaceXzKey::from_raw_keys(10, 0);
        let end = SurfaceXzKey::from_raw_keys(0, 0);
        let point = SurfaceXzKey::from_raw_keys(4, 0);

        let parameter = overlay_segment_parameter(point, start, end)
            .expect("point lies on the reversed segment");
        assert_eq!(parameter.numerator, 6);
        assert_eq!(parameter.denominator, 10);
    }

    #[test]
    fn exact_collinear_out_of_bounds_fails_bounded_membership() {
        let start = SurfaceXzKey::from_raw_keys(0, 0);
        let end = SurfaceXzKey::from_raw_keys(10, 10);
        let point = SurfaceXzKey::from_raw_keys(15, 15);

        assert!(exact_line_parameter(point, start, end).is_some());
        assert!(!key_lies_on_segment(point, start, end));
        assert!(overlay_segment_parameter(point, start, end).is_none());
    }

    #[test]
    fn interpolation_reconstructs_key_and_height() {
        let start = SurfaceXzKey::from_raw_keys(0, 0);
        let end = SurfaceXzKey::from_raw_keys(10, 20);
        let parameter = SurfaceSegmentParameter::new(3, 10).expect("positive denominator");

        assert_eq!(
            interpolate_key(start, end, parameter),
            SurfaceXzKey::from_raw_keys(3, 6)
        );
        assert_eq!(interpolate_height_i64(100, 200, parameter), 130);
    }

    #[test]
    fn raw_tuple_exact_membership_requires_exact_collinearity() {
        let start = (0, 0);
        let end = (10_000, 10_000);

        assert!(raw_tuple_key_lies_exactly_on_segment(
            (5_000, 5_000),
            start,
            end
        ));
        assert!(!raw_tuple_key_lies_exactly_on_segment(
            (5_000, 5_001),
            start,
            end
        ));
    }

    #[test]
    fn raw_tuple_quantization_cell_intersection_uses_caller_radius() {
        let start = (0, 0);
        let end = (10, 10);

        assert!(raw_tuple_quantization_cell_intersects_segment(
            (5, 6),
            start,
            end,
            1
        ));
        assert!(!raw_tuple_quantization_cell_intersects_segment(
            (5, 9),
            start,
            end,
            1
        ));
    }

    #[test]
    fn road_xz_exact_membership_uses_canonical_keys() {
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 2.0);

        assert!(road_xz_lies_exactly_on_segment(
            RoadVec2::new(1.0, 1.0),
            start,
            end
        ));
        assert!(!road_xz_lies_exactly_on_segment(
            RoadVec2::new(1.0, 1.001),
            start,
            end
        ));
    }
}
