//! Explicit visual span-piece compilation and span mouth profile construction.

use super::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, RoadSurfaceBandKind,
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, SAMPLE_EPSILON_M, WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
    terrain_clip_edge_kind_for_band,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

// Avoid resolved region construction between adjacent bands whose widths have collapsed together.
const SPAN_REGION_MIN_BAND_WIDTH_M: f32 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanBandOwner {
    source_band_index: usize,
    kind: RoadSurfaceBandKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpanResolvedRegionRole {
    Asphalt,
    CurbOrShoulder,
    NonRoad,
}

#[derive(Clone, Debug, PartialEq)]
struct SpanResolvedRegion {
    owner: SpanBandOwner,
    role: SpanResolvedRegionRole,
    polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SpanResolvedRegionSet {
    regions: Vec<SpanResolvedRegion>,
    raised_step_constraints: Vec<SpanRaisedStepConstraint>,
    outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanRaisedStepConstraint {
    lower_owner: SpanBandOwner,
    raised_owner: SpanBandOwner,
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
    lower_owner: SpanBandOwner,
    raised_owner: SpanBandOwner,
    sample: SpanRaisedStepSample,
}

impl SpanResolvedRegionRole {
    fn from_band_pair(start_kind: RoadSurfaceBandKind, end_kind: RoadSurfaceBandKind) -> Self {
        match (start_kind, end_kind) {
            (RoadSurfaceBandKind::Carriageway, RoadSurfaceBandKind::Carriageway) => Self::Asphalt,
            (RoadSurfaceBandKind::CurbOrShoulder, RoadSurfaceBandKind::CurbOrShoulder) => {
                Self::CurbOrShoulder
            }
            _ => Self::NonRoad,
        }
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
        let mut visible_regions = self.resolve_span_regions_for_ranges(sections, &visible_ranges);
        let (mut road_surface_polygons, mut curb_surface_polygons, mut sidewalk_surface_polygons) =
            Self::span_surface_polygons_from_regions(&visible_regions.regions);
        let mut raised_step_face_polygons = Self::span_raised_step_face_polygons_from_constraints(
            &visible_regions.raised_step_constraints,
        );

        if road_surface_polygons.is_empty()
            && curb_surface_polygons.is_empty()
            && sidewalk_surface_polygons.is_empty()
        {
            return None;
        }

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_visual_polygons(&mut raised_step_face_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        let outer_boundary_loops = std::mem::take(&mut visible_regions.outer_boundary_loops);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        let terrain_clip_boundary_loops =
            std::mem::take(&mut visible_regions.terrain_clip_boundary_loops);

        let earthwork_ranges = self.earthwork_section_ranges_for_edge(edge, sections, terrain);
        let mut clearance_regions =
            self.resolve_span_regions_for_ranges(sections, &earthwork_ranges);
        let (
            mut clearance_road_surface_polygons,
            mut clearance_curb_surface_polygons,
            mut clearance_sidewalk_surface_polygons,
        ) = Self::span_surface_polygons_from_regions(&clearance_regions.regions);
        Self::sort_visual_polygons(&mut clearance_road_surface_polygons);
        Self::sort_visual_polygons(&mut clearance_curb_surface_polygons);
        Self::sort_visual_polygons(&mut clearance_sidewalk_surface_polygons);
        let earthwork_boundary_loops = std::mem::take(&mut clearance_regions.outer_boundary_loops);
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &earthwork_boundary_loops,
                terrain,
            );

        let start_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::Start);
        let end_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::End);

        Some(RoadSurfaceVisualSpanPiece {
            edge_idx,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            raised_step_face_polygons,
            sidewalk_surface_polygons,
            edge_class: edge.class,
            start_mouth_profile,
            end_mouth_profile,
            clearance_road_surface_polygons,
            clearance_curb_surface_polygons,
            clearance_sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }

    fn resolve_span_regions_for_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
    ) -> SpanResolvedRegionSet {
        let mut resolved = SpanResolvedRegionSet {
            outer_boundary_loops: Self::build_span_outer_boundary_loops(sections, ranges),
            terrain_clip_boundary_loops: Self::build_span_terrain_clip_boundary_loops(
                sections, ranges,
            ),
            ..SpanResolvedRegionSet::default()
        };

        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            for pair in sections[start_index..=end_index].windows(2) {
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

                    resolved.regions.push(SpanResolvedRegion {
                        owner: SpanBandOwner {
                            source_band_index: band_index,
                            kind: band_a.kind,
                        },
                        role: SpanResolvedRegionRole::from_band_pair(band_a.kind, band_b.kind),
                        polygon,
                    });
                }
                resolved.raised_step_constraints.extend(
                    Self::span_raised_step_constraints_for_resolved_segment(pair),
                );
            }
        }

        resolved
    }

    fn span_surface_polygons_from_regions(
        regions: &[SpanResolvedRegion],
    ) -> (
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceVisualPolygon>,
    ) {
        let mut road_surface_polygons = Vec::new();
        let mut curb_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();

        for region in regions {
            match region.role {
                SpanResolvedRegionRole::Asphalt => {
                    road_surface_polygons.push(region.polygon.clone());
                }
                SpanResolvedRegionRole::CurbOrShoulder => {
                    curb_surface_polygons.push(region.polygon.clone());
                }
                SpanResolvedRegionRole::NonRoad => {
                    sidewalk_surface_polygons.push(region.polygon.clone());
                }
            }
        }

        (
            road_surface_polygons,
            curb_surface_polygons,
            sidewalk_surface_polygons,
        )
    }

    fn span_raised_step_constraints_for_resolved_segment(
        pair: &[RoadSurfaceSection],
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

        let left_owner = SpanBandOwner {
            source_band_index: lower_index,
            kind: left.kind,
        };
        let right_owner = SpanBandOwner {
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

    fn span_raised_step_face_polygons_from_constraints(
        constraints: &[SpanRaisedStepConstraint],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        constraints
            .iter()
            .filter_map(Self::span_raised_step_face_polygon_from_constraint)
            .collect()
    }

    fn span_raised_step_face_polygon_from_constraint(
        constraint: &SpanRaisedStepConstraint,
    ) -> Option<RoadSurfaceVisualPolygon> {
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

        Self::make_vertical_quad_polygon(points)
    }

    fn build_span_outer_boundary_loops(
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut loops = Vec::new();
        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            let mut left_points = Vec::new();
            let mut right_points = Vec::new();
            for section in &sections[start_index..=end_index] {
                let Some((left_point, right_point)) = Self::section_outer_boundary_pair(section)
                else {
                    continue;
                };
                left_points.push(left_point);
                right_points.push(right_point);
            }
            if left_points.len() < 2 || right_points.len() < 2 {
                continue;
            }
            right_points.reverse();
            let mut loop_points = left_points;
            loop_points.extend(right_points);
            if let Some(loop_polygon) = Self::make_visual_polygon(loop_points) {
                loops.push(loop_polygon);
            }
        }
        Self::sort_visual_polygons(&mut loops);
        loops
    }

    fn build_span_terrain_clip_boundary_loops(
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
    ) -> Vec<RoadSurfaceTerrainClipLoop> {
        let mut loops = Vec::new();
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

            let mut source_edges =
                Vec::with_capacity(left_points.len() + right_points.len().saturating_sub(2) + 2);
            for pair in left_points.windows(2) {
                source_edges.push(RoadSurfaceTerrainClipSourceEdge {
                    start: pair[0],
                    end: pair[1],
                    kind: left_kind,
                });
            }
            let last_left = *left_points.last().unwrap();
            let last_right = *right_points.last().unwrap();
            source_edges.push(RoadSurfaceTerrainClipSourceEdge {
                start: last_left,
                end: last_right,
                kind: RoadSurfaceTerrainClipEdgeKind::SpanHandoff,
            });
            for pair in right_points.windows(2) {
                source_edges.push(RoadSurfaceTerrainClipSourceEdge {
                    start: pair[1],
                    end: pair[0],
                    kind: right_kind,
                });
            }
            source_edges.push(RoadSurfaceTerrainClipSourceEdge {
                start: right_points[0],
                end: left_points[0],
                kind: RoadSurfaceTerrainClipEdgeKind::SpanHandoff,
            });
            canonicalize_span_terrain_clip_source_edges(
                &mut source_edges,
                &loop_polygon.points_world,
            );
            loops.push(RoadSurfaceTerrainClipLoop {
                points_world: loop_polygon.points_world,
                source_edges,
            });
        }
        Self::sort_terrain_clip_loops(&mut loops);
        loops
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
        (*candidate - point).length_squared() <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
    })
}
