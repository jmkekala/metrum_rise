// SPDX-License-Identifier: GPL-2.0-only

//! `PieceMesh`: a piece's vertices, triangles grouped by band kind, curb faces, outer chains,
//! the strips hanging from them and the triangle query grid, and the span mesh built from its
//! sections.
//!
//! Every triangle is wound so that `-(b - a) x (c - a)` is its visible normal, the renderer's
//! convention: top triangles face up, curb faces face the carriageway and strips face away from
//! the road.

use super::geometry::{SurfaceTriangleGrid, bounds_xz, triangulate_constrained_rings_xz};
use super::profile::{Level, MAX_PROFILE_BANDS, RoadProfile};
use super::span::MouthSection;
use super::{BandKind, RoadSection, RoadVec2, RoadVec3};

// Band-kind groups in `BandKind` declaration order, then the curb faces.
const BAND_GROUPS: usize = 8;
pub(super) const CURB_FACE_GROUP: usize = BAND_GROUPS;
pub(super) const GROUPS: usize = BAND_GROUPS + 1;

/// Depth of a road skirt below its outer chain (`TERRAIN-07-B`, `docs/terrain.md` TD6): the near
/// depth, which the road shader deepens with distance (T5).
pub const SKIRT_DEPTH_M: f64 = 0.5;
/// Depth of a bridge deck's concrete below the deck's outer edge: the fascia's height and the
/// level of the concrete slab under the deck (`TERRAIN-07-B`).
pub const BRIDGE_DECK_DEPTH_M: f64 = 0.35;

/// The strips a piece hangs from its outer chains (`TERRAIN-07-B`): one vertical strip per chain,
/// whose top vertices are the chain's own vertices and whose bottom vertices lie straight below
/// them. A layer of its own beside the top surface and the curb faces; queries never see it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StripKind {
    /// Road skirts, [`SKIRT_DEPTH_M`] deep, on every grounded piece (standard spans, and node
    /// pieces where a standard edge meets): they reach into the ground below the footprint's
    /// edge, so a terrain mesh that sags below the edge leaves no gap under the road.
    Skirt,
    /// Bridge fascias, [`BRIDGE_DECK_DEPTH_M`] deep: a bridge span's deck sides, closed down to
    /// the concrete slab, whose edges are the fascias' bottom vertices.
    Fascia,
}

impl StripKind {
    /// Metres from a strip's top vertex down to its bottom vertex.
    pub fn depth_m(self) -> f64 {
        match self {
            Self::Skirt => SKIRT_DEPTH_M,
            Self::Fascia => BRIDGE_DECK_DEPTH_M,
        }
    }

    /// The bottom vertex below the strip's top vertex `top`: straight down by the depth. The one
    /// function every strip and the bridge slab take their bottom vertices from.
    pub fn bottom(self, top: RoadVec3) -> RoadVec3 {
        RoadVec3::new(top.x, top.y - self.depth_m(), top.z)
    }
}

/// How a span closes one of its end rows (`TERRAIN-07-C`, `docs/terrain.md` T22), decided by
/// the node's arms (`arms::span_end`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SpanEnd {
    /// The row is shared with a piece that closes it, or with another structure.
    Shared,
    /// A structure's free end: capped down to the kind's depth ([`end_cap`]).
    Cap(StripKind),
    /// A grounded span's mouth shared with a bridge or tunnel: a wall run across the row,
    /// `abutment` where the structure is a bridge, whose span hangs the abutment face below it.
    WallRun {
        /// The structure beyond the row is a bridge.
        abutment: bool,
    },
    /// A bridge's mouth shared with a grounded piece: the abutment face below the slab's end
    /// edge ([`PieceMesh::for_each_abutment_triangle`]).
    Abutment,
    /// A tunnel's mouth shared with a grounded piece: its portal row, near which the graded
    /// terrain opens the ground over the deck ([`PieceMesh::portal_rows`], T22 5 (a)).
    Portal,
}

/// The height of an abutment face's bottom edge ([`PieceMesh::for_each_abutment_triangle`]) at
/// the XZ point `point` of its mouth row between the row's corners `corners`: the skirt depth
/// below them, straight between them, so the terrain's abutment wall continues the face from
/// that edge (`TERRAIN-07-D`, the abutment band). At a corner it is the face's bottom vertex's
/// height, bitwise. `O(1)`.
pub(super) fn abutment_bottom_height(corners: [RoadVec3; 2], point: RoadVec3) -> f64 {
    let [a, b] = corners.map(|corner| StripKind::Skirt.bottom(corner));
    if point == corners[0] {
        return a.y;
    }
    if point == corners[1] {
        return b.y;
    }
    let (dx, dz) = (b.x - a.x, b.z - a.z);
    let t = ((point.x - a.x) * dx + (point.z - a.z) * dz) / (dx * dx + dz * dz);
    a.y + (b.y - a.y) * t
}

