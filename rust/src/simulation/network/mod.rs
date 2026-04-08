//! Road network: graph data, topology operations, rendering, and pathfinding integration.
//!
//! The public entry point for road edits is [`TransitNetwork`], which manages the
//! pre-computed routing hierarchy and coordinates updates to the [`graph::RegionGraph`].
//! All structural modifications (add, split, merge, remove) go through `TransitNetwork` methods
//! so that the CCH graph is rebuilt atomically after each change.
//!
//! **Never modify [`graph::RegionGraph`] directly from outside this module.**

use godot::prelude::*;
pub mod graph;
pub mod render;
pub mod types;
pub use render::NetworkMeshData;
pub mod interaction;
/// Physical lane geometry and connectivity system.
pub mod lanes;
pub mod terrain;
pub mod topology;
use crate::config;
use std::collections::HashSet;

use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::pathing::flow_field::FlowFieldSystem;
use graph::*;
use render::TransitRenderer;
use render::road::RoadRenderer;
use types::*;

/// Top-level road network manager for pathfinding integration and coordinate conversion.
///
/// Use this struct for all road edits. It ensures the CCH graph is rebuilt after structural changes.
pub struct TransitNetwork {
    /// The Customizable Contraction Hierarchy (CCH) graph for global routing.
    pub cch_graph: CchGraph,
    /// Chunks (512m) that need topology recalculation. If not empty, triggers CCH Phase 1 rebuild.
    pub cch_dirty_chunks: HashSet<(i32, i32)>,
    /// Flags that edge costs have changed, requiring CCH Phase 2 (Metric Customization).
    pub metric_dirty: bool,
    /// Pre-calculated lane geometry graph for fast agent traversal.
    pub lane_system: lanes::LaneSystem,
    /// When true, `create_edge_internal` skips `lane_system.rebuild` and
    /// `rebuild_intersection_clips` on every edge add. The caller must call
    /// `finalize_bulk_load` once all edges have been added.
    pub bulk_load: bool,
    /// Edge IDs that were added or modified during the current bulk-load sequence.
    /// Drained by `finalize_bulk_load` to drive `rebuild_edges_incremental`.
    pub bulk_dirty_edges: HashSet<usize>,
    /// Per-zone-type flow fields for O(1) agent routing. Rebuilt lazily when dirty.
    pub flow_fields: FlowFieldSystem,
}

impl TransitNetwork {
    /// Creates a new, empty transit network.
    pub fn new() -> Self {
        Self {
            cch_graph: CchGraph::new(0),
            cch_dirty_chunks: HashSet::new(),
            metric_dirty: false,
            lane_system: lanes::LaneSystem::new(),
            bulk_load: false,
            bulk_dirty_edges: HashSet::new(),
            flow_fields: FlowFieldSystem::new(),
        }
    }

    /// Completes a bulk-load sequence started by setting `bulk_load = true`.
    ///
    /// Rebuilds intersection clips incrementally for the dirty edge set, then updates
    /// the distance-to-road grid. Returns the set of edges that were rebuilt so the caller
    /// can pass it to [`AgentSystem::invalidate_lane_ids_for_edges`].
    ///
    /// **Callers that need agent invalidation** should call
    /// `agent_system.invalidate_lane_ids_for_edges(&dirty, &self.lane_system)` **before**
    /// calling `lane_system.rebuild_edges_incremental`, i.e. they should inline the steps
    /// rather than using this helper. This method is provided for contexts where agent
    /// invalidation is not required (e.g. save-load restore).
    pub fn finalize_bulk_load(
        &mut self,
        graph: &mut RegionGraph,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) -> HashSet<usize> {
        self.bulk_load = false;
        graph.rebuild_intersection_clips();
        let dirty = std::mem::take(&mut self.bulk_dirty_edges);
        self.lane_system.rebuild_edges_incremental(graph, &dirty);
        allocator.rebuild_entrance_cache(graph, &self.lane_system);
        zoning.update_distance_to_road(graph);
        dirty
    }

    /// Clears the entire network, including zoning and building allocations.
    pub fn clear(
        &mut self,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        self.cch_graph = CchGraph::new(0);
        self.cch_dirty_chunks.clear();
        self.metric_dirty = false;
        self.lane_system.clear();
        zoning.clear();
        allocator.clear();
    }

