// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool input preparation: alignment simplification, endpoint snapping and the vertical
//! profile solve shared by preview and committed placement.

use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use super::support::{RebuiltPieces, StrokeSupport};
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::network::types::TransitType;
use crate::simulation::network::grade::{
    GradeScratch, ROAD_GRADE_MAX, ROAD_GRADE_WARNING, SourcePoint, construct_within_grade,
    least_grade, maximum_envelope, source_bounds, source_ground_m,
};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;
use std::cell::RefCell;

// Road profile preparation thresholds shared by preview and committed placement.
const ROAD_POINT_SIMPLIFY_DISTANCE_M: f32 = 0.5;
const ROAD_PROFILE_SAMPLE_STEP_M: f32 = 6.0;
const ROAD_PROFILE_SOLVER_ITERS: usize = 64;
const ROAD_PROFILE_SMOOTH_WEIGHT: f32 = 0.22;
const ROAD_PROFILE_TERRAIN_WEIGHT: f32 = 0.30;
const ROAD_PROFILE_MAX_GRADE_CHANGE_PER_M: f32 = 0.018;
pub(super) const PREVIEW_CLEARANCE_M: f32 = 1.0;
// The solve's design grade, which it relaxes span by span towards the ground's slope, up to the
// hard maximum (`ROAD-68` open question 37).
pub(super) const ROAD_PROFILE_MAX_GRADE: f32 = ROAD_GRADE_WARNING;

/// Solved road-tool geometry plus optional profile edits to an existing terminal edge.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRoadInput {
    /// Physical geometry for the new edge that will be previewed or committed.
    pub(crate) points: Vec<Vector3>,
    /// Road class inferred from the input heights.
    pub(crate) class: EdgeClass,
    /// True when endpoint proximity to existing graph nodes is treated as an authored snap.
    pub(crate) endpoint_snap_enabled: bool,
    /// The least maximum grade a standard profile can keep under the stroke's pins and the
    /// source-delta limit, when that exceeds the hard maximum (`ROAD-68` open question 37):
    /// the stroke is refused as `road_too_steep`. `None` when the solve constructed the profile
    /// within the maximum.
    pub(crate) forced_grade: Option<f32>,
}

#[derive(Clone, Copy)]
struct RoadVerticalProfileSample {
    x: f32,
    z: f32,
    target_y: f32,
    // The source terrain under the sample, which the source-delta limit measures from.
    ground_y: f32,
    y: f32,
    pinned: bool,
}

// The stroke solve's buffers, reused between solves so that a solve allocates only the polyline
// it returns once they have grown to its stroke (`ROAD-68-K16`): the solve runs on every
// preview pose.
#[derive(Default)]
struct StrokeScratch {
    alignment: Vec<Vector3>,
    samples: Vec<RoadVerticalProfileSample>,
    grade_limits: Vec<f32>,
    previous_y: Vec<f32>,
    grade: GradeScratch,
}

thread_local! {
    // One per thread: pool workers keep theirs between solves.
    static STROKE_SCRATCH: RefCell<StrokeScratch> = RefCell::default();
}

// Prepares road-tool input into the physical geometry used by preview and committed roads.
//
// Standard roads keep the authored XZ alignment, are densified against terrain samples, and
// receive a constrained smooth vertical profile. Bridge and tunnel strokes preserve authored
// heights after light XZ simplification when an authored endpoint is intentionally off terrain.
fn prepare_road_input_points(
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
) -> (Vec<Vector3>, EdgeClass, Option<f32>) {
    prepare_road_input_points_with_support(raw_points, terrain, |_, _| None, [None; 2])
}

// Prepares road-tool input while using visible carriageways as hard profile pins.
//
// This keeps snapped/intersection road connections continuous while open-terrain samples stay
// near source terrain. Only carriageways pin: a centreline joins the carriageway it meets and
// never climbs onto the raised sidewalk, curb or cap of a piece the edit itself rebuilds, so
// its profile does not depend on which roads were drawn first (`ROAD-68` open question 15).
// The carriageways of pieces the edit rebuilds do not pin at all (question 16, `StrokeSupport`).
fn prepare_road_input_points_to_visible_surface(
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    support: &mut StrokeSupport,
) -> (Vec<Vector3>, EdgeClass, Option<f32>) {
    let end_heights = [support.end_height(0), support.end_height(1)];
    prepare_road_input_points_with_support(
        raw_points,
        terrain,
        |x, z| support.height_at(x, z),
        end_heights,
    )
}

