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
                let start_pos = graph.nodes[edge.start_node as usize].pos;
                let end_pos = graph.nodes[edge.end_node as usize].pos;

                let d3_s = edge.physical_geometry[1] - start_pos; // ANCHOR: Use start node pos
                node_dirs.entry(edge.start_node).or_default().push((edge_id, Vector2::new(d3_s.x, d3_s.z).normalized()));

                let lc = edge.physical_geometry.len();
                let d3_e = edge.physical_geometry[lc-2] - end_pos; // ANCHOR: Use end node pos
                node_dirs.entry(edge.end_node).or_default().push((edge_id, Vector2::new(d3_e.x, d3_e.z).normalized()));
            }
        }

        // 1. Generate Schematic Lane Ribbons
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road { continue; }
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 { continue; }

            let h_offset = 0.05 + (edge_id % 100) as f32 * 0.001;
            let lane_w = 1.0;
            let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;

            // Generate each lane as a separate set of ribbons
            let lane_count = total_lanes as usize;
            for l_idx in 0..lane_count {
                let is_fwd = l_idx < edge.fwd_lanes as usize;
                let lane_color = if is_fwd { Color::from_rgb(0.1, 0.8, 0.2) } else { Color::from_rgb(0.8, 0.1, 0.2) };
                
                // RHT Logic: Fwd lanes (lower indices) stay on the Right (+lateral_offset)
                let lateral_offset = (total_lanes * 0.5 - l_idx as f32 - 0.5) * lane_w;

                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let diff = p1 - p0;
                    let dist = diff.length();
                    if dist < 0.01 { continue; }
                    let tangent = diff / dist;
                    let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x); // Simple flat side

                    p0.y += h_offset;
                    p1.y += h_offset;

                    let v0_l = p0 + side_dir * (lateral_offset - lane_w * 0.4);
                    let v0_r = p0 + side_dir * (lateral_offset + lane_w * 0.4);
                    let v1_l = p1 + side_dir * (lateral_offset - lane_w * 0.4);
                    let v1_r = p1 + side_dir * (lateral_offset + lane_w * 0.4);

                    vertices.push(v0_l); vertices.push(v0_r); vertices.push(v1_l);
                    vertices.push(v1_l); vertices.push(v0_r); vertices.push(v1_r);
                    for _ in 0..6 {
                        normals.push(Vector3::UP);
                        colors.push(lane_color);
                        uvs.push(Vector2::ZERO);
                    }
                }
            }
        }

        // Junction meshes removed for architectural pivot

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
