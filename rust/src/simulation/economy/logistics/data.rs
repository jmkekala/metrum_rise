//! Core shipment storage and stable public logistics constants.

use std::collections::HashMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::ResourceRuntimeId;

/// The shipment originates from a local supplier building.
pub const SHIPMENT_SOURCE_LOCAL: u8 = 0;
/// The shipment originates from an `OWA` border terminal.
pub const SHIPMENT_SOURCE_OWA: u8 = 1;
/// Sentinel value for `destination_building_id` on `OWA` export shipments.
///
/// When a shipment carries this as its destination, the goods are travelling from a local
/// building to the `OWA` border terminal identified by `source_border_node`. On arrival the
/// source building receives `total_cost` as revenue; no local building receives goods.
pub const SHIPMENT_DEST_OWA: usize = usize::MAX;
/// The assigned carrier is a local or border truck.
pub const CARRIER_TRUCK: u8 = 0;
/// Shipment is active and still travelling.
pub const SHIPMENT_IN_TRANSIT: u8 = 0;
/// Shipment arrived successfully.
pub const SHIPMENT_FULFILLED: u8 = 1;
/// Shipment failed and its reservations were released.
pub const SHIPMENT_FAILED: u8 = 2;

/// Maximum concurrent `OWA` import jobs allowed for one border node.
pub(super) const BORDER_ACTIVE_JOBS_PER_NODE: usize = 4;

/// One reserved freight job moving stock between buildings or from `OWA`.
#[derive(Clone, Debug)]
pub struct Shipment {
    /// Runtime resource id carried by this shipment.
    pub resource_runtime_id: ResourceRuntimeId,
    /// Reserved amount in resource units.
    pub amount: f32,
    /// Whether the source is local or `OWA`.
    pub source_kind: u8,
    /// Source building index for local shipments; `usize::MAX` for `OWA`.
    pub source_building_id: usize,
    /// Border node used by `OWA` import shipments (source) and `OWA` export shipments
    /// (destination); `u32::MAX` for purely local freight.
    pub source_border_node: u32,
    /// Destination building receiving the shipment.
    pub destination_building_id: usize,
    /// Carrier class used by the shipment.
    pub carrier_class: u8,
    /// Current shipment state.
    pub status: u8,
    /// Reserved payment held by the destination until completion or failure.
    pub total_cost: f32,
    /// Remaining operational-hour steps before the shipment arrives.
    pub eta_hours: u16,
}

/// Runtime collection of active freight jobs.
#[derive(Clone, Debug, Default)]
pub struct ShipmentSystem {
    /// All active shipment jobs.
    pub shipments: Vec<Shipment>,
}

impl ShipmentSystem {
    /// Creates an empty shipment system.
    pub fn new() -> Self {
        Self {
            shipments: Vec::new(),
        }
    }

    /// Clears all active shipments.
    pub fn clear(&mut self) {
        self.shipments.clear();
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for shipment in &mut self.shipments {
            if let Some(&new_id) = mapping.get(&shipment.destination_building_id) {
                shipment.destination_building_id = new_id;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                && let Some(&new_id) = mapping.get(&shipment.source_building_id)
            {
                shipment.source_building_id = new_id;
            }
        }
    }

    /// Cancels any shipment touching the removed building before swap-remove happens.
    pub fn invalidate_building(
        &mut self,
        removed_building: usize,
        allocator: &mut BuildingAllocator,
    ) {
        self.shipments.retain(|shipment| {
            let touches_removed = shipment.destination_building_id == removed_building
                || (shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                    && shipment.source_building_id == removed_building);

            if !touches_removed {
                return true;
            }

            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                && shipment.source_building_id == removed_building
                && shipment.destination_building_id < allocator.buildings.len()
                && shipment.destination_building_id != removed_building
            {
                allocator.buildings[shipment.destination_building_id].operating_budget +=
                    shipment.total_cost;
            }

            false
        });
    }
}
