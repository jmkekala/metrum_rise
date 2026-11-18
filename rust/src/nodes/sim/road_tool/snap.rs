// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool snapping and constraints (`ROAD-112` round B, plan section 3, decision U2).
//!
//! A cursor has two degrees of freedom and each constraint removes one or two. [`resolve`] is the
//! precedence as one rule, a pure function of the candidates: a network node wins outright; else
//! the zoning grid where it captured the cursor (one rule, `grid_rule`, rounds GS and GS2: with
//! no `D` an edge in reach combines with it as [`SnapKind::GridEdge`], across the road its
//! centreline, along it the grid's position; else the grid's point, before any guide line); else
//! the angle lock `D` and the line `L` (a network edge in reach, else the square guide the
//! session's lock holds ([`square_lock`], round UI1), else the nearest guide line within
//! [`GUIDE_REACH_M`]) meet at their intersection when it lies within `L`'s reach of the
//! raw cursor, otherwise an edge wins over the angle and the angle over a guide line; two guide
//! lines in reach and no `D` meet at their corner; one constraint alone projects; map-edge and
//! self snap keep today's place when nothing above took the cursor; the length step rounds last,
//! except on a node, an edge or an intersection. Ties go to the lower candidate in the fixed
//! order (edges ascending by id).
//!
//! The candidates come from the existing indices only: the network snap from the 16 m node grid
//! and the edge R-tree ([`network_snap`], today's acquisition with a reused candidate buffer),
//! guide lines from the R-tree within a fixed [`GUIDE_RADIUS_M`] of the cursor
//! ([`collect_guides`]). Fixed world-metre tolerances, no zoom. Per move
//! `O(log E + K * segments)` for the `K` edges inside the fixed radii; no allocation once the
//! scratch buffers have grown to the neighbourhood's candidate counts.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction::{
    NetworkSnap, NetworkSnapTarget, StructureHit, closest_network_snap_at_hit_into,
    retained_network_snap_xz,
};
use glam::DVec2;
use godot::prelude::Vector3;

/// A guide line takes the cursor within this many metres of it (round UI1: 4 m, from 2 m). About
/// a lane's width (3.5 m) and a margin, so a cursor anywhere over the lane next to the guide finds
/// it; under the network snap's 5 m reach ([`super::pick::NETWORK_SNAP_REACH_M`]), whose edge
/// keeps precedence over a guide line anyway.
pub const GUIDE_REACH_M: f64 = 4.0;
/// Guide lines within this many metres of the cursor that do not hold it are drawn faint (round
/// UI1), so the player can find them before they capture: six times the reach (the user,
/// 2026-11-18: 24 m, from 12, so guides show sooner; the reach that captures stays 4 m).
pub const GUIDE_NEAR_M: f64 = 24.0;
/// At most this many faint guide lines are drawn ([`near_guides`]).
pub const GUIDE_NEAR_MAX: usize = 2;
/// The square lock (round UI1): a stroke's first leg from a road or node takes the line across the
/// road through its start (or the road's own axis) when its direction from the start is within
/// this many degrees of it.
pub const SQUARE_CAPTURE_DEG: f64 = 8.0;
/// Once taken, the square lock holds until the leg's direction is more than this many degrees off
/// the line (hysteresis).
pub const SQUARE_RELEASE_DEG: f64 = 12.0;
/// Guide lines are gathered from roads and nodes within this many metres of the cursor.
pub const GUIDE_RADIUS_M: f64 = 64.0;
// An edge-and-angle intersection is taken within this many metres of the raw cursor: the
// network snap's own reach.
const EDGE_REACH_M: f64 = super::pick::NETWORK_SNAP_REACH_M as f64;
// Lines closer to parallel than this (sine of their angle) have no intersection.
const PARALLEL_SIN: f64 = 1e-9;
// The grid's point lies on the angle lock's ray within this many metres: their directions agree
// to the `f32` precision of the stored start and road geometry.
const GRID_ON_RAY_M: f64 = 1e-3;

