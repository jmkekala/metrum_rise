//! Explicit visual node-piece construction and incident-edge classification.

use super::band_semantics::ordered_raised_step_kinds;
use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShapes, NodeOwnedRegion,
    NodeTopSurfacePolygonSource, NodeTopSurfaceVertexSource, OrderedIncidentPieceMouth,
    RoadSurfaceBandKind, RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge, RoadSurfaceVerticalFaceSource,
    RoadSurfaceVisualNodePiece, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M,
    arrangement::{
        NodeArrangement, NodeArrangementFace, NodeArrangementKey, NodeBandOwner,
        NodeExplicitVerticalStepSegment,
    },
    backend::{RoadVec2, road_vec2_to_overlay_point},
    edge::VISUAL_MIN_SPAN_LENGTH_M,
    input::NodeInputExtractionError,
    node_boundary::{
        NodeBoundaryExportError, interpolate_missing_footprint_boundary_heights,
        remove_unsupported_numeric_boundary_vertices,
    },
    terrain_clip_edge_kind_for_band,
    validation::NodeValidationReport,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const VERTICAL_STEP_MIN_SPAN_M: f32 = 1.0e-6;
const VISUAL_DOMINANT_HANDOFF_REJECTION_RATIO: f32 = 3.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ArrangementFaceBoundaryInterval {
    owner: NodeBandOwner,
    start: ArrangementSegmentParameter,
    end: ArrangementSegmentParameter,
    edge_start: ArrangementBoundaryPointKey,
    edge_end: ArrangementBoundaryPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ArrangementBoundaryPointKey {
    x_key: i64,
    z_key: i64,
    y_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArrangementSegmentParameter {
    numerator: i128,
    denominator: i128,
}

impl ArrangementBoundaryPointKey {
    fn from_world(point: Vector3) -> Self {
        Self {
            x_key: (f64::from(point.x) * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round()
                as i64,
            z_key: (f64::from(point.z) * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round()
                as i64,
            y_mm: (point.y * 1000.0).round() as i64,
        }
    }

    fn xz_key(self) -> NodeArrangementKey {
        NodeArrangementKey::from_point(super::backend::RoadVec2::new(
            self.x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
            self.z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        ))
    }
}

impl ArrangementSegmentParameter {
    fn zero() -> Self {
        Self {
            numerator: 0,
            denominator: 1,
        }
    }

    fn one() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }

    fn new(numerator: i128, denominator: i128) -> Option<Self> {
        (denominator > 0).then_some(Self {
            numerator,
            denominator,
        })
    }

    fn min(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }

    fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }
}

impl Ord for ArrangementSegmentParameter {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.numerator * other.denominator).cmp(&(other.numerator * self.denominator))
    }
}

impl PartialOrd for ArrangementSegmentParameter {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug)]
struct NodeEarthworkBoundarySourceEdge {
    start_key: NodeArrangementKey,
    end_key: NodeArrangementKey,
    source: RoadSurfaceEarthworkFaceSource,
}

