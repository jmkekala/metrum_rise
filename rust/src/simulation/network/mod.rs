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
pub mod terrain;
pub mod topology;
pub mod lanes;
use crate::config;
use std::collections::HashSet;

use crate::simulation::pathing::cch::CchGraph;
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
    /// Edges that need their zoning obstruction cache recalculated.
    pub zoning_dirty_edges: HashSet<usize>,
    /// Pre-calculated lane geometry graph for fast agent traversal.
    pub lane_system: lanes::LaneSystem,
}

impl TransitNetwork {
    pub fn new() -> Self {
        Self {
            cch_graph: CchGraph::new(0),
            cch_dirty_chunks: HashSet::new(),
            metric_dirty: false,
            zoning_dirty_edges: HashSet::new(),
            lane_system: lanes::LaneSystem::new(),
        }
    }

    pub fn clear(
        &mut self,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        self.cch_graph = CchGraph::new(0);
        self.cch_dirty_chunks.clear();
        self.metric_dirty = false;
        self.zoning_dirty_edges.clear();
        self.lane_system.clear();
        zoning.clear();
        allocator.clear();
    }

    pub fn add_road(
        &mut self,
        graph: &mut RegionGraph,
        points: Vec<Vector3>,
        fwd_lanes: u8,
        bkw_lanes: u8,
        zoning_left: bool,
        zoning_right: bool,
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
        simplified_points[0] = graph.nodes[start_id as usize].pos;
        simplified_points[count - 1] = graph.nodes[end_id as usize].pos;

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

        // 3. SUBDIVISION LOGIC (Every 100m)
        let mut current_start_id = start_id;
        let mut active_segment = vec![simplified_points[0]];
        let mut accumulated_dist = 0.0;

        for i in 0..count - 1 {
            let p0 = simplified_points[i];
            let p1 = simplified_points[i + 1];
            let d = p0.distance_to(p1);

            if accumulated_dist + d > 100.0 {
                // Determine how many splits we need in this segment
                let remaining_in_segment = 100.0 - accumulated_dist;
                let mut t = remaining_in_segment / d;

                while t <= 1.0 {
                    let split_pos = p0.lerp(p1, t);
                    active_segment.push(split_pos);

                    // Create intermediate node
                    let mid_id = graph.find_or_add_node(split_pos, 0.1, NodeType::Junction);

                    // Add this edge
                    self.create_edge_internal(
                        graph,
                        current_start_id,
                        mid_id,
                        active_segment.clone(),
                        fwd_lanes,
                        bkw_lanes,
                        zoning_left,
                        zoning_right,
                        class,
                        zoning,
                        allocator,
                    );

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
            active_segment[last_idx] = graph.nodes[end_id as usize].pos;
            self.create_edge_internal(
                graph,
                current_start_id,
                end_id,
                active_segment,
                fwd_lanes,
                bkw_lanes,
                zoning_left,
                zoning_right,
                class,
                zoning,
                allocator,
            );
        }

        self.flush_zoning_updates(zoning, graph);
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
        zoning_left: bool,
        zoning_right: bool,
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
            zoning_left,
            zoning_right,
            deleted: false,
        });

        let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(
            &graph.edges[edge_id],
        );
        graph.edges[edge_id].base_cost = cost;
        graph.edges[edge_id].physical_length = length;

        zoning.update_edge_grid_size(edge_id, length);
        self.zoning_dirty_edges.insert(edge_id);
        self.invalidate_zoning_near_edge(edge_id, graph);
        self.mark_point_dirty(graph.nodes[start as usize].pos);
        self.mark_point_dirty(graph.nodes[end as usize].pos);

        topology::process_intersections(self, graph, edge_id, zoning, allocator);
        self.cleanup_duplicate_edges(graph); // Clean edge_id if it's dup
        graph.rebuild_intersection_clips();
        self.lane_system.rebuild(graph);

        // Mark chunks as dirty
        let chunks = graph.get_edge_chunks(edge_id);
        self.cch_dirty_chunks.extend(chunks);
    }

    pub fn generate_mesh_data(
        &self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        let renderer = RoadRenderer;
        renderer.generate_mesh_data(graph, &self.lane_system, terrain)
    }

    pub fn invalidate_zoning_near_edge(&mut self, edge_id: usize, graph: &RegionGraph) {
        if edge_id >= graph.edges.len() {
            return;
        }
        let edge = &graph.edges[edge_id];
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        for p in &edge.physical_geometry {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_z = min_z.min(p.z);
            max_z = max_z.max(p.z);
        }
        let padding = 125.0; // Zoning depth is 100m, so 125m is safe
        let nearby = graph.get_edges_near_aabb(
            Vector3::new(min_x - padding, 0.0, min_z - padding),
            Vector3::new(max_x + padding, 0.0, max_z + padding),
        );
        for &e_idx in &nearby {
            self.zoning_dirty_edges.insert(e_idx);
        }
    }

    pub fn flush_zoning_updates(
        &mut self,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        graph: &RegionGraph,
    ) {
        let dirty: Vec<usize> = self.zoning_dirty_edges.drain().collect();
        for &edge_idx in &dirty {
            if edge_idx < graph.edges.len() && !graph.edges[edge_idx].deleted {
                zoning.recalculate_obstructions(edge_idx, graph);
            }
        }
    }

    pub fn flatten_terrain(
        &self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
        output_heightmap: &mut [f32],
        map_size: Vector2,
    ) {
        terrain::flatten_terrain_for_network(graph, terrain, output_heightmap, map_size);
    }

    pub fn sync_to_terrain(
        &mut self,
        graph: &mut RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) {
        graph.sync_to_terrain(terrain);
        self.lane_system.rebuild(graph);
    }