/// Footprint boundary polyline: vertex indices into its piece's positions, ordered so the road
/// lies on the left in XZ (the counter-clockwise quarter turn from +x towards +z), from one
/// mouth corner to the next.
#[derive(Clone, Debug, PartialEq)]
pub struct OuterChain {
    vertices: Box<[u32]>,
}

impl OuterChain {
    /// Vertex indices into [`PieceMesh::positions`], in chain order.
    pub fn vertices(&self) -> &[u32] {
        &self.vertices
    }
}

/// The wall run over a mouth row's vertices `row`, given from one outer chain's end corner to the
/// next chain's start corner: a curb pair (two consecutive vertices with equal XZ) once, at its
/// lower vertex. `O(row)`.
pub(super) fn wall_run(positions: &[RoadVec3], row: impl Iterator<Item = u32>) -> OuterChain {
    let mut vertices: Vec<u32> = Vec::new();
    for vertex in row {
        let point = positions[vertex as usize];
        if let Some(last) = vertices.last_mut() {
            let previous = positions[*last as usize];
            if (previous.x, previous.z) == (point.x, point.z) {
                if point.y < previous.y {
                    *last = vertex;
                }
                continue;
            }
        }
        vertices.push(vertex);
    }
    OuterChain {
        vertices: vertices.into_boxed_slice(),
    }
}

// Segments per seam block: a block's bounds cost one test per this many chain segments.
const SEAM_BLOCK_SEGMENTS: usize = 8;

/// A run of at most `SEAM_BLOCK_SEGMENTS` consecutive segments of one outer chain, with the XZ
/// bounds and height range of its vertices, which hold every segment's own: a walk skips a run
/// none of whose segments can meet its bounds or pass its test (`TERRAIN-07-F`, T37 (b)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SeamBlock {
    pub(super) chain: u32,
    // Positions in the chain's vertex list of the run's first and last vertex.
    pub(super) vertices: (u32, u32),
    pub(super) bounds_xz: [f64; 4],
    pub(super) heights: (f64, f64),
}

// The seam blocks of `chains`, in chain order, each chain's runs from its first vertex.
// `O(chain vertices)`.
fn seam_blocks(positions: &[RoadVec3], chains: &[OuterChain]) -> Vec<SeamBlock> {
    let mut blocks = Vec::new();
    for (chain, outer) in chains.iter().enumerate() {
        let segments = outer.vertices.len().saturating_sub(1);
        for first in (0..segments).step_by(SEAM_BLOCK_SEGMENTS) {
            let last = (first + SEAM_BLOCK_SEGMENTS).min(segments);
            let points = || {
                outer.vertices[first..=last]
                    .iter()
                    .map(|&v| positions[v as usize])
            };
            blocks.push(SeamBlock {
                chain: chain as u32,
                vertices: (first as u32, last as u32),
                bounds_xz: bounds_xz(points()),
                heights: points().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(p.y), hi.max(p.y))
                }),
            });
        }
    }
    blocks
}

/// Immutable mesh of one roadbed piece.
#[derive(Debug, PartialEq)]
pub struct PieceMesh {
    positions: Vec<RoadVec3>,
    // Per vertex: station along the edge and signed lateral offset, in metres.
    road_space: Vec<[f32; 2]>,
    // Grouped: band kinds in declaration order, then curb faces.
    triangles: Vec<[u32; 3]>,
    group_starts: [u32; GROUPS + 1],
    outer_chains: Vec<OuterChain>,
    // The outer chains' segments in runs, with their bounds (`TERRAIN-07-F`).
    seam_blocks: Vec<SeamBlock>,
    // The wall runs across the structure mouths a grounded piece meets (`TERRAIN-07-C`), and per
    // run whether the structure is a bridge, whose abutment face hangs below the row.
    wall_runs: Vec<OuterChain>,
    abutment_runs: Vec<bool>,
    // The strips below the outer chains; a tunnel's road piece has none.
    strips: Option<StripKind>,
    // The caps closing a structure's free ends (`TERRAIN-07-C`): triangles over `(vertex,
    // bottom)` pairs, a mesh vertex or the bottom `cap_kind` puts below it.
    caps: Vec<[(u32, bool); 3]>,
    cap_kind: Option<StripKind>,
    // The abutment faces at a bridge's mouths shared with a grounded piece (`TERRAIN-07-C`):
    // per mouth its row's two corner vertices, the space under the bridge right of the first to
    // the second.
    abutments: Vec<[u32; 2]>,
    // The portal rows of a tunnel span (`TERRAIN-07-C`): per mouth shared with a grounded piece
    // its end row's first and last vertex, the row the vertices between them.
    portals: Vec<[u32; 2]>,
    bounds_xz: [f64; 4],
    // Top triangles only; items are triangle indices.
    grid: SurfaceTriangleGrid,
}