impl RoadSurfaceSystem {
    pub(super) fn compile_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.sorted_incident_surface_edges(graph, valid);
        match self.classify_visual_node_kind(&incidents) {
            CompiledNodeKind::Terminal => incidents.first().and_then(|incident| {
                self.build_terminal_visual_node_piece(terrain, valid, *incident)
            }),
            CompiledNodeKind::PassThrough => None,
            CompiledNodeKind::Bend => self.build_bend_visual_node_piece(terrain, valid, &incidents),
            CompiledNodeKind::JunctionN => {
                self.build_junction_visual_node_piece(graph, terrain, valid, &incidents)
            }
        }
    }
    fn build_terminal_visual_node_piece(
        &self,
        terrain: &TerrainSystem,
        node_id: u32,
        incident: IncidentSurfaceEdge,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let mouths = self.build_ordered_piece_mouths(&[incident])?;
        self.build_canonical_visual_node_piece(
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &mouths,
        )
    }

    fn build_bend_visual_node_piece(
        &self,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() != 2 {
            return None;
        }
        let mouths = self.build_ordered_piece_mouths(incidents)?;
        self.build_canonical_visual_node_piece(
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::Bend,
            &mouths,
        )
    }

    fn build_junction_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() < 3 {
            return None;
        }
        if incidents
            .iter()
            .any(|incident| self.incident_edge_visual_handoff_is_overconstrained(graph, *incident))
        {
            return None;
        }
        let Some(mouths) = self.build_ordered_piece_mouths(incidents) else {
            return None;
        };
        self.build_canonical_visual_node_piece(
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &mouths,
        )
    }

    fn build_canonical_visual_node_piece(
        &self,
        terrain: &TerrainSystem,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let node_regions = self.compile_canonical_node_surface_regions(node_id, kind, mouths)?;
        let top_surface_shapes = Self::top_surface_overlay_shapes(
            node_regions
                .road_surface_polygons
                .iter()
                .chain(node_regions.curb_surface_polygons.iter())
                .chain(node_regions.sidewalk_surface_polygons.iter()),
        );
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_segments(
                &node_regions.earthwork_boundary_segments,
                terrain,
                top_surface_shapes.as_ref(),
            );

        self.assemble_explicit_node_piece(
            node_id,
            kind,
            node_regions.outer_boundary_loops,
            node_regions.terrain_clip_boundary_loops,
            node_regions.road_surface_polygons,
            node_regions.curb_surface_polygons,
            node_regions.raised_step_faces,
            node_regions.sidewalk_surface_polygons,
            node_regions.explicit_vertical_step_segments,
            node_regions.node_grade_authorities,
            node_regions.node_top_surface_sources,
            node_regions.owned_regions,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn compile_canonical_node_surface_regions(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<super::NodeSurfaceRegionResult> {
        let input = match Self::build_node_arrangement_input_from_mouths(node_id, kind, mouths) {
            Ok(input) => input,
            Err(error) => {
                self.log_node_input_extraction_error(node_id, kind, &error);
                return None;
            }
        };
        let rails = match Self::build_node_rail_contours_from_input(&input) {
            Ok(rails) => rails,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_rail_generation_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        let ownership = match Self::build_node_boolean_ownership_from_rails(&rails) {
            Ok(ownership) => ownership,
            Err(error) => {
                self.log_node_validation_report(
                    &NodeValidationReport::from_boolean_ownership_error(node_id, kind, &error),
                );
                return None;
            }
        };
        if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
            &ownership.owned_region_arrangement,
        ) {
            self.log_node_validation_report(&report);
            return None;
        }
        let heights =
            match Self::build_node_height_solution_from_ownership(&input, &rails, &ownership) {
                Ok(heights) => heights,
                Err(error) => {
                    self.log_node_validation_report(
                        &NodeValidationReport::from_height_field_error(node_id, kind, &error),
                    );
                    return None;
                }
            };
        let mut arrangement = match NodeArrangement::from_height_solution(&heights) {
            Ok(arrangement) => arrangement,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_arrangement_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        if let Some(report) = NodeValidationReport::from_arrangement_diagnostics(&arrangement) {
            self.log_node_validation_report(&report);
            return None;
        }

        let triangulation = match Self::build_node_triangulation_from_arrangement(&arrangement) {
            Ok(triangulation) => triangulation,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_triangulation_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        match Self::validate_node_triangulation_solution(&triangulation) {
            Ok(report) => self.log_node_validation_report(&report),
            Err(error) => {
                self.log_node_validation_report(&error.report);
                if error.report.has_blocking_diagnostics() {
                    return None;
                }
            }
        }

        if let Err(error) = arrangement.attach_triangulation(&triangulation) {
            self.log_node_validation_report(&NodeValidationReport::from_arrangement_error(
                node_id, kind, &error,
            ));
            return None;
        }

        match Self::node_surface_regions_from_arrangement(&arrangement, &ownership.footprint_shapes)
        {
            Ok(regions) => Some(regions),
            Err(error) => {
                self.log_node_boundary_export_error(&arrangement, &error);
                None
            }
        }
    }

    fn node_surface_regions_from_arrangement(
        arrangement: &NodeArrangement,
        _footprint_shapes: &super::NodeOverlayShapes,
    ) -> Result<super::NodeSurfaceRegionResult, NodeBoundaryExportError> {
        let mut node_grade_authorities = arrangement
            .vertices()
            .iter()
            .map(|vertex| vertex.grade_authority())
            .collect::<Vec<_>>();
        node_grade_authorities.sort();
        node_grade_authorities.dedup();
        let authority_indices = node_grade_authorities
            .iter()
            .enumerate()
            .map(|(index, authority)| (*authority, index))
            .collect::<BTreeMap<_, _>>();

        let mut owned_region_exports = Vec::new();

        for face in arrangement.faces() {
            let owner = face.owner();
            let Some((polygon, source)) =
                Self::visual_polygon_from_arrangement_face(arrangement, face, &authority_indices)?
            else {
                continue;
            };
            if Self::signed_polygon_area_xz(&polygon.points_world).abs() <= NODE_OVERLAY_MIN_AREA_M2
            {
                continue;
            }
            owned_region_exports.push((
                NodeOwnedRegion {
                    kind: owner.kind(),
                    owner_index: owner.owner_index(),
                    polygon,
                },
                source,
            ));
        }
        let (mut owned_regions, mut node_top_surface_sources): (Vec<_>, Vec<_>) =
            owned_region_exports.into_iter().unzip();
        let explicit_vertical_step_segments = arrangement.explicit_vertical_step_segments();
        let mut raised_step_faces = Self::raised_step_face_polygons_from_arrangement(
            arrangement,
            &explicit_vertical_step_segments,
        );

        if owned_regions.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }

        let (mut road_surface_polygons, mut curb_surface_polygons, mut sidewalk_surface_polygons) =
            Self::visible_top_polygons_from_owned_regions(&owned_regions);
        push_missing_raised_step_faces_from_owned_region_boundaries(
            &mut raised_step_faces,
            &owned_regions,
            &explicit_vertical_step_segments,
        );
        push_missing_raised_step_faces_from_top_owner_boundaries(
            &mut raised_step_faces,
            &owned_regions,
        );
        retain_raised_step_faces_with_top_support(&mut raised_step_faces, &owned_regions);
        orient_raised_step_faces_to_lower_owner_support(&mut raised_step_faces, &owned_regions);
        dedup_raised_step_faces(&mut raised_step_faces);
        orient_raised_step_faces_to_lower_owner_support(&mut raised_step_faces, &owned_regions);
        if road_surface_polygons.is_empty()
            && curb_surface_polygons.is_empty()
            && sidewalk_surface_polygons.is_empty()
        {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }

        let top_polygons = road_surface_polygons
            .iter()
            .chain(curb_surface_polygons.iter())
            .chain(sidewalk_surface_polygons.iter())
            .collect::<Vec<_>>();
        let visible_top_shapes = Self::visible_top_overlay_shapes(&top_polygons)?;
        let footprint_boundary_point_loops = Self::footprint_boundary_point_loops_from_shapes(
            arrangement,
            &top_polygons,
            &visible_top_shapes,
            true,
        )?;
        let mut earthwork_boundary_segments =
            Self::node_earthwork_boundary_segments_from_footprint_loops(
                arrangement.node_id(),
                arrangement.piece_kind(),
                &footprint_boundary_point_loops,
                &owned_regions,
            )?;
        Self::orient_earthwork_boundary_segment_loops_by_nesting(&mut earthwork_boundary_segments);
        let mut outer_boundary_loops =
            Self::outer_boundary_polygons_from_point_loops(&footprint_boundary_point_loops)?;
        let mut terrain_clip_boundary_loops =
            Self::terrain_clip_boundary_loops_from_earthwork_segments(&earthwork_boundary_segments);

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_node_owned_regions_with_sources(
            &mut owned_regions,
            &mut node_top_surface_sources,
        )?;
        Self::sort_raised_step_faces(&mut raised_step_faces);

        Ok(super::NodeSurfaceRegionResult {
            outer_boundary_loops,
            earthwork_boundary_segments,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            raised_step_faces,
            sidewalk_surface_polygons,
            explicit_vertical_step_segments,
            node_grade_authorities,
            node_top_surface_sources,
            owned_regions,
        })
    }

    fn sort_node_owned_regions_with_sources(
        owned_regions: &mut Vec<NodeOwnedRegion>,
        node_top_surface_sources: &mut Vec<NodeTopSurfacePolygonSource>,
    ) -> Result<(), NodeBoundaryExportError> {
        if owned_regions.len() != node_top_surface_sources.len() {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        }
        let mut paired = owned_regions
            .drain(..)
            .zip(node_top_surface_sources.drain(..))
            .collect::<Vec<_>>();
        paired.sort_by(|(region_a, source_a), (region_b, source_b)| {
            Self::node_owned_region_ordering(region_a, region_b)
                .then(source_a.height_field_id.cmp(&source_b.height_field_id))
        });
        owned_regions.reserve(paired.len());
        node_top_surface_sources.reserve(paired.len());
        for (region, source) in paired {
            owned_regions.push(region);
            node_top_surface_sources.push(source);
        }
        Ok(())
    }

    fn visible_top_overlay_shapes(
        top_polygons: &[&RoadSurfaceVisualPolygon],
    ) -> Result<NodeOverlayShapes, NodeBoundaryExportError> {
        let contours = top_polygons
            .iter()
            .filter_map(|polygon| {
                (polygon.points_world.len() >= 3).then(|| {
                    polygon
                        .points_world
                        .iter()
                        .map(|point| {
                            road_vec2_to_overlay_point(RoadVec2::new(
                                f64::from(point.x),
                                f64::from(point.z),
                            ))
                        })
                        .collect::<NodeOverlayContour>()
                })
            })
            .collect::<Vec<_>>();
        RoadSurfaceSystem::overlay_union_contours(&contours)
            .filter(|shapes| !shapes.is_empty())
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }

    fn footprint_boundary_point_loops_from_shapes(
        arrangement: &NodeArrangement,
        top_polygons: &[&RoadSurfaceVisualPolygon],
        footprint_shapes: &super::NodeOverlayShapes,
        clean_unsupported_numeric_vertices: bool,
    ) -> Result<Vec<Vec<Vector3>>, NodeBoundaryExportError> {
        let mut loops = Vec::new();
        for shape in footprint_shapes {
            for contour in shape {
                let mut keyed_points = Vec::with_capacity(contour.len());
                for point in contour {
                    let key = NodeArrangementKey::from_point(super::backend::RoadVec2::new(
                        point[0], point[1],
                    ));
                    keyed_points.push((
                        key,
                        Self::arrangement_footprint_boundary_height_mm(arrangement, key),
                    ));
                }
                interpolate_missing_footprint_boundary_heights(&mut keyed_points)?;
                let mut points = keyed_points
                    .into_iter()
                    .map(|(key, height_mm)| {
                        arrangement_boundary_point_to_world(arrangement_key_boundary_point(
                            key,
                            height_mm.expect("footprint boundary height was solved"),
                        ))
                    })
                    .collect::<Vec<_>>();
                if clean_unsupported_numeric_vertices {
                    remove_unsupported_numeric_boundary_vertices(
                        &mut points,
                        |current_key, local_points| {
                            visible_top_boundary_height_mm_at_key(top_polygons, current_key)
                                .is_some()
                                || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                                    > boundary_points_numeric_area_budget_m2(&local_points)
                        },
                    );
                }
                let points = Self::canonicalize_loop_points(points);
                if points.len() < 3 {
                    continue;
                }
                if Self::signed_polygon_area_xz(&points).abs()
                    <= boundary_points_numeric_area_budget_m2(&points)
                {
                    continue;
                }
                loops.push(points);
            }
        }
        (!loops.is_empty())
            .then_some(loops)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }

    fn arrangement_footprint_boundary_height_mm(
        arrangement: &NodeArrangement,
        key: NodeArrangementKey,
    ) -> Option<i64> {
        let mut heights_mm = Self::arrangement_visible_top_heights_at_key(arrangement, key);
        if heights_mm.is_empty() {
            heights_mm.extend(Self::arrangement_boundary_edge_heights_at_key(
                arrangement,
                key,
            ));
        }
        if heights_mm.is_empty() {
            heights_mm.extend(
                arrangement
                    .vertices()
                    .iter()
                    .filter(|vertex| vertex.key() == key)
                    .map(|vertex| vertex.height_mm()),
            );
        }
        heights_mm.into_iter().max()
    }

    fn arrangement_visible_top_heights_at_key(
        arrangement: &NodeArrangement,
        key: NodeArrangementKey,
    ) -> Vec<i64> {
        let point = Vector2::new(
            (key.x_key() as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE) as f32,
            (key.z_key() as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE) as f32,
        );
        let mut heights = Vec::new();
        for face in arrangement.faces() {
            let Some(triangle) = Self::arrangement_face_visual_triangle(arrangement, face) else {
                continue;
            };
            let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point) else {
                continue;
            };
            let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
            heights.push((height_m * 1000.0).round() as i64);
        }
        heights.sort_unstable();
        heights.dedup();
        heights
    }

    fn arrangement_boundary_edge_heights_at_key(
        arrangement: &NodeArrangement,
        key: NodeArrangementKey,
    ) -> Vec<i64> {
        let mut heights = Vec::new();
        for face in arrangement
            .faces()
            .iter()
            .filter(|face| Self::arrangement_face_visual_triangle(arrangement, face).is_some())
        {
            let vertices = face.vertices();
            for index in 0..vertices.len() {
                let Some(start) = arrangement.vertices().get(vertices[index].index()) else {
                    continue;
                };
                let Some(end) = arrangement
                    .vertices()
                    .get(vertices[(index + 1) % vertices.len()].index())
                else {
                    continue;
                };
                if !arrangement_key_lies_on_segment(key, start.key(), end.key()) {
                    continue;
                }
                let Some(parameter) = boundary_segment_parameter_xz(
                    arrangement_key_boundary_point(key, 0),
                    arrangement_key_boundary_point(start.key(), start.height_mm()),
                    arrangement_key_boundary_point(end.key(), end.height_mm()),
                ) else {
                    continue;
                };
                heights.push(interpolated_segment_height_mm(
                    arrangement_key_boundary_point(start.key(), start.height_mm()),
                    arrangement_key_boundary_point(end.key(), end.height_mm()),
                    parameter,
                ));
            }
        }
        heights.sort_unstable();
        heights.dedup();
        heights
    }

    fn raised_step_face_polygons_from_arrangement(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) -> Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)> {
        let mut emitted = BTreeSet::new();
        let mut faces = Vec::new();
        for (step_index, segment) in explicit_vertical_step_segments.iter().copied().enumerate() {
            let Some((lower_owner, raised_owner)) =
                canonical_vertical_step_lower_and_raised_owners(segment)
            else {
                continue;
            };
            let segment_key = (segment.start(), segment.end());
            let lower_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                lower_owner,
                segment_key,
            );
            let raised_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                raised_owner,
                segment_key,
            );
            Self::push_arrangement_vertical_step_faces_from_intervals(
                arrangement,
                lower_owner,
                segment_key,
                segment_key,
                &lower_intervals,
                &raised_intervals,
                step_index,
                segment,
                &mut emitted,
                &mut faces,
            );
            if let Some((dedup_key, face)) = Self::arrangement_vertical_step_face_from_segment(
                arrangement,
                lower_owner,
                raised_owner,
                segment_key,
            ) {
                if emitted.insert(dedup_key) {
                    faces.push((
                        face,
                        RoadSurfaceVerticalFaceSource::CanonicalStep {
                            explicit_vertical_step_index: step_index,
                            segment,
                        },
                    ));
                }
            }
        }
        faces
    }

    fn push_arrangement_vertical_step_faces_from_intervals(
        arrangement: &NodeArrangement,
        lower_owner: NodeBandOwner,
        lower_segment_key: (NodeArrangementKey, NodeArrangementKey),
        raised_segment_key: (NodeArrangementKey, NodeArrangementKey),
        lower_intervals: &[ArrangementFaceBoundaryInterval],
        raised_intervals: &[ArrangementFaceBoundaryInterval],
        step_index: usize,
        segment: NodeExplicitVerticalStepSegment,
        emitted: &mut BTreeSet<(
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        )>,
        faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    ) {
        for (lower_interval, raised_interval, start_t, end_t) in
            arrangement_shared_face_boundary_intervals(lower_intervals, raised_intervals)
        {
            let Some(lower_start) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(lower_end) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                end_t,
            ) else {
                continue;
            };
            let Some(raised_start) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(raised_end) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                end_t,
            ) else {
                continue;
            };
            let Some((dedup_key, face)) = Self::arrangement_vertical_step_face_polygon(
                arrangement,
                lower_owner,
                lower_segment_key,
                lower_start,
                lower_end,
                raised_start,
                raised_end,
            ) else {
                continue;
            };
            if !emitted.insert(dedup_key) {
                continue;
            }
            faces.push((
                face,
                RoadSurfaceVerticalFaceSource::CanonicalStep {
                    explicit_vertical_step_index: step_index,
                    segment,
                },
            ));
        }
    }

    fn arrangement_vertical_step_face_from_segment(
        arrangement: &NodeArrangement,
        lower_owner: NodeBandOwner,
        raised_owner: NodeBandOwner,
        segment_key: (NodeArrangementKey, NodeArrangementKey),
    ) -> Option<(
        (
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        ),
        RoadSurfaceVisualPolygon,
    )> {
        let lower_start = arrangement_owner_boundary_point_at_key(
            arrangement,
            lower_owner,
            segment_key.0,
            false,
        )?;
        let lower_end = arrangement_owner_boundary_point_at_key(
            arrangement,
            lower_owner,
            segment_key.1,
            false,
        )?;
        let raised_start = arrangement_owner_boundary_point_at_key(
            arrangement,
            raised_owner,
            segment_key.0,
            true,
        )?;
        let raised_end = arrangement_owner_boundary_point_at_key(
            arrangement,
            raised_owner,
            segment_key.1,
            true,
        )?;
        Self::arrangement_vertical_step_face_polygon(
            arrangement,
            lower_owner,
            segment_key,
            lower_start,
            lower_end,
            raised_start,
            raised_end,
        )
    }

    fn arrangement_vertical_step_face_polygon(
        arrangement: &NodeArrangement,
        lower_owner: NodeBandOwner,
        segment_key: (NodeArrangementKey, NodeArrangementKey),
        lower_start: Vector3,
        lower_end: Vector3,
        raised_start: Vector3,
        raised_end: Vector3,
    ) -> Option<(
        (
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        ),
        RoadSurfaceVisualPolygon,
    )> {
        let lower_span_xz = Vector2::new(lower_end.x - lower_start.x, lower_end.z - lower_start.z);
        let raised_span_xz =
            Vector2::new(raised_end.x - raised_start.x, raised_end.z - raised_start.z);
        if lower_span_xz.length_squared() <= VERTICAL_STEP_MIN_SPAN_M * VERTICAL_STEP_MIN_SPAN_M
            || raised_span_xz.length_squared()
                <= VERTICAL_STEP_MIN_SPAN_M * VERTICAL_STEP_MIN_SPAN_M
        {
            return None;
        }
        if (raised_start.y - lower_start.y <= SAMPLE_EPSILON_M)
            && (raised_end.y - lower_end.y <= SAMPLE_EPSILON_M)
        {
            return None;
        }
        let dedup_key = vertical_face_dedup_key(lower_start, lower_end, raised_start, raised_end);
        let mut points = [raised_start, lower_start, lower_end, raised_end];
        if let Some(visible_dot) = arrangement_vertical_face_visible_dot_to_owner(
            arrangement,
            lower_owner,
            segment_key,
            points,
        ) {
            if visible_dot <= 0.0 {
                points = [points[3], points[2], points[1], points[0]];
            }
        } else {
            let lower_owner_direction = arrangement_owner_direction_for_segment(
                arrangement,
                lower_owner,
                segment_key,
                lower_start,
                lower_end,
            )
            .unwrap_or_else(|| {
                let edge_direction = lower_end - lower_start;
                Vector3::new(-edge_direction.z, 0.0, edge_direction.x)
            });
            let face_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            if face_normal.dot(lower_owner_direction) > 0.0 {
                points = [points[3], points[2], points[1], points[0]];
            }
        }
        Self::make_vertical_quad_polygon(points).map(|face| (dedup_key, face))
    }

    fn visible_top_polygons_from_owned_regions(
        owned_regions: &[NodeOwnedRegion],
    ) -> (
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceVisualPolygon>,
    ) {
        let mut road_surface_polygons = Vec::new();
        let mut curb_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        for region in owned_regions {
            match region.kind {
                RoadSurfaceBandKind::Carriageway => {
                    road_surface_polygons.push(region.polygon.clone())
                }
                RoadSurfaceBandKind::CurbOrShoulder => {
                    curb_surface_polygons.push(region.polygon.clone());
                }
                _ => sidewalk_surface_polygons.push(region.polygon.clone()),
            }
        }
        (
            road_surface_polygons,
            curb_surface_polygons,
            sidewalk_surface_polygons,
        )
    }

    fn outer_boundary_polygons_from_point_loops(
        point_loops: &[Vec<Vector3>],
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, NodeBoundaryExportError> {
        let mut polygons = Vec::new();
        for point_loop in point_loops {
            let points = Self::canonicalize_loop_points(point_loop.clone());
            if points.len() < 3 {
                continue;
            }
            let area_m2 = Self::signed_polygon_area_xz(&points).abs();
            if area_m2 <= boundary_points_numeric_area_budget_m2(&points) {
                continue;
            }
            let Some(polygon) = Self::make_boundary_loop_polygon_preserving_winding(points) else {
                return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
            };
            polygons.push(polygon);
        }
        (!polygons.is_empty())
            .then_some(polygons)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }

    fn terrain_clip_boundary_loops_from_earthwork_segments(
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
                        super::RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                            owner_kind,
                            ..
                        } => terrain_clip_edge_kind_for_band(owner_kind),
                        super::RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. } => {
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

    fn node_earthwork_boundary_segments_from_footprint_loops(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        footprint_loops: &[Vec<Vector3>],
        owned_regions: &[NodeOwnedRegion],
    ) -> Result<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>, NodeBoundaryExportError> {
        let source_edges = Self::node_earthwork_boundary_source_edges_from_owned_regions(
            node_id,
            kind,
            owned_regions,
        );
        if source_edges.is_empty() {
            return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
        }

        let mut loops = Vec::new();
        for footprint_loop in footprint_loops {
            for points in same_winding_boundary_point_loops_from_loop(footprint_loop) {
                let mut segments = Vec::new();
                for index in 0..points.len() {
                    Self::push_sourced_node_earthwork_boundary_segments(
                        node_id,
                        kind,
                        points[index],
                        points[(index + 1) % points.len()],
                        &source_edges,
                        owned_regions,
                        &mut segments,
                    )?;
                }
                if segments.len() >= 3 {
                    loops.push(segments);
                }
            }
        }

        (!loops.is_empty())
            .then_some(loops)
            .ok_or(NodeBoundaryExportError::MissingEarthworkBoundarySource)
    }

    fn node_earthwork_boundary_source_edges_from_owned_regions(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        owned_regions: &[NodeOwnedRegion],
    ) -> Vec<NodeEarthworkBoundarySourceEdge> {
        let mut source_edges = Vec::new();
        for region in owned_regions {
            let points = &region.polygon.points_world;
            if points.len() < 3 {
                continue;
            }
            let source = RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id,
                kind,
                owner_kind: region.kind,
                owner_index: region.owner_index,
            };
            for index in 0..points.len() {
                let start_key = ArrangementBoundaryPointKey::from_world(points[index]).xz_key();
                let end_key =
                    ArrangementBoundaryPointKey::from_world(points[(index + 1) % points.len()])
                        .xz_key();
                if start_key == end_key {
                    continue;
                }
                source_edges.push(NodeEarthworkBoundarySourceEdge {
                    start_key,
                    end_key,
                    source,
                });
            }
        }
        source_edges.sort_by(|a, b| {
            Self::node_earthwork_boundary_source_ordering(a.source, b.source)
                .then(a.start_key.cmp(&b.start_key))
                .then(a.end_key.cmp(&b.end_key))
        });
        source_edges
    }

    fn push_sourced_node_earthwork_boundary_segments(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        start: Vector3,
        end: Vector3,
        source_edges: &[NodeEarthworkBoundarySourceEdge],
        owned_regions: &[NodeOwnedRegion],
        segments: &mut Vec<RoadSurfaceEarthworkBoundarySegment>,
    ) -> Result<(), NodeBoundaryExportError> {
        let start_key = ArrangementBoundaryPointKey::from_world(start).xz_key();
        let end_key = ArrangementBoundaryPointKey::from_world(end).xz_key();
        if start_key == end_key {
            return Ok(());
        }
        let mut split_points = BTreeMap::<ArrangementSegmentParameter, Vector3>::new();
        split_points.insert(
            ArrangementSegmentParameter {
                numerator: 0,
                denominator: 1,
            },
            start,
        );
        split_points.insert(
            ArrangementSegmentParameter {
                numerator: 1,
                denominator: 1,
            },
            end,
        );
        for source_edge in source_edges {
            for split_key in [source_edge.start_key, source_edge.end_key] {
                if !arrangement_key_lies_on_segment(split_key, start_key, end_key) {
                    continue;
                }
                let Some(parameter) =
                    arrangement_key_segment_parameter_xz(split_key, start_key, end_key)
                else {
                    continue;
                };
                if parameter
                    <= (ArrangementSegmentParameter {
                        numerator: 0,
                        denominator: 1,
                    })
                    || parameter
                        >= (ArrangementSegmentParameter {
                            numerator: 1,
                            denominator: 1,
                        })
                {
                    continue;
                }
                split_points.entry(parameter).or_insert_with(|| {
                    let t = parameter.numerator as f32 / parameter.denominator as f32;
                    start + (end - start) * t
                });
            }
        }

        let ordered_points = split_points.into_iter().collect::<Vec<_>>();
        for pair in ordered_points.windows(2) {
            let sub_start = pair[0].1;
            let sub_end = pair[1].1;
            if Vector2::new(sub_end.x - sub_start.x, sub_end.z - sub_start.z).length_squared()
                <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
            {
                continue;
            }
            let sub_start_key = ArrangementBoundaryPointKey::from_world(sub_start).xz_key();
            let sub_end_key = ArrangementBoundaryPointKey::from_world(sub_end).xz_key();
            let source = Self::node_earthwork_source_for_boundary_subsegment(
                sub_start_key,
                sub_end_key,
                source_edges,
            )
            .or_else(|| {
                Self::node_earthwork_source_for_owned_boundary_subsegment(
                    node_id,
                    kind,
                    sub_start,
                    sub_end,
                    owned_regions,
                )
            });
            let Some(source) = source else {
                return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
            };
            segments.push(RoadSurfaceEarthworkBoundarySegment {
                inner_start: sub_start,
                inner_end: sub_end,
                source,
            });
        }
        Ok(())
    }

    fn node_earthwork_source_for_boundary_subsegment(
        start_key: NodeArrangementKey,
        end_key: NodeArrangementKey,
        source_edges: &[NodeEarthworkBoundarySourceEdge],
    ) -> Option<RoadSurfaceEarthworkFaceSource> {
        source_edges
            .iter()
            .filter(|source_edge| {
                arrangement_key_lies_on_segment(
                    start_key,
                    source_edge.start_key,
                    source_edge.end_key,
                ) && arrangement_key_lies_on_segment(
                    end_key,
                    source_edge.start_key,
                    source_edge.end_key,
                )
            })
            .map(|source_edge| source_edge.source)
            .min_by(|a, b| Self::node_earthwork_boundary_source_ordering(*a, *b))
    }

    fn node_earthwork_source_for_owned_boundary_subsegment(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        start: Vector3,
        end: Vector3,
        owned_regions: &[NodeOwnedRegion],
    ) -> Option<RoadSurfaceEarthworkFaceSource> {
        let midpoint = (start + end) * 0.5;
        let start_xz = Vector2::new(start.x, start.z);
        let midpoint_xz = Vector2::new(midpoint.x, midpoint.z);
        let end_xz = Vector2::new(end.x, end.z);
        owned_regions
            .iter()
            .filter(|region| {
                RoadSurfaceSystem::polygon_contains_point_xz(&region.polygon.points_world, start_xz)
                    && RoadSurfaceSystem::polygon_contains_point_xz(
                        &region.polygon.points_world,
                        midpoint_xz,
                    )
                    && RoadSurfaceSystem::polygon_contains_point_xz(
                        &region.polygon.points_world,
                        end_xz,
                    )
            })
            .map(
                |region| RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    node_id,
                    kind,
                    owner_kind: region.kind,
                    owner_index: region.owner_index,
                },
            )
            .min_by(|a, b| Self::node_earthwork_boundary_source_ordering(*a, *b))
    }

    fn node_earthwork_boundary_source_ordering(
        a: RoadSurfaceEarthworkFaceSource,
        b: RoadSurfaceEarthworkFaceSource,
    ) -> std::cmp::Ordering {
        match (a, b) {
            (
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    owner_kind: owner_kind_a,
                    owner_index: owner_index_a,
                    ..
                },
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    owner_kind: owner_kind_b,
                    owner_index: owner_index_b,
                    ..
                },
            ) => Self::band_kind_sort_key(owner_kind_a)
                .cmp(&Self::band_kind_sort_key(owner_kind_b))
                .then(owner_index_a.cmp(&owner_index_b)),
            (
                RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. },
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { .. },
            ) => std::cmp::Ordering::Less,
            (
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { .. },
                RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. },
            ) => std::cmp::Ordering::Greater,
            (
                RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. },
                RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. },
            ) => std::cmp::Ordering::Equal,
        }
    }

    fn visual_polygon_from_arrangement_face(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
        authority_indices: &BTreeMap<super::node_grade::NodeGradeVertexAuthority, usize>,
    ) -> Result<
        Option<(RoadSurfaceVisualPolygon, NodeTopSurfacePolygonSource)>,
        NodeBoundaryExportError,
    > {
        let Some((triangle, vertex_ids)) =
            Self::arrangement_face_visual_triangle_with_vertices(arrangement, face)
        else {
            return Ok(None);
        };
        let Some(region) = arrangement.regions().get(face.region().index()) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        let mut vertex_sources = Vec::with_capacity(vertex_ids.len());
        for vertex_id in vertex_ids {
            let Some(vertex) = arrangement.vertices().get(vertex_id.index()) else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            let Some(grade_authority_index) =
                authority_indices.get(&vertex.grade_authority()).copied()
            else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            vertex_sources.push(NodeTopSurfaceVertexSource {
                grade_authority_index,
            });
        }
        let triangle_sources = vec![[vertex_sources[0], vertex_sources[1], vertex_sources[2]]];
        let source = NodeTopSurfacePolygonSource {
            kind: face.owner().kind(),
            owner_index: face.owner().owner_index(),
            height_field_id: region.height_field_id(),
            vertex_sources,
            triangle_sources,
        };
        Ok(Some((
            RoadSurfaceVisualPolygon {
                points_world: triangle.to_vec(),
                triangles_world: vec![triangle],
            },
            source,
        )))
    }

    fn arrangement_face_visual_triangle(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
    ) -> Option<[Vector3; 3]> {
        Self::arrangement_face_visual_triangle_with_vertices(arrangement, face)
            .map(|(triangle, _)| triangle)
    }

    fn arrangement_face_visual_triangle_with_vertices(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
    ) -> Option<(
        [Vector3; 3],
        [super::arrangement::NodeArrangementVertexId; 3],
    )> {
        let mut vertices = face.vertices();
        let mut triangle = [
            Self::arrangement_vertex_world(arrangement, vertices[0])?,
            Self::arrangement_vertex_world(arrangement, vertices[1])?,
            Self::arrangement_vertex_world(arrangement, vertices[2])?,
        ];
        let has_area = Self::signed_polygon_area_xz(&triangle).abs() > NODE_OVERLAY_MIN_AREA_M2;
        let area_3d_m2 = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .length()
            * 0.5;
        if !has_area || area_3d_m2 < NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if Self::signed_polygon_area_xz(&triangle) < 0.0 {
            triangle.swap(1, 2);
            vertices.swap(1, 2);
        }
        Some((triangle, vertices))
    }

    fn arrangement_vertex_world(
        arrangement: &NodeArrangement,
        vertex_id: super::arrangement::NodeArrangementVertexId,
    ) -> Option<Vector3> {
        let vertex = arrangement.vertices().get(vertex_id.index())?;
        Some(super::backend::road_xz_with_height_to_godot(
            vertex.point_xz(),
            vertex.height_m(),
        ))
    }

    fn log_node_input_extraction_error(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeInputExtractionError,
    ) {
        if !self.node_validation_logging_enabled {
            return;
        }
        crate::debug_log!(
            "road",
            "node_canonical_input_failed node={} piece={:?} error={:?}",
            node_id,
            kind,
            error
        );
    }

    fn log_node_validation_report(&self, report: &NodeValidationReport) {
        if !self.node_validation_logging_enabled || report.diagnostics.is_empty() {
            return;
        }
        crate::debug_log!("road", "node_canonical_validation {}", report.debug_dump());
    }

    fn log_node_boundary_export_error(
        &self,
        arrangement: &NodeArrangement,
        error: &NodeBoundaryExportError,
    ) {
        if !self.node_validation_logging_enabled {
            return;
        }
        let report = match error {
            NodeBoundaryExportError::MissingFootprintBoundaryHeight => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_footprint_boundary_height",
                )
            }
            NodeBoundaryExportError::EmptyOuterBoundary => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "empty_outer_boundary",
                )
            }
            NodeBoundaryExportError::DegenerateOuterBoundaryLoop => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "degenerate_outer_boundary_loop",
                )
            }
            NodeBoundaryExportError::MissingEarthworkBoundarySource => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_earthwork_boundary_source",
                )
            }
            NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_node_top_surface_grade_authority",
                )
            }
        };
        self.log_node_validation_report(&report);
    }

    pub(super) fn build_ordered_piece_mouths(
        &self,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<Vec<OrderedIncidentPieceMouth>> {
        let mut mouths = Vec::with_capacity(incidents.len());
        for &incident in incidents {
            let profile = self.build_incident_mouth_profile(incident)?;
            let endpoint_profile = self.build_incident_endpoint_profile(incident)?;
            let (
                boundary_paths_world,
                band_start_paths_world,
                band_end_paths_world,
                uses_sampled_band_domain_paths,
            ) = self.build_incident_mouth_paths(incident, &profile, &endpoint_profile);
            mouths.push(OrderedIncidentPieceMouth {
                profile,
                endpoint_profile,
                boundary_paths_world,
                band_start_paths_world,
                band_end_paths_world,
                uses_sampled_band_domain_paths,
                direction_angle_ccw: Self::normalized_angle_ccw(incident.direction_xz),
                direction_xz: incident.direction_xz,
                edge_idx: incident.edge_idx,
                side: incident.side,
            });
        }
        mouths.sort_by(|a, b| {
            a.direction_angle_ccw
                .total_cmp(&b.direction_angle_ccw)
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        Some(mouths)
    }

    fn incident_edge_visual_handoff_is_overconstrained(
        &self,
        graph: &RegionGraph,
        incident: IncidentSurfaceEdge,
    ) -> bool {
        if incident.edge_idx >= graph.edge_count() {
            return true;
        }
        let edge = graph.edge(incident.edge_idx);
        let Some(piece) = self.compiled_visual_span_pieces.get(&incident.edge_idx) else {
            return true;
        };
        let Some(sections) = self.compiled_sections.get(&incident.edge_idx) else {
            return true;
        };
        let Some(total_length_m) = sections.last().map(|section| section.s_m) else {
            return true;
        };
        if total_length_m <= SAMPLE_EPSILON_M {
            return true;
        }
        let has_current_mouth_profile = match incident.side {
            IncidentEdgeSide::Start => piece.start_mouth_profile.is_some(),
            IncidentEdgeSide::End => piece.end_mouth_profile.is_some(),
        };
        if !has_current_mouth_profile {
            return true;
        }

        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let Some((start_handoff_s_m, end_handoff_s_m)) = self
            .visual_surface_handoff_range_for_edge(
                graph,
                incident.edge_idx,
                edge,
                total_length_m,
                start_kind,
                end_kind,
            )
        else {
            return true;
        };
        let actual_handoff_m = match incident.side {
            IncidentEdgeSide::Start => start_handoff_s_m,
            IncidentEdgeSide::End => total_length_m - end_handoff_s_m,
        };
        let opposite_handoff_m = match incident.side {
            IncidentEdgeSide::Start => total_length_m - end_handoff_s_m,
            IncidentEdgeSide::End => start_handoff_s_m,
        };
        let span_remaining_m = end_handoff_s_m - start_handoff_s_m;
        span_remaining_m <= VISUAL_MIN_SPAN_LENGTH_M + SAMPLE_EPSILON_M
            && actual_handoff_m > opposite_handoff_m * VISUAL_DOMINANT_HANDOFF_REJECTION_RATIO
    }

    fn build_incident_mouth_paths(
        &self,
        incident: IncidentSurfaceEdge,
        profile: &IncidentMouthProfile,
        endpoint_profile: &IncidentMouthProfile,
    ) -> (
        Vec<Vec<Vector3>>,
        Vec<Vec<Vector3>>,
        Vec<Vec<Vector3>>,
        bool,
    ) {
        let Some(sections) = self.compiled_sections.get(&incident.edge_idx) else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };
        let Some(mouth_index) = sections.iter().enumerate().find_map(|(index, section)| {
            let candidate = Self::build_mouth_profile_from_section(section, incident.side)?;
            incident_mouth_profiles_match(&candidate, profile).then_some(index)
        }) else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };

        let section_indices: Vec<usize> = match incident.side {
            IncidentEdgeSide::Start => (0..=mouth_index).rev().collect(),
            IncidentEdgeSide::End => (mouth_index..sections.len()).collect(),
        };
        let mut profile_path = Vec::with_capacity(section_indices.len());
        for section_index in section_indices {
            let Some(path_profile) =
                Self::build_mouth_profile_from_section(&sections[section_index], incident.side)
            else {
                return (Vec::new(), Vec::new(), Vec::new(), false);
            };
            if !incident_mouth_profiles_have_same_shape(profile, &path_profile) {
                return (Vec::new(), Vec::new(), Vec::new(), false);
            }
            profile_path.push(path_profile);
        }

        let Some(last_profile) = profile_path.last() else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };
        if !incident_mouth_profiles_match(last_profile, endpoint_profile) {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        }

        let uses_sampled_band_domain_paths =
            incident_profile_path_has_non_collinear_center(&profile_path);
        let boundary_paths_world = (0..profile.boundary_points_world.len())
            .map(|boundary_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.boundary_points_world[boundary_index]),
                )
            })
            .collect();
        let band_start_paths_world = (0..profile.bands.len())
            .map(|band_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.bands[band_index].start_point_world),
                )
            })
            .collect();
        let band_end_paths_world = (0..profile.bands.len())
            .map(|band_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.bands[band_index].end_point_world),
                )
            })
            .collect();
        (
            boundary_paths_world,
            band_start_paths_world,
            band_end_paths_world,
            uses_sampled_band_domain_paths,
        )
    }

    fn build_incident_mouth_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let piece = self.compiled_visual_span_pieces.get(&incident.edge_idx)?;
        match incident.side {
            IncidentEdgeSide::Start => piece.start_mouth_profile.clone(),
            IncidentEdgeSide::End => piece.end_mouth_profile.clone(),
        }
    }

    fn build_incident_endpoint_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let sections = self.compiled_sections.get(&incident.edge_idx)?;
        let section = match incident.side {
            IncidentEdgeSide::Start => sections.first()?,
            IncidentEdgeSide::End => sections.last()?,
        };
        Self::build_mouth_profile_from_section(section, incident.side)
    }

    fn normalized_angle_ccw(direction_xz: Vector2) -> f32 {
        let angle = direction_xz.y.atan2(direction_xz.x);
        if angle < 0.0 {
            angle + std::f32::consts::TAU
        } else {
            angle
        }
    }

    #[cfg(test)]
    pub(super) fn left_normal_xz(direction_xz: Vector2) -> Vector2 {
        Vector2::new(-direction_xz.y, direction_xz.x)
    }

    fn assemble_explicit_node_piece(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
        mut road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut raised_step_faces: Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
        mut sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        explicit_vertical_step_segments: Vec<NodeExplicitVerticalStepSegment>,
        node_grade_authorities: Vec<super::node_grade::NodeGradeVertexAuthority>,
        mut node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
        mut owned_regions: Vec<NodeOwnedRegion>,
        mut earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if road_surface_polygons.is_empty()
            && curb_surface_polygons.is_empty()
            && sidewalk_surface_polygons.is_empty()
        {
            return None;
        }
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_raised_step_faces(&mut raised_step_faces);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        if node_top_surface_sources.len() != owned_regions.len() {
            return None;
        }
        Self::sort_node_owned_regions_with_sources(
            &mut owned_regions,
            &mut node_top_surface_sources,
        )
        .ok()?;
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        let (raised_step_face_polygons, raised_step_face_sources) =
            raised_step_faces.into_iter().unzip();
        Some(RoadSurfaceVisualNodePiece {
            node_id,
            kind,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            raised_step_face_polygons,
            raised_step_face_sources,
            sidewalk_surface_polygons,
            explicit_vertical_step_segments,
            node_grade_authorities,
            node_top_surface_sources,
            owned_regions,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }

    pub(super) fn classify_visual_node_kind(
        &self,
        incidents: &[IncidentSurfaceEdge],
    ) -> CompiledNodeKind {
        match incidents.len() {
            0 | 1 => CompiledNodeKind::Terminal,
            2 => {
                let a = incidents[0];
                let b = incidents[1];
                let straight = a.direction_xz.dot(b.direction_xz) <= -PASS_THROUGH_DOT_THRESHOLD;
                if !straight {
                    return CompiledNodeKind::Bend;
                }
                CompiledNodeKind::PassThrough
            }
            _ => CompiledNodeKind::JunctionN,
        }
    }

    pub(super) fn classify_surface_node_kind_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Option<CompiledNodeKind> {
        let incidents = self.sorted_incident_surface_edges_from_graph_geometry(graph, node_id);
        (!incidents.is_empty()).then(|| self.classify_visual_node_kind(&incidents))
    }

    pub(super) fn sorted_incident_surface_edges_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        let mut incidents = self.collect_incident_surface_edges_from_graph_geometry(graph, node_id);
        incidents.sort_by(|a, b| {
            Self::normalized_angle_ccw(a.direction_xz)
                .total_cmp(&Self::normalized_angle_ccw(b.direction_xz))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        incidents
    }

    fn collect_incident_surface_edges_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }

        let mut incidents = Vec::new();
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                continue;
            }

            let side = if graph.get_valid_node(edge.start_node) == node_id {
                Some(IncidentEdgeSide::Start)
            } else if graph.get_valid_node(edge.end_node) == node_id {
                Some(IncidentEdgeSide::End)
            } else {
                None
            };
            let Some(side) = side else {
                continue;
            };
            let Some(direction_xz) = self.incident_direction_from_edge_geometry(edge, side) else {
                continue;
            };
            incidents.push(IncidentSurfaceEdge {
                edge_idx,
                side,
                direction_xz,
            });
        }

        incidents.sort_by(|a, b| a.edge_idx.cmp(&b.edge_idx).then(a.side.cmp(&b.side)));
        incidents
    }

    fn incident_direction_from_edge_geometry(
        &self,
        edge: &Edge,
        side: IncidentEdgeSide,
    ) -> Option<Vector2> {
        let points = self.edge_points(edge);
        if points.len() < 2 {
            return None;
        }

        match side {
            IncidentEdgeSide::Start => {
                let endpoint = points[0];
                points.iter().skip(1).find_map(|point| {
                    let direction = Vector2::new(point.x - endpoint.x, point.z - endpoint.z);
                    (direction.length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
                        .then(|| direction.normalized())
                })
            }
            IncidentEdgeSide::End => {
                let endpoint = *points.last()?;
                points.iter().rev().skip(1).find_map(|point| {
                    let direction = Vector2::new(point.x - endpoint.x, point.z - endpoint.z);
                    (direction.length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
                        .then(|| direction.normalized())
                })
            }
        }
    }

    pub(super) fn sorted_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        let mut incidents = self.collect_incident_surface_edges(graph, node_id);
        incidents.sort_by(|a, b| {
            Self::normalized_angle_ccw(a.direction_xz)
                .total_cmp(&Self::normalized_angle_ccw(b.direction_xz))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        incidents
    }

    fn collect_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }

        let mut incidents = Vec::new();
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                continue;
            }

            let side = if graph.get_valid_node(edge.start_node) == node_id {
                Some(IncidentEdgeSide::Start)
            } else if graph.get_valid_node(edge.end_node) == node_id {
                Some(IncidentEdgeSide::End)
            } else {
                None
            };
            let Some(side) = side else {
                continue;
            };
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let Some(direction_xz) = (match side {
                IncidentEdgeSide::Start => piece
                    .start_mouth_profile
                    .as_ref()
                    .map(|mouth| mouth.inward_direction_xz),
                IncidentEdgeSide::End => piece
                    .end_mouth_profile
                    .as_ref()
                    .map(|mouth| mouth.inward_direction_xz),
            }) else {
                continue;
            };
            incidents.push(IncidentSurfaceEdge {
                edge_idx,
                side,
                direction_xz,
            });
        }

        incidents.sort_by(|a, b| a.edge_idx.cmp(&b.edge_idx).then(a.side.cmp(&b.side)));
        incidents
    }
}

