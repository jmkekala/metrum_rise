//! Edge input conditioning, preview compilation, and sampled cross-section generation.

use super::{
    CompiledNodeKind, IncidentEdgeSide, PreviewRoadSurfaceResult, RoadSurfaceBand,
    RoadSurfaceBandKind, RoadSurfaceSection, RoadSurfaceSystem, SAMPLE_EPSILON_M,
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};

// Longitudinal section sampling cadence by road-edge class.
const STANDARD_SECTION_STEP_M: f32 = 8.0;
const BRIDGE_SECTION_STEP_M: f32 = 12.0;
const TUNNEL_SECTION_STEP_M: f32 = 10.0;

// Road input conditioning and preview validation thresholds.
const ROAD_POINT_SIMPLIFY_DISTANCE_M: f32 = 0.5;
const TAUBIN_SMOOTHING_ITERS: usize = 50;
const TAUBIN_LAMBDA: f32 = 0.5;
const TAUBIN_MU: f32 = -0.53;
const PREVIEW_MAX_GRADE: f32 = 0.41;
const PREVIEW_CLEARANCE_M: f32 = 1.0;
const PREVIEW_MESH_LIFT_M: f32 = 0.05;

// Standard roadbed lateral shaping.
pub(super) const CURB_BAND_WIDTH_M: f32 = 0.15;
pub(super) const CURB_STEP_HEIGHT_M: f32 = 0.12;

// Visual span/node ownership handoff guards.
const VISUAL_NODE_HANDOFF_PADDING_M: f32 = 1.0;
const VISUAL_MIN_SPAN_LENGTH_M: f32 = 0.5;
const VISUAL_CONFLICT_PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const VISUAL_CONFLICT_SIN_EPSILON: f32 = 1.0e-3;

