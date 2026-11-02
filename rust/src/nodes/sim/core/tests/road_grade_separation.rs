// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-99`: grade separation by structure, not by height. Two roads meet only where both are
//! grounded; a bridge or a tunnel's bore crosses other roads without a node, and a road passes
//! under a bridge only with the vehicle clearance under its deck.

use super::road_over_tunnels::{PieceBits, dig, pieces, refill};
use super::road_tunnels::{THROUGH_HILL, cone_hill_core};
use super::*;
use crate::nodes::sim::core::road_commit::{RoadCommitOutcome, RoadCommitRequest};
use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction::find_intersection_2d;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::road_edit::RoadPreviewValidation;
use crate::simulation::network::types::EdgeClass;
use std::collections::BTreeMap;
use std::sync::Arc;

// The vehicle clearance under a bridge, deck to deck (`road_edit::bridge::ROAD_UNDER_BRIDGE_M`).
const UNDER_BRIDGE_M: f32 = 4.85;
// A road drawn north to south across the origin of the flat world.
const UNDER: [Vector3; 2] = [Vector3::new(0.0, 0.0, -40.0), Vector3::new(0.0, 0.0, 40.0)];

// The flat 256 m world with no roads (a cone hill of no height).
fn flat() -> SimCore {
    cone_hill_core(0.0, 0.25)
}

// A bridge drawn west to east across the origin, `deck_m` over the flat ground: both ends stand
// above it, so the class inference makes it a bridge keeping its authored heights.
fn deck(deck_m: f32) -> [Vector3; 2] {
    [
        Vector3::new(-30.0, deck_m, 0.0),
        Vector3::new(30.0, deck_m, 0.0),
    ]
}

// What the hover check, the preview's exact plan and the commit (adopting the preview's plan)
// do with one stroke.
struct Drawn {
    quick: RoadPreviewValidation,
    exact: RoadPreviewValidation,
    outcome: RoadCommitOutcome,
}

impl Drawn {
    // The three answers, `valid` for an accepted stroke.
    fn reasons(&self) -> [&str; 3] {
        let reason = |v: &RoadPreviewValidation| {
            if v.is_valid {
                "valid"
            } else {
                v.invalid_reason
            }
        };
        let committed = if self.outcome.committed {
            "valid"
        } else {
            self.outcome.rejection_reason.as_str()
        };
        [reason(&self.quick), reason(&self.exact), committed]
    }
}

fn draw(core: &mut SimCore, points: &[Vector3], tunnel: bool, snap: bool) -> Drawn {
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    let request = |core: &SimCore| RoadPreviewRequest {
        surface_generation: core.road_tool_surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: snap,
        tunnel,
        median: None,
    };
    let view = road_tool_snapshot_from_core(core).expect("current roadbed");
    let quick = view.validate(&request(core)).1;
    let preview = Arc::new(RoadPreviewContext::capture(view).preview_road(request(core)));
    let exact = preview.validation.clone();
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: snap,
            tunnel,
            median: None,
            preview: Some(preview),
        },
        std::time::Instant::now(),
        Default::default(),
    );
    core.precompute_road_mesh_data();
    Drawn {
        quick,
        exact,
        outcome,
    }
}

// Every live edge's class, nodes and both polylines, and every live node's position, bitwise.
type EdgeBits = (EdgeClass, [u32; 2], Vec<[u32; 3]>, Vec<[u32; 3]>);

fn network(core: &SimCore) -> (BTreeMap<usize, EdgeBits>, Vec<[u32; 3]>) {
    let graph = &core.region_graph;
    let bits = |points: &[Vector3]| {
        (points.iter())
            .map(|p| [p.x, p.y, p.z].map(f32::to_bits))
            .collect::<Vec<_>>()
    };
    let edges = (graph.edges().iter().enumerate())
        .filter(|(_, edge)| !edge.deleted)
        .map(|(id, edge)| {
            let ends = [edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node));
            let record = (
                edge.class,
                ends,
                bits(&edge.geometry),
                bits(&edge.physical_geometry),
            );
            (id, record)
        })
        .collect();
    let nodes = (0..graph.node_count() as u32)
        .filter(|&node| graph.get_valid_node(node) == node)
        .map(|node| graph.node(node).pos)
        .collect::<Vec<_>>();
    (edges, bits(&nodes))
}

fn live(core: &SimCore, class: EdgeClass) -> Vec<usize> {
    (core.region_graph.edges().iter().enumerate())
        .filter(|(_, edge)| !edge.deleted && edge.class == class)
        .map(|(id, _)| id)
        .collect()
}

