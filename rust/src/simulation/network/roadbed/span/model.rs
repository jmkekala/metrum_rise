// SPDX-License-Identifier: GPL-2.0-only

//! The legacy section model of one edge: stations, frame, plane blend, width taper and section
//! evaluation, ported from `surface/edge/{sections,polyline,profile}.rs` with unchanged rules.

use super::super::layout::EdgeEnd;
use super::super::mesh::{RowLayout, push_section_row};
use super::super::profile::RoadProfile;
use super::super::{RoadBand, RoadSection, RoadVec2, RoadVec3};
use super::MouthSection;
use crate::config;
use crate::simulation::network::graph::rebuild::{
    JUNCTION_PROFILE_BLEND_ZONE_M, JunctionEndpointProfilePlane,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
use godot::prelude::Vector3;

// Station and direction tolerance of the section model.
const SAMPLE_EPSILON_M: f32 = 0.001;
// Two incident directions at least this opposed form a pass-through node.
pub(in crate::simulation::network::roadbed) const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
// Longitudinal section sampling cadence by road-edge class.
const STANDARD_SECTION_STEP_M: f32 = 8.0;
const BRIDGE_SECTION_STEP_M: f32 = 12.0;
const TUNNEL_SECTION_STEP_M: f32 = 10.0;
const PROFILE_TRANSITION_SECTION_STEP_M: f32 = 2.0;
const PROTECTED_SECTION_SAMPLE_CLEARANCE_M: f32 = 0.125;
const WIDTH_CHANGE_TAPER_MIN_M: f32 = 12.0;
const WIDTH_CHANGE_TAPER_MAX_M: f32 = 28.0;
const WIDTH_CHANGE_TAPER_WIDTH_FACTOR: f32 = 3.0;
const WIDTH_CHANGE_TAPER_SECTION_STEP_M: f32 = 2.0;
const WIDTH_CHANGE_TAPER_MAX_EDGE_FRACTION: f32 = 0.8;
// Endpoint directions are measured away from the shared node. Straight width changes are near -1,
// normal bends stay below this value, and overlapping same-side edges are rejected.
const WIDTH_CHANGE_CORRIDOR_DOT_MAX: f64 = 0.25;
const WIDTH_CHANGE_MIN_DELTA_M: f32 = 0.25;

/// Reusable buffers for building spans; one per Rayon worker.
#[derive(Default)]
pub(crate) struct SpanScratch {
    pub(super) points: Vec<RoadVec3>,
    pub(super) cumulative: Vec<f32>,
    pub(super) stations: Vec<f32>,
    pub(super) protected: Vec<f32>,
    pub(super) footprint: Vec<(i32, i32)>,
}

// Legacy visual node classification from incident edge directions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeKind {
    Terminal,
    PassThrough,
    Bend,
    JunctionN,
}

#[derive(Clone, Copy, Debug)]
struct EdgeWidthTaper {
    target_carriageway_half_width_m: f32,
    length_m: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct EdgeWidthTapers {
    start: Option<EdgeWidthTaper>,
    end: Option<EdgeWidthTaper>,
}

// Span framing is continuous between the exact cross-sections at its two mouths.
#[derive(Clone, Copy, Debug)]
struct SpanTangentFrame {
    start_s_m: f32,
    end_s_m: f32,
    start_tangent: RoadVec2,
    end_tangent: RoadVec2,
}

#[derive(Clone, Copy)]
struct EdgeProfilePlaneBlend {
    plane: JunctionEndpointProfilePlane,
    weight: f32,
}

impl EdgeProfilePlaneBlend {
    fn new(plane: JunctionEndpointProfilePlane, weight: f32) -> Option<Self> {
        (weight > 0.0).then_some(Self {
            plane,
            weight: weight.clamp(0.0, 1.0),
        })
    }

    fn lateral_height_at_xz(&self, center: RoadVec3, x: f32, z: f32) -> f32 {
        // Crossfall is relative to the already-solved physical centerline, not a second
        // longitudinal blend onto the junction plane.
        center.y as f32
            + self.weight
                * (self.plane.height_at_xz(x, z)
                    - self.plane.height_at_xz(center.x as f32, center.z as f32))
    }

