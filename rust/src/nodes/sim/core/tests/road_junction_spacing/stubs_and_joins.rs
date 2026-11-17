// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-118` round JS3 on small flat cities: a stroke end too close to a junction or a bend
//! joins it (a T becomes a crossroads, a bend a T), unless the joined stroke breaks a rule; a
//! crossing too close is refused with its place named. A stub shorter than its junction needs
//! goes, its road end merged into the junction, and undo restores it; one a house fronts keeps
//! its place, and the stroke end then joins its road end. No node is left without a road, also
//! after a save and a load.

use super::super::road_edit_order::{Stroke, committed_on};
use super::super::road_pockets::buildings::{grow, register_houses};
use super::*;
use crate::simulation::network::road_edit::RoadFrontages;

fn v(x: f32, z: f32) -> Vector2 {
    Vector2::new(x, z)
}

// A flat city of 1 + 1 roads drawn in order, its edits recording their undo.
fn city(roads: &[[(f32, f32); 2]]) -> SimCore {
    let strokes: Vec<Stroke> = roads
        .iter()
        .map(|&[a, b]| ([v(a.0, a.1), v(b.0, b.1)], 1))
        .collect();
    let order: Vec<usize> = (0..strokes.len()).collect();
    let mut core = committed_on(512.0, &strokes, &order, false);
    core.benchmark_mode = false;
    core
}

// The canonical node standing within half a metre of `point` with a road.
pub(super) fn node_near(core: &SimCore, point: Vector2) -> u32 {
    let graph = &core.region_graph;
    (0..graph.node_count() as u32)
        .find(|&node| {
            let pos = graph.node(node).pos;
            graph.get_valid_node(node) == node
                && degree(core, node) > 0
                && Vector2::new(pos.x, pos.z).distance_to(point) < 0.5
        })
        .expect("a node there")
}

// Canonical nodes without a live road.
pub(super) fn edgeless_nodes(core: &SimCore) -> usize {
    let graph = &core.region_graph;
    (0..graph.node_count() as u32)
        .filter(|&node| graph.get_valid_node(node) == node && degree(core, node) == 0)
        .count()
}

// The stroke `ends` previewed on the core's context and committed with that preview: it must
// build, its preview the commit's pieces. Returns the preview's validation.
fn build(core: &mut SimCore, ends: [Vector2; 2], label: &str) -> RoadPreviewValidation {
    let points = stroke(core, ends);
    let mut view = context(core);
    let view_chunks = view.view.road_mesh_chunks.clone();
    let preview = Arc::new(preview(&mut view, &points));
    assert!(
        preview.validation.is_valid,
        "{label}: {}",
        preview.validation.invalid_reason
    );
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let outcome = commit(core, &points, Some(Arc::clone(&preview)));
    assert!(
        outcome.committed,
        "{label}: {} {}",
        outcome.rejection_reason, outcome.rejection
    );
    assert_preview_composes_committed(core, &view_chunks, pieces, label);
    assert!(core.transit_network.roadbed.failed_nodes().is_empty(), "{label}");
    preview.validation.clone()
}

// A street west to east with a side street north from its middle: a T at the origin.
const T_CITY: [[(f32, f32); 2]; 2] = [[(-100.0, 0.0), (100.0, 0.0)], [(0.0, 0.0), (0.0, 100.0)]];

// A stroke from the south ending on the street 6 m east of the T: the end joins the T, which
// becomes a crossroads; the stroke's own point on the street is no node. The preview is the
// commit, and undo restores the graph.
#[test]
fn a_t_6_m_from_a_t_becomes_a_crossroads() {
    let mut core = city(&T_CITY);
    let t = node_near(&core, v(0.0, 0.0));
    assert_eq!(degree(&core, t), 3);
    let before = graph_hash(&core);
    let validation = build(&mut core, [v(6.0, -60.0), v(6.0, 0.0)], "T beside a T");
    assert_eq!(validation.joined_nodes, [None, Some(t)]);
    assert_eq!(validation.end_endpoint_snapped_node_id, t as i32);
    assert_eq!(degree(&core, t), 4, "a crossroads");
    assert!(!node_stands_at(&core, v(6.0, 0.0)));
    assert_eq!(edgeless_nodes(&core), 0);
    assert!(core.undo_action_internal());
    assert_eq!(graph_hash(&core), before, "undo restores the graph");
}

// A stroke from the south ending 6 m west of a quarter-turn bend: the end joins the bend, which
// becomes a T.
#[test]
fn a_t_6_m_from_a_bend_becomes_a_t_at_the_bend() {
    let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (0.0, 100.0)]]);
    let bend = node_near(&core, v(0.0, 0.0));
    assert_eq!(degree(&core, bend), 2);
    let before = graph_hash(&core);
    let validation = build(&mut core, [v(-6.0, -60.0), v(-6.0, 0.0)], "T beside a bend");
    assert_eq!(validation.joined_nodes, [None, Some(bend)]);
    assert_eq!(degree(&core, bend), 3, "the bend is a T");
    assert!(!node_stands_at(&core, v(-6.0, 0.0)));
    assert!(core.undo_action_internal());
    assert_eq!(graph_hash(&core), before);
}

