//! Same-material shared-vertex height authority normalization.

use super::*;
use std::collections::BTreeMap;

pub(super) fn apply_junctionn_same_owner_canonical_vertex_height_normalization(
    regions: &mut [NodeHeightedRegion],
) {
    let mut heights_by_key =
        BTreeMap::<NodeGradeVertexContextKey, SameMaterialVertexHeightCandidate>::new();
    let mut samples_by_key = BTreeMap::<NodeGradeVertexContextKey, (usize, Vec<i64>)>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let key = NodeGradeVertexContextKey {
                point: SurfaceXzKey::from_road_xz(vertex.point_xz),
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            let candidate = SameMaterialVertexHeightCandidate {
                owner: region.owner,
                height_field_id: vertex.height_field_id,
                height_m: vertex.height_m,
                height_authority: vertex.height_authority,
                has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                    vertex,
                    &region.seam_constraints,
                ),
            };
            heights_by_key
                .entry(key.clone())
                .and_modify(|selected| {
                    if same_material_vertex_height_candidate_key(candidate)
                        < same_material_vertex_height_candidate_key(*selected)
                    {
                        *selected = candidate;
                    }
                })
                .or_insert(candidate);
            let (sample_count, heights) = samples_by_key.entry(key.clone()).or_default();
            *sample_count += 1;
            let height_mm = SurfaceHeightMmKey::from_m_f64(vertex.height_m).as_i64();
            if !heights.contains(&height_mm) {
                heights.push(height_mm);
            }
        }
    }

    for region in regions {
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key = NodeGradeVertexContextKey {
                point: SurfaceXzKey::from_road_xz(vertex.point_xz),
                owner,
                height_field_id: vertex.height_field_id,
            };
            let Some((sample_count, heights)) = samples_by_key.get(&key) else {
                continue;
            };
            if *sample_count < 2 {
                continue;
            }
            if heights.len() != 1 {
                continue;
            }
            if let Some(selected) = heights_by_key.get(&key).copied() {
                set_vertex_grade_height(
                    owner,
                    vertex,
                    selected.height_m,
                    NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
                );
            }
        }
    }
}

pub(super) fn apply_junctionn_same_material_vertex_height_normalization(
    regions: &mut [NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let groups = collect_same_material_vertex_height_groups(regions);
    reject_same_material_vertex_height_group_conflicts(&groups)?;
    apply_same_material_vertex_height_groups(regions, &groups);
    Ok(())
}

fn collect_same_material_vertex_height_groups(
    regions: &[NodeHeightedRegion],
) -> SameMaterialVertexHeightGroups {
    let mut groups = SameMaterialVertexHeightGroups {
        contexts_by_key: BTreeMap::new(),
        candidates_by_key: BTreeMap::new(),
        selected_by_key: BTreeMap::new(),
    };

    for region in regions {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let key = same_material_vertex_height_support_key_from_parts(
                region.kind,
                region.owner,
                &region.seam_constraints,
                vertex,
            );
            let candidate = same_material_vertex_height_candidate_from_vertex(
                region.owner,
                vertex,
                &region.seam_constraints,
            );
            let contexts = groups.contexts_by_key.entry(key.clone()).or_default();
            let context = SameMaterialVertexHeightContext::from_candidate(candidate);
            if !contexts.contains(&context) {
                contexts.push(context);
                contexts.sort_unstable();
                groups
                    .candidates_by_key
                    .entry(key.clone())
                    .or_default()
                    .push(candidate);
            }
            groups
                .selected_by_key
                .entry(key)
                .and_modify(|selected| {
                    if same_material_vertex_height_candidate_key(candidate)
                        < same_material_vertex_height_candidate_key(*selected)
                    {
                        *selected = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }
    groups
}

fn reject_same_material_vertex_height_group_conflicts(
    groups: &SameMaterialVertexHeightGroups,
) -> Result<(), NodeHeightFieldError> {
    for (key, contexts) in &groups.contexts_by_key {
        if contexts.len() < 2 {
            continue;
        }
        if !key.explicit_seams.is_empty() {
            continue;
        }
        reject_same_material_height_conflict(
            key.kind,
            key.point,
            groups
                .candidates_by_key
                .get(key)
                .into_iter()
                .flatten()
                .copied(),
        )?;
    }
    Ok(())
}

fn apply_same_material_vertex_height_groups(
    regions: &mut [NodeHeightedRegion],
    groups: &SameMaterialVertexHeightGroups,
) {
    for region in regions {
        let owner = region.owner;
        let kind = region.kind;
        let seam_constraints = &region.seam_constraints;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key = same_material_vertex_height_support_key_from_parts(
                kind,
                owner,
                seam_constraints,
                vertex,
            );
            if groups
                .contexts_by_key
                .get(&key)
                .is_none_or(|contexts| contexts.len() < 2)
            {
                continue;
            }
            if !key.explicit_seams.is_empty() {
                continue;
            }
            if let Some(selected) = groups.selected_by_key.get(&key) {
                set_vertex_grade_height(
                    owner,
                    vertex,
                    selected.height_m,
                    NodeGradeCarrierDecision::SameMaterialVertex,
                );
            }
        }
    }
}

fn same_material_vertex_height_candidate_from_vertex(
    owner: NodeBandOwner,
    vertex: &NodeHeightedVertex,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> SameMaterialVertexHeightCandidate {
    SameMaterialVertexHeightCandidate {
        owner,
        height_field_id: vertex.height_field_id,
        height_m: vertex.height_m,
        height_authority: vertex.height_authority,
        has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
            vertex,
            seam_constraints,
        ),
    }
}

fn same_material_vertex_height_support_key_from_parts(
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    seam_constraints: &[NodeRegionSeamConstraint],
    vertex: &NodeHeightedVertex,
) -> SameMaterialVertexHeightSupportKey {
    let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
    let mut explicit_seams =
        material_height_constraints_for_vertex(vertex.point_xz, seam_constraints)
            .into_iter()
            .map(|constraint| NodeGradeExplicitSeamHeightKey::new(point, constraint))
            .collect::<Vec<_>>();
    explicit_seams.sort_unstable();
    explicit_seams.dedup();
    let mut explicit_height_splits =
        explicit_height_split_constraints_for_vertex(vertex.point_xz, seam_constraints)
            .into_iter()
            .map(|constraint| {
                (
                    owner,
                    NodeGradeExplicitSeamHeightKey::new(point, constraint),
                )
            })
            .collect::<Vec<_>>();
    explicit_height_splits.sort_unstable();
    explicit_height_splits.dedup();
    SameMaterialVertexHeightSupportKey {
        kind,
        point,
        explicit_seams,
        explicit_height_splits,
    }
}
