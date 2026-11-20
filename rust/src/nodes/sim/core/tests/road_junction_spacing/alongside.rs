// SPDX-License-Identifier: GPL-3.0-or-later

//! `ROAD-118` round JS5d: roads alongside are refused, naming the road. On a flat 2-lane street, a
//! 2-lane stroke whose side runs on top of the street with no end involved (parallel within the
//! footprint, or crossing it 5 degrees under the arm limit, `MIN_CONTACT_ANGLE_DEG`: 15 degrees
//! under today's 20) is refused as `roads_overlap` naming the street, its preview marking the
//! overlapped stretch, and the commit refuses it too; at the limit it crosses. The angles are
//! read from the limit, so a change of the limit moves them with it. An end up to 2 m past an oblique crossing (DECISION-JS5a): the stroke ends at the
//! crossing on its own line, every drawn point before it bit for bit, no sideways foot. Each
//! built case previews and commits with its preview, leaves no node without a road (also after a
//! save and a load), and undo and a second commit restore the graph and the roadbed bit for bit.

use super::contacts::{city, lay, preview_parked, v};
use super::lead_ins::drawn_of;
use super::stubs_and_joins::{edgeless_nodes, node_near};
use super::*;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::road_edit::{MIN_CONTACT_ANGLE_DEG, RoadProfile};
use crate::simulation::network::road_types::RoadAttributes;

// A 2-lane street along z = 0 from x = -50 to 50.
const STREET: [[(f32, f32); 2]; 1] = [[(-50.0, 0.0), (50.0, 0.0)]];

fn commit_2_lane(
    core: &mut SimCore,
    points: &[Vector3],
    preview: Option<Arc<RoadPreview>>,
) -> RoadCommitOutcome {
    core.commit_road(
        RoadCommitRequest {
            parking: RoadParking::NONE,
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: false,
            profile: RoadProfile::FollowGround,
            median: None,
            attributes: RoadAttributes::DEFAULT,
            preview,
        },
        std::time::Instant::now(),
        Default::default(),
    )
}

// The 2-lane stroke `ends` previewed on the street: refused as `roads_overlap` naming the street,
// the station where the footprints first overlap along the stroke and the drawn stretch beside it,
// the angle measured `angle_deg`; the marks are the street's footprint over the stretch, from
// `from_x` to `to_x` along it, and the named station lies in it; the commit refuses the stroke
// and the graph stays as it was.
fn assert_refused_alongside(
    ends: [Vector2; 2],
    (from_x, to_x): (f32, f32),
    angle_deg: f32,
    label: &str,
) {
    let mut core = city(&STREET, 1);
    let street = core
        .region_graph
        .edges()
        .iter()
        .position(|edge| !edge.deleted)
        .unwrap();
    let before = graph_hash(&core);
    let points = lay(&core, &ends);
    let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
    let refused = &preview.validation;
    assert_eq!(
        refused.invalid_reason, "roads_overlap",
        "{label}: {refused:?}"
    );
    assert_eq!(refused.overlap_end, None, "{label}: no end involved");
    let side = refused.overlap_side.expect("the side names its place");
    assert_eq!(side.road, street, "{label}: the street is named");
    let drawn = &preview.plan.prepared().points;
    assert!(
        side.points[0] < side.points[1] && side.points[1] < drawn.len(),
        "{label}: {side:?}"
    );
    assert!(
        (refused.geometry_measured - angle_deg).abs() < 0.01,
        "{label}: {refused:?}"
    );
    let text = RoadRejection::from_validation(refused.invalid_reason, refused, &points).detail;
    let at = format!("edge {street} at {:.1} m", side.station_m);
    assert!(text.contains(&at), "{label}: {text}");
    let marks = &preview.marks;
    assert!(marks.refused, "{label}: {marks:?}");
    assert_eq!(marks.starts, [0], "{label}: one stretch");
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    for point in &marks.points {
        assert!(
            (point.z.abs() - 5.0).abs() < 1.0e-3,
            "{label}: on the street's edges: {point:?}"
        );
        low = low.min(point.x);
        high = high.max(point.x);
    }
    assert!(
        (low - from_x).abs() < 0.01 && (high - to_x).abs() < 0.01,
        "{label}: {low} {high}"
    );
    let named = side.station_m - 50.0;
    assert!(low <= named && named <= high, "{label}: {side:?}");
    assert!(
        !commit_2_lane(&mut core, &points, None).committed,
        "{label}"
    );
    assert_eq!(graph_hash(&core), before, "{label}");
}