/// The snapping toggles and steps (plan 2.3; the bridge's typed setters). Defaults per U2: network
/// snap on, guide lines on, zoning grid off, angle lock off, no length step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapOptions {
    /// Snap to network nodes and edges.
    pub network: bool,
    /// Guide lines: extensions of dead ends, perpendiculars at nodes and the anchor, the stroke's
    /// own first axis.
    pub guides: bool,
    /// The zoning grid (`RoadGridSnap`).
    pub grid: bool,
    /// Angle lock step in degrees (15, 30, 45 or 90), `None` for off.
    pub angle_step_deg: Option<f64>,
    /// Length step in metres, `None` for off.
    pub length_step_m: Option<f64>,
}

impl Default for SnapOptions {
    fn default() -> Self {
        Self {
            network: true,
            guides: true,
            grid: false,
            angle_step_deg: None,
            length_step_m: None,
        }
    }
}

/// What took the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    /// Nothing: the raw pick.
    Free,
    /// A network node.
    Node,
    /// A network edge's centreline.
    Edge,
    /// A guide line.
    Guide,
    /// The corner of two guide lines.
    GuideCorner,
    /// The zoning grid.
    Grid,
    /// The angle lock's ray.
    Angle,
    /// The angle lock's ray where it meets a network edge.
    AngleEdge,
    /// The angle lock's ray where it meets a guide line.
    AngleGuide,
    /// The angle lock's ray where it meets a zoning grid line.
    AngleGrid,
    /// A network edge's centreline at the zoning grid's position along it: the grid's own point
    /// for a first point on the road it aligns to, else the grid's axis through the stroke's
    /// start where it crosses the centreline.
    GridEdge,
    /// The map edge.
    Border,
    /// The stroke's own start or control point.
    SelfPoint,
}

/// One guide line: a point on it and its unit direction (a full line, both senses).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideLine {
    /// A point of the line.
    pub origin: DVec2,
    /// Unit direction.
    pub dir: DVec2,
}

impl GuideLine {
    /// The foot of `p` on the line.
    pub fn project(&self, p: DVec2) -> DVec2 {
        self.origin + self.dir * (p - self.origin).dot(self.dir)
    }

    fn distance(&self, p: DVec2) -> f64 {
        self.dir.perp_dot(p - self.origin).abs()
    }

    // Whether `other` is the same line (parallel, either sense, through this line's origin).
    fn same_line(&self, other: &GuideLine) -> bool {
        self.dir.perp_dot(other.dir).abs() <= PARALLEL_SIN && other.distance(self.origin) <= 1e-9
    }

    // Where this line meets `other`, `None` when they are parallel.
    fn meet(&self, other: &GuideLine) -> Option<DVec2> {
        let denominator = self.dir.perp_dot(other.dir);
        if denominator.abs() <= PARALLEL_SIN {
            return None;
        }
        Some(
            self.origin
                + self.dir * ((other.origin - self.origin).perp_dot(other.dir) / denominator),
        )
    }
}

/// The candidates of one pointer pose, gathered by the session.
#[derive(Clone, Copy, Debug)]
pub struct SnapInput {
    /// The raw pick in XZ.
    pub raw: DVec2,
    /// The point the stroke grows from (the last placed point), if any.
    pub anchor: Option<DVec2>,
    /// The angle lock's reference: the start tangent, else world east.
    pub reference: Option<DVec2>,
    /// Shift held: the angle lock, guide lines, grid and length step are off for this pose.
    pub free: bool,
    /// The network snap (node or edge) in reach, with the edge's local direction.
    pub(crate) network: Option<(NetworkSnap, Option<DVec2>)>,
    /// The zoning grid's snapped point and, when the grid gives a line through the anchor, its
    /// direction.
    pub grid: Option<(DVec2, Option<DVec2>)>,
    /// The edge whose centreline holds the grid's point (`RoadGridSnap::snap_road_cursor_on`).
    pub grid_edge: Option<usize>,
    /// The square guide the session's lock holds ([`square_lock`]): it takes the cursor as the
    /// line `L` after an edge, before any other guide line.
    pub square: Option<GuideLine>,
}

/// The resolved cursor in XZ and what took it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapped {
    /// Resolved position.
    pub xz: DVec2,
    /// The constraint that placed it.
    pub kind: SnapKind,
    /// The network target, for a node or an edge.
    pub(crate) target: Option<NetworkSnapTarget>,
    /// Whether the length step rounded it.
    pub stepped: bool,
    /// Whether the angle lock was on and a node took the cursor off it.
    pub angle_broken: bool,
}

