//! Seam extraction and materialization helpers for node boolean ownership.

use super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource};
use super::super::backend::{RoadVec2, overlay_point_to_road};
use super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::{
    NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use super::{
    NodeBooleanOwnedRegion, NodeOwnershipPointKey, canonical_owned_region_edge_refs,
    canonical_source_indices, opposite_owner_for_ref, owned_region_boundary_refs,
    owners_form_raised_step_contact, ownership_key_from_overlay_point,
    ownership_key_from_road_point, point_key_collinear_with_edge,
    point_key_collinear_with_edge_on_overlay_grid, point_key_lies_on_segment,
    raised_step_contact_constrains_shared_height,
    raised_step_contact_requires_exact_constraint_span, road_point_from_key, segment_parameter_key,
};
use std::collections::BTreeSet;

pub(super) fn owned_shape_is_discardable_numeric_dust(
    shape: &NodeOverlayShape,
    area_m2: f32,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    let protected_constraints = protected_constraints_for_owner(owner, rail_constraints);
    area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(shape)
        && !shape_touches_protected_boundary_constraint(shape, &protected_constraints)
}

fn shape_touches_protected_boundary_constraint(
    shape: &NodeOverlayShape,
    protected_constraints: &[&NodeRailConstraint],
) -> bool {
    for contour in shape {
        for &point in contour {
            let point = ownership_key_from_overlay_point(point);
            if protected_constraints.iter().any(|constraint| {
                constraint.points_xz.windows(2).any(|segment| {
                    point_key_lies_on_segment(
                        point,
                        ownership_key_from_road_point(segment[0]),
                        ownership_key_from_road_point(segment[1]),
                    )
                })
            }) {
                return true;
            }
        }
        if contour.len() < 2 {
            continue;
        }
        for edge_index in 0..contour.len() {
            let start = contour[edge_index];
            let end = contour[(edge_index + 1) % contour.len()];
            if protected_constraints
                .iter()
                .any(|constraint| edge_lies_on_constraint(start, end, constraint))
            {
                return true;
            }
        }
    }
    false
}

fn protected_constraints_for_owner(
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> Vec<&NodeRailConstraint> {
    rail_constraints
        .iter()
        .filter(move |constraint| constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| {
            matches!(
                constraint.kind,
                NodeRailConstraintKind::SpanHandoff { .. }
                    | NodeRailConstraintKind::FootprintSeam { .. }
            )
        })
        .collect()
}

pub(super) fn seam_constraints_for_shape(
    shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
    allow_grid_bounded_constraint_overlap: bool,
) -> Vec<NodeRegionSeamConstraint> {
    let mut seams = Vec::new();
    for contour in shape {
        if contour.len() < 2 {
            continue;
        }
        for edge_index in 0..contour.len() {
            let start = contour[edge_index];
            let end = contour[(edge_index + 1) % contour.len()];
            if ownership_key_from_overlay_point(start) == ownership_key_from_overlay_point(end) {
                continue;
            }
            for constraint in rail_constraints
                .iter()
                .filter(|constraint| constraint_applies_to_owner(constraint, owner))
            {
                if shape_edge_carries_full_seam_constraint(start, end, constraint) {
                    push_region_seam_constraint(
                        &mut seams,
                        constraint,
                        owner,
                        overlay_point_to_road(start),
                        overlay_point_to_road(end),
                    );
                }
                for (overlap_start, overlap_end) in constraint_overlaps_shape_edge(
                    start,
                    end,
                    constraint,
                    allow_grid_bounded_constraint_overlap,
                ) {
                    push_region_seam_constraint(
                        &mut seams,
                        constraint,
                        owner,
                        road_point_from_key(overlap_start),
                        road_point_from_key(overlap_end),
                    );
                }
            }
        }
        for point in contour.iter().copied() {
            for constraint in rail_constraints
                .iter()
                .filter(|constraint| constraint_applies_to_owner(constraint, owner))
                .filter(|constraint| point_lies_on_point_constraint(point, constraint))
            {
                let point_xz = overlay_point_to_road(point);
                push_region_seam_constraint(&mut seams, constraint, owner, point_xz, point_xz);
            }
        }
    }
    canonicalize_seam_constraints(&mut seams);
    seams
}

fn push_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    seams.push(NodeRegionSeamConstraint {
        constraint_index: constraint.constraint_index,
        seam_source: seam_source_from_constraint(constraint, owner),
        owner: constraint.owner,
        opposite_owner: constraint.opposite_owner,
        constrains_shared_height: constraint_constrains_shared_height(constraint),
        is_material_transition: constraint_is_material_transition(constraint),
        start_xz,
        end_xz,
    });
}

fn push_materialized_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    let (constraint_owner, constraint_opposite_owner) =
        materialized_constraint_owner_pair(constraint, owner, opposite_owner);
    let materialized_kind =
        materialized_constraint_kind_for_owned_edge(constraint, owner, opposite_owner);
    seams.push(NodeRegionSeamConstraint {
        constraint_index: constraint.constraint_index,
        seam_source: seam_source_from_materialized_constraint_kind(
            materialized_kind,
            owner,
            opposite_owner,
        ),
        owner: constraint_owner,
        opposite_owner: constraint_opposite_owner,
        constrains_shared_height: materialized_constraint_constrains_shared_height(
            constraint,
            owner,
            opposite_owner,
        ),
        is_material_transition: materialized_constraint_kind_is_material_transition(
            materialized_kind,
        ),
        start_xz,
        end_xz,
    });
}