// Prepares road-tool input pinned to the visible surface and snaps its ends to existing nodes.
// A stroke from a road end joins it like any node, at the node's graph height: the road it
// extends keeps its own profile (`ROAD-68` open question 30), and the node's plane solve joins
// the two.
fn prepare_snapped_road_input_to_visible_surface(
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    support: &mut StrokeSupport,
) -> PreparedRoadInput {
    let (mut points, mut class, forced_grade) =
        prepare_road_input_points_to_visible_surface(raw_points, terrain, support);
    snap_road_endpoints_to_existing_nodes(&mut points, graph);
    if class == EdgeClass::Standard && is_terminal_bridge_transition(&points, terrain, graph)
    {
        solve_structural_transition_profile(&mut points);
        class = EdgeClass::Bridge;
    }
    PreparedRoadInput {
        points,
        class,
        endpoint_snap_enabled: true,
        forced_grade: forced_grade.filter(|_| class == EdgeClass::Standard),
    }
}

/// Prepares road-tool input with optional endpoint snapping to the existing road graph, pinned
/// to every visible carriageway: the first pass of [`super::plan_road_edit`], which the commit,
/// the preview and the editor's checks use (`ROAD-68` open question 16).
pub(crate) fn prepare_road_input_for_tool(
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    snap_to_existing_roads: bool,
) -> PreparedRoadInput {
    let excluded = RebuiltPieces::default();
    prepare_road_input_excluding(
        raw_points,
        terrain,
        graph,
        roadbed,
        snap_to_existing_roads,
        &excluded,
    )
    .0
}

// As `prepare_road_input_for_tool`, reading no carriageway of the `excluded` pieces; also
// returns the pieces whose carriageway the preparation read.
pub(super) fn prepare_road_input_excluding(
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    snap_to_existing_roads: bool,
    excluded: &RebuiltPieces,
) -> (PreparedRoadInput, Vec<PieceId>) {
    if snap_to_existing_roads {
        let mut support = StrokeSupport::new(roadbed, graph, excluded);
        let prepared = prepare_snapped_road_input_to_visible_surface(
            raw_points,
            terrain,
            graph,
            &mut support,
        );
        return (prepared, support.into_read());
    }

    let (points, class, forced_grade) = prepare_road_input_points(raw_points, terrain);
    let prepared = PreparedRoadInput {
        points,
        class,
        endpoint_snap_enabled: false,
        forced_grade,
    };
    (prepared, Vec::new())
}

// Snaps prepared road endpoints to existing graph nodes using the same tolerance as commit.
fn snap_road_endpoints_to_existing_nodes(prepared_points: &mut [Vector3], graph: &RegionGraph) {
    if prepared_points.len() < 2 {
        return;
    }

    if let Some(start_id) = graph.find_node_within(prepared_points[0], config::SNAP_TOLERANCE) {
        prepared_points[0] = graph.node(start_id).pos;
    }

    let last_idx = prepared_points.len() - 1;
    if let Some(end_id) = graph.find_node_within(prepared_points[last_idx], config::SNAP_TOLERANCE)
    {
        prepared_points[last_idx] = graph.node(end_id).pos;
    }
}

// `end_heights` replace the support at the first and last point: the graph height of a node
// the stroke's end joins, when its pieces are excluded from the support.
fn prepare_road_input_points_with_support(
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    visible_support_height_at: impl FnMut(f32, f32) -> Option<f32>,
    end_heights: [Option<f32>; 2],
) -> (Vec<Vector3>, EdgeClass, Option<f32>) {
    // Taken for the solve, not borrowed: a solve nested on this thread (a pool job stolen while
    // a support query waits) starts from empty buffers instead of failing a borrow.
    let mut scratch = STROKE_SCRATCH.take();
    let prepared = prepare_road_input_points_in(
        raw_points,
        terrain,
        visible_support_height_at,
        end_heights,
        &mut scratch,
    );
    STROKE_SCRATCH.set(scratch);
    prepared
}