// The structure is refused the same way by all three paths, with its least height as the
// clearance, and leaves the network and every piece as they were.
fn assert_too_low(
    core: &SimCore,
    drawn: &Drawn,
    apart_m: f32,
    before: &(BTreeMap<usize, EdgeBits>, Vec<[u32; 3]>),
    before_pieces: &PieceBits,
    case: &str,
) {
    assert_eq!(drawn.reasons(), ["bridge_too_low"; 3], "{case}");
    for validation in [&drawn.quick, &drawn.exact] {
        assert!(
            (validation.clearance_m - apart_m).abs() < 0.05,
            "{case}: {validation:?}"
        );
        assert_eq!(
            validation.required_clearance_m.to_bits(),
            UNDER_BRIDGE_M.to_bits(),
            "{case}"
        );
    }
    assert!(&network(core) == before, "{case}: the network is unchanged");
    assert!(
        &pieces(core) == before_pieces,
        "{case}: every piece is unchanged"
    );
}

// Case 1 of `ROAD-99`: a road drawn on the ground across a bridge 6 m over it passes under it.
// Before, every 6 m sample under the deck was pinned to the deck and the crossing became a
// junction on the bridge (or the stroke too steep): no road passed under a bridge. Now it keeps
// the ground, the bridge's edge, nodes and profiles stay as they were, and the hover check, the
// preview and the commit agree; undo restores every piece. Under a deck 4 m over the ground the
// road is refused as `bridge_too_low`.
#[test]
fn a_ground_road_passes_under_a_bridge() {
    for (deck_m, passes) in [(6.0, true), (4.0, false)] {
        let case = format!("deck {deck_m} m");
        let mut core = flat();
        let bridge = draw(&mut core, &deck(deck_m), false, true);
        assert_eq!(bridge.reasons(), ["valid"; 3], "{case}");
        assert_eq!(live(&core, EdgeClass::Bridge).len(), 1, "{case}");
        let (before, before_pieces) = (network(&core), pieces(&core));
        let road = draw(&mut core, &UNDER, false, true);
        if !passes {
            assert_too_low(&core, &road, deck_m, &before, &before_pieces, &case);
            continue;
        }
        assert_eq!(road.reasons(), ["valid"; 3], "{case}");
        let after = network(&core);
        let bridge_edge = live(&core, EdgeClass::Bridge);
        assert_eq!(bridge_edge.len(), 1, "{case}: the bridge is not split");
        assert_eq!(
            after.0[&bridge_edge[0]], before.0[&bridge_edge[0]],
            "{case}"
        );
        assert_eq!(
            after.1.len(),
            before.1.len() + 2,
            "{case}: the road's two ends"
        );
        let roads = live(&core, EdgeClass::Standard);
        assert_eq!(roads.len(), 1, "{case}: the road is not split");
        let profile = &core.region_graph.edge(roads[0]).physical_geometry;
        assert!(
            profile.iter().all(|p| p.y.abs() <= 0.05),
            "{case}: the road keeps the ground under the deck: {profile:?}"
        );
        assert!(core.undo_action_internal(), "{case}");
        core.precompute_road_mesh_data();
        assert!(network(&core) == before, "{case}: undo");
        assert!(
            pieces(&core) == before_pieces,
            "{case}: undo restores every piece"
        );
    }
}

// The mirror of case 1: a bridge drawn over a road on the ground, and over a lower bridge. Before,
// a bridge within 4.5 m of the road split it and itself at the road's height (the deck dipped to
// the ground), and above 4.5 m nothing checked the clearance. Now it crosses without a node while
// the decks stand 4.85 m apart, and is refused as `bridge_too_low` closer.
#[test]
fn a_bridge_passes_over_a_road_or_a_bridge() {
    let lower_bridge = [Vector3::new(0.0, 2.0, -40.0), Vector3::new(0.0, 2.0, 40.0)];
    for (under, under_m, deck_m, passes) in [
        (UNDER, 0.0, 6.0, true),
        (UNDER, 0.0, 4.0, false),
        (lower_bridge, 2.0, 7.5, true),
        (lower_bridge, 2.0, 6.0, false),
    ] {
        let case = format!("deck {deck_m} m over {under_m} m");
        let mut core = flat();
        assert_eq!(
            draw(&mut core, &under, false, true).reasons(),
            ["valid"; 3],
            "{case}"
        );
        let (before, before_pieces) = (network(&core), pieces(&core));
        let bridge = draw(&mut core, &deck(deck_m), false, true);
        if !passes {
            assert_too_low(
                &core,
                &bridge,
                deck_m - under_m,
                &before,
                &before_pieces,
                &case,
            );
            continue;
        }
        assert_eq!(bridge.reasons(), ["valid"; 3], "{case}");
        let after = network(&core);
        assert_eq!(
            after.1.len(),
            before.1.len() + 2,
            "{case}: the bridge's two ends"
        );
        for (id, record) in &before.0 {
            assert_eq!(&after.0[id], record, "{case}: the road under it is intact");
        }
        assert_eq!(after.0.len(), before.0.len() + 1, "{case}: one new edge");
        assert!(core.undo_action_internal(), "{case}");
        core.precompute_road_mesh_data();
        assert!(network(&core) == before, "{case}: undo");
        assert!(
            pieces(&core) == before_pieces,
            "{case}: undo restores every piece"
        );
    }
}

