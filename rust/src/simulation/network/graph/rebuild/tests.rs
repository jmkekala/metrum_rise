//! Junction profile rebuild regression tests.

use super::junction_profiles::{
    JUNCTION_PROFILE_MOUTH_MAX_GRADE, JUNCTION_PROFILE_SOLVE_SAMPLE_M,
    JUNCTION_PROFILE_SUPPORT_STEP_M, JunctionEndpointProfilePlane,
};
use crate::simulation::network::graph::data::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use godot::prelude::{Vector2, Vector3};
use std::collections::HashSet;

fn profile_test_edge(points: Vec<Vector3>) -> Edge {
    Edge {
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        geometry: points.clone(),
        physical_geometry: points,
        ..Default::default()
    }
}

#[test]
fn island_count_ignores_floating_nodes_and_deleted_edges() {
    let mut graph = RegionGraph::new();
    let positions = [
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(10.0, 0.0, 0.0),
        Vector3::new(20.0, 0.0, 0.0),
        Vector3::new(100.0, 0.0, 0.0),
        Vector3::new(110.0, 0.0, 0.0),
        Vector3::new(200.0, 0.0, 0.0),
    ];
    let nodes = positions
        .into_iter()
        .map(|position| graph.add_node(position, NodeType::Junction))
        .collect::<Vec<_>>();

    for (start_idx, end_idx) in [(0, 1), (1, 2), (3, 4)] {
        let mut edge = profile_test_edge(vec![positions[start_idx], positions[end_idx]]);
        edge.start_node = nodes[start_idx];
        edge.end_node = nodes[end_idx];
        graph.add_edge(edge);
    }
    let mut deleted = profile_test_edge(vec![positions[2], positions[3]]);
    deleted.start_node = nodes[2];
    deleted.end_node = nodes[3];
    let deleted_idx = graph.add_edge(deleted);
    graph.edge_mut(deleted_idx).deleted = true;

    assert_eq!(graph.get_island_count(), 2);
}

fn four_way_clip_graph() -> (RegionGraph, u32, usize) {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::ZERO;
    let center = graph.add_node(center_pos, NodeType::Junction);
    for endpoint_pos in [
        Vector3::new(-40.0, 0.0, 0.0),
        Vector3::new(40.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, -40.0),
        Vector3::new(0.0, 0.0, 40.0),
    ] {
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let mut edge = profile_test_edge(vec![center_pos, endpoint_pos]);
        edge.start_node = center;
        edge.end_node = endpoint;
        graph.add_edge(edge);
    }

    let remote_start_pos = Vector3::new(200.0, 0.0, 0.0);
    let remote_end_pos = Vector3::new(240.0, 0.0, 0.0);
    let remote_start = graph.add_node(remote_start_pos, NodeType::Junction);
    let remote_end = graph.add_node(remote_end_pos, NodeType::Junction);
    let mut remote_edge = profile_test_edge(vec![remote_start_pos, remote_end_pos]);
    remote_edge.start_node = remote_start;
    remote_edge.end_node = remote_end;
    remote_edge.start_clip = 3.0;
    remote_edge.end_clip = 4.0;
    let remote_edge_idx = graph.add_edge(remote_edge);

    (graph, center, remote_edge_idx)
}

#[test]
fn partial_clip_rebuild_matches_full_rebuild_and_preserves_unrelated_edges() {
    let (mut full, _, _) = four_way_clip_graph();
    full.rebuild_intersection_clips();
    let expected_center_clips = (0..4)
        .map(|edge_idx| full.edge(edge_idx).start_clip)
        .collect::<Vec<_>>();

    let (mut partial, center, remote_edge_idx) = four_way_clip_graph();
    partial.rebuild_intersection_clips_for_nodes(&HashSet::from([center]));

    for (edge_idx, expected_clip) in expected_center_clips.into_iter().enumerate() {
        assert_eq!(partial.edge(edge_idx).start_clip, expected_clip);
    }
    assert_eq!(partial.edge(remote_edge_idx).start_clip, 3.0);
    assert_eq!(partial.edge(remote_edge_idx).end_clip, 4.0);
}