fn prepare_road_input_points_in(
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    mut visible_support_height_at: impl FnMut(f32, f32) -> Option<f32>,
    end_heights: [Option<f32>; 2],
    scratch: &mut StrokeScratch,
) -> (Vec<Vector3>, EdgeClass, Option<f32>) {
    let StrokeScratch {
        alignment: simplified_points,
        samples,
        grade_limits,
        previous_y,
        grade,
    } = scratch;
    simplify_road_alignment_points(raw_points, simplified_points);
    if simplified_points.len() < 2 {
        return (simplified_points.clone(), EdgeClass::Standard, None);
    }

    let mut endpoint_above_clearance = false;
    let mut endpoint_below_clearance = false;
    let last_idx = simplified_points.len() - 1;
    for (end, idx) in [0, last_idx].into_iter().enumerate() {
        let point = simplified_points[idx];
        let support_h = end_heights[end].unwrap_or_else(|| {
            road_profile_support_height(terrain, &mut visible_support_height_at, point.x, point.z)
                .0
        });
        let clearance_m = point.y - support_h;
        if clearance_m > PREVIEW_CLEARANCE_M {
            endpoint_above_clearance = true;
        }
        if clearance_m < -PREVIEW_CLEARANCE_M {
            endpoint_below_clearance = true;
        }
    }

    let class = match (endpoint_above_clearance, endpoint_below_clearance) {
        (true, false) => EdgeClass::Bridge,
        (false, true) => EdgeClass::Tunnel,
        _ => EdgeClass::Standard,
    };

    if class != EdgeClass::Standard {
        return (simplified_points.clone(), class, None);
    }

    build_standard_road_profile_samples(
        simplified_points,
        terrain,
        &mut visible_support_height_at,
        samples,
    );
    // The two ends are pinned already; a joined node's graph height replaces their support.
    let last = samples.len().saturating_sub(1);
    for (end, idx) in [0, last].into_iter().enumerate() {
        if let (Some(height_m), Some(sample)) = (end_heights[end], samples.get_mut(idx)) {
            sample.target_y = height_m;
            sample.y = height_m;
        }
    }
    let forced_grade = solve_standard_road_profile(samples, grade_limits, previous_y, grade);
    let prepared_points = samples
        .iter()
        .map(|sample| Vector3::new(sample.x, sample.y, sample.z))
        .collect();

    (prepared_points, EdgeClass::Standard, forced_grade)
}

fn terminal_incident_surface_edge(
    graph: &RegionGraph,
    node_id: u32,
    required_class: EdgeClass,
) -> Option<usize> {
    let node_id = graph.get_valid_node(node_id);
    if node_id as usize >= graph.node_adjacency_count() {
        return None;
    }

    let mut active_edges = graph
        .node_adjacency(node_id)
        .iter()
        .copied()
        .filter(|&edge_idx| {
            edge_idx < graph.edge_count()
                && is_surface_edge(graph.edge(edge_idx))
                && graph
                    .edge(edge_idx)
                    .geometry
                    .len()
                    .max(graph.edge(edge_idx).physical_geometry.len())
                    >= 2
        });

    let edge_idx = active_edges.next()?;
    (active_edges.next().is_none() && graph.edge(edge_idx).class == required_class)
        .then_some(edge_idx)
}

fn is_terminal_bridge_transition(
    points: &[Vector3],
    terrain: &TerrainSystem,
    graph: &RegionGraph,
) -> bool {
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return false;
    };
    if points.len() < 2 {
        return false;
    }

    let start_is_elevated_bridge_terminal = graph
        .find_node_within(first, config::SNAP_TOLERANCE)
        .filter(|&node_id| {
            terminal_incident_surface_edge(graph, node_id, EdgeClass::Bridge).is_some()
        })
        .is_some()
        && source_terrain_clearance_m(first, terrain) > PREVIEW_CLEARANCE_M;
    let end_is_elevated_bridge_terminal = graph
        .find_node_within(last, config::SNAP_TOLERANCE)
        .filter(|&node_id| {
            terminal_incident_surface_edge(graph, node_id, EdgeClass::Bridge).is_some()
        })
        .is_some()
        && source_terrain_clearance_m(last, terrain) > PREVIEW_CLEARANCE_M;
    let start_is_grounded = source_terrain_clearance_m(first, terrain).abs() <= PREVIEW_CLEARANCE_M;
    let end_is_grounded = source_terrain_clearance_m(last, terrain).abs() <= PREVIEW_CLEARANCE_M;

    (start_is_elevated_bridge_terminal && end_is_grounded)
        != (end_is_elevated_bridge_terminal && start_is_grounded)
}

