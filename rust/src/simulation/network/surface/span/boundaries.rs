// SPDX-License-Identifier: GPL-2.0-only

//! Span boundary and terrain-clip source construction from resolved owned regions.

use super::super::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource, RoadSurfaceSystem,
    RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge,
    RoadSurfaceVisualPolygon,
    backend::RoadVec3,
    earthwork::RoadSurfaceEarthworkGeometryError,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
    terrain_clip_edge_kind_for_band,
};
use super::RoadSurfaceSpanOwnedRegion;
use crate::simulation::network::types::EdgeClass;

impl RoadSurfaceSystem {
    pub(super) fn build_span_boundary_loops_from_regions(
        regions: &[RoadSurfaceSpanOwnedRegion],
        edge_class: EdgeClass,
    ) -> Result<
        (
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceTerrainClipLoop>,
        ),
        RoadSurfaceEarthworkGeometryError,
    > {
        let candidate_segments =
            Self::span_boundary_candidate_segments_from_regions(regions, edge_class);
        let boundary_segment_loops = Self::owned_region_boundary_segment_loops(candidate_segments)?;
        let mut outer_boundary_loops = Vec::with_capacity(boundary_segment_loops.len());
        let mut terrain_clip_boundary_loops = Vec::with_capacity(boundary_segment_loops.len());

        for segments in boundary_segment_loops {
            let points_world = Self::span_boundary_loop_points(&segments);
            let point_count = points_world.len();
            let Some(loop_polygon) = Self::make_boundary_loop_polygon(points_world) else {
                return Err(RoadSurfaceEarthworkGeometryError::DegenerateBoundaryLoop {
                    point_count,
                });
            };

            let mut source_edges = segments
                .into_iter()
                .map(Self::terrain_clip_source_edge_from_span_boundary_segment)
                .collect::<Vec<_>>();
            Self::canonicalize_span_terrain_clip_source_edges(
                &mut source_edges,
                &loop_polygon.points_world,
            );
            terrain_clip_boundary_loops.push(RoadSurfaceTerrainClipLoop {
                points_world: loop_polygon.points_world.clone(),
                source_edges,
            });
            outer_boundary_loops.push(loop_polygon);
        }

        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Ok((outer_boundary_loops, terrain_clip_boundary_loops))
    }

    fn span_boundary_loop_points(
        segments: &[RoadSurfaceEarthworkBoundarySegment],
    ) -> Vec<RoadVec3> {
        segments.iter().map(|segment| segment.inner_start).collect()
    }

    fn span_boundary_candidate_segments_from_regions(
        regions: &[RoadSurfaceSpanOwnedRegion],
        edge_class: EdgeClass,
    ) -> Vec<RoadSurfaceEarthworkBoundarySegment> {
        let mut segments = Vec::new();
        for region in regions {
            let points = &region.polygon.points_world;
            if points.len() < 3 {
                continue;
            }
            for index in 0..points.len() {
                let inner_start = points[index];
                let inner_end = points[(index + 1) % points.len()];
                segments.push(RoadSurfaceEarthworkBoundarySegment {
                    inner_start,
                    inner_end,
                    source: Self::span_boundary_segment_source(
                        region,
                        edge_class,
                        inner_start,
                        inner_end,
                    ),
                });
            }
        }
        segments
    }

    fn span_boundary_segment_source(
        region: &RoadSurfaceSpanOwnedRegion,
        edge_class: EdgeClass,
        start: RoadVec3,
        end: RoadVec3,
    ) -> RoadSurfaceEarthworkFaceSource {
        let [start_left, end_left, end_right, start_right] = region.source_corners_world;
        if Self::span_boundary_segment_matches_source_edge(start, end, end_left, end_right) {
            return region.handoff_boundary_source(
                edge_class,
                region.end_section_index,
                region.end_s_m,
            );
        }
        if Self::span_boundary_segment_matches_source_edge(start, end, start_right, start_left) {
            return region.handoff_boundary_source(
                edge_class,
                region.start_section_index,
                region.start_s_m,
            );
        }
        region.support_boundary_source(edge_class)
    }

    fn span_boundary_segment_matches_source_edge(
        start: RoadVec3,
        end: RoadVec3,
        source_start: RoadVec3,
        source_end: RoadVec3,
    ) -> bool {
        (Self::span_points_share_canonical_position(start, source_start)
            && Self::span_points_share_canonical_position(end, source_end))
            || (Self::span_points_share_canonical_position(start, source_end)
                && Self::span_points_share_canonical_position(end, source_start))
    }

    fn terrain_clip_source_edge_from_span_boundary_segment(
        segment: RoadSurfaceEarthworkBoundarySegment,
    ) -> RoadSurfaceTerrainClipSourceEdge {
        RoadSurfaceTerrainClipSourceEdge {
            start: segment.inner_start,
            end: segment.inner_end,
            kind: Self::span_terrain_clip_edge_kind_for_source(segment.source),
            source: segment.source,
        }
    }

    fn span_terrain_clip_edge_kind_for_source(
        source: RoadSurfaceEarthworkFaceSource,
    ) -> RoadSurfaceTerrainClipEdgeKind {
        match source {
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                start_section_index,
                end_section_index,
                ..
            } if start_section_index == end_section_index => {
                RoadSurfaceTerrainClipEdgeKind::SpanHandoff
            }
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { owner, .. } => {
                terrain_clip_edge_kind_for_band(owner.kind)
            }
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { .. }
            | RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff { .. } => {
                unreachable!("span boundary extraction only emits span support boundary sources")
            }
        }
    }

    fn canonicalize_span_terrain_clip_source_edges(
        source_edges: &mut [RoadSurfaceTerrainClipSourceEdge],
        loop_points: &[RoadVec3],
    ) {
        for edge in source_edges {
            if let Some(point) = Self::matching_canonical_span_loop_point(edge.start, loop_points) {
                edge.start = point;
            }
            if let Some(point) = Self::matching_canonical_span_loop_point(edge.end, loop_points) {
                edge.end = point;
            }
        }
    }

    fn matching_canonical_span_loop_point(
        point: RoadVec3,
        loop_points: &[RoadVec3],
    ) -> Option<RoadVec3> {
        loop_points
            .iter()
            .copied()
            .find(|candidate| Self::span_points_share_canonical_position(*candidate, point))
    }

    fn span_points_share_canonical_position(a: RoadVec3, b: RoadVec3) -> bool {
        SurfaceXzKey::from_world_xz(a) == SurfaceXzKey::from_world_xz(b)
            && SurfaceHeightMmKey::from_m_f64(a.y) == SurfaceHeightMmKey::from_m_f64(b.y)
    }
}