    fn structural_height_at_xz(&self, x: f32, z: f32, height: f32) -> f32 {
        height * (1.0 - self.weight) + self.plane.height_at_xz(x, z) * self.weight
    }
}

/// The section model of one edge between its two mouth stations.
pub(super) struct SpanModel<'a> {
    edge: &'a Edge,
    edge_idx: usize,
    points: &'a [RoadVec3],
    cumulative: &'a [f32],
    total_length_m: f32,
    // Mouth stations along `points`; the span lies strictly between them.
    mouths_m: (f32, f32),
    profile_range: Option<(f32, f32)>,
    start_plane: Option<JunctionEndpointProfilePlane>,
    end_plane: Option<JunctionEndpointProfilePlane>,
    tapers: EdgeWidthTapers,
    frame: SpanTangentFrame,
    start_pass_through: Option<RoadVec2>,
    end_pass_through: Option<RoadVec2>,
    layout: RowLayout,
}

impl<'a> SpanModel<'a> {
    /// Prepares the model, or `None` when the edge has no span. Fills `points` and `cumulative`.
    ///
    /// `mouths` holds the node pieces' mouth sections at the start and end; an end without one
    /// (a pass-through node, or a node without a piece) takes its mouth at the graph clip.
    pub(super) fn new(
        graph: &'a RegionGraph,
        edge_idx: usize,
        points: &'a mut Vec<RoadVec3>,
        cumulative: &'a mut Vec<f32>,
        mouths: [Option<&MouthSection>; 2],
    ) -> Option<Self> {
        let edge = graph.get_edge(edge_idx)?;
        if !is_surface_edge(edge) {
            return None;
        }
        points.clear();
        points.extend(edge_points(edge).iter().map(|point| road_vec3(*point)));
        if points.len() < 2 {
            return None;
        }
        cumulative.clear();
        let mut running = 0.0;
        cumulative.push(0.0);
        for segment in points.windows(2) {
            running += segment[0].distance(segment[1]) as f32;
            cumulative.push(running);
        }
        let points: &'a [RoadVec3] = points;
        let cumulative: &'a [f32] = cumulative;
        let total_length_m = running;
        if total_length_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let start_mouth_m = mouths[0]
            .map_or(edge.start_clip, |mouth| mouth.section.s_m)
            .clamp(0.0, total_length_m);
        let end_mouth_m = mouths[1]
            .map_or(total_length_m - edge.end_clip, |mouth| mouth.section.s_m)
            .clamp(0.0, total_length_m);
        if end_mouth_m - start_mouth_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let (start_kind, start_pass_through) =
            endpoint_kind_and_tangent(graph, edge_idx, edge, EdgeEnd::Start);
        let (end_kind, end_pass_through) =
            endpoint_kind_and_tangent(graph, edge_idx, edge, EdgeEnd::End);
        let start_plane = endpoint_profile_plane(graph, edge, start_kind, edge.start_node);
        let end_plane = endpoint_profile_plane(graph, edge, end_kind, edge.end_node);
        let profile_range = profile_blend_range(
            start_plane.is_some(),
            end_plane.is_some(),
            (start_mouth_m, end_mouth_m),
            total_length_m,
        );
        // Node pieces own width changes at bends and junctions; the legacy taper remains only
        // where two spans meet at a pass-through node.
        let tapers = edge_width_tapers(
            graph,
            edge_idx,
            edge,
            total_length_m,
            [start_kind, end_kind].map(|kind| kind == Some(NodeKind::PassThrough)),
        );

        let mut start_tangent = sample_polyline(points, cumulative, start_mouth_m, None).1;
        let mut end_tangent = sample_polyline(points, cumulative, end_mouth_m, None).1;
        if start_mouth_m <= SAMPLE_EPSILON_M {
            start_tangent = start_pass_through.unwrap_or(start_tangent);
        }
        if total_length_m - end_mouth_m <= SAMPLE_EPSILON_M {
            end_tangent = end_pass_through.unwrap_or(end_tangent);
        }
        // A node piece's mouth fixes the frame at that end.
        if let Some(mouth) = mouths[0] {
            start_tangent = mouth.section.tangent_xz;
        }
        if let Some(mouth) = mouths[1] {
            end_tangent = mouth.section.tangent_xz;
        }
        Some(Self {
            edge,
            edge_idx,
            points,
            cumulative,
            total_length_m,
            mouths_m: (start_mouth_m, end_mouth_m),
            profile_range,
            start_plane,
            end_plane,
            tapers,
            frame: SpanTangentFrame {
                start_s_m: start_mouth_m,
                end_s_m: end_mouth_m,
                start_tangent,
                end_tangent,
            },
            start_pass_through,
            end_pass_through,
            layout: RowLayout::new(&RoadProfile::for_edge(edge)),
        })
    }

    /// Row layout shared by every section of this span.
    pub(super) fn layout(&self) -> &RowLayout {
        &self.layout
    }

