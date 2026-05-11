//! Explicit visual node-piece construction and incident-edge classification.

use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    NODE_OVERLAY_MIN_AREA_M2, NodeOwnedRegion, OrderedIncidentPieceMouth, RoadSurfaceBandKind,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge, RoadSurfaceVerticalFaceSource,
    RoadSurfaceVisualNodePiece, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M,
    arrangement::{
        NodeArrangement, NodeArrangementFace, NodeArrangementKey, NodeBandOwner,
        NodeExplicitVerticalStepSegment,
    },
    input::NodeInputExtractionError,
    validation::NodeValidationReport,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;

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

#[derive(Debug)]
enum NodeBoundaryExportError {
    EmptyOuterBoundary,
    MissingFootprintBoundaryHeight,
    DegenerateOuterBoundaryLoop,
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
                self.build_junction_visual_node_piece(terrain, valid, &incidents)
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
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() < 3 {
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
            self.build_closed_earthwork_geometry_from_boundary_point_loops(
                &node_regions.earthwork_boundary_point_loops,
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
            node_regions.curb_vertical_faces,
            node_regions.sidewalk_surface_polygons,
            node_regions.explicit_vertical_step_segments,
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
        footprint_shapes: &super::NodeOverlayShapes,
    ) -> Result<super::NodeSurfaceRegionResult, NodeBoundaryExportError> {
        let mut owned_regions = Vec::new();

        for face in arrangement.faces() {
            let owner = face.owner();
            let Some(polygons) = Self::visual_polygons_from_arrangement_face(arrangement, face)
            else {
                continue;
            };
            for polygon in polygons {
                owned_regions.push(NodeOwnedRegion {
                    kind: owner.kind(),
                    owner_index: owner.owner_index(),
                    polygon,
                });
            }
        }
        let explicit_vertical_step_segments = arrangement.explicit_vertical_step_segments();
        let mut curb_vertical_faces = Self::curb_vertical_face_polygons_from_arrangement(
            arrangement,
            &explicit_vertical_step_segments,
        );
        dedup_curb_vertical_faces(&mut curb_vertical_faces);

        if owned_regions.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }

        let (mut road_surface_polygons, mut curb_surface_polygons, mut sidewalk_surface_polygons) =
            Self::visible_top_polygons_from_owned_regions(&owned_regions);
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
        let footprint_boundary_point_loops = Self::footprint_boundary_point_loops_from_shapes(
            arrangement,
            &top_polygons,
            footprint_shapes,
            arrangement.piece_kind() == RoadSurfaceVisualNodePieceKind::Terminal,
        )?;
        let mut earthwork_boundary_point_loops =
            Self::earthwork_boundary_point_loops_from_footprint_loops(
                &footprint_boundary_point_loops,
            );
        Self::orient_earthwork_boundary_point_loops_by_nesting(&mut earthwork_boundary_point_loops);
        let mut outer_boundary_loops =
            Self::outer_boundary_polygons_from_point_loops(&footprint_boundary_point_loops)?;
        let mut terrain_clip_boundary_loops =
            Self::terrain_clip_boundary_loops_from_point_loops(&footprint_boundary_point_loops);

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_node_owned_regions(&mut owned_regions);
        Self::sort_curb_vertical_faces(&mut curb_vertical_faces);

        Ok(super::NodeSurfaceRegionResult {
            outer_boundary_loops,
            earthwork_boundary_point_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            curb_vertical_faces,
            sidewalk_surface_polygons,
            explicit_vertical_step_segments,
            owned_regions,
        })
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
                fill_missing_footprint_boundary_heights(&mut keyed_points)?;
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
                    remove_unsupported_numeric_boundary_vertices(&mut points, top_polygons);
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

    fn earthwork_boundary_point_loops_from_footprint_loops(
        footprint_loops: &[Vec<Vector3>],
    ) -> Vec<Vec<Vector3>> {
        let mut loops = Vec::new();
        for footprint_loop in footprint_loops {
            for points in same_winding_boundary_point_loops_from_loop(footprint_loop) {
                loops.push(points);
            }
        }
        loops
    }

    fn curb_vertical_face_polygons_from_arrangement(
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
            for (lower_interval, raised_interval, start_t, end_t) in
                arrangement_shared_face_boundary_intervals(&lower_intervals, &raised_intervals)
            {
                let Some(lower_start) = arrangement_face_boundary_interval_point_at(
                    segment_key,
                    lower_interval,
                    start_t,
                ) else {
                    continue;
                };
                let Some(lower_end) =
                    arrangement_face_boundary_interval_point_at(segment_key, lower_interval, end_t)
                else {
                    continue;
                };
                let Some(raised_start) = arrangement_face_boundary_interval_point_at(
                    segment_key,
                    raised_interval,
                    start_t,
                ) else {
                    continue;
                };
                let Some(raised_end) = arrangement_face_boundary_interval_point_at(
                    segment_key,
                    raised_interval,
                    end_t,
                ) else {
                    continue;
                };
                if lower_start.distance_squared_to(lower_end) <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
                {
                    continue;
                }
                if (raised_start.y - lower_start.y <= SAMPLE_EPSILON_M)
                    && (raised_end.y - lower_end.y <= SAMPLE_EPSILON_M)
                {
                    continue;
                }
                let dedup_key =
                    vertical_face_dedup_key(lower_start, lower_end, raised_start, raised_end);
                if !emitted.insert(dedup_key) {
                    continue;
                }
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
                if let Some(face) = Self::make_vertical_quad_polygon(points) {
                    faces.push((
                        face,
                        RoadSurfaceVerticalFaceSource {
                            explicit_vertical_step_index: step_index,
                            segment,
                        },
                    ));
                }
            }
        }
        faces
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

    fn orient_earthwork_boundary_point_loops_by_nesting(loops: &mut [Vec<Vector3>]) {
        let samples = loops
            .iter()
            .map(|points| {
                points.iter().fold(Vector2::ZERO, |sum, point| {
                    sum + Vector2::new(point.x, point.z)
                }) / points.len().max(1) as f32
            })
            .collect::<Vec<_>>();
        let should_be_ccw = loops
            .iter()
            .enumerate()
            .map(|(loop_index, _)| {
                let depth = loops
                    .iter()
                    .enumerate()
                    .filter(|(candidate_index, candidate)| {
                        *candidate_index != loop_index
                            && RoadSurfaceSystem::polygon_contains_point_xz(
                                candidate,
                                samples[loop_index],
                            )
                    })
                    .count();
                depth % 2 == 0
            })
            .collect::<Vec<_>>();
        for (points, should_be_ccw) in loops.iter_mut().zip(should_be_ccw) {
            let is_ccw = Self::signed_polygon_area_xz(points) > 0.0;
            if is_ccw != should_be_ccw {
                points.reverse();
            }
        }
    }

    fn terrain_clip_boundary_loops_from_point_loops(
        point_loops: &[Vec<Vector3>],
    ) -> Vec<RoadSurfaceTerrainClipLoop> {
        let mut loops = Vec::new();
        for point_loop in point_loops {
            for points in same_winding_boundary_point_loops_from_loop(point_loop) {
                if points.len() < 3 {
                    continue;
                }
                if Self::signed_polygon_area_xz(&points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                }
                let source_edges = (0..points.len())
                    .map(|index| RoadSurfaceTerrainClipSourceEdge {
                        start: points[index],
                        end: points[(index + 1) % points.len()],
                        kind: RoadSurfaceTerrainClipEdgeKind::FootprintBoundary,
                    })
                    .collect();
                loops.push(RoadSurfaceTerrainClipLoop {
                    points_world: points,
                    source_edges,
                });
            }
        }
        loops
    }

    fn visual_polygons_from_arrangement_face(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
    ) -> Option<Vec<RoadSurfaceVisualPolygon>> {
        let triangle = Self::arrangement_face_visual_triangle(arrangement, face)?;
        Some(
            [RoadSurfaceVisualPolygon {
                points_world: triangle.to_vec(),
                triangles_world: vec![triangle],
            }]
            .into_iter()
            .collect(),
        )
    }

    fn arrangement_face_visual_triangle(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
    ) -> Option<[Vector3; 3]> {
        let vertices = face.vertices();
        let mut triangle = [
            Self::arrangement_vertex_world(arrangement, vertices[0])?,
            Self::arrangement_vertex_world(arrangement, vertices[1])?,
            Self::arrangement_vertex_world(arrangement, vertices[2])?,
        ];
        let has_area = if face.owner().kind() == RoadSurfaceBandKind::Carriageway {
            Self::triangle_has_area_xz(triangle)
        } else {
            Self::signed_polygon_area_xz(&triangle).abs() > NODE_OVERLAY_MIN_AREA_M2
        };
        let area_3d_m2 = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .length()
            * 0.5;
        if !has_area || area_3d_m2 < NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if Self::signed_polygon_area_xz(&triangle) < 0.0 {
            triangle.swap(1, 2);
        }
        Some(triangle)
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
        };
        self.log_node_validation_report(&report);
    }

