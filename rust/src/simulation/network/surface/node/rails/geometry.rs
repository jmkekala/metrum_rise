// SPDX-License-Identifier: GPL-2.0-only

//! Canonical quantized rail geometry helpers.

use super::super::backend::RoadVec2;
use super::super::keys::{SurfaceXzKey, surface_overlay_grid_collinearity_error_bound};
use super::super::segments::raw_tuple_key_lies_on_segment as generated_point_key_lies_on_segment;
use super::topology::NodeRailPointKey;

pub(super) fn quantized_proper_segment_intersection(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
) -> Option<NodeRailPointKey> {
    if a == b || c == d {
        return None;
    }
    if a == c || a == d || b == c || b == d {
        return None;
    }
    if a.0.max(b.0) < c.0.min(d.0)
        || c.0.max(d.0) < a.0.min(b.0)
        || a.1.max(b.1) < c.1.min(d.1)
        || c.1.max(d.1) < a.1.min(b.1)
    {
        return None;
    }
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    let ad_x = i128::from(d.0 - a.0);
    let ad_z = i128::from(d.1 - a.1);
    let cd_x = i128::from(d.0 - c.0);
    let cd_z = i128::from(d.1 - c.1);
    let ca_x = -ac_x;
    let ca_z = -ac_z;
    let ab_c = ab_x * ac_z - ab_z * ac_x;
    let ab_d = ab_x * ad_z - ab_z * ad_x;
    let cd_a = cd_x * ca_z - cd_z * ca_x;
    let denominator = ab_d - ab_c;
    let cd_b = cd_a - denominator;
    quantized_proper_segment_intersection_from_orientations(
        a, b, c, d, ab_x, ab_z, ab_c, ab_d, cd_a, cd_b,
    )
}

#[allow(clippy::too_many_arguments)]
fn quantized_proper_segment_intersection_from_orientations(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
    ab_x: i128,
    ab_z: i128,
    ab_c: i128,
    ab_d: i128,
    cd_a: i128,
    cd_b: i128,
) -> Option<NodeRailPointKey> {
    if ab_c == 0 || ab_d == 0 || cd_a == 0 || cd_b == 0 {
        return None;
    }
    if (ab_c > 0) == (ab_d > 0) || (cd_a > 0) == (cd_b > 0) {
        return None;
    }

    let denominator = ab_d - ab_c;
    let x_num = i128::from(a.0) * denominator + ab_x * cd_a;
    let z_num = i128::from(a.1) * denominator + ab_z * cd_a;
    let intersection = (
        div_round_to_canonical_key_i128(x_num, denominator)?,
        div_round_to_canonical_key_i128(z_num, denominator)?,
    );
    if intersection == a
        || intersection == b
        || intersection == c
        || intersection == d
        || !generated_point_key_lies_on_segment(intersection, a, b)
        || !generated_point_key_lies_on_segment(intersection, c, d)
    {
        return None;
    }
    Some(intersection)
}

pub(super) fn append_quantized_segment_contact_points(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
    points: &mut Vec<NodeRailPointKey>,
) {
    if a != b && c != d && a.1 == b.1 && c.1 == d.1 {
        if a.1.abs_diff(c.1) <= 2 {
            append_parallel_axis_segment_contact_points(a, b, c, d, true, points);
        }
        return;
    }
    if a != b && c != d && a.0 == b.0 && c.0 == d.0 {
        if a.0.abs_diff(c.0) <= 2 {
            append_parallel_axis_segment_contact_points(a, b, c, d, false, points);
        }
        return;
    }
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    let ad_x = i128::from(d.0 - a.0);
    let ad_z = i128::from(d.1 - a.1);
    let cd_x = i128::from(d.0 - c.0);
    let cd_z = i128::from(d.1 - c.1);
    let ab_c = ab_x * ac_z - ab_z * ac_x;
    let ab_d = ab_x * ad_z - ab_z * ad_x;
    let cd_a = cd_z * ac_x - cd_x * ac_z;
    let denominator = ab_d - ab_c;
    let cd_b = cd_a - denominator;
    let distinct_segments = a != b && c != d;
    let bounds_overlap = a.0.max(b.0) >= c.0.min(d.0)
        && c.0.max(d.0) >= a.0.min(b.0)
        && a.1.max(b.1) >= c.1.min(d.1)
        && c.1.max(d.1) >= a.1.min(b.1);
    if distinct_segments
        && bounds_overlap
        && a != c
        && a != d
        && b != c
        && b != d
        && let Some(point) = quantized_proper_segment_intersection_from_orientations(
            a, b, c, d, ab_x, ab_z, ab_c, ab_d, cd_a, cd_b,
        )
    {
        points.push(point);
    }
    let ab_error = surface_overlay_grid_collinearity_error_bound(ab_x, ab_z);
    let cd_error = surface_overlay_grid_collinearity_error_bound(cd_x, cd_z);
    let a_contact = endpoint_lies_on_segment_with_cross(a, c, d, cd_x, cd_z, cd_a, cd_error);
    if a_contact {
        points.push(a);
    }
    let b_contact = endpoint_lies_on_segment_with_cross(b, c, d, cd_x, cd_z, cd_b, cd_error);
    if b_contact && b != a {
        points.push(b);
    }
    let c_contact = endpoint_lies_on_segment_with_cross(c, a, b, ab_x, ab_z, ab_c, ab_error);
    if c_contact && c != a && c != b {
        points.push(c);
    }
    let d_contact = endpoint_lies_on_segment_with_cross(d, a, b, ab_x, ab_z, ab_d, ab_error);
    if d_contact && d != a && d != b && d != c {
        points.push(d);
    }
}

