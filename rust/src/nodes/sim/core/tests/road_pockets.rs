// SPDX-License-Identifier: GPL-2.0-only

//! The turn pocket edit (`ROAD-95` round R5a2) on a city: what an add, a resize, a role change
//! and a removal build, the preview being the commit at both stored ends, the refusals, the
//! cars and buildings on the pair, undo, the publication splice, the asymmetric corridor, the
//! strokes refused onto a pocket and a bulldozed pair's transition. Ported from Veeti's
//! road-chain snapshot `efd58129` (fork `ROAD-67` stages 3-5, `nodes/sim/road_pockets.rs`) and
//! adapted to the constructive roadbed and the in-place edit's transaction.

use super::road_crosswalks::{
    arm_towards, fully_emitted_chunks, live_arms, products, published_chunks,
};
use super::road_in_place::house_fronting;
use super::*;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::pockets::PocketRequest;
use crate::simulation::network::roadbed::EdgeEnd;

mod buildings;
mod cells;

// A four-way crossing at the origin of two 300 m roads drawn west to east and north to south
// (`west_to_east`; else east to west, so the west arm's stored ends swap). Returns the core and
// the crossing node.
fn crossing(west_to_east: bool) -> (SimCore, u32) {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    let (a, b) = if west_to_east {
        (-150.0, 150.0)
    } else {
        (150.0, -150.0)
    };
    for points in [
        vec![Vector3::new(a, 0.0, 0.0), Vector3::new(b, 0.0, 0.0)],
        vec![
            Vector3::new(0.0, 0.0, -150.0),
            Vector3::new(0.0, 0.0, 150.0),
        ],
    ] {
        assert!(core.add_road_internal(points, 1, 1).committed);
    }
    core.precompute_road_mesh_data();
    let node = (0..core.region_graph.node_count() as u32)
        .find(|&node| live_arms(&core, node).len() == 4)
        .expect("the crossing");
    (core, node)
}

// The arm of `node` towards `towards` and its end at `node`.
fn arm_end(core: &SimCore, node: u32, towards: Vector3) -> (usize, EdgeEnd) {
    let arm = arm_towards(core, node, towards);
    let end = if core
        .region_graph
        .get_valid_node(core.region_graph.edge(arm).end_node)
        == node
    {
        EdgeEnd::End
    } else {
        EdgeEnd::Start
    };
    (arm, end)
}

fn request(edge: usize, end: EdgeEnd, (left, right): (bool, bool), body_m: f32) -> PocketRequest {
    PocketRequest {
        edge,
        end,
        left,
        right,
        body_m,
    }
}

// The pocket edit's verdict and its commit: `""` when applied, else the refusal's reason. The
// preview must give the same verdict and change nothing, and a refusal must change nothing.
fn pocket_edit(core: &mut SimCore, request: PocketRequest) -> String {
    let before = (products(core), core.undo_stack.len());
    let counts = (
        core.region_graph.node_count(),
        core.region_graph.edge_count(),
    );
    let preview = core
        .preview_road_pocket_internal(request)
        .err()
        .map(|refusal| {
            eprintln!("pocket refusal {}: {}", refusal.reason, refusal.detail);
            refusal.reason
        })
        .unwrap_or_default();
    assert_eq!(products(core), before.0, "a preview changes nothing");
    assert_eq!(
        (
            core.region_graph.node_count(),
            core.region_graph.edge_count()
        ),
        counts
    );
    let applied = core.set_road_pocket_internal(request);
    assert_eq!(
        applied,
        preview.is_empty(),
        "the preview is the commit: {preview}"
    );
    if !applied {
        assert_eq!((products(core), core.undo_stack.len()), before, "{preview}");
    }
    preview
}

// The vehicle lanes of `edge` in the direction `is_fwd`, by lane index.
fn vehicle_lanes(core: &SimCore, edge: usize, is_fwd: bool) -> Vec<(i8, usize)> {
    let lanes = &core.transit_network.lane_system;
    let mut found: Vec<_> = lanes.edge_lanes[&edge]
        .iter()
        .copied()
        .filter(|&id| {
            let lane = &lanes.lanes[id];
            lane.lane_type == LaneType::Vehicle && lane.is_fwd == is_fwd
        })
        .map(|id| (lanes.lanes[id].lane_idx, id))
        .collect();
    found.sort_unstable();
    found
}

// Ported: `pocket_split_keeps_agents_supported_and_undo_restores_topology`. A right pocket on
// the west arm splits it at the transition 70.5 m from the crossing (60 m body plus the taper
// half), the pocket edge has three lanes towards the crossing and every approach lane a
// successor, a car on the base part keeps its lane and place, and undo restores the city: the
// products, the counts and the car.
#[test]
fn a_right_pocket_splits_the_road_and_undo_restores_it() {
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    let (mut core, node) = crossing(true);
    let before = products(&mut core);
    let counts = (
        core.region_graph.node_count(),
        core.region_graph.edge_count(),
    );
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    let is_fwd = end == EdgeEnd::End;
    // A car 100 m out on the west arm, in its lane towards the crossing.
    let (_, lane_id) = vehicle_lanes(&core, west, is_fwd)[0];
    let far = core.region_graph.edge(west).start_node;
    let was = sample_lane_pose(&core.transit_network.lane_system.lanes[lane_id], 50.0)
        .unwrap()
        .0;
    let car = core.agents.spawn_debug_car(
        vec![far, node],
        &core.transit_network.lane_system,
        lane_id,
        godot::prelude::Vector2::new(was.x, was.z),
    );
    core.agents.agents.lane_distance[car] = 50.0;

    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    assert_eq!(
        (
            core.region_graph.node_count(),
            core.region_graph.edge_count()
        ),
        (counts.0 + 1, counts.1 + 1)
    );
    let state = core
        .get_road_pocket_internal(west as i32, end)
        .expect("the pocket");
    assert_eq!((state.pocket.left, state.pocket.right), (false, true));
    assert!((state.body_m - 60.0).abs() < 1e-3, "{}", state.body_m);
    assert_eq!(state.junction, node);
    let pocket = core.region_graph.edge(state.edge);
    assert_eq!((pocket.fwd_lanes, pocket.bkw_lanes), (2, 1));
    let transition = pocket.start_node;
    assert_eq!(
        core.region_graph.node(transition).node_type,
        NodeType::LaneTransition
    );
    assert!((core.region_graph.node(transition).pos.x + 70.5).abs() < 1e-3);
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    for (_, id) in vehicle_lanes(&core, state.edge, true) {
        assert!(
            !core.transit_network.lane_system.lanes[id]
                .next_lanes
                .is_empty(),
            "every approach lane turns or goes on"
        );
    }
    let lane = &core.transit_network.lane_system.lanes[core.agents.agents.current_lane_id[car]];
    assert_eq!(lane.lane_type, LaneType::Vehicle);
    let pose = sample_lane_pose(lane, core.agents.agents.lane_distance[car])
        .unwrap()
        .0;
    assert!(
        pose.distance_to(was) < 1e-3,
        "the car moved {} m",
        pose.distance_to(was)
    );

    assert!(core.undo_action_internal());
    assert_eq!(
        (
            core.region_graph.node_count(),
            core.region_graph.edge_count()
        ),
        counts
    );
    assert_eq!(products(&mut core), before, "undo restores the city");
    let lane = &core.transit_network.lane_system.lanes[core.agents.agents.current_lane_id[car]];
    let pose = sample_lane_pose(lane, core.agents.agents.lane_distance[car])
        .unwrap()
        .0;
    assert!(pose.distance_to(was) < 1e-3);
}

