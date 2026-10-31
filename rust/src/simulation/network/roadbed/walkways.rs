// SPDX-License-Identifier: GPL-2.0-only

//! Pedestrian walkways of one node from its junction layout (`ROAD-68-C`): where its sidewalks
//! end, its zebra crossings and the sidewalk corner routes between neighbouring arms.
//!
//! The lane rebuild reads them through [`NodeWalkways::collect`], which lays the node out with the
//! graph-to-layout function node pieces use ([`NodeArms::collect`], [`NodeArms::layout`]). Both
//! read the same graph, so lanes and node pieces agree bitwise whichever of the lane rebuild and
//! the roadbed compile runs first. Nothing here derives geometry of its own:
//!
//! - A sidewalk runs on the centre of its raised walkway, midway between the carriageway edge
//!   (the kerb) and the sidewalk's outer edge, where sidewalk lanes have always run. It ends on
//!   the crossing line of a crossable arm and at the mouth of any other arm.
//! - A crossing is the layout's [`Crosswalk`], with its kerb points on the node plane
//!   ([`crossing_kerbs`], the function node pieces publish them with).
//! - A corner route runs from one arm's sidewalk end back along its walkway to the sector, along
//!   the centre of the sector's walkway (midpoints of its carriageway-edge and outermost boundary
//!   samples) and out along the next arm's walkway to its sidewalk end, so it never leaves the
//!   sidewalk. A terminal's route wraps its round cap.
//!
//! - A footpath ends at its mouth, on its centreline. One attached to a road node (D5) joins the
//!   walk of the sector it lies in where its centreline meets it: the corner route, extended along
//!   both arms' walkways out to their mouths, past a crossable arm's sidewalk end on the crossing
//!   line (`ROAD-68` open question 17). It meets that walk by construction: its centreline runs
//!   from beyond the sector's outer boundary, between the two mouths, into the core. Where only
//!   footpaths meet, they meet at the node.
//!
//! A pass-through node has no layout, but its crossing (D8: one by default) is span-owned
//! ([`pass_through_crossing`]): the crossing arm's sidewalks end on the crossing line, taken with
//! the kerb points from the span's section there, and the node keeps the corner walks of a node
//! without a layout (no corner routes here).
//!
//! `O(degree * arm knots + sector samples)` per node, plus a far-node plan per pass-through
//! crossing; allocates only the returned buffers and the span model's.

use super::arms::{NodeArms, pass_through_crossings};
use super::layout::{
    ArmInput, ArmKey, ArmSide, Corner, EdgeEnd, JunctionLayout, NodeKind, SectionLine,
    SectorBoundary, SectorLayout, Side, for_each_run_station, segment_crossing,
};
use super::node::crossing_kerbs;
use super::span::{SpanCrossing, crossing_kerbs_right_first, pass_through_crossing};
use super::{BandKind, RoadBand, RoadSection, RoadVec2, RoadVec3};
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector3;
use std::ops::Range;

/// Where one sidewalk ends at a node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SidewalkEnd {
    /// Horizontal metres along the arm's centreline from the node.
    pub(crate) station_m: f64,
    /// End point on the walkway centre, at the arm centreline's height there.
    pub(crate) point: RoadVec3,
}

/// Where the vehicle lanes of one arm end at a node (`TRAFFIC-12`): the arm's cross-section at
/// the edge's clip there, which is the mouth station of a bend or junction layout (where the node
/// piece ends and the span begins) and zero at a terminal or a node without a layout. Lanes end on
/// its line and connectors leave and join it along its direction, so neither reads a lane
/// polyline's end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MouthFrame {
    /// Horizontal metres along the arm's centreline from the node.
    pub(crate) station_m: f64,
    line: SectionLine,
    // Centreline height at the station.
    height_m: f64,
}

impl MouthFrame {
    /// The frame of arm `key` of `node` at `station_m`: the section line the node's layout puts
    /// there ([`ArmInput::section_line`] of the arm's own knots) and the centreline height there.
    /// `None` for a degenerate edge. `O(edge points)`.
    pub(crate) fn at(graph: &RegionGraph, node: u32, key: ArmKey, station_m: f64) -> Option<Self> {
        let arms = NodeArms::single(graph, node, key)?;
        Some(Self {
            station_m,
            line: arms.input(0).section_line(station_m),
            height_m: arms.centreline_at(0, station_m).1,
        })
    }

