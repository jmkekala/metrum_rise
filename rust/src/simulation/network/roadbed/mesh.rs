// SPDX-License-Identifier: GPL-2.0-only

//! `PieceMesh`: a piece's vertices, triangles grouped by band kind, curb faces, outer chains and
//! triangle query grid, and the span mesh built from its sections.
//!
//! Every triangle is wound so that `-(b - a) x (c - a)` is its visible normal, the renderer's
//! convention: top triangles face up and curb faces face the carriageway.

use super::geometry::SurfaceTriangleGrid;
use super::profile::{Level, MAX_PROFILE_BANDS, RoadProfile};
use super::span::MouthSection;
use super::{BandKind, RoadSection, RoadVec3};

// Band-kind groups in `BandKind` declaration order, then the curb faces.
const BAND_GROUPS: usize = 8;
const CURB_FACE_GROUP: usize = BAND_GROUPS;
const GROUPS: usize = BAND_GROUPS + 1;

/// Footprint boundary polyline: vertex indices into its piece's positions, ordered so the road
/// lies on the left, from one mouth corner to the next.
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

    /// XZ bounds `[min_x, min_z, max_x, max_z]` of every vertex.
    pub fn bounds_xz(&self) -> [f64; 4] {
        self.bounds_xz
    }

    pub(super) fn grid(&self) -> &SurfaceTriangleGrid {
        &self.grid
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
#[derive(Clone, Copy, Debug)]
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
    // Per boundary: first slot and whether it holds two vertices.
    boundary_slots: [(u8, bool); MAX_PROFILE_BANDS + 1],
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
            boundary_slots: [(0, false); MAX_PROFILE_BANDS + 1],
            steps: [CurbStep {
                raised_slot: 0,
                ground_slot: 0,
                raised_on_left: false,
            }; MAX_PROFILE_BANDS],
            step_count: 0,
            stride: 0,
        };
        let mut slot = 0u8;
        for boundary in 0..=bands.len() {
            let doubled = boundary > 0
                && boundary < bands.len()
                && bands[boundary - 1].level != bands[boundary].level;
            layout.boundary_slots[boundary] = (slot, doubled);
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
        if layout.boundary_slots[boundary].1 {
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
    for_each_boundary(section, layout, |lateral_m, height_m| {
        positions.push(RoadVec3::new(
            section.center_xz.x + section.lateral_xz.x * f64::from(lateral_m),
            f64::from(height_m),
            section.center_xz.y + section.lateral_xz.y * f64::from(lateral_m),
        ));
    });
}

fn push_road_space_row(section: &RoadSection, layout: &RowLayout, road_space: &mut Vec<[f32; 2]>) {
    for_each_boundary(section, layout, |lateral_m, _| {
        road_space.push([section.s_m, lateral_m]);
    });
}

/// Builds a span mesh from its sections, whose first and last rows are copied from the two
/// mouth sections. `O(sections * bands)`; allocates only the mesh.
pub(super) fn build_span_mesh(
    sections: &[RoadSection],
    start_mouth: &MouthSection,
    end_mouth: &MouthSection,
    layout: &RowLayout,
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
        push_road_space_row(section, layout, &mut road_space);
    }
    debug_assert_eq!(positions.len(), rows * stride);

    let pairs = rows.saturating_sub(1);
    let mut counts = [0u32; GROUPS];
    for &kind in &layout.kinds[..usize::from(layout.band_count)] {
        counts[kind as usize] += 2 * pairs as u32;
    }
    counts[CURB_FACE_GROUP] = 2 * pairs as u32 * u32::from(layout.step_count);
    let mut group_starts = [0u32; GROUPS + 1];
    for group in 0..GROUPS {
        group_starts[group + 1] = group_starts[group] + counts[group];
    }
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
            if step.raised_on_left {
                push(
                    CURB_FACE_GROUP,
                    [top(start_row), top(end_row), bottom(end_row)],
                );
                push(
                    CURB_FACE_GROUP,
                    [top(start_row), bottom(end_row), bottom(start_row)],
                );
            } else {
                push(
                    CURB_FACE_GROUP,
                    [top(start_row), bottom(end_row), top(end_row)],
                );
                push(
                    CURB_FACE_GROUP,
                    [top(start_row), bottom(start_row), bottom(end_row)],
                );
            }
        }
    }

    // Road on the left: the left boundary runs back from the end mouth, the right one forward.
    let last = stride as u32 - 1;
    let outer_chains = vec![
        OuterChain {
            vertices: (0..rows as u32)
                .rev()
                .map(|row| row * stride as u32)
                .collect(),
        },
        OuterChain {
            vertices: (0..rows as u32)
                .map(|row| row * stride as u32 + last)
                .collect(),
        },
    ];
    let mesh_bounds = bounds_xz(&positions);
    let top_count = group_starts[CURB_FACE_GROUP] as usize;
    let grid = SurfaceTriangleGrid::from_triangles(
        triangles[..top_count]
            .iter()
            .map(|triangle| triangle.map(|vertex| positions[vertex as usize])),
        |triangle| triangle as u32,
    );
    PieceMesh {
        positions,
        road_space,
        triangles,
        group_starts,
        outer_chains,
        bounds_xz: mesh_bounds,
        grid,
    }
}

fn bounds_xz(positions: &[RoadVec3]) -> [f64; 4] {
    positions.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |bounds, point| {
            [
                bounds[0].min(point.x),
                bounds[1].min(point.z),
                bounds[2].max(point.x),
                bounds[3].max(point.z),
            ]
        },
    )
}
