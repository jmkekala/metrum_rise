// SPDX-License-Identifier: GPL-2.0-only

//! Round V (`ROAD-112`) tests: the label chips of each mode (values against their closed forms,
//! no upkeep, bearing and design speed present), the freeform hint and Build / Cancel chips, the
//! chip placement rule (deterministic, no overlap), the zoning-depth guide against the cells a
//! straight road generates, and the alignment guide drawn from its source.

use super::{ack, click, ground_at, move_to, scripted_simple_curve, session, view};
use crate::nodes::sim::road_tool::frame::{GizmoLineClass, RoadToolFrame};
use crate::nodes::sim::road_tool::guides::{
    GUIDE_PAST_M, grid_curb_offset_m, push_alignment_guide, push_grid_curbs, zoning_depth_m,
};
use crate::nodes::sim::road_tool::labels::{
    HintStep, LabelKind, LabelTone, PreviewFacts, ToolLabel, apply_preview, middle_label,
    place_chips,
};
use crate::nodes::sim::road_tool::measure::{RoadTypeSpec, bearing_deg, design_speed_kmh};
use crate::nodes::sim::road_tool::pick::lane_width_m;
use crate::nodes::sim::road_tool::session::{
    Pointer, RoadToolAction, RoadToolEvent, RoadToolOption,
};
use crate::nodes::sim::road_tool::snap::GuideLine;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::{Edge, RegionGraph, SidewalkSides};
use crate::simulation::network::guide::DVec2;
use crate::simulation::network::road_types::BUILTIN_ROAD_TYPES;
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::cells::{CellBounds, CellStore};
use godot::prelude::{Vector2, Vector3};

fn scripted(x: f64, z: f64) -> Pointer {
    Pointer::Scripted(Vector3::new(
        x as f32,
        ground_at(x as f32, z as f32),
        z as f32,
    ))
}

fn labels_of(frame: &RoadToolFrame, kind: LabelKind) -> Vec<ToolLabel> {
    frame
        .labels
        .iter()
        .filter(|label| label.kind == kind)
        .copied()
        .collect()
}

fn kinds(frame: &RoadToolFrame) -> Vec<LabelKind> {
    frame.labels.iter().map(|label| label.kind).collect()
}

// A straight stroke of 100 m (80 by 60) on open ground: one length chip at its middle, the cost
// and the bearing at its end, no total, no radius, no design speed, no upkeep (there is none).
#[test]
fn straight_draft_labels() {
    let mut tool = session();
    click(&mut tool, scripted(-280.0, 200.0));
    move_to(&mut tool, scripted(-200.0, 260.0));
    let frame = tool.frame();
    assert_eq!(
        kinds(frame),
        vec![LabelKind::Cost, LabelKind::Bearing, LabelKind::PieceLength]
    );
    let length = labels_of(frame, LabelKind::PieceLength)[0];
    assert!((length.value - 100.0).abs() < 1e-3, "{}", length.value);
    assert!((length.anchor.x + 240.0).abs() < 1e-3 && (length.anchor.z - 230.0).abs() < 1e-3);
    let bearing = labels_of(frame, LabelKind::Bearing)[0];
    assert!((bearing.value - bearing_deg(d(0.8, 0.6))).abs() < 1e-9);
    assert_eq!(
        labels_of(frame, LabelKind::Cost)[0].value,
        frame.hud.build_cost
    );
    assert!(
        frame
            .labels
            .iter()
            .all(|label| label.tone == LabelTone::Normal)
    );
}

fn d(x: f64, y: f64) -> DVec2 {
    DVec2::new(x, y)
}

