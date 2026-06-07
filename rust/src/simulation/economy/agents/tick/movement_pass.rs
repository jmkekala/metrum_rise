//! Parallel dispatch phase for the movement state machine.

use super::claims::LaneClaimContext;
use super::runtime::dispatch_agents;
use super::slices::{MovementSlices, RawSlice};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

impl AgentSystem {
    /// Dispatches the parallel per-agent movement state machine.
    pub(super) fn dispatch_movement_pass(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
        day_index: u32,
        minute_of_day: u16,
        n: usize,
    ) {
        self.prepare_claim_serial_agents(allocator, transit_network, graph, delta, n);

        let slices = MovementSlices {
            home: RawSlice::new(&mut self.agents.home_building),
            work: RawSlice::new(&mut self.agents.work_building),
            pos_x: RawSlice::new(&mut self.agents.pos_x),
            pos_y: RawSlice::new(&mut self.agents.pos_y),
            activity: RawSlice::new(&mut self.agents.activity),
            transit: RawSlice::new(&mut self.agents.transit),
            happiness: RawSlice::new(&mut self.agents.happiness),
            jstart: RawSlice::new(&mut self.agents.journey_start_time),
            schedule_seed: RawSlice::new(&mut self.agents.schedule_seed),
            cached_commute_minutes: RawSlice::new(&mut self.agents.cached_commute_minutes),
            next_commute_refresh_time: RawSlice::new(&mut self.agents.next_commute_refresh_time),
            next_departure_day: RawSlice::new(&mut self.agents.next_departure_day),
            next_departure_minute: RawSlice::new(&mut self.agents.next_departure_minute),
            next_departure_origin: RawSlice::new(&mut self.agents.next_departure_origin_building),
            next_departure_target: RawSlice::new(&mut self.agents.next_departure_target_building),
            next_departure_activity: RawSlice::new(&mut self.agents.next_departure_activity),
            cached_schedule_work_building: RawSlice::new(
                &mut self.agents.cached_schedule_work_building,
            ),
            cached_work_profile_index: RawSlice::new(&mut self.agents.cached_work_profile_index),
            cur_b: RawSlice::new(&mut self.agents.current_building),
            tgt_b: RawSlice::new(&mut self.agents.target_building),
            planned_tgt_b: RawSlice::new(&mut self.agents.planned_target_building),
            cur_n: RawSlice::new(&mut self.agents.current_node),
            planned_attach_n: RawSlice::new(&mut self.agents.planned_attach_node),
            planned_detach_n: RawSlice::new(&mut self.agents.planned_detach_node),
            planned_attach_lane: RawSlice::new(&mut self.agents.planned_attach_lane_id),
            planned_detach_lane: RawSlice::new(&mut self.agents.planned_detach_lane_id),
            planned_attach_lane_d: RawSlice::new(&mut self.agents.planned_attach_lane_d),
            planned_detach_lane_d: RawSlice::new(&mut self.agents.planned_detach_lane_d),
            access_flags: RawSlice::new(&mut self.agents.access_flags),
            next_replan_time: RawSlice::new(&mut self.agents.next_replan_time),
            cur_e: RawSlice::new(&mut self.agents.current_edge),
            lane_id: RawSlice::new(&mut self.agents.current_lane_id),
            lane_d: RawSlice::new(&mut self.agents.lane_distance),
            lane_change_from_lane: RawSlice::new(&mut self.agents.lane_change_from_lane_id),
            lane_change_start_d: RawSlice::new(&mut self.agents.lane_change_start_d),
            lane_change_length: RawSlice::new(&mut self.agents.lane_change_length_m),
            overtake_blocked_time: RawSlice::new(&mut self.agents.overtake_blocked_time_s),
            overtake_cooldown: RawSlice::new(&mut self.agents.overtake_cooldown_s),
            tmode: RawSlice::new(&mut self.agents.transit_mode),
            planned_activity: RawSlice::new(&mut self.agents.planned_activity),
            path: RawSlice::new(&mut self.agents.current_path),
            path_idx: RawSlice::new(&mut self.agents.current_path_index),
            has_car: RawSlice::new(&mut self.agents.has_car),
            speed: RawSlice::new(&mut self.agents.speed),
            walk_phase: RawSlice::new(&mut self.agents.walk_phase),
        };

        let lane_buckets = &self.lane_buckets;
        let lane_claims =
            LaneClaimContext::new(&self.lane_attach_claimed, &self.claim_serial_agents);
        let sim_time = self.sim_time;
        let economy_tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let economy_catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));

        dispatch_agents(n, |i| unsafe {
            if self.claim_serial_agents[i] {
                return;
            }
            Self::process_agent_movement(
                i,
                delta,
                sim_time,
                day_index,
                minute_of_day,
                allocator,
                transit_network,
                graph,
                &self.pathfind_count,
                lane_buckets,
                &lane_claims,
                &economy_tuning.operational_clock,
                &economy_catalog,
                &slices,
            );
        });

        for i in 0..n {
            if !self.claim_serial_agents[i] {
                continue;
            }
            unsafe {
                Self::process_agent_movement(
                    i,
                    delta,
                    sim_time,
                    day_index,
                    minute_of_day,
                    allocator,
                    transit_network,
                    graph,
                    &self.pathfind_count,
                    lane_buckets,
                    &lane_claims,
                    &economy_tuning.operational_clock,
                    &economy_catalog,
                    &slices,
                );
            }
        }
    }
}
