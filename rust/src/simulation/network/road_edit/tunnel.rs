// SPDX-License-Identifier: GPL-2.0-only

//! Road tunnels drawn with the road tool's tunnel intent (`ROAD-61`).
//!
//! The stroke keeps its authored profile: a straight stroke the chord between its two ends, a
//! curve the heights the tool drew along it. Preparation puts its knots on a lattice at most
//! [`TUNNEL_LATTICE_M`] apart, and a knot's cover is the least height of the source terrain above
//! it across the bore's whole section ([`section_cover_m`], round R8f): on a lattice across the
//! knot from one outer face of the lining's shell to the other, the cover over the shell's roof
//! (the lining's height and its thickness, [`TUNNEL_COVER_M`]) at its corners and between them,
//! not on the centre line alone. The bore runs from the first knot with that cover to the last,
//! and every knot between them must have it; the stretches before and after it are open
//! approaches. Quick validation and the exact plan read the same knots through [`tunnel_bore`],
//! so they accept and refuse the same strokes.
//!
//! The validation graph splits the stroke at its portals ([`portal_splits`]): the approaches are
//! standard roads in a cut, the bore a lined tunnel edge whose portal rows end the cut at a
//! collar (`TERRAIN-07`, `docs/terrain.md` T22), and an end inside the bore is a buried
//! continuation with no portal of its own. A bore meets no other road: a junction inside it, or a crossing
//! with another tunnel at any height, refuses the stroke ([`TUNNEL_CROSSING_REASON`]). Nor does an
//! approach's trench below the ground (round R8f): any stroke that makes a junction there is
//! refused ([`junction_in_trench`]).

use super::PlannedRoadSplit;
use super::input::{
    PreparedRoadInput, is_surface_edge, simplify_road_alignment_points, snap_road_endpoints_to_existing_nodes,
    xz_distance,
};
use crate::config;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction::find_intersection_2d;
use crate::simulation::network::roadbed::{
    self, BRIDGE_DECK_DEPTH_M, SKIRT_DEPTH_M, TUNNEL_LINING_THICKNESS_M,
    preview_candidate_roadbed_half_width_m, visual_roadbed_half_width_m,
};
use crate::simulation::network::types::{EdgeClass, NodeType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

/// The longest step between two knots of a tunnel stroke, metres: the lattice quick validation
/// and the exact plan both measure cover on.
pub(super) const TUNNEL_LATTICE_M: f32 = 4.0;
/// The cover a bore keeps over its deck from portal to portal: the tunnel cover rule
/// ([`roadbed::TUNNEL_COVER_M`], `ROAD-61`).
pub(super) const TUNNEL_COVER_M: f32 = roadbed::TUNNEL_COVER_M as f32;
/// How far a grounded road's deck stands at least above the deck of a tunnel it crosses
/// (`ROAD-61` round R8h, the user's rule): the cover rule and the road's own depth below its
/// deck (its skirt), so no road geometry reaches the tunnel's shell.
pub(super) const ROAD_OVER_TUNNEL_M: f32 = (roadbed::TUNNEL_COVER_M + SKIRT_DEPTH_M) as f32;
/// How far a bridge's deck stands at least above the deck of a road it passes over (`ROAD-99`):
/// the vehicle envelope ([`roadbed::VEHICLE_CLEARANCE_M`]) clear under the bridge's slab, and the
/// slab's depth ([`BRIDGE_DECK_DEPTH_M`]), deck to deck (4.85 m). The mirror of
/// [`ROAD_OVER_TUNNEL_M`]: under a tunnel's ceiling the lining keeps the same envelope clear.
pub(super) const ROAD_UNDER_BRIDGE_M: f32 =
    (roadbed::VEHICLE_CLEARANCE_M + BRIDGE_DECK_DEPTH_M) as f32;
/// The shortest open approach before a portal, metres: two lattice steps, so the portal node
/// stands clear of the stroke's end.
pub(super) const TUNNEL_MIN_APPROACH_M: f32 = 2.0 * TUNNEL_LATTICE_M;
/// A bore that meets another road: a junction inside it or a crossing with another tunnel.
pub(super) const TUNNEL_CROSSING_REASON: &str = "tunnel_crossing";
/// A junction below the ground in a tunnel approach's trench ([`junction_in_trench`]).
pub(super) const JUNCTION_IN_TRENCH_REASON: &str = "junction_in_trench";
/// A grounded road over a tunnel bore whose deck stands less than [`ROAD_OVER_TUNNEL_M`] above
/// the bore's deck ([`deck_over_tunnels_m`], `ROAD-61` round R8h).
pub(super) const TUNNEL_TOO_CLOSE_REASON: &str = "tunnel_too_close";

/// Why a tunnel stroke's knots form no bore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BoreRefusal {
    /// The section cover at `knot` ([`section_cover_m`]): the most of any knot when none
    /// reaches [`TUNNEL_COVER_M`], else the least between the first and last that do.
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
        elevated_ends: [false; 2],
    }
}

