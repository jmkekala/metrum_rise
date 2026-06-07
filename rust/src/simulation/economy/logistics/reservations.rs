//! Reservation views for active inbound, outbound, and border freight jobs.

use std::collections::HashMap;

use crate::simulation::economy::definitions::ResourceRuntimeId;

use super::data::{
    SHIPMENT_DEST_OWA, SHIPMENT_IN_TRANSIT, SHIPMENT_SOURCE_LOCAL, SHIPMENT_SOURCE_OWA,
    ShipmentSystem,
};

/// Flattened reservation state derived from active shipments.
pub(super) struct ReservationViews {
    /// Local-source reserved inventory indexed by building/resource slot.
    pub(super) reserved_outbound: Vec<f32>,
    /// Destination-side expected inventory indexed by building/resource slot.
    pub(super) reserved_inbound: Vec<f32>,
    /// Whether an inbound shipment is already open for a building/resource slot.
    pub(super) has_open_inbound: Vec<bool>,
    /// Active `OWA` import job counts indexed by border node id.
    pub(super) border_job_counts: HashMap<u32, usize>,
}

impl ShipmentSystem {
    /// Returns the current local-source outbound reservation slots indexed by building/resource.
    pub(crate) fn reserved_outbound_view(&self, resource_count: usize) -> Vec<f32> {
        self.build_reservation_views(resource_count)
            .reserved_outbound
    }

    /// Computes the flattened reservation slot for a building/resource pair.
    pub(crate) fn reservation_slot_for_building(
        building_idx: usize,
        resource_runtime_id: ResourceRuntimeId,
        resource_count: usize,
    ) -> Option<usize> {
        reservation_slot(building_idx, resource_runtime_id, resource_count)
    }

    pub(super) fn build_reservation_views(&self, resource_count: usize) -> ReservationViews {
        let mut max_building = 0usize;
        for shipment in &self.shipments {
            // Skip the OWA export sentinel; it is not a real building index.
            if shipment.destination_building_id != SHIPMENT_DEST_OWA {
                max_building = max_building.max(shipment.destination_building_id);
            }
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL {
                max_building = max_building.max(shipment.source_building_id);
            }
        }

        let slot_count = max_building
            .saturating_add(1)
            .saturating_mul(resource_count.max(1));
        let mut reserved_outbound = vec![0.0; slot_count];
        let mut reserved_inbound = vec![0.0; slot_count];
        let mut has_open_inbound = vec![false; slot_count];
        let mut border_job_counts = HashMap::new();

        for shipment in &self.shipments {
            if shipment.status != SHIPMENT_IN_TRANSIT {
                continue;
            }
            if let Some(slot) = reservation_slot(
                shipment.destination_building_id,
                shipment.resource_runtime_id,
                resource_count,
            ) && slot < reserved_inbound.len()
            {
                reserved_inbound[slot] += shipment.amount;
                has_open_inbound[slot] = true;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                && let Some(slot) = reservation_slot(
                    shipment.source_building_id,
                    shipment.resource_runtime_id,
                    resource_count,
                )
                && slot < reserved_outbound.len()
            {
                reserved_outbound[slot] += shipment.amount;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_OWA {
                *border_job_counts
                    .entry(shipment.source_border_node)
                    .or_insert(0) += 1;
            }
        }

        ReservationViews {
            reserved_outbound,
            reserved_inbound,
            has_open_inbound,
            border_job_counts,
        }
    }
}

pub(super) fn reservation_slot(
    building_idx: usize,
    resource_runtime_id: ResourceRuntimeId,
    resource_count: usize,
) -> Option<usize> {
    if resource_runtime_id == 0 || resource_count == 0 {
        return None;
    }
    building_idx
        .checked_mul(resource_count)
        .and_then(|base| base.checked_add(resource_runtime_id as usize - 1))
}
