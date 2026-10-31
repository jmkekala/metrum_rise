// SPDX-License-Identifier: GPL-2.0-only

//! The closed tunnel (`ROAD-61`, round R8): a buried piece's lining, its walls standing on the
//! deck's outer chains and its ceiling over them, the end wall of a bore that ends underground,
//! and the headwall at a portal row.
//!
//! Every vertex is a mesh vertex or a function of one ([`lining_top`]), so pieces that share a
//! mouth share its lining edges bitwise: the walls' bottom edges are the outer chains, a mouth's
//! ceiling edge joins the lining tops above its two corners, and a portal's headwall takes the
//! row and those tops as the edge of its opening. The section is a box: walls straight up from
//! the deck's outer edges, a flat ceiling between them (sized from the span's own sections). No
//! terrain height is read. Triangles are wound so that `-(b - a) x (c - a)` faces into the bore,
//! and out of the portal for a headwall.

use super::geometry::triangulate_constrained_rings_xz;
use super::mesh::{OuterChain, SKIRT_DEPTH_M};
use super::{RoadVec2, RoadVec3};

/// The lining's clear height (`ROAD-61`): its walls stand this far above the deck's outer edges,
/// where its ceiling spans.
pub const TUNNEL_LINING_HEIGHT_M: f64 = 4.5;
/// A portal headwall's height above its row's corners (`ROAD-61`): the bore's clearance of cover
/// at the portal (5 m) and half a metre of parapet, so the hill above a portal stands behind it.
pub const HEADWALL_HEIGHT_M: f64 = 5.5;
/// How far a portal headwall reaches beyond each corner of its row along the row's line
/// (`ROAD-61`): where the approach's cut, rising at the 1:2 tie-in slope from the row's corner,
/// reaches the headwall's top.
pub const HEADWALL_WING_M: f64 = 2.0 * HEADWALL_HEIGHT_M;

/// The lining top above an outer-chain vertex: the one function every piece takes it from.
pub(super) fn lining_top(vertex: RoadVec3) -> RoadVec3 {
    RoadVec3::new(vertex.x, vertex.y + TUNNEL_LINING_HEIGHT_M, vertex.z)
}

/// A buried piece's lining and its headwalls: the vertices it adds after the piece's positions
/// (the lining tops, the headwalls' outer corners), and its triangles over the piece's positions
/// followed by them, the lining's first.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Lining {
    pub(super) positions: Vec<RoadVec3>,
    pub(super) triangles: Vec<[u32; 3]>,
    // Index in `triangles` of the first headwall triangle.
    pub(super) headwalls: usize,
}

/// Why a node piece's lining has no construction: the ring of its lining tops bounds no
/// simple polygon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LiningError;

impl Lining {
    // The index of the next added vertex, after the piece's `count` positions.
    fn push(&mut self, count: usize, point: RoadVec3) -> u32 {
        self.positions.push(point);
        (count + self.positions.len() - 1) as u32
    }

    // The walls along `chains`, each chain vertex's top pushed once, in chain order; returns
    // per chain the index of its first top (the rest follow).
    fn walls(&mut self, positions: &[RoadVec3], chains: &[OuterChain]) -> Vec<u32> {
        let count = positions.len();
        let mut firsts = Vec::with_capacity(chains.len());
        for chain in chains {
            let vertices = chain.vertices();
            let first = (count + self.positions.len()) as u32;
            firsts.push(first);
            for &vertex in vertices {
                self.push(count, lining_top(positions[vertex as usize]));
            }
            // The road lies left of the chain: these windings face it.
            for (k, pair) in vertices.windows(2).enumerate() {
                let (a, b) = (pair[0], pair[1]);
                let (top_a, top_b) = (first + k as u32, first + k as u32 + 1);
                self.triangles.push([a, top_b, b]);
                self.triangles.push([a, top_a, top_b]);
            }
        }
        firsts
    }

