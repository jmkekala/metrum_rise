// SPDX-License-Identifier: GPL-2.0-only

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
    advance_along_local_access_path, local_access_path, local_access_should_log_step,
    local_access_target_segment,
};
pub(super) use timing::{
    direct_frontage_segment_time_s, frontage_time_s, local_access_distance, local_access_time_s,
};

#[cfg(test)]
mod tests {
    use super::{LocalAccessPath, advance_along_local_access_path, local_access_should_log_step};
    use godot::prelude::Vector2;

    #[test]
    fn local_access_crosses_repeated_vertices_in_both_directions() {
        let points = [
            Vector2::ZERO,
            Vector2::new(2.0, 0.0),
            Vector2::new(2.0, 0.0),
            Vector2::new(2.0, 3.0),
        ];
        for (reverse, steps) in [
            (
                false,
                [
                    (0.0, points[0]),
                    (2.0, points[1]),
                    (2.5, Vector2::new(2.0, 0.5)),
                ],
            ),
            (
                true,
                [
                    (0.0, points[3]),
                    (3.0, points[2]),
                    (3.5, Vector2::new(1.5, 0.0)),
                ],
            ),
        ] {
            let mut path = LocalAccessPath { points, count: 4 };
            if reverse {
                path.points.reverse();
            }
            for (step, expected) in steps {
                assert_eq!(
                    advance_along_local_access_path(path.points[0], &path, step),
                    (expected, false),
                    "reverse={reverse}, step={step}"
                );
            }
            for step in [5.0, 6.0] {
                assert_eq!(
                    advance_along_local_access_path(path.points[0], &path, step),
                    (path.points[3], true)
                );
            }
        }
    }

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

    #[test]
    fn test_local_access_step_logging_is_progress_sampled() {
        let path = LocalAccessPath {
            points: [
                Vector2::new(0.0, 0.0),
                Vector2::new(4.0, 0.0),
                Vector2::ZERO,
                Vector2::ZERO,
            ],
            count: 2,
        };

        assert!(
            !local_access_should_log_step(
                Vector2::new(0.10, 0.0),
                Vector2::new(0.20, 0.0),
                &path,
                false
            ),
            "sub-meter movement within one progress bucket should stay quiet"
        );
        assert!(
            local_access_should_log_step(
                Vector2::new(0.95, 0.0),
                Vector2::new(1.05, 0.0),
                &path,
                false
            ),
            "crossing a whole-meter remaining-distance bucket should be logged"
        );
        assert!(
            local_access_should_log_step(
                Vector2::new(3.95, 0.0),
                Vector2::new(4.00, 0.0),
                &path,
                true
            ),
            "completion should always be logged"
        );
    }
}
