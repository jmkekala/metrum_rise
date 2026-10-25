// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68` invariant 6 (open question 15): edit histories that produce the same graph produce
//! the same graph profiles, clips and roadbed pieces, bitwise. The fixtures are the rendered
//! reference junctions, committed stroke by stroke through `SimCore::commit_road` as the
//! reference harness does, flat and on its 2.5% slope. A stroke reads no carriageway of a piece
//! its edit rebuilds (open question 16) and joins a road end like any node (open question 30
//! (a)), so T, Y and end-to-end joins are order independent on the slope too. A mid-span meeting
//! takes the first-drawn road's height, the crossing rule (open questions 16 and 30), so on the
//! slope the cross and the two-junction fixture are asserted by that rule in every order instead.

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

// Open question 11 (c): a terrain edit re-grades the roads, and the junctions it leaves compile
// with freshly solved profiles, so it neither publishes a gap nor has to be rolled back. A 2 m
// bump in the corner of the sloped T.
#[test]
fn a_terrain_edit_beside_a_sloped_junction_leaves_no_failed_node() {
    let strokes = [
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::ZERO, Vector2::new(0.0, ARM_M)], 1),
    ];
    let mut core = committed(&strokes, &[0, 1], true);
    let at = Vector2::new(10.0, 10.0);
    let height = core.get_world_surface_height_internal(at);
    let terrain_before = core.heightmap.clone_visual_dense();
    core.level_terrain_internal(at, 12.0, height + 2.0, 1.0);
    let roadbed = &core.transit_network.roadbed;
    let failed: Vec<_> = roadbed.failed_nodes().to_vec();
    assert!(failed.is_empty(), "failed nodes after the edit: {failed:?}");
    assert!(
        core.heightmap.clone_visual_dense() != terrain_before,
        "the edit was rolled back: {:?}",
        core.last_road_rejection.as_ref().map(|r| &r.detail)
    );
    assert_profiles_fresh("terrain edit", &core);
}

// The terrain spike's `t_cutfill`: a T drawn on flat ground, then the ground sloped 8% under it,
// falling 10.24 m to the west and rising as far to the east of the junction. The edit re-grades
// the roads and re-solves the junction (open question 11 (c)), so it stands with every node
// built; before that re-solve the junction lost its piece.
#[test]
fn an_8_percent_slope_under_a_flat_t_leaves_every_node_built() {
    let strokes = [
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::ZERO, Vector2::new(0.0, ARM_M)], 1),
    ];
    let mut core = committed(&strokes, &[0, 1], false);
    let terrain_before = core.heightmap.clone_visual_dense();
    core.slope_terrain_internal(
        Vector2::ZERO,
        256.0,
        Vector2::new(-128.0, 0.0),
        -10.24,
        Vector2::new(128.0, 0.0),
        10.24,
        1.0,
    );
    let roadbed = &core.transit_network.roadbed;
    assert!(
        roadbed.is_current() && roadbed.failed_nodes().is_empty(),
        "failed nodes after the edit: {:?}",
        roadbed.failed_nodes()
    );
    assert!(
        core.heightmap.clone_visual_dense() != terrain_before,
        "the edit was rolled back: {:?}",
        core.last_road_rejection.as_ref().map(|r| &r.detail)
    );
    assert_profiles_fresh("terrain edit", &core);
}

