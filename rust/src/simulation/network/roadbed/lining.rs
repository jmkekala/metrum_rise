// SPDX-License-Identifier: GPL-2.0-only

//! The closed tunnel (`ROAD-61`, round R8): a buried piece's lining, its walls standing on the
//! deck's outer chains and its ceiling over them, the end wall of a bore that ends underground,
//! and the headwall at a portal row.
//!
//! Every vertex is a mesh vertex or a function of one ([`lining_top`], [`shell_foot`]), so pieces
//! that share a mouth share its lining edges bitwise: the walls' bottom edges are the outer
//! chains, a mouth's ceiling edge joins the lining tops above its two corners, and a portal's
//! headwall takes the row and those tops as the edge of its opening. The section is a box: walls
//! straight up from the deck's outer edges, a flat ceiling between them (sized from the span's own
//! sections). No terrain height is read. Triangles are wound so that `-(b - a) x (c - a)` faces
//! into the bore, and out of the portal for a headwall.
//!
//! Round R8b: the lining has a thickness. Its shell, outer walls `TUNNEL_LINING_THICKNESS_M`
//! outside the walls and a roof the thickness above the ceiling, is buried in the hill; it closes
//! the lining against the sun as a solid wall does. Thin walls standing on the deck left a line of sun
//! along their feet: a shadow map keeps a receiver lit within its bias of the nearest caster along
//! the sun's ray, and beside a wall's foot that caster is the wall itself, centimetres away. The
//! shell puts the nearest caster at least the thickness away.

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
/// reaches the headwall's height, and where a wing's top, falling at that slope from it, reaches
/// the corner's.
pub const HEADWALL_WING_M: f64 = 2.0 * HEADWALL_HEIGHT_M;
/// The fall of a headwall wing's top along the wing, metres per metre (`ROAD-61` round R8c, the
/// user's decision): the terrain's 1:2 tie-in slope, at which the bake grades the ground to it.
pub const HEADWALL_WING_SLOPE: f64 = 0.5;
/// The lining's thickness (`ROAD-61` round R8b): its shell's outer walls stand this far outside
/// its walls, beyond any shadow bias the sun's shadow map applies near a wall's foot.
pub const TUNNEL_LINING_THICKNESS_M: f64 = 0.5;

/// The lining top above an outer-chain vertex: the one function every piece takes it from.
pub(super) fn lining_top(vertex: RoadVec3) -> RoadVec3 {
    RoadVec3::new(vertex.x, vertex.y + TUNNEL_LINING_HEIGHT_M, vertex.z)
}

/// The shell top above a shell foot: the lining's height and its thickness above it.
fn shell_top(foot: RoadVec3) -> RoadVec3 {
    RoadVec3::new(
        foot.x,
        foot.y + TUNNEL_LINING_HEIGHT_M + TUNNEL_LINING_THICKNESS_M,
        foot.z,
    )
}

/// The shell's foot outside a mouth corner `vertex`, away from the row's other corner `other`:
/// the one function every piece sharing the row takes it from, so their shells meet bitwise.
pub(super) fn shell_foot(vertex: RoadVec3, other: RoadVec3) -> RoadVec3 {
    let away = RoadVec2::new(vertex.x - other.x, vertex.z - other.z).normalize();
    RoadVec3::new(
        vertex.x + TUNNEL_LINING_THICKNESS_M * away.x,
        vertex.y,
        vertex.z + TUNNEL_LINING_THICKNESS_M * away.y,
    )
}

/// A portal headwall's wing beside the row corner `corner`, away from the row's other corner
/// `other` (`ROAD-61` round R8c): its top edge, from `HEADWALL_HEIGHT_M` above the corner down
/// at `HEADWALL_WING_SLOPE` to its tip `HEADWALL_WING_M` beyond it, level with the corner. The
/// one function the headwall and the terrain bake take a wing from, so the ground the bake
/// grades to a wing meets its top bitwise. No terrain height is read.
pub fn headwall_wing(corner: RoadVec3, other: RoadVec3) -> [RoadVec3; 2] {
    let away = RoadVec2::new(corner.x - other.x, corner.z - other.z).normalize();
    let at = |beyond: f64| {
        RoadVec3::new(
            corner.x + beyond * away.x,
            corner.y + HEADWALL_HEIGHT_M - HEADWALL_WING_SLOPE * beyond,
            corner.z + beyond * away.y,
        )
    };
    [at(0.0), at(HEADWALL_WING_M)]
}

