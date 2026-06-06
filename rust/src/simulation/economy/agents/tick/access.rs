//! Local building access geometry, legality, and access-time helpers.

mod geometry;
mod legality;
mod path;
mod timing;

pub(super) use geometry::{
    local_access_point, local_access_side_label, projected_lane_distance_for_entrance,
};
pub(super) use legality::{planned_attach_is_legal, planned_detach_is_legal};
#[cfg(test)]
pub(super) use path::LocalAccessPath;
pub(super) use path::{
    advance_along_local_access_path, local_access_path, local_access_target_segment,
};
pub(super) use timing::{
    direct_frontage_segment_time_s, frontage_time_s, local_access_distance, local_access_time_s,
};

#[cfg(test)]
mod tests {
    use super::{LocalAccessPath, advance_along_local_access_path};
    use godot::prelude::Vector2;

    #[test]
    fn test_opposite_side_car_egress_finishes_when_already_at_lane_endpoint() {
        let path = LocalAccessPath {
            points: [
                Vector2::new(173.14, -47.41),
                Vector2::new(177.24, -47.41),
                Vector2::new(184.24, -47.41),
                Vector2::new(182.49, -47.41),
            ],
            count: 4,
        };

        let current = path.points[3];
        let (next, reached_handoff) = advance_along_local_access_path(current, &path, 0.05);

        assert!(
            reached_handoff,
            "opposite-side egress should complete at the exact lane endpoint instead of backtracking onto the crossover segment"
        );
        assert_eq!(next, path.points[3]);
    }
}
