// SPDX-License-Identifier: GPL-2.0-only

//! Vehicle junction connectors (`TRAFFIC-12`): one connector lane per allowed turn (the node's
//! whitelist, or else the default lane use of `ROAD-94`), from the incoming lane's end to the
//! outgoing lane's start. Both ends are the lanes' points on their arms' mouth frames
//! ([`MouthFrame`]), and the connector leaves and joins along the arms' directions there, never
//! along a lane polyline's end piece, a ring lane's excepted (`ring_lane_outward`). Its curve is
//! [`VehicleTurn`].
//! Each connector is checked against the node's kerb corners once per lane rebuild; a
//! violation is logged on the `traffic` debug channel, and nothing repairs the curve.

use super::super::graph::{Edge, RegionGraph};
use super::super::roadbed::{ArmKey, Corner, EdgeEnd, MouthFrame, RoadVec2};
use super::geometry::build_cum_dist;
use super::lane_use::DefaultLaneUse;
use super::rebuild::vehicle_lane_offset;
use super::{APPROACH_SOLID_LENGTH_M, Lane, LaneSuccessors, LaneType};
use godot::prelude::*;
use std::collections::HashMap;

mod turn;

#[cfg(test)]
pub(crate) use turn::TurnKind;
pub(crate) use turn::VehicleTurn;

const VEHICLE_DIRECT_PASS_THROUGH_EPS_M: f32 = 0.05;
const VEHICLE_DIRECT_PASS_THROUGH_DOT: f32 = 0.98;
// A ring lane's direction at its end is taken over at least this distance.
const RING_LANE_DIRECTION_PIECE_M: f64 = 0.01;

/// Where a vehicle lane meets the node: its point on its arm's mouth frame and the arm's
/// direction away from the node there. The lane rebuild ends the lane exactly at `point`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VehicleLaneEnd {
    /// The lane's point on the mouth line.
    pub(crate) point: Vector3,
    /// Unit XZ direction of the arm away from the node at the mouth.
    pub(crate) outward: RoadVec2,
}

/// The mouth frame of the end `end` of `edge_idx`: its arm's section at the edge's clip there,
/// the station every system that maps edge stations to lanes reads (`graph/rebuild/clips.rs` sets
/// it to the layout's mouth station, zero at a terminal or a node without a layout). `None` for a
/// degenerate edge. `O(edge points)`.
pub(crate) fn vehicle_mouth_frame(
    graph: &RegionGraph,
    edge_idx: usize,
    end: EdgeEnd,
) -> Option<MouthFrame> {
    let edge = graph.edge(edge_idx);
    let (node, clip) = match end {
        EdgeEnd::Start => (edge.start_node, edge.start_clip),
        EdgeEnd::End => (edge.end_node, edge.end_clip),
    };
    let key = ArmKey {
        edge: edge_idx,
        end,
    };
    MouthFrame::at(graph, node, key, f64::from(clip))
}

/// Where vehicle lane `lane_idx` of `edge` meets `frame`, the frame of the edge's `end`: its
/// lateral offset (in the edge's forward frame, mirrored at the edge's end) on the frame's line.
pub(crate) fn vehicle_lane_end(
    frame: &MouthFrame,
    end: EdgeEnd,
    edge: &Edge,
    lane_idx: i8,
) -> VehicleLaneEnd {
    let forward_offset = f64::from(vehicle_lane_offset(edge, lane_idx));
    let lateral_m = match end {
        EdgeEnd::Start => forward_offset,
        EdgeEnd::End => -forward_offset,
    };
    let point = frame.point(lateral_m);
    VehicleLaneEnd {
        point: Vector3::new(point.x as f32, point.y as f32, point.z as f32),
        outward: frame.direction(),
    }
}

// The direction of `lane` away from the node at its end there: from the tip to the first lane
// point at least `RING_LANE_DIRECTION_PIECE_M` off. A ring lane (`ROAD-62`) is a polygon of 1 m
// sides turning about 4 degrees at each, which a frame's direction, interpolated between two
// knots, leaves by up to half of that: a connector on the frame's direction would steer against
// the ring on leaving or joining the lane. `None` for a degenerate lane. `O(lane points)` at worst.
fn ring_lane_outward(lane: &Lane, leaves: bool) -> Option<RoadVec2> {
    let flat = |p: &Vector3| RoadVec2::new(f64::from(p.x), f64::from(p.z));
    let from_tip = |points: &mut dyn Iterator<Item = &Vector3>| {
        let tip = flat(points.next()?);
        points
            .map(|point| flat(point) - tip)
            .find(|piece| piece.length() >= RING_LANE_DIRECTION_PIECE_M)
            .map(RoadVec2::normalize)
    };
    if leaves {
        from_tip(&mut lane.geometry.iter())
    } else {
        from_tip(&mut lane.geometry.iter().rev())
    }
}

