//! Terrain-patch road loop extraction and CDT source adaptation.

use super::super::{
    ChunkCacheKind, NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
    NodeFootprintBoundaryVertexSource, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSpanRegionRole, RoadSurfaceSystem,
    RoadSurfaceTerrainClipContourRole, RoadSurfaceTerrainClipExport,
    RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipLoopTopology, RoadSurfaceVisualNodePieceKind,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtEarthworkSupportPolicy, TerrainCdtEdgeClass,
    TerrainCdtNodeFootprintBoundaryDirectSource, TerrainCdtNodeFootprintBoundarySegmentSource,
    TerrainCdtNodeFootprintBoundaryVertexSource, TerrainCdtNodePieceKind, TerrainCdtRoadBandKind,
    TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop, TerrainCdtRoadLoopSourceEdge,
    TerrainCdtSpanRegionRole, TerrainCdtVertex,
};
use godot::prelude::Vector3;
use std::collections::{BTreeMap, HashSet};

impl RoadSurfaceSystem {
    pub(crate) fn terrain_render_patch_keys_with_visible_road(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        let mut patch_keys = HashSet::new();

        let mut span_pieces = self.compiled_visual_span_pieces.iter().collect::<Vec<_>>();
        span_pieces.sort_by_key(|(edge_idx, _)| **edge_idx);
        for (_, piece) in span_pieces {
            if piece.edge_class != EdgeClass::Standard {
                continue;
            }
            let Some((min, max)) = self.visual_span_piece_bounds(piece, ChunkCacheKind::Surface)
            else {
                continue;
            };
            for key in terrain.render_patch_keys_for_world_bounds(min.x, min.z, max.x, max.z) {
                patch_keys.insert(key);
            }
        }

        let mut node_pieces = self.compiled_visual_node_pieces.iter().collect::<Vec<_>>();
        node_pieces.sort_by_key(|(node_id, _)| **node_id);
        for (&node_id, piece) in node_pieces {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
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

    pub(crate) fn terrain_cdt_road_loops_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Result<(Vec<TerrainCdtRoadLoop>, usize), RoadSurfaceTerrainClipExportError> {
        let boundary_loops =
            self.terrain_clip_boundary_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z);
        let source_count = boundary_loops.len();
        let export = Self::union_terrain_clip_boundary_export(&boundary_loops)?;
        let footprint_group_ids =
            Self::terrain_cdt_stable_footprint_group_ids_for_terrain_clip_export(&export);
        let road_loops = export
            .loops
            .iter()
            .enumerate()
            .map(|(loop_index, boundary_loop)| {
                let topology = export.loop_topologies[loop_index];
                let footprint_group_id = footprint_group_ids
                    .get(&topology.shape_index)
                    .copied()
                    .expect("terrain clip export topology must have a stable footprint group id");
                Self::terrain_cdt_road_loop_from_terrain_clip_loop(
                    loop_index,
                    boundary_loop,
                    topology,
                    footprint_group_id,
                )
            })
            .collect::<Vec<_>>();
        Ok((road_loops, source_count))
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

        let mut span_pieces = self.compiled_visual_span_pieces.iter().collect::<Vec<_>>();
        span_pieces.sort_by_key(|(edge_idx, _)| **edge_idx);
        for (_, piece) in span_pieces {
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

        let mut node_pieces = self.compiled_visual_node_pieces.iter().collect::<Vec<_>>();
        node_pieces.sort_by_key(|(node_id, _)| **node_id);
        for (&node_id, piece) in node_pieces {
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
        topology: RoadSurfaceTerrainClipLoopTopology,
        footprint_group_id: u64,
    ) -> TerrainCdtRoadLoop {
        let stable_piece_id =
            Self::terrain_cdt_stable_piece_id_for_terrain_clip_loop(boundary_loop, loop_index);
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
        TerrainCdtRoadLoop::new_with_source_edges_and_topology(
            stable_piece_id,
            footprint_group_id,
            terrain_cdt_usize_to_u32(loop_index),
            topology.role == RoadSurfaceTerrainClipContourRole::Hole,
            vertices,
            source_edges,
        )
    }

    fn terrain_cdt_stable_footprint_group_ids_for_terrain_clip_export(
        export: &RoadSurfaceTerrainClipExport,
    ) -> BTreeMap<usize, u64> {
        let mut shape_indices = export
            .loop_topologies
            .iter()
            .map(|topology| topology.shape_index)
            .collect::<Vec<_>>();
        shape_indices.sort_unstable();
        shape_indices.dedup();

        shape_indices
            .into_iter()
            .map(|shape_index| {
                (
                    shape_index,
                    Self::terrain_cdt_stable_footprint_group_id_for_terrain_clip_shape(
                        export,
                        shape_index,
                    ),
                )
            })
            .collect()
    }

    fn terrain_cdt_stable_footprint_group_id_for_terrain_clip_shape(
        export: &RoadSurfaceTerrainClipExport,
        shape_index: usize,
    ) -> u64 {
        let mut contours = export
            .loops
            .iter()
            .zip(export.loop_topologies.iter().copied())
            .filter(|(_, topology)| topology.shape_index == shape_index)
            .collect::<Vec<_>>();
        contours.sort_by_key(|(_, topology)| topology.contour_index);

        let mut hasher = TerrainClipStableHasher::new();
        hasher.write_str("terrain_clip_union_shape_v1");
        hasher.write_usize(contours.len());
        for (boundary_loop, topology) in contours {
            hasher.write_usize(topology.contour_index);
            hasher.write_usize(match topology.role {
                RoadSurfaceTerrainClipContourRole::Outer => 0,
                RoadSurfaceTerrainClipContourRole::Hole => 1,
            });
            hasher.write_usize(boundary_loop.points_world.len());
            for point in &boundary_loop.points_world {
                let key = SurfaceXzKey::from_godot_world_xz(*point);
                hasher.write_i64(key.x_key());
                hasher.write_i64(key.z_key());
                hasher.write_i64(SurfaceHeightMmKey::from_m_f32(point.y).as_i64());
            }
            hasher.write_usize(boundary_loop.source_edges.len());
            for edge in &boundary_loop.source_edges {
                hasher.write_u64(Self::terrain_cdt_stable_piece_id_for_source(edge.source));
            }
        }
        hasher.finish()
    }

    fn terrain_cdt_stable_piece_id_for_terrain_clip_loop(
        boundary_loop: &RoadSurfaceTerrainClipLoop,
        loop_index: usize,
    ) -> u64 {
        if boundary_loop.points_world.is_empty() {
            return terrain_cdt_usize_to_u64(loop_index);
        }

        let mut hasher = TerrainClipStableHasher::new();
        hasher.write_str("terrain_clip_union_loop_v1");
        hasher.write_usize(boundary_loop.points_world.len());
        for point in &boundary_loop.points_world {
            let key = SurfaceXzKey::from_godot_world_xz(*point);
            hasher.write_i64(key.x_key());
            hasher.write_i64(key.z_key());
            hasher.write_i64(SurfaceHeightMmKey::from_m_f32(point.y).as_i64());
        }
        hasher.write_usize(boundary_loop.source_edges.len());
        for edge in &boundary_loop.source_edges {
            let start_key = SurfaceXzKey::from_godot_world_xz(edge.start);
            let end_key = SurfaceXzKey::from_godot_world_xz(edge.end);
            hasher.write_i64(start_key.x_key());
            hasher.write_i64(start_key.z_key());
            hasher.write_i64(SurfaceHeightMmKey::from_m_f32(edge.start.y).as_i64());
            hasher.write_i64(end_key.x_key());
            hasher.write_i64(end_key.z_key());
            hasher.write_i64(SurfaceHeightMmKey::from_m_f32(edge.end.y).as_i64());
            hasher.write_u64(Self::terrain_cdt_stable_piece_id_for_source(edge.source));
        }
        hasher.finish()
    }

    fn terrain_cdt_stable_piece_id_for_source(source: RoadSurfaceEarthworkFaceSource) -> u64 {
        match source {
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { edge_idx, .. } => {
                terrain_cdt_usize_to_u64(edge_idx)
            }
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { node_id, .. } => {
                (1_u64 << 63) | u64::from(node_id)
            }
        }
    }

    fn terrain_cdt_boundary_source_from_surface(
        source: RoadSurfaceEarthworkFaceSource,
    ) -> TerrainCdtRoadBoundarySource {
        match source {
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
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
                edge_idx: terrain_cdt_usize_to_u64(edge_idx),
                edge_class: Self::terrain_cdt_edge_class(edge_class),
                support_policy: Self::terrain_cdt_support_policy(support_policy),
                source_band_index: terrain_cdt_usize_to_u32(owner.source_band_index),
                band_kind: Self::terrain_cdt_band_kind(owner.kind),
                role: Self::terrain_cdt_span_region_role(role),
                start_section_index: terrain_cdt_usize_to_u32(start_section_index),
                end_section_index: terrain_cdt_usize_to_u32(end_section_index),
                start_s_m,
                end_s_m,
            },
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id,
                kind,
                owner_kind,
                owner_index,
                boundary_source,
            } => TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id,
                node_kind: Self::terrain_cdt_node_piece_kind(kind),
                owner_kind: Self::terrain_cdt_band_kind(owner_kind),
                owner_index: terrain_cdt_usize_to_u32(owner_index),
                boundary_source: boundary_source
                    .map(Self::terrain_cdt_node_footprint_boundary_segment_source),
            },
        }
    }

