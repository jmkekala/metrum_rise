//! Compiled roadbed renderer coordinator.

use crate::config::ROAD_DECAL_RENDER_Z_BIAS_M;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{
    RoadSurfaceCompileReason, RoadSurfaceSystem, SurfaceChunkKey,
};
use godot::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use super::{NetworkMeshData, TransitRenderer};

/// Intersection crosswalk markings.
pub mod crosswalks;
/// Compiled roadbed top-surface, structural concrete, and lane-marking rendering.
pub mod standard_surface;

pub(super) const MIN_SEGMENT_LEN: f32 = 0.01;
pub(super) const MARKING_RENDER_Z_BIAS_M: f32 = ROAD_DECAL_RENDER_Z_BIAS_M;
pub(super) const MARKING_WIDTH: f32 = 0.16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MeshLayer {
    Earthwork,
    Curb,
    RaisedStep,
    Sidewalk,
    Road,
    Marking,
    Concrete,
}

/// Top-surface road renderer built entirely from compiled roadbed ownership.
pub struct RoadRenderer;

impl RoadRenderer {
    /// Generates road mesh data using the provided compiled road-surface cache.
    pub fn generate_mesh_data_with_surface(
        &self,
        graph: &RegionGraph,
        lane_system: &crate::simulation::network::lanes::LaneSystem,
        terrain: &crate::simulation::terrain::TerrainSystem,
        road_surface: &RoadSurfaceSystem,
    ) -> NetworkMeshData {
        let compiled_surface =
            standard_surface::build_compiled_surface_coverage(graph, road_surface, terrain);
        let mut mesh = NetworkMeshData::new();

        crosswalks::emit_crosswalk_markings(
            &mut mesh,
            graph,
            lane_system,
            terrain,
            road_surface,
            &compiled_surface,
        );
        standard_surface::emit_compiled_surface_mesh(
            &mut mesh,
            graph,
            road_surface,
            terrain,
            &compiled_surface,
        );
        standard_surface::emit_compiled_lane_markings(
            &mut mesh,
            graph,
            lane_system,
            road_surface,
            terrain,
            &compiled_surface,
        );

        mesh
    }

    /// Generates complete replacement meshes for a bounded set of render chunks.
    pub(crate) fn generate_mesh_chunks_with_surface(
        &self,
        graph: &RegionGraph,
        lane_system: &mut crate::simulation::network::lanes::LaneSystem,
        terrain: &crate::simulation::terrain::TerrainSystem,
        road_surface: &RoadSurfaceSystem,
        target_chunks: &BTreeSet<SurfaceChunkKey>,
    ) -> BTreeMap<SurfaceChunkKey, NetworkMeshData> {
        let compiled_surface = standard_surface::build_compiled_surface_coverage_for_chunks(
            graph,
            road_surface,
            terrain,
            target_chunks,
        );
        lane_system.sync_heights_to_visible_surface_for_owners(
            graph,
            terrain,
            road_surface,
            &compiled_surface.edge_indices,
            &compiled_surface.node_ids,
        );

        let mut mesh = NetworkMeshData::new_chunk_partitioned(
            road_surface.chunk_span_m(),
            target_chunks.clone(),
        );
        crosswalks::emit_crosswalk_markings(
            &mut mesh,
            graph,
            lane_system,
            terrain,
            road_surface,
            &compiled_surface,
        );
        standard_surface::emit_compiled_surface_mesh(
            &mut mesh,
            graph,
            road_surface,
            terrain,
            &compiled_surface,
        );
        standard_surface::emit_compiled_lane_markings(
            &mut mesh,
            graph,
            lane_system,
            road_surface,
            terrain,
            &compiled_surface,
        );
        mesh.into_partitioned_chunks()
    }
}

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(
        &self,
        graph: &RegionGraph,
        lane_system: &crate::simulation::network::lanes::LaneSystem,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        let mut temporary_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);
        temporary_surface.compile_dirty_with_reason(
            graph,
            terrain,
            RoadSurfaceCompileReason::MeshPrecompute,
        );
        self.generate_mesh_data_with_surface(graph, lane_system, terrain, &temporary_surface)
    }
}

pub(super) fn push_quad(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 4],
    uvs: [Vector2; 4],
    color: Color,
) {
    push_triangle(
        mesh,
        layer,
        [vertices[0], vertices[1], vertices[2]],
        [uvs[0], uvs[1], uvs[2]],
        color,
    );
    push_triangle(
        mesh,
        layer,
        [vertices[0], vertices[2], vertices[3]],
        [uvs[0], uvs[2], uvs[3]],
        color,
    );
}

pub(super) fn push_triangle(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    mut vertices: [Vector3; 3],
    mut uvs: [Vector2; 3],
    color: Color,
) {
    let projected_winding = (vertices[1].x - vertices[0].x) * (vertices[2].z - vertices[0].z)
        - (vertices[1].z - vertices[0].z) * (vertices[2].x - vertices[0].x);
    if projected_winding < 0.0 {
        vertices.swap(1, 2);
        uvs.swap(1, 2);
    }

    push_triangle_preserving_winding(mesh, layer, vertices, uvs, color);
}

