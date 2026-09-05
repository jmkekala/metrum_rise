// SPDX-License-Identifier: GPL-2.0-only

//! Explicit vertical-step ambiguity and missing-source rejection tests.

use super::*;

#[test]
fn explicit_vertical_step_segments_reject_ambiguous_exposed_endpoint_pairs() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 14);
    let first_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let second_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 13);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let contact = |constraint_index, point, curb| NodeRegionSeamConstraint {
        constraint_index,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 14 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: point,
        end_xz: point,
    };
    let heights = NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![
                height_vertex(0.0, 0.0, 0.0),
                height_vertex(2.0, 0.0, 0.0),
                height_vertex(0.0, 1.0, 0.0),
            ],
            vec![
                contact(325, start, first_curb),
                contact(327, end, first_curb),
                contact(329, start, second_curb),
                contact(331, end, second_curb),
            ],
        )],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("ambiguous endpoint contacts should still produce a canonical arrangement");

    assert!(arrangement.explicit_vertical_step_segments().is_empty());
}

#[test]
fn explicit_vertical_step_segments_require_explicit_owner_pair_source() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 93,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
        owner: Some(curb),
        opposite_owner: None,
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let heights = two_region_height_solution_with_material_heights(
        carriageway,
        curb,
        0.0,
        0.12,
        vec![seam.clone()],
        vec![seam],
    );
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("role-only material seam should produce a canonical arrangement");

    let segments = arrangement.explicit_vertical_step_segments();
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 1.0)),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");

    assert!(!segments.contains(&expected));
}
