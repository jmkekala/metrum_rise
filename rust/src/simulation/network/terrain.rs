//! Terrain adjustment utilities for the road network.
//!
//! Provides functions to modify the heightmap to ensure roads, rails,
//! and paths sit correctly on the terrain without clipping.

use super::graph::RegionGraph;
use super::types::TransitType;
use godot::prelude::*;

/// Modifies the `output_heightmap` to create flat "beds" for the road network.
///
/// Uses Voronoi-style blending to resolve overlapping road segments and
/// ensures smooth transitions between the road and the surrounding terrain.
pub fn flatten_terrain_for_network(
    graph: &RegionGraph,
    terrain: &crate::simulation::terrain::TerrainSystem,
    output_heightmap: &mut [f32],
    map_size: Vector2,
) {
    let half_size = map_size * 0.5;
    let width = terrain.width;
    let height = terrain.height;
    let cell_size = terrain.cell_size_m();

    // ATOMIC BLENDING: Use the provided terrain as a stable reference.
    let reference_heightmap = terrain.clone_visual_dense();
    let mut min_dist_map = vec![f32::MAX; width * height];

    for edge in &graph.edges {
        if edge.deleted || edge.class != crate::simulation::network::types::EdgeClass::Standard {
            continue;
        }
        if edge.primary_type != TransitType::Road
            && edge.primary_type != TransitType::Rail
            && edge.primary_type != TransitType::Foot
        {
            continue;
        }

        if edge.geometry.len() < 2 {
            continue;
        }

        for i in 0..edge.geometry.len() - 1 {
            let p_start = edge.geometry[i];
            let p_end = edge.geometry[i + 1];
            let segment_vec = p_end - p_start;
            let dist_seg = segment_vec.length();
            if dist_seg < 0.1 {
                continue;
            }

            let road_half_width = edge.width * 0.5;
            let inner_radius = road_half_width + 1.0;
            let outer_radius = road_half_width * 4.0;

            // BOUNDING BOX for this segment
            let min_x =
                ((p_start.x.min(p_end.x) + half_size.x - outer_radius) / cell_size).floor() as i32;
            let max_x =
                ((p_start.x.max(p_end.x) + half_size.x + outer_radius) / cell_size).ceil() as i32;
            let min_z =
                ((p_start.z.min(p_end.z) + half_size.y - outer_radius) / cell_size).floor() as i32;
            let max_z =
                ((p_start.z.max(p_end.z) + half_size.y + outer_radius) / cell_size).ceil() as i32;

            for nz in min_z..=max_z {
                for nx in min_x..=max_x {
                    if nx < 0 || nx >= width as i32 || nz < 0 || nz >= height as i32 {
                        continue;
                    }
                    let idx = nz as usize * width + nx as usize;

                    let (world_x, world_z) = terrain.grid_to_world_coords(nx as usize, nz as usize);
                    let p_world = Vector3::new(world_x, 0.0, world_z);
                    let p0_flat = Vector3::new(p_start.x, 0.0, p_start.z);
                    let p1_flat = Vector3::new(p_end.x, 0.0, p_end.z);
                    let segment_flat_vec = p1_flat - p0_flat;
                    let dist_flat = segment_flat_vec.length();

                    if dist_flat < 0.01 {
                        continue;
                    }
                    let dir_flat = segment_flat_vec / dist_flat;

                    // Project point onto segment in 2D (xz)
                    let t = (p_world - p0_flat).dot(dir_flat).clamp(0.0, dist_flat);
                    let nearest_flat = p0_flat + dir_flat * t;
                    let dist = p_world.distance_to(nearest_flat);

                    if dist < outer_radius {
                        // VORONOI BLENDING: Only carve this pixel if this segment is the closest one
                        if dist > min_dist_map[idx] {
                            continue;
                        }
                        min_dist_map[idx] = dist;

                        // Nearest point in 3D (Interpolate height using 2D distance ratio)
                        let height_ratio = t / dist_flat;
                        let road_y = p_start.y + (p_end.y - p_start.y) * height_ratio;

                        // Carve deeply to prevent clipping (0.002 = 4cm)
                        let pavement_depth = 0.002;
                        let target_h = (road_y / 20.0) - pavement_depth;
                        let ref_h = reference_heightmap[idx];

                        if dist < inner_radius {
                            output_heightmap[idx] = target_h;
                        } else {
                            let weight = 1.0
                                - ((dist - inner_radius) / (outer_radius - inner_radius))
                                    .clamp(0.0, 1.0);
                            let smoothed_weight = weight * weight * (3.0 - 2.0 * weight);
                            let new_h = ref_h + (target_h - ref_h) * smoothed_weight;

                            if target_h < ref_h {
                                output_heightmap[idx] = output_heightmap[idx].min(new_h);
                            } else {
                                output_heightmap[idx] = output_heightmap[idx].max(new_h);
                            }
                        }
                    }
                }
            }
        }
    }
}
