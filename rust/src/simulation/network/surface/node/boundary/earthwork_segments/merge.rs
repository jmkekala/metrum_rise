// SPDX-License-Identifier: GPL-2.0-only

//! Canonical source merge and owner selection for earthwork boundaries.

use super::*;

pub(super) fn ambiguous_earthwork_boundary_segment_source_error(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    existing: NodeEarthworkBoundarySourceCandidate,
    incoming: NodeEarthworkBoundarySourceCandidate,
) -> NodeBoundaryExportError {
    NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource {
        start_x_key: start_point_key.x_key,
        start_z_key: start_point_key.z_key,
        start_y_mm: start_point_key.y_mm,
        end_x_key: end_point_key.x_key,
        end_z_key: end_point_key.z_key,
        end_y_mm: end_point_key.y_mm,
        existing_height_field_id: existing.height_field_id,
        incoming_height_field_id: incoming.height_field_id,
        existing_source: existing.face_source,
        incoming_source: incoming.face_source,
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
            let matching_owner = a_owner_kind == b_owner_kind && a_owner_index == b_owner_index;
            let (owner_kind, owner_index, boundary_source, height_field_id) =
                merged_node_earthwork_boundary_source(
                    a_kind,
                    start_point_key,
                    end_point_key,
                    matching_owner,
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
    piece_kind: RoadSurfaceVisualNodePieceKind,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    matching_owner: bool,
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
            let start_identity_matches =
                node_earthwork_boundary_vertex_sources_share_identity_at_point(
                    start_point_key,
                    a.start,
                    b.start,
                );
            let end_identity_matches =
                node_earthwork_boundary_vertex_sources_share_identity_at_point(
                    end_point_key,
                    a.end,
                    b.end,
                );
            let start_matches = start_identity_matches
                || (matching_owner
                    && node_footprint_boundary_vertex_sources_share_boundary_point_authority(
                        start_point_key,
                        a.start,
                        b.start,
                    ));
            let end_matches = end_identity_matches
                || (matching_owner
                    && node_footprint_boundary_vertex_sources_share_boundary_point_authority(
                        end_point_key,
                        a.end,
                        b.end,
                    ));
            if start_matches && end_matches {
                let (owner_kind, owner_index, height_field_id) =
                    canonical_earthwork_boundary_owner(
                        a_owner_kind,
                        a_owner_index,
                        a_height_field_id,
                        b_owner_kind,
                        b_owner_index,
                        b_height_field_id,
                    )
                    .or_else(|| {
                        (start_identity_matches && end_identity_matches).then(|| {
                            canonical_matching_provenance_earthwork_boundary_owner(
                                a_owner_kind,
                                a_owner_index,
                                a_height_field_id,
                                b_owner_kind,
                                b_owner_index,
                                b_height_field_id,
                            )
                        })?
                    })?;
                return Some((
                    owner_kind,
                    owner_index,
                    Some(NodeFootprintBoundarySegmentSource {
                        start: if start_identity_matches {
                            a.start
                        } else {
                            canonical_boundary_point_source(start_point_key)
                        },
                        end: if end_identity_matches {
                            a.end
                        } else {
                            canonical_boundary_point_source(end_point_key)
                        },
                    }),
                    height_field_id,
                ));
            }
            let start_has_canonical_point =
                node_earthwork_boundary_vertex_sources_include_canonical_point_at(
                    start_point_key,
                    a.start,
                    b.start,
                );
            let end_has_canonical_point =
                node_earthwork_boundary_vertex_sources_include_canonical_point_at(
                    end_point_key,
                    a.end,
                    b.end,
                );
            if matching_owner
                && ((start_has_canonical_point && !end_matches)
                    || (end_has_canonical_point && !start_matches))
            {
                return Some((
                    a_owner_kind,
                    a_owner_index,
                    Some(NodeFootprintBoundarySegmentSource {
                        start: if start_identity_matches {
                            a.start
                        } else {
                            canonical_boundary_point_source(start_point_key)
                        },
                        end: if end_identity_matches {
                            a.end
                        } else {
                            canonical_boundary_point_source(end_point_key)
                        },
                    }),
                    if a_height_field_id == b_height_field_id {
                        a_height_field_id
                    } else {
                        None
                    },
                ));
            }
            if matching_owner
                && piece_kind == RoadSurfaceVisualNodePieceKind::Bend
                && a_height_field_id.is_none()
                && b_height_field_id.is_none()
            {
                return Some((
                    a_owner_kind,
                    a_owner_index,
                    Some(NodeFootprintBoundarySegmentSource {
                        start: if start_identity_matches {
                            a.start
                        } else {
                            canonical_boundary_point_source(start_point_key)
                        },
                        end: if end_identity_matches {
                            a.end
                        } else {
                            canonical_boundary_point_source(end_point_key)
                        },
                    }),
                    None,
                ));
            }
            if matching_owner
                && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
                && a_height_field_id.is_none()
                && b_height_field_id.is_none()
                && (start_identity_matches || end_identity_matches)
            {
                return Some((
                    a_owner_kind,
                    a_owner_index,
                    Some(NodeFootprintBoundarySegmentSource {
                        start: if start_identity_matches {
                            a.start
                        } else {
                            canonical_boundary_point_source(start_point_key)
                        },
                        end: if end_identity_matches {
                            a.end
                        } else {
                            canonical_boundary_point_source(end_point_key)
                        },
                    }),
                    None,
                ));
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
                        start: if start_identity_matches {
                            a.start
                        } else {
                            canonical_boundary_point_source(start_point_key)
                        },
                        end: if end_identity_matches {
                            a.end
                        } else {
                            canonical_boundary_point_source(end_point_key)
                        },
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

fn canonical_matching_provenance_earthwork_boundary_owner(
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
    if a_rank == b_rank {
        return None;
    }
    Some(if a_rank > b_rank {
        (a_owner_kind, a_owner_index, a_height_field_id)
    } else {
        (b_owner_kind, b_owner_index, b_height_field_id)
    })
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

fn node_earthwork_boundary_vertex_sources_include_canonical_point_at(
    point_key: ArrangementBoundaryPointKey,
    a: NodeFootprintBoundaryVertexSource,
    b: NodeFootprintBoundaryVertexSource,
) -> bool {
    matches!(
        (a, b),
        (
            NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm },
            _
        ) | (
            _,
            NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm }
        ) if x_key == point_key.x_key && z_key == point_key.z_key && y_mm == point_key.y_mm
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::RoadVec3;

    fn direct_source(
        top_surface_source_index: usize,
        grade_authority_index: usize,
    ) -> NodeFootprintBoundaryVertexSource {
        NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index,
            grade_authority_index,
        })
    }

    fn sidewalk_candidate(
        start: NodeFootprintBoundaryVertexSource,
        end: NodeFootprintBoundaryVertexSource,
    ) -> NodeEarthworkBoundarySourceCandidate {
        NodeEarthworkBoundarySourceCandidate {
            face_source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: 3,
                kind: RoadSurfaceVisualNodePieceKind::JunctionN,
                owner_kind: RoadSurfaceBandKind::Sidewalk,
                owner_index: 5,
                boundary_source: Some(NodeFootprintBoundarySegmentSource { start, end }),
            },
            height_field_id: None,
        }
    }

    #[test]
    fn junctionn_same_owner_one_matching_endpoint_uses_canonical_segment_source() {
        let start_key =
            ArrangementBoundaryPointKey::from_world(RoadVec3::new(-10.586514, 0.12, 17.657581));
        let end_key =
            ArrangementBoundaryPointKey::from_world(RoadVec3::new(-11.647174, 0.0, 16.596921));
        let merged = merged_node_earthwork_source_candidate(
            start_key,
            end_key,
            sidewalk_candidate(direct_source(116, 65), direct_source(0, 61)),
            sidewalk_candidate(direct_source(116, 65), direct_source(69, 62)),
        )
        .expect("same-owner JunctionN boundary with one matching endpoint should canonicalize");

        assert!(matches!(
            merged.face_source,
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                owner_kind: RoadSurfaceBandKind::Sidewalk,
                owner_index: 5,
                boundary_source: Some(NodeFootprintBoundarySegmentSource {
                    start: NodeFootprintBoundaryVertexSource::Direct(
                        NodeFootprintBoundaryDirectSource {
                            top_surface_source_index: 116,
                            grade_authority_index: 65,
                        },
                    ),
                    end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                        x_key,
                        z_key,
                        y_mm,
                    },
                }),
                ..
            } if x_key == end_key.x_key && z_key == end_key.z_key && y_mm == end_key.y_mm
        ));
        assert_eq!(merged.height_field_id, None);
    }

    #[test]
    fn junctionn_same_owner_fully_distinct_endpoints_stay_ambiguous() {
        let start_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0));
        let end_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(2.0, 0.0, 0.0));

        assert!(
            merged_node_earthwork_source_candidate(
                start_key,
                end_key,
                sidewalk_candidate(direct_source(3, 30), direct_source(3, 31)),
                sidewalk_candidate(direct_source(4, 40), direct_source(4, 41)),
            )
            .is_none()
        );
    }
}
