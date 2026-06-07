//! Per-agent movement state machine for the agent tick loop.

mod access_state;
mod building;
mod network;

use super::super::{
    MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_IMMIGRATING,
    TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use super::claims::LaneClaimContext;
use super::slices::MovementSlices;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::economy::definitions::{
    OperationalClockRuntimeTuning, RuntimeEconomyCatalog,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use access_state::{handle_access_egress, handle_access_ingress};
use building::handle_in_building;
use network::handle_network_movement;
use std::sync::atomic::AtomicU32;

const BUILDING_REPLAN_DELAY_S: f32 = 30.0;
const NETWORK_REPLAN_DELAY_S: f32 = 5.0;

fn transit_mode_label(mode: u8) -> &'static str {
    if mode == MODE_CAR { "car" } else { "foot" }
}

impl AgentSystem {
    /// Core agent movement logic (FSM and physics).
    /// Safety: Caller must ensure disjoint access to agent SoA via `MovementSlices`.
    #[inline(always)]
    pub(super) unsafe fn process_agent_movement(
        i: usize,
        delta: f32,
        sim_time: f32,
        day_index: u32,
        minute_of_day: u16,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &AtomicU32,
        lane_buckets: &Vec<Vec<(f32, usize)>>,
        lane_claims: &LaneClaimContext<'_>,
        operational_clock: &OperationalClockRuntimeTuning,
        economy_catalog: &RuntimeEconomyCatalog,
        slices: &MovementSlices,
    ) {
        // Safety: index i is unique to this thread via par_iter.
        unsafe {
            let s_cur_n = &slices.cur_n;
            let s_tmode = &slices.tmode;
            let s_speed = &slices.speed;
            let s_walk_phase = &slices.walk_phase;
            let s_transit = &slices.transit;
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

            // Update walk animation phase if not in a vehicle.
            if *s_tmode.get(i) != MODE_CAR {
                let spd = *s_speed.get(i);
                let phase = *s_walk_phase.get(i);
                // Cycle: about 1 time per meter traveled.
                *s_walk_phase.get_mut(i) = (phase + (spd.abs() * 0.8 * delta)) % 1.0;
            }

            match *s_transit.get(i) {
                TRANSIT_IN_BUILDING => {
                    handle_in_building(
                        i,
                        sim_time,
                        day_index,
                        minute_of_day,
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
                    handle_access_egress(
                        i,
                        delta,
                        sim_time,
                        allocator,
                        transit_network,
                        graph,
                        lane_buckets,
                        lane_claims,
                        slices,
                    );
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
        }
    }
}
