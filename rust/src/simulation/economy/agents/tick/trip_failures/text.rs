// SPDX-License-Identifier: GPL-2.0-only

//! The words of a trip failure line and of the per-minute summaries.

use super::super::super::{MODE_CAR, MODE_WALK};
use super::super::planning::{ReplanCandidateDiagnostic, ReplanFailure};
use super::{AgentKind, FailureCause, FailureOutcome, TripFailure};
use crate::simulation::buildings::allocator::BuildingAllocator;
use std::fmt::{self, Write as _};

/// One detailed line: who, where, where to, why and what happens next.
pub(super) fn failure_line(
    failure: &TripFailure,
    render_id: u64,
    allocator: &BuildingAllocator,
) -> String {
    let mode = match failure.mode {
        MODE_CAR => "car, ",
        MODE_WALK => "on foot, ",
        _ => "",
    };
    let mut line = format!(
        "trip failed: agent {render_id} [row {}], {}, {mode}",
        failure.agent,
        kind_label(failure.kind),
    );
    let _ = write_place(&mut line, failure);
    let _ = write_destination(&mut line, failure, allocator);
    let _ = write!(line, ": {}; {}", Cause(failure), Outcome(failure.outcome));
    line
}

/// Short names of the causes, by [`cause_index`], for the summaries.
pub(super) const CAUSE_LABELS: [&str; 11] = [
    "no road to the next node",
    "no turn at a junction",
    "no turn into the exit lane",
    "no route to the destination",
    "planned exit from the road no longer legal",
    "planned way onto the road no longer legal",
    "no route to the border",
    "no route in from the border",
    "no route at departure",
    "no position to plan from",
    "no plan",
];

/// The cause's index in [`CAUSE_LABELS`].
pub(super) fn cause_index(cause: &FailureCause) -> u8 {
    match cause {
        FailureCause::NoRoad { .. } => 0,
        FailureCause::NoTurnToEdge { .. } => 1,
        FailureCause::NoTurnToLane { .. } => 2,
        FailureCause::NoRoute(_) => 3,
        FailureCause::StaleExit { .. } => 4,
        FailureCause::StaleEntry { .. } => 5,
        FailureCause::NoBorderRoute { .. } => 6,
        FailureCause::NoImmigrationRoute { .. } => 7,
        FailureCause::NoDeparture { .. } => 8,
        FailureCause::NoPosition => 9,
        FailureCause::NoPlan => 10,
    }
}

/// Whether the cause happens at a building door rather than at a node.
pub(super) fn cause_at_building(cause: usize) -> bool {
    cause == 5 || cause == 8
}

fn kind_label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Resident => "resident",
        AgentKind::FreightCarrier => "freight carrier",
        AgentKind::MovingIn => "household moving in",
    }
}

fn write_place(line: &mut String, failure: &TripFailure) -> fmt::Result {
    if failure.lane != usize::MAX {
        write!(line, "at node {}, lane {} ", failure.node, failure.lane)?;
        if failure.edge == usize::MAX {
            write!(line, "on a junction ")?;
        } else {
            write!(line, "on edge {} ", failure.edge)?;
        }
        return write!(line, "at {:.1} m", failure.lane_d);
    }
    if failure.origin != usize::MAX {
        return write!(line, "in building {}", failure.origin);
    }
    if failure.node != u32::MAX {
        return write!(line, "at node {}", failure.node);
    }
    write!(line, "off the road")
}

fn write_destination(
    line: &mut String,
    failure: &TripFailure,
    allocator: &BuildingAllocator,
) -> fmt::Result {
    if let Some(building) = allocator.buildings.get(failure.destination) {
        return write!(
            line,
            ", to building {} ({})",
            failure.destination,
            building.zone_type.as_str()
        );
    }
    if failure.destination != usize::MAX {
        return write!(line, ", to building {} (gone)", failure.destination);
    }
    if failure.border != u32::MAX {
        return write!(line, ", to border node {}", failure.border);
    }
    write!(line, ", with no destination")
}

struct Cause<'a>(&'a TripFailure);