    /// Adds a new road segment to the network, handling snapping, smoothing, and subdivision.
    pub fn add_road(
        &mut self,
        graph: &mut RegionGraph,
        points: Vec<Vector3>,
        fwd_lanes: u8,
        bkw_lanes: u8,
        class: EdgeClass,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        // 1. Simplify points
        let mut simplified_points = Vec::with_capacity(points.len());
        if !points.is_empty() {
            simplified_points.push(points[0]);
            for i in 1..points.len() {
                if points[i].distance_to(*simplified_points.last().unwrap()) > 0.5 {
                    simplified_points.push(points[i]);
                }
            }
            if simplified_points.len() > 1
                && simplified_points.last().unwrap() != &points[points.len() - 1]
            {
                simplified_points.pop();
                simplified_points.push(points[points.len() - 1]);
            }
        }

        let count = simplified_points.len();
        if count < 2 {
            return;
        }

        // Robust Snapping
        let start_id = graph.find_or_add_node(
            simplified_points[0],
            config::SNAP_TOLERANCE,
            NodeType::Junction,
        );
        let end_id = graph.find_or_add_node(
            simplified_points[count - 1],
            config::SNAP_TOLERANCE,
            NodeType::Junction,
        );

        // Snap geometry to nodes
        simplified_points[0] = graph.node(start_id).pos;
        simplified_points[count - 1] = graph.node(end_id).pos;

        // TAUBIN SMOOTHING (Volume-Preserving Harmonic Conformance)
        // Irons out any kinks caused by Start/End node snapping without shrinking the spline.
        let iters = 50;
        if count > 2 {
            let mut temp_h = vec![0.0; count];
            let lambda = 0.5;
            let mu = -0.53;
            for _ in 0..iters {
                // Positive Pass (Shrink/Smooth)
                for j in 1..count - 1 {
                    let laplacian = 0.5 * (simplified_points[j - 1].y + simplified_points[j + 1].y)
                        - simplified_points[j].y;
                    temp_h[j] = simplified_points[j].y + lambda * laplacian;
                }
                for j in 1..count - 1 {
                    simplified_points[j].y = temp_h[j];
                }
                // Negative Pass (Inflate/Restore Volume)
                for j in 1..count - 1 {
                    let laplacian = 0.5 * (simplified_points[j - 1].y + simplified_points[j + 1].y)
                        - simplified_points[j].y;
                    temp_h[j] = simplified_points[j].y + mu * laplacian;
                }
                for j in 1..count - 1 {
                    simplified_points[j].y = temp_h[j];
                }
            }
        }

        // 3. Create a single edge from start to end with the full simplified geometry.
        {
            let last_idx = simplified_points.len() - 1;
            simplified_points[last_idx] = graph.node(end_id).pos;
            self.create_edge_internal(
                graph,
                start_id,
                end_id,
                simplified_points,
                fwd_lanes,
                bkw_lanes,
                class,
                zoning,
                allocator,
            );
        }

        if !self.bulk_load {
            zoning.update_distance_to_road(graph);
        }
    }

    /// Helper to consistently add a road edge and handle its side effects
    fn create_edge_internal(
        &mut self,
        graph: &mut RegionGraph,
        start: u32,
        end: u32,
        points: Vec<Vector3>,
        fwd: u8,
        bkw: u8,
        class: EdgeClass,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        if start == end {
            return;
        }

        // Final sanity check on points
        if points.len() < 2 {
            return;
        }

        let is_walkway = fwd == 0 && bkw == 0;
        let mut allowed_types = TransitFlags::NONE;
        if fwd > 0 || bkw > 0 {
            allowed_types |= TransitFlags::CAR;
        }
        // Pedestrians (and therefore sidewalks) on walkways and all standard roads.
        if is_walkway || fwd > 0 || bkw > 0 {
            allowed_types |= TransitFlags::FOOT;
        }
        let vehicle_frontage_access = if is_walkway {
            VehicleFrontageAccess::SameSideOnly
        } else {
            VehicleFrontageAccess::BothSides
        };

        let edge_id = graph.add_edge(graph::Edge {
            start_node: start,
            end_node: end,
            primary_type: if is_walkway {
                TransitType::Foot
            } else {
                TransitType::Road
            },
            allowed_types,
            width: ((fwd + bkw) as f32 * config::LANE_WIDTH).max(2.0),
            class,
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
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access,
        });

        let (cost, length) =
            crate::simulation::pathing::cost::CostCalculator::calculate_costs(graph.edge(edge_id));
        graph.edges[edge_id].base_cost = cost;
        graph.edges[edge_id].physical_length = length;
        // Auto-flag high-speed roads: speed_limit is stored in km/h.
        if graph.edges[edge_id].speed_limit >= 80.0 {
            graph.edges[edge_id].no_building_spawn = true;
        }

        // Distance-to-road update is deferred to after lane rebuild in non-bulk mode
        // (called below after `process_intersections` and clip rebuild).
        self.mark_point_dirty(graph.node(start).pos);
        self.mark_point_dirty(graph.node(end).pos);

        if self.bulk_load {
            self.bulk_dirty_edges.insert(edge_id);
        }

        let node_count_before = graph.node_count();
        let edges_before = graph.edge_count();
        topology::process_intersections(self, graph, edge_id, zoning, allocator);
        self.cleanup_duplicate_edges(graph); // Clean edge_id if it's dup

        if !self.bulk_load {
            // Incremental clip rebuild: only resample edges incident to nodes touched by this
            // road placement instead of doing the full O(E) resample + R-tree rebuild.
            let mut affected_nodes: HashSet<u32> = HashSet::new();
            let mut affected_edges: HashSet<usize> = HashSet::new();
            if !graph.edge(edge_id).deleted {
                affected_nodes.insert(graph.get_valid_node(graph.edge(edge_id).start_node));
                affected_nodes.insert(graph.get_valid_node(graph.edge(edge_id).end_node));
                affected_edges.insert(edge_id);
            }
            // Junction nodes and split-half edges created during intersection processing.
            for new_nid in node_count_before as u32..graph.node_count() as u32 {
                affected_nodes.insert(graph.get_valid_node(new_nid));
            }
            for new_eid in edges_before..graph.edge_count() {
                if !graph.edge(new_eid).deleted {
                    affected_nodes.insert(graph.get_valid_node(graph.edge(new_eid).start_node));
                    affected_nodes.insert(graph.get_valid_node(graph.edge(new_eid).end_node));
                    affected_edges.insert(new_eid);
                }
            }
            graph.rebuild_intersection_clips_for_nodes(&affected_nodes);
            self.lane_system
                .rebuild_edges_incremental(graph, &affected_edges);
        }

        // Mark chunks as dirty
        let chunks = graph.get_edge_chunks(edge_id);
        self.cch_dirty_chunks.extend(chunks);
    }