    /// The mouth section at an end without a node piece (a pass-through node). Node pieces
    /// evaluate their own mouths (`node::mouth_section`), which spans copy.
    pub(super) fn mouth_section(&self, end: EdgeEnd) -> MouthSection {
        let s_m = match end {
            EdgeEnd::Start => self.mouths_m.0,
            EdgeEnd::End => self.mouths_m.1,
        };
        let section = self.section_at(s_m);
        let mut vertices = Vec::with_capacity(self.layout.stride());
        push_section_row(&section, &self.layout, &mut vertices);
        MouthSection {
            profile: self.profile_at(s_m),
            section,
            vertices: vertices.into_boxed_slice(),
        }
    }

    /// Section frame at station `s_m`: the centerline point and the unit tangent in XZ.
    ///
    /// Between the mouths the tangent interpolates centered secants at the knots and the exact
    /// mouth directions; outside them it is the one-sided segment tangent. `ROAD-68-C` offset
    /// curves must use this frame so they meet the span's band boundaries exactly.
    pub(super) fn frame_at(&self, s_m: f32) -> (RoadVec3, RoadVec2) {
        let (center, sampled_tangent_xz) =
            sample_polyline(self.points, self.cumulative, s_m, Some(&self.frame));
        let tangent_xz = if s_m <= SAMPLE_EPSILON_M {
            self.start_pass_through.unwrap_or(sampled_tangent_xz)
        } else if self.total_length_m - s_m <= SAMPLE_EPSILON_M {
            self.end_pass_through.unwrap_or(sampled_tangent_xz)
        } else {
            sampled_tangent_xz
        };
        (center, tangent_xz)
    }

    /// Evaluates the cross-section at station `s_m`.
    pub(super) fn section_at(&self, s_m: f32) -> RoadSection {
        let (center, tangent_xz) = self.frame_at(s_m);
        let lateral_xz = RoadVec2::new(-tangent_xz.y, tangent_xz.x).normalize();
        let profile_blend = self.profile_range.and_then(|range| {
            edge_profile_blend_for_section(s_m, range, self.start_plane, self.end_plane)
        });
        // The topology finalizer owns the longitudinal solution. A section must not flatten
        // that solution a second time.
        let center_height_m = if self.edge.class == EdgeClass::Standard {
            center.y as f32
        } else {
            profile_blend.map_or(center.y as f32, |blend| {
                blend.structural_height_at_xz(center.x as f32, center.z as f32, center.y as f32)
            })
        };
        RoadSection {
            edge_idx: self.edge_idx,
            s_m,
            center_xz: RoadVec2::new(center.x, center.z),
            center_height_m,
            tangent_xz,
            lateral_xz,
            bands: self.lateral_bands(s_m, center, lateral_xz, profile_blend),
        }
    }

    /// Fills `out` with the sorted stations strictly between the two mouths.
    pub(super) fn interior_stations(&self, out: &mut Vec<f32>, protected: &mut Vec<f32>) {
        self.section_sample_distances(out, protected);
        let (start_m, end_m) = self.mouths_m;
        out.retain(|&s_m| s_m > start_m + SAMPLE_EPSILON_M && s_m < end_m - SAMPLE_EPSILON_M);
    }

    fn profile_at(&self, s_m: f32) -> RoadProfile {
        RoadProfile::with_carriageway_half_width(
            self.edge,
            Some(section_carriageway_half_width(
                self.edge,
                self.total_length_m,
                s_m,
                self.tapers,
            )),
        )
    }

    fn lateral_bands(
        &self,
        s_m: f32,
        center: RoadVec3,
        lateral_xz: RoadVec2,
        profile_blend: Option<EdgeProfilePlaneBlend>,
    ) -> Vec<RoadBand> {
        let boundary_height_m = |lateral_m: f32, offset_m: f32| {
            let flat_height_m = center.y as f32;
            let base_height_m = profile_blend.map_or(flat_height_m, |blend| {
                let x = (center.x + lateral_xz.x * f64::from(lateral_m)) as f32;
                let z = (center.z + lateral_xz.y * f64::from(lateral_m)) as f32;
                if self.edge.class != EdgeClass::Standard {
                    return blend.structural_height_at_xz(x, z, flat_height_m);
                }
                blend.lateral_height_at_xz(center, x, z)
            });
            base_height_m + offset_m
        };
        self.profile_at(s_m)
            .bands()
            .iter()
            .map(|band| {
                let offset_m = band.level.offset_m();
                RoadBand {
                    kind: band.kind,
                    lateral_start_m: band.start_m,
                    lateral_end_m: band.end_m,
                    height_start_m: boundary_height_m(band.start_m, offset_m),
                    height_end_m: boundary_height_m(band.end_m, offset_m),
                }
            })
            .collect()
    }

