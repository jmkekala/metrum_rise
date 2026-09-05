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
fn road_tool_sticky_network_snap_holds_inside_release_radius() {
    let graph = test_snap_graph();

    assert!(SimulationNode::road_tool_should_keep_sticky_network_snap(
        &graph,
        Vector3::new(4.0, 0.0, 7.9),
        true,
        Vector3::new(4.0, 0.0, 0.0),
        8.0,
    ));
}

#[test]
fn road_tool_sticky_network_snap_releases_outside_release_radius() {
    let graph = test_snap_graph();

    assert!(!SimulationNode::road_tool_should_keep_sticky_network_snap(
        &graph,
        Vector3::new(4.0, 0.0, 8.1),
        true,
        Vector3::new(4.0, 0.0, 0.0),
        8.0,
    ));
}

#[test]
fn road_tool_sticky_network_snap_rejects_non_network_anchor() {
    let graph = test_snap_graph();

    assert!(!SimulationNode::road_tool_should_keep_sticky_network_snap(
        &graph,
        Vector3::new(40.0, 0.0, 40.0),
        true,
        Vector3::new(40.0, 0.0, 40.0),
        8.0,
    ));
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
