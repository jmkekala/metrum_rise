use godot::prelude::*;
use std::collections::HashMap;
use crate::simulation::network::graph::TransitGraph;
use crate::simulation::network::types::TransitType;
use crate::simulation::network::render::TransitRenderer;

pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(&self, graph: &TransitGraph, terrain: &crate::simulation::terrain::TerrainSystem) -> (PackedVector3Array, PackedVector3Array, PackedVector2Array, PackedColorArray) {
        let mut vertices = PackedVector3Array::new();
        let mut normals = PackedVector3Array::new();
        let mut uvs = PackedVector2Array::new();
        let mut colors = PackedColorArray::new();
        let half_size = Vector2::new(terrain.width as f32 * 0.5, terrain.height as f32 * 0.5);
        let _void_color = Color::from_rgba(0.0, 0.0, 0.0, 0.0); // Shader drops lines natively

        // 0. Pre-calculate Road connection counts and directions for overlap checks
        let mut connection_counts = HashMap::new();
        let mut node_dirs: HashMap<u32, Vec<(usize, Vector2)>> = HashMap::new();
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road { continue; }
            *connection_counts.entry(edge.start_node).or_insert(0) += 1;
            *connection_counts.entry(edge.end_node).or_insert(0) += 1;
            
            if edge.physical_geometry.len() >= 2 {
                let d3_s = edge.physical_geometry[1] - edge.physical_geometry[0];
                node_dirs.entry(edge.start_node).or_default().push((edge_id, Vector2::new(d3_s.x, d3_s.z).normalized()));
                let lc = edge.physical_geometry.len();
                let d3_e = edge.physical_geometry[lc-2] - edge.physical_geometry[lc-1];
                node_dirs.entry(edge.end_node).or_default().push((edge_id, Vector2::new(d3_e.x, d3_e.z).normalized()));
            }
        }

        // 1. Generate Road Segments (Orthogonal Ribbons using PRE-CLIPPED physical geometry!)
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road { continue; }
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 { continue; }

            let h_offset = 0.001 + (edge_id % 100) as f32 * 0.0001;
            let half_width = edge.width * 0.5;
            
            let mut cumulative_dist = 0.0;

            // PRE-CALCULATE SIDES (Mitered Joins)
            let mut sides = Vec::with_capacity(resampled_count);
            for i in 0..resampled_count {
                let p = edge.physical_geometry[i];
                let tangent = if i == 0 {
                    let d = edge.physical_geometry[1] - p;
                    if d.length() > 0.001 { d.normalized() } else { Vector3::new(1.0, 0.0, 0.0) }
                } else if i == resampled_count - 1 {
                    let d = p - edge.physical_geometry[i-1];
                    if d.length() > 0.001 { d.normalized() } else { Vector3::new(1.0, 0.0, 0.0) }
                } else {
                    let t_in_d = p - edge.physical_geometry[i-1];
                    let t_out_d = edge.physical_geometry[i+1] - p;
                    let t_in = if t_in_d.length() > 0.001 { t_in_d.normalized() } else { Vector3::new(1.0, 0.0, 0.0) };
                    let t_out = if t_out_d.length() > 0.001 { t_out_d.normalized() } else { Vector3::new(1.0, 0.0, 0.0) };
                    let sum = t_in + t_out;
                    if sum.length() > 0.001 { sum.normalized() } else { t_in }
                };
                
                let nr = self.get_banked_normal(terrain, p, tangent, half_size);
                let cross = nr.cross(tangent);
                let side_dir = if cross.length() > 0.001 { cross.normalized() } else { Vector3::new(0.0, 0.0, 1.0) };
                
                let mut miter_scale = 1.0;
                if i > 0 && i < resampled_count - 1 {
                    let t_in = (p - edge.physical_geometry[i-1]).normalized();
                    let cos_half = tangent.dot(t_in);
                    if cos_half > 0.1 {
                        miter_scale = (1.0 / cos_half).min(2.0); // Cap extreme miters
                    }
                }
                sides.push(side_dir * half_width * miter_scale);
            }

            for i in 0..resampled_count - 1 {
                let mut p0 = edge.physical_geometry[i];
                let mut p1 = edge.physical_geometry[i + 1];
                let segment_vec = p1 - p0;
                let dist = segment_vec.length();
                if dist < 0.1 { continue; }
                let segment_tangent = segment_vec / dist;
                
                p0.y += h_offset; 
                p1.y += h_offset;
                
                let nr0 = self.get_banked_normal(terrain, p0, segment_tangent, half_size);
                let nr1 = self.get_banked_normal(terrain, p1, segment_tangent, half_size);
                
                let s0 = sides[i];
                let s1 = sides[i+1];
                
                let next_dist = cumulative_dist + dist;
                
                let kerb_w = 0.35;
                let kerb_h = 0.05;
                let asph_w_scale = (half_width - kerb_w) / half_width;

                // Mitered asphalt edges (on ground)
                let v0_l = p0 - s0 * asph_w_scale;
                let v0_r = p0 + s0 * asph_w_scale;
                let v1_l = p1 - s1 * asph_w_scale;
                let v1_r = p1 + s1 * asph_w_scale;
                
                // Raised asphalt edges (Kerb inner top)
                let v0_lh = v0_l + nr0 * kerb_h;
                let v0_rh = v0_r + nr0 * kerb_h;
                let v1_lh = v1_l + nr1 * kerb_h;
                let v1_rh = v1_r + nr1 * kerb_h;

                // Outer kerb top points
                let v0_kl = p0 - s0 + nr0 * kerb_h;
                let v0_kr = p0 + s0 + nr0 * kerb_h;
                let v1_kl = p1 - s1 + nr1 * kerb_h;
                let v1_kr = p1 + s1 + nr1 * kerb_h;

                // 1. Asphalt Ground
                vertices.push(v0_l); vertices.push(v0_r); vertices.push(v1_l);
                vertices.push(v1_l); vertices.push(v0_r); vertices.push(v1_r);
                
                // 2. Left Kerb
                // Vertical Riser
                vertices.push(v0_l); vertices.push(v0_lh); vertices.push(v1_lh);
                vertices.push(v0_l); vertices.push(v1_lh); vertices.push(v1_l);
                // Horizontal Top
                vertices.push(v0_lh); vertices.push(v0_kl); vertices.push(v1_kl);
                vertices.push(v0_lh); vertices.push(v1_kl); vertices.push(v1_lh);

                // 3. Right Kerb
                // Vertical Riser
                vertices.push(v0_r); vertices.push(v1_r); vertices.push(v0_rh);
                vertices.push(v0_rh); vertices.push(v1_r); vertices.push(v1_rh);
                // Horizontal Top
                vertices.push(v0_rh); vertices.push(v1_rh); vertices.push(v0_kr);
                vertices.push(v0_kr); vertices.push(v1_rh); vertices.push(v1_kr);

                let total_lanes = f32::max(1.0, (edge.fwd_lanes + edge.bkw_lanes) as f32);
                let mut lane_color = Color::from_rgba(edge.fwd_lanes as f32 / 10.0, edge.bkw_lanes as f32 / 10.0, 0.0, 1.0);
                let kerb_color = Color::from_rgba(0.0, 0.0, 1.0, 0.0);

                let is_start_crosswalk = *connection_counts.get(&edge.start_node).unwrap_or(&0) >= 3;
                let is_end_crosswalk = *connection_counts.get(&edge.end_node).unwrap_or(&0) >= 3;

                let mut render_start_cw = is_start_crosswalk;
                if i == 0 && is_start_crosswalk {
                    let my_dir = node_dirs.get(&edge.start_node).unwrap().iter().find(|(id, _)| *id == edge_id).unwrap().1;
                    if let Some(others) = node_dirs.get(&edge.start_node) {
                        for &(other_id, other_dir) in others {
                            if other_id < edge_id && my_dir.dot(other_dir) < -0.8 {
                                render_start_cw = false; break;
                            }
                        }
                    }
                }
                let mut render_end_cw = is_end_crosswalk;
                if i == resampled_count - 2 && is_end_crosswalk {
                    let my_dir = node_dirs.get(&edge.end_node).unwrap().iter().find(|(id, _)| *id == edge_id).unwrap().1;
                    if let Some(others) = node_dirs.get(&edge.end_node) {
                        for &(other_id, other_dir) in others {
                            if other_id < edge_id && my_dir.dot(other_dir) < -0.8 {
                                render_end_cw = false; break;
                            }
                        }
                    }
                }

                let seg_len = (p1 - p0).length();
                let mut uv_start = cumulative_dist;
                let mut uv_end = next_dist;

                // Inject Crosswalk flag (COLOR.b around 0.25)
                if (i == 0 && render_start_cw) || (i == resampled_count - 2 && render_end_cw) {
                    lane_color.b = 0.25;
                    uv_start = 0.0;
                    uv_end = seg_len; 
                }

                for _ in 0..6 { colors.push(lane_color); }
                for _ in 0..24 { colors.push(kerb_color); }

                for _ in 0..6 { normals.push(nr0); } 
                for _ in 0..24 { normals.push(Vector3::UP); }
                
                uvs.push(Vector2::new(total_lanes, uv_start)); // v0_l
                uvs.push(Vector2::new(0.0, uv_start));         // v0_r
                uvs.push(Vector2::new(total_lanes, uv_end));   // v1_l
                uvs.push(Vector2::new(total_lanes, uv_end));   // v1_l
                uvs.push(Vector2::new(0.0, uv_start));         // v0_r
                uvs.push(Vector2::new(0.0, uv_end));           // v1_r
                
                for _ in 0..24 { uvs.push(Vector2::new(0.0, 0.0)); } 

                cumulative_dist = next_dist;
            }

            // 1.1 Round Caps for Dead Ends
            let cap_steps = 12;
            let normal = Vector3::UP;

            let kerb_w = 0.35;
            let _kerb_h = 0.05;
            let asph_w = half_width - kerb_w;

            if edge.start_clip == 0.0 && *connection_counts.get(&edge.start_node).unwrap_or(&0) == 1 {
                let p0 = edge.physical_geometry[0];
                let p1 = edge.physical_geometry[1];
                let tangent = (p1 - p0).normalized();
                let base_angle = f32::atan2(-tangent.z, -tangent.x); 
                
                let start_lane_color = Color::from_rgba(edge.fwd_lanes as f32 / 10.0, edge.bkw_lanes as f32 / 10.0, 0.0, 0.0);
                let kerb_color = Color::from_rgba(0.0, 0.0, 1.0, 0.0);

                for i in 0..cap_steps {
                    let a1 = base_angle - std::f32::consts::FRAC_PI_2 + (i as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let a2 = base_angle - std::f32::consts::FRAC_PI_2 + ((i + 1) as f32 / cap_steps as f32) * std::f32::consts::PI;
                    
                    let d1 = Vector3::new(a1.cos(), 0.0, a1.sin());
                    let d2 = Vector3::new(a2.cos(), 0.0, a2.sin());
                    
                    let v1_inner = p0 + d1 * asph_w;
                    let v2_inner = p0 + d2 * asph_w;
                    let v1_outer = p0 + d1 * half_width + Vector3::UP * 0.05;
                    let v2_outer = p0 + d2 * half_width + Vector3::UP * 0.05;
                    let v1_top_in = p0 + d1 * asph_w + Vector3::UP * 0.05;
                    let v2_top_in = p0 + d2 * asph_w + Vector3::UP * 0.05;
                    
                    // Asphalt Cap
                    vertices.push(p0); vertices.push(v1_inner); vertices.push(v2_inner);
                    normals.push(normal); normals.push(normal); normals.push(normal);
                    colors.push(start_lane_color); colors.push(start_lane_color); colors.push(start_lane_color);
                    uvs.push(Vector2::new(p0.x, p0.z)); uvs.push(Vector2::new(v1_inner.x, v1_inner.z)); uvs.push(Vector2::new(v2_inner.x, v2_inner.z));

                    // Kerb Top Cap (Arc)
                    vertices.push(v1_top_in); vertices.push(v1_outer); vertices.push(v2_top_in);
                    vertices.push(v2_top_in); vertices.push(v1_outer); vertices.push(v2_outer);
                    for _ in 0..6 { 
                        normals.push(normal); colors.push(kerb_color); uvs.push(Vector2::ZERO);
                    }
                }
            }

            if edge.end_clip == 0.0 && *connection_counts.get(&edge.end_node).unwrap_or(&0) == 1 {
                let p_last = *edge.physical_geometry.last().unwrap();
                let p_prev = edge.physical_geometry[resampled_count - 2];
                let tangent = (p_last - p_prev).normalized();
                let base_angle = f32::atan2(tangent.z, tangent.x); 
                
                let end_lane_color = Color::from_rgba(edge.fwd_lanes as f32 / 10.0, edge.bkw_lanes as f32 / 10.0, 0.0, 0.0);
                let kerb_color = Color::from_rgba(0.0, 0.0, 1.0, 0.0);

                let kerb_h = 0.05;
                for i in 0..cap_steps {
                    let a1 = base_angle - std::f32::consts::FRAC_PI_2 + (i as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let a2 = base_angle - std::f32::consts::FRAC_PI_2 + ((i + 1) as f32 / cap_steps as f32) * std::f32::consts::PI;
                    
                    let d1 = Vector3::new(a1.cos(), 0.0, a1.sin());
                    let d2 = Vector3::new(a2.cos(), 0.0, a2.sin());

                    let v1_inner = p_last + d1 * asph_w;
                    let v2_inner = p_last + d2 * asph_w;
                    let v1_outer = p_last + d1 * half_width + Vector3::UP * kerb_h;
                    let v2_outer = p_last + d2 * half_width + Vector3::UP * kerb_h;
                    let v1_top_in = p_last + d1 * asph_w + Vector3::UP * kerb_h;
                    let v2_top_in = p_last + d2 * asph_w + Vector3::UP * kerb_h;
                    
                    // Asphalt Cap
                    vertices.push(p_last); vertices.push(v1_inner); vertices.push(v2_inner);
                    normals.push(normal); normals.push(normal); normals.push(normal);
                    colors.push(end_lane_color); colors.push(end_lane_color); colors.push(end_lane_color);
                    uvs.push(Vector2::new(p_last.x, p_last.z)); uvs.push(Vector2::new(v1_inner.x, v1_inner.z)); uvs.push(Vector2::new(v2_inner.x, v2_inner.z));

                    // Kerb Top Cap (Arc)
                    vertices.push(v1_top_in); vertices.push(v1_outer); vertices.push(v2_top_in);
                    vertices.push(v2_top_in); vertices.push(v1_outer); vertices.push(v2_outer);
                    for _ in 0..6 { 
                        normals.push(normal); colors.push(kerb_color); uvs.push(Vector2::ZERO);
                    }
                }
            }
        }
        
        // 2. Render Solid Asphalt Intersection Hub Polygons perfectly filling the orthogonal gaps
        let _j_offset = 0.02;
        // 2. Render Junction Polygons (Hubs)
        for (_node_id, j_mesh) in &graph.junction_polygons {
            for i in 0..j_mesh.vertices.len() {
                vertices.push(j_mesh.vertices[i]);
                normals.push(Vector3::UP);
                uvs.push(j_mesh.uvs[i]);
                colors.push(j_mesh.colors[i]);
            }
        }

        (vertices, normals, uvs, colors)
    }
}

impl RoadRenderer {
    fn get_banked_normal(&self, _terrain: &crate::simulation::terrain::TerrainSystem, _p: Vector3, _tangent: Vector3, _half_size: Vector2) -> Vector3 {
        Vector3::UP
    }
}
