// SPDX-License-Identifier: GPL-2.0-only

//! Roads that touch join (`ROAD-118` rounds JS4a, JS5a): at ground level two roads meet only in a
//! junction. A stroke end whose footprint, its roadbed's outer band and the round cap of that
//! radius at the end, touches another car road's footprint joins it before the plan is built, so
//! the preview, the cost and the commit read one stroke. Nothing the player drew moves: the join
//! adds a lead-in from the drawn end to the node the roads share ([`super::lead_in`]):
//!
//! - touching a junction's or a bend's piece (the road's stretch between its node and the arm's
//!   mouth), or a road end's cap, the end joins that node;
//! - touching a road's side, or standing on it (within the topology's 2 m end join, the user's
//!   decision D4), the stroke's own line runs on to the road's centreline and the topology's end
//!   join makes a T there: no sideways move, no pull to a grid or node. An end on a road whose
//!   last stretch reaches or crosses it keeps the topology's own join as before (a crossing's
//!   stub), as does an end standing on the centreline itself (within the topology's 5 cm node
//!   capture, where that join keeps the end's own projection: a stroke laid on a road, an end on
//!   its vertex) whose line runs along it (fix B1);
//! - where that line meets the road at under the junction arm limit (25 degrees, `ROAD-54`), or
//!   not within the contact's reach over the sine of that limit, the roads would run on top of
//!   each other: the stroke is refused as `roads_overlap`.
//!
//! A pure function of the graph and the stroke: it never reads the Roads snap toggle, whose one
//! meaning is the cursor magnet. Ends at a node, above the ground or on a structure's span keep
//! their place (`ROAD-99`: only ground meets ground), as do ends within 2 m of a road no car
//! meets on the ground (a walkway, a ring: the topology's own end join decides them, as before),
//! and ends past a road the stroke crosses just before them (the crossing joins them, round JS3
//! removes the stub).

use super::{PREVIEW_ROADS_OVERLAP_REASON, RoadPreviewValidation};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::road_edit::input::{PreparedRoadInput, is_surface_edge};
use crate::simulation::network::roadbed::visual_roadbed_half_width_m;
use crate::simulation::network::topology::INTERSECTION_NODE_CAPTURE_EPSILON;
use crate::simulation::network::types::{EdgeClass, TransitFlags};
use godot::prelude::Vector3;

/// The widest roadbed half width a road can have (`ROAD-118` round JS4a): four lanes, a parking
/// lane, half the widest (12 m) median and a sidewalk. The contact query pads an end's own half
/// width by it, as the edge R-tree holds centreline bounds. A test holds the widest road to it.
pub(crate) const MAX_ROADBED_HALF_WIDTH_M: f32 = 4.0 * config::LANE_WIDTH
    + crate::simulation::network::graph::PARKING_LANE_WIDTH_M
    + 6.0
    + config::SIDEWALK_WIDTH;

// The rounding of an end's line meeting a centreline it stands on: an end on the centreline may
// read its meeting a hair behind it.
const RUN_ROUNDING_M: f32 = 1.0e-3;

/// The least overlap of two footprints that touch (`ROAD-118` round JS4a): outlines that only
/// meet, as two roads drawn exactly a cap apart on the grid do (the first dump's dead end stands
/// 9.9998 m from its junction's arm, two 5 m half widths), are rounding, not an overlap.
const MIN_OVERLAP_M: f32 = 0.01;

/// The least angle between a stroke's end tangent and the road it runs onto (`ROAD-54`'s
/// junction arm limit, degrees); under it the two roads overlap instead of meeting. A lead-in
/// keeps its new arm as far from a node's other arms.
pub(super) const MIN_CONTACT_ANGLE_DEG: f32 = 25.0;

/// What a stroke end touches (`ROAD-118` round JS4a), in the committed graph's ids.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EndContact {
    /// A junction's or a bend's piece, or a road end's cap: the end joins the node.
    Node(u32),
    /// A road's side: the stroke's line runs on to `point` on the road's centreline.
    Road(Vector3),
}

