//! Same-XZ vertical-step arrangement tests.

use super::*;
use crate::simulation::network::surface::node::height::{
    NodeHeightAuthoritySource, NodeHeightCarrierProvenanceKey,
};
use crate::simulation::network::surface::node::ownership::{
    NodeCarrierProvenanceOrigin, NodeOwnedRegionArrangementKey,
};
use crate::simulation::network::surface::node::rails::{
    NodeGeneratedContourClaimPriority, NodeGeneratedContourPurpose,
};

#[test]
fn arrangement_rejects_same_material_same_xz_height_conflict_without_explicit_step() {
    let first = owner(RoadSurfaceBandKind::Sidewalk, 0);
    let second = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                first,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(1.0, 1.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                Vec::new(),
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                second,
                vec![
                    height_vertex(1.0, 0.0, 0.5),
                    height_vertex(2.0, 0.0, 0.5),
                    height_vertex(2.0, 1.0, 0.5),
                    height_vertex(1.0, 1.0, 0.5),
                ],
                Vec::new(),
            ),
        ],
    };

    assert!(matches!(
        NodeArrangement::from_height_solution(&heights),
        Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
    ));
}

#[test]
fn arrangement_accepts_same_material_same_xz_split_with_endpoint_material_path() {
    let first_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let second_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 2);
    let first_height = height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let second_height = height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let key = RoadVec2::new(0.0, 0.0);
    let first_step = NodeRegionSeamConstraint {
        constraint_index: 31,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 0 },
        owner: Some(first_curb),
        opposite_owner: Some(sidewalk),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: key,
        end_xz: key,
    };
    let second_step = NodeRegionSeamConstraint {
        constraint_index: 32,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
        owner: Some(second_curb),
        opposite_owner: Some(sidewalk),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: key,
        end_xz: key,
    };
    let mut arrangement = NodeArrangement::new(12, RoadSurfaceVisualNodePieceKind::JunctionN);
    let first_vertex = arrangement
        .insert_vertex(
            key,
            1.0,
            [first_curb],
            first_height,
            [first_step.seam_source],
        )
        .expect("first same-material endpoint should insert");
    let second_vertex = arrangement
        .insert_vertex(
            key,
            1.25,
            [second_curb],
            second_height,
            [second_step.seam_source],
        )
        .expect("second same-material endpoint should insert");
    arrangement.push_region(
        first_curb,
        first_height,
        vec![first_vertex],
        Vec::new(),
        Vec::new(),
        1.0,
        vec![first_step],
    );
    arrangement.push_region(
        second_curb,
        second_height,
        vec![second_vertex],
        Vec::new(),
        Vec::new(),
        1.0,
        vec![second_step],
    );

    arrangement
        .reject_implicit_material_height_conflicts()
        .expect("source-authored material endpoint path should authorize the same-material split");
}

#[test]
fn arrangement_accepts_same_material_same_xz_height_split_with_explicit_vertical_step() {
    let lower = owner(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let raised = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(1.0, 1.0);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 54,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
        owner: Some(lower),
        opposite_owner: Some(raised),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                lower,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(1.0, 1.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                vec![seam.clone()],
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                raised,
                vec![
                    height_vertex(1.0, 0.0, 0.12),
                    height_vertex(2.0, 0.0, 0.12),
                    height_vertex(2.0, 1.0, 0.12),
                    height_vertex(1.0, 1.0, 0.12),
                ],
                vec![seam],
            ),
        ],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("explicit same-material raised step should authorize split heights");
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        lower,
        raised,
    )
    .expect("test segment is non-degenerate");

    assert!(
        arrangement
            .explicit_vertical_step_segments()
            .contains(&expected)
    );
}

