//! Canonical XZ and height quantization keys shared by surface stages.

use super::NodeOverlayPoint;
use super::backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, RoadVec3};
use godot::prelude::Vector3;

pub(crate) const SURFACE_XZ_KEY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;
pub(crate) const SURFACE_MM_PER_M: f64 = 1000.0;
pub(crate) const SURFACE_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct SurfaceXzKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct SurfaceXzSegmentKey {
    start: SurfaceXzKey,
    end: SurfaceXzKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SurfaceSegmentParameter {
    pub(crate) numerator: i128,
    pub(crate) denominator: i128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct SurfaceHeightMmKey(i64);

impl SurfaceXzKey {
    pub(crate) fn from_raw_keys(x_key: i64, z_key: i64) -> Self {
        Self { x_key, z_key }
    }

    pub(crate) fn from_raw_tuple(keys: (i64, i64)) -> Self {
        Self {
            x_key: keys.0,
            z_key: keys.1,
        }
    }

    pub(crate) fn from_road_xz(point: RoadVec2) -> Self {
        Self {
            x_key: Self::coordinate_key(point.x),
            z_key: Self::coordinate_key(point.y),
        }
    }

    pub(crate) fn from_world_xz(point: RoadVec3) -> Self {
        Self {
            x_key: Self::coordinate_key(point.x),
            z_key: Self::coordinate_key(point.z),
        }
    }

    pub(crate) fn from_godot_world_xz(point: Vector3) -> Self {
        Self {
            x_key: Self::coordinate_key(f64::from(point.x)),
            z_key: Self::coordinate_key(f64::from(point.z)),
        }
    }

    pub(crate) fn from_overlay_point(point: NodeOverlayPoint) -> Self {
        Self {
            x_key: Self::coordinate_key(point[0]),
            z_key: Self::coordinate_key(point[1]),
        }
    }

    pub(crate) fn coordinate_key(value_m: f64) -> i64 {
        (value_m * SURFACE_XZ_KEY_SCALE).round() as i64
    }

    pub(crate) fn coordinate_key_to_mm(value: i64) -> i64 {
        ((value as f64 / SURFACE_XZ_KEY_SCALE) * SURFACE_MM_PER_M).round() as i64
    }

    pub(crate) fn x_key(self) -> i64 {
        self.x_key
    }

    pub(crate) fn z_key(self) -> i64 {
        self.z_key
    }

    pub(crate) fn raw_tuple(self) -> (i64, i64) {
        (self.x_key, self.z_key)
    }

    pub(crate) fn x_mm(self) -> i64 {
        Self::coordinate_key_to_mm(self.x_key)
    }

    pub(crate) fn z_mm(self) -> i64 {
        Self::coordinate_key_to_mm(self.z_key)
    }

    pub(crate) fn to_road_xz(self) -> RoadVec2 {
        RoadVec2::new(
            self.x_key as f64 / SURFACE_XZ_KEY_SCALE,
            self.z_key as f64 / SURFACE_XZ_KEY_SCALE,
        )
    }

    pub(crate) fn segment_parameter_key(self, start: Self, end: Self) -> i128 {
        let dx = i128::from(end.x_key - start.x_key);
        let dz = i128::from(end.z_key - start.z_key);
        let px = i128::from(self.x_key - start.x_key);
        let pz = i128::from(self.z_key - start.z_key);
        px * dx + pz * dz
    }

    pub(crate) fn exact_line_parameter(
        self,
        start: Self,
        end: Self,
    ) -> Option<SurfaceSegmentParameter> {
        let dx = i128::from(end.x_key - start.x_key);
        let dz = i128::from(end.z_key - start.z_key);
        let length_squared = dx * dx + dz * dz;
        if length_squared == 0 || !self.collinear_with_segment(start, end) {
            return None;
        }
        SurfaceSegmentParameter::new(self.segment_parameter_key(start, end), length_squared)
    }

    pub(crate) fn overlay_segment_parameter(
        self,
        start: Self,
        end: Self,
    ) -> Option<SurfaceSegmentParameter> {
        if start == end || !self.lies_on_segment(start, end) {
            return None;
        }
        let dx = end.x_key - start.x_key;
        let dz = end.z_key - start.z_key;
        let (mut numerator, mut denominator) = if dx.abs() >= dz.abs() {
            (self.x_key - start.x_key, dx)
        } else {
            (self.z_key - start.z_key, dz)
        };
        if denominator == 0 {
            return None;
        }
        if denominator < 0 {
            numerator = -numerator;
            denominator = -denominator;
        }
        SurfaceSegmentParameter::new(i128::from(numerator), i128::from(denominator))
    }

    pub(crate) fn interpolate(start: Self, end: Self, parameter: SurfaceSegmentParameter) -> Self {
        Self {
            x_key: parameter.interpolate_i64(start.x_key, end.x_key),
            z_key: parameter.interpolate_i64(start.z_key, end.z_key),
        }
    }

    pub(crate) fn triangle_area2(a: Self, b: Self, c: Self) -> i128 {
        let ab_x = i128::from(b.x_key - a.x_key);
        let ab_z = i128::from(b.z_key - a.z_key);
        let ac_x = i128::from(c.x_key - a.x_key);
        let ac_z = i128::from(c.z_key - a.z_key);
        ab_x * ac_z - ab_z * ac_x
    }

    pub(crate) fn raw_tuple_triangle_area2(a: (i64, i64), b: (i64, i64), c: (i64, i64)) -> i128 {
        Self::triangle_area2(
            Self::from_raw_tuple(a),
            Self::from_raw_tuple(b),
            Self::from_raw_tuple(c),
        )
    }

    pub(crate) fn raw_tuple_triangle_area_m2_abs(
        a: (i64, i64),
        b: (i64, i64),
        c: (i64, i64),
    ) -> f64 {
        Self::raw_tuple_triangle_area2(a, b, c).unsigned_abs() as f64 * 0.5
            / SURFACE_XZ_KEY_SCALE.powi(2)
    }

    pub(crate) fn collinear_with_segment(self, start: Self, end: Self) -> bool {
        Self::triangle_area2(start, end, self) == 0
    }

    pub(crate) fn collinear_with_overlay_grid_segment(self, start: Self, end: Self) -> bool {
        let cross = Self::triangle_area2(start, end, self);
        if cross == 0 {
            return true;
        }
        let dx = i128::from(end.x_key - start.x_key);
        let dz = i128::from(end.z_key - start.z_key);
        cross.abs() <= surface_overlay_grid_collinearity_error_bound(dx, dz)
    }

    pub(crate) fn lies_on_segment(self, start: Self, end: Self) -> bool {
        if self == start || self == end {
            return true;
        }
        if start == end || !self.collinear_with_overlay_grid_segment(start, end) {
            return false;
        }
        self.inside_segment_bounds(start, end, true)
    }

    #[cfg(test)]
    pub(crate) fn lies_on_open_segment(self, start: Self, end: Self) -> bool {
        if self == start || self == end || start == end {
            return false;
        }
        self.collinear_with_overlay_grid_segment(start, end)
            && self.inside_segment_bounds(start, end, false)
    }

    pub(crate) fn lies_exactly_on_segment(self, start: Self, end: Self) -> bool {
        if self == start || self == end {
            return true;
        }
        if start == end || !self.collinear_with_segment(start, end) {
            return false;
        }
        self.inside_segment_bounds(start, end, true)
    }

    fn inside_segment_bounds(self, start: Self, end: Self, include_endpoints: bool) -> bool {
        let inside_x = if start.x_key == end.x_key {
            self.x_key == start.x_key
        } else if include_endpoints {
            self.x_key >= start.x_key.min(end.x_key) && self.x_key <= start.x_key.max(end.x_key)
        } else {
            self.x_key > start.x_key.min(end.x_key) && self.x_key < start.x_key.max(end.x_key)
        };
        let inside_z = if start.z_key == end.z_key {
            self.z_key == start.z_key
        } else if include_endpoints {
            self.z_key >= start.z_key.min(end.z_key) && self.z_key <= start.z_key.max(end.z_key)
        } else {
            self.z_key > start.z_key.min(end.z_key) && self.z_key < start.z_key.max(end.z_key)
        };
        inside_x && inside_z
    }
}

impl SurfaceXzSegmentKey {
    pub(crate) fn new(a: SurfaceXzKey, b: SurfaceXzKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    pub(crate) fn non_degenerate(a: SurfaceXzKey, b: SurfaceXzKey) -> Option<Self> {
        (a != b).then(|| Self::new(a, b))
    }

    pub(crate) fn start(self) -> SurfaceXzKey {
        self.start
    }

    pub(crate) fn end(self) -> SurfaceXzKey {
        self.end
    }
}

impl SurfaceSegmentParameter {
    pub(crate) fn zero() -> Self {
        Self {
            numerator: 0,
            denominator: 1,
        }
    }

    pub(crate) fn one() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }

    pub(crate) fn new(numerator: i128, denominator: i128) -> Option<Self> {
        (denominator > 0).then_some(Self {
            numerator,
            denominator,
        })
    }

    pub(crate) fn as_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }

    pub(crate) fn min(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }

    pub(crate) fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }

    pub(crate) fn interpolate_i64(self, start: i64, end: i64) -> i64 {
        round_div_i128(
            i128::from(start) * self.denominator
                + (i128::from(end) - i128::from(start)) * self.numerator,
            self.denominator,
        )
    }
}

