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
pub mod surface;
pub mod types;
pub use render::NetworkMeshData;
pub mod interaction;
/// Physical lane geometry and connectivity system.
pub mod lanes;
pub mod topology;
use crate::config;
use std::collections::HashSet;

use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::pathing::flow_field::FlowFieldSystem;
use graph::*;
use render::road::RoadRenderer;
use surface::RoadSurfaceSystem;
use types::*;

pub(in crate::simulation::network) fn build_surface_edge(
    start_node: u32,
    end_node: u32,
    points: Vec<Vector3>,
    fwd_lanes: u8,
    bkw_lanes: u8,
    class: EdgeClass,
) -> graph::Edge {
    let is_walkway = fwd_lanes == 0 && bkw_lanes == 0;
    let mut allowed_types = TransitFlags::NONE;
    if fwd_lanes > 0 || bkw_lanes > 0 {
        allowed_types |= TransitFlags::CAR;
    }
    if is_walkway || fwd_lanes > 0 || bkw_lanes > 0 {
        allowed_types |= TransitFlags::FOOT;
    }
    let vehicle_frontage_access = if is_walkway {
        VehicleFrontageAccess::SameSideOnly
    } else {
        VehicleFrontageAccess::BothSides
    };
    let physical_length = points
        .windows(2)
        .map(|segment| segment[0].distance_to(segment[1]))
        .sum();

    graph::Edge {
        start_node,
        end_node,
        primary_type: if is_walkway {
            TransitType::Foot
        } else {
            TransitType::Road
        },
        allowed_types,
        class,
        width: ((fwd_lanes + bkw_lanes) as f32 * config::LANE_WIDTH).max(2.0),
        fwd_lanes,
        bkw_lanes,
        speed_limit: config::DEFAULT_URBAN_ROAD_SPEED_MS,
        base_cost: 0.0,
        physical_length,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access,
    }
}

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
    /// Fixed-cadence accumulator for the frontage delay cache used by exact access planning.
    pub frontage_delay_elapsed_s: f32,
    /// Phase 1 road-surface ownership shell for compiled roadbed data and dirty tracking.
    pub road_surface: RoadSurfaceSystem,
}

impl TransitNetwork {
    /// Creates a new, empty transit network.
    pub fn new() -> Self {
        Self::new_with_surface_chunk_span(RegionGraph::CHUNK_SIZE)
    }

    /// Creates a new, empty transit network with the given road-surface chunk span in metres.
    pub fn new_with_surface_chunk_span(chunk_span_m: f32) -> Self {
        Self {
            cch_graph: CchGraph::new(0),
            cch_dirty_chunks: HashSet::new(),
            metric_dirty: false,
            lane_system: lanes::LaneSystem::new(),
            bulk_load: false,
            bulk_dirty_edges: HashSet::new(),
            flow_fields: FlowFieldSystem::new(),
            frontage_delay_elapsed_s: 0.0,
            road_surface: RoadSurfaceSystem::new(chunk_span_m),
        }
    }

    /// Completes a bulk-load sequence started by setting `bulk_load = true`.
    ///
    /// Rebuilds intersection clips incrementally for the dirty edge set. Returns the set of
    /// edges that were rebuilt so the caller can pass it to
    /// [`AgentSystem::invalidate_lane_ids_for_edges`].
    ///
    /// **Callers that need agent invalidation** should call
    /// `agent_system.invalidate_lane_ids_for_edges(&dirty, &self.lane_system)` **before**
    /// calling `lane_system.rebuild_edges_incremental`, i.e. they should inline the steps
    /// rather than using this helper. This method is provided for contexts where agent
    /// invalidation is not required (e.g. save-load restore).
    pub fn finalize_bulk_load(
        &mut self,
        graph: &mut RegionGraph,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) -> HashSet<usize> {
        self.bulk_load = false;
        graph.rebuild_intersection_clips();
        let dirty = std::mem::take(&mut self.bulk_dirty_edges);
        self.lane_system.rebuild_edges_incremental(graph, &dirty);
        allocator.rebuild_entrance_cache(graph, &self.lane_system);
        dirty
    }

