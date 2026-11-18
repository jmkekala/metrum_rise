// SPDX-License-Identifier: GPL-2.0-only

//! The preview's join marks (`ROAD-118` round JS5b): what the preview draws of a join beside its
//! pieces, as line strips the tool's drawing shows ([`super::ToolDraw::draw_marks`]).
//!
//! - **The junction outline.** A valid stroke whose end joins a road forms, or re-forms, the
//!   node piece at that end (a T, a crossroads, a bend). Its outline is the piece's own outer
//!   chains, in their order and closed across each mouth from one chain's last corner to the next
//!   chain's first (`roadbed::node` emits one chain per sector, from one arm's mouth corner to the
//!   next arm's): the piece the commit installs, since the preview's pieces are the commit's.
//!   The roads stay where they were drawn; the outline shows the junction that forms around
//!   them. A stroke end on open ground (a road end's cap) or on a straight joint with no piece
//!   draws none. A road end the stroke passes grows onto it as a T (round JS5c), outlined alike
//!   (round JS5d).
//! - **The overlapped stretch** of a `roads_overlap` refusal (round JS4a's `Overlap`: the end,
//!   its road and station): the stretch of that road's centreline lying within the two roadbed
//!   half widths of the refused end's last stretch (the contact's own window, its reach over the
//!   sine of the arm limit, whole segments), so where the two footprints overlap,
//!   measured across; drawn as that stretch of the road's footprint, in the refusal's look. A
//!   refusal of the stroke's side (round JS5d: alongside a road, or crossing it under the arm
//!   limit) names the stroke's stretch beside the road instead of an end's.
//!
//! Geometry is decided here, in the preview call; the drawing only uploads it. Every buffer is
//! reused pose to pose.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::road_edit;
use crate::simulation::network::roadbed::{PieceMesh, RoadbedSystem, visual_roadbed_half_width_m};
use glam::DVec2;
use godot::prelude::Vector3;

// The least angle between a stroke's end and the road it runs onto (`ROAD-54`'s arm limit,
// `validation::contact`'s): the refused end's window is its reach over this sine.
const MIN_CONTACT_ANGLE_DEG: f64 = road_edit::MIN_CONTACT_ANGLE_DEG as f64;

// A point within this of a footprint's edge counts as on it (the closed-form crossings' rounding).
const EDGE_ROUNDING_M: f64 = 1.0e-7;

// A miter corner's length at most this many half widths (a sharp bend's offset does not spike).
const MAX_MITER: f64 = 4.0;

/// The preview's join marks: line strips (`points[starts[i]..starts[i + 1]]`, the last to the
/// end), all in one look: the junction outline (valid) or a refusal's overlapped stretch.
#[derive(Debug, Default)]
pub(crate) struct JoinMarks {
    /// All strip points, strips back to back.
    pub(crate) points: Vec<Vector3>,
    /// Each strip's first point.
    pub(crate) starts: Vec<u32>,
    /// The strips mark a refusal (the overlapped stretch) rather than the junction that forms.
    pub(crate) refused: bool,
    // The overlap's station intervals along the road and a stretch's centreline points with
    // their offsets (scratch, reused).
    intervals: Vec<(f64, f64)>,
    centre: Vec<(Vector3, DVec2, f64)>,
}

