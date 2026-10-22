// SPDX-License-Identifier: GPL-2.0-only

//! Core shipment storage and typed logistics state.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::scheduler::MinuteLeveler;
use crate::simulation::economy::agents::{AgentHandle, AgentSystem};
use crate::simulation::economy::definitions::{ResourceRuntimeId, load_runtime_economy_tuning};

use super::planning::FreightTopology;
use super::requests::{FreightRequest, FreightRequestStats, event_cooldown_until};
use super::reservations::ReservationViews;
use super::resource::refund_input_payment;
use super::route_cache::FreightRouteCache;
use super::supplier_index::SupplierRoll;

/// A physical freight endpoint inside the city or at an outside-world border terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShipmentEndpoint {
    /// A placed building by allocator index.
    Building(usize),
    /// An `OWA` border terminal by graph node id.
    OwaBorder(u32),
}

impl ShipmentEndpoint {
    pub(crate) fn building_id(self) -> Option<usize> {
        match self {
            ShipmentEndpoint::Building(building_id) => Some(building_id),
            ShipmentEndpoint::OwaBorder(_) => None,
        }
    }

    fn remap_building(&mut self, mapping: &HashMap<usize, usize>) {
        if let ShipmentEndpoint::Building(building_id) = self
            && let Some(&new_id) = mapping.get(building_id)
        {
            *building_id = new_id;
        }
    }

    fn touches_building(self, building_id: usize) -> bool {
        matches!(self, ShipmentEndpoint::Building(endpoint_id) if endpoint_id == building_id)
    }
}

/// Carrier class assigned to one aggregate freight job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarrierClass {
    /// Baseline road freight carrier.
    Truck,
}

impl CarrierClass {
    pub(crate) fn code(self) -> i64 {
        match self {
            CarrierClass::Truck => 0,
        }
    }

    pub(crate) fn from_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(CarrierClass::Truck),
            _ => None,
        }
    }
}

/// Lifecycle state for one freight job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShipmentStatus {
    /// Awaiting or carrying its cargo; `OWA` dispatches every job at once.
    InTransit,
    /// Delivered and settled; the empty carrier is returning to its source/base.
    Returning,
    /// Delivered and ready to be removed from the active job list.
    Fulfilled,
    /// Failed once and ready to be removed from the active job list.
    Failed,
    /// Its carrier timed out and the job was canceled.
    Expired,
}

impl ShipmentStatus {
    pub(crate) fn code(self) -> i64 {
        match self {
            ShipmentStatus::InTransit => 1,
            ShipmentStatus::Fulfilled => 2,
            ShipmentStatus::Failed => 3,
            ShipmentStatus::Expired => 4,
            ShipmentStatus::Returning => 5,
        }
    }

    pub(crate) fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(ShipmentStatus::InTransit),
            2 => Some(ShipmentStatus::Fulfilled),
            3 => Some(ShipmentStatus::Failed),
            4 => Some(ShipmentStatus::Expired),
            5 => Some(ShipmentStatus::Returning),
            _ => None,
        }
    }

    pub(crate) fn is_open(self) -> bool {
        matches!(self, ShipmentStatus::InTransit | ShipmentStatus::Returning)
    }

    pub(crate) fn reserves_cargo(self) -> bool {
        self == ShipmentStatus::InTransit
    }
}

