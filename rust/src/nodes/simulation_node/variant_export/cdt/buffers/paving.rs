// SPDX-License-Identifier: GPL-2.0-only

//! Material partition of final terrain buffers, and the site pads that fill the terrain's site
//! holes. No independent yard elevation solver.

use super::*;
use crate::assets::SiteSurfaceMaterial;
use crate::simulation::buildings::allocator::{
    BuildingSiteFoundation, BuildingSiteSurfaceClient, SitePavingPartition,
    foundation_faces_in_frame,
};
use crate::simulation::terrain::TerrainPatchSnapshot;

impl SimulationNode {
    // Partitions the terrain faces by the site surfaces and appends each local site's pad, in
    // the patch's frame (`ROAD-68-D`, open question 23): the pad's faces run through the same
    // partition as the terrain's, so a yard edge crossing a footprint side splits pad and terrain
    // at one point, and the pad's corners convert from f64 as the terrain's seam vertices do.
    pub(in crate::nodes::simulation_node) fn partition_terrain_site_paving(
        patch: &TerrainPatchSnapshot,
        surfaces: &[BuildingSiteSurfaceClient],
        foundations: &[BuildingSiteFoundation],
        buffers: &mut CachedRefinedTerrainMeshBuffers,
    ) {
        if (surfaces.is_empty() && foundations.is_empty()) || !buffers.variant_payload_valid {
            return;
        }
        // Partition in patch-local coordinates, retaining original seam precision.
        let (centre_x, centre_z) = patch.frame_centre();
        let mut local = surfaces.to_vec();
        for surface in &mut local {
            for p in &mut surface.vertices_world {
                p.x = render_frame_local(f64::from(p.x), centre_x);
                p.y = render_frame_local(f64::from(p.y), centre_z);
            }
        }
        let Some(partition) = SitePavingPartition::new(&local) else {
            buffers.variant_payload_valid = false;
            return;
        };
        // Keep untouched indexed faces shared. Expanding an entire 512 m patch to
        // triangle soup for one small yard would multiply upload/memory costs.
        let mut vertices = buffers.terrain_vertices.clone();
        let mut normals = buffers.terrain_normals.clone();
        let mut uvs = buffers.terrain_uvs.clone();
        let mut colors = vec![Color::from_rgb(1.0, 1.0, 1.0); vertices.len()];
        let mut output_indices = Vec::with_capacity(buffers.terrain_indices.len());
        let mut changed = false;
        for face in buffers.terrain_indices.as_chunks::<3>().0 {
            let indices = [face[0] as usize, face[1] as usize, face[2] as usize];
            let original = indices.map(|i| buffers.terrain_vertices[i]);
            partition.partition(original, |material, triangle| {
                if material.is_none() && triangle == original {
                    output_indices.extend_from_slice(face);
                    return;
                }
                changed = true;
                let color = material_color(material);
                for point in triangle {
                    let weights = barycentric_xz(original, point);
                    let mut normal = Vector3::ZERO;
                    let mut uv = Vector2::ZERO;
                    for corner in 0..3 {
                        normal += buffers.terrain_normals[indices[corner]] * weights[corner];
                        uv += buffers.terrain_uvs[indices[corner]] * weights[corner];
                    }
                    vertices.push(point);
                    output_indices.push((vertices.len() - 1) as i32);
                    normals.push(normal.normalized());
                    uvs.push(uv);
                    colors.push(color);
                }
            });
        }
        let rect = [
            f64::from(patch.world_origin_x),
            f64::from(patch.world_origin_z),
            f64::from(patch.world_origin_x + patch.world_size_x),
            f64::from(patch.world_origin_z + patch.world_size_z),
        ];
        let patch_uv = |point: Vector3| {
            Vector2::new(
                ((point.x + centre_x - patch.world_origin_x) / patch.world_size_x.max(0.001))
                    .clamp(0.0, 1.0),
                ((point.z + centre_z - patch.world_origin_z) / patch.world_size_z.max(0.001))
                    .clamp(0.0, 1.0),
            )
        };
        for foundation in foundations {
            let triangulated =
                foundation_faces_in_frame(foundation, rect, (centre_x, centre_z), |face| {
                    partition.partition(face, |material, triangle| {
                        // The pad is flat; an exactly degenerate piece covers nothing, as in
                        // the CDT export.
                        if (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0])
                            == Vector3::ZERO
                        {
                            return;
                        }
                        changed = true;
                        for point in triangle {
                            vertices.push(point);
                            output_indices.push((vertices.len() - 1) as i32);
                            normals.push(Vector3::UP);
                            uvs.push(patch_uv(point));
                            colors.push(material_color(material));
                        }
                    });
                });
            if triangulated.is_none() {
                buffers.variant_payload_valid = false;
                return;
            }
        }
        if !changed {
            return;
        }
        buffers.terrain_emitted_faces = output_indices.len() / 3;
        buffers.terrain_indices = output_indices;
        buffers.terrain_vertices = vertices;
        buffers.terrain_normals = normals;
        buffers.terrain_uvs = uvs;
        buffers.terrain_colors = colors;
        buffers.variant_payload_valid =
            Self::cached_refined_terrain_mesh_buffers_are_valid(buffers);
    }
}

// Terrain vertex colour of a material region: white terrain, red asphalt, green concrete.
fn material_color(material: Option<SiteSurfaceMaterial>) -> Color {
    match material {
        None => Color::from_rgb(1.0, 1.0, 1.0),
        Some(SiteSurfaceMaterial::Asphalt) => Color::from_rgb(1.0, 0.0, 0.0),
        Some(SiteSurfaceMaterial::Concrete) => Color::from_rgb(0.0, 1.0, 0.0),
    }
}

fn barycentric_xz([a, b, c]: [Vector3; 3], point: Vector3) -> [f32; 3] {
    let cross =
        |u: Vector3, v: Vector3| f64::from(u.x) * f64::from(v.z) - f64::from(u.z) * f64::from(v.x);
    let denominator = cross(b - a, c - a);
    let second = (cross(point - a, c - a) / denominator) as f32;
    let third = (cross(b - a, point - a) / denominator) as f32;
    [1.0 - second - third, second, third]
}
