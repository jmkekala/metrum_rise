// SPDX-License-Identifier: GPL-2.0-only

//! Round H2 (`ROAD-112`) tests: the height display. An end off the ground under it gets a ground
//! marker from its height to that ground (the segmentation's [`DraftGround`]: the highest
//! grounded carriageway, else the terrain) and a height chip, "+6.5 m bridge", whose part is the
//! segmentation's run at that end; an end on the ground gets neither; the chips place clear of
//! each other and of the handles by round V's rule.

use super::{click, move_to, session, view};
use crate::nodes::sim::road_tool::frame::GizmoLineClass;
use crate::nodes::sim::road_tool::height::ground_marks::{
    GROUND_RING_RADIUS_M, GroundMark, push_height_labels,
};
use crate::nodes::sim::road_tool::height::handles::DraftEnd;
use crate::nodes::sim::road_tool::labels::{EndPart, LabelKind, ToolLabel, place_chips};
use crate::nodes::sim::road_tool::session::{
    Pointer, RoadToolAction, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::nodes::sim::road_tool::wording::{NoticeFacts, label_text};
use crate::simulation::network::road_edit::DraftGround;
use godot::prelude::{Vector2, Vector3};

// A free stretch of the neighbourhood (round I's): no road within 50 m, flat ground.
const START: (f32, f32) = (-280.0, 250.0);
const END: (f32, f32) = (-180.0, 250.0);
// The ends' tolerance, metres.
const MM: f32 = 1e-3;

fn ground_m(x: f32, z: f32) -> f32 {
    let view = view();
    DraftGround::new(&view.terrain, &view.roadbed).height_at(x, z)
}

fn at((x, z): (f32, f32)) -> Pointer {
    Pointer::Scripted(Vector3::new(x, ground_m(x, z), z))
}

fn act(tool: &mut RoadToolSession, action: RoadToolAction, presses: usize) {
    for _ in 0..presses {
        tool.handle_event(view(), RoadToolEvent::Action(action));
    }
}

// A straight draft from `from` to `to` whose live end is `steps` height steps off the ground
// (negative: below), the frame's segmentation brought up to date.
fn draft(from: (f32, f32), to: (f32, f32), steps: i32) -> RoadToolSession {
    let mut tool = session();
    click(&mut tool, at(from));
    let action = if steps >= 0 {
        RoadToolAction::RaiseHandle
    } else {
        RoadToolAction::LowerHandle
    };
    act(&mut tool, action, steps.unsigned_abs() as usize);
    move_to(&mut tool, at(to));
    tool.update_draft(view());
    tool
}

// The frame's height chips, with their text.
fn height_chips(tool: &RoadToolSession) -> Vec<(ToolLabel, String)> {
    let frame = tool.frame();
    let mut labels = Vec::new();
    push_height_labels(
        &frame.ground_marks,
        &frame.stroke.points,
        &frame.draft.runs,
        &mut labels,
    );
    labels
        .into_iter()
        .map(|label| {
            let mut text = String::new();
            label_text(&label, &NoticeFacts::default(), &mut text);
            (label, text)
        })
        .collect()
}

// The marker's two strips: the vertical line from the end to its foot, then the ring on the
// ground around the foot, every ring point on the ground at its own XZ.
fn assert_marker(tool: &RoadToolSession, mark: &GroundMark) {
    let gizmos = &tool.frame().gizmos;
    let strips: Vec<usize> = (0..gizmos.line_classes.len())
        .filter(|&i| gizmos.line_classes[i] == GizmoLineClass::GroundMarker)
        .collect();
    let strip = |i: usize| {
        let start = gizmos.line_starts[i] as usize;
        let end = gizmos
            .line_starts
            .get(i + 1)
            .map_or(gizmos.line_points.len(), |&end| end as usize);
        &gizmos.line_points[start..end]
    };
    let line = strips
        .iter()
        .map(|&i| strip(i))
        .find(|points| points.len() == 2 && points[0] == mark.point)
        .expect("the marker's vertical line from the end");
    assert_eq!(line[1], mark.foot());
    let ring = strips
        .iter()
        .map(|&i| strip(i))
        .find(|points| {
            points.len() > 2
                && (points[0].x - mark.point.x - GROUND_RING_RADIUS_M).abs() < MM
                && (points[0].z - mark.point.z).abs() < MM
        })
        .expect("the marker's ring around the foot");
    assert_eq!(ring.first(), ring.last(), "the ring closes");
    for point in ring {
        assert!((point.y - ground_m(point.x, point.z)).abs() < MM);
    }
}

// A raised end over flat ground: a marker from the end's height to the terrain under it (1 mm),
// the chip "+6.5 m bridge"; the start on the ground has neither.
#[test]
fn a_raised_end_marks_its_ground_and_reads_bridge() {
    let tool = draft(START, END, 13);
    let frame = tool.frame();
    assert!(
        frame.ground_marks[0].is_none(),
        "the start stands on the ground"
    );
    let mark = frame.ground_marks[1].expect("the raised end's marker");
    let handle = frame.height_handles[1].unwrap();
    assert_eq!(mark.end, DraftEnd::End);
    assert!((mark.point.y - handle.height_m).abs() < MM);
    let terrain = view().terrain.sample_height_world(END.0, END.1) * crate::config::HEIGHT_SCALE;
    assert!((mark.ground_m - terrain).abs() < MM, "no road: the terrain");
    assert!((mark.height_above_m() - 6.5).abs() < MM);
    assert_marker(&tool, &mark);
    let chips = height_chips(&tool);
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].0.kind, LabelKind::EndHeight(EndPart::Bridge));
    assert_eq!(chips[0].1, "+6.5 m bridge");
    assert_eq!(chips[0].0.anchor, mark.point);
}

