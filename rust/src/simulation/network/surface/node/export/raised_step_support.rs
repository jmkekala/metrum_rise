// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step vertical face support checks against final owned top surfaces.

use super::super::{
    ArrangementBoundaryPointKey, NodeOwnedRegion, RoadSurfaceRaisedStepFace,
    RoadSurfaceVerticalFaceSource,
    arrangement::{NodeBandOwner, NodeExplicitVerticalStepSegment},
    keys, segments,
};
use crate::simulation::network::surface::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualPolygon, RoadVec3,
    band_semantics::ordered_raised_step_kinds,
    indices::{SurfaceKeyBounds, SurfaceKeyTile},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const RAISED_STEP_FACE_HEIGHT_DUST_MM: i64 = 1;

impl RoadSurfaceSystem {
    /// Builds final-support raised-step faces while reusing exact top-edge and local-span products.
    pub(super) fn raised_step_faces_with_owned_top_support(
        owned_regions: &[NodeOwnedRegion],
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
        previous: Option<&NodeRaisedStepIncrementalCache>,
    ) -> (
        Vec<RoadSurfaceRaisedStepFace>,
        NodeRaisedStepIncrementalCache,
        NodeRaisedStepReuseStats,
    ) {
        let mut current = NodeRaisedStepIncrementalCache::default();
        let mut stats = NodeRaisedStepReuseStats::default();
        let top_edges =
            owned_top_boundary_edges_with_reuse(owned_regions, previous, &mut current, &mut stats);
        let required_spans = final_required_raised_step_spans_with_reuse(
            explicit_vertical_step_segments,
            &top_edges,
            previous,
            &mut current,
            &mut stats,
        );
        let region_centroids = raised_step_region_centroids(owned_regions);
        let owner_centroids = raised_step_owner_centroids(owned_regions);
        let faces = complete_raised_step_faces_from_final_spans(
            required_spans,
            &region_centroids,
            &owner_centroids,
        );
        (faces, current, stats)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum TopRegionGeometryReuseKey {
    Triangles(Box<[[NodeTopSupportVertexKey; 3]]>),
    Loop(Box<[NodeTopSupportVertexKey]>),
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TopRegionEdgeReuseKey {
    geometry: TopRegionGeometryReuseKey,
}

#[derive(Clone, Copy, Debug)]
struct RawTopEdgeContribution {
    key: NodeTopSupportEdgeKey,
    start: NodeTopSupportVertexKey,
    end: NodeTopSupportVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct StableTopSupportEdge {
    owner: NodeBandOwner,
    start: NodeTopSupportVertexKey,
    end: NodeTopSupportVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct StableSupportCandidate {
    edge: StableTopSupportEdge,
    start_t: keys::SurfaceSegmentParameter,
    end_t: keys::SurfaceSegmentParameter,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepLocalSupportReuseKey {
    segment: NodeExplicitVerticalStepSegment,
    lower_candidates: Box<[StableSupportCandidate]>,
    raised_candidates: Box<[StableSupportCandidate]>,
}

#[derive(Clone, Copy, Debug)]
enum CachedRaisedStepSource {
    Canonical,
    SameMaterialHandoff {
        lower_owner: NodeBandOwner,
        raised_owner: NodeBandOwner,
    },
}

#[derive(Clone, Debug)]
struct CachedRaisedStepSpan {
    lower_support_edge: StableTopSupportEdge,
    boundary_keys: RaisedStepBoundaryPointKeys,
    support_key: RaisedStepFaceSupportKey,
    rendered_key: RenderedRaisedStepFaceKey,
    source: CachedRaisedStepSource,
    polygon: RoadSurfaceVisualPolygon,
}

/// Exact top-edge and step-span products reusable by a later export generation.
#[derive(Clone, Debug, Default)]
pub(super) struct NodeRaisedStepIncrementalCache {
    top_edge_contributors: BTreeMap<TopRegionEdgeReuseKey, Arc<[RawTopEdgeContribution]>>,
    raised_step_spans: BTreeMap<RaisedStepLocalSupportReuseKey, Arc<[CachedRaisedStepSpan]>>,
}

/// Reuse activity for one final raised-step export.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NodeRaisedStepReuseStats {
    /// Top-edge contributor lookups served by either current or previous cache entries.
    pub(super) top_edge_cache_hits: usize,
    /// Top-edge contributors promoted specifically from the previous generation.
    pub(super) top_edge_previous_hits: usize,
    /// Top-edge contributors extracted from current polygon geometry.
    pub(super) top_edge_cache_misses: usize,
    /// Raised-step span lookups served by either current or previous cache entries.
    pub(super) raised_step_cache_hits: usize,
    /// Raised-step span products promoted specifically from the previous generation.
    pub(super) raised_step_previous_hits: usize,
    /// Raised-step span products rebuilt from current support candidates.
    pub(super) raised_step_cache_misses: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportVertexKey {
    xz: keys::SurfaceXzKey,
    y_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportEdgeKey {
    start: NodeTopSupportVertexKey,
    end: NodeTopSupportVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportEdge {
    owner: NodeBandOwner,
    region_index: usize,
    start: NodeTopSupportVertexKey,
    end: NodeTopSupportVertexKey,
}

#[derive(Clone, Copy, Debug)]
struct NodeTopSupportEdgeCandidate {
    edge: NodeTopSupportEdge,
    bounds: SurfaceKeyBounds,
}

#[derive(Clone, Debug, Default)]
struct NodeTopSupportEdgeIndex {
    edges_by_kind: BTreeMap<RoadSurfaceBandKind, Vec<NodeTopSupportEdgeCandidate>>,
    tile_indices_by_kind: BTreeMap<RoadSurfaceBandKind, BTreeMap<SurfaceKeyTile, Vec<usize>>>,
}

#[derive(Clone, Debug)]
struct FinalRequiredRaisedStepSpan {
    lower_edge: NodeTopSupportEdge,
    boundary_keys: RaisedStepBoundaryPointKeys,
    support_key: RaisedStepFaceSupportKey,
    rendered_key: RenderedRaisedStepFaceKey,
    source: RoadSurfaceVerticalFaceSource,
    polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug)]
struct RaisedStepBoundaryPointKeys {
    lower_start: ArrangementBoundaryPointKey,
    lower_end: ArrangementBoundaryPointKey,
    raised_start: ArrangementBoundaryPointKey,
    raised_end: ArrangementBoundaryPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepFaceSupportKey {
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    lower_edge: NodeTopSupportEdgeKey,
    upper_edge: NodeTopSupportEdgeKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RenderedRaisedStepFaceKey {
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    lower_edge: RenderedRaisedStepEdgeKey,
    upper_edge: RenderedRaisedStepEdgeKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RenderedRaisedStepEdgeKey {
    start: RenderedRaisedStepVertexKey,
    end: RenderedRaisedStepVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RenderedRaisedStepVertexKey {
    x_mm: i64,
    y_mm: i64,
    z_mm: i64,
}

fn owned_top_boundary_edges_with_reuse(
    owned_regions: &[NodeOwnedRegion],
    previous: Option<&NodeRaisedStepIncrementalCache>,
    current: &mut NodeRaisedStepIncrementalCache,
    stats: &mut NodeRaisedStepReuseStats,
) -> Vec<NodeTopSupportEdge> {
    let mut edge_counts_by_owner = BTreeMap::<
        NodeBandOwner,
        BTreeMap<
            NodeTopSupportEdgeKey,
            (
                usize,
                NodeTopSupportVertexKey,
                NodeTopSupportVertexKey,
                usize,
            ),
        >,
    >::new();
    for (region_index, region) in owned_regions.iter().enumerate() {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        let edge_counts = edge_counts_by_owner.entry(owner).or_default();
        let reuse_key = TopRegionEdgeReuseKey::from_region(region);
        let contributions =
            if let Some(contributions) = current.top_edge_contributors.get(&reuse_key) {
                stats.top_edge_cache_hits += 1;
                Arc::clone(contributions)
            } else if let Some(contributions) =
                previous.and_then(|previous| previous.top_edge_contributors.get(&reuse_key))
            {
                stats.top_edge_cache_hits += 1;
                stats.top_edge_previous_hits += 1;
                Arc::clone(contributions)
            } else {
                stats.top_edge_cache_misses += 1;
                Arc::from(
                    final_polygon_boundary_edges(&region.polygon)
                        .into_iter()
                        .map(|(key, start, end)| RawTopEdgeContribution { key, start, end })
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                )
            };
        current
            .top_edge_contributors
            .insert(reuse_key, Arc::clone(&contributions));
        for contribution in contributions.iter().copied() {
            edge_counts
                .entry(contribution.key)
                .and_modify(|entry| entry.0 += 1)
                .or_insert((1, contribution.start, contribution.end, region_index));
        }
    }

    let mut top_edges = Vec::new();
    for (owner, edge_counts) in edge_counts_by_owner {
        top_edges.extend(edge_counts.into_iter().filter_map(
            |(_key, (count, start, end, region_index))| {
                (count == 1).then_some(NodeTopSupportEdge {
                    owner,
                    region_index,
                    start,
                    end,
                })
            },
        ));
    }
    top_edges
}

impl TopRegionEdgeReuseKey {
    fn from_region(region: &NodeOwnedRegion) -> Self {
        let geometry = if region.polygon.triangles_world.is_empty() {
            let mut loop_vertices = region
                .polygon
                .points_world
                .iter()
                .copied()
                .map(NodeTopSupportVertexKey::from_world_point)
                .collect::<Vec<_>>();
            rotate_directed_loop_to_canonical_start(&mut loop_vertices);
            TopRegionGeometryReuseKey::Loop(loop_vertices.into_boxed_slice())
        } else {
            let mut triangles = region
                .polygon
                .triangles_world
                .iter()
                .map(|triangle| {
                    canonical_directed_triangle([
                        NodeTopSupportVertexKey::from_world_point(triangle[0]),
                        NodeTopSupportVertexKey::from_world_point(triangle[1]),
                        NodeTopSupportVertexKey::from_world_point(triangle[2]),
                    ])
                })
                .collect::<Vec<_>>();
            triangles.sort_unstable();
            TopRegionGeometryReuseKey::Triangles(triangles.into_boxed_slice())
        };
        Self { geometry }
    }
}

fn canonical_directed_triangle(
    triangle: [NodeTopSupportVertexKey; 3],
) -> [NodeTopSupportVertexKey; 3] {
    let rotations = [
        triangle,
        [triangle[1], triangle[2], triangle[0]],
        [triangle[2], triangle[0], triangle[1]],
    ];
    rotations.into_iter().min().unwrap_or(triangle)
}

fn rotate_directed_loop_to_canonical_start(vertices: &mut [NodeTopSupportVertexKey]) {
    if vertices.is_empty() {
        return;
    }
    vertices.rotate_left(minimal_directed_rotation_start(vertices));
}

fn minimal_directed_rotation_start<T: Ord>(values: &[T]) -> usize {
    let count = values.len();
    if count <= 1 {
        return 0;
    }

    // Booth's two-candidate elimination finds the lexicographically minimal cyclic rotation
    // without materializing rotations or comparing every pair of starts.
    let mut left = 0;
    let mut right = 1;
    let mut matched = 0;
    while left < count && right < count && matched < count {
        match values[(left + matched) % count].cmp(&values[(right + matched) % count]) {
            std::cmp::Ordering::Equal => matched += 1,
            std::cmp::Ordering::Less => {
                right += matched + 1;
                if right <= left {
                    right = left + 1;
                }
                matched = 0;
            }
            std::cmp::Ordering::Greater => {
                left += matched + 1;
                if left <= right {
                    left = right + 1;
                }
                matched = 0;
            }
        }
    }
    left.min(right)
}

fn final_polygon_boundary_edges(
    polygon: &RoadSurfaceVisualPolygon,
) -> Vec<(
    NodeTopSupportEdgeKey,
    NodeTopSupportVertexKey,
    NodeTopSupportVertexKey,
)> {
    let mut edges = Vec::new();
    if polygon.triangles_world.is_empty() {
        push_loop_edges(&polygon.points_world, &mut edges);
        return edges;
    }
    for triangle in &polygon.triangles_world {
        for edge_index in 0..3 {
            if let Some(edge) = top_support_edge_from_world_points(
                triangle[edge_index],
                triangle[(edge_index + 1) % 3],
            ) {
                edges.push(edge);
            }
        }
    }
    edges
}

fn push_loop_edges(
    points: &[RoadVec3],
    edges: &mut Vec<(
        NodeTopSupportEdgeKey,
        NodeTopSupportVertexKey,
        NodeTopSupportVertexKey,
    )>,
) {
    if points.len() < 2 {
        return;
    }
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        if let Some(edge) = top_support_edge_from_world_points(points[index], points[next]) {
            edges.push(edge);
        }
    }
}

fn top_support_edge_from_world_points(
    start: RoadVec3,
    end: RoadVec3,
) -> Option<(
    NodeTopSupportEdgeKey,
    NodeTopSupportVertexKey,
    NodeTopSupportVertexKey,
)> {
    let start = NodeTopSupportVertexKey::from_world_point(start);
    let end = NodeTopSupportVertexKey::from_world_point(end);
    NodeTopSupportEdgeKey::from_vertices(start, end).map(|key| (key, start, end))
}

fn complete_raised_step_faces_from_final_spans(
    required_spans: Vec<FinalRequiredRaisedStepSpan>,
    region_centroids: &[Option<RoadVec3>],
    owner_centroids: &BTreeMap<NodeBandOwner, RoadVec3>,
) -> Vec<RoadSurfaceRaisedStepFace> {
    // Final owner-wide top boundaries are the rendered authority. Rebuild the face set from these
    // spans so stale arrangement-side quads cannot block corrected final support geometry.
    let mut rebuilt = Vec::new();
    let mut emitted = BTreeSet::new();

    for span in required_spans {
        if !emitted.insert(span.support_key) {
            continue;
        }
        let mut face = RoadSurfaceRaisedStepFace {
            polygon: span.polygon,
            source: span.source,
            lower_edge: (span.boundary_keys.lower_start, span.boundary_keys.lower_end),
        };
        orient_raised_step_face_from_lower_owner(
            &mut face,
            span.lower_edge.region_index,
            region_centroids,
            owner_centroids,
        );
        rebuilt.push(face);
    }
    rebuilt
}

fn final_required_raised_step_spans_with_reuse(
    explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    top_edges: &[NodeTopSupportEdge],
    previous: Option<&NodeRaisedStepIncrementalCache>,
    current: &mut NodeRaisedStepIncrementalCache,
    stats: &mut NodeRaisedStepReuseStats,
) -> Vec<FinalRequiredRaisedStepSpan> {
    let mut spans = Vec::new();
    let mut emitted = BTreeSet::<RaisedStepFaceSupportKey>::new();
    let top_edge_index = NodeTopSupportEdgeIndex::new(top_edges);
    let mut candidate_indices = Vec::new();
    let live_edges = top_edges
        .iter()
        .copied()
        .map(|edge| (StableTopSupportEdge::from_edge(edge), edge))
        .collect::<BTreeMap<_, _>>();

    for (step_index, source_segment) in explicit_vertical_step_segments.iter().copied().enumerate()
    {
        let Some((source_lower_owner, source_raised_owner)) =
            vertical_step_lower_and_raised_owners(source_segment)
        else {
            continue;
        };
        let segment_start = keys::SurfaceXzKey::from_raw_keys(
            source_segment.start().x_key(),
            source_segment.start().z_key(),
        );
        let segment_end = keys::SurfaceXzKey::from_raw_keys(
            source_segment.end().x_key(),
            source_segment.end().z_key(),
        );
        let mut lower_candidates = top_edge_index.support_edge_candidates_on_step_segment(
            source_lower_owner.kind(),
            segment_start,
            segment_end,
            &mut candidate_indices,
        );
        let mut raised_candidates = top_edge_index.support_edge_candidates_on_step_segment(
            source_raised_owner.kind(),
            segment_start,
            segment_end,
            &mut candidate_indices,
        );
        retain_relevant_support_candidates(
            source_lower_owner,
            source_raised_owner,
            &mut lower_candidates,
            &mut raised_candidates,
        );

        let reuse_key = RaisedStepLocalSupportReuseKey::from_candidates(
            source_segment,
            &lower_candidates,
            &raised_candidates,
        );
        let cached_spans = if let Some(cached_spans) = current.raised_step_spans.get(&reuse_key) {
            stats.raised_step_cache_hits += 1;
            Arc::clone(cached_spans)
        } else if let Some(cached_spans) =
            previous.and_then(|previous| previous.raised_step_spans.get(&reuse_key))
        {
            stats.raised_step_cache_hits += 1;
            stats.raised_step_previous_hits += 1;
            Arc::clone(cached_spans)
        } else {
            stats.raised_step_cache_misses += 1;
            Arc::from(
                cached_raised_step_spans_for_candidates(
                    source_segment,
                    source_lower_owner,
                    source_raised_owner,
                    &lower_candidates,
                    &raised_candidates,
                )
                .into_boxed_slice(),
            )
        };
        current
            .raised_step_spans
            .insert(reuse_key, Arc::clone(&cached_spans));

        for cached in cached_spans.iter() {
            let Some(lower_edge) = live_edges.get(&cached.lower_support_edge).copied() else {
                debug_assert!(false, "cached raised-step support edge must remain live");
                continue;
            };
            if emitted.insert(cached.support_key) {
                spans.push(FinalRequiredRaisedStepSpan {
                    lower_edge,
                    boundary_keys: cached.boundary_keys,
                    support_key: cached.support_key,
                    rendered_key: cached.rendered_key,
                    source: cached.source.bind(step_index, source_segment),
                    polygon: cached.polygon.clone(),
                });
            }
        }
    }
    spans.sort_by_key(|span| span.support_key);
    dedup_rendered_raised_step_spans(spans)
}

fn retain_relevant_support_candidates(
    source_lower_owner: NodeBandOwner,
    source_raised_owner: NodeBandOwner,
    lower_candidates: &mut Vec<(
        NodeTopSupportEdge,
        keys::SurfaceSegmentParameter,
        keys::SurfaceSegmentParameter,
    )>,
    raised_candidates: &mut Vec<(
        NodeTopSupportEdge,
        keys::SurfaceSegmentParameter,
        keys::SurfaceSegmentParameter,
    )>,
) {
    let has_source_lower = lower_candidates
        .iter()
        .any(|candidate| candidate.0.owner == source_lower_owner);
    let has_source_raised = raised_candidates
        .iter()
        .any(|candidate| candidate.0.owner == source_raised_owner);
    if !has_source_raised {
        lower_candidates.retain(|candidate| candidate.0.owner == source_lower_owner);
    }
    if !has_source_lower {
        raised_candidates.retain(|candidate| candidate.0.owner == source_raised_owner);
    }
}

fn cached_raised_step_spans_for_candidates(
    source_segment: NodeExplicitVerticalStepSegment,
    source_lower_owner: NodeBandOwner,
    source_raised_owner: NodeBandOwner,
    lower_candidates: &[(
        NodeTopSupportEdge,
        keys::SurfaceSegmentParameter,
        keys::SurfaceSegmentParameter,
    )],
    raised_candidates: &[(
        NodeTopSupportEdge,
        keys::SurfaceSegmentParameter,
        keys::SurfaceSegmentParameter,
    )],
) -> Vec<CachedRaisedStepSpan> {
    let segment_start = keys::SurfaceXzKey::from_raw_keys(
        source_segment.start().x_key(),
        source_segment.start().z_key(),
    );
    let segment_end = keys::SurfaceXzKey::from_raw_keys(
        source_segment.end().x_key(),
        source_segment.end().z_key(),
    );
    let mut lower_candidates = lower_candidates.to_vec();
    lower_candidates.sort_by_key(|candidate| StableSupportCandidate::from_candidate(*candidate));
    let mut raised_candidates = raised_candidates.to_vec();
    raised_candidates.sort_by_key(|candidate| StableSupportCandidate::from_candidate(*candidate));
    let mut emitted = BTreeSet::new();
    let mut spans = Vec::new();

    for (lower_edge, lower_start_t, lower_end_t) in &lower_candidates {
        for (raised_edge, raised_start_t, raised_end_t) in &raised_candidates {
            let lower_owner = lower_edge.owner;
            let raised_owner = raised_edge.owner;
            let Some(source) = cached_vertical_step_source_for_final_support_owners(
                source_lower_owner,
                source_raised_owner,
                lower_owner,
                raised_owner,
            ) else {
                continue;
            };
            let start_t = (*lower_start_t).max(*raised_start_t);
            let end_t = (*lower_end_t).min(*raised_end_t);
            if end_t <= start_t {
                continue;
            }
            let Some(boundary_keys) = raised_step_boundary_points_from_top_support(
                *lower_edge,
                *raised_edge,
                segment_start,
                segment_end,
                start_t,
                end_t,
            ) else {
                continue;
            };
            let support_key = raised_step_face_support_key_from_boundary_points(
                lower_owner,
                raised_owner,
                boundary_keys,
            );
            let Some(rendered_key) = rendered_raised_step_face_key_from_boundary_points(
                lower_owner,
                raised_owner,
                boundary_keys,
            ) else {
                continue;
            };
            let Some(polygon) = raised_step_polygon_from_top_support(
                *lower_edge,
                segment_start,
                segment_end,
                boundary_keys,
            ) else {
                continue;
            };
            if emitted.insert(support_key) {
                spans.push(CachedRaisedStepSpan {
                    lower_support_edge: StableTopSupportEdge::from_edge(*lower_edge),
                    boundary_keys,
                    support_key,
                    rendered_key,
                    source,
                    polygon,
                });
            }
        }
    }
    spans.sort_by_key(|span| span.support_key);
    spans
}

impl RaisedStepLocalSupportReuseKey {
    fn from_candidates(
        segment: NodeExplicitVerticalStepSegment,
        lower_candidates: &[(
            NodeTopSupportEdge,
            keys::SurfaceSegmentParameter,
            keys::SurfaceSegmentParameter,
        )],
        raised_candidates: &[(
            NodeTopSupportEdge,
            keys::SurfaceSegmentParameter,
            keys::SurfaceSegmentParameter,
        )],
    ) -> Self {
        let mut lower_candidates = lower_candidates
            .iter()
            .copied()
            .map(StableSupportCandidate::from_candidate)
            .collect::<Vec<_>>();
        lower_candidates.sort_unstable();
        let mut raised_candidates = raised_candidates
            .iter()
            .copied()
            .map(StableSupportCandidate::from_candidate)
            .collect::<Vec<_>>();
        raised_candidates.sort_unstable();
        Self {
            segment,
            lower_candidates: lower_candidates.into_boxed_slice(),
            raised_candidates: raised_candidates.into_boxed_slice(),
        }
    }
}

impl StableSupportCandidate {
    fn from_candidate(
        candidate: (
            NodeTopSupportEdge,
            keys::SurfaceSegmentParameter,
            keys::SurfaceSegmentParameter,
        ),
    ) -> Self {
        Self {
            edge: StableTopSupportEdge::from_edge(candidate.0),
            start_t: candidate.1,
            end_t: candidate.2,
        }
    }
}

impl StableTopSupportEdge {
    fn from_edge(edge: NodeTopSupportEdge) -> Self {
        Self {
            owner: edge.owner,
            start: edge.start,
            end: edge.end,
        }
    }
}

impl CachedRaisedStepSource {
    fn bind(
        self,
        explicit_vertical_step_index: usize,
        segment: NodeExplicitVerticalStepSegment,
    ) -> RoadSurfaceVerticalFaceSource {
        match self {
            Self::Canonical => RoadSurfaceVerticalFaceSource::CanonicalStep {
                explicit_vertical_step_index,
                segment,
            },
            Self::SameMaterialHandoff {
                lower_owner,
                raised_owner,
            } => RoadSurfaceVerticalFaceSource::CanonicalStepSameMaterialHandoff {
                explicit_vertical_step_index,
                segment,
                lower_owner,
                raised_owner,
            },
        }
    }
}

fn dedup_rendered_raised_step_spans(
    spans: Vec<FinalRequiredRaisedStepSpan>,
) -> Vec<FinalRequiredRaisedStepSpan> {
    let mut spans_by_rendered_key =
        BTreeMap::<RenderedRaisedStepFaceKey, FinalRequiredRaisedStepSpan>::new();
    for span in spans {
        let should_insert = match spans_by_rendered_key.get(&span.rendered_key) {
            Some(existing) => raised_step_span_is_preferred_over(&span, existing),
            None => true,
        };
        if should_insert {
            spans_by_rendered_key.insert(span.rendered_key, span);
        }
    }
    let mut spans = spans_by_rendered_key.into_values().collect::<Vec<_>>();
    spans.sort_by_key(|span| span.support_key);
    spans
}

fn raised_step_span_is_preferred_over(
    candidate: &FinalRequiredRaisedStepSpan,
    existing: &FinalRequiredRaisedStepSpan,
) -> bool {
    raised_step_source_preference(candidate.source) > raised_step_source_preference(existing.source)
}

fn raised_step_source_preference(source: RoadSurfaceVerticalFaceSource) -> u8 {
    match source {
        RoadSurfaceVerticalFaceSource::CanonicalStep { .. } => 1,
        RoadSurfaceVerticalFaceSource::CanonicalStepSameMaterialHandoff { .. } => 0,
    }
}

fn cached_vertical_step_source_for_final_support_owners(
    source_lower_owner: NodeBandOwner,
    source_raised_owner: NodeBandOwner,
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
) -> Option<CachedRaisedStepSource> {
    if source_lower_owner == lower_owner && source_raised_owner == raised_owner {
        return Some(CachedRaisedStepSource::Canonical);
    }
    if source_lower_owner.kind() != lower_owner.kind()
        || source_raised_owner.kind() != raised_owner.kind()
    {
        return None;
    }
    if source_lower_owner != lower_owner && source_raised_owner != raised_owner {
        return None;
    }
    Some(CachedRaisedStepSource::SameMaterialHandoff {
        lower_owner,
        raised_owner,
    })
}

fn raised_step_polygon_from_top_support(
    lower_edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
    keys: RaisedStepBoundaryPointKeys,
) -> Option<RoadSurfaceVisualPolygon> {
    let lower_start = boundary_point_to_world(keys.lower_start);
    let lower_end = boundary_point_to_world(keys.lower_end);
    let raised_start = boundary_point_to_world(keys.raised_start);
    let raised_end = boundary_point_to_world(keys.raised_end);
    let lower_owner_on_right =
        support_edge_owner_lies_right_of_segment(lower_edge, segment_start, segment_end)?;
    let points = if lower_owner_on_right {
        [raised_start, lower_start, lower_end, raised_end]
    } else {
        [raised_end, lower_end, lower_start, raised_start]
    };
    RoadSurfaceSystem::make_vertical_quad_polygon(points)
}

fn raised_step_boundary_points_from_top_support(
    lower_edge: NodeTopSupportEdge,
    raised_edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
    start_t: keys::SurfaceSegmentParameter,
    end_t: keys::SurfaceSegmentParameter,
) -> Option<RaisedStepBoundaryPointKeys> {
    let lower_start =
        support_edge_point_at_segment_parameter(lower_edge, segment_start, segment_end, start_t)?;
    let lower_end =
        support_edge_point_at_segment_parameter(lower_edge, segment_start, segment_end, end_t)?;
    let raised_start =
        support_edge_point_at_segment_parameter(raised_edge, segment_start, segment_end, start_t)?;
    let raised_end =
        support_edge_point_at_segment_parameter(raised_edge, segment_start, segment_end, end_t)?;
    let raised_start = raised_step_face_height_dust_adjusted_point(lower_start, raised_start)?;
    let raised_end = raised_step_face_height_dust_adjusted_point(lower_end, raised_end)?;
    if lower_start.xz_key() == lower_end.xz_key()
        || raised_start.xz_key() == raised_end.xz_key()
        || (raised_start.y_mm == lower_start.y_mm && raised_end.y_mm == lower_end.y_mm)
    {
        return None;
    }
    Some(RaisedStepBoundaryPointKeys {
        lower_start,
        lower_end,
        raised_start,
        raised_end,
    })
}

fn raised_step_face_height_dust_adjusted_point(
    lower: ArrangementBoundaryPointKey,
    mut raised: ArrangementBoundaryPointKey,
) -> Option<ArrangementBoundaryPointKey> {
    if raised.y_mm >= lower.y_mm {
        return Some(raised);
    }
    if lower.y_mm - raised.y_mm > RAISED_STEP_FACE_HEIGHT_DUST_MM {
        return None;
    }
    raised.y_mm = lower.y_mm;
    Some(raised)
}

fn raised_step_face_support_key_from_boundary_points(
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    keys: RaisedStepBoundaryPointKeys,
) -> RaisedStepFaceSupportKey {
    RaisedStepFaceSupportKey {
        lower_owner,
        raised_owner,
        lower_edge: NodeTopSupportEdgeKey::from_boundary_points((keys.lower_start, keys.lower_end)),
        upper_edge: NodeTopSupportEdgeKey::from_boundary_points((
            keys.raised_start,
            keys.raised_end,
        )),
    }
}

fn rendered_raised_step_face_key_from_boundary_points(
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    keys: RaisedStepBoundaryPointKeys,
) -> Option<RenderedRaisedStepFaceKey> {
    Some(RenderedRaisedStepFaceKey {
        lower_owner,
        raised_owner,
        lower_edge: RenderedRaisedStepEdgeKey::from_boundary_points(
            keys.lower_start,
            keys.lower_end,
        )?,
        upper_edge: RenderedRaisedStepEdgeKey::from_boundary_points(
            keys.raised_start,
            keys.raised_end,
        )?,
    })
}

fn raised_step_region_centroids(owned_regions: &[NodeOwnedRegion]) -> Vec<Option<RoadVec3>> {
    owned_regions
        .iter()
        .map(|region| owned_region_centroid_sum(region).map(|(sum, count)| sum / count as f64))
        .collect()
}

fn raised_step_owner_centroids(
    owned_regions: &[NodeOwnedRegion],
) -> BTreeMap<NodeBandOwner, RoadVec3> {
    let mut sums = BTreeMap::<NodeBandOwner, (RoadVec3, usize)>::new();
    for region in owned_regions {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        let entry = sums.entry(owner).or_insert((RoadVec3::ZERO, 0));
        if let Some((sum, count)) = owned_region_centroid_sum(region) {
            entry.0 += sum;
            entry.1 += count;
        }
    }
    sums.into_iter()
        .filter_map(|(owner, (sum, count))| (count > 0).then_some((owner, sum / count as f64)))
        .collect()
}

fn owned_region_centroid_sum(region: &NodeOwnedRegion) -> Option<(RoadVec3, usize)> {
    let mut sum = RoadVec3::ZERO;
    let mut count = 0usize;
    if !region.polygon.points_world.is_empty() {
        for point in &region.polygon.points_world {
            sum += RoadVec3::new(point.x, 0.0, point.z);
            count += 1;
        }
    } else {
        for point in region
            .polygon
            .triangles_world
            .iter()
            .flat_map(|triangle| triangle.iter())
        {
            sum += RoadVec3::new(point.x, 0.0, point.z);
            count += 1;
        }
    }
    (count > 0).then_some((sum, count))
}

fn orient_raised_step_face_from_lower_owner(
    face: &mut RoadSurfaceRaisedStepFace,
    lower_region_index: usize,
    region_centroids: &[Option<RoadVec3>],
    owner_centroids: &BTreeMap<NodeBandOwner, RoadVec3>,
) {
    let Some((lower_owner, _)) = face.source.lower_and_raised_owners() else {
        return;
    };
    let Some(lower_centroid) = region_centroids
        .get(lower_region_index)
        .copied()
        .flatten()
        .or_else(|| owner_centroids.get(&lower_owner).copied())
    else {
        return;
    };
    let lower_start = boundary_point_to_world(face.lower_edge.0);
    let lower_end = boundary_point_to_world(face.lower_edge.1);
    let midpoint = RoadVec3::new(
        (lower_start.x + lower_end.x) * 0.5,
        0.0,
        (lower_start.z + lower_end.z) * 0.5,
    );
    let owner_direction = RoadVec3::new(
        lower_centroid.x - midpoint.x,
        0.0,
        lower_centroid.z - midpoint.z,
    );
    if owner_direction.length_squared() <= 1e-8 {
        return;
    }
    let Some(visible_direction) = vertical_face_visible_direction(&face.polygon.points_world)
    else {
        return;
    };
    if visible_direction.dot(owner_direction.normalize()) > 0.0 {
        return;
    }
    let [a, b, c, d] = face.polygon.points_world.as_slice() else {
        return;
    };
    if let Some(flipped) = RoadSurfaceSystem::make_vertical_quad_polygon([*d, *c, *b, *a]) {
        face.polygon = flipped;
    }
}

fn vertical_face_visible_direction(points: &[RoadVec3]) -> Option<RoadVec3> {
    if points.len() < 3 {
        return None;
    }
    for index in 1..points.len().saturating_sub(1) {
        let normal = (points[index] - points[0]).cross(points[index + 1] - points[0]);
        if normal.length_squared() > 1e-8 {
            let visible = -normal.normalize();
            let visible_xz = RoadVec3::new(visible.x, 0.0, visible.z);
            return (visible_xz.length_squared() > 1e-8).then(|| visible_xz.normalize());
        }
    }
    None
}

fn vertical_step_lower_and_raised_owners(
    segment: NodeExplicitVerticalStepSegment,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    Some(if owner.kind() == lower_kind {
        (owner, opposite_owner)
    } else {
        (opposite_owner, owner)
    })
}

fn support_edge_overlap_interval_on_segment(
    edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<(keys::SurfaceSegmentParameter, keys::SurfaceSegmentParameter)> {
    let mut parameters = [keys::SurfaceSegmentParameter::zero(); 4];
    let mut parameter_count = 0;
    for point in [edge.start.xz, edge.end.xz] {
        if let Some(parameter) = endpoint_parameter_on_segment(point, segment_start, segment_end) {
            insert_support_overlap_parameter(
                &mut parameters,
                &mut parameter_count,
                clamped_unit_parameter(parameter),
            );
        }
    }
    for (point, parameter) in [
        (segment_start, keys::SurfaceSegmentParameter::zero()),
        (segment_end, keys::SurfaceSegmentParameter::one()),
    ] {
        if bounded_endpoint_parameter_on_segment(point, edge.start.xz, edge.end.xz).is_some() {
            insert_support_overlap_parameter(&mut parameters, &mut parameter_count, parameter);
        }
    }
    if parameter_count == 0 {
        return None;
    }
    let start = parameters[0];
    let end = parameters[parameter_count - 1];
    (end > start).then_some((start, end))
}

fn insert_support_overlap_parameter(
    parameters: &mut [keys::SurfaceSegmentParameter; 4],
    parameter_count: &mut usize,
    parameter: keys::SurfaceSegmentParameter,
) {
    if parameters[..*parameter_count]
        .iter()
        .any(|existing| *existing == parameter)
    {
        return;
    }
    debug_assert!(*parameter_count < parameters.len());
    if *parameter_count == parameters.len() {
        return;
    }
    let mut insert_index = *parameter_count;
    while insert_index > 0 && parameter < parameters[insert_index - 1] {
        parameters[insert_index] = parameters[insert_index - 1];
        insert_index -= 1;
    }
    parameters[insert_index] = parameter;
    *parameter_count += 1;
}

fn endpoint_parameter_on_segment(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    segments::overlay_segment_parameter(point, segment_start, segment_end)
        .or_else(|| segments::exact_line_parameter(point, segment_start, segment_end))
        .or_else(|| numeric_dust_line_parameter(point, segment_start, segment_end))
        .or_else(|| overlay_grid_line_parameter(point, segment_start, segment_end))
}

fn bounded_endpoint_parameter_on_segment(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    segments::overlay_segment_parameter(point, segment_start, segment_end)
        .or_else(|| {
            bounded_unit_parameter(segments::exact_line_parameter(
                point,
                segment_start,
                segment_end,
            )?)
        })
        .or_else(|| numeric_dust_line_parameter(point, segment_start, segment_end))
        .or_else(|| {
            bounded_unit_parameter(overlay_grid_line_parameter(
                point,
                segment_start,
                segment_end,
            )?)
        })
}

fn overlay_grid_line_parameter(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    if !segments::key_collinear_with_overlay_grid_segment(point, segment_start, segment_end) {
        return None;
    }
    let dx = i128::from(segment_end.x_key() - segment_start.x_key());
    let dz = i128::from(segment_end.z_key() - segment_start.z_key());
    let denominator = dx * dx + dz * dz;
    keys::SurfaceSegmentParameter::new(
        segments::segment_parameter_key(segment_start, segment_end, point),
        denominator,
    )
}

fn numeric_dust_line_parameter(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    if segment_start == segment_end {
        return None;
    }
    let dx = i128::from(segment_end.x_key() - segment_start.x_key());
    let dz = i128::from(segment_end.z_key() - segment_start.z_key());
    let px = i128::from(point.x_key() - segment_start.x_key());
    let pz = i128::from(point.z_key() - segment_start.z_key());
    let denominator = dx * dx + dz * dz;
    if denominator == 0 {
        return None;
    }
    let numerator = px * dx + pz * dz;
    let length_key_units = (denominator as f64).sqrt();
    let dust_key_units = final_step_support_numeric_dust_key_units() as f64;
    let endpoint_padding = dust_key_units * length_key_units;
    let numerator_f64 = numerator as f64;
    if numerator_f64 < -endpoint_padding || numerator_f64 > denominator as f64 + endpoint_padding {
        return None;
    }
    let cross = dx * pz - dz * px;
    if cross.unsigned_abs() as f64 > dust_key_units * length_key_units {
        return None;
    }
    keys::SurfaceSegmentParameter::new(numerator.clamp(0, denominator), denominator)
}

fn final_step_support_numeric_dust_key_units() -> i64 {
    (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * keys::SURFACE_XZ_KEY_SCALE).round() as i64
}

fn clamped_unit_parameter(
    parameter: keys::SurfaceSegmentParameter,
) -> keys::SurfaceSegmentParameter {
    parameter
        .max(keys::SurfaceSegmentParameter::zero())
        .min(keys::SurfaceSegmentParameter::one())
}

fn bounded_unit_parameter(
    parameter: keys::SurfaceSegmentParameter,
) -> Option<keys::SurfaceSegmentParameter> {
    (parameter >= keys::SurfaceSegmentParameter::zero()
        && parameter <= keys::SurfaceSegmentParameter::one())
    .then_some(parameter)
}

fn support_edge_point_at_segment_parameter(
    edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
    parameter: keys::SurfaceSegmentParameter,
) -> Option<ArrangementBoundaryPointKey> {
    let xz = segments::interpolate_key(segment_start, segment_end, parameter);
    support_edge_point_at_xz(edge, xz)
}

fn support_edge_point_at_xz(
    edge: NodeTopSupportEdge,
    xz: keys::SurfaceXzKey,
) -> Option<ArrangementBoundaryPointKey> {
    let edge_parameter = bounded_endpoint_parameter_on_segment(xz, edge.start.xz, edge.end.xz)?;
    let supported_xz = segments::interpolate_key(edge.start.xz, edge.end.xz, edge_parameter);
    Some(ArrangementBoundaryPointKey {
        x_key: supported_xz.x_key(),
        z_key: supported_xz.z_key(),
        y_mm: segments::interpolate_height_i64(edge.start.y_mm, edge.end.y_mm, edge_parameter),
    })
}

fn support_edge_owner_lies_right_of_segment(
    edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<bool> {
    let start_t = support_edge_order_key(edge.start.xz, segment_start, segment_end)?;
    let end_t = support_edge_order_key(edge.end.xz, segment_start, segment_end)?;
    Some(end_t < start_t)
}

fn support_edge_order_key(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<i128> {
    if segment_start == segment_end {
        return None;
    }
    Some(segments::segment_parameter_key(
        segment_start,
        segment_end,
        point,
    ))
}

fn boundary_point_to_world(point: ArrangementBoundaryPointKey) -> RoadVec3 {
    let xz = keys::SurfaceXzKey::from_raw_keys(point.x_key, point.z_key).to_road_xz();
    RoadVec3::new(xz.x, point.y_mm as f64 / 1000.0, xz.y)
}

impl NodeTopSupportEdgeKey {
    fn from_vertices(start: NodeTopSupportVertexKey, end: NodeTopSupportVertexKey) -> Option<Self> {
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn from_boundary_points(
        edge: (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    ) -> Self {
        let start = NodeTopSupportVertexKey::from_boundary_point(edge.0);
        let end = NodeTopSupportVertexKey::from_boundary_point(edge.1);
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }
}

impl RenderedRaisedStepEdgeKey {
    fn from_boundary_points(
        start: ArrangementBoundaryPointKey,
        end: ArrangementBoundaryPointKey,
    ) -> Option<Self> {
        let start = RenderedRaisedStepVertexKey::from_boundary_point(start);
        let end = RenderedRaisedStepVertexKey::from_boundary_point(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }
}

impl RenderedRaisedStepVertexKey {
    fn from_boundary_point(point: ArrangementBoundaryPointKey) -> Self {
        let xz = keys::SurfaceXzKey::from_raw_keys(point.x_key, point.z_key);
        Self {
            x_mm: xz.x_mm(),
            y_mm: point.y_mm,
            z_mm: xz.z_mm(),
        }
    }
}

impl NodeTopSupportEdgeCandidate {
    fn new(edge: NodeTopSupportEdge) -> Self {
        Self {
            edge,
            bounds: SurfaceKeyBounds::from_segment(edge.start.xz, edge.end.xz)
                .expanded(final_step_support_numeric_dust_key_units()),
        }
    }
}

impl NodeTopSupportEdgeIndex {
    fn new(top_edges: &[NodeTopSupportEdge]) -> Self {
        let mut edges_by_kind =
            BTreeMap::<RoadSurfaceBandKind, Vec<NodeTopSupportEdgeCandidate>>::new();
        let mut tile_indices_by_kind =
            BTreeMap::<RoadSurfaceBandKind, BTreeMap<SurfaceKeyTile, Vec<usize>>>::new();
        for edge in top_edges.iter().copied() {
            let kind = edge.owner.kind();
            let candidate = NodeTopSupportEdgeCandidate::new(edge);
            let candidate_index = edges_by_kind.entry(kind).or_default().len();
            SurfaceKeyTile::for_each_in_bounds(candidate.bounds, |tile| {
                tile_indices_by_kind
                    .entry(kind)
                    .or_default()
                    .entry(tile)
                    .or_default()
                    .push(candidate_index);
            });
            edges_by_kind.entry(kind).or_default().push(candidate);
        }
        Self {
            edges_by_kind,
            tile_indices_by_kind,
        }
    }

    fn support_edge_candidates_on_step_segment(
        &self,
        owner_kind: RoadSurfaceBandKind,
        segment_start: keys::SurfaceXzKey,
        segment_end: keys::SurfaceXzKey,
        candidate_indices: &mut Vec<usize>,
    ) -> Vec<(
        NodeTopSupportEdge,
        keys::SurfaceSegmentParameter,
        keys::SurfaceSegmentParameter,
    )> {
        let Some(edges) = self.edges_by_kind.get(&owner_kind) else {
            return Vec::new();
        };
        let Some(tile_indices) = self.tile_indices_by_kind.get(&owner_kind) else {
            return Vec::new();
        };
        let segment_bounds = SurfaceKeyBounds::from_segment(segment_start, segment_end)
            .expanded(final_step_support_numeric_dust_key_units());
        candidate_indices.clear();
        SurfaceKeyTile::for_each_in_bounds(segment_bounds, |tile| {
            if let Some(indices) = tile_indices.get(&tile) {
                candidate_indices.extend(indices.iter().copied());
            }
        });
        candidate_indices.sort_unstable();
        candidate_indices.dedup();
        candidate_indices
            .iter()
            .filter_map(|candidate_index| edges.get(*candidate_index))
            .filter(|candidate| candidate.bounds.overlaps(segment_bounds))
            .filter_map(|candidate| {
                let edge = candidate.edge;
                let (start_t, end_t) =
                    support_edge_overlap_interval_on_segment(edge, segment_start, segment_end)?;
                Some((edge, start_t, end_t))
            })
            .collect()
    }
}

impl NodeTopSupportVertexKey {
    fn from_world_point(point: RoadVec3) -> Self {
        Self {
            xz: keys::SurfaceXzKey::from_world_xz(point),
            y_mm: keys::SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
        }
    }

    fn from_boundary_point(point: ArrangementBoundaryPointKey) -> Self {
        Self {
            xz: keys::SurfaceXzKey::from_raw_keys(point.x_key, point.z_key),
            y_mm: point.y_mm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::node::{
        arrangement::NodeArrangementKey, backend::RoadVec2,
    };

    #[test]
    fn minimal_directed_rotation_matches_exhaustive_reference() {
        for count in 0..=7 {
            let sequence_count = 3_usize.pow(count as u32);
            for encoded in 0..sequence_count {
                let mut remaining = encoded;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push((remaining % 3) as u8);
                    remaining /= 3;
                }

                let mut actual = values.clone();
                if !actual.is_empty() {
                    let start = minimal_directed_rotation_start(&actual);
                    actual.rotate_left(start);
                }
                let expected = (0..values.len())
                    .map(|start| {
                        (0..values.len())
                            .map(|offset| values[(start + offset) % values.len()])
                            .collect::<Vec<_>>()
                    })
                    .min()
                    .unwrap_or_default();
                assert_eq!(
                    actual, expected,
                    "encoded sequence {encoded} of len {count}"
                );
            }
        }
    }

    #[test]
    fn support_index_retains_numeric_dust_candidate_at_negative_tile_boundary() {
        let vertex = |x_key, z_key| NodeTopSupportVertexKey {
            xz: keys::SurfaceXzKey::from_raw_keys(x_key, z_key),
            y_mm: 0,
        };
        let edge = NodeTopSupportEdge {
            owner: NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0),
            region_index: 0,
            start: vertex(-8_000_050, 0),
            end: vertex(-7_999_950, 0),
        };
        let segment_start = keys::SurfaceXzKey::from_raw_keys(-8_000_050, 100);
        let segment_end = keys::SurfaceXzKey::from_raw_keys(-7_999_950, 100);
        assert!(
            support_edge_overlap_interval_on_segment(edge, segment_start, segment_end).is_some(),
            "the exact downstream predicate must accept this numeric-dust offset"
        );

        let mut candidate_indices = Vec::new();
        let candidates = NodeTopSupportEdgeIndex::new(&[edge])
            .support_edge_candidates_on_step_segment(
                RoadSurfaceBandKind::Carriageway,
                segment_start,
                segment_end,
                &mut candidate_indices,
            );
        assert_eq!(
            candidates.len(),
            1,
            "the conservative index must retain every numeric-dust exact candidate"
        );
        assert_eq!(candidates[0].0, edge);
    }

    fn triangle_region(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        triangle: [RoadVec3; 3],
    ) -> NodeOwnedRegion {
        NodeOwnedRegion {
            kind,
            owner_index,
            polygon: RoadSurfaceVisualPolygon::from_parts(triangle.to_vec(), vec![triangle]),
        }
    }

    fn raised_step_fixture() -> (
        NodeOwnedRegion,
        NodeOwnedRegion,
        NodeExplicitVerticalStepSegment,
    ) {
        let lower_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let raised_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let lower = triangle_region(
            RoadSurfaceBandKind::Carriageway,
            0,
            [
                RoadVec3::new(0.0, 0.0, 0.0),
                RoadVec3::new(0.0, 0.0, -1.0),
                RoadVec3::new(2.0, 0.0, 0.0),
            ],
        );
        let raised = triangle_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            [
                RoadVec3::new(0.0, 0.12, 0.0),
                RoadVec3::new(2.0, 0.12, 0.0),
                RoadVec3::new(0.0, 0.12, 1.0),
            ],
        );
        let step = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0)),
            NodeArrangementKey::from_point(RoadVec2::new(2.0, 0.0)),
            lower_owner,
            raised_owner,
        )
        .expect("test step is non-degenerate");
        (lower, raised, step)
    }

    #[test]
    fn raised_step_cache_rebinds_indices_and_drops_removed_support() {
        let (lower, raised, step) = raised_step_fixture();
        let (seed_faces, seed_cache, _) =
            RoadSurfaceSystem::raised_step_faces_with_owned_top_support(
                &[lower.clone(), raised.clone()],
                &[step],
                None,
            );
        assert_eq!(seed_faces.len(), 1);

        let unrelated = triangle_region(
            RoadSurfaceBandKind::Carriageway,
            98,
            [
                RoadVec3::new(-10.0, 0.0, 10.0),
                RoadVec3::new(-9.0, 0.0, 10.0),
                RoadVec3::new(-10.0, 0.0, 11.0),
            ],
        );
        let unrelated_step = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(RoadVec2::new(-10.0, 10.0)),
            NodeArrangementKey::from_point(RoadVec2::new(-9.0, 10.0)),
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 98),
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 99),
        )
        .expect("unrelated test step is non-degenerate");
        let current_regions = [unrelated.clone(), lower.clone(), raised.clone()];
        let current_steps = [unrelated_step, step];
        let (warm_faces, warm_cache, warm_stats) =
            RoadSurfaceSystem::raised_step_faces_with_owned_top_support(
                &current_regions,
                &current_steps,
                Some(&seed_cache),
            );
        let (cold_faces, _, _) = RoadSurfaceSystem::raised_step_faces_with_owned_top_support(
            &current_regions,
            &current_steps,
            None,
        );
        assert_eq!(warm_faces, cold_faces);
        assert!(warm_stats.top_edge_previous_hits >= 2);
        assert_eq!(warm_stats.raised_step_previous_hits, 1);
        let target_source = warm_faces
            .iter()
            .map(|face| face.source)
            .find(|source| source.segment() == step)
            .expect("the target raised-step face must remain present");
        assert_eq!(target_source.explicit_vertical_step_index(), Some(1));

        let removed_regions = [unrelated, lower];
        let (removed_warm, _, removed_stats) =
            RoadSurfaceSystem::raised_step_faces_with_owned_top_support(
                &removed_regions,
                &current_steps,
                Some(&warm_cache),
            );
        let (removed_cold, _, _) = RoadSurfaceSystem::raised_step_faces_with_owned_top_support(
            &removed_regions,
            &current_steps,
            None,
        );
        assert_eq!(removed_warm, removed_cold);
        assert!(removed_warm.is_empty());
        assert!(removed_stats.raised_step_cache_misses > 0);
    }

    #[test]
    fn cached_region_edges_use_current_owner_wide_cancellation() {
        let first = triangle_region(
            RoadSurfaceBandKind::Carriageway,
            7,
            [
                RoadVec3::new(0.0, 0.0, 0.0),
                RoadVec3::new(1.0, 0.0, 0.0),
                RoadVec3::new(1.0, 0.0, 1.0),
            ],
        );
        let second = triangle_region(
            RoadSurfaceBandKind::Carriageway,
            7,
            [
                RoadVec3::new(0.0, 0.0, 0.0),
                RoadVec3::new(1.0, 0.0, 1.0),
                RoadVec3::new(0.0, 0.0, 1.0),
            ],
        );

        let mut both_cache = NodeRaisedStepIncrementalCache::default();
        let mut both_stats = NodeRaisedStepReuseStats::default();
        let both_edges = owned_top_boundary_edges_with_reuse(
            &[first.clone(), second.clone()],
            None,
            &mut both_cache,
            &mut both_stats,
        );
        assert_eq!(both_edges.len(), 4);

        let mut removal_cache = NodeRaisedStepIncrementalCache::default();
        let mut removal_stats = NodeRaisedStepReuseStats::default();
        let removal_warm = owned_top_boundary_edges_with_reuse(
            std::slice::from_ref(&first),
            Some(&both_cache),
            &mut removal_cache,
            &mut removal_stats,
        );
        let mut removal_cold_cache = NodeRaisedStepIncrementalCache::default();
        let removal_cold = owned_top_boundary_edges_with_reuse(
            std::slice::from_ref(&first),
            None,
            &mut removal_cold_cache,
            &mut NodeRaisedStepReuseStats::default(),
        );
        assert_eq!(removal_warm, removal_cold);
        assert_eq!(removal_warm.len(), 3);
        assert_eq!(removal_stats.top_edge_previous_hits, 1);

        let mut addition_cache = NodeRaisedStepIncrementalCache::default();
        let mut addition_stats = NodeRaisedStepReuseStats::default();
        let addition_warm = owned_top_boundary_edges_with_reuse(
            &[first, second],
            Some(&removal_cache),
            &mut addition_cache,
            &mut addition_stats,
        );
        assert_eq!(addition_warm, both_edges);
        assert_eq!(addition_warm.len(), 4);
        assert_eq!(addition_stats.top_edge_previous_hits, 1);
        assert_eq!(addition_stats.top_edge_cache_misses, 1);
    }
}
