// SPDX-License-Identifier: GPL-2.0-only

//! The trip failure log's streak, place and minute limits.

use super::report::{DETAIL_PER_MINUTE, DETAIL_PER_PLACE};
use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::MODE_CAR;
use crate::simulation::economy::agents::data::AgentSystem;

fn agents(count: usize) -> AgentSystem {
    let mut agents = AgentSystem::new();
    for _ in 0..count {
        agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
    }
    agents
}

fn no_turn(agent: usize, node: u32) -> TripFailure {
    TripFailure {
        agent,
        kind: AgentKind::Resident,
        mode: MODE_CAR,
        node,
        edge: 5,
        lane: 9,
        lane_d: 12.5,
        origin: usize::MAX,
        destination: usize::MAX,
        border: u32::MAX,
        cause: FailureCause::NoTurnToEdge {
            node,
            from_lane: 9,
            from_edge: 5,
            to_edge: 30,
        },
        outcome: FailureOutcome::Replans,
    }
}

// Reports `failures` as one movement pass at `sim_time` and returns the lines it logged.
fn report(agents: &mut AgentSystem, sim_time: f32, failures: &[TripFailure]) -> Vec<String> {
    agents.sim_time = sim_time;
    agents.trip_failures.buffer.extend_from_slice(failures);
    let before = agents.trip_failures.lines.len();
    agents.report_trip_failures(&BuildingAllocator::new());
    agents.trip_failures.lines[before..].to_vec()
}

#[test]
fn a_line_names_the_agent_its_place_its_cause_and_what_happens_next() {
    let mut agents = agents(3);
    let render_id = agents.render_id[2];
    let lines = report(&mut agents, 1.0, &[no_turn(2, 12)]);
    assert_eq!(
        lines,
        [format!(
            "trip failed: agent {render_id} [row 2], resident, car, at node 12, lane 9 on edge 5 \
             at 12.5 m, with no destination: no turn from lane 9 (edge 5) onto edge 30 at node \
             12; drops its route and replans"
        )]
    );
}

#[test]
fn an_agent_repeating_a_failure_logs_it_once_per_streak() {
    let mut agents = agents(1);
    assert_eq!(report(&mut agents, 1.0, &[no_turn(0, 12)]).len(), 1);
    // Every tick for most of a minute: a pedestrian waiting at a missing turn.
    for second in 2..60 {
        assert!(report(&mut agents, second as f32, &[no_turn(0, 12)]).is_empty());
    }
    // The same failure elsewhere, and the watchdog's recovery, are new.
    assert_eq!(report(&mut agents, 60.5, &[no_turn(0, 13)]).len(), 1);
    let mut recovery = no_turn(0, 12);
    recovery.outcome = FailureOutcome::NoBorder { after_s: 5.0 };
    let lines = report(&mut agents, 61.0, &[recovery]);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with("; no border node to send it to; retry in 5 s"));
    assert!(report(&mut agents, 66.0, &[recovery]).is_empty());
    // A minute after its last repeat the streak is over.
    assert!(report(&mut agents, 118.5, &[no_turn(0, 12)]).is_empty());
    assert_eq!(report(&mut agents, 180.0, &[no_turn(0, 12)]).len(), 1);
}

#[test]
fn failures_past_the_place_limit_fold_into_one_summary_when_the_minute_ends() {
    let place = DETAIL_PER_PLACE as usize;
    let mut agents = agents(place + 5 + 3);
    let mut failures: Vec<TripFailure> = (0..place + 5).map(|agent| no_turn(agent, 12)).collect();
    // Another junction keeps its own limit.
    failures.extend((place + 5..place + 8).map(|agent| no_turn(agent, 40)));
    let lines = report(&mut agents, 10.0, &failures);
    assert_eq!(lines.len(), place + 3);
    assert!(lines[place - 1].contains(&format!("[row {}]", place - 1)));
    assert!(lines[place].contains("at node 40"));
    assert!(report(&mut agents, 59.0, &[]).is_empty());
    let first_folded = agents.render_id[place];
    assert_eq!(
        report(&mut agents, 61.0, &[]),
        [format!(
            "trip failures: 5 more in the last simulated minute: no turn at a junction, 5 at \
             node 12 (first agent {first_folded})"
        )]
    );
    // The new minute starts with fresh limits and no summary to repeat.
    assert!(report(&mut agents, 125.0, &[]).is_empty());
    assert_eq!(report(&mut agents, 126.0, &[no_turn(1, 12)]).len(), 1);
}

#[test]
fn the_minute_limit_folds_failures_everywhere_but_never_the_inspected_agent() {
    let total = DETAIL_PER_MINUTE as usize + 50;
    let mut agents = agents(total);
    let inspected = total - 1;
    let render_id = agents.inspect_agent(inspected).unwrap();
    // Every failure at its own node, so only the minute limit applies.
    let failures: Vec<TripFailure> = (0..total)
        .map(|agent| no_turn(agent, agent as u32))
        .collect();
    let lines = report(&mut agents, 1.0, &failures);
    assert_eq!(lines.len(), DETAIL_PER_MINUTE as usize + 1);
    assert_eq!(
        agents.inspected_agent_failure(render_id),
        lines.last().map(String::as_str)
    );
    assert!(
        lines
            .last()
            .unwrap()
            .contains(&format!("[row {inspected}]"))
    );
    let summary = report(&mut agents, 60.0, &[]);
    assert_eq!(summary.len(), 1);
    assert!(
        summary[0].starts_with("trip failures: 49 more in the last simulated minute: no turn"),
        "{}",
        summary[0]
    );
}