// A bridge drawn from a road mid-span keeps working (`ROAD-99`, R5 and R2): its end at the road
// is a ground end (within 1 m of its support), so it joins the road there, and the road it joins
// is no deck it must clear. Its other end stands 6 m over the ground.
#[test]
fn a_bridge_starts_from_a_road_mid_span() {
    let mut core = flat();
    assert_eq!(draw(&mut core, &UNDER, false, true).reasons(), ["valid"; 3]);
    let ramp = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(60.0, 6.0, 0.0)];
    let drawn = draw(&mut core, &ramp, false, true);
    assert_eq!(drawn.reasons(), ["valid"; 3]);
    assert_eq!(live(&core, EdgeClass::Bridge).len(), 1);
    assert_eq!(
        live(&core, EdgeClass::Standard).len(),
        2,
        "the road is split at the bridge"
    );
}

// B1, the `ROAD-99` follow-up: a stroke whose end is snapped onto a bridge's span joins the span
// there. `deck(6.0)` runs from (-30, 6, 0) to (30, 6, 0); its middle lies 30 m from its nodes,
// where the road tool's cursor, snapped to the span, reports the deck's graph height.
const MID_SPAN: Vector3 = Vector3::new(0.0, 6.0, 0.0);

// The live canonical node standing within 0.05 m of `at` in plan.
fn node_at(core: &SimCore, at: Vector3) -> Option<u32> {
    let graph = &core.region_graph;
    (0..graph.node_count() as u32).find(|&node| {
        let pos = graph.node(node).pos;
        graph.get_valid_node(node) == node
            && !graph.node_adjacency(node).is_empty()
            && (pos.x - at.x).hypot(pos.z - at.z) < 0.05
    })
}

fn live_arms(core: &SimCore, node: u32) -> Vec<usize> {
    let graph = &core.region_graph;
    (graph.node_adjacency(node).iter().copied())
        .filter(|&edge| !graph.edge(edge).deleted)
        .collect()
}

// Draws `stroke` onto the 6 m bridge's middle and checks the join: the hover check, the preview
// and the commit accept it; the bridge is split there into two bridge edges meeting the stroke at
// one junction on the deck, at the deck's height; every arm there is a structure, so the junction
// is not grounded and none takes buildings; its node piece and its lanes are built, its
// connectors join the stroke to both halves; undo restores the network and every piece bitwise.
fn assert_joins_mid_span(stroke: &[Vector3], case: &str) -> SimCore {
    let mut core = flat();
    assert_eq!(draw(&mut core, &deck(6.0), false, true).reasons(), ["valid"; 3]);
    let (before, before_pieces) = (network(&core), pieces(&core));
    let drawn = draw(&mut core, stroke, false, true);
    assert_eq!(drawn.reasons(), ["valid"; 3], "{case}: {:?}", drawn.exact);
    let bridges = live(&core, EdgeClass::Bridge);
    assert_eq!(bridges.len(), 3, "{case}: the bridge split and the stroke");
    assert!(live(&core, EdgeClass::Standard).is_empty(), "{case}");
    let node = node_at(&core, MID_SPAN).expect("a junction mid-span");
    let graph = &core.region_graph;
    assert_eq!(
        graph.node(node).pos.y.to_bits(),
        MID_SPAN.y.to_bits(),
        "{case}: the junction stands on the deck"
    );
    let arms = live_arms(&core, node);
    assert_eq!(arms.len(), 3, "{case}");
    assert!(!graph.grounded_node(node), "{case}: a junction on a deck");
    assert!(
        bridges.iter().all(|&edge| !graph.takes_buildings(edge)),
        "{case}: a bridge takes no frontage"
    );
    let roadbed = &core.transit_network.roadbed;
    assert!(roadbed.node_piece(node).is_some(), "{case}: the node piece");
    assert!(roadbed.failed_nodes().is_empty(), "{case}");
    let lanes = &core.transit_network.lane_system;
    let node_lanes = &lanes.node_lanes[&(node as usize)];
    for &arm in &arms {
        let joined = node_lanes.iter().any(|&id| {
            let lane = &lanes.lanes[id];
            lane.next_lanes.iter().any(|&next| {
                let next = &lanes.lanes[next];
                next.edge_id == arm && next.lane_type == LaneType::Vehicle
            })
        });
        assert!(joined, "{case}: a connector leads into arm {arm}");
    }
    assert!(core.undo_action_internal(), "{case}");
    core.precompute_road_mesh_data();
    assert!(network(&core) == before, "{case}: undo");
    assert!(pieces(&core) == before_pieces, "{case}: undo restores every piece");
    core
}