impl fmt::Display for Cause<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let failure = self.0;
        match failure.cause {
            FailureCause::NoRoad { node, next_node } => write!(
                f,
                "no road from node {node} to node {next_node}, the next node on its route"
            ),
            FailureCause::NoTurnToEdge {
                node,
                from_lane,
                from_edge,
                to_edge,
            } => write!(
                f,
                "no turn from lane {from_lane} (edge {from_edge}) onto edge {to_edge} at node {node}"
            ),
            FailureCause::NoTurnToLane {
                node,
                from_lane,
                to_lane,
            } => write!(
                f,
                "no turn from lane {from_lane} into its exit lane {to_lane} at node {node}"
            ),
            FailureCause::NoRoute(replan) => write_replan(f, &replan),
            FailureCause::StaleExit {
                lane,
                lane_d,
                replan,
            } => {
                write!(
                    f,
                    "planned exit from the road no longer legal (building {}, lane {lane} at \
                     {lane_d:.1} m), then ",
                    failure.destination
                )?;
                match replan {
                    Some(replan) => write_replan(f, &replan),
                    None => write!(f, "no position to plan from"),
                }
            }
            FailureCause::StaleEntry { lane, lane_d } => write!(
                f,
                "planned way onto the road no longer legal (building {}, lane {lane} at \
                 {lane_d:.1} m)",
                failure.origin
            ),
            FailureCause::NoBorderRoute { from, border } => {
                write!(f, "no road path from node {from} to border node {border}")
            }
            FailureCause::NoImmigrationRoute { border, building } => {
                write!(
                    f,
                    "no route from border node {border} to building {building}"
                )
            }
            FailureCause::NoDeparture { has_car } => write!(
                f,
                "no route from building {} to building {} {}",
                failure.origin,
                failure.destination,
                if has_car {
                    "by car or on foot"
                } else {
                    "on foot (no car)"
                }
            ),
            FailureCause::NoPosition => write!(f, "no position on the road to plan from"),
            FailureCause::NoPlan => write!(f, "travelling with no plan to reach its destination"),
        }
    }
}

fn write_replan(f: &mut fmt::Formatter<'_>, replan: &ReplanFailure) -> fmt::Result {
    let building = replan.building;
    match replan.reason {
        "target_building_out_of_range" => write!(f, "building {building} no longer exists"),
        "destination_edge_out_of_range" | "destination_edge_deleted" => write!(
            f,
            "the road beside building {building} (edge {}) is gone",
            replan.edge
        ),
        reason => {
            write!(
                f,
                "no route from node {} to building {building}",
                replan.start_node
            )?;
            if reason != "no_valid_destination_candidate" {
                return write!(f, " ({reason})");
            }
            let mut separator = ": ";
            for candidate in replan.candidates.iter().flatten() {
                write!(f, "{separator}")?;
                write_candidate(f, replan.start_node, candidate)?;
                separator = "; ";
            }
            Ok(())
        }
    }
}

// One rejected exit candidate, by the planner's rejection code.
fn write_candidate(
    f: &mut fmt::Formatter<'_>,
    start: u32,
    candidate: &ReplanCandidateDiagnostic,
) -> fmt::Result {
    let node = candidate.planned_detach_node;
    let lane = candidate.planned_detach_lane_id;
    match candidate.reason {
        "candidate_lane_missing" => write!(f, "no lane by the door from node {node}"),
        "lane_origin_missing" => write!(f, "exit lane {lane} has no start node"),
        "lane_origin_mismatch" => write!(f, "exit lane {lane} does not start at node {node}"),
        "projection_failed" => write!(f, "the door is not beside exit lane {lane}"),
        "local_access_failed" => write!(f, "no way from exit lane {lane} to the door"),
        "frontage_failed" => write!(f, "no frontage on exit lane {lane}"),
        "zero_hop_connector_missing" => write!(f, "no turn into exit lane {lane} at node {node}"),
        "cch_no_path" => write!(f, "no road path from node {start} to node {node}"),
        "path_too_short" => write!(f, "the path from node {start} to node {node} is too short"),
        "pedestrian_connector_fallback_missing" => write!(
            f,
            "no walkable path from node {start} to exit lane {lane} at node {node}"
        ),
        reason => write!(f, "exit lane {lane} at node {node}: {reason}"),
    }
}

struct Outcome(FailureOutcome);

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            FailureOutcome::Replans => write!(f, "drops its route and replans"),
            FailureOutcome::Waits => write!(f, "waits at the end of its lane"),
            FailureOutcome::Retry {
                after_s,
                failure,
                of,
            } => write!(f, "retry in {after_s:.0} s (failure {failure} of {of})"),
            FailureOutcome::RetryDeparture { after_s } => {
                write!(f, "tries to leave again in {after_s:.0} s")
            }
            FailureOutcome::Home {
                building,
                pos,
                failures,
            } => write!(
                f,
                "sent home to building {building} at ({:.1}, {:.1}) after {failures} failures",
                pos.x, pos.y
            ),
            FailureOutcome::Border {
                node,
                pos,
                building,
                failures,
            } => {
                write!(
                    f,
                    "sent to border node {node} at ({:.1}, {:.1}) after {failures} failures",
                    pos.x, pos.y
                )?;
                if building != usize::MAX {
                    write!(f, " to drive in to building {building} again")?;
                }
                Ok(())
            }
            FailureOutcome::NoBorder { after_s } => {
                write!(f, "no border node to send it to; retry in {after_s:.0} s")
            }
        }
    }
}
