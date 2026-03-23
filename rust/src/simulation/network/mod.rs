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

    pub fn clear(&mut self, zoning: &mut crate::simulation::grid::zoning::ZoningSystem, allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator) {
        self.graph = TransitGraph::new();
        self.hpa_graph = HpaGraph::new();
        zoning.clear();
        allocator.clear();
    }

    pub fn add_road(&mut self, points: Vec<Vector3>, fwd_lanes: u8, bkw_lanes: u8, zoning_left: bool, zoning_right: bool, zoning: &mut crate::simulation::grid::zoning::ZoningSystem, allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator) {
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

        // 3. SUBDIVISION LOGIC (Every 100m)
        let mut current_start_id = start_id;
        let mut active_segment = vec![simplified_points[0]];
        let mut accumulated_dist = 0.0;
        
        for i in 0..count - 1 {
            let p0 = simplified_points[i];
            let p1 = simplified_points[i+1];
            let d = p0.distance_to(p1);
            
            if accumulated_dist + d > 100.0 {
                // Determine how many splits we need in this segment
                let remaining_in_segment = 100.0 - accumulated_dist;
                let mut t = remaining_in_segment / d;
                
                while t <= 1.0 {
                    let split_pos = p0.lerp(p1, t);
                    active_segment.push(split_pos);
                    
                    // Create intermediate node
                    let mid_id = self.graph.find_or_add_node(split_pos, 0.1, NodeType::Junction);
                    
                    // Add this edge
                    self.create_edge_internal(current_start_id, mid_id, active_segment.clone(), fwd_lanes, bkw_lanes, zoning_left, zoning_right, zoning, allocator);
                    
                    // Reset for next segment
                    current_start_id = mid_id;
                    active_segment = vec![split_pos];
                    accumulated_dist = 0.0;
                    
                    // Move to next 100m increment
                    let next_dist_target = 100.0;
                    let remaining_after_split = (1.0 - t) * d;
                    if remaining_after_split > next_dist_target {
                        t += next_dist_target / d;
                    } else {
                        accumulated_dist = remaining_after_split;
                        break;
                    }
                }
                
                if accumulated_dist > 0.0 {
                   active_segment.push(p1);
                }

            } else {
                active_segment.push(p1);
                accumulated_dist += d;
            }
        }

        // Final segment to end_id
        if current_start_id != end_id && active_segment.len() >= 2 {
            // Replace last point with snapped end_id pos
            let last_idx = active_segment.len() - 1;
            active_segment[last_idx] = self.graph.nodes[end_id as usize].pos;
            self.create_edge_internal(current_start_id, end_id, active_segment, fwd_lanes, bkw_lanes, zoning_left, zoning_right, zoning, allocator);
        }

        // Rebuild massive DoD pathing table for agents
        self.hpa_graph = HpaGraph::build(&self.graph);
    }

    /// Helper to consistently add a road edge and handle its side effects
    fn create_edge_internal(&mut self, start: u32, end: u32, points: Vec<Vector3>, fwd: u8, bkw: u8, zoning_left: bool, zoning_right: bool, zoning: &mut crate::simulation::grid::zoning::ZoningSystem, allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator) {
        if start == end { return; }
        
        // Final sanity check on points
        if points.len() < 2 { return; }

        let is_walkway = fwd == 0 && bkw == 0;
        let mut allowed_types = TransitFlags::NONE;
        if fwd > 0 || bkw > 0 {
            allowed_types |= TransitFlags::CAR;
        }
        if is_walkway || fwd > 0 || bkw > 0 { // If it's a walkway, or a road, pedestrians can use it
            allowed_types |= TransitFlags::FOOT;
        }

        let edge_id = self.graph.add_edge(graph::Edge {
            start_node: start,
            end_node: end,
            primary_type: if is_walkway { TransitType::Foot } else { TransitType::Road },
            allowed_types,
            width: ((fwd + bkw) as f32 * config::LANE_WIDTH).max(2.0),
            fwd_lanes: fwd,
            bkw_lanes: bkw,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: points.clone(),
            physical_geometry: points,
            parking_occupied: 0,
            zoning_left,
            zoning_right,
            deleted: false,
        });

        let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(&self.graph.edges[edge_id]);
        self.graph.edges[edge_id].base_cost = cost;
        self.graph.edges[edge_id].physical_length = length;
        
        zoning.update_edge_grid_size(edge_id, length);

        topology::process_intersections(self, edge_id, zoning, allocator);
        self.cleanup_duplicate_edges(); // Clean edge_id if it's dup
        self.graph.rebuild_intersection_clips();
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

    pub fn split_for_frontage(&mut self, edge_idx: usize, pos: Vector3, zoning: &mut crate::simulation::grid::zoning::ZoningSystem, allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator) -> (u32, usize, usize) {
        let (node_id, new_edge_id) = self.graph.split_edge(edge_idx, pos);

        if new_edge_id == edge_idx {
            return (node_id, edge_idx, 0); // No split actually occurred
        }

        // --- MIGRATION LOGIC ---
        let cell_size = zoning.grid_cell_size;
        let length_first = self.graph.edges[edge_idx].physical_length;
        let split_x = (length_first / cell_size).floor() as usize;

        // 1. Migrate Zoning
        zoning.split_edge_grid(edge_idx, new_edge_id, split_x);

        // 2. Migrate Buildings
        for b in &mut allocator.buildings {
            if b.edge_idx == edge_idx && b.cell_x >= split_x {
                b.edge_idx = new_edge_id;
                b.cell_x -= split_x;
            }
        }

        (node_id, new_edge_id, split_x)
    }

    pub fn remove_frontage(&mut self, node_id: u32, zoning: &mut crate::simulation::grid::zoning::ZoningSystem, allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator) {
        let first_cells_long = if let Some(first_idx) = self.graph.edges.iter().position(|e| !e.deleted && (e.start_node == node_id || e.end_node == node_id)) {
            // Peek at existing zoning cells long for the first connected edge to determine migration offset
            zoning.edge_grids.get(&first_idx).map(|g| g.cells_long).unwrap_or(0)
        } else { 
            0 
        };

        if let Some((keep, delete)) = self.graph.remove_node_and_merge_edges(node_id) {
            // Offset for second edge's buildings is based on which one was kept first
            // Wait: remove_node_and_merge_edges already ensures 'keep' is the first edge (A -> node_id).
            // So 'delete' buildings need to be offset by original 'keep' length.
            let offset = first_cells_long;
            
            // 1. Migrate Zoning
            zoning.merge_edge_grids(keep, delete);

            // 2. Migrate Buildings
            for b in &mut allocator.buildings {
                if b.edge_idx == delete {
                    b.edge_idx = keep;
                    b.cell_x += offset;
                }
            }
        }
    }

    pub fn rebuild_pathing(&mut self) {
        self.hpa_graph = HpaGraph::build(&self.graph);
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

        for &index in &to_remove {
            self.graph.edges[index].deleted = true;
        }
    }
}

pub mod test_clips;
pub mod test_topology;
pub mod test_verify;