impl Snapped {
    fn at(xz: DVec2, kind: SnapKind) -> Self {
        Self {
            xz,
            kind,
            target: None,
            stepped: false,
            angle_broken: false,
        }
    }
}

// The line constraint `L` of one pose.
#[derive(Clone, Copy)]
enum LineConstraint {
    Edge(NetworkSnap, GuideLine),
    Guide(GuideLine),
}

/// The precedence of plan section 3 (module docs) over one pose's candidates, before map-edge
/// and self snap (the session applies those to a [`SnapKind::Free`] result) and before the
/// length step ([`apply_length_step`]). `O(guides)`, no allocation.
pub fn resolve(input: &SnapInput, options: &SnapOptions, guides: &[GuideLine]) -> Snapped {
    let constrained = !input.free;
    let lock = options.angle_step_deg.filter(|_| constrained);
    // 1. A node wins outright; the lock is reported broken.
    if let Some((snap, _)) = input.network
        && matches!(snap.target, NetworkSnapTarget::Node(_))
    {
        return Snapped {
            target: Some(snap.target),
            angle_broken: lock.is_some() && input.anchor.is_some(),
            ..Snapped::at(xz_of(snap.position), SnapKind::Node)
        };
    }
    // 2. The direction constraint; the zoning grid where it takes the pose (its one rule).
    let direction = lock.and_then(|step| angle_ray(input, step));
    let edge = input.network.and_then(|(snap, dir)| {
        Some((
            snap,
            GuideLine {
                origin: xz_of(snap.position),
                dir: dir?,
            },
        ))
    });
    if let Some(snapped) = grid_rule(input, options, direction, edge) {
        return snapped;
    }
    // The line constraint: the held square guide stands first, with the nearest guide crossing
    // it beside it (their corner is taken near the cursor's foot on the square guide).
    let guided = constrained && options.guides;
    let square = input.square.filter(|_| guided);
    let at = square.map_or(input.raw, |square| square.project(input.raw));
    let nearest_guide = match (guided, square) {
        (false, _) => [None; 2],
        (true, Some(square)) => [Some((0.0, square)), nearest_crossing(guides, at, square)],
        (true, None) => nearest_guides(guides, input.raw),
    };
    let line = edge
        .map(|(snap, guide)| LineConstraint::Edge(snap, guide))
        .or_else(|| nearest_guide[0].map(|(_, guide)| LineConstraint::Guide(guide)));
    match (direction, line) {
        // 3. Both: their intersection within the line's reach, else edge over angle over line.
        (Some(ray), Some(line)) => {
            let (guide, reach, kind) = match line {
                LineConstraint::Edge(_, guide) => (guide, EDGE_REACH_M, SnapKind::AngleEdge),
                LineConstraint::Guide(guide) => (guide, GUIDE_REACH_M, SnapKind::AngleGuide),
            };
            let meet = ray
                .meet(&guide)
                .filter(|&point| (point - ray.origin).dot(ray.dir) > 0.0)
                .filter(|&point| point.distance(input.raw) <= reach);
            match (meet, line) {
                (Some(point), LineConstraint::Edge(snap, _)) => Snapped {
                    target: Some(snap.target),
                    ..Snapped::at(point, kind)
                },
                (Some(point), _) => Snapped::at(point, kind),
                (None, LineConstraint::Edge(snap, _)) => edge_snap(snap),
                (None, _) => Snapped::at(ray_projection(&ray, input.raw), SnapKind::Angle),
            }
        }
        (Some(ray), None) => Snapped::at(ray_projection(&ray, input.raw), SnapKind::Angle),
        // 4. Two guide lines and no direction: their corner.
        (None, Some(LineConstraint::Guide(first))) => nearest_guide[1]
            .and_then(|(_, second)| first.meet(&second))
            .filter(|corner| corner.distance(at) <= GUIDE_REACH_M)
            .map_or_else(
                || Snapped::at(first.project(input.raw), SnapKind::Guide),
                |corner| Snapped::at(corner, SnapKind::GuideCorner),
            ),
        // 5. One line alone: its projection.
        (None, Some(LineConstraint::Edge(snap, _))) => edge_snap(snap),
        (None, None) => match input.network {
            // An edge snap with no usable direction still snaps.
            Some((snap, _)) => edge_snap(snap),
            None => Snapped::at(input.raw, SnapKind::Free),
        },
    }
}

