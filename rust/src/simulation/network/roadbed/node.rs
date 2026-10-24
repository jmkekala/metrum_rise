// SPDX-License-Identifier: GPL-2.0-only

//! Node pieces (`ROAD-68-C`): the surface of one junction, bend or terminal, constructed from its
//! [`JunctionLayout`].
//!
//! A node is planned first ([`plan_node`]): its arms from the graph, the layout, the node plane
//! `P` and one mouth section per arm. A mouth section is the arm's full cross-section at its
//! mouth station, on the arm's offset curves (`ArmInput::offset_point`) with every height
//! `P(x, z) + level`; the span on that edge and this node piece both copy it. Bends and junctions
//! use the plane the profile solve pinned their arms to; a terminal's plane runs through the road
//! end and its mouth centreline point, with the end grade and no crossfall. A finalized
//! centreline more than 1 mm from `P` at a mouth is a profile-solve defect, reported as
//! [`NodeError::MouthOffPlane`].
//!
//! The piece mesh ([`build_node_piece`]) holds:
//! - the carriageway core: the layout's core loop, with each mouth row's interior carriageway
//!   vertices inserted on its mouth edge, triangulated by Spade's CDT
//! - one strip per band beyond the carriageway edge, per sector (pairs of boundary samples) and
//!   per arm side (the two boundaries' offset curves from the sector to the mouth, zipped by
//!   station)
//! - curb faces along every boundary between bands of different levels, except across mouths
//! - one outer chain per sector, from one arm's mouth corner back along its outermost boundary,
//!   across the sector and out to the next arm's mouth corner, so the road lies on its left
//!
//! Every vertex is `P(x, z) + level` at its XZ ([`surface_height_m`]). Vertices are shared by
//! exact position, so strips, faces and the core meet on the same vertices, and every mouth
//! vertex is the mouth section's own. `O(V + n log n)` for `V` emitted vertices and `n` core-loop
//! vertices; allocates only the piece and reuses [`NodeScratch`].

use super::arms::{CentrelinePosition, NodeArms};
use super::index::{PieceFootprint, RenderGrid};
use super::layout::{
    ArmInput, ArmKey, BoundaryBand, CoreVertexSource, Crosswalk, EdgeEnd, JunctionLayout,
    LayoutError, NodeKind, SectorBoundary, Side, for_each_run_station,
};
use super::mesh::{CURB_FACE_GROUP, GROUPS, PieceMesh, RowLayout, push_section_row_at, row_slots};
use super::profile::{Level, RoadProfile};
use super::span::{MouthSection, edge_points};
use super::{BandKind, RoadBand, RoadSection, RoadVec2, RoadVec3};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::graph::rebuild::JunctionEndpointProfilePlane;
use godot::prelude::Vector3;
use spade::handles::{FaceHandle, FixedFaceHandle, InnerTag, PossiblyOuterTag};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

// A mouth's finalized centreline may differ from the node plane by at most this much.
const MOUTH_PLANE_TOLERANCE_M: f64 = 0.001;

/// Why a node has no piece. Every variant is a placement rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeError {
    /// The junction layout failed.
    Layout(LayoutError),
    /// The profile solve left no plane at the node.
    NoPlane,
    /// A finalized centreline is more than 1 mm from the node plane at a mouth.
    MouthOffPlane {
        /// Arm whose mouth is off the plane.
        arm: ArmKey,
    },
    /// The constrained triangulation rejected the carriageway core loop.
    Triangulation,
}

/// Immutable piece of one junction, bend or terminal node.
#[derive(Debug)]
pub struct NodePiece {
    node: u32,
    layout: JunctionLayout,
    plane: JunctionEndpointProfilePlane,
    // Ascending arm key.
    mouths: Box<[(ArmKey, Arc<MouthSection>)]>,
    // Section at each crossable arm's crosswalk inner edge, ascending arm key.
    approaches: Box<[(ArmKey, RoadSection)]>,
    mesh: Arc<PieceMesh>,
    footprint: PieceFootprint,
}

impl NodePiece {
    /// Node id.
    pub fn node(&self) -> u32 {
        self.node
    }

    /// Degree class of the node.
    pub fn kind(&self) -> NodeKind {
        self.layout.kind
    }

