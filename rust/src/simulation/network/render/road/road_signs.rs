// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic cosmetic crossing, changed-limit and geometry warning signs.
//!
//! Only degree-two streets touching coverage are visited. Each chain is measured with the road
//! validator and spaced in three priority passes. Work is O(E + P W + C S + L + N),
//! for touched edges E, physical points P, radius-window segments W, crossings C, edge sections S,
//! light poles L and signs N. Pole and sign queries read bounded local buckets; final pose order
//! uses fixed-width radix sorting. Topology-dependent rebuilds are sequential, outside tick/agent
//! loops. Reusable scratch is allocated per rebuild, never per sign or measurement sample.

use super::NetworkMeshData;
use super::standard_surface::CompiledSurfaceCoverage;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::lanes::{CrosswalkMarking, LaneSystem};
use crate::simulation::network::roadbed::{
    BandKind, RoadSection, RoadVec2, RoadbedSystem, visual_roadbed_half_width_m,
};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::network::validate::{
    RADIUS_WINDOW_HALF_WIDTHS, centreline, design_radius, visit_grades, visit_turning_radii,
};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;
use std::collections::HashMap;

/// Sign models in the renderer's model order; priority signs are deliberately absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RoadSignKind {
    /// E1-1, on the right of approaching traffic.
    CrossingRight,
    /// E1-2, on the left of approaching traffic on a one-way road.
    CrossingLeft,
    /// C32-30.
    Speed30,
    /// C32-40.
    Speed40,
    /// C32-50.
    Speed50,
    /// C32-60.
    Speed60,
    /// C32-80.
    Speed80,
    /// A1.1, curve to the right.
    CurveRight,
    /// A1.2, curve to the left.
    CurveLeft,
    /// A2.1, series of curves starting right.
    CurvesRight,
    /// A2.2, series of curves starting left.
    CurvesLeft,
    /// A3.2-8, 8% descent.
    Descent8,
    /// A3.2-10, 10% descent.
    Descent10,
    /// A3.2-12, 12% descent.
    Descent12,
    /// A3.2-14, 14% descent.
    Descent14,
    /// A3.2-16, 16% descent.
    Descent16,
    /// A3.2-18, 18% descent.
    Descent18,
    /// A3.1-8, 8% ascent.
    Ascent8,
    /// A3.1-10, 10% ascent.
    Ascent10,
    /// A3.1-12, 12% ascent.
    Ascent12,
    /// A3.1-14, 14% ascent.
    Ascent14,
    /// A3.1-16, 16% ascent.
    Ascent16,
    /// A3.1-18, 18% ascent.
    Ascent18,
    /// A9, uneven road.
    Uneven,
}

/// Pole-base pose, with yaw rotating the model's +Z front toward approaching traffic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadSign {
    /// Pole base in the owning mesh's coordinate system.
    pub(crate) position: Vector3,
    /// Rotation about +Y, in radians.
    pub(crate) yaw: f32,
    /// Index of the GLB used by the thin renderer.
    pub(crate) kind: RoadSignKind,
}

const CLEARANCE_M: f32 = 2.0;
const SIGN_GAP_M: f32 = 12.0;
// A crossing sign stands just before the stripes for its traffic, out of the walking line: half
// the 2.5 m stripe length plus 0.5 m back from the crossing centre.
const CROSSING_SIGN_SETBACK_M: f32 = 1.75;
// A speed sign stands this far past the crossing sign of its junction mouth, never beside it.
const SPEED_AFTER_CROSSING_M: f32 = 25.0;
// Finnish practice repeats the limit after a junction on longer streets, so a driver who turned
// in sees it; short side streets inherit the area limit without a sign. Wider roads, which
// drivers join from side streets, repeat it on shorter stretches.
const SPEED_REPEAT_STREET_M: f32 = 300.0;
const SPEED_REPEAT_WIDE_STREET_M: f32 = 120.0;
// Save survey: 0.035 warned on ~13% of city roads; 0.07 keeps A9 exceptional.
const UNEVEN_GRADE_CHANGE_PER_M: f32 = 0.07;
const WARNING_LEAD_M: f32 = 20.0;
const SIGN_KERB_OFFSET_M: f32 = 0.5;
// A junction already requires slowing: its mouth and the following 30 m are not surprises.
const CURVE_JUNCTION_CLEARANCE_M: f32 = 30.0;
const CURVE_DESIGN_RADIUS_FRACTION: f32 = 0.25;
const CURVE_MIN_TURN_DEG: f32 = 60.0;
// Steep-hill signs go where the road averages the validator's illegal grade (16 %) over 100 m: a
// hill steeper than placement should allow, not a short ramp at a junction. Save survey: at 10 %
// over 50 m Kuopio got 62 signs on 106 roads, which read as spam on streets that look tame; at 16 %
// over 100 m it gets 9 and the steep hill in traffic-signs.sqlite keeps its pair.
const STEEP_WINDOW_M: f32 = 100.0;
const STEEP_MIN_GRADE: f32 = 0.16;
// Millimetre roundoff guard after an analytically solved pole-clearance boundary.
const CLEARANCE_ROUNDOFF_M: f64 = 0.001;
const NO_LINK: usize = usize::MAX;

fn road_family(edge: &Edge) -> Option<(EdgeClass, bool)> {
    (!edge.deleted && edge.primary_type == TransitType::Road).then_some((
        edge.class,
        u16::from(edge.fwd_lanes) + u16::from(edge.bkw_lanes) > 2,
    ))
}