    fn section_sample_distances(&self, samples: &mut Vec<f32>, protected: &mut Vec<f32>) {
        samples.clear();
        protected.clear();
        let edge = self.edge;
        let total_length = self.total_length_m;
        let (start_mouth_m, end_mouth_m) = self.mouths_m;
        for distance_m in [0.0, total_length, start_mouth_m, end_mouth_m] {
            push_protected_section_sample(samples, protected, distance_m, total_length);
        }

        if let Some((start_profile_s_m, end_profile_s_m)) = self.profile_range {
            push_protected_section_sample(samples, protected, start_profile_s_m, total_length);
            push_protected_section_sample(samples, protected, end_profile_s_m, total_length);
            let profile_fade_m = JUNCTION_PROFILE_BLEND_ZONE_M
                .min((end_profile_s_m - start_profile_s_m).max(0.0) * 0.5);
            if profile_fade_m > SAMPLE_EPSILON_M {
                if self.start_plane.is_some() {
                    push_cadence_samples(
                        samples,
                        protected,
                        start_profile_s_m,
                        start_profile_s_m + profile_fade_m,
                        total_length,
                        PROFILE_TRANSITION_SECTION_STEP_M,
                    );
                }
                if self.end_plane.is_some() {
                    push_cadence_samples(
                        samples,
                        protected,
                        end_profile_s_m - profile_fade_m,
                        end_profile_s_m,
                        total_length,
                        PROFILE_TRANSITION_SECTION_STEP_M,
                    );
                }
            }
        }
        if let Some(taper) = self.tapers.start {
            push_protected_section_sample(samples, protected, taper.length_m, total_length);
            push_cadence_samples(
                samples,
                protected,
                0.0,
                taper.length_m,
                total_length,
                WIDTH_CHANGE_TAPER_SECTION_STEP_M,
            );
        }
        if let Some(taper) = self.tapers.end {
            let taper_start_m = total_length - taper.length_m;
            push_protected_section_sample(samples, protected, taper_start_m, total_length);
            push_cadence_samples(
                samples,
                protected,
                taper_start_m,
                total_length,
                total_length,
                WIDTH_CHANGE_TAPER_SECTION_STEP_M,
            );
        }

        for &distance in self.cumulative {
            if !is_near_protected_section_sample(distance, protected) {
                samples.push(distance);
            }
        }

        let step_m = match edge.class {
            EdgeClass::Standard => STANDARD_SECTION_STEP_M,
            EdgeClass::Bridge => BRIDGE_SECTION_STEP_M,
            EdgeClass::Tunnel => TUNNEL_SECTION_STEP_M,
        };
        for segment in self.cumulative.windows(2) {
            let start_s = segment[0];
            let end_s = segment[1];
            let mut sample_s = start_s + step_m;
            while sample_s < end_s - SAMPLE_EPSILON_M {
                if !is_near_protected_section_sample(sample_s, protected) {
                    samples.push(sample_s);
                }
                sample_s += step_m;
            }
        }

        samples.sort_by(f32::total_cmp);
        samples.dedup_by(|a, b| (*a - *b).abs() <= SAMPLE_EPSILON_M);
    }
}

pub(in crate::simulation::network::roadbed) fn is_surface_edge(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}

// The physical profile when present, otherwise the control polyline.
pub(in crate::simulation::network::roadbed) fn edge_points(edge: &Edge) -> &[Vector3] {
    if edge.physical_geometry.is_empty() {
        &edge.geometry
    } else {
        &edge.physical_geometry
    }
}

fn road_vec3(point: Vector3) -> RoadVec3 {
    RoadVec3::new(f64::from(point.x), f64::from(point.y), f64::from(point.z))
}

fn node_at(graph: &RegionGraph, edge: &Edge, end: EdgeEnd) -> u32 {
    graph.get_valid_node(match end {
        EdgeEnd::Start => edge.start_node,
        EdgeEnd::End => edge.end_node,
    })
}