/// A stroke end's contact and its reach (`ROAD-118` round JS5a): the two roadbeds' half widths,
/// the most of the stroke a lead-in may take (the user's decision D3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Touched {
    pub(super) contact: EndContact,
    pub(super) reach_m: f32,
}

// The reach of a stroke end of roadbed half width `half_width_m` onto road `edge`: the two
// footprints touch when the end lies nearer its centreline (`ROAD-118` round JS4a).
pub(super) fn reach_m(half_width_m: f32, edge: &Edge) -> f32 {
    half_width_m + visual_roadbed_half_width_m(edge) - MIN_OVERLAP_M
}

/// A stroke end that would overlap a road instead of meeting it (`roads_overlap`): the end (0
/// the start, 1 the end), the live road and the station along it nearest the end, and the
/// angle the end's tangent makes with it there, degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Overlap {
    end: usize,
    edge: usize,
    station_m: f32,
    angle_deg: f32,
}

impl Overlap {
    // `validation` refused as `roads_overlap`: the end's road and station as its place, the
    // tangent's angle with the road as measured against the least angle.
    pub(super) fn refuse(self, mut validation: RoadPreviewValidation) -> RoadPreviewValidation {
        validation.geometry_measured = self.angle_deg;
        validation.geometry_limit = MIN_CONTACT_ANGLE_DEG;
        validation.endpoint_edges[self.end] = Some((self.edge, self.station_m));
        validation.with_invalid_reason(PREVIEW_ROADS_OVERLAP_REASON)
    }
}

// A road whose footprint the end's cap touches: its id, its nearest centreline point's
// distance and station, the segment holding it and its horizontal length.
#[derive(Clone, Copy)]
struct Touch {
    edge: usize,
    distance_m: f32,
    station_m: f32,
    segment: usize,
    length_m: f32,
    reach_m: f32,
}

/// What each end of `prepared` touches, start then end, and its reach (`ROAD-118` round JS4a;
/// see the module docs). `half_width_m` is the stroke's roadbed half width; `existing` the live nodes the ends
/// already stand on. An `Err` names the first overlapping end. Per end: one R-tree query of the
/// end's reach padded by [`MAX_ROADBED_HALF_WIDTH_M`], then per candidate road a nearest-point
/// pass, and for the touched one a node's degree, a crossing test over the stroke's last
/// `reach / sin 25` metres and a ray test: O(log E + candidates x points), local, no
/// allocation.
pub(super) fn end_contacts(
    prepared: &PreparedRoadInput,
    half_width_m: f32,
    graph: &RegionGraph,
    existing: [Option<u32>; 2],
) -> Result<[Option<Touched>; 2], Overlap> {
    let mut contacts: [Option<Touched>; 2] = [None; 2];
    let points = &prepared.points;
    if points.len() < 2 || prepared.tunnel || prepared.class != EdgeClass::Standard {
        return Ok(contacts);
    }
    for end in 0..2 {
        let free = existing[end].is_none()
            && prepared.joined_nodes[end].is_none()
            && prepared.joined_spans[end].is_none()
            && !prepared.elevated_ends[end];
        if !free {
            continue;
        }
        let Some(tangent) = end_tangent(points, end) else {
            continue;
        };
        let tip = if end == 0 {
            points[0]
        } else {
            points[points.len() - 1]
        };
        let Some(touch) = nearest_touch(graph, points, end, tip, half_width_m) else {
            continue;
        };
        let contact = match touch_node(graph, &touch, tip) {
            Some(node) => EndContact::Node(node),
            None => match carry(graph, &touch, tip, tangent, end) {
                Ok(contact) => contact,
                // An end on the centreline itself whose line runs along the road (under the arm
                // limit, or a stroke laid on the road) keeps the topology's own join, which keeps
                // the end's projection there: the stroke continues a road or the junction's angle
                // refuses it. Any other end would be pulled sideways onto the road (fix B1).
                Err(_) if touch.distance_m <= INTERSECTION_NODE_CAPTURE_EPSILON => continue,
                Err(overlap) => return Err(overlap),
            },
        };
        let other = existing[1 - end]
            .map(EndContact::Node)
            .or(contacts[1 - end].map(|other| other.contact));
        // Both ends onto one node would close the stroke on itself; the second keeps its place.
        if matches!((contact, other), (EndContact::Node(a), Some(EndContact::Node(b))) if a == b) {
            continue;
        }
        contacts[end] = Some(Touched {
            contact,
            reach_m: touch.reach_m,
        });
    }
    Ok(contacts)
}

