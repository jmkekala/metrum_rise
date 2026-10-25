// SPDX-License-Identifier: GPL-2.0-only

//! Junction profile rebuild regression tests.

use super::junction_profiles::{
    EndpointPin, JUNCTION_PROFILE_MOUTH_MAX_GRADE, JUNCTION_PROFILE_SOLVE_SAMPLE_M,
    JunctionEndpointProfilePlane,
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

// `ROAD-68` invariant 6: corridors rank by each arm's direction towards its first distinct
// control point, as the node layout takes it. A mirrored Y on a slope ties its two corridors; with
// every arm starting on a repeated node point, the raw first segment put each arm at angle zero
// and edge ids, which follow creation order, chose the corridor that keeps its grade.
#[test]
fn corridor_ranking_does_not_depend_on_edge_creation_order() {
    let grade = 0.05;
    let ends =
        [(-96.0, 0.0), (90.0, 32.0), (90.0, -32.0)].map(|(x, z)| Vector3::new(x, grade * x, z));
    let physical = |order: [usize; 3]| {
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let mut ids = [0; 3];
        for index in order {
            let end_node = graph.add_node(ends[index], NodeType::Junction);
            let mut edge = profile_test_edge(vec![Vector3::ZERO, Vector3::ZERO, ends[index]]);
            edge.start_node = center;
            edge.end_node = end_node;
            ids[index] = graph.add_edge(edge);
        }
        graph.rebuild_intersection_clips();
        assert!(
            ids.iter().all(|&id| graph.edge(id).start_clip > 0.0),
            "the Y lays out"
        );
        graph.finalize_junction_endpoint_profiles(&HashSet::from([center]), None);
        ids.map(|id| graph.edge(id).physical_geometry.clone())
    };
    assert_eq!(physical([0, 1, 2]), physical([2, 1, 0]));
}

#[test]
fn final_junction_profile_preserves_a_supported_hillside_corridor() {
    for grade in [0.32, 0.6] {
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
        for endpoint in [
            Vector3::new(-48.0, -48.0 * grade, 0.0),
            Vector3::new(48.0, 48.0 * grade, 0.0),
            Vector3::new(0.0, 0.0, 48.0),
            Vector3::new(0.0, 0.0, -48.0),
        ] {
            let end_node = graph.add_node(endpoint, NodeType::Junction);
            let mut edge = profile_test_edge(vec![Vector3::ZERO, endpoint]);
            edge.start_node = center;
            edge.end_node = end_node;
            graph.add_edge(edge);
        }
        graph.rebuild_intersection_clips();
        graph.finalize_junction_endpoint_profiles(&HashSet::from([center]), None);
        let plane = graph
            .solved_junction_endpoint_profile_plane(center)
            .unwrap();
        assert!((plane.grade_x - grade).abs() < 1.0e-5);
        for edge in graph.edges() {
            for point in &edge.physical_geometry {
                assert!(
                    (point.y - grade * point.x).abs() < 0.001,
                    "supported hillside was flattened into a cut platform: {point:?}"
                );
            }
        }
    }
}

#[test]
fn topology_only_dirtiness_does_not_reblend_solved_junction_profiles() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    for endpoint in [
        Vector3::new(-48.0, -15.36, 0.0),
        Vector3::new(48.0, 15.36, 0.0),
        Vector3::new(0.0, 12.0, 48.0),
    ] {
        let end_node = graph.add_node(endpoint, NodeType::Junction);
        let mut edge = profile_test_edge(vec![Vector3::ZERO, endpoint]);
        edge.start_node = center;
        edge.end_node = end_node;
        graph.add_edge(edge);
    }
    let nodes = HashSet::from([center]);
    graph.rebuild_intersection_clips();
    graph.finalize_junction_endpoint_profiles(&nodes, None);
    let solved: Vec<_> = graph
        .edges()
        .iter()
        .map(|edge| (edge.geometry.clone(), edge.physical_geometry.clone()))
        .collect();
    assert!(
        solved[2].1.len() > 2,
        "fixture must have a solved approach transition"
    );
    let branch_end = *graph.edge(2).geometry.last().unwrap();
    graph.edge_mut(2).geometry = vec![Vector3::ZERO, branch_end];
    let solved: Vec<_> = graph
        .edges()
        .iter()
        .map(|edge| (edge.geometry.clone(), edge.physical_geometry.clone()))
        .collect();
    for _ in 0..3 {
        graph.finalize_junction_endpoint_profiles(&nodes, None);
        for (edge, (control, physical)) in graph.edges().iter().zip(&solved) {
            assert_eq!(&edge.geometry, control);
            assert_eq!(edge.physical_geometry.len(), physical.len());
            for (actual, expected) in edge.physical_geometry.iter().zip(physical) {
                assert!(
                    (actual.y - expected.y).abs() < 0.0001,
                    "{actual:?} vs {expected:?}"
                );
            }
        }
    }
}