// B1: a bridge stroke whose end is snapped onto a bridge mid-span joins it there. Before, a
// structure's span took no mid-span join, so the stroke ended on its own node on the deck and
// was refused as `bridge_too_low` 0.00 m (the user's play-test, 2026-11-02).
#[test]
fn a_bridge_stroke_joins_a_bridge_mid_span() {
    assert_joins_mid_span(&[Vector3::new(0.0, 6.0, 40.0), MID_SPAN], "bridge stroke");
}

// B1: a road drawn from the ground and snapped onto a bridge mid-span climbs to it and joins it
// there, from either end; the class inference makes it a bridge, as at the bridge's nodes.
#[test]
fn a_road_from_the_ground_joins_a_bridge_mid_span() {
    let ground = Vector3::new(0.0, 0.0, 80.0);
    assert_joins_mid_span(&[ground, MID_SPAN], "climbing to the deck");
    assert_joins_mid_span(&[MID_SPAN, ground], "leaving the deck");
}

// B1: the junction a mid-span join builds on the deck is the one a join at the bridge's node
// builds there: the same pieces (node piece, the bridge's two halves, the stroke's span), each
// the same set of triangles bitwise, and the same crossings (the default zebra rule) and
// connectors. The node join's bridge is drawn as two strokes meeting at
// (0, 6, 0); its edges carry other ids, which order a node piece's vertices differently.
#[test]
fn a_mid_span_join_builds_what_a_node_join_builds() {
    let stroke = [Vector3::new(0.0, 6.0, 40.0), MID_SPAN];
    let built = |bridge: &[[Vector3; 2]]| {
        let mut core = flat();
        for part in bridge {
            assert_eq!(draw(&mut core, part, false, true).reasons(), ["valid"; 3]);
        }
        assert_eq!(draw(&mut core, &stroke, false, true).reasons(), ["valid"; 3]);
        let mut meshes: Vec<Vec<[[u64; 3]; 3]>> = (pieces(&core).into_values())
            .map(|(positions, triangles)| {
                let mut triangles: Vec<[[u64; 3]; 3]> = (triangles.iter())
                    .map(|triangle| {
                        let mut corners = triangle.map(|corner| positions[corner as usize]);
                        corners.sort();
                        corners
                    })
                    .collect();
                triangles.sort();
                triangles
            })
            .collect();
        meshes.sort();
        // The junction's crossings (the default zebra rule) and its lanes, by kind.
        let lanes = &core.transit_network.lane_system;
        let node = node_at(&core, MID_SPAN).expect("the junction");
        let node_lanes = &lanes.node_lanes[&(node as usize)];
        let count = |crossing: bool, kind: LaneType| {
            (node_lanes.iter())
                .filter(|&&id| {
                    lanes.crosswalk(id).is_some() == crossing && lanes.lanes[id].lane_type == kind
                })
                .count()
        };
        let lanes = [
            count(true, LaneType::Foot),
            count(false, LaneType::Foot),
            count(false, LaneType::Vehicle),
        ];
        (meshes, lanes)
    };
    let [west, east] = deck(6.0);
    let at_node = built(&[[west, MID_SPAN], [MID_SPAN, east]]);
    let mid_span = built(&[deck(6.0)]);
    assert_eq!(at_node.0.len(), 4, "a node piece and three spans");
    assert!(at_node.0 == mid_span.0, "the same junction on the deck");
    assert!(at_node.1[0] > 0, "zebras on the deck: {:?}", at_node.1);
    assert_eq!(at_node.1, mid_span.1, "the same crossings and connectors");
}