// The simple curve of `drawn_curve_reports_its_constructed_radius` (R = 100 m, a 20 m lead): the
// lead's length, the arc's radius at its middle, outside the curve, the total and the design
// speed, each its closed form within 1 mm.
#[test]
fn simple_curve_labels() {
    let tool = scripted_simple_curve((-280.0, 200.0), (-210.0, 200.0), (-180.0, 240.0));
    let frame = tool.frame();
    let radius = labels_of(frame, LabelKind::Radius);
    assert_eq!(radius.len(), 1);
    assert!(
        (radius[0].value - 100.0).abs() < 1e-3,
        "{}",
        radius[0].value
    );
    let lead = labels_of(frame, LabelKind::PieceLength);
    assert_eq!(lead.len(), 1);
    assert!((lead[0].value - 20.0).abs() < 1e-3, "{}", lead[0].value);
    let sweep = 4.0_f64.atan2(3.0);
    let total = labels_of(frame, LabelKind::TotalLength);
    assert!(
        (total[0].value - (20.0 + 100.0 * sweep)).abs() < 1e-3,
        "{}",
        total[0].value
    );
    // The arc's centre is 100 m left of the lead at (-260, 300): its chip is pushed away from it.
    let centre = d(-260.0, 300.0);
    let at = d(f64::from(radius[0].anchor.x), f64::from(radius[0].anchor.z));
    assert!(((at - centre).length() - 100.0).abs() < 1e-3);
    assert!(radius[0].side.dot((at - centre).normalize()) > 0.999);
    let speed = labels_of(frame, LabelKind::DesignSpeed)[0];
    assert_eq!(speed.value, design_speed_kmh(100.0));
    // 50 km/h against the road's limit of 13.89 m/s, read as a hair over 50 km/h: the HUD's
    // warning, as round D defined it.
    let warned = if frame.hud.design_speed_warning {
        LabelTone::Warning
    } else {
        LabelTone::Normal
    };
    assert_eq!(speed.tone, warned);
    assert_eq!(labels_of(frame, LabelKind::Bearing).len(), 1);
}

// A tight curve (20 m radius, 20 km/h under the road's 50) turns its design speed chip amber; a
// curve under the illegal radius is a refusal notice in place of the cost.
#[test]
fn curve_warnings_and_refusals() {
    let tool = scripted_simple_curve((-280.0, 200.0), (-260.0, 200.0), (-260.0, 220.0));
    let speed = labels_of(tool.frame(), LabelKind::DesignSpeed)[0];
    assert_eq!(speed.tone, LabelTone::Warning);
    let tight = scripted_simple_curve((-280.0, 200.0), (-276.0, 200.0), (-276.0, 204.0));
    let frame = tight.frame();
    assert!(frame.hud.tight_radius);
    assert!(labels_of(frame, LabelKind::Cost).is_empty());
    let notice = labels_of(frame, LabelKind::Notice);
    assert_eq!(notice.len(), 1);
    assert_eq!(notice[0].tone, LabelTone::Refused);
}

// A chained stroke's start angle against the stroke before it: 180 straight on, 90 square, as
// whole degrees.
#[test]
fn chained_start_angle() {
    for (end, expected) in [((-120.0, 200.0), 180.0), ((-200.0, 280.0), 90.0)] {
        let mut tool = session();
        click(&mut tool, scripted(-280.0, 200.0));
        click(&mut tool, scripted(-200.0, 200.0));
        ack(&mut tool, true);
        move_to(&mut tool, scripted(end.0, end.1));
        let angles = labels_of(tool.frame(), LabelKind::Angle);
        assert_eq!(angles.len(), 1, "{:?}", kinds(tool.frame()));
        assert_eq!(angles[0].value, expected);
        assert_eq!(angles[0].anchor.x, -200.0);
    }
}

// An S-curve: both arcs' radii, as the HUD reads them.
#[test]
fn s_curve_labels() {
    let mut tool = session();
    tool.handle_event(
        view(),
        RoadToolEvent::Action(RoadToolAction::ModeComplexCurve),
    );
    click(&mut tool, scripted(-280.0, 200.0));
    click(&mut tool, scripted(-250.0, 200.0));
    click(&mut tool, scripted(-200.0, 240.0));
    move_to(&mut tool, scripted(-170.0, 240.0));
    let frame = tool.frame();
    let mut radii: Vec<f64> = labels_of(frame, LabelKind::Radius)
        .iter()
        .map(|label| label.value)
        .collect();
    radii.sort_by(f64::total_cmp);
    let (radius, other) = (
        frame.hud.radius_m.unwrap(),
        frame.hud.other_radius_m.unwrap(),
    );
    assert_eq!(radii.len(), 2);
    assert!((radii[0] - radius).abs() < 1e-9 && (radii[1] - other).abs() < 1e-9);
    assert_eq!(labels_of(frame, LabelKind::TotalLength).len(), 1);
}

// Freeform: the placing hint at the last node, then in the edit phase the Build and Cancel chips
// and the editing hint.
#[test]
fn freeform_hint_and_buttons() {
    let mut tool = session();
    tool.handle_event(view(), RoadToolEvent::Action(RoadToolAction::ModeFreeform));
    click(&mut tool, scripted(-280.0, 200.0));
    click(&mut tool, scripted(-240.0, 220.0));
    move_to(&mut tool, scripted(-200.0, 200.0));
    let hint = labels_of(tool.frame(), LabelKind::Hint(HintStep::AddNode));
    assert_eq!(hint.len(), 1);
    assert!(labels_of(tool.frame(), LabelKind::Build).is_empty());
    tool.handle_event(view(), RoadToolEvent::Action(RoadToolAction::Confirm));
    let frame = tool.frame();
    assert_eq!(
        labels_of(frame, LabelKind::Hint(HintStep::EditNodes)).len(),
        1
    );
    assert!(labels_of(frame, LabelKind::Hint(HintStep::AddNode)).is_empty());
    for kind in [LabelKind::Build, LabelKind::Cancel] {
        let chip = labels_of(frame, kind);
        assert_eq!(chip.len(), 1);
        assert_eq!(chip[0].anchor.x, -240.0, "beside the last node");
    }
}