    /// The layout the piece was built from.
    pub fn layout(&self) -> &JunctionLayout {
        &self.layout
    }

    /// Mouth section of each arm, in ascending arm key order.
    pub fn mouths(&self) -> &[(ArmKey, Arc<MouthSection>)] {
        &self.mouths
    }

    /// The piece's mesh.
    pub fn mesh(&self) -> &PieceMesh {
        &self.mesh
    }

    pub(super) fn shared_mesh(&self) -> &Arc<PieceMesh> {
        &self.mesh
    }

    pub(super) fn footprint(&self) -> &PieceFootprint {
        &self.footprint
    }

    pub(super) fn mouth(&self, key: ArmKey) -> Option<&Arc<MouthSection>> {
        mouth_of(&self.mouths, key)
    }

    /// Height of the piece's surface at world `xz` for a band of `level`: its node plane
    /// `P(x, z) + level`, the function every vertex of the piece was placed with.
    pub fn surface_height_m(&self, xz: RoadVec2, level: Level) -> f32 {
        surface_height_m(&self.plane, xz, level)
    }

    /// The section of crossable arm `key` at its crosswalk's inner edge, in its edge's forward
    /// frame and span stations. With the arm's mouth section it bounds the zebra strip: the
    /// straight approach that road furniture at the crossing stands beside.
    pub fn crosswalk_approach(&self, key: ArmKey) -> Option<&RoadSection> {
        self.approaches
            .binary_search_by_key(&key, |(arm, _)| *arm)
            .ok()
            .map(|index| &self.approaches[index].1)
    }

    /// Kerb points of arm `key`'s zebra crossing on the piece's carriageway, right side first:
    /// the layout's [`Crosswalk`] kerbs at `P(x, z)`. `None` when the arm has no crossing.
    /// Pedestrian lanes publish the same points, through the same function.
    pub fn crosswalk_kerbs(&self, key: ArmKey) -> Option<[Vector3; 2]> {
        let arm = self.layout.arms.iter().find(|arm| arm.key == key)?;
        Some(crossing_kerbs(&self.plane, arm.crosswalk.as_ref()?))
    }
}

/// A crossing's kerb points on the node plane, right side first: `(x, P(x, z), z)` in `f32`. The
/// one function both node pieces and pedestrian lanes publish crossings with.
pub(super) fn crossing_kerbs(
    plane: &JunctionEndpointProfilePlane,
    crosswalk: &Crosswalk,
) -> [Vector3; 2] {
    crosswalk.kerbs.map(|kerb| {
        Vector3::new(
            kerb.x as f32,
            surface_height_m(plane, kerb, Level::Ground),
            kerb.y as f32,
        )
    })
}

fn mouth_of(mouths: &[(ArmKey, Arc<MouthSection>)], key: ArmKey) -> Option<&Arc<MouthSection>> {
    mouths
        .binary_search_by_key(&key, |(arm, _)| *arm)
        .ok()
        .map(|index| &mouths[index].1)
}

/// A node's arms, layout, plane and mouth sections, before its mesh.
pub(super) struct NodePlan {
    arms: NodeArms,
    layout: JunctionLayout,
    plane: JunctionEndpointProfilePlane,
    // Mouth station of each arm, in input order.
    stations: Vec<f64>,
    // Ascending arm key, the arms' input order.
    mouths: Vec<(ArmKey, Arc<MouthSection>)>,
    approaches: Vec<(ArmKey, RoadSection)>,
}

impl NodePlan {
    pub(super) fn mouth(&self, key: ArmKey) -> Option<&Arc<MouthSection>> {
        mouth_of(&self.mouths, key)
    }

    // Replaces this node's mouth on an edge whose two mouths coincide with the other node's.
    pub(super) fn share_mouth(&mut self, key: ArmKey, mouth: Arc<MouthSection>) {
        if let Ok(index) = self.mouths.binary_search_by_key(&key, |(arm, _)| *arm) {
            self.mouths[index].1 = mouth;
        }
    }
}

/// Height of the node surface at `xz` for a band of `level`: `P(x, z) + level`. The one height
/// function of node pieces and their mouth sections.
pub(super) fn surface_height_m(
    plane: &JunctionEndpointProfilePlane,
    xz: RoadVec2,
    level: Level,
) -> f32 {
    plane.height_at_xz(xz.x as f32, xz.y as f32) + level.offset_m()
}

