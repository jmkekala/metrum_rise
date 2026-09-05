// SPDX-License-Identifier: GPL-2.0-only

//! Shared-height repair for generated raised-step contact carriers.

use super::*;

#[derive(Clone, Copy, Debug)]
struct SharedHeightCandidate {
    key: NodeRailPointKey,
    contour_index: usize,
    vertex_index: usize,
    owner: NodeBandOwner,
    height_mm: i64,
    mutable: bool,
}

struct PreparedSharedHeightConstraint<'a> {
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    point_keys: &'a [NodeRailPointKey],
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

impl PreparedSharedHeightConstraint<'_> {
    fn applies_to_owner(&self, owner: NodeBandOwner) -> bool {
        owner == self.lower_owner || owner == self.raised_owner
    }

    fn touches_key(&self, key: NodeRailPointKey) -> bool {
        self.point_keys
            .windows(2)
            .any(|segment| generated_point_key_lies_on_segment(key, segment[0], segment[1]))
    }
}

pub(in crate::simulation::network::surface::node::rails) fn synchronize_shared_height_contact_vertices(
    contours: &mut [NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) {
    let mut candidates = collect_shared_height_candidates(contours);
    candidates.sort_unstable_by_key(|candidate| {
        (
            candidate.key,
            candidate.contour_index,
            candidate.vertex_index,
        )
    });
    let mut point_keys = Vec::with_capacity(
        constraints
            .iter()
            .map(|constraint| constraint.points_xz.len())
            .max()
            .unwrap_or(0),
    );
    for constraint in constraints {
        let Some((lower_owner, raised_owner)) = shared_height_constraint_owners(constraint) else {
            continue;
        };
        point_keys.clear();
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for point in constraint.points_xz.iter().copied() {
            let key = road_point_key(point);
            min_x = min_x.min(key.0);
            min_z = min_z.min(key.1);
            max_x = max_x.max(key.0);
            max_z = max_z.max(key.1);
            point_keys.push(key);
        }
        if point_keys.len() < 2 {
            continue;
        }
        synchronize_shared_height_constraint(
            contours,
            &PreparedSharedHeightConstraint {
                lower_owner,
                raised_owner,
                point_keys: &point_keys,
                min_x,
                min_z,
                max_x,
                max_z,
            },
            &mut candidates,
        );
    }
}

fn synchronize_shared_height_constraint(
    contours: &mut [NodeGeneratedContour],
    constraint: &PreparedSharedHeightConstraint<'_>,
    candidates: &mut [SharedHeightCandidate],
) {
    // Axis-aligned source segments admit at most two canonical key units of
    // overlay-grid drift; non-axis segment membership remains bounds-exact.
    let min_x = constraint.min_x.saturating_sub(2);
    let min_z = constraint.min_z.saturating_sub(2);
    let max_x = constraint.max_x.saturating_add(2);
    let max_z = constraint.max_z.saturating_add(2);
    let candidate_first = candidates.partition_point(|candidate| candidate.key.0 < min_x);
    let candidate_last = candidates.partition_point(|candidate| candidate.key.0 <= max_x);
    let candidates = &mut candidates[candidate_first..candidate_last];
    let mut first = 0;
    while first < candidates.len() {
        let key = candidates[first].key;
        let mut last = first + 1;
        while last < candidates.len() && candidates[last].key == key {
            last += 1;
        }
        let group = &mut candidates[first..last];
        first = last;
        if key.1 < min_z
            || max_z < key.1
            || !constraint.touches_key(key)
            || !candidates_cover_contact_owners(
                group,
                constraint.lower_owner,
                constraint.raised_owner,
            )
            || !group
                .iter()
                .any(|candidate| constraint.applies_to_owner(candidate.owner) && candidate.mutable)
        {
            continue;
        }
        let Some(raised_height_mm) = single_owner_height_mm(group, constraint.raised_owner) else {
            continue;
        };
        apply_shared_height_to_mutable_candidates(contours, group, constraint, raised_height_mm);
    }
}

fn collect_shared_height_candidates(
    contours: &[NodeGeneratedContour],
) -> Vec<SharedHeightCandidate> {
    let mut candidates =
        Vec::with_capacity(contours.iter().map(|contour| contour.points_xz.len()).sum());
    for (contour_index, contour) in contours.iter().enumerate() {
        let Some(owner) = contour.owner else {
            continue;
        };
        if generated_contour_band_kind(contour) != Some(owner.kind()) {
            continue;
        }
        let Some(height_points) = contour.height_points_world.as_ref() else {
            continue;
        };
        if height_points.len() != contour.points_xz.len() {
            continue;
        }
        let mutable = generated_contour_mutably_constrains_shared_height(contour);
        for (vertex_index, point_xz) in contour.points_xz.iter().copied().enumerate() {
            candidates.push(SharedHeightCandidate {
                key: road_point_key(point_xz),
                contour_index,
                vertex_index,
                owner,
                height_mm: SurfaceHeightMmKey::from_m_f64(height_points[vertex_index].y).as_i64(),
                mutable,
            });
        }
    }
    candidates
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
    candidates: &mut [SharedHeightCandidate],
    constraint: &PreparedSharedHeightConstraint<'_>,
    height_mm: i64,
) {
    let height_m = height_mm as f64 / SURFACE_MM_PER_M;
    for candidate in candidates.iter_mut().filter(|candidate| {
        constraint.applies_to_owner(candidate.owner)
            && candidate.mutable
            && candidate.height_mm != height_mm
    }) {
        let Some(height_points) = contours
            .get_mut(candidate.contour_index)
            .and_then(|contour| contour.height_points_world.as_mut())
        else {
            continue;
        };
        if let Some(point) = height_points.get_mut(candidate.vertex_index) {
            point.y = height_m;
            candidate.height_mm = height_mm;
        }
    }
}