// The terrain spike's `city_grid_t`: closed 96 m blocks, then a T split between two of the
// grid's streets at the origin, on the reference harness's slope steepened to 8%. The split
// re-derives the grid edges at the crossings beside it, whose far crossings the plan's excerpt
// held without all their arms, so one failed `MouthOffPlane` (`ROAD-68` open question 8).
#[test]
fn a_street_grid_t_split_on_an_8_percent_slope_commits() {
    let strokes: [Stroke; 6] = [
        ([Vector2::new(-150.0, 0.0), Vector2::new(150.0, 0.0)], 1),
        ([Vector2::new(-150.0, 96.0), Vector2::new(150.0, 96.0)], 1),
        ([Vector2::new(-150.0, -96.0), Vector2::new(150.0, -96.0)], 1),
        ([Vector2::new(-96.0, -150.0), Vector2::new(-96.0, 150.0)], 1),
        ([Vector2::new(96.0, -150.0), Vector2::new(96.0, 150.0)], 1),
        ([Vector2::ZERO, Vector2::new(0.0, 96.0)], 1),
    ];
    let core = committed_with(512.0, &strokes, &[0, 1, 2, 3, 4, 5], |core| {
        core.slope_terrain_internal(
            Vector2::ZERO,
            256.0,
            Vector2::new(-128.0, 0.0),
            0.0,
            Vector2::new(128.0, 0.0),
            20.48,
            1.0,
        );
    });
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    assert_profiles_fresh("street grid", &core);
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

// A road drawn as two strokes that meet end to end, straight on (a pass-through node) and bending
// 34 degrees, in either order: the second stroke starts or ends at the first one's road end.
fn end_to_end_joins(sloped: bool) {
    for bend_deg in [0.0_f32, 34.0] {
        let strokes = [
            ([Vector2::new(-ARM_M, 0.0), Vector2::ZERO], 1),
            (
                [
                    Vector2::ZERO,
                    Vector2::from_angle(bend_deg.to_radians()) * ARM_M,
                ],
                1,
            ),
        ];
        assert_order_independent(
            &format!("end_to_end_{bend_deg}"),
            &strokes,
            &[&[0, 1], &[1, 0]],
            sloped,
        );
    }
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
    end_to_end_joins(false);
    splits_and_adjacent_junctions(false);
}

// Open question 16: a stroke reads no piece its own edit rebuilds, so the through road of a T
// drawn after its side road ignores the side road's cap and span, and the side road drawn
// second meets the through road at its graph profile. Open question 30 (a): a stroke from a
// road end joins it like any node, so every order of a Y or an end-to-end join solves its roads
// alike. Mid-span meetings follow the crossing rule instead, which
// `a_mid_span_meeting_lies_on_the_first_drawn_road` asserts for every order of the sloped cross
// and pair (open question 30: the two roads' profiles at the sloped cross's meeting differ by
// an ulp, so only re-heighting the first road, which edits an authored road, would make the
// orders equal).
#[test]
fn sloped_junctions_do_not_depend_on_drawing_order() {
    y_junctions(true);
    t_junctions(true);
    end_to_end_joins(true);
}

// The height a stroke meets at `at` on the graph it is drawn on: an existing node's graph
// height, else the graph profile of the live road passing nearest.
fn met_height(core: &SimCore, at: Vector3) -> f32 {
    let graph = &core.region_graph;
    let node = (0..graph.node_count() as u32)
        .map(|node| graph.get_valid_node(node))
        .filter(|&node| live_node(graph, node))
        .find(|&node| {
            let pos = graph.node(node).pos;
            (pos.x - at.x).hypot(pos.z - at.z) < 0.5
        });
    if let Some(node) = node {
        return graph.node(node).pos.y;
    }
    let distance = |edge: &crate::simulation::network::graph::Edge| {
        edge.physical_geometry
            .windows(2)
            .map(|pair| {
                let (a, b) = (pair[0], pair[1]);
                let (dx, dz) = (b.x - a.x, b.z - a.z);
                let t =
                    (((at.x - a.x) * dx + (at.z - a.z) * dz) / (dx * dx + dz * dz)).clamp(0.0, 1.0);
                (at.x - a.x - dx * t).hypot(at.z - a.z - dz * t)
            })
            .fold(f32::INFINITY, f32::min)
    };
    let edge = graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted)
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .expect("a road to meet");
    crate::simulation::network::graph::RegionGraph::profile_height_at(
        &edge.physical_geometry,
        at.x,
        at.z,
    )
}

fn live_node(graph: &crate::simulation::network::graph::RegionGraph, node: u32) -> bool {
    graph
        .node_adjacency(node)
        .iter()
        .any(|&edge| !graph.edge(edge).deleted)
}

// Open questions 16 and 30: a stroke meets a road it crosses or ends on mid-span at that road's
// graph profile, and a road end it passes over at that node's height, so the road drawn first
// sets the meeting's height and the road drawn second edits neither. The sloped cross and pair,
// in every order the flat test draws them: each meeting node lies exactly on what the
// first-drawn road had there before the second arrived.
#[test]
fn a_mid_span_meeting_lies_on_the_first_drawn_road() {
    let cross: &[Stroke] = &[
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::new(20.0, -ARM_M), Vector2::new(20.0, ARM_M)], 1),
    ];
    let pair: &[Stroke] = &[
        ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1),
        ([Vector2::new(-30.0, 0.0), Vector2::new(-30.0, ARM_M)], 1),
        ([Vector2::new(20.0, 0.0), Vector2::new(20.0, -ARM_M)], 1),
    ];
    // Each fixture's meetings: where, and the two strokes that meet there.
    type Case<'a> = (
        &'a str,
        &'a [Stroke],
        &'a [&'a [usize]],
        &'a [(Vector2, [usize; 2])],
    );
    let cases: [Case; 2] = [
        (
            "cross",
            cross,
            &[&[0, 1], &[1, 0]],
            &[(Vector2::new(20.0, 0.0), [0, 1])],
        ),
        (
            "two_junctions",
            pair,
            &[&[0, 1, 2], &[1, 0, 2], &[1, 2, 0]],
            &[
                (Vector2::new(-30.0, 0.0), [0, 1]),
                (Vector2::new(20.0, 0.0), [0, 2]),
            ],
        ),
    ];
    for (name, strokes, orders, meetings) in cases {
        for order in orders {
            let core = committed(strokes, order, true);
            let graph = &core.region_graph;
            for &(at, roads) in meetings {
                let node = (0..graph.node_count() as u32)
                    .map(|node| graph.get_valid_node(node))
                    .filter(|&node| live_node(graph, node))
                    .find(|&node| {
                        let pos = graph.node(node).pos;
                        (pos.x - at.x).hypot(pos.z - at.y) < 0.5
                    })
                    .unwrap_or_else(|| panic!("{name} {order:?}: no node at {at:?}"));
                let pos = graph.node(node).pos;
                // The graph just before the second of the two roads was drawn.
                let second = order
                    .iter()
                    .position(|index| roads.contains(index))
                    .map(|first| first + 1)
                    .and_then(|start| {
                        order[start..]
                            .iter()
                            .position(|index| roads.contains(index))
                            .map(|offset| start + offset)
                    })
                    .unwrap();
                let before = committed(strokes, &order[..second], true);
                let expected = met_height(&before, pos);
                assert_eq!(
                    pos.y.to_bits(),
                    expected.to_bits(),
                    "{name} {order:?} at {at:?}: the meeting at {} m is not on the first road at {expected} m",
                    pos.y
                );
            }
        }
    }
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
