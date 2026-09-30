// SPDX-License-Identifier: GPL-2.0-only

//! Explicit visual node-piece construction and incident-edge classification.

#[cfg(test)]
use self::boundary::{ArrangementSegmentParameter, arrangement_boundary_point_to_world};
use self::{
    arrangement::{
        NodeArrangement, NodeArrangementKey, NodeBandOwner, NodeExplicitVerticalStepSegment,
    },
    boundary::{
        ArrangementBoundaryPointKey, NodeBoundaryExportError, NodeFootprintBoundaryExportSources,
        boundary_points_numeric_area_budget_m2,
        node_earthwork_boundary_segments_from_footprint_loops,
    },
    input::NodeInputExtractionError,
    validation::NodeValidationReport,
};
#[cfg(test)]
use super::band_semantics::ordered_raised_step_kinds;
use super::{
    CURB_STEP_HEIGHT_M, CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile,
    IncidentSurfaceEdge, OrderedIncidentPieceMouth, RoadSurfaceBandKind,
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceTerrainClipLoop,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
};
pub(super) use super::{
    IncidentMouthBand, NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M,
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes,
    RoadSurfaceVisualNodeCompileInput, SurfaceCdt,
    backend::{self, RoadVec2, RoadVec3},
    band_semantics, indices, keys, paths, segments,
};

// Fixed-seed hashing for node-compile lookup maps: SipHash with a per-process random seed cost about
// 6% of junction compile CPU (ROAD-40). Hash order must never reach compile products; these maps are
// only probed, or feed sorted and B-tree outputs.
type NodeHashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::FixedState>;
type NodeHashSet<K> = std::collections::HashSet<K, foldhash::fast::FixedState>;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use std::collections::BTreeSet;

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;

struct NodeExportTopHeightContext {
    carriageway_height_keys: BTreeSet<(arrangement::NodeArrangementKey, i64)>,
    flat_carriageway_height_mm: Option<i64>,
    explicit_step_lower_edges: NodeExportExplicitStepLowerEdges,
}

// Explicit steps sorted by raised owner, each pointing at its lower carriageway owner's edges, so a
// raised-vertex query visits only that owner's steps and, for a step containing the vertex, the
// lower owner's edges. Build O(S log S + E log E); the flat step × edge product it replaces made
// every query O(S · E_lower).
struct NodeExportExplicitStepLowerEdges {
    steps: Vec<NodeExportExplicitStep>,
    lower_edges: Vec<(NodeBandOwner, NodeExportLowerEdge)>,
    // The unindexed step × edge product, built as before the index; test builds check every
    // query against it.
    #[cfg(test)]
    reference: Vec<(NodeExportExplicitStep, NodeExportLowerEdge)>,
}

#[derive(Clone, Copy)]
struct NodeExportExplicitStep {
    raised_owner: NodeBandOwner,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
    // Range of `lower_edges` owned by this step's lower carriageway owner.
    lower_edges_start: usize,
    lower_edges_end: usize,
}

#[derive(Clone, Copy)]
struct NodeExportLowerEdge {
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
    start_height_mm: i64,
    end_height_mm: i64,
}

impl NodeExportTopHeightContext {
    fn from_arrangement(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) -> Self {
        let carriageway_height_keys = arrangement
            .vertices()
            .iter()
            .filter(|vertex| {
                vertex
                    .owners()
                    .iter()
                    .any(|owner| owner.kind() == RoadSurfaceBandKind::Carriageway)
            })
            .map(|vertex| (vertex.key(), vertex.height_mm()))
            .collect::<BTreeSet<_>>();
        Self {
            flat_carriageway_height_mm: flat_carriageway_height_mm(&carriageway_height_keys),
            carriageway_height_keys,
            explicit_step_lower_edges: NodeExportExplicitStepLowerEdges::new(
                arrangement,
                explicit_vertical_step_segments,
            ),
        }
    }

