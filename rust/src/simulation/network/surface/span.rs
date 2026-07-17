//! Explicit visual span-piece compilation and span mouth profile construction.

mod boundaries;
mod mouth_profile;
mod raised_steps;
mod regions;

use super::{
    IncidentEdgeSide, IncidentMouthProfile, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSystem,
    RoadSurfaceTerrainClipLoop, RoadSurfaceVisualPolygon, backend::RoadVec3,
    band_semantics::band_kind_sort_key,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;

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
    pub(crate) source_corners_world: [RoadVec3; 4],
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
    pub(crate) start_lower_world: RoadVec3,
    pub(crate) start_raised_world: RoadVec3,
    pub(crate) end_lower_world: RoadVec3,
    pub(crate) end_raised_world: RoadVec3,
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
    /// Whether the start node footprint belongs to a grounded bridge abutment cutout.
    pub(crate) start_terrain_clip_node: bool,
    /// Whether the end node footprint belongs to a grounded bridge abutment cutout.
    pub(crate) end_terrain_clip_node: bool,
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
    lower_world: RoadVec3,
    raised_world: RoadVec3,
    lower_direction: RoadVec3,
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

impl RoadSurfaceSpanOwnedRegion {
    pub(crate) fn support_boundary_source(
        &self,
        edge_class: EdgeClass,
    ) -> RoadSurfaceEarthworkFaceSource {
        self.support_boundary_source_for(
            edge_class,
            self.role,
            self.start_section_index,
            self.end_section_index,
            self.start_s_m,
            self.end_s_m,
        )
    }

    pub(crate) fn handoff_boundary_source(
        &self,
        edge_class: EdgeClass,
        section_index: usize,
        s_m: f32,
    ) -> RoadSurfaceEarthworkFaceSource {
        self.support_boundary_source_for(
            edge_class,
            RoadSurfaceSpanRegionRole::from_band_pair(self.owner.kind, self.owner.kind),
            section_index,
            section_index,
            s_m,
            s_m,
        )
    }

    fn support_boundary_source_for(
        &self,
        edge_class: EdgeClass,
        role: RoadSurfaceSpanRegionRole,
        start_section_index: usize,
        end_section_index: usize,
        start_s_m: f32,
        end_s_m: f32,
    ) -> RoadSurfaceEarthworkFaceSource {
        RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx: self.edge_idx,
            edge_class,
            support_policy: RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class),
            owner: self.owner,
            role,
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
        }
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
            self.resolve_span_regions_for_ranges(sections, &visible_ranges, edge.class)?;
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
        let visible_terrain_clip_boundary_loops =
            std::mem::take(&mut visible_regions.terrain_clip_boundary_loops);

        let earthwork_ranges =
            self.earthwork_section_ranges_for_edge(graph, edge_idx, edge, sections, terrain);
        let mut clearance_regions =
            self.resolve_span_regions_for_ranges(sections, &earthwork_ranges, edge.class)?;
        Self::sort_span_owned_regions(&mut clearance_regions.regions);
        let terrain_clip_boundary_loops = match edge.class {
            EdgeClass::Standard => visible_terrain_clip_boundary_loops,
            EdgeClass::Bridge => std::mem::take(&mut clearance_regions.terrain_clip_boundary_loops),
            EdgeClass::Tunnel => Vec::new(),
        };
        let span_earthwork_support_regions = clearance_regions.regions;
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            if edge.class == EdgeClass::Bridge {
                (
                    Vec::new(),
                    std::mem::take(&mut clearance_regions.outer_boundary_loops),
                    Vec::new(),
                )
            } else {
                let earthwork_boundary_segments =
                    Self::span_earthwork_boundary_segment_loops_from_support_regions(
                        &span_earthwork_support_regions,
                        edge.class,
                    )
                    .ok()?;
                self.build_closed_earthwork_geometry_from_boundary_segments(
                    &earthwork_boundary_segments,
                    terrain,
                    None,
                )
                .ok()?
            };

        let start_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::Start);
        let end_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::End);
        let start_terrain_clip_node = edge.class == EdgeClass::Bridge
            && sections
                .first()
                .is_some_and(|section| self.bridge_section_contacts_terrain(section, terrain));
        let end_terrain_clip_node = edge.class == EdgeClass::Bridge
            && sections
                .last()
                .is_some_and(|section| self.bridge_section_contacts_terrain(section, terrain));

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
            start_terrain_clip_node,
            end_terrain_clip_node,
            span_earthwork_support_regions,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }
}