    pub fn rebuild_pathing(&mut self, graph: &mut RegionGraph) {
        // Topology changes (Phase 1)
        if !self.cch_dirty_chunks.is_empty() {
            self.cch_graph = CchGraph::build(graph);
            self.cch_dirty_chunks.clear();
            self.metric_dirty = false; // Phase 1 includes customize
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

    /// Inserts a real graph node at fractional position `t` (0–1 of arc length) along
    /// `edge_idx`, creating a precise road-side waypoint for a building's frontage.
    /// The original edge A→B is split into A→F (first half, kept at `edge_idx`) and F→B
    /// (second half, new edge). Zoning grids, building `cell_x`/`frontage_t` values, lane
    /// geometry, and CCH dirty chunks are all updated atomically.
    ///
    /// Returns the ID of the newly created [`NodeType::Junction`] at the split position.
    /// The new building always lands on the first half (`edge_idx`); the caller can use
    /// its original `cell_x` unchanged.
    pub fn split_for_frontage(
        &mut self,
        graph: &mut RegionGraph,
        edge_idx: usize,
        t: f32,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) -> u32 {
        // Walk the geometry to find the segment index and 3-D position at arc-distance t.
        let target_arc = t.clamp(0.0, 1.0) * graph.edges[edge_idx].physical_length;
        let geo: Vec<Vector3> = graph.edges[edge_idx].geometry.clone();

        let mut curr = 0.0_f32;
        let mut seg = geo.len().saturating_sub(2);
        let mut split_pos = *geo.last().unwrap_or(&Vector3::ZERO);

        'walk: for i in 0..geo.len().saturating_sub(1) {
            let d = geo[i].distance_to(geo[i + 1]);
            if curr + d >= target_arc {
                let local_t = if d > 1e-6 { (target_arc - curr) / d } else { 0.0 };
                split_pos = geo[i].lerp(geo[i + 1], local_t);
                seg = i;
                break 'walk;
            }
            curr += d;
        }

        // Create the frontage node exactly at the split position.
        let f = graph.add_node(split_pos, NodeType::Frontage);

        // Split the edge; migrates zoning grids and building cell_x/frontage_t in-place.
        topology::split_edge(self, graph, edge_idx, seg, 0.0, f, zoning, allocator);
        let new_edge_id = graph.edges.len() - 1;

        // Recompute frontage_node for all existing buildings on either half-edge.
        for b in &mut allocator.buildings {
            if b.edge_idx == edge_idx || b.edge_idx == new_edge_id {
                let half_cells = b.width_cells as f32 * 0.5;
                b.frontage_node = if b.frontage_t < 0.5 {
                    graph.edges[b.edge_idx].start_node
                } else {
                    graph.edges[b.edge_idx].end_node
                };
            }
        }

        graph.rebuild_intersection_clips();
        self.lane_system.rebuild(graph);
        self.cch_dirty_chunks.extend(graph.get_edge_chunks(edge_idx));
        self.cch_dirty_chunks.extend(graph.get_edge_chunks(new_edge_id));

        f
    }

    /// Removes the frontage node `frontage_node` (created by [`split_for_frontage`]), merging
    /// its two incident half-edges back into one. All dependents are updated atomically.
    ///
    /// If `frontage_node` does not have exactly two compatible incident edges (e.g. it is a
    /// real intersection junction with degree > 2), the call is a no-op.
    pub fn remove_frontage(
        &mut self,
        graph: &mut RegionGraph,
        frontage_node: u32,
        zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
        allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        // Measure the first-half edge (end_node == frontage_node) before the graph is mutated.
        let cell_size = zoning.config.zone_cell_m;
        let first_half_len = graph
            .adjacency
            .get(frontage_node as usize)
            .and_then(|adj| {
                adj.iter().find_map(|&e_idx| {
                    let e = &graph.edges[e_idx];
                    if !e.deleted && e.end_node == frontage_node {
                        Some(e.physical_length)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0.0);
        let split_x = (first_half_len / cell_size).floor() as usize;

        if let Some((first_idx, second_idx)) = graph.remove_node_and_merge_edges(frontage_node) {
            // Remap buildings from the now-deleted second half onto the merged edge.
            let merged_len = graph.edges[first_idx].physical_length.max(0.001);
            for b in &mut allocator.buildings {
                if b.edge_idx == second_idx {
                    b.edge_idx = first_idx;
                    b.cell_x += split_x;
                    let frontage_offset = b.width_cells as f32 * 0.5;
                    b.frontage_t = (b.cell_x as f32 + frontage_offset) * cell_size / merged_len;
                }
            }

            // Recompute frontage_node for all buildings now on the merged edge.
            let start = graph.edges[first_idx].start_node;
            let end = graph.edges[first_idx].end_node;
            for b in &mut allocator.buildings {
                if b.edge_idx == first_idx {
                    b.frontage_node = if b.frontage_t < 0.5 { start } else { end };
                }
            }

            zoning.merge_edge_grids(first_idx, second_idx);

            graph.rebuild_intersection_clips();
            self.lane_system.rebuild(graph);
            self.cch_dirty_chunks.extend(graph.get_edge_chunks(first_idx));
        }
    }

    fn cleanup_duplicate_edges(&mut self, graph: &mut RegionGraph) {
        let mut seen = std::collections::HashSet::new();
        let mut to_remove = Vec::new();

        for (i, edge) in graph.edges.iter().enumerate() {
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

pub mod test_clips;
pub mod test_compaction;
pub mod test_topology;
pub mod test_verify;
pub mod test_frontage_crossing;
pub mod test_uturn;
pub mod test_ped_junction;