    // A face in the vertical plane of the end row `row` (mesh vertex indices, left to right):
    // `rings` (the outer first, then holes) over `(index, point)` pairs, triangulated in that
    // plane and wound to face `facing` in XZ. A span's end rows always bound one, as the free
    // end's cap below them does (`end_cap`): rows stand above the bottoms and below the tops.
    // `O(n log n + n^2)` for the `n` ring points.
    fn plane_face(
        &mut self,
        row: (RoadVec3, RoadVec3),
        rings: &[&[(u32, RoadVec3)]],
        facing: RoadVec2,
    ) {
        let (left, right) = row;
        let along = RoadVec2::new(right.x - left.x, right.z - left.z).normalize();
        let plane = |point: RoadVec3| {
            let lateral = (point.x - left.x) * along.x + (point.z - left.z) * along.y;
            RoadVec3::new(lateral, 0.0, point.y)
        };
        let planar: Vec<Vec<RoadVec3>> = rings
            .iter()
            .map(|ring| ring.iter().map(|&(_, point)| plane(point)).collect())
            .collect();
        let Some(triangles) = triangulate_constrained_rings_xz(&planar) else {
            debug_assert!(false, "an end row bounds its end face");
            return;
        };
        let flat: Vec<(u32, RoadVec3)> = rings
            .iter()
            .zip(&planar)
            .flat_map(|(ring, planar)| ring.iter().zip(planar).map(|(&(index, _), &p)| (index, p)))
            .collect();
        // A triangle counter-clockwise in the plane faces `(along.y, -along.x)` in XZ.
        let flip = RoadVec2::new(along.y, -along.x).dot(facing) <= 0.0;
        for triangle in triangles {
            let [a, b, c] = triangle.map(|point| {
                flat.iter()
                    .find(|(_, p)| *p == point)
                    .map(|&(index, _)| index)
                    .expect("a face vertex is a ring vertex")
            });
            let [pa, pb, pc] = triangle;
            let counter_clockwise =
                (pb.x - pa.x) * (pc.z - pa.z) - (pb.z - pa.z) * (pc.x - pa.x) > 0.0;
            self.triangles.push(if counter_clockwise != flip {
                [a, b, c]
            } else {
                [a, c, b]
            });
        }
    }
}

/// How a lined span closes one end row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LiningEnd {
    /// Another buried piece continues the lining from the row.
    Open,
    /// The bore ends underground: an end wall closes the row up to the ceiling.
    Closed,
    /// A portal row: the headwall stands in the row's plane, the lining's mouth open in it.
    Portal,
}

/// The lining of a tunnel span: walls along its two outer chains (the first forward along slot
/// 0, the second back along the last slot, as `build_span_mesh` builds them), the ceiling between
/// the two tops of each row, and each end row closed as `ends` says. `rows` section rows of
/// `stride` vertices. `O(rows + stride log stride)`; allocates only the lining.
pub(super) fn span_lining(
    positions: &[RoadVec3],
    (rows, stride): (usize, usize),
    chains: &[OuterChain],
    ends: [LiningEnd; 2],
) -> Lining {
    let mut lining = Lining::default();
    let firsts = lining.walls(positions, chains);
    // Row `r`'s left top is the first chain's `r`-th, its right top the second chain's
    // `rows - 1 - r`-th.
    let left_top = |row: usize| firsts[0] + row as u32;
    let right_top = |row: usize| firsts[1] + (rows - 1 - row) as u32;
    for row in 0..rows - 1 {
        let (a, b) = (left_top(row), right_top(row));
        let (c, d) = (left_top(row + 1), right_top(row + 1));
        // Facing down, from the left top across to the right.
        lining.triangles.push([a, b, d]);
        lining.triangles.push([a, d, c]);
    }
    let mut portals = Vec::new();
    for (end, closure) in ends.into_iter().enumerate() {
        let (row, inner) = if end == 0 {
            (0, 1)
        } else {
            (rows - 1, rows - 2)
        };
        let first = row * stride;
        let tops = [left_top(row), right_top(row)]
            .map(|index| (index, lining.positions[index as usize - positions.len()]));
        let edge = (positions[first], positions[first + stride - 1]);
        let inward = positions[inner * stride] - positions[first];
        let outward = RoadVec2::new(-inward.x, -inward.z);
        match closure {
            LiningEnd::Open => {}
            // Facing into the bore.
            LiningEnd::Closed => {
                let mut ring: Vec<(u32, RoadVec3)> = (first..first + stride)
                    .map(|vertex| (vertex as u32, positions[vertex]))
                    .collect();
                ring.extend([tops[1], tops[0]]);
                lining.plane_face(edge, &[&ring], -outward);
            }
            LiningEnd::Portal => {
                let corners = [first as u32, (first + stride - 1) as u32];
                portals.push((corners, tops, edge, outward));
            }
        }
    }
    lining.headwalls = lining.triangles.len();
    for (corners, tops, edge, outward) in portals {
        headwall(&mut lining, positions.len(), corners, tops, edge, outward);
    }
    lining
}

