// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-118` round JS4a: roads that touch join. The user's `dump-2026-11-15_11-54-18`: 16
//! roads, each drawn with its end cap on another road's side and no junction anywhere. Each of
//! the 15 ends is drawn again onto the dump's other roads (laid as the save holds them, the
//! overlaps unjoined), with the Roads snap off and on: 14 run on along their tangents and make a
//! T, one at 9 degrees is refused as `roads_overlap`. Then the user's later cases on small flat
//! cities: an end cap inside a wide T's piece, over a bend's piece, over a road end's cap (in
//! line, and only just touching at 30 degrees), and ends that stay a footprint short.

use super::super::road_edit_order::committed_on;
use super::stubs_and_joins::{edgeless_nodes, node_near};
use super::*;
use crate::nodes::sim::core::RoadPreviewRequest;
use crate::simulation::network::road_edit::EndContact;
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::roadbed::fixtures::road_edge;
use crate::simulation::network::types::NodeType;
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::pathing::cost::CostCalculator;

// Per road `edge id start end fwd bkw`, then its centreline `x z` (the dump is flat).
const DUMP: &str = include_str!("dump_2026_11_15_11_54_18.txt");

struct DumpRoad {
    lanes: (u8, u8),
    points: Vec<Vector2>,
}

fn dump_roads() -> Vec<DumpRoad> {
    let mut roads: Vec<DumpRoad> = Vec::new();
    for line in DUMP.lines().filter(|line| !line.starts_with('#')) {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words[0] == "edge" {
            let lanes = (words[4].parse().unwrap(), words[5].parse().unwrap());
            roads.push(DumpRoad {
                lanes,
                points: Vec::new(),
            });
        } else {
            let point = Vector2::new(words[0].parse().unwrap(), words[1].parse().unwrap());
            roads.last_mut().unwrap().points.push(point);
        }
    }
    roads
}

fn lay(core: &SimCore, points: &[Vector2]) -> Vec<Vector3> {
    points
        .iter()
        .map(|&point| {
            Vector3::new(
                point.x,
                core.get_world_surface_height_internal(point),
                point.y,
            )
        })
        .collect()
}

// The dump's roads but `left_out`, laid as its save holds them (each road between its own two
// road ends, the touching ones unjoined) and loaded from a save; its edits record their undo.
fn dump_city_without(left_out: usize) -> SimCore {
    let mut core = test_core();
    core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
        .expect("blank world");
    for (_, road) in dump_roads()
        .iter()
        .enumerate()
        .filter(|&(id, _)| id != left_out)
    {
        let points = lay(&core, &road.points);
        let graph = &mut core.region_graph;
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let (fwd, bkw) = road.lanes;
        let mut edge = road_edge(
            start,
            end,
            points,
            crate::config::LANE_WIDTH * f32::from(fwd + bkw),
        );
        (edge.fwd_lanes, edge.bkw_lanes) = (fwd, bkw);
        edge.speed_limit = RoadAttributes::DEFAULT.speed_ms;
        (edge.base_cost, edge.physical_length) = CostCalculator::calculate_costs(&edge);
        graph.add_edge(edge);
    }
    core.region_graph.rebuild_intersection_clips();
    core.transit_network.rebuild_roadbed(&core.region_graph);
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.transit_network.cch_graph = CchGraph::build(&core.region_graph);
    let path = temp_save_path("road_contacts_dump");
    core.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .expect("save");
    let mut loaded = test_core();
    loaded
        .load_game_internal(path.to_str().unwrap())
        .expect("the dump's roads load");
    let _ = std::fs::remove_file(&path);
    loaded.treasury.balance = 1.0e9;
    loaded.benchmark_mode = false;
    loaded
}

