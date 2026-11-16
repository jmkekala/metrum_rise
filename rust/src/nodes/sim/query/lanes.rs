// SPDX-License-Identifier: GPL-2.0-only

//! Lane-specific spatial queries (lane positions, junction connectivity, crosswalks).

use crate::nodes::sim::core::SimCore;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::lanes::approach_lane_lacks_movement;
use crate::simulation::network::lanes::rebuild::vehicle_lane_offset;
use crate::simulation::network::lanes::vehicle_junctions::node_whitelists_vehicle_turns;
use crate::simulation::network::roadbed::crossing_arm_eligible;
use godot::prelude::*;

impl SimCore {
    /// Whether road arm `edge_id` at `node_id` can cross (`ROAD-74`,
    /// [`crossing_arm_eligible`]). `O(degree)`.
    pub fn can_toggle_crosswalk_internal(&self, node_id: u32, edge_id: i32) -> bool {
        (node_id as usize) < self.region_graph.node_count()
            && usize::try_from(edge_id)
                .is_ok_and(|edge| crossing_arm_eligible(&self.region_graph, node_id, edge))
    }

    /// Reports whether the lane rebuild emitted a visible crossing for this road arm.
    pub fn has_crosswalk_internal(&self, node_id: u32, edge_id: i32) -> bool {
        let valid_id = self.region_graph.get_valid_node(node_id);
        if valid_id as usize >= self.region_graph.node_count() || edge_id < 0 {
            return false;
        }
        self.transit_network
            .lane_system
            .node_lanes
            .get(&(valid_id as usize))
            .is_some_and(|lane_ids| {
                lane_ids.iter().any(|&lane_id| {
                    self.transit_network
                        .lane_system
                        .crosswalk_marking(lane_id)
                        .is_some_and(|crosswalk| crosswalk.edge_id == edge_id as usize)
                })
            })
    }

    /// Returns an array of current lane turn restrictions at a node.
    pub fn get_lane_connections_array_internal(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();
        if node_id as usize >= self.region_graph.node_count() {
            return arr;
        }
        let node = self.region_graph.node(node_id);

        for (src, targets) in &node.lane_connections {
            for tgt in targets {
                let mut dict = VarDictionary::new();
                dict.set("from_edge", src.0 as i32);
                dict.set("from_lane", src.1 as i32);
                dict.set("to_edge", tgt.0 as i32);
                dict.set("to_lane", tgt.1 as i32);
                arr.push(&dict.to_variant());
            }
        }
        arr
    }

    /// The junction lane editor's handles at `node_id` (`TRAFFIC-11` lane rules): one dictionary
    /// per vehicle lane of each arm end at the node, with `edge_id`, `lane_id` (the lane index
    /// rules are stored by), `is_incoming`, `pos` and `no_movement` (an approach lane the node's
    /// rules leave without a movement, `ROAD-119`). See [`lane_handles`]. `O(arms + lanes + the
    /// node's rules)`.
    pub fn get_node_lanes_internal(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();
        for handle in lane_handles(&self.region_graph, node_id) {
            let mut dict = VarDictionary::new();
            dict.set("edge_id", handle.edge as i32);
            dict.set("lane_id", i32::from(handle.lane_idx));
            dict.set("is_incoming", handle.incoming);
            dict.set("pos", handle.pos);
            dict.set("no_movement", handle.no_movement);
            arr.push(&dict.to_variant());
        }
        arr
    }
}

// One handle of the junction lane editor: a vehicle lane of an arm end at the node, where the
// select tool draws its sphere.
struct LaneHandle {
    edge: usize,
    lane_idx: i8,
    incoming: bool,
    pos: Vector3,
    // An approach lane the node's vehicle rules leave without a movement (`ROAD-119`).
    no_movement: bool,
}