/// Builds vehicle intersection connection lanes at a single node: one connector per allowed
/// pair of an inbound and an outbound lane, under the node's whitelist or else the default lane
/// use (`ROAD-94`). Writes every inbound lane's solid approach zone (`Lane::solid_zone_m`). `kerb_corners` are its layout's fitted kerb corners
/// (`NodeWalkways::collect_with_kerbs`), empty without a layout.
pub(crate) fn build_vehicle_connections_at_node(
    lanes: &mut Vec<Lane>,
    lane_map: &HashMap<(usize, bool, i8), usize>,
    graph: &RegionGraph,
    node_id: usize,
    node_lanes: &mut std::collections::HashMap<usize, Vec<usize>>,
    kerb_corners: &[Corner],
) {
    // (edge, lane index, lane id, its end at the node).
    let mut inbound: Vec<(usize, i8, usize, VehicleLaneEnd)> = Vec::new();
    let mut outbound: Vec<(usize, i8, usize, VehicleLaneEnd)> = Vec::new();
    // Arms that carry vehicle lanes. Footpaths and other lane-less arms join the node for
    // pedestrians only: a road continuing past a footpath is a pass-through for cars.
    let mut vehicle_deg = 0;

    for &e_idx in graph.node_adjacency(node_id as u32) {
        let edge = graph.edge(e_idx);
        if edge.deleted {
            continue;
        }
        if edge.fwd_lanes + edge.bkw_lanes > 0 {
            vehicle_deg += 1;
        }
        for (at_node, end) in [
            (edge.start_node as usize == node_id, EdgeEnd::Start),
            (edge.end_node as usize == node_id, EdgeEnd::End),
        ] {
            if !at_node {
                continue;
            }
            let Some(frame) = vehicle_mouth_frame(graph, e_idx, end) else {
                continue;
            };
            // Forward lanes leave an edge's start and reach its end; backward lanes the reverse.
            let lane_sets = [(true, edge.fwd_lanes), (false, edge.bkw_lanes)];
            for (is_fwd, count) in lane_sets {
                for l in 0..count {
                    let lane_idx = if is_fwd { l as i8 } else { -(l as i8) - 1 };
                    let Some(&lid) = lane_map.get(&(e_idx, is_fwd, lane_idx)) else {
                        continue;
                    };
                    let mut lane_end = vehicle_lane_end(&frame, end, edge, lane_idx);
                    let leaves = is_fwd == (end == EdgeEnd::Start);
                    if edge.roundabout.is_some()
                        && let Some(outward) = ring_lane_outward(&lanes[lid], leaves)
                    {
                        lane_end.outward = outward;
                    }
                    let list = if leaves { &mut outbound } else { &mut inbound };
                    list.push((e_idx, lane_idx, lid, lane_end));
                }
            }
        }
    }

    let lane_conns = &graph.node(node_id as u32).lane_connections;
    let node_has_any_vehicle_conn = node_whitelists_vehicle_turns(graph, node_id);
    let node_allows_direct_pass_through = vehicle_deg == 2 && !node_has_any_vehicle_conn;
    // Dead ends allow the U-turn at an open node; the router reads the same edge-level rule.
    let allows_u_turn = graph.open_node_allows_u_turn(node_id as u32);

    for &(in_edge_id, in_lane_idx, in_lane_id, in_end) in &inbound {
        // Global whitelist mode: if the node has ANY user vehicle connection (lane_idx ≠ ±100),
        // ALL unspecified turns are blocked. Only explicitly listed turns are permitted; an arm
        // without an explicit connection gets no connector. Nodes with no user connections are
        // open: every turn into another arm that the default lane use (`ROAD-94`) gives this
        // lane.
        let explicit = lane_conns.get(&(in_edge_id, in_lane_idx));
        let defaults = (explicit.is_none() && !node_has_any_vehicle_conn)
            .then(|| DefaultLaneUse::new(graph, in_edge_id, node_id as u32));
        // A multi-lane approach to a whitelisted node needs its lane chosen too.
        let needs_lane_choice = defaults.as_ref().map_or_else(
            || {
                let edge = graph.edge(in_edge_id);
                let count = if lanes[in_lane_id].is_fwd {
                    edge.fwd_lanes
                } else {
                    edge.bkw_lanes
                };
                count > 1
            },
            DefaultLaneUse::needs_lane_choice,
        ) || (defaults.is_some()
            && super::lane_use::transition_needs_lane_choice(graph, in_edge_id, node_id as u32));
        lanes[in_lane_id].solid_zone_m = if needs_lane_choice {
            APPROACH_SOLID_LENGTH_M
        } else {
            0.0
        };

        for &(out_edge_id, out_lane_idx, out_lid, out_end) in &outbound {
            let allowed = match &defaults {
                Some(_) if out_edge_id == in_edge_id => allows_u_turn,
                Some(defaults) => defaults.allows(graph, in_lane_idx, out_edge_id, out_lane_idx),
                None => explicit.is_some_and(|rules| rules.contains(&(out_edge_id, out_lane_idx))),
            };
            if !allowed {
                continue;
            }
            let (p0, p3) = (in_end.point, out_end.point);
            let incoming = -in_end.outward;
            let outgoing = out_end.outward;
            // On the inside of a slight bend the offset lanes overlap: the outgoing lane starts
            // behind the incoming lane's end, and any connector between them would run backwards.
            let overlaps =
                f64::from(p3.x - p0.x) * incoming.x + f64::from(p3.z - p0.z) * incoming.y <= 0.0;
            if node_allows_direct_pass_through
                && (p0.distance_to(p3) <= VEHICLE_DIRECT_PASS_THROUGH_EPS_M || overlaps)
                && incoming.dot(outgoing) >= f64::from(VEHICLE_DIRECT_PASS_THROUGH_DOT)
            {
                lanes[in_lane_id].next_lanes.push(out_lid);
                continue;
            }

            let flat = |p: Vector3| RoadVec2::new(f64::from(p.x), f64::from(p.z));
            let Some(turn) = VehicleTurn::new(flat(p0), incoming, flat(p3), outgoing, kerb_corners)
            else {
                // Parallel ends with the exit behind the entry: no forward path exists.
                crate::traffic_log!(
                    "[TRAFFIC-12] node {node_id}: no connector from lane {in_lane_id} to lane {out_lid}"
                );
                continue;
            };
            let margin_m = turn.kerb_margin_m(kerb_corners);
            if margin_m < 0.0 {
                crate::traffic_log!(
                    "[TRAFFIC-12] node {node_id}: {:?} connector from lane {in_lane_id} to lane {out_lid} leaves the carriageway by {:.3} m at a kerb corner",
                    turn.kind,
                    -margin_m
                );
            }
            let conn_geom = turn.polyline(p0, p3);
            let conn_cum = build_cum_dist(&conn_geom);
            let conn_len = *conn_cum.last().unwrap_or(&0.0);
            let conn_id = lanes.len();
            lanes.push(Lane {
                edge_id: usize::MAX,
                is_fwd: true,
                lane_idx: 0,
                geometry: conn_geom,
                length: conn_len,
                frontage_delay_penalty_s: 0.0,
                live_frontage_delay_s: 0.0,
                cum_dist: conn_cum,
                lane_type: LaneType::Vehicle,
                next_lanes: LaneSuccessors::one(out_lid),
                node_id,
                solid_zone_m: 0.0,
            });
            node_lanes.entry(node_id).or_default().push(conn_id);
            lanes[in_lane_id].next_lanes.push(conn_id);
        }
    }
}

/// Returns `true` when the node has a user vehicle lane connection, which puts every turn at
/// the node in whitelist mode. Pedestrian rules (`RegionGraph::is_vehicle_turn_rule`: sidewalk
/// lane `±100` or a footpath edge) never do.
pub(crate) fn node_whitelists_vehicle_turns(graph: &RegionGraph, node_id: usize) -> bool {
    graph
        .node(node_id as u32)
        .lane_connections
        .keys()
        .any(|&(edge_idx, lane_idx)| graph.is_vehicle_turn_rule(edge_idx, lane_idx))
}
