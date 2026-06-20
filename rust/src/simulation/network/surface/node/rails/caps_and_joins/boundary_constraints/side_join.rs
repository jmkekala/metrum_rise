//! Side-join boundary constraint emission.

use super::owners::{
    inner_raised_step_opposite_owner_for_segment, side_join_band_material_opposite_owner,
};
use super::*;

pub(in crate::simulation::network::surface::node::rails::caps_and_joins) fn push_side_join_band_boundary_constraints(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let inner_path = side_join_band_inner_contour_path(side_join_band);
    let outer_path = side_join_band_outer_contour_path(side_join_band);
    let opposite_owner =
        side_join_band_material_opposite_owner(mouth, side_join_band, owner_by_kind_and_source);
    match side_join_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            if side_join_band.boundary_mode != NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap
            {
                push_side_join_inner_raised_step_contact_constraints(
                    mouth,
                    side_join_band,
                    owner,
                    owner_by_kind_and_source,
                    constraints,
                )?;
            }
            if side_join_band_has_material_boundary(side_join_band)
                && let Some(points) = outer_path.clone()
            {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::RaisedStepContact,
                    mouth.order_index,
                    side_join_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            if side_join_band_has_material_boundary(side_join_band) {
                for (start, end) in side_join_material_boundary_side_edges(side_join_band) {
                    push_generated_band_constraint(
                        constraints,
                        NodeRailConstraintKind::RaisedStepContact,
                        mouth.order_index,
                        side_join_band.source_band_index,
                        owner,
                        opposite_owner,
                        xz(start),
                        xz(end),
                    )?;
                }
            }
            Ok(())
        }
        RoadSurfaceBandKind::Sidewalk => {
            if side_join_band.boundary_mode != NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap
                && let Some(points) = inner_path
            {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::RaisedStepContact,
                    mouth.order_index,
                    side_join_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            if side_join_band_has_material_boundary(side_join_band)
                && let Some(points) = outer_path
            {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    },
                    mouth.order_index,
                    side_join_band.source_band_index,
                    owner,
                    None,
                    points,
                )?;
            }
            if side_join_band_has_material_boundary(side_join_band) {
                for (start, end) in side_join_material_boundary_side_edges(side_join_band) {
                    push_generated_band_constraint(
                        constraints,
                        NodeRailConstraintKind::SpanHandoff {
                            kind: RoadSurfaceBandKind::Sidewalk,
                        },
                        mouth.order_index,
                        side_join_band.source_band_index,
                        owner,
                        None,
                        xz(start),
                        xz(end),
                    )?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn push_side_join_inner_raised_step_contact_constraints(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = side_join_band_inner_contour_path(side_join_band) else {
        return Ok(());
    };
    for segment in points.windows(2) {
        let opposite_owner = inner_raised_step_opposite_owner_for_segment(
            mouth,
            side_join_band.source_band_index,
            segment[0],
            segment[1],
            owner_by_kind_and_source,
        );
        push_generated_band_constraint(
            constraints,
            NodeRailConstraintKind::RaisedStepContact,
            mouth.order_index,
            side_join_band.source_band_index,
            owner,
            opposite_owner,
            segment[0],
            segment[1],
        )?;
    }
    Ok(())
}

fn side_join_band_has_material_boundary(side_join_band: &NodeInputSideJoinBand) -> bool {
    matches!(
        side_join_band.boundary_mode,
        NodeInputSideJoinBandBoundaryMode::MaterialBand
    )
}

fn side_join_material_boundary_side_edges(
    side_join_band: &NodeInputSideJoinBand,
) -> Vec<(RoadVec3, RoadVec3)> {
    let Some(inner_start_world) = side_join_band.inner_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(inner_end_world) = side_join_band.inner_path_world.last().copied() else {
        return Vec::new();
    };
    let Some(outer_start_world) = side_join_band.outer_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(outer_end_world) = side_join_band.outer_path_world.last().copied() else {
        return Vec::new();
    };
    [
        (inner_start_world, outer_start_world),
        (inner_end_world, outer_end_world),
    ]
    .into_iter()
    .filter(|(start, end)| road_point_key(xz(*start)) != road_point_key(xz(*end)))
    .collect()
}

fn side_join_band_inner_contour_path(
    side_join_band: &NodeInputSideJoinBand,
) -> Option<Vec<RoadVec2>> {
    let points = side_join_band
        .inner_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}

fn side_join_band_outer_contour_path(
    side_join_band: &NodeInputSideJoinBand,
) -> Option<Vec<RoadVec2>> {
    let points = side_join_band
        .outer_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}
