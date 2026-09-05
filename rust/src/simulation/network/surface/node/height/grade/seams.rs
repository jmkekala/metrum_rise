// SPDX-License-Identifier: GPL-2.0-only

//! Explicit same-material and material-transition seam height normalization.

use super::*;
use crate::simulation::network::surface::band_semantics::{
    raised_step_band_rank, raised_step_kinds_can_contact,
    raised_step_requires_exact_constraint_span,
};
use std::collections::BTreeMap;

const EXPLICIT_MATERIAL_SEAM_HEIGHT_DUST_MM: i64 = 1;

pub(super) fn apply_junctionn_same_material_seam_height_normalization(
    regions: &mut [NodeHeightedRegion],
    constraint_indices: &[NodeGradeRegionConstraintIndex],
) -> Result<(), NodeHeightFieldError> {
    let mut candidates_by_key =
        BTreeMap::<NodeGradeExplicitSeamHeightKey, Vec<SameMaterialVertexHeightCandidate>>::new();

    for (region_index, region) in regions.iter().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in indexed_material_height_constraints_for_vertex(
                vertex.point_xz,
                &region.seam_constraints,
                constraint_index,
            ) {
                if constraint.is_material_transition {
                    continue;
                }
                let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
                let key = NodeGradeExplicitSeamHeightKey::new(point, constraint);
                let candidate = SameMaterialVertexHeightCandidate {
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                    height_m: vertex.height_m,
                    height_authority: vertex.height_authority,
                    source_provenance: vertex.source_provenance,
                    has_explicit_shared_material_seam: false,
                };
                push_unique_same_material_candidate(&mut candidates_by_key, key, candidate);
            }
        }
    }

    for (key, candidates) in &candidates_by_key {
        let Some(first) = candidates.first().copied() else {
            continue;
        };
        reject_same_material_height_conflict(
            first.owner.kind(),
            key.point,
            candidates.iter().copied(),
        )?;
    }

    let selected_by_key = candidates_by_key
        .into_iter()
        .filter_map(|(key, candidates)| {
            same_height_selected_candidate(&candidates).map(|selected| (key, selected))
        })
        .collect::<BTreeMap<_, _>>();

    if selected_by_key.is_empty() {
        return Ok(());
    }

    for (region_index, region) in regions.iter_mut().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            for constraint in indexed_material_height_constraints_for_vertex(
                vertex.point_xz,
                &region.seam_constraints,
                constraint_index,
            ) {
                if constraint.is_material_transition {
                    continue;
                }
                let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
                let key = NodeGradeExplicitSeamHeightKey::new(point, constraint);
                if let Some(selected) = selected_by_key.get(&key) {
                    set_vertex_grade_height(
                        owner,
                        vertex,
                        selected.height_m,
                        NodeGradeCarrierDecision::SameMaterialSeam,
                    );
                    break;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn apply_junctionn_explicit_material_seam_height_normalization(
    regions: &mut [NodeHeightedRegion],
    constraint_indices: &[NodeGradeRegionConstraintIndex],
    normalize_same_xz_shared_height_raised_steps: bool,
) {
    apply_fragmented_shared_height_raised_step_seam_normalization(regions, constraint_indices);
    if normalize_same_xz_shared_height_raised_steps {
        apply_same_xz_shared_height_raised_step_boundary_vertex_normalization(
            regions,
            constraint_indices,
        );
    }

    let mut candidates_by_key =
        BTreeMap::<NodeGradeExplicitSeamHeightKey, Vec<SameMaterialVertexHeightCandidate>>::new();

    for (region_index, region) in regions.iter().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in indexed_material_height_constraints_for_vertex(
                vertex.point_xz,
                &region.seam_constraints,
                constraint_index,
            ) {
                if !constraint.is_material_transition {
                    continue;
                }
                let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
                let key = NodeGradeExplicitSeamHeightKey::new(point, constraint);
                let candidate = SameMaterialVertexHeightCandidate {
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                    height_m: vertex.height_m,
                    height_authority: vertex.height_authority,
                    source_provenance: vertex.source_provenance,
                    has_explicit_shared_material_seam: true,
                };
                push_unique_same_material_candidate(&mut candidates_by_key, key, candidate);
            }
        }
    }

    let selected_by_key = candidates_by_key
        .into_iter()
        .filter_map(|(key, candidates)| {
            selected_explicit_material_seam_height(&key, &candidates)
                .map(|height_m| (key, height_m))
        })
        .collect::<BTreeMap<_, _>>();

    if selected_by_key.is_empty() {
        return;
    }

    for (region_index, region) in regions.iter_mut().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            for constraint in indexed_material_height_constraints_for_vertex(
                vertex.point_xz,
                &region.seam_constraints,
                constraint_index,
            ) {
                if !constraint.is_material_transition {
                    continue;
                }
                let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
                let key = NodeGradeExplicitSeamHeightKey::new(point, constraint);
                if let Some(height_m) = selected_by_key.get(&key) {
                    set_vertex_grade_height(
                        owner,
                        vertex,
                        *height_m,
                        NodeGradeCarrierDecision::ExplicitMaterialSeam,
                    );
                    break;
                }
            }
        }
    }
}

