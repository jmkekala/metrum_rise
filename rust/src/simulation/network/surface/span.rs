// SPDX-License-Identifier: GPL-2.0-only

//! Explicit visual span-piece compilation and span mouth profile construction.

mod boundaries;
mod mouth_profile;
mod quads;
mod raised_steps;
mod regions;

pub use quads::SpanQuad;

use super::{
    IncidentEdgeSide, IncidentMouthProfile, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSection,
    RoadSurfaceSystem,
    RoadSurfaceTerrainClipLoop, RoadSurfaceVisualPolygon, SurfaceTriangleGrid,
    backend::RoadVec3, band_semantics::band_kind_sort_key,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use std::sync::Arc;
use std::time::Instant;

// Avoid resolved region construction between adjacent bands whose widths have collapsed together.
const SPAN_REGION_MIN_BAND_WIDTH_M: f32 = 0.05;

fn elapsed_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

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
    // Sections the regions were resolved from; region quads are rebuilt from them on read.
    pub(crate) sections: Arc<Vec<RoadSurfaceSection>>,
    // Indices into `span_owned_regions` of the asphalt, curb and sidewalk quads in render order.
    pub(in crate::simulation::network::surface) surface_polygon_order: [Box<[u32]>; 3],
    // Raised-step faces in render order; each is rebuilt from its section pair.
    pub(in crate::simulation::network::surface) raised_steps: Box<[SpanRaisedStep]>,
    // Top-triangle grid; items are `region_index << 1 | triangle_index` into the owned regions.
    pub(in crate::simulation::network::surface) surface_query: Arc<SurfaceTriangleGrid>,
    pub(crate) span_owned_regions: Arc<Vec<RoadSurfaceSpanOwnedRegion>>,
    pub(crate) edge_class: EdgeClass,
    pub(crate) start_mouth_profile: Option<IncidentMouthProfile>,
    pub(crate) end_mouth_profile: Option<IncidentMouthProfile>,
    /// Whether the start node footprint belongs to a grounded bridge abutment cutout.
    pub(crate) start_terrain_clip_node: bool,
    /// Whether the end node footprint belongs to a grounded bridge abutment cutout.
    pub(crate) end_terrain_clip_node: bool,
    pub(crate) span_earthwork_support_regions: Arc<Vec<RoadSurfaceSpanOwnedRegion>>,
    // `render_earthwork_faces` indices in earthwork surface polygon order.
    pub(in crate::simulation::network::surface) earthwork_surface_order: Box<[u32]>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

impl RoadSurfaceVisualSpanPiece {
    pub(in crate::simulation::network::surface) fn into_with_edge_identity(
        mut self,
        edge_idx: usize,
    ) -> Self {
        let shared_regions = Arc::ptr_eq(
            &self.span_owned_regions,
            &self.span_earthwork_support_regions,
        );
        if shared_regions {
            drop(self.span_earthwork_support_regions);
            let mut regions = Arc::unwrap_or_clone(self.span_owned_regions);
            for region in &mut regions {
                region.edge_idx = edge_idx;
            }
            self.span_owned_regions = Arc::new(regions);
            self.span_earthwork_support_regions = Arc::clone(&self.span_owned_regions);
        } else {
            let mut owned_regions = Arc::unwrap_or_clone(self.span_owned_regions);
            for region in &mut owned_regions {
                region.edge_idx = edge_idx;
            }
            self.span_owned_regions = Arc::new(owned_regions);

            let mut support_regions = Arc::unwrap_or_clone(self.span_earthwork_support_regions);
            for region in &mut support_regions {
                region.edge_idx = edge_idx;
            }
            self.span_earthwork_support_regions = Arc::new(support_regions);
        }
        for boundary_loop in &mut self.terrain_clip_boundary_loops {
            for source_edge in &mut boundary_loop.source_edges {
                source_edge.source = source_edge.source.with_span_identity(edge_idx);
            }
        }
        for face in &mut self.render_earthwork_faces {
            face.source = face.source.with_span_identity(edge_idx);
        }
        self.sections = sections_with_edge_identity(self.sections, edge_idx);
        self.edge_idx = edge_idx;
        self
    }

    pub(in crate::simulation::network::surface) fn clone_with_edge_identity(
        &self,
        edge_idx: usize,
    ) -> Self {
        let remap_regions = |regions: &[RoadSurfaceSpanOwnedRegion]| {
            regions
                .iter()
                .cloned()
                .map(|mut region| {
                    region.edge_idx = edge_idx;
                    region
                })
                .collect::<Vec<_>>()
        };
        let span_owned_regions = Arc::new(remap_regions(&self.span_owned_regions));
        let span_earthwork_support_regions = if Arc::ptr_eq(
            &self.span_owned_regions,
            &self.span_earthwork_support_regions,
        ) {
            Arc::clone(&span_owned_regions)
        } else {
            Arc::new(remap_regions(&self.span_earthwork_support_regions))
        };
        let mut terrain_clip_boundary_loops = self.terrain_clip_boundary_loops.clone();
        for boundary_loop in &mut terrain_clip_boundary_loops {
            for source_edge in &mut boundary_loop.source_edges {
                source_edge.source = source_edge.source.with_span_identity(edge_idx);
            }
        }
        let mut render_earthwork_faces = self.render_earthwork_faces.clone();
        for face in &mut render_earthwork_faces {
            face.source = face.source.with_span_identity(edge_idx);
        }

        Self {
            edge_idx,
            outer_boundary_loops: self.outer_boundary_loops.clone(),
            terrain_clip_boundary_loops,
            sections: sections_with_edge_identity(Arc::clone(&self.sections), edge_idx),
            surface_polygon_order: self.surface_polygon_order.clone(),
            raised_steps: self.raised_steps.clone(),
            surface_query: Arc::clone(&self.surface_query),
            span_owned_regions,
            edge_class: self.edge_class,
            start_mouth_profile: self.start_mouth_profile.clone(),
            end_mouth_profile: self.end_mouth_profile.clone(),
            start_terrain_clip_node: self.start_terrain_clip_node,
            end_terrain_clip_node: self.end_terrain_clip_node,
            span_earthwork_support_regions,
            earthwork_surface_order: self.earthwork_surface_order.clone(),
            earthwork_outer_boundary_loops: self.earthwork_outer_boundary_loops.clone(),
            render_earthwork_faces,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SpanResolvedRegionSet {
    regions: Vec<RoadSurfaceSpanOwnedRegion>,
    raised_step_constraints: Vec<SpanRaisedStepConstraint>,
    outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanRaisedStepConstraint {
    lower_owner: RoadSurfaceSpanBandOwner,
    raised_owner: RoadSurfaceSpanBandOwner,
    start_section_index: usize,
    end_section_index: usize,
    boundary_index: usize,
    start_s_m: f32,
    end_s_m: f32,
    start: SpanRaisedStepSample,
    end: SpanRaisedStepSample,
}

/// One span raised-step face: the step at band boundary `boundary_index` between sections
/// `start_section_index` and the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SpanRaisedStep {
    start_section_index: u32,
    boundary_index: u32,
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

impl RoadSurfaceVisualSpanPiece {
    // Retained lists are built by pushing; drop their spare capacity once, at publication size.
    fn shrink_retained(&mut self) {
        self.outer_boundary_loops.shrink_to_fit();
        shrink_terrain_clip_loops(&mut self.terrain_clip_boundary_loops);
        for regions in [&mut self.span_owned_regions, &mut self.span_earthwork_support_regions] {
            if let Some(regions) = Arc::get_mut(regions) {
                regions.shrink_to_fit();
            }
        }
        self.earthwork_outer_boundary_loops.shrink_to_fit();
        self.render_earthwork_faces.shrink_to_fit();
    }

    /// Earthwork side polygons in `visual_polygon_ordering` order; each is a render face's.
    pub(crate) fn earthwork_surface_polygons(
        &self,
    ) -> impl ExactSizeIterator<Item = &RoadSurfaceVisualPolygon> + '_ {
        self.earthwork_surface_order
            .iter()
            .map(|&index| &self.render_earthwork_faces[index as usize].polygon)
    }
}

/// Drops spare capacity from retained terrain-clip loops and their source edges.
pub(in crate::simulation::network::surface) fn shrink_terrain_clip_loops(
    loops: &mut Vec<RoadSurfaceTerrainClipLoop>,
) {
    loops.shrink_to_fit();
    for clip_loop in loops {
        clip_loop.points_world.shrink_to_fit();
        clip_loop.source_edges.shrink_to_fit();
    }
}

// Remapped pieces carry their sections under the new edge id, so equality with a piece
// compiled under that id holds. Copies the sections only when the id actually changes.
fn sections_with_edge_identity(
    sections: Arc<Vec<RoadSurfaceSection>>,
    edge_idx: usize,
) -> Arc<Vec<RoadSurfaceSection>> {
    if sections.iter().all(|section| section.edge_idx == edge_idx) {
        return sections;
    }
    let mut sections = Arc::unwrap_or_clone(sections);
    for section in &mut sections {
        section.edge_idx = edge_idx;
    }
    Arc::new(sections)
}

impl RoadSurfaceSystem {
    pub(super) fn compile_visual_span_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
    ) -> Option<RoadSurfaceVisualSpanPiece> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        if edge_idx >= graph.edge_count() {
            return None;
        }
        let edge = graph.edge(edge_idx);
        let sections = Arc::clone(self.compiled_sections.get(&edge_idx)?);
        let visible_ranges_start = road_debug.then(Instant::now);
        let visible_ranges =
            self.visible_section_ranges_for_edge(graph, terrain, edge_idx, &sections);
        let visible_ranges_ms = elapsed_ms(visible_ranges_start);
        let visible_geometry_start = road_debug.then(Instant::now);
        let mut visible_regions =
            self.resolve_span_regions_for_ranges(&sections, &visible_ranges, edge.class)?;
        Self::sort_span_owned_regions(&sections, &mut visible_regions.regions);
        let surface_polygon_order =
            Self::span_surface_polygon_order(&sections, &visible_regions.regions);
        let mut raised_step_faces =
            Self::span_raised_step_faces_from_constraints(&visible_regions.raised_step_constraints);
        Self::sort_span_raised_step_faces(&mut raised_step_faces);
        let raised_steps = raised_step_faces
            .into_iter()
            .map(|(_, _, step)| step)
            .collect();

        if surface_polygon_order.iter().all(|order| order.is_empty()) {
            return None;
        }

        let outer_boundary_loops = std::mem::take(&mut visible_regions.outer_boundary_loops);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        let visible_terrain_clip_boundary_loops =
            std::mem::take(&mut visible_regions.terrain_clip_boundary_loops);
        let span_owned_regions = Arc::new(std::mem::take(&mut visible_regions.regions));
        let visible_geometry_ms = elapsed_ms(visible_geometry_start);

        let earthwork_regions_start = road_debug.then(Instant::now);
        let earthwork_ranges =
            self.earthwork_section_ranges_for_edge(graph, edge_idx, edge, &sections, terrain);
        let reuse_visible_regions_for_earthwork =
            edge.class == EdgeClass::Standard && earthwork_ranges == visible_ranges;
        let (terrain_clip_boundary_loops, span_earthwork_support_regions, bridge_outer_loops) =
            if reuse_visible_regions_for_earthwork {
                (
                    visible_terrain_clip_boundary_loops,
                    Arc::clone(&span_owned_regions),
                    Vec::new(),
                )
            } else {
                let mut clearance_regions = self.resolve_span_regions_for_ranges(
                    &sections,
                    &earthwork_ranges,
                    edge.class,
                )?;
                Self::sort_span_owned_regions(&sections, &mut clearance_regions.regions);
                let terrain_clip_boundary_loops = match edge.class {
                    EdgeClass::Standard => visible_terrain_clip_boundary_loops,
                    EdgeClass::Bridge => {
                        std::mem::take(&mut clearance_regions.terrain_clip_boundary_loops)
                    }
                    EdgeClass::Tunnel => Vec::new(),
                };
                (
                    terrain_clip_boundary_loops,
                    Arc::new(clearance_regions.regions),
                    std::mem::take(&mut clearance_regions.outer_boundary_loops),
                )
            };
        let earthwork_regions_ms = elapsed_ms(earthwork_regions_start);
        let earthwork_geometry_start = road_debug.then(Instant::now);
        let (earthwork_outer_boundary_loops, render_earthwork_faces) =
            if edge.class == EdgeClass::Bridge {
                (bridge_outer_loops, Vec::new())
            } else {
                let earthwork_boundary_segments =
                    Self::span_earthwork_boundary_segment_loops_from_support_regions(
                        &sections,
                        &span_earthwork_support_regions,
                        edge.class,
                    )
                    .map_err(|error| {
                        crate::debug_log!(
                            "road",
                            "span_earthwork_boundary_failed edge={} error={:?}",
                            edge_idx,
                            error
                        );
                    })
                    .ok()?;
                self.build_closed_earthwork_geometry_from_boundary_segments(
                    &earthwork_boundary_segments,
                    terrain,
                    None,
                )
                .map_err(|error| {
                    crate::debug_log!(
                        "road",
                        "span_earthwork_geometry_failed edge={} error={:?}",
                        edge_idx,
                        error
                    );
                })
                .ok()?
            };
        let earthwork_geometry_ms = elapsed_ms(earthwork_geometry_start);

        let finalize_start = road_debug.then(Instant::now);
        let start_mouth_profile =
            Self::section_range_mouth_profile(&sections, &visible_ranges, IncidentEdgeSide::Start);
        let end_mouth_profile =
            Self::section_range_mouth_profile(&sections, &visible_ranges, IncidentEdgeSide::End);
        let start_terrain_clip_node = edge.class == EdgeClass::Bridge
            && sections
                .first()
                .is_some_and(|section| self.bridge_section_contacts_terrain(section, terrain));
        let end_terrain_clip_node = edge.class == EdgeClass::Bridge
            && sections
                .last()
                .is_some_and(|section| self.bridge_section_contacts_terrain(section, terrain));
        let finalize_ms = elapsed_ms(finalize_start);
        if road_debug {
            crate::debug_log!(
                "road",
                "span_compile_detail edge={} class={:?} sections={} visible_ranges={} earthwork_ranges={} visible_regions={} earthwork_regions={} earthwork_faces={} visible_ranges_ms={:.3} visible_geometry_ms={:.3} earthwork_regions_ms={:.3} earthwork_geometry_ms={:.3} finalize_ms={:.3} total_ms={:.3}",
                edge_idx,
                edge.class,
                sections.len(),
                visible_ranges.len(),
                earthwork_ranges.len(),
                span_owned_regions.len(),
                span_earthwork_support_regions.len(),
                render_earthwork_faces.len(),
                visible_ranges_ms,
                visible_geometry_ms,
                earthwork_regions_ms,
                earthwork_geometry_ms,
                finalize_ms,
                elapsed_ms(total_start)
            );
        }

        let mut piece = RoadSurfaceVisualSpanPiece {
            edge_idx,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            sections,
            surface_polygon_order,
            raised_steps,
            surface_query: Arc::default(),
            span_owned_regions,
            edge_class: edge.class,
            start_mouth_profile,
            end_mouth_profile,
            start_terrain_clip_node,
            end_terrain_clip_node,
            span_earthwork_support_regions,
            earthwork_surface_order: Self::earthwork_surface_order(&render_earthwork_faces),
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        };
        piece.surface_query = Arc::new(piece.surface_triangle_grid());
        piece.shrink_retained();
        Some(piece)
    }
}
