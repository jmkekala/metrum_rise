// SPDX-License-Identifier: GPL-2.0-only

//! The straight line whose curb frames a road's cells use (`ROAD-95` round R5d1): the road
//! alone, or the pieces a turn pocket split off one straight base road, joined through their
//! lane transitions and read at the base road's width. A pocket edit adds, moves or removes a
//! transition and widens one piece; the line it leaves is the base road's, so its grid keeps
//! its phase, and each piece's widened curb reads the line's frame moved by its kerb's move.

use super::GridFrame;
use super::geometry::{CellBounds, road_source_precision};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use glam::DVec2;

// A pocket at each end of a base road leaves three pieces: a pocket edge's junction end is never
// a transition (the node must offer the pocket's turn), so no longer run of pieces is built.
const MAX_PIECES: usize = 3;

/// A straight road's frontage line: its outer ends and nodes, its base width and the pieces
/// along it, in order from `first`. `O(1)` to hold; allocation-free.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrontageLine {
    /// The line's first and last centreline points, oriented along the road it was read for.
    pub(super) first: DVec2,
    pub(super) last: DVec2,
    // The f32 bits of `first` and `last` (X, Z) as the graph holds them.
    bits: [f32; 4],
    /// The nodes at `first` and `last`.
    pub(super) ends: [u32; 2],
    /// The base road's width ([`Edge::base_width_m`]).
    pub(super) width: f32,
    // The pieces from `first`, each with whether it runs against the line.
    pieces: [(usize, bool); MAX_PIECES],
    count: usize,
}

impl FrontageLine {
    /// The line of edge `id`: the straight run of pieces it forms with the pieces beyond its
    /// live lane transitions when they share one base width and one straight centreline, else
    /// the edge alone. Oriented along `id`. `None` for an edge without two points. Every road
    /// without a pocket piece at an end is its own line, bit for bit as before.
    /// `O(points of the pieces)`, at most three pieces.
    pub(super) fn of(graph: &RegionGraph, id: usize) -> Option<Self> {
        let edge = graph.get_edge(id)?;
        let points = &edge.physical_geometry;
        let (a, b) = (*points.first()?, *points.get(1).and(points.last())?);
        let alone = Self {
            first: DVec2::new(f64::from(a.x), f64::from(a.z)),
            last: DVec2::new(f64::from(b.x), f64::from(b.z)),
            bits: [a.x, a.z, b.x, b.z],
            ends: [edge.start_node, edge.end_node],
            width: edge.base_width_m(),
            pieces: [(id, false); MAX_PIECES],
            count: 1,
        };
        if !graph.is_lane_transition(edge.start_node) && !graph.is_lane_transition(edge.end_node) {
            return Some(alone);
        }
        Some(Self::chain(graph, id, edge).unwrap_or(alone))
    }

    // The run of pieces through `edge`'s lane transitions, if it is one straight base road.
    fn chain(graph: &RegionGraph, id: usize, edge: &Edge) -> Option<Self> {
        // Walked from the start node backwards (0) and from the end node forwards (1).
        let mut walked = [[(id, false); MAX_PIECES]; 2];
        let mut counts = [0; 2];
        let mut ends = [edge.start_node, edge.end_node];
        for way in 0..2 {
            let backwards = way == 0;
            let (mut node, mut current) = (ends[way], id);
            while graph.is_lane_transition(node) {
                let node_id = graph.get_valid_node(node);
                let next = graph
                    .node_adjacency(node_id)
                    .iter()
                    .copied()
                    .find(|&other| other != current && !graph.edge(other).deleted)?;
                if next == id || 1 + counts[0] + counts[1] >= MAX_PIECES {
                    return None;
                }
                let piece = graph.edge(next);
                // A piece reached at its end runs with the line when walking backwards.
                let at_end = graph.get_valid_node(piece.end_node) == node_id;
                walked[way][counts[way]] = (next, at_end != backwards);
                counts[way] += 1;
                node = if at_end {
                    piece.start_node
                } else {
                    piece.end_node
                };
                current = next;
            }
            ends[way] = node;
        }
        let mut pieces = [(id, false); MAX_PIECES];
        for (slot, piece) in walked[0][..counts[0]]
            .iter()
            .rev()
            .chain([(id, false)].iter())
            .chain(walked[1][..counts[1]].iter())
            .enumerate()
        {
            pieces[slot] = *piece;
        }
        let count = 1 + counts[0] + counts[1];
        let width = edge.base_width_m();
        let point_of = |(piece, reversed): (usize, bool), last: bool| {
            let points = &graph.edge(piece).physical_geometry;
            if reversed == last {
                points[0]
            } else {
                points[points.len() - 1]
            }
        };
        let (a, b) = (
            point_of(pieces[0], false),
            point_of(pieces[count - 1], true),
        );
        let line = Self {
            first: DVec2::new(f64::from(a.x), f64::from(a.z)),
            last: DVec2::new(f64::from(b.x), f64::from(b.z)),
            bits: [a.x, a.z, b.x, b.z],
            ends,
            width,
            pieces,
            count,
        };
        let valid = line.members().all(|piece| {
            let piece = graph.edge(piece);
            !piece.deleted
                && piece.primary_type == TransitType::Road
                && piece.class == EdgeClass::Standard
                && piece.physical_geometry.len() >= 2
                && piece.base_width_m().to_bits() == width.to_bits()
        });
        let direction = (line.last - line.first).normalize_or_zero();
        if !valid || direction.length_squared() == 0.0 {
            return None;
        }
        // One straight centreline within the straight-road source bound, as a single edge.
        let bounds = CellBounds::from_points(line.points(graph));
        let precision = road_source_precision(bounds).max(1e-4);
        line.points(graph)
            .all(|point| (point - line.first).perp_dot(direction).abs() <= precision)
            .then_some(line)
    }