    /// Generates visual mesh data for the entire road network.
    pub fn generate_mesh_data(
        &self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        let renderer = RoadRenderer;
        renderer.generate_mesh_data(graph, &self.lane_system, terrain)
    }

    /// Flattens the terrain surface underneath the road network.
    pub fn flatten_terrain(
        &self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
        output_heightmap: &mut [f32],
        map_size: Vector2,
    ) {
        terrain::flatten_terrain_for_network(graph, terrain, output_heightmap, map_size);
    }

    /// Synchronizes road elevations with the terrain heightmap.
    pub fn sync_to_terrain(
        &mut self,
        graph: &mut RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) {
        graph.sync_to_terrain(terrain);
        self.lane_system.rebuild(graph);
    }

    /// Rebuilds the routing CCH hierarchy from scratch or perform incremental customization.
    /// Always marks flow fields dirty when topology changes — caller must call
    /// `flow_fields.rebuild_dirty` afterwards with the building allocator.
    pub fn rebuild_pathing(&mut self, graph: &mut RegionGraph) {
        // Topology changes (Phase 1)
        if !self.cch_dirty_chunks.is_empty() {
            self.cch_graph = CchGraph::build(graph);
            self.cch_dirty_chunks.clear();
            self.metric_dirty = false; // Phase 1 includes customize
            self.flow_fields.mark_all_dirty();
        } else if self.metric_dirty {
            // Metric-only change (Phase 2)
            self.cch_graph.customize(graph);
            self.metric_dirty = false;
        }
    }

    /// Rebuilds the routing hierarchy if it has been marked dirty.
    pub fn rebuild_pathing_if_dirty(&mut self, graph: &mut RegionGraph) {
        if !self.cch_dirty_chunks.is_empty() || self.metric_dirty {
            self.rebuild_pathing(graph);
        }
    }

    /// Marks the chunk containing this world-space point as requiring CCH rebuild.
    pub fn mark_point_dirty(&mut self, pos: Vector3) {
        let coords = RegionGraph::get_chunk_coords(pos);
        self.cch_dirty_chunks.insert(coords);
    }

    fn cleanup_duplicate_edges(&mut self, graph: &mut RegionGraph) {
        let mut seen = std::collections::HashSet::new();
        let mut to_remove = Vec::new();

        for (i, edge) in graph.edges().iter().enumerate() {
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
            graph.edges[index].deleted = true;
        }
    }
}

/// Automated tests for intersection clipping.
pub mod test_clips;
/// Automated tests for graph edge compaction.
pub mod test_compaction;
/// Automated tests for pedestrian movement through junctions.
pub mod test_ped_junction;
/// Automated tests for topology operations (add/split/merge).
pub mod test_topology;
/// Automated tests for vehicle U-turn constraints.
pub mod test_uturn;
/// Automated tests for graph verification and structural integrity.
pub mod test_verify;
