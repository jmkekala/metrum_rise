// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68` invariant 6 (open question 15): edit histories that produce the same graph produce
//! the same graph profiles, clips and roadbed pieces, bitwise. The fixtures are the rendered
//! reference junctions, committed stroke by stroke through `SimCore::commit_road` as the
//! reference harness does, flat and on its 2.5% slope. A stroke reads no carriageway of a piece
//! its edit rebuilds (open question 16); on the slope, terminal extensions and mid-span meetings
//! still depend on drawing order (open question 30).

use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::network::roadbed::{PieceMesh, RoadVec3};
use godot::prelude::Vector2;
use std::collections::BTreeMap;

pub(super) const ARM_M: f32 = 80.0;

type GraphBits = [u32; 3];
// Stroke ends and lanes per direction.
pub(super) type Stroke = ([Vector2; 2], i32);
type PieceBits = [u64; 3];

// Authored and derived state of one live edge, keyed by its end positions.
#[derive(Debug, PartialEq)]
struct EdgeState {
    control: Vec<GraphBits>,
    physical: Vec<GraphBits>,
    clips: [u32; 2],
}

#[derive(Debug, PartialEq)]
struct NetworkState {
    edges: BTreeMap<(GraphBits, GraphBits), EdgeState>,
    nodes: BTreeMap<GraphBits, Vec<[PieceBits; 3]>>,
    spans: BTreeMap<(GraphBits, GraphBits), Vec<[PieceBits; 3]>>,
}

fn graph_bits(point: Vector3) -> GraphBits {
    point.to_array().map(f32::to_bits)
}

fn piece_bits(point: RoadVec3) -> PieceBits {
    [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()]
}

// Every triangle as a sorted triple of vertex positions.
fn triangles(mesh: &PieceMesh) -> Vec<[PieceBits; 3]> {
    let mut triangles: Vec<_> = mesh
        .triangles()
        .iter()
        .map(|triangle| {
            let mut points = triangle.map(|vertex| piece_bits(mesh.positions()[vertex as usize]));
            points.sort_unstable();
            points
        })
        .collect();
    triangles.sort_unstable();
    triangles
}

fn state(core: &SimCore) -> NetworkState {
    let graph = &core.region_graph;
    let roadbed = &core.transit_network.roadbed;
    assert!(
        roadbed.failed_nodes().is_empty(),
        "{:?}",
        roadbed.failed_nodes()
    );
    let position = |node: u32| graph_bits(graph.node(graph.get_valid_node(node)).pos);
    let mut result = NetworkState {
        edges: BTreeMap::new(),
        nodes: BTreeMap::new(),
        spans: BTreeMap::new(),
    };
    for index in 0..graph.edge_count() {
        let edge = graph.edge(index);
        if edge.deleted {
            continue;
        }
        let key = (position(edge.start_node), position(edge.end_node));
        let points = |points: &[Vector3]| points.iter().copied().map(graph_bits).collect();
        result.edges.insert(
            key,
            EdgeState {
                control: points(&edge.geometry),
                physical: points(&edge.physical_geometry),
                clips: [edge.start_clip.to_bits(), edge.end_clip.to_bits()],
            },
        );
        if let Some(span) = roadbed.span_piece(index) {
            result.spans.insert(key, triangles(span.mesh()));
        }
        for node in [edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node)) {
            if let Some(piece) = roadbed.node_piece(node) {
                result.nodes.insert(position(node), triangles(piece.mesh()));
            }
        }
    }
    result
}

// Commits `strokes` in `order` on the reference harness's blank world.
fn build(strokes: &[Stroke], order: &[usize], sloped: bool) -> NetworkState {
    state(&committed(strokes, order, sloped))
}

pub(super) fn committed(strokes: &[Stroke], order: &[usize], sloped: bool) -> SimCore {
    committed_on(512.0, strokes, order, sloped)
}

// As `committed`, on a square world `world_m` wide; the slope stays the 256 m one at the centre.
pub(super) fn committed_on(
    world_m: f32,
    strokes: &[Stroke],
    order: &[usize],
    sloped: bool,
) -> SimCore {
    committed_with(world_m, strokes, order, |core| {
        if sloped {
            slope(core);
        }
    })
}

