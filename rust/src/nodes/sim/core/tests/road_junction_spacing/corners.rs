// SPDX-License-Identifier: GPL-3.0-or-later

//! `ROAD-118` round JS5c: road ends make way along their own line. A stroke end whose cap
//! touches a road end's cap, where the two lines meet ahead of both ends, makes a corner where
//! they meet: the stroke runs straight on to it and the old road grows straight on to it, both
//! roads straight (`ours-2`, `ref-3`/`ref-4`). A stroke whose side passes a road end's cap makes
//! a T: the old road grows straight on to the stroke. Each case holds the round's bar: the preview
//! builds and is the commit, every drawn point stays bit for bit (outside the lead-in), the old
//! road keeps every point bit for bit plus the growth and its id, no node is left without a road,
//! and undo and redo restore the graph, the roadbed, the zoning and the buildings bit for bit.
//! Guarded road ends (an occupied parcel, another road in the way) keep round JS5a's arc.

use super::super::road_pockets::buildings::{grow, register_houses};
use super::contacts::{city, lay, preview_parked, v};
use super::lead_ins::{assert_drawn_kept, drawn_of};
use super::stubs_and_joins::{edgeless_nodes, node_near};
use super::*;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::road_edit::{EndContact, Growth, RoadFrontages};

// Every live road's centreline by id, `None` for a deleted slot.
fn roads(core: &SimCore) -> Vec<Option<Vec<Vector3>>> {
    core.region_graph
        .edges()
        .iter()
        .map(|edge| (!edge.deleted).then(|| edge.geometry.clone()))
        .collect()
}

// The zoning cells and the buildings, bit for bit where they hold floats.
fn land_hash(core: &SimCore) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    format!("{:?}", core.zoning.cells.saved_road_alignments()).hash(&mut hasher);
    for building in &core.allocator.buildings {
        (building.edge_idx, building.parcel_id).hash(&mut hasher);
        [
            building.frontage_t,
            building.side_offset,
            building.center_x,
            building.center_y,
        ]
        .map(f32::to_bits)
        .hash(&mut hasher);
    }
    hasher.finish()
}

// Graph, roadbed and land, bit for bit.
fn state(core: &SimCore) -> [u64; 3] {
    [graph_hash(core), roadbed_digest(core), land_hash(core)]
}

// Previews the 2-lane stroke `ends` with `parking`, commits that preview and holds the round's
// bar (module docs): drawn points kept, every road that stood keeps its points bit for bit,
// a grown road its points plus its growth. Undo restores the state before; committing the stroke
// again (redo) the state after.
// Returns the preview's validation and the state after.
fn build(
    core: &mut SimCore,
    ends: [Vector2; 2],
    parking: RoadParking,
    label: &str,
) -> RoadPreviewValidation {
    let points = lay(core, &ends);
    let before = roads(core);
    let state_before = state(core);
    let drawn = drawn_of(core, &points);
    let chunks = context(core).view.road_mesh_chunks.clone();
    let (preview, _) = preview_parked(core, &points, (1, 1), false, parking);
    let validation = preview.validation.clone();
    assert!(
        validation.is_valid,
        "{label}: {}",
        validation.invalid_reason
    );
    let built = preview.plan.prepared().points.clone();
    assert_drawn_kept(&drawn, &built, validation.lead_ins, label);
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let outcome = commit_with(core, &points, ((1, 1), parking), Some(Arc::clone(&preview)));
    assert!(outcome.committed, "{label}: {}", outcome.rejection);
    assert_preview_composes_committed(core, &chunks, pieces, label);
    assert!(
        core.transit_network.roadbed.failed_nodes().is_empty(),
        "{label}"
    );
    assert_eq!(edgeless_nodes(core), 0, "{label}");
    let after = roads(core);
    for (id, points) in before.iter().enumerate() {
        let Some(points) = points else { continue };
        let now = after[id]
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: road {id} stays"));
        assert_eq!(
            now[..points.len().min(now.len())],
            points[..],
            "{label}: road {id} kept"
        );
        if now.len() != points.len() {
            assert_eq!(
                now.len(),
                points.len() + 1,
                "{label}: road {id} grows by one point"
            );
        }
    }
    let state_after = state(core);
    assert!(core.undo_action_internal(), "{label}");
    assert_eq!(state(core), state_before, "{label}: undo");
    let outcome = commit_with(core, &points, ((1, 1), parking), None);
    assert!(outcome.committed, "{label}: redo");
    assert_eq!(state(core), state_after, "{label}: redo");
    validation
}

// A road of `lanes` each way with kerbside `parking` along `points`, committed with `preview`.
fn commit_with(
    core: &mut SimCore,
    points: &[Vector3],
    ((fwd, bkw), parking): ((i32, i32), RoadParking),
    preview: Option<Arc<RoadPreview>>,
) -> RoadCommitOutcome {
    core.commit_road(
        RoadCommitRequest {
            parking,
            points: points.to_vec(),
            fwd_lanes: fwd,
            bkw_lanes: bkw,
            snap_to_existing_roads: false,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            preview,
        },
        std::time::Instant::now(),
        Default::default(),
    )
}