fn apply_same_xz_shared_height_raised_step_boundary_vertex_normalization(
    regions: &mut [NodeHeightedRegion],
    constraint_indices: &[NodeGradeRegionConstraintIndex],
) {
    let mut candidates_by_point =
        BTreeMap::<SurfaceXzKey, Vec<SameMaterialVertexHeightCandidate>>::new();

    for (region_index, region) in regions.iter().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
            let candidate = SameMaterialVertexHeightCandidate {
                owner: region.owner,
                height_field_id: vertex.height_field_id,
                height_m: vertex.height_m,
                height_authority: vertex.height_authority,
                source_provenance: vertex.source_provenance,
                has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                    vertex,
                    &region.seam_constraints,
                    constraint_index,
                ),
            };
            push_unique_same_material_candidate(&mut candidates_by_point, point, candidate);
        }
    }

    let mut selected_heights_by_context =
        BTreeMap::<(SurfaceXzKey, NodeBandOwner), (Vec<f64>, bool)>::new();
    for (point, candidates) in candidates_by_point {
        let mut owners = candidates
            .iter()
            .map(|candidate| candidate.owner)
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();

        for (index, owner) in owners.iter().copied().enumerate() {
            for opposite_owner in owners.iter().copied().skip(index + 1) {
                if !shared_height_raised_step_pair(owner, opposite_owner) {
                    continue;
                }
                let pair_has_explicit_shared_material_seam = candidates.iter().any(|candidate| {
                    candidate.has_explicit_shared_material_seam
                        && (candidate.owner == owner || candidate.owner == opposite_owner)
                });
                let pair_candidates = candidates
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        (candidate.owner == owner || candidate.owner == opposite_owner)
                            && (!pair_has_explicit_shared_material_seam
                                || candidate.has_explicit_shared_material_seam)
                    })
                    .collect::<Vec<_>>();
                let mut height_keys = pair_candidates
                    .iter()
                    .map(|candidate| SurfaceHeightMmKey::from_m_f64(candidate.height_m))
                    .collect::<Vec<_>>();
                height_keys.sort_unstable();
                height_keys.dedup();
                if height_keys.len() < 2 {
                    continue;
                }
                let Some(selected) = selected_shared_height_raised_step_height_for_pair(
                    owner,
                    opposite_owner,
                    &pair_candidates,
                ) else {
                    continue;
                };
                for selected_owner in [owner, opposite_owner] {
                    let (heights, allows_unseamed_vertex) = selected_heights_by_context
                        .entry((point, selected_owner))
                        .or_default();
                    heights.push(selected.height_m);
                    *allows_unseamed_vertex |= !pair_has_explicit_shared_material_seam;
                }
            }
        }
    }

    let selected_by_context = selected_heights_by_context
        .into_iter()
        .filter_map(|(context, (heights, allows_unseamed_vertex))| {
            let mut height_keys = heights
                .iter()
                .map(|height_m| SurfaceHeightMmKey::from_m_f64(*height_m))
                .collect::<Vec<_>>();
            height_keys.sort_unstable();
            height_keys.dedup();
            if height_keys.len() == 1 {
                Some((
                    context,
                    (
                        height_keys[0].as_i64() as f64 / SURFACE_MM_PER_M,
                        allows_unseamed_vertex,
                    ),
                ))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();

    if selected_by_context.is_empty() {
        return;
    }

    for (region_index, region) in regions.iter_mut().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
            if let Some((height_m, allows_unseamed_vertex)) =
                selected_by_context.get(&(point, owner))
            {
                if !*allows_unseamed_vertex
                    && !vertex_has_explicit_shared_material_seam(
                        vertex,
                        &region.seam_constraints,
                        constraint_index,
                    )
                {
                    continue;
                }
                set_vertex_grade_height(
                    owner,
                    vertex,
                    *height_m,
                    NodeGradeCarrierDecision::ExplicitMaterialSeam,
                );
            }
        }
    }
}

