use godot::prelude::*;
pub mod types;
pub mod graph;
pub mod render;
pub use render::NetworkMeshData;
pub mod terrain;
pub mod interaction;
pub mod topology;
use crate::config;

use types::*;
use graph::*;
use render::TransitRenderer;
use render::road::RoadRenderer;
use crate::simulation::pathing::hpa::HpaGraph;

pub struct TransitNetwork {
    pub graph: TransitGraph,
    pub hpa_graph: HpaGraph,
}

impl TransitNetwork {
    pub fn new() -> Self {
        Self {
            graph: TransitGraph::new(),
            hpa_graph: HpaGraph::new(),
        }
    }

    pub fn add_road(&mut self, points: Vec<Vector3>, fwd_lanes: u8, bkw_lanes: u8) {
        // 1. Simplify points
        let mut simplified_points = Vec::with_capacity(points.len());
        if !points.is_empty() {
            simplified_points.push(points[0]);
            for i in 1..points.len() {
                if points[i].distance_to(*simplified_points.last().unwrap()) > 0.5 {
                    simplified_points.push(points[i]);
                }
            }
            if simplified_points.len() > 1 && simplified_points.last().unwrap() != &points[points.len()-1] {
                simplified_points.pop();
                simplified_points.push(points[points.len()-1]);
            }
        }
        
        let count = simplified_points.len();
        if count < 2 { return; }

        // Robust Snapping
        let start_id = self.graph.find_or_add_node(simplified_points[0], config::SNAP_TOLERANCE, NodeType::Junction);
        let end_id = self.graph.find_or_add_node(simplified_points[count - 1], config::SNAP_TOLERANCE, NodeType::Junction);

        // Snap geometry to nodes
        simplified_points[0] = self.graph.nodes[start_id as usize].pos;
        simplified_points[count - 1] = self.graph.nodes[end_id as usize].pos;

        // TAUBIN SMOOTHING (Volume-Preserving Harmonic Conformance)
        // Irons out any kinks caused by Start/End node snapping without shrinking the spline.
        let iters = 50;
        if count > 2 {
            let mut temp_h = vec![0.0; count];
            let lambda = 0.5;
            let mu = -0.53;
            for _ in 0..iters {
                // Positive Pass (Shrink/Smooth)
                for j in 1..count-1 {
                    let laplacian = 0.5 * (simplified_points[j-1].y + simplified_points[j+1].y) - simplified_points[j].y;
                    temp_h[j] = simplified_points[j].y + lambda * laplacian;
                }
                for j in 1..count-1 {
                    simplified_points[j].y = temp_h[j];
                }
                // Negative Pass (Inflate/Restore Volume)
                for j in 1..count-1 {
                    let laplacian = 0.5 * (simplified_points[j-1].y + simplified_points[j+1].y) - simplified_points[j].y;
                    temp_h[j] = simplified_points[j].y + mu * laplacian;
                }
                for j in 1..count-1 {
                    simplified_points[j].y = temp_h[j];
                }
            }
        }

        let edge_id = self.graph.add_edge(graph::Edge {
            start_node: start_id,
            end_node: end_id,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: ((fwd_lanes + bkw_lanes) as f32 * 3.0).max(2.0),
            fwd_lanes,
            bkw_lanes,
            speed_limit: 50.0,
            base_cost: 0.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: simplified_points.clone(),
            physical_geometry: simplified_points,
        });

        let cost = crate::simulation::pathing::cost::CostCalculator::calculate_base_cost(&self.graph.edges[edge_id]);
        self.graph.edges[edge_id].base_cost = cost;

        topology::process_intersections(self, edge_id);
        self.cleanup_duplicate_edges();
        self.graph.rebuild_intersection_clips();
        
        // Rebuild massive DoD pathing table for agents
        self.hpa_graph = HpaGraph::build(&self.graph);
    }

    pub fn generate_mesh_data(&self, terrain: &crate::simulation::terrain::TerrainSystem) -> NetworkMeshData {
        let renderer = RoadRenderer; 
        renderer.generate_mesh_data(&self.graph, terrain)
    }

    pub fn flatten_terrain(&self, terrain: &crate::simulation::terrain::TerrainSystem, output_heightmap: &mut [f32], map_size: Vector2) {
        terrain::flatten_terrain_for_network(&self.graph, terrain, output_heightmap, map_size);
    }

    pub fn sync_to_terrain(&mut self, terrain: &crate::simulation::terrain::TerrainSystem) {
        self.graph.sync_to_terrain(terrain);
    }

    fn cleanup_duplicate_edges(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut to_remove = Vec::new();

        for (i, edge) in self.graph.edges.iter().enumerate() {
            let pair = if edge.start_node < edge.end_node {
                (edge.start_node, edge.end_node)
            } else {
                (edge.end_node, edge.start_node)
            };

            if seen.contains(&pair) || edge.start_node == edge.end_node {
                to_remove.push(i);
            } else {
                seen.insert(pair);
            }
        }

        for &index in to_remove.iter().rev() {
            self.graph.edges.remove(index);
        }
    }
}

pub mod test_clips;
pub mod test_topology;
pub mod test_verify;
