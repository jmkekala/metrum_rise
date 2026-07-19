//! Immutable reuse of post-triangulation explicit vertical-step topology.

use super::extraction::{
    NodeArrangementEdgeGeometry, NodeArrangementOwnedBoundaryEdgeGeometry,
    arrangement_edge_overlap_segment, arrangement_edges_have_positive_raised_step_delta,
    explicit_segments_cover_owned_boundary_step, seam_constraint_authorizes_explicit_height_split,
};
use super::*;
use crate::simulation::network::surface::{
    RoadSurfaceBandKind,
    indices::{SurfaceKeyBounds, SurfaceKeyTile},
};
use std::sync::Arc;

const FINAL_STEP_OVERLAY_GRID_BOUNDS_PADDING_KEYS: i64 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FinalStepAuthoritySpan {
    owners: (NodeBandOwner, NodeBandOwner),
    start: NodeArrangementKey,
    end: NodeArrangementKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FinalStepBoundaryEdgeReuseKey {
    owner: NodeBandOwner,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
    start_height_mm: i64,
    end_height_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FinalStepBoundaryPairReuseKey {
    left: FinalStepBoundaryEdgeReuseKey,
    right: FinalStepBoundaryEdgeReuseKey,
}

#[derive(Clone, Copy, Debug)]
struct FinalStepBoundaryEdgeCandidate {
    edge: FinalStepBoundaryEdgeReuseKey,
    bounds: SurfaceKeyBounds,
}

#[derive(Clone, Debug, Default)]
struct FinalStepBoundaryEdgeIndex {
    edges_by_kind: BTreeMap<RoadSurfaceBandKind, Vec<FinalStepBoundaryEdgeCandidate>>,
    tile_indices_by_kind: BTreeMap<RoadSurfaceBandKind, BTreeMap<SurfaceKeyTile, Vec<usize>>>,
}

/// Exact post-attach step contributors reusable by a later node export generation.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeFinalExplicitStepTopologyCache {
    base_segments: Arc<[NodeExplicitVerticalStepSegment]>,
    boundary_edges: Arc<BTreeMap<FinalStepBoundaryEdgeReuseKey, Arc<[FinalStepAuthoritySpan]>>>,
    positive_pair_candidates:
        Arc<BTreeMap<FinalStepBoundaryPairReuseKey, NodeExplicitVerticalStepSegment>>,
    final_segments: Arc<[NodeExplicitVerticalStepSegment]>,
}

/// Reuse activity for one post-attach explicit-step topology build.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NodeFinalExplicitStepTopologyReuseStats {
    /// Complete final step topology products replayed from the previous generation.
    pub(crate) previous_hits: usize,
    /// Complete final step topology products rebuilt for the current generation.
    pub(crate) misses: usize,
    /// Positive unchanged boundary-pair candidates retained from the previous generation.
    pub(crate) pair_previous_hits: usize,
    /// Boundary pairs evaluated because at least one contributor changed.
    pub(crate) pair_misses: usize,
}