fn degree(graph: &RegionGraph, node: u32, degrees: &mut HashMap<u32, usize>) -> usize {
    *degrees
        .entry(node)
        .or_insert_with(|| graph.live_node_connection_count(node))
}

struct StreetSigns {
    lengths: HashMap<usize, f32>,
    warnings: HashMap<usize, Warnings>,
    stations: HashMap<usize, ChainSpan>,
    edges: Vec<usize>,
}

fn street_lengths(
    graph: &RegionGraph,
    covered: &[usize],
    degrees: &mut HashMap<u32, usize>,
) -> StreetSigns {
    // Only streets touching the rebuild are traversed. A local chunk must not scan the city.
    let mut lengths = HashMap::with_capacity(covered.len());
    let mut chain = Vec::with_capacity(covered.len());
    let mut warnings = HashMap::with_capacity(covered.len());
    let mut stations = HashMap::with_capacity(covered.len());
    let mut edges = Vec::with_capacity(covered.len());
    let mut points = Vec::new();
    let mut spans = Vec::new();
    let mut bends = Vec::with_capacity(2);
    let mut warning_chain = Vec::with_capacity(covered.len());
    let mut warning_visited = std::collections::HashSet::with_capacity(covered.len());
    for &edge_id in covered {
        let Some(family) = road_family(graph.edge(edge_id)) else {
            continue;
        };
        if lengths.contains_key(&edge_id) {
            continue;
        }
        chain.clear();
        chain.push(edge_id);
        lengths.insert(edge_id, 0.0);
        let edge = graph.edge(edge_id);
        for mut node in [edge.start_node, edge.end_node] {
            let mut previous = edge_id;
            while degree(graph, node, degrees) == 2 {
                let Some(&next) = graph
                    .node_adjacency(node)
                    .iter()
                    .find(|&&id| id != previous && !graph.edge(id).deleted)
                else {
                    break;
                };
                if lengths.contains_key(&next) || road_family(graph.edge(next)) != Some(family) {
                    break;
                }
                lengths.insert(next, 0.0);
                chain.push(next);
                let edge = graph.edge(next);
                node = if edge.start_node == node {
                    edge.end_node
                } else {
                    edge.start_node
                };
                previous = next;
            }
        }
        let length = chain
            .iter()
            .map(|&id| f64::from(graph.edge(id).physical_length))
            .sum::<f64>() as f32;
        for &id in &chain {
            lengths.insert(id, length);
        }
        // Warning chains additionally require matching physical endpoints and directed lane
        // availability. A driver cannot approach a hazard through a disconnected alignment.
        for &seed in &chain {
            if !warning_visited.insert(seed) {
                continue;
            }
            warning_chain.clear();
            warning_chain.push(seed);
            let edge = graph.edge(seed);
            for mut node in [edge.start_node, edge.end_node] {
                let mut previous = seed;
                while let Some(next) = continuation(graph, node, previous, degrees) {
                    if !warning_visited.insert(next) {
                        break;
                    }
                    warning_chain.push(next);
                    let edge = graph.edge(next);
                    node = if edge.start_node == node {
                        edge.end_node
                    } else {
                        edge.start_node
                    };
                    previous = next;
                }
            }
            chain_warnings(
                graph,
                &warning_chain,
                degrees,
                &mut points,
                &mut spans,
                &mut bends,
                &mut warnings,
                &mut stations,
                &mut edges,
            );
        }
    }
    StreetSigns {
        lengths,
        warnings,
        stations,
        edges,
    }
}

// Each edge has three warning families for each stored travel direction.
type Warnings = [Option<(f32, RoadSignKind)>; 6];

#[derive(Clone, Copy)]
struct ChainSpan {
    chain_id: usize,
    edge_id: usize,
    start: f32,
    end: f32,
    forward: bool,
    loop_length: Option<f32>,
}

#[derive(Clone, Copy)]
struct Bend {
    start: f32,
    end: f32,
    turn: f32,
    sharp: bool,
}

fn continuation(
    graph: &RegionGraph,
    node: u32,
    previous: usize,
    degrees: &mut HashMap<u32, usize>,
) -> Option<usize> {
    if degree(graph, node, degrees) != 2 {
        return None;
    }
    let previous_edge = graph.edge(previous);
    let at_end = previous_edge.end_node == node;
    let endpoint = if at_end {
        centreline(previous_edge).last()
    } else {
        centreline(previous_edge).first()
    };
    let incoming = if at_end {
        previous_edge.fwd_lanes
    } else {
        previous_edge.bkw_lanes
    } > 0;
    let outgoing = if at_end {
        previous_edge.bkw_lanes
    } else {
        previous_edge.fwd_lanes
    } > 0;
    graph.node_adjacency(node).iter().copied().find(|&id| {
        let edge = graph.edge(id);
        let at_start = edge.start_node == node;
        let next_endpoint = if at_start {
            centreline(edge).first()
        } else {
            centreline(edge).last()
        };
        let next_outgoing = if at_start {
            edge.fwd_lanes
        } else {
            edge.bkw_lanes
        } > 0;
        let next_incoming = if at_start {
            edge.bkw_lanes
        } else {
            edge.fwd_lanes
        } > 0;
        id != previous
            && road_family(edge) == road_family(previous_edge)
            && endpoint.is_some()
            && endpoint == next_endpoint
            && incoming == next_outgoing
            && outgoing == next_incoming
    })
}

fn span_at(spans: &[ChainSpan], station: f32) -> &ChainSpan {
    &spans[spans
        .partition_point(|span| span.end < station)
        .min(spans.len() - 1)]
}

