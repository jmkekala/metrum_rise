// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-agent movement state machine for the agent tick loop.

mod access_state;
mod building;
mod network;
mod replan_watchdog;

use super::super::{
    MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_IMMIGRATING,
    TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use super::claims::LaneClaimContext;
use super::slices::MovementSlices;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::LaneEntry;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::economy::agents::walker_gait::{walker_is_child, walker_size};
use crate::simulation::economy::definitions::{
    OperationalClockRuntimeTuning, RuntimeEconomyCatalog,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use access_state::{handle_access_egress, handle_access_ingress};
use building::handle_in_building;
use network::handle_network_movement;
pub(super) use network::retry_after_s;
use std::sync::atomic::AtomicU32;

const BUILDING_REPLAN_DELAY_S: f32 = 30.0;
const NETWORK_REPLAN_DELAY_S: f32 = 5.0;
const WALK_PHASE_CYCLES_PER_METER: f32 = 0.8;
// The cap keeps fast-forwarded legs from flailing: cycles a tick of a walker of the model's size,
// over its drawn size for others, so a child's quicker steps keep their rate. At 1x a 60 Hz tick
// moves the fastest walker (AGENT_WALK_SPEED_MS at the top pace, 4.72 m/s) 0.079 m, 0.063
// of a cycle before its size: under the cap, so a walker's feet keep pace with its body at any
// pace and size.
const WALK_PHASE_MAX_ADVANCE_PER_TICK: f32 = 0.07;

// Cycles a walker of drawn size `stride` (`walker_gait::walker_size`) steps over `distance_m`.
#[inline(always)]
fn walk_phase_advance(distance_m: f32, stride: f32) -> f32 {
    (distance_m.max(0.0) * WALK_PHASE_CYCLES_PER_METER).min(WALK_PHASE_MAX_ADVANCE_PER_TICK)
        / stride
}

fn transit_mode_label(mode: u8) -> &'static str {
    if mode == MODE_CAR { "car" } else { "foot" }
}

impl AgentSystem {
    /// Core agent movement logic (FSM and physics).
    /// Safety: Caller must ensure disjoint access to agent SoA via `MovementSlices`.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn process_agent_movement(
        i: usize,
        delta: f32,
        sim_time: f32,
        time: &TimeSystem,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &AtomicU32,
        lane_buckets: &[Vec<LaneEntry>],
        lane_claims: &LaneClaimContext<'_>,
        operational_clock: &OperationalClockRuntimeTuning,
        economy_catalog: &RuntimeEconomyCatalog,
        slices: &MovementSlices,
    ) {
        // Safety: index i is unique to this thread via par_iter.
        unsafe {
            if slices.parking_trip.get(i).ready {
                slices.note_frozen_parking_agent(i);
                return;
            }
            let s_cur_n = &slices.cur_n;
            let s_tmode = &slices.tmode;
            let s_walk_phase = &slices.walk_phase;
            let s_transit = &slices.transit;
            let s_pos_x = &slices.pos_x;
            let s_pos_y = &slices.pos_y;
            let s_lane_change_from_lane = &slices.lane_change_from_lane;
            let s_lane_change_start_d = &slices.lane_change_start_d;
            let s_lane_change_length = &slices.lane_change_length;
            let s_overtake_blocked_time = &slices.overtake_blocked_time;

            *s_cur_n.get_mut(i) = graph.get_valid_node(*s_cur_n.get(i));
            if *s_transit.get(i) != TRANSIT_NETWORK && *s_lane_change_from_lane.get(i) != u32::MAX {
                *s_lane_change_from_lane.get_mut(i) = u32::MAX;
                *s_lane_change_start_d.get_mut(i) = 0.0;
                *s_lane_change_length.get_mut(i) = 0.0;
                *s_overtake_blocked_time.get_mut(i) = 0.0;
            }

            let was_walking = *s_tmode.get(i) != MODE_CAR;
            let previous_x = *s_pos_x.get(i);
            let previous_z = *s_pos_y.get(i);
            let pulling_out =
                slices.parking_trip.get(i).phase == crate::simulation::parking::ParkingTripPhase::PullOut;

            match *s_transit.get(i) {
                TRANSIT_IN_BUILDING => {
                    handle_in_building(
                        i,
                        sim_time,
                        time,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                        operational_clock,
                        economy_catalog,
                        slices,
                    );
                }

                TRANSIT_ACCESS_EGRESS => {
                    if handle_access_egress(
                        i,
                        delta,
                        sim_time,
                        allocator,
                        transit_network,
                        graph,
                        lane_buckets,
                        lane_claims,
                        pathfind_count,
                        slices,
                    ) {
                        // Left untouched for the serial part, which runs the whole step: the
                        // writes above are idempotent (`PERF-33`).
                        super::TripQueryScratch::note_deferred_egress(i);
                        return;
                    }
                }

                TRANSIT_NETWORK | TRANSIT_IMMIGRATING | TRANSIT_INTERSECTION => {
                    handle_network_movement(
                        i,
                        delta,
                        sim_time,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                        lane_buckets,
                        lane_claims,
                        slices,
                    );
                }

                TRANSIT_ACCESS_INGRESS => {
                    handle_access_ingress(
                        i,
                        delta,
                        sim_time,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                        slices,
                    );
                }
                _ => {
                    *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                }
            }

            slices.note_parking_step(i, pulling_out, sim_time);

            // Visual-only state: derive the walk cycle from actual movement after
            // the FSM has advanced the agent. This keeps local access walking and
            // foot-lane movement animated even when `speed` is not persisted.
            if was_walking && *s_tmode.get(i) != MODE_CAR {
                let dx = *s_pos_x.get(i) - previous_x;
                let dz = *s_pos_y.get(i) - previous_z;
                let distance_m = dx.hypot(dz);
                // Each walker's stride scales with its drawn size (`walker_gait`).
                let child = walker_is_child(*slices.age_group.get(i), *slices.household_id.get(i));
                let stride = walker_size(*slices.schedule_seed.get(i), child);
                let phase_delta = walk_phase_advance(distance_m, stride);
                if phase_delta > 0.0 {
                    let phase = *s_walk_phase.get(i);
                    *s_walk_phase.get_mut(i) = (phase + phase_delta) % 1.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WALK_PHASE_MAX_ADVANCE_PER_TICK, walk_phase_advance};
    use crate::config::AGENT_WALK_SPEED_MS;

    #[test]
    fn walk_phase_advance_caps_fast_forwarded_ticks() {
        let realtime_delta = 1.0 / 60.0;
        let normal = walk_phase_advance(AGENT_WALK_SPEED_MS * realtime_delta, 1.0);
        assert!(normal > 0.0);
        assert!(normal < WALK_PHASE_MAX_ADVANCE_PER_TICK);

        let fast_forward = walk_phase_advance(AGENT_WALK_SPEED_MS * realtime_delta * 60.0, 1.0);
        assert_eq!(fast_forward, WALK_PHASE_MAX_ADVANCE_PER_TICK);

        // The fastest walker at 1x is not capped, at any drawn size from the smallest
        // child to the tallest adult: its feet match its body, its cycles a metre are the
        // model's over its size (`RENDER-72`: shorter legs, quicker steps).
        use crate::simulation::economy::agents::walker_gait::{
            CHILD_SIZE, PACE_SPREAD, SIZE_SPREAD,
        };
        let top_pace = 1.0 + PACE_SPREAD;
        let sidewalk = AGENT_WALK_SPEED_MS * top_pace * realtime_delta;
        for stride in [CHILD_SIZE[0], CHILD_SIZE[1], 1.0 - SIZE_SPREAD, 1.0 + SIZE_SPREAD] {
            let cycles = walk_phase_advance(sidewalk, stride);
            let unclamped = sidewalk * super::WALK_PHASE_CYCLES_PER_METER / stride;
            assert!((cycles - unclamped).abs() < 1e-7, "{stride}: {cycles} {unclamped}");
        }
        // Fast-forwarded, a child's legs keep their quicker rate against an adult's.
        let child = walk_phase_advance(1.0, CHILD_SIZE[0]);
        let adult = walk_phase_advance(1.0, 1.0 + SIZE_SPREAD);
        assert!((child / adult - (1.0 + SIZE_SPREAD) / CHILD_SIZE[0]).abs() < 1e-5);
    }
}
