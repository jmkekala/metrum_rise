// SPDX-License-Identifier: GPL-2.0-only

//! Minimal graph fixtures and regression coverage for road geometry measurements.

use super::*;
use crate::simulation::network::types::{NodeType, TransitFlags};

fn add_edge(graph: &mut RegionGraph, start: u32, end: u32, points: Vec<Vector3>) -> usize {
    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0 / MS_TO_KMH,
        physical_length: points.windows(2).map(|p| p[0].distance_to(p[1])).sum(),
        geometry: points.clone(),
        physical_geometry: points,
        ..Default::default()
    })
}

fn single_edge(points: Vec<Vector3>) -> (RegionGraph, usize) {
    let mut graph = RegionGraph::new();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    let id = add_edge(&mut graph, start, end, points);
    (graph, id)
}

fn ramp(rise: f32) -> (RegionGraph, usize) {
    single_edge(vec![Vector3::ZERO, Vector3::new(100.0, rise, 0.0)])
}

fn junction(angles_degrees: &[f32]) -> (RegionGraph, u32) {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    for (index, &degrees) in angles_degrees.iter().enumerate() {
        let angle = degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 100.0, 0.0, angle.sin() * 100.0);
        let end = graph.add_node(endpoint, NodeType::Junction);
        // Exercise both stored orientations at the shared node.
        if index % 2 == 0 {
            add_edge(&mut graph, center, end, vec![Vector3::ZERO, endpoint]);
        } else {
            add_edge(&mut graph, end, center, vec![endpoint, Vector3::ZERO]);
        }
    }
    (graph, center)
}

fn edge_violations(graph: &RegionGraph, id: usize) -> Vec<RoadGeometryViolation> {
    let mut out = Vec::new();
    validate_edge(graph, id, &RoadGeometryLimits::default(), &mut out);
    out
}

fn node_violations(graph: &RegionGraph, id: u32) -> Vec<RoadGeometryViolation> {
    let mut out = Vec::new();
    validate_node(graph, id, &RoadGeometryLimits::default(), &mut out);
    out
}

#[test]
fn flat_straight_road_has_no_violations() {
    let (graph, _) = single_edge(vec![
        Vector3::ZERO,
        Vector3::new(20.0, 0.0, 0.0),
        Vector3::new(100.0, 0.0, 0.0),
    ]);
    assert!(audit_graph(&graph, &RoadGeometryLimits::default()).is_empty());
}

#[test]
fn thirty_percent_ramp_is_illegal_grade() {
    let (graph, id) = ramp(30.0);
    let out = edge_violations(&graph, id);
    assert!(matches!(out.as_slice(), [RoadGeometryViolation::Grade {
        edge_id, measured, limit: ILLEGAL_GRADE, tier: RoadGeometryTier::Illegal, ..
    }] if *edge_id == id && (*measured - 0.30).abs() < 1e-6));
}

#[test]
fn twelve_percent_standard_ramp_is_design_grade() {
    let (graph, id) = ramp(12.0);
    let out = edge_violations(&graph, id);
    assert!(matches!(out.as_slice(), [RoadGeometryViolation::Grade {
        measured, limit: STANDARD_DESIGN_GRADE, tier: RoadGeometryTier::Design, ..
    }] if (*measured - 0.12).abs() < 1e-6));
}

#[test]
fn hairpin_below_visual_half_width_is_illegal_radius() {
    let (graph, id) = single_edge(vec![
        Vector3::ZERO,
        Vector3::new(2.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 0.2),
    ]);
    let half_width = crate::simulation::network::roadbed::visual_roadbed_half_width_m(graph.edge(id));
    let out = edge_violations(&graph, id);
    assert!(
        matches!(out.as_slice(), [RoadGeometryViolation::HorizontalRadius {
        measured, limit, tier: RoadGeometryTier::Illegal, ..
    }] if *measured < half_width && (*limit - half_width * 1.25).abs() < 1e-6)
    );
}

