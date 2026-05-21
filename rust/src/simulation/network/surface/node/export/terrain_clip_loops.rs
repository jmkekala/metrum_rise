//! Terrain clip loop export from source-backed earthwork boundary segments.

use crate::simulation::network::surface::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipSourceEdge, terrain_clip_edge_kind_for_band,
};

impl RoadSurfaceSystem {
    pub(super) fn terrain_clip_boundary_loops_from_earthwork_segments(
        segment_loops: &[Vec<RoadSurfaceEarthworkBoundarySegment>],
    ) -> Vec<RoadSurfaceTerrainClipLoop> {
        let mut loops = Vec::new();
        for segment_loop in segment_loops {
            if segment_loop.len() < 3 {
                continue;
            }
            let points = segment_loop
                .iter()
                .map(|segment| segment.inner_start)
                .collect::<Vec<_>>();
            if Self::signed_polygon_area_xz(&points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                continue;
            }
            let source_edges = segment_loop
                .iter()
                .copied()
                .map(|segment| RoadSurfaceTerrainClipSourceEdge {
                    start: segment.inner_start,
                    end: segment.inner_end,
                    kind: match segment.source {
                        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                            owner_kind,
                            ..
                        } => terrain_clip_edge_kind_for_band(owner_kind),
                        RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. } => {
                            RoadSurfaceTerrainClipEdgeKind::FootprintBoundary
                        }
                    },
                    source: segment.source,
                })
                .collect();
            loops.push(RoadSurfaceTerrainClipLoop {
                points_world: points,
                source_edges,
            });
        }
        loops
    }
}