/// Plans `node`: `Ok(None)` when it has no piece in this gate (see [`NodeArms::collect`]).
pub(super) fn plan_node(graph: &RegionGraph, node: u32) -> Result<Option<NodePlan>, NodeError> {
    let Some(arms) = NodeArms::collect(graph, node) else {
        return Ok(None);
    };
    let layout = arms.layout().map_err(NodeError::Layout)?;
    let mut stations = vec![0.0; arms.len()];
    for arm in &layout.arms {
        stations[arm.input_index] = arm.mouth_station_m;
    }
    let plane = match layout.kind {
        NodeKind::Terminal => {
            let (_, height_m) = arms.centreline_at(0, stations[0]);
            let mouth = arms.input(0).offset_point(stations[0], 0.0);
            let (node_xz, node_y) = (arms.position, arms.node_height_m(0));
            JunctionEndpointProfilePlane::terminal(
                Vector3::new(node_xz.x as f32, node_y as f32, node_xz.y as f32),
                Vector3::new(mouth.x as f32, height_m as f32, mouth.y as f32),
            )
        }
        NodeKind::Bend | NodeKind::Junction => {
            graph.solved_junction_endpoint_profile_plane(arms.node)
        }
    }
    .ok_or(NodeError::NoPlane)?;
    let mouths = (0..arms.len())
        .map(|index| {
            let section = mouth_section(graph, &arms, index, stations[index], &plane)?;
            Ok((arms.key(index), Arc::new(section)))
        })
        .collect::<Result<Vec<_>, NodeError>>()?;
    let mut approaches: Vec<_> = layout
        .arms
        .iter()
        .filter_map(|arm| {
            let crosswalk = arm.crosswalk?;
            let station_m = crosswalk.inner_station_m;
            let (section, _) = section_at(graph, &arms, arm.input_index, station_m, &plane);
            Some((arm.key, section))
        })
        .collect();
    approaches.sort_unstable_by_key(|(key, _)| *key);
    Ok(Some(NodePlan {
        arms,
        layout,
        plane,
        stations,
        mouths,
        approaches,
    }))
}

/// Checks that `node` lays out and its mouths sit on its plane, as its piece would need; nodes
/// without a piece pass. Placement validation runs it on the post-edit excerpt.
pub(crate) fn check_node(graph: &RegionGraph, node: u32) -> Result<(), NodeError> {
    plan_node(graph, node).map(|_| ())
}

fn lateral_sign(end: EdgeEnd) -> f64 {
    match end {
        EdgeEnd::Start => 1.0,
        EdgeEnd::End => -1.0,
    }
}

// The mouth section of arm `index` at `station_m`, in its edge's forward frame: vertices on the
// arm's offset curves, heights on the node plane.
fn mouth_section(
    graph: &RegionGraph,
    arms: &NodeArms,
    index: usize,
    station_m: f64,
    plane: &JunctionEndpointProfilePlane,
) -> Result<MouthSection, NodeError> {
    let key = arms.key(index);
    let (section, centre_height_m) = section_at(graph, arms, index, station_m, plane);
    if (centre_height_m - f64::from(section.center_height_m)).abs() > MOUTH_PLANE_TOLERANCE_M {
        return Err(NodeError::MouthOffPlane { arm: key });
    }
    let profile = RoadProfile::for_edge(graph.edge(key.edge));
    let sign = lateral_sign(key.end);
    let input = arms.input(index);
    let xz = |lateral_m: f32| input.offset_point(station_m, sign * f64::from(lateral_m));
    let row = RowLayout::new(&profile);
    let mut vertices = Vec::with_capacity(row.stride());
    push_section_row_at(&section, &row, xz, &mut vertices);
    Ok(MouthSection {
        section,
        profile,
        vertices: vertices.into_boxed_slice(),
    })
}