// Direction away from the node, from the endpoint to the first distinct point.
pub(in crate::simulation::network::roadbed) fn incident_direction(
    edge: &Edge,
    end: EdgeEnd,
) -> Option<RoadVec2> {
    let points = edge_points(edge);
    if points.len() < 2 {
        return None;
    }
    let direction_from = |endpoint: Vector3, point: &Vector3| {
        let direction = RoadVec2::new(
            f64::from(point.x - endpoint.x),
            f64::from(point.z - endpoint.z),
        );
        (direction.length_squared() > f64::from(SAMPLE_EPSILON_M * SAMPLE_EPSILON_M))
            .then(|| direction.normalize())
    };
    match end {
        EdgeEnd::Start => points[1..]
            .iter()
            .find_map(|point| direction_from(points[0], point)),
        EdgeEnd::End => {
            let endpoint = points[points.len() - 1];
            points[..points.len() - 1]
                .iter()
                .rev()
                .find_map(|point| direction_from(endpoint, point))
        }
    }
}

// Node kind at one end of the edge and, at a pass-through node, the bisected shared axis
// oriented along this edge's forward direction. Incident order does not matter: the axis is
// normalized and re-oriented, and negation is exact.
fn endpoint_kind_and_tangent(
    graph: &RegionGraph,
    edge_idx: usize,
    edge: &Edge,
    end: EdgeEnd,
) -> (Option<NodeKind>, Option<RoadVec2>) {
    let node_id = node_at(graph, edge, end);
    if node_id as usize >= graph.node_adjacency_count() {
        return (None, None);
    }
    let mut count = 0usize;
    let mut first_two = [RoadVec2::ZERO; 2];
    let mut current = None;
    for &incident_idx in graph.node_adjacency(node_id) {
        let Some(incident) = graph.get_edge(incident_idx) else {
            continue;
        };
        if !is_surface_edge(incident) {
            continue;
        }
        let side = if graph.get_valid_node(incident.start_node) == node_id {
            EdgeEnd::Start
        } else if graph.get_valid_node(incident.end_node) == node_id {
            EdgeEnd::End
        } else {
            continue;
        };
        let Some(direction) = incident_direction(incident, side) else {
            continue;
        };
        if count < 2 {
            first_two[count] = direction;
        }
        if current.is_none() && incident_idx == edge_idx && side == end {
            current = Some(direction);
        }
        count += 1;
    }
    let kind = match count {
        0 => return (None, None),
        1 => NodeKind::Terminal,
        2 if first_two[0].dot(first_two[1]) <= f64::from(-PASS_THROUGH_DOT_THRESHOLD) => {
            NodeKind::PassThrough
        }
        2 => NodeKind::Bend,
        _ => NodeKind::JunctionN,
    };
    if kind != NodeKind::PassThrough {
        return (Some(kind), None);
    }
    let Some(current) = current else {
        return (Some(kind), None);
    };
    let axis = first_two[0] - first_two[1];
    if axis.length_squared() <= f64::from(SAMPLE_EPSILON_M * SAMPLE_EPSILON_M) {
        return (Some(kind), None);
    }
    let mut tangent_xz = axis.normalize();
    let forward_xz = match end {
        EdgeEnd::Start => current,
        EdgeEnd::End => -current,
    };
    if tangent_xz.dot(forward_xz) < 0.0 {
        tangent_xz = -tangent_xz;
    }
    (Some(kind), Some(tangent_xz))
}

fn endpoint_profile_plane(
    graph: &RegionGraph,
    edge: &Edge,
    kind: Option<NodeKind>,
    node: u32,
) -> Option<JunctionEndpointProfilePlane> {
    if !matches!(kind, Some(NodeKind::Bend | NodeKind::JunctionN)) {
        return None;
    }
    let node = graph.get_valid_node(node);
    if edge.class == EdgeClass::Standard {
        graph.solved_junction_endpoint_profile_plane(node)
    } else {
        graph.junction_endpoint_profile_plane(node)
    }
}

// Crossfall and cadence bounds: the mouth at a profiled end, the edge end otherwise.
fn profile_blend_range(
    has_start_profile: bool,
    has_end_profile: bool,
    (start_mouth_m, end_mouth_m): (f32, f32),
    total_length_m: f32,
) -> Option<(f32, f32)> {
    if !has_start_profile && !has_end_profile {
        return None;
    }
    let start_s_m = if has_start_profile {
        start_mouth_m
    } else {
        0.0
    };
    let end_s_m = if has_end_profile {
        end_mouth_m
    } else {
        total_length_m
    };
    (end_s_m - start_s_m > SAMPLE_EPSILON_M).then_some((start_s_m, end_s_m))
}

