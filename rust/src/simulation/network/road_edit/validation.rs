// SPDX-License-Identifier: GPL-2.0-only

//! Road placement validation: stroke length, profile grade and clearance, endpoint snapping,
//! same-node connections and the `ROAD-54` edit geometry limits on the bounded post-edit graph
//! excerpt that also yields the shared [`RoadTopologyPlan`].

use super::input::{
    PREVIEW_CLEARANCE_M, PreparedRoadInput, ROAD_PROFILE_MAX_GRADE, is_surface_edge,
};
use super::{FinalizedRoadGeometry, PlannedRoadSplit, RoadExtensionReprofile, RoadTopologyPlan};
use crate::config;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::preview_candidate_roadbed_half_width_m;
use crate::simulation::network::types::{EdgeClass, NodeType};
use crate::simulation::network::validate::{RoadGeometryLimits, RoadGeometryViolation};
use crate::simulation::network::{
    TransitNetwork, build_surface_edge, road_insert_surface_dirty_nodes, topology,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector3;
use std::collections::{HashMap, HashSet};

const PREVIEW_VALID_REASON: &str = "";
const PREVIEW_BRIDGE_CLEARANCE_REASON: &str = "bridge_clearance";
const PREVIEW_TUNNEL_CLEARANCE_REASON: &str = "tunnel_clearance";
const PREVIEW_SURFACE_GEOMETRY_REASON: &str = "surface_geometry_invalid";
const PREVIEW_SAME_NODE_REASON: &str = "same_node_connection";
const PREVIEW_TOO_SHORT_REASON: &str = "too_short";
const PREVIEW_GRADE_CHANGE_REASON: &str = "road_grade_change";
const PREVIEW_TIGHT_CURVE_REASON: &str = "road_tight_curve";
const PREVIEW_JUNCTION_ANGLE_REASON: &str = "junction_arm_angle";
const PREVIEW_JUNCTION_DEGREE_REASON: &str = "junction_degree";
const PREVIEW_MIN_ENDPOINT_SEGMENT_M: f32 = 2.0;
const PREVIEW_BRIDGE_GROUND_TOLERANCE_M: f32 = 0.05;
// Degenerate profile steps shorter than this carry no grade.
const SAMPLE_EPSILON_M: f32 = 0.001;

struct ValidationGraph {
    graph: RegionGraph,
    new_edge_idx: usize,
    required_edge_ids: Vec<usize>,
    source_node_ids: HashMap<u32, u32>,
    source_edge_ids: HashMap<usize, usize>,
    finalized: FinalizedRoadGeometry,
    splits: Vec<PlannedRoadSplit>,
}

/// Machine-readable road-placement validation state shared by the UI and commit guard.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadPreviewValidation {
    /// True when the stroke can be placed as a road under the current road-profile rules.
    pub(crate) is_valid: bool,
    /// Stable reason key for invalid strokes, or an empty string when valid.
    pub(crate) invalid_reason: &'static str,
    /// Largest centerline grade seen in the prepared profile.
    pub(crate) max_grade: f32,
    /// Standard-road profile-solver grade target retained for diagnostics.
    pub(crate) allowed_grade: f32,
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
    /// Measured value of the geometry check that refused the road (`ROAD-54`): grade change per
    /// metre, curve radius in metres, junction arm angle in degrees or junction arm count.
    pub(crate) geometry_measured: f32,
    /// The limit that measurement broke, in the same unit.
    pub(crate) geometry_limit: f32,
}