// Arm `index`'s cross-section at `station_m`, in its edge's forward frame and span stations, on
// the arm's offset curves with heights on the node plane; and the finalized centreline height
// there.
fn section_at(
    graph: &RegionGraph,
    arms: &NodeArms,
    index: usize,
    station_m: f64,
    plane: &JunctionEndpointProfilePlane,
) -> (RoadSection, f64) {
    let key = arms.key(index);
    let edge = graph.edge(key.edge);
    let input = arms.input(index);
    let sign = lateral_sign(key.end);
    let xz = |lateral_m: f32| input.offset_point(station_m, sign * f64::from(lateral_m));
    let (position, centre_height_m) = arms.centreline_at(index, station_m);
    let center_xz = xz(0.0);
    let center_height_m = surface_height_m(plane, center_xz, Level::Ground);
    let profile = RoadProfile::for_edge(edge);
    let lateral_xz = ((input.offset_point(station_m, 1.0) - center_xz) * sign).normalize();
    let section = RoadSection {
        edge_idx: key.edge,
        s_m: span_station_m(edge_points(edge), position),
        center_xz,
        center_height_m,
        tangent_xz: RoadVec2::new(lateral_xz.y, -lateral_xz.x),
        lateral_xz,
        bands: profile
            .bands()
            .iter()
            .map(|band| RoadBand {
                kind: band.kind,
                lateral_start_m: band.start_m,
                lateral_end_m: band.end_m,
                height_start_m: surface_height_m(plane, xz(band.start_m), band.level),
                height_end_m: surface_height_m(plane, xz(band.end_m), band.level),
            })
            .collect(),
    };
    (section, centre_height_m)
}

// The span station of a centreline position: the span model's cumulative 3-D length in `f32`.
fn span_station_m(points: &[Vector3], position: CentrelinePosition) -> f32 {
    let road = |point: Vector3| RoadVec3::new(point.x.into(), point.y.into(), point.z.into());
    let (mut running, mut at_lo) = (0.0_f32, 0.0_f32);
    for index in 0..position.hi {
        if index == position.lo {
            at_lo = running;
        }
        running += road(points[index]).distance(road(points[index + 1])) as f32;
    }
    at_lo + (running - at_lo) * position.t as f32
}

// Multiply-rotate hash of exact vertex bits: deduplication runs a few thousand lookups per piece,
// where SipHash dominated. Only lookups use it, never iteration order.
#[derive(Default)]
struct PointHasher(u64);

impl Hasher for PointHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

type VertexIds = HashMap<[u64; 3], u32, BuildHasherDefault<PointHasher>>;

/// Reusable buffers for building node pieces; one per Rayon worker.
#[derive(Default)]
pub(crate) struct NodeScratch {
    vertex_ids: VertexIds,
    ring: Vec<u32>,
    outside: Vec<bool>,
    stack: Vec<FixedFaceHandle<InnerTag>>,
    lines: Vec<LinePoint>,
    sample_ids: Vec<u32>,
    footprint: Vec<(i32, i32)>,
}

// One point of a boundary's run along an arm side, from the sector to the mouth.
#[derive(Clone, Copy)]
struct LinePoint {
    station_m: f64,
    xz: RoadVec2,
    // Index of the arm (input order) whose mouth this point is, with its layout offset.
    mouth: Option<(usize, f64)>,
}

struct MeshBuilder<'a> {
    plane: &'a JunctionEndpointProfilePlane,
    arms: &'a NodeArms,
    stations: &'a [f64],
    mouths: &'a [(ArmKey, Arc<MouthSection>)],
    positions: Vec<RoadVec3>,
    ids: &'a mut VertexIds,
    groups: [Vec<[u32; 3]>; GROUPS],
}

