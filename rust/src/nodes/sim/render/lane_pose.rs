//! Shared lane sampling helpers for agent render transforms.

use crate::simulation::network::lanes::Lane;
use godot::prelude::Vector3;

const LANE_TANGENT_LOOK_M: f32 = 2.0;

/// Samples a lane position and normalized tangent at `lane_d` metres along the lane.
pub(crate) fn sample_lane_pose(lane: &Lane, lane_d: f32) -> Option<(Vector3, Vector3)> {
    if lane.geometry.is_empty() {
        return None;
    }
    if lane.geometry.len() == 1 {
        return Some((lane.geometry[0], Vector3::FORWARD));
    }

    let dist = lane_d.clamp(0.0, lane.length.max(0.0));
    let pos = sample_lane_position(lane, dist)?;
    let tangent = sample_lane_tangent(lane, dist).unwrap_or(Vector3::FORWARD);
    Some((pos, tangent))
}

/// Samples a speed-scaled lane-change S-curve from `from_lane` into `to_lane`.
pub(crate) fn sample_lane_change_pose(
    from_lane: &Lane,
    to_lane: &Lane,
    lane_d: f32,
    start_d: f32,
    length_m: f32,
) -> Option<(Vector3, Vector3)> {
    if length_m <= 1e-5 || !length_m.is_finite() {
        return sample_lane_pose(to_lane, lane_d);
    }

    let progress = ((lane_d - start_d) / length_m).clamp(0.0, 1.0);
    if progress >= 1.0 {
        return sample_lane_pose(to_lane, lane_d);
    }

    let from_d = lane_d.clamp(0.0, from_lane.length.max(0.0));
    let to_d = lane_d.clamp(0.0, to_lane.length.max(0.0));
    let from_pos = sample_lane_position(from_lane, from_d)?;
    let to_pos = sample_lane_position(to_lane, to_d)?;
    let from_tangent = sample_lane_tangent(from_lane, from_d).unwrap_or(Vector3::FORWARD);
    let to_tangent = sample_lane_tangent(to_lane, to_d).unwrap_or(Vector3::FORWARD);

    let blend = smoothstep(progress);
    let blend_derivative = smoothstep_derivative(progress) / length_m;
    let lateral = to_pos - from_pos;
    let tangent_raw =
        from_tangent * (1.0 - blend) + to_tangent * blend + lateral * blend_derivative;
    let tangent = if tangent_raw.length_squared() > 1e-8 {
        tangent_raw.normalized()
    } else {
        to_tangent
    };

    Some((from_pos.lerp(to_pos, blend), tangent))
}

#[inline(always)]
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[inline(always)]
fn smoothstep_derivative(t: f32) -> f32 {
    6.0 * t * (1.0 - t)
}

fn sample_lane_position(lane: &Lane, dist: f32) -> Option<Vector3> {
    if lane.cum_dist.len() == lane.geometry.len() {
        let seg = lane
            .cum_dist
            .partition_point(|&d| d <= dist)
            .saturating_sub(1)
            .min(lane.geometry.len() - 2);
        return sample_lane_segment(lane, seg, dist);
    }

    let mut acc = 0.0;
    for seg in 0..lane.geometry.len() - 1 {
        let seg_len = lane.geometry[seg].distance_to(lane.geometry[seg + 1]);
        if acc + seg_len >= dist || seg == lane.geometry.len() - 2 {
            return sample_lane_segment_by_length(lane, seg, dist - acc, seg_len);
        }
        acc += seg_len;
    }

    None
}

fn sample_lane_tangent(lane: &Lane, dist: f32) -> Option<Vector3> {
    let look = LANE_TANGENT_LOOK_M.min((lane.length * 0.5).max(0.1));
    let back_d = (dist - look).max(0.0);
    let fwd_d = (dist + look).min(lane.length.max(0.0));
    let back = sample_lane_position(lane, back_d)?;
    let fwd = sample_lane_position(lane, fwd_d)?;
    let raw = fwd - back;
    if raw.length_squared() <= 1e-8 {
        return segment_tangent_at(lane, dist);
    }
    Some(raw.normalized())
}