    /// The point at signed lateral offset `lateral_m` (positive to the left of the outward
    /// direction) on the line, at the centreline height there.
    pub(crate) fn point(&self, lateral_m: f64) -> RoadVec3 {
        let xz = self.line.point(lateral_m);
        RoadVec3::new(xz.x, self.height_m, xz.y)
    }

    /// Unit XZ direction away from the node, square to the line.
    pub(crate) fn direction(&self) -> RoadVec2 {
        self.line.direction()
    }
}

/// A sidewalk corner route between two neighbouring arms, counter-clockwise from `from`'s left
/// sidewalk end to `to`'s right sidewalk end.
pub(crate) struct CornerRoute<'a> {
    /// Arm whose left sidewalk the route leaves.
    pub(crate) from: ArmKey,
    /// Arm whose right sidewalk the route reaches; `from` itself at a terminal.
    pub(crate) to: ArmKey,
    /// Route points, starting and ending exactly at the two sidewalk ends.
    pub(crate) points: &'a [RoadVec3],
}

// Sidewalk ends and crossing of one arm, in the layout's counter-clockwise order.
#[derive(Clone, Copy, Debug)]
struct WalkArm {
    key: ArmKey,
    // Right side, then left; `None` where the side has no sidewalk.
    ends: [Option<SidewalkEnd>; 2],
    kerbs: Option<[Vector3; 2]>,
}

const RIGHT: usize = 0;
const LEFT: usize = 1;

/// The walkways of one node, for the lane rebuild.
pub(crate) struct NodeWalkways {
    // Every layout arm; at a pass-through node only its crossing arms.
    arms: Vec<WalkArm>,
    // False at a pass-through node, which has no layout and no corner routes.
    layout: bool,
    // Sector walks, each holding its corner route.
    corners: Vec<SectorWalk>,
    points: Vec<RoadVec3>,
    // Footpath ends at their mouths, and an attached one's join with its sector's walk.
    paths: Vec<(ArmKey, SidewalkEnd, Option<Join>)>,
}

// The walk of one sector with sidewalks on both sides: from its from-arm's mouth along the
// walkway centre to its to-arm's mouth (`walk`, into `points`), and the corner route within it
// between the two sidewalk ends (`route`).
struct SectorWalk {
    sector: usize,
    from: usize,
    to: usize,
    walk: Range<usize>,
    route: Range<usize>,
}

// Where an attached footpath's centreline, walked in from its mouth, first meets its sector's
// walk: the walk (index into `corners`), its segment there and the meeting point.
#[derive(Clone, Copy, Debug)]
struct Join {
    corner: usize,
    segment: usize,
    point: RoadVec3,
}

/// An attached footpath's join with its sector's walk (D5).
pub(crate) struct FootpathJoin<'a> {
    /// The sector's corner route.
    pub(crate) corner: CornerRoute<'a>,
    /// The sector's walk, from its from-arm's mouth to its to-arm's; the corner route is
    /// `walk[route]`.
    pub(crate) walk: &'a [RoadVec3],
    /// Where the corner route lies in `walk`.
    pub(crate) route: Range<usize>,
    /// Segment of `walk` the footpath meets.
    pub(crate) segment: usize,
    /// The meeting point on that segment, where the footpath's centreline crosses it.
    pub(crate) point: RoadVec3,
}

impl NodeWalkways {
    /// Walkways of `node`: its layout's, or a pass-through node's span-owned crossings. `None`
    /// for a pass-through node without a crossing and for the other nodes without a layout: the
    /// cases of [`NodeArms::collect`], a layout that fails (a placement rejection) or a bend or
    /// junction without a solved plane, which leaves it without a node piece too.
    #[cfg(test)]
    pub(crate) fn collect(graph: &RegionGraph, node: u32) -> Option<Self> {
        Self::collect_with_kerbs(graph, node).0
    }

