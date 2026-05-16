//! Node-local grade-carrier decisions for canonical owned node vertices.

use super::RoadSurfaceBandKind;
use super::arrangement::{
    NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint, seam_source_priority,
};
use super::backend::RoadVec2;
use super::height::{NodeHeightAuthoritySource, NodeHeightedRegion, NodeHeightedVertex};
use super::keys::{SURFACE_CANONICAL_HEIGHT_EPS_M, SurfaceHeightMmKey, SurfaceXzKey};
use super::segments::road_xz_lies_exactly_on_segment;
use std::collections::BTreeMap;

const SAME_MATERIAL_SHARED_EDGE_HEIGHT_CANONICAL_EPS_M: f64 = SURFACE_CANONICAL_HEIGHT_EPS_M;
const EXPLICIT_MATERIAL_SEAM_HEIGHT_CANONICAL_EPS_M: f64 = SURFACE_CANONICAL_HEIGHT_EPS_M;
const JUNCTIONN_SAME_MATERIAL_SEAM_BLEND_LIMIT_M: f64 = 0.25;
const JUNCTIONN_UNCONSTRAINED_SEAM_ADOPTION_LIMIT_M: f64 = 2.0;

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
    ExplicitMaterialSeamAdoption,
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
struct SameMaterialVertexHeightTieKey {
    kind: RoadSurfaceBandKind,
    point: SurfaceXzKey,
    explicit_seams: Vec<NodeGradeExplicitSeamHeightKey>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeGradeVertexContextKey {
    point: SurfaceXzKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
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

pub(crate) fn apply_junctionn_node_grade_carrier(regions: &mut [NodeHeightedRegion]) {
    apply_junctionn_same_owner_height_field_vertex_unification(regions);
    apply_junctionn_same_material_shared_edge_height_tiebreak(regions);
    apply_junctionn_same_material_vertex_height_tiebreak(regions);
    apply_junctionn_same_material_seam_height_unification(regions);
    apply_junctionn_explicit_material_seam_height_unification(regions);
    apply_junctionn_explicit_material_seam_height_to_unconstrained_same_material_vertices(regions);
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
            !constraint.is_material_transition,
            seam_source_priority(&constraint.seam_source),
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

fn apply_junctionn_same_owner_height_field_vertex_unification(regions: &mut [NodeHeightedRegion]) {
    let mut heights_by_key =
        BTreeMap::<NodeGradeVertexContextKey, SameMaterialVertexHeightCandidate>::new();
    let mut distinct_heights_by_key = BTreeMap::<NodeGradeVertexContextKey, Vec<i64>>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let key = NodeGradeVertexContextKey {
                point: SurfaceXzKey::from_road_xz(vertex.point_xz),
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            heights_by_key
                .entry(key.clone())
                .and_modify(|selected| {
                    let candidate = SameMaterialVertexHeightCandidate {
                        owner: region.owner,
                        height_field_id: vertex.height_field_id,
                        height_m: vertex.height_m,
                        height_authority: vertex.height_authority,
                        has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                            vertex,
                            &region.seam_constraints,
                        ),
                    };
                    if same_material_vertex_height_candidate_key(candidate)
                        < same_material_vertex_height_candidate_key(*selected)
                    {
                        *selected = candidate;
                    }
                })
                .or_insert(SameMaterialVertexHeightCandidate {
                    owner: region.owner,
                    height_field_id: vertex.height_field_id,
                    height_m: vertex.height_m,
                    height_authority: vertex.height_authority,
                    has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                        vertex,
                        &region.seam_constraints,
                    ),
                });
            let heights = distinct_heights_by_key.entry(key).or_default();
            let height_mm = SurfaceHeightMmKey::from_m_f64(vertex.height_m).as_i64();
            if !heights.contains(&height_mm) {
                heights.push(height_mm);
            }
        }
    }