// Whether `points` all stand on one horizontal line, within `f32` rounding.
fn straight(points: &[Vector3]) -> bool {
    let (a, b) = (points[0], points[points.len() - 1]);
    let (dx, dz) = (b.x - a.x, b.z - a.z);
    let length = dx.hypot(dz);
    points
        .iter()
        .all(|p| ((p.x - a.x) * dz - (p.z - a.z) * dx).abs() / length < 1.0e-3)
}

// The live road from `node` and its points, `node` first.
fn road_from(core: &SimCore, node: u32) -> Vec<Vec<Vector3>> {
    let graph = &core.region_graph;
    graph
        .node_adjacency(node)
        .iter()
        .filter(|&&edge| !graph.edge(edge).deleted)
        .map(|&edge| {
            let record = graph.edge(edge);
            let mut points = record.geometry.clone();
            if graph.get_valid_node(record.start_node) != node {
                points.reverse();
            }
            points
        })
        .collect()
}

// A 2-lane street ending at the origin from the west. Strokes whose end caps touch its end, at
// 30 degrees (the end 0.8 m short of the street's line), 46 degrees (`ours-2`: on the line, 8.3 m
// beyond the end) and 90 degrees (6 m beside the line): the corner stands where the lines meet,
// 6, 8.3 and 6 m beyond the old end; the street grows straight on to it and keeps its id, the
// stroke runs straight on to it, and the corner is a bend of two straight roads.
#[test]
fn two_road_ends_whose_lines_meet_make_a_corner() {
    let street = [[(-100.0, 0.0), (0.0, 0.0)]];
    // (label, the drawn end, the stroke's heading at it in degrees from +x, the corner's x)
    let cases: [(&str, (f32, f32), f32, f32); 3] = [
        ("30 degrees", (6.696, -0.402), 150.0, 6.0),
        ("46 degrees", (8.345, 0.0), 134.0, 8.345),
        ("90 degrees", (6.0, 6.0), 270.0, 6.0),
    ];
    for (label, end, heading, corner_x) in cases {
        let mut core = city(&street, 1);
        let road_end = node_near(&core, v(0.0, 0.0));
        let street_id = core.region_graph.node_adjacency(road_end)[0];
        let (dx, dz) = (heading.to_radians().cos(), heading.to_radians().sin());
        let ends = [v(end.0 - 80.0 * dx, end.1 - 80.0 * dz), v(end.0, end.1)];
        let validation = build(&mut core, ends, RoadParking::NONE, label);
        assert_eq!(
            validation.contacts,
            [None, Some(EndContact::Node(road_end))],
            "{label}"
        );
        assert_eq!(validation.joined_nodes, [None; 2], "{label}");
        let Some(Growth { node, road, target }) = validation.growths[1] else {
            panic!("{label}: a corner");
        };
        assert_eq!((node, road), (road_end, street_id), "{label}");
        assert!(
            (target.x - corner_x).abs() < 1.0e-3 && target.z.abs() < 1.0e-3,
            "{label}"
        );
        let corner = node_near(&core, v(target.x, target.z));
        assert_eq!(degree(&core, corner), 2, "{label}: a bend");
        assert_ne!(
            core.region_graph.get_valid_node(road_end),
            road_end,
            "{label}: merged"
        );
        let street_points = &core.region_graph.edge(street_id).geometry;
        assert_eq!(
            *street_points.last().unwrap(),
            target,
            "{label}: the street runs on"
        );
        for points in road_from(&core, corner) {
            assert!(straight(&points), "{label}: both roads straight");
        }
    }
}

// `ref-3`/`ref-4`: a street running north-east ends at the origin; a stroke heading north-west
// ends 6.3 m from it, its cap over the street's end. They make a square corner at (4, 4): the
// street grows 5.7 m on its own line, the stroke 2.8 m on its own.
#[test]
fn a_cap_over_a_road_end_makes_the_reference_corner() {
    let mut core = city(&[[(-60.0, -60.0), (0.0, 0.0)]], 1);
    let road_end = node_near(&core, v(0.0, 0.0));
    let validation = build(
        &mut core,
        [v(66.0, -58.0), v(6.0, 2.0)],
        RoadParking::NONE,
        "ref-3",
    );
    let growth = validation.growths[1].expect("a corner");
    assert_eq!(growth.node, road_end);
    assert!((growth.target.x - 4.0).abs() < 1.0e-3 && (growth.target.z - 4.0).abs() < 1.0e-3);
    let corner = node_near(&core, v(4.0, 4.0));
    assert_eq!(degree(&core, corner), 2);
    for points in road_from(&core, corner) {
        assert!(straight(&points), "both roads straight");
    }
}

