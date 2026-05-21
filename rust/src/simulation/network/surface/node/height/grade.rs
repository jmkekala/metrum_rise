//! Node-height authority agreement for canonical owned node vertices.

use super::super::RoadSurfaceBandKind;
use super::super::arrangement::{
    NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource,
};
use super::super::backend::RoadVec2;
use super::super::keys::{SURFACE_MM_PER_M, SurfaceHeightMmKey, SurfaceXzKey};
use super::super::segments::road_xz_lies_exactly_on_segment;
use super::model::{
    NodeHeightAuthoritySource, NodeHeightFieldError, NodeHeightedRegion, NodeHeightedVertex,
};
use std::collections::BTreeMap;

mod seams;
mod shared_edges;
mod shared_vertices;

use seams::{
    apply_junctionn_explicit_material_seam_height_normalization,
    apply_junctionn_same_material_seam_height_normalization,
};
use shared_edges::apply_junctionn_same_material_shared_edge_height_normalization;
use shared_vertices::{
    apply_junctionn_same_material_vertex_height_normalization,
    apply_junctionn_same_owner_canonical_vertex_height_normalization,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum NodeGradeCarrierDecision {
    SourceCarrier {
        authority: Option<NodeHeightAuthoritySource>,
    },
    SameOwnerCanonicalVertex,
    SameMaterialSharedEdge,
    SameMaterialVertex,
    SameMaterialSeam,
    ExplicitMaterialSeam,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeGradeVertexAuthority {
    pub(crate) key: SurfaceXzKey,
    pub(crate) owner: NodeBandOwner,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) height_key: SurfaceHeightMmKey,
    pub(crate) decision: NodeGradeCarrierDecision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeGradeExplicitSeamHeightKey {
    point: SurfaceXzKey,
    constraint_index: usize,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
}

#[derive(Clone, Copy)]
struct SameMaterialVertexHeightCandidate {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    height_m: f64,
    height_authority: Option<NodeHeightAuthoritySource>,
    has_explicit_shared_material_seam: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialSharedEdgeKey {
    kind: RoadSurfaceBandKind,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SameMaterialSharedEdgeCandidate {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    start: SurfaceXzKey,
    start_height_m: f64,
    start_height_authority: Option<NodeHeightAuthoritySource>,
    start_has_explicit_shared_material_seam: bool,
    end: SurfaceXzKey,
    end_height_m: f64,
    end_height_authority: Option<NodeHeightAuthoritySource>,
    end_has_explicit_shared_material_seam: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialSharedVertexKey {
    kind: RoadSurfaceBandKind,
    point: SurfaceXzKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialSharedVertexContext {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialVertexHeightContext {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    height_mm: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialVertexHeightSupportKey {
    kind: RoadSurfaceBandKind,
    point: SurfaceXzKey,
    explicit_seams: Vec<NodeGradeExplicitSeamHeightKey>,
    explicit_height_splits: Vec<(NodeBandOwner, NodeGradeExplicitSeamHeightKey)>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeGradeVertexContextKey {
    point: SurfaceXzKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

struct SameMaterialSharedEdgeHeightAgreement {
    selected_by_vertex: BTreeMap<SameMaterialSharedVertexKey, SameMaterialVertexHeightCandidate>,
    affected_contexts_by_vertex:
        BTreeMap<SameMaterialSharedVertexKey, Vec<SameMaterialSharedVertexContext>>,
}

struct SameMaterialVertexHeightGroups {
    contexts_by_key:
        BTreeMap<SameMaterialVertexHeightSupportKey, Vec<SameMaterialVertexHeightContext>>,
    candidates_by_key:
        BTreeMap<SameMaterialVertexHeightSupportKey, Vec<SameMaterialVertexHeightCandidate>>,
    selected_by_key:
        BTreeMap<SameMaterialVertexHeightSupportKey, SameMaterialVertexHeightCandidate>,
}

impl NodeGradeVertexAuthority {
    pub(crate) fn new(
        point_xz: RoadVec2,
        height_m: f64,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        decision: NodeGradeCarrierDecision,
    ) -> Self {
        Self {
            key: SurfaceXzKey::from_road_xz(point_xz),
            owner,
            height_field_id,
            height_key: SurfaceHeightMmKey::from_m_f64(height_m),
            decision,
        }
    }
}

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
    fn new(
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
        if end_key < start_key {
            std::mem::swap(&mut start_key, &mut end_key);
            std::mem::swap(&mut start_height_m, &mut end_height_m);
            std::mem::swap(&mut start_height_authority, &mut end_height_authority);
            std::mem::swap(
                &mut start_has_explicit_shared_material_seam,
                &mut end_has_explicit_shared_material_seam,
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
                end: end_key,
                end_height_m,
                end_height_authority,
                end_has_explicit_shared_material_seam,
            },
        ))
    }

    fn endpoint_candidate(self, point: SurfaceXzKey) -> SameMaterialVertexHeightCandidate {
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
}

impl SameMaterialVertexHeightContext {
    fn from_candidate(candidate: SameMaterialVertexHeightCandidate) -> Self {
        Self {
            owner: candidate.owner,
            height_field_id: candidate.height_field_id,
            height_mm: SurfaceHeightMmKey::from_m_f64(candidate.height_m).as_i64(),
        }
    }
}

pub(crate) fn apply_junctionn_height_authority_normalization(
    regions: &mut [NodeHeightedRegion],
) -> Result<(), NodeHeightFieldError> {
    apply_junctionn_same_owner_canonical_vertex_height_normalization(regions);
    apply_junctionn_same_material_shared_edge_height_normalization(regions)?;
    apply_junctionn_same_material_vertex_height_normalization(regions)?;
    apply_junctionn_same_material_seam_height_normalization(regions);
    apply_junctionn_explicit_material_seam_height_normalization(regions);
    Ok(())
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

fn same_material_vertex_height_candidate_key(
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

fn vertex_has_explicit_shared_material_seam(
    vertex: &NodeHeightedVertex,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> bool {
    material_height_constraints_for_vertex(vertex.point_xz, seam_constraints)
        .into_iter()
        .any(|constraint| constraint.is_material_transition)
}

fn edge_has_explicit_height_split(
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

fn explicit_height_split_constraints_for_vertex(
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
    constraint.is_material_transition
        && !constraint.constrains_shared_height
        && matches!(
            constraint.seam_source,
            NodeSeamSource::RaisedStepContact { .. }
        )
}

fn point_lies_on_height_segment(point: RoadVec2, start: RoadVec2, end: RoadVec2) -> bool {
    road_xz_lies_exactly_on_segment(point, start, end)
}

fn push_unique_same_material_candidate<K: Ord>(
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

fn same_height_selected_candidate(
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

fn reject_same_material_height_conflict(
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

fn set_vertex_grade_height(
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

#[cfg(test)]
mod tests {
    use super::super::super::RoadSurfaceBandKind;
    use super::super::super::arrangement::NodeBandHeightFieldId;
    use super::super::super::backend::RoadVec2;
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

    fn manual_region(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        height_m: f64,
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
                height_authority: Some(NodeHeightAuthoritySource::SourceInterval),
                grade_authority: Some(NodeGradeVertexAuthority::new(
                    RoadVec2::new(-1.0, 0.0),
                    height_m,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier {
                        authority: Some(NodeHeightAuthoritySource::SourceInterval),
                    },
                )),
            }]],
            area_m2: 1.0,
            seam_constraints: Vec::new(),
        }
    }
}