// The reference harness's 2.5% slope across the 256 m around the centre.
fn slope(core: &mut SimCore) {
    core.slope_terrain_internal(
        Vector2::ZERO,
        256.0,
        Vector2::new(-128.0, 0.0),
        0.0,
        Vector2::new(128.0, 0.0),
        6.4,
        1.0,
    );
}

// As `committed_on`, with the blank world's terrain shaped by `shape` first.
fn committed_with(
    world_m: f32,
    strokes: &[Stroke],
    order: &[usize],
    shape: impl Fn(&mut SimCore),
) -> SimCore {
    let mut core = test_core();
    core.create_blank_world_internal(world_m, world_m, 8.0, 128.0, 0.0)
        .expect("blank world");
    core.treasury.balance = 1.0e9;
    shape(&mut core);
    for &index in order {
        let surface = |core: &SimCore, point: Vector2| {
            Vector3::new(
                point.x,
                core.get_world_surface_height_internal(point),
                point.y,
            )
        };
        let (ends, lanes) = strokes[index];
        let points = ends.map(|point| surface(&core, point)).to_vec();
        let outcome = core.commit_road(
            RoadCommitRequest {
                points,
                fwd_lanes: lanes,
                bkw_lanes: lanes,
                snap_to_existing_roads: true,
                preview: None,
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert!(outcome.committed, "stroke {index}: {}", outcome.rejection);
    }
    core
}

// Every live edge's physical profile, against the same graph's clips and profiles solved from
// scratch: the graph an edit leaves must not depend on what it held before.
fn assert_profiles_fresh(name: &str, core: &SimCore) {
    let mut fresh = core.region_graph.clone();
    fresh.rebuild_intersection_clips();
    let nodes = (0..fresh.node_count() as u32).collect();
    fresh.finalize_junction_endpoint_profiles(&nodes);
    for index in 0..fresh.edge_count() {
        let (edge, solved) = (core.region_graph.edge(index), fresh.edge(index));
        if edge.deleted {
            continue;
        }
        assert_eq!(
            [edge.start_clip, edge.end_clip],
            [solved.start_clip, solved.end_clip],
            "{name}: clips of edge {index}"
        );
        assert_eq!(
            edge.physical_geometry, solved.physical_geometry,
            "{name}: profile of edge {index}"
        );
    }
}

// A node move and a road removal re-derive every profile they reach: moving the T's side-road end
// changes its arm's sample, so the junction's plane and every arm's pin; removing the side road
// turns the junction into a pass-through.
#[test]
fn node_moves_and_removals_leave_freshly_solved_profiles() {
    let strokes = [
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::ZERO, Vector2::new(0.0, ARM_M)], 1),
    ];
    for sloped in [false, true] {
        let mut core = committed(&strokes, &[0, 1], sloped);
        let graph = &core.region_graph;
        let side_end = (0..graph.node_count() as u32)
            .find(|&node| {
                let pos = graph.node(node).pos;
                (pos.x, pos.z) == (0.0, ARM_M)
            })
            .unwrap();
        let target = Vector2::new(24.0, ARM_M);
        let height = core.get_world_surface_height_internal(target);
        let to = Vector3::new(target.x, height + 1.5, target.y);
        assert!(core.move_network_node_internal(side_end as i32, to));
        assert_profiles_fresh(&format!("move, sloped {sloped}"), &core);
        let side = (0..core.region_graph.edge_count())
            .find(|&edge| {
                let edge = core.region_graph.edge(edge);
                !edge.deleted && [edge.start_node, edge.end_node].contains(&side_end)
            })
            .unwrap();
        assert!(core.bulldoze_road_edge_for_test(side));
        assert_profiles_fresh(&format!("removal, sloped {sloped}"), &core);
    }
}

fn assert_order_independent(name: &str, strokes: &[Stroke], orders: &[&[usize]], sloped: bool) {
    let reference = build(strokes, orders[0], sloped);
    for order in &orders[1..] {
        let other = build(strokes, order, sloped);
        assert_eq!(
            reference.edges, other.edges,
            "{name} sloped {sloped}: graph profiles and clips, order {order:?}"
        );
        assert_eq!(
            reference.nodes, other.nodes,
            "{name} sloped {sloped}: node pieces, order {order:?}"
        );
        assert_eq!(
            reference.spans, other.spans,
            "{name} sloped {sloped}: span pieces, order {order:?}"
        );
    }
}

