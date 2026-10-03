// SPDX-License-Identifier: GPL-2.0-only

//! Span boundary and terrain-clip source construction from resolved owned regions.

use super::super::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource, RoadSurfaceSection,
    RoadSurfaceSystem,
    RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge,
    RoadSurfaceVisualPolygon,
    backend::RoadVec3,
    earthwork::RoadSurfaceEarthworkGeometryError,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
    terrain_clip_edge_kind_for_band,
};
use super::RoadSurfaceSpanOwnedRegion;
use crate::simulation::network::types::EdgeClass;

// Bound exported terrain footprints along the existing section station lattice. A patch must
// not drag a kilometre-scale span into the micrometre i32 overlay. No section is resampled.
const TERRAIN_CLIP_RUN_M: f32 = 64.0;

impl RoadSurfaceSystem {
    pub(super) fn build_span_boundary_loops_from_regions(
        sections: &[RoadSurfaceSection],
        regions: &[RoadSurfaceSpanOwnedRegion],
        edge_class: EdgeClass,
    ) -> Result<
        (
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceTerrainClipLoop>,
        ),
        RoadSurfaceEarthworkGeometryError,
    > {
        let (outer_boundary_loops, mut terrain_clip_boundary_loops) =
            Self::build_span_boundary_run(sections, regions, edge_class)?;
        let run_key = |region: &RoadSurfaceSpanOwnedRegion| {
            (region.start_s_m / TERRAIN_CLIP_RUN_M).floor() as i64
        };
        let mut runs = regions.chunk_by(|a, b| run_key(a) == run_key(b));
        let Some(first) = runs.next() else {
            return Ok((outer_boundary_loops, terrain_clip_boundary_loops));
        };
        if first.len() != regions.len() {
            // Preserve the full render/earthwork outline; only terrain-query footprints split.
            // Existing handoff sources close each run and cancel at the subsequent union.
            // One additional linear boundary pass, O(regions) retained source data.
            terrain_clip_boundary_loops.clear();
            for run in std::iter::once(first).chain(runs) {
                terrain_clip_boundary_loops
                    .extend(Self::build_span_boundary_run(sections, run, edge_class)?.1);
            }
            Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        }
        Ok((outer_boundary_loops, terrain_clip_boundary_loops))
    }

    fn build_span_boundary_run(
        sections: &[RoadSurfaceSection],
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
            Self::span_boundary_candidate_segments_from_regions(sections, regions, edge_class);
        let boundary_segment_loops = Self::owned_region_boundary_segment_loops(candidate_segments)?;
        let mut outer_boundary_loops = Vec::with_capacity(boundary_segment_loops.len());
        let mut terrain_clip_boundary_loops = Vec::with_capacity(boundary_segment_loops.len());

        for segments in boundary_segment_loops {
            let points_world = Self::span_boundary_loop_points(&segments);
            let point_count = points_world.len();
            let Some(loop_polygon) = Self::make_boundary_loop_polygon(points_world) else {
                crate::debug_log!("road", "span_boundary_loop_invalid segments={:?}", segments);
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
        sections: &[RoadSurfaceSection],
        regions: &[RoadSurfaceSpanOwnedRegion],
        edge_class: EdgeClass,
    ) -> Vec<RoadSurfaceEarthworkBoundarySegment> {
        let mut segments = Vec::new();
        for region in regions {
            let corners = region.corners(sections);
            let quad = region.quad(sections);
            let points = quad.points();
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
                        corners,
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
        corners: [RoadVec3; 4],
        edge_class: EdgeClass,
        start: RoadVec3,
        end: RoadVec3,
    ) -> RoadSurfaceEarthworkFaceSource {
        let [start_left, end_left, end_right, start_right] = corners;
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
        // Quantize each loop point once. The original loop index breaks equal-key ties so
        // the first matching coordinate remains authoritative, including its exact height.
        // O(P log P + E log P), replacing an O(E * P) scan for P points and E source edges.
        let mut keyed_points: Vec<_> = loop_points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                (
                    (
                        SurfaceXzKey::from_world_xz(*point),
                        SurfaceHeightMmKey::from_m_f64(point.y),
                    ),
                    index,
                )
            })
            .collect();
        keyed_points.sort_unstable();
        let matching_point = |point: RoadVec3| {
            let key = (
                SurfaceXzKey::from_world_xz(point),
                SurfaceHeightMmKey::from_m_f64(point.y),
            );
            let index = keyed_points.partition_point(|entry| entry.0 < key);
            keyed_points
                .get(index)
                .filter(|entry| entry.0 == key)
                .map(|entry| loop_points[entry.1])
        };
        for edge in source_edges {
            if let Some(point) = matching_point(edge.start) {
                edge.start = point;
            }
            if let Some(point) = matching_point(edge.end) {
                edge.end = point;
            }
        }
    }

    fn span_points_share_canonical_position(a: RoadVec3, b: RoadVec3) -> bool {
        SurfaceXzKey::from_world_xz(a) == SurfaceXzKey::from_world_xz(b)
            && SurfaceHeightMmKey::from_m_f64(a.y) == SurfaceHeightMmKey::from_m_f64(b.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::{
        RoadSurfaceBandKind, RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSpanBandOwner,
        RoadSurfaceSpanRegionRole,
    };

    #[test]
    fn clip_source_endpoints_keep_first_canonical_match_and_unmatched_coordinates() {
        let first = RoadVec3::new(20_000.000_000_1, 10.0001, -20_000.0);
        let duplicate = RoadVec3::new(20_000.000_000_2, 10.0002, -20_000.0);
        let higher = RoadVec3::new(20_000.000_000_1, 10.001, -20_000.0);
        let missing = RoadVec3::new(20_001.0, 10.0, -20_000.0);
        assert!(RoadSurfaceSystem::span_points_share_canonical_position(
            first, duplicate
        ));
        assert!(!RoadSurfaceSystem::span_points_share_canonical_position(
            first, higher
        ));
        let source = RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx: 7,
            edge_class: EdgeClass::Standard,
            support_policy: RoadSurfaceEarthworkSupportPolicy::StandardFullGroundedSpan,
            owner: RoadSurfaceSpanBandOwner {
                source_band_index: 2,
                kind: RoadSurfaceBandKind::Sidewalk,
            },
            role: RoadSurfaceSpanRegionRole::NonRoad,
            start_section_index: 3,
            end_section_index: 4,
            start_s_m: 12.0,
            end_s_m: 16.0,
        };
        for (points, expected_first) in [
            ([higher, first, duplicate], first),
            ([duplicate, first, higher], duplicate),
        ] {
            let edge = RoadSurfaceTerrainClipSourceEdge {
                start: duplicate,
                end: higher,
                kind: RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
                source,
            };
            let unmatched = RoadSurfaceTerrainClipSourceEdge {
                start: missing,
                end: missing,
                ..edge
            };
            let mut edges = [edge, unmatched];
            RoadSurfaceSystem::canonicalize_span_terrain_clip_source_edges(&mut edges, &points);
            assert_eq!(
                edges,
                [
                    RoadSurfaceTerrainClipSourceEdge {
                        start: expected_first,
                        ..edge
                    },
                    unmatched
                ]
            );
        }
    }
}