fn preview_lanes(
    core: &mut SimCore,
    points: &[Vector3],
    (fwd, bkw): (u8, u8),
    snap: bool,
) -> (
    Arc<RoadPreview>,
    crate::simulation::network::render::RoadChunkMap,
) {
    let mut view = context(core);
    let chunks = view.view.road_mesh_chunks.clone();
    let surface_generation = view.view.surface_generation;
    let preview = view.preview_road(RoadPreviewRequest {
        parking: Default::default(),
        surface_generation,
        points: points.to_vec(),
        fwd_lanes: i32::from(fwd),
        bkw_lanes: i32::from(bkw),
        snap_to_existing_roads: snap,
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        attributes: RoadAttributes::DEFAULT,
    });
    (Arc::new(preview), chunks)
}

fn commit_lanes(
    core: &mut SimCore,
    points: &[Vector3],
    (fwd, bkw): (u8, u8),
    snap: bool,
    preview: Option<Arc<RoadPreview>>,
) -> RoadCommitOutcome {
    core.commit_road(
        RoadCommitRequest {
            parking: Default::default(),
            points: points.to_vec(),
            fwd_lanes: i32::from(fwd),
            bkw_lanes: i32::from(bkw),
            snap_to_existing_roads: snap,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            attributes: RoadAttributes::DEFAULT,
            preview,
        },
        std::time::Instant::now(),
        Default::default(),
    )
}

// Previews `points` and commits that preview: it must build, the preview the commit's pieces.
// Returns the preview's validation.
fn build_lanes(
    core: &mut SimCore,
    points: &[Vector3],
    lanes: (u8, u8),
    snap: bool,
    label: &str,
) -> RoadPreviewValidation {
    let (preview, view_chunks) = preview_lanes(core, points, lanes, snap);
    let validation = preview.validation.clone();
    assert!(
        validation.is_valid,
        "{label}: {}",
        validation.invalid_reason
    );
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let outcome = commit_lanes(core, points, lanes, snap, Some(Arc::clone(&preview)));
    assert!(
        outcome.committed,
        "{label}: {} {}",
        outcome.rejection_reason, outcome.rejection
    );
    assert_preview_composes_committed(core, &view_chunks, pieces, label);
    assert!(
        core.transit_network.roadbed.failed_nodes().is_empty(),
        "{label}"
    );
    assert_eq!(edgeless_nodes(core), 0, "{label}");
    validation
}

// Where the built stroke's end `end` stands: the node it joined or the point it ran on to.
fn contact_node(core: &SimCore, validation: &RoadPreviewValidation, end: usize) -> u32 {
    match validation.contacts[end] {
        Some(EndContact::Node(node)) => node,
        Some(EndContact::Road(point)) => node_near(core, Vector2::new(point.x, point.z)),
        None => panic!("end {end} touched nothing"),
    }
}

