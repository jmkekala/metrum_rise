//! Canonical source merge and owner selection for earthwork boundaries.

use super::*;

pub(super) fn ambiguous_earthwork_boundary_segment_source_error(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    existing_source: RoadSurfaceEarthworkFaceSource,
    incoming_source: RoadSurfaceEarthworkFaceSource,
) -> NodeBoundaryExportError {
    NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource {
        start_x_key: start_point_key.x_key,
        start_z_key: start_point_key.z_key,
        end_x_key: end_point_key.x_key,
        end_z_key: end_point_key.z_key,
        existing_source,
        incoming_source,
    }
}

pub(super) fn merged_node_earthwork_source_candidate(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    a: NodeEarthworkBoundarySourceCandidate,
    b: NodeEarthworkBoundarySourceCandidate,
) -> Option<NodeEarthworkBoundarySourceCandidate> {
    match (a.face_source, b.face_source) {
        (
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: a_node_id,
                kind: a_kind,
                owner_kind: a_owner_kind,
                owner_index: a_owner_index,
                boundary_source: a_boundary_source,
            },
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: b_node_id,
                kind: b_kind,
                owner_kind: b_owner_kind,
                owner_index: b_owner_index,
                boundary_source: b_boundary_source,
            },
        ) => {
            if a_node_id != b_node_id || a_kind != b_kind {
                return None;
            }
            let (owner_kind, owner_index, boundary_source, height_field_id) =
                merged_node_earthwork_boundary_source(
                    start_point_key,
                    end_point_key,
                    a_owner_kind,
                    a_owner_index,
                    a_boundary_source,
                    a.height_field_id,
                    b_owner_kind,
                    b_owner_index,
                    b_boundary_source,
                    b.height_field_id,
                )?;
            Some(NodeEarthworkBoundarySourceCandidate {
                face_source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    node_id: a_node_id,
                    kind: a_kind,
                    owner_kind,
                    owner_index,
                    boundary_source,
                },
                height_field_id,
            })
        }
        _ => {
            (a.face_source == b.face_source && a.height_field_id == b.height_field_id).then_some(a)
        }
    }
}

fn merged_node_earthwork_boundary_source(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    a_owner_kind: RoadSurfaceBandKind,
    a_owner_index: usize,
    a: Option<NodeFootprintBoundarySegmentSource>,
    a_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
    b_owner_kind: RoadSurfaceBandKind,
    b_owner_index: usize,
    b: Option<NodeFootprintBoundarySegmentSource>,
    b_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
) -> Option<(
    RoadSurfaceBandKind,
    usize,
    Option<NodeFootprintBoundarySegmentSource>,
    Option<arrangement::NodeBandHeightFieldId>,
)> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let start_matches = node_earthwork_boundary_vertex_sources_share_identity_at_point(
                start_point_key,
                a.start,
                b.start,
            );
            let end_matches = node_earthwork_boundary_vertex_sources_share_identity_at_point(
                end_point_key,
                a.end,
                b.end,
            );
            if start_matches && end_matches {
                let (owner_kind, owner_index, height_field_id) =
                    canonical_earthwork_boundary_owner(
                        a_owner_kind,
                        a_owner_index,
                        a_height_field_id,
                        b_owner_kind,
                        b_owner_index,
                        b_height_field_id,
                    )?;
                return Some((owner_kind, owner_index, Some(a), height_field_id));
            }
            if a_owner_kind == b_owner_kind && a_owner_index == b_owner_index {
                return None;
            }
            if a_owner_kind == b_owner_kind {
                let (owner_kind, owner_index, height_field_id) =
                    canonical_earthwork_boundary_owner(
                        a_owner_kind,
                        a_owner_index,
                        a_height_field_id,
                        b_owner_kind,
                        b_owner_index,
                        b_height_field_id,
                    )?;
                return Some((
                    owner_kind,
                    owner_index,
                    Some(NodeFootprintBoundarySegmentSource {
                        start: canonical_boundary_point_source(start_point_key),
                        end: canonical_boundary_point_source(end_point_key),
                    }),
                    height_field_id,
                ));
            }
            if !raised_step_kinds_can_contact(a_owner_kind, b_owner_kind) {
                return None;
            }
            let (owner_kind, owner_index, height_field_id) =
                canonical_adjacent_material_earthwork_boundary_owner(
                    a_owner_kind,
                    a_owner_index,
                    a_height_field_id,
                    b_owner_kind,
                    b_owner_index,
                    b_height_field_id,
                )?;
            Some((
                owner_kind,
                owner_index,
                Some(NodeFootprintBoundarySegmentSource {
                    start: if start_matches {
                        a.start
                    } else {
                        canonical_boundary_point_source(start_point_key)
                    },
                    end: if end_matches {
                        a.end
                    } else {
                        canonical_boundary_point_source(end_point_key)
                    },
                }),
                height_field_id,
            ))
        }
        (None, None) => {
            let (owner_kind, owner_index, height_field_id) = canonical_earthwork_boundary_owner(
                a_owner_kind,
                a_owner_index,
                a_height_field_id,
                b_owner_kind,
                b_owner_index,
                b_height_field_id,
            )?;
            Some((owner_kind, owner_index, None, height_field_id))
        }
        _ => None,
    }
}