fn normalized_arrangement_boundary_segment_key(
    start: Vector3,
    end: Vector3,
) -> (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey) {
    let start = ArrangementBoundaryPointKey::from_world(start);
    let end = ArrangementBoundaryPointKey::from_world(end);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn vertical_face_dedup_key(
    lower_start: Vector3,
    lower_end: Vector3,
    upper_start: Vector3,
    upper_end: Vector3,
) -> (
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
) {
    (
        normalized_arrangement_boundary_segment_key(lower_start, lower_end),
        normalized_arrangement_boundary_segment_key(upper_start, upper_end),
    )
}

fn canonical_vertical_step_lower_and_raised_owners(
    segment: NodeExplicitVerticalStepSegment,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    if owner.kind() == lower_kind {
        Some((owner, opposite_owner))
    } else {
        Some((opposite_owner, owner))
    }
}

fn dedup_raised_step_faces(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
) {
    let mut emitted = BTreeSet::new();
    faces.retain(|(polygon, _)| {
        let Some(key) = raised_step_face_span_key(polygon) else {
            return true;
        };
        emitted.insert(key)
    });
}

fn push_missing_raised_step_faces_from_owned_region_boundaries(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
    explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
) {
    for (step_index, segment) in explicit_vertical_step_segments.iter().copied().enumerate() {
        let Some((lower_owner, raised_owner)) =
            canonical_vertical_step_lower_and_raised_owners(segment)
        else {
            continue;
        };
        for lower_edge in owned_region_boundary_edges_for_owner(owned_regions, lower_owner) {
            if !world_edge_lies_on_explicit_vertical_step_segment(lower_edge, segment) {
                continue;
            }
            for raised_edge in owned_region_boundary_edges_for_owner(owned_regions, raised_owner) {
                let Some(raised_edge) = clip_edge_to_reference_xz(raised_edge, lower_edge) else {
                    continue;
                };
                if (raised_edge[0].y - lower_edge[0].y <= SAMPLE_EPSILON_M)
                    && (raised_edge[1].y - lower_edge[1].y <= SAMPLE_EPSILON_M)
                {
                    continue;
                }
                let Some(face) = RoadSurfaceSystem::make_vertical_quad_polygon([
                    raised_edge[0],
                    lower_edge[0],
                    lower_edge[1],
                    raised_edge[1],
                ]) else {
                    continue;
                };
                faces.push((
                    face,
                    RoadSurfaceVerticalFaceSource::CanonicalStep {
                        explicit_vertical_step_index: step_index,
                        segment,
                    },
                ));
            }
        }
    }
}

fn push_missing_raised_step_faces_from_top_owner_boundaries(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
) {
    let mut edges_by_xz = BTreeMap::<
        (NodeArrangementKey, NodeArrangementKey),
        Vec<(NodeBandOwner, [Vector3; 2])>,
    >::new();
    for region in owned_regions {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        for edge in owned_region_boundary_edges(region) {
            let start = ArrangementBoundaryPointKey::from_world(edge[0]).xz_key();
            let end = ArrangementBoundaryPointKey::from_world(edge[1]).xz_key();
            if start == end {
                continue;
            }
            let key = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            edges_by_xz.entry(key).or_default().push((owner, edge));
        }
    }

    for (key, edges) in edges_by_xz {
        for (left_index, (left_owner, left_edge)) in edges.iter().copied().enumerate() {
            for (right_owner, right_edge) in edges.iter().copied().skip(left_index + 1) {
                let Some(segment) =
                    NodeExplicitVerticalStepSegment::new(key.0, key.1, left_owner, right_owner)
                else {
                    continue;
                };
                let Some((lower_owner, raised_owner)) =
                    canonical_vertical_step_lower_and_raised_owners(segment)
                else {
                    continue;
                };
                let (lower_edge, raised_edge) =
                    if left_owner == lower_owner && right_owner == raised_owner {
                        (left_edge, right_edge)
                    } else if right_owner == lower_owner && left_owner == raised_owner {
                        (right_edge, left_edge)
                    } else {
                        continue;
                    };
                let Some(raised_edge) = clip_edge_to_reference_xz(raised_edge, lower_edge) else {
                    continue;
                };
                if (raised_edge[0].y - lower_edge[0].y <= SAMPLE_EPSILON_M)
                    && (raised_edge[1].y - lower_edge[1].y <= SAMPLE_EPSILON_M)
                {
                    continue;
                }
                let Some(face) = RoadSurfaceSystem::make_vertical_quad_polygon([
                    raised_edge[0],
                    lower_edge[0],
                    lower_edge[1],
                    raised_edge[1],
                ]) else {
                    continue;
                };
                faces.push((
                    face,
                    RoadSurfaceVerticalFaceSource::FinalOwnedBoundary { segment },
                ));
            }
        }
    }
}

fn owned_region_boundary_edges_for_owner(
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
) -> Vec<[Vector3; 2]> {
    let mut edges = Vec::new();
    for region in owned_regions
        .iter()
        .filter(|region| node_owned_region_matches_owner(region, owner))
    {
        edges.extend(owned_region_boundary_edges(region));
    }
    edges
}

fn owned_region_boundary_edges(region: &NodeOwnedRegion) -> Vec<[Vector3; 2]> {
    let mut edges = Vec::new();
    if region.polygon.triangles_world.is_empty() {
        let points = &region.polygon.points_world;
        if points.len() < 2 {
            return edges;
        }
        for index in 0..points.len() {
            let start = points[index];
            let end = points[(index + 1) % points.len()];
            if ArrangementBoundaryPointKey::from_world(start).xz_key()
                == ArrangementBoundaryPointKey::from_world(end).xz_key()
            {
                continue;
            }
            edges.push([start, end]);
        }
        return edges;
    }

    let mut triangle_edges = BTreeMap::<
        (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        (usize, [Vector3; 2]),
    >::new();
    for triangle in &region.polygon.triangles_world {
        for edge_index in 0..3 {
            let start = triangle[edge_index];
            let end = triangle[(edge_index + 1) % 3];
            if ArrangementBoundaryPointKey::from_world(start).xz_key()
                == ArrangementBoundaryPointKey::from_world(end).xz_key()
            {
                continue;
            }
            triangle_edges
                .entry(normalized_arrangement_boundary_segment_key(start, end))
                .and_modify(|entry| entry.0 += 1)
                .or_insert((1, [start, end]));
        }
    }
    edges.extend(
        triangle_edges
            .into_values()
            .filter_map(|(count, edge)| (count == 1).then_some(edge)),
    );
    edges
}

fn world_edge_lies_on_explicit_vertical_step_segment(
    edge: [Vector3; 2],
    segment: NodeExplicitVerticalStepSegment,
) -> bool {
    let edge_start = ArrangementBoundaryPointKey::from_world(edge[0]).xz_key();
    let edge_end = ArrangementBoundaryPointKey::from_world(edge[1]).xz_key();
    arrangement_segments_exact_overlap_with_length(
        edge_start,
        edge_end,
        segment.start(),
        segment.end(),
    )
}

fn clip_edge_to_reference_xz(edge: [Vector3; 2], reference: [Vector3; 2]) -> Option<[Vector3; 2]> {
    let edge_start = ArrangementBoundaryPointKey::from_world(edge[0]);
    let edge_end = ArrangementBoundaryPointKey::from_world(edge[1]);
    let reference_start = ArrangementBoundaryPointKey::from_world(reference[0]);
    let reference_end = ArrangementBoundaryPointKey::from_world(reference[1]);
    if !arrangement_segments_exact_overlap_with_length(
        edge_start.xz_key(),
        edge_end.xz_key(),
        reference_start.xz_key(),
        reference_end.xz_key(),
    ) {
        return None;
    }
    let start_t = boundary_segment_parameter_xz(reference_start, edge_start, edge_end)?;
    let end_t = boundary_segment_parameter_xz(reference_end, edge_start, edge_end)?;
    if start_t < ArrangementSegmentParameter::zero()
        || start_t > ArrangementSegmentParameter::one()
        || end_t < ArrangementSegmentParameter::zero()
        || end_t > ArrangementSegmentParameter::one()
    {
        return None;
    }
    let point_at = |reference: ArrangementBoundaryPointKey, parameter| {
        arrangement_boundary_point_to_world(ArrangementBoundaryPointKey {
            x_key: reference.x_key,
            z_key: reference.z_key,
            y_mm: interpolated_segment_height_mm(edge_start, edge_end, parameter),
        })
    };
    Some([
        point_at(reference_start, start_t),
        point_at(reference_end, end_t),
    ])
}

fn retain_raised_step_faces_with_top_support(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
) {
    faces.retain(|(polygon, source)| {
        let Some((lower_owner, raised_owner)) =
            canonical_vertical_step_lower_and_raised_owners(source.segment())
        else {
            return false;
        };
        let Some((lower_edge, upper_edge)) = vertical_face_support_edges(polygon) else {
            return false;
        };
        owned_region_has_top_boundary_edge(owned_regions, lower_owner, lower_edge)
            && owned_region_has_top_boundary_edge(owned_regions, raised_owner, upper_edge)
    });
}

fn orient_raised_step_faces_to_lower_owner_support(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
) {
    for (polygon, source) in faces {
        let Some((lower_owner, _)) =
            canonical_vertical_step_lower_and_raised_owners(source.segment())
        else {
            continue;
        };
        let Some(lower_edge) =
            vertical_face_support_edge_for_owner(polygon, owned_regions, lower_owner)
        else {
            continue;
        };
        let Some(visible_dot) = vertical_face_visible_dot_to_supported_owner(
            polygon,
            lower_edge,
            owned_regions,
            lower_owner,
        ) else {
            continue;
        };
        if visible_dot > 0.0 {
            continue;
        }
        let Some(points) = reversed_vertical_face_points(polygon) else {
            continue;
        };
        if let Some(oriented) = RoadSurfaceSystem::make_vertical_quad_polygon(points) {
            *polygon = oriented;
        }
    }
}

fn owned_region_has_top_boundary_edge(
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
    edge: [Vector3; 2],
) -> bool {
    owned_regions
        .iter()
        .filter(|region| node_owned_region_matches_owner(region, owner))
        .any(|region| visual_polygon_boundary_overlaps_edge_at_height(&region.polygon, edge))
}

fn vertical_face_support_edge_for_owner(
    polygon: &RoadSurfaceVisualPolygon,
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
) -> Option<[Vector3; 2]> {
    vertical_face_side_edges(polygon).and_then(|edges| {
        edges
            .into_iter()
            .find(|edge| owned_region_has_top_boundary_edge_xz(owned_regions, owner, *edge))
    })
}

fn owned_region_has_top_boundary_edge_xz(
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
    edge: [Vector3; 2],
) -> bool {
    owned_regions
        .iter()
        .filter(|region| node_owned_region_matches_owner(region, owner))
        .any(|region| visual_polygon_boundary_overlaps_edge_xz(&region.polygon, edge))
}

fn node_owned_region_matches_owner(region: &NodeOwnedRegion, owner: NodeBandOwner) -> bool {
    region.kind == owner.kind() && region.owner_index == owner.owner_index()
}

fn vertical_face_visible_dot_to_supported_owner(
    polygon: &RoadSurfaceVisualPolygon,
    lower_edge: [Vector3; 2],
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
) -> Option<f32> {
    let visible_direction = vertical_face_visible_direction(polygon)?;
    let midpoint = (lower_edge[0] + lower_edge[1]) * 0.5;
    let mut best_dot: Option<f32> = None;
    for region in owned_regions
        .iter()
        .filter(|region| node_owned_region_matches_owner(region, owner))
    {
        if !visual_polygon_boundary_overlaps_edge_xz(&region.polygon, lower_edge) {
            continue;
        }
        let Some(centroid) = visual_polygon_centroid(&region.polygon) else {
            continue;
        };
        let owner_direction = Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
        if owner_direction.length_squared() <= 1e-8 {
            continue;
        }
        let dot = visible_direction.dot(owner_direction.normalized());
        best_dot = Some(best_dot.map_or(dot, |current| current.max(dot)));
    }
    best_dot
}

fn vertical_face_visible_direction(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let [upper_start, lower_start, lower_end, _upper_end] = polygon.points_world.as_slice() else {
        return None;
    };
    let normal = (*lower_start - *upper_start).cross(*lower_end - *upper_start);
    let visible_direction = Vector3::new(-normal.x, 0.0, -normal.z);
    if visible_direction.length_squared() <= 1e-8 {
        return None;
    }
    Some(visible_direction.normalized())
}

fn visual_polygon_centroid(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let mut sum = Vector3::ZERO;
    let mut count = 0usize;
    for point in &polygon.points_world {
        sum += Vector3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f32)
}

fn reversed_vertical_face_points(polygon: &RoadSurfaceVisualPolygon) -> Option<[Vector3; 4]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([*d, *c, *b, *a])
}

fn vertical_face_side_edges(polygon: &RoadSurfaceVisualPolygon) -> Option<[[Vector3; 2]; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([[*a, *d], [*b, *c]])
}

fn vertical_face_support_edges(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<([Vector3; 2], [Vector3; 2])> {
    let [first_edge, second_edge] = vertical_face_side_edges(polygon)?;
    let first_avg_y = (first_edge[0].y + first_edge[1].y) * 0.5;
    let second_avg_y = (second_edge[0].y + second_edge[1].y) * 0.5;
    if first_avg_y <= second_avg_y {
        Some((first_edge, second_edge))
    } else {
        Some((second_edge, first_edge))
    }
}

fn visual_polygon_boundary_overlaps_edge_at_height(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [Vector3; 2],
) -> bool {
    if !polygon.triangles_world.is_empty() {
        let mut triangle_edges = BTreeMap::<
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (usize, [Vector3; 2]),
        >::new();
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                if ArrangementBoundaryPointKey::from_world(start).xz_key()
                    == ArrangementBoundaryPointKey::from_world(end).xz_key()
                {
                    continue;
                }
                triangle_edges
                    .entry(normalized_arrangement_boundary_segment_key(start, end))
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, [start, end]));
            }
        }
        return triangle_edges
            .into_values()
            .filter_map(|(count, boundary_edge)| (count == 1).then_some(boundary_edge))
            .any(|boundary_edge| boundary_edge_contains_edge_at_height(boundary_edge, edge));
    }

    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        boundary_edge_contains_edge_at_height([start, end], edge)
    })
}

