// SPDX-License-Identifier: GPL-2.0-only

//! Node-height authority agreement for canonical owned node vertices.

use super::super::RoadSurfaceBandKind;
use super::super::arrangement::{
    NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource,
};
use super::super::backend::{RoadVec2, quantize_road_vec2_to_overlay_grid};
use super::super::keys::{SURFACE_MM_PER_M, SurfaceHeightMmKey, SurfaceXzKey};
use super::super::segments::key_lies_on_segment;
use super::model::{
    NodeHeightAuthoritySource, NodeHeightCarrierProvenanceKey, NodeHeightFieldError,
    NodeHeightedRegion, NodeHeightedVertex,
};
use std::collections::BTreeMap;
use std::time::Instant;

fn elapsed_profile_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

mod constraints;
mod seams;
mod shared_edges;
mod shared_vertices;
mod types;

use constraints::{
    NodeGradeRegionConstraintIndex, indexed_explicit_height_split_constraints_for_vertex,
    indexed_material_height_constraints_for_vertex, push_unique_same_material_candidate,
    reject_same_material_height_conflict, same_height_selected_candidate,
    same_material_vertex_height_candidate_key, set_vertex_grade_height,
    vertex_has_explicit_shared_material_seam,
};
pub(crate) use constraints::{
    canonical_explicit_seam_owner_pair, material_height_constraints_for_vertex,
};
use seams::{
    apply_junctionn_explicit_material_seam_height_normalization,
    apply_junctionn_same_material_seam_height_normalization,
};
use shared_edges::apply_junctionn_same_material_shared_edge_height_normalization;
use shared_vertices::{
    apply_junctionn_same_material_vertex_height_normalization,
    apply_junctionn_same_owner_canonical_vertex_height_normalization,
};
pub(crate) use types::{
    NodeGradeCarrierDecision, NodeGradeExplicitSeamHeightKey, NodeGradeVertexAuthority,
};
use types::{
    NodeGradeVertexContextKey, SameMaterialSharedEdgeCandidate,
    SameMaterialSharedEdgeHeightAgreement, SameMaterialSharedEdgeKey,
    SameMaterialSharedVertexContext, SameMaterialSharedVertexKey,
    SameMaterialVertexHeightCandidate, SameMaterialVertexHeightContext,
    SameMaterialVertexHeightGroup, SameMaterialVertexHeightGroups,
    SameMaterialVertexHeightSupportKey,
};

