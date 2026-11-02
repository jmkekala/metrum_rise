// SPDX-License-Identifier: GPL-2.0-only

//! The vehicle clearance under a bridge (`ROAD-99`, grade separation by structure).
//!
//! A bridge crosses the roads it passes over, and every road crosses a bridge, without a node
//! (`topology::StrokeGrade`); the crossing holds while the higher deck stands at least
//! [`ROAD_UNDER_BRIDGE_M`] above the lower wherever their footprints overlap: a road drawn on
//! the ground under a bridge, a bridge drawn over a road, a bridge over or under another bridge.
//! Quick validation and the exact plan read the same prepared points and the same excerpt of the
//! committed graph, so they refuse the same strokes.
//!
//! A stroke whose end is snapped onto a bridge's span joins it there (`ROAD-99` follow-up B1,
//! [`snapped_structure_span`]): the span is split and a junction stands on its deck, as where the
//! end meets one of the bridge's nodes. The end meets that deck and does not pass under it.

use super::PreparedRoadInput;
use super::input::{is_surface_edge, xz_distance};
use super::tunnel::{RoadDeck, TUNNEL_LATTICE_M, edge_profile, edges_near};
use crate::config;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{self, BRIDGE_DECK_DEPTH_M, visual_roadbed_half_width_m};
use crate::simulation::network::types::EdgeClass;
use godot::prelude::Vector3;

/// How far a bridge's deck stands at least above the deck of a road it passes over (`ROAD-99`):
/// the vehicle envelope ([`roadbed::VEHICLE_CLEARANCE_M`]) clear under the bridge's slab, and the
/// slab's depth ([`BRIDGE_DECK_DEPTH_M`]), deck to deck (4.85 m). The mirror of
/// `ROAD_OVER_TUNNEL_M`: under a tunnel's ceiling the lining keeps the same envelope clear.
pub(super) const ROAD_UNDER_BRIDGE_M: f32 =
    (roadbed::VEHICLE_CLEARANCE_M + BRIDGE_DECK_DEPTH_M) as f32;
/// A road and a bridge, or two bridges, whose decks stand less than [`ROAD_UNDER_BRIDGE_M`] apart
/// where one passes over the other ([`deck_to_deck_m`]).
pub(super) const BRIDGE_TOO_LOW_REASON: &str = "bridge_too_low";

/// A stroke end snapped onto a tunnel inside its bore (B1): roads meet a tunnel at its portals.
pub(super) const TUNNEL_JOIN_REASON: &str = "tunnel_join";

// How far in plan an end may lie from a span's centre line and still be the point the road
// tool's cursor snapped onto it: the snap projects the cursor onto the centre line, so only the
// rounding of that projection separates them. A free cursor almost never lands on the line.
const ON_CENTRE_LINE_M: f32 = 0.01;

/// The structure span a stroke's end is snapped onto, with its graph height there (`ROAD-99`
/// follow-up B1). The road tool's cursor snaps to a structure only where its ray hit that
/// structure's deck (`interaction::StructureHit`), and then reports the point on the span's
/// centre line at the span's graph height. So an end joins the bridge or tunnel span whose centre
/// line it lies on in plan (within rounding, [`ON_CENTRE_LINE_M`]) and whose graph profile there
/// stands nearer the end's height than `ground_m` does, the ground level under the end (a
/// grounded road's carriageway or the terrain): the cursor's own choice between the levels. An
/// end on the ground beside or under a bridge, or on a hill over a bore, stands nearer the
/// ground and joins nothing. Ties go to the ground, then to the nearer centre line, then to the
/// lower edge id. `None` for an end within the node snap of a node, which joins the node.
/// `O(log E + K x profile points)` for the `K` edges near the end.
pub(super) fn snapped_structure_span(
    graph: &RegionGraph,
    end: Vector3,
    ground_m: f32,
) -> Option<(usize, f32)> {
    if graph
        .find_node_within(end, config::SNAP_TOLERANCE)
        .is_some()
    {
        return None;
    }
    let reach = ON_CENTRE_LINE_M;
    let mut best: Option<(f32, f32, usize, f32)> = None;
    let mut edges = edges_near(graph, &[end], reach);
    edges.sort_unstable();
    edges.dedup();
    for id in edges {
        let edge = graph.edge(id);
        if !is_surface_edge(edge) || edge.class == EdgeClass::Standard {
            continue;
        }
        let (distance_sq, _) = RegionGraph::profile_nearest_at(&edge.geometry, end.x, end.z);
        if distance_sq >= reach * reach {
            continue;
        }
        let deck_m = RegionGraph::profile_height_at(edge_profile(edge), end.x, end.z);
        let off_m = (end.y - deck_m).abs();
        if off_m >= (end.y - ground_m).abs() {
            continue;
        }
        let key = (off_m, distance_sq, id, deck_m);
        if best.is_none_or(|best| (key.0, key.1, key.2) < (best.0, best.1, best.2)) {
            best = Some(key);
        }
    }
    best.map(|(_, _, id, deck_m)| (id, deck_m))
}

