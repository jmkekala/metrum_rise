// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic lane reservations and dynamic-handoff dispatch for movement.

use super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_ACCESS_EGRESS,
    TRANSIT_IMMIGRATING, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use super::access::planned_detach_is_legal;
use super::traffic::{
    LaneChangeParams, choose_lane_change, connector_turn_speed, junction_car_speed,
    lane_change_finished, planned_lane_change_target,
};
use crate::config::CAR_JUNCTION_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::data::{AgentSystem, AgentVec, MovementClaimMode};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const CLAIM_REACH_EPS_M: f32 = 0.05;
// Batch independent classification and fixed lateral reservations to amortize Rayon dispatch.
const CLAIM_CLASSIFICATION_CHUNK: usize = 4_096;

/// Per-tick lane reservations and dispatch ownership.
pub(in crate::simulation::economy::agents::tick) struct LaneClaimContext<'a> {
    owners: &'a [AtomicUsize],
    modes: &'a [MovementClaimMode],
}

impl<'a> LaneClaimContext<'a> {
    /// Builds a context after fixed lateral reservations have completed.
    pub(in crate::simulation::economy::agents::tick) fn new(
        owners: &'a [AtomicUsize],
        modes: &'a [MovementClaimMode],
    ) -> Self {
        Self { owners, modes }
    }

    /// Returns whether an agent may acquire an unreserved lane during movement.
    #[inline(always)]
    fn agent_is_serial(&self, agent_idx: usize) -> bool {
        self.modes
            .get(agent_idx)
            .is_some_and(|&mode| mode == MovementClaimMode::Dynamic)
    }

    /// Returns whether this step can include a fixed or dynamically discovered lateral move.
    #[inline(always)]
    pub(in crate::simulation::economy::agents::tick) fn agent_may_change_lanes(
        &self,
        agent_idx: usize,
    ) -> bool {
        self.modes
            .get(agent_idx)
            .is_some_and(|&mode| mode != MovementClaimMode::Parallel)
    }

    /// Uses an existing reservation, or acquires a fresh lane in the serial handoff phase.
    /// An owner may reuse its reservation, including an entry followed by a frontage detach.
    #[inline(always)]
    pub(in crate::simulation::economy::agents::tick) fn claim_lane(
        &self,
        agent_idx: usize,
        lane_id: usize,
    ) -> bool {
        let Some(owner) = self.owners.get(lane_id) else {
            return false;
        };
        let current = owner.load(Ordering::Relaxed);
        if current != usize::MAX {
            return current == agent_idx;
        }
        let serial = self.agent_is_serial(agent_idx);
        debug_assert!(
            serial,
            "agent {agent_idx} requested an unreserved lane outside the serial handoff phase"
        );
        serial
            && owner
                .compare_exchange(usize::MAX, agent_idx, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
    }
}

// One read-only view of the current movement inputs; only lane-owner atomics are written here.
struct ClaimPreparation<'a> {
    allocator: &'a BuildingAllocator,
    network: &'a TransitNetwork,
    graph: &'a RegionGraph,
    lane_buckets: &'a [Vec<(f32, usize)>],
    owners: &'a [AtomicUsize],
    delta: f32,
}

impl AgentSystem {
    /// Reserves fixed lateral moves by minimum agent ID and marks dynamic handoffs for serial work.
    /// Lane occupancy and empty owner slots must already have been prepared for this tick.
    pub(super) fn prepare_lane_claims(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
        n: usize,
    ) {
        self.movement_claim_modes
            .resize(n, MovementClaimMode::Parallel);
        let agents = &self.agents;
        let preparation = ClaimPreparation {
            allocator,
            network: transit_network,
            graph,
            delta,
            lane_buckets: &self.lane_buckets,
            owners: &self.lane_claim_owner,
        };
        let classify = |(chunk_idx, modes): (usize, &mut [MovementClaimMode])| {
            let start = chunk_idx * CLAIM_CLASSIFICATION_CHUNK;
            for (offset, mode) in modes.iter_mut().enumerate() {
                *mode = classify_agent_claims(agents, start + offset, &preparation);
            }
        };
        if n <= CLAIM_CLASSIFICATION_CHUNK || rayon::current_num_threads() == 1 {
            classify((0, &mut self.movement_claim_modes));
        } else {
            // The commutative minimum selects the same owner regardless of worker scheduling.
            self.movement_claim_modes
                .par_chunks_mut(CLAIM_CLASSIFICATION_CHUNK)
                .enumerate()
                .for_each(classify);
        }
    }
}