#[test]
fn junction_profile_sampling_uses_requested_distance_from_edge_end() {
    let edge = profile_test_edge(vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(10.0, 0.0, 0.0),
        Vector3::new(20.0, 0.0, 0.0),
    ]);

    let sample = RegionGraph::sample_edge_geometry_from_endpoint(&edge, false, 5.0)
        .expect("edge-end profile sample should exist");

    assert!((sample.x - 15.0).abs() <= f32::EPSILON);
    assert!((sample.y - 0.0).abs() <= f32::EPSILON);
    assert!((sample.z - 0.0).abs() <= f32::EPSILON);
}

#[test]
fn bend_profile_plane_stays_horizontal_on_hillside_corner() {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 10.0, 0.0);
    let west_pos = Vector3::new(-48.0, 10.0, 0.0);
    let north_pos = Vector3::new(0.0, 22.0, 48.0);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let west = graph.add_node(west_pos, NodeType::Junction);
    let north = graph.add_node(north_pos, NodeType::Junction);

    let mut west_edge = profile_test_edge(vec![west_pos, center_pos]);
    west_edge.start_node = west;
    west_edge.end_node = center;
    graph.add_edge(west_edge);

    let mut north_edge = profile_test_edge(vec![center_pos, north_pos]);
    north_edge.start_node = center;
    north_edge.end_node = north;
    graph.add_edge(north_edge);

    let plane = graph
        .junction_endpoint_profile_plane(center)
        .expect("non-pass-through two-road node should expose a Bend profile plane");

    assert!(
        plane.grade() <= 1.0e-6,
        "Bend nodes must keep a horizontal local platform instead of inheriting the hillside grade: grade={:.6}",
        plane.grade()
    );
    assert!(
        (plane.height_at_xz(12.0, 12.0) - center_pos.y).abs() <= 1.0e-6,
        "horizontal Bend platform should be anchored at the graph node height"
    );
}

#[test]
fn bend_profile_adapts_new_corner_without_rewriting_stable_edge() {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 10.0, 0.0);
    let west_pos = Vector3::new(-48.0, 10.0, 0.0);
    let north_pos = Vector3::new(0.0, 22.0, 48.0);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let west = graph.add_node(west_pos, NodeType::Junction);
    let north = graph.add_node(north_pos, NodeType::Junction);

    let mut west_edge = profile_test_edge(vec![west_pos, center_pos]);
    west_edge.start_node = west;
    west_edge.end_node = center;
    graph.add_edge(west_edge);

    let mut north_edge = profile_test_edge(vec![center_pos, north_pos]);
    north_edge.start_node = center;
    north_edge.end_node = north;
    graph.add_edge(north_edge);

    let stable_before = graph.edge(0).geometry.clone();
    let changed_edges = graph
        .solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &HashSet::from([1]));

    assert_eq!(
        changed_edges,
        HashSet::from([1]),
        "only the newly authored bend leg should be adapted"
    );
    assert_eq!(
        graph.edge(0).geometry,
        stable_before,
        "stable existing bend leg should not be rewritten"
    );
    assert!(
        graph.edge(1).geometry.len() >= 8,
        "adapted bend leg should receive vertical-curve support points"
    );

    let plane = graph
        .junction_endpoint_profile_plane(center)
        .expect("adapted bend should keep exposing the horizontal platform plane");
    let solve_sample = RegionGraph::sample_edge_geometry_from_endpoint(
        graph.edge(1),
        true,
        JUNCTION_PROFILE_SOLVE_SAMPLE_M,
    )
    .expect("adapted bend leg should contain a solve-distance sample");
    let delta_m = (solve_sample.y - plane.height_at_xz(solve_sample.x, solve_sample.z)).abs();
    assert!(
        delta_m <= 0.05,
        "control geometry should bring the new bend leg to the horizontal platform at the solve sample: delta={delta_m:.6}"
    );
}