impl RoadPreviewValidation {
    /// Creates a successful validation result with the measured maximum grade.
    pub(crate) fn valid(max_grade: f32) -> Self {
        Self {
            is_valid: true,
            invalid_reason: PREVIEW_VALID_REASON,
            max_grade,
            allowed_grade: ROAD_PROFILE_MAX_GRADE,
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
/// checks, for synchronous editor feedback. O(prepared points + endpoint snap lookups).
pub(crate) fn validate_road_candidate_fast(
    prepared_input: &PreparedRoadInput,
    terrain: &TerrainSystem,
    existing_graph: &RegionGraph,
) -> RoadPreviewValidation {
    let mut validation = profile_validation(
        prepared_input.class,
        &prepared_input.validation_points,
        terrain,
        true,
    );
    record_endpoint_height_debug(&mut validation, &prepared_input.points, terrain);
    let (start_existing, end_existing) = record_endpoint_snap_debug(
        &mut validation,
        &prepared_input.points,
        existing_graph,
        prepared_input.extension.as_ref(),
        prepared_input.endpoint_snap_enabled,
    );
    if !validation.is_valid || prepared_input.points.len() < 2 {
        return validation;
    }
    if start_existing.is_some() && start_existing == end_existing {
        return validation.with_invalid_reason(PREVIEW_SAME_NODE_REASON);
    }
    validation
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
    let new_edge_validation =
        profile_validation(prepared_input.class, &prepared_input.points, terrain, false);
    if stroke_too_short(&prepared_input.validation_points) {
        return (
            new_edge_validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON),
            None,
        );
    }
    if prepared_input.validation_points != prepared_input.points {
        let corridor_validation = profile_validation(
            prepared_input.class,
            &prepared_input.validation_points,
            terrain,
            false,
        );
        if !corridor_validation.is_valid {
            return (corridor_validation, None);
        }
    }
    validate_against_graph(
        prepared_input,
        fwd_lanes,
        bkw_lanes,
        existing_graph,
        new_edge_validation,
    )
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

// Steepest prepared span and bridge / tunnel clearance of one profile. With `require_length`
// (the fast editor check) a stroke below the minimum length is also refused; the full check
// applies that rule once, to the validation corridor.
fn profile_validation(
    edge_class: EdgeClass,
    prepared_points: &[Vector3],
    terrain: &TerrainSystem,
    require_length: bool,
) -> RoadPreviewValidation {
    if prepared_points.len() < 2 && !require_length {
        return RoadPreviewValidation::valid(0.0);
    }
    let mut validation = RoadPreviewValidation::valid(0.0);
    record_endpoint_height_debug(&mut validation, prepared_points, terrain);
    if prepared_points.len() < 2 {
        return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
    }

    let mut station_m = 0.0;
    for pair in prepared_points.windows(2) {
        let run = (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z);
        if run <= SAMPLE_EPSILON_M {
            continue;
        }
        let next_station_m = station_m + run;
        let grade = (pair[1].y - pair[0].y).abs() / run;
        if grade > validation.max_grade {
            validation.max_grade = grade;
            validation.offending_span_start_m = station_m;
            validation.offending_span_end_m = next_station_m;
            validation.offending_span_run_m = run;
            validation.offending_span_height_delta_m = pair[1].y - pair[0].y;
            validation.offending_span_start_height_m = pair[0].y;
            validation.offending_span_end_height_m = pair[1].y;
            validation.offending_span_start_terrain_height_m =
                terrain_height_m(terrain, pair[0].x, pair[0].z);
            validation.offending_span_end_terrain_height_m =
                terrain_height_m(terrain, pair[1].x, pair[1].z);
            validation.offending_span_start_support_delta_m =
                pair[0].y - validation.offending_span_start_terrain_height_m;
            validation.offending_span_end_support_delta_m =
                pair[1].y - validation.offending_span_end_terrain_height_m;
        }
        station_m = next_station_m;
    }

    if require_length && stroke_too_short(prepared_points) {
        return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
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
            EdgeClass::Tunnel => {
                validation.clearance_m = terrain_h - mid.y;
                validation.required_clearance_m = PREVIEW_CLEARANCE_M;
                if validation.clearance_m < PREVIEW_CLEARANCE_M {
                    return validation.with_invalid_reason(PREVIEW_TUNNEL_CLEARANCE_REASON);
                }
            }
            EdgeClass::Standard => {}
        }
    }

    validation
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
    extension: Option<&RoadExtensionReprofile>,
    endpoint_snap_enabled: bool,
) -> (Option<u32>, Option<u32>) {
    let (Some(start), Some(end)) = (prepared_points.first(), prepared_points.last()) else {
        return (None, None);
    };
    let start_existing =
        endpoint_existing_node(*start, existing_graph, extension, endpoint_snap_enabled);
    let end_existing =
        endpoint_existing_node(*end, existing_graph, extension, endpoint_snap_enabled);
    let debug_node_id = |node_id: u32| node_id.min(i32::MAX as u32) as i32;
    validation.start_endpoint_snapped_node_id = start_existing.map(debug_node_id).unwrap_or(-1);
    validation.end_endpoint_snapped_node_id = end_existing.map(debug_node_id).unwrap_or(-1);
    (start_existing, end_existing)
}

fn validate_against_graph(
    prepared_input: &PreparedRoadInput,
    fwd_lanes: u8,
    bkw_lanes: u8,
    existing_graph: &RegionGraph,
    mut validation: RoadPreviewValidation,
) -> (RoadPreviewValidation, Option<RoadTopologyPlan>) {
    let prepared_points = &prepared_input.points;
    let extension = prepared_input.extension.as_ref();
    let (start_existing, end_existing) = record_endpoint_snap_debug(
        &mut validation,
        prepared_points,
        existing_graph,
        extension,
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

    let Some(ValidationGraph {
        graph: validation_graph,
        new_edge_idx,
        required_edge_ids,
        source_node_ids,
        source_edge_ids,
        finalized,
        splits,
    }) = build_validation_graph(
        prepared_points,
        prepared_input.class,
        fwd_lanes,
        bkw_lanes,
        existing_graph,
        extension,
        prepared_input.endpoint_snap_enabled,
    )
    else {
        return (
            validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON),
            None,
        );
    };
    if new_edge_idx >= validation_graph.edge_count()
        || validation_graph.edge(new_edge_idx).deleted
        || required_edge_ids.is_empty()
    {
        return (
            validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON),
            None,
        );
    }
    if let Some(refused) = edit_geometry_violation(
        &validation_graph,
        existing_graph,
        new_edge_idx,
        &splits,
        &source_node_ids,
    ) {
        return (refuse_geometry(validation, &refused), None);
    }

    let topology_plan = RoadTopologyPlan::capture(
        &validation_graph,
        existing_graph,
        &source_node_ids,
        &source_edge_ids,
        finalized,
        splits,
    );
    (validation, topology_plan)
}

fn build_validation_graph(
    prepared_points: &[Vector3],
    edge_class: EdgeClass,
    fwd_lanes: u8,
    bkw_lanes: u8,
    existing_graph: &RegionGraph,
    extension: Option<&RoadExtensionReprofile>,
    endpoint_snap_enabled: bool,
) -> Option<ValidationGraph> {
    if prepared_points.len() < 2 {
        return None;
    }

    let mut validation_graph = RegionGraph::new();
    let mut node_map = HashMap::new();
    let mut copied_edges = HashMap::new();
    let start_existing = endpoint_existing_node(
        prepared_points[0],
        existing_graph,
        extension,
        endpoint_snap_enabled,
    );
    let end_existing = endpoint_existing_node(
        *prepared_points.last().unwrap(),
        existing_graph,
        extension,
        endpoint_snap_enabled,
    );

    let existing_edge_ids = candidate_existing_edge_ids(
        prepared_points,
        fwd_lanes,
        bkw_lanes,
        edge_class,
        existing_graph,
        extension,
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
            extension,
        );
    }

