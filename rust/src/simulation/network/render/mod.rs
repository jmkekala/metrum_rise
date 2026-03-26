pub mod road;
pub mod test_road_mesh;
use godot::prelude::*;
use super::graph::RegionGraph;

pub struct NetworkMeshData {
    pub vertices: Vec<Vector3>,
    pub normals: Vec<Vector3>,
    pub uvs: Vec<Vector2>,
    pub colors: Vec<Color>,
    pub marking_vertices: Vec<Vector3>,
    pub marking_normals: Vec<Vector3>,
    pub marking_uvs: Vec<Vector2>,
    pub marking_colors: Vec<Color>,
    pub concrete_vertices: Vec<Vector3>,
    pub concrete_normals: Vec<Vector3>,
    pub concrete_uvs: Vec<Vector2>,
    pub concrete_colors: Vec<Color>,
}

impl NetworkMeshData {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(), normals: Vec::new(), uvs: Vec::new(), colors: Vec::new(),
            marking_vertices: Vec::new(), marking_normals: Vec::new(), marking_uvs: Vec::new(), marking_colors: Vec::new(),
            concrete_vertices: Vec::new(), concrete_normals: Vec::new(), concrete_uvs: Vec::new(), concrete_colors: Vec::new(),
        }
    }
}

pub trait TransitRenderer {
    fn generate_mesh_data(&self, graph: &RegionGraph, terrain: &crate::simulation::terrain::TerrainSystem) -> NetworkMeshData;
}
