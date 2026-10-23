// SPDX-License-Identifier: GPL-2.0-only

//! Street lights along road and footpath stretches (`ROAD-59`).
//!
//! Placement depends only on a canonical street chain and its endpoint neighbours, so every
//! chunk rebuild agrees. Endpoint arms reserve pole clearance in family/edge-id order, without
//! consulting the opposite endpoint's decisions. Work per covered edge is
//! O(degree * (chain sections + walk steps * degree) + lights), junction degree <= 6; no
//! city-wide scan or recursive chain dependency.

use super::NetworkMeshData;
use super::standard_surface::CompiledSurfaceCoverage;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::roadbed::{BandKind, RoadSection, RoadbedSystem};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

/// Street light models, in the order of `godot/assets/models/street_lights/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum StreetLightKind {
    /// 8 m galvanised conical pole with one arm, on residential streets.
    ResidentialSteel = 0,
    /// Wooden pole with a steel bracket and an older luminaire, on older residential streets.
    ResidentialWood = 1,
    /// 10 m galvanised pole with a longer arm, on wider roads.
    Main = 2,
    /// Post-top light beside footpaths.
    Path = 4,
}

/// One placed street light: the pole base on the ground and the yaw that turns the model's
/// +X arm toward the road.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StreetLight {
    pub(crate) position: Vector3,
    pub(crate) yaw: f32,
    pub(crate) kind: StreetLightKind,
}

// Distance kept clear at each end of a visible stretch, beyond the junction clip, so poles
// stay out of crosswalks and corner radii.
const JUNCTION_MARGIN_M: f32 = 6.0;
// Dead ends keep a shorter clear margin than junctions.
const DEAD_END_MARGIN_M: f32 = 2.0;
// Matches the residential minimum longitudinal spacing, but leaves junctions lit well inside
// the usual 30/36 m spacing. This applies between chains, including both poles of Main arms.
const MIN_POLE_GAP_M: f32 = 12.0;
// Step of the walk that moves a crowded end pole away from its node. Poles are cosmetic: half a
// metre past the first clear spot is invisible, and every accepted spot is checked exactly.
const CLEARANCE_STEP_M: f32 = 0.5;
// A street shorter than this gets no light.
const MIN_STREET_M: f32 = 4.0;
// Pole axis inside the back edge of the sidewalk, clear of the parcel boundary behind it.
const SIDEWALK_BACK_INSET_M: f32 = 0.35;
// Without a sidewalk the pole stands this far beyond the kerb or shoulder, on the verge.
const VERGE_OFFSET_M: f32 = 0.75;
// Post-top lights stand this far beyond a footpath's edge.
const PATH_VERGE_OFFSET_M: f32 = 0.5;
// Older streets keep wooden poles: the share of cosmetic age above this uses them.
const WOODEN_POLE_MIN_AGE: f32 = 0.65;

struct Rule {
    kind: StreetLightKind,
    family: Family,
    // Upper bound: lights are spread evenly along a street at this spacing or less.
    spacing_m: f32,
    // A street whose lit length is shorter than this gets one light in its middle.
    min_gap_m: f32,
    both_sides: bool,
}

fn rule_for_edge(edge: &Edge) -> Option<Rule> {
    if edge.deleted || edge.class != EdgeClass::Standard {
        return None;
    }
    // Spacings are the look-dev defaults from the street-light models' metadata (assumed, not
    // from a lighting calculation). Spacing belongs to the family, so a street whose edges mix
    // steel and wooden poles still spaces them evenly.
    let (kind, family, spacing_m, min_gap_m, both_sides) = match edge.primary_type {
        TransitType::Foot => (StreetLightKind::Path, Family::Path, 20.0, 8.0, false),
        TransitType::Road if u16::from(edge.fwd_lanes) + u16::from(edge.bkw_lanes) <= 2 => {
            let kind = if super::cosmetic_road_age(edge) > WOODEN_POLE_MIN_AGE {
                StreetLightKind::ResidentialWood
            } else {
                StreetLightKind::ResidentialSteel
            };
            (kind, Family::Residential, 30.0, 12.0, false)
        }
        TransitType::Road => (StreetLightKind::Main, Family::Main, 36.0, 14.0, true),
        _ => return None,
    };
    Some(Rule {
        kind,
        family,
        spacing_m,
        min_gap_m,
        both_sides,
    })
}

