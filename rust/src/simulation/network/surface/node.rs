//! Explicit visual node-piece construction and incident-edge classification.

use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    NODE_OVERLAY_MIN_AREA_M2, NodeOwnedRegion, OrderedIncidentPieceMouth, RoadSurfaceBandKind,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceVisualNodePiece,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    arrangement::{NodeArrangement, NodeArrangementKey, NodeBandOwner},
    input::NodeInputExtractionError,
    validation::NodeValidationReport,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const ARRANGEMENT_EDGE_SPLIT_TOLERANCE_M: f32 = 0.02;
const ARRANGEMENT_EDGE_SPLIT_HEIGHT_TOLERANCE_M: f32 = 0.004;

#[derive(Clone, Copy)]
struct ArrangementBoundarySegment {
    start_key: ArrangementBoundaryPointKey,
    end_key: ArrangementBoundaryPointKey,
    start: Vector3,
    end: Vector3,
    owner: NodeBandOwner,
}

#[derive(Clone, Copy)]
struct ArrangementCanonicalBoundaryEdge {
    start: Vector3,
    end: Vector3,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start_key: NodeArrangementKey,
    end_key: NodeArrangementKey,
    owner: NodeBandOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ArrangementBoundaryPointKey {
    x_mm: i64,
    z_mm: i64,
    y_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ArrangementBoundaryEdgeKey {
    start: NodeArrangementKey,
    end: NodeArrangementKey,
}

impl ArrangementBoundaryPointKey {
    fn from_world(point: Vector3) -> Self {
        Self {
            x_mm: (point.x * 1000.0).round() as i64,
            z_mm: (point.z * 1000.0).round() as i64,
            y_mm: (point.y * 1000.0).round() as i64,
        }
    }

    fn xz_key(self) -> NodeArrangementKey {
        NodeArrangementKey::from_point(super::backend::RoadVec2::new(
            self.x_mm as f64 / 1000.0,
            self.z_mm as f64 / 1000.0,
        ))
    }
}

impl ArrangementBoundaryEdgeKey {
    fn new(a: ArrangementBoundaryPointKey, b: ArrangementBoundaryPointKey) -> Self {
        let a = a.xz_key();
        let b = b.xz_key();
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
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
        let mouths = self.build_ordered_piece_mouths(incidents)?;
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
            node_regions.road_surface_polygons,
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
                Self::log_node_input_extraction_error(node_id, kind, &error);
                return None;
            }
        };
        let rails = match Self::build_node_rail_contours_from_input(&input) {
            Ok(rails) => rails,
            Err(error) => {
                Self::log_node_validation_report(
                    &NodeValidationReport::from_rail_generation_error(node_id, kind, &error),
                );
                return None;
            }
        };
        let ownership = match Self::build_node_boolean_ownership_from_rails(&rails) {
            Ok(ownership) => ownership,
            Err(error) => {
                Self::log_node_validation_report(
                    &NodeValidationReport::from_boolean_ownership_error(node_id, kind, &error),
                );
                return None;
            }
        };
        if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
            &ownership.owned_region_arrangement,
        ) {
            Self::log_node_validation_report(&report);
        }
        let heights = match Self::build_node_height_solution_from_ownership(&input, &ownership) {
            Ok(heights) => heights,
            Err(error) => {
                Self::log_node_validation_report(&NodeValidationReport::from_height_source_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        let triangulation = match Self::build_node_triangulation_from_height_solution(&heights) {
            Ok(triangulation) => triangulation,
            Err(error) => {
                Self::log_node_validation_report(&NodeValidationReport::from_triangulation_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        match Self::validate_node_triangulation_solution(&triangulation) {
            Ok(report) => Self::log_node_validation_report(&report),
            Err(error) => {
                Self::log_node_validation_report(&error.report);
                if error.report.has_blocking_diagnostics() {
                    return None;
                }
            }
        }
        let arrangement =
            match NodeArrangement::from_height_solution_and_triangulation(&heights, &triangulation)
            {
                Ok(arrangement) => arrangement,
                Err(error) => {
                    Self::log_node_validation_report(
                        &NodeValidationReport::from_arrangement_error(node_id, kind, &error),
                    );
                    return None;
                }
            };
        if let Some(report) = NodeValidationReport::from_arrangement_diagnostics(&arrangement) {
            Self::log_node_validation_report(&report);
        }

        match Self::node_surface_regions_from_arrangement(&arrangement) {
            Ok(regions) => Some(regions),
            Err(error) => {
                Self::log_node_boundary_export_error(&arrangement, &error);
                None
            }
        }
    }

    fn node_surface_regions_from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Result<super::NodeSurfaceRegionResult, NodeBoundaryExportError> {
        let mut owned_regions = Vec::new();

        for face in arrangement.faces() {
            let owner = face.owner();
            let Some(polygons) = Self::visual_polygons_from_arrangement_face(arrangement, face)
            else {
                return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
            };
            for polygon in polygons {
                owned_regions.push(NodeOwnedRegion {
                    kind: owner.kind(),
                    owner_index: owner.owner_index(),
                    polygon,
                });
            }
        }

        if owned_regions.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }

        let (mut road_surface_polygons, mut sidewalk_surface_polygons) =
            Self::visible_top_polygons_from_owned_regions(&owned_regions);
        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }

        let mut outer_boundary_loops = Self::outer_boundary_loops_from_arrangement(arrangement)?;

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_node_owned_regions(&mut owned_regions);

        Ok(super::NodeSurfaceRegionResult {
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            owned_regions,
        })
    }

    fn visible_top_polygons_from_owned_regions(
        owned_regions: &[NodeOwnedRegion],
    ) -> (Vec<RoadSurfaceVisualPolygon>, Vec<RoadSurfaceVisualPolygon>) {
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        for region in owned_regions {
            if region.kind == RoadSurfaceBandKind::Carriageway {
                road_surface_polygons.push(region.polygon.clone());
            } else {
                sidewalk_surface_polygons.push(region.polygon.clone());
            }
        }
        (road_surface_polygons, sidewalk_surface_polygons)
    }

    fn outer_boundary_loops_from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, NodeBoundaryExportError> {
        let (canonical_segments, top_boundary_segments) =
            Self::arrangement_outer_boundary_segment_sources(arrangement)?;
        let canonical_loops =
            Self::outer_boundary_segment_loops_from_arrangement_segments(&canonical_segments)?;
        Self::outer_boundary_polygons_from_segment_loops(
            &canonical_loops,
            &top_boundary_segments,
            arrangement.piece_kind() == RoadSurfaceVisualNodePieceKind::JunctionN,
        )
    }

    fn arrangement_outer_boundary_segment_sources(
        arrangement: &NodeArrangement,
    ) -> Result<
        (
            Vec<ArrangementBoundarySegment>,
            Vec<ArrangementBoundarySegment>,
        ),
        NodeBoundaryExportError,
    > {
        let canonical_edges = Self::arrangement_canonical_boundary_edges(arrangement)?;
        let canonical_segments = canonical_edges
            .iter()
            .map(|edge| ArrangementBoundarySegment {
                start_key: edge.start_point_key,
                end_key: edge.end_point_key,
                start: edge.start,
                end: edge.end,
                owner: edge.owner,
            })
            .collect::<Vec<_>>();
        let mut edge_counts = BTreeMap::<ArrangementBoundaryEdgeKey, usize>::new();
        let mut top_segments = Vec::new();

        for face in arrangement.faces() {
            let Some(mut points) = Self::noded_arrangement_face_boundary(arrangement, face) else {
                return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
            };
            if points.len() < 3 {
                continue;
            }
            if Self::signed_polygon_area_xz(&points) < 0.0 {
                points.reverse();
            }

            for triangle in Self::triangulate_noded_arrangement_face(&points, face.owner().kind()) {
                for edge_index in 0..3 {
                    let start = triangle[edge_index];
                    let end = triangle[(edge_index + 1) % 3];
                    let start_key = ArrangementBoundaryPointKey::from_world(start);
                    let end_key = ArrangementBoundaryPointKey::from_world(end);
                    if start_key == end_key {
                        continue;
                    }
                    let edge_key = ArrangementBoundaryEdgeKey::new(start_key, end_key);
                    *edge_counts.entry(edge_key).or_default() += 1;
                    top_segments.push(ArrangementBoundarySegment {
                        start_key,
                        end_key,
                        start,
                        end,
                        owner: face.owner(),
                    });
                }
            }
        }

        let mut top_boundary_segments = top_segments
            .into_iter()
            .filter(|segment| {
                edge_counts
                    .get(&ArrangementBoundaryEdgeKey::new(
                        segment.start_key,
                        segment.end_key,
                    ))
                    .copied()
                    == Some(1)
            })
            .collect::<Vec<_>>();

        if canonical_segments.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }
        top_boundary_segments.sort_by(|a, b| {
            a.start_key
                .cmp(&b.start_key)
                .then(a.end_key.cmp(&b.end_key))
                .then(a.owner.kind().cmp(&b.owner.kind()))
                .then(a.owner.owner_index().cmp(&b.owner.owner_index()))
                .then(a.start.y.total_cmp(&b.start.y))
                .then(a.end.y.total_cmp(&b.end.y))
        });
        Ok((canonical_segments, top_boundary_segments))
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
                start_point_key,
                end_point_key,
                start_key,
                end_key,
                owner: edge.owner(),
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
                        arrangement_boundary_segment_has_endpoint(segments[*index], current.end_key)
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
                current =
                    oriented_arrangement_boundary_segment(segments[current_index], current.end_key)
                        .ok_or_else(|| NodeBoundaryExportError::AmbiguousOuterBoundaryOwner)?;
            }

            for simple_loop in split_boundary_segment_loop_at_repeated_xz(loop_segments) {
                let points = boundary_points_from_segment_loop(&simple_loop);
                let area_m2 = Self::signed_polygon_area_xz(&points).abs();
                if area_m2 <= NODE_OVERLAY_MIN_AREA_M2 {
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
        top_boundary_segments: &[ArrangementBoundarySegment],
        allow_top_boundary_paths: bool,
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, NodeBoundaryExportError> {
        let mut polygons = Vec::new();
        for canonical_loop in canonical_loops {
            let mut points = Vec::new();
            for &segment in canonical_loop {
                let expanded_segments = complete_top_segments_for_canonical_segment(
                    segment,
                    top_boundary_segments,
                    allow_top_boundary_paths,
                );
                let segment_iter: Box<dyn Iterator<Item = ArrangementBoundarySegment>> =
                    if expanded_segments.is_empty() {
                        Box::new(std::iter::once(segment))
                    } else {
                        Box::new(expanded_segments.into_iter())
                    };
                for expanded in segment_iter {
                    if points.is_empty() {
                        points.push(expanded.start);
                    }
                    points.push(expanded.end);
                }
            }
            let area_m2 = Self::signed_polygon_area_xz(&points).abs();
            if area_m2 <= NODE_OVERLAY_MIN_AREA_M2 {
                continue;
            }
            let Some(polygon) = Self::make_boundary_loop_polygon(points.clone())
                .or_else(|| boundary_loop_polygon_after_uncrossing(points.clone()))
            else {
                if allow_top_boundary_paths {
                    if let Some(fallback) = Self::make_boundary_loop_polygon(
                        boundary_points_from_segment_loop(canonical_loop),
                    ) {
                        polygons.push(fallback);
                        continue;
                    }
                }
                continue;
            };
            polygons.push(polygon);
        }
        (!polygons.is_empty())
            .then_some(polygons)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
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
            return (candidates.len() == 1).then_some(candidates[0]);
        }
        candidates.iter().copied().min_by(|a, b| {
            let segment_a = oriented_arrangement_boundary_segment(segments[*a], current.end_key)
                .unwrap_or(segments[*a]);
            let segment_b = oriented_arrangement_boundary_segment(segments[*b], current.end_key)
                .unwrap_or(segments[*b]);
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
        let mut points_world = Self::noded_arrangement_face_boundary(arrangement, face)?;
        points_world.dedup_by(|a, b| {
            (*a - *b).length_squared() <= super::WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
        });
        if points_world.len() < 3 {
            return None;
        }
        if Self::signed_polygon_area_xz(&points_world) < 0.0 {
            points_world.reverse();
        }
        let triangles_world =
            Self::triangulate_noded_arrangement_face(&points_world, face.owner().kind());
        Some(
            triangles_world
                .into_iter()
                .map(|triangle| RoadSurfaceVisualPolygon {
                    points_world: triangle.to_vec(),
                    triangles_world: vec![triangle],
                })
                .collect(),
        )
    }

    fn noded_arrangement_face_boundary(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
    ) -> Option<Vec<Vector3>> {
        let vertices = face.vertices();
        let mut boundary = Vec::new();
        for edge_index in 0..3 {
            let start_id = vertices[edge_index];
            let end_id = vertices[(edge_index + 1) % 3];
            let start = Self::arrangement_vertex_world(arrangement, start_id)?;
            let end = Self::arrangement_vertex_world(arrangement, end_id)?;
            boundary.push(start);
            let mut split_points = arrangement
                .vertices()
                .iter()
                .enumerate()
                .filter_map(|(candidate_index, candidate)| {
                    if candidate_index == start_id.index() || candidate_index == end_id.index() {
                        return None;
                    }
                    let candidate_world = super::backend::road_xz_with_height_to_godot(
                        candidate.point_xz(),
                        candidate.height_m(),
                    );
                    arrangement_vertex_on_triangle_edge(start, end, candidate_world)
                        .map(|t| (t, candidate_world))
                })
                .collect::<Vec<_>>();
            split_points.sort_by(|a, b| a.0.total_cmp(&b.0));
            boundary.extend(split_points.into_iter().map(|(_, point)| point));
        }
        Some(boundary)
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

    fn triangulate_noded_arrangement_face(
        points_world: &[Vector3],
        kind: RoadSurfaceBandKind,
    ) -> Vec<[Vector3; 3]> {
        if points_world.len() < 3 {
            return Vec::new();
        }
        let anchor = points_world[0];
        let mut triangles = Vec::with_capacity(points_world.len().saturating_sub(2));
        for index in 1..points_world.len() - 1 {
            let triangle = [anchor, points_world[index], points_world[index + 1]];
            let has_area = if kind == RoadSurfaceBandKind::Carriageway {
                Self::triangle_has_area_xz(triangle)
            } else {
                Self::signed_polygon_area_xz(&triangle).abs() > NODE_OVERLAY_MIN_AREA_M2
            };
            let area_3d_m2 = (triangle[1] - triangle[0])
                .cross(triangle[2] - triangle[0])
                .length()
                * 0.5;
            if has_area && area_3d_m2 >= NODE_OVERLAY_MIN_AREA_M2 {
                triangles.push(triangle);
            }
        }
        triangles
    }

    fn log_node_input_extraction_error(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeInputExtractionError,
    ) {
        crate::debug_log!(
            "road",
            "node_canonical_input_failed node={} piece={:?} error={:?}",
            node_id,
            kind,
            error
        );
    }

    fn log_node_validation_report(report: &NodeValidationReport) {
        if report.diagnostics.is_empty() {
            return;
        }
        crate::debug_log!("road", "node_canonical_validation {}", report.debug_dump());
    }

    fn log_node_boundary_export_error(
        arrangement: &NodeArrangement,
        error: &NodeBoundaryExportError,
    ) {
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
        Self::log_node_validation_report(&report);
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
        mut road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut owned_regions: Vec<NodeOwnedRegion>,
        mut earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_node_owned_regions(&mut owned_regions);
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
            road_surface_polygons,
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

fn arrangement_vertex_on_triangle_edge(
    start: Vector3,
    end: Vector3,
    candidate: Vector3,
) -> Option<f32> {
    let start_xz = Vector2::new(start.x, start.z);
    let end_xz = Vector2::new(end.x, end.z);
    let candidate_xz = Vector2::new(candidate.x, candidate.z);
    let segment = end_xz - start_xz;
    let length_squared = segment.length_squared();
    if length_squared <= SAMPLE_EPSILON_M {
        return None;
    }
    let t = (candidate_xz - start_xz).dot(segment) / length_squared;
    if !(0.0005..=0.9995).contains(&t) {
        return None;
    }
    let closest_xz = start_xz + segment * t;
    if candidate_xz.distance_squared_to(closest_xz)
        > ARRANGEMENT_EDGE_SPLIT_TOLERANCE_M * ARRANGEMENT_EDGE_SPLIT_TOLERANCE_M
    {
        return None;
    }
    let edge_height = start.y + (end.y - start.y) * t;
    ((candidate.y - edge_height).abs() <= ARRANGEMENT_EDGE_SPLIT_HEIGHT_TOLERANCE_M).then_some(t)
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
    let dx = i128::from(end.x_mm() - start.x_mm());
    let dz = i128::from(end.z_mm() - start.z_mm());
    let px = i128::from(point.x_mm() - start.x_mm());
    let pz = i128::from(point.z_mm() - start.z_mm());
    if px * dz - pz * dx != 0 {
        return false;
    }
    point.x_mm() >= start.x_mm().min(end.x_mm())
        && point.x_mm() <= start.x_mm().max(end.x_mm())
        && point.z_mm() >= start.z_mm().min(end.z_mm())
        && point.z_mm() <= start.z_mm().max(end.z_mm())
}

fn arrangement_boundary_segment_t(
    point: NodeArrangementKey,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> f64 {
    let dx = (end.x_mm() - start.x_mm()) as f64;
    let dz = (end.z_mm() - start.z_mm()) as f64;
    if dx.abs() >= dz.abs() && dx.abs() > f64::EPSILON {
        (point.x_mm() - start.x_mm()) as f64 / dx
    } else if dz.abs() > f64::EPSILON {
        (point.z_mm() - start.z_mm()) as f64 / dz
    } else {
        0.0
    }
}

fn complete_top_segments_for_canonical_segment(
    canonical: ArrangementBoundarySegment,
    top_boundary_segments: &[ArrangementBoundarySegment],
    allow_top_boundary_paths: bool,
) -> Vec<ArrangementBoundarySegment> {
    if !allow_top_boundary_paths {
        return Vec::new();
    }
    let mut segments = top_boundary_segments
        .iter()
        .copied()
        .filter(|segment| {
            arrangement_key_lies_on_segment(
                segment.start_key.xz_key(),
                canonical.start_key.xz_key(),
                canonical.end_key.xz_key(),
            ) && arrangement_key_lies_on_segment(
                segment.end_key.xz_key(),
                canonical.start_key.xz_key(),
                canonical.end_key.xz_key(),
            )
        })
        .map(|segment| oriented_segment_along_canonical_segment(segment, canonical))
        .collect::<Vec<_>>();
    segments.sort_by(|a, b| {
        arrangement_boundary_segment_t(
            a.start_key.xz_key(),
            canonical.start_key.xz_key(),
            canonical.end_key.xz_key(),
        )
        .total_cmp(&arrangement_boundary_segment_t(
            b.start_key.xz_key(),
            canonical.start_key.xz_key(),
            canonical.end_key.xz_key(),
        ))
        .then(a.end_key.cmp(&b.end_key))
        .then(a.owner.kind().cmp(&b.owner.kind()))
        .then(a.owner.owner_index().cmp(&b.owner.owner_index()))
    });
    segments.dedup_by_key(|segment| {
        ArrangementBoundaryEdgeKey::new(segment.start_key, segment.end_key)
    });
    top_segments_cover_canonical_segment(canonical, &segments)
        .then_some(segments)
        .or_else(|| {
            allow_top_boundary_paths
                .then(|| top_boundary_path_between(canonical, top_boundary_segments))
                .flatten()
        })
        .unwrap_or_default()
}

fn top_segments_cover_canonical_segment(
    canonical: ArrangementBoundarySegment,
    segments: &[ArrangementBoundarySegment],
) -> bool {
    let Some(first) = segments.first() else {
        return false;
    };
    let Some(last) = segments.last() else {
        return false;
    };
    if first.start_key.xz_key() != canonical.start_key.xz_key()
        || last.end_key.xz_key() != canonical.end_key.xz_key()
    {
        return false;
    }
    segments
        .windows(2)
        .all(|pair| pair[0].end_key.xz_key() == pair[1].start_key.xz_key())
}

fn top_boundary_path_between(
    canonical: ArrangementBoundarySegment,
    top_boundary_segments: &[ArrangementBoundarySegment],
) -> Option<Vec<ArrangementBoundarySegment>> {
    let start_key = canonical.start_key;
    let end_key = canonical.end_key;
    if start_key == end_key {
        return None;
    }

    let mut adjacency =
        BTreeMap::<ArrangementBoundaryPointKey, Vec<ArrangementBoundarySegment>>::new();
    for segment in top_boundary_segments {
        let forward = ArrangementBoundarySegment {
            owner: canonical.owner,
            ..*segment
        };
        let reverse = ArrangementBoundarySegment {
            start_key: segment.end_key,
            end_key: segment.start_key,
            start: segment.end,
            end: segment.start,
            owner: canonical.owner,
        };
        adjacency
            .entry(forward.start_key)
            .or_default()
            .push(forward);
        adjacency
            .entry(reverse.start_key)
            .or_default()
            .push(reverse);
    }
    for segments in adjacency.values_mut() {
        segments.sort_by(|a, b| {
            a.end_key
                .cmp(&b.end_key)
                .then(a.owner.kind().cmp(&b.owner.kind()))
                .then(a.owner.owner_index().cmp(&b.owner.owner_index()))
        });
    }

    let mut distances = BTreeMap::<ArrangementBoundaryPointKey, f32>::new();
    let mut previous = BTreeMap::<
        ArrangementBoundaryPointKey,
        (ArrangementBoundaryPointKey, ArrangementBoundarySegment),
    >::new();
    let mut settled = BTreeSet::<ArrangementBoundaryPointKey>::new();
    distances.insert(start_key, 0.0);

    while let Some((current_key, current_distance)) = distances
        .iter()
        .filter(|(key, _)| !settled.contains(key))
        .min_by(|(key_a, distance_a), (key_b, distance_b)| {
            distance_a.total_cmp(distance_b).then(key_a.cmp(key_b))
        })
        .map(|(key, distance)| (*key, *distance))
    {
        if current_key == end_key {
            break;
        }
        settled.insert(current_key);
        for segment in adjacency.get(&current_key).into_iter().flatten() {
            if settled.contains(&segment.end_key) {
                continue;
            }
            let length_m = Vector2::new(
                segment.end.x - segment.start.x,
                segment.end.z - segment.start.z,
            )
            .length();
            let candidate_distance = current_distance + length_m;
            let should_update = distances
                .get(&segment.end_key)
                .is_none_or(|distance| candidate_distance < *distance - SAMPLE_EPSILON_M);
            if should_update {
                distances.insert(segment.end_key, candidate_distance);
                previous.insert(segment.end_key, (current_key, *segment));
            }
        }
    }

    distances.get(&end_key)?;
    let mut path = Vec::new();
    let mut cursor = end_key;
    while cursor != start_key {
        let (previous_key, segment) = previous.get(&cursor).copied()?;
        path.push(segment);
        cursor = previous_key;
    }
    path.reverse();
    Some(path)
}

fn oriented_segment_along_canonical_segment(
    segment: ArrangementBoundarySegment,
    canonical: ArrangementBoundarySegment,
) -> ArrangementBoundarySegment {
    let start_t = arrangement_boundary_segment_t(
        segment.start_key.xz_key(),
        canonical.start_key.xz_key(),
        canonical.end_key.xz_key(),
    );
    let end_t = arrangement_boundary_segment_t(
        segment.end_key.xz_key(),
        canonical.start_key.xz_key(),
        canonical.end_key.xz_key(),
    );
    if start_t <= end_t {
        ArrangementBoundarySegment {
            owner: canonical.owner,
            ..segment
        }
    } else {
        ArrangementBoundarySegment {
            start_key: segment.end_key,
            end_key: segment.start_key,
            start: segment.end,
            end: segment.start,
            owner: canonical.owner,
        }
    }
}

fn arrangement_boundary_segment_has_endpoint(
    segment: ArrangementBoundarySegment,
    key: ArrangementBoundaryPointKey,
) -> bool {
    arrangement_boundary_point_same_xz(segment.start_key, key)
        || arrangement_boundary_point_same_xz(segment.end_key, key)
}

fn oriented_arrangement_boundary_segment(
    segment: ArrangementBoundarySegment,
    start_key: ArrangementBoundaryPointKey,
) -> Option<ArrangementBoundarySegment> {
    if arrangement_boundary_point_same_xz(segment.start_key, start_key) {
        return Some(segment);
    }
    if arrangement_boundary_point_same_xz(segment.end_key, start_key) {
        return Some(ArrangementBoundarySegment {
            start_key: segment.end_key,
            end_key: segment.start_key,
            start: segment.end,
            end: segment.start,
            owner: segment.owner,
        });
    }
    None
}

fn arrangement_boundary_point_same_xz(
    a: ArrangementBoundaryPointKey,
    b: ArrangementBoundaryPointKey,
) -> bool {
    a.x_mm == b.x_mm && a.z_mm == b.z_mm
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

fn uncross_boundary_loop_points(mut points: Vec<Vector3>) -> Vec<Vector3> {
    let len = points.len();
    if len < 4 {
        return points;
    }
    for _ in 0..len {
        let mut changed = false;
        'edges: for edge_a in 0..len {
            let edge_a_next = (edge_a + 1) % len;
            for edge_b in edge_a + 1..len {
                let edge_b_next = (edge_b + 1) % len;
                if edge_a == edge_b
                    || edge_a == edge_b_next
                    || edge_a_next == edge_b
                    || edge_a_next == edge_b_next
                {
                    continue;
                }
                if boundary_segments_strictly_intersect_xz(
                    points[edge_a],
                    points[edge_a_next],
                    points[edge_b],
                    points[edge_b_next],
                ) {
                    points[edge_a_next..=edge_b].reverse();
                    changed = true;
                    break 'edges;
                }
            }
        }
        if !changed {
            break;
        }
    }
    points
}

fn boundary_loop_polygon_after_uncrossing(
    points: Vec<Vector3>,
) -> Option<RoadSurfaceVisualPolygon> {
    let original_area = RoadSurfaceSystem::signed_polygon_area_xz(&points).abs();
    let uncrossed = uncross_boundary_loop_points(points.clone());
    if let Some(polygon) = RoadSurfaceSystem::make_boundary_loop_polygon(uncrossed) {
        let uncrossed_area = RoadSurfaceSystem::signed_polygon_area_xz(&polygon.points_world).abs();
        if (uncrossed_area - original_area).abs() <= 0.25 {
            return Some(polygon);
        }
    }
    boundary_loop_polygon_without_crossing_repair(points)
}

fn boundary_loop_polygon_without_crossing_repair(
    mut points_world: Vec<Vector3>,
) -> Option<RoadSurfaceVisualPolygon> {
    points_world.dedup_by(|a, b| {
        (*a - *b).length_squared() <= super::WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
    });
    if points_world.len() >= 2
        && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
            <= super::WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
    {
        points_world.pop();
    }
    if points_world.len() < 3 {
        return None;
    }
    let signed_area = RoadSurfaceSystem::signed_polygon_area_xz(&points_world);
    if signed_area.abs() <= NODE_OVERLAY_MIN_AREA_M2 {
        return None;
    }
    if signed_area < 0.0 {
        points_world.reverse();
    }
    let (start_index, _) = points_world.iter().enumerate().min_by(|(_, a), (_, b)| {
        a.x.total_cmp(&b.x)
            .then(a.z.total_cmp(&b.z))
            .then(a.y.total_cmp(&b.y))
    })?;
    points_world.rotate_left(start_index);
    Some(RoadSurfaceVisualPolygon {
        points_world,
        triangles_world: Vec::new(),
    })
}

fn boundary_segments_strictly_intersect_xz(a: Vector3, b: Vector3, c: Vector3, d: Vector3) -> bool {
    let ab_c = boundary_cross_points_xz(a, b, c);
    let ab_d = boundary_cross_points_xz(a, b, d);
    let cd_a = boundary_cross_points_xz(c, d, a);
    let cd_b = boundary_cross_points_xz(c, d, b);
    ab_c * ab_d < -SAMPLE_EPSILON_M && cd_a * cd_b < -SAMPLE_EPSILON_M
}

fn boundary_cross_points_xz(a: Vector3, b: Vector3, c: Vector3) -> f32 {
    (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
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