impl NodeArrangement {
    /// Finalizes pre-attach step topology against attached face boundaries with exact reuse.
    pub(crate) fn final_explicit_vertical_step_segments_with_reuse(
        &self,
        base_segments: &[NodeExplicitVerticalStepSegment],
        previous: Option<&NodeFinalExplicitStepTopologyCache>,
    ) -> (
        Vec<NodeExplicitVerticalStepSegment>,
        NodeFinalExplicitStepTopologyCache,
        NodeFinalExplicitStepTopologyReuseStats,
    ) {
        let mut base_segments = base_segments.to_vec();
        base_segments.sort_unstable();
        base_segments.dedup();
        let base_segments: Arc<[NodeExplicitVerticalStepSegment]> = Arc::from(base_segments);

        let authority_spans_by_region_and_owner = self
            .regions
            .iter()
            .map(|region| {
                let mut authority_spans =
                    BTreeMap::<NodeBandOwner, Vec<FinalStepAuthoritySpan>>::new();
                for constraint in region.seam_constraints.iter().filter(|constraint| {
                    seam_constraint_authorizes_explicit_height_split(constraint)
                }) {
                    let (Some(left), Some(right)) = (constraint.owner, constraint.opposite_owner)
                    else {
                        continue;
                    };
                    let span = FinalStepAuthoritySpan {
                        owners: ordered_owner_pair(left, right),
                        start: NodeArrangementKey::from_point(constraint.start_xz),
                        end: NodeArrangementKey::from_point(constraint.end_xz),
                    };
                    let (start, end) = normalized_key_pair(span.start, span.end);
                    let span = FinalStepAuthoritySpan { start, end, ..span };
                    authority_spans.entry(left).or_default().push(span);
                    if right != left {
                        authority_spans.entry(right).or_default().push(span);
                    }
                }
                authority_spans
                    .into_iter()
                    .map(|(owner, mut spans)| {
                        spans.sort_unstable();
                        spans.dedup();
                        (owner, Arc::<[FinalStepAuthoritySpan]>::from(spans))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .collect::<Vec<_>>();
        let empty_authority_spans = Arc::<[FinalStepAuthoritySpan]>::from([]);
        let boundary_edges = self
            .final_owned_boundary_edge_geometries()
            .into_iter()
            .filter_map(|edge| {
                let authority_spans_by_owner =
                    authority_spans_by_region_and_owner.get(edge.region.0)?;
                let edge_key = FinalStepBoundaryEdgeReuseKey::from_edge(edge);
                let edge_bounds = edge_key.bounds();
                let relevant_spans = authority_spans_by_owner
                    .get(&edge.owner)
                    .into_iter()
                    .flat_map(|spans| spans.iter().copied())
                    .filter(|span| span.bounds().overlaps(edge_bounds))
                    .collect::<Vec<_>>();
                let authority_spans = if relevant_spans.is_empty() {
                    Arc::clone(&empty_authority_spans)
                } else {
                    Arc::from(relevant_spans)
                };
                Some((edge_key, authority_spans))
            })
            .collect::<BTreeMap<_, _>>();

        if let Some(previous) = previous
            && previous.base_segments.as_ref() == base_segments.as_ref()
            && previous.boundary_edges.as_ref() == &boundary_edges
        {
            let stats = NodeFinalExplicitStepTopologyReuseStats {
                previous_hits: 1,
                ..Default::default()
            };
            return (
                previous.final_segments.as_ref().to_vec(),
                previous.clone(),
                stats,
            );
        }

        let mut stats = NodeFinalExplicitStepTopologyReuseStats {
            misses: 1,
            ..Default::default()
        };
        let unchanged_edges = previous
            .map(|previous| {
                unchanged_final_step_boundary_edges(
                    previous.boundary_edges.as_ref(),
                    &boundary_edges,
                )
            })
            .unwrap_or_default();
        let mut positive_pair_candidates = previous
            .map(|previous| {
                previous
                    .positive_pair_candidates
                    .iter()
                    .filter(|(pair, _)| {
                        unchanged_edges.contains(&pair.left)
                            && unchanged_edges.contains(&pair.right)
                    })
                    .map(|(pair, segment)| (*pair, *segment))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        stats.pair_previous_hits = positive_pair_candidates.len();

        let new_edges = boundary_edges
            .keys()
            .filter(|edge| !unchanged_edges.contains(edge))
            .copied()
            .collect::<BTreeSet<_>>();
        let boundary_edge_index = FinalStepBoundaryEdgeIndex::new(boundary_edges.keys().copied());
        let mut overlap_candidates = Vec::new();
        let mut candidate_indices = Vec::new();
        for new_key in &new_edges {
            boundary_edge_index.overlap_candidates(
                *new_key,
                &mut overlap_candidates,
                &mut candidate_indices,
            );
            for other_key in overlap_candidates.iter().copied() {
                if *new_key == other_key || (new_edges.contains(&other_key) && other_key < *new_key)
                {
                    continue;
                }
                let pair = FinalStepBoundaryPairReuseKey::new(*new_key, other_key);
                stats.pair_misses += 1;
                if let Some(segment) = authorized_final_boundary_raised_step_overlap_segment(
                    pair.left,
                    boundary_edges[&pair.left].as_ref(),
                    pair.right,
                    boundary_edges[&pair.right].as_ref(),
                ) {
                    positive_pair_candidates.insert(pair, segment);
                }
            }
        }

        let base_segment_set = base_segments.iter().copied().collect::<BTreeSet<_>>();
        let mut final_segments = base_segment_set.clone();
        final_segments.extend(
            positive_pair_candidates
                .values()
                .copied()
                .filter(|candidate| {
                    !explicit_segments_cover_owned_boundary_step(&base_segment_set, *candidate)
                }),
        );
        let final_segments: Arc<[NodeExplicitVerticalStepSegment]> =
            Arc::from(final_segments.into_iter().collect::<Vec<_>>());
        let current = NodeFinalExplicitStepTopologyCache {
            base_segments,
            boundary_edges: Arc::new(boundary_edges),
            positive_pair_candidates: Arc::new(positive_pair_candidates),
            final_segments: Arc::clone(&final_segments),
        };
        (final_segments.as_ref().to_vec(), current, stats)
    }
}

impl FinalStepBoundaryEdgeReuseKey {
    fn from_edge(edge: NodeArrangementOwnedBoundaryEdgeGeometry) -> Self {
        let (start, end, start_height_mm, end_height_mm) = if edge.start <= edge.end {
            (
                edge.start,
                edge.end,
                edge.start_height_mm,
                edge.end_height_mm,
            )
        } else {
            (
                edge.end,
                edge.start,
                edge.end_height_mm,
                edge.start_height_mm,
            )
        };
        Self {
            owner: edge.owner,
            start,
            end,
            start_height_mm,
            end_height_mm,
        }
    }

    fn geometry(&self) -> NodeArrangementEdgeGeometry {
        NodeArrangementEdgeGeometry {
            start: self.start,
            end: self.end,
            start_height_mm: self.start_height_mm,
            end_height_mm: self.end_height_mm,
        }
    }

    fn bounds(self) -> SurfaceKeyBounds {
        SurfaceKeyBounds::from_segment(self.start.surface_key(), self.end.surface_key())
            .expanded(FINAL_STEP_OVERLAY_GRID_BOUNDS_PADDING_KEYS)
    }

    fn authorizes_overlap_with(
        &self,
        authority_spans: &[FinalStepAuthoritySpan],
        opposite_owner: NodeBandOwner,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    ) -> bool {
        let owners = ordered_owner_pair(self.owner, opposite_owner);
        authority_spans.iter().any(|span| {
            span.owners == owners
                && start.lies_on_segment(span.start, span.end)
                && end.lies_on_segment(span.start, span.end)
        })
    }
}

impl FinalStepAuthoritySpan {
    fn bounds(self) -> SurfaceKeyBounds {
        SurfaceKeyBounds::from_segment(self.start.surface_key(), self.end.surface_key())
            .expanded(FINAL_STEP_OVERLAY_GRID_BOUNDS_PADDING_KEYS)
    }
}

impl FinalStepBoundaryPairReuseKey {
    fn new(left: FinalStepBoundaryEdgeReuseKey, right: FinalStepBoundaryEdgeReuseKey) -> Self {
        if left <= right {
            Self { left, right }
        } else {
            Self {
                left: right,
                right: left,
            }
        }
    }
}

fn unchanged_final_step_boundary_edges(
    previous: &BTreeMap<FinalStepBoundaryEdgeReuseKey, Arc<[FinalStepAuthoritySpan]>>,
    current: &BTreeMap<FinalStepBoundaryEdgeReuseKey, Arc<[FinalStepAuthoritySpan]>>,
) -> BTreeSet<FinalStepBoundaryEdgeReuseKey> {
    current
        .iter()
        .filter_map(|(edge, authority_spans)| {
            previous
                .get(edge)
                .is_some_and(|previous_spans| previous_spans.as_ref() == authority_spans.as_ref())
                .then_some(*edge)
        })
        .collect()
}

impl FinalStepBoundaryEdgeIndex {
    fn new(edges: impl IntoIterator<Item = FinalStepBoundaryEdgeReuseKey>) -> Self {
        let mut edges_by_kind =
            BTreeMap::<RoadSurfaceBandKind, Vec<FinalStepBoundaryEdgeCandidate>>::new();
        let mut tile_indices_by_kind =
            BTreeMap::<RoadSurfaceBandKind, BTreeMap<SurfaceKeyTile, Vec<usize>>>::new();
        for edge in edges {
            let kind = edge.owner.kind();
            let bounds = edge.bounds();
            let candidate_index = edges_by_kind.entry(kind).or_default().len();
            SurfaceKeyTile::for_each_in_bounds(bounds, |tile| {
                tile_indices_by_kind
                    .entry(kind)
                    .or_default()
                    .entry(tile)
                    .or_default()
                    .push(candidate_index);
            });
            edges_by_kind
                .entry(kind)
                .or_default()
                .push(FinalStepBoundaryEdgeCandidate { edge, bounds });
        }
        Self {
            edges_by_kind,
            tile_indices_by_kind,
        }
    }

    fn overlap_candidates(
        &self,
        edge: FinalStepBoundaryEdgeReuseKey,
        output: &mut Vec<FinalStepBoundaryEdgeReuseKey>,
        candidate_indices: &mut Vec<usize>,
    ) {
        output.clear();
        let bounds = edge.bounds();
        for (kind, candidates) in &self.edges_by_kind {
            if !raised_step_kinds_can_contact(edge.owner.kind(), *kind) {
                continue;
            }
            let Some(tile_indices) = self.tile_indices_by_kind.get(kind) else {
                continue;
            };
            candidate_indices.clear();
            SurfaceKeyTile::for_each_in_bounds(bounds, |tile| {
                if let Some(indices) = tile_indices.get(&tile) {
                    candidate_indices.extend(indices.iter().copied());
                }
            });
            candidate_indices.sort_unstable();
            candidate_indices.dedup();
            output.extend(
                candidate_indices
                    .iter()
                    .filter_map(|candidate_index| candidates.get(*candidate_index))
                    .filter(|candidate| candidate.bounds.overlaps(bounds))
                    .map(|candidate| candidate.edge),
            );
        }
        output.sort_unstable();
        output.dedup();
    }
}

fn authorized_final_boundary_raised_step_overlap_segment(
    left_key: FinalStepBoundaryEdgeReuseKey,
    left_authority_spans: &[FinalStepAuthoritySpan],
    right_key: FinalStepBoundaryEdgeReuseKey,
    right_authority_spans: &[FinalStepAuthoritySpan],
) -> Option<NodeExplicitVerticalStepSegment> {
    if left_key.owner == right_key.owner
        || !raised_step_kinds_can_contact(left_key.owner.kind(), right_key.owner.kind())
    {
        return None;
    }
    let left_rank = raised_step_band_rank(left_key.owner.kind())?;
    let right_rank = raised_step_band_rank(right_key.owner.kind())?;
    if left_rank == right_rank {
        return None;
    }
    let (start, end) = arrangement_edge_overlap_segment(left_key.geometry(), right_key.geometry())?;
    let (lower_key, lower, raised_key, raised) = if left_rank < right_rank {
        (
            left_key,
            left_key.geometry(),
            right_key,
            right_key.geometry(),
        )
    } else {
        (
            right_key,
            right_key.geometry(),
            left_key,
            left_key.geometry(),
        )
    };
    let (lower_authority_spans, raised_authority_spans) = if left_rank < right_rank {
        (left_authority_spans, right_authority_spans)
    } else {
        (right_authority_spans, left_authority_spans)
    };
    if !arrangement_edges_have_positive_raised_step_delta(lower, raised, start, end)
        || !lower_key.authorizes_overlap_with(lower_authority_spans, raised_key.owner, start, end)
        || !raised_key.authorizes_overlap_with(raised_authority_spans, lower_key.owner, start, end)
    {
        return None;
    }

    NodeExplicitVerticalStepSegment::new(start, end, lower_key.owner, raised_key.owner)
}

fn ordered_owner_pair(left: NodeBandOwner, right: NodeBandOwner) -> (NodeBandOwner, NodeBandOwner) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn normalized_key_pair(
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> (NodeArrangementKey, NodeArrangementKey) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        start: (i64, i64),
        end: (i64, i64),
    ) -> FinalStepBoundaryEdgeReuseKey {
        FinalStepBoundaryEdgeReuseKey {
            owner: NodeBandOwner::new(kind, owner_index),
            start: NodeArrangementKey::from_surface_key(SurfaceXzKey::from_raw_tuple(start)),
            end: NodeArrangementKey::from_surface_key(SurfaceXzKey::from_raw_tuple(end)),
            start_height_mm: 0,
            end_height_mm: 0,
        }
    }

    #[test]
    fn final_step_index_retains_overlay_grid_tolerant_candidates_at_negative_tile_boundary() {
        let lower = edge(
            RoadSurfaceBandKind::Carriageway,
            0,
            (-8_000_001, 0),
            (-7_999_999, 0),
        );
        let raised = edge(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            (-8_000_001, 2),
            (-7_999_999, 2),
        );
        assert!(
            arrangement_edge_overlap_segment(lower.geometry(), raised.geometry()).is_some(),
            "the exact downstream predicate must accept this overlay-grid offset"
        );

        let index = FinalStepBoundaryEdgeIndex::new([lower, raised]);
        let mut candidates = Vec::new();
        let mut candidate_indices = Vec::new();
        index.overlap_candidates(lower, &mut candidates, &mut candidate_indices);
        assert!(
            candidates.contains(&raised),
            "the conservative index must be a superset of tolerant exact overlap"
        );

        let authority = FinalStepAuthoritySpan {
            owners: ordered_owner_pair(lower.owner, raised.owner),
            start: raised.start,
            end: raised.end,
        };
        assert!(
            authority.bounds().overlaps(lower.bounds()),
            "authority relevance pruning must retain overlay-grid-tolerant spans"
        );
    }
}