impl MeshBuilder<'_> {
    fn vertex_at(&mut self, point: RoadVec3) -> u32 {
        let key = [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()];
        let next = self.positions.len() as u32;
        let id = *self.ids.entry(key).or_insert(next);
        if id == next {
            self.positions.push(point);
        }
        id
    }

    fn vertex(&mut self, xz: RoadVec2, level: Level) -> u32 {
        let y = f64::from(surface_height_m(self.plane, xz, level));
        self.vertex_at(RoadVec3::new(xz.x, y, xz.y))
    }

    // The mouth section's own vertex at a layout offset and level on arm `arm` (input order).
    fn mouth_vertex(&mut self, arm: usize, offset_m: f64, level: Level) -> u32 {
        let mouth = &self.mouths[arm].1;
        let lateral = (lateral_sign(self.arms.key(arm).end) * offset_m) as f32;
        let slot = row_slots(&mouth.profile).position(|slot| slot == (lateral, level));
        let point = slot.map(|slot| mouth.vertices[slot]);
        match point {
            Some(point) => self.vertex_at(point),
            None => {
                let xz = self
                    .arms
                    .input(arm)
                    .offset_point(self.stations[arm], offset_m);
                self.vertex(xz, level)
            }
        }
    }

    fn line_vertex(&mut self, point: LinePoint, level: Level) -> u32 {
        match point.mouth {
            Some((arm, offset_m)) => self.mouth_vertex(arm, offset_m, level),
            None => self.vertex(point.xz, level),
        }
    }

    // A top triangle, wound counter-clockwise in XZ so it faces up; degenerate ones are dropped.
    fn top(&mut self, kind: BandKind, [a, b, c]: [u32; 3]) {
        if a == b || b == c || a == c {
            return;
        }
        let xz = |vertex: u32| {
            let point = self.positions[vertex as usize];
            robust::Coord {
                x: point.x,
                y: point.z,
            }
        };
        let orientation = robust::orient2d(xz(a), xz(b), xz(c));
        let group = &mut self.groups[kind as usize];
        if orientation > 0.0 {
            group.push([a, b, c]);
        } else if orientation < 0.0 {
            group.push([a, c, b]);
        }
    }

    // A vertical curb face between `p` and `q`, each a (lower, upper) vertex pair, facing the
    // left of `p -> q` when `face_left`.
    fn curb(&mut self, p: (u32, u32), q: (u32, u32), face_left: bool) {
        let ((p_low, p_high), (q_low, q_high)) = if face_left { (p, q) } else { (q, p) };
        if p_low == q_low {
            return;
        }
        let faces = &mut self.groups[CURB_FACE_GROUP];
        faces.push([p_high, q_high, q_low]);
        faces.push([p_high, q_low, p_low]);
    }

    // Curb faces along a run of points between an inner and an outer band of different levels;
    // `inner_left` says on which side of the run's direction the inner band lies.
    fn curb_run(
        &mut self,
        points: impl Iterator<Item = LinePoint>,
        boundary: &SectorBoundary,
        inner_left: bool,
    ) {
        let Some(outer) = boundary.outer else {
            return;
        };
        if outer.level == boundary.inner.level {
            return;
        }
        let inner_lower = boundary.inner.level == Level::Ground;
        let (low, high) = if inner_lower {
            (boundary.inner.level, outer.level)
        } else {
            (outer.level, boundary.inner.level)
        };
        let mut previous: Option<(u32, u32)> = None;
        for point in points {
            let pair = (self.line_vertex(point, low), self.line_vertex(point, high));
            if let Some(previous) = previous {
                self.curb(previous, pair, inner_left == inner_lower);
            }
            previous = Some(pair);
        }
    }

    // Zips the strip between two runs ordered by station.
    fn zip(&mut self, kind: BandKind, level: Level, inner: &[LinePoint], outer: &[LinePoint]) {
        let (mut i, mut j) = (0, 0);
        while i + 1 < inner.len() || j + 1 < outer.len() {
            let advance_inner = j + 1 == outer.len()
                || (i + 1 < inner.len() && inner[i + 1].station_m <= outer[j + 1].station_m);
            let (a, b) = (
                self.line_vertex(inner[i], level),
                self.line_vertex(outer[j], level),
            );
            let c = if advance_inner {
                i += 1;
                self.line_vertex(inner[i], level)
            } else {
                j += 1;
                self.line_vertex(outer[j], level)
            };
            self.top(kind, [a, b, c]);
        }
    }
}