fn visual_polygon_boundary_overlaps_edge_xz(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [Vector3; 2],
) -> bool {
    if !polygon.triangles_world.is_empty() {
        let mut triangle_edges = BTreeMap::<
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (usize, [Vector3; 2]),
        >::new();
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                if ArrangementBoundaryPointKey::from_world(start).xz_key()
                    == ArrangementBoundaryPointKey::from_world(end).xz_key()
                {
                    continue;
                }
                triangle_edges
                    .entry(normalized_arrangement_boundary_segment_key(start, end))
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, [start, end]));
            }
        }
        return triangle_edges
            .into_values()
            .filter_map(|(count, boundary_edge)| (count == 1).then_some(boundary_edge))
            .any(|boundary_edge| boundary_edge_contains_edge_xz(boundary_edge, edge));
    }

    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        boundary_edge_contains_edge_xz([start, end], edge)
    })
}

fn boundary_edge_contains_edge_xz(boundary_edge: [Vector3; 2], edge: [Vector3; 2]) -> bool {
    let boundary_start = ArrangementBoundaryPointKey::from_world(boundary_edge[0]);
    let boundary_end = ArrangementBoundaryPointKey::from_world(boundary_edge[1]);
    let edge_start = ArrangementBoundaryPointKey::from_world(edge[0]);
    let edge_end = ArrangementBoundaryPointKey::from_world(edge[1]);
    if !arrangement_segments_exact_overlap_with_length(
        boundary_start.xz_key(),
        boundary_end.xz_key(),
        edge_start.xz_key(),
        edge_end.xz_key(),
    ) {
        return false;
    }
    let Some(start_parameter) =
        boundary_segment_parameter_xz(edge_start, boundary_start, boundary_end)
    else {
        return false;
    };
    let Some(end_parameter) = boundary_segment_parameter_xz(edge_end, boundary_start, boundary_end)
    else {
        return false;
    };
    start_parameter >= ArrangementSegmentParameter::zero()
        && start_parameter <= ArrangementSegmentParameter::one()
        && end_parameter >= ArrangementSegmentParameter::zero()
        && end_parameter <= ArrangementSegmentParameter::one()
}

