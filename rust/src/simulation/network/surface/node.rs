//! Explicit visual node-piece construction and incident-edge classification.

use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayPoint, NodeOverlayPointKey, NodeOverlayShapes,
    NodeOwnedRegion, OrderedIncidentPieceMouth, RoadSurfaceBandKind,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceVisualNodePiece,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    arrangement::NodeArrangement, backend::ROAD_OVERLAY_COORDINATE_SCALE,
    height::NodeHeightedRegion, input::NodeInputExtractionError, validation::NodeValidationReport,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const ARRANGEMENT_EDGE_SPLIT_TOLERANCE_M: f32 = 0.02;
const ARRANGEMENT_EDGE_SPLIT_HEIGHT_TOLERANCE_M: f32 = 0.004;

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

        match Self::node_surface_regions_from_arrangement(
            &arrangement,
            &ownership.footprint_shapes,
            &heights.regions,
        ) {
            Some(regions) => Some(regions),
            None => {
                Self::log_node_validation_report(
                    &NodeValidationReport::from_boundary_export_error(
                        node_id,
                        kind,
                        "outer_boundary_extraction_failed",
                    ),
                );
                None
            }
        }
    }

    fn node_surface_regions_from_arrangement(
        arrangement: &NodeArrangement,
        footprint_shapes: &NodeOverlayShapes,
        height_regions: &[NodeHeightedRegion],
    ) -> Option<super::NodeSurfaceRegionResult> {
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        let mut owned_regions = Vec::new();

        for face in arrangement.faces() {
            let owner = face.owner();
            let Some(polygons) = Self::visual_polygons_from_arrangement_face(arrangement, face)
            else {
                return None;
            };
            for polygon in polygons {
                if owner.kind() == RoadSurfaceBandKind::Carriageway {
                    road_surface_polygons.push(polygon.clone());
                } else {
                    sidewalk_surface_polygons.push(polygon.clone());
                }
                owned_regions.push(NodeOwnedRegion {
                    kind: owner.kind(),
                    owner_index: owner.owner_index(),
                    polygon,
                });
            }
        }

        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }

        let mut outer_boundary_loops =
            Self::outer_boundary_loops_from_footprint_shapes(footprint_shapes, height_regions)?;
        Self::align_outer_boundary_loop_heights_to_visible_top(
            &mut outer_boundary_loops,
            &road_surface_polygons,
            &sidewalk_surface_polygons,
        );
        if Self::should_node_outer_boundary_loop_edges(arrangement.piece_kind(), height_regions) {
            Self::node_outer_boundary_loop_edges_with_visible_top_vertices(
                &mut outer_boundary_loops,
                &road_surface_polygons,
                &sidewalk_surface_polygons,
            )?;
        }

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_node_owned_regions(&mut owned_regions);

        Some(super::NodeSurfaceRegionResult {
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            owned_regions,
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

    fn node_outer_boundary_loop_edges_with_visible_top_vertices(
        outer_boundary_loops: &mut [RoadSurfaceVisualPolygon],
        road_surface_polygons: &[RoadSurfaceVisualPolygon],
        sidewalk_surface_polygons: &[RoadSurfaceVisualPolygon],
    ) -> Option<()> {
        let candidates = Self::visible_top_boundary_candidate_vertices(
            road_surface_polygons,
            sidewalk_surface_polygons,
        );
        if candidates.is_empty() {
            return Some(());
        }

        for polygon in outer_boundary_loops {
            let original_points = polygon.points_world.clone();
            let mut noded_points = Vec::new();
            for edge_index in 0..original_points.len() {
                let start = original_points[edge_index];
                let end = original_points[(edge_index + 1) % original_points.len()];
                noded_points.push(start);
                let mut split_points = candidates
                    .iter()
                    .filter_map(|candidate| {
                        boundary_vertex_on_edge_xz(start, end, *candidate).map(|t| {
                            (
                                t,
                                Vector3::new(
                                    start.x + (end.x - start.x) * t,
                                    candidate.y,
                                    start.z + (end.z - start.z) * t,
                                ),
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                split_points.sort_by(|a, b| a.0.total_cmp(&b.0));
                split_points.dedup_by(|a, b| {
                    (a.0 - b.0).abs() <= 0.0001
                        || (a.1 - b.1).length_squared()
                            <= super::WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
                });
                noded_points.extend(split_points.into_iter().map(|(_, point)| point));
            }
            let area_m2 = Self::signed_polygon_area_xz(&noded_points).abs();
            if area_m2 <= NODE_OVERLAY_MIN_AREA_M2 {
                continue;
            }
            let area_budget_m2 = Self::overlay_numeric_area_budget_for_world_loop(&noded_points);
            let Some(noded_polygon) = Self::make_boundary_loop_polygon(noded_points) else {
                if area_m2 <= area_budget_m2 {
                    continue;
                }
                return None;
            };
            *polygon = noded_polygon;
        }
        Some(())
    }

    fn should_node_outer_boundary_loop_edges(
        piece_kind: RoadSurfaceVisualNodePieceKind,
        height_regions: &[NodeHeightedRegion],
    ) -> bool {
        match piece_kind {
            RoadSurfaceVisualNodePieceKind::Bend => true,
            RoadSurfaceVisualNodePieceKind::JunctionN => {
                let mut mouth_indices = height_regions
                    .iter()
                    .map(|region| region.source_mouth_order_index)
                    .collect::<Vec<_>>();
                mouth_indices.sort_unstable();
                mouth_indices.dedup();
                mouth_indices.len() <= 3
            }
            RoadSurfaceVisualNodePieceKind::Terminal => false,
        }
    }

    fn visible_top_boundary_candidate_vertices(
        road_surface_polygons: &[RoadSurfaceVisualPolygon],
        sidewalk_surface_polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<Vector3> {
        let mut vertices = BTreeMap::<NodeOverlayPointKey, Vector3>::new();
        for point in road_surface_polygons
            .iter()
            .chain(sidewalk_surface_polygons.iter())
            .flat_map(|polygon| {
                polygon.points_world.iter().chain(
                    polygon
                        .triangles_world
                        .iter()
                        .flat_map(|triangle| triangle.iter()),
                )
            })
        {
            let key = Self::world_boundary_point_key(*point);
            vertices
                .entry(key)
                .and_modify(|existing| {
                    if point.y > existing.y {
                        *existing = *point;
                    }
                })
                .or_insert(*point);
        }
        vertices.into_values().collect()
    }

    fn outer_boundary_loops_from_footprint_shapes(
        footprint_shapes: &NodeOverlayShapes,
        height_regions: &[NodeHeightedRegion],
    ) -> Option<Vec<RoadSurfaceVisualPolygon>> {
        let boundary_heights = Self::heighted_region_boundary_heights_by_key(height_regions);
        let mut polygons = Vec::new();
        for shape in footprint_shapes {
            for contour in shape {
                let boundary_points = contour
                    .iter()
                    .copied()
                    .map(|point| {
                        let key = Self::overlay_boundary_point_key(point);
                        let point_xz = super::backend::overlay_point_to_road(point);
                        (point_xz, boundary_heights.get(&key).copied())
                    })
                    .collect::<Vec<_>>();
                let points = Self::resolve_boundary_contour_heights(&boundary_points)?;
                let area_m2 = Self::signed_polygon_area_xz(&points).abs();
                if area_m2 <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                };
                let area_budget_m2 = Self::overlay_numeric_area_budget_for_world_loop(&points);
                let Some(polygon) = Self::make_boundary_loop_polygon(points) else {
                    if area_m2 <= area_budget_m2 {
                        continue;
                    }
                    return None;
                };
                polygons.push(polygon);
            }
        }
        (!polygons.is_empty()).then_some(polygons)
    }

    fn resolve_boundary_contour_heights(
        points: &[(super::backend::RoadVec2, Option<f64>)],
    ) -> Option<Vec<Vector3>> {
        if points.len() < 3 {
            return None;
        }
        let known_indices = points
            .iter()
            .enumerate()
            .filter_map(|(index, (_, height))| height.is_some().then_some(index))
            .collect::<Vec<_>>();
        if known_indices.is_empty() {
            return None;
        }

        let mut heights = vec![0.0; points.len()];
        for &index in &known_indices {
            heights[index] = points[index].1?;
        }
        if known_indices.len() < points.len() {
            for pair_index in 0..known_indices.len() {
                let start_index = known_indices[pair_index];
                let end_index = known_indices[(pair_index + 1) % known_indices.len()];
                if Self::next_loop_index(start_index, points.len()) == end_index {
                    continue;
                }
                let start_height = points[start_index].1?;
                let end_height = points[end_index].1?;
                let total_distance =
                    Self::contour_distance_between(points, start_index, end_index)?;
                if total_distance <= f64::EPSILON {
                    return None;
                }
                let mut cursor = start_index;
                let mut walked = 0.0;
                loop {
                    let next = Self::next_loop_index(cursor, points.len());
                    walked += points[cursor].0.distance(points[next].0);
                    if next == end_index {
                        break;
                    }
                    let t = (walked / total_distance).clamp(0.0, 1.0);
                    heights[next] = start_height + (end_height - start_height) * t;
                    cursor = next;
                }
            }
        }

        Some(
            points
                .iter()
                .enumerate()
                .map(|(index, (point_xz, _))| {
                    super::backend::road_xz_with_height_to_godot(*point_xz, heights[index])
                })
                .collect(),
        )
    }

    fn contour_distance_between(
        points: &[(super::backend::RoadVec2, Option<f64>)],
        start_index: usize,
        end_index: usize,
    ) -> Option<f64> {
        let mut cursor = start_index;
        let mut distance = 0.0;
        for _ in 0..points.len() {
            let next = Self::next_loop_index(cursor, points.len());
            distance += points[cursor].0.distance(points[next].0);
            if next == end_index {
                return Some(distance);
            }
            cursor = next;
        }
        None
    }

    fn next_loop_index(index: usize, len: usize) -> usize {
        if index + 1 == len { 0 } else { index + 1 }
    }

    fn heighted_region_boundary_heights_by_key(
        regions: &[NodeHeightedRegion],
    ) -> BTreeMap<NodeOverlayPointKey, f64> {
        let mut heights = BTreeMap::new();
        for region in regions {
            for contour in &region.shape {
                for vertex in contour {
                    let key = Self::heighted_boundary_point_key(vertex.point_xz);
                    heights
                        .entry(key)
                        .and_modify(|height: &mut f64| *height = height.max(vertex.height_m))
                        .or_insert(vertex.height_m);
                }
            }
        }
        heights
    }

    fn overlay_boundary_point_key(point: NodeOverlayPoint) -> NodeOverlayPointKey {
        (
            (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
            (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        )
    }

    fn heighted_boundary_point_key(point: super::backend::RoadVec2) -> NodeOverlayPointKey {
        (
            (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
            (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        )
    }

    fn world_boundary_point_key(point: Vector3) -> NodeOverlayPointKey {
        (
            (f64::from(point.x) * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
            (f64::from(point.z) * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        )
    }

    fn align_outer_boundary_loop_heights_to_visible_top(
        outer_boundary_loops: &mut [RoadSurfaceVisualPolygon],
        road_surface_polygons: &[RoadSurfaceVisualPolygon],
        sidewalk_surface_polygons: &[RoadSurfaceVisualPolygon],
    ) {
        for point in outer_boundary_loops
            .iter_mut()
            .flat_map(|polygon| polygon.points_world.iter_mut())
        {
            if let Some(height) = Self::visible_top_height_at_boundary_point(
                *point,
                sidewalk_surface_polygons,
                road_surface_polygons,
            ) {
                point.y = height;
            }
        }
    }

    fn visible_top_height_at_boundary_point(
        point: Vector3,
        primary_polygons: &[RoadSurfaceVisualPolygon],
        fallback_polygons: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        Self::visible_top_vertex_height_at_boundary_point(point, primary_polygons)
            .or_else(|| Self::visible_top_vertex_height_at_boundary_point(point, fallback_polygons))
            .or_else(|| {
                Self::visible_top_triangle_height_at_boundary_point(point, primary_polygons)
            })
            .or_else(|| {
                Self::visible_top_triangle_height_at_boundary_point(point, fallback_polygons)
            })
    }

    fn visible_top_vertex_height_at_boundary_point(
        point: Vector3,
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        let tolerance_m = SAMPLE_EPSILON_M * 2.0;
        polygons
            .iter()
            .flat_map(|polygon| {
                polygon.points_world.iter().chain(
                    polygon
                        .triangles_world
                        .iter()
                        .flat_map(|triangle| triangle.iter()),
                )
            })
            .filter_map(|candidate| {
                let distance_m =
                    Vector2::new(candidate.x - point.x, candidate.z - point.z).length();
                (distance_m <= tolerance_m).then_some((distance_m, candidate.y))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, height)| height)
    }

    fn visible_top_triangle_height_at_boundary_point(
        point: Vector3,
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        let point_xz = Vector2::new(point.x, point.z);
        polygons.iter().find_map(|polygon| {
            polygon.triangles_world.iter().find_map(|&triangle| {
                Self::triangle_barycentric_weights_xz(triangle, point_xz).map(|(wa, wb, wc)| {
                    triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc
                })
            })
        })
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

fn boundary_vertex_on_edge_xz(start: Vector3, end: Vector3, candidate: Vector3) -> Option<f32> {
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
    (candidate_xz.distance_squared_to(closest_xz) <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
        .then_some(t)
}
