//! Owned top-surface region export from triangulated arrangement faces.

use super::super::*;
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
        top_height_context: &NodeExportTopHeightContext,
    ) -> Result<
        Option<(RoadSurfaceVisualPolygon, NodeTopSurfacePolygonSource)>,
        NodeBoundaryExportError,
    > {
        let Some(vertex_ids) = Self::arrangement_face_canonical_vertex_ids(arrangement, face)
        else {
            return Ok(None);
        };
        let Some(mut triangle) = Self::arrangement_face_world_triangle(arrangement, vertex_ids)
        else {
            return Ok(None);
        };
        if Self::top_surface_triangle_is_numeric_dust(&triangle) {
            return Ok(None);
        }
        let Some(region) = arrangement.regions().get(face.region().index()) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        let mut vertex_sources = Vec::with_capacity(vertex_ids.len());
        let mut vertex_keys = Vec::with_capacity(vertex_ids.len());
        let mut vertex_height_mm = Vec::with_capacity(vertex_ids.len());
        for (point_index, vertex_id) in vertex_ids.into_iter().enumerate() {
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
            let source_kind = vertex
                .grade_authority()
                .source_provenance
                .map_or(vertex.height_field_id().kind(), |provenance| {
                    provenance.source_kind
                });
            let source_kind = super::super::node_export_top_source_kind(
                face.owner(),
                source_kind,
                vertex.key(),
                vertex.height_mm(),
                top_height_context,
            );
            triangle[point_index].y = super::super::node_export_top_height_m(
                face.owner(),
                source_kind,
                vertex.key(),
                vertex.height_m(),
                vertex.height_mm(),
                top_height_context,
            );
            vertex_height_mm.push(super::super::node_export_top_height_mm(
                face.owner(),
                source_kind,
                vertex.key(),
                vertex.height_mm(),
                top_height_context,
            ));
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
            RoadSurfaceVisualPolygon::from_parts(triangle.to_vec(), vec![triangle]),
            source,
        )))
    }

    pub(in crate::simulation::network::surface::node) fn arrangement_face_canonical_vertex_ids(
        arrangement: &NodeArrangement,
        face: &arrangement::NodeArrangementFace,
    ) -> Option<[arrangement::NodeArrangementVertexId; 3]> {
        let mut vertices = face.vertices();
        let signed_area = arrangement_face_signed_area_xz_m2(arrangement, vertices)?;
        if signed_area.abs() <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
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
    ) -> Option<[RoadVec3; 3]> {
        let road_triangle = [
            Self::arrangement_vertex_canonical_world(arrangement, vertices[0])?,
            Self::arrangement_vertex_canonical_world(arrangement, vertices[1])?,
            Self::arrangement_vertex_canonical_world(arrangement, vertices[2])?,
        ];
        let area_3d_m2 = (road_triangle[1] - road_triangle[0])
            .cross(road_triangle[2] - road_triangle[0])
            .length()
            * 0.5;
        if area_3d_m2 < f64::from(NODE_OVERLAY_MIN_AREA_M2) {
            return None;
        }
        Some(road_triangle)
    }

    fn top_surface_triangle_is_numeric_dust(triangle: &[RoadVec3; 3]) -> bool {
        let area_m2 = f64::from(Self::signed_polygon_area_xz(triangle).abs());
        if area_m2 <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
            return true;
        }

        let edge_lengths = [
            xz_distance(triangle[0], triangle[1]),
            xz_distance(triangle[1], triangle[2]),
            xz_distance(triangle[2], triangle[0]),
        ];
        let max_edge_m = edge_lengths
            .iter()
            .copied()
            .fold(0.0_f64, |max_edge, edge| max_edge.max(edge));
        if max_edge_m <= f64::EPSILON {
            return true;
        }

        let min_altitude_m = area_m2 * 2.0 / max_edge_m;
        let perimeter_m = edge_lengths.iter().copied().sum::<f64>() as f32;
        area_m2
            <= f64::from(Self::overlay_numeric_area_budget_m2(
                perimeter_m,
                triangle.len(),
            ))
            && min_altitude_m <= f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
    }

    fn arrangement_vertex_canonical_world(
        arrangement: &NodeArrangement,
        vertex_id: arrangement::NodeArrangementVertexId,
    ) -> Option<backend::RoadVec3> {
        let vertex = arrangement.vertices().get(vertex_id.index())?;
        let point_xz = arrangement_vertex_canonical_xz(vertex);
        Some(backend::RoadVec3::new(
            point_xz.x,
            vertex.height_m(),
            point_xz.y,
        ))
    }
}

fn xz_distance(start: RoadVec3, end: RoadVec3) -> f64 {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_surface_export_rejects_logged_long_sidewalk_dust_sliver() {
        let logged_long_edge_m = 16.376;
        let logged_area_m2 = 0.00003701;
        let logged_dust_width_m = logged_area_m2 * 2.0 / logged_long_edge_m;
        let triangle = [
            RoadVec3::new(0.0, 128.392, 0.0),
            RoadVec3::new(logged_long_edge_m, 128.485, 0.0),
            RoadVec3::new(0.0, 128.388, logged_dust_width_m),
        ];

        assert!(
            RoadSurfaceSystem::top_surface_triangle_is_numeric_dust(&triangle),
            "junction sidewalk bridge with logged area/length ratio must not become a visual spike"
        );
    }

    #[test]
    fn top_surface_export_keeps_small_real_sidewalk_triangle() {
        let triangle = [
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(0.005, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 0.005),
        ];

        assert!(
            !RoadSurfaceSystem::top_surface_triangle_is_numeric_dust(&triangle),
            "millimeter-scale triangles are small, but not collapsed overlay dust"
        );
    }
}

fn arrangement_vertex_canonical_xz(
    vertex: &arrangement::NodeArrangementVertex,
) -> backend::RoadVec2 {
    let key = vertex.key();
    keys::SurfaceXzKey::from_raw_keys(key.x_key(), key.z_key()).to_road_xz()
}

fn arrangement_face_signed_area_xz_m2(
    arrangement: &NodeArrangement,
    vertices: [arrangement::NodeArrangementVertexId; 3],
) -> Option<f64> {
    let points = [
        arrangement_vertex_canonical_xz(arrangement.vertices().get(vertices[0].index())?),
        arrangement_vertex_canonical_xz(arrangement.vertices().get(vertices[1].index())?),
        arrangement_vertex_canonical_xz(arrangement.vertices().get(vertices[2].index())?),
    ];
    Some(
        ((points[0].x * points[1].y - points[1].x * points[0].y)
            + (points[1].x * points[2].y - points[2].x * points[1].y)
            + (points[2].x * points[0].y - points[0].x * points[2].y))
            * 0.5,
    )
}
