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
        }
        
        // 2. Render Solid Asphalt Intersection Hub Polygons perfectly filling the orthogonal gaps
        let mut j_offset = 0.005;
        for (_node_id, poly) in &graph.junction_polygons {
            j_offset += 0.0001;
            let mut center = Vector3::ZERO;
            for p in poly { center += *p; }
            let plen = poly.len() as f32;
            center.x /= plen;
            center.y /= plen;
            center.z /= plen;
            center.y += j_offset;
            
            // Generate CCW Triangle Fan facing UP
            for i in 0..poly.len() {
                let mut c1 = poly[i];
                let mut c2 = poly[(i+1) % poly.len()];
                c1.y += j_offset;
                c2.y += j_offset;
                
                vertices.push(center); vertices.push(c1); vertices.push(c2);
                let up = Vector3::UP;
                normals.push(up); normals.push(up); normals.push(up);
                
                uvs.push(Vector2::new(0.5, 0.5)); uvs.push(Vector2::new(0.5, 0.5)); uvs.push(Vector2::new(0.5, 0.5));
                let void_color = Color::from_rgba(0.0, 0.0, 0.0, 0.0); // Shader drops lines natively
                colors.push(void_color); colors.push(void_color); colors.push(void_color);
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
