// SPDX-License-Identifier: GPL-2.0-only

//! Junction-layout fixtures: arm order and rotation invariance, symmetry, exact corner endpoints,
//! mouth stations, fitting, degrees, angles, bends, terminals, mixed widths, curved arms and the
//! typed errors.

use super::*;

const TWO_LANE_HALF_M: f64 = 3.5;
const FOUR_LANE_HALF_M: f64 = 7.0;
const LONG_ARM_M: f64 = 60.0;
const KNOT_SPACING_M: f64 = 6.0;

#[derive(Clone)]
struct OwnedArm {
    key: ArmKey,
    knots: Vec<ArmKnot>,
    left: Vec<ArmBand>,
    right: Vec<ArmBand>,
    max_station_m: f64,
    crossable: bool,
}

impl OwnedArm {
    fn input(&self) -> ArmInput<'_> {
        ArmInput {
            key: self.key,
            knots: &self.knots,
            left: ArmSide {
                bands: &self.left,
                core_bands: 1,
            },
            right: ArmSide {
                bands: &self.right,
                core_bands: 1,
            },
            max_station_m: self.max_station_m,
            crossable: self.crossable,
        }
    }
}

fn key(edge: usize) -> ArmKey {
    ArmKey {
        edge,
        end: EdgeEnd::Start,
    }
}

fn band(kind: BandKind, level: Level, outer_m: f64) -> ArmBand {
    ArmBand {
        kind,
        level,
        outer_m,
    }
}

fn sidewalk_bands(half_m: f64) -> Vec<ArmBand> {
    vec![
        band(BandKind::Carriageway, Level::Ground, half_m),
        band(BandKind::CurbOrShoulder, Level::Raised, half_m + 0.15),
        band(BandKind::Sidewalk, Level::Raised, half_m + 1.5),
    ]
}

fn shoulder_bands(half_m: f64) -> Vec<ArmBand> {
    vec![
        band(BandKind::Carriageway, Level::Ground, half_m),
        band(BandKind::CurbOrShoulder, Level::Raised, half_m + 0.75),
    ]
}

fn unit(angle_rad: f64) -> RoadVec2 {
    RoadVec2::new(angle_rad.cos(), angle_rad.sin())
}

fn straight_knots(node: RoadVec2, angle_rad: f64, length_m: f64) -> Vec<ArmKnot> {
    let direction = unit(angle_rad);
    let count = (length_m / KNOT_SPACING_M).ceil() as usize;
    (0..=count)
        .map(|index| {
            let station_m = (index as f64 * KNOT_SPACING_M).min(length_m);
            ArmKnot {
                position: if index == 0 {
                    node
                } else {
                    node + direction * station_m
                },
                lateral: direction.perp(),
                station_m,
            }
        })
        .collect()
}

// Circular centreline leaving `node` at `angle_rad`, turning left for a positive radius.
fn arc_knots(node: RoadVec2, angle_rad: f64, radius_m: f64, length_m: f64) -> Vec<ArmKnot> {
    let (along, left) = (unit(angle_rad), unit(angle_rad).perp());
    let count = (length_m / 2.0).ceil() as usize;
    let mut station_m = 0.0;
    let mut knots: Vec<ArmKnot> = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let turn = (index as f64 * 2.0).min(length_m) / radius_m;
        let position = if index == 0 {
            node
        } else {
            node + along * (radius_m * turn.sin()) + left * (radius_m * (1.0 - turn.cos()))
        };
        if let Some(previous) = knots.last() {
            station_m += (position - previous.position).length();
        }
        let tangent = along * turn.cos() + left * turn.sin();
        knots.push(ArmKnot {
            position,
            lateral: tangent.perp(),
            station_m,
        });
    }
    knots
}

fn road_arm(edge: usize, node: RoadVec2, angle_deg: f64, half_m: f64) -> OwnedArm {
    OwnedArm {
        key: key(edge),
        knots: straight_knots(node, angle_deg.to_radians(), LONG_ARM_M),
        left: sidewalk_bands(half_m),
        right: sidewalk_bands(half_m),
        max_station_m: LONG_ARM_M,
        crossable: true,
    }
}

fn equal_arms(node: RoadVec2, angles_deg: &[f64]) -> Vec<OwnedArm> {
    angles_deg
        .iter()
        .enumerate()
        .map(|(edge, &angle)| road_arm(edge, node, angle, TWO_LANE_HALF_M))
        .collect()
}

fn build(node: RoadVec2, arms: &[OwnedArm]) -> Result<JunctionLayout, LayoutError> {
    let inputs: Vec<_> = arms.iter().map(OwnedArm::input).collect();
    JunctionLayout::build(node, &inputs)
}

fn build_checked(node: RoadVec2, arms: &[OwnedArm]) -> JunctionLayout {
    let layout = build(node, arms).expect("fixture lays out");
    assert_invariants(&layout, arms);
    layout
}