fn push_materialized_endpoint_pair_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint_index: usize,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    seams.push(NodeRegionSeamConstraint {
        constraint_index,
        seam_source: seam_source_from_materialized_constraint_kind(kind, owner, opposite_owner),
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        constrains_shared_height: materialized_constraint_kind_constrains_shared_height(
            kind,
            owner,
            opposite_owner,
        ),
        is_material_transition: materialized_constraint_kind_is_material_transition(kind),
        start_xz,
        end_xz,
    });
}

fn constraint_overlaps_shape_edge(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
    allow_grid_bounded_constraint_overlap: bool,
) -> Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    let edge_start = ownership_key_from_overlay_point(edge_start);
    let edge_end = ownership_key_from_overlay_point(edge_end);
    if edge_start == edge_end || constraint.points_xz.len() < 2 {
        return Vec::new();
    }
    let mut overlaps = BTreeSet::new();
    for segment in constraint.points_xz.windows(2) {
        let start = ownership_key_from_road_point(segment[0]);
        let end = ownership_key_from_road_point(segment[1]);
        if start == end {
            continue;
        }
        if !allow_grid_bounded_constraint_overlap
            && (!point_key_collinear_with_edge(start, edge_start, edge_end)
                || !point_key_collinear_with_edge(end, edge_start, edge_end))
        {
            continue;
        }
        let mut points = [edge_start, edge_end, start, end]
            .into_iter()
            .filter(|point| {
                point_key_lies_on_segment(*point, edge_start, edge_end)
                    && point_key_lies_on_segment(*point, start, end)
            })
            .collect::<Vec<_>>();
        points.sort_by_key(|point| segment_parameter_key(edge_start, edge_end, *point));
        points.dedup();
        let Some(first) = points.first().copied() else {
            continue;
        };
        let Some(last) = points.last().copied() else {
            continue;
        };
        let first = canonical_constraint_overlap_endpoint(first, start, end);
        let last = canonical_constraint_overlap_endpoint(last, start, end);
        if first != last {
            overlaps.insert((first, last));
        }
    }
    overlaps.into_iter().collect()
}

fn canonical_constraint_overlap_endpoint(
    point: NodeOwnershipPointKey,
    constraint_start: NodeOwnershipPointKey,
    constraint_end: NodeOwnershipPointKey,
) -> NodeOwnershipPointKey {
    if point == constraint_start {
        return constraint_start;
    }
    if point == constraint_end {
        return constraint_end;
    }
    point
}

