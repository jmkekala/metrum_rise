//! Edge input conditioning before preview or committed section compilation.

use super::super::RoadSurfaceSystem;
use crate::config;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

// Road profile preparation thresholds shared by preview and committed placement.
const ROAD_POINT_SIMPLIFY_DISTANCE_M: f32 = 0.5;
const ROAD_PROFILE_SAMPLE_STEP_M: f32 = 6.0;
const ROAD_PROFILE_SOLVER_ITERS: usize = 64;
const ROAD_PROFILE_SMOOTH_WEIGHT: f32 = 0.22;
const ROAD_PROFILE_TERRAIN_WEIGHT: f32 = 0.30;
const ROAD_PROFILE_MAX_GRADE_CHANGE_PER_M: f32 = 0.018;
pub(in crate::simulation::network::surface::edge) const PREVIEW_CLEARANCE_M: f32 = 1.0;
pub(in crate::simulation::network::surface::edge) const ROAD_PROFILE_MAX_GRADE: f32 = 0.16;

/// Solved road-tool geometry plus optional profile edits to an existing terminal edge.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRoadInput {
    /// Physical geometry for the new edge that will be previewed or committed.
    pub(crate) points: Vec<Vector3>,
    /// Physical geometry used to validate the full vertical profile solve.
    pub(crate) validation_points: Vec<Vector3>,
    /// Road class inferred from the input heights.
    pub(crate) class: EdgeClass,
    /// Existing terminal edge reprofile needed before committing the new edge.
    pub(crate) extension: Option<RoadExtensionReprofile>,
}

/// Existing-edge geometry changes produced by a terminal road-extension solve.
#[derive(Clone, Debug)]
pub(crate) struct RoadExtensionReprofile {
    /// Degree-1 graph node that becomes an internal road-profile point.
    pub(crate) snapped_node_id: u32,
    /// Existing edge incident to `snapped_node_id` whose profile is re-solved.
    pub(crate) existing_edge_idx: usize,
    /// Replacement geometry for the existing edge in its stored graph orientation.
    pub(crate) existing_points: Vec<Vector3>,
    /// Updated world-space position for `snapped_node_id`.
    pub(crate) snapped_node_pos: Vector3,
}

#[derive(Clone, Copy)]
struct RoadVerticalProfileSample {
    x: f32,
    z: f32,
    target_y: f32,
    y: f32,
    pinned: bool,
}

