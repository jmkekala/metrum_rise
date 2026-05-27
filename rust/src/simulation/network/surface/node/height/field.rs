//! Height-field construction and evaluation API.

use super::evaluate::*;
use super::model::*;
use super::*;
use std::collections::BTreeSet;

impl NodeBandHeightField {
    pub(super) fn from_interval(
        mouth_order_index: usize,
        interval: &NodeInputBandInterval,
        source_support_points: Option<&[RoadVec3]>,
    ) -> Result<Self, NodeHeightFieldError> {
        let id =
            NodeBandHeightFieldId::new(mouth_order_index, interval.band_index, interval.band_kind);
        Ok(Self {
            id,
            kind: interval.band_kind,
            patches: vec![NodeBandHeightPatch::from_interval(
                id,
                interval,
                source_support_points,
            )?],
            source_handoff_keys: BTreeSet::new(),
        })
    }

    pub(super) fn from_terminal_cap_band(
        mouth_order_index: usize,
        cap_band: &NodeTerminalCapBand,
    ) -> Result<Self, NodeHeightFieldError> {
        let id = NodeBandHeightFieldId::new(
            mouth_order_index,
            cap_band.source_band_index,
            cap_band.band_kind,
        );
        Ok(Self {
            id,
            kind: cap_band.band_kind,
            patches: vec![NodeBandHeightPatch::from_terminal_cap_band(
                id,
                cap_band.band_kind,
                cap_band,
            )?],
            source_handoff_keys: BTreeSet::new(),
        })
    }

    pub(super) fn extend_with_terminal_cap_band(
        &mut self,
        mouth_order_index: usize,
        cap_band: &NodeTerminalCapBand,
    ) -> Result<(), NodeHeightFieldError> {
        if cap_band.band_kind != self.kind {
            return Err(NodeHeightFieldError::SourceBandKindMismatch {
                mouth_order_index,
                band_index: cap_band.source_band_index,
                region_kind: self.kind,
                source_kind: cap_band.band_kind,
            });
        }
        self.patches
            .push(NodeBandHeightPatch::from_terminal_cap_band(
                self.id, self.kind, cap_band,
            )?);
        Ok(())
    }

    pub(super) fn extend_with_generated_contour(
        &mut self,
        contour: &NodeGeneratedContour,
    ) -> Result<(), NodeHeightFieldError> {
        self.register_generated_contour_source_handoffs(contour)?;
        self.patches
            .push(NodeBandHeightPatch::from_generated_contour(
                self.id, self.kind, contour,
            )?);
        Ok(())
    }

    pub(super) fn extend_with_generated_contour_edge_support(
        &mut self,
        contour: &NodeGeneratedContour,
    ) -> Result<(), NodeHeightFieldError> {
        self.register_generated_contour_source_handoffs(contour)?;
        self.patches
            .push(NodeBandHeightPatch::from_generated_contour_edge_support(
                self.id, self.kind, contour,
            )?);
        Ok(())
    }

    pub(super) fn evaluate_height(&self, point_xz: RoadVec2) -> Result<f64, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        let mut outside_error = None;
        for patch in &self.patches {
            match patch.evaluate_surface_height(self.id, self.kind, point_xz)? {
                NodeHeightPatchEvaluation::Inside(height_m) => {
                    candidates.push(NodeAuthorizedHeightCandidate {
                        authority: patch.authority.source(),
                        height_m,
                    });
                }
                NodeHeightPatchEvaluation::Outside(error) => {
                    if outside_error.is_none() {
                        outside_error = Some(error);
                    }
                }
            }
        }
        if candidates.is_empty() {
            return Err(outside_error.unwrap_or_else(|| {
                let key = NodeHeightPointKey::from_point(point_xz);
                NodeHeightFieldError::VertexOutsideHeightField {
                    mouth_order_index: self.id.mouth_order_index(),
                    band_index: self.id.band_index(),
                    source_kind: self.kind,
                    height_field_id: self.id,
                    owner: None,
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    axis: "patch",
                    raw_parameter: f64::NAN,
                }
            }));
        }

        self.agreed_height(point_xz, None, candidates)
            .map(|height| height.height_m)
    }

    pub(super) fn evaluate_authorized_height(
        &self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) -> Result<NodeEvaluatedHeight, NodeHeightFieldError> {
        let mut outside_error = None;
        for target_rank in (1..=4).rev() {
            if target_rank == 4 && self.source_handoff_authorized(owner, claim_priority, point_xz) {
                if let Some(candidate) = self.source_handoff_candidate_at(point_xz) {
                    return Ok(NodeEvaluatedHeight {
                        height_m: candidate.height_m,
                        authority: candidate.authority,
                    });
                }
            }
            let mut candidates = Vec::new();
            for patch in &self.patches {
                let Some(authority_rank) =
                    patch.authority.rank_for_owned_region(owner, claim_priority)
                else {
                    continue;
                };
                if authority_rank != target_rank {
                    continue;
                }
                match patch.evaluate_surface_height(self.id, self.kind, point_xz) {
                    Ok(NodeHeightPatchEvaluation::Inside(height_m)) => {
                        candidates.push(NodeAuthorizedHeightCandidate {
                            authority: patch.authority.source(),
                            height_m,
                        });
                    }
                    Ok(NodeHeightPatchEvaluation::Outside(error)) => {
                        if outside_error.is_none() {
                            outside_error = Some(owner_scoped_outside_height_error(error, owner));
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            if !candidates.is_empty() {
                return self.agreed_height(point_xz, Some(owner), candidates);
            }
        }

        if let Some(error) = outside_error {
            return Err(error);
        }
        let key = NodeHeightPointKey::from_point(point_xz);
        Err(NodeHeightFieldError::VertexOutsideHeightField {
            mouth_order_index: self.id.mouth_order_index(),
            band_index: self.id.band_index(),
            source_kind: self.kind,
            height_field_id: self.id,
            owner: Some(owner),
            point_x_mm: key.x_mm(),
            point_z_mm: key.z_mm(),
            axis: "canonical_authority",
            raw_parameter: f64::NAN,
        })
    }
}