fn apply_fragmented_shared_height_raised_step_seam_normalization(
    regions: &mut [NodeHeightedRegion],
    constraint_indices: &[NodeGradeRegionConstraintIndex],
) {
    let mut candidates_by_key = BTreeMap::<
        RaisedStepSharedHeightSeamPointKey,
        Vec<SameMaterialVertexHeightCandidate>,
    >::new();

    for (region_index, region) in regions.iter().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in raised_step_shared_height_constraints_for_vertex(
                vertex.point_xz,
                &region.seam_constraints,
                constraint_index,
            ) {
                let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
                let Some(key) = raised_step_shared_height_seam_point_key(point, constraint) else {
                    continue;
                };
                let candidate = SameMaterialVertexHeightCandidate {
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                    height_m: vertex.height_m,
                    height_authority: vertex.height_authority,
                    source_provenance: vertex.source_provenance,
                    has_explicit_shared_material_seam: true,
                };
                push_unique_same_material_candidate(&mut candidates_by_key, key, candidate);
            }
        }
    }

    let selected_by_key = candidates_by_key
        .into_iter()
        .filter_map(|(key, candidates)| {
            selected_shared_height_raised_step_height_for_pair(
                key.owner,
                key.opposite_owner,
                &candidates,
            )
            .map(|selected| (key, selected.height_m))
        })
        .collect::<BTreeMap<_, _>>();

    if selected_by_key.is_empty() {
        return;
    }

    for (region_index, region) in regions.iter_mut().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            for constraint in raised_step_shared_height_constraints_for_vertex(
                vertex.point_xz,
                &region.seam_constraints,
                constraint_index,
            ) {
                let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
                let Some(key) = raised_step_shared_height_seam_point_key(point, constraint) else {
                    continue;
                };
                if let Some(height_m) = selected_by_key.get(&key) {
                    set_vertex_grade_height(
                        owner,
                        vertex,
                        *height_m,
                        NodeGradeCarrierDecision::ExplicitMaterialSeam,
                    );
                    break;
                }
            }
        }
    }
}