fn edge_profile_blend_for_section(
    s_m: f32,
    profile_range: (f32, f32),
    start_profile_plane: Option<JunctionEndpointProfilePlane>,
    end_profile_plane: Option<JunctionEndpointProfilePlane>,
) -> Option<EdgeProfilePlaneBlend> {
    let (start_profile_s_m, end_profile_s_m) = profile_range;
    let span_m = (end_profile_s_m - start_profile_s_m).max(0.0);
    let fade_m = JUNCTION_PROFILE_BLEND_ZONE_M.min(span_m * 0.5);

    let start_blend = start_profile_plane.and_then(|plane| {
        let weight = if s_m <= start_profile_s_m + SAMPLE_EPSILON_M {
            1.0
        } else if fade_m > SAMPLE_EPSILON_M && s_m < start_profile_s_m + fade_m {
            let t = ((s_m - start_profile_s_m) / fade_m).clamp(0.0, 1.0);
            1.0 - smootherstep(t)
        } else {
            0.0
        };
        EdgeProfilePlaneBlend::new(plane, weight)
    });

    let end_blend = end_profile_plane.and_then(|plane| {
        let weight = if s_m >= end_profile_s_m - SAMPLE_EPSILON_M {
            1.0
        } else if fade_m > SAMPLE_EPSILON_M && s_m > end_profile_s_m - fade_m {
            let t = ((end_profile_s_m - s_m) / fade_m).clamp(0.0, 1.0);
            1.0 - smootherstep(t)
        } else {
            0.0
        };
        EdgeProfilePlaneBlend::new(plane, weight)
    });

    match (start_blend, end_blend) {
        (Some(start), Some(end)) if start.weight >= end.weight => Some(start),
        (Some(_), Some(end)) => Some(end),
        (Some(start), None) => Some(start),
        (None, Some(end)) => Some(end),
        (None, None) => None,
    }
}

fn smootherstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn edge_width_tapers(
    graph: &RegionGraph,
    edge_idx: usize,
    edge: &Edge,
    total_length_m: f32,
    [start_passes, end_passes]: [bool; 2],
) -> EdgeWidthTapers {
    if !edge_supports_width_change_taper(edge) || total_length_m <= SAMPLE_EPSILON_M {
        return EdgeWidthTapers::default();
    }
    let taper = |passes: bool, node: u32| {
        passes
            .then(|| {
                edge_endpoint_width_taper(
                    graph,
                    edge_idx,
                    edge,
                    graph.get_valid_node(node),
                    total_length_m,
                )
            })
            .flatten()
    };
    EdgeWidthTapers {
        start: taper(start_passes, edge.start_node),
        end: taper(end_passes, edge.end_node),
    }
}

fn edge_endpoint_width_taper(
    graph: &RegionGraph,
    edge_idx: usize,
    edge: &Edge,
    node_id: u32,
    total_length_m: f32,
) -> Option<EdgeWidthTaper> {
    let current_direction = edge_endpoint_direction_away_from_node(graph, edge, node_id)?;
    let mut road_incident_count = 0usize;
    let mut other_edge_idx = None;
    for &incident_edge_idx in graph.node_adjacency(node_id) {
        if incident_edge_idx >= graph.edge_count() {
            continue;
        }
        let incident_edge = graph.edge(incident_edge_idx);
        if !edge_supports_width_change_taper(incident_edge)
            || !edge_is_incident_to_node(graph, incident_edge, node_id)
        {
            continue;
        }
        road_incident_count += 1;
        if incident_edge_idx != edge_idx {
            other_edge_idx = Some(incident_edge_idx);
        }
    }
    if road_incident_count != 2 {
        return None;
    }

    let other_edge = graph.edge(other_edge_idx?);
    let other_direction = edge_endpoint_direction_away_from_node(graph, other_edge, node_id)?;
    if current_direction.dot(other_direction) > WIDTH_CHANGE_CORRIDOR_DOT_MAX {
        return None;
    }

    let full_half_width_m = base_carriageway_half_width(edge);
    let target_half_width_m = base_carriageway_half_width(other_edge);
    if full_half_width_m <= target_half_width_m + WIDTH_CHANGE_MIN_DELTA_M * 0.5 {
        return None;
    }

    let width_delta_m = (full_half_width_m - target_half_width_m) * 2.0;
    let requested_length_m = (width_delta_m * WIDTH_CHANGE_TAPER_WIDTH_FACTOR)
        .clamp(WIDTH_CHANGE_TAPER_MIN_M, WIDTH_CHANGE_TAPER_MAX_M);
    let max_length_m = total_length_m * WIDTH_CHANGE_TAPER_MAX_EDGE_FRACTION;
    let length_m = requested_length_m.min(max_length_m);
    (length_m > WIDTH_CHANGE_TAPER_SECTION_STEP_M).then_some(EdgeWidthTaper {
        target_carriageway_half_width_m: target_half_width_m,
        length_m,
    })
}

