use crate::config::CLEARANCE_THRESHOLD;
use godot::prelude::*;
use super::{ZoningSystem, ZoneType};
use crate::simulation::network::types::EdgeClass;

impl ZoningSystem {
    /// Tests whether a specific cell is obstructed by asphalt, other road footprints, or competitor zoning.
    pub fn is_cell_obstructed(
        &self,
        edge_idx: usize,
        side: i8,
        x: usize,
        y: usize,
        graph: &crate::simulation::network::graph::RegionGraph,
        nearby_edges_cache: Option<&[usize]>,
    ) -> bool {
        let center = self.get_cell_center(edge_idx, side, x, y, graph);
        if center.x == 0.0 && center.y == 0.0 {
            return true;
        }

        let size = self.config.zone_cell_m;
        let edge = graph.edge(edge_idx);

        let t_us = (x as f32 + 0.5) * size / edge.physical_length;

        // 1. JUNCTION BOX GUARD
        if edge.start_clip > 0.0 {
            let sn = graph.node(edge.start_node).pos;
            let d2 = (center.x - sn.x).powi(2) + (center.y - sn.z).powi(2);
            if d2 < edge.start_clip * edge.start_clip {
                return true;
            }
        }
        if edge.end_clip > 0.0 {
            let en = graph.node(edge.end_node).pos;
            let d2 = (center.x - en.x).powi(2) + (center.y - en.z).powi(2);
            if d2 < edge.end_clip * edge.end_clip {
                return true;
            }
        }

        let check_pts_with_t = vec![(center, t_us)];

        for (pt, t_us) in check_pts_with_t {
            let my_y = edge.get_y_at_t(t_us);
            let mut closest_competitor_has_zone = false;
            let mut asphalt_collision = false;

            let pt_3d = godot::prelude::Vector3::new(pt.x, 0.0, pt.y);

            let nearby_edges_vec;
            let nearby_edges = if let Some(cache) = nearby_edges_cache {
                cache
            } else {
                nearby_edges_vec = graph.get_edges_near_point(pt_3d, 120.0);
                &nearby_edges_vec
            };

            for &i in nearby_edges {
                let e = graph.edge(i);
                if e.deleted { continue; }

                let is_self = i == edge_idx;

                // Sister Edge Detection (Road continuation at 2-way junctions)
                let is_sister = i != edge_idx
                    && (e.start_node == edge.start_node
                        || e.start_node == edge.end_node
                        || e.end_node == edge.start_node
                        || e.end_node == edge.end_node)
                    && e.width == edge.width
                    && e.primary_type == edge.primary_type;

                if is_sister {
                    let shared_node = if e.start_node == edge.start_node || e.start_node == edge.end_node {
                        e.start_node
                    } else {
                        e.end_node
                    };
                    let conn_count = graph.node_adjacency(shared_node).len();

                    if conn_count <= 2 {
                        let t1 = if edge.start_node == shared_node {
                            (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized()
                        } else {
                            (edge.physical_geometry[edge.physical_geometry.len() - 2]
                                - edge.physical_geometry[edge.physical_geometry.len() - 1])
                                .normalized()
                        };
                        let t2 = if e.start_node == shared_node {
                            (e.physical_geometry[1] - e.physical_geometry[0]).normalized()
                        } else {
                            (e.physical_geometry[e.physical_geometry.len() - 2]
                                - e.physical_geometry[e.physical_geometry.len() - 1])
                                .normalized()
                        };
                        if t1.dot(t2) < -0.85 {
                            continue;
                        }
                    }
                }

                let pts = &e.physical_geometry;
                if pts.len() < 2 { continue; }

                let mut cur_l_other = 0.0;
                for j in 0..pts.len() - 1 {
                    if is_self { continue; }

                    let p1 = pts[j];
                    let p2 = pts[j + 1];
                    let p1_2d = Vector2::new(p1.x, p1.z);
                    let p2_2d = Vector2::new(p2.x, p2.z);
                    let seg_vec = p2_2d - p1_2d;
                    let l_seg = seg_vec.length();
                    if l_seg == 0.0 { continue; }
                    let l2 = l_seg * l_seg;

                    let t_raw = ((pt.x - p1_2d.x) * seg_vec.x + (pt.y - p1_2d.y) * seg_vec.y) / l2;
                    let is_past_end = t_raw < -1e-4 || t_raw > 1.0 + 1e-4;
                    let t_proj = t_raw.clamp(0.0, 1.0);

                    let proj = p1_2d + seg_vec * t_proj;
                    let d_sq = pt.distance_squared_to(proj);
                    let tangent_other = seg_vec.normalized();
                    let normal_right = Vector2::new(-tangent_other.y, tangent_other.x);
                    let to_pt_other = pt - proj;

                    let other_y = p1.y + (p2.y - p1.y) * t_proj;
                    if (other_y - my_y).abs() > CLEARANCE_THRESHOLD {
                        cur_l_other += l_seg;
                        continue;
                    }

                    // A. Road Footprint Collision
                    let mut hw_other = (e.width * 0.5) + crate::config::SIDEWALK_WIDTH;
                    let is_neighbor = i != edge_idx
                        && (e.start_node == edge.start_node
                            || e.start_node == edge.end_node
                            || e.end_node == edge.start_node
                            || e.end_node == edge.end_node);

                    if is_neighbor {
                        hw_other = e.width * 0.5;
                    }

                    if !is_past_end && d_sq < (hw_other + 0.1).powi(2) {
                        asphalt_collision = true;
                        break; 
                    }

                    // B. Zoning Claim Check
                    if e.class == EdgeClass::Standard {
                        let d_other = f32::sqrt(d_sq);
                        let side_other = if to_pt_other.dot(normal_right) > 0.0 { -1 } else { 1 };
                        let curb_dist_other = e.width * 0.5 + crate::config::SIDEWALK_WIDTH;
                        let row_other = ((d_other - curb_dist_other) / size).floor() as usize;
                        let col_other = ((cur_l_other + t_proj * l_seg) / size).floor() as usize;

                        let depth_other = if side_other > 0 {
                            self.edge_grids.get(&i).map(|g| g.left_depth).unwrap_or(0)
                        } else {
                            self.edge_grids.get(&i).map(|g| g.right_depth).unwrap_or(0)
                        };
                        if !is_past_end && row_other < depth_other {
                            'col_check: for dc in [0_i32, -1, 1] {
                                let c = col_other as i32 + dc;
                                if c >= 0 && self.get_cell(i, side_other, c as usize, row_other) != ZoneType::None {
                                    closest_competitor_has_zone = true;
                                    break 'col_check;
                                }
                            }
                        }
                    }
                    cur_l_other += l_seg;
                }
                if asphalt_collision { break; }
            }

            if asphalt_collision { return true; }

            // 2. Competitor Check
            if closest_competitor_has_zone { return true; }
            
            // 3. SPLAY CHECK
            if y == 0 {
                let t_eps = 5.0 / edge.physical_length.max(10.0);
                let t_low = (t_us - t_eps).max(0.0);
                let t_high = (t_us + t_eps).min(1.0);
                let tan_low = edge.get_tangent_at_t(t_low);
                let tan_high = edge.get_tangent_at_t(t_high);
                let angle_diff = tan_low.angle_to(tan_high).abs();
                let dist = (t_high - t_low) * edge.physical_length;

                if dist > 0.001 {
                    let curvature = angle_diff / dist;
                    let radius = 1.0 / curvature.max(0.0001);
                    let hw = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH;

                    let splay_ratio = (radius + hw) / radius.max(0.1);
                    if splay_ratio > 1.15 {
                        return true;
                    }
                }
            }
        }
        false
    }
}
