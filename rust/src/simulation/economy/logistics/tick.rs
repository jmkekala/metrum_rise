// SPDX-License-Identifier: GPL-2.0-only

//! The logistics hour and the freight route cache's revision check.
//!
//! Freight planning runs in the `freight requests` queue at each building's phase, cargo
//! loads when a slot's plans are made, carriers leave at their own slots and every leg times
//! out at one of its checks (`SIM-02-J`). The hour only advances the clock that leg timeouts
//! count; freight cooldowns are slot deadlines (`requests.rs`).

use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;

use super::data::ShipmentSystem;

impl ShipmentSystem {
    /// Starts the next logistics hour, the clock of carrier leg timeouts
    /// (`Shipment::leg_started_hour`), after logging the finished hour's request queue counts.
    /// O(1); test builds also compare the maintained reservations with a fresh build, once an
    /// hour as before, except in timing runs ([`crate::debug::work::city_checks`]).
    pub fn hourly_tick(&mut self) {
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            self.assert_reservations_current();
        }
        self.close_request_stats_hour();
        self.hour_ticks += 1;
    }

    /// Invalidates dispatch and forecast route tables together after topology changes.
    pub(super) fn refresh_freight_route_cache(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
    ) {
        let building_revision = allocator.building_ref_revision();
        let entrance_revision = allocator.entrance_ref_revision();
        let cch_generation = transit_network.cch_graph.build_generation;
        let metric_revision = transit_network.cch_graph.metric_revision;
        if self.freight_route_cache_building_revision != building_revision
            || self.freight_route_cache_entrance_revision != entrance_revision
            || self.freight_route_cache_cch_generation != cch_generation
        {
            if self.freight_route_cache_cch_generation != cch_generation
                || self.freight_route_cache_metric_revision != metric_revision
            {
                self.freight_route_cache.clear();
            } else {
                // Site edits invalidate indexed entrances, not the unchanged road network.
                self.freight_route_cache.clear_buildings();
            }
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
            self.freight_route_cache_metric_revision = metric_revision;
        } else if self.freight_route_cache_metric_revision != metric_revision {
            // A speed/cost edit invalidates prices, not the request's failure history.
            self.freight_route_cache.clear();
            self.freight_route_cache_metric_revision = metric_revision;
        }
    }
}