impl RoadSurfaceSystem {
    /// Grounds standard-road input to terrain and classifies bridge / tunnel previews using the
    /// same threshold as committed placement.
    pub(crate) fn classify_and_ground_road_points(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> (Vec<Vector3>, EdgeClass) {
        let mut fixed_points = raw_points.to_vec();
        let mut all_points_above_clearance = !fixed_points.is_empty();
        let mut all_points_below_clearance = !fixed_points.is_empty();

        for point in &fixed_points {
            let terrain_h = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
            let clearance_m = point.y - terrain_h;
            if clearance_m <= PREVIEW_CLEARANCE_M {
                all_points_above_clearance = false;
            }
            if clearance_m >= -PREVIEW_CLEARANCE_M {
                all_points_below_clearance = false;
            }
        }

        let class = if all_points_above_clearance {
            EdgeClass::Bridge
        } else if all_points_below_clearance {
            EdgeClass::Tunnel
        } else {
            EdgeClass::Standard
        };

        if class == EdgeClass::Standard {
            for point in &mut fixed_points {
                point.y = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
            }
        }

        (fixed_points, class)
    }

    /// Applies the same point simplification threshold used by committed road placement.
    pub(crate) fn simplify_road_input_points(points: &[Vector3]) -> Vec<Vector3> {
        let mut simplified_points = Vec::with_capacity(points.len());
        if !points.is_empty() {
            simplified_points.push(points[0]);
            for point in points.iter().skip(1) {
                if point.distance_to(*simplified_points.last().unwrap())
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

    /// Applies the Taubin height-smoothing pass shared by committed placement and preview.
    pub(crate) fn taubin_smooth_road_heights(points: &mut [Vector3]) {
        if points.len() <= 2 {
            return;
        }

        let mut temp_h = vec![0.0; points.len()];
        for _ in 0..TAUBIN_SMOOTHING_ITERS {
            for index in 1..points.len() - 1 {
                let laplacian = 0.5 * (points[index - 1].y + points[index + 1].y) - points[index].y;
                temp_h[index] = points[index].y + TAUBIN_LAMBDA * laplacian;
            }
            for index in 1..points.len() - 1 {
                points[index].y = temp_h[index];
            }
            for index in 1..points.len() - 1 {
                let laplacian = 0.5 * (points[index - 1].y + points[index + 1].y) - points[index].y;
                temp_h[index] = points[index].y + TAUBIN_MU * laplacian;
            }
            for index in 1..points.len() - 1 {
                points[index].y = temp_h[index];
            }
        }
    }

    /// Compiles one temporary road preview using the same point conditioning and section compiler
    /// as committed placement while keeping preview cache lifetime transient.
    pub fn compile_preview_surface(
        &self,
        raw_points: &[Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
    ) -> PreviewRoadSurfaceResult {
        let (conditioned_points, edge_class) =
            Self::classify_and_ground_road_points(raw_points, terrain);
        let mut prepared_points = Self::simplify_road_input_points(&conditioned_points);
        Self::taubin_smooth_road_heights(&mut prepared_points);

        if prepared_points.len() < 2 {
            return PreviewRoadSurfaceResult {
                edge_class,
                prepared_points,
                compiled_sections: Vec::new(),
                compiled_visual_node_pieces: Vec::new(),
                surface_vertices: Vec::new(),
                is_valid: true,
            };
        }

        let mut graph = RegionGraph::new();
        let start_node = graph.add_node(prepared_points[0], NodeType::Junction);
        let end_node = graph.add_node(*prepared_points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(Self::build_preview_edge(
            start_node,
            end_node,
            prepared_points.clone(),
            fwd_lanes,
            bkw_lanes,
            edge_class,
        ));

        let mut preview_surface = RoadSurfaceSystem::new(self.chunk_span_m);
        preview_surface.node_validation_logging_enabled = false;
        preview_surface.compile_dirty(&graph, terrain);

        let compiled_sections = preview_surface
            .compiled_sections()
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();
        let compiled_visual_node_pieces = [start_node, end_node]
            .into_iter()
            .filter_map(|node_id| {
                preview_surface
                    .compiled_visual_node_pieces()
                    .get(&node_id)
                    .cloned()
            })
            .collect();
        let surface_vertices = self.build_preview_surface_vertices(&compiled_sections);
        let is_valid = Self::preview_surface_is_valid(
            edge_class,
            &prepared_points,
            &compiled_sections,
            terrain,
        );

        PreviewRoadSurfaceResult {
            edge_class,
            prepared_points,
            compiled_sections,
            compiled_visual_node_pieces,
            surface_vertices,
            is_valid,
        }
    }

    pub(super) fn compile_edge_sections(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
    ) -> Vec<RoadSurfaceSection> {
        let edge = graph.edge(edge_idx);
        let points = self.edge_points(edge);
        if points.is_empty() {
            return Vec::new();
        }
        if points.len() == 1 {
            let center = points[0];
            let center_height_m = self.solve_section_height(center);
            let tangent_xz = Vector2::RIGHT;
            let lateral_xz = Vector2::new(-tangent_xz.y, tangent_xz.x);
            return vec![RoadSurfaceSection {
                edge_idx,
                s_m: 0.0,
                center_xz: Vector2::new(center.x, center.z),
                center_height_m,
                tangent_xz,
                lateral_xz,
                bands: self.build_lateral_bands(edge, center_height_m),
            }];
        }

        let cumulative = self.build_cumulative_distances(points);
        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let sample_distances = self.build_section_sample_distances(
            graph,
            edge_idx,
            edge,
            &cumulative,
            start_kind,
            end_kind,
        );
        sample_distances
            .into_iter()
            .map(|s_m| {
                let (center, tangent_xz) = self.sample_polyline(points, &cumulative, s_m);
                let center_height_m = self.solve_section_height(center);
                let lateral_xz = Vector2::new(-tangent_xz.y, tangent_xz.x).normalized();
                RoadSurfaceSection {
                    edge_idx,
                    s_m,
                    center_xz: Vector2::new(center.x, center.z),
                    center_height_m,
                    tangent_xz,
                    lateral_xz,
                    bands: self.build_lateral_bands(edge, center_height_m),
                }
            })
            .collect()
    }

    fn build_preview_edge(
        start_node: u32,
        end_node: u32,
        points: Vec<Vector3>,
        fwd_lanes: u8,
        bkw_lanes: u8,
        class: EdgeClass,
    ) -> Edge {
        let is_walkway = fwd_lanes == 0 && bkw_lanes == 0;
        let mut allowed_types = TransitFlags::NONE;
        if fwd_lanes > 0 || bkw_lanes > 0 {
            allowed_types |= TransitFlags::CAR;
        }
        if is_walkway || fwd_lanes > 0 || bkw_lanes > 0 {
            allowed_types |= TransitFlags::FOOT;
        }
        let vehicle_frontage_access = if is_walkway {
            VehicleFrontageAccess::SameSideOnly
        } else {
            VehicleFrontageAccess::BothSides
        };
        let physical_length = points
            .windows(2)
            .map(|segment| segment[0].distance_to(segment[1]))
            .sum();

        Edge {
            start_node,
            end_node,
            primary_type: if is_walkway {
                TransitType::Foot
            } else {
                TransitType::Road
            },
            allowed_types,
            class,
            width: ((fwd_lanes + bkw_lanes) as f32 * config::LANE_WIDTH).max(2.0),
            fwd_lanes,
            bkw_lanes,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: points.clone(),
            physical_geometry: points,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access,
        }
    }

    fn build_lateral_bands(&self, edge: &Edge, center_height_m: f32) -> Vec<RoadSurfaceBand> {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            let half_width = edge.width.max(2.0) * 0.5;
            return vec![RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Footpath,
                lateral_start_m: -half_width,
                lateral_end_m: half_width,
                height_start_m: center_height_m,
                height_end_m: center_height_m,
            }];
        }

        let half_carriageway = edge.width.max(config::LANE_WIDTH) * 0.5;
        let carriageway_height = center_height_m;
        let sidewalk_total = if edge.allowed_types & TransitFlags::FOOT != 0 {
            config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        let curb_width = if sidewalk_total > 0.0 {
            CURB_BAND_WIDTH_M.min(sidewalk_total)
        } else {
            0.0
        };
        let sidewalk_width = (sidewalk_total - curb_width).max(0.0);
        let sidewalk_height = carriageway_height
            + if curb_width > 0.0 {
                CURB_STEP_HEIGHT_M
            } else {
                0.0
            };
        let mut bands = Vec::new();
        if sidewalk_width > 0.0 {
            bands.push(RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: -(half_carriageway + curb_width + sidewalk_width),
                lateral_end_m: -(half_carriageway + curb_width),
                height_start_m: sidewalk_height,
                height_end_m: sidewalk_height,
            });
        }

        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: -(half_carriageway + curb_width),
            lateral_end_m: -half_carriageway,
            height_start_m: sidewalk_height,
            height_end_m: sidewalk_height,
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: -half_carriageway,
            lateral_end_m: 0.0,
            height_start_m: carriageway_height,
            height_end_m: carriageway_height,
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: 0.0,
            lateral_end_m: half_carriageway,
            height_start_m: carriageway_height,
            height_end_m: carriageway_height,
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: half_carriageway,
            lateral_end_m: half_carriageway + curb_width,
            height_start_m: sidewalk_height,
            height_end_m: sidewalk_height,
        });

        if sidewalk_width > 0.0 {
            bands.push(RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: half_carriageway + curb_width,
                lateral_end_m: half_carriageway + curb_width + sidewalk_width,
                height_start_m: sidewalk_height,
                height_end_m: sidewalk_height,
            });
        }

        bands
    }

    fn solve_section_height(&self, center: Vector3) -> f32 {
        center.y
    }

    fn build_section_sample_distances(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        cumulative: &[f32],
        start_kind: Option<CompiledNodeKind>,
        end_kind: Option<CompiledNodeKind>,
    ) -> Vec<f32> {
        let Some(&total_length) = cumulative.last() else {
            return vec![0.0];
        };
        if total_length <= SAMPLE_EPSILON_M {
            return vec![0.0];
        }

        let mut samples = vec![0.0, total_length];
        samples.push(edge.start_clip.clamp(0.0, total_length));
        samples.push((total_length - edge.end_clip).clamp(0.0, total_length));
        if let Some((start_throat, end_throat)) = self.visual_surface_handoff_range_for_edge(
            graph,
            edge_idx,
            edge,
            total_length,
            start_kind,
            end_kind,
        ) {
            samples.push(start_throat);
            samples.push(end_throat);
        }

        for &distance in cumulative {
            samples.push(distance);
        }

        let step_m = self.section_step_for_class(edge.class);
        for segment in cumulative.windows(2) {
            let start_s = segment[0];
            let end_s = segment[1];
            let mut sample_s = start_s + step_m;
            while sample_s < end_s - SAMPLE_EPSILON_M {
                samples.push(sample_s);
                sample_s += step_m;
            }
        }

        samples.sort_by(f32::total_cmp);
        samples.dedup_by(|a, b| (*a - *b).abs() <= SAMPLE_EPSILON_M);
        samples
    }

    pub(super) fn visual_roadbed_half_width_m(edge: &Edge) -> f32 {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            return edge.width.max(2.0) * 0.5;
        }

        let sidewalk_total = if edge.allowed_types & TransitFlags::FOOT != 0 {
            config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        edge.width.max(config::LANE_WIDTH) * 0.5 + sidewalk_total
    }

    pub(super) fn visual_carriageway_half_width_m(edge: &Edge) -> f32 {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            0.0
        } else {
            edge.width.max(config::LANE_WIDTH) * 0.5
        }
    }

