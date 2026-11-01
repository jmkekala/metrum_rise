// SPDX-License-Identifier: GPL-2.0-only

//! Road tunnels drawn with the road tool's tunnel intent (`ROAD-61`).
//!
//! The stroke keeps its authored profile: a straight stroke the chord between its two ends, a
//! curve the heights the tool drew along it. Preparation puts its knots on a lattice at most
//! [`TUNNEL_LATTICE_M`] apart, and a knot's cover is the source terrain's height above it. The
//! bore runs from the first knot with the bore clearance [`TUNNEL_COVER_M`] of cover to the last,
//! and every knot between them must have it; the stretches before and after it are open
//! approaches. Quick validation and the exact plan read the same knots through [`tunnel_bore`],
//! so they accept and refuse the same strokes.
//!
//! The validation graph splits the stroke at its portals ([`portal_splits`]): the approaches are
//! standard roads in a cut, the bore a lined tunnel edge whose portal rows end the cut at a
//! collar (`TERRAIN-07`, `docs/terrain.md` T22), and an end inside the bore is a buried
//! continuation with no portal of its own. A bore meets no other road: a junction inside it, or a crossing
//! with another tunnel at any height, refuses the stroke ([`TUNNEL_CROSSING_REASON`]).

use super::PlannedRoadSplit;
use super::input::{
    PreparedRoadInput, simplify_road_alignment_points, snap_road_endpoints_to_existing_nodes,
    xz_distance,
};
use crate::config;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction::find_intersection_2d;
use crate::simulation::network::types::{EdgeClass, NodeType};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::PORTAL_CLEARANCE_M;
use godot::prelude::Vector3;

/// The longest step between two knots of a tunnel stroke, metres: the lattice quick validation
/// and the exact plan both measure cover on.
pub(super) const TUNNEL_LATTICE_M: f32 = 4.0;
/// The cover a bore keeps over its deck from portal to portal: the bore clearance, which the
/// lining's ceiling and the portal's collar stand within (`ROAD-61`).
pub(super) const TUNNEL_COVER_M: f32 = PORTAL_CLEARANCE_M as f32;
/// The shortest open approach before a portal, metres: two lattice steps, so the portal node
/// stands clear of the stroke's end.
pub(super) const TUNNEL_MIN_APPROACH_M: f32 = 2.0 * TUNNEL_LATTICE_M;
/// A bore that meets another road: a junction inside it or a crossing with another tunnel.
pub(super) const TUNNEL_CROSSING_REASON: &str = "tunnel_crossing";

/// Why a tunnel stroke's knots form no bore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BoreRefusal {
    /// The cover at `knot`: the most of any knot when none reaches [`TUNNEL_COVER_M`], else the
    /// least between the first and last that do.
    Cover { cover_m: f32, knot: usize },
    /// An open approach shorter than [`TUNNEL_MIN_APPROACH_M`], with its length.
    Approach { length_m: f32 },
}

/// Prepares a stroke drawn with the tunnel intent: the alignment simplified as for any road, its
/// ends snapped to existing nodes, every height as authored, and each segment divided into equal
/// steps of at most [`TUNNEL_LATTICE_M`], so the knots interpolate the authored profile and a
/// straight stroke keeps the chord between its ends. `O(stroke length / lattice + points)`.
pub(super) fn prepare_tunnel_input(
    raw_points: &[Vector3],
    graph: &RegionGraph,
    snap_to_existing_roads: bool,
) -> PreparedRoadInput {
    let mut alignment = Vec::new();
    simplify_road_alignment_points(raw_points, &mut alignment);
    if snap_to_existing_roads {
        snap_road_endpoints_to_existing_nodes(&mut alignment, graph);
    }
    let mut points = Vec::with_capacity(alignment.len());
    points.extend(alignment.first().copied());
    for pair in alignment.windows(2) {
        let steps = (xz_distance(pair[0], pair[1]) / TUNNEL_LATTICE_M)
            .ceil()
            .max(1.0) as usize;
        points.extend((1..steps).map(|step| pair[0].lerp(pair[1], step as f32 / steps as f32)));
        points.push(pair[1]);
    }
    PreparedRoadInput {
        points,
        class: EdgeClass::Tunnel,
        endpoint_snap_enabled: snap_to_existing_roads,
        tunnel: true,
        forced_grade: None,
    }
}

/// The source terrain's height above `point`, metres.
pub(super) fn cover_m(terrain: &TerrainSystem, point: Vector3) -> f32 {
    terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE - point.y
}

/// The bore of a prepared tunnel stroke: the indices of its first and last knot with the bore
/// clearance of cover, every knot between them covered too and each open approach at least
/// [`TUNNEL_MIN_APPROACH_M`] long. `O(knots)`, allocation-free.
pub(super) fn tunnel_bore(
    points: &[Vector3],
    terrain: &TerrainSystem,
) -> Result<[usize; 2], BoreRefusal> {
    let covered = |index: usize| cover_m(terrain, points[index]) >= TUNNEL_COVER_M;
    let Some(first) = (0..points.len()).find(|&index| covered(index)) else {
        let (knot, cover_m) = (0..points.len())
            .map(|index| (index, cover_m(terrain, points[index])))
            .fold((0, f32::NEG_INFINITY), |best, knot| {
                if knot.1 > best.1 { knot } else { best }
            });
        return Err(BoreRefusal::Cover { cover_m, knot });
    };
    let last = (first..points.len())
        .rev()
        .find(|&index| covered(index))
        .unwrap_or(first);
    if let Some((knot, cover_m)) = (first..=last)
        .map(|index| (index, cover_m(terrain, points[index])))
        .filter(|&(_, cover)| cover < TUNNEL_COVER_M)
        .fold(None, |least: Option<(usize, f32)>, knot| match least {
            Some(least) if least.1 <= knot.1 => Some(least),
            _ => Some(knot),
        })
    {
        return Err(BoreRefusal::Cover { cover_m, knot });
    }
    let length_m = |range: &[Vector3]| {
        range
            .windows(2)
            .map(|pair| xz_distance(pair[0], pair[1]))
            .sum::<f32>()
    };
    for approach in [&points[..=first], &points[last..]] {
        let length_m = length_m(approach);
        if approach.len() > 1 && length_m < TUNNEL_MIN_APPROACH_M {
            return Err(BoreRefusal::Approach { length_m });
        }
    }
    Ok([first, last])
}

