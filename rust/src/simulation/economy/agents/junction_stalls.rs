// SPDX-License-Identifier: GPL-2.0-only

//! Stuck-at-junction detector for the `junction_connectors` debug view.
//!
//! An observer, not part of the simulation tick: its owner calls [`JunctionStallDetector::scan`]
//! while the view is on and drops the detector when it goes off. Each scan reads the retained
//! lane occupancy snapshot, so cost is O(occupied lanes + cars on connectors) once per simulated
//! second, and nothing is spent while no detector exists.

use super::tick::lane_attach_slot_clear;
use super::{AgentSystem, MODE_CAR, TRANSIT_INTERSECTION};
use crate::simulation::network::lanes::{LaneSystem, LaneType};
use std::collections::HashMap;
use std::fmt;

/// Simulated seconds a car must stay put on a connector before it is reported.
pub(crate) const STALL_AFTER_S: f32 = 10.0;
const SCAN_INTERVAL_S: f32 = 1.0;
// Movement below this since the stall began does not count as progress.
const MOVED_EPS_M: f32 = 0.5;
// A car this close to the connector end is waiting to exit.
const AT_END_EPS_M: f32 = 0.05;

/// Returns `true` when a car at `lane_d` on a connector of `length` waits at its end, where
/// `exit_connector_lane` holds a car whose exit lane entry is not free.
pub(crate) fn waits_at_connector_end(lane_d: f32, length: f32) -> bool {
    lane_d + AT_END_EPS_M >= length
}

/// Why a reported car is not leaving its connector, read from the occupancy snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StallCause {
    /// The car waits at the connector end and the exit lane's entry slot is taken.
    ExitOccupied {
        exit_lane: usize,
        blocker_render_id: u64,
        blocker_d: f32,
    },
    /// The car waits at the connector end, yet the exit entry is clear at scan time: another
    /// connector merging into the same lane keeps winning the entry claim.
    ExitClaimLost { exit_lane: usize },
    /// The connector has no exit lane.
    NoExit,
    /// The car is queued behind another car on the same connector.
    QueuedBehind {
        leader_render_id: u64,
        leader_d: f32,
    },
    /// None of the above holds at scan time.
    Unexplained { speed: f32 },
}

impl fmt::Display for StallCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ExitOccupied {
                exit_lane,
                blocker_render_id,
                blocker_d,
            } => write!(
                f,
                "exit lane {exit_lane} entry occupied by car {blocker_render_id} at {blocker_d:.1} m"
            ),
            Self::ExitClaimLost { exit_lane } => write!(
                f,
                "exit lane {exit_lane} entry is clear now; merging connectors keep winning its claim"
            ),
            Self::NoExit => write!(f, "connector has no exit lane"),
            Self::QueuedBehind {
                leader_render_id,
                leader_d,
            } => write!(
                f,
                "queued behind car {leader_render_id} at {leader_d:.1} m on the same connector"
            ),
            Self::Unexplained { speed } => {
                write!(f, "no blocker found (speed {speed:.2} m/s)")
            }
        }
    }
}

/// One car that has not moved on a connector for [`STALL_AFTER_S`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StallReport {
    /// Runtime-stable agent id.
    pub render_id: u64,
    /// Agent index at scan time; indices change on swap-remove.
    pub agent: usize,
    /// Junction node of the connector.
    pub node_id: usize,
    /// Connector lane id.
    pub connector: usize,
    /// Car position along the connector.
    pub lane_d: f32,
    /// Connector length.
    pub connector_length: f32,
    /// Simulated seconds without progress.
    pub stalled_s: f32,
    /// Why the car is not moving.
    pub cause: StallCause,
}

impl fmt::Display for StallReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "junction stall: car {} (agent {}) stopped {:.0} s at node {} on connector {} \
             ({:.1}/{:.1} m): {}",
            self.render_id,
            self.agent,
            self.stalled_s,
            self.node_id,
            self.connector,
            self.lane_d,
            self.connector_length,
            self.cause
        )
    }
}

struct Watch {
    lane_id: usize,
    anchor_d: f32,
    since_s: f32,
    reported: bool,
    seen_scan: u64,
}

/// Tracks cars on vehicle connectors and reports each stall once.
#[derive(Default)]
pub(crate) struct JunctionStallDetector {
    last_scan_s: Option<f32>,
    scan: u64,
    // Keyed by render id, which survives swap-remove; entries leave when the car leaves its
    // connector or moves.
    watched: HashMap<u64, Watch>,
}

