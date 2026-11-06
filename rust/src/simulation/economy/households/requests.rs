// SPDX-License-Identifier: GPL-2.0-only

//! The requests households queue at their phases (`SIM-02-J`,
//! `docs/simulation.md#request-queues`), and the destination index restock and visit requests
//! read.
//!
//! After its hour a household queues its rehousing while it has no usable home, its restock
//! when its stock calls for one and a member is free to shop, and the visible service visit
//! the hour's draw gives it. The
//! `housing requests`, `restock requests` and `visit requests` queues serve them a few per slot;
//! a household waits at most once in each.

use super::destinations::{DestinationFold, DestinationFront};
use super::metrics::{
    household_demand_profile, household_has_independent_member, household_is_housed,
    household_supply_resource_runtime_id,
};
use super::replenishment::{first_eligible_shopper, restock_due};
use super::service_visits::{personal_services_resource, service_visit_rate};
use super::{Household, HouseholdSystem};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

pub(super) use crate::simulation::core::request_queue::QueueStats;

// What a slot's request checks read, loaded once per slot.
struct PhaseRequests {
    absolute_hour: u32,
    building_count: usize,
    check_interval: u32,
    trigger_days: f32,
    visit_rate_per_resident: f32,
    // Whether the hour's destination front lists stores and service stores; `true` before
    // the hour has one.
    stores_listed: bool,
    services_listed: bool,
}

impl HouseholdSystem {
    /// Queues the requests of the households in hourly phase `slot`, after their hour: the
    /// rehousing of one with an independent member and no usable home, the restock of one
    /// whose stock calls for it while a member is free to shop, and the visible visit the
    /// hour's draw gives. A household already waiting in a queue keeps its place there.
    /// O(members of the phase households).
    pub(crate) fn queue_household_requests(
        &mut self,
        phase_households: &[Handle<Household>],
        slot: u64,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
    ) {
        if phase_households.is_empty() {
            return;
        }
        let checks = self.phase_requests(slot, allocator);
        for &handle in phase_households {
            if let Some(household_id) = self.handles.resolve(handle) {
                self.queue_requests_of(household_id, handle, slot, &checks, agents, allocator);
            }
        }
    }

    /// Queues every household's requests at once, in household order, as their hourly phases
    /// in the hour of `slot` would.
    #[cfg(test)]
    pub(super) fn queue_every_household_for_test(
        &mut self,
        slot: u64,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
    ) {
        self.handles.register_appended(self.households.len());
        agents.register_appended_household_members();
        let checks = self.phase_requests(slot, allocator);
        for household_id in 0..self.households.len() {
            let handle = self.handles.handle(household_id);
            self.queue_requests_of(household_id, handle, slot, &checks, agents, allocator);
        }
    }

    fn phase_requests(&self, slot: u64, allocator: &BuildingAllocator) -> PhaseRequests {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let hour = slot / SLOTS_PER_HOUR;
        let [stores_listed, services_listed] = self.destinations.listed(hour).unwrap_or([true; 2]);
        PhaseRequests {
            absolute_hour: hour as u32,
            building_count: allocator.buildings.len(),
            check_interval: u32::from(
                tuning
                    .operational_clock
                    .household_replenishment_check_interval_hours,
            ),
            trigger_days: household_demand_profile(&catalog).reorder_threshold_days,
            visit_rate_per_resident: service_visit_rate(&catalog),
            stores_listed,
            services_listed,
        }
    }

    fn queue_requests_of(
        &mut self,
        household_id: usize,
        handle: Handle<Household>,
        slot: u64,
        checks: &PhaseRequests,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
    ) {
        let household = &self.households[household_id];
        if household.member_count > 0
            && household_has_independent_member(household)
            && !household_is_housed(household, allocator)
        {
            self.rehousing.push(slot, handle);
        }
        if household.stock_days == 0.0 && checks.stores_listed && household.cooldown_hours > 0 {
            self.replenishment_hour.count_urgent_cooldown_skip();
        }
        if restock_due(
            household,
            checks.building_count,
            checks.absolute_hour,
            checks.check_interval,
            checks.stores_listed,
            checks.trigger_days,
        ) {
            // A household with no member free to shop waits for one without a request, as
            // the queue would have left it; it checks again at its next phase.
            if first_eligible_shopper(agents, household_id, household) == usize::MAX {
                self.wait_for_shopper(household_id);
            } else {
                self.restock_requests.push(slot, handle);
            }
        }
        if checks.services_listed {
            self.request_service_visit(
                household_id,
                slot,
                checks.building_count,
                checks.visit_rate_per_resident,
            );
        }
    }

    /// Lends the destination front for the hour of `slot`, built at once when the hour has
    /// none for the current road topology; `destinations.restore_front` takes it back.
    pub(super) fn destination_front(
        &mut self,
        slot: u64,
        allocator: &mut BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        parking: Option<&crate::simulation::parking::ParkingSystem>,
    ) -> DestinationFront {
        allocator
            .building_handles
            .register_appended(allocator.buildings.len());
        self.destination_topology.refresh(network, graph);
        let fold = DestinationFold {
            allocator,
            graph,
            catalog,
            topology: &self.destination_topology,
            supplies: household_supply_resource_runtime_id(catalog),
            services: personal_services_resource(catalog),
            parking,
        };
        self.destinations.take_front(slot / SLOTS_PER_HOUR, &fold)
    }

    /// Slot `slot`'s chunk of the hour-long job board sweep that the next hour's job searches
    /// read; the caller registers appended buildings' handles first. O(B/960) per slot.
    pub(crate) fn advance_job_board(&mut self, slot: u64, allocator: &BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.job_board.advance(slot, allocator, &catalog);
    }

    /// Stamps a loaded destination roll with the loaded road graph's components, which label
    /// as the saved graph did. O(nodes + edges).
    pub(crate) fn adopt_loaded_topology(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        self.destination_topology.refresh(network, graph);
        self.destinations
            .adopt_loaded_topology(&self.destination_topology);
    }

    /// Slot `slot`'s chunk of the hour-long destination sweep that the next hour's restock and
    /// visit requests read; the caller registers appended buildings' handles first. O(B/960 ×
    /// outputs) per slot, the hour's last slot O(E log E) for E destinations.
    pub(crate) fn advance_destination_sweep(
        &mut self,
        slot: u64,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        parking: Option<&crate::simulation::parking::ParkingSystem>,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.destination_topology.refresh(network, graph);
        let fold = DestinationFold {
            allocator,
            graph,
            catalog: &catalog,
            topology: &self.destination_topology,
            supplies: household_supply_resource_runtime_id(&catalog),
            services: personal_services_resource(&catalog),
            parking,
        };
        self.destinations.advance(slot, &fold);
    }
}