impl PieceMesh {
    /// Vertex positions in world metres.
    pub fn positions(&self) -> &[RoadVec3] {
        &self.positions
    }

    /// Per-vertex road-space coordinates: metres along the edge from its start, and signed metres
    /// across it from the centerline (negative is left).
    pub fn road_space(&self) -> &[[f32; 2]] {
        &self.road_space
    }

    /// Every triangle: the top triangles grouped by band kind, then the curb faces.
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// Top triangles of one band kind.
    pub fn band_triangles(&self, kind: BandKind) -> &[[u32; 3]] {
        self.group(kind as usize)
    }

    /// Vertical curb-face triangles along the carriageway edges.
    pub fn curb_face_triangles(&self) -> &[[u32; 3]] {
        self.group(CURB_FACE_GROUP)
    }

    /// Number of top triangles; they are the first triangles of [`Self::triangles`].
    pub fn top_triangle_count(&self) -> usize {
        self.group_starts[CURB_FACE_GROUP] as usize
    }

    /// Band kind of a top triangle; `None` for a curb face or an index out of range.
    pub fn triangle_band(&self, triangle: usize) -> Option<BandKind> {
        if triangle >= self.top_triangle_count() {
            return None;
        }
        let group = self.group_starts[1..].partition_point(|&end| end as usize <= triangle);
        Some(BAND_KINDS[group])
    }

    /// World positions of one triangle.
    pub fn triangle_points(&self, triangle: usize) -> [RoadVec3; 3] {
        self.triangles[triangle].map(|vertex| self.positions[vertex as usize])
    }

    /// Footprint boundary chains.
    pub fn outer_chains(&self) -> &[OuterChain] {
        &self.outer_chains
    }

    // The outer chains' segments in runs of consecutive segments, with their bounds.
    pub(super) fn seam_blocks(&self) -> &[SeamBlock] {
        &self.seam_blocks
    }

    /// The wall runs across the structure mouths a grounded piece meets (`TERRAIN-07-C`,
    /// `docs/terrain.md` T22 3 (a)): per mouth row where a bridge or tunnel begins, its vertices
    /// from the end corner of one outer chain to the start corner of the next, a curb pair once
    /// at its lower vertex, so the road lies on the left like the chains'. With the outer chains
    /// they close the grounded footprint; the graded terrain reads their segments as wall seams.
    pub fn wall_runs(&self) -> &[OuterChain] {
        &self.wall_runs
    }

    /// Per wall run ([`Self::wall_runs`]), whether its mouth is a bridge's: the bridge span
    /// hangs the abutment face below the row ([`Self::for_each_abutment_triangle`]), whose
    /// bottom edge the terrain's abutment wall continues (`TERRAIN-07-D`, the abutment band).
    pub fn abutment_runs(&self) -> &[bool] {
        &self.abutment_runs
    }

    /// The kind of strips hanging from the outer chains, `None` for a tunnel's road piece.
    pub fn strip_kind(&self) -> Option<StripKind> {
        self.strips
    }

    /// The kind whose depth the bottoms of the piece's free-end caps take (`TERRAIN-07-C`): a
    /// bridge's fascias, a tunnel's skirt depth; `None` without a free end.
    pub fn cap_kind(&self) -> Option<StripKind> {
        self.cap_kind
    }

    /// The strip's bottom vertex below outer-chain vertex `vertex` (an index into
    /// [`Self::positions`]): the vertex moved straight down by the strip's depth. `None` without
    /// strips.
    pub fn strip_bottom(&self, vertex: u32) -> Option<RoadVec3> {
        Some(self.strips?.bottom(self.positions[vertex as usize]))
    }

