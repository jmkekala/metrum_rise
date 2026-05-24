//! Shared seam and candidate constraints for node grade normalization.

use super::*;

impl NodeGradeExplicitSeamHeightKey {
    pub(crate) fn new(point: SurfaceXzKey, constraint: &NodeRegionSeamConstraint) -> Self {
        let (owner, opposite_owner) =
            canonical_explicit_seam_owner_pair(constraint.owner, constraint.opposite_owner);
        let start = SurfaceXzKey::from_road_xz(constraint.start_xz);
        let end = SurfaceXzKey::from_road_xz(constraint.end_xz);
        let (start, end) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        Self {
            point,
            constraint_index: constraint.constraint_index,
            owner,
            opposite_owner,
            start,
            end,
        }
    }
}

impl SameMaterialSharedEdgeCandidate {
    pub(super) fn new(
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        seam_constraints: &[NodeRegionSeamConstraint],
        start: &NodeHeightedVertex,
        end: &NodeHeightedVertex,
    ) -> Option<(SameMaterialSharedEdgeKey, Self)> {
        let mut start_key = SurfaceXzKey::from_road_xz(start.point_xz);
        let mut end_key = SurfaceXzKey::from_road_xz(end.point_xz);
        if start_key == end_key {
            return None;
        }
        if edge_has_explicit_height_split(start.point_xz, end.point_xz, seam_constraints) {
            return None;
        }
        let mut start_height_m = start.height_m;
        let mut end_height_m = end.height_m;
        let mut start_height_authority = start.height_authority;
        let mut end_height_authority = end.height_authority;
        let mut start_has_explicit_shared_material_seam =
            vertex_has_explicit_shared_material_seam(start, seam_constraints);
        let mut end_has_explicit_shared_material_seam =
            vertex_has_explicit_shared_material_seam(end, seam_constraints);
        let mut start_has_explicit_height_split =
            vertex_has_explicit_height_split(start, seam_constraints);
        let mut end_has_explicit_height_split =
            vertex_has_explicit_height_split(end, seam_constraints);
        if end_key < start_key {
            std::mem::swap(&mut start_key, &mut end_key);
            std::mem::swap(&mut start_height_m, &mut end_height_m);
            std::mem::swap(&mut start_height_authority, &mut end_height_authority);
            std::mem::swap(
                &mut start_has_explicit_shared_material_seam,
                &mut end_has_explicit_shared_material_seam,
            );
            std::mem::swap(
                &mut start_has_explicit_height_split,
                &mut end_has_explicit_height_split,
            );
        }
        Some((
            SameMaterialSharedEdgeKey {
                kind,
                start: start_key,
                end: end_key,
            },
            Self {
                owner,
                height_field_id: start.height_field_id,
                start: start_key,
                start_height_m,
                start_height_authority,
                start_has_explicit_shared_material_seam,
                start_has_explicit_height_split,
                end: end_key,
                end_height_m,
                end_height_authority,
                end_has_explicit_shared_material_seam,
                end_has_explicit_height_split,
            },
        ))
    }

    pub(super) fn endpoint_candidate(
        self,
        point: SurfaceXzKey,
    ) -> SameMaterialVertexHeightCandidate {
        let (height_m, height_authority, has_explicit_shared_material_seam) = if point == self.start
        {
            (
                self.start_height_m,
                self.start_height_authority,
                self.start_has_explicit_shared_material_seam,
            )
        } else {
            debug_assert_eq!(point, self.end);
            (
                self.end_height_m,
                self.end_height_authority,
                self.end_has_explicit_shared_material_seam,
            )
        };
        SameMaterialVertexHeightCandidate {
            owner: self.owner,
            height_field_id: self.height_field_id,
            height_m,
            height_authority,
            has_explicit_shared_material_seam,
        }
    }

    pub(super) fn endpoint_has_explicit_height_split(self, point: SurfaceXzKey) -> bool {
        if point == self.start {
            self.start_has_explicit_height_split
        } else {
            debug_assert_eq!(point, self.end);
            self.end_has_explicit_height_split
        }
    }
}

