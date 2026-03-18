pub mod road;
use godot::prelude::*;
use super::graph::TransitGraph;

pub trait TransitRenderer {
    fn generate_mesh_data(&self, graph: &TransitGraph, terrain: &crate::simulation::terrain::TerrainSystem) -> (PackedVector3Array, PackedVector3Array, PackedVector2Array, PackedColorArray);
}
