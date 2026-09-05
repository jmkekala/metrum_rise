// SPDX-License-Identifier: GPL-2.0-only

//! Same-material shared-vertex height authority normalization.

use super::*;
use std::collections::BTreeMap;

pub(super) fn apply_junctionn_same_owner_canonical_vertex_height_normalization(
    regions: &mut [NodeHeightedRegion],
    constraint_indices: &[NodeGradeRegionConstraintIndex],
) {
    let mut heights_by_key =
        BTreeMap::<NodeGradeVertexContextKey, SameMaterialVertexHeightCandidate>::new();
    let mut samples_by_key = BTreeMap::<NodeGradeVertexContextKey, (usize, Vec<i64>)>::new();

    for (region_index, region) in regions.iter().enumerate() {
        let constraint_index = &constraint_indices[region_index];
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
                source_provenance: vertex.source_provenance,
                has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                    vertex,
                    &region.seam_constraints,
                    constraint_index,
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
    constraint_indices: &[NodeGradeRegionConstraintIndex],
) -> Result<(), NodeHeightFieldError> {
    let groups = collect_same_material_vertex_height_groups(regions, constraint_indices);
    reject_same_material_vertex_height_group_conflicts(&groups)?;
    apply_same_material_vertex_height_groups(regions, &groups);
    Ok(())
}

fn collect_same_material_vertex_height_groups(
    regions: &[NodeHeightedRegion],
    constraint_indices: &[NodeGradeRegionConstraintIndex],
) -> SameMaterialVertexHeightGroups {
    let mut groups = SameMaterialVertexHeightGroups {
        by_key: BTreeMap::new(),
    };

    for (region_index, region) in regions.iter().enumerate() {
        let constraint_index = &constraint_indices[region_index];
        for (contour_index, contour) in region.shape.iter().enumerate() {
            for (vertex_index, vertex) in contour.iter().enumerate() {
                let key = same_material_vertex_height_support_key_from_parts(
                    region.kind,
                    region.owner,
                    &region.seam_constraints,
                    constraint_index,
                    vertex,
                );
                let candidate = same_material_vertex_height_candidate_from_vertex(
                    region.owner,
                    vertex,
                    &region.seam_constraints,
                    constraint_index,
                );
                let context = SameMaterialVertexHeightContext::from_candidate(candidate);
                match groups.by_key.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(SameMaterialVertexHeightGroup {
                            contexts: vec![context],
                            candidates: vec![candidate],
                            selected: candidate,
                            occurrences: vec![(region_index, contour_index, vertex_index)],
                        });
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let group = entry.get_mut();
                        group
                            .occurrences
                            .push((region_index, contour_index, vertex_index));
                        if !group.contexts.contains(&context) {
                            group.contexts.push(context);
                            group.contexts.sort_unstable();
                            group.candidates.push(candidate);
                        }
                        if same_material_vertex_height_candidate_key(candidate)
                            < same_material_vertex_height_candidate_key(group.selected)
                        {
                            group.selected = candidate;
                        }
                    }
                }
            }
        }
    }
    groups
}

fn reject_same_material_vertex_height_group_conflicts(
    groups: &SameMaterialVertexHeightGroups,
) -> Result<(), NodeHeightFieldError> {
    for (key, group) in &groups.by_key {
        if group.contexts.len() < 2 {
            continue;
        }
        if !key.explicit_seams.is_empty() {
            continue;
        }
        reject_same_material_height_conflict(
            key.kind,
            key.point,
            group.candidates.iter().copied(),
        )?;
    }
    Ok(())
}

fn apply_same_material_vertex_height_groups(
    regions: &mut [NodeHeightedRegion],
    groups: &SameMaterialVertexHeightGroups,
) {
    for (key, group) in &groups.by_key {
        if group.contexts.len() < 2 || !key.explicit_seams.is_empty() {
            continue;
        }
        for &(region_index, contour_index, vertex_index) in &group.occurrences {
            let region = &mut regions[region_index];
            let owner = region.owner;
            let vertex = &mut region.shape[contour_index][vertex_index];
            set_vertex_grade_height(
                owner,
                vertex,
                group.selected.height_m,
                NodeGradeCarrierDecision::SameMaterialVertex,
            );
        }
    }
}

fn same_material_vertex_height_candidate_from_vertex(
    owner: NodeBandOwner,
    vertex: &NodeHeightedVertex,
    seam_constraints: &[NodeRegionSeamConstraint],
    constraint_index: &NodeGradeRegionConstraintIndex,
) -> SameMaterialVertexHeightCandidate {
    SameMaterialVertexHeightCandidate {
        owner,
        height_field_id: vertex.height_field_id,
        height_m: vertex.height_m,
        height_authority: vertex.height_authority,
        source_provenance: vertex.source_provenance,
        has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
            vertex,
            seam_constraints,
            constraint_index,
        ),
    }
}

fn same_material_vertex_height_support_key_from_parts(
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    seam_constraints: &[NodeRegionSeamConstraint],
    constraint_index: &NodeGradeRegionConstraintIndex,
    vertex: &NodeHeightedVertex,
) -> SameMaterialVertexHeightSupportKey {
    let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
    let mut explicit_seams = indexed_material_height_constraints_for_vertex(
        vertex.point_xz,
        seam_constraints,
        constraint_index,
    )
    .map(|constraint| NodeGradeExplicitSeamHeightKey::new(point, constraint))
    .collect::<Vec<_>>();
    explicit_seams.sort_unstable();
    explicit_seams.dedup();
    let mut explicit_height_splits = indexed_explicit_height_split_constraints_for_vertex(
        vertex.point_xz,
        seam_constraints,
        constraint_index,
    )
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
        source_provenance: vertex.source_provenance,
        explicit_seams,
        explicit_height_splits,
    }
}