// The lane handles at `node_id`'s canonical node, arm by arm in ascending edge order, each arm
// end's forward lanes then its backward lanes. A schematic row 5 m out along the arm across it,
// 1 m apart, in the order the lanes run across the road: each handle's slot is its lane's rank by
// [`vehicle_lane_offset`] in the edge's forward frame, the frame the lane geometry is offset in,
// so a handle sits on its lane's side and rank (`ROAD-119`: the row used to put forward lane 0
// outermost, mirroring every forward lane's handle). `O(arms + lanes + the node's rules)`.
fn lane_handles(graph: &RegionGraph, node_id: u32) -> Vec<LaneHandle> {
    let mut handles = Vec::new();
    let node = graph.get_valid_node(node_id);
    if node as usize >= graph.node_count().min(graph.node_adjacency_count()) {
        return handles;
    }
    let junction_pos = graph.node(node).pos;
    let whitelisted = node_whitelists_vehicle_turns(graph, node as usize);
    // This arm end's lanes by lateral offset: (offset, lane index).
    let mut across: Vec<(f32, i8)> = Vec::new();

    for &e_id in graph.node_adjacency(node) {
        let edge = graph.edge(e_id);
        if edge.deleted {
            continue;
        }
        // PREFER LOGICAL GEOMETRY for robust visuals
        let geo = if edge.geometry.len() >= 2 {
            &edge.geometry
        } else {
            &edge.physical_geometry
        };
        if geo.len() < 2 {
            continue;
        }

        // Each end at this junction; a self-loop has both.
        for is_start_side in [true, false] {
            let at_node = if is_start_side {
                edge.start_node
            } else {
                edge.end_node
            };
            if at_node != node {
                continue;
            }
            // 1. Establish robust "Into-the-Leg" direction
            // ANCHOR: We must skip the "stub" (points near the center from merged nodes)
            // Search for the first point at least 3.1m away (HUB_RADIUS + margin)
            const SEARCH_RADIUS: f32 = 3.1;
            let mut diff = Vector3::ZERO;
            let mut best_stub = Vector3::ZERO;
            let points: &mut dyn Iterator<Item = &Vector3> = if is_start_side {
                &mut geo.iter()
            } else {
                &mut geo.iter().rev()
            };
            for &point in points {
                let d = point - junction_pos;
                if d.length() > SEARCH_RADIUS {
                    diff = d;
                    break;
                }
                if d.length() > 0.1 {
                    best_stub = d;
                }
            }

            // Fallback: If the road is very short, use the best stub or just the other end.
            if diff.length_squared() < 0.01 {
                if best_stub.length_squared() > 0.01 {
                    diff = best_stub;
                } else {
                    // Absolute fallback: other node's pos
                    let other_node = if is_start_side {
                        edge.end_node
                    } else {
                        edge.start_node
                    };
                    diff = graph.node(other_node).pos - junction_pos;
                }
            }

            if diff.length_squared() < 1e-6 {
                continue;
            }
            let dir_to_leg = diff.normalized();

            // The edge's forward frame at both ends, the lane geometry's own: its normal is the
            // side positive lane offsets lie on.
            let forward_tangent = if is_start_side {
                dir_to_leg
            } else {
                -dir_to_leg
            };
            let road_normal = Vector3::new(-forward_tangent.z, 0.0, forward_tangent.x);

            // 2. Base position offset (5.0m ensures it's clearly past the 3.0m hub)
            let mut current_pos = junction_pos + dir_to_leg * 5.0;
            current_pos.y += 0.4;

            across.clear();
            across.extend(
                lane_indices_in_order(edge).map(|lane| (vehicle_lane_offset(edge, lane), lane)),
            );
            across.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let total_lanes = across.len() as f32;
            const LANE_PITCH_M: f32 = 1.0;

            for lane_idx in lane_indices_in_order(edge) {
                let is_fwd = lane_idx >= 0;
                let slot = across
                    .iter()
                    .position(|&(_, lane)| lane == lane_idx)
                    .unwrap_or_default() as f32;
                let lateral_offset = (slot + 0.5 - total_lanes * 0.5) * LANE_PITCH_M;
                let mut pos = current_pos + road_normal * lateral_offset;
                pos.y += 0.2; // Slightly lower spheres for schematic view
                // Forward lanes reach an edge's end, backward lanes its start.
                let incoming = is_fwd != is_start_side;
                handles.push(LaneHandle {
                    edge: e_id,
                    lane_idx,
                    incoming,
                    pos,
                    no_movement: incoming
                        && approach_lane_lacks_movement(graph, node, whitelisted, e_id, lane_idx),
                });
            }
        }
    }
    handles
}

