// SPDX-License-Identifier: GPL-2.0-only

//! Road Tool regression tests for the simulation-node bridge.

use super::*;

#[test]
fn road_tool_cursor_classifies_closer_road_hit_as_absolute_height() {
    let ray_origin = Vector3::new(0.0, 10.0, 0.0);
    let road_hit = Vector3::new(0.0, 4.0, 0.0);
    let terrain_hit = Vector3::new(0.0, 0.0, 0.0);

    let (hit, road_owned) =
        SimulationNode::road_tool_closest_cursor_hit(ray_origin, Some(road_hit), Some(terrain_hit))
            .expect("cursor should select one visible hit");

    assert_eq!(hit, road_hit);
    assert!(
        road_owned,
        "road-owned cursor hits must be treated as absolute heights, not terrain plus heightpoint"
    );
}

#[test]
fn road_tool_cursor_classifies_closer_terrain_hit_as_offsettable_height() {
    let ray_origin = Vector3::new(0.0, 10.0, 0.0);
    let road_hit = Vector3::new(0.0, -2.0, 0.0);
    let terrain_hit = Vector3::new(0.0, 0.0, 0.0);

    let (hit, road_owned) =
        SimulationNode::road_tool_closest_cursor_hit(ray_origin, Some(road_hit), Some(terrain_hit))
            .expect("cursor should select one visible hit");

    assert_eq!(hit, terrain_hit);
    assert!(
        !road_owned,
        "terrain cursor hits must remain eligible for the road-tool heightpoint"
    );
}

#[test]
fn road_tool_sticky_network_snap_slides_inside_release_radius() {
    let graph = test_snap_graph();
    for step in 0..40 {
        let x = 6.0 + step as f32 * 0.2;
        let snap = SimulationNode::road_tool_cursor_network_snap(
            &graph,
            Vector3::new(x, 40.0, 7.9),
            0,
            -1,
            1,
            1,
            8.0,
        )
        .unwrap();
        assert!((snap.position.x - x).abs() < 0.0001);
        assert_eq!(snap.position.z, 0.0);
    }
}

#[test]
fn road_tool_sticky_network_snap_releases_outside_release_radius() {
    let graph = test_snap_graph();

    use crate::simulation::network::interaction::{NetworkSnapTarget, retained_network_snap_xz};
    assert!(
        retained_network_snap_xz(
            &graph,
            Vector3::new(10.0, 0.0, 8.1),
            NetworkSnapTarget::Edge(0),
            5.0,
            8.0
        )
        .is_none()
    );
}

#[test]
fn road_tool_sticky_network_snap_rejects_stale_generation() {
    let graph = test_snap_graph();
    use crate::simulation::network::interaction::NetworkSnapTarget;
    let position = Vector3::new(7.0, 0.0, 0.0);
    let retained =
        SimulationNode::road_tool_cursor_network_snap(&graph, position, -1, 0, 1, 1, 8.0).unwrap();
    assert_eq!(retained.target, NetworkSnapTarget::Node(0));
    let fresh =
        SimulationNode::road_tool_cursor_network_snap(&graph, position, -1, 0, 1, 2, 8.0).unwrap();
    assert_eq!(fresh.target, NetworkSnapTarget::Edge(0));
    assert_eq!(fresh.position, position);
}

#[test]
fn road_tool_border_precheck_matches_strict_heightmap_bounds() {
    let half_width_m = 50.0;
    let half_height_m = 40.0;
    let threshold_m = 3.0;

    assert!(!SimulationNode::road_tool_is_near_border(
        Vector3::ZERO,
        half_width_m,
        half_height_m,
        threshold_m,
    ));
    assert!(!SimulationNode::road_tool_is_near_border(
        Vector3::new(47.0, 0.0, 37.0),
        half_width_m,
        half_height_m,
        threshold_m,
    ));
    assert!(SimulationNode::road_tool_is_near_border(
        Vector3::new(47.001, 0.0, 0.0),
        half_width_m,
        half_height_m,
        threshold_m,
    ));
    assert!(SimulationNode::road_tool_is_near_border(
        Vector3::new(0.0, 0.0, -37.001),
        half_width_m,
        half_height_m,
        threshold_m,
    ));
}