pub(crate) fn material_height_constraints_for_vertex(
    point_xz: RoadVec2,
    constraints: &[NodeRegionSeamConstraint],
) -> Vec<&NodeRegionSeamConstraint> {
    let mut matches = constraints
        .iter()
        .filter(|constraint| constraint.constrains_shared_height)
        .filter(|constraint| {
            point_lies_on_height_segment(point_xz, constraint.start_xz, constraint.end_xz)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| {
        (
            constraint.priority_key(),
            constraint.constraint_index,
            SurfaceXzKey::from_road_xz(constraint.start_xz),
            SurfaceXzKey::from_road_xz(constraint.end_xz),
        )
    });
    matches.dedup_by_key(|constraint| {
        let point = SurfaceXzKey::from_road_xz(point_xz);
        NodeGradeExplicitSeamHeightKey::new(point, constraint)
    });
    matches
}

pub(crate) fn canonical_explicit_seam_owner_pair(
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    match (owner, opposite_owner) {
        (Some(owner), Some(opposite_owner)) if opposite_owner < owner => {
            (Some(opposite_owner), Some(owner))
        }
        pair => pair,
    }
}

pub(super) fn same_material_vertex_height_candidate_key(
    candidate: SameMaterialVertexHeightCandidate,
) -> (bool, bool, usize, usize, usize) {
    (
        !candidate.has_explicit_shared_material_seam,
        candidate.height_authority != Some(NodeHeightAuthoritySource::SourceInterval),
        candidate.height_field_id.mouth_order_index(),
        candidate.height_field_id.band_index(),
        candidate.owner.owner_index(),
    )
}

pub(super) fn vertex_has_explicit_shared_material_seam(
    vertex: &NodeHeightedVertex,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> bool {
    material_height_constraints_for_vertex(vertex.point_xz, seam_constraints)
        .into_iter()
        .any(|constraint| constraint.is_material_transition)
}

pub(super) fn vertex_has_explicit_height_split(
    vertex: &NodeHeightedVertex,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> bool {
    !explicit_height_split_constraints_for_vertex(vertex.point_xz, seam_constraints).is_empty()
}

pub(super) fn edge_has_explicit_height_split(
    start_xz: RoadVec2,
    end_xz: RoadVec2,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> bool {
    seam_constraints.iter().any(|constraint| {
        explicit_height_split_constraint(constraint)
            && point_lies_on_height_segment(start_xz, constraint.start_xz, constraint.end_xz)
            && point_lies_on_height_segment(end_xz, constraint.start_xz, constraint.end_xz)
    })
}

pub(super) fn explicit_height_split_constraints_for_vertex(
    point_xz: RoadVec2,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> Vec<&NodeRegionSeamConstraint> {
    let mut matches = seam_constraints
        .iter()
        .filter(|constraint| explicit_height_split_constraint(constraint))
        .filter(|constraint| {
            point_lies_on_height_segment(point_xz, constraint.start_xz, constraint.end_xz)
        })
        .collect::<Vec<_>>();
    matches.sort_unstable_by_key(|constraint| {
        (
            constraint.priority_key(),
            SurfaceXzKey::from_road_xz(constraint.start_xz),
            SurfaceXzKey::from_road_xz(constraint.end_xz),
        )
    });
    matches.dedup_by_key(|constraint| {
        (
            constraint.constraint_index,
            constraint.owner,
            constraint.opposite_owner,
            SurfaceXzKey::from_road_xz(constraint.start_xz),
            SurfaceXzKey::from_road_xz(constraint.end_xz),
        )
    });
    matches
}

fn explicit_height_split_constraint(constraint: &NodeRegionSeamConstraint) -> bool {
    if !constraint.is_material_transition || constraint.constrains_shared_height {
        return false;
    }
    if matches!(
        constraint.seam_source,
        NodeSeamSource::RaisedStepContact { .. }
    ) {
        return true;
    }
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(owner), Some(opposite_owner)) if owner.kind() == opposite_owner.kind()
    )
}

fn point_lies_on_height_segment(point: RoadVec2, start: RoadVec2, end: RoadVec2) -> bool {
    key_lies_on_segment(
        SurfaceXzKey::from_road_xz(quantize_road_vec2_to_overlay_grid(point)),
        SurfaceXzKey::from_road_xz(quantize_road_vec2_to_overlay_grid(start)),
        SurfaceXzKey::from_road_xz(quantize_road_vec2_to_overlay_grid(end)),
    )
}

pub(super) fn push_unique_same_material_candidate<K: Ord>(
    candidates_by_key: &mut BTreeMap<K, Vec<SameMaterialVertexHeightCandidate>>,
    key: K,
    candidate: SameMaterialVertexHeightCandidate,
) {
    let candidates = candidates_by_key.entry(key).or_default();
    let context = SameMaterialVertexHeightContext::from_candidate(candidate);
    if candidates
        .iter()
        .copied()
        .map(SameMaterialVertexHeightContext::from_candidate)
        .any(|existing| existing == context)
    {
        return;
    }
    candidates.push(candidate);
}

pub(super) fn same_height_selected_candidate(
    candidates: &[SameMaterialVertexHeightCandidate],
) -> Option<SameMaterialVertexHeightCandidate> {
    let first = candidates.first().copied()?;
    let height_key = SurfaceHeightMmKey::from_m_f64(first.height_m);
    if candidates
        .iter()
        .copied()
        .all(|candidate| SurfaceHeightMmKey::from_m_f64(candidate.height_m) == height_key)
    {
        candidates
            .iter()
            .copied()
            .min_by_key(|candidate| same_material_vertex_height_candidate_key(*candidate))
    } else {
        None
    }
}

pub(super) fn reject_same_material_height_conflict(
    kind: RoadSurfaceBandKind,
    point: SurfaceXzKey,
    candidates: impl IntoIterator<Item = SameMaterialVertexHeightCandidate>,
) -> Result<(), NodeHeightFieldError> {
    let mut candidates_by_height = BTreeMap::<i64, SameMaterialVertexHeightCandidate>::new();
    for candidate in candidates {
        let height_mm = SurfaceHeightMmKey::from_m_f64(candidate.height_m).as_i64();
        candidates_by_height
            .entry(height_mm)
            .and_modify(|selected| {
                if same_material_vertex_height_candidate_key(candidate)
                    < same_material_vertex_height_candidate_key(*selected)
                {
                    *selected = candidate;
                }
            })
            .or_insert(candidate);
    }
    if candidates_by_height.len() < 2 {
        return Ok(());
    }

    let mut ordered = candidates_by_height
        .into_iter()
        .map(|(height_mm, candidate)| {
            (
                same_material_vertex_height_candidate_key(candidate),
                height_mm,
                candidate,
            )
        })
        .collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|(candidate_key, height_mm, _)| (*candidate_key, *height_mm));
    let (_, existing_height_mm, existing) = ordered[0];
    let (_, incoming_height_mm, incoming) = ordered
        .into_iter()
        .find(|(_, height_mm, _)| *height_mm != existing_height_mm)
        .expect("same-material conflict has at least two distinct height keys");
    Err(NodeHeightFieldError::SharedSourceHeightConflict {
        point_x_mm: point.x_mm(),
        point_z_mm: point.z_mm(),
        kind,
        owner: existing.owner,
        opposite_owner: None,
        height_field_id: Some(existing.height_field_id),
        incoming_owner: incoming.owner,
        incoming_height_field_id: Some(incoming.height_field_id),
        constraint_index: None,
        existing_authority: existing.height_authority,
        incoming_authority: incoming.height_authority,
        existing_height_mm,
        incoming_height_mm,
    })
}

pub(super) fn set_vertex_grade_height(
    owner: NodeBandOwner,
    vertex: &mut NodeHeightedVertex,
    height_m: f64,
    decision: NodeGradeCarrierDecision,
) {
    let height_m = canonical_height_m(height_m);
    vertex.height_m = height_m;
    vertex.grade_authority = Some(NodeGradeVertexAuthority::new(
        vertex.point_xz,
        height_m,
        owner,
        vertex.height_field_id,
        decision,
    ));
}

fn canonical_height_m(height_m: f64) -> f64 {
    SurfaceHeightMmKey::from_m_f64(height_m).as_i64() as f64 / SURFACE_MM_PER_M
}