fn measurement_span<'a>(spans: &'a [ChainSpan], station: f32, index: &mut usize) -> &'a ChainSpan {
    while *index + 1 < spans.len() && spans[*index].end < station {
        *index += 1;
    }
    while *index > 0 && spans[*index].start > station {
        *index -= 1;
    }
    &spans[*index]
}

// A chain joins physical node endpoints, but the clipped portions belong to node fillets/mouths.
// Visit only spans intersecting this measurement window: O(window segments), no allocation.
fn travelled_interval(
    graph: &RegionGraph,
    spans: &[ChainSpan],
    start: f32,
    end: f32,
    index: &mut usize,
) -> bool {
    measurement_span(spans, start, index);
    for span in &spans[*index..] {
        if span.start > end {
            break;
        }
        let edge = graph.edge(span.edge_id);
        let (start_clip, end_clip) = if span.forward {
            (edge.start_clip, edge.end_clip)
        } else {
            (edge.end_clip, edge.start_clip)
        };
        if start.max(span.start) < span.start + start_clip
            || end.min(span.end) > span.end - end_clip
        {
            return false;
        }
    }
    true
}

fn steep_kind(grade: f32) -> RoadSignKind {
    let kinds = if grade > 0.0 {
        [
            RoadSignKind::Ascent8,
            RoadSignKind::Ascent10,
            RoadSignKind::Ascent12,
            RoadSignKind::Ascent14,
            RoadSignKind::Ascent16,
            RoadSignKind::Ascent18,
        ]
    } else {
        [
            RoadSignKind::Descent8,
            RoadSignKind::Descent10,
            RoadSignKind::Descent12,
            RoadSignKind::Descent14,
            RoadSignKind::Descent16,
            RoadSignKind::Descent18,
        ]
    };
    // One ULP guards an exact even percentage represented just below its boundary.
    kinds[(((grade.abs() * 100.0).next_up() / 2.0).floor() as usize).clamp(4, 9) - 4]
}

// Earliest hazard of each family in travel order, with whole-stretch steepest grade.
// Shared validator callbacks do all grade/change/radius measurement; no point-loop allocation.
fn hazards(
    graph: &RegionGraph,
    points: &[Vector3],
    spans: &[ChainSpan],
    reverse: bool,
    bends: &mut Vec<Bend>,
) -> [Option<(f32, RoadSignKind)>; 3] {
    let length = spans.last().unwrap().end;
    let canonical = |s| if reverse { length - s } else { s };
    let mut result = [None; 3];
    // Steep: the mean grade over `STEEP_WINDOW_M` of horizontal run, not the steepest segment. A
    // junction tie-in or a terrain bump is steep for a few metres on an otherwise gentle street;
    // signing those put an 18 % sign on roads that read as flat. Two pointers over the polyline,
    // O(points), no allocation. The sign leads the first qualifying window; its label is the
    // steepest window of that stretch.
    let mut stretch: Option<(f32, f32)> = None;
    let (mut tail, mut run, mut tail_station) = (0, 0.0_f32, 0.0_f32);
    let horizontal = |a: Vector3, b: Vector3| (b.x - a.x).hypot(b.z - a.z);
    for head in 1..points.len() {
        if result[1].is_some() {
            break;
        }
        run += horizontal(points[head - 1], points[head]);
        while tail + 1 < head && run - horizontal(points[tail], points[tail + 1]) >= STEEP_WINDOW_M
        {
            run -= horizontal(points[tail], points[tail + 1]);
            tail_station += points[tail].distance_to(points[tail + 1]);
            tail += 1;
        }
        let grade = if run >= STEEP_WINDOW_M {
            (points[head].y - points[tail].y) / run
        } else {
            0.0
        };
        match stretch {
            Some((start, peak))
                if grade.abs() >= STEEP_MIN_GRADE && grade.signum() == peak.signum() =>
            {
                stretch = Some((
                    start,
                    if grade.abs() > peak.abs() {
                        grade
                    } else {
                        peak
                    },
                ));
            }
            Some((start, peak)) => {
                result[1] = Some((start, steep_kind(peak)));
                stretch = None;
            }
            None if grade.abs() >= STEEP_MIN_GRADE => stretch = Some((tail_station, grade)),
            None => {}
        }
    }
    if let Some((start, peak)) = stretch
        && result[1].is_none()
    {
        result[1] = Some((start, steep_kind(peak)));
    }
    visit_grades(points, |station, _, _, change| {
        if change > UNEVEN_GRADE_CHANGE_PER_M && result[2].is_none() {
            result[2] = Some((station, RoadSignKind::Uneven));
        }
    });

    // Open chains exclude bends within 30 m of either junction, in both travel directions.
    // Every radius window also excludes clipped node fillets, including internal degree-two
    // joins. Work remains O(points * window segments), using the validator's measurement.
    let endpoint = |span: &ChainSpan, start: bool| {
        let edge = graph.edge(span.edge_id);
        let at_start = span.forward == start;
        let (node, clip) = if at_start {
            (edge.start_node, edge.start_clip)
        } else {
            (edge.end_node, edge.end_clip)
        };
        let clearance = if span.loop_length.is_none() && graph.live_node_connection_count(node) >= 3
        {
            CURVE_JUNCTION_CLEARANCE_M
        } else {
            0.0
        };
        (clip, clearance)
    };
    let (start_clip, start_clearance) = endpoint(&spans[0], true);
    let (end_clip, end_clearance) = endpoint(spans.last().unwrap(), false);
    let (bend_start, bend_end) = if reverse {
        (
            end_clip + end_clearance,
            length - start_clip - start_clearance,
        )
    } else {
        (
            start_clip + start_clearance,
            length - end_clip - end_clearance,
        )
    };
    bends.clear();
    let mut bend: Option<Bend> = None;
    let finish_bend = |bend: Option<Bend>, bends: &mut Vec<Bend>| {
        if let Some(bend) = bend
            && bends.len() < 2
            && bend.sharp
            && bend.turn.abs().to_degrees() >= CURVE_MIN_TURN_DEG - 0.001
            && bend.start >= bend_start
            && bend.end <= bend_end
        {
            bends.push(bend);
        }
    };
    let mut window_index = if reverse { spans.len() - 1 } else { 0 };
    let mut radius_index = window_index;
    let mut interval_index = window_index;
    visit_turning_radii(
        points,
        |station| {
            let edge =
                graph.edge(measurement_span(spans, canonical(station), &mut window_index).edge_id);
            RADIUS_WINDOW_HALF_WIDTHS * visual_roadbed_half_width_m(edge)
        },
        |station, radius, turn, measured| {
            let edge =
                graph.edge(measurement_span(spans, canonical(station), &mut radius_index).edge_id);
            // Never accept a radius from a truncated window, or one looking into a mouth.
            let sharp = radius
                < CURVE_DESIGN_RADIUS_FRACTION * design_radius(edge.speed_limit, 0.20)
                && measured.is_some_and(|(start, end)| {
                    let (start, end) = if reverse {
                        (canonical(end), canonical(start))
                    } else {
                        (start, end)
                    };
                    travelled_interval(graph, spans, start, end, &mut interval_index)
                });
            if turn.abs() > 0.000001
                && travelled_interval(
                    graph,
                    spans,
                    canonical(station),
                    canonical(station),
                    &mut radius_index,
                )
            {
                if let Some(mut current) = bend
                    && current.turn.signum() == turn.signum()
                {
                    current.end = station;
                    current.turn += turn;
                    current.sharp |= sharp;
                    bend = Some(current);
                } else {
                    finish_bend(bend, bends);
                    bend = Some(Bend {
                        start: station,
                        end: station,
                        turn,
                        sharp,
                    });
                }
            } else {
                finish_bend(bend, bends);
                bend = None;
            }
        },
    );
    finish_bend(bend, bends);
    if let Some(first) = bends.first() {
        let series = bends.get(1).is_some_and(|next| {
            next.turn.signum() != first.turn.signum() && next.start - first.end <= 100.0
        });
        let kind = match (series, first.turn > 0.0) {
            (false, true) => RoadSignKind::CurveRight,
            (false, false) => RoadSignKind::CurveLeft,
            (true, true) => RoadSignKind::CurvesRight,
            (true, false) => RoadSignKind::CurvesLeft,
        };
        result[0] = Some((first.start, kind));
    }
    result
}

