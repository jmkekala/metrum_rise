// SPDX-License-Identifier: GPL-2.0-only

//! Ring plan tests (`ROAD-62`): orientation, sidewalk side, lanes at the arm nodes, splits and the
//! plan's refusals. The junction layouts and pieces over the size and arm matrix are
//! `roadbed::node::tests::rings`.

use super::*;
use crate::simulation::network::lanes::{LaneSystem, LaneType};
use std::collections::HashSet;
use crate::simulation::network::roadbed::fixtures::{
    bare_ring, connect, converted_ring, finalize, hub_graph, join_ring_arm, point, road_edge,
};
use crate::simulation::network::roadbed::{
    ArmKey, EdgeEnd, NodeArms, NodeWalkways, RoadbedSystem, SectorKind,
};
use crate::simulation::network::topology::split_edge_graph;

fn equal(arms: usize) -> Vec<f32> {
    (0..arms).map(|i| i as f32 * 360.0 / arms as f32).collect()
}

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    roadbed
}

// The ring edges through ring node `node`: (arriving, leaving).
fn ring_arms(graph: &RegionGraph, node: u32) -> (usize, usize) {
    let ring = |leaving: bool| {
        let arms = graph.node_adjacency(node).iter().copied();
        arms.filter(|&id| graph.edge(id).roundabout.is_some() && !graph.edge(id).deleted)
            .find(|&id| (graph.edge(id).start_node == node) == leaving)
            .expect("a ring node has a ring edge each way")
    };
    (ring(false), ring(true))
}

// Vehicle connectors at `node` as (from edge, from lane, to edge, to lane), sorted.
fn connectors(lanes: &LaneSystem, node: u32) -> Vec<(usize, i8, usize, i8)> {
    let mut source = vec![usize::MAX; lanes.lanes.len()];
    for (id, lane) in lanes.lanes.iter().enumerate() {
        if lane.edge_id != usize::MAX {
            for &next in &lane.next_lanes {
                source[next] = id;
            }
        }
    }
    let mut found = Vec::new();
    for (id, lane) in lanes.lanes.iter().enumerate() {
        if lane.edge_id == usize::MAX
            && lane.lane_type == LaneType::Vehicle
            && lane.node_id == node as usize
            && source[id] != usize::MAX
        {
            let from = &lanes.lanes[source[id]];
            let to = &lanes.lanes[lane.next_lanes[0]];
            found.push((from.edge_id, from.lane_idx, to.edge_id, to.lane_idx));
        }
    }
    found.sort_unstable();
    found
}

// Ported from Veeti's `roundabout_ring_is_counterclockwise_and_entries_yield_to_it`, without its
// control asserts: the yield is round R7c's.
#[test]
fn a_ring_runs_with_the_island_on_the_drivers_left_and_every_arm_enters_and_leaves() {
    for count in 3..=6 {
        let (mut graph, applied) = converted_ring(60.0, &equal(count), (1, 1), false).unwrap();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        assert_eq!(applied.ring.len(), count);
        for (i, &id) in applied.ring.iter().enumerate() {
            let edge = graph.edge(id);
            assert_eq!(edge.start_node, applied.nodes[i]);
            let (a, b) = (edge.geometry[0], edge.geometry[1]);
            assert!(
                a.x * b.z - a.z * b.x < 0.0,
                "ring edge {id} runs against the angle"
            );
            assert_eq!((edge.fwd_lanes, edge.bkw_lanes), (1, 0));
            assert_eq!(edge.speed_limit, RING_SPEED_MS);
            assert!(edge.no_building_spawn && !edge.no_street_lights);
            assert_eq!(edge.class, EdgeClass::Standard);
            assert_eq!(edge.roundabout.unwrap().radius_m, 28.25);
            assert_eq!(validate_ring_edge(&graph, id), Ok(()));
        }
        for &arm in &applied.arms {
            let edge = graph.edge(arm);
            let node = if applied.nodes.contains(&edge.start_node) {
                edge.start_node
            } else {
                edge.end_node
            };
            let (ring_in, ring_out) = ring_arms(&graph, node);
            let found = connectors(&lanes, node);
            let enters = found.iter().any(|c| c.0 == arm && c.2 == ring_out);
            let leaves = found.iter().any(|c| c.0 == ring_in && c.2 == arm);
            let circulates = found.iter().any(|c| c.0 == ring_in && c.2 == ring_out);
            assert!(enters && leaves && circulates, "node {node}: {found:?}");
        }
    }
}