fn edge_supports_width_change_taper(edge: &Edge) -> bool {
    !edge.deleted
        && edge.class == EdgeClass::Standard
        && edge.primary_type == TransitType::Road
        && edge.allowed_types & TransitFlags::CAR != 0
}

fn edge_is_incident_to_node(graph: &RegionGraph, edge: &Edge, node_id: u32) -> bool {
    graph.get_valid_node(edge.start_node) == node_id
        || graph.get_valid_node(edge.end_node) == node_id
}

fn edge_endpoint_direction_away_from_node(
    graph: &RegionGraph,
    edge: &Edge,
    node_id: u32,
) -> Option<RoadVec2> {
    let points = edge_points(edge);
    if points.len() < 2 {
        return None;
    }
    let (from, to) = if graph.get_valid_node(edge.start_node) == node_id {
        (points[0], points[1])
    } else if graph.get_valid_node(edge.end_node) == node_id {
        let last_idx = points.len() - 1;
        (points[last_idx], points[last_idx - 1])
    } else {
        return None;
    };
    let delta_x = f64::from(to.x - from.x);
    let delta_z = f64::from(to.z - from.z);
    let length = (delta_x * delta_x + delta_z * delta_z).sqrt();
    (length > f64::from(SAMPLE_EPSILON_M))
        .then(|| RoadVec2::new(delta_x / length, delta_z / length))
}

fn section_carriageway_half_width(
    edge: &Edge,
    total_length_m: f32,
    s_m: f32,
    width_tapers: EdgeWidthTapers,
) -> f32 {
    let base_half_width_m = base_carriageway_half_width(edge);
    let mut half_width_m = base_half_width_m;
    if let Some(taper) = width_tapers.start {
        half_width_m = half_width_m.min(tapered_carriageway_half_width(
            base_half_width_m,
            taper,
            s_m,
        ));
    }
    if let Some(taper) = width_tapers.end {
        half_width_m = half_width_m.min(tapered_carriageway_half_width(
            base_half_width_m,
            taper,
            total_length_m - s_m,
        ));
    }
    half_width_m.max(config::LANE_WIDTH * 0.5)
}

fn tapered_carriageway_half_width(
    base_half_width_m: f32,
    taper: EdgeWidthTaper,
    distance_from_node_m: f32,
) -> f32 {
    let blend = 1.0 - smootherstep((distance_from_node_m / taper.length_m).clamp(0.0, 1.0));
    base_half_width_m + (taper.target_carriageway_half_width_m - base_half_width_m) * blend
}

fn base_carriageway_half_width(edge: &Edge) -> f32 {
    edge.width.max(config::LANE_WIDTH) * 0.5
}

// Samples every `step_m` strictly inside `(start_m, end_m)`, skipping the protected clearance.
fn push_cadence_samples(
    samples: &mut Vec<f32>,
    protected: &[f32],
    start_m: f32,
    end_m: f32,
    total_length_m: f32,
    step_m: f32,
) {
    let start_m = start_m.clamp(0.0, total_length_m);
    let end_m = end_m.clamp(0.0, total_length_m);
    if end_m <= start_m + step_m {
        return;
    }
    let mut sample_m = start_m + step_m;
    while sample_m < end_m - SAMPLE_EPSILON_M {
        if !is_near_protected_section_sample(sample_m, protected) {
            samples.push(sample_m);
        }
        sample_m += step_m;
    }
}

fn push_protected_section_sample(
    samples: &mut Vec<f32>,
    protected: &mut Vec<f32>,
    distance_m: f32,
    total_length_m: f32,
) {
    let clamped = distance_m.clamp(0.0, total_length_m);
    samples.push(clamped);
    protected.push(clamped);
}

fn is_near_protected_section_sample(distance_m: f32, protected: &[f32]) -> bool {
    protected.iter().any(|&protected_m| {
        let delta_m = (distance_m - protected_m).abs();
        delta_m > SAMPLE_EPSILON_M && delta_m <= PROTECTED_SECTION_SAMPLE_CLEARANCE_M
    })
}