/// One reserved freight job moving stock between buildings or through `OWA`.
#[derive(Clone, Debug)]
pub struct Shipment {
    /// Stable shipment id used to bind the physical carrier agent to this freight job.
    pub id: u64,
    /// Runtime resource id carried by this shipment.
    pub resource_runtime_id: ResourceRuntimeId,
    /// Reserved amount in resource units.
    pub amount: f32,
    /// Physical source endpoint.
    pub source: ShipmentEndpoint,
    /// Physical destination endpoint.
    pub destination: ShipmentEndpoint,
    /// Carrier class used by the shipment.
    pub carrier_class: CarrierClass,
    /// Current shipment state.
    pub status: ShipmentStatus,
    /// Physical carrier agent; [`AgentHandle::NONE`] until dispatched. It stays set after the
    /// carrier leaves, so "dispatched" never depends on the agent store.
    pub(crate) carrier: AgentHandle,
    /// Input buyer's reserved goods-plus-freight payment, or gross OWA export sale value.
    pub total_cost: f32,
    /// Fixed carrier quote. Settled once on delivery; included in input reservations,
    /// deducted from export proceeds, and fully refunded with failed input orders.
    pub freight_cost: f32,
    /// Estimated trip duration used for capacity and timeout bounds; arrival is physical.
    pub eta_hours: u16,
    /// Logistics hour (`ShipmentSystem::hour_ticks`) in which the current leg's carrier set out:
    /// the leg times out `max(6, 4 × eta_hours + 2)` hours later, at the first check after
    /// that hour starts (`SIM-02-J`). Zero until dispatch.
    pub leg_started_hour: u32,
    /// Slot of the shipment's one live trip event, its departure or its next carrier check; an
    /// event of this shipment due at another slot is a leftover and does nothing (`SIM-02-J`).
    /// Zero before the first is scheduled.
    pub(crate) trip_event_slot: u64,
    /// Whether the cargo has left the source building's stock: loaded at the end of the slot that
    /// planned it, carried from the carrier's departure on (`SIM-02-E`, `SIM-02-J`).
    pub(crate) cargo_loaded: bool,
}

/// Stable key for unresolved input-freight request failure tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FreightRequestKey {
    /// Destination building that needs the input resource.
    pub destination_building_id: usize,
    /// Runtime resource id requested by the destination.
    pub resource_runtime_id: ResourceRuntimeId,
}

/// Failure history for one building/resource freight request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreightRequestFailure {
    /// Consecutive failed planning attempts.
    pub failures: u16,
    /// Whether the request has escalated to a terminal unresolved state.
    pub terminal: bool,
}

/// Shipments created between two drops of the closed ones: closed jobs leave the list whenever
/// a new shipment id is a multiple of this, so the list grows by at most this many jobs between
/// drops and a drop costs O(list) once per this many shipments (`SIM-02-J`).
const SHIPMENT_DROP_INTERVAL: u64 = 256;

/// Runtime collection of active freight jobs.
#[derive(Clone, Debug)]
pub struct ShipmentSystem {
    /// Active and recently closed jobs in ascending stable shipment-ID order.
    /// Append allocates increasing IDs; retention, loading and undo preserve this order.
    pub shipments: Vec<Shipment>,
    /// Next stable shipment id to assign. Saved, so ids are never reused across a load.
    pub next_shipment_id: u64,
    /// Logistics hours started so far; the clock of carrier leg timeouts. Saved.
    pub(crate) hour_ticks: u32,
    /// Shipments loaded since SimCore last scheduled their carriers' departures, in id order.
    pub(crate) held_dispatches: Vec<u64>,
    /// Freight departures already placed in each upcoming minute (`SIM-02-J`). Saved.
    pub(crate) departure_minutes: MinuteLeveler,
    /// Freight requests the building phases queued and the `freight requests` queue has not
    /// served yet, oldest first (`SIM-02-J`). Saved in order.
    pub(crate) freight_requests: VecDeque<FreightRequest>,
    /// Per building handle slot, the generation and the kinds it has queued (`requests.rs`).
    /// Derived from the queue: rebuilt on load, not saved.
    pub(super) request_pending: Vec<(u32, u8)>,
    /// The queue's counts this logistics hour, for its debug line. Not saved.
    pub(super) request_stats: FreightRequestStats,
    /// Consecutive request failures keyed by destination building and resource, including terminal unresolved requests.
    pub request_failures: BTreeMap<FreightRequestKey, FreightRequestFailure>,
    /// Freight ETA cache reused while topology revisions remain unchanged.
    pub(super) freight_route_cache: FreightRouteCache,
    /// Building-reference revision captured when the freight cache was last validated.
    pub(super) freight_route_cache_building_revision: u64,
    /// Entrance-reference revision captured when the freight cache was last validated.
    pub(super) freight_route_cache_entrance_revision: u64,
    /// CCH graph generation captured when the freight cache was last validated.
    pub(super) freight_route_cache_cch_generation: u32,
    /// Customized network metric captured by the cached free-flow costs.
    pub(super) freight_route_cache_metric_revision: u64,
    /// Reservation views kept in step with every shipment change (`SIM-02-I`); `None` until
    /// first use and after a change they cannot follow. Derived, not saved.
    pub(super) reservations: Option<ReservationViews>,
    /// Border nodes, freight components and top freight speed, rebuilt only when the road
    /// graph changes. Derived, not saved.
    pub(super) freight_topology: FreightTopology,
    /// Hour-long rolling build of the dispatch supplier index (`SIM-02-I`); saved.
    pub(crate) supplier_roll: SupplierRoll,
}