// The exit is a right turn off the ring under right-hand traffic, so ring-in, arm, ring-out
// keep the arm on the outside of the circle at every node.
#[test]
fn the_ring_sidewalk_is_on_the_outer_side_and_the_island_has_no_walk() {
    for (diameter_m, arm) in [(36.0, (1, 1)), (50.0, (2, 2))] {
        let (mut graph, applied) = converted_ring(diameter_m, &equal(4), arm, false).unwrap();
        let radius_m = graph.edge(applied.ring[0]).roundabout.unwrap().radius_m;
        let lane_half_m = 0.5 * graph.edge(applied.ring[0]).width;
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        for &id in &applied.ring {
            assert_eq!(graph.edge(id).sidewalks, SidewalkSides::RIGHT);
            let foot: Vec<_> = lanes.edge_lanes[&id]
                .iter()
                .map(|&lane| &lanes.lanes[lane])
                .filter(|lane| lane.lane_type == LaneType::Foot)
                .collect();
            assert_eq!(foot.len(), 2, "one sidewalk, walked both ways");
            for lane in foot {
                for p in &lane.geometry {
                    let from_centre = p.x.hypot(p.z);
                    assert!(
                        from_centre > radius_m + lane_half_m,
                        "a foot lane {from_centre} m from the centre of a {radius_m} m ring"
                    );
                }
            }
        }
        let roadbed = compiled(&graph);
        assert!(
            roadbed.height_at(0.0, 0.0).is_none(),
            "the island is terrain"
        );
        for &node in &applied.nodes {
            let layout = NodeArms::collect(&graph, node).unwrap().layout().unwrap();
            let (ring_in, ring_out) = ring_arms(&graph, node);
            for arm in &layout.arms {
                let on_ring = arm.key.edge == ring_in || arm.key.edge == ring_out;
                assert_eq!(
                    arm.crosswalk.is_none(),
                    on_ring,
                    "node {node}: a zebra on the approach only, arm {:?}",
                    arm.key
                );
            }
            // The island sector is straight, the two approach corners are real.
            let straight = layout
                .sectors
                .iter()
                .filter(|s| s.kind == SectorKind::Straight);
            assert_eq!(straight.count(), 1);
            let walkways = NodeWalkways::collect(&graph, node).unwrap();
            assert_eq!(
                walkways.corners().count(),
                2,
                "a corner walk each side of the approach"
            );
            let keys = [
                (
                    ArmKey {
                        edge: ring_in,
                        end: EdgeEnd::End,
                    },
                    true,
                ),
                (
                    ArmKey {
                        edge: ring_out,
                        end: EdgeEnd::Start,
                    },
                    true,
                ),
            ];
            for (key, positive) in keys {
                assert!(walkways.sidewalk_end(key, positive).is_some(), "outer end");
                assert!(
                    walkways.sidewalk_end(key, !positive).is_none(),
                    "island end"
                );
            }
        }
    }
}

// Walkers go round on the outer sidewalk and cross the arms on their zebras: every sidewalk of the
// ring and of its arms is one walk.
#[test]
fn the_ring_sidewalk_is_one_walk_with_its_arms() {
    let (mut graph, applied) = converted_ring(36.0, &equal(4), (1, 1), false).unwrap();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    let foot = |id: usize| lanes.lanes[id].lane_type == LaneType::Foot;
    let start = lanes.edge_lanes[&applied.ring[0]]
        .iter()
        .copied()
        .find(|&id| foot(id))
        .unwrap();
    let mut seen = vec![false; lanes.lanes.len()];
    seen[start] = true;
    let mut open = vec![start];
    while let Some(id) = open.pop() {
        for &next in &lanes.lanes[id].next_lanes {
            if foot(next) && !seen[next] {
                seen[next] = true;
                open.push(next);
            }
        }
    }
    for &id in applied.ring.iter().chain(&applied.arms) {
        for &lane in &lanes.edge_lanes[&id] {
            assert!(
                !foot(lane) || seen[lane],
                "edge {id}: lane {lane} is cut off"
            );
        }
    }
}