/// Which part of the closed tunnel a lining triangle belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiningPart {
    /// The walls and the ceiling, facing into the bore.
    Bore,
    /// The end wall of a bore that ends underground, facing into it.
    EndWall,
    /// A portal's headwall, facing out of the tunnel.
    Headwall,
    /// The shell (round R8b): the outer walls and the roof over them, buried.
    Shell,
}

/// A buried piece's lining and its headwalls: the vertices it adds after the piece's positions
/// (the lining tops, the headwalls' outer corners, the shell's feet and tops), and its triangles
/// over the piece's positions followed by them: the bore's, the end walls', the headwalls', then
/// the shell's.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Lining {
    pub(super) positions: Vec<RoadVec3>,
    // Per added vertex the mesh vertex it stands on (a lining top's or a shell vertex's chain
    // vertex), or `NO_SOURCE` for a headwall's outer corner.
    pub(super) sources: Vec<u32>,
    pub(super) triangles: Vec<[u32; 3]>,
    // Indices in `triangles` of the first end-wall, headwall and shell triangle.
    pub(super) end_walls: usize,
    pub(super) headwalls: usize,
    pub(super) shell: usize,
    // Whether the lining runs along a span's rows, whose stations measure the bore.
    pub(super) along_rows: bool,
}

/// The source of a lining vertex that stands on no mesh vertex.
pub(super) const NO_SOURCE: u32 = u32::MAX;

/// Why a node piece's lining has no construction: the ring of its lining tops bounds no
/// simple polygon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LiningError;

impl Lining {
    // The index of the next added vertex, after the piece's `count` positions, standing on mesh
    // vertex `source`.
    fn push(&mut self, count: usize, point: RoadVec3, source: u32) -> u32 {
        self.positions.push(point);
        self.sources.push(source);
        (count + self.positions.len() - 1) as u32
    }

    /// The part `triangle` (an index in `triangles`) belongs to.
    pub(super) fn part(&self, triangle: usize) -> LiningPart {
        if triangle < self.end_walls {
            LiningPart::Bore
        } else if triangle < self.headwalls {
            LiningPart::EndWall
        } else if triangle < self.shell {
            LiningPart::Headwall
        } else {
            LiningPart::Shell
        }
    }

