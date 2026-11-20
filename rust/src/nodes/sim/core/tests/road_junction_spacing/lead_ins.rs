// SPDX-License-Identifier: GPL-3.0-or-later

//! `ROAD-118` round JS5a: joins form the junction; nothing the player drew moves. Every join
//! below previews and commits a 2-lane stroke on a small flat city and holds the round's bar:
//! the preview builds and is the commit, every drawn point outside the lead-ins stays bit for
//! bit, every point of the roads that stood stays bit for bit (a split keeps them), no node is
//! left without a road. The user's reference cases: a cap over a road end in line with an offset,
//! at 30, 46 and 90 degrees; an end in a T's open side; an end beside a crossing's piece off its
//! axis; an end on a road's side; an end on a road 6 m from a T (round JS3's shear before).
//! Refused: a road end the arc cannot reach within the cap; an end 1 m off a road at 15 degrees
//! (fix B1: `roads_overlap`, no sideways capture).

use super::contacts::{commit_parked, lay, preview_parked, v};
use super::stubs_and_joins::{edgeless_nodes, node_near};
use super::*;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::road_edit::{
    EndContact, LeadIn, RoadProfile, prepare_road_input_for_tool,
};

// Every live road's centreline, by id.
fn roads(core: &SimCore) -> Vec<Vec<Vector3>> {
    let graph = &core.region_graph;
    (0..graph.edge_count())
        .filter(|&edge| !graph.edge(edge).deleted)
        .map(|edge| graph.edge(edge).geometry.clone())
        .collect()
}

// A flat city of 2-lane roads drawn in order, without parking.
fn city(roads: &[[(f32, f32); 2]]) -> SimCore {
    super::contacts::city(roads, 1)
}

// The stroke `points` as drawn on the core's published state (Roads snap off): the prepared
// points the plan starts from.
pub(super) fn drawn_of(core: &mut SimCore, points: &[Vector3]) -> Vec<Vector3> {
    let view = context(core);
    prepare_road_input_for_tool(
        points,
        &view.view.terrain,
        &view.replica,
        &view.view.roadbed,
        false,
        RoadProfile::FollowGround,
    )
    .points
}

// Where each kept drawn point lies in `built`: the drawn points with more than their end's trim
// of the stroke between them and that end (an end with no trim keeps its own point) must stand
// in `built` in order, bit for bit, as one run.
pub(super) fn assert_drawn_kept(drawn: &[Vector3], built: &[Vector3], lead_ins: [Option<LeadIn>; 2], label: &str) {
    let mut stations = Vec::with_capacity(drawn.len());
    let mut station = 0.0f32;
    for (index, point) in drawn.iter().enumerate() {
        if index > 0 {
            station += (point.x - drawn[index - 1].x).hypot(point.z - drawn[index - 1].z);
        }
        stations.push(station);
    }
    let total = station;
    let kept = |trim: Option<LeadIn>, from_end: f32| {
        trim.is_none_or(|lead_in| from_end > lead_in.trim_m + 1.0e-3 || lead_in.trim_m == 0.0)
    };
    let kept: Vec<Vector3> = drawn
        .iter()
        .zip(&stations)
        .filter(|&(_, &at)| kept(lead_ins[0], at) && kept(lead_ins[1], total - at))
        .map(|(point, _)| *point)
        .collect();
    assert!(!kept.is_empty(), "{label}: some of the stroke is kept");
    let first = built
        .iter()
        .position(|point| *point == kept[0])
        .unwrap_or_else(|| panic!("{label}: the first kept point moved"));
    assert_eq!(
        built[first..first + kept.len()],
        kept[..],
        "{label}: the drawn points stay bit for bit"
    );
    for (end, lead_in) in lead_ins.into_iter().enumerate() {
        if let Some(lead_in) = lead_in {
            let tip = if end == 0 { built[0] } else { built[built.len() - 1] };
            assert_eq!(tip, lead_in.target, "{label}: the lead-in ends on its target");
        }
    }
}

// Every point of `before`'s roads stands in a live road after, bit for bit.
fn assert_roads_kept(before: &[Vec<Vector3>], core: &SimCore, label: &str) {
    let after = roads(core);
    for (road, points) in before.iter().enumerate() {
        for point in points {
            assert!(
                after.iter().any(|road| road.contains(point)),
                "{label}: road {road}'s point {point:?} moved"
            );
        }
    }
}

