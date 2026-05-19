//! Node surface export from canonical arrangement output.

use super::arrangement_faces::*;
use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn node_surface_regions_from_arrangement(
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
        Self::sort_node_owned_regions_with_sources(
            &mut owned_regions,
            &mut node_top_surface_sources,
        )?;
        let explicit_vertical_step_segments = arrangement.explicit_vertical_step_segments();
        let mut boundary_export_sources = NodeFootprintBoundaryExportSources::from_owned_regions(
            arrangement.node_id(),
            arrangement.piece_kind(),
            &owned_regions,
            &node_top_surface_sources,
            &explicit_vertical_step_segments,
        )?;
        boundary_export_sources.extend_arrangement_exposed_boundary_edges(arrangement)?;
        let mut raised_step_faces = Self::raised_step_face_polygons_from_arrangement(
            arrangement,
            &explicit_vertical_step_segments,
        );

        if owned_regions.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }

        let (mut road_surface_polygons, mut curb_surface_polygons, mut sidewalk_surface_polygons) =
            Self::top_polygons_from_owned_regions_by_material(&owned_regions);
        if road_surface_polygons.is_empty()
            && curb_surface_polygons.is_empty()
            && sidewalk_surface_polygons.is_empty()
        {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }
        let footprint_boundary_point_loops =
            Self::footprint_boundary_point_loops_from_arrangement_edges(
                arrangement,
                &mut boundary_export_sources,
            )?;
        let mut earthwork_boundary_segments =
            node_earthwork_boundary_segments_from_footprint_loops(
                arrangement.node_id(),
                arrangement.piece_kind(),
                &footprint_boundary_point_loops,
                &boundary_export_sources,
            )?;
        Self::orient_earthwork_boundary_segment_loops_by_nesting(&mut earthwork_boundary_segments);
        let mut outer_boundary_loops =
            Self::outer_boundary_polygons_from_arrangement_regions(arrangement)?;
        let mut terrain_clip_boundary_loops =
            Self::terrain_clip_boundary_loops_from_earthwork_segments(&earthwork_boundary_segments);

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
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

    fn footprint_boundary_point_loops_from_arrangement_edges(
        arrangement: &NodeArrangement,
        boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
    ) -> Result<Vec<Vec<NodeFootprintBoundaryPoint>>, NodeBoundaryExportError> {
        let mut boundary_edges = Vec::<FootprintBoundaryDirectedEdge>::new();
        for edge in arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
        {
            let Some(start_vertex) = arrangement.vertices().get(edge.start().index()) else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            let Some(end_vertex) = arrangement.vertices().get(edge.end().index()) else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            if start_vertex.key() == end_vertex.key() {
                continue;
            }
            let directed_edge = FootprintBoundaryDirectedEdge {
                start: footprint_boundary_point_from_arrangement_vertex(start_vertex),
                end: footprint_boundary_point_from_arrangement_vertex(end_vertex),
            };
            boundary_edges.push(directed_edge);
        }
        boundary_edges.sort_by(footprint_boundary_directed_edge_ordering);
        let mut adjacency = BTreeMap::<NodeArrangementKey, Vec<usize>>::new();
        for (edge_index, edge) in boundary_edges.iter().enumerate() {
            adjacency
                .entry(edge.start.xz_key())
                .or_default()
                .push(edge_index);
            adjacency
                .entry(edge.end.xz_key())
                .or_default()
                .push(edge_index);
        }
        for edges in adjacency.values_mut() {
            edges.sort_unstable();
            edges.dedup();
        }

        let mut loops = Vec::new();
        let mut emitted_loop_identities = BTreeSet::<Vec<ArrangementBoundaryPointKey>>::new();
        let mut visited_half_edges = BTreeSet::<(usize, bool)>::new();
        for edge_index in 0..boundary_edges.len() {
            for reversed in [false, true] {
                if visited_half_edges.contains(&(edge_index, reversed)) {
                    continue;
                }
                let Some(mut points) = trace_footprint_boundary_face(
                    &boundary_edges,
                    &adjacency,
                    &mut visited_half_edges,
                    edge_index,
                    reversed,
                )?
                else {
                    continue;
                };
                remove_subbudget_unsupported_numeric_boundary_vertices(
                    &mut points,
                    |current_point_key, local_points| {
                        boundary_export_sources
                            .has_exact_final_owned_footprint_boundary_support_at_point(
                                current_point_key,
                            )
                            || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                                > boundary_points_numeric_area_budget_m2(&local_points)
                    },
                );
                let points = canonicalize_footprint_boundary_point_loop(points);
                if points.len() < 3 {
                    continue;
                }
                if signed_footprint_boundary_point_loop_area_xz(&points).abs()
                    <= footprint_boundary_point_loop_numeric_area_budget_m2(&points)
                {
                    continue;
                }
                for split_points in same_winding_boundary_point_loops_from_loop(&points) {
                    if signed_footprint_boundary_point_loop_area_xz(&split_points).abs()
                        <= footprint_boundary_point_loop_numeric_area_budget_m2(&split_points)
                    {
                        continue;
                    }
                    if !emitted_loop_identities
                        .insert(footprint_boundary_point_loop_identity(&split_points))
                    {
                        continue;
                    }
                    for point in &split_points {
                        boundary_export_sources
                            .reject_boundary_vertex_height_conflict(point.xz_key())?;
                        if !boundary_export_sources
                            .has_exact_final_owned_footprint_boundary_support_at_point(
                                point.point_key,
                            )
                        {
                            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight {
                                x_key: point.point_key.x_key,
                                z_key: point.point_key.z_key,
                            });
                        }
                    }
                    loops.push(split_points);
                }
            }
        }
        (!loops.is_empty())
            .then_some(loops)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }

    fn top_polygons_from_owned_regions_by_material(
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

    fn outer_boundary_polygons_from_arrangement_regions(
        arrangement: &NodeArrangement,
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, NodeBoundaryExportError> {
        let mut polygons = Vec::new();
        for region in arrangement.regions() {
            let points = region
                .outer_loop()
                .iter()
                .copied()
                .filter_map(|vertex_id| Self::arrangement_vertex_world(arrangement, vertex_id))
                .collect::<Vec<_>>();
            if points.len() < 3 {
                continue;
            }
            if Self::signed_polygon_area_xz(&points).abs()
                <= boundary_points_numeric_area_budget_m2(&points)
            {
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

    fn visual_polygon_from_arrangement_face(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
        authority_indices: &BTreeMap<super::height::NodeGradeVertexAuthority, usize>,
    ) -> Result<
        Option<(RoadSurfaceVisualPolygon, NodeTopSurfacePolygonSource)>,
        NodeBoundaryExportError,
    > {
        let Some(vertex_ids) = Self::arrangement_face_canonical_vertex_ids(arrangement, face)
        else {
            return Ok(None);
        };
        let Some(triangle) = Self::arrangement_face_world_triangle(arrangement, vertex_ids) else {
            return Ok(None);
        };
        let Some(region) = arrangement.regions().get(face.region().index()) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        let mut vertex_sources = Vec::with_capacity(vertex_ids.len());
        let mut vertex_keys = Vec::with_capacity(vertex_ids.len());
        let mut vertex_height_mm = Vec::with_capacity(vertex_ids.len());
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
            vertex_keys.push(vertex.key());
            vertex_height_mm.push(vertex.height_mm());
        }
        let triangle_sources = vec![[vertex_sources[0], vertex_sources[1], vertex_sources[2]]];
        let source = NodeTopSurfacePolygonSource {
            kind: face.owner().kind(),
            owner_index: face.owner().owner_index(),
            height_field_id: region.height_field_id(),
            vertex_keys,
            vertex_height_mm,
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

    pub(super) fn arrangement_face_canonical_vertex_ids(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
    ) -> Option<[super::arrangement::NodeArrangementVertexId; 3]> {
        let mut vertices = face.vertices();
        let triangle = [
            Self::arrangement_vertex_flat_world(arrangement, vertices[0])?,
            Self::arrangement_vertex_flat_world(arrangement, vertices[1])?,
            Self::arrangement_vertex_flat_world(arrangement, vertices[2])?,
        ];
        let signed_area = Self::signed_polygon_area_xz(&triangle);
        if signed_area.abs() <= NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if signed_area < 0.0 {
            vertices.swap(1, 2);
        }
        Some(vertices)
    }

    fn arrangement_face_world_triangle(
        arrangement: &NodeArrangement,
        vertices: [super::arrangement::NodeArrangementVertexId; 3],
    ) -> Option<[Vector3; 3]> {
        let triangle = [
            Self::arrangement_vertex_world(arrangement, vertices[0])?,
            Self::arrangement_vertex_world(arrangement, vertices[1])?,
            Self::arrangement_vertex_world(arrangement, vertices[2])?,
        ];
        let area_3d_m2 = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .length()
            * 0.5;
        if area_3d_m2 < NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        Some(triangle)
    }

    fn arrangement_vertex_flat_world(
        arrangement: &NodeArrangement,
        vertex_id: super::arrangement::NodeArrangementVertexId,
    ) -> Option<Vector3> {
        let vertex = arrangement.vertices().get(vertex_id.index())?;
        let point_xz = vertex.point_xz();
        Some(super::backend::road_xz_with_height_to_godot(point_xz, 0.0))
    }

    pub(super) fn arrangement_vertex_world(
        arrangement: &NodeArrangement,
        vertex_id: super::arrangement::NodeArrangementVertexId,
    ) -> Option<Vector3> {
        let vertex = arrangement.vertices().get(vertex_id.index())?;
        Some(super::backend::road_xz_with_height_to_godot(
            vertex.point_xz(),
            vertex.height_m(),
        ))
    }

    pub(super) fn assemble_explicit_node_piece(
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
        node_grade_authorities: Vec<super::height::NodeGradeVertexAuthority>,
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
}

fn canonicalize_footprint_boundary_point_loop(
    mut points: Vec<NodeFootprintBoundaryPoint>,
) -> Vec<NodeFootprintBoundaryPoint> {
    points.dedup_by(|a, b| a.point_key == b.point_key);
    if points.len() >= 2
        && points.first().map(|point| point.point_key) == points.last().map(|point| point.point_key)
    {
        points.pop();
    }
    points
}

fn footprint_boundary_point_loop_identity(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<ArrangementBoundaryPointKey> {
    let keys = points
        .iter()
        .map(|point| point.point_key)
        .collect::<Vec<_>>();
    let forward = canonical_footprint_boundary_loop_rotation(&keys);
    let mut reversed = keys;
    reversed.reverse();
    let reversed = canonical_footprint_boundary_loop_rotation(&reversed);
    forward.min(reversed)
}

fn canonical_footprint_boundary_loop_rotation(
    keys: &[ArrangementBoundaryPointKey],
) -> Vec<ArrangementBoundaryPointKey> {
    if keys.is_empty() {
        return Vec::new();
    }
    let start_index = keys
        .iter()
        .enumerate()
        .min_by_key(|(_, key)| **key)
        .map(|(index, _)| index)
        .unwrap_or(0);
    keys[start_index..]
        .iter()
        .chain(&keys[..start_index])
        .copied()
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct FootprintBoundaryDirectedEdge {
    start: NodeFootprintBoundaryPoint,
    end: NodeFootprintBoundaryPoint,
}

impl FootprintBoundaryDirectedEdge {
    fn reversed(self) -> Self {
        Self {
            start: self.end,
            end: self.start,
        }
    }
}

fn trace_footprint_boundary_face(
    edges: &[FootprintBoundaryDirectedEdge],
    adjacency: &BTreeMap<NodeArrangementKey, Vec<usize>>,
    visited_half_edges: &mut BTreeSet<(usize, bool)>,
    first_edge_index: usize,
    first_reversed: bool,
) -> Result<Option<Vec<NodeFootprintBoundaryPoint>>, NodeBoundaryExportError> {
    let first_edge = oriented_footprint_boundary_edge(edges[first_edge_index], first_reversed);
    let first_point_key = first_edge.start.point_key;
    let mut points = Vec::new();
    let mut local_visited_half_edges = BTreeSet::<(usize, bool)>::new();
    let mut current_edge_index = first_edge_index;
    let mut current_reversed = first_reversed;
    loop {
        if visited_half_edges.contains(&(current_edge_index, current_reversed)) {
            return Ok(None);
        }
        if !local_visited_half_edges.insert((current_edge_index, current_reversed)) {
            return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
        }
        let current_edge =
            oriented_footprint_boundary_edge(edges[current_edge_index], current_reversed);
        points.push(current_edge.start);
        let Some((next_edge_index, next_reversed, next_edge)) = next_footprint_boundary_half_edge(
            edges,
            adjacency,
            visited_half_edges,
            &local_visited_half_edges,
            (first_edge_index, first_reversed),
            current_edge_index,
            current_edge,
        ) else {
            return Ok(None);
        };
        if (next_edge_index, next_reversed) == (first_edge_index, first_reversed) {
            if current_edge.end.point_key != first_point_key {
                points.push(current_edge.end);
            }
            visited_half_edges.extend(local_visited_half_edges);
            return Ok(Some(points));
        }
        if next_edge.start.point_key != current_edge.end.point_key {
            points.push(current_edge.end);
        }
        current_edge_index = next_edge_index;
        current_reversed = next_reversed;
    }
}

fn next_footprint_boundary_half_edge(
    edges: &[FootprintBoundaryDirectedEdge],
    adjacency: &BTreeMap<NodeArrangementKey, Vec<usize>>,
    visited_half_edges: &BTreeSet<(usize, bool)>,
    local_visited_half_edges: &BTreeSet<(usize, bool)>,
    first_half_edge: (usize, bool),
    current_edge_index: usize,
    current_edge: FootprintBoundaryDirectedEdge,
) -> Option<(usize, bool, FootprintBoundaryDirectedEdge)> {
    let current_xz = current_edge.end.xz_key();
    let incident_edges = adjacency.get(&current_xz)?;
    let mut candidates = incident_edges
        .iter()
        .copied()
        .filter(|edge_index| *edge_index != current_edge_index)
        .filter_map(|edge_index| {
            let (reversed, edge) =
                oriented_footprint_boundary_edge_from_xz(edges[edge_index], current_xz)?;
            let half_edge = (edge_index, reversed);
            (half_edge == first_half_edge
                || (!visited_half_edges.contains(&half_edge)
                    && !local_visited_half_edges.contains(&half_edge)))
            .then_some((edge_index, reversed, edge))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|a, b| {
        let a_exact = a.2.start.point_key == current_edge.end.point_key;
        let b_exact = b.2.start.point_key == current_edge.end.point_key;
        b_exact
            .cmp(&a_exact)
            .then(
                footprint_boundary_turn_ordering(current_edge, b.2)
                    .total_cmp(&footprint_boundary_turn_ordering(current_edge, a.2)),
            )
            .then(a.2.start.point_key.cmp(&b.2.start.point_key))
            .then(a.2.end.point_key.cmp(&b.2.end.point_key))
    });
    candidates.into_iter().next()
}

fn oriented_footprint_boundary_edge(
    edge: FootprintBoundaryDirectedEdge,
    reversed: bool,
) -> FootprintBoundaryDirectedEdge {
    if reversed { edge.reversed() } else { edge }
}

fn oriented_footprint_boundary_edge_from_xz(
    edge: FootprintBoundaryDirectedEdge,
    start_xz: NodeArrangementKey,
) -> Option<(bool, FootprintBoundaryDirectedEdge)> {
    if edge.start.xz_key() == start_xz {
        Some((false, edge))
    } else if edge.end.xz_key() == start_xz {
        Some((true, edge.reversed()))
    } else {
        None
    }
}

fn footprint_boundary_turn_ordering(
    current: FootprintBoundaryDirectedEdge,
    candidate: FootprintBoundaryDirectedEdge,
) -> f64 {
    let current_start = current.start.point_world();
    let current_end = current.end.point_world();
    let candidate_end = candidate.end.point_world();
    let back_x = f64::from(current_start.x - current_end.x);
    let back_z = f64::from(current_start.z - current_end.z);
    let out_x = f64::from(candidate_end.x - current_end.x);
    let out_z = f64::from(candidate_end.z - current_end.z);
    let back_angle = back_z.atan2(back_x);
    let out_angle = out_z.atan2(out_x);
    (back_angle - out_angle).rem_euclid(std::f64::consts::TAU)
}

fn footprint_boundary_directed_edge_ordering(
    a: &FootprintBoundaryDirectedEdge,
    b: &FootprintBoundaryDirectedEdge,
) -> std::cmp::Ordering {
    a.start
        .point_key
        .cmp(&b.start.point_key)
        .then(a.end.point_key.cmp(&b.end.point_key))
}

fn footprint_boundary_point_from_arrangement_vertex(
    vertex: &super::arrangement::NodeArrangementVertex,
) -> NodeFootprintBoundaryPoint {
    NodeFootprintBoundaryPoint::new(arrangement_key_boundary_point(
        vertex.key(),
        vertex.height_mm(),
    ))
}

fn footprint_boundary_point_loop_world_points(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<Vector3> {
    points.iter().map(|point| point.point_world()).collect()
}

fn signed_footprint_boundary_point_loop_area_xz(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    RoadSurfaceSystem::signed_polygon_area_xz(&footprint_boundary_point_loop_world_points(points))
}

fn footprint_boundary_point_loop_numeric_area_budget_m2(
    points: &[NodeFootprintBoundaryPoint],
) -> f32 {
    super::boundary_points_numeric_area_budget_m2(&footprint_boundary_point_loop_world_points(
        points,
    ))
}