// B1: a road ending on the ground under a bridge does not join it: its end stands on the
// ground's level, not the deck's. Under a 6 m deck it passes; under a 4 m deck it is refused as
// too low, which it is.
#[test]
fn a_road_ending_under_a_bridge_does_not_join_it() {
    for (deck_m, passes) in [(6.0, true), (4.0, false)] {
        let case = format!("deck {deck_m} m");
        let mut core = flat();
        assert_eq!(draw(&mut core, &deck(deck_m), false, true).reasons(), ["valid"; 3]);
        let (before, before_pieces) = (network(&core), pieces(&core));
        let road = [Vector3::new(0.0, 0.0, 40.0), Vector3::ZERO];
        let drawn = draw(&mut core, &road, false, true);
        if !passes {
            assert_too_low(&core, &drawn, deck_m, &before, &before_pieces, &case);
            continue;
        }
        assert_eq!(drawn.reasons(), ["valid"; 3], "{case}");
        assert_eq!(live(&core, EdgeClass::Bridge).len(), 1, "{case}: not split");
        assert_eq!(live(&core, EdgeClass::Standard).len(), 1, "{case}");
    }
}

// B1: a refused mid-span join names its real cause, never the clearance under the bridge it
// joins or the deck that bridge continues: a ramp from 12 m away climbs 6 m (too steep), a stroke
// meets the deck at 7.6 degrees (too sharp an angle between the junction's arms), and a join 5 m
// from the bridge's bend node at (0, 6, 0) leaves the arm between them too short. The hover
// check runs no junction rule and does not refuse these.
#[test]
fn a_refused_mid_span_join_names_its_cause() {
    let [west, east] = deck(6.0);
    let bent = [[west, MID_SPAN], [MID_SPAN, east]];
    let near_bend = Vector3::new(5.0, 6.0, 0.0);
    for (bridge, stroke, cause) in [
        (&[deck(6.0)][..], [Vector3::new(0.0, 0.0, 12.0), MID_SPAN], "road_too_steep"),
        (&[deck(6.0)], [Vector3::new(-60.0, 6.0, 8.0), MID_SPAN], "junction_arm_angle"),
        (&bent, [Vector3::new(5.0, 6.0, 40.0), near_bend], "junction_arm_too_short"),
    ] {
        let mut core = flat();
        for part in bridge {
            assert_eq!(draw(&mut core, part, false, true).reasons(), ["valid"; 3]);
        }
        let before = network(&core);
        let drawn = draw(&mut core, &stroke, false, true);
        assert_eq!(
            drawn.reasons(),
            ["valid", cause, cause],
            "{cause}: {:?}",
            drawn.exact
        );
        assert!(network(&core) == before, "{cause}: nothing committed");
    }
}

// B1: a road ending on the ground beside or under a bridge, 1 to 10 m from its centre line in
// plan, stays on the ground and joins nothing: the bridge is not split. The cursor gives such
// an end the ground's height (its ray hit the ground, `StructureHit`).
#[test]
fn a_road_ending_beside_a_bridge_stays_on_the_ground() {
    for off in [1.0, 3.0, 5.0, 8.0, 10.0] {
        let case = format!("{off} m from the centre line");
        let mut core = flat();
        assert_eq!(draw(&mut core, &deck(6.0), false, true).reasons(), ["valid"; 3]);
        let bridge = network(&core).0[&live(&core, EdgeClass::Bridge)[0]].clone();
        let road = [Vector3::new(10.0, 0.0, 40.0), Vector3::new(0.0, 0.0, off)];
        let drawn = draw(&mut core, &road, false, true);
        assert_eq!(drawn.reasons(), ["valid"; 3], "{case}: {:?}", drawn.exact);
        let bridges = live(&core, EdgeClass::Bridge);
        assert_eq!(bridges.len(), 1, "{case}: not split");
        assert!(network(&core).0[&bridges[0]] == bridge, "{case}: the bridge intact");
        let roads = live(&core, EdgeClass::Standard);
        assert_eq!(roads.len(), 1, "{case}");
        let profile = &core.region_graph.edge(roads[0]).physical_geometry;
        assert!(
            profile.iter().all(|p| p.y.abs() <= 0.05),
            "{case}: on the ground: {profile:?}"
        );
    }
}