// Previews the 2-lane stroke `ends` with `parking`, commits that preview and holds the round's
// bar (module docs). Returns the preview's validation and the built stroke.
fn join(
    core: &mut SimCore,
    ends: [Vector2; 2],
    parking: RoadParking,
    label: &str,
) -> (RoadPreviewValidation, Vec<Vector3>) {
    let points = lay(core, &ends);
    let before = roads(core);
    let drawn = drawn_of(core, &points);
    let chunks = context(core).view.road_mesh_chunks.clone();
    let (preview, _) = preview_parked(core, &points, (1, 1), false, parking);
    let validation = preview.validation.clone();
    assert!(validation.is_valid, "{label}: {}", validation.invalid_reason);
    let built = preview.plan.prepared().points.clone();
    assert_drawn_kept(&drawn, &built, validation.lead_ins, label);
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let outcome = core.commit_road(
        RoadCommitRequest {
            parking,
            points: points.clone(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: false,
            profile: RoadProfile::FollowGround,
            median: None,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            preview: Some(Arc::clone(&preview)),
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{label}: {}", outcome.rejection);
    assert_preview_composes_committed(core, &chunks, pieces, label);
    assert!(core.transit_network.roadbed.failed_nodes().is_empty(), "{label}");
    assert_eq!(edgeless_nodes(core), 0, "{label}");
    assert_roads_kept(&before, core, label);
    (validation, built)
}

// A street ending at the origin from the west. A cap over its end in line 2 m to the side (ref-1:
// a slight bend): the end joins the road end, one road on, through an arc tangent to the stroke;
// the drawn points before the trim stay, and the street's. Round JS5c: caps whose line meets the
// street's ahead of both ends (30, 46 and 90 degrees here before) make a corner instead
// (`corners::two_road_ends_whose_lines_meet_make_a_corner`).
#[test]
fn caps_over_a_road_end_join_it_through_a_lead_in() {
    let street = [[(-100.0, 0.0), (0.0, 0.0)]];
    // (label, the drawn end, the stroke's direction at it, degrees from +x)
    let cases: [(&str, (f32, f32), f32); 1] = [("in line, 2 m beside", (6.0, 2.0), 180.0)];
    for (label, end, heading) in cases {
        let mut core = city(&street);
        let road_end = node_near(&core, v(0.0, 0.0));
        let (dx, dz) = (heading.to_radians().cos(), heading.to_radians().sin());
        let ends = [v(end.0 - 80.0 * dx, end.1 - 80.0 * dz), v(end.0, end.1)];
        let (validation, _) = join(&mut core, ends, RoadParking::NONE, label);
        assert_eq!(
            validation.contacts,
            [None, Some(EndContact::Node(road_end))],
            "{label}"
        );
        assert_eq!(validation.joined_nodes, [None, Some(road_end)], "{label}");
        let lead_in = validation.lead_ins[1].expect("a lead-in");
        assert!(lead_in.trim_m <= 10.0, "{label}: within the reach");
        assert_eq!(super::degree(&core, road_end), 2, "{label}: one road on");
    }
}

// A T of 2-lane roads (side street north): a stroke from the south ends 8 m short of the street,
// 2 m east of the T, in the T's open side (ref-5): it joins the T, a crossroads.
#[test]
fn an_end_in_a_ts_open_side_joins_it() {
    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)], [(0.0, 0.0), (0.0, 100.0)]]);
    let t = node_near(&core, v(0.0, 0.0));
    let (validation, _) = join(&mut core, [v(2.0, -70.0), v(2.0, -8.0)], RoadParking::NONE, "T");
    assert_eq!(validation.joined_nodes, [None, Some(t)]);
    assert_eq!(super::degree(&core, t), 4, "a crossroads");
}

// A crossroads of 2-lane roads: a stroke heading north-west whose line passes 3 m beside the
// crossing ends 8 m before it, beside the crossing's piece (ref-2): it joins the crossing.
#[test]
fn an_end_beside_a_crossings_piece_joins_it() {
    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)], [(0.0, -100.0), (0.0, 100.0)]]);
    let crossing = node_near(&core, v(0.0, 0.0));
    let (t, n) = (v(-1.0, 1.0).normalized(), v(-1.0, -1.0).normalized());
    let end = -t * 8.0 - n * 3.0;
    let (validation, _) = join(&mut core, [end - t * 70.0, end], RoadParking::NONE, "crossing");
    assert_eq!(validation.joined_nodes, [None, Some(crossing)]);
    assert_eq!(super::degree(&core, crossing), 5);
}