/// The source terrain's height above `point`, metres.
pub(super) fn cover_m(terrain: &TerrainSystem, point: Vector3) -> f32 {
    terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE - point.y
}

/// The half width of a tunnel stroke's section at the outer faces of the lining's shell: the
/// roadbed's half width, on whose outer edges the lining's walls stand, and the lining's
/// thickness (`ROAD-61` round R8f). Allocates one two-point edge; once per stroke.
pub(super) fn tunnel_section_half_width_m(points: &[Vector3], fwd_lanes: u8, bkw_lanes: u8) -> f32 {
    preview_candidate_roadbed_half_width_m(points, EdgeClass::Tunnel, fwd_lanes, bkw_lanes)
        + TUNNEL_LINING_THICKNESS_M as f32
}

/// A grounded road's deck as the tunnel rules read it (`ROAD-61` round R8h): its graph profile
/// and its roadbed's half width. Over a tunnel bore the deck is the surface the bore keeps its
/// cover under, wherever its footprint lies over the bore's section; beside the deck the road's
/// graded cut rises from it at the 1:2 tie-in slope, steeper than any road grade
/// (`ROAD_GRADE_MAX`), so the deck bounds the cut over the section.
pub(super) struct RoadDeck<'a> {
    profile: &'a [Vector3],
    half_width_m: f32,
}

impl<'a> RoadDeck<'a> {
    /// The deck of a profile, `half_width_m` to either side of it.
    pub(super) fn new(profile: &'a [Vector3], half_width_m: f32) -> Self {
        Self {
            profile,
            half_width_m,
        }
    }

    // The deck's height over the XZ point: its profile's at the nearest point of its centre
    // line, where that lies within the half width. `O(profile points)`.
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let (distance_sq, height) = RegionGraph::profile_nearest_at(self.profile, x, z);
        (distance_sq <= self.half_width_m * self.half_width_m).then_some(height)
    }
}

/// The decks of the grounded roads of `graph` that can lie over the section of a tunnel stroke
/// along `points` reaching `half_width_m` to either side (`ROAD-61` round R8h): standard road
/// and footpath edges whose centre lines lie within that and [`ROAD_DECK_REACH_M`] of the
/// stroke's XZ bounds, in ascending edge order. Bridges stand above the ground and tunnels in
/// it. A local excerpt query: `O(log E + K)` for the `K` edges met, one profile table per edge.
pub(super) fn grounded_decks_near<'g>(
    graph: &'g RegionGraph,
    points: &[Vector3],
    half_width_m: f32,
) -> Vec<RoadDeck<'g>> {
    let mut edges = edges_near(graph, points, half_width_m);
    edges.sort_unstable();
    edges.dedup();
    edges
        .into_iter()
        .map(|id| graph.edge(id))
        .filter(|edge| is_surface_edge(edge) && edge.class == EdgeClass::Standard)
        .map(|edge| RoadDeck::new(edge_profile(edge), visual_roadbed_half_width_m(edge)))
        .collect()
}

/// How far beyond a road's centre line its roadbed reaches at most, metres: wider than the
/// widest roadbed half width a road profile builds, the shell of a tunnel's section included
/// (`ROAD-61` round R8h). Bounds the excerpt queries of the tunnel rules; an edge's spatial
/// index entry holds its centre line only.
pub(super) const ROAD_DECK_REACH_M: f32 = 32.0;

// The edges of `graph` whose centre-line bounds lie within `half_width_m` and
// `ROAD_DECK_REACH_M` of the XZ bounds of `points`, unsorted.
fn edges_near(graph: &RegionGraph, points: &[Vector3], half_width_m: f32) -> Vec<usize> {
    let ((x0, z0), (x1, z1)) = xz_bounds(points);
    let pad = half_width_m + ROAD_DECK_REACH_M;
    graph.get_edges_near_aabb(
        Vector3::new(x0 - pad, 0.0, z0 - pad),
        Vector3::new(x1 + pad, 0.0, z1 + pad),
    )
}

