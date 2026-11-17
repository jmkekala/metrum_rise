// SPDX-License-Identifier: GPL-2.0-only

//! Road placement validation: stroke length, profile grade and clearance, endpoint snapping,
//! same-node connections and the `ROAD-54` edit geometry limits, the 1:3 maximum grade among
//! them, on the bounded post-edit graph excerpt that also yields the shared
//! [`RoadTopologyPlan`].

use super::bridge::{
    BRIDGE_TOO_LOW_REASON, ROAD_UNDER_BRIDGE_M, TUNNEL_JOIN_REASON, deck_to_deck_m,
};
use super::input::{
    PREVIEW_CLEARANCE_M, PreparedRoadInput, ROAD_PROFILE_MAX_GRADE, is_surface_edge,
};
use super::tunnel::{
    BoreRefusal, JUNCTION_IN_TRENCH_REASON, Portals, ROAD_OVER_TUNNEL_M, RoadDeck, TUNNEL_COVER_M,
    TUNNEL_CROSSING_REASON, TUNNEL_MIN_APPROACH_M, TUNNEL_TOO_CLOSE_REASON, bore_crosses_a_tunnel,
    classify_tunnel_stroke, deck_over_tunnels_m, grounded_decks_near, junction_in_trench,
    portal_splits, section_cover_m, tunnel_bore, tunnel_section_half_width_m,
};
use super::{FinalizedRoadGeometry, PlannedRoadSplit, RoadTopologyPlan};
use crate::simulation::network::roadbed;
use crate::simulation::network::topology::JointDissolve;
use crate::config;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::{Edge, RegionGraph, RoadMedian, RoadParking};
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::roadbed::{
    ArmKey, EdgeEnd, LayoutError, NodeError, NodeRole, arm_edge_needed_m, check_node, node_role,
    preview_candidate_roadbed_half_width_m,
};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags};
use crate::simulation::network::grade::{ROAD_GRADE_MAX, SteepestGrade, steepest_grade};
use crate::simulation::network::validate::{
    RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, centreline, grade_violation,
};
use crate::simulation::network::{
    TransitNetwork, build_surface_edge, road_insert_surface_dirty_nodes, roundabouts, topology,
    topology::StrokeGrade,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector3;
use std::collections::{HashMap, HashSet};

const PREVIEW_VALID_REASON: &str = "";
const PREVIEW_BRIDGE_CLEARANCE_REASON: &str = "bridge_clearance";
const PREVIEW_TUNNEL_COVER_REASON: &str = "tunnel_cover";
const PREVIEW_TUNNEL_APPROACH_REASON: &str = "tunnel_approach_too_short";
const PREVIEW_SURFACE_GEOMETRY_REASON: &str = "surface_geometry_invalid";
const PREVIEW_SAME_NODE_REASON: &str = "same_node_connection";
const PREVIEW_TOO_SHORT_REASON: &str = "too_short";
const STROKE_SPLITS_POCKET_REASON: &str = "stroke_splits_pocket";
const ROUNDABOUT_CROSSED_REASON: &str = "roundabout_crossed";
const ROUNDABOUT_ARM_NODE_TAKEN_REASON: &str = "roundabout_arm_node_taken";
const ROUNDABOUT_ARMS_TOO_CLOSE_REASON: &str = "roundabout_arms_too_close";
const ROUNDABOUT_NEIGHBOUR_NODES_REASON: &str = "roundabout_neighbour_nodes";
const PREVIEW_GRADE_CHANGE_REASON: &str = "road_grade_change";
const PREVIEW_TOO_STEEP_REASON: &str = "road_too_steep";
// The road tool's warning for a grade above its class's design tier, which it does not refuse.
const PREVIEW_STEEP_WARNING: &str = "road_steep";
const PREVIEW_TIGHT_CURVE_REASON: &str = "road_tight_curve";
const PREVIEW_JUNCTION_ANGLE_REASON: &str = "junction_arm_angle";
const PREVIEW_JUNCTION_DEGREE_REASON: &str = "junction_degree";
const PREVIEW_JUNCTION_CORNER_REASON: &str = "junction_corner_does_not_fit";
const PREVIEW_JUNCTION_CORE_REASON: &str = "junction_core_not_simple";
const PREVIEW_JUNCTION_BANDS_REASON: &str = "junction_band_mismatch";
const PREVIEW_JUNCTION_ARM_REASON: &str = "junction_arm_too_short";
const PREVIEW_JUNCTION_PROFILE_REASON: &str = "junction_profile_mismatch";
const PREVIEW_JUNCTION_LAYOUT_REASON: &str = "junction_layout_invalid";
/// The shortest stroke the checks accept, metres of horizontal length (`too_short`).
pub(crate) const PREVIEW_MIN_ENDPOINT_SEGMENT_M: f32 = 2.0;
const PREVIEW_BRIDGE_GROUND_TOLERANCE_M: f32 = 0.05;
// Degenerate profile steps shorter than this add no length.
const SAMPLE_EPSILON_M: f32 = 0.001;

struct ValidationGraph {
    graph: RegionGraph,
    new_edge_idx: usize,
    required_edge_ids: Vec<usize>,
    source_node_ids: HashMap<u32, u32>,
    source_edge_ids: HashMap<usize, usize>,
    finalized: FinalizedRoadGeometry,
    splits: Vec<PlannedRoadSplit>,
    // Local nodes at the stroke's first and last point.
    stroke_ends: [u32; 2],
    // The joints dissolved (`ROAD-118` round JS2), in the order done.
    dissolves: Vec<JointDissolve>,
}

/// Machine-readable road-placement validation state shared by the UI and commit guard.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadPreviewValidation {
    /// True when the stroke can be placed as a road under the current road-profile rules.
    pub(crate) is_valid: bool,
    /// Stable reason key for invalid strokes, or an empty string when valid.
    pub(crate) invalid_reason: &'static str,
    /// The steepest grade (`grade::steepest_grade`) of the profiles the edit builds: the
    /// drawn road's finalized pieces and every profile it re-derives steeper than before, or,
    /// in the hover check and a refused stroke, the prepared profile's.
    pub(crate) max_grade: f32,
    /// Standard-road profile-solver grade target retained for diagnostics.
    pub(crate) allowed_grade: f32,
    /// `road_steep` when `max_grade` exceeds its road class's design grade (`ROAD-54`; 16% for
    /// streets): shown by the road tool, never refused. Empty otherwise.
    pub(crate) warning_reason: &'static str,
    /// The design grade `max_grade` exceeds, or zero without a warning.
    pub(crate) warning_limit: f32,
    /// Start station of the steepest measured span, in meters along the road.
    pub(crate) offending_span_start_m: f32,
    /// End station of the steepest measured span, in meters along the road.
    pub(crate) offending_span_end_m: f32,
    /// Horizontal length of the steepest span, in metres.
    pub(crate) offending_span_run_m: f32,
    /// Signed height delta across the steepest span, in metres.
    pub(crate) offending_span_height_delta_m: f32,
    /// Road center height at the start of the steepest span.
    pub(crate) offending_span_start_height_m: f32,
    /// Road center height at the end of the steepest span.
    pub(crate) offending_span_end_height_m: f32,
    /// Source terrain height at the start of the steepest span.
    pub(crate) offending_span_start_terrain_height_m: f32,
    /// Source terrain height at the end of the steepest span.
    pub(crate) offending_span_end_terrain_height_m: f32,
    /// Road-minus-terrain delta at the start of the steepest span.
    pub(crate) offending_span_start_support_delta_m: f32,
    /// Road-minus-terrain delta at the end of the steepest span.
    pub(crate) offending_span_end_support_delta_m: f32,
    /// Existing graph node snapped to the start endpoint, or -1 when the start is open terrain.
    pub(crate) start_endpoint_snapped_node_id: i32,
    /// Existing graph node snapped to the end endpoint, or -1 when the end is open terrain.
    pub(crate) end_endpoint_snapped_node_id: i32,
    /// Road height at the start endpoint after profile preparation and endpoint snapping.
    pub(crate) start_endpoint_height_m: f32,
    /// Road height at the end endpoint after profile preparation and endpoint snapping.
    pub(crate) end_endpoint_height_m: f32,
    /// Source terrain height below the start endpoint.
    pub(crate) start_endpoint_terrain_height_m: f32,
    /// Source terrain height below the end endpoint.
    pub(crate) end_endpoint_terrain_height_m: f32,
    /// Start endpoint road-minus-terrain delta; positive means fill.
    pub(crate) start_endpoint_support_delta_m: f32,
    /// End endpoint road-minus-terrain delta; positive means fill.
    pub(crate) end_endpoint_support_delta_m: f32,
    /// Measured bridge/tunnel clearance at the sampled validation point.
    pub(crate) clearance_m: f32,
    /// Required bridge/tunnel clearance at the sampled validation point.
    pub(crate) required_clearance_m: f32,
    /// Measured value of the geometry check that refused the road (`ROAD-54`): grade, grade
    /// change per metre, curve radius in metres, junction arm angle in degrees or junction arm
    /// count.
    pub(crate) geometry_measured: f32,
    /// The limit that measurement broke, in the same unit. For a junction that does not fit
    /// (`ROAD-118`), the failing arm's road length as measured and the shortest that fits as the
    /// limit (0 when no length fits), rounded up to 0.1 m.
    pub(crate) geometry_limit: f32,
    /// The place a junction refusal names (`ROAD-118` round JS1); `None` for any other verdict.
    pub(crate) junction_place: Option<JunctionPlace>,
    /// Each stroke end's road where a refused stroke ends on one between its nodes, start then
    /// end: the live edge id and the end's station along it from its start node, horizontal
    /// metres. `None` at a node, on open terrain and on accepted strokes.
    pub(crate) endpoint_edges: [Option<(usize, f32)>; 2],
}

