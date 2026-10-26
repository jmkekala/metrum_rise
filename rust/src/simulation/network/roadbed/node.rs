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
    LayoutError, NodeKind, PathPoint, SectorBoundary, Side,
};
use super::mesh::{
    CURB_FACE_GROUP, GROUPS, PieceMesh, RowLayout, StripKind, push_section_row_at, row_slots,
    wall_run,
};
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
    /// The profile solve found no plane at the node: an arm too short for its sample, or samples
    /// that fix no plane.
    NoPlane,
    /// A finalized centreline is more than 1 mm from the node plane at a mouth.
    MouthOffPlane {
        /// Arm whose mouth is off the plane.
        arm: ArmKey,
    },
    /// The constrained triangulation rejected the carriageway core loop.
    Triangulation,
    /// A run or the core reaches a mouth at an offset and level its mouth section does not hold.
    MouthVertex {
        /// Arm whose mouth section lacks the vertex.
        arm: ArmKey,
    },
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
    // Whether the piece stands on the terrain (`NodeArms::grounded`).
    grounded: bool,
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

    // Whether the piece stands on the terrain: a standard edge meets the node. Where only
    // bridges and tunnels meet it hangs no skirt and gives the terrain no seam or coverage.
    pub(super) fn grounded(&self) -> bool {
        self.grounded
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
    /// frame and span stations: where the straight approach that road furniture at the crossing
    /// stands beside begins. The approach runs on to the mouth section.
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
    let Some(mut arms) = NodeArms::collect(graph, node) else {
        return Ok(None);
    };
    let layout = arms.layout().map_err(NodeError::Layout)?;
    let mut stations = vec![0.0; arms.len()];
    for arm in &layout.arms {
        stations[arm.input_index] = arm.mouth_station_m;
    }
    for attachment in &layout.attachments {
        stations[attachment.input_index] = attachment.mouth_station_m;
    }
    let plane = match layout.kind {
        NodeKind::Terminal => layout_terminal_plane(&arms, &layout),
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
        mouths,
        approaches,
    }))
}

// A road end's plane: through the road end and its mouth centreline point, with the arm's end
// grade along that run and no crossfall.
fn layout_terminal_plane(
    arms: &NodeArms,
    layout: &JunctionLayout,
) -> Option<JunctionEndpointProfilePlane> {
    let (arm, station_m) = (layout.arms[0].input_index, layout.arms[0].mouth_station_m);
    let (_, height_m) = arms.centreline_at(arm, station_m);
    let mouth = arms.input(arm).offset_point(station_m, 0.0);
    let (node_xz, node_y) = (arms.position, arms.node_height_m(arm));
    JunctionEndpointProfilePlane::terminal(
        Vector3::new(node_xz.x as f32, node_y as f32, node_xz.y as f32),
        Vector3::new(mouth.x as f32, height_m as f32, mouth.y as f32),
    )
}

/// The plane of the road end or footpath end at `node`, as its node piece uses it; `None` at any
/// other node. The profile solve pins a road end's attached footpaths to it (D5).
pub(crate) fn terminal_plane(
    graph: &RegionGraph,
    node: u32,
) -> Option<JunctionEndpointProfilePlane> {
    let mut arms = NodeArms::collect(graph, node)?;
    let layout = arms.layout().ok()?;
    (layout.kind == NodeKind::Terminal)
        .then(|| layout_terminal_plane(&arms, &layout))
        .flatten()
}

/// Checks that `node` lays out, that the profile solve found its plane (a bend or junction that
/// it pins), and that its mouths sit on that plane, as its piece would need; nodes without a
/// piece pass. Placement validation runs it on the post-edit excerpt.
pub(crate) fn check_node(graph: &RegionGraph, node: u32) -> Result<(), NodeError> {
    let plan = plan_node(graph, node)?;
    if plan.is_some_and(|plan| plan.layout.kind != NodeKind::Terminal)
        && graph.junction_profile_solve_fails(node)
    {
        return Err(NodeError::NoPlane);
    }
    Ok(())
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
    let level = |level: Level| arms.level_at_node(index, level);
    let center_height_m = surface_height_m(plane, center_xz, level(Level::Ground));
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
                height_start_m: surface_height_m(plane, xz(band.start_m), level(band.level)),
                height_end_m: surface_height_m(plane, xz(band.end_m), level(band.level)),
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
    lines: Vec<PathPoint>,
    path: Vec<PathPoint>,
    sample_ids: Vec<u32>,
    footprint: Vec<(i32, i32)>,
}