// B1: a road ending on the hill right over a tunnel's bore commits, unjoined, at the hill's
// height: the cursor never snaps to a buried bore (`StructureHit`), and an end on the hill stands
// nearer the ground than the bore. On the 1:2 hill the bore runs 25 m under (-30, 0).
#[test]
fn a_road_ending_on_the_hill_over_a_bore_stays_on_the_hill() {
    let mut core = cone_hill_core(80.0, 0.5);
    assert_eq!(draw(&mut core, &THROUGH_HILL, true, true).reasons(), ["valid"; 3]);
    let before = network(&core);
    let hill = |x: f32, z: f32| 0.5 * (80.0 - x.hypot(z)).max(0.0);
    let end = Vector3::new(-30.0, hill(-30.0, 0.0), 0.0);
    let start = Vector3::new(-30.0, hill(-30.0, -30.0), -30.0);
    let drawn = draw(&mut core, &[start, end], false, true);
    assert_eq!(drawn.reasons(), ["valid"; 3], "{:?}", drawn.exact);
    let after = network(&core);
    for (id, record) in &before.0 {
        assert!(&after.0[id] == record, "the tunnel and its approaches intact");
    }
    assert_eq!(after.0.len(), before.0.len() + 1, "one new road, unjoined");
    let road = live(&core, EdgeClass::Standard)
        .into_iter()
        .find(|id| !before.0.contains_key(id))
        .unwrap();
    let profile = &core.region_graph.edge(road).physical_geometry;
    let last = profile[profile.len() - 1];
    assert!(
        (last.y - core.get_height_at_internal(godot::prelude::Vector2::new(last.x, last.z))).abs() <= 0.05,
        "the end at the hill's height: {last:?}"
    );
}

// B1: a stroke whose end lies on a tunnel's bore, at the bore's graph height on its centre line
// (a scripted or replayed point: the cursor never snaps to a bore), is refused with its own
// reason. On the 1:2 hill the bore runs 15 m under the flank at x = -50.
#[test]
fn a_road_does_not_join_a_tunnel_mid_bore() {
    let mut core = cone_hill_core(80.0, 0.5);
    assert_eq!(draw(&mut core, &THROUGH_HILL, true, true).reasons(), ["valid"; 3]);
    let bore = live(&core, EdgeClass::Tunnel);
    assert_eq!(bore.len(), 1);
    let profile = core.region_graph.edge(bore[0]).physical_geometry.clone();
    let end = Vector3::new(-50.0, RegionGraph::profile_height_at(&profile, -50.0, 0.0), 0.0);
    let before = network(&core);
    let drawn = draw(&mut core, &[Vector3::new(-30.0, 15.0, -40.0), end], false, true);
    assert_eq!(drawn.reasons(), ["tunnel_join"; 3], "{:?}", drawn.exact);
    assert!(network(&core) == before);
}

// The live standard edges cross in plan nowhere without a node.
fn ground_roads_meet_where_they_cross(core: &SimCore) -> bool {
    !scan_height_only(&core.region_graph)
        .crossings
        .contains_key(&(EdgeClass::Standard as u8, EdgeClass::Standard as u8))
}

// Case 2 of `ROAD-99`: a road in a cut across the flank of a hill, and a road climbing the hill
// over it: on the 1:4 hill 8.5 m above the cut road, on the 1:8 hill 5.25 m above it. Before, a
// crossing with no sample pinned to the cut road (the support off, or a sampling phase that
// straddled it) was more than 4.5 m apart and committed with the two roads crossing without a
// node: the upper road on an embankment damming the cut. Now two ground roads always meet:
// every variant commits with a junction (the 1:8 hill with the support off) or is refused as too
// steep to reach the cut.
#[test]
fn a_road_over_a_cut_road_meets_it_or_is_refused() {
    let cut_road = |slope: f32, cut_m: f32| {
        let mut core = cone_hill_core(100.0, slope);
        dig(&mut core, -56.0, -44.0, cut_m);
        let cut = [
            Vector3::new(-50.0, 0.0, -60.0),
            Vector3::new(-50.0, 0.0, 60.0),
        ];
        assert_eq!(draw(&mut core, &cut, false, true).reasons(), ["valid"; 3]);
        refill(&mut core, -56.0, -44.0);
        core
    };
    let mut junctions = 0;
    for (slope, cut_m) in [(0.25, 4.0), (0.125, 1.0)] {
        for snap in [true, false] {
            for shift in [0.0, 2.0, 4.0] {
                let case = format!("1:{} snap {snap} start shifted {shift} m", 1.0 / slope);
                let mut core = cut_road(slope, cut_m);
                let along = [
                    Vector3::new(-100.0 + shift, 0.0, 0.0),
                    Vector3::new(-10.0, 0.0, 0.0),
                ];
                let drawn = draw(&mut core, &along, false, snap);
                let [_, exact, committed] = drawn.reasons();
                assert_eq!(exact, committed, "{case}");
                if drawn.outcome.committed {
                    assert!(ground_roads_meet_where_they_cross(&core), "{case}");
                    assert_eq!(
                        live(&core, EdgeClass::Standard).len(),
                        4,
                        "{case}: one junction"
                    );
                    junctions += 1;
                } else {
                    assert_eq!(committed, "road_too_steep", "{case}");
                }
            }
        }
    }
    assert!(junctions >= 3, "{junctions} crossings made a junction");
}