    /// Clears the entire network, including zoning and building allocations.
    pub fn clear(
        &mut self,
        zoning: &mut crate::simulation::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        self.cch_graph = CchGraph::new(0);
        self.cch_dirty_chunks.clear();
        self.metric_dirty = false;
        self.lane_system.clear();
        self.frontage_delay_elapsed_s = 0.0;
        self.road_surface.clear();
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
        zoning: &mut crate::simulation::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        // 1. Simplify points using the same threshold shared with preview compilation.
        let mut simplified_points = RoadSurfaceSystem::simplify_road_input_points(&points);

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

        // 2. Apply the same Taubin height-smoothing pass shared with preview compilation.
        RoadSurfaceSystem::taubin_smooth_road_heights(&mut simplified_points);

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
        zoning: &mut crate::simulation::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        if start == end {
            return;
        }

        // Final sanity check on points
        if points.len() < 2 {
            return;
        }

        let edge_id = graph.add_edge(build_surface_edge(start, end, points, fwd, bkw, class));

        let (cost, length) =
            crate::simulation::pathing::cost::CostCalculator::calculate_costs(graph.edge(edge_id));
        graph.edges[edge_id].base_cost = cost;
        graph.edges[edge_id].physical_length = length;
        // Auto-flag high-speed roads; speed_limit is stored in m/s.
        if graph.edges[edge_id].speed_limit >= config::HIGH_SPEED_ROAD_THRESHOLD_MS {
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

        let mut surface_dirty_nodes: HashSet<u32> = HashSet::new();
        surface_dirty_nodes.insert(graph.get_valid_node(start));
        surface_dirty_nodes.insert(graph.get_valid_node(end));
        if edge_id < graph.edge_count() && !graph.edge(edge_id).deleted {
            surface_dirty_nodes.insert(graph.get_valid_node(graph.edge(edge_id).start_node));
            surface_dirty_nodes.insert(graph.get_valid_node(graph.edge(edge_id).end_node));
        }
        for new_nid in node_count_before as u32..graph.node_count() as u32 {
            surface_dirty_nodes.insert(graph.get_valid_node(new_nid));
        }
        for new_eid in edges_before..graph.edge_count() {
            if !graph.edge(new_eid).deleted {
                surface_dirty_nodes.insert(graph.get_valid_node(graph.edge(new_eid).start_node));
                surface_dirty_nodes.insert(graph.get_valid_node(graph.edge(new_eid).end_node));
            }
        }
        self.mark_surface_dirty_for_nodes(graph, &surface_dirty_nodes);

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
            let profile_changed_edges =
                graph.solve_junction_endpoint_profiles_for_edges(&affected_nodes, &affected_edges);
            affected_edges.extend(profile_changed_edges);
            let regrade_changed_edges =
                graph.regrade_junction_endpoint_profiles_for_nodes(&affected_nodes);
            affected_edges.extend(regrade_changed_edges);
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
        &mut self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        self.road_surface.compile_dirty(graph, terrain);
        let renderer = RoadRenderer;
        renderer.generate_mesh_data_with_surface(
            graph,
            &self.lane_system,
            terrain,
            &self.road_surface,
        )
    }

    /// Rebuilds terrain earthworks only for the currently dirty roadbed chunks.
    pub fn rebuild_dirty_terrain_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut crate::simulation::terrain::TerrainSystem,
    ) -> Vec<surface::SurfaceChunkKey> {
        self.road_surface.rebuild_dirty_earthworks(graph, terrain)
    }