fn input_of<'a>(layout: &JunctionLayout, arms: &'a [OwnedArm], sorted: usize) -> ArmInput<'a> {
    arms[layout.arms[sorted].input_index].input()
}

// Exact endpoint copies, sample counts, mouth limits and core-loop provenance.
fn assert_invariants(layout: &JunctionLayout, arms: &[OwnedArm]) {
    for sector in &layout.sectors {
        let a = input_of(layout, arms, sector.from_arm);
        let b = input_of(layout, arms, sector.to_arm);
        for boundary in &layout.boundaries[sector.boundaries.clone()] {
            let samples = &layout.samples[boundary.samples.clone()];
            assert_eq!(samples.len(), sector.samples_per_boundary);
            let start = a.offset_point(boundary.from_station_m, boundary.from_offset_m);
            let end = if is_shared_miter(sector.kind, boundary) {
                start
            } else {
                b.offset_point(boundary.to_station_m, boundary.to_offset_m)
            };
            assert_eq!(samples[0], start, "boundary starts on its offset curve");
            assert_eq!(
                samples[samples.len() - 1],
                end,
                "boundary ends on its offset curve"
            );
        }
        if let Some(corner) = sector.corner {
            assert!(corner.radius_m <= CURB_RETURN_RADIUS_M);
            assert!(corner.radius_m >= 1.5 + CURB_RETURN_CLEARANCE_M - 1e-12);
        }
    }
    for (index, arm) in layout.arms.iter().enumerate() {
        let input = input_of(layout, arms, index);
        assert!(arm.mouth_station_m >= MIN_MOUTH_STATION_M);
        assert!(arm.mouth_station_m <= input.max_station_m);
    }
    let core = &layout.core;
    assert!(signed_area(core) > 0.0, "core loop runs counter-clockwise");
    // An arm side's carriageway edge runs on its sector's first boundary.
    let edge_of = |arm: usize, side: Side| {
        let count = layout.arms.len();
        let sector = match side {
            Side::Left => &layout.sectors[arm],
            Side::Right => &layout.sectors[(arm + count - 1) % count],
        };
        &layout.boundaries[sector.boundaries.start]
    };
    for (index, vertex) in core.iter().enumerate() {
        assert_ne!(vertex.position, core[(index + 1) % core.len()].position);
        let expected = match vertex.source {
            CoreVertexSource::Sample(sample) => layout.samples[sample],
            CoreVertexSource::Knot { arm, side, knot } => {
                let input = input_of(layout, arms, arm);
                edge_of(arm, side).run_point(&input, side, input.knots[knot].station_m)
            }
            CoreVertexSource::TaperEnd { arm, side } => {
                let input = input_of(layout, arms, arm);
                let edge = edge_of(arm, side);
                let taper = edge.taper.expect("a taper end lies on a tapered edge");
                edge.run_point(&input, side, taper.station_m)
            }
            CoreVertexSource::Mouth { arm, side } => {
                let input = input_of(layout, arms, arm);
                input.offset_point(layout.arms[arm].mouth_station_m, core_offset(&input, side))
            }
        };
        assert_eq!(vertex.position, expected, "core vertex copies its source");
    }
}

fn core_offset(arm: &ArmInput<'_>, side: Side) -> f64 {
    match side {
        Side::Left => arm.left.core_offset(),
        Side::Right => -arm.right.core_offset(),
    }
}

fn arm_by_key(layout: &JunctionLayout, edge: usize) -> &ArmLayout {
    layout
        .arms
        .iter()
        .find(|arm| arm.key == key(edge))
        .expect("arm present")
}

fn sector_from(layout: &JunctionLayout, edge: usize) -> &SectorLayout {
    layout
        .sectors
        .iter()
        .find(|sector| layout.arms[sector.from_arm].key == key(edge))
        .expect("sector present")
}

fn boundary_samples<'a>(layout: &'a JunctionLayout, boundary: &SectorBoundary) -> &'a [RoadVec2] {
    &layout.samples[boundary.samples.clone()]
}

fn assert_near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} differs from {expected} by more than {tolerance}"
    );
}

fn assert_near_point(actual: RoadVec2, expected: RoadVec2, tolerance: f64) {
    assert!(
        actual.distance(expected) <= tolerance,
        "{actual} differs from {expected} by more than {tolerance}"
    );
}

// Four-way with irregular angles and mixed widths.
fn irregular_four_way(node: RoadVec2) -> Vec<OwnedArm> {
    vec![
        road_arm(4, node, 0.0, TWO_LANE_HALF_M),
        road_arm(1, node, 80.0, FOUR_LANE_HALF_M),
        road_arm(7, node, 175.0, TWO_LANE_HALF_M),
        road_arm(2, node, 260.0, FOUR_LANE_HALF_M),
    ]
}