// The Y fixtures at 25, 40 and 60 degrees: the stem ends at the origin heading +x, the branches
// leave it at +/- half the angle. Every order of the three strokes.
fn y_junctions(sloped: bool) {
    for angle_deg in [25.0_f32, 40.0, 60.0] {
        let half = (0.5 * angle_deg).to_radians();
        let strokes = [
            ([Vector2::new(-ARM_M, 0.0), Vector2::ZERO], 1),
            ([Vector2::ZERO, Vector2::from_angle(half) * ARM_M], 1),
            ([Vector2::ZERO, Vector2::from_angle(-half) * ARM_M], 1),
        ];
        assert_order_independent(
            &format!("y_{angle_deg}"),
            &strokes,
            &[
                &[0, 1, 2],
                &[0, 2, 1],
                &[1, 0, 2],
                &[1, 2, 0],
                &[2, 0, 1],
                &[2, 1, 0],
            ],
            sloped,
        );
    }
}

// A cross whose second stroke splits the first between its knots, and a through road with two
// side roads 50 m apart, whose link edge is pinned at both ends: through road first, last and
// between.
fn splits_and_adjacent_junctions(sloped: bool) {
    let cross = [
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::new(20.0, -ARM_M), Vector2::new(20.0, ARM_M)], 1),
    ];
    assert_order_independent("cross", &cross, &[&[0, 1], &[1, 0]], sloped);
    let pair = [
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::new(-30.0, 0.0), Vector2::new(-30.0, ARM_M)], 1),
        ([Vector2::new(20.0, 0.0), Vector2::new(20.0, -ARM_M)], 1),
    ];
    assert_order_independent(
        "two_junctions",
        &pair,
        &[&[0, 1, 2], &[1, 0, 2], &[1, 2, 0]],
        sloped,
    );
}

// The plain T and the T whose through road is 2+2 lanes, through road first and last.
fn t_junctions(sloped: bool) {
    for through_lanes in [1, 2] {
        let strokes = [
            (
                [Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)],
                through_lanes,
            ),
            ([Vector2::ZERO, Vector2::new(0.0, ARM_M)], 1),
        ];
        assert_order_independent(
            &format!("t_{through_lanes}"),
            &strokes,
            &[&[0, 1], &[1, 0]],
            sloped,
        );
    }
}

#[test]
fn flat_junctions_do_not_depend_on_drawing_order() {
    y_junctions(false);
    t_junctions(false);
    splits_and_adjacent_junctions(false);
}

// Open question 16: a stroke reads no piece its own edit rebuilds, so the through road of a T
// drawn after its side road ignores the side road's cap and span, and the side road drawn
// second meets the through road at its graph profile. Open question 30 (a): a stroke from a
// road end joins it like any node, so every order of a Y solves its roads alike.
#[test]
fn sloped_junctions_do_not_depend_on_drawing_order() {
    y_junctions(true);
    t_junctions(true);
}

// Mid-span meetings still depend on drawing order (open question 30): by question 16's crossing
// rule the road drawn second meets the first at its graph profile, so the first sets the height.
#[test]
#[ignore = "ROAD-68-E: open question 30, a mid-span meeting takes the first-drawn road's height"]
fn sloped_mid_span_meetings_do_not_depend_on_drawing_order() {
    splits_and_adjacent_junctions(true);
}