    /// Visits the strip triangles, chain by chain and segment by segment, two per chain segment,
    /// then the free-end caps' (`TERRAIN-07-C`): each as its world points and, per point, whether
    /// it is a bottom vertex. The top points are the mesh's positions bitwise and the bottoms
    /// [`StripKind::bottom`]'s of the strip kind or the cap kind. Each triangle is wound so that
    /// `-(b - a) x (c - a)` faces away from the road: the right of the chain, out of the free end.
    /// `O(chain vertices + cap triangles)`, without allocation.
    pub fn for_each_strip_triangle(&self, mut visit: impl FnMut([RoadVec3; 3], [bool; 3])) {
        if let Some(kind) = self.strips {
            for chain in &self.outer_chains {
                for pair in chain.vertices.windows(2) {
                    let [a, b] = [pair[0], pair[1]].map(|vertex| self.positions[vertex as usize]);
                    // Down the chain's start, along the bottom, back up its end: the road lies left
                    // of the chain, so this order faces right, away from it.
                    visit([a, kind.bottom(a), kind.bottom(b)], [false, true, true]);
                    visit([a, kind.bottom(b), b], [false, true, false]);
                }
            }
        }
        let Some(kind) = self.cap_kind else {
            return;
        };
        let point = |(vertex, bottom): (u32, bool)| {
            let top = self.positions[vertex as usize];
            if bottom { kind.bottom(top) } else { top }
        };
        for triangle in &self.caps {
            visit(triangle.map(point), triangle.map(|(_, bottom)| bottom));
        }
    }

    /// Visits the abutment faces' triangles (`TERRAIN-07-C`, `docs/terrain.md` T22 4 (a)): at
    /// each mouth a bridge span shares with a grounded piece, the quad in the mouth row's
    /// vertical plane from the slab's end edge (the fascias' bottoms below the row's two
    /// corners, [`StripKind::Fascia`]) down to the skirt depth below the corners
    /// ([`StripKind::Skirt`]), where the grounded piece's skirts end and the terrain's abutment
    /// wall continues it to the graded ground. Each triangle comes with its points and, per
    /// point, whether it is a bottom vertex (at the skirt depth), wound so that
    /// `-(b - a) x (c - a)` faces the space under the bridge. `O(mouths)`, without allocation.
    pub fn for_each_abutment_triangle(&self, mut visit: impl FnMut([RoadVec3; 3], [bool; 3])) {
        for &corners in &self.abutments {
            let [a, b] = corners.map(|vertex| self.positions[vertex as usize]);
            let ([top_a, top_b], [bottom_a, bottom_b]) = (
                [a, b].map(|corner| StripKind::Fascia.bottom(corner)),
                [a, b].map(|corner| StripKind::Skirt.bottom(corner)),
            );
            // As a strip below the chain from `a` to `b`: facing right of it.
            visit([top_a, bottom_a, bottom_b], [false, true, true]);
            visit([top_a, bottom_b, top_b], [false, true, false]);
        }
    }

    /// The portal rows of a tunnel span (`TERRAIN-07-C`, `docs/terrain.md` T22 5 (a)): per end
    /// row it shares with a grounded piece, the row's vertices from one corner to the other,
    /// bitwise the vertices the grounded piece's wall run across that mouth takes. Empty for
    /// every other piece. `O(1)` per row.
    pub fn portal_rows(&self) -> impl Iterator<Item = &[RoadVec3]> + '_ {
        self.portals
            .iter()
            .map(|&[first, last]| &self.positions[first as usize..=last as usize])
    }

    /// XZ bounds `[min_x, min_z, max_x, max_z]` of every vertex.
    pub fn bounds_xz(&self) -> [f64; 4] {
        self.bounds_xz
    }

    pub(super) fn grid(&self) -> &SurfaceTriangleGrid {
        &self.grid
    }

    /// Visits the top triangles whose cells of the piece's query grid touch the XZ bounds
    /// `[min_x, min_z, max_x, max_z]`, by index, until `visit` returns true; returns whether it
    /// did. The one triangle walk of every roadbed overlap query (`ROAD-68-F` part 2): the
    /// roadbed's own, and the zoning, field and vegetation tests against a piece or the published
    /// roadbed. A triangle may be visited once per cell it shares with the bounds; cost follows
    /// the touched cells.
    pub fn any_top_triangle_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        mut visit: impl FnMut(usize) -> bool,
    ) -> bool {
        let top = self.top_triangle_count();
        self.grid
            .items_in_bounds(bounds_xz)
            .any(|item| (item as usize) < top && visit(item as usize))
    }

    fn group(&self, group: usize) -> &[[u32; 3]] {
        &self.triangles[self.group_starts[group] as usize..self.group_starts[group + 1] as usize]
    }
}