impl Ord for SurfaceSegmentParameter {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.numerator * other.denominator).cmp(&(other.numerator * self.denominator))
    }
}

impl PartialOrd for SurfaceSegmentParameter {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) fn surface_overlay_grid_collinearity_error_bound(dx: i128, dz: i128) -> i128 {
    // Source contours and backend-owned shapes are both projected to the overlay integer grid.
    // A point that is exactly on a source segment before projection can land within this
    // determinant envelope after independent endpoint rounding; this is representation noding,
    // not owner or height repair.
    (dx.abs() + dz.abs()) * 2
}

fn round_div_i128(numerator: i128, denominator: i128) -> i64 {
    debug_assert!(denominator > 0);
    let half = denominator / 2;
    let rounded = if numerator >= 0 {
        (numerator + half) / denominator
    } else {
        (numerator - half) / denominator
    };
    rounded as i64
}

impl SurfaceHeightMmKey {
    pub(crate) fn from_m_f64(value_m: f64) -> Self {
        Self((value_m * SURFACE_MM_PER_M).round() as i64)
    }

    pub(crate) fn from_m_f32(value_m: f32) -> Self {
        Self::from_m_f64(f64::from(value_m))
    }

    pub(crate) fn as_i64(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_world_overlay_conversions_share_the_same_xz_key() {
        let road = SurfaceXzKey::from_road_xz(RoadVec2::new(1.25, -3.5));
        let world = SurfaceXzKey::from_world_xz(RoadVec3::new(1.25, 42.0, -3.5));
        let overlay = SurfaceXzKey::from_overlay_point([1.25, -3.5]);

        assert_eq!(road, world);
        assert_eq!(road, overlay);
        assert_eq!(road, SurfaceXzKey::from_raw_tuple(road.raw_tuple()));
    }

    #[test]
    fn normalized_segment_key_is_direction_independent() {
        let a = SurfaceXzKey::from_road_xz(RoadVec2::new(-1.0, 0.5));
        let b = SurfaceXzKey::from_road_xz(RoadVec2::new(2.0, -0.5));

        assert_eq!(
            SurfaceXzSegmentKey::new(a, b),
            SurfaceXzSegmentKey::new(b, a)
        );
        assert_eq!(SurfaceXzSegmentKey::non_degenerate(a, a), None);
    }

    #[test]
    fn segment_predicates_separate_exact_and_overlay_grid_collinearity() {
        let start = SurfaceXzKey::from_raw_keys(0, 0);
        let end = SurfaceXzKey::from_raw_keys(10_000, 10_000);
        let exact_middle = SurfaceXzKey::from_raw_keys(5_000, 5_000);
        let near_middle = SurfaceXzKey::from_raw_keys(5_000, 5_001);

        assert!(exact_middle.lies_on_segment(start, end));
        assert!(exact_middle.lies_exactly_on_segment(start, end));
        assert!(near_middle.lies_on_segment(start, end));
        assert!(!near_middle.lies_exactly_on_segment(start, end));
        assert!(!start.lies_on_open_segment(start, end));
        assert!(exact_middle.lies_on_open_segment(start, end));
    }

    #[test]
    fn segment_parameters_separate_exact_line_and_overlay_grid_membership() {
        let start = SurfaceXzKey::from_raw_keys(0, 0);
        let end = SurfaceXzKey::from_raw_keys(10, 10);
        let exact_middle = SurfaceXzKey::from_raw_keys(5, 5);
        let near_middle = SurfaceXzKey::from_raw_keys(5, 6);
        let beyond_end = SurfaceXzKey::from_raw_keys(15, 15);

        let exact_parameter = exact_middle
            .exact_line_parameter(start, end)
            .expect("exact midpoint should have a bounded parameter");
        assert_eq!(exact_parameter.numerator, 100);
        assert_eq!(exact_parameter.denominator, 200);
        assert_eq!(exact_parameter.as_f64(), 0.5);
        assert!(exact_parameter >= SurfaceSegmentParameter::zero());
        assert!(exact_parameter <= SurfaceSegmentParameter::one());
        assert!(near_middle.exact_line_parameter(start, end).is_none());

        let overlay_parameter = near_middle
            .overlay_segment_parameter(start, end)
            .expect("overlay-grid tolerant midpoint should stay on the canonical segment");
        assert_eq!(overlay_parameter.as_f64(), 0.5);

        let unbounded_parameter = beyond_end
            .exact_line_parameter(start, end)
            .expect("collinear point beyond the endpoint should have a line parameter");
        assert!(unbounded_parameter > SurfaceSegmentParameter::one());
        assert!(beyond_end.overlay_segment_parameter(start, end).is_none());
    }

    #[test]
    fn segment_interpolation_uses_canonical_integer_rounding() {
        let start = SurfaceXzKey::from_raw_keys(0, 0);
        let end = SurfaceXzKey::from_raw_keys(3, 9);
        let half = SurfaceSegmentParameter::new(1, 2).expect("positive denominator is valid");

        assert_eq!(
            SurfaceXzKey::interpolate(start, end, half),
            SurfaceXzKey::from_raw_keys(2, 5)
        );
        assert_eq!(half.interpolate_i64(100, 103), 102);
        assert_eq!(half.interpolate_i64(-100, -103), -102);
    }

    #[test]
    fn raw_tuple_triangle_area_preserves_signed_winding() {
        assert_eq!(
            SurfaceXzKey::raw_tuple_triangle_area2((0, 0), (2, 0), (0, 3)),
            6
        );
        assert_eq!(
            SurfaceXzKey::raw_tuple_triangle_area2((0, 0), (0, 3), (2, 0)),
            -6
        );
    }

    #[test]
    fn raw_tuple_triangle_area_m2_abs_uses_canonical_scale() {
        let one_meter = SurfaceXzKey::coordinate_key(1.0);
        let area_m2 =
            SurfaceXzKey::raw_tuple_triangle_area_m2_abs((0, 0), (one_meter, 0), (0, one_meter));

        assert!((area_m2 - 0.5).abs() <= f64::EPSILON);
    }
}
