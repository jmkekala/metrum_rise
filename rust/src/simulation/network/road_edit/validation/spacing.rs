// SPDX-License-Identifier: GPL-2.0-only

//! Junction spacing (`ROAD-118` round JS3): a short stub to a road end goes, and a stroke's end
//! too close to an occupied node joins that node. Both answer a junction refusal of one build of
//! the excerpt; the plan is built again with them, as a dissolved joint's (round JS2). Pure
//! functions of the graph, the stroke and the land's frontages: preview and commit decide alike.

use super::{JointYield, ValidationGraph, horizontal_length_m};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::road_edit::RoadFrontages;
use crate::simulation::network::road_edit::input::{PreparedRoadInput, is_surface_edge};
use crate::simulation::network::road_edit::PlannedRoadSplit;
use crate::simulation::network::roadbed::{
    self, ArmKey, EdgeEnd, LayoutError, NodeError, arm_edge_needed_m, check_node,
};
use crate::simulation::network::topology::{self, StubRemoval};
use crate::simulation::network::types::EdgeClass;
use godot::prelude::Vector3;

/// The farthest a stroke's end moves to join an occupied node (`ROAD-118` round JS3, the user's
/// decision of 2026-11-15): a 2-lane junction arm's least mouth station, so any end too close to
/// a junction or bend on such a road joins it.
pub(super) const MAX_END_JOIN_M: f32 = 8.5;

// A stub that makes way (`ROAD-118` round JS3), in the excerpt's local ids: the road end, the
// stub from it to the junction that does not fit, and, where the stub is the head of a split
// road, the split's second half, on which the road keeps its id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct StubYield {
    end: u32,
    piece: usize,
    rest: Option<usize>,
}

// Every arm, in `(node, ArmKey)` order, of each affected canonical node whose layout fails on an
// arm (`CornerDoesNotFit`, `ArmTooShort`). The excerpt holds every affected node's full
// incidence. Refusals only: O(affected nodes x (layout + degree log degree)).
pub(super) fn failing_arms(excerpt: &ValidationGraph) -> Vec<(u32, ArmKey)> {
    let local = &excerpt.graph;
    let mut nodes: Vec<u32> = excerpt
        .finalized
        .affected_nodes
        .iter()
        .copied()
        .filter(|&node| local.get_valid_node(node) == node)
        .collect();
    nodes.sort_unstable();
    let mut failing = Vec::new();
    for node in nodes {
        if !matches!(
            check_node(local, node),
            Err(NodeError::Layout(
                LayoutError::CornerDoesNotFit { .. } | LayoutError::ArmTooShort { .. }
            ))
        ) {
            continue;
        }
        let mut arms: Vec<(u32, ArmKey)> = local.node_adjacency(node).iter().filter_map(|&edge| {
            let record = local.get_edge(edge).filter(|record| !record.deleted)?;
            let (start, end) = (
                local.get_valid_node(record.start_node),
                local.get_valid_node(record.end_node),
            );
            let end = match (start == node, end == node) {
                (true, false) => EdgeEnd::Start,
                (false, true) => EdgeEnd::End,
                _ => return None,
            };
            Some((node, ArmKey { edge, end }))
        }).collect();
        arms.sort_unstable();
        arms.dedup();
        failing.extend(arms);
    }
    failing
}

// The node across arm `arm`'s edge from its node, canonical.
fn far_node(local: &RegionGraph, arm: ArmKey) -> u32 {
    let edge = local.edge(arm.edge);
    local.get_valid_node(match arm.end {
        EdgeEnd::Start => edge.end_node,
        EdgeEnd::End => edge.start_node,
    })
}

// The live road local edge `edge` lies on: its own source, or the source its chain of splits
// started from; `None` for a piece of the stroke. O(source edges x split chain).
pub(super) fn source_root(excerpt: &ValidationGraph, mut edge: usize) -> Option<usize> {
    loop {
        if let Some((&source, _)) = excerpt
            .source_edge_ids
            .iter()
            .find(|&(_, &local)| local == edge)
        {
            return Some(source);
        }
        edge = excerpt
            .splits
            .iter()
            .find(|split| split.new_edge_id == edge)?
            .edge_id;
    }
}

// The live id of local node `node`, `None` for a node the edit makes.
fn source_node(excerpt: &ValidationGraph, node: u32) -> Option<u32> {
    excerpt
        .source_node_ids
        .iter()
        .find(|&(_, &local)| local == node)
        .map(|(&source, _)| source)
}

// The live arms of local node `node`.
fn degree(local: &RegionGraph, node: u32) -> usize {
    local
        .node_adjacency(node)
        .iter()
        .filter(|&&edge| local.get_edge(edge).is_some_and(|edge| !edge.deleted))
        .count()
}

