// SPDX-License-Identifier: GPL-3.0-or-later

//! `ROAD-126` round JS5f: `ROAD-54`'s junction arm limit is 20 degrees (the user's decision of
//! 2026-11-18; it was 25). On a flat city of 2-lane roads, a Y whose second branch leaves a road
//! end 20 degrees from the first and a T whose side road ends on a street's centreline 20 degrees
//! off it build, held to the alongside tests' bar: the preview is the commit, no node is left
//! without a road (also after a save and a load), undo and a second commit restore the graph and
//! the roadbed bit for bit. At their node every vehicle connector keeps to the carriageway and
//! meets its mouths (`TRAFFIC-12`'s measure), and every crossing stands across its own arm. The
//! same strokes 5 degrees under the limit (15 degrees) are refused, naming the angle and the limit.

use super::alongside::build_on;
use super::contacts::{city, lay, preview_parked, v};
use super::stubs_and_joins::node_near;
use super::*;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::lanes::trajectory_tests::measure_connectors;
use crate::simulation::network::road_edit::MIN_CONTACT_ANGLE_DEG;

// A 2-lane street along z = 0 from x = -50 to 50.
const STREET: [[(f32, f32); 2]; 1] = [[(-50.0, 0.0), (50.0, 0.0)]];

// The unit direction `deg` degrees from +x towards +z.
fn heading(deg: f32) -> Vector2 {
    v(deg.to_radians().cos(), deg.to_radians().sin())
}

// A road from the west ending at the origin and a first branch from that end 50 m along 10
// degrees: the Y's stem and branch, a bend until the second branch joins.
fn y_city() -> [[(f32, f32); 2]; 2] {
    let branch = heading(10.0) * 50.0;
    [
        [(-50.0, 0.0), (0.0, 0.0)],
        [(0.0, 0.0), (branch.x, branch.y)],
    ]
}

// The Y's second branch, from the road end at the origin 50 m along `10 - angle` degrees:
// `angle` degrees from the first.
fn y_branch(angle: f32) -> [Vector2; 2] {
    [v(0.0, 0.0), heading(10.0 - angle) * 50.0]
}

// The T's side road, from 60 m out ending on the street's centreline at the origin, its arm
// `angle` degrees from the street's west arm.
fn t_side(angle: f32) -> [Vector2; 2] {
    [heading(180.0 + angle) * 60.0, v(0.0, 0.0)]
}

// The node at the origin: a junction of three roads whose vehicle connectors keep to the
// carriageway and meet their mouths, and whose crossings each stand across their own road, the
// kerb-to-kerb stretch centred on its centreline and as long as every other crossing there (all
// arms are the same 2-lane road).
fn assert_junction_lays_out(core: &SimCore, label: &str) {
    let node = node_near(core, v(0.0, 0.0));
    assert_eq!(degree(core, node), 3, "{label}: a junction of three roads");
    let (graph, network) = (&core.region_graph, &core.transit_network);
    let lanes = &network.lane_system;
    let counts = measure_connectors(graph, &network.roadbed, lanes, [node as usize], true);
    assert!(counts.connectors > 0, "{label}: no connectors");
    assert_eq!(
        (
            counts.outside,
            counts.backward,
            counts.countersteer,
            counts.kinks
        ),
        (0, 0, 0, 0),
        "{label}: {}",
        counts.report(label)
    );
    assert_eq!(
        (counts.end_line_misses, counts.end_angle_misses),
        (0, 0),
        "{label}: {}",
        counts.report(label)
    );
    let mut widths = Vec::new();
    for &id in &lanes.node_lanes[&(node as usize)] {
        let Some(crossing) = lanes.crosswalk(id) else {
            continue;
        };
        let g = &lanes.lanes[id].geometry;
        let (a, b) = (Vector2::new(g[1].x, g[1].z), Vector2::new(g[2].x, g[2].z));
        let middle = (a + b) * 0.5;
        let road = &graph.edge(crossing.edge_id).geometry;
        let off = road
            .windows(2)
            .map(|pair| {
                let (p, q) = (
                    Vector2::new(pair[0].x, pair[0].z),
                    Vector2::new(pair[1].x, pair[1].z),
                );
                let t = ((middle - p).dot(q - p) / (q - p).length_squared()).clamp(0.0, 1.0);
                middle.distance_to(p + (q - p) * t)
            })
            .fold(f32::INFINITY, f32::min);
        assert!(
            off < 0.01,
            "{label}: crossing {id} {off} m off edge {}",
            crossing.edge_id
        );
        widths.push(a.distance_to(b));
    }
    assert_eq!(
        widths.len(),
        6,
        "{label}: a crossing each way over each arm"
    );
    assert!(
        widths.iter().all(|width| (width - widths[0]).abs() < 0.01),
        "{label}: {widths:?}"
    );
}

