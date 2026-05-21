//! Terminal-cap boundary constraint emission.

use super::owners::{
    inner_raised_step_opposite_owner_for_segment, terminal_cap_band_material_opposite_owner,
};
use super::*;

pub(in crate::simulation::network::surface::node::rails::caps_and_joins) fn push_terminal_cap_band_boundary_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let inner_path = terminal_cap_band_inner_contour_path(cap_band);
    let outer_path = terminal_cap_band_outer_contour_path(cap_band);
    let opposite_owner =
        terminal_cap_band_material_opposite_owner(mouth, cap_band, owner_by_kind_and_source);
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            push_terminal_cap_inner_raised_step_contact_constraints(
                mouth,
                cap_band,
                owner,
                owner_by_kind_and_source,
                constraints,
            )?;
            if let Some(points) = outer_path.clone() {
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
        RoadSurfaceBandKind::Sidewalk => push_terminal_cap_sidewalk_boundary_constraints(
            mouth,
            cap_band,
            owner,
            opposite_owner,
            inner_path,
            outer_path,
            constraints,
        ),
        _ => Ok(()),
    }
}

fn push_terminal_cap_sidewalk_boundary_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    inner_path: Option<Vec<RoadVec2>>,
    outer_path: Option<Vec<RoadVec2>>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
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

fn terminal_cap_material_boundary_side_edges(
    cap_band: &NodeTerminalCapBand,
) -> Vec<(RoadVec3, RoadVec3)> {
    let Some(inner_start_world) = cap_band.inner_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(inner_end_world) = cap_band.inner_path_world.last().copied() else {
        return Vec::new();
    };
    let Some(outer_start_world) = cap_band.outer_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(outer_end_world) = cap_band.outer_path_world.last().copied() else {
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

fn terminal_cap_side_footprint_edge_for_role(
    cap_band: &NodeTerminalCapBand,
    role: TerminalCapBandRole,
) -> Option<(RoadVec3, RoadVec3)> {
    match role {
        TerminalCapBandRole::LeftSide => terminal_cap_start_side_edge(cap_band),
        TerminalCapBandRole::RightSide => terminal_cap_end_side_edge(cap_band),
        TerminalCapBandRole::LeftCorner
        | TerminalCapBandRole::EndBand
        | TerminalCapBandRole::RightCorner => None,
    }
}

fn terminal_cap_corner_material_edge_for_role(
    cap_band: &NodeTerminalCapBand,
    role: TerminalCapBandRole,
) -> Option<(RoadVec3, RoadVec3)> {
    match role {
        TerminalCapBandRole::LeftCorner => terminal_cap_end_side_edge(cap_band),
        TerminalCapBandRole::RightCorner => terminal_cap_start_side_edge(cap_band),
        TerminalCapBandRole::LeftSide
        | TerminalCapBandRole::EndBand
        | TerminalCapBandRole::RightSide => None,
    }
}

fn terminal_cap_corner_footprint_edge_for_role(
    cap_band: &NodeTerminalCapBand,
    role: TerminalCapBandRole,
) -> Option<(RoadVec3, RoadVec3)> {
    match role {
        TerminalCapBandRole::LeftCorner => terminal_cap_start_side_edge(cap_band),
        TerminalCapBandRole::RightCorner => terminal_cap_end_side_edge(cap_band),
        TerminalCapBandRole::LeftSide
        | TerminalCapBandRole::EndBand
        | TerminalCapBandRole::RightSide => None,
    }
}

fn terminal_cap_start_side_edge(cap_band: &NodeTerminalCapBand) -> Option<(RoadVec3, RoadVec3)> {
    let inner_start_world = cap_band.inner_path_world.first().copied()?;
    let outer_start_world = cap_band.outer_path_world.first().copied()?;
    (road_point_key(xz(inner_start_world)) != road_point_key(xz(outer_start_world)))
        .then_some((inner_start_world, outer_start_world))
}

fn terminal_cap_end_side_edge(cap_band: &NodeTerminalCapBand) -> Option<(RoadVec3, RoadVec3)> {
    let inner_end_world = cap_band.inner_path_world.last().copied()?;
    let outer_end_world = cap_band.outer_path_world.last().copied()?;
    (road_point_key(xz(inner_end_world)) != road_point_key(xz(outer_end_world)))
        .then_some((inner_end_world, outer_end_world))
}

fn terminal_cap_band_inner_contour_path(cap_band: &NodeTerminalCapBand) -> Option<Vec<RoadVec2>> {
    let points = cap_band
        .inner_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}

fn terminal_cap_band_outer_contour_path(cap_band: &NodeTerminalCapBand) -> Option<Vec<RoadVec2>> {
    let points = cap_band
        .outer_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}