struct MeshBuilder<'a> {
    plane: &'a JunctionEndpointProfilePlane,
    arms: &'a NodeArms,
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

    // The mouth section's own vertex at a layout offset and level on arm `arm` (input order). An
    // attached arm's single band stands at sidewalk height whatever its profile level. Layout
    // offsets at a mouth are the profile's own band edges (`ArmBand::outer_m`, exact), so a
    // missing slot is a construction defect, never re-derived.
    fn mouth_vertex(&mut self, arm: usize, offset_m: f64, level: Level) -> Result<u32, NodeError> {
        let mouth = &self.mouths[arm].1;
        let key = self.arms.key(arm);
        let lateral = (lateral_sign(key.end) * offset_m) as f32;
        let attached = self.arms.is_attached(arm);
        let slot = row_slots(&mouth.profile)
            .position(|slot| slot.0 == lateral && (attached || slot.1 == level))
            .ok_or(NodeError::MouthVertex { arm: key })?;
        let point = mouth.vertices[slot];
        Ok(self.vertex_at(point))
    }

    fn line_vertex(&mut self, point: PathPoint, level: Level) -> Result<u32, NodeError> {
        match point.mouth {
            Some((arm, offset_m)) => self.mouth_vertex(arm, offset_m, level),
            None => Ok(self.vertex(point.xz, level)),
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
        points: impl Iterator<Item = PathPoint>,
        boundary: &SectorBoundary,
        inner_left: bool,
    ) -> Result<(), NodeError> {
        let Some(outer) = boundary.outer else {
            return Ok(());
        };
        if outer.level == boundary.inner.level {
            return Ok(());
        }
        let inner_lower = boundary.inner.level == Level::Ground;
        let (low, high) = if inner_lower {
            (boundary.inner.level, outer.level)
        } else {
            (outer.level, boundary.inner.level)
        };
        let mut previous: Option<(u32, u32)> = None;
        for point in points {
            let pair = (
                self.line_vertex(point, low)?,
                self.line_vertex(point, high)?,
            );
            if let Some(previous) = previous {
                self.curb(previous, pair, inner_left == inner_lower);
            }
            previous = Some(pair);
        }
        Ok(())
    }

    // Zips the strip between two runs ordered by station.
    fn zip(
        &mut self,
        kind: BandKind,
        level: Level,
        inner: &[PathPoint],
        outer: &[PathPoint],
    ) -> Result<(), NodeError> {
        let (mut i, mut j) = (0, 0);
        while i + 1 < inner.len() || j + 1 < outer.len() {
            let advance_inner = j + 1 == outer.len()
                || (i + 1 < inner.len() && inner[i + 1].station_m <= outer[j + 1].station_m);
            let (a, b) = (
                self.line_vertex(inner[i], level)?,
                self.line_vertex(outer[j], level)?,
            );
            let c = if advance_inner {
                i += 1;
                self.line_vertex(inner[i], level)?
            } else {
                j += 1;
                self.line_vertex(outer[j], level)?
            };
            self.top(kind, [a, b, c]);
        }
        Ok(())
    }
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
        path,
        sample_ids,
        footprint,
    } = scratch;
    vertex_ids.clear();
    let layout = &plan.layout;
    let mut builder = MeshBuilder {
        plane: &plan.plane,
        arms: &plan.arms,
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
    let inputs: Vec<ArmInput<'_>> = (0..plan.arms.len())
        .map(|index| plan.arms.input(index))
        .collect();
    // The core's band: the carriageway, or a footpath where only footpaths meet.
    let core_input = &inputs[layout.arms[0].input_index];
    let core_kind = core_input.left.bands[core_input.left.core_bands - 1].kind;
    core_ring(&mut builder, &plan, ring)?;
    if !fan_core(&mut builder, plan.arms.position, ring, core_kind) {
        triangulate_ring(&mut builder, ring, core_kind, outside, stack)?;
    }
    let mut chains = Vec::with_capacity(layout.sectors.len() + layout.attachments.len());
    for (index, sector) in layout.sectors.iter().enumerate() {
        let boundaries = &layout.boundaries[sector.boundaries.clone()];
        sector_strips(&mut builder, layout, boundaries, sample_ids);
        // Arm sides: this sector's from-arm left side and to-arm right side.
        for (arm, side) in [(sector.from_arm, Side::Left), (sector.to_arm, Side::Right)] {
            let input = &inputs[layout.arms[arm].input_index];
            lines.clear();
            let mut runs = Vec::with_capacity(boundaries.len());
            for (j, boundary) in boundaries.iter().enumerate() {
                let start = lines.len();
                layout.side_run(input, (arm, side), (boundary, j), lines);
                runs.push(start..lines.len());
            }
            for (j, boundary) in boundaries.iter().enumerate() {
                let run = &lines[runs[j].clone()];
                // Walking outward, the inner band lies right of a left side's runs.
                builder.curb_run(run.iter().copied(), boundary, side == Side::Right)?;
                if let Some(outer) = boundary.outer {
                    let next = &lines[runs[j + 1].clone()];
                    builder.zip(outer.kind, outer.level, run, next)?;
                }
            }
        }
        let level = boundaries[boundaries.len() - 1].inner.level;
        layout.outer_path(&inputs, index, path);
        outer_chains(
            &mut builder,
            layout,
            &inputs,
            (index, level),
            path,
            lines,
            &mut chains,
        )?;
        for attachment in (0..layout.attachments.len())
            .filter(|&attachment| layout.attachments[attachment].sector == index)
        {
            // Each attached arm's ribbon, from the outer boundary to its mouth at the boundary's
            // level, on the boundary's own vertices; triangulated as the core is.
            let input = &inputs[layout.attachments[attachment].input_index];
            layout
                .ribbon(input, attachment, path, lines)
                .ok_or(NodeError::Triangulation)?;
            ring.clear();
            for &point in lines.iter() {
                ring.push(builder.line_vertex(point, level)?);
            }
            let kind = input.left.bands[0].kind;
            triangulate_ring(&mut builder, ring, kind, outside, stack)?;
        }
    }
    let outer_chains: Vec<Box<[u32]>> = chains
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
    let grounded = plan.arms.grounded();
    let strips = grounded.then_some(StripKind::Skirt);
    // A grounded node closes its chains across each structure mouth with a wall run
    // (`TERRAIN-07-C`, T22 3 (a)): the mouth row from the corner where one chain ends to the
    // corner where the next starts.
    let id =
        |point: RoadVec3| vertex_ids[&[point.x.to_bits(), point.y.to_bits(), point.z.to_bits()]];
    let wall_runs = (0..plan.mouths.len())
        .filter(|&arm| grounded && plan.arms.is_structure(arm))
        .map(|arm| {
            let row = &plan.mouths[arm].1.vertices;
            let first = id(row[0]);
            let forward = outer_chains
                .iter()
                .any(|chain| chain.last() == Some(&first));
            let ids = row.iter().map(|&point| id(point));
            let run = if forward {
                wall_run(&positions, ids)
            } else {
                wall_run(&positions, ids.rev())
            };
            debug_assert!(
                outer_chains
                    .iter()
                    .any(|chain| chain.first() == run.vertices().last()),
                "a structure mouth's wall run ends where an outer chain starts"
            );
            run
        })
        .collect();
    let mesh = PieceMesh::from_groups(
        positions,
        road_space,
        &groups,
        (outer_chains, wall_runs),
        strips,
    );
    let footprint = PieceFootprint::of_bounds(mesh.bounds_xz(), grid, footprint);
    Ok(NodePiece {
        node: plan.arms.node,
        layout: plan.layout,
        plane: plan.plane,
        mouths: plan.mouths.into_boxed_slice(),
        approaches: plan.approaches.into_boxed_slice(),
        mesh: Arc::new(mesh),
        footprint,
        grounded,
    })
}