#[test]
fn input_order_does_not_change_the_layout() {
    let node = RoadVec2::new(812.25, -431.5);
    let arms = irregular_four_way(node);
    let normalized = |mut layout: JunctionLayout| {
        layout.arms.iter_mut().for_each(|arm| arm.input_index = 0);
        layout
    };
    let reference = normalized(build_checked(node, &arms));
    for order in [[3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]] {
        let permuted: Vec<_> = order.iter().map(|&index| arms[index].clone()).collect();
        assert_eq!(normalized(build_checked(node, &permuted)), reference);
    }
}

#[test]
fn rotating_the_junction_rotates_the_layout() {
    let rotation = 37.5_f64.to_radians();
    let offset = RoadVec2::new(1234.5, -876.25);
    let transform = |point: RoadVec2| RoadVec2::from_angle(rotation).rotate(point) + offset;
    let arms = irregular_four_way(RoadVec2::ZERO);
    let rotated: Vec<_> = arms
        .iter()
        .map(|arm| OwnedArm {
            knots: arm
                .knots
                .iter()
                .map(|knot| ArmKnot {
                    position: transform(knot.position),
                    lateral: RoadVec2::from_angle(rotation).rotate(knot.lateral),
                    station_m: knot.station_m,
                })
                .collect(),
            ..arm.clone()
        })
        .collect();
    let base = build_checked(RoadVec2::ZERO, &arms);
    let turned = build_checked(offset, &rotated);
    for arm in &base.arms {
        let other = arm_by_key(&turned, arm.key.edge);
        assert_near(other.mouth_station_m, arm.mouth_station_m, 1e-9);
        let radius = |corner: Option<CornerTangent>| corner.map_or(0.0, |tangent| tangent.radius_m);
        assert_near(radius(other.left_corner), radius(arm.left_corner), 1e-9);
        let sector = sector_from(&base, arm.key.edge);
        let other_sector = sector_from(&turned, arm.key.edge);
        assert_eq!(other_sector.kind, sector.kind);
        for (boundary, other_boundary) in base.boundaries[sector.boundaries.clone()]
            .iter()
            .zip(&turned.boundaries[other_sector.boundaries.clone()])
        {
            let samples = boundary_samples(&base, boundary);
            let other_samples = boundary_samples(&turned, other_boundary);
            assert_eq!(samples.len(), other_samples.len());
            for (&sample, &other_sample) in samples.iter().zip(other_samples) {
                assert_near_point(other_sample, transform(sample), 1e-9);
            }
        }
    }
}

#[test]
fn symmetric_junctions_have_mirrored_layouts() {
    let cross = build_checked(
        RoadVec2::ZERO,
        &equal_arms(RoadVec2::ZERO, &[0.0, 90.0, 180.0, 270.0]),
    );
    let first_mouth = cross.arms[0].mouth_station_m;
    for arm in &cross.arms {
        assert_near(arm.mouth_station_m, first_mouth, 1e-12);
    }

    // A Y mirrored about the x axis: arm 0 at 0 degrees, arms 1 and 2 at +-150 degrees.
    let y = build_checked(
        RoadVec2::ZERO,
        &equal_arms(RoadVec2::ZERO, &[0.0, 150.0, 210.0]),
    );
    assert_near(
        arm_by_key(&y, 1).mouth_station_m,
        arm_by_key(&y, 2).mouth_station_m,
        1e-9,
    );
    let mirror = |point: RoadVec2| RoadVec2::new(point.x, -point.y);
    let upper = sector_from(&y, 0);
    let lower = sector_from(&y, 2);
    for (boundary, mirrored) in y.boundaries[upper.boundaries.clone()]
        .iter()
        .zip(&y.boundaries[lower.boundaries.clone()])
    {
        let samples = boundary_samples(&y, boundary);
        let reversed = boundary_samples(&y, mirrored);
        assert_eq!(samples.len(), reversed.len());
        for (index, &sample) in samples.iter().enumerate() {
            assert_near_point(mirror(sample), reversed[reversed.len() - 1 - index], 1e-9);
        }
    }
}

#[test]
fn corner_endpoints_are_the_offset_curve_points() {
    let node = RoadVec2::new(-2048.75, 3071.125);
    let arms = irregular_four_way(node);
    let layout = build_checked(node, &arms);
    for sector in &layout.sectors {
        assert_eq!(sector.kind, SectorKind::Concave);
        let a = input_of(&layout, &arms, sector.from_arm);
        let b = input_of(&layout, &arms, sector.to_arm);
        let tangent_a = layout.arms[sector.from_arm].left_corner.unwrap().station_m;
        let tangent_b = layout.arms[sector.to_arm].right_corner.unwrap().station_m;
        for boundary in &layout.boundaries[sector.boundaries.clone()] {
            assert_eq!(boundary.from_station_m, tangent_a);
            assert_eq!(boundary.to_station_m, tangent_b);
            let samples = boundary_samples(&layout, boundary);
            assert_eq!(
                samples[0],
                a.offset_point(tangent_a, boundary.from_offset_m)
            );
            assert_eq!(
                samples[samples.len() - 1],
                b.offset_point(tangent_b, boundary.to_offset_m)
            );
        }
    }
}