// Differing road types: a 4-lane street's end and a 2-lane stroke with parking on both sides make
// the corner; the street keeps its 4 lanes and grows, the stroke keeps its parking.
#[test]
fn road_ends_of_differing_types_make_a_corner() {
    let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)]], 2);
    let road_end = node_near(&core, v(0.0, 0.0));
    let street = core.region_graph.node_adjacency(road_end)[0];
    let validation = build(
        &mut core,
        [v(8.0, 83.0), v(8.0, 3.0)],
        RoadParking {
            left: true,
            right: true,
        },
        "types",
    );
    assert_eq!(
        validation.growths[1].map(|growth| growth.node),
        Some(road_end)
    );
    let record = core.region_graph.edge(street);
    assert_eq!(
        (record.fwd_lanes, record.bkw_lanes),
        (2, 2),
        "the street's own lanes"
    );
    assert!(
        (record.geometry.last().unwrap().x - 8.0).abs() < 1.0e-3,
        "grown to the corner"
    );
}

// Guarded road ends keep round JS5a's arc into the road end: a house fronting the street's last
// metres, and a walkway lying across the growth's way.
#[test]
fn guarded_road_ends_keep_the_arc() {
    // A house on the street's north curb by its end.
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
    let houses = grow(&mut core, street, 1.0, [-14.0, -0.5]);
    assert!(!houses.is_empty(), "a house stands");
    let land = core.refresh_road_tool_land().expect("the land");
    let length_m = core.region_graph.edge(street).physical_length;
    assert!(land.fronted(&core.region_graph, street, [length_m - 10.0, length_m]));
    // The live parcels answer the commit's guard (the view's published land answers a preview's
    // alike, `a_stub_with_a_house_is_not_removed`).
    let road_end = node_near(&core, v(0.0, 0.0));
    let street_points = core.region_graph.edge(street).geometry.clone();
    let points = lay(&core, &[v(6.0, 86.0), v(6.0, 6.0)]);
    let outcome = commit_with(&mut core, &points, ((1, 1), RoadParking::NONE), None);
    assert_eq!(
        core.region_graph.edge(street).geometry,
        street_points,
        "the housed end keeps its place"
    );
    if outcome.committed {
        assert_eq!(degree(&core, road_end), 2, "the arc joins the road end");
    }

    // A walkway across the corner's way, 1 m beyond the road end.
    let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)]], 1);
    let walkway = lay(&core, &[v(1.5, -40.0), v(1.5, 40.0)]);
    let outcome = commit_with(&mut core, &walkway, ((0, 0), RoadParking::NONE), None);
    assert!(outcome.committed, "{}", outcome.rejection);
    let points = lay(&core, &[v(6.0, 86.0), v(6.0, 6.0)]);
    let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
    assert_eq!(
        preview.validation.growths, [None; 2],
        "the walkway is in the way"
    );
}

// A stroke running north 6 m beyond a street's end, its side over the end's cap: the street grows
// 6 m straight on to it and the stroke is split there, a T. Every drawn point stays.
#[test]
fn a_stroke_passing_a_road_end_makes_a_t() {
    let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)]], 1);
    let road_end = node_near(&core, v(0.0, 0.0));
    let street = core.region_graph.node_adjacency(road_end)[0];
    let validation = build(
        &mut core,
        [v(6.0, -60.0), v(6.0, 60.0)],
        RoadParking::NONE,
        "pass",
    );
    assert_eq!(
        validation.growths, [None; 2],
        "no stroke end makes a corner"
    );
    let t = node_near(&core, v(6.0, 0.0));
    assert_eq!(degree(&core, t), 3, "a T");
    let record = core.region_graph.edge(street);
    let last = *record.geometry.last().unwrap();
    assert!(
        (last.x - 6.0).abs() < 1.0e-3 && last.z.abs() < 1.0e-3,
        "the street runs on to it"
    );
    assert!(straight(&record.geometry));
}

// A corner's game and a pass's save and load: no node without a road, the same curb frames and
// the same network after a load (the grown road keeps `ROAD-125`'s one curb rule).
#[test]
fn a_corners_game_saves_and_loads() {
    for (label, ends) in [
        ("corner", [v(6.0, 86.0), v(6.0, 6.0)]),
        ("pass", [v(6.0, -60.0), v(6.0, 60.0)]),
    ] {
        let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)]], 1);
        build(&mut core, ends, RoadParking::NONE, label);
        let loaded = saved_and_loaded(&mut core, label);
        assert_eq!(edgeless_nodes(&loaded), 0, "{label}");
        assert_eq!(
            loaded.zoning.cells.saved_road_alignments(),
            core.zoning.cells.saved_road_alignments(),
            "{label}"
        );
        assert_same_network(&core, &loaded, label);
    }
}