// The outer chains of sector `sector` along its outer path, at `level`: one from the from-arm's
// mouth corner to the to-arm's, cut at each attached arm. Its right edge run ends a chain at its
// mouth corner and its left edge run starts the next; the path between its two edge points lies
// under its ribbon.
fn outer_chains(
    builder: &mut MeshBuilder<'_>,
    layout: &JunctionLayout,
    inputs: &[ArmInput<'_>],
    (sector, level): (usize, Level),
    path: &[PathPoint],
    run: &mut Vec<PathPoint>,
    chains: &mut Vec<Vec<u32>>,
) -> Result<(), NodeError> {
    let mut attached = (0..layout.attachments.len())
        .filter(|&attachment| layout.attachments[attachment].sector == sector)
        .peekable();
    let mut chain = Vec::with_capacity(path.len());
    let mut under: Option<RoadVec2> = None;
    for &point in path {
        if under.is_some_and(|left| point.xz != left) {
            continue;
        }
        under = None;
        chain.push(builder.line_vertex(point, level)?);
        let Some(index) =
            attached.next_if(|&index| layout.attachments[index].edges[0].point == point.xz)
        else {
            continue;
        };
        let input = &inputs[layout.attachments[index].input_index];
        run.clear();
        layout.attached_run(input, index, Side::Right, run);
        for &point in &run[1..] {
            chain.push(builder.line_vertex(point, level)?);
        }
        chains.push(std::mem::take(&mut chain));
        run.clear();
        layout.attached_run(input, index, Side::Left, run);
        // Out at the mouth corner and back to the left edge point, which the path supplies.
        for &point in run[1..].iter().rev() {
            chain.push(builder.line_vertex(point, level)?);
        }
        under = Some(layout.attachments[index].edges[1].point);
    }
    chains.push(chain);
    Ok(())
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
fn core_ring(
    builder: &mut MeshBuilder<'_>,
    plan: &NodePlan,
    ring: &mut Vec<u32>,
) -> Result<(), NodeError> {
    let layout = &plan.layout;
    ring.clear();
    for vertex in &layout.core {
        let id = match vertex.source {
            CoreVertexSource::Mouth { arm, side } => {
                let input_index = layout.arms[arm].input_index;
                let input = plan.arms.input(input_index);
                let (right, left) = (-input.right.core_offset(), input.left.core_offset());
                if side == Side::Left {
                    push_mouth_interior(builder, plan, input_index, (right, left), ring)?;
                }
                let offset_m = if side == Side::Right { right } else { left };
                builder.mouth_vertex(input_index, offset_m, Level::Ground)?
            }
            _ => builder.vertex(vertex.position, Level::Ground),
        };
        ring.push(id);
    }
    Ok(())
}

// Mouth-row ground vertices strictly between the two carriageway edges, right to left.
fn push_mouth_interior(
    builder: &mut MeshBuilder<'_>,
    plan: &NodePlan,
    arm: usize,
    (right, left): (f64, f64),
    ring: &mut Vec<u32>,
) -> Result<(), NodeError> {
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
        ring.push(builder.mouth_vertex(arm, offset_m, Level::Ground)?);
    }
    Ok(())
}

// Fans the core ring from the node when the node sees every ring edge strictly from its left
// (exact orientation), which holds for straight arms. That is an exact triangulation of the same
// loop without the Delaunay incircle tests, whose exact fallback the cocircular corner samples
// trigger on every flip. `O(n)`; returns false, emitting nothing, when the node does not see
// the whole loop.
fn fan_core(builder: &mut MeshBuilder<'_>, node: RoadVec2, ring: &[u32], kind: BandKind) -> bool {
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
        builder.groups[kind as usize].push([centre, a, b]);
    }
    true
}

// Triangulates a simple ring (the core, or an attached arm's ribbon) as band `kind` and keeps the
// faces the ring encloses: those not reachable from the outer face without crossing a ring edge.
// `O(n log n)`.
fn triangulate_ring(
    builder: &mut MeshBuilder<'_>,
    ring: &[u32],
    kind: BandKind,
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
        builder.top(kind, [a, b, c]);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
