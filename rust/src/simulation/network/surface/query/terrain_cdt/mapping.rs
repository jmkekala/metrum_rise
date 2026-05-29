//! Road-surface terrain clip provenance mapping into terrain CDT DTOs.

use super::stable_ids::{terrain_cdt_usize_to_u32, terrain_cdt_usize_to_u64};
use super::*;

impl RoadSurfaceSystem {
    pub(super) fn terrain_cdt_boundary_source_from_surface(
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
            RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                node_id,
                kind,
                owner_kind,
                owner_index_a,
                owner_index_b,
                boundary_source,
            } => TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
                node_id,
                node_kind: Self::terrain_cdt_node_piece_kind(kind),
                owner_kind: Self::terrain_cdt_band_kind(owner_kind),
                owner_index_a: terrain_cdt_usize_to_u32(owner_index_a),
                owner_index_b: terrain_cdt_usize_to_u32(owner_index_b),
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
            NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm } => {
                TerrainCdtNodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key,
                    z_key,
                    y_mm,
                }
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
}