const BAND_KINDS: [BandKind; BAND_GROUPS] = [
    BandKind::Carriageway,
    BandKind::CurbOrShoulder,
    BandKind::Sidewalk,
    BandKind::Footpath,
    BandKind::Median,
    BandKind::Parking,
    BandKind::CycleTrack,
    BandKind::TramReservation,
];

// A ground / raised boundary inside a section row.
#[derive(Clone, Copy, Debug, Default)]
struct CurbStep {
    raised_slot: u8,
    ground_slot: u8,
    // True when the raised band is on the left, so the face looks right (+lateral).
    raised_on_left: bool,
}

/// Vertex slots of one section row, fixed by the profile's band kinds and levels.
#[derive(Clone, Copy, Debug)]
pub(super) struct RowLayout {
    band_count: u8,
    kinds: [BandKind; MAX_PROFILE_BANDS],
    // Start and end vertex slot of each band.
    band_slots: [(u8, u8); MAX_PROFILE_BANDS],
    // Per boundary: whether it holds two vertices, one per level.
    doubled: [bool; MAX_PROFILE_BANDS + 1],
    steps: [CurbStep; MAX_PROFILE_BANDS],
    step_count: u8,
    stride: u8,
}

impl RowLayout {
    pub(super) fn new(profile: &RoadProfile) -> Self {
        let bands = profile.bands();
        let mut layout = Self {
            band_count: bands.len() as u8,
            kinds: [BandKind::Carriageway; MAX_PROFILE_BANDS],
            band_slots: [(0, 0); MAX_PROFILE_BANDS],
            doubled: [false; MAX_PROFILE_BANDS + 1],
            steps: [CurbStep::default(); MAX_PROFILE_BANDS],
            step_count: 0,
            stride: 0,
        };
        let mut slot = 0u8;
        for boundary in 0..=bands.len() {
            let doubled = boundary > 0
                && boundary < bands.len()
                && bands[boundary - 1].level != bands[boundary].level;
            layout.doubled[boundary] = doubled;
            if boundary > 0 {
                layout.band_slots[boundary - 1].1 = slot;
            }
            if boundary < bands.len() {
                layout.kinds[boundary] = bands[boundary].kind;
                layout.band_slots[boundary].0 = slot + u8::from(doubled);
            }
            if doubled {
                let raised_on_left = bands[boundary - 1].level == Level::Raised;
                layout.steps[usize::from(layout.step_count)] = CurbStep {
                    raised_slot: if raised_on_left { slot } else { slot + 1 },
                    ground_slot: if raised_on_left { slot + 1 } else { slot },
                    raised_on_left,
                };
                layout.step_count += 1;
            }
            slot += 1 + u8::from(doubled);
        }
        layout.stride = slot;
        layout
    }

    /// Vertices per section row.
    pub(super) fn stride(&self) -> usize {
        usize::from(self.stride)
    }

    /// Row slots of the two carriageway-edge vertices at ground level: the first carriageway
    /// band's start and the last one's end. `None` without a carriageway band.
    pub(super) fn carriageway_edge_slots(&self) -> Option<(usize, usize)> {
        let kinds = &self.kinds[..usize::from(self.band_count)];
        let first = kinds
            .iter()
            .position(|&kind| kind == BandKind::Carriageway)?;
        let last = kinds
            .iter()
            .rposition(|&kind| kind == BandKind::Carriageway)?;
        Some((
            usize::from(self.band_slots[first].0),
            usize::from(self.band_slots[last].1),
        ))
    }

    fn steps(&self) -> &[CurbStep] {
        &self.steps[..usize::from(self.step_count)]
    }
}

// Lateral offset and the vertex heights of each boundary, left to right.
fn for_each_boundary(section: &RoadSection, layout: &RowLayout, mut emit: impl FnMut(f32, f32)) {
    let bands = &section.bands;
    let band_count = usize::from(layout.band_count);
    debug_assert_eq!(bands.len(), band_count, "section and row layout disagree");
    for boundary in 0..=band_count {
        if boundary == band_count {
            let band = &bands[band_count - 1];
            emit(band.lateral_end_m, band.height_end_m);
            continue;
        }
        let band = &bands[boundary];
        if layout.doubled[boundary] {
            emit(band.lateral_start_m, bands[boundary - 1].height_end_m);
        }
        emit(band.lateral_start_m, band.height_start_m);
    }
}

