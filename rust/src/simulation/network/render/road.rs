//! Road mesh generation: lane ribbons, junction fill polygons, bridge decks, and walkway strips.
//!
//! [`RoadRenderer`] implements [`TransitRenderer`] and is the sole entry point for converting
//! a [`RegionGraph`] into renderable geometry. All output is flat-shaded triangle soup uploaded
//! to Godot via [`NetworkMeshData`].

use godot::prelude::*;
use crate::config;
use std::collections::HashMap;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{TransitType, EdgeClass};
use super::{TransitRenderer, NetworkMeshData};

fn line_line_intersection_xz(p1: Vector3, d1: Vector3, p2: Vector3, d2: Vector3) -> Option<Vector3> {
    let det = d1.x * (-d2.z) - (-d2.x) * d1.z;
    if det.abs() < 1e-6 {
        return None;
    }

    let dx = p2.x - p1.x;
    let dz = p2.z - p1.z;

    let t = (dx * (-d2.z) - (-d2.x) * dz) / det;
    Some(p1 + d1 * t)
}

/// Generates road, junction, bridge, and walkway mesh data from the road graph.
pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(&self, graph: &RegionGraph, terrain: &crate::simulation::terrain::TerrainSystem) -> NetworkMeshData {
        let mut vertices = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut colors = Vec::new();
        
        let mut marking_vertices = Vec::new();
        let mut marking_normals = Vec::new();
        let mut marking_uvs = Vec::new();
        let mut marking_colors = Vec::new();

        let mut concrete_vertices = Vec::new();
        let mut concrete_normals = Vec::new();
        let mut concrete_uvs = Vec::new();
        let mut concrete_colors = Vec::new();

        let _hw = (terrain.width as f32 - 1.0) * 0.5;
        let _hh = (terrain.height as f32 - 1.0) * 0.5;

        // 1. Build node-to-edge adjacency
        let mut node_to_edges = vec![Vec::new(); graph.nodes.len()];
        for (e_idx, edge) in graph.edges.iter().enumerate() {
            if edge.deleted || (edge.primary_type != TransitType::Road && edge.primary_type != TransitType::Foot) { continue; }
            node_to_edges[graph.get_valid_node(edge.start_node) as usize].push((e_idx, edge));
            node_to_edges[graph.get_valid_node(edge.end_node) as usize].push((e_idx, edge));
        }

        // 2. Compute connection counts and miter directions
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

        let mut node_miters: HashMap<u32, Vector2> = HashMap::new();

        for (&node_id, dirs) in &node_dirs {
            if dirs.len() != 2 { continue; }
            let d1 = dirs[0].1;
            let d2 = dirs[1].1;
            let s1 = Vector2::new(-d1.y, d1.x);
            let s2 = Vector2::new(-d2.y, d2.x);
            let d_diff = s1 - s2;
            if d_diff.length_squared() > 1e-6 {
                let miter = d_diff.normalized();
                let cos_half = s1.dot(miter).abs();
                if cos_half > 0.1 {
                    node_miters.insert(node_id, miter / cos_half);
                }
            }
        }



        // 3. Lane ribbons
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

                    let mut v0_l = p0 + side0 * (lateral_offset - lane_w * 0.5);
                    let mut v0_r = p0 + side0 * (lateral_offset + lane_w * 0.5);
                    let mut v1_l = p1 + side1 * (lateral_offset - lane_w * 0.5);
                    let mut v1_r = p1 + side1 * (lateral_offset + lane_w * 0.5);


                    // Area check for degenerate ribbon triangles
                    let area1 = (v1_l - v0_l).cross(v1_r - v0_l).length() * 0.5;
                    let area2 = (v1_r - v0_l).cross(v0_r - v0_l).length() * 0.5;

                    if area1 >= 0.001 || area2 >= 0.001 {
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
                    }
                    dist_acc += segment_len;
                }
            }

            if edge.class == EdgeClass::Bridge {
                let hw = edge.width * 0.5 + config::SIDEWALK_WIDTH;
                let thickness = 1.0;
                let deck_color = Color::from_rgb(0.3, 0.3, 0.31);
                let concrete_color = Color::from_rgba(0.9, 0.9, 0.9, 1.0);
                
                let mut dist_acc = 0.0;
                let mut dist_acc_pillars = 0.0;

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
                    if segment_start < start_clip { t0 = (start_clip - segment_start) / segment_len; }
                    if segment_end > total_len - end_clip { t1 = (total_len - end_clip - segment_start) / segment_len; }

                    let p0 = p0_raw + (p1_raw - p0_raw) * t0;
                    let p1 = p0_raw + (p1_raw - p0_raw) * t1;
                    
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i+1];

                    let p0_l = p0 - side0 * hw; let p0_r = p0 + side0 * hw;
                    let p1_l = p1 - side1 * hw; let p1_r = p1 + side1 * hw;

                    let p0_lb = p0_l - Vector3::UP * thickness; let p0_rb = p0_r - Vector3::UP * thickness;
                    let p1_lb = p1_l - Vector3::UP * thickness; let p1_rb = p1_r - Vector3::UP * thickness;

                    // 1. Sidewalk/Deck structure (Clipped)
                    vertices.push(p0_l); vertices.push(p1_lb); vertices.push(p0_lb);
                    vertices.push(p0_l); vertices.push(p1_l); vertices.push(p1_lb);
                    for _ in 0..6 { normals.push(-side0); colors.push(deck_color); uvs.push(Vector2::ZERO); }

                    vertices.push(p0_r); vertices.push(p0_rb); vertices.push(p1_rb);
                    vertices.push(p0_r); vertices.push(p1_rb); vertices.push(p1_r);
                    for _ in 0..6 { normals.push(side0); colors.push(deck_color); uvs.push(Vector2::ZERO); }

                    vertices.push(p0_lb); vertices.push(p1_rb); vertices.push(p1_lb);
                    vertices.push(p0_lb); vertices.push(p0_rb); vertices.push(p1_rb);
                    for _ in 0..6 { normals.push(Vector3::DOWN); colors.push(deck_color); uvs.push(Vector2::ZERO); }

                    // 2. CLEARANCE-BASED ADDITIONS (Railings & Walls)
                    let p_mid = p0.lerp(p1, 0.5);
                    let gx = p_mid.x + _hw; let gz = p_mid.z + _hh;
                    let terrain_y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
                    let clearance = p_mid.y - terrain_y;

                    // RAILINGS (1.2m tall, 10cm thick)
                    let rail_h = 1.2; let rail_t = 0.1;
                    let p0_lo = p0_l + side0 * rail_t; let p1_lo = p1_l + side0 * rail_t; 
                    let p0_lt = p0_l + Vector3::UP * rail_h; let p1_lt = p1_l + Vector3::UP * rail_h;
                    let p0_lto = p0_lo + Vector3::UP * rail_h; let p1_lto = p1_lo + Vector3::UP * rail_h;

                    // Inner face
                    concrete_vertices.push(p0_l); concrete_vertices.push(p1_lt); concrete_vertices.push(p0_lt);
                    concrete_vertices.push(p0_l); concrete_vertices.push(p1_l); concrete_vertices.push(p1_lt);
                    for _ in 0..6 { concrete_normals.push(side0); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }
                    // Outer face
                    concrete_vertices.push(p0_lo); concrete_vertices.push(p0_lto); concrete_vertices.push(p1_lto);
                    concrete_vertices.push(p0_lo); concrete_vertices.push(p1_lto); concrete_vertices.push(p1_lo);
                    for _ in 0..6 { concrete_normals.push(-side0); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }
                    // Top face
                    concrete_vertices.push(p0_lt); concrete_vertices.push(p1_lt); concrete_vertices.push(p1_lto);
                    concrete_vertices.push(p0_lt); concrete_vertices.push(p1_lto); concrete_vertices.push(p0_lto);
                    for _ in 0..6 { concrete_normals.push(Vector3::UP); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }

                    let rail_dir_r = -side0;
                    let p0_ro = p0_r + rail_dir_r * rail_t; let p1_ro = p1_r + rail_dir_r * rail_t;
                    let p0_rt = p0_r + Vector3::UP * rail_h; let p1_rt = p1_r + Vector3::UP * rail_h;
                    let p0_rto = p0_ro + Vector3::UP * rail_h; let p1_rto = p1_ro + Vector3::UP * rail_h;

                    // Inner face
                    concrete_vertices.push(p0_r); concrete_vertices.push(p0_rt); concrete_vertices.push(p1_rt);
                    concrete_vertices.push(p0_r); concrete_vertices.push(p1_rt); concrete_vertices.push(p1_r);
                    for _ in 0..6 { concrete_normals.push(-side0); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }
                    // Outer face
                    concrete_vertices.push(p0_ro); concrete_vertices.push(p1_rto); concrete_vertices.push(p0_rto);
                    concrete_vertices.push(p0_ro); concrete_vertices.push(p1_ro); concrete_vertices.push(p1_rto);
                    for _ in 0..6 { concrete_normals.push(side0); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }
                    // Top face
                    concrete_vertices.push(p0_rt); concrete_vertices.push(p0_rto); concrete_vertices.push(p1_rto);
                    concrete_vertices.push(p0_rt); concrete_vertices.push(p1_rto); concrete_vertices.push(p1_rt);
                    for _ in 0..6 { concrete_normals.push(Vector3::UP); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }

                    if clearance <= 5.0 {
                        // SIDE WALLS down to terrain (with 1m sink to prevent floating)
                        let sink = 1.0;
                        let p0_lg = Vector3::new(p0_l.x, (terrain.get_height_interpolated(p0_l.x + _hw, p0_l.z + _hh) * crate::config::HEIGHT_SCALE) - sink, p0_l.z);
                        let p0_rg = Vector3::new(p0_r.x, (terrain.get_height_interpolated(p0_r.x + _hw, p0_r.z + _hh) * crate::config::HEIGHT_SCALE) - sink, p0_r.z);
                        let p1_lg = Vector3::new(p1_l.x, (terrain.get_height_interpolated(p1_l.x + _hw, p1_l.z + _hh) * crate::config::HEIGHT_SCALE) - sink, p1_l.z);
                        let p1_rg = Vector3::new(p1_r.x, (terrain.get_height_interpolated(p1_r.x + _hw, p1_r.z + _hh) * crate::config::HEIGHT_SCALE) - sink, p1_r.z);

                        // Left Wall
                        concrete_vertices.push(p0_l); concrete_vertices.push(p0_lg); concrete_vertices.push(p1_lg);
                        concrete_vertices.push(p0_l); concrete_vertices.push(p1_lg); concrete_vertices.push(p1_l);
                        for _ in 0..6 { concrete_normals.push(-side0); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }

                        // Right Wall
                        concrete_vertices.push(p0_r); concrete_vertices.push(p1_rg); concrete_vertices.push(p0_rg);
                        concrete_vertices.push(p0_r); concrete_vertices.push(p1_r); concrete_vertices.push(p1_rg);
                        for _ in 0..6 { concrete_normals.push(side0); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }
                    } else {
                        // PILLARS every 15m
                        let seg_len = (p1 - p0).length();
                        dist_acc_pillars += seg_len;
                        if dist_acc_pillars >= 15.0 || i == 0 {
                            if i > 0 { dist_acc_pillars = 0.0; }
                            let p_w = edge.width * 0.3; let p_h_top = p_mid.y - thickness; let p_h_bot = terrain_y;
                            let fwd = (p1 - p0).normalized();
                            let c_p0 = p_mid - side0 * (p_w * 0.5); let c_p1 = p_mid + side0 * (p_w * 0.5);
                            let c_p2 = c_p1 + fwd * p_w; let c_p3 = c_p0 + fwd * p_w;
                            let verts = [c_p0, c_p1, c_p2, c_p3];
                            for j in 0..4 {
                                let va = verts[j]; let vb = verts[(j+1)%4];
                                let va_g = Vector3::new(va.x, p_h_bot, va.z); let vb_g = Vector3::new(vb.x, p_h_bot, vb.z);
                                let va_t = Vector3::new(va.x, p_h_top, va.z); let vb_t = Vector3::new(vb.x, p_h_top, vb.z);
                                concrete_vertices.push(va_t); concrete_vertices.push(vb_g); concrete_vertices.push(va_g);
                                concrete_vertices.push(va_t); concrete_vertices.push(vb_t); concrete_vertices.push(vb_g);
                                let n = (vb - va).cross(Vector3::UP).normalized();
                                for _ in 0..6 { concrete_normals.push(n); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }
                            }
                        }
                    }

                    // 3. END CAPS (Only at DEAD ENDS, respecting B_BRIDGE5)
                    let start_deg = *connection_counts.get(&edge.start_node).unwrap_or(&0);
                    let end_deg = *connection_counts.get(&edge.end_node).unwrap_or(&0);
                    let is_start_cap = (i == 0) && (t0 == 0.0) && (start_deg == 1);
                    let is_end_cap = (i == (resampled_count - 2)) && (t1 == 1.0) && (end_deg == 1);
                    if is_start_cap || is_end_cap {
                        let fwd = (p1 - p0).normalized();
                        let (v_l, v_r, v_lb, v_rb, norm) = if is_start_cap {
                            (p0_l, p0_r, p0_lb, p0_rb, -fwd)
                        } else {
                            (p1_l, p1_r, p1_lb, p1_rb, fwd)
                        };
                        concrete_vertices.push(v_l); concrete_vertices.push(v_rb); concrete_vertices.push(v_lb);
                        concrete_vertices.push(v_l); concrete_vertices.push(v_r); concrete_vertices.push(v_rb);
                        for _ in 0..6 { concrete_normals.push(norm); concrete_colors.push(concrete_color); concrete_uvs.push(Vector2::ZERO); }
                    }
                    dist_acc += segment_len;
                }
            }
        }

        // 4. Junction fill polygons
        for (n_idx, node) in graph.nodes.iter().enumerate() {
            if graph.get_valid_node(n_idx as u32) != n_idx as u32 { continue; }
            let edges_at_node = &node_to_edges[n_idx];
            if edges_at_node.len() < 2 { continue; }

            // Skip junction mesh if all edges are tunnels
            if edges_at_node.iter().all(|(_, e)| e.class == EdgeClass::Tunnel) {
                continue;
            }

            struct EdgeInfo {
                clip_pos: Vector3,
                outward_tangent: Vector3,
                side_dir: Vector3,
                road_left: Vector3,
                road_right: Vector3,
                angle: f32,
            }

            let mut edge_infos = Vec::new();
            for &(e_idx, edge) in edges_at_node {
                if edge.deleted { continue; }
                if edge.physical_geometry.len() < 2 { continue; }

                let is_start = edge.start_node == n_idx as u32;
                let clip = if is_start { edge.start_clip } else { edge.end_clip };

                let mut dist_acc = 0.0;
                let total_l = edge.physical_length;
                let target_l = if is_start { clip } else { total_l - clip };

                let mut clip_pos = node.pos;
                let mut tangent = Vector3::FORWARD;
                let geom = &edge.physical_geometry;
                for i in 0..geom.len() - 1 {
                    let p1 = geom[i];
                    let p2 = geom[i + 1];
                    let d = (p2 - p1).length();
                    if dist_acc + d >= target_l || i == geom.len() - 2 {
                        let t = if d > 1e-6 { (target_l - dist_acc) / d } else { 0.0 };
                        clip_pos = p1 + (p2 - p1) * t;
                        tangent = (p2 - p1).normalized();
                        break;
                    }
                    dist_acc += d;
                }

                let outward_tangent = if is_start { tangent } else { -tangent };
                let side_dir = Vector3::new(-outward_tangent.z, 0.0, outward_tangent.x);
                let angle = f32::atan2(outward_tangent.z, outward_tangent.x);
                let rw = edge.width * 0.5;

                edge_infos.push(EdgeInfo {
                    clip_pos,
                    outward_tangent,
                    side_dir,
                    road_left: clip_pos - side_dir * rw,
                    road_right: clip_pos + side_dir * rw,
                    angle,
                });
            }

            if edge_infos.len() < 2 { continue; }
            edge_infos.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap());

            let center = {
                let mut sum = Vector3::ZERO;
                for info in &edge_infos {
                    sum += info.road_left;
                    sum += info.road_right;
                }
                let mut c = sum / (edge_infos.len() as f32 * 2.0);
                c.y = node.pos.y;
                c
            };

            // CS-style junction fill: build a polygon ring from outer sidewalk/road edges,
            // then fan-triangulate from center. Each edge contributes three ring vertices:
            // outer_left, outer_right, and the clamped intersection corner with the next edge.
            let mut ring: Vec<Vector3> = Vec::with_capacity(edge_infos.len() * 3);
            for i in 0..edge_infos.len() {
                let a = &edge_infos[i];
                let b = &edge_infos[(i + 1) % edge_infos.len()];

                let outer_left   = a.road_left;
                let outer_right  = a.road_right;
                let b_outer_left = b.road_left;

                ring.push(outer_left);
                ring.push(outer_right);

                // Corner: intersection of A's outer-right line with B's outer-left line.
                let a_ow = (outer_right - a.clip_pos).length();
                let b_ow = (b_outer_left - b.clip_pos).length();
                let clip_sep = {
                    let dx = a.clip_pos.x - b.clip_pos.x;
                    let dz = a.clip_pos.z - b.clip_pos.z;
                    (dx * dx + dz * dz).sqrt()
                };
                let max_ext = a_ow.max(b_ow) * 3.0 + clip_sep + 1.0;

                let mut corner = line_line_intersection_xz(
                    outer_right,   a.outward_tangent,
                    b_outer_left,  b.outward_tangent,
                ).unwrap_or_else(|| outer_right.lerp(b_outer_left, 0.5));

                let d_a = (corner - a.clip_pos).dot(a.outward_tangent);
                let d_b = (corner - b.clip_pos).dot(b.outward_tangent);
                let dist_from_center = {
                    let dx = corner.x - center.x;
                    let dz = corner.z - center.z;
                    (dx * dx + dz * dz).sqrt()
                };
                // d > 0 means corner is behind the clip plane (outward spike) — clamp it.
                if d_a > 0.0 || d_b > 0.0 || dist_from_center > max_ext {
                    corner = outer_right.lerp(b_outer_left, 0.5);
                }
                ring.push(corner);
            }

            // Fan-triangulate ring from center.
            let road_color = Color::from_rgba(1.0, 1.0, 1.0, 0.5);
            let mut c = center;
            c.y += config::ROAD_H_OFFSET;
            for j in 0..ring.len() {
                let mut p1 = ring[j];
                p1.y += config::ROAD_H_OFFSET;
                let mut p2 = ring[(j + 1) % ring.len()];
                p2.y += config::ROAD_H_OFFSET;
                let cross = (p1.x - c.x) * (p2.z - c.z) - (p1.z - c.z) * (p2.x - c.x);
                if cross.abs() * 0.5 > 0.001 {
                    let (ta, tb) = if cross >= 0.0 { (p1, p2) } else { (p2, p1) };
                    vertices.push(c);
                    vertices.push(ta);
                    vertices.push(tb);
                    for _ in 0..3 {
                        normals.push(Vector3::UP);
                        colors.push(road_color);
                        uvs.push(Vector2::new(c.x, c.z));
                    }
                }
            }
        }

        NetworkMeshData {
            vertices, normals, uvs, colors,
            marking_vertices, marking_normals, marking_uvs, marking_colors,
            concrete_vertices, concrete_normals, concrete_uvs, concrete_colors,
        }
    }
}