// The same end from a flat angle: drawn, the stroke meets the street at 26 degrees (allowed);
// joined to the T it would meet the street's east arm at 24 degrees, under the 25 degree limit
// (`ROAD-54`), so the end does not join and the junction's own refusal stands, naming its place.
#[test]
fn a_join_under_25_degrees_is_refused() {
    let mut core = city(&T_CITY);
    let points = stroke(&core, [v(60.0, -26.5), v(6.0, 0.0)]);
    let mut view = context(&mut core);
    let refused = preview(&mut view, &points).validation;
    assert!(!refused.is_valid);
    assert!(
        matches!(
            refused.invalid_reason,
            "junction_corner_does_not_fit" | "junction_arm_too_short"
        ),
        "{}",
        refused.invalid_reason
    );
    assert!(refused.junction_place.is_some(), "the refusal names its place");
    assert_eq!(refused.joined_nodes, [None, None]);
    // The report names the road the stroke ends on and the station there.
    let rejection = RoadRejection::from_validation(refused.invalid_reason, &refused, &points);
    assert!(
        rejection.ends.starts_with("end edge ") || rejection.ends.contains(" end edge "),
        "{}",
        rejection.ends
    );
    assert!(rejection.ends.contains("at 6.0 m"), "{}", rejection.ends);
    assert!(!commit(&mut core, &points, None).committed, "the commit refuses it too");
}

// A stroke crossing the street 6 m east of the T cannot move its crossing: refused, naming the
// two junctions and the 6 m between them. (It leans away from the side street: an end 6 m beside
// it would touch it, round JS4a.)
#[test]
fn a_crossing_near_a_junction_is_refused_with_the_node_named() {
    let mut core = city(&T_CITY);
    let t = node_near(&core, v(0.0, 0.0));
    let points = stroke(&core, [v(1.0, -60.0), v(11.0, 60.0)]);
    let mut view = context(&mut core);
    let refused = preview(&mut view, &points).validation;
    assert!(!refused.is_valid);
    let place = refused.junction_place.expect("the refusal names its place");
    assert_eq!(place.roles, [NodeRole::Junction, NodeRole::Junction]);
    assert!(place.node.is_none_or(|node| node == t), "{place:?}");
    assert!((refused.geometry_measured - 6.0).abs() < 0.05);
    let text = reason_text(refused.invalid_reason, &facts(&refused)).unwrap();
    assert!(text.starts_with("Junctions only 6.0 m apart"), "{text}");
}

// A stroke from the south ending on a street 5 m short of its dead end, the street drawn either
// way: the 5 m stub goes and the T is a bend; the dead end merges into it, so no node stands
// there and none is left without a road, also after a save and a load. The street keeps its id.
// Undo restores the graph. A stroke crossing a street and ending 5 m past it loses that end too.
#[test]
fn a_5_m_stub_is_removed_and_undo_restores_it() {
    for (from, to) in [((-100.0, 0.0), (0.0, 0.0)), ((0.0, 0.0), (-100.0, 0.0))] {
        let mut core = city(&[[from, to]]);
        let street = (0..core.region_graph.edge_count())
            .find(|&edge| !core.region_graph.edge(edge).deleted)
            .unwrap();
        let before = graph_hash(&core);
        let label = format!("stub of a street from {from:?}");
        build(&mut core, [v(-5.0, -60.0), v(-5.0, 0.0)], &label);
        let bend = node_near(&core, v(-5.0, 0.0));
        assert_eq!(degree(&core, bend), 2, "{label}: the T is a bend");
        assert!(!node_stands_at(&core, v(0.0, 0.0)), "{label}: the dead end merged");
        let road = core.region_graph.edge(street);
        assert!(!road.deleted, "{label}: the street keeps its id");
        let length_m = core.region_graph.calculate_length(&road.geometry);
        assert!((length_m - 95.0).abs() < 0.05, "{label}: {length_m}");
        assert_eq!(edgeless_nodes(&core), 0, "{label}");

        let path = temp_save_path("road_stub_removal");
        core.save_game_settled_at(path.to_str().unwrap(), None, 0)
            .expect("save");
        let mut loaded = test_core();
        loaded.load_game_internal(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(edgeless_nodes(&loaded), 0, "{label}: after a load");
        assert!(!node_stands_at(&loaded, v(0.0, 0.0)), "{label}: after a load");

        assert!(core.undo_action_internal());
        assert_eq!(graph_hash(&core), before, "{label}: undo restores the stub");
    }

    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]]);
    build(&mut core, [v(-50.0, -60.0), v(-50.0, 5.0)], "stroke past a street");
    assert_eq!(degree(&core, node_near(&core, v(-50.0, 0.0))), 3, "a T");
    assert!(!node_stands_at(&core, v(-50.0, 5.0)));
    assert_eq!(edgeless_nodes(&core), 0);
}