#[test]
fn regrade_junction_profile_caps_steep_platform_and_materializes_supports() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let east = graph.add_node(Vector3::new(48.0, 24.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 48.0), NodeType::Junction);

    let mut east_edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(48.0, 24.0, 0.0)]);
    east_edge.start_node = center;
    east_edge.end_node = east;
    graph.add_edge(east_edge);

    let mut north_edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 48.0)]);
    north_edge.start_node = center;
    north_edge.end_node = north;
    graph.add_edge(north_edge);

    graph.rebuild_adjacency_list();
    let adaptable_edges = HashSet::from([0, 1]);
    let changed_edges = graph
        .regrade_junction_endpoint_profiles_for_nodes(&HashSet::from([center]), &adaptable_edges);

    assert_eq!(changed_edges.len(), 2);
    let plane = graph
        .junction_endpoint_profile_plane(center)
        .expect("regraded support vertices should define a readable platform plane");
    let grade = plane.grade_x.hypot(plane.grade_z);
    assert!(
        grade <= JUNCTION_PROFILE_MOUTH_MAX_GRADE + 1.0e-4,
        "platform grade should be capped after regrade, got {grade:.3}"
    );

    let east_geometry = &graph.edge(0).geometry;
    assert!(
        east_geometry.len() >= 8,
        "regrade should insert vertical-curve support vertices outside the protected mouth"
    );
    let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(graph.edge(0).physical_length * 0.5);
    let solve_sample =
        RegionGraph::sample_edge_geometry_from_endpoint(graph.edge(0), true, solve_sample_m)
            .expect("solve-distance support sample should exist");
    let expected_y = plane.height_at_xz(solve_sample.x, solve_sample.z);
    let solve_sample_delta_m = (solve_sample.y - expected_y).abs();
    assert!(
        solve_sample_delta_m <= 0.05,
        "control geometry should keep the capped solve sample available for later profile solves: delta={solve_sample_delta_m:.6}"
    );

    let mut visible_edge = graph.edge(0).clone();
    visible_edge.geometry = visible_edge.physical_geometry.clone();
    let visible_solve_sample =
        RegionGraph::sample_edge_geometry_from_endpoint(&visible_edge, true, solve_sample_m)
            .expect("visible solve-distance support sample should exist");
    let visible_expected_y = plane.height_at_xz(visible_solve_sample.x, visible_solve_sample.z);
    let visible_solve_sample_delta_m = (visible_solve_sample.y - visible_expected_y).abs();
    let original_source_y = solve_sample_m * 0.5;
    let original_delta_m = (original_source_y - visible_expected_y).abs();
    assert!(
        visible_solve_sample_delta_m > 0.05 && visible_solve_sample_delta_m < original_delta_m,
        "visible solve-distance support must be eased toward, not pinned to, the capped profile plane: delta={visible_solve_sample_delta_m:.6} original_delta={original_delta_m:.6} sample={visible_solve_sample:?} expected_y={visible_expected_y:.6}"
    );

    let transition_sample = RegionGraph::sample_edge_geometry_from_endpoint(
        &visible_edge,
        true,
        solve_sample_m + JUNCTION_PROFILE_SUPPORT_STEP_M * 2.0,
    )
    .expect("vertical-curve transition sample should exist");
    let transition_plane_y = plane.height_at_xz(transition_sample.x, transition_sample.z);
    assert!(
        (transition_sample.y - transition_plane_y).abs() > 0.05,
        "post-hard-zone sample should have started easing away from the platform plane"
    );
    let local_grade = (transition_sample.y - visible_solve_sample.y).abs()
        / (JUNCTION_PROFILE_SUPPORT_STEP_M * 2.0);
    assert!(
        local_grade < 0.75,
        "profile support transition should not create a near-vertical cliff: local_grade={local_grade:.3}"
    );
}

#[test]
fn conservative_profile_materializes_supports_for_visible_vertical_curve() {
    let mut edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(80.0, 20.0, 0.0)]);
    let plane = JunctionEndpointProfilePlane {
        origin: Vector3::ZERO,
        grade_x: 0.0,
        grade_z: 0.0,
    };

    RegionGraph::apply_junction_profile_plane_to_edge(&mut edge, true, plane, false);

    assert!(
        edge.geometry.len() >= 8,
        "conservative profile adaptation should materialize support vertices when the visible mouth needs a vertical curve"
    );
    let control_solve_sample = RegionGraph::sample_edge_geometry_from_endpoint(
        &edge,
        true,
        JUNCTION_PROFILE_SOLVE_SAMPLE_M,
    )
    .expect("control solve sample should exist");
    assert!(
        control_solve_sample.y.abs() <= 0.05,
        "control geometry keeps the solve sample on the profile plane"
    );

    let mut visible_edge = edge.clone();
    visible_edge.geometry = visible_edge.physical_geometry.clone();
    let visible_solve_sample = RegionGraph::sample_edge_geometry_from_endpoint(
        &visible_edge,
        true,
        JUNCTION_PROFILE_SOLVE_SAMPLE_M,
    )
    .expect("visible solve sample should exist");
    assert!(
        visible_solve_sample.y > 0.05 && visible_solve_sample.y < 3.0,
        "physical geometry should be eased toward the plane, not flattened or left raw: y={:.3}",
        visible_solve_sample.y
    );
}