/// Where a junction refusal happened (`ROAD-118` round JS1), in the committed graph's ids where
/// the edit keeps them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct JunctionPlace {
    /// The node whose layout fails: its live id, `None` for a node the stroke makes.
    pub(crate) node: Option<u32>,
    /// Where that node stands.
    pub(crate) position: Vector3,
    /// The road the failing arm lies on: its live id (a split piece names the road it splits),
    /// `None` for a piece of the stroke.
    pub(crate) arm_edge: Option<usize>,
    /// The failing node's role and the role of the node across the failing arm, after the edit.
    pub(crate) roles: [NodeRole; 2],
}

impl RoadPreviewValidation {
    /// Creates a successful validation result with the measured maximum grade.
    pub(crate) fn valid(max_grade: f32) -> Self {
        Self {
            is_valid: true,
            invalid_reason: PREVIEW_VALID_REASON,
            max_grade,
            allowed_grade: ROAD_PROFILE_MAX_GRADE,
            warning_reason: "",
            warning_limit: 0.0,
            offending_span_start_m: 0.0,
            offending_span_end_m: 0.0,
            offending_span_run_m: 0.0,
            offending_span_height_delta_m: 0.0,
            offending_span_start_height_m: 0.0,
            offending_span_end_height_m: 0.0,
            offending_span_start_terrain_height_m: 0.0,
            offending_span_end_terrain_height_m: 0.0,
            offending_span_start_support_delta_m: 0.0,
            offending_span_end_support_delta_m: 0.0,
            start_endpoint_snapped_node_id: -1,
            end_endpoint_snapped_node_id: -1,
            start_endpoint_height_m: 0.0,
            end_endpoint_height_m: 0.0,
            start_endpoint_terrain_height_m: 0.0,
            end_endpoint_terrain_height_m: 0.0,
            start_endpoint_support_delta_m: 0.0,
            end_endpoint_support_delta_m: 0.0,
            clearance_m: 0.0,
            required_clearance_m: 0.0,
            geometry_measured: 0.0,
            geometry_limit: 0.0,
            junction_place: None,
            endpoint_edges: [None; 2],
        }
    }

    /// Replaces a valid result with one stable machine-readable rejection reason.
    pub(crate) fn with_invalid_reason(mut self, invalid_reason: &'static str) -> Self {
        self.is_valid = false;
        self.invalid_reason = invalid_reason;
        self
    }
}

/// Validates a road-tool candidate using only prepared-profile samples and local endpoint graph
/// checks, for synchronous editor feedback; the lanes size a tunnel stroke's section. Reads the
/// local excerpt's tunnels and grounded roads for the crossings over a bore (`ROAD-61` round
/// R8h). O(prepared points + endpoint snap lookups + the tunnel rules' excerpt work).
pub(crate) fn validate_road_candidate_fast(
    prepared_input: &PreparedRoadInput,
    (fwd_lanes, bkw_lanes): (u8, u8),
    terrain: &TerrainSystem,
    existing_graph: &RegionGraph,
) -> RoadPreviewValidation {
    let lanes = (fwd_lanes, bkw_lanes);
    let mut validation = profile_validation(prepared_input, lanes, terrain, existing_graph, true);
    record_endpoint_height_debug(&mut validation, &prepared_input.points, terrain);
    let (start_existing, end_existing) = record_endpoint_snap_debug(
        &mut validation,
        &prepared_input.points,
        existing_graph,
        prepared_input.endpoint_snap_enabled,
    );
    if !validation.is_valid || prepared_input.points.len() < 2 {
        return validation;
    }
    if start_existing.is_some() && start_existing == end_existing {
        return validation.with_invalid_reason(PREVIEW_SAME_NODE_REASON);
    }
    refuse_close_crossing(validation, prepared_input, lanes, existing_graph)
}

/// Validates a prepared stroke against the committed graph and solves its local post-edit
/// topology. The plan is `None` when validation fails or the excerpt's mutation frontier is
/// incomplete. O(stroke points + local excerpt records), independent of distant roads.
pub(crate) fn validate_road_edit(
    prepared_input: &PreparedRoadInput,
    fwd_lanes: u8,
    bkw_lanes: u8,
    terrain: &TerrainSystem,
    existing_graph: &RegionGraph,
) -> (RoadPreviewValidation, Option<RoadTopologyPlan>) {
    let new_edge_validation = profile_validation(
        prepared_input,
        (fwd_lanes, bkw_lanes),
        terrain,
        existing_graph,
        false,
    );
    if stroke_too_short(&prepared_input.points) {
        return (
            new_edge_validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON),
            None,
        );
    }
    let (mut validation, plan) = validate_against_graph(
        prepared_input,
        fwd_lanes,
        bkw_lanes,
        terrain,
        existing_graph,
        new_edge_validation,
    );
    if !validation.is_valid {
        record_endpoint_edges(&mut validation, &prepared_input.points, existing_graph);
    }
    (validation, plan)
}

// Where a refused stroke's ends stand on a road between its nodes (`ROAD-118`): the road the
// topology's end join takes, the nearest centreline within `INTERSECTION_TOLERANCE` (lowest id
// on a tie), and the station of the nearest point along it. For the refusal report only.
// O(log edges + nearby edges x points).
fn record_endpoint_edges(
    validation: &mut RoadPreviewValidation,
    prepared_points: &[Vector3],
    existing_graph: &RegionGraph,
) {
    let (Some(&start), Some(&end)) = (prepared_points.first(), prepared_points.last()) else {
        return;
    };
    let nodes = [
        validation.start_endpoint_snapped_node_id,
        validation.end_endpoint_snapped_node_id,
    ];
    for (index, point) in [start, end].into_iter().enumerate() {
        if nodes[index] >= 0 {
            continue;
        }
        let reach = Vector3::new(
            config::INTERSECTION_TOLERANCE,
            0.0,
            config::INTERSECTION_TOLERANCE,
        );
        let mut best: Option<(f32, usize, f32)> = None;
        for edge_idx in existing_graph.get_edges_near_aabb(point - reach, point + reach) {
            let edge = existing_graph.edge(edge_idx);
            if !is_surface_edge(edge) {
                continue;
            }
            let Some((distance, station)) = horizontal_projection(&edge.geometry, point) else {
                continue;
            };
            if distance <= config::INTERSECTION_TOLERANCE
                && best.is_none_or(|(d, id, _)| (distance, edge_idx) < (d, id))
            {
                best = Some((distance, edge_idx, station));
            }
        }
        validation.endpoint_edges[index] = best.map(|(_, edge_idx, station)| (edge_idx, station));
    }
}

// The horizontal distance from `point` to a polyline and the station of the nearest point along
// it. O(points).
fn horizontal_projection(points: &[Vector3], point: Vector3) -> Option<(f32, f32)> {
    let mut station = 0.0;
    let mut best: Option<(f32, f32)> = None;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dz) = (b.x - a.x, b.z - a.z);
        let length = dx.hypot(dz);
        let t = if length > 0.0 {
            (((point.x - a.x) * dx + (point.z - a.z) * dz) / (length * length)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let distance = (a.x + t * dx - point.x).hypot(a.z + t * dz - point.z);
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, station + t * length));
        }
        station += length;
    }
    best
}

// The one road length rule: fewer than two points, or a horizontal length, ignoring
// degenerate steps, below the endpoint minimum.
fn stroke_too_short(points: &[Vector3]) -> bool {
    let length_m = points
        .windows(2)
        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z))
        .filter(|&run| run > SAMPLE_EPSILON_M)
        .fold(0.0, |length, run| length + run);
    points.len() < 2 || length_m < PREVIEW_MIN_ENDPOINT_SEGMENT_M
}

