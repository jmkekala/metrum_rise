//! Node surface export from canonical arrangement output.

use super::*;

mod footprint_loops;
mod raised_step_support;
mod terrain_clip_loops;

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
        Self::retain_raised_step_faces_with_owned_top_support(
            &mut raised_step_faces,
            &owned_regions,
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
        Self::orient_earthwork_boundary_segment_loops_by_nesting(&mut earthwork_boundary_segments)
            .map_err(|_| NodeBoundaryExportError::DegenerateOuterBoundaryLoop)?;
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
        mut earthwork_owner_sources: Vec<NodeEarthworkOwnerSource>,
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
        earthwork_owner_sources.sort_by(|a, b| {
            a.owner_kind
                .cmp(&b.owner_kind)
                .then(a.owner_index.cmp(&b.owner_index))
                .then(a.mouth_order_index.cmp(&b.mouth_order_index))
                .then(a.edge_idx.cmp(&b.edge_idx))
        });
        earthwork_owner_sources.dedup_by(|a, b| {
            a.owner_kind == b.owner_kind
                && a.owner_index == b.owner_index
                && a.mouth_order_index == b.mouth_order_index
                && a.edge_idx == b.edge_idx
                && a.edge_class == b.edge_class
        });
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
            earthwork_owner_sources,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }
}
