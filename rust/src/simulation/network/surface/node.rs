//! Explicit visual node-piece construction and incident-edge classification.

use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    NODE_OVERLAY_MIN_AREA_M2, NodeOwnedRegion, OrderedIncidentPieceMouth, RoadSurfaceBandKind,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge, RoadSurfaceVisualNodePiece,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    arrangement::{
        NodeArrangement, NodeArrangementFace, NodeArrangementKey, NodeBandOwner,
        NodeExplicitVerticalStepSegment, NodeSeamSource,
    },
    input::NodeInputExtractionError,
    terrain_clip_edge_kind_for_band,
    validation::NodeValidationReport,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;

#[derive(Clone, Copy)]
struct ArrangementBoundarySegment {
    start_key: ArrangementBoundaryPointKey,
    end_key: ArrangementBoundaryPointKey,
    start: Vector3,
    end: Vector3,
    owner: NodeBandOwner,
    seam_source: NodeSeamSource,
}

#[derive(Clone, Copy)]
struct ArrangementCanonicalBoundaryEdge {
    start: Vector3,
    end: Vector3,
    start_key: NodeArrangementKey,
    end_key: NodeArrangementKey,
    owner: NodeBandOwner,
    seam_source: NodeSeamSource,
}

#[derive(Clone, Copy, Debug)]
struct OuterBoundaryHeightEntry {
    point: ArrangementBoundaryPointKey,
    owner: NodeBandOwner,
    seam_source: NodeSeamSource,
}

#[derive(Clone, Copy)]
struct ArrangementTerrainClipSourceSegment {
    start: Vector3,
    end: Vector3,
    owner: NodeBandOwner,
    kind: RoadSurfaceTerrainClipEdgeKind,
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
    MissingOuterBoundaryOwner {
        owner: NodeBandOwner,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    },
    ContradictoryOuterBoundaryHeight {
        owner: NodeBandOwner,
        point: NodeArrangementKey,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    AmbiguousOuterBoundaryOwner,
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
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &node_regions.outer_boundary_loops,
                terrain,
            );

        self.assemble_explicit_node_piece(
            node_id,
            kind,
            node_regions.outer_boundary_loops,
            node_regions.terrain_clip_boundary_loops,
            node_regions.road_surface_polygons,
            node_regions.curb_surface_polygons,
            node_regions.curb_vertical_face_polygons,
            node_regions.sidewalk_surface_polygons,
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
        let heights = match Self::build_node_height_solution_from_ownership(&input, &ownership) {
            Ok(heights) => heights,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_height_field_error(
                    node_id, kind, &error,
                ));
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

        match Self::node_surface_regions_from_arrangement(&arrangement) {
            Ok(regions) => Some(regions),
            Err(error) => {
                self.log_node_boundary_export_error(&arrangement, &error);
                None
            }
        }
    }

