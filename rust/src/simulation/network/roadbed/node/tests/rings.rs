// SPDX-License-Identifier: GPL-2.0-only

//! Roundabout rings (`ROAD-62`) over the size and arm matrix, flat and sloped: every arm node lays
//! out with a straight island sector and full corners, and every piece is watertight against its
//! spans.

use super::*;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::fixtures::{
    bare_ring, converted_ring, finalize, join_ring_arm,
};
use crate::simulation::network::roadbed::{NodeArms, SectorKind};

// Three to six arms that a bare ring takes on its own nodes and between them.
const JOINED_ANGLES: [f32; 6] = [90.0, 210.0, 330.0, 30.0, 150.0, 270.0];

fn equal(arms: usize) -> Vec<f32> {
    (0..arms).map(|i| i as f32 * 360.0 / arms as f32).collect()
}

fn check_ring(name: &str, graph: &RegionGraph, nodes: &[u32]) {
    let roadbed = compiled(graph);
    assert!(
        roadbed.failed_nodes().is_empty(),
        "{name}: {:?}",
        roadbed.failed_nodes()
    );
    for &node in nodes {
        let layout = NodeArms::collect(graph, node)
            .unwrap_or_else(|| panic!("{name}: node {node} passes through"))
            .layout()
            .unwrap_or_else(|error| panic!("{name}: node {node}: {error:?}"));
        let straight: Vec<_> = layout
            .sectors
            .iter()
            .filter(|sector| sector.kind == SectorKind::Straight)
            .collect();
        assert_eq!(
            straight.len(),
            1,
            "{name}: node {node}: {:?}",
            layout.sectors
        );
        assert!(
            (straight[0].angle_rad.to_degrees() - 180.0).abs() < 0.01,
            "{name}"
        );
        for sector in layout
            .sectors
            .iter()
            .filter(|s| s.kind != SectorKind::Straight)
        {
            let radius_m = sector.corner.expect("an approach corner").radius_m;
            assert!(
                (radius_m - 3.5).abs() < 1e-6,
                "{name}: node {node}: R {radius_m}"
            );
        }
        let piece = roadbed.node_piece(node).expect("a piece");
        check_watertight(name, &roadbed, piece);
    }
}

#[test]
fn rings_over_the_size_and_arm_matrix_lay_out_watertight() {
    for sloped in [false, true] {
        // Conversions reach 60 m (D14).
        for (diameter_m, arm) in [
            (30.0, (1, 1)),
            (36.0, (1, 1)),
            (45.0, (1, 1)),
            (60.0, (1, 1)),
            (45.0, (2, 2)),
            (50.0, (2, 2)),
            (60.0, (2, 2)),
        ] {
            for arms in 3..=6 {
                let name = format!("D{diameter_m} {arm:?} x{arms} sloped {sloped}");
                let (graph, applied) = converted_ring(diameter_m, &equal(arms), arm, sloped)
                    .unwrap_or_else(|reason| panic!("{name}: {reason}"));
                check_ring(&name, &graph, &applied.nodes);
            }
        }
        // Larger rings and two-lane rings of one-lane arms come from the ground and joins.
        for (diameter_m, lanes, arm) in [
            (45.0, 2, (1, 1)),
            (50.0, 2, (1, 1)),
            (80.0, 2, (1, 1)),
            (80.0, 2, (2, 2)),
        ] {
            for arms in 3..=6 {
                let name = format!("joined D{diameter_m} L{lanes} {arm:?} x{arms} sloped {sloped}");
                let (mut graph, _) = bare_ring(diameter_m, lanes, sloped);
                let nodes: Vec<u32> = JOINED_ANGLES[..arms]
                    .iter()
                    .map(|&angle| join_ring_arm(&mut graph, angle, arm, sloped).0)
                    .collect();
                finalize(&mut graph);
                check_ring(&name, &graph, &nodes);
            }
        }
    }
}

// How close two arms may be, as ring arc between their nodes (four arms, one moved towards its
// neighbour). PLAN-R7's probe measured two-sided rings, whose ring arms' zebras added 3 m at each
// mouth; the outer-only ring has none, so six equal arms fit at every size.
#[test]
fn two_arms_need_their_two_mouths_of_ring_between_them() {
    for (diameter_m, arm, radius_m, refused_m, fits_m) in [
        (30.0, (1, 1), 13.25, 8.0, 9.0),
        (36.0, (1, 1), 16.25, 9.0, 10.0),
        (60.0, (1, 1), 28.25, 9.0, 10.0),
        (45.0, (2, 2), 19.0, 14.0, 15.0),
        (50.0, (2, 2), 21.5, 14.0, 15.0),
        (60.0, (2, 2), 26.5, 15.0, 16.0),
    ] {
        let fits = |arc_m: f32| {
            let gap_deg = (arc_m / radius_m as f32).to_degrees();
            let (graph, applied) =
                converted_ring(diameter_m, &[0.0, gap_deg, 180.0, 270.0], arm, false).unwrap();
            compiled(&graph).failed_nodes().is_empty()
                && applied
                    .nodes
                    .iter()
                    .all(|&node| NodeArms::collect(&graph, node).unwrap().layout().is_ok())
        };
        assert!(!fits(refused_m), "D{diameter_m} {arm:?} at {refused_m} m");
        assert!(fits(fits_m), "D{diameter_m} {arm:?} at {fits_m} m");
    }
}

// A pass-through ring node holds a join as an arm would: the ring between them must take the
// join's two ring mouths, so a stroke within this arc of a bare node is refused unless it snaps to
// the node (a finding of round R7a, for the tool's review).
#[test]
fn a_join_near_a_bare_ring_node_needs_the_arc_of_a_mouth() {
    for (diameter_m, lanes, arm, radius_m, refused_m, fits_m) in [
        (36.0, 1, (1, 1), 16.25f32, 9.0, 10.0),
        (60.0, 1, (1, 1), 28.25, 9.0, 10.0),
        (50.0, 2, (2, 2), 21.5, 14.0, 15.0),
    ] {
        let fits = |arc_m: f32| {
            let (mut graph, _) = bare_ring(diameter_m, lanes, false);
            let angle = 330.0 + (arc_m / radius_m).to_degrees();
            let (node, _) = join_ring_arm(&mut graph, angle, arm, false);
            finalize(&mut graph);
            NodeArms::collect(&graph, node).unwrap().layout().is_ok()
                && compiled(&graph).failed_nodes().is_empty()
        };
        assert!(!fits(refused_m), "D{diameter_m} {arm:?} at {refused_m} m");
        assert!(fits(fits_m), "D{diameter_m} {arm:?} at {fits_m} m");
    }
}