#[inline(always)]
fn classify_agent_claims(
    agents: &AgentVec,
    i: usize,
    preparation: &ClaimPreparation<'_>,
) -> MovementClaimMode {
    match agents.transit[i] {
        TRANSIT_ACCESS_EGRESS => {
            if agents.transit_mode[i] == MODE_CAR {
                MovementClaimMode::Dynamic
            } else {
                MovementClaimMode::Parallel
            }
        }
        TRANSIT_NETWORK | TRANSIT_IMMIGRATING | TRANSIT_INTERSECTION => {
            classify_network_claims(agents, i, preparation)
        }
        _ => MovementClaimMode::Parallel,
    }
}

#[inline(always)]
fn classify_network_claims(
    agents: &AgentVec,
    i: usize,
    preparation: &ClaimPreparation<'_>,
) -> MovementClaimMode {
    let network = preparation.network;
    let lane_id = agents.current_lane_id[i];
    if lane_id == usize::MAX || lane_id >= network.lane_system.lanes.len() {
        return MovementClaimMode::Dynamic;
    }
    let remaining = agent_network_claim_distance(agents, i, network) * preparation.delta;
    if remaining <= 0.0 {
        return MovementClaimMode::Parallel;
    }
    let lane_d = agents.lane_distance[i];
    let lane = &network.lane_system.lanes[lane_id];
    if remaining + CLAIM_REACH_EPS_M >= (lane.length - lane_d).max(0.0) {
        return MovementClaimMode::Dynamic;
    }

    // A replan or a detach can change which lane is claimed. Keep those moves in the dynamic phase.
    let target = agents.target_building[i];
    let entrances = &preparation.allocator.entrances;
    let has_plan = (agents.access_flags[i] & ACCESS_PLAN_VALID) != 0;
    if target != usize::MAX && (target >= entrances.len() || !has_plan) {
        return MovementClaimMode::Dynamic;
    }
    if has_plan {
        let detach_lane = agents.planned_detach_lane_id[i] as usize;
        if (target == usize::MAX
            && (agents.access_flags[i] & ACCESS_FREIGHT_BORDER_DESTINATION) == 0)
            || (target < entrances.len()
                && !planned_detach_is_legal(
                    agents.transit_mode[i],
                    &entrances[target],
                    detach_lane,
                    agents.planned_detach_lane_d[i],
                    agents.planned_detach_node[i],
                    network,
                    preparation.graph,
                ))
        {
            return MovementClaimMode::Dynamic;
        }
        let detach_d = agents.planned_detach_lane_d[i];
        if lane_d + remaining + CLAIM_REACH_EPS_M >= detach_d
            && (lane_id == detach_lane
                || planned_lane_change_target(lane_id, detach_lane, lane_d, detach_d, network)
                    .is_some())
        {
            return MovementClaimMode::Dynamic;
        }
    }

    if lane.edge_id != usize::MAX
        && agents.transit_mode[i] == MODE_CAR
        && agents.transit[i] == TRANSIT_NETWORK
    {
        if let Some(target_lane) = agent_lane_change_target(agents, i, preparation) {
            let target_length = network.lane_system.lanes[target_lane].length;
            // Winning a move onto a shorter lane must not reach another handoff this tick.
            if remaining + CLAIM_REACH_EPS_M >= (target_length - lane_d.min(target_length)).max(0.0)
            {
                return MovementClaimMode::Dynamic;
            }
            preparation.owners[target_lane].fetch_min(i, Ordering::Relaxed);
            return MovementClaimMode::Lateral;
        }
    }
    MovementClaimMode::Parallel
}

// Keep the lane-change decision out of the otherwise tiny per-agent classification loop.
#[inline(never)]
fn agent_lane_change_target(
    agents: &AgentVec,
    i: usize,
    preparation: &ClaimPreparation<'_>,
) -> Option<usize> {
    if agents.lane_change_from_lane_id[i] != u32::MAX
        && !lane_change_finished(
            agents.lane_distance[i],
            agents.lane_change_start_d[i],
            agents.lane_change_length_m[i],
        )
    {
        return None;
    }
    choose_lane_change(
        LaneChangeParams {
            lane_id: agents.current_lane_id[i],
            lane_d: agents.lane_distance[i],
            speed: agents.speed[i],
            detach_lane_id: agents.planned_detach_lane_id[i] as usize,
            detach_lane_d: agents.planned_detach_lane_d[i],
            has_access_plan: (agents.access_flags[i] & ACCESS_PLAN_VALID) != 0,
            blocked_time: agents.overtake_blocked_time_s[i],
            cooldown: agents.overtake_cooldown_s[i],
        },
        preparation.network,
        preparation.lane_buckets,
    )
    .map(|choice| choice.target_lane_id)
}