/// Appends one section row: one vertex per band boundary per level, left to right. Vertex XZ is
/// `center + lateral * offset` and Y is the band's evaluated height, so both vertices of a curb
/// step share bit-identical XZ.
pub(super) fn push_section_row(
    section: &RoadSection,
    layout: &RowLayout,
    positions: &mut Vec<RoadVec3>,
) {
    push_section_row_at(
        section,
        layout,
        |lateral_m| section.center_xz + section.lateral_xz * f64::from(lateral_m),
        positions,
    );
}

/// [`push_section_row`] with the boundary XZ taken from `xz(lateral offset)`: a node piece's
/// mouth places its row on the arm's offset curves.
pub(super) fn push_section_row_at(
    section: &RoadSection,
    layout: &RowLayout,
    xz: impl Fn(f32) -> RoadVec2,
    positions: &mut Vec<RoadVec3>,
) {
    for_each_boundary(section, layout, |lateral_m, height_m| {
        let point = xz(lateral_m);
        positions.push(RoadVec3::new(point.x, f64::from(height_m), point.y));
    });
}

/// The `(lateral offset, level)` of each vertex slot of a section row, left to right: one per
/// band boundary per level, the left band's first at a level change.
pub(super) fn row_slots(profile: &RoadProfile) -> impl Iterator<Item = (f32, Level)> + '_ {
    let bands = profile.bands();
    (0..=bands.len()).flat_map(move |boundary| {
        let (lateral_m, level) = match boundary {
            0 => (bands[0].start_m, bands[0].level),
            _ if boundary == bands.len() => (bands[boundary - 1].end_m, bands[boundary - 1].level),
            _ => (bands[boundary].start_m, bands[boundary].level),
        };
        let doubled = boundary > 0
            && boundary < bands.len()
            && bands[boundary - 1].level != bands[boundary].level;
        let left = doubled.then(|| (lateral_m, bands[boundary - 1].level));
        left.into_iter().chain(std::iter::once((lateral_m, level)))
    })
}