#[test]
fn straight_equal_arms_make_concentric_arcs() {
    let node = RoadVec2::new(500.0, 250.0);
    let layout = build_checked(node, &equal_arms(node, &[0.0, 90.0, 180.0, 270.0]));
    for sector in &layout.sectors {
        let corner = sector.corner.expect("a cross has four corners");
        assert_eq!(corner.radius_m, CURB_RETURN_RADIUS_M);
        for boundary in &layout.boundaries[sector.boundaries.clone()] {
            let samples = boundary_samples(&layout, boundary);
            let radius = samples[0].distance(corner.centre);
            for sample in samples {
                assert_near(sample.distance(corner.centre), radius, 1e-9);
            }
        }
        let edge = &layout.boundaries[sector.boundaries.start];
        let radius = boundary_samples(&layout, edge)[0].distance(corner.centre);
        assert_near(radius, CURB_RETURN_RADIUS_M, 1e-9);
    }
}

#[test]
fn crossable_mouths_extend_three_metres_past_the_corner() {
    let node = RoadVec2::new(64.0, 32.0);
    let crossable = equal_arms(node, &[0.0, 90.0, 180.0, 270.0]);
    let plain: Vec<_> = crossable
        .iter()
        .map(|arm| OwnedArm {
            crossable: false,
            ..arm.clone()
        })
        .collect();
    let with = build_checked(node, &crossable);
    let without = build_checked(node, &plain);
    for (arm, plain_arm) in with.arms.iter().zip(&without.arms) {
        let tangent = arm.left_corner.unwrap().station_m;
        assert_near(tangent, TWO_LANE_HALF_M + CURB_RETURN_RADIUS_M, 1e-9);
        assert_eq!(plain_arm.mouth_station_m, tangent);
        assert_eq!(arm.mouth_station_m, tangent + CROSSWALK_DEPTH_M);
        let crosswalk = arm.crosswalk.unwrap();
        assert_eq!(crosswalk.inner_station_m, tangent);
        assert_eq!(crosswalk.station_m, tangent + 1.5);
        let input = crossable.iter().find(|input| input.key == arm.key).unwrap();
        let input = input.input();
        assert_eq!(
            crosswalk.kerbs,
            [
                input.offset_point(crosswalk.station_m, -TWO_LANE_HALF_M),
                input.offset_point(crosswalk.station_m, TWO_LANE_HALF_M),
            ]
        );
        assert_eq!(plain_arm.crosswalk, None);
    }
}

#[test]
fn arms_without_corners_use_the_minimum_mouth_station() {
    let node = RoadVec2::new(10.0, 20.0);
    let mut arm = road_arm(0, node, 30.0, TWO_LANE_HALF_M);
    let crossable = build_checked(node, std::slice::from_ref(&arm));
    assert_eq!(
        crossable.arms[0].mouth_station_m,
        MIN_MOUTH_STATION_M + CROSSWALK_DEPTH_M
    );
    arm.crossable = false;
    let plain = build_checked(node, &[arm]);
    assert_eq!(plain.arms[0].mouth_station_m, MIN_MOUTH_STATION_M);
}

#[test]
fn short_arms_shrink_the_corner_radius() {
    // A straight cross puts every corner tangent at `la + R`, so a 9.5 m limit with a 3 m
    // crosswalk leaves room for R = 3 m exactly.
    let node = RoadVec2::new(-300.0, 120.0);
    let mut arms = equal_arms(node, &[0.0, 90.0, 180.0, 270.0]);
    arms.iter_mut().for_each(|arm| arm.max_station_m = 9.5);
    let layout = build_checked(node, &arms);
    for arm in &layout.arms {
        let corner = arm.left_corner.unwrap();
        assert!(corner.radius_m < CURB_RETURN_RADIUS_M);
        assert_near(corner.radius_m, 3.0, 1e-5);
        assert!(arm.mouth_station_m <= 9.5);
        assert_near(arm.mouth_station_m, 9.5, 1e-5);
    }
}

#[test]
fn corners_that_cannot_shrink_enough_do_not_fit() {
    // R_min is the 1.5 m raised width plus 0.5 m, so the tangent needs at least 5.5 m.
    let node = RoadVec2::ZERO;
    let mut arms = equal_arms(node, &[0.0, 90.0, 180.0, 270.0]);
    arms[2].max_station_m = CROSSWALK_DEPTH_M + 5.4;
    let result = build(node, &arms);
    assert!(
        matches!(result, Err(LayoutError::CornerDoesNotFit { arm }) if arm == key(1) || arm == key(2)),
        "{result:?}"
    );
}