pub(super) fn materialize_noded_region_seam_constraints(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    piece_kind: RoadSurfaceVisualNodePieceKind,
) {
    let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
    let mut additions = vec![Vec::new(); regions.len()];
    for (edge_key, refs) in boundary_refs.edges {
        let refs = canonical_owned_region_edge_refs(&refs);
        for edge_ref in &refs {
            let Some(opposite_owner) = opposite_owner_for_ref(&refs, *edge_ref) else {
                continue;
            };
            let Some(region) = regions.get(edge_ref.region_index) else {
                continue;
            };
            let matching_constraints = rail_constraints
                .iter()
                .filter(|constraint| {
                    rail_constraint_can_materialize_for_owned_edge(
                        constraint,
                        edge_ref.owner,
                        opposite_owner,
                    )
                })
                .filter(|constraint| {
                    owned_edge_lies_on_rail_constraint(
                        edge_key.start,
                        edge_key.end,
                        constraint,
                        edge_ref.owner,
                        opposite_owner,
                        piece_kind,
                    )
                })
                .collect::<Vec<_>>();
            if matching_constraints.is_empty() {
                let endpoint_pair_sources =
                    materialized_endpoint_pair_constraint_indices_for_owned_edge(
                        edge_key.start,
                        edge_key.end,
                        rail_constraints,
                        edge_ref.owner,
                        opposite_owner,
                    );
                if !endpoint_pair_sources.is_empty() {
                    let Some(materialized_kind) =
                        material_contact_kind_for_owned_edge(edge_ref.owner, opposite_owner)
                    else {
                        continue;
                    };
                    let start_xz = road_point_from_key(edge_key.start);
                    let end_xz = road_point_from_key(edge_key.end);
                    for constraint_index in endpoint_pair_sources {
                        push_materialized_endpoint_pair_region_seam_constraint(
                            &mut additions[edge_ref.region_index],
                            constraint_index,
                            materialized_kind,
                            region.owner,
                            opposite_owner,
                            start_xz,
                            end_xz,
                        );
                    }
                    continue;
                }
                if let Some((constraint_index, materialized_kind)) =
                    materialized_source_constraint_for_owned_step_edge(
                        edge_key.start,
                        edge_key.end,
                        rail_constraints,
                        edge_ref.owner,
                        opposite_owner,
                        piece_kind,
                    )
                {
                    push_materialized_endpoint_pair_region_seam_constraint(
                        &mut additions[edge_ref.region_index],
                        constraint_index,
                        materialized_kind,
                        region.owner,
                        opposite_owner,
                        road_point_from_key(edge_key.start),
                        road_point_from_key(edge_key.end),
                    );
                }
            }
            let has_exact_owner_pair_source = matching_constraints.iter().any(|constraint| {
                rail_constraint_owner_pair_matches_edge(constraint, edge_ref.owner, opposite_owner)
            });
            for constraint in matching_constraints {
                if has_exact_owner_pair_source
                    && !rail_constraint_owner_pair_matches_edge(
                        constraint,
                        edge_ref.owner,
                        opposite_owner,
                    )
                {
                    continue;
                }
                push_materialized_region_seam_constraint(
                    &mut additions[edge_ref.region_index],
                    constraint,
                    region.owner,
                    opposite_owner,
                    road_point_from_key(edge_key.start),
                    road_point_from_key(edge_key.end),
                );
            }
        }
    }
    for (region, mut seam_additions) in regions.iter_mut().zip(additions) {
        region.seam_constraints.append(&mut seam_additions);
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }
}

fn rail_constraint_owner_pair_matches_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(left), Some(right))
            if (left == owner && right == opposite_owner)
                || (left == opposite_owner && right == owner)
    )
}

fn rail_constraint_can_materialize_for_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        || rail_constraint_owner_kinds_authorize_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_band_contour_authorizes_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_role_matches_owned_edge(constraint, owner, opposite_owner)
}

fn rail_constraint_band_contour_authorizes_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    let NodeRailConstraintKind::BandContour { kind } = constraint.kind else {
        return false;
    };
    if material_contact_kind_for_owned_edge(owner, opposite_owner).is_none() {
        return false;
    }
    if kind != owner.kind() && kind != opposite_owner.kind() {
        return false;
    }
    constraint.owner.is_none_or(|constraint_owner| {
        constraint_owner == owner || constraint_owner == opposite_owner
    })
}

