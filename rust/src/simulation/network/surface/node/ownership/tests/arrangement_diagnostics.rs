use super::*;

#[test]
fn owned_region_arrangement_reports_shared_edge_without_seam_constraint() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]],
        ),
    ];

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        43,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &Vec::new(),
        &[],
    );

    assert!(arrangement.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic,
        NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
            region_index: 0,
            owner,
            opposite_owner,
            start,
            end,
        } if *owner == carriageway
            && *opposite_owner == sidewalk
            && *start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 0.0))
            && *end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 2.0))
    )));
}

#[test]
fn owned_region_arrangement_reports_ambiguous_multi_owner_edge() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let shared_right_contour = vec![[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]];
    let regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            shared_right_contour.clone(),
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            shared_right_contour,
        ),
    ];

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        45,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &Vec::new(),
        &[],
    );

    assert!(arrangement.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic,
        NodeOwnedRegionArrangementDiagnostic::AmbiguousOwnedBoundaryEdge {
            region_index: 0,
            owner,
            opposite_owners,
            start,
            end,
        } if *owner == carriageway
            && opposite_owners.len() == 2
            && opposite_owners.contains(&sidewalk)
            && opposite_owners.contains(&curb)
            && *start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 0.0))
            && *end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 2.0))
    )));
}

#[test]
fn same_band_owned_region_edge_does_not_require_material_seam_constraint() {
    let first = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let second = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 1);
    let regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            first,
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            second,
            vec![[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]],
        ),
    ];

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        44,
        RoadSurfaceVisualNodePieceKind::Bend,
        &regions,
        &Vec::new(),
        &[],
    );

    assert!(arrangement.diagnostics().is_empty());
}