// A straight stroke 6 m beside the street's centreline, running past both its ends: the two 5 m
// footprints overlap along the whole street, its ends touch nothing. Refused at 0 degrees.
#[test]
fn a_stroke_alongside_a_road_is_refused_naming_it() {
    let ends = [v(-100.0, -6.0), v(100.0, -6.0)];
    assert_refused_alongside(ends, (-50.0, 50.0), 0.0, "alongside");
}

// A straight stroke crossing the street at its middle 5 degrees under the arm limit (15 degrees
// under 20), both ends 70 m from the crossing (18 m off the street at 15 degrees): refused at
// that angle. The footprints overlap where the street's centreline lies within 10 m of the
// stroke's, up to 10 / sin 15 = 38.6 m either side of the crossing; the street ends 50 m west and
// east of it.
#[test]
fn a_crossing_under_the_arm_limit_is_refused_naming_the_road() {
    let angle = MIN_CONTACT_ANGLE_DEG - 5.0;
    let heading = v(angle.to_radians().cos(), angle.to_radians().sin());
    let to_x = 10.0 / angle.to_radians().sin();
    let ends = [-heading * 70.0, heading * 70.0];
    assert_refused_alongside(ends, (-to_x, to_x), angle, "under the limit");
}

// The stroke `ends` previewed, committed with its preview and held to the round's bar (module
// docs). Returns the core after, the preview and the drawn points.
fn build(ends: [Vector2; 2], label: &str) -> (SimCore, Arc<RoadPreview>, Vec<Vector3>) {
    build_on(&STREET, ends, label)
}

// The same on a flat city of the 2-lane `roads` (`ROAD-126`'s cases at the arm limit).
pub(super) fn build_on(
    roads: &[[(f32, f32); 2]],
    ends: [Vector2; 2],
    label: &str,
) -> (SimCore, Arc<RoadPreview>, Vec<Vector3>) {
    let mut core = city(roads, 1);
    let points = lay(&core, &ends);
    let drawn = drawn_of(&mut core, &points);
    let state_before = (graph_hash(&core), roadbed_digest(&core));
    let chunks = context(&mut core).view.road_mesh_chunks.clone();
    let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
    assert!(
        preview.validation.is_valid,
        "{label}: {}",
        preview.validation.invalid_reason
    );
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let outcome = commit_2_lane(&mut core, &points, Some(Arc::clone(&preview)));
    assert!(outcome.committed, "{label}: {}", outcome.rejection);
    assert_preview_composes_committed(&core, &chunks, pieces, label);
    assert!(
        core.transit_network.roadbed.failed_nodes().is_empty(),
        "{label}"
    );
    assert_eq!(edgeless_nodes(&core), 0, "{label}");
    let state_after = (graph_hash(&core), roadbed_digest(&core));
    let loaded = saved_and_loaded(&mut core, label);
    assert_eq!(edgeless_nodes(&loaded), 0, "{label}: loaded");
    assert!(core.undo_action_internal(), "{label}");
    core.precompute_road_mesh_data();
    assert_eq!(
        (graph_hash(&core), roadbed_digest(&core)),
        state_before,
        "{label}: undo"
    );
    assert!(
        commit_2_lane(&mut core, &points, None).committed,
        "{label}: redo"
    );
    core.precompute_road_mesh_data();
    assert_eq!(
        (graph_hash(&core), roadbed_digest(&core)),
        state_after,
        "{label}: redo"
    );
    (core, preview, drawn)
}