fn source_terrain_clearance_m(point: Vector3, terrain: &TerrainSystem) -> f32 {
    point.y - terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE
}

fn solve_structural_transition_profile(points: &mut [Vector3]) {
    let Some(first) = points.first().copied() else {
        return;
    };
    let Some(last) = points.last().copied() else {
        return;
    };
    let total_length_m: f32 = points
        .windows(2)
        .map(|segment| xz_distance(segment[0], segment[1]))
        .sum();
    if total_length_m <= f32::EPSILON {
        return;
    }

    let mut station_m = 0.0;
    for index in 1..points.len() {
        station_m += xz_distance(points[index - 1], points[index]);
        let t = (station_m / total_length_m).clamp(0.0, 1.0);
        points[index].y = first.y + (last.y - first.y) * t;
    }
}

// Into `simplified_points`, which allocates only to grow.
fn simplify_road_alignment_points(points: &[Vector3], simplified_points: &mut Vec<Vector3>) {
    simplified_points.clear();
    if !points.is_empty() {
        simplified_points.push(points[0]);
        for point in points.iter().skip(1) {
            if xz_distance(*point, *simplified_points.last().unwrap())
                > ROAD_POINT_SIMPLIFY_DISTANCE_M
            {
                simplified_points.push(*point);
            }
        }
        if simplified_points.len() > 1
            && simplified_points.last().unwrap() != points.last().unwrap()
        {
            simplified_points.pop();
            simplified_points.push(*points.last().unwrap());
        }
    }
}

// Into `samples`, which allocates only to grow.
fn build_standard_road_profile_samples(
    alignment_points: &[Vector3],
    terrain: &TerrainSystem,
    visible_support_height_at: &mut impl FnMut(f32, f32) -> Option<f32>,
    samples: &mut Vec<RoadVerticalProfileSample>,
) {
    samples.clear();
    for segment in alignment_points.windows(2) {
        let a = segment[0];
        let b = segment[1];
        let len_xz = xz_distance(a, b);
        if len_xz <= f32::EPSILON {
            continue;
        }
        if samples.is_empty() {
            push_standard_road_profile_sample(
                samples,
                terrain,
                visible_support_height_at,
                a.x,
                a.z,
                false,
            );
        }
        let segment_count = (len_xz / ROAD_PROFILE_SAMPLE_STEP_M).ceil().max(1.0) as usize;
        for step in 1..=segment_count {
            let t = step as f32 / segment_count as f32;
            push_standard_road_profile_sample(
                samples,
                terrain,
                visible_support_height_at,
                a.x + (b.x - a.x) * t,
                a.z + (b.z - a.z) * t,
                false,
            );
        }
    }

    if let Some(first) = samples.first_mut() {
        first.pinned = true;
    }
    if let Some(last) = samples.last_mut() {
        last.pinned = true;
    }
}

fn push_standard_road_profile_sample(
    samples: &mut Vec<RoadVerticalProfileSample>,
    terrain: &TerrainSystem,
    visible_support_height_at: &mut impl FnMut(f32, f32) -> Option<f32>,
    x: f32,
    z: f32,
    pinned: bool,
) {
    let (target_y, visible_support_found) =
        road_profile_support_height(terrain, visible_support_height_at, x, z);
    samples.push(RoadVerticalProfileSample {
        x,
        z,
        target_y,
        ground_y: source_ground_m(terrain, x, z),
        y: target_y,
        pinned: pinned || visible_support_found,
    });
}

