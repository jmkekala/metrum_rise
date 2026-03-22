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

        let _half_size = Vector2::new(terrain.width as f32 * 0.5, terrain.height as f32 * 0.5);

        // 0. Connection mapping
        let mut connection_counts = HashMap::new();
        let mut node_dirs: HashMap<u32, Vec<(usize, Vector2)>> = HashMap::new();
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road && edge.primary_type != TransitType::Foot { continue; }
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
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 { continue; }

            let h_offset = 0.05 + (edge_id % 100) as f32 * 0.001;

            if edge.primary_type == TransitType::Foot {
                // Walkway Rendering: Single center ribbon
                let lane_color = Color::from_rgb(0.4, 0.4, 0.45); // Darker grey for paths
                let lane_w = 1.0;
                
                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let diff = p1 - p0;
                    let dist = diff.length();
                    if dist < 0.01 { continue; }
                    let tangent = diff / dist;
                    let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);

                    p0.y += h_offset;
                    p1.y += h_offset;

                    let v0_l = p0 - side_dir * (lane_w * 0.5);
                    let v0_r = p0 + side_dir * (lane_w * 0.5);
                    let v1_l = p1 - side_dir * (lane_w * 0.5);
                    let v1_r = p1 + side_dir * (lane_w * 0.5);

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

            if edge.primary_type != TransitType::Road { continue; }

            let lane_w = 1.0;
            let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;

            // Lane Ribbons
            let lane_count = (edge.fwd_lanes + edge.bkw_lanes) as usize;
            for l_idx in 0..lane_count {
                let is_fwd = l_idx < edge.fwd_lanes as usize;
                let lane_color = if is_fwd { Color::from_rgb(0.1, 0.8, 0.2) } else { Color::from_rgb(0.8, 0.1, 0.2) };
                let lateral_offset = (total_lanes * 0.5 - l_idx as f32 - 0.5) * lane_w;
                
                let mut dist_acc = 0.0;
                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let diff = p1 - p0;
                    let dist = diff.length();
                    if dist < 0.01 { continue; }
                    let tangent = diff / dist;
                    let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);

                    p0.y += h_offset;
                    p1.y += h_offset;

                    let v0_l = p0 + side_dir * (lateral_offset - lane_w * 0.5);
                    let v0_r = p0 + side_dir * (lateral_offset + lane_w * 0.5);
                    let v1_l = p1 + side_dir * (lateral_offset - lane_w * 0.5);
                    let v1_r = p1 + side_dir * (lateral_offset + lane_w * 0.5);

                    // UV mapping: x = distance along road, y = normalized offset across entire road width
                    // Mapping across entire road: (offset + half_width) / width
                    let uv_y0_l = (lateral_offset - lane_w * 0.5 + edge.width * 0.5) / edge.width;
                    let uv_y0_r = (lateral_offset + lane_w * 0.5 + edge.width * 0.5) / edge.width;
                    
                    vertices.push(v0_l); vertices.push(v1_l); vertices.push(v1_r);
                    vertices.push(v0_l); vertices.push(v1_r); vertices.push(v0_r);
                    
                    uvs.push(Vector2::new(dist_acc, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_r));
                    
                    uvs.push(Vector2::new(dist_acc, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_r));
                    uvs.push(Vector2::new(dist_acc, uv_y0_r));

                    for _ in 0..6 {
                        normals.push(Vector3::UP);
                        colors.push(Color::from_rgba(1.0, 1.0, 1.0, 0.0));
                    }
                    dist_acc += dist;
                }
            }

            // Sidewalk Ribbons (Grey)
            let sw_color = Color::from_rgb(0.6, 0.6, 0.6);
            let sw_w = 0.5;
            let sw_offsets = [edge.width * 0.5 - 0.25, -(edge.width * 0.5 - 0.25)];
            
            for &lateral_offset in &sw_offsets {
                let mut dist_acc = 0.0;
                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let diff = p1 - p0;
                    let dist = diff.length();
                    if dist < 0.01 { continue; }
                    let tangent = diff / dist;
                    let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);

                    p0.y += h_offset + 0.001;
                    p1.y += h_offset + 0.001;

                    let v0_l = p0 + side_dir * (lateral_offset - sw_w * 0.5);
                    let v0_r = p0 + side_dir * (lateral_offset + sw_w * 0.5);
                    let v1_l = p1 + side_dir * (lateral_offset - sw_w * 0.5);
                    let v1_r = p1 + side_dir * (lateral_offset + sw_w * 0.5);

                    let uv_y0_l = (lateral_offset - sw_w * 0.5 + edge.width * 0.5) / edge.width;
                    let uv_y0_r = (lateral_offset + sw_w * 0.5 + edge.width * 0.5) / edge.width;

                    vertices.push(v0_l); vertices.push(v1_l); vertices.push(v1_r);
                    vertices.push(v0_l); vertices.push(v1_r); vertices.push(v0_r);

                    uvs.push(Vector2::new(dist_acc, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc, uv_y0_r));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_r));
                    
                    uvs.push(Vector2::new(dist_acc, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_r));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_l));

                    for _ in 0..6 {
                        normals.push(Vector3::UP);
                        colors.push(sw_color);
                    }
                    dist_acc += dist;
                }
            }
        }

        // 2. Junction meshes
        // Optimize by pre-building an adjacency list
        let mut node_to_edges = vec![Vec::new(); graph.nodes.len()];
        for (e_idx, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road { continue; }
            node_to_edges[edge.start_node as usize].push((e_idx, edge));
            node_to_edges[edge.end_node as usize].push((e_idx, edge));
        }

        for (n_idx, node) in graph.nodes.iter().enumerate() {
            let edges_at_node = &node_to_edges[n_idx];
            if edges_at_node.len() < 2 { continue; }

            // Collect boundary vertices from all connected roads
            let mut boundary_pts = Vec::new();

            for &(e_idx, edge) in edges_at_node {
                if edge.physical_geometry.len() < 2 { continue; }
                
                let is_start = edge.start_node == n_idx as u32;
                let (p_idx, next_idx) = if is_start { (0, 1) } else { (edge.physical_geometry.len() - 1, edge.physical_geometry.len() - 2) };
                
                let p = edge.physical_geometry[p_idx];
                let p_next = edge.physical_geometry[next_idx];
                let tangent = (p - p_next).normalized();
                let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);
                let hw = edge.width * 0.5;

                // Push both corners
                boundary_pts.push(p + side_dir * hw);
                boundary_pts.push(p - side_dir * hw);
            }

            if boundary_pts.len() < 3 { continue; }

            // Sort points angularly
            let center = node.pos;
            boundary_pts.sort_by(|a, b| {
                let da = *a - center;
                let db = *b - center;
                let angle_a = f32::atan2(da.z, da.x);
                let angle_b = f32::atan2(db.z, db.x);
                angle_a.partial_cmp(&angle_b).unwrap()
            });

            // Create triangle fan
            for i in 0..boundary_pts.len() {
                let v1 = boundary_pts[i];
                let v2 = boundary_pts[(i + 1) % boundary_pts.len()];

                vertices.push(center);
                vertices.push(v1);
                vertices.push(v2);

                for _ in 0..3 {
                    normals.push(Vector3::UP);
                    colors.push(Color::from_rgba(1.0, 1.0, 1.0, 1.0));
                }

                // Planar top-down UVs for junctions (shader uses world_pos anyway, but keep for consistency)
                uvs.push(Vector2::new(center.x, center.z));
                uvs.push(Vector2::new(v1.x, v1.z));
                uvs.push(Vector2::new(v2.x, v2.z));
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