/// Builds a span mesh from its sections, whose first and last rows are copied from the two
/// mouth sections, with `strips` below its two outer chains, and each end row closed as `ends`
/// says (`TERRAIN-07-C`): a cap ([`end_cap`]), a wall run, an abutment face or a portal row.
/// `O(sections * bands)`; allocates only the mesh.
pub(super) fn build_span_mesh(
    sections: &[RoadSection],
    (start_mouth, end_mouth): (&MouthSection, &MouthSection),
    layout: &RowLayout,
    strips: Option<StripKind>,
    ends: [SpanEnd; 2],
) -> PieceMesh {
    let rows = sections.len();
    let stride = layout.stride();
    let mut positions = Vec::with_capacity(rows * stride);
    let mut road_space = Vec::with_capacity(rows * stride);
    for (row, section) in sections.iter().enumerate() {
        if row == 0 {
            positions.extend_from_slice(&start_mouth.vertices);
        } else if row + 1 == rows {
            positions.extend_from_slice(&end_mouth.vertices);
        } else {
            push_section_row(section, layout, &mut positions);
        }
        for_each_boundary(section, layout, |lateral_m, _| {
            road_space.push([section.s_m, lateral_m]);
        });
    }
    debug_assert_eq!(positions.len(), rows * stride);

    let pairs = rows.saturating_sub(1);
    let mut counts = [0u32; GROUPS];
    for &kind in &layout.kinds[..usize::from(layout.band_count)] {
        counts[kind as usize] += 2 * pairs as u32;
    }
    counts[CURB_FACE_GROUP] = 2 * pairs as u32 * u32::from(layout.step_count);
    let group_starts = group_starts(counts);
    let mut triangles = vec![[0u32; 3]; group_starts[GROUPS] as usize];
    let mut cursors = [0u32; GROUPS];
    cursors.copy_from_slice(&group_starts[..GROUPS]);
    let mut push = |group: usize, triangle: [u32; 3]| {
        triangles[cursors[group] as usize] = triangle;
        cursors[group] += 1;
    };
    for pair in 0..pairs {
        let start_row = (pair * stride) as u32;
        let end_row = start_row + stride as u32;
        for band in 0..usize::from(layout.band_count) {
            let (start_slot, end_slot) = layout.band_slots[band];
            let start_left = start_row + u32::from(start_slot);
            let end_left = end_row + u32::from(start_slot);
            let end_right = end_row + u32::from(end_slot);
            let start_right = start_row + u32::from(end_slot);
            // Legacy fan from the start-left corner; positive XZ winding faces up.
            let group = layout.kinds[band] as usize;
            push(group, [start_left, end_left, end_right]);
            push(group, [start_left, end_right, start_right]);
        }
        for step in layout.steps() {
            let top = |row: u32| row + u32::from(step.raised_slot);
            let bottom = |row: u32| row + u32::from(step.ground_slot);
            // Listed facing right (raised band on the left); a face looking left is mirrored.
            for mut face in [
                [top(start_row), top(end_row), bottom(end_row)],
                [top(start_row), bottom(end_row), bottom(start_row)],
            ] {
                if !step.raised_on_left {
                    face.swap(1, 2);
                }
                push(CURB_FACE_GROUP, face);
            }
        }
    }

    // Road on the left in XZ (the counter-clockwise side, as the node chains): slot 0 runs
    // forward from the start mouth, the last slot back from the end mouth (`ROAD-68-D`).
    let last = stride as u32 - 1;
    let outer_chains = vec![
        OuterChain {
            vertices: (0..rows as u32).map(|row| row * stride as u32).collect(),
        },
        OuterChain {
            vertices: (0..rows as u32)
                .rev()
                .map(|row| row * stride as u32 + last)
                .collect(),
        },
    ];
    let (mut wall_runs, mut abutment_runs, mut faces) =
        (Vec::new(), Vec::new(), EndFaces::default());
    for (end, closure) in ends.into_iter().enumerate() {
        // The end row, the row next to it inside the span, and the end row's first and last
        // vertices. A wall run lists the row with the span's road on its left: the start row
        // from chain 1's end (the last slot) back to chain 0's start, the end row from chain
        // 0's end (slot 0) to chain 1's start.
        let (row, inner) = if end == 0 {
            (0, 1)
        } else {
            (rows - 1, rows - 2)
        };
        let (first, last) = ((row * stride) as u32, (row * stride + stride - 1) as u32);
        match closure {
            SpanEnd::Shared => {}
            SpanEnd::WallRun { abutment } => {
                wall_runs.push(if end == 0 {
                    wall_run(&positions, (first..=last).rev())
                } else {
                    wall_run(&positions, first..=last)
                });
                abutment_runs.push(abutment);
            }
            SpanEnd::Cap(kind) => {
                faces.cap_kind = Some(kind);
                end_cap(
                    &positions,
                    (row * stride, inner * stride),
                    stride,
                    kind.depth_m(),
                    &mut faces.caps,
                );
            }
            // An abutment runs the other way, so the space under the bridge lies on its right.
            SpanEnd::Abutment if end == 0 => faces.abutments.push([first, last]),
            SpanEnd::Abutment => faces.abutments.push([last, first]),
            SpanEnd::Portal => faces.portals.push([first, last]),
        }
    }
    PieceMesh::new(
        positions,
        road_space,
        triangles,
        group_starts,
        (outer_chains, wall_runs, abutment_runs),
        strips,
        faces,
    )
}

// The faces closing a span's end rows (`TERRAIN-07-C`): the free-end caps' triangles over
// `(vertex, bottom)` pairs with the kind their bottoms take, the abutment faces' corner pairs,
// and the portal rows' first and last vertices.
#[derive(Default)]
struct EndFaces {
    caps: Vec<[(u32, bool); 3]>,
    cap_kind: Option<StripKind>,
    abutments: Vec<[u32; 2]>,
    portals: Vec<[u32; 2]>,
}