#[test]
fn junction_profile_preserves_authority_corridor_when_branch_connects_from_hill() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let west = graph.add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(48.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 24.0, 48.0), NodeType::Junction);

    let mut west_edge = profile_test_edge(vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::ZERO]);
    west_edge.start_node = west;
    west_edge.end_node = center;
    graph.add_edge(west_edge);

    let mut east_edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(48.0, 0.0, 0.0)]);
    east_edge.start_node = center;
    east_edge.end_node = east;
    graph.add_edge(east_edge);

    let mut north_edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(0.0, 24.0, 48.0)]);
    north_edge.start_node = center;
    north_edge.end_node = north;
    graph.add_edge(north_edge);

    graph.rebuild_adjacency_list();
    let changed_edges = graph.solve_junction_endpoint_profiles_for_edges(
        &HashSet::from([center]),
        &HashSet::from([1, 2]),
    );

    assert_eq!(
        changed_edges,
        HashSet::from([2]),
        "split through-road mouth should stay authoritative; only the branch adapts"
    );
    assert_eq!(
        graph.edge(1).geometry,
        vec![Vector3::ZERO, Vector3::new(48.0, 0.0, 0.0)],
        "opposite split half of the existing corridor should not be regraded"
    );
    assert!(
        graph.edge(2).geometry.len() >= 8,
        "branch adaptation should materialize a smooth support transition"
    );

    let plane = graph
        .junction_endpoint_profile_plane(center)
        .expect("authority corridor should define the JunctionN profile plane");
    assert!(
        plane.grade() <= 1.0e-4,
        "flat through corridor should keep the junction plane flat, got grade {:.6}",
        plane.grade()
    );
    let solve_sample = RegionGraph::sample_edge_geometry_from_endpoint(
        graph.edge(2),
        true,
        JUNCTION_PROFILE_SOLVE_SAMPLE_M,
    )
    .expect("branch solve-distance support should exist");
    assert!(
        (solve_sample.y - plane.height_at_xz(solve_sample.x, solve_sample.z)).abs() <= 0.05,
        "branch solve sample should adapt to the authority corridor plane"
    );
}

#[test]
fn pass_through_bridge_approach_profile_does_not_exceed_authored_grade_cap() {
    let mut graph = RegionGraph::new();
    let west_pos = Vector3::new(-48.0, 0.096, 0.0);
    let center_pos = Vector3::ZERO;
    let east_pos = Vector3::new(96.0, 0.0, 0.0);
    let west = graph.add_node(west_pos, NodeType::Junction);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let east = graph.add_node(east_pos, NodeType::Junction);

    let mut bridge = profile_test_edge(vec![west_pos, center_pos]);
    bridge.class = EdgeClass::Bridge;
    bridge.start_node = west;
    bridge.end_node = center;
    graph.add_edge(bridge);

    let mut approach_points = vec![
        center_pos,
        Vector3::new(6.0, -0.96, 0.0),
        Vector3::new(12.0, -1.92, 0.0),
        Vector3::new(18.0, -2.40, 0.0),
    ];
    for x in (24..=96).step_by(6) {
        let t = (x - 18) as f32 / (96 - 18) as f32;
        approach_points.push(Vector3::new(x as f32, -2.40 * (1.0 - t), 0.0));
    }
    let mut approach = profile_test_edge(approach_points);
    approach.start_node = center;
    approach.end_node = east;
    let approach_idx = graph.add_edge(approach);
    graph.rebuild_adjacency_list();
    let authored_profile = graph.edge(approach_idx).physical_geometry.clone();

    graph.solve_junction_endpoint_profiles_for_edges(
        &HashSet::from([center, east]),
        &HashSet::from([approach_idx]),
    );
    graph.regrade_junction_endpoint_profiles_for_nodes(
        &HashSet::from([center, east]),
        &HashSet::from([approach_idx]),
    );

    assert_eq!(
        graph.edge(approach_idx).physical_geometry,
        authored_profile,
        "PassThrough nodes have no node platform and must not rewrite an already validated approach profile"
    );
    let max_grade = graph
        .edge(approach_idx)
        .physical_geometry
        .windows(2)
        .map(|pair| {
            let run_m = (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z);
            (pair[1].y - pair[0].y).abs() / run_m
        })
        .fold(0.0_f32, f32::max);
    assert!(
        max_grade <= JUNCTION_PROFILE_MOUTH_MAX_GRADE + 1.0e-4,
        "post-commit pass-through approach must preserve the authored grade cap; max_grade={max_grade:.3}"
    );
}