    /// The walkways of `node` (as `collect`, which tests use alone) and its layout's kerb
    /// corners, from one layout: the fitted carriageway-edge corners of its concave sectors, which
    /// vehicle connectors keep clear of (`TRAFFIC-12`). The corners exist wherever the layout
    /// does, a bend or junction without a solved plane included; empty without concave sectors.
    pub(crate) fn collect_with_kerbs(
        graph: &RegionGraph,
        node: u32,
    ) -> (Option<Self>, Option<Vec<Corner>>) {
        let Some(mut arms) = NodeArms::collect(graph, node) else {
            return (Self::pass_through(graph, node), None);
        };
        let Ok(layout) = arms.layout() else {
            return (None, None);
        };
        let kerbs = layout
            .sectors
            .iter()
            .filter_map(|sector| sector.corner)
            .collect();
        (Self::from_layout(graph, node, &arms, &layout), Some(kerbs))
    }

    fn from_layout(
        graph: &RegionGraph,
        node: u32,
        arms: &NodeArms,
        layout: &JunctionLayout,
    ) -> Option<Self> {
        let plane = match layout.kind {
            NodeKind::Terminal => None,
            NodeKind::Bend | NodeKind::Junction => {
                Some(graph.solved_junction_endpoint_profile_plane(node)?)
            }
        };
        // Arm `index`'s point at `station_m` and signed offset, at its centreline height there.
        let end_at = |index: usize, station_m: f64, offset_m: f64| {
            let xz = arms.input(index).offset_point(station_m, offset_m);
            let height_m = arms.centreline_at(index, station_m).1;
            SidewalkEnd {
                station_m,
                point: RoadVec3::new(xz.x, height_m, xz.y),
            }
        };
        let walk_arms: Vec<WalkArm> = layout
            .arms
            .iter()
            .map(|arm| {
                let input = arms.input(arm.input_index);
                let station_m = arm
                    .crosswalk
                    .map_or(arm.mouth_station_m, |crosswalk| crosswalk.station_m);
                let end = |offset_m: Option<f64>| {
                    offset_m.map(|offset_m| end_at(arm.input_index, station_m, offset_m))
                };
                WalkArm {
                    key: arm.key,
                    ends: [
                        end(walk_offset(input.right).map(|offset| -offset)),
                        end(walk_offset(input.left)),
                    ],
                    kerbs: arm
                        .crosswalk
                        .zip(plane.as_ref())
                        .map(|(crosswalk, plane)| crossing_kerbs(plane, &crosswalk)),
                }
            })
            .collect();
        let path_end = |index: usize, station_m: f64| end_at(index, station_m, 0.0);
        let footpaths = layout.arms.iter().filter(|arm| {
            let input = arms.input(arm.input_index);
            input.left.bands[0].kind == BandKind::Footpath
        });
        let paths = footpaths
            .map(|arm| {
                (
                    arm.key,
                    path_end(arm.input_index, arm.mouth_station_m),
                    None,
                )
            })
            .collect();
        let mut walkways = Self {
            corners: Vec::with_capacity(layout.sectors.len()),
            points: Vec::new(),
            arms: walk_arms,
            layout: true,
            paths,
        };
        for (index, sector) in layout.sectors.iter().enumerate() {
            let (Some(start), Some(end)) = (
                walkways.arms[sector.from_arm].ends[LEFT],
                walkways.arms[sector.to_arm].ends[RIGHT],
            ) else {
                continue;
            };
            let (walk, route) =
                push_sector_walk(&mut walkways.points, layout, arms, sector, [start, end]);
            walkways.corners.push(SectorWalk {
                sector: index,
                from: sector.from_arm,
                to: sector.to_arm,
                walk,
                route,
            });
        }
        for attached in &layout.attachments {
            let end = path_end(attached.input_index, attached.mouth_station_m);
            let corner = walkways
                .corners
                .iter()
                .position(|corner| corner.sector == attached.sector);
            let join = corner.and_then(|corner| {
                let walk = &walkways.points[walkways.corners[corner].walk.clone()];
                let input = arms.input(attached.input_index);
                let met = centreline_meets(&input, attached.mouth_station_m, walk);
                debug_assert!(
                    met.is_some(),
                    "an attached centreline meets its sector's walk"
                );
                let (segment, point) = met?;
                Some(Join {
                    corner,
                    segment,
                    point,
                })
            });
            walkways.paths.push((attached.key, end, join));
        }
        Some(walkways)
    }