/// Places the street lights of every covered edge into `mesh`, which routes each to its chunk.
///
/// Lights are spaced along whole streets: chains of edges through degree-two nodes that take the
/// same family of light. Each edge emits the chain's lights that fall on it, so every chunk
/// rebuild agrees. Endpoint resolution is O(degree * (chain sections + walk steps)), degree <= 6,
/// plus O(chain length + lights) for placement. Scratch allocation is per chain, never per pole.
pub(super) fn emit_street_lights(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    for &edge_idx in &coverage.edge_indices {
        let Some(rule) = rule_for_edge(graph.edge(edge_idx)) else {
            continue;
        };
        let chain = street_chain(graph, edge_idx, rule.family);
        let Some(runs) = chain_runs(roadbed, &chain) else {
            continue;
        };
        let (start, end) = chain_end_nodes(graph, &chain);
        let start_m = separated_end_margin(graph, roadbed, terrain, start, chain[0].0);
        let end_m = separated_end_margin(graph, roadbed, terrain, end, chain[chain.len() - 1].0);
        place_along_chain(
            mesh, terrain, &chain, &runs, edge_idx, &rule, start_m, end_m,
        );
    }
}

// The visible stretch of one chain edge, in chain order.
struct Run<'a> {
    sections: &'a [RoadSection],
    // Where this edge's stretch starts along the chain, node gaps collapsed.
    offset_m: f32,
    length_m: f32,
}

fn chain_runs<'a>(roadbed: &'a RoadbedSystem, chain: &[(usize, bool)]) -> Option<Vec<Run<'a>>> {
    let mut runs = Vec::with_capacity(chain.len());
    let mut offset_m = 0.0;
    for &(edge_idx, _) in chain {
        let sections = roadbed
            .sections(edge_idx)
            .filter(|sections| sections.len() >= 2)?;
        let length_m = (sections[sections.len() - 1].s_m - sections[0].s_m).max(0.0);
        runs.push(Run {
            sections,
            offset_m,
            length_m,
        });
        offset_m += length_m;
    }
    Some(runs)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Residential,
    Main,
    Path,
}

fn live_degree(graph: &RegionGraph, node: u32) -> usize {
    graph
        .node_adjacency(node)
        .iter()
        .filter(|&&edge_idx| !graph.edge(edge_idx).deleted)
        .count()
}

// The street through `edge_idx`: (edge, runs along the chain from its start to its end node),
// extended through degree-two nodes whose other edge takes the same family. The order is
// canonical (from the end with the lower node id, or for a closed loop from its lowest edge), so
// every edge of a chain computes the same placement.
fn street_chain(graph: &RegionGraph, edge_idx: usize, family: Family) -> Vec<(usize, bool)> {
    let extend = |from_edge: usize, mut node: u32, out: &mut Vec<(usize, bool)>| {
        let mut previous = from_edge;
        while live_degree(graph, node) == 2 {
            let Some(&next) = graph
                .node_adjacency(node)
                .iter()
                .find(|&&e| e != previous && !graph.edge(e).deleted)
            else {
                break;
            };
            if next == edge_idx
                || rule_for_edge(graph.edge(next)).is_none_or(|rule| rule.family != family)
            {
                break;
            }
            let edge = graph.edge(next);
            let forward = edge.start_node == node;
            out.push((next, forward));
            node = if forward {
                edge.end_node
            } else {
                edge.start_node
            };
            previous = next;
        }
        node
    };
    let edge = graph.edge(edge_idx);
    let mut behind = Vec::new();
    let first_node = extend(edge_idx, edge.start_node, &mut behind);
    let mut ahead = Vec::new();
    let last_node = extend(edge_idx, edge.end_node, &mut ahead);
    // `behind` walks away from the start node: reverse it and flip each edge's direction.
    let mut chain: Vec<(usize, bool)> = behind
        .into_iter()
        .rev()
        .map(|(e, forward)| (e, !forward))
        .collect();
    chain.push((edge_idx, true));
    chain.extend(ahead);
    let closed = first_node == last_node && chain.len() > 1 && live_degree(graph, first_node) == 2;
    if closed {
        let lowest = (0..chain.len()).min_by_key(|&i| chain[i].0).unwrap_or(0);
        chain.rotate_left(lowest);
    } else if first_node > last_node {
        chain.reverse();
        for link in &mut chain {
            link.1 = !link.1;
        }
    }
    chain
}