// The 15 ends of the dump, each drawn again onto the dump's other roads with the Roads snap off
// and on. 14 run on to the road they touch and make a T there (no node where the end was drawn);
// edge 14's end meets edge 13 at 9 degrees and is refused as `roads_overlap`, naming the road.
// The preview is the commit, nothing is left without a road, also after a save and a load, undo
// restores the graph and the roadbed, and the snap does not change the graph.
#[test]
fn the_dumps_15_ends_join_the_roads_they_touch() {
    let roads = dump_roads();
    for (id, road) in roads.iter().enumerate().skip(1) {
        let mut graphs = Vec::new();
        for snap in [false, true] {
            let label = format!("edge {id}, snap {snap}");
            let mut core = dump_city_without(id);
            let before = (graph_hash(&core), roadbed_digest(&core));
            let points = lay(&core, &road.points);
            if id == 14 {
                let (preview, _) = preview_lanes(&mut core, &points, road.lanes, snap);
                let refused = &preview.validation;
                assert_eq!(refused.invalid_reason, "roads_overlap", "{label}");
                assert_eq!(
                    refused.endpoint_edges[1].map(|(edge, _)| edge),
                    Some(13),
                    "{label}"
                );
                assert!(
                    (refused.geometry_measured - 9.0).abs() < 1.0,
                    "{label}: {refused:?}"
                );
                let text = reason_text(refused.invalid_reason, &facts(refused)).unwrap();
                assert!(text.starts_with("Roads overlap: they meet at 9°"), "{text}");
                let outcome = commit_lanes(&mut core, &points, road.lanes, snap, None);
                assert!(!outcome.committed, "{label}: the commit refuses it too");
                continue;
            }
            let validation = build_lanes(&mut core, &points, road.lanes, snap, &label);
            assert!(
                matches!(validation.contacts, [None, Some(EndContact::Road(_))]),
                "{label}: {:?}",
                validation.contacts
            );
            let t = contact_node(&core, &validation, 1);
            assert_eq!(degree(&core, t), 3, "{label}: a T");
            let drawn = *road.points.last().unwrap();
            assert!(
                !node_stands_at(&core, drawn),
                "{label}: no node where it was drawn"
            );
            graphs.push(graph_hash(&core));

            if !snap {
                let path = temp_save_path("road_contacts_join");
                core.save_game_settled_at(path.to_str().unwrap(), None, 0)
                    .expect("save");
                let mut loaded = test_core();
                loaded.load_game_internal(path.to_str().unwrap()).unwrap();
                let _ = std::fs::remove_file(&path);
                assert_eq!(edgeless_nodes(&loaded), 0, "{label}: after a load");
            }
            assert!(core.undo_action_internal(), "{label}");
            core.precompute_road_mesh_data();
            assert_eq!(
                (graph_hash(&core), roadbed_digest(&core)),
                before,
                "{label}: undo restores the graph and the roadbed"
            );
        }
        if id != 14 {
            assert_eq!(graphs[0], graphs[1], "edge {id}: the snap changes nothing");
        }
    }
}

// The dump's first stroke drawn the other way: its start touches the road and joins it as a T.
#[test]
fn a_start_touching_a_road_joins_it() {
    let roads = dump_roads();
    let mut core = dump_city_without(1);
    let mut points = lay(&core, &roads[1].points);
    points.reverse();
    let validation = build_lanes(&mut core, &points, roads[1].lanes, false, "start");
    assert!(
        matches!(validation.contacts, [Some(EndContact::Road(_)), None]),
        "{:?}",
        validation.contacts
    );
    let t = contact_node(&core, &validation, 0);
    assert_eq!(degree(&core, t), 3, "a T");
}

fn v(x: f32, z: f32) -> Vector2 {
    Vector2::new(x, z)
}

// A flat city of roads with `lanes` each way, drawn in order, its edits recording their undo.
fn city(roads: &[[(f32, f32); 2]], lanes: i32) -> SimCore {
    let strokes: Vec<_> = roads
        .iter()
        .map(|&[a, b]| ([v(a.0, a.1), v(b.0, b.1)], lanes))
        .collect();
    let order: Vec<usize> = (0..strokes.len()).collect();
    let mut core = committed_on(512.0, &strokes, &order, false);
    core.benchmark_mode = false;
    core
}

// A 2-lane stroke between `ends`, built.
fn build_2_lane(core: &mut SimCore, ends: [Vector2; 2], label: &str) -> RoadPreviewValidation {
    let points = lay(core, &ends);
    build_lanes(core, &points, (1, 1), false, label)
}

