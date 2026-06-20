//! Shared-height repair for generated raised-step contact carriers.

use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
struct SharedHeightCandidate {
    contour_index: usize,
    vertex_index: usize,
    owner: NodeBandOwner,
    height_mm: i64,
    mutable: bool,
}

pub(in crate::simulation::network::surface::node::rails) fn synchronize_shared_height_contact_vertices(
    contours: &mut [NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) {
    for constraint in constraints
        .iter()
        .filter(|constraint| shared_height_constraint_owners(constraint).is_some())
    {
        synchronize_shared_height_constraint(contours, constraint);
    }
}

fn synchronize_shared_height_constraint(
    contours: &mut [NodeGeneratedContour],
    constraint: &NodeRailConstraint,
) {
    let Some((lower_owner, raised_owner)) = shared_height_constraint_owners(constraint) else {
        return;
    };
    let mut candidates_by_key = BTreeMap::<NodeRailPointKey, Vec<SharedHeightCandidate>>::new();
    for (contour_index, contour) in contours.iter().enumerate() {
        collect_shared_height_candidates(
            contour_index,
            contour,
            constraint,
            &mut candidates_by_key,
        );
    }

    for candidates in candidates_by_key.values() {
        if !candidates_cover_contact_owners(candidates, lower_owner, raised_owner)
            || !candidates.iter().any(|candidate| candidate.mutable)
        {
            continue;
        }
        let Some(raised_height_mm) = single_owner_height_mm(candidates, raised_owner) else {
            continue;
        };
        apply_shared_height_to_mutable_candidates(contours, candidates, raised_height_mm);
    }
}

fn collect_shared_height_candidates(
    contour_index: usize,
    contour: &NodeGeneratedContour,
    constraint: &NodeRailConstraint,
    candidates_by_key: &mut BTreeMap<NodeRailPointKey, Vec<SharedHeightCandidate>>,
) {
    let Some(owner) = contour.owner else {
        return;
    };
    if constraint.owner != Some(owner) && constraint.opposite_owner != Some(owner) {
        return;
    }
    if generated_contour_band_kind(contour) != Some(owner.kind()) {
        return;
    }
    let Some(height_points) = contour.height_points_world.as_ref() else {
        return;
    };
    if height_points.len() != contour.points_xz.len() {
        return;
    }
    let mutable = generated_contour_mutably_constrains_shared_height(contour);
    for (vertex_index, point_xz) in contour.points_xz.iter().copied().enumerate() {
        let key = road_point_key(point_xz);
        if !generated_constraint_touches_key(constraint, key) {
            continue;
        }
        candidates_by_key
            .entry(key)
            .or_default()
            .push(SharedHeightCandidate {
                contour_index,
                vertex_index,
                owner,
                height_mm: SurfaceHeightMmKey::from_m_f64(height_points[vertex_index].y).as_i64(),
                mutable,
            });
    }
}

fn shared_height_constraint_owners(
    constraint: &NodeRailConstraint,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    if constraint.kind != NodeRailConstraintKind::RaisedStepContact {
        return None;
    }
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if !raised_step_kinds_can_contact(owner.kind(), opposite_owner.kind())
        || raised_step_requires_exact_constraint_span(owner.kind(), opposite_owner.kind())
    {
        return None;
    }
    let owner_rank = raised_step_band_rank(owner.kind())?;
    let opposite_rank = raised_step_band_rank(opposite_owner.kind())?;
    if owner_rank < opposite_rank {
        Some((owner, opposite_owner))
    } else if opposite_rank < owner_rank {
        Some((opposite_owner, owner))
    } else {
        None
    }
}

fn generated_contour_mutably_constrains_shared_height(contour: &NodeGeneratedContour) -> bool {
    matches!(
        contour.purpose,
        NodeGeneratedContourPurpose::TerminalCap
            | NodeGeneratedContourPurpose::BendSideJoin
            | NodeGeneratedContourPurpose::JunctionSideJoin
    )
}

fn candidates_cover_contact_owners(
    candidates: &[SharedHeightCandidate],
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.owner == lower_owner)
        && candidates
            .iter()
            .any(|candidate| candidate.owner == raised_owner)
}

fn single_owner_height_mm(
    candidates: &[SharedHeightCandidate],
    owner: NodeBandOwner,
) -> Option<i64> {
    let mut height_mm = None;
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.owner == owner)
    {
        if let Some(existing_height_mm) = height_mm
            && existing_height_mm != candidate.height_mm
        {
            return None;
        }
        height_mm = Some(candidate.height_mm);
    }
    height_mm
}

fn apply_shared_height_to_mutable_candidates(
    contours: &mut [NodeGeneratedContour],
    candidates: &[SharedHeightCandidate],
    height_mm: i64,
) {
    let height_m = height_mm as f64 / SURFACE_MM_PER_M;
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.mutable && candidate.height_mm != height_mm)
    {
        let Some(height_points) = contours
            .get_mut(candidate.contour_index)
            .and_then(|contour| contour.height_points_world.as_mut())
        else {
            continue;
        };
        if let Some(point) = height_points.get_mut(candidate.vertex_index) {
            point.y = height_m;
        }
    }
}
