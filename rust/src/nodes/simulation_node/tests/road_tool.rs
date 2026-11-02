// SPDX-License-Identifier: GPL-2.0-only

//! Road Tool regression tests for the simulation-node bridge.

use super::*;
use crate::simulation::network::interaction::StructureHit;

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
            StructureHit::Ground,
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
    let retained = SimulationNode::road_tool_cursor_network_snap(
        &graph,
        position,
        StructureHit::Ground,
        -1,
        0,
        1,
        1,
        8.0,
    )
    .unwrap();
    assert_eq!(retained.target, NetworkSnapTarget::Node(0));
    let fresh = SimulationNode::road_tool_cursor_network_snap(
        &graph,
        position,
        StructureHit::Ground,
        -1,
        0,
        1,
        2,
        8.0,
    )
    .unwrap();
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

// `ROAD-99` follow-up B1: the cursor snaps only to what its ray hit, at that level. A bridge 6 m
// over the flat ground from (-30, 6, 0) to (30, 6, 0), its deck the only road. (a) A ray down
// onto the deck snaps to the bridge at its graph height. (b) A ray down onto the ground 1-5 m
// beyond the deck's edge, and (c) a low ray under the deck onto the ground beneath its centre
// line, stay on the ground: no snap. Under the deck the snap without the ray's hit (as before)
// took the elevated span. Tunnels are covered by `interaction`'s test: a bore is never a hit.
#[test]
fn road_tool_cursor_snaps_only_to_the_structure_its_ray_hit() {
    use crate::simulation::network::interaction::{NetworkSnapTarget, get_closest_network_snap_xz};
    use crate::simulation::network::types::EdgeClass;
    let mut core = test_core_with_flat_terrain(0.0);
    core.treasury.balance = 1.0e7;
    let outcome = core.add_road_internal(
        vec![Vector3::new(-30.0, 6.0, 0.0), Vector3::new(30.0, 6.0, 0.0)],
        1,
        1,
    );
    assert!(outcome.committed, "{outcome:?}");
    core.precompute_road_mesh_data();
    let graph = &core.region_graph;
    let bridge = (0..graph.edge_count())
        .find(|&edge| !graph.edge(edge).deleted)
        .unwrap();
    assert_eq!(graph.edge(bridge).class, EdgeClass::Bridge);
    let roadbed = &core.transit_network.roadbed;
    // The flat ground's hit along a ray, as the graded surface reports it at height 0.
    let ground =
        |origin: Vector3, direction: Vector3| origin + direction * (-origin.y / direction.y);
    let cursor = |origin: Vector3, direction: Vector3| {
        let road = roadbed.raycast_piece(origin, direction);
        let terrain = ground(origin, direction);
        let hit = SimulationNode::road_tool_structure_hit(graph, origin, road, Some(terrain));
        let at = match road {
            Some((road_hit, _)) if hit != StructureHit::Ground => road_hit,
            _ => terrain,
        };
        let snap =
            SimulationNode::road_tool_cursor_network_snap(graph, at, hit, -1, -1, -1, 0, 8.0);
        (hit, at, snap)
    };

    let (hit, _, snap) = cursor(Vector3::new(0.0, 30.0, 1.0), Vector3::DOWN);
    assert_eq!(hit, StructureHit::Span(bridge), "(a) the ray hit the deck");
    let snap = snap.expect("(a) a snap on the deck");
    assert_eq!(snap.target, NetworkSnapTarget::Edge(bridge));
    assert_eq!(snap.position.y.to_bits(), 6.0_f32.to_bits());

    let deck_edge = (1..200)
        .map(|step| step as f32 * 0.05)
        .find(|&z| {
            roadbed
                .raycast(Vector3::new(0.0, 30.0, z), Vector3::DOWN)
                .is_none()
        })
        .expect("the deck's edge");
    for beyond in [1.0, 3.0, 5.0] {
        let origin = Vector3::new(0.0, 30.0, deck_edge + beyond);
        let (hit, _, snap) = cursor(origin, Vector3::DOWN);
        assert_eq!(hit, StructureHit::Ground, "(b) {beyond} m beside");
        assert_eq!(snap, None, "(b) {beyond} m beside: no snap to the deck");
    }

    let low = Vector3::new(-20.0, 2.0, 0.0);
    let (hit, at, snap) = cursor(low, Vector3::new(1.0, -0.1, 0.0));
    assert_eq!(hit, StructureHit::Ground, "(c) under the deck");
    assert!(at.x.abs() < 1.0e-3 && at.y.abs() < 1.0e-3, "(c) {at:?}");
    assert_eq!(snap, None, "(c) no snap to the deck over it");
    let before = get_closest_network_snap_xz(graph, at, 5.0).map(|snap| snap.target);
    assert_eq!(
        before,
        Some(NetworkSnapTarget::Edge(bridge)),
        "(c) before B1"
    );
}