// The physical profile when present, otherwise the control polyline.
fn edge_profile(edge: &crate::simulation::network::graph::Edge) -> &[Vector3] {
    if edge.physical_geometry.len() >= 2 {
        &edge.physical_geometry
    } else {
        &edge.geometry
    }
}

// The XZ bounds `((min_x, min_z), (max_x, max_z))` of `points`.
fn xz_bounds(points: &[Vector3]) -> ((f32, f32), (f32, f32)) {
    points.iter().fold(
        ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN)),
        |((x0, z0), (x1, z1)), p| ((x0.min(p.x), z0.min(p.z)), (x1.max(p.x), z1.max(p.z))),
    )
}

// The XZ points of knot `index`'s section lattice (round R8f): across the knot, perpendicular in
// plan to the chord of its neighbours, from `-half_width_m` to `half_width_m` in equal steps of
// at most [`TUNNEL_LATTICE_M`], both shell corners and the centre among them; the knot alone
// for a zero chord or width. `O(half_width_m / TUNNEL_LATTICE_M)`, allocation-free.
fn section_lattice(
    points: &[Vector3],
    index: usize,
    half_width_m: f32,
) -> impl Iterator<Item = (f32, f32)> {
    let knot = points[index];
    let (before, after) = (
        points[index.saturating_sub(1)],
        points[(index + 1).min(points.len() - 1)],
    );
    let (dx, dz) = (after.x - before.x, after.z - before.z);
    let length = dx.hypot(dz);
    let flat = length == 0.0 || half_width_m <= 0.0;
    let (across_x, across_z) = if flat {
        (0.0, 0.0)
    } else {
        (-dz / length, dx / length)
    };
    let steps = if flat {
        0
    } else {
        (half_width_m / TUNNEL_LATTICE_M).ceil() as i32
    };
    (-steps..=steps).map(move |step| {
        let offset = if steps == 0 {
            0.0
        } else {
            half_width_m * step as f32 / steps as f32
        };
        (knot.x + offset * across_x, knot.z + offset * across_z)
    })
}

/// Knot `index`'s cover across the bore's section (`ROAD-61` round R8f): the least height of
/// the ground above the knot on its section lattice ([`section_lattice`]), the source terrain
/// or, where a grounded road's deck of `decks` lies over a lattice point lower than it, that
/// deck (round R8h: a tunnel drawn under a road keeps its cover under the road as built). The
/// section is level, so every lattice point stands at the knot's height.
/// `O(half_width_m / TUNNEL_LATTICE_M x deck profile points)`, allocation-free.
pub(super) fn section_cover_m(
    terrain: &TerrainSystem,
    decks: &[RoadDeck],
    points: &[Vector3],
    index: usize,
    half_width_m: f32,
) -> f32 {
    let knot = points[index];
    section_lattice(points, index, half_width_m)
        .map(|(x, z)| {
            let ground = cover_m(terrain, Vector3::new(x, 0.0, z));
            decks
                .iter()
                .filter_map(|deck| deck.height_at(x, z))
                .fold(ground, f32::min)
                - knot.y
        })
        .fold(f32::INFINITY, f32::min)
}

/// The least height the deck of a grounded road drawn along `points`, `half_width_m` to either
/// side, keeps above the deck of a tunnel bore of `graph` it passes over (`ROAD-61` round R8h):
/// over every lattice point of a tunnel knot's section ([`section_cover_m`]'s, across the
/// lining's shell) that the road's footprint covers. A portal row's knot is left out: the
/// approach's open trench meets the bore there, and a road at the mouth stands in the trench,
/// which the trench rules own ([`junction_in_trench`]); the bore's next knot, one lattice step
/// in, is checked. `None` when it passes over no bore. The crossing is grade-separated, no node
/// and no split, while this stays at least [`ROAD_OVER_TUNNEL_M`]: the cover the bore keeps
/// under the ground and the road's own depth. Reads the local excerpt only: `O(log E + T x ceil(W / TUNNEL_LATTICE_M) x stroke
/// points)` for the `T` knots of the tunnels met within reach of the road.
pub(super) fn deck_over_tunnels_m(
    points: &[Vector3],
    half_width_m: f32,
    graph: &RegionGraph,
) -> Option<f32> {
    let deck = RoadDeck::new(points, half_width_m);
    let ((x0, z0), (x1, z1)) = xz_bounds(points);
    let mut least: Option<f32> = None;
    let mut tunnels = edges_near(graph, points, half_width_m);
    tunnels.sort_unstable();
    tunnels.dedup();
    for id in tunnels {
        let edge = graph.edge(id);
        let knots = edge_profile(edge);
        if !is_surface_edge(edge) || edge.class != EdgeClass::Tunnel || knots.len() < 2 {
            continue;
        }
        let section_m = visual_roadbed_half_width_m(edge) + TUNNEL_LINING_THICKNESS_M as f32;
        let reach = half_width_m + section_m;
        let portal = |node: u32| {
            let node = graph.get_valid_node(node);
            (node as usize) < graph.node_adjacency_count()
                && graph.node_adjacency(node).iter().any(|&other| {
                    let other = graph.edge(other);
                    is_surface_edge(other) && other.class == EdgeClass::Standard
                })
        };
        let last = knots.len() - 1;
        for (index, knot) in knots.iter().enumerate() {
            if (index == 0 && portal(edge.start_node)) || (index == last && portal(edge.end_node)) {
                continue;
            }
            if knot.x < x0 - reach
                || knot.x > x1 + reach
                || knot.z < z0 - reach
                || knot.z > z1 + reach
            {
                continue;
            }
            for (x, z) in section_lattice(knots, index, section_m) {
                if let Some(height) = deck.height_at(x, z) {
                    let above = height - knot.y;
                    least = Some(least.map_or(above, |least| least.min(above)));
                }
            }
        }
    }
    least
}

