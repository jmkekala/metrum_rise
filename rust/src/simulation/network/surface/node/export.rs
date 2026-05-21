//! Node surface export from canonical arrangement output.

use super::*;

mod assembly;
mod footprint_loops;
mod outer_boundary;
mod raised_step_support;
mod terrain_clip_loops;
mod top_regions;

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
}