fn append_parallel_axis_segment_contact_points(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
    horizontal: bool,
    points: &mut Vec<NodeRailPointKey>,
) {
    let coordinate = |point: NodeRailPointKey| if horizontal { point.0 } else { point.1 };
    let ab_min = coordinate(a).min(coordinate(b));
    let ab_max = coordinate(a).max(coordinate(b));
    let cd_min = coordinate(c).min(coordinate(d));
    let cd_max = coordinate(c).max(coordinate(d));
    let a_contact = cd_min <= coordinate(a) && coordinate(a) <= cd_max;
    if a_contact {
        points.push(a);
    }
    let b_contact = cd_min <= coordinate(b) && coordinate(b) <= cd_max;
    if b_contact {
        points.push(b);
    }
    let c_contact = ab_min <= coordinate(c) && coordinate(c) <= ab_max;
    if c_contact && c != a && c != b {
        points.push(c);
    }
    let d_contact = ab_min <= coordinate(d) && coordinate(d) <= ab_max;
    if d_contact && d != a && d != b && d != c {
        points.push(d);
    }
}

#[allow(clippy::too_many_arguments)]
fn endpoint_lies_on_segment_with_cross(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    dx: i128,
    dz: i128,
    cross: i128,
    error_bound: i128,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if (dx == 0 && dz == 0)
        || (dx != 0 && (point.0 < start.0.min(end.0) || start.0.max(end.0) < point.0))
        || (dz != 0 && (point.1 < start.1.min(end.1) || start.1.max(end.1) < point.1))
    {
        return false;
    }
    cross.abs() <= error_bound
}

fn div_round_to_canonical_key_i128(numerator: i128, denominator: i128) -> Option<i64> {
    if denominator == 0 {
        return None;
    }
    let (numerator, denominator) = if denominator < 0 {
        (-numerator, -denominator)
    } else {
        (numerator, denominator)
    };
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    i64::try_from(rounded).ok()
}
pub(super) fn road_point_key(point: RoadVec2) -> NodeRailPointKey {
    let key = SurfaceXzKey::from_road_xz(point);
    (key.x_key(), key.z_key())
}
pub(super) fn road_point_from_key(point: NodeRailPointKey) -> RoadVec2 {
    SurfaceXzKey::from_raw_keys(point.0, point.1).to_road_xz()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_segment_contacts_match_independent_reference_on_small_grid() {
        let keys = (-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| (x, z)))
            .collect::<Vec<_>>();
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        for &a in &keys {
            for &b in &keys {
                for &c in &keys {
                    for &d in &keys {
                        actual.clear();
                        append_quantized_segment_contact_points(a, b, c, d, &mut actual);
                        expected.clear();
                        if let Some(point) = quantized_proper_segment_intersection(a, b, c, d) {
                            expected.push(point);
                        }
                        for point in [a, b, c, d] {
                            let (start, end) = if point == a || point == b {
                                (c, d)
                            } else {
                                (a, b)
                            };
                            if generated_point_key_lies_on_segment(point, start, end)
                                && !expected.contains(&point)
                            {
                                expected.push(point);
                            }
                        }
                        assert_eq!(actual, expected, "segments {a:?}->{b:?}, {c:?}->{d:?}");
                    }
                }
            }
        }
    }
}
