use godot::prelude::*;
use crate::config;
use std::collections::HashMap;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{TransitType, EdgeClass};
use super::{TransitRenderer, NetworkMeshData};

pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(&self, graph: &RegionGraph, terrain: &crate::simulation::terrain::TerrainSystem) -> NetworkMeshData {
        let mut vertices = PackedVector3Array::new();
        let mut normals = PackedVector3Array::new();
        let mut uvs = PackedVector2Array::new();
        let mut colors = PackedColorArray::new();
        
        let marking_vertices = PackedVector3Array::new();
        let marking_normals = PackedVector3Array::new();
        let marking_uvs = PackedVector2Array::new();
        let marking_colors = PackedColorArray::new();

        let _half_size = Vector2::new(terrain.width as f32 * 0.5, terrain.height as f32 * 0.5);

        // 0. Connection mapping and Miter calculation
        let mut connection_counts = HashMap::new();
        let mut node_dirs: HashMap<u32, Vec<(usize, Vector2)>> = HashMap::new();
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.deleted { continue; }
            if edge.primary_type != TransitType::Road && edge.primary_type != TransitType::Foot { continue; }
            *connection_counts.entry(edge.start_node).or_insert(0) += 1;
            *connection_counts.entry(edge.end_node).or_insert(0) += 1;
            
            if edge.physical_geometry.len() >= 2 {
                let start_pos = graph.nodes[edge.start_node as usize].pos;
                let end_pos = graph.nodes[edge.end_node as usize].pos;

                let d3_s = edge.physical_geometry[1] - start_pos;
                let d2_s = Vector2::new(d3_s.x, d3_s.z);
                if d2_s.length_squared() > 1e-6 {
                    node_dirs.entry(edge.start_node).or_default().push((edge_id, d2_s.normalized()));
                }

                let lc = edge.physical_geometry.len();
                let d3_e = edge.physical_geometry[lc-2] - end_pos;
                let d2_e = Vector2::new(d3_e.x, d3_e.z);
                if d2_e.length_squared() > 1e-6 {
                    node_dirs.entry(edge.end_node).or_default().push((edge_id, d2_e.normalized()));
                }
            }
        }

        // Calculate miters for 2-edge joints
        let mut node_miters: HashMap<u32, Vector2> = HashMap::new();
        for (node_id, dirs) in &node_dirs {
            if dirs.len() == 2 {
                let d1 = dirs[0].1;
                let d2 = dirs[1].1;
                
                let s1 = Vector2::new(-d1.y, d1.x);
                let s2 = Vector2::new(-d2.y, d2.x);
                
                // Miter direction is the average of the two side directions
                // Note: we need to handle the case where they are nearly opposite (sharp turn)
                let d_diff = s1 - s2;
                if d_diff.length_squared() > 1e-6 {
                    let miter = d_diff.normalized();
                    // Scale miter length: len = 1.0 / cos(half_angle)
                    let cos_half = s1.dot(miter).abs();
                    if cos_half > 0.1 {
                        node_miters.insert(*node_id, miter / cos_half);
                    }
                }
            }
        }

        // 1. Generate Schematic Lane Ribbons
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 { continue; }

            let h_offset = config::ROAD_H_OFFSET + (edge_id % 100) as f32 * 0.001;
            let z_bias = config::Z_FIGHT_BIAS;

            if edge.primary_type == TransitType::Foot {
                // Walkway Rendering: Single center ribbon
                let lane_color = Color::from_rgb(0.4, 0.4, 0.45); // Darker grey for paths
                let lane_w = 1.0;
                
                // Pre-calculate side directions for miter joins
                let mut point_side_dirs = Vec::with_capacity(resampled_count);
                for i in 0..resampled_count {
                    if i == 0 {
                        if let Some(miter) = node_miters.get(&edge.start_node) {
                            point_side_dirs.push(Vector3::new(miter.x, 0.0, miter.y));
                        } else {
                            let d = edge.physical_geometry[1] - edge.physical_geometry[0];
                            let tangent = if d.length_squared() > 1e-6 { d.normalized() } else { Vector3::FORWARD };
                            point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                        }
                    } else if i == resampled_count - 1 {
                        if let Some(miter) = node_miters.get(&edge.end_node) {
                            point_side_dirs.push(Vector3::new(-miter.x, 0.0, -miter.y));
                        } else {
                            let d = edge.physical_geometry[i] - edge.physical_geometry[i-1];
                            let tangent = if d.length_squared() > 1e-6 { d.normalized() } else { Vector3::FORWARD };
                            point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                        }
                    } else {
                        let d = edge.physical_geometry[i+1] - edge.physical_geometry[i-1];
                        let tangent = if d.length_squared() > 1e-6 { d.normalized() } else { Vector3::FORWARD };
                        point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                    }
                }

                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i+1];

                    let dist = (p1 - p0).length();
                    if dist < 0.01 { continue; }

                    p0.y += h_offset + z_bias;
                    p1.y += h_offset + z_bias;

                    let v0_l = p0 - side0 * (lane_w * 0.5);
                    let v0_r = p0 + side0 * (lane_w * 0.5);
                    let v1_l = p1 - side1 * (lane_w * 0.5);
                    let v1_r = p1 + side1 * (lane_w * 0.5);

                    vertices.push(v0_l); vertices.push(v1_l); vertices.push(v1_r);
                    vertices.push(v0_l); vertices.push(v1_r); vertices.push(v0_r);
                    for _ in 0..6 {
                        normals.push(Vector3::UP);
                        colors.push(lane_color);
                        uvs.push(Vector2::ZERO);
                    }
                }
                continue; // Skip the road-specific sidewalk/lane logic below
            }

            if edge.class == EdgeClass::Tunnel {
                // Portal entrance quads at endpoints only
                for &p_idx in &[0, resampled_count - 1] {
                    let p = edge.physical_geometry[p_idx];
                    let tangent = if p_idx == 0 { edge.physical_geometry[1] - edge.physical_geometry[0] } else { edge.physical_geometry[p_idx] - edge.physical_geometry[p_idx-1] };
                    let tangent = if tangent.length_squared() > 1e-6 { tangent.normalized() } else { Vector3::FORWARD };
                    let side = Vector3::new(-tangent.z, 0.0, tangent.x);
                    let hw = edge.width * 0.5 + config::SIDEWALK_WIDTH;
                    let hh = 4.0; // Portal height
                    
                    let p_elevated = p + Vector3::UP * 0.2;
                    let v_bl = p_elevated - side * hw;
                    let v_br = p_elevated + side * hw;
                    let v_tl = v_bl + Vector3::UP * hh;
                    let v_tr = v_br + Vector3::UP * hh;

                    vertices.push(v_bl); vertices.push(v_tl); vertices.push(v_tr);
                    vertices.push(v_bl); vertices.push(v_tr); vertices.push(v_br);
                    for _ in 0..6 { normals.push(-tangent); colors.push(Color::from_rgb(0.1, 0.1, 0.1)); uvs.push(Vector2::ZERO); }
                }
                continue;
            }

            if edge.primary_type != TransitType::Road { continue; }

            let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;
            let lane_w = edge.width / total_lanes;

            // Pre-calculate side directions for miter joins
            let mut point_side_dirs = Vec::with_capacity(resampled_count);
            for i in 0..resampled_count {
                let d = if i == 0 {
                    edge.physical_geometry[1] - edge.physical_geometry[0]
                } else if i == resampled_count - 1 {
                    edge.physical_geometry[i] - edge.physical_geometry[i-1]
                } else {
                    edge.physical_geometry[i+1] - edge.physical_geometry[i-1]
                };
                let tangent = if d.length_squared() > 1e-6 { d.normalized() } else { Vector3::FORWARD };
                let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);
                point_side_dirs.push(side_dir);
            }

            // Lane Ribbons
            // Road Ribbons (Lanes)
            let lane_count = (edge.fwd_lanes + edge.bkw_lanes) as usize;
            let start_clip = edge.start_clip;
            let end_clip = edge.end_clip;
            let total_len = edge.physical_length;

            for l_idx in 0..lane_count {
                let lateral_offset = (total_lanes * 0.5 - l_idx as f32 - 0.5) * lane_w;
                let mut dist_acc = 0.0;

                for i in 0..resampled_count - 1 {
                    let p0_raw = edge.physical_geometry[i];
                    let p1_raw = edge.physical_geometry[i + 1];
                    let segment_len = (p1_raw - p0_raw).length();
                    
                    let segment_start = dist_acc;
                    let segment_end = dist_acc + segment_len;

                    // Skip if entirely clipped
                    if segment_end <= start_clip || segment_start >= total_len - end_clip {
                        dist_acc += segment_len;
                        continue;
                    }

                    // Clip the segment
                    let mut t0 = 0.0f32;
                    let mut t1 = 1.0f32;
                    if segment_start < start_clip {
                        t0 = (start_clip - segment_start) / segment_len;
                    }
                    if segment_end > total_len - end_clip {
                        t1 = (total_len - end_clip - segment_start) / segment_len;
                    }

                    let mut p0 = p0_raw + (p1_raw - p0_raw) * t0;
                    let mut p1 = p0_raw + (p1_raw - p0_raw) * t1;
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i+1];

                    p0.y += h_offset;
                    p1.y += h_offset;

                    let v0_l = p0 + side0 * (lateral_offset - lane_w * 0.5);
                    let v0_r = p0 + side0 * (lateral_offset + lane_w * 0.5);
                    let v1_l = p1 + side1 * (lateral_offset - lane_w * 0.5);
                    let v1_r = p1 + side1 * (lateral_offset + lane_w * 0.5);

                    vertices.push(v0_l); vertices.push(v1_l); vertices.push(v1_r);
                    vertices.push(v0_l); vertices.push(v1_r); vertices.push(v0_r);
                    
                    let uv_y0_l = 0.0; let uv_y0_r = 1.0;
                    uvs.push(Vector2::new(segment_start + t0 * segment_len, uv_y0_l));
                    uvs.push(Vector2::new(segment_start + t1 * segment_len, uv_y0_l));
                    uvs.push(Vector2::new(segment_start + t1 * segment_len, uv_y0_r));
                    uvs.push(Vector2::new(segment_start + t0 * segment_len, uv_y0_l));
                    uvs.push(Vector2::new(segment_start + t1 * segment_len, uv_y0_r));
                    uvs.push(Vector2::new(segment_start + t0 * segment_len, uv_y0_r));

                    let is_lane_boundary = if l_idx > 0 { 1.0 } else { 0.0 };
                    let is_center_boundary = if l_idx == edge.fwd_lanes as usize && edge.fwd_lanes > 0 && edge.bkw_lanes > 0 { 1.0 } else { 0.0 };

                    for _ in 0..6 {
                        normals.push(Vector3::UP);
                        colors.push(Color::from_rgba(1.0, is_lane_boundary, is_center_boundary, 0.0));
                    }
                    dist_acc += segment_len;
                }
            }

            // Sidewalk Ribbons
            let sw_color = Color::from_rgb(1.0, 1.0, 1.0);
            let sw_w = config::SIDEWALK_WIDTH;
            let sw_offsets = [edge.width * 0.5 + sw_w * 0.5, -(edge.width * 0.5 + sw_w * 0.5)];

            for &lateral_offset in &sw_offsets {
                let mut dist_acc = 0.0;
                for i in 0..resampled_count - 1 {
                    let p0_raw = edge.physical_geometry[i];
                    let p1_raw = edge.physical_geometry[i + 1];
                    let segment_len = (p1_raw - p0_raw).length();
                    let segment_start = dist_acc;
                    let segment_end = dist_acc + segment_len;

                    if segment_end <= start_clip || segment_start >= total_len - end_clip {
                        dist_acc += segment_len;
                        continue;
                    }

                    let mut t0 = 0.0f32;
                    let mut t1 = 1.0f32;
                    if segment_start < start_clip { t0 = (start_clip - segment_start) / segment_len; }
                    if segment_end > total_len - end_clip { t1 = (total_len - end_clip - segment_start) / segment_len; }

                    let mut p0 = p0_raw + (p1_raw - p0_raw) * t0;
                    let mut p1 = p0_raw + (p1_raw - p0_raw) * t1;
                    
                    p0.y += h_offset + 0.001;
                    p1.y += h_offset + 0.001;

                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i+1];

                    let v0_l = p0 + side0 * (lateral_offset - sw_w * 0.5);
                    let v0_r = p0 + side0 * (lateral_offset + sw_w * 0.5);
                    let v1_l = p1 + side1 * (lateral_offset - sw_w * 0.5);
                    let v1_r = p1 + side1 * (lateral_offset + sw_w * 0.5);

                    vertices.push(v0_l); vertices.push(v1_l); vertices.push(v1_r);
                    vertices.push(v0_l); vertices.push(v1_r); vertices.push(v0_r);

                    let (uv_y_inner, uv_y_outer) = if lateral_offset > 0.0 { (0.0, 1.0) } else { (1.0, 0.0) };
                    uvs.push(Vector2::new(segment_start + t0 * segment_len, uv_y_inner));
                    uvs.push(Vector2::new(segment_start + t0 * segment_len, uv_y_outer));
                    uvs.push(Vector2::new(segment_start + t1 * segment_len, uv_y_outer));
                    uvs.push(Vector2::new(segment_start + t0 * segment_len, uv_y_inner));
                    uvs.push(Vector2::new(segment_start + t1 * segment_len, uv_y_outer));
                    uvs.push(Vector2::new(segment_start + t1 * segment_len, uv_y_inner));

                    for _ in 0..6 {
                        normals.push(Vector3::UP);
                        colors.push(sw_color);
                    }
                    dist_acc += segment_len;
                }
            }

            if edge.class == EdgeClass::Bridge {
                let hw = edge.width * 0.5 + config::SIDEWALK_WIDTH;
                let thickness = 1.0;
                let deck_color = Color::from_rgb(0.3, 0.3, 0.31);

                for i in 0..resampled_count - 1 {
                    let p0 = edge.physical_geometry[i];
                    let p1 = edge.physical_geometry[i+1];
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i+1];

                    let p0_l = p0 - side0 * hw; let p0_r = p0 + side0 * hw;
                    let p1_l = p1 - side1 * hw; let p1_r = p1 + side1 * hw;

                    let p0_lb = p0_l - Vector3::UP * thickness; let p0_rb = p0_r - Vector3::UP * thickness;
                    let p1_lb = p1_l - Vector3::UP * thickness; let p1_rb = p1_r - Vector3::UP * thickness;

                    // Left Side Quad
                    vertices.push(p0_l); vertices.push(p1_lb); vertices.push(p0_lb);
                    vertices.push(p0_l); vertices.push(p1_l); vertices.push(p1_lb);
                    for _ in 0..6 { normals.push(-side0); colors.push(deck_color); uvs.push(Vector2::ZERO); }

                    // Right Side Quad
                    vertices.push(p0_r); vertices.push(p0_rb); vertices.push(p1_rb);
                    vertices.push(p0_r); vertices.push(p1_rb); vertices.push(p1_r);
                    for _ in 0..6 { normals.push(side0); colors.push(deck_color); uvs.push(Vector2::ZERO); }

                    // Bottom face (inverted Up)
                    vertices.push(p0_lb); vertices.push(p1_rb); vertices.push(p1_lb);
                    vertices.push(p0_lb); vertices.push(p0_rb); vertices.push(p1_rb);
                    for _ in 0..6 { normals.push(Vector3::DOWN); colors.push(deck_color); uvs.push(Vector2::ZERO); }
                }
            }
        }

        // 2. Junction meshes
        // Optimize by pre-building an adjacency list
        let mut node_to_edges = vec![Vec::new(); graph.nodes.len()];
        for (e_idx, edge) in graph.edges.iter().enumerate() {
            if edge.deleted || (edge.primary_type != TransitType::Road && edge.primary_type != TransitType::Foot) { continue; }
            node_to_edges[graph.get_valid_node(edge.start_node) as usize].push((e_idx, edge));
            node_to_edges[graph.get_valid_node(edge.end_node) as usize].push((e_idx, edge));
        }

        for (n_idx, node) in graph.nodes.iter().enumerate() {
            if graph.get_valid_node(n_idx as u32) != n_idx as u32 { continue; }
            let edges_at_node = &node_to_edges[n_idx];
            if edges_at_node.is_empty() { continue; }

            if edges_at_node.len() < 2 { continue; }
            
            // If all edges connecting here are tunnels, hide the junction mesh
            if edges_at_node.iter().all(|(_, e)| e.class == EdgeClass::Tunnel) {
                continue;
            }

            struct JPoint {
                e_idx: usize,
                inner: Vector3,
                outer: Vector3,
                angle: f32,
            }
            let mut j_pts = Vec::new();

            for &(e_idx, edge) in edges_at_node {
                if edge.deleted { continue; }
                if edge.physical_geometry.len() < 2 { continue; }
                
                let is_start = edge.start_node == n_idx as u32;
                let clip = if is_start { edge.start_clip } else { edge.end_clip };
                
                let mut dist_acc = 0.0;
                let total_l = edge.physical_length;
                let target_l = if is_start { clip } else { total_l - clip };
                
                let mut p = node.pos; 
                let mut tangent = Vector3::FORWARD;
                let geom = &edge.physical_geometry;
                for i in 0..geom.len() - 1 {
                    let p1 = geom[i];
                    let p2 = geom[i+1];
                    let d = (p2 - p1).length();
                    if dist_acc + d >= target_l || i == geom.len() - 2 {
                        let t = if d > 1e-6 { (target_l - dist_acc) / d } else { 0.0 };
                        p = p1 + (p2 - p1) * t;
                        tangent = (p2 - p1).normalized();
                        break;
                    }
                    dist_acc += d;
                }

                p.y += config::ROAD_H_OFFSET;
                let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);
                let rw = edge.width * 0.5;
                let sw = rw + config::SIDEWALK_WIDTH;
                
                for side in [1.0, -1.0] {
                    let pt_inner = p + side_dir * (rw * side);
                    let pt_outer = p + side_dir * (sw * side);
                    let da = pt_inner - node.pos;
                    let angle = f32::atan2(da.z, da.x);
                    j_pts.push(JPoint { e_idx, inner: pt_inner, outer: pt_outer, angle });
                }
            }

            if j_pts.len() < 3 { continue; }
            j_pts.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap());

            let mut center = node.pos;
            center.y += config::ROAD_H_OFFSET;

            for i in 0..j_pts.len() {
                let p1 = &j_pts[i];
                let p2 = &j_pts[(i + 1) % j_pts.len()];

                // 1. Asphalt Inner Triangle (Center Fan)
                vertices.push(center);
                vertices.push(p1.inner);
                vertices.push(p2.inner);
                for _ in 0..3 {
                    normals.push(Vector3::UP);
                    colors.push(Color::from_rgba(1.0, 1.0, 1.0, 0.5)); // Asphalt
                    uvs.push(Vector2::new(center.x, center.z));
                }

                // 2. Outer Quad (Between inner and outer points)
                let is_mouth = p1.e_idx == p2.e_idx;
                let alpha = if is_mouth { 0.5 } else { 1.0 };
                
                // Use consistent UV mapping
                let uv1_i = Vector2::new(p1.inner.x * 2.0, 0.0);
                let uv1_o = Vector2::new(p1.outer.x * 2.0, 1.0);
                let uv2_i = Vector2::new(p2.inner.x * 2.0, 0.0);
                let uv2_o = Vector2::new(p2.outer.x * 2.0, 1.0);

                // Quad Winding: p1_i -> p2_o -> p2_inner and p1_i -> p1_o -> p2_o
                vertices.push(p1.inner); vertices.push(p2.outer); vertices.push(p2.inner);
                uvs.push(uv1_i); uvs.push(uv2_o); uvs.push(uv2_i);
                vertices.push(p1.inner); vertices.push(p1.outer); vertices.push(p2.outer);
                uvs.push(uv1_i); uvs.push(uv1_o); uvs.push(uv2_o);

                for _ in 0..6 {
                    normals.push(Vector3::UP);
                    colors.push(Color::from_rgba(1.0, 1.0, 1.0, alpha));
                }
            }
        }

        NetworkMeshData {
            vertices, normals, uvs, colors,
            marking_vertices, marking_normals, marking_uvs, marking_colors,
        }
    }
}