fn boundary_edge_contains_edge_at_height(boundary_edge: [Vector3; 2], edge: [Vector3; 2]) -> bool {
    let boundary_start = ArrangementBoundaryPointKey::from_world(boundary_edge[0]);
    let boundary_end = ArrangementBoundaryPointKey::from_world(boundary_edge[1]);
    let edge_start = ArrangementBoundaryPointKey::from_world(edge[0]);
    let edge_end = ArrangementBoundaryPointKey::from_world(edge[1]);
    if !arrangement_segments_exact_overlap_with_length(
        boundary_start.xz_key(),
        boundary_end.xz_key(),
        edge_start.xz_key(),
        edge_end.xz_key(),
    ) {
        return false;
    }
    let Some(start_parameter) =
        boundary_segment_parameter_xz(edge_start, boundary_start, boundary_end)
    else {
        return false;
    };
    let Some(end_parameter) = boundary_segment_parameter_xz(edge_end, boundary_start, boundary_end)
    else {
        return false;
    };
    if start_parameter < ArrangementSegmentParameter::zero()
        || start_parameter > ArrangementSegmentParameter::one()
        || end_parameter < ArrangementSegmentParameter::zero()
        || end_parameter > ArrangementSegmentParameter::one()
    {
        return false;
    }
    (interpolated_segment_height_mm(boundary_start, boundary_end, start_parameter)
        - edge_start.y_mm)
        .abs()
        <= 1
        && (interpolated_segment_height_mm(boundary_start, boundary_end, end_parameter)
            - edge_end.y_mm)
            .abs()
            <= 1
}

