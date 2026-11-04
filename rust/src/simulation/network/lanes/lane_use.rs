// SPDX-License-Identifier: GPL-2.0-only

//! Default junction lane use (`ROAD-94`): which movements (left, straight, right) each approach
//! lane of an open node serves, and which target lanes each movement reaches. Graph-only and
//! evaluated only while junction connectors are rebuilt; a node with a vehicle lane whitelist
//! bypasses it entirely.

use super::super::graph::RegionGraph;
use crate::config;
use godot::prelude::*;

const LEFT: u8 = 1;
const STRAIGHT: u8 = 2;
const RIGHT: u8 = 4;

// Tangents point out from the node. Physical centerlines keep classification independent of
// lane offsets, clipping, median width and the order in which lanes are rebuilt.
fn outward(graph: &RegionGraph, edge: usize, node: u32) -> Vector2 {
    let edge = graph.edge(edge);
    let points = &edge.physical_geometry;
    let direction = if edge.start_node == node {
        points
            .windows(2)
            .map(|p| p[1] - p[0])
            .find(|d| d.x * d.x + d.z * d.z > 1e-8)
    } else {
        points
            .windows(2)
            .rev()
            .map(|p| p[0] - p[1])
            .find(|d| d.x * d.x + d.z * d.z > 1e-8)
    }
    .unwrap_or_else(|| {
        let other = if edge.start_node == node {
            edge.end_node
        } else {
            edge.start_node
        };
        graph.node(other).pos - graph.node(node).pos
    });
    Vector2::new(direction.x, direction.z).normalized()
}

fn movement(incoming: Vector2, outgoing: Vector2) -> u8 {
    // The inclusive 45-degree straight sector has a fixed boundary, with no best-arm heuristic.
    if incoming.dot(outgoing) >= std::f32::consts::FRAC_1_SQRT_2 {
        STRAIGHT
    } else if incoming.cross(outgoing) < 0.0 {
        LEFT
    } else {
        RIGHT
    }
}

/// Rank of lane `index` among `count` lanes of one direction: 0 is the leftmost, the innermost
/// lane when driving on the right (forward lanes `0, 1, ..`, backward lanes `-1, -2, ..`).
pub(crate) fn lane_rank(index: i8, count: usize) -> usize {
    let center_rank = if index >= 0 {
        index as usize
    } else {
        (-i16::from(index) - 1) as usize
    };
    if config::DRIVE_ON_LEFT {
        count - 1 - center_rank
    } else {
        center_rank
    }
}

// The movements approach lane `rank` of `count` serves (user decision 2026-11-04, permissive
// default lane use: turns are restricted with the junction inspection tool's whitelist, not by
// default). With straight ahead available every lane goes straight, the innermost lane also turns
// left and the outermost also right, so no lane's movement crosses another's; without it (a T's
// stem) the inner half turns left and the outer half right.
fn lane_mask(rank: usize, count: usize, available: u8) -> u8 {
    if count == 1 || available.count_ones() <= 1 {
        return available;
    }
    if available & STRAIGHT == 0 {
        return if rank < count.div_ceil(2) {
            LEFT
        } else {
            RIGHT
        };
    }
    let mut mask = STRAIGHT;
    if rank == 0 {
        mask |= available & LEFT;
    }
    if rank == count - 1 {
        mask |= available & RIGHT;
    }
    mask
}

// Overlapping integer rank intervals produce monotone fans/merges for unequal lane counts.
// Left/right turns use the corresponding outer target group; straight uses the whole group.
fn target_matches(rank: usize, count: usize, target: usize, targets: usize, turn: u8) -> bool {
    // A single-lane turn group may enter any lane of its target group (D10 relaxed for one
    // lane, `TRAFFIC-23`): the car's entry then picks the lane its route needs next, since a
    // short multi-lane link may leave no room to change lanes after the turn.
    let span = if turn == STRAIGHT || count == 1 {
        targets
    } else {
        count.min(targets)
    };
    let start = if turn == RIGHT { targets - span } else { 0 };
    target >= start
        && target < start + span
        && (rank + 1) * span > (target - start) * count
        && (target - start + 1) * count > rank * span
}

/// The default lane use of one approach to an open node: the movements its arms offer and the
/// approach's lane count towards the node. Rebuild-only; whitelists bypass it entirely.
pub(super) struct DefaultLaneUse {
    incoming: Vector2,
    available: u8,
    count: usize,
    node: u32,
}