#[test]
fn twenty_degree_kink_at_degree_two_is_illegal() {
    let (graph, center) = junction(&[180.0, 20.0]);
    let out = node_violations(&graph, center);
    assert!(
        matches!(out.as_slice(), [RoadGeometryViolation::DirectionKink {
        node_id, measured, limit: ILLEGAL_KINK_DEGREES, tier: RoadGeometryTier::Illegal,
    }] if *node_id == center && (*measured - 20.0).abs() < 1e-4)
    );
}

#[test]
fn four_arm_right_angle_junction_is_clean() {
    let (graph, _) = junction(&[0.0, 90.0, 180.0, 270.0]);
    assert!(audit_graph(&graph, &RoadGeometryLimits::default()).is_empty());
}

#[test]
fn ten_arm_junction_has_illegal_degree_and_arm_angle() {
    let (graph, center) = junction(&[
        0.0, 20.0, 55.0, 90.0, 125.0, 160.0, 200.0, 240.0, 280.0, 320.0,
    ]);
    let out = node_violations(&graph, center);
    assert_eq!(out.len(), 2);
    assert!(
        out.iter()
            .any(|v| matches!(v, RoadGeometryViolation::JunctionDegree {
        node_id, measured: 10, limit: ILLEGAL_JUNCTION_DEGREE, tier: RoadGeometryTier::Illegal,
    } if *node_id == center))
    );
    assert!(
        out.iter()
            .any(|v| matches!(v, RoadGeometryViolation::JunctionArmAngle {
        measured, limit: ILLEGAL_ARM_ANGLE_DEGREES, tier: RoadGeometryTier::Illegal, ..
    } if (*measured - 20.0).abs() < 1e-4))
    );
}

#[test]
fn deleted_edges_are_ignored_in_edge_and_node_checks() {
    let (mut graph, _) = junction(&[0.0, 90.0, 180.0, 270.0, 10.0]);
    let deleted = 4;
    graph.edge_mut(deleted).physical_geometry[1].y = 1000.0;
    graph.edge_mut(deleted).deleted = true;
    // Leave the stale adjacency entry to exercise the validator's own deleted filtering.
    assert!(edge_violations(&graph, deleted).is_empty());
    assert!(audit_graph(&graph, &RoadGeometryLimits::default()).is_empty());
}

#[test]
fn structural_classes_use_six_percent_design_grade() {
    for class in [EdgeClass::Bridge, EdgeClass::Tunnel] {
        let (mut graph, id) = ramp(8.0);
        graph.edge_mut(id).class = class;
        assert!(matches!(
            edge_violations(&graph, id).as_slice(),
            [RoadGeometryViolation::Grade {
                limit: STRUCTURAL_DESIGN_GRADE,
                tier: RoadGeometryTier::Design,
                ..
            }]
        ));
        graph.edge_mut(id).physical_geometry[1].y = 30.0;
        assert!(matches!(
            edge_violations(&graph, id).as_slice(),
            [RoadGeometryViolation::Grade {
                limit: ILLEGAL_GRADE,
                tier: RoadGeometryTier::Illegal,
                ..
            }]
        ));
    }
}

#[test]
fn signed_grade_change_uses_mean_horizontal_run_and_worst_station() {
    let (graph, id) = single_edge(vec![
        Vector3::ZERO,
        Vector3::new(2.0, 0.2, 0.0),
        Vector3::new(6.0, -0.2, 0.0),
        Vector3::new(16.0, 0.8, 0.0),
    ]);
    let out = edge_violations(&graph, id);
    let changes: Vec<_> = out
        .iter()
        .filter(|v| matches!(v, RoadGeometryViolation::GradeChange { .. }))
        .collect();
    assert_eq!(changes.len(), 1);
    assert!(matches!(changes[0], RoadGeometryViolation::GradeChange {
        measured, station_m, limit: ILLEGAL_GRADE_CHANGE_PER_M, tier: RoadGeometryTier::Illegal, ..
    } if (*measured - 0.2 / 3.0).abs() < 1e-6 && (*station_m - 2.0_f32.hypot(0.2)).abs() < 1e-6));
}

