//! Arrangement edge ownership and diagnostics tests.

use super::*;

#[test]
fn arrangement_edges_match_opposite_owners_by_canonical_xz_segment() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let generic_seam = NodeRegionSeamConstraint {
        constraint_index: 2,
        seam_source: NodeSeamSource::FootprintBoundary { owner_index: 0 },
        owner: None,
        opposite_owner: None,
        constrains_shared_height: true,
        is_material_transition: false,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let shared_seam = NodeRegionSeamConstraint {
        constraint_index: 17,
        seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
        owner: None,
        opposite_owner: None,
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
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
                vec![generic_seam.clone(), shared_seam.clone()],
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                sidewalk,
                vec![
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(2.0, 0.0, 0.0),
                    height_vertex(2.0, 1.0, 0.0),
                    height_vertex(1.0, 1.0, 0.0),
                ],
                vec![generic_seam, shared_seam],
            ),
        ],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("height-owned regions should produce canonical arrangement");

    assert_eq!(arrangement.vertices().len(), 8);
    assert!(arrangement.edges().iter().any(|edge| {
        edge.owner == carriageway
            && edge.opposite_owner == Some(sidewalk)
            && matches!(edge.seam_source, NodeSeamSource::AsphaltBoundary { .. })
            && edge.source_constraint_indices == vec![2, 17]
    }));
    assert!(arrangement.edges().iter().any(|edge| {
        edge.owner == sidewalk
            && edge.opposite_owner == Some(carriageway)
            && matches!(edge.seam_source, NodeSeamSource::AsphaltBoundary { .. })
            && edge.source_constraint_indices == vec![2, 17]
    }));
}

#[test]
fn shared_arrangement_edge_reports_missing_source_constraint() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let heights = two_region_height_solution(carriageway, sidewalk, Vec::new(), Vec::new());

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("height-owned regions should produce canonical arrangement diagnostics");

    assert!(matches!(
        arrangement.diagnostics().first(),
        Some(NodeArrangementDiagnostic::MissingSeamConstraint {
            region_index: 0,
            owner,
            opposite_owner,
            ..
        }) if *owner == carriageway && *opposite_owner == sidewalk
    ));
}

#[test]
fn same_band_arrangement_edge_does_not_require_material_seam_constraint() {
    let first = owner(RoadSurfaceBandKind::Carriageway, 0);
    let second = owner(RoadSurfaceBandKind::Carriageway, 1);
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
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
                RoadSurfaceBandKind::Carriageway,
                second,
                vec![
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(2.0, 0.0, 0.0),
                    height_vertex(2.0, 1.0, 0.0),
                    height_vertex(1.0, 1.0, 0.0),
                ],
                Vec::new(),
            ),
        ],
    };

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("same-band owned regions should share a non-material boundary");

    assert!(arrangement.diagnostics().is_empty());
}

#[test]
fn equally_ranked_conflicting_arrangement_seams_are_reported() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let first = NodeRegionSeamConstraint {
        constraint_index: 20,
        seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
        owner: None,
        opposite_owner: None,
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let second = NodeRegionSeamConstraint {
        constraint_index: 21,
        seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 1 },
        owner: None,
        opposite_owner: None,
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let seams = vec![first, second];
    let heights = two_region_height_solution(carriageway, sidewalk, seams.clone(), seams);

    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("height-owned regions should produce canonical arrangement diagnostics");

    assert!(matches!(
        arrangement.diagnostics().first(),
        Some(NodeArrangementDiagnostic::AmbiguousSeamConstraint {
            region_index: 0,
            owner,
            opposite_owner,
            ..
        }) if *owner == carriageway && *opposite_owner == sidewalk
    ));
}