pub(crate) fn apply_junctionn_height_authority_normalization(
    regions: &mut [NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    apply_node_height_authority_normalization(regions, true)
}

pub(crate) fn apply_bend_height_authority_normalization(
    regions: &mut [NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    apply_node_height_authority_normalization(regions, true)
}

fn apply_node_height_authority_normalization(
    regions: &mut [NodeHeightedRegion],
    normalize_same_xz_shared_height_raised_steps: bool,
) -> Result<(), NodeHeightFieldError> {
    let road_debug = crate::debug::category_enabled("road");
    let total_start = road_debug.then(Instant::now);
    let constraint_index_start = road_debug.then(Instant::now);
    let constraint_indices = regions
        .iter()
        .map(NodeGradeRegionConstraintIndex::from_region)
        .collect::<Vec<_>>();
    let constraint_index_ms = elapsed_profile_ms(constraint_index_start);
    let same_owner_start = road_debug.then(Instant::now);
    apply_junctionn_same_owner_canonical_vertex_height_normalization(regions, &constraint_indices);
    let same_owner_ms = elapsed_profile_ms(same_owner_start);
    let shared_edge_start = road_debug.then(Instant::now);
    apply_junctionn_same_material_shared_edge_height_normalization(regions, &constraint_indices)?;
    let shared_edge_ms = elapsed_profile_ms(shared_edge_start);
    let shared_vertex_start = road_debug.then(Instant::now);
    apply_junctionn_same_material_vertex_height_normalization(regions, &constraint_indices)?;
    let shared_vertex_ms = elapsed_profile_ms(shared_vertex_start);
    let same_material_seam_start = road_debug.then(Instant::now);
    apply_junctionn_same_material_seam_height_normalization(regions, &constraint_indices)?;
    let same_material_seam_ms = elapsed_profile_ms(same_material_seam_start);
    let explicit_material_seam_start = road_debug.then(Instant::now);
    apply_junctionn_explicit_material_seam_height_normalization(
        regions,
        &constraint_indices,
        normalize_same_xz_shared_height_raised_steps,
    );
    let explicit_material_seam_ms = elapsed_profile_ms(explicit_material_seam_start);
    if road_debug {
        let total_ms = elapsed_profile_ms(total_start);
        if total_ms >= 1.0 {
            let vertices = regions
                .iter()
                .flat_map(|region| &region.shape)
                .map(Vec::len)
                .sum::<usize>();
            let seam_constraints = regions
                .iter()
                .map(|region| region.seam_constraints.len())
                .sum::<usize>();
            crate::debug_log!(
                "road",
                "node_height_normalization_detail regions={} vertices={} seam_constraints={} constraint_index_ms={:.3} same_owner_ms={:.3} shared_edge_ms={:.3} shared_vertex_ms={:.3} same_material_seam_ms={:.3} explicit_material_seam_ms={:.3} total_ms={:.3}",
                regions.len(),
                vertices,
                seam_constraints,
                constraint_index_ms,
                same_owner_ms,
                shared_edge_ms,
                shared_vertex_ms,
                same_material_seam_ms,
                explicit_material_seam_ms,
                total_ms,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::RoadSurfaceBandKind;
    use super::super::super::arrangement::NodeBandHeightFieldId;
    use super::super::super::backend::RoadVec2;
    use super::super::super::rails::{
        NodeGeneratedContourClaimPriority, NodeGeneratedContourPurpose,
    };
    use super::super::model::{NodeHeightedRegion, NodeHeightedVertex};
    use super::*;

    #[test]
    fn carrier_records_same_material_vertex_decision() {
        let mut regions = vec![
            manual_region(RoadSurfaceBandKind::Carriageway, 9, 2.0004),
            manual_region(RoadSurfaceBandKind::Carriageway, 14, 2.00049),
            manual_region(RoadSurfaceBandKind::Sidewalk, 1, 3.0),
        ];

        apply_junctionn_height_authority_normalization(&mut regions)
            .expect("same-material heights with one height key may share authority");

        let normalized = &regions[1].shape[0][0];
        assert_eq!(
            SurfaceHeightMmKey::from_m_f64(normalized.height_m).as_i64(),
            2000
        );
        assert_eq!(
            normalized
                .grade_authority
                .expect("carrier should write explicit grade authority")
                .decision,
            NodeGradeCarrierDecision::SameMaterialVertex
        );
        assert_eq!(
            regions[2].shape[0][0].height_m, 3.0,
            "different materials must not be pulled into same-material carrier decisions"
        );
    }

    #[test]
    fn carrier_rejects_same_material_vertex_height_conflict() {
        let mut regions = vec![
            manual_region(RoadSurfaceBandKind::Carriageway, 9, 2.0),
            manual_region(RoadSurfaceBandKind::Carriageway, 14, 1.0),
        ];

        assert!(matches!(
            apply_junctionn_height_authority_normalization(&mut regions),
            Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
        ));
        assert_eq!(regions[0].shape[0][0].height_m, 2.0);
        assert_eq!(regions[1].shape[0][0].height_m, 1.0);
    }

    #[test]
    fn carrier_rejects_same_owner_generated_contour_height_conflict() {
        let mut regions = vec![
            manual_region_with_authority(
                RoadSurfaceBandKind::Sidewalk,
                5,
                151.379,
                Some(NodeHeightAuthoritySource::GeneratedContour {
                    purpose: NodeGeneratedContourPurpose::NonRoadBand,
                    claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
                }),
            ),
            manual_region_with_authority(
                RoadSurfaceBandKind::Sidewalk,
                5,
                151.378,
                Some(NodeHeightAuthoritySource::GeneratedContour {
                    purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                    claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                }),
            ),
        ];

        assert!(matches!(
            apply_junctionn_height_authority_normalization(&mut regions),
            Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
        ));
    }

    fn manual_region(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        height_m: f64,
    ) -> NodeHeightedRegion {
        manual_region_with_authority(
            kind,
            owner_index,
            height_m,
            Some(NodeHeightAuthoritySource::SourceInterval),
        )
    }

    fn manual_region_with_authority(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        height_m: f64,
        authority: Option<NodeHeightAuthoritySource>,
    ) -> NodeHeightedRegion {
        let owner = NodeBandOwner::new(kind, owner_index);
        let height_field_id = NodeBandHeightFieldId::new(owner_index, owner_index, kind);
        NodeHeightedRegion {
            kind,
            owner,
            height_field_id,
            shape: vec![vec![NodeHeightedVertex {
                point_xz: RoadVec2::new(-1.0, 0.0),
                height_m,
                height_field_id,
                height_authority: authority,
                source_provenance: None,
                grade_authority: Some(NodeGradeVertexAuthority::new(
                    RoadVec2::new(-1.0, 0.0),
                    height_m,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority },
                )),
            }]],
            area_m2: 1.0,
            seam_constraints: Vec::new(),
        }
    }
}