/// Rule 6: rounds the distance from `anchor` along the direction left free, for a free, angle or
/// guide result; a node, edge, intersection, grid, border or self point is kept. A guide line
/// rounds along itself from the anchor's foot on it. Steps of zero length are taken as one step
/// (the cursor never collapses onto the anchor). `O(1)`.
pub fn apply_length_step(
    snapped: &mut Snapped,
    anchor: Option<DVec2>,
    step_m: Option<f64>,
    guide: Option<GuideLine>,
) {
    let (Some(anchor), Some(step)) = (anchor, step_m.filter(|step| *step > 0.0)) else {
        return;
    };
    let round = |distance: f64| (distance / step).round().max(1.0) * step;
    match snapped.kind {
        SnapKind::Free | SnapKind::Angle => {
            let offset = snapped.xz - anchor;
            let distance = offset.length();
            if distance > 0.0 {
                snapped.xz = anchor + offset * (round(distance) / distance);
                snapped.stepped = true;
            }
        }
        SnapKind::Guide => {
            if let Some(guide) = guide {
                let foot = guide.project(anchor);
                let along = (snapped.xz - foot).dot(guide.dir);
                if along != 0.0 {
                    snapped.xz = foot + guide.dir * (round(along.abs()) * along.signum());
                    snapped.stepped = true;
                }
            }
        }
        _ => {}
    }
}

/// The guide line a [`SnapKind::Guide`] result lies on (the nearest in reach), for the length
/// step and the gizmos.
pub fn nearest_guide(guides: &[GuideLine], raw: DVec2) -> Option<GuideLine> {
    nearest_guides(guides, raw)[0].map(|(_, guide)| guide)
}

/// The guide lines holding a cursor at `xz`: the nearest in reach and the nearest in reach that
/// crosses it (both meet at a [`SnapKind::GuideCorner`]). `O(guides)`.
pub fn holding_guides(guides: &[GuideLine], xz: DVec2) -> [Option<GuideLine>; 2] {
    nearest_guides(guides, xz).map(|guide| guide.map(|(_, guide)| guide))
}

// The nearest guide line within reach and the nearest one in reach that crosses it (a corner
// needs two directions); on equal distance the earlier candidate. Two passes, `O(guides)`.
fn nearest_guides(guides: &[GuideLine], raw: DVec2) -> [Option<(f64, GuideLine)>; 2] {
    let first = nearest_within(guides, raw, |_| true);
    let second = first.and_then(|(_, first)| nearest_crossing(guides, raw, first));
    [first, second]
}

// The nearest guide line within reach of `raw` that crosses `line`. `O(guides)`.
fn nearest_crossing(guides: &[GuideLine], raw: DVec2, line: GuideLine) -> Option<(f64, GuideLine)> {
    nearest_within(guides, raw, |guide| {
        line.dir.perp_dot(guide.dir).abs() > PARALLEL_SIN
    })
}

// The nearest accepted guide line within reach of `raw`; on equal distance the earlier one.
fn nearest_within(
    guides: &[GuideLine],
    raw: DVec2,
    accept: impl Fn(&GuideLine) -> bool,
) -> Option<(f64, GuideLine)> {
    let mut best: Option<(f64, GuideLine)> = None;
    for guide in guides.iter().filter(|guide| accept(guide)) {
        let distance = guide.distance(raw);
        if distance <= GUIDE_REACH_M && best.is_none_or(|(old, _)| distance < old) {
            best = Some((distance, *guide));
        }
    }
    best
}

/// The guide lines drawn faint (round UI1): up to [`GUIDE_NEAR_MAX`] within [`GUIDE_NEAR_M`] of
/// the raw cursor, nearest first (on equal distance the earlier candidate), leaving out the lines
/// in `holding` (drawn as the guides that took the cursor) and any line already chosen (two
/// candidates on one line, such as the perpendiculars of a straight road's two arms at a node).
/// `O(guides * GUIDE_NEAR_MAX)`, no allocation.
pub fn near_guides(
    guides: &[GuideLine],
    raw: DVec2,
    holding: [Option<GuideLine>; 2],
) -> [Option<GuideLine>; GUIDE_NEAR_MAX] {
    let mut near = [None; GUIDE_NEAR_MAX];
    for slot in 0..GUIDE_NEAR_MAX {
        let taken = |guide: &GuideLine| {
            holding
                .iter()
                .chain(&near[..slot])
                .flatten()
                .any(|other| other.same_line(guide))
        };
        let mut best: Option<(f64, GuideLine)> = None;
        for guide in guides.iter().filter(|guide| !taken(guide)) {
            let distance = guide.distance(raw);
            if distance <= GUIDE_NEAR_M && best.is_none_or(|(old, _)| distance < old) {
                best = Some((distance, *guide));
            }
        }
        let Some((_, guide)) = best else {
            break;
        };
        near[slot] = Some(guide);
    }
    near
}