/// The bore of a prepared tunnel stroke whose section reaches `half_width_m` to either side
/// ([`tunnel_section_half_width_m`]): the indices of its first and last knot with the bore
/// clearance of cover across the section ([`section_cover_m`], under the ground or the
/// `decks` of the grounded roads over it), every knot between them covered too and each open
/// approach at least [`TUNNEL_MIN_APPROACH_M`] long.
/// `O(knots x half_width_m / TUNNEL_LATTICE_M x deck profile points)`, allocation-free.
pub(super) fn tunnel_bore(
    points: &[Vector3],
    half_width_m: f32,
    terrain: &TerrainSystem,
    decks: &[RoadDeck],
) -> Result<[usize; 2], BoreRefusal> {
    let cover_m = |index: usize| section_cover_m(terrain, decks, points, index, half_width_m);
    let covered = |index: usize| cover_m(index) >= TUNNEL_COVER_M;
    let Some(first) = (0..points.len()).find(|&index| covered(index)) else {
        let (knot, cover_m) = (0..points.len())
            .map(|index| (index, cover_m(index)))
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
        .map(|index| (index, cover_m(index)))
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

/// How far below the source terrain ([`cover_m`], the cover the bore check reads) `node` of
/// `graph` stands when it is a junction in a tunnel approach's trench (`ROAD-61` round R8f): three
/// or more surface arms, one of them a tunnel or an approach (a standard edge whose far node has
/// only it and a tunnel, the span sharing the mouth that the roadbed retains and the terrain
/// walls, R8e), and the node below the ground. The node's piece is not retained, so the ground
/// would be graded down to it in a cut and step against the walled trench at the piece's
/// boundary. `None` for any other node. The far nodes of `node`'s arms must have complete
/// incidence, as the validation graph gives an affected node's. `O(degree^2)`.
pub(super) fn junction_in_trench(
    graph: &RegionGraph,
    node: u32,
    terrain: &TerrainSystem,
) -> Option<f32> {
    let node = graph.get_valid_node(node);
    let arms = |node: u32| {
        let edges = if (node as usize) < graph.node_adjacency_count() {
            graph.node_adjacency(node)
        } else {
            &[]
        };
        edges
            .iter()
            .copied()
            .filter(|&edge_idx| is_surface_edge(graph.edge(edge_idx)))
    };
    if arms(node).count() < 3 {
        return None;
    }
    let approach = |edge_idx: usize| {
        let edge = graph.edge(edge_idx);
        let ends = [edge.start_node, edge.end_node].map(|end| graph.get_valid_node(end));
        let far = if ends[0] == node { ends[1] } else { ends[0] };
        let mut others = arms(far).filter(|&other| other != edge_idx);
        matches!((others.next(), others.next()),
            (Some(other), None) if graph.edge(other).class == EdgeClass::Tunnel)
    };
    let in_approach = arms(node).any(|edge_idx| match graph.edge(edge_idx).class {
        EdgeClass::Tunnel => true,
        EdgeClass::Standard => approach(edge_idx),
        EdgeClass::Bridge => false,
    });
    let depth = cover_m(terrain, graph.node(node).pos);
    (in_approach && depth > 0.0).then_some(depth)
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