#[test]
fn a_crossing_core_loop_is_rejected() {
    // A terminal arm that doubles back inside its node extent: its edges cross the cap.
    let node = RoadVec2::new(5.0, 5.0);
    let back = unit(170.0_f64.to_radians());
    let positions = [node, node + RoadVec2::X, node + RoadVec2::X + back * 3.0];
    let secant = (positions[2] - positions[0]).normalize();
    let knots = vec![
        ArmKnot {
            position: positions[0],
            lateral: RoadVec2::Y,
            station_m: 0.0,
        },
        ArmKnot {
            position: positions[1],
            lateral: secant.perp(),
            station_m: 1.0,
        },
        ArmKnot {
            position: positions[2],
            lateral: back.perp(),
            station_m: 4.0,
        },
    ];
    let arm = OwnedArm {
        key: key(0),
        knots,
        left: sidewalk_bands(TWO_LANE_HALF_M),
        right: sidewalk_bands(TWO_LANE_HALF_M),
        max_station_m: 4.0,
        crossable: true,
    };
    assert_eq!(build(node, &[arm]), Err(LayoutError::CoreNotSimple));
}

#[test]
fn degrees_one_to_six_lay_out() {
    let node = RoadVec2::new(1500.0, -1500.0);
    for degree in 1..=6 {
        let angles: Vec<_> = (0..degree)
            .map(|index| 7.0 + index as f64 * 360.0 / degree as f64)
            .collect();
        let layout = build_checked(node, &equal_arms(node, &angles));
        let expected = match degree {
            1 => NodeKind::Terminal,
            2 => NodeKind::Bend,
            _ => NodeKind::Junction,
        };
        assert_eq!(layout.kind, expected);
        assert_eq!(layout.sectors.len(), degree);
        let expected_kind = match degree {
            1 => SectorKind::Convex,
            2 => SectorKind::Straight,
            _ => SectorKind::Concave,
        };
        assert!(
            layout
                .sectors
                .iter()
                .all(|sector| sector.kind == expected_kind)
        );
    }
}

#[test]
fn arm_angles_from_25_to_180_degrees_lay_out() {
    let node = RoadVec2::new(-77.0, 91.0);
    let mut previous_tangent = f64::INFINITY;
    for angle in [25.0, 40.0, 60.0, 90.0, 180.0] {
        let arms = equal_arms(node, &[0.0, angle, 180.0 + angle / 2.0]);
        let layout = build_checked(node, &arms);
        let sector = sector_from(&layout, 0);
        if angle < 180.0 {
            assert_eq!(sector.kind, SectorKind::Concave);
            let from = arm_by_key(&layout, 0).left_corner.unwrap().station_m;
            let to = arm_by_key(&layout, 1).right_corner.unwrap().station_m;
            assert_near(from, to, 1e-9);
            assert!(from < previous_tangent, "wider arms need shorter corners");
            previous_tangent = from;
        } else {
            assert_eq!(sector.kind, SectorKind::Straight);
        }
    }
}

#[test]
fn bends_from_10_to_170_degrees_lay_out() {
    let node = RoadVec2::new(333.0, 444.0);
    for angle in (10..=170).step_by(10).map(f64::from) {
        let arms: Vec<_> = [0.0, angle]
            .iter()
            .enumerate()
            .map(|(edge, &direction)| OwnedArm {
                knots: straight_knots(node, f64::to_radians(direction), 150.0),
                max_station_m: 150.0,
                crossable: false,
                ..road_arm(edge, node, direction, TWO_LANE_HALF_M)
            })
            .collect();
        let layout = build_checked(node, &arms);
        assert_eq!(layout.kind, NodeKind::Bend);
        assert_eq!(sector_from(&layout, 0).kind, SectorKind::Concave);
        assert_eq!(sector_from(&layout, 1).kind, SectorKind::Convex);
        // A straight bend's curb return touches both arms at (la + R) / tan(phi / 2).
        let expected = (TWO_LANE_HALF_M + CURB_RETURN_RADIUS_M) / (angle.to_radians() / 2.0).tan();
        let tangent = arm_by_key(&layout, 0).left_corner.unwrap().station_m;
        assert_near(tangent, expected, 1e-9);
    }
}