// A T of 4-lane roads: a 2-lane stroke from the south ends 12 m east of the T's node and 10 m
// south of the street, its cap inside the T's piece (the street's mouth there lies 13.5 m out),
// 15.6 m from the node (round JS3 joined ends within 8.5 m only): it joins the T, a crossroads.
// Undo restores the graph. Ending on the street itself 12 m out, inside the piece, it joins the
// T as well (round JS3's join, its reach now the piece).
#[test]
fn a_cap_inside_a_wide_t_joins_it() {
    let wide_t = [[(-100.0, 0.0), (100.0, 0.0)], [(0.0, 0.0), (0.0, 100.0)]];
    let mut core = city(&wide_t, 2);
    let t = node_near(&core, v(0.0, 0.0));
    let before = graph_hash(&core);
    let validation = build_2_lane(&mut core, [v(12.0, -70.0), v(12.0, -10.0)], "wide T");
    assert_eq!(validation.contacts, [None, Some(EndContact::Node(t))]);
    assert_eq!(validation.joined_nodes, [None, Some(t)]);
    assert_eq!(degree(&core, t), 4, "a crossroads");
    assert!(core.undo_action_internal());
    assert_eq!(graph_hash(&core), before);

    let mut core = city(&wide_t, 2);
    let validation = build_2_lane(&mut core, [v(12.0, -70.0), v(12.0, 0.0)], "on the street");
    assert_eq!(validation.contacts, [None; 2]);
    assert_eq!(validation.joined_nodes, [None, Some(t)]);
    assert_eq!(degree(&core, t), 4, "on the street: a crossroads");
}

// A quarter-turn bend of 2-lane roads: a stroke from the south ends 4 m west of the bend and
// 9 m south of its west road, 9.8 m from the node, its cap on the bend's piece: it joins the
// bend, which becomes a T.
#[test]
fn a_cap_on_a_bend_joins_it() {
    let mut core = city(
        &[[(-100.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (0.0, 100.0)]],
        1,
    );
    let bend = node_near(&core, v(0.0, 0.0));
    let west = (0..core.region_graph.edge_count())
        .find(|&edge| !core.region_graph.edge(edge).deleted)
        .unwrap();
    assert!(
        core.region_graph.edge(west).end_clip >= 4.0,
        "the bend's piece reaches 4 m"
    );
    let validation = build_2_lane(&mut core, [v(-4.0, -70.0), v(-4.0, -9.0)], "bend");
    assert_eq!(validation.contacts, [None, Some(EndContact::Node(bend))]);
    assert_eq!(degree(&core, bend), 3, "the bend is a T");
}

// A street ending at the origin: a stroke in line from the east whose cap overlaps the street's
// end joins it, one road on; one 30 degrees off whose cap only just touches it (9.9 m between
// the ends of two 5 m caps) joins it as a bend. 10.5 m apart they stay two road ends.
#[test]
fn a_cap_over_a_road_end_joins_it() {
    let street = [[(-100.0, 0.0), (0.0, 0.0)]];
    let (dx, dz) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
    let cases = [
        ("in line", [v(90.0, 0.0), v(6.0, 0.0)], true),
        (
            "just touching",
            [v(90.0 * dx, 90.0 * dz), v(9.9 * dx, 9.9 * dz)],
            true,
        ),
        (
            "apart",
            [v(90.0 * dx, 90.0 * dz), v(10.5 * dx, 10.5 * dz)],
            false,
        ),
    ];
    for (label, ends, joins) in cases {
        let mut core = city(&street, 1);
        let road_end = node_near(&core, v(0.0, 0.0));
        let validation = build_2_lane(&mut core, ends, label);
        if joins {
            assert_eq!(
                validation.contacts,
                [None, Some(EndContact::Node(road_end))],
                "{label}"
            );
            assert_eq!(degree(&core, road_end), 2, "{label}: one road on");
            assert!(!node_stands_at(&core, ends[1]), "{label}");
        } else {
            assert_eq!(validation.contacts, [None; 2], "{label}");
            assert_eq!(degree(&core, road_end), 1, "{label}: two road ends");
        }
    }
}

// A stroke from the south ending 11 m short of a 2-lane street's centreline, its cap a metre
// short of the street's sidewalk: the roads stay apart.
#[test]
fn an_end_a_metre_short_of_touching_stays_separate() {
    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]], 1);
    let validation = build_2_lane(&mut core, [v(10.0, -70.0), v(10.0, -11.0)], "short");
    assert_eq!(validation.contacts, [None; 2]);
    let roads = (0..core.region_graph.edge_count())
        .filter(|&edge| !core.region_graph.edge(edge).deleted)
        .count();
    assert_eq!(roads, 2, "the street is not split");
}