    // The shell along `chains` (their lining tops from `firsts`, the ceiling's triangles at
    // `ceiling` in `triangles`): per chain vertex its foot (`feet` gives it per chain and
    // position) and the shell top above it, per chain segment the outer wall between the feet
    // and their tops, facing away from the road, and the roof: the ceiling's triangles over the
    // shell tops standing for its lining tops, facing up. It shares no edge with the bore.
    // `O(chain vertices + ceiling triangles)`.
    fn shell(
        &mut self,
        positions: &[RoadVec3],
        chains: &[OuterChain],
        (firsts, ceiling): (&[u32], std::ops::Range<usize>),
        feet: impl Fn(usize, usize) -> RoadVec3,
    ) {
        let count = positions.len();
        self.shell = self.triangles.len();
        let mut shell_firsts = Vec::with_capacity(chains.len());
        for (c, chain) in chains.iter().enumerate() {
            let vertices = chain.vertices();
            let first = (count + self.positions.len()) as u32;
            shell_firsts.push(first);
            for (k, &vertex) in vertices.iter().enumerate() {
                let foot = feet(c, k);
                self.push(count, foot, vertex);
                self.push(count, shell_top(foot), vertex);
            }
            for k in 0..vertices.len().saturating_sub(1) {
                let (foot_a, top_a) = (first + 2 * k as u32, first + 2 * k as u32 + 1);
                let (foot_b, top_b) = (foot_a + 2, top_a + 2);
                // The inner walls' windings mirrored: these face away from the road.
                self.triangles.push([foot_a, foot_b, top_b]);
                self.triangles.push([foot_a, top_b, top_a]);
            }
        }
        // A lining top `firsts[c] + k` stands for chain `c`'s vertex `k`, whose shell top is
        // `shell_firsts[c] + 2 k + 1`.
        let outer = |top: u32| {
            let c = firsts.partition_point(|&first| first <= top) - 1;
            shell_firsts[c] + 2 * (top - firsts[c]) + 1
        };
        for t in ceiling {
            let [a, b, c] = self.triangles[t].map(outer);
            self.triangles.push([a, c, b]);
        }
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
                self.push(count, lining_top(positions[vertex as usize]), vertex);
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
    let ceiling = lining.triangles.len();
    for row in 0..rows - 1 {
        let (a, b) = (left_top(row), right_top(row));
        let (c, d) = (left_top(row + 1), right_top(row + 1));
        // Facing down, from the left top across to the right.
        lining.triangles.push([a, b, d]);
        lining.triangles.push([a, d, c]);
    }
    let ceiling = ceiling..lining.triangles.len();
    lining.end_walls = lining.triangles.len();
    lining.along_rows = true;
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
    // A chain vertex's row is its index over the stride; its foot lies away from the row's
    // other corner.
    lining.shell(positions, chains, (&firsts, ceiling), |c, k| {
        let vertex = chains[c].vertices()[k] as usize;
        let row = vertex / stride * stride;
        let other = if vertex == row { row + stride - 1 } else { row };
        shell_foot(positions[vertex], positions[other])
    });
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
    let ceiling = lining.triangles.len();
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
    lining.end_walls = lining.triangles.len();
    lining.headwalls = lining.triangles.len();
    let ceiling = ceiling..lining.triangles.len();
    lining.shell(positions, chains, (&firsts, ceiling), |c, k| {
        node_foot(positions, chains, c, k)
    });
    Ok(lining)
}

// The shell's foot outside vertex `k` of chain `c` of a node piece whose chains run from mouth
// corner to mouth corner, each mouth between one chain's last vertex and the next chain's first:
// at a mouth corner [`shell_foot`] away from the mouth's other corner, as the span sharing the
// mouth takes it; elsewhere the thickness out along the mitred normal of the chain's two
// segments there, away from the road (its right).
fn node_foot(positions: &[RoadVec3], chains: &[OuterChain], c: usize, k: usize) -> RoadVec3 {
    let vertices = chains[c].vertices();
    let at = |chain: usize, index: usize| positions[chains[chain].vertices()[index] as usize];
    let n = chains.len();
    let vertex = at(c, k);
    if k == 0 {
        let previous = (c + n - 1) % n;
        return shell_foot(vertex, at(previous, chains[previous].vertices().len() - 1));
    }
    if k + 1 == vertices.len() {
        return shell_foot(vertex, at((c + 1) % n, 0));
    }
    // The road lies on the counter-clockwise side of `along` in XZ; this is the other.
    let right = |from: RoadVec3, to: RoadVec3| {
        let along = RoadVec2::new(to.x - from.x, to.z - from.z).normalize();
        RoadVec2::new(along.y, -along.x)
    };
    let (before, after) = (right(at(c, k - 1), vertex), right(vertex, at(c, k + 1)));
    let mitre = (before + after).normalize();
    // At most twice the thickness out, at a turn of 120 degrees or sharper.
    let reach = TUNNEL_LINING_THICKNESS_M / mitre.dot(before).max(0.5);
    RoadVec3::new(vertex.x + reach * mitre.x, vertex.y, vertex.z + reach * mitre.y)
}

// The headwall at a portal row (its corners `edge` and the lining tops `tops` above them; the
// piece has `count` positions): in the row's vertical plane, from the skirt depth below the
// corners up to `HEADWALL_HEIGHT_M` above them, and its wings `HEADWALL_WING_M` beyond them along
// the row's line, their tops falling at the 1:2 tie-in slope to the corners' height at the tips
// (`headwall_wing`, round R8c), with the lining's mouth open in it from below: the polygon runs
// from the left wing's tip along its bottom to below the left corner, up to it, up the left
// wall's end edge, across the ceiling's, down the right wall's, down below the right corner,
// along the bottom to the right wing's tip, up it and back along the wings' tops, so it meets the
// lining edge for edge and leaves the deck's row to the deck and the piece beyond it. Facing
// `outward`, out of the tunnel.
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
    let ([left_top, left_tip], [right_top, right_tip]) =
        (headwall_wing(left, right), headwall_wing(right, left));
    let points = [
        left_tip,
        at(left, -HEADWALL_WING_M, -SKIRT_DEPTH_M),
        at(left, 0.0, -SKIRT_DEPTH_M),
    ];
    let mut ring: Vec<(u32, RoadVec3)> = points
        .into_iter()
        .map(|point| (lining.push(count, point, NO_SOURCE), point))
        .collect();
    ring.extend([(left_corner, left), tops[0], tops[1], (right_corner, right)]);
    for point in [
        at(right, 0.0, -SKIRT_DEPTH_M),
        at(right, HEADWALL_WING_M, -SKIRT_DEPTH_M),
        right_tip,
        right_top,
        left_top,
    ] {
        ring.push((lining.push(count, point, NO_SOURCE), point));
    }
    lining.plane_face((left, right), &[&ring], outward);
}
