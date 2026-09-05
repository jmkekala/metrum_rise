//! Height-patch authority ranking and candidate agreement.

use super::model::*;
use super::*;

impl NodeHeightPatchAuthority {
    pub(super) fn source_interval() -> Self {
        Self {
            owner: None,
            role: NodeHeightPatchAuthorityRole::SourceInterval,
        }
    }

    pub(super) fn terminal_cap() -> Self {
        Self {
            owner: None,
            role: NodeHeightPatchAuthorityRole::TerminalCap,
        }
    }

    pub(super) fn generated_contour(contour: &NodeGeneratedContour) -> Self {
        Self {
            owner: contour.owner,
            role: NodeHeightPatchAuthorityRole::GeneratedContour {
                purpose: contour.purpose,
                claim_priority: contour.claim_priority,
            },
        }
    }

    pub(super) fn rank_for_owned_region(
        self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
    ) -> Option<u8> {
        if let Some(authority_owner) = self.owner
            && authority_owner != owner
        {
            return None;
        }
        Some(match self.role {
            NodeHeightPatchAuthorityRole::SourceInterval => 1,
            NodeHeightPatchAuthorityRole::TerminalCap => 2,
            NodeHeightPatchAuthorityRole::GeneratedContour {
                claim_priority: authority_claim_priority,
                ..
            } => {
                if authority_claim_priority != claim_priority {
                    return None;
                }
                3
            }
        })
    }

    pub(super) fn source(self) -> NodeHeightAuthoritySource {
        match self.role {
            NodeHeightPatchAuthorityRole::SourceInterval => {
                NodeHeightAuthoritySource::SourceInterval
            }
            NodeHeightPatchAuthorityRole::TerminalCap => NodeHeightAuthoritySource::TerminalCap,
            NodeHeightPatchAuthorityRole::GeneratedContour {
                purpose,
                claim_priority,
            } => NodeHeightAuthoritySource::GeneratedContour {
                purpose,
                claim_priority,
            },
        }
    }
}

impl NodeBandHeightField {
    pub(super) fn merge_agreed_height_candidate(
        &self,
        point_xz: RoadVec2,
        owner: Option<NodeBandOwner>,
        accepted: Option<NodeAuthorizedHeightCandidate>,
        incoming: NodeAuthorizedHeightCandidate,
    ) -> Result<NodeAuthorizedHeightCandidate, NodeHeightFieldError> {
        let Some(accepted) = accepted else {
            return Ok(incoming);
        };
        let accepted_height_mm = quantize_m(accepted.height_m);
        let incoming_height_mm = quantize_m(incoming.height_m);
        if accepted_height_mm == incoming_height_mm {
            return Ok(accepted);
        }
        let key = NodeHeightPointKey::from_point(point_xz);
        Err(NodeHeightFieldError::SourceHeightFieldConflict {
            mouth_order_index: self.id.mouth_order_index(),
            band_index: self.id.band_index(),
            source_kind: self.kind,
            height_field_id: self.id,
            owner,
            existing_authority: accepted.authority,
            incoming_authority: incoming.authority,
            point_x_mm: key.x_mm(),
            point_z_mm: key.z_mm(),
            existing_height_mm: accepted_height_mm,
            incoming_height_mm,
        })
    }

    #[cfg(test)]
    pub(super) fn agreed_height(
        &self,
        point_xz: RoadVec2,
        owner: Option<NodeBandOwner>,
        candidates: Vec<NodeAuthorizedHeightCandidate>,
    ) -> Result<NodeEvaluatedHeight, NodeHeightFieldError> {
        let Some(first_candidate) = candidates.first().copied() else {
            let key = NodeHeightPointKey::from_point(point_xz);
            return Err(NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                source_kind: self.kind,
                height_field_id: self.id,
                owner,
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
                axis: "patch",
                raw_parameter: f64::NAN,
            });
        };
        let mut accepted = first_candidate;
        for candidate in candidates.iter().copied().skip(1) {
            accepted =
                self.merge_agreed_height_candidate(point_xz, owner, Some(accepted), candidate)?;
        }
        Ok(NodeEvaluatedHeight {
            height_m: accepted.height_m,
            authority: accepted.authority,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn same_owner_generated_contour_conflict_is_rejected() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let field = test_field(RoadSurfaceBandKind::Sidewalk, 5);

        assert!(matches!(
            field.agreed_height(
                RoadVec2::new(0.037962, 0.004996),
                Some(owner),
                vec![
                    NodeAuthorizedHeightCandidate {
                        authority: NodeHeightAuthoritySource::GeneratedContour {
                            purpose: NodeGeneratedContourPurpose::NonRoadBand,
                            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
                        },
                        height_m: 151.379,
                    },
                    NodeAuthorizedHeightCandidate {
                        authority: NodeHeightAuthoritySource::GeneratedContour {
                            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                        },
                        height_m: 151.378,
                    },
                ],
            ),
            Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
        ));
    }

    #[test]
    fn mixed_authority_height_conflict_is_still_rejected() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let field = test_field(RoadSurfaceBandKind::Sidewalk, 5);

        assert!(matches!(
            field.agreed_height(
                RoadVec2::new(0.0, 0.0),
                Some(owner),
                vec![
                    NodeAuthorizedHeightCandidate {
                        authority: NodeHeightAuthoritySource::SourceInterval,
                        height_m: 1.0,
                    },
                    NodeAuthorizedHeightCandidate {
                        authority: NodeHeightAuthoritySource::GeneratedContour {
                            purpose: NodeGeneratedContourPurpose::NonRoadBand,
                            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
                        },
                        height_m: 2.0,
                    },
                ],
            ),
            Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
        ));
    }

    fn test_field(kind: RoadSurfaceBandKind, band_index: usize) -> NodeBandHeightField {
        NodeBandHeightField {
            id: NodeBandHeightFieldId::new(0, band_index, kind),
            kind,
            patches: Vec::new(),
            source_handoff_keys: BTreeSet::new(),
        }
    }
}