/// Freight records removed when one building is bulldozed.
pub(crate) struct ShipmentBuildingUndo {
    pub(crate) shipments: Vec<Shipment>,
    pub(crate) request_failures: Vec<(FreightRequestKey, FreightRequestFailure)>,
    pub(crate) carrier_agent_ids: Vec<usize>,
    pub(crate) mutated_building_ids: Vec<usize>,
}

impl ShipmentSystem {
    /// Creates an empty shipment system.
    pub fn new() -> Self {
        Self {
            shipments: Vec::new(),
            next_shipment_id: 0,
            hour_ticks: 0,
            held_dispatches: Vec::new(),
            departure_minutes: MinuteLeveler::default(),
            freight_requests: VecDeque::new(),
            request_pending: Vec::new(),
            request_stats: FreightRequestStats::default(),
            request_failures: BTreeMap::new(),
            freight_route_cache: FreightRouteCache::default(),
            freight_route_cache_building_revision: u64::MAX,
            freight_route_cache_entrance_revision: u64::MAX,
            freight_route_cache_cch_generation: u32::MAX,
            freight_route_cache_metric_revision: u64::MAX,
            reservations: None,
            freight_topology: FreightTopology::default(),
            supplier_roll: SupplierRoll::default(),
        }
    }

    /// Captures only freight state that building invalidation removes or refunds.
    pub(crate) fn capture_building_undo(
        &self,
        removed_building: usize,
        agents: &AgentSystem,
    ) -> ShipmentBuildingUndo {
        let mut shipments = Vec::new();
        let mut carrier_agent_ids = Vec::new();
        let mut mutated_building_ids = Vec::new();
        for shipment in &self.shipments {
            let touches_removed = shipment.source.touches_building(removed_building)
                || shipment.destination.touches_building(removed_building);
            if !touches_removed {
                continue;
            }
            shipments.push(shipment.clone());
            if let Some(carrier) = agents.resolve_agent(shipment.carrier) {
                carrier_agent_ids.push(carrier);
            }
            if shipment.status.reserves_cargo()
                && shipment.source.touches_building(removed_building)
                && let Some(destination_id) = shipment.destination.building_id()
                && destination_id != removed_building
            {
                mutated_building_ids.push(destination_id);
            }
            if shipment.destination.touches_building(removed_building)
                && shipment.status == ShipmentStatus::InTransit
                && shipment.cargo_loaded
                && let Some(source_id) = shipment.source.building_id()
                && source_id != removed_building
            {
                mutated_building_ids.push(source_id);
            }
        }
        carrier_agent_ids.sort_unstable();
        carrier_agent_ids.dedup();
        mutated_building_ids.sort_unstable();
        mutated_building_ids.dedup();
        let request_failures = self
            .request_failures
            .iter()
            .filter(|(key, _)| key.destination_building_id == removed_building)
            .map(|(&key, &failure)| (key, failure))
            .collect();
        ShipmentBuildingUndo {
            shipments,
            request_failures,
            carrier_agent_ids,
            mutated_building_ids,
        }
    }

    /// Reinserts freight state removed by building invalidation and returns the restored
    /// shipments' ids, whose trips the caller resumes.
    pub(crate) fn restore_building_undo(&mut self, undo: ShipmentBuildingUndo) -> Vec<u64> {
        let restored = undo.shipments.iter().map(|shipment| shipment.id).collect();
        for shipment in undo.shipments {
            if let Some(views) = &mut self.reservations {
                views.add_shipment(&shipment);
            }
            let shipment_idx = self.shipments.partition_point(|job| job.id < shipment.id);
            self.shipments.insert(shipment_idx, shipment);
        }
        for (key, failure) in undo.request_failures {
            self.request_failures.insert(key, failure);
        }
        self.invalidate_route_cache();
        restored
    }