fn end_margin(graph: &RegionGraph, node: u32) -> f32 {
    // Junctions keep their mouths and crosswalks clear; a dead end needs only a short margin.
    if live_degree(graph, node) >= 3 {
        JUNCTION_MARGIN_M
    } else {
        DEAD_END_MARGIN_M
    }
}

fn chain_end_nodes(graph: &RegionGraph, chain: &[(usize, bool)]) -> (u32, u32) {
    let (first, forward) = chain[0];
    let first = graph.edge(first);
    let start = if forward {
        first.start_node
    } else {
        first.end_node
    };
    let (last, forward) = chain[chain.len() - 1];
    let last = graph.edge(last);
    let end = if forward {
        last.end_node
    } else {
        last.start_node
    };
    (start, end)
}

fn arm_rank(graph: &RegionGraph, edge: usize) -> Option<(u8, usize)> {
    let family = rule_for_edge(graph.edge(edge))?.family;
    Some((
        match family {
            Family::Main => 0,
            Family::Residential => 1,
            Family::Path => 2,
        },
        edge,
    ))
}

// Arms at `node` are resolved in rank order (Main, Residential, Path, then edge id) up to
// `target`: each arm's end pole keeps the normal margin unless it would stand within
// `MIN_POLE_GAP_M` of an end pole already accepted there, in which case it walks away from the
// node in `CLEARANCE_STEP_M` steps to the first clear spot. Candidates use only this endpoint's
// margin and the opposite end's normal margin, never another endpoint's resolution, so there is
// no dependency cycle. O(degree * (chain sections + walk steps * degree)) per call, degree <= 6.
fn separated_end_margin(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    terrain: &TerrainSystem,
    node: u32,
    target: usize,
) -> f32 {
    let normal = end_margin(graph, node);
    let degree = live_degree(graph, node);
    if degree < 2 {
        return normal;
    }
    let target_rank = arm_rank(graph, target);
    // A same-family degree-two node is internal to a chain, including closed chains.
    if degree == 2
        && graph.node_adjacency(node).iter().all(|&edge| {
            graph.edge(edge).deleted
                || arm_rank(graph, edge).map(|rank| rank.0) == target_rank.map(|rank| rank.0)
        })
    {
        return normal;
    }
    let mut accepted: Vec<(usize, StreetLight)> = Vec::with_capacity(degree * 2);
    let mut previous = None;
    while let Some((rank, edge)) = graph
        .node_adjacency(node)
        .iter()
        .filter_map(|&edge| {
            let rank = arm_rank(graph, edge)?;
            (Some(rank) > previous && Some(rank) <= target_rank).then_some((rank, edge))
        })
        .min_by_key(|&(rank, _)| rank)
    {
        previous = Some(rank);
        let rule = rule_for_edge(graph.edge(edge)).unwrap();
        let chain = street_chain(graph, edge, rule.family);
        let Some(runs) = chain_runs(roadbed, &chain) else {
            continue;
        };
        let total = runs.iter().map(|run| run.length_m).sum::<f32>();
        if total < MIN_STREET_M {
            continue;
        }
        let (start, end) = chain_end_nodes(graph, &chain);
        let from_start = start == node && chain[0].0 == edge;
        let cap = total - end_margin(graph, if from_start { end } else { start });
        if cap < normal {
            // No end candidate fits: keep the existing middle-light fallback.
            if edge == target {
                return normal;
            }
            continue;
        }
        // Two arms of a chain that returns to this node must not compete with themselves.
        let chain_key = chain.iter().map(|link| link.0).min().unwrap();
        let mut margin = normal;
        let found = loop {
            let along = if from_start { margin } else { total - margin };
            let poles = poles_at(terrain, &chain, &runs, along, &rule);
            let clear = poles.iter().flatten().all(|pole| {
                accepted.iter().all(|&(owner, other)| {
                    owner == chain_key
                        || planar_distance(pole.position, other.position) >= MIN_POLE_GAP_M
                })
            });
            if clear {
                break Some(poles);
            }
            if margin >= cap {
                break None;
            }
            margin = (margin + CLEARANCE_STEP_M).min(cap);
        };
        if edge == target {
            return if found.is_some() { margin } else { cap };
        }
        if let Some(poles) = found {
            accepted.extend(poles.into_iter().flatten().map(|pole| (chain_key, pole)));
        }
    }
    normal
}