// Ported and extended: `invalid_pocket_is_transactionally_refused`. Every refusal is the
// preview's and the commit's and changes nothing.
#[test]
fn pocket_edits_are_refused_with_a_reason() {
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    let right = (false, true);
    assert_eq!(
        pocket_edit(&mut core, request(usize::MAX >> 1, end, right, 60.0)),
        "invalid_road"
    );
    assert_eq!(
        pocket_edit(&mut core, request(west, end, right, 130.0)),
        "pocket_road_too_short"
    );
    assert_eq!(
        pocket_edit(&mut core, request(west, end, right, 65.0)),
        "invalid_pocket_dimensions"
    );
    assert_eq!(
        pocket_edit(&mut core, request(west, end, right, 10.0)),
        "invalid_pocket_dimensions"
    );
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, false), 60.0)),
        "pocket_unchanged"
    );
    // The far end of the west arm is a dead end: no turn to pocket.
    let far_end = if end == EdgeEnd::End {
        EdgeEnd::Start
    } else {
        EdgeEnd::End
    };
    assert_eq!(
        pocket_edit(&mut core, request(west, far_end, right, 60.0)),
        "pocket_movement_missing"
    );
    // A junction with turn rules (D9).
    let east = arm_towards(&core, node, Vector3::new(150.0, 0.0, 0.0));
    let from = if end == EdgeEnd::End { 0 } else { -1 };
    let to = if core.region_graph.edge(east).start_node == node {
        0
    } else {
        -1
    };
    core.set_lane_connection_internal(node, west as i32, from, east as i32, to);
    assert_eq!(
        pocket_edit(&mut core, request(west, end, right, 60.0)),
        "pocket_junction_whitelisted"
    );
    core.clear_lane_connections_internal(node);
    // A road with a median: a right pocket's laid-out transition carries it through (round
    // R5b1a); a left pocket needs a raised median of at least 3.8 m (round R5b1, D3).
    let grass = crate::simulation::network::graph::RoadMedian {
        style: crate::simulation::network::graph::MedianStyle::KerbedGrass,
        width_m: 3.0,
    };
    let attributes = crate::simulation::network::road_types::RoadAttributes::of_edge(
        core.region_graph.edge(west),
    );
    assert!(core.set_edge_cross_section_internal(west as i32, 1, 1, Some(grass), attributes));
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (true, false), 60.0)),
        "pocket_requires_wide_median"
    );
    assert!(core.preview_road_pocket_internal(request(west, end, right, 60.0)).is_ok());
}

// Arm `west` of `crossing` made a `lanes` + `lanes` road on a `median_m` kerbed grass median.
fn divide_arm(core: &mut SimCore, west: usize, lanes: u8, median_m: f32) {
    let grass = crate::simulation::network::graph::RoadMedian {
        style: crate::simulation::network::graph::MedianStyle::KerbedGrass,
        width_m: median_m,
    };
    let attributes = crate::simulation::network::road_types::RoadAttributes::of_edge(
        core.region_graph.edge(west),
    );
    assert!(core.set_edge_cross_section_internal(
        west as i32,
        lanes,
        lanes,
        Some(grass),
        attributes
    ));
}