/// The lining of a node piece where only tunnels meet: walls along its outer chains, which run
/// from mouth corner to mouth corner around it with the road on their left, and the ceiling over
/// the ring of their tops, each mouth's ceiling edge the straight edge between its corners' tops
/// as the span's. `O(V log V + V^2)` for the `V` chain vertices; allocates only the lining.
pub(super) fn node_lining(
    positions: &[RoadVec3],
    chains: &[OuterChain],
) -> Result<Lining, LiningError> {
    let mut lining = Lining::default();
    let firsts = lining.walls(positions, chains);
    let count = positions.len();
    let ring: Vec<(u32, RoadVec3)> = chains
        .iter()
        .zip(&firsts)
        .flat_map(|(chain, &first)| (0..chain.vertices().len() as u32).map(move |k| first + k))
        .map(|index| (index, lining.positions[index as usize - count]))
        .collect();
    let points: Vec<RoadVec3> = ring.iter().map(|&(_, point)| point).collect();
    let triangles = triangulate_constrained_rings_xz(&[&points]).ok_or(LiningError)?;
    for triangle in triangles {
        let [a, b, c] = triangle.map(|point| {
            ring.iter()
                .find(|(_, p)| *p == point)
                .map(|&(index, _)| index)
                .expect("a ceiling vertex is a ring vertex")
        });
        let [pa, pb, pc] = triangle;
        // Counter-clockwise in XZ faces up: the ceiling takes the other winding.
        let up = (pb.x - pa.x) * (pc.z - pa.z) - (pb.z - pa.z) * (pc.x - pa.x) > 0.0;
        lining
            .triangles
            .push(if up { [a, c, b] } else { [a, b, c] });
    }
    lining.headwalls = lining.triangles.len();
    Ok(lining)
}

// The headwall at a portal row (its corners `edge` and the lining tops `tops` above them; the
// piece has `count` positions): in the row's vertical plane, from the skirt depth below the
// corners up to `HEADWALL_HEIGHT_M` above them and `HEADWALL_WING_M` beyond them along the row's
// line, with the lining's mouth open in it from below: the polygon runs along its bottom to below
// the left corner, up to it, up the left wall's end edge, across the ceiling's, down the right
// wall's, down below the right corner and along the bottom again, so it meets the lining edge for
// edge and leaves the deck's row to the deck and the piece beyond it. Facing `outward`, out of
// the tunnel.
fn headwall(
    lining: &mut Lining,
    count: usize,
    [left_corner, right_corner]: [u32; 2],
    tops: [(u32, RoadVec3); 2],
    (left, right): (RoadVec3, RoadVec3),
    outward: RoadVec2,
) {
    let along = RoadVec2::new(right.x - left.x, right.z - left.z).normalize();
    let at = |corner: RoadVec3, beyond: f64, height: f64| {
        RoadVec3::new(
            corner.x + beyond * along.x,
            corner.y + height,
            corner.z + beyond * along.y,
        )
    };
    let points = [
        at(left, -HEADWALL_WING_M, HEADWALL_HEIGHT_M),
        at(left, -HEADWALL_WING_M, -SKIRT_DEPTH_M),
        at(left, 0.0, -SKIRT_DEPTH_M),
    ];
    let mut ring: Vec<(u32, RoadVec3)> = points
        .into_iter()
        .map(|point| (lining.push(count, point), point))
        .collect();
    ring.extend([(left_corner, left), tops[0], tops[1], (right_corner, right)]);
    for point in [
        at(right, 0.0, -SKIRT_DEPTH_M),
        at(right, HEADWALL_WING_M, -SKIRT_DEPTH_M),
        at(right, HEADWALL_WING_M, HEADWALL_HEIGHT_M),
    ] {
        ring.push((lining.push(count, point), point));
    }
    lining.plane_face((left, right), &[&ring], outward);
}