// Open question 30 (a): a stroke from a road end on the slope, straight on or bending, leaves the
// road it extends as it was and meets it at the node's height, with grades within the profile
// limits and no kink across the joint beyond the profile's curvature rule.
#[test]
fn a_stroke_from_a_road_end_joins_it_continuously_on_a_slope() {
    // The profile solve's design grade and grade change per metre (`road_edit/input.rs`).
    const MAX_GRADE: f32 = 0.16;
    const MAX_GRADE_CHANGE_PER_M: f32 = 0.018;
    let stem = ([Vector2::new(-ARM_M, 0.0), Vector2::ZERO], 1);
    for end in [Vector2::new(ARM_M, 0.0), Vector2::new(60.0, 40.0)] {
        let strokes = [stem, ([Vector2::ZERO, end], 1)];
        let first = committed(&strokes, &[0], true);
        let core = committed(&strokes, &[0, 1], true);
        let graph = &core.region_graph;
        let node = (0..graph.node_count() as u32)
            .map(|node| graph.get_valid_node(node))
            .find(|&node| graph.node(node).pos.x == 0.0 && graph.node(node).pos.z == 0.0)
            .expect("the joint");
        let joint = graph.node(node).pos;
        let arms: Vec<_> = graph
            .node_adjacency(node)
            .iter()
            .map(|&edge| graph.edge(edge))
            .filter(|edge| !edge.deleted)
            .collect();
        assert_eq!(arms.len(), 2, "end {end:?}");
        let stem_before = &first.region_graph.edges()[0];
        let stem_after = arms
            .iter()
            .find(|edge| edge.geometry[0] == stem_before.geometry[0])
            .expect("the stem keeps its id and start");
        assert_eq!(
            stem_after.geometry, stem_before.geometry,
            "end {end:?}: the stroke leaves the stem's control profile as it was"
        );
        // Each arm's grade beside the joint, measured away from it.
        let grades: Vec<_> = arms
            .iter()
            .map(|edge| {
                let profile = &edge.physical_geometry;
                let (at, next) = if edge.end_node == node {
                    (profile[profile.len() - 1], profile[profile.len() - 2])
                } else {
                    (profile[0], profile[1])
                };
                assert_eq!(at, joint, "end {end:?}: the arm ends at the joint");
                let run = (next.x - at.x).hypot(next.z - at.z);
                ((next.y - at.y) / run, run)
            })
            .collect();
        for &(grade, _) in &grades {
            assert!(grade.abs() <= MAX_GRADE, "end {end:?}: grade {grade}");
        }
        // Through the joint the road climbs on one side as much as it falls on the other.
        let change = (grades[0].0 + grades[1].0).abs();
        let mean_run = 0.5 * (grades[0].1 + grades[1].1);
        assert!(
            change <= MAX_GRADE_CHANGE_PER_M * mean_run,
            "end {end:?}: grade change {change} over {mean_run} m at the joint"
        );
    }
}

// Open question 16's crossing exception: a road crossing a raised road mid-span meets it on top,
// at the crossed road's graph profile, although the edit rebuilds the crossed span. The through
// road crosses a 3 m pit on fill; the side road crosses it at the pit's centre.
#[test]
fn a_crossing_of_a_raised_road_meets_it_on_top() {
    let centre = Vector2::new(20.0, 0.0);
    let strokes = [
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::new(centre.x, -ARM_M), Vector2::new(centre.x, ARM_M)], 1),
    ];
    let pit = |core: &mut SimCore| {
        slope(core);
        core.level_terrain_internal(centre, 14.0, -3.0, 1.0);
    };
    let through = committed_with(512.0, &strokes, &[0], pit);
    let terrain_m =
        through.heightmap.sample_height_world(centre.x, centre.y) * crate::config::HEIGHT_SCALE;
    let graph = &through.region_graph;
    let edge = (0..graph.edge_count())
        .find(|&edge| !graph.edge(edge).deleted)
        .unwrap();
    let road_m = graph
        .edge(edge)
        .physical_geometry
        .windows(2)
        .find(|pair| pair[0].x <= centre.x && centre.x <= pair[1].x)
        .map(|pair| pair[0].lerp(pair[1], (centre.x - pair[0].x) / (pair[1].x - pair[0].x)).y)
        .unwrap();
    assert!(
        road_m > terrain_m + 1.0,
        "the through road is not raised: road {road_m} m, terrain {terrain_m} m"
    );
    let crossed = committed_with(512.0, &strokes, &[0, 1], pit);
    let graph = &crossed.region_graph;
    let junction = (0..graph.node_count() as u32)
        .map(|node| graph.get_valid_node(node))
        .find(|&node| {
            let pos = graph.node(node).pos;
            (pos.x - centre.x).abs() < 0.5 && pos.z.abs() < 0.5
        })
        .expect("the side road crosses the through road at a junction");
    assert_eq!(graph.node_adjacency(junction).len(), 4);
    let junction_m = graph.node(junction).pos.y;
    assert!(
        (junction_m - road_m).abs() < 0.05,
        "the crossing at {junction_m} m does not meet the raised road at {road_m} m"
    );
}