// Round R5b1 (D3): a left pocket on a divided 2+2 arm through the edit, at both stored ends. The
// pocket edge keeps the footprint and narrows the 5 m median to 1.5 m; the transition passes
// through (no node piece) and the median continues there without a nose; preview is commit for
// the add, a resize and the removal, which gives the base road its median back; undo restores
// the pocket. A 3.8 m median takes one with a 0.3 m stub, and a body too short for the
// narrowing and the junction's nose is refused.
#[test]
fn a_left_pocket_on_a_divided_road_narrows_its_median_through_the_edit() {
    for west_to_east in [true, false] {
        let (mut core, node) = crossing(west_to_east);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        divide_arm(&mut core, west, 2, 5.0);
        let base = core.region_graph.edge(west).clone();
        let left = (true, false);
        assert_eq!(pocket_edit(&mut core, request(west, end, left, 60.0)), "");
        let state = core.get_road_pocket_internal(west as i32, end).unwrap();
        let pocket = core.region_graph.edge(state.edge).clone();
        assert_eq!(pocket.median.unwrap().width_m, 1.5);
        assert_eq!(pocket.width, base.width, "the footprint stays");
        assert_eq!(pocket.fwd_lanes + pocket.bkw_lanes, 5);
        let transition = if end == EdgeEnd::End {
            pocket.start_node
        } else {
            pocket.end_node
        };
        assert_eq!(
            core.region_graph.node(transition).node_type,
            NodeType::LaneTransition
        );
        core.precompute_road_mesh_data();
        assert!(
            core.transit_network
                .roadbed
                .node_piece(transition)
                .is_none(),
            "a pass-through"
        );
        // The median continues through: the pocket span's row there is a divided body row.
        let span = core.transit_network.roadbed.span_piece(state.edge).unwrap();
        let row = if end == EdgeEnd::End {
            span.start_mouth()
        } else {
            span.end_mouth()
        };
        assert_eq!(row.profile.bands().len(), 9, "no nose at the transition");
        assert_eq!(pocket_edit(&mut core, request(west, end, left, 90.0)), "");
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, false), 90.0)),
            ""
        );
        let joined = core.region_graph.edge(west);
        assert_eq!(
            (
                joined.fwd_lanes,
                joined.bkw_lanes,
                joined.median,
                joined.width
            ),
            (base.fwd_lanes, base.bkw_lanes, base.median, base.width)
        );
        assert!(core.undo_action_internal());
        let restored = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert!(restored.pocket.left && !restored.pocket.right);
        assert!((restored.body_m - 90.0).abs() < 1e-3);
    }
    // The widest grass median (12 m, an 8.5 m remainder) at a crossing of a 4+4 road on 12 m: the
    // shortest body leaves no room for the narrowing before the junction's nose; 40 m does.
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    for towards in [
        Vector3::new(0.0, 0.0, -150.0),
        Vector3::new(0.0, 0.0, 150.0),
    ] {
        let (arm, _) = arm_end(&core, node, towards);
        divide_arm(&mut core, arm, 4, 12.0);
    }
    divide_arm(&mut core, west, 2, 12.0);
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (true, false), 20.0)),
        "pocket_road_too_short"
    );
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (true, false), 40.0)),
        ""
    );
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    divide_arm(&mut core, west, 2, 3.8);
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (true, false), 60.0)),
        ""
    );
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    let stub = core.region_graph.edge(state.edge).median.unwrap().width_m;
    assert!((stub - 0.3).abs() < 1e-6, "{stub}");
}

// Round R5b1a: a right pocket on a divided 2+2 arm on 5 m grass through the edit, at both stored
// ends, then a left pocket added beside it. The transition is laid out and its piece carries the
// median: both mouths are divided body rows, which the spans start from. Preview is commit for
// the add, the resize, the role change and the removal, which gives the base road back; undo
// restores the pair. On a painted median a right pocket is accepted too (its hatching ends either
// side of the transition's piece).
#[test]
fn a_right_pocket_on_a_divided_road_carries_the_median_through_its_transition() {
    let body_rows = |core: &SimCore, transition: u32| {
        let piece = core.transit_network.roadbed.node_piece(transition).unwrap();
        piece
            .mouths()
            .iter()
            .all(|(_, mouth)| mouth.profile.bands().len() == 9)
    };
    for west_to_east in [true, false] {
        let (mut core, node) = crossing(west_to_east);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        divide_arm(&mut core, west, 2, 5.0);
        let base = core.region_graph.edge(west).clone();
        assert_eq!(pocket_edit(&mut core, request(west, end, (false, true), 60.0)), "");
        let state = core.get_road_pocket_internal(west as i32, end).unwrap();
        let pocket = core.region_graph.edge(state.edge).clone();
        assert_eq!(pocket.median, base.median, "the median is kept");
        let transition = if end == EdgeEnd::End {
            pocket.start_node
        } else {
            pocket.end_node
        };
        core.precompute_road_mesh_data();
        assert!(body_rows(&core, transition), "the median runs through");
        assert_eq!(pocket_edit(&mut core, request(west, end, (false, true), 90.0)), "");
        core.precompute_road_mesh_data();
        assert!(body_rows(&core, transition));
        // A left pocket beside it: the narrowing starts at the transition's mouth.
        assert_eq!(pocket_edit(&mut core, request(west, end, (true, true), 90.0)), "");
        let state = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert!(state.pocket.left && state.pocket.right);
        assert_eq!(core.region_graph.edge(state.edge).median.unwrap().width_m, 1.5);
        core.precompute_road_mesh_data();
        assert!(body_rows(&core, transition));
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, false), 90.0)),
            ""
        );
        let joined = core.region_graph.edge(west);
        assert_eq!(
            (joined.fwd_lanes, joined.bkw_lanes, joined.median, joined.width),
            (base.fwd_lanes, base.bkw_lanes, base.median, base.width)
        );
        assert!(core.undo_action_internal());
        let restored = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert!(restored.pocket.left && restored.pocket.right);
    }
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    let painted = crate::simulation::network::graph::RoadMedian {
        style: crate::simulation::network::graph::MedianStyle::Painted,
        width_m: 2.0,
    };
    let attributes = crate::simulation::network::road_types::RoadAttributes::of_edge(
        core.region_graph.edge(west),
    );
    assert!(core.set_edge_cross_section_internal(west as i32, 2, 2, Some(painted), attributes));
    assert_eq!(pocket_edit(&mut core, request(west, end, (false, true), 60.0)), "");
}