/// A tunnel stroke's portals in the validation graph.
pub(super) struct Portals {
    /// The portal node before and after the bore, absent at a buried end.
    pub(super) nodes: [Option<u32>; 2],
    /// Each portal as a split of the stroke's edge, `(knot, node)`.
    pub(super) splits: Vec<(f32, u32)>,
}

/// The portal nodes of a tunnel stroke's bore in the validation graph, one per open approach,
/// each added at its knot as a split of the stroke's edge for
/// [`crate::simulation::network::topology::process_intersections_splitting_at`].
pub(super) fn portal_splits(
    graph: &mut RegionGraph,
    points: &[Vector3],
    [first, last]: [usize; 2],
) -> Portals {
    let mut splits = Vec::with_capacity(2);
    let mut portal = |knot: usize| {
        let node = graph.add_node(points[knot], NodeType::Junction);
        splits.push((knot as f32, node));
        node
    };
    let start = (first > 0).then(|| portal(first));
    let end = (last + 1 < points.len()).then(|| portal(last));
    Portals {
        nodes: [start, end],
        splits,
    }
}

/// Classes the drawn road's edges once the validation graph split it: tunnel between the
/// portals (or from a buried end), standard on the approaches. Refuses with
/// [`TUNNEL_CROSSING_REASON`] when another road meets the bore: a node inside it, a portal that
/// is not a plain pass-through, or a buried end shared by more than one other road.
/// `O(stroke edges^2)`; a stroke has a handful.
pub(super) fn classify_tunnel_stroke(
    graph: &mut RegionGraph,
    new_edge_idx: usize,
    splits: &[PlannedRoadSplit],
    [start_node, end_node]: [u32; 2],
    portals: [Option<u32>; 2],
) -> Result<(), &'static str> {
    let mut stroke = vec![new_edge_idx];
    for split in splits {
        if stroke.contains(&split.edge_id) {
            stroke.push(split.new_edge_id);
        }
    }
    let degree = |graph: &RegionGraph, node: u32| {
        graph
            .node_adjacency(node)
            .iter()
            .filter(|&&edge| !graph.edge(edge).deleted)
            .count()
    };
    let valid =
        |graph: &RegionGraph, node: Option<u32>| node.map(|node| graph.get_valid_node(node));
    let [portal_in, portal_out] = portals.map(|node| valid(graph, node));
    let end_node = graph.get_valid_node(end_node);
    let mut node = graph.get_valid_node(start_node);
    let mut inside = portal_in.is_none();
    if inside && degree(graph, node) > 2 {
        return Err(TUNNEL_CROSSING_REASON);
    }
    while node != end_node {
        let Some(&edge_idx) = stroke.iter().find(|&&edge_idx| {
            let edge = graph.edge(edge_idx);
            !edge.deleted && graph.get_valid_node(edge.start_node) == node
        }) else {
            return Err(TUNNEL_CROSSING_REASON);
        };
        graph.edge_mut(edge_idx).class = if inside {
            EdgeClass::Tunnel
        } else {
            EdgeClass::Standard
        };
        node = graph.get_valid_node(graph.edge(edge_idx).end_node);
        let portal = Some(node) == portal_in || Some(node) == portal_out;
        if portal && degree(graph, node) != 2 {
            return Err(TUNNEL_CROSSING_REASON);
        }
        if Some(node) == portal_in {
            inside = true;
        } else if Some(node) == portal_out {
            inside = false;
        } else if inside && (node != end_node || degree(graph, node) > 2) {
            return Err(TUNNEL_CROSSING_REASON);
        }
    }
    Ok(())
}

/// Whether the bore `points` crosses, in plan, a tunnel edge of `graph` that does not end at
/// one of `stroke_ends`, at any height. `O(bore segments * local tunnel segments)` over the
/// validation graph's local excerpt.
pub(super) fn bore_crosses_a_tunnel(
    graph: &RegionGraph,
    points: &[Vector3],
    stroke_ends: [Option<u32>; 2],
) -> bool {
    graph.edges().iter().any(|edge| {
        let joins_end = stroke_ends.iter().flatten().any(|&node| {
            graph.get_valid_node(edge.start_node) == node
                || graph.get_valid_node(edge.end_node) == node
        });
        !edge.deleted
            && edge.class == EdgeClass::Tunnel
            && !joins_end
            && edge.geometry.windows(2).any(|other| {
                points.windows(2).any(|bore| {
                    find_intersection_2d(bore[0], bore[1], other[0], other[1]).is_some()
                })
            })
    })
}