fn agent_network_claim_distance(
    agents: &AgentVec,
    i: usize,
    transit_network: &TransitNetwork,
) -> f32 {
    if agents.transit_mode[i] != MODE_CAR {
        return 4.0;
    }
    if agents.transit[i] == TRANSIT_INTERSECTION {
        let lane_id = agents.current_lane_id[i];
        let turn_speed = transit_network
            .lane_system
            .lanes
            .get(lane_id)
            .map(connector_turn_speed)
            .unwrap_or(CAR_JUNCTION_SPEED_MS);
        junction_car_speed(agents.speed[i]).min(turn_speed)
    } else {
        agents.speed[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::{MODE_WALK, TRANSIT_IN_BUILDING};
    use crate::simulation::network::lanes::Lane;

    fn claim_fixture(count: usize, idle: bool) -> (AgentSystem, TransitNetwork) {
        let mut network = TransitNetwork::new();
        network.lane_system.lanes.push(Lane {
            length: 1_000.0,
            ..Default::default()
        });
        let mut agents = AgentSystem::new();
        for idx in 0..count {
            let i = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            agents.transit_mode[i] = if idx % 8 == 2 { MODE_WALK } else { MODE_CAR };
            agents.transit[i] = if idle {
                TRANSIT_IN_BUILDING
            } else {
                match idx % 8 {
                    0 => TRANSIT_IN_BUILDING,
                    1 | 2 => TRANSIT_ACCESS_EGRESS,
                    7 => TRANSIT_INTERSECTION,
                    _ => TRANSIT_NETWORK,
                }
            };
            agents.current_lane_id[i] = if idx % 8 == 6 { usize::MAX } else { 0 };
            agents.lane_distance[i] = if matches!(idx % 8, 4 | 5) {
                995.0
            } else {
                10.0
            };
            agents.speed[i] = if idx % 8 == 5 { 0.0 } else { 10.0 };
        }
        (agents, network)
    }

    #[test]
    fn claim_preparation_replaces_every_dynamic_flag() {
        let allocator = BuildingAllocator::new();
        let graph = RegionGraph::new();
        for count in [8, 1_024, 32_768] {
            let (mut agents, network) = claim_fixture(count, false);
            agents
                .movement_claim_modes
                .resize(count, MovementClaimMode::Lateral);
            agents.lane_claim_owner.push(AtomicUsize::new(usize::MAX));
            agents.prepare_lane_claims(&allocator, &network, &graph, 1.0, count);
            for (idx, &mode) in agents.movement_claim_modes.iter().enumerate() {
                let expected = if matches!(idx % 8, 1 | 4 | 6) {
                    MovementClaimMode::Dynamic
                } else {
                    MovementClaimMode::Parallel
                };
                assert_eq!(mode, expected, "agent {idx}");
            }
            assert_eq!(
                agents.lane_claim_owner[0].load(Ordering::Relaxed),
                usize::MAX
            );
            agents.transit.fill(TRANSIT_IN_BUILDING);
            agents.prepare_lane_claims(&allocator, &network, &graph, 1.0, count);
            assert!(
                agents
                    .movement_claim_modes
                    .iter()
                    .all(|&mode| mode == MovementClaimMode::Parallel)
            );
            agents.agents.clear();
            agents.prepare_lane_claims(&allocator, &network, &graph, 1.0, 0);
            assert!(agents.movement_claim_modes.is_empty());
        }
    }

    #[test]
    #[ignore = "manual matched release timing of lane-claim preparation"]
    fn benchmark_claim_preparation() {
        use std::{hint::black_box, time::Instant};

        let allocator = BuildingAllocator::new();
        let graph = RegionGraph::new();
        for count in [1_024, 16_384, 131_072, 1_048_576] {
            for idle in [true, false] {
                let (mut agents, network) = claim_fixture(count, idle);
                let mut samples = [0.0; 21];
                for _ in 0..3 {
                    agents.prepare_lane_claims(&allocator, &network, &graph, 1.0, count);
                }
                for sample in &mut samples {
                    let start = Instant::now();
                    for _ in 0..8 {
                        agents.prepare_lane_claims(
                            black_box(&allocator),
                            black_box(&network),
                            black_box(&graph),
                            black_box(1.0),
                            black_box(count),
                        );
                    }
                    *sample = start.elapsed().as_secs_f64() * 1_000.0 / 8.0;
                    black_box(&agents.movement_claim_modes);
                }
                assert_eq!(
                    agents
                        .movement_claim_modes
                        .iter()
                        .filter(|&&mode| mode == MovementClaimMode::Dynamic)
                        .count(),
                    if idle { 0 } else { count / 8 * 3 }
                );
                let checksum = agents
                    .movement_claim_modes
                    .iter()
                    .fold(0_u64, |hash, &mode| {
                        hash.wrapping_mul(31).wrapping_add(u64::from(mode as u8))
                    });
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "claim_preparation agents={count} mode={} median_ms={:.6} checksum={checksum}",
                    if idle { "idle" } else { "mixed" },
                    samples[10]
                );
            }
        }
    }
}
