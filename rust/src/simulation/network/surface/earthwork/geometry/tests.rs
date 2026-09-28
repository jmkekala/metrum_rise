// SPDX-License-Identifier: GPL-2.0-only

//! Earthwork geometry helper tests.

use super::*;
use crate::simulation::network::surface::{RoadVec2, RoadVec3};

#[test]
fn earthwork_vertex_outward_rejects_degenerate_spur() {
    let points = vec![
        RoadVec3::new(-1.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(-1.0, 0.0, 0.0),
    ];

    assert!(RoadSurfaceSystem::closed_loop_vertex_outward_xz(&points, 1, false).is_none());
}

#[test]
fn earthwork_vertex_outward_matches_both_loop_windings() {
    let mut points = [
        RoadVec3::new(-1.0, 0.0, -1.0),
        RoadVec3::new(1.0, 0.0, -1.0),
        RoadVec3::new(1.0, 0.0, 1.0),
        RoadVec3::new(-1.0, 0.0, 1.0),
    ];
    for winding_ccw in [true, false] {
        assert_eq!(
            RoadSurfaceSystem::earthwork_signed_polygon_area_xz(&points) > 0.0,
            winding_ccw
        );
        for (index, point) in points.iter().enumerate() {
            assert_eq!(
                RoadSurfaceSystem::closed_loop_vertex_outward_xz(&points, index, winding_ccw),
                Some(RoadVec2::new(point.x, point.z).normalize()),
            );
        }
        points.reverse();
    }
}

#[test]
fn earthwork_edge_outward_accepts_short_nonzero_edges() {
    let outward = RoadSurfaceSystem::edge_outward_normal_xz(
        RoadVec2::new(f64::from(SAMPLE_EPSILON_M) * 10.0, 0.0),
        true,
    );

    assert_eq!(outward, Some(RoadVec2::new(0.0, -1.0)));
}