// Steepest prepared span, the grade the stroke's constraints force, bridge / tunnel clearance
// of one profile and a grounded road's clearance over the tunnels of `graph` it crosses
// (`ROAD-61` round R8h). With `require_length` (the fast editor check) a stroke below the
// minimum length is also refused; the full check applies that rule once, to the validation
// corridor.
fn profile_validation(
    prepared_input: &PreparedRoadInput,
    (fwd_lanes, bkw_lanes): (u8, u8),
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    require_length: bool,
) -> RoadPreviewValidation {
    let (edge_class, prepared_points) = (prepared_input.class, &prepared_input.points[..]);
    if prepared_points.len() < 2 && !require_length {
        return RoadPreviewValidation::valid(0.0);
    }
    let mut validation = RoadPreviewValidation::valid(0.0);
    record_endpoint_height_debug(&mut validation, prepared_points, terrain);
    if prepared_points.len() < 2 {
        return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
    }
    if let Some(steepest) = steepest_grade(prepared_points) {
        record_steepest_span(&mut validation, prepared_points, steepest, terrain);
    }

    if require_length && stroke_too_short(prepared_points) {
        return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
    }
    // The stroke's median takes the in-place edit's rule (`ROAD-80`, `ROAD-92`): a car road
    // with both directions. Kerbside parking needs a car road and its side's sidewalk
    // (`PARK-05`).
    let allowed_types = if fwd_lanes > 0 || bkw_lanes > 0 {
        TransitFlags::CAR
    } else {
        TransitFlags::FOOT
    };
    if let Err(reason) =
        Edge::validate_median(allowed_types, fwd_lanes, bkw_lanes, prepared_input.median)
            .and_then(|()| {
                Edge::validate_parking(
                    allowed_types,
                    fwd_lanes,
                    bkw_lanes,
                    prepared_input.parking,
                    prepared_input.attributes.sidewalks,
                )
            })
    {
        return validation.with_invalid_reason(reason);
    }
    // An end snapped onto a tunnel inside its bore (`ROAD-99` follow-up B1): roads meet a tunnel
    // at its portals.
    if (prepared_input.joined_spans.into_iter().flatten())
        .any(|span| graph.edge(span).class == EdgeClass::Tunnel)
    {
        return validation.with_invalid_reason(TUNNEL_JOIN_REASON);
    }
    if let Some(forced_grade) = prepared_input.forced_grade {
        let mut validation = validation.with_invalid_reason(PREVIEW_TOO_STEEP_REASON);
        validation.geometry_measured = forced_grade;
        validation.geometry_limit = ROAD_GRADE_MAX;
        return validation;
    }
    if prepared_input.tunnel {
        let half_width_m = tunnel_section_half_width_m(
            prepared_points,
            fwd_lanes,
            bkw_lanes,
            prepared_input.median,
            prepared_input.parking,
            prepared_input.attributes,
        );
        let decks = grounded_decks_near(graph, prepared_points, half_width_m);
        return tunnel_profile_validation(
            validation,
            prepared_points,
            half_width_m,
            terrain,
            &decks,
        );
    }
    if edge_class == EdgeClass::Bridge
        && bridge_profile_is_ground_transition(prepared_points, terrain)
    {
        validation.clearance_m = prepared_points
            .iter()
            .map(|point| point.y - terrain_height_m(terrain, point.x, point.z))
            .fold(f32::INFINITY, f32::min);
        validation.required_clearance_m = 0.0;
        if validation.clearance_m < -PREVIEW_BRIDGE_GROUND_TOLERANCE_M {
            return validation.with_invalid_reason(PREVIEW_BRIDGE_CLEARANCE_REASON);
        }
    } else if prepared_points.len() > 2 {
        let mid = prepared_points[prepared_points.len() / 2];
        let terrain_h = terrain_height_m(terrain, mid.x, mid.z);
        match edge_class {
            EdgeClass::Bridge => {
                validation.clearance_m = mid.y - terrain_h;
                validation.required_clearance_m = PREVIEW_CLEARANCE_M;
                if validation.clearance_m < PREVIEW_CLEARANCE_M {
                    return validation.with_invalid_reason(PREVIEW_BRIDGE_CLEARANCE_REASON);
                }
            }
            // A stroke without the tunnel intent is never a tunnel (`ROAD-61` round R8f).
            EdgeClass::Tunnel | EdgeClass::Standard => {}
        }
    }

    validation
}

// The clearance rules of the crossings a stroke makes without a node, against the structures of
// `graph`. A grounded stroke over a tunnel (`ROAD-61` round R8h): a grade-separated crossing while
// its prepared deck stands at least `ROAD_OVER_TUNNEL_M` (5.5 m) above the bore's deck over the
// bore's whole section (`deck_over_tunnels_m`), else `tunnel_too_close`. A grounded or bridge
// stroke and a bridge, or a bridge stroke over a grounded road (`ROAD-99`): the higher deck at
// least `ROAD_UNDER_BRIDGE_M` (4.85 m) above the lower wherever their footprints overlap
// (`deck_to_deck_m`), else `bridge_too_low`. Each refusal takes the least height as the
// clearance. Quick validation and the exact plan read the same prepared points and excerpt, so
// they agree; the exact plan checks them after the junction rules, which the hover check does not
// run. A bridge stands above a bore, and a tunnel stroke's bore meets no tunnel
// (`bore_crosses_a_tunnel`).
fn refuse_close_crossing(
    mut validation: RoadPreviewValidation,
    prepared_input: &PreparedRoadInput,
    (fwd_lanes, bkw_lanes): (u8, u8),
    graph: &RegionGraph,
) -> RoadPreviewValidation {
    let points = &prepared_input.points[..];
    let class = prepared_input.class;
    if !validation.is_valid
        || prepared_input.tunnel
        || class == EdgeClass::Tunnel
        || points.len() < 2
    {
        return validation;
    }
    let half_width_m = preview_candidate_roadbed_half_width_m(
        points,
        class,
        fwd_lanes,
        bkw_lanes,
        prepared_input.median,
        prepared_input.parking,
        prepared_input.attributes,
    );
    let over_tunnel = (class == EdgeClass::Standard)
        .then(|| deck_over_tunnels_m(points, half_width_m, graph))
        .flatten()
        .filter(|&above_m| above_m < ROAD_OVER_TUNNEL_M)
        .map(|above_m| (above_m, ROAD_OVER_TUNNEL_M, TUNNEL_TOO_CLOSE_REASON));
    let refusal = over_tunnel.or_else(|| {
        deck_to_deck_m(prepared_input, half_width_m, graph)
            .filter(|&apart_m| apart_m < ROAD_UNDER_BRIDGE_M)
            .map(|apart_m| (apart_m, ROAD_UNDER_BRIDGE_M, BRIDGE_TOO_LOW_REASON))
    });
    let Some((clearance_m, required_clearance_m, reason)) = refusal else {
        return validation;
    };
    validation.clearance_m = clearance_m;
    validation.required_clearance_m = required_clearance_m;
    validation.with_invalid_reason(reason)
}

// The cover check of a stroke drawn with the tunnel intent (`ROAD-61`), on its own knots as the
// exact plan splits it, across the section `half_width_m` to either side (round R8f), under the
// ground or the `decks` of the grounded roads over it (round R8h): the least cover between the
// portals as the clearance, or the refusal.
// O(knots x half_width_m / TUNNEL_LATTICE_M x deck profile points), allocation-free.
fn tunnel_profile_validation(
    mut validation: RoadPreviewValidation,
    points: &[Vector3],
    half_width_m: f32,
    terrain: &TerrainSystem,
    decks: &[RoadDeck],
) -> RoadPreviewValidation {
    validation.required_clearance_m = TUNNEL_COVER_M;
    match tunnel_bore(points, half_width_m, terrain, decks) {
        Ok([first, last]) => {
            validation.clearance_m = (first..=last)
                .map(|index| section_cover_m(terrain, decks, points, index, half_width_m))
                .fold(f32::INFINITY, f32::min);
            validation
        }
        Err(BoreRefusal::Cover { cover_m, .. }) => {
            validation.clearance_m = cover_m;
            validation.with_invalid_reason(PREVIEW_TUNNEL_COVER_REASON)
        }
        Err(BoreRefusal::Approach { length_m }) => {
            validation.geometry_measured = length_m;
            validation.geometry_limit = TUNNEL_MIN_APPROACH_M;
            validation.with_invalid_reason(PREVIEW_TUNNEL_APPROACH_REASON)
        }
    }
}

// Records the steepest segment of `points` as the validation's grade and offending span.
fn record_steepest_span(
    validation: &mut RoadPreviewValidation,
    points: &[Vector3],
    steepest: SteepestGrade,
    terrain: &TerrainSystem,
) {
    let (a, b) = (points[steepest.segment], points[steepest.segment + 1]);
    validation.max_grade = steepest.grade;
    validation.offending_span_start_m = steepest.station_m;
    validation.offending_span_end_m = steepest.station_m + steepest.length_m;
    validation.offending_span_run_m = (b.x - a.x).hypot(b.z - a.z);
    validation.offending_span_height_delta_m = b.y - a.y;
    validation.offending_span_start_height_m = a.y;
    validation.offending_span_end_height_m = b.y;
    validation.offending_span_start_terrain_height_m = terrain_height_m(terrain, a.x, a.z);
    validation.offending_span_end_terrain_height_m = terrain_height_m(terrain, b.x, b.z);
    validation.offending_span_start_support_delta_m =
        a.y - validation.offending_span_start_terrain_height_m;
    validation.offending_span_end_support_delta_m =
        b.y - validation.offending_span_end_terrain_height_m;
}

fn bridge_profile_is_ground_transition(
    prepared_points: &[Vector3],
    terrain: &TerrainSystem,
) -> bool {
    let (Some(first), Some(last)) = (prepared_points.first(), prepared_points.last()) else {
        return false;
    };
    let first_clearance = first.y - terrain_height_m(terrain, first.x, first.z);
    let last_clearance = last.y - terrain_height_m(terrain, last.x, last.z);
    (first_clearance.abs() <= PREVIEW_CLEARANCE_M && last_clearance > PREVIEW_CLEARANCE_M)
        || (last_clearance.abs() <= PREVIEW_CLEARANCE_M && first_clearance > PREVIEW_CLEARANCE_M)
}

fn terrain_height_m(terrain: &TerrainSystem, x: f32, z: f32) -> f32 {
    terrain.sample_height_world(x, z) * config::HEIGHT_SCALE
}