fn raised_step_shared_height_constraints_for_vertex<'a>(
    point_xz: RoadVec2,
    constraints: &'a [NodeRegionSeamConstraint],
    constraint_index: &'a NodeGradeRegionConstraintIndex,
) -> impl Iterator<Item = &'a NodeRegionSeamConstraint> + 'a {
    let point = SurfaceXzKey::from_road_xz(point_xz);
    constraint_index
        .shared_height_raised_step_constraint_indices(point_xz)
        .iter()
        .map(|&index| &constraints[index])
        .filter(move |constraint| {
            raised_step_shared_height_seam_point_key(point, constraint).is_some()
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepSharedHeightSeamPointKey {
    point: SurfaceXzKey,
    constraint_index: usize,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
}

fn raised_step_shared_height_seam_point_key(
    point: SurfaceXzKey,
    constraint: &NodeRegionSeamConstraint,
) -> Option<RaisedStepSharedHeightSeamPointKey> {
    if !constraint.is_material_transition {
        return None;
    }
    let (Some(owner), Some(opposite_owner)) =
        canonical_explicit_seam_owner_pair(constraint.owner, constraint.opposite_owner)
    else {
        return None;
    };
    if !shared_height_raised_step_pair(owner, opposite_owner) {
        return None;
    }
    Some(RaisedStepSharedHeightSeamPointKey {
        point,
        constraint_index: constraint.constraint_index,
        owner,
        opposite_owner,
    })
}

fn selected_explicit_material_seam_height(
    key: &NodeGradeExplicitSeamHeightKey,
    candidates: &[SameMaterialVertexHeightCandidate],
) -> Option<f64> {
    same_height_selected_candidate(candidates)
        .map(|selected| selected.height_m)
        .or_else(|| {
            selected_sidewalk_footpath_tie_in_height(key, candidates)
                .map(|selected| selected.height_m)
        })
        .or_else(|| {
            selected_shared_height_raised_step_height(key, candidates)
                .map(|selected| selected.height_m)
        })
        .or_else(|| selected_dust_matched_explicit_material_seam_height(candidates))
}

fn selected_shared_height_raised_step_height(
    key: &NodeGradeExplicitSeamHeightKey,
    candidates: &[SameMaterialVertexHeightCandidate],
) -> Option<SameMaterialVertexHeightCandidate> {
    let owner = key.owner?;
    let opposite_owner = key.opposite_owner?;
    selected_shared_height_raised_step_height_for_pair(owner, opposite_owner, candidates)
}

fn selected_shared_height_raised_step_height_for_pair(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    candidates: &[SameMaterialVertexHeightCandidate],
) -> Option<SameMaterialVertexHeightCandidate> {
    if !shared_height_raised_step_pair(owner, opposite_owner) {
        return None;
    }
    let owner_rank = raised_step_band_rank(owner.kind())?;
    let opposite_rank = raised_step_band_rank(opposite_owner.kind())?;
    if owner_rank == opposite_rank {
        return None;
    }
    let (raised_owner, lower_owner) = if owner_rank > opposite_rank {
        (owner, opposite_owner)
    } else {
        (opposite_owner, owner)
    };
    if !candidates
        .iter()
        .any(|candidate| candidate.owner == raised_owner)
        || !candidates
            .iter()
            .any(|candidate| candidate.owner == lower_owner)
    {
        return None;
    }
    candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.owner == raised_owner)
        .min_by_key(|candidate| same_material_vertex_height_candidate_key(*candidate))
}

fn shared_height_raised_step_pair(owner: NodeBandOwner, opposite_owner: NodeBandOwner) -> bool {
    raised_step_kinds_can_contact(owner.kind(), opposite_owner.kind())
        && !raised_step_requires_exact_constraint_span(owner.kind(), opposite_owner.kind())
}

fn selected_dust_matched_explicit_material_seam_height(
    candidates: &[SameMaterialVertexHeightCandidate],
) -> Option<f64> {
    let mut min_height_mm = i64::MAX;
    let mut max_height_mm = i64::MIN;
    for candidate in candidates {
        let height_mm = SurfaceHeightMmKey::from_m_f64(candidate.height_m).as_i64();
        min_height_mm = min_height_mm.min(height_mm);
        max_height_mm = max_height_mm.max(height_mm);
    }
    if max_height_mm - min_height_mm > EXPLICIT_MATERIAL_SEAM_HEIGHT_DUST_MM {
        return None;
    }
    candidates
        .iter()
        .copied()
        .min_by_key(|candidate| same_material_vertex_height_candidate_key(*candidate))
        .map(|selected| selected.height_m)
}

fn selected_sidewalk_footpath_tie_in_height(
    key: &NodeGradeExplicitSeamHeightKey,
    candidates: &[SameMaterialVertexHeightCandidate],
) -> Option<SameMaterialVertexHeightCandidate> {
    let owner = key.owner?;
    let opposite_owner = key.opposite_owner?;
    if !owners_form_sidewalk_footpath_contact(owner, opposite_owner) {
        return None;
    }
    candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.owner.kind() == RoadSurfaceBandKind::Sidewalk)
        .min_by_key(|candidate| same_material_vertex_height_candidate_key(*candidate))
}

fn owners_form_sidewalk_footpath_contact(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (owner.kind(), opposite_owner.kind()),
        (RoadSurfaceBandKind::Sidewalk, RoadSurfaceBandKind::Footpath)
            | (RoadSurfaceBandKind::Footpath, RoadSurfaceBandKind::Sidewalk)
    )
}
