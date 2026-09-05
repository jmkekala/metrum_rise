// SPDX-License-Identifier: GPL-2.0-only

//! Terminal curb/shoulder raised-step constraint emission.

use super::super::owners::inner_raised_step_opposite_owner_for_segment;
use super::super::*;
use super::paths::{
    terminal_cap_band_inner_contour_path, terminal_cap_band_outer_contour_path,
    terminal_cap_material_boundary_side_edges,
};

pub(super) fn push_terminal_cap_curb_or_shoulder_boundary_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    push_terminal_cap_inner_raised_step_contact_constraints(
        mouth,
        cap_band,
        owner,
        owner_by_kind_and_source,
        constraints,
    )?;
    if let Some(points) = terminal_cap_band_outer_contour_path(cap_band) {
        push_generated_band_path_constraint(
            constraints,
            NodeRailConstraintKind::RaisedStepContact,
            mouth.order_index,
            cap_band.source_band_index,
            owner,
            opposite_owner,
            points,
        )?;
    }
    for (start, end) in terminal_cap_material_boundary_side_edges(cap_band) {
        push_generated_band_constraint(
            constraints,
            NodeRailConstraintKind::RaisedStepContact,
            mouth.order_index,
            cap_band.source_band_index,
            owner,
            opposite_owner,
            xz(start),
            xz(end),
        )?;
    }
    Ok(())
}

fn push_terminal_cap_inner_raised_step_contact_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = terminal_cap_band_inner_contour_path(cap_band) else {
        return Ok(());
    };
    for segment in points.windows(2) {
        let opposite_owner = inner_raised_step_opposite_owner_for_segment(
            mouth,
            cap_band.source_band_index,
            segment[0],
            segment[1],
            owner_by_kind_and_source,
        );
        push_generated_band_constraint(
            constraints,
            NodeRailConstraintKind::RaisedStepContact,
            mouth.order_index,
            cap_band.source_band_index,
            owner,
            opposite_owner,
            segment[0],
            segment[1],
        )?;
    }
    Ok(())
}