fn record_endpoint_height_debug(
    validation: &mut RoadPreviewValidation,
    prepared_points: &[Vector3],
    terrain: &TerrainSystem,
) {
    let (Some(start), Some(end)) = (prepared_points.first(), prepared_points.last()) else {
        return;
    };
    validation.start_endpoint_height_m = start.y;
    validation.end_endpoint_height_m = end.y;
    validation.start_endpoint_terrain_height_m = terrain_height_m(terrain, start.x, start.z);
    validation.end_endpoint_terrain_height_m = terrain_height_m(terrain, end.x, end.z);
    validation.start_endpoint_support_delta_m =
        start.y - validation.start_endpoint_terrain_height_m;
    validation.end_endpoint_support_delta_m = end.y - validation.end_endpoint_terrain_height_m;
}

fn record_endpoint_snap_debug(
    validation: &mut RoadPreviewValidation,
    prepared_points: &[Vector3],
    existing_graph: &RegionGraph,
    endpoint_snap_enabled: bool,
) -> (Option<u32>, Option<u32>) {
    let (Some(start), Some(end)) = (prepared_points.first(), prepared_points.last()) else {
        return (None, None);
    };
    let start_existing = endpoint_existing_node(*start, existing_graph, endpoint_snap_enabled);
    let end_existing = endpoint_existing_node(*end, existing_graph, endpoint_snap_enabled);
    let debug_node_id = |node_id: u32| node_id.min(i32::MAX as u32) as i32;
    validation.start_endpoint_snapped_node_id = start_existing.map(debug_node_id).unwrap_or(-1);
    validation.end_endpoint_snapped_node_id = end_existing.map(debug_node_id).unwrap_or(-1);
    (start_existing, end_existing)
}

fn validate_against_graph(
    prepared_input: &PreparedRoadInput,
    fwd_lanes: u8,
    bkw_lanes: u8,
    terrain: &TerrainSystem,
    existing_graph: &RegionGraph,
    mut validation: RoadPreviewValidation,
) -> (RoadPreviewValidation, Option<RoadTopologyPlan>) {
    let prepared_points = &prepared_input.points;
    let (start_existing, end_existing) = record_endpoint_snap_debug(
        &mut validation,
        prepared_points,
        existing_graph,
        prepared_input.endpoint_snap_enabled,
    );
    if !validation.is_valid || prepared_points.len() < 2 {
        return (validation, None);
    }
    if start_existing.is_some() && start_existing == end_existing {
        return (
            validation.with_invalid_reason(PREVIEW_SAME_NODE_REASON),
            None,
        );
    }

    let grounded = prepared_input.class == EdgeClass::Standard && !prepared_input.tunnel;
    if let Some(reason) =
        roundabouts::stroke_ring_refusal(existing_graph, prepared_points, grounded)
    {
        return (validation.with_invalid_reason(reason), None);
    }

    // The profile check passed, so a tunnel stroke has its bore.
    let bore = prepared_input
        .tunnel
        .then(|| {
            let half_width_m = tunnel_section_half_width_m(
                prepared_points,
                fwd_lanes,
                bkw_lanes,
                prepared_input.median,
                prepared_input.parking,
                prepared_input.attributes,
            );
            let decks = grounded_decks_near(existing_graph, prepared_points, half_width_m);
            tunnel_bore(prepared_points, half_width_m, terrain, &decks).ok()
        })
        .flatten();
    if prepared_input.tunnel && bore.is_none() {
        return (
            validation.with_invalid_reason(PREVIEW_TUNNEL_COVER_REASON),
            None,
        );
    }
    // A junction that does not fit beside an empty joint takes the joint's place (`ROAD-118`
    // round JS2): the plan is built again with the joint dissolved, at most
    // `MAX_PLAN_BUILDS` builds, only for a pose the first build refuses.
    let mut yields: Vec<JointYield> = Vec::new();
    let mut refused_before = None;
    let mut builds = 0;
    let excerpt = loop {
        builds += 1;
        let excerpt = match solve_excerpt(
            prepared_input,
            (fwd_lanes, bkw_lanes),
            terrain,
            existing_graph,
            bore,
            &yields,
            &validation,
        ) {
            Ok(excerpt) => excerpt,
            // A dissolve that leads to another refusal leaves the junction's refusal standing.
            Err(refused) => return (refused_before.unwrap_or(refused), None),
        };
        let Some(refusal) = &excerpt.refusal else {
            break excerpt;
        };
        let refused = refuse_junction(validation.clone(), refusal, &excerpt.graph);
        let more = if prepared_input.tunnel || builds >= MAX_PLAN_BUILDS {
            Vec::new()
        } else {
            joint_yields(&excerpt.graph, existing_graph, &yields)
        };
        if more.is_empty() {
            return (refused, None);
        }
        yields.extend(more);
        refused_before = Some(refused);
    };
    let Excerpt {
        graph:
            ValidationGraph {
                graph: validation_graph,
                source_node_ids,
                source_edge_ids,
                finalized,
                splits,
                stroke_ends,
                dissolves,
                ..
            },
        stroke,
        rederived,
        refusal: _,
    } = excerpt;
    let mut validation = refuse_close_crossing(
        validation,
        prepared_input,
        (fwd_lanes, bkw_lanes),
        existing_graph,
    );
    if !validation.is_valid {
        return (validation, None);
    }
    record_edit_grade(
        &mut validation,
        &validation_graph,
        existing_graph,
        &stroke,
        &rederived,
        terrain,
    );

    let topology_plan = RoadTopologyPlan::capture(
        &validation_graph,
        existing_graph,
        &source_node_ids,
        &source_edge_ids,
        finalized,
        splits,
        stroke_ends,
        dissolves,
    );
    (validation, topology_plan)
}

// The most plans one pose builds (`ROAD-118` round JS2): the plan as drawn and up to two rounds
// of joints dissolving for the junctions it refuses.
const MAX_PLAN_BUILDS: usize = 3;

// A joint that makes way (`ROAD-118` round JS2), in the excerpt's local ids, which every build of
// one pose numbers alike up to the dissolves: the joint, the piece between it and the junction
// that does not fit, and, where that piece is the head of a split road, the split's second half,
// on which the road keeps its id.
#[derive(Clone, Copy, Debug, PartialEq)]
struct JointYield {
    joint: u32,
    piece: usize,
    rest: Option<usize>,
}

// One build of the post-edit excerpt with its checks up to the junction rules: the drawn road's
// pieces, the profiles it re-derives, and the first junction refusal.
struct Excerpt {
    graph: ValidationGraph,
    stroke: Vec<usize>,
    rederived: Vec<(usize, Option<usize>)>,
    refusal: Option<JunctionRefusal>,
}

// Builds the excerpt with the `yields` dissolved and runs the checks the plan runs before its
// junctions: an `Err` is the refused validation. O(stroke points + local excerpt records).
fn solve_excerpt(
    prepared_input: &PreparedRoadInput,
    (fwd_lanes, bkw_lanes): (u8, u8),
    terrain: &TerrainSystem,
    existing_graph: &RegionGraph,
    bore: Option<[usize; 2]>,
    yields: &[JointYield],
    validation: &RoadPreviewValidation,
) -> Result<Excerpt, RoadPreviewValidation> {
    let refuse = |reason| validation.clone().with_invalid_reason(reason);
    let excerpt = build_validation_graph(
        &prepared_input.points,
        prepared_input.class,
        prepared_input.grade(bore),
        (
            fwd_lanes,
            bkw_lanes,
            prepared_input.median,
            prepared_input.parking,
            prepared_input.attributes,
        ),
        existing_graph,
        prepared_input.endpoint_snap_enabled,
        terrain,
        bore,
        yields,
    )
    .map_err(refuse)?;
    let ValidationGraph {
        graph: validation_graph,
        new_edge_idx,
        required_edge_ids,
        source_node_ids,
        source_edge_ids,
        finalized,
        splits,
        stroke_ends,
        dissolves,
    } = &excerpt;
    // A dissolved joint's piece may be the stroke itself, which then runs on in the road it joins.
    let absorbed = dissolves.iter().any(|dissolve| dissolve.dropped == *new_edge_idx);
    if *new_edge_idx >= validation_graph.edge_count()
        || (validation_graph.edge(*new_edge_idx).deleted && !absorbed)
        || required_edge_ids.is_empty()
    {
        // The rule is the graph's; a road doubling a ring edge only gets its own words (D9).
        let reason = validation_graph
            .edges()
            .get(*new_edge_idx)
            .filter(|edge| {
                roundabouts::ring_neighbours(validation_graph, edge.start_node, edge.end_node)
            })
            .map_or(PREVIEW_SURFACE_GEOMETRY_REASON, |_| {
                ROUNDABOUT_NEIGHBOUR_NODES_REASON
            });
        return Err(refuse(reason));
    }
    if stroke_splits_pocket(
        validation_graph,
        existing_graph,
        source_node_ids,
        source_edge_ids,
    ) {
        return Err(refuse(STROKE_SPLITS_POCKET_REASON));
    }
    let (stroke, rederived) = edit_profiles(
        validation_graph,
        *new_edge_idx,
        splits,
        required_edge_ids,
        source_edge_ids,
    );
    if let Some(reason) = ring_join_refusal(validation_graph, &stroke) {
        return Err(refuse(reason));
    }
    if let Some(refused) = edit_geometry_violation(
        validation_graph,
        existing_graph,
        (&stroke, *stroke_ends),
        &rederived,
        source_node_ids,
    ) {
        return Err(refuse_geometry(validation.clone(), &refused));
    }
    // A junction in a tunnel approach's trench (`ROAD-61` round R8f), its depth below the ground
    // as the measurement; the lowest node id reports.
    let mut affected: Vec<u32> = finalized.affected_nodes.iter().copied().collect();
    affected.sort_unstable();
    if let Some(depth) = affected
        .into_iter()
        .find_map(|node| junction_in_trench(validation_graph, node, terrain))
    {
        let mut validation = refuse(JUNCTION_IN_TRENCH_REASON);
        validation.geometry_measured = depth;
        return Err(validation);
    }
    let refusal = junction_rejection(validation_graph, &finalized.affected_nodes);
    Ok(Excerpt {
        graph: excerpt,
        stroke,
        rederived,
        refusal,
    })
}