fn segment_tangent_at(lane: &Lane, dist: f32) -> Option<Vector3> {
    if lane.cum_dist.len() == lane.geometry.len() {
        let seg = lane
            .cum_dist
            .partition_point(|&d| d <= dist)
            .saturating_sub(1)
            .min(lane.geometry.len() - 2);
        return segment_tangent(lane, seg);
    }

    let mut acc = 0.0;
    for seg in 0..lane.geometry.len() - 1 {
        let seg_len = lane.geometry[seg].distance_to(lane.geometry[seg + 1]);
        if acc + seg_len >= dist || seg == lane.geometry.len() - 2 {
            return segment_tangent(lane, seg);
        }
        acc += seg_len;
    }

    None
}

fn segment_tangent(lane: &Lane, seg: usize) -> Option<Vector3> {
    let raw = lane.geometry[seg + 1] - lane.geometry[seg];
    if raw.length_squared() <= 1e-8 {
        None
    } else {
        Some(raw.normalized())
    }
}

fn sample_lane_segment(lane: &Lane, seg: usize, dist: f32) -> Option<Vector3> {
    let seg_len = (lane.cum_dist[seg + 1] - lane.cum_dist[seg]).max(0.0);
    sample_lane_segment_by_length(lane, seg, dist - lane.cum_dist[seg], seg_len)
}

fn sample_lane_segment_by_length(
    lane: &Lane,
    seg: usize,
    local_dist: f32,
    seg_len: f32,
) -> Option<Vector3> {
    let p0 = lane.geometry[seg];
    let p1 = lane.geometry[seg + 1];
    let t = if seg_len > 1e-5 {
        (local_dist / seg_len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pos = p0.lerp(p1, t);
    Some(pos)
}

#[cfg(test)]
mod tests {
    use super::{sample_lane_change_pose, sample_lane_pose};
    use crate::simulation::network::lanes::{Lane, LaneType};
    use godot::prelude::Vector3;

    #[test]
    fn sample_lane_pose_uses_distance_along_geometry() {
        let geometry = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(3.0, 0.0, 0.0),
            Vector3::new(3.0, 0.0, 4.0),
        ];
        let lane = Lane {
            geometry,
            length: 7.0,
            cum_dist: vec![0.0, 3.0, 7.0],
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };

        let (pos, tangent) = sample_lane_pose(&lane, 5.0).expect("pose");

        assert!((pos.x - 3.0).abs() < f32::EPSILON);
        assert!((pos.z - 2.0).abs() < f32::EPSILON);
        assert!((tangent.z - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn sample_lane_pose_smooths_tangent_near_polyline_corner() {
        let geometry = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(3.0, 0.0, 0.0),
            Vector3::new(3.0, 0.0, 4.0),
        ];
        let lane = Lane {
            geometry,
            length: 7.0,
            cum_dist: vec![0.0, 3.0, 7.0],
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };

        let (_, tangent) = sample_lane_pose(&lane, 3.0).expect("pose");

        assert!(
            tangent.x > 0.0,
            "corner tangent should retain incoming direction"
        );
        assert!(
            tangent.z > 0.0,
            "corner tangent should include outgoing direction"
        );
    }

    #[test]
    fn sample_lane_change_pose_uses_s_curve_between_parallel_lanes() {
        let from_lane = Lane {
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(80.0, 0.0, 0.0)],
            length: 80.0,
            cum_dist: vec![0.0, 80.0],
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };
        let to_lane = Lane {
            geometry: vec![Vector3::new(0.0, 0.0, 3.5), Vector3::new(80.0, 0.0, 3.5)],
            length: 80.0,
            cum_dist: vec![0.0, 80.0],
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };

        let (pos, tangent) =
            sample_lane_change_pose(&from_lane, &to_lane, 30.0, 10.0, 40.0).expect("pose");

        assert!(
            pos.z > 0.0 && pos.z < 3.5,
            "lane-change position should be between source and target lanes"
        );
        assert!(
            tangent.z > 0.0,
            "lane-change tangent should include lateral motion"
        );
    }
}