/// Which line of the square lock holds a stroke's first leg ([`square_lock`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquareAxis {
    /// Across the road at the start: the square branch.
    Across,
    /// Along the road's own axis (a dead end's extension, a chain's straight continuation).
    Along,
}

/// The square lock of round UI1: a stroke's first leg from a road or node (`reference` the road's
/// direction at `anchor`, the start) takes the line across the road through the start, or the
/// road's own axis, when the leg's direction from the start to `raw` is within
/// [`SQUARE_CAPTURE_DEG`] of it (the nearer line first, across on a tie); the line `held` last
/// pose keeps it until the direction is more than [`SQUARE_RELEASE_DEG`] off. `None` at the start
/// itself. Returns the axis (the session keeps it for the next pose) and its guide line, the same
/// line [`collect_guides`] gathers at the anchor. `O(1)`.
pub fn square_lock(
    anchor: DVec2,
    reference: DVec2,
    raw: DVec2,
    held: Option<SquareAxis>,
) -> Option<(SquareAxis, GuideLine)> {
    let offset = raw - anchor;
    let reference = reference.try_normalize()?;
    if offset.length_squared() < 1e-6 {
        return None;
    }
    let line = |axis| GuideLine {
        origin: anchor,
        dir: match axis {
            SquareAxis::Across => reference.perp(),
            SquareAxis::Along => reference,
        },
    };
    // Degrees between the leg and a full line (either sense), `0..=90`.
    let off = |axis| {
        let dir = line(axis).dir;
        dir.perp_dot(offset)
            .abs()
            .atan2(dir.dot(offset).abs())
            .to_degrees()
    };
    if let Some(axis) = held.filter(|&axis| off(axis) <= SQUARE_RELEASE_DEG) {
        return Some((axis, line(axis)));
    }
    let (across, along) = (off(SquareAxis::Across), off(SquareAxis::Along));
    let axis = if across <= along {
        SquareAxis::Across
    } else {
        SquareAxis::Along
    };
    (across.min(along) <= SQUARE_CAPTURE_DEG).then(|| (axis, line(axis)))
}

// The zoning grid's one rule (rounds GS, GS2), after a node (which wins outright) and before
// guide lines; `None` leaves the pose to the other constraints, as with the grid off. The grid
// takes a pose only where it moved the cursor ("captured": a first point near an aligned road or
// on a road's lattice, a stroke's end near an axis through its start); outside that the cursor
// draws freely. Where it captured:
// - a network edge in reach and no angle lock: the edge's centreline at the grid's position, the
//   grid's own point on the road it aligns to (a first point), else where the start's grid axis
//   crosses the centreline within the edge's reach ([`SnapKind::GridEdge`]); otherwise the edge;
// - an edge and the angle lock: the lock's edge rules;
// - no edge, no lock: the grid's point ([`SnapKind::Grid`]), before any guide line, so a stroke's
//   end takes whole cells whatever guides its start brings (a start on a road or node carries
//   guides along and across that road: the grid's own axes);
// - no edge, the lock: the grid's point where the lock's ray holds it (the ray along the grid's
//   axis through the start), else the ray's meeting with that axis ahead of the anchor
//   ([`SnapKind::AngleGrid`]); otherwise the lock's rules.
// `O(1)`.
fn grid_rule(
    input: &SnapInput,
    options: &SnapOptions,
    direction: Option<GuideLine>,
    edge: Option<(NetworkSnap, GuideLine)>,
) -> Option<Snapped> {
    let (point, axis) = input.grid.filter(|_| !input.free && options.grid)?;
    if point == input.raw {
        return None;
    }
    match (edge, direction) {
        (Some((snap, edge)), None) => {
            let NetworkSnapTarget::Edge(id) = snap.target else {
                return None;
            };
            let xz = if input.grid_edge == Some(id) {
                point
            } else {
                let axis = GuideLine {
                    origin: point,
                    dir: axis?,
                };
                axis.meet(&edge)
                    .filter(|meet| meet.distance(input.raw) <= EDGE_REACH_M)?
            };
            Some(Snapped {
                target: Some(snap.target),
                ..Snapped::at(xz, SnapKind::GridEdge)
            })
        }
        (Some(_), Some(_)) => None,
        (None, None) => Some(Snapped::at(point, SnapKind::Grid)),
        (None, Some(ray)) => {
            let ahead = |at: DVec2| (at - ray.origin).dot(ray.dir) > 0.0;
            let held = (ray.distance(point) <= GRID_ON_RAY_M && ahead(point)).then_some(point);
            held.or_else(|| {
                axis.and_then(|dir| ray.meet(&GuideLine { origin: point, dir }))
                    .filter(|&at| ahead(at))
            })
            .map(|at| Snapped::at(at, SnapKind::AngleGrid))
        }
    }
}