// A stroke from the south at 60 degrees ending 4 m short of a street's centreline (its side), and
// one ending on it 1 m off (D4: no sideways capture): each runs on along its own line to the
// centreline, a T there; every drawn point stays, the drawn end included.
#[test]
fn an_end_on_or_beside_a_side_runs_on_along_its_line() {
    let (dx, dz) = (60f32.to_radians().cos(), 60f32.to_radians().sin());
    for (label, off) in [("beside", 4.0f32), ("on the road", 1.0)] {
        let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]]);
        let end = v(10.0, -off);
        let (validation, built) = join(
            &mut core,
            [end - v(dx, dz) * 60.0, end],
            RoadParking::NONE,
            label,
        );
        let Some(EndContact::Road(point)) = validation.contacts[1] else {
            panic!("{label}: {:?}", validation.contacts);
        };
        let lead_in = validation.lead_ins[1].expect("a lead-in");
        assert_eq!(lead_in.trim_m, 0.0, "{label}: straight on, nothing trimmed");
        let run = off / 60f32.to_radians().sin();
        assert!((point.x - (10.0 + off / 60f32.to_radians().tan())).abs() < 1.0e-3, "{label}");
        assert!(point.z.abs() < 1.0e-4, "{label}");
        let n = built.len();
        assert!(
            (built[n - 1].x - built[n - 2].x).hypot(built[n - 1].z - built[n - 2].z) <= run + 1.0e-3,
            "{label}"
        );
        assert_eq!(super::degree(&core, node_near(&core, v(point.x, 0.0))), 3, "{label}: a T");
    }
}

// A stroke from the south ending on the street 6 m east of a T (round JS3 sheared the whole stroke
// onto the T): the end joins the T through a lead-in, the drawn stroke kept, a crossroads.
#[test]
fn an_end_on_a_road_beside_a_t_joins_it_without_a_shear() {
    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)], [(0.0, 0.0), (0.0, 100.0)]]);
    let t = node_near(&core, v(0.0, 0.0));
    let (validation, _) = join(&mut core, [v(6.0, -60.0), v(6.0, 0.0)], RoadParking::NONE, "6 m");
    assert_eq!(validation.joined_nodes, [None, Some(t)]);
    let lead_in = validation.lead_ins[1].expect("a lead-in");
    assert!(lead_in.trim_m > 0.0 && lead_in.trim_m <= 10.0, "{lead_in:?}");
    assert_eq!(super::degree(&core, t), 4, "a crossroads");
}

// A stroke from the north passing a road end 6 m to its side and ending 6 m past it: the arc back
// into the road end would need more of the stroke than the reach (12.3 m against 10 m), so the
// stroke is refused as too tight a curve, and the commit refuses it too.
#[test]
fn a_lead_in_past_the_reach_is_refused() {
    let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)]]);
    let points = lay(&core, &[v(6.0, 60.0), v(6.0, -6.0)]);
    let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
    let refused = &preview.validation;
    assert_eq!(refused.invalid_reason, "road_tight_curve", "{refused:?}");
    let outcome = commit_parked(&mut core, &points, RoadParking::NONE);
    assert!(!outcome.committed);
}

// A 2-lane stroke from the south-west whose end meets a street at 15 degrees (fix B1), straight
// (two points) or curved (a 40 m radius in 1 m segments, 15 degrees at the end), ending at
// x = 10 `off` metres short of the centreline. 1 m off, the old 2 m end capture would pull the
// drawn end sideways onto the street: refused as `roads_overlap` with the street as its place,
// the commit refuses too and the graph is unchanged. Ending on the centreline, the topology's own
// join keeps the end's projection there and the T's arm angle refuses it, as before the fix.
fn fifteen_degrees(off: f32, curved: bool) -> Vec<Vector2> {
    let end = v(10.0, -off);
    let heading = |at: f32| v(at.cos(), at.sin());
    let start = 15f32.to_radians();
    if !curved {
        return vec![end - heading(start) * 60.0, end];
    }
    // Walked back from the end, the heading opening by 1/40 rad per metre over 30 m.
    let mut points = vec![end];
    for step in 0..30 {
        let back = *points.last().unwrap() - heading(start + (step as f32 + 0.5) / 40.0);
        points.push(back);
    }
    points.reverse();
    points
}

#[test]
fn an_end_a_metre_off_a_road_at_15_degrees_is_refused_as_an_overlap() {
    for curved in [false, true] {
        let label = if curved { "curved" } else { "straight" };
        let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]]);
        let street = core.region_graph.edges().iter().position(|edge| !edge.deleted);
        let before = super::graph_hash(&core);
        let points = lay(&core, &fifteen_degrees(1.0, curved));
        let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
        let refused = &preview.validation;
        assert_eq!(refused.invalid_reason, "roads_overlap", "{label}: {refused:?}");
        let (edge, station_m) = refused.endpoint_edges[1].expect("the place");
        assert_eq!(Some(edge), street, "{label}");
        assert!((station_m - 110.0).abs() < 0.01, "{label}: {station_m}");
        assert!((refused.geometry_measured - 15.0).abs() < 1.0, "{label}: {refused:?}");
        assert!(!commit_parked(&mut core, &points, RoadParking::NONE).committed, "{label}");
        assert_eq!(super::graph_hash(&core), before, "{label}");

        let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]]);
        let points = lay(&core, &fifteen_degrees(0.0, curved));
        let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
        let on = &preview.validation;
        assert_eq!(on.invalid_reason, "junction_arm_angle", "{label}: {on:?}");
        assert_eq!(on.contacts, [None; 2], "{label}");
    }
}
