//! Owned top-surface region export from triangulated arrangement faces.

use super::super::*;
use godot::prelude::Vector3;
use std::collections::BTreeMap;

impl RoadSurfaceSystem {
    pub(super) fn sort_node_owned_regions_with_sources(
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

    pub(super) fn top_polygons_from_owned_regions_by_material(
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

    pub(super) fn visual_polygon_from_arrangement_face(
        arrangement: &NodeArrangement,
        face: &arrangement::NodeArrangementFace,
        authority_indices: &BTreeMap<height::NodeGradeVertexAuthority, usize>,
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

    pub(in crate::simulation::network::surface::node) fn arrangement_face_canonical_vertex_ids(
        arrangement: &NodeArrangement,
        face: &arrangement::NodeArrangementFace,
    ) -> Option<[arrangement::NodeArrangementVertexId; 3]> {
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
        vertices: [arrangement::NodeArrangementVertexId; 3],
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
        vertex_id: arrangement::NodeArrangementVertexId,
    ) -> Option<Vector3> {
        let vertex = arrangement.vertices().get(vertex_id.index())?;
        let point_xz = vertex.point_xz();
        Some(backend::road_xz_with_height_to_godot(point_xz, 0.0))
    }

    pub(super) fn arrangement_vertex_world(
        arrangement: &NodeArrangement,
        vertex_id: arrangement::NodeArrangementVertexId,
    ) -> Option<Vector3> {
        let vertex = arrangement.vertices().get(vertex_id.index())?;
        Some(backend::road_xz_with_height_to_godot(
            vertex.point_xz(),
            vertex.height_m(),
        ))
    }
}
