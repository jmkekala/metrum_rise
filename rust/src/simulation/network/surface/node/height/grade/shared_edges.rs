//! Same-material shared-edge height authority normalization.

use super::*;
use std::collections::BTreeMap;

pub(super) fn apply_junctionn_same_material_shared_edge_height_normalization(
    regions: &mut [NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let candidates_by_edge = collect_same_material_shared_edge_height_candidates(regions);
    let agreement = resolve_same_material_shared_edge_height_agreement(candidates_by_edge)?;
    apply_same_material_shared_edge_height_agreement(regions, &agreement);
    Ok(())
}

fn collect_same_material_shared_edge_height_candidates(
    regions: &[NodeHeightedRegion],
) -> BTreeMap<SameMaterialSharedEdgeKey, Vec<SameMaterialSharedEdgeCandidate>> {
    let mut candidates_by_edge =
        BTreeMap::<SameMaterialSharedEdgeKey, Vec<SameMaterialSharedEdgeCandidate>>::new();

    for region in regions {
        for contour in &region.shape {
            if contour.len() < 2 {
                continue;
            }
            for index in 0..contour.len() {
                let start = &contour[index];
                let end = &contour[(index + 1) % contour.len()];
                let Some((key, candidate)) = SameMaterialSharedEdgeCandidate::new(
                    region.kind,
                    region.owner,
                    &region.seam_constraints,
                    start,
                    end,
                ) else {
                    continue;
                };
                let candidates = candidates_by_edge.entry(key).or_default();
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
    }
    candidates_by_edge
}

fn resolve_same_material_shared_edge_height_agreement(
    candidates_by_edge: BTreeMap<SameMaterialSharedEdgeKey, Vec<SameMaterialSharedEdgeCandidate>>,
) -> Result<SameMaterialSharedEdgeHeightAgreement, NodeHeightFieldError> {
    let mut selected_by_vertex =
        BTreeMap::<SameMaterialSharedVertexKey, SameMaterialVertexHeightCandidate>::new();
    let mut affected_contexts_by_vertex =
        BTreeMap::<SameMaterialSharedVertexKey, Vec<SameMaterialSharedVertexContext>>::new();

    for (edge, candidates) in candidates_by_edge {
        if candidates.len() < 2 {
            continue;
        }
        for endpoint in [edge.start, edge.end] {
            if candidates
                .iter()
                .copied()
                .any(|candidate| candidate.endpoint_has_explicit_height_split(endpoint))
            {
                continue;
            }
            reject_same_material_height_conflict(
                edge.kind,
                endpoint,
                candidates
                    .iter()
                    .copied()
                    .map(|candidate| candidate.endpoint_candidate(endpoint)),
            )?;
            let selected = candidates
                .iter()
                .copied()
                .map(|candidate| candidate.endpoint_candidate(endpoint))
                .min_by_key(|candidate| same_material_vertex_height_candidate_key(*candidate))
                .expect("shared edge with candidates has an endpoint candidate");
            let vertex_key = SameMaterialSharedVertexKey {
                kind: edge.kind,
                point: endpoint,
            };
            selected_by_vertex
                .entry(vertex_key)
                .and_modify(|existing| {
                    if same_material_vertex_height_candidate_key(selected)
                        < same_material_vertex_height_candidate_key(*existing)
                    {
                        *existing = selected;
                    }
                })
                .or_insert(selected);
            let contexts = affected_contexts_by_vertex.entry(vertex_key).or_default();
            for candidate in &candidates {
                let context = SameMaterialSharedVertexContext {
                    owner: candidate.owner,
                    height_field_id: candidate.height_field_id,
                };
                if !contexts.contains(&context) {
                    contexts.push(context);
                }
            }
        }
    }
    Ok(SameMaterialSharedEdgeHeightAgreement {
        selected_by_vertex,
        affected_contexts_by_vertex,
    })
}

fn apply_same_material_shared_edge_height_agreement(
    regions: &mut [NodeHeightedRegion],
    agreement: &SameMaterialSharedEdgeHeightAgreement,
) {
    for region in regions {
        let owner = region.owner;
        let kind = region.kind;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key = SameMaterialSharedVertexKey {
                kind,
                point: SurfaceXzKey::from_road_xz(vertex.point_xz),
            };
            let Some(contexts) = agreement.affected_contexts_by_vertex.get(&key) else {
                continue;
            };
            let context = SameMaterialSharedVertexContext {
                owner,
                height_field_id: vertex.height_field_id,
            };
            if !contexts.contains(&context) {
                continue;
            }
            if let Some(selected) = agreement.selected_by_vertex.get(&key) {
                set_vertex_grade_height(
                    owner,
                    vertex,
                    selected.height_m,
                    NodeGradeCarrierDecision::SameMaterialSharedEdge,
                );
            }
        }
    }
}