    for region in regions {
        let owner = region.owner;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key = NodeGradeVertexContextKey {
                point: SurfaceXzKey::from_road_xz(vertex.point_xz),
                owner,
                height_field_id: vertex.height_field_id,
            };
            if distinct_heights_by_key
                .get(&key)
                .is_none_or(|heights| heights.len() < 2)
            {
                continue;
            }
            if let Some(selected) = heights_by_key.get(&key) {
                set_vertex_grade_height(
                    owner,
                    vertex,
                    selected.height_m,
                    NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
                );
            }
        }
    }
}

fn apply_junctionn_same_material_shared_edge_height_tiebreak(regions: &mut [NodeHeightedRegion]) {
    let mut candidates_by_edge =
        BTreeMap::<SameMaterialSharedEdgeKey, Vec<SameMaterialSharedEdgeCandidate>>::new();

    for region in regions.iter() {
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

    let mut selected_by_vertex =
        BTreeMap::<SameMaterialSharedVertexKey, SameMaterialVertexHeightCandidate>::new();
    let mut affected_contexts_by_vertex =
        BTreeMap::<SameMaterialSharedVertexKey, Vec<SameMaterialSharedVertexContext>>::new();

    for (edge, candidates) in candidates_by_edge {
        if candidates.len() < 2 {
            continue;
        }
        if !same_material_shared_edge_candidates_are_canonical_drift(&candidates) {
            continue;
        }
        for endpoint in [edge.start, edge.end] {
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
            let Some(contexts) = affected_contexts_by_vertex.get(&key) else {
                continue;
            };
            let context = SameMaterialSharedVertexContext {
                owner,
                height_field_id: vertex.height_field_id,
            };
            if !contexts.contains(&context) {
                continue;
            }
            if let Some(selected) = selected_by_vertex.get(&key) {
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

fn same_material_shared_edge_candidates_are_canonical_drift(
    candidates: &[SameMaterialSharedEdgeCandidate],
) -> bool {
    let mut start_min = f64::INFINITY;
    let mut start_max = f64::NEG_INFINITY;
    let mut end_min = f64::INFINITY;
    let mut end_max = f64::NEG_INFINITY;
    for candidate in candidates {
        start_min = start_min.min(candidate.start_height_m);
        start_max = start_max.max(candidate.start_height_m);
        end_min = end_min.min(candidate.end_height_m);
        end_max = end_max.max(candidate.end_height_m);
    }
    start_max - start_min <= SAME_MATERIAL_SHARED_EDGE_HEIGHT_CANONICAL_EPS_M
        && end_max - end_min <= SAME_MATERIAL_SHARED_EDGE_HEIGHT_CANONICAL_EPS_M
}

fn apply_junctionn_same_material_vertex_height_tiebreak(regions: &mut [NodeHeightedRegion]) {
    let mut contexts_by_key =
        BTreeMap::<SameMaterialVertexHeightTieKey, Vec<SameMaterialVertexHeightContext>>::new();
    let mut selected_by_key =
        BTreeMap::<SameMaterialVertexHeightTieKey, SameMaterialVertexHeightCandidate>::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let key = same_material_vertex_height_tie_key_from_parts(
                region.kind,
                &region.seam_constraints,
                vertex,
            );
            let candidate = SameMaterialVertexHeightCandidate {
                owner: region.owner,
                height_field_id: vertex.height_field_id,
                height_m: vertex.height_m,
                height_authority: vertex.height_authority,
                has_explicit_shared_material_seam: vertex_has_explicit_shared_material_seam(
                    vertex,
                    &region.seam_constraints,
                ),
            };
            let contexts = contexts_by_key.entry(key.clone()).or_default();
            let context = SameMaterialVertexHeightContext::from_candidate(candidate);
            if !contexts.contains(&context) {
                contexts.push(context);
                contexts.sort_unstable();
            }
            selected_by_key
                .entry(key)
                .and_modify(|selected| {
                    if same_material_vertex_height_candidate_key(candidate)
                        < same_material_vertex_height_candidate_key(*selected)
                    {
                        *selected = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }

    for region in regions {
        let owner = region.owner;
        let kind = region.kind;
        let seam_constraints = &region.seam_constraints;
        for vertex in region
            .shape
            .iter_mut()
            .flat_map(|contour| contour.iter_mut())
        {
            let key =
                same_material_vertex_height_tie_key_from_parts(kind, seam_constraints, vertex);
            if contexts_by_key
                .get(&key)
                .is_none_or(|contexts| contexts.len() < 2)
            {
                continue;
            }
            if let Some(selected) = selected_by_key.get(&key) {
                set_vertex_grade_height(
                    owner,
                    vertex,
                    selected.height_m,
                    NodeGradeCarrierDecision::SameMaterialVertex,
                );
            }
        }
    }
}

fn same_material_vertex_height_tie_key_from_parts(
    kind: RoadSurfaceBandKind,
    seam_constraints: &[NodeRegionSeamConstraint],
    vertex: &NodeHeightedVertex,
) -> SameMaterialVertexHeightTieKey {
    let point = SurfaceXzKey::from_road_xz(vertex.point_xz);
    let mut explicit_seams =
        material_height_constraints_for_vertex(vertex.point_xz, seam_constraints)
            .into_iter()
            .map(|constraint| NodeGradeExplicitSeamHeightKey::new(point, constraint))
            .collect::<Vec<_>>();
    explicit_seams.sort_unstable();
    explicit_seams.dedup();
    SameMaterialVertexHeightTieKey {
        kind,
        point,
        explicit_seams,
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

fn apply_junctionn_same_material_seam_height_unification(regions: &mut [NodeHeightedRegion]) {
    let mut ranges_by_key = BTreeMap::<
        NodeGradeExplicitSeamHeightKey,
        (f64, f64, SameMaterialVertexHeightCandidate),
    >::new();

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
                ranges_by_key
                    .entry(key)
                    .and_modify(|(min_height, max_height, selected)| {
                        *min_height = min_height.min(vertex.height_m);
                        *max_height = max_height.max(vertex.height_m);
                        if same_material_vertex_height_candidate_key(candidate)
                            < same_material_vertex_height_candidate_key(*selected)
                        {
                            *selected = candidate;
                        }
                    })
                    .or_insert((vertex.height_m, vertex.height_m, candidate));
            }
        }
    }

    let selected_by_key = ranges_by_key
        .into_iter()
        .filter_map(|(key, (min_height, max_height, selected))| {
            (max_height - min_height <= JUNCTIONN_SAME_MATERIAL_SEAM_BLEND_LIMIT_M)
                .then_some((key, selected))
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
}

fn apply_junctionn_explicit_material_seam_height_unification(regions: &mut [NodeHeightedRegion]) {
    let mut ranges_by_key = BTreeMap::<
        NodeGradeExplicitSeamHeightKey,
        (f64, f64, SameMaterialVertexHeightCandidate),
    >::new();

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
                ranges_by_key
                    .entry(key)
                    .and_modify(|(min_height, max_height, selected)| {
                        *min_height = min_height.min(vertex.height_m);
                        *max_height = max_height.max(vertex.height_m);
                        if same_material_vertex_height_candidate_key(candidate)
                            < same_material_vertex_height_candidate_key(*selected)
                        {
                            *selected = candidate;
                        }
                    })
                    .or_insert((vertex.height_m, vertex.height_m, candidate));
            }
        }
    }

    let selected_by_key = ranges_by_key
        .into_iter()
        .filter_map(|(key, (min_height, max_height, selected))| {
            (max_height - min_height <= EXPLICIT_MATERIAL_SEAM_HEIGHT_CANONICAL_EPS_M)
                .then_some((key, selected.height_m))
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

fn vertex_has_explicit_shared_material_seam(
    vertex: &NodeHeightedVertex,
    seam_constraints: &[NodeRegionSeamConstraint],
) -> bool {
    material_height_constraints_for_vertex(vertex.point_xz, seam_constraints)
        .into_iter()
        .any(|constraint| constraint.is_material_transition)
}

fn apply_junctionn_explicit_material_seam_height_to_unconstrained_same_material_vertices(
    regions: &mut [NodeHeightedRegion],
) {
    let mut explicit_by_key = BTreeMap::<
        SameMaterialSharedVertexKey,
        (f64, f64, SameMaterialVertexHeightCandidate),
    >::new();

    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            if !vertex_has_explicit_shared_material_seam(vertex, &region.seam_constraints) {
                continue;
            }
            let key = SameMaterialSharedVertexKey {
                kind: region.kind,
                point: SurfaceXzKey::from_road_xz(vertex.point_xz),
            };
            let candidate = SameMaterialVertexHeightCandidate {
                owner: region.owner,
                height_field_id: vertex.height_field_id,
                height_m: vertex.height_m,
                height_authority: vertex.height_authority,
                has_explicit_shared_material_seam: true,
            };
            explicit_by_key
                .entry(key)
                .and_modify(|(min_height, max_height, selected)| {
                    *min_height = min_height.min(vertex.height_m);
                    *max_height = max_height.max(vertex.height_m);
                    if same_material_vertex_height_candidate_key(candidate)
                        < same_material_vertex_height_candidate_key(*selected)
                    {
                        *selected = candidate;
                    }
                })
                .or_insert((vertex.height_m, vertex.height_m, candidate));
        }
    }

    let selected_by_key = explicit_by_key
        .into_iter()
        .filter_map(|(key, (min_height, max_height, selected))| {
            (max_height - min_height <= EXPLICIT_MATERIAL_SEAM_HEIGHT_CANONICAL_EPS_M)
                .then_some((key, selected.height_m))
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
            if vertex_has_explicit_shared_material_seam(vertex, &region.seam_constraints) {
                continue;
            }
            let key = SameMaterialSharedVertexKey {
                kind: region.kind,
                point: SurfaceXzKey::from_road_xz(vertex.point_xz),
            };
            if let Some(height_m) = selected_by_key.get(&key)
                && (*height_m - vertex.height_m).abs()
                    <= JUNCTIONN_UNCONSTRAINED_SEAM_ADOPTION_LIMIT_M
            {
                set_vertex_grade_height(
                    owner,
                    vertex,
                    *height_m,
                    NodeGradeCarrierDecision::ExplicitMaterialSeamAdoption,
                );
            }
        }
    }
}

fn point_lies_on_height_segment(point: RoadVec2, start: RoadVec2, end: RoadVec2) -> bool {
    road_xz_lies_exactly_on_segment(point, start, end)
}

fn set_vertex_grade_height(
    owner: NodeBandOwner,
    vertex: &mut NodeHeightedVertex,
    height_m: f64,
    decision: NodeGradeCarrierDecision,
) {
    vertex.height_m = height_m;
    vertex.grade_authority = Some(NodeGradeVertexAuthority::new(
        vertex.point_xz,
        height_m,
        owner,
        vertex.height_field_id,
        decision,
    ));
}

#[cfg(test)]
mod tests {
    use super::super::RoadSurfaceBandKind;
    use super::super::arrangement::NodeBandHeightFieldId;
    use super::super::backend::RoadVec2;
    use super::super::height::{NodeHeightedRegion, NodeHeightedVertex};
    use super::*;

    #[test]
    fn carrier_records_same_material_vertex_decision() {
        let mut regions = vec![
            manual_region(RoadSurfaceBandKind::Carriageway, 9, 2.0),
            manual_region(RoadSurfaceBandKind::Carriageway, 14, 1.0),
            manual_region(RoadSurfaceBandKind::Sidewalk, 1, 3.0),
        ];

        apply_junctionn_node_grade_carrier(&mut regions);

        let adopted = &regions[1].shape[0][0];
        assert_eq!(adopted.height_m, 2.0);
        assert_eq!(
            adopted
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