// An edge's vehicle lane indices, forward `0, 1, ..` then backward `-1, -2, ..`.
fn lane_indices_in_order(edge: &Edge) -> impl Iterator<Item = i8> {
    (0..edge.fwd_lanes)
        .map(|l| l as i8)
        .chain((0..edge.bkw_lanes).map(|l| -(l as i8) - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::simulation::network::lanes::{LaneSystem, LaneType};
    use crate::simulation::network::roadbed::fixtures::t_junction;

    const CENTRE: u32 = 0;

    // The fixture T (edge 0 west to centre, 1 centre to east, 2 south to centre) with lanes
    // `[fwd, bkw]` per edge, as wide as its lanes, its clips and lanes built.
    fn junction(lanes: [[u8; 2]; 3]) -> (RegionGraph, LaneSystem) {
        let mut graph = t_junction(false);
        for (edge, [fwd, bkw]) in lanes.into_iter().enumerate() {
            let edge = graph.edge_mut(edge);
            edge.fwd_lanes = fwd;
            edge.bkw_lanes = bkw;
            edge.width = f32::from(fwd + bkw) * config::LANE_WIDTH;
        }
        graph.rebuild_intersection_clips();
        let mut system = LaneSystem::new();
        system.rebuild(&mut graph);
        (graph, system)
    }

    // The unit normal of `edge`'s forward frame at its end at the centre, the side positive lane
    // offsets lie on.
    fn forward_normal(graph: &RegionGraph, edge: usize) -> Vector3 {
        let points = &graph.edge(edge).physical_geometry;
        let tangent = if graph.edge(edge).start_node == CENTRE {
            points[1] - points[0]
        } else {
            points[points.len() - 1] - points[points.len() - 2]
        }
        .normalized();
        Vector3::new(-tangent.z, 0.0, tangent.x)
    }

    // `edge`'s vehicle lane indices ordered by `lateral` across the road (ascending).
    fn order_by(lanes: &[i8], lateral: impl Fn(i8) -> f32) -> Vec<i8> {
        let mut ordered = lanes.to_vec();
        ordered.sort_by(|&a, &b| lateral(a).total_cmp(&lateral(b)));
        ordered
    }

    // `ROAD-119`: each handle sits on its lane's side and rank across the road, for forward and
    // backward lanes at an edge's start and at its end: the handles' order across the road equals
    // the order of `vehicle_lane_offset` and of the built lanes' ends at the junction.
    #[test]
    fn lane_handles_run_across_the_road_in_lane_order_at_both_edge_ends() {
        let (graph, system) = junction([[2, 2], [3, 1], [1, 3]]);
        let handles = lane_handles(&graph, CENTRE);
        let centre = graph.node(CENTRE).pos;
        for edge in 0..3 {
            let normal = forward_normal(&graph, edge);
            let mine: Vec<&LaneHandle> = handles.iter().filter(|h| h.edge == edge).collect();
            let lanes: Vec<i8> = lane_indices_in_order(graph.edge(edge)).collect();
            assert_eq!(mine.len(), lanes.len(), "edge {edge}: one handle per lane");
            let at_end = graph.edge(edge).end_node == CENTRE;
            for handle in &mine {
                assert_eq!(handle.incoming, (handle.lane_idx >= 0) == at_end);
            }
            let handle_lateral = |lane: i8| {
                let handle = mine.iter().find(|h| h.lane_idx == lane).unwrap();
                (handle.pos - centre).dot(normal)
            };
            let built_lateral = |lane: i8| {
                let built = system
                    .lanes
                    .iter()
                    .find(|l| {
                        l.edge_id == edge && l.lane_type == LaneType::Vehicle && l.lane_idx == lane
                    })
                    .unwrap();
                let (first, last) = (built.geometry[0], *built.geometry.last().unwrap());
                let tip = if first.distance_to(centre) < last.distance_to(centre) {
                    first
                } else {
                    last
                };
                (tip - centre).dot(normal)
            };
            let by_handles = order_by(&lanes, handle_lateral);
            let by_offset = order_by(&lanes, |lane| vehicle_lane_offset(graph.edge(edge), lane));
            let by_lanes = order_by(&lanes, built_lateral);
            assert_eq!(by_handles, by_offset, "edge {edge}: handles against offsets");
            assert_eq!(by_handles, by_lanes, "edge {edge}: handles against the lanes");
        }
    }

    // `ROAD-119`: an open node flags no lane; one vehicle rule whitelists the node, and every
    // approach lane without a listed connection to a lane leaving the node is flagged, exactly
    // the approach lanes the connector builder gives no connector. A rule to a lane that does not
    // leave the node is no movement; a listed U-turn is one.
    #[test]
    fn approach_lanes_without_a_movement_are_flagged_as_the_connectors_leave_them() {
        let (mut graph, _) = junction([[2, 2], [1, 1], [1, 1]]);
        let flagged = |graph: &RegionGraph| -> Vec<(usize, i8)> {
            lane_handles(graph, CENTRE)
                .iter()
                .filter(|h| h.no_movement)
                .map(|h| (h.edge, h.lane_idx))
                .collect()
        };
        assert!(flagged(&graph).is_empty(), "an open node strands no lane");

        // West lane 0 turns into the east road; lane 1, east's backward lane and south's lane
        // have no movement.
        graph.add_lane_connection(CENTRE, 0, 0, 1, 0);
        // A rule naming a lane that arrives at the node, not one leaving it.
        graph.add_lane_connection(CENTRE, 2, 0, 0, 0);
        let mut system = LaneSystem::new();
        system.rebuild(&mut graph);
        let stranded = flagged(&graph);
        assert_eq!(stranded, vec![(0, 1), (1, -1), (2, 0)]);
        for handle in lane_handles(&graph, CENTRE).iter().filter(|h| h.incoming) {
            let lane = system
                .lanes
                .iter()
                .find(|l| {
                    l.edge_id == handle.edge
                        && l.lane_type == LaneType::Vehicle
                        && l.lane_idx == handle.lane_idx
                })
                .unwrap();
            assert_eq!(
                handle.no_movement,
                lane.next_lanes.is_empty(),
                "edge {} lane {}: flag against its connectors",
                handle.edge,
                handle.lane_idx
            );
        }

        // A listed U-turn onto the west road's leaving lane is a movement.
        graph.add_lane_connection(CENTRE, 0, 1, 0, -1);
        assert_eq!(flagged(&graph), vec![(1, -1), (2, 0)]);
    }
}
