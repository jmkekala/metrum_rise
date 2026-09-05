// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step endpoint authority tests.

use super::*;

#[test]
fn raised_step_footprint_height_requires_explicit_step_authority() {
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let step_start = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let step_end = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::Carriageway,
        0,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut missing_authority = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = missing_authority
        .height_mm_at_key(step_start)
        .expect_err("material rank alone must not resolve same-XZ footprint height conflict");
    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));

    let mut authorized = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                step_start,
                step_end,
                lower_owner,
                raised_owner,
            )
            .expect("test step should be non-degenerate"),
        ],
    };

    let height_mm = authorized
        .height_mm_at_key(step_start)
        .expect("explicit owner-pair step should authorize raised footprint corner height");
    assert_eq!(height_mm, Some(120));
}

#[test]
fn explicit_step_segment_authorizes_raised_boundary_height_at_endpoint() {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let step_end = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 120,
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(
        lower_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1,
                grade_authority_index: 91,
            }),
            owner_kind: lower_owner.kind(),
            owner_index: lower_owner.owner_index(),
        }],
    );
    direct_vertex_source_candidates.insert(
        raised_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 23,
                grade_authority_index: 92,
            }),
            owner_kind: raised_owner.kind(),
            owner_index: raised_owner.owner_index(),
        }],
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                step_end,
                lower_owner,
                raised_owner,
            )
            .expect("test step should be non-degenerate"),
        ],
    };

    let height_mm = sources
        .boundary_height_mm_at_key(key)
        .expect("explicit endpoint step should authorize the raised footprint height");

    assert_eq!(height_mm, Some(120));
}

#[test]
fn explicit_step_segment_uses_semantic_raised_side_when_endpoint_rounding_inverts_height_order() {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let step_end = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 19);
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 150_305,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 150_304,
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(
        lower_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 112,
                grade_authority_index: 123,
            }),
            owner_kind: lower_owner.kind(),
            owner_index: lower_owner.owner_index(),
        }],
    );
    direct_vertex_source_candidates.insert(
        raised_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 115,
                grade_authority_index: 124,
            }),
            owner_kind: raised_owner.kind(),
            owner_index: raised_owner.owner_index(),
        }],
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                step_end,
                lower_owner,
                raised_owner,
            )
            .expect("test step should be non-degenerate"),
        ],
    };

    let height_mm = sources
        .boundary_height_mm_at_key(key)
        .expect("explicit step identity should choose the semantic raised-side owner height");

    assert_eq!(height_mm, Some(150_304));
}

#[test]
fn raised_step_height_accepts_equivalent_same_height_plateau_candidates() {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let step_end = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let raised_curb_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let raised_sidewalk_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 7);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 120,
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(
        lower_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1,
                grade_authority_index: 91,
            }),
            owner_kind: lower_owner.kind(),
            owner_index: lower_owner.owner_index(),
        }],
    );
    direct_vertex_source_candidates.insert(
        raised_point,
        vec![
            NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 23,
                        grade_authority_index: 92,
                    },
                ),
                owner_kind: raised_curb_owner.kind(),
                owner_index: raised_curb_owner.owner_index(),
            },
            NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 24,
                        grade_authority_index: 93,
                    },
                ),
                owner_kind: raised_sidewalk_owner.kind(),
                owner_index: raised_sidewalk_owner.owner_index(),
            },
        ],
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                step_end,
                lower_owner,
                raised_curb_owner,
            )
            .expect("test step should be non-degenerate"),
        ],
    };

    let height_mm = sources
        .boundary_height_mm_at_key(key)
        .expect("one canonical owner-pair step should authorize the raised plateau height");

    assert_eq!(height_mm, Some(120));
}

#[test]
fn raised_step_height_uses_explicit_pair_with_unrelated_lower_plateau_candidate() {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let step_end = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let curb_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let raised_sidewalk_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 23);
    let peer_sidewalk_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 150_303,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 150_304,
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(
        lower_point,
        vec![
            NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 63,
                        grade_authority_index: 161,
                    },
                ),
                owner_kind: curb_owner.kind(),
                owner_index: curb_owner.owner_index(),
            },
            NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 64,
                        grade_authority_index: 162,
                    },
                ),
                owner_kind: peer_sidewalk_owner.kind(),
                owner_index: peer_sidewalk_owner.owner_index(),
            },
        ],
    );
    direct_vertex_source_candidates.insert(
        raised_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 145,
                grade_authority_index: 163,
            }),
            owner_kind: raised_sidewalk_owner.kind(),
            owner_index: raised_sidewalk_owner.owner_index(),
        }],
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                step_end,
                curb_owner,
                raised_sidewalk_owner,
            )
            .expect("test step should be non-degenerate"),
        ],
    };

    let height_mm = sources
        .boundary_height_mm_at_key(key)
        .expect("explicit owner-pair step should not be vetoed by unrelated same-XZ support");

    assert_eq!(height_mm, Some(150_304));
}

#[test]
fn raised_step_height_accepts_separate_explicit_endpoint_step_groups() {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let lower_step_end =
        ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let raised_step_end =
        ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 1.0)).xz_key();
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let lower_step_raised_owner =
        arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let raised_step_lower_owner =
        arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 15);
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 16);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 120,
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(
        lower_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 0,
                grade_authority_index: 77,
            }),
            owner_kind: lower_owner.kind(),
            owner_index: lower_owner.owner_index(),
        }],
    );
    direct_vertex_source_candidates.insert(
        raised_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 56,
                grade_authority_index: 78,
            }),
            owner_kind: raised_owner.kind(),
            owner_index: raised_owner.owner_index(),
        }],
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                lower_step_end,
                lower_owner,
                lower_step_raised_owner,
            )
            .expect("lower-side test step should be non-degenerate"),
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                raised_step_end,
                raised_step_lower_owner,
                raised_owner,
            )
            .expect("raised-side test step should be non-degenerate"),
        ],
    };

    let height_mm = sources
        .boundary_height_mm_at_key(key)
        .expect("separate explicit endpoint step groups should authorize the raised height");

    assert_eq!(height_mm, Some(120));
}