// Same degree-two/light-family chain as street_lengths: one warning per family/direction for
// the entire chain. Canonical endpoint order (minimum node/edge; minimum edge on a loop) makes
// results independent of chunk coverage. Lead distance may put the pole on the preceding edge.
#[allow(clippy::too_many_arguments)]
fn chain_warnings(
    graph: &RegionGraph,
    chain: &[usize],
    degrees: &mut HashMap<u32, usize>,
    points: &mut Vec<Vector3>,
    spans: &mut Vec<ChainSpan>,
    bends: &mut Vec<Bend>,
    warnings: &mut HashMap<usize, Warnings>,
    stations: &mut HashMap<usize, ChainSpan>,
    edges: &mut Vec<usize>,
) {
    let chain_id = *chain.iter().min().unwrap();
    let mut endpoint = None;
    for &id in chain {
        let edge = graph.edge(id);
        for node in [edge.start_node, edge.end_node] {
            if continuation(graph, node, id, degrees).is_none() {
                endpoint =
                    Some(endpoint.map_or((node, id), |old: (u32, usize)| old.min((node, id))));
            }
        }
    }
    let (mut node, mut id) = endpoint.unwrap_or_else(|| {
        let id = *chain.iter().min().unwrap();
        (graph.edge(id).start_node, id)
    });
    points.clear();
    spans.clear();
    let point_count: usize = chain
        .iter()
        .map(|&id| centreline(graph.edge(id)).len())
        .sum();
    points.reserve(point_count);
    spans.reserve(chain.len());
    let mut length = 0.0;
    for _ in 0..chain.len() {
        let edge = graph.edge(id);
        let forward = edge.start_node == node;
        let geometry = centreline(edge);
        let start = length;
        for i in 0..geometry.len() {
            let point = geometry[if forward { i } else { geometry.len() - 1 - i }];
            if i == 0 && !points.is_empty() {
                // Continuation requires this exact shared physical endpoint.
                continue;
            }
            if let Some(&previous) = points.last() {
                length += previous.distance_to(point);
            }
            points.push(point);
        }
        let span = ChainSpan {
            chain_id,
            edge_id: id,
            start,
            end: length,
            forward,
            loop_length: None,
        };
        spans.push(span);
        node = if forward {
            edge.end_node
        } else {
            edge.start_node
        };
        let Some(next) = continuation(graph, node, id, degrees) else {
            break;
        };
        id = next;
    }
    for span in spans.iter_mut() {
        if endpoint.is_none() && length > 0.0 {
            span.loop_length = Some(length);
        }
        stations.insert(span.edge_id, *span);
        edges.push(span.edge_id);
    }
    if points.len() < 2 {
        return;
    }
    for reverse in [false, true] {
        if reverse {
            points.reverse();
        }
        for (family, hazard) in hazards(graph, points, spans, reverse, bends)
            .into_iter()
            .enumerate()
        {
            let Some((station, kind)) = hazard else {
                continue;
            };
            let lead = (station - WARNING_LEAD_M).max(0.0);
            let canonical = if reverse { length - lead } else { lead };
            let span = span_at(spans, canonical);
            let forward = span.forward != reverse;
            let edge = graph.edge(span.edge_id);
            if (if forward {
                edge.fwd_lanes
            } else {
                edge.bkw_lanes
            }) == 0
            {
                continue;
            }
            let s = if span.forward {
                canonical - span.start
            } else {
                span.end - canonical
            };
            warnings.entry(span.edge_id).or_insert([None; 6])[usize::from(!forward) * 3 + family] =
                Some((s, kind));
        }
    }
}