#[test]
fn arrangement_accepts_source_authorized_side_join_asphalt_sidewalk_height_split() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 33);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 35);
    let carriageway_field = NodeBandHeightFieldId::new(5, 3, RoadSurfaceBandKind::Carriageway);
    let sidewalk_field = NodeBandHeightFieldId::new(5, 5, RoadSurfaceBandKind::Sidewalk);
    let shared_start = RoadVec2::new(1.0, 0.0);
    let shared_end = RoadVec2::new(1.0, 1.0);
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            NodeHeightedRegion {
                kind: RoadSurfaceBandKind::Carriageway,
                owner: carriageway,
                height_field_id: carriageway_field,
                shape: vec![vec![
                    side_join_vertex(RoadVec2::new(0.0, 0.0), 0.0, carriageway, carriageway_field),
                    side_join_vertex(shared_start, 0.0, carriageway, carriageway_field),
                    side_join_intersection_vertex(shared_end, 0.0, carriageway, carriageway_field),
                    side_join_vertex(RoadVec2::new(0.0, 1.0), 0.0, carriageway, carriageway_field),
                ]],
                area_m2: 1.0,
                seam_constraints: Vec::new(),
            },
            NodeHeightedRegion {
                kind: RoadSurfaceBandKind::Sidewalk,
                owner: sidewalk,
                height_field_id: sidewalk_field,
                shape: vec![vec![
                    sidewalk_source_vertex(shared_start, 0.12, sidewalk, sidewalk_field),
                    sidewalk_source_vertex(RoadVec2::new(2.0, 0.0), 0.12, sidewalk, sidewalk_field),
                    sidewalk_source_vertex(RoadVec2::new(2.0, 1.0), 0.12, sidewalk, sidewalk_field),
                    sidewalk_source_vertex(shared_end, 0.12, sidewalk, sidewalk_field),
                ]],
                area_m2: 1.0,
                seam_constraints: Vec::new(),
            },
        ],
    };

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("side-join asphalt/sidewalk source carriers should authorize split heights");

    assert!(
        arrangement.diagnostics().is_empty(),
        "source-authorized side-join boundary should not emit seam diagnostics: {:?}",
        arrangement.diagnostics()
    );
}

#[test]
fn arrangement_accepts_same_material_owned_boundary_as_source_authorized_height_split() {
    let first = owner(RoadSurfaceBandKind::Sidewalk, 0);
    let second = owner(RoadSurfaceBandKind::Sidewalk, 5);
    let start = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(1.0, 1.0);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 77,
        seam_source: NodeSeamSource::SidewalkOuter { owner_index: 0 },
        owner: Some(first),
        opposite_owner: Some(second),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                first,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(1.0, 1.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                vec![seam.clone()],
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                second,
                vec![
                    height_vertex(1.0, 0.0, 0.5),
                    height_vertex(2.0, 0.0, 0.5),
                    height_vertex(2.0, 1.0, 0.5),
                    height_vertex(1.0, 1.0, 0.5),
                ],
                vec![seam],
            ),
        ],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("post-boolean same-material seam should authorize split sidewalk heights");
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        first,
        second,
    )
    .expect("test segment is non-degenerate");

    assert!(
        arrangement
            .explicit_vertical_step_segments()
            .contains(&expected)
    );
}

#[test]
fn arrangement_accepts_height_ranked_step_endpoint_grouping() {
    let lower_left = owner(RoadSurfaceBandKind::Carriageway, 2);
    let raised_left = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_right = owner(RoadSurfaceBandKind::Carriageway, 15);
    let raised_right = owner(RoadSurfaceBandKind::CurbOrShoulder, 16);
    let lower_left_field = height_field_id(RoadSurfaceBandKind::Carriageway, 2);
    let raised_right_field = height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 16);
    let key = RoadVec2::new(0.0, 0.0);
    let left_end = RoadVec2::new(1.0, 0.0);
    let right_end = RoadVec2::new(0.0, 1.0);
    let left_step = NodeRegionSeamConstraint {
        constraint_index: 19,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 2 },
        owner: Some(lower_left),
        opposite_owner: Some(raised_left),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: key,
        end_xz: left_end,
    };
    let right_step = NodeRegionSeamConstraint {
        constraint_index: 72,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 16 },
        owner: Some(lower_right),
        opposite_owner: Some(raised_right),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: key,
        end_xz: right_end,
    };
    let mut arrangement = NodeArrangement::new(12, RoadSurfaceVisualNodePieceKind::JunctionN);

    let lower_start = arrangement
        .insert_vertex(
            key,
            0.0,
            [lower_left],
            lower_left_field,
            [left_step.seam_source],
        )
        .expect("lower step endpoint should insert");
    let lower_end = arrangement
        .insert_vertex(
            left_end,
            0.0,
            [lower_left],
            lower_left_field,
            [left_step.seam_source],
        )
        .expect("lower step edge endpoint should insert");
    let lower_edge = arrangement.push_edge(
        lower_start,
        lower_end,
        lower_left,
        lower_left_field,
        Some(raised_left),
        Some(height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1)),
        false,
        false,
        true,
        left_step.seam_source,
        vec![left_step.constraint_index],
    );
    arrangement.push_region(
        lower_left,
        lower_left_field,
        vec![lower_start, lower_end],
        Vec::new(),
        vec![lower_edge],
        1.0,
        vec![left_step],
    );

    let raised_start = arrangement
        .insert_vertex(
            key,
            0.12,
            [raised_right],
            raised_right_field,
            [right_step.seam_source],
        )
        .expect("raised step endpoint should insert");
    let raised_end = arrangement
        .insert_vertex(
            right_end,
            0.12,
            [raised_right],
            raised_right_field,
            [right_step.seam_source],
        )
        .expect("raised step edge endpoint should insert");
    let raised_edge = arrangement.push_edge(
        raised_start,
        raised_end,
        raised_right,
        raised_right_field,
        Some(lower_right),
        Some(height_field_id(RoadSurfaceBandKind::Carriageway, 15)),
        false,
        false,
        true,
        right_step.seam_source,
        vec![right_step.constraint_index],
    );
    arrangement.push_region(
        raised_right,
        raised_right_field,
        vec![raised_start, raised_end],
        Vec::new(),
        vec![raised_edge],
        1.0,
        vec![right_step],
    );

    arrangement
        .reject_implicit_material_height_conflicts()
        .expect("separate canonical step endpoints should authorize the ranked height split");
}