// Ported: `pocket_roles_resize_remove_and_preview_cover_both_stored_directions`. At both stored
// ends of the west arm: an add, a resize that moves the transition and keeps the pair's ids, a
// role change on a one-way road, and a removal that joins the pair back into one road with its
// lanes; each step's preview is its commit, and undo of the removal restores the pocket.
#[test]
fn pocket_roles_resize_remove_and_preview_cover_both_stored_directions() {
    for west_to_east in [true, false] {
        let (mut core, node) = crossing(west_to_east);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        assert_eq!(
            end,
            if west_to_east {
                EdgeEnd::End
            } else {
                EdgeEnd::Start
            }
        );
        // One-way towards the crossing, so a left kerb pocket is legal too (D15).
        let attributes = crate::simulation::network::road_types::RoadAttributes::of_edge(
            core.region_graph.edge(west),
        );
        let (fwd, bkw) = if end == EdgeEnd::End { (1, 0) } else { (0, 1) };
        assert!(core.set_edge_cross_section_internal(west as i32, fwd, bkw, None, attributes));
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
            ""
        );
        let counts = (
            core.region_graph.node_count(),
            core.region_graph.edge_count(),
        );
        let state = core.get_road_pocket_internal(west as i32, end).unwrap();
        let transition = core.region_graph.edge(state.edge);
        let transition = if end == EdgeEnd::End {
            transition.start_node
        } else {
            transition.end_node
        };
        let old_pos = core.region_graph.node(transition).pos;
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, true), 90.0)),
            ""
        );
        let resized = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert_eq!(resized.edge, state.edge, "a resize keeps the pair's ids");
        assert!((resized.body_m - 90.0).abs() < 1e-3);
        assert!((core.region_graph.node(transition).pos.distance_to(old_pos) - 30.0).abs() < 1e-3);
        assert_eq!(
            (
                core.region_graph.node_count(),
                core.region_graph.edge_count()
            ),
            counts,
            "the transition moves, no node or edge is added"
        );
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (true, true), 90.0)),
            ""
        );
        let pocket = core.region_graph.edge(state.edge);
        assert_eq!(pocket.fwd_lanes + pocket.bkw_lanes, 3);
        // The base road's end at the transition edits the same pocket.
        let base = core
            .region_graph
            .node_adjacency(transition)
            .iter()
            .copied()
            .find(|&edge| edge != state.edge)
            .unwrap();
        let base_end = if core.region_graph.edge(base).end_node == transition {
            EdgeEnd::End
        } else {
            EdgeEnd::Start
        };
        assert_eq!(
            core.get_road_pocket_internal(base as i32, base_end)
                .map(|s| s.edge),
            Some(state.edge)
        );
        assert_eq!(
            pocket_edit(&mut core, request(base, base_end, (false, false), 90.0)),
            ""
        );
        let joined = core.region_graph.edge(west);
        assert_eq!((joined.fwd_lanes, joined.bkw_lanes), (fwd, bkw));
        assert!(joined.pocket.is_none());
        assert!(!core.region_graph.node_has_live_incident_edge(transition));
        assert_eq!(
            core.region_graph.node(transition).node_type,
            NodeType::Junction
        );
        assert!(core.get_road_pocket_internal(west as i32, end).is_none());
        assert!(core.undo_action_internal());
        let restored = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert_eq!((restored.pocket.left, restored.pocket.right), (true, true));
        assert_eq!(
            core.region_graph.node(transition).node_type,
            NodeType::LaneTransition
        );
    }
}

// Ported: `one_way_right_pocket_retains_the_through_lane_alignment`. On a one-way road the
// through lane keeps the centreline on the pocket edge; the pocket lane lies one lane out.
#[test]
fn one_way_right_pocket_retains_the_through_lane_alignment() {
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    let attributes = crate::simulation::network::road_types::RoadAttributes::of_edge(
        core.region_graph.edge(west),
    );
    assert!(core.set_edge_cross_section_internal(west as i32, 1, 0, None, attributes));
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    let lanes = vehicle_lanes(&core, state.edge, true);
    assert_eq!(lanes.len(), 2);
    let lane = |id: usize| &core.transit_network.lane_system.lanes[id];
    assert!(lane(lanes[0].1).geometry.iter().all(|p| p.z.abs() < 1e-3));
    assert!(
        lane(lanes[1].1)
            .geometry
            .iter()
            .all(|p| (p.z - 3.5).abs() < 1e-3)
    );
}

// Ported: `opposing_end_pockets_keep_the_middle_road_and_remove_independently`. A 300 m road
// between two crossings takes a pocket at each end; the middle road stays a plain road, and
// removing one pocket leaves the other.
#[test]
fn opposing_end_pockets_keep_the_middle_road_and_remove_independently() {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    for points in [
        [(-150.0, 0.0), (150.0, 0.0)],
        [(-150.0, -100.0), (-150.0, 100.0)],
        [(150.0, -100.0), (150.0, 100.0)],
    ] {
        let points = points.map(|(x, z)| Vector3::new(x, 0.0, z)).to_vec();
        assert!(core.add_road_internal(points, 1, 1).committed);
    }
    core.precompute_road_mesh_data();
    let west_node = core
        .region_graph
        .find_node_within(Vector3::new(-150.0, 0.0, 0.0), 0.5)
        .unwrap();
    let east_node = core
        .region_graph
        .find_node_within(Vector3::new(150.0, 0.0, 0.0), 0.5)
        .unwrap();
    let (road, east_end) = arm_end(&core, east_node, Vector3::ZERO);
    let west_end = if east_end == EdgeEnd::End {
        EdgeEnd::Start
    } else {
        EdgeEnd::End
    };
    assert_eq!(
        pocket_edit(&mut core, request(road, east_end, (false, true), 60.0)),
        ""
    );
    let east = core
        .get_road_pocket_internal(road as i32, east_end)
        .unwrap();
    let (middle, west_end_now) = arm_end(&core, west_node, Vector3::ZERO);
    assert_eq!(west_end_now, west_end);
    assert_eq!(
        pocket_edit(&mut core, request(middle, west_end, (false, true), 60.0)),
        ""
    );
    assert_eq!(
        core.get_road_pocket_internal(east.edge as i32, east_end),
        Some(east)
    );
    let west = core
        .get_road_pocket_internal(middle as i32, west_end)
        .unwrap();
    assert_ne!(east.edge, west.edge);
    assert_eq!(
        core.region_graph
            .edges()
            .iter()
            .filter(|edge| !edge.deleted && edge.pocket.is_some())
            .count(),
        2
    );
    assert_eq!(
        pocket_edit(
            &mut core,
            request(west.edge, west_end, (false, false), 60.0)
        ),
        ""
    );
    assert!(
        core.get_road_pocket_internal(middle as i32, west_end)
            .is_none()
    );
    let east_again = core
        .get_road_pocket_internal(east.edge as i32, east_end)
        .unwrap();
    assert_eq!(east_again, east);
}

