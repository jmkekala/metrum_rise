//! Canonical quantized rail geometry helpers.

use super::super::backend::RoadVec2;
use super::super::keys::SurfaceXzKey;
use super::super::segments::raw_tuple_key_lies_on_segment as generated_point_key_lies_on_segment;
use super::topology::NodeRailPointKey;

pub(super) fn remove_generated_contour_spikes(keys: &mut Vec<NodeRailPointKey>) {
    keys.dedup();
    loop {
        if keys.len() < 3 {
            return;
        }
        let mut removed = false;
        for index in 0..keys.len() {
            let previous = if index == 0 {
                keys.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == keys.len() {
                0
            } else {
                index + 1
            };
            if keys[previous] == keys[next] {
                keys.remove(index);
                removed = true;
                break;
            }
        }
        if !removed {
            return;
        }
    }
}
fn generated_triangle_double_area(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
) -> i128 {
    SurfaceXzKey::raw_tuple_triangle_area2(a, b, c)
}
pub(super) fn quantized_proper_segment_intersection(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
) -> Option<NodeRailPointKey> {
    if a == b || c == d {
        return None;
    }
    let ab_c = generated_triangle_double_area(a, b, c);
    let ab_d = generated_triangle_double_area(a, b, d);
    let cd_a = generated_triangle_double_area(c, d, a);
    let cd_b = generated_triangle_double_area(c, d, b);
    if ab_c == 0 || ab_d == 0 || cd_a == 0 || cd_b == 0 {
        return None;
    }
    if (ab_c > 0) == (ab_d > 0) || (cd_a > 0) == (cd_b > 0) {
        return None;
    }

    let r_x = i128::from(b.0 - a.0);
    let r_z = i128::from(b.1 - a.1);
    let s_x = i128::from(d.0 - c.0);
    let s_z = i128::from(d.1 - c.1);
    let offset_x = i128::from(c.0 - a.0);
    let offset_z = i128::from(c.1 - a.1);
    let denominator = r_x * s_z - r_z * s_x;
    if denominator == 0 {
        return None;
    }
    let numerator = offset_x * s_z - offset_z * s_x;
    let x_num = i128::from(a.0) * denominator + r_x * numerator;
    let z_num = i128::from(a.1) * denominator + r_z * numerator;
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
