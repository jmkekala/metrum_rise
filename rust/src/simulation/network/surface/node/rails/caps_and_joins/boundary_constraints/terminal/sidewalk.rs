// SPDX-License-Identifier: GPL-2.0-only

//! Terminal sidewalk footprint and raised-step constraint emission.

use super::super::*;
use super::paths::{
    terminal_cap_band_inner_contour_path, terminal_cap_band_outer_contour_path,
    terminal_cap_corner_footprint_edge_for_role, terminal_cap_corner_material_edge_for_role,
    terminal_cap_side_footprint_edge_for_role,
};

pub(super) fn push_terminal_cap_sidewalk_boundary_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let inner_path = terminal_cap_band_inner_contour_path(cap_band);
    let outer_path = terminal_cap_band_outer_contour_path(cap_band);
    match cap_band.provenance.role {
        TerminalCapBandRole::EndBand => {
            if let Some(points) = inner_path {
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
            if let Some(points) = outer_path {
                push_terminal_cap_footprint_path_constraint(
                    mouth,
                    cap_band,
                    owner,
                    points,
                    constraints,
                )?;
            }
        }
        TerminalCapBandRole::LeftSide | TerminalCapBandRole::RightSide => {
            if let Some(points) = outer_path {
                push_terminal_cap_footprint_path_constraint(
                    mouth,
                    cap_band,
                    owner,
                    points,
                    constraints,
                )?;
            }
            if let Some((start, end)) =
                terminal_cap_side_footprint_edge_for_role(cap_band, cap_band.provenance.role)
            {
                push_terminal_cap_footprint_edge_constraint(
                    mouth,
                    cap_band,
                    owner,
                    start,
                    end,
                    constraints,
                )?;
            }
        }
        TerminalCapBandRole::LeftCorner | TerminalCapBandRole::RightCorner => {
            if let Some((start, end)) =
                terminal_cap_corner_material_edge_for_role(cap_band, cap_band.provenance.role)
            {
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
            if let Some((start, end)) =
                terminal_cap_corner_footprint_edge_for_role(cap_band, cap_band.provenance.role)
            {
                push_terminal_cap_footprint_edge_constraint(
                    mouth,
                    cap_band,
                    owner,
                    start,
                    end,
                    constraints,
                )?;
            }
        }
    }
    Ok(())
}

fn push_terminal_cap_footprint_path_constraint(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    points: Vec<RoadVec2>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    push_generated_band_path_constraint(
        constraints,
        NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: RoadSurfaceBandKind::Sidewalk,
        },
        mouth.order_index,
        cap_band.source_band_index,
        owner,
        None,
        points,
    )
}

fn push_terminal_cap_footprint_edge_constraint(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    start: RoadVec3,
    end: RoadVec3,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    push_generated_band_constraint(
        constraints,
        NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: RoadSurfaceBandKind::Sidewalk,
        },
        mouth.order_index,
        cap_band.source_band_index,
        owner,
        None,
        xz(start),
        xz(end),
    )
}