fn planar_distance(a: Vector3, b: Vector3) -> f32 {
    (a.x - b.x).hypot(a.z - b.z)
}

// The poles `place_along_chain` would put `along` metres from the chain's start.
fn poles_at(
    terrain: &TerrainSystem,
    chain: &[(usize, bool)],
    runs: &[Run],
    along: f32,
    rule: &Rule,
) -> [Option<StreetLight>; 2] {
    let index = runs
        .iter()
        .position(|run| along < run.offset_m + run.length_m)
        .unwrap_or(runs.len() - 1);
    let run = &runs[index];
    let forward = chain[index].1;
    let local = (along - run.offset_m).clamp(0.0, run.length_m);
    let sections = run.sections;
    let s = if forward {
        sections[0].s_m + local
    } else {
        sections[sections.len() - 1].s_m - local
    };
    let i = sections
        .windows(2)
        .position(|pair| pair[1].s_m >= s)
        .unwrap_or(sections.len() - 2);
    let (a, b) = (&sections[i], &sections[i + 1]);
    let side = chain_side(&runs[0], chain[0].1) * if forward { 1.0 } else { -1.0 };
    let place = |side| place_at(terrain, a, b, s, side, rule.kind);
    if rule.both_sides {
        [place(1.0), place(-1.0)]
    } else {
        [place(side), None]
    }
}

#[allow(clippy::too_many_arguments)]
fn place_along_chain(
    mesh: &mut NetworkMeshData,
    terrain: &TerrainSystem,
    chain: &[(usize, bool)],
    runs: &[Run],
    edge_idx: usize,
    rule: &Rule,
    start_m: f32,
    end_m: f32,
) {
    let Some(index) = chain.iter().position(|&(e, _)| e == edge_idx) else {
        return;
    };
    let total_m = runs.iter().map(|run| run.length_m).sum::<f32>();
    let usable_m = total_m - start_m - end_m;
    // Positions along the chain: one light in the middle of a short street, else evenly spread
    // with both ends lit and every gap between the family's minimum and maximum.
    let (first_m, step_m, count) = if usable_m < rule.min_gap_m {
        if total_m < MIN_STREET_M {
            return;
        }
        (start_m.min(total_m * 0.5) + usable_m.max(0.0) * 0.5, 0.0, 1)
    } else {
        let gaps = (usable_m / rule.spacing_m).ceil().max(1.0);
        (start_m, usable_m / gaps, gaps as usize + 1)
    };
    let run = &runs[index];
    let forward = chain[index].1;
    let side_sign = chain_side(&runs[0], chain[0].1) * if forward { 1.0 } else { -1.0 };
    let sides: &[f32] = if rule.both_sides {
        &[1.0, -1.0]
    } else {
        &[side_sign]
    };
    let is_last = index + 1 == chain.len();
    let mut section_index = 0;
    for k in 0..count {
        let along = first_m + step_m * k as f32 - run.offset_m;
        // Each edge owns [offset, offset + length); the last also owns the street's end.
        let inside = along >= 0.0 && (along < run.length_m || (is_last && along <= run.length_m));
        if !inside {
            continue;
        }
        let sections = run.sections;
        let s = if forward {
            sections[0].s_m + along
        } else {
            sections[sections.len() - 1].s_m - along
        };
        while section_index + 2 < sections.len() && sections[section_index + 1].s_m < s {
            section_index += 1;
        }
        // Reversed runs visit decreasing s: search from the start for each light.
        if !forward {
            section_index = sections
                .windows(2)
                .position(|pair| pair[1].s_m >= s)
                .unwrap_or(sections.len() - 2);
        }
        let (a, b) = (&sections[section_index], &sections[section_index + 1]);
        for &side in sides {
            if let Some(light) = place_at(terrain, a, b, s, side, rule.kind) {
                mesh.push_street_light(light);
            }
        }
    }
}

