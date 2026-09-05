// SPDX-License-Identifier: GPL-2.0-only

//! Height agreement validation for explicit seams and shared sources.

use super::grade::{
    NodeGradeExplicitSeamHeightKey, canonical_explicit_seam_owner_pair,
    material_height_constraints_for_vertex,
};
use super::model::*;
use super::*;

pub(super) fn validate_explicit_material_seam_heights(
    regions: &[NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let mut shared_heights = BTreeMap::<NodeGradeExplicitSeamHeightKey, ExplicitSeamHeight>::new();
    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
                let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
                let key = NodeGradeExplicitSeamHeightKey::new(point, constraint);
                let height_mm = quantize_m(vertex.height_m);
                let incoming = ExplicitSeamHeight {
                    height_mm,
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                };
                if let Some(existing) = shared_heights.insert(key, incoming) {
                    if existing.height_mm != height_mm {
                        let (owner, opposite_owner) = canonical_explicit_seam_owner_pair(
                            constraint.owner,
                            constraint.opposite_owner,
                        );
                        return Err(NodeHeightFieldError::SharedSourceHeightConflict {
                            point_x_mm: point.x_mm(),
                            point_z_mm: point.z_mm(),
                            kind: region.kind,
                            owner: owner.unwrap_or(existing.owner),
                            opposite_owner,
                            height_field_id: Some(existing.height_field_id),
                            incoming_owner: region.owner,
                            incoming_height_field_id: Some(vertex.height_field_id),
                            constraint_index: Some(constraint.constraint_index),
                            existing_authority: None,
                            incoming_authority: None,
                            existing_height_mm: existing.height_mm,
                            incoming_height_mm: height_mm,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct ExplicitSeamHeight {
    height_mm: i64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

pub(super) fn validate_shared_source_height_agreement(
    regions: &[NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    let mut heights = BTreeMap::<NodeHeightVertexContextKey, SharedSourceHeight>::new();
    for region in regions {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let point = NodeHeightPointKey::from_point(vertex.point_xz);
            let key = NodeHeightVertexContextKey {
                point,
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            let height_mm = quantize_m(vertex.height_m);
            let incoming = SharedSourceHeight {
                height_mm,
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            if let Some(existing) = heights.insert(key, incoming)
                && existing.height_mm != height_mm
            {
                return Err(NodeHeightFieldError::SharedSourceHeightConflict {
                    point_x_mm: point.x_mm(),
                    point_z_mm: point.z_mm(),
                    kind: region.kind,
                    owner: existing.owner,
                    opposite_owner: None,
                    height_field_id: Some(existing.height_field_id),
                    incoming_owner: region.owner,
                    incoming_height_field_id: Some(vertex.height_field_id),
                    constraint_index: None,
                    existing_authority: None,
                    incoming_authority: None,
                    existing_height_mm: existing.height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct SharedSourceHeight {
    height_mm: i64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

pub(super) fn quantize_source_height_m(value_m: f64) -> f64 {
    (value_m * HEIGHT_SOURCE_KEY_SCALE).round() / HEIGHT_SOURCE_KEY_SCALE
}