impl DefaultLaneUse {
    /// Classifies the node's other live arms that have lanes leaving it, from node-local
    /// adjacency only, for the lanes of `edge` that approach `node`. `O(degree)`.
    pub(super) fn new(graph: &RegionGraph, edge: usize, node: u32) -> Self {
        let incoming = -outward(graph, edge, node);
        let approach = graph.edge(edge);
        let count = usize::from(if approach.end_node == node {
            approach.fwd_lanes
        } else {
            approach.bkw_lanes
        });
        let mut available = 0;
        for &other in graph.node_adjacency(node) {
            let arm = graph.edge(other);
            let outgoing = if arm.start_node == node {
                arm.fwd_lanes
            } else {
                arm.bkw_lanes
            };
            if other != edge && !arm.deleted && outgoing > 0 {
                available |= movement(incoming, outward(graph, other, node));
            }
        }
        Self {
            incoming,
            available,
            count,
            node,
        }
    }

    /// Whether the approach's lanes serve different movements, so a car must pick its lane
    /// before the node: more than one lane and more than one movement.
    pub(super) fn needs_lane_choice(&self) -> bool {
        self.count > 1 && self.available.count_ones() > 1
    }

    /// Whether approach lane `from` may turn into lane `to` of arm `edge` (not the approach's
    /// own edge: U-turns are the caller's rule). `O(lanes)`, allocation-free.
    pub(super) fn allows(&self, graph: &RegionGraph, from: i8, edge: usize, to: i8) -> bool {
        let turn = movement(self.incoming, outward(graph, edge, self.node));
        let rank = lane_rank(from, self.count);
        if lane_mask(rank, self.count, self.available) & turn == 0 {
            return false;
        }
        let mut group_rank = 0;
        let mut group_count = 0;
        for candidate in 0..self.count {
            if lane_mask(candidate, self.count, self.available) & turn != 0 {
                group_rank += usize::from(candidate < rank);
                group_count += 1;
            }
        }
        let target = graph.edge(edge);
        let target_count = usize::from(if target.start_node == self.node {
            target.fwd_lanes
        } else {
            target.bkw_lanes
        });
        target_matches(
            group_rank,
            group_count,
            lane_rank(to, target_count),
            target_count,
            turn,
        )
    }
}

/// The lanes of `edge` approaching `node` whose default lane use makes the movement onto
/// `out_edge`: their ranks as `(first, len)` (one contiguous run, inner first) and whether that
/// run maps 1:1 onto `out_edge`'s lanes (as wide as the exit: a car keeps its rank, plus `first`,
/// across the node). `None` at a whitelisted node, for a U-turn, or when no lane makes the
/// movement. O(degree + the node's lane connections), no allocation.
pub(crate) fn approach_group(
    graph: &RegionGraph,
    edge: usize,
    node: u32,
    out_edge: usize,
) -> Option<(usize, usize, bool)> {
    if out_edge == edge
        || super::vehicle_junctions::node_whitelists_vehicle_turns(graph, node as usize)
    {
        return None;
    }
    let policy = DefaultLaneUse::new(graph, edge, node);
    let turn = movement(policy.incoming, outward(graph, out_edge, node));
    let mut first = usize::MAX;
    let mut len = 0;
    for rank in 0..policy.count {
        if lane_mask(rank, policy.count, policy.available) & turn != 0 {
            if first == usize::MAX {
                first = rank;
            }
            len += 1;
        }
    }
    if len == 0 {
        return None;
    }
    let target = graph.edge(out_edge);
    let targets = usize::from(if target.start_node == node {
        target.fwd_lanes
    } else {
        target.bkw_lanes
    });
    Some((first, len, len == targets))
}

