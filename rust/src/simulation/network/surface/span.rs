//! Explicit visual span-piece compilation and span mouth profile construction.

use super::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, RoadSurfaceBandKind,
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, terrain_clip_edge_kind_for_band,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

// Avoid strip construction between adjacent bands whose widths have collapsed together.
const BAND_WIDTH_MATCH_EPSILON_M: f32 = 0.05;

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
        let (mut road_surface_polygons, mut sidewalk_surface_polygons) =
            self.compile_surface_polygons_for_ranges(sections, &visible_ranges);

        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        let outer_boundary_loops = Self::build_span_outer_boundary_loops(sections, &visible_ranges);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        let terrain_clip_boundary_loops =
            Self::build_span_terrain_clip_boundary_loops(sections, &visible_ranges);

        let earthwork_ranges = self.earthwork_section_ranges_for_edge(edge, sections, terrain);
        let (mut clearance_road_surface_polygons, mut clearance_sidewalk_surface_polygons) =
            self.compile_surface_polygons_for_ranges(sections, &earthwork_ranges);
        Self::sort_visual_polygons(&mut clearance_road_surface_polygons);
        Self::sort_visual_polygons(&mut clearance_sidewalk_surface_polygons);
        let earthwork_boundary_loops =
            Self::build_span_outer_boundary_loops(sections, &earthwork_ranges);
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
            sidewalk_surface_polygons,
            edge_class: edge.class,
            start_mouth_profile,
            end_mouth_profile,
            clearance_road_surface_polygons,
            clearance_sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }

    fn compile_surface_polygons_for_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
    ) -> (Vec<RoadSurfaceVisualPolygon>, Vec<RoadSurfaceVisualPolygon>) {
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();

        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            for pair in sections[start_index..=end_index].windows(2) {
                if pair[0].bands.len() != pair[1].bands.len() {
                    continue;
                }
                for (band_a, band_b) in pair[0].bands.iter().zip(&pair[1].bands) {
                    let width_a = (band_a.lateral_end_m - band_a.lateral_start_m).abs();
                    let width_b = (band_b.lateral_end_m - band_b.lateral_start_m).abs();
                    if width_a <= BAND_WIDTH_MATCH_EPSILON_M
                        && width_b <= BAND_WIDTH_MATCH_EPSILON_M
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

                    if band_a.kind == RoadSurfaceBandKind::Carriageway
                        && band_b.kind == RoadSurfaceBandKind::Carriageway
                    {
                        road_surface_polygons.push(polygon);
                    } else {
                        sidewalk_surface_polygons.push(polygon);
                    }
                }
            }
        }

        (road_surface_polygons, sidewalk_surface_polygons)
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