// A junction refusal with the place it names (`ROAD-118` round JS1).
fn refuse_junction(
    validation: RoadPreviewValidation,
    refusal: &JunctionRefusal,
    excerpt: &ValidationGraph,
) -> RoadPreviewValidation {
    let place = refusal.place(
        &excerpt.graph,
        &excerpt.source_node_ids,
        &excerpt.source_edge_ids,
        &excerpt.splits,
    );
    let mut validation = validation.with_invalid_reason(refusal.reason);
    if let Some((place, length_m, needed_m)) = place {
        validation.junction_place = Some(place);
        validation.geometry_measured = length_m;
        validation.geometry_limit = needed_m;
    }
    validation
}

// The joints that make way for the junctions this excerpt refuses (`ROAD-118` round JS2): for
// each affected node in id order whose layout fails on an arm (`CornerDoesNotFit`,
// `ArmTooShort`), each of its arms, in key order, that is too short for the node as its other arms
// stand (the layout's own answer, `roadbed::arm_edge_needed_m`, longer than the arm's road) and
// whose far node is an empty joint (`roadbed::empty_joint`, with no vehicle turn rule on the live
// node either), when the arm's piece is the edit's own: a piece of the stroke or of a split, never
// a whole existing road (that would retire a live edge id). Not those already in `done`.
// Refusals only: O(affected nodes x degree x the need's bisection).
fn joint_yields(
    excerpt: &ValidationGraph,
    existing_graph: &RegionGraph,
    done: &[JointYield],
) -> Vec<JointYield> {
    let local = &excerpt.graph;
    let mut nodes: Vec<u32> = excerpt
        .finalized
        .affected_nodes
        .iter()
        .copied()
        .filter(|&node| local.get_valid_node(node) == node)
        .collect();
    nodes.sort_unstable();
    let mut yields: Vec<JointYield> = Vec::new();
    for node in nodes {
        if !matches!(
            check_node(local, node),
            Err(NodeError::Layout(
                LayoutError::CornerDoesNotFit { .. } | LayoutError::ArmTooShort { .. }
            ))
        ) {
            continue;
        }
        let mut arms: Vec<ArmKey> = local
            .node_adjacency(node)
            .iter()
            .filter_map(|&edge| {
                let record = local.get_edge(edge).filter(|record| !record.deleted)?;
                let (start, end) = (
                    local.get_valid_node(record.start_node),
                    local.get_valid_node(record.end_node),
                );
                match (start == node, end == node) {
                    (true, false) => Some(ArmKey {
                        edge,
                        end: EdgeEnd::Start,
                    }),
                    (false, true) => Some(ArmKey {
                        edge,
                        end: EdgeEnd::End,
                    }),
                    _ => None,
                }
            })
            .collect();
        arms.sort_unstable();
        arms.dedup();
        for arm in arms {
            if let Some(found) = joint_yield(excerpt, existing_graph, node, arm)
                && !done.iter().chain(&yields).any(|other| other.joint == found.joint)
            {
                yields.push(found);
            }
        }
    }
    yields
}

// Arm `arm` of failing node `node` as a yield of [`joint_yields`], or `None`.
fn joint_yield(
    excerpt: &ValidationGraph,
    existing_graph: &RegionGraph,
    node: u32,
    arm: ArmKey,
) -> Option<JointYield> {
    let local = &excerpt.graph;
    let piece = local.edge(arm.edge);
    let joint = local.get_valid_node(match arm.end {
        EdgeEnd::Start => piece.end_node,
        EdgeEnd::End => piece.start_node,
    });
    if joint == node || roadbed::empty_joint(local, joint).is_none() {
        return None;
    }
    let live_rules = excerpt
        .source_node_ids
        .iter()
        .find(|&(_, &copy)| copy == joint)
        .is_some_and(|(&source, _)| roadbed::has_vehicle_turn_rules(existing_graph, source));
    if live_rules {
        return None;
    }
    // A copied road's head, cut off it by the edit's last split of it, comes away with the
    // road's id kept on the rest; an edge of the edit's own goes whole.
    let copied = excerpt
        .source_edge_ids
        .values()
        .any(|&copy| copy == arm.edge);
    let rest = if copied {
        let split = excerpt
            .splits
            .iter()
            .rev()
            .find(|split| split.edge_id == arm.edge)
            .filter(|_| local.get_valid_node(piece.start_node) == joint)?;
        Some(split.new_edge_id)
    } else {
        None
    };
    // The arm is one the node cannot lay out on, as its other arms stand.
    let needed_m = arm_edge_needed_m(local, node, arm)?;
    (needed_m > horizontal_length_m(&piece.geometry)).then_some(JointYield {
        joint,
        piece: arm.edge,
        rest,
    })
}

// D9 (`ROAD-62`): the drawn road `stroke` joins a ring at one point. Two of its pieces at a ring
// node run on through the ring (`roundabout_crossed`; a stroke through the island is
// [`roundabouts::stroke_ring_refusal`]'s), and a ring node takes one arm
// (`roundabout_arm_node_taken`: a second at one point never fits and is not a roundabout). The
// excerpt has every arm of a node the stroke touches. `O(stroke pieces x degree)`.
fn ring_join_refusal(local: &RegionGraph, stroke: &[usize]) -> Option<&'static str> {
    stroke.iter().find_map(|&piece| {
        let edge = local.edge(piece);
        [edge.start_node, edge.end_node]
            .into_iter()
            .find_map(|node| {
                let node = local.get_valid_node(node);
                roundabouts::ring_at(local, node)?;
                let (mut drawn, mut others) = (0, 0);
                for &id in local.node_adjacency(node) {
                    let arm = local.edge(id);
                    if arm.deleted || arm.roundabout.is_some() {
                        continue;
                    }
                    if stroke.contains(&id) {
                        drawn += 1;
                    } else {
                        others += 1;
                    }
                }
                if drawn > 1 {
                    Some(ROUNDABOUT_CROSSED_REASON)
                } else if others > 0 {
                    Some(ROUNDABOUT_ARM_NODE_TAKEN_REASON)
                } else {
                    None
                }
            })
    })
}

// D10 (`ROAD-95`): whether the stroke's excerpt split, merged or rerouted a turn pocket edge, or
// gave a pocket's transition node a third arm (a crossing, an end snapped onto it or a node
// captured on the stroke's line): a split would leave the pocket on a half that no longer reaches
// its junction, and a third arm ends the transition. The pocket is removed or the road drawn
// elsewhere first. The excerpt may hold only one of a transition's two arms (a pocket edge of a
// junction the stroke's excerpt reaches, without its base road), so its arms are counted against
// the existing arms the excerpt copied: a split of a copied arm keeps the count, a new arm raises
// it. Order-independent over the excerpt's maps, O(excerpt records).
fn stroke_splits_pocket(
    local: &RegionGraph,
    existing: &RegionGraph,
    source_nodes: &HashMap<u32, u32>,
    source_edges: &HashMap<usize, usize>,
) -> bool {
    let local_node = |source: u32| {
        source_nodes
            .get(&existing.get_valid_node(source))
            .map(|&node| local.get_valid_node(node))
    };
    let edge_changed = source_edges.iter().any(|(&source, &copy)| {
        let edge = existing.edge(source);
        let copied = local.edge(copy);
        edge.pocket.is_some()
            && (copied.deleted
                || Some(local.get_valid_node(copied.start_node)) != local_node(edge.start_node)
                || Some(local.get_valid_node(copied.end_node)) != local_node(edge.end_node))
    });
    edge_changed
        || source_nodes.iter().any(|(&source, &copy)| {
            existing.is_lane_transition(source)
                && (local.get_valid_node(copy) != copy
                    || local.live_node_connection_count(copy)
                        != existing
                            .node_adjacency(source)
                            .iter()
                            .filter(|edge| source_edges.contains_key(edge))
                            .count())
        })
}