#[test]
fn a_terminal_has_a_round_cap() {
    let node = RoadVec2::new(42.0, -17.0);
    let arm = road_arm(3, node, 63.0, TWO_LANE_HALF_M);
    let layout = build_checked(node, std::slice::from_ref(&arm));
    assert_eq!(layout.kind, NodeKind::Terminal);
    let cap = &layout.sectors[0];
    assert_eq!((cap.kind, cap.angle_rad), (SectorKind::Convex, TAU));
    let behind = node - unit(63.0_f64.to_radians());
    for (boundary, outer_band) in layout.boundaries[cap.boundaries.clone()]
        .iter()
        .zip(&arm.left)
    {
        let samples = boundary_samples(&layout, boundary);
        for sample in samples {
            assert_near(sample.distance(node), outer_band.outer_m, 1e-9);
        }
        // The middle sample lies straight behind the arm.
        let middle = samples[samples.len() / 2];
        assert_near_point(
            node + (behind - node) * outer_band.outer_m,
            middle,
            outer_band.outer_m * (PI / (samples.len() - 1) as f64),
        );
    }
}

#[test]
fn two_lane_meets_four_lane_with_tapered_bands() {
    // Two-lane arms with sidewalks east and west, four-lane arms with shoulders north and south.
    let node = RoadVec2::new(900.0, 900.0);
    let arms: Vec<_> = [0.0, 90.0, 180.0, 270.0]
        .iter()
        .enumerate()
        .map(|(edge, &angle)| {
            let mut arm = road_arm(edge, node, angle, TWO_LANE_HALF_M);
            if edge % 2 == 1 {
                arm.left = shoulder_bands(FOUR_LANE_HALF_M);
                arm.right = shoulder_bands(FOUR_LANE_HALF_M);
            }
            arm
        })
        .collect();
    let layout = build_checked(node, &arms);
    let sector = sector_from(&layout, 0);
    let corner = sector.corner.unwrap();
    let boundaries = &layout.boundaries[sector.boundaries.clone()];
    assert_eq!(boundaries.len(), 3);
    // The sidewalk exists only on the east arm: its outer boundary meets the shoulder's edge.
    assert_eq!(boundaries[2].to_offset_m, boundaries[1].to_offset_m);
    let last = |boundary: &SectorBoundary| *boundary_samples(&layout, boundary).last().unwrap();
    assert_eq!(last(&boundaries[2]), last(&boundaries[1]));
    let outer = boundary_samples(&layout, &boundaries[2]);
    let (start, end) = (
        outer[0].distance(corner.centre),
        last(&boundaries[2]).distance(corner.centre),
    );
    assert_near(start, corner.radius_m - 1.5, 1e-9);
    assert_near(end, corner.radius_m - 0.75, 1e-9);
    assert_eq!(
        boundaries[2].inner,
        BoundaryBand {
            kind: BandKind::Sidewalk,
            level: Level::Raised
        }
    );
    assert_eq!(boundaries[2].outer, None);
}

// Open question 6, defaulted to the backward extension: an obtuse sector between a two-lane and
// a four-lane arm has its corner centre on the four-lane arm's curb-offset line just behind the
// node, so its tangent station there is negative.
fn assert_backward_corner(layout: &JunctionLayout, sector: &SectorLayout) {
    let corner = sector.corner.expect("an obtuse unequal corner fits");
    let boundary = &layout.boundaries[sector.boundaries.start];
    assert!(
        boundary.from_station_m.min(boundary.to_station_m) < 0.0,
        "the corner touches the wider arm behind its station zero"
    );
    let samples = boundary_samples(layout, boundary);
    for sample in samples {
        assert_near(sample.distance(corner.centre), corner.radius_m, 1e-6);
    }
}

#[test]
fn obtuse_corners_between_unequal_widths_extend_the_wider_arm_backwards() {
    let node = RoadVec2::new(-5.0, 7.0);
    for concave_deg in [120.0, 130.0, 140.0, 150.0, 160.0] {
        for wide_edge in [0, 1] {
            let half = |edge: usize| {
                if edge == wide_edge {
                    FOUR_LANE_HALF_M
                } else {
                    TWO_LANE_HALF_M
                }
            };
            let arms = vec![
                road_arm(0, node, 0.0, half(0)),
                road_arm(1, node, concave_deg, half(1)),
                road_arm(2, node, 270.0, TWO_LANE_HALF_M),
            ];
            let layout = build_checked(node, &arms);
            let sector = sector_from(&layout, 0);
            assert_eq!(sector.kind, SectorKind::Concave);
            let boundary = &layout.boundaries[sector.boundaries.start];
            let backward = boundary.from_station_m.min(boundary.to_station_m) < 0.0;
            // From about 137 degrees no crossing exists at positive stations.
            if concave_deg >= 140.0 {
                assert_backward_corner(&layout, sector);
            } else {
                assert!(!backward, "{concave_deg} deg crosses forward");
            }
        }
    }
}