// The preview's facts: the class-priced cost, a grade from 0.5 % on the middle chip's other
// side, a warning notice, and a refusal in place of the cost.
#[test]
fn preview_facts_apply() {
    let mut tool = session();
    click(&mut tool, scripted(-280.0, 200.0));
    move_to(&mut tool, scripted(-200.0, 260.0));
    let base = tool.frame().labels.clone();
    let middle = middle_label(&base);
    let mut labels = base.clone();
    let facts = PreviewFacts {
        build_cost: Some(1234.0),
        max_grade: Some(0.004),
        ..PreviewFacts::default()
    };
    apply_preview(&mut labels, facts, middle);
    assert_eq!(labels.len(), base.len(), "no slope chip under 0.5 %");
    assert_eq!(labels[0].value, 1234.0);
    let mut labels = base.clone();
    apply_preview(
        &mut labels,
        PreviewFacts {
            max_grade: Some(-0.05),
            warning: true,
            ..facts
        },
        middle,
    );
    let slope = labels
        .iter()
        .find(|label| label.kind == LabelKind::Slope)
        .unwrap();
    assert!((slope.value + 5.0).abs() < 1e-9);
    assert_eq!(slope.side, -middle.unwrap().side);
    assert_eq!(labels[0].kind, LabelKind::Notice);
    assert_eq!(labels[0].tone, LabelTone::Warning);
    let mut labels = base.clone();
    apply_preview(
        &mut labels,
        PreviewFacts {
            refused: true,
            ..facts
        },
        middle,
    );
    assert_eq!(labels[0].kind, LabelKind::Notice);
    assert_eq!(labels[0].tone, LabelTone::Refused);
    assert!(labels.iter().all(|label| label.kind != LabelKind::Cost));
}

fn overlap(a: Vector2, a_size: Vector2, b: Vector2, b_size: Vector2) -> bool {
    a.x < b.x + b_size.x && b.x < a.x + a_size.x && a.y < b.y + b_size.y && b.y < a.y + a_size.y
}

// The placement rule on fixtures: a dozen chips on one anchor, chips along a line with
// alternating sides, chips round a handle: no two chips overlap, none covers a handle, and the
// same input places them bit for bit the same.
#[test]
fn chip_placement_is_deterministic_and_disjoint() {
    let size = |index: usize| Vector2::new(60.0 + (index % 4) as f32 * 17.0, 22.0);
    let fixtures: Vec<(Vec<Vector2>, Vec<Vector2>, Vec<Vector2>)> = vec![
        (
            vec![Vector2::new(400.0, 300.0); 12],
            (0..12)
                .map(|index| Vector2::from_angle(index as f32 * 0.7))
                .collect(),
            vec![],
        ),
        (
            (0..10)
                .map(|index| Vector2::new(100.0 + index as f32 * 25.0, 200.0 + index as f32 * 9.0))
                .collect(),
            (0..10)
                .map(|index| Vector2::new(0.3, if index % 2 == 0 { 1.0 } else { -1.0 }))
                .collect(),
            vec![],
        ),
        (
            vec![Vector2::new(500.0, 500.0); 6],
            vec![Vector2::new(1.0, 0.0); 6],
            vec![Vector2::new(540.0, 500.0), Vector2::new(600.0, 520.0)],
        ),
    ];
    for (anchors, sides, obstacles) in fixtures {
        let sizes: Vec<Vector2> = (0..anchors.len()).map(size).collect();
        let mut first = Vec::new();
        place_chips(&anchors, &sides, &sizes, &obstacles, 18.0, 6.0, &mut first);
        let mut again = Vec::new();
        place_chips(&anchors, &sides, &sizes, &obstacles, 18.0, 6.0, &mut again);
        assert_eq!(first, again);
        assert_eq!(first.len(), anchors.len());
        for i in 0..first.len() {
            for j in i + 1..first.len() {
                assert!(
                    !overlap(first[i], sizes[i], first[j], sizes[j]),
                    "chips {i} and {j} overlap"
                );
            }
            for obstacle in &obstacles {
                let corner = *obstacle - Vector2::splat(9.0);
                assert!(!overlap(first[i], sizes[i], corner, Vector2::splat(18.0)));
            }
        }
    }
}