    let start_node = validation_graph_endpoint_node(
        &mut validation_graph,
        existing_graph,
        &mut node_map,
        start_existing,
        prepared_points[0],
        extension,
    );
    let end_node = validation_graph_endpoint_node(
        &mut validation_graph,
        existing_graph,
        &mut node_map,
        end_existing,
        *prepared_points.last().unwrap(),
        extension,
    );
    if start_node == end_node {
        return None;
    }

    let mut points = prepared_points.to_vec();
    points[0] = validation_graph.node(start_node).pos;
    let last_idx = points.len() - 1;
    points[last_idx] = validation_graph.node(end_node).pos;
    let new_edge_idx = validation_graph.add_edge(build_surface_edge(
        start_node, end_node, points, fwd_lanes, bkw_lanes, edge_class,
    ));
    let (base_cost, physical_length) =
        crate::simulation::pathing::cost::CostCalculator::calculate_costs(
            validation_graph.edge(new_edge_idx),
        );
    validation_graph.edge_mut(new_edge_idx).base_cost = base_cost;
    validation_graph.edge_mut(new_edge_idx).physical_length = physical_length;
    let reprofiled_edge =
        extension.and_then(|extension| copied_edges.get(&extension.existing_edge_idx).copied());
    let (finalized, splits) = process_validation_graph_intersections(
        &mut validation_graph,
        new_edge_idx,
        reprofiled_edge,
    );

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

