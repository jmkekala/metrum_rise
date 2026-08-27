//! Lane-specific spatial queries (lane positions, junction connectivity, crosswalks).

use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

impl SimCore {
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
                lane_ids.iter().any(|lane_id| {
                    self.transit_network
                        .lane_system
                        .lanes
                        .get(*lane_id)
                        .and_then(|lane| lane.crosswalk_marking)
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

    /// Returns a list of all lanes at the given node for visual tool feedback.
    pub fn get_node_lanes_internal(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();

        let valid_node_id = self.region_graph.get_valid_node(node_id);
        if valid_node_id as usize >= self.region_graph.node_count() {
            return arr;
        }

        let junction_pos = self.region_graph.node(valid_node_id).pos;

        for (e_id, edge) in self.region_graph.edges().iter().enumerate() {
            // Check both ends independently
            let check_start = edge.start_node == valid_node_id;
            let check_end = edge.end_node == valid_node_id;

            if !check_start && !check_end {
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
            let lc = geo.len();

            // Process each end that matches this junction.
            // If both match (self-loop), we process it twice for both stub ends!
            let possible_ends = if check_start && check_end {
                vec![true, false]
            } else if check_start {
                vec![true]
            } else {
                vec![false]
            };

            for is_start_side in possible_ends {
                // 1. Establish robust "Into-the-Leg" direction
                // ANCHOR: We must skip the "stub" (points near the center from merged nodes)
                // Search for the first point at least 3.1m away (HUB_RADIUS + margin)
                const SEARCH_RADIUS: f32 = 3.1;
                let mut diff = Vector3::ZERO;
                let mut best_stub = Vector3::ZERO;

                if is_start_side {
                    for j in 0..lc {
                        let d = geo[j] - junction_pos;
                        if d.length() > SEARCH_RADIUS {
                            diff = d;
                            break;
                        }
                        if d.length() > 0.1 {
                            best_stub = d;
                        }
                    }
                } else {
                    for j in (0..lc).rev() {
                        let d = geo[j] - junction_pos;
                        if d.length() > SEARCH_RADIUS {
                            diff = d;
                            break;
                        }
                        if d.length() > 0.1 {
                            best_stub = d;
                        }
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
                        diff = self.region_graph.node(other_node).pos - junction_pos;
                    }
                }

                if diff.length_squared() < 1e-6 {
                    continue;
                }
                let dir_to_leg = diff.normalized();

                // ANCHOR: Use a CONSISTENT Forward Tangent to prevent side-flipping (criss-cross)
                // If at start, dir_to_leg is forward. If at end, -dir_to_leg is forward.
                let forward_tangent = if is_start_side {
                    dir_to_leg
                } else {
                    -dir_to_leg
                };
                let road_normal = Vector3::new(-forward_tangent.z, 0.0, forward_tangent.x);

                // 2. Base position offset (5.0m ensures it's clearly past the 3.0m hub)
                let mut current_pos = junction_pos + dir_to_leg * 5.0;
                current_pos.y += 0.4;

                let fwd_lanes = edge.fwd_lane_count();
                let bkw_lanes = edge.bkw_lane_count();
                let total_lanes = (fwd_lanes + bkw_lanes) as i32;
                let lane_w = 1.0;

                // Process ALL lanes at this end
                for l_idx in 0..total_lanes {
                    let is_fwd = l_idx < fwd_lanes as i32;
                    // RHT Logic: Fwd lanes (lower indices) stay on the Right (+lateral_offset)
                    let lateral_offset = (total_lanes as f32 * 0.5 - l_idx as f32 - 0.5) * lane_w;

                    // Always use road_normal for lateral placement
                    let mut lane_pos = current_pos + road_normal * lateral_offset;
                    lane_pos.y += 0.2; // Slightly lower spheres for schematic view

                    let mut dict = VarDictionary::new();
                    dict.set("edge_id", e_id as i32);
                    dict.set(
                        "lane_id",
                        if is_fwd {
                            l_idx
                        } else {
                            -(l_idx - fwd_lanes as i32 + 1)
                        },
                    );
                    dict.set(
                        "is_incoming",
                        if is_fwd {
                            !is_start_side
                        } else {
                            is_start_side
                        },
                    );
                    dict.set("pos", lane_pos);
                    arr.push(&dict.to_variant());
                }
            }
        }
        arr
    }
}