// The zoning-depth guide's radius and outline offset equal the outer edge of the cells a
// straight road of the type generates, to 1 mm: kerb plus six 10 m rows (65 m for the default
// two-lane road).
#[test]
fn zoning_depth_matches_generated_cells() {
    let spec = RoadTypeSpec::default();
    let config = WorldConfig::default();
    let cell_m = f64::from(config.zone_cell_m);
    let depth = zoning_depth_m(&spec, cell_m).expect("the default road zones");
    let (_graph, store) = zoned_straight_road(&spec, &config);
    let bounds = CellBounds {
        min: DVec2::splat(-300.0),
        max: DVec2::splat(300.0),
    };
    let mut outer: f64 = 0.0;
    store.visit_in_bounds(bounds, |key| {
        let frame = store.frame(key.grid).expect("a cell's grid");
        for corner in frame.corners(key.x, key.y) {
            outer = outer.max(corner.y.abs());
        }
    });
    assert!(outer > 0.0, "the road generates cells");
    assert!(
        (depth - outer).abs() < 1e-3,
        "guide {depth} m, cells reach {outer} m"
    );
    // No guide for a type without zoning.
    let mut plain = spec;
    plain.attributes.zoning = false;
    assert_eq!(zoning_depth_m(&plain, cell_m), None);
}

// A straight road of `spec` from (0, 0) to (120, 0) and the cells it generates: kerbs at
// z = +-5 m for the default road, so cell lines at x = 10 k and z = 5 + 10 k.
pub(super) fn zoned_straight_road(
    spec: &RoadTypeSpec,
    config: &WorldConfig,
) -> (RegionGraph, CellStore) {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(120.0, 0.0, 0.0), NodeType::Junction);
    let points = vec![graph.node(a).pos, graph.node(b).pos];
    let edge = graph.add_edge(Edge {
        start_node: a,
        end_node: b,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        sidewalks: SidewalkSides::BOTH,
        class: EdgeClass::Standard,
        width: Edge::cross_section_width_m(
            spec.fwd_lanes,
            spec.bkw_lanes,
            spec.median,
            spec.parking,
        ),
        physical_length: 120.0,
        geometry: points.clone(),
        physical_geometry: points,
        ..Default::default()
    });
    let mut store = CellStore::default();
    let bounds = CellBounds {
        min: DVec2::splat(-300.0),
        max: DVec2::splat(300.0),
    };
    store.generate_in_bounds(&graph, config, bounds, |_| false);
    store.refresh_road_alignments(&graph, config, [edge]);
    (graph, store)
}

// Round V item 8: for every built-in type, the drawn curbs lie the snap's curb offset either
// side of a grid-snapped stroke, the curb nearer the road on a cell line along it, and the end's
// tick on the column line the snap ended the road half a road past.
#[test]
fn grid_curbs_match_the_snap_for_every_built_in_type() {
    let config = WorldConfig::default();
    let cell_m = f64::from(config.zone_cell_m);
    let (graph, store) = zoned_straight_road(&RoadTypeSpec::default(), &config);
    let snap = store.road_grid_snap(cell_m);
    let on_line = |value: f64, base: f64| {
        let cells = (value - base) / cell_m;
        (cells - cells.round()).abs() < 1e-6
    };
    for builtin in BUILTIN_ROAD_TYPES {
        let section = builtin.cross_section;
        let spec = RoadTypeSpec {
            fwd_lanes: section.fwd_lanes,
            bkw_lanes: section.bkw_lanes,
            ..RoadTypeSpec::default()
        };
        let width = f64::from(lane_width_m(
            i32::from(spec.fwd_lanes),
            i32::from(spec.bkw_lanes),
        ));
        let offset = grid_curb_offset_m(&spec);
        assert_eq!(offset, width * 0.5 + 1.5, "{}", builtin.id);
        let start = snap.snap_road_cursor(&graph, d(23.3, 47.1), None, width);
        let end = snap.snap_road_cursor(&graph, d(83.7, 47.4), Some(start), width);
        assert!(
            (end.y - start.y).abs() < 1e-6,
            "{}: the stroke runs along x",
            builtin.id
        );
        // The curb nearer the road lies on a cell line (z = 5 + 10 k).
        assert!(on_line(start.y - offset, 5.0), "{}: {start:?}", builtin.id);
        let mut gizmos = crate::nodes::sim::road_tool::frame::Gizmos::default();
        let world = |p: DVec2| Vector3::new(p.x as f32, 0.0, p.y as f32);
        push_grid_curbs(&mut gizmos, world(start), world(end), offset);
        // Two curbs and two ticks: each curb at the offset, each tick on the end's cell line.
        assert_eq!(gizmos.line_classes, [GizmoLineClass::GridCurb; 4]);
        for strip in [0, 2] {
            let point = gizmos.line_points[gizmos.line_starts[strip] as usize];
            let lateral = (f64::from(point.z) - start.y).abs();
            assert!((lateral - offset).abs() < 1e-4, "{}: {lateral}", builtin.id);
        }
        for strip in [1, 3] {
            let point = gizmos.line_points[gizmos.line_starts[strip] as usize];
            assert!(
                on_line(f64::from(point.x), 0.0),
                "{}: tick at {point:?}",
                builtin.id
            );
        }
    }
}