#[test]
fn grade_change_between_design_and_illegal_is_design() {
    let (graph, id) = single_edge(vec![
        Vector3::ZERO,
        Vector3::new(2.0, 0.08, 0.0),
        Vector3::new(4.0, 0.0, 0.0),
    ]);
    assert!(
        matches!(edge_violations(&graph, id).as_slice(), [RoadGeometryViolation::GradeChange {
        measured, limit: DESIGN_GRADE_CHANGE_PER_M, tier: RoadGeometryTier::Design, ..
    }] if (*measured - 0.04).abs() < 1e-6)
    );
}

#[test]
fn radius_design_limit_converts_metres_per_second_to_kmh() {
    let (mut graph, id) = single_edge(vec![
        Vector3::ZERO,
        Vector3::new(20.0, 0.0, 0.0),
        Vector3::new(20.0, 0.0, 20.0),
    ]);
    let out = edge_violations(&graph, id);
    assert!(
        matches!(out.as_slice(), [RoadGeometryViolation::HorizontalRadius {
        measured, limit, station_m: 20.0, tier: RoadGeometryTier::Design, ..
    }] if (*measured - 20.0 / std::f32::consts::FRAC_PI_2).abs() < 1e-5
        && (*limit - 2500.0 / (127.0 * 0.20)).abs() < 1e-4)
    );
    graph.edge_mut(id).speed_limit = 10.0 / MS_TO_KMH;
    assert!(edge_violations(&graph, id).is_empty());
}

#[test]
fn physical_profile_is_checked_with_empty_profile_control_fallback() {
    let (mut graph, id) = ramp(30.0);
    graph.edge_mut(id).geometry[1].y = 0.0;
    assert!(matches!(
        edge_violations(&graph, id).as_slice(),
        [RoadGeometryViolation::Grade {
            tier: RoadGeometryTier::Illegal,
            ..
        }]
    ));
    graph.edge_mut(id).physical_geometry.clear();
    assert!(edge_violations(&graph, id).is_empty());
    graph.edge_mut(id).geometry[1].y = 30.0;
    assert!(matches!(
        edge_violations(&graph, id).as_slice(),
        [RoadGeometryViolation::Grade { .. }]
    ));
}

#[test]
fn short_segments_are_skipped_and_break_adjacent_comparisons() {
    let (graph, id) = single_edge(vec![
        Vector3::ZERO,
        Vector3::new(10.0, 0.0, 0.0),
        Vector3::new(10.01, 100.0, 0.0),
        Vector3::new(0.01, 100.0, 0.0),
    ]);
    assert!(edge_violations(&graph, id).is_empty());
    let (graph, id) = single_edge(vec![Vector3::ZERO, Vector3::new(0.05, 0.015, 0.0)]);
    assert!(matches!(
        edge_violations(&graph, id).as_slice(),
        [RoadGeometryViolation::Grade {
            tier: RoadGeometryTier::Illegal,
            ..
        }]
    ));
}

#[test]
fn worst_absolute_grade_is_reported_once_at_its_midpoint() {
    let (graph, id) = single_edge(vec![
        Vector3::ZERO,
        Vector3::new(10.0, 2.0, 0.0),
        Vector3::new(20.0, -2.0, 0.0),
        Vector3::new(30.0, -6.0, 0.0),
    ]);
    let out = edge_violations(&graph, id);
    let grades: Vec<_> = out
        .iter()
        .filter(|v| matches!(v, RoadGeometryViolation::Grade { .. }))
        .collect();
    assert_eq!(grades.len(), 1);
    assert!(
        matches!(grades[0], RoadGeometryViolation::Grade { measured, station_m, .. }
        if (*measured - 0.4).abs() < 1e-6
        && (*station_m - (10.0_f32.hypot(2.0) + 0.5 * 10.0_f32.hypot(4.0))).abs() < 1e-5)
    );
}