fn edge_snap(snap: NetworkSnap) -> Snapped {
    Snapped {
        target: Some(snap.target),
        ..Snapped::at(xz_of(snap.position), SnapKind::Edge)
    }
}

// The angle lock's ray: from the anchor along the step multiple of the reference (world east
// without one) nearest the cursor's direction. None without an anchor or a direction.
fn angle_ray(input: &SnapInput, step_deg: f64) -> Option<GuideLine> {
    let anchor = input.anchor?;
    let offset = input.raw - anchor;
    if offset.length_squared() < 1e-6 || step_deg <= 0.0 {
        return None;
    }
    let reference = input
        .reference
        .and_then(|reference| reference.try_normalize())
        .unwrap_or(DVec2::X);
    let step = step_deg.to_radians();
    let relative = reference.perp_dot(offset).atan2(reference.dot(offset));
    let turn = (relative / step).round() * step;
    let (sin, cos) = turn.sin_cos();
    Some(GuideLine {
        origin: anchor,
        dir: DVec2::new(
            reference.x * cos - reference.y * sin,
            reference.x * sin + reference.y * cos,
        ),
    })
}

// The cursor projected onto the ray, never behind its origin.
fn ray_projection(ray: &GuideLine, raw: DVec2) -> DVec2 {
    ray.origin + ray.dir * (raw - ray.origin).dot(ray.dir).max(0.0)
}

/// A world point's XZ.
pub fn xz_of(point: Vector3) -> DVec2 {
    DVec2::new(f64::from(point.x), f64::from(point.z))
}

/// Reused candidate buffers: edge ids from the R-tree, guide lines.
#[derive(Debug, Default)]
pub struct SnapScratch {
    edges: Vec<usize>,
    /// The guide lines of the current pose, in candidate order.
    pub guides: Vec<GuideLine>,
}

/// Gathers the guide lines of a pose into `scratch.guides` (cleared first), in a fixed order:
/// at the anchor, the line along the reference and its perpendicular; the stroke's own first
/// axis; then per edge within [`GUIDE_RADIUS_M`] of `cursor`, ascending by id, at each end node
/// within that radius the perpendicular to the edge's end direction and, at a dead end, the
/// edge's extension. `O(log E + K * end points)`, no allocation after warm-up.
pub(crate) fn collect_guides(
    graph: &RegionGraph,
    cursor: DVec2,
    anchor: Option<DVec2>,
    reference: Option<DVec2>,
    first_axis: Option<GuideLine>,
    scratch: &mut SnapScratch,
) {
    scratch.guides.clear();
    if let (Some(anchor), Some(dir)) = (anchor, reference.and_then(DVec2::try_normalize)) {
        scratch.guides.push(GuideLine {
            origin: anchor,
            dir,
        });
        scratch.guides.push(GuideLine {
            origin: anchor,
            dir: dir.perp(),
        });
    }
    if let Some(axis) = first_axis {
        scratch.guides.push(axis);
    }
    let radius = GUIDE_RADIUS_M as f32;
    let (x, z) = (cursor.x as f32, cursor.y as f32);
    scratch.edges.clear();
    graph.visit_edges_near_aabb(
        Vector3::new(x - radius, 0.0, z - radius),
        Vector3::new(x + radius, 0.0, z + radius),
        |edge| scratch.edges.push(edge),
    );
    scratch.edges.sort_unstable();
    for &edge_id in &scratch.edges {
        let edge = graph.edge(edge_id);
        if edge.deleted {
            continue;
        }
        for node in [edge.start_node, edge.end_node] {
            let position = xz_of(graph.node(node).pos);
            if position.distance_squared(cursor) > GUIDE_RADIUS_M * GUIDE_RADIUS_M {
                continue;
            }
            let Some(into) =
                direction_into_edge(edge_points(graph, edge_id), edge.start_node == node)
            else {
                continue;
            };
            scratch.guides.push(GuideLine {
                origin: position,
                dir: into.perp(),
            });
            let live_arms = graph
                .node_adjacency(node)
                .iter()
                .filter(|&&arm| !graph.edge(arm).deleted)
                .count();
            if live_arms == 1 {
                scratch.guides.push(GuideLine {
                    origin: position,
                    dir: into,
                });
            }
        }
    }
}