// Case 3 of `ROAD-99`, roads across a tunnel's approach in its trench. (a) A ground road across
// the 1:2 hill's approach meets it in the trench and is refused (`junction_in_trench`, as
// before). (b) A bridge across it 6 m over its deck crosses it without a node, the approach and
// the tunnel intact; 4 m over it is `bridge_too_low` (before, it split the approach and was
// refused for the junction in the trench).
#[test]
fn a_road_over_a_tunnel_approach_meets_it_or_bridges_it() {
    let tunnel = || {
        let mut core = cone_hill_core(80.0, 0.5);
        assert_eq!(
            draw(&mut core, &THROUGH_HILL, true, true).reasons(),
            ["valid"; 3]
        );
        core
    };
    let across = |height_m: f32| {
        [
            Vector3::new(-75.0, height_m, -30.0),
            Vector3::new(-75.0, height_m, 30.0),
        ]
    };
    let mut core = tunnel();
    let ground = draw(&mut core, &across(0.0), false, true);
    assert_eq!(ground.reasons()[1..], ["junction_in_trench"; 2], "(a)");
    for (deck_m, passes) in [(6.0, true), (4.0, false)] {
        let case = format!("(b) deck {deck_m} m");
        let mut core = tunnel();
        let (before, before_pieces) = (network(&core), pieces(&core));
        let bridge = draw(&mut core, &across(deck_m), false, true);
        if !passes {
            assert_too_low(&core, &bridge, deck_m, &before, &before_pieces, &case);
            continue;
        }
        assert_eq!(bridge.reasons(), ["valid"; 3], "{case}");
        let after = network(&core);
        for (id, record) in &before.0 {
            assert_eq!(
                &after.0[id], record,
                "{case}: the approach and the tunnel are intact"
            );
        }
    }
}

// The mirror of case 3: a tunnel drawn under a road on the 1:4 hill, its approach crossing 5 m
// under the road. Before, more than 4.5 m apart, they crossed without a node and the road hung
// over the new trench; now the approach meets the road, and the tunnel is refused (as too steep
// to climb to the junction).
#[test]
fn a_tunnel_approach_drawn_under_a_road_meets_it() {
    let mut core = cone_hill_core(100.0, 0.25);
    let hill_road = [
        Vector3::new(-80.0, 0.0, -40.0),
        Vector3::new(-80.0, 0.0, 40.0),
    ];
    assert_eq!(
        draw(&mut core, &hill_road, false, true).reasons(),
        ["valid"; 3]
    );
    let before = network(&core);
    let under = draw(&mut core, &THROUGH_HILL, true, true);
    let [_, exact, committed] = under.reasons();
    assert_eq!(exact, committed, "(c)");
    assert_ne!(
        committed, "valid",
        "(c): the approach meets the road and is refused"
    );
    assert!(network(&core) == before, "(c): the hill road is intact");
}

// The profile a crossing's heights are read on: the physical profile when present.
fn profile(edge: &Edge) -> &[Vector3] {
    if edge.physical_geometry.len() >= 2 {
        &edge.physical_geometry
    } else {
        &edge.geometry
    }
}

fn live_surface(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}

// What a graph holds that only a height difference separated before `ROAD-99`: every crossing in
// plan of two live road edges with no node there, by class pair, with the height between the two
// profiles at the crossing; and the junctions standing on a bridge's deck (a node with two bridge
// arms and a standard one, made by a ground road the bridge's deck pinned). Each edge's
// candidates come from the edge R-tree: `O(E log E + crossings)`.
#[derive(Default)]
struct HeightOnlyScan {
    // (class, class) -> heights between the profiles at each crossing.
    crossings: BTreeMap<(u8, u8), Vec<f32>>,
    junctions_on_decks: usize,
}