fn arrangement_segments_exact_overlap_with_length(
    a_start: NodeArrangementKey,
    a_end: NodeArrangementKey,
    b_start: NodeArrangementKey,
    b_end: NodeArrangementKey,
) -> bool {
    if a_start == a_end || b_start == b_end {
        return false;
    }
    let a_dx = i128::from(a_end.x_key() - a_start.x_key());
    let a_dz = i128::from(a_end.z_key() - a_start.z_key());
    let b_dx = i128::from(b_end.x_key() - b_start.x_key());
    let b_dz = i128::from(b_end.z_key() - b_start.z_key());
    if a_dx * b_dz - a_dz * b_dx != 0 {
        return false;
    }
    if !arrangement_key_lies_exactly_on_segment(a_start, b_start, b_end)
        && !arrangement_key_lies_exactly_on_segment(a_end, b_start, b_end)
        && !arrangement_key_lies_exactly_on_segment(b_start, a_start, a_end)
        && !arrangement_key_lies_exactly_on_segment(b_end, a_start, a_end)
    {
        return false;
    }
    let use_x = (a_end.x_key() - a_start.x_key()).abs() >= (a_end.z_key() - a_start.z_key()).abs();
    let coordinate = |key: NodeArrangementKey| {
        if use_x { key.x_key() } else { key.z_key() }
    };
    let a0 = coordinate(a_start);
    let a1 = coordinate(a_end);
    let b0 = coordinate(b_start);
    let b1 = coordinate(b_end);
    a0.min(a1).max(b0.min(b1)) < a0.max(a1).min(b0.max(b1))
}

fn arrangement_key_lies_exactly_on_segment(
    point: NodeArrangementKey,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key() - start.x_key());
    let dz = i128::from(end.z_key() - start.z_key());
    let px = i128::from(point.x_key() - start.x_key());
    let pz = i128::from(point.z_key() - start.z_key());
    if px * dz - pz * dx != 0 {
        return false;
    }
    let dot = px * dx + pz * dz;
    let len_squared = dx * dx + dz * dz;
    dot >= 0 && dot <= len_squared
}

impl RoadSurfaceSystem {
    fn sort_raised_step_faces(
        faces: &mut [(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)],
    ) {
        faces.sort_by(
            |(left_polygon, left_source), (right_polygon, right_source)| {
                Self::visual_polygon_ordering(left_polygon, right_polygon)
                    .then(left_source.sort_key().cmp(&right_source.sort_key()))
            },
        );
    }
}

fn raised_step_face_span_key(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<(
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
)> {
    if polygon.points_world.len() != 4 {
        return None;
    }
    let mut span_edges = Vec::new();
    for index in 0..polygon.points_world.len() {
        let start = polygon.points_world[index];
        let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
        if ArrangementBoundaryPointKey::from_world(start).xz_key()
            != ArrangementBoundaryPointKey::from_world(end).xz_key()
        {
            span_edges.push((start, end, (start.y + end.y) * 0.5));
        }
    }
    if span_edges.len() != 2 {
        return None;
    }
    span_edges.sort_by(|a, b| a.2.total_cmp(&b.2));
    Some(vertical_face_dedup_key(
        span_edges[0].0,
        span_edges[0].1,
        span_edges[1].0,
        span_edges[1].1,
    ))
}

fn incident_mouth_profiles_match(
    left: &IncidentMouthProfile,
    right: &IncidentMouthProfile,
) -> bool {
    incident_mouth_profiles_have_same_shape(left, right)
        && left
            .boundary_points_world
            .iter()
            .zip(&right.boundary_points_world)
            .all(|(left, right)| {
                ArrangementBoundaryPointKey::from_world(*left)
                    == ArrangementBoundaryPointKey::from_world(*right)
            })
        && left.bands.iter().zip(&right.bands).all(|(left, right)| {
            ArrangementBoundaryPointKey::from_world(left.start_point_world)
                == ArrangementBoundaryPointKey::from_world(right.start_point_world)
                && ArrangementBoundaryPointKey::from_world(left.end_point_world)
                    == ArrangementBoundaryPointKey::from_world(right.end_point_world)
        })
}

fn incident_mouth_profiles_have_same_shape(
    left: &IncidentMouthProfile,
    right: &IncidentMouthProfile,
) -> bool {
    left.boundary_points_world.len() == right.boundary_points_world.len()
        && left.bands.len() == right.bands.len()
        && left
            .bands
            .iter()
            .zip(&right.bands)
            .all(|(left, right)| left.kind == right.kind)
}

fn incident_world_path(points: impl IntoIterator<Item = Vector3>) -> Vec<Vector3> {
    points.into_iter().collect()
}

fn incident_profile_path_has_non_collinear_center(path: &[IncidentMouthProfile]) -> bool {
    if path.len() <= 2 {
        return false;
    }
    let Some(start) = path.first().and_then(incident_profile_center_key) else {
        return false;
    };
    let Some(end) = path.last().and_then(incident_profile_center_key) else {
        return false;
    };
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key() - start.x_key());
    let dz = i128::from(end.z_key() - start.z_key());
    path[1..path.len() - 1].iter().any(|profile| {
        let Some(key) = incident_profile_center_key(profile) else {
            return false;
        };
        let px = i128::from(key.x_key() - start.x_key());
        let pz = i128::from(key.z_key() - start.z_key());
        px * dz - pz * dx != 0
    })
}

fn incident_profile_center_key(profile: &IncidentMouthProfile) -> Option<NodeArrangementKey> {
    let first = profile.boundary_points_world.first()?;
    let last = profile.boundary_points_world.last()?;
    let center = (*first + *last) * 0.5;
    Some(NodeArrangementKey::from_point(
        super::backend::godot_vec3_xz_to_road(center),
    ))
}

fn arrangement_owner_face_boundary_intervals_for_segment(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
) -> Vec<ArrangementFaceBoundaryInterval> {
    let mut intervals = Vec::new();
    for face in arrangement
        .faces()
        .iter()
        .filter(|face| face.owner() == owner)
        .filter(|face| {
            RoadSurfaceSystem::arrangement_face_visual_triangle(arrangement, face).is_some()
        })
    {
        let vertices = face.vertices();
        for index in 0..vertices.len() {
            let Some(edge_start) =
                arrangement_vertex_boundary_point_key(arrangement, vertices[index])
            else {
                continue;
            };
            let Some(edge_end) = arrangement_vertex_boundary_point_key(
                arrangement,
                vertices[(index + 1) % vertices.len()],
            ) else {
                continue;
            };
            if let Some((start, end)) =
                arrangement_face_boundary_overlap_interval(segment_key, edge_start, edge_end)
            {
                intervals.push(ArrangementFaceBoundaryInterval {
                    owner: face.owner(),
                    start,
                    end,
                    edge_start,
                    edge_end,
                });
            }
        }
    }
    intervals.sort();
    intervals.dedup();
    intervals
}

fn arrangement_vertex_boundary_point_key(
    arrangement: &NodeArrangement,
    vertex_id: super::arrangement::NodeArrangementVertexId,
) -> Option<ArrangementBoundaryPointKey> {
    let vertex = arrangement.vertices().get(vertex_id.index())?;
    Some(arrangement_key_boundary_point(
        vertex.key(),
        vertex.height_mm(),
    ))
}

fn arrangement_shared_face_boundary_intervals(
    lower_intervals: &[ArrangementFaceBoundaryInterval],
    raised_intervals: &[ArrangementFaceBoundaryInterval],
) -> Vec<(
    ArrangementFaceBoundaryInterval,
    ArrangementFaceBoundaryInterval,
    ArrangementSegmentParameter,
    ArrangementSegmentParameter,
)> {
    let mut shared = Vec::new();
    for lower in lower_intervals {
        for raised in raised_intervals {
            let start = lower.start.max(raised.start);
            let end = lower.end.min(raised.end);
            if end > start {
                shared.push((*lower, *raised, start, end));
            }
        }
    }
    shared.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.cmp(&b.3)));
    shared
}

fn arrangement_face_boundary_overlap_interval(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    edge_start: ArrangementBoundaryPointKey,
    edge_end: ArrangementBoundaryPointKey,
) -> Option<(ArrangementSegmentParameter, ArrangementSegmentParameter)> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    let edge_start_t = boundary_segment_parameter_xz(edge_start, segment_start, segment_end)?;
    let edge_end_t = boundary_segment_parameter_xz(edge_end, segment_start, segment_end)?;
    let start = edge_start_t
        .min(edge_end_t)
        .max(ArrangementSegmentParameter::zero());
    let end = edge_start_t
        .max(edge_end_t)
        .min(ArrangementSegmentParameter::one());
    (end > start).then_some((start, end))
}