// The run of `boundary` along one side of arm `arm` (sorted index): its sector endpoint sample,
// the knots and taper station strictly between, and the mouth, at the run's offsets (D4).
fn push_side_run(
    lines: &mut Vec<LinePoint>,
    layout: &JunctionLayout,
    input: &ArmInput<'_>,
    arm: usize,
    side: Side,
    boundary: &SectorBoundary,
) {
    let samples = &layout.samples[boundary.samples.clone()];
    let (station_m, start) = match side {
        Side::Left => (boundary.from_station_m, samples[0]),
        Side::Right => (boundary.to_station_m, samples[samples.len() - 1]),
    };
    let mouth_m = layout.arms[arm].mouth_station_m;
    lines.push(LinePoint {
        station_m,
        xz: start,
        mouth: None,
    });
    let taper_m = boundary.taper.map(|taper| taper.station_m);
    for_each_run_station(input, station_m, mouth_m, taper_m, |station_m, _| {
        lines.push(LinePoint {
            station_m,
            xz: boundary.run_point(input, side, station_m),
            mouth: None,
        });
    });
    let offset_m = boundary.run_offset_m(side, mouth_m);
    lines.push(LinePoint {
        station_m: mouth_m,
        xz: input.offset_point(mouth_m, offset_m),
        mouth: Some((layout.arms[arm].input_index, offset_m)),
    });
}

/// Builds the piece of a planned node. Mouth vertices come from the plan's mouth sections; an
/// edge whose two mouths coincide shares one section between both nodes
/// ([`NodePlan::share_mouth`]).
pub(super) fn build_node_piece(
    plan: NodePlan,
    grid: RenderGrid,
    scratch: &mut NodeScratch,
) -> Result<NodePiece, NodeError> {
    let NodeScratch {
        vertex_ids,
        ring,
        outside,
        stack,
        lines,
        sample_ids,
        footprint,
    } = scratch;
    vertex_ids.clear();
    let layout = &plan.layout;
    let mut builder = MeshBuilder {
        plane: &plan.plane,
        arms: &plan.arms,
        stations: &plan.stations,
        mouths: &plan.mouths,
        positions: Vec::with_capacity(layout.samples.len() * 2 + layout.core.len()),
        ids: vertex_ids,
        groups: Default::default(),
    };
    for (_, mouth) in &plan.mouths {
        for &vertex in mouth.vertices.iter() {
            builder.vertex_at(vertex);
        }
    }
    core_ring(&mut builder, &plan, ring);
    if !fan_core(&mut builder, plan.arms.position, ring) {
        triangulate_core(&mut builder, ring, outside, stack)?;
    }
    let mut chains = Vec::with_capacity(layout.sectors.len());
    for (index, sector) in layout.sectors.iter().enumerate() {
        let boundaries = &layout.boundaries[sector.boundaries.clone()];
        sector_strips(&mut builder, layout, boundaries, sample_ids);
        // Arm sides: this sector's from-arm left side and to-arm right side.
        for (arm, side) in [(sector.from_arm, Side::Left), (sector.to_arm, Side::Right)] {
            let input = plan.arms.input(layout.arms[arm].input_index);
            lines.clear();
            let mut runs = Vec::with_capacity(boundaries.len());
            for boundary in boundaries {
                let start = lines.len();
                push_side_run(lines, layout, &input, arm, side, boundary);
                runs.push(start..lines.len());
            }
            for (j, boundary) in boundaries.iter().enumerate() {
                let run = &lines[runs[j].clone()];
                // Walking outward, the inner band lies right of a left side's runs.
                builder.curb_run(run.iter().copied(), boundary, side == Side::Right);
                if let Some(outer) = boundary.outer {
                    let next = &lines[runs[j + 1].clone()];
                    builder.zip(outer.kind, outer.level, run, next);
                }
            }
            // Outer chain: back along the from-arm's outermost run, across the sector's
            // outermost samples, out along the to-arm's.
            let outermost = &boundaries[boundaries.len() - 1];
            let level = outermost.inner.level;
            let run = &lines[runs[runs.len() - 1].clone()];
            let ids = run.iter().map(|&point| builder.line_vertex(point, level));
            if side == Side::Left {
                let mut chain: Vec<u32> = ids.collect();
                chain.reverse();
                chains.push(chain);
            } else {
                let ids: Vec<u32> = ids.collect();
                let samples = &layout.samples[outermost.samples.clone()];
                let across: Vec<u32> = samples
                    .iter()
                    .map(|&sample| builder.vertex(sample, level))
                    .collect();
                let chain = &mut chains[index];
                chain.extend(across);
                chain.extend(ids);
            }
        }
    }
    let outer_chains = chains
        .into_iter()
        .map(|mut chain| {
            chain.dedup();
            chain.into_boxed_slice()
        })
        .collect();
    let MeshBuilder {
        positions, groups, ..
    } = builder;
    let road_space = positions
        .iter()
        .map(|point| [point.x as f32, point.z as f32])
        .collect();
    let mesh = PieceMesh::from_groups(positions, road_space, &groups, outer_chains);
    let footprint = PieceFootprint::of_bounds(mesh.bounds_xz(), grid, footprint);
    Ok(NodePiece {
        node: plan.arms.node,
        layout: plan.layout,
        plane: plan.plane,
        mouths: plan.mouths.into_boxed_slice(),
        approaches: plan.approaches.into_boxed_slice(),
        mesh: Arc::new(mesh),
        footprint,
    })
}

