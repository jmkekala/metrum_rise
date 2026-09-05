// SPDX-License-Identifier: GPL-2.0-only

//! Residual rejection and post-claim ordering validation.

use super::*;

pub(in crate::simulation::network::surface::node::ownership) fn reject_residual(
    residual: NodeOverlayShapes,
    residual_kind: ResidualKind,
) -> Result<(), NodeBooleanOwnershipError> {
    if residual.is_empty() {
        return Ok(());
    }

    let shape_count = residual.len();
    let area_m2 = residual
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum();
    if area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&residual) {
        return Ok(());
    }
    match residual_kind {
        ResidualKind::Asphalt => Err(NodeBooleanOwnershipError::UnownedAsphaltResidual {
            shape_count,
            area_m2,
        }),
        ResidualKind::Band(kind) => Err(NodeBooleanOwnershipError::UnownedBandResidual {
            kind,
            shape_count,
            area_m2,
        }),
        ResidualKind::NonRoad => Err(NodeBooleanOwnershipError::UnownedNonRoadResidual {
            shape_count,
            area_m2,
        }),
    }
}

pub(in crate::simulation::network::surface::node::ownership) fn sort_boolean_owned_regions(
    regions: &mut [NodeBooleanOwnedRegion],
) {
    regions.sort_by(|a, b| {
        RoadSurfaceSystem::band_kind_sort_key(a.kind)
            .cmp(&RoadSurfaceSystem::band_kind_sort_key(b.kind))
            .then(a.claim_priority.cmp(&b.claim_priority))
            .then(a.source_mouth_order_index.cmp(&b.source_mouth_order_index))
            .then(a.source_band_index.cmp(&b.source_band_index))
            .then(a.area_m2.total_cmp(&b.area_m2))
    });
}

pub(in crate::simulation::network::surface::node::ownership) fn validate_non_road_regions_have_explicit_profile_seam_rails(
    regions: &[NodeBooleanOwnedRegion],
    rail_constraints: &[NodeRailConstraint],
) -> Result<(), NodeBooleanOwnershipError> {
    let mut missing_by_kind = BTreeMap::<RoadSurfaceBandKind, (usize, f32)>::new();
    for region in regions {
        if !band_kind_requires_explicit_profile_seam_rail(region.kind)
            || region_has_explicit_profile_seam_rail(&region.seam_constraints, rail_constraints)
        {
            continue;
        }
        let entry = missing_by_kind.entry(region.kind).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += region.area_m2;
    }
    if let Some((kind, (shape_count, area_m2))) = missing_by_kind.into_iter().next() {
        return Err(NodeBooleanOwnershipError::UnownedBandResidual {
            kind,
            shape_count,
            area_m2,
        });
    }
    Ok(())
}

pub(super) fn band_kind_requires_explicit_profile_seam_rail(kind: RoadSurfaceBandKind) -> bool {
    matches!(
        kind,
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    )
}

pub(super) fn region_has_explicit_profile_seam_rail(
    seam_constraints: &[NodeRegionSeamConstraint],
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    seam_constraints.iter().any(|seam| {
        rail_constraints
            .iter()
            .find(|constraint| constraint.constraint_index == seam.constraint_index)
            .is_some_and(rail_constraint_is_explicit_profile_seam_rail)
    })
}

fn rail_constraint_is_explicit_profile_seam_rail(constraint: &NodeRailConstraint) -> bool {
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::FootprintSeam { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::RaisedStepContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
}