fn arrangement_face_boundary_interval_point_at(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    interval: ArrangementFaceBoundaryInterval,
    parameter: ArrangementSegmentParameter,
) -> Option<Vector3> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    let segment_point = interpolated_segment_point_key(segment_start, segment_end, parameter);
    let edge_t =
        boundary_segment_parameter_xz(segment_point, interval.edge_start, interval.edge_end)?;
    let edge_point = interpolated_segment_point_key(interval.edge_start, interval.edge_end, edge_t);
    let y_mm = interpolated_segment_height_mm(interval.edge_start, interval.edge_end, edge_t);
    Some(arrangement_boundary_point_to_world(
        ArrangementBoundaryPointKey {
            x_key: edge_point.x_key,
            z_key: edge_point.z_key,
            y_mm,
        },
    ))
}

fn arrangement_key_boundary_point(
    key: NodeArrangementKey,
    y_mm: i64,
) -> ArrangementBoundaryPointKey {
    ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm,
    }
}

fn arrangement_owner_boundary_point_at_key(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    key: NodeArrangementKey,
    prefer_highest: bool,
) -> Option<Vector3> {
    let mut candidates = arrangement
        .vertices()
        .iter()
        .filter(|vertex| vertex.owners().contains(&owner))
        .filter(|vertex| vertex.key() == key)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|vertex| {
        let height_key = if prefer_highest {
            -vertex.height_mm()
        } else {
            vertex.height_mm()
        };
        (height_key, vertex.key())
    });
    candidates.first().map(|vertex| {
        arrangement_boundary_point_to_world(arrangement_key_boundary_point(
            vertex.key(),
            vertex.height_mm(),
        ))
    })
}

fn arrangement_boundary_point_to_world(point: ArrangementBoundaryPointKey) -> Vector3 {
    Vector3::new(
        (point.x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE) as f32,
        point.y_mm as f32 / 1000.0,
        (point.z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE) as f32,
    )
}

fn arrangement_owner_direction_for_segment(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    start: Vector3,
    end: Vector3,
) -> Option<Vector3> {
    let midpoint = (start + end) * 0.5;
    let mut best = None;
    for face in arrangement.faces() {
        if face.owner() != owner
            || !arrangement_face_boundary_overlaps_segment(arrangement, face, segment_key)
        {
            continue;
        }
        let Some(centroid) = arrangement_face_centroid(arrangement, face) else {
            continue;
        };
        let direction = Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
        let distance_squared = direction.length_squared();
        if distance_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            continue;
        }
        if best.is_none_or(|(best_distance_squared, _)| distance_squared < best_distance_squared) {
            best = Some((distance_squared, direction));
        }
    }
    if let Some((_, direction)) = best {
        return Some(direction);
    }

    for region in arrangement.regions() {
        if region.owner() != owner
            || !arrangement_region_boundary_overlaps_segment(arrangement, region, segment_key)
        {
            continue;
        }
        let Some(centroid) = arrangement_region_centroid(arrangement, region) else {
            continue;
        };
        let direction = Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
        let distance_squared = direction.length_squared();
        if distance_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            continue;
        }
        if best.is_none_or(|(best_distance_squared, _)| distance_squared < best_distance_squared) {
            best = Some((distance_squared, direction));
        }
    }
    best.map(|(_, direction)| direction)
}

fn arrangement_vertical_face_visible_dot_to_owner(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    points: [Vector3; 4],
) -> Option<f32> {
    let [upper_start, lower_start, lower_end, _upper_end] = points;
    let normal = (lower_start - upper_start).cross(lower_end - upper_start);
    if normal.length_squared() <= 1e-8 {
        return None;
    }
    let visible_direction = Vector3::new(-normal.x, 0.0, -normal.z);
    if visible_direction.length_squared() <= 1e-8 {
        return None;
    }
    let visible_direction = visible_direction.normalized();
    let midpoint = (lower_start + lower_end) * 0.5;
    let mut best_dot: Option<f32> = None;
    for face in arrangement.faces() {
        if face.owner() != owner
            || !arrangement_face_boundary_overlaps_segment(arrangement, face, segment_key)
        {
            continue;
        }
        let Some(centroid) = arrangement_face_centroid(arrangement, face) else {
            continue;
        };
        let owner_direction = Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
        if owner_direction.length_squared() <= 1e-8 {
            continue;
        }
        let dot = visible_direction.dot(owner_direction.normalized());
        best_dot = Some(best_dot.map_or(dot, |current| current.max(dot)));
    }
    best_dot
}

fn arrangement_face_boundary_overlaps_segment(
    arrangement: &NodeArrangement,
    face: &NodeArrangementFace,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
) -> bool {
    let Some(triangle) = RoadSurfaceSystem::arrangement_face_visual_triangle(arrangement, face)
    else {
        return false;
    };
    for index in 0..triangle.len() {
        let start =
            NodeArrangementKey::from_point(super::backend::godot_vec3_xz_to_road(triangle[index]));
        let end = NodeArrangementKey::from_point(super::backend::godot_vec3_xz_to_road(
            triangle[(index + 1) % triangle.len()],
        ));
        if arrangement_segments_overlap_with_length(start, end, segment_key.0, segment_key.1) {
            return true;
        }
    }
    false
}

fn arrangement_face_centroid(
    arrangement: &NodeArrangement,
    face: &NodeArrangementFace,
) -> Option<Vector3> {
    let triangle = RoadSurfaceSystem::arrangement_face_visual_triangle(arrangement, face)?;
    let mut sum = Vector3::ZERO;
    for point in triangle {
        sum += Vector3::new(point.x, 0.0, point.z);
    }
    Some(sum / triangle.len() as f32)
}

fn arrangement_region_boundary_overlaps_segment(
    arrangement: &NodeArrangement,
    region: &super::arrangement::NodeOwnedRegion,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
) -> bool {
    region.boundary_edges().iter().any(|edge_id| {
        let Some(edge) = arrangement.edges().get(edge_id.index()) else {
            return false;
        };
        let Some(edge_start) = arrangement
            .vertices()
            .get(edge.start().index())
            .map(|vertex| NodeArrangementKey::from_point(vertex.point_xz()))
        else {
            return false;
        };
        let Some(edge_end) = arrangement
            .vertices()
            .get(edge.end().index())
            .map(|vertex| NodeArrangementKey::from_point(vertex.point_xz()))
        else {
            return false;
        };
        arrangement_segments_overlap_with_length(edge_start, edge_end, segment_key.0, segment_key.1)
    })
}

fn arrangement_segments_overlap_with_length(
    a_start: NodeArrangementKey,
    a_end: NodeArrangementKey,
    b_start: NodeArrangementKey,
    b_end: NodeArrangementKey,
) -> bool {
    if a_start == a_end || b_start == b_end {
        return false;
    }
    let a_dx = i128::from(a_end.x_key() - a_start.x_key());
    let a_dz = i128::from(a_end.z_key() - a_start.z_key());
    let b_dx = i128::from(b_end.x_key() - b_start.x_key());
    let b_dz = i128::from(b_end.z_key() - b_start.z_key());
    let cross = a_dx * b_dz - a_dz * b_dx;
    let collinearity_bound = arrangement_overlay_grid_collinearity_error_bound(a_dx, a_dz).max(
        arrangement_overlay_grid_collinearity_error_bound(b_dx, b_dz),
    );
    if cross != 0 && cross.abs() > collinearity_bound {
        return false;
    }
    if !arrangement_key_lies_on_segment(a_start, b_start, b_end)
        && !arrangement_key_lies_on_segment(a_end, b_start, b_end)
        && !arrangement_key_lies_on_segment(b_start, a_start, a_end)
        && !arrangement_key_lies_on_segment(b_end, a_start, a_end)
    {
        return false;
    }
    let use_x = (a_end.x_key() - a_start.x_key()).abs() >= (a_end.z_key() - a_start.z_key()).abs();
    let coordinate = |key: NodeArrangementKey| {
        if use_x { key.x_key() } else { key.z_key() }
    };
    let a0 = coordinate(a_start);
    let a1 = coordinate(a_end);
    let b0 = coordinate(b_start);
    let b1 = coordinate(b_end);
    let a_min = a0.min(a1);
    let a_max = a0.max(a1);
    let b_min = b0.min(b1);
    let b_max = b0.max(b1);
    a_min.max(b_min) < a_max.min(b_max)
}

fn arrangement_region_centroid(
    arrangement: &NodeArrangement,
    region: &super::arrangement::NodeOwnedRegion,
) -> Option<Vector3> {
    let mut sum = Vector3::ZERO;
    let mut count = 0usize;
    for vertex_id in region.outer_loop() {
        let Some(point) = RoadSurfaceSystem::arrangement_vertex_world(arrangement, *vertex_id)
        else {
            continue;
        };
        sum += Vector3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f32)
}

fn arrangement_key_lies_on_segment(
    point: NodeArrangementKey,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key() - start.x_key());
    let dz = i128::from(end.z_key() - start.z_key());
    let px = i128::from(point.x_key() - start.x_key());
    let pz = i128::from(point.z_key() - start.z_key());
    let cross = px * dz - pz * dx;
    if cross != 0 && cross.abs() > arrangement_overlay_grid_collinearity_error_bound(dx, dz) {
        return false;
    }
    let inside_x = if start.x_key() == end.x_key() {
        point.x_key() == start.x_key()
    } else {
        point.x_key() >= start.x_key().min(end.x_key())
            && point.x_key() <= start.x_key().max(end.x_key())
    };
    let inside_z = if start.z_key() == end.z_key() {
        point.z_key() == start.z_key()
    } else {
        point.z_key() >= start.z_key().min(end.z_key())
            && point.z_key() <= start.z_key().max(end.z_key())
    };
    inside_x && inside_z
}