// A 2 + 2 stroke from the south ending on a 1 + 1 street 12 m short of its dead end: its T needs
// 13 m or more on the stub, so the stub goes, the dead end merged into a bend. With a house on the
// street's north curb fronting that stub, the stub keeps its place; the end, 12 m from the dead
// end, is too far to join it, so the stroke is refused, naming the road end. The road tool's
// published land answers the guard as the live parcels do.
#[test]
fn a_stub_with_a_house_is_not_removed() {
    let commit_wide = |core: &mut SimCore, points: &[Vector3]| {
        core.commit_road(
            RoadCommitRequest {
                parking: Default::default(),
                points: points.to_vec(),
                fwd_lanes: 2,
                bkw_lanes: 2,
                snap_to_existing_roads: true,
                profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
                median: None,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                preview: None,
            },
            std::time::Instant::now(),
            Default::default(),
        )
    };
    for housed in [false, true] {
        let mut core = test_core();
        core.treasury.balance = 1.0e9;
        register_houses(&mut core);
        super::super::road_terrain_plan::commit_ready(
            &mut core,
            vec![Vector3::new(-150.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        );
        core.benchmark_mode = false;
        let street = (0..core.region_graph.edge_count())
            .find(|&edge| !core.region_graph.edge(edge).deleted)
            .unwrap();
        let length_m = core.region_graph.edge(street).physical_length;
        let stub = [length_m - 12.0, length_m];
        let houses = if housed {
            grow(&mut core, street, 1.0, [-24.0, -0.5])
        } else {
            Vec::new()
        };
        let land = core.refresh_road_tool_land().expect("the land");
        assert_eq!(
            core.zoning
                .parcels
                .fronted(&core.region_graph, street, stub),
            housed,
            "a house fronts the stub"
        );
        assert_eq!(
            land.fronted(&core.region_graph, street, stub),
            housed,
            "the view's land agrees"
        );
        let points = stroke(&core, [v(-12.0, -60.0), v(-12.0, 0.0)]);
        let outcome = commit_wide(&mut core, &points);
        if !housed {
            assert!(outcome.committed, "{}", outcome.rejection_reason);
            assert!(!node_stands_at(&core, v(0.0, 0.0)), "the stub went");
            assert_eq!(degree(&core, node_near(&core, v(-12.0, 0.0))), 2, "a bend");
            continue;
        }
        assert!(!outcome.committed, "the housed stub keeps its place");
        assert!(
            matches!(
                outcome.rejection_reason.as_str(),
                "junction_corner_does_not_fit" | "junction_arm_too_short"
            ),
            "{}",
            outcome.rejection_reason
        );
        assert!(node_stands_at(&core, v(0.0, 0.0)), "the dead end stands");
        assert!(houses.iter().all(|&house| core.allocator.buildings[house].edge_idx == street));
    }
}

// A stub removal and a join as one batch: the batch leaves the graph as the strokes one by one,
// its records share one undo group, and one undo restores the network.
#[test]
fn a_batch_of_stub_and_join_strokes_is_one_undo_group() {
    use crate::nodes::sim::core::RoadBatchRequest;
    let roads = [
        [(-100.0, 0.0), (100.0, 0.0)],
        [(0.0, 0.0), (0.0, 100.0)],
        [(-100.0, -80.0), (0.0, -80.0)],
    ];
    let ends = [[v(6.0, -60.0), v(6.0, 0.0)], [v(-5.0, -140.0), v(-5.0, -80.0)]];
    let mut single = city(&roads);
    let strokes = ends.map(|ends| stroke(&single, ends));
    for points in &strokes {
        assert!(commit(&mut single, points, None).committed);
    }
    let mut core = city(&roads);
    let before = graph_hash(&core);
    let depth = core.undo_stack.len();
    let request = |points: &Vec<Vector3>| RoadCommitRequest {
        parking: Default::default(),
        points: points.clone(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        preview: None,
    };
    let batch = core.commit_road_batch(
        RoadBatchRequest {
            strokes: strokes.iter().map(request).collect(),
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(batch.outcome.committed, "{}", batch.outcome.rejection_reason);
    assert_eq!(graph_hash(&core), graph_hash(&single), "the batch is its strokes");
    let groups: Vec<u64> = core.undo_stack.iter().skip(depth).map(|record| record.group).collect();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0], groups[1], "one undo group");
    assert!(core.undo_action_internal());
    assert_eq!(graph_hash(&core), before);
}
