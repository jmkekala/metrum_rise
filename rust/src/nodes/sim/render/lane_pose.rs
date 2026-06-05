//! Shared lane sampling helpers for agent render transforms.

use crate::simulation::network::lanes::Lane;
use godot::prelude::Vector3;

/// Samples a lane position and normalized tangent at `lane_d` metres along the lane.
pub(crate) fn sample_lane_pose(lane: &Lane, lane_d: f32) -> Option<(Vector3, Vector3)> {
    if lane.geometry.is_empty() {
        return None;
    }
    if lane.geometry.len() == 1 {
        return Some((lane.geometry[0], Vector3::FORWARD));
    }

    let dist = lane_d.clamp(0.0, lane.length.max(0.0));
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

fn sample_lane_segment(lane: &Lane, seg: usize, dist: f32) -> Option<(Vector3, Vector3)> {
    let seg_len = (lane.cum_dist[seg + 1] - lane.cum_dist[seg]).max(0.0);
    sample_lane_segment_by_length(lane, seg, dist - lane.cum_dist[seg], seg_len)
}

fn sample_lane_segment_by_length(
    lane: &Lane,
    seg: usize,
    local_dist: f32,
    seg_len: f32,
) -> Option<(Vector3, Vector3)> {
    let p0 = lane.geometry[seg];
    let p1 = lane.geometry[seg + 1];
    let t = if seg_len > 1e-5 {
        (local_dist / seg_len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pos = p0.lerp(p1, t);
    let raw = p1 - p0;
    if raw.length_squared() <= 1e-8 {
        return Some((pos, Vector3::FORWARD));
    }
    Some((pos, raw.normalized()))
}

#[cfg(test)]
mod tests {
    use super::sample_lane_pose;
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
}
