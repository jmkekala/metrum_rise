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

mod constraints;
mod seams;
mod shared_edges;
mod shared_vertices;
mod types;

pub(crate) use constraints::{
    canonical_explicit_seam_owner_pair, material_height_constraints_for_vertex,
};
use constraints::{
    explicit_height_split_constraints_for_vertex, point_lies_on_height_segment,
    push_unique_same_material_candidate, reject_same_material_height_conflict,
    same_height_selected_candidate, same_material_vertex_height_candidate_key,
    set_vertex_grade_height, vertex_has_explicit_shared_material_seam,
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
    SameMaterialVertexHeightGroups, SameMaterialVertexHeightSupportKey,
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
    apply_junctionn_same_owner_canonical_vertex_height_normalization(regions);
    apply_junctionn_same_material_shared_edge_height_normalization(regions)?;
    apply_junctionn_same_material_vertex_height_normalization(regions)?;
    apply_junctionn_same_material_seam_height_normalization(regions)?;
    apply_junctionn_explicit_material_seam_height_normalization(
        regions,
        normalize_same_xz_shared_height_raised_steps,
    );
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