pub(super) fn push_triangle_with_normal(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    mut vertices: [Vector3; 3],
    mut uvs: [Vector2; 3],
    color: Color,
    normal: Vector3,
) {
    let projected_winding = (vertices[1].x - vertices[0].x) * (vertices[2].z - vertices[0].z)
        - (vertices[1].z - vertices[0].z) * (vertices[2].x - vertices[0].x);
    if projected_winding < 0.0 {
        vertices.swap(1, 2);
        uvs.swap(1, 2);
    }

    push_triangle_preserving_winding_with_normal(mesh, layer, vertices, uvs, color, normal);
}

pub(super) fn push_triangle_preserving_winding(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
) {
    let mut normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
    if normal.length_squared() <= 1e-8 {
        normal = Vector3::UP;
    } else {
        if normal.y < 0.0 {
            normal = -normal;
        }
        normal = normal.normalized();
    }

    push_triangle_to_layer(mesh, layer, vertices, uvs, color, normal);
}

pub(super) fn push_triangle_preserving_winding_with_normal(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
    mut normal: Vector3,
) {
    if normal.length_squared() <= 1e-8 {
        normal = Vector3::UP;
    } else {
        if normal.y < 0.0 {
            normal = -normal;
        }
        normal = normal.normalized();
    }

    push_triangle_to_layer(mesh, layer, vertices, uvs, color, normal);
}

pub(super) fn push_triangle_preserving_winding_with_exact_normal(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
    mut normal: Vector3,
) {
    if normal.length_squared() <= 1e-8 {
        normal = Vector3::UP;
    } else {
        normal = normal.normalized();
    }

    push_triangle_to_layer(mesh, layer, vertices, uvs, color, normal);
}

fn push_triangle_to_layer(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
    normal: Vector3,
) {
    if let Some(partition) = mesh.chunk_partition.as_mut() {
        let centroid_x =
            (f64::from(vertices[0].x) + f64::from(vertices[1].x) + f64::from(vertices[2].x)) / 3.0;
        let centroid_z =
            (f64::from(vertices[0].z) + f64::from(vertices[1].z) + f64::from(vertices[2].z)) / 3.0;
        let chunk_span_m = f64::from(partition.chunk_span_m);
        let chunk = (
            (centroid_x / chunk_span_m).floor() as i32,
            (centroid_z / chunk_span_m).floor() as i32,
        );
        if !partition.target_chunks.contains(&chunk) {
            return;
        }
        let origin_x = chunk.0 as f32 * partition.chunk_span_m;
        let origin_z = chunk.1 as f32 * partition.chunk_span_m;
        let local_vertices =
            vertices.map(|vertex| Vector3::new(vertex.x - origin_x, vertex.y, vertex.z - origin_z));
        let chunk_mesh = partition
            .chunks
            .entry(chunk)
            .or_insert_with(NetworkMeshData::new);
        push_triangle_to_layer(chunk_mesh, layer, local_vertices, uvs, color, normal);
        return;
    }

    let target = match layer {
        MeshLayer::Earthwork => (
            &mut mesh.earthwork_vertices,
            &mut mesh.earthwork_normals,
            &mut mesh.earthwork_uvs,
            &mut mesh.earthwork_colors,
        ),
        MeshLayer::Curb => (
            &mut mesh.curb_vertices,
            &mut mesh.curb_normals,
            &mut mesh.curb_uvs,
            &mut mesh.curb_colors,
        ),
        MeshLayer::RaisedStep => (
            &mut mesh.raised_step_vertices,
            &mut mesh.raised_step_normals,
            &mut mesh.raised_step_uvs,
            &mut mesh.raised_step_colors,
        ),
        MeshLayer::Sidewalk => (
            &mut mesh.sidewalk_vertices,
            &mut mesh.sidewalk_normals,
            &mut mesh.sidewalk_uvs,
            &mut mesh.sidewalk_colors,
        ),
        MeshLayer::Road => (
            &mut mesh.road_vertices,
            &mut mesh.road_normals,
            &mut mesh.road_uvs,
            &mut mesh.road_colors,
        ),
        MeshLayer::Marking => (
            &mut mesh.marking_vertices,
            &mut mesh.marking_normals,
            &mut mesh.marking_uvs,
            &mut mesh.marking_colors,
        ),
        MeshLayer::Concrete => (
            &mut mesh.concrete_vertices,
            &mut mesh.concrete_normals,
            &mut mesh.concrete_uvs,
            &mut mesh.concrete_colors,
        ),
    };
    for index in 0..3 {
        target.0.push(vertices[index]);
        target.1.push(normal);
        target.2.push(uvs[index]);
        target.3.push(color);
    }
}

pub(super) fn earthwork_color() -> Color {
    Color::from_rgba(0.48, 0.41, 0.30, 1.0)
}

pub(super) fn road_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 0.0)
}

pub(super) fn curb_color() -> Color {
    Color::from_rgba(0.56, 0.55, 0.53, 1.0)
}

pub(super) fn sidewalk_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 1.0)
}

pub(super) fn concrete_color() -> Color {
    Color::from_rgba(0.75, 0.75, 0.75, 1.0)
}

pub(super) fn marking_center_color() -> Color {
    Color::from_rgba(1.0, 0.86, 0.12, 1.0)
}

pub(super) fn marking_dash_color() -> Color {
    Color::from_rgba(1.0, 1.0, 1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{marking_center_color, marking_dash_color};

    #[test]
    fn lane_marking_colors_are_visible() {
        assert!(marking_center_color().a > 0.0);
        assert!(marking_dash_color().a > 0.0);
    }
}