    // A pass-through node's crossing arms, each with its sidewalk ends and kerbs on the span's
    // section at the crossing line.
    fn pass_through(graph: &RegionGraph, node: u32) -> Option<Self> {
        let arms: Vec<WalkArm> = pass_through_crossings(graph, node)?
            .into_iter()
            .filter(|&(_, crosses)| crosses)
            .filter_map(|(key, _)| Some(span_walk_arm(key, &pass_through_crossing(graph, key)?)))
            .collect();
        (!arms.is_empty()).then_some(Self {
            arms,
            layout: false,
            corners: Vec::new(),
            points: Vec::new(),
            paths: Vec::new(),
        })
    }

    /// Whether the node has a junction layout, whose corner routes join its sidewalks; a
    /// pass-through node's sidewalks join as at a node without one.
    pub(crate) fn has_layout(&self) -> bool {
        self.layout
    }

    /// The end of the sidewalk on the side of arm `key` whose lateral offset is positive
    /// (`positive_lateral`) or negative in its edge's forward frame.
    pub(crate) fn sidewalk_end(&self, key: ArmKey, positive_lateral: bool) -> Option<SidewalkEnd> {
        // At an edge's start the layout's left is the positive lateral side; at its end, the
        // negative one.
        let side = if positive_lateral == (key.end == EdgeEnd::Start) {
            LEFT
        } else {
            RIGHT
        };
        self.arm(key)?.ends[side]
    }

    /// Crossings in the layout's counter-clockwise arm order: each crossable arm's key and its
    /// kerb points, right side first.
    pub(crate) fn crossings(&self) -> impl Iterator<Item = (ArmKey, [Vector3; 2])> + '_ {
        self.arms
            .iter()
            .filter_map(|arm| Some((arm.key, arm.kerbs?)))
    }

    /// Corner routes in the layout's sector order.
    pub(crate) fn corners(&self) -> impl Iterator<Item = CornerRoute<'_>> {
        self.corners.iter().map(|corner| self.corner(corner))
    }

    fn corner(&self, corner: &SectorWalk) -> CornerRoute<'_> {
        CornerRoute {
            from: self.arms[corner.from].key,
            to: self.arms[corner.to].key,
            points: &self.points[corner.route.clone()],
        }
    }

    /// Where footpath arm `key`'s lanes end: its mouth, on its centreline.
    pub(crate) fn footpath_end(&self, key: ArmKey) -> Option<SidewalkEnd> {
        let (_, end, _) = self.paths.iter().find(|(arm, ..)| *arm == key)?;
        Some(*end)
    }

    /// Where attached footpath `key` joins its sector's walk (D5): `None` where that sector has
    /// no sidewalk round it (`ROAD-68` open question 17) or the footpath is not attached.
    pub(crate) fn footpath_join(&self, key: ArmKey) -> Option<FootpathJoin<'_>> {
        let (.., join) = self.paths.iter().find(|(arm, ..)| *arm == key)?;
        let join = (*join)?;
        let corner = &self.corners[join.corner];
        let start = corner.walk.start;
        Some(FootpathJoin {
            corner: self.corner(corner),
            walk: &self.points[corner.walk.clone()],
            route: corner.route.start - start..corner.route.end - start,
            segment: join.segment,
            point: join.point,
        })
    }

    fn arm(&self, key: ArmKey) -> Option<&WalkArm> {
        self.arms.iter().find(|arm| arm.key == key)
    }
}

// A crossing arm of a pass-through node from its span crossing: sidewalk ends on the walkway
// centre of the crossing section, at the centreline height as layout ends are, and the section
// row's kerb vertices.
fn span_walk_arm(key: ArmKey, crossing: &SpanCrossing) -> WalkArm {
    let section = &crossing.section;
    let point = |lateral_m: f64| {
        let xz = section.center_xz + section.lateral_xz * lateral_m;
        RoadVec3::new(xz.x, f64::from(section.center_height_m), xz.y)
    };
    let [negative, positive] = section_walk_laterals(section).map(|lateral_m| {
        lateral_m.map(|lateral_m| SidewalkEnd {
            station_m: crossing.station_m,
            point: point(lateral_m),
        })
    });
    // Walking away from the node, the right side is the negative lateral one at an edge's start
    // and the positive one at its end.
    let ends = match key.end {
        EdgeEnd::Start => [negative, positive],
        EdgeEnd::End => [positive, negative],
    };
    WalkArm {
        key,
        ends,
        kerbs: Some(crossing_kerbs_right_first(crossing.kerbs, key.end)),
    }
}