    /// The pieces along the line, from `first`.
    pub(super) fn members(&self) -> impl Iterator<Item = usize> + '_ {
        self.pieces[..self.count].iter().map(|&(piece, _)| piece)
    }

    /// Whether `edge` is a piece of this line.
    pub(super) fn contains(&self, edge: usize) -> bool {
        self.members().any(|piece| piece == edge)
    }

    /// The lowest piece id: the line's name among a node's arms. A pocket edit keeps its base
    /// road's id on one of its pieces, so the name and a junction's arm order survive it.
    pub(super) fn key(&self) -> usize {
        self.members().min().unwrap_or(usize::MAX)
    }

    /// Every centreline point of the pieces, from `first` to `last`.
    pub(super) fn points<'a>(&'a self, graph: &'a RegionGraph) -> impl Iterator<Item = DVec2> + 'a {
        self.pieces[..self.count]
            .iter()
            .flat_map(move |&(piece, reversed)| {
                let points = &graph.edge(piece).physical_geometry;
                let forward = (!reversed).then(|| points.iter());
                let backward = reversed.then(|| points.iter().rev());
                forward
                    .into_iter()
                    .flatten()
                    .chain(backward.into_iter().flatten())
            })
            .map(|point| DVec2::new(f64::from(point.x), f64::from(point.z)))
    }

    /// The exact source address ([`source_address`]): first X/Z, last X/Z and the base width.
    pub(super) fn source(&self) -> [u32; 5] {
        let [ax, az, bx, bz] = self.bits;
        source_address([ax, az, bx, bz, self.width])
    }
}

/// A straight road's exact source address from its first X/Z, last X/Z and width: their f32
/// bits. SQLite REAL canonicalizes signed zero, so both encodings of zero name the same point.
pub(super) fn source_address(values: [f32; 5]) -> [u32; 5] {
    values.map(|v| if v == 0.0 { 0 } else { v.to_bits() })
}

/// How far `edge`'s curb on each directed side lies outside its base road's, indexed as its
/// curb frames are (side -1 and +1 of `straight_frame` about the edge's own direction): a kerb
/// pocket's lane on its side, nothing on the other or without a pocket (`ROAD-95` round R5d1).
/// `O(1)`.
pub(super) fn kerb_moves_m(edge: &Edge) -> [f64; 2] {
    if edge.pocket.is_none() {
        return [0.0; 2];
    }
    // The carriageway's axis lies `kerb_shift_m` right of the centreline (its forward normal),
    // which is the frames' directed side 0.
    let widened = (f64::from(edge.width) - f64::from(edge.base_width_m())) * 0.5;
    let shift = f64::from(edge.kerb_shift_m());
    [widened + shift, widened - shift]
}

/// `frames`, a line's directed left and right curb frames, as edge `id`'s curbs read them: each
/// moved out by its kerb's move ([`kerb_moves_m`]); unchanged without a pocket. `O(1)`.
pub(super) fn frames_for_edge(
    graph: &RegionGraph,
    id: usize,
    frames: [GridFrame; 2],
) -> [GridFrame; 2] {
    let edge = graph.edge(id);
    let moves = kerb_moves_m(edge);
    if moves == [0.0; 2] {
        return frames;
    }
    let points = &edge.physical_geometry;
    let (a, b) = (points[0], points[points.len() - 1]);
    let delta = DVec2::new(f64::from(b.x - a.x), f64::from(b.z - a.z));
    let (u, v) = frames[0].axes();
    let along = usize::from(delta.dot(v).abs() > delta.dot(u).abs());
    let axes = [u, v];
    let tangent = axes[along] * delta.dot(axes[along]).signum();
    let mut moved = frames;
    for (side, frame) in moved.iter_mut().enumerate() {
        if moves[side] != 0.0 {
            // The side's outward normal, as `straight_frame` builds it, along the lateral axis.
            let normal = DVec2::new(tangent.y, -tangent.x) * if side == 0 { -1.0 } else { 1.0 };
            let sign = normal.dot(axes[1 - along]).signum();
            *frame = frame.moved(1 - along, sign * moves[side]);
        }
    }
    moved
}

/// The distance from `edge`'s centreline to its curb line on the side `normal` points to (a unit
/// vector across the edge): half its width and the sidewalk, plus or minus a pocket edge's kerb
/// shift (`ROAD-95`, D17). Bit for bit `width / 2 + SIDEWALK_WIDTH` without a pocket. `O(1)`.
pub(super) fn kerb_offset_m(edge: &Edge, normal: DVec2) -> f64 {
    let half = f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
    let shift = f64::from(edge.kerb_shift_m());
    if shift == 0.0 {
        return half;
    }
    // The shift points along the edge's forward normal, as `Edge::carriageway_axis` reads it.
    let points = &edge.physical_geometry;
    let (a, b) = (points[0], points[points.len() - 1]);
    let right = DVec2::new(f64::from(a.z - b.z), f64::from(b.x - a.x));
    half + shift * right.dot(normal).signum()
}