fn road_profile_support_height(
    terrain: &TerrainSystem,
    visible_support_height_at: &mut impl FnMut(f32, f32) -> Option<f32>,
    x: f32,
    z: f32,
) -> (f32, bool) {
    if let Some(support_h) = visible_support_height_at(x, z) {
        return (support_h, true);
    }
    (
        terrain.sample_height_world(x, z) * config::HEIGHT_SCALE,
        false,
    )
}

// Solves a standard road's heights through its samples and returns the least maximum grade its
// constraints force, when that exceeds the hard maximum. The pinned samples (the ends and the
// carriageways the stroke meets) and the source-delta limit bound where the profile may lie; when
// a profile within 33% (else within the 1:3 maximum) exists there, the solve constructs one: the
// smoothing solve below, then the nearest heights inside those bounds (`ROAD-68` open question
// 37). The other arguments are working buffers; O(samples), no allocation beyond their growing.
fn solve_standard_road_profile(
    samples: &mut [RoadVerticalProfileSample],
    grade_limits: &mut Vec<f32>,
    previous_y: &mut Vec<f32>,
    scratch: &mut GradeScratch,
) -> Option<f32> {
    let GradeScratch {
        runs,
        sources,
        bounds,
        envelope,
        heights,
        backward,
    } = scratch;
    runs.clear();
    runs.extend(
        samples
            .windows(2)
            .map(|span| profile_sample_run_m(span[0], span[1])),
    );
    // A pinned sample at its target, any other within the source-delta limit of the ground.
    sources.clear();
    sources.extend(samples.iter().map(|sample| SourcePoint {
        ground_m: sample.ground_y,
        held_m: sample.pinned.then_some(sample.target_y),
    }));
    source_bounds(sources, runs, bounds);
    // Built within 33% where the constraints allow it, else within the 1:3 maximum.
    let Some(max_grade) = maximum_envelope(bounds, runs, envelope) else {
        let forced_grade = least_grade(bounds, runs);
        solve_smooth_road_profile(samples, ROAD_GRADE_MAX, grade_limits, previous_y);
        return Some(forced_grade);
    };
    solve_smooth_road_profile(samples, max_grade, grade_limits, previous_y);
    heights.clear();
    heights.extend(samples.iter().map(|sample| sample.y));
    construct_within_grade(heights, envelope, runs, max_grade, backward);
    for (sample, &height) in samples.iter_mut().zip(&*heights) {
        sample.y = height;
    }
    None
}

fn solve_smooth_road_profile(
    samples: &mut [RoadVerticalProfileSample],
    max_grade: f32,
    grade_limits: &mut Vec<f32>,
    previous_y: &mut Vec<f32>,
) {
    if samples.len() <= 2 {
        return;
    }

    // The design grade is a target, not a hard gameplay rejection limit. Preserve
    // a feasible source-supported envelope at EACH interval: equal-height pins can
    // still surround a hill or valley that a uniform design-grade clamp would erase.
    // Local bounds also avoid lending one steep section's allowance to the entire
    // road. Each span relaxes only up to `max_grade`, so steeper ground is cut or
    // filled. Compute once in O(P); the fixed-pass smoothing/curvature solve stays O(P).
    grade_limits.clear();
    grade_limits.extend(samples.windows(2).map(|span| {
        ((span[1].target_y - span[0].target_y).abs() / profile_sample_run_m(span[0], span[1]))
            .clamp(ROAD_PROFILE_MAX_GRADE, max_grade)
    }));
    previous_y.clear();
    previous_y.resize(samples.len(), 0.0);
    for _ in 0..ROAD_PROFILE_SOLVER_ITERS {
        for (dst, sample) in previous_y.iter_mut().zip(samples.iter()) {
            *dst = sample.y;
        }

        for index in 1..samples.len() - 1 {
            if samples[index].pinned {
                continue;
            }
            let neighbor_y = 0.5 * (previous_y[index - 1] + previous_y[index + 1]);
            let smooth_delta = neighbor_y - previous_y[index];
            let terrain_delta = samples[index].target_y - previous_y[index];
            samples[index].y = previous_y[index]
                + smooth_delta * ROAD_PROFILE_SMOOTH_WEIGHT
                + terrain_delta * ROAD_PROFILE_TERRAIN_WEIGHT;
        }
        restore_road_profile_pins(samples);
        enforce_road_profile_grade(samples, grade_limits);
        restore_road_profile_pins(samples);
        enforce_road_profile_curvature(samples);
        restore_road_profile_pins(samples);
        enforce_road_profile_grade(samples, grade_limits);
        restore_road_profile_pins(samples);
    }
}