// The horizontal unit direction in which end `end` (0 the start, 1 the end) leaves the stroke,
// from the nearest point back that stands apart from it. `None` for a stroke of one place.
fn end_tangent(points: &[Vector3], end: usize) -> Option<(f32, f32)> {
    let tip = if end == 0 {
        points[0]
    } else {
        points[points.len() - 1]
    };
    let direction = |point: &Vector3| {
        let (dx, dz) = (tip.x - point.x, tip.z - point.z);
        let length = dx.hypot(dz);
        (length > 1.0e-3).then(|| (dx / length, dz / length))
    };
    if end == 0 {
        points.iter().find_map(direction)
    } else {
        points.iter().rev().find_map(direction)
    }
}

// The car road on the ground whose footprint the cap of radius `half_width_m` at `tip` touches
// nearest, by centreline distance and then id, among the roads the stroke does not cross near
// this end; an end on such a road (within `config::INTERSECTION_TOLERANCE` of its centreline)
// touches it too (round JS5a, D4). `None` for an end within that tolerance of a road no car meets
// on the ground or of one the stroke's last stretch reaches or crosses: the topology's own end
// join decides, as before. O(log E + candidates x points).
fn nearest_touch(
    graph: &RegionGraph,
    points: &[Vector3],
    end: usize,
    tip: Vector3,
    half_width_m: f32,
) -> Option<Touch> {
    let pad = half_width_m + MAX_ROADBED_HALF_WIDTH_M;
    let reach = Vector3::new(pad, 0.0, pad);
    let mut best: Option<Touch> = None;
    let mut on_a_road = false;
    graph.visit_edges_near_aabb(tip - reach, tip + reach, |edge| {
        let record = graph.edge(edge);
        if on_a_road || !is_surface_edge(record) || record.class != EdgeClass::Standard {
            return;
        }
        let Some((distance_m, station_m, segment, length_m)) = nearest_point(&record.geometry, tip)
        else {
            return;
        };
        let on = distance_m <= config::INTERSECTION_TOLERANCE;
        if !meets_on_the_ground(record) {
            on_a_road |= on;
            return;
        }
        let reach_m = reach_m(half_width_m, record);
        if distance_m >= reach_m
            || best.is_some_and(|best| (best.distance_m, best.edge) < (distance_m, edge))
        {
            return;
        }
        if crosses_near_end(points, end, &record.geometry, reach_m) {
            // An end on a road its last stretch reaches or crosses stands on the crossing or
            // just past it: the topology joins it there, as before.
            on_a_road |= on;
            return;
        }
        best = Some(Touch {
            edge,
            distance_m,
            station_m,
            segment,
            length_m,
            reach_m,
        });
    });
    best.filter(|_| !on_a_road)
}

// A live car road at ground level, off any roundabout: the roads a stroke end meets by contact.
fn meets_on_the_ground(edge: &Edge) -> bool {
    is_surface_edge(edge)
        && edge.class == EdgeClass::Standard
        && edge.roundabout.is_none()
        && edge.allowed_types & TransitFlags::CAR != 0
}

