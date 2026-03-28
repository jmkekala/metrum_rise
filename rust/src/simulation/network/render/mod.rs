pub mod road;
pub mod test_road_mesh;
use super::graph::RegionGraph;
use godot::prelude::*;

pub struct NetworkMeshData {
    pub sidewalk_vertices: Vec<Vector3>,
    pub sidewalk_normals: Vec<Vector3>,
    pub sidewalk_uvs: Vec<Vector2>,
    pub sidewalk_colors: Vec<Color>,
    pub road_vertices: Vec<Vector3>,
    pub road_normals: Vec<Vector3>,
    pub road_uvs: Vec<Vector2>,
    pub road_colors: Vec<Color>,
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
            sidewalk_vertices: Vec::new(),
            sidewalk_normals: Vec::new(),
            sidewalk_uvs: Vec::new(),
            sidewalk_colors: Vec::new(),
            road_vertices: Vec::new(),
            road_normals: Vec::new(),
            road_uvs: Vec::new(),
            road_colors: Vec::new(),
            marking_vertices: Vec::new(),
            marking_normals: Vec::new(),
            marking_uvs: Vec::new(),
            marking_colors: Vec::new(),
            concrete_vertices: Vec::new(),
            concrete_normals: Vec::new(),
            concrete_uvs: Vec::new(),
            concrete_colors: Vec::new(),
        }
    }
}

pub trait TransitRenderer {
    fn generate_mesh_data(
        &self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData;
}
