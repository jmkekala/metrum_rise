//! Hourly logistics orchestration.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::{debug, debug_log};
use rayon::prelude::*;
use std::time::Instant;

use super::data::ShipmentSystem;
use super::planning::FreightPlanningContext;

impl ShipmentSystem {
    /// Advances freight deliveries and opens new bounded restock jobs on one operational hour.
    pub fn hourly_tick(
        &mut self,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        treasury_balance: &mut f64,
    ) {
        let timing_enabled = debug::category_enabled("economy");
        let total_start = Instant::now();
        let mut phase_start = total_start;
        let shipments_before = self.shipments.len();
        let agents_before = agents.len();
        self.refresh_freight_route_cache(allocator, transit_network);
        let route_cache_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.decrement_building_cooldowns(allocator);
        let cooldowns_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let mut planning = FreightPlanningContext::build(self, allocator, graph);
        let planning_context_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.create_profile_input_shipments(
            allocator,
            transit_network,
            graph,
            minute_of_day,
            &mut planning,
            treasury_balance,
        );
        let input_shipments_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.create_profile_output_exports(
            allocator,
            transit_network,
            graph,
            minute_of_day,
            &mut planning,
        );
        let output_exports_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        planning.finish(self);
        let finish_planning_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.progress_shipments(allocator, agents, transit_network, graph, treasury_balance);
        let progress_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.shipments.retain(|shipment| shipment.status.is_open());
        let retain_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                "economy",
                "logistics_hour_detail minute={} buildings={} agents_before={} agents_after={} shipments_before={} shipments_after={} route_cache_ms={:.3} cooldowns_ms={:.3} planning_context_ms={:.3} input_shipments_ms={:.3} output_exports_ms={:.3} finish_planning_ms={:.3} progress_ms={:.3} retain_ms={:.3} total_ms={:.3}",
                minute_of_day,
                allocator.buildings.len(),
                agents_before,
                agents.len(),
                shipments_before,
                self.shipments.len(),
                route_cache_ms,
                cooldowns_ms,
                planning_context_ms,
                input_shipments_ms,
                output_exports_ms,
                finish_planning_ms,
                progress_ms,
                retain_ms,
                total_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
    }

    pub(super) fn decrement_building_cooldowns(&self, allocator: &mut BuildingAllocator) {
        allocator.buildings.par_iter_mut().for_each(|building| {
            if building.shipment_cooldown_hours > 0 {
                building.shipment_cooldown_hours -= 1;
            }
        });
    }

    fn refresh_freight_route_cache(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
    ) {
        let building_revision = allocator.building_ref_revision();
        let entrance_revision = allocator.entrance_ref_revision();
        let cch_generation = transit_network.cch_graph.build_generation;
        if self.freight_route_cache_building_revision != building_revision
            || self.freight_route_cache_entrance_revision != entrance_revision
            || self.freight_route_cache_cch_generation != cch_generation
        {
            self.freight_route_cache.clear();
            if !self.request_failures.is_empty() {
                debug_log!(
                    "economy",
                    "freight request failures cleared after route topology changed count={}",
                    self.request_failures.len()
                );
                self.request_failures.clear();
            }
            self.freight_route_cache_building_revision = building_revision;
            self.freight_route_cache_entrance_revision = entrance_revision;
            self.freight_route_cache_cch_generation = cch_generation;
        }
    }
}
