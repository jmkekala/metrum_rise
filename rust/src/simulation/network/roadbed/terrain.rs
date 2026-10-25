// SPDX-License-Identifier: GPL-2.0-only

//! Terrain patch loops (`ROAD-68-D`): the grounded footprint inside an XZ rectangle as closed
//! loops, road on the left, from the outer chains of the pieces it touches. Chains join at their
//! bit-identical mouth corners; the generic [`crate::simulation::terrain::rect_loops`] cuts the
//! joined paths to the rectangle and joins their runs along its boundary, and orientation tells
//! holes from outer boundaries. Bridge and tunnel spans publish no chains (`ROAD-68-F`).
//! `O(S log S)` for `S` chain segments.

use super::query::PieceId;
use super::system::{Piece, PublishedPiece};
use super::{PieceMesh, RoadVec3, RoadbedSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::rect_loops::{RectLoop, clip_path, join_runs, within};
use std::collections::BTreeMap;

/// One outer-chain segment of a published piece, from chain vertex `segment` to the next.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChainSegment {
    /// The piece that owns the chain.
    pub piece: PieceId,
    /// Index into the piece's [`super::PieceMesh::outer_chains`].
    pub chain: u32,
    /// Segment index along the chain.
    pub segment: u32,
}

/// One closed boundary of the grounded footprint inside a rectangle, road on its left: chain
/// vertices bitwise, chain crossings of the rectangle and rectangle corners, each edge with the
/// chain segment it lies on (`None` along the rectangle). A hole (a closed block) runs clockwise.
pub type TerrainLoop = RectLoop<ChainSegment>;

/// Why the loops of a rectangle cannot be built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainLoopError {
    /// A chain inside the rectangle ends (or starts) where no published grounded chain
    /// continues: a failed node's gap, a node without a piece (bridge or tunnel terminal), or a
    /// bridge or tunnel span (`ROAD-68-F`).
    OpenEnd {
        /// The piece whose chain is open.
        piece: PieceId,
        /// Chain index in that piece.
        chain: u32,
    },
}

impl TerrainLoopError {
    /// Stable reason key for diagnostics and the terrain clip status.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenEnd { .. } => "roadbed_chain_open_end",
        }
    }
}

/// Where compiles changed what [`terrain_loops`] reads (`ROAD-68-E`), from
/// [`RoadbedSystem::take_terrain_changes`]: the terrain an edit must rebuild.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainChanges {
    /// XZ bounds `[min_x, min_z, max_x, max_z]` of every old and new piece the compiles
    /// replaced, added or removed, in compile order: whatever building-site grading reads from
    /// the roads (surface heights and the graph edges they belong to) changed only inside them.
    pub pieces: Vec<[f64; 4]>,
    /// XZ bounds of every outer-chain segment the old pieces had and the new ones lack, or the
    /// reverse, in segment order: the loops of a rectangle that meets none of them are equal
    /// before and after, bit for bit, labels included (the CDT input numbers its chain runs by
    /// geometry, `road_terrain_loops`). A split trunk's far half keeps its segments, so only the
    /// junction's surroundings appear here.
    pub chains: Vec<[f64; 4]>,
    /// The changed segments of [`Self::chains`] themselves, endpoints with heights, in the same
    /// order (`TERRAIN-07-A`): the graded terrain's dirty tiles follow each segment's own reach,
    /// and a removed segment's heights are gone from the roadbed. A one-vertex chain gives a
    /// segment from the vertex to itself.
    pub segments: Vec<[RoadVec3; 2]>,
}

// An outer-chain segment as `terrain_loops` reads it: its endpoints' coordinate bits, whether
// its piece is a node, and whether it starts or ends its chain (where chains join). A
// one-vertex chain is a segment from the vertex to itself that starts and ends its chain.
type ChainSegmentKey = ([u64; 6], bool, bool, bool);

// Net count of each grounded chain segment over the pieces published (`+1`) and replaced
// (`-1`) since the last take, so a piece rebuilt unchanged, or a clear followed by the same
// pieces, cancels out (`ROAD-68-E`). Structure spans' chains never reach the loops and are not
// counted.
#[derive(Clone, Debug, Default)]
pub(super) struct ChainChanges(BTreeMap<ChainSegmentKey, i64>);

impl ChainChanges {
    // `O(V log S)` for the piece's chain vertices `V` and the counted segments `S`.
    pub(super) fn count(&mut self, piece: &PublishedPiece, sign: i64) {
        let (mesh, node) = match &piece.piece {
            Piece::Span(span) if !span.grounded() => return,
            Piece::Span(span) => (span.mesh(), false),
            Piece::Node(node) => (node.mesh(), true),
        };
        for chain in mesh.outer_chains() {
            let vertices = chain.vertices();
            let segments = vertices.len().saturating_sub(1).max(1);
            for index in 0..segments {
                let start = vertices[index];
                let end = vertices.get(index + 1).copied().unwrap_or(start);
                let key = (
                    segment_bits(mesh, start, end),
                    node,
                    index == 0,
                    index + 1 == segments,
                );
                let count = self.0.entry(key).or_insert(0);
                *count += sign;
                if *count == 0 {
                    self.0.remove(&key);
                }
            }
        }
    }

