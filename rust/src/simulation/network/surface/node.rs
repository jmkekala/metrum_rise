//! Explicit visual node-piece construction and incident-edge classification.

use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayPoint, NodeOverlayPointKey, NodeOverlayShapes,
    NodeOwnedRegion, OrderedIncidentPieceMouth, RoadSurfaceBandKind,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceVisualNodePiece,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    backend::{ROAD_OVERLAY_COORDINATE_SCALE, road_vec3_to_godot},
    height::NodeHeightedRegion,
    input::NodeInputExtractionError,
    triangulation::{NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulationSolution},
    validation::NodeValidationReport,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;

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

        match Self::node_surface_regions_from_triangulation(
            &triangulation,
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

    fn node_surface_regions_from_triangulation(
        triangulation: &NodeTriangulationSolution,
        footprint_shapes: &NodeOverlayShapes,
        height_regions: &[NodeHeightedRegion],
    ) -> Option<super::NodeSurfaceRegionResult> {
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        let mut owned_regions = Vec::new();

        for (region_index, region) in triangulation.regions.iter().enumerate() {
            let polygons = Self::visual_polygons_from_triangulated_region(region)?;
            for polygon in polygons {
                if region.kind == RoadSurfaceBandKind::Carriageway {
                    road_surface_polygons.push(polygon.clone());
                } else {
                    sidewalk_surface_polygons.push(polygon.clone());
                }
                owned_regions.push(NodeOwnedRegion {
                    kind: region.kind,
                    owner_index: region_index,
                    polygon,
                });
            }
        }

        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }

        let mut outer_boundary_loops =
            Self::outer_boundary_loops_from_footprint_shapes(footprint_shapes, height_regions)?;

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

    fn visual_polygons_from_triangulated_region(
        region: &NodeTriangulatedRegion,
    ) -> Option<Vec<RoadSurfaceVisualPolygon>> {
        let mut polygons = Vec::with_capacity(region.triangles.len());
        for triangle in &region.triangles {
            let triangle_world = Self::triangle_world_from_triangulated_region(region, *triangle)?;
            if !Self::triangle_has_area_xz(triangle_world) {
                continue;
            }
            polygons.push(RoadSurfaceVisualPolygon {
                points_world: triangle_world.to_vec(),
                triangles_world: vec![triangle_world],
            });
        }
        (!polygons.is_empty()).then_some(polygons)
    }

    fn triangle_world_from_triangulated_region(
        region: &NodeTriangulatedRegion,
        triangle: NodeTriangulatedTriangle,
    ) -> Option<[Vector3; 3]> {
        let mut triangle_world = [
            road_vec3_to_godot(region.vertices.get(triangle.vertices[0])?.point_world),
            road_vec3_to_godot(region.vertices.get(triangle.vertices[1])?.point_world),
            road_vec3_to_godot(region.vertices.get(triangle.vertices[2])?.point_world),
        ];
        if Self::signed_polygon_area_xz(&triangle_world) < 0.0 {
            triangle_world.swap(1, 2);
        }
        Some(triangle_world)
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
