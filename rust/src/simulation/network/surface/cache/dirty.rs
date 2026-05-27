//! Dirty tracking and graph-surface membership helpers.

use super::*;

impl RoadSurfaceSystem {
    /// Clears compiled caches and dirty tracking without changing the configured chunk span.
    pub fn clear(&mut self) {
        self.clear_dirty_tracking();
        self.compiled_sections.clear();
        self.compiled_visual_span_pieces.clear();
        self.compiled_visual_node_pieces.clear();
        self.clear_piece_chunk_coverage();
        self.surface_chunk_cache.clear();
        self.earthwork_chunk_cache.clear();
        self.last_rebuilt_surface_chunks.clear();
        self.last_rebuilt_terrain_chunks.clear();
        self.compiled_once = false;
    }

    /// Clears only the dirty tracking sets.
    pub fn clear_dirty_tracking(&mut self) {
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.dirty_surface_chunks.clear();
        self.dirty_terrain_chunks.clear();
    }

    /// Reconfigures the chunk span and clears all caches and dirty sets.
    pub fn set_chunk_span_m(&mut self, chunk_span_m: f32) {
        self.chunk_span_m = chunk_span_m.max(f32::EPSILON);
        self.clear();
    }

    /// Marks one world-space point as dirty for both surface and terrain chunk rebuilds.
    pub fn mark_world_point_dirty(&mut self, pos: Vector3) {
        let pos = godot_vec3_to_road(pos);
        self.mark_road_world_point_dirty(pos);
    }

    fn mark_road_world_point_dirty(&mut self, pos: RoadVec3) {
        let chunk = self.chunk_coords_for_world(pos.x, pos.z);
        self.dirty_surface_chunks.insert(chunk);
        self.dirty_terrain_chunks.insert(chunk);
    }

    /// Marks one world-space AABB as dirty for both surface and terrain chunk rebuilds.
    pub fn mark_world_aabb_dirty(&mut self, min: Vector3, max: Vector3) {
        let min = godot_vec3_to_road(min);
        let max = godot_vec3_to_road(max);
        self.mark_road_world_aabb_dirty(min, max);
    }

    fn mark_road_world_aabb_dirty(&mut self, min: RoadVec3, max: RoadVec3) {
        let min_chunk = self.chunk_coords_for_world(min.x.min(max.x), min.z.min(max.z));
        let max_chunk = self.chunk_coords_for_world(min.x.max(max.x), min.z.max(max.z));
        for cx in min_chunk.0..=max_chunk.0 {
            for cz in min_chunk.1..=max_chunk.1 {
                let chunk = (cx, cz);
                self.dirty_surface_chunks.insert(chunk);
                self.dirty_terrain_chunks.insert(chunk);
            }
        }
    }

    /// Marks one edge dirty; chunk invalidation is derived from compiled piece coverage.
    pub fn mark_edge_dirty(&mut self, graph: &RegionGraph, edge_idx: usize) {
        if edge_idx >= graph.edge_count() {
            return;
        }
        self.dirty_edges.insert(edge_idx);
    }

    /// Marks one node dirty; chunk invalidation is derived from compiled piece coverage.
    pub fn mark_node_dirty(&mut self, graph: &RegionGraph, node_id: u32) {
        if node_id as usize >= graph.node_count() {
            return;
        }
        let valid = graph.get_valid_node(node_id);
        self.dirty_nodes.insert(valid);
    }

    /// Marks a terrain edit dirty using the brush center and radius in world metres.
    ///
    /// This marks both touched terrain chunks and any nearby road edges / nodes whose compiled
    /// roadbed may need recompilation when terrain-dependent grades are rebuilt.
    pub fn mark_terrain_edit_dirty(&mut self, graph: &RegionGraph, center: Vector2, radius_m: f32) {
        let radius_m = f64::from(radius_m.max(0.0));
        let center: RoadVec2 = godot_vec2_to_road(center);
        let min = RoadVec3::new(center.x - radius_m, 0.0, center.y - radius_m);
        let max = RoadVec3::new(center.x + radius_m, 0.0, center.y + radius_m);
        self.mark_road_world_aabb_dirty(min, max);

        let graph_min = Vector3::new(min.x as f32, min.y as f32, min.z as f32);
        let graph_max = Vector3::new(max.x as f32, max.y as f32, max.z as f32);
        for edge_idx in graph.get_edges_near_aabb(graph_min, graph_max) {
            self.mark_edge_dirty(graph, edge_idx);
            let edge = graph.edge(edge_idx);
            self.mark_node_dirty(graph, edge.start_node);
            self.mark_node_dirty(graph, edge.end_node);
        }
    }

    pub(in crate::simulation::network::surface) fn all_surface_edge_ids(
        &self,
        graph: &RegionGraph,
    ) -> Vec<usize> {
        graph
            .edges()
            .iter()
            .enumerate()
            .filter_map(|(edge_idx, edge)| Self::is_surface_edge(edge).then_some(edge_idx))
            .collect()
    }

    pub(in crate::simulation::network::surface) fn all_surface_node_ids(
        &self,
        graph: &RegionGraph,
    ) -> Vec<u32> {
        let mut node_ids = HashSet::new();
        for edge in graph.edges() {
            if !Self::is_surface_edge(edge) {
                continue;
            }
            node_ids.insert(graph.get_valid_node(edge.start_node));
            node_ids.insert(graph.get_valid_node(edge.end_node));
        }
        let mut node_ids: Vec<u32> = node_ids.into_iter().collect();
        node_ids.sort_unstable();
        node_ids
    }

    pub(in crate::simulation::network::surface) fn node_has_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> bool {
        (node_id as usize) < graph.node_adjacency_count()
            && graph.node_adjacency(node_id).iter().any(|&edge_idx| {
                edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(edge_idx))
            })
    }

    pub(in crate::simulation::network::surface) fn node_has_standard_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> bool {
        (node_id as usize) < graph.node_adjacency_count()
            && graph.node_adjacency(node_id).iter().any(|&edge_idx| {
                if edge_idx >= graph.edge_count() {
                    return false;
                }
                let edge = graph.edge(edge_idx);
                Self::is_surface_edge(edge) && edge.class == EdgeClass::Standard
            })
    }

    pub(in crate::simulation::network::surface) fn is_surface_edge(edge: &Edge) -> bool {
        !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
    }

    pub(in crate::simulation::network::surface) fn edge_points<'a>(
        &self,
        edge: &'a Edge,
    ) -> &'a [Vector3] {
        if edge.physical_geometry.is_empty() {
            &edge.geometry
        } else {
            &edge.physical_geometry
        }
    }
}