// The pair's buildings re-partition with the transition and return on undo (the join's
// inverse, D8): a house fronting the base part keeps its road through an add, moves onto the
// pocket edge when a resize lengthens the pocket past it, and every step's undo puts it back.
#[test]
fn buildings_on_the_pair_repartition_on_resize_and_return_on_undo() {
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    // A house 90 m out on the far side (north of the west arm), past a 60 m pocket's transition
    // and inside a 100 m one.
    let (house, _) = house_fronting(&mut core, west, (-90.0, -12.0));
    // Its column along the arm from the arm's start at x = -150, which the split partitions.
    core.allocator.buildings[house].cell_x = (60.0 / core.config.zone_cell_m).floor() as usize;
    let row = |core: &SimCore| {
        let b = &core.allocator.buildings[house];
        (b.edge_idx, b.cell_x, b.frontage_t.to_bits())
    };
    let original = row(&core);
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    assert_eq!(
        core.allocator.buildings[house].edge_idx, west,
        "beyond the transition"
    );
    let added = row(&core);
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 100.0)),
        ""
    );
    assert_eq!(
        core.allocator.buildings[house].edge_idx, state.edge,
        "the longer pocket reaches it"
    );
    assert!(core.undo_action_internal());
    assert_eq!(row(&core), added);
    assert!(core.undo_action_internal());
    assert_eq!(row(&core), original);
}

// D10: a stroke crossing a pocket edge, or ending on its transition, is refused; one crossing
// the base road beyond the transition commits.
#[test]
fn strokes_onto_a_pocket_are_refused() {
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    let counts = (
        core.region_graph.node_count(),
        core.region_graph.edge_count(),
    );
    for points in [
        [(-40.0, -60.0), (-40.0, 60.0)],
        [(-70.5, 0.0), (-70.5, 60.0)],
    ] {
        let points = points.map(|(x, z)| Vector3::new(x, 0.0, z)).to_vec();
        core.last_road_rejection = None;
        assert!(!core.add_road_internal(points, 1, 1).committed);
        assert_eq!(
            core.last_road_rejection
                .as_ref()
                .map(|refusal| refusal.reason.as_str()),
            Some("stroke_splits_pocket")
        );
        assert_eq!(
            (
                core.region_graph.node_count(),
                core.region_graph.edge_count()
            ),
            counts
        );
    }
    let points = vec![
        Vector3::new(-120.0, 0.0, -60.0),
        Vector3::new(-120.0, 0.0, 60.0),
    ];
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    assert!(core.add_road_internal(points, 1, 1).committed);
    assert_eq!(
        core.get_road_pocket_internal(state.edge as i32, end),
        Some(state)
    );
}

// D10: a stroke from a pocket road's far end commits, and the pockets stay. Its excerpt reaches
// the crossing's other pocket edges but not their base roads, so their transitions hold one arm
// there and gain none (the user's save, 2026-11-05).
#[test]
fn strokes_from_a_pocket_roads_far_end_commit() {
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    let (east, east_end) = arm_end(&core, node, Vector3::new(150.0, 0.0, 0.0));
    assert_eq!(
        pocket_edit(&mut core, request(east, east_end, (false, true), 60.0)),
        ""
    );
    let east_state = core.get_road_pocket_internal(east as i32, east_end).unwrap();
    let points = vec![
        Vector3::new(-150.0, 0.0, 0.0),
        Vector3::new(-150.0, 0.0, 100.0),
    ];
    core.last_road_rejection = None;
    let committed = core.add_road_internal(points, 1, 1).committed;
    assert_eq!(
        core.last_road_rejection
            .as_ref()
            .map(|refusal| refusal.reason.as_str()),
        None
    );
    assert!(committed);
    assert_eq!(
        core.get_road_pocket_internal(state.edge as i32, end),
        Some(state)
    );
    assert_eq!(
        core.get_road_pocket_internal(east_state.edge as i32, east_end),
        Some(east_state)
    );
}

// Beside a left turn pocket the centre line is a no-crossing line, solid white over the pocket
// edge (the user, 2026-11-05; every longitudinal line is white since the 2020 Road Traffic Act):
// no car crosses between the oncoming lane and the pocket lane. A right pocket on a 1 + 1 road
// keeps the dashed centre line.
#[test]
fn a_left_pockets_centre_line_is_solid() {
    use crate::simulation::network::render::road::RoadRenderer;
    const SOLID: f32 = 2.0;
    // The pocket edge runs from its transition (x = -70.5) to the crossing. A left pocket's
    // centre line lies between the oncoming lane (z = -5.25) and the pocket lane (z = -1.75), a
    // right pocket's between the oncoming lane (z = -1.75) and the through lane (z = 1.75).
    for ((left, right), centre_z, solid) in
        [((true, false), -3.5, true), ((false, true), 0.0, false)]
    {
        let (mut core, node) = crossing(true);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (left, right), 60.0)),
            ""
        );
        core.precompute_road_mesh_data();
        let mesh = RoadRenderer.generate_mesh_data_with_surface(
            &core.region_graph,
            &core.heightmap,
            &core.transit_network.roadbed,
        );
        let patterns: Vec<f32> = mesh
            .marking_vertices
            .chunks(3)
            .zip(mesh.marking_uvs.chunks(3))
            .filter_map(|(vertices, uvs)| {
                let centroid = vertices.iter().fold(Vector3::ZERO, |sum, &v| sum + v) / 3.0;
                ((-55.0..-20.0).contains(&centroid.x) && (centroid.z - centre_z).abs() < 0.3)
                    .then_some(uvs[0].y)
            })
            .collect();
        assert!(!patterns.is_empty(), "the pocket edge has a centre line");
        assert!(
            patterns.iter().all(|&pattern| (pattern == SOLID) == solid),
            "left {left}: {patterns:?}"
        );
    }
}

