// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-118` round JS5b: the preview shows the junction that will form. A valid stroke whose
//! end joins a road carries the outline of the node piece that forms there, equal to the outer
//! chains of the piece its commit installs (a T on a road's side, a crossroads in a T's open
//! side, a bend at a road end); a stroke that joins nothing carries none; a `roads_overlap`
//! refusal carries the overlapped stretch of the road it names; the draft line (the rubber band)
//! ends on the drawn end, never on the node the join builds to. Round JS5d: round JS5c's corner
//! and pass T carry their pieces' outlines too.

use super::contacts::{lay, preview_parked, v};
use super::stubs_and_joins::node_near;
use super::*;
use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
use crate::nodes::sim::road_tool::draw::marks::JoinMarks;
use crate::nodes::sim::road_tool::frame::GizmoLineClass;
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::road_edit::{EndContact, RoadProfile};
use crate::simulation::network::road_types::RoadAttributes;

// A flat city of 2-lane roads drawn in order, without parking.
fn city(roads: &[[(f32, f32); 2]]) -> SimCore {
    super::contacts::city(roads, 1)
}

// Previews the 2-lane stroke `ends`, commits that preview and returns the preview.
fn preview_and_commit(core: &mut SimCore, ends: [Vector2; 2], label: &str) -> Arc<RoadPreview> {
    let points = lay(core, &ends);
    let (preview, _) = preview_parked(core, &points, (1, 1), false, RoadParking::NONE);
    assert!(
        preview.validation.is_valid,
        "{label}: {}",
        preview.validation.invalid_reason
    );
    let outcome = core.commit_road(
        RoadCommitRequest {
            parking: RoadParking::NONE,
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: false,
            profile: RoadProfile::FollowGround,
            median: None,
            attributes: RoadAttributes::DEFAULT,
            preview: Some(Arc::clone(&preview)),
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{label}: {}", outcome.rejection);
    preview
}

// The committed piece of `node`, outlined as the preview outlines it: its outer chains in order,
// closed across its mouths.
fn committed_outline(core: &SimCore, node: u32, label: &str) -> JoinMarks {
    let piece = core
        .transit_network
        .roadbed
        .node_piece(node)
        .unwrap_or_else(|| panic!("{label}: the joined node has a piece"));
    let mut marks = JoinMarks::default();
    marks.push_piece(piece.mesh());
    marks
}

fn assert_outline(preview: &RoadPreview, expected: &JoinMarks, label: &str) {
    let marks = &preview.marks;
    assert!(!marks.refused, "{label}: the outline is not a refusal");
    assert_eq!(marks.starts.len(), 1, "{label}: one piece outlined");
    assert!(marks.points.len() > 4, "{label}: {:?}", marks.points);
    assert_eq!(
        marks, expected,
        "{label}: the outline is the committed piece's outer chains"
    );
    assert_eq!(marks.points.first(), marks.points.last(), "{label}: closed");
}

// A stroke from the south at 60 degrees ending 4 m short of a street's centreline: a T on its
// side. A stroke from the south ending 8 m short of a T's street, 2 m east of the T, in its open
// side: a crossroads. A cap over a street's road end in line 2 m to its side: a bend. Each
// preview's outline equals the committed piece's outer chains.
#[test]
fn the_outline_is_the_committed_pieces_outer_chains() {
    let (dx, dz) = (60f32.to_radians().cos(), 60f32.to_radians().sin());
    let end = v(10.0, -4.0);
    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]]);
    let preview = preview_and_commit(&mut core, [end - v(dx, dz) * 60.0, end], "side T");
    let Some(EndContact::Road(point)) = preview.validation.contacts[1] else {
        panic!("side T: {:?}", preview.validation.contacts);
    };
    let t = node_near(&core, v(point.x, 0.0));
    assert_outline(&preview, &committed_outline(&core, t, "side T"), "side T");

    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)], [(0.0, 0.0), (0.0, 100.0)]]);
    let t = node_near(&core, v(0.0, 0.0));
    let preview = preview_and_commit(&mut core, [v(2.0, -70.0), v(2.0, -8.0)], "node join");
    assert_eq!(preview.validation.joined_nodes, [None, Some(t)]);
    assert_outline(
        &preview,
        &committed_outline(&core, t, "node join"),
        "node join",
    );

    let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)]]);
    let road_end = node_near(&core, v(0.0, 0.0));
    let preview = preview_and_commit(&mut core, [v(86.0, 2.0), v(6.0, 2.0)], "road end");
    assert_eq!(preview.validation.joined_nodes, [None, Some(road_end)]);
    assert_outline(
        &preview,
        &committed_outline(&core, road_end, "road end"),
        "road end",
    );
}

// Round JS5c's corner and pass (round JS5d): a cap over a street's road end at 46 degrees
// (`ours-2`) makes a corner where the two lines meet, and a stroke running north 6 m beyond the
// street's end grows the street onto it, a T. Each preview's outline equals the outer chains of
// the piece the commit installs at that node.
#[test]
fn the_corner_and_the_pass_outline_their_pieces() {
    let street = [[(-100.0, 0.0), (0.0, 0.0)]];
    let (dx, dz) = (134f32.to_radians().cos(), 134f32.to_radians().sin());
    let end = v(8.345, 0.0);
    let mut core = city(&street);
    let preview = preview_and_commit(&mut core, [end - v(dx, dz) * 80.0, end], "corner");
    let growth = preview.validation.growths[1].expect("a corner");
    let corner = node_near(&core, v(growth.target.x, growth.target.z));
    assert_outline(
        &preview,
        &committed_outline(&core, corner, "corner"),
        "corner",
    );

    let mut core = city(&street);
    let preview = preview_and_commit(&mut core, [v(6.0, -60.0), v(6.0, 60.0)], "pass");
    assert_eq!(preview.validation.growths, [None; 2], "pass: no corner");
    let t = node_near(&core, v(6.0, 0.0));
    assert_outline(&preview, &committed_outline(&core, t, "pass"), "pass");
}

