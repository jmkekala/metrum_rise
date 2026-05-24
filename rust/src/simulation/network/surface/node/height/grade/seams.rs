//! Explicit same-material and material-transition seam height normalization.

use super::*;
use std::collections::BTreeMap;

pub(super) fn apply_junctionn_same_material_seam_height_normalization(
    regions: &mut [NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let mut candidates_by_key =
        BTreeMap::<NodeGradeExplicitSeamHeightKey, Vec<SameMaterialVertexHeightCandidate>>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
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

    for region in regions {
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
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
) {
    let mut candidates_by_key =
        BTreeMap::<NodeGradeExplicitSeamHeightKey, Vec<SameMaterialVertexHeightCandidate>>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
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
                    has_explicit_shared_material_seam: true,
                };
                push_unique_same_material_candidate(&mut candidates_by_key, key, candidate);
            }
        }
    }

    let selected_by_key = candidates_by_key
        .into_iter()
        .filter_map(|(key, candidates)| {
            same_height_selected_candidate(&candidates).map(|selected| (key, selected.height_m))
        })
        .collect::<BTreeMap<_, _>>();

    if selected_by_key.is_empty() {
        return;
    }

    for region in regions {
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let constraints =
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints);
            for constraint in constraints {
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
