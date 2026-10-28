// SPDX-License-Identifier: GPL-2.0-only

//! Local change accounting for the roadbed inputs consumed by graded terrain.

use super::system::{Piece, PublishedPiece};
use super::{PieceMesh, RoadVec3};
use std::collections::BTreeMap;

/// Where compiles changed the graded terrain inputs, from
/// [`RoadbedSystem::take_terrain_changes`]: the terrain an edit must rebuild.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainChanges {
    /// XZ bounds `[min_x, min_z, max_x, max_z]` of every old and new piece the compiles
    /// replaced, added or removed, in compile order: whatever building-site grading reads from
    /// the roads (surface heights and the graph edges they belong to) changed only inside them.
    pub pieces: Vec<[f64; 4]>,
    /// XZ bounds of every outer-chain segment the old pieces had and the new ones lack, or the
    /// reverse, in segment order: the chains a rectangle that meets none of them sees are equal
    /// before and after, bit for bit. A split trunk's far half keeps its segments, so only the
    /// junction's surroundings appear here.
    pub chains: Vec<[f64; 4]>,
    /// The changed segments of [`Self::chains`] themselves, endpoints with heights, in the same
    /// order (`TERRAIN-07-A`): the graded terrain's dirty tiles follow each segment's own reach,
    /// and a removed segment's heights are gone from the roadbed. A one-vertex chain gives a
    /// segment from the vertex to itself.
    pub segments: Vec<[RoadVec3; 2]>,
    /// The bounds of [`Self::pieces`] whose piece has a portal row ([`PieceMesh::portal_rows`],
    /// `TERRAIN-07-C`), in the same order: the graded terrain's portal opening reads only those
    /// pieces' parts, so it changes only near them.
    pub portals: Vec<[f64; 4]>,
}

// An outer-chain segment as terrain grading reads it: its endpoints' coordinate bits, whether
// its piece is a node, and whether it starts or ends its chain (where chains join). A
// one-vertex chain is a segment from the vertex to itself that starts and ends its chain.
type ChainSegmentKey = ([u64; 6], bool, bool, bool);

// Net count of each grounded chain segment over the pieces published (`+1`) and replaced
// (`-1`) since the last take, so a piece rebuilt unchanged, or a clear followed by the same
// pieces, cancels out (`ROAD-68-E`). The chains of structure spans, and of node pieces where only
// structures meet, never reach the loops and are not counted.
#[derive(Clone, Debug, Default)]
pub(super) struct ChainChanges(BTreeMap<ChainSegmentKey, i64>);

impl ChainChanges {
    // `O(V log S)` for the piece's chain vertices `V` and the counted segments `S`.
    pub(super) fn count(&mut self, piece: &PublishedPiece, sign: i64) {
        if !piece.grounded() {
            return;
        }
        let (mesh, node) = match &piece.piece {
            Piece::Span(span) => (span.mesh(), false),
            Piece::Node(node) => (node.mesh(), true),
        };
        for chain in mesh.outer_chains().iter().chain(mesh.wall_runs()) {
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

#[cfg(test)]
mod tests;
