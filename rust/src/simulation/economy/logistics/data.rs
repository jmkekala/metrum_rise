//! Core shipment storage and typed logistics state.

use std::collections::{BTreeMap, HashMap};

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::ResourceRuntimeId;

use super::route_cache::FreightRouteCache;

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

    pub(crate) fn border_node(self) -> Option<u32> {
        match self {
            ShipmentEndpoint::Building(_) => None,
            ShipmentEndpoint::OwaBorder(border_node) => Some(border_node),
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
    /// Waiting at a border terminal for active-job capacity.
    Queued,
    /// Travelling and consuming active freight capacity.
    InTransit,
    /// Delivered and ready to be removed from the active job list.
    Fulfilled,
    /// Failed once and ready to be removed from the active job list.
    Failed,
    /// Queued too long and was canceled.
    Expired,
}

impl ShipmentStatus {
    pub(crate) fn code(self) -> i64 {
        match self {
            ShipmentStatus::Queued => 0,
            ShipmentStatus::InTransit => 1,
            ShipmentStatus::Fulfilled => 2,
            ShipmentStatus::Failed => 3,
            ShipmentStatus::Expired => 4,
        }
    }

    pub(crate) fn from_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(ShipmentStatus::Queued),
            1 => Some(ShipmentStatus::InTransit),
            2 => Some(ShipmentStatus::Fulfilled),
            3 => Some(ShipmentStatus::Failed),
            4 => Some(ShipmentStatus::Expired),
            _ => None,
        }
    }

    pub(crate) fn is_open(self) -> bool {
        matches!(self, ShipmentStatus::Queued | ShipmentStatus::InTransit)
    }
}

/// One reserved freight job moving stock between buildings or through `OWA`.
#[derive(Clone, Debug)]
pub struct Shipment {
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
    /// Reserved payment held by the destination until completion or failure.
    pub total_cost: f32,
    /// Reserved purchase tax held until completion or failure.
    pub tax_cost: f32,
    /// Remaining operational-hour steps before the shipment arrives once dispatched.
    pub eta_hours: u16,
    /// Operational hours spent queued at a border terminal.
    pub queued_hours: u16,
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

/// Runtime collection of active freight jobs.
#[derive(Clone, Debug)]
pub struct ShipmentSystem {
    /// All queued or in-transit shipment jobs plus fulfilled/failed jobs awaiting cleanup.
    pub shipments: Vec<Shipment>,
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
}

impl ShipmentSystem {
    /// Creates an empty shipment system.
    pub fn new() -> Self {
        Self {
            shipments: Vec::new(),
            request_failures: BTreeMap::new(),
            freight_route_cache: FreightRouteCache::default(),
            freight_route_cache_building_revision: u64::MAX,
            freight_route_cache_entrance_revision: u64::MAX,
            freight_route_cache_cch_generation: u32::MAX,
        }
    }

    /// Clears all active shipments and freight request diagnostics.
    pub fn clear(&mut self) {
        self.shipments.clear();
        self.request_failures.clear();
        self.freight_route_cache.clear();
        self.freight_route_cache_building_revision = u64::MAX;
        self.freight_route_cache_entrance_revision = u64::MAX;
        self.freight_route_cache_cch_generation = u32::MAX;
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for shipment in &mut self.shipments {
            shipment.source.remap_building(mapping);
            shipment.destination.remap_building(mapping);
        }

        let mut remapped_failures = BTreeMap::new();
        for (mut key, failure) in std::mem::take(&mut self.request_failures) {
            if let Some(&new_id) = mapping.get(&key.destination_building_id) {
                key.destination_building_id = new_id;
            }
            remapped_failures.insert(key, failure);
        }
        self.request_failures = remapped_failures;
        self.freight_route_cache.clear();
        self.freight_route_cache_building_revision = u64::MAX;
        self.freight_route_cache_entrance_revision = u64::MAX;
        self.freight_route_cache_cch_generation = u32::MAX;
    }

    /// Cancels any shipment touching the removed building before swap-remove happens.
    pub fn invalidate_building(
        &mut self,
        removed_building: usize,
        allocator: &mut BuildingAllocator,
    ) {
        self.shipments.retain(|shipment| {
            let touches_removed = shipment.source.touches_building(removed_building)
                || shipment.destination.touches_building(removed_building);

            if !touches_removed {
                return true;
            }

            if shipment.source.touches_building(removed_building)
                && let Some(destination_id) = shipment.destination.building_id()
                && destination_id < allocator.buildings.len()
                && destination_id != removed_building
            {
                allocator.buildings[destination_id].operating_budget +=
                    shipment.total_cost + shipment.tax_cost;
            }

            false
        });

        self.request_failures
            .retain(|key, _| key.destination_building_id != removed_building);
        self.freight_route_cache.clear();
        self.freight_route_cache_building_revision = u64::MAX;
        self.freight_route_cache_entrance_revision = u64::MAX;
        self.freight_route_cache_cch_generation = u32::MAX;
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

    pub(super) fn request_is_terminal(&self, key: FreightRequestKey) -> bool {
        self.request_failures
            .get(&key)
            .is_some_and(|failure| failure.terminal)
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