#[test]
fn junction_profile_preserves_primary_corridor_when_opposite_branch_is_added() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let through_axis = Vector2::new(0.866_025_4, 0.5);
    let branch_axis = Vector2::new(0.5, -0.866_025_4);
    let through_grade = 0.07;
    let branch_grade = 0.24;
    let length_m = 48.0;

    let west_pos = Vector3::new(
        -through_axis.x * length_m,
        -through_grade * length_m,
        -through_axis.y * length_m,
    );
    let east_pos = Vector3::new(
        through_axis.x * length_m,
        through_grade * length_m,
        through_axis.y * length_m,
    );
    let first_branch_pos = Vector3::new(
        branch_axis.x * length_m,
        branch_grade * length_m,
        branch_axis.y * length_m,
    );
    let opposite_branch_pos = Vector3::new(
        -branch_axis.x * length_m,
        -branch_grade * length_m,
        -branch_axis.y * length_m,
    );

    let west = graph.add_node(west_pos, NodeType::Junction);
    let east = graph.add_node(east_pos, NodeType::Junction);
    let first_branch = graph.add_node(first_branch_pos, NodeType::Junction);
    let opposite_branch = graph.add_node(opposite_branch_pos, NodeType::Junction);

    for (start, end, points) in [
        (west, center, vec![west_pos, Vector3::ZERO]),
        (center, east, vec![Vector3::ZERO, east_pos]),
        (center, first_branch, vec![Vector3::ZERO, first_branch_pos]),
    ] {
        let mut edge = profile_test_edge(points);
        edge.start_node = start;
        edge.end_node = end;
        graph.add_edge(edge);
    }
    graph.rebuild_adjacency_list();

    let before = graph
        .junction_endpoint_profile_plane(center)
        .expect("primary through corridor should define the initial JunctionN plane");
    let stable_before = [0, 1, 2].map(|edge_idx| graph.edge(edge_idx).geometry.clone());
    let mut opposite_edge = profile_test_edge(vec![Vector3::ZERO, opposite_branch_pos]);
    opposite_edge.start_node = center;
    opposite_edge.end_node = opposite_branch;
    graph.add_edge(opposite_edge);
    graph.rebuild_adjacency_list();
    let changed_edges = graph
        .solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &HashSet::from([3]));
    assert_eq!(
        changed_edges,
        HashSet::from([3]),
        "the new opposite branch should adapt to the existing primary corridor"
    );
    let regrade_changed_edges = graph.regrade_junction_endpoint_profiles_for_nodes(
        &HashSet::from([center]),
        &HashSet::from([3]),
    );
    assert!(
        regrade_changed_edges.is_subset(&HashSet::from([3])),
        "the stronger regrade pass must not make stable incident roads mutable again: changed={regrade_changed_edges:?}"
    );
    for (edge_idx, before_geometry) in stable_before.into_iter().enumerate() {
        assert_eq!(
            graph.edge(edge_idx).geometry,
            before_geometry,
            "adding and regrading a branch must not rewrite stable edge {edge_idx}"
        );
    }
    let after = graph
        .junction_endpoint_profile_plane(center)
        .expect("adding an opposite branch should keep a canonical JunctionN plane");
    assert!(
        (after.grade_x - before.grade_x).abs() <= 1.0e-4
            && (after.grade_z - before.grade_z).abs() <= 1.0e-4,
        "secondary opposite branch corridor must not rotate the primary JunctionN plane: before=({:.6},{:.6}) after=({:.6},{:.6})",
        before.grade_x,
        before.grade_z,
        after.grade_x,
        after.grade_z
    );
    assert!(
        (after.grade() - through_grade).abs() <= 1.0e-4,
        "the primary through-road grade should stay authoritative, got {:.6}",
        after.grade()
    );
}