// The local post-edit graph of a prepared stroke, or the refusal: a stroke whose ends meet one
// node or whose excerpt is incomplete is `surface_geometry_invalid`; a tunnel stroke's `bore`
// (`ROAD-61`) is split at its portals and refused when it meets another road. The stroke meets
// the roads it reaches by `grade` (`ROAD-99`).
#[allow(clippy::too_many_arguments)]
fn build_validation_graph(
    prepared_points: &[Vector3],
    edge_class: EdgeClass,
    grade: StrokeGrade,
    (fwd_lanes, bkw_lanes, median, parking, attributes): (
        u8,
        u8,
        Option<RoadMedian>,
        RoadParking,
        RoadAttributes,
    ),
    existing_graph: &RegionGraph,
    endpoint_snap_enabled: bool,
    terrain: &TerrainSystem,
    bore: Option<[usize; 2]>,
    yields: &[JointYield],
) -> Result<ValidationGraph, &'static str> {
    if prepared_points.len() < 2 {
        return Err(PREVIEW_SURFACE_GEOMETRY_REASON);
    }

    let mut validation_graph = RegionGraph::new();
    let mut node_map = HashMap::new();
    let mut copied_edges = HashMap::new();
    let start_existing = endpoint_existing_node(
        prepared_points[0],
        existing_graph,
        endpoint_snap_enabled,
    );
    let end_existing = endpoint_existing_node(
        *prepared_points.last().unwrap(),
        existing_graph,
        endpoint_snap_enabled,
    );

    let existing_edge_ids = candidate_existing_edge_ids(
        prepared_points,
        (fwd_lanes, bkw_lanes, median, parking, attributes),
        edge_class,
        existing_graph,
        start_existing,
        end_existing,
    );
    for edge_idx in existing_edge_ids {
        copy_validation_graph_edge(
            &mut validation_graph,
            existing_graph,
            &mut node_map,
            &mut copied_edges,
            edge_idx,
        );
    }

    // The spans the stroke's ends are snapped onto lie within the end-join tolerance of its
    // points, inside the excerpt's pad.
    let grade = grade.map_joined_spans(|span| copied_edges[&span]);
    // D9 (`ROAD-62`): an end on a ring near a bare node takes that node's place.
    let by_ring = validation_graph.edges().iter().any(|edge| edge.roundabout.is_some());
    let half_width_m = if by_ring {
        preview_candidate_roadbed_half_width_m(
            prepared_points,
            edge_class,
            fwd_lanes,
            bkw_lanes,
            median,
            parking,
            attributes,
        )
    } else {
        0.0
    };
    let mut ring_edges = Vec::new();
    let mut ring_splits = Vec::new();
    let mut join_end = |graph: &mut RegionGraph, existing: Option<u32>, point, taken| {
        let joins = by_ring && endpoint_snap_enabled && existing.is_none();
        let join = joins
            .then(|| roundabouts::join_near_bare_node(graph, point, half_width_m, taken))
            .flatten()?;
        ring_edges.extend(join.edges);
        ring_splits.extend(join.splits);
        Some(join.node)
    };
    let start_join = join_end(&mut validation_graph, start_existing, prepared_points[0], None);
    let last_point = *prepared_points.last().unwrap();
    let end_join = join_end(&mut validation_graph, end_existing, last_point, start_join);
    let start_node = start_join.unwrap_or_else(|| {
        validation_graph_endpoint_node(
            &mut validation_graph,
            existing_graph,
            &mut node_map,
            start_existing,
            prepared_points[0],
        )
    });
    let end_node = end_join.unwrap_or_else(|| {
        validation_graph_endpoint_node(
            &mut validation_graph,
            existing_graph,
            &mut node_map,
            end_existing,
            last_point,
        )
    });
    if start_node == end_node {
        return Err(PREVIEW_SURFACE_GEOMETRY_REASON);
    }
    copy_validation_crosswalk_overrides(
        &mut validation_graph,
        existing_graph,
        &node_map,
        &copied_edges,
    );

    let mut points = prepared_points.to_vec();
    points[0] = validation_graph.node(start_node).pos;
    let last_idx = points.len() - 1;
    points[last_idx] = validation_graph.node(end_node).pos;
    let portals = match bore {
        Some([first, last]) => {
            let ends = [start_node, end_node].map(Some);
            if bore_crosses_a_tunnel(&validation_graph, &points[first..=last], ends) {
                return Err(TUNNEL_CROSSING_REASON);
            }
            Some(portal_splits(&mut validation_graph, &points, [first, last]))
        }
        None => None,
    };
    let new_edge_idx = validation_graph.add_edge(build_surface_edge(
        start_node, end_node, points, fwd_lanes, bkw_lanes, edge_class, median, parking,
        attributes,
    ));
    let (base_cost, physical_length) =
        crate::simulation::pathing::cost::CostCalculator::calculate_costs(
            validation_graph.edge(new_edge_idx),
        );
    validation_graph.edge_mut(new_edge_idx).base_cost = base_cost;
    validation_graph.edge_mut(new_edge_idx).physical_length = physical_length;
    let (finalized, stroke_splits, dissolves) = process_validation_graph_intersections(
        &mut validation_graph,
        new_edge_idx,
        grade,
        terrain,
        portals.as_ref(),
        &ring_edges,
        yields,
    )?;
    // The bare nodes a join inserted split the ring before the stroke's own splits ran.
    let splits: Vec<PlannedRoadSplit> = ring_splits
        .into_iter()
        .map(|(edge_id, halves): (usize, topology::SplitHalves)| PlannedRoadSplit {
            edge_id,
            new_edge_id: halves.new_edge_id,
            far_node: halves.far_node,
            first_length_m: halves.first_length_m,
            second_length_m: halves.second_length_m,
        })
        .chain(stroke_splits)
        .collect();

    let mut required_edge_ids = validation_graph
        .edges()
        .iter()
        .enumerate()
        .skip(new_edge_idx)
        .filter_map(|(edge_idx, edge)| (!edge.deleted && is_surface_edge(edge)).then_some(edge_idx))
        .collect::<Vec<_>>();
    // Junction finalization can change another approach beyond the inserted stroke.
    required_edge_ids.extend(finalized.dirty_edges.iter().copied().filter(|&id| {
        !validation_graph.edge(id).deleted && is_surface_edge(validation_graph.edge(id))
    }));
    required_edge_ids.sort_unstable();
    required_edge_ids.dedup();

    // Profile-solving nodes must already have complete incidence, and so must the far ends of
    // the edges they re-derive, whose pins the solve reproduces from all their arms. A later
    // read-only halo must not disguise an incomplete solve as a valid plan.
    let mut pinned_nodes = HashSet::new();
    for &node in &finalized.affected_nodes {
        let node = validation_graph.get_valid_node(node);
        pinned_nodes.insert(node);
        if node as usize >= validation_graph.node_adjacency_count() {
            continue;
        }
        for &edge_idx in validation_graph.node_adjacency(node) {
            let edge = validation_graph.edge(edge_idx);
            if !edge.deleted && is_surface_edge(edge) {
                pinned_nodes.insert(validation_graph.get_valid_node(edge.start_node));
                pinned_nodes.insert(validation_graph.get_valid_node(edge.end_node));
            }
        }
    }
    if node_map.iter().any(|(&source, &local)| {
        pinned_nodes.contains(&validation_graph.get_valid_node(local))
            && existing_graph
                .node_adjacency(source)
                .iter()
                .any(|id| !existing_graph.edge(*id).deleted && !copied_edges.contains_key(id))
    }) {
        return Err(PREVIEW_SURFACE_GEOMETRY_REASON);
    }

    Ok(ValidationGraph {
        graph: validation_graph,
        new_edge_idx,
        required_edge_ids,
        source_node_ids: node_map,
        source_edge_ids: copied_edges,
        finalized,
        splits,
        stroke_ends: [start_node, end_node],
        dissolves,
    })
}

#[allow(clippy::too_many_arguments)]
fn candidate_existing_edge_ids(
    prepared_points: &[Vector3],
    (fwd_lanes, bkw_lanes, median, parking, attributes): (
        u8,
        u8,
        Option<RoadMedian>,
        RoadParking,
        RoadAttributes,
    ),
    edge_class: EdgeClass,
    existing_graph: &RegionGraph,
    start_existing: Option<u32>,
    end_existing: Option<u32>,
) -> Vec<usize> {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_z = f32::MAX;
    let mut max_z = f32::MIN;
    for point in prepared_points {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_z = min_z.min(point.z);
        max_z = max_z.max(point.z);
    }

    let pad = config::SNAP_TOLERANCE
        + config::INTERSECTION_TOLERANCE
        + preview_candidate_roadbed_half_width_m(
            prepared_points,
            edge_class,
            fwd_lanes,
            bkw_lanes,
            median,
            parking,
            attributes,
        );
    let mut edge_ids = existing_graph.get_edges_near_aabb(
        Vector3::new(min_x - pad, 0.0, min_z - pad),
        Vector3::new(max_x + pad, 0.0, max_z + pad),
    );
    for existing_node in [start_existing, end_existing].into_iter().flatten() {
        push_incident_validation_edges(existing_graph, existing_node, &mut edge_ids);
    }

    // A join may move any bare node of a ring (D9, `ROAD-62`): the excerpt holds the whole ring.
    let rings: Vec<usize> = edge_ids
        .iter()
        .copied()
        .filter(|&id| id < existing_graph.edge_count())
        .filter(|&id| !existing_graph.edge(id).deleted)
        .filter(|&id| existing_graph.edge(id).roundabout.is_some())
        .collect();
    for id in rings {
        let walk = roundabouts::ring_walk(existing_graph, id);
        edge_ids.extend(walk.iter().flatten().map(|&(_, edge)| edge));
    }
    edge_ids.sort_unstable();
    edge_ids.dedup();

    // Two rings of incidence. The nodes of the roads the stroke reaches may solve their planes,
    // which takes all their arms; the solve then re-derives every edge at those nodes, pinned at
    // both ends, and a far end's plane is again a function of all its arms
    // (`RegionGraph::finalize_junction_endpoint_profiles`), so the far nodes need theirs too.
    let mut incident_nodes = Vec::new();
    for _ in 0..2 {
        incident_nodes.clear();
        for &edge_idx in &edge_ids {
            if edge_idx >= existing_graph.edge_count() {
                continue;
            }
            let edge = existing_graph.edge(edge_idx);
            if edge.deleted || !is_surface_edge(edge) {
                continue;
            }
            incident_nodes.push(existing_graph.get_valid_node(edge.start_node));
            incident_nodes.push(existing_graph.get_valid_node(edge.end_node));
        }
        incident_nodes.sort_unstable();
        incident_nodes.dedup();
        for &node_id in &incident_nodes {
            push_incident_validation_edges(existing_graph, node_id, &mut edge_ids);
        }
        edge_ids.sort_unstable();
        edge_ids.dedup();
    }
    edge_ids
}