// Strips between consecutive sector boundaries, sample pair by sample pair, and the curb faces
// along the sector's level-changing boundaries.
fn sector_strips(
    builder: &mut MeshBuilder<'_>,
    layout: &JunctionLayout,
    boundaries: &[SectorBoundary],
    ids: &mut Vec<u32>,
) {
    // Vertex ids of every boundary's samples at both levels, looked up once.
    let count = layout.samples[boundaries[0].samples.clone()].len();
    ids.clear();
    for boundary in boundaries {
        for level in [Level::Ground, Level::Raised] {
            for &sample in &layout.samples[boundary.samples.clone()] {
                let used = boundary.inner.level == level
                    || boundary.outer.is_some_and(|outer| outer.level == level);
                ids.push(if used {
                    builder.vertex(sample, level)
                } else {
                    u32::MAX
                });
            }
        }
    }
    let row = |j: usize, level: Level| {
        let start = (2 * j + usize::from(level == Level::Raised)) * count;
        start..start + count
    };
    for (j, boundary) in boundaries.iter().enumerate() {
        // Samples run from the from-arm to the to-arm, with the inner band on their left.
        if let Some(outer) = boundary.outer
            && outer.level != boundary.inner.level
        {
            let (low, high) = if boundary.inner.level == Level::Ground {
                (row(j, boundary.inner.level), row(j, outer.level))
            } else {
                (row(j, outer.level), row(j, boundary.inner.level))
            };
            let face_left = boundary.inner.level == Level::Ground;
            for i in 0..count - 1 {
                let p = (ids[low.start + i], ids[high.start + i]);
                let q = (ids[low.start + i + 1], ids[high.start + i + 1]);
                builder.curb(p, q, face_left);
            }
        }
        let Some(BoundaryBand { kind, level }) = boundary.outer else {
            continue;
        };
        let (this, next) = (row(j, level), row(j + 1, level));
        for i in 0..count - 1 {
            let a = ids[this.start + i];
            let b = ids[this.start + i + 1];
            let c = ids[next.start + i + 1];
            let d = ids[next.start + i];
            builder.top(kind, [a, b, c]);
            builder.top(kind, [a, c, d]);
        }
    }
}

// The carriageway core ring: the layout's core loop at ground level, with each mouth row's
// interior carriageway vertices inserted between its two carriageway-edge corners. The loop drops
// a mouth's right corner when the corner before it ends on the same point, never its left one,
// so the interior goes in just before the left corner.
fn core_ring(builder: &mut MeshBuilder<'_>, plan: &NodePlan, ring: &mut Vec<u32>) {
    let layout = &plan.layout;
    ring.clear();
    for vertex in &layout.core {
        let id = match vertex.source {
            CoreVertexSource::Mouth { arm, side } => {
                let input_index = layout.arms[arm].input_index;
                let input = plan.arms.input(input_index);
                let (right, left) = (-input.right.core_offset(), input.left.core_offset());
                if side == Side::Left {
                    push_mouth_interior(builder, plan, input_index, (right, left), ring);
                }
                let offset_m = if side == Side::Right { right } else { left };
                builder.mouth_vertex(input_index, offset_m, Level::Ground)
            }
            _ => builder.vertex(vertex.position, Level::Ground),
        };
        ring.push(id);
    }
}