#[test]
fn a_bare_ring_is_three_pass_through_nodes_without_a_node_piece() {
    for (diameter_m, lanes) in [
        (30.0, 1),
        (36.0, 1),
        (60.0, 1),
        (45.0, 2),
        (50.0, 2),
        (80.0, 2),
    ] {
        for sloped in [false, true] {
            let (graph, applied) = bare_ring(diameter_m, lanes, sloped);
            assert_eq!(applied.nodes.len(), 3);
            assert_eq!(applied.ring.len(), 3);
            assert!(applied.arms.is_empty());
            let roadbed = compiled(&graph);
            assert!(roadbed.failed_nodes().is_empty(), "{diameter_m}");
            for &node in &applied.nodes {
                assert!(NodeArms::collect(&graph, node).is_none(), "{diameter_m}");
                assert!(roadbed.node_piece(node).is_none(), "{diameter_m}");
            }
            for &id in &applied.ring {
                assert_eq!(graph.edge(id).fwd_lanes, lanes);
                assert_eq!(validate_ring_edge(&graph, id), Ok(()));
            }
            // A car rides round with no connector: each lane leads into the next ring edge's.
            let mut graph = graph;
            let mut system = LaneSystem::new();
            system.rebuild(&mut graph);
            for (i, &id) in applied.ring.iter().enumerate() {
                let next = applied.ring[(i + 2) % 3];
                for &lane in &system.edge_lanes[&id] {
                    let lane = &system.lanes[lane];
                    if lane.lane_type == LaneType::Vehicle {
                        let successors: Vec<_> = lane
                            .next_lanes
                            .iter()
                            .map(|&after| system.lanes[after].edge_id)
                            .collect();
                        assert_eq!(successors, vec![next], "D{diameter_m}");
                    }
                }
            }
        }
    }
}

// Each end of a ring edge is a straight lead along the tangent, on the circle at the node.
fn assert_leads(graph: &RegionGraph, id: usize) {
    let edge = graph.edge(id);
    let ring = edge.roundabout.expect("a ring edge");
    let n = edge.geometry.len();
    for (end, next) in [
        (edge.geometry[0], edge.geometry[1]),
        (edge.geometry[n - 1], edge.geometry[n - 2]),
    ] {
        let radial = Vector2::new(end.x, end.z) - ring.centre;
        assert!(
            (radial.length() - ring.radius_m).abs() < 0.02,
            "on the circle"
        );
        let lead = next - end;
        assert!(
            (lead.length() - LEAD_M).abs() < 1e-3,
            "a 1 m lead, not {}",
            lead.length()
        );
        let along = Vector2::new(lead.x, lead.z).normalized();
        assert!(
            along.dot(radial.normalized()).abs() < 2e-3,
            "along the tangent"
        );
    }
}

#[test]
fn a_split_keeps_the_ring_with_leads_at_the_new_node() {
    for (diameter_m, lanes) in [(36.0, 1), (50.0, 2), (60.0, 1)] {
        for sloped in [false, true] {
            let (mut graph, applied) = bare_ring(diameter_m, lanes, sloped);
            // Ring edge 1 runs from the node at 210 degrees to the one at 90.
            let before = graph.edge(applied.ring[1]).clone();
            let ring = before.roundabout;
            // Mid-chord, the join that failed without leads at 60 m.
            let (node, _) = join_ring_arm(&mut graph, 150.0, (1, 1), sloped);
            finalize(&mut graph);
            let (ring_in, ring_out) = ring_arms(&graph, node);
            assert_eq!(graph.edge(ring_in).roundabout, ring);
            assert_eq!(graph.edge(ring_out).roundabout, ring);
            assert_eq!(graph.edge(ring_in).end_node, node);
            assert_eq!(graph.edge(ring_out).start_node, node);
            assert_eq!(graph.edge(ring_in).start_node, before.start_node);
            assert_eq!(graph.edge(ring_out).end_node, before.end_node);
            assert_leads(&graph, ring_in);
            assert_leads(&graph, ring_out);
            let layout = NodeArms::collect(&graph, node).unwrap().layout().unwrap();
            let straight = layout
                .sectors
                .iter()
                .find(|sector| sector.kind == SectorKind::Straight)
                .unwrap_or_else(|| panic!("D{diameter_m}: {:?}", layout.sectors));
            assert!((straight.angle_rad.to_degrees() - 180.0).abs() < 0.01);
            assert!(compiled(&graph).failed_nodes().is_empty(), "D{diameter_m}");
        }
    }
}