// Item 9: for a straight stroke the zoning guide's outside is the stadium (both side offsets and
// the circles' outer half caps, every point the depth from the stroke) and its inside the two
// inner half circles; none without zoning.
#[test]
fn zoning_guide_splits_into_the_stadium_and_the_inner_half_circles() {
    let mut tool = session();
    click(&mut tool, scripted(-280.0, 200.0));
    move_to(&mut tool, scripted(-200.0, 200.0));
    let depth = tool.frame().zoning_depth_m.unwrap();
    let gizmos = &tool.frame().gizmos;
    // Distance to the stroke, the segment (-280, 200)-(-200, 200).
    let distance = |p: Vector3| {
        let x = f64::from(p.x).clamp(-280.0, -200.0);
        d(f64::from(p.x) - x, f64::from(p.z) - 200.0).length()
    };
    let mut lengths = [0.0; 2];
    let mut strips = [0; 2];
    for (strip, class) in gizmos.line_classes.iter().enumerate() {
        let inside = match class {
            GizmoLineClass::ZoningOutside => false,
            GizmoLineClass::ZoningInside => true,
            _ => continue,
        };
        let from = gizmos.line_starts[strip] as usize;
        let to = gizmos
            .line_starts
            .get(strip + 1)
            .map_or(gizmos.line_points.len(), |&to| to as usize);
        let points = &gizmos.line_points[from..to];
        for point in points {
            let r = distance(*point);
            if inside {
                assert!(r < depth + 1e-3, "inside point {point:?} at {r}");
                assert!(point.x >= -280.001 && point.x <= -199.999, "{point:?}");
            } else {
                assert!(
                    (r - depth).abs() < 0.1 + 1e-3,
                    "outside point {point:?} at {r}"
                );
            }
        }
        lengths[usize::from(inside)] += points
            .windows(2)
            .map(|pair| f64::from(pair[0].distance_to(pair[1])))
            .sum::<f64>();
        strips[usize::from(inside)] += 1;
    }
    let circle = std::f64::consts::TAU * depth;
    assert_eq!(
        strips,
        [5, 3],
        "outside: two caps (one split at angle 0) and two sides"
    );
    assert!(
        (lengths[0] - (160.0 + circle)).abs() < 0.01 * circle,
        "{lengths:?}"
    );
    assert!((lengths[1] - circle).abs() < 0.01 * circle, "{lengths:?}");
    let mut spec = RoadTypeSpec::default();
    spec.attributes.zoning = false;
    tool.set_option(RoadToolOption::RoadType(spec));
    move_to(&mut tool, scripted(-200.0, 201.0));
    assert!(
        tool.frame()
            .gizmos
            .line_classes
            .iter()
            .all(|class| !matches!(
                class,
                GizmoLineClass::ZoningOutside | GizmoLineClass::ZoningInside
            ))
    );
}

// An alignment guide runs from its source through the snapped point and on past it, on the side
// the cursor is.
#[test]
fn alignment_guide_runs_from_its_source() {
    let mut gizmos = crate::nodes::sim::road_tool::frame::Gizmos::default();
    let guide = GuideLine {
        origin: d(10.0, 0.0),
        dir: DVec2::X,
    };
    push_alignment_guide(&mut gizmos, guide, d(-30.0, 0.0), 1.0);
    assert_eq!(gizmos.line_points[0], Vector3::new(10.0, 1.0, 0.0));
    assert_eq!(
        gizmos.line_points[1],
        Vector3::new((-30.0 - GUIDE_PAST_M) as f32, 1.0, 0.0)
    );
}