// An edge's centreline: its physical profile when present.
fn edge_points(graph: &RegionGraph, edge_id: usize) -> &[Vector3] {
    let edge = graph.edge(edge_id);
    if edge.physical_geometry.len() >= 2 {
        &edge.physical_geometry
    } else {
        &edge.geometry
    }
}

// Unit XZ direction leaving an end of `points` into the edge, skipping sub-5 cm runs.
fn direction_into_edge(points: &[Vector3], from_start: bool) -> Option<DVec2> {
    let first = xz_of(*if from_start {
        points.first()?
    } else {
        points.last()?
    });
    let away = |p: &Vector3| {
        let offset = xz_of(*p) - first;
        (offset.length() >= 0.05).then(|| offset.normalize())
    };
    if from_start {
        points.iter().find_map(away)
    } else {
        points.iter().rev().find_map(away)
    }
}

/// The unit XZ direction of `edge_id`'s centreline at the segment nearest `point`, for the edge
/// constraint's line. `O(edge points)`.
pub(crate) fn edge_direction_at(
    graph: &RegionGraph,
    edge_id: usize,
    point: DVec2,
) -> Option<DVec2> {
    let mut best: Option<(f64, DVec2)> = None;
    for pair in edge_points(graph, edge_id).windows(2) {
        let (a, b) = (xz_of(pair[0]), xz_of(pair[1]));
        let segment = b - a;
        let length_sq = segment.length_squared();
        if length_sq <= 1e-12 {
            continue;
        }
        let t = ((point - a).dot(segment) / length_sq).clamp(0.0, 1.0);
        let distance = (a + segment * t).distance_squared(point);
        if best.is_none_or(|(old, _)| distance < old) {
            best = Some((distance, segment / length_sq.sqrt()));
        }
    }
    best.map(|(_, dir)| dir)
}

/// The network snap of a cursor whose ray hit `structure`: the retained target while in its
/// release radius, else the acquisition (`interaction::closest_network_snap_at_hit_into`, nodes
/// 2.5x as attractive, absolute node priority within reach, ground before structure, the nearer
/// level on a tie of both) with the edge candidates in `scratch`. `O(nodes in visited cells + log E + K * segments)`, no
/// allocation after warm-up.
#[allow(clippy::too_many_arguments)]
pub(crate) fn network_snap(
    graph: &RegionGraph,
    world_pos: Vector3,
    structure: StructureHit,
    previous: Option<(NetworkSnapTarget, u64)>,
    surface_generation: u64,
    release_dist_m: f32,
    max_dist: f32,
    scratch: &mut SnapScratch,
) -> Option<NetworkSnap> {
    let retained = previous
        .filter(|&(_, generation)| generation == surface_generation)
        .map(|(target, _)| target);
    if let Some(snap) = retained
        .filter(|&target| structure.admits(graph, target))
        .and_then(|target| {
            retained_network_snap_xz(graph, world_pos, target, max_dist, release_dist_m)
        })
        .filter(|snap| structure.admits(graph, snap.target))
    {
        return Some(snap);
    }
    closest_network_snap_at_hit_into(graph, world_pos, max_dist, structure, &mut scratch.edges)
}