// Signed laterals of a section's walkway centres, negative side first: midway between the
// carriageway edge and the section's outer edge, on a side with a sidewalk band.
fn section_walk_laterals(section: &RoadSection) -> [Option<f64>; 2] {
    let bands = &section.bands;
    let carriageway = |band: &RoadBand| band.kind == BandKind::Carriageway;
    let (Some(first), Some(last)) = (
        bands.iter().position(carriageway),
        bands.iter().rposition(carriageway),
    ) else {
        return [None, None];
    };
    let has_sidewalk = |side: &[RoadBand]| side.iter().any(|band| band.kind == BandKind::Sidewalk);
    let centre = |kerb: f32, outer: f32| 0.5 * (f64::from(kerb) + f64::from(outer));
    [
        has_sidewalk(&bands[..first])
            .then(|| centre(bands[first].lateral_start_m, bands[0].lateral_start_m)),
        has_sidewalk(&bands[last + 1..]).then(|| {
            centre(
                bands[last].lateral_end_m,
                bands[bands.len() - 1].lateral_end_m,
            )
        }),
    ]
}

// Unsigned offset of a side's walkway centre: midway between the carriageway edge and the
// sidewalk's outer edge. `None` without a sidewalk band.
fn walk_offset(side: ArmSide<'_>) -> Option<f64> {
    let outer = side.bands.last()?;
    side.bands[side.core_bands..]
        .iter()
        .any(|band| band.kind == BandKind::Sidewalk)
        .then(|| 0.5 * (side.core_offset() + outer.outer_m))
}