fn canonical_earthwork_boundary_owner(
    a_owner_kind: RoadSurfaceBandKind,
    a_owner_index: usize,
    a_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
    b_owner_kind: RoadSurfaceBandKind,
    b_owner_index: usize,
    b_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
) -> Option<(
    RoadSurfaceBandKind,
    usize,
    Option<arrangement::NodeBandHeightFieldId>,
)> {
    if a_owner_kind == b_owner_kind && a_owner_index == b_owner_index {
        if a_height_field_id != b_height_field_id {
            return None;
        }
        return Some((a_owner_kind, a_owner_index, a_height_field_id));
    }
    if a_owner_kind == b_owner_kind {
        return Some(if a_owner_index <= b_owner_index {
            (a_owner_kind, a_owner_index, a_height_field_id)
        } else {
            (b_owner_kind, b_owner_index, b_height_field_id)
        });
    }
    canonical_adjacent_material_earthwork_boundary_owner(
        a_owner_kind,
        a_owner_index,
        a_height_field_id,
        b_owner_kind,
        b_owner_index,
        b_height_field_id,
    )
}

fn canonical_adjacent_material_earthwork_boundary_owner(
    a_owner_kind: RoadSurfaceBandKind,
    a_owner_index: usize,
    a_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
    b_owner_kind: RoadSurfaceBandKind,
    b_owner_index: usize,
    b_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
) -> Option<(
    RoadSurfaceBandKind,
    usize,
    Option<arrangement::NodeBandHeightFieldId>,
)> {
    let a_rank = raised_step_band_rank(a_owner_kind)?;
    let b_rank = raised_step_band_rank(b_owner_kind)?;
    if a_rank == b_rank || a_rank.abs_diff(b_rank) != 1 {
        return None;
    }
    Some(if a_rank > b_rank {
        (a_owner_kind, a_owner_index, a_height_field_id)
    } else {
        (b_owner_kind, b_owner_index, b_height_field_id)
    })
}

fn canonical_boundary_point_source(
    point_key: ArrangementBoundaryPointKey,
) -> NodeFootprintBoundaryVertexSource {
    NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
        x_key: point_key.x_key,
        z_key: point_key.z_key,
        y_mm: point_key.y_mm,
    }
}

pub(super) fn node_footprint_boundary_direct_vertex_is_canonical_point(
    vertex: NodeFootprintBoundaryDirectVertex,
) -> bool {
    matches!(
        vertex.source,
        NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. }
    )
}

fn node_earthwork_boundary_vertex_sources_share_identity_at_point(
    point_key: ArrangementBoundaryPointKey,
    a: NodeFootprintBoundaryVertexSource,
    b: NodeFootprintBoundaryVertexSource,
) -> bool {
    if node_footprint_boundary_vertex_sources_share_identity(a, b) {
        return true;
    }
    match (a, b) {
        (NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm }, _)
        | (_, NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm }) => {
            x_key == point_key.x_key && z_key == point_key.z_key && y_mm == point_key.y_mm
        }
        _ => false,
    }
}