// Mouth-row ground vertices strictly between the two carriageway edges, right to left.
fn push_mouth_interior(
    builder: &mut MeshBuilder<'_>,
    plan: &NodePlan,
    arm: usize,
    (right, left): (f64, f64),
    ring: &mut Vec<u32>,
) {
    let mouth = &plan.mouths[arm].1;
    let sign = lateral_sign(plan.arms.key(arm).end);
    let interior = |(lateral, level): (f32, Level)| {
        let offset_m = sign * f64::from(lateral);
        (level == Level::Ground && offset_m > right && offset_m < left).then_some(offset_m)
    };
    let slots = row_slots(&mouth.profile).filter_map(interior);
    let ordered: Vec<f64> = if sign > 0.0 {
        slots.collect()
    } else {
        let mut reversed: Vec<f64> = slots.collect();
        reversed.reverse();
        reversed
    };
    for offset_m in ordered {
        ring.push(builder.mouth_vertex(arm, offset_m, Level::Ground));
    }
}

// Fans the core ring from the node when the node sees every ring edge strictly from its left
// (exact orientation), which holds for straight arms. That is an exact triangulation of the same
// loop without the Delaunay incircle tests, whose exact fallback the cocircular corner samples
// trigger on every flip. `O(n)`; returns false, emitting nothing, when the node does not see
// the whole loop.
fn fan_core(builder: &mut MeshBuilder<'_>, node: RoadVec2, ring: &[u32]) -> bool {
    let coord = |point: RoadVec2| robust::Coord {
        x: point.x,
        y: point.y,
    };
    let xz = |builder: &MeshBuilder<'_>, vertex: u32| {
        let point = builder.positions[vertex as usize];
        RoadVec2::new(point.x, point.z)
    };
    let sees_every_edge = (0..ring.len()).all(|index| {
        let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
        robust::orient2d(coord(node), coord(xz(builder, a)), coord(xz(builder, b))) > 0.0
    });
    if !sees_every_edge {
        return false;
    }
    let centre = builder.vertex(node, Level::Ground);
    for index in 0..ring.len() {
        let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
        builder.groups[BandKind::Carriageway as usize].push([centre, a, b]);
    }
    true
}

// Triangulates the core ring and keeps the faces the ring encloses: those not reachable from the
// outer face without crossing a ring edge. `O(n log n)`.
fn triangulate_core(
    builder: &mut MeshBuilder<'_>,
    ring: &[u32],
    outside: &mut Vec<bool>,
    stack: &mut Vec<FixedFaceHandle<InnerTag>>,
) -> Result<(), NodeError> {
    let points: Vec<Point2<f64>> = ring
        .iter()
        .map(|&vertex| {
            let point = builder.positions[vertex as usize];
            Point2::new(point.x, point.z)
        })
        .collect();
    let edges = (0..ring.len())
        .map(|index| [index, (index + 1) % ring.len()])
        .collect();
    let mut conflicts = 0usize;
    let cdt =
        ConstrainedDelaunayTriangulation::<Point2<f64>>::try_bulk_load_cdt(points, edges, |_| {
            conflicts += 1
        })
        .map_err(|_| NodeError::Triangulation)?;
    if conflicts > 0 || cdt.num_vertices() != ring.len() {
        return Err(NodeError::Triangulation);
    }
    outside.clear();
    outside.resize(cdt.num_all_faces(), false);
    stack.clear();
    let reach = |face: FaceHandle<'_, PossiblyOuterTag, _, _, _, _>,
                 outside: &mut Vec<bool>,
                 stack: &mut Vec<FixedFaceHandle<InnerTag>>| {
        if let Some(inner) = face.as_inner()
            && !outside[face.fix().index()]
        {
            outside[face.fix().index()] = true;
            stack.push(inner.fix());
        }
    };
    for edge in cdt.directed_edges() {
        if edge.face().is_outer() && !edge.as_undirected().data().is_constraint_edge() {
            reach(edge.rev().face(), outside, stack);
        }
    }
    while let Some(face) = stack.pop() {
        for edge in cdt.face(face).adjacent_edges() {
            if !edge.as_undirected().data().is_constraint_edge() {
                reach(edge.rev().face(), outside, stack);
            }
        }
    }
    for face in cdt.inner_faces() {
        if outside[face.fix().index()] {
            continue;
        }
        let [a, b, c] = face.vertices().map(|vertex| ring[vertex.fix().index()]);
        builder.top(BandKind::Carriageway, [a, b, c]);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