// A turn pocket's lanes carry a second arrow set where they begin, 8 m past the transition's
// piece, besides the one 12 m before the junction, when the pocket's marking range holds both
// (the user, 2026-11-05: double arrows on pockets). The arrows' stations on the right pocket lane
// (z = 5.25) of a 60 m and a shorter pocket.
#[test]
fn a_long_pocket_lane_carries_a_second_arrow_where_it_begins() {
    use crate::simulation::network::render::road::RoadRenderer;
    const SOLID: f32 = 2.0;
    for (body_m, sets) in [(60.0, 2), (40.0, 2), (30.0, 1)] {
        let (mut core, node) = crossing(true);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, true), body_m)),
            ""
        );
        core.precompute_road_mesh_data();
        let mesh = RoadRenderer.generate_mesh_data_with_surface(
            &core.region_graph,
            &core.heightmap,
            &core.transit_network.roadbed,
        );
        let mut stations: Vec<f32> = mesh
            .marking_vertices
            .chunks(3)
            .zip(mesh.marking_uvs.chunks(3))
            .filter_map(|(vertices, uvs)| {
                let centroid = vertices.iter().fold(Vector3::ZERO, |sum, &v| sum + v) / 3.0;
                (uvs[0].y == SOLID && (centroid.z - 5.25).abs() < 1.2 && centroid.x < 0.0)
                    .then_some(centroid.x)
            })
            .collect();
        stations.sort_by(f32::total_cmp);
        // A glyph is 5 m long: a gap over 3 m between triangle centroids starts the next set.
        let found = stations.windows(2).filter(|pair| pair[1] - pair[0] > 3.0).count()
            + usize::from(!stations.is_empty());
        assert_eq!(found, sets, "{body_m} m pocket: arrow triangles at {stations:?}");
    }
}

// D10: bulldozing the base road leaves the transition a road end, typed `Junction`, and the
// pocket edge a short wide road whose lanes take the base rule.
#[test]
fn bulldozing_a_pocket_pair_resets_its_transition() {
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    let transition = core.region_graph.edge(state.edge).start_node;
    assert!(core.bulldoze_road_edge_for_test(west));
    assert_eq!(
        core.region_graph.node(transition).node_type,
        NodeType::Junction
    );
    core.precompute_road_mesh_data();
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    assert!(core.undo_action_internal());
    assert_eq!(
        core.region_graph.node(transition).node_type,
        NodeType::LaneTransition
    );
}

// D17: the pocket's corridor is its carriageway's, read on its own axis: a parcel 2 m beyond the
// base road's sidewalk on the pocket side is refused, one as close on the far side is not.
#[test]
fn the_pocket_corridor_is_asymmetric() {
    for (z, expected) in [(10.5, "parcel_overlap"), (-10.5, "")] {
        let (mut core, node) = crossing(true);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        let residential = core
            .zoning
            .profiles
            .default_runtime_id_for_zone_type(ZoneType::Residential)
            .unwrap();
        core.zoning
            .place_or_rezone_default_parcel_at(-40.0, z, residential, &core.region_graph)
            .expect("a parcel fronting the west arm");
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
            expected,
            "parcel at z {z}"
        );
    }
}

// Round R5c1: a left pocket on the undivided 1+1 crossing's west arm through the edit, at both
// stored ends. The pocket edge has two lanes towards the crossing and its kerb shift on the
// oncoming side; the transition is laid out (a node piece) and no node fails; it costs what a
// right pocket of the same length costs (D14). Preview is commit for the add, a resize, a right
// pocket added beside it and the removal, which gives the base road back; undo restores the
// pair. The corridor is checked on the widened side (D17): a parcel on the left of the approach
// refuses a left pocket, one on the right does not. A junction with turn rules still refuses.
#[test]
fn a_left_pocket_on_an_undivided_road_widens_its_oncoming_side_through_the_edit() {
    for west_to_east in [true, false] {
        let (mut core, node) = crossing(west_to_east);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        let base = core.region_graph.edge(west).clone();
        let left = (true, false);
        let cost = |core: &mut SimCore, roles| {
            core.preview_road_pocket_internal(request(west, end, roles, 60.0))
                .unwrap()
                .cost_delta
        };
        let left_cost = cost(&mut core, left);
        assert!(left_cost > 0.0);
        assert_eq!(left_cost, cost(&mut core, (false, true)));
        assert_eq!(pocket_edit(&mut core, request(west, end, left, 60.0)), "");
        let state = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert_eq!((state.pocket.left, state.pocket.right), (true, false));
        let pocket = core.region_graph.edge(state.edge).clone();
        let is_fwd = end == EdgeEnd::End;
        assert_eq!(
            (pocket.fwd_lanes, pocket.bkw_lanes),
            if is_fwd { (2, 1) } else { (1, 2) }
        );
        assert_eq!(pocket.width, base.width + 3.5);
        // Towards the oncoming side: -z, whichever way the arm is stored.
        let shift = if is_fwd { -1.75 } else { 1.75 };
        assert_eq!(pocket.kerb_shift_m(), shift);
        let transition = if is_fwd {
            pocket.start_node
        } else {
            pocket.end_node
        };
        assert_eq!(
            core.region_graph.node(transition).node_type,
            NodeType::LaneTransition
        );
        core.precompute_road_mesh_data();
        assert!(core.transit_network.roadbed.failed_nodes().is_empty());
        assert!(
            core.transit_network
                .roadbed
                .node_piece(transition)
                .is_some(),
            "a laid-out transition"
        );
        // The pocket lane lies beside the centre line (z = -1.75), the oncoming lane a lane out.
        let lanes = &core.transit_network.lane_system;
        let mid_z = |id: usize| {
            let geometry = &lanes.lanes[id].geometry;
            geometry[geometry.len() / 2].z
        };
        let towards = vehicle_lanes(&core, state.edge, is_fwd);
        let away = vehicle_lanes(&core, state.edge, !is_fwd);
        let pocket_lane = if is_fwd { towards[0].1 } else { towards[1].1 };
        assert!((mid_z(pocket_lane) + 1.75).abs() < 1e-3);
        assert!((mid_z(away[0].1) + 5.25).abs() < 1e-3);
        assert_eq!(pocket_edit(&mut core, request(west, end, left, 90.0)), "");
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (true, true), 90.0)),
            ""
        );
        let both = core.region_graph.edge(state.edge);
        assert_eq!(both.fwd_lanes + both.bkw_lanes, 4);
        assert_eq!(both.kerb_shift_m(), 0.0);
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, false), 90.0)),
            ""
        );
        let joined = core.region_graph.edge(west);
        assert_eq!(
            (joined.fwd_lanes, joined.bkw_lanes, joined.width),
            (base.fwd_lanes, base.bkw_lanes, base.width)
        );
        assert!(joined.pocket.is_none());
        assert!(core.undo_action_internal());
        let restored = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert_eq!((restored.pocket.left, restored.pocket.right), (true, true));
        assert!(core.undo_action_internal());
        let restored = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert_eq!((restored.pocket.left, restored.pocket.right), (true, false));
        assert!((restored.body_m - 90.0).abs() < 1e-3);
    }
    for (z, expected) in [(-10.5, "parcel_overlap"), (10.5, "")] {
        let (mut core, node) = crossing(true);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        let residential = core
            .zoning
            .profiles
            .default_runtime_id_for_zone_type(ZoneType::Residential)
            .unwrap();
        core.zoning
            .place_or_rezone_default_parcel_at(-40.0, z, residential, &core.region_graph)
            .expect("a parcel fronting the west arm");
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (true, false), 60.0)),
            expected,
            "parcel at z {z}"
        );
    }
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    let east = arm_towards(&core, node, Vector3::new(150.0, 0.0, 0.0));
    core.set_lane_connection_internal(node, west as i32, 0, east as i32, 0);
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (true, false), 60.0)),
        "pocket_junction_whitelisted"
    );
}

