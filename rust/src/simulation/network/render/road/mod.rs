//! Compiled roadbed renderer coordinator.

use crate::config::ROAD_DECAL_RENDER_Z_BIAS_M;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use godot::prelude::*;

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

        crosswalks::emit_crosswalk_markings(&mut mesh, lane_system);
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
            road_surface,
            terrain,
            &compiled_surface,
        );

        mesh
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
        temporary_surface.compile_dirty(graph, terrain);
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

    let mut normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
    if normal.length_squared() <= 1e-8 {
        normal = Vector3::UP;
    } else {
        if normal.y < 0.0 {
            normal = -normal;
        }
        normal = normal.normalized();
    }

    let target = match layer {
        MeshLayer::Earthwork => (
            &mut mesh.earthwork_vertices,
            &mut mesh.earthwork_normals,
            &mut mesh.earthwork_uvs,
            &mut mesh.earthwork_colors,
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

pub(super) fn sidewalk_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 1.0)
}

pub(super) fn concrete_color() -> Color {
    Color::from_rgba(0.75, 0.75, 0.75, 1.0)
}

pub(super) fn marking_center_color() -> Color {
    Color::from_rgba(0.0, 1.0, 1.0, 0.0)
}

pub(super) fn marking_dash_color() -> Color {
    Color::from_rgba(0.0, 1.0, 0.0, 0.0)
}