#[test]
fn a_split_close_to_a_ring_node_still_builds_its_edge() {
    let (mut graph, applied) = bare_ring(36.0, 1, false);
    let id = applied.ring[0];
    let edge = graph.edge(id);
    let split = edge.geometry[0].lerp(edge.geometry[1], 0.4);
    let node = graph.add_node(split, NodeType::Junction);
    let halves = split_edge_graph(&mut graph, id, 0, node, None).unwrap();
    for half in [id, halves.new_edge_id] {
        let points = &graph.edge(half).geometry;
        assert!(points.len() >= 5);
        assert!(
            points
                .windows(2)
                .all(|pair| pair[0].distance_to(pair[1]) > 1e-3)
        );
    }
}

#[test]
fn a_ring_far_from_the_origin_lays_out_as_at_it() {
    let mut graph = RegionGraph::new();
    let site = RoundaboutSite::Ground {
        centre: Vector3::new(8123.25, 40.0, -6321.5),
        lanes: 1,
    };
    let request = RoundaboutRequest {
        site,
        diameter_m: 36.0,
    };
    RoundaboutLayout::prepare(&graph, request)
        .unwrap()
        .apply(&mut graph);
    let nodes: Vec<u32> = [90.0, 210.0, 330.0, 30.0]
        .into_iter()
        .map(|angle| join_ring_arm(&mut graph, angle, (1, 1), false).0)
        .collect();
    finalize(&mut graph);
    for node in nodes {
        NodeArms::collect(&graph, node).unwrap().layout().unwrap();
    }
    assert!(compiled(&graph).failed_nodes().is_empty());
}

// The default use of a ring node, with D10's rule: every ring lane leaves at every arm. An arm
// drawn away from the ring has its forward lanes leaving it.
#[test]
fn connectors_at_a_one_and_a_two_lane_ring() {
    type Table = fn(usize, usize, usize) -> Vec<(usize, i8, usize, i8)>;
    let cases: [(f32, u8, (u8, u8), Table); 4] = [
        (36.0, 1, (1, 1), |arm, ring_in, out| {
            vec![
                (arm, -1, out, 0),
                (ring_in, 0, arm, 0),
                (ring_in, 0, out, 0),
            ]
        }),
        // The entry fans onto both ring lanes; a one-lane exit merges them.
        (50.0, 2, (1, 1), |arm, ring_in, out| {
            vec![
                (arm, -1, out, 0),
                (arm, -1, out, 1),
                (ring_in, 0, arm, 0),
                (ring_in, 0, out, 0),
                (ring_in, 1, arm, 0),
                (ring_in, 1, out, 1),
            ]
        }),
        // Inner to inner, outer to outer, both ways.
        (50.0, 2, (2, 2), |arm, ring_in, out| {
            vec![
                (arm, -2, out, 1),
                (arm, -1, out, 0),
                (ring_in, 0, arm, 0),
                (ring_in, 0, out, 0),
                (ring_in, 1, arm, 1),
                (ring_in, 1, out, 1),
            ]
        }),
        // An exit-only arm.
        (50.0, 2, (2, 0), |arm, ring_in, out| {
            vec![
                (ring_in, 0, arm, 0),
                (ring_in, 0, out, 0),
                (ring_in, 1, arm, 1),
                (ring_in, 1, out, 1),
            ]
        }),
    ];
    for (diameter_m, ring_lanes, arm_lanes, expected) in cases {
        let (mut graph, _) = bare_ring(diameter_m, ring_lanes, false);
        let joined: Vec<_> = [90.0, 210.0, 330.0]
            .into_iter()
            .map(|angle| join_ring_arm(&mut graph, angle, arm_lanes, false))
            .collect();
        finalize(&mut graph);
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        for (node, arm) in joined {
            let (ring_in, ring_out) = ring_arms(&graph, node);
            let mut table = expected(arm, ring_in, ring_out);
            table.sort_unstable();
            assert_eq!(
                connectors(&lanes, node),
                table,
                "D{diameter_m} {arm_lanes:?}"
            );
        }
    }
}