    /// Drops the building-indexed freight quotes after a removal, remap or undo moved dense
    /// building indices, keeping the network sweeps and node costs, which no building index
    /// reaches; the next use revalidates the rest (`FreightRouteCache::clear_buildings`). O(1).
    /// A full clear here rebuilt every border sweep after each building removal, which the
    /// hourly pass did once an hour but the per-slot freight queue would do in the slot after
    /// every removal (`SIM-02-J`).
    pub(crate) fn invalidate_route_cache(&mut self) {
        self.freight_route_cache.clear_buildings();
        self.freight_route_cache_building_revision = u64::MAX;
        self.freight_route_cache_entrance_revision = u64::MAX;
    }

    /// Clears all active shipments, queued freight requests and request diagnostics.
    pub fn clear(&mut self) {
        self.shipments.clear();
        self.next_shipment_id = 0;
        self.departure_minutes = MinuteLeveler::default();
        self.freight_requests.clear();
        self.request_pending.clear();
        self.request_failures.clear();
        self.reservations = None;
        self.supplier_roll = SupplierRoll::default();
        self.freight_route_cache.clear();
        self.freight_route_cache_building_revision = u64::MAX;
        self.freight_route_cache_entrance_revision = u64::MAX;
        self.freight_route_cache_cch_generation = u32::MAX;
        self.freight_route_cache_metric_revision = u64::MAX;
    }

    /// The next shipment id. Every [`SHIPMENT_DROP_INTERVAL`]-th id first drops the closed
    /// shipments, which hold no reservation; the rule follows the saved id counter alone, so a
    /// loaded game drops them at the same shipment. O(1), or O(shipments) at a drop.
    pub(super) fn allocate_shipment_id(&mut self) -> u64 {
        let id = self.next_shipment_id;
        if id.is_multiple_of(SHIPMENT_DROP_INTERVAL) {
            self.shipments.retain(|shipment| shipment.status.is_open());
        }
        self.next_shipment_id = self.next_shipment_id.saturating_add(1);
        id
    }

    /// The lowest id a new shipment may take after loading `next_shipment_id`: past every
    /// saved shipment.
    pub(crate) fn first_free_shipment_id(&self) -> u64 {
        self.shipments
            .iter()
            .map(|shipment| shipment.id)
            .max()
            .map_or(0, |max_id| max_id.saturating_add(1))
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        self.remap_reservations(mapping);
        for shipment in &mut self.shipments {
            shipment.source.remap_building(mapping);
            shipment.destination.remap_building(mapping);
        }

        // Rebuilt only when a key moves: otherwise the rebuild is the same map (`SIM-02-K`).
        // O(F), else O(F log F).
        if self
            .request_failures
            .keys()
            .any(|key| mapping.contains_key(&key.destination_building_id))
        {
            let mut remapped_failures = BTreeMap::new();
            for (mut key, failure) in std::mem::take(&mut self.request_failures) {
                if let Some(&new_id) = mapping.get(&key.destination_building_id) {
                    key.destination_building_id = new_id;
                }
                remapped_failures.insert(key, failure);
            }
            self.request_failures = remapped_failures;
        }
        self.invalidate_route_cache();
    }

    /// Records `slot` as shipment `id`'s one live trip event, so any other event of it is a
    /// leftover. O(log S).
    pub(crate) fn set_trip_event_slot(&mut self, id: u64, slot: u64) {
        if let Ok(idx) = self.shipments.binary_search_by_key(&id, |job| job.id) {
            self.shipments[idx].trip_event_slot = slot;
        }
    }

    /// Whether shipment `id` is listed with a live trip event due at `slot`. O(log S).
    pub(crate) fn trip_event_due(&self, id: u64, slot: u64) -> bool {
        self.shipments
            .binary_search_by_key(&id, |job| job.id)
            .is_ok_and(|idx| self.shipments[idx].trip_event_slot == slot)
    }