fn push_incident_validation_edges(
    existing_graph: &RegionGraph,
    node_id: u32,
    edge_ids: &mut Vec<usize>,
) {
    let valid = existing_graph.get_valid_node(node_id);
    if valid as usize >= existing_graph.node_adjacency_count() {
        return;
    }
    edge_ids.extend(existing_graph.node_adjacency(valid).iter().copied());
}

// A tunnel stroke's `portals` (its portal nodes and their splits of the new edge) split it with
// its crossings, and its edges are classed before the profiles are solved. The `yields` dissolve
// after the topology and before the solve (`ROAD-118` round JS2), so the solve lays out the
// joined roads.
#[allow(clippy::type_complexity)]
fn process_validation_graph_intersections(
    validation_graph: &mut RegionGraph,
    new_edge_idx: usize,
    grade: StrokeGrade,
    terrain: &TerrainSystem,
    portals: Option<&Portals>,
    moved_ring_edges: &[usize],
    yields: &[JointYield],
) -> Result<(FinalizedRoadGeometry, Vec<PlannedRoadSplit>, Vec<JointDissolve>), &'static str> {
    let mut network = TransitNetwork::new();
    let world_config = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&world_config);
    let mut allocator = BuildingAllocator::new();
    let start_node = validation_graph.edge(new_edge_idx).start_node;
    let end_node = validation_graph.edge(new_edge_idx).end_node;
    let node_count_before = validation_graph.node_count();
    let edge_count_before = validation_graph.edge_count();
    // Production commits batch intersection edits and profile every edge touched by splitting.
    // Populate the same bulk ledger here so planned profiles are commit-identical.
    network.bulk_load = true;
    network.recorded_road_splits = Some(Vec::new());
    network.bulk_dirty_edges.insert(new_edge_idx);
    network.bulk_dirty_edges.extend(moved_ring_edges);
    let joined_road_ends = topology::process_intersections_splitting_at(
        &mut network,
        validation_graph,
        new_edge_idx,
        grade,
        portals.map_or(&[][..], |portals| &portals.splits),
        &mut zoning,
        &mut allocator,
    );
    cleanup_validation_duplicate_edges(validation_graph);
    if let Some(portals) = portals {
        classify_tunnel_stroke(
            validation_graph,
            new_edge_idx,
            network.recorded_road_splits.as_deref().unwrap_or_default(),
            [start_node, end_node],
            portals.nodes,
        )?;
    }

    let mut surface_dirty_nodes = road_insert_surface_dirty_nodes(
        validation_graph,
        start_node,
        end_node,
        new_edge_idx,
        node_count_before,
        edge_count_before,
    );
    // A joined road end's neighbour lays out again (`ROAD-118`); the excerpt's second ring of
    // incidence holds its arms and their far ends.
    surface_dirty_nodes.extend(joined_road_ends);
    let mut splits = network.recorded_road_splits.take().unwrap();
    let dissolves = dissolve_joints(validation_graph, yields, &mut splits);
    for dissolve in &dissolves {
        // Both roads' far ends lay out again on their new lengths.
        network
            .bulk_dirty_edges
            .extend([dissolve.into, dissolve.from, dissolve.dropped]);
        surface_dirty_nodes.insert(dissolve.junction);
    }
    // A merged node lays out as its survivor.
    let surface_dirty_nodes: HashSet<u32> = surface_dirty_nodes
        .into_iter()
        .map(|node| validation_graph.get_valid_node(node))
        .collect();
    network.mark_surface_dirty_for_nodes(validation_graph, &surface_dirty_nodes);

    let finalized = network.finalize_road_geometry(validation_graph, Some(terrain));
    Ok((finalized, splits, dissolves))
}

// Dissolves the `yields` that are still empty joints with their pieces in place, in order
// (`ROAD-118` round JS2). A head cut off a copied road drops the split that cut it: the road keeps
// its id on the rest, and no dependents or turn rules move to the second half's slot.
// O(yields x (points + degree + splits)).
fn dissolve_joints(
    graph: &mut RegionGraph,
    yields: &[JointYield],
    splits: &mut Vec<PlannedRoadSplit>,
) -> Vec<JointDissolve> {
    let mut dissolves = Vec::with_capacity(yields.len());
    for &JointYield { joint, piece, rest } in yields {
        if graph.get_valid_node(joint) != joint || roadbed::empty_joint(graph, joint).is_none() {
            continue;
        }
        let Some(dissolve) = topology::dissolve_joint_graph(graph, joint, piece, rest) else {
            continue;
        };
        if let Some(rest) = rest {
            splits.retain(|split| !(split.edge_id == piece && split.new_edge_id == rest));
        }
        dissolves.push(dissolve);
    }
    dissolves
}

fn cleanup_validation_duplicate_edges(graph: &mut RegionGraph) {
    let mut seen = HashSet::new();
    let mut to_remove = Vec::new();

    for (edge_idx, edge) in graph.edges().iter().enumerate() {
        if edge.deleted {
            continue;
        }
        let pair = if edge.start_node < edge.end_node {
            (edge.start_node, edge.end_node)
        } else {
            (edge.end_node, edge.start_node)
        };

        if seen.contains(&pair) || edge.start_node == edge.end_node {
            to_remove.push(edge_idx);
        } else {
            seen.insert(pair);
        }
    }

    for edge_idx in to_remove {
        graph.edge_mut(edge_idx).deleted = true;
    }
    graph.rebuild_adjacency_list();
}

fn validation_graph_endpoint_node(
    validation_graph: &mut RegionGraph,
    existing_graph: &RegionGraph,
    node_map: &mut HashMap<u32, u32>,
    existing_node: Option<u32>,
    fallback_pos: Vector3,
) -> u32 {
    match existing_node {
        Some(node_id) => copy_validation_graph_node(
            validation_graph,
            existing_graph,
            node_map,
            node_id,
        ),
        None => validation_graph.add_node(fallback_pos, NodeType::Junction),
    }
}

// The copied nodes' crossing overrides on their copied arms (`ROAD-74`), in local ids, so the
// excerpt's junction solve lays the nodes out as the live graph does and the splits and merges
// it applies carry them. O(copied nodes x degree).
fn copy_validation_crosswalk_overrides(
    validation_graph: &mut RegionGraph,
    existing_graph: &RegionGraph,
    node_map: &HashMap<u32, u32>,
    copied_edges: &HashMap<usize, usize>,
) {
    for (&source, &local) in node_map {
        for (edge, &marked) in &existing_graph.node(source).crosswalk_overrides {
            if let Some(&local_edge) = copied_edges.get(edge) {
                validation_graph.set_crosswalk_override(local, local_edge, marked);
            }
        }
    }
}

fn copy_validation_graph_node(
    validation_graph: &mut RegionGraph,
    existing_graph: &RegionGraph,
    node_map: &mut HashMap<u32, u32>,
    node_id: u32,
) -> u32 {
    let valid = existing_graph.get_valid_node(node_id);
    if let Some(&mapped) = node_map.get(&valid) {
        return mapped;
    }

    let node = existing_graph.node(valid);
    let mapped = validation_graph.add_node(node.pos, node.node_type);
    node_map.insert(valid, mapped);
    mapped
}

fn copy_validation_graph_edge(
    validation_graph: &mut RegionGraph,
    existing_graph: &RegionGraph,
    node_map: &mut HashMap<u32, u32>,
    copied_edges: &mut HashMap<usize, usize>,
    edge_idx: usize,
) {
    if copied_edges.contains_key(&edge_idx) || edge_idx >= existing_graph.edge_count() {
        return;
    }
    let edge = existing_graph.edge(edge_idx);
    if edge.deleted || !is_surface_edge(edge) {
        return;
    }

    let start_node = copy_validation_graph_node(
        validation_graph,
        existing_graph,
        node_map,
        edge.start_node,
    );
    let end_node = copy_validation_graph_node(
        validation_graph,
        existing_graph,
        node_map,
        edge.end_node,
    );
    if start_node == end_node {
        return;
    }

    let mut copied_edge = edge.clone();
    copied_edge.start_node = start_node;
    copied_edge.end_node = end_node;
    let local_edge = validation_graph.add_edge(copied_edge);
    copied_edges.insert(edge_idx, local_edge);
}

fn endpoint_existing_node(
    point: Vector3,
    existing_graph: &RegionGraph,
    endpoint_snap_enabled: bool,
) -> Option<u32> {
    if !endpoint_snap_enabled {
        return None;
    }
    existing_graph.find_node_within(point, config::SNAP_TOLERANCE)
}