#[test]
fn uneven_arms_keep_their_angles_and_lay_out() {
    let degrees = |radians: &[f32]| radians.iter().map(|r| r.to_degrees()).collect::<Vec<_>>();
    for (diameter_m, arm, angles) in [
        (36.0, (1, 1), degrees(&[0.0, 1.4, 3.0, 4.7])),
        (50.0, (2, 2), degrees(&[0.0, 1.4, 3.0, 4.7])),
        (36.0, (1, 1), degrees(&[0.0, 0.9, 2.0, 3.2, 4.2, 5.3])),
        (50.0, (2, 2), degrees(&[0.0, 0.9, 2.0, 3.2, 4.2, 5.3])),
    ] {
        let (graph, applied) = converted_ring(diameter_m, &angles, arm, false).unwrap();
        assert_eq!(applied.nodes.len(), angles.len());
        for &node in &applied.nodes {
            NodeArms::collect(&graph, node).unwrap().layout().unwrap();
        }
        assert!(
            compiled(&graph).failed_nodes().is_empty(),
            "D{diameter_m} {angles:?}"
        );
    }
}

// Ported from Veeti's `roundabout_crowded_geometry_refuses_without_mutation`: on `dev` the plan
// builds what it can and the layout refuses arms that do not fit, in the transaction's closure
// compile; only nodes too close for an edge at all stop the plan.
#[test]
fn crowded_arms_are_refused_by_the_layout_or_the_plan() {
    let (graph, applied) = converted_ring(30.0, &[0.0, 30.0, 120.0, 240.0], (1, 1), false).unwrap();
    let refused = applied
        .nodes
        .iter()
        .filter(|&&node| NodeArms::collect(&graph, node).unwrap().layout().is_err())
        .count();
    assert_eq!(refused, 2, "the two arms 30 degrees apart");
    assert_eq!(compiled(&graph).failed_nodes().len(), 2);
    assert_eq!(
        converted_ring(30.0, &[0.0, 8.0, 120.0, 240.0], (1, 1), false).err(),
        Some("roundabout_arms_too_close")
    );
}