// The same crossing at the arm limit (20 degrees): it crosses, a crossroads at the middle.
#[test]
fn a_crossing_at_the_arm_limit_crosses() {
    let angle = MIN_CONTACT_ANGLE_DEG;
    let heading = v(angle.to_radians().cos(), angle.to_radians().sin());
    let (core, preview, _) = build([-heading * 70.0, heading * 70.0], "at the limit");
    assert_eq!(preview.validation.overlap_side, None);
    assert_eq!(
        degree(&core, node_near(&core, v(0.0, 0.0))),
        4,
        "a crossroads"
    );
}

// A stroke from the south-west at 45 degrees crossing the street at x = 10 and ending 0.5 to 2 m
// past it (within the topology's 2 m end join, which moved such an end sideways to its square
// foot and sheared the stroke): the stroke ends at the crossing, on its own line, its last metres
// past it trimmed; every drawn point before the crossing stays bit for bit, also in the
// committed road, and no node stands at the square foot.
#[test]
fn an_end_just_past_an_oblique_crossing_ends_at_the_crossing() {
    let heading = v(45f32.to_radians().cos(), 45f32.to_radians().sin());
    let crossing = v(10.0, 0.0);
    for past in [0.5f32, 1.0, 1.5, 2.0] {
        let label = format!("{past} m past");
        let (core, preview, drawn) = build(
            [crossing - heading * 60.0, crossing + heading * past],
            &label,
        );
        let built = &preview.plan.prepared().points;
        let lead_in = preview.validation.lead_ins[1].expect("the end trimmed to the crossing");
        assert!(
            (lead_in.trim_m - past).abs() < 1.0e-3,
            "{label}: {lead_in:?}"
        );
        assert!(
            lead_in.target.distance_to(Vector3::new(10.0, 0.0, 0.0)) < 1.0e-3,
            "{label}: {lead_in:?}"
        );
        super::lead_ins::assert_drawn_kept(&drawn, built, preview.validation.lead_ins, &label);
        let graph = &core.region_graph;
        let t = node_near(&core, crossing);
        assert_eq!(
            degree(&core, t),
            3,
            "{label}: a T at the crossing, the stub gone"
        );
        let stroke = graph
            .node_adjacency(t)
            .iter()
            .map(|&edge| graph.edge(edge))
            .find(|edge| !edge.deleted && edge.geometry.contains(&drawn[0]))
            .unwrap_or_else(|| panic!("{label}: the stroke runs to the T"));
        // The committed road is the built stroke, bit for bit across, ending on the crossing:
        // the T stands on the street's centreline, within `f32` rounding of the crossing on the
        // stroke's line.
        let mut points = stroke.geometry.clone();
        if points[0] != drawn[0] {
            points.reverse();
        }
        let flat = |points: &[Vector3]| -> Vec<[u32; 2]> {
            points
                .iter()
                .map(|p| [p.x.to_bits(), p.z.to_bits()])
                .collect()
        };
        let n = points.len();
        assert_eq!(n, built.len(), "{label}");
        assert_eq!(
            flat(&points[..n - 1]),
            flat(&built[..n - 1]),
            "{label}: the committed road"
        );
        let (end, target) = (points[n - 1], built[n - 1]);
        assert!(
            (end.x - target.x).hypot(end.z - target.z) < 1.0e-5,
            "{label}: {end:?} against {target:?}"
        );
        let b = drawn[drawn.len() - 1];
        let foot = v(b.x, 0.0);
        assert!(
            !super::node_stands_at(&core, foot) || foot.distance_to(crossing) < 0.5,
            "{label}: no node at the square foot"
        );
    }
}