// The node the touch is with: a road end of the touched road whose cap the end's cap overlaps
// (the nearer end of the road first), or the junction or bend whose piece holds the touched
// point (its station within the arm's mouth). Only a node whose every road meets on the ground.
fn touch_node(graph: &RegionGraph, touch: &Touch, tip: Vector3) -> Option<u32> {
    let record = graph.edge(touch.edge);
    let ends = [
        (
            graph.get_valid_node(record.start_node),
            touch.station_m,
            record.start_clip,
        ),
        (
            graph.get_valid_node(record.end_node),
            touch.length_m - touch.station_m,
            record.end_clip,
        ),
    ];
    let nearer = usize::from(ends[1].1 < ends[0].1);
    for (node, from_node_m, clip_m) in [ends[nearer], ends[1 - nearer]] {
        let mut arms = 0;
        let mut on_the_ground = true;
        for &arm in graph.node_adjacency(node) {
            let arm = graph.edge(arm);
            if !arm.deleted {
                arms += 1;
                on_the_ground &= meets_on_the_ground(arm);
            }
        }
        if !on_the_ground {
            continue;
        }
        let pos = graph.node(node).pos;
        let cap = arms == 1 && (pos.x - tip.x).hypot(pos.z - tip.z) < touch.reach_m;
        let piece = clip_m > 0.0 && from_node_m <= clip_m;
        if cap || piece {
            return Some(node);
        }
    }
    None
}

// Where the end's line meets the touched road's centreline: the nearest crossing ahead within the
// contact's reach over the sine of the least angle (an end on the centreline meets it where it
// stands, within `f32` rounding); refused as an overlap where the line meets the road under that
// angle or not within that run. O(road points).
fn carry(
    graph: &RegionGraph,
    touch: &Touch,
    tip: Vector3,
    (tx, tz): (f32, f32),
    end: usize,
) -> Result<EndContact, Overlap> {
    let geometry = &graph.edge(touch.edge).geometry;
    let max_run_m = touch.reach_m / MIN_CONTACT_ANGLE_DEG.to_radians().sin();
    let mut hit: Option<(f32, Vector3, f32)> = None;
    for pair in geometry.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dz) = (b.x - a.x, b.z - a.z);
        let length = dx.hypot(dz);
        let denominator = tx * dz - tz * dx;
        if length <= 0.0 || denominator.abs() <= 1.0e-6 * length {
            continue;
        }
        let (ax, az) = (a.x - tip.x, a.z - tip.z);
        let run_m = (ax * dz - az * dx) / denominator;
        let along = (ax * tz - az * tx) / denominator;
        if !(0.0..=1.0).contains(&along) || !(-RUN_ROUNDING_M..=max_run_m).contains(&run_m) {
            continue;
        }
        let run_m = run_m.max(0.0);
        if hit.is_none_or(|(best_m, ..)| run_m < best_m) {
            let point = Vector3::new(
                tip.x + tx * run_m,
                a.y + (b.y - a.y) * along,
                tip.z + tz * run_m,
            );
            hit = Some((run_m, point, angle_deg((tx, tz), (dx, dz))));
        }
    }
    match hit {
        Some((_, point, angle)) if angle >= MIN_CONTACT_ANGLE_DEG => Ok(EndContact::Road(point)),
        _ => {
            let (a, b) = (geometry[touch.segment], geometry[touch.segment + 1]);
            Err(Overlap {
                end,
                edge: touch.edge,
                station_m: touch.station_m,
                angle_deg: angle_deg((tx, tz), (b.x - a.x, b.z - a.z)),
            })
        }
    }
}

// The angle between a unit direction and a line along `(dx, dz)`, degrees in [0, 90].
fn angle_deg((tx, tz): (f32, f32), (dx, dz): (f32, f32)) -> f32 {
    let length = dx.hypot(dz);
    if length <= 0.0 {
        return 0.0;
    }
    ((tx * dz - tz * dx).abs() / length)
        .min(1.0)
        .asin()
        .to_degrees()
}