// The edit's publication splices the rebuilt chunks: what Godot is sent equals a full emission
// after the add, a resize and each undo.
#[test]
fn a_pocket_edit_publishes_the_chunks_a_full_emission_draws() {
    let (mut core, node) = crossing(true);
    core.benchmark_mode = false;
    core.finalize_network_render_products();
    let drawn = published_chunks(&core);
    assert_eq!(fully_emitted_chunks(&mut core), drawn);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    let mut states = vec![drawn];
    for body in [60.0, 90.0] {
        assert!(core.set_road_pocket_internal(request(west, end, (false, true), body)));
        core.finalize_network_render_products();
        let edited = published_chunks(&core);
        assert_ne!(&edited, states.last().unwrap());
        assert_eq!(fully_emitted_chunks(&mut core), edited);
        states.push(edited);
    }
    states.pop();
    while let Some(state) = states.pop() {
        assert!(core.undo_action_internal());
        core.finalize_network_render_products();
        assert_eq!(published_chunks(&core), state);
        assert_eq!(fully_emitted_chunks(&mut core), state);
    }
}

// Ported: `moving_a_pocket_and_undo_preserve_a_cars_world_station`. A car on the pocket edge's
// through lane keeps its place when a resize moves the transition behind it, and again on undo.
#[test]
fn moving_a_pocket_and_undo_preserve_a_cars_world_station() {
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    let (_, lane_id) = vehicle_lanes(&core, state.edge, true)[0];
    let transition = core.region_graph.edge(state.edge).start_node;
    let was = sample_lane_pose(&core.transit_network.lane_system.lanes[lane_id], 30.0)
        .unwrap()
        .0;
    let car = core.agents.spawn_debug_car(
        vec![transition, node],
        &core.transit_network.lane_system,
        lane_id,
        godot::prelude::Vector2::new(was.x, was.z),
    );
    core.agents.agents.lane_distance[car] = 30.0;
    let pose = |core: &SimCore| {
        let lane = &core.transit_network.lane_system.lanes[core.agents.agents.current_lane_id[car]];
        assert_eq!(lane.lane_type, LaneType::Vehicle);
        sample_lane_pose(lane, core.agents.agents.lane_distance[car])
            .unwrap()
            .0
    };
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 90.0)),
        ""
    );
    assert!(
        pose(&core).distance_to(was) < 1e-3,
        "moved {} m",
        pose(&core).distance_to(was)
    );
    assert!(core.undo_action_internal());
    assert!(
        pose(&core).distance_to(was) < 1e-3,
        "moved {} m",
        pose(&core).distance_to(was)
    );
}

// Rounds R5b2 and R5c2: what a Junction window row reads for a road end (`PocketEndView`): the
// same junction and the whole road from it through the base road's transition, from either
// piece; the roles the road can carry: left on a road without a median (round R5c1) or beside a
// raised median of at least 3.8 m, right beside a median too (round R5b1a: its transition carries
// the median through).
#[test]
fn a_pocket_end_view_reads_the_road_from_its_junction() {
    use crate::simulation::network::pockets::PocketEndView;
    let (mut core, node) = crossing(true);
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    let graph = &core.region_graph;
    let view = PocketEndView::of(graph, west, end).unwrap();
    assert_eq!(view.junction, node);
    assert_eq!(
        (view.left, view.right),
        (Ok(()), Ok(()))
    );
    assert_eq!(
        pocket_edit(&mut core, request(west, end, (false, true), 60.0)),
        ""
    );
    let state = core.get_road_pocket_internal(west as i32, end).unwrap();
    let graph = &core.region_graph;
    let pocket = graph.edge(state.edge);
    let transition = match state.pocket.end {
        EdgeEnd::End => pocket.start_node,
        EdgeEnd::Start => pocket.end_node,
    };
    let base = graph
        .node_adjacency(transition)
        .iter()
        .copied()
        .find(|&edge| edge != state.edge);
    let base = base.expect("the base road");
    // The base road's end at the transition, towards the junction.
    let base_end = if graph.get_valid_node(graph.edge(base).end_node) == transition {
        EdgeEnd::End
    } else {
        EdgeEnd::Start
    };
    let view = PocketEndView::of(graph, base, base_end).unwrap();
    assert_eq!(view.junction, node);
    let (first, last) = (view.road[0], *view.road.last().unwrap());
    assert!(first.x.abs() < 1e-3 && first.z.abs() < 1e-3, "{first:?}");
    assert!(
        (last.x + 150.0).abs() < 1e-3 && last.z.abs() < 1e-3,
        "{last:?}"
    );
    assert!(
        view.width_m > graph.edge(base).width,
        "the pocket edge is wider"
    );
    assert_eq!(view.right, Ok(()));
    for (median_m, left) in [(5.0, Ok(())), (3.0, Err("pocket_requires_wide_median"))] {
        let (mut core, node) = crossing(true);
        let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
        divide_arm(&mut core, west, 2, median_m);
        let view = PocketEndView::of(&core.region_graph, west, end).unwrap();
        assert_eq!((view.left, view.right), (left, Ok(())));
    }
}