// Ported from Veeti's `replacement_preserves_one_way_entry_and_exit_arms`.
#[test]
fn one_way_arms_keep_their_direction_and_lanes() {
    for reverse in [false, true] {
        let mut graph = RegionGraph::new();
        let centre = graph.add_node(Vector3::ZERO, NodeType::Junction);
        for (i, (fwd, bkw)) in [(1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
            let angle = i as f32 * TAU / 3.0;
            let end = Vector3::new(angle.cos(), 0.0, angle.sin()) * 120.0;
            let far = graph.add_node(end, NodeType::Junction);
            let (a, b, points) = if reverse {
                (far, centre, vec![end, Vector3::ZERO])
            } else {
                (centre, far, vec![Vector3::ZERO, end])
            };
            let mut edge = road_edge(a, b, points, 7.0);
            (edge.fwd_lanes, edge.bkw_lanes) = (fwd, bkw);
            graph.add_edge(edge);
        }
        let site = RoundaboutSite::Junction(centre);
        let request = RoundaboutRequest {
            site,
            diameter_m: 40.0,
        };
        let applied = RoundaboutLayout::prepare(&graph, request)
            .unwrap()
            .apply(&mut graph);
        finalize(&mut graph);
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        for (id, counts) in [(1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
            let edge = graph.edge(id);
            assert_eq!((edge.fwd_lanes, edge.bkw_lanes), counts);
            let vehicle = lanes.edge_lanes[&id]
                .iter()
                .filter(|&&lane| lanes.lanes[lane].lane_type == LaneType::Vehicle)
                .count();
            assert_eq!(vehicle, usize::from(counts.0 + counts.1));
        }
        assert_eq!(applied.arms.len(), 3);
    }
}

#[test]
fn a_conversion_keeps_arm_ids_and_far_ends_and_retires_the_junction() {
    let (before, hub) = hub_graph(&equal(4), (1, 1), false);
    let far: Vec<u32> = (0..4)
        .map(|id| {
            let edge = before.edge(id);
            if edge.start_node == hub {
                edge.end_node
            } else {
                edge.start_node
            }
        })
        .collect();
    let (graph, applied) = converted_ring(36.0, &equal(4), (1, 1), false).unwrap();
    assert_eq!(graph.node_adjacency(hub), &[] as &[usize]);
    assert_eq!(graph.find_node_within(Vector3::ZERO, 1.0), None);
    let mut arms = applied.arms.clone();
    arms.sort_unstable();
    assert_eq!(arms, [0, 1, 2, 3]);
    for (id, far) in far.into_iter().enumerate() {
        let edge = graph.edge(id);
        let ends = [edge.start_node, edge.end_node];
        assert!(ends.contains(&far));
        assert!(!ends.contains(&hub));
        let node = if ends[0] == far { ends[1] } else { ends[0] };
        assert!(applied.nodes.contains(&node));
        let pos = graph.node(node).pos;
        assert!((pos.x.hypot(pos.z) - 16.25).abs() < 1e-3);
        assert!(edge.physical_length < before.edge(id).physical_length);
        assert_eq!(
            edge.geometry.first().unwrap().x,
            graph.node(edge.start_node).pos.x
        );
        assert_eq!(
            edge.geometry.last().unwrap().x,
            graph.node(edge.end_node).pos.x
        );
    }
}

#[test]
fn the_rings_lanes_follow_its_arms() {
    let lanes_of = |arm: (u8, u8), diameter_m: f32| {
        let (graph, applied) = converted_ring(diameter_m, &equal(4), arm, false).unwrap();
        graph.edge(applied.ring[0]).fwd_lanes
    };
    assert_eq!(lanes_of((1, 1), 50.0), 1);
    assert_eq!(lanes_of((2, 1), 50.0), 1);
    assert_eq!(lanes_of((1, 2), 50.0), 2);
    assert_eq!(lanes_of((2, 2), 50.0), 2);
    assert_eq!(lanes_of((3, 3), 50.0), 2);
    // The ring's centreline moves in with its lanes: the diameter is the outside kerbs'.
    let radius = |arm, diameter_m| {
        let (graph, applied) = converted_ring(diameter_m, &equal(4), arm, false).unwrap();
        graph.edge(applied.ring[0]).roundabout.unwrap().radius_m
    };
    assert_eq!(radius((1, 1), 50.0), 23.25);
    assert_eq!(radius((2, 2), 50.0), 21.5);
}

#[test]
fn the_plan_refuses_with_its_keys_and_reads_the_graph_only() {
    let refusal = |graph: &RegionGraph, node: u32, diameter_m: f32| {
        let site = RoundaboutSite::Junction(node);
        RoundaboutLayout::prepare(graph, RoundaboutRequest { site, diameter_m }).err()
    };
    let (graph, hub) = hub_graph(&equal(4), (1, 1), false);
    assert_eq!(refusal(&graph, hub, 36.0), None);
    assert_eq!(
        refusal(&graph, 999, 36.0),
        Some("invalid_roundabout_junction")
    );
    assert_eq!(
        refusal(&graph, hub, 29.0),
        Some("invalid_roundabout_dimensions")
    );
    assert_eq!(
        refusal(&graph, hub, f32::NAN),
        Some("invalid_roundabout_dimensions")
    );
    assert_eq!(
        refusal(&graph, hub, 62.0),
        Some("roundabout_replacement_too_large")
    );
    let (wide, wide_hub) = hub_graph(&equal(4), (2, 2), false);
    assert_eq!(
        refusal(&wide, wide_hub, 44.0),
        Some("invalid_roundabout_dimensions")
    );
    let (bend, bend_hub) = hub_graph(&[0.0, 90.0], (1, 1), false);
    assert_eq!(
        refusal(&bend, bend_hub, 36.0),
        Some("roundabout_requires_three_to_six_arms")
    );
    let (seven, seven_hub) = hub_graph(&equal(7), (1, 1), false);
    assert_eq!(
        refusal(&seven, seven_hub, 36.0),
        Some("roundabout_requires_three_to_six_arms")
    );
    let edited = |edit: fn(&mut Edge)| {
        let (mut graph, hub) = hub_graph(&equal(4), (1, 1), false);
        edit(graph.edge_mut(0));
        refusal(&graph, hub, 36.0)
    };
    assert_eq!(
        edited(|edge| edge.class = EdgeClass::Bridge),
        Some("roundabout_requires_ordinary_surface_arms")
    );
    assert_eq!(
        edited(|edge| {
            edge.primary_type = TransitType::Foot;
            edge.allowed_types = TransitFlags::FOOT;
            edge.sidewalks = SidewalkSides::NONE;
        }),
        Some("roundabout_requires_ordinary_surface_arms")
    );
    assert_eq!(
        edited(|edge| {
            edge.pocket = Some(crate::simulation::network::graph::RoadPocket {
                end: EdgeEnd::End,
                left: false,
                right: true,
            });
        }),
        Some("roundabout_arm_has_pocket")
    );
    // A whitelisted node.
    let (mut limited, limited_hub) = hub_graph(&equal(4), (1, 1), false);
    limited.add_lane_connection(limited_hub, 1, 0, 0, 0);
    assert_eq!(
        refusal(&limited, limited_hub, 36.0),
        Some("roundabout_has_lane_restrictions")
    );
    // An arm that ends inside the circle.
    let (mut short, short_hub) = hub_graph(&equal(3), (1, 1), false);
    let far = short.add_node(point(10.0, 10.0, false), NodeType::Junction);
    connect(&mut short, short_hub, far, 7.0, false);
    finalize(&mut short);
    assert_eq!(
        refusal(&short, short_hub, 36.0),
        Some("roundabout_arm_too_short")
    );
}

#[test]
fn the_ring_keeps_clear_of_other_roads_and_on_empty_ground_refuses_to_touch_them() {
    let ground = |graph: &RegionGraph| {
        let site = RoundaboutSite::Ground {
            centre: Vector3::ZERO,
            lanes: 1,
        };
        RoundaboutLayout::prepare(
            graph,
            RoundaboutRequest {
                site,
                diameter_m: 36.0,
            },
        )
        .err()
    };
    for (z, refused) in [(20.0, true), (24.0, true), (26.0, false)] {
        let mut graph = RegionGraph::new();
        let a = graph.add_node(point(-60.0, z, false), NodeType::Junction);
        let b = graph.add_node(point(60.0, z, false), NodeType::Junction);
        connect(&mut graph, a, b, 7.0, false);
        finalize(&mut graph);
        assert_eq!(
            ground(&graph),
            refused.then_some("roundabout_overlaps_road"),
            "{z}"
        );
    }
    // A road beside a junction that the conversion's ring would reach.
    let (mut graph, hub) = hub_graph(&[0.0, 120.0, 240.0], (1, 1), false);
    let a = graph.add_node(point(-16.0, -60.0, false), NodeType::Junction);
    let b = graph.add_node(point(-16.0, 60.0, false), NodeType::Junction);
    let far = connect(&mut graph, a, b, 7.0, false);
    finalize(&mut graph);
    let site = RoundaboutSite::Junction(hub);
    let request = RoundaboutRequest {
        site,
        diameter_m: 36.0,
    };
    assert_eq!(
        RoundaboutLayout::prepare(&graph, request).err(),
        Some("roundabout_overlaps_road"),
        "road {far}"
    );
}

#[test]
fn a_ring_edge_loads_only_as_built() {
    let (graph, applied) = bare_ring(36.0, 1, false);
    let id = applied.ring[0];
    let verdict = |edit: fn(&mut Edge)| {
        let mut graph = graph.clone();
        edit(graph.edge_mut(id));
        validate_ring_edge(&graph, id)
    };
    assert_eq!(verdict(|_| ()), Ok(()));
    assert_eq!(verdict(|edge| edge.bkw_lanes = 1), Err("ring_edge_section"));
    assert_eq!(verdict(|edge| edge.fwd_lanes = 3), Err("ring_edge_section"));
    assert_eq!(
        verdict(|edge| edge.speed_limit = 10.0),
        Err("ring_edge_section")
    );
    assert_eq!(
        verdict(|edge| edge.no_building_spawn = false),
        Err("ring_edge_section")
    );
    assert_eq!(
        verdict(|edge| edge.sidewalks = SidewalkSides::LEFT),
        Err("ring_edge_section")
    );
    assert_eq!(
        verdict(|edge| edge.roundabout.as_mut().unwrap().radius_m = 0.0),
        Err("ring_edge_circle")
    );
    assert_eq!(
        verdict(|edge| edge.roundabout.as_mut().unwrap().radius_m = 20.0),
        Err("ring_edge_not_closed")
    );
}

// Every connector from an arm onto a ring lane gives way to the circulating connector into the
// same ring lane; nothing else does. `(arm lanes)` 1+1 on a one-lane ring, 1+1 fanned onto a
// two-lane ring, 2+2 on a two-lane ring.
fn assert_ring_entries(graph: &RegionGraph, lanes: &LaneSystem, nodes: &[u32]) {
    let on_ring = |edge: usize| edge != usize::MAX && graph.edge(edge).roundabout.is_some();
    let all = &lanes.lanes;
    let mut seen = 0;
    for &id in lanes.edge_lanes.values().flatten() {
        let lane = &all[id];
        let entries_ring = !on_ring(lane.edge_id) && lane.lane_type == LaneType::Vehicle;
        for &connector in lane.next_lanes.iter().filter(|_| entries_ring) {
            let target = all[connector].next_lanes[0];
            let Some(entry) = lanes.ring_entry(connector) else {
                assert!(!on_ring(all[target].edge_id), "lane {id} enters unmarked");
                continue;
            };
            seen += 1;
            assert!(nodes.contains(&(all[connector].node_id as u32)));
            let (circulating, ring_in) = (entry.circulating as usize, entry.ring_in as usize);
            assert!(on_ring(all[target].edge_id));
            assert!(on_ring(all[ring_in].edge_id));
            assert_eq!(all[circulating].next_lanes[0], target);
            assert!(all[ring_in].next_lanes.contains(&circulating));
            assert_eq!(all[circulating].node_id, all[connector].node_id);
            assert_eq!(lanes.ring_entry(circulating), None);
        }
    }
    assert!(seen > 0);
    for lane in all.iter() {
        assert!(lane.edge_id == usize::MAX || lane.ring_entry.is_none());
    }
}

#[test]
fn an_arms_connectors_onto_a_ring_give_way_to_the_ring() {
    for (diameter_m, arm) in [(36.0, (1, 1)), (45.0, (2, 2))] {
        let (mut graph, applied) = converted_ring(diameter_m, &equal(4), arm, false).unwrap();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        assert_ring_entries(&graph, &lanes, &applied.nodes);
    }
    // A one-lane arm joined to a two-lane ring fans onto both ring lanes.
    let (mut graph, applied) = bare_ring(50.0, 2, false);
    let (node, _) = join_ring_arm(&mut graph, 40.0, (1, 1), false);
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    assert_ring_entries(&graph, &lanes, &[node]);
    assert_eq!(applied.nodes.len(), 3);
}

#[test]
fn give_way_survives_an_incremental_rebuild_and_retires_with_its_connectors() {
    let (mut graph, applied) = converted_ring(36.0, &equal(4), (1, 1), false).unwrap();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    let old: Vec<usize> = (0..lanes.lanes.len())
        .filter(|&id| lanes.ring_entry(id).is_some())
        .collect();
    assert_eq!(old.len(), 4);
    lanes.rebuild_edges_incremental(&mut graph, &HashSet::from([applied.arms[0]]));
    assert_ring_entries(&graph, &lanes, &applied.nodes);
    let live = (0..lanes.lanes.len()).filter(|&id| lanes.ring_entry(id).is_some());
    assert_eq!(live.count(), 4);
    // The arm's rebuild retired its node's connectors and every ring node next to it.
    assert!(old.iter().any(|&id| lanes.ring_entry(id).is_none()));
}