    // Profile-solving nodes must already have complete incidence. A later read-only halo
    // must not disguise an incomplete solve as a valid plan.
    if node_map.iter().any(|(&source, &local)| {
        finalized.affected_nodes.contains(&local)
            && existing_graph
                .node_adjacency(source)
                .iter()
                .any(|id| !existing_graph.edge(*id).deleted && !copied_edges.contains_key(id))
    }) {
        return None;
    }

    Some(ValidationGraph {
        graph: validation_graph,
        new_edge_idx,
        required_edge_ids,
        source_node_ids: node_map,
        source_edge_ids: copied_edges,
        finalized,
        splits,
    })
}

#[allow(clippy::too_many_arguments)]
fn candidate_existing_edge_ids(
    prepared_points: &[Vector3],
    fwd_lanes: u8,
    bkw_lanes: u8,
    edge_class: EdgeClass,
    existing_graph: &RegionGraph,
    extension: Option<&RoadExtensionReprofile>,
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
        + preview_candidate_roadbed_half_width_m(prepared_points, edge_class, fwd_lanes, bkw_lanes);
    let mut edge_ids = existing_graph.get_edges_near_aabb(
        Vector3::new(min_x - pad, 0.0, min_z - pad),
        Vector3::new(max_x + pad, 0.0, max_z + pad),
    );
    if let Some(extension) = extension {
        edge_ids.push(extension.existing_edge_idx);
    }
    for existing_node in [start_existing, end_existing].into_iter().flatten() {
        push_incident_validation_edges(existing_graph, existing_node, &mut edge_ids);
    }

    edge_ids.sort_unstable();
    edge_ids.dedup();

    let mut incident_nodes = Vec::new();
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
    for node_id in incident_nodes {
        push_incident_validation_edges(existing_graph, node_id, &mut edge_ids);
    }

    edge_ids.sort_unstable();
    edge_ids.dedup();
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

fn process_validation_graph_intersections(
    validation_graph: &mut RegionGraph,
    new_edge_idx: usize,
    reprofiled_edge: Option<usize>,
) -> (FinalizedRoadGeometry, Vec<PlannedRoadSplit>) {
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
    network.mark_road_profile_authored(new_edge_idx);
    // Terminal extensions already replaced the source edge's profile in the excerpt.
    // Commit adds that edge to the bulk ledger too; omitting it re-solves sloped bends
    // with a different set of pinned/adjustable endpoints than authoritative insertion.
    if let Some(edge_idx) = reprofiled_edge {
        network.bulk_dirty_edges.insert(edge_idx);
        network.mark_road_profile_authored(edge_idx);
    }
    topology::process_intersections(
        &mut network,
        validation_graph,
        new_edge_idx,
        &mut zoning,
        &mut allocator,
    );
    cleanup_validation_duplicate_edges(validation_graph);

    let surface_dirty_nodes = road_insert_surface_dirty_nodes(
        validation_graph,
        start_node,
        end_node,
        new_edge_idx,
        node_count_before,
        edge_count_before,
    );
    network.mark_surface_dirty_for_nodes(validation_graph, &surface_dirty_nodes);

    let finalized = network.finalize_road_geometry(validation_graph);
    (finalized, network.recorded_road_splits.take().unwrap())
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
    extension: Option<&RoadExtensionReprofile>,
) -> u32 {
    match existing_node {
        Some(node_id) => copy_validation_graph_node(
            validation_graph,
            existing_graph,
            node_map,
            node_id,
            extension,
        ),
        None => validation_graph.add_node(fallback_pos, NodeType::Junction),
    }
}

fn copy_validation_graph_node(
    validation_graph: &mut RegionGraph,
    existing_graph: &RegionGraph,
    node_map: &mut HashMap<u32, u32>,
    node_id: u32,
    extension: Option<&RoadExtensionReprofile>,
) -> u32 {
    let valid = existing_graph.get_valid_node(node_id);
    if let Some(&mapped) = node_map.get(&valid) {
        return mapped;
    }

    let node = existing_graph.node(valid);
    let pos = extension
        .filter(|extension| existing_graph.get_valid_node(extension.snapped_node_id) == valid)
        .map_or(node.pos, |extension| extension.snapped_node_pos);
    let mapped = validation_graph.add_node(pos, node.node_type);
    node_map.insert(valid, mapped);
    mapped
}

fn copy_validation_graph_edge(
    validation_graph: &mut RegionGraph,
    existing_graph: &RegionGraph,
    node_map: &mut HashMap<u32, u32>,
    copied_edges: &mut HashMap<usize, usize>,
    edge_idx: usize,
    extension: Option<&RoadExtensionReprofile>,
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
        extension,
    );
    let end_node = copy_validation_graph_node(
        validation_graph,
        existing_graph,
        node_map,
        edge.end_node,
        extension,
    );
    if start_node == end_node {
        return;
    }

    let mut copied_edge = edge.clone();
    copied_edge.start_node = start_node;
    copied_edge.end_node = end_node;
    if let Some(extension) = extension
        && edge_idx == extension.existing_edge_idx
    {
        copied_edge.geometry = extension.existing_points.clone();
        copied_edge.physical_geometry = extension.existing_points.clone();
        let (cost, length) =
            crate::simulation::pathing::cost::CostCalculator::calculate_costs(&copied_edge);
        copied_edge.base_cost = cost;
        copied_edge.physical_length = length;
    }
    let local_edge = validation_graph.add_edge(copied_edge);
    copied_edges.insert(edge_idx, local_edge);
}