struct TerminalRoadExtensionPreparation {
    new_points: Vec<Vector3>,
    validation_points: Vec<Vector3>,
    reprofile: RoadExtensionReprofile,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalExtensionEndpoint {
    Start,
    End,
}

impl RoadSurfaceSystem {
    /// Prepares road-tool input into the physical geometry used by preview and committed roads.
    ///
    /// Standard roads keep the authored XZ alignment, are densified against terrain samples, and
    /// receive a constrained smooth vertical profile. Bridge and tunnel strokes preserve authored
    /// heights after light XZ simplification when an authored endpoint is intentionally off terrain.
    pub(crate) fn prepare_road_input_points(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> (Vec<Vector3>, EdgeClass) {
        Self::prepare_road_input_points_with_support(raw_points, terrain, |_, _| None)
    }

    /// Prepares road-tool input while using compiled visible road surfaces as hard profile pins.
    ///
    /// This keeps snapped/intersection road connections continuous while open-terrain samples stay
    /// near source terrain.
    pub(crate) fn prepare_road_input_points_to_visible_surface(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
    ) -> (Vec<Vector3>, EdgeClass) {
        Self::prepare_road_input_points_with_support(raw_points, terrain, |x, z| {
            road_surface.sample_visible_surface_height(graph, terrain, x, z)
        })
    }

    /// Prepares road-tool input and, when extending a terminal, solves the old and new edge as
    /// one vertical corridor.
    pub(crate) fn prepare_road_input_with_extension_to_visible_surface(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
    ) -> PreparedRoadInput {
        let (mut points, mut class) = Self::prepare_road_input_points_to_visible_surface(
            raw_points,
            terrain,
            graph,
            road_surface,
        );
        Self::snap_road_endpoints_to_existing_nodes(&mut points, graph);
        if class == EdgeClass::Standard
            && Self::terminal_bridge_transition_endpoint(&points, terrain, graph).is_some()
        {
            Self::solve_structural_transition_profile(&mut points);
            class = EdgeClass::Bridge;
        }

        if class != EdgeClass::Standard || points.len() < 2 {
            return PreparedRoadInput {
                validation_points: points.clone(),
                points,
                class,
                extension: None,
            };
        }

        if let Some(extension) =
            Self::prepare_terminal_road_extension(raw_points, &points, terrain, graph)
        {
            return PreparedRoadInput {
                points: extension.new_points,
                validation_points: extension.validation_points,
                class,
                extension: Some(extension.reprofile),
            };
        }

        PreparedRoadInput {
            validation_points: points.clone(),
            points,
            class,
            extension: None,
        }
    }

    /// Snaps prepared road endpoints to existing graph nodes using the same tolerance as commit.
    pub(crate) fn snap_road_endpoints_to_existing_nodes(
        prepared_points: &mut [Vector3],
        graph: &RegionGraph,
    ) {
        if prepared_points.len() < 2 {
            return;
        }

        if let Some(start_id) = graph.find_node_within(prepared_points[0], config::SNAP_TOLERANCE) {
            prepared_points[0] = graph.node(start_id).pos;
        }

        let last_idx = prepared_points.len() - 1;
        if let Some(end_id) =
            graph.find_node_within(prepared_points[last_idx], config::SNAP_TOLERANCE)
        {
            prepared_points[last_idx] = graph.node(end_id).pos;
        }
    }

    fn prepare_road_input_points_with_support(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
        mut visible_support_height_at: impl FnMut(f32, f32) -> Option<f32>,
    ) -> (Vec<Vector3>, EdgeClass) {
        let simplified_points = Self::simplify_road_alignment_points(raw_points);
        if simplified_points.len() < 2 {
            return (simplified_points, EdgeClass::Standard);
        }

        let mut endpoint_above_clearance = false;
        let mut endpoint_below_clearance = false;
        let last_idx = simplified_points.len() - 1;
        for (idx, point) in simplified_points.iter().enumerate() {
            let support_h = Self::road_profile_support_height(
                terrain,
                &mut visible_support_height_at,
                point.x,
                point.z,
            )
            .0;
            let clearance_m = point.y - support_h;
            if idx != 0 && idx != last_idx {
                continue;
            }
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
            return (simplified_points, class);
        }

        let mut samples = Self::build_standard_road_profile_samples(
            &simplified_points,
            terrain,
            &mut visible_support_height_at,
        );
        Self::solve_standard_road_profile(&mut samples);
        let prepared_points = samples
            .into_iter()
            .map(|sample| Vector3::new(sample.x, sample.y, sample.z))
            .collect();

        (prepared_points, EdgeClass::Standard)
    }

    fn prepare_terminal_road_extension(
        raw_points: &[Vector3],
        prepared_points: &[Vector3],
        terrain: &TerrainSystem,
        graph: &RegionGraph,
    ) -> Option<TerminalRoadExtensionPreparation> {
        let start_node = graph.find_node_within(prepared_points[0], config::SNAP_TOLERANCE);
        let end_node =
            graph.find_node_within(*prepared_points.last().unwrap(), config::SNAP_TOLERANCE);
        let start_extension = start_node.and_then(|node_id| {
            Self::terminal_extension_incident_edge(graph, node_id)
                .map(|edge_idx| (node_id, edge_idx, TerminalExtensionEndpoint::Start))
        });
        let end_extension = end_node.and_then(|node_id| {
            Self::terminal_extension_incident_edge(graph, node_id)
                .map(|edge_idx| (node_id, edge_idx, TerminalExtensionEndpoint::End))
        });

        let extension = match (start_extension, end_extension) {
            (Some(extension), None) | (None, Some(extension)) => extension,
            _ => return None,
        };

        let (snapped_node_id, existing_edge_idx, endpoint) = extension;
        Self::build_terminal_extension_profile(
            raw_points,
            prepared_points,
            terrain,
            graph,
            snapped_node_id,
            existing_edge_idx,
            endpoint,
        )
    }

    fn terminal_extension_incident_edge(graph: &RegionGraph, node_id: u32) -> Option<usize> {
        Self::terminal_incident_surface_edge(graph, node_id, EdgeClass::Standard)
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
                    && Self::is_surface_edge(graph.edge(edge_idx))
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

    fn terminal_bridge_transition_endpoint(
        points: &[Vector3],
        terrain: &TerrainSystem,
        graph: &RegionGraph,
    ) -> Option<TerminalExtensionEndpoint> {
        let first = *points.first()?;
        let last = *points.last()?;
        if points.len() < 2 {
            return None;
        }

        let start_is_elevated_bridge_terminal = graph
            .find_node_within(first, config::SNAP_TOLERANCE)
            .filter(|&node_id| {
                Self::terminal_incident_surface_edge(graph, node_id, EdgeClass::Bridge).is_some()
            })
            .is_some()
            && Self::source_terrain_clearance_m(first, terrain) > PREVIEW_CLEARANCE_M;
        let end_is_elevated_bridge_terminal = graph
            .find_node_within(last, config::SNAP_TOLERANCE)
            .filter(|&node_id| {
                Self::terminal_incident_surface_edge(graph, node_id, EdgeClass::Bridge).is_some()
            })
            .is_some()
            && Self::source_terrain_clearance_m(last, terrain) > PREVIEW_CLEARANCE_M;
        let start_is_grounded =
            Self::source_terrain_clearance_m(first, terrain).abs() <= PREVIEW_CLEARANCE_M;
        let end_is_grounded =
            Self::source_terrain_clearance_m(last, terrain).abs() <= PREVIEW_CLEARANCE_M;

        match (
            start_is_elevated_bridge_terminal && end_is_grounded,
            end_is_elevated_bridge_terminal && start_is_grounded,
        ) {
            (true, false) => Some(TerminalExtensionEndpoint::Start),
            (false, true) => Some(TerminalExtensionEndpoint::End),
            _ => None,
        }
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
            .map(|segment| Self::xz_distance(segment[0], segment[1]))
            .sum();
        if total_length_m <= f32::EPSILON {
            return;
        }

        let mut station_m = 0.0;
        for index in 1..points.len() {
            station_m += Self::xz_distance(points[index - 1], points[index]);
            let t = (station_m / total_length_m).clamp(0.0, 1.0);
            points[index].y = first.y + (last.y - first.y) * t;
        }
    }

    fn build_terminal_extension_profile(
        raw_points: &[Vector3],
        prepared_points: &[Vector3],
        terrain: &TerrainSystem,
        graph: &RegionGraph,
        snapped_node_id: u32,
        existing_edge_idx: usize,
        endpoint: TerminalExtensionEndpoint,
    ) -> Option<TerminalRoadExtensionPreparation> {
        let existing_edge = graph.edge(existing_edge_idx);
        let stored_existing_points = if existing_edge.geometry.len() >= 2 {
            &existing_edge.geometry
        } else {
            &existing_edge.physical_geometry
        };
        if stored_existing_points.len() < 2 {
            return None;
        }

        let mut new_alignment = Self::simplify_road_alignment_points(raw_points);
        if new_alignment.len() < 2 {
            return None;
        }

        let snapped_node_pos = graph.node(snapped_node_id).pos;
        match endpoint {
            TerminalExtensionEndpoint::Start => {
                new_alignment[0] = snapped_node_pos;
                let last_idx = new_alignment.len() - 1;
                new_alignment[last_idx] = *prepared_points.last().unwrap();
            }
            TerminalExtensionEndpoint::End => {
                new_alignment[0] = prepared_points[0];
                let last_idx = new_alignment.len() - 1;
                new_alignment[last_idx] = snapped_node_pos;
            }
        }

        let mut existing_corridor_points = stored_existing_points.to_vec();
        let existing_corridor_matches_stored_orientation = match endpoint {
            TerminalExtensionEndpoint::Start => existing_edge.end_node == snapped_node_id,
            TerminalExtensionEndpoint::End => existing_edge.start_node == snapped_node_id,
        };
        if !existing_corridor_matches_stored_orientation {
            existing_corridor_points.reverse();
        }

        let mut combined_alignment = match endpoint {
            TerminalExtensionEndpoint::Start => existing_corridor_points.clone(),
            TerminalExtensionEndpoint::End => new_alignment.clone(),
        };
        match endpoint {
            TerminalExtensionEndpoint::Start => {
                combined_alignment.extend_from_slice(&new_alignment[1..]);
            }
            TerminalExtensionEndpoint::End => {
                combined_alignment.extend_from_slice(&existing_corridor_points[1..]);
            }
        }
        if combined_alignment.len() < 2 {
            return None;
        }

        let join_xz = Vector3::new(snapped_node_pos.x, 0.0, snapped_node_pos.z);
        let mut samples = Self::build_standard_road_profile_samples_with_endpoint_height_pins(
            &combined_alignment,
            terrain,
        );
        Self::solve_standard_road_profile(&mut samples);
        let solved_points: Vec<Vector3> = samples
            .into_iter()
            .map(|sample| Vector3::new(sample.x, sample.y, sample.z))
            .collect();
        let join_idx = Self::nearest_xz_point_index(&solved_points, join_xz)?;

        let (new_points, mut existing_points) = match endpoint {
            TerminalExtensionEndpoint::Start => (
                solved_points[join_idx..].to_vec(),
                solved_points[..=join_idx].to_vec(),
            ),
            TerminalExtensionEndpoint::End => (
                solved_points[..=join_idx].to_vec(),
                solved_points[join_idx..].to_vec(),
            ),
        };
        if !existing_corridor_matches_stored_orientation {
            existing_points.reverse();
        }
        if new_points.len() < 2 || existing_points.len() < 2 {
            return None;
        }

        let snapped_node_pos = solved_points[join_idx];
        Some(TerminalRoadExtensionPreparation {
            new_points,
            validation_points: solved_points,
            reprofile: RoadExtensionReprofile {
                snapped_node_id,
                existing_edge_idx,
                existing_points,
                snapped_node_pos,
            },
        })
    }

    fn simplify_road_alignment_points(points: &[Vector3]) -> Vec<Vector3> {
        let mut simplified_points = Vec::with_capacity(points.len());
        if !points.is_empty() {
            simplified_points.push(points[0]);
            for point in points.iter().skip(1) {
                if Self::xz_distance(*point, *simplified_points.last().unwrap())
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
        simplified_points
    }

    fn build_standard_road_profile_samples(
        alignment_points: &[Vector3],
        terrain: &TerrainSystem,
        visible_support_height_at: &mut impl FnMut(f32, f32) -> Option<f32>,
    ) -> Vec<RoadVerticalProfileSample> {
        let mut samples = Vec::new();
        for segment in alignment_points.windows(2) {
            let a = segment[0];
            let b = segment[1];
            let len_xz = Self::xz_distance(a, b);
            if len_xz <= f32::EPSILON {
                continue;
            }
            if samples.is_empty() {
                Self::push_standard_road_profile_sample(
                    &mut samples,
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
                Self::push_standard_road_profile_sample(
                    &mut samples,
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
        samples
    }

    fn build_standard_road_profile_samples_with_endpoint_height_pins(
        alignment_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> Vec<RoadVerticalProfileSample> {
        let mut no_visible_support = |_: f32, _: f32| None;
        let mut samples = Self::build_standard_road_profile_samples(
            alignment_points,
            terrain,
            &mut no_visible_support,
        );
        if samples.is_empty() {
            return samples;
        }

        if let Some(first_alignment) = alignment_points.first()
            && let Some(first) = samples.first_mut()
        {
            first.target_y = first_alignment.y;
            first.y = first_alignment.y;
            first.pinned = true;
        }
        if let Some(last_alignment) = alignment_points.last()
            && let Some(last) = samples.last_mut()
        {
            last.target_y = last_alignment.y;
            last.y = last_alignment.y;
            last.pinned = true;
        }
        samples
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
            Self::road_profile_support_height(terrain, visible_support_height_at, x, z);
        samples.push(RoadVerticalProfileSample {
            x,
            z,
            target_y,
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

    fn solve_standard_road_profile(samples: &mut [RoadVerticalProfileSample]) {
        if samples.len() <= 2 {
            return;
        }

        let mut previous_y = vec![0.0; samples.len()];
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
            Self::restore_road_profile_pins(samples);
            Self::enforce_road_profile_grade(samples);
            Self::restore_road_profile_pins(samples);
            Self::enforce_road_profile_curvature(samples);
            Self::restore_road_profile_pins(samples);
            Self::enforce_road_profile_grade(samples);
            Self::restore_road_profile_pins(samples);
        }
    }

    fn restore_road_profile_pins(samples: &mut [RoadVerticalProfileSample]) {
        for sample in samples {
            if sample.pinned {
                sample.y = sample.target_y;
            }
        }
    }

    fn enforce_road_profile_grade(samples: &mut [RoadVerticalProfileSample]) {
        if samples.len() < 2 {
            return;
        }

        for index in 1..samples.len() {
            if samples[index].pinned {
                continue;
            }
            let run_m = Self::profile_sample_run_m(samples[index - 1], samples[index]);
            let max_delta = ROAD_PROFILE_MAX_GRADE * run_m;
            samples[index].y = samples[index].y.clamp(
                samples[index - 1].y - max_delta,
                samples[index - 1].y + max_delta,
            );
        }

        for index in (0..samples.len() - 1).rev() {
            if samples[index].pinned {
                continue;
            }
            let run_m = Self::profile_sample_run_m(samples[index], samples[index + 1]);
            let max_delta = ROAD_PROFILE_MAX_GRADE * run_m;
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

            let prev_run_m = Self::profile_sample_run_m(samples[index - 1], samples[index]);
            let next_run_m = Self::profile_sample_run_m(samples[index], samples[index + 1]);
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

    fn nearest_xz_point_index(points: &[Vector3], target_xz: Vector3) -> Option<usize> {
        points
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = (a.x - target_xz.x).hypot(a.z - target_xz.z);
                let db = (b.x - target_xz.x).hypot(b.z - target_xz.z);
                da.total_cmp(&db)
            })
            .map(|(index, _)| index)
    }

    fn xz_distance(a: Vector3, b: Vector3) -> f32 {
        (b.x - a.x).hypot(b.z - a.z)
    }
}
