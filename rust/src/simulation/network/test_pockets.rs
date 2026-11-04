// SPDX-License-Identifier: GPL-2.0-only

//! Turn pockets (`ROAD-95`, round R5a1): the asymmetric profile of a right (kerb) pocket, the lane
//! offsets that keep the through lanes in place, the D4 taper at the lane transition, the
//! connectors and lane use across it and at the junction, and the plain lane-count change (D16).
//! The fixtures (`roadbed::fixtures::pocket_junction`) are right-hand traffic.

use super::graph::{Edge, RegionGraph, RoadPocket};
use super::lanes::rebuild::vehicle_lane_offset;
use super::lanes::{LaneSystem, LaneType, approach_arrow_masks, approach_group};
use super::roadbed::fixtures::{
    one_way_right_pocket, pocket_junction, road_edge, straight_1x1_to_2x2, t_right_pocket,
    two_plus_one_right_pocket,
};
use super::roadbed::{
    BandKind, EdgeEnd, JunctionLayout, NodeArms, RoadProfile, Side, crossing_arm_eligible,
};
use super::types::NodeType;
use crate::config;
use godot::prelude::Vector3;
use std::collections::BTreeSet;

const LEFT: u8 = 1;
const STRAIGHT: u8 = 2;
const RIGHT: u8 = 4;

// The pocket edge of the fixtures and its two nodes.
const POCKET_EDGE: usize = 1;
const JUNCTION: u32 = 0;
const TRANSITION: u32 = 1;

// A road of `fwd` + `bkw` lanes with sidewalks along +x, with `pocket` (lanes added as the pocket
// edit adds them).
fn road(fwd: u8, bkw: u8, pocket: Option<RoadPocket>) -> Edge {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let b = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let mut edge = road_edge(
        a,
        b,
        vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
        7.0,
    );
    edge.fwd_lanes = fwd;
    edge.bkw_lanes = bkw;
    edge.width = Edge::cross_section_width_m(fwd, bkw, None);
    let id = graph.add_edge(edge);
    graph.set_edge_pocket(id, pocket);
    graph.edge(id).clone()
}

fn right_at(end: EdgeEnd) -> Option<RoadPocket> {
    Some(RoadPocket {
        end,
        left: false,
        right: true,
    })
}

fn layout_of(graph: &RegionGraph, node: u32) -> JunctionLayout {
    NodeArms::collect(graph, node)
        .expect("the node is laid out")
        .layout()
        .expect("the layout fits")
}

fn lanes_of(graph: &mut RegionGraph) -> LaneSystem {
    let mut lanes = LaneSystem::new();
    lanes.rebuild(graph);
    lanes
}

// The vehicle connections out of `edge`'s lanes that approach `node`: `(from lane, to edge, to
// lane)`, each through its connector, or directly for a pass-through successor.
fn connections(
    graph: &RegionGraph,
    lanes: &LaneSystem,
    edge: usize,
    node: u32,
) -> BTreeSet<(i8, usize, i8)> {
    let road = graph.edge(edge);
    let mut pairs = BTreeSet::new();
    for &id in &lanes.edge_lanes[&edge] {
        let lane = &lanes.lanes[id];
        let approaches = if lane.is_fwd {
            road.end_node == node
        } else {
            road.start_node == node
        };
        if lane.lane_type != LaneType::Vehicle || !approaches {
            continue;
        }
        for &next in &lane.next_lanes {
            let target = if lanes.lanes[next].edge_id == usize::MAX {
                &lanes.lanes[lanes.lanes[next].next_lanes[0]]
            } else {
                &lanes.lanes[next]
            };
            pairs.insert((lane.lane_idx, target.edge_id, target.lane_idx));
        }
    }
    pairs
}

fn arrows(graph: &RegionGraph, edge: usize, node: u32) -> Vec<(i8, u8)> {
    let mut painted = Vec::new();
    approach_arrow_masks(graph, edge, node, |lane, mask| painted.push((lane, mask)));
    painted
}

fn carriageway(profile: &RoadProfile) -> (f32, f32) {
    let edges: Vec<_> = profile
        .bands()
        .iter()
        .filter(|band| band.kind == BandKind::Carriageway)
        .collect();
    (edges[0].start_m, edges[edges.len() - 1].end_m)
}