#[test]
fn arm_direction_averages_tiny_end_segments_in_both_orientations_without_clips() {
    let (mut graph, center) = junction(&[0.0, 180.0]);
    let tiny = Vector3::new(0.0, 0.0, 0.01);
    graph.edge_mut(0).physical_geometry.insert(1, tiny);
    graph.edge_mut(1).physical_geometry.insert(1, tiny);
    for id in [0, 1] {
        graph.edge_mut(id).start_clip = 90.0;
        graph.edge_mut(id).end_clip = 90.0;
    }
    assert!(node_violations(&graph, center).is_empty());
    // The east edge bends after the one-metre sample; a clipped-mouth tangent would
    // see north instead, but the validator measures continuity at the shared node.
    graph.edge_mut(0).physical_geometry = vec![
        Vector3::ZERO,
        Vector3::new(4.0, 0.0, 0.0),
        Vector3::new(4.0, 0.0, 100.0),
    ];
    assert!(node_violations(&graph, center).is_empty());
}

#[test]
fn junction_angles_include_wraparound_and_design_tiers() {
    let (graph, center) = junction(&[10.0, 120.0, 240.0, 350.0]);
    assert!(
        matches!(node_violations(&graph, center).as_slice(), [RoadGeometryViolation::JunctionArmAngle {
        measured, tier: RoadGeometryTier::Illegal, ..
    }] if (*measured - 20.0).abs() < 1e-4)
    );
    let (graph, center) = junction(&[0.0, 45.0, 120.0, 200.0, 280.0]);
    let out = node_violations(&graph, center);
    assert_eq!(out.len(), 2);
    assert!(out.iter().any(|v| matches!(
        v,
        RoadGeometryViolation::JunctionArmAngle {
            tier: RoadGeometryTier::Design,
            ..
        }
    )));
    assert!(out.iter().any(|v| matches!(
        v,
        RoadGeometryViolation::JunctionDegree {
            measured: 5,
            limit: DESIGN_JUNCTION_DEGREE,
            tier: RoadGeometryTier::Design,
            ..
        }
    )));
    let (graph, center) = junction(&[0.0, 175.0]);
    assert!(matches!(
        node_violations(&graph, center).as_slice(),
        [RoadGeometryViolation::DirectionKink {
            tier: RoadGeometryTier::Design,
            ..
        }]
    ));
}

#[test]
fn directionless_edges_count_for_degree_and_non_road_modes_are_excluded() {
    let (mut graph, center) = junction(&[0.0, 90.0, 180.0, 270.0, 10.0, 20.0, 30.0]);
    for id in 4..7 {
        graph.edge_mut(id).physical_geometry = vec![Vector3::ZERO, Vector3::ZERO];
    }
    assert!(matches!(
        node_violations(&graph, center).as_slice(),
        [RoadGeometryViolation::JunctionDegree {
            measured: 7,
            tier: RoadGeometryTier::Illegal,
            ..
        }]
    ));
    for id in 4..7 {
        graph.edge_mut(id).primary_type = TransitType::Rail;
    }
    assert!(audit_graph(&graph, &RoadGeometryLimits::default()).is_empty());
    let (mut graph, id) = ramp(30.0);
    graph.edge_mut(id).primary_type = TransitType::Foot;
    assert!(!edge_violations(&graph, id).is_empty());
    for mode in [TransitType::Rail, TransitType::Ship, TransitType::Air] {
        graph.edge_mut(id).primary_type = mode;
        assert!(edge_violations(&graph, id).is_empty());
    }
}