#[test]
fn a_y_at_the_arm_limit_builds() {
    assert_eq!(MIN_CONTACT_ANGLE_DEG, 20.0);
    let (core, preview, _) = build_on(&y_city(), y_branch(MIN_CONTACT_ANGLE_DEG), "Y");
    assert_eq!(preview.validation.overlap_side, None);
    assert_junction_lays_out(&core, "Y");
}

#[test]
fn a_t_at_the_arm_limit_builds() {
    let (core, preview, _) = build_on(&STREET, t_side(MIN_CONTACT_ANGLE_DEG), "T");
    assert_eq!(preview.validation.overlap_side, None);
    assert_junction_lays_out(&core, "T");
}

// The Y and the T 5 degrees under the limit: refused naming 15 and 20 degrees, the commit refuses
// too and the graph stays as it was.
#[test]
fn fifteen_degrees_are_refused_naming_the_angle() {
    let angle = MIN_CONTACT_ANGLE_DEG - 5.0;
    for (label, roads, ends) in [
        ("Y", &y_city()[..], y_branch(angle)),
        ("T", &STREET[..], t_side(angle)),
    ] {
        let mut core = city(roads, 1);
        let before = graph_hash(&core);
        let points = lay(&core, &ends);
        let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
        let refused = &preview.validation;
        assert_eq!(
            refused.invalid_reason, "junction_arm_angle",
            "{label}: {refused:?}"
        );
        assert!(
            (refused.geometry_measured - angle).abs() < 0.01,
            "{label}: {refused:?}"
        );
        assert_eq!(refused.geometry_limit, MIN_CONTACT_ANGLE_DEG, "{label}");
        let text = reason_text(refused.invalid_reason, &facts(refused)).unwrap();
        assert_eq!(text, "Roads meet at 15°; junction arms need 20°", "{label}");
        let outcome = super::contacts::commit_parked(&mut core, &points, RoadParking::NONE);
        assert!(!outcome.committed, "{label}: the commit refuses it too");
        assert_eq!(graph_hash(&core), before, "{label}");
    }
}

// The arm angle's rounding (decision D-JS5f-1): the reference Y's second branch, drawn exactly 20
// degrees from the first (80 m arms), whose `f32` points read under 20 degrees, builds; at 19.998
// degrees it is refused.
#[test]
fn a_y_drawn_exactly_at_the_limit_builds_and_19_998_degrees_is_refused() {
    let arm =
        |deg: f32| Vector2::from_angle(deg.to_radians()) * super::super::road_edit_order::ARM_M;
    for (angle, builds) in [(20.0_f32, true), (19.998, false)] {
        let (first, second) = (arm(0.5 * angle), arm(-0.5 * angle));
        if builds {
            let (cross, dot) = (
                f64::from(first.x) * f64::from(second.y) - f64::from(first.y) * f64::from(second.x),
                f64::from(first.x) * f64::from(second.x) + f64::from(first.y) * f64::from(second.y),
            );
            assert!(
                cross.abs().atan2(dot).to_degrees() < 20.0,
                "the points read under the limit"
            );
        }
        let mut core = city(
            &[[(-80.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (first.x, first.y)]],
            1,
        );
        let points = lay(&core, &[v(0.0, 0.0), second]);
        let (preview, _) = preview_parked(&mut core, &points, (1, 1), false, RoadParking::NONE);
        let validation = &preview.validation;
        assert_eq!(validation.is_valid, builds, "{angle}: {validation:?}");
        if !builds {
            assert_eq!(validation.invalid_reason, "junction_arm_angle", "{angle}");
        }
    }
}