#[test]
fn a_two_lane_side_road_meets_a_four_lane_through_road() {
    let node = RoadVec2::new(310.0, -42.0);
    for side_deg in [40.0, 60.0, 90.0] {
        let arms = vec![
            road_arm(0, node, 0.0, FOUR_LANE_HALF_M),
            road_arm(1, node, 180.0, FOUR_LANE_HALF_M),
            road_arm(2, node, 180.0 - side_deg, TWO_LANE_HALF_M),
        ];
        let layout = build_checked(node, &arms);
        assert_eq!(layout.kind, NodeKind::Junction);
        let obtuse = sector_from(&layout, 0);
        assert_eq!(obtuse.kind, SectorKind::Concave);
        if 180.0 - side_deg >= 140.0 {
            assert_backward_corner(&layout, obtuse);
        }
        assert_eq!(sector_from(&layout, 2).kind, SectorKind::Concave);
        assert_eq!(sector_from(&layout, 1).kind, SectorKind::Straight);
    }
}

#[test]
fn a_straight_width_change_tapers_along_both_approaches() {
    // A two-lane road continues west as a four-lane road, with a side road to the south.
    let node = RoadVec2::new(12.0, 34.0);
    let arms = vec![
        road_arm(0, node, 0.0, TWO_LANE_HALF_M),
        road_arm(1, node, 180.0, FOUR_LANE_HALF_M),
        road_arm(2, node, 270.0, TWO_LANE_HALF_M),
    ];
    let layout = build_checked(node, &arms);
    let sector = sector_from(&layout, 0);
    assert_eq!(sector.kind, SectorKind::Straight);
    // The legacy taper for a 7 m width change: 21 m, half on each approach.
    let taper_m = 10.5;
    let (east, west) = (&arms[0].input(), &arms[1].input());
    for boundary in &layout.boundaries[sector.boundaries.clone()] {
        let taper = boundary.taper.expect("every boundary changes offset");
        assert_eq!(taper.station_m, taper_m);
        // The join is that of equal widths: one point on the node's lateral line.
        assert_eq!(boundary.from_offset_m, -boundary.to_offset_m);
        let samples = boundary_samples(&layout, boundary);
        assert!(samples.iter().all(|&sample| sample == samples[0]));
        let mean_m = 0.5 * (taper.from_offset_m - taper.to_offset_m);
        assert_near_point(samples[0], node + RoadVec2::Y * mean_m, 1e-12);
        // Linear in station on both approaches, to the arms' own offsets.
        for (arm, side, own_m) in [
            (east, Side::Left, taper.from_offset_m),
            (west, Side::Right, taper.to_offset_m),
        ] {
            assert_eq!(boundary.run_offset_m(side, taper_m), own_m);
            assert_eq!(boundary.run_offset_m(side, 40.0), own_m);
            let half = boundary.run_offset_m(side, 0.5 * taper_m);
            let start = boundary.run_offset_m(side, 0.0);
            assert_near(half, 0.5 * (start + own_m), 1e-12);
            assert_eq!(
                boundary.run_point(arm, side, taper_m),
                arm.offset_point(taper_m, own_m)
            );
        }
    }
    // Both mouths lie beyond the taper, so the spans start at full width.
    for edge in [0, 1] {
        assert!(arm_by_key(&layout, edge).mouth_station_m >= taper_m + CROSSWALK_DEPTH_M);
    }
    assert!(
        layout
            .core
            .iter()
            .any(|vertex| matches!(vertex.source, CoreVertexSource::TaperEnd { .. })),
        "the core loop follows the tapered carriageway edges"
    );
}

#[test]
fn a_width_change_taper_that_cannot_fit_is_rejected() {
    let node = RoadVec2::new(-3.0, 8.0);
    let mut arms = vec![
        road_arm(0, node, 0.0, TWO_LANE_HALF_M),
        road_arm(1, node, 180.0, FOUR_LANE_HALF_M),
        road_arm(2, node, 270.0, TWO_LANE_HALF_M),
    ];
    // Half the legacy minimum taper plus the crosswalk is 9 m.
    arms[1].max_station_m = 8.9;
    assert_eq!(
        build(node, &arms),
        Err(LayoutError::ArmTooShort { arm: key(1) })
    );
    arms[1].max_station_m = 9.0;
    build_checked(node, &arms);
}

#[test]
fn nearly_straight_sectors_keep_the_core_simple() {
    let node = RoadVec2::new(4096.5, -2048.25);
    for (deviation, west_half) in [
        (0.0, TWO_LANE_HALF_M),
        (1e-9, TWO_LANE_HALF_M),
        (-1e-9, TWO_LANE_HALF_M),
        (1e-7, TWO_LANE_HALF_M),
        (-1e-7, TWO_LANE_HALF_M),
        (1e-4, TWO_LANE_HALF_M),
        (-1e-4, TWO_LANE_HALF_M),
        (0.9_f64.to_radians(), TWO_LANE_HALF_M),
        (-0.9_f64.to_radians(), TWO_LANE_HALF_M),
        (0.5_f64.to_radians(), FOUR_LANE_HALF_M),
        (-0.5_f64.to_radians(), FOUR_LANE_HALF_M),
        (1e-6, FOUR_LANE_HALF_M),
    ] {
        let west = 180.0 - f64::to_degrees(deviation);
        let arms = vec![
            road_arm(0, node, 0.0, TWO_LANE_HALF_M),
            road_arm(1, node, west, west_half),
            road_arm(2, node, 270.0, TWO_LANE_HALF_M),
        ];
        let layout = build_checked(node, &arms);
        assert_eq!(sector_from(&layout, 0).kind, SectorKind::Straight);
    }
}