fn arrangement_key_segment_parameter_xz(
    point: NodeArrangementKey,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> Option<ArrangementSegmentParameter> {
    if !arrangement_key_lies_on_segment(point, start, end) || start == end {
        return None;
    }
    let dx = end.x_key() - start.x_key();
    let dz = end.z_key() - start.z_key();
    let (mut numerator, mut denominator) = if dx.abs() >= dz.abs() {
        (point.x_key() - start.x_key(), dx)
    } else {
        (point.z_key() - start.z_key(), dz)
    };
    if denominator == 0 {
        return None;
    }
    if denominator < 0 {
        numerator = -numerator;
        denominator = -denominator;
    }
    Some(ArrangementSegmentParameter {
        numerator: i128::from(numerator),
        denominator: i128::from(denominator),
    })
}

fn arrangement_overlay_grid_collinearity_error_bound(dx: i128, dz: i128) -> i128 {
    (dx.abs() + dz.abs()) * 2
}

fn visible_top_boundary_height_mm_at_key(
    top_polygons: &[&RoadSurfaceVisualPolygon],
    key: NodeArrangementKey,
) -> Option<i64> {
    let mut heights = Vec::new();
    for polygon in top_polygons {
        append_boundary_loop_heights_at_key(&mut heights, &polygon.points_world, key);
        for triangle in &polygon.triangles_world {
            append_boundary_loop_heights_at_key(&mut heights, triangle, key);
        }
    }
    heights.sort_unstable();
    heights.dedup();
    heights.into_iter().max()
}

fn append_boundary_loop_heights_at_key(
    heights: &mut Vec<i64>,
    points: &[Vector3],
    key: NodeArrangementKey,
) {
    if points.len() < 2 {
        return;
    }
    let point = arrangement_key_boundary_point(key, 0);
    for index in 0..points.len() {
        let start = ArrangementBoundaryPointKey::from_world(points[index]);
        let end = ArrangementBoundaryPointKey::from_world(points[(index + 1) % points.len()]);
        if !arrangement_key_lies_on_segment(key, start.xz_key(), end.xz_key()) {
            continue;
        }
        let Some(parameter) = boundary_segment_parameter_xz(point, start, end) else {
            continue;
        };
        heights.push(interpolated_segment_height_mm(start, end, parameter));
    }
}

fn split_boundary_point_loop_at_repeated_xz(points: Vec<Vector3>) -> Vec<Vec<Vector3>> {
    let points = RoadSurfaceSystem::canonicalize_loop_points(points);
    if points.len() < 3 {
        return Vec::new();
    }

    let mut loops = Vec::new();
    let mut stack = vec![points[0]];
    let mut seen = BTreeMap::<NodeArrangementKey, usize>::new();
    seen.insert(
        ArrangementBoundaryPointKey::from_world(points[0]).xz_key(),
        0,
    );

    for index in 1..=points.len() {
        let current = points[index % points.len()];
        let current_key = ArrangementBoundaryPointKey::from_world(current).xz_key();
        if let Some(start_index) = seen.get(&current_key).copied() {
            let mut cycle = stack[start_index..].to_vec();
            cycle.push(current);
            let cycle = RoadSurfaceSystem::canonicalize_loop_points(cycle);
            if cycle.len() >= 3 {
                loops.push(cycle);
            }
            stack.truncate(start_index + 1);
            if let Some(last) = stack.last_mut() {
                *last = current;
            }
            seen.clear();
            for (stack_index, point) in stack.iter().enumerate() {
                seen.insert(
                    ArrangementBoundaryPointKey::from_world(*point).xz_key(),
                    stack_index,
                );
            }
        } else {
            stack.push(current);
            seen.insert(current_key, stack.len() - 1);
        }
    }

    if loops.is_empty() {
        vec![points]
    } else {
        loops
    }
}

fn same_winding_boundary_point_loops_from_loop(points: &[Vector3]) -> Vec<Vec<Vector3>> {
    if !boundary_point_loop_has_repeated_xz(points) {
        return vec![points.to_vec()];
    }

    let source_area_m2 = RoadSurfaceSystem::signed_polygon_area_xz(points);
    split_boundary_point_loop_at_repeated_xz(points.to_vec())
        .into_iter()
        .filter_map(|points| {
            let points = RoadSurfaceSystem::canonicalize_loop_points(points);
            if points.len() < 3 {
                return None;
            }
            let split_area_m2 = RoadSurfaceSystem::signed_polygon_area_xz(&points);
            if split_area_m2.abs() <= boundary_points_numeric_area_budget_m2(&points) {
                return None;
            }
            (source_area_m2.signum() == split_area_m2.signum()).then_some(points)
        })
        .collect()
}

fn boundary_point_loop_has_repeated_xz(points: &[Vector3]) -> bool {
    let mut seen = BTreeSet::new();
    for point in RoadSurfaceSystem::canonicalize_loop_points(points.to_vec()) {
        if !seen.insert(ArrangementBoundaryPointKey::from_world(point).xz_key()) {
            return true;
        }
    }
    false
}

fn boundary_segment_parameter_xz(
    point: ArrangementBoundaryPointKey,
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
) -> Option<ArrangementSegmentParameter> {
    let dx = end.x_key - start.x_key;
    let dz = end.z_key - start.z_key;
    let px = point.x_key - start.x_key;
    let pz = point.z_key - start.z_key;
    let length_squared = squared_key_length(dx, dz);
    if length_squared == 0 || cross_key_delta(dx, dz, px, pz) != 0 {
        return None;
    }
    ArrangementSegmentParameter::new(
        i128::from(px) * i128::from(dx) + i128::from(pz) * i128::from(dz),
        length_squared,
    )
}

fn interpolated_segment_height_mm(
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
    parameter: ArrangementSegmentParameter,
) -> i64 {
    round_div_i128(
        i128::from(start.y_mm) * parameter.denominator
            + i128::from(end.y_mm - start.y_mm) * parameter.numerator,
        parameter.denominator,
    )
}

fn interpolated_segment_point_key(
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
    parameter: ArrangementSegmentParameter,
) -> ArrangementBoundaryPointKey {
    ArrangementBoundaryPointKey {
        x_key: round_div_i128(
            i128::from(start.x_key) * parameter.denominator
                + i128::from(end.x_key - start.x_key) * parameter.numerator,
            parameter.denominator,
        ),
        z_key: round_div_i128(
            i128::from(start.z_key) * parameter.denominator
                + i128::from(end.z_key - start.z_key) * parameter.numerator,
            parameter.denominator,
        ),
        y_mm: interpolated_segment_height_mm(start, end, parameter),
    }
}

fn cross_key_delta(ax: i64, az: i64, bx: i64, bz: i64) -> i128 {
    i128::from(ax) * i128::from(bz) - i128::from(az) * i128::from(bx)
}

fn squared_key_length(dx: i64, dz: i64) -> i128 {
    i128::from(dx) * i128::from(dx) + i128::from(dz) * i128::from(dz)
}

fn round_div_i128(numerator: i128, denominator: i128) -> i64 {
    debug_assert!(denominator > 0);
    let half = denominator / 2;
    let rounded = if numerator >= 0 {
        (numerator + half) / denominator
    } else {
        (numerator - half) / denominator
    };
    rounded as i64
}

fn boundary_points_numeric_area_budget_m2(points: &[Vector3]) -> f32 {
    if points.len() < 2 {
        return NODE_OVERLAY_MIN_AREA_M2;
    }
    let perimeter_m = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| Vector2::new(start.x - end.x, start.z - end.z).length())
        .sum::<f32>();
    RoadSurfaceSystem::overlay_numeric_area_budget_m2(perimeter_m, points.len())
}

#[cfg(test)]
mod tests {
    use super::super::arrangement::{
        NodeBandHeightFieldId, NodeRegionSeamConstraint, NodeSeamSource,
    };
    use super::super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
    use super::super::node_grade::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
    use super::*;

    fn owner(kind: RoadSurfaceBandKind, owner_index: usize) -> NodeBandOwner {
        NodeBandOwner::new(kind, owner_index)
    }

    fn height_field(owner: NodeBandOwner) -> NodeBandHeightFieldId {
        NodeBandHeightFieldId::new(owner.owner_index(), owner.owner_index(), owner.kind())
    }

    fn raised_step_seam(
        lower_owner: NodeBandOwner,
        raised_owner: NodeBandOwner,
        start: RoadVec2,
        end: RoadVec2,
    ) -> NodeRegionSeamConstraint {
        NodeRegionSeamConstraint {
            constraint_index: 7,
            seam_source: NodeSeamSource::RaisedStepContact {
                owner_index: raised_owner.owner_index(),
            },
            owner: Some(lower_owner),
            opposite_owner: Some(raised_owner),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        }
    }

    fn arrangement_with_vertical_step_support(
        raised_start: RoadVec2,
        raised_end: RoadVec2,
    ) -> (NodeArrangement, Vec<NodeExplicitVerticalStepSegment>) {
        arrangement_with_owner_pair_vertical_step_support(
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder,
            raised_start,
            raised_end,
        )
    }

    fn arrangement_with_owner_pair_vertical_step_support(
        lower_kind: RoadSurfaceBandKind,
        raised_kind: RoadSurfaceBandKind,
        raised_start: RoadVec2,
        raised_end: RoadVec2,
    ) -> (NodeArrangement, Vec<NodeExplicitVerticalStepSegment>) {
        let lower_owner = owner(lower_kind, 0);
        let raised_owner = owner(raised_kind, 1);
        let lower_height = height_field(lower_owner);
        let raised_height = height_field(raised_owner);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 0.0);
        let seam = raised_step_seam(lower_owner, raised_owner, start, end);
        let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::Bend);

        let lower_start = arrangement
            .insert_vertex(start, 0.0, [lower_owner], lower_height, [])
            .expect("lower start vertex is valid");
        let lower_end = arrangement
            .insert_vertex(end, 0.0, [lower_owner], lower_height, [])
            .expect("lower end vertex is valid");
        let lower_apex = arrangement
            .insert_vertex(
                RoadVec2::new(0.0, -1.0),
                0.0,
                [lower_owner],
                lower_height,
                [],
            )
            .expect("lower apex vertex is valid");
        let lower_edge = arrangement.push_edge(
            lower_start,
            lower_end,
            lower_owner,
            lower_height,
            Some(raised_owner),
            Some(raised_height),
            false,
            false,
            true,
            NodeSeamSource::RaisedStepContact {
                owner_index: raised_owner.owner_index(),
            },
            vec![seam.constraint_index],
        );
        let lower_region = arrangement.push_region(
            lower_owner,
            lower_height,
            vec![lower_start, lower_end, lower_apex],
            Vec::new(),
            vec![lower_edge],
            1.0,
            vec![seam.clone()],
        );
        arrangement.push_face(
            lower_region,
            lower_owner,
            [lower_start, lower_end, lower_apex],
        );

        let upper_start = arrangement
            .insert_vertex(raised_start, 0.12, [raised_owner], raised_height, [])
            .expect("upper start vertex is valid");
        let upper_end = arrangement
            .insert_vertex(raised_end, 0.12, [raised_owner], raised_height, [])
            .expect("upper end vertex is valid");
        let upper_apex = arrangement
            .insert_vertex(
                RoadVec2::new(raised_start.x, 1.0),
                0.12,
                [raised_owner],
                raised_height,
                [],
            )
            .expect("upper apex vertex is valid");
        let upper_region = arrangement.push_region(
            raised_owner,
            raised_height,
            vec![upper_start, upper_apex, upper_end],
            Vec::new(),
            Vec::new(),
            1.0,
            vec![seam],
        );
        arrangement.push_face(
            upper_region,
            raised_owner,
            [upper_start, upper_apex, upper_end],
        );

        let segments = arrangement.explicit_vertical_step_segments();
        (arrangement, segments)
    }

    fn heighted_vertex_with_grade_decision(
        point_xz: RoadVec2,
        height_m: f64,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        decision: NodeGradeCarrierDecision,
    ) -> NodeHeightedVertex {
        NodeHeightedVertex {
            point_xz,
            height_m,
            height_field_id,
            height_authority: None,
            grade_authority: Some(NodeGradeVertexAuthority::new(
                point_xz,
                height_m,
                owner,
                height_field_id,
                decision,
            )),
        }
    }

    #[test]
    fn node_top_surface_sources_preserve_explicit_material_seam_adoption() {
        let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
        let height_field_id = height_field(owner);
        let decision = NodeGradeCarrierDecision::ExplicitMaterialSeamAdoption;
        let heights = NodeHeightSolution {
            node_id: 82,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![NodeHeightedRegion {
                kind: RoadSurfaceBandKind::Carriageway,
                owner,
                height_field_id,
                shape: vec![vec![
                    heighted_vertex_with_grade_decision(
                        RoadVec2::new(0.0, 0.0),
                        2.0,
                        owner,
                        height_field_id,
                        decision,
                    ),
                    heighted_vertex_with_grade_decision(
                        RoadVec2::new(1.0, 0.0),
                        2.0,
                        owner,
                        height_field_id,
                        decision,
                    ),
                    heighted_vertex_with_grade_decision(
                        RoadVec2::new(0.0, 1.0),
                        2.0,
                        owner,
                        height_field_id,
                        decision,
                    ),
                ]],
                area_m2: 0.5,
                seam_constraints: Vec::new(),
            }],
        };
        let mut arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("grade-authorized seam adoption should arrange");
        let triangulation =
            RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement)
                .expect("grade-authorized seam adoption should triangulate");
        arrangement
            .attach_triangulation(&triangulation)
            .expect("grade-authorized seam adoption should attach triangulation");
        let footprint_shapes = Vec::new();
        let regions = RoadSurfaceSystem::node_surface_regions_from_arrangement(
            &arrangement,
            &footprint_shapes,
        )
        .expect("grade-authorized seam adoption should export node top provenance");

        assert_eq!(regions.node_top_surface_sources.len(), 1);
        let source = &regions.node_top_surface_sources[0];
        assert_eq!(source.kind, RoadSurfaceBandKind::Carriageway);
        assert_eq!(source.owner_index, owner.owner_index());
        assert_eq!(source.height_field_id, height_field_id);
        assert_eq!(source.vertex_sources.len(), 3);
        assert_eq!(source.triangle_sources.len(), 1);
        for grade_authority_index in
            source
                .vertex_sources
                .iter()
                .map(|source| source.grade_authority_index)
                .chain(source.triangle_sources.iter().flat_map(|triangle| {
                    triangle.iter().map(|source| source.grade_authority_index)
                }))
        {
            assert_eq!(
                regions.node_grade_authorities[grade_authority_index].decision,
                NodeGradeCarrierDecision::ExplicitMaterialSeamAdoption
            );
        }
    }

    #[test]
    fn vertical_step_export_uses_exact_canonical_arrangement_keys() {
        let (arrangement, segments) = arrangement_with_vertical_step_support(
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
        );

        let faces =
            RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

        assert_eq!(segments.len(), 1);
        assert_eq!(faces.len(), 1);
    }

    #[test]
    fn vertical_step_export_uses_generic_curb_sidewalk_owner_pair() {
        let (arrangement, segments) = arrangement_with_owner_pair_vertical_step_support(
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Sidewalk,
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
        );

        let faces =
            RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

        assert_eq!(segments.len(), 1);
        assert_eq!(faces.len(), 1);
    }

    #[test]
    fn vertical_step_export_does_not_repair_overlay_sibling_support() {
        let (arrangement, segments) = arrangement_with_vertical_step_support(
            RoadVec2::new(0.0, 0.000001),
            RoadVec2::new(2.0, 0.000001),
        );

        let faces =
            RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

        assert_eq!(segments.len(), 1);
        assert!(
            faces.is_empty(),
            "overlay-neighbor support must not synthesize a vertical face"
        );
    }
}