// The cap closing a structure's free end (`TERRAIN-07-C`): the polygon in the end row's vertical
// plane between the row (its `stride` vertices from `first`, left to right, whose consecutive
// pairs are the row's band and curb-step edges) and the straight edge between the bottoms `depth`
// below its two corners (a bridge's slab end), triangulated in that plane, so its top edges are
// the row's edges, its sides the strips' end edges and its bottom the slab's end edge. Each
// triangle, over `(vertex, bottom)` pairs, faces out of the span, away from the row from `inner`.
// A row that dips below the bottom edge bounds no simple polygon and gets no cap: its slab would
// cut its deck. `O(stride log stride)`.
fn end_cap(
    positions: &[RoadVec3],
    (first, inner): (usize, usize),
    stride: usize,
    depth: f64,
    out: &mut Vec<[(u32, bool); 3]>,
) {
    let (left, right) = (positions[first], positions[first + stride - 1]);
    let along = RoadVec2::new(right.x - left.x, right.z - left.z).normalize();
    let ring: Vec<(u32, bool)> = std::iter::once((first as u32, true))
        .chain((first..first + stride).map(|vertex| (vertex as u32, false)))
        .chain(std::iter::once(((first + stride - 1) as u32, true)))
        .collect();
    // The ring in the plane, as the XZ of points `(lateral metres along the row, 0, height)`.
    let plane = |(vertex, bottom): (u32, bool)| {
        let point = positions[vertex as usize];
        let lateral = (point.x - left.x) * along.x + (point.z - left.z) * along.y;
        RoadVec3::new(lateral, 0.0, if bottom { point.y - depth } else { point.y })
    };
    let points: Vec<RoadVec3> = ring.iter().map(|&corner| plane(corner)).collect();
    let Some(triangles) = triangulate_constrained_rings_xz(&[&points]) else {
        debug_assert!(false, "a free end's row stands above its bottom edge");
        return;
    };
    // A triangle counter-clockwise in the plane faces `(along.y, -along.x)` in XZ: flip it when
    // that is not out of the span.
    let inward = positions[inner] - positions[first];
    let outward = RoadVec2::new(-inward.x, -inward.z);
    let facing = RoadVec2::new(along.y, -along.x).dot(outward);
    for triangle in triangles {
        let [a, b, c] = triangle.map(|point| {
            let index = points.iter().position(|&p| p == point);
            ring[index.expect("a cap vertex is a ring vertex")]
        });
        let [pa, pb, pc] = triangle;
        let counter_clockwise = (pb.x - pa.x) * (pc.z - pa.z) - (pb.z - pa.z) * (pc.x - pa.x) > 0.0;
        out.push(if counter_clockwise == (facing > 0.0) {
            [a, b, c]
        } else {
            [a, c, b]
        });
    }
}

impl PieceMesh {
    /// Assembles a node piece's mesh from its triangle groups (band kinds in declaration order,
    /// then curb faces), hanging `strips` from its chains: skirts where it stands on the terrain,
    /// none where only structures meet. `wall_runs` close the chains across its structure
    /// mouths ([`Self::wall_runs`]), each with whether it is a bridge's ([`Self::abutment_runs`]).
    pub(super) fn from_groups(
        positions: Vec<RoadVec3>,
        road_space: Vec<[f32; 2]>,
        groups: &[Vec<[u32; 3]>; GROUPS],
        outer_chains: Vec<Box<[u32]>>,
        wall_runs: Vec<(OuterChain, bool)>,
        strips: Option<StripKind>,
    ) -> Self {
        let group_starts = group_starts(groups.each_ref().map(|group| group.len() as u32));
        let triangles = groups.iter().flatten().copied().collect();
        let outer_chains = outer_chains
            .into_iter()
            .map(|vertices| OuterChain { vertices })
            .collect();
        let (wall_runs, abutment_runs) = wall_runs.into_iter().unzip();
        Self::new(
            positions,
            road_space,
            triangles,
            group_starts,
            (outer_chains, wall_runs, abutment_runs),
            strips,
            EndFaces::default(),
        )
    }

    // Builds the bounds and top-triangle grid of grouped triangles.
    fn new(
        positions: Vec<RoadVec3>,
        road_space: Vec<[f32; 2]>,
        triangles: Vec<[u32; 3]>,
        group_starts: [u32; GROUPS + 1],
        (outer_chains, wall_runs, abutment_runs): (Vec<OuterChain>, Vec<OuterChain>, Vec<bool>),
        strips: Option<StripKind>,
        EndFaces {
            caps,
            cap_kind,
            abutments,
            portals,
        }: EndFaces,
    ) -> Self {
        let top_count = group_starts[CURB_FACE_GROUP] as usize;
        let grid = SurfaceTriangleGrid::from_triangles(
            triangles[..top_count]
                .iter()
                .map(|triangle| triangle.map(|vertex| positions[vertex as usize])),
            |triangle| triangle as u32,
        );
        Self {
            bounds_xz: bounds_xz(positions.iter().copied()),
            seam_blocks: seam_blocks(&positions, &outer_chains),
            positions,
            road_space,
            triangles,
            group_starts,
            outer_chains,
            wall_runs,
            abutment_runs,
            strips,
            caps,
            cap_kind,
            abutments,
            portals,
            grid,
        }
    }
}

// Start of each triangle group, and the end of the last, from the groups' triangle counts.
fn group_starts(counts: [u32; GROUPS]) -> [u32; GROUPS + 1] {
    let mut starts = [0u32; GROUPS + 1];
    for (group, count) in counts.into_iter().enumerate() {
        starts[group + 1] = starts[group] + count;
    }
    starts
}