    /// Points shipment `shipment_id` at its carrier restored with a new handle by undo. O(log S).
    pub(crate) fn rebind_carrier(&mut self, shipment_id: u64, carrier: AgentHandle) {
        if let Ok(idx) = self
            .shipments
            .binary_search_by_key(&shipment_id, |job| job.id)
        {
            self.shipments[idx].carrier = carrier;
        }
    }

    /// Cancels any shipment touching the removed building before swap-remove happens.
    pub fn invalidate_building(
        &mut self,
        removed_building: usize,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        treasury_balance: &mut f64,
    ) {
        let mut carriers_to_remove = Vec::new();
        let retry_cooldown_hours = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"))
            .operational_clock
            .shipment_retry_cooldown_hours;
        // Endpoints retry from their first phase after this slot's (`event_cooldown_until`).
        let retry_from = allocator.now_slot + 1;
        self.shipments.retain(|shipment| {
            let touches_removed = shipment.source.touches_building(removed_building)
                || shipment.destination.touches_building(removed_building);

            if !touches_removed {
                return true;
            }
            Self::forget_shipment_reservations(&mut self.reservations, shipment);

            if shipment.status.reserves_cargo()
                && let Some(destination_id) = shipment.destination.building_id()
                && destination_id < allocator.buildings.len()
            {
                refund_input_payment(
                    allocator,
                    treasury_balance,
                    destination_id,
                    shipment.total_cost,
                );
                allocator.buildings[destination_id].shipment_cooldown_until_slot =
                    event_cooldown_until(
                        allocator,
                        destination_id,
                        retry_from,
                        retry_cooldown_hours,
                    );
            }
            if shipment.destination.touches_building(removed_building)
                && shipment.status == ShipmentStatus::InTransit
                && shipment.cargo_loaded
                && let ShipmentEndpoint::Building(source_id) = shipment.source
                && source_id < allocator.buildings.len()
                && source_id != removed_building
            {
                allocator.buildings[source_id]
                    .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                allocator.buildings[source_id].shipment_cooldown_until_slot =
                    event_cooldown_until(allocator, source_id, retry_from, retry_cooldown_hours);
            }

            if let Some(carrier) = agents.resolve_agent(shipment.carrier)
                && agents.freight_carrier_matches(carrier, shipment.id)
            {
                carriers_to_remove.push((carrier, shipment.id));
            }
            false
        });
        carriers_to_remove.sort_unstable();
        carriers_to_remove.dedup();
        // Descending dense order keeps the indices still to remove valid.
        for (carrier, shipment_id) in carriers_to_remove.into_iter().rev() {
            agents.remove_freight_carrier(carrier, shipment_id);
        }

        self.request_failures
            .retain(|key, _| key.destination_building_id != removed_building);
        self.invalidate_route_cache();
    }

    // The slot that starts the current logistics hour, for test drivers that step logistics
    // hours without the slot clock.
    #[cfg(test)]
    pub(super) fn hour_start_slot_for_test(&self) -> u64 {
        u64::from(self.hour_ticks) * crate::simulation::core::scheduler::SLOTS_PER_HOUR
    }

    pub(super) fn request_key(
        destination_building_id: usize,
        resource_runtime_id: ResourceRuntimeId,
    ) -> FreightRequestKey {
        FreightRequestKey {
            destination_building_id,
            resource_runtime_id,
        }
    }

    pub(super) fn clear_request_failure(&mut self, key: FreightRequestKey) {
        self.request_failures.remove(&key);
    }

    pub(super) fn record_request_failure(
        &mut self,
        key: FreightRequestKey,
        terminal_failure_attempts: u16,
    ) -> bool {
        let failure = self
            .request_failures
            .entry(key)
            .or_insert(FreightRequestFailure {
                failures: 0,
                terminal: false,
            });
        if failure.terminal {
            return false;
        }
        failure.failures = failure.failures.saturating_add(1);
        failure.terminal = failure.failures >= terminal_failure_attempts;
        failure.terminal
    }
}

impl Default for ShipmentSystem {
    fn default() -> Self {
        Self::new()
    }
}
