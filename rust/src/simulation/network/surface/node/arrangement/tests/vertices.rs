// SPDX-License-Identifier: GPL-2.0-only

//! Arrangement vertex identity and height-context tests.

use super::*;

#[test]
fn duplicate_arrangement_vertex_key_merges_matching_owner_source_context() {
    let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::JunctionN);
    let point = RoadVec2::new(12.345, -6.789);

    let first = arrangement
        .insert_vertex(
            point,
            3.25,
            [owner(RoadSurfaceBandKind::Sidewalk, 2)],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 2),
            [seam_source(2)],
        )
        .expect("first vertex should insert");
    let second = arrangement
        .insert_vertex(
            point,
            3.25,
            [owner(RoadSurfaceBandKind::Sidewalk, 2)],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 2),
            [seam_source(2)],
        )
        .expect("matching context vertex should merge");

    assert_eq!(first, second);
    let vertex = &arrangement.vertices()[first.0];
    assert_eq!(vertex.owners, vec![owner(RoadSurfaceBandKind::Sidewalk, 2)]);
    assert_eq!(vertex.seam_sources, vec![seam_source(2)]);
}

#[test]
fn duplicate_arrangement_vertex_key_merges_same_height_field_and_quantized_height() {
    let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::JunctionN);
    let point = RoadVec2::new(12.345, -6.789);
    let field_id = height_field_id(RoadSurfaceBandKind::Sidewalk, 2);

    let first = arrangement
        .insert_vertex(
            point,
            3.25,
            [owner(RoadSurfaceBandKind::Sidewalk, 2)],
            field_id,
            [seam_source(2)],
        )
        .expect("first vertex should insert");
    let second = arrangement
        .insert_vertex(
            point,
            3.25,
            [owner(RoadSurfaceBandKind::Sidewalk, 2)],
            field_id,
            [NodeSeamSource::SidewalkOuter { owner_index: 2 }],
        )
        .expect("same height-field and solved height should share the canonical vertex");

    assert_eq!(first, second);
    let vertex = &arrangement.vertices()[first.0];
    assert_eq!(vertex.height_field_id(), field_id);
    assert_eq!(
        vertex.seam_sources,
        vec![
            NodeSeamSource::SidewalkOuter { owner_index: 2 },
            seam_source(2)
        ]
    );
}

#[test]
fn duplicate_arrangement_vertex_key_keeps_distinct_material_height_field_contexts() {
    let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::JunctionN);
    let point = RoadVec2::new(12.345, -6.789);

    let first = arrangement
        .insert_vertex(
            point,
            3.25,
            [owner(RoadSurfaceBandKind::Sidewalk, 2)],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 2),
            [seam_source(2)],
        )
        .expect("first vertex should insert");
    let second = arrangement
        .insert_vertex(
            point,
            3.25,
            [owner(RoadSurfaceBandKind::CurbOrShoulder, 1)],
            height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1),
            [seam_source(1)],
        )
        .expect("distinct height source context should keep its own vertex");

    assert_ne!(first, second);
    assert_eq!(arrangement.vertices().len(), 2);
}

#[test]
fn junctionn_arrangement_vertices_preserve_node_grade_authority() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let heights = two_region_height_solution(carriageway, sidewalk, Vec::new(), Vec::new());
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("heighted JunctionN should arrange with grade authority");

    assert!(!arrangement.vertices().is_empty());
    assert!(arrangement.vertices().iter().all(|vertex| {
        matches!(
            vertex.grade_authority().decision,
            NodeGradeCarrierDecision::SourceCarrier { .. }
        )
    }));
}

#[test]
fn arrangement_contour_removes_immediate_backtracking_spur() {
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 3);
    let heights = NodeHeightSolution {
        node_id: 41,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![
                height_vertex(0.0, 0.0, 0.0),
                height_vertex(1.0, 0.0, 0.0),
                height_vertex(1.001, 0.05, 0.0),
                height_vertex(1.0, 0.0, 0.0),
                height_vertex(0.0, 1.0, 0.0),
            ],
            Vec::new(),
        )],
    };

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("zero-area backtracking spur should canonicalize before CDT");

    assert_eq!(arrangement.regions()[0].outer_loop().len(), 3);
}

#[test]
fn arrangement_contour_preserves_same_millimetre_distinct_canonical_keys() {
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 3);
    let heights = NodeHeightSolution {
        node_id: 42,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![
                height_vertex(0.0000, 0.0, 0.0),
                height_vertex(0.0004, 0.4, 0.0),
                height_vertex(0.0004, 0.0, 0.0),
                height_vertex(1.0000, 0.0, 0.0),
                height_vertex(1.0000, 1.0, 0.0),
                height_vertex(0.0000, 1.0, 0.0),
            ],
            Vec::new(),
        )],
    };

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("same-millimetre canonical vertices must remain distinct");

    assert_eq!(
        arrangement.regions()[0].outer_loop().len(),
        6,
        "arrangement cleanup must not collapse distinct canonical XZ keys just because they share a millimetre bucket"
    );
}

#[test]
fn arrangement_rejects_heighted_vertex_without_node_grade_authority() {
    let owner = owner(RoadSurfaceBandKind::Sidewalk, 4);
    let field = NodeBandHeightFieldId::new(0, 4, RoadSurfaceBandKind::Sidewalk);
    let mut shape = vec![
        height_vertex(0.0, 0.0, 1.0),
        height_vertex(1.0, 0.0, 1.0),
        height_vertex(0.0, 1.0, 1.0),
    ];
    for vertex in &mut shape {
        vertex.height_field_id = field;
    }
    let heights = NodeHeightSolution {
        node_id: 81,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner,
            height_field_id: field,
            shape: vec![shape],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };

    assert!(matches!(
        NodeArrangement::from_height_solution(&heights),
        Err(NodeArrangementError::MissingGradeAuthority {
            owner: missing_owner,
            height_field_id,
            ..
        }) if missing_owner == owner && height_field_id == field
    ));
}

#[test]
fn duplicate_arrangement_vertex_key_rejects_same_context_height_conflict() {
    let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::JunctionN);
    let point = RoadVec2::new(0.0, 0.0);

    arrangement
        .insert_vertex(
            point,
            1.0,
            [owner(RoadSurfaceBandKind::Sidewalk, 0)],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
            [seam_source(0)],
        )
        .expect("first vertex should insert");

    let result = arrangement.insert_vertex(
        point,
        1.01,
        [owner(RoadSurfaceBandKind::Sidewalk, 0)],
        height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
        [seam_source(0)],
    );

    assert!(matches!(
        result,
        Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
    ));
}

#[test]
fn arrangement_vertex_requires_explicit_owner() {
    let mut arrangement = NodeArrangement::new(9, RoadSurfaceVisualNodePieceKind::Terminal);

    let result = arrangement.insert_vertex(
        RoadVec2::new(1.0, 2.0),
        0.0,
        [],
        height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
        [seam_source(0)],
    );

    assert!(matches!(
        result,
        Err(NodeArrangementError::EmptyOwnerSet { .. })
    ));
}