// One-sided streets keep their lights on the same side of the world whichever way the street
// was drawn: the side is chosen once on the chain's first run, in the chain's direction of travel,
// so a street made of several edges does not hop sides at its nodes.
fn chain_side(first: &Run, forward: bool) -> f32 {
    let lateral = first.sections[first.sections.len() / 2].lateral_xz;
    let world = if lateral.x * 0.6 + lateral.y * 0.8 >= 0.0 {
        1.0
    } else {
        -1.0
    };
    if forward { world } else { -world }
}

fn place_at(
    terrain: &TerrainSystem,
    a: &RoadSection,
    b: &RoadSection,
    s: f32,
    side: f32,
    kind: StreetLightKind,
) -> Option<StreetLight> {
    let span = b.s_m - a.s_m;
    let t = if span > f32::EPSILON {
        f64::from(((s - a.s_m) / span).clamp(0.0, 1.0))
    } else {
        0.0
    };
    let center = a.center_xz.lerp(b.center_xz, t);
    let lateral = a.lateral_xz.lerp(b.lateral_xz, t).try_normalize()?;
    let near = if t < 0.5 { a } else { b };
    let (offset_m, band_height) = pole_offset(near, side, kind)?;
    let ground = center + lateral * f64::from(offset_m * side);
    let height = band_height
        .unwrap_or_else(|| terrain.sample_height_world(ground.x as f32, ground.y as f32));
    if !height.is_finite() {
        return None;
    }
    // The model's arm points along +X; turn it toward the road centre.
    let toward_road = -lateral * f64::from(side);
    Some(StreetLight {
        position: Vector3::new(ground.x as f32, height, ground.y as f32),
        yaw: (-toward_road.y).atan2(toward_road.x) as f32,
        kind,
    })
}

// Lateral distance of the pole from the centreline on `side`, and the surface height there when
// the pole stands on a compiled band (None: on the verge, use the terrain).
fn pole_offset(
    section: &RoadSection,
    side: f32,
    kind: StreetLightKind,
) -> Option<(f32, Option<f32>)> {
    let on_side = |kinds: &[BandKind]| {
        section
            .bands
            .iter()
            .filter(|band| kinds.contains(&band.kind))
            .filter_map(|band| {
                let (near, far, near_h, far_h) = if side > 0.0 {
                    (
                        band.lateral_start_m,
                        band.lateral_end_m,
                        band.height_start_m,
                        band.height_end_m,
                    )
                } else {
                    (
                        -band.lateral_end_m,
                        -band.lateral_start_m,
                        band.height_end_m,
                        band.height_start_m,
                    )
                };
                (far > 0.0).then_some((near.max(0.0), far, near_h, far_h))
            })
            .max_by(|x, y| x.1.total_cmp(&y.1))
    };
    if kind == StreetLightKind::Path {
        let (_, far, _, _) =
            on_side(&[BandKind::Footpath, BandKind::Sidewalk])?;
        return Some((far + PATH_VERGE_OFFSET_M, None));
    }
    if let Some((near, far, near_h, far_h)) = on_side(&[BandKind::Sidewalk])
        && far - near > 2.0 * SIDEWALK_BACK_INSET_M
    {
        let offset = far - SIDEWALK_BACK_INSET_M;
        let t = (offset - near) / (far - near);
        return Some((offset, Some(near_h + (far_h - near_h) * t)));
    }
    let (_, far, _, _) = on_side(&[
        BandKind::CurbOrShoulder,
        BandKind::Carriageway,
    ])?;
    Some((far + VERGE_OFFSET_M, None))
}

#[cfg(test)]
mod tests;
