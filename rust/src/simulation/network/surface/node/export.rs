//! Node surface export from canonical arrangement output.

use super::arrangement_faces::*;
use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn node_surface_regions_from_arrangement(
        arrangement: &NodeArrangement,
        footprint_shapes: &super::NodeOverlayShapes,
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
        let mut raised_step_faces = Self::raised_step_face_polygons_from_arrangement(
            arrangement,
            &explicit_vertical_step_segments,
        );

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

        let footprint_boundary_point_loops = Self::footprint_boundary_point_loops_from_shapes(
            &mut boundary_export_sources,
            footprint_shapes,
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
            Self::outer_boundary_polygons_from_point_loops(&footprint_boundary_point_loops)?;
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

    fn footprint_boundary_point_loops_from_shapes(
        boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
        footprint_shapes: &super::NodeOverlayShapes,
    ) -> Result<Vec<Vec<Vector3>>, NodeBoundaryExportError> {
        let mut loops = Vec::new();
        for shape in footprint_shapes {
            for contour in shape {
                let mut keyed_points = Vec::with_capacity(contour.len());
                for point in contour {
                    let key = NodeArrangementKey::from_point(super::backend::RoadVec2::new(
                        point[0], point[1],
                    ));
                    let height_mm = Self::arrangement_footprint_boundary_height_mm(
                        boundary_export_sources,
                        key,
                    )?;
                    keyed_points.push((key, height_mm));
                }
                boundary_export_sources
                    .interpolate_missing_authorized_footprint_boundary_heights(&mut keyed_points)?;
                let mut points = keyed_points
                    .into_iter()
                    .map(|(key, height_mm)| {
                        let height_mm = height_mm
                            .ok_or(NodeBoundaryExportError::MissingFootprintBoundaryHeight)?;
                        Ok(arrangement_boundary_point_to_world(
                            arrangement_key_boundary_point(key, height_mm),
                        ))
                    })
                    .collect::<Result<Vec<_>, NodeBoundaryExportError>>()?;
                remove_subbudget_unsupported_numeric_boundary_vertices(
                    &mut points,
                    |current_point_key, local_points| {
                        boundary_export_sources
                            .has_final_owned_footprint_boundary_support_at_point(current_point_key)
                            || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                                > boundary_points_numeric_area_budget_m2(&local_points)
                    },
                );
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
        boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
        key: NodeArrangementKey,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        boundary_export_sources.height_mm_at_key(key)
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

    fn visual_polygon_from_arrangement_face(
        arrangement: &NodeArrangement,
        face: &super::arrangement::NodeArrangementFace,
        authority_indices: &BTreeMap<super::node_grade::NodeGradeVertexAuthority, usize>,
    ) -> Result<
        Option<(RoadSurfaceVisualPolygon, NodeTopSurfacePolygonSource)>,
        NodeBoundaryExportError,
    > {
        let Some((triangle, vertex_ids)) =
            Self::arrangement_face_canonical_triangle_with_vertices(arrangement, face)
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

    pub(super) fn arrangement_face_canonical_triangle_with_vertices(
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
}