    pub(super) fn visual_node_handoff_limit_m(edge: &Edge) -> f32 {
        Self::visual_roadbed_half_width_m(edge) + VISUAL_NODE_HANDOFF_PADDING_M
    }

    fn visual_terminal_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        Self::visual_node_handoff_limit_m(edge).clamp(0.0, total_length_m)
    }

    pub(super) fn visual_surface_handoff_range_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        start_kind: Option<CompiledNodeKind>,
        end_kind: Option<CompiledNodeKind>,
    ) -> Option<(f32, f32)> {
        if total_length_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let mut start_handoff = self.visual_node_handoff_distance_for_edge(
            graph,
            edge_idx,
            edge,
            total_length_m,
            start_kind,
            true,
        );
        let mut end_handoff = self.visual_node_handoff_distance_for_edge(
            graph,
            edge_idx,
            edge,
            total_length_m,
            end_kind,
            false,
        );
        let max_handoff_total = (total_length_m - VISUAL_MIN_SPAN_LENGTH_M).max(0.0);
        let handoff_total = start_handoff + end_handoff;
        if handoff_total > max_handoff_total && handoff_total > SAMPLE_EPSILON_M {
            let scale = max_handoff_total / handoff_total;
            start_handoff *= scale;
            end_handoff *= scale;
        }

        let start_s = start_handoff.clamp(0.0, total_length_m);
        let end_s = (total_length_m - end_handoff).clamp(0.0, total_length_m);
        (end_s - start_s > SAMPLE_EPSILON_M).then_some((start_s, end_s))
    }

    fn visual_node_handoff_distance_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        kind: Option<CompiledNodeKind>,
        at_start: bool,
    ) -> f32 {
        match kind {
            Some(CompiledNodeKind::Terminal) if edge.class == EdgeClass::Standard => {
                Self::visual_terminal_handoff_m(edge, total_length_m)
            }
            Some(CompiledNodeKind::Terminal) => 0.0,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN) => self
                .visual_material_conflict_handoff_m(
                    graph,
                    edge_idx,
                    edge,
                    total_length_m,
                    at_start,
                ),
            _ => 0.0,
        }
    }

    fn visual_material_conflict_handoff_m(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        at_start: bool,
    ) -> f32 {
        let side = if at_start {
            IncidentEdgeSide::Start
        } else {
            IncidentEdgeSide::End
        };
        let node_id = if at_start {
            graph.get_valid_node(edge.start_node)
        } else {
            graph.get_valid_node(edge.end_node)
        };
        let mut required_handoff = if at_start {
            Self::visual_start_handoff_m(edge, total_length_m)
        } else {
            Self::visual_end_handoff_m(edge, total_length_m)
        };

        let incidents = self.sorted_incident_surface_edges_from_graph_geometry(graph, node_id);
        let Some(current) = incidents
            .iter()
            .find(|incident| incident.edge_idx == edge_idx && incident.side == side)
        else {
            return required_handoff.clamp(0.0, total_length_m);
        };
        let roadbed_half_width_m = Self::visual_roadbed_half_width_m(edge);
        let carriageway_half_width_m = Self::visual_carriageway_half_width_m(edge);

        for other in &incidents {
            if other.edge_idx == edge_idx && other.side == side {
                continue;
            }
            let other_edge = graph.edge(other.edge_idx);
            let dot = current
                .direction_xz
                .dot(other.direction_xz)
                .clamp(-1.0, 1.0);
            if dot <= -VISUAL_CONFLICT_PASS_THROUGH_DOT_THRESHOLD {
                continue;
            }

            let sin_theta = (current.direction_xz.x * other.direction_xz.y
                - current.direction_xz.y * other.direction_xz.x)
                .abs();
            let pair_required = if sin_theta <= VISUAL_CONFLICT_SIN_EPSILON {
                total_length_m
            } else {
                let other_roadbed_half_width_m = Self::visual_roadbed_half_width_m(other_edge);
                let other_carriageway_half_width_m =
                    Self::visual_carriageway_half_width_m(other_edge);
                [
                    roadbed_half_width_m + other_roadbed_half_width_m,
                    roadbed_half_width_m + other_carriageway_half_width_m,
                    carriageway_half_width_m + other_roadbed_half_width_m,
                ]
                .into_iter()
                .map(|width_m| width_m / sin_theta)
                .fold(0.0, f32::max)
            };
            if pair_required.is_finite() {
                required_handoff = required_handoff.max(pair_required);
            }
        }

        required_handoff.clamp(0.0, total_length_m)
    }

    pub(super) fn visual_start_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        edge.start_clip
            .max(Self::visual_node_handoff_limit_m(edge))
            .clamp(0.0, total_length_m)
    }

    fn visual_end_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        edge.end_clip
            .max(Self::visual_node_handoff_limit_m(edge))
            .clamp(0.0, total_length_m)
    }

    pub(super) fn visual_end_handoff_s_m(edge: &Edge, total_length_m: f32) -> f32 {
        (total_length_m - Self::visual_end_handoff_m(edge, total_length_m))
            .clamp(0.0, total_length_m)
    }

    fn section_step_for_class(&self, class: EdgeClass) -> f32 {
        match class {
            EdgeClass::Standard => STANDARD_SECTION_STEP_M,
            EdgeClass::Bridge => BRIDGE_SECTION_STEP_M,
            EdgeClass::Tunnel => TUNNEL_SECTION_STEP_M,
        }
    }

    fn sample_polyline(
        &self,
        points: &[Vector3],
        cumulative: &[f32],
        s_m: f32,
    ) -> (Vector3, Vector2) {
        if points.len() == 1 {
            return (points[0], Vector2::RIGHT);
        }

        let total_length = cumulative.last().copied().unwrap_or(0.0);
        let clamped_s = s_m.clamp(0.0, total_length);

        for index in 0..points.len() - 1 {
            let start_s = cumulative[index];
            let end_s = cumulative[index + 1];
            if clamped_s > end_s && index + 2 < points.len() {
                continue;
            }

            let start = points[index];
            let end = points[index + 1];
            let segment_length = (end_s - start_s).max(SAMPLE_EPSILON_M);
            let local_t = ((clamped_s - start_s) / segment_length).clamp(0.0, 1.0);
            let point = start.lerp(end, local_t);
            let tangent_xz = self.segment_tangent_xz(points, index);
            return (point, tangent_xz);
        }

        (
            *points.last().unwrap(),
            self.segment_tangent_xz(points, points.len().saturating_sub(2)),
        )
    }

    fn segment_tangent_xz(&self, points: &[Vector3], preferred_index: usize) -> Vector2 {
        if points.len() < 2 {
            return Vector2::RIGHT;
        }

        let mut candidates = Vec::new();
        candidates.push(preferred_index.min(points.len() - 2));
        if preferred_index > 0 {
            candidates.push(preferred_index - 1);
        }
        if preferred_index + 1 < points.len() - 1 {
            candidates.push(preferred_index + 1);
        }

        for index in candidates {
            let delta = points[index + 1] - points[index];
            let tangent_xz = Vector2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalized();
            }
        }

        for window in points.windows(2) {
            let delta = window[1] - window[0];
            let tangent_xz = Vector2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalized();
            }
        }

        Vector2::RIGHT
    }

    fn build_cumulative_distances(&self, points: &[Vector3]) -> Vec<f32> {
        let mut cumulative = Vec::with_capacity(points.len());
        let mut running = 0.0;
        cumulative.push(0.0);
        for segment in points.windows(2) {
            running += segment[0].distance_to(segment[1]);
            cumulative.push(running);
        }
        cumulative
    }

    fn build_preview_surface_vertices(&self, sections: &[RoadSurfaceSection]) -> Vec<Vector3> {
        if sections.len() < 2 {
            return Vec::new();
        }

        let mut vertices = Vec::new();
        for pair in sections.windows(2) {
            let profile_a = self.section_profile_world_points(&pair[0], PREVIEW_MESH_LIFT_M);
            let profile_b = self.section_profile_world_points(&pair[1], PREVIEW_MESH_LIFT_M);
            if profile_a.len() < 2 || profile_a.len() != profile_b.len() {
                continue;
            }

            for index in 0..profile_a.len() - 1 {
                let a0 = profile_a[index];
                let a1 = profile_a[index + 1];
                let b0 = profile_b[index];
                let b1 = profile_b[index + 1];
                vertices.extend_from_slice(&[a0, b0, a1, a1, b0, b1]);
            }
        }

        vertices
    }

    pub(super) fn section_profile_world_points(
        &self,
        section: &RoadSurfaceSection,
        y_lift_m: f32,
    ) -> Vec<Vector3> {
        let Some(first_band) = section.bands.first() else {
            return Vec::new();
        };

        let mut points = Vec::with_capacity(section.bands.len() + 1);
        let mut first_point = self.section_boundary_world_point(
            section,
            first_band.lateral_start_m,
            first_band.height_start_m,
        );
        first_point.y += y_lift_m;
        points.push(first_point);

        for band in &section.bands {
            let mut point =
                self.section_boundary_world_point(section, band.lateral_end_m, band.height_end_m);
            point.y += y_lift_m;
            points.push(point);
        }

        points
    }

    fn preview_surface_is_valid(
        edge_class: EdgeClass,
        prepared_points: &[Vector3],
        compiled_sections: &[RoadSurfaceSection],
        terrain: &TerrainSystem,
    ) -> bool {
        for pair in compiled_sections.windows(2) {
            let run = (pair[1].s_m - pair[0].s_m).abs();
            if run <= SAMPLE_EPSILON_M {
                continue;
            }
            let grade = (pair[1].center_height_m - pair[0].center_height_m).abs() / run;
            if grade > PREVIEW_MAX_GRADE {
                return false;
            }
        }

        if prepared_points.len() > 2 {
            if let Some(mid_section) = compiled_sections.get(compiled_sections.len() / 2) {
                let terrain_h = terrain
                    .sample_height_world(mid_section.center_xz.x, mid_section.center_xz.y)
                    * config::HEIGHT_SCALE;
                match edge_class {
                    EdgeClass::Bridge => {
                        if mid_section.center_height_m < terrain_h + PREVIEW_CLEARANCE_M {
                            return false;
                        }
                    }
                    EdgeClass::Tunnel => {
                        if mid_section.center_height_m > terrain_h - PREVIEW_CLEARANCE_M {
                            return false;
                        }
                    }
                    EdgeClass::Standard => {}
                }
            }
        }

        true
    }
}