#[test]
fn repeated_junction_solve_does_not_import_hard_control_pins_into_physical_profile() {
    let mut edge = profile_test_edge(vec![
        Vector3::ZERO,
        Vector3::new(12.02, 6.01, 0.0),
        Vector3::new(48.0, 24.0, 0.0),
    ]);
    let pin = EndpointPin {
        plane: JunctionEndpointProfilePlane {
            origin: Vector3::ZERO,
            grade_x: 0.1,
            grade_z: 0.0,
        },
        // A hard zone, as a junction layout mouth station gives it.
        hard_zone_m: 1.75,
        force_supports: true,
        half_edge: false,
    };
    let control = edge.geometry.clone();
    for _ in 0..2 {
        let derived = RegionGraph::pinned_physical_profile(&edge, [Some(pin), None], None);
        assert_eq!(edge.geometry, control, "control is never rewritten");
        if edge.physical_geometry.len() > 3 {
            assert_eq!(derived, edge.physical_geometry, "derivation is idempotent");
        }
        edge.physical_geometry = derived;
        let mut checked_support_seam = false;
        for span in edge.physical_geometry.windows(2) {
            let run = (span[1].x - span[0].x).hypot(span[1].z - span[0].z);
            // The regression is the 2 cm gap between the materialized 12 m plane support
            // and the authored 12.02 m sample, not a design-grade limit for the whole ramp.
            if run >= 0.1 {
                continue;
            }
            checked_support_seam = true;
            let grade = (span[1].y - span[0].y).abs() / run;
            assert!(
                grade < 1.0,
                "control support leaked into the road: grade={grade}, span={span:?}"
            );
        }
        assert!(checked_support_seam);
    }
}

// A terrain whose source height at world `x` is `height(x)`, the same along z.
fn profile_test_terrain(height: impl Fn(f32) -> f32) -> crate::simulation::terrain::TerrainSystem {
    let mut terrain = crate::simulation::terrain::TerrainSystem::with_chunking(256, 256, 1.0, 8, 0.0);
    for grid_z in 0..256 {
        for grid_x in 0..256 {
            let (x, _) = terrain.grid_to_world_coords(grid_x, grid_z);
            terrain.set_height(grid_x, grid_z, height(x) / crate::config::HEIGHT_SCALE);
        }
    }
    terrain
}