fn restore_road_profile_pins(samples: &mut [RoadVerticalProfileSample]) {
    for sample in samples {
        if sample.pinned {
            sample.y = sample.target_y;
        }
    }
}

fn enforce_road_profile_grade(samples: &mut [RoadVerticalProfileSample], grade_limits: &[f32]) {
    if samples.len() < 2 {
        return;
    }

    for index in 1..samples.len() {
        if samples[index].pinned {
            continue;
        }
        let run_m = profile_sample_run_m(samples[index - 1], samples[index]);
        let max_delta = grade_limits[index - 1] * run_m;
        samples[index].y = samples[index].y.clamp(
            samples[index - 1].y - max_delta,
            samples[index - 1].y + max_delta,
        );
    }

    for index in (0..samples.len() - 1).rev() {
        if samples[index].pinned {
            continue;
        }
        let run_m = profile_sample_run_m(samples[index], samples[index + 1]);
        let max_delta = grade_limits[index] * run_m;
        samples[index].y = samples[index].y.clamp(
            samples[index + 1].y - max_delta,
            samples[index + 1].y + max_delta,
        );
    }
}

fn enforce_road_profile_curvature(samples: &mut [RoadVerticalProfileSample]) {
    if samples.len() < 3 {
        return;
    }

    for index in 1..samples.len() - 1 {
        if samples[index].pinned {
            continue;
        }

        let prev_run_m = profile_sample_run_m(samples[index - 1], samples[index]);
        let next_run_m = profile_sample_run_m(samples[index], samples[index + 1]);
        let max_grade_change =
            ROAD_PROFILE_MAX_GRADE_CHANGE_PER_M * ((prev_run_m + next_run_m) * 0.5);
        let weighted_neighbors =
            samples[index - 1].y / prev_run_m + samples[index + 1].y / next_run_m;
        let weight = 1.0 / prev_run_m + 1.0 / next_run_m;
        let min_y = (weighted_neighbors - max_grade_change) / weight;
        let max_y = (weighted_neighbors + max_grade_change) / weight;
        samples[index].y = samples[index].y.clamp(min_y, max_y);
    }
}

fn profile_sample_run_m(a: RoadVerticalProfileSample, b: RoadVerticalProfileSample) -> f32 {
    ((b.x - a.x).hypot(b.z - a.z)).max(f32::EPSILON)
}

fn xz_distance(a: Vector3, b: Vector3) -> f32 {
    (b.x - a.x).hypot(b.z - a.z)
}

// Road and footpath edges carry a visible roadbed; other transit edges do not.
pub(super) fn is_surface_edge(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feasible_endpoints_do_not_flatten_an_interior_hill_or_valley() {
        // Equal endpoint heights satisfy the design grade, but say nothing about
        // the relief between them. This broad curve is already curvature-feasible.
        for sign in [-1.0, 1.0] {
            let mut samples: Vec<_> = (-10_i32..=10)
                .map(|i| {
                    let x = i as f32 * 6.0;
                    // Up to 30% at the ends, within the 1:3 maximum (`ROAD-68` question 37).
                    let y = sign * 0.0025 * x * x;
                    RoadVerticalProfileSample {
                        x,
                        z: 0.0,
                        target_y: y,
                        ground_y: y,
                        y,
                        pinned: i.abs() == 10,
                    }
                })
                .collect();
            let mut scratch = StrokeScratch::default();
            solve_standard_road_profile(
                &mut samples,
                &mut scratch.grade_limits,
                &mut scratch.previous_y,
                &mut scratch.grade,
            );
            for sample in &samples {
                assert!(
                    (sample.y - sample.target_y).abs() < 0.2,
                    "interior relief lost at {}: {} vs {}",
                    sample.x,
                    sample.y,
                    sample.target_y
                );
                if sample.pinned {
                    assert_eq!(sample.y, sample.target_y);
                }
            }
        }
    }
}