fn rail_constraint_owner_kinds_authorize_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if !constraint_is_material_transition(constraint) {
        return false;
    }
    let Some((constraint_owner, constraint_opposite_owner)) =
        constraint.owner.zip(constraint.opposite_owner)
    else {
        return false;
    };
    if ![constraint_owner, constraint_opposite_owner]
        .into_iter()
        .any(|constraint_owner| constraint_owner == owner || constraint_owner == opposite_owner)
    {
        return false;
    }
    owner_sets_match_by_kind(
        owner,
        opposite_owner,
        constraint_owner,
        constraint_opposite_owner,
    )
}

fn owner_sets_match_by_kind(
    left_owner: NodeBandOwner,
    left_opposite_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    right_opposite_owner: NodeBandOwner,
) -> bool {
    (left_owner.kind() == right_owner.kind()
        && left_opposite_owner.kind() == right_opposite_owner.kind())
        || (left_owner.kind() == right_opposite_owner.kind()
            && left_opposite_owner.kind() == right_owner.kind())
}

fn rail_constraint_role_matches_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if constraint.owner.zip(constraint.opposite_owner).is_some() {
        return false;
    }
    let Some(role_owner) = constraint.owner.or(constraint.opposite_owner) else {
        return false;
    };
    if role_owner != owner && role_owner != opposite_owner {
        return false;
    }
    match constraint.kind {
        NodeRailConstraintKind::RaisedStepContact => {
            owners_form_raised_step_contact(owner, opposite_owner)
        }
        _ => false,
    }
}

fn materialized_constraint_owner_pair(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    if rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner) {
        (constraint.owner, constraint.opposite_owner)
    } else {
        (Some(owner), Some(opposite_owner))
    }
}

fn materialized_constraint_kind_for_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> NodeRailConstraintKind {
    if rail_constraint_band_contour_authorizes_owned_edge(constraint, owner, opposite_owner) {
        return material_contact_kind_for_owned_edge(owner, opposite_owner)
            .expect("band contour authorization requires a material contact kind");
    }
    constraint.kind
}

fn material_contact_kind_for_owned_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    owners_form_raised_step_contact(owner, opposite_owner)
        .then_some(NodeRailConstraintKind::RaisedStepContact)
}

fn seam_source_from_materialized_constraint_kind(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    _opposite_owner: NodeBandOwner,
) -> NodeSeamSource {
    match kind {
        NodeRailConstraintKind::RaisedStepContact => NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::AsphaltBoundary { .. } => NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::FootprintSeam { .. }
        | NodeRailConstraintKind::FullRoadbedContour => NodeSeamSource::FootprintBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::BandBoundary { .. } => NodeSeamSource::for_owner(owner),
    }
}

fn materialized_constraint_kind_constrains_shared_height(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    match kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. } => true,
        NodeRailConstraintKind::RaisedStepContact => {
            raised_step_contact_constrains_shared_height(owner, opposite_owner)
        }
        _ => false,
    }
}

fn materialized_constraint_constrains_shared_height(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    let kind = materialized_constraint_kind_for_owned_edge(constraint, owner, opposite_owner);
    if kind == NodeRailConstraintKind::RaisedStepContact
        && !rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
    {
        return false;
    }
    materialized_constraint_kind_constrains_shared_height(kind, owner, opposite_owner)
}

fn materialized_constraint_kind_is_material_transition(kind: NodeRailConstraintKind) -> bool {
    match kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::RaisedStepContact
        | NodeRailConstraintKind::BandBoundary { .. } => true,
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            adjacent_kind != RoadSurfaceBandKind::Carriageway
        }
        _ => false,
    }
}

fn owned_edge_lies_on_rail_constraint(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> bool {
    if start == end || constraint.points_xz.len() < 2 {
        return false;
    }
    if edge_lies_on_single_constraint_segment(start, end, constraint) {
        return true;
    }
    if matches!(constraint.kind, NodeRailConstraintKind::BandContour { .. }) {
        return false;
    }
    let exact_owner_pair =
        rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner);
    if materialized_edge_requires_exact_constraint_span(constraint, owner, opposite_owner) {
        if exact_owner_pair && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN {
            return edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
        }
        if !exact_owner_pair
            || (constraint.source_boundary_index.is_some()
                && piece_kind != RoadSurfaceVisualNodePieceKind::Terminal)
        {
            return false;
        }
    }
    if constraint.kind == NodeRailConstraintKind::RaisedStepContact
        && exact_owner_pair
        && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
    {
        return edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
    }
    matches!(
        piece_kind,
        RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::Terminal
    ) && edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint)
}