// `ROAD-68` open question 39: a junction blend keeps within the 5 m source band of the ground,
// widened only near a held point off it, as the stroke solve's samples do; where the band and the
// plane leave no blend within 1:3, the blend is built inside the band at the least grade that
// allows it, steeper than the maximum, which placement refuses (`road_too_steep`).
#[test]
fn junction_blends_keep_the_source_band_or_read_steeper_than_the_maximum() {
    use crate::simulation::network::grade::{ROAD_GRADE_MAX, source_ground_m, steepest_grade};
    let pin = |height_m: f32| EndpointPin {
        plane: JunctionEndpointProfilePlane {
            origin: Vector3::new(0.0, height_m, 0.0),
            grade_x: 0.0,
            grade_z: 0.0,
        },
        hard_zone_m: 3.0,
        force_supports: true,
        half_edge: false,
    };
    let edge_on = |terrain: &crate::simulation::terrain::TerrainSystem| {
        profile_test_edge(
            (0..=10)
                .map(|step| {
                    let x = step as f32 * 6.0;
                    Vector3::new(x, source_ground_m(terrain, x, 0.0), 0.0)
                })
                .collect(),
        )
    };
    let offset = |terrain: &crate::simulation::terrain::TerrainSystem, point: &Vector3| {
        (point.y - source_ground_m(terrain, point.x, point.z)).abs()
    };

    // A plane 6 m above flat ground: the blend is back inside 5 m once the plane's 1 m excess has
    // run out (6.25 m past the mouth at the 16% the band shrinks at). Without the band the blend
    // follows the plane past 5 m further out.
    let flat = profile_test_terrain(|_| 0.0);
    let edge = edge_on(&flat);
    let unbanded = RegionGraph::pinned_physical_profile(&edge, [Some(pin(6.0)), None], None);
    assert!(unbanded.iter().any(|point| point.x > 9.3 && offset(&flat, point) > 5.0));
    let banded = RegionGraph::pinned_physical_profile(&edge, [Some(pin(6.0)), None], Some(&flat));
    assert!(steepest_grade(&banded).unwrap().grade <= ROAD_GRADE_MAX);
    for point in &banded {
        assert!(point.x <= 9.3 || offset(&flat, point) <= 5.0, "{point:?}");
    }
    assert_eq!(banded.len(), unbanded.len(), "the band moves heights, never knots");

    // Ground falling 10 m at 1:1 past the mouth: the arm's far control (on the ground, held) is
    // 10 m below the plane over the 32 m blend, within 1:3 without the band, but a blend within
    // 5 m of the falling ground has to drop faster.
    let cliff = profile_test_terrain(|x| -(x - 4.0).clamp(0.0, 10.0));
    let edge = edge_on(&cliff);
    let unbanded = RegionGraph::pinned_physical_profile(&edge, [Some(pin(0.0)), None], None);
    assert!(steepest_grade(&unbanded).unwrap().grade <= ROAD_GRADE_MAX);
    let banded = RegionGraph::pinned_physical_profile(&edge, [Some(pin(0.0)), None], Some(&cliff));
    assert!(steepest_grade(&banded).unwrap().grade > ROAD_GRADE_MAX);
    for point in &banded {
        assert!(offset(&cliff, point) <= 5.0, "{point:?}");
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
fn bend_profile_plane_follows_hillside_corner_grades() {
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
        (plane.grade() - 0.25).abs() <= 1.0e-6,
        "a bend should fit its incident hillside grades: grade={:.6}",
        plane.grade()
    );
    assert!(
        (plane.height_at_xz(0.0, 0.0) - center_pos.y).abs() <= 1.0e-6,
        "the fitted bend plane must remain anchored at the graph node height"
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

    let controls_before = [0, 1].map(|edge| graph.edge(edge).geometry.clone());
    graph.rebuild_intersection_clips();
    let changed_edges = graph.solve_junction_endpoint_profiles_for_nodes(&HashSet::from([center]));

    assert_eq!(
        changed_edges,
        HashSet::from([0, 1]),
        "both bend legs take the mouth knots of their physical profiles"
    );
    for (edge, control) in controls_before.iter().enumerate() {
        assert_eq!(
            &graph.edge(edge).geometry,
            control,
            "the profile solve never rewrites control geometry"
        );
    }
    assert!(
        graph
            .edge(1)
            .physical_geometry
            .iter()
            .all(|point| { (point.y - (10.0 + 0.25 * point.z)).abs() <= 0.0001 }),
        "an already-compatible hillside grade must not receive an artificial flat platform"
    );

    let plane = graph
        .junction_endpoint_profile_plane(center)
        .expect("adapted bend should keep exposing its fitted plane");
    let mut visible_edge = graph.edge(1).clone();
    visible_edge.geometry = visible_edge.physical_geometry.clone();
    let solve_sample = RegionGraph::sample_edge_geometry_from_endpoint(
        &visible_edge,
        true,
        JUNCTION_PROFILE_SOLVE_SAMPLE_M,
    )
    .expect("adapted bend leg should contain a solve-distance sample");
    let delta_m = (solve_sample.y - plane.height_at_xz(solve_sample.x, solve_sample.z)).abs();
    assert!(
        delta_m <= 0.05,
        "the physical profile should bring the new bend leg to the platform at the solve sample: delta={delta_m:.6}"
    );
}

#[test]
fn final_junction_profile_limits_small_source_adjustments_and_materializes_supports() {
    let mut graph = RegionGraph::new();
    // The 0.20 -> 0.16 grade adjustment stays within the source-fit budget;
    // steeper supported hillsides must keep their grade instead of being flattened.
    let source_grade = 0.20;
    let east_pos = Vector3::new(48.0, 48.0 * source_grade, 0.0);
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let east = graph.add_node(east_pos, NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 48.0), NodeType::Junction);

    let mut east_edge = profile_test_edge(vec![Vector3::ZERO, east_pos]);
    east_edge.start_node = center;
    east_edge.end_node = east;
    graph.add_edge(east_edge);

    let mut north_edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 48.0)]);
    north_edge.start_node = center;
    north_edge.end_node = north;
    graph.add_edge(north_edge);

    // Clips come from the junction layouts and set the solve's hard zones.
    graph.rebuild_intersection_clips();
    let controls_before = [0, 1].map(|edge| graph.edge(edge).geometry.clone());
    let changed_edges = graph.finalize_junction_endpoint_profiles(&HashSet::from([center]), None);

    assert_eq!(changed_edges.len(), 2);
    let plane = graph
        .solved_junction_endpoint_profile_plane(center)
        .expect("final support vertices should define a readable crossing plane");
    let grade = plane.grade_x.hypot(plane.grade_z);
    assert!(
        grade <= JUNCTION_PROFILE_MOUTH_MAX_GRADE + 1.0e-4,
        "a source-compatible grade adjustment should respect the cap, got {grade:.3}"
    );

    assert_eq!(
        [0, 1].map(|edge| graph.edge(edge).geometry.clone()),
        controls_before,
        "the profile solve never rewrites control geometry"
    );
    assert!(
        graph.edge(0).physical_geometry.len() >= 8,
        "finalization should insert vertical-curve support vertices outside the crossing core"
    );
    let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(graph.edge(0).physical_length * 0.5);

    let mut visible_edge = graph.edge(0).clone();
    visible_edge.geometry = visible_edge.physical_geometry.clone();
    let visible_solve_sample =
        RegionGraph::sample_edge_geometry_from_endpoint(&visible_edge, true, solve_sample_m)
            .expect("visible solve-distance support sample should exist");
    let visible_expected_y = plane.height_at_xz(visible_solve_sample.x, visible_solve_sample.z);
    let visible_solve_sample_delta_m = (visible_solve_sample.y - visible_expected_y).abs();
    let original_source_y = solve_sample_m * source_grade;
    let original_delta_m = (original_source_y - visible_expected_y).abs();
    assert!(
        visible_solve_sample_delta_m > 1.0e-4 && visible_solve_sample_delta_m < original_delta_m,
        "visible solve-distance support must be eased toward, not pinned to, the capped profile plane: delta={visible_solve_sample_delta_m:.6} original_delta={original_delta_m:.6} sample={visible_solve_sample:?} expected_y={visible_expected_y:.6}"
    );

    // The hard zone is the bend's layout mouth (9.5 m); sample a solve distance beyond it.
    let transition_m = graph.edge(0).start_clip + JUNCTION_PROFILE_SOLVE_SAMPLE_M;
    let transition_sample =
        RegionGraph::sample_edge_geometry_from_endpoint(&visible_edge, true, transition_m)
            .expect("vertical-curve transition sample should exist");
    let transition_plane_y = plane.height_at_xz(transition_sample.x, transition_sample.z);
    assert!(
        (transition_sample.y - transition_plane_y).abs() > 0.05,
        "post-hard-zone sample should have started easing away from the platform plane"
    );
    let local_grade =
        (transition_sample.y - visible_solve_sample.y).abs() / (transition_m - solve_sample_m);
    assert!(
        local_grade < source_grade * 1.5,
        "profile support transition should not create a near-vertical cliff: local_grade={local_grade:.3}"
    );
}

