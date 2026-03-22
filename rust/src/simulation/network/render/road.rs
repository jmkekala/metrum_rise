use godot::prelude::*;
use crate::config;
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

        // 0. Connection mapping and Miter calculation
        let mut connection_counts = HashMap::new();
        let mut node_dirs: HashMap<u32, Vec<(usize, Vector2)>> = HashMap::new();
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            if edge.primary_type != TransitType::Road && edge.primary_type != TransitType::Foot { continue; }
            *connection_counts.entry(edge.start_node).or_insert(0) += 1;
            *connection_counts.entry(edge.end_node).or_insert(0) += 1;
            
            if edge.physical_geometry.len() >= 2 {
                let start_pos = graph.nodes[edge.start_node as usize].pos;
                let end_pos = graph.nodes[edge.end_node as usize].pos;

                let d3_s = edge.physical_geometry[1] - start_pos;
                node_dirs.entry(edge.start_node).or_default().push((edge_id, Vector2::new(d3_s.x, d3_s.z).normalized()));

                let lc = edge.physical_geometry.len();
                let d3_e = edge.physical_geometry[lc-2] - end_pos;
                node_dirs.entry(edge.end_node).or_default().push((edge_id, Vector2::new(d3_e.x, d3_e.z).normalized()));
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
                let mut miter = (s1 - s2).normalized(); // One points "in", one points "out" relative to the angle?
                // Wait, if d1 and d2 both point AWAY from node, then s1 and s2 follow CCW.
                // For a straight road, d1 = -d2. s1 = -s2. s1 - s2 = 2*s1. Normalized = s1. Correct.
                miter = (s1 - s2).normalized();
                
                // Scale miter length: len = 1.0 / cos(half_angle)
                // dot(s1, miter) is cos(half_angle)
                let cos_half = s1.dot(miter).abs();
                if cos_half > 0.1 {
                    node_miters.insert(*node_id, miter / cos_half);
                }
            }
        }

        // 1. Generate Schematic Lane Ribbons
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            let resampled_count = edge.physical_geometry.len();
            if resampled_count < 2 { continue; }

            let h_offset = config::ROAD_H_OFFSET + (edge_id % 100) as f32 * 0.001;
            let sw_w = config::SIDEWALK_WIDTH;
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
                            let tangent = (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized();
                            point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                        }
                    } else if i == resampled_count - 1 {
                        if let Some(miter) = node_miters.get(&edge.end_node) {
                            // Note: We need to flip the miter if it's the end node?
                            // Actually, d1 and d2 both point AWAY.
                            // If d_end is the incoming direction, then d_away = -d_end.
                            // In my node_miters, both d1 and d2 are AWAY.
                            // So if this road is d1, we use miter as is.
                            // But wait! Which side is which?
                            // For start node, d points AWAY (into edge). Side is perp(d).
                            // For end node, d also points AWAY (out of edge). 
                            // So we need to flip the side direction for the end node?
                            // Let's check: edge goes from P0 to P1. Tangent T = P1 - P0.
                            // At start (P0): d = T. side = perp(T).
                            // At end (P1): d = -T. side = perp(-T) = -perp(T).
                            // Yes, the end node miter needs to be flipped to match the edge's winding!
                            point_side_dirs.push(Vector3::new(-miter.x, 0.0, -miter.y));
                        } else {
                            let tangent = (edge.physical_geometry[i] - edge.physical_geometry[i-1]).normalized();
                            point_side_dirs.push(Vector3::new(-tangent.z, 0.0, tangent.x));
                        }
                    } else {
                        let tangent = (edge.physical_geometry[i+1] - edge.physical_geometry[i-1]).normalized();
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

            if edge.primary_type != TransitType::Road { continue; }

            let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;
            let lane_w = edge.width / total_lanes;
            let lane_count_f = total_lanes;

            // Pre-calculate side directions for miter joins
            let mut point_side_dirs = Vec::with_capacity(resampled_count);
            for i in 0..resampled_count {
                let tangent = if i == 0 {
                    (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized()
                } else if i == resampled_count - 1 {
                    (edge.physical_geometry[i] - edge.physical_geometry[i-1]).normalized()
                } else {
                    (edge.physical_geometry[i+1] - edge.physical_geometry[i-1]).normalized()
                };
                let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);
                point_side_dirs.push(side_dir);
            }

            // Lane Ribbons
            let lane_count = (edge.fwd_lanes + edge.bkw_lanes) as usize;
            for l_idx in 0..lane_count {
                let lateral_offset = (total_lanes * 0.5 - l_idx as f32 - 0.5) * lane_w;
                
                let mut dist_acc = 0.0;
                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i+1];

                    let dist = (p1 - p0).length();
                    if dist < 0.01 { continue; }

                    p0.y += h_offset;
                    p1.y += h_offset;

                    let v0_l = p0 + side0 * (lateral_offset - lane_w * 0.5);
                    let v0_r = p0 + side0 * (lateral_offset + lane_w * 0.5);
                    let v1_l = p1 + side1 * (lateral_offset - lane_w * 0.5);
                    let v1_r = p1 + side1 * (lateral_offset + lane_w * 0.5);

                    let uv_y0_l = 0.0;
                    let uv_y0_r = 1.0;
                    
                    vertices.push(v0_l); vertices.push(v1_l); vertices.push(v1_r);
                    vertices.push(v0_l); vertices.push(v1_r); vertices.push(v0_r);
                    
                    uvs.push(Vector2::new(dist_acc, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_r));
                    
                    uvs.push(Vector2::new(dist_acc, uv_y0_l));
                    uvs.push(Vector2::new(dist_acc + dist, uv_y0_r));
                    uvs.push(Vector2::new(dist_acc, uv_y0_r));

                    // Color: R=WorldMask, G=IsLaneBoundary, B=IsCenterBoundary, A=JunctionMask
                    let is_lane_boundary = if l_idx > 0 { 1.0 } else { 0.0 };
                    let is_center_boundary = if l_idx == edge.fwd_lanes as usize && edge.fwd_lanes > 0 && edge.bkw_lanes > 0 { 1.0 } else { 0.0 };

                    for _ in 0..6 {
                        normals.push(Vector3::UP);
                        colors.push(Color::from_rgba(1.0, is_lane_boundary, is_center_boundary, 0.0));
                    }
                    dist_acc += dist;
                }
            }

            // Sidewalk Ribbons (Grey)
            let sw_color = Color::from_rgb(1.0, 1.0, 1.0); // Use white for PBR asphalt look too? No, grey is fine.
            let sw_w = 0.5;
            let sw_offsets = [edge.width * 0.5 + sw_w * 0.5, -(edge.width * 0.5 + sw_w * 0.5)];
            
            for &lateral_offset in &sw_offsets {
                let mut dist_acc = 0.0;
                for i in 0..resampled_count - 1 {
                    let mut p0 = edge.physical_geometry[i];
                    let mut p1 = edge.physical_geometry[i + 1];
                    let side0 = point_side_dirs[i];
                    let side1 = point_side_dirs[i+1];

                    let dist = (p1 - p0).length();
                    if dist < 0.01 { continue; }

                    p0.y += h_offset + 0.001;
                    p1.y += h_offset + 0.001;

                    let v0_l = p0 + side0 * (lateral_offset - sw_w * 0.5);
                    let v0_r = p0 + side0 * (lateral_offset + sw_w * 0.5);
                    let v1_l = p1 + side1 * (lateral_offset - sw_w * 0.5);
                    let v1_r = p1 + side1 * (lateral_offset + sw_w * 0.5);

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

            // Collect boundary vertices from all connected roads, tagged by edge ID
            let mut boundary_pts = Vec::new();

            for &(e_idx, edge) in edges_at_node {
                if edge.physical_geometry.len() < 2 { continue; }
                
                let is_start = edge.start_node == n_idx as u32;
                let (p_idx, next_idx) = if is_start { (0, 1) } else { (edge.physical_geometry.len() - 1, edge.physical_geometry.len() - 2) };
                
                let mut p = edge.physical_geometry[p_idx];
                p.y += config::ROAD_H_OFFSET;
                let p_next = edge.physical_geometry[next_idx];
                let tangent = (p - p_next).normalized();
                let side_dir = Vector3::new(-tangent.z, 0.0, tangent.x);
                let hw = edge.width * 0.5 + config::SIDEWALK_WIDTH; // Include sidewalk width
                
                // Push both corners with the same edge index
                boundary_pts.push((e_idx, p + side_dir * hw));
                boundary_pts.push((e_idx, p - side_dir * hw));
            }

            if boundary_pts.len() < 3 { continue; }

            // Sort points angularly
            let mut center = node.pos;
            center.y += config::ROAD_H_OFFSET;
            boundary_pts.sort_by(|a, b| {
                let da = a.1 - center;
                let db = b.1 - center;
                let angle_a = f32::atan2(da.z, da.x);
                let angle_b = f32::atan2(db.z, db.x);
                angle_a.partial_cmp(&angle_b).unwrap()
            });

            // Create triangle fan with road-aware arc interpolation
            for i in 0..boundary_pts.len() {
                let (e1, v1) = boundary_pts[i];
                let (e2, v2) = boundary_pts[(i + 1) % boundary_pts.len()];

                let da = v1 - center;
                let db = v2 - center;
                let angle_a = f32::atan2(da.z, da.x);
                let mut angle_b = f32::atan2(db.z, db.x);

                // Handle wrapping
                if angle_b < angle_a {
                    angle_b += 2.0 * std::f32::consts::PI;
                }

                let delta = angle_b - angle_a;
                
                // Only interpolate if the points belong to DIFFERENT roads.
                // Reverted from arc interpolation to straight fan to avoid "bulges".
                let subdivisions = 1;
                
                let r1 = da.length();
                let r2 = db.length();

                for s in 0..subdivisions {
                    let t0 = s as f32 / subdivisions as f32;
                    let t1 = (s + 1) as f32 / subdivisions as f32;

                    let ang0 = angle_a + delta * t0;
                    let ang1 = angle_a + delta * t1;
                    let rad0 = r1 + (r2 - r1) * t0;
                    let rad1 = r1 + (r2 - r1) * t1;

                    let p0 = center + Vector3::new(ang0.cos() * rad0, 0.0, ang0.sin() * rad0);
                    let p1 = center + Vector3::new(ang1.cos() * rad1, 0.0, ang1.sin() * rad1);

                    vertices.push(center);
                    vertices.push(p0);
                    vertices.push(p1);

                    for _ in 0..3 {
                        normals.push(Vector3::UP);
                        colors.push(Color::from_rgba(1.0, 1.0, 1.0, 1.0));
                    }

                    // Planar top-down UVs for junctions
                    uvs.push(Vector2::new(center.x, center.z));
                    uvs.push(Vector2::new(p0.x, p0.z));
                    uvs.push(Vector2::new(p1.x, p1.z));
                }
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