// The profile table: a right pocket moves the kerb on its lanes' right by one lane and keeps the
// other side bit for bit; the sidewalk lanes follow their own side. Backward lanes mirror it.
#[test]
fn a_right_pocket_moves_only_its_own_kerb() {
    // (base fwd, bkw, the pocket edge's carriageway edges).
    for (fwd, bkw, expected) in [
        (1, 1, (-3.5, 7.0)),
        (1, 0, (-1.75, 5.25)),
        (2, 1, (-5.25, 8.75)),
    ] {
        let base = road(fwd, bkw, None);
        let pocket = road(fwd, bkw, right_at(EdgeEnd::End));
        assert_eq!(pocket.fwd_lanes, fwd + 1);
        assert_eq!(pocket.width, base.width + config::LANE_WIDTH);
        assert_eq!(pocket.kerb_shift_m(), config::LANE_WIDTH * 0.5);
        let (base_profile, profile) =
            (RoadProfile::for_edge(&base), RoadProfile::for_edge(&pocket));
        assert_eq!(carriageway(&profile), expected, "{fwd}+{bkw}");
        assert_eq!(base_profile.bands().len(), profile.bands().len());
        for (base_band, band) in base_profile.bands().iter().zip(profile.bands()) {
            assert_eq!(base_band.kind, band.kind);
            assert_eq!(base_band.level, band.level);
            if band.end_m <= 0.0 {
                // The far side: bitwise.
                assert_eq!(band.start_m.to_bits(), base_band.start_m.to_bits());
                assert_eq!(band.end_m.to_bits(), base_band.end_m.to_bits());
            } else {
                // The pocket side, one lane out (the split at the centreline stays).
                let start = if band.start_m == 0.0 {
                    0.0
                } else {
                    base_band.start_m + config::LANE_WIDTH
                };
                assert!((band.start_m - start).abs() < 1e-5, "{band:?}");
                assert!((band.end_m - (base_band.end_m + config::LANE_WIDTH)).abs() < 1e-5);
            }
        }
        let (base_left, base_right) = (
            base_profile.side_half_widths_m(false),
            base_profile.side_half_widths_m(true),
        );
        assert_eq!(profile.side_half_widths_m(false), base_left);
        let right = profile.side_half_widths_m(true);
        assert!((right.0 - base_right.0 - config::LANE_WIDTH).abs() < 1e-5);
        assert!((right.1 - base_right.1 - config::LANE_WIDTH).abs() < 1e-5);
        assert_eq!(profile.half_width_m(), right.1);
        // The sidewalk lanes: 100 on the left keeps its offset, -100 moves out with its kerb.
        let walk = super::lanes::rebuild::sidewalk_offset;
        assert_eq!(walk(&pocket, 100).to_bits(), walk(&base, 100).to_bits());
        assert!((walk(&pocket, -100) - walk(&base, -100) - config::LANE_WIDTH).abs() < 1e-5);

        // The same pocket on the backward lanes is the mirror image.
        if bkw > 0 {
            let mirrored = road(bkw, fwd, right_at(EdgeEnd::Start));
            let mirror = RoadProfile::for_edge(&mirrored);
            assert_eq!(mirrored.kerb_shift_m(), -config::LANE_WIDTH * 0.5);
            for (band, reflected) in profile.bands().iter().zip(mirror.bands().iter().rev()) {
                assert_eq!(band.kind, reflected.kind);
                assert_eq!(band.start_m, -reflected.end_m);
                assert_eq!(band.end_m, -reflected.start_m);
            }
        }
    }
    // A road without a pocket has no shift.
    assert_eq!(road(2, 2, None).kerb_shift_m(), 0.0);
}