impl JunctionStallDetector {
    /// Scans at most once per simulated second and returns the newly stalled cars in occupancy
    /// order. Skips the scan while the occupancy snapshot is invalid; restarts after the clock
    /// goes backwards, as on a load.
    pub(crate) fn scan(&mut self, agents: &AgentSystem, lanes: &LaneSystem) -> Vec<StallReport> {
        let now = agents.sim_time;
        match self.last_scan_s {
            Some(last) if now < last => *self = Self::default(),
            Some(last) if now < last + SCAN_INTERVAL_S => return Vec::new(),
            _ => {}
        }
        if !agents.lane_bucket_snapshot_valid {
            return Vec::new();
        }
        self.last_scan_s = Some(now);
        self.scan += 1;
        let mut reports = Vec::new();
        for &lane_id in &agents.dirty_lanes {
            let Some(lane) = lanes.lanes.get(lane_id) else {
                continue;
            };
            if lane.edge_id != usize::MAX || lane.lane_type != LaneType::Vehicle {
                continue;
            }
            let bucket = &agents.lane_buckets[lane_id];
            for (slot, &(lane_d, agent)) in bucket.iter().enumerate() {
                if agent >= agents.agents.len()
                    || agents.current_lane_id[agent] != lane_id
                    || agents.transit[agent] != TRANSIT_INTERSECTION
                    || agents.transit_mode[agent] != MODE_CAR
                {
                    continue;
                }
                let render_id = agents.render_id[agent];
                let watch = self.watched.entry(render_id).or_insert(Watch {
                    lane_id,
                    anchor_d: lane_d,
                    since_s: now,
                    reported: false,
                    seen_scan: self.scan,
                });
                watch.seen_scan = self.scan;
                if watch.lane_id != lane_id || (lane_d - watch.anchor_d).abs() > MOVED_EPS_M {
                    *watch = Watch {
                        lane_id,
                        anchor_d: lane_d,
                        since_s: now,
                        reported: false,
                        seen_scan: self.scan,
                    };
                    continue;
                }
                if watch.reported || now - watch.since_s < STALL_AFTER_S {
                    continue;
                }
                watch.reported = true;
                reports.push(StallReport {
                    render_id,
                    agent,
                    node_id: lane.node_id,
                    connector: lane_id,
                    lane_d,
                    connector_length: lane.length,
                    stalled_s: now - watch.since_s,
                    cause: stall_cause(agents, lanes, lane_id, slot, agent),
                });
            }
        }
        let scan = self.scan;
        self.watched.retain(|_, watch| watch.seen_scan == scan);
        reports
    }
}

fn stall_cause(
    agents: &AgentSystem,
    lanes: &LaneSystem,
    lane_id: usize,
    slot: usize,
    agent: usize,
) -> StallCause {
    let lane = &lanes.lanes[lane_id];
    let bucket = &agents.lane_buckets[lane_id];
    if waits_at_connector_end(bucket[slot].0, lane.length) {
        return exit_cause(agents, lanes, lane_id);
    }
    if let Some(&(leader_d, leader)) = bucket.get(slot + 1) {
        return StallCause::QueuedBehind {
            leader_render_id: agents.render_id[leader],
            leader_d,
        };
    }
    StallCause::Unexplained {
        speed: agents.speed[agent],
    }
}

