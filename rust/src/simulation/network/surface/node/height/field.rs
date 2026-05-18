//! Height-field and height-patch evaluation.

use super::carriers::*;
use super::evaluate::*;
use super::model::*;
use super::seams::quantize_source_height_m;
use super::source_edges::*;
use super::triangles::*;
use super::vertices::height_vertex_heights_from_vertices;
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

    pub(super) fn evaluate_height(&self, point_xz: RoadVec2) -> Result<f64, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        let mut outside_error = None;
        for patch in &self.patches {
            match patch.evaluate_surface_height(self.id, self.kind, point_xz)? {
                NodeHeightPatchEvaluation::Inside(height_m) => {
                    candidates.push(NodeAuthorizedHeightCandidate {
                        authority_rank: 0,
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

    pub(super) fn register_contour_edge_support(
        &mut self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) {
        let point = height_source_point_key(point_xz);
        for patch in &mut self.patches {
            if patch
                .authority
                .rank_for_owned_region(owner, claim_priority)
                .is_some()
                && let Some(height_m) = patch.explicit_vertex_heights.get(&point).copied()
            {
                patch.contour_edge_support_heights.insert(point, height_m);
            }
        }
    }

    pub(super) fn evaluate_authorized_height(
        &self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) -> Result<NodeEvaluatedHeight, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        let mut outside_error = None;
        for patch in &self.patches {
            let Some(authority_rank) = patch.authority.rank_for_owned_region(owner, claim_priority)
            else {
                continue;
            };
            if self.source_handoff_authorized(owner, claim_priority, point_xz)
                && let Some(height_m) = patch.source_handoff_height_at(point_xz)
            {
                candidates.push(NodeAuthorizedHeightCandidate {
                    authority_rank: 4,
                    authority: patch.authority.source(),
                    height_m,
                });
                continue;
            }
            match patch.evaluate_surface_height(self.id, self.kind, point_xz)? {
                NodeHeightPatchEvaluation::Inside(height_m) => {
                    candidates.push(NodeAuthorizedHeightCandidate {
                        authority_rank,
                        authority: patch.authority.source(),
                        height_m,
                    });
                }
                NodeHeightPatchEvaluation::Outside(error) => {
                    if outside_error.is_none() {
                        outside_error = Some(owner_scoped_outside_height_error(error, owner));
                    }
                }
            }
        }
        if candidates.is_empty() {
            if let Some(error) = outside_error {
                return Err(error);
            }
            let key = NodeHeightPointKey::from_point(point_xz);
            return Err(NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                source_kind: self.kind,
                height_field_id: self.id,
                owner: Some(owner),
                point_x_mm: key.x_mm(),
                point_z_mm: key.z_mm(),
                axis: "canonical_authority",
                raw_parameter: f64::NAN,
            });
        }

        let best_rank = candidates
            .iter()
            .map(|candidate| candidate.authority_rank)
            .max()
            .expect("non-empty candidate set has a maximum rank");
        let heights_m = if best_rank == 4 {
            candidates
        } else {
            candidates
                .into_iter()
                .filter(|candidate| candidate.authority_rank == best_rank)
                .collect()
        };
        self.agreed_height(point_xz, Some(owner), heights_m)
    }

    fn register_generated_contour_source_handoffs(
        &mut self,
        contour: &NodeGeneratedContour,
    ) -> Result<(), NodeHeightFieldError> {
        let (Some(owner), Some(points_world)) =
            (contour.owner, contour.height_points_world.as_ref())
        else {
            return Ok(());
        };
        for point in points_world {
            let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
            let Some(source_height_m) = self.source_interval_height_at(point_xz)? else {
                continue;
            };
            let source_height_key = SurfaceHeightMmKey::from_m_f64(source_height_m);
            let contour_height_key = SurfaceHeightMmKey::from_m_f64(point.y);
            if source_height_key != contour_height_key {
                let key = NodeHeightPointKey::from_point(point_xz);
                return Err(
                    NodeHeightFieldError::GeneratedContourSourceHandoffMismatch {
                        mouth_order_index: self.id.mouth_order_index(),
                        band_index: self.id.band_index(),
                        source_kind: self.kind,
                        height_field_id: self.id,
                        purpose: contour.purpose,
                        claim_priority: contour.claim_priority,
                        owner: contour.owner,
                        point_x_mm: key.x_mm(),
                        point_z_mm: key.z_mm(),
                        source_height_mm: source_height_key.as_i64(),
                        contour_height_mm: contour_height_key.as_i64(),
                    },
                );
            }
            self.source_handoff_keys
                .insert(NodeAuthorizedSourceHandoffKey {
                    owner,
                    claim_priority: contour.claim_priority,
                    point: height_source_point_key(point_xz),
                });
        }
        Ok(())
    }

    fn source_interval_height_at(
        &self,
        point_xz: RoadVec2,
    ) -> Result<Option<f64>, NodeHeightFieldError> {
        for patch in &self.patches {
            match patch.source_handoff_height_at(point_xz) {
                Some(height_m) => return Ok(Some(height_m)),
                None => continue,
            }
        }
        Ok(None)
    }

    fn source_handoff_authorized(
        &self,
        owner: NodeBandOwner,
        claim_priority: NodeGeneratedContourClaimPriority,
        point_xz: RoadVec2,
    ) -> bool {
        self.source_handoff_keys
            .contains(&NodeAuthorizedSourceHandoffKey {
                owner,
                claim_priority,
                point: height_source_point_key(point_xz),
            })
    }

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
        let first_height_mm = quantize_m(first_candidate.height_m);
        for candidate in candidates.iter().copied().skip(1) {
            let height_mm = quantize_m(candidate.height_m);
            if height_mm != first_height_mm {
                let key = NodeHeightPointKey::from_point(point_xz);
                return Err(NodeHeightFieldError::SourceHeightFieldConflict {
                    mouth_order_index: self.id.mouth_order_index(),
                    band_index: self.id.band_index(),
                    source_kind: self.kind,
                    height_field_id: self.id,
                    owner,
                    existing_authority: first_candidate.authority,
                    incoming_authority: candidate.authority,
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    existing_height_mm: first_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
        Ok(NodeEvaluatedHeight {
            height_m: first_candidate.height_m,
            authority: first_candidate.authority,
        })
    }
}

impl NodeBandHeightPatch {
    pub(super) fn from_interval(
        id: NodeBandHeightFieldId,
        interval: &NodeInputBandInterval,
        source_support_points: Option<&[RoadVec3]>,
    ) -> Result<Self, NodeHeightFieldError> {
        let triangles = interval_height_carrier(id, interval)?;
        let explicit_vertices = interval_height_carrier_vertices(id, interval)?;
        let explicit_vertex_heights = height_vertex_heights_from_vertices(&explicit_vertices)
            .map_err(|error| {
                invalid_source_band_height_carrier_error(
                    id,
                    interval.band_kind,
                    error.diagnostic_reason(),
                )
            })?;
        let source_support_heights = source_support_heights(source_support_points);
        let source_handoff_support_heights =
            source_handoff_support_heights(interval, &source_support_heights);
        let mut contour_edge_support_heights = explicit_vertex_heights.clone();
        contour_edge_support_heights.extend(source_support_heights);
        Ok(Self {
            authority: NodeHeightPatchAuthority::source_interval(),
            explicit_vertex_heights,
            source_handoff_support_heights,
            contour_edge_support_heights,
            triangles: Some(triangles),
        })
    }

    pub(super) fn from_terminal_cap_band(
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        cap_band: &NodeTerminalCapBand,
    ) -> Result<Self, NodeHeightFieldError> {
        let authority = NodeHeightPatchAuthority::terminal_cap();
        let explicit_vertex_heights = height_vertex_heights_from_vertices(&cap_band.contour_world)
            .map_err(|error| NodeHeightFieldError::InvalidHeightCarrierContour {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                source_kind,
                height_field_id: id,
                authority: authority.source(),
                reason: error.diagnostic_reason(),
            })?;
        let contour_edge_support_heights = explicit_vertex_heights.clone();
        Ok(Self {
            authority,
            explicit_vertex_heights,
            source_handoff_support_heights: BTreeMap::new(),
            contour_edge_support_heights,
            triangles: Some(terminal_cap_band_height_triangles(
                id,
                source_kind,
                authority,
                cap_band,
            )?),
        })
    }

    pub(super) fn from_generated_contour(
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        contour: &NodeGeneratedContour,
    ) -> Result<Self, NodeHeightFieldError> {
        let Some(points_world) = &contour.height_points_world else {
            return Err(NodeHeightFieldError::MissingGeneratedContourHeightPoints {
                mouth_order_index: id.mouth_order_index(),
                band_index: id.band_index(),
                source_kind,
                height_field_id: id,
                purpose: contour.purpose,
                claim_priority: contour.claim_priority,
            });
        };
        Self::from_heighted_contour(
            id,
            source_kind,
            points_world,
            NodeHeightPatchAuthority::generated_contour(contour),
        )
    }

    pub(super) fn from_heighted_contour(
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        points: &[RoadVec3],
        authority: NodeHeightPatchAuthority,
    ) -> Result<Self, NodeHeightFieldError> {
        let explicit_vertex_heights =
            height_vertex_heights_from_vertices(points).map_err(|error| {
                NodeHeightFieldError::InvalidHeightCarrierContour {
                    mouth_order_index: id.mouth_order_index(),
                    band_index: id.band_index(),
                    source_kind,
                    height_field_id: id,
                    authority: authority.source(),
                    reason: error.diagnostic_reason(),
                }
            })?;
        let contour_edge_support_heights = explicit_vertex_heights.clone();
        Ok(Self {
            authority,
            explicit_vertex_heights,
            source_handoff_support_heights: BTreeMap::new(),
            contour_edge_support_heights,
            triangles: Some(height_triangles_from_contour(
                id,
                source_kind,
                authority,
                points,
            )?),
        })
    }

    pub(super) fn source_handoff_height_at(&self, point_xz: RoadVec2) -> Option<f64> {
        if self.authority.role != NodeHeightPatchAuthorityRole::SourceInterval {
            return None;
        }
        self.source_handoff_support_heights
            .get(&height_source_point_key(point_xz))
            .copied()
    }

    pub(super) fn evaluate_surface_height(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
    ) -> Result<NodeHeightPatchEvaluation, NodeHeightFieldError> {
        if let Some(height_m) = self
            .explicit_vertex_heights
            .get(&height_source_point_key(point_xz))
            .copied()
        {
            return Ok(NodeHeightPatchEvaluation::Inside(height_m));
        }
        if let Some(height_m) = self
            .contour_edge_support_heights
            .get(&height_source_point_key(point_xz))
            .copied()
        {
            return Ok(NodeHeightPatchEvaluation::Inside(height_m));
        }
        let mut triangle_outside_error = None;
        if let Some(triangles) = &self.triangles {
            match self.evaluate_triangle_surface_height(id, source_kind, point_xz, triangles)? {
                NodeHeightPatchEvaluation::Inside(height_m) => {
                    return Ok(NodeHeightPatchEvaluation::Inside(height_m));
                }
                NodeHeightPatchEvaluation::Outside(error) => {
                    triangle_outside_error = Some(error);
                }
            }
        }
        Ok(NodeHeightPatchEvaluation::Outside(
            triangle_outside_error.unwrap_or_else(|| {
                self.outside_field_error(id, source_kind, point_xz, "height_carrier", f64::NAN)
            }),
        ))
    }

    pub(super) fn evaluate_triangle_surface_height(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
        triangles: &[NodeBandHeightTriangle],
    ) -> Result<NodeHeightPatchEvaluation, NodeHeightFieldError> {
        let mut candidates = Vec::new();
        for triangle in triangles {
            if let Some(height_m) = triangle.height_at(point_xz) {
                candidates.push(height_m);
            }
        }
        if candidates.is_empty() {
            return Ok(NodeHeightPatchEvaluation::Outside(
                self.outside_field_error(id, source_kind, point_xz, "triangle", f64::NAN),
            ));
        }
        let first_height_m = candidates[0];
        let first_height_mm = quantize_m(first_height_m);
        for height_m in candidates.iter().copied().skip(1) {
            let height_mm = quantize_m(height_m);
            if height_mm != first_height_mm {
                let key = NodeHeightPointKey::from_point(point_xz);
                return Err(NodeHeightFieldError::SourceHeightFieldConflict {
                    mouth_order_index: id.mouth_order_index(),
                    band_index: id.band_index(),
                    source_kind,
                    height_field_id: id,
                    owner: self.authority.owner,
                    existing_authority: self.authority.source(),
                    incoming_authority: self.authority.source(),
                    point_x_mm: key.x_mm(),
                    point_z_mm: key.z_mm(),
                    existing_height_mm: first_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
        Ok(NodeHeightPatchEvaluation::Inside(first_height_m))
    }

    pub(super) fn outside_field_error(
        &self,
        id: NodeBandHeightFieldId,
        source_kind: RoadSurfaceBandKind,
        point_xz: RoadVec2,
        axis: &'static str,
        raw_parameter: f64,
    ) -> NodeHeightFieldError {
        let key = NodeHeightPointKey::from_point(point_xz);
        NodeHeightFieldError::VertexOutsideHeightField {
            mouth_order_index: id.mouth_order_index(),
            band_index: id.band_index(),
            source_kind,
            height_field_id: id,
            owner: self.authority.owner,
            point_x_mm: key.x_mm(),
            point_z_mm: key.z_mm(),
            axis,
            raw_parameter,
        }
    }
}

fn source_handoff_support_heights(
    interval: &NodeInputBandInterval,
    source_support_heights: &BTreeMap<NodeHeightSourcePointKey, f64>,
) -> BTreeMap<NodeHeightSourcePointKey, f64> {
    let base_keys = interval_declared_source_point_keys(interval);
    let mut support_keys = BTreeMap::new();
    for (&point_key, &height_m) in source_support_heights {
        if base_keys.contains(&point_key) {
            continue;
        }
        support_keys.insert(point_key, height_m);
    }
    support_keys
}

fn source_support_heights(
    source_support_points: Option<&[RoadVec3]>,
) -> BTreeMap<NodeHeightSourcePointKey, f64> {
    let mut support_heights =
        BTreeMap::<NodeHeightSourcePointKey, Option<(SurfaceHeightMmKey, f64)>>::new();
    for point in source_support_points.unwrap_or(&[]) {
        let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
        let point_key = height_source_point_key(point_xz);
        let height_key = SurfaceHeightMmKey::from_m_f64(point.y);
        let height_m = quantize_source_height_m(point.y);
        match support_heights.get_mut(&point_key) {
            Some(Some((existing_height_key, _))) if *existing_height_key == height_key => {}
            Some(existing) => {
                *existing = None;
            }
            None => {
                support_heights.insert(point_key, Some((height_key, height_m)));
            }
        }
    }
    support_heights
        .into_iter()
        .filter_map(|(point_key, height)| height.map(|(_, height_m)| (point_key, height_m)))
        .collect()
}

fn interval_declared_source_point_keys(
    interval: &NodeInputBandInterval,
) -> BTreeSet<NodeHeightSourcePointKey> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        return [
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ]
        .into_iter()
        .map(|point| height_source_point_key(quantize_road_vec2_to_overlay_grid(xz(point))))
        .collect();
    }

    interval
        .start_path_world
        .iter()
        .chain(interval.end_path_world.iter())
        .map(|point| height_source_point_key(quantize_road_vec2_to_overlay_grid(xz(*point))))
        .collect()
}