#[test]
fn arrangement_ignores_same_xz_height_split_without_final_boundary_contact() {
    let lower_left = owner(RoadSurfaceBandKind::Carriageway, 0);
    let raised = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_right = owner(RoadSurfaceBandKind::Carriageway, 2);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(1.0, 0.0);
    let step = NodeRegionSeamConstraint {
        constraint_index: 88,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 0 },
        owner: Some(lower_left),
        opposite_owner: Some(raised),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                lower_left,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                vec![step.clone()],
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                lower_right,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(-1.0, 0.0, 0.0),
                    height_vertex(0.0, -1.0, 0.0),
                ],
                Vec::new(),
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                raised,
                vec![
                    height_vertex(0.0, 0.0, 0.12),
                    height_vertex(1.0, 0.0, 0.12),
                    height_vertex(1.0, -1.0, 0.12),
                ],
                vec![step],
            ),
        ],
    };

    NodeArrangement::from_height_solution(&heights)
        .expect("point-only coincidence is not a final owned boundary height conflict");
}

fn side_join_vertex(
    point_xz: RoadVec2,
    height_m: f64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
) -> NodeHeightedVertex {
    sourced_vertex(
        point_xz,
        height_m,
        owner,
        height_field_id,
        RoadSurfaceBandKind::Carriageway,
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        },
        NodeCarrierProvenanceOrigin::GeneratedCarrierVertex {
            contour_index: 73,
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        },
    )
}

fn side_join_intersection_vertex(
    point_xz: RoadVec2,
    height_m: f64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
) -> NodeHeightedVertex {
    sourced_vertex(
        point_xz,
        height_m,
        owner,
        height_field_id,
        RoadSurfaceBandKind::Carriageway,
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        },
        NodeCarrierProvenanceOrigin::SourceIntersection { peer_count: 1 },
    )
}

fn sidewalk_source_vertex(
    point_xz: RoadVec2,
    height_m: f64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
) -> NodeHeightedVertex {
    sourced_vertex(
        point_xz,
        height_m,
        owner,
        height_field_id,
        RoadSurfaceBandKind::Sidewalk,
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        },
        NodeCarrierProvenanceOrigin::SourceIntersection { peer_count: 1 },
    )
}

fn sourced_vertex(
    point_xz: RoadVec2,
    height_m: f64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    source_kind: RoadSurfaceBandKind,
    claim_priority: NodeGeneratedContourClaimPriority,
    height_authority: NodeHeightAuthoritySource,
    origin: NodeCarrierProvenanceOrigin,
) -> NodeHeightedVertex {
    let source_provenance = NodeHeightCarrierProvenanceKey {
        owner,
        source_kind,
        source_mouth_order_index: height_field_id.mouth_order_index(),
        source_band_index: height_field_id.band_index(),
        height_field_id,
        claim_priority,
        point: NodeOwnedRegionArrangementKey::from_point(point_xz),
        origin,
    };
    NodeHeightedVertex {
        point_xz,
        height_m,
        height_field_id,
        height_authority: Some(height_authority),
        source_provenance: Some(source_provenance),
        grade_authority: Some(NodeGradeVertexAuthority::new_with_source_provenance(
            point_xz,
            height_m,
            owner,
            height_field_id,
            NodeGradeCarrierDecision::SourceCarrier {
                authority: Some(height_authority),
            },
            Some(source_provenance),
        )),
    }
}