// By hand for `clone_from`: the preview's outcome and the drawing keep copies refreshed in place,
// reusing their buffers (the derived one would allocate them anew).
impl Clone for JoinMarks {
    fn clone(&self) -> Self {
        Self {
            points: self.points.clone(),
            starts: self.starts.clone(),
            refused: self.refused,
            intervals: Vec::new(),
            centre: Vec::new(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.points.clone_from(&source.points);
        self.starts.clone_from(&source.starts);
        self.refused = source.refused;
    }
}

// The marks drawn, without the scratch.
impl PartialEq for JoinMarks {
    fn eq(&self, other: &Self) -> bool {
        self.points == other.points && self.starts == other.starts && self.refused == other.refused
    }
}

impl JoinMarks {
    /// Empties the marks, keeping the buffers.
    pub(crate) fn clear(&mut self) {
        self.points.clear();
        self.starts.clear();
        self.refused = false;
    }

    /// Whether there is nothing to draw.
    pub(crate) fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// The junction outline of a valid preview: the closed outline of the node piece at each
    /// stroke end `ends` (live ids on `graph`, the graph with the plan applied) that joins a road
    /// there (two roads or more meet), then at each node `passes` where a passed road end grew
    /// onto the stroke (round JS5d; none of them an end), from `roadbed` (the view's roadbed with
    /// the preview's delta installed). An end's piece is drawn once. `O(outline vertices)`.
    pub(crate) fn outline(
        &mut self,
        graph: &RegionGraph,
        roadbed: &RoadbedSystem,
        ends: [u32; 2],
        passes: &[u32],
    ) {
        self.clear();
        let nodes = ends.iter().chain(passes);
        for (index, &node) in nodes.enumerate() {
            if index == 1 && node == ends[0] {
                continue;
            }
            if graph.node_adjacency(node).len() < 2 {
                continue;
            }
            if let Some(piece) = roadbed.node_piece(node) {
                self.push_piece(piece.mesh());
            }
        }
    }

    /// One piece's closed outline, a strip: its outer chains in order, each joined to the next
    /// across a mouth, and back to the first chain's first corner. `O(chain vertices)`.
    pub(crate) fn push_piece(&mut self, mesh: &PieceMesh) {
        let positions = mesh.positions();
        let chains = mesh.outer_chains();
        let Some(first) = chains.first().and_then(|chain| chain.vertices().first()) else {
            return;
        };
        self.starts.push(self.points.len() as u32);
        let point = |vertex: u32| {
            let p = positions[vertex as usize];
            Vector3::new(p.x as f32, p.y as f32, p.z as f32)
        };
        for chain in chains {
            self.points
                .extend(chain.vertices().iter().map(|&vertex| point(vertex)));
        }
        self.points.push(point(*first));
    }

    /// The overlapped stretch of a `roads_overlap` refusal (module header): road `road`'s
    /// centreline stretch within `stroke_half_width_m` plus its own half width of stroke end
    /// `end`'s last stretch, as the road's footprint over that stretch, one closed strip per run.
    /// `O(road points x window points)`, refused poses only.
    pub(crate) fn overlap(
        &mut self,
        graph: &RegionGraph,
        road: usize,
        stroke: &[Vector3],
        end: usize,
        stroke_half_width_m: f32,
    ) {
        let road_half_width_m = visual_roadbed_half_width_m(graph.edge(road));
        let reach_m = f64::from(stroke_half_width_m) + f64::from(road_half_width_m);
        let window = end_window(stroke, end, reach_m);
        self.overlap_beside(graph, road, window, stroke_half_width_m);
    }

    /// The overlapped stretch of a `roads_overlap` refusal of the stroke's side (round JS5d): as
    /// [`Self::overlap`], beside the stroke's stretch `window` (the drawn points the refusal
    /// names) instead of an end's last stretch. `O(road points x window points)`, refused poses
    /// only.
    pub(crate) fn overlap_beside(
        &mut self,
        graph: &RegionGraph,
        road: usize,
        window: &[Vector3],
        stroke_half_width_m: f32,
    ) {
        self.clear();
        self.refused = true;
        let edge = graph.edge(road);
        let road_half_width_m = f64::from(visual_roadbed_half_width_m(edge));
        let reach_m = f64::from(stroke_half_width_m) + road_half_width_m;
        let geometry = &edge.geometry;
        let xz = |point: Vector3| DVec2::new(f64::from(point.x), f64::from(point.z));
        let (mut low, mut high) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for &point in window {
            low = low.min(xz(point));
            high = high.max(xz(point));
        }
        let (low, high) = (low - reach_m, high + reach_m);
        self.intervals.clear();
        let mut station = 0.0;
        for pair in geometry.windows(2) {
            let (a, b) = (xz(pair[0]), xz(pair[1]));
            let length = a.distance(b);
            if a.min(b).cmple(high).all() && a.max(b).cmpge(low).all() {
                for side in window.windows(2) {
                    let (c, d) = (xz(side[0]), xz(side[1]));
                    if let Some((t0, t1)) = capsule_interval(a, b, (c, d), reach_m) {
                        self.intervals
                            .push((station + t0 * length, station + t1 * length));
                    }
                }
            }
            station += length;
        }
        self.intervals.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Option<(f64, f64)> = None;
        for index in 0..self.intervals.len() {
            let (from, to) = self.intervals[index];
            match &mut merged {
                Some((_, until)) if from <= *until => *until = until.max(to),
                _ => {
                    if let Some(run) = merged {
                        self.push_footprint(geometry, run, road_half_width_m);
                    }
                    merged = Some((from, to));
                }
            }
        }
        if let Some(run) = merged {
            self.push_footprint(geometry, run, road_half_width_m);
        }
    }

    // The footprint of `road` between stations `from` and `to` (horizontal metres from its
    // first point), `half_width_m` to either side: left side forward, right side back, closed.
    // The offsets meet at miter corners. `O(road points)`.
    fn push_footprint(&mut self, road: &[Vector3], (from, to): (f64, f64), half_width_m: f64) {
        if to <= from {
            return;
        }
        // The stretch's centreline points, each with its offset direction (unit left normal, or
        // the miter at an interior vertex) and length.
        self.centre.clear();
        let mut station = 0.0;
        for (index, pair) in road.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let length = f64::from(a.x - b.x).hypot(f64::from(a.z - b.z));
            let s0 = station;
            station += length;
            if length <= 0.0 || station < from || s0 > to {
                continue;
            }
            let normal = left_normal(a, b);
            if self.centre.is_empty() {
                let t = ((from - s0) / length).clamp(0.0, 1.0);
                self.centre.push((lerp(a, b, t), normal, half_width_m));
            }
            if station >= to {
                let t = ((to - s0) / length).clamp(0.0, 1.0);
                self.centre.push((lerp(a, b, t), normal, half_width_m));
                break;
            }
            // The interior vertex `b`: the miter of this segment and the next one of length.
            let next = road[index + 2..]
                .iter()
                .find(|&&c| f64::from(c.x - b.x).hypot(f64::from(c.z - b.z)) > 0.0)
                .map_or(normal, |&c| left_normal(b, c));
            let miter = (normal + next).normalize_or(normal);
            let scale = 1.0 / miter.dot(normal).max(1.0 / MAX_MITER);
            self.centre.push((b, miter, half_width_m * scale));
        }
        if self.centre.len() < 2 {
            return;
        }
        let start = self.points.len();
        self.starts.push(start as u32);
        for &(point, direction, distance) in &self.centre {
            self.points.push(offset(point, direction, distance));
        }
        for &(point, direction, distance) in self.centre.iter().rev() {
            self.points.push(offset(point, direction, -distance));
        }
        self.points.push(self.points[start]);
    }
}

// Stroke end `end`'s last stretch as the contact reads it: its points from the end back over
// whole segments until `reach_m` over the arm limit's sine is covered.
fn end_window(stroke: &[Vector3], end: usize, reach_m: f64) -> &[Vector3] {
    let window_m = reach_m / MIN_CONTACT_ANGLE_DEG.to_radians().sin();
    let last = stroke.len().saturating_sub(1);
    let mut run_m = 0.0;
    let mut steps = 0;
    while steps < last && run_m < window_m {
        let (p, q) = if end == 0 {
            (stroke[steps], stroke[steps + 1])
        } else {
            (stroke[last - steps], stroke[last - steps - 1])
        };
        run_m += f64::from(q.x - p.x).hypot(f64::from(q.z - p.z));
        steps += 1;
    }
    if end == 0 {
        &stroke[..=steps.min(last)]
    } else {
        &stroke[last - steps..]
    }
}

// The interval of `t` in `[0, 1]` for which `a + t (b - a)` lies within `r` of segment `(c, d)`,
// horizontal: the segment meeting the capsule around `(c, d)`. The distance to a segment is
// convex along a line, so the set is one interval, bounded by `0`, `1` or a crossing of the
// capsule's boundary (its end circles, its two sides); the crossings are closed form. `O(1)`.
fn capsule_interval(a: DVec2, b: DVec2, (c, d): (DVec2, DVec2), r: f64) -> Option<(f64, f64)> {
    let ab = b - a;
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut take = |t: f64| {
        if (0.0..=1.0).contains(&t) && segment_distance(a + ab * t, c, d) <= r + EDGE_ROUNDING_M {
            low = low.min(t);
            high = high.max(t);
        }
    };
    take(0.0);
    take(1.0);
    let qa = ab.length_squared();
    for centre in [c, d] {
        let f = a - centre;
        let qb = 2.0 * f.dot(ab);
        let qc = f.length_squared() - r * r;
        let discriminant = qb * qb - 4.0 * qa * qc;
        if qa > 0.0 && discriminant >= 0.0 {
            let root = discriminant.sqrt();
            take((-qb - root) / (2.0 * qa));
            take((-qb + root) / (2.0 * qa));
        }
    }
    let cd = d - c;
    let length = cd.length();
    if length > 0.0 {
        let normal = DVec2::new(-cd.y, cd.x) / length;
        let across = normal.dot(ab);
        if across != 0.0 {
            for side in [r, -r] {
                take((side - normal.dot(a - c)) / across);
            }
        }
    }
    (low <= high).then_some((low, high))
}

// The horizontal distance from `p` to segment `(c, d)`.
fn segment_distance(p: DVec2, c: DVec2, d: DVec2) -> f64 {
    let cd = d - c;
    let length_squared = cd.length_squared();
    let t = if length_squared > 0.0 {
        ((p - c).dot(cd) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(c + cd * t)
}

// The unit left normal of `a -> b`, horizontal (the counter-clockwise quarter turn).
fn left_normal(a: Vector3, b: Vector3) -> DVec2 {
    DVec2::new(-f64::from(b.z - a.z), f64::from(b.x - a.x)).normalize_or_zero()
}

fn lerp(a: Vector3, b: Vector3, t: f64) -> Vector3 {
    a + (b - a) * t as f32
}

fn offset(point: Vector3, direction: DVec2, distance: f64) -> Vector3 {
    Vector3::new(
        point.x + (direction.x * distance) as f32,
        point.y,
        point.z + (direction.y * distance) as f32,
    )
}