// The profiles the edit derives: the drawn road (the inserted edge and every piece split off it),
// and the other surface edges it re-derives, each with the existing edge it came from (pieces
// split off an existing road come from that road). The re-derived edges in ascending local ids.
// O((required edges + splits) * stroke pieces).
fn edit_profiles(
    validation_graph: &RegionGraph,
    new_edge_idx: usize,
    splits: &[PlannedRoadSplit],
    required_edge_ids: &[usize],
    source_edge_ids: &HashMap<usize, usize>,
) -> (Vec<usize>, Vec<(usize, Option<usize>)>) {
    let mut stroke = vec![new_edge_idx];
    let mut source_of: HashMap<usize, usize> = source_edge_ids
        .iter()
        .map(|(&source, &local)| (local, source))
        .collect();
    for split in splits {
        if stroke.contains(&split.edge_id) {
            stroke.push(split.new_edge_id);
        } else if let Some(&source) = source_of.get(&split.edge_id) {
            source_of.insert(split.new_edge_id, source);
        }
    }
    stroke.retain(|&id| !validation_graph.edge(id).deleted);
    let rederived = required_edge_ids
        .iter()
        .filter(|id| !stroke.contains(id))
        .map(|&id| (id, source_of.get(&id).copied()))
        .collect();
    (stroke, rederived)
}

// The illegal geometry this edit adds (`validate::edit_violation`) on the drawn road `stroke`
// and the profiles it re-derives. Its endpoints, and the stroke's ends where a dissolved joint's
// road took the piece that reached them (`ROAD-118`), are judged against what the same nodes were
// before. O(stroke points + re-derived points + endpoint incidence).
fn edit_geometry_violation(
    validation_graph: &RegionGraph,
    existing_graph: &RegionGraph,
    (stroke, stroke_ends): (&[usize], [u32; 2]),
    rederived: &[(usize, Option<usize>)],
    source_node_ids: &HashMap<u32, u32>,
) -> Option<RoadGeometryViolation> {
    let source_of: HashMap<u32, u32> = source_node_ids
        .iter()
        .map(|(&source, &local)| (local, source))
        .collect();
    let mut nodes: Vec<(u32, Option<u32>)> = stroke
        .iter()
        .flat_map(|&id| {
            let edge = validation_graph.edge(id);
            [edge.start_node, edge.end_node]
        })
        .chain(stroke_ends)
        .map(|node| {
            let node = validation_graph.get_valid_node(node);
            (node, source_of.get(&node).copied())
        })
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    crate::simulation::network::validate::edit_violation(
        validation_graph,
        stroke,
        rederived,
        &nodes,
        existing_graph,
        &RoadGeometryLimits::default(),
    )
}

// Records the steepest profile the edit builds as the validation's grade: the drawn road's
// pieces and each re-derived profile steeper than it was, by the one grade
// (`grade::steepest_grade`). Above its class's design tier it is the road tool's warning.
// O(stroke points + re-derived points and their sources).
fn record_edit_grade(
    validation: &mut RoadPreviewValidation,
    validation_graph: &RegionGraph,
    existing_graph: &RegionGraph,
    stroke: &[usize],
    rederived: &[(usize, Option<usize>)],
    terrain: &TerrainSystem,
) {
    let limits = RoadGeometryLimits::default();
    let before = |source: Option<usize>| {
        source
            .and_then(|source| steepest_grade(centreline(existing_graph.edge(source))))
            .map_or(-1.0, |steepest| steepest.grade)
    };
    let mut steepest: Option<(usize, SteepestGrade)> = None;
    let edges = stroke
        .iter()
        .map(|&id| (id, -1.0))
        .chain(rederived.iter().map(|&(id, source)| (id, before(source))));
    for (id, old_grade) in edges {
        let Some(grade) = steepest_grade(centreline(validation_graph.edge(id))) else {
            continue;
        };
        if grade.grade > old_grade && steepest.is_none_or(|(_, worst)| grade.grade > worst.grade) {
            steepest = Some((id, grade));
        }
    }
    let Some((id, grade)) = steepest else {
        return;
    };
    record_steepest_span(validation, centreline(validation_graph.edge(id)), grade, terrain);
    if let Some(RoadGeometryViolation::Grade {
        limit,
        tier: RoadGeometryTier::Design,
        ..
    }) = grade_violation(validation_graph, id, &limits)
    {
        validation.warning_reason = PREVIEW_STEEP_WARNING;
        validation.warning_limit = limit;
    }
}

// A node whose junction layout or mouth heights fail, with its error and refusal key.
struct JunctionRefusal {
    node: u32,
    error: NodeError,
    reason: &'static str,
}

impl JunctionRefusal {
    // The place it names (`ROAD-118` round JS1) with the failing arm's road length and the
    // shortest that fits, rounded up to 0.1 m (0 when none does): `None` for an error that names
    // no arm. Refusals only: O(local records + the need's bisection, `arm_edge_needed_m`).
    fn place(
        &self,
        local: &RegionGraph,
        source_nodes: &HashMap<u32, u32>,
        source_edges: &HashMap<usize, usize>,
        splits: &[PlannedRoadSplit],
    ) -> Option<(JunctionPlace, f32, f32)> {
        let arm = match self.error {
            NodeError::Layout(
                LayoutError::CornerDoesNotFit { arm } | LayoutError::ArmTooShort { arm },
            ) => arm,
            _ => return None,
        };
        let edge = local.get_edge(arm.edge)?;
        let far = local.get_valid_node(match arm.end {
            EdgeEnd::Start => edge.end_node,
            EdgeEnd::End => edge.start_node,
        });
        let roles = [node_role(local, self.node)?, node_role(local, far)?];
        let source_node = source_nodes
            .iter()
            .find(|&(_, &local_id)| local_id == self.node)
            .map(|(&source, _)| source);
        // A split piece lies on the road its chain of splits started from.
        let mut root = arm.edge;
        let source_edge = loop {
            if let Some((&source, _)) = source_edges.iter().find(|&(_, &local_id)| local_id == root)
            {
                break Some(source);
            }
            match splits.iter().find(|split| split.new_edge_id == root) {
                Some(split) => root = split.edge_id,
                None => break None,
            }
        };
        let length_m = horizontal_length_m(&edge.geometry);
        let needed_m = arm_edge_needed_m(local, self.node, arm)
            // Up to 0.1 m, past the bisection's sub-millimetre overshoot.
            .map_or(0.0, |needed_m| (needed_m * 10.0 - 0.01).ceil() / 10.0);
        let place = JunctionPlace {
            node: source_node,
            position: local.node(self.node).pos,
            arm_edge: source_edge,
            roles,
        };
        Some((place, length_m as f32, needed_m as f32))
    }
}

// Horizontal length of a control polyline, metres.
fn horizontal_length_m(points: &[Vector3]) -> f64 {
    points
        .windows(2)
        .map(|pair| f64::from((pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z)))
        .sum()
}

// The first node, in id order, whose junction layout or mouth heights fail (`ROAD-68-C`). The
// excerpt holds every affected node's full incidence. O(affected degree * arm points).
fn junction_rejection(graph: &RegionGraph, affected_nodes: &HashSet<u32>) -> Option<JunctionRefusal> {
    let mut nodes: Vec<u32> = affected_nodes.iter().copied().collect();
    nodes.sort_unstable();
    let (node, error) = nodes
        .into_iter()
        .find_map(|node| check_node(graph, node).err().map(|error| (node, error)))?;
    let reason = match error {
        // Bare nodes make way for a join (D9), so a ring's corner fails on arm against arm.
        NodeError::Layout(LayoutError::CornerDoesNotFit { .. })
            if roundabouts::ring_at(graph, node).is_some() =>
        {
            ROUNDABOUT_ARMS_TOO_CLOSE_REASON
        }
        NodeError::Layout(LayoutError::CornerDoesNotFit { .. }) => PREVIEW_JUNCTION_CORNER_REASON,
        NodeError::Layout(LayoutError::CoreNotSimple) | NodeError::Triangulation => {
            PREVIEW_JUNCTION_CORE_REASON
        }
        NodeError::Layout(LayoutError::BandMismatch { .. }) => PREVIEW_JUNCTION_BANDS_REASON,
        NodeError::Layout(LayoutError::ArmTooShort { .. }) => PREVIEW_JUNCTION_ARM_REASON,
        NodeError::Layout(_) | NodeError::MouthVertex { .. } => PREVIEW_JUNCTION_LAYOUT_REASON,
        NodeError::NoPlane | NodeError::MouthOffPlane { .. } => PREVIEW_JUNCTION_PROFILE_REASON,
    };
    Some(JunctionRefusal {
        node,
        error,
        reason,
    })
}

fn refuse_geometry(
    validation: RoadPreviewValidation,
    violation: &RoadGeometryViolation,
) -> RoadPreviewValidation {
    use RoadGeometryViolation::*;
    let (reason, measured, limit) = match *violation {
        Grade {
            measured, limit, ..
        } => (PREVIEW_TOO_STEEP_REASON, measured, limit),
        GradeChange {
            measured, limit, ..
        } => (PREVIEW_GRADE_CHANGE_REASON, measured, limit),
        HorizontalRadius {
            measured, limit, ..
        } => (PREVIEW_TIGHT_CURVE_REASON, measured, limit),
        JunctionArmAngle {
            measured, limit, ..
        } => (PREVIEW_JUNCTION_ANGLE_REASON, measured, limit),
        JunctionDegree {
            measured, limit, ..
        } => (
            PREVIEW_JUNCTION_DEGREE_REASON,
            measured as f32,
            limit as f32,
        ),
        DirectionKink { .. } => (PREVIEW_SURFACE_GEOMETRY_REASON, 0.0, 0.0),
    };
    let mut validation = validation.with_invalid_reason(reason);
    validation.geometry_measured = measured;
    validation.geometry_limit = limit;
    validation
}