#[test]
fn conservative_profile_materializes_supports_for_visible_vertical_curve() {
    let edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(80.0, 20.0, 0.0)]);
    let pin = EndpointPin {
        plane: JunctionEndpointProfilePlane {
            origin: Vector3::ZERO,
            grade_x: 0.0,
            grade_z: 0.0,
        },
        // A hard zone, as a junction layout mouth station gives it.
        hard_zone_m: 1.75,
        force_supports: false,
        half_edge: false,
    };

    let mut visible_edge = edge.clone();
    visible_edge.geometry = RegionGraph::pinned_physical_profile(&edge, [Some(pin), None], None);
    assert!(
        visible_edge.geometry.len() >= 8,
        "profile adaptation should materialize support vertices when the visible mouth needs a vertical curve"
    );
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

    // Clips come from the junction layouts and set the solve's hard zones.
    graph.rebuild_intersection_clips();
    graph.solve_junction_endpoint_profiles_for_nodes(&HashSet::from([center]));

    for edge in [0, 1] {
        assert!(
            graph
                .edge(edge)
                .physical_geometry
                .iter()
                .all(|point| point.y == 0.0),
            "the flat through corridor keeps its grade; only the branch adapts"
        );
    }
    assert_eq!(
        graph.edge(1).geometry,
        vec![Vector3::ZERO, Vector3::new(48.0, 0.0, 0.0)],
        "the profile solve never rewrites control geometry"
    );
    assert!(
        graph.edge(2).physical_geometry.len() >= 8,
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
    let branch = graph.edge(2);
    for point in &branch.physical_geometry {
        if point.z <= branch.start_clip {
            assert_eq!(
                point.y,
                plane.height_at_xz(point.x, point.z),
                "the branch lies on the authority corridor plane up to its mouth"
            );
        }
    }
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
    // Clips come from the junction layouts and set the solve's hard zones.
    graph.rebuild_intersection_clips();
    let authored_profile = graph.edge(approach_idx).physical_geometry.clone();

    graph.finalize_junction_endpoint_profiles(&HashSet::from([center, east]), None);

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
    // Clips come from the junction layouts and set the solve's hard zones.
    graph.rebuild_intersection_clips();

    let before = graph
        .junction_endpoint_profile_plane(center)
        .expect("primary through corridor should define the initial JunctionN plane");
    let stable_before = [0, 1, 2].map(|edge_idx| graph.edge(edge_idx).geometry.clone());
    let mut opposite_edge = profile_test_edge(vec![Vector3::ZERO, opposite_branch_pos]);
    opposite_edge.start_node = center;
    opposite_edge.end_node = opposite_branch;
    graph.add_edge(opposite_edge);
    // Clips come from the junction layouts and set the solve's hard zones.
    graph.rebuild_intersection_clips();
    let changed_edges = graph.finalize_junction_endpoint_profiles(&HashSet::from([center]), None);
    assert!(
        changed_edges.contains(&2) && changed_edges.contains(&3),
        "both branch surfaces must meet the preserved primary corridor; unchanged control authority does not imply unchanged physical geometry"
    );
    for (edge_idx, before_geometry) in stable_before.into_iter().enumerate() {
        assert_eq!(
            graph.edge(edge_idx).geometry,
            before_geometry,
            "finalizing an added branch must not rewrite stable edge {edge_idx}"
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