fn scan_height_only(graph: &RegionGraph) -> HeightOnlyScan {
    let mut scan = HeightOnlyScan::default();
    let class_id = |class: EdgeClass| class as u8;
    for (id, edge) in graph.edges().iter().enumerate() {
        if !live_surface(edge) {
            continue;
        }
        let a = profile(edge);
        let (lo, hi) = a.iter().fold(
            (Vector3::splat(f32::MAX), Vector3::splat(f32::MIN)),
            |(lo, hi), p| {
                (
                    Vector3::new(lo.x.min(p.x), 0.0, lo.z.min(p.z)),
                    Vector3::new(hi.x.max(p.x), 0.0, hi.z.max(p.z)),
                )
            },
        );
        let mut others = graph.get_edges_near_aabb(lo, hi);
        others.sort_unstable();
        let ends =
            |edge: &Edge| [edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node));
        for other_id in others.into_iter().filter(|&other| other > id) {
            let other = graph.edge(other_id);
            if !live_surface(other) {
                continue;
            }
            let shared: Vec<Vector3> = ends(edge)
                .into_iter()
                .filter(|node| ends(other).contains(node))
                .map(|node| graph.node(node).pos)
                .collect();
            let b = profile(other);
            let mut seen: Vec<Vector3> = Vec::new();
            for sa in a.windows(2) {
                for sb in b.windows(2) {
                    let Some((t, u)) = find_intersection_2d(sa[0], sa[1], sb[0], sb[1]) else {
                        continue;
                    };
                    let (pa, pb) = (sa[0].lerp(sa[1], t), sb[0].lerp(sb[1], u));
                    let near = |p: &Vector3, q: Vector3| (p.x - q.x).hypot(p.z - q.z) < 1.0;
                    if shared.iter().any(|node| near(node, pa)) || seen.iter().any(|q| near(q, pa))
                    {
                        continue;
                    }
                    seen.push(pa);
                    let pair = (class_id(edge.class), class_id(other.class));
                    let key = (pair.0.min(pair.1), pair.0.max(pair.1));
                    scan.crossings
                        .entry(key)
                        .or_default()
                        .push((pa.y - pb.y).abs());
                }
            }
        }
    }
    for node in 0..graph.node_adjacency_count() as u32 {
        if graph.get_valid_node(node) != node {
            continue;
        }
        let arms = graph
            .node_adjacency(node)
            .iter()
            .map(|&edge| graph.edge(edge))
            .filter(|edge| live_surface(edge));
        let bridges = arms
            .clone()
            .filter(|edge| edge.class == EdgeClass::Bridge)
            .count();
        let ground = arms
            .filter(|edge| edge.class == EdgeClass::Standard)
            .count();
        if bridges >= 2 && ground >= 1 {
            scan.junctions_on_decks += 1;
        }
    }
    scan
}

fn scan_report(name: &str, graph: &RegionGraph) -> String {
    let scan = scan_height_only(graph);
    let live = graph.edges().iter().filter(|edge| live_surface(edge));
    let count = |class: EdgeClass| live.clone().filter(|edge| edge.class == class).count();
    let mut line = format!(
        "ROAD99 SCAN {name}: edges {} bridges {} tunnels {}; junctions on a bridge deck {}",
        live.clone().count(),
        count(EdgeClass::Bridge),
        count(EdgeClass::Tunnel),
        scan.junctions_on_decks,
    );
    let names = ["standard", "bridge", "tunnel"];
    for ((a, b), heights) in &scan.crossings {
        let within = |lo: f32, hi: f32| heights.iter().filter(|&&h| h >= lo && h < hi).count();
        let (least, most) = heights
            .iter()
            .fold((f32::INFINITY, 0.0_f32), |(lo, hi), &h| {
                (lo.min(h), hi.max(h))
            });
        line += &format!(
            "; {} x {} crossings without a node {} (under 4.5 m {}, 4.5-4.85 m {}, 4.85 m or more \
             {}; least {least:.2} m, most {most:.2} m)",
            names[*a as usize],
            names[*b as usize],
            heights.len(),
            within(0.0, 4.5),
            within(4.5, 4.85),
            within(4.85, f32::INFINITY),
        );
    }
    if scan.crossings.is_empty() {
        line += "; no crossing without a node";
    }
    line
}

// Before `ROAD-99` lands (its plan, step 1): what the reference save and the user's dumps
// (`METRUM_ROAD99_DUMPS`, `;`-separated copies at the current save format) hold that only height
// separated, by class pair and the height between the profiles. Nothing repairs these on load;
// they change only when one of their roads is redrawn. Read-only; ignored because the dumps are
// not in the repository.
#[test]
#[ignore = "scan: METRUM_ROAD99_DUMPS"]
fn scan_crossings_separated_by_height_alone() {
    let mut sources = vec![(
        "kuopio".to_owned(),
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
        )
        .to_owned(),
    )];
    sources.extend(
        std::env::var("METRUM_ROAD99_DUMPS")
            .unwrap_or_default()
            .split(';')
            .filter(|path| !path.is_empty())
            .enumerate()
            .map(|(index, path)| (format!("dump {index}"), path.to_owned())),
    );
    for (name, path) in sources {
        let mut core = test_core();
        core.load_game_internal(&path).expect("load the save");
        println!("{}", scan_report(&name, &core.region_graph));
    }
}