// The stubs that make way for the junctions this excerpt refuses (`ROAD-118` round JS3): for
// each failing arm ([`failing_arms`]) of a node with three arms or more, too short for the node
// as its other arms stand (`arm_edge_needed_m` longer than the arm's road), whose far node is a
// road end of a plain standard road, when the stub is the edit's own: the stroke's end beyond
// its last node (never the stroke's only piece), the tail a split cut off a live road, or a live
// road's head cut by the edit's last split of it (never a whole live edge, whose id would
// retire). A stub of a live road goes only when no building fronts it (`frontages`) and its road
// end holds no turn rule; otherwise it is an occupied node and keeps its place. Not those already
// in `done` or among the joint yields. Refusals only: O(failing arms x the need's bisection +
// the stub's frontage query).
pub(super) fn stub_yields(
    excerpt: &ValidationGraph,
    existing_graph: &RegionGraph,
    (failing, stroke): (&[(u32, ArmKey)], &[usize]),
    frontages: &dyn RoadFrontages,
    done: (&[StubYield], &[JointYield]),
) -> Vec<StubYield> {
    let local = &excerpt.graph;
    let mut yields: Vec<StubYield> = Vec::new();
    for &(node, arm) in failing {
        let piece = local.edge(arm.edge);
        let end = far_node(local, arm);
        if end == node
            || degree(local, node) < 3
            || degree(local, end) != 1
            || piece.class != EdgeClass::Standard
            || piece.roundabout.is_some()
            || piece.pocket.is_some()
            || !is_surface_edge(piece)
        {
            continue;
        }
        let taken = done.0.iter().chain(&yields).any(|other| other.end == end)
            || done.1.iter().any(|other| other.joint == end || other.piece == arm.edge);
        if taken {
            continue;
        }
        let Some(found) = stub_of(excerpt, existing_graph, stroke, frontages, (end, arm.edge))
        else {
            continue;
        };
        let Some(needed_m) = arm_edge_needed_m(local, node, arm) else {
            continue;
        };
        if needed_m > horizontal_length_m(&piece.geometry) {
            yields.push(found);
        }
    }
    yields
}

// Road end `end`'s stub `piece` as a [`stub_yields`] yield when it is the edit's own and free,
// else `None`.
fn stub_of(
    excerpt: &ValidationGraph,
    existing_graph: &RegionGraph,
    stroke: &[usize],
    frontages: &dyn RoadFrontages,
    (end, piece): (u32, usize),
) -> Option<StubYield> {
    let local = &excerpt.graph;
    let record = local.edge(piece);
    let Some(root) = source_root(excerpt, piece) else {
        // The stroke's own: its end beyond its last node, while another piece of it stays.
        let at_stroke_end = excerpt
            .stroke_ends
            .iter()
            .any(|&stroke_end| local.get_valid_node(stroke_end) == end);
        let others = stroke.iter().filter(|&&id| id != piece).count();
        return (at_stroke_end && source_node(excerpt, end).is_none() && others > 0).then_some(
            StubYield {
                end,
                piece,
                rest: None,
            },
        );
    };
    // A live road end, the stub's road its only arm, with no turn rule there.
    let live_end = source_node(excerpt, end)?;
    let live_arms = existing_graph
        .node_adjacency(live_end)
        .iter()
        .filter(|&&id| !existing_graph.edge(id).deleted)
        .count();
    if live_arms != 1 || roadbed::has_vehicle_turn_rules(existing_graph, live_end) {
        return None;
    }
    let live = existing_graph.edge(root);
    let stub_m = record.physical_length;
    let copied = excerpt.source_edge_ids.get(&root) == Some(&piece);
    let (stations_m, rest) = if copied {
        // The road's head, cut off by the edit's last split of it.
        let split = excerpt
            .splits
            .iter()
            .rev()
            .find(|split: &&PlannedRoadSplit| split.edge_id == piece)?;
        if local.get_valid_node(record.start_node) != end {
            return None;
        }
        ([0.0, stub_m], Some(split.new_edge_id))
    } else {
        // The road's tail, a split's second half.
        if local.get_valid_node(record.end_node) != end {
            return None;
        }
        (
            [(live.physical_length - stub_m).max(0.0), live.physical_length],
            None,
        )
    };
    (!frontages.fronted(existing_graph, root, stations_m)).then_some(StubYield {
        end,
        piece,
        rest,
    })
}

