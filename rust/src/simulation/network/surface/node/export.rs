//! Node surface export from canonical arrangement output.

use super::*;
use std::collections::BTreeMap;

mod assembly;
mod footprint_loops;
mod outer_boundary;
mod raised_step_support;
mod terrain_clip_loops;
mod top_regions;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn node_surface_regions_from_arrangement(
        arrangement: &NodeArrangement,
        footprint_shapes: &super::NodeOverlayShapes,
    ) -> Result<super::NodeSurfaceRegionResult, NodeBoundaryExportError> {
        reject_unauthorized_arrangement_height_splits(arrangement)?;
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
            &node_top_surface_sources,
        );
        let mut raised_step_faces = raised_step_faces
            .into_iter()
            .map(|face| (face.polygon, face.source))
            .collect::<Vec<_>>();

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
            Self::footprint_boundary_point_loops_from_footprint_shapes(
                footprint_shapes,
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
            Self::outer_boundary_polygons_from_footprint_boundary_point_loops(
                &footprint_boundary_point_loops,
            )?;
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

fn reject_unauthorized_arrangement_height_splits(
    arrangement: &NodeArrangement,
) -> Result<(), NodeBoundaryExportError> {
    let explicit_vertical_step_segments = arrangement.explicit_vertical_step_segments();
    let mut vertices_by_key = BTreeMap::<arrangement::NodeArrangementKey, Vec<_>>::new();
    for vertex in arrangement.vertices() {
        vertices_by_key
            .entry(vertex.key())
            .or_default()
            .push(vertex);
    }

    let mut conflicts_by_owner_pair =
        BTreeMap::<(arrangement::NodeBandOwner, arrangement::NodeBandOwner), Vec<_>>::new();
    for (key, vertices) in vertices_by_key {
        for left_index in 0..vertices.len() {
            for right in vertices.iter().copied().skip(left_index + 1) {
                let left = vertices[left_index];
                if left.height_mm() == right.height_mm() {
                    continue;
                }
                for left_owner in left.owners() {
                    for right_owner in right.owners() {
                        if left_owner == right_owner {
                            continue;
                        }
                        if arrangement_height_split_authorized(
                            arrangement,
                            key,
                            left.height_mm(),
                            *left_owner,
                            right.height_mm(),
                            *right_owner,
                            &explicit_vertical_step_segments,
                        ) {
                            continue;
                        }
                        let owner_pair = if left_owner <= right_owner {
                            (*left_owner, *right_owner)
                        } else {
                            (*right_owner, *left_owner)
                        };
                        conflicts_by_owner_pair
                            .entry(owner_pair)
                            .or_default()
                            .push((
                                key,
                                left.height_mm(),
                                *left_owner,
                                right.height_mm(),
                                *right_owner,
                            ));
                    }
                }
            }
        }
    }

    for conflicts in conflicts_by_owner_pair.into_values() {
        let mut keys = conflicts
            .iter()
            .map(|(key, _, _, _, _)| *key)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys.dedup();
        if keys.len() < 2 {
            continue;
        }
        let (key, existing_y_mm, existing_owner, incoming_y_mm, incoming_owner) = conflicts[0];
        return Err(
            NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
                x_key: key.x_key(),
                z_key: key.z_key(),
                existing_y_mm,
                incoming_y_mm,
                existing_owner_kind: existing_owner.kind(),
                existing_owner_index: existing_owner.owner_index(),
                existing_source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: key.x_key(),
                    z_key: key.z_key(),
                    y_mm: existing_y_mm,
                },
                incoming_owner_kind: incoming_owner.kind(),
                incoming_owner_index: incoming_owner.owner_index(),
                incoming_source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: key.x_key(),
                    z_key: key.z_key(),
                    y_mm: incoming_y_mm,
                },
            },
        );
    }
    Ok(())
}

fn arrangement_height_split_authorized(
    arrangement: &NodeArrangement,
    key: arrangement::NodeArrangementKey,
    left_height_mm: i64,
    left_owner: arrangement::NodeBandOwner,
    right_height_mm: i64,
    right_owner: arrangement::NodeBandOwner,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> bool {
    let (lower_owner, raised_owner) = if left_height_mm <= right_height_mm {
        (left_owner, right_owner)
    } else {
        (right_owner, left_owner)
    };
    explicit_vertical_step_segments.iter().any(|segment| {
        arrangement_key_lies_exactly_on_step_segment(key, segment.start(), segment.end())
            && ((segment.owner() == lower_owner && segment.opposite_owner() == raised_owner)
                || (segment.owner() == raised_owner && segment.opposite_owner() == lower_owner))
    }) || exposed_final_boundary_authorizes_raised_step_endpoint(
        arrangement,
        key,
        left_height_mm,
        left_owner,
        right_height_mm,
        right_owner,
    )
}

fn exposed_final_boundary_authorizes_raised_step_endpoint(
    arrangement: &NodeArrangement,
    key: arrangement::NodeArrangementKey,
    left_height_mm: i64,
    left_owner: arrangement::NodeBandOwner,
    right_height_mm: i64,
    right_owner: arrangement::NodeBandOwner,
) -> bool {
    let (lower_owner, raised_owner) = if left_height_mm <= right_height_mm {
        (left_owner, right_owner)
    } else {
        (right_owner, left_owner)
    };
    let Some(lower_rank) = band_semantics::raised_step_band_rank(lower_owner.kind()) else {
        return false;
    };
    let Some(raised_rank) = band_semantics::raised_step_band_rank(raised_owner.kind()) else {
        return false;
    };
    if lower_rank >= raised_rank
        || !band_semantics::raised_step_kinds_can_contact(lower_owner.kind(), raised_owner.kind())
    {
        return false;
    }

    let mut raised_exposed = false;
    let mut lower_exposed = false;
    for edge in arrangement
        .edges()
        .iter()
        .filter(|edge| edge.exposed_boundary())
    {
        let Some(start) = arrangement.vertices().get(edge.start().index()) else {
            continue;
        };
        let Some(end) = arrangement.vertices().get(edge.end().index()) else {
            continue;
        };
        if !arrangement_key_lies_exactly_on_step_segment(key, start.key(), end.key()) {
            continue;
        }
        raised_exposed |= edge.owner() == raised_owner;
        lower_exposed |= edge.owner() == lower_owner;
    }
    raised_exposed && !lower_exposed
}

fn arrangement_key_lies_exactly_on_step_segment(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> bool {
    let point = super::keys::SurfaceXzKey::from_raw_keys(point.x_key(), point.z_key());
    let start = super::keys::SurfaceXzKey::from_raw_keys(start.x_key(), start.z_key());
    let end = super::keys::SurfaceXzKey::from_raw_keys(end.x_key(), end.z_key());
    super::segments::key_lies_exactly_on_segment(point, start, end)
}
