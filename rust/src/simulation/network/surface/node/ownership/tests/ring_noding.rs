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