    // Every segment whose net count is not zero, with its XZ bounds; taking them clears the
    // counts.
    pub(super) fn take(&mut self) -> (Vec<[f64; 4]>, Vec<[RoadVec3; 2]>) {
        std::mem::take(&mut self.0)
            .into_keys()
            .map(|(bits, ..)| {
                let [ax, ay, az, bx, by, bz] = bits.map(f64::from_bits);
                (
                    [ax.min(bx), az.min(bz), ax.max(bx), az.max(bz)],
                    [RoadVec3::new(ax, ay, az), RoadVec3::new(bx, by, bz)],
                )
            })
            .unzip()
    }
}

fn segment_bits(mesh: &PieceMesh, start: u32, end: u32) -> [u64; 6] {
    let [a, b] = [start, end].map(|vertex| mesh.positions()[vertex as usize]);
    [a.x, a.y, a.z, b.x, b.y, b.z].map(f64::to_bits)
}

// One piece chain: its vertex range in the gathered points and its owner.
struct Chain {
    piece: PieceId,
    index: u32,
    start: usize,
    end: usize,
}

/// The grounded footprint's loops inside `rect = [min_x, min_z, max_x, max_z]`, in the order of
/// the lowest piece each loop meets (ascending [`PieceId`]).
pub fn terrain_loops(
    roadbed: &RoadbedSystem,
    graph: &RegionGraph,
    rect: [f64; 4],
) -> Result<Vec<TerrainLoop>, TerrainLoopError> {
    let mut points = Vec::new();
    let mut chains = Vec::new();
    for (id, _) in roadbed.pieces_in_bounds(rect) {
        if let PieceId::Span(edge) = id
            && graph.edge(edge).class != EdgeClass::Standard
        {
            continue;
        }
        let mesh = roadbed.pieces[&id].mesh();
        for (index, chain) in mesh.outer_chains().iter().enumerate() {
            let start = points.len();
            points.extend(
                chain
                    .vertices()
                    .iter()
                    .map(|&v| mesh.positions()[v as usize]),
            );
            chains.push(Chain {
                piece: id,
                index: index as u32,
                start,
                end: points.len(),
            });
        }
    }
    let key = |point: super::RoadVec3| (point.x.to_bits(), point.z.to_bits());
    let open = |record: &Chain| TerrainLoopError::OpenEnd {
        piece: record.piece,
        chain: record.index,
    };
    let starts: BTreeMap<_, usize> = (0..chains.len())
        .map(|chain| (key(points[chains[chain].start]), chain))
        .collect();
    let mut next = vec![None; chains.len()];
    let mut has_prev = vec![false; chains.len()];
    for (chain, record) in chains.iter().enumerate() {
        let last = points[record.end - 1];
        if let Some(&successor) = starts.get(&key(last)) {
            next[chain] = Some(successor);
            has_prev[successor] = true;
        } else if within(rect, last) {
            return Err(open(record));
        }
    }
    if let Some(record) = (0..chains.len())
        .find(|&chain| !has_prev[chain] && within(rect, points[chains[chain].start]))
        .map(|chain| &chains[chain])
    {
        return Err(open(record));
    }

    let mut loops = Vec::new();
    let mut runs = Vec::new();
    let mut visited = vec![false; chains.len()];
    // Open paths first (they start outside the rectangle), then the cycles left.
    let heads = (0..chains.len())
        .filter(|&chain| !has_prev[chain])
        .chain(0..chains.len());
    let mut path = Vec::new();
    for head in heads {
        if visited[head] {
            continue;
        }
        path.clear();
        let mut chain = Some(head);
        while let Some(current) = chain.filter(|&current| !visited[current]) {
            visited[current] = true;
            let record = &chains[current];
            // Joined chains share their mouth corner: it starts the next chain's first segment.
            path.pop();
            path.extend((record.start..record.end).map(|vertex| {
                let segment = (vertex + 1 < record.end).then(|| ChainSegment {
                    piece: record.piece,
                    chain: record.index,
                    segment: (vertex - record.start) as u32,
                });
                (points[vertex], segment)
            }));
            chain = next[current];
        }
        let closed = chain.is_some();
        if closed {
            path.pop();
        }
        clip_path(&mut path, closed, rect, &mut runs, &mut loops);
    }
    join_runs(runs, rect, &mut loops);
    Ok(loops)
}

#[cfg(test)]
mod tests;