fn endpoint_existing_node(
    point: Vector3,
    existing_graph: &RegionGraph,
    extension: Option<&RoadExtensionReprofile>,
    endpoint_snap_enabled: bool,
) -> Option<u32> {
    if let Some(extension) = extension {
        let node_pos = existing_graph.node(extension.snapped_node_id).pos;
        let dx = point.x - node_pos.x;
        let dz = point.z - node_pos.z;
        if dx.hypot(dz) < config::SNAP_TOLERANCE {
            return Some(extension.snapped_node_id);
        }
    }
    if !endpoint_snap_enabled {
        return None;
    }
    existing_graph.find_node_within(point, config::SNAP_TOLERANCE)
}

// The illegal geometry this edit adds (`validate::edit_violation`). The drawn road is the
// inserted edge and every piece split off it; pieces split off existing roads are theirs.
// Its endpoints are judged against what the same nodes were before. O(stroke points +
// splits + endpoint incidence).
fn edit_geometry_violation(
    validation_graph: &RegionGraph,
    existing_graph: &RegionGraph,
    new_edge_idx: usize,
    splits: &[PlannedRoadSplit],
    source_node_ids: &HashMap<u32, u32>,
) -> Option<RoadGeometryViolation> {
    let mut stroke = vec![new_edge_idx];
    for split in splits {
        if stroke.contains(&split.edge_id) {
            stroke.push(split.new_edge_id);
        }
    }
    stroke.retain(|&id| !validation_graph.edge(id).deleted);
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
        .map(|node| {
            let node = validation_graph.get_valid_node(node);
            (node, source_of.get(&node).copied())
        })
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    crate::simulation::network::validate::edit_violation(
        validation_graph,
        &stroke,
        &nodes,
        existing_graph,
        &RoadGeometryLimits::default(),
    )
}

fn refuse_geometry(
    validation: RoadPreviewValidation,
    violation: &RoadGeometryViolation,
) -> RoadPreviewValidation {
    use RoadGeometryViolation::*;
    let (reason, measured, limit) = match *violation {
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
        Grade { .. } | DirectionKink { .. } => (PREVIEW_SURFACE_GEOMETRY_REASON, 0.0, 0.0),
    };
    let mut validation = validation.with_invalid_reason(reason);
    validation.geometry_measured = measured;
    validation.geometry_limit = limit;
    validation
}