    fn flat_raised_top_height_mm(&self, owner: NodeBandOwner) -> Option<i64> {
        if !matches!(
            owner.kind(),
            RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
        ) {
            return None;
        }
        self.flat_carriageway_height_mm
            .map(|height_mm| height_mm + curb_step_height_mm())
    }

    fn raised_owner_vertex_matches_explicit_step_lower_height(
        &self,
        owner: NodeBandOwner,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
    ) -> bool {
        self.explicit_step_lower_edges
            .raised_vertex_matches_lower_height(owner, key, height_mm)
    }
}

fn flat_carriageway_height_mm(
    carriageway_height_keys: &BTreeSet<(arrangement::NodeArrangementKey, i64)>,
) -> Option<i64> {
    let mut heights = carriageway_height_keys
        .iter()
        .map(|(_, height_mm)| *height_mm);
    let first = heights.next()?;
    heights.all(|height_mm| height_mm == first).then_some(first)
}

fn curb_step_height_mm() -> i64 {
    (f64::from(CURB_STEP_HEIGHT_M) * 1000.0).round() as i64
}

impl NodeExportExplicitStepLowerEdges {
    fn new(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) -> Self {
        let owned_steps = explicit_vertical_step_segments
            .iter()
            .copied()
            .filter_map(|segment| {
                explicit_step_lower_and_raised_owner(segment)
                    .map(|(lower_owner, raised_owner)| (lower_owner, raised_owner, segment))
            })
            .collect::<Vec<_>>();
        let mut lower_owners = owned_steps
            .iter()
            .map(|(lower_owner, _, _)| *lower_owner)
            .collect::<Vec<_>>();
        lower_owners.sort_unstable();
        lower_owners.dedup();
        let mut lower_edges = arrangement
            .edges()
            .iter()
            .filter(|edge| lower_owners.binary_search(&edge.owner()).is_ok())
            .filter_map(|edge| {
                let start = arrangement.vertices().get(edge.start().index())?;
                let end = arrangement.vertices().get(edge.end().index())?;
                Some((
                    edge.owner(),
                    NodeExportLowerEdge {
                        start: start.key(),
                        end: end.key(),
                        start_height_mm: start.height_mm(),
                        end_height_mm: end.height_mm(),
                    },
                ))
            })
            .collect::<Vec<_>>();
        lower_edges.sort_by_key(|(owner, _)| *owner);
        let mut steps = owned_steps
            .into_iter()
            .map(
                |(lower_owner, raised_owner, segment)| NodeExportExplicitStep {
                    raised_owner,
                    start: segment.start(),
                    end: segment.end(),
                    lower_edges_start: lower_edges
                        .partition_point(|(owner, _)| *owner < lower_owner),
                    lower_edges_end: lower_edges
                        .partition_point(|(owner, _)| *owner <= lower_owner),
                },
            )
            .collect::<Vec<_>>();
        steps.sort_by_key(|step| step.raised_owner);
        Self {
            #[cfg(test)]
            reference: if EXPLICIT_STEP_REFERENCE_CHECK.with(std::cell::Cell::get) {
                reference_step_lower_edges(arrangement, explicit_vertical_step_segments)
            } else {
                Vec::new()
            },
            steps,
            lower_edges,
        }
    }

    fn raised_vertex_matches_lower_height(
        &self,
        raised_owner: NodeBandOwner,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
    ) -> bool {
        let first = self
            .steps
            .partition_point(|step| step.raised_owner < raised_owner);
        let matches = self.steps[first..]
            .iter()
            .take_while(|step| step.raised_owner == raised_owner)
            .filter(|step| key.lies_on_segment(step.start, step.end))
            .any(|step| {
                self.lower_edges[step.lower_edges_start..step.lower_edges_end]
                    .iter()
                    .any(|(_, edge)| edge.lower_height_mm_at(key) == Some(height_mm))
            });
        #[cfg(test)]
        {
            if EXPLICIT_STEP_REFERENCE_CHECK.with(std::cell::Cell::get) {
                assert_eq!(
                    matches,
                    self.reference_matches(raised_owner, key, height_mm),
                    "indexed explicit-step lower-height match diverged from the full scan"
                );
            }
            EXPLICIT_STEP_MATCH_STATS.with(|stats| {
                let (queries, hits) = stats.get();
                stats.set((queries + 1, hits + usize::from(matches)));
            });
        }
        matches
    }