#[test]
fn audit_order_is_kind_then_id_and_independent_of_rayon_worker_count() {
    let (mut graph, _) = junction(&[
        0.0, 20.0, 55.0, 90.0, 125.0, 160.0, 200.0, 240.0, 280.0, 320.0,
    ]);
    for id in [0, 3, 9] {
        let edge = graph.edge_mut(id);
        edge.physical_geometry[1].y = 30.0;
    }
    let run = |workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| audit_graph(&graph, &RoadGeometryLimits::default()))
    };
    let out = run(1);
    assert!(!out.is_empty());
    assert_eq!(out, run(3));
    assert!(
        out.windows(2)
            .all(|pair| pair[0].sort_key() < pair[1].sort_key())
    );
    assert_eq!(out, run(3));
}

#[test]
fn limits_are_customizable_and_threshold_equality_is_clean() {
    let (graph, id) = ramp(12.0);
    let mut limits = RoadGeometryLimits::default();
    limits.standard_grade.design = 0.12;
    let mut out = Vec::new();
    validate_edge(&graph, id, &limits, &mut out);
    assert!(out.is_empty());
    limits.standard_grade.illegal = 0.11;
    validate_edge(&graph, id, &limits, &mut out);
    assert!(matches!(
        out.as_slice(),
        [RoadGeometryViolation::Grade {
            limit: 0.11,
            tier: RoadGeometryTier::Illegal,
            ..
        }]
    ));
    assert_eq!(
        upper_violation(0.16, &RoadGeometryLimits::default().standard_grade),
        Some((0.10, RoadGeometryTier::Design))
    );
    assert_eq!(
        lower_violation(
            60.0,
            &RoadGeometryLimits::default().junction_arm_angle_degrees
        ),
        None
    );
}

#[test]
fn empty_graph_and_retired_node_alias_are_ignored() {
    assert!(audit_graph(&RegionGraph::new(), &RoadGeometryLimits::default()).is_empty());
    let (mut graph, center) = junction(&[180.0, 20.0]);
    let alias = graph.add_node(Vector3::ZERO, NodeType::Junction);
    graph.node_aliases.insert(alias, center);
    graph.adjacency[alias as usize] = graph.node_adjacency(center).to_vec();
    assert!(node_violations(&graph, alias).is_empty());
    assert_eq!(
        audit_graph(&graph, &RoadGeometryLimits::default()),
        node_violations(&graph, center)
    );
}

#[test]
fn smooth_curve_split_at_a_node_is_not_a_kink() {
    // A 50 m radius arc sampled every 1 m and split into two edges at its midpoint.
    let arc = |from: i32, to: i32| -> Vec<Vector3> {
        (from..=to)
            .map(|i| {
                let a = i as f32 / 50.0;
                Vector3::new(50.0 * a.sin(), 0.0, 50.0 * (1.0 - a.cos()))
            })
            .collect()
    };
    let mut graph = RegionGraph::new();
    let start = graph.add_node(arc(-20, -20)[0], NodeType::Junction);
    let middle = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let end = graph.add_node(arc(20, 20)[0], NodeType::Junction);
    add_edge(&mut graph, start, middle, arc(-20, 0));
    add_edge(&mut graph, middle, end, arc(0, 20));
    graph.rebuild_adjacency_list();
    assert!(node_violations(&graph, middle).is_empty());
}

#[test]
fn a_densely_sampled_street_corner_is_not_a_tight_curve() {
    // A right-angle corner drawn with the straight tool, densified to 20 cm near the vertex as
    // the profile solver does: per vertex it would read as a radius of centimetres.
    let mut points: Vec<_> = (0..=100)
        .map(|i| Vector3::new(i as f32 * 0.2, 0.0, 0.0))
        .collect();
    points.extend((1..=100).map(|i| Vector3::new(20.0, 0.0, i as f32 * 0.2)));
    let (graph, id) = single_edge(points);
    assert!(
        !edge_violations(&graph, id).iter().any(|v| matches!(
            v,
            RoadGeometryViolation::HorizontalRadius {
                tier: RoadGeometryTier::Illegal,
                ..
            }
        )),
        "{:?}",
        edge_violations(&graph, id)
    );
}
