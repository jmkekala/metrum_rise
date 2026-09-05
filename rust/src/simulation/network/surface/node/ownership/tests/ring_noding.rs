// SPDX-License-Identifier: GPL-2.0-only

//! Owned-region ring noding tests.

use super::*;

#[test]
fn owned_region_rings_are_noded_before_explicit_seam_validation() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[1.0, 0.0], [3.0, 0.0], [3.0, -1.0], [1.0, -1.0]],
        ),
    ];
    let footprint_shapes = Vec::new();

    canonicalize_owned_region_rings(&mut regions, &footprint_shapes);
    for region in &mut regions {
        region.seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 0,
            seam_source: NodeSeamSource::AsphaltBoundary {
                owner_index: region.owner.owner_index(),
            },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(3.0, 0.0),
        });
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }

    let carriageway_contour = &regions[0].shape[0];
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_overlay_point([1.0, 0.0]))
    );
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_overlay_point([3.0, 0.0]))
    );
    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                let start = ownership_key_from_road_point(constraint.start_xz);
                let end = ownership_key_from_road_point(constraint.end_xz);
                start == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0))
                    && end == ownership_key_from_road_point(RoadVec2::new(3.0, 0.0))
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
            }),
            "region {:?} must own the exact shared sub-edge seam before height/CDT without inventing height authority",
            region.owner
        );
    }
    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &footprint_shapes,
        &[],
    );
    assert!(arrangement.diagnostics().is_empty());
    assert!(arrangement.edges().iter().any(|edge| {
        edge.owner == carriageway
            && edge.opposite_owner == Some(sidewalk)
            && edge.start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(1.0, 0.0))
            && edge.end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(3.0, 0.0))
            && !edge.source_constraint_indices.is_empty()
    }));
}

#[test]
fn owner_source_points_are_materialized_on_overlay_grid_edges_before_height_evaluation() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 8);
    let source_point = RoadVec2::new(5.0, 5.000001);
    let constraints = [NodeRailConstraint {
        constraint_index: 0,
        kind: NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(carriageway),
        opposite_owner: None,
        points_xz: vec![source_point],
    }];
    let rail_canonical_points = test_rail_canonical_points_from_constraints(&constraints);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![[0.0, 0.0], [10.0, 10.0], [10.0, 12.0], [0.0, 2.0]],
    )];

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("source-authorized rail point should canonicalize onto owned edge");

    let source_key = ownership_key_from_road_point(source_point);
    assert!(
        regions[0].shape[0]
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point) == source_key),
        "source-authorized split point must be present before height-field completeness runs"
    );
}