// A lowered end: 3 m down reads a cut (a held ground run below the clearance), 10 m down a
// tunnel; each marker's foot is the terrain over the end (1 mm).
#[test]
fn a_lowered_end_reads_cut_then_tunnel() {
    for (steps, text, part) in [
        (-6, "-3.0 m cut", EndPart::Cut),
        (-20, "-10.0 m tunnel", EndPart::Tunnel),
    ] {
        let tool = draft(START, END, steps);
        let frame = tool.frame();
        assert_eq!(frame.draft.refusal, None, "{text}");
        let mark = frame.ground_marks[1].expect("the lowered end's marker");
        assert!((mark.point.y - frame.height_handles[1].unwrap().height_m).abs() < MM);
        assert!((mark.ground_m - ground_m(END.0, END.1)).abs() < MM);
        assert!(mark.foot().y > mark.point.y, "the foot is over the end");
        assert_marker(&tool, &mark);
        let chips = height_chips(&tool);
        assert_eq!(chips.len(), 1, "{text}");
        assert_eq!(chips[0].0.kind, LabelKind::EndHeight(part), "{text}");
        assert_eq!(chips[0].1, text);
    }
}

// An end raised over a lower road (the road along z = -60, network snap off so the end stays
// free): the ground is that road's carriageway top, the highest grounded carriageway as the
// segmentation reads it, not the terrain.
#[test]
fn an_end_over_a_lower_road_marks_the_road() {
    let mut tool = session();
    tool.set_option(RoadToolOption::NetworkSnap(false));
    click(&mut tool, at((-150.0, -120.0)));
    act(&mut tool, RoadToolAction::RaiseHandle, 16);
    move_to(&mut tool, at((-150.0, -60.0)));
    tool.update_draft(view());
    let mark = tool.frame().ground_marks[1].expect("the raised end's marker");
    let mut carriageway: Option<f32> = None;
    view()
        .roadbed
        .for_each_grounded_carriageway_top_at(-150.0, -60.0, |_, height_m| {
            carriageway = Some(carriageway.map_or(height_m, |best| best.max(height_m)));
        });
    let road = carriageway.expect("the road along z = -60 under the end");
    assert!((mark.ground_m - road).abs() < MM);
    assert!((mark.point.y - tool.frame().height_handles[1].unwrap().height_m).abs() < MM);
    assert_marker(&tool, &mark);
    let chips = height_chips(&tool);
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].0.kind, LabelKind::EndHeight(EndPart::Bridge));
    let expected = format!("{:+.1} m bridge", mark.height_above_m());
    assert_eq!(chips[0].1, expected);
}