// Round R5c2: the Junction window's rows at a crossing. Its four arms clockwise from north,
// each at its stored end at the junction, named by its road without the pocket; a pocket edit
// splits and joins the arm, never the junction: the node keeps its id and its four rows (the
// west one now the pocket edge, at its pocket end) through the add, a resize, undo and the
// removal; the pocket's transition lists none; an outgoing one-way arm (no lane into the
// junction) has no row.
#[test]
fn the_junction_window_lists_each_car_arm_clockwise_from_north() {
    for west_to_east in [true, false] {
        let (mut core, node) = crossing(west_to_east);
        let rows = |core: &SimCore, node: u32| -> Vec<(usize, EdgeEnd, i32)> {
            core.junction_pocket_arms_internal(node)
                .iter()
                .map(|arm| {
                    assert_eq!(arm.view.junction, node);
                    let section = arm.base.0;
                    assert_eq!((section.fwd_lanes, section.bkw_lanes), (1, 1));
                    (arm.edge, arm.end, arm.bearing_deg.round() as i32)
                })
                .collect()
        };
        let towards = [
            Vector3::new(0.0, 0.0, -150.0),
            Vector3::new(150.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 150.0),
            Vector3::new(-150.0, 0.0, 0.0),
        ];
        let expected: Vec<_> = towards
            .iter()
            .zip([0, 90, 180, 270])
            .map(|(&point, bearing)| {
                let (edge, end) = arm_end(&core, node, point);
                (edge, end, bearing)
            })
            .collect();
        assert_eq!(rows(&core, node), expected);
        let (west, end) = arm_end(&core, node, towards[3]);
        for (body_m, roles) in [(60.0, (false, true)), (80.0, (false, true))] {
            assert_eq!(
                pocket_edit(&mut core, request(west, end, roles, body_m)),
                ""
            );
            let after = rows(&core, node);
            assert_eq!(after.len(), 4);
            assert_eq!(after[..3], expected[..3], "the other arms keep their rows");
            let state = core.get_road_pocket_internal(west as i32, end).unwrap();
            assert_eq!(
                (after[3].0, after[3].1, after[3].2),
                (state.edge, state.pocket.end, 270)
            );
            let pocket = core.region_graph.edge(state.edge);
            let transition = match state.pocket.end {
                EdgeEnd::End => pocket.start_node,
                EdgeEnd::Start => pocket.end_node,
            };
            assert!(core.junction_pocket_arms_internal(transition).is_empty());
        }
        assert!(core.undo_action_internal());
        let state = core.get_road_pocket_internal(west as i32, end).unwrap();
        assert!((state.body_m - 60.0).abs() < 1e-3);
        assert_eq!(rows(&core, node)[3].0, state.edge);
        assert_eq!(
            pocket_edit(&mut core, request(west, end, (false, false), 60.0)),
            ""
        );
        let (joined, joined_end) = arm_end(&core, node, towards[3]);
        assert_eq!(rows(&core, node)[3], (joined, joined_end, 270));
        assert!(core.undo_action_internal());
        assert_eq!(
            rows(&core, node).len(),
            4,
            "the node keeps its id through undo"
        );
    }
    // A one-way road through a two-way one: its leaving arm has no lane into the junction.
    let mut core = test_core();
    for (points, bkw) in [
        (
            [
                Vector3::new(-150.0, 0.0, 0.0),
                Vector3::new(150.0, 0.0, 0.0),
            ],
            0,
        ),
        (
            [
                Vector3::new(0.0, 0.0, -150.0),
                Vector3::new(0.0, 0.0, 150.0),
            ],
            1,
        ),
    ] {
        assert!(core.add_road_internal(points.to_vec(), 1, bkw).committed);
    }
    let node = (0..core.region_graph.node_count() as u32)
        .find(|&node| live_arms(&core, node).len() == 4)
        .expect("the crossing");
    let bearings: Vec<i32> = core
        .junction_pocket_arms_internal(node)
        .iter()
        .map(|arm| arm.bearing_deg.round() as i32)
        .collect();
    assert_eq!(bearings, [0, 180, 270]);
}

// Round R5b2 (found by `road_pocket_test.gd`): the road tool's replica follows every pocket edit
// and undo through the publications Godot's calls make (`finalize_network_render_products`);
// debug builds compare it with the live graph at each follow. Add, resize, undo, removal, as the
// Road Properties window runs them: the removal's join listed the junction's arms out of order
// ([1, 2, 3, 0]), which the replica (and a load) rebuild ascending.
#[test]
fn the_road_tool_replica_follows_pocket_edits_and_undo() {
    use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
    use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
    let (mut core, node) = crossing(true);
    core.benchmark_mode = false;
    let (west, end) = arm_end(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    core.refresh_road_terrain_inputs();
    let mut context =
        RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).expect("current"));
    let first = core.publish_road_tool_view().expect("current roadbed");
    assert!(context.apply(&first));
    let right = (false, true);
    let steps: [(&str, Option<PocketRequest>); 4] = [
        ("add", Some(request(west, end, right, 60.0))),
        ("resize", Some(request(west, end, right, 80.0))),
        ("undo", None),
        ("removal", Some(request(west, end, (false, false), 60.0))),
    ];
    for (step, edit) in steps {
        match edit {
            Some(edit) => assert!(core.set_road_pocket_internal(edit), "{step}"),
            None => assert!(core.undo_action_internal(), "{step}"),
        }
        let publication = core.finalize_network_render_products().expect(step);
        assert!(context.apply(&publication), "{step}");
    }
}