fn speed_kind(speed_ms: f32) -> Option<RoadSignKind> {
    if !speed_ms.is_finite() {
        return None;
    }
    let speed = speed_ms * 3.6;
    let choices = [
        (30.0, RoadSignKind::Speed30),
        (40.0, RoadSignKind::Speed40),
        (50.0, RoadSignKind::Speed50),
        (60.0, RoadSignKind::Speed60),
        (80.0, RoadSignKind::Speed80),
    ];
    // Lower value wins an exact tie.
    let &(limit, kind) = choices
        .iter()
        .min_by(|a, b| (speed - a.0).abs().total_cmp(&(speed - b.0).abs()))?;
    ((speed - limit).abs() <= 5.0).then_some(kind)
}

// Transient sparse version of the existing node-grid bucket pattern. The road R-tree indexes
// road envelopes, not pole bases; a world-sized DataGrid would allocate by empty terrain area.
// One preallocated linked record per emitted pole avoids allocating a Vec for each bucket.
struct PoleGrid {
    heads: HashMap<(i32, i32), usize>,
    poles: Vec<(Vector3, usize)>,
}

impl PoleGrid {
    fn new(mesh: &NetworkMeshData) -> Self {
        let mut grid = Self {
            heads: HashMap::with_capacity(mesh.street_lights.len()),
            poles: Vec::with_capacity(mesh.street_lights.len()),
        };
        for light in &mesh.street_lights {
            let key = Self::cell(light.position);
            let previous = grid.heads.insert(key, grid.poles.len()).unwrap_or(NO_LINK);
            grid.poles.push((light.position, previous));
        }
        grid
    }

    fn cell(p: Vector3) -> (i32, i32) {
        (
            (p.x / CLEARANCE_M).floor() as i32,
            (p.z / CLEARANCE_M).floor() as i32,
        )
    }

    // Required progress along the current roadside segment to leave all pole discs containing
    // this point. The query only reads nine cells and never iterates HashMap keys.
    fn advance(&self, position: Vector3, velocity: RoadVec2) -> f64 {
        let (x, z) = Self::cell(position);
        let speed_sq = velocity.length_squared();
        let mut advance: f64 = 0.0;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let mut index = self
                    .heads
                    .get(&(x + dx, z + dz))
                    .copied()
                    .unwrap_or(NO_LINK);
                while index != NO_LINK {
                    let (pole, next) = self.poles[index];
                    let delta = RoadVec2::new(
                        f64::from(position.x) - f64::from(pole.x),
                        f64::from(position.z) - f64::from(pole.z),
                    );
                    let inside = delta.length_squared() - f64::from(CLEARANCE_M * CLEARANCE_M);
                    // Planar clearance is conservative for the other side and elevated poles.
                    if inside < 0.0 {
                        if speed_sq <= f64::EPSILON {
                            return f64::INFINITY;
                        }
                        let dot = delta.dot(velocity);
                        let exit = (-dot + (dot * dot - speed_sq * inside).sqrt()) / speed_sq;
                        advance = advance.max(exit + CLEARANCE_ROUNDOFF_M);
                    }
                    index = next;
                }
            }
        }
        advance
    }
}