// (base fwd, bkw, pocket, the pocket edge's lane offsets by lane index).
type OffsetCase = (u8, u8, Option<RoadPocket>, &'static [(i8, f32)]);

// The lane offsets of section 2.1 of the plan: the base road's lanes keep their offsets on the
// pocket edge, the pocket lane lies one lane beyond its kerb-side neighbour. The one-way left
// kerb pocket (D15) is the mirror.
#[test]
fn pocket_lanes_leave_the_through_lanes_in_place() {
    let left_at_end = Some(RoadPocket {
        end: EdgeEnd::End,
        left: true,
        right: false,
    });
    let cases: [OffsetCase; 4] = [
        (
            1,
            1,
            right_at(EdgeEnd::End),
            &[(-1, -1.75), (0, 1.75), (1, 5.25)],
        ),
        (1, 0, right_at(EdgeEnd::End), &[(0, 0.0), (1, 3.5)]),
        (
            2,
            1,
            right_at(EdgeEnd::End),
            &[(-1, -3.5), (0, 0.0), (1, 3.5), (2, 7.0)],
        ),
        (1, 0, left_at_end, &[(0, -3.5), (1, 0.0)]),
    ];
    for (fwd, bkw, pocket, offsets) in cases {
        let base = road(fwd, bkw, None);
        let edge = road(fwd, bkw, pocket);
        assert_eq!(edge.validate_pocket(), Ok(()));
        for &(lane, offset) in offsets {
            assert_eq!(
                vehicle_lane_offset(&edge, lane),
                offset,
                "{fwd}+{bkw} lane {lane}"
            );
        }
        // Each base lane has a lane of the pocket edge at its exact offset.
        for lane in (-(bkw as i8)..0).chain(0..fwd as i8) {
            let offset = vehicle_lane_offset(&base, lane).to_bits();
            assert!(
                offsets.iter().any(|&(_, own)| own.to_bits() == offset),
                "{fwd}+{bkw}: base lane {lane}"
            );
        }
    }
    // The left kerb pocket moves the left kerb: carriageway -5.25 to 1.75.
    let one_way_left = road(1, 0, left_at_end);
    assert_eq!(
        carriageway(&RoadProfile::for_edge(&one_way_left)),
        (-5.25, 1.75)
    );
}

// Validation (`Edge::validate_pocket`) shared by loads and the pocket edit.
#[test]
fn pocket_validation_refuses_what_no_pocket_edit_builds() {
    let mut edge = road(1, 1, right_at(EdgeEnd::End));
    assert_eq!(edge.validate_pocket(), Ok(()));
    // An inner (left) kerb pocket needs a one-way road; a median pocket is round R5b1's.
    edge.pocket = Some(RoadPocket {
        end: EdgeEnd::End,
        left: true,
        right: false,
    });
    assert_eq!(edge.validate_pocket(), Err("pocket_requires_one_way"));
    edge.pocket = Some(RoadPocket {
        end: EdgeEnd::End,
        left: false,
        right: false,
    });
    assert_eq!(edge.validate_pocket(), Err("invalid_pocket_dimensions"));
    // Every lane of the direction a pocket lane: no through lane.
    let mut edge = road(1, 1, right_at(EdgeEnd::End));
    edge.fwd_lanes = 1;
    assert_eq!(edge.validate_pocket(), Err("invalid_pocket_dimensions"));
    let mut edge = road(4, 1, None);
    edge.fwd_lanes = 5;
    edge.pocket = right_at(EdgeEnd::End);
    assert_eq!(edge.validate_pocket(), Err("pocket_lane_limit"));
    let mut edge = road(1, 1, right_at(EdgeEnd::End));
    edge.class = super::types::EdgeClass::Bridge;
    assert_eq!(edge.validate_pocket(), Err("pocket_requires_surface_road"));
    let mut edge = road(1, 1, right_at(EdgeEnd::End));
    edge.allowed_types = super::types::TransitFlags::FOOT;
    assert_eq!(edge.validate_pocket(), Err("invalid_road"));
    // Codes round-trip.
    for pocket in [None, right_at(EdgeEnd::Start), right_at(EdgeEnd::End)] {
        let (end, roles) = RoadPocket::codes(pocket);
        assert_eq!(RoadPocket::from_codes(end, roles), Ok(pocket));
    }
    for (end, roles) in [(0, 2), (3, 2), (1, 0), (2, 4)] {
        assert!(
            RoadPocket::from_codes(end, roles).is_err(),
            "({end}, {roles})"
        );
    }
}

// The transition node of a right pocket is laid out (two profiles); its straight sector on the
// pocket side tapers every boundary (D4: a 7 m width change, 21 m, half on each arm) and the
// other side keeps its offsets. No crossing there (D6), and its arms cannot be toggled.
#[test]
fn the_transition_tapers_the_pocket_kerb_only_and_never_crosses() {
    for graph in [
        t_right_pocket(),
        one_way_right_pocket(),
        two_plus_one_right_pocket(),
    ] {
        assert!(graph.is_lane_transition(TRANSITION));
        let layout = layout_of(&graph, TRANSITION);
        assert_eq!(layout.arms.len(), 2);
        let mut tapered_sectors = 0;
        for sector in &layout.sectors {
            let boundaries = &layout.boundaries[sector.boundaries.clone()];
            let tapered = boundaries.iter().filter(|b| b.taper.is_some()).count();
            assert!(tapered == 0 || tapered == boundaries.len(), "{tapered}");
            if tapered == 0 {
                continue;
            }
            tapered_sectors += 1;
            for boundary in boundaries {
                let taper = boundary.taper.unwrap();
                assert_eq!(taper.station_m, 10.5);
                // Linear from the mean at the sector to each arm's own offset.
                let (start_m, start) = boundary.run_start(Side::Left);
                let half = boundary.run_offset_m(Side::Left, 0.5 * (start_m + taper.station_m));
                assert!((half - 0.5 * (start + taper.from_offset_m)).abs() < 1e-9);
                let step = (taper.from_offset_m + taper.to_offset_m).abs();
                assert!(
                    (step - f64::from(config::LANE_WIDTH)).abs() < 1e-5,
                    "{step}"
                );
            }
        }
        assert_eq!(tapered_sectors, 1, "one side tapers");
        for arm in &layout.arms {
            assert_eq!(arm.crosswalk, None, "no crossing at a transition");
            assert!(
                (arm.mouth_station_m - 10.5).abs() < 1e-9,
                "{}",
                arm.mouth_station_m
            );
            assert!(!crossing_arm_eligible(&graph, TRANSITION, arm.key.edge));
        }
        // The junction still lays out with the wider arm.
        layout_of(&graph, JUNCTION);
    }
    // Without the type the same node would cross by default.
    let mut graph = t_right_pocket();
    graph.set_node_type(TRANSITION, NodeType::Junction);
    assert!(
        layout_of(&graph, TRANSITION)
            .arms
            .iter()
            .any(|arm| arm.crosswalk.is_some())
    );
}

// Connectors across the transition (D5): the through lanes keep their place and the lane beside
// the pocket also forks into it; the leaving direction is 1:1. Every pair is a connector (the
// mouths lie 21 m apart): the through lane's straight, the fork a lateral shift.
#[test]
fn the_transition_forks_into_the_pocket_and_keeps_the_through_lanes() {
    for (mut graph, expected, leaving) in [
        (t_right_pocket(), vec![(0, 0), (0, 1)], vec![(-1, -1)]),
        (one_way_right_pocket(), vec![(0, 0), (0, 1)], vec![]),
        (
            two_plus_one_right_pocket(),
            vec![(0, 0), (1, 1), (1, 2)],
            vec![(-1, -1)],
        ),
    ] {
        let lanes = lanes_of(&mut graph);
        let entering: Vec<_> = connections(&graph, &lanes, 0, TRANSITION)
            .into_iter()
            .map(|(from, to_edge, to)| {
                assert_eq!(to_edge, POCKET_EDGE);
                (from, to)
            })
            .collect();
        assert_eq!(entering, expected);
        let left: Vec<_> = connections(&graph, &lanes, POCKET_EDGE, TRANSITION)
            .into_iter()
            .map(|(from, to_edge, to)| {
                assert_eq!(to_edge, 0);
                (from, to)
            })
            .collect();
        assert_eq!(left, leaving);
        // Every connector at the transition runs along the road (+x); the through ones keep
        // their lateral offset exactly, the fork shifts by one lane.
        for &id in &lanes.node_lanes[&(TRANSITION as usize)] {
            let lane = &lanes.lanes[id];
            if lane.lane_type != LaneType::Vehicle {
                continue;
            }
            let (first, last) = (lane.geometry[0], *lane.geometry.last().unwrap());
            assert!((last.x - first.x).abs() > 15.0, "the mouths lie apart");
            let shift = (last.z - first.z).abs();
            assert!(
                shift < 1e-4 || (shift - config::LANE_WIDTH).abs() < 1e-3,
                "{shift}"
            );
            if shift < 1e-4 {
                assert!(lane.geometry.iter().all(|p| (p.z - first.z).abs() < 1e-4));
            }
        }
        // The transition offers one movement, but the pocket edge is too short to change lanes
        // on: a base road with more than one lane towards it prepares the junction's turn
        // before the fork (`transition_needs_lane_choice`, round R5a3); its leaving lanes never.
        let several = graph.edge(0).fwd_lanes > 1;
        for &id in &lanes.edge_lanes[&0] {
            let lane = &lanes.lanes[id];
            let approaches = lane.lane_type == LaneType::Vehicle && lane.is_fwd;
            assert_eq!(lanes.needs_turn_preparation(id), approaches && several);
        }
    }
}

// The pocket mask at the junction (D4) for connectors, the entry-lane walk and the arrows: the
// pocket lane serves its turn alone, the other lanes the permissive default minus that turn; if
// the junction stops offering the turn, the base rule.
#[test]
fn a_pocket_lane_serves_its_turn_alone() {
    // A four-way: 1+1 with a right pocket paints L+S / R.
    let mut graph = pocket_junction(1, 1, false, true, true);
    assert_eq!(
        arrows(&graph, POCKET_EDGE, JUNCTION),
        [(0, LEFT | STRAIGHT), (1, RIGHT)]
    );
    let lanes = lanes_of(&mut graph);
    let pairs = connections(&graph, &lanes, POCKET_EDGE, JUNCTION);
    // Edge 2 east (straight), 3 south (right), 4 north (left); lane 1 is the pocket.
    assert!(
        pairs.iter().all(|&(from, to, _)| (from == 1) == (to == 3)),
        "{pairs:?}"
    );
    assert!(pairs.iter().any(|&(from, to, _)| from == 0 && to == 4));
    assert!(pairs.iter().any(|&(from, to, _)| from == 0 && to == 2));
    for &id in &lanes.edge_lanes[&POCKET_EDGE] {
        let lane = &lanes.lanes[id];
        if lane.lane_type == LaneType::Vehicle && lane.is_fwd {
            assert!(lanes.needs_turn_preparation(id), "solid zone and window");
        }
    }
    // The entry-lane groups: straight and left from the base lane, right from the pocket.
    assert_eq!(
        approach_group(&graph, POCKET_EDGE, JUNCTION, 2),
        Some((0, 1, true))
    );
    assert_eq!(
        approach_group(&graph, POCKET_EDGE, JUNCTION, 4),
        Some((0, 1, true))
    );
    assert_eq!(
        approach_group(&graph, POCKET_EDGE, JUNCTION, 3),
        Some((1, 1, true))
    );

    // 2+2 with one: L+S / S / R.
    let graph = pocket_junction(2, 2, false, true, true);
    assert_eq!(
        arrows(&graph, POCKET_EDGE, JUNCTION),
        [(0, LEFT | STRAIGHT), (1, STRAIGHT), (2, RIGHT)]
    );
    // A T without the left turn: S / R.
    let graph = t_right_pocket();
    assert_eq!(
        arrows(&graph, POCKET_EDGE, JUNCTION),
        [(0, STRAIGHT), (1, RIGHT)]
    );
    // The one-way left kerb pocket (D15): L / S+R.
    let graph = pocket_junction(1, 0, true, false, true);
    assert_eq!(
        arrows(&graph, POCKET_EDGE, JUNCTION),
        [(0, LEFT), (1, STRAIGHT | RIGHT)]
    );
    // The right turn's arm removed: the base rule, both lanes straight, the inner one also left.
    let mut graph = pocket_junction(1, 1, false, true, true);
    graph.delete_edge_locally(3);
    assert_eq!(
        arrows(&graph, POCKET_EDGE, JUNCTION),
        [(0, LEFT | STRAIGHT), (1, STRAIGHT)]
    );
}

// D16: a plain lane-count change has no transition. Today's behaviour, pinned: the straight node
// is laid out and tapers both sides, the forward lane fans into both lanes ahead and the two
// backward lanes merge into one, and the node keeps D8's one default zebra.
#[test]
fn straight_1x1_to_2x2_tapers_both_sides_fans_and_merges() {
    let mut graph = straight_1x1_to_2x2();
    let node = 0;
    assert!(!graph.is_lane_transition(node));
    let layout = layout_of(&graph, node);
    for sector in &layout.sectors {
        assert!(
            layout.boundaries[sector.boundaries.clone()]
                .iter()
                .all(|boundary| boundary.taper.is_some_and(|taper| taper.station_m == 10.5)),
            "both sides taper"
        );
    }
    assert_eq!(
        layout
            .arms
            .iter()
            .filter(|arm| arm.crosswalk.is_some())
            .count(),
        1,
        "one zebra"
    );
    let lanes = lanes_of(&mut graph);
    assert_eq!(
        connections(&graph, &lanes, 0, node),
        BTreeSet::from([(0, 1, 0), (0, 1, 1)]),
        "1 -> 2 fan"
    );
    assert_eq!(
        connections(&graph, &lanes, 1, node),
        BTreeSet::from([(-1, 0, -1), (-2, 0, -1)]),
        "2 -> 1 merge"
    );
}

// A bulldozed pocket pair leaves a road end, not a hidden node.
#[test]
fn a_transition_left_with_one_arm_becomes_a_junction() {
    let mut graph = t_right_pocket();
    graph.reset_stale_lane_transitions([TRANSITION]);
    assert_eq!(graph.node(TRANSITION).node_type, NodeType::LaneTransition);
    graph.delete_edge_locally(0);
    assert!(!graph.is_lane_transition(TRANSITION));
    graph.reset_stale_lane_transitions([TRANSITION]);
    assert_eq!(graph.node(TRANSITION).node_type, NodeType::Junction);
}

// Ported and rewritten from the fork's `pocket_keeps_opposite_kerb_and_two_through_lanes_aligned`
// (its right half; the left pocket inside the median is round R5b1's): a 2+2 road with a 5 m
// grass median gains a right pocket on its forward lanes. The opposite kerb, the median and both
// through lanes keep their place across the transition; only the right kerb moves, one lane out.
#[test]
fn pocket_keeps_opposite_kerb_and_two_through_lanes_aligned() {
    let mut graph = pocket_junction(2, 2, false, true, true);
    let grass = super::graph::RoadMedian::from_tool(2, 5.0).unwrap();
    for edge in [0, POCKET_EDGE] {
        let edge = graph.edge_mut(edge);
        edge.median = grass;
        edge.width = Edge::cross_section_width_m(edge.fwd_lanes, edge.bkw_lanes, grass);
        assert_eq!(edge.validate_pocket(), Ok(()));
    }
    super::roadbed::fixtures::finalize(&mut graph);
    let (base, pocket) = (graph.edge(0).clone(), graph.edge(POCKET_EDGE).clone());
    let (base, pocket) = (&base, &pocket);
    assert_eq!((pocket.fwd_lanes, pocket.width), (3, 22.5));
    assert_eq!(base.lane_group_offset(), pocket.lane_group_offset());
    let (base_profile, profile) = (RoadProfile::for_edge(base), RoadProfile::for_edge(pocket));
    assert_eq!(carriageway(&base_profile), (-9.5, 9.5));
    assert_eq!(carriageway(&profile), (-9.5, 13.0));
    for lane in [-2, -1, 0, 1] {
        assert_eq!(
            vehicle_lane_offset(pocket, lane).to_bits(),
            vehicle_lane_offset(base, lane).to_bits()
        );
    }
    assert_eq!(vehicle_lane_offset(pocket, 1), 7.75);
    assert_eq!(vehicle_lane_offset(pocket, 2), 11.25);
    // The built lanes: along the straight road their lateral position is their offset.
    let lanes = lanes_of(&mut graph);
    for &id in &lanes.edge_lanes[&POCKET_EDGE] {
        let lane = &lanes.lanes[id];
        if lane.lane_type != LaneType::Vehicle {
            continue;
        }
        let offset = vehicle_lane_offset(pocket, lane.lane_idx);
        assert!(lane.geometry.iter().all(|p| (p.z - offset).abs() < 1e-4));
    }
    layout_of(&graph, TRANSITION);
    layout_of(&graph, JUNCTION);
}
