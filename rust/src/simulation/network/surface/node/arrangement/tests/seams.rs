//! Arrangement seam source and height-context tests.

use super::*;

#[test]
fn arrangement_exports_explicit_material_seam_grade_decision() {
    let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
    let field = NodeBandHeightFieldId::new(0, 6, RoadSurfaceBandKind::Carriageway);
    let mut heights = NodeHeightSolution {
        node_id: 82,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner,
            height_field_id: field,
            shape: vec![vec![
                height_vertex(0.0, 0.0, 2.0),
                height_vertex(1.0, 0.0, 2.0),
                height_vertex(0.0, 1.0, 2.0),
            ]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };
    for vertex in heights.regions[0].shape[0].iter_mut() {
        vertex.height_field_id = field;
        vertex.grade_authority = Some(NodeGradeVertexAuthority::new(
            vertex.point_xz,
            vertex.height_m,
            owner,
            field,
            NodeGradeCarrierDecision::ExplicitMaterialSeam,
        ));
    }

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("grade-authorized explicit seam should arrange");
    assert!(arrangement.vertices().iter().all(|vertex| {
        vertex.grade_authority().decision == NodeGradeCarrierDecision::ExplicitMaterialSeam
    }));
}

#[test]
fn arrangement_keeps_height_distinct_explicit_seam_contexts() {
    let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::Bend);
    let point = RoadVec2::new(0.0, 0.0);

    let low = arrangement
        .insert_vertex(
            point,
            1.0,
            [owner(RoadSurfaceBandKind::Carriageway, 0)],
            height_field_id(RoadSurfaceBandKind::Carriageway, 0),
            [seam_source(0)],
        )
        .expect("first vertex should insert");
    let high = arrangement
        .insert_vertex(
            point,
            2.0,
            [owner(RoadSurfaceBandKind::Sidewalk, 1)],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 1),
            [seam_source(1)],
        )
        .expect("explicit owner context keeps steep endpoint-height duplicates deterministic");

    assert_ne!(low, high);
    assert_eq!(arrangement.vertices().len(), 2);
}

#[test]
fn arrangement_rejects_different_material_height_context_without_explicit_seam() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let heights = two_region_height_solution_with_material_heights(
        carriageway,
        sidewalk,
        0.0,
        1.0,
        Vec::new(),
        Vec::new(),
    );

    assert!(matches!(
        NodeArrangement::from_height_solution(&heights),
        Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
    ));
}

#[test]
fn arrangement_accepts_different_material_height_context_at_explicit_seam_endpoint() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 31,
        seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
        owner: None,
        opposite_owner: None,
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let heights = two_region_height_solution_with_material_heights(
        carriageway,
        sidewalk,
        0.0,
        1.0,
        vec![seam.clone()],
        vec![seam],
    );

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("explicit material seam endpoints may carry distinct field heights");

    assert_eq!(arrangement.vertices().len(), 8);
    assert!(arrangement.edges().iter().any(|edge| {
        edge.owner == carriageway
            && edge.opposite_owner == Some(sidewalk)
            && edge.is_material_transition
            && edge.source_constraint_indices == vec![31]
    }));
}

#[test]
fn arrangement_accepts_different_material_height_context_at_explicit_point_seam() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 32,
        seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
        owner: None,
        opposite_owner: None,
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 1.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let heights = NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
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
                sidewalk,
                vec![
                    height_vertex(1.0, 1.0, 1.0),
                    height_vertex(2.0, 1.0, 1.0),
                    height_vertex(2.0, 2.0, 1.0),
                    height_vertex(1.0, 2.0, 1.0),
                ],
                vec![seam],
            ),
        ],
    };

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("explicit material point seam may carry distinct field heights");

    assert_eq!(arrangement.vertices().len(), 8);
    assert!(arrangement.edges().iter().all(|edge| {
        edge.opposite_owner != Some(sidewalk) || edge.source_constraint_indices != vec![32]
    }));
}

#[test]
fn arrangement_rejects_mismatched_explicit_seam_owner_pair() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let adjacent_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let terminal_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 6);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 92,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 6 },
        owner: Some(carriageway),
        opposite_owner: Some(terminal_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let heights = NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::Terminal,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
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
                adjacent_curb,
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

    assert!(matches!(
        NodeArrangement::from_height_solution(&heights),
        Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
    ));
}
