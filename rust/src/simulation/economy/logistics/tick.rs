// SPDX-License-Identifier: GPL-3.0-or-later

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

    /// How far the freight route cache is valid for the current topology, as a save stores it
    /// (`SAVE-08`): 2 when its topology and metric revisions are current, 1 when only the
    /// metric changed since (the next refresh clears the routes and keeps the request
    /// failures), 0 when the topology changed (the next refresh clears both). The cache itself
    /// is not saved: its routes are free-flow costs of the saved network. O(1).
    pub(crate) fn route_cache_validity(
        &self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
    ) -> u8 {
        let cch = &transit_network.cch_graph;
        let topology = transit_network.cch_current()
            && self.freight_route_cache_building_revision == allocator.building_ref_revision()
            && self.freight_route_cache_entrance_revision == allocator.entrance_ref_revision()
            && self.freight_route_cache_cch_generation == cch.build_generation;
        match (
            topology,
            self.freight_route_cache_metric_revision == cch.metric_revision,
        ) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        }
    }

    /// Takes the loaded game's revisions as the cache's when the saved game's cache was valid
    /// ([`Self::route_cache_validity`]), so the first refresh after a load keeps the saved
    /// request failures as the saved game's next refresh did. Before (`SAVE-08`) the loaded
    /// cache never matched and the first refresh cleared them. O(1).
    pub(crate) fn restore_route_cache_validity(
        &mut self,
        validity: u8,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
    ) {
        if validity == 0 {
            return;
        }
        let cch = &transit_network.cch_graph;
        self.freight_route_cache_building_revision = allocator.building_ref_revision();
        self.freight_route_cache_entrance_revision = allocator.entrance_ref_revision();
        self.freight_route_cache_cch_generation = cch.build_generation;
        if validity == 2 {
            self.freight_route_cache_metric_revision = cch.metric_revision;
        }
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
