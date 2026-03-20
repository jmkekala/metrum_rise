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
        let void_color = Color::from_rgba(0.0, 0.0, 0.0, 0.0); // Shader drops lines natively

        // 0. Pre-calculate Road connection counts to detect dead ends
        let mut connection_counts = HashMap::new();
        for edge in &graph.edges {
            if edge.primary_type != TransitType::Road { continue; }
            *connection_counts.entry(edge.start_node).or_insert(0) += 1;
            *connection_counts.entry(edge.end_node).or_insert(0) += 1;
        }

        // 1. Generate Road Segments (Orthogonal Ribbons using PRE-CLIPPED physical geometry!)
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road { continue; }
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 { continue; }

            let h_offset = 0.001 + (edge_id % 100) as f32 * 0.0001;
            let half_width = edge.width * 0.5;
            
            let mut cumulative_dist = 0.0;

            for i in 0..resampled_count - 1 {
                let mut p0 = edge.physical_geometry[i];
                let mut p1 = edge.physical_geometry[i + 1];
                let segment_vec = p1 - p0;
                let dist = segment_vec.length();
                if dist < 0.1 { continue; }
                let tangent = segment_vec / dist;
                
                p0.y += h_offset; 
                p1.y += h_offset;
                
                let nr0 = self.get_banked_normal(terrain, p0, tangent, half_size);
                let nr1 = self.get_banked_normal(terrain, p1, tangent, half_size);
                
                let s0 = nr0.cross(tangent).normalized() * half_width;
                let s1 = nr1.cross(tangent).normalized() * half_width;
                
                let next_dist = cumulative_dist + dist;
                
                // Purely orthogonal points (Flat Cut)
                let v0_l = p0 - s0;
                let v0_r = p0 + s0;
                let v1_l = p1 - s1;
                let v1_r = p1 + s1;

                vertices.push(v0_l); vertices.push(v0_r); vertices.push(v1_l);
                vertices.push(v1_l); vertices.push(v0_r); vertices.push(v1_r);
                
                let total_lanes = f32::max(1.0, (edge.fwd_lanes + edge.bkw_lanes) as f32);

                let mut push_color = |_d: f32| {
                    colors.push(Color::from_rgba(edge.fwd_lanes as f32 / 10.0, edge.bkw_lanes as f32 / 10.0, 0.0, 1.0));
                };

                push_color(cumulative_dist); push_color(cumulative_dist); push_color(next_dist);
                push_color(next_dist); push_color(cumulative_dist); push_color(next_dist);

                normals.push(nr0); normals.push(nr0); normals.push(nr1);
                normals.push(nr1); normals.push(nr0); normals.push(nr1);
                
                uvs.push(Vector2::new(total_lanes, cumulative_dist)); // v0_l
                uvs.push(Vector2::new(0.0, cumulative_dist));         // v0_r
                uvs.push(Vector2::new(total_lanes, next_dist));       // v1_l
                
                uvs.push(Vector2::new(total_lanes, next_dist));       // v1_l
                uvs.push(Vector2::new(0.0, cumulative_dist));         // v0_r
                uvs.push(Vector2::new(0.0, next_dist));               // v1_r

                cumulative_dist = next_dist;
            }

            // 1.1 Round Caps for Dead Ends
            let cap_steps = 12;
            let normal = Vector3::UP;

            if edge.start_clip == 0.0 && *connection_counts.get(&edge.start_node).unwrap_or(&0) == 1 {
                let p0 = edge.physical_geometry[0];
                let p1 = edge.physical_geometry[1];
                let tangent = (p1 - p0).normalized();
                let base_angle = f32::atan2(-tangent.z, -tangent.x); // Pointing BACK from start
                
                let start_lane_color = Color::from_rgba(edge.fwd_lanes as f32 / 10.0, edge.bkw_lanes as f32 / 10.0, 0.0, 0.0);

                for i in 0..cap_steps {
                    let a1 = base_angle - std::f32::consts::FRAC_PI_2 + (i as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let a2 = base_angle - std::f32::consts::FRAC_PI_2 + ((i + 1) as f32 / cap_steps as f32) * std::f32::consts::PI;
                    
                    let v1 = p0 + Vector3::new(a1.cos(), 0.0, a1.sin()) * half_width;
                    let v2 = p0 + Vector3::new(a2.cos(), 0.0, a2.sin()) * half_width;
                    
                    // Center, V1, V2 CW
                    vertices.push(p0); vertices.push(v1); vertices.push(v2);
                    normals.push(normal); normals.push(normal); normals.push(normal);
                    colors.push(start_lane_color); colors.push(start_lane_color); colors.push(start_lane_color);
                    uvs.push(Vector2::new(p0.x, p0.z)); uvs.push(Vector2::new(v1.x, v1.z)); uvs.push(Vector2::new(v2.x, v2.z));
                }
            }

            if edge.end_clip == 0.0 && *connection_counts.get(&edge.end_node).unwrap_or(&0) == 1 {
                let p_last = *edge.physical_geometry.last().unwrap();
                let p_prev = edge.physical_geometry[resampled_count - 2];
                let tangent = (p_last - p_prev).normalized();
                let base_angle = f32::atan2(tangent.z, tangent.x); // Pointing FORWARD from end
                
                let end_lane_color = Color::from_rgba(edge.fwd_lanes as f32 / 10.0, edge.bkw_lanes as f32 / 10.0, 0.0, 0.0);

                for i in 0..cap_steps {
                    let a1 = base_angle - std::f32::consts::FRAC_PI_2 + (i as f32 / cap_steps as f32) * std::f32::consts::PI;
                    let a2 = base_angle - std::f32::consts::FRAC_PI_2 + ((i + 1) as f32 / cap_steps as f32) * std::f32::consts::PI;
                    
                    let v1 = p_last + Vector3::new(a1.cos(), 0.0, a1.sin()) * half_width;
                    let v2 = p_last + Vector3::new(a2.cos(), 0.0, a2.sin()) * half_width;
                    
                    // Center, V1, V2 CW
                    vertices.push(p_last); vertices.push(v1); vertices.push(v2);
                    normals.push(normal); normals.push(normal); normals.push(normal);
                    colors.push(end_lane_color); colors.push(end_lane_color); colors.push(end_lane_color);
                    uvs.push(Vector2::new(p_last.x, p_last.z)); uvs.push(Vector2::new(v1.x, v1.z)); uvs.push(Vector2::new(v2.x, v2.z));
                }
            }
        }
        
        // 2. Render Solid Asphalt Intersection Hub Polygons perfectly filling the orthogonal gaps
        let mut j_offset = 0.02;
        for (_node_id, poly) in &graph.junction_polygons {
            j_offset += 0.0001;
            
            let void_color = Color::from_rgba(0.0, 0.0, 0.0, 0.0); // Shader drops lines natively
            
            for c in poly.chunks(3) {
                if c.len() == 3 {
                    let mut p0 = c[0]; let mut p1 = c[1]; let mut p2 = c[2];
                    p0.y += j_offset; p1.y += j_offset; p2.y += j_offset;
                    
                    vertices.push(p0); vertices.push(p1); vertices.push(p2);
                    let up = Vector3::UP;
                    normals.push(up); normals.push(up); normals.push(up);
                    
                    uvs.push(Vector2::new(p0.x, p0.z)); 
                    uvs.push(Vector2::new(p1.x, p1.z)); 
                    uvs.push(Vector2::new(p2.x, p2.z));
                    
                    colors.push(void_color); colors.push(void_color); colors.push(void_color);
                }
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