fn sample_polyline(
    points: &[RoadVec3],
    cumulative: &[f32],
    s_m: f32,
    span_frame: Option<&SpanTangentFrame>,
) -> (RoadVec3, RoadVec2) {
    let total_length = cumulative.last().copied().unwrap_or(0.0);
    let clamped_s = s_m.clamp(0.0, total_length);
    // The first segment ending at or after the station, else the last one.
    let index = (0..points.len() - 1)
        .find(|&index| clamped_s <= cumulative[index + 1])
        .unwrap_or(points.len() - 2);
    let start_s = cumulative[index];
    let end_s = cumulative[index + 1];
    let segment_length = (end_s - start_s).max(SAMPLE_EPSILON_M);
    let local_t = f64::from(((clamped_s - start_s) / segment_length).clamp(0.0, 1.0));
    let point = points[index].lerp(points[index + 1], local_t);
    let Some(frame) =
        span_frame.filter(|frame| clamped_s > frame.start_s_m && clamped_s < frame.end_s_m)
    else {
        return (point, segment_tangent_xz(points, index));
    };
    // Mouths keep their exact directions. Within the span, interpolate centered secants at
    // every original or inserted section: a one-sided segment tangent can fold offset bands
    // across a centimetre interval.
    let (frame_start_s, start_tangent) = if start_s <= frame.start_s_m {
        (frame.start_s_m, frame.start_tangent)
    } else {
        (start_s, vertex_tangent_xz(points, index))
    };
    let (frame_end_s, end_tangent) = if end_s >= frame.end_s_m {
        (frame.end_s_m, frame.end_tangent)
    } else {
        (end_s, vertex_tangent_xz(points, index + 1))
    };
    let frame_t = f64::from(
        (clamped_s - frame_start_s) / (frame_end_s - frame_start_s).max(SAMPLE_EPSILON_M),
    );
    let tangent = start_tangent.lerp(end_tangent, frame_t);
    let tangent_xz = if tangent.length_squared() > 1e-8 {
        tangent.normalize()
    } else {
        segment_tangent_xz(points, index)
    };
    (point, tangent_xz)
}

fn vertex_tangent_xz(points: &[RoadVec3], index: usize) -> RoadVec2 {
    if index == 0 {
        return segment_tangent_xz(points, 0);
    }
    if index + 1 >= points.len() {
        return segment_tangent_xz(points, index - 1);
    }
    // Preserve segment lengths before normalization: a centimetre profile interval must not
    // amplify float-coordinate roundoff as much as its metre-long neighbours.
    let delta = points[index + 1] - points[index - 1];
    let tangent = RoadVec2::new(delta.x, delta.z);
    if tangent.length_squared() > 1e-8 {
        tangent.normalize()
    } else {
        segment_tangent_xz(points, index)
    }
}

fn segment_tangent_xz(points: &[RoadVec3], preferred_index: usize) -> RoadVec2 {
    let candidates = [
        Some(preferred_index.min(points.len() - 2)),
        preferred_index.checked_sub(1),
        (preferred_index + 1 < points.len() - 1).then_some(preferred_index + 1),
    ];
    for index in candidates.into_iter().flatten() {
        let delta = points[index + 1] - points[index];
        let tangent_xz = RoadVec2::new(delta.x, delta.z);
        if tangent_xz.length_squared() > 1e-8 {
            return tangent_xz.normalize();
        }
    }
    for window in points.windows(2) {
        let delta = window[1] - window[0];
        let tangent_xz = RoadVec2::new(delta.x, delta.z);
        if tangent_xz.length_squared() > 1e-8 {
            return tangent_xz.normalize();
        }
    }
    RoadVec2::X
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;

    /// Samples the polyline the way sections do, for the frame regression.
    pub(in crate::simulation::network::roadbed) fn sample_with_mouth_frame(
        points: &[RoadVec3],
        s_m: f32,
    ) -> (RoadVec3, RoadVec2) {
        let mut cumulative = vec![0.0];
        let mut running = 0.0;
        for segment in points.windows(2) {
            running += segment[0].distance(segment[1]) as f32;
            cumulative.push(running);
        }
        let frame = SpanTangentFrame {
            start_s_m: 0.0,
            end_s_m: running,
            start_tangent: sample_polyline(points, &cumulative, 0.0, None).1,
            end_tangent: sample_polyline(points, &cumulative, running, None).1,
        };
        sample_polyline(points, &cumulative, s_m, Some(&frame))
    }

    /// Cumulative station of each knot.
    pub(in crate::simulation::network::roadbed) fn knot_stations(points: &[RoadVec3]) -> Vec<f32> {
        let mut cumulative = vec![0.0];
        let mut running = 0.0;
        for segment in points.windows(2) {
            running += segment[0].distance(segment[1]) as f32;
            cumulative.push(running);
        }
        cumulative
    }
}
