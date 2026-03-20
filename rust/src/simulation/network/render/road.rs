use godot::prelude::*;
use std::collections::HashMap;
use crate::simulation::network::graph::TransitGraph;
use crate::simulation::network::types::TransitType;
use super::{TransitRenderer, NetworkMeshData};

pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(&self, graph: &TransitGraph, terrain: &crate::simulation::terrain::TerrainSystem) -> NetworkMeshData {
        let mut vertices = PackedVector3Array::new();
        let mut normals = PackedVector3Array::new();
        let mut uvs = PackedVector2Array::new();
        let mut colors = PackedColorArray::new();
        
        let marking_vertices = PackedVector3Array::new();
        let marking_normals = PackedVector3Array::new();
        let marking_uvs = PackedVector2Array::new();
        let marking_colors = PackedColorArray::new();

        let half_size = Vector2::new(terrain.width as f32 * 0.5, terrain.height as f32 * 0.5);

        // 0. Connection mapping
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

        // 1. Generate Road Segments
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road { continue; }
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 { continue; }

            let h_offset = 0.001 + (edge_id % 100) as f32 * 0.0001;
            let half_width = edge.width * 0.5;
            let _lane_w = 3.5;
            
            let mut cumulative_dist = 0.0;

            let mut sides = Vec::with_capacity(resampled_count);
            for i in 0..resampled_count {
                let p = edge.physical_geometry[i];
                let tangent = if i == 0 {
                    let mut dir = Vector3::new(1.0, 0.0, 0.0);
                    for j in 0..edge.geometry.len() - 1 {
                        let d = edge.geometry[j+1] - edge.geometry[j];
                        if d.length() > 0.01 { dir = d.normalized(); break; }
                    }
                    dir
                } else if i == resampled_count - 1 {
                    let mut dir = Vector3::new(1.0, 0.0, 0.0);
                    let lc = edge.geometry.len();
                    for j in (1..lc).rev() {
                        let d = edge.geometry[j] - edge.geometry[j-1];
                        if d.length() > 0.01 { dir = d.normalized(); break; }
                    }
                    dir
                } else {
                    let d1 = p - edge.physical_geometry[i - 1];
                    let d2 = edge.physical_geometry[i + 1] - p;
                    (d1 + d2).normalized()
                };
                
                let nr = self.get_banked_normal(terrain, p, tangent, half_size);
                let cross = nr.cross(tangent);
                let side_dir = if cross.length() > 0.001 { cross.normalized() } else { Vector3::new(0.0, 0.0, 1.0) };
                
                let mut miter_scale = 1.0;
                if i > 0 && i < resampled_count - 1 {
                    let t_in = (p - edge.physical_geometry[i-1]).normalized();
                    let cos_half = tangent.dot(t_in);
                    if cos_half > 0.1 {
                        miter_scale = (1.0 / cos_half).min(2.0);
                    }
                }
                sides.push(side_dir * half_width * miter_scale);
            }
            let mut total_length = 0.0;
            for i in 0..resampled_count - 1 {
                let p0 = edge.physical_geometry[i];
                let p1 = edge.physical_geometry[i+1];
                let d2 = Vector2::new(p1.x - p0.x, p1.z - p0.z).length();
                total_length += d2;
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
                let _nr1 = self.get_banked_normal(terrain, p1, segment_tangent, half_size);
                let s0 = sides[i];
                let s1 = sides[i+1];
                let next_dist = cumulative_dist + dist;

                let asph_w1 = s0.length();
                let asph_w2 = s1.length();

                // 1. Asphalt Layer
                let s0n = s0.normalized();
                let s1n = s1.normalized();
                let v0_l = p0 + s0n * asph_w1;
                let v0_r = p0 - s0n * asph_w1;
                let v1_l = p1 + s1n * asph_w2;
                let v1_r = p1 - s1n * asph_w2;

                let mid = (p0 + p1) * 0.5;
                if v0_l.x.is_nan() || v0_l.z.is_nan() || v1_r.x.is_nan() {
                    println!("CRITICAL: NaN generated in road.rs physical_geometry! Segment [{} -> {}]", i, i+1);
                }
                if (v0_l - mid).length() > 50.0 || (v0_r - mid).length() > 50.0 || (v1_l - mid).length() > 50.0 || (v1_r - mid).length() > 50.0 {
                    println!("CRITICAL: Massive Geometric Spike (>50m) generated in road.rs! Mid: {:?}. Distances: v0_l={}, v0_r={}, v1_l={}, v1_r={}", mid, (v0_l - mid).length(), (v0_r - mid).length(), (v1_l - mid).length(), (v1_r - mid).length());
                }

                let fwd = edge.fwd_lanes as f32 / 10.0;
                let bkw = edge.bkw_lanes as f32 / 10.0;
                let asph_color = Color::from_rgba(fwd, bkw, 0.0, 1.0);

                vertices.push(v0_l); vertices.push(v0_r); vertices.push(v1_l);
                vertices.push(v1_l); vertices.push(v0_r); vertices.push(v1_r);
                for _ in 0..6 { normals.push(nr0); colors.push(asph_color); }
                uvs.push(Vector2::new(asph_w1, cumulative_dist * 0.2)); uvs.push(Vector2::new(-asph_w1, cumulative_dist * 0.2));
                uvs.push(Vector2::new(asph_w1, next_dist * 0.2)); uvs.push(Vector2::new(asph_w1, next_dist * 0.2));
                uvs.push(Vector2::new(-asph_w1, cumulative_dist * 0.2)); uvs.push(Vector2::new(-asph_w1, next_dist * 0.2));
                // Markings removed to use procedural shader lines instead
                cumulative_dist = next_dist;
            }

            // Caps
            let fwd = edge.fwd_lanes as f32 / 10.0;
            let bkw = edge.bkw_lanes as f32 / 10.0;
            let asph_color_cap = Color::from_rgba(fwd, bkw, 0.0, 1.0);

            let cap_steps = 12;
            let asph_w = half_width;
            if edge.start_clip == 0.0 && *connection_counts.get(&edge.start_node).unwrap_or(&0) == 1 {
                let mut p0 = edge.physical_geometry[0];
                p0.y += h_offset; // Synchronize Y elevation exactly flush with adjoining segment baseline!
                
                let tangent = (edge.physical_geometry[1] - p0).normalized();
                let base_angle = f32::atan2(-tangent.z, -tangent.x); 
                for i in 0..cap_steps {
                    let a1 = base_angle - std::f32::consts::FRAC_PI_2 + (i as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let a2 = base_angle - std::f32::consts::FRAC_PI_2 + ((i + 1) as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let d1 = Vector3::new(a1.cos(), 0.0, a1.sin()); let d2 = Vector3::new(a2.cos(), 0.0, a2.sin());
                    // Sweep d2 before d1 permanently locks winding to Face UP in Godot engine space!
                    vertices.push(p0); vertices.push(p0 + d2 * asph_w); vertices.push(p0 + d1 * asph_w);
                    for _ in 0..3 { normals.push(Vector3::UP); colors.push(asph_color_cap); uvs.push(Vector2::ZERO); }
                }
            }
            if edge.end_clip == 0.0 && *connection_counts.get(&edge.end_node).unwrap_or(&0) == 1 {
                let mut p_last = *edge.physical_geometry.last().unwrap();
                p_last.y += h_offset;
                
                let tangent = (p_last - edge.physical_geometry[resampled_count - 2]).normalized();
                let base_angle = f32::atan2(tangent.z, tangent.x); 
                for i in 0..cap_steps {
                    let a1 = base_angle - std::f32::consts::FRAC_PI_2 + (i as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let a2 = base_angle - std::f32::consts::FRAC_PI_2 + ((i + 1) as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let d1 = Vector3::new(a1.cos(), 0.0, a1.sin()); let d2 = Vector3::new(a2.cos(), 0.0, a2.sin());
                    vertices.push(p_last); vertices.push(p_last + d2 * asph_w); vertices.push(p_last + d1 * asph_w);
                    for _ in 0..3 { normals.push(Vector3::UP); colors.push(asph_color_cap); uvs.push(Vector2::ZERO); }
                }
            }
        }

        for (_node_id, j_mesh) in &graph.junction_polygons {
            for i in 0..j_mesh.vertices.len() {
                vertices.push(j_mesh.vertices[i]);
                normals.push(Vector3::UP);
                uvs.push(j_mesh.uvs[i]);
                colors.push(j_mesh.colors[i]);
            }
        }

        NetworkMeshData {
            vertices, normals, uvs, colors,
            marking_vertices, marking_normals, marking_uvs, marking_colors,
        }
    }
}

impl RoadRenderer {
    fn get_banked_normal(&self, _terrain: &crate::simulation::terrain::TerrainSystem, _p: Vector3, _tangent: Vector3, _half_size: Vector2) -> Vector3 {
        Vector3::UP
    }
}
