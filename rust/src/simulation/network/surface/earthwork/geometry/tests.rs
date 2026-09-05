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

    assert!(RoadSurfaceSystem::closed_loop_vertex_outward_xz(&points, 1).is_none());
}

#[test]
fn earthwork_edge_outward_accepts_short_nonzero_edges() {
    let outward = RoadSurfaceSystem::edge_outward_normal_xz(
        RoadVec2::new(f64::from(SAMPLE_EPSILON_M) * 10.0, 0.0),
        true,
    );

    assert_eq!(outward, Some(RoadVec2::new(0.0, -1.0)));
}