    fn terrain_cdt_node_footprint_boundary_segment_source(
        source: NodeFootprintBoundarySegmentSource,
    ) -> TerrainCdtNodeFootprintBoundarySegmentSource {
        TerrainCdtNodeFootprintBoundarySegmentSource {
            start: Self::terrain_cdt_node_footprint_boundary_vertex_source(source.start),
            end: Self::terrain_cdt_node_footprint_boundary_vertex_source(source.end),
        }
    }

    fn terrain_cdt_node_footprint_boundary_vertex_source(
        source: NodeFootprintBoundaryVertexSource,
    ) -> TerrainCdtNodeFootprintBoundaryVertexSource {
        match source {
            NodeFootprintBoundaryVertexSource::Direct(direct) => {
                TerrainCdtNodeFootprintBoundaryVertexSource::Direct(
                    Self::terrain_cdt_node_footprint_boundary_direct_source(direct),
                )
            }
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start,
                owning_segment_end,
                height_mm,
            } => TerrainCdtNodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: Self::terrain_cdt_node_footprint_boundary_direct_source(
                    owning_segment_start,
                ),
                owning_segment_end: Self::terrain_cdt_node_footprint_boundary_direct_source(
                    owning_segment_end,
                ),
                height_mm,
            },
        }
    }

    fn terrain_cdt_node_footprint_boundary_direct_source(
        source: NodeFootprintBoundaryDirectSource,
    ) -> TerrainCdtNodeFootprintBoundaryDirectSource {
        TerrainCdtNodeFootprintBoundaryDirectSource {
            top_surface_source_index: terrain_cdt_usize_to_u64(source.top_surface_source_index),
            grade_authority_index: terrain_cdt_usize_to_u64(source.grade_authority_index),
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
        policy: RoadSurfaceEarthworkSupportPolicy,
    ) -> TerrainCdtEarthworkSupportPolicy {
        match policy {
            RoadSurfaceEarthworkSupportPolicy::StandardFullGroundedSpan => {
                TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan
            }
            RoadSurfaceEarthworkSupportPolicy::BridgeEndpointAbutments => {
                TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments
            }
            RoadSurfaceEarthworkSupportPolicy::TunnelVisiblePortals => {
                TerrainCdtEarthworkSupportPolicy::TunnelVisiblePortals
            }
        }
    }

    fn terrain_cdt_band_kind(kind: RoadSurfaceBandKind) -> TerrainCdtRoadBandKind {
        match kind {
            RoadSurfaceBandKind::Carriageway => TerrainCdtRoadBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder => TerrainCdtRoadBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Sidewalk => TerrainCdtRoadBandKind::Sidewalk,
            RoadSurfaceBandKind::Footpath => TerrainCdtRoadBandKind::Footpath,
            RoadSurfaceBandKind::Median => TerrainCdtRoadBandKind::Median,
            RoadSurfaceBandKind::Parking => TerrainCdtRoadBandKind::Parking,
            RoadSurfaceBandKind::CycleTrack => TerrainCdtRoadBandKind::CycleTrack,
            RoadSurfaceBandKind::TramReservation => TerrainCdtRoadBandKind::TramReservation,
        }
    }

    fn terrain_cdt_span_region_role(role: RoadSurfaceSpanRegionRole) -> TerrainCdtSpanRegionRole {
        match role {
            RoadSurfaceSpanRegionRole::Asphalt => TerrainCdtSpanRegionRole::Asphalt,
            RoadSurfaceSpanRegionRole::CurbOrShoulder => TerrainCdtSpanRegionRole::CurbOrShoulder,
            RoadSurfaceSpanRegionRole::NonRoad => TerrainCdtSpanRegionRole::NonRoad,
        }
    }

    fn terrain_cdt_node_piece_kind(
        kind: RoadSurfaceVisualNodePieceKind,
    ) -> TerrainCdtNodePieceKind {
        match kind {
            RoadSurfaceVisualNodePieceKind::Terminal => TerrainCdtNodePieceKind::Terminal,
            RoadSurfaceVisualNodePieceKind::Bend => TerrainCdtNodePieceKind::Bend,
            RoadSurfaceVisualNodePieceKind::JunctionN => TerrainCdtNodePieceKind::JunctionN,
        }
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

struct TerrainClipStableHasher {
    state: u64,
}

impl TerrainClipStableHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf29ce484222325,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= u64::from(byte);
            self.state = self.state.wrapping_mul(0x100000001b3);
        }
        self.state ^= 0xff;
        self.state = self.state.wrapping_mul(0x100000001b3);
    }

    fn write_i64(&mut self, value: i64) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(terrain_cdt_usize_to_u64(value));
    }

    fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    fn finish(self) -> u64 {
        self.state
    }
}

fn terrain_cdt_usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("terrain CDT export index must fit u32")
}

fn terrain_cdt_usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).expect("terrain CDT export index must fit u64")
}