    fn node_surface_regions_from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Result<super::NodeSurfaceRegionResult, NodeBoundaryExportError> {
        let canonical_segments = Self::arrangement_outer_boundary_segments(arrangement)?;
        let canonical_loops =
            Self::outer_boundary_segment_loops_from_arrangement_segments(&canonical_segments)?;
        validate_outer_boundary_loop_height_consistency(arrangement, &canonical_loops)?;

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
        let mut curb_vertical_face_polygons =
            Self::curb_vertical_face_polygons_from_arrangement(arrangement);
        dedup_curb_vertical_face_polygons(&mut curb_vertical_face_polygons);

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

        let mut outer_boundary_loops =
            Self::outer_boundary_polygons_from_segment_loops(&canonical_loops)?;
        let mut terrain_clip_boundary_loops =
            Self::terrain_clip_boundary_loops_from_segment_loops(&canonical_loops);

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_visual_polygons(&mut curb_vertical_face_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_node_owned_regions(&mut owned_regions);

        Ok(super::NodeSurfaceRegionResult {
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            curb_vertical_face_polygons,
            sidewalk_surface_polygons,
            owned_regions,
        })
    }

    fn curb_vertical_face_polygons_from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut emitted = BTreeSet::new();
        let mut faces = Vec::new();
        for segment in arrangement.explicit_vertical_step_segments() {
            let segment_key = (segment.start(), segment.end());
            let Some((lower_owner, raised_owner)) =
                arrangement_explicit_step_owners_for_segment(arrangement, segment_key)
                    .or_else(|| explicit_step_lower_and_raised_owners(segment))
            else {
                continue;
            };
            let Some((lower_start, raised_start, lower_end, raised_end)) =
                arrangement_vertical_step_points_for_segment(
                    arrangement,
                    segment,
                    lower_owner,
                    raised_owner,
                )
            else {
                continue;
            };
            let key = normalized_arrangement_segment_key(lower_start, lower_end);
            if !emitted.insert(key) {
                continue;
            }
            let mut points = [raised_start, lower_start, lower_end, raised_end];
            if let Some(visible_dot) = arrangement_vertical_face_visible_dot_to_owner(
                arrangement,
                lower_owner,
                key,
                points,
            ) {
                if visible_dot <= 0.0 {
                    points = [points[3], points[2], points[1], points[0]];
                }
            } else {
                let asphalt_direction = arrangement_carriageway_direction_for_segment(
                    arrangement,
                    lower_owner,
                    key,
                    lower_start,
                    lower_end,
                )
                .unwrap_or_else(|| {
                    let edge_direction = lower_end - lower_start;
                    Vector3::new(-edge_direction.z, 0.0, edge_direction.x)
                });
                let face_normal = (points[1] - points[0]).cross(points[2] - points[0]);
                if face_normal.dot(asphalt_direction) > 0.0 {
                    points = [points[3], points[2], points[1], points[0]];
                }
            }
            if let Some(face) = Self::make_vertical_quad_polygon(points) {
                faces.push(face);
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

    fn arrangement_outer_boundary_segments(
        arrangement: &NodeArrangement,
    ) -> Result<Vec<ArrangementBoundarySegment>, NodeBoundaryExportError> {
        let canonical_edges = Self::arrangement_canonical_boundary_edges(arrangement)?;
        let canonical_segments = canonical_edges
            .iter()
            .map(|edge| ArrangementBoundarySegment {
                start_key: ArrangementBoundaryPointKey::from_world(edge.start),
                end_key: ArrangementBoundaryPointKey::from_world(edge.end),
                start: edge.start,
                end: edge.end,
                owner: edge.owner,
                seam_source: edge.seam_source,
            })
            .collect::<Vec<_>>();

        if canonical_segments.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }
        Ok(canonical_segments)
    }

    fn arrangement_canonical_boundary_edges(
        arrangement: &NodeArrangement,
    ) -> Result<Vec<ArrangementCanonicalBoundaryEdge>, NodeBoundaryExportError> {
        let mut edges = Vec::new();
        for edge in arrangement
            .edges()
            .iter()
            .filter(|edge| edge.is_exposed_boundary())
        {
            let Some(start) = Self::arrangement_vertex_world(arrangement, edge.start()) else {
                return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
            };
            let Some(end) = Self::arrangement_vertex_world(arrangement, edge.end()) else {
                return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
            };
            let start_point_key = ArrangementBoundaryPointKey::from_world(start);
            let end_point_key = ArrangementBoundaryPointKey::from_world(end);
            let start_key = start_point_key.xz_key();
            let end_key = end_point_key.xz_key();
            if start_key == end_key {
                continue;
            }
            edges.push(ArrangementCanonicalBoundaryEdge {
                start,
                end,
                start_key,
                end_key,
                owner: edge.owner(),
                seam_source: edge.seam_source(),
            });
        }
        if edges.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }
        edges.sort_by(|a, b| {
            a.start_key
                .cmp(&b.start_key)
                .then(a.end_key.cmp(&b.end_key))
                .then(a.owner.kind().cmp(&b.owner.kind()))
                .then(a.owner.owner_index().cmp(&b.owner.owner_index()))
        });
        Ok(edges)
    }

    fn outer_boundary_segment_loops_from_arrangement_segments(
        segments: &[ArrangementBoundarySegment],
    ) -> Result<Vec<Vec<ArrangementBoundarySegment>>, NodeBoundaryExportError> {
        let mut unused = (0..segments.len()).collect::<BTreeSet<_>>();
        let mut loops = Vec::new();
        while let Some(first_index) = unused.iter().next().copied() {
            let first = segments[first_index];
            let first_key = first.start_key;
            let mut current_index = first_index;
            let mut current = first;
            let mut loop_segments = Vec::new();

            loop {
                if !unused.remove(&current_index) {
                    return Err(NodeBoundaryExportError::AmbiguousOuterBoundaryOwner);
                }
                loop_segments.push(current);

                if arrangement_boundary_point_same_xz(current.end_key, first_key) {
                    break;
                }

                let next_candidates = unused
                    .iter()
                    .copied()
                    .filter(|index| {
                        arrangement_boundary_point_same_xz(
                            segments[*index].start_key,
                            current.end_key,
                        )
                    })
                    .collect::<Vec<_>>();
                if next_candidates.is_empty() {
                    return Err(NodeBoundaryExportError::MissingOuterBoundaryOwner {
                        owner: current.owner,
                        start: current.end_key.xz_key(),
                        end: first_key.xz_key(),
                    });
                }
                current_index = Self::select_next_arrangement_boundary_segment(
                    segments,
                    current,
                    &next_candidates,
                )
                .ok_or_else(|| NodeBoundaryExportError::AmbiguousOuterBoundaryOwner)?;
                current = segments[current_index];
            }

            for simple_loop in split_boundary_segment_loop_at_repeated_xz(loop_segments) {
                let points =
                    Self::canonicalize_loop_points(boundary_points_from_segment_loop(&simple_loop));
                if points.len() < 3 {
                    continue;
                }
                let area_m2 = Self::signed_polygon_area_xz(&points).abs();
                if area_m2 <= boundary_points_numeric_area_budget_m2(&points) {
                    continue;
                }
                loops.push(simple_loop);
            }
        }

        (!loops.is_empty())
            .then_some(loops)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }

    fn outer_boundary_polygons_from_segment_loops(
        canonical_loops: &[Vec<ArrangementBoundarySegment>],
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, NodeBoundaryExportError> {
        let mut polygons = Vec::new();
        for canonical_loop in canonical_loops {
            let points =
                Self::canonicalize_loop_points(boundary_points_from_segment_loop(canonical_loop));
            if points.len() < 3 {
                continue;
            }
            let area_m2 = Self::signed_polygon_area_xz(&points).abs();
            if area_m2 <= boundary_points_numeric_area_budget_m2(&points) {
                continue;
            }
            let Some(polygon) = Self::make_boundary_loop_polygon(points) else {
                crate::debug_log!(
                    "road",
                    "node_outer_boundary_degenerate_loop segments={} area={:.6}",
                    canonical_loop.len(),
                    area_m2
                );
                return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
            };
            polygons.push(polygon);
        }
        (!polygons.is_empty())
            .then_some(polygons)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }

    fn terrain_clip_boundary_loops_from_segment_loops(
        canonical_loops: &[Vec<ArrangementBoundarySegment>],
    ) -> Vec<RoadSurfaceTerrainClipLoop> {
        let mut loops = Vec::new();
        for canonical_loop in canonical_loops {
            let (mut points, mut source_edges) =
                terrain_clip_boundary_points_and_source_edges_from_segments(canonical_loop);
            if points.len() < 3 {
                continue;
            }
            let signed_area = Self::signed_polygon_area_xz(&points);
            if signed_area.abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                continue;
            }
            if signed_area < 0.0 {
                // Terrain overlay uses positive-fill contours; arrangement traversal direction
                // must not decide whether an owner seam reaches the terrain CDT.
                points.reverse();
                source_edges.reverse();
                for edge in &mut source_edges {
                    std::mem::swap(&mut edge.start, &mut edge.end);
                }
            }
            loops.push(RoadSurfaceTerrainClipLoop {
                points_world: points,
                source_edges,
            });
        }
        loops
    }

    fn select_next_arrangement_boundary_segment(
        segments: &[ArrangementBoundarySegment],
        current: ArrangementBoundarySegment,
        candidates: &[usize],
    ) -> Option<usize> {
        let incoming = Vector2::new(
            current.end.x - current.start.x,
            current.end.z - current.start.z,
        );
        if incoming.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return candidates.iter().copied().min_by(|a, b| {
                let segment_a = segments[*a];
                let segment_b = segments[*b];
                arrangement_boundary_continuity_key(current, segment_a, *a)
                    .cmp(&arrangement_boundary_continuity_key(current, segment_b, *b))
            });
        }
        candidates.iter().copied().min_by(|a, b| {
            let segment_a = segments[*a];
            let segment_b = segments[*b];
            let turn_a = arrangement_boundary_turn_abs(incoming, segment_a);
            let turn_b = arrangement_boundary_turn_abs(incoming, segment_b);
            turn_a
                .total_cmp(&turn_b)
                .then(segment_a.end_key.cmp(&segment_b.end_key))
                .then(segment_a.owner.kind().cmp(&segment_b.owner.kind()))
                .then(
                    segment_a
                        .owner
                        .owner_index()
                        .cmp(&segment_b.owner.owner_index()),
                )
                .then(a.cmp(b))
        })
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
            NodeBoundaryExportError::MissingOuterBoundaryOwner { owner, start, end } => {
                NodeValidationReport::from_missing_outer_boundary_owner(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    owner.kind(),
                    owner.owner_index(),
                    *start,
                    *end,
                )
            }
            NodeBoundaryExportError::ContradictoryOuterBoundaryHeight {
                owner,
                point,
                existing_height_mm,
                incoming_height_mm,
            } => NodeValidationReport::from_outer_boundary_height_conflict(
                arrangement.node_id(),
                arrangement.piece_kind(),
                owner.kind(),
                owner.owner_index(),
                *point,
                *existing_height_mm,
                *incoming_height_mm,
            ),
            NodeBoundaryExportError::EmptyOuterBoundary => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "empty_outer_boundary",
                )
            }
            NodeBoundaryExportError::AmbiguousOuterBoundaryOwner => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "ambiguous_outer_boundary_owner",
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
            mouths.push(OrderedIncidentPieceMouth {
                profile,
                endpoint_profile,
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
        mut curb_vertical_face_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
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
        Self::sort_visual_polygons(&mut curb_vertical_face_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_node_owned_regions(&mut owned_regions);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        Some(RoadSurfaceVisualNodePiece {
            node_id,
            kind,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            curb_vertical_face_polygons,
            sidewalk_surface_polygons,
            owned_regions,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }

    fn classify_visual_node_kind(&self, incidents: &[IncidentSurfaceEdge]) -> CompiledNodeKind {
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

    fn sorted_incident_surface_edges(
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

fn arrangement_boundary_point_same_xz(
    a: ArrangementBoundaryPointKey,
    b: ArrangementBoundaryPointKey,
) -> bool {
    a.x_key == b.x_key && a.z_key == b.z_key
}

fn normalized_arrangement_segment_key(
    start: Vector3,
    end: Vector3,
) -> (NodeArrangementKey, NodeArrangementKey) {
    let start = ArrangementBoundaryPointKey::from_world(start).xz_key();
    let end = ArrangementBoundaryPointKey::from_world(end).xz_key();
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn dedup_curb_vertical_face_polygons(polygons: &mut Vec<RoadSurfaceVisualPolygon>) {
    let mut emitted = BTreeSet::new();
    polygons.retain(|polygon| {
        let Some(key) = curb_vertical_face_lower_edge_key(polygon) else {
            return true;
        };
        emitted.insert(key)
    });
}

fn curb_vertical_face_lower_edge_key(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<(NodeArrangementKey, NodeArrangementKey)> {
    if polygon.points_world.len() != 4 {
        return None;
    }
    let lower_y = polygon
        .points_world
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let lower_points = polygon
        .points_world
        .iter()
        .copied()
        .filter(|point| (point.y - lower_y).abs() <= SAMPLE_EPSILON_M)
        .collect::<Vec<_>>();
    if lower_points.len() != 2 {
        return None;
    }
    Some(normalized_arrangement_segment_key(
        lower_points[0],
        lower_points[1],
    ))
}

fn arrangement_vertical_step_points_for_segment(
    arrangement: &NodeArrangement,
    segment: NodeExplicitVerticalStepSegment,
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
) -> Option<(Vector3, Vector3, Vector3, Vector3)> {
    let lower_start =
        arrangement_step_point_at_key(arrangement, segment.start(), lower_owner, false)?;
    let raised_start =
        arrangement_step_point_at_key(arrangement, segment.start(), raised_owner, true)?;
    let lower_end = arrangement_step_point_at_key(arrangement, segment.end(), lower_owner, false)?;
    let raised_end = arrangement_step_point_at_key(arrangement, segment.end(), raised_owner, true)?;
    ((raised_start.y - lower_start.y > SAMPLE_EPSILON_M)
        || (raised_end.y - lower_end.y > SAMPLE_EPSILON_M))
        .then_some((lower_start, raised_start, lower_end, raised_end))
}

fn arrangement_explicit_step_owners_for_segment(
    arrangement: &NodeArrangement,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let lower_owner = arrangement_face_owner_for_segment_kind(
        arrangement,
        segment_key,
        RoadSurfaceBandKind::Carriageway,
    )?;
    let raised_owner = arrangement_face_owner_for_segment_kind(
        arrangement,
        segment_key,
        RoadSurfaceBandKind::CurbOrShoulder,
    )?;
    Some((lower_owner, raised_owner))
}

fn arrangement_face_owner_for_segment_kind(
    arrangement: &NodeArrangement,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    kind: RoadSurfaceBandKind,
) -> Option<NodeBandOwner> {
    arrangement
        .faces()
        .iter()
        .filter(|face| face.owner().kind() == kind)
        .filter(|face| arrangement_face_boundary_overlaps_segment(arrangement, face, segment_key))
        .map(|face| face.owner())
        .min()
}

fn explicit_step_lower_and_raised_owners(
    segment: NodeExplicitVerticalStepSegment,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    match (segment.owner().kind(), segment.opposite_owner().kind()) {
        (RoadSurfaceBandKind::Carriageway, RoadSurfaceBandKind::CurbOrShoulder) => {
            Some((segment.owner(), segment.opposite_owner()))
        }
        (RoadSurfaceBandKind::CurbOrShoulder, RoadSurfaceBandKind::Carriageway) => {
            Some((segment.opposite_owner(), segment.owner()))
        }
        _ => None,
    }
}

fn arrangement_step_point_at_key(
    arrangement: &NodeArrangement,
    key: NodeArrangementKey,
    owner: NodeBandOwner,
    prefer_raised: bool,
) -> Option<Vector3> {
    let mut selected = None;
    for vertex in arrangement.vertices().iter().filter(|vertex| {
        NodeArrangementKey::from_point(vertex.point_xz()) == key && vertex.owners().contains(&owner)
    }) {
        let point = Vector3::new(
            vertex.point_xz().x as f32,
            vertex.height_m() as f32,
            vertex.point_xz().y as f32,
        );
        if selected.is_none_or(|candidate: Vector3| {
            if prefer_raised {
                point.y > candidate.y
            } else {
                point.y < candidate.y
            }
        }) {
            selected = Some(point);
        }
    }
    selected
}

fn arrangement_carriageway_direction_for_segment(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    start: Vector3,
    end: Vector3,
) -> Option<Vector3> {
    arrangement_owner_direction_for_segment(arrangement, owner, segment_key, start, end)
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

fn validate_outer_boundary_loop_height_consistency(
    arrangement: &NodeArrangement,
    loops: &[Vec<ArrangementBoundarySegment>],
) -> Result<(), NodeBoundaryExportError> {
    let mut entries_by_xz = BTreeMap::<NodeArrangementKey, Vec<OuterBoundaryHeightEntry>>::new();
    for loop_segments in loops {
        for segment in loop_segments {
            push_outer_boundary_height_entry(&mut entries_by_xz, segment.start_key, *segment);
            push_outer_boundary_height_entry(&mut entries_by_xz, segment.end_key, *segment);
        }
    }
    for (key, entries) in entries_by_xz {
        if let Some((existing, incoming)) = outer_boundary_height_conflict(&entries) {
            if outer_boundary_entries_allow_vertical_step(arrangement, key, &entries) {
                continue;
            }
            return Err(NodeBoundaryExportError::ContradictoryOuterBoundaryHeight {
                owner: existing.owner,
                point: key,
                existing_height_mm: existing.point.y_mm,
                incoming_height_mm: incoming.point.y_mm,
            });
        }
    }
    Ok(())
}

fn push_outer_boundary_height_entry(
    entries_by_xz: &mut BTreeMap<NodeArrangementKey, Vec<OuterBoundaryHeightEntry>>,
    point: ArrangementBoundaryPointKey,
    segment: ArrangementBoundarySegment,
) {
    entries_by_xz
        .entry(point.xz_key())
        .or_default()
        .push(OuterBoundaryHeightEntry {
            point,
            owner: segment.owner,
            seam_source: segment.seam_source,
        });
}

fn outer_boundary_height_conflict(
    entries: &[OuterBoundaryHeightEntry],
) -> Option<(OuterBoundaryHeightEntry, OuterBoundaryHeightEntry)> {
    let first = entries.first().copied()?;
    entries
        .iter()
        .copied()
        .find(|entry| entry.point.y_mm != first.point.y_mm)
        .map(|incoming| (first, incoming))
}

fn outer_boundary_entries_allow_vertical_step(
    arrangement: &NodeArrangement,
    key: NodeArrangementKey,
    entries: &[OuterBoundaryHeightEntry],
) -> bool {
    let has_carriageway = entries
        .iter()
        .any(|entry| entry.owner.kind() == RoadSurfaceBandKind::Carriageway);
    let entries_have_explicit_curb_contact = entries.iter().any(|entry| {
        entry.owner.kind() == RoadSurfaceBandKind::CurbOrShoulder
            && matches!(entry.seam_source, NodeSeamSource::AsphaltCurbContact { .. })
    });
    if !has_carriageway
        || (!entries_have_explicit_curb_contact
            && !arrangement_has_explicit_asphalt_curb_step_at_key(arrangement, key))
    {
        return false;
    }
    entries.iter().all(|entry| {
        matches!(
            entry.owner.kind(),
            RoadSurfaceBandKind::Carriageway
                | RoadSurfaceBandKind::CurbOrShoulder
                | RoadSurfaceBandKind::Sidewalk
        )
    })
}

fn arrangement_has_explicit_asphalt_curb_step_at_key(
    arrangement: &NodeArrangement,
    key: NodeArrangementKey,
) -> bool {
    arrangement
        .explicit_vertical_step_segments()
        .iter()
        .any(|segment| arrangement_key_lies_on_segment(key, segment.start(), segment.end()))
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

fn terrain_clip_source_segment_from_boundary(
    segment: ArrangementBoundarySegment,
) -> ArrangementTerrainClipSourceSegment {
    ArrangementTerrainClipSourceSegment {
        start: segment.start,
        end: segment.end,
        owner: segment.owner,
        kind: terrain_clip_edge_kind_for_band(segment.owner.kind()),
    }
}

fn terrain_clip_boundary_points_and_source_edges_from_segments(
    segments: &[ArrangementBoundarySegment],
) -> (Vec<Vector3>, Vec<RoadSurfaceTerrainClipSourceEdge>) {
    let source_segments = segments
        .iter()
        .map(|segment| terrain_clip_source_segment_from_boundary(*segment))
        .collect::<Vec<_>>();

    let filtered = source_segments
        .iter()
        .copied()
        .filter(|segment| {
            // Lower material contacts are not terrain seams when an explicit non-carriageway
            // arrangement edge already covers the same interval at the solved top height.
            !arrangement_source_is_lower_material_contact(*segment)
                || !arrangement_source_is_covered_by_outer_top(*segment, &source_segments)
        })
        .collect::<Vec<_>>();
    let points = filtered.iter().map(|segment| segment.start).collect();
    let source_edges = filtered
        .iter()
        .map(|segment| RoadSurfaceTerrainClipSourceEdge {
            start: segment.start,
            end: segment.end,
            kind: segment.kind,
        })
        .collect();
    (points, source_edges)
}

fn arrangement_source_is_lower_material_contact(
    segment: ArrangementTerrainClipSourceSegment,
) -> bool {
    segment.owner.kind() == RoadSurfaceBandKind::Carriageway
}

fn arrangement_source_is_covered_by_outer_top(
    segment: ArrangementTerrainClipSourceSegment,
    source_segments: &[ArrangementTerrainClipSourceSegment],
) -> bool {
    let mut intervals = source_segments
        .iter()
        .copied()
        .filter(|candidate| arrangement_source_can_cover_lower(*candidate, segment))
        .filter_map(|candidate| arrangement_source_overlap_interval(segment, candidate))
        .collect::<Vec<_>>();
    if intervals.is_empty() {
        return false;
    }

    intervals.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut covered_end = ArrangementSegmentParameter::zero();
    for (start, end) in intervals {
        if start > covered_end {
            return false;
        }
        covered_end = covered_end.max(end);
        if covered_end >= ArrangementSegmentParameter::one() {
            return true;
        }
    }
    false
}

fn arrangement_source_overlap_interval(
    segment: ArrangementTerrainClipSourceSegment,
    candidate: ArrangementTerrainClipSourceSegment,
) -> Option<(ArrangementSegmentParameter, ArrangementSegmentParameter)> {
    let candidate_start_t =
        boundary_line_parameter_xz(candidate.start, segment.start, segment.end)?;
    let candidate_end_t = boundary_line_parameter_xz(candidate.end, segment.start, segment.end)?;
    let start = candidate_start_t
        .min(candidate_end_t)
        .max(ArrangementSegmentParameter::zero());
    let end = candidate_start_t
        .max(candidate_end_t)
        .min(ArrangementSegmentParameter::one());
    if end <= start {
        return None;
    }
    if !candidate_covers_lower_heights(segment, candidate, start)
        || !candidate_covers_lower_heights(segment, candidate, end)
    {
        return None;
    }
    Some((start, end))
}

fn boundary_line_parameter_xz(
    point: Vector3,
    start: Vector3,
    end: Vector3,
) -> Option<ArrangementSegmentParameter> {
    boundary_segment_parameter_xz(
        ArrangementBoundaryPointKey::from_world(point),
        ArrangementBoundaryPointKey::from_world(start),
        ArrangementBoundaryPointKey::from_world(end),
    )
}

fn arrangement_source_can_cover_lower(
    candidate: ArrangementTerrainClipSourceSegment,
    lower: ArrangementTerrainClipSourceSegment,
) -> bool {
    if candidate.owner.kind() == RoadSurfaceBandKind::Carriageway {
        return false;
    }
    !arrangement_sources_same_xz(candidate, lower)
}

fn arrangement_sources_same_xz(
    a: ArrangementTerrainClipSourceSegment,
    b: ArrangementTerrainClipSourceSegment,
) -> bool {
    let a_start = ArrangementBoundaryPointKey::from_world(a.start);
    let a_end = ArrangementBoundaryPointKey::from_world(a.end);
    let b_start = ArrangementBoundaryPointKey::from_world(b.start);
    let b_end = ArrangementBoundaryPointKey::from_world(b.end);
    let same_direction = arrangement_boundary_point_same_xz(a_start, b_start)
        && arrangement_boundary_point_same_xz(a_end, b_end);
    let opposite_direction = arrangement_boundary_point_same_xz(a_start, b_end)
        && arrangement_boundary_point_same_xz(a_end, b_start);
    same_direction || opposite_direction
}

fn candidate_covers_lower_heights(
    lower: ArrangementTerrainClipSourceSegment,
    candidate: ArrangementTerrainClipSourceSegment,
    lower_t: ArrangementSegmentParameter,
) -> bool {
    let lower_start = ArrangementBoundaryPointKey::from_world(lower.start);
    let lower_end = ArrangementBoundaryPointKey::from_world(lower.end);
    let candidate_start = ArrangementBoundaryPointKey::from_world(candidate.start);
    let candidate_end = ArrangementBoundaryPointKey::from_world(candidate.end);
    let point = interpolated_segment_point_key(lower_start, lower_end, lower_t);
    let Some(candidate_t) = boundary_segment_parameter_xz(point, candidate_start, candidate_end)
    else {
        return false;
    };
    let lower_y_mm = interpolated_segment_height_mm(lower_start, lower_end, lower_t);
    let candidate_y_mm =
        interpolated_segment_height_mm(candidate_start, candidate_end, candidate_t);
    candidate_y_mm >= lower_y_mm
}

fn split_boundary_segment_loop_at_repeated_xz(
    segments: Vec<ArrangementBoundarySegment>,
) -> Vec<Vec<ArrangementBoundarySegment>> {
    let mut loops = Vec::new();
    let mut stack = Vec::<ArrangementBoundarySegment>::new();
    let mut seen = BTreeMap::<NodeArrangementKey, usize>::new();
    if let Some(first) = segments.first() {
        seen.insert(first.start_key.xz_key(), 0);
    }

    for segment in segments {
        stack.push(segment);
        let end_key = segment.end_key.xz_key();
        if let Some(start_index) = seen.get(&end_key).copied() {
            let cycle = stack[start_index..].to_vec();
            if cycle.len() >= 3 {
                loops.push(cycle);
            }
            stack.truncate(start_index);
            seen.clear();
            if let Some(first) = stack.first() {
                seen.insert(first.start_key.xz_key(), 0);
                for (index, segment) in stack.iter().enumerate() {
                    seen.insert(segment.end_key.xz_key(), index + 1);
                }
            } else {
                seen.insert(end_key, 0);
            }
        } else {
            seen.insert(end_key, stack.len());
        }
    }

    if stack.len() >= 3 {
        loops.push(stack);
    }
    loops
}

fn boundary_points_from_segment_loop(segments: &[ArrangementBoundarySegment]) -> Vec<Vector3> {
    let mut points = Vec::new();
    for &segment in segments {
        if points.is_empty() {
            points.push(segment.start);
        } else if points
            .last()
            .is_some_and(|last| ArrangementBoundaryPointKey::from_world(*last) != segment.start_key)
        {
            points.push(segment.start);
        }
        points.push(segment.end);
    }
    points
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

fn arrangement_boundary_turn_abs(incoming: Vector2, candidate: ArrangementBoundarySegment) -> f32 {
    let outgoing = Vector2::new(
        candidate.end.x - candidate.start.x,
        candidate.end.z - candidate.start.z,
    );
    if outgoing.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
        return f32::INFINITY;
    }
    let cross = incoming.x * outgoing.y - incoming.y * outgoing.x;
    let dot = incoming.dot(outgoing);
    cross.atan2(dot).abs()
}

fn arrangement_boundary_continuity_key(
    current: ArrangementBoundarySegment,
    candidate: ArrangementBoundarySegment,
    candidate_index: usize,
) -> (
    bool,
    bool,
    ArrangementBoundaryPointKey,
    RoadSurfaceBandKind,
    usize,
    usize,
) {
    (
        candidate.owner != current.owner,
        candidate.seam_source != current.seam_source,
        candidate.end_key,
        candidate.owner.kind(),
        candidate.owner.owner_index(),
        candidate_index,
    )
}