fn materialized_endpoint_pair_constraint_indices_for_owned_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    let Some(kind) = material_contact_kind_for_owned_edge(owner, opposite_owner) else {
        return Vec::new();
    };
    let Some(start_constraint_index) = exact_owner_pair_point_contact_constraint_index_at_key(
        start,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
    ) else {
        return Vec::new();
    };
    let Some(end_constraint_index) = exact_owner_pair_point_contact_constraint_index_at_key(
        end,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
    ) else {
        return Vec::new();
    };
    canonical_source_indices([start_constraint_index, end_constraint_index])
}

fn materialized_source_constraint_for_owned_step_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Option<(usize, NodeRailConstraintKind)> {
    let kind = material_contact_kind_for_owned_edge(owner, opposite_owner)?;
    rail_constraints
        .iter()
        .filter(|constraint| {
            constraint_applies_to_owner(constraint, owner)
                || constraint_applies_to_owner(constraint, opposite_owner)
        })
        .filter(|constraint| {
            owned_edge_lies_on_rail_constraint(
                start,
                end,
                constraint,
                owner,
                opposite_owner,
                piece_kind,
            )
        })
        .min_by_key(|constraint| {
            (
                constraint_is_material_transition(constraint),
                constraint.constraint_index,
            )
        })
        .map(|constraint| (constraint.constraint_index, kind))
}

fn exact_owner_pair_point_contact_constraint_index_at_key(
    key: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    kind: NodeRailConstraintKind,
) -> Option<usize> {
    rail_constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| {
            rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        })
        .filter(|constraint| constraint_is_point_contact(constraint))
        .filter(|constraint| {
            constraint
                .points_xz
                .first()
                .copied()
                .map(ownership_key_from_road_point)
                == Some(key)
        })
        .map(|constraint| constraint.constraint_index)
        .min()
}

fn materialized_edge_requires_exact_constraint_span(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(constraint.kind, NodeRailConstraintKind::RaisedStepContact)
        && raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
}

pub(super) fn owned_source_constraints_for_edge<'a>(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraints: &'a [NodeRegionSeamConstraint],
) -> Vec<&'a NodeRegionSeamConstraint> {
    let mut matches = constraints
        .iter()
        .filter(|constraint| {
            let constraint_start = ownership_key_from_road_point(constraint.start_xz);
            let constraint_end = ownership_key_from_road_point(constraint.end_xz);
            point_key_lies_on_segment(start, constraint_start, constraint_end)
                && point_key_lies_on_segment(end, constraint_start, constraint_end)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| (constraint.priority_key(), constraint.constraint_index));
    matches.dedup_by_key(|constraint| constraint.constraint_index);
    matches
}

pub(super) fn owned_boundary_requires_explicit_seam(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    owner.kind() != opposite_owner.kind()
}

pub(super) fn junctionn_unmaterialized_raised_step_authority_indices_for_edge(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    if piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN {
        return Vec::new();
    }
    let mut source_constraint_indices = rail_constraints
        .iter()
        .filter(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        .filter(|constraint| !constraint_is_point_contact(constraint))
        .filter(|constraint| {
            rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        })
        .filter(|constraint| {
            edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint)
        })
        .map(|constraint| constraint.constraint_index)
        .collect::<Vec<_>>();
    source_constraint_indices.sort_unstable();
    source_constraint_indices.dedup();
    source_constraint_indices
}

pub(super) fn source_constraints_materialize_raised_step_authority(
    source_constraints: &[&NodeRegionSeamConstraint],
    source_constraint_indices: &[usize],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    source_constraints.iter().any(|constraint| {
        source_constraint_indices.contains(&constraint.constraint_index)
            && constraint.is_material_transition
            && seam_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
            && matches!(
                constraint.seam_source,
                NodeSeamSource::RaisedStepContact { .. }
            )
    })
}

fn seam_constraint_owner_pair_matches_edge(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(left), Some(right))
            if (left == owner && right == opposite_owner)
                || (left == opposite_owner && right == owner)
    )
}

