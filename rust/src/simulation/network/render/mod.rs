pub mod road;
use godot::prelude::*;
use super::graph::RegionGraph;

pub struct NetworkMeshData {
    pub vertices: PackedVector3Array,
    pub normals: PackedVector3Array,
    pub uvs: PackedVector2Array,
    pub colors: PackedColorArray,
    pub marking_vertices: PackedVector3Array,
    pub marking_normals: PackedVector3Array,
    pub marking_uvs: PackedVector2Array,
    pub marking_colors: PackedColorArray,
}

pub trait TransitRenderer {
    fn generate_mesh_data(&self, graph: &RegionGraph, terrain: &crate::simulation::terrain::TerrainSystem) -> NetworkMeshData;
}