// A stroke on open ground, and one whose end stops a footprint short of a road end, join
// nothing: no marks.
#[test]
fn a_stroke_that_joins_nothing_marks_nothing() {
    let mut core = city(&[[(-100.0, 0.0), (0.0, 0.0)]]);
    for (label, ends) in [
        ("open ground", [v(-60.0, 60.0), v(60.0, 80.0)]),
        ("12 m short", [v(92.0, 0.0), v(12.0, 0.0)]),
    ] {
        let points = lay(&core, &ends);
        let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
        assert!(preview.validation.is_valid, "{label}");
        assert_eq!(preview.validation.joined_nodes, [None; 2], "{label}");
        assert!(preview.marks.is_empty(), "{label}: {:?}", preview.marks);
    }
}

// A 2-lane stroke whose end meets a street at 15 degrees 1 m off its centreline is refused as
// `roads_overlap`: its marks are the street's footprint (5 m to either side) over the stretch the
// two footprints overlap, measured across.
#[test]
fn an_overlap_refusal_marks_its_stretch() {
    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]]);
    let heading = v(15f32.to_radians().cos(), 15f32.to_radians().sin());
    let end = v(10.0, -1.0);
    let points = lay(&core, &[end - heading * 60.0, end]);
    let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
    let refused = &preview.validation;
    assert_eq!(refused.invalid_reason, "roads_overlap", "{refused:?}");
    assert_eq!(refused.overlap_end, Some(1));
    let marks = &preview.marks;
    assert!(marks.refused, "{marks:?}");
    assert_eq!(marks.starts, [0], "one stretch");
    assert_eq!(marks.points.first(), marks.points.last(), "closed");
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    for point in &marks.points {
        assert!(
            (point.z.abs() - 5.0).abs() < 1.0e-3,
            "on the street's edges: {point:?}"
        );
        low = low.min(point.x);
        high = high.max(point.x);
    }
    // The two footprints (5 m half widths) overlap where the street's centreline lies within
    // 10 m of the end's last stretch: the prepared stroke's points back from the end over whole
    // segments until 10 m / sin 25 is covered, as the contact reads it. Back there the stroke
    // stands `w` off the centreline, so the stretch starts 10 m from that point; ahead it ends
    // at x = 10 + sqrt(99), the end's round cap.
    let prepared = &preview.plan.prepared().points;
    let window_m = 10.0 / 25f32.to_radians().sin();
    let (mut run, mut back) = (0.0, prepared.len() - 1);
    while back > 0 && run < window_m {
        run += (prepared[back].x - prepared[back - 1].x)
            .hypot(prepared[back].z - prepared[back - 1].z);
        back -= 1;
    }
    let w = prepared[back];
    let from = w.x - (100.0 - w.z * w.z).sqrt();
    assert!((low - from).abs() < 0.01, "{low} against {from}");
    assert!((high - (10.0 + 99f32.sqrt())).abs() < 0.01, "{high}");
}

// The draft line follows the drawn end: a session drawing the side T's stroke (no snap) ends its
// draft line, and its stroke, on the cursor, 4 m short of the centreline the join builds to.
#[test]
fn the_rubber_band_ends_on_the_drawn_end() {
    let mut core = city(&[[(-100.0, 0.0), (100.0, 0.0)]]);
    let view = road_tool_snapshot_from_core(&core).expect("a published view");
    let (dx, dz) = (60f32.to_radians().cos(), 60f32.to_radians().sin());
    let ends = lay(&core, &[v(10.0, -4.0) - v(dx, dz) * 60.0, v(10.0, -4.0)]);
    let mut session = RoadToolSession::new();
    for option in [
        RoadToolOption::NetworkSnap(false),
        RoadToolOption::GuideLines(false),
    ] {
        session.set_option(option);
    }
    let point = |session: &mut RoadToolSession, at: Vector3| {
        session.handle_event(
            &view,
            RoadToolEvent::Pointer {
                pointer: Pointer::Scripted(at),
                shift: false,
                ctrl: false,
            },
        );
        session.update_draft(&view);
    };
    point(&mut session, ends[0]);
    session.handle_event(&view, RoadToolEvent::Press(PointerButton::Left));
    point(&mut session, ends[1]);
    let frame = session.frame();
    let drawn = *frame.stroke.points.last().expect("a stroke");
    assert!(drawn.distance_to(ends[1]) < 1.0e-3, "{drawn:?}");
    let gizmos = &frame.gizmos;
    let strip = gizmos
        .line_classes
        .iter()
        .position(|&class| class == GizmoLineClass::Draft)
        .expect("a draft line");
    let to = gizmos
        .line_starts
        .get(strip + 1)
        .map_or(gizmos.line_points.len(), |&end| end as usize);
    let band_end = gizmos.line_points[to - 1];
    assert!(
        (band_end.x - drawn.x).hypot(band_end.z - drawn.z) < 1.0e-3,
        "the band ends on the drawn end: {band_end:?}"
    );
    let points = frame.stroke.points.clone();
    let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
    assert!(preview.validation.is_valid);
    let built = *preview.plan.prepared().points.last().unwrap();
    assert!(
        built.z.abs() < 1.0e-3,
        "the join builds to the centreline: {built:?}"
    );
    assert!(
        (band_end.x - built.x).hypot(band_end.z - built.z) > 4.0,
        "the band is not pulled to the node"
    );
}