// The horizontal distance from `point` to a polyline, the station of its nearest point, that
// point's segment, and the polyline's horizontal length. O(points).
fn nearest_point(points: &[Vector3], point: Vector3) -> Option<(f32, f32, usize, f32)> {
    let mut station = 0.0;
    let mut best: Option<(f32, f32, usize)> = None;
    for (segment, pair) in points.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dz) = (b.x - a.x, b.z - a.z);
        let length = dx.hypot(dz);
        let t = if length > 0.0 {
            (((point.x - a.x) * dx + (point.z - a.z) * dz) / (length * length)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let distance = (a.x + t * dx - point.x).hypot(a.z + t * dz - point.z);
        if best.is_none_or(|(d, ..)| distance < d) {
            best = Some((distance, station + t * length, segment));
        }
        station += length;
    }
    best.map(|(distance, at, segment)| (distance, at, segment, station))
}

// Whether the stroke's last `reach_m / sin 25` metres before end `end` cross the polyline
// `road`: the crossing joins them and the end past it is the crossing's stub. O(those stroke
// points x road points).
fn crosses_near_end(points: &[Vector3], end: usize, road: &[Vector3], reach_m: f32) -> bool {
    let window_m = reach_m / MIN_CONTACT_ANGLE_DEG.to_radians().sin();
    let last = points.len() - 1;
    let mut run_m = 0.0;
    for step in 0..last {
        let (p, q) = if end == 0 {
            (points[step], points[step + 1])
        } else {
            (points[last - step], points[last - step - 1])
        };
        if road
            .windows(2)
            .any(|pair| segments_cross((p, q), (pair[0], pair[1])))
        {
            return true;
        }
        run_m += (q.x - p.x).hypot(q.z - p.z);
        if run_m >= window_m {
            break;
        }
    }
    false
}

