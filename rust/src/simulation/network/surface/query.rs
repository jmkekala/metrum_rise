//! Runtime visible-surface queries and terrain clip extraction for compiled road surfaces.

use super::earthwork::EARTHWORK_PAVEMENT_DEPTH_M;
use super::{
    ChunkCacheKind, RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceTerrainClipExportError,
    RoadSurfaceTerrainClipLoop, RoadSurfaceVisualNodePiece, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, SurfaceChunkKey,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtEarthworkSupportPolicy, TerrainCdtEdgeClass, TerrainCdtNodePieceKind,
    TerrainCdtRoadBandKind, TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop,
    TerrainCdtRoadLoopSourceEdge, TerrainCdtSpanRegionRole, TerrainCdtVertex,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::HashSet;

impl RoadSurfaceSystem {
    pub(crate) fn terrain_render_patch_keys_with_visible_road(
        &self,
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        let mut patch_keys = HashSet::new();

        for piece in self.compiled_visual_span_pieces.values() {
            let Some((min, max)) = self.visual_span_piece_bounds(piece, ChunkCacheKind::Surface)
            else {
                continue;
            };
            for key in terrain.render_patch_keys_for_world_bounds(min.x, min.z, max.x, max.z) {
                patch_keys.insert(key);
            }
        }

        for piece in self.compiled_visual_node_pieces.values() {
            let Some((min, max)) = self.visual_node_piece_bounds(piece, ChunkCacheKind::Surface)
            else {
                continue;
            };
            for key in terrain.render_patch_keys_for_world_bounds(min.x, min.z, max.x, max.z) {
                patch_keys.insert(key);
            }
        }

        let mut keys: Vec<(usize, usize)> = patch_keys.into_iter().collect();
        keys.sort_unstable();
        keys
    }

    #[cfg(test)]
    pub(crate) fn terrain_clip_polygons_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<RoadSurfaceVisualPolygon> {
        self.terrain_clip_polygons_and_source_count_for_world_bounds(
            graph, min_x, min_z, max_x, max_z,
        )
        .expect("terrain clip export should preserve owned source coverage")
        .0
    }

    pub(crate) fn terrain_clip_polygons_and_source_count_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Result<(Vec<RoadSurfaceVisualPolygon>, usize), RoadSurfaceTerrainClipExportError> {
        let boundary_loops =
            self.terrain_clip_boundary_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z);
        let source_count = boundary_loops.len();
        Self::union_terrain_clip_boundary_loops(&boundary_loops)
            .map(|polygons| (polygons, source_count))
    }

    pub(crate) fn terrain_cdt_road_loops_and_clip_polygons_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Result<
        (
            Vec<TerrainCdtRoadLoop>,
            Vec<RoadSurfaceVisualPolygon>,
            usize,
        ),
        RoadSurfaceTerrainClipExportError,
    > {
        let boundary_loops =
            self.terrain_clip_boundary_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z);
        let source_count = boundary_loops.len();
        let export = Self::union_terrain_clip_boundary_export(&boundary_loops)?;
        let road_loops = export
            .loops
            .iter()
            .enumerate()
            .map(|(loop_index, boundary_loop)| {
                Self::terrain_cdt_road_loop_from_terrain_clip_loop(loop_index, boundary_loop)
            })
            .collect::<Vec<_>>();
        Ok((road_loops, export.polygons, source_count))
    }

    fn terrain_clip_boundary_loops_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<RoadSurfaceTerrainClipLoop> {
        let mut boundary_loops = Vec::new();

        for piece in self.compiled_visual_span_pieces.values() {
            if piece.edge_class != EdgeClass::Standard {
                continue;
            }
            Self::collect_terrain_clip_boundary_loops_from_piece(
                &piece.terrain_clip_boundary_loops,
                min_x,
                min_z,
                max_x,
                max_z,
                &mut boundary_loops,
            );
        }

        for (&node_id, piece) in &self.compiled_visual_node_pieces {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
            Self::collect_terrain_clip_boundary_loops_from_piece(
                &piece.terrain_clip_boundary_loops,
                min_x,
                min_z,
                max_x,
                max_z,
                &mut boundary_loops,
            );
        }

        boundary_loops
    }

    fn terrain_cdt_road_loop_from_terrain_clip_loop(
        loop_index: usize,
        boundary_loop: &RoadSurfaceTerrainClipLoop,
    ) -> TerrainCdtRoadLoop {
        let stable_piece_id = boundary_loop
            .source_edges
            .iter()
            .map(|edge| Self::terrain_cdt_stable_piece_id_for_source(edge.source))
            .min()
            .unwrap_or_else(|| u64::try_from(loop_index).unwrap_or(u64::MAX));
        let vertices = boundary_loop
            .points_world
            .iter()
            .map(|point| TerrainCdtVertex::new(f64::from(point.x), point.y, f64::from(point.z)))
            .collect::<Vec<_>>();
        let source_edges = boundary_loop
            .source_edges
            .iter()
            .map(|edge| TerrainCdtRoadLoopSourceEdge {
                start: TerrainCdtVertex::new(
                    f64::from(edge.start.x),
                    edge.start.y,
                    f64::from(edge.start.z),
                ),
                end: TerrainCdtVertex::new(
                    f64::from(edge.end.x),
                    edge.end.y,
                    f64::from(edge.end.z),
                ),
                source: Self::terrain_cdt_boundary_source_from_surface(edge.source),
            })
            .collect::<Vec<_>>();
        TerrainCdtRoadLoop::new_with_source_edges(
            stable_piece_id,
            u32::try_from(loop_index).unwrap_or(u32::MAX),
            vertices,
            source_edges,
        )
    }

    fn terrain_cdt_stable_piece_id_for_source(
        source: super::RoadSurfaceEarthworkFaceSource,
    ) -> u64 {
        match source {
            super::RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { edge_idx, .. } => {
                u64::try_from(edge_idx).unwrap_or(u64::MAX)
            }
            super::RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { node_id, .. } => {
                (1_u64 << 63) | u64::from(node_id)
            }
        }
    }

    fn terrain_cdt_boundary_source_from_surface(
        source: super::RoadSurfaceEarthworkFaceSource,
    ) -> TerrainCdtRoadBoundarySource {
        match source {
            super::RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                edge_idx,
                edge_class,
                support_policy,
                owner,
                role,
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m,
            } => TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: u64::try_from(edge_idx).unwrap_or(u64::MAX),
                edge_class: Self::terrain_cdt_edge_class(edge_class),
                support_policy: Self::terrain_cdt_support_policy(support_policy),
                source_band_index: u32::try_from(owner.source_band_index).unwrap_or(u32::MAX),
                band_kind: Self::terrain_cdt_band_kind(owner.kind),
                role: Self::terrain_cdt_span_region_role(role),
                start_section_index: u32::try_from(start_section_index).unwrap_or(u32::MAX),
                end_section_index: u32::try_from(end_section_index).unwrap_or(u32::MAX),
                start_s_m,
                end_s_m,
            },
            super::RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id,
                kind,
                owner_kind,
                owner_index,
                ..
            } => TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id,
                node_kind: Self::terrain_cdt_node_piece_kind(kind),
                owner_kind: Self::terrain_cdt_band_kind(owner_kind),
                owner_index: u32::try_from(owner_index).unwrap_or(u32::MAX),
            },
        }
    }

    fn terrain_cdt_edge_class(edge_class: EdgeClass) -> TerrainCdtEdgeClass {
        match edge_class {
            EdgeClass::Standard => TerrainCdtEdgeClass::Standard,
            EdgeClass::Bridge => TerrainCdtEdgeClass::Bridge,
            EdgeClass::Tunnel => TerrainCdtEdgeClass::Tunnel,
        }
    }

    fn terrain_cdt_support_policy(
        policy: super::RoadSurfaceEarthworkSupportPolicy,
    ) -> TerrainCdtEarthworkSupportPolicy {
        match policy {
            super::RoadSurfaceEarthworkSupportPolicy::StandardFullGroundedSpan => {
                TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan
            }
            super::RoadSurfaceEarthworkSupportPolicy::BridgeEndpointAbutments => {
                TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments
            }
            super::RoadSurfaceEarthworkSupportPolicy::TunnelVisiblePortals => {
                TerrainCdtEarthworkSupportPolicy::TunnelVisiblePortals
            }
        }
    }

    fn terrain_cdt_band_kind(kind: super::RoadSurfaceBandKind) -> TerrainCdtRoadBandKind {
        match kind {
            super::RoadSurfaceBandKind::Carriageway => TerrainCdtRoadBandKind::Carriageway,
            super::RoadSurfaceBandKind::CurbOrShoulder => TerrainCdtRoadBandKind::CurbOrShoulder,
            super::RoadSurfaceBandKind::Sidewalk => TerrainCdtRoadBandKind::Sidewalk,
            super::RoadSurfaceBandKind::Footpath => TerrainCdtRoadBandKind::Footpath,
            super::RoadSurfaceBandKind::Median => TerrainCdtRoadBandKind::Median,
            super::RoadSurfaceBandKind::Parking => TerrainCdtRoadBandKind::Parking,
            super::RoadSurfaceBandKind::CycleTrack => TerrainCdtRoadBandKind::CycleTrack,
            super::RoadSurfaceBandKind::TramReservation => TerrainCdtRoadBandKind::TramReservation,
        }
    }

    fn terrain_cdt_span_region_role(
        role: super::RoadSurfaceSpanRegionRole,
    ) -> TerrainCdtSpanRegionRole {
        match role {
            super::RoadSurfaceSpanRegionRole::Asphalt => TerrainCdtSpanRegionRole::Asphalt,
            super::RoadSurfaceSpanRegionRole::CurbOrShoulder => {
                TerrainCdtSpanRegionRole::CurbOrShoulder
            }
            super::RoadSurfaceSpanRegionRole::NonRoad => TerrainCdtSpanRegionRole::NonRoad,
        }
    }

    fn terrain_cdt_node_piece_kind(
        kind: super::RoadSurfaceVisualNodePieceKind,
    ) -> TerrainCdtNodePieceKind {
        match kind {
            super::RoadSurfaceVisualNodePieceKind::Terminal => TerrainCdtNodePieceKind::Terminal,
            super::RoadSurfaceVisualNodePieceKind::Bend => TerrainCdtNodePieceKind::Bend,
            super::RoadSurfaceVisualNodePieceKind::JunctionN => TerrainCdtNodePieceKind::JunctionN,
        }
    }

    pub(crate) fn sample_visible_surface_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let chunk = self.chunk_coords_for_world(world_x, world_z);
        let (edge_indices, node_ids) = self.collect_query_contributors(chunk, chunk);
        let point = Vector2::new(world_x, world_z);
        let mut best_height_m: Option<f32> = None;

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_piece_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                    best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
                },
            );
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_span_piece_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            self.visit_span_piece_earthwork_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                continue;
            }
            self.visit_node_piece_earthwork_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                    best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
                },
            );
        }

        best_height_m
    }

    #[cfg(test)]
    pub(crate) fn sample_paved_support_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let chunk = self.chunk_coords_for_world(world_x, world_z);
        let (edge_indices, node_ids) = self.collect_query_contributors(chunk, chunk);
        let point = Vector2::new(world_x, world_z);
        let mut best_height_m: Option<f32> = None;

        // Terrain support clearance is a lower envelope: where terminal caps, spans, or raised
        // bands overlap in XZ, terrain must remain below every road-owned top surface. Visible
        // picking uses the highest rendered surface instead.
        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
                continue;
            }
            let height_offset_m =
                self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);

            for polygon in piece
                .road_surface_polygons
                .iter()
                .chain(&piece.curb_surface_polygons)
                .chain(&piece.sidewalk_surface_polygons)
            {
                Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc
                        - height_offset_m;
                    best_height_m = Some(best_height_m.map_or(height_m, |best| best.min(height_m)));
                });
            }
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
            self.visit_span_piece_clearance_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m =
                    triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc - height_offset_m;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.min(height_m)));
            });
        }

        best_height_m
    }

    pub(crate) fn raycast_visible_surface(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        if ray_dir.length_squared() <= f32::EPSILON {
            return None;
        }

        let terrain_hit = terrain.raycast_visual_terrain(ray_origin, ray_dir)?;
        let terrain_t =
            (terrain_hit - ray_origin).dot(ray_dir) / ray_dir.length_squared().max(f32::EPSILON);
        if terrain_t < 0.0 {
            return Some(terrain_hit);
        }

        let min_chunk = self.chunk_coords_for_world(
            ray_origin.x.min(terrain_hit.x),
            ray_origin.z.min(terrain_hit.z),
        );
        let max_chunk = self.chunk_coords_for_world(
            ray_origin.x.max(terrain_hit.x),
            ray_origin.z.max(terrain_hit.z),
        );
        let (edge_indices, node_ids) = self.collect_query_contributors(min_chunk, max_chunk);

        let mut best_t = terrain_t;
        let mut best_hit = None;

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_piece_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                    else {
                        return;
                    };
                    if t >= 0.0 && t <= best_t {
                        best_t = t;
                        best_hit = Some(ray_origin + ray_dir * t);
                    }
                },
            );
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_span_piece_triangles(piece, &mut |triangle| {
                let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                else {
                    return;
                };
                if t >= 0.0 && t <= best_t {
                    best_t = t;
                    best_hit = Some(ray_origin + ray_dir * t);
                }
            });
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            self.visit_span_piece_earthwork_triangles(piece, &mut |triangle| {
                let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                else {
                    return;
                };
                if t >= 0.0 && t <= best_t {
                    best_t = t;
                    best_hit = Some(ray_origin + ray_dir * t);
                }
            });
        }

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                continue;
            }
            self.visit_node_piece_earthwork_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                    else {
                        return;
                    };
                    if t >= 0.0 && t <= best_t {
                        best_t = t;
                        best_hit = Some(ray_origin + ray_dir * t);
                    }
                },
            );
        }

        best_hit.or(Some(terrain_hit))
    }

    pub(crate) fn node_uses_visible_surface(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> bool {
        if node_id as usize >= graph.node_adjacency_count() {
            return false;
        }

        let mut has_supported_surface = false;
        let mut has_visible_surface_attachment = false;
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                return false;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted {
                continue;
            }
            if !Self::is_surface_edge(edge) || !self.compiled_sections.contains_key(&edge_idx) {
                return false;
            }

            has_supported_surface = true;
            if edge.primary_type == TransitType::Foot || edge.class != EdgeClass::Tunnel {
                has_visible_surface_attachment = true;
                continue;
            }

            let at_start = graph.get_valid_node(edge.start_node) == node_id;
            if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                has_visible_surface_attachment = true;
            }
        }

        has_supported_surface && has_visible_surface_attachment
    }

    pub(crate) fn span_piece_uses_visible_earthwork(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> bool {
        piece.edge_class != EdgeClass::Standard
    }

    pub(crate) fn node_piece_uses_visible_earthwork(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> bool {
        if node_id as usize >= graph.node_adjacency_count() {
            return false;
        }

        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || !Self::is_surface_edge(edge) {
                continue;
            }

            match edge.class {
                EdgeClass::Standard => {}
                EdgeClass::Bridge => return true,
                EdgeClass::Tunnel => {
                    let at_start = graph.get_valid_node(edge.start_node) == node_id;
                    if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                        return true;
                    }
                }
            }
        }

        false
    }

    pub(super) fn span_piece_integrated_surface_offset_m(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> f32 {
        if self.span_piece_uses_visible_earthwork(piece) {
            EARTHWORK_PAVEMENT_DEPTH_M
        } else {
            0.0
        }
    }

    pub(super) fn node_piece_integrated_surface_offset_m(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> f32 {
        if self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            EARTHWORK_PAVEMENT_DEPTH_M
        } else {
            0.0
        }
    }

    pub(crate) fn visible_section_ranges_for_edge(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
        sections: &[RoadSurfaceSection],
    ) -> Vec<(usize, usize)> {
        let Some((start_index, end_index)) =
            self.visible_corridor_index_range_for_edge(graph, edge_idx, sections)
        else {
            return Vec::new();
        };
        if graph.edge(edge_idx).class != EdgeClass::Tunnel {
            return vec![(start_index, end_index)];
        }

        self.tunnel_visible_section_ranges(sections, start_index, end_index, terrain)
    }
    fn collect_query_contributors(
        &self,
        min_chunk: SurfaceChunkKey,
        max_chunk: SurfaceChunkKey,
    ) -> (Vec<usize>, Vec<u32>) {
        let mut edge_indices = Vec::new();
        let mut node_ids = Vec::new();
        for cx in (min_chunk.0 - 1)..=(max_chunk.0 + 1) {
            for cz in (min_chunk.1 - 1)..=(max_chunk.1 + 1) {
                let chunk = (cx, cz);
                if let Some(entry) = self.surface_chunk_cache.get(&chunk) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
                if let Some(entry) = self.earthwork_chunk_cache.get(&chunk) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
            }
        }

        edge_indices.sort_unstable();
        edge_indices.dedup();
        node_ids.sort_unstable();
        node_ids.dedup();
        (edge_indices, node_ids)
    }

    fn visit_visible_span_piece_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for region in &piece.span_owned_regions {
            Self::visit_visual_polygon_triangles(&region.polygon, visitor);
        }
    }

    fn visit_span_piece_earthwork_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for polygon in &piece.earthwork_surface_polygons {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    #[cfg(test)]
    fn visit_span_piece_clearance_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for region in &piece.span_earthwork_support_regions {
            Self::visit_visual_polygon_triangles(&region.polygon, visitor);
        }
    }

    fn visit_visible_node_piece_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        if !self.node_uses_visible_surface(graph, terrain, node_id) {
            return;
        }

        for polygon in piece
            .road_surface_polygons
            .iter()
            .chain(&piece.curb_surface_polygons)
            .chain(&piece.sidewalk_surface_polygons)
        {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    fn visit_node_piece_earthwork_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
            return;
        }

        for polygon in &piece.earthwork_surface_polygons {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    pub(super) fn visit_visual_polygon_triangles<F>(
        polygon: &RoadSurfaceVisualPolygon,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for &triangle in &polygon.triangles_world {
            if Self::triangle_has_area_xz(triangle) {
                visitor(triangle);
            }
        }
    }

    fn visible_corridor_index_range_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        sections: &[RoadSurfaceSection],
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 || edge_idx >= graph.edge_count() {
            return None;
        }

        let edge = graph.edge(edge_idx);
        let total_length = sections.last()?.s_m.max(0.0);
        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let Some((start_handoff, end_handoff)) = self.visual_surface_handoff_range_for_edge(
            graph,
            edge_idx,
            edge,
            total_length,
            start_kind,
            end_kind,
        ) else {
            return None;
        };

        Self::section_index_range_for_s_bounds(sections, start_handoff, end_handoff)
    }

    fn collect_terrain_clip_boundary_loops_from_piece(
        source: &[RoadSurfaceTerrainClipLoop],
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        out: &mut Vec<RoadSurfaceTerrainClipLoop>,
    ) {
        for boundary_loop in source {
            if Self::visual_points_overlap_bounds_xz(
                &boundary_loop.points_world,
                min_x,
                min_z,
                max_x,
                max_z,
            ) {
                out.push(boundary_loop.clone());
            }
        }
    }

    fn visual_points_overlap_bounds_xz(
        points_world: &[Vector3],
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> bool {
        let mut polygon_min_x = f32::MAX;
        let mut polygon_max_x = f32::MIN;
        let mut polygon_min_z = f32::MAX;
        let mut polygon_max_z = f32::MIN;
        for point in points_world {
            polygon_min_x = polygon_min_x.min(point.x);
            polygon_max_x = polygon_max_x.max(point.x);
            polygon_min_z = polygon_min_z.min(point.z);
            polygon_max_z = polygon_max_z.max(point.z);
        }

        polygon_min_x <= max_x
            && polygon_max_x >= min_x
            && polygon_min_z <= max_z
            && polygon_max_z >= min_z
    }
}