pub(super) fn canonicalize_seam_constraints(seams: &mut Vec<NodeRegionSeamConstraint>) {
    seams.sort_by(|a, b| seam_constraint_sort_key(a).cmp(&seam_constraint_sort_key(b)));
    seams.dedup_by(|a, b| seam_constraint_sort_key(a) == seam_constraint_sort_key(b));
}

fn seam_constraint_sort_key(
    constraint: &NodeRegionSeamConstraint,
) -> (
    usize,
    NodeOwnershipPointKey,
    NodeOwnershipPointKey,
    Option<NodeBandOwner>,
    Option<NodeBandOwner>,
) {
    (
        constraint.constraint_index,
        ownership_key_from_road_point(constraint.start_xz),
        ownership_key_from_road_point(constraint.end_xz),
        constraint.owner,
        constraint.opposite_owner,
    )
}

fn constraint_constrains_shared_height(constraint: &NodeRailConstraint) -> bool {
    if constraint_is_point_contact(constraint) {
        return false;
    }
    match constraint.kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. } => true,
        NodeRailConstraintKind::RaisedStepContact => {
            let Some((owner, opposite_owner)) = constraint.owner.zip(constraint.opposite_owner)
            else {
                return false;
            };
            raised_step_contact_constrains_shared_height(owner, opposite_owner)
        }
        _ => false,
    }
}

fn constraint_is_point_contact(constraint: &NodeRailConstraint) -> bool {
    let Some(first) = constraint
        .points_xz
        .first()
        .copied()
        .map(ownership_key_from_road_point)
    else {
        return false;
    };
    constraint
        .points_xz
        .iter()
        .copied()
        .map(ownership_key_from_road_point)
        .all(|point| point == first)
}

fn constraint_is_material_transition(constraint: &NodeRailConstraint) -> bool {
    match constraint.kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::RaisedStepContact
        | NodeRailConstraintKind::BandBoundary { .. } => true,
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            adjacent_kind != RoadSurfaceBandKind::Carriageway
        }
        _ => false,
    }
}

fn constraint_applies_to_owner(constraint: &NodeRailConstraint, owner: NodeBandOwner) -> bool {
    if constraint.owner.is_some() || constraint.opposite_owner.is_some() {
        return constraint.owner == Some(owner) || constraint.opposite_owner == Some(owner);
    }
    match constraint.kind {
        NodeRailConstraintKind::FullRoadbedContour => true,
        NodeRailConstraintKind::BandContour { kind }
        | NodeRailConstraintKind::SpanHandoff { kind }
        | NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: kind,
        } => kind == owner.kind(),
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            owner.kind() == RoadSurfaceBandKind::Carriageway || adjacent_kind == owner.kind()
        }
        NodeRailConstraintKind::RaisedStepContact => false,
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => left_kind == owner.kind() || right_kind == owner.kind(),
    }
}

fn edge_lies_on_constraint(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
) -> bool {
    if constraint.points_xz.len() < 2 {
        return false;
    }
    let edge_start = ownership_key_from_overlay_point(edge_start);
    let edge_end = ownership_key_from_overlay_point(edge_end);
    constraint.points_xz.windows(2).any(|segment| {
        let start = ownership_key_from_road_point(segment[0]);
        let end = ownership_key_from_road_point(segment[1]);
        point_key_lies_on_segment(edge_start, start, end)
            && point_key_lies_on_segment(edge_end, start, end)
    }) || edge_lies_on_constraint_polyline(edge_start, edge_end, constraint)
        || edge_endpoints_lie_on_constraint_path(edge_start, edge_end, constraint)
}

fn shape_edge_carries_full_seam_constraint(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
) -> bool {
    if !shape_edge_requires_exact_constraint_span(constraint) {
        return edge_lies_on_constraint(edge_start, edge_end, constraint);
    }
    edge_lies_on_single_constraint_segment(
        ownership_key_from_overlay_point(edge_start),
        ownership_key_from_overlay_point(edge_end),
        constraint,
    )
}

fn shape_edge_requires_exact_constraint_span(constraint: &NodeRailConstraint) -> bool {
    matches!(constraint.kind, NodeRailConstraintKind::RaisedStepContact)
}