    #[cfg(test)]
    fn reference_matches(
        &self,
        raised_owner: NodeBandOwner,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
    ) -> bool {
        self.reference.iter().any(|(step, edge)| {
            step.raised_owner == raised_owner
                && key.lies_on_segment(step.start, step.end)
                && edge.lower_height_mm_at(key) == Some(height_mm)
        })
    }
}

#[cfg(test)]
thread_local! {
    // (queries, matches) of explicit-step lower-height lookups on this thread; export runs them
    // serially on the compiling thread.
    static EXPLICIT_STEP_MATCH_STATS: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
    // Whether export on this thread also answers every lookup with the unindexed full scan. Timing
    // harnesses turn it off so the O(S * E) reference does not dilute their measurements.
    static EXPLICIT_STEP_REFERENCE_CHECK: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Enables or disables the full-scan reference check for export on this thread.
#[cfg(test)]
pub(in crate::simulation::network::surface) fn set_explicit_step_reference_check(enabled: bool) {
    EXPLICIT_STEP_REFERENCE_CHECK.with(|check| check.set(enabled));
}

/// Returns and resets this thread's explicit-step lower-height (queries, matches) counts.
#[cfg(test)]
pub(in crate::simulation::network::surface) fn take_explicit_step_match_stats() -> (usize, usize) {
    EXPLICIT_STEP_MATCH_STATS.with(|stats| stats.replace((0, 0)))
}

#[cfg(test)]
fn reference_step_lower_edges(
    arrangement: &NodeArrangement,
    explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
) -> Vec<(NodeExportExplicitStep, NodeExportLowerEdge)> {
    let mut lower_edges = Vec::new();
    for segment in explicit_vertical_step_segments.iter().copied() {
        let Some((lower_owner, raised_owner)) = explicit_step_lower_and_raised_owner(segment)
        else {
            continue;
        };
        let step = NodeExportExplicitStep {
            raised_owner,
            start: segment.start(),
            end: segment.end(),
            lower_edges_start: 0,
            lower_edges_end: 0,
        };
        lower_edges.extend(
            arrangement
                .edges()
                .iter()
                .filter(|edge| edge.owner() == lower_owner)
                .filter_map(|edge| {
                    let start = arrangement.vertices().get(edge.start().index())?;
                    let end = arrangement.vertices().get(edge.end().index())?;
                    Some((
                        step,
                        NodeExportLowerEdge {
                            start: start.key(),
                            end: end.key(),
                            start_height_mm: start.height_mm(),
                            end_height_mm: end.height_mm(),
                        },
                    ))
                }),
        );
    }
    lower_edges
}

impl NodeExportLowerEdge {
    fn lower_height_mm_at(self, key: arrangement::NodeArrangementKey) -> Option<i64> {
        if !key.lies_on_segment(self.start, self.end) {
            return None;
        }
        let parameter = segments::overlay_segment_parameter(
            segments::arrangement_key(key),
            segments::arrangement_key(self.start),
            segments::arrangement_key(self.end),
        )
        .or_else(|| {
            segments::exact_line_parameter(
                segments::arrangement_key(key),
                segments::arrangement_key(self.start),
                segments::arrangement_key(self.end),
            )
        })?;
        Some(segments::interpolate_height_i64(
            self.start_height_mm,
            self.end_height_mm,
            parameter,
        ))
    }
}

// Returns the (lower, raised) owners of a step whose lower side is carriageway; other steps never
// lift a raised vertex.
fn explicit_step_lower_and_raised_owner(
    segment: NodeExplicitVerticalStepSegment,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let owners = [segment.owner(), segment.opposite_owner()];
    let lower_owner = owners
        .into_iter()
        .find(|owner| segment.owner_matches_height_side(*owner, true))?;
    if lower_owner.kind() != RoadSurfaceBandKind::Carriageway {
        return None;
    }
    let raised_owner = owners
        .into_iter()
        .find(|owner| segment.owner_matches_height_side(*owner, false))?;
    Some((lower_owner, raised_owner))
}

fn node_export_top_height_m(
    owner: NodeBandOwner,
    source_kind: RoadSurfaceBandKind,
    key: arrangement::NodeArrangementKey,
    height_m: f64,
    height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> f64 {
    let height_mm = node_export_top_height_mm(owner, source_kind, key, height_mm, context);
    if height_mm == keys::SurfaceHeightMmKey::from_m_f64(height_m).as_i64() {
        height_m
    } else {
        height_mm as f64 / keys::SURFACE_MM_PER_M
    }
}

fn node_export_top_height_mm(
    owner: NodeBandOwner,
    source_kind: RoadSurfaceBandKind,
    key: arrangement::NodeArrangementKey,
    height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> i64 {
    if let Some(flat_raised_top_height_mm) = context.flat_raised_top_height_mm(owner) {
        return flat_raised_top_height_mm;
    }
    if node_export_top_height_needs_curb_lift(owner, source_kind, key, height_mm, context) {
        height_mm + curb_step_height_mm()
    } else {
        height_mm
    }
}

fn node_export_top_height_needs_curb_lift(
    owner: NodeBandOwner,
    source_kind: RoadSurfaceBandKind,
    key: arrangement::NodeArrangementKey,
    height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> bool {
    if !matches!(
        owner.kind(),
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    ) {
        return false;
    }
    source_kind == RoadSurfaceBandKind::Carriageway
        || context.raised_owner_vertex_matches_explicit_step_lower_height(owner, key, height_mm)
}

fn node_export_top_source_kind(
    owner: NodeBandOwner,
    fallback_source_kind: RoadSurfaceBandKind,
    vertex_key: arrangement::NodeArrangementKey,
    vertex_height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> RoadSurfaceBandKind {
    if matches!(
        owner.kind(),
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    ) && (context
        .carriageway_height_keys
        .contains(&(vertex_key, vertex_height_mm))
        || context.raised_owner_vertex_matches_explicit_step_lower_height(
            owner,
            vertex_key,
            vertex_height_mm,
        ))
    {
        RoadSurfaceBandKind::Carriageway
    } else {
        fallback_source_kind
    }
}

pub(crate) mod arrangement;
#[cfg(test)]
mod arrangement_faces;
pub(crate) mod boundary;
#[cfg(test)]
mod boundary_edges;
mod compile;
mod export;
pub(crate) mod height;
mod incident;
pub(crate) mod input;
pub(crate) mod joins;
pub(crate) mod ownership;
mod piece;
pub(crate) mod rails;
pub(crate) mod terminal;
#[cfg(test)]
mod tests;
pub(crate) mod triangulation;
pub(crate) mod validation;
mod vertical_faces;

pub(crate) use compile::{NodeCanonicalTopologyCache, NodeVisualCompileResult};
pub(crate) use joins::rounded_sidewalk_corner_path_xz;
pub use piece::RoadSurfaceVisualNodePiece;
pub(crate) use piece::{
    NodeBooleanDebugSnapshot, NodeCornerTrimDebug, NodeCornerTrimSideJoinIntersectionDebug,
    NodeEarthworkOwnerSource, NodeFootprintBoundaryDirectSource,
    NodeFootprintBoundarySegmentSource, NodeFootprintBoundaryVertexSource, NodeOwnedRegion,
    NodePostBooleanOwnedRegionDebug, NodeSideJoinContourDebug, NodeSideJoinGapDebug,
    NodeSideJoinMaterialTrimDebug, NodeSurfaceRegionResult, NodeTopSurfacePolygonSource,
    NodeTopSurfaceVertexSource, RoadSurfaceVerticalFaceSource,
};
pub(in crate::simulation::network::surface::node) use vertical_faces::RoadSurfaceRaisedStepFace;