#[test]
fn curved_arms_lay_out() {
    let node = RoadVec2::new(-640.0, 1280.0);
    let arms: Vec<_> = [(0.0, 80.0), (95.0, -60.0), (180.0, 120.0), (265.0, -90.0)]
        .iter()
        .enumerate()
        .map(|(edge, &(angle, radius))| {
            let knots = arc_knots(node, f64::to_radians(angle), radius, LONG_ARM_M);
            OwnedArm {
                // The chords of a curve are shorter than its arc.
                max_station_m: knots[knots.len() - 1].station_m,
                knots,
                ..road_arm(edge, node, angle, TWO_LANE_HALF_M)
            }
        })
        .collect();
    let layout = build_checked(node, &arms);
    assert!(
        layout
            .sectors
            .iter()
            .all(|sector| sector.kind == SectorKind::Concave)
    );
    for arm in &layout.arms {
        let corner = arm.left_corner.unwrap();
        assert_eq!(corner.radius_m, CURB_RETURN_RADIUS_M);
        assert!(corner.station_m > 5.0 && corner.station_m < 15.0);
    }
}

// One edit that breaks an otherwise valid arm.
type ArmChange<'a> = &'a dyn Fn(&mut OwnedArm);

#[test]
fn degenerate_input_returns_typed_errors() {
    let node = RoadVec2::new(1.0, 2.0);
    let good = road_arm(0, node, 0.0, TWO_LANE_HALF_M);
    let other = road_arm(1, node, 120.0, TWO_LANE_HALF_M);
    let arm = key(0);
    let with = |change: &dyn Fn(&mut OwnedArm)| {
        let mut broken = good.clone();
        change(&mut broken);
        build(node, &[broken, other.clone()])
    };
    assert_eq!(build(node, &[]), Err(LayoutError::NoArms));
    assert_eq!(
        build(RoadVec2::new(f64::NAN, 0.0), std::slice::from_ref(&good)),
        Err(LayoutError::NonFiniteNode)
    );
    let cases: [(ArmChange, LayoutError); 14] = [
        (
            &|a| a.knots[3].position.x = f64::NAN,
            LayoutError::NonFinite { arm },
        ),
        (
            &|a| a.left[1].outer_m = f64::INFINITY,
            LayoutError::NonFinite { arm },
        ),
        (&|a| a.knots.truncate(1), LayoutError::DegenerateArm { arm }),
        (
            &|a| a.knots[1].position = node,
            LayoutError::DegenerateArm { arm },
        ),
        (
            &|a| a.knots[0].position.x += 0.01,
            LayoutError::DegenerateArm { arm },
        ),
        (
            &|a| a.knots[2].station_m = 1.0,
            LayoutError::DegenerateArm { arm },
        ),
        (
            &|a| a.knots[2].lateral *= 1.01,
            LayoutError::DegenerateArm { arm },
        ),
        (
            &|a| a.knots[2].lateral = -a.knots[2].lateral,
            LayoutError::DegenerateArm { arm },
        ),
        (
            &|a| a.max_station_m = LONG_ARM_M + 1.0,
            LayoutError::DegenerateArm { arm },
        ),
        (
            &|a| {
                a.knots = straight_knots(node, 0.0, 0.03);
                a.max_station_m = 0.03;
            },
            LayoutError::DegenerateArm { arm },
        ),
        (
            &|a| a.left[2].outer_m = 3.6,
            LayoutError::InvalidBands { arm },
        ),
        (
            &|a| a.key = key(1),
            LayoutError::DuplicateArm { arm: key(1) },
        ),
        (&|a| a.max_station_m = 3.5, LayoutError::ArmTooShort { arm }),
        (
            &|a| {
                a.left = vec![band(BandKind::Footpath, Level::Ground, 1.0)];
                a.right = a.left.clone();
            },
            LayoutError::BandMismatch { arm },
        ),
    ];
    for (change, expected) in cases {
        assert_eq!(with(change), Err(expected));
    }
    let mut no_core = good.input();
    no_core.left.core_bands = 0;
    assert_eq!(
        JunctionLayout::build(node, &[no_core]),
        Err(LayoutError::InvalidBands { arm })
    );
    let twin = OwnedArm {
        key: key(5),
        ..good.clone()
    };
    assert_eq!(
        build(node, &[good.clone(), twin]),
        Err(LayoutError::DuplicateDirection { arm: key(5) })
    );
}