/// The least height between the deck of a stroke and a deck it passes under or over
/// (`ROAD-99`): over every point of the stroke's footprint lattice (stations along each segment
/// and across its roadbed, `half_width_m` to either side, at most [`TUNNEL_LATTICE_M`] apart)
/// that the other deck's footprint covers. The other decks are the bridges of `graph`, and for
/// a bridge stroke the grounded roads too, within reach of the stroke, less the roads the stroke
/// joins at its ends ([`joined_edges`]): it meets those at a node. `None` when it passes over or
/// under none. Reads the local excerpt only: `O(log E + K + P x D)` for the `K` edges met, the
/// `P` lattice points and the `D` profile points of the decks kept. Allocates the excerpt and its
/// decks once per stroke.
pub(super) fn deck_to_deck_m(
    prepared_input: &PreparedRoadInput,
    half_width_m: f32,
    graph: &RegionGraph,
) -> Option<f32> {
    let points = &prepared_input.points[..];
    let bridge_stroke = prepared_input.class == EdgeClass::Bridge;
    let joined = joined_edges(prepared_input, graph);
    let mut edges = edges_near(graph, points, half_width_m);
    edges.sort_unstable();
    edges.dedup();
    let decks: Vec<RoadDeck> = edges
        .into_iter()
        .filter(|id| joined.binary_search(id).is_err())
        .map(|id| graph.edge(id))
        .filter(|edge| {
            is_surface_edge(edge)
                && (edge.class == EdgeClass::Bridge
                    || (bridge_stroke && edge.class == EdgeClass::Standard))
        })
        .map(|edge| RoadDeck::new(edge_profile(edge), visual_roadbed_half_width_m(edge)))
        .collect();
    if decks.is_empty() {
        return None;
    }
    let mut least: Option<f32> = None;
    for_each_footprint_point(points, half_width_m, |x, z, height_m| {
        for deck in &decks {
            if let Some(deck_m) = deck.height_at(x, z) {
                let apart = (deck_m - height_m).abs();
                least = Some(least.map_or(apart, |least| least.min(apart)));
            }
        }
    });
    least
}

// The roads a stroke meets at its ends, sorted: every road at the existing node an end snaps to
// (the endpoint snap's sphere, as the validation graph joins it), the bridge span an end is
// snapped onto (`PreparedRoadInput::joined_spans`, B1) with the bridges at its nodes, and the
// standard road whose centre line passes within the join tolerance of a ground end
// (`topology::StrokeGrade`'s end join; a bridge's end at its support, as the class inference
// found it). A stroke from a bridge's
// terminal or abutment, or a bridge from a road mid-span, starts on the deck it joins.
// `O(log E + K x profile points + degree)`.
fn joined_edges(prepared_input: &PreparedRoadInput, graph: &RegionGraph) -> Vec<usize> {
    let points = &prepared_input.points;
    let mut joined = Vec::new();
    let ends = [points[0], points[points.len() - 1]];
    for (end, point) in ends.into_iter().enumerate() {
        let node = prepared_input
            .endpoint_snap_enabled
            .then(|| graph.find_node_within(point, config::SNAP_TOLERANCE))
            .flatten();
        if let Some(node) = node {
            joined.extend_from_slice(graph.node_adjacency(graph.get_valid_node(node)));
            continue;
        }
        if let Some(span) = prepared_input.joined_spans[end] {
            // The span and the bridge decks it continues at its nodes: the junction on the deck
            // meets them at its level, and the junction rules decide how close it stands to them.
            joined.push(span);
            let span = graph.edge(span);
            for node in [span.start_node, span.end_node] {
                joined.extend(
                    (graph.node_adjacency(graph.get_valid_node(node)).iter())
                        .filter(|&&arm| graph.edge(arm).class == EdgeClass::Bridge),
                );
            }
            continue;
        }
        if prepared_input.elevated_ends[end] {
            continue;
        }
        let reach = config::INTERSECTION_TOLERANCE;
        joined.extend(
            edges_near(graph, &[point], reach)
                .into_iter()
                .filter(|&id| {
                    let edge = graph.edge(id);
                    let (distance_sq, _) =
                        RegionGraph::profile_nearest_at(&edge.geometry, point.x, point.z);
                    is_surface_edge(edge)
                        && edge.class == EdgeClass::Standard
                        && distance_sq < reach * reach
                }),
        );
    }
    joined.sort_unstable();
    joined.dedup();
    joined
}

// Visits the stroke's footprint lattice: along each segment of `points`, stations at most
// `TUNNEL_LATTICE_M` apart (both ends included), and across each station, perpendicular in plan
// to its segment, points from `-half_width_m` to `half_width_m` at most `TUNNEL_LATTICE_M` apart,
// the centre line and both edges among them, each with the station's height (the section is
// level). `O(length / lattice x half width / lattice)`, allocation-free.
fn for_each_footprint_point(
    points: &[Vector3],
    half_width_m: f32,
    mut visit: impl FnMut(f32, f32, f32),
) {
    let across_steps = (half_width_m / TUNNEL_LATTICE_M).ceil().max(1.0) as i32;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let length = xz_distance(a, b);
        if length == 0.0 {
            continue;
        }
        let (across_x, across_z) = (-(b.z - a.z) / length, (b.x - a.x) / length);
        let steps = (length / TUNNEL_LATTICE_M).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let station = a.lerp(b, step as f32 / steps as f32);
            for across in -across_steps..=across_steps {
                let offset = half_width_m * across as f32 / across_steps as f32;
                visit(
                    station.x + offset * across_x,
                    station.z + offset * across_z,
                    station.y,
                );
            }
        }
    }
}