// Removes the `stubs` that are still road ends with their stubs in place, in order (`ROAD-118`
// round JS3). A head cut off a copied road drops the split that cut it, as a dissolved joint's
// does. O(stubs x (points + degree + splits)).
pub(super) fn remove_stubs(
    graph: &mut RegionGraph,
    stubs: &[StubYield],
    splits: &mut Vec<PlannedRoadSplit>,
) -> Vec<StubRemoval> {
    let mut removals = Vec::with_capacity(stubs.len());
    for &StubYield { end, piece, rest } in stubs {
        if graph.get_valid_node(end) != end {
            continue;
        }
        let Some(removal) = topology::remove_stub_graph(graph, end, piece, rest) else {
            continue;
        };
        if let Some(rest) = rest {
            splits.retain(|split| !(split.edge_id == piece && split.new_edge_id == rest));
        }
        removals.push(removal);
    }
    removals
}

// The stroke end that joins an occupied node (`ROAD-118` round JS3): among the failing arms
// ([`failing_arms`]) on a live road, too short for their node, between a node the stroke's end
// makes on that road and a live node of the road (an occupied node: a joint or a free stub there
// already made way), the nearest one within [`MAX_END_JOIN_M`], ties by `(node, arm)` order.
// Each end moves once (`done`), never onto the stroke's other end. Returns the end (0 start, 1
// end) and the live node. Refusals only: O(failing arms x the need's bisection).
pub(super) fn end_join(
    excerpt: &ValidationGraph,
    failing: &[(u32, ArmKey)],
    done: [bool; 2],
) -> Option<(usize, u32)> {
    let local = &excerpt.graph;
    let ends = excerpt.stroke_ends.map(|end| local.get_valid_node(end));
    let mut best: Option<(f32, usize, u32)> = None;
    for &(node, arm) in failing {
        let far = far_node(local, arm);
        if source_root(excerpt, arm.edge).is_none() {
            continue;
        }
        let length_m = horizontal_length_m(&local.edge(arm.edge).geometry);
        if arm_edge_needed_m(local, node, arm).is_none_or(|needed_m| needed_m <= length_m) {
            continue;
        }
        for (near, target) in [(node, far), (far, node)] {
            let Some(end) = ends.iter().position(|&stroke_end| stroke_end == near) else {
                continue;
            };
            if done[end] || ends[1 - end] == target || source_node(excerpt, near).is_some() {
                continue;
            }
            let Some(live) = source_node(excerpt, target) else {
                continue;
            };
            let (a, b) = (local.node(near).pos, local.node(target).pos);
            let distance_m = (b.x - a.x).hypot(b.z - a.z);
            if distance_m <= MAX_END_JOIN_M && best.is_none_or(|(best_m, ..)| distance_m < best_m)
            {
                best = Some((distance_m, end, live));
            }
        }
    }
    best.map(|(_, end, live)| (end, live))
}

/// Carries end `end` (0 the start, 1 the end) of stroke `points` onto `target` (`ROAD-118` round
/// JS3): every point moves by the end's displacement times its share of the stroke's horizontal
/// length counted from the other end, so a straight stroke stays straight, a curve stays smooth,
/// the other end stays put and the end lands on `target` bit for bit. `O(points)`, no
/// allocation.
pub(super) fn join_stroke_end(points: &mut [Vector3], end: usize, target: Vector3) {
    let Some(last) = points.len().checked_sub(1).filter(|&last| last > 0) else {
        return;
    };
    let total_m: f32 = points
        .windows(2)
        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z))
        .sum();
    let moved = if end == 0 { points[0] } else { points[last] };
    let shift = target - moved;
    let mut station_m = 0.0;
    let mut previous = points[0];
    for point in points.iter_mut() {
        station_m += (point.x - previous.x).hypot(point.z - previous.z);
        previous = *point;
        let share = if total_m > 0.0 {
            station_m / total_m
        } else {
            1.0
        };
        let weight = if end == 0 { 1.0 - share } else { share };
        *point += shift * weight;
    }
    if end == 0 {
        points[0] = target;
    } else {
        points[last] = target;
    }
}

/// `prepared` with its ends carried onto the live nodes `nodes` of `graph` (`ROAD-118` round
/// JS3), the start before the end ([`join_stroke_end`]), and those nodes as its joined ends: the
/// stroke a plan with joined ends builds. `O(points)`.
pub(crate) fn joined_stroke(
    prepared: &PreparedRoadInput,
    nodes: [Option<u32>; 2],
    graph: &RegionGraph,
) -> PreparedRoadInput {
    let mut joined = prepared.clone();
    for (end, node) in nodes.into_iter().enumerate() {
        if let Some(node) = node {
            join_stroke_end(&mut joined.points, end, graph.node(node).pos);
            joined.joined_nodes[end] = Some(node);
        }
    }
    joined
}