// Mirrors the exit check in `exit_connector_lane`: the exit lane's entry slot must be clear.
fn exit_cause(agents: &AgentSystem, lanes: &LaneSystem, lane_id: usize) -> StallCause {
    let Some(&exit_lane) = lanes.lanes[lane_id].next_lanes.first() else {
        return StallCause::NoExit;
    };
    let bucket = agents
        .lane_buckets
        .get(exit_lane)
        .map_or(&[][..], Vec::as_slice);
    if lane_attach_slot_clear(bucket, 0.0) {
        return StallCause::ExitClaimLost { exit_lane };
    }
    // The bucket is sorted by distance, so the first car is the one nearest the entry.
    let (blocker_d, blocker) = bucket[0];
    StallCause::ExitOccupied {
        exit_lane,
        blocker_render_id: agents.render_id[blocker],
        blocker_d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::TRANSIT_NETWORK;
    use crate::simulation::network::lanes::{Lane, LaneSuccessors};
    use godot::prelude::Vector3;

    // Connectors 0 and 2 at node 7 exit into road lanes 1 and 3.
    fn lanes() -> LaneSystem {
        let mut lanes = LaneSystem::new();
        for (edge_id, length, next) in [
            (usize::MAX, 10.0, Some(1)),
            (0, 100.0, None),
            (usize::MAX, 10.0, Some(3)),
            (1, 100.0, None),
        ] {
            lanes.lanes.push(Lane {
                edge_id,
                node_id: if edge_id == usize::MAX { 7 } else { usize::MAX },
                geometry: vec![Vector3::ZERO, Vector3::new(length, 0.0, 0.0)],
                cum_dist: vec![0.0, length],
                length,
                next_lanes: next.map(LaneSuccessors::one).unwrap_or_default(),
                ..Default::default()
            });
        }
        lanes
    }

    fn car(agents: &mut AgentSystem, lane_id: usize, lane_d: f32, transit: u8) -> usize {
        let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.transit_mode[agent] = MODE_CAR;
        agents.transit[agent] = transit;
        agents.current_lane_id[agent] = lane_id;
        agents.lane_distance[agent] = lane_d;
        agent
    }

    // Publishes the occupancy snapshot the tick would leave behind.
    fn publish(agents: &mut AgentSystem, lane_count: usize) {
        agents.lane_buckets = vec![Vec::new(); lane_count];
        agents.dirty_lanes.clear();
        for agent in 0..agents.agents.len() {
            let lane_id = agents.current_lane_id[agent];
            if agents.lane_buckets[lane_id].is_empty() {
                agents.dirty_lanes.push(lane_id);
            }
            let lane_d = agents.lane_distance[agent];
            agents.lane_buckets[lane_id].push((lane_d, agent));
        }
        for bucket in &mut agents.lane_buckets {
            bucket.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        agents.lane_bucket_snapshot_valid = true;
    }

    #[test]
    fn held_car_is_reported_once_and_moving_car_never() {
        let lanes = lanes();
        let mut agents = AgentSystem::new();
        let held = car(&mut agents, 0, 10.0, TRANSIT_INTERSECTION);
        let blocker = car(&mut agents, 1, 1.0, TRANSIT_NETWORK);
        let moving = car(&mut agents, 2, 0.0, TRANSIT_INTERSECTION);
        let mut detector = JunctionStallDetector::default();
        let mut reports = Vec::new();
        for step in 0..40 {
            agents.sim_time = step as f32 * 0.5;
            // The moving car advances 1 m per simulated second and wraps on its connector.
            agents.lane_distance[moving] = (agents.sim_time) % 9.0;
            publish(&mut agents, lanes.lanes.len());
            reports.extend(detector.scan(&agents, &lanes));
        }
        assert_eq!(reports.len(), 1, "{reports:?}");
        let report = reports[0];
        assert_eq!(report.render_id, agents.render_id[held]);
        assert_eq!((report.node_id, report.connector), (7, 0));
        assert!(report.stalled_s >= STALL_AFTER_S && report.stalled_s < STALL_AFTER_S + 1.0);
        assert_eq!(
            report.cause,
            StallCause::ExitOccupied {
                exit_lane: 1,
                blocker_render_id: agents.render_id[blocker],
                blocker_d: 1.0,
            }
        );
        assert!(report.to_string().contains("exit lane 1 entry occupied"));

        // Leaving the connector forgets the car; a new stall is reported again.
        agents.transit[held] = TRANSIT_NETWORK;
        agents.current_lane_id[held] = 1;
        agents.lane_distance[held] = 50.0;
        agents.sim_time += 1.0;
        publish(&mut agents, lanes.lanes.len());
        assert!(detector.scan(&agents, &lanes).is_empty());
        assert!(!detector.watched.contains_key(&agents.render_id[held]));
    }

    #[test]
    fn causes_name_the_blocker() {
        let lanes = lanes();
        let mut agents = AgentSystem::new();
        // Connector 0: a car mid-way queued behind a car at its end; the exit is clear.
        let queued = car(&mut agents, 0, 2.0, TRANSIT_INTERSECTION);
        let leader = car(&mut agents, 0, 10.0, TRANSIT_INTERSECTION);
        let mut detector = JunctionStallDetector::default();
        let mut reports = Vec::new();
        for step in 0..=11 {
            agents.sim_time = step as f32;
            publish(&mut agents, lanes.lanes.len());
            reports.extend(detector.scan(&agents, &lanes));
        }
        reports.sort_by_key(|report| report.agent);
        assert_eq!(reports.len(), 2, "{reports:?}");
        assert_eq!(reports[0].agent, queued);
        assert_eq!(
            reports[0].cause,
            StallCause::QueuedBehind {
                leader_render_id: agents.render_id[leader],
                leader_d: 10.0,
            }
        );
        assert_eq!(reports[1].agent, leader);
        assert_eq!(reports[1].cause, StallCause::ExitClaimLost { exit_lane: 1 });

        // A clock that goes backwards, as after a load, starts over.
        agents.sim_time = 0.0;
        assert!(detector.scan(&agents, &lanes).is_empty());
        assert_eq!(detector.watched.len(), 2);
        assert!(detector.watched.values().all(|watch| !watch.reported));
    }
}
