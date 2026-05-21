//! Terminal-cap and side-join rail boundary constraints.

use super::*;

pub(super) fn push_terminal_cap_band_boundary_constraints(
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

pub(super) fn push_side_join_band_boundary_constraints(
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
            Ok(())
        }
        _ => Ok(()),
    }
}

fn terminal_cap_band_material_opposite_owner(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if let Some(owner) =
        terminal_cap_generated_material_opposite_owner(cap_band, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => adjacent_source_band_owner(
            mouth,
            cap_band.source_band_index,
            RoadSurfaceBandKind::Sidewalk,
            owner_by_kind_and_source,
        ),
        RoadSurfaceBandKind::Sidewalk => adjacent_source_band_owner(
            mouth,
            cap_band.source_band_index,
            RoadSurfaceBandKind::CurbOrShoulder,
            owner_by_kind_and_source,
        ),
        _ => None,
    }
}

fn terminal_cap_generated_material_opposite_owner(
    cap_band: &NodeTerminalCapBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            cap_band
                .source_band_index
                .checked_add(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::Sidewalk, source_band_index))
                        .copied()
                })
        }
        RoadSurfaceBandKind::Sidewalk => {
            cap_band
                .source_band_index
                .checked_sub(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::CurbOrShoulder, source_band_index))
                        .copied()
                })
        }
        _ => None,
    }
}

fn side_join_band_material_opposite_owner(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    match side_join_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => adjacent_source_band_owner(
            mouth,
            side_join_band.source_band_index,
            RoadSurfaceBandKind::Sidewalk,
            owner_by_kind_and_source,
        ),
        RoadSurfaceBandKind::Sidewalk => adjacent_source_band_owner(
            mouth,
            side_join_band.source_band_index,
            RoadSurfaceBandKind::CurbOrShoulder,
            owner_by_kind_and_source,
        ),
        _ => None,
    }
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

fn inner_raised_step_opposite_owner_for_segment(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if let Some(owner) =
        endpoint_raised_step_opposite_owner(mouth, start, end, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    adjacent_source_band_owner(
        mouth,
        source_band_index,
        RoadSurfaceBandKind::Carriageway,
        owner_by_kind_and_source,
    )
}

fn endpoint_raised_step_opposite_owner(
    mouth: &NodeInputMouth,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let start_boundary = endpoint_boundary_index_for_point(mouth, start)?;
    let end_boundary = endpoint_boundary_index_for_point(mouth, end)?;
    let (lower_boundary, upper_boundary) = if start_boundary <= end_boundary {
        (start_boundary, end_boundary)
    } else {
        (end_boundary, start_boundary)
    };
    if lower_boundary + 1 != upper_boundary {
        return None;
    }
    let interval = mouth.band_intervals.get(lower_boundary)?;
    if !is_carriageway(interval.band_kind) {
        return None;
    }
    owner_by_kind_and_source
        .get(&(RoadSurfaceBandKind::Carriageway, interval.band_index))
        .copied()
}

fn adjacent_source_band_owner(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    adjacent_kind: RoadSurfaceBandKind,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let source = mouth.band_intervals.get(source_band_index)?;
    let mut adjacent_owner = None;
    for adjacent_index in [
        source_band_index.checked_sub(1),
        source_band_index.checked_add(1),
    ]
    .into_iter()
    .flatten()
    {
        let Some(adjacent) = mouth.band_intervals.get(adjacent_index) else {
            continue;
        };
        if adjacent.band_kind != adjacent_kind {
            continue;
        }
        let owner = owner_by_kind_and_source
            .get(&(adjacent.band_kind, adjacent.band_index))
            .copied()?;
        if adjacent_owner.replace(owner).is_some() {
            return None;
        }
    }
    (source.band_kind != adjacent_kind)
        .then_some(adjacent_owner)
        .flatten()
}

fn endpoint_boundary_index_for_point(mouth: &NodeInputMouth, point: RoadVec2) -> Option<usize> {
    let key = road_point_key(point);
    mouth
        .boundary_rails
        .iter()
        .find(|rail| road_point_key(xz(rail.endpoint_world)) == key)
        .map(|rail| rail.boundary_index)
}

fn side_join_band_has_material_boundary(side_join_band: &NodeInputSideJoinBand) -> bool {
    matches!(
        side_join_band.boundary_mode,
        NodeInputSideJoinBandBoundaryMode::MaterialBand
    )
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