/// Whether the lanes of `edge` that approach `node` must pick their lane before it: more than one
/// such lane, and the node whitelists its vehicle turns or its default lane use gives the lanes
/// different movements. The connector builder writes the same rule into
/// `LaneSystem::needs_turn_preparation` per lane; the markings read it here, from the graph alone,
/// to paint the solid approach zone (`APPROACH_SOLID_LENGTH_M`), so a preview paints what its
/// commit paints. O(degree + the node's lane connections), no allocation.
pub(crate) fn approach_needs_lane_choice(graph: &RegionGraph, edge: usize, node: u32) -> bool {
    let approach = graph.edge(edge);
    let count = if approach.end_node == node {
        approach.fwd_lanes
    } else {
        approach.bkw_lanes
    };
    count > 1
        && (super::vehicle_junctions::node_whitelists_vehicle_turns(graph, node as usize)
            || DefaultLaneUse::new(graph, edge, node).needs_lane_choice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_use_shared_and_dedicated_one_to_four_lanes() {
        for (count, expected) in [
            (1, vec![7]),
            (2, vec![3, 6]),
            // User decision: permissive default lane use (every lane goes straight).
            (3, vec![3, 2, 6]),
            (4, vec![3, 2, 2, 6]),
        ] {
            assert_eq!(
                (0..count)
                    .map(|rank| lane_mask(rank, count, 7))
                    .collect::<Vec<_>>(),
                expected
            );
            for available in 1..=7 {
                let masks: Vec<_> = (0..count)
                    .map(|rank| lane_mask(rank, count, available))
                    .collect();
                assert!(
                    masks
                        .iter()
                        .all(|&mask| mask != 0 && mask & !available == 0)
                );
                assert_eq!(masks.iter().fold(0, |a, b| a | b), available);
                // No lane's movement crosses another's: left turns only inside of right turns.
                for (a, &inner) in masks.iter().enumerate() {
                    for &outer in &masks[a + 1..] {
                        assert!(!(inner & RIGHT != 0 && outer & (LEFT | STRAIGHT) != 0));
                        assert!(!(outer & LEFT != 0 && inner & (STRAIGHT | RIGHT) != 0));
                    }
                }
                if available & STRAIGHT != 0 {
                    assert!(masks.iter().all(|&mask| mask & STRAIGHT != 0));
                }
            }
        }
        assert_eq!([lane_mask(0, 2, 5), lane_mask(1, 2, 5)], [LEFT, RIGHT]);
        assert_eq!(
            [lane_mask(0, 2, 3), lane_mask(1, 2, 3)],
            [LEFT | STRAIGHT, STRAIGHT]
        );
        assert_eq!(
            [lane_mask(0, 2, 6), lane_mask(1, 2, 6)],
            [STRAIGHT, STRAIGHT | RIGHT]
        );
    }

    #[test]
    fn lane_use_angle_sectors_include_skewed_and_sloped_arms() {
        for (degrees, expected) in [
            (-160.0_f32, LEFT),
            (-90.0, LEFT),
            (-46.0, LEFT),
            (-44.0, STRAIGHT),
            (0.0, STRAIGHT),
            (44.0, STRAIGHT),
            (46.0, RIGHT),
            (90.0, RIGHT),
            (160.0, RIGHT),
        ] {
            let angle = degrees.to_radians();
            assert_eq!(
                movement(Vector2::RIGHT, Vector2::new(angle.cos(), angle.sin())),
                expected
            );
        }
    }

    #[test]
    fn lane_use_target_fans_never_cross_and_every_source_has_an_exit() {
        for count in 1..=4 {
            for targets in 1..=4 {
                for turn in [LEFT, STRAIGHT, RIGHT] {
                    let pairs: Vec<_> = (0..count)
                        .flat_map(|rank| {
                            (0..targets).filter_map(move |target| {
                                target_matches(rank, count, target, targets, turn)
                                    .then_some((rank, target))
                            })
                        })
                        .collect();
                    for rank in 0..count {
                        assert!(pairs.iter().any(|&(r, _)| r == rank));
                    }
                    // A single-lane group reaches every target lane (`TRAFFIC-23`).
                    if count == 1 {
                        assert_eq!(pairs.len(), targets);
                    }
                    for &(a, b) in &pairs {
                        for &(c, d) in &pairs {
                            assert!(!(a < c && b > d));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod junction_tests {
    use super::*;
    use crate::simulation::network::lanes::{LaneSystem, LaneType};
    use crate::simulation::network::roadbed::fixtures::{finalize, road_edge};
    use crate::simulation::network::types::{NodeType, TransitFlags};
    use std::collections::HashSet;

    #[test]
    fn lane_use_t_cross_and_skew_preserve_every_legal_road_turn() {
        for count in 1..=4 {
            for arms in [3, 4] {
                for skew in [false, true] {
                    let mut graph = RegionGraph::new();
                    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
                    let positions = if skew {
                        [
                            Vector3::new(-100.0, 0.0, 0.0),
                            Vector3::new(90.0, 4.0, 20.0),
                            Vector3::new(-25.0, 0.0, -100.0),
                            Vector3::new(35.0, 0.0, 100.0),
                        ]
                    } else {
                        [
                            Vector3::LEFT * 100.0,
                            Vector3::RIGHT * 100.0,
                            Vector3::FORWARD * 100.0,
                            Vector3::BACK * 100.0,
                        ]
                    };
                    // Every arm's forward lanes approach the centre.
                    for point in positions.into_iter().take(arms) {
                        let end = graph.add_node(point, NodeType::Junction);
                        let mut edge = road_edge(
                            end,
                            center,
                            vec![point, Vector3::ZERO],
                            f32::from(count) * config::LANE_WIDTH * 2.0,
                        );
                        edge.allowed_types = TransitFlags::CAR;
                        edge.fwd_lanes = count;
                        edge.bkw_lanes = count;
                        graph.add_edge(edge);
                    }
                    finalize(&mut graph);
                    let mut lanes = LaneSystem::new();
                    lanes.rebuild(&mut graph);
                    for from in 0..arms {
                        let policy = DefaultLaneUse::new(&graph, from, center);
                        // The paint's graph-only rule is the builder's flag.
                        assert_eq!(
                            approach_needs_lane_choice(&graph, from, center),
                            policy.needs_lane_choice()
                        );
                        let mut reachable = vec![false; arms];
                        for &id in &lanes.edge_lanes[&from] {
                            let source = &lanes.lanes[id];
                            if source.lane_type != LaneType::Vehicle {
                                continue;
                            }
                            assert_eq!(
                                lanes.needs_turn_preparation(id),
                                source.is_fwd && policy.needs_lane_choice(),
                            );
                            if !source.is_fwd {
                                continue;
                            }
                            assert!(!source.next_lanes.is_empty());
                            for &conn in &source.next_lanes {
                                let target = &lanes.lanes[lanes.lanes[conn].next_lanes[0]];
                                assert_ne!(from, target.edge_id, "NET-07");
                                assert!(policy.allows(
                                    &graph,
                                    source.lane_idx,
                                    target.edge_id,
                                    target.lane_idx
                                ));
                                reachable[target.edge_id] = true;
                            }
                        }
                        for (to, reached) in reachable.into_iter().enumerate() {
                            assert_eq!(
                                reached,
                                from != to,
                                "{count} lanes, {arms} arms, skew={skew}: {from}->{to}"
                            );
                        }
                    }
                    if arms == 4 && !skew {
                        // Arm 1 is straight ahead of arm 0, arm 2 its left and arm 3 its right.
                        assert_eq!(
                            approach_group(&graph, 0, center, 1),
                            Some((0, usize::from(count), true))
                        );
                        assert_eq!(approach_group(&graph, 0, center, 0), None);
                        if count > 1 {
                            let outer = usize::from(count) - 1;
                            assert_eq!(approach_group(&graph, 0, center, 2), Some((0, 1, false)));
                            assert_eq!(
                                approach_group(&graph, 0, center, 3),
                                Some((outer, 1, false))
                            );
                        }
                    }
                    // An explicit connection from the normally right-turning lane to the far
                    // left target still overrides defaults; all unspecified turns are blocked.
                    graph.add_lane_connection(center, 0, count as i8 - 1, 2, -1);
                    lanes.rebuild(&mut graph);
                    let connections: Vec<_> = lanes.node_lanes[&(center as usize)]
                        .iter()
                        .filter(|&&id| lanes.lanes[id].lane_type == LaneType::Vehicle)
                        .collect();
                    assert_eq!(connections.len(), 1);
                    assert_eq!(approach_group(&graph, 0, center, 1), None);
                    // The incremental rebuild re-flags every lane it rebuilds and unflags the
                    // lanes it retires.
                    let retired = lanes.edge_lanes[&0].clone();
                    lanes.rebuild_edges_incremental(&mut graph, &HashSet::from([0]));
                    assert!(retired.iter().all(|&id| !lanes.needs_turn_preparation(id)));
                    for from in 0..arms {
                        assert_eq!(approach_needs_lane_choice(&graph, from, center), count > 1);
                        for &id in &lanes.edge_lanes[&from] {
                            let lane = &lanes.lanes[id];
                            if lane.lane_type == LaneType::Vehicle {
                                assert_eq!(
                                    lanes.needs_turn_preparation(id),
                                    lane.is_fwd && count > 1
                                );
                            }
                        }
                    }
                    lanes.clear();
                    assert!(lanes.turn_preparation.is_empty());
                }
            }
        }
    }
}
