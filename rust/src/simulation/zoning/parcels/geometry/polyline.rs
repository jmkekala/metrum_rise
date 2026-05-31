//! Polyline sampling helpers for road-attached parcels.

use godot::prelude::{Vector2, Vector3};

pub(crate) fn sample_pos_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
    if points.is_empty() {
        return Vector2::ZERO;
    }
    if points.len() == 1 || total_len <= 1e-6 {
        return Vector2::new(points[0].x, points[0].z);
    }

    let target_s = s_m.clamp(0.0, total_len);
    let mut acc_len = 0.0;
    for window in points.windows(2) {
        let seg_len = window[0].distance_to(window[1]);
        if seg_len <= 1e-6 {
            continue;
        }
        if acc_len + seg_len >= target_s {
            let local_t = ((target_s - acc_len) / seg_len).clamp(0.0, 1.0);
            let p0 = Vector2::new(window[0].x, window[0].z);
            let p1 = Vector2::new(window[1].x, window[1].z);
            return p0.lerp(p1, local_t);
        }
        acc_len += seg_len;
    }
    let last = points.last().unwrap();
    Vector2::new(last.x, last.z)
}

pub(super) fn sample_tangent_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
    if points.len() <= 1 || total_len <= 1e-6 {
        return Vector2::RIGHT;
    }

    let target_s = s_m.clamp(0.0, total_len);
    let mut acc_len = 0.0;
    for window in points.windows(2) {
        let seg = Vector2::new(window[1].x - window[0].x, window[1].z - window[0].z);
        let seg_len = window[0].distance_to(window[1]);
        if seg_len <= 1e-6 || seg.length_squared() <= 1e-12 {
            continue;
        }
        if acc_len + seg_len >= target_s {
            return seg.normalized();
        }
        acc_len += seg_len;
    }

    for window in points.windows(2).rev() {
        let seg = Vector2::new(window[1].x - window[0].x, window[1].z - window[0].z);
        if seg.length_squared() > 1e-12 {
            return seg.normalized();
        }
    }
    Vector2::RIGHT
}