    fn build_ordered_piece_mouths(
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
        mut curb_vertical_faces: Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
        mut sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        explicit_vertical_step_segments: Vec<NodeExplicitVerticalStepSegment>,
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
        Self::sort_curb_vertical_faces(&mut curb_vertical_faces);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_node_owned_regions(&mut owned_regions);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        let (curb_vertical_face_polygons, curb_vertical_face_sources) =
            curb_vertical_faces.into_iter().unzip();
        Some(RoadSurfaceVisualNodePiece {
            node_id,
            kind,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            curb_vertical_face_polygons,
            curb_vertical_face_sources,
            sidewalk_surface_polygons,
            explicit_vertical_step_segments,
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

    fn sorted_incident_surface_edges_from_graph_geometry(
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
    if owner.kind() == RoadSurfaceBandKind::Carriageway
        && opposite_owner.kind() != RoadSurfaceBandKind::Carriageway
    {
        return Some((owner, opposite_owner));
    }
    if opposite_owner.kind() == RoadSurfaceBandKind::Carriageway
        && owner.kind() != RoadSurfaceBandKind::Carriageway
    {
        return Some((opposite_owner, owner));
    }
    None
}

fn dedup_curb_vertical_faces(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
) {
    let mut emitted = BTreeSet::new();
    faces.retain(|(polygon, _)| {
        let Some(key) = curb_vertical_face_span_key(polygon) else {
            return true;
        };
        emitted.insert(key)
    });
}

impl RoadSurfaceSystem {
    fn sort_curb_vertical_faces(
        faces: &mut [(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)],
    ) {
        faces.sort_by(
            |(left_polygon, left_source), (right_polygon, right_source)| {
                Self::visual_polygon_ordering(left_polygon, right_polygon)
                    .then(
                        left_source
                            .explicit_vertical_step_index
                            .cmp(&right_source.explicit_vertical_step_index),
                    )
                    .then(left_source.segment.cmp(&right_source.segment))
            },
        );
    }
}

fn curb_vertical_face_span_key(
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
    let y_mm = interpolated_segment_height_mm(interval.edge_start, interval.edge_end, edge_t);
    Some(arrangement_boundary_point_to_world(
        ArrangementBoundaryPointKey {
            x_key: segment_point.x_key,
            z_key: segment_point.z_key,
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
    if a_dx * b_dz - a_dz * b_dx != 0 {
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
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.x_key() == end.x_key() {
        point.x_key() == start.x_key()
    } else {
        point.x_key() > start.x_key().min(end.x_key())
            && point.x_key() < start.x_key().max(end.x_key())
    };
    let inside_z = if start.z_key() == end.z_key() {
        point.z_key() == start.z_key()
    } else {
        point.z_key() > start.z_key().min(end.z_key())
            && point.z_key() < start.z_key().max(end.z_key())
    };
    inside_x && inside_z
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

fn remove_unsupported_numeric_boundary_vertices(
    points: &mut Vec<Vector3>,
    top_polygons: &[&RoadSurfaceVisualPolygon],
) {
    loop {
        if points.len() < 4 {
            return;
        }
        let mut removed = false;
        for index in 0..points.len() {
            let previous = if index == 0 {
                points.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == points.len() {
                0
            } else {
                index + 1
            };
            let current_key = ArrangementBoundaryPointKey::from_world(points[index]).xz_key();
            if visible_top_boundary_height_mm_at_key(top_polygons, current_key).is_some() {
                continue;
            }
            let local_area_m2 = RoadSurfaceSystem::signed_polygon_area_xz(&[
                points[previous],
                points[index],
                points[next],
            ])
            .abs();
            if local_area_m2
                > boundary_points_numeric_area_budget_m2(&[
                    points[previous],
                    points[index],
                    points[next],
                ])
            {
                continue;
            }
            points.remove(index);
            removed = true;
            break;
        }
        if !removed {
            return;
        }
    }
}

fn fill_missing_footprint_boundary_heights(
    vertices: &mut [(NodeArrangementKey, Option<i64>)],
) -> Result<(), NodeBoundaryExportError> {
    let Some(_first_missing_key) = vertices
        .iter()
        .find_map(|(key, height_mm)| height_mm.is_none().then_some(*key))
    else {
        return Ok(());
    };
    let Some(first_solved_index) = vertices
        .iter()
        .position(|(_, height_mm)| height_mm.is_some())
    else {
        return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
    };
    if vertices
        .iter()
        .filter(|(_, height_mm)| height_mm.is_some())
        .count()
        < 2
    {
        return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
    }

    let mut ordered_indices = Vec::with_capacity(vertices.len() + 1);
    ordered_indices.extend(first_solved_index..vertices.len());
    ordered_indices.extend(0..=first_solved_index);

    let mut start_pos = 0;
    while start_pos + 1 < ordered_indices.len() {
        let start_index = ordered_indices[start_pos];
        let Some(start_height_mm) = vertices[start_index].1 else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };
        let Some(end_pos) = (start_pos + 1..ordered_indices.len())
            .find(|pos| vertices[ordered_indices[*pos]].1.is_some())
        else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };
        if end_pos == start_pos + 1 {
            start_pos = end_pos;
            continue;
        }

        let end_index = ordered_indices[end_pos];
        let Some(end_height_mm) = vertices[end_index].1 else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };

        let mut cumulative_lengths = Vec::with_capacity(end_pos - start_pos + 1);
        cumulative_lengths.push(0.0);
        let mut total_length_m = 0.0;
        for pair_pos in start_pos..end_pos {
            total_length_m += arrangement_key_distance_m(
                vertices[ordered_indices[pair_pos]].0,
                vertices[ordered_indices[pair_pos + 1]].0,
            );
            cumulative_lengths.push(total_length_m);
        }
        if total_length_m <= f64::EPSILON {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        }

        for run_offset in 1..cumulative_lengths.len() - 1 {
            let index = ordered_indices[start_pos + run_offset];
            let t = cumulative_lengths[run_offset] / total_length_m;
            vertices[index].1 = Some(
                (start_height_mm as f64 + (end_height_mm - start_height_mm) as f64 * t).round()
                    as i64,
            );
        }
        start_pos = end_pos;
    }
    Ok(())
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

fn arrangement_key_distance_m(start: NodeArrangementKey, end: NodeArrangementKey) -> f64 {
    let dx = (end.x_key() - start.x_key()) as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    let dz = (end.z_key() - start.z_key()) as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    dx.hypot(dz)
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
