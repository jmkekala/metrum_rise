//! Explicit visual span-piece compilation and span mouth profile construction.

use super::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, RoadSurfaceBandKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace,
    RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSection, RoadSurfaceSystem,
    RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge,
    RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    band_semantics::band_kind_sort_key,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
    terrain_clip_edge_kind_for_band,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

// Avoid resolved region construction between adjacent bands whose widths have collapsed together.
const SPAN_REGION_MIN_BAND_WIDTH_M: f32 = 0.05;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceSpanRegionRole {
    Asphalt,
    CurbOrShoulder,
    NonRoad,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RoadSurfaceSpanBandOwner {
    pub(crate) source_band_index: usize,
    pub(crate) kind: RoadSurfaceBandKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceSpanOwnedRegion {
    pub(crate) edge_idx: usize,
    pub(crate) owner: RoadSurfaceSpanBandOwner,
    pub(crate) role: RoadSurfaceSpanRegionRole,
    pub(crate) start_section_index: usize,
    pub(crate) end_section_index: usize,
    pub(crate) start_s_m: f32,
    pub(crate) end_s_m: f32,
    pub(crate) polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadSurfaceSpanRaisedStepSource {
    pub(crate) lower_owner: RoadSurfaceSpanBandOwner,
    pub(crate) raised_owner: RoadSurfaceSpanBandOwner,
    pub(crate) start_section_index: usize,
    pub(crate) end_section_index: usize,
    pub(crate) start_s_m: f32,
    pub(crate) end_s_m: f32,
    pub(crate) start_lower_world: Vector3,
    pub(crate) start_raised_world: Vector3,
    pub(crate) end_lower_world: Vector3,
    pub(crate) end_raised_world: Vector3,
}

/// Explicit visual span piece compiled from one edge corridor.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualSpanPiece {
    /// Owning edge id.
    pub edge_idx: usize,
    /// Outer piece-owned boundaries used for debug, surface chunk bounds, and terrain clipping.
    pub outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    /// Explicit asphalt-owned polygons for the span piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit curb / shoulder-owned polygons for the span piece.
    pub curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit vertical faces at raised owner-pair material contacts.
    pub raised_step_face_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) span_raised_step_sources: Vec<RoadSurfaceSpanRaisedStepSource>,
    /// Explicit sidewalk-owned polygons for the span piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) span_owned_regions: Vec<RoadSurfaceSpanOwnedRegion>,
    pub(crate) edge_class: EdgeClass,
    pub(crate) start_mouth_profile: Option<IncidentMouthProfile>,
    pub(crate) end_mouth_profile: Option<IncidentMouthProfile>,
    pub(crate) span_earthwork_support_regions: Vec<RoadSurfaceSpanOwnedRegion>,
    pub(crate) earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SpanResolvedRegionSet {
    regions: Vec<RoadSurfaceSpanOwnedRegion>,
    raised_step_constraints: Vec<SpanRaisedStepConstraint>,
    outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SpanRenderRegionBuckets {
    road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanRaisedStepConstraint {
    lower_owner: RoadSurfaceSpanBandOwner,
    raised_owner: RoadSurfaceSpanBandOwner,
    start_section_index: usize,
    end_section_index: usize,
    start_s_m: f32,
    end_s_m: f32,
    start: SpanRaisedStepSample,
    end: SpanRaisedStepSample,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanRaisedStepSample {
    lower_world: Vector3,
    raised_world: Vector3,
    lower_direction: Vector3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanResolvedRaisedStepSample {
    lower_owner: RoadSurfaceSpanBandOwner,
    raised_owner: RoadSurfaceSpanBandOwner,
    sample: SpanRaisedStepSample,
}

impl RoadSurfaceSpanRegionRole {
    fn from_band_pair(start_kind: RoadSurfaceBandKind, end_kind: RoadSurfaceBandKind) -> Self {
        match (start_kind, end_kind) {
            (RoadSurfaceBandKind::Carriageway, RoadSurfaceBandKind::Carriageway) => Self::Asphalt,
            (RoadSurfaceBandKind::CurbOrShoulder, RoadSurfaceBandKind::CurbOrShoulder) => {
                Self::CurbOrShoulder
            }
            _ => Self::NonRoad,
        }
    }

    pub(crate) fn sort_key(self) -> u8 {
        match self {
            Self::Asphalt => 0,
            Self::CurbOrShoulder => 1,
            Self::NonRoad => 2,
        }
    }
}

impl RoadSurfaceSpanBandOwner {
    pub(crate) fn sort_key(self) -> (u8, usize) {
        (band_kind_sort_key(self.kind), self.source_band_index)
    }
}

impl SpanRenderRegionBuckets {
    fn is_empty(&self) -> bool {
        self.road_surface_polygons.is_empty()
            && self.curb_surface_polygons.is_empty()
            && self.sidewalk_surface_polygons.is_empty()
    }

    fn sort(&mut self) {
        RoadSurfaceSystem::sort_visual_polygons(&mut self.road_surface_polygons);
        RoadSurfaceSystem::sort_visual_polygons(&mut self.curb_surface_polygons);
        RoadSurfaceSystem::sort_visual_polygons(&mut self.sidewalk_surface_polygons);
    }
}

impl RoadSurfaceSystem {
    pub(super) fn compile_visual_span_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
    ) -> Option<RoadSurfaceVisualSpanPiece> {
        if edge_idx >= graph.edge_count() {
            return None;
        }
        let edge = graph.edge(edge_idx);
        let sections = self.compiled_sections.get(&edge_idx)?;
        let visible_ranges =
            self.visible_section_ranges_for_edge(graph, terrain, edge_idx, sections);
        let mut visible_regions =
            self.resolve_span_regions_for_ranges(sections, &visible_ranges, edge.class);
        Self::sort_span_owned_regions(&mut visible_regions.regions);
        let mut render_buckets =
            Self::span_render_region_buckets_from_owned_regions(&visible_regions.regions);
        let mut raised_step_faces =
            Self::span_raised_step_faces_from_constraints(&visible_regions.raised_step_constraints);
        Self::sort_span_raised_step_faces(&mut raised_step_faces);
        let (raised_step_face_polygons, span_raised_step_sources): (
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceSpanRaisedStepSource>,
        ) = raised_step_faces.into_iter().unzip();

        if render_buckets.is_empty() {
            return None;
        }

        render_buckets.sort();
        let outer_boundary_loops = std::mem::take(&mut visible_regions.outer_boundary_loops);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        let terrain_clip_boundary_loops =
            std::mem::take(&mut visible_regions.terrain_clip_boundary_loops);

        let earthwork_ranges = self.earthwork_section_ranges_for_edge(edge, sections, terrain);
        let mut clearance_regions =
            self.resolve_span_regions_for_ranges(sections, &earthwork_ranges, edge.class);
        Self::sort_span_owned_regions(&mut clearance_regions.regions);
        let span_earthwork_support_regions = clearance_regions.regions;
        let earthwork_boundary_segments =
            Self::span_earthwork_boundary_segment_loops_from_support_regions(
                &span_earthwork_support_regions,
                edge.class,
            );
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_segments(
                &earthwork_boundary_segments,
                terrain,
                None,
            );

        let start_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::Start);
        let end_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::End);

        Some(RoadSurfaceVisualSpanPiece {
            edge_idx,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons: render_buckets.road_surface_polygons,
            curb_surface_polygons: render_buckets.curb_surface_polygons,
            raised_step_face_polygons,
            span_raised_step_sources,
            sidewalk_surface_polygons: render_buckets.sidewalk_surface_polygons,
            span_owned_regions: visible_regions.regions,
            edge_class: edge.class,
            start_mouth_profile,
            end_mouth_profile,
            span_earthwork_support_regions,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }

    fn resolve_span_regions_for_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
        edge_class: crate::simulation::network::types::EdgeClass,
    ) -> SpanResolvedRegionSet {
        let (outer_boundary_loops, terrain_clip_boundary_loops) =
            Self::build_span_boundary_loops(sections, ranges, edge_class);
        let mut resolved = SpanResolvedRegionSet {
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            ..SpanResolvedRegionSet::default()
        };

        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            for (segment_offset, pair) in sections[start_index..=end_index].windows(2).enumerate() {
                let start_section_index = start_index + segment_offset;
                let end_section_index = start_section_index + 1;
                if pair[0].bands.len() != pair[1].bands.len() {
                    continue;
                }
                for (band_index, (band_a, band_b)) in
                    pair[0].bands.iter().zip(&pair[1].bands).enumerate()
                {
                    let width_a = (band_a.lateral_end_m - band_a.lateral_start_m).abs();
                    let width_b = (band_b.lateral_end_m - band_b.lateral_start_m).abs();
                    if width_a <= SPAN_REGION_MIN_BAND_WIDTH_M
                        && width_b <= SPAN_REGION_MIN_BAND_WIDTH_M
                    {
                        continue;
                    }

                    let Some(polygon) = Self::make_visual_strip_polygon(vec![
                        self.section_boundary_world_point(
                            &pair[0],
                            band_a.lateral_start_m,
                            band_a.height_start_m,
                        ),
                        self.section_boundary_world_point(
                            &pair[1],
                            band_b.lateral_start_m,
                            band_b.height_start_m,
                        ),
                        self.section_boundary_world_point(
                            &pair[1],
                            band_b.lateral_end_m,
                            band_b.height_end_m,
                        ),
                        self.section_boundary_world_point(
                            &pair[0],
                            band_a.lateral_end_m,
                            band_a.height_end_m,
                        ),
                    ]) else {
                        continue;
                    };

                    resolved.regions.push(RoadSurfaceSpanOwnedRegion {
                        edge_idx: pair[0].edge_idx,
                        owner: RoadSurfaceSpanBandOwner {
                            source_band_index: band_index,
                            kind: band_a.kind,
                        },
                        role: RoadSurfaceSpanRegionRole::from_band_pair(band_a.kind, band_b.kind),
                        start_section_index,
                        end_section_index,
                        start_s_m: pair[0].s_m,
                        end_s_m: pair[1].s_m,
                        polygon,
                    });
                }
                resolved.raised_step_constraints.extend(
                    Self::span_raised_step_constraints_for_resolved_segment(
                        pair,
                        start_section_index,
                        end_section_index,
                    ),
                );
            }
        }

        resolved
    }

    fn span_render_region_buckets_from_owned_regions(
        regions: &[RoadSurfaceSpanOwnedRegion],
    ) -> SpanRenderRegionBuckets {
        let mut buckets = SpanRenderRegionBuckets::default();

        for region in regions {
            match region.role {
                RoadSurfaceSpanRegionRole::Asphalt => {
                    buckets.road_surface_polygons.push(region.polygon.clone());
                }
                RoadSurfaceSpanRegionRole::CurbOrShoulder => {
                    buckets.curb_surface_polygons.push(region.polygon.clone());
                }
                RoadSurfaceSpanRegionRole::NonRoad => {
                    buckets
                        .sidewalk_surface_polygons
                        .push(region.polygon.clone());
                }
            }
        }

        buckets
    }

    fn sort_span_owned_regions(regions: &mut [RoadSurfaceSpanOwnedRegion]) {
        regions.sort_by(|a, b| {
            a.edge_idx
                .cmp(&b.edge_idx)
                .then(a.start_section_index.cmp(&b.start_section_index))
                .then(a.end_section_index.cmp(&b.end_section_index))
                .then(a.start_s_m.total_cmp(&b.start_s_m))
                .then(a.end_s_m.total_cmp(&b.end_s_m))
                .then(a.role.sort_key().cmp(&b.role.sort_key()))
                .then(a.owner.sort_key().cmp(&b.owner.sort_key()))
                .then_with(|| Self::visual_polygon_ordering(&a.polygon, &b.polygon))
        });
    }

    fn sort_span_raised_step_faces(
        faces: &mut [(RoadSurfaceVisualPolygon, RoadSurfaceSpanRaisedStepSource)],
    ) {
        faces.sort_by(|(polygon_a, source_a), (polygon_b, source_b)| {
            source_a
                .lower_owner
                .sort_key()
                .cmp(&source_b.lower_owner.sort_key())
                .then(
                    source_a
                        .raised_owner
                        .sort_key()
                        .cmp(&source_b.raised_owner.sort_key()),
                )
                .then(
                    source_a
                        .start_section_index
                        .cmp(&source_b.start_section_index),
                )
                .then(source_a.end_section_index.cmp(&source_b.end_section_index))
                .then(source_a.start_s_m.total_cmp(&source_b.start_s_m))
                .then(source_a.end_s_m.total_cmp(&source_b.end_s_m))
                .then_with(|| Self::visual_polygon_ordering(polygon_a, polygon_b))
        });
    }

    fn span_raised_step_constraints_for_resolved_segment(
        pair: &[RoadSurfaceSection],
        start_section_index: usize,
        end_section_index: usize,
    ) -> Vec<SpanRaisedStepConstraint> {
        if pair.len() != 2 || pair[0].bands.len() != pair[1].bands.len() {
            return Vec::new();
        }

        let mut constraints = Vec::new();
        for boundary_index in 0..pair[0].bands.len().saturating_sub(1) {
            let Some(start) = Self::span_raised_step_sample(&pair[0], boundary_index) else {
                continue;
            };
            let Some(end) = Self::span_raised_step_sample(&pair[1], boundary_index) else {
                continue;
            };
            if start.lower_owner != end.lower_owner || start.raised_owner != end.raised_owner {
                continue;
            }
            constraints.push(SpanRaisedStepConstraint {
                lower_owner: start.lower_owner,
                raised_owner: start.raised_owner,
                start_section_index,
                end_section_index,
                start_s_m: pair[0].s_m,
                end_s_m: pair[1].s_m,
                start: start.sample,
                end: end.sample,
            });
        }
        constraints
    }

    fn span_raised_step_sample(
        section: &RoadSurfaceSection,
        boundary_index: usize,
    ) -> Option<SpanResolvedRaisedStepSample> {
        let lower_index = boundary_index;
        let upper_index = boundary_index + 1;
        let left = section.bands.get(lower_index)?;
        let right = section.bands.get(upper_index)?;
        if left.lateral_end_m != right.lateral_start_m {
            return None;
        }

        let boundary_lateral_m = left.lateral_end_m;
        let left_mid_lateral_m = (left.lateral_start_m + left.lateral_end_m) * 0.5;
        let right_mid_lateral_m = (right.lateral_start_m + right.lateral_end_m) * 0.5;
        if left.kind == right.kind {
            return None;
        }
        if (left.height_end_m - right.height_start_m).abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let left_owner = RoadSurfaceSpanBandOwner {
            source_band_index: lower_index,
            kind: left.kind,
        };
        let right_owner = RoadSurfaceSpanBandOwner {
            source_band_index: upper_index,
            kind: right.kind,
        };
        let (lower_owner, raised_owner, lower_height_m, raised_height_m, lower_mid_lateral_m) =
            if left.height_end_m < right.height_start_m {
                (
                    left_owner,
                    right_owner,
                    left.height_end_m,
                    right.height_start_m,
                    left_mid_lateral_m,
                )
            } else {
                (
                    right_owner,
                    left_owner,
                    right.height_start_m,
                    left.height_end_m,
                    right_mid_lateral_m,
                )
            };
        if raised_height_m <= lower_height_m {
            return None;
        }

        let lower_world =
            Self::section_boundary_world_point_static(section, boundary_lateral_m, lower_height_m);
        let raised_world =
            Self::section_boundary_world_point_static(section, boundary_lateral_m, raised_height_m);
        let lower_direction_xz = section.lateral_xz * (lower_mid_lateral_m - boundary_lateral_m);

        Some(SpanResolvedRaisedStepSample {
            lower_owner,
            raised_owner,
            sample: SpanRaisedStepSample {
                lower_world,
                raised_world,
                lower_direction: Vector3::new(lower_direction_xz.x, 0.0, lower_direction_xz.y),
            },
        })
    }

    fn span_raised_step_faces_from_constraints(
        constraints: &[SpanRaisedStepConstraint],
    ) -> Vec<(RoadSurfaceVisualPolygon, RoadSurfaceSpanRaisedStepSource)> {
        constraints
            .iter()
            .filter_map(Self::span_raised_step_face_from_constraint)
            .collect()
    }

    fn span_raised_step_face_from_constraint(
        constraint: &SpanRaisedStepConstraint,
    ) -> Option<(RoadSurfaceVisualPolygon, RoadSurfaceSpanRaisedStepSource)> {
        let mut points = [
            constraint.start.raised_world,
            constraint.start.lower_world,
            constraint.end.lower_world,
            constraint.end.raised_world,
        ];
        let lower_direction = constraint.start.lower_direction + constraint.end.lower_direction;
        let face_normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if face_normal.dot(lower_direction) > 0.0 {
            points = [points[3], points[2], points[1], points[0]];
        }

        let polygon = Self::make_vertical_quad_polygon(points)?;
        let source = RoadSurfaceSpanRaisedStepSource {
            lower_owner: constraint.lower_owner,
            raised_owner: constraint.raised_owner,
            start_section_index: constraint.start_section_index,
            end_section_index: constraint.end_section_index,
            start_s_m: constraint.start_s_m,
            end_s_m: constraint.end_s_m,
            start_lower_world: constraint.start.lower_world,
            start_raised_world: constraint.start.raised_world,
            end_lower_world: constraint.end.lower_world,
            end_raised_world: constraint.end.raised_world,
        };
        Some((polygon, source))
    }

    fn build_span_boundary_loops(
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
        edge_class: crate::simulation::network::types::EdgeClass,
    ) -> (
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceTerrainClipLoop>,
    ) {
        let mut outer_boundary_loops = Vec::new();
        let mut terrain_clip_boundary_loops = Vec::new();
        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            let mut left_points = Vec::new();
            let mut right_points = Vec::new();
            let mut left_kind = RoadSurfaceTerrainClipEdgeKind::FootprintBoundary;
            let mut right_kind = RoadSurfaceTerrainClipEdgeKind::FootprintBoundary;
            for section in &sections[start_index..=end_index] {
                let Some((left_point, right_point)) = Self::section_outer_boundary_pair(section)
                else {
                    continue;
                };
                let Some((section_left_kind, section_right_kind)) =
                    Self::section_outer_boundary_edge_kinds(section)
                else {
                    continue;
                };
                left_kind = section_left_kind;
                right_kind = section_right_kind;
                left_points.push(left_point);
                right_points.push(right_point);
            }
            if left_points.len() < 2 || right_points.len() < 2 {
                continue;
            }

            let mut loop_points = left_points.clone();
            let mut reversed_right_points = right_points.clone();
            reversed_right_points.reverse();
            loop_points.extend(reversed_right_points);
            let Some(loop_polygon) = Self::make_boundary_loop_polygon(loop_points) else {
                continue;
            };
            outer_boundary_loops.push(loop_polygon.clone());

            let mut source_edges =
                Vec::with_capacity(left_points.len() + right_points.len().saturating_sub(2) + 2);
            for (offset, pair) in left_points.windows(2).enumerate() {
                let section_index = start_index + offset;
                let source = Self::span_terrain_clip_source_for_outer_band_edge(
                    &sections[section_index],
                    &sections[section_index + 1],
                    edge_class,
                    0,
                    section_index,
                    section_index + 1,
                );
                source_edges.push(RoadSurfaceTerrainClipSourceEdge {
                    start: pair[0],
                    end: pair[1],
                    kind: left_kind,
                    source,
                });
            }
            let last_left = *left_points.last().unwrap();
            let last_right = *right_points.last().unwrap();
            Self::push_span_handoff_terrain_clip_source_edges(
                &sections[end_index],
                edge_class,
                last_left,
                last_right,
                end_index,
                &mut source_edges,
            );
            for (offset, pair) in right_points.windows(2).enumerate() {
                let section_index = start_index + offset;
                let source_band_index = sections[section_index].bands.len().saturating_sub(1);
                let source = Self::span_terrain_clip_source_for_outer_band_edge(
                    &sections[section_index],
                    &sections[section_index + 1],
                    edge_class,
                    source_band_index,
                    section_index,
                    section_index + 1,
                );
                source_edges.push(RoadSurfaceTerrainClipSourceEdge {
                    start: pair[1],
                    end: pair[0],
                    kind: right_kind,
                    source,
                });
            }
            Self::push_span_handoff_terrain_clip_source_edges(
                &sections[start_index],
                edge_class,
                right_points[0],
                left_points[0],
                start_index,
                &mut source_edges,
            );
            canonicalize_span_terrain_clip_source_edges(
                &mut source_edges,
                &loop_polygon.points_world,
            );
            terrain_clip_boundary_loops.push(RoadSurfaceTerrainClipLoop {
                points_world: loop_polygon.points_world,
                source_edges,
            });
        }
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        (outer_boundary_loops, terrain_clip_boundary_loops)
    }

    fn span_terrain_clip_source_for_outer_band_edge(
        start_section: &RoadSurfaceSection,
        end_section: &RoadSurfaceSection,
        edge_class: crate::simulation::network::types::EdgeClass,
        source_band_index: usize,
        start_section_index: usize,
        end_section_index: usize,
    ) -> RoadSurfaceEarthworkFaceSource {
        let start_band = &start_section.bands[source_band_index.min(start_section.bands.len() - 1)];
        let end_band = &end_section.bands[source_band_index.min(end_section.bands.len() - 1)];
        RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx: start_section.edge_idx,
            edge_class,
            support_policy: RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class),
            owner: RoadSurfaceSpanBandOwner {
                source_band_index,
                kind: start_band.kind,
            },
            role: RoadSurfaceSpanRegionRole::from_band_pair(start_band.kind, end_band.kind),
            start_section_index,
            end_section_index,
            start_s_m: start_section.s_m,
            end_s_m: end_section.s_m,
        }
    }

    fn push_span_handoff_terrain_clip_source_edges(
        section: &RoadSurfaceSection,
        edge_class: crate::simulation::network::types::EdgeClass,
        start: Vector3,
        end: Vector3,
        section_index: usize,
        source_edges: &mut Vec<RoadSurfaceTerrainClipSourceEdge>,
    ) {
        if section.bands.is_empty() {
            return;
        }
        let forward = Vector3::new(end.x - start.x, end.y - start.y, end.z - start.z);
        let source_points = Self::section_boundary_points_for_handoff(section);
        for (band_index, pair) in source_points.windows(2).enumerate() {
            let mut source_start = pair[0];
            let mut source_end = pair[1];
            let source_forward = Vector3::new(
                source_end.x - source_start.x,
                source_end.y - source_start.y,
                source_end.z - source_start.z,
            );
            if source_forward.x * forward.x + source_forward.z * forward.z < 0.0 {
                std::mem::swap(&mut source_start, &mut source_end);
            }
            let band = &section.bands[band_index];
            source_edges.push(RoadSurfaceTerrainClipSourceEdge {
                start: source_start,
                end: source_end,
                kind: RoadSurfaceTerrainClipEdgeKind::SpanHandoff,
                source: RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                    edge_idx: section.edge_idx,
                    edge_class,
                    support_policy: RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class),
                    owner: RoadSurfaceSpanBandOwner {
                        source_band_index: band_index,
                        kind: band.kind,
                    },
                    role: RoadSurfaceSpanRegionRole::from_band_pair(band.kind, band.kind),
                    start_section_index: section_index,
                    end_section_index: section_index,
                    start_s_m: section.s_m,
                    end_s_m: section.s_m,
                },
            });
        }
    }

    fn section_boundary_points_for_handoff(section: &RoadSurfaceSection) -> Vec<Vector3> {
        let mut points = Vec::with_capacity(section.bands.len() + 1);
        for band in &section.bands {
            if points.is_empty() {
                points.push(Self::section_boundary_world_point_static(
                    section,
                    band.lateral_start_m,
                    band.height_start_m,
                ));
            }
            points.push(Self::section_boundary_world_point_static(
                section,
                band.lateral_end_m,
                band.height_end_m,
            ));
        }
        points
    }

    fn section_outer_boundary_pair(section: &RoadSurfaceSection) -> Option<(Vector3, Vector3)> {
        let first_band = section.bands.first()?;
        let last_band = section.bands.last()?;
        let left_point = Self::section_boundary_world_point_static(
            section,
            first_band.lateral_start_m,
            first_band.height_start_m,
        );
        let right_point = Self::section_boundary_world_point_static(
            section,
            last_band.lateral_end_m,
            last_band.height_end_m,
        );
        Some((left_point, right_point))
    }

    fn section_outer_boundary_edge_kinds(
        section: &RoadSurfaceSection,
    ) -> Option<(
        RoadSurfaceTerrainClipEdgeKind,
        RoadSurfaceTerrainClipEdgeKind,
    )> {
        Some((
            terrain_clip_edge_kind_for_band(section.bands.first()?.kind),
            terrain_clip_edge_kind_for_band(section.bands.last()?.kind),
        ))
    }

    fn section_range_mouth_profile(
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
        side: IncidentEdgeSide,
    ) -> Option<IncidentMouthProfile> {
        let section = match side {
            IncidentEdgeSide::Start => {
                let &(start_index, _) = ranges.first()?;
                sections.get(start_index)?
            }
            IncidentEdgeSide::End => {
                let &(_, end_index) = ranges.last()?;
                sections.get(end_index)?
            }
        };
        Self::build_mouth_profile_from_section(section, side)
    }

    pub(super) fn build_mouth_profile_from_section(
        section: &RoadSurfaceSection,
        side: IncidentEdgeSide,
    ) -> Option<IncidentMouthProfile> {
        let mut boundary_points_world = Vec::with_capacity(section.bands.len() + 1);
        let mut bands = Vec::with_capacity(section.bands.len());

        if side == IncidentEdgeSide::Start {
            for band in &section.bands {
                let start_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_start_m,
                    band.height_start_m,
                );
                let end_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_end_m,
                    band.height_end_m,
                );
                if boundary_points_world.is_empty() {
                    boundary_points_world.push(start_point_world);
                }
                boundary_points_world.push(end_point_world);
                bands.push(IncidentMouthBand {
                    kind: band.kind,
                    start_point_world,
                    end_point_world,
                });
            }
        } else {
            for band in section.bands.iter().rev() {
                let start_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_end_m,
                    band.height_end_m,
                );
                let end_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_start_m,
                    band.height_start_m,
                );
                if boundary_points_world.is_empty() {
                    boundary_points_world.push(start_point_world);
                }
                boundary_points_world.push(end_point_world);
                bands.push(IncidentMouthBand {
                    kind: band.kind,
                    start_point_world,
                    end_point_world,
                });
            }
        }

        Some(IncidentMouthProfile {
            inward_direction_xz: match side {
                IncidentEdgeSide::Start => section.tangent_xz,
                IncidentEdgeSide::End => -section.tangent_xz,
            },
            boundary_points_world,
            bands,
        })
    }
}

fn canonicalize_span_terrain_clip_source_edges(
    source_edges: &mut [RoadSurfaceTerrainClipSourceEdge],
    loop_points: &[Vector3],
) {
    for edge in source_edges {
        if let Some(point) = matching_canonical_loop_point(edge.start, loop_points) {
            edge.start = point;
        }
        if let Some(point) = matching_canonical_loop_point(edge.end, loop_points) {
            edge.end = point;
        }
    }
}

fn matching_canonical_loop_point(point: Vector3, loop_points: &[Vector3]) -> Option<Vector3> {
    loop_points.iter().copied().find(|candidate| {
        SurfaceXzKey::from_godot_world_xz(*candidate) == SurfaceXzKey::from_godot_world_xz(point)
            && SurfaceHeightMmKey::from_m_f32(candidate.y)
                == SurfaceHeightMmKey::from_m_f32(point.y)
    })
}