// Whether two horizontal segments cross or touch.
fn segments_cross((p, q): (Vector3, Vector3), (a, b): (Vector3, Vector3)) -> bool {
    let side =
        |o: Vector3, s: Vector3, t: Vector3| (s.x - o.x) * (t.z - o.z) - (s.z - o.z) * (t.x - o.x);
    let (d1, d2) = (side(a, b, p), side(a, b, q));
    let (d3, d4) = (side(p, q, a), side(p, q, b));
    (d1 * d2 <= 0.0) && (d3 * d4 <= 0.0) && !(d1 == 0.0 && d2 == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::{MedianStyle, RoadMedian, RoadParking};
    use crate::simulation::network::road_types::RoadAttributes;
    use crate::simulation::network::roadbed::fixtures::{point, road_edge};
    use crate::simulation::network::types::NodeType;

    // A 2-lane street along z = 0 from x = -100 to 100, its roadbed 5 m to either side.
    fn street() -> RegionGraph {
        let mut graph = RegionGraph::new();
        let west = graph.add_node(point(-100.0, 0.0, false), NodeType::Junction);
        let east = graph.add_node(point(100.0, 0.0, false), NodeType::Junction);
        graph.add_edge(road_edge(
            west,
            east,
            vec![point(-100.0, 0.0, false), point(100.0, 0.0, false)],
            7.0,
        ));
        graph.rebuild_intersection_clips();
        graph
    }

    // `end_contacts` without the reaches.
    fn contacts_of(
        prepared: &PreparedRoadInput,
        half_width_m: f32,
        graph: &RegionGraph,
        existing: [Option<u32>; 2],
    ) -> Result<[Option<EndContact>; 2], Overlap> {
        end_contacts(prepared, half_width_m, graph, existing)
            .map(|touched| touched.map(|touched| touched.map(|touched| touched.contact)))
    }

    fn stroke(points: &[(f32, f32)]) -> PreparedRoadInput {
        PreparedRoadInput {
            points: points.iter().map(|&(x, z)| point(x, z, false)).collect(),
            class: EdgeClass::Standard,
            endpoint_snap_enabled: false,
            tunnel: false,
            forced_grade: None,
            elevated_ends: [false; 2],
            joined_spans: [None; 2],
            joined_nodes: [None; 2],
            median: None,
            parking: RoadParking::NONE,
            attributes: RoadAttributes::DEFAULT,
        }
    }

    // The widest road the cross-section allows stays within the contact query's pad.
    #[test]
    fn the_widest_road_is_within_the_query_pad() {
        let median = RoadMedian {
            style: MedianStyle::KerbedGrass,
            width_m: RoadMedian::width_range_m(MedianStyle::KerbedGrass).1,
        };
        let parking = RoadParking {
            left: true,
            right: true,
        };
        let points = vec![point(0.0, 0.0, false), point(10.0, 0.0, false)];
        let edge = crate::simulation::network::build_surface_edge(
            0,
            1,
            points,
            4,
            4,
            EdgeClass::Standard,
            Some(median),
            parking,
            RoadAttributes::DEFAULT,
        );
        let widest = visual_roadbed_half_width_m(&edge);
        assert!(
            widest <= MAX_ROADBED_HALF_WIDTH_M,
            "{widest} > {MAX_ROADBED_HALF_WIDTH_M}"
        );
    }

    // A stroke from the south ending 4 m short of the street's centreline (inside its 10 m
    // reach) at 60 degrees runs on along its tangent to the centreline; from 11 m it touches
    // nothing; at 10 degrees it would overlap; above the ground or as a bridge it keeps its end.
    #[test]
    fn an_end_touching_a_side_runs_on_to_the_centreline() {
        let graph = street();
        let (dx, dz) = (60f32.to_radians().cos(), 60f32.to_radians().sin());
        let end = (10.0, -4.0);
        let from = (end.0 - 50.0 * dx, end.1 - 50.0 * dz);
        let touching = stroke(&[from, end]);
        let contacts = contacts_of(&touching, 5.0, &graph, [None; 2]).unwrap();
        let Some(EndContact::Road(at)) = contacts[1] else {
            panic!("{contacts:?}");
        };
        assert!(at.z.abs() < 1.0e-4, "{at:?}");
        assert!(
            (at.x - (10.0 + 4.0 / 60f32.to_radians().tan())).abs() < 1.0e-3,
            "{at:?}"
        );
        assert_eq!(contacts[0], None);

        let apart = stroke(&[(10.0, -61.0), (10.0, -11.0)]);
        assert_eq!(contacts_of(&apart, 5.0, &graph, [None; 2]), Ok([None; 2]));

        let flat = stroke(&[
            (-40.0, -4.0 - 50.0 * 10f32.to_radians().tan()),
            (10.0, -4.0),
        ]);
        let overlap = contacts_of(&flat, 5.0, &graph, [None; 2]).unwrap_err();
        assert_eq!((overlap.end, overlap.edge), (1, 0));
        assert!((overlap.angle_deg - 10.0).abs() < 0.01, "{overlap:?}");
        assert!((overlap.station_m - 110.0).abs() < 0.01, "{overlap:?}");

        let mut elevated = touching.clone();
        elevated.elevated_ends = [false, true];
        assert_eq!(
            contacts_of(&elevated, 5.0, &graph, [None; 2]),
            Ok([None; 2])
        );
        let mut bridge = touching;
        bridge.class = EdgeClass::Bridge;
        assert_eq!(contacts_of(&bridge, 5.0, &graph, [None; 2]), Ok([None; 2]));
    }

    // An end cap over the street's end cap joins the road end; the start's tangent points out of
    // the stroke the other way.
    #[test]
    fn a_cap_over_a_road_end_joins_it() {
        let graph = street();
        let along = stroke(&[(108.0, 0.0), (190.0, 0.0)]);
        let contacts = contacts_of(&along, 5.0, &graph, [None; 2]).unwrap();
        assert_eq!(contacts, [Some(EndContact::Node(1)), None]);
        let short = stroke(&[(110.5, 0.0), (190.0, 0.0)]);
        assert_eq!(contacts_of(&short, 5.0, &graph, [None; 2]), Ok([None; 2]));
    }
}
