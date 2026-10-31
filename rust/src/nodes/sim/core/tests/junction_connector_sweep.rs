// SPDX-License-Identifier: GPL-2.0-only

//! Save sweep of vehicle junction connectors (`TRAFFIC-12`): loads a saved city and measures
//! every vehicle connector at every node against the compiled carriageway, and every vehicle
//! lane end against its arm's mouth frame (`lanes::trajectory_tests::measure_connectors`).
//!
//! `METRUM_SWEEP_SAVE` names the save; `METRUM_SWEEP_OUT`, if set, receives one line per failing
//! connector; `METRUM_SWEEP_STRICT=1` fails the test on any violation.

use super::super::scenario::install_bootstrap_assets;
use super::test_core;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::lanes::trajectory_tests::measure_connectors;
use godot::prelude::Vector2;

#[test]
#[ignore = "manual save sweep: METRUM_SWEEP_SAVE=<save.sqlite>, release build"]
fn junction_connector_save_sweep() {
    let path = std::env::var("METRUM_SWEEP_SAVE").expect("METRUM_SWEEP_SAVE names a save");
    let mut core = test_core();
    install_bootstrap_assets(&mut core);
    core.load_game_internal(&path)
        .unwrap_or_else(|error| panic!("could not load {path}: {error}"));
    let started = std::time::Instant::now();
    let graph = &core.region_graph;
    let network = &core.transit_network;
    let out = std::env::var("METRUM_SWEEP_OUT").ok();
    let counts = measure_connectors(
        graph,
        &network.roadbed,
        &network.lane_system,
        0..graph.node_count(),
        out.is_some(),
    );
    eprintln!(
        "SWEEP {path}: {} nodes, measured in {:.1} s\n{}",
        graph.node_count(),
        started.elapsed().as_secs_f64(),
        counts.report("sweep")
    );
    // Span lanes (`TRAFFIC-12` finding): the mitred offset of a sharp bend can spike or reverse a
    // vehicle lane inside its span. Turns between steps of at least 1 cm.
    let (mut kinked, mut reversed, mut max_turn) = (0, 0, 0.0_f32);
    for lane in &network.lane_system.lanes {
        if lane.edge_id == usize::MAX || lane.lane_type != LaneType::Vehicle {
            continue;
        }
        let mut steps = Vec::new();
        let mut anchor = lane.geometry.first().copied();
        for &point in lane.geometry.iter().skip(1) {
            let Some(from) = anchor else { break };
            let step = Vector2::new(point.x - from.x, point.z - from.z);
            if step.length() > 0.01 {
                steps.push(step.normalized());
                anchor = Some(point);
            }
        }
        let turns = steps.windows(2).map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            (a.x * b.y - a.y * b.x).atan2(a.dot(b)).abs()
        });
        let lane_max = turns.fold(0.0_f32, f32::max);
        max_turn = max_turn.max(lane_max);
        kinked += usize::from(lane_max > 0.2);
        reversed += usize::from(lane_max > std::f32::consts::FRAC_PI_2);
    }
    eprintln!(
        "SPAN LANES: {kinked} vehicle lanes turn more than 0.2 rad between steps, {reversed} \
         reverse (over a right angle); largest turn {max_turn:.3} rad"
    );
    if let Some(out) = out {
        let mut lines = counts.lines.clone();
        // Every laid-out node: position, kind and arms (direction in degrees, `z` for a zebra).
        for node in 0..graph.node_count() {
            let Some(piece) = network.roadbed.node_piece(node as u32) else {
                continue;
            };
            let layout = piece.layout();
            let arms: Vec<String> = layout
                .arms
                .iter()
                .map(|arm| {
                    let zebra = if arm.crosswalk.is_some() { "z" } else { "" };
                    format!("{:.0}{zebra}", arm.angle_rad.to_degrees())
                })
                .collect();
            let pos = graph.node(node as u32).pos;
            lines.push(format!(
                "NODE {node} at ({:.1}, {:.1}) {:?} arms [{}]",
                pos.x,
                pos.z,
                layout.kind,
                arms.join(", ")
            ));
        }
        std::fs::write(&out, lines.join("\n") + "\n")
            .unwrap_or_else(|error| panic!("could not write {out}: {error}"));
    }
    if std::env::var("METRUM_SWEEP_STRICT").is_ok_and(|strict| strict == "1") {
        assert!(counts.failed + counts.end_line_misses + counts.end_angle_misses == 0);
    }
}