    /// Rebuilds terrain earthworks for the entire world from the compiled roadbed cache.
    pub fn rebuild_all_terrain_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut crate::simulation::terrain::TerrainSystem,
    ) -> Vec<surface::SurfaceChunkKey> {
        self.road_surface.rebuild_all_earthworks(graph, terrain)
    }

    /// Synchronizes road elevations with the terrain heightmap.
    pub fn sync_to_terrain(
        &mut self,
        graph: &mut RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) {
        graph.sync_to_terrain(terrain);
        self.lane_system.rebuild(graph);

        // `sync_to_terrain` rewrites standard-edge section heights and then rebuilds junction
        // clips globally. Since the visible renderer and earthworks now consume only compiled
        // roadbed cache data, that cache must be invalidated for every visible surface edge here.
        let mut edge_ids = HashSet::new();
        let mut node_ids = HashSet::new();
        for (edge_idx, edge) in graph.edges().iter().enumerate() {
            if edge.deleted || !matches!(edge.primary_type, TransitType::Road | TransitType::Foot) {
                continue;
            }
            edge_ids.insert(edge_idx);
            node_ids.insert(graph.get_valid_node(edge.start_node));
            node_ids.insert(graph.get_valid_node(edge.end_node));
        }
        self.mark_surface_dirty_from_sets(graph, &edge_ids, &node_ids);
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
            log_road_connectivity(graph);
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

    /// Builds the CCH from scratch and immediately runs the connectivity check.
    ///
    /// Use this in editor paths that call `CchGraph::build` directly rather than going
    /// through `rebuild_pathing`. This ensures the `[ROAD_NET]` disconnection warning
    /// fires on every structural road edit, not only during simulation ticks.
    pub fn rebuild_cch_and_check(&mut self, graph: &RegionGraph) {
        self.cch_graph = CchGraph::build(graph);
        log_road_connectivity(graph);
    }

    /// Marks the chunk containing this world-space point as requiring CCH rebuild.
    pub fn mark_point_dirty(&mut self, pos: Vector3) {
        let coords = RegionGraph::get_chunk_coords(pos);
        self.cch_dirty_chunks.insert(coords);
    }

    /// Marks the provided road edges and nodes dirty in the replacement road-surface shell.
    pub fn mark_surface_dirty_from_sets(
        &mut self,
        graph: &RegionGraph,
        edge_ids: &HashSet<usize>,
        node_ids: &HashSet<u32>,
    ) {
        for &edge_idx in edge_ids {
            self.road_surface.mark_edge_dirty(graph, edge_idx);
        }
        for &node_id in node_ids {
            self.road_surface.mark_node_dirty(graph, node_id);
        }
    }

    /// Marks the provided nodes plus all of their incident non-deleted edges dirty.
    pub fn mark_surface_dirty_for_nodes(&mut self, graph: &RegionGraph, node_ids: &HashSet<u32>) {
        let mut edge_ids = HashSet::new();
        for &node_id in node_ids {
            let valid = graph.get_valid_node(node_id);
            if valid as usize >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_idx in graph.node_adjacency(valid) {
                if !graph.edge(edge_idx).deleted {
                    edge_ids.insert(edge_idx);
                }
            }
        }
        self.mark_surface_dirty_from_sets(graph, &edge_ids, node_ids);
    }

    /// Marks terrain-edit-adjacent roads and affected chunks dirty in the road-surface shell.
    pub fn mark_surface_dirty_for_terrain_edit(
        &mut self,
        graph: &RegionGraph,
        center: Vector2,
        radius_m: f32,
    ) {
        self.road_surface
            .mark_terrain_edit_dirty(graph, center, radius_m);
    }

    /// Marks the chunk containing this world-space point dirty in the road-surface shell.
    pub fn mark_surface_point_dirty(&mut self, pos: Vector3) {
        self.road_surface.mark_world_point_dirty(pos);
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

/// Performs a BFS over car-accessible nodes and logs connected-component stats.
///
/// BFS connectivity check run after every CCH topology rebuild.
///
/// Disconnected components are always reported to stdout so they appear in any log
/// regardless of which debug flags are active — a disconnected CCH is a P0 routing
/// failure and must never be silently swallowed. The "fully connected" confirmation
/// is kept behind `--debug traffic` (stderr) to avoid log noise in normal runs.
fn log_road_connectivity(graph: &RegionGraph) {
    use std::collections::{HashSet, VecDeque};

    // Collect all canonical nodes that have at least one active car edge.
    let mut car_nodes: HashSet<u32> = HashSet::new();
    let mut total_car_edges = 0usize;
    for edge in graph.edges() {
        if edge.deleted || (edge.allowed_types & TransitFlags::CAR) == 0 {
            continue;
        }
        total_car_edges += 1;
        car_nodes.insert(graph.get_valid_node(edge.start_node));
        car_nodes.insert(graph.get_valid_node(edge.end_node));
    }

    if car_nodes.is_empty() {
        println!("[ROAD_NET] CCH rebuilt: no car-accessible edges");
        return;
    }

    // BFS to identify connected components (treating each edge as undirected).
    let mut visited: HashSet<u32> = HashSet::new();
    let mut components: Vec<(usize, u32)> = Vec::new(); // (size, anchor_node)

    for &start in &car_nodes {
        if visited.contains(&start) {
            continue;
        }
        let mut queue = VecDeque::new();
        let mut size = 0usize;
        queue.push_back(start);
        visited.insert(start);
        while let Some(node) = queue.pop_front() {
            size += 1;
            if (node as usize) < graph.node_adjacency_count() {
                for &edge_idx in graph.node_adjacency(node) {
                    let e = graph.edge(edge_idx);
                    if e.deleted || (e.allowed_types & TransitFlags::CAR) == 0 {
                        continue;
                    }
                    for &nb_raw in &[e.start_node, e.end_node] {
                        let nb = graph.get_valid_node(nb_raw);
                        if car_nodes.contains(&nb) && !visited.contains(&nb) {
                            visited.insert(nb);
                            queue.push_back(nb);
                        }
                    }
                }
            }
        }
        components.push((size, start));
    }

    let total_nodes = car_nodes.len();
    let n_components = components.len();
    // Sort largest component first.
    components.sort_by(|a, b| b.0.cmp(&a.0));

    if n_components == 1 {
        println!(
            "[ROAD_NET] CCH rebuilt: edges={} car_nodes={} → 1 component, fully connected ✓",
            total_car_edges, total_nodes
        );
    } else {
        println!(
            "[ROAD_NET] CCH rebuilt: edges={} car_nodes={} → {} DISCONNECTED components ← NET-01",
            total_car_edges, total_nodes, n_components
        );
        for (idx, (size, anchor)) in components.iter().enumerate() {
            let pos = graph.node(*anchor).pos;
            let tag = if idx == 0 { "largest" } else { "ISOLATED" };
            println!(
                "[ROAD_NET]   component {}: {} nodes, anchor=N{} ({:.1},{:.1}) [{}]",
                idx, size, anchor, pos.x, pos.z, tag
            );
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