fn offset(section: &RoadSection, side: f32) -> Option<(f32, Option<f32>)> {
    let band_on_side = |kind| {
        section
            .bands
            .iter()
            .filter(|b| b.kind == kind)
            .filter_map(|b| {
                let (near, far, h0, h1) = if side > 0.0 {
                    (
                        b.lateral_start_m,
                        b.lateral_end_m,
                        b.height_start_m,
                        b.height_end_m,
                    )
                } else {
                    (
                        -b.lateral_end_m,
                        -b.lateral_start_m,
                        b.height_end_m,
                        b.height_start_m,
                    )
                };
                (far > 0.0).then_some((near.max(0.0), far, h0, h1))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
    };
    if let Some((near, far, h0, h1)) = band_on_side(BandKind::Sidewalk)
        && far - near >= 0.5
    {
        // The sidewalk begins behind the raised kerb band. Measure from the carriageway
        // boundary (the kerb face), rather than adding another 0.5 m behind that band.
        let kerb_face = band_on_side(BandKind::Carriageway)?.1;
        let distance = (kerb_face + SIGN_KERB_OFFSET_M).max(near);
        if distance >= far {
            return None;
        }
        return Some((
            distance,
            Some(h0 + (h1 - h0) * (distance - near) / (far - near)),
        ));
    }
    let far = [
        BandKind::CurbOrShoulder,
        BandKind::Carriageway,
    ]
    .into_iter()
    .filter_map(band_on_side)
    .map(|b| b.1)
    .max_by(f32::total_cmp)?;
    Some((far + 0.75, None))
}

fn pose(
    sections: &[RoadSection],
    terrain: &TerrainSystem,
    s: f32,
    side: f32,
    direction: f32,
    kind: RoadSignKind,
) -> Option<RoadSign> {
    if sections.len() < 2 || s < sections[0].s_m || s > sections.last()?.s_m {
        return None;
    }
    let index = sections
        .partition_point(|section| section.s_m < s)
        .clamp(1, sections.len() - 1);
    let (a, b) = (&sections[index - 1], &sections[index]);
    let t = f64::from(((s - a.s_m) / (b.s_m - a.s_m).max(f32::EPSILON)).clamp(0.0, 1.0));
    let lateral = a.lateral_xz.lerp(b.lateral_xz, t).try_normalize()?;
    let (oa, ha) = offset(a, side)?;
    let (ob, hb) = offset(b, side)?;
    // Interpolate the roadside knots themselves, matching the compiled band quads. This also
    // makes each clearance segment linear, so pole avoidance can solve its exact disc exit.
    let ground = (a.center_xz + a.lateral_xz * f64::from(side * oa))
        .lerp(b.center_xz + b.lateral_xz * f64::from(side * ob), t);
    let height = match (ha, hb) {
        (Some(a), Some(b)) => a + (b - a) * t as f32,
        _ => terrain.sample_height_world(ground.x as f32, ground.y as f32),
    };
    if !height.is_finite() {
        return None;
    }
    let tangent = RoadVec2::new(lateral.y, -lateral.x);
    let face = -tangent * f64::from(direction);
    Some(RoadSign {
        position: Vector3::new(ground.x as f32, height, ground.y as f32),
        yaw: face.x.atan2(face.y) as f32,
        kind,
    })
}

#[allow(clippy::too_many_arguments)]
fn place(
    sections: &[RoadSection],
    terrain: &TerrainSystem,
    poles: &PoleGrid,
    s: f32,
    away: f32,
    side: f32,
    direction: f32,
    kind: RoadSignKind,
) -> Option<(RoadSign, f32)> {
    if sections.len() < 2 || s < sections[0].s_m || s > sections.last()?.s_m {
        return None;
    }
    let mut s = s;
    let mut index = if away > 0.0 {
        sections.partition_point(|section| section.s_m <= s)
    } else {
        sections.partition_point(|section| section.s_m < s)
    }
    .clamp(1, sections.len() - 1);
    loop {
        let (a, b) = (&sections[index - 1], &sections[index]);
        let (oa, _) = offset(a, side)?;
        let (ob, _) = offset(b, side)?;
        let pa = a.center_xz + a.lateral_xz * f64::from(side * oa);
        let pb = b.center_xz + b.lateral_xz * f64::from(side * ob);
        let span = f64::from((b.s_m - a.s_m).max(f32::EPSILON));
        let velocity = (pb - pa) * (f64::from(away) / span);
        let sign = pose(sections, terrain, s, side, direction, kind)?;
        let progress = poles.advance(sign.position, velocity);
        if progress == 0.0 {
            return Some((sign, s));
        }
        let boundary = if away > 0.0 { b.s_m } else { a.s_m };
        let remaining = f64::from((boundary - s) * away);
        if progress < remaining {
            let shifted = (f64::from(s) + f64::from(away) * progress) as f32;
            s = if away > 0.0 {
                shifted.max(s.next_up())
            } else {
                shifted.min(s.next_down())
            };
        } else {
            // Move only along this edge, away from the owning junction; never through it.
            s = boundary;
            if away > 0.0 {
                if index + 1 >= sections.len() {
                    return None;
                }
                index += 1;
            } else {
                if index <= 1 {
                    return None;
                }
                index -= 1;
            }
        }
    }
}

fn crossing_s(sections: &[RoadSection], marking: CrosswalkMarking) -> Option<f32> {
    let center = (marking.start + marking.end) * 0.5;
    let point = RoadVec2::new(f64::from(center.x), f64::from(center.z));
    sections
        .windows(2)
        .filter_map(|pair| {
            let d = pair[1].center_xz - pair[0].center_xz;
            let norm = d.length_squared();
            if norm <= f64::EPSILON {
                return None;
            }
            let t = ((point - pair[0].center_xz).dot(d) / norm).clamp(0.0, 1.0);
            Some((
                (point - (pair[0].center_xz + d * t)).length_squared(),
                pair[0].s_m + (pair[1].s_m - pair[0].s_m) * t as f32,
            ))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
}

// Sparse station buckets of the touched street chains, sharing the linked-bucket pattern of
// PoleGrid. Query three station buckets (five at a loop seam); no per-sign allocation.
struct SignSpacing {
    heads: HashMap<(usize, i8, i8, i32), usize>,
    stations: Vec<(f32, usize)>,
}

impl SignSpacing {
    fn new(capacity: usize) -> Self {
        Self {
            heads: HashMap::with_capacity(capacity),
            stations: Vec::with_capacity(capacity),
        }
    }

    fn station(span: &ChainSpan, s: f32) -> f32 {
        if span.forward {
            span.start + s
        } else {
            span.end - s
        }
    }

    fn key(span: &ChainSpan, s: f32, side: f32, direction: f32) -> (usize, i8, i8, i32) {
        let orientation = if span.forward { 1.0 } else { -1.0 };
        (
            span.chain_id,
            (side * orientation) as i8,
            (direction * orientation) as i8,
            (Self::station(span, s) / SIGN_GAP_M).floor() as i32,
        )
    }

    fn insert(&mut self, span: &ChainSpan, s: f32, side: f32, direction: f32) {
        let key = Self::key(span, s, side, direction);
        let next = self
            .heads
            .insert(key, self.stations.len())
            .unwrap_or(NO_LINK);
        self.stations.push((Self::station(span, s), next));
    }

    fn advance(&self, span: &ChainSpan, s: f32, side: f32, direction: f32) -> f32 {
        let (chain, side, face, bucket) = Self::key(span, s, side, direction);
        let station = Self::station(span, s);
        let mut progress = 0.0f32;
        let mut buckets = [bucket - 1, bucket, bucket + 1, -1, -1];
        if let Some(length) = span.loop_length {
            if station < SIGN_GAP_M {
                let last = (length / SIGN_GAP_M).floor() as i32;
                buckets[3] = last - 1;
                buckets[4] = last;
            } else if station > length - SIGN_GAP_M {
                buckets[3] = 0;
                buckets[4] = 1;
            }
        }
        for (i, &query_bucket) in buckets.iter().enumerate() {
            if query_bucket < 0 || buckets[..i].contains(&query_bucket) {
                continue;
            }
            let mut index = self
                .heads
                .get(&(chain, side, face, query_bucket))
                .copied()
                .unwrap_or(NO_LINK);
            while index != NO_LINK {
                let (other, next) = self.stations[index];
                let mut distance = (other - station) * f32::from(face);
                if let Some(length) = span.loop_length {
                    distance = (distance + 0.5 * length).rem_euclid(length) - 0.5 * length;
                }
                if distance.abs() < SIGN_GAP_M {
                    progress = progress.max(distance + SIGN_GAP_M);
                }
                index = next;
            }
        }
        progress
    }
}

// Recheck gaps after light-pole avoidance; both can push a candidate into another sign.
// Progress is monotone along this edge. Crossing poles are registered at their zebra poses.
#[allow(clippy::too_many_arguments)]
fn place_spaced(
    sections: &[RoadSection],
    terrain: &TerrainSystem,
    poles: &PoleGrid,
    mut s: f32,
    direction: f32,
    kind: RoadSignKind,
    spacing: &SignSpacing,
    span: &ChainSpan,
) -> Option<(RoadSign, f32)> {
    loop {
        let advance = spacing.advance(span, s, direction, direction);
        if advance > 0.0 {
            s += direction * (advance + CLEARANCE_ROUNDOFF_M as f32);
        }
        let (sign, actual) = place(
            sections, terrain, poles, s, direction, direction, direction, kind,
        )?;
        if spacing.advance(span, actual, direction, direction) == 0.0 {
            return Some((sign, actual));
        }
        s = actual;
    }
}

// Fixed-width total key: raw IEEE bits of x, y, z, yaw, then model index. An unsigned bit
// order is sufficient for a total order; this does not promise a numerical coordinate order.
fn order_signs(signs: &mut Vec<RoadSign>) {
    if signs.len() < 2 {
        return;
    }
    let mut scratch = signs.clone();
    for word in (0..5).rev() {
        for byte in 0..4 {
            let key = |sign: &RoadSign| {
                let value = match word {
                    0 => sign.position.x.to_bits(),
                    1 => sign.position.y.to_bits(),
                    2 => sign.position.z.to_bits(),
                    3 => sign.yaw.to_bits(),
                    _ => sign.kind as u32,
                };
                ((value >> (byte * 8)) & 255) as usize
            };
            let mut offsets = [0usize; 256];
            for sign in signs.iter() {
                offsets[key(sign)] += 1;
            }
            let mut total = 0;
            for count in &mut offsets {
                let next = total + *count;
                *count = total;
                total = next;
            }
            for sign in signs.iter() {
                let bucket = key(sign);
                scratch[offsets[bucket]] = *sign;
                offsets[bucket] += 1;
            }
            std::mem::swap(signs, &mut scratch);
        }
    }
}

/// Emits signs after the light pass, retaining world-space clearance even across chunk borders.
pub(super) fn emit_road_signs(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    lanes: &LaneSystem,
    roadbed: &RoadbedSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    let mut degrees = HashMap::with_capacity(coverage.node_ids.len());
    let StreetSigns {
        lengths,
        warnings,
        stations,
        edges,
    } = street_lengths(graph, &coverage.edge_indices, &mut degrees);
    let poles = PoleGrid::new(mesh);
    // Include complete touched chains in all priority passes. This makes gaps across degree-two
    // nodes identical for full and local chunk rebuilds; only covered edges publish their poses.
    // Edge order within each independent chain is canonical; chain order does not affect gaps.
    let covered: std::collections::HashSet<_> = coverage.edge_indices.iter().copied().collect();
    let capacity = edges.len() * 12;
    let mut spacing = SignSpacing::new(capacity);
    let mut signs = Vec::with_capacity(coverage.edge_indices.len() * 12);
    let mut changed_limits = HashMap::with_capacity(coverage.node_ids.len());
    // Span sections run between the edge's two mouths; edges without a span are skipped.
    let visible_edges: Vec<_> = edges
        .iter()
        .filter_map(|&edge_id| {
            let sections = roadbed.sections(edge_id)?;
            (sections.len() >= 2).then_some((edge_id, sections))
        })
        .collect();
    for priority in 0..3 {
        for &(edge_id, sections) in &visible_edges {
            let edge = graph.edge(edge_id);
            let visible = sections;
            let span = &stations[&edge_id];
            let mut starts = [visible[0].s_m + 2.0, visible.last().unwrap().s_m - 2.0];
            let mut speed_starts = starts;
            for node in [edge.start_node, edge.end_node] {
                let Some(ids) = lanes.node_lanes.get(&(node as usize)) else {
                    continue;
                };
                for lane in ids {
                    let Some(marking) = lanes
                        .crosswalk_marking(*lane)
                        .filter(|m| m.edge_id == edge_id)
                    else {
                        continue;
                    };
                    let Some(s) = crossing_s(sections, marking) else {
                        continue;
                    };
                    let at_start = node == edge.start_node;
                    let away = if at_start { 1.0 } else { -1.0 };
                    for side in [1.0, -1.0] {
                        let direction = if edge.fwd_lanes > 0 && edge.bkw_lanes > 0 {
                            side
                        } else if edge.fwd_lanes > 0 {
                            1.0
                        } else if edge.bkw_lanes > 0 {
                            -1.0
                        } else {
                            continue;
                        };
                        let kind = if side == direction {
                            RoadSignKind::CrossingRight
                        } else {
                            RoadSignKind::CrossingLeft
                        };
                        let first = sections.first().map_or(s, |section| section.s_m);
                        let last = sections.last().map_or(s, |section| section.s_m);
                        let sign_s = (s - direction * CROSSING_SIGN_SETBACK_M).clamp(first, last);
                        if let Some((sign, placed_s)) = place(
                            sections, terrain, &poles, sign_s, away, side, direction, kind,
                        ) {
                            if side == direction && direction == away {
                                let slot = usize::from(!at_start);
                                starts[slot] = if at_start {
                                    starts[slot].max(placed_s + SIGN_GAP_M)
                                } else {
                                    starts[slot].min(placed_s - SIGN_GAP_M)
                                };
                                speed_starts[slot] = if at_start {
                                    speed_starts[slot].max(placed_s + SPEED_AFTER_CROSSING_M)
                                } else {
                                    speed_starts[slot].min(placed_s - SPEED_AFTER_CROSSING_M)
                                };
                            }
                            if priority == 0
                                && spacing.advance(span, placed_s, side, direction) == 0.0
                            {
                                if covered.contains(&edge_id) {
                                    signs.push(sign);
                                }
                                spacing.insert(span, placed_s, side, direction);
                            }
                        }
                    }
                }
            }
            if priority == 1
                && lengths[&edge_id] >= 40.0
                && let Some(kind) = speed_kind(edge.speed_limit)
            {
                for (slot, node, direction, lane_count) in [
                    (0, edge.start_node, 1.0, edge.fwd_lanes),
                    (1, edge.end_node, -1.0, edge.bkw_lanes),
                ] {
                    if lane_count == 0 || degree(graph, node, &mut degrees) < 3 {
                        continue;
                    }
                    let changes = *changed_limits.entry(node).or_insert_with(|| {
                        let mut first = None;
                        for &other in graph.node_adjacency(node) {
                            let arm = graph.edge(other);
                            if arm.deleted || arm.primary_type != TransitType::Road {
                                continue;
                            }
                            if let Some(mapped) = speed_kind(arm.speed_limit) {
                                if first.is_some_and(|first| first != mapped) {
                                    return true;
                                }
                                first = Some(mapped);
                            }
                        }
                        false
                    });
                    let repeat_m = if road_family(edge).is_some_and(|(_, wide)| wide) {
                        SPEED_REPEAT_WIDE_STREET_M
                    } else {
                        SPEED_REPEAT_STREET_M
                    };
                    if (changes || lengths[&edge_id] >= repeat_m)
                        && let Some((sign, s)) = place_spaced(
                            visible,
                            terrain,
                            &poles,
                            speed_starts[slot],
                            direction,
                            kind,
                            &spacing,
                            span,
                        )
                    {
                        if covered.contains(&edge_id) {
                            signs.push(sign);
                        }
                        spacing.insert(span, s, direction, direction);
                    }
                }
            }
            if priority == 2
                && let Some(warnings) = warnings.get(&edge_id)
            {
                for (slot, direction) in [(0, 1.0), (1, -1.0)] {
                    let mut candidates = [None; 3];
                    candidates.copy_from_slice(&warnings[slot * 3..slot * 3 + 3]);
                    // Nearest candidate wins ties in family order: curve, steep, uneven.
                    candidates.sort_by(|a, b| match (a, b) {
                        (Some(a), Some(b)) => (a.0 * direction).total_cmp(&(b.0 * direction)),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    });
                    let node = if slot == 0 {
                        edge.start_node
                    } else {
                        edge.end_node
                    };
                    let mouth = if degree(graph, node, &mut degrees) >= 3 {
                        starts[slot]
                    } else if slot == 0 {
                        visible[0].s_m
                    } else {
                        visible.last().unwrap().s_m
                    };
                    for (s, kind) in candidates.into_iter().flatten() {
                        let s = s.clamp(visible[0].s_m, visible.last().unwrap().s_m);
                        let s = if direction > 0.0 {
                            s.max(mouth)
                        } else {
                            s.min(mouth)
                        };
                        if let Some((sign, s)) = place_spaced(
                            visible, terrain, &poles, s, direction, kind, &spacing, span,
                        ) {
                            if covered.contains(&edge_id) {
                                signs.push(sign);
                            }
                            spacing.insert(span, s, direction, direction);
                        }
                    }
                }
            }
        }
    }
    order_signs(&mut signs);
    for sign in signs {
        mesh.push_road_sign(sign);
    }
}

#[cfg(test)]
mod tests;