// Appends the walk of `sector` along its walkway centre (midway between its carriageway edge and
// outermost boundary, tapered with them, D4), from its from-arm's mouth back to the sector, across
// it and out to its to-arm's mouth. Returns its range and that of its corner route inside it, from
// the from-arm's left sidewalk end to the to-arm's right one: route heights run linearly by
// horizontal length between the two ends, the rest of the walk takes its arm's centreline height.
fn push_sector_walk(
    points: &mut Vec<RoadVec3>,
    layout: &JunctionLayout,
    arms: &NodeArms,
    sector: &SectorLayout,
    [start, end]: [SidewalkEnd; 2],
) -> (Range<usize>, Range<usize>) {
    let [a_arm, b_arm] = [sector.from_arm, sector.to_arm].map(|arm| &layout.arms[arm]);
    let (a_index, a_mouth_m) = (a_arm.input_index, a_arm.mouth_station_m);
    let (b_index, b_mouth_m) = (b_arm.input_index, b_arm.mouth_station_m);
    let (a, b) = (arms.input(a_index), arms.input(b_index));
    let boundaries = &layout.boundaries[sector.boundaries.clone()];
    let (edge, outer) = (&boundaries[0], &boundaries[boundaries.len() - 1]);
    let taper_m = edge.taper.or(outer.taper).map(|taper| taper.station_m);
    let walkway = |arm: &ArmInput<'_>, side: Side, station_m: f64| {
        let offset_m =
            0.5 * (edge.run_offset_m(side, station_m) + outer.run_offset_m(side, station_m));
        arm.offset_point(station_m, offset_m)
    };
    let beyond = |index: usize, arm: &ArmInput<'_>, side: Side, station_m: f64| {
        let xz = walkway(arm, side, station_m);
        RoadVec3::new(xz.x, arms.centreline_at(index, station_m).1, xz.y)
    };
    let first = points.len();
    // In along the from-arm's walkway from its mouth to the sidewalk end, if that lies inside.
    if a_mouth_m > start.station_m {
        points.push(beyond(a_index, &a, Side::Left, a_mouth_m));
        let inward = points.len();
        for_each_run_station(&a, start.station_m, a_mouth_m, taper_m, |station_m, _| {
            points.push(beyond(a_index, &a, Side::Left, station_m));
        });
        points[inward..].reverse();
    }
    let route_start = points.len();
    let push = |points: &mut Vec<RoadVec3>, xz: RoadVec2| {
        if points[route_start..]
            .last()
            .is_none_or(|last| RoadVec2::new(last.x, last.z) != xz)
        {
            points.push(RoadVec3::new(xz.x, 0.0, xz.y));
        }
    };
    push(points, RoadVec2::new(start.point.x, start.point.z));
    // Back along the from-arm's walkway to where the sector's boundaries leave it.
    let a_corner_m = edge.from_station_m.max(outer.from_station_m);
    let back = points.len();
    for_each_run_station(&a, a_corner_m, start.station_m, taper_m, |station_m, _| {
        let xz = walkway(&a, Side::Left, station_m);
        points.push(RoadVec3::new(xz.x, 0.0, xz.y));
    });
    points[back..].reverse();
    let samples = |boundary: &SectorBoundary| &layout.samples[boundary.samples.clone()];
    for (inner, outer) in samples(edge).iter().zip(samples(outer)) {
        push(points, (*inner + *outer) * 0.5);
    }
    // Out along the to-arm's walkway to its sidewalk end.
    let b_corner_m = edge.to_station_m.max(outer.to_station_m);
    for_each_run_station(&b, b_corner_m, end.station_m, taper_m, |station_m, _| {
        push(points, walkway(&b, Side::Right, station_m));
    });
    push(points, RoadVec2::new(end.point.x, end.point.z));
    // The two ends are the arms' sidewalk ends exactly, heights included.
    let route = &mut points[route_start..];
    let flat = |point: RoadVec3| RoadVec2::new(point.x, point.z);
    let step_m = |pair: &[RoadVec3]| flat(pair[0]).distance(flat(pair[1]));
    let total_m: f64 = route.windows(2).map(step_m).sum();
    let mut walked_m = 0.0;
    for index in 1..route.len() {
        walked_m += step_m(&route[index - 1..=index]);
        let t = if total_m > 0.0 {
            walked_m / total_m
        } else {
            0.0
        };
        route[index].y = start.point.y * (1.0 - t) + end.point.y * t;
    }
    route[0] = start.point;
    if let [_, .., last] = route {
        *last = end.point;
    }
    let route_end = points.len();
    // Out along the to-arm's walkway from the sidewalk end to its mouth, if that lies beyond.
    if b_mouth_m > end.station_m {
        for_each_run_station(&b, end.station_m, b_mouth_m, taper_m, |station_m, _| {
            points.push(beyond(b_index, &b, Side::Right, station_m));
        });
        points.push(beyond(b_index, &b, Side::Right, b_mouth_m));
    }
    (first..points.len(), route_start..route_end)
}

// The first point at which `arm`'s centreline, walked in from station `from_m`, meets the polyline
// `route`, by the layout's segment crossing: the route segment there and the point on it, with
// the route's height interpolated. `O(centreline knots * route points)`, both short near a node.
fn centreline_meets(
    arm: &ArmInput<'_>,
    from_m: f64,
    route: &[RoadVec3],
) -> Option<(usize, RoadVec3)> {
    let xz = |point: &RoadVec3| RoadVec2::new(point.x, point.z);
    let mut inward = std::iter::once(from_m)
        .chain(
            arm.knots[arm.knots_between(0.0, from_m)]
                .iter()
                .rev()
                .map(|knot| knot.station_m),
        )
        .chain(std::iter::once(0.0))
        .map(|station_m| arm.offset_point(station_m, 0.0));
    let mut previous = inward.next()?;
    for next in inward {
        // The crossing nearest the mouth along this centreline segment.
        let crossings = route.windows(2).enumerate().filter_map(|(segment, pair)| {
            let (t, u) = segment_crossing(previous, next, xz(&pair[0]), xz(&pair[1]))?;
            Some((t, segment, pair[0].lerp(pair[1], u)))
        });
        if let Some((_, segment, point)) = crossings.min_by(|a, b| a.0.total_cmp(&b.0)) {
            return Some((segment, point));
        }
        previous = next;
    }
    None
}