fn edge_lies_on_single_constraint_segment(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    constraint.points_xz.windows(2).any(|segment| {
        let start = ownership_key_from_road_point(segment[0]);
        let end = ownership_key_from_road_point(segment[1]);
        point_key_lies_on_segment(edge_start, start, end)
            && point_key_lies_on_segment(edge_end, start, end)
    })
}

fn edge_lies_on_constraint_polyline(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    edge_lies_on_constraint_polyline_with_collinearity(
        edge_start,
        edge_end,
        constraint,
        point_key_collinear_with_edge,
    )
}

fn edge_lies_on_constraint_polyline_on_overlay_grid(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    edge_lies_on_constraint_polyline_with_collinearity(
        edge_start,
        edge_end,
        constraint,
        point_key_collinear_with_edge_on_overlay_grid,
    )
}

fn edge_lies_on_constraint_polyline_with_collinearity(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    point_collinear_with_edge: fn(
        NodeOwnershipPointKey,
        NodeOwnershipPointKey,
        NodeOwnershipPointKey,
    ) -> bool,
) -> bool {
    if edge_start == edge_end || constraint.points_xz.len() < 2 {
        return false;
    }
    let edge_end_parameter = segment_parameter_key(edge_start, edge_end, edge_end);
    if edge_end_parameter <= 0 {
        return false;
    }
    let mut intervals = Vec::new();
    for segment in constraint.points_xz.windows(2) {
        let start = ownership_key_from_road_point(segment[0]);
        let end = ownership_key_from_road_point(segment[1]);
        if start == end
            || !point_collinear_with_edge(start, edge_start, edge_end)
            || !point_collinear_with_edge(end, edge_start, edge_end)
        {
            continue;
        }
        let start_parameter = segment_parameter_key(edge_start, edge_end, start);
        let end_parameter = segment_parameter_key(edge_start, edge_end, end);
        let overlap_start = start_parameter.min(end_parameter).max(0);
        let overlap_end = start_parameter.max(end_parameter).min(edge_end_parameter);
        if overlap_start < overlap_end {
            intervals.push((overlap_start, overlap_end));
        }
    }
    if intervals.is_empty() {
        return false;
    }
    intervals.sort_unstable();
    let mut covered_end = 0;
    for (start, end) in intervals {
        if start > covered_end {
            return false;
        }
        covered_end = covered_end.max(end);
        if covered_end >= edge_end_parameter {
            return true;
        }
    }
    false
}

fn edge_endpoints_lie_on_constraint_path(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    if edge_start == edge_end
        || constraint.points_xz.len() < 2
        || !constraint_allows_path_chord(constraint)
    {
        return false;
    }
    constraint_path_contains_ordered_endpoints(edge_start, edge_end, constraint)
        || constraint_path_contains_ordered_endpoints(edge_end, edge_start, constraint)
}

fn constraint_path_contains_ordered_endpoints(
    first: NodeOwnershipPointKey,
    second: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    let mut first_seen = false;
    for segment in constraint.points_xz.windows(2) {
        let start = ownership_key_from_road_point(segment[0]);
        let end = ownership_key_from_road_point(segment[1]);
        if point_key_lies_on_segment(first, start, end) {
            first_seen = true;
        }
        if first_seen && point_key_lies_on_segment(second, start, end) {
            return true;
        }
    }
    false
}

fn constraint_allows_path_chord(constraint: &NodeRailConstraint) -> bool {
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::RaisedStepContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
}

fn point_lies_on_point_constraint(
    point: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
) -> bool {
    if constraint.points_xz.len() < 2 {
        return false;
    }
    let point = ownership_key_from_overlay_point(point);
    constraint.points_xz.windows(2).any(|segment| {
        let start = ownership_key_from_road_point(segment[0]);
        let end = ownership_key_from_road_point(segment[1]);
        start == end && point == start
    })
}

fn seam_source_from_constraint(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> NodeSeamSource {
    match constraint.kind {
        NodeRailConstraintKind::RaisedStepContact => NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::AsphaltBoundary { .. } => NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::FootprintSeam { .. }
        | NodeRailConstraintKind::FullRoadbedContour => NodeSeamSource::FootprintBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::BandBoundary { .. } => NodeSeamSource::for_owner(owner),
    }
}