// Ends on the ground: no marker, no ground marker strip, no height chip.
#[test]
fn ends_on_the_ground_show_no_marker_or_chip() {
    let tool = draft(START, END, 0);
    let frame = tool.frame();
    assert_eq!(frame.ground_marks, [None, None]);
    assert!(
        !frame
            .gizmos
            .line_classes
            .contains(&GizmoLineClass::GroundMarker)
    );
    assert!(height_chips(&tool).is_empty());

    // A raised live end on the placed start (no stroke): its marker, but no chip (no part yet).
    let mut tool = session();
    click(&mut tool, at(START));
    act(&mut tool, RoadToolAction::RaiseHandle, 13);
    move_to(&mut tool, at(START));
    tool.update_draft(view());
    assert!(tool.frame().stroke.points.is_empty());
    assert!(tool.frame().ground_marks[1].is_some());
    assert!(height_chips(&tool).is_empty());
}

// Both ends of a short draft raised (a new start takes the live end's offset): every chip,
// the two height chips included, placed by round V's rule on a top-down projection, overlaps
// no other chip and no handle.
#[test]
fn height_chips_place_clear_of_the_other_chips_and_the_handles() {
    let mut tool = session();
    act(&mut tool, RoadToolAction::RaiseHandle, 13);
    click(&mut tool, at(START));
    move_to(&mut tool, at((START.0 + 30.0, START.1 + 10.0)));
    tool.update_draft(view());
    let frame = tool.frame();
    assert!(frame.ground_marks.iter().all(Option::is_some));
    let mut labels = frame.labels.clone();
    push_height_labels(
        &frame.ground_marks,
        &frame.stroke.points,
        &frame.draft.runs,
        &mut labels,
    );
    let heights = labels
        .iter()
        .filter(|label| matches!(label.kind, LabelKind::EndHeight(EndPart::Bridge)))
        .count();
    assert_eq!(heights, 2);
    // Top down, 4 px a metre, the screen's y down along +z.
    let screen = |point: Vector3| Vector2::new(point.x * 4.0, point.z * 4.0);
    let anchors: Vec<Vector2> = labels.iter().map(|label| screen(label.anchor)).collect();
    let sides: Vec<Vector2> = labels
        .iter()
        .map(|label| Vector2::new(label.side.x as f32, label.side.y as f32))
        .collect();
    let sizes = vec![Vector2::new(96.0, 22.0); labels.len()];
    let obstacles: Vec<Vector2> = frame.gizmos.handles.iter().map(|&h| screen(h)).collect();
    let mut corners = Vec::new();
    place_chips(
        &anchors,
        &sides,
        &sizes,
        &obstacles,
        18.0,
        6.0,
        &mut corners,
    );
    let overlap = |a: Vector2, a_size: Vector2, b: Vector2, b_size: Vector2| {
        a.x < b.x + b_size.x && b.x < a.x + a_size.x && a.y < b.y + b_size.y && b.y < a.y + a_size.y
    };
    for i in 0..corners.len() {
        for j in i + 1..corners.len() {
            assert!(
                !overlap(corners[i], sizes[i], corners[j], sizes[j]),
                "chips {i} and {j}"
            );
        }
        for obstacle in &obstacles {
            let corner = *obstacle - Vector2::splat(9.0);
            assert!(
                !overlap(corners[i], sizes[i], corner, Vector2::splat(18.0)),
                "chip {i}"
            );
        }
    }
}
